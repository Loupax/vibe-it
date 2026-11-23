//! The one page: its window (CEF Views, page only), the CEF handlers that become
//! protocol events, and the commands that act on it. Everything here runs on CEF's
//! UI thread; the page state lives in a thread-local.
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
use std::sync::OnceLock;

pub static START_URL: OnceLock<String> = OnceLock::new();
/// `--no-window`: no page at all (the host only runs `--forget-site`).
pub static NO_WINDOW: OnceLock<bool> = OnceLock::new();
/// `--forget-site <site>`: run at start.
pub static FORGET_AT_START: OnceLock<String> = OnceLock::new();

/// The page's browser, once created.
pub fn browser() -> Option<Browser> {
    with_page(|p| p.browser.clone())
}

#[derive(Default)]
struct Page {
    browser: Option<Browser>,
    id: TabId,
    announced: bool,
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

thread_local! {
    static PAGE: RefCell<Page> = RefCell::new(Page::default());
}

fn with_page<R>(f: impl FnOnce(&mut Page) -> R) -> R {
    PAGE.with(|p| f(&mut p.borrow_mut()))
}

fn is_page(browser: &Option<&mut Browser>) -> bool {
    match browser {
        Some(b) => with_page(|p| p.id != 0 && p.id == b.identifier()),
        None => false,
    }
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

/// Announces the page (once) and maps its window. `tab-created` always goes out
/// before `show()`, so the shell can pair it with the window that appears.
fn announce_and_show(browser: &Browser, window: &Window) {
    let id = browser.identifier();
    let first = with_page(|p| {
        if p.announced {
            return false;
        }
        p.announced = true;
        p.id = id;
        p.browser = Some(browser.clone());
        true
    });
    if first {
        emit(Event::TabCreated { tab: id });
    }
    if window.is_visible() == 0 {
        window.show();
        log!("window of tab {id} shown");
    }
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
            if !is_page(&browser) {
                return 1;
            }
            let editable = msg.argument_list().map(|a| a.bool(0) != 0).unwrap_or(false);
            set_editable(editable);
            1
        }
    }
}

