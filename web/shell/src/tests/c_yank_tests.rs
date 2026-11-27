//! C-YANK: KEYS.md T54 (`y`), T56 (`g` `y`), the `:yank` rows and the host's `selection` answer,
//! and T118 with the user mapping `yy` that replaced the built-in URL copy (removed 2026-11-27).

use std::time::{Duration, Instant};

use smithay::input::keyboard::Keysym;

use super::c6_tests::{clip, fake, keys_with, tap, type_str, unreachable_clipboard, FakeClipboard, URL};
use super::c_script_map_tests as map;
use crate::config::{evdev, MAP_TIMEOUT_MS};
use crate::keys::*;
use crate::protocol::Command;
use crate::registers::{CLIPBOARD_UNREACHABLE, COPIED, NOTHING_SELECTED};

/// The user's replacement for the built-in `yy` (SCRIPTING.md "web.map").
const YY_MAP: &str = r#"web.map("n", "yy", ":let @+ = @%<CR>");"#;

/// `keys_with(cb)` showing buffer 1 on page 1.
fn on_page(cb: &FakeClipboard) -> Keys {
    let mut k = keys_with(cb);
    let _ = k.buffers.tab_created(1, None);
    k
}

fn sends(k: &mut Keys) -> Vec<Effect> {
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter(|e| matches!(e, Effect::Send(_) | Effect::SendTo(..)))
        .collect()
}

fn sel(tab: u64, req: u64) -> Effect {
    Effect::SendTo(tab, Command::Selection { req })
}

fn enter(k: &mut Keys) {
    tap(k, KeyInput::sym(Keysym::Return));
}

// ---------- T54: y; :yank ----------

#[test]
fn yank_t54_y_asks_at_once_and_the_answer_goes_to_the_clipboard() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    assert!(!tap(&mut k, KeyInput::ch('y')), "the page does not get y");
    assert_eq!(sends(&mut k), vec![sel(1, 1)], "no wait: no built-in sequence starts with y");
    assert_eq!(k.map_wait(Instant::now()), None);
    assert_eq!(clip(&cb).as_deref(), Some("old"), "nothing written before the answer");
    k.selection_result(1, 1, "hello\nworld");
    assert_eq!(clip(&cb).as_deref(), Some("hello\nworld"), "as is, newlines kept");
    assert_eq!(k.message, COPIED);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(k.key_mode, KeyMode::Normal);
}

#[test]
fn yank_command_line_does_the_same() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    type_str(&mut k, ":yank");
    enter(&mut k);
    assert_eq!(sends(&mut k), vec![sel(1, 1)]);
    k.selection_result(1, 1, "a  b\t c");
    assert_eq!(clip(&cb).as_deref(), Some("a  b\t c"));
    assert_eq!(k.message, COPIED);
    assert_eq!(k.vim_mode, VimMode::Normal);
    // web.cmd("yank") runs the same line.
    k.run_command_line("yank");
    assert_eq!(sends(&mut k), vec![sel(1, 2)]);
}

#[test]
fn yank_empty_answer_is_nothing_selected() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    k.run_command_line("yank");
    k.selection_result(1, 1, "");
    assert_eq!(k.message, NOTHING_SELECTED);
    assert_eq!(clip(&cb).as_deref(), Some("old"));
    assert_eq!(cb.0.borrow().writes, 0);
}

#[test]
fn yank_unreachable_clipboard() {
    let cb = unreachable_clipboard();
    let mut k = on_page(&cb);
    k.run_command_line("yank");
    k.selection_result(1, 1, "text");
    assert_eq!(k.message, CLIPBOARD_UNREACHABLE);
    // No desktop clipboard at all.
    let mut k = on_page(&cb);
    k.clipboard = None;
    k.run_command_line("yank");
    k.selection_result(1, 1, "text");
    assert_eq!(k.message, CLIPBOARD_UNREACHABLE);
}

#[test]
fn yank_no_page_is_nothing_selected() {
    let cb = fake("old");
    let mut k = keys_with(&cb);
    k.run_command_line("yank");
    assert_eq!(k.message, NOTHING_SELECTED);
    assert!(sends(&mut k).is_empty());
}

#[test]
fn yank_with_an_argument_is_usage_and_sends_nothing() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    for line in ["yank link", "yank image", "yank x y"] {
        k.run_command_line(line);
        assert_eq!(k.message, "web: usage: :yank", "{line}");
        assert!(sends(&mut k).is_empty(), "{line}");
    }
}

#[test]
fn yank_is_a_command_name_for_completion() {
    assert!(COMMAND_NAMES.contains(&"yank"));
    let mut sorted = COMMAND_NAMES.to_vec();
    sorted.sort();
    assert_eq!(sorted, COMMAND_NAMES, "kept sorted");
}

// ---------- answers that are ignored ----------

#[test]
fn yank_an_older_request_is_ignored() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    tap(&mut k, KeyInput::ch('y'));
    tap(&mut k, KeyInput::ch('y'));
    assert_eq!(sends(&mut k), vec![sel(1, 1), sel(1, 2)]);
    // The second answer first; the first one, arriving last, changes nothing.
    k.selection_result(1, 2, "new");
    assert_eq!(clip(&cb).as_deref(), Some("new"));
    k.message.clear();
    k.selection_result(1, 1, "stale");
    assert_eq!(clip(&cb).as_deref(), Some("new"));
    assert!(k.message.is_empty());
    // Only the latest counts even before it is answered.
    tap(&mut k, KeyInput::ch('y'));
    tap(&mut k, KeyInput::ch('y'));
    k.selection_result(1, 3, "older");
    assert_eq!(clip(&cb).as_deref(), Some("new"));
    assert!(k.message.is_empty());
    // One answer per request: the same one again is ignored.
    k.selection_result(1, 4, "latest");
    k.message.clear();
    k.selection_result(1, 4, "again");
    assert_eq!(clip(&cb).as_deref(), Some("latest"));
    assert!(k.message.is_empty());
}

