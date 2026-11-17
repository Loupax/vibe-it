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
    /// QF13: this request was preceded by a CORS preflight: wire is told not
    /// to follow redirects (a 3xx fails closed, see `finish`).
    preflighted: bool,
    /// QF7a: `send()` already logged a policy warning for this target.
    warned: bool,
}

/// QF13: a request that waits for its preflight (or is about to be sent).
#[derive(Debug, Clone)]
struct Prepared {
    id: u64,
    method: String,
    url: Url,
    /// Page headers plus `Origin`, the cap headers and the cookie headers.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    sync_ms: Option<u64>,
}

/// QF13: a preflight in flight, keyed by the preflight job's id.
struct PfPending {
    actual: Prepared,
    /// Its own `--js-report` line.
    entry: usize,
    key: String,
    unsafe_headers: Vec<String>,
    origin: String,
    credentials: bool,
}

/// What to do after a preflight answered.
enum PfStep {
    /// Allowed: send this (the actual request).
    Go(NetJob),
    /// Denied or failed: the actual request's failure (`None`: the page had
    /// already aborted it).
    Fail(Option<Completion>),
}

/// QF13: the in-memory preflight cache. Key: origin, URL, method, sorted
/// non-safelisted header names and the credentials flag (exactly, not by
/// subset). Values are expiry times in milliseconds on the engine's clock
/// (the manual clock in tests). Nothing is persisted (QF-D7).
#[derive(Debug, Default)]
pub(crate) struct PreflightCache {
    map: HashMap<String, f64>,
}

impl PreflightCache {
    pub(crate) fn key(origin: &str, url: &Url, method: &str, unsafe_headers: &[String], credentials: bool) -> String {
        format!("{origin}\n{url}\n{method}\n{}\n{}", unsafe_headers.join(","), if credentials { "include" } else { "omit" })
    }

    pub(crate) fn hit(&mut self, key: &str, now_ms: f64) -> bool {
        match self.map.get(key) {
            Some(&exp) if exp > now_ms => true,
            Some(_) => {
                self.map.remove(key);
                false
            }
            None => false,
        }
    }

