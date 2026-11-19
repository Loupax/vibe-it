use super::*;
use crate::js_style;

// ---------- windowing ----------

/// A fully-fetched, ready-to-display page — the `Ok` payload of
/// `AppEvent::NavResult` (v67). `html` is ready-to-parse bytes (same
/// bytes `Input::File`'s own `fs::read_to_string` would have read from a
/// saved page — already dispatched through `dispatch_wire_response`, so
/// a fetched image has already been wrapped into a synthetic HTML
/// document by this point too); `extra_css` is every external
/// stylesheet `process_external_resources` found and fetched for this
/// page, concatenated — see `App::extra_css`'s own docs. `url` is the
/// real address this page was fetched from — needed as `NavResult`'s
/// own fallback base URL (v68) for a fetched page that declares no
/// `<base href>` of its own, the overwhelmingly common real-world case
/// (a real browser's own "document base URL" already defaults to
/// exactly this, same as `snag`'s own bundled pages already get via a
/// synthesized `<base href>` — see `find_base_url`'s own docs). All
/// three are plain owned data (`Vec<u8>`/`String`/`Url`, all `Send`),
/// deliberately never a parsed `Handle` (an `Rc`-based type — `!Send` —
/// see `process_external_resources`'s own docs for why its own DOM walk
/// never crosses this same thread boundary).
pub(crate) struct FetchedPage {
    pub(crate) html: Vec<u8>,
    pub(crate) extra_css: String,
    pub(crate) scripts: Vec<ScriptSource>,
    pub(crate) url: Url,
}

pub(crate) enum AppEvent {
    /// QM1g: mpv has a new video frame (posted from its render-update thread).
    VideoFrame,
    Chunk(Vec<u8>),
    Eof,
    /// The result of a background navigation fetch (`App::start_
    /// navigation`'s spawned thread) — `Err` is a human-readable message
    /// to show in place of the page (no `--socket` configured, the
    /// daemon unreachable, or a response `wire` sent back in a content
    /// type scratchy can't render — see `fetch_raw_via_wire`'s own docs
    /// for exactly which). The `u64` is `App::nav_generation` at the
    /// moment this navigation started — compared against the *current*
    /// generation when this event arrives so a stale result from a
    /// since-superseded navigation (the user clicked a second link
    /// before the first one's fetch finished) is silently dropped
    /// instead of clobbering newer, already-displayed content. `App::
    /// history`'s own back/forward moves (`navigate_history`) are built
    /// on top of exactly this same generation check, via the shared
    /// `fetch_url` path — a back/forward move the user has since moved
    /// on from gets dropped the same way a stale forward navigation
    /// does.
    NavResult(u64, Result<FetchedPage, String>),
    /// QF7: a page-JS network request finished (or was refused at once):
    /// `(nav_generation, request id, result)`. Stale generations are dropped.
    JsNet(u64, u64, Result<WireResponse, NetError>),
    /// QF19w2: WebSocket events are queued for the current page's engine.
    JsWs,
    /// One non-`data:` `<img src>` reference's own fetch finishing (v67)
    /// — `String` is the exact, unresolved `src` attribute text
    /// `process_external_resources` found it under (see `App::image_
    /// cache`'s own docs for why that's the right cache key), `None` for
    /// any failure (unreachable daemon, non-image content type, corrupt/
    /// unsupported image data — `fetch_image_via_wire`'s own docs). Each
    /// arrival inserts (or, on failure, simply doesn't) one cache entry
    /// and marks layout dirty — images materialize one at a time as
    /// their own fetches land, the page never blocks on all of them the
    /// way `extra_css` necessarily does. Tagged with the same `nav_
    /// generation` convention `NavResult` already uses, for the same
    /// reason: a since-superseded page's late-arriving image must never
    /// populate the *new* page's cache.
    ImageFetched(u64, String, Option<DecodedImage>),
    /// Q22b: the stylesheets of a streamed page, fetched after EOF
    /// (`start_stream_resources`); replaces `App::extra_css`.
    StreamCss(u64, String, Vec<ScriptSource>),
    /// One tick of the persistent background shaping worker's own
    /// progress (`App::spawn_shaping_worker`) — the "move page shaping
    /// off the main thread" plan in TODO.txt, landed, fires *once per
    /// tick*, not once per page: a big page arrives as a short burst of
    /// these, each one a little further along, which is what lets it
    /// fill in progressively again on the main thread despite shaping
    /// happening on a different thread entirely (see `spawn_shaping_
    /// worker`'s own docs for why that took a second attempt). Tagged
    /// with `App::layout_generation` at the moment the request that
    /// produced this was queued, same `NavResult`/`ImageFetched`
    /// convention: compared against the *current* generation on
    /// arrival, a mismatch (a resize, a new chunk, a newly-arrived
    /// image, a navigation — anything that bumped `layout_generation`
    /// again before this landed) means the reshape this was computing
    /// is since-superseded, dropped silently instead of clobbering
    /// newer content. The `ShapeCache` crosses back too, growing a
    /// little with each successive delivery for the same generation —
    /// `paint` needs the *matching* cache for whatever ids this exact
    /// `Laid` references, so `App::shape_cache` is replaced wholesale
    /// on arrival, never merged with whatever was there before (always
    /// safe here: each delivery's cache is a strict superset of the
    /// last one's, same tree, same worker, nothing removed between
    /// ticks). No `incomplete` flag, unlike `measure`'s own parameter
    /// of that name — nothing downstream needs to tell a mid-page
    /// delivery apart from the final one; both just replace `cached`/
    /// `shape_cache` the same way. `Laid` itself is boxed — otherwise
    /// this variant alone (`Laid` plus a whole `ShapeCache`) would
    /// dwarf every other `AppEvent` variant's size and every one of
    /// them pays for that on the stack (`clippy::large_enum_variant`);
    /// `self.cached: Option<Laid>` on the receiving end stays unboxed,
    /// same as before this plan.
    ShapeResult(u64, Box<Laid>, ShapeCache, bool),
    /// A deferred hover restyle (R2.3) is due; see `App::update_hover`.
    HoverFlush,
    /// Q35b2: the image-relayout debounce window (`IMAGE_RELAYOUT_DEBOUNCE`)
    /// has passed; see `App::flush_image_relayout`.
    ImageFlush,
    /// QY19: a `<meta http-equiv=refresh>` delay elapsed for the page of
    /// navigation generation `.0`; follow `.1` unless the user has since
    /// navigated (generation changed).
    MetaRefresh(u64, Url),
    /// `SIGUSR1`'s own background thread (`run()`) waking the event
    /// loop — the out-of-band bootstrap the vim-mode plan in TODO.txt
    /// settled on for turning vim mode back *on* once it's off: with
    /// `vim_enabled == false`, scratchy intercepts zero keys, so
    /// nothing in-band (no keystroke, however it's bound) could ever
    /// type `:set vim` again. A signal sent to the process sidesteps
    /// that entirely, immune to both a WM claiming a reserved chord and
    /// unusual keyboard-firmware Unicode-input quirks (the two reasons
    /// a dedicated keyboard chord was considered and dropped — see
    /// TODO.txt). Toggles rather than only turning vim mode on, so the
    /// same external signal also works as an off switch.
    ToggleVimMode,
}

/// `Normal` resolves bare keys into an `Action` via `resolve_binding`;
/// `Command` is the `:`-entered command line itself, editing `App::
/// command_line` directly rather than going through any binding at
/// all. Meaningless while `App::vim_enabled` is `false` — vim mode off
/// always means `Normal` with every key ignored by `resolve_binding`
/// (`mode` is checked there too, not just by the caller), not a third
/// state of its own (see `App::handle_key_event`'s own docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VimMode {
    Normal,
    Command,
}

/// The effect a resolved Normal-mode binding actually has — kept as its
/// own small enum, not performed inline inside `resolve_binding` itself,
/// so that function can stay pure (no `&mut App`) and unit-testable on
/// its own, same reasoning `resolve_href`'s own split from `start_
/// navigation` already follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    ScrollDown,
    ScrollUp,
    HalfPageDown,
    HalfPageUp,
    EnterCommandMode,
    HistoryBack,
    HistoryFront,
    /// `gg` / `Home`.
    ScrollTop,
    /// `G` / `End`.
    ScrollBottom,
}

/// Whether `action` must ignore OS key-repeat rather than fire on every
/// repeat tick while the key stays held. A continuous action (`Scroll*`/
/// `HalfPage*`) is supposed to keep firing on repeat, same as any
/// ordinary scrollable app — that's the whole point of holding the key
/// down. `HistoryBack`/`HistoryFront` are the opposite: `navigate_
/// history` only moves `history_index` and kicks off a fetch, with
/// nothing gating it on that fetch actually completing, so an un-gated
/// repeat fires many times before a single `NavResult` can land —
/// decrementing straight through the whole stack in the time it takes
/// to notice the key was held at all, not one page per press (the exact
/// bug this function exists to close). `EnterCommandMode` is harmless
/// either way — repeating it just re-clears an already-empty command
/// line — but listed here too since repeating it serves no purpose.
pub(crate) fn action_ignores_repeat(action: Action) -> bool {
    matches!(action, Action::HistoryBack | Action::HistoryFront | Action::EnterCommandMode)
}

/// The one chokepoint a real key event gets resolved into an `Action`
/// through — hardcoded `match` arms today (dwm `config.h` style,
/// explicitly fine per the user's own call in TODO.txt), but the exact
/// spot a future JS-scriptable `bind()` config layer would call into or
/// override instead of a redesign. `mode` gates this to `VimMode::
/// Normal` even though every caller today only ever calls this from
/// that branch already — kept as a real parameter (not dropped) so a
/// future third mode (link hints — see TODO.txt's own "explicitly out
/// of scope for now" note) extends this one `match`'s arm list instead
/// of requiring a new chokepoint. `mods` is checked, not `logical_key`
/// alone, only for the one binding that needs it (`Ctrl`+`d`/`u`) —
/// `winit`'s own `logical_key` is already shift-aware (`Key::
/// Character(":")` for `Shift`+`;`), so bare `j`/`k`/`:` need no
/// modifier check at all.
pub(crate) fn resolve_binding(key: &Key, mods: ModifiersState, mode: VimMode) -> Option<Action> {
    if mode != VimMode::Normal {
        return None;
    }
    match key {
        Key::Character(s) if s.as_str() == "j" => Some(Action::ScrollDown),
        Key::Character(s) if s.as_str() == "k" => Some(Action::ScrollUp),
        Key::Character(s) if s.as_str() == "d" && mods.control_key() => Some(Action::HalfPageDown),
        Key::Character(s) if s.as_str() == "u" && mods.control_key() => Some(Action::HalfPageUp),
        Key::Character(s) if s.as_str() == ":" => Some(Action::EnterCommandMode),
        Key::Character(s) if s.as_str() == "h" => Some(Action::HistoryBack),
        Key::Character(s) if s.as_str() == "l" => Some(Action::HistoryFront),
        Key::Character(s) if s.as_str() == "G" => Some(Action::ScrollBottom),
        Key::Named(NamedKey::Home) => Some(Action::ScrollTop),
        Key::Named(NamedKey::End) => Some(Action::ScrollBottom),
        _ => None,
    }
}

/// The pending-`g` state machine for vim's `gg`: given whether a `g` is
/// already pending and the key just pressed, returns the action to run
/// (if any), the new pending flag, and whether the key was consumed (so
/// the caller must not also run `resolve_binding` on it). Bare `g`
/// arms; a second bare `g` fires `ScrollTop`; every other key (Esc
/// included) clears the flag and falls through unconsumed.
pub(crate) fn resolve_pending_g(pending: bool, key: &Key, mods: ModifiersState) -> (Option<Action>, bool, bool) {
    let is_g = matches!(key, Key::Character(s) if s.as_str() == "g") && !mods.control_key() && !mods.alt_key();
    match (is_g, pending) {
        (true, true) => (Some(Action::ScrollTop), false, true),
        (true, false) => (None, true, true),
        (false, _) => (None, false, false),
    }
}

/// What a key event should insert into a text-editing buffer, if
/// anything at all — the one chokepoint `Key::Named` vs. `Key::
/// Character` gets resolved into actual insertable text through,
/// instead of leaving every text-editing call site (today: the command
/// line; later: a focused `<input>`, per `TextEditBuffer`'s own docs)
/// to independently remember the one surprising case: `winit` reports
/// the space bar as `Key::Named(NamedKey::Space)`, *not* `Key::
/// Character(" ")` — confirmed the hard way, post-merge (what was then
/// `:visit https://...` landed as `:visithttp...`, the space silently
/// dropped by a bare `_ => {}` catch-all that only matched `Key::
/// Character`; the command itself is `:go` now, same bug either way).
/// Kept pure and separate from `resolve_binding` (same reasoning that
/// one's own docs give for staying a plain `key -> Option<Action>` map)
/// so this exact class of bug — a `Key::Named` variant quietly falling
/// through a wildcard arm — has a real unit test instead of only
/// surfacing by hand-testing every key on a real keyboard.
pub(crate) fn key_insertion_text(key: &Key) -> Option<&str> {
    match key {
        Key::Named(NamedKey::Space) => Some(" "),
        Key::Character(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Every name `dispatch_command` recognizes as the *first* word of a
/// command line — the candidate pool `Tab` completion offers there. A
/// flat list purely for completion purposes, kept next to (not inside)
/// `dispatch_command`'s own `match`, which stays the one real source of
/// truth for what a command actually does: a name landing here with no
/// matching `dispatch_command` arm would complete to something that
/// then reports itself unknown, not silently do nothing.
/// QY48: `history_doc` value of an entry whose document is not loaded.
pub(crate) const NO_DOC: u64 = u64::MAX;

pub(crate) const COMMAND_NAMES: &[&str] = &["go", "help", "historyBack", "historyFront", "q", "quit", "refresh", "set"];

/// `:set`'s own one valid argument, completed the same way `COMMAND_
/// NAMES` completes a command name itself — see `command_completions`.
pub(crate) const SET_ARGS: &[&str] = &["js", "motion", "nojs", "nomotion", "novim", "vim"];

/// Candidate completions for the token the command line's cursor is
/// currently positioned at the end of (`token_start..cursor`, from
/// `token_start_before`) — the command name itself if `token_start` is
/// the very first token, `SET_ARGS` if it's the second token of a
/// `set` command, or nothing at all anywhere else (an arbitrary `:go`
/// URL has no finite set of "correct" values to complete against, so
/// there's deliberately no candidate pool for it). Plain-prefix
/// (`starts_with`) matching only, sorted alphabetically (`COMMAND_
/// NAMES`/`SET_ARGS` are both already stored in that order) — no fuzzy
/// or substring matching, the smallest thing that actually satisfies
/// "complete the command name," not a speculative ranking algorithm
/// nobody asked for.
pub(crate) fn command_completions(text: &str, token_start: usize, cursor: usize) -> Vec<&'static str> {
    let prefix = &text[token_start..cursor];
    let before = text[..token_start].trim_end();
    let pool: &[&str] = if before.is_empty() {
        COMMAND_NAMES
    } else if before == "set" {
        SET_ARGS
    } else {
        return Vec::new();
    };
    pool.iter().copied().filter(|candidate| candidate.starts_with(prefix)).collect()
}

/// The byte offset the token ending at `cursor` starts at — the last
/// whitespace position before `cursor`, plus one, or `0` if there isn't
/// one. `rfind` (not a manual byte scan) so this can't land `token_
/// start` mid-codepoint: `str::rfind` only ever returns a char-boundary
/// index, same guarantee `TextEditBuffer`'s own methods rely on `char`-
/// at-a-time movement for.
pub(crate) fn token_start_before(text: &str, cursor: usize) -> usize {
    text[..cursor].rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0)
}

/// `Tab`'s own live cycling state inside the command bar — `App::
/// completion` is `None` whenever no completion is in progress. Reset
/// on every command-line keystroke except a repeated `Tab` itself (see
/// `App::handle_command_line_key`), so cycling only continues across
/// *consecutive* `Tab` presses: any other edit starts over from a
/// fresh match list next time, rather than cycling through candidates
/// that no longer correspond to what's actually typed.
pub(crate) struct CommandCompletion {
    pub(crate) token_start: usize,
    pub(crate) candidates: Vec<&'static str>,
    pub(crate) index: usize,
}

/// A single-line text-editing buffer — insert, backspace, cursor move,
/// nothing else — written standalone (knows nothing about `App`,
/// chrome, or the page it might eventually sit over) on purpose: the
/// Form elements plan's own step 4 (TODO.txt) needs the *exact* same
/// primitive for a focused `<input type=text>`'s own editing later, and
/// the command line below is meant to be its first caller, not its
/// only one. `cursor` is always a byte offset on a `char` boundary
/// within `text` — every method below moves it by exactly one `char`'s
/// `len_utf8()`, never a raw byte, so it can't land mid-codepoint.
#[derive(Default)]
pub(crate) struct TextEditBuffer {
    pub(crate) text: String,
    pub(crate) cursor: usize,
    /// Q30d: the selection's fixed end (a byte offset on a `char` boundary);
    /// `None` = no selection. The command bar never sets it, so its
    /// behaviour is unchanged; form controls use it for Shift+arrows, mouse
    /// selection and select-all. The selection is `anchor..cursor`, either order.
    pub(crate) anchor: Option<usize>,
}

/// Q30d: the three classes a Ctrl+arrow word move distinguishes.
fn word_class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

impl TextEditBuffer {
    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
    }

    /// The ordered selection `(start, end)`, or `None` when empty.
    pub(crate) fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?.min(self.text.len());
        let c = self.cursor.min(self.text.len());
        (a != c).then(|| (a.min(c), a.max(c)))
    }

    pub(crate) fn selected_text(&self) -> Option<&str> {
        self.selection().map(|(a, b)| &self.text[a..b])
    }

    /// Removes the selected text (cursor lands at its start). True if any.
    pub(crate) fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        self.text.replace_range(a..b, "");
        self.cursor = a;
        self.anchor = None;
        true
    }

    pub(crate) fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.text.len();
    }

    pub(crate) fn insert(&mut self, c: char) {
        self.delete_selection();
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Multi-char insert at the cursor, one `insert` per char — the
    /// command line's own paste path (`App::paste_into_command_line`).
    /// Control characters (a trailing newline off the clipboard being
    /// the real-world case) are dropped rather than inserted literally:
    /// this buffer is always single-line, and a bare `\n` landing in
    /// `text` would silently desync `paint_command_bar`'s own single-
    /// `Buffer`/`Wrap::None` rendering from what `run_command_line` later
    /// splits on submit.
    pub(crate) fn insert_str(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            self.insert(c);
        }
    }

    /// Q30d: a `<textarea>` insert: `\r\n`/`\r` become `\n`, which is kept;
    /// other control characters are dropped.
    pub(crate) fn insert_multiline(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        for c in s.chars().filter(|c| *c == '\n' || !c.is_control()) {
            self.insert(c);
        }
    }

    pub(crate) fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        let Some(prev) = self.text[..self.cursor].chars().next_back() else { return };
        self.cursor -= prev.len_utf8();
        self.text.remove(self.cursor);
    }

    /// Forward delete (the Delete key).
    pub(crate) fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    pub(crate) fn move_left(&mut self) {
        if let Some(prev) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= prev.len_utf8();
        }
    }

    pub(crate) fn move_right(&mut self) {
        if let Some(next) = self.text[self.cursor..].chars().next() {
            self.cursor += next.len_utf8();
        }
    }

    /// Q30d: moves the cursor to `pos` (clamped to a `char` boundary), keeping
    /// the anchor when `extend` (Shift held), else dropping the selection.
    pub(crate) fn move_to(&mut self, pos: usize, extend: bool) {
        let mut pos = pos.min(self.text.len());
        while !self.text.is_char_boundary(pos) {
            pos -= 1;
        }
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = pos;
    }

    /// Left/Right arrow: without Shift a selection collapses to its edge.
    pub(crate) fn nav_horizontal(&mut self, right: bool, extend: bool) {
        if !extend && let Some((a, b)) = self.selection() {
            self.move_to(if right { b } else { a }, false);
            return;
        }
        let saved = self.cursor;
        if right {
            self.move_right();
        } else {
            self.move_left();
        }
        let pos = std::mem::replace(&mut self.cursor, saved);
        self.move_to(pos, extend);
    }

    /// Start of the word before the cursor (Ctrl+Left).
    pub(crate) fn word_left_pos(&self) -> usize {
        let mut it = self.text[..self.cursor].char_indices().rev().peekable();
        let mut pos = self.cursor;
        while let Some(&(i, c)) = it.peek() {
            if word_class(c) != 0 {
                break;
            }
            pos = i;
            it.next();
        }
        let Some(&(_, first)) = it.peek() else { return pos };
        let class = word_class(first);
        for (i, c) in it {
            if word_class(c) != class {
                break;
            }
            pos = i;
        }
        pos
    }

    /// End of the word after the cursor (Ctrl+Right).
    pub(crate) fn word_right_pos(&self) -> usize {
        let mut it = self.text[self.cursor..].char_indices().peekable();
        let mut pos = self.cursor;
        while let Some(&(i, c)) = it.peek() {
            if word_class(c) != 0 {
                break;
            }
            pos = self.cursor + i + c.len_utf8();
            it.next();
        }
        let Some(&(_, first)) = it.peek() else { return pos };
        let class = word_class(first);
        for (i, c) in it {
            if word_class(c) != class {
                break;
            }
            pos = self.cursor + i + c.len_utf8();
        }
        pos
    }

    /// Byte range `(start, end)` of the word (or whitespace/punctuation run)
    /// containing `pos`: what a double click selects.
    pub(crate) fn word_at(&self, pos: usize) -> (usize, usize) {
        let pos = pos.min(self.text.len());
        let class = self.text[pos..]
            .chars()
            .next()
            .or_else(|| self.text[..pos].chars().next_back())
            .map(word_class)
            .unwrap_or(0);
        let mut a = pos;
        for (i, c) in self.text[..pos].char_indices().rev() {
            if word_class(c) != class {
                break;
            }
            a = i;
        }
        let mut b = pos;
        for (i, c) in self.text[pos..].char_indices() {
            if word_class(c) != class {
                break;
            }
            b = pos + i + c.len_utf8();
        }
        (a, b)
    }

    /// Start of the logical line (after the previous `\n`) holding the cursor.
    pub(crate) fn line_start_pos(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    /// End of the logical line holding the cursor (before its `\n`).
    pub(crate) fn line_end_pos(&self) -> usize {
        self.text[self.cursor..].find('\n').map_or(self.text.len(), |i| self.cursor + i)
    }

    /// Up/Down in a multi-line buffer by LOGICAL line (a soft-wrapped line is
    /// one line here), keeping the column in chars. `None` at the first/last line.
    pub(crate) fn vertical_pos(&self, down: bool) -> Option<usize> {
        let start = self.line_start_pos();
        let col = self.text[start..self.cursor].chars().count();
        let (ls, le) = if down {
            let end = self.line_end_pos();
            if end >= self.text.len() {
                return None;
            }
            let ls = end + 1;
            (ls, self.text[ls..].find('\n').map_or(self.text.len(), |i| ls + i))
        } else {
            if start == 0 {
                return None;
            }
            let le = start - 1;
            (self.text[..le].rfind('\n').map_or(0, |i| i + 1), le)
        };
        let line = &self.text[ls..le];
        Some(ls + line.char_indices().nth(col).map_or(line.len(), |(i, _)| i))
    }
}