#[test]
fn yank_another_tabs_answer_is_ignored() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    tap(&mut k, KeyInput::ch('y'));
    k.selection_result(2, 1, "other page");
    assert_eq!(clip(&cb).as_deref(), Some("old"));
    assert!(k.message.is_empty());
}

#[test]
fn yank_after_a_buffer_switch_is_ignored() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    k.buffers.adopt(2);
    tap(&mut k, KeyInput::ch('y'));
    assert_eq!(sends(&mut k), vec![sel(1, 1)]);
    k.run_command_line("bufferNext");
    assert_eq!(k.buffers.current_tab(), Some(2));
    k.message.clear();
    k.selection_result(1, 1, "page 1");
    assert_eq!(clip(&cb).as_deref(), Some("old"));
    assert!(k.message.is_empty());
}

#[test]
fn yank_after_a_navigation_is_ignored() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    tap(&mut k, KeyInput::ch('y'));
    k.navigated();
    k.selection_result(1, 1, "old page");
    assert_eq!(clip(&cb).as_deref(), Some("old"));
    assert!(k.message.is_empty());
}

// ---------- key repeat, T56, INPUT ----------

#[test]
fn yank_t54_a_held_y_yanks_once() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    let t = Instant::now();
    let y = KeyInput::ch('y');
    k.key(y.sym.raw(), true, &y, t);
    k.key(y.sym.raw(), true, &y, t + Duration::from_millis(600)); // OS repeats of the held y
    k.key(y.sym.raw(), true, &y, t + Duration::from_millis(650));
    k.key(y.sym.raw(), false, &y, t + Duration::from_millis(700));
    assert_eq!(sends(&mut k), vec![sel(1, 1)]);
}

#[test]
fn yank_t56_g_y_drops_the_g_and_yanks() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    type_str(&mut k, "gy");
    assert_eq!(sends(&mut k), vec![sel(1, 1)]);
    assert_eq!(clip(&cb).as_deref(), Some("old"));
}

#[test]
fn yank_t54_y_in_input_is_the_pages() {
    let cb = fake("old");
    let mut k = on_page(&cb);
    k.set_key_mode(KeyMode::Input);
    assert!(tap(&mut k, KeyInput::ch('y')));
    assert!(sends(&mut k).is_empty());
}

// ---------- T118: the user's `yy` mapping ----------

/// A shell with the `yy` mapping in init.js, on page 1, with clipboard `cb`.
fn mapped(cb: &FakeClipboard) -> Keys {
    let mut k = map::shell(YY_MAP);
    k.clipboard = Some(Box::new(cb.clone()));
    k.url = URL.into();
    let _ = k.buffers.tab_created(1, None);
    k
}

fn press_at(k: &mut Keys, c: char, t: Instant) {
    let key = KeyInput::ch(c);
    k.key(key.sym.raw(), true, &key, t);
    k.key(key.sym.raw(), false, &key, t);
}

#[test]
fn yank_t118_yy_mapping_copies_the_url_and_never_asks() {
    let cb = fake("old");
    let mut k = mapped(&cb);
    let t = Instant::now();
    press_at(&mut k, 'y', t);
    assert!(sends(&mut k).is_empty(), "y waits for the mapping");
    assert!(k.map_wait(t).is_some());
    press_at(&mut k, 'y', map::ms(t, MAP_TIMEOUT_MS - 1));
    assert_eq!(clip(&cb).as_deref(), Some(URL));
    assert_eq!(k.message, COPIED);
    // Not then, and not later.
    assert_eq!(k.map_tick(map::ms(t, 10 * MAP_TIMEOUT_MS)), None);
    assert!(map::effects(&mut k)
        .iter()
        .all(|e| !matches!(e, Effect::SendTo(_, Command::Selection { .. }))));
}

#[test]
fn yank_t118_lone_y_yanks_when_the_wait_runs_out() {
    let cb = fake("old");
    let mut k = mapped(&cb);
    let t = Instant::now();
    press_at(&mut k, 'y', t);
    assert!(sends(&mut k).is_empty());
    assert_eq!(k.map_tick(map::ms(t, MAP_TIMEOUT_MS - 1)), Some(Duration::from_millis(1)));
    assert!(sends(&mut k).is_empty(), "not yet");
    assert_eq!(k.map_tick(map::ms(t, MAP_TIMEOUT_MS)), None);
    assert_eq!(sends(&mut k), vec![sel(1, 1)]);
    k.selection_result(1, 1, "picked");
    assert_eq!(clip(&cb).as_deref(), Some("picked"));
}

#[test]
fn yank_t118_y_then_j_yanks_then_scrolls() {
    let cb = fake("old");
    let mut k = mapped(&cb);
    let t = Instant::now();
    press_at(&mut k, 'y', t);
    press_at(&mut k, 'j', map::ms(t, 50));
    assert_eq!(
        map::effects(&mut k),
        vec![sel(1, 1), Effect::KeyDown(evdev::KEY_DOWN), Effect::KeyUp(evdev::KEY_DOWN)]
    );
    assert_eq!(k.map_wait(t), None);
}
