//! KEYS.md Search rows (C5): T7, T8, T15-T17, T22-T25, T29, T31 and smartcase. Each test
//! names its row(s). The host is not involved: the tests read the commands the key model
//! queues and feed it the `find-result`s the host would send.

use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::keys::*;
use crate::protocol::{Command, FindDir};

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
        tap(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

/// The queued effects, with the `find` request ids zeroed (the id tests read `k.effects`).
fn effects(k: &mut Keys) -> Vec<Effect> {
    let mut v = std::mem::take(&mut k.effects);
    for e in &mut v {
        if let Effect::Send(Command::Find { id, .. }) = e {
            *id = 0;
        }
    }
    v
}

/// The host answers the oldest `find` waiting (its id echoed); with none waiting the
/// result carries an id nobody asked for.
fn answer(k: &mut Keys, index: u32, count: u32) {
    let id = k.search.pending_id().unwrap_or(0);
    k.find_result(id, index, count);
}

fn find(query: &str, dir: FindDir, match_case: bool) -> Effect {
    Effect::Send(Command::Find {
        id: 0,
        query: query.into(),
        dir,
        match_case,
        screen_top: false,
    })
}

/// The find a `/query` Enter sends (C5b, T23): from the top of the screen.
fn find_top(query: &str, match_case: bool) -> Effect {
    Effect::Send(Command::Find {
        id: 0,
        query: query.into(),
        dir: FindDir::Next,
        match_case,
        screen_top: true,
    })
}

/// `/query` Enter, with the host answering `index/count`; the commands it queued are drained.
fn search(k: &mut Keys, query: &str, index: u32, count: u32) {
    tap(k, KeyInput::ch('/'));
    type_str(k, query);
    tap(k, KeyInput::sym(Keysym::Return));
    effects(k);
    answer(k, index, count);
}

#[test]
fn keys_t08_slash_opens_an_empty_search_line() {
    let mut k = keys();
    tap(&mut k, KeyInput::ch('/'));
    assert_eq!(k.vim_mode, VimMode::Search);
    assert_eq!(k.key_mode, KeyMode::Normal);
    assert_eq!(
        k.bar_view().line,
        Some(('/', String::new(), 0, None)),
        "the line takes the bar, prompt /"
    );
    assert!(effects(&mut k).is_empty());
}

#[test]
fn keys_t22_the_search_line_edits_like_the_command_line() {
    let mut k = keys();
    tap(&mut k, KeyInput::ch('/'));
    type_str(&mut k, "foo bar");
    assert_eq!(k.bar_view().line, Some(('/', "foo bar".into(), 7, None)));
    tap(&mut k, KeyInput::sym(Keysym::BackSpace));
    tap(&mut k, KeyInput::sym(Keysym::Left));
    type_str(&mut k, "X");
    assert_eq!(k.line.text, "foo bXa");
    tap(&mut k, KeyInput::ch('w').ctrl());
    assert_eq!(k.line.text, "foo a", "Ctrl-w deletes the word before the cursor");
    tap(&mut k, KeyInput::sym(Keysym::Home));
    type_str(&mut k, "ab");
    tap(&mut k, KeyInput::ch('u').ctrl());
    assert_eq!(k.line.text, "foo a");
    // No ghost completion in a search line, and Tab does nothing.
    tap(&mut k, KeyInput::sym(Keysym::Tab));
    assert_eq!(k.bar_view().ghost, "");
    assert_eq!(k.line.text, "foo a");
    // Paste (with no desktop clipboard here) changes nothing; c6_tests.rs pastes for real.
    tap(&mut k, KeyInput::ch('v').ctrl());
    assert_eq!(k.line.text, "foo a");
}

#[test]
fn keys_t23_enter_runs_the_query_and_shows_the_index() {
    let mut k = keys();
    tap(&mut k, KeyInput::ch('/'));
    type_str(&mut k, "needle");
    tap(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(effects(&mut k), vec![find_top("needle", false)]);
    assert_eq!(k.search.query.as_deref(), Some("needle"), "the kept query");
    answer(&mut k, 3, 7);
    assert_eq!(k.message, "[3/7]");
}

#[test]
fn keys_t23_a_new_search_starts_a_fresh_host_session() {
    let mut k = keys();
    search(&mut k, "needle", 1, 4);
    // The same query again must not step from the active match: end the session first.
    tap(&mut k, KeyInput::ch('/'));
    type_str(&mut k, "needle");
    tap(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(effects(&mut k), vec![Effect::Send(Command::FindClear), find_top("needle", false)]);
    answer(&mut k, 2, 4);
    assert_eq!(k.message, "[2/4]", "a fresh session never reports a wrap");
}

#[test]
fn keys_t23_no_match_says_pattern_not_found_and_keeps_the_query() {
    let mut k = keys();
    search(&mut k, "zzz", 0, 0);
    assert_eq!(k.message, "Pattern not found: zzz");
    assert_eq!(k.search.query.as_deref(), Some("zzz"));
}

#[test]
fn keys_t24_enter_on_an_empty_line_is_esc() {
    let mut k = keys();
    search(&mut k, "keep", 1, 2);
    tap(&mut k, KeyInput::ch('/'));
    tap(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(effects(&mut k).is_empty());
    assert_eq!(k.search.query.as_deref(), Some("keep"));
    assert_eq!(k.message, "[1/2]", "nothing else changed");
}

#[test]
fn keys_t25_esc_cancels_and_restores_the_previous_search() {
    let mut k = keys();
    search(&mut k, "keep", 2, 5);
    tap(&mut k, KeyInput::ch('/'));
    type_str(&mut k, "other");
    tap(&mut k, KeyInput::sym(Keysym::Escape));
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.line.text.is_empty());
    assert!(effects(&mut k).is_empty(), "nothing was sent: the highlights are as they were");
    assert_eq!(k.search.query.as_deref(), Some("keep"));
    assert!(k.search.session);
    // And `n` still uses the kept query.
    tap(&mut k, KeyInput::ch('n'));
    assert_eq!(effects(&mut k), vec![find("keep", FindDir::Next, false)]);
}

#[test]
fn keys_t15_n_and_capital_n_step_and_show_the_index() {
    let mut k = keys();
    search(&mut k, "x", 2, 5);
    tap(&mut k, KeyInput::ch('n'));
    assert_eq!(effects(&mut k), vec![find("x", FindDir::Next, false)]);
    answer(&mut k, 3, 5);
    assert_eq!(k.message, "[3/5]");
    tap(&mut k, KeyInput::ch('N'));
    assert_eq!(effects(&mut k), vec![find("x", FindDir::Prev, false)]);
    answer(&mut k, 2, 5);
    assert_eq!(k.message, "[2/5]");
    assert_eq!(k.vim_mode, VimMode::Normal);
}

#[test]
fn keys_t15_wrap_messages_come_from_the_indices() {
    let mut k = keys();
    search(&mut k, "x", 5, 5);
    tap(&mut k, KeyInput::ch('n'));
    effects(&mut k);
    answer(&mut k, 1, 5);
    assert_eq!(k.message, "search hit BOTTOM, continuing at TOP  [1/5]");
    tap(&mut k, KeyInput::ch('N'));
    effects(&mut k);
    answer(&mut k, 5, 5);
    assert_eq!(k.message, "search hit TOP, continuing at BOTTOM  [5/5]");
    // One match: n and N both wrap onto it.
    let mut k = keys();
    search(&mut k, "x", 1, 1);
    tap(&mut k, KeyInput::ch('n'));
    effects(&mut k);
    answer(&mut k, 1, 1);
    assert_eq!(k.message, "search hit BOTTOM, continuing at TOP  [1/1]");
    tap(&mut k, KeyInput::ch('N'));
    effects(&mut k);
    answer(&mut k, 1, 1);
    assert_eq!(k.message, "search hit TOP, continuing at BOTTOM  [1/1]");
}

#[test]
fn keys_t15_fast_presses_are_answered_in_order() {
    let mut k = keys();
    search(&mut k, "x", 4, 5);
    tap(&mut k, KeyInput::ch('n'));
    tap(&mut k, KeyInput::ch('n'));
    assert_eq!(effects(&mut k).len(), 2);
    answer(&mut k, 5, 5);
    assert_eq!(k.message, "[5/5]");
    answer(&mut k, 1, 5);
    assert_eq!(k.message, "search hit BOTTOM, continuing at TOP  [1/5]");
}

#[test]
fn keys_t16_n_with_no_match_here_says_pattern_not_found() {
    let mut k = keys();
    search(&mut k, "gone", 1, 3);
    k.navigated();
    tap(&mut k, KeyInput::ch('n'));
    assert_eq!(effects(&mut k), vec![find("gone", FindDir::Next, false)]);
    answer(&mut k, 0, 0);
    assert_eq!(k.message, "Pattern not found: gone");
    tap(&mut k, KeyInput::ch('N'));
    effects(&mut k);
    answer(&mut k, 0, 0);
    assert_eq!(k.message, "Pattern not found: gone");
}

#[test]
fn keys_t17_n_with_no_query_says_no_previous_search() {
    for c in ['n', 'N'] {
        let mut k = keys();
        tap(&mut k, KeyInput::ch(c));
        assert_eq!(k.message, "No previous search", "{c}");
        assert!(effects(&mut k).is_empty());
        assert_eq!(k.vim_mode, VimMode::Normal);
    }
}

#[test]
fn keys_t07_esc_in_normal_clears_the_highlights_and_keeps_the_query() {
    let mut k = keys();
    search(&mut k, "keep", 2, 5);
    tap(&mut k, KeyInput::sym(Keysym::Escape));
    assert_eq!(effects(&mut k), vec![Effect::Send(Command::FindClear)]);
    assert_eq!(k.search.query.as_deref(), Some("keep"));
    assert_eq!(k.key_mode, KeyMode::Normal, "never changes the mode");
    assert_eq!(k.message, "");
    // A second Esc has nothing to clear.
    tap(&mut k, KeyInput::sym(Keysym::Escape));
    assert!(effects(&mut k).is_empty());
    // `n` searches again from the host's first match: a fresh session, no wrap message.
    tap(&mut k, KeyInput::ch('n'));
    assert_eq!(effects(&mut k), vec![find("keep", FindDir::Next, false)]);
    answer(&mut k, 1, 5);
    assert_eq!(k.message, "[1/5]");
}

#[test]
fn keys_t07_esc_with_no_search_sends_nothing() {
    let mut k = keys();
    tap(&mut k, KeyInput::sym(Keysym::Escape));
    assert!(effects(&mut k).is_empty());
}

#[test]
fn keys_t31_navigation_clears_the_highlights_and_keeps_the_query() {
    let mut k = keys();
    search(&mut k, "keep", 2, 5);
    k.navigated();
    assert_eq!(k.search.query.as_deref(), Some("keep"));
    assert!(!k.search.session);
    assert_eq!((k.search.index, k.search.count), (0, 0));
    assert_eq!(k.message, "", "the [i/N] of the old page is gone");
    assert!(effects(&mut k).is_empty(), "the host dropped its session itself");
    // A result that arrives late (for the old page) is ignored.
    answer(&mut k, 3, 5);
    assert_eq!(k.message, "");
    // Esc after a navigation has nothing to clear; `n` searches the new page.
    tap(&mut k, KeyInput::sym(Keysym::Escape));
    assert!(effects(&mut k).is_empty());
    tap(&mut k, KeyInput::ch('n'));
    assert_eq!(effects(&mut k), vec![find("keep", FindDir::Next, false)]);
    // An open search line stays open across a navigation.
    let mut k = keys();
    tap(&mut k, KeyInput::ch('/'));
    type_str(&mut k, "ab");
    k.navigated();
    assert_eq!(k.vim_mode, VimMode::Search);
    assert_eq!(k.line.text, "ab");
}

#[test]
fn keys_t29_a_click_into_a_field_cancels_the_search_line() {
    let mut k = keys();
    tap(&mut k, KeyInput::ch('/'));
    type_str(&mut k, "ab");
    k.user_focused_control();
    assert_eq!(k.key_mode, KeyMode::Input);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(effects(&mut k).is_empty(), "the cancelled search did not run");
}

#[test]
fn keys_smartcase_a_capital_makes_the_search_case_sensitive() {
    assert!(!smartcase_match_case("needle"));
    assert!(smartcase_match_case("Needle"));
    assert!(smartcase_match_case("needLe 1"));
    assert!(!smartcase_match_case("needle 1 !"));
    let mut k = keys();
    tap(&mut k, KeyInput::ch('/'));
    type_str(&mut k, "Foo");
    tap(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(effects(&mut k), vec![find_top("Foo", true)]);
    answer(&mut k, 1, 1);
    tap(&mut k, KeyInput::ch('n'));
    assert_eq!(effects(&mut k), vec![find("Foo", FindDir::Next, true)], "n keeps the case flag");
}

#[test]
fn keys_s7_enter_after_a_search_ends_it_and_clicks_nothing_itself() {
    // C-ENTER: Enter in NORMAL is the page's (T18, c_enter_tests.rs): the shell only ends the
    // find (T7) and hands the page an Enter; no click, no `Match spans N links`.
    let mut k = keys();
    search(&mut k, "x", 1, 2);
    tap(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(
        effects(&mut k),
        vec![
            Effect::Send(Command::FindClear),
            Effect::Pause(crate::config::ENTER_AFTER_FIND_CLEAR_MS),
            Effect::KeyDown(crate::config::evdev::KEY_ENTER),
            Effect::KeyUp(crate::config::evdev::KEY_ENTER)
        ]
    );
    assert_eq!(k.message, "");
}

fn sent_find_ids(k: &mut Keys) -> Vec<u64> {
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter_map(|e| match e {
            Effect::Send(Command::Find { id, .. }) => Some(id),
            _ => None,
        })
        .collect()
}

#[test]
fn keys_t16_c5_stale_a_result_nobody_asked_for_is_never_taken_as_the_answer() {
    // CEF's stale `0 0` at a load start: the session is gone, the result has no request.
    let mut k = keys();
    search(&mut k, "gone", 1, 3);
    k.navigated();
    k.find_result(1, 0, 0);
    assert_eq!(k.message, "");
    // `n` right after: the stale result (old id, or none) must not answer it.
    tap(&mut k, KeyInput::ch('n'));
    let ids = sent_find_ids(&mut k);
    assert_eq!(ids.len(), 1);
    k.find_result(1, 0, 0);
    k.find_result(0, 0, 0);
    assert_eq!(k.message, "", "still waiting for its own answer");
    k.find_result(ids[0], 2, 3);
    assert_eq!(k.message, "[2/3]");
}

#[test]
fn keys_t15_c5_stale_ids_are_per_request_and_the_latest_wins() {
    let mut k = keys();
    search(&mut k, "x", 1, 5);
    tap(&mut k, KeyInput::ch('n'));
    tap(&mut k, KeyInput::ch('n'));
    let ids = sent_find_ids(&mut k);
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    // The host answers only the latest request: the older one is dropped with it.
    k.find_result(ids[1], 3, 5);
    assert_eq!(k.message, "[3/5]");
    assert_eq!(k.search.pending_id(), None);
    // An answer to the older one afterwards changes nothing.
    k.find_result(ids[0], 2, 5);
    assert_eq!(k.message, "[3/5]");
}

#[test]
fn keys_t31_a_same_document_navigation_clears_the_find_session_and_keeps_the_query() {
    let mut k = keys();
    search(&mut k, "keep", 2, 5);
    k.navigated_same_document();
    assert_eq!(
        effects(&mut k),
        vec![Effect::Send(Command::FindClear)],
        "the host still has its session"
    );
    assert_eq!(k.search.query.as_deref(), Some("keep"));
    assert!(!k.search.session);
    assert_eq!((k.search.index, k.search.count), (0, 0));
    assert_eq!(k.message, "", "the [i/N] is gone");
    // No session: nothing to clear a second time.
    k.navigated_same_document();
    assert!(effects(&mut k).is_empty());
    // `n` searches again.
    tap(&mut k, KeyInput::ch('n'));
    assert_eq!(effects(&mut k), vec![find("keep", FindDir::Next, false)]);
}
