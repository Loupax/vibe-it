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
use crate::config::{ScrollInput, BINDINGS, CLICK_FOCUS_MS, ESC_ESC_MS, KEY_SEQUENCES, SCROLL_INPUTS, START_MODE};
use crate::protocol::{Command, FindDir};

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

/// The NORMAL sub-state: the bindings, the `:` line (KEYS.md COMMAND) or the `/` line
/// (SEARCH).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Command,
    Search,
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
}

/// Whether `action` ignores OS key repeat (T14 `h`/`l` "not a repeat", T55).
pub fn action_ignores_repeat(action: Action) -> bool {
    !matches!(action, Action::Scroll(_))
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

/// The status bar's mode name. ESC1 and a pending `g`/`y` show their parent mode.
pub fn mode_name(mode: KeyMode) -> &'static str {
    match mode {
        KeyMode::Input => "INPUT",
        KeyMode::Normal => "NORMAL",
    }
}

/// Every name `dispatch_command` knows as the first word (what the first word completes
/// from). Keep it in step with `dispatch_command`'s `match` by hand.
pub const COMMAND_NAMES: &[&str] = &["go", "help", "historyBack", "historyFront", "let", "q", "quit", "refresh"];

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
}

impl TextEditBuffer {
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
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
    /// Quit web (`:q`).
    Quit,
}

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
    /// The id of the oldest `find` still waiting for its result.
    #[cfg(test)]
    pub fn pending_id(&self) -> Option<u64> {
        self.waiting.front().map(|w| w.0)
    }
}
pub const NOT_YET_PASTE: &str = "web: paste needs the clipboard, not built yet (web/TODO.txt C6)";

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

/// The status bar's contents (drawn by `statusbar.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarView {
    pub mode: String,
    pub url: String,
    pub message: String,
    /// While the `:` line is open: the prompt, the text and the cursor (it takes the strip).
    pub line: Option<(char, String, usize)>,
    /// The dimmed ghost after the line's text (empty = none).
    pub ghost: String,
}

/// The shell's keyboard state (KEYS.md), fed by the compositor.
pub struct Keys {
    pub key_mode: KeyMode,
    pub vim_mode: VimMode,
    esc: EscState,
    pending_seq: Option<char>,
    pub line: TextEditBuffer,
    pub ghost: Option<Ghost>,
    pub cache: CompleteCache,
    /// The last message (KEYS.md "Status bar").
    pub message: String,
    /// The page's URL as the host reported it (register `%`).
    pub url: String,
    /// `:help`'s page.
    pub help_url: String,
    /// The kept query, the host's find session, the last result (T7, T15-T25).
    pub search: SearchState,
    state_dir: Option<PathBuf>,
    sink: Option<ExtSink>,
    /// Key codes held down (a press of one already down is an OS repeat).
    down: HashSet<u32>,
    /// Key codes whose press reached the page: exactly those get their release (T33, T34).
    forwarded: HashSet<u32>,
    /// Scroll keys given to the page, by the key code of the binding holding them.
    held: Vec<(u32, u32)>,
    /// What the compositor must do next (drained by it).
    pub effects: Vec<Effect>,
    /// xdg_popups currently open (a submenu counts too): a popup session ends when the last closes.
    open_popups: usize,
    /// T57: the mode the user's click left behind when its popup switched to INPUT; restored when the
    /// popup session ends, unless the click focused a field (`user_focused_control`).
    popup_return: Option<KeyMode>,
}

impl Keys {
    pub fn new(state_dir: Option<PathBuf>, help_url: String, sink: Option<ExtSink>) -> Keys {
        Keys {
            key_mode: START_MODE,
            vim_mode: VimMode::Normal,
            esc: EscState::Idle,
            pending_seq: None,
            open_popups: 0,
            popup_return: None,
            line: TextEditBuffer::default(),
            ghost: None,
            cache: CompleteCache::new(state_dir.clone()),
            message: String::new(),
            url: String::new(),
            help_url,
            search: SearchState::default(),
            state_dir,
            sink,
            down: HashSet::new(),
            forwarded: HashSet::new(),
            held: Vec::new(),
            effects: Vec::new(),
        }
    }

