//! C-AUTOFOCUS (KEYS.md T29, T32, T13, `gi`; DECISIONS.md "Only a focus the user caused switches to
//! INPUT"): the user-caused window, park / restore / clear, the refocus guard, `gi`. A fake clock: every
//! time is `t0` plus some milliseconds.

use std::time::{Duration, Instant};

use smithay::input::keyboard::Keysym;

use crate::buffers::TabCreated;
use crate::config::{CLICK_FOCUS_MS, KEY_SEQUENCES};
use crate::keys::*;
use crate::protocol::{Command, Event, FieldWhich};

const W: u64 = CLICK_FOCUS_MS;

fn ms(t0: Instant, n: u64) -> Instant {
    t0 + Duration::from_millis(n)
}

/// Buffer 1 on page 1, NORMAL, no effects.
fn keys() -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    assert_eq!(k.key_mode, KeyMode::Normal);
    k
}

fn tap(k: &mut Keys, key: KeyInput, t: Instant) -> (bool, bool) {
    (k.key(key.sym.raw(), true, &key, t), k.key(key.sym.raw(), false, &key, t))
}

fn effects(k: &mut Keys) -> Vec<Effect> {
    std::mem::take(&mut k.effects)
}

fn blur(tab: u64) -> Effect {
    Effect::SendTo(tab, Command::BlurField)
}

fn focus(which: FieldWhich) -> Effect {
    Effect::Send(Command::FocusField { which })
}

/// The page focused a field itself (no click): `editable-focused true` at `t`.
fn page_focus(k: &mut Keys, t: Instant) -> FocusOutcome {
    k.editable_focused(1, false, t)
}

fn esc_esc(k: &mut Keys, t: Instant) {
    tap(k, KeyInput::sym(Keysym::Escape), t);
    tap(k, KeyInput::sym(Keysym::Escape), t);
    assert_eq!(k.key_mode, KeyMode::Normal);
}

fn load_start(k: &mut Keys, tab: u64) {
    let ev = Event::Load {
        tab,
        phase: "start".into(),
        status: 0,
        kind: "new".into(),
        error: None,
    };
    k.page_event(tab, &ev);
}

// ---------- the user-caused window ----------

#[test]
fn autofocus_t32_a_page_focus_keeps_normal_and_parks_the_field() {
    let t0 = Instant::now();
    let mut k = keys();
    assert_eq!(page_focus(&mut k, t0), FocusOutcome::Parked);
    assert_eq!(k.key_mode, KeyMode::Normal, "the page's own focus leaves the mode alone");
    assert_eq!(effects(&mut k), vec![blur(1)], "the host blurs it, so j/k scroll (T13)");
    assert!(k.buffers.current().parked);
    // A blur (`editable-focused false`) has no key-model call: nothing changes.
    assert_eq!(k.key_mode, KeyMode::Normal);
}

#[test]
fn autofocus_t29_a_click_within_the_window_switches_to_input() {
    let t0 = Instant::now();
    // `after_click` is State's `click_recent(last_click, now)`: inside the window, and just past it.
    assert!(click_recent(Some(t0), ms(t0, W)));
    assert!(!click_recent(Some(t0), ms(t0, W + 1)));
    let mut k = keys();
    assert_eq!(k.editable_focused(1, true, ms(t0, 5)), FocusOutcome::Input);
    assert_eq!(k.key_mode, KeyMode::Input);
    assert!(effects(&mut k).is_empty(), "nothing blurred");
    assert!(!k.buffers.current().parked);
}

#[test]
fn autofocus_t19_an_enter_given_to_the_page_counts_for_the_window() {
    let t0 = Instant::now();
    let mut k = keys();
    assert_eq!(tap(&mut k, KeyInput::sym(Keysym::Return), t0), (true, true));
    assert_eq!(
        k.editable_focused(1, false, ms(t0, W)),
        FocusOutcome::Input,
        "the Enter pressed a button that focused a field"
    );
    let mut k = keys();
    tap(&mut k, KeyInput::sym(Keysym::Return), t0);
    assert_eq!(page_focus(&mut k, ms(t0, W + 1)), FocusOutcome::Parked, "too late: the page's own");
    assert_eq!(k.key_mode, KeyMode::Normal);
}

