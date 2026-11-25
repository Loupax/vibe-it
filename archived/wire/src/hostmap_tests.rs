// QWPT8 tests: `--map-host` end to end through the request path (the pure parser and matcher
// tests live in hostmap.rs). Every server here is a loopback TCP listener of the test itself.

use crate::hostmap::{self, HostMap};
use crate::{ParsedRequest, Shared, build_agent, execute_with};
use std::io::{Read, Write};
use std::net::TcpListener;
use url::Url;

/// Loopback origin: answers `/redir` with a 302 to `http://127.0.0.1:<port>/ok`, everything else 200 "mapped-ok".
fn origin() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let resp = if head.starts_with("GET /redir") {
                format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/ok\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nmapped-ok".to_string()
            };
            let _ = s.write_all(resp.as_bytes());
        }
    });
    port
}

fn fetch(url: &str, public_only: bool) -> (u16, Vec<(String, String)>, String) {
    let mut headers = Vec::new();
    if public_only {
        headers.push(("X-Wire-Policy".to_string(), "public-only".to_string()));
    }
    let req = ParsedRequest { method: "GET".into(), url: Url::parse(url).unwrap(), headers, body: Vec::new() };
    let mut raw = Vec::new();
    execute_with(&build_agent(), &Shared::new(true), &req, &mut raw).unwrap();
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = std::str::from_utf8(&raw[..end]).unwrap();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    let hs = lines.filter_map(|l| l.split_once(':').map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))).collect();
    (status, hs, String::from_utf8_lossy(&raw[end + 4..]).to_string())
}

fn hdr<'a>(h: &'a [(String, String)], n: &str) -> Option<&'a str> {
    h.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.as_str())
}

fn map() {
    hostmap::add(HostMap::parse("*.hm-e2e.test=127.0.0.1").unwrap());
}

#[test]
fn a_mapped_host_reaches_loopback_even_under_public_only() {
    map();
    let port = origin();
    for host in ["hm-e2e.test", "www.hm-e2e.test", "www2.hm-e2e.test"] {
        for po in [true, false] {
            let (st, h, body) = fetch(&format!("http://{host}:{port}/"), po);
            assert_eq!((st, body.as_str()), (200, "mapped-ok"), "{host} public_only={po}");
            assert_eq!(hdr(&h, "X-Wire-Target-Class"), Some("loopback"));
        }
    }
}

#[test]
fn other_hosts_are_not_mapped_and_stay_policed() {
    map();
    let port = origin();
    // A name that merely looks like the mapped one never reaches the loopback origin (it needs real DNS).
    for host in ["hm-e2e.test.invalid", "evil-hm-e2e.test", "hm-e2e.testx"] {
        let (st, _, body) = fetch(&format!("http://{host}:{port}/"), true);
        assert_ne!(body, "mapped-ok", "{host} must not be mapped");
        assert_ne!(st, 200, "{host}");
    }
    // The loopback IP literal itself is still refused by public-only while a mapping exists.
    let (st, h, _) = fetch(&format!("http://127.0.0.1:{port}/"), true);
    assert_eq!(st, 403);
    assert_eq!(hdr(&h, "X-Wire-Error"), Some("policy"));
}

#[test]
fn a_redirect_from_a_mapped_host_to_an_unmapped_loopback_target_is_still_refused() {
    map();
    let port = origin();
    let (st, h, body) = fetch(&format!("http://www.hm-e2e.test:{port}/redir"), true);
    assert_eq!(st, 403, "the second hop (127.0.0.1) is not a mapped name");
    assert_eq!(hdr(&h, "X-Wire-Error"), Some("policy"));
    assert_ne!(body, "mapped-ok");
}

#[test]
fn mount_override_applies_to_a_mapped_host_only() {
    map();
    let port = origin();
    let f = std::env::temp_dir().join(format!("wire-hm-override-{}.js", std::process::id()));
    std::fs::write(&f, "hook();").unwrap();
    let mut shared = Shared::new(true);
    shared.mounts.add_override(&format!("/resources/hook.js={}", f.display())).unwrap();
    let get = |url: String| {
        let req = ParsedRequest { method: "GET".into(), url: Url::parse(&url).unwrap(), headers: Vec::new(), body: Vec::new() };
        let mut raw = Vec::new();
        execute_with(&build_agent(), &shared, &req, &mut raw).unwrap();
        String::from_utf8_lossy(&raw).to_string()
    };
    assert!(get(format!("http://www.hm-e2e.test:{port}/resources/hook.js?x=1")).ends_with("hook();"));
    assert!(get(format!("http://www.hm-e2e.test:{port}/other.js")).ends_with("mapped-ok"));
    // 127.0.0.1 is not a mapped name: the override is not applied (and it goes to the network, plain request).
    assert!(get(format!("http://127.0.0.1:{port}/resources/hook.js")).ends_with("mapped-ok"));
    let _ = std::fs::remove_file(&f);
}

#[test]
fn startup_refuses_other_hosts_and_non_loopback_targets() {
    for spec in [
        "*.web-platform.test=8.8.8.8",
        "*.web-platform.test=10.1.2.3",
        "*.web-platform.test=0.0.0.0",
        "*.example.com=127.0.0.1",
        "*.com=127.0.0.1",
        "*=127.0.0.1",
        "www.web-platform.test=127.0.0.1",
    ] {
        assert!(HostMap::parse(spec).is_err(), "{spec}");
    }
}
