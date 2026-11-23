//! The pages (C4: several, one per buffer of the shell): each page's window (CEF Views,
//! page only), the CEF handlers that become protocol events, and the commands that act on
//! a page. Everything here runs on CEF's UI thread; the pages' state lives in a
//! thread-local, keyed by the page id (the CEF browser identifier).
//!
//! Several pages (PROTOCOL.md "Several pages"): every page is a browser in its own
//! top-level window (CEF 156 cannot put two Chrome-style BrowserViews in one window,
//! C4-SPIKE). One page is shown; the host hides every other one itself
//! (`BrowserView::SetVisible(false)`: a background page, rAF stopped, timers throttled), since
//! a window the compositor merely does not draw keeps running at full speed.
//!
//! Load events: CEF calls `on_load_start` when a cross-document navigation commits
//! (with its transition type) and `on_load_end` when it finished. Same-document
//! navigations (fragment, pushState) get no load callbacks, only `on_address_change`;
//! an address change with no cross-document load around it becomes a
//! `load start/end same-document` pair. A failed load that shows an error page gets
//! `on_load_error` and `on_load_end` but no `on_load_start`: the host sends the start.
//! A load stopped before commit gets only `on_load_error` (ERR_ABORTED): after the
//! shell's `stop`, the host sends the start/end pair itself.

use crate::control::emit;
use crate::newtab;
use crate::protocol::{self, kind_from_transition, kind_name, Command, Event, FindDir, LoadKind, LoadPhase, TabId};
use cef::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub static START_URL: OnceLock<String> = OnceLock::new();
/// `--no-window`: no page at all (the host only runs `--forget-site`).
pub static NO_WINDOW: OnceLock<bool> = OnceLock::new();
/// `--forget-site <site>`: run at start.
pub static FORGET_AT_START: OnceLock<String> = OnceLock::new();

/// Every page's browser (forget-site clears sessionStorage in all of them).
pub fn browsers() -> Vec<Browser> {
    with_pages(|ps| ps.pages.values().filter_map(|p| p.browser.clone()).collect())
}

#[derive(Default)]
struct Page {
    browser: Option<Browser>,
    /// The page's view in its own window (shown or hidden with `set_visible`).
    view: Option<BrowserView>,
    id: TabId,
    /// Chromium's loading state (on_loading_state_change).
    loading: bool,
    /// A cross-document load started during the current loading period. A
    /// back-forward-cache restore calls on_address_change after on_load_end.
    period_load: bool,
    /// A cross-document load has started (committed) and not ended: its kind.
    load: Option<LoadKind>,
    /// The main frame's address changed during `loading` with no load start yet.
    addr_pending: bool,
    /// Error of the current main-frame load (on_load_error), for its end event.
    error: Option<String>,
    url: String,
    title: String,
    editable: bool,
    find_query: Option<String>,
    find_index: i32,
    /// A new find session still to step once it has counted matches (see Command::Find):
    /// the query, `forward` and `match_case` for `CefBrowserHost::Find`.
    find_step: Option<(String, i32, i32)>,
    /// The id of the `find` whose `find-result` is still owed (C5-STALE). CEF also reports
    /// results nobody asked for (a stale `0 0` at a load start): with no request pending
    /// they are dropped, and a result carries the id of the request it answers.
    find_pending: Option<u64>,
    /// C5b: a find from the top of the screen waits for the page's `web-host.find-start` line:
    /// (query, forward, match_case).
    find_start: Option<(String, i32, i32)>,
    /// C5b: the page said there is no match at or below the top of the screen (the find
    /// wrapped); goes out with the `find-result`.
    find_wrapped: bool,
    /// The shell sent `stop` and no load has started since.
    stop_requested: bool,
}

/// The kind of the navigation entry the page shows (or is about to show).
fn entry_kind(browser: Option<&Browser>) -> LoadKind {
    browser
        .and_then(|b| b.host())
        .and_then(|h| h.visible_navigation_entry())
        .map(|e| {
            let t = e.transition_type();
            // See on_load_start: a bit set in a #[repr(u32)] enum wrapper.
            kind_from_transition(unsafe { std::mem::transmute_copy::<TransitionType, u32>(&t) })
        })
        .unwrap_or(LoadKind::New)
}

/// Every page of the host, which one is shown, and the bookkeeping of `new-tab` and `quit`.
#[derive(Default)]
struct Pages {
    pages: BTreeMap<TabId, Page>,
    /// The page shown (the first page at start, then the latest `show-tab`).
    shown: Option<TabId>,
    /// `quit` was asked: the host exits when the last page has closed.
    quitting: bool,
    /// `new-tab`s whose browser is not announced yet: its view and the shell's `req`.
    pending: Vec<(BrowserView, u64)>,
    /// Pages announced so far (the first one is shown, later ones start hidden).
    announced: u32,
}