/// One rebuild's worth of work for `App::spawn_shaping_worker`'s own
/// persistent background thread — everything `measure_document` needs
/// that `App` itself held inline before this plan started. `tree` is
/// `LayoutBox`, built synchronously on the main thread (the DOM walk +
/// cascade match can't leave it — see `App::redraw`'s own comment)
/// just before this is queued.
pub(crate) struct ShapingRequest {
    pub(crate) tree: LayoutBox,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) generation: u64,
    /// `true` for a restyle of the same document (hover/focus state,
    /// height-only resize): the worker keeps its content-keyed shaping
    /// memo so unchanged paragraphs aren't reshaped. `false` clears it.
    pub(crate) reuse_shaping: bool,
}

pub(crate) struct App {
    pub(crate) document: Handle,
    pub(crate) decoder: Option<Utf8LossyDecoder<Parser<RcDom>>>,
    pub(crate) window: Option<Rc<Window>>,
    pub(crate) surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    pub(crate) font_system: FontSystem,
    pub(crate) swash_cache: SwashCache,
    pub(crate) scroll_offset: f32,
    /// Cached layout from the last `document`/width combination. Scrolling
    /// alone never invalidates this — relaying out and reshaping the whole
    /// page (expensive: real measurements showed full-document shaping can
    /// take seconds under full bidi/complex-script shaping, worse on large
    /// or RTL-heavy pages) on every mouse-wheel tick is what made scrolling
    /// feel laggy.
    pub(crate) cached: Option<Laid>,
    pub(crate) cached_width: u32,
    /// Window height the cached tree was built for (`vh` units and the
    /// initial containing block depend on it).
    pub(crate) cached_height: u32,
    pub(crate) layout_dirty: bool,
    /// Shaped paragraphs matching whatever `Laid` tree is currently in
    /// `cached`, keyed by the stable id `measure` assigned them when it
    /// was built — `paint` looks a `LaidContent::Text(id)` up in here.
    /// Replaced wholesale, never merged, every time an `AppEvent::
    /// ShapeResult` lands from the background shaping worker (see that
    /// type's own docs) — not touched in between arrivals, so `cached`/
    /// `shape_cache` stay a consistent pair for `paint` to read at any
    /// point. Since the worker reports progress one tick at a time
    /// (`App::spawn_shaping_worker`), this normally updates every
    /// `PROGRESSIVE_LAYOUT_TICK_BUDGET` or so while a page is still
    /// shaping, each delivery a little more complete than the last —
    /// what makes a big page fill in progressively again despite
    /// shaping happening off the main thread entirely.
    pub(crate) shape_cache: ShapeCache,
    /// The size `surface.resize()` was last called with — skip the call
    /// when nothing changed instead of re-issuing it on every redraw.
    pub(crate) surface_size: (u32, u32),
    /// The width seen on the previous redraw, tracked unconditionally
    /// (whether or not a rebuild is needed). Tiling window managers
    /// commonly resize a window immediately after creating it (request
    /// size, then retile to the actual tile size) — committing to a
    /// rebuild as soon as one looks needed meant the very first layout
    /// session routinely started at a width that was already obsolete a
    /// tick later, throwing that work away. A rebuild only commits once
    /// the width has been the same for two ticks in a row.
    pub(crate) pending_width: Option<u32>,
    /// The OS cursor state last actually set, so `CursorMoved` only calls
    /// `window.set_cursor`/`set_cursor_visible` when the hit-tested style
    /// actually changes, not on every mouse-move tick. `Some(icon)` means
    /// visible showing that icon; `None` means hidden (`cursor: none`,
    /// the one `CursorStyle` with no `CursorIcon` of its own — see
    /// `CursorStyle::to_winit`'s own docs).
    pub(crate) last_cursor: Option<CursorIcon>,
    /// `overflow: scroll`/`auto`'s own live per-element scroll amounts
    /// (v62), keyed by `ScrollInfo::id` — the one piece of this
    /// feature's state that genuinely persists across frames and is
    /// deliberately *not* part of `Laid` (see `ScrollInfo`'s own docs).
    /// Cleared alongside `shape_cache` whenever the layout tree is
    /// rebuilt (`redraw`'s own `need_rebuild` branch) — stale ids from a
    /// since-replaced tree are harmless either way (just unused map
    /// entries), but clearing keeps this from growing unboundedly across
    /// many stdin chunks in streaming mode.
    pub(crate) scroll_offsets: HashMap<usize, f32>,
    /// QJ6: the same offsets keyed by DOM node (survive relayouts, which
    /// renumber `ScrollInfo::id`); script's `scrollTop` reads these.
    pub(crate) node_scroll: HashMap<usize, f32>,
    /// QJ6: the wheel changed `node_scroll`; tell the script engine next time.
    pub(crate) node_scroll_dirty: bool,
    /// The last `CursorMoved` position, window/screen space — `winit`'s
    /// own `MouseWheel` event carries no position at all, so routing a
    /// wheel tick to the right scroll region (`hit_test_scroll_region`)
    /// needs this remembered from whatever `CursorMoved` last reported.
    /// `None` until the first `CursorMoved` ever arrives (e.g. a wheel
    /// tick before the mouse has moved over the window at all) — falls
    /// back to the page's own top-level scroll in that case, same as
    /// "cursor not over any scroll region."
    pub(crate) last_mouse_pos: Option<(f32, f32)>,
    /// Document-space position at the most recent `MouseInput`
    /// `Pressed(Left)`, if any — compared against `last_mouse_pos` at
    /// the matching `Released(Left)` to tell a real click (navigates)
    /// apart from a drag (scratchy has no text selection yet, but a
    /// careless same-event binding would still misfire once it exists —
    /// see `is_click`'s own docs). Cleared back to `None` on release
    /// regardless of outcome.
    pub(crate) mouse_down_pos: Option<(f32, f32)>,
    /// Q31: live `:hover`/`:active`/`:focus*` state (also mirrored into the
    /// `INTERACTION` thread-local style matching reads).
    pub(crate) interaction: InteractionState,
    /// `STATE_*` bits of state pseudo-classes the current stylesheets use,
    /// captured after each rebuild. A state change restyles only if its bit
    /// is set here.
    pub(crate) state_rules: u8,
    /// `node_id` last seen under the mouse (0 = none).
    pub(crate) hover_node: usize,
    /// Q31b: a `#fragment` waiting for the new page's first layout that
    /// contains its target (set by `fetch_url`, consumed in `ShapeResult`).
    pub(crate) pending_fragment: Option<String>,
    /// QA6: fragment of the navigation in flight; becomes `pending_fragment`
    /// when its page arrives (not before: "Loading…" lays out first).
    pub(crate) nav_fragment: Option<String>,
    /// QY19: meta-refresh redirects followed for the current user navigation.
    pub(crate) meta_chain: u8,
    /// The next rebuild is a pure restyle (state change): keep the live
    /// per-element scroll offsets instead of clearing them.
    pub(crate) keep_scroll_on_rebuild: bool,
    /// The next rebuild is a restyle of the *same* document (interaction
    /// state or a height-only resize), not new content (R2.1).
    pub(crate) restyle_requested: bool,
    /// While `true`, partial `ShapeResult`s are ignored and the previous
    /// complete frame stays on screen until the new generation reports
    /// `incomplete == false` (R2.1). Cleared by `replace_document`.
    pub(crate) hold_old_frame: bool,
    /// QB2: JS wrote the DOM while a restyle was in flight; rebuild once
    /// that restyle's complete `ShapeResult` lands.
    pub(crate) js_restyle_pending: bool,
    /// QB2: a shaping pass is queued or running and its complete result
    /// has not landed. JS writes in this window are coalesced into
    /// `js_restyle_pending` instead of making the pass stale (an early
    /// layout of a big page takes longer than a 300 ms timer period).
    pub(crate) layout_in_flight: bool,
    /// Did the current layout read the viewport height (R4)?
    pub(crate) reads_viewport_h: bool,
    /// Which elements a `:hover` rule can restyle (R2.4).
    pub(crate) hover_keys: HoverKeys,
    /// Hover restyle throttle (R2.3).
    pub(crate) last_hover_restyle: Option<std::time::Instant>,
    pub(crate) last_wheel: Option<std::time::Instant>,
    /// QB3: when the pointer last moved, and how long the last
    /// `build_layout_tree_with_resources` took (the hover idle gate).
    pub(crate) last_pointer_move: Option<std::time::Instant>,
    pub(crate) last_build_time: Option<std::time::Duration>,
    /// QAN1c: running CSS animations and their pending events.
    pub(crate) anim: AnimTracker,
    pub(crate) hover_flush_scheduled: bool,
    /// Q35b2: when the first not-yet-applied image arrival of the current
    /// burst landed. An image changes layout, but a burst of them is
    /// collapsed into one rebuild (`image_relayout_wait`).
    pub(crate) image_relayout_since: Option<std::time::Instant>,
    pub(crate) image_flush_scheduled: bool,
    /// Lets a background navigation thread (`start_navigation`) get its
    /// result back onto this (the event-loop) thread — the exact same
    /// `thread::spawn` + `event_loop.create_proxy().send_event(..)`
    /// pattern `run()`'s own `Input::Stdin` case already uses for
    /// reading stdin off the main thread, reused here for the socket
    /// I/O a real page's fetch (through `wire`, formerly `snag --serve`)
    /// can take 10-30+ seconds to finish (confirmed directly against
    /// real Wikipedia in an earlier session). Created once in `run()`
    /// (an `ActiveEventLoop`, unlike
    /// the top-level `EventLoop`, has no `create_proxy` of its own —
    /// confirmed by reading winit's vendored source — so this has to be
    /// threaded in at construction time instead of fetched lazily from
    /// `resumed`'s own `&ActiveEventLoop`).
    pub(crate) proxy: EventLoopProxy<AppEvent>,
    /// The persistent background shaping worker's inbox — see `spawn_
    /// shaping_worker`'s own docs. `redraw`'s rebuild branch sends a
    /// fresh `ShapingRequest` here (`queue_shaping_request`) instead of
    /// spawning a new thread (and paying for a new `FontSystem`) every
    /// time a page needs reshaping.
    pub(crate) shaping_requests: mpsc::Sender<ShapingRequest>,
    /// The current page's own `<base href>` (recomputed from the real
    /// document every time it's replaced — initial load included, not
    /// just navigation — see `replace_document`), used to resolve a
    /// clicked `<a href>` into an absolute URL (`resolve_href`). `None`
    /// means a relative `href` can't be resolved at all (no base to
    /// join against) — reported as an inline error rather than guessed
    /// at; an already-absolute `href` still resolves fine either way,
    /// same as a real browser.
    pub(crate) base_url: Option<Url>,
    /// QF14b: the page that initiated the navigation about to start (a link
    /// click, a script, a meta refresh); `None` = browser-initiated (`:go`,
    /// startup, `:refresh`, history), which the jar treats as same-site.
    /// Consumed (`take`) by `fetch_url_inner`.
    pub(crate) nav_initiator: Option<Url>,
    /// Q30e: the body of the form POST the next `fetch_url_inner` sends
    /// (taken there; back/forward/refresh never have one: they re-GET).
    pub(crate) pending_post: Option<PostBody>,
    /// Every URL navigated to via a real click or `:go`, oldest
    /// first — `:historyBack`/`:historyFront` (bound to `h`/`l` in vim
    /// Normal mode) move `history_index` through this instead of
    /// fetching anything new. Session-only by design: nothing here ever
    /// touches disk, so it's gone the moment the process exits, same as
    /// a plain `Vec` naturally is — there's no persistence layer to add,
    /// only one to avoid adding. Navigating to a new page while sitting
    /// behind the front of history drops everything ahead of the
    /// current entry first (`push_history`), the same "going somewhere
    /// new forks off the back half" rule a real browser's history
    /// follows. The session's own opening page (loaded from a file,
    /// stdin, etc. — never itself pushed, since nothing navigated *to*
    /// it) gets backfilled as entry 0 on the very first real navigation
    /// away from it, so `:historyBack` has somewhere to go on the very
    /// first press — see `push_history`'s own docs.
    pub(crate) history: Vec<Url>,
    /// Index into `history` for the page currently on screen. Only
    /// meaningful once `history` is non-empty (true as soon as any
    /// navigation has happened); `push_history` and `navigate_history`
    /// are the only two places that ever change it.
    pub(crate) history_index: usize,
    /// QY48: per `history` entry, the `doc_gen` of the document it belongs to
    /// (`NO_DOC` = not loaded yet). Entries that share the current document's
    /// id are same-document (pushState / fragment): back/forward to them
    /// fires `popstate` in the page instead of reloading. May be shorter than
    /// `history` (missing = `NO_DOC`).
    pub(crate) history_doc: Vec<u64>,
    /// QY48: the `nav_generation` that loaded the document on screen.
    pub(crate) doc_gen: u64,
    /// Incremented every time `start_navigation` fires — tags the
    /// `AppEvent::NavResult` its spawned thread will eventually send
    /// back, so a result from a navigation the user has since moved on
    /// from (clicked a second link before the first one's fetch
    /// finished) is recognized as stale and dropped instead of
    /// clobbering newer content. See `AppEvent::NavResult`'s own docs.
    pub(crate) nav_generation: u64,
    /// Bumped every time this *page*'s own layout would need reshaping
    /// from scratch — every site that sets `layout_dirty = true` today
    /// (a new/changed `document`: `replace_document`, `AppEvent::Chunk`/
    /// `Eof`, `AppEvent::ImageFetched`), plus the one place `cached_
    /// width` itself changes (`redraw`'s rebuild commit, the purely-
    /// resize-driven case that never touches `layout_dirty` at all).
    /// Unlike `nav_generation` — which answers "which navigated-to
    /// *page*" — this answers "which *reshape* of the current page";
    /// see TODO.txt's "Move page shaping off the main thread" plan for
    /// why that's a different question, not yet reusable as-is. Not
    /// read anywhere yet — this field only exists so a future
    /// background-shaping thread has something correct to tag its
    /// spawn with and check on arrival; landed on its own, ahead of
    /// that thread, so the bookkeeping itself is proven not to regress
    /// anything before anything depends on it.
    pub(crate) layout_generation: u64,
    /// Q33t: the current page's live JS engine (its timers). Replaced
    /// whenever a page's scripts run and cleared on navigation, so a
    /// stale page's timers never fire; driven from `about_to_wait`.
    pub(crate) js: Option<JsEngine>,
    /// The `--socket <path>` CLI flag, if given — where a `wire` daemon
    /// is expected to be listening. `None` means the user never opted
    /// in at all; `start_navigation` treats that identically to "gave a
    /// path but nothing's listening there" (see its own docs), just
    /// without even attempting a connection first.
    pub(crate) socket_path: Option<PathBuf>,
    /// QF7: the page's network request queue (caps from `RequestPolicy`).
    pub(crate) net: NetScheduler,
    /// QF7: results that arrived while `:help` had the engine parked.
    pub(crate) parked_net: Vec<(u64, Result<WireResponse, NetError>)>,
    /// CSS fetched via `wire` from the current page's own `<link
    /// rel=stylesheet href>` references (v67) — concatenated once, by
    /// `start_navigation`'s own background thread (`process_external_
    /// resources`), *before* `AppEvent::NavResult` is even sent, so it's
    /// fully present for the very first `build_layout_tree_with_
    /// resources` call rather than arriving after an initial unstyled
    /// flash. Set by `replace_document`, same lifetime as `document`
    /// itself — cleared (to empty) on every navigation/`show_message`,
    /// never merged across pages.
    pub(crate) extra_css: String,
    /// QSD4: `document.adoptedStyleSheets` CSS, appended after `extra_css`.
    pub(crate) adopted_css: String,
    /// Already-decoded images fetched via `wire` for the current page's
    /// own non-`data:` `<img src>` references (v67), keyed by the exact,
    /// unresolved `src` attribute text — see `resolve_img`'s own docs
    /// for why that raw-string key is safe here. Unlike `extra_css`,
    /// populated gradually, *after* the page is already showing: each
    /// `AppEvent::ImageFetched` that arrives inserts one entry and marks
    /// layout dirty, so images materialize one at a time as their own
    /// fetches finish rather than blocking the page on all of them.
    /// Cleared on every navigation/`show_message`, same as `extra_css`.
    pub(crate) image_cache: HashMap<String, Arc<DecodedImage>>,
    /// Q35a: external CSS image URLs already handed to a fetch thread for
    /// this page (success or failure), so a rebuild never refetches. Cleared
    /// with `image_cache`.
    pub(crate) css_images_requested: HashSet<String>,
    /// QY56: images requested after load (script-set `src`, created or
    /// inserted `<img>`) this page, against `MAX_LATE_IMAGE_FETCHES`.
    pub(crate) late_images_requested: usize,
    /// QY56: late images refused by the cap, to settle as `error` in `js_sync`.
    pub(crate) late_image_errors: Vec<String>,
    /// Whether vim-mode key interception is on at all — `:set vim`/
    /// `:set novim`'s own stored state (plus `AppEvent::ToggleVimMode`'s
    /// own flip). `false` means `resolve_binding` is never even reached
    /// from `handle_key_event` for an ordinary keystroke: the explicit,
    /// stated use case (TODO.txt) is turning this fully off to play a
    /// browser-rendered game that wants raw keys, so "off" has to mean
    /// zero claimed keys, not "off except for the keys scratchy still
    /// wants." Starts `true` — real bare-key Normal-mode bindings work
    /// immediately, no setup step required, same as real vim's own
    /// default.
    pub(crate) vim_enabled: bool,
    /// `Normal` vs. the `:`-entered command line itself — see `VimMode`'s
    /// own docs. Only ever meaningfully `Command` while `vim_enabled` is
    /// `true` (there is no key left to reach `Action::EnterCommandMode`
    /// through otherwise), but not reset to `Normal` on `vim_enabled`
    /// flipping false — `dispatch_command`'s own `:set novim` handler
    /// already runs from inside a `Command`-mode keystroke and resets
    /// this itself right after, same as every other command-line
    /// submission does.
    pub(crate) vim_mode: VimMode,
    /// A bare `g` was just pressed in Normal mode; the next `g` completes `gg`.
    pub(crate) pending_g: bool,
    /// The `:`-command line's own live text, editable only while `vim_
    /// mode` is `Command` — see `TextEditBuffer`'s own docs for why this
    /// is a standalone type rather than a plain `String` field.
    pub(crate) command_line: TextEditBuffer,
    /// Live modifier-key state, kept current by `WindowEvent::
    /// ModifiersChanged` — `winit`'s own `KeyEvent` carries no
    /// modifiers of its own (`logical_key` is already shift-adjusted,
    /// but `Ctrl` specifically is excluded from that — see `KeyEvent::
    /// logical_key`'s own docs), so this is the only way `resolve_
    /// binding` can tell a bare `d`/`u` apart from `Ctrl`+`d`/`u`.
    pub(crate) modifiers: ModifiersState,
    /// Live `Tab`-cycling state for the command bar — see `Command
    /// Completion`'s own docs. `None` outside of an in-progress
    /// completion, which is most of the time (every non-`Tab` command-
    /// line keystroke resets it, including `Enter`/`Escape` leaving
    /// command mode entirely).
    pub(crate) completion: Option<CommandCompletion>,
    /// Set by `dispatch_command`'s own `:q`/`:quit` handler — only when
    /// `pre_help_page` is `None`, i.e. there's no page to fall back to
    /// (see that field's own docs for the vim-"closing the last buffer
    /// quits" half of this). Checked right after `handle_key_event`
    /// returns from `WindowEvent::KeyboardInput` (`window_event`'s own
    /// match arm) to actually call `ActiveEventLoop::exit()` —
    /// `dispatch_command` has no `&ActiveEventLoop` of its own to call
    /// that with directly (same reason `AppEvent::ToggleVimMode`'s own
    /// SIGUSR1 bootstrap docs give for why `resumed`'s `&ActiveEventLoop`
    /// can't just be stashed on `App` once and reused later — it's
    /// instead synthesized fresh at the one real call site each time,
    /// `window_event`'s own parameter, which this flag just has to
    /// survive one event handler call to reach). Checked, never read
    /// back down to `false` anywhere: once set, the event loop really is
    /// exiting, so there's no later point where un-setting it would mean
    /// anything.
    pub(crate) should_quit: bool,
    /// Q30g: the open `<select>` dropdown (painted last, above the page).
    pub(crate) select_popup: Option<SelectPopup>,
    /// Q30g: the release that ends a press the popup consumed is not a click.
    pub(crate) swallow_release: bool,
    /// Q30g: type-ahead buffer of a focused closed select.
    pub(crate) select_ahead: TypeAhead,
    /// The page that was showing right before `:help` opened, if any —
    /// `App::show_help`'s own snapshot, popped by `:q`/`:quit`
    /// (`App::restore_saved_page`) instead of quitting, the same way
    /// real vim's `:q` closes the current buffer/window and falls back
    /// to whatever was open before it, only actually exiting once
    /// there's nothing left to fall back to. `None` means either
    /// there's no overlay open at all (the common case), or `:help`
    /// itself was already open when this was captured (`show_help` only
    /// snapshots once — re-running `:help` while already viewing it
    /// must not overwrite the *real* saved page with the help page
    /// itself). Cleared — without restoring anything — by a real
    /// navigation (`start_navigation`'s own first line): clicking a
    /// link or running `:go` while `:help` is open means the *current*
    /// page is now genuinely new, not just an overlay on top of the old
    /// one, so a later `:q` has nothing sensible left to pop back to and
    /// should quit instead, not resurrect a page from before `:help`
    /// was ever opened.
    pub(crate) pre_help_page: Option<SavedPage>,
    /// QM1g/QM1g1: `<video>` boxes of the last full redraw and the kept page pixels.
    pub(crate) video: AppVideo,
}

