//! Popups and new-tab requests (C-NOTAB, C-POPUPS; PROTOCOL.md "Popups"). The decisions, as pure
//! functions (unit-tested without CEF):
//!
//! - `popup_action`: what `OnBeforePopup` does with a popup (`window.open`, `target=_blank`), by
//!   disposition and user gesture. The host is the popup blocker (it runs Chromium with
//!   `--disable-popup-blocking`, so every popup reaches it, C4-SPIKE).
//! - `open_url_action`: what `OnOpenURLFromTab` does (a navigation the engine wants in another tab
//!   outside the popup path: real Ctrl/middle/Shift clicks on links).
//!
//! The context menu is gone altogether since C-NOMENU: see `HostContextMenuHandler`.

use cef::sys::cef_window_open_disposition_t as D;

/// What the host does with a popup (`OnBeforePopup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupAction {
    /// Let it through: a new page (`tab-created` with `opener`), `window.opener` kept.
    Open,
    /// Cancel it (`window.open` returns null) and report it: `popup-blocked`.
    Block,
    /// Cancel it and load its URL in the page that asked (dispositions that mean "this tab").
    LoadInPlace,
    /// Cancel it and do nothing: no new page, and this page is not navigated either.
    Drop,
}

/// What to do with a popup, by disposition (Chromium's) and whether the page had a user gesture
/// (`allowed`: the host's one-shot allow from `popup-open` matches it):
///
/// - NEW_FOREGROUND_TAB (window.open() without features, target=_blank links and forms) and
///   NEW_POPUP (window.open() with window features): a new page with a gesture (or the allow),
///   else blocked. Chromium gives a Ctrl+Shift+click the same disposition and no modifiers, so the
///   host cannot tell it from a target=_blank click here: `CTRL_SHIFT_JS` in the renderer cancels
///   that click before it becomes a navigation.
/// - CURRENT_TAB, UNKNOWN, SINGLETON_TAB: load here.
/// - NEW_BACKGROUND_TAB (Ctrl+click, middle-click), NEW_WINDOW (Shift+click), OFF_THE_RECORD,
///   SAVE_TO_DISK, SWITCH_TO_TAB, IGNORE_ACTION, picture-in-picture, split view: drop (C-NOTAB,
///   DECISIONS.md: "Ctrl/middle-click on links still open nothing").
pub fn popup_action(disposition: D, user_gesture: bool, allowed: bool) -> PopupAction {
    match disposition {
        D::CEF_WOD_NEW_POPUP | D::CEF_WOD_NEW_FOREGROUND_TAB => {
            if user_gesture || allowed {
                PopupAction::Open
            } else {
                PopupAction::Block
            }
        }
        D::CEF_WOD_UNKNOWN | D::CEF_WOD_CURRENT_TAB | D::CEF_WOD_SINGLETON_TAB => PopupAction::LoadInPlace,
        _ => PopupAction::Drop,
    }
}

/// What `OnOpenURLFromTab` does (always cancelled there): load in this page for "this tab" and
/// foreground-tab dispositions, drop the rest (Ctrl/middle/Shift click: C-NOTAB). Only
/// `LoadInPlace` or `Drop`.
pub fn open_url_action(disposition: D) -> PopupAction {
    match disposition {
        D::CEF_WOD_UNKNOWN | D::CEF_WOD_CURRENT_TAB | D::CEF_WOD_SINGLETON_TAB | D::CEF_WOD_NEW_POPUP | D::CEF_WOD_NEW_FOREGROUND_TAB => {
            PopupAction::LoadInPlace
        }
        _ => PopupAction::Drop,
    }
}

/// The window features of a popup as `window.open` takes them (for `popup-blocked` and the re-run
/// of `popup-open`): `popup` when the page asked for a popup window, then the size and position
/// it gave. Sizes are ignored when the popup opens (the shell sizes every page), but the re-run
/// passes them so `window.open` sees the same kind of request.
pub fn features_text(is_popup: bool, x: Option<i32>, y: Option<i32>, width: Option<i32>, height: Option<i32>) -> String {
    let mut parts = Vec::new();
    if is_popup {
        parts.push("popup".to_string());
    }
    for (name, v) in [("left", x), ("top", y), ("width", width), ("height", height)] {
        if let Some(v) = v {
            parts.push(format!("{name}={v}"));
        }
    }
    parts.join(",")
}

/// The script `popup-open` runs in the frame that asked: the page's own `window.open` with the
/// recorded URL, name and features (JSON strings, so any text is safe in a script).
pub fn reopen_js(url: &str, name: &str, features: &str) -> String {
    let s = |t: &str| {
        serde_json::to_string(t)
            .expect("a string serializes")
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
    };
    format!("window.open({}, {}, {});", s(url), s(name), s(features))
}

/// Whether the URL of a popup matches the one recorded for the one-shot allow: equal, or both
/// "no URL" (`window.open()` reaches the host as `""` or `about:blank`).
pub fn same_popup_url(recorded: &str, now: &str) -> bool {
    let blank = |u: &str| u.is_empty() || u == "about:blank";
    recorded == now || (blank(recorded) && blank(now))
}

