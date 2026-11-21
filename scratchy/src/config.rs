//! scratchy's configuration: edit and rebuild; every new user-tunable knob goes here.
//!
//! dwm `config.h` style (user, 2026-11-20, KEYS.md "Configuration"): plain constants and tables,
//! no runtime config file. Each knob has a one-line comment. To add a binding, add a row to
//! `BINDINGS`; to add a knob, add a `pub(crate) const` below with a comment, then read it from
//! the code that needs it. Keep this file free of logic beyond the tiny `Binding` helpers.
//! (Tracker block/allow lists are the exception and stay in QCFG1's runtime file.)

use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::Action;

// ---------- key bindings ----------

/// Which key a `Binding` matches.
#[derive(Clone, Copy, Debug)]
pub(crate) enum KeySpec {
    /// A character key, exactly as winit's shift-aware `logical_key` reports it
    /// (`"G"`, `":"`; so no `Shift` requirement is needed for those).
    Char(&'static str),
    /// A named key (arrows, Home, Space, ...).
    Named(NamedKey),
}

/// The modifier condition of a `Binding`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mods {
    /// Fires whatever modifiers are held.
    Any,
    /// Fires only while Ctrl is held.
    Ctrl,
    /// Fires only while Shift is held.
    Shift,
}

/// One Normal-mode key binding: key, modifier condition, resulting action.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Binding {
    pub key: KeySpec,
    pub mods: Mods,
    pub action: Action,
}

impl Binding {
    /// Whether this row matches a key press.
    pub(crate) fn matches(&self, key: &Key, mods: ModifiersState) -> bool {
        let key_ok = match (self.key, key) {
            (KeySpec::Char(c), Key::Character(s)) => s.as_str() == c,
            (KeySpec::Named(n), Key::Named(k)) => n == *k,
            _ => false,
        };
        key_ok
            && match self.mods {
                Mods::Any => true,
                Mods::Ctrl => mods.control_key(),
                Mods::Shift => mods.shift_key(),
            }
    }
}

const fn bind(key: KeySpec, mods: Mods, action: Action) -> Binding {
    Binding { key, mods, action }
}

use Action as A;
use KeySpec::{Char, Named};

/// Normal-mode key bindings. The FIRST matching row wins, so a more specific row (a modifier
/// condition) must come before a more general one on the same key (Shift+Space before Space).
pub(crate) const BINDINGS: &[Binding] = &[
    bind(Char("j"), Mods::Any, A::ScrollDown),
    bind(Char("k"), Mods::Any, A::ScrollUp),
    bind(Char("d"), Mods::Ctrl, A::HalfPageDown),
    bind(Char("u"), Mods::Ctrl, A::HalfPageUp),
    bind(Char(":"), Mods::Any, A::EnterCommandMode),
    bind(Char("h"), Mods::Any, A::HistoryBack),
    bind(Char("l"), Mods::Any, A::HistoryFront),
    bind(Char("G"), Mods::Any, A::ScrollBottom),
    // QYSCR1: the usual browser scroll keys.
    bind(Named(NamedKey::ArrowDown), Mods::Any, A::ScrollDown),
    bind(Named(NamedKey::ArrowUp), Mods::Any, A::ScrollUp),
    bind(Named(NamedKey::Space), Mods::Shift, A::PageUp),
    bind(Named(NamedKey::Space), Mods::Any, A::PageDown),
    bind(Named(NamedKey::PageDown), Mods::Any, A::PageDown),
    bind(Named(NamedKey::PageUp), Mods::Any, A::PageUp),
    bind(Named(NamedKey::Home), Mods::Any, A::ScrollTop),
    bind(Named(NamedKey::End), Mods::Any, A::ScrollBottom),
    // `gg` (ScrollTop) is the pending-`g` state machine in `resolve_pending_g`, not a row here.
];

// ---------- scrolling ----------

/// Px one line scroll (`j`/`k`/arrows) moves.
pub(crate) const SCROLL_LINE_PX: f32 = 40.0;
/// Px one mouse-wheel `LineDelta` tick moves (matches `SCROLL_LINE_PX`).
pub(crate) const WHEEL_LINE_PX: f32 = 40.0;
/// Page scroll (Space/PageDown) overlap: px of the old view kept visible.
pub(crate) const PAGE_OVERLAP_PX: f32 = 40.0;
/// Shortest page scroll, px (tiny windows).
pub(crate) const PAGE_MIN_PX: f32 = 40.0;
/// Px of context kept around a newly focused control when scrolling it into view.
pub(crate) const SCROLL_INTO_VIEW_MARGIN: f32 = 24.0;

// ---------- scrollbar (QSB1) ----------

/// Width of the overlay scrollbar track, px.
pub(crate) const TRACK_W: f32 = 8.0;
/// Shortest scrollbar thumb, px (never longer than the track).
pub(crate) const MIN_THUMB_H: f32 = 16.0;

// ---------- command bar ----------

/// Arguments `:set` completes (Tab), alphabetical. `dispatch_command` decides what they do.
pub(crate) const SET_ARGS: &[&str] = &["js", "motion", "nojs", "nomotion", "novim", "vim"];
