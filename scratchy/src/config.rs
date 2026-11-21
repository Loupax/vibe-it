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
    // QSEARCH1: `/` opens the search line, Esc clears the highlights (never changes the mode).
    bind(Char("/"), Mods::Any, A::EnterSearchMode),
    bind(Named(NamedKey::Escape), Mods::Any, A::ClearSearch),
    // QSEARCH2: next / previous match (vim's `n` / `N`).
    bind(Char("n"), Mods::Any, A::SearchNext),
    bind(Char("N"), Mods::Any, A::SearchPrev),
    bind(Char("i"), Mods::Any, A::EnterInputMode),
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

// ---------- modes (QKEYS2) ----------

/// Esc Esc: the second Esc must come within this many ms of the first (input -> normal mode).
/// A QMK user can swap the trigger for F13 by editing `resolve_esc_esc`'s key test.
pub(crate) const ESC_ESC_MS: u64 = 300;

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
/// QSB2: holding the mouse on a track: wait this long, then page again every interval (ms).
pub(crate) const SB_REPEAT_DELAY_MS: u64 = 400;
pub(crate) const SB_REPEAT_INTERVAL_MS: u64 = 60;
/// QSB2: track and thumb colours as (r, g, b, a): rest, thumb hovered, thumb pressed (dragging).
pub(crate) const SB_TRACK_RGBA: (u8, u8, u8, u8) = (0, 0, 0, 28);
pub(crate) const SB_THUMB_RGBA: (u8, u8, u8, u8) = (0, 0, 0, 120);
pub(crate) const SB_THUMB_HOVER_RGBA: (u8, u8, u8, u8) = (0, 0, 0, 170);
pub(crate) const SB_THUMB_ACTIVE_RGBA: (u8, u8, u8, u8) = (0, 0, 0, 210);

// ---------- status bar (QKEYS1) ----------

/// Height of the status bar at the bottom of the window, px. The page's viewport is the window
/// minus this.
pub(crate) const STATUS_BAR_H: u32 = 24;
/// Horizontal padding inside the status bar, px.
pub(crate) const STATUS_BAR_PAD_X: f32 = 6.0;
/// Status bar background, RGB.
pub(crate) const STATUS_BAR_BG: (u8, u8, u8) = (24, 24, 24);
/// Status bar text colour (URL, message, the `:` line), RGB.
pub(crate) const STATUS_BAR_FG: (u8, u8, u8) = (225, 225, 225);
/// Status bar mode-name colour (`INPUT` / `NORMAL`), RGB.
pub(crate) const STATUS_BAR_MODE_FG: (u8, u8, u8) = (120, 200, 255);

// ---------- search highlights (QSEARCH1) ----------

/// Every match's highlight colour, RGBA (painted over the page).
pub(crate) const SEARCH_MATCH_RGBA: (u8, u8, u8, u8) = (255, 214, 0, 110);
/// The current match's stronger highlight colour, RGBA.
pub(crate) const SEARCH_CURRENT_RGBA: (u8, u8, u8, u8) = (255, 140, 0, 180);

// ---------- command bar ----------

/// Arguments `:set` completes, alphabetical. `dispatch_command` decides what they do.
pub(crate) const SET_ARGS: &[&str] = &["js", "motion", "nojs", "nomotion", "novim", "vim"];

/// Where a command's arguments complete from (QCOMP1, QCOMP-D C1). Only built-in sources for now;
/// QCOMP2 adds `Cmd(argv)` (an external program).
#[derive(Clone, Copy)]
pub(crate) enum Source {
    /// A Rust function: given the line, the word and its argument index, the candidates (full
    /// words that start with the word), best first.
    Builtin(fn(&crate::completion::CompleteCtx) -> Vec<String>),
}

/// The completion table, one row per command (the first word always completes from the command
/// names). No row = no ghost after that command's name. QCOMP4 adds
/// `("go", Source::Builtin(complete_history_urls))` here (`:go` has no ghost until then).
pub(crate) const COMPLETERS: &[(&str, Source)] = &[("set", Source::Builtin(crate::completion::complete_set_args))];

/// Commands whose argument, once selected with Tab (a single match), is followed by a space
/// (T53: `:set vim ` yes; a URL no). The command name itself always gets one.
pub(crate) const SPACE_AFTER_ARG: &[&str] = &["set"];

/// Ghost (suggestion) text colour in the `:` line, RGB (dimmed).
pub(crate) const GHOST_FG: (u8, u8, u8) = (120, 120, 120);

// ---------- history (QCOMP3, STASH.md "History") ----------

/// Record visits in `DIR/history` (the `:go` completion reads it). false = never write the file.
pub(crate) const HISTORY: bool = true;
/// At startup a history file with more lines than this keeps only the newest ones.
pub(crate) const HISTORY_MAX_LINES: usize = 50_000;
