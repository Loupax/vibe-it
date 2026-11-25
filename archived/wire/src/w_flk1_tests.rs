// W-FLK1 tests: a pooled connection that still holds bytes of the previous
// answer (a server that sends a body after a 204 or a HEAD response, as WPT's
// status.py does). ureq 2 pools such a connection; the next request on it
// read those bytes as its status line and failed with a 502.

use crate::{ParsedRequest, build_agent, execute_and_stream};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use url::Url;

struct Server {
    base: String,
    conns: Arc<AtomicUsize>,
    /// Requests the server read completely (head and body).
    requests: Arc<AtomicUsize>,
}

/// Reads one request (head plus Content-Length body), returns its target; None on EOF.
fn read_request(s: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut b = [0u8; 4096];
    loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..p]).to_ascii_lowercase();
            let cl = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            while buf.len() < p + 4 + cl {
                let n = s.read(&mut b).ok()?;
                if n == 0 {
                    return None;
                }
                buf.extend_from_slice(&b[..n]);
            }
            return Some(head.split_whitespace().nth(1).unwrap_or("").to_string());
        }
        let n = s.read(&mut b).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&b[..n]);
    }
}

/// Keep-alive server. A request for `/poison` is answered with `poison` (sent
/// in one write, so the stray body arrives together with the head and lands
/// in the client's read buffer); every other request with a clean 200 "ok".
fn server(poison: &'static [u8]) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let conns = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let (c2, r2) = (Arc::clone(&conns), Arc::clone(&requests));
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            c2.fetch_add(1, Ordering::SeqCst);
            let r3 = Arc::clone(&r2);
            std::thread::spawn(move || {
                while let Some(target) = read_request(&mut s) {
                    r3.fetch_add(1, Ordering::SeqCst);
                    let answer: &[u8] = if target == "/poison" { poison } else { b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok" };
                    if s.write_all(answer).is_err() {
                        return;
                    }
                }
            });
        }
    });
    Server { base, conns, requests }
}

const NO_CONTENT_WITH_BODY: &[u8] = b"HTTP/1.1 204 No Content\r\nContent-Length: 11\r\n\r\nhello-world";
const HEAD_WITH_BODY: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nhello-world";

fn request(base: &str, method: &str, body: &[u8]) -> ParsedRequest {
    at(base, "/x", method, body)
}

fn poison(base: &str, method: &str) -> ParsedRequest {
    at(base, "/poison", method, b"")
}

fn at(base: &str, path: &str, method: &str, body: &[u8]) -> ParsedRequest {
    ParsedRequest { method: method.to_string(), url: Url::parse(&format!("{base}{path}")).unwrap(), headers: Vec::new(), body: body.to_vec() }
}

fn status_of(agent: &ureq::Agent, r: &ParsedRequest) -> u16 {
    let mut raw = Vec::new();
    execute_and_stream(agent, r, &mut raw).unwrap();
    String::from_utf8_lossy(&raw).split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[test]
fn a_get_after_a_204_that_carried_a_body_succeeds() {
    let srv = server(NO_CONTENT_WITH_BODY);
    let agent = build_agent();
    for _ in 0..20 {
        assert_eq!(status_of(&agent, &poison(&srv.base, "GET")), 204);
        // ureq pooled the connection with "hello-world" still in its buffer.
        assert_eq!(status_of(&agent, &request(&srv.base, "GET", b"")), 200, "the GET is resent on a fresh connection");
    }
    assert_eq!(srv.requests.load(Ordering::SeqCst), 60, "20 x (204, garbled GET, resent GET)");
}

#[test]
fn a_get_after_a_head_answer_that_carried_a_body_succeeds() {
    let srv = server(HEAD_WITH_BODY);
    let agent = build_agent();
    assert_eq!(status_of(&agent, &poison(&srv.base, "HEAD")), 200);
    assert_eq!(status_of(&agent, &request(&srv.base, "GET", b"")), 200);
}

#[test]
fn head_and_options_are_resent_too() {
    for method in ["HEAD", "OPTIONS"] {
        let srv = server(NO_CONTENT_WITH_BODY);
        let agent = build_agent();
        assert_eq!(status_of(&agent, &poison(&srv.base, "GET")), 204);
        assert_eq!(status_of(&agent, &request(&srv.base, method, b"")), 200, "{method}");
    }
}

#[test]
fn a_post_on_such_a_connection_is_not_resent() {
    // The server read the POST (the stray bytes are in front of its answer),
    // so a retry would send it twice (W-POOL4: no duplicate non-idempotent requests).
    let srv = server(NO_CONTENT_WITH_BODY);
    let agent = build_agent();
    assert_eq!(status_of(&agent, &poison(&srv.base, "GET")), 204);
    assert_eq!(status_of(&agent, &request(&srv.base, "POST", b"payload=1")), 502);
    assert_eq!(srv.requests.load(Ordering::SeqCst), 2, "the POST reached the server once");
    assert_eq!(srv.conns.load(Ordering::SeqCst), 1, "no retry opened another connection");
}

#[test]
fn a_garbled_status_line_on_a_new_connection_is_not_resent() {
    // Only a REUSED connection is retried: garbage as the very first answer is the server's own.
    let srv = server(b"hello-worldHTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
    let agent = build_agent();
    assert_eq!(status_of(&agent, &poison(&srv.base, "GET")), 502);
    assert_eq!(srv.conns.load(Ordering::SeqCst), 1);
    assert_eq!(srv.requests.load(Ordering::SeqCst), 1);
}
