//! web's configuration: edit and rebuild; every new user-tunable knob goes here.
//!
//! dwm `config.h` style (KEYS.md "Configuration", carried over from scratchy's
//! `src/config.rs`): plain constants and tables, no runtime config file. Each knob has a
//! one-line comment. To add a binding, add a row to `BINDINGS`; to add a knob, add a
//! `pub const` below with a comment, then read it from the code that needs it. Keep this
//! file free of logic beyond the tiny `Binding` helpers.

use smithay::input::keyboard::Keysym;

use crate::keys::{Action, KeyInput, KeyMode};

// ---------- key bindings ----------

/// Which key a `Binding` matches.
#[derive(Clone, Copy, Debug)]
pub enum KeySpec {
    /// A character key, as the keymap gives it with Shift applied (`"G"`, `":"`; so no
    /// `Shift` condition is needed for those).
    Char(char),
    /// A non-character key (arrows, Home, Space, ...).
    Sym(Keysym),
}

/// The modifier condition of a `Binding`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mods {
    /// Fires whatever modifiers are held.
    Any,
    /// Fires only while Ctrl is held.
    Ctrl,
    /// Fires only while Shift is held.
    Shift,
    /// Fires only with no modifier held (no Ctrl, Alt, Super or Shift).
    None,
}

/// One NORMAL-mode key binding: key, modifier condition, resulting action.
#[derive(Clone, Copy, Debug)]
pub struct Binding {
    pub key: KeySpec,
    pub mods: Mods,
    pub action: Action,
}

impl Binding {
    /// Whether this row matches a key press.
    pub fn matches(&self, key: &KeyInput) -> bool {
        let key_ok = match self.key {
            KeySpec::Char(c) => key.ch == Some(c),
            KeySpec::Sym(s) => key.sym == s,
        };
        key_ok
            && match self.mods {
                Mods::Any => true,
                Mods::Ctrl => key.ctrl,
                Mods::Shift => key.shift,
                Mods::None => !key.ctrl && !key.alt && !key.logo && !key.shift,
            }
    }
}

const fn bind(key: KeySpec, mods: Mods, action: Action) -> Binding {
    Binding { key, mods, action }
}

use Action as A;
use KeySpec::{Char, Sym};

/// NORMAL-mode key bindings. The FIRST matching row wins, so a more specific row (a
/// modifier condition) must come before a more general one on the same key (Shift+Space
/// before Space). `A::Scroll(n)` is row `n` of `SCROLL_INPUTS` below.
pub const BINDINGS: &[Binding] = &[
    bind(Char('j'), Mods::Any, A::Scroll(DOWN)),
    bind(Char('k'), Mods::Any, A::Scroll(UP)),
    bind(Char('d'), Mods::Ctrl, A::Scroll(PAGE_DOWN)),
    bind(Char('u'), Mods::Ctrl, A::Scroll(PAGE_UP)),
    bind(Char(':'), Mods::Any, A::EnterCommandMode),
    bind(Char('/'), Mods::Any, A::SearchOpen),
    bind(Sym(Keysym::Escape), Mods::Any, A::ClearSearch),
    bind(Char('n'), Mods::Any, A::SearchNext),
    bind(Char('N'), Mods::Any, A::SearchPrev),
    bind(Char('i'), Mods::Any, A::EnterInputMode),
    bind(Char('h'), Mods::Any, A::HistoryBack),
    bind(Char('l'), Mods::Any, A::HistoryForward),
    bind(Char('G'), Mods::Any, A::Scroll(BOTTOM)),
    // Ctrl-^ (C4): the previous buffer. On a US layout it is Ctrl-Shift-6, which the keymap gives
    // as `^`; vim also takes Ctrl-6.
    bind(Char('^'), Mods::Ctrl, A::AlternateBuffer),
    bind(Char('6'), Mods::Ctrl, A::AlternateBuffer),
    bind(Sym(Keysym::Down), Mods::Any, A::Scroll(DOWN)),
    bind(Sym(Keysym::Up), Mods::Any, A::Scroll(UP)),
    bind(Sym(Keysym::Left), Mods::Any, A::Scroll(LEFT)),
    bind(Sym(Keysym::Right), Mods::Any, A::Scroll(RIGHT)),
    bind(Sym(Keysym::space), Mods::Shift, A::Scroll(PAGE_UP)),
    bind(Sym(Keysym::space), Mods::Any, A::Scroll(PAGE_DOWN)),
    bind(Sym(Keysym::Page_Down), Mods::Any, A::Scroll(PAGE_DOWN)),
    bind(Sym(Keysym::Page_Up), Mods::Any, A::Scroll(PAGE_UP)),
    bind(Sym(Keysym::Home), Mods::Any, A::Scroll(TOP)),
    bind(Sym(Keysym::End), Mods::Any, A::Scroll(BOTTOM)),
    // Enter goes to the page (T18, user 2026-11-23): the engine acts on what has focus. With a
    // `/` search open the shell ends it first (T7), which focuses the match's link, so Enter
    // follows it. A modified Enter does nothing (T21).
    bind(Sym(Keysym::Return), Mods::None, A::ToPage),
    bind(Sym(Keysym::KP_Enter), Mods::None, A::ToPage),
];

