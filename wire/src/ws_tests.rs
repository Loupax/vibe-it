// QF19w1 tests: a local echo WebSocket server (std only; SHA-1 and base64 by
// hand for the RFC 6455 accept key) behind a real wire handler on a unix socket.
// wire never parses frames, so the "frames" here are arbitrary bytes.

use super::*;
use crate::{build_agent, handle_connection};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};

/// Extra trust root (DER) for the wss test only.
pub static EXTRA_ROOT: Mutex<Option<Vec<u8>>> = Mutex::new(None);

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[test]
fn sha1_and_base64_match_rfc6455_example() {
    assert_eq!(b64(&sha1(b"dGhlIHNhbXBsZSBub25jZQ==258EAFA5-E914-47DA-95CA-C5AB0DC85B11")), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

fn read_head_text(s: &mut impl Read) -> String {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if s.read(&mut b).unwrap_or(0) == 0 {
            break;
        }
        buf.push(b[0]);
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Serves the post-handshake part: echo until EOF (or close after the first
/// message when `close_after_one`).
fn echo(mut s: impl Read + Write, close_after_one: bool) {
    let mut buf = [0u8; 4096];
    loop {
        let n = s.read(&mut buf).unwrap_or(0);
        if n == 0 {
            return;
        }
        if s.write_all(&buf[..n]).and_then(|()| s.flush()).is_err() || close_after_one {
            return;
        }
    }
}

fn answer_handshake(head: &str, s: &mut (impl Read + Write)) -> Option<bool> {
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
    let key = head.lines().find_map(|l| l.strip_prefix("Sec-WebSocket-Key: ").or_else(|| l.strip_prefix("sec-websocket-key: "))).unwrap_or("").trim().to_string();
    let accept = b64(&sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()));
    let switch = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSet-Cookie: ws=1; Path=/\r\nX-Wire-Spoof: 1\r\n\r\n"
    );
    match path.as_str() {
        "/echo" | "/srvclose" => {
            s.write_all(switch.as_bytes()).ok()?;
            s.flush().ok()?;
            Some(path == "/srvclose")
        }
        "/plain" => {
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope").ok()?;
            None
        }
        "/redir" => {
            s.write_all(b"HTTP/1.1 302 Found\r\nLocation: /echo\r\nSet-Cookie: r=1; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").ok()?;
            None
        }
        "/chunked" => {
            s.write_all(b"HTTP/1.1 400 Bad Request\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n").ok()?;
            None
        }
        _ => None,
    }
}

struct WsServer {
    addr: SocketAddr,
    conns: Arc<AtomicUsize>,
    heads: Receiver<String>,
}

fn ws_server() -> WsServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let conns = Arc::new(AtomicUsize::new(0));
    let (tx, heads) = channel();
    let c2 = Arc::clone(&conns);
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            c2.fetch_add(1, Ordering::SeqCst);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut s = s;
                let head = read_head_text(&mut s);
                let _ = tx.send(head.clone());
                if let Some(close_after_one) = answer_handshake(&head, &mut s) {
                    echo(s, close_after_one);
                }
            });
        }
    });
    WsServer { addr, conns, heads }
}

fn wire_socket(shared: Arc<Shared>) -> std::path::PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("wire-ws-test-{}-{}.sock", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        let agent = build_agent();
        for s in l.incoming().flatten() {
            let (a, sh) = (agent.clone(), Arc::clone(&shared));
            std::thread::spawn(move || handle_connection(s, &a, &sh));
        }
    });
    path
}

/// Sends a handshake through wire; returns (stream, response head).
fn handshake(sock: &std::path::Path, url: &str, extra: &[(&str, &str)]) -> (UnixStream, String) {
    let mut c = UnixStream::connect(sock).unwrap();
    let mut req = format!(
        "GET {url} HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nOrigin: https://page.example\r\nX-Wire-Upgrade: websocket\r\n"
    );
    for (n, v) in extra {
        req.push_str(&format!("{n}: {v}\r\n"));
    }
    req.push_str("\r\n");
    c.write_all(req.as_bytes()).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let head = read_head_text(&mut c);
    (c, head)
}

fn lc_has(head: &str, needle: &str) -> bool {
    head.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}

