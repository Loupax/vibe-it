//! Filters (SPEC "Filter protocol", "Routing and decryption", "Failure handling", "Security
//! requirements: Socket trust, Peer credentials").
//!
//! A filter is an HTTP/1.1 server on a Unix socket given with `--filter unix:<path>`. One supervisor task
//! per filter owns its [`State`]: it dials the socket, sends the handshake (`GET /hello` with `Proxy-Id`,
//! `Proxy-Filter-Version: 1`, `Proxy-Hub`), validates the JSON answer and publishes the capabilities.
//! When the data path finds the filter gone it calls [`Filter::mark_down`]; the supervisor then redials
//! with exponential backoff (100 ms to 5 s) and repeats the handshake. An http-layer filter's hosts are
//! registered with [`Policy::add_http_claim`] while it is up (which creates the CA the first time) and
//! removed when it goes down.
//!
//! [`Filters::route`] computes a flow's chain once, when the flow opens: the filters that are up, whose
//! hosts match, split by layer, in `--filter` order. A filter that is down refuses the flow (fail
//! closed; before any handshake that covers every host) or is skipped (fail open). Each request the
//! proxy sends a filter carries `Proxy-Flow: <proxy id>/<flow id>`; the hub accepts a request back only
//! for a flow whose stage is waiting for one ([`Route::expect`] / [`Route::take`]).

use std::future::Future;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::decrypt::Policy;
use crate::host::glob_match;
use crate::http::{self, Framing, HeadError};

/// Protocol version this proxy speaks (`Proxy-Filter-Version`, handshake `version`).
pub const VERSION: u64 = 1;
/// First and largest delay between redials of a filter that is down.
const BACKOFF_MIN: Duration = Duration::from_millis(100);
const BACKOFF_MAX: Duration = Duration::from_secs(5);
/// Largest handshake answer body.
const MAX_HELLO: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// Sees `CONNECT` requests and the encrypted bytes.
    Tunnel = 0,
    /// Sees decrypted requests and responses; makes the proxy decrypt the hosts it claims.
    Http = 1,
}

impl Layer {
    fn name(self) -> &'static str {
        match self {
            Layer::Tunnel => "tunnel",
            Layer::Http => "http",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    Open,
    Closed,
}

/// What a filter declared in its handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    pub name: String,
    pub layer: Layer,
    /// Normalised host globs (`*` = all hosts).
    pub hosts: Vec<String>,
    pub fail: Fail,
}

impl Caps {
    pub fn matches(&self, host: &str) -> bool {
        self.hosts.iter().any(|g| glob_match(g, host))
    }
}

#[derive(Debug, Clone)]
pub enum State {
    /// Startup: the first handshake is still being tried (at most `--connect-timeout`).
    Starting,
    Up(Arc<Caps>),
    /// Down. `last` holds the capabilities of the last good handshake; `None` (never shook hands, or
    /// the last handshake was rejected) means fail closed for every host.
    Down {
        last: Option<Arc<Caps>>,
    },
}

pub struct Filter {
    pub path: PathBuf,
    state: watch::Sender<State>,
}

/// How the data path may use a filter right now.
pub enum Use {
    Send,
    Skip,
    /// Fail closed: refuse, naming the filter.
    Refuse(String),
}

impl Filter {
    pub fn state(&self) -> State {
        self.state.borrow().clone()
    }

    /// The name from its last handshake, else the socket path.
    pub fn label(&self) -> String {
        match &*self.state.borrow() {
            State::Up(c) | State::Down { last: Some(c) } => c.name.clone(),
            _ => format!("unix:{}", self.path.display()),
        }
    }

    pub fn usable(&self) -> Use {
        match &*self.state.borrow() {
            State::Up(_) => Use::Send,
            State::Down { last: Some(c) } if c.fail == Fail::Open => Use::Skip,
            State::Down { last: Some(c) } => Use::Refuse(c.name.clone()),
            _ => Use::Refuse(format!("unix:{}", self.path.display())),
        }
    }

    /// The data path found the filter gone: mark it down (the supervisor redials it).
    pub fn mark_down(&self, why: &str) {
        let changed = self.state.send_if_modified(|s| match s {
            State::Up(c) => {
                *s = State::Down {
                    last: Some(c.clone()),
                };
                true
            }
            _ => false,
        });
        if changed {
            crate::warn!(
                "filter {} is DOWN: {why}; redialling unix:{}",
                self.label(),
                self.path.display()
            );
        }
    }

