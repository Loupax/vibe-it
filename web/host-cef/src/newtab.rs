//! C-NOTAB: no way to open a second tab or window from the page (user, 2026-11-23).
//! The decisions, as pure functions (unit-tested without CEF):
//!
//! - `popup_action`: what to do with a popup / new-tab request, by disposition.
//!
//! The context menu is gone altogether since C-NOMENU: see `HostContextMenuHandler`.

use cef::sys::cef_window_open_disposition_t as D;

/// What the host does with a request to open a page somewhere other than this page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupAction {
    /// Cancel the popup and load its URL in this page (script `window.open`,
    /// plain `target=_blank`: PROTOCOL.md "Several pages"; C-POPUPS will open them as pages).
    LoadInPlace,
    /// Cancel it and do nothing: no new page, and this page is not navigated either.
    Drop,
}

/// What to do with a popup / new-tab request, by disposition (Chromium's):
///
/// - CURRENT_TAB, UNKNOWN, SINGLETON_TAB: load here.
/// - NEW_FOREGROUND_TAB: window.open() without features, target=_blank: load here.
///   (Chromium gives a Ctrl+Shift+click the same disposition and no modifiers, so the
///   host cannot tell it from a target=_blank click here: `CTRL_SHIFT_JS` in the
///   renderer cancels that click before it becomes a navigation.)
/// - NEW_POPUP: window.open() with window features: load here.
/// - NEW_BACKGROUND_TAB (Ctrl+click, middle-click), NEW_WINDOW (Shift+click),
///   OFF_THE_RECORD, SAVE_TO_DISK, SWITCH_TO_TAB, IGNORE_ACTION, picture-in-picture,
///   split view: drop.
pub fn popup_action(disposition: D) -> PopupAction {
    match disposition {
        D::CEF_WOD_UNKNOWN | D::CEF_WOD_CURRENT_TAB | D::CEF_WOD_SINGLETON_TAB | D::CEF_WOD_NEW_POPUP | D::CEF_WOD_NEW_FOREGROUND_TAB => {
            PopupAction::LoadInPlace
        }
        _ => PopupAction::Drop,
    }
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
    fn plain_loads_in_place() {
        for d in [
            D::CEF_WOD_UNKNOWN,
            D::CEF_WOD_CURRENT_TAB,
            D::CEF_WOD_SINGLETON_TAB,
            D::CEF_WOD_NEW_POPUP,
            D::CEF_WOD_NEW_FOREGROUND_TAB,
        ] {
            assert_eq!(popup_action(d), PopupAction::LoadInPlace, "{d:?}");
        }
    }

    #[test]
    fn user_new_tab_gestures_drop() {
        // Ctrl+click, middle-click: background tab. Shift+click: new window.
        assert_eq!(popup_action(D::CEF_WOD_NEW_BACKGROUND_TAB), PopupAction::Drop);
        assert_eq!(popup_action(D::CEF_WOD_NEW_WINDOW), PopupAction::Drop);
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
            assert_eq!(popup_action(d), PopupAction::Drop, "{d:?}");
        }
    }

    #[test]
    fn ctrl_shift_js_is_one_listener() {
        assert!(CTRL_SHIFT_JS.contains("ctrlKey&&e.shiftKey"));
        assert!(CTRL_SHIFT_JS.contains("preventDefault"));
        assert!(!CTRL_SHIFT_JS.contains("stopPropagation"));
    }
}