    /// One key event (`code`: the key's code, any numbering as long as press and release
    /// agree). Returns whether the page gets it.
    pub fn key(&mut self, code: u32, pressed: bool, key: &KeyInput, now: Instant) -> bool {
        if !pressed {
            self.down.remove(&code);
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
        let repeat = !self.down.insert(code);
        let forward = self.press(code, key, repeat, now);
        if forward {
            self.forwarded.insert(code);
        }
        forward
    }

    fn press(&mut self, code: u32, key: &KeyInput, repeat: bool, now: Instant) -> bool {
        match self.key_mode {
            KeyMode::Normal => {
                match self.vim_mode {
                    VimMode::Command => self.command_line_key(key),
                    VimMode::Search => self.search_line_key(key),
                    VimMode::Normal => self.normal_key(code, key, repeat),
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

    /// A key press in NORMAL (T6-T21): a pending sequence first, then the bindings.
    fn normal_key(&mut self, code: u32, key: &KeyInput, repeat: bool) {
        let (seq_action, pending, consumed) = resolve_sequence(self.pending_seq, key);
        self.pending_seq = pending;
        let action = if consumed {
            seq_action
        } else {
            resolve_binding(key, self.vim_mode)
        };
        if let Some(action) = action {
            if !(repeat && action_ignores_repeat(action)) {
                self.perform(code, action, repeat);
            }
        }
    }

    fn perform(&mut self, code: u32, action: Action, repeat: bool) {
        match action {
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
            Action::EnterInputMode => self.set_key_mode(KeyMode::Input),
            Action::HistoryBack => self.effects.push(Effect::Send(Command::Back)),
            Action::HistoryForward => self.effects.push(Effect::Send(Command::Forward)),
            Action::YankUrl => self.run_command_line("let @+ = @%"),
        }
    }

    /// Switches the mode. Leaving NORMAL drops a pending `g`/`y` and an open `:` line.
    pub fn set_key_mode(&mut self, mode: KeyMode) {
        self.key_mode = mode;
        self.esc = EscState::Idle;
        self.pending_seq = None;
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

    fn open_line(&mut self) {
        self.vim_mode = VimMode::Command;
        self.line.clear();
        self.ghost = None;
        // A new cache per line: the history is read once per line (KEYS.md "Per keystroke cost").
        let mut cache = CompleteCache::new(self.state_dir.clone());
        if let Some(sink) = &self.sink {
            cache = cache.with_external(sink.clone(), self.url.clone());
        }
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
        self.send_find(&query, FindDir::Next, true);
        self.search.query = Some(query);
    }

    fn send_find(&mut self, query: &str, dir: FindDir, fresh: bool) {
        self.search.session = true;
        self.search.last_id += 1;
        let id = self.search.last_id;
        self.search.waiting.push_back((id, dir, fresh));
        self.effects.push(Effect::Send(Command::Find {
            id,
            query: query.to_string(),
            dir,
            match_case: smartcase_match_case(query),
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
        self.send_find(&query, dir, fresh);
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
        self.search.session = false;
        self.search.index = 0;
        self.search.count = 0;
        self.search.waiting.clear();
    }

    /// The host's `find-result` for the oldest `find` still waiting (T15, T16, T23): the
    /// status bar gets `Pattern not found: <q>`, or `[i/N]` with a wrap message when the
    /// index went the wrong way (next: not higher than before; previous: not lower).
    ///
    /// C5-STALE: `id` is the request it answers. A result whose id is not waiting (CEF's
    /// `0/0` at a load start, an answer to a find that a navigation or Esc dropped) is
    /// ignored; the requests older than the answered one will never be answered (the host
    /// answers only the latest) and are dropped with it.
    pub fn find_result(&mut self, id: u64, index: u32, count: u32) {
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
        let wrapped = !fresh
            && old > 0
            && match dir {
                FindDir::Next => index <= old,
                FindDir::Prev => index >= old,
            };
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

    fn close_line(&mut self) {
        self.vim_mode = VimMode::Normal;
        self.line.clear();
        self.ghost = None;
    }

    /// A key in the `:` line (T26-T28, T45-T53).
    fn command_line_key(&mut self, key: &KeyInput) {
        let s = key.sym;
        if s == Keysym::Escape {
            self.close_line();
        } else if s == Keysym::Return || s == Keysym::KP_Enter {
            let mut input = std::mem::take(&mut self.line.text);
            if let Some(g) = self.ghost.take() {
                input = g.accepted(&input);
            }
            self.close_line();
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
        }
    }

    fn line_edit_key(&mut self, key: &KeyInput) {
        let s = key.sym;
        let paste = (key.ctrl && matches!(key.ch, Some('v') | Some('V'))) || (key.shift && s == Keysym::Insert);
        if paste {
            self.message = NOT_YET_PASTE.to_string();
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

    /// Splits a submitted line into its name and arguments and runs it.
    pub fn run_command_line(&mut self, input: &str) {
        let mut parts = input.split_whitespace();
        let Some(name) = parts.next() else { return };
        let args: Vec<&str> = parts.collect();
        if name == "let" {
            let raw = input.trim_start().strip_prefix(name).unwrap_or("");
            self.message = crate::registers::let_message(raw, &self.url);
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
            ("go", [word]) => match go_url(word, &self.url) {
                Ok(url) => send(self, Command::Open { url }),
                Err(m) => self.message = m,
            },
            ("go", _) => self.message = "web: usage: :go <url>".to_string(),
            ("historyBack", []) => send(self, Command::Back),
            ("historyFront", []) => send(self, Command::Forward),
            ("refresh", []) => send(self, Command::Reload { hard: false }),
            ("refresh", ["hard"]) => send(self, Command::Reload { hard: true }),
            ("help", []) => {
                let url = self.help_url.clone();
                send(self, Command::Open { url })
            }
            ("q", []) | ("quit", []) => {
                if !self.help_url.is_empty() && self.url == self.help_url {
                    send(self, Command::Back)
                } else {
                    self.effects.push(Effect::Quit)
                }
            }
            _ => {
                let rest = args.join(" ");
                self.message = format!("web: unknown command: :{name}{}{rest}", if rest.is_empty() { "" } else { " " });
            }
        }
    }

    /// What the status bar shows now.
    pub fn bar_view(&self) -> BarView {
        let prompt = match self.vim_mode {
            VimMode::Command => Some(':'),
            VimMode::Search => Some('/'),
            VimMode::Normal => None,
        };
        let line = prompt.map(|p| (p, self.line.text.clone(), self.line.cursor));
        BarView {
            mode: mode_name(self.key_mode).to_string(),
            url: self.url.clone(),
            message: self.message.clone(),
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
