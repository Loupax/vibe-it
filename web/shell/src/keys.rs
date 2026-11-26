//! The keyboard model (KEYS.md, ported from scratchy's `src/app.rs`): the modes INPUT /
//! NORMAL / COMMAND, Esc Esc, the NORMAL bindings and two-key sequences, the `:` line with
//! its ghost completion, and the commands it runs.
//!
//! Pure: `Keys` knows nothing about Wayland. The compositor (`input.rs`) asks it, for every
//! key event, whether the page gets the event, and carries out the `Effect`s it queues
//! (commands to the engine host, key presses given to the page for scrolling, quit).

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use smithay::input::keyboard::{Keysym, ModifiersState};

use crate::completion::{compute_ghost, CompleteCache, ExtResult, ExtSink, Ghost};
use crate::config::{
    ScrollInput, BINDINGS, CLICK_FOCUS_MS, ESC_ESC_MS, KEY_REPEAT_DELAY_MS, KEY_REPEAT_RATE, KEY_SEQUENCES, MAP_TIMEOUT_MS, SCROLL_INPUTS,
    START_MODE,
};
use crate::protocol::{Command, FieldWhich, FindDir};

/// C-BARREPEAT: the key the shell repeats in a line, and when it repeats next.
struct Repeating {
    code: u32,
    key: KeyInput,
    next: Instant,
}

/// A key press as the bindings see it: the keysym with Shift (and the layout) applied, the
/// character it types (none for Escape, arrows, ...), and the modifiers held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub sym: Keysym,
    pub ch: Option<char>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

impl KeyInput {
    /// From the keymap: `sym` is smithay's `modified_sym()` (Shift applied).
    pub fn from_xkb(sym: Keysym, mods: &ModifiersState) -> KeyInput {
        KeyInput {
            sym,
            ch: sym.key_char().filter(|c| !c.is_control()),
            ctrl: mods.ctrl,
            alt: mods.alt,
            shift: mods.shift,
            logo: mods.logo,
        }
    }

    /// A key with no modifiers (tests).
    #[cfg(test)]
    pub fn sym(sym: Keysym) -> KeyInput {
        KeyInput {
            sym,
            ch: sym.key_char().filter(|c| !c.is_control()),
            ctrl: false,
            alt: false,
            shift: false,
            logo: false,
        }
    }

    /// The key that types `c` (a capital or `:` as the keymap gives it with Shift).
    #[cfg(test)]
    pub fn ch(c: char) -> KeyInput {
        let mut k = KeyInput::sym(Keysym::from_char(c));
        k.shift = c.is_uppercase() || ":?<>{}|\"~!@#$%^&*()_+".contains(c);
        k
    }

    #[cfg(test)]
    pub fn ctrl(mut self) -> KeyInput {
        self.ctrl = true;
        self
    }

    #[cfg(test)]
    pub fn shift(mut self) -> KeyInput {
        self.shift = true;
        self
    }

    #[cfg(test)]
    pub fn alt(mut self) -> KeyInput {
        self.alt = true;
        self
    }

    fn is(&self, sym: Keysym) -> bool {
        self.sym == sym
    }
}

/// KEYS.md "Modes": who gets the keys. `Input` = the page; `Normal` = web's bindings, the
/// page gets no key events. Sticky: only Esc Esc (Input -> Normal), `i` or a click into a
/// text control or select (Normal -> Input) change it. The `:` line (`VimMode::Command`)
/// is a sub-state of `Normal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyMode {
    Input,
    Normal,
}

/// The NORMAL sub-state: the bindings, the `:` line (KEYS.md COMMAND), the `/` line
/// (SEARCH) or the picker (PICK, C-PICK, `picker.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Command,
    Search,
    Pick,
}

/// The Esc Esc recogniser's state (KEYS.md ESC1). `Esc1(t)` = one Esc seen at `t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscState {
    Idle,
    Esc1(Instant),
}

/// KEYS.md T1-T4. Given the state, a key PRESS (`repeat` = OS auto-repeat) and the time
/// now, returns the new state and whether this press completes Esc Esc (enter NORMAL; the
/// caller must not deliver this press to the page). Only a plain Esc (no Ctrl/Alt/Super)
/// counts; a repeat or any other key resets to `Idle`; an Esc later than `ESC_ESC_MS`
/// after the first starts over (T3). The timeout is checked lazily here, no timer.
pub fn resolve_esc_esc(state: EscState, key: &KeyInput, repeat: bool, now: Instant) -> (EscState, bool) {
    let plain_esc = key.is(Keysym::Escape) && !key.ctrl && !key.alt && !key.logo;
    if !plain_esc || repeat {
        return (EscState::Idle, false);
    }
    match state {
        EscState::Esc1(t) if now.saturating_duration_since(t) <= Duration::from_millis(ESC_ESC_MS) => (EscState::Idle, true),
        _ => (EscState::Esc1(now), false),
    }
}

/// What a NORMAL binding does (kept apart from `resolve_binding` so that stays pure).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Row `n` of `config::SCROLL_INPUTS`: real input to the page (T13).
    Scroll(usize),
    /// `:` (T9).
    EnterCommandMode,
    /// `/` (T8).
    SearchOpen,
    /// `n` (T15-T17).
    SearchNext,
    /// `N` (T15-T17).
    SearchPrev,
    /// Esc in NORMAL (T7): clears the search highlights (the query is kept); never changes the mode.
    ClearSearch,
    /// `i` (T6).
    EnterInputMode,
    /// `h` (T14).
    HistoryBack,
    /// `l` (T14).
    HistoryForward,
    /// `yy` (T55): `:let @+ = @%`.
    YankUrl,
    /// Ctrl-^ (C4, KEYS.md "Buffers"): the buffer shown before this one.
    AlternateBuffer,
    /// The key press (and its release) goes to the page (T19: Enter; T18 ends an open find first).
    ToPage,
    /// `gi` (C-AUTOFOCUS, as Vimium): focus the last text field of the page, else the first visible
    /// one, and enter INPUT when the focus arrives.
    FocusField,
    /// `@:` (C-REPEAT, as vim): run the last `:` line again.
    RepeatLine,
    /// `@@` (C-REPEAT, as vim): repeat the last `@:`.
    RepeatRepeat,
}

/// Whether `action` ignores OS key repeat (T14 `h`/`l` "not a repeat", T55).
pub fn action_ignores_repeat(action: Action) -> bool {
    !matches!(action, Action::Scroll(_) | Action::ToPage)
}

/// The one place a key press becomes a NORMAL `Action` (the `config::BINDINGS` table).
pub fn resolve_binding(key: &KeyInput, mode: VimMode) -> Option<Action> {
    if mode != VimMode::Normal {
        return None;
    }
    BINDINGS.iter().find(|b| b.matches(key)).map(|b| b.action)
}

/// The pending-key machine of `config::KEY_SEQUENCES` (`gg`, `yy`; KEYS.md T10-T12,
/// T54-T56): given the pending first key and the key just pressed, returns the action to
/// run, the new pending key, and whether the key was consumed (so `resolve_binding` must
/// not also run on it). A first key arms; its second fires the row; any other key clears
/// the pending state and is handled as in NORMAL, so it may itself arm (`g` `y` leaves `y`
/// pending).
pub fn resolve_sequence(pending: Option<char>, key: &KeyInput) -> (Option<Action>, Option<char>, bool) {
    // C-REPEAT: a modifier's own press (the Shift of the `@` and of the `:` in `@:`) keeps the pending key.
    if pending.is_some() && is_modifier_sym(key.sym) {
        return (None, pending, true);
    }
    let Some(c) = key.ch else { return (None, None, false) };
    if key.ctrl || key.alt {
        return (None, None, false);
    }
    if let Some(row) = pending.and_then(|first| KEY_SEQUENCES.iter().find(|r| r.first == first && r.second == c)) {
        return (Some(row.action), None, true);
    }
    match KEY_SEQUENCES.iter().find(|r| r.first == c) {
        Some(row) => (None, Some(row.first), true),
        None => (None, None, false),
    }
}

// ---------- user mappings (C-SCRIPT-MAP, SCRIPTING.md "web.map") ----------

/// One key of a `web.map` mapping: the character it types (Shift and the layout applied, as
/// `KeyInput::ch`) and whether Ctrl is held. A letter under Ctrl is kept in lower case, so
/// `<C-x>` is also Ctrl-Shift-x (vim cannot tell them apart either).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapKey {
    pub ch: char,
    pub ctrl: bool,
}

impl MapKey {
    fn new(ch: char, ctrl: bool) -> MapKey {
        let ch = if ctrl { ch.to_ascii_lowercase() } else { ch };
        MapKey { ch, ctrl }
    }

    /// The mapping key a press is, if any: one with a character and no Alt/Super. Escape, arrows,
    /// Enter, ... are none, so they never match (and Esc can never be mapped).
    pub fn from_input(key: &KeyInput) -> Option<MapKey> {
        if key.alt || key.logo {
            return None;
        }
        key.ch.map(|c| MapKey::new(c, key.ctrl))
    }
}

/// A modifier key's own press (Shift_L, Control_R, ISO_Level3_Shift, ...): it never takes part in
/// a mapping, so holding Shift for the `T` of `gT` does not break the sequence.
pub fn is_modifier_sym(sym: Keysym) -> bool {
    let raw = sym.raw();
    (0xffe1..=0xffee).contains(&raw) || (0xfe01..=0xfe13).contains(&raw)
}

