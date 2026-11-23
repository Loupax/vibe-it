//! The front: accepts browser connections, reads the request head and dispatches.
//!
//! `CONNECT host:port`  -> [`tunnel`]: one decrypt decision ([`crate::decrypt::Policy::decide`]), then
//!                         either dial, `200`, splice both ways (opaque), or [`decrypted`].
//! `GET http://...`     -> [`plain`]: one request per connection, streamed both ways, `Connection: close`.
//! decrypted tunnel     -> [`decrypted`]: `200`, TLS toward the browser with a minted leaf, then each
//!                         HTTP/1.1 request on that connection goes through [`forward`].
//!
//! [`forward`] is the one place every parsed request and its response pass through, plain or decrypted:
//! W2-F puts the http filter chain at its top, before the upstream connector. W2-F routes CONNECTs
//! through tunnel filters inside the upstream connector.

use std::io;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::config::Config;
use crate::conn::{Conn, Prefixed};
use crate::decrypt::{Decision, Policy};
use crate::flow::{Flow, Kind, Registry};
use crate::host::{authority_host, normalise_host, split_authority};
use crate::http::{self, ChunkScanner, Framing, HeadError, Headers, RequestHead};
use crate::upstream;

/// How long a browser may take to send a complete request head (and, decrypted, its TLS handshake).
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a decrypted browser connection may sit idle between requests.
const KEEPALIVE_IDLE: Duration = Duration::from_secs(120);
/// Userspace copy buffer for parsed (non-spliced) bodies.
const COPY_BUF: usize = 64 * 1024;

pub struct Ctx {
    pub cfg: Config,
    pub reg: Arc<Registry>,
    pub policy: Policy,
}

/// Handle one browser connection from accept to close.
pub async fn handle(mut conn: Conn, peer: String, ctx: Arc<Ctx>) {
    let mut buf = Vec::new();
    let head =
        match tokio::time::timeout(HEAD_TIMEOUT, http::read_request(&mut conn, &mut buf)).await {
            Err(_) => {
                crate::debug!(
                    "peer={peer} no request head within {}s",
                    HEAD_TIMEOUT.as_secs()
                );
                refuse(&mut conn, 408, "wire2: request head timeout").await;
                return;
            }
            Ok(Err(HeadError::Eof)) => return,
            Ok(Err(HeadError::Io(e))) => {
                crate::debug!("peer={peer} read error before request head: {e}");
                return;
            }
            Ok(Err(HeadError::TooLarge)) => {
                crate::warn!("peer={peer} request head too large");
                refuse(&mut conn, 431, "wire2: request head too large").await;
                return;
            }
            Ok(Err(e)) => {
                crate::warn!("peer={peer} bad request: {e}");
                refuse(&mut conn, 400, "wire2: malformed request").await;
                return;
            }
            Ok(Ok(h)) => h,
        };
    crate::debug!("peer={peer} {} {}", head.method, head.target);
    if head.method.eq_ignore_ascii_case("CONNECT") {
        tunnel(conn, head, buf, &ctx).await;
    } else {
        plain(conn, head, buf, &ctx).await;
    }
}

/// Answer with a short plain-text error and close (a response the proxy produces itself).
async fn refuse(conn: &mut Conn, code: u16, body: &str) -> usize {
    let resp = http::error_response(code, body);
    if conn.write_all(&resp).await.is_err() {
        return 0;
    }
    finish(conn).await;
    resp.len()
}

/// Half-close and give the browser a moment to read the response before the socket goes away, so
/// unread request bytes on our side do not turn the close into a reset that eats the response.
async fn finish<S: AsyncRead + AsyncWrite + Unpin>(conn: &mut S) {
    let _ = conn.shutdown().await;
    let mut sink = [0u8; 4096];
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        let mut total = 0usize;
        while total < 1 << 20 {
            match conn.read(&mut sink).await {
                Ok(0) | Err(_) => break,
                Ok(n) => total += n,
            }
        }
    })
    .await;
}

async fn refuse_flow(conn: &mut Conn, flow: &Flow, code: u16, body: &str) {
    flow.set_status(code);
    let n = refuse(conn, code, body).await;
    flow.down.fetch_add(n as u64, Ordering::Relaxed);
}

fn log_upstream_error(flow: &Flow, e: &upstream::DialError) {
    crate::warn!(
        "flow={} host={} port={} upstream error, answering {}: {e}",
        flow.id,
        flow.host,
        flow.port,
        e.status()
    );
}

// ---------------------------------------------------------------------------------------------------
// CONNECT