thread_local! {
    static PAGES: RefCell<Pages> = RefCell::new(Pages::default());
}

fn with_pages<R>(f: impl FnOnce(&mut Pages) -> R) -> R {
    PAGES.with(|p| f(&mut p.borrow_mut()))
}

/// Runs `f` on page `id`; None when there is no such page (closed, or never announced).
fn with_page<R>(id: TabId, f: impl FnOnce(&mut Page) -> R) -> Option<R> {
    with_pages(|ps| ps.pages.get_mut(&id).map(f))
}

/// The id of the page `browser` is, if it is one (forget-site's helper browser is not).
fn page_of(browser: &Option<&mut Browser>) -> Option<TabId> {
    let id = browser.as_ref()?.identifier();
    with_pages(|ps| ps.pages.contains_key(&id)).then_some(id)
}

fn is_main(frame: &Option<&mut Frame>) -> bool {
    frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false)
}

fn load_event(id: TabId, phase: LoadPhase, status: i32, kind: LoadKind, error: Option<String>) {
    emit(Event::Load {
        tab: id,
        phase,
        status,
        kind,
        error,
    });
}

/// Announces a page (once) and maps its window. `tab-created` always goes out before
/// `show()`, and windows are shown in the order of their `tab-created`s, so the shell can
/// pair each with the window that appears (PROTOCOL.md "Pairing"). The first page is the
/// shown one; a page from `new-tab` starts hidden (its view invisible, its window mapped).
fn announce_and_show(browser: &Browser, window: &Window) {
    let id = browser.identifier();
    let mut b = browser.clone();
    let view = browser_view_get_for_browser(Some(&mut b));
    let announce = with_pages(|ps| {
        if ps.pages.contains_key(&id) {
            return None;
        }
        let req = view.as_ref().and_then(|v| {
            let i = ps.pending.iter().position(|(pv, _)| pv.is_same(Some(&mut View::from(v))) != 0)?;
            Some(ps.pending.remove(i).1)
        });
        let first = ps.announced == 0;
        ps.announced += 1;
        if first {
            ps.shown = Some(id);
        }
        ps.pages.insert(
            id,
            Page {
                browser: Some(browser.clone()),
                view: view.clone(),
                id,
                ..Default::default()
            },
        );
        Some((req, first))
    });
    let Some((req, first)) = announce else {
        if window.is_visible() == 0 {
            window.show();
        }
        return;
    };
    emit(Event::TabCreated { tab: id, req });
    if window.is_visible() == 0 {
        window.show();
        log!("window of tab {id} shown");
    }
    if !first {
        // Hidden right here, the page still reports `visible` (the view is not laid out in its
        // window yet, checked with test/drive.py): hide it from a task, once the window is up.
        post_delayed_task(ThreadId::UI, Some(&mut HideTask::new(id)), HIDE_NEW_PAGE_MS);
    }
}

/// How long after its window is shown a new page is hidden (`HideTask`).
const HIDE_NEW_PAGE_MS: i64 = 50;

wrap_task! {
    struct HideTask {
        id: TabId,
    }

    impl Task {
        fn execute(&self) {
            let id = self.id;
            let view = with_pages(|ps| (ps.shown != Some(id)).then(|| ps.pages.get(&id).and_then(|p| p.view.clone())).flatten());
            if let Some(v) = view {
                v.set_visible(0);
                log!("tab {id} is hidden until show-tab");
            }
        }
    }
}

/// `show-tab`: page `id` visible and focused, every other page hidden.
fn show_tab(id: TabId) -> Result<(), String> {
    let views = with_pages(|ps| {
        if !ps.pages.contains_key(&id) {
            return None;
        }
        ps.shown = Some(id);
        Some(ps.pages.values().map(|p| (p.id, p.view.clone())).collect::<Vec<_>>())
    })
    .ok_or_else(|| format!("show-tab: no tab {id}"))?;
    for (pid, v) in &views {
        if *pid != id {
            if let Some(v) = v {
                v.set_visible(0);
            }
        }
    }
    if let Some((_, Some(v))) = views.iter().find(|(pid, _)| *pid == id) {
        v.set_visible(1);
        v.request_focus();
    }
    log!("tab {id} shown, {} hidden", views.len() - 1);
    Ok(())
}