#[test]
fn autofocus_t18_find_enter_is_timed_from_the_real_press() {
    let t0 = Instant::now();
    let mut k = keys();
    tap(&mut k, KeyInput::ch('/'), t0);
    tap(&mut k, KeyInput::ch('q'), t0);
    tap(&mut k, KeyInput::sym(Keysym::Return), t0);
    assert!(k.search.session);
    effects(&mut k);
    // Enter with a find open: find-clear (Chromium focuses the match's field there), the Enter later.
    let (to_page, _) = tap(&mut k, KeyInput::sym(Keysym::Return), ms(t0, 100));
    assert!(!to_page, "T18: the Enter is the shell's evdev one, after the pause");
    assert_eq!(k.editable_focused(1, false, ms(t0, 100 + W)), FocusOutcome::Input);
}

#[test]
fn autofocus_t31_a_new_document_ends_the_window() {
    // Enter on a link (or gi) loads a page that autofocuses a field within CLICK_FOCUS_MS: the new
    // document's focus is its own. (State forgets its mouse press at the same load start.)
    let t0 = Instant::now();
    let mut k = keys();
    tap(&mut k, KeyInput::sym(Keysym::Return), t0);
    k.navigated();
    assert_eq!(page_focus(&mut k, ms(t0, 200)), FocusOutcome::Parked);
    assert_eq!(k.key_mode, KeyMode::Normal);
}

#[test]
fn autofocus_scroll_keys_do_not_count_as_given_to_the_page() {
    let t0 = Instant::now();
    let mut k = keys();
    tap(&mut k, KeyInput::ch('j'), t0);
    effects(&mut k);
    assert_eq!(page_focus(&mut k, ms(t0, 10)), FocusOutcome::Parked);
}

#[test]
fn autofocus_page_focus_in_input_changes_nothing() {
    let t0 = Instant::now();
    let mut k = keys();
    k.set_key_mode(KeyMode::Input);
    assert_eq!(page_focus(&mut k, t0), FocusOutcome::AlreadyInput);
    assert!(effects(&mut k).is_empty(), "typing in INPUT: the page may move its focus");
    assert!(!k.buffers.current().parked);
}

#[test]
fn autofocus_t29_t68_an_open_line_or_picker_stays_open_on_a_page_focus() {
    let t0 = Instant::now();
    for opener in [':', '/'] {
        let mut k = keys();
        tap(&mut k, KeyInput::ch(opener), t0);
        tap(&mut k, KeyInput::ch('x'), t0);
        let mode = k.vim_mode;
        assert_eq!(page_focus(&mut k, ms(t0, 2 * W)), FocusOutcome::Parked);
        assert_eq!((k.key_mode, k.vim_mode), (KeyMode::Normal, mode), "{opener}: not cancelled");
        assert_eq!(k.line.text, "x");
        // A click into a field still cancels it (T29).
        assert_eq!(k.editable_focused(1, true, ms(t0, 5 * W)), FocusOutcome::Input);
        assert_eq!(k.vim_mode, VimMode::Normal);
    }
}

// ---------- the refocus guard ----------

#[test]
fn autofocus_refocus_guard_blurs_at_most_once_per_window() {
    let t0 = Instant::now();
    let mut k = keys();
    assert_eq!(page_focus(&mut k, t0), FocusOutcome::Parked);
    assert_eq!(effects(&mut k), vec![blur(1)]);
    // The page focuses the field again on blur: left focused, NORMAL.
    assert_eq!(page_focus(&mut k, ms(t0, 20)), FocusOutcome::Kept);
    assert_eq!(page_focus(&mut k, ms(t0, W)), FocusOutcome::Kept);
    assert!(effects(&mut k).is_empty(), "no blur loop");
    assert_eq!(k.key_mode, KeyMode::Normal);
    assert!(
        !k.buffers.current().parked,
        "a later editable focus ends the park; the field has the focus"
    );
    // A window later the page's next focus is parked again.
    assert_eq!(page_focus(&mut k, ms(t0, W + 1)), FocusOutcome::Parked);
    assert_eq!(effects(&mut k), vec![blur(1)]);
}

