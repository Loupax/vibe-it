//! The front: accepts browser connections, reads the request head and dispatches.
//!
//! `CONNECT host:port`  -> [`tunnel`]: dial, `200`, splice both ways.
//! `GET http://...`     -> [`plain`]: one request per connection, streamed both ways, `Connection: close`.
//!
//! Where later steps plug in (see NOTES.md): W2-2 decides per CONNECT whether to terminate TLS (in
//! [`tunnel`], before the `200`); W2-F routes the CONNECT through tunnel filters inside the upstream
//! connector, and sends the parsed request of [`plain`] (and of decrypted tunnels) through the http chain
//! before the upstream connector.

use std::io;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::config::Config;
use crate::conn::Conn;
use crate::flow::{Flow, Kind, Registry};
use crate::host::{authority_host, normalise_host, split_authority};
use crate::http::{self, ChunkScanner, Framing, HeadError, Headers, RequestHead};
use crate::upstream;

/// How long a browser may take to send a complete request head.
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Userspace copy buffer for parsed (non-spliced) bodies.
const COPY_BUF: usize = 64 * 1024;

pub struct Ctx {
    pub cfg: Config,
    pub reg: Arc<Registry>,
}

/// Handle one browser connection from accept to close.
pub async fn handle(mut conn: Conn, peer: String, ctx: Arc<Ctx>) {
    let mut buf = Vec::new();
    let head = match tokio::time::timeout(HEAD_TIMEOUT, http::read_request(&mut conn, &mut buf)).await {
        Err(_) => {
            crate::debug!("peer={peer} no request head within {}s", HEAD_TIMEOUT.as_secs());
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
            refuse_flow(&mut conn, &flow, e.status(), &format!("wire2: {host}:{port}: {e}")).await;
            return;
        }
    };
    // W2-2: a decrypted host answers 200 here too, then terminates TLS instead of splicing.
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
    let r = tokio::select! {
        r = crate::splice::splice_bidi(&conn, &up, &flow.up, &flow.down) => r,
        _ = flow.cancel.cancelled() => Ok(()),
    };
    if let Err(e) = r {
        crate::debug!("flow={} tunnel ended: {e}", flow.id);
    }
}

// ---------------------------------------------------------------------------------------------------
// Plain http://

/// `http://host[:port]/path?q#frag` -> (authority, origin-form target without the fragment).
fn split_http_url(target: &str) -> Option<(&str, String)> {
    let scheme = target.get(..7)?;
    if !scheme.eq_ignore_ascii_case("http://") {
        return None;
    }
    let rest = &target[7..];
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

async fn put<W: AsyncWrite + Unpin>(w: &mut W, data: &[u8], ctr: &AtomicU64) -> Result<(), CopyError> {
    if data.is_empty() {
        return Ok(());
    }
    w.write_all(data).await.map_err(CopyError::Dst)?;
    ctr.fetch_add(data.len() as u64, Ordering::Relaxed);
    Ok(())
}

/// Stream one message body from `r` to `w` exactly as framed (bytes unchanged, chunked included).
/// `pre` holds body bytes already read together with the head.
async fn copy_body<R, W>(r: &mut R, w: &mut W, framing: Framing, pre: &[u8], ctr: &AtomicU64) -> Result<(), CopyError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let eof = || CopyError::Src(io::Error::new(io::ErrorKind::UnexpectedEof, "body ended early"));
    match framing {
        Framing::None => {}
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
        }
        Framing::Chunked => {
            let mut sc = ChunkScanner::new();
            match sc.feed(pre).map_err(CopyError::Framing)? {
                Some(end) => return put(w, &pre[..end], ctr).await.and(flush(w).await),
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
                        break;
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
                    break;
                }
                put(w, &buf[..got], ctr).await?;
            }
        }
    }
    flush(w).await
}

async fn flush<W: AsyncWrite + Unpin>(w: &mut W) -> Result<(), CopyError> {
    w.flush().await.map_err(CopyError::Dst)
}

/// Plain byte copy until EOF, then half-close `w` (the two directions of an upgraded connection).
async fn copy_until_eof<R, W>(r: &mut R, w: &mut W, pre: &[u8], ctr: &AtomicU64) -> Result<(), CopyError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    copy_body(r, w, Framing::UntilClose, pre, ctr).await?;
    w.shutdown().await.map_err(CopyError::Dst)
}

