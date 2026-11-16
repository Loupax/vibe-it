//! QF7: the engine/App plumbing that lets page JS send a request and get the
//! answer, without `fetch`/`XMLHttpRequest` (QF8/QF9) existing yet.
//!
//! Flow (JS cannot touch `App`, same pattern as QB5/Q33d1):
//!
//! 1. `__net_send(desc)` (native, see `install_net`) validates the request
//!    with `RequestPolicy::check_request`, classifies it (CORS kind, beacon),
//!    serialises it with the QF5 codec and records a `NetJob` in
//!    `JsRequests.net`. Anything refused (or a suppressed beacon) is recorded
//!    in `JsRequests.net_done` as an immediate result instead, so the page
//!    still sees it asynchronously.
//! 2. `App::js_sync` hands the jobs to a `NetScheduler` (parallelism and
//!    in-flight byte caps come from the one `RequestPolicy`), which starts
//!    each on its own thread through the injectable `NetExecutor`. The answer
//!    comes back as `AppEvent::JsNet(nav_generation, id, result)`.
//! 3. `JsEngine::deliver_net` applies CORS (`NetState::finish`) and calls the
//!    JS-side `__net_complete` under the callback budget, then drains jobs.
//!
//! `--js-report` has no App: `drive_report_net` runs the same queue on the
//! calling thread. Sync mode (`desc.sync`) runs the executor on the engine
//! thread with `sync_xhr_timeout_ms`.
use super::*;
use crate::net_policy::*;
use rquickjs::{Ctx, Function, Object, TypedArray};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// One validated request ready for a thread: the serialised wire request
/// plus its limits. `reserve` is what it counts against the in-flight cap.
#[derive(Debug, Clone)]
pub(crate) struct NetJob {
    pub id: u64,
    pub request: Vec<u8>,
    pub limits: WireLimits,
    pub reserve: usize,
}

/// Performs one request. The real one talks to wire; tests inject a fake.
pub(crate) trait NetExecutor: Send + Sync {
    fn execute(&self, job: &NetJob) -> Result<WireResponse, NetError>;
    /// False when there is no wire at all (no `--socket`): cookies are then
    /// silently unavailable instead of a logged failure.
    fn has_wire(&self) -> bool {
        true
    }
}

pub(crate) struct WireExecutor(pub PathBuf);

impl NetExecutor for WireExecutor {
    fn execute(&self, job: &NetJob) -> Result<WireResponse, NetError> {
        fetch_full_via_wire(&self.0, &job.request, &job.limits)
    }
}

/// No `--socket`: every request fails with a network error.
pub(crate) struct NoSocketExecutor;

impl NetExecutor for NoSocketExecutor {
    fn has_wire(&self) -> bool {
        false
    }

    fn execute(&self, _job: &NetJob) -> Result<WireResponse, NetError> {
        Err(NetError::Connect("no --socket was given".into()))
    }
}

thread_local! {
    static NET_EXECUTOR: RefCell<Arc<dyn NetExecutor>> = RefCell::new(Arc::new(NoSocketExecutor));
}

/// Sets the executor new engines (and the App's scheduler) on this thread use.
pub(crate) fn set_net_executor(e: Arc<dyn NetExecutor>) {
    NET_EXECUTOR.with(|x| *x.borrow_mut() = e);
}

pub(crate) fn net_executor() -> Arc<dyn NetExecutor> {
    NET_EXECUTOR.with(|x| x.borrow().clone())
}

// ------------------------------------------------------------ scheduler

/// The App-side queue: at most `max_concurrent_requests` in flight, and the
/// request bytes in flight stay under `max_in_flight_bytes` (a request is
/// held back while the rest would push it over, but a lone request always
/// goes). Pure data, no threads: the caller starts what `pump` hands out.
pub(crate) struct NetScheduler {
    max_concurrent: usize,
    max_in_flight_bytes: usize,
    queue: VecDeque<NetJob>,
    in_flight: HashMap<u64, usize>,
    in_flight_bytes: usize,
}

