//! QF19w2: page `WebSocket` on top of wire's upgrade pipe (QF19w1).
//!
//! scratchy never opens an internet socket. For `new WebSocket(url)` it
//! validates the URL against the one `RequestPolicy`, builds the HTTP upgrade
//! request (random `Sec-WebSocket-Key`, `Origin`, the cookie partition headers
//! like a credentialed fetch, `X-Wire-Upgrade: websocket`) and sends it to
//! wire's unix socket. wire connects (DNS, TCP, TLS, policy) and answers `101`;
//! from then on the connection is a plain pipe and the RFC 6455 framing
//! (`ws_frames.rs`) runs here, over that unix socket.
//!
//! Threads: one per socket reads (handshake first, then frames), one writes.
//! Events cross to the engine through `WsShared` (a queue plus a wake-up for
//! the App), and `JsEngine::pump_ws` hands them to page JS one task at a time,
//! each under the callback CPU budget, in order. The engine owns the socket
//! handles: dropping it (navigation, page replacement) sends close code 1001
//! and shuts every socket down. The JS class lives in `ws_prelude.js`.
//!
//! An old wire (no `ws` in `X-Wire-Features`) answers the upgrade like a
//! normal request: the socket then fails (error + close 1006), ONE log line
//! per page, and later sockets fail at once without sending anything.
use super::*;
use crate::net_policy::*;
use crate::ws_frames::*;
use crate::js_engine::{self, native, Bytes, BytesRef, Ctx, JsResult};
use std::collections::VecDeque;
use std::net::Shutdown;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Sockets one page may have open at once.
pub(crate) const MAX_SOCKETS: usize = 16;
/// How long a close we started waits for the server's close frame.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// Queued-but-undelivered message bytes after which the reader thread waits.
const MAX_QUEUED_BYTES: usize = 64 * 1024 * 1024;
/// Events handed to JS per `pump_ws` call (the App re-posts for the rest).
const PUMP_BATCH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WsEvent {
    /// A line for the log (the `bool`: this wire is too old for WebSocket).
    Log(String, bool),
    Open(String),
    Text(String),
    Binary(Vec<u8>),
    Error,
    Close { code: u16, reason: String, clean: bool },
    /// `n` payload bytes were written to wire (`bufferedAmount` goes down).
    Flushed(usize),
}

impl WsEvent {
    fn weight(&self) -> usize {
        match self {
            WsEvent::Text(s) => s.len(),
            WsEvent::Binary(b) => b.len(),
            _ => 0,
        }
    }
}

type Wake = Box<dyn Fn() + Send + Sync>;
static WAKE: std::sync::Mutex<Option<std::sync::Arc<Wake>>> = std::sync::Mutex::new(None);

/// The App's "WebSocket events are waiting" poster (None: nobody to wake,
/// `--js-report` and tests poll `pump_ws` themselves).
pub(crate) fn set_ws_wake(w: Option<Wake>) {
    *WAKE.lock().unwrap() = w.map(std::sync::Arc::new);
}

fn wake() {
    let w = WAKE.lock().unwrap().clone();
    if let Some(w) = w {
        w();
    }
}

/// The queue the socket threads fill and the engine drains.
pub(crate) struct WsShared {
    queue: std::sync::Mutex<VecDeque<(u64, WsEvent)>>,
    /// A wake-up is outstanding (or the queue is being drained).
    pending: AtomicBool,
    queued_bytes: AtomicUsize,
    /// The engine is gone: nobody reads, never wait for it.
    closed: AtomicBool,
}

impl WsShared {
    fn new() -> Arc<Self> {
        Arc::new(WsShared { queue: Default::default(), pending: AtomicBool::new(false), queued_bytes: AtomicUsize::new(0), closed: AtomicBool::new(false) })
    }