/// A two-key NORMAL sequence: `first` arms, `second` fires `action` (KEYS.md T10-T12,
/// T54-T56). Any other key clears the pending state and is handled as in NORMAL.
#[derive(Clone, Copy, Debug)]
pub struct KeySeq {
    pub first: char,
    pub second: char,
    pub action: Action,
}

/// The two-key sequences (character keys, no Ctrl/Alt).
pub const KEY_SEQUENCES: &[KeySeq] = &[
    // `gg`: scroll to the top.
    KeySeq {
        first: 'g',
        second: 'g',
        action: Action::Scroll(TOP),
    },
    // `gi` (C-AUTOFOCUS, as Vimium): focus the last text field, else the first visible one; INPUT.
    KeySeq {
        first: 'g',
        second: 'i',
        action: Action::FocusField,
    },
    // `yy`: copy the URL (`:let @+ = @%`); needs the clipboard (C6), says so until then.
    KeySeq {
        first: 'y',
        second: 'y',
        action: Action::YankUrl,
    },
];

// ---------- scrolling (KEYS.md T13, web note) ----------

/// What a scroll binding sends to the page: real input, never a page API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollInput {
    /// A key press given to the page window (evdev key code, below), held for as long as
    /// the bound key is held, so the engine's own key repeat keeps scrolling.
    Key(u32),
    /// Wheel steps (positive = down) at the pointer; only when the pointer is over the
    /// page (the wheel goes to the surface under it). One press, one event.
    #[allow(dead_code)] // only used by a user's SCROLL_INPUTS row (and the tests)
    Wheel(i32),
}

/// evdev key codes (linux/input-event-codes.h) for `ScrollInput::Key`.
pub mod evdev {
    pub const KEY_HOME: u32 = 102;
    pub const KEY_UP: u32 = 103;
    pub const KEY_PAGEUP: u32 = 104;
    pub const KEY_LEFT: u32 = 105;
    pub const KEY_RIGHT: u32 = 106;
    pub const KEY_END: u32 = 107;
    pub const KEY_DOWN: u32 = 108;
    pub const KEY_PAGEDOWN: u32 = 109;
    pub const KEY_ENTER: u32 = 28;
}

pub const DOWN: usize = 0;
pub const UP: usize = 1;
pub const PAGE_DOWN: usize = 2;
pub const PAGE_UP: usize = 3;
pub const TOP: usize = 4;
pub const BOTTOM: usize = 5;
pub const LEFT: usize = 6;
pub const RIGHT: usize = 7;

/// The scroll table, indexed by the constants above. Chromium has no half-page key, so
/// Ctrl-d / Ctrl-u scroll a page (`PAGE_DOWN`/`PAGE_UP`); for half a page put e.g.
/// `ScrollInput::Wheel(5)` in a row of its own and bind Ctrl-d to it.
/// Note: a key goes to whatever has focus in the page, so `j` in NORMAL moves the caret
/// of a text field (T29/T32: focus switches to INPUT, so rarely); `Wheel` rows do not.
pub const SCROLL_INPUTS: &[ScrollInput] = &[
    ScrollInput::Key(evdev::KEY_DOWN),     // DOWN
    ScrollInput::Key(evdev::KEY_UP),       // UP
    ScrollInput::Key(evdev::KEY_PAGEDOWN), // PAGE_DOWN
    ScrollInput::Key(evdev::KEY_PAGEUP),   // PAGE_UP
    ScrollInput::Key(evdev::KEY_HOME),     // TOP
    ScrollInput::Key(evdev::KEY_END),      // BOTTOM
    ScrollInput::Key(evdev::KEY_LEFT),     // LEFT
    ScrollInput::Key(evdev::KEY_RIGHT),    // RIGHT
];

// ---------- modes ----------

/// Esc Esc: the second Esc must come within this many ms of the first (INPUT -> NORMAL).
/// A QMK user can swap the trigger for F13 by editing `resolve_esc_esc`'s key test.
pub const ESC_ESC_MS: u64 = 300;

/// The key mode a fresh window starts in (user 2026-11-21, scratchy QKEYS5: NORMAL).
/// Navigations keep the current mode; a click into a text control still switches to INPUT.
pub const START_MODE: KeyMode = KeyMode::Normal;

/// T29: a new popup (a `<select>` menu, a context menu) switches to INPUT only this many ms
/// after a mouse button press; later it was the page's own script and changes nothing.
pub const CLICK_FOCUS_MS: u64 = 1000;