impl NetScheduler {
    pub(crate) fn new(policy: &RequestPolicy) -> Self {
        NetScheduler {
            max_concurrent: policy.max_concurrent_requests.max(1),
            max_in_flight_bytes: policy.max_in_flight_bytes,
            queue: VecDeque::new(),
            in_flight: HashMap::new(),
            in_flight_bytes: 0,
        }
    }

    pub(crate) fn enqueue(&mut self, job: NetJob) {
        self.queue.push_back(job);
    }

    /// Removes a still-queued job; false when it already started (or never
    /// existed), in which case its late result is ignored by the engine.
    pub(crate) fn abort(&mut self, id: u64) -> bool {
        match self.queue.iter().position(|j| j.id == id) {
            Some(i) => {
                self.queue.remove(i);
                true
            }
            None => false,
        }
    }

    /// Starts as many queued jobs as the caps allow, in order.
    pub(crate) fn pump(&mut self, start: &mut dyn FnMut(NetJob)) {
        while let Some(front) = self.queue.front() {
            if self.in_flight.len() >= self.max_concurrent {
                break;
            }
            if !self.in_flight.is_empty() && self.in_flight_bytes + front.reserve > self.max_in_flight_bytes {
                break;
            }
            let job = self.queue.pop_front().unwrap();
            self.in_flight_bytes += job.reserve;
            self.in_flight.insert(job.id, job.reserve);
            start(job);
        }
    }

    /// A started job finished. Unknown ids (immediate results) are a no-op.
    pub(crate) fn done(&mut self, id: u64) {
        if let Some(r) = self.in_flight.remove(&id) {
            self.in_flight_bytes = self.in_flight_bytes.saturating_sub(r);
        }
    }

    pub(crate) fn reset(&mut self) {
        self.queue.clear();
        self.in_flight.clear();
        self.in_flight_bytes = 0;
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.queue.len(), self.in_flight.len())
    }
}

// ------------------------------------------------------------ engine state

/// What `__net_send` is given (parsed from the JS descriptor).
#[derive(Debug, Default)]
struct SendDesc {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    mode: String,
    credentials: String,
    keepalive: bool,
    api: String,
    sync: bool,
}

struct NetMeta {
    method: String,
    url: Url,
    page: Url,
    kind: FetchKind,
    credentials: bool,
    /// A suppressed beacon: whatever arrives is turned into an opaque response.
    force_opaque: bool,
    /// Already logged and recorded (refused or suppressed): do not log again.
    immediate: bool,
    started: Instant,
    entry: usize,
}

/// One line of the `--js-report` network section.
#[derive(Debug, Clone)]
pub(crate) struct NetEntry {
    pub method: String,
    pub url: String,
    /// `pending`, `<status> <text>`, `blocked: ..`, `error: ..`,
    /// `suppressed (beacon)`, `aborted`.
    pub outcome: String,
    pub bytes: usize,
    pub elapsed_ms: Option<u128>,
    pub warnings: Vec<String>,
    /// QF14b: the request carried `X-Wire-Jar` (the cookie jar was used).
    pub jar: bool,
}

/// What the JS side receives in `__net_complete`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Completion {
    pub id: u64,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub final_url: String,
    pub redirected: bool,
    /// `None`, `network` or `timeout`.
    pub error: Option<&'static str>,
    /// `basic`, `cors`, `opaque`.
    pub rtype: &'static str,
}

pub(crate) struct NetState {
    pub(crate) policy: RequestPolicy,
    pub(crate) executor: Arc<dyn NetExecutor>,
    log: LogFn,
    next_id: u64,
    sent: usize,
    meta: HashMap<u64, NetMeta>,
    sync_results: HashMap<u64, Completion>,
    pub(crate) entries: Vec<NetEntry>,
    /// QF14b: what wire told us about cookie support.
    pub(crate) support: &'static CookieSupport,
    /// QF14b: a `document.cookie` control call failed: `''` for the rest of
    /// the page (the engine thread must not keep blocking).
    cookie_broken: bool,
}

/// Response headers a `cors` response always exposes to the page.
const CORS_SAFELISTED_RESPONSE: &[&str] =
    &["cache-control", "content-language", "content-length", "content-type", "expires", "last-modified", "pragma"];

