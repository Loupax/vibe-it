//! `forget-site` (STASH.md "Forget site", PROTOCOL.md): deletes a site's cookies and
//! the storage of every origin under it, with CEF's own API and no DevTools /
//! automation protocol (DECISIONS.md "Architecture C": no CDP/BiDi).
//!
//! CEF has a cookie API but no per-origin storage API (its only one is the DevTools
//! method `Storage.clearDataForOrigin`). So the host clears an origin's storage from
//! inside that origin, the way the site itself could:
//!
//! 1. Cookies: `CookieManager::visit_all_cookies` deletes every cookie whose domain is
//!    the site or under it, and collects those domains.
//! 2. Origins: which origins have storage cannot be asked of CEF, so the host records
//!    the origin of every frame its page loads (`<profile>/web-host-origins`, one per
//!    line). The origins cleared are the recorded ones under the site, plus http and
//!    https of the domain, its `www.` and every cookie host (protocol::forget_origins).
//! 3. Storage: a hidden, windowless browser of the host's own loads
//!    `<origin>/__web-host-forget__` for each origin. The host answers that request
//!    itself (a `ResourceHandler`) with a page whose script clears localStorage,
//!    IndexedDB (every database), Cache Storage and unregisters every service worker,
//!    then reports through its title. Every other request of that browser is
//!    cancelled, so no request reaches the site (Chromium may still open a connection
//!    to it first: its navigation preconnect). A service worker of the origin may
//!    answer the navigation itself (its page has another title); then the host
//!    reloads with `reload_ignore_cache`, which bypasses service workers (Shift+Reload).
//! 4. sessionStorage lives with the page that made it: frames of the host's page that
//!    are under the site run `sessionStorage.clear()`.
//! 5. The HTTP cache: CEF can only clear all of it (`RequestContext::clear_http_cache`),
//!    so the whole cache goes. Then the cookie store is flushed to disk.
//!
//! Known gaps (PROTOCOL.md): storage the site has as a third party inside another
//! site's page (partitioned storage) is not reached; origins the host never loaded and
//! that set no cookie (e.g. pages loaded before this record existed) are not known.

use crate::control::emit;
use crate::protocol::{self, Event, ForgetResult};
use cef::*;
use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

pub const MAGIC_PATH: &str = "/__web-host-forget__";
const TITLE_PREFIX: &str = "web-host-forget:";
const ORIGIN_TIMEOUT_MS: i64 = 15_000;

/// `<profile>/web-host-origins`, set in main.
pub static RECORD: OnceLock<String> = OnceLock::new();
/// `--no-window`: quit after the first forget; its exit code.
pub static ONE_SHOT: OnceLock<bool> = OnceLock::new();
pub static EXIT_CODE: Mutex<i32> = Mutex::new(0);

struct Run {
    site: String,
    domain: String,
    cookies: u32,
    cookie_hosts: BTreeSet<String>,
    todo: VecDeque<String>,
    origins: Vec<String>,
    failed: Vec<String>,
    /// The origin being cleared and whether it was reloaded past a service worker.
    current: Option<String>,
    retried: bool,
    /// Bumped per origin so a stale timeout does nothing.
    gen: u64,
}

#[derive(Default)]
struct State {
    queue: VecDeque<(String, String)>,
    run: Option<Run>,
    browser: Option<Browser>,
    recorded: Option<BTreeSet<String>>,
    /// The hidden browser's current title (the forget page's is `web-host-forget`).
    title: String,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn load_record(s: &mut State) -> &mut BTreeSet<String> {
    s.recorded.get_or_insert_with(|| {
        RECORD
            .get()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| t.lines().filter(|l| !l.is_empty()).map(String::from).collect())
            .unwrap_or_default()
    })
}

/// Records the origin of a URL the page loaded (any frame). UI thread.
pub fn record(url: &str) {
    let Some((origin, _)) = protocol::origin_of(url) else { return };
    let new = with(|s| load_record(s).insert(origin.clone()));
    if !new {
        return;
    }
    let Some(path) = RECORD.get() else { return };
    let f = std::fs::OpenOptions::new().create(true).append(true).open(path);
    match f {
        Ok(mut f) => {
            use std::os::unix::fs::PermissionsExt;
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
            let _ = f.write_all(format!("{origin}\n").as_bytes());
        }
        Err(e) => log!("cannot record origin {origin} in {path}: {e}"),
    }
}

/// Drops the forgotten origins from the record (the record names visited sites).
fn unrecord(domain: &str) {
    let kept: Vec<String> = with(|s| {
        let rec = load_record(s);
        rec.retain(|o| {
            protocol::origin_of(o)
                .map(|(_, h)| !protocol::host_in_site(&h, domain))
                .unwrap_or(false)
        });
        rec.iter().cloned().collect()
    });
    let Some(path) = RECORD.get() else { return };
    let tmp = format!("{path}.tmp");
    let body: String = kept.iter().map(|o| format!("{o}\n")).collect();
    let res = std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, path));
    if let Err(e) = res {
        log!("cannot rewrite the origin record {path}: {e}");
    }
}