async fn tunnel(mut conn: Conn, head: RequestHead, leftover: Vec<u8>, ctx: &Ctx) {
    let (raw_host, port) = match split_authority(&head.target) {
        Ok((h, Some(p))) => (h, p),
        Ok((_, None)) => {
            crate::warn!("bad CONNECT target {:?}: no port", head.target);
            refuse(&mut conn, 400, "wire2: CONNECT needs host:port").await;
            return;
        }
        Err(e) => {
            crate::warn!("{e}");
            refuse(&mut conn, 400, &format!("wire2: {e}")).await;
            return;
        }
    };
    let host = match normalise_host(raw_host) {
        Ok(h) => h,
        Err(e) => {
            crate::warn!("CONNECT {e}");
            refuse(&mut conn, 400, &format!("wire2: {e}")).await;
            return;
        }
    };
    // The one decrypt decision of this CONNECT (SPEC: made per CONNECT, never changed later).
    if let Decision::Decrypt { by } = ctx.policy.decide(&host) {
        drop(head);
        decrypted(conn, host, port, leftover, &by, ctx).await;
        return;
    }
    let flow = ctx.reg.open(Kind::Tunnel, "CONNECT", &host, port);
    // W2-F: the chain of tunnel filters for `host` is computed here, once, and the dial goes through it.
    let up = tokio::select! {
        r = upstream::dial(&host, port, ctx.cfg.dial_timeout) => r,
        _ = flow.cancel.cancelled() => return,
    };
    let mut up = match up {
        Ok(s) => Conn::Tcp(s),
        Err(e) => {
            log_upstream_error(&flow, &e);
            refuse_flow(
                &mut conn,
                &flow,
                e.status(),
                &format!("wire2: {host}:{port}: {e}"),
            )
            .await;
            return;
        }
    };
    const OK: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";
    if conn.write_all(OK).await.is_err() {
        return;
    }
    flow.set_status(200);
    flow.down.fetch_add(OK.len() as u64, Ordering::Relaxed);
    // Bytes the browser sent right behind the CONNECT head (an early ClientHello) go first.
    if !leftover.is_empty() {
        if up.write_all(&leftover).await.is_err() {
            return;
        }
        flow.up.fetch_add(leftover.len() as u64, Ordering::Relaxed);
    }
    // The head buffer is not needed for the rest of a possibly long-lived tunnel.
    drop(leftover);
    drop(head);
    let r = tokio::select! {
        r = crate::splice::splice_bidi(&conn, &up, &flow.up, &flow.down) => r,
        _ = flow.cancel.cancelled() => Ok(()),
    };
    if let Err(e) = r {
        crate::debug!("flow={} tunnel ended: {e}", flow.id);
    }
}

/// A decrypted CONNECT: answer `200` ourselves, terminate TLS toward the browser with a leaf for
/// `host` signed by the local CA (ALPN http/1.1 only), then serve HTTP/1.1 requests on that connection
/// (keep-alive), each through [`forward`] to a verified TLS connection to the origin.
async fn decrypted(
    mut conn: Conn,
    host: String,
    port: u16,
    leftover: Vec<u8>,
    by: &str,
    ctx: &Ctx,
) {
    let flow = ctx
        .reg
        .open_with(Kind::Tunnel, "CONNECT", &host, port, true);
    crate::debug!("flow={} decrypting {host}:{port} for {by}", flow.id);
    let tls_cfg = match ctx.policy.ca.server_config(&host) {
        Ok(c) => c,
        Err(e) => {
            crate::error!(
                "flow={} host={host} cannot mint a certificate, answering 502: {e}",
                flow.id
            );
            refuse_flow(
                &mut conn,
                &flow,
                502,
                &format!("wire2: cannot decrypt {host}"),
            )
            .await;
            return;
        }
    };
    const OK: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";
    if conn.write_all(OK).await.is_err() {
        return;
    }
    flow.set_status(200);
    flow.down.fetch_add(OK.len() as u64, Ordering::Relaxed);
    let accept = tokio_rustls::TlsAcceptor::from(tls_cfg).accept(Prefixed::new(leftover, conn));
    let tls = tokio::select! {
        r = tokio::time::timeout(HEAD_TIMEOUT, accept) => r,
        _ = flow.cancel.cancelled() => return,
    };
    let tls = match tls {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            crate::info!(
                "flow={} host={host} TLS handshake with the browser failed: {e} (does it trust {}?)",
                flow.id,
                ctx.policy.ca.cert_path()
            );
            return;
        }
        Err(_) => {
            crate::debug!("flow={} no TLS handshake from the browser", flow.id);
            return;
        }
    };
    let (mut br, mut bw) = tokio::io::split(tls);
    let mut buf = Vec::new();
    let mut n = 0u64;
    loop {
        let wait = if n == 0 { HEAD_TIMEOUT } else { KEEPALIVE_IDLE };
        let head = tokio::select! {
            r = tokio::time::timeout(wait, http::read_request(&mut br, &mut buf)) => r,
            _ = flow.cancel.cancelled() => return,
        };
        let status = AtomicU16::new(0);
        let head = match head {
            // Idle, or the browser closed / reset between requests.
            Err(_) | Ok(Err(HeadError::Eof)) | Ok(Err(HeadError::Io(_))) => break,
            Ok(Err(HeadError::TooLarge)) => {
                crate::warn!("flow={} request head too large", flow.id);
                let _ = answer(
                    &mut bw,
                    &flow.down,
                    &status,
                    431,
                    "wire2: request head too large",
                )
                .await;
                break;
            }
            Ok(Err(e)) => {
                crate::warn!("flow={} bad request: {e}", flow.id);
                let _ = answer(
                    &mut bw,
                    &flow.down,
                    &status,
                    400,
                    "wire2: malformed request",
                )
                .await;
                break;
            }
            Ok(Ok(h)) => h,
        };
        n += 1;
        let started = Instant::now();
        let (up0, down0) = (
            flow.up.load(Ordering::Relaxed),
            flow.down.load(Ordering::Relaxed),
        );
        let (next, url) = match decrypted_request(&head, &host, port) {
            Ok(req) => {
                let dest = Dest {
                    host: &host,
                    port,
                    tls: true,
                };
                let next = forward(
                    &mut br,
                    &mut bw,
                    std::mem::take(&mut buf),
                    &req,
                    dest,
                    ctx,
                    &flow,
                    &status,
                    true,
                )
                .await;
                (next, req.url)
            }
            Err((code, why)) => {
                crate::warn!(
                    "flow={} refusing {} {:?}: {why}",
                    flow.id,
                    head.method,
                    head.target
                );
                let next =
                    answer(&mut bw, &flow.down, &status, code, &format!("wire2: {why}")).await;
                (next, head.target.clone())
            }
        };
        // SPEC "Flow lifecycle": a decrypted flow also logs one line per request.
        let code = status.load(Ordering::Relaxed);
        crate::info!(
            "access flow={} req={n} {} {url} status={} up={} down={} duration={}",
            flow.id,
            head.method,
            if code == 0 {
                "-".to_string()
            } else {
                code.to_string()
            },
            flow.up.load(Ordering::Relaxed) - up0,
            flow.down.load(Ordering::Relaxed) - down0,
            crate::flow::fmt_secs(started.elapsed()),
        );
        match next {
            Next::KeepAlive(rest) => buf = rest,
            Next::Close => break,
            Next::Abort => return,
        }
    }
    let mut tls = br.unsplit(bw);
    finish(&mut tls).await;
}

