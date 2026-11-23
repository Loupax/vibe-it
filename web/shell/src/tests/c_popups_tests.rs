//! C-POPUPS (KEYS.md "Popups", T93-T98). Each test names its row.

use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::buffers::TabCreated;
use crate::keys::*;
use crate::picker::PickAction;
use crate::popups::{blocked_message, rows, BlockedPopup, NONE_BLOCKED};
use crate::protocol::{Command, Event};

const A: &str = "https://a.test/login";

fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

fn type_text(k: &mut Keys, text: &str) {
    for c in text.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

/// Runs `:line` as typed (Delete hides a ghost) and returns the commands it sent.
fn run(k: &mut Keys, line: &str) -> Vec<Command> {
    k.effects.clear();
    press(k, KeyInput::ch(':'));
    type_text(k, line);
    press(k, KeyInput::sym(Keysym::Delete));
    press(k, KeyInput::sym(Keysym::Return));
    sends(k)
}

fn sends(k: &mut Keys) -> Vec<Command> {
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter_map(|e| match e {
            Effect::Send(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// Buffer 1 on tab 1 showing A.
fn one_buffer() -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = A.into();
    k.effects.clear();
    k
}

fn blocked(popup: u32, url: &str) -> BlockedPopup {
    BlockedPopup {
        popup,
        url: url.into(),
        name: String::new(),
        in_frame: false,
    }
}

fn load_start(tab: u64, kind: &str) -> Event {
    Event::Load {
        tab,
        phase: "start".into(),
        status: 0,
        kind: kind.into(),
        error: None,
    }
}

#[test]
fn c_popups_t93_a_gesture_popup_is_a_new_buffer_shown_at_once() {
    let mut k = one_buffer();
    let n = k.popup_created(7, 1, "popup");
    assert_eq!(n, 2);
    assert_eq!(k.buffers.current, 2);
    assert_eq!(k.buffers.current_tab(), Some(7));
    assert_eq!(k.buffers.alternate, Some(1), "Ctrl-^ goes back to the opener");
    assert_eq!(sends(&mut k), vec![Command::ShowTab { tab: 7 }]);
    assert_eq!(k.message, "buffer 2: popup from buffer 1");
    assert_eq!(k.buffers.get(1).unwrap().url, A, "the opener's state is kept in its buffer");
    assert_eq!(k.key_mode, KeyMode::Normal, "the mode stays");
}

#[test]
fn c_popups_t94_a_target_blank_link_opens_a_buffer_the_same_way() {
    // The engine gives a target=_blank click the same `tab-created` (cause `popup`); only
    // window.opener differs (Chromium's implicit noopener), which the shell never sees.
    let mut k = one_buffer();
    k.open_buffer("https://b.test/");
    k.buffers.tab_created(2, Some(1));
    k.switch_to(1);
    k.effects.clear();
    let n = k.popup_created(9, 1, "popup");
    assert_eq!(n, 3, "after the existing buffers, numbers never reused");
    assert_eq!(k.buffers.current, 3);
    assert_eq!(sends(&mut k), vec![Command::ShowTab { tab: 9 }]);
}

#[test]
fn c_popups_t95_window_close_in_a_popup_kills_only_its_buffer() {
    let mut k = one_buffer();
    k.popup_created(7, 1, "popup");
    k.effects.clear();
    // The host closed the page by itself (tab-closed the shell did not ask for).
    k.tab_gone(7);
    assert_eq!(k.buffers.list.len(), 1);
    assert_eq!(k.buffers.current, 1, "back to the opener");
    let s = sends(&mut k);
    assert_eq!(s, vec![Command::ShowTab { tab: 1 }], "no close-tab for a page already gone");
    assert_eq!(k.message, "buffer 2 closed by the page");
}

#[test]
fn c_popups_t96_a_blocked_popup_is_listed_and_the_bar_says_so() {
    assert_eq!(blocked_message("https://idp.test/x"), "popup blocked: https://idp.test/x");
    assert_eq!(blocked_message(""), "popup blocked (no URL)");
    let mut k = one_buffer();
    assert_eq!(
        k.popup_blocked(1, blocked(1, "https://idp.test/x")).as_deref(),
        Some("popup blocked: https://idp.test/x")
    );
    assert_eq!(k.buffers.current().blocked.len(), 1);
    // A background buffer's blocked popup: listed there, no message.
    k.open_buffer("https://b.test/");
    k.buffers.tab_created(2, Some(1));
    assert_eq!(k.popup_blocked(1, blocked(2, "")), None);
    assert_eq!(k.buffers.get(1).unwrap().blocked.len(), 2);
    assert_eq!(k.popup_blocked(99, blocked(3, "")), None, "a page with no buffer");
    // A same-document navigation keeps the list; another document clears it (shown or not).
    k.page_event(1, &load_start(1, "same-document"));
    assert_eq!(k.buffers.get(1).unwrap().blocked.len(), 2);
    k.page_event(1, &load_start(1, "new"));
    assert!(k.buffers.get(1).unwrap().blocked.is_empty());
    k.popup_blocked(2, blocked(1, "https://c.test/"));
    k.page_event(2, &load_start(2, "reload"));
    assert!(k.buffers.current().blocked.is_empty(), "a reload is another document too");
}

#[test]
fn c_popups_t97_popup_open_opens_the_latest_blocked_popup() {
    let mut k = one_buffer();
    assert!(run(&mut k, "popupOpen").is_empty());
    assert_eq!(k.message, NONE_BLOCKED);
    k.popup_blocked(1, blocked(1, "https://one.test/"));
    k.popup_blocked(1, blocked(2, ""));
    assert_eq!(run(&mut k, "popupOpen"), vec![Command::PopupOpen { tab: 1, popup: 2 }]);
    assert_eq!(k.message, "opening blocked popup: (no URL)");
    assert_eq!(run(&mut k, "popupOpen"), vec![Command::PopupOpen { tab: 1, popup: 1 }]);
    assert_eq!(k.message, "opening blocked popup: https://one.test/");
    assert!(run(&mut k, "popupOpen").is_empty(), "each blocked popup opens once");
    assert_eq!(k.message, NONE_BLOCKED);
    assert!(run(&mut k, "popupOpen 3").is_empty());
    assert_eq!(k.message, "web: usage: :popupOpen");
    // The host's answer: a buffer with the opener kept, or loaded fresh.
    assert_eq!(k.popup_created(5, 1, "popup-open"), 2);
    assert_eq!(k.message, "buffer 2: blocked popup opened from buffer 1");
    k.switch_to(1);
    assert_eq!(k.popup_created(6, 1, "popup-fresh"), 3);
    assert_eq!(
        k.message,
        "buffer 3: blocked popup loaded fresh from buffer 1 (its frame is gone: no window.opener)"
    );
}

#[test]
fn c_popups_t98_popup_search_picks_a_blocked_popup() {
    let mut k = one_buffer();
    assert!(run(&mut k, "popupSearch").is_empty());
    assert_eq!(k.vim_mode, VimMode::Normal, "no rows: no picker");
    assert_eq!(k.message, NONE_BLOCKED);
    k.popup_blocked(1, blocked(1, "https://one.test/"));
    k.popup_blocked(
        1,
        BlockedPopup {
            popup: 2,
            url: "https://idp.test/auth".into(),
            name: "login".into(),
            in_frame: true,
        },
    );
    k.popup_blocked(1, blocked(3, ""));
    let r = rows(&k.buffers.current().blocked);
    assert_eq!(
        r.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
        ["(no URL)", "https://idp.test/auth", "https://one.test/"],
        "newest first"
    );
    assert_eq!(r[1].detail, "name login, in a frame");
    assert_eq!(r[1].action, PickAction::Popup(2));
    run(&mut k, "popupSearch");
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert!(k.pick_log().contains("pick popups [1/3 of 3] popup 3"), "{}", k.pick_log());
    type_text(&mut k, "login");
    assert!(k.pick_log().contains("[1/1 of 3] popup 2"), "{}", k.pick_log());
    k.effects.clear();
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(sends(&mut k), vec![Command::PopupOpen { tab: 1, popup: 2 }]);
    assert_eq!(k.buffers.current().blocked.len(), 2, "taken out of the list");
}

#[test]
fn c_popups_t98_the_ghost_offers_the_popup_commands() {
    assert!(COMMAND_NAMES.contains(&"popupOpen"));
    assert!(COMMAND_NAMES.contains(&"popupSearch"));
    let mut sorted = COMMAND_NAMES.to_vec();
    sorted.sort();
    assert_eq!(sorted, COMMAND_NAMES, "kept sorted");
}