/// `new-tab`: a new page in a window of its own, hidden until `show-tab`; `tab-created`
/// with `req` answers when its browser exists.
fn new_tab(req: u64, url: &str) {
    let url = protocol::normalize_url(url);
    let mut client = Some(HostClient::new());
    let bv = browser_view_create(
        client.as_mut(),
        Some(&CefString::from(url.as_str())),
        Some(&BrowserSettings::default()),
        None,
        None,
        Some(&mut HostBrowserViewDelegate::new()),
    );
    let Some(bv) = bv else {
        return emit(Event::Error {
            message: format!("new-tab {req}: the engine could not create a page"),
        });
    };
    log!("new-tab {req}: {url}");
    with_pages(|ps| ps.pending.push((bv.clone(), req)));
    let mut d = HostWindowDelegate::new(RefCell::new(Some(bv)));
    window_create_top_level(Some(&mut d));
}

wrap_window_delegate! {
    struct HostWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 1200, height: 800 }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let bv = self.browser_view.borrow();
            let (Some(window), Some(bv)) = (window, bv.as_ref()) else { return };
            window.add_child_view(Some(&mut View::from(bv)));
            trace!("on_window_created: browser {}", bv.browser().map(|b| b.identifier()).unwrap_or(0));
            // The browser may not exist yet; then on_after_created shows the window.
            if let Some(b) = bv.browser() {
                announce_and_show(&b, window);
            }
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            *self.browser_view.borrow_mut() = None;
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let bv = self.browser_view.borrow();
            match bv.as_ref().and_then(|bv| bv.browser()) {
                Some(b) => b.host().map(|h| h.try_close_browser()).unwrap_or(1),
                None => 1,
            }
        }

        // The shell (C2) sizes the page window to its own. The cef crate's default (0)
        // pins the window at preferred_size (xdg_toplevel min = max size), so a
        // configure from the shell would be ignored.
        fn can_resize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_maximize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn linux_window_properties(&self, _window: Option<&mut Window>, properties: Option<&mut LinuxWindowProperties>) -> i32 {
            let Some(p) = properties else { return 0 };
            p.wayland_app_id = CefString::from("web-host-cef");
            p.wm_class_class = CefString::from("web-host-cef");
            p.wm_class_name = CefString::from("web-host-cef");
            1
        }
    }
}

wrap_browser_view_delegate! {
    struct HostBrowserViewDelegate {}

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        // The cef crate's default is CEF_CTT_UNKNOWN; say it: no Chrome toolbar.
        fn chrome_toolbar_type(&self, _browser_view: Option<&mut BrowserView>) -> ChromeToolbarType {
            ChromeToolbarType::from(sys::cef_chrome_toolbar_type_t::CEF_CTT_NONE)
        }

        fn on_popup_browser_view_created(&self, _bv: Option<&mut BrowserView>, _popup: Option<&mut BrowserView>, is_devtools: i32) -> i32 {
            // on_before_popup cancels every popup, so this is never reached for pages.
            log!("unexpected popup browser view (devtools {is_devtools}); not shown");
            1
        }
    }
}

wrap_client! {
    struct HostClient {}

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(HostDisplayHandler::new())
        }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(HostLifeSpanHandler::new())
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(HostLoadHandler::new())
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(HostRequestHandler::new())
        }
        fn find_handler(&self) -> Option<FindHandler> {
            Some(HostFindHandler::new())
        }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(HostContextMenuHandler::new())
        }
        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(crate::download::HostDownloadHandler::new())
        }
        fn command_handler(&self) -> Option<CommandHandler> {
            Some(HostCommandHandler::new())
        }

        fn on_process_message_received(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            let Some(msg) = message else { return 0 };
            if CefString::from(&msg.name()).to_string() != crate::renderer::EDITABLE_MSG {
                return 0;
            }
            let Some(id) = page_of(&browser) else { return 1 };
            let editable = msg.argument_list().map(|a| a.bool(0) != 0).unwrap_or(false);
            set_editable(id, editable);
            1
        }
    }
}

fn set_editable(id: TabId, editable: bool) {
    let changed = with_page(id, |p| {
        let c = p.editable != editable;
        p.editable = editable;
        c
    });
    if changed == Some(true) {
        emit(Event::EditableFocused {
            tab: id,
            focused: editable,
        });
    }
}