/// QM1g1: what the video-rect-only redraw keeps between full redraws.
#[derive(Default)]
pub(crate) struct AppVideo {
    pub(crate) boxes: Vec<crate::media_video::VideoBoxInfo>,
    /// The last presented page (video included); only kept while a video box is visible.
    pub(crate) page_buf: Vec<u32>,
    pub(crate) size: (u32, u32),
    /// QM1g2d: per controls video, when its bar was last shown by the pointer/keys and
    /// what the bar last drew.
    pub(crate) controls: std::collections::HashMap<usize, CtlRt>,
    /// Does node N carry `controls`? Cleared when the layout is rebuilt.
    pub(crate) ctl_attr: std::collections::HashMap<usize, bool>,
    /// A press-and-drag on the seek or volume track.
    pub(crate) drag: Option<CtlDrag>,
}

#[derive(Default)]
pub(crate) struct CtlRt {
    pub(crate) shown_until: Option<std::time::Instant>,
    pub(crate) drawn: Option<crate::media_controls::DrawKey>,
}

#[derive(Clone, Copy)]
pub(crate) struct CtlDrag {
    pub(crate) node: usize,
    pub(crate) volume: bool,
    pub(crate) last: std::time::Instant,
}

/// State plus "is the bar shown" for a controls element.
pub(crate) type CtlView = (crate::media_controls::ControlsState, bool);

impl AppVideo {
    fn has_controls(&mut self, doc: &Handle, node: usize) -> bool {
        *self.ctl_attr.entry(node).or_insert_with(|| find_node_by_id(doc, node).is_some_and(|h| is_media_controls(&h)))
    }

    /// The controls elements among `nodes` with their state and bar visibility
    /// (always shown while paused; while playing for 2 s after the pointer/keys).
    pub(crate) fn states(&mut self, doc: &Handle, js: Option<&JsEngine>, nodes: &[usize]) -> std::collections::HashMap<usize, CtlView> {
        let with: Vec<usize> = nodes.iter().copied().filter(|n| self.has_controls(doc, *n)).collect();
        let Some(js) = js else { return Default::default() };
        if with.is_empty() {
            return Default::default();
        }
        let now = std::time::Instant::now();
        js.media_controls_state(&with)
            .into_iter()
            .map(|(n, s)| {
                let shown = self.controls.get(&n).and_then(|r| r.shown_until).is_some_and(|t| t > now);
                let vis = s.paused || shown;
                (n, (s, vis))
            })
            .collect()
    }

    /// The topmost controls box whose visible, unobstructed pixel is `pos`.
    fn box_at(&self, pos: (f32, f32), need_mask: bool) -> Option<usize> {
        let (x, y) = (pos.0.floor() as i32, pos.1.floor() as i32);
        self.boxes.iter().rev().find_map(|b| {
            if b.layered || b.mask.is_empty() || x < b.vis.0 || y < b.vis.1 || x >= b.vis.0 + b.vis.2 as i32 || y >= b.vis.1 + b.vis.3 as i32 {
                return None;
            }
            let mi = (y - b.vis.1) as usize * b.vis.2 as usize + (x - b.vis.0) as usize;
            (!need_mask || b.mask[mi]).then_some(b.node)
        })
    }
}

/// Installs the thread-safe "video frame ready" poster for this event loop.
fn install_video_wake(proxy: &EventLoopProxy<AppEvent>) {
    let p = Mutex::new(proxy.clone());
    crate::media_video::set_wake(Some(Box::new(move || {
        let _ = p.lock().unwrap().send_event(AppEvent::VideoFrame);
    })));
    // QF19w2: the WebSocket threads wake the loop the same way.
    let p = Mutex::new(proxy.clone());
    crate::js_ws::set_ws_wake(Some(Box::new(move || {
        let _ = p.lock().unwrap().send_event(AppEvent::JsWs);
    })));
}

/// `App::pre_help_page`'s own snapshot — every piece of per-page state
/// `App::replace_document` either changes or clears, captured so
/// `App::restore_saved_page` can put it back exactly, scroll position
/// included (unlike an ordinary navigation, which always starts a new
/// page at the top — see `replace_document`'s own docs for why that's
/// right there and wrong here: closing `:help` is not supposed to look
/// like navigating to a new page at all). `base_url` deliberately isn't
/// part of this: `show_help`'s own `replace_document` call never
/// touches `App::base_url`, so it's already untouched and needs no
/// restoring.
pub(crate) struct SavedPage {
    pub(crate) document: Handle,
    pub(crate) extra_css: String,
    /// QSD4: `document.adoptedStyleSheets` CSS, appended after `extra_css`.
    pub(crate) adopted_css: String,
    pub(crate) image_cache: HashMap<String, Arc<DecodedImage>>,
    pub(crate) scroll_offset: f32,
    pub(crate) scroll_offsets: HashMap<usize, f32>,
    /// QB6: the page's JS engine, suspended while help is shown (its
    /// timers, listeners and dirty flag must not touch the help page).
    pub(crate) js: Option<JsEngine>,
}

impl App {
    pub(crate) fn new_static(document: Handle, proxy: EventLoopProxy<AppEvent>, socket_path: Option<PathBuf>) -> Self {
        install_video_wake(&proxy);
        crate::js_net::set_net_executor(match &socket_path {
            Some(p) => Arc::new(WireExecutor(p.clone())),
            None => Arc::new(crate::js_net::NoSocketExecutor),
        });
        let base_url = find_base_url(&document);
        let shaping_requests = Self::spawn_shaping_worker(proxy.clone());
        Self {
            document,
            decoder: None,
            window: None,
            surface: None,
            font_system: new_font_system(),
            swash_cache: SwashCache::new(),
            scroll_offset: 0.0,
            cached: None,
            cached_width: 0,
            cached_height: 0,
            layout_dirty: true,
            shape_cache: ShapeCache::new(),
            surface_size: (0, 0),
            pending_width: None,
            last_cursor: Some(CursorIcon::Default),
            scroll_offsets: HashMap::new(),
            node_scroll: HashMap::new(),
            node_scroll_dirty: false,
            last_mouse_pos: None,
            mouse_down_pos: None,
            interaction: InteractionState::default(),
            state_rules: 0,
            hover_node: 0,
            pending_fragment: None,
            nav_fragment: None,
            meta_chain: 0,
            keep_scroll_on_rebuild: false,
            restyle_requested: false,
            hold_old_frame: false,
            js_restyle_pending: false,
            layout_in_flight: false,
            reads_viewport_h: true,
            hover_keys: HoverKeys { any: true, ..Default::default() },
            last_hover_restyle: None,
            last_wheel: None,
            last_pointer_move: None,
            last_build_time: None,
            anim: AnimTracker::default(),
            hover_flush_scheduled: false,
            image_relayout_since: None,
            image_flush_scheduled: false,
            proxy,
            shaping_requests,
            base_url,
            nav_initiator: None,
            pending_post: None,
            history: Vec::new(),
            history_index: 0,
            history_doc: Vec::new(),
            doc_gen: 0,
            nav_generation: 0,
            layout_generation: 0,
            js: None,
            socket_path,
            net: NetScheduler::new(&RequestPolicy::default()),
            parked_net: Vec::new(),
            extra_css: String::new(),
            adopted_css: String::new(),
            image_cache: HashMap::new(),
            css_images_requested: HashSet::new(),
            late_images_requested: 0,
            late_image_errors: Vec::new(),
            vim_enabled: true,
            vim_mode: VimMode::Normal,
            pending_g: false,
            command_line: TextEditBuffer::default(),
            modifiers: ModifiersState::empty(),
            completion: None,
            should_quit: false,
            select_popup: None,
            swallow_release: false,
            select_ahead: TypeAhead::default(),
            pre_help_page: None,
            video: AppVideo::default(),
        }
    }

    pub(crate) fn new_streaming(proxy: EventLoopProxy<AppEvent>, socket_path: Option<PathBuf>) -> Self {
        install_video_wake(&proxy);
        crate::js_net::set_net_executor(match &socket_path {
            Some(p) => Arc::new(WireExecutor(p.clone())),
            None => Arc::new(crate::js_net::NoSocketExecutor),
        });
        let decoder = new_parser();
        let document = decoder.inner_sink.tokenizer.sink.sink.document.clone();
        let shaping_requests = Self::spawn_shaping_worker(proxy.clone());
        Self {
            document,
            decoder: Some(decoder),
            window: None,
            surface: None,
            font_system: new_font_system(),
            swash_cache: SwashCache::new(),
            scroll_offset: 0.0,
            cached: None,
            cached_width: 0,
            cached_height: 0,
            layout_dirty: true,
            shape_cache: ShapeCache::new(),
            surface_size: (0, 0),
            pending_width: None,
            last_cursor: Some(CursorIcon::Default),
            scroll_offsets: HashMap::new(),
            node_scroll: HashMap::new(),
            node_scroll_dirty: false,
            last_mouse_pos: None,
            mouse_down_pos: None,
            interaction: InteractionState::default(),
            state_rules: 0,
            hover_node: 0,
            pending_fragment: None,
            nav_fragment: None,
            meta_chain: 0,
            keep_scroll_on_rebuild: false,
            restyle_requested: false,
            hold_old_frame: false,
            js_restyle_pending: false,
            layout_in_flight: false,
            reads_viewport_h: true,
            hover_keys: HoverKeys { any: true, ..Default::default() },
            last_hover_restyle: None,
            last_wheel: None,
            last_pointer_move: None,
            last_build_time: None,
            anim: AnimTracker::default(),
            hover_flush_scheduled: false,
            image_relayout_since: None,
            image_flush_scheduled: false,
            proxy,
            shaping_requests,
            base_url: None,
            nav_initiator: None,
            pending_post: None,
            history: Vec::new(),
            history_index: 0,
            history_doc: Vec::new(),
            doc_gen: 0,
            nav_generation: 0,
            layout_generation: 0,
            js: None,
            socket_path,
            net: NetScheduler::new(&RequestPolicy::default()),
            parked_net: Vec::new(),
            extra_css: String::new(),
            adopted_css: String::new(),
            image_cache: HashMap::new(),
            css_images_requested: HashSet::new(),
            late_images_requested: 0,
            late_image_errors: Vec::new(),
            vim_enabled: true,
            vim_mode: VimMode::Normal,
            pending_g: false,
            command_line: TextEditBuffer::default(),
            modifiers: ModifiersState::empty(),
            completion: None,
            should_quit: false,
            select_popup: None,
            swallow_release: false,
            select_ahead: TypeAhead::default(),
            pre_help_page: None,
            video: AppVideo::default(),
        }
    }

    pub(crate) fn redraw(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        if self.surface_size != (width, height) {
            let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
                return;
            };
            if let Err(e) = surface.resize(w, h) {
                log_redraw_error_once("resize the surface", &e);
                return;
            }
            self.surface_size = (width, height);
        }

        // Track the width seen on consecutive ticks unconditionally — not
        // just while a rebuild is pending — so even the very first rebuild
        // (layout_dirty from construction) waits for it to settle. The
        // first redraw after window creation routinely still reports the
        // requested size, not the tiling window manager's actual tile size
        // a tick later; building at the wrong one and then immediately
        // discarding it when the real size arrives was wasting real time
        // (measured: ~600ms of duplicate shaping on a small page).
        let video_on = crate::media_video::boxes_enabled();
        if video_on {
            // QM1g: layout reads the videos' sizes, paint their newest frames.
            let (frames, sizes) = self.js.as_ref().map(|j| j.media_video_state()).unwrap_or_default();
            crate::media_video::set_frames(frames);
            crate::media_video::set_intrinsic(sizes);
        }
        let width_matches_pending = self.pending_width == Some(width);
        self.pending_width = Some(width);
        // R4: a height-only change only needs a rebuild if the layout
        // actually read the viewport height; otherwise just track it.
        if self.cached_width == width && self.cached_height != height && self.cached.is_some() && !self.reads_viewport_h {
            self.cached_height = height;
        }
        let height_rebuild = self.cached_width == width && self.cached_height != height && self.cached.is_some();
        let need_rebuild = self.layout_dirty || self.cached_width != width || self.cached_height != height;

        if need_rebuild && width_matches_pending {
            // The DOM walk + cascade match itself stays synchronous,
            // right here, on the main thread — `Handle` (`self.document`)
            // is `Rc`-based, so it can never cross a thread boundary (see
            // `spawn_shaping_worker`'s own docs). Only the `LayoutBox`
            // tree this produces — plain owned data, no `Handle` left
            // inside it — gets handed off below.
            set_app_clock(self.nav_generation);
            let build_started = std::time::Instant::now();
            let tree = build_layout_tree_with_resources(&self.document, width as f32, Some(height as f32), &crate::images::with_adopted_css(&self.extra_css, &self.adopted_css), &self.image_cache);
            self.last_build_time = Some(build_started.elapsed());
            self.anim.after_build(self.nav_generation, take_animation_report(), document_clock_ms());
            fetch_wanted_css_images(&self.proxy, self.socket_path.as_deref(), self.base_url.as_ref(), &mut self.css_images_requested, self.nav_generation);
            let late = crate::images::take_wanted_img_srcs();
            fetch_late_images(&self.proxy, self.socket_path.as_deref(), self.base_url.as_ref(), &mut self.late_images_requested, &mut self.late_image_errors, self.nav_generation, late);
            // Ids the worker is about to assign restart at `0` for this
            // new tree (see `spawn_shaping_worker`'s own docs) — stale
            // entries keyed by an old tree's ids would just be harmless
            // dead weight otherwise, but clearing keeps this from
            // growing unboundedly across many stdin chunks in
            // streaming mode.
            if !self.keep_scroll_on_rebuild {
                self.scroll_offsets.clear();
            }
            self.keep_scroll_on_rebuild = false;
            self.video.ctl_attr.clear();
            self.state_rules = state_rules_mask();
            self.reads_viewport_h = viewport_height_read();
            self.hover_keys = HOVER_KEYS.with(|k| k.borrow().clone());
            // R2.1: same document, new style/height -> keep painting the
            // previous complete frame until the new one is fully shaped.
            let restyle = (self.restyle_requested || height_rebuild || self.hold_old_frame) && self.cached.is_some();
            self.restyle_requested = false;
            self.hold_old_frame = restyle;
            let reuse = restyle && self.cached_width == width;
            self.cached_width = width;
            self.cached_height = height;
            self.layout_dirty = false;
            self.layout_in_flight = true;
            self.layout_generation += 1;
            Self::queue_shaping_request(&self.shaping_requests, tree, width as f32, height as f32, self.layout_generation, reuse);
        } else if need_rebuild {
            window.request_redraw();
        }

        let mut pixels = match surface.buffer_mut() {
            Ok(p) => p,
            Err(e) => {
                log_redraw_error_once("get the frame buffer", &e);
                return;
            }
        };
        for pixel in pixels.iter_mut() {
            *pixel = 0x00FFFFFF;
        }

        if video_on {
            crate::media_video::begin_paint(&pixels);
        }
        // `None` on the very first tick or two, while we're still
        // waiting to see whether the width is about to change again
        // (see `pending_width`); also `None` until the background
        // shaping worker's own *first* tick for this page lands
        // (`AppEvent::ShapeResult`) — normally one `PROGRESSIVE_LAYOUT_
        // TICK_BUDGET` or so, same order of magnitude as the old inline
        // call's own first partial result used to take. Either way,
        // paint a blank frame rather than stall on it.
        if let Some(laid) = self.cached.as_ref() {
            // QW6: the canvas colour covers the whole window, also past a
            // short document's end.
            if let Some(c) = laid.style.background_color {
                for pixel in pixels.iter_mut() {
                    *pixel = crate::paint::blend(*pixel, c);
                }
            }
            let max_scroll = (laid.h - height as f32).max(0.0);
            self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll);

