//! C-FOCUSKEYS (the page's pressed set is released on focus-out) and C3-SHIFTSYNC (the desktop's
//! modifier masks are the shell's modifier state).

use crate::keymap::{mods_for_masks, same_mods};
use crate::keys::*;
use smithay::input::keyboard::{xkb, Keysym};
use std::time::Instant;

fn keys() -> Keys {
    let mut k = Keys::new(None, "file:///help.html".into(), None);
    k.set_key_mode(KeyMode::Input);
    k
}

const ALT: u32 = 56;
const A: u32 = 30;

#[test]
fn keys_t33_focus_out_releases_exactly_what_the_page_has_pressed() {
    let mut k = keys();
    let t = Instant::now();
    assert!(k.key(ALT, true, &KeyInput::sym(Keysym::Alt_L), t));
    assert!(k.key(A, true, &KeyInput::ch('a').alt(), t));
    assert!(k.key(A, false, &KeyInput::ch('a'), t), "an ordinary release still pairs");
    assert_eq!(k.release_all(), (vec![ALT], vec![]), "only Alt is still pressed in the page");
    // The late physical release of Alt (after refocus) is not delivered: it was released already.
    assert!(!k.key(ALT, false, &KeyInput::sym(Keysym::Alt_L), t));
    assert_eq!(k.release_all(), (vec![], vec![]));
}

#[test]
fn keys_t34_focus_out_does_not_release_what_the_page_never_got() {
    let mut k = keys();
    k.set_key_mode(KeyMode::Normal);
    let t = Instant::now();
    // `j` in NORMAL scrolls: held by the shell, not forwarded to the page.
    assert!(!k.key(36, true, &KeyInput::ch('j'), t));
    assert_eq!(
        k.release_all(),
        (vec![], vec![36]),
        "nothing was pressed in the page; the shell's own key is dropped"
    );
    assert!(
        k.effects.iter().any(|e| matches!(e, Effect::KeyUp(_))),
        "the scroll key held for the binding is released"
    );
    assert!(k.idle());
}

#[test]
fn keys_focus_in_starts_clean() {
    let mut k = keys();
    let t = Instant::now();
    assert!(k.key(A, true, &KeyInput::ch('a'), t));
    assert!(!k.idle());
    k.release_all();
    assert!(k.idle());
    // A press of the same key now is a fresh press, not an OS repeat.
    assert!(k.key(A, true, &KeyInput::ch('a'), t));
}

fn keymap() -> xkb::Keymap {
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::Keymap::new_from_names(&ctx, "", "", "us", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS).unwrap()
}

#[test]
fn shiftsync_masks_become_modifiers() {
    let km = keymap();
    let shift = 1 << km.mod_get_index(xkb::MOD_NAME_SHIFT);
    let alt = 1 << km.mod_get_index(xkb::MOD_NAME_ALT);
    let caps = 1 << km.mod_get_index(xkb::MOD_NAME_CAPS);
    let m = mods_for_masks(&km, shift, 0, 0);
    assert!(m.shift && !m.alt && !m.ctrl);
    let m = mods_for_masks(&km, alt, 0, caps);
    assert!(m.alt && m.caps_lock && !m.shift);
    let none = mods_for_masks(&km, 0, 0, 0);
    assert!(!none.shift && !none.alt && !none.caps_lock);
    assert!(same_mods(&none, &mods_for_masks(&km, 0, 0, 0)));
    assert!(!same_mods(&none, &mods_for_masks(&km, shift, 0, 0)));
}

#[test]
fn shiftsync_shift_from_masks_makes_semicolon_a_colon() {
    // What keys.rs sees: the modified keysym of `;` under the synced state.
    let km = keymap();
    let shift = 1 << km.mod_get_index(xkb::MOD_NAME_SHIFT);
    let mut st = xkb::State::new(&km);
    let semi = xkb::Keycode::new(39 + 8);
    assert_eq!(st.key_get_one_sym(semi), Keysym::semicolon);
    st.update_mask(shift, 0, 0, 0, 0, 0);
    assert_eq!(st.key_get_one_sym(semi), Keysym::colon);
}
