//! web-host-cef's [`SiteForgetter`] (forget.rs, PROTOCOL.md "forget-site").
//!
//! 1. Cookies: `CookieManager::visit_all_cookies` deletes every cookie whose domain is
//!    the site or under it and collects those domains. Still needed next to step 3,
//!    whose `cookies` type removes only host cookies of the origin: checked with
//!    test/forget.py, a `Domain=site-a.test` cookie survived clearDataForOrigin on
//!    every origin of the site. The cookie domains are also how hosts the page never
//!    loaded are found.
//! 2. Origins: the recorded ones under the site (forget.rs), plus http and https of
//!    the domain, its `www.` and every cookie host (protocol::forget_origins).
//! 3. Storage: Chromium's own `Storage.clearDataForOrigin` with `storageTypes: all`,
//!    called in process through `CefBrowserHost::ExecuteDevToolsMethod` (no
//!    remote-debugging port, no DevTools socket) and awaited through a
//!    `DevToolsMessageObserver`: cookies, localStorage, IndexedDB, Cache Storage,
//!    service workers, file systems, shared storage, storage buckets, ... of the
//!    origin. It is called on a hidden windowless browser of the host's own (about:
//!    blank, every request cancelled), so DevTools never attaches to the user's page.
//! 4. sessionStorage is not on disk and not cleared by step 3: frames of the page
//!    that show the site run `sessionStorage.clear()`.
//! 5. The HTTP cache: `RequestContext::ClearHttpCache` clears all of it; Chromium
//!    exposes no per-origin HTTP cache removal to CEF or DevTools (PROTOCOL.md).
//! 6. The cookie store is flushed to disk; then `forget::finished`.

use crate::forget::{self, ForgetJob, ForgetReport, SiteForgetter};
use crate::protocol;
use cef::*;
use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

const ORIGIN_TIMEOUT_MS: i64 = 15_000;
const STORAGE_TYPES: &str = "all";

pub struct CefForgetter;

#[derive(Default)]
struct State {
    job: Option<ForgetJob>,
    report: ForgetReport,
    todo: VecDeque<String>,
    /// The origin whose clearDataForOrigin is pending, and its DevTools message id.
    pending: Option<(String, i32)>,
    /// Bumped per call so a stale timeout does nothing.
    gen: u64,
    browser: Option<Browser>,
    observer: Option<Registration>,
    on_closed: Option<Box<dyn FnOnce()>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn fail(what: String) {
    log!("forget-site: failed: {what}");
    with(|s| s.report.failed.push(what));
}

impl SiteForgetter for CefForgetter {
    fn start(&self, job: ForgetJob) {
        let domain = job.domain.clone();
        with(|s| {
            s.job = Some(job);
            s.report = ForgetReport::default();
        });
        let acc = Arc::new(CookieAcc {
            domain,
            deleted: Mutex::new((0, Vec::new())),
        });
        let started = cookie_manager_get_global_manager(None)
            .map(|m| m.visit_all_cookies(Some(&mut CookieDeleter::new(acc.clone()))) != 0)
            .unwrap_or(false);
        if !started {
            fail("cookies".into());
        }
        // The visitor holds the other reference; when CEF releases it (all cookies
        // visited, or none to visit), CookieAcc's drop moves on to the storage.
        drop(acc);
    }