/// Run in every page's main world by the renderer (renderer.rs): a click with Ctrl and
/// Shift held on a link is cancelled (default action only; the page's handlers still
/// run), since Chromium would open it in a new foreground tab, which the host cannot
/// tell from a target=_blank click.
pub const CTRL_SHIFT_JS: &str = "addEventListener('click',function(e){if(e.ctrlKey&&e.shiftKey&&e.target&&e.target.closest&&e.target.closest('a[href],area[href]'))e.preventDefault();},true);";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_popups_with_a_gesture_open_a_page() {
        for d in [D::CEF_WOD_NEW_POPUP, D::CEF_WOD_NEW_FOREGROUND_TAB] {
            assert_eq!(popup_action(d, true, false), PopupAction::Open, "{d:?}");
        }
    }

    #[test]
    fn script_popups_without_a_gesture_are_blocked_unless_allowed() {
        for d in [D::CEF_WOD_NEW_POPUP, D::CEF_WOD_NEW_FOREGROUND_TAB] {
            assert_eq!(popup_action(d, false, false), PopupAction::Block, "{d:?}");
            assert_eq!(popup_action(d, false, true), PopupAction::Open, "{d:?} with popup-open's allow");
        }
    }

    #[test]
    fn this_tab_loads_in_place() {
        for d in [D::CEF_WOD_UNKNOWN, D::CEF_WOD_CURRENT_TAB, D::CEF_WOD_SINGLETON_TAB] {
            assert_eq!(popup_action(d, true, false), PopupAction::LoadInPlace, "{d:?}");
            assert_eq!(popup_action(d, false, false), PopupAction::LoadInPlace, "{d:?}");
        }
    }

    #[test]
    fn user_new_tab_gestures_drop() {
        // Ctrl+click, middle-click: background tab. Shift+click: new window. With a gesture too.
        for g in [true, false] {
            assert_eq!(popup_action(D::CEF_WOD_NEW_BACKGROUND_TAB, g, false), PopupAction::Drop);
            assert_eq!(popup_action(D::CEF_WOD_NEW_WINDOW, g, true), PopupAction::Drop);
            assert_eq!(open_url_action(D::CEF_WOD_NEW_BACKGROUND_TAB), PopupAction::Drop);
            assert_eq!(open_url_action(D::CEF_WOD_NEW_WINDOW), PopupAction::Drop);
        }
    }

    #[test]
    fn everything_else_drops() {
        for d in [
            D::CEF_WOD_OFF_THE_RECORD,
            D::CEF_WOD_SAVE_TO_DISK,
            D::CEF_WOD_SWITCH_TO_TAB,
            D::CEF_WOD_IGNORE_ACTION,
            D::CEF_WOD_NEW_PICTURE_IN_PICTURE,
            D::CEF_WOD_NEW_SPLIT_VIEW,
        ] {
            assert_eq!(popup_action(d, true, true), PopupAction::Drop, "{d:?}");
            assert_eq!(open_url_action(d), PopupAction::Drop, "{d:?}");
        }
    }

    #[test]
    fn open_url_from_tab_keeps_loading_in_place() {
        for d in [
            D::CEF_WOD_UNKNOWN,
            D::CEF_WOD_CURRENT_TAB,
            D::CEF_WOD_SINGLETON_TAB,
            D::CEF_WOD_NEW_POPUP,
            D::CEF_WOD_NEW_FOREGROUND_TAB,
        ] {
            assert_eq!(open_url_action(d), PopupAction::LoadInPlace, "{d:?}");
        }
    }

    #[test]
    fn features_are_rebuilt() {
        assert_eq!(features_text(false, None, None, None, None), "");
        assert_eq!(features_text(true, None, None, Some(400), Some(300)), "popup,width=400,height=300");
        assert_eq!(features_text(false, Some(5), Some(6), None, None), "left=5,top=6");
    }

    #[test]
    fn reopen_js_quotes_everything() {
        assert_eq!(
            reopen_js("https://a.test/x?\"y", "kid", "popup"),
            r#"window.open("https://a.test/x?\"y", "kid", "popup");"#
        );
        let js = reopen_js("a\u{2028}b", "</script>", "");
        assert!(!js.contains('\u{2028}'));
        assert!(js.contains(r#""</script>""#));
    }

    #[test]
    fn no_url_matches_about_blank() {
        assert!(same_popup_url("https://a.test/", "https://a.test/"));
        assert!(same_popup_url("", "about:blank"));
        assert!(same_popup_url("about:blank", ""));
        assert!(!same_popup_url("https://a.test/", "https://b.test/"));
        assert!(!same_popup_url("", "https://b.test/"));
    }

    #[test]
    fn ctrl_shift_js_is_one_listener() {
        assert!(CTRL_SHIFT_JS.contains("ctrlKey&&e.shiftKey"));
        assert!(CTRL_SHIFT_JS.contains("preventDefault"));
        assert!(!CTRL_SHIFT_JS.contains("stopPropagation"));
    }
}
