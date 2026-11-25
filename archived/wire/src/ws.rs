// QF19w1: the WebSocket pipe. A request carrying the control header
// `X-Wire-Upgrade: websocket` goes through the same checks as any request
// (public-only resolver, class headers, cookie jar, User-Agent), wire opens its
// own connection (TLS for `wss:`), sends the handshake and, when upstream
// answers `101`, returns that head and then copies bytes both ways on the unix
// socket until either side closes. wire never parses a WebSocket frame: the
// framing lives in scratchy. Any other status is an ordinary buffered answer.
//
// Not pooled (a fresh connection through the checking resolver, W-POOL1 safe),
// no redirects followed (a 3xx is returned as is), `X-Wire-Timeout-Ms` covers
// the handshake only, there is no idle limit.

use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use url::Url;

use crate::{
    COOKIES_HDR, Caps, DEFAULT_TIMEOUT, DEFAULT_UA, JAR_NAME, ParsedRequest, Shared, TRACKER, Tracker, TrackingResolver, class_rank, cookies, header,
    forwardable_request_headers, now_unix, parse_caps, resolve_user_agent, timing_enabled, write_bad_ua_response, write_policy_response,
    write_status_and_headers, write_synthetic_error_response,
};

/// Longest handshake answer head accepted from upstream.
const MAX_HEAD: usize = 64 * 1024;
/// Buffered cap for a non-101 body when the client sent no `X-Wire-Max-Bytes`.
const DEFAULT_BODY_CAP: usize = 4 * 1024 * 1024;
/// After one side closed, how long the other direction may keep draining.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
/// TLS pipe: how often the single owning thread looks at both directions.
const TLS_POLL: Duration = Duration::from_millis(10);
const CHUNK: usize = 16 * 1024;

/// The upstream connection: plain TCP or TLS over TCP.
enum Upstream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Upstream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Upstream::Plain(s) => s,
            Upstream::Tls(s) => &s.sock,
        }
    }
}

impl Read for Upstream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Upstream::Plain(s) => s.read(buf),
            Upstream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Upstream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Upstream::Plain(s) => s.write(buf),
            Upstream::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Upstream::Plain(s) => s.flush(),
            Upstream::Tls(s) => s.flush(),
        }
    }
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, rustls::Error> {
    #[cfg_attr(not(test), allow(unused_mut))]
    let mut roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    #[cfg(test)]
    if let Some(der) = tests::EXTRA_ROOT.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        let _ = roots.add(rustls::pki_types::CertificateDer::from(der));
    }
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() { Err(io::Error::new(io::ErrorKind::TimedOut, "handshake timed out")) } else { Ok(left) }
}

/// A reader over the upstream that serves `prefix` first and applies the
/// handshake deadline to every read.
struct DeadlineReader<'a> {
    prefix: Cursor<Vec<u8>>,
    up: &'a mut Upstream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.prefix.read(buf)?;
        if n > 0 {
            return Ok(n);
        }
        self.up.tcp().set_read_timeout(Some(remaining(self.deadline)?))?;
        self.up.read(buf)
    }
}

/// Reads up to and including the blank line; returns (head text, bytes read
/// beyond it).
fn read_head(up: &mut Upstream, deadline: Instant) -> io::Result<(String, Vec<u8>)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        up.tcp().set_read_timeout(Some(remaining(deadline)?))?;
        let n = up.read(&mut chunk)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "upstream closed during the handshake"));
        }
        let from = buf.len().saturating_sub(3);
        buf.extend_from_slice(&chunk[..n]);
        if let Some(p) = buf[from..].windows(4).position(|w| w == b"\r\n\r\n") {
            let end = from + p + 4;
            let rest = buf.split_off(end);
            return Ok((String::from_utf8_lossy(&buf[..end - 4]).to_string(), rest));
        }
        if buf.len() > MAX_HEAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "handshake answer head too large"));
        }
    }
}

struct Head {
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
}

fn parse_head(text: &str) -> io::Result<Head> {
    let mut lines = text.split("\r\n");
    let line = lines.next().unwrap_or("");
    let mut parts = line.splitn(3, ' ');
    let (Some(ver), Some(code)) = (parts.next(), parts.next()) else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad status line from upstream"));
    };
    let status: u16 = code.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad status code from upstream"))?;
    if !ver.starts_with("HTTP/1.") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "upstream did not answer HTTP/1.x"));
    }
    let headers = lines
        .filter_map(|l| l.split_once(':').map(|(n, v)| (n.trim().to_string(), v.trim().to_string())))
        .collect();
    Ok(Head { status, reason: parts.next().unwrap_or("").to_string(), headers })
}