    fn shutdown(&self, then: Box<dyn FnOnce()>) {
        let b = with(|s| {
            s.observer = None;
            s.browser.clone()
        });
        match b.and_then(|b| b.host()) {
            Some(h) => {
                with(|s| s.on_closed = Some(then));
                h.close_browser(1);
            }
            None => then(),
        }
    }
}

/// Shared by the cookie visitor's copies; dropped when CEF releases the visitor.
struct CookieAcc {
    domain: String,
    deleted: Mutex<(u32, Vec<String>)>,
}

impl Drop for CookieAcc {
    fn drop(&mut self) {
        let (n, hosts) = std::mem::take(&mut *self.deleted.lock().unwrap());
        post_task(ThreadId::UI, Some(&mut CookiesDoneTask::new(n, hosts)));
    }
}

wrap_cookie_visitor! {
    struct CookieDeleter {
        acc: Arc<CookieAcc>,
    }

    impl CookieVisitor {
        fn visit(&self, cookie: Option<&Cookie>, _count: i32, _total: i32, delete_cookie: Option<&mut i32>) -> i32 {
            let Some(c) = cookie else { return 1 };
            let domain = c.domain.to_string();
            if protocol::host_in_site(&domain, &self.acc.domain) {
                if let Some(d) = delete_cookie {
                    *d = 1;
                }
                let mut g = self.acc.deleted.lock().unwrap();
                g.0 += 1;
                g.1.push(domain);
            }
            1
        }
    }
}

wrap_task! {
    struct CookiesDoneTask {
        deleted: u32,
        hosts: Vec<String>,
    }

    impl Task {
        fn execute(&self) {
            cookies_done(self.deleted, &self.hosts);
        }
    }
}

fn cookies_done(deleted: u32, hosts: &[String]) {
    let origins = with(|s| {
        let job = s.job.as_ref()?;
        s.report.cookies = deleted;
        let hosts: BTreeSet<String> = hosts.iter().map(|h| h.trim_start_matches('.').to_string()).collect();
        let origins = protocol::forget_origins(&job.domain, &job.recorded, &hosts);
        s.todo = origins.iter().cloned().collect();
        Some(origins)
    });
    let Some(origins) = origins else { return };
    log!(
        "forget-site: {deleted} cookies deleted; clearing the storage of {} origins",
        origins.len()
    );
    if !ensure_browser() {
        fail("storage (no browser to run Storage.clearDataForOrigin on)".into());
        with(|s| s.todo.clear());
    }
    next_origin();
}

/// The hidden browser that DevTools calls run on, with its result observer.
fn ensure_browser() -> bool {
    if with(|s| s.browser.is_some() && s.observer.is_some()) {
        return true;
    }
    let info = WindowInfo {
        windowless_rendering_enabled: 1,
        ..Default::default()
    };
    let b = browser_host_create_browser_sync(
        Some(&info),
        Some(&mut ForgetClient::new()),
        Some(&CefString::from("about:blank")),
        Some(&BrowserSettings::default()),
        None,
        None,
    );
    let Some(b) = b else { return false };
    let reg = b
        .host()
        .and_then(|h| h.add_dev_tools_message_observer(Some(&mut ResultObserver::new())));
    let ok = reg.is_some();
    with(|s| {
        s.browser = Some(b);
        s.observer = reg;
    });
    ok
}

fn next_origin() {
    let next = with(|s| {
        s.gen += 1;
        s.todo.pop_front().map(|o| (o, s.gen))
    });
    let Some((origin, gen)) = next else {
        return after_origins();
    };
    let host = with(|s| s.browser.as_ref().and_then(|b| b.host()));
    let mut params = dictionary_value_create();
    if let Some(p) = params.as_mut() {
        p.set_string(Some(&CefString::from("origin")), Some(&CefString::from(origin.as_str())));
        p.set_string(Some(&CefString::from("storageTypes")), Some(&CefString::from(STORAGE_TYPES)));
    }
    let method = CefString::from("Storage.clearDataForOrigin");
    let id = host
        .map(|h| h.execute_dev_tools_method(0, Some(&method), params.as_mut()))
        .unwrap_or(0);
    if id == 0 {
        fail(format!("storage {origin} (DevTools call refused)"));
        return next_origin();
    }
    trace!("forget: Storage.clearDataForOrigin {origin} (message {id})");
    with(|s| s.pending = Some((origin, id)));
    post_delayed_task(ThreadId::UI, Some(&mut TimeoutTask::new(gen)), ORIGIN_TIMEOUT_MS);
}

/// The result of the pending clearDataForOrigin (UI thread).
fn origin_result(id: i32, success: bool, text: String) {
    let origin = with(|s| match &s.pending {
        Some((o, pid)) if *pid == id => {
            let o = o.clone();
            s.pending = None;
            Some(o)
        }
        _ => None,
    });
    let Some(origin) = origin else { return };
    if success {
        log!("forget-site: {origin}: storage cleared");
        with(|s| s.report.origins.push(origin));
    } else {
        fail(format!("storage {origin} ({text})"));
    }
    next_origin();
}

wrap_task! {
    struct TimeoutTask {
        gen: u64,
    }

    impl Task {
        fn execute(&self) {
            let pending = with(|s| (s.gen == self.gen).then(|| s.pending.take()).flatten());
            if let Some((origin, _)) = pending {
                fail(format!("storage {origin} (timeout)"));
                next_origin();
            }
        }
    }
}

wrap_dev_tools_message_observer! {
    struct ResultObserver {}

    impl DevToolsMessageObserver {
        fn on_dev_tools_method_result(&self, _browser: Option<&mut Browser>, message_id: i32, success: i32, result: Option<&[u8]>) {
            let text = result.map(|r| String::from_utf8_lossy(r).into_owned()).unwrap_or_default();
            trace!("forget: DevTools result {message_id} success {success}: {text}");
            // Chromium's error object: {"code":-32000,"message":"..."}.
            let text = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
                .unwrap_or(text);
            origin_result(message_id, success != 0, text);
        }
    }
}

fn after_origins() {
    // sessionStorage of the page's frames under the site.
    let domain = with(|s| s.job.as_ref().map(|j| j.domain.clone()));
    let Some(domain) = domain else { return };
    if let Some(b) = crate::page::browser() {
        let mut ids = CefStringList::new();
        b.frame_identifiers(Some(&mut ids));
        for id in ids {
            let id = CefString::from(id.to_string().as_str());
            let Some(f) = b.frame_by_identifier(Some(&id)) else { continue };
            let url = CefString::from(&f.url()).to_string();
            if protocol::origin_of(&url)
                .map(|(_, h)| protocol::host_in_site(&h, &domain))
                .unwrap_or(false)
            {
                let js = CefString::from("try { sessionStorage.clear() } catch (e) {}");
                f.execute_java_script(Some(&js), Some(&CefString::from(url.as_str())), 0);
                log!("forget-site: sessionStorage cleared in the page's frame {url}");
            }
        }
    }
    // Storage.clearDataForOrigin leaves the HTTP cache (checked: test/forget.py's
    // cached response was still served from disk), and nothing CEF or DevTools offers
    // removes one site's entries (PROTOCOL.md "forget-site"): clear all of it.
    match request_context_get_global_context() {
        Some(c) => c.clear_http_cache(Some(&mut StepDone::new(1))),
        None => {
            fail("http-cache".into());
            flush_cookies();
        }
    }
}

fn flush_cookies() {
    let ok = cookie_manager_get_global_manager(None)
        .map(|m| m.flush_store(Some(&mut StepDone::new(2))) != 0)
        .unwrap_or(false);
    if !ok {
        done();
    }
}

fn done() {
    let report = with(|s| {
        s.job = None;
        std::mem::take(&mut s.report)
    });
    forget::finished(report);
}

wrap_completion_callback! {
    struct StepDone {
        step: u8,
    }

    impl CompletionCallback {
        fn on_complete(&self) {
            // Not called on a fixed thread for every step: continue on the UI thread.
            post_task(ThreadId::UI, Some(&mut StepTask::new(self.step)));
        }
    }
}

wrap_task! {
    struct StepTask {
        step: u8,
    }

    impl Task {
        fn execute(&self) {
            match self.step {
                1 => {
                    log!("forget-site: HTTP cache cleared (all of it)");
                    flush_cookies()
                }
                _ => done(),
            }
        }
    }
}

fn is_forget(browser: &Option<&mut Browser>) -> bool {
    let id = browser.as_ref().map(|b| b.identifier()).unwrap_or(-1);
    with(|s| s.browser.as_ref().map(|b| b.identifier() == id).unwrap_or(false))
}

wrap_client! {
    struct ForgetClient {}

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(ForgetRenderHandler::new())
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(ForgetRequestHandler::new())
        }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(ForgetLifeSpanHandler::new())
        }
    }
}

wrap_render_handler! {
    struct ForgetRenderHandler {}

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(r) = rect {
                *r = Rect { x: 0, y: 0, width: 64, height: 64 };
            }
        }
    }
}

wrap_request_handler! {
    struct ForgetRequestHandler {}

    impl RequestHandler {
        // The hidden browser stays on about:blank: it never loads anything.
        fn on_before_browse(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _user_gesture: i32,
            _is_redirect: i32,
        ) -> i32 {
            let url = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
            (url != "about:blank") as i32
        }
    }
}

wrap_life_span_handler! {
    struct ForgetLifeSpanHandler {}

    impl LifeSpanHandler {
        fn on_before_close(&self, browser: Option<&mut Browser>) {
            if !is_forget(&browser) {
                return;
            }
            trace!("forget: hidden browser closed");
            let then = with(|s| {
                s.browser = None;
                s.observer = None;
                s.on_closed.take()
            });
            if let Some(then) = then {
                then();
            }
        }
    }
}