wrap_display_handler! {
    struct HostDisplayHandler {}

    impl DisplayHandler {
        fn on_address_change(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            let Some(id) = page_of(&browser) else { return };
            if !is_main(&frame) {
                return;
            }
            let url = url.map(|u| u.to_string()).unwrap_or_default();
            let Some((same_doc, changed)) = with_page(id, |p| {
                trace!("on_address_change {url} (loading {}, load {:?})", p.loading, p.load);
                let changed = p.url != url;
                p.url = url.clone();
                let same_doc = if p.load.is_some() || (p.loading && p.period_load) {
                    false
                } else if p.loading {
                    // Decided when loading stops: a load start may still come.
                    p.addr_pending = true;
                    false
                } else {
                    true
                };
                (same_doc, changed)
            }) else {
                return;
            };
            if changed {
                emit(Event::Url { tab: id, url });
            }
            if same_doc {
                same_document(id);
            }
        }

        // C5b: the answer of find_start.js. A page can print the same line; the worst it does
        // is start a waiting find early, or claim a wrap.
        fn on_console_message(
            &self,
            browser: Option<&mut Browser>,
            _level: LogSeverity,
            message: Option<&CefString>,
            _source: Option<&CefString>,
            _line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let m = message.map(|m| m.to_string()).unwrap_or_default();
            let Some(wrapped) = m.strip_prefix("web-host.find-start:") else { return 0 };
            let Some(id) = page_of(&browser) else { return 0 };
            let Some((query, forward, match_case)) = with_page(id, |p| p.find_start.take()).flatten() else { return 1 };
            with_page(id, |p| p.find_wrapped = wrapped == "1");
            if let Some(h) = browser.as_ref().and_then(|b| b.host()) {
                start_find(id, &h, &query, forward, match_case, false);
            }
            1
        }

        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let Some(id) = page_of(&browser) else { return };
            let t = title.map(|t| t.to_string()).unwrap_or_default();
            // CEF repeats the title on every navigation start; send changes only.
            let changed = with_page(id, |p| {
                let c = p.title != t;
                p.title = t.clone();
                c
            });
            if changed != Some(true) {
                return;
            }
            emit(Event::Title { tab: id, title: t });
            let mut browser = browser.cloned();
            if let Some(w) = browser_view_get_for_browser(browser.as_mut()).and_then(|bv| bv.window()) {
                w.set_title(title);
            }
        }
    }
}

/// Loads `url` in the page, from a task (callers are inside a CEF popup request).
fn load_in_place(browser: Option<&mut Browser>, url: String) {
    if !url.is_empty() && url != "about:blank" {
        if let Some(f) = browser.and_then(|b| b.main_frame()) {
            post_task(ThreadId::UI, Some(&mut LoadTask::new(f, url)));
        }
    }
}

fn same_document(id: TabId) {
    load_event(id, LoadPhase::Start, 0, LoadKind::SameDocument, None);
    load_event(id, LoadPhase::End, 0, LoadKind::SameDocument, None);
}

wrap_life_span_handler! {
    struct HostLifeSpanHandler {}

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            // One page per host (newtab.rs): window.open and plain target=_blank load in
            // this page, user new-tab gestures (Ctrl/Shift/middle click) do nothing;
            // the popup is always cancelled (the opener gets null from window.open).
            let url = target_url.map(|u| u.to_string()).unwrap_or_default();
            let disp = sys::cef_window_open_disposition_t::from(target_disposition);
            let action = newtab::popup_action(disp);
            log!("popup {action:?}: {url} (disposition {disp:?}, user gesture {user_gesture})");
            if action == newtab::PopupAction::LoadInPlace {
                load_in_place(browser, url);
            }
            1
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(b) = browser else { return };
            trace!("on_after_created {}", b.identifier());
            let mut bb = Some(b.clone());
            if let Some(w) = browser_view_get_for_browser(bb.as_mut()).and_then(|bv| bv.window()) {
                announce_and_show(b, &w);
            }
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let Some(id) = page_of(&browser) else { return };
            let (left, quitting) = with_pages(|ps| {
                ps.pages.remove(&id);
                if ps.shown == Some(id) {
                    ps.shown = None;
                }
                (ps.pages.len(), ps.quitting)
            });
            emit(Event::TabClosed { tab: id });
            // The last page gone ends the host only on `quit` (or standalone, when its window was
            // closed): the shell opens a new page before it closes its last one (PROTOCOL.md).
            if left == 0 && (quitting || !crate::control::connected()) {
                log!("tab {id} closed, no page left: quitting");
                quit_message_loop();
            } else {
                log!("tab {id} closed, {left} left");
            }
        }
    }
}

wrap_task! {
    struct LoadTask {
        frame: Frame,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            self.frame.load_url(Some(&CefString::from(self.url.as_str())));
        }
    }
}

/// CEF's error code as Chromium's name (`ERR_ABORTED`).
fn error_name(code: Errorcode) -> String {
    format!("{:?}", sys::cef_errorcode_t::from(code))
}