enum Response {
    Final(http::ResponseHead, Vec<u8>),
    Upgrade(http::ResponseHead, Vec<u8>),
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
    let framing = match http::request_framing(&head.headers) {
        Ok(f) => f,
        Err(e) => {
            crate::warn!("flow={} bad request body framing: {e}", flow.id);
            refuse_flow(&mut conn, &flow, 400, &format!("wire2: {e}")).await;
            return;
        }
    };
    let upgrade = http::is_upgrade_request(&head.headers);
    let mut hs: Headers = http::strip_hop_by_hop(&head.headers, upgrade);
    hs.retain(|(n, _)| !n.eq_ignore_ascii_case("host"));
    let host_hdr = match port {
        80 => authority_host(&host),
        p => format!("{}:{p}", authority_host(&host)),
    };
    hs.insert(0, ("Host".into(), host_hdr.into_bytes()));
    hs.push(("Connection".into(), if upgrade { &b"upgrade"[..] } else { b"close" }.to_vec()));
    http::drop_length_if_chunked(&mut hs);
    let up_head = http::write_request_head(&head.method, &path, &hs);
    crate::debug!("flow={} -> {} {} (host {host}:{port}, body {framing:?})", flow.id, head.method, path);

    // W2-F: the http filter chain runs here, before the upstream connector.
    let up = tokio::select! {
        r = upstream::dial(&host, port, ctx.cfg.dial_timeout) => r,
        _ = flow.cancel.cancelled() => return,
    };
    let up = match up {
        Ok(s) => s,
        Err(e) => {
            log_upstream_error(&flow, &e);
            refuse_flow(&mut conn, &flow, e.status(), &format!("wire2: {host}:{port}: {e}")).await;
            return;
        }
    };
    let (mut ur, mut uw) = up.into_split();
    let (mut br, mut bw) = tokio::io::split(conn);

    // Request: head + body, streamed, concurrently with waiting for the response (a server may answer
    // before it has read the whole body).
    let mut send = Box::pin(async {
        put(&mut uw, &up_head, &flow.up).await?;
        if upgrade && framing == Framing::None {
            put(&mut uw, &leftover, &flow.up).await?;
        } else {
            copy_body(&mut br, &mut uw, framing, &leftover, &flow.up).await?;
        }
        flush(&mut uw).await
    });
    let mut send_done = false;

    let resp = {
        let mut recv = pin!(async {
            let mut rbuf = Vec::new();
            loop {
                let rh = http::read_response(&mut ur, &mut rbuf).await.map_err(|e| e.to_string())?;
                match rh.code {
                    101 if upgrade => return Ok(Response::Upgrade(rh, rbuf)),
                    101 => return Err("101 to a request that did not ask to upgrade".to_string()),
                    100..=199 => {
                        // Interim responses (100 Continue, 103 Early Hints) are relayed, minus hop-by-hop.
                        let ih = http::write_response_head(rh.code, &rh.reason, &http::strip_hop_by_hop(&rh.headers, false));
                        bw.write_all(&ih).await.map_err(|e| format!("browser: {e}"))?;
                        flow.down.fetch_add(ih.len() as u64, Ordering::Relaxed);
                    }
                    _ => return Ok(Response::Final(rh, rbuf)),
                }
            }
        });
        loop {
            tokio::select! {
                r = &mut send, if !send_done => {
                    send_done = true;
                    match r {
                        Ok(()) => {}
                        Err(CopyError::Dst(e)) => {
                            // Upstream stopped reading; it may still have answered.
                            crate::debug!("flow={} upstream stopped reading the request: {e}", flow.id);
                        }
                        Err(e) => {
                            crate::debug!("flow={} request from browser failed: {e}", flow.id);
                            return;
                        }
                    }
                }
                r = &mut recv => break r,
                _ = flow.cancel.cancelled() => return,
            }
        }
    };

    let (rh, rbuf, is_upgrade) = match resp {
        Ok(Response::Final(rh, rbuf)) => (rh, rbuf, false),
        Ok(Response::Upgrade(rh, rbuf)) => (rh, rbuf, true),
        Err(e) => {
            crate::warn!("flow={} host={host} port={port} bad upstream response, answering 502: {e}", flow.id);
            drop(send);
            let mut conn = br.unsplit(bw);
            refuse_flow(&mut conn, &flow, 502, &format!("wire2: {host}:{port}: bad upstream response")).await;
            return;
        }
    };

