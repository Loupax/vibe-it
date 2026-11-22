// W-CACHE1 tests: the on-disk HTTP cache. Every test uses its own cache
// directory under the temp dir (never ~/.cache/wire) and removes it again; the
// 1 GiB test stores under `target/` (on disk, not tmpfs).

use crate::cache::*;
use crate::{ParsedRequest, Shared, build_agent, execute_with, handle_connection};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use url::Url;

type Handler = dyn Fn(&str, usize, &mut TcpStream) + Send + Sync;

/// A loopback origin that counts its requests. `handler(request head, n,
/// stream)` writes the answer; every connection gets its own thread.
struct Origin {
    base: String,
    hits: Arc<AtomicUsize>,
    heads: Arc<Mutex<Vec<String>>>,
}

impl Origin {
    fn new(handler: impl Fn(&str, usize, &mut TcpStream) + Send + Sync + 'static) -> Origin {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let hits = Arc::new(AtomicUsize::new(0));
        let heads = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let (h2, hd2) = (Arc::clone(&hits), Arc::clone(&heads));
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let (handler, hits, heads) = (Arc::clone(&handler), Arc::clone(&h2), Arc::clone(&hd2));
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut b = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match s.read(&mut b) {
                            Ok(0) | Err(_) => return,
                            Ok(k) => buf.extend_from_slice(&b[..k]),
                        }
                    }
                    let head = String::from_utf8_lossy(&buf).to_string();
                    let n = hits.fetch_add(1, Ordering::SeqCst);
                    heads.lock().unwrap().push(head.clone());
                    handler(&head, n, &mut s);
                });
            }
        });
        Origin { base, hits, heads }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    fn last_head(&self) -> String {
        self.heads.lock().unwrap().last().cloned().unwrap_or_default()
    }
}

fn path_of(head: &str) -> &str {
    head.split_whitespace().nth(1).unwrap_or("")
}

fn req_header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().filter_map(|l| l.split_once(':')).find(|(n, _)| n.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())
}

/// A complete answer with Content-Length and `Connection: close`.
fn answer(s: &mut TcpStream, status: &str, headers: &[(&str, &str)], body: &[u8]) {
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (n, v) in headers {
        out.push_str(&format!("{n}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let _ = s.write_all(out.as_bytes());
    let _ = s.write_all(body);
}

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wire-cache-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn with_cache(dir: &Path, limit: u64) -> Shared {
    let mut s = Shared::new(true);
    s.cache = Some(Arc::new(Cache::open(dir, limit).unwrap()));
    s
}

struct Resp {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Resp {
    fn h(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).filter_map(|l| l.split_once(':')).find(|(n, _)| n.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())
    }
    fn cache(&self) -> &str {
        self.h("x-wire-cache").unwrap_or("")
    }
}

fn send(shared: &Shared, method: &str, url: &str, headers: &[(&str, &str)]) -> Resp {
    let req = ParsedRequest {
        method: method.into(),
        url: Url::parse(url).unwrap(),
        headers: headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        body: if method == "POST" { b"x=1".to_vec() } else { Vec::new() },
    };
    let mut raw = Vec::new();
    execute_with(&build_agent(), shared, &req, &mut raw).unwrap();
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(raw[..end].to_vec()).unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    Resp { status, head, body: raw[end + 4..].to_vec() }
}

fn get(shared: &Shared, url: &str, headers: &[(&str, &str)]) -> Resp {
    send(shared, "GET", url, headers)
}

/// Entry files (not temp files) under the cache dir.
fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
            if !f.file_name().to_string_lossy().starts_with(".tmp-") {
                out.push(f.path());
            }
        }
    }
    out
}

fn temps(dir: &Path) -> usize {
    std::fs::read_dir(dir).into_iter().flatten().flatten().flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten()).filter(|f| f.file_name().to_string_lossy().starts_with(".tmp-")).count()
}

fn fresh_origin(body: &'static str) -> Origin {
    Origin::new(move |_, _, s| answer(s, "200 OK", &[("Content-Type", "text/css"), ("Cache-Control", "max-age=600")], body.as_bytes()))
}