    /// Connect to the filter's socket after the trust checks: its directory must be ours and not
    /// group/world-writable, and the listening peer must run as our uid (or root, for a socket that
    /// systemd created).
    pub async fn connect(&self) -> io::Result<UnixStream> {
        check_socket_dir(&self.path).map_err(io::Error::other)?;
        let s = UnixStream::connect(&self.path).await?;
        crate::conn::tune_unix(&s);
        check_peer(&s).map_err(io::Error::other)?;
        Ok(s)
    }
}

/// SPEC "Socket trust": the socket's parent directory is owned by the proxy's uid and is not group- or
/// world-writable.
pub fn check_socket_dir(sock: &Path) -> Result<(), String> {
    let dir = match sock.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let m = std::fs::metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // SAFETY: plain getter.
    let me = unsafe { libc::geteuid() };
    if m.mode() & 0o022 != 0 {
        return Err(format!(
            "refusing socket {}: its directory {} is group- or world-writable (mode {:o})",
            sock.display(),
            dir.display(),
            m.mode() & 0o7777
        ));
    }
    if m.uid() != me {
        return Err(format!(
            "refusing socket {}: its directory {} is owned by uid {}, not by this proxy's uid {me}",
            sock.display(),
            dir.display(),
            m.uid()
        ));
    }
    Ok(())
}