/// A request read inside a decrypted tunnel -> the [`Request`] for the upstream connector, or the
/// status and reason to refuse it with.
fn decrypted_request(head: &RequestHead, host: &str, port: u16) -> Result<Request, (u16, String)> {
    if head.method.eq_ignore_ascii_case("CONNECT") {
        return Err((400, "CONNECT inside a decrypted tunnel".into()));
    }
    let same_host = |raw: &str| -> Result<(), (u16, String)> {
        let h = split_authority(raw)
            .and_then(|(h, _)| normalise_host(h))
            .map_err(|e| (400, e.to_string()))?;
        if h == host {
            Ok(())
        } else {
            Err((421, format!("request for {h} inside a tunnel to {host}")))
        }
    };
    let path = if head.target.starts_with('/') {
        head.target.split('#').next().unwrap_or("/").to_string()
    } else if head.target == "*" && head.method.eq_ignore_ascii_case("OPTIONS") {
        "*".to_string()
    } else if let Some((auth, path)) = split_url(&head.target, "https://") {
        same_host(auth)?;
        path
    } else {
        return Err((400, format!("bad request target {:?}", head.target)));
    };
    if let Some(h) = http::header(&head.headers, "host") {
        same_host(&String::from_utf8_lossy(h))?;
    }
    let auth = match port {
        443 => authority_host(host),
        p => format!("{}:{p}", authority_host(host)),
    };
    let url = if path == "*" {
        format!("https://{auth}")
    } else {
        format!("https://{auth}{path}")
    };
    build_request(head, url, path, auth).map_err(|e| (400, e))
}

// ---------------------------------------------------------------------------------------------------
// Plain http://

