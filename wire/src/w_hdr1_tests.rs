// W-HDR1 tests: a request that repeats a header reaches the server with every value, on the
// shared-agent path, the jar path and the WebSocket handshake; repeated response headers
// (three Set-Cookie lines) reach the client.

use crate::{ParsedRequest, Shared, build_agent, execute_and_stream, execute_with, handle_connection};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;
use url::Url;

/// Answers every request with a body that is the received request head, and three Set-Cookie
/// lines. Also forwards each head on the channel.
fn echo_server() -> (SocketAddr, Receiver<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut s = s;
                let mut buf = Vec::new();
                let mut b = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut b) {
                        Ok(0) | Err(_) => return,
                        Ok(k) => buf.extend_from_slice(&b[..k]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let _ = tx.send(head.clone());
                let out = format!(
                    "HTTP/1.1 200 OK\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2; Path=/\r\nSet-Cookie: c=3; HttpOnly\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{head}",
                    head.len()
                );
                let _ = s.write_all(out.as_bytes());
            });
        }
    });
    (addr, rx)
}

fn req(addr: SocketAddr, headers: &[(&str, &str)]) -> ParsedRequest {
    ParsedRequest {
        method: "GET".into(),
        url: Url::parse(&format!("http://{addr}/")).unwrap(),
        headers: headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        body: Vec::new(),
    }
}

/// Values of header `name` in a head, one entry per header line.
fn values(head: &str, name: &str) -> Vec<String> {
    head.lines().skip(1).filter_map(|l| l.split_once(':')).filter(|(n, _)| n.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()).collect()
}

fn dup_headers() -> Vec<(&'static str, &'static str)> {
    vec![("Cookie", "a=1"), ("Accept", "text/html"), ("Cookie", "b=2"), ("Accept", "application/json"), ("X-Wire-Spoof", "1"), ("X-Custom", "one"), ("X-Custom", "two")]
}

/// All values of `name` the server saw, split on the merge separator, order kept.
fn flat(head: &str, name: &str, sep: &str) -> Vec<String> {
    values(head, name).iter().flat_map(|v| v.split(sep).map(str::to_string).collect::<Vec<_>>()).collect()
}

#[test]
fn repeated_request_headers_all_reach_the_server_on_the_shared_agent_path() {
    let (addr, heads) = echo_server();
    let mut raw = Vec::new();
    execute_and_stream(&build_agent(), &req(addr, &dup_headers()), &mut raw).unwrap();
    let head = heads.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(flat(&head, "cookie", "; "), vec!["a=1", "b=2"], "{head}");
    assert_eq!(flat(&head, "accept", ", "), vec!["text/html", "application/json"], "{head}");
    assert_eq!(flat(&head, "x-custom", ", "), vec!["one", "two"], "{head}");
    assert!(!head.to_ascii_lowercase().contains("x-wire-"), "{head}");
}

#[test]
fn repeated_request_headers_all_reach_the_server_on_the_jar_path() {
    let (addr, heads) = echo_server();
    let mut h = dup_headers();
    h.push(("X-Wire-Jar", "scratchy"));
    let shared = Shared::new(true);
    let mut raw = Vec::new();
    execute_with(&build_agent(), &shared, &req(addr, &h), &mut raw).unwrap();
    let head = heads.recv_timeout(Duration::from_secs(5)).unwrap();
    // On the jar path the client's Cookie is replaced by the jar's (empty here).
    assert!(values(&head, "cookie").is_empty(), "{head}");
    assert_eq!(flat(&head, "accept", ", "), vec!["text/html", "application/json"], "{head}");
    assert_eq!(flat(&head, "x-custom", ", "), vec!["one", "two"], "{head}");
    assert!(!head.to_ascii_lowercase().contains("x-wire-"), "{head}");
}

#[test]
fn repeated_request_headers_all_reach_the_server_on_a_websocket_handshake() {
    let (addr, heads) = echo_server();
    let sock = std::env::temp_dir().join(format!("wire-hdr1-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let l = UnixListener::bind(&sock).unwrap();
    std::thread::spawn(move || {
        let agent = build_agent();
        let shared = Shared::new(true);
        for s in l.incoming().flatten() {
            handle_connection(s, &agent, &shared);
        }
    });
    let mut c = UnixStream::connect(&sock).unwrap();
    let mut r = format!("GET ws://{addr}/ HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nX-Wire-Upgrade: websocket\r\n");
    for (n, v) in dup_headers() {
        r.push_str(&format!("{n}: {v}\r\n"));
    }
    r.push_str("\r\n");
    c.write_all(r.as_bytes()).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let head = heads.recv_timeout(Duration::from_secs(5)).unwrap();
    let _ = std::fs::remove_file(&sock);
    assert_eq!(flat(&head, "cookie", "; "), vec!["a=1", "b=2"], "{head}");
    assert_eq!(flat(&head, "accept", ", "), vec!["text/html", "application/json"], "{head}");
    assert!(!head.to_ascii_lowercase().contains("x-wire-"), "{head}");
}

#[test]
fn every_set_cookie_line_reaches_the_client() {
    let (addr, _heads) = echo_server();
    let mut raw = Vec::new();
    execute_and_stream(&build_agent(), &req(addr, &[]), &mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).to_string();
    let end = text.find("\r\n\r\n").unwrap();
    assert_eq!(values(&text[..end], "set-cookie"), vec!["a=1", "b=2; Path=/", "c=3; HttpOnly"], "{text}");
}