/// T18: Enter in NORMAL with a find open ends the find (Chromium then focuses the match's link
/// or control) and gives the page the Enter this many ms later, so it lands on that focus.
pub const ENTER_AFTER_FIND_CLEAR_MS: u64 = 50;

/// C-BARREPEAT: winit (smithay's backend) drops the OS's key repeats, so the shell repeats a held key
/// in its own `:` and `/` lines and the picker's filter itself. These are the rate (keys per second;
/// 0 = no repeat) and delay (ms before the first repeat) used until the desktop tells its own
/// (wl_keyboard.repeat_info on Wayland, XKB controls on X11).
pub const KEY_REPEAT_RATE: u32 = 25;
pub const KEY_REPEAT_DELAY_MS: u32 = 600;

// ---------- status bar ----------

/// Height of the status bar at the bottom of the window, px. The page is the window minus this.
pub const STATUS_BAR_H: i32 = 24;
/// Horizontal padding inside the status bar, px.
pub const STATUS_BAR_PAD_X: f32 = 6.0;
/// Space between the mode, the URL and the message, px.
pub const STATUS_BAR_GAP: f32 = 12.0;
/// Least room the URL keeps when a message is shown, px.
pub const STATUS_BAR_MIN_URL_W: f32 = 160.0;
/// Font size of the status bar text, px.
pub const STATUS_BAR_FONT_PX: f32 = 14.0;
/// Status bar background, RGB.
pub const STATUS_BAR_BG: (u8, u8, u8) = (24, 24, 24);
/// Status bar text colour (URL, message, the `:` line), RGB.
pub const STATUS_BAR_FG: (u8, u8, u8) = (225, 225, 225);
/// Status bar mode-name colour (`INPUT` / `NORMAL`), RGB.
pub const STATUS_BAR_MODE_FG: (u8, u8, u8) = (120, 200, 255);
/// Ghost (suggestion) text colour in the `:` line, RGB (dimmed).
pub const GHOST_FG: (u8, u8, u8) = (120, 120, 120);

// ---------- notifications (C-BELL, KEYS.md "Notifications") ----------

/// Which sites' Web Notifications reach the shell (never the desktop either way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifySites {
    /// Every site: pages see permission "granted" without asking.
    All,
    /// Only these hostnames, exactly (`"chat.example.com"`; no subdomains); every other site sees
    /// "denied". `Only(&[])`: no site.
    #[allow(dead_code)] // only used by a user's NOTIFY_SITES (and the tests)
    Only(&'static [&'static str]),
}

/// Sites allowed to notify (DECISIONS.md "Web Notifications go to the status bar"). Read when the
/// engine host starts: a change needs a rebuild and a restart.
pub const NOTIFY_SITES: NotifySites = NotifySites::All;
/// The visual bell: how long the status bar shows the bell colours, ms (0 = no bell).
pub const BELL_FLASH_MS: u64 = 150;
/// No second flash within this many ms of the last one (a chatty page flashes at most this often).
pub const BELL_MIN_INTERVAL_MS: u64 = 1000;
/// Status bar background while the bell is lit, RGB (the mode colour: an inverted bar).
pub const BELL_BG: (u8, u8, u8) = (120, 200, 255);
/// Status bar text colour while the bell is lit, RGB.
pub const BELL_FG: (u8, u8, u8) = (24, 24, 24);
/// Unread notifications kept per buffer (the oldest go).
pub const NOTIFY_UNREAD_MAX: usize = 20;
/// C-NOTIFLIST: notifications `:notifications` keeps this session (memory only; the oldest go).
pub const NOTIFY_LIST_MAX: usize = 100;

// ---------- picker (C-PICK, KEYS.md "Picker") ----------

/// At most this many rows are shown (fewer when the page is shorter); the list scrolls.
pub const PICKER_MAX_ROWS: usize = 12;
/// Height of one picker row (and of its header), logical px.
pub const PICKER_ROW_H: i32 = 22;
/// Picker background, RGB.
pub const PICKER_BG: (u8, u8, u8) = (36, 36, 36);
/// The selected row's background, RGB.
pub const PICKER_SEL_BG: (u8, u8, u8) = (60, 72, 96);
/// The source's name in the header, RGB.
pub const PICKER_HEADER_FG: (u8, u8, u8) = (225, 225, 225);

// ---------- command line and completion ----------

/// Arguments `:refresh` completes (`hard` bypasses the cache).
pub const REFRESH_ARGS: &[&str] = &["hard"];

/// Arguments `:set` completes (C-CARETCMD): `caret` turns Chromium's caret browsing on,
/// `nocaret` off, `caret?` shows which. An exact word comes before its longer forms, so the
/// ghost of a typed `caret` is empty and Enter runs `:set caret`.
pub const SET_ARGS: &[&str] = &["caret", "caret?", "nocaret"];