/// `<scheme>host[:port]/path?q#frag` -> (authority, origin-form target without the fragment).
/// `scheme` is `http://` or `https://`, matched case-insensitively.
fn split_url<'a>(target: &'a str, scheme: &str) -> Option<(&'a str, String)> {
    let head = target.get(..scheme.len())?;
    if !head.eq_ignore_ascii_case(scheme) {
        return None;
    }
    let rest = &target[scheme.len()..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (auth, tail) = rest.split_at(end);
    let tail = tail.split('#').next().unwrap_or("");
    let path = if tail.is_empty() {
        "/".to_string()
    } else if tail.starts_with('?') {
        format!("/{tail}")
    } else {
        tail.to_string()
    };
    Some((auth, path))
}

fn split_http_url(target: &str) -> Option<(&str, String)> {
    split_url(target, "http://")
}

async fn plain(mut conn: Conn, head: RequestHead, leftover: Vec<u8>, ctx: &Ctx) {
    let Some((auth, path)) = split_http_url(&head.target) else {
        let why = if head.target.len() > 8 && head.target[..8].eq_ignore_ascii_case("https://") {
            "https:// URLs go through CONNECT"
        } else {
            "this is a proxy: send CONNECT or an absolute http:// URL"
        };
        crate::warn!("refusing {} {:?}: {why}", head.method, head.target);
        refuse(&mut conn, 400, &format!("wire2: {why}")).await;
        return;
    };
    let parsed = if auth.contains('@') {
        Err("credentials in the URL".to_string())
    } else {
        split_authority(auth)
            .and_then(|(h, p)| Ok((normalise_host(h)?, p.unwrap_or(80))))
            .map_err(|e| e.to_string())
    };
    let (host, port) = match parsed {
        Ok(hp) => hp,
        Err(e) => {
            crate::warn!("refusing {} {:?}: {e}", head.method, head.target);
            refuse(&mut conn, 400, &format!("wire2: {e}")).await;
            return;
        }
    };
    let flow = ctx.reg.open(Kind::Http, &head.method, &host, port);
    let host_hdr = match port {
        80 => authority_host(&host),
        p => format!("{}:{p}", authority_host(&host)),
    };
    let url = format!("http://{host_hdr}{path}");
    let req = match build_request(&head, url, path, host_hdr) {
        Ok(r) => r,
        Err(e) => {
            crate::warn!("flow={} bad request body framing: {e}", flow.id);
            refuse_flow(&mut conn, &flow, 400, &format!("wire2: {e}")).await;
            return;
        }
    };
    let (mut br, mut bw) = tokio::io::split(conn);
    let dest = Dest {
        host: &host,
        port,
        tls: false,
    };
    let next = forward(
        &mut br,
        &mut bw,
        leftover,
        &req,
        dest,
        ctx,
        &flow,
        &flow.status,
        false,
    )
    .await;
    if !matches!(next, Next::Abort) {
        let mut conn = br.unsplit(bw);
        finish(&mut conn).await;
    }
}

// ---------------------------------------------------------------------------------------------------
// The shared request path

/// A request as it reaches the upstream connector, plain or decrypted. This is what W2-F's http chain
/// will see (it sends `url` in absolute form to the filters).
#[derive(Debug)]
struct Request {
    method: String,
    /// Full URL (`http://host[:port]/path`, `https://host[:port]/path`), for logs and filters.
    url: String,
    /// Origin-form target sent upstream.
    path: String,
    /// Headers for upstream: hop-by-hop and `Proxy-*` removed, `Host` from the URL, `Connection` set.
    headers: Headers,
    framing: Framing,
    upgrade: bool,
    /// The browser wants its connection closed after this exchange.
    close: bool,
}

fn build_request(
    head: &RequestHead,
    url: String,
    path: String,
    host_hdr: String,
) -> Result<Request, String> {
    let framing = http::request_framing(&head.headers)?;
    let upgrade = http::is_upgrade_request(&head.headers);
    let mut hs: Headers = http::strip_hop_by_hop(&head.headers, upgrade);
    hs.retain(|(n, _)| !n.eq_ignore_ascii_case("host"));
    hs.insert(0, ("Host".into(), host_hdr.into_bytes()));
    hs.push((
        "Connection".into(),
        if upgrade { &b"upgrade"[..] } else { b"close" }.to_vec(),
    ));
    http::drop_length_if_chunked(&mut hs);
    Ok(Request {
        method: head.method.clone(),
        url,
        path,
        headers: hs,
        framing,
        upgrade,
        close: head.wants_close(),
    })
}

/// Where a request goes: the origin, over TLS for decrypted hosts.
#[derive(Clone, Copy)]
struct Dest<'a> {
    host: &'a str,
    port: u16,
    tls: bool,
}

/// What the browser connection does after one exchange.
enum Next {
    /// It may carry another request; these bytes were already read past this one.
    KeepAlive(Vec<u8>),
    /// The answer is complete; close gracefully.
    Close,
    /// Stop at once (cancelled, broken connection, an upgraded pipe that ended).
    Abort,
}

/// Write an error response the proxy produces itself (it says `Connection: close`).
async fn answer<W: AsyncWrite + Unpin>(
    bw: &mut W,
    down: &AtomicU64,
    status: &AtomicU16,
    code: u16,
    body: &str,
) -> Next {
    status.store(code, Ordering::Relaxed);
    let resp = http::error_response(code, body);
    if bw.write_all(&resp).await.is_err() || bw.flush().await.is_err() {
        return Next::Abort;
    }
    down.fetch_add(resp.len() as u64, Ordering::Relaxed);
    Next::Close
}

/// The one place every parsed request passes through on its way to the origin, and its response on
/// the way back. W2-F's http filter chain goes at the top of this function, before the upstream
/// connector. `status` receives the response status (the flow's for plain http, a per-request slot
/// for decrypted tunnels). `keep_alive`: the browser connection may carry further requests.
#[allow(clippy::too_many_arguments)]
async fn forward<BR, BW>(
    br: &mut BR,
    bw: &mut BW,
    leftover: Vec<u8>,
    req: &Request,
    dest: Dest<'_>,
    ctx: &Ctx,
    flow: &Flow,
    status: &AtomicU16,
    keep_alive: bool,
) -> Next
where
    BR: AsyncRead + Unpin,
    BW: AsyncWrite + Unpin,
{
    crate::debug!(
        "flow={} -> {} {} (body {:?})",
        flow.id,
        req.method,
        req.url,
        req.framing
    );
    // W2-F: the http filter chain runs here, before the upstream connector.
    let timeout = ctx.cfg.dial_timeout;
    if dest.tls {
        let up = tokio::select! {
            r = upstream::dial_tls(dest.host, dest.port, timeout) => r,
            _ = flow.cancel.cancelled() => return Next::Abort,
        };
        match up {
            Ok(s) => {
                let (ur, uw) = tokio::io::split(s);
                exchange(br, bw, leftover, req, ur, uw, flow, status, keep_alive).await
            }
            Err(e) => upstream_failed(bw, flow, status, dest, &e).await,
        }
    } else {
        let up = tokio::select! {
            r = upstream::dial(dest.host, dest.port, timeout) => r,
            _ = flow.cancel.cancelled() => return Next::Abort,
        };
        match up {
            Ok(s) => {
                let (ur, uw) = s.into_split();
                exchange(br, bw, leftover, req, ur, uw, flow, status, keep_alive).await
            }
            Err(e) => upstream_failed(bw, flow, status, dest, &e).await,
        }
    }
}

