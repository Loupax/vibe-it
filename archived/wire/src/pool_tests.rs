// W-POOL3 tests: a request on a stale pooled connection. Loopback servers
// that close the connection after (or without) answering.

use crate::{ParsedRequest, build_agent, execute_and_stream};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use url::Url;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Answer with a Content-Length, then close anyway (keep-alive implied).
    CloseAfterLength,
    /// Answer without Content-Length, then close (body ends at EOF).
    CloseNoLength,
    /// First request of a connection: answer with a Content-Length and keep
    /// the connection. Second: read the whole request (body included), then
    /// close without answering.
    ReadThenDrop,
}

struct Server {
    base: String,
    conns: Arc<AtomicUsize>,
    /// Requests whose head and body the server read completely.
    bodies: Arc<AtomicUsize>,
}

/// Reads one request (head plus Content-Length body); None on EOF.
fn read_request(s: &mut TcpStream) -> Option<usize> {
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
            return Some(cl);
        }
        let n = s.read(&mut b).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&b[..n]);
    }
}

fn server(mode: Mode) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let conns = Arc::new(AtomicUsize::new(0));
    let bodies = Arc::new(AtomicUsize::new(0));
    let (c2, b2) = (Arc::clone(&conns), Arc::clone(&bodies));
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            c2.fetch_add(1, Ordering::SeqCst);
            let b3 = Arc::clone(&b2);
            std::thread::spawn(move || {
                if read_request(&mut s).is_none() {
                    return;
                }
                b3.fetch_add(1, Ordering::SeqCst);
                match mode {
                    Mode::CloseAfterLength => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
                        // Keep the connection open until the next request's first
                        // bytes arrive, then close WITHOUT reading them: the kernel
                        // answers with an RST and the application never saw the
                        // request. (Closing right after the answer instead races
                        // ureq's own liveness check of pooled connections, and
                        // ends in an RST or a clean EOF depending on timing.)
                        let _ = s.peek(&mut [0u8; 1]);
                    }
                    Mode::CloseNoLength => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nok");
                    }
                    Mode::ReadThenDrop => {
                        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
                        if read_request(&mut s).is_some() {
                            b3.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
            });
        }
    });
    Server { base, conns, bodies }
}

fn request(base: &str, method: &str, body: &[u8]) -> ParsedRequest {
    ParsedRequest { method: method.to_string(), url: Url::parse(&format!("{base}/x")).unwrap(), headers: Vec::new(), body: body.to_vec() }
}

/// Status of the answer wire writes for `request` (502 for a transport failure).
fn status_of(agent: &ureq::Agent, r: &ParsedRequest) -> u16 {
    let mut raw = Vec::new();
    execute_and_stream(agent, r, &mut raw).unwrap();
    String::from_utf8_lossy(&raw).split_whitespace().nth(1).unwrap().parse().unwrap()
}

fn failures(srv: &Server, method: &str, n: usize) -> usize {
    let agent = build_agent();
    let r = request(&srv.base, method, if method == "GET" { b"" } else { b"payload=1" });
    (0..n)
        .filter(|_| {
            let mut raw = Vec::new();
            execute_and_stream(&agent, &r, &mut raw).unwrap();
            let t = String::from_utf8_lossy(&raw).to_string();
            let ok = t.starts_with("HTTP/1.1 200");
            if !ok {
                eprintln!("ERR {}", t.lines().last().unwrap_or(""));
            }
            !ok
        })
        .count()
}

#[test]
fn a_thousand_posts_against_a_server_that_closes_after_each_response_all_succeed() {
    let srv = server(Mode::CloseAfterLength);
    assert_eq!(failures(&srv, "POST", 1000), 0);
    // The failed first tries never reached the server's application: it saw
    // every body exactly once (a retry never duplicates a request).
    assert_eq!(srv.bodies.load(Ordering::SeqCst), 1000);
    assert!(srv.conns.load(Ordering::SeqCst) > 1);
}

#[test]
fn puts_with_a_body_and_get_requests_also_survive_stale_connections() {
    let srv = server(Mode::CloseAfterLength);
    assert_eq!(failures(&srv, "PUT", 300), 0);
    assert_eq!(srv.bodies.load(Ordering::SeqCst), 300);
    let srv = server(Mode::CloseAfterLength);
    assert_eq!(failures(&srv, "GET", 300), 0, "GET is retried by ureq itself, unchanged");
    assert_eq!(srv.bodies.load(Ordering::SeqCst), 300);
}

#[test]
fn responses_without_content_length_are_never_pooled() {
    let srv = server(Mode::CloseNoLength);
    assert_eq!(failures(&srv, "POST", 100), 0);
    assert_eq!(srv.conns.load(Ordering::SeqCst), 100, "each POST opened its own connection");
}

#[test]
fn a_post_the_server_read_before_closing_is_not_retried() {
    let srv = server(Mode::ReadThenDrop);
    let agent = build_agent();
    let r = request(&srv.base, "POST", b"payload=1");
    // The first request on a connection is answered and the connection kept;
    // the second one reuses it, the server reads the whole body and closes.
    assert_eq!(status_of(&agent, &r), 200);
    assert_eq!(status_of(&agent, &r), 502, "the client gets the error");
    assert_eq!(srv.bodies.load(Ordering::SeqCst), 2, "the server saw the second body once, not twice");
    assert_eq!(srv.conns.load(Ordering::SeqCst), 1, "no retry opened another connection");
}
