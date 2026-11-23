//! C4-SPIKE experiments (NOT for merge): several browsers in one host, popups as pages,
//! blocked popups and re-running them. Enabled by WEB_HOST_C4=views|windows.
//!   views:   one top-level window; every page is a BrowserView child of it, one visible.
//!   windows: every page gets its own top-level window, announced by `tab-created` first.
//! WEB_HOST_C4_BLOCK=host: the host blocks popups without a user gesture itself (run with
//! --disable-popup-blocking); otherwise Chromium's blocker decides and the host allows the rest.
//! Commands (control socket, raw text): `c4 new <url>`, `c4 show <id>`, `c4 hide-window <id>`,
//! `c4 show-window <id>`, `c4 washidden <id> 0|1`, `c4 blocked`, `c4 popup-open js|devtools|fresh`,
//! `c4 devtools-watch <id>`, `c4 close <id>`, `c4 list`.

use super::*;

pub fn enabled() -> bool {
    std::env::var("WEB_HOST_C4").is_ok()
}

fn windows_mode() -> bool {
    std::env::var("WEB_HOST_C4").map(|v| v == "windows").unwrap_or(false)
}

fn host_blocker() -> bool {
    std::env::var("WEB_HOST_C4_BLOCK").map(|v| v == "host").unwrap_or(false)
}

struct Tab {
    id: i32,
    browser: Browser,
    window: Option<Window>,
    view: Option<BrowserView>,
}

#[derive(Clone, Debug)]
struct Blocked {
    tab: i32,
    frame_id: String,
    frame_url: String,
    url: String,
    name: String,
    popup: bool,
}

#[derive(Default)]
struct St {
    tabs: Vec<Tab>,
    main_window: Option<Window>,
    blocked: Vec<Blocked>,
    /// One-shot: allow the next gesture-less popup from this frame to this URL.
    allow: Option<(String, String)>,
    observers: Vec<Registration>,
}

thread_local! {
    static ST: RefCell<St> = RefCell::new(St::default());
}

fn with<R>(f: impl FnOnce(&mut St) -> R) -> R {
    ST.with(|s| f(&mut s.borrow_mut()))
}


/// The first page announced (page.rs keeps it as the Page).
pub fn register_first(browser: &Browser, window: &Window) {
    if !enabled() {
        return;
    }
    with(|s| {
        s.main_window = Some(window.clone());
        s.tabs.push(Tab {
            id: browser.identifier(),
            browser: browser.clone(),
            window: Some(window.clone()),
            view: browser_view_get_for_browser(Some(&mut browser.clone())),
        });
    });
    log!("c4: tab {} is the first page", browser.identifier());
}

/// Any later browser (popup or `c4 new`): announce it, show its window (windows mode).
pub fn announce_other(browser: &Browser, window: &Window) {
    let id = browser.identifier();
    let known = with(|s| s.tabs.iter().any(|t| t.id == id));
    if known {
        return;
    }
    let own = windows_mode();
    with(|s| {
        s.tabs.push(Tab {
            id,
            browser: browser.clone(),
            window: own.then(|| window.clone()),
            view: browser_view_get_for_browser(Some(&mut browser.clone())),
        })
    });
    emit(Event::TabCreated { tab: id });
    if own && window.is_visible() == 0 {
        window.show();
        log!("c4: window of tab {id} shown");
    }
    log!("c4: tab {id} announced ({})", if own { "own window" } else { "view in the main window" });
}

/// Puts a new BrowserView on screen: its own window, or the main window (shown, others hidden).
fn place(bv: BrowserView) {
    if windows_mode() {
        let mut d = HostWindowDelegate::new(RefCell::new(Some(bv)));
        window_create_top_level(Some(&mut d));
        return;
    }
    let Some(w) = with(|s| s.main_window.clone()) else {
        log!("c4: no main window");
        return;
    };
    // CEF: "Cannot add multiple Chrome style BrowserViews" to one window: swap instead.
    w.remove_all_child_views();
    let mut v = View::from(&bv);
    w.add_child_view(Some(&mut v));
    bv.set_visible(1);
    w.layout();
    bv.request_focus();
    log!("c4: new view placed in the main window ({} children)", w.child_view_count());
}