/// Decodes a chunked body into `out`, stopping at `cap + 1` bytes.
fn read_chunked(r: &mut impl BufRead, out: &mut Vec<u8>, cap: usize) -> io::Result<()> {
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof in chunked body"));
        }
        let size = line.trim().split(';').next().unwrap_or("");
        let size = usize::from_str_radix(size, 16).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
        if size == 0 {
            return Ok(());
        }
        if out.len() + size > cap {
            out.resize(cap + 1, 0);
            return Ok(());
        }
        let start = out.len();
        out.resize(start + size, 0);
        r.read_exact(&mut out[start..])?;
        let mut crlf = [0u8; 2];
        r.read_exact(&mut crlf)?;
    }
}

fn read_body(up: &mut Upstream, rest: Vec<u8>, head: &Head, deadline: Instant, cap: usize) -> io::Result<Vec<u8>> {
    let mut r = BufReader::new(DeadlineReader { prefix: Cursor::new(rest), up, deadline });
    let mut out = Vec::new();
    if header(&head.headers, "transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        read_chunked(&mut r, &mut out, cap)?;
    } else if let Some(n) = header(&head.headers, "content-length").and_then(|v| v.parse::<u64>().ok()) {
        (&mut r).take(n.min(cap as u64 + 1)).read_to_end(&mut out)?;
        if (out.len() as u64) < n.min(cap as u64 + 1) {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "upstream body shorter than Content-Length"));
        }
    } else {
        (&mut r).take(cap as u64 + 1).read_to_end(&mut out)?;
    }
    Ok(out)
}

fn bad_upgrade(w: &mut impl Write, msg: &str) -> io::Result<u16> {
    write_status_and_headers(
        w,
        400,
        "Bad Request",
        &[("Content-Type".to_string(), "text/plain".to_string()), ("X-Wire-Error".to_string(), "bad-upgrade".to_string())],
    )?;
    w.write_all(format!("wire: {msg}").as_bytes())?;
    w.flush()?;
    Ok(400)
}