fn set_editable(editable: bool) {
    let changed = with_page(|p| {
        let c = p.editable != editable;
        p.editable = editable;
        c.then_some(p.id)
    });
    if let Some(id) = changed {
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
            if !is_page(&browser) || !is_main(&frame) {
                return;
            }
            let url = url.map(|u| u.to_string()).unwrap_or_default();
            let (id, same_doc, changed) = with_page(|p| {
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
                (p.id, same_doc, changed)
            });
            if changed {
                emit(Event::Url { tab: id, url });
            }
            if same_doc {
                same_document(id);
            }
        }

        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            if !is_page(&browser) {
                return;
            }
            let t = title.map(|t| t.to_string()).unwrap_or_default();
            // CEF repeats the title on every navigation start; send changes only.
            let changed = with_page(|p| (p.title != t).then(|| {
                p.title = t.clone();
                p.id
            }));
            let Some(id) = changed else { return };
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
            if !is_page(&browser) {
                return;
            }
            let id = with_page(|p| {
                p.browser = None;
                p.id
            });
            emit(Event::TabClosed { tab: id });
            log!("page closed, quitting");
            quit_message_loop();
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
            if !is_page(&browser) {
                return;
            }
            trace!("on_loading_state_change loading {is_loading} back {can_go_back} forward {can_go_forward}");
            let (id, same_doc) = with_page(|p| {
                p.loading = is_loading != 0;
                if p.loading {
                    p.period_load = false;
                }
                let same_doc = !p.loading && std::mem::take(&mut p.addr_pending) && !p.period_load;
                (p.id, same_doc)
            });
            if same_doc {
                same_document(id);
            }
        }

        fn on_load_start(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, transition_type: TransitionType) {
            // Every frame's origin, for forget-site (forget.rs: CEF cannot list origins).
            if is_page(&browser) {
                if let Some(f) = frame.as_ref() {
                    crate::forget::record(&CefString::from(&f.url()).to_string());
                }
            }
            if !is_page(&browser) || !is_main(&frame) {
                return;
            }
            // TransitionType wraps a #[repr(u32)] enum that really is a bit set
            // (source | qualifier flags): read the raw value.
            let raw: u32 = unsafe { std::mem::transmute_copy(&transition_type) };
            let kind = kind_from_transition(raw);
            trace!("on_load_start transition {raw:#x} -> {}", kind_name(kind));
            let (id, was_editable) = with_page(|p| {
                p.load = Some(kind);
                p.stop_requested = false;
                p.period_load = true;
                p.addr_pending = false;
                p.find_query = None;
                p.find_step = None;
                (p.id, p.editable)
            });
            load_event(id, LoadPhase::Start, 0, kind, None);
            // The new document starts with nothing focused.
            if was_editable {
                set_editable(false);
            }
        }

        fn on_load_end(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, http_status_code: i32) {
            if !is_page(&browser) || !is_main(&frame) {
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
            let (id, kind, error, new_url) = with_page(|p| {
                let new_url = (!url.is_empty() && p.url != url).then(|| {
                    p.url = url.clone();
                    url.clone()
                });
                (p.id, p.load.take(), p.error.take(), new_url)
            });
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
            if !is_page(&browser) || !is_main(&frame) {
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
            let (stopped, in_end) = with_page(|p| {
                if aborted && p.load.is_none() {
                    (std::mem::take(&mut p.stop_requested), false)
                } else {
                    p.error = Some(name.clone());
                    (false, true)
                }
            });
            // The shell hears of every failure it will also see in a `load end`.
            if in_end || stopped {
                let id = with_page(|p| p.id);
                let code = sys::cef_errorcode_t::from(error_code) as i32;
                let text = error_text.map(|t| t.to_string()).unwrap_or_default();
                emit(Event::LoadError { tab: id, code, name: name.clone(), text });
            }
            if stopped {
                let id = with_page(|p| p.id);
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
            if !is_page(&browser) {
                return;
            }
            let reason = format!("{:?} (exit code {error_code})", sys::cef_termination_status_t::from(status));
            let id = with_page(|p| {
                p.load = None;
                p.loading = false;
                p.addr_pending = false;
                p.editable = false;
                p.id
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
            if !is_page(&browser) {
                return;
            }
            trace!("on_find_result count {count} active {active_match_ordinal} final {final_update}");
            let (id, index, step) = with_page(|p| {
                if active_match_ordinal > 0 {
                    p.find_index = active_match_ordinal;
                }
                // The count of a new session is final: with matches, step once (below)
                // and report that result; with none, report the zero.
                let step = if final_update != 0 && count > 0 { p.find_step.take() } else { None };
                if final_update != 0 {
                    p.find_step = None;
                }
                (p.id, p.find_index, step)
            });
            if let Some((query, forward, match_case)) = step {
                if let Some(h) = browser.as_ref().and_then(|b| b.host()) {
                    h.find(Some(&CefString::from(query.as_str())), forward, match_case, 1);
                }
                return;
            }
            // Intermediate updates carry partial counts; the shell gets the final one.
            if final_update != 0 {
                emit(Event::FindResult { tab: id, index: if count == 0 { 0 } else { index }, count });
            }
        }
    }
}

/// Cleans `model` and its submenus by the allow-list (newtab::MENU_ALLOW): every command
/// not on it is removed, then the submenus and separators left useless. Collects what was
/// removed as `id:label`.
fn clean_menu(model: &MenuModel, removed: &mut Vec<String>) {
    // Submenus first, so "empty" is known when this level is judged.
    for i in 0..model.count() {
        if let Some(sub) = model.sub_menu_at(i) {
            clean_menu(&sub, removed);
        }
    }
    let items: Vec<newtab::Item> = (0..model.count())
        .map(|i| {
            if let Some(sub) = model.sub_menu_at(i) {
                return newtab::Item::Sub { empty: sub.count() == 0 };
            }
            let t = sys::cef_menu_item_type_t::from(model.type_at(i));
            if t == sys::cef_menu_item_type_t::MENUITEMTYPE_SEPARATOR {
                newtab::Item::Separator
            } else {
                newtab::Item::Command(model.command_id_at(i))
            }
        })
        .collect();
    for i in newtab::menu_removals(&items) {
        match items[i] {
            newtab::Item::Separator => {}
            newtab::Item::Command(id) => {
                removed.push(format!("{id}:{}", CefString::from(&model.label_at(i)).to_string()));
            }
            newtab::Item::Sub { .. } => {
                removed.push(format!("submenu:{}", CefString::from(&model.label_at(i)).to_string()));
            }
        }
        model.remove_at(i);
    }
}

fn menu_items(model: &MenuModel) -> Vec<String> {
    (0..model.count())
        .map(|i| {
            let label = CefString::from(&model.label_at(i)).to_string();
            match model.sub_menu_at(i) {
                Some(sub) => format!("{label}[{}]", menu_items(&sub).join(", ")),
                None => format!("{}:{label}", model.command_id_at(i)),
            }
        })
        .collect()
}

wrap_context_menu_handler! {
    struct HostContextMenuHandler {}

    impl ContextMenuHandler {
        // Allow-list (newtab.rs MENU_ALLOW): what stays acts on the page itself or the
        // clipboard/disk; nothing that opens a tab, a window or Chrome UI.
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            let Some(model) = model else { return };
            let mut removed = Vec::new();
            clean_menu(model, &mut removed);
            log!("context menu: removed [{}]; remaining [{}]", removed.join(", "), menu_items(model).join(", "));
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

/// Closes the page (no beforeunload prompts); on_before_close then quits.
pub fn quit() {
    crate::forget::shutdown(Box::new(|| {}));
    let b = with_page(|p| p.browser.clone());
    match b.and_then(|b| b.host()) {
        Some(h) => h.close_browser(1),
        None => quit_message_loop(),
    }
}

/// Runs one command line from the shell (UI thread).
pub fn run_command(line: &str) {
    let cmd = match protocol::parse_command(line) {
        Ok(c) => c,
        Err(message) => return emit(Event::Error { message }),
    };
    if cmd == Command::Quit {
        return quit();
    }
    // Needs no page (also with --no-window).
    if let Command::ForgetSite { site } = cmd {
        return crate::forget::forget_site(site);
    }
    let Some(browser) = with_page(|p| p.browser.clone()) else {
        return emit(Event::Error {
            message: "no page yet".into(),
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
            with_page(|p| p.stop_requested = true);
            browser.stop_load()
        }
        Command::Find { query, dir, match_case } => {
            let Some(h) = host else { return };
            let again = with_page(|p| {
                let again = p.find_query.as_deref() == Some(query.as_str());
                if !again {
                    p.find_query = Some(query.clone());
                    p.find_index = 0;
                }
                again
            });
            let text = CefString::from(query.as_str());
            let forward = (dir == FindDir::Next) as i32;
            // A new find session counts the matches but activates none (active ordinal 0);
            // on_find_result steps once so the first (or, for prev, last) match is active,
            // as in a browser's find bar, and reports only that.
            with_page(|p| p.find_step = (!again).then(|| (query.clone(), forward, match_case as i32)));
            if !again {
                // After a navigation CEF still holds the old session; the same text would be
                // taken for the same search and never answer (found live by C5). Ending it
                // first is what `find-clear` does, which works.
                h.stop_finding(0);
            }
            h.find(Some(&text), forward, match_case as i32, again as i32);
        }
        Command::FindClear => {
            with_page(|p| {
                p.find_query = None;
                p.find_index = 0;
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
                    log!("zoom factor {factor} -> level {level:.3}");
                }
            }
            Err(message) => emit(Event::Error { message }),
        },
        Command::Quit | Command::ForgetSite { .. } => unreachable!(),
    }
}