fn cors_filter_headers(headers: Vec<(String, String)>, credentials: bool) -> Vec<(String, String)> {
    let mut allowed: Vec<String> = CORS_SAFELISTED_RESPONSE.iter().map(|s| s.to_string()).collect();
    for (n, v) in &headers {
        if n == "access-control-expose-headers" {
            for t in v.split(',').map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty()) {
                if t == "*" && !credentials {
                    return headers.into_iter().collect();
                }
                allowed.push(t);
            }
        }
    }
    headers.into_iter().filter(|(n, _)| allowed.iter().any(|a| a == n)).collect()
}

impl NetState {
    pub(crate) fn new(log: LogFn) -> Self {
        NetState {
            policy: RequestPolicy::default(),
            executor: net_executor(),
            log,
            next_id: 1,
            sent: 0,
            meta: HashMap::new(),
            sync_results: HashMap::new(),
            entries: Vec::new(),
            support: cookie_support(),
            cookie_broken: false,
        }
    }

    /// QF14b: one synchronous `document.cookie` control call (`get` returns
    /// the cookie string, `set` sends `value` and returns ''). Blocks the
    /// engine thread for at most `cookie_control_timeout_ms`. Every failure
    /// (no wire, old wire, cookies off, timeout) is logged once and answers
    /// '' from then on.
    fn cookie_op(&mut self, op: &str, value: &str) -> String {
        if self.cookie_broken || !self.support.usable() || !self.executor.has_wire() {
            return String::new();
        }
        let Some(page) = page_url() else { return String::new() };
        let Some(mut headers) = cookie_control_headers(op, &page) else { return String::new() };
        let set = op == "set";
        if set {
            headers.push(("Content-Type".into(), "text/plain".into()));
        }
        let Ok(request) = build_wire_request(if set { "POST" } else { "GET" }, &page, &headers, value.as_bytes()) else {
            return String::new();
        };
        let mut limits = self.policy.wire_limits();
        let t = Duration::from_millis(self.policy.cookie_control_timeout_ms);
        limits.read_timeout = t;
        limits.connect_timeout = limits.connect_timeout.min(t);
        let job = NetJob { id: 0, request, limits, reserve: 0 };
        let fail = |this: &mut NetState, why: String| {
            this.cookie_broken = true;
            this.note(&format!("document.cookie: {why}; it reads '' for the rest of this page"));
        };
        match self.executor.execute(&job) {
            Err(e) => {
                fail(self, e.to_string());
                String::new()
            }
            Ok(resp) => {
                if let Some(line) = self.support.observe(resp.header("x-wire-version"), resp.header("x-wire-cookies")) {
                    (self.log)("net-warning", &line);
                }
                if !self.support.usable() {
                    self.cookie_broken = true;
                    return String::new();
                }
                match resp.status {
                    200 if !set => String::from_utf8_lossy(&resp.body).into_owned(),
                    204 if set => String::new(),
                    s => {
                        if self.support.usable() {
                            fail(self, format!("wire answered {s} to the cookie control request"));
                        } else {
                            self.cookie_broken = true;
                        }
                        String::new()
                    }
                }
            }
        }
    }

    fn note(&self, text: &str) {
        (self.log)("net", text);
    }

    /// Records a refused request: logs it, and makes the page see a network
    /// error asynchronously (or immediately for sync mode).
    fn refuse(&mut self, id: u64, d: &SendDesc, entry: usize, reason: String, reqs: &mut js_dom::JsRequests) {
        self.note(&format!("blocked {} {}: {reason}", d.method, d.url));
        self.entries[entry].outcome = format!("blocked: {reason}");
        self.entries[entry].elapsed_ms = Some(0);
        let url = Url::parse(&d.url).unwrap_or_else(|_| Url::parse("about:blank").unwrap());
        self.meta.insert(
            id,
            NetMeta {
                method: d.method.clone(),
                page: url.clone(),
                url,
                kind: FetchKind::Basic,
                credentials: false,
                force_opaque: false,
                immediate: true,
                started: Instant::now(),
                entry,
            },
        );
        self.queue_immediate(id, Err(NetError::BadRequest(reason)), d.sync, reqs);
    }