wrap_load_handler! {
    struct HostLoadHandler {}

    impl LoadHandler {
        fn on_loading_state_change(&self, browser: Option<&mut Browser>, is_loading: i32, can_go_back: i32, can_go_forward: i32) {
            let Some(id) = page_of(&browser) else { return };
            trace!("on_loading_state_change loading {is_loading} back {can_go_back} forward {can_go_forward}");
            let same_doc = with_page(id, |p| {
                p.loading = is_loading != 0;
                if p.loading {
                    p.period_load = false;
                }
                !p.loading && std::mem::take(&mut p.addr_pending) && !p.period_load
            });
            if same_doc == Some(true) {
                same_document(id);
            }
        }

        fn on_load_start(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, transition_type: TransitionType) {
            // Every frame's origin, for forget-site (forget.rs: CEF cannot list origins).
            let Some(id) = page_of(&browser) else { return };
            if let Some(f) = frame.as_ref() {
                crate::forget::record(&CefString::from(&f.url()).to_string());
            }
            if !is_main(&frame) {
                return;
            }
            // TransitionType wraps a #[repr(u32)] enum that really is a bit set
            // (source | qualifier flags): read the raw value.
            let raw: u32 = unsafe { std::mem::transmute_copy(&transition_type) };
            let kind = kind_from_transition(raw);
            trace!("on_load_start transition {raw:#x} -> {}", kind_name(kind));
            let was_editable = with_page(id, |p| {
                p.load = Some(kind);
                p.stop_requested = false;
                p.period_load = true;
                p.addr_pending = false;
                p.find_query = None;
                p.find_step = None;
                p.find_pending = None;
                p.find_start = None;
                p.editable
            });
            load_event(id, LoadPhase::Start, 0, kind, None);
            // The new document starts with nothing focused.
            if was_editable == Some(true) {
                set_editable(id, false);
            }
        }

        fn on_load_end(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, http_status_code: i32) {
            let Some(id) = page_of(&browser) else { return };
            if !is_main(&frame) {
                return;
            }
            trace!("on_load_end {http_status_code}");
            // A back-forward-cache restore reports 0: the entry still has the status
            // the page was loaded with.
            let status = if http_status_code == 0 {
                browser
                    .as_ref()
                    .and_then(|b| b.host())
                    .and_then(|h| h.visible_navigation_entry())
                    .map(|e| e.http_status_code())
                    .unwrap_or(0)
            } else {
                http_status_code
            };
            // A back-forward-cache restore calls on_address_change only after this:
            // send the new URL first, so `url` always precedes its `load end`.
            let url = frame.as_ref().map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default();
            let Some((kind, error, new_url)) = with_page(id, |p| {
                let new_url = (!url.is_empty() && p.url != url).then(|| {
                    p.url = url.clone();
                    url.clone()
                });
                (p.load.take(), p.error.take(), new_url)
            }) else {
                return;
            };
            // An error page (on_load_error, e.g. ERR_NAME_NOT_RESOLVED) ends with no
            // on_load_start: send the start here.
            let kind = kind.unwrap_or_else(|| {
                let k = entry_kind(browser.as_deref());
                load_event(id, LoadPhase::Start, 0, k, None);
                k
            });
            if let Some(url) = new_url {
                emit(Event::Url { tab: id, url });
            }
            load_event(id, LoadPhase::End, status, kind, error);
        }

        fn on_load_error(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let Some(id) = page_of(&browser) else { return };
            if !is_main(&frame) {
                return;
            }
            let name = error_name(error_code);
            log!(
                "load error: {} {name} {}",
                failed_url.map(|u| u.to_string()).unwrap_or_default(),
                error_text.map(|t| t.to_string()).unwrap_or_default()
            );
            // ERR_ABORTED commits no error page, so no load start/end follows. Report it
            // when the shell stopped the load; otherwise it was replaced by another
            // navigation (which gets its own events) or was the engine's own retry of
            // an error page, which the shell has no use for.
            let aborted = name == "ERR_ABORTED";
            let (stopped, in_end) = with_page(id, |p| {
                if aborted && p.load.is_none() {
                    (std::mem::take(&mut p.stop_requested), false)
                } else {
                    p.error = Some(name.clone());
                    (false, true)
                }
            })
            .unwrap_or((false, false));
            // The shell hears of every failure it will also see in a `load end`.
            if in_end || stopped {
                let code = sys::cef_errorcode_t::from(error_code) as i32;
                let text = error_text.map(|t| t.to_string()).unwrap_or_default();
                emit(Event::LoadError { tab: id, code, name: name.clone(), text });
            }
            if stopped {
                let kind = entry_kind(browser.as_deref());
                load_event(id, LoadPhase::Start, 0, kind, None);
                load_event(id, LoadPhase::End, 0, kind, Some(name));
            }
        }
    }
}