/// `forget-site <site>` (UI thread). Runs now, or after the forget in progress.
pub fn forget_site(site: String) {
    let domain = match protocol::forget_domain(&site) {
        Ok(d) => d,
        Err(e) => {
            emit(Event::Error {
                message: format!("forget-site: {e}"),
            });
            return one_shot_done(2);
        }
    };
    let busy = with(|s| {
        s.queue.push_back((site, domain));
        s.run.is_some()
    });
    if !busy {
        start_next();
    }
}

fn start_next() {
    let Some((site, domain)) = with(|s| s.queue.pop_front()) else {
        return;
    };
    log!("forget-site {site}: domain {domain} and every host under it");
    with(|s| {
        s.run = Some(Run {
            site,
            domain: domain.clone(),
            cookies: 0,
            cookie_hosts: BTreeSet::new(),
            todo: VecDeque::new(),
            origins: Vec::new(),
            failed: Vec::new(),
            current: None,
            retried: false,
            gen: 0,
        })
    });
    let acc = Arc::new(CookieAcc {
        domain,
        deleted: Mutex::new((0, Vec::new())),
    });
    let started = cookie_manager_get_global_manager(None)
        .map(|m| m.visit_all_cookies(Some(&mut CookieDeleter::new(acc.clone()))) != 0)
        .unwrap_or(false);
    if !started {
        log!("forget-site: the cookie store cannot be read");
        with(|s| s.run.as_mut().map(|r| r.failed.push("cookies".into())));
    }
    // The visitor holds the other reference; when CEF releases it (all cookies
    // visited, or none to visit), CookieAcc's drop moves on to the origins.
    drop(acc);
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
            cookies_done(self.deleted, self.hosts.clone());
        }
    }
}

fn cookies_done(deleted: u32, hosts: Vec<String>) {
    let origins = with(|s| {
        let recorded = load_record(s).clone();
        let run = s.run.as_mut()?;
        run.cookies = deleted;
        run.cookie_hosts.extend(hosts.iter().map(|h| h.trim_start_matches('.').to_string()));
        let origins = protocol::forget_origins(&run.domain, &recorded, &run.cookie_hosts);
        run.todo = origins.iter().cloned().collect();
        Some(origins)
    });
    let Some(origins) = origins else { return };
    log!(
        "forget-site: {deleted} cookies deleted; clearing the storage of {} origins",
        origins.len()
    );
    next_origin();
}

fn next_origin() {
    let next = with(|s| {
        let run = s.run.as_mut()?;
        run.gen += 1;
        run.retried = false;
        run.current = run.todo.pop_front();
        Some((run.current.clone(), run.gen))
    });
    let Some((next, gen)) = next else { return };
    let Some(origin) = next else {
        return after_origins();
    };
    with(|s| s.title.clear());
    let url = format!("{origin}{MAGIC_PATH}");
    trace!("forget: loading {url}");
    let browser = with(|s| s.browser.clone());
    match browser.and_then(|b| b.main_frame()) {
        Some(f) => f.load_url(Some(&CefString::from(url.as_str()))),
        None => {
            if !create_browser(&url) {
                with(|s| s.run.as_mut().map(|r| r.failed.push("storage (no hidden browser)".into())));
                return after_origins();
            }
        }
    }
    post_delayed_task(ThreadId::UI, Some(&mut OriginTimeoutTask::new(gen)), ORIGIN_TIMEOUT_MS);
}

wrap_task! {
    struct OriginTimeoutTask {
        gen: u64,
    }

    impl Task {
        fn execute(&self) {
            let stale = with(|s| s.run.as_ref().map(|r| r.gen != self.gen || r.current.is_none()).unwrap_or(true));
            if !stale {
                origin_done(Some("timeout".into()), vec![]);
            }
        }
    }
}

/// One origin finished: `err` when the forget page did not report, else the storage
/// kinds its script could not clear.
fn origin_done(err: Option<String>, failed_kinds: Vec<String>) {
    with(|s| {
        let Some(run) = s.run.as_mut() else { return };
        let Some(origin) = run.current.take() else { return };
        match &err {
            Some(e) => run.failed.push(format!("storage {origin} ({e})")),
            None => {
                for k in &failed_kinds {
                    run.failed.push(format!("{k} {origin}"));
                }
                run.origins.push(origin.clone());
            }
        }
        log!(
            "forget-site: {origin}: {}",
            err.clone().unwrap_or_else(|| if failed_kinds.is_empty() {
                "cleared".into()
            } else {
                format!("failed {failed_kinds:?}")
            })
        );
    });
    next_origin();
}

