//! C5b (KEYS.md T23): `/query` Enter makes the first match at or below the top of the screen
//! current; none below: the first match, with the wrap message. The host chooses (the page's
//! text positions are its to read); the shell asks for it (`screen_top`) and tells the wrap.

use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::keys::*;
use crate::protocol::Command;

const HELP: &str = "file:///run/user/1000/web-1/help.html";

fn keys() -> Keys {
    Keys::new(None, HELP.into(), None)
}

fn tap(k: &mut Keys, key: KeyInput) {
    let code = key.sym.raw();
    let t = Instant::now();
    k.key(code, true, &key, t);
    k.key(code, false, &key, t);
}

fn type_str(k: &mut Keys, s: &str) {
    for c in s.chars() {
        tap(k, KeyInput::ch(c));
    }
}

/// The `find` commands queued (id, screen_top), the rest dropped.
fn finds(k: &mut Keys) -> Vec<(u64, bool)> {
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter_map(|e| match e {
            Effect::Send(Command::Find { id, screen_top, .. }) => Some((id, screen_top)),
            _ => None,
        })
        .collect()
}

fn search_enter(k: &mut Keys, query: &str) -> u64 {
    tap(k, KeyInput::ch('/'));
    type_str(k, query);
    tap(k, KeyInput::sym(Keysym::Return));
    let f = finds(k);
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].1, "T23: `/` Enter asks for the start at the top of the screen");
    f[0].0
}

#[test]
fn keys_t23_enter_asks_for_the_screen_top_start_and_n_does_not() {
    let mut k = keys();
    let id = search_enter(&mut k, "needle");
    k.find_result_wrapped(id, 3, 6, false);
    tap(&mut k, KeyInput::ch('n'));
    assert_eq!(finds(&mut k).iter().map(|f| f.1).collect::<Vec<_>>(), vec![false], "n steps the open session");
    tap(&mut k, KeyInput::sym(Keysym::Escape));
    k.effects.clear();
    tap(&mut k, KeyInput::ch('n'));
    assert_eq!(
        finds(&mut k).iter().map(|f| f.1).collect::<Vec<_>>(),
        vec![false],
        "n after Esc starts where the engine picks (T7)"
    );
}

#[test]
fn keys_t23_the_match_below_the_top_is_current_with_its_index_and_no_wrap_message() {
    let mut k = keys();
    let id = search_enter(&mut k, "needle");
    k.find_result_wrapped(id, 3, 6, false);
    assert_eq!(k.message, "[3/6]");
    assert_eq!((k.search.index, k.search.count), (3, 6));
}

#[test]
fn keys_t23_no_match_below_the_top_wraps_to_the_first_with_the_message() {
    let mut k = keys();
    let id = search_enter(&mut k, "needle");
    k.find_result_wrapped(id, 1, 6, true);
    assert_eq!(k.message, format!("{SEARCH_WRAP_BOTTOM}  [1/6]"));
    // The next `n` is an ordinary step from match 1.
    tap(&mut k, KeyInput::ch('n'));
    let id = finds(&mut k)[0].0;
    k.find_result_wrapped(id, 2, 6, false);
    assert_eq!(k.message, "[2/6]");
}

#[test]
fn keys_t23_the_first_match_alone_is_no_wrap_unless_the_host_says_so() {
    let mut k = keys();
    let id = search_enter(&mut k, "needle");
    k.find_result_wrapped(id, 1, 6, false);
    assert_eq!(k.message, "[1/6]", "match 1 is at or below the top of the screen: no wrap message");
}

#[test]
fn keys_t23_the_wrap_flag_without_a_match_is_pattern_not_found() {
    let mut k = keys();
    let id = search_enter(&mut k, "zzz");
    k.find_result_wrapped(id, 0, 0, true);
    assert_eq!(k.message, "Pattern not found: zzz");
}

#[test]
fn keys_t23_n_after_the_screen_top_search_still_wraps_at_the_bottom() {
    let mut k = keys();
    let id = search_enter(&mut k, "needle");
    k.find_result_wrapped(id, 6, 6, false);
    tap(&mut k, KeyInput::ch('n'));
    let f = finds(&mut k);
    assert_eq!(f.len(), 1);
    k.find_result_wrapped(f[0].0, 1, 6, false);
    assert_eq!(k.message, format!("{SEARCH_WRAP_BOTTOM}  [1/6]"));
}
