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
//! Popups (C-POPUPS, PROTOCOL.md "Popups"): the host is the popup blocker (Chromium runs with
//! `--disable-popup-blocking`, main.rs). A `window.open` / `target=_blank` with a user gesture
//! becomes a page of its own (`on_popup_browser_view_created` gives it a window of ours;
//! `tab-created` with `opener`); one without is cancelled, recorded in its page (`Page::blocked`)
//! and reported (`popup-blocked`). `popup-open` re-runs a recorded `window.open` in the frame that
//! asked, let through once (`Pages::allow`), or loads its URL in a new page when the frame is gone.
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
use crate::protocol::{self, kind_from_transition, kind_name, Cause, Command, Event, FindDir, FrameKind, LoadKind, LoadPhase, TabId};
use cef::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

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
    /// C-AUTOFOCUS: the frame (CEF's identifier) of the last `editable-focused true`: where
    /// `blur-field` and `focus-field last` run.
    focus_frame: Option<String>,
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
    /// C-POPUPS: the popups blocked since the page's last cross-document load start, oldest first.
    blocked: Vec<Blocked>,
    /// The number of the last blocked popup (`popup-blocked`'s `popup`; never reused in the page).
    last_popup: u32,
}

/// C-POPUPS: a blocked popup, as `popup-open` needs it.
#[derive(Debug, Clone, Default)]
struct Blocked {
    popup: u32,
    /// The frame that called `window.open` (CEF's frame identifier).
    frame: String,
    url: String,
    name: String,
    features: String,
}

/// How a page came to be, for its `tab-created`.
#[derive(Debug, Clone, Copy)]
enum Origin {
    /// `new-tab` with this `req`.
    NewTab(u64),
    /// Opened from page `opener` (a popup, `popup-open`).
    Popup(TabId, Cause),
}

/// C-POPUPS: `popup-open`'s one-shot allow: the next gesture-less popup from this frame of this
/// page to this URL goes through (as `popup-open`). Used up by that popup, or after
/// `REOPEN_WAIT_MS` (then the URL is loaded fresh in a new page).
#[derive(Debug, Clone)]
struct Allow {
    tab: TabId,
    frame: String,
    url: String,
    /// Tells a later allow from this one (the timeout task's check).
    seq: u64,
    at: Instant,
}

/// How long `popup-open` waits for the re-run `window.open` to reach the host before it loads the
/// URL fresh instead (the spike saw it come after 1 ms).
const REOPEN_WAIT_MS: i64 = 1000;

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

/// The URL the page on screen was requested as before redirects (the visible navigation entry's
/// original URL), only when it is not empty and differs from `url`.
fn original_url(browser: Option<&Browser>, url: &str) -> Option<String> {
    let e = browser?.host()?.visible_navigation_entry()?;
    let o = CefString::from(&e.original_url()).to_string();
    (!o.is_empty() && o != url).then_some(o)
}