async fn upstream_failed<W: AsyncWrite + Unpin>(
    bw: &mut W,
    flow: &Flow,
    status: &AtomicU16,
    dest: Dest<'_>,
    e: &upstream::DialError,
) -> Next {
    log_upstream_error(flow, e);
    let body = format!("wire2: {}:{}: {e}", dest.host, dest.port);
    answer(bw, &flow.down, status, e.status(), &body).await
}

#[derive(Debug)]
enum CopyError {
    /// Reading the source failed or it ended before the body did.
    Src(io::Error),
    Dst(io::Error),
    Framing(String),
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CopyError::Src(e) => write!(f, "read: {e}"),
            CopyError::Dst(e) => write!(f, "write: {e}"),
            CopyError::Framing(m) => write!(f, "framing: {m}"),
        }
    }
}

async fn put<W: AsyncWrite + Unpin>(
    w: &mut W,
    data: &[u8],
    ctr: &AtomicU64,
) -> Result<(), CopyError> {
    if data.is_empty() {
        return Ok(());
    }
    w.write_all(data).await.map_err(CopyError::Dst)?;
    ctr.fetch_add(data.len() as u64, Ordering::Relaxed);
    Ok(())
}

/// Stream one message body from `r` to `w` exactly as framed (bytes unchanged, chunked included).
/// `pre` holds body bytes already read together with the head. Returns the bytes read past the end of
/// the body (the start of a next request on a kept-alive connection).
async fn copy_body<R, W>(
    r: &mut R,
    w: &mut W,
    framing: Framing,
    pre: &[u8],
    ctr: &AtomicU64,
) -> Result<Vec<u8>, CopyError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let eof = || {
        CopyError::Src(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "body ended early",
        ))
    };
    let excess = match framing {
        Framing::None => pre.to_vec(),
        Framing::Length(n) => {
            let first = (n.min(pre.len() as u64)) as usize;
            put(w, &pre[..first], ctr).await?;
            let mut left = n - first as u64;
            let mut buf = vec![0u8; COPY_BUF.min(left as usize)];
            while left > 0 {
                let want = buf.len().min(left as usize);
                let got = r.read(&mut buf[..want]).await.map_err(CopyError::Src)?;
                if got == 0 {
                    return Err(eof());
                }
                put(w, &buf[..got], ctr).await?;
                left -= got as u64;
            }
            pre[first..].to_vec()
        }
        Framing::Chunked => {
            let mut sc = ChunkScanner::new();
            match sc.feed(pre).map_err(CopyError::Framing)? {
                Some(end) => {
                    put(w, &pre[..end], ctr).await?;
                    flush(w).await?;
                    return Ok(pre[end..].to_vec());
                }
                None => put(w, pre, ctr).await?,
            }
            let mut buf = vec![0u8; COPY_BUF];
            loop {
                let got = r.read(&mut buf).await.map_err(CopyError::Src)?;
                if got == 0 {
                    return Err(eof());
                }
                match sc.feed(&buf[..got]).map_err(CopyError::Framing)? {
                    Some(end) => {
                        put(w, &buf[..end], ctr).await?;
                        break buf[end..got].to_vec();
                    }
                    None => put(w, &buf[..got], ctr).await?,
                }
            }
        }
        Framing::UntilClose => {
            put(w, pre, ctr).await?;
            let mut buf = vec![0u8; COPY_BUF];
            loop {
                let got = r.read(&mut buf).await.map_err(CopyError::Src)?;
                if got == 0 {
                    break Vec::new();
                }
                put(w, &buf[..got], ctr).await?;
            }
        }
    };
    flush(w).await?;
    Ok(excess)
}

async fn flush<W: AsyncWrite + Unpin>(w: &mut W) -> Result<(), CopyError> {
    w.flush().await.map_err(CopyError::Dst)
}

/// Plain byte copy until EOF, then half-close `w` (the two directions of an upgraded connection).
async fn copy_until_eof<R, W>(
    r: &mut R,
    w: &mut W,
    pre: &[u8],
    ctr: &AtomicU64,
) -> Result<(), CopyError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    copy_body(r, w, Framing::UntilClose, pre, ctr).await?;
    w.shutdown().await.map_err(CopyError::Dst)
}

/// Buffers in flight between the two halves of [`pipelined_body`] (at most this many x [`COPY_BUF`]).
const PIPE_DEPTH: usize = 4;

/// An `AsyncWrite` that hands each write to a bounded channel (backpressure: it waits for a slot).
///
/// Before each chunk it yields once (wakes itself and returns `Pending`). Why: the receiving task,
/// woken by our send, sits in tokio's per-worker LIFO slot, which other workers cannot steal, so the
/// two halves of [`pipelined_body`] would ping-pong on one thread (measured: every sample of a
/// decrypted transfer on one worker). A self-woken task is requeued on the stealable local queue and
/// an idle worker is notified, so the halves end up on two cores. (`disable_lifo_slot` would do this
/// globally but needs `tokio_unstable`.)
struct ChanWriter {
    tx: tokio_util::sync::PollSender<Vec<u8>>,
    yielded: bool,
    /// Buffers the writing half has finished with, reused instead of allocating per chunk.
    spare: Spare,
}