#[test]
fn max_age_hit_with_zero_upstream_requests() {
    let dir = tmp_dir("maxage");
    let o = fresh_origin("p{color:red}");
    let sh = with_cache(&dir, 1 << 20);
    let a = get(&sh, &format!("{}/a.css", o.base), &[]);
    assert_eq!((a.status, a.cache()), (200, "miss"), "{}", a.head);
    let b = get(&sh, &format!("{}/a.css", o.base), &[]);
    assert_eq!((b.status, b.cache(), b.body.as_slice()), (200, "hit", &b"p{color:red}"[..]), "{}", b.head);
    assert_eq!(o.hits(), 1, "the hit made no upstream request");
    assert!(b.h("age").is_some(), "Age on every stored answer: {}", b.head);
    assert_eq!(b.h("x-wire-target-class"), Some("loopback"), "classes replayed from the entry");
    assert_eq!(b.h("x-wire-final-url"), Some(format!("{}/a.css", o.base).as_str()));
    assert_eq!(b.h("x-wire-version"), Some("4"));
    assert!(b.h("x-wire-features").unwrap().split(',').any(|f| f == "cache"));
    assert_eq!(b.h("content-type"), Some("text/css"));
    assert_eq!(entries(&dir).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stale_entry_is_revalidated_and_304_merges_headers() {
    let dir = tmp_dir("reval");
    let o = Origin::new(|head, n, s| {
        if n == 0 {
            answer(s, "200 OK", &[("ETag", "\"v1\""), ("Cache-Control", "max-age=0"), ("X-Test", "one"), ("Content-Type", "text/plain")], b"body-v1");
        } else {
            assert_eq!(req_header(head, "if-none-match"), Some("\"v1\""), "wire sends the stored validator: {head}");
            let _ = s.write_all(b"HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\nX-Test: two\r\nCache-Control: max-age=0\r\nConnection: close\r\n\r\n");
        }
    });
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/r", o.base);
    assert_eq!(get(&sh, &url, &[]).cache(), "miss");
    let b = get(&sh, &url, &[]);
    assert_eq!((b.status, b.cache(), b.body.as_slice()), (200, "revalidated", &b"body-v1"[..]), "{}", b.head);
    assert_eq!(b.h("x-test"), Some("two"), "304 headers replace stored ones");
    assert_eq!(b.h("content-type"), Some("text/plain"), "other stored headers stay");
    let c = get(&sh, &url, &[]);
    assert_eq!((c.cache(), c.h("x-test")), ("revalidated", Some("two")), "the merged headers were written to disk");
    assert_eq!(o.hits(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn heuristic_freshness_from_last_modified() {
    let dir = tmp_dir("heur");
    let now = crate::now_unix();
    let (date, lm) = (http_date(now), http_date(now - 10 * 86400));
    let o = Origin::new(move |_, _, s| answer(s, "200 OK", &[("Date", &date), ("Last-Modified", &lm)], b"old file"));
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/h", o.base);
    get(&sh, &url, &[]);
    assert_eq!(get(&sh, &url, &[]).cache(), "hit", "10% of 10 days = 1 day fresh");
    assert_eq!(o.hits(), 1);
    // A status that is not heuristically cacheable is not.
    let o2 = Origin::new(move |_, _, s| answer(s, "403 Forbidden", &[("Last-Modified", &http_date(now - 10 * 86400))], b"no"));
    get(&sh, &format!("{}/x", o2.base), &[]);
    get(&sh, &format!("{}/x", o2.base), &[]);
    assert_eq!(o2.hits(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_store_is_never_written_to_disk() {
    let dir = tmp_dir("nostore");
    let o = Origin::new(|_, _, s| answer(s, "200 OK", &[("Cache-Control", "no-store, max-age=600")], b"secret"));
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/s", o.base);
    assert_eq!(get(&sh, &url, &[]).body, b"secret");
    get(&sh, &url, &[]);
    assert_eq!(o.hits(), 2);
    assert!(entries(&dir).is_empty() && temps(&dir) == 0, "nothing on disk");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vary_user_agent_compares_the_ua_wire_sent() {
    let dir = tmp_dir("vary");
    let o = Origin::new(|head, _, s| {
        let ua = req_header(head, "user-agent").unwrap_or("").to_string();
        answer(s, "200 OK", &[("Cache-Control", "max-age=600"), ("Vary", "User-Agent, Accept-Encoding")], ua.as_bytes());
    });
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/v", o.base);
    assert_eq!(get(&sh, &url, &[("X-Wire-User-Agent", "UA-one")]).body, b"UA-one");
    let b = get(&sh, &url, &[("X-Wire-User-Agent", "UA-two")]);
    assert_eq!((b.cache(), b.body.as_slice()), ("miss", &b"UA-two"[..]));
    assert_eq!(o.hits(), 2, "two User-Agents, two upstream fetches");
    let c = get(&sh, &url, &[("X-Wire-User-Agent", "UA-two")]);
    assert_eq!((c.cache(), c.body.as_slice()), ("hit", &b"UA-two"[..]));
    // `Vary: *` is never stored.
    let o2 = Origin::new(|_, _, s| answer(s, "200 OK", &[("Cache-Control", "max-age=600"), ("Vary", "*")], b"x"));
    get(&sh, &format!("{}/star", o2.base), &[]);
    get(&sh, &format!("{}/star", o2.base), &[]);
    assert_eq!(o2.hits(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_top_sites_are_two_partitions() {
    let dir = tmp_dir("part");
    let o = fresh_origin("shared.css");
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/s.css", o.base);
    get(&sh, &url, &[("X-Wire-Top-Site", "https://a.example")]);
    let b = get(&sh, &url, &[("X-Wire-Top-Site", "https://b.example/page")]);
    assert_eq!(b.cache(), "miss");
    assert_eq!(o.hits(), 2, "partitioned: no sharing across top sites");
    assert_eq!(get(&sh, &url, &[("X-Wire-Top-Site", "https://www.a.example/other")]).cache(), "hit", "same site, same partition");
    assert_eq!(get(&sh, &url, &[]).cache(), "miss", "no top site: the unpartitioned bucket");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3, "one directory per partition");
    assert!(sh.cache.as_ref().unwrap().part_dir("https://a.example").is_dir());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn public_only_never_gets_a_stored_private_answer() {
    let dir = tmp_dir("policy");
    let o = fresh_origin("private stuff");
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("http://localhost:{}/p", o.base.rsplit(':').next().unwrap());
    assert_eq!(get(&sh, &url, &[]).status, 200);
    let b = get(&sh, &url, &[("X-Wire-Policy", "public-only")]);
    assert_eq!((b.status, b.h("x-wire-error")), (403, Some("policy")), "{}", b.head);
    assert_ne!(b.cache(), "hit");
    // only-if-cached cannot get it either.
    let c = get(&sh, &url, &[("X-Wire-Policy", "public-only"), ("Cache-Control", "only-if-cached")]);
    assert_eq!(c.status, 504);
    assert_eq!(get(&sh, &url, &[]).cache(), "hit", "a plain request still may");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn post_invalidates_the_url_and_its_location() {
    let dir = tmp_dir("post");
    let o = Origin::new(|head, _, s| {
        if head.starts_with("POST") {
            answer(s, "200 OK", &[("Content-Location", "/other")], b"posted");
        } else {
            answer(s, "200 OK", &[("Cache-Control", "max-age=600")], path_of(head).as_bytes());
        }
    });
    let sh = with_cache(&dir, 1 << 20);
    let (url, other) = (format!("{}/item", o.base), format!("{}/other", o.base));
    get(&sh, &url, &[]);
    get(&sh, &other, &[]);
    assert_eq!(get(&sh, &url, &[]).cache(), "hit");
    let p = send(&sh, "POST", &url, &[]);
    assert_eq!((p.status, p.cache()), (200, "bypass"));
    assert_eq!(get(&sh, &url, &[]).cache(), "miss", "POST dropped the entry");
    assert_eq!(get(&sh, &other, &[]).cache(), "miss", "and the Content-Location one");
    assert_eq!(o.hits(), 5);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_six_fetch_cache_modes() {
    let dir = tmp_dir("modes");
    // Every answer: ETag, max-age=600, body = the request number.
    let o = Origin::new(|head, n, s| {
        if req_header(head, "if-none-match").is_some() {
            let _ = s.write_all(b"HTTP/1.1 304 Not Modified\r\nETag: \"e\"\r\nCache-Control: max-age=600\r\nConnection: close\r\n\r\n");
        } else {
            answer(s, "200 OK", &[("ETag", "\"e\""), ("Cache-Control", "max-age=600")], format!("n{n}").as_bytes());
        }
    });
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/m", o.base);
    // default
    assert_eq!(get(&sh, &url, &[]).body, b"n0");
    assert_eq!(get(&sh, &url, &[]).cache(), "hit");
    assert_eq!(o.hits(), 1);
    // no-store: skips the cache entirely (no lookup, nothing stored)
    let r = get(&sh, &url, &[("Cache-Control", "no-store")]);
    assert_eq!((r.cache(), r.body.as_slice()), ("bypass", &b"n1"[..]));
    assert!(req_header(&o.last_head(), "cache-control") == Some("no-store"), "forwarded upstream as browsers do");
    assert_eq!(get(&sh, &url, &[]).body, b"n0", "the stored copy was not replaced");
    // reload: unconditional, stores the result
    let r = get(&sh, &url, &[("Cache-Control", "no-cache"), ("Pragma", "no-cache")]);
    assert_eq!((r.cache(), r.body.as_slice()), ("miss", &b"n2"[..]));
    assert!(req_header(&o.last_head(), "if-none-match").is_none(), "no conditional on reload");
    assert_eq!(get(&sh, &url, &[]).body, b"n2", "the reload answer was stored");
    // no-cache: always revalidate
    let r = get(&sh, &url, &[("Cache-Control", "max-age=0")]);
    assert_eq!((r.cache(), r.body.as_slice()), ("revalidated", &b"n2"[..]));
    assert_eq!(o.hits(), 4);
    // force-cache: any stored copy, even a stale one
    let o2 = Origin::new(|_, n, s| answer(s, "200 OK", &[("ETag", "\"s\""), ("Expires", "Thu, 01 Jan 1970 00:00:00 GMT")], format!("s{n}").as_bytes()));
    let stale = format!("{}/stale", o2.base);
    get(&sh, &stale, &[]);
    let r = get(&sh, &stale, &[("Cache-Control", "max-stale")]);
    assert_eq!((r.cache(), r.body.as_slice()), ("stale", &b"s0"[..]));
    assert_eq!(o2.hits(), 1);
    let r = get(&sh, &format!("{}/never", o2.base), &[("Cache-Control", "max-stale")]);
    assert_eq!((r.cache(), r.status), ("miss", 200), "nothing stored: network");
    // only-if-cached: the stored copy, or 504
    let r = get(&sh, &stale, &[("Cache-Control", "only-if-cached")]);
    assert_eq!((r.status, r.cache(), r.body.as_slice()), (200, "stale", &b"s0"[..]));
    let before = o2.hits();
    let r = get(&sh, &format!("{}/nothing", o2.base), &[("Cache-Control", "only-if-cached")]);
    assert_eq!((r.status, r.h("x-wire-error")), (504, Some("only-if-cached")));
    assert_eq!(o2.hits(), before, "only-if-cached never goes upstream");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_truncated_upstream_body_is_not_stored() {
    let dir = tmp_dir("trunc");
    let o = Origin::new(|_, _, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nCache-Control: max-age=600\r\nConnection: close\r\n\r\n0123456789");
    });
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/t", o.base);
    let a = get(&sh, &url, &[]);
    assert_eq!(a.body, b"0123456789");
    assert!(entries(&dir).is_empty() && temps(&dir) == 0, "announced 1000, got 10: nothing stored");
    get(&sh, &url, &[]);
    assert_eq!(o.hits(), 2);
    // Also on the X-Wire-Max-Bytes (buffered) path.
    get(&sh, &url, &[("X-Wire-Max-Bytes", "5000")]);
    assert!(entries(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn temp_files_are_never_visible_as_entries() {
    let dir = tmp_dir("tmpvis");
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let rx = Mutex::new(rx);
    let o = Origin::new(move |head, _, s| {
        if path_of(head) == "/slow" {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nCache-Control: max-age=600\r\nConnection: close\r\n\r\n0123456789");
            let _ = s.flush();
            let _ = rx.lock().unwrap().recv_timeout(Duration::from_secs(10));
            let _ = s.write_all(b"abcdefghij");
        } else {
            answer(s, "200 OK", &[], b"other");
        }
    });
    let sh = Arc::new(with_cache(&dir, 1 << 20));
    let url = format!("{}/slow", o.base);
    let (sh2, url2) = (Arc::clone(&sh), url.clone());
    let t = std::thread::spawn(move || get(&sh2, &url2, &[]));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while temps(&dir) == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(temps(&dir), 1, "the body streams into a temp file");
    assert!(entries(&dir).is_empty(), "no entry before it is complete");
    let path = sh.cache.as_ref().unwrap().entry_path("", &cache_key(false, &Url::parse(&url).unwrap()));
    assert!(Entry::open(&path).is_none());
    tx.send(()).unwrap();
    let r = t.join().unwrap();
    assert_eq!(r.body, b"0123456789abcdefghij");
    assert_eq!((entries(&dir).len(), temps(&dir)), (1, 0), "renamed into place");
    assert_eq!(get(&sh, &url, &[]).cache(), "hit");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn eviction_keeps_the_directory_under_the_limit_oldest_first() {
    let dir = tmp_dir("evict");
    let limit = 64 * 1024;
    let o = Origin::new(|head, _, s| answer(s, "200 OK", &[("Cache-Control", "max-age=600")], &[path_of(head).as_bytes()[1]; 6000]));
    let sh = with_cache(&dir, limit);
    let c = sh.cache.as_ref().unwrap();
    let mut paths = Vec::new();
    for (i, name) in "abcdefghijklmnopqrst".chars().enumerate() {
        let url = format!("{}/{name}", o.base);
        get(&sh, &url, &[]);
        let p = c.entry_path("", &cache_key(false, &Url::parse(&url).unwrap()));
        // Deterministic ages: a is the oldest.
        File::options().write(true).open(&p).unwrap().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + i as u64)).unwrap();
        paths.push(p);
    }
    assert!(c.approx_bytes() > limit, "20 x 6 KB is over 64 KiB");
    // A hit touches mtime: `a` becomes the newest.
    assert_eq!(get(&sh, &format!("{}/a", o.base), &[]).cache(), "hit");
    let s = c.evict_now();
    assert!(s.removed > 0 && s.after <= limit, "{s:?}");
    let total: u64 = entries(&dir).iter().map(|p| std::fs::metadata(p).unwrap().len()).sum();
    assert!(total <= limit, "{total}");
    assert!(paths[0].exists(), "the touched entry stays");
    assert!(!paths[1].exists() && !paths[2].exists(), "oldest-mtime files go first");
    assert!(paths[19].exists(), "the newest stays");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_vanishing_mid_scan_is_not_an_error() {
    let dir = tmp_dir("vanish");
    let o = Origin::new(|head, _, s| answer(s, "200 OK", &[("Cache-Control", "max-age=600")], &[path_of(head).as_bytes()[1]; 3000]));
    let sh = with_cache(&dir, 16 * 1024);
    for name in ["a", "b", "c", "d", "e", "f", "g", "h"] {
        get(&sh, &format!("{}/{name}", o.base), &[]);
    }
    let c = sh.cache.as_ref().unwrap();
    let (parts, items, total) = c.scan();
    // Another wire (or this one) deletes files between the scan and the eviction.
    for p in entries(&dir).iter().take(3) {
        std::fs::remove_file(p).unwrap();
    }
    let s = c.evict_items(&parts, items, total);
    assert!(s.after <= 16 * 1024, "{s:?}");
    // A lookup of a vanished entry is a miss, not an error.
    let r = get(&sh, &format!("{}/a", o.base), &[]);
    assert_eq!(r.status, 200);
    // A whole partition directory vanishing mid-scan, too.
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(c.evict_items(&parts, Vec::new(), 0).after, 0);
    assert_eq!(c.evict_now().before, 0);
}

#[test]
fn range_is_answered_from_a_stored_full_body() {
    let dir = tmp_dir("range");
    let o = Origin::new(|_, _, s| {
        let body: Vec<u8> = (0..100).map(|i| b'0' + (i % 10) as u8).collect();
        answer(s, "200 OK", &[("Cache-Control", "max-age=600"), ("Content-Type", "video/mp4")], &body);
    });
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/v.mp4", o.base);
    get(&sh, &url, &[]);
    let r = get(&sh, &url, &[("Range", "bytes=10-19")]);
    assert_eq!((r.status, r.cache(), r.body.as_slice()), (206, "hit", &b"0123456789"[..]));
    assert_eq!(r.h("content-range"), Some("bytes 10-19/100"));
    assert_eq!(r.h("accept-ranges"), Some("bytes"));
    let r = get(&sh, &url, &[("Range", "bytes=-3")]);
    assert_eq!((r.status, r.body.as_slice()), (206, &b"789"[..]));
    let r = get(&sh, &url, &[("Range", "bytes=95-")]);
    assert_eq!((r.status, r.body.as_slice(), r.h("content-range")), (206, &b"56789"[..], Some("bytes 95-99/100")));
    let r = get(&sh, &url, &[("Range", "bytes=200-")]);
    assert_eq!((r.status, r.h("content-range")), (416, Some("bytes */100")));
    let r = get(&sh, &url, &[("Range", "bytes=0-1,5-6")]);
    assert_eq!((r.status, r.body.len()), (200, 100), "multi-range: the full body");
    assert_eq!(o.hits(), 1);
    // HEAD is answered from the stored GET.
    let h = send(&sh, "HEAD", &url, &[]);
    assert_eq!((h.status, h.cache(), h.body.len()), (200, "hit", 0));
    assert_eq!(o.hits(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn byte_range_parser() {
    assert_eq!(parse_byte_range("bytes=0-9", 100), Some(Ok((0, 9))));
    assert_eq!(parse_byte_range("bytes=5-", 100), Some(Ok((5, 99))));
    assert_eq!(parse_byte_range("bytes=-3", 100), Some(Ok((97, 99))));
    assert_eq!(parse_byte_range("bytes=-300", 100), Some(Ok((0, 99))));
    assert_eq!(parse_byte_range("bytes=90-200", 100), Some(Ok((90, 99))));
    assert_eq!(parse_byte_range("bytes=100-", 100), Some(Err(())));
    assert_eq!(parse_byte_range("bytes=1-2,4-5", 100), None);
    assert_eq!(parse_byte_range("items=1-2", 100), None);
    assert_eq!(parse_byte_range("bytes=9-1", 100), None);
}

#[test]
fn cache_size_parser() {
    assert_eq!(parse_size("256M"), Ok(256 << 20));
    assert_eq!(parse_size("1G"), Ok(1 << 30));
    assert_eq!(parse_size("512k"), Ok(512 << 10));
    assert_eq!(parse_size("1GiB"), Ok(1 << 30));
    assert_eq!(parse_size("4096"), Ok(4096));
    assert!(parse_size("0").is_err() && parse_size("lots").is_err() && parse_size("").is_err());
}

#[test]
fn authorization_needs_public_must_revalidate_or_s_maxage() {
    let dir = tmp_dir("auth");
    let o = Origin::new(|head, _, s| {
        let cc = if path_of(head) == "/pub" { "max-age=600, public" } else { "max-age=600" };
        answer(s, "200 OK", &[("Cache-Control", cc)], b"for you");
    });
    let sh = with_cache(&dir, 1 << 20);
    let auth = [("Authorization", "Bearer x")];
    get(&sh, &format!("{}/plain", o.base), &auth);
    assert_eq!(get(&sh, &format!("{}/plain", o.base), &auth).cache(), "miss");
    get(&sh, &format!("{}/pub", o.base), &auth);
    assert_eq!(get(&sh, &format!("{}/pub", o.base), &auth).cache(), "hit");
    assert_eq!(o.hits(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn immutable_is_not_revalidated_on_a_normal_reload() {
    let dir = tmp_dir("immutable");
    let o = Origin::new(|head, _, s| {
        let cc = if path_of(head) == "/i" { "max-age=600, immutable" } else { "max-age=600" };
        if req_header(head, "if-none-match").is_some() {
            let _ = s.write_all(b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n");
        } else {
            answer(s, "200 OK", &[("Cache-Control", cc), ("ETag", "\"t\"")], b"lib.js");
        }
    });
    let sh = with_cache(&dir, 1 << 20);
    let reload = [("Cache-Control", "max-age=0")];
    get(&sh, &format!("{}/i", o.base), &[]);
    assert_eq!(get(&sh, &format!("{}/i", o.base), &reload).cache(), "hit");
    get(&sh, &format!("{}/m", o.base), &[]);
    assert_eq!(get(&sh, &format!("{}/m", o.base), &reload).cache(), "revalidated");
    assert_eq!(o.hits(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_wires_can_share_one_cache_dir() {
    let dir = tmp_dir("sharedir");
    let o = fresh_origin("shared");
    let (a, b) = (with_cache(&dir, 1 << 20), with_cache(&dir, 1 << 20));
    let url = format!("{}/x", o.base);
    get(&a, &url, &[]);
    assert_eq!(get(&b, &url, &[]).cache(), "hit");
    // b evicts everything under a; a just misses.
    let small = Cache::open(&dir, 1).unwrap();
    small.evict_now();
    assert_eq!(get(&a, &url, &[]).cache(), "miss");
    assert_eq!(o.hits(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_write_error_still_answers_from_the_network() {
    let dir = tmp_dir("werr");
    let o = fresh_origin("still here");
    let sh = with_cache(&dir, 1 << 20);
    // A plain file where the partition directory should be: creating the
    // temp file fails (as a full or read-only disk would).
    std::fs::write(sh.cache.as_ref().unwrap().part_dir(""), b"in the way").unwrap();
    let url = format!("{}/w", o.base);
    let r = get(&sh, &url, &[]);
    assert_eq!((r.status, r.body.as_slice(), r.cache()), (200, &b"still here"[..], "miss"));
    assert_eq!(get(&sh, &url, &[]).body, b"still here");
    assert_eq!(o.hits(), 2, "not stored");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn set_cookie_is_stored_without_set_cookie() {
    let dir = tmp_dir("cookie");
    let o = Origin::new(|_, _, s| answer(s, "200 OK", &[("Cache-Control", "max-age=600"), ("Set-Cookie", "a=1")], b"c"));
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/c", o.base);
    assert_eq!(get(&sh, &url, &[]).h("set-cookie"), Some("a=1"));
    let b = get(&sh, &url, &[]);
    assert_eq!((b.cache(), b.h("set-cookie")), ("hit", None), "never replayed");
    assert!(!std::fs::read_to_string(&entries(&dir)[0]).unwrap().contains("a=1"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_client_conditional_is_answered_from_the_store() {
    let dir = tmp_dir("cond");
    let o = Origin::new(|_, _, s| answer(s, "200 OK", &[("Cache-Control", "max-age=600"), ("ETag", "W/\"abc\"")], b"c"));
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/c", o.base);
    get(&sh, &url, &[]);
    let r = get(&sh, &url, &[("If-None-Match", "\"abc\"")]);
    assert_eq!((r.status, r.cache(), r.body.len()), (304, "hit", 0));
    assert_eq!(r.h("etag"), Some("W/\"abc\""));
    let r = get(&sh, &url, &[("If-None-Match", "\"other\"")]);
    assert_eq!((r.status, r.body.as_slice()), (200, &b"c"[..]), "not matching: the client gets the upstream answer");
    assert_eq!(o.hits(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn x_wire_cache_on_every_response_and_none_without_cache() {
    let dir = tmp_dir("hdr");
    let sh = with_cache(&dir, 1 << 20);
    let f = std::env::temp_dir().join(format!("wire-cache-file-{}.txt", std::process::id()));
    std::fs::write(&f, "x").unwrap();
    assert_eq!(get(&sh, Url::from_file_path(&f).unwrap().as_str(), &[]).cache(), "bypass", "file: is never cached");
    assert_eq!(get(&sh, "http://127.0.0.1:1/down", &[]).cache(), "miss", "wire-made errors carry it too");
    get(&sh, "http://127.0.0.1:1/down", &[]);
    assert!(entries(&dir).is_empty(), "a wire-made error is never stored");
    let plain = Shared::new(true);
    assert_eq!(get(&plain, Url::from_file_path(&f).unwrap().as_str(), &[]).h("x-wire-cache"), None);
    let _ = std::fs::remove_file(&f);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_redirected_answer_is_not_stored() {
    let dir = tmp_dir("redir");
    let o = Origin::new(|head, _, s| {
        if path_of(head) == "/from" {
            answer(s, "302 Found", &[("Location", "/to"), ("Cache-Control", "max-age=600")], b"");
        } else {
            answer(s, "200 OK", &[("Cache-Control", "max-age=600")], b"target");
        }
    });
    let sh = with_cache(&dir, 1 << 20);
    let from = format!("{}/from", o.base);
    assert_eq!(get(&sh, &from, &[]).body, b"target");
    assert_eq!(get(&sh, &from, &[]).cache(), "miss");
    assert_eq!(o.hits(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_jar_path_is_cached_with_its_own_key() {
    let dir = tmp_dir("jar");
    let o = Origin::new(|head, _, s| {
        let c = req_header(head, "cookie").unwrap_or("none").to_string();
        answer(s, "200 OK", &[("Cache-Control", "max-age=600"), ("Vary", "Cookie"), ("Set-Cookie", "k=v; Max-Age=600")], c.as_bytes());
    });
    let sh = with_cache(&dir, 1 << 20);
    let url = format!("{}/j", o.base);
    let jar = [("X-Wire-Jar", "scratchy"), ("X-Wire-Top-Site", "http://127.0.0.1")];
    let a = get(&sh, &url, &jar);
    assert_eq!((a.body.as_slice(), a.h("x-wire-cookies")), (&b"none"[..], Some("on")));
    // The jar now holds k=v, so `Vary: Cookie` differs: a second fetch.
    let b = get(&sh, &url, &jar);
    assert_eq!((b.cache(), b.body.as_slice()), ("miss", &b"k=v"[..]));
    let c = get(&sh, &url, &jar);
    assert_eq!((c.cache(), c.h("x-wire-cookies")), ("hit", Some("on")), "jar requests served from the store still say cookies: on");
    assert_eq!(get(&sh, &url, &[("X-Wire-Top-Site", "http://127.0.0.1")]).cache(), "miss", "jar on/off is part of the key");
    let _ = std::fs::remove_dir_all(&dir);
}

/// VmHWM (peak RSS) of this process, in KiB.
fn peak_rss_kib() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    s.lines().find_map(|l| l.strip_prefix("VmHWM:")).and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok()).unwrap()
}

#[test]
fn storing_a_1_gib_body_does_not_grow_peak_rss() {
    const SIZE: u64 = 1 << 30;
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("wire-cache-big-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let o = Origin::new(|_, _, s| {
        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {SIZE}\r\nCache-Control: max-age=600\r\nConnection: close\r\n\r\n").as_bytes());
        let chunk = vec![b'z'; 1 << 20];
        for _ in 0..SIZE >> 20 {
            if s.write_all(&chunk).is_err() {
                return;
            }
        }
    });
    let mut sh = Shared::new(true);
    sh.cache = Some(Arc::new(Cache::open(&dir, 16 * SIZE).unwrap()));
    let sh = Arc::new(sh);
    let before = peak_rss_kib();
    let (client, server) = UnixStream::pair().unwrap();
    let sh2 = Arc::clone(&sh);
    let t = std::thread::spawn(move || handle_connection(server, &build_agent(), &sh2));
    let mut client = client;
    client.write_all(format!("GET {}/big HTTP/1.1\r\nHost: x\r\n\r\n", o.base).as_bytes()).unwrap();
    let mut n = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        match client.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => n += k as u64,
            Err(e) => panic!("{e}"),
        }
    }
    t.join().unwrap();
    let grew = peak_rss_kib().saturating_sub(before);
    assert!(n > SIZE, "head + 1 GiB body came through: {n}");
    let e = entries(&dir);
    assert_eq!(e.len(), 1, "stored");
    assert!(std::fs::metadata(&e[0]).unwrap().len() > SIZE);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(grew < 64 * 1024, "peak RSS grew by {grew} KiB while storing 1 GiB");
}

use std::fs::File;
