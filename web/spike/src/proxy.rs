//! W6: the loopback proxy that routes WebSockets through wire.
//!
//! Chromium's WebSocket client never asks the resource handler: it opens its own
//! connection in the network service. With `--proxy-server=http://127.0.0.1:<port>`
//! every connection Chromium's network stack makes itself (WebSockets, and anything
//! else that bypassed the handler) arrives here instead, as an HTTP proxy request.
//! The listener is on the loopback of the EMPTY network namespace (run.sh brings `lo`
//! up), so only the CEF process tree can reach it, and the only way out is wire's
//! Unix socket.
//!
//! - `CONNECT host:port` (Chromium tunnels every WebSocket, ws: and wss:, through
//!   CONNECT): we answer 200, then read what Chromium sends inside the tunnel.
//!   - plaintext (ws:) : the HTTP/1.1 upgrade request itself.
//!   - TLS (wss:, or an https request that bypassed the handler): we terminate TLS with
//!     a self-signed certificate for that host. All certificates share one ephemeral
//!     key made at startup; Chromium accepts it through
//!     `--ignore-certificate-errors-spki-list=<sha256 of that key>`. The key never
//!     leaves the process, and no other TLS connection is possible inside the
//!     namespace. wire then does the real TLS to the server.
//! - The inner request is a WebSocket upgrade: it goes to wire's upgrade pipe as
//!   `GET ws[s]://host[:port]/path` + `X-Wire-Upgrade: websocket`; wire's 101 head comes
//!   back (minus `X-Wire-*`) and then bytes are copied both ways until either side
//!   closes. wire never parses frames, nor do we.
//! - Anything else is refused (403 inside the tunnel, or a closed connection) and
//!   logged as `proxy: refused ...`: a census of what still bypasses the handler.

use crate::wire;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub static WS_OPENED: AtomicU64 = AtomicU64::new(0); // upgrades wire answered 101
pub static REFUSED: AtomicU64 = AtomicU64::new(0); // connections refused (not a WebSocket)

const MAX_HEAD: usize = 64 * 1024;
const CHUNK: usize = 16 * 1024;
const TLS_POLL: Duration = Duration::from_millis(10);

/// A parsed HTTP/1.x request head (request line + headers).
#[derive(Debug, Clone, PartialEq)]
pub struct ReqHead {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
}

impl ReqHead {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// A WebSocket opening handshake: GET with `Upgrade: websocket`.
    pub fn is_websocket(&self) -> bool {
        self.method == "GET" && self.header("upgrade").is_some_and(|v| v.trim().eq_ignore_ascii_case("websocket"))
    }
}

/// Reads one request head (up to the blank line). Bytes after it stay in `r`.
pub fn read_request_head(r: &mut impl BufRead) -> io::Result<ReqHead> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut total = 0;
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed before a request"));
    }
    total += line.len();
    let mut parts = line.trim_end_matches(['\r', '\n']).split(' ');
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(bad("bad request line"));
    };
    if !version.starts_with("HTTP/1.") || method.is_empty() || target.is_empty() {
        return Err(bad("bad request line"));
    }
    let (method, target) = (method.to_string(), target.to_string());
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 {
            return Err(bad("eof in request headers"));
        }
        total += h.len();
        if total > MAX_HEAD {
            return Err(bad("request head too large"));
        }
        let h = h.trim_end_matches(['\r', '\n']);
        if h.is_empty() {
            break;
        }
        let (n, v) = h.split_once(':').ok_or_else(|| bad("bad header line"))?;
        headers.push((n.trim().to_string(), v.trim().to_string()));
    }
    Ok(ReqHead { method, target, headers })
}

/// `host:port` of a CONNECT target, split. IPv6 literals keep their brackets.
pub fn split_authority(auth: &str) -> Option<(String, u16)> {
    let (host, port) = auth.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    if host.is_empty() || (host.contains(':') && !host.starts_with('[')) {
        return None;
    }
    Some((host.to_string(), port))
}

/// The WebSocket URL for a handshake that arrived through `CONNECT host:port`.
/// The default port is left out, as in the URL the page used.
pub fn ws_url(tls: bool, host: &str, port: u16, path: &str) -> String {
    let scheme = if tls { "wss" } else { "ws" };
    let path = if path.starts_with('/') { path } else { "/" };
    if port == if tls { 443 } else { 80 } {
        format!("{scheme}://{host}{path}")
    } else {
        format!("{scheme}://{host}:{port}{path}")
    }
}

