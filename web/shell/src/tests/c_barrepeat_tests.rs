//! C-BARREPEAT: winit drops the OS's key repeats, so the shell repeats a held key of its own
//! `:` / `/` lines and the picker filter itself (fake clock). NORMAL and the page are untouched.

use std::time::{Duration, Instant};

use smithay::input::keyboard::Keysym;

use crate::keys::*;

const BS: u32 = 14;
const SHIFT: u32 = 42;

fn keys() -> Keys {
    let mut k = Keys::new(None, "file:///help.html".into(), None);
    k.repeat_rate = 20; // 50 ms
    k.repeat_delay_ms = 400;
    k
}

fn tap(k: &mut Keys, key: KeyInput, t: Instant) {
    let code = key.sym.raw();
    k.key(code, true, &key, t);
    k.key(code, false, &key, t);
}

fn line_with(k: &mut Keys, text: &str, t: Instant) {
    tap(k, KeyInput::ch(':'), t);
    for c in text.chars() {
        tap(k, KeyInput::ch(c), t);
    }
    assert_eq!(k.line.text, text);
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn held_backspace_repeats_after_the_delay_at_the_rate() {
    let mut k = keys();
    let t = Instant::now();
    line_with(&mut k, "abcdefgh", t);
    assert!(!k.key(BS, true, &KeyInput::sym(Keysym::BackSpace), t));
    assert_eq!(k.line.text, "abcdefg");
    assert_eq!(k.repeat_wait(t), Some(ms(400)));
    // Before the delay: nothing.
    assert_eq!(k.repeat_tick(t + ms(399)), Some(ms(1)));
    assert_eq!(k.line.text, "abcdefg");
    // At the delay: one repeat, next in 50 ms.
    assert_eq!(k.repeat_tick(t + ms(400)), Some(ms(50)));
    assert_eq!(k.line.text, "abcdef");
    assert_eq!(k.repeat_tick(t + ms(450)), Some(ms(50)));
    assert_eq!(k.line.text, "abcde");
    // Release stops it.
    k.key(BS, false, &KeyInput::sym(Keysym::BackSpace), t + ms(460));
    assert_eq!(k.repeat_tick(t + ms(500)), None);
    assert_eq!(k.line.text, "abcde");
}

#[test]
fn letters_and_arrows_repeat_in_the_search_line() {
    let mut k = keys();
    let t = Instant::now();
    tap(&mut k, KeyInput::ch('/'), t);
    assert_eq!(k.vim_mode, VimMode::Search);
    k.key(30, true, &KeyInput::ch('a'), t);
    k.repeat_tick(t + ms(400));
    k.repeat_tick(t + ms(450));
    assert_eq!(k.line.text, "aaa");
    k.key(30, false, &KeyInput::ch('a'), t + ms(460));
    k.key(105, true, &KeyInput::sym(Keysym::Left), t + ms(470));
    assert_eq!(k.line.cursor, 2);
    k.repeat_tick(t + ms(870));
    k.repeat_tick(t + ms(920));
    assert_eq!(k.line.cursor, 0);
}

#[test]
fn a_newer_key_takes_over_and_a_modifier_does_not() {
    let mut k = keys();
    let t = Instant::now();
    line_with(&mut k, "ab", t);
    k.key(BS, true, &KeyInput::sym(Keysym::BackSpace), t);
    k.key(SHIFT, true, &KeyInput::sym(Keysym::Shift_L), t + ms(10));
    assert_eq!(k.repeat_wait(t + ms(10)), Some(ms(390)), "Shift did not restart the repeat");
    k.key(30, true, &KeyInput::ch('x'), t + ms(20));
    assert_eq!(k.line.text, "ax");
    k.repeat_tick(t + ms(420));
    assert_eq!(k.line.text, "axx", "the newest key repeats, not Backspace");
}

#[test]
fn enter_escape_and_a_mode_change_do_not_repeat() {
    let mut k = keys();
    let t = Instant::now();
    line_with(&mut k, "a", t);
    k.key(1, true, &KeyInput::sym(Keysym::Escape), t);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(k.repeat_tick(t + ms(500)), None);
    // Backspace held while the line closes by other means.
    line_with(&mut k, "a", t);
    k.key(BS, true, &KeyInput::sym(Keysym::BackSpace), t);
    k.close_line();
    assert_eq!(k.repeat_tick(t + ms(400)), None);
    assert_eq!(k.repeat_wait(t), None);
}

#[test]
fn focus_loss_stops_the_repeat() {
    let mut k = keys();
    let t = Instant::now();
    line_with(&mut k, "abc", t);
    k.key(BS, true, &KeyInput::sym(Keysym::BackSpace), t);
    k.release_all();
    assert_eq!(k.repeat_tick(t + ms(400)), None);
    assert_eq!(k.line.text, "ab");
}

#[test]
fn rate_zero_means_no_repeat() {
    let mut k = keys();
    k.repeat_rate = 0;
    let t = Instant::now();
    line_with(&mut k, "abc", t);
    k.key(BS, true, &KeyInput::sym(Keysym::BackSpace), t);
    assert_eq!(k.repeat_wait(t), None);
}

#[test]
fn normal_mode_keys_never_repeat_through_the_shell() {
    let mut k = keys();
    let t = Instant::now();
    k.key(36, true, &KeyInput::ch('j'), t);
    assert_eq!(k.repeat_wait(t), None);
    assert_eq!(k.repeat_tick(t + ms(500)), None);
    // Opening the line with `:` does not start repeating `:`.
    k.key(36, false, &KeyInput::ch('j'), t);
    k.key(39, true, &KeyInput::ch(':'), t);
    assert_eq!(k.vim_mode, VimMode::Command);
    assert_eq!(k.repeat_wait(t), None);
    // INPUT mode (keys go to the page, the engine repeats them) neither.
    let mut k = keys();
    k.set_key_mode(KeyMode::Input);
    assert!(k.key(BS, true, &KeyInput::sym(Keysym::BackSpace), t));
    assert_eq!(k.repeat_wait(t), None);
}