    /// `max_age_secs == 0` is not cached. A full cache first drops expired
    /// entries, then the one that expires soonest.
    pub(crate) fn insert(&mut self, key: String, now_ms: f64, max_age_secs: u64, max_entries: usize) {
        if max_age_secs == 0 || max_entries == 0 {
            return;
        }
        if !self.map.contains_key(&key) && self.map.len() >= max_entries {
            self.map.retain(|_, exp| *exp > now_ms);
            while self.map.len() >= max_entries {
                let Some(oldest) = self.map.iter().min_by(|a, b| a.1.total_cmp(b.1)).map(|(k, _)| k.clone()) else { break };
                self.map.remove(&oldest);
            }
        }
        self.map.insert(key, now_ms + max_age_secs as f64 * 1000.0);
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }
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
    /// QF7a: refuse an answer from a wire older than protocol 2 (no
    /// `X-Wire-Version`/`X-Wire-Final-Url`): off in test builds, because the
    /// fake executors send no version header.
    pub(crate) check_wire_version: bool,
    /// QF7a: an old wire answered: page networking is off for the rest of
    /// the page (one log line).
    wire_disabled: bool,
    /// QF13: preflights in flight (by the preflight job's id), the cache, the
    /// actual requests a finished preflight released (the engine moves them
    /// into `JsRequests.net`) and the clock the cache expires on.
    preflights: HashMap<u64, PfPending>,
    pub(crate) pf_cache: PreflightCache,
    released: Vec<NetJob>,
    clock: Option<Rc<std::cell::Cell<f64>>>,
    base: Instant,
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

/// QF7a: the one log line when wire's protocol is older than 2.
pub(crate) const WIRE_TOO_OLD_LINE: &str = "scratchy: page networking disabled: wire is too old (restart it)";
const WIRE_TOO_OLD: &str = "page networking disabled: wire is too old (restart it)";

/// `X-Wire-Version` of an answer; 0 when absent (a wire older than QF6).
fn wire_version(resp: &WireResponse) -> u32 {
    resp.header("x-wire-version").and_then(|v| v.trim().parse().ok()).unwrap_or(0)
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
            check_wire_version: !cfg!(test),
            wire_disabled: false,
            preflights: HashMap::new(),
            pf_cache: PreflightCache::default(),
            released: Vec::new(),
            clock: None,
            base: Instant::now(),
        }
    }

    /// Tests drive the preflight cache's expiry with the engine's manual clock.
    pub(crate) fn with_clock(mut self, clock: Option<Rc<std::cell::Cell<f64>>>) -> Self {
        self.clock = clock;
        self
    }

    fn now_ms(&self) -> f64 {
        match &self.clock {
            Some(c) => c.get(),
            None => self.base.elapsed().as_secs_f64() * 1000.0,
        }
    }

    /// The actual requests that finished preflights released.
    pub(crate) fn take_released(&mut self) -> Vec<NetJob> {
        std::mem::take(&mut self.released)
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
        if !self.wire_disabled {
            self.note(&format!("blocked {} {}: {reason}", d.method, d.url));
        }
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
                preflighted: false,
                warned: false,
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
        if self.wire_disabled {
            // The one log line was written when the old wire was found.
            self.refuse(id, &d, entry, WIRE_TOO_OLD.into(), reqs);
            return id;
        }
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
            preflighted: false,
            warned: outcome.warning.is_some(),
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
        if d.body.len() > self.policy.max_in_flight_bytes {
            let msg = format!("request body is too large ({} bytes, limit {})", d.body.len(), self.policy.max_in_flight_bytes);
            self.refuse(id, &d, entry, msg, reqs);
            return id;
        }
        let preflighted = kind == (FetchKind::Cors { preflight: true });
        let unsafe_headers = cors_unsafe_request_headers(&headers);
        let mut wire_headers = headers;
        if matches!(kind, FetchKind::Cors { .. }) {
            wire_headers.push(("Origin".into(), origin_header(&page)));
        }
        let sync_ms = d.sync.then_some(self.policy.sync_xhr_timeout_ms);
        wire_headers.extend(self.cap_headers(sync_ms));
        if preflighted {
            // wire must not follow a redirect of a preflighted request (it
            // could not be re-preflighted): a 3xx comes back and fails closed.
            wire_headers.retain(|(n, _)| n != WIRE_HEADER_MAX_REDIRECTS);
            wire_headers.push((WIRE_HEADER_MAX_REDIRECTS.to_string(), "0".to_string()));
        }
        // QF14b: the Fetch spec's credentials mode decides whether the jar is used.
        let cookie_headers = wire_cookie_headers_with(self.support, &CookieContext::fetch(&page, parse_credentials(&d.credentials), is_same_origin(&page, &url)));
        self.entries[entry].jar = cookie_headers.iter().any(|(n, _)| n == WIRE_HEADER_JAR);
        wire_headers.extend(cookie_headers);
        meta.preflighted = preflighted;
        let prepared = Prepared { id, method: d.method.clone(), url: url.clone(), headers: wire_headers, body: d.body.clone(), sync_ms };
        if preflighted {
            self.meta.insert(id, meta);
            self.start_preflight(prepared, entry, unsafe_headers, &page, credentials, &d, reqs);
            return id;
        }
        let job = match self.release(&prepared, None) {
            Ok(j) => j,
            Err(msg) => {
                self.refuse(id, &d, entry, msg, reqs);
                return id;
            }
        };
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

    /// The cap headers of the policy (QF6). wire refuses private targets
    /// outright with `public-only`: only ask for it when the policy blocks
    /// them (Warn/Allow want to reach them, and to hear the target class).
    fn cap_headers(&self, timeout_ms: Option<u64>) -> Vec<(String, String)> {
        self.policy
            .wire_cap_headers(timeout_ms)
            .into_iter()
            .filter(|(n, _)| n != WIRE_HEADER_POLICY || self.policy.private_targets == PrivatePolicy::Block)
            .collect()
    }

    /// Builds the job of a prepared request and counts it toward the per-page
    /// cap. `remaining_ms` (sync, after a preflight) shortens its timeout so
    /// preflight + request stay within `sync_xhr_timeout_ms`.
    fn release(&mut self, p: &Prepared, remaining_ms: Option<u64>) -> Result<NetJob, String> {
        if self.sent >= self.policy.max_requests_per_page {
            return Err(format!("too many requests from this page (limit {})", self.policy.max_requests_per_page));
        }
        let mut headers = p.headers.clone();
        if let Some(ms) = remaining_ms {
            headers.retain(|(n, _)| n != WIRE_HEADER_TIMEOUT_MS);
            headers.push((WIRE_HEADER_TIMEOUT_MS.to_string(), ms.to_string()));
        }
        let request = build_wire_request(&p.method, &p.url, &headers, &p.body).map_err(|e| e.to_string())?;
        let mut limits = self.policy.wire_limits();
        if let Some(ms) = remaining_ms.or(p.sync_ms) {
            let t = Duration::from_millis(ms);
            limits.read_timeout = t;
            limits.connect_timeout = limits.connect_timeout.min(t);
        }
        self.sent += 1;
        Ok(NetJob { id: p.id, request, limits, reserve: p.body.len() })
    }

    /// QF13: the CORS preflight. Cache hit: the actual request goes at once.
    /// Otherwise an `OPTIONS` job (no cookies, no body) is sent first and the
    /// actual request is released by `preflight_done`. Sync requests do both
    /// inside this call.
    #[allow(clippy::too_many_arguments)]
    fn start_preflight(
        &mut self,
        actual: Prepared,
        actual_entry: usize,
        unsafe_headers: Vec<String>,
        page: &Url,
        credentials: bool,
        d: &SendDesc,
        reqs: &mut js_dom::JsRequests,
    ) {
        let id = actual.id;
        let origin = origin_header(page);
        let key = PreflightCache::key(&origin, &actual.url, &actual.method, &unsafe_headers, credentials);
        let hdrs = if unsafe_headers.is_empty() { "none".to_string() } else { unsafe_headers.join(",") };
        let t0 = Instant::now();
        if self.pf_cache.hit(&key, self.now_ms()) {
            self.note(&format!("preflight cache hit: OPTIONS {} for {} (headers: {hdrs}), not sent", actual.url, actual.method));
            self.entries.push(NetEntry {
                method: "OPTIONS (preflight)".into(),
                url: actual.url.to_string(),
                outcome: "cache hit (not sent)".into(),
                bytes: 0,
                elapsed_ms: Some(0),
                warnings: vec![],
                jar: false,
            });
            match self.release(&actual, None) {
                Ok(job) => {
                    if d.sync {
                        self.note(&format!("sync {} {}: blocking the page for up to {} ms", d.method, actual.url, self.policy.sync_xhr_timeout_ms));
                        let r = self.executor.execute(&job);
                        if let Some(c) = self.finish(id, r) {
                            self.sync_results.insert(id, c);
                        }
                    } else {
                        reqs.net.push(job);
                    }
                }
                Err(msg) => self.refuse(id, d, actual_entry, msg, reqs),
            }
            return;
        }
        let pid = self.next_id;
        self.next_id += 1;
        let mut headers = vec![
            ("Origin".to_string(), origin.clone()),
            ("Access-Control-Request-Method".to_string(), actual.method.clone()),
            ("Accept".to_string(), "*/*".to_string()),
        ];
        if !unsafe_headers.is_empty() {
            headers.push(("Access-Control-Request-Headers".to_string(), unsafe_headers.join(",")));
        }
        headers.extend(self.cap_headers(actual.sync_ms));
        headers.retain(|(n, _)| n != WIRE_HEADER_MAX_REDIRECTS);
        headers.push((WIRE_HEADER_MAX_REDIRECTS.to_string(), "0".to_string()));
        // A preflight never carries credentials.
        headers.extend(wire_cookie_headers_with(self.support, &CookieContext::fetch(page, CredentialsMode::Omit, false)));
        let pre = Prepared { id: pid, method: "OPTIONS".into(), url: actual.url.clone(), headers, body: vec![], sync_ms: actual.sync_ms };
        // The preflight is a request of its own: it counts toward the cap.
        let job = match self.release(&pre, None) {
            Ok(j) => j,
            Err(msg) => {
                self.refuse(id, d, actual_entry, msg, reqs);
                return;
            }
        };
        self.note(&format!(
            "preflight OPTIONS {} for {} (Access-Control-Request-Headers: {hdrs}) origin={origin}",
            actual.url, actual.method
        ));
        self.entries.push(NetEntry {
            method: "OPTIONS (preflight)".into(),
            url: actual.url.to_string(),
            outcome: "pending".into(),
            bytes: 0,
            elapsed_ms: None,
            warnings: vec![],
            jar: false,
        });
        let entry = self.entries.len() - 1;
        let sync_ms = actual.sync_ms;
        self.preflights.insert(pid, PfPending { actual, entry, key, unsafe_headers, origin, credentials });
        if d.sync {
            let total = self.policy.sync_xhr_timeout_ms;
            self.note(&format!("sync {} {}: blocking the page for up to {total} ms (including its preflight)", d.method, d.url));
            let r = self.executor.execute(&job);
            let left = sync_ms.unwrap_or(total).saturating_sub(t0.elapsed().as_millis() as u64).max(1);
            match self.preflight_done(pid, r, Some(left)) {
                PfStep::Go(actual_job) => {
                    let r = self.executor.execute(&actual_job);
                    if let Some(c) = self.finish(id, r) {
                        self.sync_results.insert(id, c);
                    }
                }
                PfStep::Fail(Some(c)) => {
                    self.sync_results.insert(id, c);
                }
                PfStep::Fail(None) => {}
            }
        } else {
            reqs.net.push(job);
        }
    }

    /// QF13: a preflight answered (or failed). Validates it with
    /// `check_preflight_response`, caches it, and either releases the actual
    /// request or fails it with a console line naming what was missing.
    fn preflight_done(&mut self, pid: u64, result: Result<WireResponse, NetError>, remaining_ms: Option<u64>) -> PfStep {
        let Some(pf) = self.preflights.remove(&pid) else { return PfStep::Fail(None) };
        let id = pf.actual.id;
        let elapsed = self.meta.get(&id).map(|m| m.started.elapsed().as_millis());
        self.entries[pf.entry].elapsed_ms = elapsed;
        let outcome: Result<(), String> = match result {
            Err(e) => Err(e.to_string()),
            Ok(resp) if resp.error.is_some() => Err(format!("wire: {}", resp.error.clone().unwrap_or_default())),
            Ok(resp) => {
                self.entries[pf.entry].outcome = format!("{} {}", resp.status, resp.status_text).trim().to_string();
                self.entries[pf.entry].bytes = resp.body.len();
                match self.policy.check_preflight_response(&pf.origin, pf.credentials, &pf.actual.method, &pf.unsafe_headers, resp.status, &resp.headers) {
                    Ok(r) => {
                        let now = self.now_ms();
                        self.pf_cache.insert(pf.key.clone(), now, r.max_age_secs, self.policy.preflight_cache_max_entries);
                        Ok(())
                    }
                    Err(PolicyError::Cors(m)) => Err(m),
                    Err(e) => Err(e.to_string()),
                }
            }
        };
        // The page aborted the request while the preflight was in flight.
        if !self.meta.contains_key(&id) {
            return PfStep::Fail(None);
        }
        let fail = |this: &mut NetState, msg: String| {
            let m = this.meta.remove(&id)?;
            this.note(&format!("CORS preflight failed: {}: {msg}", m.url));
            this.entries[pf.entry].outcome = format!("blocked: {msg}");
            this.entries[m.entry].outcome = format!("blocked: CORS preflight failed: {msg}");
            this.entries[m.entry].elapsed_ms = Some(m.started.elapsed().as_millis());
            Some(Completion { id, final_url: m.url.to_string(), error: Some("network"), rtype: "error", ..Default::default() })
        };
        match outcome {
            Err(msg) => PfStep::Fail(fail(self, msg)),
            Ok(()) => match self.release(&pf.actual, remaining_ms) {
                Ok(job) => PfStep::Go(job),
                Err(msg) => PfStep::Fail(fail(self, msg)),
            },
        }
    }

    /// The page gave up on `id`: forget it (a late result is ignored) and
    /// cancel it if it is still queued. A preflight still in flight for it
    /// is forgotten too.
    fn abort(&mut self, id: u64, reqs: &mut js_dom::JsRequests) {
        let Some(m) = self.meta.remove(&id) else { return };
        let e = &mut self.entries[m.entry];
        if e.outcome == "pending" {
            e.outcome = "aborted".into();
            e.elapsed_ms = Some(m.started.elapsed().as_millis());
        }
        let pf = self.preflights.iter().find(|(_, p)| p.actual.id == id).map(|(k, _)| *k);
        if let Some(pid) = pf {
            if let Some(p) = self.preflights.remove(&pid) {
                self.entries[p.entry].outcome = "aborted".into();
            }
            Self::cancel_job(pid, reqs);
        }
        Self::cancel_job(id, reqs);
    }

    fn cancel_job(id: u64, reqs: &mut js_dom::JsRequests) {
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
        // QF7a: an answer without wire protocol >= 2 (`X-Wire-Version`; such a
        // wire also sends no `X-Wire-Final-Url`, so redirects and the policy
        // could not be checked) fails CLOSED instead of relaxing the policy.
        // Only answers that really came from wire: refused requests and
        // suppressed beacons are synthesized here (`immediate`) and carry no
        // wire headers.
        let from_wire = self.preflights.contains_key(&id) || self.meta.get(&id).is_some_and(|m| !m.immediate);
        let result = match result {
            // Requests already in flight when the old wire was found.
            _ if from_wire && self.wire_disabled => Ok(WireResponse { error: Some(WIRE_TOO_OLD.into()), ..Default::default() }),
            Ok(resp) if from_wire && self.check_wire_version && self.executor.has_wire() && wire_version(&resp) < 2 => {
                if !self.wire_disabled {
                    self.wire_disabled = true;
                    (self.log)("net-warning", WIRE_TOO_OLD_LINE);
                }
                Ok(WireResponse { error: Some(WIRE_TOO_OLD.into()), ..Default::default() })
            }
            r => r,
        };
        if self.preflights.contains_key(&id) {
            // QF13: the answer to a preflight, not to a page request.
            return match self.preflight_done(id, result, None) {
                PfStep::Go(job) => {
                    self.released.push(job);
                    None
                }
                PfStep::Fail(c) => c,
            };
        }
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
            Ok(resp) if resp.error.as_deref() == Some(WIRE_TOO_OLD) => {
                // Already logged once (QF7a): only record it.
                self.entries[m.entry].outcome = format!("blocked: {WIRE_TOO_OLD}");
                self.entries[m.entry].elapsed_ms = Some(elapsed);
                c.error = Some("network");
                c.rtype = "error";
            }
            Ok(resp) if resp.error.is_some() => {
                let msg = resp.error.clone().unwrap_or_default();
                // QF7a: `private_targets: Block` makes scratchy send
                // `X-Wire-Policy: public-only`; wire answers 403 `policy` when
                // a DNS name resolved to a non-public address.
                if self.policy.private_targets == PrivatePolicy::Block && resp.status == 403 && msg.contains("blocked") {
                    fail(self, "network", format!("blocked: {} resolves to a non-public address (private_targets is block): {msg}", m.url.host_str().unwrap_or("")), &mut c);
                } else {
                    fail(self, "network", format!("wire: {msg}"), &mut c);
                }
            }
            Ok(resp) => {
                let bytes = resp.body.len();
                // QF7a: wire resolved a DNS NAME to a non-public address (the
                // URL check cannot know): the same warning `send()` writes for
                // literal private hosts, once per request.
                let by_name = !m.warned && !m.immediate && self.policy.private_targets == PrivatePolicy::Warn && matches!(m.url.host(), Some(url::Host::Domain(_)));
                let class = resp.header("x-wire-target-class").filter(|c| by_name && !matches!(*c, "public" | "file" | ""));
                if let Some(class) = class {
                    let w = format_net_warning(&m.page, &m.url, class);
                    (self.log)("net-warning", &w);
                    self.entries[m.entry].warnings.push(w);
                }
                if m.preflighted && matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                    // wire was told not to follow it: the new target would
                    // need its own preflight (QF13a), so fail closed.
                    let loc = resp.header("location").unwrap_or("").to_string();
                    fail(self, "network", format!("redirect to {loc:?} of a preflighted request is not followed (blocked)"), &mut c);
                    return Some(c);
                }
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
                        // wire follows redirects itself, so only the FINAL answer
                        // is checked (not each hop's headers). After a
                        // cross-origin redirect a browser sends `Origin: null`
                        // from then on: check against that.
                        let crossed = resp.final_url.as_deref().and_then(|f| Url::parse(f).ok()).is_some_and(|f| f.origin() != m.url.origin());
                        let origin = if crossed { "null".to_string() } else { origin_header(&m.page) };
                        if let Err(e) = check_cors_response(&origin, m.credentials, &resp.headers) {
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
                    // wire's own control headers are not the page's business.
                    let headers: Vec<(String, String)> = resp.headers.into_iter().filter(|(n, _)| !n.starts_with("x-wire-")).collect();
                    c.headers = if rtype == "cors" { cors_filter_headers(headers, m.credentials) } else { headers };
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
    if let Ok(f) = Function::new(ctx.clone(), move || -> u32 { st.borrow().policy.beacon_queue_bytes as u32 }) {
        let _ = g.set("__net_beacon_limit", f);
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
        let (c, released) = {
            let mut n = self.net.borrow_mut();
            let c = n.finish(id, result);
            (c, n.take_released())
        };
        // QF13: a preflight that passed released the actual request.
        self.reqs.borrow_mut().net.extend(released);
        let Some(c) = c else { return };
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
        self.drain_after_task();
    }

    #[cfg(test)]
    pub(crate) fn set_cookie_support(&self, s: &'static CookieSupport) {
        self.net.borrow_mut().support = s;
    }

    #[cfg(test)]
    pub(crate) fn set_check_wire_version(&self, on: bool) {
        self.net.borrow_mut().check_wire_version = on;
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
