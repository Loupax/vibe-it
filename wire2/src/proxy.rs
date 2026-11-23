//! The front: accepts browser connections, reads the request head and dispatches.
//!
//! `CONNECT host:port`  -> [`tunnel`]: one decrypt decision ([`crate::decrypt::Policy::decide`]), then
//!                         either dial, `200`, splice both ways (opaque), or [`decrypted`].
//! `GET http://...`     -> [`plain`]: one request per connection, streamed both ways, `Connection: close`.
//! decrypted tunnel     -> [`decrypted`]: `200`, TLS toward the browser with a minted leaf, then each
//!                         HTTP/1.1 request on that connection goes through [`forward`].
//!
//! [`forward`] is the one place every parsed request and its response pass through, plain or decrypted.
//! It hands the request to [`http_from`], which sends it through the flow's http filters (each forwards
//! it back to the hub, [`crate::hub`], which calls [`http_from`] for the next stage) and finally to the
//! upstream connector [`connect_from`], which opens the server connection as `CONNECT host:port`
//! through the flow's tunnel filters (the hub again continues each stage), then dials the server.
//! Undecrypted browser CONNECTs use [`connect_from`] directly and splice the browser to its result.

use std::io;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;

use crate::config::Config;
use crate::conn::{Conn, Prefixed};
use crate::decrypt::{Decision, Policy};
use crate::filter::{Filter, Filters, Layer, Route, StageState, Use, stage_wait};
use crate::flow::{Flow, FlowInfo, Kind, Registry};
use crate::host::{authority_host, normalise_host, split_authority};
use crate::http::{self, ChunkScanner, Framing, HeadError, Headers, RequestHead};
use crate::upstream;

/// How long a browser (or a filter, at the hub) may take to send a complete request head (and,
/// decrypted, its TLS handshake).
pub const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a decrypted browser connection may sit idle between requests.
const KEEPALIVE_IDLE: Duration = Duration::from_secs(120);
/// Userspace copy buffer for parsed (non-spliced) bodies.
const COPY_BUF: usize = 64 * 1024;

pub struct Ctx {
    pub cfg: Config,
    pub reg: Arc<Registry>,
    pub policy: Arc<Policy>,
    pub filters: Arc<Filters>,
}

/// Byte counters a hop adds to: the flow's own for the browser-facing hop (the access log counts what
/// the browser sent and got), scratch ones for hops the hub runs.
#[derive(Clone, Copy)]
pub struct Ctr<'a> {
    pub up: &'a AtomicU64,
    pub down: &'a AtomicU64,
}