/// `web.map`'s `keys` (vim's notation): a character stands for itself, `<C-x>` is Ctrl with `x`,
/// `<lt>` is `<` (names are case-insensitive). Anything else in `<...>`, a `<` with no `>`, or no
/// key at all is an error.
pub fn parse_map_keys(keys: &str) -> Result<Vec<MapKey>, String> {
    const KNOWN: &str = "characters, <C-x>, <lt>";
    let mut out = Vec::new();
    let mut rest = keys;
    while let Some(c) = rest.chars().next() {
        if c == '<' {
            let Some(end) = rest.find('>') else {
                return Err(format!("web.map: a lone '<' in {keys:?} (write <lt> for '<')"));
            };
            let name = &rest[1..end];
            let mut chars = name.chars();
            match (chars.next(), chars.next(), chars.next(), chars.next()) {
                _ if name.eq_ignore_ascii_case("lt") => out.push(MapKey::new('<', false)),
                (Some('c' | 'C'), Some('-'), Some(x), None) if !x.is_control() => out.push(MapKey::new(x, true)),
                _ => return Err(format!("web.map: unknown key <{name}> in {keys:?} (known: {KNOWN})")),
            }
            rest = &rest[end + 1..];
        } else {
            if c.is_control() {
                return Err(format!("web.map: a control character in {keys:?} (known: {KNOWN})"));
            }
            out.push(MapKey::new(c, false));
            rest = &rest[c.len_utf8()..];
        }
    }
    if out.is_empty() {
        return Err("web.map: no keys to map".to_string());
    }
    Ok(out)
}

/// A string `rhs` of `web.map`: one `:` line (SCRIPTING.md; DECISIONS.md 2026-11-24 "A mapping's
/// `:` line runs with `<CR>`").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapLine {
    /// The line's text, without the leading `:`, `<lt>` already `<`, without the `<CR>`.
    pub text: String,
    /// It ended in `<CR>` (`<Enter>`, `<Return>`; any case): it runs. Otherwise the `:` line opens
    /// with `text` typed.
    pub run: bool,
}

/// Parses a string `rhs`. It must start with `:`; `<CR>` (`<Enter>`, `<Return>`) only at the very
/// end; `<lt>` is `<`; any other `<...>`, a `<` with no `>`, or a `<CR>` before the end is an error.
pub fn parse_map_rhs(rhs: &str) -> Result<MapLine, String> {
    let Some(body) = rhs.strip_prefix(':') else {
        return Err(format!(
            "web.map: a string rhs is a ':' line, {rhs:?} does not start with ':' (or pass a function)"
        ));
    };
    let mut text = String::new();
    let mut rest = body;
    while let Some(c) = rest.chars().next() {
        if c != '<' {
            text.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let Some(end) = rest.find('>') else {
            return Err(format!("web.map: a lone '<' in {rhs:?} (write <lt> for '<')"));
        };
        let name = &rest[1..end];
        rest = &rest[end + 1..];
        if name.eq_ignore_ascii_case("lt") {
            text.push('<');
        } else if ["cr", "enter", "return"].iter().any(|k| name.eq_ignore_ascii_case(k)) {
            if !rest.is_empty() {
                return Err(format!(
                    "web.map: <{name}> only at the end of {rhs:?} (one ':' line per mapping; for more, pass a function)"
                ));
            }
            return Ok(MapLine { text, run: true });
        } else {
            return Err(format!(
                "web.map: <{name}> in {rhs:?}: a mapping types no keys (only <CR> at the end, <lt> for '<')"
            ));
        }
    }
    Ok(MapLine { text, run: false })
}

/// What the keys pending so far do (`resolve_mapping`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapStep {
    /// They are the start of a longer mapping: wait for the next key (up to `MAP_TIMEOUT_MS`).
    Wait,
    /// Mapping `index` matches the first `len` of them: run it; the rest are handled after.
    Fire { index: usize, len: usize },
    /// No mapping: the first one is handled as without mappings; the rest are matched again.
    Replay,
}

/// The pending-sequence matcher (vim's): `typed` are the pending keys (`None` = a key no mapping
/// can hold), `maps` every mapping's keys. While `can_wait` and `typed` is a proper prefix of some
/// mapping: `Wait`. Else the longest mapping `typed` starts with fires; else the first key is
/// replayed. A timeout is the same call with `can_wait` false.
pub fn resolve_mapping<'a>(typed: &[Option<MapKey>], maps: impl Iterator<Item = &'a [MapKey]> + Clone, can_wait: bool) -> MapStep {
    let starts = |m: &[MapKey], t: &[Option<MapKey>]| t.len() <= m.len() && t.iter().zip(m).all(|(t, m)| *t == Some(*m));
    if can_wait && maps.clone().any(|m| m.len() > typed.len() && starts(m, typed)) {
        return MapStep::Wait;
    }
    let mut best: Option<(usize, usize)> = None;
    for (i, m) in maps.enumerate() {
        if m.len() <= typed.len() && starts(m, &typed[..m.len()]) && best.is_none_or(|(_, len)| m.len() > len) {
            best = Some((i, m.len()));
        }
    }
    match best {
        Some((index, len)) => MapStep::Fire { index, len },
        None => MapStep::Replay,
    }
}

/// The status bar's mode name. ESC1 and a pending `g`/`y` show their parent mode.
pub fn mode_name(mode: KeyMode) -> &'static str {
    match mode {
        KeyMode::Input => "INPUT",
        KeyMode::Normal => "NORMAL",
    }
}

/// Every name `dispatch_command` knows as the first word (what the first word completes
/// from). Keep it in step with `dispatch_command`'s `match` by hand.
pub const COMMAND_NAMES: &[&str] = &[
    "bookmarkAdd",
    "bookmarkDelete",
    "bookmarkSearch",
    "buffer",
    "bufferDescribe",
    "bufferKill",
    "bufferList",
    "bufferNew",
    "bufferNext",
    "bufferPrev",
    "echo",
    "forget-site",
    "go",
    "help",
    "historyBack",
    "historyDelete",
    "historyFront",
    "historySearch",
    "let",
    "new",
    "notifications",
    "openExternal",
    "popupOpen",
    "popupSearch",
    "q",
    "quit",
    "refresh",
    "set",
    "source",
];

/// C-EDITURL: the commands that open a URL; `<name> ` typed fills in the page's URL, selected.
pub const URL_COMMANDS: [&str; 4] = ["go", "new", "bufferNew", "openExternal"];

/// Short names `dispatch_command` also runs (C4: vim's aliases of the buffer commands). The ghost
/// never offers them; typed in full, an alias leads the first word's candidates, so Enter runs it
/// as typed (`:b` stays `:b`, not the ghost's `bookmarkAdd`).
pub const COMMAND_ALIASES: &[&str] = &["b", "bd", "bn", "bp", "ls"];

/// `:set`'s usage line (C-CARETCMD).
pub const SET_USAGE: &str = "web: usage: :set caret | nocaret | caret?";

/// KEYS.md "Paste": Ctrl-v, Ctrl-Shift-v, Shift-Insert.
pub fn is_paste_chord(key: &KeyInput) -> bool {
    (key.ctrl && matches!(key.ch, Some('v') | Some('V'))) || (key.shift && key.sym == Keysym::Insert)
}

/// The byte offset the token ending at `cursor` starts at: after the last whitespace before
/// `cursor`, or 0.
pub fn token_start_before(text: &str, cursor: usize) -> usize {
    text[..cursor].rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0)
}

/// The status bar's one-line editor (scratchy's `TextEditBuffer`, the command-line part).
/// `cursor` is a byte offset on a `char` boundary.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct TextEditBuffer {
    pub text: String,
    pub cursor: usize,
    /// C-EDITURL: the selected byte range of `text` (the page's URL just filled in), if any.
    pub sel: Option<(usize, usize)>,
}

impl TextEditBuffer {
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.sel = None;
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Inserts `s` at the cursor; control characters (a newline off a clipboard) are dropped.
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            self.insert(c);
        }
    }

    pub fn backspace(&mut self) {
        let Some(prev) = self.text[..self.cursor].chars().next_back() else {
            return;
        };
        self.cursor -= prev.len_utf8();
        self.text.remove(self.cursor);
    }

    pub fn move_left(&mut self) {
        if let Some(prev) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= prev.len_utf8();
        }
    }

    pub fn move_right(&mut self) {
        if let Some(next) = self.text[self.cursor..].chars().next() {
            self.cursor += next.len_utf8();
        }
    }

    /// Ctrl-w: deletes the whitespace before the cursor, then the word before it.
    pub fn delete_word_before(&mut self) {
        let head = self.text[..self.cursor].trim_end();
        let start = token_start_before(head, head.len());
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// Ctrl-u: deletes everything before the cursor.
    pub fn delete_to_start(&mut self) {
        self.text.replace_range(..self.cursor, "");
        self.cursor = 0;
    }
}

/// What the compositor must do after a key event or a host event.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// A command for the engine host.
    Send(Command),
    /// Press this evdev key in the page window (a scroll binding, T13).
    KeyDown(u32),
    /// Release it (the bound key was released).
    KeyUp(u32),
    /// Wheel steps at the pointer (positive = down).
    Wheel(i32),
    /// Wait this many ms before carrying out the effects queued after it (T18: the Enter
    /// after a `find-clear`); effects queued meanwhile wait too, in order.
    Pause(u64),
    /// Quit web (`:q`).
    Quit,
    /// C-EXTERNAL: hand this checked URL to the other browser (`external::open`).
    OpenExternal(String),
    /// A command for page `tab`, shown or not (C-AUTOFOCUS: `blur-field` to a background buffer).
    SendTo(u64, Command),
    /// C-SCRIPT: a line for web's log (a script's error with its stack, what `web.cmd` ran).
    Log(String),
}

/// C-AUTOFOCUS: what an `editable-focused true` did (`Keys::editable_focused`; the compositor logs it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusOutcome {
    /// The user caused it (a click, a key given to the page, `gi`): NORMAL (or a line) -> INPUT (T29).
    Input,
    /// Already INPUT: nothing changes.
    AlreadyInput,
    /// The page's own focus: the mode stays and the host is asked to blur the field (parked, T32).
    Parked,
    /// The page's own focus again within `CLICK_FOCUS_MS` of the last blur (a page that refocuses
    /// on blur): the field keeps the focus, the mode stays (the refocus guard).
    Kept,
    /// No buffer has that page.
    Ignored,
}