#[test]
fn upgrade_returns_101_and_echoes_both_ways() {
    let srv = ws_server();
    let sock = wire_socket(Arc::new(Shared::new(true)));
    let (mut c, head) = handshake(&sock, &format!("ws://{}/echo", srv.addr), &[("X-Wire-Policy", "none")]);
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    assert!(lc_has(&head, "sec-websocket-accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "{head}");
    assert!(lc_has(&head, "x-wire-version: 3") && lc_has(&head, "x-wire-features: ua,ws"), "{head}");
    assert!(lc_has(&head, "x-wire-target-class: loopback"), "{head}");
    assert!(!lc_has(&head, "x-wire-spoof"), "upstream x-wire-* must not reach the client: {head}");
    for msg in [&b"hello"[..], &[0u8, 255, 1, 2, 3][..], b"third"] {
        c.write_all(msg).unwrap();
        let mut got = vec![0u8; msg.len()];
        c.read_exact(&mut got).unwrap();
        assert_eq!(got, msg);
    }
    let up = srv.heads.recv().unwrap();
    assert!(up.starts_with("GET /echo HTTP/1.1\r\n"), "{up}");
    assert!(lc_has(&up, "connection: upgrade") && lc_has(&up, "upgrade: websocket"), "{up}");
    assert!(lc_has(&up, "origin: https://page.example") && lc_has(&up, "sec-websocket-key: dGhlIHNhbXBsZSBub25jZQ=="), "{up}");
    assert!(lc_has(&up, "sec-websocket-version: 13"), "{up}");
    assert!(!lc_has(&up, "x-wire-"), "control header leaked upstream: {up}");
    assert!(lc_has(&up, &format!("host: {}", srv.addr)), "{up}");
}

#[test]
fn a_megabyte_goes_through_both_ways_intact() {
    let srv = ws_server();
    let sock = wire_socket(Arc::new(Shared::new(true)));
    let (c, head) = handshake(&sock, &format!("ws://{}/echo", srv.addr), &[]);
    assert!(head.starts_with("HTTP/1.1 101 "));
    let data: Vec<u8> = (0..1_048_576u32).map(|i| (i.wrapping_mul(31) >> 3) as u8).collect();
    let mut w = c.try_clone().unwrap();
    let d2 = data.clone();
    let writer = std::thread::spawn(move || w.write_all(&d2).unwrap());
    let mut got = vec![0u8; data.len()];
    let mut r = c;
    r.read_exact(&mut got).unwrap();
    writer.join().unwrap();
    assert!(got == data);
}

#[test]
fn non_101_answers_are_buffered_responses_and_the_connection_closes() {
    let srv = ws_server();
    let sock = wire_socket(Arc::new(Shared::new(true)));
    let (mut c, head) = handshake(&sock, &format!("ws://{}/plain", srv.addr), &[]);
    assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
    let mut body = Vec::new();
    c.read_to_end(&mut body).unwrap();
    assert_eq!(body, b"nope");
    // A 3xx is not followed: it is the answer.
    let (mut c, head) = handshake(&sock, &format!("ws://{}/redir", srv.addr), &[]);
    assert!(head.starts_with("HTTP/1.1 302 "), "{head}");
    assert!(lc_has(&head, "location: /echo") && lc_has(&head, "set-cookie: r=1"), "{head}");
    let mut rest = Vec::new();
    c.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty());
    assert_eq!(srv.conns.load(Ordering::SeqCst), 2, "the redirect must not have been followed");
    // A chunked error body is decoded.
    let (mut c, head) = handshake(&sock, &format!("ws://{}/chunked", srv.addr), &[]);
    assert!(head.starts_with("HTTP/1.1 400 "), "{head}");
    assert!(!lc_has(&head, "transfer-encoding"), "{head}");
    let mut body = Vec::new();
    c.read_to_end(&mut body).unwrap();
    assert_eq!(body, b"abcde");
}

#[test]
fn public_only_refuses_a_loopback_target_and_never_connects() {
    let srv = ws_server();
    let sock = wire_socket(Arc::new(Shared::new(true)));
    let (mut c, head) = handshake(&sock, &format!("ws://{}/echo", srv.addr), &[("X-Wire-Policy", "public-only")]);
    assert!(head.starts_with("HTTP/1.1 403 "), "{head}");
    assert!(lc_has(&head, "x-wire-error: policy"), "{head}");
    let mut body = Vec::new();
    c.read_to_end(&mut body).unwrap();
    assert!(String::from_utf8_lossy(&body).contains("blocked"));
    assert_eq!(srv.conns.load(Ordering::SeqCst), 0);
    // file: under public-only is refused as for any request.
    let (_c, head) = handshake(&sock, "file:///etc/passwd", &[("X-Wire-Policy", "public-only")]);
    assert!(head.starts_with("HTTP/1.1 403 "), "{head}");
}

#[test]
fn a_plain_request_that_pooled_a_loopback_connection_does_not_bypass_public_only() {
    // W-POOL1: the ordinary pool may hold a loopback connection; the WebSocket
    // path never uses a pool, so the resolver check still runs.
    let srv = ws_server();
    let shared = Arc::new(Shared::new(true));
    let sock = wire_socket(Arc::clone(&shared));
    let mut c = UnixStream::connect(&sock).unwrap();
    write!(c, "GET http://{}/plain HTTP/1.1\r\n\r\n", srv.addr).unwrap();
    let mut sink = Vec::new();
    c.read_to_end(&mut sink).unwrap();
    let (_c, head) = handshake(&sock, &format!("ws://{}/echo", srv.addr), &[("X-Wire-Policy", "public-only")]);
    assert!(head.starts_with("HTTP/1.1 403 "), "{head}");
}

#[test]
fn bad_upgrade_requests_are_400() {
    let sock = wire_socket(Arc::new(Shared::new(true)));
    let (_c, head) = handshake(&sock, "ws://127.0.0.1:1/x", &[]);
    assert!(head.starts_with("HTTP/1.1 502 "), "connection refused is a 502: {head}");
    let mut c = UnixStream::connect(&sock).unwrap();
    write!(c, "GET ws://127.0.0.1:1/x HTTP/1.1\r\nX-Wire-Upgrade: banana\r\n\r\n").unwrap();
    let head = read_head_text(&mut c);
    assert!(head.starts_with("HTTP/1.1 400 ") && lc_has(&head, "x-wire-error: bad-upgrade"), "{head}");
    let mut c = UnixStream::connect(&sock).unwrap();
    write!(c, "POST ws://127.0.0.1:1/x HTTP/1.1\r\nX-Wire-Upgrade: websocket\r\n\r\n").unwrap();
    let head = read_head_text(&mut c);
    assert!(head.starts_with("HTTP/1.1 400 ") && lc_has(&head, "x-wire-error: bad-upgrade"), "{head}");
    let mut c = UnixStream::connect(&sock).unwrap();
    write!(c, "GET ws://127.0.0.1:1/x HTTP/1.1\r\nX-Wire-Upgrade: websocket\r\nX-Wire-User-Agent: a\x01b\r\n\r\n").unwrap();
    let head = read_head_text(&mut c);
    assert!(head.starts_with("HTTP/1.1 400 ") && lc_has(&head, "x-wire-error: bad-user-agent"), "{head}");
}

#[test]
fn without_x_wire_upgrade_an_upgrade_request_behaves_as_before() {
    let srv = ws_server();
    let sock = wire_socket(Arc::new(Shared::new(true)));
    let mut c = UnixStream::connect(&sock).unwrap();
    write!(c, "GET http://{}/plain HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n", srv.addr).unwrap();
    let mut all = Vec::new();
    c.read_to_end(&mut all).unwrap();
    let all = String::from_utf8_lossy(&all).to_string();
    assert!(all.starts_with("HTTP/1.1 200 ") && all.ends_with("nope"), "{all}");
    assert!(!lc_has(&all, "x-wire-error"));
}

#[test]
fn close_from_the_client_side_ends_the_upstream_connection() {
    let srv = ws_server();
    let sock = wire_socket(Arc::new(Shared::new(true)));
    let (mut c, head) = handshake(&sock, &format!("ws://{}/echo", srv.addr), &[]);
    assert!(head.starts_with("HTTP/1.1 101 "));
    c.write_all(b"bye").unwrap();
    let mut got = [0u8; 3];
    c.read_exact(&mut got).unwrap();
    // Half-close: upstream sees EOF, finishes, wire closes the client side.
    c.shutdown(Shutdown::Write).unwrap();
    let mut rest = Vec::new();
    c.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty());
}

