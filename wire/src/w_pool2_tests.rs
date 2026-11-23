// W-POOL2 tests: wire follows redirects itself on the non-jar path too, so
// every hop of a chain is listed in `X-Wire-Hop-Classes`, also when the hops
// reuse pooled connections; ureq's redirect rules are kept (limits, method
// rewriting, dropped credentials, one deadline for the whole chain), and
// `Clear-Site-Data` on a redirect hop is honoured.

use crate::cache::Cache;
use crate::{ParsedRequest, Shared, build_agent, execute_with};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use url::Url;

/// The answer a handler gives: status line rest ("302 Found"), headers, body.
type Answer = (String, Vec<(String, String)>, Vec<u8>);
type Handler = dyn Fn(&str, &[u8]) -> Answer + Send + Sync;

/// A keep-alive loopback server: answers every request of a connection with
/// `Content-Length`, counts connections and records (head, body) of every
/// request.
struct Srv {
    base: String,
    conns: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
}

impl Srv {
    fn new(handler: impl Fn(&str, &[u8]) -> Answer + Send + Sync + 'static) -> Srv {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let conns = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let (c2, s2) = (Arc::clone(&conns), Arc::clone(&seen));
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(s) = s else { continue };
                c2.fetch_add(1, Ordering::SeqCst);
                let (handler, seen) = (Arc::clone(&handler), Arc::clone(&s2));
                std::thread::spawn(move || serve(s, &*handler, &seen));
            }
        });
        Srv { base, conns, seen }
    }

    fn conns(&self) -> usize {
        self.conns.load(Ordering::SeqCst)
    }

    fn seen(&self) -> Vec<(String, Vec<u8>)> {
        self.seen.lock().unwrap().clone()
    }
}

fn serve(mut s: TcpStream, handler: &Handler, seen: &Mutex<Vec<(String, Vec<u8>)>>) {
    let mut buf = Vec::new();
    let mut b = [0u8; 4096];
    loop {
        let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
            match s.read(&mut b) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&b[..n]),
            }
            continue;
        };
        let head = String::from_utf8_lossy(&buf[..p]).to_string();
        let cl = req_header(&head, "content-length").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        while buf.len() < p + 4 + cl {
            match s.read(&mut b) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&b[..n]),
            }
        }
        let body = buf[p + 4..p + 4 + cl].to_vec();
        buf.drain(..p + 4 + cl);
        seen.lock().unwrap().push((head.clone(), body.clone()));
        let (status, headers, rbody) = handler(&head, &body);
        let mut out = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", rbody.len());
        for (n, v) in headers {
            out.push_str(&format!("{n}: {v}\r\n"));
        }
        out.push_str("\r\n");
        let head_only = head.starts_with("HEAD ");
        if s.write_all(out.as_bytes()).is_err() || (!head_only && s.write_all(&rbody).is_err()) {
            return;
        }
    }
}

fn req_header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).filter_map(|l| l.split_once(':')).find(|(n, _)| n.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())
}

fn path_of(head: &str) -> &str {
    head.split_whitespace().nth(1).unwrap_or("")
}

fn method_of(head: &str) -> &str {
    head.split_whitespace().next().unwrap_or("")
}

fn redirect(status: &str, to: &str) -> Answer {
    (status.to_string(), vec![("Location".into(), to.into())], Vec::new())
}

fn ok(body: &str) -> Answer {
    ("200 OK".into(), Vec::new(), body.as_bytes().to_vec())
}