    if is_upgrade {
        let mut hs = http::strip_hop_by_hop(&rh.headers, true);
        hs.push(("Connection".into(), b"upgrade".to_vec()));
        let h = http::write_response_head(101, &rh.reason, &hs);
        if put(&mut bw, &h, &flow.down).await.is_err() {
            return;
        }
        flow.set_status(101);
        drop(send);
        let r = tokio::select! {
            r = async {
                tokio::try_join!(
                    copy_until_eof(&mut br, &mut uw, &[], &flow.up),
                    copy_until_eof(&mut ur, &mut bw, &rbuf, &flow.down),
                )
            } => r.map(|_| ()),
            _ = flow.cancel.cancelled() => Ok(()),
        };
        if let Err(e) = r {
            crate::debug!("flow={} upgraded connection ended: {e}", flow.id);
        }
        return;
    }

    let rframing = match http::response_framing(&head.method, rh.code, &rh.headers) {
        Ok(f) => f,
        Err(e) => {
            crate::warn!("flow={} host={host} port={port} bad upstream framing, answering 502: {e}", flow.id);
            drop(send);
            let mut conn = br.unsplit(bw);
            refuse_flow(&mut conn, &flow, 502, &format!("wire2: {host}:{port}: bad upstream response")).await;
            return;
        }
    };
    let mut hs = http::strip_hop_by_hop(&rh.headers, false);
    http::drop_length_if_chunked(&mut hs);
    hs.push(("Connection".into(), b"close".to_vec()));
    let h = http::write_response_head(rh.code, &rh.reason, &hs);
    if put(&mut bw, &h, &flow.down).await.is_err() {
        return;
    }
    flow.set_status(rh.code);
    crate::debug!("flow={} <- {} (body {rframing:?})", flow.id, rh.code);

    let r = {
        let mut body = pin!(copy_body(&mut ur, &mut bw, rframing, &rbuf, &flow.down));
        loop {
            tokio::select! {
                r = &mut send, if !send_done => {
                    send_done = true;
                    if let Err(e @ (CopyError::Src(_) | CopyError::Framing(_))) = r {
                        crate::debug!("flow={} request from browser failed: {e}", flow.id);
                        return;
                    }
                }
                r = &mut body => break r,
                _ = flow.cancel.cancelled() => return,
            }
        }
    };
    if let Err(e) = r {
        crate::debug!("flow={} response body ended: {e}", flow.id);
        return;
    }
    drop(send);
    let mut conn = br.unsplit(bw);
    finish(&mut conn).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_urls() {
        assert_eq!(split_http_url("http://a.test/x?y#z"), Some(("a.test", "/x?y".to_string())));
        assert_eq!(split_http_url("HTTP://A.test:81"), Some(("A.test:81", "/".to_string())));
        assert_eq!(split_http_url("http://[::1]:8080?q"), Some(("[::1]:8080", "/?q".to_string())));
        assert_eq!(split_http_url("http://a#frag"), Some(("a", "/".to_string())));
        assert_eq!(split_http_url("https://a/"), None);
        assert_eq!(split_http_url("/relative"), None);
    }

    #[tokio::test]
    async fn copy_body_streams_exact_framing() {
        let ctr = AtomicU64::new(0);
        let mut out = Vec::new();
        let mut src: &[u8] = b"lo\r\n0\r\n\r\nGARBAGE";
        copy_body(&mut src, &mut out, Framing::Chunked, b"5\r\nhel", &ctr).await.unwrap();
        assert_eq!(out, b"5\r\nhello\r\n0\r\n\r\n");
        assert_eq!(ctr.load(Ordering::Relaxed), out.len() as u64);

        let mut out = Vec::new();
        let mut src: &[u8] = b"defgh";
        copy_body(&mut src, &mut out, Framing::Length(6), b"abc", &ctr).await.unwrap();
        assert_eq!(out, b"abcdef");

        let mut out = Vec::new();
        let mut src: &[u8] = b"de";
        assert!(matches!(
            copy_body(&mut src, &mut out, Framing::Length(6), b"abc", &ctr).await,
            Err(CopyError::Src(_))
        ));
    }
}