/// The status bar's answer to a `gi` that found no text field.
pub const GI_NO_FIELD: &str = "gi: no text field on this page";

/// T17.
pub const NO_PREVIOUS_SEARCH: &str = "No previous search";
/// T15: the wrap messages (vim's), by the direction that wrapped.
pub const SEARCH_WRAP_BOTTOM: &str = "search hit BOTTOM, continuing at TOP";
pub const SEARCH_WRAP_TOP: &str = "search hit TOP, continuing at BOTTOM";

/// T16: the status bar's answer when a query has no match.
pub fn pattern_not_found(query: &str) -> String {
    format!("Pattern not found: {query}")
}

/// Smartcase (KEYS.md Search): case matters only when the query has a capital.
pub fn smartcase_match_case(query: &str) -> bool {
    query.chars().any(char::is_uppercase)
}

/// The search the shell keeps (KEYS.md S1-S6, T7, T15-T17, T23-T25, T31). The host's `find`
/// does the searching and painting; the shell keeps the query, which result it is waiting
/// for and the last `find-result`, from which the wrap messages are derived.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SearchState {
    /// The kept query (`n`/`N` use it; Esc in NORMAL and navigation keep it).
    pub query: Option<String>,
    /// The host has a find session (highlights shown, or a count of zero): `find-clear`
    /// ends it.
    pub session: bool,
    /// The last `find-result`: 1-based active match (0 = none) and match count.
    pub index: u32,
    pub count: u32,
    /// One entry per `find` sent whose `find-result` has not arrived (the host answers each
    /// command once, in order). `true` = it started a session (no wrap can be derived).
    waiting: std::collections::VecDeque<(u64, FindDir, bool)>,
    /// The id of the last `find` sent (C5-STALE: the host echoes it in its `find-result`).
    last_id: u64,
}

impl SearchState {
    /// The host has no find session any more (a navigation, Esc): the query stays.
    pub fn forget_session(&mut self) {
        self.session = false;
        self.index = 0;
        self.count = 0;
        self.waiting.clear();
    }

    /// The id of the oldest `find` still waiting for its result.
    #[cfg(test)]
    pub fn pending_id(&self) -> Option<u64> {
        self.waiting.front().map(|w| w.0)
    }
}

/// Whether a mouse press at `click` was recent enough at `now` for a focus change to count
/// as the user's click: a popup after it is the click's (T29, T57), later ones are the page's own.
pub fn click_recent(click: Option<Instant>, now: Instant) -> bool {
    click.is_some_and(|t| now.saturating_duration_since(t) <= Duration::from_millis(CLICK_FOCUS_MS))
}

/// `:go`'s argument as a URL (scratchy QCOMP6, decided with the user): a word without a
/// scheme gets `https://`; anything with `://` and `about:`/`data:`/`javascript:`/
/// `mailto:`/`file:` is kept as typed; only explicit relative forms (`/x`, `./x`, `../x`,
/// `#frag`, `?q`) resolve against the current page.
pub fn go_url(word: &str, current: &str) -> Result<String, String> {
    let relative =
        word.starts_with('/') || word.starts_with("./") || word.starts_with("../") || word.starts_with('#') || word.starts_with('?');
    if relative {
        let base = url::Url::parse(current).map_err(|_| format!("web: {word}: no page to resolve it against"))?;
        return base.join(word).map(String::from).map_err(|e| format!("web: {word}: {e}"));
    }
    let kept = ["about:", "data:", "javascript:", "mailto:", "file:"];
    if word.contains("://")
        || kept
            .iter()
            .any(|p| word.len() >= p.len() && word[..p.len()].eq_ignore_ascii_case(p))
    {
        return Ok(word.to_string());
    }
    Ok(format!("https://{word}"))
}

/// C-EDITURL: a selected byte range `(start, end)` of the line's text.
pub type Selection = (usize, usize);

/// The status bar's contents (drawn by `statusbar.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarView {
    pub mode: String,
    pub url: String,
    pub message: String,
    /// While the `:` line is open: the prompt, the text and the cursor (it takes the strip).
    /// C-EDITURL: the last is the selected byte range of the text (drawn highlighted), if any.
    pub line: Option<(char, String, usize, Option<Selection>)>,
    /// The dimmed ghost after the line's text (empty = none).
    pub ghost: String,
    /// C-BELL: the visual bell is lit (the bar takes `BELL_BG`/`BELL_FG`).
    pub flash: bool,
}

/// The shell's keyboard state (KEYS.md), fed by the compositor.
pub struct Keys {
    pub key_mode: KeyMode,
    pub vim_mode: VimMode,
    esc: EscState,
    pending_seq: Option<char>,
    /// An `@:` ran since start (what `@@` repeats; C-REPEAT).
    repeated: bool,
    pub line: TextEditBuffer,
    pub ghost: Option<Ghost>,
    pub cache: CompleteCache,
    /// The last message (KEYS.md "Status bar").
    pub message: String,
    /// C-PICKMSG: the message set while PICK is open, drawn beside the filter line.
    pub(crate) pick_msg: String,
    /// C-SCRIPT-ERRBAR: the text of a script's error or an `:echo` while it is still the message
    /// and no key was pressed since; a page load leaves such a message alone (`page_loaded`).
    pub(crate) sticky: String,
    /// What `:source` loads again, and the budget its calls get (set by `load_scripts_with`).
    pub(crate) script_files: Vec<crate::script::ScriptFile>,
    pub(crate) script_budget: Duration,
    /// `web.cmd` lines being run right now (`:source` is refused from them).
    pub(crate) script_lines: usize,
    /// C-CMDRESULT: the command running now reported an error (`fail`, a script's error).
    pub(crate) cmd_failed: bool,
    /// C-CMDRESULT: lines run so far from the outermost `web.cmd` list (SCRIPT_CHAIN_MAX).
    pub(crate) script_chain: usize,
    /// The page's URL as the host reported it (register `%`).
    pub url: String,
    /// The URL the page was requested as before redirects (the host's `url` event), if it differs.
    pub original_url: Option<String>,
    /// The page's title as the host reported it ("" when none yet; `:bookmarkAdd` stores it).
    pub title: String,
    /// `:help`'s page.
    pub help_url: String,
    /// The kept query, the host's find session, the last result (T7, T15-T25).
    pub search: SearchState,
    /// C-CMDHIST: the `:` lines Enter ran (`cmdhist.rs`).
    pub cmd_history: crate::cmdhist::CmdHistory,
    state_dir: Option<PathBuf>,
    sink: Option<ExtSink>,
    /// Key codes held down (a press of one already down is an OS repeat).
    down: HashSet<u32>,
    /// C-BARREPEAT: the desktop's repeat rate (per second, 0 = off) and delay (ms).
    pub repeat_rate: u32,
    pub repeat_delay_ms: u32,
    /// The key the shell repeats itself (winit delivers no OS repeats) and when it repeats next.
    repeating: Option<Repeating>,
    /// A repeat timer is running (set by the compositor, `State::arm_key_repeat`).
    pub repeat_armed: bool,
    /// Key codes whose press reached the page: exactly those get their release (T33, T34).
    forwarded: HashSet<u32>,
    /// Scroll keys given to the page, by the key code of the binding holding them.
    held: Vec<(u32, u32)>,
    /// What the compositor must do next (drained by it).
    pub effects: Vec<Effect>,
    /// xdg_popups currently open (a submenu counts too): a popup session ends when the last closes.
    open_popups: usize,
    /// T57: the mode the user's click left behind when its popup switched to INPUT; restored when the
    /// popup session ends, unless the click focused a field (`user_focused_control`). Per buffer (C4).
    pub(crate) popup_return: Option<KeyMode>,
    /// The desktop's clipboard (register `+`, paste; C6). None: there is none.
    pub clipboard: Option<Box<dyn crate::clipboard::SystemClipboard>>,
    /// `:set caret` (C-CARETCMD): Chromium's caret browsing, as last asked of the host. Off at
    /// start, not kept between runs (the host also turns it off at its start).
    pub caret: bool,
    /// The open picker (C-PICK, `picker.rs`), while `vim_mode` is `Pick`.
    pub picker: Option<crate::picker::Picker>,
    /// C-FILEPICK: the open file chooser (`filepick.rs`), while its picker is shown.
    pub filedlg: Option<crate::filepick::FileDlg>,
    /// C-FILEPICK: the directory the last chooser ended in, where the next one starts.
    pub file_dir: Option<PathBuf>,
    /// The buffers (C4, `buffers.rs`); `url`, `title`, `search` and `popup_return` above are the
    /// current buffer's.
    pub buffers: crate::buffers::Buffers,
    /// C-BELL: the visual bell (`notify.rs`).
    pub bell: crate::notify::Bell,
    /// C-NOTIFLIST: this session's notifications (`notify.rs`), oldest first.
    pub notices: crate::notify::Notices,
    /// C-AUTOFOCUS: when NORMAL last gave the page a key (Enter, T19; the T18 find+Enter, from the real
    /// press): a field focused soon after is the user's doing.
    page_key_at: Option<Instant>,
    /// When `gi` was pressed (a field focused soon after is the user's doing), and whether its
    /// `field-result` is still awaited (`last` failed: ask for `first`; none: the message).
    gi_at: Option<Instant>,
    gi_waiting: bool,
    /// C-SCRIPT: the user's scripts (`script.rs`); None until a file exists.
    pub scripts: Option<crate::script::Scripts>,
    /// C-SCRIPT-MAP: NORMAL key presses held back because they start a user mapping, oldest first,
    /// and when the wait for the next key ends (`MAP_TIMEOUT_MS`). Empty while no mapping is typed.
    map_pending: Vec<PendingKey>,
    map_deadline: Option<Instant>,
    /// A mapping timer is running (set by the compositor, `State::arm_map_timer`).
    pub map_armed: bool,
}

