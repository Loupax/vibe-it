//! C-NORMALMODS: a click in NORMAL carries the modifiers held (KEYS.md T92); NORMAL's keys stay unmodified (T21/T33).

use crate::input::{click_mods, ClickMods};

#[test]
fn keys_t92_a_normal_press_with_a_modifier_sends_it_and_the_release_restores() {
    assert_eq!(click_mods(false, true, true, 0), ClickMods::Send);
    assert_eq!(click_mods(false, false, true, 1), ClickMods::Restore);
}

#[test]
fn keys_t92_a_plain_click_in_normal_touches_nothing() {
    assert_eq!(click_mods(false, true, false, 0), ClickMods::Nothing);
    assert_eq!(click_mods(false, false, false, 0), ClickMods::Nothing);
}

#[test]
fn keys_t92_input_already_has_the_modifiers_but_a_release_still_pairs() {
    assert_eq!(click_mods(true, true, true, 0), ClickMods::Nothing);
    // pressed in NORMAL, released after a click moved the shell to INPUT (T29): still restored
    assert_eq!(click_mods(true, false, true, 1), ClickMods::Restore);
}