/// At most `PIPE_DEPTH` + 1 recycled chunk buffers; the lock is held only to push or pop.
type Spare = Arc<std::sync::Mutex<Vec<Vec<u8>>>>;

impl AsyncWrite for ChanWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let me = self.get_mut();
        if !me.yielded {
            me.yielded = true;
            cx.waker().wake_by_ref();
            return std::task::Poll::Pending;
        }
        let closed = || io::Error::from(io::ErrorKind::BrokenPipe);
        std::task::ready!(me.tx.poll_reserve(cx)).map_err(|_| closed())?;
        let mut chunk = me
            .spare
            .lock()
            .ok()
            .and_then(|mut v| v.pop())
            .unwrap_or_default();
        chunk.clear();
        chunk.extend_from_slice(buf);
        me.tx.send_item(chunk).map_err(|_| closed())?;
        me.yielded = false;
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        self.get_mut().tx.close();
        std::task::Poll::Ready(Ok(()))
    }
}

/// A reader that, once it has data, keeps reading while more is immediately available, up to the
/// caller's buffer. tokio-rustls hands out one TLS record (<= 16 KiB) per read; coalescing makes each
/// hop of [`pipelined_body`] carry up to [`COPY_BUF`] for one cross-thread wakeup. An error after some
/// data is kept for the next call.
struct Coalesce<R> {
    inner: R,
    err: Option<io::Error>,
}

impl<R: AsyncRead + Unpin> AsyncRead for Coalesce<R> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;
        let me = self.get_mut();
        if let Some(e) = me.err.take() {
            return Poll::Ready(Err(e));
        }
        let start = buf.filled().len();
        std::task::ready!(std::pin::Pin::new(&mut me.inner).poll_read(cx, buf))?;
        loop {
            let before = buf.filled().len();
            if before == start || buf.remaining() == 0 {
                break;
            }
            match std::pin::Pin::new(&mut me.inner).poll_read(cx, buf) {
                Poll::Ready(Ok(())) if buf.filled().len() > before => {}
                Poll::Ready(Err(e)) => {
                    me.err = Some(e);
                    break;
                }
                // Pending (the waker stays registered; a spurious wake-up later is harmless) or EOF,
                // which the next call reports again.
                _ => break,
            }
        }
        Poll::Ready(Ok(()))
    }
}