/// Handshake headers not passed to wire: wire sets `Host`, `Connection` and `Upgrade`
/// itself (we add the two it requires), proxy headers are for us, `X-Wire-*` is the
/// control channel. Everything else (Origin, Cookie from Chromium's jar,
/// Sec-WebSocket-Key/Version/Extensions/Protocol, User-Agent, ...) passes through.
fn skip_handshake_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(n.as_str(), "host" | "connection" | "upgrade" | "content-length" | "transfer-encoding" | "keep-alive")
        || n.starts_with("proxy-")
        || n.starts_with("x-wire-")
}

/// The request for wire's upgrade pipe.
pub fn encode_upgrade(url: &str, head: &ReqHead) -> Vec<u8> {
    let mut out = format!("GET {url} HTTP/1.1\r\n");
    for (n, v) in &head.headers {
        let bad = |s: &str| s.bytes().any(|b| b == b'\r' || b == b'\n');
        if skip_handshake_header(n) || bad(n) || bad(v) {
            continue;
        }
        out.push_str(&format!("{n}: {v}\r\n"));
    }
    out.push_str("Connection: Upgrade\r\nUpgrade: websocket\r\nX-Wire-Upgrade: websocket\r\n\r\n");
    out.into_bytes()
}

/// wire's answer head as Chromium gets it: without `X-Wire-*`. A non-101 answer
/// (wire sends its body without a length and closes) gets `Connection: close`.
pub fn encode_answer_head(head: &wire::WireHead) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {} {}\r\n", head.status, head.reason);
    for (n, v) in head.page_headers() {
        if head.status != 101 && (n.eq_ignore_ascii_case("connection") || n.eq_ignore_ascii_case("content-length")) {
            continue;
        }
        out.push_str(&format!("{n}: {v}\r\n"));
    }
    if head.status != 101 {
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");
    out.into_bytes()
}

/// base64 (standard alphabet, padded), for the SPKI hash switch.
pub fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// The proxy's TLS identity: one key pair, a certificate per host made on demand.
pub struct Tls {
    key: rcgen::KeyPair,
    /// base64(sha256(SubjectPublicKeyInfo)) for `--ignore-certificate-errors-spki-list`.
    pub spki_sha256: String,
    configs: Mutex<HashMap<String, Arc<rustls::ServerConfig>>>,
}

impl Tls {
    pub fn new() -> Result<Tls, String> {
        let key = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
        let spki = key.public_key_der();
        let spki_sha256 = base64(ring::digest::digest(&ring::digest::SHA256, &spki).as_ref());
        Ok(Tls { key, spki_sha256, configs: Mutex::new(HashMap::new()) })
    }

    fn config(&self, host: &str) -> Result<Arc<rustls::ServerConfig>, String> {
        let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
        if let Some(c) = self.configs.lock().unwrap().get(&host) {
            return Ok(Arc::clone(c));
        }
        let mut params = rcgen::CertificateParams::new(vec![host.clone()]).map_err(|e| e.to_string())?;
        params.distinguished_name.push(rcgen::DnType::CommonName, host.clone());
        let cert = params.self_signed(&self.key).map_err(|e| e.to_string())?;
        let key_der = rustls::pki_types::PrivateKeyDer::try_from(self.key.serialize_der()).map_err(|e| e.to_string())?;
        let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(vec![cert.der().clone()], key_der)
            .map_err(|e| e.to_string())?;
        // http/1.1 only: a WebSocket handshake is HTTP/1.1 (RFC 8441 over h2 is not offered).
        let mut cfg = cfg;
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        let cfg = Arc::new(cfg);
        self.configs.lock().unwrap().insert(host, Arc::clone(&cfg));
        Ok(cfg)
    }
}

/// Starts the listener on 127.0.0.1 (an ephemeral port) and returns the port.
pub fn start(wire_socket: Option<String>, tls: Option<Arc<Tls>>) -> io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::Builder::new().name("proxy".into()).spawn(move || {
        let mut n = 0u64;
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            n += 1;
            let (wire_socket, tls) = (wire_socket.clone(), tls.clone());
            let _ = std::thread::Builder::new().name(format!("proxy-{n}")).spawn(move || {
                if let Err(e) = serve(n, conn, wire_socket.as_deref(), tls.as_deref()) {
                    log!("proxy #{n}: {e}");
                }
            });
        }
    })?;
    Ok(port)
}