wrap_request_handler! {
    struct HostRequestHandler {}

    impl RequestHandler {
        // A navigation CEF wants to open in another tab or window (outside the popup
        // path, e.g. a context-menu or Chrome command): always cancelled, loaded here
        // only when newtab::popup_action says so.
        fn on_open_urlfrom_tab(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            target_url: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: i32,
        ) -> i32 {
            let url = target_url.map(|u| u.to_string()).unwrap_or_default();
            let disp = sys::cef_window_open_disposition_t::from(target_disposition);
            let action = newtab::popup_action(disp);
            log!("open-url-from-tab {action:?}: {url} (disposition {disp:?}, user gesture {user_gesture})");
            if action == newtab::PopupAction::LoadInPlace {
                load_in_place(browser, url);
            }
            1
        }

        fn on_render_process_terminated(
            &self,
            browser: Option<&mut Browser>,
            status: TerminationStatus,
            error_code: i32,
            _error_string: Option<&CefString>,
        ) {
            let Some(id) = page_of(&browser) else { return };
            let reason = format!("{:?} (exit code {error_code})", sys::cef_termination_status_t::from(status));
            with_page(id, |p| {
                p.load = None;
                p.loading = false;
                p.addr_pending = false;
                p.editable = false;
            });
            emit(Event::Crashed { tab: id, reason });
        }
    }
}

wrap_find_handler! {
    struct HostFindHandler {}

    impl FindHandler {
        fn on_find_result(
            &self,
            browser: Option<&mut Browser>,
            _identifier: i32,
            count: i32,
            _selection_rect: Option<&Rect>,
            active_match_ordinal: i32,
            final_update: i32,
        ) {
            let Some(id) = page_of(&browser) else { return };
            trace!("on_find_result count {count} active {active_match_ordinal} final {final_update}");
            let Some((find_id, index, step)) = with_page(id, |p| {
                if p.find_pending.is_none() {
                    return (None, 0, None);
                }
                if active_match_ordinal > 0 {
                    p.find_index = active_match_ordinal;
                }
                // The count of a new session is final: with matches, step once (below)
                // and report that result; with none, report the zero.
                let step = if final_update != 0 && count > 0 { p.find_step.take() } else { None };
                if final_update != 0 {
                    p.find_step = None;
                }
                (p.find_pending, p.find_index, step)
            }) else {
                return;
            };
            let Some(find_id) = find_id else {
                trace!("on_find_result dropped: no find pending");
                return;
            };
            if let Some((query, forward, match_case)) = step {
                if let Some(h) = browser.as_ref().and_then(|b| b.host()) {
                    h.find(Some(&CefString::from(query.as_str())), forward, match_case, 1);
                }
                return;
            }
            // Intermediate updates carry partial counts; the shell gets the final one.
            if final_update != 0 {
                let wrapped = with_page(id, |p| {
                    p.find_pending = None;
                    std::mem::take(&mut p.find_wrapped)
                })
                .unwrap_or(false);
                emit(Event::FindResult { tab: id, id: find_id, index: if count == 0 { 0 } else { index }, count, wrapped });
            }
        }
    }
}

wrap_context_menu_handler! {
    struct HostContextMenuHandler {}

    impl ContextMenuHandler {
        // No context menu at all (C-NOMENU, user 2026-11-23): its useful actions move to
        // `:` commands (C-CMDS). Two layers: the model is emptied here (an empty model
        // shows nothing), and run_context_menu says "handled" and cancels, in case a
        // build runs the menu anyway. The right-click itself still reaches the page
        // (the `contextmenu` event is dispatched before the browser asks for a menu), so
        // pages with their own right-click menus keep working.
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            let Some(model) = model else { return };
            let n = model.count();
            model.clear();
            log!("context menu: suppressed ({n} items cleared)");
        }

        fn run_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _params: Option<&mut ContextMenuParams>,
            _model: Option<&mut MenuModel>,
            callback: Option<&mut RunContextMenuCallback>,
        ) -> ::std::os::raw::c_int {
            if let Some(cb) = callback {
                cb.cancel();
            }
            log!("context menu: run suppressed");
            1
        }
    }
}

wrap_command_handler! {
    struct HostCommandHandler {}

    impl CommandHandler {
        // Chrome style maps keys like Ctrl+T, Ctrl+N, Ctrl+F, Ctrl+L to Chrome commands
        // that would open Chrome UI (new windows, tab strip, find bar). The shell owns
        // keys and UI: block every Chrome command.
        fn on_chrome_command(&self, _browser: Option<&mut Browser>, command_id: i32, _disposition: WindowOpenDisposition) -> i32 {
            log!("chrome command {command_id} blocked");
            1
        }
    }
}

wrap_browser_process_handler! {
    pub struct HostBrowserProcessHandler {}

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            apply_cookie_policy();
            // C-CARETCMD: caret browsing starts off in every run (the preference is saved in
            // the profile, so a `:set caret` of an earlier run would otherwise come back).
            set_caret_browsing(false);
            if let Some(site) = FORGET_AT_START.get() {
                crate::forget::forget_site(site.clone());
            }
            if *NO_WINDOW.get().unwrap_or(&false) {
                log!("context initialized, no window");
                return;
            }
            let url = START_URL.get().cloned().unwrap_or_else(|| "about:blank".into());
            log!("context initialized, opening {url}");
            let mut client = Some(HostClient::new());
            let browser_view = browser_view_create(
                client.as_mut(),
                Some(&CefString::from(url.as_str())),
                Some(&BrowserSettings::default()),
                None,
                None,
                Some(&mut HostBrowserViewDelegate::new()),
            );
            let mut d = HostWindowDelegate::new(RefCell::new(browser_view));
            window_create_top_level(Some(&mut d));
        }
    }
}