/// C-SCRIPT-MAP: a key press waiting in `Keys::map_pending`. `live`: the press being handled right
/// now (its return value says whether the page gets it); a press held back from an earlier event
/// can no longer go to the page.
#[derive(Clone, Copy, Debug)]
struct PendingKey {
    code: u32,
    key: KeyInput,
    live: bool,
}

impl Keys {
    pub fn new(state_dir: Option<PathBuf>, help_url: String, sink: Option<ExtSink>) -> Keys {
        Keys {
            key_mode: START_MODE,
            vim_mode: VimMode::Normal,
            esc: EscState::Idle,
            pending_seq: None,
            repeated: false,
            open_popups: 0,
            popup_return: None,
            caret: false,
            picker: None,
            filedlg: None,
            file_dir: None,
            buffers: crate::buffers::Buffers::new(),
            bell: Default::default(),
            notices: Default::default(),
            page_key_at: None,
            gi_at: None,
            gi_waiting: false,
            scripts: None,
            map_pending: Vec::new(),
            map_deadline: None,
            map_armed: false,
            line: TextEditBuffer::default(),
            ghost: None,
            cache: CompleteCache::new(state_dir.clone()),
            message: String::new(),
            pick_msg: String::new(),
            sticky: String::new(),
            script_files: Vec::new(),
            script_budget: Duration::from_millis(crate::config::SCRIPT_BUDGET_MS),
            script_lines: 0,
            cmd_failed: false,
            script_chain: 0,
            url: String::new(),
            original_url: None,
            title: String::new(),
            help_url,
            search: SearchState::default(),
            cmd_history: crate::cmdhist::CmdHistory::open(state_dir.clone()),
            state_dir,
            sink,
            down: HashSet::new(),
            repeat_rate: KEY_REPEAT_RATE,
            repeat_delay_ms: KEY_REPEAT_DELAY_MS,
            repeating: None,
            repeat_armed: false,
            forwarded: HashSet::new(),
            held: Vec::new(),
            effects: Vec::new(),
            clipboard: None,
        }
    }

    /// One key event (`code`: the key's code, any numbering as long as press and release
    /// agree). Returns whether the page gets it.
    pub fn key(&mut self, code: u32, pressed: bool, key: &KeyInput, now: Instant) -> bool {
        if !pressed {
            self.down.remove(&code);
            if self.repeating.as_ref().is_some_and(|r| r.code == code) {
                self.repeating = None;
            }
            let mut i = 0;
            while i < self.held.len() {
                if self.held[i].0 == code {
                    let (_, synth) = self.held.remove(i);
                    self.effects.push(Effect::KeyUp(synth));
                } else {
                    i += 1;
                }
            }
            // T33/T34: a release goes to the page exactly when its press did, so the page
            // never sees a keyup without its keydown, nor misses one (no stuck keys).
            return self.forwarded.remove(&code);
        }
        // C-SCRIPT-ERRBAR: the user's next key ends the protection of a script's message.
        self.sticky.clear();
        let repeat = !self.down.insert(code);
        let was_line = self.in_line_mode();
        let forward = self.press(code, key, repeat, now);
        if !repeat && was_line {
            self.arm_repeat(code, key, now);
        }
        if forward {
            self.forwarded.insert(code);
        }
        forward
    }

    /// C-FOCUSKEYS: the shell window lost the desktop's keyboard focus. Returns the key codes the
    /// page has pressed (their releases were not seen yet and winit's synthetic ones never
    /// arrive), in order, for the compositor to release in the page. Everything held is forgotten
    /// and the scroll keys held for bindings are released, so refocusing starts clean.
    /// The second list is the keys the shell itself consumed (down, never given to the page): the seat's
    /// key state forgets them too, quietly.
    pub fn release_all(&mut self) -> (Vec<u32>, Vec<u32>) {
        let mut pressed: Vec<u32> = self.forwarded.drain().collect();
        pressed.sort_unstable();
        let mut ours: Vec<u32> = self.down.drain().filter(|c| !pressed.contains(c)).collect();
        ours.sort_unstable();
        for (_, synth) in self.held.drain(..) {
            self.effects.push(Effect::KeyUp(synth));
        }
        self.esc = EscState::Idle;
        self.pending_seq = None;
        self.drop_map_pending();
        self.repeating = None;
        (pressed, ours)
    }

    /// The shell consumes keys itself: the `:` and `/` lines and the picker (NORMAL key mode).
    fn in_line_mode(&self) -> bool {
        self.key_mode == KeyMode::Normal && matches!(self.vim_mode, VimMode::Command | VimMode::Search | VimMode::Pick)
    }

    /// C-BARREPEAT: a first press of an editing key in a line starts the shell's own repeat of it.
    /// Modifiers, Enter and Escape never repeat; a mode change ends it.
    fn arm_repeat(&mut self, code: u32, key: &KeyInput, now: Instant) {
        let raw = key.sym.raw();
        let modifier = (0xffe1..=0xffee).contains(&raw) || (0xfe01..=0xfe13).contains(&raw);
        if modifier {
            return;
        }
        if !self.in_line_mode() || self.repeat_rate == 0 || matches!(key.sym, Keysym::Return | Keysym::KP_Enter | Keysym::Escape) {
            self.repeating = None;
            return;
        }
        self.repeating = Some(Repeating {
            code,
            key: *key,
            next: now + Duration::from_millis(self.repeat_delay_ms as u64),
        });
    }

    /// Whether a key is waiting to repeat, and after how long (the compositor's timer).
    pub fn repeat_wait(&self, now: Instant) -> Option<Duration> {
        self.repeating.as_ref().map(|r| r.next.saturating_duration_since(now))
    }

    /// C-BARREPEAT: repeats the held key when its time has come. Returns the wait until the next
    /// repeat, None when nothing repeats any more (released, mode changed, focus lost).
    pub fn repeat_tick(&mut self, now: Instant) -> Option<Duration> {
        if !self.in_line_mode() {
            self.repeating = None;
            return None;
        }
        let (code, key, next) = self.repeating.as_ref().map(|r| (r.code, r.key, r.next))?;
        if now < next {
            return Some(next - now);
        }
        self.press(code, &key, true, now);
        let interval = Duration::from_millis(1000 / self.repeat_rate.max(1) as u64);
        if !self.in_line_mode() {
            self.repeating = None;
            return None;
        }
        if let Some(r) = &mut self.repeating {
            r.next = now + interval;
        }
        Some(interval)
    }

    /// C3-SHIFTSYNC: no key is physically down (as far as the shell saw), so the desktop's
    /// modifier state is not racing the key events still being handled.
    pub fn idle(&self) -> bool {
        self.down.is_empty()
    }

    fn press(&mut self, code: u32, key: &KeyInput, repeat: bool, now: Instant) -> bool {
        match self.key_mode {
            KeyMode::Normal => {
                match self.vim_mode {
                    VimMode::Command => self.command_line_key(key),
                    VimMode::Search => self.search_line_key(key),
                    VimMode::Pick => self.pick_key(key),
                    VimMode::Normal => return self.normal_key_mapped(code, key, repeat, now),
                }
                false
            }
            KeyMode::Input => {
                let (st, enter) = resolve_esc_esc(self.esc, key, repeat, now);
                self.esc = st;
                if enter {
                    self.set_key_mode(KeyMode::Normal);
                    return false;
                }
                true
            }
        }
    }

    /// C-SCRIPT-MAP: a NORMAL key press with the user's mappings in front of `normal_key`
    /// (SCRIPTING.md "web.map"). With no mapping defined it is exactly `normal_key`. So it is for a
    /// modifier key's own press (Shift for the `T` of `gT` does not break a sequence), for an OS
    /// repeat with nothing pending, and for the key after a half-typed built-in sequence (`g`, `y`):
    /// that one takes its second key unmapped, as vim reads the second key of `gg` with mappings
    /// off. Otherwise the press joins the pending keys and `resolve_mapping` decides: wait, run a
    /// mapping, or hand the first key to `normal_key` as if no mapping existed (the rest are matched
    /// again).
    fn normal_key_mapped(&mut self, code: u32, key: &KeyInput, repeat: bool, now: Instant) -> bool {
        if !self.scripts.as_ref().is_some_and(|s| s.has_maps()) {
            return self.normal_key(code, key, repeat, now);
        }
        // A wait that ran out before the timer fired (or the tests' clock): its keys go first.
        if self.map_deadline.is_some_and(|d| now >= d) {
            self.flush_map_pending(now);
        }
        let idle = self.map_pending.is_empty();
        if idle && (self.pending_seq.is_some() || repeat || is_modifier_sym(key.sym)) {
            return self.normal_key(code, key, repeat, now);
        }
        if is_modifier_sym(key.sym) {
            // Shift for the `T` of `gT`: waits in line but never takes part in a match, and it is
            // handled in its turn if the keys before it are replayed (`normal_key` gives a
            // modifier's press nothing but the end of a half-typed `g`, as without mappings).
            self.map_pending.push(PendingKey {
                code,
                key: *key,
                live: false,
            });
            return false;
        }
        if repeat {
            // An OS repeat ends the wait (the key is held, not typed again): the pending keys are
            // handled at once, then the repeat as without mappings.
            self.flush_map_pending(now);
            return self.normal_key(code, key, repeat, now);
        }
        self.map_pending.push(PendingKey {
            code,
            key: *key,
            live: true,
        });
        let forward = self.drain_map_pending(now, true);
        for p in &mut self.map_pending {
            p.live = false;
        }
        forward
    }

    /// The mapping wait ran out: the pending keys are handled without waiting further.
    fn flush_map_pending(&mut self, now: Instant) {
        self.drain_map_pending(now, false);
    }

