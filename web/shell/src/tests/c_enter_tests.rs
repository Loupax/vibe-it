//! C-ENTER (KEYS.md T18-T20: Enter in NORMAL goes to the page) and C-CARETCMD (`:set caret`,
//! `:set nocaret`, `:set caret?`, their completion). Each test names its row or command.

use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::completion::{compute_ghost, CompleteCache};
use crate::keys::*;
use crate::protocol::{Command, FindDir};

const HELP: &str = "file:///run/user/1000/web-1/help.html";

fn keys() -> Keys {
    Keys::new(None, HELP.into(), None)
}

fn press(k: &mut Keys, key: KeyInput) -> bool {
    k.key(key.sym.raw(), true, &key, Instant::now())
}

fn release(k: &mut Keys, key: KeyInput) -> bool {
    k.key(key.sym.raw(), false, &key, Instant::now())
}

/// Press and release: (page got the press, page got the release).
fn tap(k: &mut Keys, key: KeyInput) -> (bool, bool) {
    (press(k, key), release(k, key))
}

fn type_str(k: &mut Keys, s: &str) {
    for c in s.chars() {
        tap(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

fn effects(k: &mut Keys) -> Vec<Effect> {
    std::mem::take(&mut k.effects)
}

/// T18: the wait between `find-clear` and the Enter given to the page.
const PAUSE: Effect = Effect::Pause(crate::config::ENTER_AFTER_FIND_CLEAR_MS);

fn enter() -> KeyInput {
    KeyInput::sym(Keysym::Return)
}

/// `:<line>` Enter.
fn run(k: &mut Keys, line: &str) {
    tap(k, KeyInput::ch(':'));
    type_str(k, line);
    tap(k, enter());
}

// ---------- C-ENTER: T18-T20 ----------

#[test]
fn keys_t18_enter_in_normal_goes_to_the_page() {
    let mut k = keys();
    assert_eq!(tap(&mut k, enter()), (true, true), "press and release reach the page");
    assert_eq!(tap(&mut k, KeyInput::sym(Keysym::KP_Enter)), (true, true), "keypad Enter too");
    assert!(effects(&mut k).is_empty(), "no shell-side click, no command");
    assert!(k.message.is_empty());
    assert_eq!((k.key_mode, k.vim_mode), (KeyMode::Normal, VimMode::Normal));
}

/// `/query` Enter with the host answering match 1 of `count`; the queued commands drained.
fn searched(k: &mut Keys, query: &str, count: u32) {
    tap(k, KeyInput::ch('/'));
    type_str(k, query);
    assert_eq!(tap(k, enter()), (false, false), "T23: the line's Enter is the shell's");
    let id = match effects(k).as_slice() {
        [Effect::Send(Command::Find {
            id, dir: FindDir::Next, ..
        })] => *id,
        e => panic!("{e:?}"),
    };
    k.find_result(id, 1, count);
}

#[test]
fn keys_t18_after_a_search_enter_ends_the_find_then_goes_to_the_page() {
    // `/x` Enter, host: match 1 of 2. Enter ends the find as T7 does (`find-clear`: Chromium
    // focuses the link holding the active match) and then gives the page an Enter, held while
    // the real key is down. The real key itself is not forwarded (no double Enter).
    let mut k = keys();
    searched(&mut k, "x", 2);
    assert_eq!(k.message, "[1/2]");
    assert!(!press(&mut k, enter()), "the real press is replaced by the synthesized one");
    assert_eq!(
        effects(&mut k),
        vec![Effect::Send(Command::FindClear), PAUSE, Effect::KeyDown(crate::config::evdev::KEY_ENTER)],
        "find-clear first, then the Enter"
    );
    assert!(!press(&mut k, enter()), "a repeat: the engine repeats the held Enter");
    assert!(effects(&mut k).is_empty());
    assert!(!release(&mut k, enter()));
    assert_eq!(effects(&mut k), vec![Effect::KeyUp(crate::config::evdev::KEY_ENTER)]);
    assert!(!k.search.session, "the session ended (T7)");
    assert!(k.message.is_empty(), "[i/N] goes with it");
    assert_eq!(k.search.query.as_deref(), Some("x"), "the query is kept for n");
    // The next Enter (no session) goes to the page as it is.
    assert_eq!(tap(&mut k, enter()), (true, true));
    assert!(effects(&mut k).is_empty());
}

#[test]
fn keys_t18_t07_after_esc_enter_goes_straight_to_the_page() {
    let mut k = keys();
    searched(&mut k, "x", 2);
    tap(&mut k, KeyInput::sym(Keysym::Escape));
    assert_eq!(effects(&mut k), vec![Effect::Send(Command::FindClear)]);
    assert_eq!(tap(&mut k, enter()), (true, true));
    assert!(effects(&mut k).is_empty());
}

#[test]
fn keys_t19_no_match_spans_n_links_message() {
    // T19 is gone: whatever the match covers, Enter is the page's.
    let mut k = keys();
    searched(&mut k, "two links", 1);
    press(&mut k, enter());
    release(&mut k, enter());
    assert!(!k.message.contains("spans"), "{}", k.message);
}

#[test]
fn keys_t18_no_matches_still_ends_the_session() {
    // `Pattern not found` leaves a host session (count 0): Enter ends it too, then the page
    // gets the Enter (nothing focused: nothing happens).
    let mut k = keys();
    tap(&mut k, KeyInput::ch('/'));
    type_str(&mut k, "zzz");
    tap(&mut k, enter());
    effects(&mut k);
    let id = k.search.pending_id().unwrap();
    k.find_result(id, 0, 0);
    assert!(!press(&mut k, enter()));
    assert_eq!(
        effects(&mut k),
        vec![Effect::Send(Command::FindClear), PAUSE, Effect::KeyDown(crate::config::evdev::KEY_ENTER)]
    );
}

#[test]
fn keys_t20_enter_with_no_match_still_goes_to_the_page() {
    // No search at all: the page gets Enter (with nothing focused Chromium does nothing).
    let mut k = keys();
    assert!(k.search.query.is_none());
    assert_eq!(tap(&mut k, enter()), (true, true));
    assert!(effects(&mut k).is_empty());
}

#[test]
fn keys_t18_t21_a_modified_enter_does_nothing() {
    let mut k = keys();
    for key in [enter().ctrl(), enter().shift(), enter().alt(), KeyInput::sym(Keysym::KP_Enter).ctrl()] {
        assert_eq!(tap(&mut k, key), (false, false), "{key:?}");
    }
    let mut super_enter = enter();
    super_enter.logo = true;
    assert_eq!(tap(&mut k, super_enter), (false, false));
    assert!(effects(&mut k).is_empty());
}

#[test]
fn keys_t12_t18_pending_g_then_enter_goes_to_the_page_and_clears_g() {
    let mut k = keys();
    tap(&mut k, KeyInput::ch('g'));
    assert_eq!(tap(&mut k, enter()), (true, true));
    tap(&mut k, KeyInput::ch('g'));
    assert!(effects(&mut k).is_empty(), "the g before Enter no longer pairs with this one");
}

#[test]
fn keys_t18_held_enter_repeats_reach_the_page() {
    let mut k = keys();
    assert!(press(&mut k, enter()));
    assert!(press(&mut k, enter()), "a repeat (key already down) is forwarded too");
    assert!(release(&mut k, enter()));
}

#[test]
fn keys_t27_t23_enter_in_the_lines_stays_with_the_shell() {
    let mut k = keys();
    tap(&mut k, KeyInput::ch(':'));
    assert_eq!(tap(&mut k, enter()), (false, false));
    tap(&mut k, KeyInput::ch('/'));
    assert_eq!(tap(&mut k, enter()), (false, false));
    assert_eq!(k.vim_mode, VimMode::Normal);
}

#[test]
fn keys_t33_t29_enter_that_focuses_a_field_is_released_in_the_page() {
    // Enter goes down in NORMAL, the page focuses a field (editable-focused true -> INPUT)
    // before the key comes up: the release still reaches the page (no stuck Enter).
    let mut k = keys();
    assert!(press(&mut k, enter()));
    k.user_focused_control();
    assert_eq!(k.key_mode, KeyMode::Input);
    assert!(release(&mut k, enter()));
}

#[test]
fn keys_t05_enter_in_input_is_unchanged() {
    let mut k = keys();
    k.set_key_mode(KeyMode::Input);
    assert_eq!(tap(&mut k, enter()), (true, true));
    assert_eq!(tap(&mut k, enter().ctrl()), (true, true), "INPUT forwards modified Enter too");
}

// ---------- C-CARETCMD: :set caret / nocaret / caret? ----------

#[test]
fn cmd_set_caret_turns_caret_browsing_on() {
    let mut k = keys();
    assert!(!k.caret, "off at start");
    run(&mut k, "set caret");
    assert_eq!(effects(&mut k), vec![Effect::Send(Command::Caret { on: true })]);
    assert!(k.caret);
    assert!(k.message.contains("caret browsing on"), "{}", k.message);
    assert_eq!(k.key_mode, KeyMode::Normal, "the mode does not change");
}

#[test]
fn cmd_set_nocaret_turns_it_off() {
    let mut k = keys();
    run(&mut k, "set caret");
    effects(&mut k);
    run(&mut k, "set nocaret");
    assert_eq!(effects(&mut k), vec![Effect::Send(Command::Caret { on: false })]);
    assert!(!k.caret);
    assert_eq!(k.message, "caret browsing off");
}

#[test]
fn cmd_set_caret_query_shows_the_state_and_sends_nothing() {
    let mut k = keys();
    run(&mut k, "set caret?");
    assert_eq!(k.message, "nocaret");
    run(&mut k, "set caret");
    effects(&mut k);
    run(&mut k, "set caret?");
    assert_eq!(k.message, "caret");
    assert!(effects(&mut k).is_empty());
}

#[test]
fn cmd_set_unknown_option_and_usage() {
    let mut k = keys();
    run(&mut k, "set js");
    assert!(k.message.starts_with("web: unknown option: js"), "{}", k.message);
    run(&mut k, "set");
    assert_eq!(k.message, SET_USAGE);
    run(&mut k, "set caret nocaret");
    assert_eq!(k.message, SET_USAGE);
    assert!(effects(&mut k).is_empty());
    assert!(!k.caret);
}

#[test]
fn cmd_set_caret_leaves_the_normal_bindings_alone() {
    // The user wants to experiment: j/k/G still send Down/Up/End with caret on.
    let mut k = keys();
    run(&mut k, "set caret");
    effects(&mut k);
    for (key, want) in [('j', 108), ('k', 103), ('G', 107)] {
        press(&mut k, KeyInput::ch(key));
        assert_eq!(effects(&mut k), vec![Effect::KeyDown(want)], "{key}");
        release(&mut k, KeyInput::ch(key));
        effects(&mut k);
    }
}

fn ghost_rest(line: &str) -> Option<String> {
    compute_ghost(line, line.len(), &CompleteCache::new(None)).map(|g| g.rest(line))
}

#[test]
fn keys_t45_set_completes_its_name_and_arguments() {
    assert_eq!(ghost_rest("se").as_deref(), Some("t"));
    assert_eq!(ghost_rest("set c").as_deref(), Some("aret"));
    assert_eq!(ghost_rest("set n").as_deref(), Some("ocaret"));
    assert_eq!(ghost_rest("set caret").as_deref(), Some(""), "the exact word first: no ghost text");
    assert_eq!(ghost_rest("set x"), None);
    let g = compute_ghost("set c", 5, &CompleteCache::new(None)).unwrap();
    assert_eq!(g.candidates, vec!["caret", "caret?"]);
}

#[test]
fn keys_t50_t53_set_tab_and_enter() {
    // `:se` Tab -> `:set ` (a command name), `no` Tab -> `nocaret ` (T53: a :set argument
    // gets a space), Enter runs it.
    let mut k = keys();
    tap(&mut k, KeyInput::ch(':'));
    type_str(&mut k, "se");
    tap(&mut k, KeyInput::sym(Keysym::Tab));
    assert_eq!(k.line.text, "set ");
    type_str(&mut k, "no");
    tap(&mut k, KeyInput::sym(Keysym::Tab));
    assert_eq!(k.line.text, "set nocaret ");
    tap(&mut k, enter());
    assert_eq!(effects(&mut k), vec![Effect::Send(Command::Caret { on: false })]);
    // `:set c` Enter takes the ghost: `:set caret`, not `caret?`.
    run(&mut k, "set c");
    assert_eq!(effects(&mut k), vec![Effect::Send(Command::Caret { on: true })]);
    // `:set caret` Tab cycles to `caret?`, Enter asks.
    tap(&mut k, KeyInput::ch(':'));
    type_str(&mut k, "set caret");
    tap(&mut k, KeyInput::sym(Keysym::Tab));
    tap(&mut k, enter());
    assert_eq!(k.message, "caret");
    assert!(effects(&mut k).is_empty());
}