/// Every page of the host, which one is shown, and the bookkeeping of `new-tab` and `quit`.
#[derive(Default)]
struct Pages {
    pages: BTreeMap<TabId, Page>,
    /// The page shown (the first page at start, then the latest `show-tab`).
    shown: Option<TabId>,
    /// `quit` was asked: the host exits when the last page has closed.
    quitting: bool,
    /// C-CLOSE-POLITE: `close-all` was asked: the host quits once the last page has closed. Not
    /// `quitting`: a page whose user stayed keeps the host fully working (`new-tab`, popups).
    closing_all: bool,
    /// Pages created by the host whose browser is not announced yet: the view and how it came
    /// (`new-tab`'s `req`, or the opener of a popup).
    pending: Vec<(BrowserView, Origin)>,
    /// Pages announced so far (the first one is shown, later ones start hidden).
    announced: u32,
    /// C-POPUPS: popups let through by `on_before_popup` whose view has not been created yet:
    /// (opener, CEF's popup id, cause), oldest first.
    opening: Vec<(TabId, i32, Cause)>,
    /// `popup-open`'s one-shot allow.
    allow: Option<Allow>,
    allow_seq: u64,
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
        let origin = view.as_ref().and_then(|v| {
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
        Some((origin, first))
    });
    let Some((origin, first)) = announce else {
        if window.is_visible() == 0 {
            window.show();
        }
        return;
    };
    let (req, opener, cause) = match origin {
        Some(Origin::NewTab(r)) => (Some(r), None, None),
        Some(Origin::Popup(o, c)) => (None, Some(o), Some(c)),
        None => (None, None, None),
    };
    emit(Event::TabCreated {
        tab: id,
        req,
        opener,
        cause,
    });
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
/// with `req` answers when its browser exists. Also `popup-open`'s fresh load (`Origin::Popup`).
fn new_tab(origin: Origin, url: &str) {
    let what = match origin {
        Origin::NewTab(req) => format!("new-tab {req}"),
        Origin::Popup(o, c) => format!("{} from tab {o}", protocol::cause_name(c)),
    };
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
            message: format!("{what}: the engine could not create a page"),
        });
    };
    log!("{what}: {url}");
    with_pages(|ps| ps.pending.push((bv.clone(), origin)));
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

        // C-POPUPS: a popup's view gets this delegate too (no Chrome toolbar; its own popups come
        // here). The cef crate's default gives it none.
        fn delegate_for_popup_browser_view(
            &self,
            _browser_view: Option<&mut BrowserView>,
            _settings: Option<&BrowserSettings>,
            _client: Option<&mut Client>,
            _is_devtools: i32,
        ) -> Option<BrowserViewDelegate> {
            Some(HostBrowserViewDelegate::new())
        }

        // C-POPUPS: a popup on_before_popup let through. It becomes a page in a window of its own
        // (as `new-tab`'s), announced with its opener; CEF would otherwise give it a Chrome window.
        fn on_popup_browser_view_created(&self, bv: Option<&mut BrowserView>, popup: Option<&mut BrowserView>, is_devtools: i32) -> i32 {
            let (0, Some(popup)) = (is_devtools, popup) else {
                log!("unexpected popup browser view (devtools {is_devtools}); not shown");
                return 1;
            };
            let opener = bv.and_then(|v| v.browser()).map(|b| b.identifier()).unwrap_or(0);
            let origin = with_pages(|ps| {
                let cause = match ps.opening.iter().position(|(o, _, _)| *o == opener) {
                    Some(i) => ps.opening.remove(i).2,
                    None => Cause::Popup,
                };
                let origin = Origin::Popup(opener, cause);
                ps.pending.push((popup.clone(), origin));
                origin
            });
            log!("popup view created: {origin:?}");
            let mut d = HostWindowDelegate::new(RefCell::new(Some(popup.clone())));
            window_create_top_level(Some(&mut d));
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
        // C-CLOSE-POLITE: a forced quit answers `beforeunload` itself; otherwise Chrome's dialog.
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> {
            Some(HostJsdialogHandler::new())
        }
        // C-BELL: notification prompts are denied at once (notify.rs); others keep Chrome's handling.
        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(crate::notify::HostPermissionHandler::new())
        }

        fn on_process_message_received(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            let Some(msg) = message else { return 0 };
            if crate::notify::handle_message(page_of(&browser), msg) || crate::field::result_message(page_of(&browser), msg) {
                return 1;
            }
            let name = CefString::from(&msg.name()).to_string();
            if name == crate::renderer::MIDDLE_MSG {
                // C-MIDCLICK: a middle-click on a link (newtab::MIDDLE_JS): the shell opens it as `:new`.
                let url = msg.argument_list().map(|a| CefString::from(&a.string(0)).to_string()).unwrap_or_default();
                match (page_of(&browser), newtab::link_click_url(&url)) {
                    (Some(tab), Some(url)) => {
                        log!("middle-click on a link: {url}");
                        emit(Event::OpenLink { tab, url });
                    }
                    _ => log!("middle-click message refused: {url:?}"),
                }
                return 1;
            }
            if name == crate::renderer::BLANK_MSG {
                // C-FOCUSEXISTING: a plain left-click on a target=_blank link (newtab::BLANK_JS): the
                // shell opens it, or switches to a buffer already showing it (web.buffers.focusExisting).
                let url = msg.argument_list().map(|a| CefString::from(&a.string(0)).to_string()).unwrap_or_default();
                match (page_of(&browser), newtab::link_click_url(&url)) {
                    (Some(tab), Some(url)) => {
                        log!("target=_blank click on a link: {url}");
                        emit(Event::BlankLink { tab, url });
                    }
                    _ => log!("target=_blank click message refused: {url:?}"),
                }
                return 1;
            }
            if name != crate::renderer::EDITABLE_MSG {
                return 0;
            }
            let Some(id) = page_of(&browser) else { return 1 };
            let editable = msg.argument_list().map(|a| a.bool(0) != 0).unwrap_or(false);
            let frame = frame.map(|f| CefString::from(&f.identifier()).to_string());
            set_editable(id, editable, frame);
            1
        }
    }
}