    /// Resolves `map_pending` from the front until it is empty or waits for more (`can_wait`).
    /// Returns whether the page gets the live press.
    fn drain_map_pending(&mut self, now: Instant, can_wait: bool) -> bool {
        let mut forward = false;
        self.map_deadline = None;
        while !self.map_pending.is_empty() {
            let normal = self.key_mode == KeyMode::Normal && self.vim_mode == VimMode::Normal;
            // A mapping or a replayed key left NORMAL (opened the `:` line, ...): the keys typed
            // after it go where they would have gone (into the line), in order. A replayed key that
            // armed a built-in sequence: the next key completes it, unmapped.
            if !normal || self.pending_seq.is_some() || is_modifier_sym(self.map_pending[0].key.sym) {
                let p = self.map_pending.remove(0);
                let f = if normal {
                    self.normal_key(p.code, &p.key, false, now)
                } else {
                    self.press(p.code, &p.key, false, now)
                };
                forward |= self.settle_replayed(p, f);
                continue;
            }
            let typed: Vec<Option<MapKey>> = self
                .map_pending
                .iter()
                .filter(|p| !is_modifier_sym(p.key.sym))
                .map(|p| MapKey::from_input(&p.key))
                .collect();
            let step = match self.scripts.as_ref() {
                Some(s) => s.resolve_map(&typed, can_wait),
                None => MapStep::Replay,
            };
            match step {
                MapStep::Wait => {
                    self.map_deadline = Some(now + Duration::from_millis(MAP_TIMEOUT_MS));
                    break;
                }
                MapStep::Fire { index, len } => {
                    // The mapping's keys and the modifier presses among them.
                    let mut keys = 0;
                    while keys < len && !self.map_pending.is_empty() {
                        let p = self.map_pending.remove(0);
                        if !is_modifier_sym(p.key.sym) {
                            keys += 1;
                        }
                    }
                    self.run_mapping(index);
                }
                MapStep::Replay => {
                    let p = self.map_pending.remove(0);
                    let f = self.normal_key(p.code, &p.key, false, now);
                    forward |= self.settle_replayed(p, f);
                }
            }
        }
        forward
    }

    /// After a pending key was handled: whether the page gets it (only the live press can go to the
    /// page), and a key released while it waited lets go of what its binding holds (a scroll key).
    fn settle_replayed(&mut self, p: PendingKey, forward: bool) -> bool {
        if p.live {
            return forward;
        }
        if !self.down.contains(&p.code) {
            let mut i = 0;
            while i < self.held.len() {
                if self.held[i].0 == p.code {
                    let (_, synth) = self.held.remove(i);
                    self.effects.push(Effect::KeyUp(synth));
                } else {
                    i += 1;
                }
            }
        }
        false
    }

    /// Mode changes and focus loss drop the keys waiting for a mapping (as they drop a pending `g`).
    fn drop_map_pending(&mut self) {
        self.map_pending.clear();
        self.map_deadline = None;
    }

    /// C-SOURCE: with no key waiting for a mapping, no timer runs for one.
    pub(crate) fn stop_idle_map_timer(&mut self) {
        if self.map_pending.is_empty() {
            self.map_deadline = None;
        }
    }

    /// Whether keys wait for a mapping, and for how long (the compositor's timer).
    pub fn map_wait(&self, now: Instant) -> Option<Duration> {
        self.map_deadline.map(|d| d.saturating_duration_since(now))
    }

    /// The compositor's mapping timer: when the wait has run out, the pending keys are handled as
    /// without the longer mapping. Returns the wait left, None when nothing waits any more.
    pub fn map_tick(&mut self, now: Instant) -> Option<Duration> {
        let d = self.map_deadline?;
        if now < d {
            return Some(d - now);
        }
        self.flush_map_pending(now);
        self.map_wait(now)
    }

    /// Runs mapping `index`'s rhs: a `:` line (runs with `<CR>`, else opens the line with it typed)
    /// or a function (a call into JS under `SCRIPT_BUDGET_MS`).
    fn run_mapping(&mut self, index: usize) {
        let Some(s) = self.scripts.as_mut() else { return };
        let Some((keys, rhs)) = s.map_rhs(index) else { return };
        self.script_sync();
        let Some(s) = self.scripts.as_mut() else { return };
        match rhs {
            crate::script::MapRhs::Line(MapLine { text, run: true }) => {
                self.effects.push(Effect::Log(format!("map {keys}: :{text}")));
                self.run_command_line(&text);
            }
            crate::script::MapRhs::Line(MapLine { text, run: false }) => self.open_line_with(&text),
            crate::script::MapRhs::Func(f) => {
                let r = s.call_map(f);
                self.finish_script_call(r);
            }
        }
    }

    /// A key press in NORMAL (T6-T21): a pending sequence first, then the bindings. Returns
    /// whether the page gets the press (only an `Action::ToPage` binding, T19).
    fn normal_key(&mut self, code: u32, key: &KeyInput, repeat: bool, now: Instant) -> bool {
        let (seq_action, pending, consumed) = resolve_sequence(self.pending_seq, key);
        self.pending_seq = pending;
        let action = if consumed {
            seq_action
        } else {
            resolve_binding(key, self.vim_mode)
        };
        match action {
            // T18: with a find session open, end it first, as T7 does: Chromium then focuses the
            // link or control holding the active match (or selects it), so the Enter acts on it.
            // The Enter goes to the page `ENTER_AFTER_FIND_CLEAR_MS` after the `find-clear` (an
            // evdev Enter held until the real key is released): sent at once, it sometimes
            // reached the page before Chromium had moved the focus (1 in 4 live runs).
            Some(Action::ToPage) if self.search.session && !repeat => {
                // C-AUTOFOCUS: timed from the real press (Chromium moves the focus at the find-clear).
                self.page_key_at = Some(now);
                self.clear_search();
                self.effects.push(Effect::Pause(crate::config::ENTER_AFTER_FIND_CLEAR_MS));
                self.effects.push(Effect::KeyDown(crate::config::evdev::KEY_ENTER));
                self.held.push((code, crate::config::evdev::KEY_ENTER));
                false
            }
            Some(Action::ToPage) => {
                let to_page = !self.held.iter().any(|h| h.0 == code);
                if to_page {
                    self.page_key_at = Some(now);
                }
                to_page
            }
            Some(action) => {
                if !(repeat && action_ignores_repeat(action)) {
                    self.perform(code, action, repeat, now);
                }
                false
            }
            None => false,
        }
    }

    fn perform(&mut self, code: u32, action: Action, repeat: bool, now: Instant) {
        match action {
            // Handled by `normal_key`: the press itself is forwarded.
            Action::ToPage => {}
            Action::Scroll(i) => match SCROLL_INPUTS.get(i) {
                Some(ScrollInput::Key(k)) => {
                    // Held until the binding's key is released: the engine repeats it.
                    if !repeat {
                        self.effects.push(Effect::KeyDown(*k));
                        self.held.push((code, *k));
                    }
                }
                Some(ScrollInput::Wheel(n)) => self.effects.push(Effect::Wheel(*n)),
                None => self.message = format!("web: no SCROLL_INPUTS row {i} (config.rs)"),
            },
            Action::EnterCommandMode => self.open_line(),
            Action::SearchOpen => self.open_search_line(),
            Action::SearchNext => self.search_step(FindDir::Next),
            Action::SearchPrev => self.search_step(FindDir::Prev),
            Action::ClearSearch => self.clear_search(),
            Action::EnterInputMode => self.enter_input_restoring(),
            Action::FocusField => self.focus_field(now),
            Action::HistoryBack => self.effects.push(Effect::Send(Command::Back)),
            Action::HistoryForward => self.effects.push(Effect::Send(Command::Forward)),
            Action::YankUrl => self.run_command_line("let @+ = @%"),
            Action::RepeatLine => self.repeat_line(),
            Action::RepeatRepeat => self.repeat_repeat(),
            Action::AlternateBuffer => self.buffer_alternate(),
        }
    }

    /// Switches the mode. Leaving NORMAL drops a pending `g`/`y` and an open `:` line.
    pub fn set_key_mode(&mut self, mode: KeyMode) {
        self.key_mode = mode;
        self.esc = EscState::Idle;
        self.pending_seq = None;
        if mode == KeyMode::Input {
            self.drop_map_pending();
        }
        if mode == KeyMode::Input && self.vim_mode != VimMode::Normal {
            self.close_line();
        }
    }

    /// T29: the user clicked into a text field or select: INPUT (an open line is cancelled).
    pub fn user_focused_control(&mut self) {
        // The click focused a field: the popup (a context menu on it) no longer returns the mode.
        self.popup_return = None;
        if self.key_mode != KeyMode::Input {
            self.set_key_mode(KeyMode::Input);
        }
    }

    /// A new xdg_popup. `after_click`: a user mouse press just before it. T29 + T57: the first
    /// popup of a session opened by a click switches to INPUT and remembers the mode to return to.
    /// Nested popups (submenus) join the running session; a popup without a click changes nothing.
    pub fn popup_opened(&mut self, after_click: bool) {
        if self.open_popups == 0 && after_click && self.key_mode != KeyMode::Input {
            self.popup_return = Some(self.key_mode);
            self.user_focused_control_keep_return();
        }
        self.open_popups += 1;
    }

    fn user_focused_control_keep_return(&mut self) {
        let keep = self.popup_return;
        self.user_focused_control();
        self.popup_return = keep;
    }

    /// An xdg_popup closed (picked, Esc, click outside). T57: the last one of a session returns to the
    /// mode from before the click, if still in INPUT.
    pub fn popup_closed(&mut self) {
        self.open_popups = self.open_popups.saturating_sub(1);
        if self.open_popups == 0 {
            if let Some(mode) = self.popup_return.take() {
                if self.key_mode == KeyMode::Input {
                    self.set_key_mode(mode);
                }
            }
        }
    }