pub fn on_popup_view(popup: Option<&mut BrowserView>) -> i32 {
    let Some(p) = popup else { return 0 };
    log!("c4: on_popup_browser_view_created (popup browser {:?})", p.browser().map(|b| b.identifier()));
    place(p.clone());
    1
}

#[allow(clippy::too_many_arguments)]
pub fn before_popup(
    browser: Option<&mut Browser>,
    frame: Option<&mut Frame>,
    url: &str,
    name: &str,
    disp: sys::cef_window_open_disposition_t,
    gesture: i32,
    features: Option<&PopupFeatures>,
    no_js: Option<&mut i32>,
) -> i32 {
    let tab = browser.as_ref().map(|b| b.identifier()).unwrap_or(0);
    let (frame_id, frame_url, frame_main) = frame
        .as_ref()
        .map(|f| (CefString::from(&f.identifier()).to_string(), CefString::from(&f.url()).to_string(), f.is_main() != 0))
        .unwrap_or_default();
    let f = features.cloned().unwrap_or_default();
    let nojs = no_js.as_ref().map(|v| **v).unwrap_or(-1);
    log!(
        "c4: on_before_popup tab {tab} frame {frame_id} main={frame_main} ({frame_url}) url={url:?} name={name:?} disposition {disp:?} gesture {gesture} features popup={} x={}/{} y={}/{} w={}/{} h={}/{} no_javascript_access(in)={nojs}",
        f.is_popup, f.x, f.x_set, f.y, f.y_set, f.width, f.width_set, f.height, f.height_set
    );
    if newtab::popup_action(disp) == newtab::PopupAction::Drop {
        log!("c4: dropped by disposition");
        return 1;
    }
    if host_blocker() && gesture == 0 {
        let allowed = with(|s| {
            if s.allow.as_ref() == Some(&(frame_id.clone(), url.to_string())) {
                s.allow = None;
                true
            } else {
                false
            }
        });
        if !allowed {
            let b = Blocked {
                tab,
                frame_id,
                frame_url,
                url: url.to_string(),
                name: name.to_string(),
                popup: f.is_popup != 0,
            };
            log!("c4: popup BLOCKED (no gesture): {b:?}");
            with(|s| s.blocked.push(b));
            return 1;
        }
        log!("c4: one-shot allow used");
    }
    0
}

pub fn closed(browser: &Browser) {
    let id = browser.identifier();
    let t = with(|s| {
        let i = s.tabs.iter().position(|t| t.id == id)?;
        Some(s.tabs.remove(i))
    });
    if t.is_some() {
        emit(Event::TabClosed { tab: id });
        log!("c4: tab {id} closed");
    }
}

pub fn console(browser: &Option<&mut Browser>, m: &str) {
    let id = browser.as_ref().map(|b| b.identifier()).unwrap_or(0);
    log!("c4 console tab {id}: {m}");
}

pub fn title(browser: &Option<&mut Browser>, t: &str) {
    let id = browser.as_ref().map(|b| b.identifier()).unwrap_or(0);
    log!("c4 title tab {id}: {t}");
}

wrap_dev_tools_message_observer! {
    struct WatchObserver {}

    impl DevToolsMessageObserver {
        fn on_dev_tools_event(&self, browser: Option<&mut Browser>, method: Option<&CefString>, params: Option<&[u8]>) {
            let m = method.map(|m| m.to_string()).unwrap_or_default();
            let p = params.map(|r| String::from_utf8_lossy(r).into_owned()).unwrap_or_default();
            let p: String = p.chars().take(400).collect();
            log!("c4 devtools tab {}: {m} {p}", browser.map(|b| b.identifier()).unwrap_or(0));
        }
        fn on_dev_tools_method_result(&self, _browser: Option<&mut Browser>, message_id: i32, success: i32, result: Option<&[u8]>) {
            let r = result.map(|r| String::from_utf8_lossy(r).into_owned()).unwrap_or_default();
            let r: String = r.chars().take(400).collect();
            log!("c4 devtools result {message_id} success {success}: {r}");
        }
    }
}