#[test]
fn close_from_the_server_side_closes_the_client_connection() {
    let srv = ws_server();
    let sock = wire_socket(Arc::new(Shared::new(true)));
    let (mut c, head) = handshake(&sock, &format!("ws://{}/srvclose", srv.addr), &[]);
    assert!(head.starts_with("HTTP/1.1 101 "));
    c.write_all(b"one").unwrap();
    let mut got = Vec::new();
    c.read_to_end(&mut got).unwrap(); // the server echoes once, then closes: EOF
    assert_eq!(got, b"one");
}

#[test]
fn cookies_are_sent_on_the_handshake_and_set_cookie_is_stored_and_stripped() {
    let srv = ws_server();
    let shared = Arc::new(Shared::new(true));
    let sock = wire_socket(Arc::clone(&shared));
    let url = format!("ws://{}/echo", srv.addr);
    let jar = [("X-Wire-Jar", "scratchy"), ("Cookie", "client=ignored")];
    let (c, head) = handshake(&sock, &url, &jar);
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    assert!(lc_has(&head, "x-wire-cookies: on"), "{head}");
    assert!(!lc_has(&head, "set-cookie"), "Set-Cookie belongs to the jar: {head}");
    let first = srv.heads.recv().unwrap();
    assert!(!lc_has(&first, "cookie:"), "nothing stored yet and the client Cookie is dropped: {first}");
    drop(c);
    let (_c, _) = handshake(&sock, &url, &jar);
    let second = srv.heads.recv().unwrap();
    assert!(lc_has(&second, "cookie: ws=1"), "{second}");
    assert!(!lc_has(&second, "client=ignored"), "{second}");
    // Without the jar header nothing is sent or stored, and Set-Cookie passes through.
    let (_c, head) = handshake(&sock, &url, &[]);
    assert!(lc_has(&head, "set-cookie: ws=1"), "{head}");
    assert!(!lc_has(&srv.heads.recv().unwrap(), "cookie:"));
    // A 302's Set-Cookie is stored too.
    let (_c, head) = handshake(&sock, &format!("ws://{}/redir", srv.addr), &jar);
    assert!(head.starts_with("HTTP/1.1 302 ") && !lc_has(&head, "set-cookie"), "{head}");
    let _ = srv.heads.recv().unwrap();
    let (_c, _) = handshake(&sock, &url, &jar);
    assert!(lc_has(&srv.heads.recv().unwrap(), "r=1"));
}