pub static ALLOW_THIRD_PARTY_COOKIES: OnceLock<bool> = OnceLock::new();

/// Third-party cookies off (STASH.md "Cookies"): `profile.cookie_controls_mode` = 1 on
/// the global request context, before any page loads; read back and logged.
/// Note: the `error` out-parameter must not be `CefString::default()` (an unset
/// string): with it `SetPreference` returns false with no message and sets nothing.
fn apply_cookie_policy() {
    let allow = *ALLOW_THIRD_PARTY_COOKIES.get().unwrap_or(&false);
    let mode = crate::protocol::cookie_controls_mode(allow);
    let Some(ctx) = request_context_get_global_context() else {
        log!("cookies: no global request context, third-party cookie policy NOT set");
        return;
    };
    let name = CefString::from("profile.cookie_controls_mode");
    let Some(mut v) = value_create() else { return };
    v.set_int(mode);
    let mut err = CefString::from("");
    let ok = ctx.set_preference(Some(&name), Some(&mut v), Some(&mut err));
    let now = ctx.preference(Some(&name)).map(|v| v.int());
    log!(
        "cookies: profile.cookie_controls_mode={mode} (third-party {}): set={ok} now={now:?} {err}",
        if allow { "default" } else { "blocked" }
    );
}

/// Caret browsing on/off (C-CARETCMD, `:set caret`): Chrome's preference
/// `settings.a11y.caretbrowsing.enabled` on the global request context, the one the page
/// uses. Chrome watches it and updates every open page's settings, so it takes effect
/// without a reload. Read back and logged; a failure is an `error` event.
fn set_caret_browsing(on: bool) -> bool {
    let Some(ctx) = request_context_get_global_context() else {
        log!("caret: no global request context");
        return false;
    };
    let name = CefString::from(protocol::CARET_PREF);
    let Some(mut v) = value_create() else { return false };
    v.set_bool(on as i32);
    // Not CefString::default(): SetPreference then fails silently (see apply_cookie_policy).
    let mut err = CefString::from("");
    let ok = ctx.set_preference(Some(&name), Some(&mut v), Some(&mut err));
    let now = ctx.preference(Some(&name)).map(|v| v.bool() != 0);
    log!("caret: {}={on}: set={ok} now={now:?} {err}", protocol::CARET_PREF);
    ok != 0 && now == Some(on)
}

/// `quit`: closes every page (no beforeunload prompts); on_before_close quits the message
/// loop when the last one is gone.
pub fn quit() {
    crate::forget::shutdown(Box::new(|| {}));
    let hosts = with_pages(|ps| {
        ps.quitting = true;
        ps.pages
            .values()
            .filter_map(|p| p.browser.as_ref().and_then(|b| b.host()))
            .collect::<Vec<_>>()
    });
    if hosts.is_empty() {
        return quit_message_loop();
    }
    log!("quit: closing {} page(s)", hosts.len());
    for h in hosts {
        h.close_browser(1);
    }
}

/// The page script that puts the selection at the top of the screen (find_start.js), called with
/// the query and the case flag.
fn find_start_js(query: &str, match_case: bool) -> String {
    let q = serde_json::to_string(query).expect("a string serializes");
    // JSON allows U+2028/2029 inside strings, JavaScript before ES2019 did not: escape them.
    let q = q.replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029");
    format!("{}({q}, {match_case});", include_str!("find_start.js"))
}

/// Starts (or steps) the engine's find. A new session counts the matches but activates none
/// (active ordinal 0); on_find_result steps once so the first (or, for prev, last) match is
/// active, as in a browser's find bar, and reports only that.
fn start_find(id: TabId, h: &BrowserHost, query: &str, forward: i32, match_case: i32, again: bool) {
    with_page(id, |p| p.find_step = (!again).then(|| (query.to_string(), forward, match_case)));
    if !again {
        // After a navigation CEF still holds the old session; the same text would be
        // taken for the same search and never answer (found live by C5). Ending it
        // first is what `find-clear` does, which works.
        h.stop_finding(0);
    }
    h.find(Some(&CefString::from(query)), forward, match_case, again as i32);
}