            paint(
                laid,
                &mut self.font_system,
                &mut self.swash_cache,
                &mut self.shape_cache,
                &self.scroll_offsets,
                &mut pixels,
                width,
                height,
                self.scroll_offset,
                None,
            );
            // Q31b: default focus ring for keyboard focus, unless the author
            // already styled an outline on the element.
            if let (Some(f), true) = (self.interaction.focus, self.interaction.focus_visible) {
                let geom = node_geometry(laid, &self.shape_cache, f);
                if !geom.outline {
                    for &(x, y, w, h) in &geom.rects {
                        paint_focus_ring(&mut pixels, width, height, x, y - self.scroll_offset, w, h);
                    }
                }
            }
        }

        // Q30g: the open select dropdown is the top layer, above the page.
        if let (Some(p), Some(laid)) = (&self.select_popup, self.cached.as_ref())
            && let Some(view) = popup_view(&p.st, &self.document, laid, &self.shape_cache, self.scroll_offset, (width as f32, height as f32))
        {
            paint_select_popup(&view, &p.st, &mut self.font_system, &mut self.swash_cache, &mut pixels, width, height);
        }

        if self.vim_mode == VimMode::Command {
            paint_command_bar(&self.command_line, &mut self.font_system, &mut self.swash_cache, &mut pixels, width, height);
        }

        // QM1g: video pixels go in last, through the overlay mask of whatever
        // was painted above them; the finished page is kept for QM1g1.
        let painted = if video_on { crate::media_video::end_paint() } else { Vec::new() };
        let laid: Vec<usize> = painted.iter().map(|b| b.node).collect();
        let boxes = crate::media_video::finish(painted, &mut pixels, width);
        // QM1g2d: the controls bars go over the finished picture (same overlay mask).
        {
            let nodes: Vec<usize> = boxes.iter().map(|b| b.node).collect();
            let states = self.video.states(&self.document, self.js.as_ref(), &nodes);
            for b in &boxes {
                if let Some((st, vis)) = states.get(&b.node) {
                    crate::media_controls::paint(&mut pixels, width, b, st, *vis);
                    self.video.controls.entry(b.node).or_default().drawn = Some(st.key(*vis));
                }
            }
        }
        self.video.page_buf.clear();
        if !boxes.is_empty() {
            self.video.page_buf.extend_from_slice(&pixels);
            self.video.size = (width, height);
        }
        let targets: Vec<_> = boxes.iter().filter(|b| crate::media_video::wants_render(b.visible_pixels(), b.layered, false)).map(|b| (b.node, b.target())).collect();
        self.video.boxes = boxes;

        window.pre_present_notify();
        if let Err(e) = pixels.present() {
            log_redraw_error_once("present the frame", &e);
        }
        if video_on && let Some(js) = &self.js {
            js.media_set_targets(&targets);
            js.media_set_laid(&laid);
        }
    }

    /// QM1g1: an mpv frame is ready. Renders it and, when the page itself is
    /// unchanged, copies it into the video's visible rect only (through the
    /// overlay mask, so content above it stays) and presents just that damage.
    /// False = a full redraw is needed (the caller requests it).
    fn video_tick(&mut self) -> bool {
        let Some(js) = &self.js else { return true };
        let changed = js.media_render();
        if changed.is_empty() {
            return true;
        }
        let (frames, _) = js.media_video_state();
        let (w, h) = self.surface_size;
        let ok = !self.layout_dirty && self.video.size == (w, h) && self.video.page_buf.len() == w as usize * h as usize;
        crate::media_video::set_frames(frames.clone());
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else { return true };
        if !ok || window.inner_size().width != w || window.inner_size().height != h {
            return false;
        }
        let mut damage = Vec::new();
        let ctl = self.video.states(&self.document, self.js.as_ref(), &changed);
        for node in &changed {
            let (Some(info), Some(frame)) = (self.video.boxes.iter().find(|b| b.node == *node), frames.get(node)) else { continue };
            match crate::media_video::update_box(info, frame, &mut self.video.page_buf, w) {
                Some(r) => damage.push(r),
                None => return false,
            }
            if let Some((st, vis)) = ctl.get(node) {
                crate::media_controls::paint(&mut self.video.page_buf, w, info, st, *vis);
            }
        }
        for (n, (st, vis)) in &ctl {
            self.video.controls.entry(*n).or_default().drawn = Some(st.key(*vis));
        }
        if damage.is_empty() {
            return true;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return false };
        if buf.age() == 1 {
            for node in &changed {
                if let (Some(info), Some(frame)) = (self.video.boxes.iter().find(|b| b.node == *node), frames.get(node)) {
                    crate::media_video::update_box(info, frame, &mut buf, w);
                    if let Some((st, vis)) = ctl.get(node) {
                        crate::media_controls::paint(&mut buf, w, info, st, *vis);
                    }
                }
            }
        } else {
            buf.copy_from_slice(&self.video.page_buf);
        }
        let rects: Vec<softbuffer::Rect> = damage
            .iter()
            .filter_map(|r| Some(softbuffer::Rect { x: r.0 as u32, y: r.1 as u32, width: NonZeroU32::new(r.2)?, height: NonZeroU32::new(r.3)? }))
            .collect();
        // No `pre_present_notify` here: it arms winit's frame-callback gate, and a
        // damage present every ~33 ms would keep the next full redraw (a scroll)
        // waiting for a callback that the compositor delays.
        if let Err(e) = buf.present_with_damage(&rects) {
            log_redraw_error_once("present the video damage", &e);
        }
        true
    }

    /// QM1g2d: repaints the bars whose state or visibility changed since they were drawn
    /// (4 Hz while playing, show/hide, play/pause). Bar rect only: no page redraw. Returns
    /// when the next refresh is due.
    pub(crate) fn controls_refresh(&mut self) -> Option<std::time::Instant> {
        if self.video.boxes.is_empty() {
            return None;
        }
        let nodes: Vec<usize> = self.video.boxes.iter().map(|b| b.node).collect();
        let states = self.video.states(&self.document, self.js.as_ref(), &nodes);
        if states.is_empty() {
            return None;
        }
        let now = std::time::Instant::now();
        let mut stale = Vec::new();
        let mut deadline: Option<std::time::Instant> = None;
        for (n, (s, vis)) in &states {
            let rt = self.video.controls.entry(*n).or_default();
            let key = s.key(*vis);
            if rt.drawn.as_ref() != Some(&key) {
                stale.push((*n, s.big_button() || rt.drawn.as_ref().is_none_or(|d| d.big)));
            }
            if !s.paused {
                let mut cands = Vec::new();
                if let Some(t) = rt.shown_until.filter(|t| *t > now) {
                    cands.push(t);
                }
                if *vis {
                    cands.push(now + std::time::Duration::from_millis(crate::media_controls::REFRESH_MS));
                }
                for t in cands {
                    deadline = Some(deadline.map_or(t, |d| d.min(t)));
                }
            }
        }
        if !stale.is_empty()
            && !self.redraw_bars(&stale, &states)
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
        deadline
    }

    /// Repaints the bars of `stale` (node, whole box?) into the kept page and presents just
    /// those rects. False = a full redraw is needed instead.
    fn redraw_bars(&mut self, stale: &[(usize, bool)], states: &std::collections::HashMap<usize, CtlView>) -> bool {
        let (w, h) = self.surface_size;
        if self.layout_dirty || self.video.size != (w, h) || self.video.page_buf.len() != w as usize * h as usize {
            return false;
        }
        let mut damage = Vec::new();
        for &(n, full) in stale {
            let (Some(info), Some((st, vis))) = (self.video.boxes.iter().find(|b| b.node == n), states.get(&n)) else { continue };
            let sub = if full { info.vis } else { crate::media_controls::layout(info.rect, crate::media_controls::time_text(st).chars().count()).map_or(info.vis, |l| l.bar) };
            if let Some(r) = crate::media_video::recomposite(info, sub, &mut self.video.page_buf, w) {
                damage.push(r);
            }
            crate::media_controls::paint(&mut self.video.page_buf, w, info, st, *vis);
            self.video.controls.entry(n).or_default().drawn = Some(st.key(*vis));
        }
        let (Some(_), Some(surface)) = (&self.window, &mut self.surface) else { return true };
        let Ok(mut buf) = surface.buffer_mut() else { return false };
        if buf.len() != self.video.page_buf.len() {
            return false;
        }
        buf.copy_from_slice(&self.video.page_buf);
        let rects: Vec<softbuffer::Rect> = damage
            .iter()
            .filter_map(|r| Some(softbuffer::Rect { x: r.0 as u32, y: r.1 as u32, width: NonZeroU32::new(r.2)?, height: NonZeroU32::new(r.3)? }))
            .collect();
        if let Err(e) = buf.present_with_damage(&rects) {
            log_redraw_error_once("present the controls damage", &e);
        }
        true
    }

    fn controls_run(&mut self, node: usize, op: crate::media_controls::UiOp) {
        if let Some(js) = &self.js {
            js.media_ui(node, op);
        }
        self.js_sync();
        self.video.controls.entry(node).or_default().shown_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(crate::media_controls::HIDE_AFTER_MS));
        self.controls_refresh();
    }

    /// The bar element under `pos`: (node, its state, the hit).
    fn controls_hit_at(&mut self, pos: (f32, f32)) -> Option<(usize, crate::media_controls::ControlsState, crate::media_controls::Hit)> {
        let node = self.video.box_at(pos, true)?;
        let (st, vis) = self.video.states(&self.document, self.js.as_ref(), &[node]).remove(&node)?;
        if !vis {
            return None;
        }
        let rect = self.video.boxes.iter().find(|b| b.node == node)?.rect;
        let l = crate::media_controls::layout(rect, crate::media_controls::time_text(&st).chars().count())?;
        let hit = crate::media_controls::hit(&l, pos.0.floor() as i32, pos.1.floor() as i32)?;
        Some((node, st, hit))
    }

    /// A left press at `pos`: true when it landed on a visible controls bar (UA chrome: the
    /// page gets no mouse events for it).
    pub(crate) fn controls_press(&mut self, pos: (f32, f32)) -> bool {
        let Some((node, st, hit)) = self.controls_hit_at(pos) else { return false };
        self.press_state(pos);
        if let Some(op) = crate::media_controls::op_for_hit(hit, &st) {
            self.controls_run(node, op);
        }
        let dragging = match hit {
            crate::media_controls::Hit::Seek(_) => Some(false),
            crate::media_controls::Hit::Volume(_) => Some(true),
            _ => None,
        };
        self.video.drag = dragging.map(|volume| CtlDrag { node, volume, last: std::time::Instant::now() });
        true
    }

    /// Pointer motion while dragging a track (seeks at most every 80 ms; the release applies the last).
    pub(crate) fn controls_drag_move(&mut self, pos: (f32, f32), force: bool) {
        let Some(d) = self.video.drag else { return };
        if !force && d.last.elapsed() < std::time::Duration::from_millis(80) {
            return;
        }
        let Some((st, _)) = self.video.states(&self.document, self.js.as_ref(), &[d.node]).remove(&d.node) else { return };
        let Some(rect) = self.video.boxes.iter().find(|b| b.node == d.node).map(|b| b.rect) else { return };
        let Some(l) = crate::media_controls::layout(rect, crate::media_controls::time_text(&st).chars().count()) else { return };
        let x = pos.0.floor() as i32;
        let op = if d.volume {
            l.vol.map(|t| crate::media_controls::UiOp::Volume(crate::media_controls::track_frac(t, x)))
        } else {
            l.seek.filter(|_| st.duration.is_finite() && st.duration > 0.0).map(|t| crate::media_controls::UiOp::Seek(crate::media_controls::track_frac(t, x) * st.duration))
        };
        if let Some(op) = op {
            if let Some(dd) = &mut self.video.drag {
                dd.last = std::time::Instant::now();
            }
            self.controls_run(d.node, op);
        }
    }

    /// The pointer is over a controls video: show its bar for 2 s.
    pub(crate) fn controls_pointer(&mut self, pos: (f32, f32)) {
        let Some(node) = self.video.box_at(pos, false) else { return };
        if !self.video.has_controls(&self.document, node) {
            return;
        }
        let until = std::time::Instant::now() + std::time::Duration::from_millis(crate::media_controls::HIDE_AFTER_MS);
        self.video.controls.entry(node).or_default().shown_until = Some(until);
        self.controls_refresh();
    }

    /// D6: keys act on a FOCUSED controls video only (the page saw the keydown first).
    pub(crate) fn media_key(&mut self, key: &Key, repeat: bool) -> bool {
        let Some(f) = self.interaction.focus else { return false };
        if self.modifiers.control_key() || self.modifiers.alt_key() || !find_node_by_id(&self.document, f).is_some_and(|h| is_media_controls(&h)) {
            return false;
        }
        let Some(st) = self.js.as_ref().and_then(|j| j.media_controls_state(&[f]).remove(&f)) else { return false };
        let Some(op) = crate::media_controls::key_op(key, &st) else { return false };
        if !(repeat && op == crate::media_controls::UiOp::Toggle) {
            self.controls_run(f, op);
        }
        true
    }

    /// Queues a fresh `LayoutBox` tree for the persistent background
    /// shaping worker (`spawn_shaping_worker`, started once in `new_
    /// static`/`new_streaming`) instead of spawning a brand-new thread
    /// per rebuild — step 3 of the "move page shaping off the main
    /// thread" plan in TODO.txt, landed, then revised once real-world
    /// testing (a real Wikipedia page, through a merged PR) surfaced
    /// two problems the one-shot-thread-per-rebuild version had: (1)
    /// paying `new_font_system()`'s own system-font-database scan again
    /// on *every* navigation/resize, not once ever the way `self.font_
    /// system` already was; (2) running `measure_document` to
    /// completion in one call, with no progressive fill-in at all,
    /// which turned "laggy while loading" into "blank for most of a
    /// minute" on a big real page — worse than the problem this whole
    /// plan set out to fix, even though input itself stayed responsive
    /// throughout. A free function, not an `&self` method: `redraw`'s
    /// own caller already holds `self.window`/`self.surface` borrowed
    /// apart from the rest of `self` (see its own destructuring `let`),
    /// which an `&self` method call can't see past.
    pub(crate) fn queue_shaping_request(
        requests: &mpsc::Sender<ShapingRequest>, tree: LayoutBox, width: f32, height: f32, generation: u64,
        reuse_shaping: bool,
    ) {
        let _ = requests.send(ShapingRequest { tree, width, height, generation, reuse_shaping });
    }

    /// Starts the persistent background shaping worker — called once,
    /// from `new_static`/`new_streaming`, not per rebuild. Returns the
    /// `Sender` half; `redraw`'s own rebuild branch queues a `Shaping
    /// Request` through it (`queue_shaping_request`) instead of
    /// spawning a new thread each time.
    ///
    /// **Started eagerly at construction, not lazily on first use** —
    /// so its own one-time cost (building a `FontSystem`, scanning the
    /// system's font database — the same call `self.font_system` itself
    /// already pays once at startup) overlaps with whatever `App`
    /// construction/the very first document parse is doing anyway,
    /// instead of sitting serially in front of the very first page's
    /// own first shape, which is exactly when a user is already
    /// waiting on it.
    ///
    /// **A second, independently-owned `FontSystem`/`SwashCache` pair,
    /// not `self`'s own, but paid for only once, here, not per
    /// request.** Confirmed by reading cosmic-text's own vendored
    /// source: `FontSystem`/`Buffer`/`SwashCache` hold no `Rc`/`RefCell`
    /// internally, so sharing one *instance* with the main thread would
    /// at least compile (behind a `Mutex`, say) — but would just
    /// reintroduce the exact stall this plan exists to remove: the
    /// main thread blocking on the lock while this thread holds it
    /// mid-shape. So this thread keeps its own independent copy for its
    /// entire lifetime instead. `self.font_system`/`self.swash_cache`
    /// stay exactly what `paint`/`paint_command_bar` already use to
    /// paint a finished `Laid` tree on the main thread; nothing about
    /// painting itself changes.
    ///
    /// **Ticks internally, same `PROGRESSIVE_LAYOUT_TICK_BUDGET` this
    /// plan's own first version removed** — but keeps its own
    /// `ShapeCache` growing across ticks *locally*, the same continuity
    /// `App::redraw`'s old inline loop used to get for free by staying
    /// on one thread the whole time, now reproduced across a real
    /// thread boundary: every tick, the whole map — cheap to clone now
    /// that each entry is `Arc<Mutex<_>>` (see `ShapeCache`'s own docs)
    /// — goes out as one `AppEvent::ShapeResult`, so the page fills in
    /// progressively on the main thread exactly as it did before this
    /// plan started, while this thread keeps shaping the rest into the
    /// *same* cache underneath, never losing already-shaped work the
    /// way sending the cache away by value would.
    ///
    /// **Checks for a newer, superseding request between ticks, not
    /// just at the very start** — a real improvement over the no-
    /// cancellation tradeoff the single-spawn-per-rebuild version
    /// (and `start_navigation`'s own fetch thread) still accept:
    /// since this thread is already idle between ticks anyway, a
    /// cheap non-blocking check (`try_recv`) can switch straight to a
    /// resize/navigation/chunk that arrived mid-shape instead of
    /// finishing the now-stale request first, bounding the wasted work
    /// to at most one tick instead of an entire shape pass.
    pub(crate) fn spawn_shaping_worker(proxy: EventLoopProxy<AppEvent>) -> mpsc::Sender<ShapingRequest> {
        let (tx, rx) = mpsc::channel::<ShapingRequest>();
        // QSTK1: measure recurses once per nesting level (~14 KB a level); the 2 MB default stack
        // reached `crate::stack::grow`'s red zone ~100 levels deep, so give the worker room.
        let worker = thread::Builder::new().name("shaping".into()).stack_size(32 << 20);
        let spawned = worker.spawn(move || {
            let mut font_system = new_font_system();
            SHAPE_MEMO_ENABLED.with(|m| m.set(true));
            // `rx.recv()` blocks here whenever there's no page waiting
            // to be shaped — this thread is otherwise entirely idle,
            // not spinning. `next` carries a request straight into the
            // next iteration when the inner loop below was interrupted
            // by one arriving mid-shape (see its own comment), instead
            // of going back through `rx.recv()` and losing it.
            let mut next = rx.recv();
            while let Ok(mut request) = next {
                // Jump straight to the newest already-queued request,
                // if more than one piled up while this thread was busy
                // with something else (e.g. several resizes in a row) —
                // see this function's own docs.
                while let Ok(newer) = rx.try_recv() {
                    request = newer;
                }
                let ShapingRequest { tree, width, height, generation, reuse_shaping } = request;
                if !reuse_shaping {
                    SHAPE_MEMO.with(|m| m.borrow_mut().clear());
                }
                let mut shape_cache = ShapeCache::new();
                let mut superseded_by = None;
                loop {
                    let deadline = std::time::Instant::now() + PROGRESSIVE_LAYOUT_TICK_BUDGET;
                    let mut incomplete = false;
                    let mut next_id = 0usize;
                    // Isolated behind `catch_unwind` — this is the one
                    // persistent worker shaping *every* future page, for
                    // the life of the process (see this function's own
                    // docs); a panic on some one pathological page (a
                    // real bug, not an expected outcome — see TODO.txt
                    // for any specific one found this way) must not take
                    // the whole worker down with it, permanently
                    // breaking every navigation after it with no visible
                    // error at all (`App::queue_shaping_request`'s own
                    // `let _ = requests.send(..)` already discards a
                    // disconnected-channel error silently, by design,
                    // for the ordinary "event loop gone at shutdown"
                    // case — which is exactly why a dead worker must
                    // never happen from an ordinary bad page). Abandons
                    // *only* this one request on a panic (falls through
                    // to `rx.recv()` for the next one, `superseded_by`
                    // left `None`) — `font_system` keeps being reused
                    // across the panic rather than rebuilt, a calculated
                    // risk (its own internal cache could in principle be
                    // left mid-update) accepted because the alternative,
                    // tearing down and restarting this whole thread, is
                    // far more disruptive for what should be a rare
                    // event; `shape_cache` is this request's own local
                    // value regardless, already discarded either way.
                    let measured = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        measure_document(
                            &tree,
                            &mut font_system,
                            width,
                            // Same real, definite window dimension
                            // `redraw`'s own synchronous call used to
                            // pass here before this plan started — see
                            // `measure_document`'s own docs for why
                            // `viewport_h` specifically (only `position:
                            // fixed`/an orphaned `position: absolute`'s
                            // own containing block) is the one place
                            // that's true.
                            height,
                            &mut next_id,
                            &mut shape_cache,
                            deadline,
                            &mut incomplete,
                        )
                    }));
                    let laid = match measured {
                        Ok(laid) => laid,
                        Err(payload) => {
                            let message = payload
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| payload.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "non-string panic payload".to_string());
                            diag!(
                                "scratchy: shaping worker: measure_document panicked on generation {generation} ({message}) — abandoning this page, worker still running"
                            );
                            break;
                        }
                    };
                    if proxy.send_event(AppEvent::ShapeResult(generation, Box::new(laid), shape_cache.clone(), incomplete)).is_err() {
                        return; // the event loop (and App) is gone
                    }
                    if !incomplete {
                        break;
                    }
                    // Mid-shape staleness check — see this function's
                    // own docs. A fresh request replaces this one
                    // outright; its own first tick starts from a brand
                    // new, empty `shape_cache`, same as any other new
                    // generation's first tick.
                    if let Ok(newer) = rx.try_recv() {
                        superseded_by = Some(newer);
                        break;
                    }
                }
                next = match superseded_by {
                    Some(newer) => Ok(newer),
                    None => rx.recv(),
                };
            }
        });
        spawned.expect("shaping worker thread");
        tx
    }

    /// Swaps in a brand-new document — the one place both link
    /// navigation and the "Loading…"/error placeholders it shows while
    /// waiting (`show_message`) go through. Reuses `run()`'s own
    /// initial `App::new_static(dom.document)` path's *shape* (build a
    /// fresh DOM, hand it to the engine) but mid-session instead of only
    /// at startup — everything downstream (`redraw`'s own layout-dirty
    /// rebuild) already works unmodified, since from its point of view
    /// this looks exactly like the document simply having changed, the
    /// same thing `AppEvent::Chunk`/`Eof` already do for streaming mode.
    /// Does *not* touch `base_url` — callers that are loading a real new
    /// page (not just a transient status message) update that
    /// separately, since a synthetic "Loading…" page has no `<base>` of
    /// its own and clobbering `base_url` from it would make a page's
    /// own links briefly unresolvable while its replacement is loading.
    ///
    /// `extra_css` (v67) replaces `self.extra_css` wholesale — a page's
    /// own externally-fetched stylesheets never carry over to whatever
    /// replaces it, same "nothing leaks across a document swap" rule
    /// `scroll_offsets`/`shape_cache` already follow just below.
    /// `image_cache` is deliberately NOT cleared here (R7): the cache
    /// is cleared when a navigation *starts* (`fetch_url_inner`,
    /// `show_message`, `show_help`), because images for the new page
    /// are fetched while `process_external_resources` is still running
    /// scripts, so an `ImageFetched` for the current generation can
    /// land *before* `NavResult` does. Stale-generation fetches are
    /// dropped by `ImageFetched`'s own `nav_generation` check.
    pub(crate) fn replace_document(&mut self, document: Handle, extra_css: String) {
        self.document = document;
        self.interaction = InteractionState::default();
        set_interaction(&self.interaction);
        clear_check_state();
        clear_text_state();
        clear_select_state();
        self.select_popup = None;
        self.hover_node = 0;
        js_style::set_js_extra_css(&extra_css);
        js_style::set_js_adopted_css("");
        crate::images::clear_style_overrides();
        self.adopted_css.clear();
        self.extra_css = extra_css;
        self.layout_dirty = true;
        self.layout_generation += 1;
        self.cached = None;
        self.restyle_requested = false;
        self.hold_old_frame = false;
        self.js_restyle_pending = false;
        self.layout_in_flight = false;
        self.image_relayout_since = None;
        self.shape_cache.clear();
        self.scroll_offsets.clear();
        self.node_scroll.clear();
        self.node_scroll_dirty = false;
        self.scroll_offset = 0.0;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Replaces the page with a plain one-paragraph status message —
    /// used for both the "Loading…" indicator `start_navigation` shows
    /// immediately (a real page can take 10-30+ seconds to fetch through
    /// a real `wire` daemon, confirmed directly against Wikipedia in an
    /// earlier session, back when this fetched through `snag --serve`
    /// instead — freezing with no feedback would look broken) and a
    /// failed navigation's inline error (no `--socket` configured, the
    /// daemon unreachable, or a response in a content type scratchy
    /// can't render — see `fetch_raw_via_wire`'s own docs for which).
    /// `escape_html` is needed since `text` can echo back arbitrary
    /// attacker^Wpage-controlled content (a clicked `href`, or an error
    /// string) — without it, a stray `<`/`&` in either would corrupt the
    /// synthetic markup instead of just displaying literally. Never has
    /// any externally-fetched CSS of its own — a synthetic status page
    /// is never real content, so `replace_document` always clears
    /// `extra_css` for it.
    pub(crate) fn show_message(&mut self, text: &str) {
        let html = format!("<html><body><p>{}</p></body></html>", escape_html(text));
        self.image_cache.clear();
        self.css_images_requested.clear();
        // QB6: a status/error page replaces the document, so the old
        // page's engine must not keep running against it.
        self.js = None;
        self.replace_document(parse_html(&html), String::new());
    }

    pub(crate) fn handle_shape_result(&mut self, generation: u64, laid: Box<Laid>, shape_cache: ShapeCache, incomplete: bool) {
        // Same staleness guard `NavResult`/`ImageFetched` already
        // use, tagged with `layout_generation` instead of `nav_
        // generation` — a background shaping pass that only
        // finishes after a resize/chunk/navigation/image already
        // started a newer one must never clobber content that's
        // already further along. See `AppEvent::ShapeResult`'s
        // own docs.
        if generation != self.layout_generation {
            return;
        }
        // R2.1: a restyle keeps the old complete frame (and its
        // shape cache) on screen until the new one is complete.
        if self.hold_old_frame {
            if incomplete {
                return;
            }
            self.hold_old_frame = false;
        }
        if !incomplete {
            self.layout_in_flight = false;
            if self.js_restyle_pending {
                self.request_js_restyle();
            }
            // Q35b2: images that arrived during the pass rebuild now.
            self.flush_image_relayout();
        }
        // Replaced wholesale on every tick this generation's
        // worker reports, not merged — see `App::shape_cache`'s
        // own docs for why that's always safe (every entry here
        // is a strict superset of the last delivery's, same
        // generation, same underlying tree).
        self.shape_cache = shape_cache;
        self.cached = Some(*laid);
        self.restore_node_scroll();
        // Q31b: a fragment from a cross-page navigation waits for
        // the first layout that contains its target.
        // QA6: a target that does not exist yet (page still shaping) keeps
        // waiting; one that never appears is given up on when the layout
        // is complete. `apply_fragment` alone returns true for "missing".
        if let Some(frag) = self.pending_fragment.clone() {
            let applied = self.apply_fragment(&frag);
            // Re-applied on every incomplete tick (earlier content may still
            // grow and move the target) and dropped on the first complete one.
            if applied && (!incomplete || frag.is_empty() || frag.eq_ignore_ascii_case("top")) {
                self.pending_fragment = None;
            }
        }
    }

    /// QJ6: after a layout install, node-keyed element scroll offsets become
    /// the per-region offsets of the new tree (region ids change per layout).
    fn restore_node_scroll(&mut self) {
        if self.node_scroll.is_empty() {
            return;
        }
        let Some(l) = &self.cached else { return };
        let mut out = HashMap::new();
        crate::paint::collect_scroll_regions(l, &self.node_scroll, &mut out);
        self.scroll_offsets = out;
    }

    /// QJ6: the wheel-scrolled element offsets script has not seen yet.
    fn sync_node_scroll(&mut self) -> Option<Vec<(usize, f32)>> {
        if !std::mem::take(&mut self.node_scroll_dirty) {
            return None;
        }
        Some(self.node_scroll.iter().map(|(k, v)| (*k, *v)).collect())
    }

    /// Q33c: if page scripts wrote to the DOM, request one layout
    /// rebuild (never one per write) and a redraw.
    pub(crate) fn js_sync(&mut self) {
        for src in std::mem::take(&mut self.late_image_errors) {
            self.image_settled(&src, None);
        }
        // QB5: focus/blur and window.scroll* requests made by page script.
        if let Some(js) = &self.js {
            let mut reqs = js.take_requests();
            if let Some(css) = reqs.adopted_css.take() {
                self.adopted_css = css;
            }
            match reqs.focus {
                Some(crate::js_dom::FocusReq::Focus(h)) => {
                    self.set_focus_element(Some(&h), false);
                    self.scroll_focus_into_view();
                }
                Some(crate::js_dom::FocusReq::Blur(h)) => {
                    if self.interaction.focus == Some(node_id(&h)) {
                        self.set_focus_element(None, false);
                    }
                }
                None => {}
            }
            if reqs.check_changed {
                self.check_state_changed();
            }
            if reqs.text_changed
                && let Some(w) = &self.window
            {
                w.request_redraw();
            }
            for ev in std::mem::take(&mut reqs.url_events) {
                self.apply_url_event(ev);
            }
            if let Some(href) = reqs.nav {
                self.navigate_to(&href, true, reqs.nav_replace);
            } else if reqs.reload {
                self.refresh();
            }
            if let Some((form, submitter)) = reqs.submit {
                self.form_navigate(&form, submitter.as_ref());
            }
            // QF7: page network requests (enqueue before abort, so a request
            // made and aborted in one turn never starts).
            for job in reqs.net {
                self.net.enqueue(job);
            }
            for id in reqs.net_abort {
                self.net.abort(id);
            }
            for (id, result) in reqs.net_done {
                let _ = self.proxy.send_event(AppEvent::JsNet(self.nav_generation, id, result));
            }
            self.pump_net();
            let imgs = std::mem::take(&mut reqs.imgs);
            fetch_late_images(&self.proxy, self.socket_path.as_deref(), self.base_url.as_ref(), &mut self.late_images_requested, &mut self.late_image_errors, self.nav_generation, imgs);
            for (h, y) in reqs.el_scroll {
                let node = node_id(&h);
                let mut v = (y as f32).max(0.0);
                if let Some(info) = self.cached.as_ref().and_then(|l| crate::paint::find_scroll_node(l, node)) {
                    v = v.min(info.max_scroll());
                    self.scroll_offsets.insert(info.id, v);
                }
                self.node_scroll.insert(node, v);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            if let Some(sr) = reqs.scroll
                && let Some(y) = sr.y
            {
                self.scroll_offset = if sr.relative { self.scroll_offset + y as f32 } else { y as f32 }.max(0.0);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
        }
        if self.js.as_ref().is_some_and(|js| js.take_dirty() | js.media_size_changed()) {
            // QB2: a DOM write is a restyle of the same document. While a
            // layout is still being shaped, don't start another (it would
            // make the in-flight result stale, so no complete frame would
            // ever land under steady writes); remember it and rebuild once
            // the current pass completes.
            if self.layout_in_flight && !self.layout_dirty {
                self.js_restyle_pending = true;
                return;
            }
            self.request_js_restyle();
        }
    }

    /// QF7: starts queued page requests, one thread each (like image
    /// fetches), as far as the policy's parallelism and byte caps allow.
    fn pump_net(&mut self) {
        let (proxy, generation) = (self.proxy.clone(), self.nav_generation);
        let exec = crate::js_net::net_executor();
        self.net.pump(&mut |job| {
            let (proxy, exec) = (proxy.clone(), exec.clone());
            thread::spawn(move || {
                let result = exec.execute(&job);
                let _ = proxy.send_event(AppEvent::JsNet(generation, job.id, result));
            });
        });
    }

    /// QF7: `AppEvent::JsNet` handler. A result of a superseded page is
    /// dropped; while `:help` parks the engine it is kept for `restore_saved_page`.
    pub(crate) fn handle_js_net(&mut self, generation: u64, id: u64, result: Result<WireResponse, NetError>) {
        if generation != self.nav_generation {
            return;
        }
        self.net.done(id);
        self.pump_net();
        if let Some(js) = &self.js {
            js.deliver_net(id, result);
            self.js_sync();
        } else if self.pre_help_page.as_ref().is_some_and(|p| p.js.is_some()) {
            self.parked_net.push((id, result));
        }
    }

    /// QF19w2: `AppEvent::JsWs`. Events of a page that is parked behind
    /// `:help` stay queued in its engine (`restore_saved_page` pumps them);
    /// events of a dropped engine are never read.
    pub(crate) fn handle_js_ws(&mut self) {
        let Some(js) = &self.js else { return };
        let more = js.pump_ws();
        if more {
            let _ = self.proxy.send_event(AppEvent::JsWs);
        }
        self.js_sync();
    }

    /// QB2: queue a restyle rebuild for JS DOM writes (the old complete
    /// frame stays up until the new one is fully shaped).
    fn request_js_restyle(&mut self) {
        self.js_restyle_pending = false;
        self.layout_dirty = true;
        self.restyle_requested = true;
        self.keep_scroll_on_rebuild = true;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Q33d: fires a trusted event at `target` (`None` = `<body>`) in the
    /// page's JS engine, then syncs any DOM writes the handlers made.
    /// Returns false if a handler cancelled the default action. Free for
    /// pages with no engine.
    pub(crate) fn js_dispatch(&mut self, target: Option<&Handle>, kind: &str, init: &[(&str, EventField)]) -> bool {
        let els = self.sync_node_scroll();
        let Some(js) = &self.js else { return true };
        js.set_scroll(self.scroll_offset, els);
        let ok = js.dispatch_event(target, kind, init);
        self.js_sync();
        ok
    }

    /// QAN1c: dispatches the animation events due at app clock `now` (ms),
    /// queues one restyle if an animation started or ended since the last
    /// build, and returns the app-clock time of the next change.
    pub(crate) fn anim_tick(&mut self, now: f64) -> Option<f64> {
        if self.anim.is_idle() {
            return None;
        }
        let t = self.anim.tick(now);
        for ev in t.events {
            let init = [
                ("bubbles", EventField::Bool(true)),
                ("animationName", EventField::Str(ev.name)),
                ("elapsedTime", EventField::Num(ev.elapsed_s)),
                ("pseudoElement", EventField::Str(ev.pseudo)),
            ];
            self.js_dispatch(ev.node.as_ref(), ev.kind, &init);
        }
        if t.restyle {
            if self.layout_in_flight && !self.layout_dirty {
                self.js_restyle_pending = true;
            } else {
                self.request_js_restyle();
            }
        }
        t.next
    }

    /// Q33d: a `click` at window position `pos` on `target`; true if the
    /// default action (following a link) may proceed.
    pub(crate) fn js_click(&mut self, target: Option<&Handle>, pos: (f32, f32)) -> bool {
        self.js_mouse("click", target, pos)
    }

    /// Q33d1: `mousedown`/`mouseup` at the element under `pos`.
    fn js_mouse_at(&mut self, kind: &str, pos: (f32, f32)) {
        if self.js.is_none() {
            return;
        }
        let node = self.element_at(pos);
        let target = if node == 0 { None } else { find_node_by_id(&self.document, node) };
        self.js_mouse(kind, target.as_ref(), pos);
    }

    fn js_mouse(&mut self, kind: &str, target: Option<&Handle>, pos: (f32, f32)) -> bool {
        let m = &self.modifiers;
        let init = [
            ("bubbles", EventField::Bool(true)),
            ("cancelable", EventField::Bool(true)),
            ("clientX", EventField::Num(pos.0 as f64)),
            ("clientY", EventField::Num(pos.1 as f64)),
            ("button", EventField::Num(0.0)),
            ("ctrlKey", EventField::Bool(m.control_key())),
            ("shiftKey", EventField::Bool(m.shift_key())),
            ("altKey", EventField::Bool(m.alt_key())),
            ("metaKey", EventField::Bool(m.super_key())),
        ];
        self.js_dispatch(target, kind, &init)
    }

    /// Q33d: `keydown`/`keyup` at the focused element (else `<body>`);
    /// true if the page did not cancel it.
    pub(crate) fn js_key(&mut self, kind: &str, event: &KeyEvent) -> bool {
        self.js_key_codes(kind, &event.logical_key, Some(event.physical_key), event.repeat)
    }

    pub(crate) fn js_key_for(&mut self, kind: &str, logical_key: &Key, repeat: bool) -> bool {
        self.js_key_codes(kind, logical_key, None, repeat)
    }

    /// `js_key` for a bare logical key (Q30d: the text-control path and its
    /// tests have no winit `KeyEvent`, which cannot be constructed by hand).
    pub(crate) fn js_key_codes(&mut self, kind: &str, logical_key: &Key, physical: Option<PhysicalKey>, repeat: bool) -> bool {
        if self.js.is_none() {
            return true;
        }
        let key = match logical_key {
            Key::Character(s) => s.to_string(),
            Key::Named(NamedKey::Space) => " ".to_string(),
            Key::Named(n) => format!("{n:?}"),
            _ => "Unidentified".to_string(),
        };
        let kc = key_codes(logical_key, physical, kind == "keypress");
        let m = &self.modifiers;
        let init = [
            ("bubbles", EventField::Bool(true)),
            ("cancelable", EventField::Bool(true)),
            ("key", EventField::Str(key)),
            ("code", EventField::Str(kc.code)),
            ("keyCode", EventField::Num(f64::from(kc.key_code))),
            ("charCode", EventField::Num(f64::from(kc.char_code))),
            ("location", EventField::Num(f64::from(kc.location))),
            ("repeat", EventField::Bool(repeat)),
            ("ctrlKey", EventField::Bool(m.control_key())),
            ("shiftKey", EventField::Bool(m.shift_key())),
            ("altKey", EventField::Bool(m.alt_key())),
            ("metaKey", EventField::Bool(m.super_key())),
        ];
        let target = self.interaction.focus.and_then(|f| find_node_by_id(&self.document, f));
        self.js_dispatch(target.as_ref(), kind, &init)
    }

    /// `AppEvent::NavResult` handler (see its docs).
    pub(crate) fn handle_nav_result(&mut self, generation: u64, result: Result<FetchedPage, String>) {
        // A result from a navigation the user has since clicked
        // past (a second link, before the first one's fetch
        // finished) — drop it silently rather than clobbering
        // whatever's already newer on screen. See `AppEvent::
        // NavResult`'s own docs.
        if generation != self.nav_generation {
            return;
        }
        match result {
            Ok(FetchedPage { html, extra_css, scripts, url }) => {
                let dom = new_parser().read_from(&mut html.as_slice()).unwrap();
                // Q30e: the history entry is where the page really is (after
                // redirects), so back/refresh re-GET the result, never a POST.
                if let Some(slot) = self.history.get_mut(self.history_index) {
                    let strip = |u: &Url| {
                        let mut u = u.clone();
                        u.set_fragment(None);
                        u
                    };
                    if matches!(url.scheme(), "http" | "https") && strip(slot) != url {
                        let frag = slot.fragment().map(str::to_string);
                        *slot = url.clone();
                        slot.set_fragment(frag.as_deref());
                    }
                }
                self.base_url = Some(resolve_page_base_url(&dom.document, url));
                // QY19: an immediate meta redirect replaces this page: do not
                // even install it (nor run its scripts).
                let meta = meta_refresh_action(&dom.document, self.base_url.as_ref(), self.meta_chain);
                if let Some((0, target)) = &meta {
                    self.follow_meta_refresh(target.clone());
                    return;
                }
                self.doc_gen = generation;
                self.history_doc.resize(self.history.len(), NO_DOC);
                if let Some(slot) = self.history_doc.get_mut(self.history_index) {
                    *slot = generation;
                }
                self.replace_document(dom.document, extra_css);
                // QA6: only now is the document the one the fragment names.
                self.pending_fragment = self.nav_fragment.take();
                self.js = run_page_scripts(&self.document, &scripts, self.base_url.as_ref());
                self.js_sync();
                if let Some((delay, target)) = meta {
                    let (proxy, generation) = (self.proxy.clone(), self.nav_generation);
                    thread::spawn(move || {
                        thread::sleep(std::time::Duration::from_secs(delay));
                        let _ = proxy.send_event(AppEvent::MetaRefresh(generation, target));
                    });
                }
            }
            Err(message) => {
                self.nav_fragment = None;
                self.show_message(&message);
            }
        }
    }

    /// QY19: follows a meta refresh to `target`. Counts against
    /// `META_REFRESH_MAX_CHAIN`. A 0-second redirect (the page was never
    /// installed) replaces the current history entry like an HTTP redirect;
    /// a delayed one pushes normally (`self.meta_chain` is only reset by user
    /// navigations).
    pub(crate) fn follow_meta_refresh(&mut self, target: Url) {
        self.meta_chain += 1;
        self.push_or_replace_history(target.clone(), true);
        self.nav_initiator = self.base_url.clone();
        self.fetch_url(target);
    }

    /// `AppEvent::MetaRefresh` handler: a delayed redirect pushes normally.
    pub(crate) fn handle_meta_refresh(&mut self, generation: u64, target: Url) {
        if generation == self.nav_generation {
            self.meta_chain += 1;
            self.push_history(target.clone());
            self.nav_initiator = self.base_url.clone();
            self.fetch_url(target);
        }
    }

    /// Replaces the current history entry with `url` (`replace`, no extra
    /// back step) when there is one; with none yet nothing is recorded (the
    /// next push seeds from `base_url`, which is then the target).
    pub(crate) fn push_or_replace_history(&mut self, url: Url, replace: bool) {
        if replace {
            if let Some(slot) = self.history.get_mut(self.history_index) {
                *slot = url;
            }
        } else {
            self.push_history(url);
        }
    }

    /// `AppEvent::ImageFetched` handler (see its docs).
    pub(crate) fn handle_image_fetched(&mut self, generation: u64, src: String, image: Option<DecodedImage>) {
        // Same staleness guard `NavResult` already uses — a slow
        // image fetch that only finishes after the user has since
        // navigated elsewhere must never populate the *new*
        // page's `image_cache` just because it happens to share
        // the same raw `src` string. See `AppEvent::ImageFetched`'s
        // own docs.
        if generation != self.nav_generation {
            return;
        }
        let Some(image) = image else {
            // QY56: a failed fetch still ends the image's loading: `complete`
            // turns true and `error` fires.
            self.image_settled(&src, None);
            return;
        };
        let size = (image.width, image.height);
        self.image_cache.insert(src.clone(), Arc::new(image));
        self.image_settled(&src, Some(size));
        // A newly-arrived image can change this element's own
        // intrinsic size (no explicit CSS/HTML width/height —
        // see `resolve_img`), so the whole tree needs a real
        // rebuild, not just a repaint. Q35b2: but a rebuild costs up to
        // ~430 ms on big pages, so a burst of arrivals is collapsed:
        // the rebuild is deferred by `IMAGE_RELAYOUT_DEBOUNCE` and, if
        // a shaping pass is still in flight, until it completes.
        if self.layout_dirty {
            // A rebuild is already queued (first layout, chunk, resize...).
            return;
        }
        let now = std::time::Instant::now();
        let since = *self.image_relayout_since.get_or_insert(now);
        if image_relayout_wait(Some(now.duration_since(since)), self.layout_dirty).is_some() && !self.image_flush_scheduled {
            self.schedule_image_flush(IMAGE_RELAYOUT_DEBOUNCE);
        }
        self.flush_image_relayout();
    }

    /// QY56: records the outcome of fetching `src` for page script
    /// (`complete`, `naturalWidth`) and fires `load` / `error` at the images
    /// that use it.
    pub(crate) fn image_settled(&mut self, src: &str, size: Option<(u32, u32)>) {
        crate::js_dom::note_image_state(src, size);
        if let Some(js) = &self.js {
            js.eval("img-done", &format!("__scratchy_img_done({})", js_string_literal(src)));
            self.js_sync();
        }
    }

    /// Q35b2: applies the pending image relayout if its debounce window
    /// has passed and no shaping pass is in flight (a completing pass
    /// calls this again, so nothing is lost).
    pub(crate) fn flush_image_relayout(&mut self) {
        let Some(since) = self.image_relayout_since else { return };
        if self.layout_in_flight || self.hold_old_frame {
            return;
        }
        if image_relayout_wait(Some(since.elapsed()), self.layout_dirty).is_some() {
            return;
        }
        self.image_relayout_since = None;
        self.layout_dirty = true;
        self.layout_generation += 1;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    pub(crate) fn schedule_image_flush(&mut self, delay: std::time::Duration) {
        self.image_flush_scheduled = true;
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            thread::sleep(delay);
            let _ = proxy.send_event(AppEvent::ImageFlush);
        });
    }

    /// `:help` — built the same way `show_message` builds its own
    /// synthetic pages (`parse_html` + `replace_document`), just with
    /// real, static content (`HELP_PAGE_HTML`) instead of one escaped
    /// paragraph. Deliberately not wired through any external fetch or
    /// file read — scratchy still does neither on its own (TODO.txt) —
    /// so a reference page is always there, with nothing that can fail
    /// to load. Snapshots the current page into `pre_help_page` first —
    /// but only if nothing's snapshotted already: re-running `:help`
    /// while already viewing it (its own text, read back from `self.
    /// document`, would otherwise get captured as "the page to go back
    /// to") must leave the real saved page alone, same "once, not
    /// every open" rule a real vim buffer-switch follows.
    pub(crate) fn show_help(&mut self) {
        if self.pre_help_page.is_none() {
            self.pre_help_page = Some(SavedPage {
                document: self.document.clone(),
                extra_css: self.extra_css.clone(),
                adopted_css: self.adopted_css.clone(),
                image_cache: self.image_cache.clone(),
                scroll_offset: self.scroll_offset,
                scroll_offsets: self.scroll_offsets.clone(),
                js: self.js.take(),
            });
            // QM1g: the parked page is hidden: its videos stop rendering.
            if let Some(js) = self.pre_help_page.as_ref().and_then(|p| p.js.as_ref()) {
                js.media_set_targets(&[]);
                js.media_set_laid(&[]);
            }
        }
        // QB6: help has no JS; a page's engine is parked in `pre_help_page`
        // (or already dropped when re-running `:help`).
        self.js = None;
        self.image_cache.clear();
        self.css_images_requested.clear();
        self.replace_document(parse_html(HELP_PAGE_HTML), String::new());
    }

    /// `:q`/`:quit`'s own "close the buffer" half (`dispatch_command`) —
    /// pops `pre_help_page` back onto `App` via the same fields `replace_
    /// document` itself sets, then restores the three it always clears/
    /// resets (`image_cache`, `scroll_offsets`, `scroll_offset`) to the
    /// saved page's own real values instead of empty/zero. Does nothing
    /// if `pre_help_page` is `None` — `dispatch_command`'s own caller is
    /// expected to have already checked that and fallen back to `should_
    /// quit` instead, but this stays a safe no-op either way rather than
    /// a `panic!`/`unwrap`, same defensive shape `TextEditBuffer`'s own
    /// start-of-buffer no-ops follow for a call that technically
    /// shouldn't happen rather than one that's actually expected to.
    pub(crate) fn restore_saved_page(&mut self) {
        let Some(saved) = self.pre_help_page.take() else { return };
        self.replace_document(saved.document, saved.extra_css);
        self.adopted_css = saved.adopted_css;
        js_style::set_js_adopted_css(&self.adopted_css);
        self.image_cache = saved.image_cache;
        self.scroll_offsets = saved.scroll_offsets;
        self.scroll_offset = saved.scroll_offset;
        self.js = saved.js;
        // QM1g: mpv's frame callback fires once until a frame is taken; frames that
        // arrived while the page was parked were dropped, so ask for one again.
        crate::media_video::wake();
        for (id, result) in std::mem::take(&mut self.parked_net) {
            if let Some(js) = &self.js {
                js.deliver_net(id, result);
            }
        }
        // QF19w2: WebSocket events that arrived while the page was parked.
        self.handle_js_ws();
        self.js_sync();
    }

    /// Entry point for `WindowEvent::MouseInput`'s click handler:
    /// resolves `href` against the current page's own `<base href>`
    /// (`resolve_href` — ordinary URL-join logic via the `url` crate,
    /// not hand-rolled), then, if that succeeds, shows "Loading…" and
    /// fetches the resolved page on a background thread, exactly the
    /// `thread::spawn` + proxy pattern `run()`'s own `Input::Stdin` case
    /// already established — see `AppEvent::NavResult`'s own docs for
    /// how the result gets back onto this thread and `nav_generation`'s
    /// own docs for why it's tagged.
    ///
    /// Per `scratchy/TODO.txt`'s own settled design, `--socket` is a
    /// strict opt-in: with none configured, or the configured one
    /// unreachable, this never silently does nothing — it prints the
    /// literal HTTP/1.1 request text to stdout (`print_fallback_
    /// request`, directly replayable by `nc`/a raw socket/a manually-run
    /// `wire`) and shows a clear inline error, rather than either
    /// guessing at a default daemon location or leaving the user
    /// wondering whether the click even registered.
    ///
    /// Also drops `pre_help_page`, if any was set — a real navigation
    /// (a real click, or `:go`) while `:help` happens to be open means
    /// the page *about* to show is genuinely new, not an overlay on top
    /// of whatever `:help` saved; a later `:q` should quit from there,
    /// not resurrect a page from before `:help` was ever opened. Cleared
    /// unconditionally, even on `resolve_href`'s own error path just
    /// below — an error message shown over `:help` is still "no longer
    /// showing `:help`."
    /// Q31: installs `new` as the live interaction state. The thread-local
    /// is always updated; a restyle (`layout_dirty`) is requested only when
    /// a kind of state that changed is actually mentioned by a stylesheet
    /// rule (`state_rules`), so pages without `:hover` rules pay nothing.
    pub(crate) fn set_interaction_state(&mut self, new: InteractionState) {
        if new == self.interaction {
            return;
        }
        let old = std::mem::replace(&mut self.interaction, new);
        let new = &self.interaction;
        set_interaction(new);
        let mut changed = 0;
        if old.hover != new.hover {
            changed |= STATE_HOVER;
        }
        if old.active != new.active {
            changed |= STATE_ACTIVE;
        }
        if old.focus != new.focus || old.focus_within != new.focus_within || old.focus_visible != new.focus_visible {
            changed |= STATE_FOCUS;
        }
        if old.target != new.target {
            changed |= STATE_TARGET;
        }
        if changed & STATE_FOCUS != 0 && changed & self.state_rules == 0 {
            // No stylesheet rule depends on focus: only the painted focus
            // ring changes, so repaint without a restyle.
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        if changed & self.state_rules != 0 {
            self.layout_dirty = true;
            self.restyle_requested = true;
            self.keep_scroll_on_rebuild = true;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    /// The `node_id` of the element under window-space `pos` (0 = none).
    pub(crate) fn element_at(&self, pos: (f32, f32)) -> usize {
        match &self.cached {
            Some(laid) => hit_test_element(
                laid, pos.0, pos.1 + self.scroll_offset, &self.shape_cache, &self.scroll_offsets, self.scroll_offset,
                self.surface_size.1 as f32,
            ),
            None => 0,
        }
    }

    /// Ancestor-inclusive id set for a `node_id`, empty if it is gone.
    pub(crate) fn chain_for(&self, node: usize) -> std::collections::HashSet<usize> {
        if node == 0 {
            return Default::default();
        }
        find_node_by_id(&self.document, node).map(|h| ancestor_ids(&h).into_iter().collect()).unwrap_or_default()
    }

    /// Q33d1: `mouseout` on the node the pointer left, `mouseover` on the
    /// one it entered (both bubble; `relatedTarget` is the other). Runs only
    /// on a change of the hit node and never restyles by itself, so it is
    /// independent of the debounced `:hover` restyle machinery (QB3).
    fn js_hover_events(&mut self, old: usize, new: usize) {
        if self.js.is_none() {
            return;
        }
        let pos = self.last_mouse_pos.unwrap_or((0.0, 0.0));
        let m = &self.modifiers;
        let hold = |n: usize| (n != 0).then(|| find_node_by_id(&self.document, n)).flatten();
        let (old_h, new_h) = (hold(old), hold(new));
        let mk = |related: &Option<Handle>| {
            let mut v = vec![
                ("bubbles", EventField::Bool(true)),
                ("cancelable", EventField::Bool(true)),
                ("clientX", EventField::Num(pos.0 as f64)),
                ("clientY", EventField::Num(pos.1 as f64)),
                ("ctrlKey", EventField::Bool(m.control_key())),
                ("shiftKey", EventField::Bool(m.shift_key())),
                ("altKey", EventField::Bool(m.alt_key())),
                ("metaKey", EventField::Bool(m.super_key())),
            ];
            if let Some(r) = related {
                v.push(("relatedTargetId", EventField::Node(r.clone())));
            }
            v
        };
        let (out_init, over_init) = (mk(&new_h), mk(&old_h));
        if let Some(h) = &old_h {
            self.js_dispatch(Some(h), "mouseout", &out_init);
        }
        if let Some(h) = &new_h {
            self.js_dispatch(Some(h), "mouseover", &over_init);
        }
    }

    pub(crate) fn update_hover(&mut self, node: usize) {
        if node == self.hover_node {
            return;
        }
        let old = self.hover_node;
        self.hover_node = node;
        self.js_hover_events(old, node);
        if self.state_rules & STATE_HOVER == 0 {
            return;
        }
        // R2.3: a hover restyle is a full rebuild, so coalesce them: at
        // most one per `HOVER_RESTYLE_INTERVAL`, none while a restyle is
        // still in flight or the wheel was just used. The latest hover
        // target is applied by a deferred `AppEvent::HoverFlush`.
        if let Some(delay) = self.hover_restyle_delay() {
            self.schedule_hover_flush(delay);
            return;
        }
        self.apply_hover();
    }

    pub(crate) fn hover_restyle_delay(&self) -> Option<std::time::Duration> {
        let now = std::time::Instant::now();
        let mut delay = std::time::Duration::ZERO;
        if let Some(t) = self.last_hover_restyle {
            delay = delay.max(HOVER_RESTYLE_INTERVAL.saturating_sub(now.duration_since(t)));
        }
        if let Some(t) = self.last_wheel {
            delay = delay.max(HOVER_WHEEL_QUIET.saturating_sub(now.duration_since(t)));
        }
        if self.hold_old_frame || self.layout_dirty {
            delay = delay.max(HOVER_RESTYLE_INTERVAL);
        }
        // QB3: on a page whose builds are slow, wait for the pointer to stop.
        if let Some(w) = hover_idle_wait(self.last_build_time, self.last_pointer_move.map(|t| now.duration_since(t))) {
            delay = delay.max(w);
        }
        (!delay.is_zero()).then_some(delay)
    }

    pub(crate) fn schedule_hover_flush(&mut self, delay: std::time::Duration) {
        if self.hover_flush_scheduled {
            return;
        }
        self.hover_flush_scheduled = true;
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            thread::sleep(delay);
            let _ = proxy.send_event(AppEvent::HoverFlush);
        });
    }

    /// Installs the latest hover chain as interaction state (restyling
    /// if a `:hover` rule exists).
    pub(crate) fn apply_hover(&mut self) {
        let mut st = self.interaction.clone();
        st.hover = self.chain_for(self.hover_node);
        if st.hover == self.interaction.hover {
            return;
        }
        // R2.4: only elements entering/leaving the hover chain can change
        // style. If none of them could match a `:hover` compound, record
        // the new chain (so later diffs are right) but don't restyle.
        let relevant = st.hover.symmetric_difference(&self.interaction.hover).any(|&id| {
            find_node_by_id(&self.document, id).is_none_or(|h| self.hover_keys.may_match(&h))
        });
        if relevant {
            self.last_hover_restyle = Some(std::time::Instant::now());
            self.set_interaction_state(st);
        } else {
            self.interaction.hover = st.hover;
            set_interaction(&self.interaction);
        }
    }

    /// Focuses `h` (or blurs on `None`). `visible` = reached by keyboard
    /// (`:focus-visible`). The entry point forms (Q30) reuse.
    pub(crate) fn set_focus_element(&mut self, h: Option<&Handle>, visible: bool) {
        let mut st = self.interaction.clone();
        match h {
            Some(h) => {
                let ids = ancestor_ids(h);
                st.focus = ids.first().copied();
                st.focus_within = ids.into_iter().collect();
                st.focus_visible = visible;
            }
            None => {
                st.focus = None;
                st.focus_within.clear();
                st.focus_visible = false;
            }
        }
        let (old_focus, new_focus) = (self.interaction.focus, st.focus);
        crate::js_dom::set_active_element(h);
        self.set_interaction_state(st);
        if old_focus != new_focus {
            self.select_popup_close(PopupEvent::Blur);
            // Q30d: `change` fires before `blur`; the value is remembered at focus.
            if let Some(h) = old_focus.and_then(|f| find_node_by_id(&self.document, f)) {
                self.text_focus_lost(&h);
            }
            if let Some(h) = new_focus.and_then(|f| find_node_by_id(&self.document, f)) {
                self.text_focus_gained(&h);
            }
        }
        if old_focus != new_focus && self.js.is_some() {
            // Q33d: blur/focusout on the old element, focus/focusin on the
            // new one (focus and blur do not bubble; the *in/out* pair do).
            let init = |bubbles: bool| [("bubbles", EventField::Bool(bubbles))];
            if let Some(h) = old_focus.and_then(|f| find_node_by_id(&self.document, f)) {
                self.js_dispatch(Some(&h), "blur", &init(false));
                self.js_dispatch(Some(&h), "focusout", &init(true));
            }
            if let Some(h) = new_focus.and_then(|f| find_node_by_id(&self.document, f)) {
                self.js_dispatch(Some(&h), "focus", &init(false));
                self.js_dispatch(Some(&h), "focusin", &init(true));
            }
        }
    }

    /// Mouse press: element + ancestors become `:active`, and the nearest
    /// focusable ancestor-or-self takes focus (not `:focus-visible`).
    pub(crate) fn press_state(&mut self, pos: (f32, f32)) {
        let node = self.element_at(pos);
        let target = if node == 0 { None } else { find_node_by_id(&self.document, node) };
        let mut focus = None;
        let mut cur = target.clone();
        while let Some(n) = cur {
            if is_click_focusable(&n) {
                focus = Some(n);
                break;
            }
            // Q30b: a press inside a disabled control focuses nothing.
            if is_disabled_control(&n) {
                break;
            }
            cur = parent_handle(&n);
        }
        let visible = focus.as_ref().is_some_and(click_focus_is_visible);
        let mut st = self.interaction.clone();
        st.active = target.map(|h| ancestor_ids(&h).into_iter().collect()).unwrap_or_default();
        self.set_interaction_state(st);
        self.set_focus_element(focus.as_ref(), visible);
    }

    /// Q30b: a click on `<label>` is forwarded to its control: the control
    /// takes focus and gets its own click event (checkbox/radio toggling is
    /// Q30c). Returns whether a control was found.
    pub(crate) fn forward_label_click(&mut self, label: &Handle, pos: (f32, f32)) -> bool {
        let Some(control) = label_control(&self.document, label) else { return false };
        if is_disabled_control(&control) {
            return true;
        }
        self.set_focus_element(Some(&control), click_focus_is_visible(&control));
        self.click_activate(Some(&control), pos);
        true
    }

    pub(crate) fn release_state(&mut self) {
        if !self.interaction.active.is_empty() {
            let mut st = self.interaction.clone();
            st.active.clear();
            self.set_interaction_state(st);
        }
    }

    /// Tab / Shift+Tab: move focus through `focusable_in_tab_order`.
    pub(crate) fn tab_focus(&mut self, backwards: bool) {
        let order = focusable_in_tab_order(&self.document);
        if order.is_empty() {
            return;
        }
        let cur = self.interaction.focus.and_then(|f| order.iter().position(|h| node_id(h) == f));
        let next = match (cur, backwards) {
            (None, false) => 0,
            (None, true) => order.len() - 1,
            (Some(i), false) => (i + 1) % order.len(),
            (Some(i), true) => (i + order.len() - 1) % order.len(),
        };
        self.set_focus_element(Some(&order[next]), true);
        self.text_focus_by_tab(&order[next]);
        self.scroll_focus_into_view();
    }

    /// Q31b: scrolls the page the minimum amount that brings the focused
    /// element fully into view (with a small margin). No-op when it is
    /// already visible or has no laid-out geometry yet.
    pub(crate) fn scroll_focus_into_view(&mut self) {
        let (Some(f), Some(laid)) = (self.interaction.focus, &self.cached) else { return };
        let geom = node_geometry(laid, &self.shape_cache, f);
        let Some((top, bottom)) = geom.rects.iter().fold(None, |acc: Option<(f32, f32)>, r| {
            Some(acc.map_or((r.1, r.1 + r.3), |(t, b)| (t.min(r.1), b.max(r.1 + r.3))))
        }) else {
            return;
        };
        let view = self.surface_size.1 as f32;
        const MARGIN: f32 = 24.0;
        if top < self.scroll_offset {
            self.scroll_offset = (top - MARGIN).max(0.0);
        } else if bottom > self.scroll_offset + view {
            self.scroll_offset = (bottom + MARGIN - view).max(0.0).min(top.max(0.0));
        } else {
            return;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Q31b: `#fragment` navigation within the current document: makes the
    /// named element `:target` and scrolls it to the top of the viewport
    /// (an empty or `top` fragment scrolls to the top and clears the
    /// target). Returns false while the element has no layout yet (a page
    /// still shaping), so the caller can retry; true once done or when the
    /// fragment names nothing.
    pub(crate) fn apply_fragment(&mut self, frag: &str) -> bool {
        // Raw first (an id may really contain `%`), then percent-decoded.
        let target = find_fragment_target(&self.document, frag)
            .or_else(|| find_fragment_target(&self.document, &percent_decode_fragment(frag)));
        let frag = frag.to_string();
        let mut st = self.interaction.clone();
        st.target = target.as_ref().map(node_id);
        self.set_interaction_state(st);
        let Some(target) = target else {
            if frag.is_empty() || frag.eq_ignore_ascii_case("top") {
                self.scroll_offset = 0.0;
            }
            return true;
        };
        let Some(laid) = &self.cached else { return false };
        // An inline element without its own box falls back to its ancestors.
        let mut cur = Some(target);
        while let Some(n) = cur {
            let geom = node_geometry(laid, &self.shape_cache, node_id(&n));
            if let Some(top) = geom.rects.iter().map(|r| r.1).reduce(f32::min) {
                self.scroll_offset = top.max(0.0);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                return true;
            }
            cur = parent_handle(&n);
        }
        false
    }

    /// Q30c: a toggle of checkbox/radio live state changed what is painted.
    /// Pages whose stylesheets use `:checked`/`:indeterminate` get a restyle
    /// (the same async path as `:hover`); otherwise only a repaint, since the
    /// mark is drawn from the live state at paint time (no layout rebuild).
    pub(crate) fn check_state_changed(&mut self) {
        if self.state_rules & STATE_CHECKED != 0 {
            self.layout_dirty = true;
            self.restyle_requested = true;
            self.keep_scroll_on_rebuild = true;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Q30c: a click on `target` (None = body). A checkbox/radio toggles BEFORE
    /// the click event (legacy pre-activation) and is restored when a handler
    /// cancels it; `input` then `change` bubble from it when the state really
    /// changed. A reset button resets its form's live state. Returns whether
    /// the click was not cancelled.
    pub(crate) fn click_activate(&mut self, target: Option<&Handle>, pos: (f32, f32)) -> bool {
        let Some(control) = target.and_then(enclosing_control) else { return self.js_click(target, pos) };
        let Some(radio) = checkable_kind(&control) else {
            let ok = self.js_click(target, pos);
            let is_reset = matches!(element_tag(&control), "button" | "input")
                && element_attr(&control, "type").is_some_and(|t| t.trim().eq_ignore_ascii_case("reset"));
            if ok && is_reset {
                self.reset_owner_form(&control);
            }
            // Q30g: a click on a select opens its dropdown / picks a listbox row.
            if ok && element_tag(&control) == "select" {
                self.select_click(&control, pos);
            }
            // Q30e: a submit button submits its form owner.
            if ok && is_submit_button(&control) && !is_disabled_control(&control) && let Some(form) = form_owner(&control) {
                self.request_submit(&form, Some(&control));
            }
            return ok;
        };
        let (was, was_ind) = (live_checked(&control), live_indeterminate(&control));
        if radio && was {
            return self.js_click(target, pos);
        }
        let peer = if radio { checked_radio_peer(&control) } else { None };
        set_indeterminate(&control, false);
        set_checked(&control, if radio { true } else { !was });
        self.check_state_changed();
        if !self.js_click(target, pos) {
            set_checked(&control, was);
            set_indeterminate(&control, was_ind);
            if let Some(p) = peer {
                set_checked(&p, true);
            }
            self.check_state_changed();
            return false;
        }
        let ev = [("bubbles", EventField::Bool(true))];
        self.js_dispatch(Some(&control), "input", &ev);
        self.js_dispatch(Some(&control), "change", &ev);
        true
    }

    /// Q30c: the `reset` event (cancelable) at the form owning `control`, then
    /// its checkbox/radio state goes back to the defaults.
    pub(crate) fn reset_owner_form(&mut self, control: &Handle) {
        let Some(form) = form_owner(control) else { return };
        let ev = [("bubbles", EventField::Bool(true)), ("cancelable", EventField::Bool(true))];
        if self.js_dispatch(Some(&form), "reset", &ev) && reset_form_state(&form) {
            self.check_state_changed();
        }
    }

    /// Enter on a focused link follows it.
    pub(crate) fn activate_focus(&mut self) {
        let Some(f) = self.interaction.focus else { return };
        let Some(h) = find_node_by_id(&self.document, f) else { return };
        if matches!(element_tag(&h), "a" | "area") {
            // Q33d: activation fires a (synthetic) click the page may cancel.
            if !self.js_click(Some(&h), (0.0, 0.0)) {
                return;
            }
            let href = element_attr(&h, "href").map(|v| v.to_string());
            if let Some(href) = href {
                self.start_navigation(&href);
            }
        }
    }

    /// A page-initiated navigation (link click, script): the current page is
    /// the initiator for the cookie jar's SameSite decision (QF14b).
    pub(crate) fn start_navigation(&mut self, href: &str) {
        self.start_navigation_as(href, true);
    }

    /// `page_initiated` false = typed by the user (`:go`): no initiator.
    pub(crate) fn start_navigation_as(&mut self, href: &str, page_initiated: bool) {
        self.navigate_to(href, page_initiated, false);
    }

    /// QF11: `replace` (location.replace) overwrites the current history
    /// entry instead of pushing one.
    pub(crate) fn navigate_to(&mut self, href: &str, page_initiated: bool, replace: bool) {
        self.meta_chain = 0;
        self.pre_help_page = None;
        if let (Some(frag), None) = (href.strip_prefix('#'), &self.base_url) {
            // No base to resolve against, but a bare fragment needs none.
            self.apply_fragment(frag);
            return;
        }
        let url = match resolve_href(self.base_url.as_ref(), href) {
            Ok(url) => url,
            Err(message) => {
                self.show_message(&message);
                return;
            }
        };
        self.push_or_replace_history(url.clone(), replace);
        self.nav_initiator = if page_initiated { self.base_url.clone() } else { None };
        self.fetch_url(url);
    }

    /// Records `url` as the page now on screen, for `:historyBack`/
    /// `:historyFront` to move through later — called once per real
    /// navigation (a click, `:go`), never for a back/forward move
    /// itself (`navigate_history` only moves `history_index`, it never
    /// calls this). Truncates anything ahead of the current entry first,
    /// same as a real browser: navigating somewhere new while sitting
    /// behind the front of history forks off the back half rather than
    /// leaving an old "forward" branch dangling.
    ///
    /// The very first call is a special case: `history` starts empty
    /// (the page the session opened on — loaded from a file, stdin, or
    /// wherever — was never itself pushed, there having been no
    /// navigation yet to push it *for*), so a *first* click with
    /// nothing seeded would leave `history` holding only the page just
    /// navigated *to*, with no earlier entry for `:historyBack` to ever
    /// reach — "no more history" on the very first press despite a real
    /// previous page having been on screen a moment before. `self.
    /// base_url` at this point is still the *old* page's own address
    /// (this runs from `start_navigation`, before `fetch_url`'s
    /// eventual `NavResult` overwrites it — see `resolve_page_base_url`'s
    /// own docs for why that's reliably the real page URL, not just a
    /// link-resolution base) — exactly the one entry missing, so it's
    /// seeded here, once, the first time there's nothing else in
    /// `history` yet to truncate against.
    pub(crate) fn push_history(&mut self, url: Url) {
        let mut seeded = false;
        if self.history.is_empty() {
            if let Some(current) = self.base_url.clone().filter(|b| *b != url) {
                self.history.push(current);
                seeded = true;
            }
        } else {
            self.history.truncate(self.history_index + 1);
        }
        self.history_doc.resize(self.history.len(), NO_DOC);
        if seeded && let Some(slot) = self.history_doc.last_mut() {
            *slot = self.doc_gen;
        }
        self.history.push(url);
        self.history_doc.push(NO_DOC);
        self.history_index = self.history.len() - 1;
    }

    /// QY48: the history entry at `i` belongs to the document on screen.
    fn same_doc_entry(&self, i: usize) -> bool {
        self.history_doc.get(i).copied().unwrap_or(NO_DOC) == self.doc_gen
    }

    /// QY48: the page's URL changed without a navigation (`old` -> `new`):
    /// `base_url` follows unless the page set its own `<base href>` (then
    /// `base_url` was not the page URL to begin with).
    fn retarget_base(&mut self, old: Option<&Url>, new: &Url) {
        let strip = |u: &Url| {
            let mut u = u.clone();
            u.set_fragment(None);
            u
        };
        let cur = old.or(self.base_url.as_ref()).map(strip);
        if self.base_url.as_ref().map(strip) == cur {
            self.base_url = Some(strip(new));
        }
    }

    /// QY48: `history.pushState/replaceState/go` made by the page: update the
    /// App's URL and history without a fetch.
    fn apply_url_event(&mut self, ev: crate::js_dom::UrlEv) {
        use crate::js_dom::UrlEv;
        match ev {
            UrlEv::Set(href, replace) => {
                let Ok(url) = Url::parse(&href) else { return };
                let old = self.history.get(self.history_index).cloned();
                if replace {
                    if let Some(slot) = self.history.get_mut(self.history_index) {
                        *slot = url.clone();
                    }
                } else {
                    // pushState to the very same URL still adds an entry: seed the
                    // current page first so it stays reachable.
                    if self.history.is_empty()
                        && let Some(cur) = self.base_url.clone()
                    {
                        self.history.push(cur);
                        self.history_doc.push(self.doc_gen);
                        self.history_index = 0;
                    }
                    self.push_history(url.clone());
                    if let Some(slot) = self.history_doc.get_mut(self.history_index) {
                        *slot = self.doc_gen;
                    }
                }
                self.retarget_base(old.as_ref(), &url);
            }
            UrlEv::Move(delta) => {
                let Some(i) = self.history_index.checked_add_signed(delta).filter(|&i| i < self.history.len()) else { return };
                if !self.same_doc_entry(i) {
                    return;
                }
                let old = self.history.get(self.history_index).cloned();
                self.history_index = i;
                let url = self.history[i].clone();
                self.retarget_base(old.as_ref(), &url);
            }
        }
    }

    /// QY48: back/forward to a same-document entry: the page's own state is
    /// restored (`popstate`), nothing is fetched. False = not applicable.
    fn traverse_same_document(&mut self, new_index: usize) -> bool {
        if self.js.is_none() || !self.same_doc_entry(new_index) || !self.same_doc_entry(self.history_index) {
            return false;
        }
        let delta = new_index as isize - self.history_index as isize;
        let url = self.history[new_index].clone();
        let Some(js) = &self.js else { return false };
        if !js.history_traverse(delta, url.as_str()) {
            return false;
        }
        let old = self.history.get(self.history_index).cloned();
        self.history_index = new_index;
        self.retarget_base(old.as_ref(), &url);
        if let Some(frag) = url.fragment() {
            let frag = frag.to_string();
            self.apply_fragment(&frag);
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        true
    }

    /// `:historyBack`/`:historyFront` (`delta` of `-1`/`1`, bound to
    /// `h`/`l` in vim Normal mode via `Action::HistoryBack`/`
    /// HistoryFront`) — moves `history_index` and re-fetches that entry
    /// through the ordinary `fetch_url` path, but deliberately never
    /// through `push_history`: moving through history must never itself
    /// grow the stack, or going back then forward again would duplicate
    /// entries instead of retracing them. Already at either end of
    /// `history` is a silent no-op, not an error message: a real
    /// browser's back/forward button simply does nothing once there's
    /// nowhere further to go, rather than replacing the page on screen
    /// with an error.
    pub(crate) fn navigate_history(&mut self, delta: isize) {
        let Some(new_index) = self.history_index.checked_add_signed(delta).filter(|&i| i < self.history.len()) else {
            return;
        };
        if self.traverse_same_document(new_index) {
            return;
        }
        self.history_index = new_index;
        self.meta_chain = 0;
        let url = self.history[new_index].clone();
        self.fetch_url(url);
    }

    /// `:refresh` — re-fetches the page currently on screen in place,
    /// through the same `fetch_url` path `navigate_history` uses, and
    /// for the same reason: this must never call `push_history`, or
    /// refreshing would grow the stack instead of just reloading what's
    /// already there. Not yet bound to any key — command-line only for
    /// now. Prefers the current `history` entry, but a page reached
    /// without ever going through `push_history` (the initial file/
    /// stdin/URL load, before any click or `:go` away from it) has
    /// nothing there yet — `base_url` is the fallback for exactly that
    /// case (reliably the real page URL by then — see `push_history`'s
    /// own docs for why), so `scratchy <url>` followed immediately by
    /// `:refresh`, with no navigation in between, still works. Only a
    /// `base_url`-less page (a local file with no `<base href>` of its
    /// own) has truly nothing to refresh.
    pub(crate) fn refresh(&mut self) {
        self.meta_chain = 0;
        let url = match self.history.get(self.history_index).cloned() {
            Some(url) => url,
            None => match self.base_url.clone() {
                Some(url) => url,
                None => {
                    self.show_message("scratchy: nothing to refresh");
                    return;
                }
            },
        };
        self.fetch_url_inner(url);
    }

    /// The actual fetch-and-swap logic shared by a fresh navigation
    /// (`start_navigation`, after `push_history`), a back/forward move
    /// (`navigate_history`), and `:refresh` (both of the latter two
    /// deliberately never call `push_history`) — see `AppEvent::
    /// NavResult`'s own docs for how the result gets back onto this
    /// thread.
    pub(crate) fn fetch_url(&mut self, url: Url) {
        // Q31b: a link that only changes the fragment of the page already on
        // screen is not a fetch: just move `:target` and scroll.
        if let (Some(frag), Some(base)) = (url.fragment(), &self.base_url) {
            let strip = |u: &Url| {
                let mut u = u.clone();
                u.set_fragment(None);
                u
            };
            if strip(base) == strip(&url) {
                let frag = frag.to_string();
                self.nav_initiator = None;
                // QY48: the entry for this fragment lives in the current document.
                if let Some(slot) = self.history_doc.get_mut(self.history_index) {
                    *slot = self.doc_gen;
                }
                self.apply_fragment(&frag);
                return;
            }
        }
        self.fetch_url_inner(url);
    }

    /// `fetch_url` without the same-document shortcut (`:refresh` must
    /// really reload). The fragment is kept for after the new page's
    /// layout (`pending_fragment`) and never sent in the request.
    pub(crate) fn fetch_url_inner(&mut self, url: Url) {
        // QA6: the "Loading…" page shown below finishes layout before the
        // real one arrives, so the fragment must not be pending yet; it is
        // held in `nav_fragment` and promoted by `handle_nav_result`.
        self.pending_fragment = None;
        self.nav_fragment = url.fragment().map(str::to_string);
        let mut url = url;
        url.set_fragment(None);
        self.nav_generation += 1;
        self.js = None;
        self.net.reset();
        self.parked_net.clear();
        self.image_cache.clear();
        self.css_images_requested.clear();
        self.late_images_requested = 0;
        crate::js_dom::clear_image_states();
        let generation = self.nav_generation;
        self.show_message("Loading…");
        let proxy = self.proxy.clone();
        let socket_path = self.socket_path.clone();
        let initiator = self.nav_initiator.take();
        let post = self.pending_post.take();
        let page_for_origin = self.base_url.clone();
        thread::spawn(move || {
            let ctx = CookieContext::navigation(&url, initiator.as_ref());
            let request_text = match &post {
                Some(p) => build_post_request_ctx(&url, &ctx, p, page_for_origin.as_ref()),
                None => build_get_request_ctx(&url, &ctx),
            };
            let url_for_log = url.clone();
            let outcome: Result<FetchedPage, String> = match &socket_path {
                Some(path) => {
                    begin_wire_load(generation);
                    let fetch_start = std::time::Instant::now();
                    let wire_result = fetch_raw_via_wire(path, &request_text);
                    if timing_enabled() {
                        diag!(
                            "scratchy: TIMING page_fetch={:>7.1}ms {url_for_log}",
                            fetch_start.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    match wire_result {
                        WireFetchRaw::Responded { content_type, body, final_url } => match dispatch_wire_response(content_type, body) {
                            Ok(html) => {
                                // Sub-resources (v67): fetched from *this*
                                // background thread, not the event-loop one —
                                // stylesheets synchronously, since CSS must be
                                // fully in hand before the very first layout
                                // (see `process_external_resources`'s own
                                // docs); images only kicked off here, each
                                // resolving independently and later, via its
                                // own `AppEvent::ImageFetched`.
                                let css_start = std::time::Instant::now();
                                let PageResources { css: extra_css, scripts } = process_external_resources(&proxy, path, &url, &html, generation);
                                if timing_enabled() {
                                    diag!(
                                        "scratchy: TIMING stylesheets={:>7.1}ms {url_for_log}",
                                        css_start.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                // Q30e: a redirect (a form POST's 303, Wikipedia's
                                // search 302) moves the page: relative links
                                // resolve against where it ended up.
                                let url = final_url.and_then(|f| Url::parse(&f).ok()).filter(|f| matches!(f.scheme(), "http" | "https")).map(|mut f| { f.set_fragment(None); f }).unwrap_or(url);
                                Ok(FetchedPage { html, extra_css, scripts, url })
                            }
                            Err(e) => Err(e),
                        },
                        WireFetchRaw::Unreachable => {
                            print_fallback_request(&request_text);
                            Err(format!(
                                "wire daemon not reachable at {} — printed the request to stdout; \
                                 start it there (e.g. `wire --socket {}`) and click again, or replay \
                                 the printed request manually",
                                path.display(),
                                path.display()
                            ))
                        }
                    }
                }
                None => {
                    print_fallback_request(&request_text);
                    Err("no --socket given — scratchy never fetches a page on its own without \
                         one; printed the request to stdout for manual replay"
                        .to_string())
                }
            };
            let _ = proxy.send_event(AppEvent::NavResult(generation, outcome));
        });
    }

    /// The one real entry point a pressed key reaches `App` through —
    /// see its own call site (`WindowEvent::KeyboardInput`, filtered to
    /// `ElementState::Pressed` there, release events never reach here
    /// at all). `vim_mode == Command` always goes to the command-line
    /// editor regardless of `vim_enabled`: once `:` has been pressed
    /// (only possible while vim mode was already on), backing out with
    /// `Escape` — or finishing a `:set novim` typed from *inside* the
    /// bar — must still work even the instant after `vim_enabled` would
    /// otherwise gate everything else out. Otherwise, vim mode off is
    /// real pass-through: this function does nothing at all, the "zero
    /// claimed keys" guarantee the whole toggle exists for (TODO.txt).
    pub(crate) fn handle_key_event(&mut self, event: &KeyEvent) {
        if self.vim_mode == VimMode::Command {
            self.pending_g = false;
            self.handle_command_line_key(event);
            return;
        }
        if !self.vim_enabled {
            self.pending_g = false;
            return;
        }
        let (g_action, pending, consumed) = resolve_pending_g(self.pending_g, &event.logical_key, self.modifiers);
        self.pending_g = pending;
        if consumed {
            if let Some(action) = g_action {
                self.perform_action(action);
            }
            return;
        }
        if let Some(action) = resolve_binding(&event.logical_key, self.modifiers, self.vim_mode) {
            if event.repeat && action_ignores_repeat(action) {
                return;
            }
            self.perform_action(action);
        }
    }

    /// Carries out an already-resolved `Action` — kept separate from
    /// `resolve_binding` itself so that function can stay a pure,
    /// unit-testable `key -> Option<Action>` mapping with no `&mut App`
    /// at all.
    pub(crate) fn perform_action(&mut self, action: Action) {
        const SCROLL_LINE_PX: f32 = 40.0; // matches `MouseWheel`'s own `LineDelta` step
        match action {
            Action::ScrollDown => self.scroll_by(SCROLL_LINE_PX),
            Action::ScrollUp => self.scroll_by(-SCROLL_LINE_PX),
            Action::HalfPageDown => self.scroll_by(self.surface_size.1 as f32 / 2.0),
            Action::HalfPageUp => self.scroll_by(-(self.surface_size.1 as f32 / 2.0)),
            Action::EnterCommandMode => {
                self.vim_mode = VimMode::Command;
                self.command_line.clear();
                // Doesn't go through `handle_command_line_key`'s own
                // "any non-`Tab` key resets `completion`" rule at all
                // (that function isn't even reachable yet — `vim_mode`
                // is still `Normal` here) — reset directly, or a stale
                // completion left over from a previous command-line
                // session (Tab-cycled, then `Escape`d without typing
                // anything else) would wrongly survive into this new,
                // now-empty one.
                self.completion = None;
            }
            Action::ScrollTop => self.scroll_offset = 0.0,
            // `redraw` clamps to the real max scroll every frame, same as `j`.
            Action::ScrollBottom => self.scroll_offset = f32::MAX,
            Action::HistoryBack => self.navigate_history(-1),
            Action::HistoryFront => self.navigate_history(1),
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Only clamps the lower bound (`0.0`) — same split `WindowEvent::
    /// MouseWheel`'s own top-level-scroll branch already relies on:
    /// `redraw`'s own per-frame `self.scroll_offset.clamp(0.0, max_
    /// scroll)` is what enforces the upper bound, since `max_scroll`
    /// depends on `self.cached.h`, which this method has no reason to
    /// duplicate.
    pub(crate) fn scroll_by(&mut self, dy: f32) {
        self.scroll_offset = (self.scroll_offset + dy).max(0.0);
    }

    /// `vim_mode == Command`'s own key handling — direct text-buffer
    /// editing (`TextEditBuffer`'s methods), not a `resolve_binding`
    /// lookup: there's no sensible "binding" for "the character the
    /// user just typed," and command-mode text input needs to see the
    /// actual OS-composed logical key output (dead keys, non-Latin
    /// layouts via `logical_key`'s own IME-adjacent handling), which is
    /// exactly what the ordinary page a browser would otherwise send
    /// key events to gets too.
    pub(crate) fn handle_command_line_key(&mut self, event: &KeyEvent) {
        // Any key other than `Tab` itself ends a `Tab`-cycling run in
        // progress — see `CommandCompletion`'s own docs. Checked before
        // the `match` below (not folded into each of its non-`Tab` arms
        // individually) so a future arm can't accidentally skip it the
        // way a one-off `_ =>` branch addition easily could.
        if !matches!(event.logical_key, Key::Named(NamedKey::Tab)) {
            self.completion = None;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.vim_mode = VimMode::Normal;
                self.command_line.clear();
            }
            Key::Named(NamedKey::Enter) => {
                let input = std::mem::take(&mut self.command_line.text);
                self.command_line.cursor = 0;
                self.vim_mode = VimMode::Normal;
                self.run_command_line(&input);
            }
            Key::Named(NamedKey::Backspace) => self.command_line.backspace(),
            Key::Named(NamedKey::ArrowLeft) => self.command_line.move_left(),
            Key::Named(NamedKey::ArrowRight) => self.command_line.move_right(),
            Key::Named(NamedKey::Tab) => self.cycle_command_completion(),
            // Same `mods`-checked-alongside-`logical_key` shape `resolve_
            // binding`'s own `Ctrl`+`d`/`u` arms use, for the same reason:
            // `Ctrl` is excluded from `logical_key`'s own shift-adjustment,
            // so a bare `v` has to be told apart from `Ctrl`+`v` here, not
            // left to `key_insertion_text`'s catch-all below (which would
            // otherwise insert a literal "v" on every paste attempt).
            Key::Character(s) if s.as_str() == "v" && self.modifiers.control_key() => {
                self.paste_into_command_line();
            }
            key => {
                if let Some(text) = key_insertion_text(key) {
                    for c in text.chars() {
                        self.command_line.insert(c);
                    }
                }
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// `Tab`'s own handler inside the command bar. The first `Tab` in a
    /// fresh run computes the candidate list for whatever token the
    /// cursor is at (`command_completions`) and completes to its first
    /// match, alphabetically; every *immediately following* `Tab` (no
    /// other key in between — enforced by `handle_command_line_key`'s
    /// own reset, not here) instead advances `self.completion`'s own
    /// index, wrapping back to the first candidate after the last. A
    /// token with no candidates at all (anything beyond `:go`'s own
    /// URL argument, today) leaves `self.completion` untouched — `Tab`
    /// is simply a no-op there, same as a real shell's completion when
    /// there's nothing to offer.
    pub(crate) fn cycle_command_completion(&mut self) {
        match &mut self.completion {
            Some(state) => state.index = (state.index + 1) % state.candidates.len(),
            None => {
                let cursor = self.command_line.cursor;
                let token_start = token_start_before(&self.command_line.text, cursor);
                let candidates = command_completions(&self.command_line.text, token_start, cursor);
                if candidates.is_empty() {
                    return;
                }
                self.completion = Some(CommandCompletion { token_start, candidates, index: 0 });
            }
        }
        let state = self.completion.as_ref().expect("just set or matched Some above");
        let candidate = state.candidates[state.index];
        let token_start = state.token_start;
        // `token_start..cursor` is exactly the text to replace, both on
        // a fresh completion (cursor is the end of whatever prefix was
        // actually typed) and on every cycle after (the previous call
        // left `cursor` at `token_start + candidate.len()`, i.e. right
        // after the *previous* candidate it inserted) — no separate
        // bookkeeping needed for "how much to remove" beyond `cursor`
        // itself.
        self.command_line.text.replace_range(token_start..self.command_line.cursor, candidate);
        self.command_line.cursor = token_start + candidate.len();
    }

    /// `Ctrl`+`v` inside the command bar: reads the system clipboard
    /// and inserts whatever text it holds at the cursor — the one way
    /// to get a URL copied from somewhere else (another app, a
    /// terminal) into `:go` without retyping it by hand. Opens a fresh
    /// `Clipboard` connection on every call rather than keeping one on
    /// `App` permanently: pasting is a rare, explicitly user-initiated
    /// action, not a hot path, so there's no real cost to paying `arboard`'s
    /// connection setup each time, versus a permanent connection that
    /// would need its own lifecycle (reconnect-on-failure, `Send`
    /// across `App`'s otherwise plain, non-threaded fields) for a
    /// feature that doesn't need one. Either the connection or the read
    /// failing (no clipboard server reachable, or genuinely empty) is a
    /// silent no-op, not an error message — same as a real terminal or
    /// browser's own paste behavior when there's simply nothing to
    /// paste.
    pub(crate) fn paste_into_command_line(&mut self) {
        if let Ok(mut clipboard) = Clipboard::new()
            && let Ok(text) = clipboard.get_text()
        {
            self.command_line.insert_str(&text);
        }
    }

    /// Splits a submitted command line into its name and whitespace-
    /// separated args and hands it to `dispatch_command` — kept as its
    /// own step so `dispatch_command` itself only ever deals with
    /// already-tokenized input, same shape a future `:`-command with
    /// quoted args could still slot into later without reworking the
    /// split-and-dispatch boundary.
    pub(crate) fn run_command_line(&mut self, input: &str) {
        let mut parts = input.split_whitespace();
        let Some(name) = parts.next() else { return };
        let args: Vec<&str> = parts.collect();
        self.dispatch_command(name, &args);
    }

    /// The one chokepoint a submitted `:`-command's name+args get
    /// resolved into its actual effect through — hardcoded `match` arms
    /// today, same "named override point for a future JS-scriptable
    /// config layer" shape `resolve_binding` already follows for
    /// keybindings (TODO.txt). `COMMAND_NAMES` is this `match`'s own
    /// completion-candidate shadow, not the other way around — adding a
    /// name to one without the other either completes to something
    /// that reports itself unknown (missing here) or leaves a working
    /// command un-completable (missing there), so the two have to be
    /// kept in step by hand. `:go` reuses `start_navigation` outright —
    /// exactly the same href-resolve-then-fetch path a real `<a>` click
    /// already goes through, so an already-absolute URL (the common
    /// case typed here) or one relative to the current page's own
    /// `<base href>` both just work. `:q`/`:quit` (real vim's own two
    /// spellings for the same command) are real-vim-flavored too in a
    /// second way: with `:help` open, they close it and fall back to
    /// whatever was showing before (`restore_saved_page`), same as vim
    /// closing a buffer and dropping back to the previous one — only
    /// actually quitting (`should_quit`, not `ActiveEventLoop::exit()`
    /// directly — see that field's own docs for why `dispatch_command`
    /// has no event loop handle to call it with) once there's nothing
    /// left to fall back to.
    pub(crate) fn dispatch_command(&mut self, name: &str, args: &[&str]) {
        match (name, args) {
            ("go", [url]) => self.start_navigation_as(url, false),
            ("historyBack", []) => self.navigate_history(-1),
            ("historyFront", []) => self.navigate_history(1),
            ("refresh", []) => self.refresh(),
            ("set", ["vim"]) => self.vim_enabled = true,
            ("set", ["novim"]) => self.vim_enabled = false,
            // Takes effect from the next page load (`:refresh`).
            ("set", ["js"]) => set_js_enabled(true),
            ("set", ["nojs"]) => set_js_enabled(false),
            // QAN1d0: takes effect from the next page load (`:refresh`).
            ("set", ["motion"]) => set_reduced_motion(false),
            ("set", ["nomotion"]) => set_reduced_motion(true),
            ("help", []) => self.show_help(),
            ("q", []) | ("quit", []) => {
                if self.pre_help_page.is_some() {
                    self.restore_saved_page();
                } else {
                    self.should_quit = true;
                }
            }
            _ => {
                let rest = args.join(" ");
                self.show_message(&format!("scratchy: unknown command: :{name}{}{rest}", if rest.is_empty() { "" } else { " " }));
            }
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window_attrs = Window::default_attributes()
            .with_title("scratchy")
            .with_inner_size(LogicalSize::new(1024.0, 768.0))
            .with_decorations(false);
        let window = Rc::new(event_loop.create_window(window_attrs).unwrap());
        let context = Context::new(window.clone()).unwrap();
        let surface = Surface::new(&context, window.clone()).unwrap();

        self.window = Some(window);
        self.surface = Some(surface);
    }

    /// Q33t: run due JS timers, then sleep until the next one (or
    /// indefinitely when none are pending, so timer-less pages never
    /// wake up on their own).
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let els = self.sync_node_scroll();
        if let Some(js) = &self.js {
            js.set_scroll(self.scroll_offset, els);
            js.run_due();
        }
        self.js_sync();
        let now_ms = app_clock_ms();
        let anim_at = self.anim_tick(now_ms).map(|ms| std::time::Instant::now() + std::time::Duration::from_secs_f64(((ms - now_ms).max(0.0) + 1.0) / 1000.0));
        let js_at = self.js.as_ref().and_then(|js| js.next_deadline());
        let ctl_at = self.controls_refresh();
        let flow = match [js_at, anim_at, ctl_at].into_iter().flatten().min() {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        };
        event_loop.set_control_flow(flow);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::VideoFrame => {
                crate::media_video::wake_handled();
                if self.video_tick() {
                    return;
                }
            }
            AppEvent::Chunk(bytes) => {
                if let Some(decoder) = &mut self.decoder {
                    decoder.process(ByteTendril::from_slice(&bytes));
                    self.document = decoder.inner_sink.tokenizer.sink.sink.document.clone();
                    self.layout_dirty = true;
                    self.layout_generation += 1;
                }
            }
            AppEvent::Eof => {
                if let Some(decoder) = self.decoder.take() {
                    let dom = decoder.finish();
                    self.document = dom.document;
                    self.layout_dirty = true;
                    self.layout_generation += 1;
                    // Streaming mode (`snag url | scratchy`) never had a
                    // `<base href>` to read before now — the whole
                    // document, `<head>` included, only exists once EOF
                    // has been seen. Picking it up here (rather than
                    // leaving `base_url` permanently `None` for a piped
                    // page) is what makes clicking a link actually work
                    // after `snag url | scratchy`, not just after
                    // `scratchy saved-bundle.html`.
                    self.base_url = find_base_url(&self.document);
                    if let Some(socket) = self.socket_path.clone() {
                        start_stream_resources(&self.proxy, &socket, &self.document, self.nav_generation);
                    } else {
                        self.js = run_page_scripts(&self.document, &inline_scripts(&self.document), self.base_url.as_ref());
                        self.js_sync();
                    }
                }
            }
            AppEvent::StreamCss(generation, css, scripts) => {
                if generation != self.nav_generation {
                    return;
                }
                if !css.is_empty() {
                    js_style::set_js_extra_css(&css);
                    self.extra_css = css;
                    self.layout_dirty = true;
                    self.layout_generation += 1;
                }
                self.js = run_page_scripts(&self.document, &scripts, self.base_url.as_ref());
                self.js_sync();
            }
            AppEvent::NavResult(generation, result) => self.handle_nav_result(generation, result),
            AppEvent::JsNet(generation, id, result) => self.handle_js_net(generation, id, result),
            AppEvent::JsWs => self.handle_js_ws(),
            AppEvent::ImageFetched(generation, src, image) => self.handle_image_fetched(generation, src, image),
            AppEvent::ShapeResult(generation, laid, shape_cache, incomplete) => self.handle_shape_result(generation, laid, shape_cache, incomplete),
            AppEvent::MetaRefresh(generation, target) => self.handle_meta_refresh(generation, target),
            AppEvent::ImageFlush => {
                self.image_flush_scheduled = false;
                if let Some(since) = self.image_relayout_since {
                    // The burst this timer was armed for may have been
                    // replaced (navigation); re-arm for the remainder.
                    if let Some(left) = image_relayout_wait(Some(since.elapsed()), self.layout_dirty) {
                        self.schedule_image_flush(left);
                    } else {
                        self.flush_image_relayout();
                    }
                }
            }
            AppEvent::HoverFlush => {
                self.hover_flush_scheduled = false;
                if self.state_rules & STATE_HOVER != 0 {
                    if let Some(delay) = self.hover_restyle_delay() {
                        self.schedule_hover_flush(delay);
                    } else {
                        self.apply_hover();
                    }
                }
            }
            AppEvent::ToggleVimMode => {
                self.vim_enabled = !self.vim_enabled;
                self.vim_mode = VimMode::Normal;
                self.command_line.clear();
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Release events never reach `handle_key_event` at all —
                // nothing here is driven by a key actually being *held*
                // (as opposed to repeatedly *pressed*, which winit's own
                // OS-level key-repeat already reports as more `Pressed`
                // events with `repeat: true`, no release in between), so
                // there's nothing a release would need to do. Scroll
                // relies on exactly that repeat to feel continuous while
                // held; `action_ignores_repeat` is where a one-shot
                // action like `HistoryBack`/`HistoryFront` opts back out
                // of it instead.
                // Q33d: page `keydown`/`keyup` handlers see keys first (never
                // the `:` command line) and can cancel them.
                if self.vim_mode != VimMode::Command {
                    let kind = if event.state == ElementState::Pressed { "keydown" } else { "keyup" };
                    if !self.js_key(kind, &event) {
                        return;
                    }
                }
                if event.state == ElementState::Pressed {
                    // Q30g: an open select dropdown owns every key (Tab closes it, then moves focus).
                    if self.vim_mode != VimMode::Command && self.select_popup.is_some() && self.select_popup_key(&event.logical_key) {
                        return;
                    }
                    // Q30g: a focused closed select / listbox owns the keys like a text control.
                    if self.vim_mode != VimMode::Command
                        && !matches!(event.logical_key, Key::Named(NamedKey::Tab))
                        && let Some(h) = self.focused_select()
                        && self.select_control_key(&h, &event.logical_key)
                    {
                        return;
                    }
                    // Q30d: a focused text control owns the keys (Tab and
                    // the page's own keydown handlers excepted).
                    if self.vim_mode != VimMode::Command
                        && !matches!(event.logical_key, Key::Named(NamedKey::Tab))
                        && let Some(h) = self.focused_text_control()
                        && self.text_control_key(&h, &event.logical_key, event.repeat)
                    {
                        return;
                    }
                    if self.vim_mode != VimMode::Command && self.media_key(&event.logical_key, event.repeat) {
                        return;
                    }
                    if self.vim_mode != VimMode::Command {
                        match event.logical_key {
                            Key::Named(NamedKey::Tab) => {
                                self.tab_focus(self.modifiers.shift_key());
                                if let Some(w) = &self.window {
                                    w.request_redraw();
                                }
                                return;
                            }
                            // Q30c: Space toggles a focused checkbox/radio.
                            Key::Named(NamedKey::Space)
                                if !event.repeat
                                    && !self.modifiers.control_key()
                                    && !self.modifiers.alt_key()
                                    && let Some(h) = self.interaction.focus.and_then(|f| find_node_by_id(&self.document, f))
                                    && checkable_kind(&h).is_some()
                                    && !is_disabled_control(&h) =>
                            {
                                self.click_activate(Some(&h), (0.0, 0.0));
                                return;
                            }
                            // Q30e: Enter/Space presses a focused button.
                            Key::Named(NamedKey::Enter | NamedKey::Space)
                                if !event.repeat
                                    && let Some(h) = self.interaction.focus.and_then(|f| find_node_by_id(&self.document, f))
                                    && (element_tag(&h) == "button" || (element_tag(&h) == "input" && matches!(input_type(&h).as_str(), "submit" | "image" | "reset" | "button")))
                                    && !is_disabled_control(&h) =>
                            {
                                self.click_activate(Some(&h), (0.0, 0.0));
                                return;
                            }
                            Key::Named(NamedKey::Enter) if self.interaction.focus.is_some() => {
                                self.activate_focus();
                                return;
                            }
                            _ => {}
                        }
                    }
                    self.handle_key_event(&event);
                    // `:q`/`:quit` (`dispatch_command`) only set this
                    // flag — `event_loop.exit()` itself has to happen
                    // here, the one place this whole call chain actually
                    // has an `&ActiveEventLoop` to call it on (see `App::
                    // should_quit`'s own docs).
                    if self.should_quit {
                        event_loop.exit();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // Remembered in window/screen space, same form this event
                // itself reports — `MouseWheel` (just below) needs it to
                // pick the right scroll region, since that event carries
                // no position of its own at all.
                self.last_mouse_pos = Some((position.x as f32, position.y as f32));
                self.last_pointer_move = Some(std::time::Instant::now());
                // `position` is window/screen space; `Laid.x`/`.y` are
                // document space (scroll isn't applied until paint time),
                // so the same `+ scroll_offset` adjustment `paint` itself
                // makes (in reverse) is needed here too.
                self.text_mouse_drag((position.x as f32, position.y as f32));
                if self.video.drag.is_some() {
                    self.controls_drag_move((position.x as f32, position.y as f32), false);
                    return;
                }
                self.controls_pointer((position.x as f32, position.y as f32));
                if self.select_popup_hover((position.x as f32, position.y as f32)) {
                    return;
                }
                let hovered = self.element_at((position.x as f32, position.y as f32));
                self.update_hover(hovered);
                if let (Some(laid), Some(window)) = (&self.cached, &self.window) {
                    let doc_x = position.x as f32;
                    let doc_y = position.y as f32 + self.scroll_offset;
                    let (style, _href) = hit_test_cursor(
                        laid, doc_x, doc_y, &self.shape_cache, &self.scroll_offsets, self.scroll_offset,
                        self.surface_size.1 as f32,
                    );
                    let icon = style.to_winit();
                    if icon != self.last_cursor {
                        match icon {
                            // Restore visibility too, in case the cursor
                            // was hidden (`cursor: none`) immediately
                            // beforehand — only when actually transitioning
                            // out of that state, not on every tick.
                            Some(ic) => {
                                window.set_cursor(ic);
                                if self.last_cursor.is_none() {
                                    window.set_cursor_visible(true);
                                }
                            }
                            None => window.set_cursor_visible(false),
                        }
                        self.last_cursor = icon;
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                // Scratchy has no text selection yet (see `is_click`'s
                // own docs), but binding navigation to mouse-down alone
                // would still be wrong on its own terms — a drag that
                // happens to start and end over a link shouldn't
                // navigate either. `winit`'s own `MouseInput` carries no
                // position of its own (same gap `MouseWheel` already
                // has) — `last_mouse_pos`, kept live by `CursorMoved`
                // just above, stands in for both the press and release
                // point.
                if button != MouseButton::Left {
                    return;
                }
                match state {
                    ElementState::Pressed => {
                        if let Some(pos) = self.last_mouse_pos
                            && self.select_popup_press(pos)
                        {
                            self.mouse_down_pos = None;
                            return;
                        }
                        if let Some(pos) = self.last_mouse_pos
                            && self.controls_press(pos)
                        {
                            self.mouse_down_pos = None;
                            return;
                        }
                        self.mouse_down_pos = self.last_mouse_pos;
                        if let Some(pos) = self.last_mouse_pos {
                            self.js_mouse_at("mousedown", pos);
                            self.press_state(pos);
                            self.text_mouse_press(pos);
                        }
                    }
                    ElementState::Released => {
                        if self.video.drag.is_some() {
                            if let Some(pos) = self.last_mouse_pos {
                                self.controls_drag_move(pos, true);
                            }
                            self.video.drag = None;
                            self.release_state();
                            self.mouse_down_pos = None;
                            return;
                        }
                        if std::mem::take(&mut self.swallow_release) {
                            self.mouse_down_pos = None;
                            return;
                        }
                        self.text_mouse_release();
                        if let Some(pos) = self.last_mouse_pos {
                            self.js_mouse_at("mouseup", pos);
                        }
                        self.release_state();
                        // Computed into a plain, owned `Option<&'static
                        // str>` (not borrowed from `self`) *before*
                        // calling `start_navigation` below, which needs
                        // `&mut self` — `laid`/`shape_cache`/
                        // `scroll_offsets` are only borrowed for this
                        // one `match`, not held across that call.
                        let clicked = match (self.mouse_down_pos, self.last_mouse_pos) {
                            (Some(down), Some(up)) if is_click(down, up, CLICK_DRAG_THRESHOLD_PX) => Some(up),
                            _ => None,
                        };
                        // Q33d: a page's click handler runs first and can
                        // cancel the navigation with `preventDefault`.
                        // Q30b: a click inside a disabled control does
                        // nothing at all; a click inside any control (or on
                        // a label forwarding to one) never follows a link
                        // around it ("the control wins").
                        let mut in_control = false;
                        let allowed = match clicked {
                            Some(up) => {
                                let node = self.element_at(up);
                                let target = if node == 0 { None } else { find_node_by_id(&self.document, node) };
                                let control = target.as_ref().and_then(enclosing_control);
                                in_control = control.is_some();
                                if control.as_ref().is_some_and(is_disabled_control) {
                                    in_control = true;
                                    false
                                } else {
                                    let ok = self.click_activate(target.as_ref(), up);
                                    if ok
                                        && control.is_none()
                                        && let Some(label) = target.as_ref().and_then(enclosing_label)
                                    {
                                        in_control = self.forward_label_click(&label, up);
                                    }
                                    ok
                                }
                            }
                            _ => true,
                        };
                        let href = match (self.mouse_down_pos.take(), self.last_mouse_pos, &self.cached) {
                            (Some(down), Some(up), Some(laid)) if is_click(down, up, CLICK_DRAG_THRESHOLD_PX) => {
                                let doc_x = up.0;
                                let doc_y = up.1 + self.scroll_offset;
                                hit_test_cursor(
                                    laid, doc_x, doc_y, &self.shape_cache, &self.scroll_offsets,
                                    self.scroll_offset, self.surface_size.1 as f32,
                                )
                                .1
                            }
                            _ => None,
                        };
                        self.mouse_down_pos = None;
                        if let (Some(href), true, false) = (href, allowed, in_control) {
                            self.start_navigation(href);
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.last_wheel = Some(std::time::Instant::now());
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 40.0,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32,
                };
                if self.select_popup_wheel(dy) {
                    return;
                }
                // Routes to the deepest `overflow: scroll`/`auto` region
                // under the last known mouse position, if any — falls
                // through to the page's own top-level scroll (the
                // pre-v62 behavior, unchanged) whenever the cursor isn't
                // over one, or hasn't moved over the window at all yet.
                let region = self.last_mouse_pos.zip(self.cached.as_ref()).and_then(|((mx, my), laid)| {
                    let doc_x = mx;
                    let doc_y = my + self.scroll_offset;
                    hit_test_scroll_region(
                        laid, doc_x, doc_y, &self.scroll_offsets, self.scroll_offset, self.surface_size.1 as f32,
                    )
                });
                match region {
                    Some(info) => {
                        let current = self.scroll_offsets.get(&info.id).copied().unwrap_or(0.0);
                        let next = (current - dy).clamp(0.0, info.max_scroll());
                        self.scroll_offsets.insert(info.id, next);
                        if info.node != 0 {
                            self.node_scroll.insert(info.node, next);
                            self.node_scroll_dirty = true;
                        }
                    }
                    None => {
                        self.scroll_offset -= dy;
                        if self.scroll_offset < 0.0 {
                            self.scroll_offset = 0.0;
                        }
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

/// Q35a: starts a fetch for every external `background-image`/`mask-image`
/// `url()` the layout build just missed in `image_cache`
/// (`take_wanted_css_images`), through the same `spawn_image_fetches` path
/// `<img>` uses; the result arrives as `AppEvent::ImageFetched` keyed by
/// the url text. Each url is requested once per page and at most
/// `MAX_CSS_IMAGE_FETCHES` distinct urls are fetched.
pub(crate) fn fetch_wanted_css_images(
    proxy: &EventLoopProxy<AppEvent>,
    socket: Option<&std::path::Path>,
    base: Option<&Url>,
    requested: &mut HashSet<String>,
    generation: u64,
) {
    let wanted = take_wanted_css_images();
    if wanted.is_empty() {
        return;
    }
    let Some(socket) = socket else { return };
    let budget = MAX_CSS_IMAGE_FETCHES.saturating_sub(requested.len());
    let fresh: Vec<String> = wanted.into_iter().filter(|u| !requested.contains(u)).take(budget).collect();
    if fresh.is_empty() {
        return;
    }
    requested.extend(fresh.iter().cloned());
    let base = base.cloned().unwrap_or_else(|| Url::parse("about:blank").expect("static url"));
    spawn_image_fetches(proxy, socket, &base, &base, fresh, generation);
}

/// QY56: starts fetches for `<img>` sources that turned up after load (the
/// layout build missed them in `image_cache`, or a script set a `src` on an
/// element that may never be laid out). Each URL is fetched once per page
/// (`claim_image_fetch`, shared with the load-time fetch), at most
/// `MAX_LATE_IMAGE_FETCHES` late ones; the rest land in `errors` (they fail
/// with `error`).
pub(crate) fn fetch_late_images(
    proxy: &EventLoopProxy<AppEvent>,
    socket: Option<&std::path::Path>,
    base: Option<&Url>,
    count: &mut usize,
    errors: &mut Vec<String>,
    generation: u64,
    srcs: Vec<String>,
) {
    let Some(socket) = socket else { return };
    let mut fresh = Vec::new();
    for src in srcs {
        if src.trim().is_empty() || crate::images::decode_data_uri(&src).is_some() || !claim_image_fetch(generation, &src) {
            continue;
        }
        if *count >= MAX_LATE_IMAGE_FETCHES {
            if *count == MAX_LATE_IMAGE_FETCHES {
                *count += 1;
                diag!("scratchy: image fetch cap ({MAX_LATE_IMAGE_FETCHES} late images) reached; further script-added images are not loaded");
            }
            errors.push(src);
            continue;
        }
        *count += 1;
        fresh.push(src);
    }
    if fresh.is_empty() {
        return;
    }
    let base = base.cloned().unwrap_or_else(|| Url::parse("about:blank").expect("static url"));
    spawn_claimed_image_fetches(proxy, socket, &base, &base, fresh, generation);
}

/// QJ12: the legacy and physical-key fields of a KeyboardEvent.
#[derive(Debug, PartialEq)]
pub(crate) struct KeyCodes {
    pub code: String,
    pub key_code: u32,
    pub char_code: u32,
    pub location: u32,
}

/// DOM `code` for a logical key when there is no physical key (text-control path).
fn code_from_logical(k: &Key) -> String {
    match k {
        Key::Character(s) => {
            let mut it = s.chars();
            match (it.next(), it.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => format!("Key{}", c.to_ascii_uppercase()),
                (Some(c), None) if c.is_ascii_digit() => format!("Digit{c}"),
                (Some(' '), None) => "Space".to_string(),
                _ => String::new(),
            }
        }
        Key::Named(NamedKey::Space) => "Space".to_string(),
        Key::Named(n) => {
            let s = format!("{n:?}");
            if s.starts_with('F') || matches!(s.as_str(), "Enter" | "Escape" | "Tab" | "Backspace" | "Delete" | "Home" | "End" | "PageUp" | "PageDown" | "Insert" | "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown") {
                s
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Legacy `keyCode` (Windows virtual-key values) for a DOM `code`.
fn vk_from_code(code: &str) -> u32 {
    if let Some(l) = code.strip_prefix("Key")
        && let [c @ b'A'..=b'Z'] = l.as_bytes()
    {
        return u32::from(*c);
    }
    if let Some(d) = code.strip_prefix("Digit")
        && let [c @ b'0'..=b'9'] = d.as_bytes()
    {
        return u32::from(*c);
    }
    if let Some(d) = code.strip_prefix("Numpad")
        && let [c @ b'0'..=b'9'] = d.as_bytes()
    {
        return 96 + u32::from(c - b'0');
    }
    if let Some(n) = code.strip_prefix('F').and_then(|n| n.parse::<u32>().ok())
        && (1..=24).contains(&n)
    {
        return 111 + n;
    }
    match code {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" | "NumpadEnter" => 13,
        "ShiftLeft" | "ShiftRight" => 16,
        "ControlLeft" | "ControlRight" => 17,
        "AltLeft" | "AltRight" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" => 27,
        "Space" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Insert" => 45,
        "Delete" => 46,
        "MetaLeft" => 91,
        "MetaRight" => 92,
        "ContextMenu" => 93,
        "NumpadMultiply" => 106,
        "NumpadAdd" => 107,
        "NumpadSubtract" => 109,
        "NumpadDecimal" => 110,
        "NumpadDivide" => 111,
        "NumLock" => 144,
        "ScrollLock" => 145,
        "Semicolon" => 186,
        "Equal" => 187,
        "Comma" => 188,
        "Minus" => 189,
        "Period" => 190,
        "Slash" => 191,
        "Backquote" => 192,
        "BracketLeft" => 219,
        "Backslash" => 220,
        "BracketRight" => 221,
        "Quote" => 222,
        _ => 0,
    }
}

/// QJ12: `code`, `keyCode`, `charCode`, `location` for a key event. `physical`
/// is the winit physical key when there is one; without it `code` is derived
/// from the logical key. `keypress` has `charCode` = the code point (13 for
/// Enter) and `keyCode` the same; keydown/keyup have `charCode` 0.
pub(crate) fn key_codes(logical: &Key, physical: Option<PhysicalKey>, keypress: bool) -> KeyCodes {
    let mut code = match physical {
        Some(PhysicalKey::Code(c)) => format!("{c:?}"),
        _ => String::new(),
    };
    if code.is_empty() {
        code = code_from_logical(logical);
    }
    let location = if code.starts_with("Numpad") {
        3
    } else if code.ends_with("Left") && !code.starts_with("Arrow") && !code.starts_with("Bracket") {
        1
    } else if code.ends_with("Right") && !code.starts_with("Arrow") && !code.starts_with("Bracket") {
        2
    } else {
        0
    };
    let mut key_code = vk_from_code(&code);
    if key_code == 0
        && let Key::Character(s) = logical
        && let Some(c) = s.chars().next().filter(|_| s.chars().count() == 1)
        && c.is_ascii_alphanumeric()
    {
        key_code = u32::from(c.to_ascii_uppercase());
    }
    if keypress {
        let ch = match logical {
            Key::Character(s) => s.chars().next().map_or(0, |c| c as u32),
            Key::Named(NamedKey::Space) => 32,
            Key::Named(NamedKey::Enter) => 13,
            _ => 0,
        };
        return KeyCodes { code, key_code: ch, char_code: ch, location };
    }
    KeyCodes { code, key_code, char_code: 0, location }
}