#[test]
fn autofocus_the_guard_is_per_buffer_and_a_load_resets_it() {
    let t0 = Instant::now();
    let mut k = keys();
    page_focus(&mut k, t0);
    effects(&mut k);
    load_start(&mut k, 1);
    assert!(!k.buffers.current().parked, "a cross-document load ends the park");
    assert_eq!(
        page_focus(&mut k, ms(t0, 10)),
        FocusOutcome::Parked,
        "the new document's autofocus is parked too"
    );
}

// ---------- i: restore ----------

#[test]
fn autofocus_t6_i_focuses_the_parked_field_again() {
    let t0 = Instant::now();
    let mut k = keys();
    page_focus(&mut k, t0);
    effects(&mut k);
    tap(&mut k, KeyInput::ch('i'), ms(t0, 50));
    assert_eq!(k.key_mode, KeyMode::Input);
    assert_eq!(effects(&mut k), vec![focus(FieldWhich::Last)]);
    assert!(!k.buffers.current().parked);
    // Its focus event arrives in INPUT: nothing more.
    assert_eq!(page_focus(&mut k, ms(t0, 60)), FocusOutcome::AlreadyInput);
    // Esc Esc, then i: nothing to restore.
    esc_esc(&mut k, ms(t0, 100));
    tap(&mut k, KeyInput::ch('i'), ms(t0, 200));
    assert_eq!(k.key_mode, KeyMode::Input);
    assert!(effects(&mut k).is_empty());
}

#[test]
fn autofocus_t6_i_without_a_park_only_switches() {
    let mut k = keys();
    tap(&mut k, KeyInput::ch('i'), Instant::now());
    assert_eq!(k.key_mode, KeyMode::Input);
    assert!(effects(&mut k).is_empty());
}

#[test]
fn autofocus_a_clicked_field_ends_the_park() {
    let t0 = Instant::now();
    let mut k = keys();
    page_focus(&mut k, t0);
    effects(&mut k);
    // The user clicks into another field, types, leaves with Esc Esc.
    assert_eq!(k.editable_focused(1, true, ms(t0, 2 * W)), FocusOutcome::Input);
    esc_esc(&mut k, ms(t0, 3 * W));
    // i does not jump back to the old autofocused field.
    tap(&mut k, KeyInput::ch('i'), ms(t0, 4 * W));
    assert!(effects(&mut k).is_empty());
}

#[test]
fn autofocus_a_load_ends_the_park() {
    let t0 = Instant::now();
    let mut k = keys();
    page_focus(&mut k, t0);
    effects(&mut k);
    load_start(&mut k, 1);
    tap(&mut k, KeyInput::ch('i'), ms(t0, 10));
    assert!(effects(&mut k).is_empty(), "the old document's field is gone");
}

// ---------- background buffers ----------

#[test]
fn autofocus_c4_a_background_buffer_is_parked_and_the_mode_stays() {
    let t0 = Instant::now();
    let mut k = keys();
    k.open_buffer("https://b.test/");
    assert_eq!(k.buffers.tab_created(2, Some(1)), TabCreated::Buffer(2));
    effects(&mut k);
    assert_eq!(k.buffers.current, 2);
    // Page 1 (hidden) focuses a field, even right after a click on page 2.
    assert_eq!(k.editable_focused(1, true, t0), FocusOutcome::Parked);
    assert_eq!(effects(&mut k), vec![blur(1)]);
    assert_eq!(k.key_mode, KeyMode::Normal);
    assert!(k.buffers.get(1).unwrap().parked);
    assert!(!k.buffers.current().parked);
    // Back on buffer 1, i restores it.
    k.switch_to(1);
    effects(&mut k);
    tap(&mut k, KeyInput::ch('i'), ms(t0, 10));
    assert_eq!(effects(&mut k), vec![focus(FieldWhich::Last)]);
    // A page with no buffer: ignored.
    assert_eq!(k.editable_focused(9, false, t0), FocusOutcome::Ignored);
}

// ---------- gi ----------