/// Resolves through the checking resolver and connects; the first address that
/// connects wins. `Err` carries whether the policy refused.
fn connect(url: &Url, caps: &Caps, deadline: Instant) -> Result<Upstream, io::Error> {
    let host = url.host_str().unwrap_or("");
    let tls = matches!(url.scheme(), "https");
    let port = url.port_or_known_default().unwrap_or(if tls { 443 } else { 80 });
    let addrs: Vec<SocketAddr> = ureq::Resolver::resolve(&TrackingResolver, &format!("{host}:{port}"))?;
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
    for a in addrs {
        match TcpStream::connect_timeout(&a, remaining(deadline)?.min(caps.timeout.unwrap_or(DEFAULT_TIMEOUT))) {
            Ok(tcp) => {
                tcp.set_nodelay(true).ok();
                tcp.set_write_timeout(Some(remaining(deadline)?))?;
                if !tls {
                    return Ok(Upstream::Plain(tcp));
                }
                let cfg = tls_config().map_err(io::Error::other)?;
                let name = rustls::pki_types::ServerName::try_from(host.trim_matches(['[', ']']).to_string())
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("bad TLS server name: {e}")))?;
                let conn = rustls::ClientConnection::new(cfg, name).map_err(io::Error::other)?;
                return Ok(Upstream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))));
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Entry point from `handle_connection` (request has `X-Wire-Upgrade`).
pub fn handle(shared: &Shared, request: &ParsedRequest, client_reader: BufReader<UnixStream>, client: &mut UnixStream) -> io::Result<()> {
    let start = Instant::now();
    let caps = parse_caps(&request.headers);
    let h = &request.headers;
    let public_only_refusal = |c: &mut UnixStream, what: &str| write_policy_response(c, &format!("blocked: {what} targets are not public"));
    if request.url.scheme() == "file" && caps.public_only {
        public_only_refusal(client, "file:")?;
        return Ok(());
    }
    if !header(h, "x-wire-upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket")) {
        bad_upgrade(client, "X-Wire-Upgrade must be websocket")?;
        return Ok(());
    }
    if request.method != "GET" || !matches!(request.url.scheme(), "ws" | "wss" | "http" | "https") || request.url.host_str().is_none() {
        bad_upgrade(client, "a WebSocket handshake is a GET of a ws:, wss:, http: or https: URL")?;
        return Ok(());
    }
    let Ok(ua) = resolve_user_agent(h) else {
        write_bad_ua_response(client)?;
        return Ok(());
    };
    let use_jar = shared.enabled
        && header(h, "x-wire-jar") == Some(JAR_NAME)
        && !header(h, "x-wire-cookies").is_some_and(|v| v.eq_ignore_ascii_case("off"));
    if use_jar {
        COOKIES_HDR.with(|c| c.set("on"));
    }
    // The connection and the jar use the http(s) twin of a ws(s) URL.
    let mut url = request.url.clone();
    let _ = match request.url.scheme() {
        "ws" => url.set_scheme("http"),
        "wss" => url.set_scheme("https"),
        _ => Ok(()),
    };
    TRACKER.with(|t| *t.borrow_mut() = Tracker { public_only: caps.public_only, ..Tracker::default() });
    let deadline = start + caps.timeout.unwrap_or(DEFAULT_TIMEOUT);

    // Fresh connection through the checking resolver, never a pooled one.
    let mut up = match connect(&url, &caps, deadline) {
        Ok(u) => u,
        Err(e) => {
            let blocked = TRACKER.with(|t| t.borrow().blocked.clone());
            match blocked {
                Some(msg) => write_policy_response(client, &msg)?,
                None => write_synthetic_error_response(client, &e.to_string())?,
            };
            return Ok(());
        }
    };

    let ctx = cookies::RequestCtx::new(&url, "GET", header(h, "x-wire-top-site"), header(h, "x-wire-nav") == Some("1"), header(h, "x-wire-initiator-site"), false);
    let cookie = if use_jar { shared.jar().cookie_header(&ctx, now_unix()) } else { None };
    let sent = cookie.as_ref().map_or(0, |c| c.split("; ").count());

    let host = url.host_str().unwrap_or("");
    let host_hdr = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };
    let target = match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    };
    let mut req = format!(
        "GET {target} HTTP/1.1\r\nHost: {host_hdr}\r\nUser-Agent: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n",
        ua.as_deref().unwrap_or(DEFAULT_UA)
    );
    let drop = |n: &str| {
        n.eq_ignore_ascii_case("upgrade") || n.eq_ignore_ascii_case("transfer-encoding") || (use_jar && n.eq_ignore_ascii_case("cookie"))
    };
    for (name, value) in forwardable_request_headers(h, drop) {
        req.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(c) = &cookie {
        req.push_str(&format!("Cookie: {c}\r\n"));
    }
    req.push_str("\r\n");

    let exchange = (|| -> io::Result<(Head, Vec<u8>)> {
        up.write_all(req.as_bytes())?;
        up.flush()?;
        let (text, rest) = read_head(&mut up, deadline)?;
        Ok((parse_head(&text)?, rest))
    })();
    let (head, rest) = match exchange {
        Ok(x) => x,
        Err(e) => {
            write_synthetic_error_response(client, &format!("websocket handshake failed: {e}"))?;
            return Ok(());
        }
    };

    let mut stored = 0usize;
    if use_jar {
        let mut jar = shared.jar();
        for (n, v) in &head.headers {
            if n.eq_ignore_ascii_case("set-cookie") && jar.store(&ctx, v, now_unix()) {
                stored += 1;
            }
        }
    }
    if timing_enabled() {
        eprintln!("wire: TIMING websocket status={} {} ({:.1}ms)", head.status, request.url, start.elapsed().as_secs_f64() * 1000.0);
        if use_jar {
            eprintln!("{}", cookies::log_line(host, sent, stored));
        }
    }

    let is_101 = head.status == 101;
    let mut out_headers: Vec<(String, String)> = head
        .headers
        .iter()
        .filter(|(n, _)| {
            let n = n.to_ascii_lowercase();
            let hidden = n.starts_with("x-wire-")
                || (use_jar && n == "set-cookie")
                || matches!(n.as_str(), "content-length" | "transfer-encoding")
                || (is_101 && n == "content-encoding");
            !hidden
        })
        .cloned()
        .collect();
    out_headers.push(("X-Wire-Final-Url".to_string(), request.url.to_string()));
    let hops: Vec<&'static str> = TRACKER.with(|t| t.borrow().hops.iter().map(|h| h.1).collect());
    if let Some(worst) = hops.iter().copied().max_by_key(|c| class_rank(c)) {
        out_headers.push(("X-Wire-Target-Class".to_string(), worst.to_string()));
        out_headers.push(("X-Wire-Hop-Classes".to_string(), hops.join(",")));
    }

    if !is_101 {
        let cap = caps.max_bytes.unwrap_or(DEFAULT_BODY_CAP);
        let body = match read_body(&mut up, rest, &head, deadline, cap) {
            Ok(b) => b,
            Err(e) => {
                write_synthetic_error_response(client, &format!("websocket handshake answer body: {e}"))?;
                return Ok(());
            }
        };
        if body.len() > cap {
            write_status_and_headers(
                client,
                502,
                "Bad Gateway",
                &[("Content-Type".to_string(), "text/plain".to_string()), ("X-Wire-Error".to_string(), "too-large".to_string())],
            )?;
            client.write_all(format!("wire: response too large (limit {cap} bytes)").as_bytes())?;
        } else {
            write_status_and_headers(client, head.status, &head.reason, &out_headers)?;
            client.write_all(&body)?;
        }
        client.flush()?;
        return Ok(());
    }

    // 101: send the head, then pipe until either side closes.
    write_status_and_headers(client, 101, &head.reason, &out_headers)?;
    if !rest.is_empty() {
        client.write_all(&rest)?;
    }
    client.flush()?;
    up.tcp().set_read_timeout(None)?;
    up.tcp().set_write_timeout(None)?;
    pipe(client_reader, client, up);
    Ok(())
}