fn after_origins() {
    // sessionStorage of the page's frames under the site.
    let domain = with(|s| s.run.as_ref().map(|r| r.domain.clone()));
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
    let started = request_context_get_global_context().map(|c| {
        c.clear_http_cache(Some(&mut StepDone::new(1)));
    });
    if started.is_none() {
        with(|s| s.run.as_mut().map(|r| r.failed.push("http-cache".into())));
        flush_cookies();
    }
}

fn flush_cookies() {
    let ok = cookie_manager_get_global_manager(None)
        .map(|m| m.flush_store(Some(&mut StepDone::new(2))) != 0)
        .unwrap_or(false);
    if !ok {
        finish();
    }
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
                _ => finish(),
            }
        }
    }
}

fn finish() {
    let Some(run) = with(|s| s.run.take()) else { return };
    unrecord(&run.domain);
    let result = if run.failed.is_empty() {
        ForgetResult::Ok
    } else {
        ForgetResult::Partial
    };
    let ev = Event::Forgot {
        site: run.site,
        domain: run.domain,
        result,
        failed: run.failed,
        cookies: run.cookies,
        origins: run.origins,
    };
    if *ONE_SHOT.get().unwrap_or(&false) {
        println!("{}", protocol::encode_event(&ev));
    }
    emit(ev);
    let more = with(|s| !s.queue.is_empty());
    if more {
        return start_next();
    }
    close_browser();
    one_shot_done(if result == ForgetResult::Ok { 0 } else { 1 });
}

/// `--no-window`: quit when the one forget is done.
fn one_shot_done(code: i32) {
    if *ONE_SHOT.get().unwrap_or(&false) {
        *EXIT_CODE.lock().unwrap() = code;
        if with(|s| s.browser.is_none()) {
            quit_message_loop();
        }
        // Else on_before_close of the hidden browser quits.
    }
}

pub fn close_browser() {
    if let Some(h) = with(|s| s.browser.clone()).and_then(|b| b.host()) {
        h.close_browser(1);
    }
}

fn create_browser(url: &str) -> bool {
    let info = WindowInfo {
        windowless_rendering_enabled: 1,
        ..Default::default()
    };
    let mut client = ForgetClient::new();
    let b = browser_host_create_browser_sync(
        Some(&info),
        Some(&mut client),
        Some(&CefString::from(url)),
        Some(&BrowserSettings::default()),
        None,
        None,
    );
    let ok = b.is_some();
    with(|s| s.browser = b);
    ok
}

fn is_forget(browser: &Option<&mut Browser>) -> bool {
    let id = browser.as_ref().map(|b| b.identifier()).unwrap_or(-1);
    with(|s| s.browser.as_ref().map(|b| b.identifier() == id).unwrap_or(false))
}

/// The forget page: clears what an origin keeps, then reports in its title.
const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><title>web-host-forget</title><script>
(async () => {
  const failed = [];
  const step = async (name, f) => { try { await f(); } catch (e) { failed.push(name); } };
  await step('localStorage', () => localStorage.clear());
  await step('indexeddb', async () => {
    if (!self.indexedDB) return;
    const dbs = await indexedDB.databases();
    await Promise.all(dbs.map(d => new Promise((ok, bad) => {
      const r = indexedDB.deleteDatabase(d.name);
      // blocked: a page still has it open; the delete completes when it closes.
      r.onsuccess = ok; r.onblocked = ok; r.onerror = () => bad(r.error);
    })));
  });
  await step('cache-storage', async () => {
    if (!self.caches) return;
    for (const k of await caches.keys()) await caches.delete(k);
  });
  await step('service-workers', async () => {
    if (!navigator.serviceWorker) return;
    for (const r of await navigator.serviceWorker.getRegistrations()) await r.unregister();
  });
  document.title = 'web-host-forget:' + JSON.stringify(failed);
})();
</script>"#;

wrap_client! {
    struct ForgetClient {}

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(ForgetRenderHandler::new())
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(ForgetRequestHandler::new())
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(ForgetDisplayHandler::new())
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(ForgetLoadHandler::new())
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
        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            _is_navigation: i32,
            _is_download: i32,
            _request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut i32>,
        ) -> Option<ResourceRequestHandler> {
            Some(ForgetResourceRequestHandler::new())
        }
    }
}

fn is_magic(request: &Option<&mut Request>) -> Option<String> {
    let url = CefString::from(&request.as_ref()?.url()).to_string();
    let path = url.split_once("://")?.1;
    let path = &path[path.find('/')?..];
    (path == MAGIC_PATH).then_some(url)
}

