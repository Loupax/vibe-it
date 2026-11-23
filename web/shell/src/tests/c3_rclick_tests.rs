//! T57: popups return to the previous mode (C3-RCLICK). Each test names its row.

use crate::keys::*;

fn normal() -> Keys {
    let mut k = Keys::new(None, "file:///help.html".into(), None);
    k.set_key_mode(KeyMode::Normal);
    k
}

#[test]
fn keys_t57_a_clicked_popup_returns_to_the_previous_mode() {
    let mut k = normal();
    k.popup_opened(true);
    assert_eq!(k.key_mode, KeyMode::Input, "T29: INPUT while the popup is open");
    k.popup_closed();
    assert_eq!(k.key_mode, KeyMode::Normal, "T57: back after it closes");
}

#[test]
fn keys_t57_nested_popups_are_one_session() {
    let mut k = normal();
    k.popup_opened(true);
    k.popup_opened(false); // a submenu: no click of its own
    k.popup_closed();
    assert_eq!(k.key_mode, KeyMode::Input, "the submenu closing is not the end");
    k.popup_closed();
    assert_eq!(k.key_mode, KeyMode::Normal);
}

#[test]
fn keys_t57_a_focused_field_stays_input() {
    // The editable-focused event after the popup opened.
    let mut k = normal();
    k.popup_opened(true);
    k.user_focused_control();
    k.popup_closed();
    assert_eq!(k.key_mode, KeyMode::Input);
    // Before it: already INPUT, nothing to return to.
    let mut k = normal();
    k.user_focused_control();
    k.popup_opened(true);
    k.popup_closed();
    assert_eq!(k.key_mode, KeyMode::Input);
}

#[test]
fn keys_t57_a_popup_from_input_stays_input() {
    let mut k = normal();
    k.set_key_mode(KeyMode::Input);
    k.popup_opened(true);
    k.popup_closed();
    assert_eq!(k.key_mode, KeyMode::Input);
}

#[test]
fn keys_t57_a_popup_without_a_click_changes_nothing() {
    let mut k = normal();
    k.popup_opened(false);
    assert_eq!(k.key_mode, KeyMode::Normal);
    k.popup_closed();
    assert_eq!(k.key_mode, KeyMode::Normal);
}

#[test]
fn keys_t57_an_open_line_is_cancelled_and_the_return_is_normal() {
    let mut k = normal();
    k.vim_mode = VimMode::Command;
    k.popup_opened(true);
    assert_eq!(k.vim_mode, VimMode::Normal);
    k.popup_closed();
    assert_eq!((k.key_mode, k.vim_mode), (KeyMode::Normal, VimMode::Normal));
}
