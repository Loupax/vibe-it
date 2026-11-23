// W3 (web/): `X-Wire-Max-Redirects: 0` returns the 3xx itself and keeps
// connection reuse (the shared no-follow agent, not a one-off agent per request).

use crate::{ParsedRequest, Shared, build_agent, execute_with};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use url::Url;

/// Keep-alive server: every request is answered `302 Location: /next`.
fn redirecting_server() -> (String, Arc<AtomicUsize>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let conns = Arc::new(AtomicUsize::new(0));
    let c2 = Arc::clone(&conns);
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            c2.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let mut b = [0u8; 4096];
                loop {
                    while let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        buf.drain(..p + 4);
                        if s.write_all(b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\n\r\n").is_err() {
                            return;
                        }
                    }
                    match s.read(&mut b) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&b[..n]),
                    }
                }
            });
        }
    });
    (base, conns)
}

#[test]
fn max_redirects_zero_returns_the_redirect_and_reuses_the_connection() {
    let (base, conns) = redirecting_server();
    let shared = Shared::new(true);
    let agent = build_agent();
    let req = ParsedRequest {
        method: "GET".into(),
        url: Url::parse(&format!("{base}/start")).unwrap(),
        headers: vec![("X-Wire-Max-Redirects".into(), "0".into())],
        body: Vec::new(),
    };
    for _ in 0..5 {
        let mut raw = Vec::new();
        execute_with(&agent, &shared, &req, &mut raw).unwrap();
        let t = String::from_utf8_lossy(&raw);
        assert!(t.starts_with("HTTP/1.1 302"), "{t}");
        assert!(t.to_ascii_lowercase().contains("location: /next\r\n"), "{t}");
        assert!(t.contains(&format!("X-Wire-Final-Url: {base}/start\r\n")), "the URL asked for, not followed: {t}");
    }
    assert_eq!(conns.load(Ordering::SeqCst), 1, "all five requests used one pooled connection");
}