impl<'a> Ctr<'a> {
    fn of(flow: &'a FlowInfo) -> Ctr<'a> {
        Ctr {
            up: &flow.up,
            down: &flow.down,
        }
    }
}

/// A hop that could not deliver: the status to answer with and why (the body says `wire2: <msg>`).
pub struct HopErr {
    pub status: u16,
    pub msg: String,
}

impl HopErr {
    pub fn body(&self) -> String {
        format!("wire2: {}", self.msg)
    }
}

/// `Proxy-Flow` value for `flow` (SPEC "Traffic": the proxy's id, a slash, the flow number).
fn flow_token(ctx: &Ctx, flow: &FlowInfo) -> String {
    format!("{}/{}", ctx.filters.proxy_id, flow.id)
}

/// A down fail-closed filter refuses the flow: 503 naming it (SPEC "Failure handling").
fn down_err(flow: &FlowInfo, name: &str) -> HopErr {
    crate::warn!(
        "flow={} host={} port={} refused with 503: filter {name} is down and fails closed",
        flow.id,
        flow.host,
        flow.port
    );
    HopErr {
        status: 503,
        msg: format!(
            "filter {name} is down (fail closed), so {} is refused",
            flow.host
        ),
    }
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
pub async fn refuse(conn: &mut Conn, code: u16, body: &str) -> usize {
    let resp = http::error_response(code, body);
    if conn.write_all(&resp).await.is_err() {
        return 0;
    }
    finish(conn).await;
    resp.len()
}

/// Half-close and give the browser a moment to read the response before the socket goes away, so
/// unread request bytes on our side do not turn the close into a reset that eats the response.
pub async fn finish<S: AsyncRead + AsyncWrite + Unpin>(conn: &mut S) {
    let (r, w) = tokio::io::split(conn);
    finish_halves(r, w).await;
}

/// [`finish`] for a connection already split into its two halves.
pub async fn finish_halves<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(mut r: R, mut w: W) {
    let _ = w.shutdown().await;
    let mut sink = [0u8; 4096];
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        let mut total = 0usize;
        while total < 1 << 20 {
            match r.read(&mut sink).await {
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

fn log_upstream_error(flow: &FlowInfo, e: &upstream::DialError) {
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
    // The flow's chain, once (SPEC "Routing"); a down fail-closed filter covering the host refuses it.
    let route = match ctx.filters.route(&host, true, &ctx.policy).await {
        Ok(r) => r,
        Err(name) => {
            let flow = ctx.reg.open(
                Kind::Tunnel,
                "CONNECT",
                &host,
                port,
                false,
                Route::default(),
            );
            let e = down_err(&flow, &name);
            refuse_flow(&mut conn, &flow, e.status, &e.body()).await;
            return;
        }
    };
    // The one decrypt decision of this CONNECT (SPEC: made per CONNECT, never changed later).
    if let Decision::Decrypt { by } = ctx.policy.decide(&host) {
        drop(head);
        decrypted(conn, host, port, leftover, &by, route, ctx).await;
        return;
    }
    // Not decrypted: http filters never see this flow.
    let flow = ctx.reg.open(
        Kind::Tunnel,
        "CONNECT",
        &host,
        port,
        false,
        route.tunnel_only(),
    );
    let up = tokio::select! {
        r = connect_from(ctx, &flow, 0) => r,
        _ = flow.cancel.cancelled() => return,
    };
    let (up, pre) = match up {
        Ok(x) => x,
        Err(e) => {
            refuse_flow(&mut conn, &flow, e.status, &e.body()).await;
            return;
        }
    };
    const OK: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";
    if conn.write_all(OK).await.is_err() {
        return;
    }
    flow.set_status(200);
    flow.down.fetch_add(OK.len() as u64, Ordering::Relaxed);
    // Bytes a filter passed on right behind its 2xx belong to the browser.
    if !pre.is_empty() {
        if conn.write_all(&pre).await.is_err() {
            return;
        }
        flow.down.fetch_add(pre.len() as u64, Ordering::Relaxed);
    }
    drop(pre);
    let mut up = up;
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
    route: Route,
    ctx: &Ctx,
) {
    let flow = ctx
        .reg
        .open(Kind::Tunnel, "CONNECT", &host, port, true, route);
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
        Ok(Err(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
            // Browsers open speculative connections and close some before any ClientHello.
            crate::debug!(
                "flow={} host={host} browser closed before the TLS handshake",
                flow.id
            );
            return;
        }
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
                let next = forward(
                    &mut br,
                    &mut bw,
                    std::mem::take(&mut buf),
                    &req,
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
    let route = match ctx.filters.route(&host, false, &ctx.policy).await {
        Ok(r) => r,
        Err(name) => {
            let flow = ctx.reg.open(
                Kind::Http,
                &head.method,
                &host,
                port,
                false,
                Route::default(),
            );
            let e = down_err(&flow, &name);
            refuse_flow(&mut conn, &flow, e.status, &e.body()).await;
            return;
        }
    };
    let flow = ctx
        .reg
        .open(Kind::Http, &head.method, &host, port, false, route);
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
    let next = forward(
        &mut br,
        &mut bw,
        leftover,
        &req,
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

/// A request as it reaches the http chain and the upstream connector, plain or decrypted. Filters get
/// it in absolute form (`url`), the server in origin form (`path`).
#[derive(Debug)]
pub struct Request {
    method: String,
    /// Full URL (`http://host[:port]/path`, `https://host[:port]/path`), for logs and filters.
    url: String,
    /// Origin-form target sent upstream.
    path: String,
    /// Headers for the next hop: hop-by-hop and every `Proxy-*` removed, `Host` from the URL,
    /// `Connection` set.
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

/// A request a filter forwarded to the hub, for the flow it belongs to. The target may be absolute
/// (as the filter got it) or origin-form; a filter may change anything but the destination (scheme,
/// host and port stay the flow's: tunnel filters and the decrypt decision were chosen for them).
pub fn hop_request(head: &RequestHead, flow: &FlowInfo) -> Result<Request, (u16, String)> {
    let (scheme, default_port) = if flow.decrypted {
        ("https://", 443)
    } else {
        ("http://", 80)
    };
    let path = if head.target.starts_with('/') {
        head.target.split('#').next().unwrap_or("/").to_string()
    } else if let Some((auth, path)) = split_url(&head.target, scheme) {
        let (h, p) = split_authority(auth)
            .and_then(|(h, p)| Ok((normalise_host(h)?, p.unwrap_or(default_port))))
            .map_err(|e| (400, e.to_string()))?;
        if h != flow.host || p != flow.port {
            return Err((
                502,
                format!(
                    "a filter changed the destination to {h}:{p} (the flow is for {}:{})",
                    flow.host, flow.port
                ),
            ));
        }
        path
    } else {
        return Err((
            400,
            format!(
                "bad request target {:?} (expected {scheme}{}...)",
                head.target, flow.host
            ),
        ));
    };
    if head.method.eq_ignore_ascii_case("CONNECT") {
        return Err((400, "CONNECT at the http layer".into()));
    }
    let auth = if flow.port == default_port {
        authority_host(&flow.host)
    } else {
        format!("{}:{}", authority_host(&flow.host), flow.port)
    };
    let url = format!("{scheme}{auth}{path}");
    build_request(head, url, path, auth).map_err(|e| (400, e))
}

/// What the browser connection does after one exchange.
pub enum Next {
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

/// The one place every parsed request from the browser passes through on its way to the origin, and
/// its response on the way back: the flow's http filters in chain order, then the upstream connector.
/// `status` receives the response status (the flow's for plain http, a per-request slot for decrypted
/// tunnels). `keep_alive`: the browser connection may carry further requests.
#[allow(clippy::too_many_arguments)]
async fn forward<BR, BW>(
    br: &mut BR,
    bw: &mut BW,
    leftover: Vec<u8>,
    req: &Request,
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
    http_from(
        br,
        bw,
        leftover,
        req,
        ctx,
        flow,
        Ctr::of(flow),
        status,
        keep_alive,
        0,
    )
    .await
}

/// Send `req` to stage `stage` of the flow's http chain (a filter that is down is skipped when it
/// fails open, and refuses with 503 when it fails closed); past the last stage, to the origin through
/// [`connect_from`]. `br`/`bw` is the side the request came from: the browser, or (at the hub) the
/// filter of the previous stage.
#[allow(clippy::too_many_arguments)]
pub async fn http_from<BR, BW>(
    br: &mut BR,
    bw: &mut BW,
    leftover: Vec<u8>,
    req: &Request,
    ctx: &Ctx,
    flow: &FlowInfo,
    ctr: Ctr<'_>,
    status: &AtomicU16,
    keep_alive: bool,
    mut stage: usize,
) -> Next
where
    BR: AsyncRead + Unpin,
    BW: AsyncWrite + Unpin,
{
    let chain = &flow.route.http;
    let mut redials = 0;
    while stage < chain.len() {
        let (f, name) = &chain[stage];
        match f.usable() {
            Use::Skip => {
                crate::debug!(
                    "flow={} skipping http filter {name}: down, fails open",
                    flow.id
                );
                stage += 1;
                continue;
            }
            Use::Refuse(n) => {
                let e = down_err(flow, &n);
                return answer(bw, ctr.down, status, e.status, &e.body()).await;
            }
            Use::Send => {}
        }
        let s = tokio::select! {
            r = f.connect() => r,
            _ = flow.cancel.cancelled() => return Next::Abort,
        };
        let s = match s {
            Ok(s) => s,
            Err(e) => {
                // Gone: mark it down and apply its failure policy to this request.
                f.mark_down(&format!("connect: {e}"));
                redials += 1;
                if redials > 2 {
                    let body = format!("wire2: filter {name}: {e}");
                    return answer(bw, ctr.down, status, 502, &body).await;
                }
                continue;
            }
        };
        let (_expect, rx) = flow.route.expect(Layer::Http, stage);
        let mut hs = req.headers.clone();
        hs.push(("Proxy-Flow".into(), flow_token(ctx, flow).into_bytes()));
        let head = http::write_request_head(&req.method, &req.url, &hs);
        crate::debug!("flow={} -> http filter {name}", flow.id);
        let (ur, uw) = Conn::Unix(s).into_split();
        let hop = Hop::Filter {
            f,
            name,
            rx,
            timeout: ctx.filters.stage_timeout(),
        };
        return exchange(
            br,
            bw,
            leftover,
            head,
            req,
            (ur, uw, Vec::new()),
            flow,
            ctr,
            status,
            keep_alive,
            hop,
        )
        .await;
    }
    let up = tokio::select! {
        r = connect_from(ctx, flow, 0) => r,
        _ = flow.cancel.cancelled() => return Next::Abort,
    };
    let (conn, pre) = match up {
        Ok(x) => x,
        Err(e) => return answer(bw, ctr.down, status, e.status, &e.body()).await,
    };
    let head = http::write_request_head(&req.method, &req.path, &req.headers);
    if flow.decrypted {
        let tls = tokio::select! {
            r = upstream::tls(Prefixed::new(pre, conn), &flow.host, ctx.cfg.dial_timeout) => r,
            _ = flow.cancel.cancelled() => return Next::Abort,
        };
        match tls {
            Ok(s) => {
                let (ur, uw) = tokio::io::split(s);
                exchange(
                    br,
                    bw,
                    leftover,
                    head,
                    req,
                    (ur, uw, Vec::new()),
                    flow,
                    ctr,
                    status,
                    keep_alive,
                    Hop::Origin,
                )
                .await
            }
            Err(e) => {
                log_upstream_error(flow, &e);
                let body = format!("wire2: {}:{}: {e}", flow.host, flow.port);
                answer(bw, ctr.down, status, e.status(), &body).await
            }
        }
    } else {
        // Bytes a tunnel filter passed on behind its 2xx are the start of the server's response.
        let (ur, uw) = conn.into_split();
        exchange(
            br,
            bw,
            leftover,
            head,
            req,
            (ur, uw, pre),
            flow,
            ctr,
            status,
            keep_alive,
            Hop::Origin,
        )
        .await
    }
}

/// The upstream connector (SPEC "Path of a flow", step 3): open the connection to the flow's origin
/// through its tunnel filters from `stage` on, each sent `CONNECT host:port` (a 2xx turns that filter
/// connection into the byte pipe; the filter forwards to the hub, which continues with the next stage),
/// then dial the server. Returns the connection and any bytes that arrived behind the last 2xx.
pub async fn connect_from(
    ctx: &Ctx,
    flow: &FlowInfo,
    mut stage: usize,
) -> Result<(Conn, Vec<u8>), HopErr> {
    let chain = &flow.route.tunnel;
    let auth = format!("{}:{}", authority_host(&flow.host), flow.port);
    let mut redials = 0;
    while stage < chain.len() {
        let (f, name) = &chain[stage];
        match f.usable() {
            Use::Skip => {
                crate::debug!(
                    "flow={} skipping tunnel filter {name}: down, fails open",
                    flow.id
                );
                stage += 1;
                continue;
            }
            Use::Refuse(n) => return Err(down_err(flow, &n)),
            Use::Send => {}
        }
        let mut s = match f.connect().await {
            Ok(s) => s,
            Err(e) => {
                f.mark_down(&format!("connect: {e}"));
                redials += 1;
                if redials > 2 {
                    return Err(HopErr {
                        status: 502,
                        msg: format!("filter {name}: {e}"),
                    });
                }
                continue;
            }
        };
        let (_expect, rx) = flow.route.expect(Layer::Tunnel, stage);
        let req = format!(
            "CONNECT {auth} HTTP/1.1\r\nHost: {auth}\r\nProxy-Flow: {}\r\n\r\n",
            flow_token(ctx, flow)
        );
        crate::debug!("flow={} -> tunnel filter {name}: CONNECT {auth}", flow.id);
        if let Err(e) = s.write_all(req.as_bytes()).await {
            f.mark_down(&format!("write: {e}"));
            return Err(lost(flow, name, &e.to_string()));
        }
        let mut buf = Vec::new();
        let timeout = ctx.filters.stage_timeout();
        let r = stage_wait(http::read_response(&mut s, &mut buf), rx, timeout).await;
        return match r {
            None => Err(stage_timed_out(flow, name, timeout)),
            Some(Err(HeadError::Eof | HeadError::Truncated | HeadError::Io(_))) => {
                f.mark_down("closed the connection before response headers");
                Err(lost(flow, name, "closed before response headers"))
            }
            Some(Err(e)) => Err(malformed(flow, name, &e.to_string())),
            Some(Ok(rh)) if (200..300).contains(&rh.code) => Ok((Conn::Unix(s), buf)),
            Some(Ok(rh)) => {
                crate::info!(
                    "flow={} CONNECT {auth} refused at tunnel filter {name}: {} {}",
                    flow.id,
                    rh.code,
                    rh.reason
                );
                Err(HopErr {
                    status: if rh.code < 200 { 502 } else { rh.code },
                    msg: format!("{auth}: refused ({}) at filter {name}", rh.code),
                })
            }
        };
    }
    match upstream::dial(&flow.host, flow.port, ctx.cfg.dial_timeout).await {
        Ok(s) => Ok((Conn::Tcp(s), Vec::new())),
        Err(e) => {
            log_upstream_error(flow, &e);
            Err(HopErr {
                status: e.status(),
                msg: format!("{auth}: {e}"),
            })
        }
    }
}

/// SPEC "Failure handling": a filter connection that fails or closes before response headers -> 502.
fn lost(flow: &FlowInfo, name: &str, why: &str) -> HopErr {
    crate::warn!(
        "flow={} host={} port={} filter {name} failed ({why}), answering 502",
        flow.id,
        flow.host,
        flow.port
    );
    HopErr {
        status: 502,
        msg: format!("filter {name} failed: {why}"),
    }
}

/// SPEC "Failure handling": no response headers within `--stage-timeout` -> 504; the filter stays up.
fn stage_timed_out(flow: &FlowInfo, name: &str, t: Duration) -> HopErr {
    crate::warn!(
        "flow={} host={} port={} filter {name} sent no response headers within --stage-timeout {}ms, \
         answering 504",
        flow.id,
        flow.host,
        flow.port,
        t.as_millis()
    );
    HopErr {
        status: 504,
        msg: format!("filter {name} did not answer in time"),
    }
}

/// SPEC "Failure handling": malformed HTTP from a filter -> 502, connection closed, logged.
fn malformed(flow: &FlowInfo, name: &str, why: &str) -> HopErr {
    crate::error!(
        "flow={} host={} port={} malformed HTTP from filter {name}: {why}; answering 502",
        flow.id,
        flow.host,
        flow.port
    );
    HopErr {
        status: 502,
        msg: format!("malformed response from filter {name}"),
    }
}

/// The next hop of an exchange: a filter (under `--stage-timeout`) or the origin.
enum Hop<'a> {
    Origin,
    Filter {
        f: &'a Filter,
        name: &'a str,
        rx: watch::Receiver<StageState>,
        timeout: Duration,
    },
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
                // Flush every chunk: with TLS on `w` (decrypted tunnels) the encrypted tail of a write
                // that did not fit in the socket buffer stays pending until a flush, and an upgraded
                // connection may send nothing more for a long time (found by W2-3: a 32 MiB WebSocket
                // message stalled ~16 KiB short).
                flush(w).await?;
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

/// Why no usable response head came back.
enum RecvErr {
    Head(HeadError),
    /// The stage sent no response headers within `--stage-timeout`.
    Timeout,
    /// `101` to a request that did not ask to upgrade.
    Unexpected101,
    /// Relaying an interim response to our side failed.
    Back,
}

/// One request/response exchange over an open connection to the next hop (`up`: read half, write
/// half, bytes of the response already read), both bodies streamed. `up_head` is the request head as
/// that hop gets it (absolute form plus `Proxy-Flow` for a filter, origin form for the server).
#[allow(clippy::too_many_arguments)]
async fn exchange<BR, BW, UR, UW>(
    br: &mut BR,
    bw: &mut BW,
    leftover: Vec<u8>,
    up_head: Vec<u8>,
    req: &Request,
    up: (UR, UW, Vec<u8>),
    flow: &FlowInfo,
    ctr: Ctr<'_>,
    status: &AtomicU16,
    keep_alive: bool,
    hop: Hop<'_>,
) -> Next
where
    BR: AsyncRead + Unpin,
    BW: AsyncWrite + Unpin,
    UR: AsyncRead + Unpin + Send + 'static,
    UW: AsyncWrite + Unpin,
{
    let (mut ur, mut uw, rpre) = up;
    // Request: head + body, streamed, concurrently with waiting for the response (a server may answer
    // before it has read the whole body).
    let mut send = Box::pin(async {
        put(&mut uw, &up_head, ctr.up).await?;
        let rest = if req.upgrade && req.framing == Framing::None {
            put(&mut uw, &leftover, ctr.up).await?;
            Vec::new()
        } else {
            copy_body(&mut *br, &mut uw, req.framing, &leftover, ctr.up).await?
        };
        flush(&mut uw).await?;
        Ok::<_, CopyError>(rest)
    });
    let mut sent: Option<Result<Vec<u8>, CopyError>> = None;

    let resp = {
        let hop = &hop;
        let mut recv = pin!(async {
            let mut rbuf = rpre;
            let mut first = true;
            loop {
                let rh = match (first, hop) {
                    (true, Hop::Filter { rx, timeout, .. }) => {
                        let r = http::read_response(&mut ur, &mut rbuf);
                        match stage_wait(r, rx.clone(), *timeout).await {
                            None => return Err(RecvErr::Timeout),
                            Some(r) => r.map_err(RecvErr::Head)?,
                        }
                    }
                    _ => http::read_response(&mut ur, &mut rbuf)
                        .await
                        .map_err(RecvErr::Head)?,
                };
                first = false;
                match rh.code {
                    101 if req.upgrade => return Ok(Response::Upgrade(rh, rbuf)),
                    101 => return Err(RecvErr::Unexpected101),
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
                            .map_err(|_| RecvErr::Back)?;
                        ctr.down.fetch_add(ih.len() as u64, Ordering::Relaxed);
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
                            // The next hop stopped reading; it may still have answered.
                            crate::debug!("flow={} next hop stopped reading the request: {e}", flow.id);
                            sent = Some(Err(CopyError::Dst(e)));
                        }
                        Err(e) => {
                            crate::debug!("flow={} request body failed: {e}", flow.id);
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
        Err(RecvErr::Back) => return Next::Abort,
        Err(e) => {
            drop(send);
            let err = match (&hop, e) {
                (Hop::Filter { name, timeout, .. }, RecvErr::Timeout) => {
                    stage_timed_out(flow, name, *timeout)
                }
                (
                    Hop::Filter { f, name, .. },
                    RecvErr::Head(HeadError::Eof | HeadError::Truncated | HeadError::Io(_)),
                ) => {
                    f.mark_down("closed the connection before response headers");
                    lost(flow, name, "closed before response headers")
                }
                (Hop::Filter { name, .. }, RecvErr::Head(h)) => {
                    malformed(flow, name, &h.to_string())
                }
                (Hop::Filter { name, .. }, _) => {
                    malformed(flow, name, "101 to a request that did not upgrade")
                }
                (Hop::Origin, e) => {
                    let why = match e {
                        RecvErr::Head(h) => h.to_string(),
                        _ => "101 to a request that did not ask to upgrade".to_string(),
                    };
                    crate::warn!(
                        "flow={} host={} port={} bad upstream response, answering 502: {why}",
                        flow.id,
                        flow.host,
                        flow.port
                    );
                    HopErr {
                        status: 502,
                        msg: format!("{}:{}: bad upstream response", flow.host, flow.port),
                    }
                }
            };
            return answer(bw, ctr.down, status, err.status, &err.body()).await;
        }
    };

    if is_upgrade {
        let mut hs = http::strip_hop_by_hop(&rh.headers, true);
        hs.push(("Connection".into(), b"upgrade".to_vec()));
        let h = http::write_response_head(101, &rh.reason, &hs);
        if put(bw, &h, ctr.down).await.is_err() || flush(bw).await.is_err() {
            return Next::Abort;
        }
        status.store(101, Ordering::Relaxed);
        drop(send);
        let r = tokio::select! {
            r = crate::splice::both_ways(
                copy_until_eof(br, &mut uw, &[], ctr.up),
                copy_until_eof(&mut ur, bw, &rbuf, ctr.down),
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
            drop(send);
            let err = match &hop {
                Hop::Filter { name, .. } => malformed(flow, name, &e),
                Hop::Origin => {
                    crate::warn!(
                        "flow={} host={} port={} bad upstream framing, answering 502: {e}",
                        flow.id,
                        flow.host,
                        flow.port
                    );
                    HopErr {
                        status: 502,
                        msg: format!("{}:{}: bad upstream response", flow.host, flow.port),
                    }
                }
            };
            return answer(bw, ctr.down, status, err.status, &err.body()).await;
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
    if put(bw, &h, ctr.down).await.is_err() {
        return Next::Abort;
    }
    status.store(rh.code, Ordering::Relaxed);
    crate::debug!("flow={} <- {} (body {rframing:?})", flow.id, rh.code);

    let r = {
        let mut body = pin!(pipelined_body(ur, bw, rframing, rbuf, ctr.down));
        loop {
            tokio::select! {
                r = &mut send, if sent.is_none() => {
                    if let Err(e @ (CopyError::Src(_) | CopyError::Framing(_))) = &r {
                        crate::debug!("flow={} request body failed: {e}", flow.id);
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