    fn queue_immediate(&mut self, id: u64, result: Result<WireResponse, NetError>, sync: bool, reqs: &mut js_dom::JsRequests) {
        if sync {
            if let Some(c) = self.finish(id, result) {
                self.sync_results.insert(id, c);
            }
        } else {
            reqs.net_done.push((id, result));
        }
    }

    fn send(&mut self, d: SendDesc, reqs: &mut js_dom::JsRequests) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let mut d = d;
        d.method = normalize_method(&d.method);
        self.entries.push(NetEntry {
            method: d.method.clone(),
            url: d.url.clone(),
            outcome: "pending".into(),
            bytes: 0,
            elapsed_ms: None,
            warnings: vec![],
            jar: false,
        });
        let entry = self.entries.len() - 1;
        let Some(page) = page_url() else {
            self.refuse(id, &d, entry, "the page has no URL".into(), reqs);
            return id;
        };
        let url = match Url::parse(&d.url) {
            Ok(u) => u,
            Err(e) => {
                self.refuse(id, &d, entry, format!("invalid URL: {e}"), reqs);
                return id;
            }
        };
        if self.sent >= self.policy.max_requests_per_page {
            let msg = format!("too many requests from this page (limit {})", self.policy.max_requests_per_page);
            self.refuse(id, &d, entry, msg, reqs);
            return id;
        }
        let outcome = match self.policy.check_request(&page, &url, &d.method, !d.body.is_empty()) {
            Ok(o) => o,
            Err(e) => {
                self.refuse(id, &d, entry, e.to_string(), reqs);
                return id;
            }
        };
        if let Some(class) = outcome.warning {
            let w = format_net_warning(&page, &url, class);
            (self.log)("net-warning", &w);
            self.entries[entry].warnings.push(w);
        }
        let headers: Vec<(String, String)> =
            d.headers.iter().filter(|(n, _)| !is_forbidden_request_header(n) && !is_wire_reserved_header(n)).cloned().collect();
        let mode = match d.mode.as_str() {
            "no-cors" => RequestMode::NoCors,
            "same-origin" => RequestMode::SameOrigin,
            _ => RequestMode::Cors,
        };
        let kind = match classify_fetch(mode, &page, &url, &d.method, &headers) {
            Ok(k) => k,
            Err(e) => {
                self.refuse(id, &d, entry, e.to_string(), reqs);
                return id;
            }
        };
        let api = match d.api.as_str() {
            "beacon" => NetApi::SendBeacon,
            "xhr" => NetApi::Xhr,
            _ => NetApi::Fetch,
        };
        let beacon = classify_beacon(api, d.keepalive, Some(kind));
        let credentials = d.credentials == "include";
        let mut meta = NetMeta {
            method: d.method.clone(),
            url: url.clone(),
            page: page.clone(),
            kind,
            credentials,
            force_opaque: false,
            immediate: false,
            started: Instant::now(),
            entry,
        };
        if self.policy.suppress_beacon(beacon) {
            let ct = headers.iter().find(|(n, _)| n.eq_ignore_ascii_case("content-type")).map(|(_, v)| v.as_str());
            let line = format_beacon_log(beacon.unwrap(), &d.method, &url, ct, &d.body, &page, false);
            self.note(&line);
            self.entries[entry].outcome = "suppressed (beacon, not sent)".into();
            self.entries[entry].elapsed_ms = Some(0);
            meta.force_opaque = true;
            meta.immediate = true;
            self.meta.insert(id, meta);
            self.queue_immediate(id, Ok(WireResponse::default()), d.sync, reqs);
            return id;
        }
        if kind == (FetchKind::Cors { preflight: true }) {
            self.refuse(id, &d, entry, "CORS preflight is not implemented (QF13 preflight)".into(), reqs);
            return id;
        }
        if d.body.len() > self.policy.max_in_flight_bytes {
            let msg = format!("request body is too large ({} bytes, limit {})", d.body.len(), self.policy.max_in_flight_bytes);
            self.refuse(id, &d, entry, msg, reqs);
            return id;
        }
        let mut wire_headers = headers;
        if matches!(kind, FetchKind::Cors { .. }) {
            wire_headers.push(("Origin".into(), origin_header(&page)));
        }
        let sync_ms = d.sync.then_some(self.policy.sync_xhr_timeout_ms);
        wire_headers.extend(
            self.policy
                .wire_cap_headers(sync_ms)
                .into_iter()
                // wire refuses private targets outright with `public-only`:
                // only ask for it when the policy blocks them (Warn/Allow
                // want to reach them, and to hear the target class).
                .filter(|(n, _)| n != WIRE_HEADER_POLICY || self.policy.private_targets == PrivatePolicy::Block),
        );
        // QF14b: the Fetch spec's credentials mode decides whether the jar is used.
        let cookie_headers = wire_cookie_headers_with(self.support, &CookieContext::fetch(&page, parse_credentials(&d.credentials), is_same_origin(&page, &url)));
        self.entries[entry].jar = cookie_headers.iter().any(|(n, _)| n == WIRE_HEADER_JAR);
        wire_headers.extend(cookie_headers);
        let request = match build_wire_request(&d.method, &url, &wire_headers, &d.body) {
            Ok(r) => r,
            Err(e) => {
                self.refuse(id, &d, entry, e.to_string(), reqs);
                return id;
            }
        };
        let mut limits = self.policy.wire_limits();
        if let Some(ms) = sync_ms {
            let t = Duration::from_millis(ms);
            limits.read_timeout = t;
            limits.connect_timeout = limits.connect_timeout.min(t);
        }
        self.sent += 1;
        let job = NetJob { id, request, limits, reserve: d.body.len() };
        self.meta.insert(id, meta);
        if d.sync {
            self.note(&format!("sync {} {url}: blocking the page for up to {} ms", d.method, self.policy.sync_xhr_timeout_ms));
            let result = self.executor.execute(&job);
            if let Some(c) = self.finish(id, result) {
                self.sync_results.insert(id, c);
            }
        } else {
            reqs.net.push(job);
        }
        id
    }

    /// The page gave up on `id`: forget it (a late result is ignored) and
    /// cancel it if it is still queued.
    fn abort(&mut self, id: u64, reqs: &mut js_dom::JsRequests) {
        let Some(m) = self.meta.remove(&id) else { return };
        let e = &mut self.entries[m.entry];
        if e.outcome == "pending" {
            e.outcome = "aborted".into();
            e.elapsed_ms = Some(m.started.elapsed().as_millis());
        }
        if let Some(i) = reqs.net.iter().position(|j| j.id == id) {
            reqs.net.remove(i);
        } else if let Some(i) = reqs.net_done.iter().position(|(i, _)| *i == id) {
            let _ = reqs.net_done.remove(i);
        } else {
            reqs.net_abort.push(id);
        }
    }

    /// Turns a transport result into what the page sees: applies CORS to a
    /// cross-origin response (opaque for `no-cors`), hides what the page may
    /// not read, logs failures. `None` when the page aborted the request.
    pub(crate) fn finish(&mut self, id: u64, result: Result<WireResponse, NetError>) -> Option<Completion> {
        let m = self.meta.remove(&id)?;
        let elapsed = m.started.elapsed().as_millis();
        let mut c = Completion { id, final_url: m.url.to_string(), rtype: "basic", ..Default::default() };
        let fail = |this: &mut NetState, kind: &'static str, msg: String, c: &mut Completion| {
            if !m.immediate {
                this.note(&format!("{} {}: {msg}", m.method, m.url));
                this.entries[m.entry].outcome = format!("error: {msg}");
                this.entries[m.entry].elapsed_ms = Some(elapsed);
            }
            c.error = Some(kind);
            c.rtype = "error";
        };
        match result {
            Err(e) => {
                let kind = if e == NetError::Timeout { "timeout" } else { "network" };
                fail(self, kind, e.to_string(), &mut c);
            }
            Ok(resp) if resp.error.is_some() => {
                let msg = resp.error.clone().unwrap_or_default();
                fail(self, "network", format!("wire: {msg}"), &mut c);
            }
            Ok(resp) => {
                let bytes = resp.body.len();
                if !m.immediate {
                    self.entries[m.entry].outcome = format!("{} {}", resp.status, resp.status_text).trim().to_string();
                    self.entries[m.entry].bytes = bytes;
                    self.entries[m.entry].elapsed_ms = Some(elapsed);
                }
                if m.force_opaque || m.kind == FetchKind::Opaque {
                    c.rtype = "opaque";
                    c.final_url = String::new();
                } else {
                    let mut rtype = "basic";
                    if let FetchKind::Cors { .. } = m.kind {
                        if let Err(e) = check_cors_response(&origin_header(&m.page), m.credentials, &resp.headers) {
                            self.note(&format!("{} {}: {e} (blocked by CORS policy)", m.method, m.url));
                            self.entries[m.entry].outcome = format!("blocked: {e}");
                            c.error = Some("network");
                            c.rtype = "error";
                            return Some(c);
                        }
                        rtype = "cors";
                    }
                    c.rtype = rtype;
                    c.status = resp.status;
                    c.status_text = resp.status_text;
                    c.headers = if rtype == "cors" { cors_filter_headers(resp.headers, m.credentials) } else { resp.headers };
                    c.body = resp.body;
                    if let Some(f) = resp.final_url {
                        c.redirected = f != m.url.as_str();
                        c.final_url = f;
                    }
                }
            }
        }
        Some(c)
    }
}

