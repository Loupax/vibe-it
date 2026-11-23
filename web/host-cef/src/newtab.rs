//! C-NOTAB: no way to open a second tab or window from the page (user, 2026-11-23).
//! The decisions, as pure functions (unit-tested without CEF):
//!
//! - `popup_action`: what to do with a popup / new-tab request, by disposition.
//! - `MENU_REMOVE`: the Chrome-style context menu commands that open a tab, a window
//!   or another UI, removed in `on_before_context_menu`.

use cef::sys::cef_window_open_disposition_t as D;

/// What the host does with a request to open a page somewhere other than this page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupAction {
    /// Cancel the popup and load its URL in this page (script `window.open`,
    /// plain `target=_blank`: PROTOCOL.md "One page per host").
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

/// Context-menu command ids (Chrome style: `IDC_*` of cef_command_ids.h, plus CEF's
/// own `MENU_ID_*`) whose item opens a new tab, window or profile, a search in one,
/// the view-source tab or DevTools. Removed from every menu, submenus included.
pub const MENU_REMOVE: &[(i32, &str)] = &[
    (34000, "IDC_NEW_WINDOW"),
    (34001, "IDC_NEW_INCOGNITO_WINDOW"),
    (34014, "IDC_NEW_TAB"),
    (34056, "IDC_MOVE_TAB_TO_NEW_WINDOW"),
    (34057, "IDC_NEW_SPLIT_TAB"),
    (35002, "IDC_VIEW_SOURCE"),
    (132, "MENU_ID_VIEW_SOURCE"),
    (35024, "IDC_NEW_TAB_TO_RIGHT"),
    (35356, "IDC_OPEN_GUEST_PROFILE"),
    (40004, "IDC_DEV_TOOLS"),
    (40005, "IDC_DEV_TOOLS_CONSOLE"),
    (40023, "IDC_DEV_TOOLS_INSPECT"),
    (40237, "IDC_DEV_TOOLS_TOGGLE"),
    (40282, "IDC_SHOW_GOOGLE_LENS_SHORTCUT"),
    (40301, "IDC_CONTENT_CONTEXT_INSPECTELEMENT_WITH_DEVTOOLS"),
    (50100, "IDC_CONTENT_CONTEXT_OPENLINKNEWTAB"),
    (50101, "IDC_CONTENT_CONTEXT_OPENLINKNEWWINDOW"),
    (50102, "IDC_CONTENT_CONTEXT_OPENLINKOFFTHERECORD"),
    (50108, "IDC_CONTENT_CONTEXT_OPENLINKINPROFILE"),
    (50109, "IDC_CONTENT_CONTEXT_OPENLINKBOOKMARKAPP"),
    (50111, "IDC_CONTENT_CONTEXT_OPENLINKSPLITVIEW"),
    (50113, "IDC_CONTENT_CONTEXT_OPENLINK_ISOLATED"),
    (50123, "IDC_CONTENT_CONTEXT_OPENIMAGENEWTAB"),
    (50124, "IDC_CONTENT_CONTEXT_SEARCHWEBFORIMAGE"),
    (50125, "IDC_CONTENT_CONTEXT_OPEN_ORIGINAL_IMAGE_NEW_TAB"),
    (50127, "IDC_CONTENT_CONTEXT_SEARCHLENSFORIMAGE"),
    (50134, "IDC_CONTENT_CONTEXT_SEARCHLENSFORVIDEOFRAME"),
    (50135, "IDC_CONTENT_CONTEXT_SEARCHWEBFORVIDEOFRAME"),
    (50136, "IDC_CONTENT_CONTEXT_OPENAVNEWTAB"),
    (50162, "IDC_CONTENT_CONTEXT_INSPECTELEMENT"),
    (50171, "IDC_CONTENT_CONTEXT_INSPECTBACKGROUNDPAGE"),
    (50174, "IDC_CONTENT_CONTEXT_LENS_REGION_SEARCH"),
    (50191, "IDC_CONTENT_CONTEXT_SEARCHWEBFOR"),
    (50192, "IDC_CONTENT_CONTEXT_SEARCHWEBFORNEWTAB"),
    (50193, "IDC_CONTENT_CONTEXT_LENS_OVERLAY"),
    (57316, "IDC_CONTENT_CONTEXT_OPENLINKWITH"),
];

pub fn menu_name_to_remove(command_id: i32) -> Option<&'static str> {
    MENU_REMOVE.iter().find(|(id, _)| *id == command_id).map(|(_, n)| *n)
}

/// One item of a menu as the cleanup sees it: a separator, or anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Separator,
    Other,
}

/// Indexes (descending, so they can be removed in order) of the separators that are
/// useless once items were removed: leading, trailing and doubled ones.
pub fn useless_separators(items: &[Item]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut prev_sep = true; // a separator at the start is useless
    for (i, it) in items.iter().enumerate() {
        match it {
            Item::Separator if prev_sep => out.push(i),
            Item::Separator => prev_sep = true,
            Item::Other => prev_sep = false,
        }
    }
    // A trailing separator: the last item not already marked.
    if let Some(last) = (0..items.len()).rev().find(|i| !out.contains(i)) {
        if items[last] == Item::Separator {
            out.push(last);
        }
    }
    out.sort_unstable_by(|a, b| b.cmp(a));
    out
}

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

    #[test]
    fn menu_remove_list() {
        assert_eq!(menu_name_to_remove(50100), Some("IDC_CONTENT_CONTEXT_OPENLINKNEWTAB"));
        assert_eq!(menu_name_to_remove(50192), Some("IDC_CONTENT_CONTEXT_SEARCHWEBFORNEWTAB"));
        assert_eq!(menu_name_to_remove(132), Some("MENU_ID_VIEW_SOURCE"));
        // Back, forward, reload, copy, paste, select all, print stay.
        for id in [100, 101, 102, 113, 114, 117, 131] {
            assert_eq!(menu_name_to_remove(id), None, "{id}");
        }
        let mut ids: Vec<i32> = MENU_REMOVE.iter().map(|(i, _)| *i).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MENU_REMOVE.len(), "duplicate id");
    }

    #[test]
    fn separators_cleanup() {
        use Item::*;
        assert_eq!(useless_separators(&[Other, Separator, Other]), Vec::<usize>::new());
        assert_eq!(useless_separators(&[Separator, Other]), vec![0]);
        assert_eq!(useless_separators(&[Other, Separator]), vec![1]);
        assert_eq!(useless_separators(&[Other, Separator, Separator, Other]), vec![2]);
        assert_eq!(useless_separators(&[Separator, Separator]), vec![1, 0]);
        assert_eq!(useless_separators(&[]), Vec::<usize>::new());
    }
}