fn refuse(what: &str) {
    REFUSED.fetch_add(1, Ordering::Relaxed);
    log!("proxy: refused {what}");
}

const FORBIDDEN: &[u8] = b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

fn serve(n: u64, conn: TcpStream, wire_socket: Option<&str>, tls: Option<&Tls>) -> io::Result<()> {
    conn.set_nodelay(true).ok();
    let mut r = BufReader::new(conn.try_clone()?);
    let head = read_request_head(&mut r)?;
    let mut conn = conn;
    if head.method != "CONNECT" {
        // An absolute-form proxy request (plain http: from the network stack).
        refuse(&format!("{} {} (plain proxy request, not a tunnel)", head.method, head.target));
        return conn.write_all(FORBIDDEN);
    }
    let Some((host, port)) = split_authority(&head.target) else {
        refuse(&format!("CONNECT {} (bad target)", head.target));
        return conn.write_all(FORBIDDEN);
    };
    conn.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    // Peek the first byte of the tunnel: 0x16 = a TLS ClientHello.
    let first = r.fill_buf()?.first().copied();
    match first {
        None => {
            refuse(&format!("CONNECT {host}:{port} (closed without a byte: a preconnect?)"));
            Ok(())
        }
        Some(0x16) => {
            let Some(tls) = tls else {
                refuse(&format!("CONNECT {host}:{port} (TLS, and the proxy has no TLS identity)"));
                return Ok(());
            };
            let cfg = tls.config(&host).map_err(io::Error::other)?;
            let sconn = rustls::ServerConnection::new(cfg).map_err(io::Error::other)?;
            // The ClientHello may already sit in `r`'s buffer: hand those bytes to rustls first.
            let pre = r.buffer().to_vec();
            drop(r);
            let stream = rustls::StreamOwned::new(sconn, Prefixed { pre: io::Cursor::new(pre), s: conn });
            let mut br = BufReader::new(stream);
            let inner = match read_request_head(&mut br) {
                Ok(h) => h,
                Err(e) => {
                    refuse(&format!("CONNECT {host}:{port} (TLS: {e})"));
                    return Ok(());
                }
            };
            let path = inner.target.clone();
            if !inner.is_websocket() {
                refuse(&format!("{} https://{host}:{port}{path} (not a WebSocket)", inner.method));
                let mut s = br.into_inner();
                s.write_all(FORBIDDEN)?;
                s.conn.send_close_notify();
                return s.flush();
            }
            let url = ws_url(true, &host, port, &path);
            let rest = br.buffer().to_vec();
            let s = br.into_inner();
            upgrade_tls(n, &url, &inner, rest, s, wire_socket)
        }
        Some(_) => {
            let inner = read_request_head(&mut r)?;
            if !inner.is_websocket() {
                refuse(&format!("{} http://{host}:{port}{} (not a WebSocket)", inner.method, inner.target));
                return conn.write_all(FORBIDDEN);
            }
            let url = ws_url(false, &host, port, &inner.target);
            let rest = r.buffer().to_vec();
            drop(r);
            upgrade_plain(n, &url, &inner, rest, conn, wire_socket)
        }
    }
}

/// A TCP stream that first serves bytes already read from it.
struct Prefixed {
    pre: io::Cursor<Vec<u8>>,
    s: TcpStream,
}

impl Read for Prefixed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.pre.read(buf)?;
        if n > 0 {
            return Ok(n);
        }
        self.s.read(buf)
    }
}

impl Write for Prefixed {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.s.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.s.flush()
    }
}

/// Sends the handshake to wire and reads its answer head. Err: wire unreachable.
fn ask_wire(url: &str, head: &ReqHead, wire_socket: Option<&str>) -> io::Result<(wire::WireHead, BufReader<UnixStream>)> {
    let Some(sock) = wire_socket else { return Err(io::Error::other("WEB_SPIKE_WIRE unset")) };
    let mut w = UnixStream::connect(sock)?;
    w.write_all(&encode_upgrade(url, head))?;
    w.flush()?;
    let mut wr = BufReader::new(w);
    let answer = wire::read_head(&mut wr)?;
    Ok((answer, wr))
}

