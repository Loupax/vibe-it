//! C-NOTAB: no way to open a second tab or window from the page (user, 2026-11-23).
//! The decisions, as pure functions (unit-tested without CEF):
//!
//! - `popup_action`: what to do with a popup / new-tab request, by disposition.
//! - `MENU_ALLOW`: the only Chrome-style context menu commands that stay (allow-list),
//!   applied in `on_before_context_menu`; every other item is removed.

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

/// Context-menu allow-list (C-NOTAB-ALLOW): the only commands that stay in a menu. Chrome
/// style ids (`IDC_*` of cef_command_ids.h) and CEF's own `MENU_ID_*`. Everything else is
/// removed, so a new Chrome item that opens a tab, a window or Chrome UI (Print, Cast, QR
/// code, ...) cannot slip in. Only items that act on the page itself or on the
/// clipboard/disk stay.
pub const MENU_ALLOW: &[(i32, &str)] = &[
    (100, "MENU_ID_BACK"),
    (101, "MENU_ID_FORWARD"),
    (102, "MENU_ID_RELOAD"),
    (110, "MENU_ID_UNDO"),
    (111, "MENU_ID_REDO"),
    (112, "MENU_ID_CUT"),
    (113, "MENU_ID_COPY"),
    (114, "MENU_ID_PASTE"),
    (115, "MENU_ID_PASTE_MATCH_STYLE"),
    (116, "MENU_ID_DELETE"),
    (117, "MENU_ID_SELECT_ALL"),
    (200, "MENU_ID_SPELLCHECK_SUGGESTION_0"),
    (201, "MENU_ID_SPELLCHECK_SUGGESTION_1"),
    (202, "MENU_ID_SPELLCHECK_SUGGESTION_2"),
    (203, "MENU_ID_SPELLCHECK_SUGGESTION_3"),
    (204, "MENU_ID_SPELLCHECK_SUGGESTION_4"),
    (33000, "IDC_BACK"),
    (33001, "IDC_FORWARD"),
    (33002, "IDC_RELOAD"),
    (35004, "IDC_SAVE_PAGE"),
    (50103, "IDC_CONTENT_CONTEXT_SAVELINKAS"),
    (50104, "IDC_CONTENT_CONTEXT_COPYLINKLOCATION"),
    (50107, "IDC_CONTENT_CONTEXT_COPYLINKTEXT"),
    (50120, "IDC_CONTENT_CONTEXT_SAVEIMAGEAS"),
    (50121, "IDC_CONTENT_CONTEXT_COPYIMAGELOCATION"),
    (50122, "IDC_CONTENT_CONTEXT_COPYIMAGE"),
    (50150, "IDC_CONTENT_CONTEXT_COPY"),
    (50151, "IDC_CONTENT_CONTEXT_CUT"),
    (50152, "IDC_CONTENT_CONTEXT_PASTE"),
    (50154, "IDC_CONTENT_CONTEXT_UNDO"),
    (50155, "IDC_CONTENT_CONTEXT_REDO"),
    (50156, "IDC_CONTENT_CONTEXT_SELECTALL"),
    (50157, "IDC_CONTENT_CONTEXT_PASTE_AND_MATCH_STYLE"),
    (50158, "IDC_CONTENT_CONTEXT_COPYLINKTOTEXT"),
    (50167, "IDC_CONTENT_CONTEXT_SPELLING_SUGGESTION"),
];

pub fn menu_allows(command_id: i32) -> bool {
    MENU_ALLOW.iter().any(|(id, _)| *id == command_id)
}

/// One item of a menu as the cleanup sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Separator,
    /// A submenu; `empty` when nothing is left in it after its own cleanup.
    Sub {
        empty: bool,
    },
    /// A command item with its command id.
    Command(i32),
}

/// Indexes (descending, so they can be removed in order) to remove from one menu level:
/// commands not on the allow-list, submenus left empty, then the separators that are
/// useless once those are gone (leading, trailing and doubled ones). A submenu with
/// content stays (its own items were cleaned by the caller first).
pub fn menu_removals(items: &[Item]) -> Vec<usize> {
    let mut gone: Vec<bool> = items
        .iter()
        .map(|it| match it {
            Item::Separator => false,
            Item::Sub { empty } => *empty,
            Item::Command(id) => !menu_allows(*id),
        })
        .collect();
    let mut prev_sep = true; // a separator at the start is useless
    for (i, it) in items.iter().enumerate() {
        if gone[i] {
            continue;
        }
        match it {
            Item::Separator if prev_sep => gone[i] = true,
            Item::Separator => prev_sep = true,
            _ => prev_sep = false,
        }
    }
    // A trailing separator: the last item left.
    if let Some(last) = (0..items.len()).rev().find(|i| !gone[*i]) {
        if items[last] == Item::Separator {
            gone[last] = true;
        }
    }
    (0..items.len()).rev().filter(|i| gone[*i]).collect()
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
    fn menu_allow_list() {
        // Back, forward, reload, copy, paste, select all, save as, spellcheck stay.
        for id in [
            100, 101, 102, 113, 114, 117, 33000, 33001, 33002, 35004, 50103, 50120, 50122, 50150, 50152, 50157, 50158,
        ] {
            assert!(menu_allows(id), "{id}");
        }
        // Print, Cast, QR code, new tab/window, view source, inspect, search, Lens: out.
        for id in [131, 132, 130, 35003, 34014, 34000, 35002, 40004, 50100, 50162, 50191, 50193, 50124] {
            assert!(!menu_allows(id), "{id}");
        }
        // Unknown ids (any future Chrome item) are out.
        for id in [0, -1, 1, 99, 99999, 123456, 220, 26500, i32::MAX] {
            assert!(!menu_allows(id), "{id}");
        }
        let mut ids: Vec<i32> = MENU_ALLOW.iter().map(|(i, _)| *i).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MENU_ALLOW.len(), "duplicate id");
    }

    #[test]
    fn removals_filter_commands() {
        use Item::*;
        // Back kept, Print removed, unknown removed; descending indexes.
        assert_eq!(menu_removals(&[Command(100), Command(131), Command(99999)]), vec![2, 1]);
        assert_eq!(menu_removals(&[Command(100), Command(113)]), Vec::<usize>::new());
    }

    #[test]
    fn removals_clean_separators() {
        use Item::*;
        // A separator between kept items stays; ones left leading, trailing or doubled go.
        assert_eq!(menu_removals(&[Command(100), Separator, Command(113)]), Vec::<usize>::new());
        assert_eq!(menu_removals(&[Command(131), Separator, Command(113)]), vec![1, 0]);
        assert_eq!(menu_removals(&[Command(113), Separator, Command(131)]), vec![2, 1]);
        assert_eq!(
            menu_removals(&[Command(100), Separator, Command(131), Separator, Command(113)]),
            vec![3, 2]
        );
        assert_eq!(menu_removals(&[Separator, Command(100)]), vec![0]);
        assert_eq!(menu_removals(&[Separator, Separator]), vec![1, 0]);
        assert_eq!(menu_removals(&[]), Vec::<usize>::new());
    }

    #[test]
    fn removals_submenus() {
        use Item::*;
        // An empty submenu goes (and the separator it leaves behind); one with content stays.
        assert_eq!(menu_removals(&[Command(113), Separator, Sub { empty: true }]), vec![2, 1]);
        assert_eq!(menu_removals(&[Command(113), Separator, Sub { empty: false }]), Vec::<usize>::new());
    }
}