    /// C-AUTOFOCUS (KEYS.md T29, T32): the host's `editable-focused true` for page `tab`, `after_click`
    /// = a mouse press within `CLICK_FOCUS_MS`. The user caused it (that click, a key NORMAL gave the
    /// page, `gi`, each within `CLICK_FOCUS_MS`): INPUT, as T29. Otherwise the page focused the field
    /// itself (`autofocus`, a late `el.focus()`), or the page is a background buffer's: the mode stays
    /// (an open line or the picker too) and the host blurs the field, which the page remembers as its
    /// last field ("parked": `i` focuses it again). At most one blur per buffer per `CLICK_FOCUS_MS`: a
    /// page that refocuses on blur keeps its focus after that. Any editable focus ends an older park.
    pub fn editable_focused(&mut self, tab: u64, after_click: bool, now: Instant) -> FocusOutcome {
        let current = self.buffers.current_tab() == Some(tab);
        let user = current && (after_click || click_recent(self.page_key_at, now) || click_recent(self.gi_at, now));
        let input = self.key_mode == KeyMode::Input;
        let Some(b) = self.buffers.by_tab_mut(tab) else {
            return FocusOutcome::Ignored;
        };
        b.parked = false;
        if user {
            self.user_focused_control();
            return if input { FocusOutcome::AlreadyInput } else { FocusOutcome::Input };
        }
        if current && input {
            return FocusOutcome::AlreadyInput;
        }
        if click_recent(b.blurred_at, now) {
            return FocusOutcome::Kept;
        }
        b.blurred_at = Some(now);
        b.parked = true;
        self.effects.push(Effect::SendTo(tab, Command::BlurField));
        FocusOutcome::Parked
    }

    /// `i` (T6): INPUT; C-AUTOFOCUS: if the page focused a field that was parked, it gets the focus
    /// back, so typing lands in the field the page picked.
    fn enter_input_restoring(&mut self) {
        self.set_key_mode(KeyMode::Input);
        let b = self.buffers.current_mut();
        if std::mem::take(&mut b.parked) {
            self.effects.push(Effect::Send(Command::FocusField { which: FieldWhich::Last }));
        }
    }

    /// `gi` (C-AUTOFOCUS): the page's last text field (the parked one too), else its first visible
    /// one (`field_result` asks for it); INPUT when one has the focus.
    fn focus_field(&mut self, now: Instant) {
        self.gi_at = Some(now);
        self.gi_waiting = true;
        self.effects.push(Effect::Send(Command::FocusField { which: FieldWhich::Last }));
    }

    /// The host's `field-result` for page `tab`: `op` is `blur`, `last` or `first`, `ok` whether a
    /// field was blurred or has the focus now. Only `gi`'s answers change anything: a field focused
    /// means INPUT (also when it already had the focus, so no `editable-focused` comes); `last` found
    /// none: ask for `first`; `first` found none: a message, the mode stays.
    pub fn field_result(&mut self, tab: u64, op: &str, ok: bool) {
        if self.buffers.current_tab() != Some(tab) || !self.gi_waiting || op == "blur" {
            return;
        }
        match (op, ok) {
            (_, true) => {
                self.gi_waiting = false;
                self.user_focused_control();
            }
            ("last", false) => self.effects.push(Effect::Send(Command::FocusField { which: FieldWhich::First })),
            _ => {
                self.gi_waiting = false;
                self.message = GI_NO_FIELD.to_string();
            }
        }
    }

    fn open_line(&mut self) {
        self.vim_mode = VimMode::Command;
        self.cmd_history.end_browse();
        self.line.clear();
        self.ghost = None;
        // A new cache per line: the history is read once per line (KEYS.md "Per keystroke cost").
        let mut cache = CompleteCache::new(self.state_dir.clone());
        if let Some(sink) = &self.sink {
            cache = cache.with_external(sink.clone(), self.url.clone());
        }
        cache.user_commands = self.user_command_names();
        self.cache = cache;
    }

    /// T8: `/` opens an empty search line.
    fn open_search_line(&mut self) {
        self.vim_mode = VimMode::Search;
        self.line.clear();
        self.ghost = None;
    }

    /// A key in the `/` line (T22-T25): the `:` line's editing keys, no completion.
    fn search_line_key(&mut self, key: &KeyInput) {
        let s = key.sym;
        if s == Keysym::Escape {
            // T25: nothing was changed while typing, so cancelling restores nothing.
            self.close_line();
        } else if s == Keysym::Return || s == Keysym::KP_Enter {
            let query = std::mem::take(&mut self.line.text);
            self.close_line();
            // T24: an empty query is Esc.
            if !query.is_empty() {
                self.search_run(query);
            }
        } else if s == Keysym::Right || s == Keysym::End || s == Keysym::KP_Right || s == Keysym::KP_End {
            if s == Keysym::End || s == Keysym::KP_End {
                self.line.cursor = self.line.text.len();
            } else {
                self.line.move_right();
            }
        } else if s != Keysym::Tab && s != Keysym::ISO_Left_Tab && s != Keysym::Delete && s != Keysym::KP_Delete {
            self.line_edit_key(key);
        }
    }

    /// T23: the query becomes the kept one and runs from a fresh session.
    fn search_run(&mut self, query: String) {
        self.message.clear();
        if self.search.session {
            self.effects.push(Effect::Send(Command::FindClear));
        }
        self.search.waiting.clear();
        self.search.index = 0;
        self.search.count = 0;
        self.send_find(&query, FindDir::Next, true, true);
        self.search.query = Some(query);
    }

    /// `screen_top`: T23's start (the first match at or below the top of the screen) rather
    /// than where the engine picks.
    fn send_find(&mut self, query: &str, dir: FindDir, fresh: bool, screen_top: bool) {
        self.search.session = true;
        self.search.last_id += 1;
        let id = self.search.last_id;
        self.search.waiting.push_back((id, dir, fresh));
        self.effects.push(Effect::Send(Command::Find {
            id,
            query: query.to_string(),
            dir,
            match_case: smartcase_match_case(query),
            screen_top,
        }));
    }

    /// T15-T17: `n` / `N`.
    fn search_step(&mut self, dir: FindDir) {
        let Some(query) = self.search.query.clone() else {
            self.message = NO_PREVIOUS_SEARCH.to_string();
            return;
        };
        // No host session (Esc, a navigation, or the page changed): the host starts a new
        // one, at its first (n) or last (N) match, so no wrap can be told.
        let fresh = !self.search.session;
        self.send_find(&query, dir, fresh, false);
    }

    /// T7: highlights go, the query stays.
    fn clear_search(&mut self) {
        if self.search.session {
            self.effects.push(Effect::Send(Command::FindClear));
            self.forget_session();
            self.message.clear();
        }
    }

    /// T31: any navigation. The host dropped its session itself; the query stays.
    pub fn navigated(&mut self) {
        // A new document has no title yet (the previous page's must not be bookmarked with it).
        self.title.clear();
        // C-AUTOFOCUS: a field the new document focuses (`autofocus`) is the page's own, even right
        // after the click, Enter or `gi` that loaded it (the compositor forgets its click too).
        self.page_key_at = None;
        self.gi_at = None;
        self.gi_waiting = false;
        if self.search.session {
            self.forget_session();
            self.message.clear();
        }
    }

    /// T31, C5-FRAG: a same-document navigation. The host still has its session (no load
    /// callbacks): end it so its highlights go; the query stays.
    pub fn navigated_same_document(&mut self) {
        self.clear_search();
    }

    fn forget_session(&mut self) {
        self.search.forget_session();
    }

    /// The host's `find-result` for the oldest `find` still waiting (T15, T16, T23): the
    /// status bar gets `Pattern not found: <q>`, or `[i/N]` with a wrap message when the
    /// index went the wrong way (next: not higher than before; previous: not lower).
    ///
    /// C5-STALE: `id` is the request it answers. A result whose id is not waiting (CEF's
    /// `0/0` at a load start, an answer to a find that a navigation or Esc dropped) is
    /// ignored; the requests older than the answered one will never be answered (the host
    /// answers only the latest) and are dropped with it.
    #[cfg(test)]
    pub fn find_result(&mut self, id: u64, index: u32, count: u32) {
        self.find_result_wrapped(id, index, count, false);
    }

    /// `wrapped` (C5b, T23): the host found no match at or below the top of the screen and
    /// went to the first one: the BOTTOM wrap message.
    pub fn find_result_wrapped(&mut self, id: u64, index: u32, count: u32, wrapped: bool) {
        if !self.search.waiting.iter().any(|w| w.0 == id) {
            return;
        }
        while self.search.waiting.front().is_some_and(|w| w.0 != id) {
            self.search.waiting.pop_front();
        }
        let Some((_, dir, fresh)) = self.search.waiting.pop_front() else {
            return;
        };
        let old = self.search.index;
        self.search.index = index;
        self.search.count = count;
        let Some(query) = self.search.query.clone() else { return };
        if count == 0 || index == 0 {
            self.message = pattern_not_found(&query);
            return;
        }
        let wrapped = wrapped
            || (!fresh
                && old > 0
                && match dir {
                    FindDir::Next => index <= old,
                    FindDir::Prev => index >= old,
                });
        let wrap = match (wrapped, dir) {
            (false, _) => "",
            (true, FindDir::Next) => SEARCH_WRAP_BOTTOM,
            (true, FindDir::Prev) => SEARCH_WRAP_TOP,
        };
        self.message = if wrap.is_empty() {
            format!("[{index}/{count}]")
        } else {
            format!("{wrap}  [{index}/{count}]")
        };
    }

    /// Closes the `:` or `/` line or the picker (T25, T28, T29, T64).
    pub(crate) fn close_line(&mut self) {
        self.vim_mode = VimMode::Normal;
        self.line.clear();
        self.ghost = None;
        self.picker = None;
        // C-FILEPICK: a chooser that ends without an answer (Esc, a click) is cancelled at the host.
        self.file_dialog_cancelled();
    }

    /// The state directory (the history file's), if any.
    pub fn state_dir(&self) -> Option<PathBuf> {
        self.state_dir.clone()
    }

