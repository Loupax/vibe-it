//! The hub (SPEC "Filter protocol: Traffic"): the proxy's own Unix socket that filters forward requests
//! to. Each connection carries one request with the `Proxy-Flow` the proxy gave the filter. The hub
//! refuses (403) any request whose `Proxy-Flow` it did not issue, or for a flow that is not waiting for
//! a request from a filter at that layer; otherwise it strips every `Proxy-*` header and sends the
//! request on to the flow's next stage (the next filter, or the server), streaming the answer back.
//!
//! `CONNECT` is the tunnel layer: the next stage is [`proxy::connect_from`]; on success the hub answers
//! `200` and splices the filter's connection to it. Anything else is the http layer:
//! [`proxy::http_from`] with the filter's connection in the browser's place.

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;

use crate::conn::{self, Conn};
use crate::filter::{Layer, StageState, check_peer, check_socket_dir};
use crate::flow::FlowInfo;
use crate::host::{normalise_host, split_authority};
use crate::http::{self, HeadError, RequestHead};
use crate::proxy::{self, Ctr, Ctx, HEAD_TIMEOUT, Next};

/// Create the hub socket (mode 0600) after the socket-trust check of its directory.
pub fn bind(path: &Path) -> Result<UnixListener, String> {
    check_socket_dir(path)?;
    conn::bind_unix("--hub", path)
}