// ------------------------------------------------------------ JS bindings

fn completion_to_js<'js>(ctx: &Ctx<'js>, c: &Completion) -> rquickjs::Result<Object<'js>> {
    let o = Object::new(ctx.clone())?;
    o.set("id", c.id as u32)?;
    o.set("status", c.status as u32)?;
    o.set("statusText", c.status_text.as_str())?;
    let pairs: Vec<Vec<String>> = c.headers.iter().map(|(n, v)| vec![n.clone(), v.clone()]).collect();
    o.set("headers", pairs)?;
    o.set("body", TypedArray::<u8>::new_copy(ctx.clone(), &c.body)?)?;
    o.set("finalUrl", c.final_url.as_str())?;
    o.set("redirected", c.redirected)?;
    o.set("error", c.error.unwrap_or(""))?;
    o.set("type", c.rtype)?;
    Ok(o)
}

fn parse_desc(o: &Object<'_>) -> SendDesc {
    let s = |k: &str| o.get::<_, Option<String>>(k).ok().flatten().unwrap_or_default();
    let headers = o
        .get::<_, Option<Vec<Vec<String>>>>("headers")
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| (p.len() == 2).then(|| (p[0].clone(), p[1].clone())))
        .collect();
    // The body crosses as a binary string (one char per byte).
    let body = s("body").chars().map(|c| c as u32 as u8).collect();
    SendDesc {
        method: s("method"),
        url: s("url"),
        headers,
        body,
        mode: s("mode"),
        credentials: s("credentials"),
        keepalive: o.get::<_, Option<bool>>("keepalive").ok().flatten().unwrap_or(false),
        api: s("api"),
        sync: o.get::<_, Option<bool>>("sync").ok().flatten().unwrap_or(false),
    }
}