    /// A key in the `:` line (T26-T28, T45-T53).
    fn command_line_key(&mut self, key: &KeyInput) {
        let s = key.sym;
        let updown = matches!(s, Keysym::Up | Keysym::KP_Up | Keysym::Down | Keysym::KP_Down);
        if !updown {
            self.cmd_history.end_browse(); // C-CMDHIST: any other key ends browsing, keeping the text
        }
        let enter = s == Keysym::Return || s == Keysym::KP_Enter;
        if self.line.sel.is_some() && s != Keysym::Escape && !enter {
            self.selection_key(key);
            return;
        }
        if updown {
            self.history_key(matches!(s, Keysym::Up | Keysym::KP_Up));
        } else if s == Keysym::Escape {
            self.close_line();
        } else if s == Keysym::Return || s == Keysym::KP_Enter {
            let mut input = std::mem::take(&mut self.line.text);
            if let Some(g) = self.ghost.take() {
                input = g.accepted(&input);
            }
            self.close_line();
            self.cmd_history.record(&input);
            self.run_command_line(&input);
        } else if s == Keysym::Tab || s == Keysym::ISO_Left_Tab {
            let back = key.shift || s == Keysym::ISO_Left_Tab;
            let text = self.line.text.clone();
            if let Some(g) = &mut self.ghost {
                if back {
                    g.prev();
                } else if let Some(add) = g.tab(&text) {
                    self.line.insert_str(&add);
                    self.recompute_ghost();
                    // C-EDITURL: Tab completing the name (it adds the space) fills in the URL.
                    if self.line.text != text {
                        let page = self.url.clone();
                        self.maybe_fill_url(&page);
                    }
                }
            }
        } else if s == Keysym::Delete || s == Keysym::KP_Delete {
            self.ghost = None;
        } else if s == Keysym::Right || s == Keysym::End || s == Keysym::KP_Right || s == Keysym::KP_End {
            if let Some(g) = self.ghost.take() {
                let add = g.rest(&self.line.text);
                self.line.insert_str(&add);
            } else if s == Keysym::End || s == Keysym::KP_End {
                self.line.cursor = self.line.text.len();
            } else {
                self.line.move_right();
            }
            self.recompute_ghost();
        } else {
            self.line_edit_key(key);
            self.recompute_ghost();
            // C-EDITURL: the Space that makes the line exactly `go ` fills in the URL.
            if key.ch == Some(' ') && !key.ctrl && !key.alt && !key.logo {
                let page = self.url.clone();
                self.maybe_fill_url(&page);
            }
        }
    }

    /// C-EDITURL (KEYS.md T129-T133): the line is exactly `<command> `, cursor at the end, for a command
    /// that opens a URL: the page's URL goes after it, selected. No page (empty, `about:blank`, the
    /// help page): nothing. Called only where a trigger happened (typed Space, Tab, a line opened with
    /// text), never after Backspace, Ctrl-w, a paste or a history recall.
    fn maybe_fill_url(&mut self, page: &str) {
        let is_cmd = URL_COMMANDS.iter().any(|c| self.line.text.strip_suffix(' ') == Some(*c));
        if !is_cmd || self.line.cursor != self.line.text.len() {
            return;
        }
        if page.is_empty() || page == "about:blank" || page == self.help_url {
            return;
        }
        let start = self.line.cursor;
        self.line.insert_str(page);
        let end = self.line.cursor;
        if end > start {
            self.line.sel = Some((start, end));
            self.ghost = None; // T45 waits for the first edit
        }
    }

    /// C-EDITURL (T130-T133): a key while the URL is selected (Esc and Enter are the line's own).
    fn selection_key(&mut self, key: &KeyInput) {
        let Some((a, b)) = self.line.sel else { return };
        let s = key.sym;
        let left = matches!(s, Keysym::Left | Keysym::KP_Left | Keysym::Home | Keysym::KP_Home);
        let right = matches!(s, Keysym::Right | Keysym::KP_Right | Keysym::End | Keysym::KP_End);
        if matches!(s, Keysym::Up | Keysym::KP_Up | Keysym::Down | Keysym::KP_Down) {
            // History filtered by what is before the selection (`go `), not by the URL.
            let up = matches!(s, Keysym::Up | Keysym::KP_Up);
            let prefix = self.line.text[..a].to_string();
            let t = if up {
                self.cmd_history.up(&prefix)
            } else {
                self.cmd_history.down()
            };
            if let Some(t) = t {
                self.line.sel = None;
                self.line.cursor = t.len();
                self.line.text = t;
                self.ghost = None;
            }
        } else if left || right {
            self.line.sel = None;
            self.line.cursor = if left { a } else { self.line.text.len() };
            self.recompute_ghost();
        } else if s == Keysym::BackSpace || s == Keysym::Delete || s == Keysym::KP_Delete || (key.ctrl && key.ch == Some('w')) {
            self.remove_selection();
        } else if key.ctrl && key.ch == Some('u') {
            self.line.text.replace_range(..b, "");
            self.line.cursor = 0;
            self.line.sel = None;
            self.recompute_ghost();
        } else if is_paste_chord(key) {
            let before = self.line.text.clone();
            self.remove_selection();
            self.paste();
            if self.line.text == format!("{}{}", &before[..a], &before[b..]) {
                // an empty or unreadable clipboard changes nothing: the selection stays
                self.line.text = before;
                self.line.sel = Some((a, b));
                self.line.cursor = b;
                self.ghost = None;
            } else {
                self.recompute_ghost();
            }
        } else if !key.ctrl && !key.alt && !key.logo {
            if let Some(c) = key.ch {
                self.remove_selection();
                self.line.insert(c);
                self.recompute_ghost();
            }
        }
    }

    fn remove_selection(&mut self) {
        if let Some((a, b)) = self.line.sel.take() {
            self.line.text.replace_range(a..b, "");
            self.line.cursor = a;
        }
        self.recompute_ghost();
    }

    /// C-CMDHIST (KEYS.md T125-T128): Up/Down recall the `:` lines run, filtered by what was typed.
    /// A recalled line shows no ghost (Enter runs exactly it).
    fn history_key(&mut self, up: bool) {
        let text = if up {
            self.cmd_history.up(&self.line.text)
        } else {
            self.cmd_history.down()
        };
        if let Some(t) = text {
            self.line.text = t;
            self.line.cursor = self.line.text.len();
            self.ghost = None;
        }
    }

    /// The `:` line run last (C-REPEAT's `@:` reads it), kept across restarts.
    pub fn last_command_line(&self) -> Option<&str> {
        self.cmd_history.last()
    }

    /// `@:` and `:@:` (C-REPEAT, vim's E30 when there is no line): runs the last `:` line again. It is
    /// not recorded, so it never becomes the last line itself.
    pub fn repeat_line(&mut self) {
        let Some(line) = self.last_command_line().map(str::to_string) else {
            self.fail("web: no previous command line".to_string());
            return;
        };
        self.repeated = true;
        self.run_command_line(&line);
    }

    /// `@@` and `:@@` (C-REPEAT, vim's E748 before any `@:`): repeats the last `@:`.
    pub fn repeat_repeat(&mut self) {
        if !self.repeated {
            self.fail("web: no previously used register".to_string());
            return;
        }
        self.repeat_line();
    }

    pub(crate) fn line_edit_key(&mut self, key: &KeyInput) {
        let s = key.sym;
        if is_paste_chord(key) {
            self.paste();
        } else if s == Keysym::BackSpace {
            self.line.backspace();
        } else if s == Keysym::Left || s == Keysym::KP_Left {
            self.line.move_left();
        } else if s == Keysym::Home || s == Keysym::KP_Home {
            self.line.cursor = 0;
        } else if key.ctrl && key.ch == Some('w') {
            self.line.delete_word_before();
        } else if key.ctrl && key.ch == Some('u') {
            self.line.delete_to_start();
        } else if !key.ctrl && !key.alt && !key.logo {
            if let Some(c) = key.ch {
                self.line.insert(c);
            }
        }
    }

    /// KEYS.md "Paste" (T22, T26): the desktop clipboard's text, made one line, at the cursor.
    /// An empty or unreadable clipboard changes nothing.
    fn paste(&mut self) {
        let Some(cb) = self.clipboard.as_mut() else { return };
        if let Ok(text) = cb.read(crate::clipboard::Which::Clipboard) {
            self.line.insert_str(&crate::clipboard::clean_paste_text(&text));
        }
    }

    /// T45/T46: the ghost for the line as it is now.
    fn recompute_ghost(&mut self) {
        self.ghost = compute_ghost(&self.line.text, self.line.cursor, &self.cache);
    }

    /// An external completion source answered (KEYS.md "Completion"). A result for another
    /// `:` line is dropped; an error goes to the status bar.
    pub fn complete_result(&mut self, r: ExtResult) {
        if self.vim_mode != VimMode::Command || !self.cache.accept(&r) {
            return;
        }
        if let Err(msg) = &r.outcome {
            self.message = msg.clone();
        }
        self.recompute_ghost();
    }

    /// What `:go word` opens (shared with `:new`): `go_url`, except that a scheme-less word that is a
    /// history URL minus its scheme runs as that URL (C3-GOHTTP: an http:// site stays http://). A
    /// typed scheme never gets that far.
    pub fn go_target(&self, word: &str) -> Result<String, String> {
        let url = go_url(word, &self.url)?;
        if !word.contains("://") && url.starts_with("https://") {
            if let Some(scheme) = crate::completion::history_scheme_for(&self.cache, word) {
                return Ok(format!("{scheme}{word}"));
            }
        }
        Ok(url)
    }

    /// Opens the `:` line with `text` already typed (`:new` without an argument types `go `; a
    /// `web.map` string rhs without `<CR>`, C-SCRIPT-MAP). C-EDITURL's fill-in (the page's URL after
    /// `go ` and the other URL commands, selected) belongs here, so both of those get it.
    pub fn open_line_with(&mut self, text: &str) {
        let page = self.url.clone();
        self.open_line_filled(text, &page);
    }

    /// `open_line_with`, the URL to fill in given (`:new` fills the one of the buffer it left).
    pub fn open_line_filled(&mut self, text: &str, page: &str) {
        self.open_line();
        self.line.insert_str(text);
        self.recompute_ghost();
        self.maybe_fill_url(page);
    }