/// Aborts a spawned task when dropped (the flow ended or was cancelled first).
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Copy a response body in two halves that run on different worker threads: a spawned task reads (for
/// TLS: decrypts) and frames it, this task writes (for TLS: encrypts) to the browser. With one task
/// doing both, a decrypted transfer is bounded by decrypt + encrypt on one core (measured 47 % of
/// direct TLS); split, the two overlap. At most [`PIPE_DEPTH`] buffers are in flight, so memory
/// stays flat.
async fn pipelined_body<UR, BW>(
    ur: UR,
    bw: &mut BW,
    framing: Framing,
    pre: Vec<u8>,
    down: &AtomicU64,
) -> Result<(), CopyError>
where
    UR: AsyncRead + Unpin + Send + 'static,
    BW: AsyncWrite + Unpin,
{
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(PIPE_DEPTH);
    let spare: Spare = Arc::default();
    let spare_w = spare.clone();
    let reader = tokio::spawn(async move {
        let mut ur = Coalesce {
            inner: ur,
            err: None,
        };
        let mut w = ChanWriter {
            tx: tokio_util::sync::PollSender::new(tx),
            yielded: false,
            spare: spare_w,
        };
        let ctr = AtomicU64::new(0);
        copy_body(&mut ur, &mut w, framing, &pre, &ctr)
            .await
            .map(drop)
    });
    let _guard = AbortOnDrop(reader.abort_handle());
    while let Some(chunk) = rx.recv().await {
        put(bw, &chunk, down).await?;
        if let Ok(mut v) = spare.lock()
            && v.len() <= PIPE_DEPTH
        {
            v.push(chunk);
        }
    }
    flush(bw).await?;
    match reader.await {
        Ok(r) => r,
        Err(e) => Err(CopyError::Src(io::Error::other(e.to_string()))),
    }
}

enum Response {
    Final(http::ResponseHead, Vec<u8>),
    Upgrade(http::ResponseHead, Vec<u8>),
}

/// One request/response exchange over an open upstream connection (`ur`/`uw`), both bodies streamed.
#[allow(clippy::too_many_arguments)]
async fn exchange<BR, BW, UR, UW>(
    br: &mut BR,
    bw: &mut BW,
    leftover: Vec<u8>,
    req: &Request,
    mut ur: UR,
    mut uw: UW,
    flow: &Flow,
    status: &AtomicU16,
    keep_alive: bool,
) -> Next
where
    BR: AsyncRead + Unpin,
    BW: AsyncWrite + Unpin,
    UR: AsyncRead + Unpin + Send + 'static,
    UW: AsyncWrite + Unpin,
{
    let up_head = http::write_request_head(&req.method, &req.path, &req.headers);
    // Request: head + body, streamed, concurrently with waiting for the response (a server may answer
    // before it has read the whole body).
    let mut send = Box::pin(async {
        put(&mut uw, &up_head, &flow.up).await?;
        let rest = if req.upgrade && req.framing == Framing::None {
            put(&mut uw, &leftover, &flow.up).await?;
            Vec::new()
        } else {
            copy_body(&mut *br, &mut uw, req.framing, &leftover, &flow.up).await?
        };
        flush(&mut uw).await?;
        Ok::<_, CopyError>(rest)
    });
    let mut sent: Option<Result<Vec<u8>, CopyError>> = None;

    let resp = {
        let mut recv = pin!(async {
            let mut rbuf = Vec::new();
            loop {
                let rh = http::read_response(&mut ur, &mut rbuf)
                    .await
                    .map_err(|e| e.to_string())?;
                match rh.code {
                    101 if req.upgrade => return Ok(Response::Upgrade(rh, rbuf)),
                    101 => return Err("101 to a request that did not ask to upgrade".to_string()),
                    100..=199 => {
                        // Interim responses (100 Continue, 103 Early Hints) are relayed, minus hop-by-hop.
                        let ih = http::write_response_head(
                            rh.code,
                            &rh.reason,
                            &http::strip_hop_by_hop(&rh.headers, false),
                        );
                        bw.write_all(&ih)
                            .await
                            .and(bw.flush().await)
                            .map_err(|e| format!("browser: {e}"))?;
                        flow.down.fetch_add(ih.len() as u64, Ordering::Relaxed);
                    }
                    _ => return Ok(Response::Final(rh, rbuf)),
                }
            }
        });
        loop {
            tokio::select! {
                r = &mut send, if sent.is_none() => {
                    match r {
                        Err(CopyError::Dst(e)) => {
                            // Upstream stopped reading; it may still have answered.
                            crate::debug!("flow={} upstream stopped reading the request: {e}", flow.id);
                            sent = Some(Err(CopyError::Dst(e)));
                        }
                        Err(e) => {
                            crate::debug!("flow={} request from browser failed: {e}", flow.id);
                            return Next::Abort;
                        }
                        Ok(rest) => sent = Some(Ok(rest)),
                    }
                }
                r = &mut recv => break r,
                _ = flow.cancel.cancelled() => return Next::Abort,
            }
        }
    };

    let (rh, rbuf, is_upgrade) = match resp {
        Ok(Response::Final(rh, rbuf)) => (rh, rbuf, false),
        Ok(Response::Upgrade(rh, rbuf)) => (rh, rbuf, true),
        Err(e) => {
            crate::warn!(
                "flow={} host={} port={} bad upstream response, answering 502: {e}",
                flow.id,
                flow.host,
                flow.port
            );
            drop(send);
            let body = format!("wire2: {}:{}: bad upstream response", flow.host, flow.port);
            return answer(bw, &flow.down, status, 502, &body).await;
        }
    };

    if is_upgrade {
        let mut hs = http::strip_hop_by_hop(&rh.headers, true);
        hs.push(("Connection".into(), b"upgrade".to_vec()));
        let h = http::write_response_head(101, &rh.reason, &hs);
        if put(bw, &h, &flow.down).await.is_err() || flush(bw).await.is_err() {
            return Next::Abort;
        }
        status.store(101, Ordering::Relaxed);
        drop(send);
        let r = tokio::select! {
            r = crate::splice::both_ways(
                copy_until_eof(br, &mut uw, &[], &flow.up),
                copy_until_eof(&mut ur, bw, &rbuf, &flow.down),
            ) => r,
            _ = flow.cancel.cancelled() => Ok(()),
        };
        if let Err(e) = r {
            crate::debug!("flow={} upgraded connection ended: {e}", flow.id);
        }
        return Next::Abort;
    }

    let rframing = match http::response_framing(&req.method, rh.code, &rh.headers) {
        Ok(f) => f,
        Err(e) => {
            crate::warn!(
                "flow={} host={} port={} bad upstream framing, answering 502: {e}",
                flow.id,
                flow.host,
                flow.port
            );
            drop(send);
            let body = format!("wire2: {}:{}: bad upstream response", flow.host, flow.port);
            return answer(bw, &flow.down, status, 502, &body).await;
        }
    };
    // The browser connection survives only if this response's end is visible without a close.
    let ka = keep_alive && !req.close && rframing != Framing::UntilClose;
    let mut hs = http::strip_hop_by_hop(&rh.headers, false);
    http::drop_length_if_chunked(&mut hs);
    if !ka {
        hs.push(("Connection".into(), b"close".to_vec()));
    }
    let h = http::write_response_head(rh.code, &rh.reason, &hs);
    if put(bw, &h, &flow.down).await.is_err() {
        return Next::Abort;
    }
    status.store(rh.code, Ordering::Relaxed);
    crate::debug!("flow={} <- {} (body {rframing:?})", flow.id, rh.code);

    let r = {
        let mut body = pin!(pipelined_body(ur, bw, rframing, rbuf, &flow.down));
        loop {
            tokio::select! {
                r = &mut send, if sent.is_none() => {
                    if let Err(e @ (CopyError::Src(_) | CopyError::Framing(_))) = &r {
                        crate::debug!("flow={} request from browser failed: {e}", flow.id);
                        return Next::Abort;
                    }
                    sent = Some(r);
                }
                r = &mut body => break r,
                _ = flow.cancel.cancelled() => return Next::Abort,
            }
        }
    };
    if let Err(e) = r {
        crate::debug!("flow={} response body ended: {e}", flow.id);
        return Next::Abort;
    }
    if !ka {
        return Next::Close;
    }
    // Keep-alive needs the whole request read from the browser; a request still streaming after the
    // response is complete (the origin answered early) ends the connection instead.
    let sent = match sent {
        Some(r) => r,
        None => match tokio::time::timeout(Duration::ZERO, &mut send).await {
            Ok(r) => r,
            Err(_) => return Next::Close,
        },
    };
    match sent {
        Ok(rest) => Next::KeepAlive(rest),
        Err(_) => Next::Close,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_urls() {
        assert_eq!(
            split_http_url("http://a.test/x?y#z"),
            Some(("a.test", "/x?y".to_string()))
        );
        assert_eq!(
            split_http_url("HTTP://A.test:81"),
            Some(("A.test:81", "/".to_string()))
        );
        assert_eq!(
            split_http_url("http://[::1]:8080?q"),
            Some(("[::1]:8080", "/?q".to_string()))
        );
        assert_eq!(
            split_http_url("http://a#frag"),
            Some(("a", "/".to_string()))
        );
        assert_eq!(split_http_url("https://a/"), None);
        assert_eq!(split_http_url("/relative"), None);
        assert_eq!(
            split_url("HTTPS://a.test:8443/p", "https://"),
            Some(("a.test:8443", "/p".to_string()))
        );
    }

    fn head(method: &str, target: &str, hs: &[(&str, &str)]) -> RequestHead {
        RequestHead {
            method: method.into(),
            target: target.into(),
            headers: hs
                .iter()
                .map(|(n, v)| (n.to_string(), v.as_bytes().to_vec()))
                .collect(),
            version: 1,
        }
    }

    #[test]
    fn decrypted_targets() {
        let r = decrypted_request(
            &head(
                "GET",
                "/a?b#c",
                &[("Host", "WWW.Example.Test"), ("Proxy-Flow", "x")],
            ),
            "www.example.test",
            443,
        )
        .unwrap();
        assert_eq!(r.url, "https://www.example.test/a?b");
        assert_eq!(r.path, "/a?b");
        assert_eq!(
            http::header(&r.headers, "host"),
            Some(&b"www.example.test"[..])
        );
        assert!(http::header(&r.headers, "proxy-flow").is_none());
        let r = decrypted_request(
            &head("GET", "https://www.example.test:8443/x", &[]),
            "www.example.test",
            8443,
        )
        .unwrap();
        assert_eq!(r.url, "https://www.example.test:8443/x");
        assert_eq!(
            http::header(&r.headers, "host"),
            Some(&b"www.example.test:8443"[..])
        );
        let e = decrypted_request(
            &head("GET", "/", &[("Host", "evil.test")]),
            "www.example.test",
            443,
        )
        .unwrap_err();
        assert_eq!(e.0, 421);
        assert_eq!(
            decrypted_request(&head("GET", "https://evil.test/", &[]), "a.test", 443)
                .unwrap_err()
                .0,
            421
        );
        assert_eq!(
            decrypted_request(&head("CONNECT", "a.test:443", &[]), "a.test", 443)
                .unwrap_err()
                .0,
            400
        );
        assert_eq!(
            decrypted_request(&head("GET", "http://a.test/", &[]), "a.test", 443)
                .unwrap_err()
                .0,
            400
        );
        assert!(
            decrypted_request(&head("GET", "/", &[("Connection", "close")]), "a.test", 443)
                .unwrap()
                .close
        );
    }

    #[tokio::test]
    async fn copy_body_streams_exact_framing() {
        let ctr = AtomicU64::new(0);
        let mut out = Vec::new();
        let mut src: &[u8] = b"lo\r\n0\r\n\r\nGARBAGE";
        let rest = copy_body(&mut src, &mut out, Framing::Chunked, b"5\r\nhel", &ctr)
            .await
            .unwrap();
        assert_eq!(out, b"5\r\nhello\r\n0\r\n\r\n");
        assert_eq!(rest, b"GARBAGE");
        assert_eq!(ctr.load(Ordering::Relaxed), out.len() as u64);

        let mut out = Vec::new();
        let mut src: &[u8] = b"defgh";
        let rest = copy_body(&mut src, &mut out, Framing::Length(6), b"abc", &ctr)
            .await
            .unwrap();
        assert_eq!(out, b"abcdef");
        assert!(rest.is_empty());

        let mut out = Vec::new();
        let mut src: &[u8] = b"";
        let rest = copy_body(&mut src, &mut out, Framing::Length(2), b"abGET /", &ctr)
            .await
            .unwrap();
        assert_eq!(
            (out.as_slice(), rest.as_slice()),
            (&b"ab"[..], &b"GET /"[..])
        );

        let mut out = Vec::new();
        let mut src: &[u8] = b"de";
        assert!(matches!(
            copy_body(&mut src, &mut out, Framing::Length(6), b"abc", &ctr).await,
            Err(CopyError::Src(_))
        ));
    }
}