/// Installs `__net_send(desc) -> id`, `__net_abort(id)` and
/// `__net_sync_result(id) -> completion` (what `__net_complete` would get,
/// for a `sync` request that already ran).
pub(crate) fn install_net<'js>(ctx: &Ctx<'js>, state: Rc<RefCell<NetState>>, reqs: Rc<RefCell<js_dom::JsRequests>>) {
    let g = ctx.globals();
    let (st, rq) = (state.clone(), reqs.clone());
    if let Ok(f) = Function::new(ctx.clone(), move |desc: Object<'js>| -> u32 {
        let d = parse_desc(&desc);
        st.borrow_mut().send(d, &mut rq.borrow_mut()) as u32
    }) {
        let _ = g.set("__net_send", f);
    }
    let (st, rq) = (state.clone(), reqs);
    if let Ok(f) = Function::new(ctx.clone(), move |id: u32| {
        st.borrow_mut().abort(id as u64, &mut rq.borrow_mut());
    }) {
        let _ = g.set("__net_abort", f);
    }
    let st = state.clone();
    if let Ok(f) = Function::new(ctx.clone(), move || -> String { st.borrow_mut().cookie_op("get", "") }) {
        let _ = g.set("__cookie_get", f);
    }
    let st = state.clone();
    if let Ok(f) = Function::new(ctx.clone(), move |v: String| {
        st.borrow_mut().cookie_op("set", &v);
    }) {
        let _ = g.set("__cookie_set", f);
    }
    let st = state;
    if let Ok(f) = Function::new(ctx.clone(), move |ctx: Ctx<'js>, id: u32| -> rquickjs::Result<Option<Object<'js>>> {
        let c = st.borrow_mut().sync_results.remove(&(id as u64));
        c.map(|c| completion_to_js(&ctx, &c)).transpose()
    }) {
        let _ = g.set("__net_sync_result", f);
    }
}