wrap_resource_request_handler! {
    struct ForgetResourceRequestHandler {}

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            // The hidden browser never touches the network: only the forget page loads.
            let req = request;
            if is_magic(&req).is_some() {
                ReturnValue::CONTINUE
            } else {
                trace!("forget: cancelled {}", req.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default());
                ReturnValue::CANCEL
            }
        }

        fn resource_handler(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>) -> Option<ResourceHandler> {
            is_magic(&request)?;
            Some(ForgetPage::new(Arc::new(Mutex::new(0))))
        }
    }
}

wrap_resource_handler! {
    struct ForgetPage {
        offset: Arc<Mutex<usize>>,
    }

    impl ResourceHandler {
        fn open(&self, _request: Option<&mut Request>, handle_request: Option<&mut i32>, _callback: Option<&mut Callback>) -> i32 {
            if let Some(h) = handle_request {
                *h = 1;
            }
            1
        }

        fn response_headers(&self, response: Option<&mut Response>, response_length: Option<&mut i64>, _redirect_url: Option<&mut CefString>) {
            if let Some(r) = response {
                r.set_status(200);
                r.set_mime_type(Some(&CefString::from("text/html")));
                r.set_header_by_name(Some(&CefString::from("Cache-Control")), Some(&CefString::from("no-store")), 1);
            }
            if let Some(l) = response_length {
                *l = PAGE.len() as i64;
            }
        }

        fn read(&self, data_out: *mut u8, bytes_to_read: i32, bytes_read: Option<&mut i32>, _callback: Option<&mut ResourceReadCallback>) -> i32 {
            let mut off = self.offset.lock().unwrap();
            let rest = &PAGE.as_bytes()[*off..];
            let n = rest.len().min(bytes_to_read.max(0) as usize);
            if n > 0 {
                unsafe { std::ptr::copy_nonoverlapping(rest.as_ptr(), data_out, n) };
                *off += n;
            }
            if let Some(b) = bytes_read {
                *b = n as i32;
            }
            (n > 0) as i32
        }
    }
}

wrap_display_handler! {
    struct ForgetDisplayHandler {}

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            if !is_forget(&browser) {
                return;
            }
            let t = title.map(|t| t.to_string()).unwrap_or_default();
            trace!("forget: title {t:?}");
            with(|s| s.title = t.clone());
            let Some(json) = t.strip_prefix(TITLE_PREFIX) else { return };
            let failed: Vec<String> = serde_json::from_str(json).unwrap_or_else(|_| vec!["storage (bad report)".into()]);
            origin_done(None, failed);
        }
    }
}

wrap_load_handler! {
    struct ForgetLoadHandler {}

    impl LoadHandler {
        fn on_load_end(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, http_status_code: i32) {
            trace!(
                "forget: load end {http_status_code} {} (forget browser {}, main {})",
                frame.as_ref().map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default(),
                is_forget(&browser),
                frame.as_ref().map(|f| f.is_main()).unwrap_or(-1)
            );
            if !is_forget(&browser) || !frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false) {
                return;
            }
            let url = frame.map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default();
            // A service worker of the origin answered the navigation (its page has
            // another title): reload past it. Whether the host's resource handler was
            // asked says nothing: a starting service worker races the network request.
            let retry = with(|s| {
                let ours = s.title.starts_with("web-host-forget");
                let run = s.run.as_mut()?;
                let cur = run.current.as_ref()?;
                if !url.starts_with(cur.as_str()) || ours || run.retried {
                    return None;
                }
                run.retried = true;
                Some(cur.clone())
            });
            if let (Some(origin), Some(b)) = (retry, browser) {
                log!("forget-site: {origin}: the page did not come from the host (a service worker?); reloading past it");
                b.reload_ignore_cache();
            }
        }

        fn on_load_error(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, error_code: Errorcode, _error_text: Option<&CefString>, failed_url: Option<&CefString>) {
            if !is_forget(&browser) || !frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false) {
                return;
            }
            let url = failed_url.map(|u| u.to_string()).unwrap_or_default();
            let name = format!("{:?}", sys::cef_errorcode_t::from(error_code));
            log!("forget-site: hidden browser load error {name} for {url}");
            let ours = with(|s| s.run.as_ref().and_then(|r| r.current.as_ref()).map(|c| url.starts_with(c.as_str())).unwrap_or(false));
            if ours && name != "ERR_ABORTED" {
                origin_done(Some(name), vec![]);
            }
        }
    }
}

wrap_life_span_handler! {
    struct ForgetLifeSpanHandler {}

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            _target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            1
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            if !is_forget(&browser) {
                return;
            }
            with(|s| s.browser = None);
            trace!("forget: hidden browser closed");
            if *ONE_SHOT.get().unwrap_or(&false) && with(|s| s.run.is_none()) {
                quit_message_loop();
            }
        }
    }
}
