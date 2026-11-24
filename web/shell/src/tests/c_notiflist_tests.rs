//! C-NOTIFLIST (KEYS.md "Notifications", T112-T114). Each test names its row.

use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::buffers::TabCreated;
use crate::config::NOTIFY_LIST_MAX;
use crate::keys::*;
use crate::notify::{rows, Note, NONE_YET};
use crate::picker::PickAction;
use crate::protocol::Command;

fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

fn type_text(k: &mut Keys, text: &str) {
    for c in text.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

fn run(k: &mut Keys, line: &str) {
    press(k, KeyInput::ch(':'));
    type_text(k, line);
    press(k, KeyInput::sym(Keysym::Delete));
    press(k, KeyInput::sym(Keysym::Return));
}

/// Buffer 1 (tab 1, chat) and buffer 2 (tab 2) shown.
fn two_buffers() -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = "https://chat.example.com/room".into();
    k.title = "Chat".into();
    k.open_buffer("https://b.test/");
    assert_eq!(k.buffers.tab_created(2, Some(1)), TabCreated::Buffer(2));
    k.effects.clear();
    k
}

fn note(title: &str, body: &str) -> Note {
    Note {
        title: title.into(),
        body: body.into(),
        tag: String::new(),
        origin: "https://chat.example.com".into(),
    }
}

#[test]
fn c_notiflist_t112_lists_newest_first_with_unread_marked_and_filters() {
    let mut k = two_buffers();
    run(&mut k, "notifications");
    assert_eq!(k.vim_mode, VimMode::Normal, "nothing yet: no picker");
    assert_eq!(k.message, NONE_YET);
    k.notified(1, note("First", "hello"));
    k.notified(2, note("Shown", "on screen"));
    k.notified(1, note("Second", "lunch?"));
    let r = rows(&k.notices, &k.buffers);
    assert_eq!(r.len(), 3);
    assert_eq!(r[0].label, "1! Second");
    assert_eq!(r[0].detail, "lunch?  chat.example.com");
    assert_eq!(r[1].label, "2 Shown", "the shown buffer's is read at once");
    assert_eq!(r[2].label, "1! First");
    run(&mut k, "notifications");
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert!(k.pick_log().contains("pick notifications [1/3 of 3]"), "{}", k.pick_log());
    type_text(&mut k, "lunch");
    assert!(k.pick_log().contains("[1/1 of 3]"), "{}", k.pick_log());
}

#[test]
fn c_notiflist_t113_enter_shows_the_buffer_and_marks_it_read() {
    let mut k = two_buffers();
    k.notified(1, note("Ping", "x"));
    k.notified(1, note("Pong", "y"));
    run(&mut k, "notifications");
    type_text(&mut k, "ping");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(k.buffers.current, 1);
    assert_eq!(k.unread_buffers(), 0, "showing a buffer reads all of its notifications (T108)");
    assert!(k.notices.list.iter().all(|n| !n.unread));
    assert!(std::mem::take(&mut k.effects)
        .iter()
        .any(|e| matches!(e, Effect::Send(Command::ShowTab { tab: 1 }))));
}

#[test]
fn c_notiflist_t113_the_notice_is_read_even_when_its_buffer_is_shown() {
    let mut k = two_buffers();
    k.notified(1, note("A", ""));
    assert!(k.notices.list[0].unread);
    let id = k.notices.list[0].id;
    k.notice_open(id);
    assert_eq!(k.buffers.current, 1);
    assert!(!k.notices.list[0].unread);
}

#[test]
fn c_notiflist_t114_a_closed_buffer_is_reopened_by_its_url() {
    let mut k = two_buffers();
    k.notified(1, note("Gone", "b"));
    k.buffer_kill(&["1"]);
    assert!(k.buffers.get(1).is_none());
    let r = rows(&k.notices, &k.buffers);
    assert_eq!(r[0].detail, "b  chat.example.com  (closed)");
    assert!(matches!(r[0].action, PickAction::Notice(_)));
    k.effects.clear();
    run(&mut k, "notifications");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(k.buffers.current, 3);
    assert_eq!(k.buffers.current().url, "https://chat.example.com/room");
    assert_eq!(k.message, "buffer 1 is closed: reopened https://chat.example.com/room as buffer 3");
    assert!(!k.notices.list[0].unread);
}

#[test]
fn c_notiflist_t114_a_closed_buffer_with_no_url_only_says_so() {
    let mut k = two_buffers();
    k.notices.push(7, String::new(), Note::default(), true);
    let id = k.notices.list[0].id;
    k.notice_open(id);
    assert_eq!(k.buffers.current, 2, "nothing opened");
    assert_eq!(k.message, "web: buffer 7 is closed and the notification has no page URL");
    assert!(!k.notices.list[0].unread);
}

#[test]
fn c_notiflist_t112_the_list_is_capped_and_the_command_is_offered() {
    let mut k = two_buffers();
    for i in 0..NOTIFY_LIST_MAX + 5 {
        k.notified(1, note(&i.to_string(), ""));
    }
    assert_eq!(k.notices.list.len(), NOTIFY_LIST_MAX);
    assert_eq!(k.notices.list[0].note.title, "5", "the oldest went");
    assert!(COMMAND_NAMES.contains(&"notifications"));
    let mut sorted = COMMAND_NAMES.to_vec();
    sorted.sort();
    assert_eq!(sorted, COMMAND_NAMES, "kept sorted");
}