/// `editable-focused`: `false` on a change only; `true` for every focus that lands in an editable
/// (C-AUTOFOCUS: also from one field straight to another, so a click into a second field after
/// Esc Esc still switches to INPUT, and a page moving its own focus is parked again).
fn set_editable(id: TabId, editable: bool, frame: Option<String>) {
    let changed = with_page(id, |p| {
        let c = p.editable != editable;
        p.editable = editable;
        if editable && frame.is_some() {
            p.focus_frame = frame;
        }
        c
    });
    if changed == Some(true) || (editable && changed.is_some()) {
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
                let original = original_url(browser.as_deref(), &url);
                emit(Event::Url { tab: id, url, original });
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

/// C-POPUPS: `on_before_popup`'s decision for page `id`. Returns 1 to cancel the popup.
#[allow(clippy::too_many_arguments)]
fn before_popup(
    browser: Option<&mut Browser>,
    frame: Option<&mut Frame>,
    popup_id: i32,
    url: String,
    name: String,
    disp: sys::cef_window_open_disposition_t,
    user_gesture: i32,
    features: Option<&PopupFeatures>,
) -> i32 {
    let Some(id) = page_of(&browser) else {
        log!("popup Drop: {url} (not from a page)");
        return 1;
    };
    if with_pages(|ps| ps.quitting) {
        log!("popup Drop: {url} (quitting)");
        return 1;
    }
    let (frame_id, main) = frame
        .as_ref()
        .map(|f| (CefString::from(&f.identifier()).to_string(), f.is_main() != 0))
        .unwrap_or_default();
    // popup-open's one-shot allow: this frame, this URL, not too old. Used up here either way.
    let allowed = user_gesture == 0
        && with_pages(|ps| {
            let ok = ps.allow.as_ref().is_some_and(|a| {
                a.tab == id
                    && a.frame == frame_id
                    && newtab::same_popup_url(&a.url, &url)
                    && a.at.elapsed() < Duration::from_millis(2 * REOPEN_WAIT_MS as u64)
            });
            if ok {
                ps.allow = None;
            }
            ok
        });
    let action = newtab::popup_action(disp, user_gesture != 0, allowed);
    log!(
        "popup {action:?}: {url} (disposition {disp:?}, user gesture {user_gesture}{}, frame {})",
        if allowed { ", popup-open's allow" } else { "" },
        if main { "main" } else { "sub" }
    );
    match action {
        newtab::PopupAction::Open => {
            let cause = if allowed { Cause::PopupOpen } else { Cause::Popup };
            with_pages(|ps| ps.opening.push((id, popup_id, cause)));
            0
        }
        newtab::PopupAction::Block => {
            let f = features.cloned().unwrap_or_default();
            let set = |v: i32, s: i32| (s != 0).then_some(v);
            let features = newtab::features_text(
                f.is_popup != 0,
                set(f.x, f.x_set),
                set(f.y, f.y_set),
                set(f.width, f.width_set),
                set(f.height, f.height_set),
            );
            let popup = with_page(id, |p| {
                p.last_popup += 1;
                p.blocked.push(Blocked {
                    popup: p.last_popup,
                    frame: frame_id,
                    url: url.clone(),
                    name: name.clone(),
                    features: features.clone(),
                });
                p.last_popup
            })
            .unwrap_or(0);
            emit(Event::PopupBlocked {
                tab: id,
                popup,
                url,
                name,
                frame: if main { FrameKind::Main } else { FrameKind::Sub },
                features,
            });
            1
        }
        newtab::PopupAction::LoadInPlace => {
            load_in_place(browser, url);
            1
        }
        newtab::PopupAction::Drop => 1,
    }
}

/// C-POPUPS `popup-open <tab> <n>`: blocked popup `n` of page `tab`, taken out of its record. Its
/// `window.open` runs again in the frame that asked with a one-shot allow (`window.opener` kept);
/// with the frame gone, its URL loads in a new page. A re-run that never reaches the host within
/// `REOPEN_WAIT_MS` (the page replaced `window.open`, the frame went meanwhile) loads it fresh too.
fn popup_open(tab: TabId, n: u32) -> Result<(), String> {
    if with_pages(|ps| ps.quitting) {
        return Err(format!("popup-open {tab} {n}: the host is quitting"));
    }
    let (browser, b) = with_page(tab, |p| {
        let i = p.blocked.iter().position(|b| b.popup == n)?;
        Some((p.browser.clone(), p.blocked.remove(i)))
    })
    .ok_or_else(|| format!("popup-open: no tab {tab}"))?
    .ok_or_else(|| format!("popup-open: tab {tab} has no blocked popup {n}"))?;
    let frame = browser
        .and_then(|br| br.frame_by_identifier(Some(&CefString::from(b.frame.as_str()))))
        .filter(|f| f.is_valid() != 0);
    let Some(frame) = frame else {
        log!("popup-open {tab} {n}: its frame is gone: {} in a new page", b.url);
        new_tab(Origin::Popup(tab, Cause::PopupFresh), &b.url);
        return Ok(());
    };
    let seq = with_pages(|ps| {
        ps.allow_seq += 1;
        ps.allow = Some(Allow {
            tab,
            frame: b.frame.clone(),
            url: b.url.clone(),
            seq: ps.allow_seq,
            at: Instant::now(),
        });
        ps.allow_seq
    });
    let js = newtab::reopen_js(&b.url, &b.name, &b.features);
    log!("popup-open {tab} {n}: {js} in frame {}", b.frame);
    frame.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
    post_delayed_task(ThreadId::UI, Some(&mut ReopenCheck::new(seq, tab, b.url)), REOPEN_WAIT_MS);
    Ok(())
}

wrap_task! {
    struct ReopenCheck {
        seq: u64,
        tab: TabId,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            let unused = with_pages(|ps| {
                let mine = ps.allow.as_ref().is_some_and(|a| a.seq == self.seq);
                if mine {
                    ps.allow = None;
                }
                mine && !ps.quitting
            });
            if unused {
                log!("popup-open: the re-run never reached the host: {} in a new page", self.url);
                new_tab(Origin::Popup(self.tab, Cause::PopupFresh), &self.url);
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
        // C-POPUPS (newtab.rs): with a user gesture a new page, without one blocked and reported;
        // user new-tab gestures (Ctrl/Shift/middle click) do nothing.
        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            popup_id: i32,
            target_url: Option<&CefString>,
            target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: i32,
            popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            let url = target_url.map(|u| u.to_string()).unwrap_or_default();
            let name = target_frame_name.map(|u| u.to_string()).unwrap_or_default();
            let disp = sys::cef_window_open_disposition_t::from(target_disposition);
            before_popup(browser, frame, popup_id, url, name, disp, user_gesture, popup_features)
        }

        // A popup let through and then not created after all: forget its opening record.
        fn on_before_popup_aborted(&self, browser: Option<&mut Browser>, popup_id: i32) {
            let opener = browser.map(|b| b.identifier()).unwrap_or(0);
            with_pages(|ps| ps.opening.retain(|(o, p, _)| !(*o == opener && *p == popup_id)));
            log!("popup {popup_id} of tab {opener} aborted");
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
            let (left, quitting, closing_all) = with_pages(|ps| {
                ps.pages.remove(&id);
                if ps.shown == Some(id) {
                    ps.shown = None;
                }
                (ps.pages.len(), ps.quitting, ps.closing_all)
            });
            crate::notify::forget_tab(id);
            emit(Event::TabClosed { tab: id });
            // The last page gone ends the host only on `quit` (or standalone, when its window was
            // closed): the shell opens a new page before it closes its last one (PROTOCOL.md).
            if left == 0 && (quitting || !crate::control::connected()) {
                log!("tab {id} closed, no page left: quitting");
                quit_message_loop();
            } else if left == 0 && closing_all {
                // C-CLOSE-POLITE: every page left after `close-all`; quit() (forget::shutdown, no
                // page left) ends the message loop.
                log!("tab {id} closed, no page left after close-all: quitting");
                quit();
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
                // C-POPUPS: another document: its blocked popups go (the shell's list too).
                p.blocked.clear();
                p.focus_frame = None;
                p.editable
            });
            load_event(id, LoadPhase::Start, 0, kind, None);
            // The new document starts with nothing focused.
            if was_editable == Some(true) {
                set_editable(id, false, None);
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
                let original = original_url(browser.as_deref(), &url);
                emit(Event::Url { tab: id, url, original });
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
            let action = newtab::open_url_action(disp);
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
        // C-BELL: the renderers learn `--notify-sites` (notify.rs reads it at context creation).
        fn on_before_child_process_launch(&self, command_line: Option<&mut CommandLine>) {
            if let Some(cmd) = command_line {
                crate::notify::child_switch(cmd);
            }
        }

        fn on_context_initialized(&self) {
            apply_cookie_policy();
            // C-BELL: Chromium's own notifications are blocked (they would go to the desktop).
            crate::notify::block_native();
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

/// The hosts of every page that has a browser.
fn page_hosts(ps: &Pages) -> Vec<BrowserHost> {
    ps.pages
        .values()
        .filter_map(|p| p.browser.as_ref().and_then(|b| b.host()))
        .collect()
}

/// How long `quit` waits for its pages to close before it ends the message loop anyway.
const QUIT_GRACE_MS: i64 = 2000;

/// `quit` (also SIGTERM, and the shell closing the control socket): closes every page with
/// `close_browser(1)`, forced. CEF 156 still runs the pages' `beforeunload` then and would show
/// the "Leave site?" dialog (seen live, C-CLOSE-POLITE): `HostJsdialogHandler` answers it "Leave"
/// itself while quitting. A dialog already on screen (from a `close-all`) is not asked again, so
/// whatever happens the host ends `QUIT_GRACE_MS` later (`QuitGraceTask`); normally
/// on_before_close ends the message loop when the last page is gone. A second call does nothing.
pub fn quit() {
    let Some(hosts) = with_pages(|ps| {
        if ps.quitting {
            return None;
        }
        ps.quitting = true;
        Some(page_hosts(ps))
    }) else {
        return;
    };
    crate::forget::shutdown(Box::new(|| {}));
    if hosts.is_empty() {
        return quit_message_loop();
    }
    log!("quit: closing {} page(s)", hosts.len());
    for h in hosts {
        h.close_browser(1);
    }
    post_delayed_task(ThreadId::UI, Some(&mut QuitGraceTask::new()), QUIT_GRACE_MS);
}

wrap_task! {
    struct QuitGraceTask {}

    impl Task {
        fn execute(&self) {
            let left = with_pages(|ps| ps.pages.len());
            if left > 0 {
                log!("quit: {left} page(s) did not close; ending anyway");
                quit_message_loop();
                // With Chrome-style browsers CefQuitMessageLoop only drops CEF's keep-alive and
                // waits for every browser window to go (libcef main_runner.cc
                // HandleMainMessageLoopQuit; seen live: the loop kept running with the dialog up),
                // so the message loop would never end: exit without CEF's shutdown. The helper
                // processes end when their browser process is gone.
                log!("quit: exiting without CEF shutdown");
                std::process::exit(0);
            }
        }
    }
}

wrap_jsdialog_handler! {
    struct HostJsdialogHandler {}

    impl JsdialogHandler {
        /// C-CLOSE-POLITE: while quitting (forced: `quit`, SIGTERM) the host answers "Leave"
        /// itself, without a dialog; otherwise (`close-all`, a navigation) Chrome's own dialog asks.
        fn on_before_unload_dialog(
            &self,
            browser: Option<&mut Browser>,
            _message_text: Option<&CefString>,
            _is_reload: ::std::os::raw::c_int,
            callback: Option<&mut JsdialogCallback>,
        ) -> ::std::os::raw::c_int {
            if !with_pages(|ps| ps.quitting) {
                return 0;
            }
            let Some(cb) = callback else { return 0 };
            let id = browser.map(|b| b.identifier()).unwrap_or(0);
            log!("quit: tab {id}: beforeunload answered Leave without a dialog");
            cb.cont(1, Some(&CefString::from("")));
            1
        }
    }
}

/// `close-all` (C-CLOSE-POLITE: the shell's window X, `:q`, SIGINT): closes every page with
/// `close_browser(0)`, so each runs its `beforeunload`; a page whose user chose to stay simply
/// stays open (Chrome: each page decides for itself). on_before_close quits once the last page
/// is gone.
pub fn close_all_politely() {
    let Some(hosts) = with_pages(|ps| {
        if ps.quitting {
            return None;
        }
        ps.closing_all = true;
        Some(page_hosts(ps))
    }) else {
        return;
    };
    if hosts.is_empty() {
        return quit();
    }
    log!("close-all: asking {} page(s) to close", hosts.len());
    for h in hosts {
        h.close_browser(0);
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
        Command::CloseAll => return close_all_politely(),
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
        Command::NewTab { req, url } => return new_tab(Origin::NewTab(req), url.as_deref().unwrap_or("about:blank")),
        Command::PopupOpen { tab, popup } => {
            if let Err(message) = popup_open(tab, popup) {
                emit(Event::Error { message });
            }
            return;
        }
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
        // C-AUTOFOCUS: the frame of the last editable focus (else the main frame) for `blur` and
        // `last`; `first` looks in the main frame.
        Command::BlurField | Command::FocusField { .. } => {
            let op = match cmd {
                Command::FocusField { which } => which.into(),
                _ => protocol::FieldOp::Blur,
            };
            let frame = match op {
                protocol::FieldOp::First => None,
                _ => with_page(id, |p| p.focus_frame.clone()).flatten(),
            };
            crate::field::request(id, &browser, frame.as_deref(), op);
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
        | Command::CloseAll
        | Command::ForgetSite { .. }
        | Command::Caret { .. }
        | Command::NewTab { .. }
        | Command::ShowTab { .. }
        | Command::CloseTab { .. }
        | Command::PopupOpen { .. } => unreachable!(),
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