    /// `:openExternal`: checks the scheme and queues the hand-over (the message comes from it).
    fn open_external(&mut self, url: &str) {
        match crate::external::check_url(url) {
            Ok(url) => self.effects.push(Effect::OpenExternal(url)),
            Err(m) => self.fail(m),
        }
    }

    /// C-CMDRESULT: a command's error. The text goes to the bar as any message does, and the
    /// command is marked failed, which is what `web.cmd`'s promise reports as `ok: false`. Every
    /// error a command sets goes through here (never a message parsed for a `web: ` prefix).
    pub(crate) fn fail(&mut self, message: String) {
        self.cmd_failed = true;
        self.message = message;
    }

    /// Splits a submitted line into its name and arguments and runs it.
    pub fn run_command_line(&mut self, input: &str) {
        let mut parts = input.split_whitespace();
        let Some(name) = parts.next() else { return };
        let args: Vec<&str> = parts.collect();
        if name == "let" {
            let raw = input.trim_start().strip_prefix(name).unwrap_or("");
            let m = crate::registers::let_message(raw, &self.url, self.clipboard.as_deref_mut());
            // Every message of `:let` that is not `COPIED` is an error (they all start with `web: `
            // or are LET_USAGE); the check is against the one success text, not a prefix.
            if m == crate::registers::COPIED {
                self.message = m;
            } else {
                self.fail(m);
            }
            return;
        }
        // C-ECHO: the rest of the line, verbatim (not split on whitespace).
        if name == "echo" {
            let raw = input.trim_start().strip_prefix(name).unwrap_or("");
            self.echo(raw.trim_start());
            return;
        }
        self.dispatch_command(name, &args);
    }

    /// What each command does. `COMMAND_NAMES` is its completion shadow.
    /// `:q` on the help page goes back to the page before it (as scratchy's `:q` closed
    /// `:help`); anywhere else it quits.
    pub fn dispatch_command(&mut self, name: &str, args: &[&str]) {
        let send = |me: &mut Keys, c: Command| me.effects.push(Effect::Send(c));
        match (name, args) {
            ("go", [word]) => match self.go_target(word) {
                Ok(url) => send(self, Command::Open { url }),
                Err(m) => self.fail(m),
            },
            ("go", _) => self.fail("web: usage: :go <url>".to_string()),
            // C-NEWBUF (KEYS.md T103-T104): a new buffer; the argument is `:go`'s.
            ("new", rest) | ("bufferNew", rest) => self.buffer_new(rest),
            // C-EXTERNAL (KEYS.md "Opening a page in another browser").
            ("openExternal", []) => {
                let url = self.url.clone();
                self.open_external(&url)
            }
            ("openExternal", [word]) => match go_url(word, &self.url) {
                Ok(url) => self.open_external(&url),
                Err(m) => self.fail(m),
            },
            ("openExternal", _) => self.fail("web: usage: :openExternal [url]".to_string()),
            // C-BOOKMARKS (KEYS.md "Bookmarks").
            ("bookmarkAdd", []) => self.bookmark_add(),
            ("bookmarkAdd", _) => self.fail("web: usage: :bookmarkAdd".to_string()),
            ("bookmarkDelete", []) => {
                let url = self.url.clone();
                self.bookmark_delete(&url)
            }
            ("bookmarkDelete", [url]) => self.bookmark_delete(url),
            ("bookmarkDelete", _) => self.fail("web: usage: :bookmarkDelete [url]".to_string()),
            ("bookmarkSearch", rest) => self.bookmark_search(&rest.join(" ")),
            ("historyDelete", []) => {
                let url = self.url.clone();
                self.history_delete(&url)
            }
            ("historyDelete", [url]) => self.history_delete(url),
            ("historyDelete", _) => self.fail("web: usage: :historyDelete [url]".to_string()),
            ("historyBack", []) => send(self, Command::Back),
            ("historyFront", []) => send(self, Command::Forward),
            // C-PICK: the picker over the history; an argument is the filter already typed.
            ("historySearch", rest) => self.history_search(&rest.join(" ")),
            ("refresh", []) => send(self, Command::Reload { hard: false }),
            ("refresh", ["hard"]) => send(self, Command::Reload { hard: true }),
            // `:forget-site [site]`: the site is the page's own when omitted.
            ("forget-site", rest) if rest.len() <= 1 => {
                let site = rest.first().map(|s| s.to_string()).unwrap_or_else(|| self.url.clone());
                if site.is_empty() || (rest.is_empty() && !site.starts_with("http")) {
                    self.fail("web: :forget-site: no http(s) page to forget; give a site".to_string());
                } else {
                    send(self, Command::ForgetSite { site })
                }
            }
            ("forget-site", _) => self.fail("web: usage: :forget-site [site or URL]".to_string()),
            ("help", []) => {
                let url = self.help_url.clone();
                send(self, Command::Open { url })
            }
            // C4 (KEYS.md "Buffers"): camelCase names with vim's aliases.
            ("bufferList", rest) | ("ls", rest) => self.buffer_list(&rest.join(" ")),
            ("buffer", rest) | ("b", rest) => self.buffer_command(rest),
            ("bufferNext", []) | ("bn", []) => self.buffer_step(1),
            ("bufferPrev", []) | ("bp", []) => self.buffer_step(-1),
            ("bufferKill", rest) | ("bd", rest) => self.buffer_kill(rest),
            ("bufferDescribe", rest) => self.buffer_describe(rest),
            // C-NOTIFLIST (KEYS.md "Notifications").
            ("notifications", rest) => self.notifications(&rest.join(" ")),
            // C-POPUPS (KEYS.md "Popups").
            ("popupOpen", rest) => self.popup_open(rest),
            ("popupSearch", rest) => self.popup_search(&rest.join(" ")),
            // C-REPEAT: vim's `:@:` and `:@@`.
            ("@:", []) => self.repeat_line(),
            ("@@", []) => self.repeat_repeat(),
            ("@:" | "@@", _) => self.fail(format!("web: usage: :{name}")),
            ("echo", rest) => self.echo(&rest.join(" ")),
            // C-SOURCE: the scripts again, clean (arguments: not yet).
            ("source", []) => self.source(),
            ("source", _) => self.fail("web: usage: :source (no arguments yet)".to_string()),
            ("set", [opt]) => self.set_option(opt),
            ("set", _) => self.fail(SET_USAGE.to_string()),
            ("q", []) | ("quit", []) => {
                if !self.help_url.is_empty() && self.url == self.help_url {
                    send(self, Command::Back)
                } else {
                    self.effects.push(Effect::Quit)
                }
            }
            // C-SCRIPT: a command a script defined (`web.command`; uppercase, so never a built-in).
            _ if self.user_command(name, args) => {}
            _ => {
                let rest = args.join(" ");
                self.fail(format!(
                    "web: unknown command: :{name}{}{rest}",
                    if rest.is_empty() { "" } else { " " }
                ));
            }
        }
    }

    /// `:set <opt>` (C-CARETCMD): `caret` / `nocaret` switch Chromium's caret browsing,
    /// `caret?` says which is on (vim's `:set opt?`).
    fn set_option(&mut self, opt: &str) {
        let on = match opt {
            "caret" => true,
            "nocaret" => false,
            "caret?" => {
                self.message = if self.caret { "caret" } else { "nocaret" }.to_string();
                return;
            }
            _ => {
                self.fail(format!("web: unknown option: {opt} ({})", SET_USAGE.trim_start_matches("web: ")));
                return;
            }
        };
        self.caret = on;
        self.effects.push(Effect::Send(Command::Caret { on }));
        self.message = if on {
            "caret browsing on (:set nocaret turns it off)".to_string()
        } else {
            "caret browsing off".to_string()
        };
    }

    /// What the status bar shows now.
    pub fn bar_view(&self) -> BarView {
        let prompt = match self.vim_mode {
            VimMode::Command => Some(':'),
            VimMode::Search => Some('/'),
            VimMode::Pick => Some('>'),
            VimMode::Normal => None,
        };
        // C-PICKDEL: while the picker asks, the question takes the filter line's place.
        let asking = self.picker.as_ref().and_then(|p| p.confirm.as_ref().map(|r| (p.name.as_str(), r)));
        let question = asking.map(|(name, row)| match (name, &row.action) {
            ("history", crate::picker::PickAction::Open(u)) => format!("delete history entry {u}? (y/n)"),
            ("bookmarks", crate::picker::PickAction::Open(u)) => format!("delete bookmark {u}? (y/n)"),
            (_, crate::picker::PickAction::Buffer(n)) => {
                let label = row.label.split_once(' ').map_or(row.label.as_str(), |(_, l)| l);
                format!("kill buffer {n} {label}? (y/n)")
            }
            _ => String::new(),
        });
        let prompt = if question.is_some() { None } else { prompt };
        let line = prompt.map(|p| (p, self.line.text.clone(), self.line.cursor, self.line.sel));
        BarView {
            mode: mode_name(self.key_mode).to_string(),
            // C4: the current buffer's number before the URL; C-BELL: before it, how many buffers
            // have unread notifications (none: nothing).
            url: match self.unread_buffers() {
                0 => format!("[{}] {}", self.buffers.current, self.url),
                n => format!("[{n}!] [{}] {}", self.buffers.current, self.url),
            },
            flash: self.bell.lit,
            message: match question {
                Some(q) => q,
                // C-PICKMSG: PICK draws the filter line, so the message beside it is the one set in PICK.
                None if self.vim_mode == VimMode::Pick => self.pick_msg.clone(),
                None => self.message.clone(),
            },
            ghost: match (&self.ghost, &line) {
                (Some(g), Some(_)) => g.rest(&self.line.text),
                _ => String::new(),
            },
            line,
        }
    }

    /// The keys held as scroll input (tests).
    #[cfg(test)]
    pub fn held(&self) -> &[(u32, u32)] {
        &self.held
    }
}