const BAD_GATEWAY: &[u8] = b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

fn log_answer(n: u64, url: &str, answer: &wire::WireHead) {
    if answer.status == 101 {
        WS_OPENED.fetch_add(1, Ordering::Relaxed);
    }
    log!(
        "proxy #{n}: websocket {url} -> wire {} {}{}",
        answer.status,
        answer.reason,
        answer.wire_error().map(|e| format!(" (X-Wire-Error: {e})")).unwrap_or_default()
    );
}

fn upgrade_plain(n: u64, url: &str, head: &ReqHead, rest: Vec<u8>, mut conn: TcpStream, wire_socket: Option<&str>) -> io::Result<()> {
    let (answer, wr) = match ask_wire(url, head, wire_socket) {
        Ok(x) => x,
        Err(e) => {
            log!("proxy #{n}: websocket {url}: wire: {e}");
            return conn.write_all(BAD_GATEWAY);
        }
    };
    log_answer(n, url, &answer);
    conn.write_all(&encode_answer_head(&answer))?;
    conn.write_all(wr.buffer())?;
    let w = wr.into_inner();
    let mut w_in = w.try_clone()?;
    w_in.write_all(&rest)?;
    // Two blocking copies; either side's EOF shuts the other down.
    let (c2, w2) = (conn.try_clone()?, w.try_clone()?);
    let t = std::thread::spawn(move || {
        copy(c2, &w_in);
        let _ = w_in.shutdown(Shutdown::Write);
    });
    copy(w2, &conn);
    let _ = conn.shutdown(Shutdown::Write);
    let _ = t.join();
    let _ = conn.shutdown(Shutdown::Both);
    let _ = w.shutdown(Shutdown::Both);
    Ok(())
}

fn copy(mut from: impl Read, mut to: impl Write) {
    let mut buf = vec![0u8; CHUNK];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(k) => {
                if to.write_all(&buf[..k]).and_then(|()| to.flush()).is_err() {
                    return;
                }
            }
        }
    }
}

type TlsStream = rustls::StreamOwned<rustls::ServerConnection, Prefixed>;