/// SPEC "Peer credentials": the kernel-reported uid of the other end must be ours or root.
pub fn check_peer(s: &UnixStream) -> Result<(), String> {
    let c = s.peer_cred().map_err(|e| format!("SO_PEERCRED: {e}"))?;
    // SAFETY: plain getter.
    let me = unsafe { libc::geteuid() };
    if c.uid() != me && c.uid() != 0 {
        return Err(format!(
            "peer runs as uid {} (pid {:?}), not as this proxy's uid {me}",
            c.uid(),
            c.pid()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------
// The filter set and its supervisors

pub struct Filters {
    pub list: Vec<Arc<Filter>>,
    pub proxy_id: String,
    pub hub: PathBuf,
    connect_timeout: Duration,
    stage_timeout: Duration,
    check_interval: Duration,
}

/// Why a handshake attempt failed.
enum HsError {
    /// Socket missing, refused, or closed/silent before an answer: retried quietly.
    Unreachable(String),
    /// The filter answered, wrongly (SPEC: non-200, bad JSON, unknown version or layer, duplicate name).
    Rejected(String),
}

impl Filters {
    pub fn new(
        paths: &[PathBuf],
        proxy_id: String,
        hub: PathBuf,
        connect_timeout: Duration,
        stage_timeout: Duration,
        check_interval: Duration,
    ) -> Filters {
        Filters {
            list: paths
                .iter()
                .map(|p| {
                    Arc::new(Filter {
                        path: p.clone(),
                        state: watch::Sender::new(State::Starting),
                    })
                })
                .collect(),
            proxy_id,
            hub,
            connect_timeout,
            stage_timeout,
            check_interval,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn stage_timeout(&self) -> Duration {
        self.stage_timeout
    }

    /// Start one supervisor task per filter.
    pub fn start(self: &Arc<Self>, policy: Arc<Policy>) {
        for f in &self.list {
            tokio::spawn(self.clone().supervise(f.clone(), policy.clone()));
        }
    }

    async fn supervise(self: Arc<Self>, f: Arc<Filter>, policy: Arc<Policy>) {
        let deadline = Instant::now() + self.connect_timeout;
        let mut backoff = BACKOFF_MIN;
        let mut last_err = String::new();
        loop {
            let starting = matches!(f.state(), State::Starting);
            let limit = if starting {
                deadline.saturating_duration_since(Instant::now())
            } else {
                self.stage_timeout
            };
            let r = match tokio::time::timeout(limit, self.handshake(&f)).await {
                Ok(r) => r,
                Err(_) => Err(HsError::Unreachable("no handshake answer in time".into())),
            };
            let r = r.and_then(|caps| self.accept(&f, caps, &policy));
            match r {
                Ok(caps) => {
                    crate::info!(
                        "filter {} up (unix:{}): layer={} hosts={} fail={}",
                        caps.name,
                        f.path.display(),
                        caps.layer.name(),
                        caps.hosts.join(","),
                        if caps.fail == Fail::Open {
                            "open"
                        } else {
                            "closed"
                        }
                    );
                    f.state.send_replace(State::Up(caps.clone()));
                    last_err.clear();
                    backoff = BACKOFF_MIN;
                    let caps = self.watch_up(&f, caps, &policy).await;
                    if caps.layer == Layer::Http {
                        policy.remove_http_claim(&caps.name);
                    }
                }
                Err(HsError::Rejected(why)) => {
                    if why != last_err {
                        crate::warn!(
                            "filter unix:{} handshake REJECTED: {why}; it counts as down and fail-closed \
                             for ALL hosts until a handshake succeeds",
                            f.path.display()
                        );
                    }
                    last_err = why;
                    f.state.send_replace(State::Down { last: None });
                }
                Err(HsError::Unreachable(why)) => {
                    if why != last_err {
                        crate::info!("filter unix:{} unreachable: {why}", f.path.display());
                    }
                    last_err = why;
                    if starting && Instant::now() >= deadline {
                        crate::error!(
                            "filter unix:{} did not complete its handshake within --connect-timeout {}ms: \
                             it is DOWN and counts as fail-closed, so EVERY host is refused (503) until \
                             its handshake succeeds; still redialling",
                            f.path.display(),
                            self.connect_timeout.as_millis()
                        );
                        f.state.send_replace(State::Down { last: None });
                    }
                }
            }
            let mut sleep = backoff;
            if matches!(f.state(), State::Starting) {
                sleep = sleep.min(deadline.saturating_duration_since(Instant::now()));
            }
            tokio::time::sleep(sleep).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    }

    /// While the filter is up: repeat the handshake every `--filter-check` so a restart is noticed
    /// without traffic (SPEC "Failure handling"). Returns the capabilities in force when the filter went
    /// down (the data path marked it down, the check found it gone, or it answered wrongly). New
    /// capabilities replace the old ones for new flows only; open flows keep their chain.
    async fn watch_up(&self, f: &Filter, mut caps: Arc<Caps>, policy: &Policy) -> Arc<Caps> {
        let mut rx = f.state.subscribe();
        loop {
            tokio::select! {
                _ = rx.wait_for(|s| !matches!(s, State::Up(_))) => return caps,
                _ = tokio::time::sleep(self.check_interval) => {}
            }
            let r = match tokio::time::timeout(self.stage_timeout, self.handshake(f)).await {
                Ok(r) => r,
                Err(_) => Err(HsError::Unreachable("no handshake answer in time".into())),
            };
            let new = match r {
                Ok(new) if new == *caps => continue,
                Ok(new) => new,
                Err(HsError::Unreachable(why)) => {
                    f.mark_down(&format!("it did not answer a handshake check ({why})"));
                    return caps;
                }
                Err(HsError::Rejected(why)) => {
                    self.reject_up(f, &why);
                    return caps;
                }
            };
            if caps.layer == Layer::Http {
                policy.remove_http_claim(&caps.name);
            }
            match self.accept(f, new, policy) {
                Ok(new) => {
                    crate::info!(
                        "filter {} CHANGED (unix:{}): layer={} hosts={} fail={}; new flows use it",
                        new.name,
                        f.path.display(),
                        new.layer.name(),
                        new.hosts.join(","),
                        if new.fail == Fail::Open {
                            "open"
                        } else {
                            "closed"
                        }
                    );
                    let replaced = f.state.send_if_modified(|s| {
                        if matches!(s, State::Up(_)) {
                            *s = State::Up(new.clone());
                            true
                        } else {
                            false
                        }
                    });
                    caps = new;
                    if !replaced {
                        return caps;
                    }
                }
                Err(HsError::Rejected(why)) | Err(HsError::Unreachable(why)) => {
                    self.reject_up(f, &why);
                    return caps;
                }
            }
        }
    }

    /// An up filter answered a handshake check wrongly: down, fail-closed for every host.
    fn reject_up(&self, f: &Filter, why: &str) {
        crate::warn!(
            "filter unix:{} handshake REJECTED: {why}; it counts as down and fail-closed for ALL hosts \
             until a handshake succeeds",
            f.path.display()
        );
        f.state.send_replace(State::Down { last: None });
    }

    /// Final checks on a good answer: unique name, and the CA for an http-layer claim.
    fn accept(&self, f: &Filter, caps: Caps, policy: &Policy) -> Result<Arc<Caps>, HsError> {
        for other in &self.list {
            if std::ptr::eq(&**other, f) {
                continue;
            }
            if let State::Up(c) = other.state()
                && c.name == caps.name
            {
                return Err(HsError::Rejected(format!(
                    "name {:?} is already used by the filter at unix:{}",
                    caps.name,
                    other.path.display()
                )));
            }
        }
        if caps.layer == Layer::Http {
            policy
                .add_http_claim(&caps.name, caps.hosts.clone())
                .map_err(|e| HsError::Rejected(format!("cannot set up decryption: {e}")))?;
        }
        Ok(Arc::new(caps))
    }

    async fn handshake(&self, f: &Filter) -> Result<Caps, HsError> {
        let unreachable = |e: io::Error| HsError::Unreachable(e.to_string());
        let mut s = f.connect().await.map_err(unreachable)?;
        let req = format!(
            "GET /hello HTTP/1.1\r\nHost: proxy\r\nProxy-Id: {}\r\nProxy-Filter-Version: {VERSION}\r\n\
             Proxy-Hub: unix:{}\r\nConnection: close\r\n\r\n",
            self.proxy_id,
            self.hub.display()
        );
        s.write_all(req.as_bytes()).await.map_err(unreachable)?;
        let mut buf = Vec::new();
        let head = match http::read_response(&mut s, &mut buf).await {
            Ok(h) => h,
            Err(HeadError::Eof | HeadError::Truncated | HeadError::Io(_)) => {
                return Err(HsError::Unreachable(
                    "closed before answering the handshake".into(),
                ));
            }
            Err(e) => return Err(HsError::Rejected(format!("malformed answer: {e}"))),
        };
        if head.code != 200 {
            return Err(HsError::Rejected(format!(
                "answered {} {} instead of 200",
                head.code, head.reason
            )));
        }
        let framing = http::response_framing("GET", head.code, &head.headers)
            .map_err(|e| HsError::Rejected(format!("bad framing: {e}")))?;
        let body = read_small_body(&mut s, buf, framing)
            .await
            .map_err(HsError::Rejected)?;
        parse_caps(&body).map_err(HsError::Rejected)
    }

    /// Wait (bounded by `--connect-timeout`) for every filter still in its first handshake, so the first
    /// flows after startup are routed by the filters' real capabilities.
    async fn settled(&self) {
        for f in &self.list {
            if matches!(f.state(), State::Starting) {
                let mut rx = f.state.subscribe();
                let _ = tokio::time::timeout(
                    self.connect_timeout + Duration::from_secs(1),
                    rx.wait_for(|s| !matches!(s, State::Starting)),
                )
                .await;
            }
        }
    }

    /// A flow's chain, computed once when it opens (SPEC "Routing"). `connect`: the flow is a browser
    /// `CONNECT` (http filters whose hosts are on `--no-decrypt` never see it, so they cannot refuse it).
    /// `Err` names a down fail-closed filter covering `host`.
    pub async fn route(&self, host: &str, connect: bool, policy: &Policy) -> Result<Route, String> {
        if self.list.is_empty() {
            return Ok(Route::default());
        }
        self.settled().await;
        let mut r = Route::default();
        for f in &self.list {
            match f.state() {
                State::Starting | State::Down { last: None } => return Err(f.label()),
                State::Down { last: Some(c) } => {
                    let applies = c.matches(host)
                        && !(connect && c.layer == Layer::Http && policy.never_decrypt(host));
                    if applies && c.fail == Fail::Closed {
                        return Err(c.name.clone());
                    }
                }
                State::Up(c) if c.matches(host) => match c.layer {
                    Layer::Tunnel => r.tunnel.push((f.clone(), c.name.clone())),
                    Layer::Http => r.http.push((f.clone(), c.name.clone())),
                },
                State::Up(_) => {}
            }
        }
        Ok(r)
    }
}

/// Read a small (handshake) body: at most [`MAX_HELLO`] bytes, chunked decoded.
async fn read_small_body(
    s: &mut UnixStream,
    mut buf: Vec<u8>,
    framing: Framing,
) -> Result<Vec<u8>, String> {
    let mut tmp = [0u8; 4096];
    let done = |b: &Vec<u8>| -> Result<bool, String> {
        Ok(match framing {
            Framing::None => true,
            Framing::Length(n) => b.len() as u64 >= n,
            Framing::Chunked => http::ChunkScanner::new().feed(b)?.is_some(),
            Framing::UntilClose => false,
        })
    };
    while !done(&buf)? {
        if buf.len() > MAX_HELLO {
            return Err(format!("handshake answer larger than {MAX_HELLO} bytes"));
        }
        let n = s.read(&mut tmp).await.map_err(|e| e.to_string())?;
        if n == 0 {
            if framing == Framing::UntilClose {
                break;
            }
            return Err("handshake answer ended early".into());
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    match framing {
        Framing::None => Ok(Vec::new()),
        Framing::Length(n) => {
            buf.truncate(n as usize);
            Ok(buf)
        }
        Framing::Chunked => dechunk(&buf),
        Framing::UntilClose => Ok(buf),
    }
}

/// Decode a complete chunked body (extensions and trailers ignored).
fn dechunk(mut b: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n").ok_or("bad chunk")?;
        let line = std::str::from_utf8(&b[..eol]).map_err(|_| "bad chunk size")?;
        let size = line.split(';').next().unwrap_or("").trim();
        let n = usize::from_str_radix(size, 16).map_err(|_| "bad chunk size")?;
        b = &b[eol + 2..];
        if n == 0 {
            return Ok(out);
        }
        if b.len() < n + 2 {
            return Err("chunk ended early".into());
        }
        out.extend_from_slice(&b[..n]);
        b = &b[n + 2..];
    }
}

/// Validate a handshake answer (SPEC "Handshake").
pub fn parse_caps(body: &[u8]) -> Result<Caps, String> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("bad JSON: {e}"))?;
    let o = v.as_object().ok_or("the answer is not a JSON object")?;
    let name = match o.get("name") {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        _ => return Err("missing or empty \"name\"".into()),
    };
    if name.len() > 64
        || name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == ',')
    {
        return Err(format!(
            "bad name {name:?} (at most 64 characters, no spaces, commas or control characters)"
        ));
    }
    match o.get("version").and_then(|v| v.as_u64()) {
        Some(VERSION) => {}
        Some(v) => {
            return Err(format!(
                "unsupported protocol version {v} (this proxy speaks 1)"
            ));
        }
        None => return Err("missing or bad \"version\"".into()),
    }
    let layer = match o.get("layer").and_then(|v| v.as_str()) {
        Some("tunnel") => Layer::Tunnel,
        Some("http") => Layer::Http,
        Some(l) => return Err(format!("unknown layer {l:?}")),
        None => return Err("missing \"layer\"".into()),
    };
    let hosts = match o.get("hosts") {
        None => vec!["*".to_string()],
        Some(serde_json::Value::Array(a)) => {
            let mut out = Vec::new();
            for h in a {
                let s = h.as_str().ok_or("\"hosts\" must be a list of strings")?;
                out.push(crate::host::normalise_glob(s).map_err(|e| e.to_string())?);
            }
            out
        }
        Some(_) => return Err("\"hosts\" must be a list of strings".into()),
    };
    let fail = match o.get("fail").map(|v| v.as_str()) {
        None => Fail::Closed,
        Some(Some("closed")) => Fail::Closed,
        Some(Some("open")) => Fail::Open,
        Some(f) => return Err(format!("bad \"fail\" {f:?} (open or closed)")),
    };
    Ok(Caps {
        name,
        layer,
        hosts,
        fail,
    })
}

// ---------------------------------------------------------------------------------------------------
// A flow's route and the hub's check of Proxy-Flow

/// Where a stage that was sent a request stands, as seen by the hub (drives `--stage-timeout`: the
/// clock does not run while the request is further down the chain).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageState {
    /// The filter has the request; its own response headers are due within the stage timeout.
    Waiting,
    /// The filter forwarded it to the hub; the rest of the chain is working on it.
    Downstream,
    /// The hub sent the filter the response headers; the filter must pass them on in time.
    Answered,
}

struct Pending {
    stage: usize,
    seq: u64,
    tx: Arc<watch::Sender<StageState>>,
    /// W2-2e: ends the hub's pipe for the one upstream connection this request opens (see `take`).
    kill: CancellationToken,
}

/// A flow's chain (computed once) and, per layer, the stage the hub currently expects a request from.
/// A flow has at most one request in flight per layer (decrypted requests are served one after the
/// other, and each opens its own upstream connection), so one slot per layer is enough.
#[derive(Default)]
pub struct Route {
    pub tunnel: Vec<(Arc<Filter>, String)>,
    pub http: Vec<(Arc<Filter>, String)>,
    pending: [Mutex<Option<Pending>>; 2],
    seq: AtomicU64,
}

impl Route {
    /// Filter names in the order a request meets them (http chain, then the upstream connector's
    /// tunnel chain).
    pub fn names(&self) -> Vec<String> {
        self.http
            .iter()
            .chain(self.tunnel.iter())
            .map(|(_, n)| n.clone())
            .collect()
    }

    /// The route of a flow that is not decrypted: http filters never see it.
    pub fn tunnel_only(mut self) -> Route {
        self.http.clear();
        self
    }

    pub fn chain(&self, layer: Layer) -> &[(Arc<Filter>, String)] {
        match layer {
            Layer::Tunnel => &self.tunnel,
            Layer::Http => &self.http,
        }
    }

    fn slot(&self, layer: Layer) -> std::sync::MutexGuard<'_, Option<Pending>> {
        self.pending[layer as usize]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// The request is about to go to `stage` of `layer`: the hub may now accept one request back from
    /// it. The guard withdraws that permission when the stage is over.
    /// The third value ends the hub's pipe for this one request (a tunnel stage's upstream connection):
    /// the connection's owner cancels it when it drops the connection (W2-2e, so a parked connection
    /// does not depend on the flow that dialled it).
    pub fn expect(
        &self,
        layer: Layer,
        stage: usize,
    ) -> (Expect<'_>, watch::Receiver<StageState>, CancellationToken) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = watch::channel(StageState::Waiting);
        let kill = CancellationToken::new();
        *self.slot(layer) = Some(Pending {
            stage,
            seq,
            tx: Arc::new(tx),
            kill: kill.clone(),
        });
        (
            Expect {
                route: self,
                layer,
                seq,
            },
            rx,
            kill,
        )
    }

    /// The hub got a request for this flow at `layer`: which stage sent it (once per [`Route::expect`]).
    pub fn take(
        &self,
        layer: Layer,
    ) -> Option<(usize, Arc<watch::Sender<StageState>>, CancellationToken)> {
        self.slot(layer).take().map(|p| (p.stage, p.tx, p.kill))
    }
}

pub struct Expect<'a> {
    route: &'a Route,
    layer: Layer,
    seq: u64,
}

impl Drop for Expect<'_> {
    fn drop(&mut self) {
        let mut s = self.route.slot(self.layer);
        if s.as_ref().is_some_and(|p| p.seq == self.seq) {
            *s = None;
        }
    }
}

/// Run `fut` (reading a stage's response head) under `--stage-timeout`. The clock stops while the
/// request is downstream of this stage and restarts when the hub hands the stage its answer. `None`:
/// the stage timed out.
pub async fn stage_wait<T>(
    fut: impl Future<Output = T>,
    mut rx: watch::Receiver<StageState>,
    timeout: Duration,
) -> Option<T> {
    let mut fut = pin!(fut);
    let mut deadline = Instant::now() + timeout;
    let mut live = true;
    loop {
        let downstream = live && *rx.borrow_and_update() == StageState::Downstream;
        tokio::select! {
            r = &mut fut => return Some(r),
            _ = tokio::time::sleep_until(deadline), if !downstream => return None,
            c = rx.changed(), if live => {
                if c.is_err() {
                    live = false;
                }
                deadline = Instant::now() + timeout;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_answers() {
        let c = parse_caps(
            br#"{"name":"pass","version":1,"layer":"tunnel","hosts":["*"],"fail":"open"}"#,
        )
        .unwrap();
        assert_eq!(
            c,
            Caps {
                name: "pass".into(),
                layer: Layer::Tunnel,
                hosts: vec!["*".into()],
                fail: Fail::Open
            }
        );
        // Defaults: all hosts, fail closed; hosts normalised; unknown fields ignored.
        let c = parse_caps(
            br#"{"name":"x","version":1,"layer":"http","hosts":["*.Example.TEST."],"future":true}"#,
        )
        .unwrap();
        assert_eq!(c.hosts, vec!["*.example.test"]);
        assert_eq!(c.fail, Fail::Closed);
        assert_eq!(
            parse_caps(br#"{"name":"x","version":1,"layer":"tunnel"}"#)
                .unwrap()
                .hosts,
            vec!["*"]
        );
        for bad in [
            &br#"not json"#[..],
            br#"[]"#,
            br#"{"version":1,"layer":"tunnel"}"#,
            br#"{"name":"","version":1,"layer":"tunnel"}"#,
            br#"{"name":"a b","version":1,"layer":"tunnel"}"#,
            br#"{"name":"x","version":2,"layer":"tunnel"}"#,
            br#"{"name":"x","version":"1","layer":"tunnel"}"#,
            br#"{"name":"x","version":1,"layer":"frames"}"#,
            br#"{"name":"x","version":1}"#,
            br#"{"name":"x","version":1,"layer":"http","hosts":"*"}"#,
            br#"{"name":"x","version":1,"layer":"http","hosts":["a..b"]}"#,
            br#"{"name":"x","version":1,"layer":"http","fail":"maybe"}"#,
        ] {
            assert!(parse_caps(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn dechunks() {
        assert_eq!(
            dechunk(b"3;x=y\r\nabc\r\n2\r\nde\r\n0\r\nT: v\r\n\r\n").unwrap(),
            b"abcde"
        );
        assert!(dechunk(b"5\r\nab").is_err());
    }

    #[test]
    fn socket_dir_trust() {
        let d = tempfile::tempdir().unwrap();
        assert!(check_socket_dir(&d.path().join("f.sock")).is_ok());
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        let e = check_socket_dir(&d.path().join("f.sock")).unwrap_err();
        assert!(e.contains("world-writable"), "{e}");
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o720)).unwrap();
        assert!(check_socket_dir(&d.path().join("f.sock")).is_err());
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let e = check_socket_dir(Path::new("/f.sock")).unwrap_err();
        assert!(e.contains("owned by uid 0"), "{e}");
    }

    #[test]
    fn expect_and_take() {
        let r = Route::default();
        assert!(r.take(Layer::Http).is_none());
        {
            let (_g, _rx, _) = r.expect(Layer::Http, 2);
            assert!(r.take(Layer::Tunnel).is_none());
            assert_eq!(r.take(Layer::Http).map(|(s, _, _)| s), Some(2));
            // Taken once only.
            assert!(r.take(Layer::Http).is_none());
        }
        {
            let (_g, _rx, _) = r.expect(Layer::Tunnel, 0);
        }
        // The guard withdrew it.
        assert!(r.take(Layer::Tunnel).is_none());
        // A stale guard does not clear a newer expectation.
        let (g1, _rx1, _) = r.expect(Layer::Tunnel, 0);
        let (_g2, _rx2, _) = r.expect(Layer::Tunnel, 1);
        drop(g1);
        assert_eq!(r.take(Layer::Tunnel).map(|(s, _, _)| s), Some(1));
    }

    #[tokio::test(start_paused = true)]
    async fn stage_clock_stops_downstream() {
        let (tx, rx) = watch::channel(StageState::Waiting);
        let t = Duration::from_secs(1);
        // Nothing comes: times out.
        assert!(
            stage_wait(std::future::pending::<()>(), rx.clone(), t)
                .await
                .is_none()
        );
        // Downstream for 10 s, then answered: the stage gets a fresh second and makes it.
        let tx = Arc::new(tx);
        let tx2 = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            tx2.send_replace(StageState::Downstream);
            tokio::time::sleep(Duration::from_secs(10)).await;
            tx2.send_replace(StageState::Answered);
        });
        let fut = async {
            tokio::time::sleep(Duration::from_millis(10_900)).await;
        };
        assert!(stage_wait(fut, rx.clone(), t).await.is_some());
    }
}