#[test]
fn autofocus_gi_is_a_sequence_and_asks_for_the_last_field() {
    assert!(KEY_SEQUENCES
        .iter()
        .any(|s| s.first == 'g' && s.second == 'i' && s.action == Action::FocusField));
    let t0 = Instant::now();
    let mut k = keys();
    assert_eq!(tap(&mut k, KeyInput::ch('g'), t0), (false, false));
    assert_eq!(tap(&mut k, KeyInput::ch('i'), t0), (false, false));
    assert_eq!(k.key_mode, KeyMode::Normal, "INPUT only when the focus arrives");
    assert_eq!(effects(&mut k), vec![focus(FieldWhich::Last)]);
    // The focus arrives: user-caused.
    assert_eq!(k.editable_focused(1, false, ms(t0, 30)), FocusOutcome::Input);
    k.field_result(1, "last", true);
    assert_eq!(k.key_mode, KeyMode::Input);
    assert!(effects(&mut k).is_empty());
    // gg still scrolls.
    let mut k = keys();
    tap(&mut k, KeyInput::ch('g'), t0);
    tap(&mut k, KeyInput::ch('g'), t0);
    assert!(matches!(effects(&mut k).as_slice(), [Effect::KeyDown(_), Effect::KeyUp(_)]));
}

#[test]
fn autofocus_gi_on_a_field_that_already_has_the_focus_enters_input() {
    // After Esc Esc out of a field (still focused) no `editable-focused` comes: the result does it.
    let t0 = Instant::now();
    let mut k = keys();
    tap(&mut k, KeyInput::ch('g'), t0);
    tap(&mut k, KeyInput::ch('i'), t0);
    effects(&mut k);
    k.field_result(1, "last", true);
    assert_eq!(k.key_mode, KeyMode::Input);
}

#[test]
fn autofocus_gi_falls_back_to_the_first_field_then_says_none() {
    let t0 = Instant::now();
    let mut k = keys();
    tap(&mut k, KeyInput::ch('g'), t0);
    tap(&mut k, KeyInput::ch('i'), t0);
    effects(&mut k);
    k.field_result(1, "last", false);
    assert_eq!(effects(&mut k), vec![focus(FieldWhich::First)]);
    assert_eq!(k.key_mode, KeyMode::Normal);
    k.field_result(1, "first", true);
    assert_eq!(k.key_mode, KeyMode::Input);
    // No field at all.
    let mut k = keys();
    tap(&mut k, KeyInput::ch('g'), t0);
    tap(&mut k, KeyInput::ch('i'), t0);
    k.field_result(1, "last", false);
    k.field_result(1, "first", false);
    assert_eq!(k.key_mode, KeyMode::Normal, "mode unchanged");
    assert_eq!(k.message, GI_NO_FIELD);
    // A later stray answer changes nothing.
    k.message.clear();
    k.field_result(1, "first", false);
    assert!(k.message.is_empty());
}

#[test]
fn autofocus_field_results_other_than_gis_change_nothing() {
    let t0 = Instant::now();
    let mut k = keys();
    // i's restore answer, a blur's answer: no mode change, no message.
    k.field_result(1, "last", false);
    k.field_result(1, "blur", true);
    k.field_result(1, "last", true);
    assert_eq!(k.key_mode, KeyMode::Normal);
    assert!(k.message.is_empty() && k.effects.is_empty());
    // gi's answer from another page is not the current one's.
    tap(&mut k, KeyInput::ch('g'), t0);
    tap(&mut k, KeyInput::ch('i'), t0);
    effects(&mut k);
    k.field_result(2, "last", true);
    assert_eq!(k.key_mode, KeyMode::Normal);
}

#[test]
fn autofocus_gi_window_ends() {
    let t0 = Instant::now();
    let mut k = keys();
    tap(&mut k, KeyInput::ch('g'), t0);
    tap(&mut k, KeyInput::ch('i'), t0);
    k.field_result(1, "first", false);
    effects(&mut k);
    assert_eq!(page_focus(&mut k, ms(t0, W + 1)), FocusOutcome::Parked, "an old gi is not a cause");
}