    fn push(&self, id: u64, ev: WsEvent) {
        // Back-pressure on a flood: the reader stops reading from wire (which
        // blocks the page's server) while the page is far behind.
        while self.queued_bytes.load(Ordering::Relaxed) > MAX_QUEUED_BYTES && !self.closed.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(2));
        }
        self.queued_bytes.fetch_add(ev.weight(), Ordering::Relaxed);
        let first = {
            let mut q = self.queue.lock().unwrap();
            q.push_back((id, ev));
            !self.pending.swap(true, Ordering::AcqRel)
        };
        if first {
            wake();
        }
    }

    fn take(&self) -> Option<(u64, WsEvent)> {
        let mut q = self.queue.lock().unwrap();
        match q.pop_front() {
            Some(e) => {
                self.queued_bytes.fetch_sub(e.1.weight(), Ordering::Relaxed);
                Some(e)
            }
            None => {
                self.pending.store(false, Ordering::Release);
                None
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.queue.lock().unwrap().is_empty()
    }
}

enum Cmd {
    Msg(u8, Vec<u8>),
    Close(Option<u16>, String),
    Pong(Vec<u8>),
    /// The page went away: close 1001 unless a close was sent, then shut down.
    Goaway,
    Shutdown,
}

struct Ctl {
    aborted: AtomicBool,
    close_sent: AtomicBool,
    /// The 101 was accepted and the writer thread runs.
    established: AtomicBool,
    close_deadline: std::sync::Mutex<Option<Instant>>,
    stream: std::sync::Mutex<Option<UnixStream>>,
}

struct WsSocket {
    tx: mpsc::Sender<Cmd>,
    ctl: Arc<Ctl>,
}

impl Drop for WsSocket {
    fn drop(&mut self) {
        self.ctl.aborted.store(true, Ordering::Relaxed);
        if self.ctl.established.load(Ordering::Relaxed) {
            let _ = self.tx.send(Cmd::Goaway);
        } else if let Some(s) = self.ctl.stream.lock().unwrap().as_ref() {
            // A thread still in the handshake must not outlive the page.
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

pub(crate) struct WsState {
    net: Rc<RefCell<js_net::NetState>>,
    pub(crate) shared: Arc<WsShared>,
    sockets: HashMap<u64, WsSocket>,
    next_id: u64,
    /// wire answered an upgrade without the `ws` feature.
    unsupported: bool,
    log: LogFn,
}

impl Drop for WsState {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
    }
}

impl WsState {
    pub(crate) fn new(net: Rc<RefCell<js_net::NetState>>, log: LogFn) -> Self {
        WsState { net, shared: WsShared::new(), sockets: HashMap::new(), next_id: 1, unsupported: false, log }
    }

    /// Queues the error + close(1006) a failed socket shows the page.
    fn fail(&mut self, id: u64, line: Option<String>) {
        if let Some(l) = line {
            (self.log)("net", &l);
        }
        self.shared.push(id, WsEvent::Error);
        self.shared.push(id, WsEvent::Close { code: 1006, reason: String::new(), clean: false });
    }

    /// `__ws_open(url, protocols)`: the id of a new socket. Every refusal still
    /// returns an id; the page sees error + close 1006 asynchronously.
    fn open(&mut self, url_s: &str, protocols: Vec<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let Some(page) = page_url() else {
            self.fail(id, Some(format!("blocked WebSocket {url_s}: the page has no URL")));
            return id;
        };
        let url = match Url::parse(url_s) {
            Ok(u) if matches!(u.scheme(), "ws" | "wss") => u,
            _ => {
                self.fail(id, Some(format!("blocked WebSocket {url_s}: not a ws: or wss: URL")));
                return id;
            }
        };
        if self.unsupported {
            self.fail(id, None);
            return id;
        }
        if self.sockets.len() >= MAX_SOCKETS {
            self.fail(id, Some(format!("blocked WebSocket {url}: too many open WebSockets on this page (limit {MAX_SOCKETS})")));
            return id;
        }
        let (policy, support, socket, caps) = {
            let n = self.net.borrow();
            let (p, s, sock) = n.ws_context();
            (p, s, sock, n.ws_cap_headers())
        };
        // The policy sees the http(s) twin: same host, port, mixed-content and
        // private-target rules as a fetch (an https page may not open ws:).
        let mut twin = url.clone();
        let _ = twin.set_scheme(if url.scheme() == "ws" { "http" } else { "https" });
        match policy.check_request(&page, &twin, "GET", false) {
            Ok(o) => {
                if let Some(class) = o.warning {
                    (self.log)("net-warning", &format_net_warning(&page, &twin, class));
                }
            }
            Err(e) => {
                self.fail(id, Some(format!("blocked WebSocket {url}: {e}")));
                return id;
            }
        }
        let Some(socket) = socket else {
            self.fail(id, Some(format!("WebSocket {url}: no wire (no --socket was given)")));
            return id;
        };
        let key = new_key();
        let mut headers = vec![
            ("Upgrade".to_string(), "websocket".to_string()),
            ("Sec-WebSocket-Key".to_string(), key.clone()),
            ("Sec-WebSocket-Version".to_string(), "13".to_string()),
            ("Origin".to_string(), origin_header(&page)),
            ("X-Wire-Upgrade".to_string(), "websocket".to_string()),
        ];
        if !protocols.is_empty() {
            headers.push(("Sec-WebSocket-Protocol".to_string(), protocols.join(", ")));
        }
        headers.extend(caps);
        // A WebSocket always sends credentials (the jar), like `credentials: include`.
        headers.extend(wire_cookie_headers_with(support, &CookieContext::fetch(&page, CredentialsMode::Include, is_same_origin(&page, &twin))));
        let mut request = match build_wire_request("GET", &url, &headers, &[]) {
            Ok(r) => r,
            Err(e) => {
                self.fail(id, Some(format!("blocked WebSocket {url}: {e}")));
                return id;
            }
        };
        // build_wire_request drops `Connection`; the upgrade request needs it.
        request.truncate(request.len() - 2);
        request.extend_from_slice(b"Connection: Upgrade\r\n\r\n");
        let (tx, rx) = mpsc::channel();
        let ctl = Arc::new(Ctl {
            aborted: AtomicBool::new(false),
            close_sent: AtomicBool::new(false),
            established: AtomicBool::new(false),
            close_deadline: Default::default(),
            stream: Default::default(),
        });
        let job = Job {
            id,
            socket,
            url: url.to_string(),
            request,
            key,
            protocols,
            limits: policy.wire_limits(),
            shared: self.shared.clone(),
            ctl: ctl.clone(),
            tx: tx.clone(),
        };
        std::thread::spawn(move || run_socket(job, rx));
        self.sockets.insert(id, WsSocket { tx, ctl });
        id
    }

    /// `close()` while CONNECTING: fail the connection.
    fn abort(&mut self, id: u64) {
        if let Some(s) = self.sockets.get(&id) {
            s.ctl.aborted.store(true, Ordering::Relaxed);
            if let Some(st) = s.ctl.stream.lock().unwrap().as_ref() {
                let _ = st.shutdown(Shutdown::Both);
            }
            self.fail(id, None);
        }
    }

    fn send(&mut self, id: u64, opcode: u8, data: Vec<u8>) {
        if let Some(s) = self.sockets.get(&id) {
            let _ = s.tx.send(Cmd::Msg(opcode, data));
        }
    }

    fn close(&mut self, id: u64, code: Option<u16>, reason: String) {
        if let Some(s) = self.sockets.get(&id)
            && !s.ctl.close_sent.swap(true, Ordering::Relaxed)
        {
            *s.ctl.close_deadline.lock().unwrap() = Some(Instant::now() + CLOSE_WAIT);
            let _ = s.tx.send(Cmd::Close(code, reason));
        }
    }
}

// ------------------------------------------------------------ socket threads

struct Job {
    id: u64,
    socket: PathBuf,
    url: String,
    request: Vec<u8>,
    key: String,
    protocols: Vec<String>,
    limits: WireLimits,
    shared: Arc<WsShared>,
    ctl: Arc<Ctl>,
    tx: mpsc::Sender<Cmd>,
}

fn timed_out(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted)
}

struct Head {
    status: u16,
    headers: Vec<(String, String)>,
}

impl Head {
    fn get(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

fn parse_head(text: &str) -> Option<Head> {
    let mut lines = text.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Some(Head { status, headers })
}

/// Reads up to the blank line; returns the head text and the bytes after it.
fn read_head(stream: &mut UnixStream, deadline: Instant) -> Result<(String, Vec<u8>), String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf[p + 4..].to_vec();
            return Ok((String::from_utf8_lossy(&buf[..p]).into_owned(), rest));
        }
        if buf.len() > 64 * 1024 {
            return Err("the answer head is too large".into());
        }
        let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or("handshake timed out")?;
        stream.set_read_timeout(Some(left)).map_err(|e| e.to_string())?;
        match stream.read(&mut chunk) {
            Ok(0) => return Err("wire closed the connection during the handshake".into()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if timed_out(&e) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Validates the 101 answer; returns the selected subprotocol.
fn check_handshake(head: &Head, key: &str, offered: &[String]) -> Result<String, String> {
    if !head.get("upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket")) {
        return Err("the answer has no `Upgrade: websocket`".into());
    }
    if !head.get("connection").is_some_and(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case("upgrade"))) {
        return Err("the answer has no `Connection: Upgrade`".into());
    }
    if head.get("sec-websocket-accept") != Some(accept_key(key).as_str()) {
        return Err("the answer's Sec-WebSocket-Accept is wrong".into());
    }
    if head.get("sec-websocket-extensions").is_some() {
        return Err("the server chose an extension that was not offered".into());
    }
    match head.get("sec-websocket-protocol") {
        None => Ok(String::new()),
        Some(p) if offered.iter().any(|o| o == p) => Ok(p.to_string()),
        Some(p) => Err(format!("the server chose subprotocol {p:?}, which was not offered")),
    }
}

fn run_socket(job: Job, rx: mpsc::Receiver<Cmd>) {
    let Job { id, socket, url, request, key, protocols, limits, shared, ctl, tx } = job;
    let fail = |msg: String, old_wire: bool| {
        if ctl.aborted.load(Ordering::Relaxed) {
            return;
        }
        shared.push(id, WsEvent::Log(format!("WebSocket {url}: {msg}"), old_wire));
        shared.push(id, WsEvent::Error);
        shared.push(id, WsEvent::Close { code: 1006, reason: String::new(), clean: false });
    };
    let mut stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(e) => return fail(format!("cannot reach wire: {e}"), false),
    };
    *ctl.stream.lock().unwrap() = stream.try_clone().ok();
    if ctl.aborted.load(Ordering::Relaxed) {
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }
    let deadline = Instant::now() + limits.read_timeout;
    let _ = stream.set_write_timeout(Some(limits.read_timeout));
    if let Err(e) = stream.write_all(&request) {
        return fail(format!("cannot send the handshake to wire: {e}"), false);
    }
    let (text, rest) = match read_head(&mut stream, deadline) {
        Ok(x) => x,
        Err(e) => return fail(e, false),
    };
    let Some(head) = parse_head(&text) else { return fail("wire's answer is not HTTP".into(), false) };
    if head.status != 101 {
        let has_ws = head.get("x-wire-features").is_some_and(|f| f.split(',').any(|t| t.trim() == "ws"));
        if !has_ws && head.get("x-wire-error").is_none() {
            return fail("wire is too old for WebSocket (restart it from a current build)".into(), true);
        }
        let why = head.get("x-wire-error").map(|e| format!(" ({e})")).unwrap_or_default();
        return fail(format!("the handshake was refused: HTTP {}{why}", head.status), false);
    }
    let protocol = match check_handshake(&head, &key, &protocols) {
        Ok(p) => p,
        Err(e) => {
            let _ = stream.shutdown(Shutdown::Both);
            return fail(format!("handshake failed: {e}"), false);
        }
    };
    if ctl.aborted.load(Ordering::Relaxed) {
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }
    ctl.established.store(true, Ordering::Relaxed);
    let Ok(wstream) = stream.try_clone() else { return fail("cannot duplicate the socket".into(), false) };
    {
        let (ctl, shared) = (ctl.clone(), shared.clone());
        std::thread::spawn(move || run_writer(wstream, rx, ctl, shared, id));
    }
    shared.push(id, WsEvent::Open(protocol));
    run_reader(stream, rest, id, &shared, &ctl, &tx, &url);
    let _ = tx.send(Cmd::Shutdown);
}

fn run_reader(mut stream: UnixStream, rest: Vec<u8>, id: u64, shared: &WsShared, ctl: &Ctl, tx: &mpsc::Sender<Cmd>, url: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let mut dec = FrameDecoder::default();
    dec.push(&rest);
    let mut asm = Assembler::default();
    let mut chunk = vec![0u8; 64 * 1024];
    let abnormal = |shared: &WsShared| shared.push(id, WsEvent::Close { code: 1006, reason: String::new(), clean: false });
    loop {
        loop {
            let frame = match dec.next_frame() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(v) => return protocol_fail(v, id, shared, ctl, tx, url),
            };
            match asm.accept(frame) {
                Ok(None) | Ok(Some(Incoming::Pong(_))) => {}
                Ok(Some(Incoming::Text(s))) => shared.push(id, WsEvent::Text(s)),
                Ok(Some(Incoming::Binary(b))) => shared.push(id, WsEvent::Binary(b)),
                Ok(Some(Incoming::Ping(p))) => {
                    let _ = tx.send(Cmd::Pong(p));
                }
                Ok(Some(Incoming::Close(code, reason))) => {
                    if !ctl.close_sent.swap(true, Ordering::Relaxed) {
                        let _ = tx.send(Cmd::Close(code, String::new()));
                    }
                    shared.push(id, WsEvent::Close { code: code.unwrap_or(1005), reason, clean: true });
                    return;
                }
                Err(v) => return protocol_fail(v, id, shared, ctl, tx, url),
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) => return abnormal(shared),
            Ok(n) => dec.push(&chunk[..n]),
            Err(e) if timed_out(&e) => {
                if ctl.aborted.load(Ordering::Relaxed) {
                    return;
                }
                if ctl.close_deadline.lock().unwrap().is_some_and(|d| Instant::now() >= d) {
                    return abnormal(shared);
                }
            }
            Err(_) => return abnormal(shared),
        }
    }
}

fn protocol_fail(v: Violation, id: u64, shared: &WsShared, ctl: &Ctl, tx: &mpsc::Sender<Cmd>, url: &str) {
    if !ctl.close_sent.swap(true, Ordering::Relaxed) {
        let _ = tx.send(Cmd::Close(Some(v.code), String::new()));
    }
    shared.push(id, WsEvent::Log(format!("WebSocket {url}: failed: {}", v.msg), false));
    shared.push(id, WsEvent::Error);
    shared.push(id, WsEvent::Close { code: 1006, reason: String::new(), clean: false });
}

fn run_writer(mut stream: UnixStream, rx: mpsc::Receiver<Cmd>, ctl: Arc<Ctl>, shared: Arc<WsShared>, id: u64) {
    let put = |stream: &mut UnixStream, op: u8, payload: &[u8]| stream.write_all(&encode_frame(true, op, payload, random_bytes::<4>())).is_ok();
    while let Ok(first) = rx.recv() {
        let mut flushed = 0usize;
        let mut cmd = Some(first);
        let mut done = false;
        while let Some(c) = cmd.take() {
            match c {
                Cmd::Msg(op, p) => {
                    if put(&mut stream, op, &p) {
                        flushed += p.len();
                    } else {
                        done = true;
                    }
                }
                Cmd::Pong(p) => done = !put(&mut stream, OP_PONG, &p),
                Cmd::Close(code, reason) => done = !put(&mut stream, OP_CLOSE, &close_payload(code, &reason)),
                Cmd::Goaway => {
                    if !ctl.close_sent.swap(true, Ordering::Relaxed) {
                        put(&mut stream, OP_CLOSE, &close_payload(Some(1001), ""));
                    }
                    done = true;
                }
                Cmd::Shutdown => done = true,
            }
            if done {
                break;
            }
            cmd = rx.try_recv().ok();
        }
        if flushed > 0 {
            shared.push(id, WsEvent::Flushed(flushed));
        }
        if done {
            break;
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

// ------------------------------------------------------------ JS natives

/// Installs `__ws_open`, `__ws_send_text`, `__ws_send_bin`, `__ws_close`,
/// `__ws_abort` (the JS class in `ws_prelude.js` captures them).
pub(crate) fn install_ws<'js>(ctx: &Ctx<'js>, state: Rc<RefCell<WsState>>) {
    let st = state.clone();
    native!(ctx, "__ws_open", move |url: String, protocols: Vec<String>| -> u32 { st.borrow_mut().open(&url, protocols) as u32 });
    let st = state.clone();
    native!(ctx, "__ws_send_text", move |id: u32, text: String| {
        st.borrow_mut().send(id as u64, OP_TEXT, text.into_bytes());
    });
    let st = state.clone();
    native!(ctx, "__ws_send_bin", move |id: u32, data: Bytes| {
        st.borrow_mut().send(id as u64, OP_BINARY, data.0);
    });
    let st = state.clone();
    native!(ctx, "__ws_close", move |id: u32, code: i32, reason: String| {
        st.borrow_mut().close(id as u64, (code >= 0).then_some(code as u16), reason);
    });
    let st = state;
    native!(ctx, "__ws_abort", move |id: u32| {
        st.borrow_mut().abort(id as u64);
    });
}

impl JsEngine {
    /// Hands queued WebSocket events to page JS, one task each, in order and
    /// under the callback budget. Returns true when more are waiting (the App
    /// posts itself again so other events interleave).
    pub(crate) fn pump_ws(&self) -> bool {
        let shared = self.ws.borrow().shared.clone();
        for _ in 0..PUMP_BATCH {
            let Some((id, ev)) = shared.take() else { return false };
            self.deliver_ws(id, ev);
        }
        !shared.is_empty()
    }

    fn deliver_ws(&self, id: u64, ev: WsEvent) {
        let kind = match &ev {
            WsEvent::Log(line, old) => {
                (self.log)("net", line);
                if *old {
                    self.ws.borrow_mut().unsupported = true;
                }
                return;
            }
            WsEvent::Open(_) => "open",
            WsEvent::Text(_) => "text",
            WsEvent::Binary(_) => "bin",
            WsEvent::Error => "error",
            WsEvent::Close { .. } => "close",
            WsEvent::Flushed(_) => "flushed",
        };
        if matches!(ev, WsEvent::Close { .. }) {
            // Dropping the handle sends Goaway: a no-op on a closed socket.
            let s = self.ws.borrow_mut().sockets.remove(&id);
            drop(s);
        }
        let budget = crate::perf::callback_budget();
        self.arm_budget(budget);
        let span = crate::perf::begin(Some(crate::perf::Kind::NetCallback));
        self.eng.with(|ctx| {
            let id = id as u32;
            let res: JsResult<()> = match ev {
                WsEvent::Binary(b) => js_engine::call_global::<_, ()>(&ctx, "__ws_ev_bin", (id, BytesRef(&b))),
                other => {
                    let (s, n, b) = match other {
                        WsEvent::Open(p) => (p, 0.0, false),
                        WsEvent::Text(t) => (t, 0.0, false),
                        WsEvent::Close { code, reason, clean } => (reason, code as f64, clean),
                        WsEvent::Flushed(n) => (String::new(), n as f64, false),
                        _ => (String::new(), 0.0, false),
                    };
                    js_engine::call_global::<_, ()>(&ctx, "__ws_ev", (id, kind, s, n, b))
                }
            }
            .unwrap_or_else(|| Err(js_engine::not_a_function()));
            if let Err(e) = res {
                let msg = js_engine::describe_error(&ctx, e);
                if self.tripped() {
                    self.log_interrupted("websocket", "callback", budget, 0);
                } else {
                    (self.log)("error", &format!("websocket: {msg}"));
                }
            }
        });
        crate::perf::end(span, self.tripped());
        self.drain_after_task();
    }

    /// True while a socket is still CONNECTING or its events are queued
    /// (`--js-report` waits for these).
    pub(crate) fn ws_busy(&self) -> bool {
        let w = self.ws.borrow();
        !w.shared.is_empty() || w.sockets.values().any(|s| !s.ctl.established.load(Ordering::Relaxed) && !s.ctl.aborted.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    pub(crate) fn ws_socket_count(&self) -> usize {
        self.ws.borrow().sockets.len()
    }
}

/// `--js-report` has no App: deliver what the sockets queue, waiting a short
/// while for sockets that are still connecting.
pub(crate) fn drive_report_ws(engine: &JsEngine) {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(3) {
        while engine.pump_ws() {}
        if !engine.ws_busy() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