fn would_block(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

/// Copies `r` into `w` until EOF or error (Interrupted retried).
fn copy_until_end(mut r: impl Read, mut w: impl Write) {
    let mut buf = vec![0u8; CHUNK];
    loop {
        match r.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if w.write_all(&buf[..n]).and_then(|()| w.flush()).is_err() {
                    return;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

/// The bidirectional copy. A blocking write into the other side is the only
/// buffering: a slow reader stalls the writer (no unbounded memory).
fn pipe(client_reader: BufReader<UnixStream>, client: &UnixStream, up: Upstream) {
    match up {
        Upstream::Plain(tcp) => {
            let Ok(tcp_w) = tcp.try_clone() else { return };
            std::thread::scope(|s| {
                s.spawn(move || {
                    copy_until_end(client_reader, &tcp_w);
                    // Client closed: tell upstream, then give it a grace to finish.
                    let _ = tcp_w.shutdown(Shutdown::Write);
                    let _ = tcp_w.set_read_timeout(Some(CLOSE_GRACE));
                });
                copy_until_end(&tcp, client);
                let _ = client.shutdown(Shutdown::Both);
                let _ = tcp.shutdown(Shutdown::Both);
            });
        }
        Upstream::Tls(mut tls) => {
            // rustls buffers plaintext internally, so fd readiness is not
            // enough: ONE thread owns the TLS stream and polls both
            // directions; a second thread only reads the client.
            let (tx, rx) = sync_channel::<Vec<u8>>(8);
            let _ = tls.sock.set_read_timeout(Some(TLS_POLL));
            std::thread::scope(|s| {
                s.spawn(move || {
                    let mut r = client_reader;
                    let mut buf = vec![0u8; CHUNK];
                    loop {
                        match r.read(&mut buf) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => {
                                if tx.send(buf[..n].to_vec()).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                });
                let mut closing: Option<Instant> = None;
                let mut buf = vec![0u8; CHUNK];
                let mut dst = client;
                'run: loop {
                    if closing.is_none() {
                        loop {
                            match rx.try_recv() {
                                Ok(data) => {
                                    if tls.write_all(&data).and_then(|()| tls.flush()).is_err() {
                                        break 'run;
                                    }
                                }
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => {
                                    tls.conn.send_close_notify();
                                    let _ = tls.flush();
                                    let _ = tls.sock.shutdown(Shutdown::Write);
                                    closing = Some(Instant::now());
                                    break;
                                }
                            }
                        }
                    }
                    if closing.is_some_and(|t| t.elapsed() > CLOSE_GRACE) {
                        break;
                    }
                    match tls.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if dst.write_all(&buf[..n]).and_then(|()| dst.flush()).is_err() {
                                break;
                            }
                        }
                        Err(e) if would_block(&e) || e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                let _ = client.shutdown(Shutdown::Both);
                let _ = tls.sock.shutdown(Shutdown::Both);
                drop(rx);
            });
        }
    }
}

#[cfg(test)]
#[path = "ws_tests.rs"]
mod tests;