impl JsEngine {
    /// Delivers a transport result for request `id` (see the module docs).
    /// Unknown (aborted) ids are ignored.
    pub(crate) fn deliver_net(&self, id: u64, result: Result<WireResponse, NetError>) {
        let Some(c) = self.net.borrow_mut().finish(id, result) else { return };
        self.arm_budget(CALLBACK_BUDGET);
        self.ctx.with(|ctx| {
            let f: Option<Function> = ctx.globals().get("__net_complete").ok();
            let Some(f) = f else { return };
            let res = completion_to_js(&ctx, &c).and_then(|o| f.call::<_, ()>((o,)));
            if let Err(e) = res {
                let msg = describe_error(&ctx, e);
                if self.tripped.load(Ordering::Relaxed) {
                    self.log_interrupted("net", "callback", CALLBACK_BUDGET, 0);
                } else {
                    (self.log)("error", &format!("net: {msg}"));
                }
            }
        });
        drain_jobs(&self.ctx, &self.rt, &*self.log);
    }

    #[cfg(test)]
    pub(crate) fn set_cookie_support(&self, s: &'static CookieSupport) {
        self.net.borrow_mut().support = s;
    }

    #[cfg(test)]
    pub(crate) fn set_net_policy(&self, p: RequestPolicy) {
        self.net.borrow_mut().policy = p;
    }

    pub(crate) fn net_entries(&self) -> Vec<NetEntry> {
        self.net.borrow().entries.clone()
    }

    #[cfg(test)]
    pub(crate) fn net_policy(&self) -> RequestPolicy {
        self.net.borrow().policy.clone()
    }
}

// ------------------------------------------------------------ --js-report

/// Runs everything the page has queued, to completion, on this thread, in
/// order: the report has no App, so this is its `js_sync` + event loop. The
/// same scheduler caps apply (one in flight at a time here). Results that
/// make the page send more requests are run too; the per-page cap bounds it.
pub(crate) fn drive_report_net(engine: &JsEngine, sched: &mut NetScheduler, exec: &dyn NetExecutor) {
    loop {
        let reqs = engine.take_requests();
        let nothing = reqs.net.is_empty() && reqs.net_done.is_empty() && reqs.net_abort.is_empty();
        for id in &reqs.net_abort {
            sched.abort(*id);
        }
        for job in reqs.net {
            sched.enqueue(job);
        }
        for (id, r) in reqs.net_done {
            engine.deliver_net(id, r);
        }
        let mut started = Vec::new();
        sched.pump(&mut |j| started.push(j));
        if nothing && started.is_empty() {
            break;
        }
        for job in started {
            let r = exec.execute(&job);
            sched.done(job.id);
            engine.deliver_net(job.id, r);
        }
    }
}

/// The `--- network ---` section of `--js-report`.
pub(crate) fn format_net_report(entries: &[NetEntry]) -> String {
    let total: usize = entries.iter().map(|e| e.bytes).sum();
    let mut out = format!("--- network ({} requests, {} bytes) ---\n", entries.len(), total);
    for e in entries {
        let ms = e.elapsed_ms.map(|m| format!(" {m} ms")).unwrap_or_default();
        let jar = if e.jar { "scratchy" } else { "off" };
        out.push_str(&format!("{} {} -> {} {} bytes{ms} jar: {jar}\n", e.method, e.url, e.outcome, e.bytes));
        for w in &e.warnings {
            out.push_str(&format!("    {w}\n"));
        }
    }
    out
}