#[test]
fn user_agent_is_honoured_like_for_any_request() {
    let srv = ws_server();
    let sock = wire_socket(Arc::new(Shared::new(true)));
    let url = format!("ws://{}/echo", srv.addr);
    let ua_of = |h: &str| h.lines().find_map(|l| l.strip_prefix("User-Agent: ")).map(str::to_string);
    let _a = handshake(&sock, &url, &[]);
    assert!(ua_of(&srv.heads.recv().unwrap()).unwrap().starts_with("wire/0.1"));
    let _b = handshake(&sock, &url, &[("User-Agent", "OwnUA/1")]);
    assert_eq!(ua_of(&srv.heads.recv().unwrap()).as_deref(), Some("OwnUA/1"));
    let _c = handshake(&sock, &url, &[("User-Agent", "OwnUA/1"), ("X-Wire-User-Agent", "Wins/2")]);
    let up = srv.heads.recv().unwrap();
    assert_eq!(ua_of(&up).as_deref(), Some("Wins/2"));
    assert_eq!(up.to_ascii_lowercase().matches("user-agent:").count(), 1, "{up}");
}

#[test]
fn every_handshake_is_its_own_upstream_connection() {
    let srv = ws_server();
    let sock = wire_socket(Arc::new(Shared::new(true)));
    for _ in 0..3 {
        let (mut c, head) = handshake(&sock, &format!("ws://{}/echo", srv.addr), &[]);
        assert!(head.starts_with("HTTP/1.1 101 "));
        c.write_all(b"x").unwrap();
        let mut b = [0u8; 1];
        c.read_exact(&mut b).unwrap();
        drop(c);
    }
    // 3 handshakes -> 3 distinct upstream connections, one handshake each.
    assert_eq!(srv.conns.load(Ordering::SeqCst), 3);
    let heads: Vec<String> = std::iter::from_fn(|| srv.heads.try_recv().ok()).collect();
    assert_eq!(heads.iter().filter(|h| h.starts_with("GET /echo ")).count(), 3);
}