struct Resp {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Resp {
    fn h(&self, name: &str) -> Option<&str> {
        req_header(&self.head, name)
    }
}

fn run(agent: &ureq::Agent, shared: &Shared, method: &str, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Resp {
    let req = ParsedRequest {
        method: method.into(),
        url: Url::parse(url).unwrap(),
        headers: headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        body: body.to_vec(),
    };
    let mut raw = Vec::new();
    execute_with(agent, shared, &req, &mut raw).unwrap();
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(raw[..end].to_vec()).unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    Resp { status, head, body: raw[end + 4..].to_vec() }
}

/// Three servers chained A/r -> B/r -> C/end (anything else answers "warm").
fn chain() -> (Srv, Srv, Srv) {
    let c = Srv::new(|head, _| if path_of(head) == "/end" { ok("end") } else { ok("warm") });
    let cb = c.base.clone();
    let b = Srv::new(move |head, _| if path_of(head) == "/r" { redirect("302 Found", &format!("{cb}/end")) } else { ok("warm") });
    let bb = b.base.clone();
    let a = Srv::new(move |head, _| if path_of(head) == "/r" { redirect("302 Found", &format!("{bb}/r")) } else { ok("warm") });
    (a, b, c)
}

#[test]
fn every_hop_of_a_chain_on_pooled_connections_is_in_the_hop_classes() {
    let (a, b, c) = chain();
    let agent = build_agent();
    let shared = Shared::new(true);
    // Fresh connections: the resolver sees all three hops.
    let r = run(&agent, &shared, "GET", &format!("{}/r", a.base), &[], b"");
    assert_eq!((r.status, r.body.as_slice()), (200, &b"end"[..]));
    assert_eq!(r.h("X-Wire-Hop-Classes"), Some("loopback,loopback,loopback"), "{}", r.head);
    // Now every hop reuses a pooled connection (no resolution at all): still three.
    for _ in 0..3 {
        let r = run(&agent, &shared, "GET", &format!("{}/r", a.base), &[], b"");
        assert_eq!((r.status, r.body.as_slice()), (200, &b"end"[..]));
        assert_eq!(r.h("X-Wire-Hop-Classes"), Some("loopback,loopback,loopback"), "{}", r.head);
        assert_eq!(r.h("X-Wire-Target-Class"), Some("loopback"));
        assert_eq!(r.h("X-Wire-Final-Url"), Some(format!("{}/end", c.base).as_str()));
    }
    assert_eq!((a.conns(), b.conns(), c.conns()), (1, 1, 1), "every hop used its pooled connection");
}

#[test]
fn a_pooled_chain_with_a_non_default_limit_keeps_its_connections_and_hops() {
    let (a, b, c) = chain();
    let agent = build_agent();
    let shared = Shared::new(true);
    for _ in 0..3 {
        let r = run(&agent, &shared, "GET", &format!("{}/r", a.base), &[("X-Wire-Max-Redirects", "3")], b"");
        assert_eq!(r.status, 200);
        assert_eq!(r.h("X-Wire-Hop-Classes"), Some("loopback,loopback,loopback"), "{}", r.head);
    }
    // Before W-POOL2 a non-default limit built a one-off agent (3 connections each).
    assert_eq!((a.conns(), b.conns(), c.conns()), (1, 1, 1));
}

/// A chain /h0 -> /h1 -> ... -> /h{n} on one server; /h{n} answers 200.
fn long_chain(n: usize) -> Srv {
    Srv::new(move |head, _| {
        let i: usize = path_of(head).trim_start_matches("/h").parse().unwrap_or(n);
        if i < n { redirect("302 Found", &format!("/h{}", i + 1)) } else { ok("done") }
    })
}

#[test]
fn redirect_limits_count_as_ureq_did() {
    let agent = build_agent();
    let shared = Shared::new(true);
    // Default limit 5: four redirects are followed, the fifth fails.
    let s = long_chain(4);
    let r = run(&agent, &shared, "GET", &format!("{}/h0", s.base), &[], b"");
    assert_eq!(r.status, 200);
    assert_eq!(r.h("X-Wire-Hop-Classes").map(|h| h.split(',').count()), Some(5));
    let s = long_chain(5);
    let r = run(&agent, &shared, "GET", &format!("{}/h0", s.base), &[], b"");
    assert_eq!(r.status, 502);
    assert_eq!(r.h("X-Wire-Error"), Some("1"));
    assert!(String::from_utf8_lossy(&r.body).contains("Too Many Redirects"), "{:?}", String::from_utf8_lossy(&r.body));
    // `X-Wire-Max-Redirects: n` follows n - 1.
    let s = long_chain(2);
    assert_eq!(run(&agent, &shared, "GET", &format!("{}/h0", s.base), &[("X-Wire-Max-Redirects", "2")], b"").status, 502);
    assert_eq!(run(&agent, &shared, "GET", &format!("{}/h0", s.base), &[("X-Wire-Max-Redirects", "3")], b"").status, 200);
    // 0 returns the 3xx as is.
    let r = run(&agent, &shared, "GET", &format!("{}/h0", s.base), &[("X-Wire-Max-Redirects", "0")], b"");
    assert_eq!(r.status, 302);
    assert_eq!(r.h("Location"), Some("/h1"));
    // A 3xx without Location is the answer.
    let s = Srv::new(|_, _| ("302 Found".into(), Vec::new(), b"nowhere".to_vec()));
    let r = run(&agent, &shared, "GET", &s.base, &[], b"");
    assert_eq!((r.status, r.body.as_slice()), (302, &b"nowhere"[..]));
    // A Location with another scheme is a marked 502.
    let s = Srv::new(|_, _| redirect("302 Found", "ftp://example.invalid/x"));
    let r = run(&agent, &shared, "GET", &s.base, &[], b"");
    assert_eq!((r.status, r.h("X-Wire-Error")), (502, Some("1")));
}

#[test]
fn methods_bodies_and_credentials_follow_ureqs_rules() {
    let s = Srv::new(|head, _| match path_of(head) {
        "/301" => redirect("301 Moved Permanently", "/end"),
        "/302" => redirect("302 Found", "/end"),
        "/303" => redirect("303 See Other", "/end"),
        "/307" => redirect("307 Temporary Redirect", "/end"),
        "/308" => redirect("308 Permanent Redirect", "/end"),
        _ => ok(&format!("end {}", method_of(head))),
    });
    let agent = build_agent();
    let shared = Shared::new(true);
    let creds = [("Cookie", "a=1"), ("Authorization", "Basic eDp5"), ("X-Keep", "k")];
    let cases = [
        ("POST", "/302", 200, "end GET"),
        ("PUT", "/301", 200, "end GET"),
        ("DELETE", "/303", 200, "end GET"),
        ("HEAD", "/303", 200, ""),
        ("GET", "/307", 200, "end GET"),
        ("OPTIONS", "/308", 200, "end OPTIONS"),
        // 307/308 with a method that may carry a body: the 3xx itself.
        ("POST", "/307", 307, ""),
        ("PUT", "/308", 308, ""),
    ];
    for (method, path, status, body) in cases {
        let before = s.seen().len();
        let payload: &[u8] = if matches!(method, "POST" | "PUT") { b"payload=1" } else { b"" };
        let r = run(&agent, &shared, method, &format!("{}{path}", s.base), &creds, payload);
        assert_eq!(r.status, status, "{method} {path}: {}", r.head);
        assert_eq!(String::from_utf8_lossy(&r.body), body, "{method} {path}");
        let seen = s.seen();
        let (first, rest) = seen[before..].split_first().unwrap();
        assert_eq!(first.1, payload, "the first hop gets the body");
        assert_eq!(req_header(&first.0, "cookie"), Some("a=1"));
        assert_eq!(req_header(&first.0, "authorization"), Some("Basic eDp5"));
        assert_eq!(rest.len(), usize::from(status == 200), "{method} {path}");
        for (head, b) in rest {
            assert!(b.is_empty(), "the body is never re-sent: {head}");
            assert_eq!(req_header(head, "cookie"), None, "Cookie dropped on a redirect: {head}");
            assert_eq!(req_header(head, "authorization"), None, "Authorization dropped on a redirect: {head}");
            assert_eq!(req_header(head, "x-keep"), Some("k"), "other headers stay: {head}");
        }
    }
}

#[test]
fn the_timeout_bounds_the_whole_chain() {
    let s = Srv::new(|head, _| {
        std::thread::sleep(Duration::from_millis(300));
        let i: usize = path_of(head).trim_start_matches("/h").parse().unwrap_or(9);
        if i < 3 { redirect("302 Found", &format!("/h{}", i + 1)) } else { ok("done") }
    });
    let agent = build_agent();
    let shared = Shared::new(true);
    // Four hops of 300 ms each: 700 ms is enough for every single hop, not for the chain.
    let r = run(&agent, &shared, "GET", &format!("{}/h0", s.base), &[("X-Wire-Timeout-Ms", "700")], b"");
    assert_eq!((r.status, r.h("X-Wire-Error")), (502, Some("1")), "{}", r.head);
    let r = run(&agent, &shared, "GET", &format!("{}/h0", s.base), &[("X-Wire-Timeout-Ms", "5000")], b"");
    assert_eq!(r.status, 200);
}

#[test]
fn clear_site_data_on_a_redirect_hop_clears_the_cache() {
    let dir = std::env::temp_dir().join(format!("wire-cache-test-wpool2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let s = Srv::new(|head, _| match path_of(head) {
        "/go" => ("302 Found".into(), vec![("Location".into(), "/landed".into()), ("Clear-Site-Data".into(), "\"cache\"".into())], Vec::new()),
        "/landed" => ("200 OK".into(), vec![("Cache-Control".into(), "no-store".into())], b"landed".to_vec()),
        _ => ("200 OK".into(), vec![("Cache-Control".into(), "max-age=600".into())], b"page".to_vec()),
    });
    let mut shared = Shared::new(true);
    shared.cache = Some(Arc::new(Cache::open(&dir, 1 << 20).unwrap()));
    let agent = build_agent();
    let top = [("X-Wire-Top-Site", "http://a.example")];
    let page = format!("{}/page.css", s.base);
    run(&agent, &shared, "GET", &page, &top, b"");
    assert_eq!(run(&agent, &shared, "GET", &page, &top, b"").h("X-Wire-Cache"), Some("hit"));
    let r = run(&agent, &shared, "GET", &format!("{}/go", s.base), &top, b"");
    assert_eq!((r.status, r.body.as_slice()), (200, &b"landed"[..]));
    assert_eq!(run(&agent, &shared, "GET", &page, &top, b"").h("X-Wire-Cache"), Some("miss"), "the redirect hop's Clear-Site-Data emptied the origin's entries");
    let _ = std::fs::remove_dir_all(&dir);
}
