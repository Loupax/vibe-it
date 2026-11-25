// W-ENC1 tests: wire decodes br as well as gzip, and never strips a
// Content-Encoding it did not undo.

use crate::{ParsedRequest, build_agent, execute_and_stream};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use url::Url;

/// `brotli -c` of "hello brotli hello brotli hello brotli".
const BR: &[u8] = &[
    0x1f, 0x25, 0x00, 0xf8, 0x1d, 0xa9, 0x53, 0x9f, 0xbb, 0x70, 0x2d, 0x56, 0x86, 0x26, 0x27, 0x41, 0xd8, 0xe8, 0x92, 0x2d, 0xed, 0x2d, 0x88, 0x8a, 0x06, 0x5e,
    0x66, 0xfc, 0x02,
];
const BR_PLAIN: &[u8] = b"hello brotli hello brotli hello brotli";
/// A gzip member of "hello gzip".
const GZ: &[u8] = &[
    0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0x48, 0xaf, 0xca, 0x2c, 0x00, 0x00, 0x19, 0x6a, 0xd2, 0xdf, 0x0a,
    0x00, 0x00, 0x00,
];

/// Serves `head_extra` headers plus `body` once per connection; the request head
/// each connection sent comes back on the channel.
fn serve(head_extra: &'static str, body: &'static [u8]) -> (std::net::SocketAddr, mpsc::Receiver<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
            let head = format!("HTTP/1.1 200 OK\r\n{head_extra}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(body);
        }
    });
    (addr, rx)
}

fn fetch(addr: std::net::SocketAddr, headers: Vec<(String, String)>) -> (Vec<(String, String)>, Vec<u8>) {
    let url = Url::parse(&format!("http://{addr}/")).unwrap();
    let req = ParsedRequest { method: "GET".into(), url, headers, body: Vec::new() };
    let mut raw = Vec::new();
    execute_and_stream(&build_agent(), &req, &mut raw).unwrap();
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let headers = std::str::from_utf8(&raw[..end])
        .unwrap()
        .split("\r\n")
        .skip(1)
        .filter_map(|l| l.split_once(':').map(|(n, v)| (n.trim().to_string(), v.trim().to_string())))
        .collect();
    (headers, raw[end + 4..].to_vec())
}

fn has(h: &[(String, String)], name: &str) -> bool {
    h.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
}

#[test]
fn br_body_is_decoded_and_wire_asks_for_it() {
    let (addr, rx) = serve("Content-Encoding: br\r\n", BR);
    let (h, body) = fetch(addr, vec![]);
    assert_eq!(body, BR_PLAIN);
    assert!(!has(&h, "content-encoding"), "{h:?}");
    assert!(!has(&h, "x-wire-upstream-length"), "{h:?}");
    let req = rx.recv().unwrap().to_ascii_lowercase();
    assert!(req.contains("accept-encoding: gzip, br"), "{req}");
}

#[test]
fn a_clients_accept_encoding_is_not_forwarded() {
    let (addr, rx) = serve("Content-Encoding: br\r\n", BR);
    let (_, body) = fetch(addr, vec![("Accept-Encoding".into(), "gzip, deflate, br, zstd".into())]);
    assert_eq!(body, BR_PLAIN);
    let req = rx.recv().unwrap().to_ascii_lowercase();
    assert!(!req.contains("zstd"), "{req}");
}

#[test]
fn gzip_is_still_decoded() {
    let (addr, _rx) = serve("Content-Encoding: gzip\r\n", GZ);
    let (h, body) = fetch(addr, vec![]);
    assert_eq!(body, b"hello gzip");
    assert!(!has(&h, "content-encoding") && !has(&h, "x-wire-upstream-length"), "{h:?}");
}

#[test]
fn an_encoding_wire_did_not_undo_passes_through_with_its_header() {
    let (addr, _rx) = serve("Content-Encoding: x-custom\r\n", b"opaque");
    let (h, body) = fetch(addr, vec![]);
    assert_eq!(body, b"opaque");
    assert!(h.iter().any(|(n, v)| n.eq_ignore_ascii_case("content-encoding") && v == "x-custom"), "{h:?}");
    // The body is the upstream's own bytes, so its length is still reported.
    assert!(h.iter().any(|(n, v)| n.eq_ignore_ascii_case("x-wire-upstream-length") && v == "6"), "{h:?}");
}

#[test]
fn zstd_from_a_server_passes_through_undecoded() {
    let (addr, _rx) = serve("Content-Encoding: zstd\r\n", b"\x28\xb5\x2f\xfd");
    let (h, body) = fetch(addr, vec![]);
    assert_eq!(body, b"\x28\xb5\x2f\xfd");
    assert!(h.iter().any(|(n, v)| n.eq_ignore_ascii_case("content-encoding") && v == "zstd"), "{h:?}");
}

#[test]
fn a_range_request_asks_for_no_encoding() {
    let (addr, rx) = serve("", b"abc");
    let _ = fetch(addr, vec![("Range".into(), "bytes=0-2".into())]);
    let req = rx.recv().unwrap().to_ascii_lowercase();
    assert!(!req.contains("accept-encoding: gzip"), "{req}");
}

/// Cache interplay: the cache stores what wire emits, so a decoded br body is stored
/// decoded (no Content-Encoding) and a passed-through encoding is stored with its header.
fn cached_get(sh: &crate::Shared, addr: std::net::SocketAddr) -> (String, Vec<u8>) {
    let url = Url::parse(&format!("http://{addr}/")).unwrap();
    let req = ParsedRequest { method: "GET".into(), url, headers: Vec::new(), body: Vec::new() };
    let mut raw = Vec::new();
    crate::execute_with(&build_agent(), sh, &req, &mut raw).unwrap();
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (String::from_utf8(raw[..end].to_vec()).unwrap().to_ascii_lowercase(), raw[end + 4..].to_vec())
}

fn cache_shared(name: &str) -> (crate::Shared, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("wire-enc1-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = crate::Shared::new(true);
    s.cache = Some(std::sync::Arc::new(crate::cache::Cache::open(&dir, 1 << 20).unwrap()));
    (s, dir)
}

#[test]
fn a_decoded_br_body_is_cached_decoded() {
    let (addr, rx) = serve("Content-Encoding: br\r\nCache-Control: max-age=600\r\n", BR);
    let (sh, dir) = cache_shared("br");
    let (h1, b1) = cached_get(&sh, addr);
    let (h2, b2) = cached_get(&sh, addr);
    assert!(h2.contains("x-wire-cache: hit"), "{h2}");
    assert_eq!((b1.as_slice(), b2.as_slice()), (BR_PLAIN, BR_PLAIN));
    for h in [&h1, &h2] {
        assert!(!h.contains("content-encoding") && !h.contains("x-wire-upstream-length"), "{h}");
    }
    assert_eq!(rx.try_iter().count(), 1, "one origin request");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_passed_through_encoding_is_cached_with_its_header() {
    let (addr, _rx) = serve("Content-Encoding: x-custom\r\nCache-Control: max-age=600\r\n", b"opaque");
    let (sh, dir) = cache_shared("custom");
    let _ = cached_get(&sh, addr);
    let (h2, b2) = cached_get(&sh, addr);
    assert!(h2.contains("x-wire-cache: hit"), "{h2}");
    assert_eq!(b2, b"opaque");
    assert!(h2.contains("content-encoding: x-custom"), "{h2}");
    let _ = std::fs::remove_dir_all(&dir);
}