#[test]
fn timeout_covers_the_handshake_only_not_idleness() {
    // Never answers: the handshake times out as a 502.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = l.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for s in l.incoming().flatten() {
            held.push(s);
        }
    });
    let sock = wire_socket(Arc::new(Shared::new(true)));
    let t = Instant::now();
    let (_c, head) = handshake(&sock, &format!("ws://{dead}/x"), &[("X-Wire-Timeout-Ms", "300")]);
    assert!(head.starts_with("HTTP/1.1 502 ") && t.elapsed() < Duration::from_secs(5), "{head}");
    // An established socket idles past the timeout and still works.
    let srv = ws_server();
    let (mut c, head) = handshake(&sock, &format!("ws://{}/echo", srv.addr), &[("X-Wire-Timeout-Ms", "300")]);
    assert!(head.starts_with("HTTP/1.1 101 "));
    std::thread::sleep(Duration::from_millis(900));
    c.write_all(b"late").unwrap();
    let mut b = [0u8; 4];
    c.read_exact(&mut b).unwrap();
    assert_eq!(&b, b"late");
}

fn have_openssl() -> bool {
    std::process::Command::new("openssl").arg("version").output().is_ok_and(|o| o.status.success())
}

#[test]
fn wss_goes_through_tls_when_the_server_cert_is_trusted() {
    if !have_openssl() {
        eprintln!("skipping wss test: no openssl binary to make a test certificate");
        return;
    }
    let dir = std::env::temp_dir().join(format!("wire-wss-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (cert, key, key8) = (dir.join("c.der"), dir.join("k.pem"), dir.join("k8.der"));
    let pem = dir.join("c.pem");
    let run = |args: &[&str]| std::process::Command::new("openssl").args(args).output().unwrap().status.success();
    let ok = run(&[
        "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", key.to_str().unwrap(), "-out", pem.to_str().unwrap(), "-days", "2",
        "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1", "-addext", "basicConstraints=critical,CA:FALSE",
    ]) && run(&["x509", "-in", pem.to_str().unwrap(), "-outform", "DER", "-out", cert.to_str().unwrap()])
        && run(&["pkcs8", "-topk8", "-nocrypt", "-in", key.to_str().unwrap(), "-outform", "DER", "-out", key8.to_str().unwrap()]);
    assert!(ok, "openssl could not make the test certificate");
    let cert_der = std::fs::read(&cert).unwrap();
    let key_der = std::fs::read(&key8).unwrap();
    *EXTRA_ROOT.lock().unwrap() = Some(cert_der.clone());

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server_cfg = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![rustls::pki_types::CertificateDer::from(cert_der)], rustls::pki_types::PrivateKeyDer::try_from(key_der).unwrap())
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let cfg = Arc::clone(&server_cfg);
            std::thread::spawn(move || {
                let conn = rustls::ServerConnection::new(cfg).unwrap();
                let mut s = rustls::StreamOwned::new(conn, tcp);
                let head = read_head_text(&mut s);
                if answer_handshake(&head, &mut s).is_some() {
                    echo(s, false);
                }
            });
        }
    });

    let sock = wire_socket(Arc::new(Shared::new(true)));
    let (mut c, head) = handshake(&sock, &format!("wss://{addr}/echo"), &[]);
    assert!(head.starts_with("HTTP/1.1 101 "), "{head}");
    for msg in [&b"over tls"[..], b"again"] {
        c.write_all(msg).unwrap();
        let mut got = vec![0u8; msg.len()];
        c.read_exact(&mut got).unwrap();
        assert_eq!(got, msg);
    }
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let mut w = c.try_clone().unwrap();
    let d2 = data.clone();
    let writer = std::thread::spawn(move || w.write_all(&d2).unwrap());
    let mut got = vec![0u8; data.len()];
    c.read_exact(&mut got).unwrap();
    writer.join().unwrap();
    assert!(got == data);
    c.shutdown(Shutdown::Write).unwrap();
    let mut rest = Vec::new();
    c.read_to_end(&mut rest).unwrap();
    // An untrusted certificate (webpki roots only) is a TLS failure, not a pipe.
    *EXTRA_ROOT.lock().unwrap() = None;
    let (_c, head) = handshake(&sock, &format!("wss://{addr}/echo"), &[]);
    assert!(head.starts_with("HTTP/1.1 502 "), "{head}");
    let _ = std::fs::remove_dir_all(&dir);
}