/// Where a command's arguments complete from (KEYS.md "Completion").
#[derive(Clone, Copy)]
pub enum Source {
    /// A Rust function: given the line, the word and its argument index, the candidates
    /// (full words that start with the word), best first.
    Builtin(fn(&crate::completion::CompleteCtx) -> Vec<String>),
    /// An external program (argv, run with NO shell, so `$(...)` and `;` are plain text). It
    /// gets the context in its environment: WEB_LINE (the whole line), WEB_WORD (the word
    /// being completed), WEB_ARG (its index, 1 = first argument), WEB_URL (the current
    /// page), WEB_STATE_DIR. It prints one candidate per line on stdout; a non-zero exit or
    /// no output means no ghost. It runs on a thread once per command line and argument
    /// position (not per key) and is killed after `SOURCE_TIMEOUT_MS`.
    #[allow(dead_code)] // only used by a user's COMPLETERS row (and the tests)
    Cmd(&'static [&'static str]),
}

/// The completion table, one row per command (the first word always completes from the
/// command names). No row = no ghost after that command's name. `:go` completes from the
/// history file. To use your own program, replace a row, e.g.
/// `("go", Source::Cmd(&["my-url-source", "--flag"]))`.
pub const COMPLETERS: &[(&str, Source)] = &[
    ("go", Source::Builtin(crate::completion::complete_history_urls)),
    ("new", Source::Builtin(crate::completion::complete_history_urls)),
    ("bufferNew", Source::Builtin(crate::completion::complete_history_urls)),
    ("openExternal", Source::Builtin(crate::completion::complete_history_urls)),
    ("bookmarkDelete", Source::Builtin(crate::completion::complete_bookmark_urls)),
    ("refresh", Source::Builtin(crate::completion::complete_refresh_args)),
    ("set", Source::Builtin(crate::completion::complete_set_args)),
    // ("go", Source::Cmd(&["my-url-source"])),
];

/// C-EXTERNAL: the program `:openExternal` runs, as an argv with no shell (`&["firefox", "--new-tab", "%s"]`;
/// `%s` becomes the URL, else the URL is appended as the last argument). Empty = `$BROWSER` (entries
/// separated by `:`, the first whose program exists; an entry may carry arguments and `%s`), else `xdg-open`.
pub const EXTERNAL_BROWSER: &[&str] = &[];

/// An external completion source still running after this many milliseconds is killed and
/// gives no candidates.
pub const SOURCE_TIMEOUT_MS: u64 = 2000;
/// At most this many bytes of an external source's stdout are read; the rest is dropped.
pub const SOURCE_MAX_OUTPUT: usize = 1 << 20;

/// Commands whose argument, once selected with Tab (a single match), is followed by a
/// space (T53). The command name itself always gets one; a URL never.
pub const SPACE_AFTER_ARG: &[&str] = &["set"];

// ---------- history (STASH.md "History") ----------

/// Record visits in `DIR/history` (the `:go` completion reads it). false = never write the file.
pub const HISTORY: bool = true;
/// At startup a history file with more lines than this keeps only the newest ones.
pub const HISTORY_MAX_LINES: usize = 50_000;

/// C-CMDHIST: the `:` lines kept (memory and `DIR/cmdhistory`); vim's 'history' option.
pub const CMD_HISTORY_MAX: usize = 1000;

// ---------- user scripts (SCRIPTING.md, C-SCRIPT) ----------

/// Every call into a user script (the file itself, a user command, the `ready` handler) may run this
/// many milliseconds; QuickJS's interrupt handler then ends it with an error in the status bar.
pub const SCRIPT_BUDGET_MS: u64 = 100;
/// `web.cmd` calls may nest through user commands this deep (`:A` running `:B` running ...); deeper,
/// the chain stops with an error.
pub const SCRIPT_DEPTH_MAX: usize = 16;
/// C-SCRIPT-MEM: the script runtime's heap limit (QuickJS `set_memory_limit`). A script that allocates past it
/// gets QuickJS's out-of-memory error (status bar, log) and web keeps running. 64 MiB: a config script holds
/// kilobytes (a 100k-entry array is ~2 MiB), so this is ~30x any real use, and a runaway loop is stopped at
/// a size that costs the shell nothing (the 100 ms budget alone allowed a few hundred MB).
pub const SCRIPT_MEMORY_MAX: usize = 64 * 1024 * 1024;
/// C-SCRIPT-MAP: while the keys typed in NORMAL are the start of a longer `web.map` mapping, the shell
/// waits this many milliseconds for the next key (vim's `timeoutlen`, same default); then the keys are
/// handled as without the mapping (or a shorter mapping they complete runs). With no mapping sharing a
/// prefix with what is typed, nothing waits.
pub const MAP_TIMEOUT_MS: u64 = 1000;