fn upgrade_tls(n: u64, url: &str, head: &ReqHead, rest: Vec<u8>, mut s: TlsStream, wire_socket: Option<&str>) -> io::Result<()> {
    let (answer, wr) = match ask_wire(url, head, wire_socket) {
        Ok(x) => x,
        Err(e) => {
            log!("proxy #{n}: websocket {url}: wire: {e}");
            s.write_all(BAD_GATEWAY)?;
            return s.flush();
        }
    };
    log_answer(n, url, &answer);
    s.write_all(&encode_answer_head(&answer))?;
    s.write_all(wr.buffer())?;
    s.flush()?;
    let mut w = wr.into_inner();
    w.write_all(&rest)?;
    // rustls keeps plaintext buffers, so one thread owns the TLS stream: it polls the
    // client (short socket timeout) and drains a bounded channel fed by a thread that
    // reads wire. (Same scheme as wire's own wss pipe.)
    let (tx, rx) = sync_channel::<Vec<u8>>(8);
    let mut w_read = w.try_clone()?;
    let reader = std::thread::spawn(move || {
        let mut buf = vec![0u8; CHUNK];
        loop {
            match w_read.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(k) => {
                    if tx.send(buf[..k].to_vec()).is_err() {
                        return;
                    }
                }
            }
        }
    });
    s.sock.s.set_read_timeout(Some(TLS_POLL))?;
    let mut buf = vec![0u8; CHUNK];
    'pipe: loop {
        loop {
            match rx.try_recv() {
                Ok(chunk) => {
                    if s.write_all(&chunk).and_then(|()| s.flush()).is_err() {
                        break 'pipe;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // wire closed: close the TLS side too.
                    s.conn.send_close_notify();
                    let _ = s.flush();
                    break 'pipe;
                }
            }
        }
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => {
                if w.write_all(&buf[..k]).is_err() {
                    break;
                }
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
            Err(_) => break,
        }
    }
    let _ = w.shutdown(Shutdown::Both);
    let _ = s.sock.s.shutdown(Shutdown::Both);
    let _ = reader.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_head_and_rest() {
        let raw = b"GET /chat?x=1 HTTP/1.1\r\nHost: a.test\r\nUpgrade: WebSocket\r\nConnection: Upgrade\r\n\r\nFRAME";
        let mut r = BufReader::new(&raw[..]);
        let h = read_request_head(&mut r).unwrap();
        assert_eq!(h.method, "GET");
        assert_eq!(h.target, "/chat?x=1");
        assert!(h.is_websocket());
        assert_eq!(r.buffer(), b"FRAME");
        let mut r = BufReader::new(&b"CONNECT a.test:443 HTTP/1.1\r\nHost: a.test:443\r\n\r\n"[..]);
        let h = read_request_head(&mut r).unwrap();
        assert_eq!((h.method.as_str(), h.target.as_str()), ("CONNECT", "a.test:443"));
        assert!(!h.is_websocket());
        assert!(read_request_head(&mut BufReader::new(&b"garbage\r\n\r\n"[..])).is_err());
    }

    #[test]
    fn authority_and_url() {
        assert_eq!(split_authority("echo.test:443"), Some(("echo.test".into(), 443)));
        assert_eq!(split_authority("[::1]:8080"), Some(("[::1]".into(), 8080)));
        assert_eq!(split_authority("::1:80"), None);
        assert_eq!(split_authority("nohost"), None);
        assert_eq!(ws_url(true, "echo.test", 443, "/raw"), "wss://echo.test/raw");
        assert_eq!(ws_url(false, "localhost", 18461, "/echo?a"), "ws://localhost:18461/echo?a");
        assert_eq!(ws_url(false, "a.test", 80, "*"), "ws://a.test/");
    }

    #[test]
    fn upgrade_request_for_wire() {
        let head = ReqHead {
            method: "GET".into(),
            target: "/e".into(),
            headers: vec![
                ("Host".into(), "a.test".into()),
                ("Connection".into(), "Upgrade".into()),
                ("Upgrade".into(), "websocket".into()),
                ("Origin".into(), "http://a.test".into()),
                ("Sec-WebSocket-Key".into(), "k==".into()),
                ("Cookie".into(), "a=1".into()),
                ("Proxy-Connection".into(), "keep-alive".into()),
                ("X-Wire-Jar".into(), "scratchy".into()),
                ("Evil".into(), "x\r\nX-Wire-Policy: off".into()),
            ],
        };
        let out = String::from_utf8(encode_upgrade("ws://a.test/e", &head)).unwrap();
        assert!(out.starts_with("GET ws://a.test/e HTTP/1.1\r\n"));
        for want in ["Origin: http://a.test\r\n", "Sec-WebSocket-Key: k==\r\n", "Cookie: a=1\r\n", "X-Wire-Upgrade: websocket\r\n", "Upgrade: websocket\r\n"] {
            assert!(out.contains(want), "{want} missing in {out}");
        }
        for not in ["Host:", "Proxy-", "X-Wire-Jar", "Evil", "X-Wire-Policy"] {
            assert!(!out.contains(not), "{not} leaked into {out}");
        }
        assert_eq!(out.matches("Connection:").count(), 1);
        assert!(out.ends_with("\r\n\r\n"));
    }

    #[test]
    fn answer_head_hides_wire_headers() {
        let h = wire::WireHead {
            status: 101,
            reason: "Switching Protocols".into(),
            headers: vec![
                ("Upgrade".into(), "websocket".into()),
                ("Connection".into(), "Upgrade".into()),
                ("Sec-WebSocket-Accept".into(), "abc".into()),
                ("X-Wire-Final-Url".into(), "ws://a/".into()),
            ],
        };
        let s = String::from_utf8(encode_answer_head(&h)).unwrap();
        assert_eq!(s, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: abc\r\n\r\n");
        let h = wire::WireHead { status: 502, reason: "Bad Gateway".into(), headers: vec![("X-Wire-Error".into(), "1".into())] };
        assert_eq!(String::from_utf8(encode_answer_head(&h)).unwrap(), "HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n");
    }

    #[test]
    fn base64_matches_rfc4648() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn tls_identity_makes_per_host_configs() {
        let t = Tls::new().unwrap();
        assert_eq!(t.spki_sha256.len(), 44);
        let a = t.config("echo.test").unwrap();
        let b = t.config("ECHO.test").unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(t.config("[::1]").is_ok());
    }
}