fn devtools(id: i32, method: &str, params: serde_json::Value) {
    let b = with(|s| s.tabs.iter().find(|t| t.id == id).map(|t| t.browser.clone()));
    let Some(h) = b.and_then(|b| b.host()) else { return log!("c4: no tab {id}") };
    let msg = serde_json::json!({"id": 9000 + rand_id(), "method": method, "params": params}).to_string();
    let r = h.send_dev_tools_message(Some(msg.as_bytes()));
    log!("c4: devtools {method} sent to tab {id}: {r}");
}

fn rand_id() -> i32 {
    use std::sync::atomic::{AtomicI32, Ordering};
    static N: AtomicI32 = AtomicI32::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

pub fn command(rest: &str) {
    let mut it = rest.splitn(2, ' ');
    let op = it.next().unwrap_or("");
    let arg = it.next().unwrap_or("").trim();
    let id: i32 = arg.split(' ').next().and_then(|x| x.parse().ok()).unwrap_or(0);
    match op {
        "list" => with(|s| {
            for t in &s.tabs {
                log!(
                    "c4: tab {} url {} own-window {}",
                    t.id,
                    t.browser.main_frame().map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default(),
                    t.window.is_some()
                );
            }
        }),
        "new" => {
            let url = protocol::normalize_url(arg);
            let mut client = Some(HostClient::new());
            let bv = browser_view_create(
                client.as_mut(),
                Some(&CefString::from(url.as_str())),
                Some(&BrowserSettings::default()),
                None,
                None,
                Some(&mut HostBrowserViewDelegate::new()),
            );
            match bv {
                Some(bv) => place(bv),
                None => log!("c4: browser_view_create failed"),
            }
        }
        "show" => {
            let Some(w) = with(|s| s.main_window.clone()) else { return };
            let target = with(|s| s.tabs.iter().find(|t| t.id == id).and_then(|t| t.view.clone()));
            let Some(target) = target else { return log!("c4: no view for {id}") };
            if std::env::var("WEB_HOST_C4_SHOW").map(|v| v == "visible").unwrap_or(false) {
                // Only for the first page in its own window: set_visible on the one child.
                target.set_visible((arg.split(' ').nth(1) != Some("off")) as i32);
                return log!("c4: tab {id} set_visible");
            }
            w.remove_all_child_views();
            let mut v = View::from(&target);
            w.add_child_view(Some(&mut v));
            target.set_visible(1);
            w.layout();
            target.request_focus();
            log!("c4: tab {id} shown in the main window");
        }
        "hide-window" | "show-window" => {
            let w = with(|s| s.tabs.iter().find(|t| t.id == id).and_then(|t| t.window.clone()));
            let Some(w) = w else { return log!("c4: tab {id} has no own window") };
            if op == "hide-window" {
                w.hide()
            } else {
                w.show()
            }
            log!("c4: {op} {id}: visible now {}", w.is_visible());
        }
        "vis" => {
            let on: i32 = arg.split(' ').nth(1).and_then(|x| x.parse().ok()).unwrap_or(1);
            let v = with(|s| s.tabs.iter().find(|t| t.id == id).and_then(|t| t.view.clone()));
            if let Some(v) = v {
                v.set_visible(on);
                log!("c4: view of {id} set_visible({on})");
            }
        }
        "detach" | "attach" => {
            let t = with(|s| s.tabs.iter().find(|t| t.id == id).map(|t| (t.view.clone(), t.window.clone())));
            if let Some((Some(v), Some(w))) = t {
                if op == "detach" {
                    w.remove_child_view(Some(&mut View::from(&v)));
                } else {
                    w.add_child_view(Some(&mut View::from(&v)));
                    w.layout();
                }
                log!("c4: {op} view of {id}: window children {}", w.child_view_count());
            }
        }
        "eval-gesture" => {
            let js = arg.splitn(2, ' ').nth(1).unwrap_or("");
            devtools(id, "Runtime.evaluate", serde_json::json!({"expression": js, "userGesture": true}));
        }
        "minimize" => {
            let w = with(|s| s.tabs.iter().find(|t| t.id == id).and_then(|t| t.window.clone()));
            if let Some(w) = w {
                w.minimize();
                log!("c4: minimized {id}");
            }
        }
        "washidden" => {
            let hidden: i32 = arg.split(' ').nth(1).and_then(|x| x.parse().ok()).unwrap_or(1);
            let b = with(|s| s.tabs.iter().find(|t| t.id == id).map(|t| t.browser.clone()));
            if let Some(h) = b.and_then(|b| b.host()) {
                h.was_hidden(hidden);
                log!("c4: was_hidden({hidden}) on {id}");
            }
        }
        "close" => {
            let b = with(|s| s.tabs.iter().find(|t| t.id == id).map(|t| t.browser.clone()));
            if let Some(h) = b.and_then(|b| b.host()) {
                h.close_browser(1);
            }
        }
        "blocked" => with(|s| {
            for b in &s.blocked {
                log!("c4: blocked {b:?}");
            }
        }),
        "devtools-watch" => {
            let b = with(|s| s.tabs.iter().find(|t| t.id == id).map(|t| t.browser.clone()));
            if let Some(h) = b.and_then(|b| b.host()) {
                if let Some(r) = h.add_dev_tools_message_observer(Some(&mut WatchObserver::new())) {
                    with(|s| s.observers.push(r));
                }
            }
            devtools(id, "Page.enable", serde_json::json!({}));
            devtools(id, "Runtime.enable", serde_json::json!({}));
            devtools(id, "Log.enable", serde_json::json!({}));
            devtools(id, "Audits.enable", serde_json::json!({}));
            devtools(id, "Target.setDiscoverTargets", serde_json::json!({"discover": true}));
        }
        "popup-open" => {
            let Some(b) = with(|s| s.blocked.last().cloned()) else { return log!("c4: nothing blocked") };
            let how = arg;
            let js = format!(
                "window.__c4w = window.open({}, {}, {}); console.log('c4 reopen result: ' + (window.__c4w ? 'window' : 'null'));",
                serde_json::to_string(&b.url).unwrap(),
                serde_json::to_string(&b.name).unwrap(),
                serde_json::to_string(if b.popup { "popup" } else { "" }).unwrap()
            );
            let browser = with(|s| s.tabs.iter().find(|t| t.id == b.tab).map(|t| t.browser.clone()));
            let Some(browser) = browser else { return log!("c4: tab {} gone", b.tab) };
            match how {
                "js" => {
                    let Some(f) = browser.frame_by_identifier(Some(&CefString::from(b.frame_id.as_str()))) else {
                        return log!("c4: frame {} gone", b.frame_id);
                    };
                    with(|s| s.allow = Some((b.frame_id.clone(), b.url.clone())));
                    f.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
                    log!("c4: popup-open js in frame {} ({}): {js}", b.frame_id, b.frame_url);
                }
                "devtools" => {
                    // Main frame's world only (no context id for subframes here).
                    devtools(b.tab, "Runtime.evaluate", serde_json::json!({"expression": js, "userGesture": true}));
                }
                _ => {
                    log!("c4: popup-open fresh {}", b.url);
                    command(&format!("new {}", b.url));
                }
            }
        }
        _ => log!("c4: unknown command {op}"),
    }
}