/// Runs one command line from the shell (UI thread).
pub fn run_command(line: &str) {
    let (cmd, tab) = match protocol::parse_command_tab(line) {
        Ok(c) => c,
        Err(message) => return emit(Event::Error { message }),
    };
    match cmd {
        Command::Quit => return quit(),
        // Needs no page (also with --no-window).
        Command::ForgetSite { site } => return crate::forget::forget_site(site),
        // A profile setting: needs no page.
        Command::Caret { on } => {
            if !set_caret_browsing(on) {
                emit(Event::Error {
                    message: format!("caret: could not set {} to {on}", protocol::CARET_PREF),
                });
            }
            return;
        }
        Command::NewTab { req, .. } if with_pages(|ps| ps.quitting) => {
            // A browser created while CEF shuts down crashes the host (seen live by C4).
            return emit(Event::Error {
                message: format!("new-tab {req}: the host is quitting"),
            });
        }
        Command::NewTab { req, url } => return new_tab(req, url.as_deref().unwrap_or("about:blank")),
        Command::ShowTab { tab } => {
            if let Err(message) = show_tab(tab) {
                emit(Event::Error { message });
            }
            return;
        }
        Command::CloseTab { tab } => {
            let h = with_page(tab, |p| p.browser.as_ref().and_then(|b| b.host())).flatten();
            match h {
                Some(h) => h.close_browser(1),
                None => emit(Event::Error {
                    message: format!("close-tab: no tab {tab}"),
                }),
            }
            return;
        }
        _ => {}
    }
    // A page command: the page `tab` names, else the shown one.
    let Some(id) = tab.or_else(|| with_pages(|ps| ps.shown)) else {
        return emit(Event::Error {
            message: "no page yet".into(),
        });
    };
    let Some(browser) = with_page(id, |p| p.browser.clone()).flatten() else {
        return emit(Event::Error {
            message: format!("no tab {id}"),
        });
    };
    let host = browser.host();
    match cmd {
        Command::Open { url } => {
            if let Some(f) = browser.main_frame() {
                f.load_url(Some(&CefString::from(protocol::normalize_url(&url).as_str())));
            }
        }
        Command::Back => browser.go_back(),
        Command::Forward => browser.go_forward(),
        Command::Reload { hard: false } => browser.reload(),
        Command::Reload { hard: true } => browser.reload_ignore_cache(),
        Command::Stop => {
            with_page(id, |p| p.stop_requested = true);
            browser.stop_load()
        }
        Command::Find {
            id: find_id,
            query,
            dir,
            match_case,
            screen_top,
        } => {
            let Some(h) = host else { return };
            let again = with_page(id, |p| {
                p.find_pending = Some(find_id);
                p.find_start = None;
                p.find_wrapped = false;
                let again = p.find_query.as_deref() == Some(query.as_str());
                if !again {
                    p.find_query = Some(query.clone());
                    p.find_index = 0;
                }
                again
            })
            .unwrap_or(false);
            let forward = (dir == FindDir::Next) as i32;
            if screen_top && !again {
                // C5b (T23): the engine's find starts from the selection, so first put the
                // selection at the first text at or below the top of the screen; the page
                // answers with a console line (see on_console_message) and the find runs then.
                if let Some(f) = browser.main_frame() {
                    with_page(id, |p| p.find_start = Some((query.clone(), forward, match_case as i32)));
                    f.execute_java_script(Some(&CefString::from(find_start_js(&query, match_case).as_str())), None, 0);
                    return;
                }
            }
            start_find(id, &h, &query, forward, match_case as i32, again);
        }
        Command::FindClear => {
            with_page(id, |p| {
                p.find_query = None;
                p.find_index = 0;
                p.find_pending = None;
                p.find_start = None;
            });
            // Keep the active match selected (C5: Enter on a match acts on it).
            if let Some(h) = host {
                h.stop_finding(0);
            }
        }
        Command::Zoom { factor } => match protocol::zoom_level(factor) {
            Ok(level) => {
                if let Some(h) = host {
                    h.set_zoom_level(level);
                    log!("zoom factor {factor} -> level {level:.3} (tab {id})");
                }
            }
            Err(message) => emit(Event::Error { message }),
        },
        Command::Quit
        | Command::ForgetSite { .. }
        | Command::Caret { .. }
        | Command::NewTab { .. }
        | Command::ShowTab { .. }
        | Command::CloseTab { .. } => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::find_start_js;

    #[test]
    fn find_start_js_passes_the_query_as_a_json_string() {
        let js = find_start_js("a\"b\\c\n</script>\u{2028}", true);
        assert!(js.ends_with(r#"("a\"b\\c\n</script>\u2028", true);"#), "{js}");
        assert!(!js.contains('\u{2028}'));
        let js = find_start_js("needle", false);
        assert!(js.ends_with(r#"("needle", false);"#));
        assert!(js.contains("web-host.find-start:"));
    }
}