pub async fn serve(l: UnixListener, ctx: Arc<Ctx>) {
    loop {
        match l.accept().await {
            Ok((s, _)) => {
                if let Err(e) = check_peer(&s) {
                    crate::warn!("hub: refusing a connection: {e}");
                    continue;
                }
                tokio::spawn(handle(s, ctx.clone()));
            }
            Err(e) => {
                crate::warn!("hub accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// The flow a `Proxy-Flow` names, if this proxy issued it and the flow is still open.
fn issued_flow(ctx: &Ctx, head: &RequestHead) -> Result<Arc<FlowInfo>, String> {
    let mut values = head
        .headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("proxy-flow"));
    let v = values.next().ok_or("no Proxy-Flow header")?.1.as_slice();
    if values.next().is_some() {
        return Err("more than one Proxy-Flow header".into());
    }
    let v = String::from_utf8_lossy(v);
    let v = v.trim();
    let not_ours = || format!("Proxy-Flow {v:?} was not issued by this proxy");
    let (id, n) = v.split_once('/').ok_or_else(not_ours)?;
    if id != ctx.filters.proxy_id {
        return Err(not_ours());
    }
    let n: u64 = n.parse().map_err(|_| not_ours())?;
    ctx.reg
        .get(n)
        .ok_or_else(|| format!("Proxy-Flow {v:?}: no such open flow"))
}

async fn handle(s: UnixStream, ctx: Arc<Ctx>) {
    let mut conn = Conn::Unix(s);
    let mut buf = Vec::new();
    let head = match tokio::time::timeout(HEAD_TIMEOUT, http::read_request(&mut conn, &mut buf)).await
    {
        Ok(Ok(h)) => h,
        Ok(Err(HeadError::Eof | HeadError::Io(_))) => return,
        Err(_) => {
            proxy::refuse(&mut conn, 408, "wire2 hub: request head timeout").await;
            return;
        }
        Ok(Err(e)) => {
            crate::warn!("hub: bad request from a filter: {e}");
            proxy::refuse(&mut conn, 400, "wire2 hub: malformed request").await;
            return;
        }
    };
    let layer = if head.method.eq_ignore_ascii_case("CONNECT") {
        Layer::Tunnel
    } else {
        Layer::Http
    };
    let flow = match issued_flow(&ctx, &head) {
        Ok(f) => f,
        Err(why) => {
            crate::warn!(
                "hub: refusing {} {} with 403: {why}",
                head.method,
                head.target
            );
            proxy::refuse(&mut conn, 403, &format!("wire2 hub: {why}")).await;
            return;
        }
    };
    let Some((stage, tx)) = flow.route.take(layer) else {
        let why = format!(
            "flow {} is not waiting for a request from a filter at this layer",
            flow.id
        );
        crate::warn!(
            "hub: refusing {} {} with 403: {why}",
            head.method,
            head.target
        );
        proxy::refuse(&mut conn, 403, &format!("wire2 hub: {why}")).await;
        return;
    };
    tx.send_replace(StageState::Downstream);
    crate::debug!(
        "flow={} hub <- {}: {} {}",
        flow.id,
        flow.route.chain(layer)[stage].1,
        head.method,
        head.target
    );
    match layer {
        Layer::Tunnel => tunnel(conn, head, buf, &ctx, &flow, stage, tx).await,
        Layer::Http => http_hop(conn, head, buf, &ctx, &flow, stage, tx).await,
    }
}

async fn tunnel(
    mut conn: Conn,
    head: RequestHead,
    leftover: Vec<u8>,
    ctx: &Ctx,
    flow: &FlowInfo,
    stage: usize,
    tx: Arc<watch::Sender<StageState>>,
) {
    let same = split_authority(&head.target)
        .and_then(|(h, p)| Ok((normalise_host(h)?, p)))
        .is_ok_and(|(h, p)| h == flow.host && p == Some(flow.port));
    if !same {
        tx.send_replace(StageState::Answered);
        crate::warn!(
            "flow={} hub: refusing CONNECT {}: the flow is for {}:{}",
            flow.id,
            head.target,
            flow.host,
            flow.port
        );
        proxy::refuse(
            &mut conn,
            400,
            "wire2 hub: CONNECT target differs from the flow's",
        )
        .await;
        return;
    }
    let r = tokio::select! {
        r = proxy::connect_from(ctx, flow, stage + 1) => r,
        _ = flow.cancel.cancelled() => return,
    };
    tx.send_replace(StageState::Answered);
    let (mut up, pre) = match r {
        Ok(x) => x,
        Err(e) => {
            proxy::refuse(&mut conn, e.status, &e.body()).await;
            return;
        }
    };
    const OK: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";
    if conn.write_all(OK).await.is_err()
        || conn.write_all(&pre).await.is_err()
        || up.write_all(&leftover).await.is_err()
    {
        return;
    }
    drop((pre, leftover, head));
    // Scratch counters: the access log counts the browser-facing hop only.
    let (a, b) = (AtomicU64::new(0), AtomicU64::new(0));
    let r = tokio::select! {
        r = crate::splice::splice_bidi(&conn, &up, &a, &b) => r,
        _ = flow.cancel.cancelled() => Ok(()),
    };
    if let Err(e) = r {
        crate::debug!("flow={} hub tunnel ended: {e}", flow.id);
    }
}

async fn http_hop(
    mut conn: Conn,
    head: RequestHead,
    leftover: Vec<u8>,
    ctx: &Ctx,
    flow: &FlowInfo,
    stage: usize,
    tx: Arc<watch::Sender<StageState>>,
) {
    let req = match proxy::hop_request(&head, flow) {
        Ok(r) => r,
        Err((code, why)) => {
            tx.send_replace(StageState::Answered);
            crate::warn!(
                "flow={} hub: refusing {} {} with {code}: {why}",
                flow.id,
                head.method,
                head.target
            );
            proxy::refuse(&mut conn, code, &format!("wire2 hub: {why}")).await;
            return;
        }
    };
    let (mut br, bw) = tokio::io::split(conn);
    let mut bw = Signal {
        inner: bw,
        tx: Some(tx),
    };
    let (u, d) = (AtomicU64::new(0), AtomicU64::new(0));
    let status = AtomicU16::new(0);
    let next = proxy::http_from(
        &mut br,
        &mut bw,
        leftover,
        &req,
        ctx,
        flow,
        Ctr { up: &u, down: &d },
        &status,
        false,
        stage + 1,
    )
    .await;
    crate::debug!(
        "flow={} hub -> stage {stage}: {}",
        flow.id,
        status.load(Ordering::Relaxed)
    );
    if !matches!(next, Next::Abort) {
        let mut conn = br.unsplit(bw.inner);
        proxy::finish(&mut conn).await;
    }
}

/// A writer that tells the waiting stage, on the first byte back, that its answer has arrived (the
/// stage's `--stage-timeout` clock restarts then).
struct Signal<W> {
    inner: W,
    tx: Option<Arc<watch::Sender<StageState>>>,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for Signal<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let me = self.get_mut();
        if let Some(tx) = me.tx.take() {
            tx.send_replace(StageState::Answered);
        }
        Pin::new(&mut me.inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
