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
    /// QY54b1: what `extra_css` is made of (empty: unknown).
    pub(crate) sheets: crate::link_css::SheetParts,
    pub(crate) scripts: Vec<ScriptSource>,
    pub(crate) url: Url,
}

pub(crate) enum AppEvent {
    /// QCOMP2: an external completion source finished (or could not start); dropped unless it is for
    /// the `:` line that is open now.
    CompleteResult(crate::completion::ExtResult),
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
    /// `(nav generation, request id, result, when the result was complete)`; QOBS6 stamps the time.
    JsNet(u64, u64, Result<WireResponse, NetError>, std::time::Instant),
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
    /// QFF1a: a web font fetch finished: (nav generation, face key, registry page, the decoded face).
    FontFetched(u64, String, u64, Option<Box<fontdb::FaceInfo>>),
    /// Q22b: the stylesheets of a streamed page, fetched after EOF
    /// (`start_stream_resources`); replaces `App::extra_css`.
    StreamCss(u64, String, crate::link_css::SheetParts, Vec<ScriptSource>),
    /// QY54b1: a stylesheet a link asked for after load arrived (or failed):
    /// (nav_generation, the sheet key from `link_css::link_key`, its text).
    LinkCss(u64, String, Option<String>),
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
    /// The last fields are the worker's next free paragraph id (QINC1b) and, on the complete
    /// delivery of an engine job (QINC1j), the laid-out box-tree copy for the engine to adopt.
    ShapeResult(u64, Box<Laid>, ShapeCache, bool, usize, Option<Box<crate::incr_frame::WorkerResult>>),
    /// A deferred hover restyle (R2.3) is due; see `App::update_hover`.
    HoverFlush,
    /// Q35b2: the image-relayout debounce window (`IMAGE_RELAYOUT_DEBOUNCE`)
    /// has passed; see `App::flush_image_relayout`.
    ImageFlush,
    /// QY19: a `<meta http-equiv=refresh>` delay elapsed for the page of
    /// navigation generation `.0`; follow `.1` unless the user has since
    /// navigated (generation changed).
    MetaRefresh(u64, Url),
}

/// `Normal` resolves bare keys into an `Action` via `resolve_binding`;
/// `Command` is the `:`-entered command line itself, editing `App::
/// command_line` directly rather than going through any binding at
/// all. Only reachable from `KeyMode::Normal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VimMode {
    Normal,
    Command,
    /// QSEARCH1: the `/` line (status bar), same editor (`command_line`) as `:`.
    Search,
}

/// QKEYS2 (KEYS.md): who gets the keys. `Input` = the page (and the UA defaults), scratchy's
/// bindings are never consulted; `Normal` = scratchy's bindings, the page sees no key events.
/// Sticky: only Esc Esc (Input -> Normal), `i` or a click into a text control/select
/// (Normal -> Input) change it. The `:` line (`VimMode::Command`) is a sub-state of `Normal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyMode {
    Input,
    Normal,
}

/// QKEYS2: the Esc Esc recogniser's state (KEYS.md ESC1). `Esc1(t)` = one Esc seen at `t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EscState {
    Idle,
    Esc1(std::time::Instant),
}

/// QKEYS2: KEYS.md T1-T5. Given the state, a key PRESS (`repeat` = OS auto-repeat), the
/// modifiers and the time now, returns the new state and whether this press completes Esc Esc
/// (enter normal mode; the caller must not deliver this press to the page). Pure: the caller
/// passes the clock. Only a plain Esc (no Ctrl/Alt/Super) counts; a repeat or any other key
/// resets to `Idle`; an Esc later than `ESC_ESC_MS` after the first starts over (T3).
pub(crate) fn resolve_esc_esc(state: EscState, key: &Key, mods: ModifiersState, repeat: bool, now: std::time::Instant) -> (EscState, bool) {
    let plain_esc = matches!(key, Key::Named(NamedKey::Escape)) && !mods.control_key() && !mods.alt_key() && !mods.super_key();
    if !plain_esc || repeat {
        return (EscState::Idle, false);
    }
    match state {
        EscState::Esc1(t) if now.saturating_duration_since(t) <= std::time::Duration::from_millis(crate::config::ESC_ESC_MS) => {
            (EscState::Idle, true)
        }
        _ => (EscState::Esc1(now), false),
    }
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
    /// QXSCR1: ArrowLeft / ArrowRight scroll the window horizontally by one line.
    ScrollLeft,
    ScrollRight,
    HalfPageDown,
    HalfPageUp,
    /// QYSCR1: `Space` / `PageDown`: one viewport (less one line of overlap).
    PageDown,
    /// QYSCR1: `Shift+Space` / `PageUp`.
    PageUp,
    EnterCommandMode,
    /// QSEARCH1 (T8): `/` opens the search line.
    EnterSearchMode,
    /// QSEARCH1 (T7): Esc in normal mode clears the search highlights.
    ClearSearch,
    /// QSEARCH2 (T15-T17): `n`, the next match.
    SearchNext,
    /// QSEARCH2 (T15-T17): `N`, the previous match.
    SearchPrev,
    /// QSEARCH3 (T18-T20): Enter clicks the element holding the current match.
    ActivateMatch,
    /// QKEYS2 (T6): `i` hands the keyboard back to the page.
    EnterInputMode,
    HistoryBack,
    HistoryFront,
    /// QREG1 (T55): `yy`, copy the URL (`:let @+ = @%`).
    YankUrl,
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
    matches!(action, Action::ActivateMatch | Action::HistoryBack | Action::HistoryFront | Action::YankUrl | Action::EnterCommandMode | Action::EnterSearchMode | Action::EnterInputMode)
}

/// The one chokepoint a real key event gets resolved into an `Action`
/// through — reads the `config::BINDINGS` table (QCONF1), but the exact
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
    crate::config::BINDINGS.iter().find(|b| b.matches(key, mods)).map(|b| b.action)
}

/// QYSCR1: whether a wheel tick of `dy` px (positive = up, winit's sign)
/// over scroll region `info` moves it. A region with nothing to scroll
/// (`overflow: scroll` on a box whose content fits) or already at its end in
/// that direction passes the tick on to the page (scroll chaining).
pub(crate) fn region_takes_wheel(info: &ScrollInfo, offsets: &HashMap<usize, f32>, dy: f32) -> bool {
    let current = offsets.get(&info.id).copied().unwrap_or(0.0);
    (current - dy).clamp(0.0, info.max_scroll()) != current
}

/// QXSCR1: `region_takes_wheel` for the x axis (`dx` positive = left, winit's sign).
pub(crate) fn region_takes_wheel_x(info: &ScrollInfo, offsets: &HashMap<usize, f32>, dx: f32) -> bool {
    let current = offsets.get(&crate::layout::xk(info.id)).copied().unwrap_or(0.0);
    info.user_x && (current - dx).clamp(0.0, info.max_scroll_x()) != current
}

/// QXSCR1: a wheel event's `(dx, dy)` in px (winit's signs: positive = content moves right/down).
/// Shift turns a purely vertical wheel into a horizontal one (the common mouse convention); a
/// horizontal wheel or touchpad already reports `dx`.
pub(crate) fn wheel_axes(dx: f32, dy: f32, shift: bool) -> (f32, f32) {
    if shift && dx == 0.0 { (dy, 0.0) } else { (dx, dy) }
}

/// QSMOOTH1 D5: one frame-rate-independent exponential easing step of `offset` toward `target`
/// after `dt_ms` (capped at 50 ms so a stalled frame does not teleport). Returns the new offset and
/// whether the scroll settled (within 0.5 px: snapped to `target`). `tau_ms <= 0` jumps at once.
pub(crate) fn scroll_ease(offset: f32, target: f32, dt_ms: f32, tau_ms: f32) -> (f32, bool) {
    if tau_ms <= 0.0 {
        return (target, true);
    }
    let next = offset + (target - offset) * (1.0 - (-dt_ms.clamp(0.0, 50.0) / tau_ms).exp());
    if (target - next).abs() < 0.5 { (target, true) } else { (next, false) }
}

/// QYSCR1: how far `Action::PageDown`/`PageUp` scroll for a viewport `view_h`
/// px tall: a viewport less one 40 px line, so the last line stays in view.
/// QKEYS1: the viewport the page sees for a window of `win` px: the window minus the status bar
/// (every place that used the window size as the viewport uses this).
pub(crate) fn page_viewport(win: (u32, u32)) -> (u32, u32) {
    (win.0, win.1.saturating_sub(crate::config::STATUS_BAR_H).max(1))
}

/// QSB2b: `page_viewport` while the window's horizontal bar shows (`hbar`): it takes `TRACK_W`
/// px of the page's height, above the status bar.
pub(crate) fn page_viewport_hb(win: (u32, u32), hbar: bool) -> (u32, u32) {
    let (w, h) = page_viewport(win);
    (w, if hbar { h.saturating_sub(crate::config::TRACK_W as u32).max(1) } else { h })
}

/// QKEYS1/QKEYS2: the status bar's mode name.
pub(crate) fn mode_name(mode: KeyMode) -> &'static str {
    match mode {
        KeyMode::Input => "INPUT",
        KeyMode::Normal => "NORMAL",
    }
}

pub(crate) fn page_step(view_h: f32) -> f32 {
    (view_h - crate::config::PAGE_OVERLAP_PX).max(crate::config::PAGE_MIN_PX)
}

/// The pending-key state machine for the two-key sequences of `config::KEY_SEQUENCES` (`gg`,
/// `yy`; KEYS.md T10-T12, T54-T56): given the pending first key (if any) and the key just
/// pressed, returns the action to run (if any), the new pending first key, and whether the key
/// was consumed (so the caller must not also run `resolve_binding` on it). A first key arms; its
/// second key fires the row; every other key (Esc included) clears the pending state and is
/// handled as in NORMAL, so it may itself arm (`g` `y` leaves `y` pending, `y` `g` leaves `g`).
pub(crate) fn resolve_sequence(
    pending: Option<&'static str>,
    key: &Key,
    mods: ModifiersState,
) -> (Option<Action>, Option<&'static str>, bool) {
    use crate::config::KEY_SEQUENCES;
    let Key::Character(s) = key else { return (None, None, false) };
    if mods.control_key() || mods.alt_key() {
        return (None, None, false);
    }
    if let Some(first) = pending
        && let Some(row) = KEY_SEQUENCES.iter().find(|r| r.first == first && r.second == s.as_str())
    {
        return (Some(row.action), None, true);
    }
    match KEY_SEQUENCES.iter().find(|r| r.first == s.as_str()) {
        Some(row) => (None, Some(row.first), true),
        None => (None, None, false),
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

pub(crate) const COMMAND_NAMES: &[&str] = &["engine", "go", "help", "historyBack", "historyFront", "let", "q", "quit", "refresh", "set"];

// `SET_ARGS` and `COMPLETERS` (what the ghost completes) live in `config.rs`; the pure ghost
// logic is in `completion.rs` (QCOMP1).

/// The byte offset the token ending at `cursor` starts at: the last whitespace position before
/// `cursor`, plus one, or `0` if there is none. `rfind` only returns char-boundary indices.
pub(crate) fn token_start_before(text: &str, cursor: usize) -> usize {
    text[..cursor].rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0)
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
    /// `text` would silently desync `paint_status_bar`'s own single-
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
    /// QINC1b: the App's paragraph-id counter; the worker never hands out an id below it.
    pub(crate) id_base: usize,
    /// QINC1j: lay the tree out as the incremental engine does and return it for adoption.
    pub(crate) engine: Option<crate::incr_box::WorkerJob>,
}

pub(crate) struct App {
    pub(crate) document: Handle,
    pub(crate) decoder: Option<Utf8LossyDecoder<Parser<RcDom>>>,
    pub(crate) window: Option<Rc<Window>>,
    pub(crate) surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    pub(crate) font_system: FontSystem,
    /// QFF1a: `font_system`'s position in the web-font log (`webfonts::sync`).
    pub(crate) font_sync: crate::webfonts::FontSync,
    pub(crate) swash_cache: SwashCache,
    pub(crate) scroll_offset: f32,
    /// QSMOOTH1: where the eased viewport scroll is heading (`None` = not animating). `scroll_offset`
    /// stays "what is painted now"; `redraw` steps it toward this (`scroll_ease_step`).
    pub(crate) scroll_target: Option<f32>,
    /// QSMOOTH1: when the animation was last stepped (for the frame-rate-independent `dt`).
    pub(crate) scroll_anim_last: std::time::Instant,
    /// QSMOOTH1: the easing time constant, ms (`config::SMOOTH_SCROLL_TAU_MS`); 0 = off. Unit tests start with 0.
    pub(crate) smooth_scroll_tau_ms: f32,
    /// QXSCR1: the window's horizontal scroll (document x at the viewport's left edge). Clamped
    /// to `0..=doc_scroll_width - viewport width` by `redraw`, like `scroll_offset`.
    pub(crate) scroll_x: f32,
    /// Cached layout from the last `document`/width combination. Scrolling
    /// alone never invalidates this — relaying out and reshaping the whole
    /// page (expensive: real measurements showed full-document shaping can
    /// take seconds under full bidi/complex-script shaping, worse on large
    /// or RTL-heavy pages) on every mouse-wheel tick is what made scrolling
    /// feel laggy.
    /// QINC1j: shared with the incremental engine's kept tree when frames use it (a view, never
    /// copied); otherwise the App's own.
    pub(crate) cached: Option<Rc<Laid>>,
    /// QINC1j: version of the engine tree `cached` shows (0: not the engine's).
    pub(crate) live_ver: u64,
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
    /// QINC1b (R60 4.4): next unused paragraph id; never restarts (the worker request carries it,
    /// the result returns the worker's next value).
    pub(crate) next_para_id: usize,
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
    /// QM1k1n6a: newest not-yet-processed `CursorMoved` position.
    pub(crate) pending_pointer_move: Option<(f32, f32)>,
    /// QM1k1n6b: when the last input event arrived (sizes the timer slice).
    pub(crate) last_input: Option<std::time::Instant>,
    /// QM1k1n6a: pointer moves actually processed (test counter).
    pub(crate) pointer_moves_processed: u64,
    /// Document-space position at the most recent `MouseInput`
    /// `Pressed(Left)`, if any — compared against `last_mouse_pos` at
    /// the matching `Released(Left)` to tell a real click (navigates)
    /// apart from a drag (scratchy has no text selection yet, but a
    /// careless same-event binding would still misfire once it exists —
    /// see `is_click`'s own docs). Cleared back to `None` on release
    /// regardless of outcome.
    pub(crate) mouse_down_pos: Option<(f32, f32)>,
    /// QCUR1: position of the last `mousemove` handed to the page (for `movementX/Y`).
    pub(crate) last_move_dispatched: Option<(f32, f32)>,
    pub(crate) last_move_dispatch_at: Option<std::time::Instant>,
    pub(crate) move_dispatch_gap: std::time::Duration,
    /// QCUR1: when the pointer last moved (any move), a throttled move's position
    /// waiting for its trailing dispatch, and when a video frame was last presented.
    pub(crate) last_move_seen: Option<std::time::Instant>,
    pub(crate) move_trailing: Option<(f32, f32)>,
    pub(crate) last_video_frame: Option<std::time::Instant>,
    /// QMEM3d: once-per-idle-period malloc_trim gate.
    pub(crate) idle_trim: crate::mem_trim::IdleTrim,
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
    /// Q33b2: the window size last pushed into the page's engine.
    pub(crate) js_viewport: Option<(u32, u32)>,
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
    /// QEL1: app-clock ms of the next animation event/change (`anim_tick`'s answer); `-inf` right
    /// after a build (new animations may have started), `None` when nothing is scheduled.
    pub(crate) anim_wake: Option<f64>,
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
    #[cfg_attr(test, allow(dead_code))] // tests shape inline (`shape_inline`)
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
    /// QCOMP3: the load in flight is a visit (link, `:go`, page script): record it once it loaded.
    pub(crate) visit_pending: bool,
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
    /// QY53e2: `js_geom::render_stamp()` taken when the tree of the current `layout_generation` was built.
    build_stamp: crate::js_geom::RenderStamp,
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
    /// QY54b1: the stylesheet links `extra_css` is built from (script-inserted,
    /// removed or changed links rebuild it).
    pub(crate) link_sheets: crate::link_css::LinkSheets,
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
    /// QKEYS2: who gets the keys (see `KeyMode`). Starts in `config::START_MODE` (Normal; QKEYS5).
    pub(crate) key_mode: KeyMode,
    /// QKEYS2: the Esc Esc recogniser (only advanced in `Input`).
    pub(crate) esc_state: EscState,
    /// QKEYS2 (T34): keys whose press was NOT delivered to the page (the `i` of T6, the second
    /// Esc of T2); their release is dropped too, so the page never sees a `keyup` without a `keydown`.
    pub(crate) undelivered_presses: Vec<Key>,
    /// own docs. Only `Command` while `key_mode` is `Normal`; an `i` or a click into a text
    /// control/select closes it (`set_key_mode`).
    pub(crate) vim_mode: VimMode,
    /// A bare `g` was just pressed in Normal mode; the next `g` completes `gg`.
    pub(crate) pending_seq: Option<&'static str>,
    /// QKEYS1: the last message shown in the status bar (D8); empty = none.
    pub(crate) status_message: String,
    /// QSEARCH1: the kept query, the matches and the current one.
    pub(crate) search: crate::search::SearchState,
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
    /// QCOMP1: the `:` line's ghost suggestion (`completion.rs`); `None` = no ghost (also after
    /// Delete, until the next edit). Always `None` outside the `:` line.
    pub(crate) ghost: Option<Ghost>,
    /// QCOMP4: what the ghost sources read once per `:` line (the ranked history); new each time the line opens.
    pub(crate) complete_cache: CompleteCache,
    /// Set by `dispatch_command`'s own `:q`/`:quit` handler — only when
    /// `pre_help_page` is `None`, i.e. there's no page to fall back to
    /// (see that field's own docs for the vim-"closing the last buffer
    /// quits" half of this). Checked right after `handle_key_event`
    /// returns from `WindowEvent::KeyboardInput` (`window_event`'s own
    /// match arm) to actually call `ActiveEventLoop::exit()` —
    /// `dispatch_command` has no `&ActiveEventLoop` of its own to call
    /// that with directly (`resumed`'s `&ActiveEventLoop` can't just be
    /// stashed on `App` once and reused later; the live one is
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
    /// QSB1: a scrollbar thumb drag in progress.
    pub(crate) sb_drag: Option<SbDrag>,
    /// QEL7: when the scroll steps last ran (`SCROLL_STEP_GAP`).
    pub(crate) last_scroll_step: Option<std::time::Instant>,
    /// QSB2b: the window's horizontal bar shows (the page viewport is `TRACK_W` shorter).
    pub(crate) hbar: bool,
    /// QSB2: a track press being held (page repeat) and the hovered thumb's bar id.
    pub(crate) sb_hold: Option<SbHold>,
    pub(crate) sb_hover: Option<usize>,
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
    /// QOBS5: `SCRATCHY_FRAME_STATS` aggregator; `None` (the default) costs one test per hook.
    pub(crate) frame_stats: Option<crate::frame_stats::FrameStats>,
    /// QY54a2 (R57): the `wl-drain` thread reading the Wayland socket while the main thread is busy;
    /// started in `resumed`, joined in `exiting`. `None` off Wayland or with `SCRATCHY_WL_DRAIN=0`.
    pub(crate) wl_drain: Option<crate::wl_drain::WlDrain>,
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

/// QOBS5: reads `SCRATCHY_FRAME_STATS`; when on, the perf recorder runs in window mode.
fn new_frame_stats() -> Option<crate::frame_stats::FrameStats> {
    let fs = crate::frame_stats::FrameStats::from_env()?;
    crate::perf::start_window();
    Some(fs)
}

/// QKEYS1b: the status bar URL from the pieces of `App` (see `App::status_url`).
fn status_url(history: &[Url], index: usize, base: Option<&Url>) -> String {
    current_url(history, index, base).unwrap_or_else(|| "(stdin)".to_string())
}

/// The URL the status bar shows, or None for a stdin start page (QREG1: register `%`).
fn current_url(history: &[Url], index: usize, base: Option<&Url>) -> Option<String> {
    history.get(index).or(base).map(|u| u.to_string())
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
    pub(crate) link_sheets: crate::link_css::LinkSheets,
    /// QSD4: `document.adoptedStyleSheets` CSS, appended after `extra_css`.
    pub(crate) adopted_css: String,
    pub(crate) image_cache: HashMap<String, Arc<DecodedImage>>,
    pub(crate) scroll_offset: f32,
    pub(crate) scroll_x: f32,
    pub(crate) scroll_offsets: HashMap<usize, f32>,
    /// QB6: the page's JS engine, suspended while help is shown (its
    /// timers, listeners and dirty flag must not touch the help page).
    pub(crate) js: Option<JsEngine>,
}

/// QY53o3c: the shaping worker's memo keeps what any of the last `WORKER_MEMO_KEEP` finished
/// builds used (a hover toggled back, a JS write undone, finds its shape again); past
/// `WORKER_MEMO_CAP` entries after that sweep it starts over (the JS thread's cap).
const WORKER_MEMO_KEEP: u32 = 8;
const WORKER_MEMO_CAP: usize = 32768;

/// QCUR1: minimum gap between `mousemove`/`pointermove` dispatches to the page (no button down).
pub(crate) const MOVE_DISPATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// QCUR1: minimum gap between motion dispatches while a video plays (user decision).
pub(crate) const MOVE_GAP_PLAYING: std::time::Duration = std::time::Duration::from_millis(150);

/// QCUR1: a move after this long without any move is dispatched immediately.
pub(crate) const MOVE_IDLE: std::time::Duration = std::time::Duration::from_secs(1);

/// QEL7: user-scroll events are coalesced to one per target per frame: the scroll steps run at most this
/// often when only they asked for an update.
const SCROLL_STEP_GAP: std::time::Duration = std::time::Duration::from_millis(16);

/// QCUR1: upper bound of the adaptive gap (3x the last dispatch's handler time).
pub(crate) const MOVE_DISPATCH_MAX_GAP: std::time::Duration = std::time::Duration::from_millis(500);

/// QCUR1: bit order of `JsEngine::pointer_listener_mask` (js.rs `PTR_TYPES`).
const PTR_BITS: [&str; 9] =
    ["mousemove", "pointermove", "mouseenter", "mouseleave", "pointerover", "pointerout", "pointerenter", "pointerleave", "wheel"];

/// QSB1: what a thumb drag scrolls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SbTarget {
    Viewport,
    /// `ScrollInfo::id`, `ScrollInfo::node`.
    Region(usize, usize),
}

/// QSB2: what `scrollbar_find` found under the pointer.
struct SbFound {
    target: SbTarget,
    axis: crate::scrollbar::Axis,
    hit: crate::scrollbar::BarHit,
    cur: f32,
    max: f32,
    ratio: f32,
    page: f32,
}

/// QSB2: a track press being held: the side pressed and when the next page step is due.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SbHold {
    pub(crate) axis: crate::scrollbar::Axis,
    pub(crate) dir: crate::scrollbar::BarHit,
    pub(crate) next: std::time::Instant,
}

/// QSB1: a thumb drag: the scroll position at the press and the pointer y it
/// started at; each move is `start_scroll + dy * ratio`, clamped to `0..=max`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SbDrag {
    pub(crate) target: SbTarget,
    /// QSB2b: the axis the bar scrolls; `start_ptr` is the pointer along it.
    pub(crate) axis: crate::scrollbar::Axis,
    pub(crate) start_ptr: f32,
    pub(crate) start_scroll: f32,
    pub(crate) ratio: f32,
    pub(crate) max: f32,
}

impl App {
    /// QM1k1n6a: remember the newest pointer position; the hit test, hover
    /// and `mouseover`/`mouseout` work happens once in `flush_pointer_move`.
    pub(crate) fn queue_pointer_move(&mut self, pos: (f32, f32)) {
        self.pending_pointer_move = Some(pos);
        self.last_input = Some(std::time::Instant::now());
    }

    /// QM1k1n6b: input arrived within the last 100 ms.
    pub(crate) fn input_recent(&self) -> bool {
        self.pending_pointer_move.is_some() || self.last_input.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(100))
    }

    /// QM1k1n6a: process the queued pointer move, if any (one hit test no
    /// matter how many moves were queued). Called from `about_to_wait` and
    /// before any button, wheel or key event, so those see the latest position.
    pub(crate) fn flush_pointer_move(&mut self) {
        if let Some(pos) = self.pending_pointer_move.take() {
            self.pointer_moves_processed += 1;
            self.process_pointer_move(pos);
        }
    }

    fn process_pointer_move(&mut self, position: (f32, f32)) {
        // Remembered in window/screen space, same form this event
        // itself reports — `MouseWheel` (just below) needs it to
        // pick the right scroll region, since that event carries
        // no position of its own at all.
        self.last_mouse_pos = Some(position);
        self.last_pointer_move = Some(std::time::Instant::now());
        // `position` is window/screen space; `Laid.x`/`.y` are
        // document space (scroll isn't applied until paint time),
        // so the same `+ scroll_offset` adjustment `paint` itself
        // makes (in reverse) is needed here too.
        if self.sb_drag.is_some() {
            self.scrollbar_drag_move(position);
            return;
        }
        self.scrollbar_hover_update(position);
        self.text_mouse_drag(position);
        if self.video.drag.is_some() {
            self.controls_drag_move(position, false);
            return;
        }
        self.controls_pointer(position);
        if self.select_popup_hover(position) {
            return;
        }
        let hovered = self.element_at(position);
        self.update_hover(hovered);
        self.js_pointer_move(position, hovered);
        self.refresh_cursor();
    }

    /// QKEYS1: the viewport the page sees (window minus the status bar).
    pub(crate) fn page_size(&self) -> (u32, u32) {
        page_viewport_hb(self.surface_size, self.hbar)
    }

    /// QKEYS1: publishes the page viewport (window minus the status bar) to new engines and to the
    /// running one (`innerHeight`, `visualViewport`, `resize`).
    /// QEL1: publishes the window size for engines created from now on (every loop turn; the
    /// `resize` event itself is the rendering update's step, `sync_js_viewport`).
    pub(crate) fn publish_viewport(&mut self) {
        if self.surface_size.0 != 0 {
            let vp = self.page_size();
            crate::js::set_window_viewport(vp.0, vp.1);
        }
    }

    /// QEL1: the page's viewport differs from the window's (a `resize` is owed).
    pub(crate) fn viewport_changed(&self) -> bool {
        self.js.is_some() && self.surface_size.0 != 0 && self.js_viewport != Some(self.page_size())
    }

    /// QEL1: runs the rendering update (`event_loop::update_the_rendering`) when its frame is due:
    /// an animation frame callback waits and its 60 Hz frame time has come, a `resize` is owed, or an
    /// animation event/change is due. Returns true when it ran.
    pub(crate) fn rendering_update_if_due(&mut self) -> bool {
        let anim_due = !self.anims_idle() && self.anim_wake.is_some_and(|t| t <= app_clock_ms());
        let raf_ts = self.js.as_ref().and_then(|js| {
            let due = js.raf_due_ms()?;
            let now = js.now_ms();
            (due <= now).then_some(now)
        });
        if !(anim_due || raf_ts.is_some() || self.viewport_changed() || self.scroll_step_due()) {
            return false;
        }
        crate::event_loop::update_the_rendering(self, raf_ts);
        true
    }

    /// QEL7: a trusted, cancelable `wheel` event (deltas in pixels: DOM `deltaY` > 0 scrolls down, winit's `dy`
    /// is the opposite) at the element under the pointer (the body when none). True when the page called
    /// `preventDefault`. Free when no page listener exists.
    fn dispatch_wheel(&mut self, dx: f32, dy: f32) -> bool {
        let wheel_bit = PTR_BITS.iter().position(|t| *t == "wheel").map_or(0, |i| 1u32 << i);
        if self.js.as_ref().is_none_or(|js| js.pointer_listener_mask() & wheel_bit == 0) {
            return false;
        }
        let pos = self.last_mouse_pos.unwrap_or((0.0, 0.0));
        let node = self.element_at(pos);
        let target = (node != 0).then(|| find_node_by_id(&self.document, node)).flatten();
        let mut init = self.pointer_init(pos, &None, true, false);
        init.push(("deltaX", EventField::Num(f64::from(-dx))));
        init.push(("deltaY", EventField::Num(f64::from(-dy))));
        init.push(("deltaZ", EventField::Num(0.0)));
        init.push(("deltaMode", EventField::Num(0.0)));
        !self.js_dispatch(target.as_ref(), "wheel", &init)
    }

    /// QEL7: the window is still scrolling (easing to a target, or a scrollbar thumb is held): `scrollend`
    /// waits.
    pub(crate) fn scroll_busy(&self) -> bool {
        self.scroll_target.is_some() || self.sb_drag.is_some()
    }

    /// QEL7: scroll events the page owes (`scroll` queued, or a `scrollend` that can fire now) and the
    /// frame interval since the last scroll steps has passed: user scrolling is coalesced to one event per
    /// target per frame.
    fn scroll_step_due(&self) -> bool {
        self.scroll_owed() && self.last_scroll_step.is_none_or(|t| t.elapsed() >= SCROLL_STEP_GAP)
    }

    fn scroll_owed(&self) -> bool {
        let bits = self.js.as_ref().map_or(0, |js| js.qel7_bits());
        bits & 1 != 0 || (bits & 2 != 0 && !self.scroll_busy())
    }

    /// QEL7: when the scroll steps are next due (`None` = nothing owed), for the event loop's wake-up.
    fn scroll_wake(&self) -> Option<std::time::Instant> {
        self.scroll_owed().then(|| self.last_scroll_step.map_or_else(std::time::Instant::now, |t| t + SCROLL_STEP_GAP))
    }

    pub(crate) fn sync_js_viewport(&mut self) {
        if self.surface_size.0 == 0 {
            return;
        }
        let vp = self.page_size();
        crate::js::set_window_viewport(vp.0, vp.1);
        if let Some(js) = &self.js
            && self.js_viewport != Some(vp)
        {
            self.js_viewport = Some(vp);
            js.set_viewport(vp.0, vp.1);
        }
    }

    /// The window cursor for the element under the last pointer position. Also run
    /// after each layout install (QCUR1): the page reacts to `mousemove` by clearing
    /// `cursor: none` (YouTube's `ytp-autohide`), which only the NEXT layout shows.
    pub(crate) fn refresh_cursor(&mut self) {
        let Some(position) = self.last_mouse_pos else { return };
        if let (Some(laid), Some(window)) = (&self.cached, &self.window) {
            let doc_x = position.0 + self.scroll_x;
            let doc_y = position.1 + self.scroll_offset;
            let (style, _href) = crate::paint::with_scroll_x(self.scroll_x, || {
                hit_test_cursor(
                    laid, doc_x, doc_y, &self.shape_cache, &self.scroll_offsets, self.scroll_offset,
                    self.page_size().1 as f32,
                )
            });
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
            font_sync: crate::webfonts::FontSync::default(),
            swash_cache: SwashCache::new(),
            scroll_offset: 0.0,
            scroll_target: None,
            scroll_anim_last: std::time::Instant::now(),
            smooth_scroll_tau_ms: if cfg!(test) { 0.0 } else { crate::config::SMOOTH_SCROLL_TAU_MS },
            scroll_x: 0.0,
            cached: None,
            live_ver: 0,
            cached_width: 0,
            cached_height: 0,
            layout_dirty: true,
            shape_cache: ShapeCache::new(),
            next_para_id: 0,
            surface_size: (0, 0),
            pending_width: None,
            last_cursor: Some(CursorIcon::Default),
            scroll_offsets: HashMap::new(),
            node_scroll: HashMap::new(),
            node_scroll_dirty: false,
            last_mouse_pos: None,
            pending_pointer_move: None,
            last_input: None,
            pointer_moves_processed: 0,
            mouse_down_pos: None,
            last_move_dispatched: None,
            last_move_dispatch_at: None,
            move_dispatch_gap: MOVE_DISPATCH_INTERVAL,
            last_move_seen: None,
            move_trailing: None,
            last_video_frame: None,
            idle_trim: Default::default(),
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
            js_viewport: None,
            layout_in_flight: false,
            reads_viewport_h: true,
            hover_keys: HoverKeys { any: true, ..Default::default() },
            last_hover_restyle: None,
            last_wheel: None,
            last_pointer_move: None,
            last_build_time: None,
            anim: AnimTracker::default(),
            anim_wake: None,
            hover_flush_scheduled: false,
            image_relayout_since: None,
            image_flush_scheduled: false,
            proxy,
            shaping_requests,
            base_url,
            nav_initiator: None,
            pending_post: None,
            visit_pending: false,
            history: Vec::new(),
            history_index: 0,
            history_doc: Vec::new(),
            doc_gen: 0,
            nav_generation: 0,
            layout_generation: 0,
            build_stamp: (0, (0, 0)),
            js: None,
            socket_path,
            net: NetScheduler::new(&RequestPolicy::default()),
            parked_net: Vec::new(),
            extra_css: String::new(),
            link_sheets: Default::default(),
            adopted_css: String::new(),
            image_cache: HashMap::new(),
            css_images_requested: HashSet::new(),
            late_images_requested: 0,
            late_image_errors: Vec::new(),
            key_mode: crate::config::START_MODE,
            esc_state: EscState::Idle,
            undelivered_presses: Vec::new(),
            vim_mode: VimMode::Normal,
            status_message: String::new(),
            pending_seq: None,
            command_line: TextEditBuffer::default(),
            search: crate::search::SearchState::default(),
            modifiers: ModifiersState::empty(),
            ghost: None,
            complete_cache: CompleteCache::new(None),
            should_quit: false,
            select_popup: None,
            swallow_release: false,
            sb_drag: None,
            last_scroll_step: None,
            hbar: false,
            sb_hold: None,
            sb_hover: None,
            select_ahead: TypeAhead::default(),
            pre_help_page: None,
            video: AppVideo::default(),
            frame_stats: new_frame_stats(),
            wl_drain: None,
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
            font_sync: crate::webfonts::FontSync::default(),
            swash_cache: SwashCache::new(),
            scroll_offset: 0.0,
            scroll_target: None,
            scroll_anim_last: std::time::Instant::now(),
            smooth_scroll_tau_ms: if cfg!(test) { 0.0 } else { crate::config::SMOOTH_SCROLL_TAU_MS },
            scroll_x: 0.0,
            cached: None,
            live_ver: 0,
            cached_width: 0,
            cached_height: 0,
            layout_dirty: true,
            shape_cache: ShapeCache::new(),
            next_para_id: 0,
            surface_size: (0, 0),
            pending_width: None,
            last_cursor: Some(CursorIcon::Default),
            scroll_offsets: HashMap::new(),
            node_scroll: HashMap::new(),
            node_scroll_dirty: false,
            last_mouse_pos: None,
            pending_pointer_move: None,
            last_input: None,
            pointer_moves_processed: 0,
            mouse_down_pos: None,
            last_move_dispatched: None,
            last_move_dispatch_at: None,
            move_dispatch_gap: MOVE_DISPATCH_INTERVAL,
            last_move_seen: None,
            move_trailing: None,
            last_video_frame: None,
            idle_trim: Default::default(),
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
            js_viewport: None,
            layout_in_flight: false,
            reads_viewport_h: true,
            hover_keys: HoverKeys { any: true, ..Default::default() },
            last_hover_restyle: None,
            last_wheel: None,
            last_pointer_move: None,
            last_build_time: None,
            anim: AnimTracker::default(),
            anim_wake: None,
            hover_flush_scheduled: false,
            image_relayout_since: None,
            image_flush_scheduled: false,
            proxy,
            shaping_requests,
            base_url: None,
            nav_initiator: None,
            pending_post: None,
            visit_pending: false,
            history: Vec::new(),
            history_index: 0,
            history_doc: Vec::new(),
            doc_gen: 0,
            nav_generation: 0,
            layout_generation: 0,
            build_stamp: (0, (0, 0)),
            js: None,
            socket_path,
            net: NetScheduler::new(&RequestPolicy::default()),
            parked_net: Vec::new(),
            extra_css: String::new(),
            link_sheets: Default::default(),
            adopted_css: String::new(),
            image_cache: HashMap::new(),
            css_images_requested: HashSet::new(),
            late_images_requested: 0,
            late_image_errors: Vec::new(),
            key_mode: crate::config::START_MODE,
            esc_state: EscState::Idle,
            undelivered_presses: Vec::new(),
            vim_mode: VimMode::Normal,
            status_message: String::new(),
            pending_seq: None,
            command_line: TextEditBuffer::default(),
            search: crate::search::SearchState::default(),
            modifiers: ModifiersState::empty(),
            ghost: None,
            complete_cache: CompleteCache::new(None),
            should_quit: false,
            select_popup: None,
            swallow_release: false,
            sb_drag: None,
            last_scroll_step: None,
            hbar: false,
            sb_hold: None,
            sb_hover: None,
            select_ahead: TypeAhead::default(),
            pre_help_page: None,
            video: AppVideo::default(),
            frame_stats: new_frame_stats(),
            wl_drain: None,
        }
    }

    pub(crate) fn redraw(&mut self) {
        crate::paint::SB_HOT.with(|h| h.set(self.scrollbar_hot()));
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        if let Some(fs) = &mut self.frame_stats {
            fs.redraw_start(std::time::Instant::now());
        }

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
        // QKEYS1: the page's viewport is the window minus the status bar.
        let (_, page_h) = page_viewport_hb((width, height), self.hbar);
        let width_matches_pending = self.pending_width == Some(width);
        self.pending_width = Some(width);
        // R4: a height-only change only needs a rebuild if the layout
        // actually read the viewport height; otherwise just track it.
        if self.cached_width == width && self.cached_height != page_h && self.cached.is_some() && !self.reads_viewport_h {
            self.cached_height = page_h;
        }
        let height_rebuild = self.cached_width == width && self.cached_height != page_h && self.cached.is_some();
        if self.cached.is_some() && (self.cached_width, self.cached_height) != (width, page_h) {
            crate::incr_frame::note_ambient(crate::incr_log::Ambient::Viewport);
        }
        let need_rebuild = self.layout_dirty || self.cached_width != width || self.cached_height != page_h;

        let rebuild_now = need_rebuild && width_matches_pending;
        if rebuild_now {
            self.rebuild_layout(width, page_h, height_rebuild);
        }
        // QSB2b: the window's horizontal bar shows or goes: the page viewport changes by `TRACK_W`
        // px of height (layout, `innerHeight`), so redo this frame.
        if self.cached.is_some() && Self::want_hbar(&self.cached, width) != self.hbar {
            self.hbar = !self.hbar;
            self.request_redraw_now();
            return;
        }
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        if need_rebuild && !rebuild_now {
            window.request_redraw();
        }

        let mut pixels = match surface.buffer_mut() {
            Ok(p) => p,
            Err(e) => {
                log_redraw_error_once("get the frame buffer", &e);
                return;
            }
        };
        let paint_started = self.frame_stats.is_some().then(std::time::Instant::now);
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
            let max_scroll = (laid.h - page_h as f32).max(0.0);
            if let Some(target) = self.scroll_target {
                let now = std::time::Instant::now();
                let dt = now.duration_since(self.scroll_anim_last).as_secs_f32() * 1000.0;
                self.scroll_anim_last = now;
                let target = target.clamp(0.0, max_scroll);
                let (offset, done) = scroll_ease(self.scroll_offset.clamp(0.0, max_scroll), target, dt, self.smooth_scroll_tau_ms);
                self.scroll_offset = offset;
                self.scroll_target = if done { None } else { Some(target) };
            }
            self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll);
            let max_scroll_x = (crate::layout::doc_scroll_width(laid) - width as f32).max(0.0);
            self.scroll_x = self.scroll_x.clamp(0.0, max_scroll_x);

            crate::paint::with_scroll_x(self.scroll_x, || {
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
                )
            });
            // Q31b: default focus ring for keyboard focus, unless the author
            // already styled an outline on the element.
            if let (Some(f), true) = (self.interaction.focus, self.interaction.focus_visible) {
                let geom = node_geometry(laid, &self.shape_cache, f);
                if !geom.outline {
                    for &(x, y, w, h) in &geom.rects {
                        paint_focus_ring(&mut pixels, width, height, x - self.scroll_x, y - self.scroll_offset, w, h);
                    }
                }
            }
        }

        // QSB1: the window's own scrollbar (overlay, same look as a region's).
        let vbar = Self::viewport_bar(&self.cached, (width, page_h));
        if let Some((rect, content_h, client_h)) = vbar {
            let info = crate::layout::ScrollInfo {
                id: crate::scrollbar::VIEWPORT_ID, content_h, client_h, node: 0, user: true, user_x: false, content_w: 0.0, client_w: 0.0,
            };
            crate::paint::paint_scrollbar(&info, self.scroll_offset, false, &mut pixels, width, height, rect.0, rect.1, rect.2, rect.3, None);
        }
        // QSB2b: the horizontal bar in the strip above the status bar (canvas colour under it),
        // with the corner square when the vertical one shows too.
        if let Some((rect, content_w, client_w)) = Self::viewport_bar_x(&self.cached, (width, page_h)) {
            let canvas = self.cached.as_ref().and_then(|l| l.style.background_color);
            for row in page_h..(page_h + crate::config::TRACK_W as u32).min(height) {
                for x in 0..width {
                    let i = (row * width + x) as usize;
                    pixels[i] = 0x00FFFFFF;
                    if let Some(c) = canvas {
                        pixels[i] = crate::paint::blend(pixels[i], c);
                    }
                }
            }
            crate::paint::paint_viewport_hbar(content_w, client_w, self.scroll_x, vbar.is_some(), &mut pixels, width, height, rect);
        }

        // QSEARCH1 (S3): search highlights, chrome over the page (below the status bar).
        if self.search.highlight {
            for (i, m) in self.search.matches.iter().enumerate() {
                let (r, g, b, a) = if self.search.current == Some(i) { crate::config::SEARCH_CURRENT_RGBA } else { crate::config::SEARCH_MATCH_RGBA };
                // QSEARCH2: matches inside scroll regions follow the regions' offsets and are
                // clipped to the regions' boxes.
                for (x, y, w, h) in m.visible_rects(&self.scroll_offsets) {
                    let (x, y) = (x - self.scroll_x, y - self.scroll_offset);
                    if y + h < 0.0 || y > page_h as f32 {
                        continue;
                    }
                    let clip = Some((0.0, 0.0, width as f32, page_h as f32));
                    crate::paint::fill_rect(&mut pixels, width, height, x, y, w, h, cosmic_text::Color::rgba(r, g, b, a), clip);
                }
            }
        }

        // Q30g: the open select dropdown is the top layer, above the page.
        if let (Some(p), Some(laid)) = (&self.select_popup, self.cached.as_ref())
            && let Some(view) = popup_view(&p.st, &self.document, laid, &self.shape_cache, (self.scroll_x, self.scroll_offset), (width as f32, page_h as f32))
        {
            paint_select_popup(&view, &p.st, &mut self.font_system, &mut self.swash_cache, &mut pixels, width, height);
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
        // QKEYS1: the status bar is the top layer (over a video that reaches into its strip).
        {
            let url = status_url(&self.history, self.history_index, self.base_url.as_ref());
            let ghost_text = match (&self.ghost, self.vim_mode) {
                (Some(g), VimMode::Command) => g.rest(&self.command_line.text),
                _ => String::new(),
            };
            let bar = crate::paint::StatusBar {
                mode: mode_name(self.key_mode),
                url: &url,
                message: &self.status_message,
                command: (self.vim_mode != VimMode::Normal).then_some(&self.command_line),
                prompt: if self.vim_mode == VimMode::Search { '/' } else { ':' },
                ghost: &ghost_text,
            };
            crate::paint::paint_status_bar(&bar, &mut self.font_system, &mut self.swash_cache, &mut pixels, width, height);
        }
        self.video.page_buf.clear();
        if !boxes.is_empty() {
            self.video.page_buf.extend_from_slice(&pixels);
            self.video.size = (width, height);
        }
        let targets: Vec<_> = boxes.iter().filter(|b| crate::media_video::wants_render(b.visible_pixels(), b.layered, false)).map(|b| (b.node, b.target())).collect();
        self.video.boxes = boxes;

        let present_started = std::time::Instant::now();
        if let (Some(fs), Some(t)) = (&mut self.frame_stats, paint_started) {
            fs.paint(present_started.saturating_duration_since(t));
        }
        window.pre_present_notify();
        if let Err(e) = pixels.present() {
            log_redraw_error_once("present the frame", &e);
        }
        if let Some(fs) = &mut self.frame_stats {
            let now = std::time::Instant::now();
            fs.present(now.saturating_duration_since(present_started));
            fs.presented(now);
            for &n in &laid {
                fs.video_painted(n);
            }
        }
        if video_on && let Some(js) = &self.js {
            js.media_set_targets(&targets);
            js.media_set_laid(&laid);
        }
    }

    /// The layout part of a frame (`redraw`'s rebuild branch; tests call it without a window).
    pub(crate) fn rebuild_layout(&mut self, width: u32, height: u32, height_rebuild: bool) {
        // QINC1i: script forced layouts through the engine use the images this frame lays out with.
        crate::incr_engine::sync_window_images(&self.document, &self.image_cache);
        self.rebuild_live(width, height, height_rebuild);
    }

    /// QINC1j (R60 sections 3, 9): a frame through the incremental engine.
    /// Incremental updates are shown at once; big ones go to the worker as before (the engine adopts
    /// the result); nothing to lay out (a forced layout already did it) shows the engine's tree.
    fn rebuild_live(&mut self, width: u32, height: u32, height_rebuild: bool) {
        use crate::incr_frame::Frame;
        set_app_clock(self.nav_generation);
        let build_started = std::time::Instant::now();
        self.build_stamp = crate::js_geom::render_stamp();
        // The engine updates its tree in place when the App's view of it is the only other handle.
        let shown = self.cached.take_if(|c| crate::incr_layout::owns(c)).is_some();
        let css = crate::images::with_adopted_css(&self.extra_css, &self.adopted_css).into_owned();
        // QINC1n: the URL engine log lines name (the status bar's).
        crate::incr_evidence::set_page(&self.document, || status_url(&self.history, self.history_index, self.base_url.as_ref()));
        let frame = crate::incr_frame::frame(&self.document, width as f32, height as f32, &css, &self.image_cache, &mut self.font_system);
        if !matches!(frame, Frame::Current) {
            self.last_build_time = Some(build_started.elapsed());
            if let Some(fs) = &mut self.frame_stats {
                fs.build(build_started.elapsed());
            }
            self.anim.after_build(self.nav_generation, take_animation_report(), document_clock_ms());
            self.anim_wake = Some(f64::NEG_INFINITY);
            fetch_wanted_css_images(&self.proxy, self.socket_path.as_deref(), self.base_url.as_ref(), &mut self.css_images_requested, self.nav_generation);
            let late = crate::images::take_wanted_img_srcs();
            fetch_late_images(&self.proxy, self.socket_path.as_deref(), self.base_url.as_ref(), &mut self.late_images_requested, &mut self.late_image_errors, self.nav_generation, late);
            if !self.keep_scroll_on_rebuild {
                self.scroll_offsets.clear();
            }
            self.keep_scroll_on_rebuild = false;
            self.video.ctl_attr.clear();
            self.state_rules |= state_rules_mask();
            self.reads_viewport_h |= viewport_height_read();
            self.hover_keys = HOVER_KEYS.with(|k| k.borrow().clone());
        }
        let restyle = (self.restyle_requested || height_rebuild || self.hold_old_frame) && (self.cached.is_some() || shown);
        self.restyle_requested = false;
        let reuse = restyle && self.cached_width == width;
        self.cached_width = width;
        self.cached_height = height;
        self.layout_dirty = false;
        // A worker result still in flight is older than this frame.
        self.layout_generation += 1;
        match frame {
            Frame::Current | Frame::Main => {
                self.hold_old_frame = false;
                match crate::incr_layout::view(if self.cached.is_none() && shown { self.live_ver } else { 0 }) {
                    Some(v) => {
                        let cache = if v.ver == self.live_ver && shown { std::mem::take(&mut self.shape_cache) } else { v.cache };
                        self.live_ver = v.ver;
                        self.install_laid(v.laid, cache, false);
                    }
                    None => self.layout_dirty = true,
                }
            }
            Frame::Fresh(laid, cache) => {
                self.hold_old_frame = false;
                self.live_ver = 0;
                self.install_laid(Rc::new(laid), cache, false);
            }
            Frame::Worker(tree, job) => {
                if shown && self.cached.is_none() {
                    // Keep painting the engine's (now stale) tree until the worker's arrives.
                    self.cached = crate::incr_layout::view(self.live_ver).map(|v| v.laid);
                }
                let restyle = restyle && self.cached.is_some();
                self.hold_old_frame = restyle;
                self.layout_in_flight = true;
                let id_base = self.next_para_id.max(crate::incr_layout::next_id());
                // Tests: no event loop delivers the worker's result; lay it out here, the same way.
                #[cfg(test)]
                {
                    let _ = reuse;
                    self.shape_inline(tree, width as f32, height as f32, id_base, job);
                }
                #[cfg(not(test))]
                Self::queue_shaping_request(&self.shaping_requests, tree, width as f32, height as f32, self.layout_generation, reuse, id_base, job);
            }
        }
    }

    /// Test stand-in for the shaping worker (no event loop delivers its result).
    #[cfg(test)]
    fn shape_inline(&mut self, tree: LayoutBox, w: f32, h: f32, id_base: usize, job: Option<crate::incr_box::WorkerJob>) {
        let (mut cache, mut next, mut inc) = (ShapeCache::new(), id_base, false);
        let far = std::time::Instant::now() + std::time::Duration::from_secs(3600);
        let (laid, adopt) = match job {
            Some(job) => {
                let (laid, root_h) = crate::incr_layout::worker_measure(&tree, &mut self.font_system, w, h, &mut next, &mut cache, far, &mut inc, &job.ver);
                (laid, Some(Box::new(crate::incr_frame::WorkerResult { tree, job, root_h, vw: w, vh: h })))
            }
            None => (measure_document(&tree, &mut self.font_system, w, h, &mut next, &mut cache, far, &mut inc), None),
        };
        self.next_para_id = self.next_para_id.max(next);
        let generation = self.layout_generation;
        self.handle_shape_result_job(generation, Box::new(laid), cache, false, adopt.map(|a| (a, next)));
    }

    /// QM1g1: an mpv frame is ready. Renders it and, when the page itself is
    /// unchanged, copies it into the video's visible rect only (through the
    /// overlay mask, so content above it stays) and presents just that damage.
    /// False = a full redraw is needed (the caller requests it).
    fn video_tick(&mut self) -> bool {
        let t0 = std::time::Instant::now();
        let r = self.video_tick_inner();
        if let Some(fs) = &mut self.frame_stats {
            fs.video_tick(t0.elapsed(), std::time::Duration::ZERO);
        }
        r
    }

    fn video_tick_inner(&mut self) -> bool {
        let Some(js) = &self.js else { return true };
        let t0 = std::time::Instant::now();
        let changed = js.media_render();
        if let Some(fs) = &mut self.frame_stats {
            fs.video_tick(std::time::Duration::ZERO, t0.elapsed());
        }
        if changed.is_empty() {
            return true;
        }
        if let Some(fs) = &mut self.frame_stats {
            for &n in &changed {
                fs.video_rendered(n);
            }
        }
        let (frames, _) = js.media_video_state();
        let (w, h) = self.surface_size;
        // QM1k1n6c: a pending relayout does not matter here. `page_buf` and `boxes`
        // are what the last full redraw painted and what is on screen; the frame goes
        // into that rect and the relayout's own redraw moves/resizes it afterwards.
        let ok = video_damage_ok(self.video.size, self.video.page_buf.len(), (w, h));
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
        let present_started = std::time::Instant::now();
        if let Err(e) = buf.present_with_damage(&rects) {
            log_redraw_error_once("present the video damage", &e);
        }
        if let Some(fs) = &mut self.frame_stats {
            let now = std::time::Instant::now();
            fs.present(now.saturating_duration_since(present_started));
            fs.damage_present();
            fs.presented(now);
            for &n in &changed {
                fs.video_painted(n);
            }
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
    #[allow(clippy::too_many_arguments)]
    #[cfg_attr(test, allow(dead_code))] // tests shape inline (`shape_inline`)
    pub(crate) fn queue_shaping_request(
        requests: &mpsc::Sender<ShapingRequest>, tree: LayoutBox, width: f32, height: f32, generation: u64,
        reuse_shaping: bool, id_base: usize, engine: Option<crate::incr_box::WorkerJob>,
    ) {
        let _ = requests.send(ShapingRequest { tree, width, height, generation, reuse_shaping, id_base, engine });
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
    /// stay exactly what `paint`/`paint_status_bar` already use to
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
        // QFF1a: the App's web-font registry (faces are replayed into this thread's font system).
        let webfonts = crate::webfonts::handle();
        let spawned = worker.spawn(move || {
            crate::webfonts::install(webfonts);
            let mut font_sync = crate::webfonts::FontSync::default();
            let mut font_system = new_font_system();
            SHAPE_MEMO_ENABLED.with(|m| m.set(true));
            // `rx.recv()` blocks here whenever there's no page waiting
            // to be shaped — this thread is otherwise entirely idle,
            // not spinning. `next` carries a request straight into the
            // next iteration when the inner loop below was interrupted
            // by one arriving mid-shape (see its own comment), instead
            // of going back through `rx.recv()` and losing it.
            let mut next = rx.recv();
            // QINC1b: persists across requests; swept after a finished layout.
            let mut shape_cache = ShapeCache::new();
            let mut worker_next_id = 0usize;
            // Ids the last finished layout references (what the App may still be showing).
            let mut last_live: std::collections::HashSet<usize> = std::collections::HashSet::new();
            while let Ok(mut request) = next {
                // Jump straight to the newest already-queued request,
                // if more than one piled up while this thread was busy
                // with something else (e.g. several resizes in a row) —
                // see this function's own docs.
                while let Ok(newer) = rx.try_recv() {
                    request = newer;
                }
                let ShapingRequest { tree, width, height, generation, reuse_shaping, id_base, engine } = request;
                // QFF1a: web faces that arrived (or went) since the last request; a change drops the
                // shape memo (`sync` did) and every cached shape.
                let fonts_changed = crate::webfonts::sync(&mut font_system, &mut font_sync);
                if !reuse_shaping || fonts_changed {
                    crate::layout::memo_clear();
                    // A new document: nothing in the cache can be live.
                    shape_cache.clear();
                    last_live.clear();
                }
                // Superseded/abandoned builds leave ids no layout uses; drop them here so a
                // storm of requests that never finish cannot grow the cache without bound.
                crate::layout::sweep_shape_cache(&mut shape_cache, &last_live);
                // QINC1b: ids never restart on this worker; every pass of this build reuses
                // the same ids (a pass resumes the shaping of the one before it).
                let build_base = id_base.max(worker_next_id);
                // QY53o3c: record which memo entries this build uses; a finished build drops the
                // entries no recent build used (the memo grew without bound on a long-lived page).
                crate::layout::memo_track_begin();
                let mut superseded_by = None;
                let mut finished = false;
                loop {
                    let deadline = std::time::Instant::now() + PROGRESSIVE_LAYOUT_TICK_BUDGET;
                    let mut incomplete = false;
                    let mut next_id = build_base;
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
                    let worker_t0 = crate::frame_stats::worker_timing().then(std::time::Instant::now);
                    let mut root_h = 0.0f32;
                    let measured = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // QINC1j: an engine job records every `Ck` (the main thread adopts it).
                        if let Some(job) = &engine {
                            let (laid, h) = crate::incr_layout::worker_measure(
                                &tree, &mut font_system, width, height, &mut next_id, &mut shape_cache, deadline, &mut incomplete, &job.ver,
                            );
                            root_h = h;
                            return laid;
                        }
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
                    if let Some(t) = worker_t0 {
                        crate::frame_stats::add_worker_ns(t.elapsed().as_nanos() as u64);
                    }
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
                            shape_cache.clear(); // ids of the abandoned pass may be reused
                            break;
                        }
                    };
                    worker_next_id = worker_next_id.max(next_id);
                    if !incomplete {
                        let mut live = std::collections::HashSet::new();
                        crate::layout::collect_live_ids(&laid, &mut live);
                        crate::layout::sweep_shape_cache(&mut shape_cache, &live);
                        last_live = live;
                    }
                    if !incomplete && let Some(job) = engine {
                        // The tree goes back with its layout (the `Ck`s point into it).
                        let r = crate::incr_frame::WorkerResult { tree, job, root_h, vw: width, vh: height };
                        let _ = proxy.send_event(AppEvent::ShapeResult(generation, Box::new(laid), shape_cache.clone(), false, worker_next_id, Some(Box::new(r))));
                        finished = true;
                        break;
                    }
                    if proxy.send_event(AppEvent::ShapeResult(generation, Box::new(laid), shape_cache.clone(), incomplete, worker_next_id, None)).is_err() {
                        return; // the event loop (and App) is gone
                    }
                    if !incomplete {
                        finished = true;
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
                if finished {
                    crate::layout::memo_sweep(WORKER_MEMO_CAP, WORKER_MEMO_KEEP);
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
        // QY53i2: the old document's custom-property tables (and the QY53i memo) go with it.
        crate::css::reset_custom_props_arena();
        // QFF1a: the old page's web fonts go (from every font system at its next sync).
        crate::webfonts::new_page();
        crate::webfonts::sync(&mut self.font_system, &mut self.font_sync);
        crate::js_geom::sync_forced_fonts();
        self.adopted_css.clear();
        self.link_sheets = crate::link_css::LinkSheets::adopt(&self.document, self.base_url.as_ref(), extra_css.len(), Vec::new());
        self.extra_css = extra_css;
        self.layout_dirty = true;
        self.layout_generation += 1;
        self.cached = None;
        // QSEARCH1 (T31, S6): a navigation clears the highlights; the query is kept.
        self.search.clear_highlights();
        self.restyle_requested = false;
        self.hold_old_frame = false;
        self.js_restyle_pending = false;
        self.layout_in_flight = false;
        self.image_relayout_since = None;
        self.shape_cache.clear();
        self.scroll_offsets.clear();
        self.node_scroll.clear();
        self.node_scroll_dirty = false;
        self.set_scroll_now(0.0);
        self.scroll_x = 0.0;
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
        self.status_message = text.to_string();
        let html = format!("<html><body><p>{}</p></body></html>", escape_html(text));
        self.image_cache.clear();
        self.css_images_requested.clear();
        // QB6: a status/error page replaces the document, so the old
        // page's engine must not keep running against it.
        self.js = None;
        self.replace_document(parse_html(&html), String::new());
    }

    #[cfg_attr(not(test), allow(dead_code))] // the event loop passes the engine job (`handle_shape_result_job`)
    pub(crate) fn handle_shape_result(&mut self, generation: u64, laid: Box<Laid>, shape_cache: ShapeCache, incomplete: bool) {
        self.handle_shape_result_job(generation, laid, shape_cache, incomplete, None)
    }

    /// `handle_shape_result`, plus (QINC1j) the worker's box-tree copy of an engine job for the
    /// engine to adopt with the complete layout (and the worker's next paragraph id).
    pub(crate) fn handle_shape_result_job(
        &mut self,
        generation: u64,
        laid: Box<Laid>,
        shape_cache: ShapeCache,
        incomplete: bool,
        adopt: Option<(Box<crate::incr_frame::WorkerResult>, usize)>,
    ) {
        // Same staleness guard `NavResult`/`ImageFetched` already
        // use, tagged with `layout_generation` instead of `nav_
        // generation` — a background shaping pass that only
        // finishes after a resize/chunk/navigation/image already
        // started a newer one must never clobber content that's
        // already further along. See `AppEvent::ShapeResult`'s
        // own docs.
        if let Some(fs) = &mut self.frame_stats {
            fs.shape_result(incomplete);
        }
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
        let laid = Rc::new(*laid);
        self.live_ver = match adopt {
            Some((r, next_id)) if !incomplete => crate::incr_frame::adopt_worker_result(*r, &laid, &shape_cache, next_id),
            _ => 0,
        };
        self.install_laid(laid, shape_cache, incomplete);
    }

    /// Shows a laid tree (a worker delivery or, QINC1j, the engine's tree) with its shape cache.
    fn install_laid(&mut self, laid: Rc<Laid>, shape_cache: ShapeCache, incomplete: bool) {
        if !incomplete {
            self.layout_in_flight = false;
            if self.js_restyle_pending {
                self.request_js_restyle();
            }
            // Q35b2: images that arrived during the pass rebuild now.
            self.flush_image_relayout();
            // QEL2b: the ResizeObserver targets' boxes of this frame's layout (a change with no DOM
            // write, e.g. an image decode or an animated size, asks for a rendering update).
            if self.js.is_some() && crate::ro_geom::active() {
                crate::ro_geom::app_install(&self.document, &laid, self.build_stamp, self.cached_width, self.cached_height);
            }
        }
        // Replaced wholesale on every tick this generation's
        // worker reports, not merged — see `App::shape_cache`'s
        // own docs for why that's always safe (every entry here
        // is a strict superset of the last delivery's, same
        // generation, same underlying tree).
        self.shape_cache = shape_cache;
        self.cached = Some(laid);
        // QSEARCH1 (S5): matches follow every relayout; the current one stays the nearest.
        if let Some(l) = &self.cached {
            self.search.relayout(l, &self.shape_cache);
        }
        self.restore_node_scroll();
        self.refresh_cursor();
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
    /// QXSCR1: entries for x offsets keep their `xk` key; `JsEngine::set_scroll` splits them.
    fn sync_node_scroll(&mut self) -> Option<Vec<(usize, f32)>> {
        if !std::mem::take(&mut self.node_scroll_dirty) {
            return None;
        }
        Some(self.node_scroll.iter().map(|(k, v)| (*k, *v)).collect())
    }

    /// Q33c: if page scripts wrote to the DOM, request one layout
    /// rebuild (never one per write) and a redraw.
    pub(crate) fn js_sync(&mut self) {
        // QFF1a: faces layout asked for, and faces that arrived (fetches, script `FontFace`s).
        self.webfonts_tick();
        for src in std::mem::take(&mut self.late_image_errors) {
            self.image_settled(&src, None);
        }
        let mut links_changed = false;
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
                let _ = self.proxy.send_event(AppEvent::JsNet(self.nav_generation, id, result, std::time::Instant::now()));
            }
            self.pump_net();
            links_changed = reqs.links_changed;
            let imgs = std::mem::take(&mut reqs.imgs);
            fetch_late_images(&self.proxy, self.socket_path.as_deref(), self.base_url.as_ref(), &mut self.late_images_requested, &mut self.late_image_errors, self.nav_generation, imgs);
            for (h, y, x_axis) in reqs.el_scroll {
                let node = node_id(&h);
                let mut v = (y as f32).max(0.0);
                // QXSCR1: `scrollLeft` writes go to the x keys (`layout::xk`).
                let key = |k: usize| if x_axis { crate::layout::xk(k) } else { k };
                if let Some(info) = self.cached.as_ref().and_then(|l| crate::paint::find_scroll_node(l, node)) {
                    v = v.min(if x_axis { info.max_scroll_x() } else { info.max_scroll() });
                    self.scroll_offsets.insert(key(info.id), v);
                }
                self.node_scroll.insert(key(node), v);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            if let Some(sr) = reqs.scroll {
                if let Some(y) = sr.y {
                    self.set_scroll_now(if sr.relative { self.scroll_offset + y as f32 } else { y as f32 }.max(0.0));
                }
                // QXSCR1: and the x axis.
                if let Some(x) = sr.x {
                    self.scroll_x = if sr.relative { self.scroll_x + x as f32 } else { x as f32 }.max(0.0);
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
        }
        if links_changed {
            self.sync_link_sheets();
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

    /// QY54b1: rebuilds `extra_css` from the page's stylesheet links after one
    /// changed or a sheet arrived, starts fetches for new hrefs (one thread each,
    /// through wire, like images) and fires `load`/`error` at settled links.
    pub(crate) fn sync_link_sheets(&mut self) {
        let mut out = self.link_sheets.sync(&self.document, self.base_url.as_ref(), &self.extra_css);
        let fetch = std::mem::take(&mut out.fetch);
        match (&self.socket_path, &self.base_url) {
            (Some(socket), Some(base)) => {
                for href in fetch {
                    let (proxy, socket, base, generation) = (self.proxy.clone(), socket.clone(), base.clone(), self.nav_generation);
                    thread::spawn(move || {
                        let css = crate::link_css::fetch_link_sheet(&socket, &base, &base, &href);
                        let _ = proxy.send_event(AppEvent::LinkCss(generation, href, css));
                    });
                }
            }
            // No wire (or nothing to resolve against): the sheet cannot load.
            _ => {
                for href in fetch {
                    let _ = self.proxy.send_event(AppEvent::LinkCss(self.nav_generation, href, None));
                }
            }
        }
        let fires = !out.fire.is_empty();
        if crate::link_css::apply_sync(&mut out, &mut self.extra_css, self.js.as_ref()) {
            self.layout_dirty = true;
            self.layout_generation += 1;
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        if fires {
            self.js_sync();
        }
    }

    /// QF7: starts queued page requests, one thread each (like image
    /// fetches), as far as the policy's parallelism and byte caps allow.
    pub(crate) fn pump_net(&mut self) {
        // QM1k1n6: borrow, never clone, the proxy here. Dropping an `EventLoopProxy`
        // pings the event loop (calloop `Sender::drop`), and this runs every turn
        // (`about_to_wait` -> `js_sync`), so a clone per call woke the loop at once,
        // forever: 20-40k idle turns/s and one core at 100 % on any page with JS.
        let (proxy, generation) = (&self.proxy, self.nav_generation);
        let exec = crate::js_net::net_executor();
        self.net.pump(&mut |job| {
            let (proxy, exec) = (proxy.clone(), exec.clone());
            thread::spawn(move || {
                let result = exec.execute(&job);
                let _ = proxy.send_event(AppEvent::JsNet(generation, job.id, result, std::time::Instant::now()));
            });
        });
    }

    /// QF7: `AppEvent::JsNet` handler. A result of a superseded page is
    /// dropped; while `:help` parks the engine it is kept for `restore_saved_page`.
    pub(crate) fn handle_js_net(&mut self, generation: u64, id: u64, result: Result<WireResponse, NetError>, completed: std::time::Instant) {
        if generation != self.nav_generation {
            return;
        }
        self.net.done(id);
        self.pump_net();
        if let Some(js) = &self.js {
            // QOBS6: how long the finished result waited for the main thread.
            crate::perf::net_delivery(completed.elapsed().as_nanos() as u64);
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
        js.set_scroll(self.scroll_x, self.scroll_offset, els);
        let ok = js.dispatch_event(target, kind, init);
        self.js_sync();
        ok
    }

    /// QANIMHLb: no CSS animation and no CSS transition needs the clock.
    pub(crate) fn anims_idle(&self) -> bool {
        self.anim.is_idle() && !crate::transition::live()
    }

    /// QAN1c: dispatches the animation events due at app clock `now` (ms),
    /// queues one restyle if an animation started or ended since the last
    /// build, and returns the app-clock time of the next change.
    pub(crate) fn anim_tick(&mut self, now: f64) -> Option<f64> {
        if self.anims_idle() {
            return None;
        }
        let mut t = if self.anim.is_idle() { AnimTick::default() } else { self.anim.tick(now) };
        // QANIMHLb: CSS transitions. In-between restyles no more often than every max(33 ms, twice
        // the last build) (QAN1-D D3's 30 fps cap); none at all (end values only) after a build slower
        // than `HOVER_SLOW_BUILD`'s 50 ms, as QAN1d planned for layout-affecting animations.
        let build_ms = self.last_build_time.map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        let interval = if build_ms > 50.0 { f64::INFINITY } else { (1000.0 / 30.0f64).max(2.0 * build_ms) };
        let tt = crate::transition::tick(now, interval);
        t.restyle |= tt.restyle;
        t.next = match (t.next, tt.next) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        for ev in tt.events {
            let init = [
                ("bubbles", EventField::Bool(true)),
                ("propertyName", EventField::Str(ev.property.to_string())),
                ("elapsedTime", EventField::Num(ev.elapsed_s)),
                ("pseudoElement", EventField::Str(ev.pseudo)),
            ];
            if let Some(node) = ev.node {
                self.js_dispatch(Some(&node), ev.kind, &init);
            }
        }
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
            crate::incr_frame::note_ambient(crate::incr_log::Ambient::AnimTick);
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

    /// Q33d: `keydown`/`keyup` at the focused element (else `<body>`) is `js_key_codes`;
    /// true if the page did not cancel it.
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
            Ok(FetchedPage { html, extra_css, sheets, scripts, url }) => {
                let dom = new_parser().read_from(&mut html.as_slice()).unwrap();
                let final_url = url.clone();
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
                if !sheets.is_empty() {
                    self.link_sheets = crate::link_css::LinkSheets::adopt(&self.document, self.base_url.as_ref(), self.extra_css.len(), sheets);
                }
                // QA6: only now is the document the one the fragment names.
                self.pending_fragment = self.nav_fragment.take();
                self.js_viewport = None;
                // QNAV9b: a local file read without wire has no fetched scripts: its
                // inline ones run like those of a file opened from the command line.
                let scripts = if scripts.is_empty() && self.socket_path.is_none() && self.base_url.as_ref().is_some_and(|b| b.scheme() == "file") {
                    inline_scripts(&self.document)
                } else {
                    scripts
                };
                self.js = run_page_scripts(&self.document, &scripts, self.base_url.as_ref());
                self.js_sync();
                self.record_visit_if_pending(&final_url);
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
                self.visit_pending = false; // QCOMP3: failed load
                self.show_message(&message);
            }
        }
    }

    /// QCOMP3: the page just loaded was a visit: one history line (final URL, current title).
    fn record_visit_if_pending(&mut self, url: &Url) {
        if !std::mem::take(&mut self.visit_pending) {
            return;
        }
        let title = self
            .js
            .as_ref()
            .and_then(|js| js.eval_value::<String>("title", "document.title"))
            .unwrap_or_else(|| crate::history::dom_title(&self.document));
        crate::history::record_visit(url, &title);
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
            self.visit_pending = true;
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
        let image = Arc::new(image);
        self.image_cache.insert(src.clone(), image.clone());
        // QINC1k6: the engine's forced-layout images see it before `load` fires.
        crate::incr_engine::note_window_image(&self.document, &self.image_cache, &src, &image);
        crate::incr_frame::note_ambient(crate::incr_log::Ambient::Image);
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

    /// QFF1a: `AppEvent::FontFetched`: registers the face (stale pages are dropped) and lays out again.
    pub(crate) fn handle_font_fetched(&mut self, generation: u64, key: &str, page: u64, face: Option<fontdb::FaceInfo>) {
        if generation != self.nav_generation {
            return;
        }
        let failed = face.is_none();
        if crate::webfonts::finish_load(key, page, face) {
            if failed && let Some(js) = &self.js {
                js.fonts_sync();
            }
            self.webfonts_tick();
        }
    }

    /// QFF1a: starts fetches for the web faces layout asked for (`webfonts::take_wanted`): `local()`
    /// sources are looked up here in the system fonts, the rest is fetched through wire on a thread
    /// per face; the result comes back as `AppEvent::FontFetched`.
    pub(crate) fn fetch_wanted_fonts(&mut self) {
        if !crate::webfonts::has_wanted() {
            return;
        }
        let jobs = crate::webfonts::take_wanted();
        if jobs.is_empty() {
            return;
        }
        let base = self.base_url.clone().unwrap_or_else(|| Url::parse("about:blank").expect("static url"));
        for job in jobs {
            let locals = crate::webfonts::resolve_locals(self.font_system.db(), &job.srcs);
            let (proxy, socket, base, generation) = (self.proxy.clone(), self.socket_path.clone(), base.clone(), self.nav_generation);
            thread::spawn(move || {
                let face = crate::webfonts::load_sources(socket.as_deref(), &base, &base, &base, &job.srcs, &locals);
                let _ = proxy.send_event(AppEvent::FontFetched(generation, job.key, job.page, face.map(Box::new)));
            });
        }
        if let Some(js) = &self.js {
            js.fonts_sync();
        }
    }

    /// QFF1a: after events and script tasks: fetch wanted faces, and when faces were added or removed
    /// (a fetch, script `FontFace`s), bring the App's and the forced-layout font systems up to date,
    /// restart the incremental engine (its kept shapes used the fallback font) and lay out again.
    pub(crate) fn webfonts_tick(&mut self) {
        self.fetch_wanted_fonts();
        if !crate::webfonts::sync(&mut self.font_system, &mut self.font_sync) {
            return;
        }
        crate::js_geom::sync_forced_fonts();
        crate::incr_style::drop_engines();
        if let Some(js) = &self.js {
            js.fonts_sync();
        }
        // Collapsed like image arrivals (a burst of faces is one relayout).
        self.restyle_requested = true;
        if self.layout_dirty {
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
                link_sheets: self.link_sheets.clone(),
                adopted_css: self.adopted_css.clone(),
                image_cache: self.image_cache.clone(),
                scroll_offset: self.scroll_offset,
                scroll_x: self.scroll_x,
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
        self.link_sheets = saved.link_sheets;
        self.adopted_css = saved.adopted_css;
        js_style::set_js_adopted_css(&self.adopted_css);
        self.image_cache = saved.image_cache;
        self.scroll_offsets = saved.scroll_offsets;
        self.set_scroll_now(saved.scroll_offset);
        self.scroll_x = saved.scroll_x;
        self.js = saved.js;
        self.js_viewport = None;
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
        crate::incr_frame::note_ambient(crate::incr_log::Ambient::Interaction);
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
        // QKEYS1: the status bar is chrome; the page is not under it.
        if self.surface_size.1 > 0 && pos.1 >= self.page_size().1 as f32 {
            return 0;
        }
        match &self.cached {
            Some(laid) => crate::paint::with_scroll_x(self.scroll_x, || {
                hit_test_element(
                    laid, pos.0 + self.scroll_x, pos.1 + self.scroll_offset, &self.shape_cache, &self.scroll_offsets,
                    self.scroll_offset, self.page_size().1 as f32,
                )
            }),
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
        let mask = self.js.as_ref().map_or(0, |j| j.pointer_listener_mask());
        let hold = |n: usize| (n != 0).then(|| find_node_by_id(&self.document, n)).flatten();
        let (old_h, new_h) = (hold(old), hold(new));
        // Elements the pointer left / entered (innermost first / outermost first).
        let chain = |h: &Option<Handle>| h.as_ref().map(ancestor_ids).unwrap_or_default();
        let (old_chain, new_chain) = (chain(&old_h), chain(&new_h));
        let left: Vec<Handle> =
            old_chain.iter().filter(|i| !new_chain.contains(i)).filter_map(|&i| find_node_by_id(&self.document, i)).collect();
        let entered: Vec<Handle> =
            new_chain.iter().rev().filter(|i| !old_chain.contains(i)).filter_map(|&i| find_node_by_id(&self.document, i)).collect();
        let want = |kind: &str| mask & PTR_BITS.iter().position(|t| *t == kind).map_or(0, |i| 1 << i) != 0;
        let go = |app: &mut Self, h: &Handle, kind: &str, bubbles: bool, related: &Option<Handle>| {
            let init = app.pointer_init(pos, related, bubbles, kind.starts_with("pointer"));
            app.js_dispatch(Some(h), kind, &init);
        };
        if let Some(h) = &old_h {
            if want("pointerout") {
                go(self, h, "pointerout", true, &new_h);
            }
            if want("pointerleave") {
                for l in &left {
                    go(self, l, "pointerleave", false, &new_h);
                }
            }
            go(self, h, "mouseout", true, &new_h);
            if want("mouseleave") {
                for l in &left {
                    go(self, l, "mouseleave", false, &new_h);
                }
            }
        }
        if let Some(h) = &new_h {
            if want("pointerover") {
                go(self, h, "pointerover", true, &old_h);
            }
            if want("pointerenter") {
                for e in &entered {
                    go(self, e, "pointerenter", false, &old_h);
                }
            }
            go(self, h, "mouseover", true, &old_h);
            if want("mouseenter") {
                for e in &entered {
                    go(self, e, "mouseenter", false, &old_h);
                }
            }
        }
    }

    /// Init dict shared by the pointer-motion events (QCUR1): client/page/screen
    /// coordinates, modifiers, `buttons`, and for `pointer*` types the PointerEvent fields.
    fn pointer_init(&self, pos: (f32, f32), related: &Option<Handle>, bubbles: bool, pointer: bool) -> Vec<(&'static str, EventField)> {
        let m = &self.modifiers;
        let buttons = if self.mouse_down_pos.is_some() { 1.0 } else { 0.0 };
        let mut v = vec![
            ("bubbles", EventField::Bool(bubbles)),
            ("cancelable", EventField::Bool(bubbles)),
            ("clientX", EventField::Num(f64::from(pos.0))),
            ("clientY", EventField::Num(f64::from(pos.1))),
            ("screenX", EventField::Num(f64::from(pos.0))),
            ("screenY", EventField::Num(f64::from(pos.1))),
            ("pageX", EventField::Num(f64::from(pos.0 + self.scroll_x))),
            ("pageY", EventField::Num(f64::from(pos.1 + self.scroll_offset))),
            ("buttons", EventField::Num(buttons)),
            ("ctrlKey", EventField::Bool(m.control_key())),
            ("shiftKey", EventField::Bool(m.shift_key())),
            ("altKey", EventField::Bool(m.alt_key())),
            ("metaKey", EventField::Bool(m.super_key())),
        ];
        if pointer {
            v.push(("pointerId", EventField::Num(1.0)));
            v.push(("pointerType", EventField::Str("mouse".into())));
            v.push(("isPrimary", EventField::Bool(true)));
            v.push(("pressure", EventField::Num(buttons * 0.5)));
        }
        if let Some(r) = related {
            v.push(("relatedTargetId", EventField::Node(r.clone())));
        }
        v
    }

    /// QCUR1 (Q33d2): `pointermove` then `mousemove` on the element under the pointer,
    /// bubbling, at most once per event-loop turn (called from `process_pointer_move`)
    /// with the latest position; skipped when the page has no such listener. YouTube
    /// un-hides its controls and the cursor (`ytp-autohide`) from these.
    fn js_pointer_move(&mut self, pos: (f32, f32), node: usize) {
        if self.js.is_none() {
            return;
        }
        // Page handlers for motion events are costly on heavy pages (YouTube: forced
        // layouts), so without a button held they are spaced out: at least the adaptive
        // gap, and `MOVE_GAP_PLAYING` while a video plays. The first move after a second
        // without one always goes straight through (YouTube un-hides at once), and a
        // throttled move leaves a trailing dispatch (`flush_move_trailing`) so the final
        // position is not lost. A drag (button down) gets every turn's latest position.
        let now = std::time::Instant::now();
        let idle = self.last_move_seen.is_none_or(|t| now.duration_since(t) >= MOVE_IDLE);
        self.last_move_seen = Some(now);
        if self.mouse_down_pos.is_none() && !idle && self.last_move_dispatch_at.is_some_and(|t| t.elapsed() < self.move_gap()) {
            self.move_trailing = Some(pos);
            return;
        }
        self.move_trailing = None;
        self.dispatch_pointer_move(pos, node);
    }

    /// The minimum spacing between two `mousemove` dispatches right now.
    fn move_gap(&self) -> std::time::Duration {
        if self.video_playing() { self.move_dispatch_gap.max(MOVE_GAP_PLAYING) } else { self.move_dispatch_gap }
    }

    /// A video frame was presented within the last 500 ms.
    pub(crate) fn video_playing(&self) -> bool {
        self.last_video_frame.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(500))
    }

    /// The trailing `mousemove` of a throttled burst, once its gap has passed. Returns
    /// when to be called again (`None` = nothing pending). Called from `about_to_wait`.
    pub(crate) fn flush_move_trailing(&mut self) -> Option<std::time::Instant> {
        let pos = self.move_trailing?;
        let due = self.last_move_dispatch_at.map_or_else(std::time::Instant::now, |t| t + self.move_gap());
        if std::time::Instant::now() < due {
            return Some(due);
        }
        self.move_trailing = None;
        let node = self.element_at(pos);
        self.dispatch_pointer_move(pos, node);
        None
    }

    fn dispatch_pointer_move(&mut self, pos: (f32, f32), node: usize) {
        let Some(js) = &self.js else { return };
        let mask = js.pointer_listener_mask();
        let (want_p, want_m) = (mask & 2 != 0, mask & 1 != 0);
        if !want_p && !want_m {
            return;
        }
        let target = (node != 0).then(|| find_node_by_id(&self.document, node)).flatten();
        let prev = self.last_move_dispatched.replace(pos).unwrap_or(pos);
        let (mx, my) = (f64::from(pos.0 - prev.0), f64::from(pos.1 - prev.1));
        let started = std::time::Instant::now();
        for (kind, pointer, want) in [("pointermove", true, want_p), ("mousemove", false, want_m)] {
            if want {
                let mut init = self.pointer_init(pos, &None, true, pointer);
                init.push(("movementX", EventField::Num(mx)));
                init.push(("movementY", EventField::Num(my)));
                self.js_dispatch(target.as_ref(), kind, &init);
            }
        }
        // Adaptive: leave the main thread at least 3x as long as the handlers took.
        self.move_dispatch_gap = (started.elapsed() * 3).clamp(MOVE_DISPATCH_INTERVAL, MOVE_DISPATCH_MAX_GAP);
        self.last_move_dispatch_at = Some(std::time::Instant::now());
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
        // QY53n5: a hover that needs no build (skip / paint patch) is applied now.
        if self.apply_hover_cheap() {
            return;
        }
        if let Some(delay) = self.hover_restyle_delay() {
            self.schedule_hover_flush(delay);
            return;
        }
        self.apply_hover_build();
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
        if !self.apply_hover_cheap() {
            self.apply_hover_build();
        }
    }

    /// QY53n5: the hover change that needs no build: same chain, or a chain no
    /// `:hover` rule can read (R2.4). Returns false, with the
    /// state untouched, when a build is needed. These cost 4-10 ms, so
    /// `update_hover` applies them without the R2.3/QB3 coalescing delay.
    fn apply_hover_cheap(&mut self) -> bool {
        let mut st = self.interaction.clone();
        st.hover = self.chain_for(self.hover_node);
        if st.hover == self.interaction.hover {
            return true;
        }
        // R2.4: only elements entering/leaving the hover chain can change
        // style. If none of them could match a `:hover` compound, record
        // the new chain (so later diffs are right) but don't restyle.
        let relevant = st.hover.symmetric_difference(&self.interaction.hover).any(|&id| {
            find_node_by_id(&self.document, id).is_none_or(|h| self.hover_keys.may_match(&h))
        });
        if !relevant {
            self.interaction.hover = st.hover;
            set_interaction(&self.interaction);
            return true;
        }
        // QINC1j: otherwise a hover is an ordinary (incremental) engine frame.
        false
    }

    /// The hover change that needs a full build (R2.3: coalesced by the caller).
    fn apply_hover_build(&mut self) {
        let mut st = self.interaction.clone();
        st.hover = self.chain_for(self.hover_node);
        if st.hover == self.interaction.hover {
            return;
        }
        if let Some(fs) = &mut self.frame_stats {
            fs.hover_restyle();
            if self.state_rules & STATE_HOVER != 0 {
                fs.hover_built();
            }
        }
        self.last_hover_restyle = Some(std::time::Instant::now());
        self.set_interaction_state(st);
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
        let view = self.page_size().1 as f32;
        const MARGIN: f32 = crate::config::SCROLL_INTO_VIEW_MARGIN;
        // QXSCR1: the x axis the same way (only when the element is off screen sideways).
        let (left, right) = geom.rects.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, r), g| (l.min(g.0), r.max(g.0 + g.2)));
        let view_w = self.page_size().0 as f32;
        let before = (self.scroll_x, self.scroll_offset);
        if left < self.scroll_x {
            self.scroll_x = (left - MARGIN).max(0.0);
        } else if right > self.scroll_x + view_w {
            self.scroll_x = (right + MARGIN - view_w).max(0.0).min(left.max(0.0));
        }
        if top < self.scroll_offset {
            self.set_scroll_now((top - MARGIN).max(0.0));
        } else if bottom > self.scroll_offset + view {
            self.set_scroll_now((bottom + MARGIN - view).max(0.0).min(top.max(0.0)));
        }
        if before == (self.scroll_x, self.scroll_offset) {
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
                self.set_scroll_now(0.0);
            }
            return true;
        };
        let Some(laid) = &self.cached else { return false };
        // An inline element without its own box falls back to its ancestors.
        let mut cur = Some(target);
        while let Some(n) = cur {
            let geom = node_geometry(laid, &self.shape_cache, node_id(&n));
            if let Some(top) = geom.rects.iter().map(|r| r.1).reduce(f32::min) {
                self.set_scroll_now(top.max(0.0));
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
        self.visit_pending = true;
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

    /// QKEYS1b: the status bar's URL: the current history entry, else (a file or
    /// stdin start page has no history yet) the page's own address, else `(stdin)`.
    #[cfg(test)]
    pub(crate) fn status_url(&self) -> String {
        status_url(&self.history, self.history_index, self.base_url.as_ref())
    }

    /// QREG1: register `%`: the status bar's URL, None for a stdin page.
    pub(crate) fn url_register(&self) -> Option<String> {
        current_url(&self.history, self.history_index, self.base_url.as_ref())
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
        self.visit_pending = false; // QCOMP3: back/forward is not a visit
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
        self.visit_pending = false; // QCOMP3: reload is not a visit
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
                self.visit_pending = false; // QCOMP3: same-document
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
        // QMEM3d: the old page (engine, DOM, layout) is gone: hand its freed heap back.
        self.idle_trim.activity();
        crate::mem_trim::request();
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
                        WireFetchRaw::Responded { content_type, body, final_url, .. } => match dispatch_wire_response(content_type, body) {
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
                                let PageResources { css: extra_css, sheets, scripts } = process_external_resources(&proxy, path, &url, &html, generation);
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
                                Ok(FetchedPage { html, extra_css, sheets, scripts, url })
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
                None if url.scheme() == "file" => read_local_file_page(&url),
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

    /// A pressed key in the `:` line, or in normal mode (QKEYS2: `route_key` is the entry point for
    /// everything else). `vim_mode == Command` always goes to the command-line editor.
    pub(crate) fn handle_key_event(&mut self, event: &KeyEvent) {
        if self.vim_mode != VimMode::Normal {
            self.pending_seq = None;
            if self.vim_mode == VimMode::Search {
                self.handle_search_key(event);
            } else {
                self.handle_command_line_key(event);
            }
            return;
        }
        self.handle_normal_key(&event.logical_key, event.repeat);
    }

    /// QKEYS2: a key press in normal mode (T6-T14): pending `g`, then the binding table.
    pub(crate) fn handle_normal_key(&mut self, key: &Key, repeat: bool) {
        let (seq_action, pending, consumed) = resolve_sequence(self.pending_seq, key, self.modifiers);
        self.pending_seq = pending;
        if consumed {
            if let Some(action) = seq_action
                && !(repeat && action_ignores_repeat(action))
            {
                self.perform_action(action);
            }
            return;
        }
        if let Some(action) = resolve_binding(key, self.modifiers, self.vim_mode) {
            if repeat && action_ignores_repeat(action) {
                return;
            }
            self.perform_action(action);
        }
    }

    /// QKEYS2: switch mode. Leaving normal mode drops a pending `g` and an open `:` line.
    pub(crate) fn set_key_mode(&mut self, mode: KeyMode) {
        self.key_mode = mode;
        self.esc_state = EscState::Idle;
        self.pending_seq = None;
        if mode == KeyMode::Input && self.vim_mode != VimMode::Normal {
            self.vim_mode = VimMode::Normal;
            self.command_line.clear();
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// QKEYS2 (P4, T29): a left press at `pos` on a text control or select switches to input mode.
    pub(crate) fn press_switches_to_input(&mut self, pos: (f32, f32)) {
        if self.key_mode == KeyMode::Input && self.vim_mode == VimMode::Normal {
            return;
        }
        let node = self.element_at(pos);
        let Some(target) = (node != 0).then(|| find_node_by_id(&self.document, node)).flatten() else { return };
        let Some(control) = enclosing_control(&target) else { return };
        if !is_disabled_control(&control) && (text_control_kind(&control).is_some() || element_tag(&control) == "select") {
            self.set_key_mode(KeyMode::Input);
        }
    }

    /// QKEYS2: one keyboard event outside the `:` line (KEYS.md T1-T21, T33, T34), with the clock
    /// passed in. Input mode: Esc Esc check, then page `keydown`/`keyup` and the UA controls in
    /// today's order, bindings never consulted. Normal mode: no page events, straight to the bindings.
    pub(crate) fn route_key(&mut self, key: &Key, physical: Option<PhysicalKey>, pressed: bool, repeat: bool, now: std::time::Instant) {
        if !pressed {
            if let Some(i) = self.undelivered_presses.iter().position(|k| k == key) {
                self.undelivered_presses.remove(i);
                return;
            }
            if self.key_mode == KeyMode::Input {
                self.js_key_codes("keyup", key, physical, repeat);
            }
            return;
        }
        if self.key_mode == KeyMode::Normal {
            self.handle_normal_key(key, repeat);
            if self.key_mode == KeyMode::Input {
                self.undeliver(key);
            }
            return;
        }
        let (st, enter) = resolve_esc_esc(self.esc_state, key, self.modifiers, repeat, now);
        self.esc_state = st;
        if enter {
            self.set_key_mode(KeyMode::Normal);
            self.undeliver(key);
            return;
        }
        if !self.js_key_codes("keydown", key, physical, repeat) {
            return;
        }
        // Q30g: an open select dropdown owns every key (Tab closes it, then moves focus).
        if self.select_popup.is_some() && self.select_popup_key(key) {
            return;
        }
        // Q30g: a focused closed select / listbox owns the keys like a text control.
        if !matches!(key, Key::Named(NamedKey::Tab))
            && let Some(h) = self.focused_select()
            && self.select_control_key(&h, key)
        {
            return;
        }
        // Q30d: a focused text control owns the keys (Tab and the page's own keydown handlers excepted).
        if !matches!(key, Key::Named(NamedKey::Tab))
            && let Some(h) = self.focused_text_control()
            && self.text_control_key(&h, key, repeat)
        {
            return;
        }
        if self.media_key(key, repeat) {
            return;
        }
        match key {
            Key::Named(NamedKey::Tab) => {
                self.tab_focus(self.modifiers.shift_key());
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            // Q30c: Space toggles a focused checkbox/radio.
            Key::Named(NamedKey::Space)
                if !repeat
                    && !self.modifiers.control_key()
                    && !self.modifiers.alt_key()
                    && let Some(h) = self.interaction.focus.and_then(|f| find_node_by_id(&self.document, f))
                    && checkable_kind(&h).is_some()
                    && !is_disabled_control(&h) =>
            {
                self.click_activate(Some(&h), (0.0, 0.0));
            }
            // Q30e: Enter/Space presses a focused button.
            Key::Named(NamedKey::Enter | NamedKey::Space)
                if !repeat
                    && let Some(h) = self.interaction.focus.and_then(|f| find_node_by_id(&self.document, f))
                    && (element_tag(&h) == "button" || (element_tag(&h) == "input" && matches!(input_type(&h).as_str(), "submit" | "image" | "reset" | "button")))
                    && !is_disabled_control(&h) =>
            {
                self.click_activate(Some(&h), (0.0, 0.0));
            }
            Key::Named(NamedKey::Enter) if self.interaction.focus.is_some() => self.activate_focus(),
            // QKEYS4 (P6): the UA default scroll keys, after the page and every focused control declined.
            _ => self.ua_scroll_key(key),
        }
    }

    /// QKEYS4: ArrowUp/Down, PageUp/PageDown, Space/Shift-Space, Home/End (QXSCR1: and ArrowLeft/Right,
    /// on the x axis) scroll the page in input mode
    /// (same actions as the normal-mode bindings, but only the named keys: `j`/`k`/`G` are the page's).
    pub(crate) fn ua_scroll_key(&mut self, key: &Key) {
        if !matches!(key, Key::Named(_)) {
            return;
        }
        if let Some(a) = resolve_binding(key, self.modifiers, VimMode::Normal)
            && matches!(
                a,
                Action::ScrollDown | Action::ScrollUp | Action::ScrollLeft | Action::ScrollRight | Action::PageDown
                    | Action::PageUp | Action::ScrollTop | Action::ScrollBottom
            )
        {
            self.perform_action(a);
        }
    }

    fn undeliver(&mut self, key: &Key) {
        self.undelivered_presses.push(key.clone());
        if self.undelivered_presses.len() > 8 {
            self.undelivered_presses.remove(0);
        }
    }

    /// Carries out an already-resolved `Action` — kept separate from
    /// `resolve_binding` itself so that function can stay a pure,
    /// unit-testable `key -> Option<Action>` mapping with no `&mut App`
    /// at all.
    pub(crate) fn perform_action(&mut self, action: Action) {
        use crate::config::SCROLL_LINE_PX;
        // QYSCR1b: a viewport with `overflow: hidden` ignores the scroll keys
        // (scratchy has no focused scroll region, so keys always target the viewport).
        if self.viewport_user_scroll_blocked()
            && matches!(
                action,
                Action::ScrollDown | Action::ScrollUp | Action::HalfPageDown | Action::HalfPageUp | Action::PageDown
                    | Action::PageUp | Action::ScrollTop | Action::ScrollBottom
            )
        {
            return;
        }
        // QXSCR1: the same for the x axis (`overflow-x: hidden` propagated to the viewport).
        if self.viewport_user_scroll_blocked_x() && matches!(action, Action::ScrollLeft | Action::ScrollRight) {
            return;
        }
        match action {
            Action::ScrollLeft => self.scroll_x_by(-SCROLL_LINE_PX),
            Action::ScrollRight => self.scroll_x_by(SCROLL_LINE_PX),
            Action::ScrollDown => self.scroll_by(SCROLL_LINE_PX),
            Action::ScrollUp => self.scroll_by(-SCROLL_LINE_PX),
            Action::HalfPageDown => self.scroll_by(self.page_size().1 as f32 / 2.0),
            Action::HalfPageUp => self.scroll_by(-(self.page_size().1 as f32 / 2.0)),
            Action::PageDown => self.scroll_by(page_step(self.page_size().1 as f32)),
            Action::PageUp => self.scroll_by(-page_step(self.page_size().1 as f32)),
            Action::EnterCommandMode => {
                self.vim_mode = VimMode::Command;
                self.command_line.clear();
                self.ghost = None;
                let proxy = Mutex::new(self.proxy.clone());
                let sink: crate::completion::ExtSink = std::sync::Arc::new(move |r| {
                    let _ = proxy.lock().unwrap_or_else(|e| e.into_inner()).send_event(AppEvent::CompleteResult(r));
                });
                let url = self.base_url.as_ref().map(|u| u.to_string()).unwrap_or_default();
                self.complete_cache = CompleteCache::new(crate::history::dir()).with_external(sink, url);
            }
            Action::EnterSearchMode => {
                self.vim_mode = VimMode::Search;
                self.command_line.clear();
                // The ghost is for the `:` line only (QCOMP1).
                self.ghost = None;
            }
            Action::ClearSearch => self.search.clear_highlights(),
            Action::SearchNext => self.search_step(true),
            Action::SearchPrev => self.search_step(false),
            Action::ActivateMatch => self.activate_match(),
            Action::EnterInputMode => self.set_key_mode(KeyMode::Input),
            Action::ScrollTop => self.smooth_scroll_to(0.0),
            // `redraw` clamps to the real max scroll every frame, same as `j`.
            Action::ScrollBottom => self.smooth_scroll_to(f32::MAX),
            Action::HistoryBack => self.navigate_history(-1),
            Action::HistoryFront => self.navigate_history(1),
            Action::YankUrl => self.dispatch_command("let", &["@+", "=", "@%"]),
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
    /// QYSCR1b: the viewport's propagated overflow is `hidden`: the user cannot
    /// scroll it (wheel, keys); `window.scrollTo`/`scrollIntoView` still do.
    pub(crate) fn viewport_user_scroll_blocked(&self) -> bool {
        self.cached.as_ref().is_some_and(|l| l.style.viewport_hidden)
    }

    /// QXSCR1: the same for the x axis (the propagated `overflow-x` is `hidden`).
    pub(crate) fn viewport_user_scroll_blocked_x(&self) -> bool {
        self.cached.as_ref().is_some_and(|l| l.style.viewport_hidden_x)
    }

    /// QSB1: the viewport's own bar: `(rect, content_h, client_h)`, `None` when the page fits
    /// or the viewport cannot be scrolled by the user.
    pub(crate) fn viewport_bar(cached: &Option<Rc<Laid>>, size: (u32, u32)) -> Option<crate::scrollbar::ViewportBar> {
        let laid = cached.as_ref()?;
        let (w, h) = (size.0 as f32, size.1 as f32);
        (laid.h > h && !laid.style.viewport_hidden).then_some(((0.0, 0.0, w, h), laid.h, h))
    }

    /// QSB2b: the window's horizontal bar shows: the page is wider than the window and the
    /// viewport's `overflow-x` is not `hidden`.
    pub(crate) fn want_hbar(cached: &Option<Rc<Laid>>, width: u32) -> bool {
        cached.as_ref().is_some_and(|l| !l.style.viewport_hidden_x && crate::layout::doc_scroll_width(l) > width as f32)
    }

    /// QSB2b: the window's horizontal bar: `(strip, content_w, client_w)`; the strip is the
    /// `TRACK_W` px just below the page viewport `size` (already shortened by it).
    pub(crate) fn viewport_bar_x(cached: &Option<Rc<Laid>>, size: (u32, u32)) -> Option<crate::scrollbar::ViewportBar> {
        let laid = cached.as_ref()?;
        let w = size.0 as f32;
        Self::want_hbar(cached, size.0).then_some(((0.0, size.1 as f32, w, crate::config::TRACK_W), crate::layout::doc_scroll_width(laid), w))
    }

    /// QSB2: the scrollbar under `pos` (the window's, else the outermost region's, since a
    /// parent's bar paints over its children): what it scrolls and where it was hit.
    fn scrollbar_find(&self, pos: (f32, f32)) -> Option<SbFound> {
        use crate::scrollbar::{self as sb, Axis};
        let (px, py) = pos;
        let view_h = self.page_size().1 as f32;
        let vbar = Self::viewport_bar(&self.cached, self.page_size());
        if let Some((rect, content_h, client_h)) = vbar {
            let max = content_h - client_h;
            let cur = self.scroll_offset.clamp(0.0, max);
            if let Some(hit) = sb::hit(rect, content_h, client_h, cur, px, py) {
                let ratio = sb::drag_ratio(rect.3, content_h, client_h);
                return Some(SbFound { target: SbTarget::Viewport, axis: Axis::Y, hit, cur, max, ratio, page: page_step(view_h) });
            }
        }
        if let Some((rect, content_w, client_w)) = Self::viewport_bar_x(&self.cached, self.page_size()) {
            let max = content_w - client_w;
            let cur = self.scroll_x.clamp(0.0, max);
            let corner = vbar.is_some();
            if let Some(hit) = sb::hit_axis(Axis::X, corner, rect, content_w, client_w, cur, px, py) {
                let track_w = if corner { rect.2 - crate::config::TRACK_W } else { rect.2 };
                let ratio = sb::drag_ratio(track_w, content_w, client_w);
                return Some(SbFound { target: SbTarget::Viewport, axis: Axis::X, hit, cur, max, ratio, page: client_w });
            }
        }
        let laid = self.cached.as_ref()?;
        let chain = crate::paint::with_scroll_x(self.scroll_x, || {
            crate::paint::hit_test_scroll_bars(
                laid, px + self.scroll_x, py + self.scroll_offset, &self.scroll_offsets, self.scroll_offset, view_h,
            )
        });
        for r in chain.iter().rev().filter(|r| r.info.user || r.info.user_x) {
            let info = r.info;
            let (max_y, max_x) = (info.max_scroll(), info.max_scroll_x());
            let corner = info.user && info.user_x && max_y > 0.0 && max_x > 0.0;
            if info.user {
                let cur = self.scroll_offsets.get(&info.id).copied().unwrap_or(0.0).clamp(0.0, max_y);
                if let Some(hit) = sb::hit_axis(Axis::Y, corner, r.rect, info.content_h, info.client_h, cur, r.px, r.py) {
                    let track = if corner { r.rect.3 - crate::config::TRACK_W } else { r.rect.3 };
                    let ratio = sb::drag_ratio(track, info.content_h, info.client_h);
                    return Some(SbFound { target: SbTarget::Region(info.id, info.node), axis: Axis::Y, hit, cur, max: max_y, ratio, page: page_step(info.client_h) });
                }
            }
            if info.user_x {
                let cur = self.scroll_offsets.get(&crate::layout::xk(info.id)).copied().unwrap_or(0.0).clamp(0.0, max_x);
                if let Some(hit) = sb::hit_axis(Axis::X, corner, r.rect, info.content_w, info.client_w, cur, r.px, r.py) {
                    let track = if corner { r.rect.2 - crate::config::TRACK_W } else { r.rect.2 };
                    let ratio = sb::drag_ratio(track, info.content_w, info.client_w);
                    return Some(SbFound { target: SbTarget::Region(info.id, info.node), axis: Axis::X, hit, cur, max: max_x, ratio, page: info.client_w });
                }
            }
        }
        None
    }

    /// QSB2b: the hover/active id of the bar `target`'s `axis` (see `scrollbar::VIEWPORT_ID`).
    fn bar_id(target: SbTarget, axis: crate::scrollbar::Axis) -> usize {
        use crate::scrollbar as sb;
        match (target, axis) {
            (SbTarget::Viewport, sb::Axis::Y) => sb::VIEWPORT_ID,
            (SbTarget::Viewport, sb::Axis::X) => sb::VIEWPORT_X_ID,
            (SbTarget::Region(id, _), sb::Axis::Y) => id,
            (SbTarget::Region(id, _), sb::Axis::X) => crate::layout::xk(id),
        }
    }

    /// Sets the scroll position of `target` on `axis` (horizontal moves are instant, QSMOOTH1).
    fn scrollbar_set(&mut self, target: SbTarget, axis: crate::scrollbar::Axis, next: f32) {
        use crate::scrollbar::Axis;
        match (target, axis) {
            (SbTarget::Viewport, Axis::Y) => self.set_scroll_now(next),
            (SbTarget::Viewport, Axis::X) => self.scroll_x = next,
            (SbTarget::Region(id, node), Axis::Y) => self.set_region_scroll(id, node, next),
            (SbTarget::Region(id, node), Axis::X) => {
                let node = if node == 0 { 0 } else { crate::layout::xk(node) };
                self.set_region_scroll(crate::layout::xk(id), node, next);
            }
        }
    }

    /// QSB1: a left press at `pos` on a scrollbar. Thumb: starts a drag. Track: a page
    /// up/down (QSB2b: left/right on a horizontal bar), and (QSB2) a hold that repeats it.
    /// True when the press was consumed (the page then sees no mousedown/click).
    pub(crate) fn scrollbar_press(&mut self, pos: (f32, f32)) -> bool {
        use crate::scrollbar::{self as sb, BarHit};
        let Some(f) = self.scrollbar_find(pos) else { return false };
        match f.hit {
            BarHit::Thumb => {
                self.sb_drag = Some(SbDrag { target: f.target, axis: f.axis, start_ptr: sb::along(f.axis, pos), start_scroll: f.cur, ratio: f.ratio, max: f.max });
            }
            _ => {
                self.scrollbar_set(f.target, f.axis, sb::page_target(f.hit, f.cur, f.page, f.max));
                let delay = std::time::Duration::from_millis(crate::config::SB_REPEAT_DELAY_MS);
                self.sb_hold = Some(SbHold { axis: f.axis, dir: f.hit, next: std::time::Instant::now() + delay });
            }
        }
        self.request_redraw_now();
        true
    }

    /// QSB2: a held track press: once due, page again while the pointer is still on the same
    /// side of the thumb. Returns the next wake-up while the hold lasts.
    pub(crate) fn scrollbar_hold_tick(&mut self) -> Option<std::time::Instant> {
        use crate::scrollbar as sb;
        let hold = self.sb_hold?;
        let now = std::time::Instant::now();
        if now < hold.next {
            return Some(hold.next);
        }
        let interval = std::time::Duration::from_millis(crate::config::SB_REPEAT_INTERVAL_MS);
        let next = now + interval;
        self.sb_hold = Some(SbHold { next, ..hold });
        let f = self.last_mouse_pos.and_then(|p| self.scrollbar_find(p)).filter(|f| f.axis == hold.axis);
        if let Some(f) = f
            && sb::repeat_applies(hold.dir, Some(f.hit))
        {
            self.scrollbar_set(f.target, f.axis, sb::page_target(f.hit, f.cur, f.page, f.max));
            self.request_redraw_now();
        }
        Some(next)
    }

    /// QSB2: thumb hover follows the pointer (a redraw only when it changes).
    fn scrollbar_hover_update(&mut self, pos: (f32, f32)) {
        let hover = self
            .scrollbar_find(pos)
            .filter(|f| f.hit == crate::scrollbar::BarHit::Thumb)
            .map(|f| Self::bar_id(f.target, f.axis));
        if hover != self.sb_hover {
            self.sb_hover = hover;
            self.request_redraw_now();
        }
    }

    /// QSB2: the `(id, pressed)` the painter highlights.
    fn scrollbar_hot(&self) -> Option<(usize, bool)> {
        if let Some(d) = self.sb_drag {
            return Some((Self::bar_id(d.target, d.axis), true));
        }
        self.sb_hover.map(|id| (id, false))
    }

    /// QSB1: the pointer moved to window `pos` during a thumb drag (paint-only, like the wheel).
    pub(crate) fn scrollbar_drag_move(&mut self, pos: (f32, f32)) {
        let Some(d) = self.sb_drag else { return };
        let next = crate::scrollbar::drag_scroll(d.start_scroll, d.start_ptr, crate::scrollbar::along(d.axis, pos), d.ratio, d.max);
        self.scrollbar_set(d.target, d.axis, next);
        self.request_redraw_now();
    }

    /// The wheel's region scroll write (also tells script, `scroll` events).
    fn set_region_scroll(&mut self, id: usize, node: usize, next: f32) {
        self.scroll_offsets.insert(id, next);
        if node != 0 {
            self.node_scroll.insert(node, next);
            self.node_scroll_dirty = true;
        }
    }

    fn request_redraw_now(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// QSMOOTH1 D2: a smoothed relative scroll (j/k/arrows, PageUp/PageDown, wheel notches). With
    /// easing on it moves the TARGET (accumulating over a running animation); off, it is instant.
    pub(crate) fn scroll_by(&mut self, dy: f32) {
        if self.smooth_scroll_tau_ms > 0.0 {
            let base = self.scroll_target.unwrap_or(self.scroll_offset);
            self.smooth_scroll_to(base + dy);
        } else {
            self.set_scroll_now(self.scroll_offset + dy);
        }
    }

    /// QSMOOTH1: a smoothed absolute scroll (gg = 0, G = `f32::MAX`, clamped in `redraw`).
    pub(crate) fn smooth_scroll_to(&mut self, y: f32) {
        let y = y.max(0.0);
        if self.smooth_scroll_tau_ms > 0.0 {
            if self.scroll_target.is_none() {
                self.scroll_anim_last = std::time::Instant::now();
            }
            self.scroll_target = Some(y);
        } else {
            self.set_scroll_now(y);
        }
    }

    /// QSMOOTH1 D4: an instant write to the painted viewport scroll; cancels a running animation.
    pub(crate) fn set_scroll_now(&mut self, y: f32) {
        self.scroll_offset = y;
        self.scroll_target = None;
    }

    /// QSMOOTH1 D5: one animation step with a fixed `dt_ms` (tests; `redraw` inlines the same step with the real dt).
    #[cfg(test)]
    pub(crate) fn scroll_anim_step_dt(&mut self, max_scroll: f32, dt_ms: f32, target: f32) {
        let target = target.clamp(0.0, max_scroll);
        let (offset, done) = scroll_ease(self.scroll_offset.clamp(0.0, max_scroll), target, dt_ms, self.smooth_scroll_tau_ms);
        self.scroll_offset = offset;
        self.scroll_target = if done { None } else { Some(target) };
    }

    /// QXSCR1: the window's horizontal counterpart of `scroll_by` (`redraw` clamps the far end).
    pub(crate) fn scroll_x_by(&mut self, dx: f32) {
        self.scroll_x = (self.scroll_x + dx).max(0.0);
    }

    /// QXSCR1: a horizontal wheel/touchpad tick of `dx` px (winit's sign: positive = left) over
    /// the scroll `chain` (innermost first): the first region that can scroll on x takes it, else
    /// the window (unless its propagated `overflow-x` is `hidden`).
    pub(crate) fn wheel_x(&mut self, chain: &[ScrollInfo], dx: f32) {
        use crate::layout::xk;
        match chain.iter().find(|info| region_takes_wheel_x(info, &self.scroll_offsets, dx)) {
            Some(info) => {
                let current = self.scroll_offsets.get(&xk(info.id)).copied().unwrap_or(0.0);
                let next = (current - dx).clamp(0.0, info.max_scroll_x());
                let node = if info.node == 0 { 0 } else { xk(info.node) };
                self.set_region_scroll(xk(info.id), node, next);
            }
            None if self.viewport_user_scroll_blocked_x() => {}
            None => self.scroll_x_by(-dx),
        }
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
        self.command_line_key(&event.logical_key);
    }

    /// `handle_command_line_key` on a bare key (tests call this: a `KeyEvent` cannot be built).
    /// QCOMP1 (KEYS.md T45-T53): the ghost suggestion. Tab / Shift-Tab cycle it (2+ matches) or
    /// select the only match, Enter runs the typed text plus the ghost, Right / End accept it,
    /// Delete hides it, and every other key edits the line and recomputes it.
    pub(crate) fn command_line_key(&mut self, key: &Key) {
        match key {
            Key::Named(NamedKey::Escape) => {
                self.vim_mode = VimMode::Normal;
                self.command_line.clear();
                self.ghost = None;
            }
            Key::Named(NamedKey::Enter) => {
                let mut input = std::mem::take(&mut self.command_line.text);
                if let Some(g) = self.ghost.take() {
                    input = g.accepted(&input);
                }
                self.command_line.cursor = 0;
                self.vim_mode = VimMode::Normal;
                self.run_command_line(&input);
            }
            Key::Named(NamedKey::Tab) => {
                let text = &self.command_line.text;
                if let Some(g) = &mut self.ghost {
                    if self.modifiers.shift_key() {
                        g.prev();
                    } else if let Some(add) = g.tab(text) {
                        self.command_line.insert_str(&add);
                        self.recompute_ghost();
                    }
                }
            }
            Key::Named(NamedKey::Delete) => self.ghost = None,
            Key::Named(NamedKey::ArrowRight | NamedKey::End) => {
                if let Some(g) = self.ghost.take() {
                    let add = g.rest(&self.command_line.text);
                    self.command_line.insert_str(&add);
                } else if matches!(key, Key::Named(NamedKey::End)) {
                    self.command_line.cursor = self.command_line.text.len();
                } else {
                    self.command_line.move_right();
                }
                self.recompute_ghost();
            }
            key => {
                self.line_edit_key(key);
                self.recompute_ghost();
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// QCOMP2: an external source's output arrived. Stale (another `:` line) results are dropped; an
    /// error goes to the status bar; the ghost is recomputed for what is typed now.
    pub(crate) fn handle_complete_result(&mut self, r: crate::completion::ExtResult) {
        if self.vim_mode != VimMode::Command || !self.complete_cache.accept(&r) {
            return;
        }
        if let Err(msg) = &r.outcome {
            self.show_message(msg);
        }
        self.recompute_ghost();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// T45/T46: the ghost for the line as it is now.
    fn recompute_ghost(&mut self) {
        self.ghost = crate::compute_ghost(&self.command_line.text, self.command_line.cursor, &self.complete_cache);
    }

    /// QSEARCH1 (T22-T25): a key in the `/` line. Enter commits, Esc cancels, the rest edits
    /// the line exactly like the `:` line (`line_edit_key`, so paste is shared).
    pub(crate) fn handle_search_key(&mut self, event: &KeyEvent) {
        self.search_line_key(&event.logical_key);
    }

    /// `handle_search_key` on a bare key (tests call this: a `KeyEvent` cannot be built).
    pub(crate) fn search_line_key(&mut self, key: &Key) {
        match key {
            Key::Named(NamedKey::Escape) => {
                self.vim_mode = VimMode::Normal;
                self.command_line.clear();
            }
            Key::Named(NamedKey::Enter) => {
                let query = std::mem::take(&mut self.command_line.text);
                self.command_line.cursor = 0;
                self.vim_mode = VimMode::Normal;
                self.commit_search(&query);
            }
            Key::Named(NamedKey::Tab) => {}
            key => self.line_edit_key(key),
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// QSEARCH1 (T23, T24): commit `query`: find the matches, make the first one at or below the
    /// top of the screen current (wrapping to the first). An empty query is a cancel (T24).
    /// The screen is brought to the match (QSEARCH2, T23, S4).
    pub(crate) fn commit_search(&mut self, query: &str) {
        if query.is_empty() {
            return;
        }
        self.search.query = query.to_string();
        self.search.highlight = true;
        self.search.matches = match &self.cached {
            Some(laid) => crate::search::find_in_laid(laid, &self.shape_cache, query),
            None => Vec::new(),
        };
        if self.search.matches.is_empty() {
            self.search.current = None;
            self.status_message = format!("Pattern not found: {query}");
            return;
        }
        let n = self.search.matches.len();
        let first = crate::search::first_at_or_below_in(&self.search.matches, self.scroll_offset, &self.scroll_offsets);
        self.search.current = Some(first.unwrap_or(0));
        self.status_message = match first {
            Some(i) => format!("[{}/{n}]", i + 1),
            None => format!("search hit BOTTOM, continuing at TOP [1/{n}]"),
        };
        self.reveal_current_match();
    }

    /// QSEARCH2 (S4): bring the screen to the current match: every scroll region it sits in
    /// (innermost first), then the page. Nothing scrolls where the match is already fully visible.
    pub(crate) fn reveal_current_match(&mut self) {
        let Some(m) = self.search.current.and_then(|i| self.search.matches.get(i)) else { return };
        let page_h = self.cached.as_ref().map_or(0.0, |l| l.h);
        let (changes, page) = m.reveal(&self.scroll_offsets, self.scroll_offset, self.page_size().1 as f32, page_h);
        for (id, node, v) in changes {
            self.set_region_scroll(id, node, v);
        }
        if let Some(p) = page {
            self.set_scroll_now(p);
        }
        // QXSCR1: and horizontally (regions first, then the window).
        let Some(m) = self.search.current.and_then(|i| self.search.matches.get(i)) else { return };
        let page_w = self.cached.as_ref().map_or(0.0, |l| crate::layout::doc_scroll_width(l));
        let (changes, page) = m.reveal_x(&self.scroll_offsets, self.scroll_x, self.page_size().0 as f32, page_w);
        for (id, node, v) in changes {
            self.set_region_scroll(id, node, v);
        }
        if let Some(p) = page {
            self.scroll_x = p;
        }
    }

    /// QSEARCH2 (T15-T17): `n` (`forward`) / `N`: the next / previous match in document order,
    /// wrapping with a message, then the screen follows. With the highlights gone (Esc, a new page)
    /// the kept query runs again first (S6).
    pub(crate) fn search_step(&mut self, forward: bool) {
        use crate::search::{find_in_laid, first_at_or_below_in, last_above_in};
        if self.search.query.is_empty() {
            self.status_message = "No previous search".to_string();
            return;
        }
        let query = self.search.query.clone();
        let mut from_top = false;
        if !self.search.highlight || self.search.matches.is_empty() {
            self.search.matches = match &self.cached {
                Some(laid) => find_in_laid(laid, &self.shape_cache, &query),
                None => Vec::new(),
            };
            self.search.highlight = true;
            self.search.current = None;
            from_top = true;
        }
        let n = self.search.matches.len();
        if n == 0 {
            self.search.current = None;
            self.status_message = format!("Pattern not found: {query}");
            return;
        }
        let top = self.scroll_offset;
        let (next, wrapped) = match (self.search.current, from_top) {
            (Some(c), false) if forward => if c + 1 < n { (c + 1, false) } else { (0, true) },
            (Some(c), false) => if c > 0 { (c - 1, false) } else { (n - 1, true) },
            _ if forward => match first_at_or_below_in(&self.search.matches, top, &self.scroll_offsets) {
                Some(i) => (i, false),
                None => (0, true),
            },
            _ => match last_above_in(&self.search.matches, top, &self.scroll_offsets) {
                Some(i) => (i, false),
                None => (n - 1, true),
            },
        };
        self.search.current = Some(next);
        self.status_message = match (wrapped, forward) {
            (false, _) => format!("[{}/{n}]", next + 1),
            (true, true) => format!("search hit BOTTOM, continuing at TOP [{}/{n}]", next + 1),
            (true, false) => format!("search hit TOP, continuing at BOTTOM [{}/{n}]", next + 1),
        };
        self.reveal_current_match();
    }

    /// QSEARCH3 (S7, T18-T20): Enter in normal mode: a real click on the element holding the
    /// current match, through the same path as a mouse click (the page's handlers first, then the
    /// default action: following a link, pressing a button), never a direct jump to an `href`.
    /// Matched text inside 2+ different links: a warning and nothing else; one link: the first
    /// matched char inside it; none: the first matched char. No current match: nothing.
    pub(crate) fn activate_match(&mut self) {
        let Some(m) = self.search.current.and_then(|i| self.search.matches.get(i)) else { return };
        let nodes: Vec<usize> = m.nodes.iter().copied().filter(|&n| n != 0).collect();
        let x = m.doc_left(&self.scroll_offsets) - self.scroll_x;
        let pos = ((x + 1.0).max(0.0), (m.doc_top(&self.scroll_offsets) - self.scroll_offset + 1.0).max(0.0));
        // (node, nearest link ancestor's id) per matched char.
        let mut links: Vec<usize> = Vec::new();
        let mut first_in: Vec<(usize, usize)> = Vec::new();
        for &n in &nodes {
            let Some(h) = find_node_by_id(&self.document, n) else { continue };
            if let Some(l) = enclosing_link(&h) {
                let id = node_id(&l);
                if !links.contains(&id) {
                    links.push(id);
                    first_in.push((id, n));
                }
            }
        }
        let node = match links.len() {
            0 => nodes.first().copied(),
            1 => first_in.first().map(|f| f.1),
            n => {
                self.status_message = format!("Match spans {n} links");
                return;
            }
        };
        let target = node.and_then(|n| find_node_by_id(&self.document, n));
        let control = target.as_ref().and_then(enclosing_control);
        if control.as_ref().is_some_and(is_disabled_control) {
            return;
        }
        let node_href = target.as_ref().and_then(enclosing_link_href);
        let mut in_control = control.is_some();
        let ok = self.click_activate(target.as_ref(), pos);
        if ok && control.is_none() && let Some(label) = target.as_ref().and_then(enclosing_label) {
            in_control = self.forward_label_click(&label, pos);
        }
        if let (Some(href), true, false) = (node_href, ok, in_control) {
            self.start_navigation(&href);
        }
    }

    /// The editing keys shared by the `:` and `/` lines.
    fn line_edit_key(&mut self, key: &Key) {
        match key {
            Key::Named(NamedKey::Backspace) => self.command_line.backspace(),
            Key::Named(NamedKey::ArrowLeft) => self.command_line.move_left(),
            Key::Named(NamedKey::ArrowRight) => self.command_line.move_right(),
            // Same `mods`-checked-alongside-`logical_key` shape `resolve_
            // binding`'s own `Ctrl`+`d`/`u` arms use, for the same reason:
            // `Ctrl` is excluded from `logical_key`'s own shift-adjustment,
            // so a bare `v` has to be told apart from `Ctrl`+`v` here, not
            // left to `key_insertion_text`'s catch-all below (which would
            // otherwise insert a literal "v" on every paste attempt).
            key if is_paste_chord(key, &self.modifiers) => {
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
        // QPASTE1: the shared clipboard (Wayland data-control first, X11 fallback), cleaned to one line.
        if let Some(text) = crate::forms_text::clipboard_get() {
            let text = clean_paste_text(&text);
            if !text.is_empty() {
                self.command_line.insert_str(&text);
            }
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
            // Takes effect from the next page load (`:refresh`).
            ("set", ["js"]) => set_js_enabled(true),
            ("set", ["nojs"]) => set_js_enabled(false),
            // QAN1d0: takes effect from the next page load (`:refresh`).
            ("set", ["motion"]) => set_reduced_motion(false),
            ("set", ["nomotion"]) => set_reduced_motion(true),
            ("help", []) => self.show_help(),
            // QREG1: `:let @<dst> = @<src>` copies a register (registers.rs).
            ("let", _) => self.let_register(&args.join(" ")),
            // QINC1n: this session's engine counts (updates, restarts, fallbacks, checks, mismatches).
            ("engine", []) => self.status_message = crate::incr_evidence::status_text(),
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

/// QEL1: the App as the rendering-steps host (event_loop.rs). Layout and paint themselves stay in
/// `redraw` (incremental engine, shaping worker, progressive shaping: R64 E15): the style/layout
/// step hands the frame's DOM writes over (`js_sync` -> `layout_dirty` / restyle) and the paint
/// step makes sure the redraw that builds and paints them is requested.
impl crate::event_loop::RenderingHost for App {
    fn engine(&self) -> Option<&crate::js::JsEngine> {
        self.js.as_ref()
    }
    fn run_resize_steps(&mut self) {
        self.sync_js_viewport();
    }
    fn run_scroll_steps(&mut self) {
        let busy = self.scroll_busy();
        if let Some(js) = &self.js {
            if js.qel7_bits() & 3 != 0 {
                self.last_scroll_step = Some(std::time::Instant::now());
            }
            js.run_scroll_steps(busy);
        }
    }
    fn evaluate_media_queries(&mut self) {
        if let Some(js) = &self.js {
            js.run_media_queries();
        }
    }
    fn update_animations(&mut self) {
        if !self.anims_idle() {
            self.anim_wake = self.anim_tick(app_clock_ms());
        }
    }
    fn update_style_layout(&mut self) {
        self.js_sync();
    }
    /// Step 7. QEL2b: ResizeObserver reads this frame's App layout. Shown layout of the DOM as it is
    /// now: its boxes are recorded (when not yet); the frame's layout still to come (DOM writes since
    /// the shown one): the observers wait for it (`install_laid` records it and ends the wait) instead of
    /// forcing a layout of their own. IntersectionObserver as before.
    fn run_observers(&mut self) {
        if self.js.is_some() && crate::ro_geom::active() {
            let (vw, vh) = (self.cached_width, self.cached_height);
            let pending = self.layout_dirty || self.layout_in_flight;
            let shown_current = !pending && !self.hold_old_frame && self.build_stamp == crate::js_geom::render_stamp();
            if crate::ro_geom::current(&self.document, vw, vh) {
                crate::ro_geom::set_wait(false);
            } else if shown_current && let Some(laid) = self.cached.clone() {
                crate::ro_geom::app_layout(&self.document, &laid, self.build_stamp, vw, vh);
            } else {
                crate::ro_geom::set_wait(pending);
            }
        }
        if let Some(js) = &self.js {
            js.observer_hook();
        }
    }
    fn paint(&mut self) {
        if self.layout_dirty
            && let Some(w) = &self.window
        {
            w.request_redraw();
        }
    }
}

/// QM1k1n6b: how long `run_due` may run timers in one event-loop turn: 16 ms
/// while a video is on screen or input is fresh, 50 ms otherwise.
pub(crate) fn timer_slice(input_recent: bool, video: bool) -> std::time::Duration {
    std::time::Duration::from_millis(if input_recent || video { 16 } else { 50 })
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

        // Q33b2w: scripts may start before the first redraw measures the surface, so publish the
        // window's size now (new engines read it; `surface_size` stays 0 so the first redraw still
        // resizes the surface).
        let size = window.inner_size();
        let (vw, vh) = page_viewport_hb((size.width, size.height), self.hbar);
        crate::js::set_window_viewport(vw, vh);
        self.window = Some(window);
        self.surface = Some(surface);
        self.wl_drain = crate::wl_drain::WlDrain::start(event_loop);
        if let Some(d) = &self.wl_drain {
            d.busy(); // this iteration's new_events ran before the drain existed
        }
    }

    /// QY54a4: a loop iteration starts: stamp "busy since" for the wl-drain thread. It stays set
    /// through every callback until the end of `about_to_wait`, so a stall anywhere in between is seen.
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {
        if let Some(d) = &self.wl_drain {
            d.busy();
        }
    }

    /// QY54a2: join the wl-drain thread while winit's display is still alive (winit calls this on a
    /// clean exit and on the ExitFailure path, before it drops the event loop).
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.wl_drain.take();
        // QINC1j: with frame stats on, the window's engine counters (the gate reads them).
        if self.frame_stats.is_some() {
            diag!("scratchy: incr: {}", crate::perf::incr_json());
            diag!("scratchy: {}", crate::incr_evidence::status_text());
            diag!("scratchy: resize-observer: {}", crate::ro_geom::stats_json());
        }
    }

    /// Q33t: run due JS timers, then sleep until the next one (or
    /// indefinitely when none are pending, so timer-less pages never
    /// wake up on their own).
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // (QY54a4: `new_events` marked the wl-drain busy; the stamp is cleared at the very end.)
        // QY54a2: a panic on this thread stopped the drain; we are still running, so it was caught.
        if self.wl_drain.as_ref().is_some_and(crate::wl_drain::WlDrain::stopped_by_panic) {
            self.wl_drain = None;
            self.wl_drain = crate::wl_drain::WlDrain::start(event_loop);
        }
        self.flush_pointer_move();
        let trail_at = self.flush_move_trailing();
        let els = self.sync_node_scroll();
        self.publish_viewport();
        // Tasks: due timers (sliced), then their DOM writes.
        if let Some(js) = &self.js {
            js.set_scroll(self.scroll_x, self.scroll_offset, els);
            js.run_due_slice(Some(timer_slice(self.input_recent(), !self.video.boxes.is_empty())));
        }
        self.js_sync();
        // QEL1: the rendering update (resize -> ... -> animation events -> rAF batch -> style/layout ->
        // observer hooks -> paint), when its frame is due.
        self.rendering_update_if_due();
        let now_ms = app_clock_ms();
        let anim_at = self.anim_wake.filter(|_| !self.anims_idle()).map(|ms| std::time::Instant::now() + std::time::Duration::from_secs_f64(((ms - now_ms).max(0.0) + 1.0) / 1000.0));
        let js_at = self.js.as_ref().and_then(|js| [js.next_deadline(), js.raf_deadline()].into_iter().flatten().min());
        let ctl_at = self.controls_refresh();
        let sb_at = self.scrollbar_hold_tick().into_iter().chain(self.scroll_wake()).min();
        // QSMOOTH1 D6: keep frames coming only while the scroll is easing.
        let scroll_at = self.scroll_target.map(|_| {
            self.request_redraw_now();
            std::time::Instant::now() + std::time::Duration::from_millis(16)
        });
        let stats_at = match &mut self.frame_stats {
            Some(fs) => {
                let now = std::time::Instant::now();
                if now >= fs.next_due() {
                    let ext = crate::frame_stats::FrameStats::take_external();
                    if let Some(line) = fs.roll(now, ext) {
                        fs.emit(&line);
                    }
                }
                Some(fs.next_due())
            }
            None => None,
        };
        // QMEM3d: trim once when the window has been idle (no input, no video) for a while.
        let input_age = self.last_input.map(|t| t.elapsed());
        let video_age = self.last_video_frame.map(|t| t.elapsed());
        if self.idle_trim.should_trim(input_age, video_age, !self.video.boxes.is_empty()) {
            crate::mem_trim::request();
        }
        let idle_at = self.idle_trim.wake_in(input_age, video_age).map(|d| std::time::Instant::now() + d);
        let flow = match [js_at, anim_at, ctl_at, stats_at, trail_at, idle_at, sb_at, scroll_at].into_iter().flatten().min() {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        };
        event_loop.set_control_flow(flow);
        // QY54a4: main is about to sleep in calloop (which reads the Wayland socket itself).
        if let Some(d) = &self.wl_drain {
            d.idle();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::VideoFrame => {
                crate::media_video::wake_handled();
                self.last_video_frame = Some(std::time::Instant::now());
                if let Some(fs) = &mut self.frame_stats {
                    fs.video_wake();
                }
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
            AppEvent::StreamCss(generation, css, sheets, scripts) => {
                if generation != self.nav_generation {
                    return;
                }
                if !css.is_empty() {
                    js_style::set_js_extra_css(&css);
                    self.link_sheets = crate::link_css::LinkSheets::adopt(&self.document, self.base_url.as_ref(), css.len(), sheets);
                    self.extra_css = css;
                    self.layout_dirty = true;
                    self.layout_generation += 1;
                }
                self.js = run_page_scripts(&self.document, &scripts, self.base_url.as_ref());
                self.js_sync();
            }
            AppEvent::NavResult(generation, result) => self.handle_nav_result(generation, result),
            AppEvent::CompleteResult(r) => self.handle_complete_result(r),
            AppEvent::LinkCss(generation, href, css) => {
                if generation == self.nav_generation {
                    self.link_sheets.settle(&href, css);
                    self.sync_link_sheets();
                }
            }
            AppEvent::JsNet(generation, id, result, completed) => self.handle_js_net(generation, id, result, completed),
            AppEvent::JsWs => self.handle_js_ws(),
            AppEvent::ImageFetched(generation, src, image) => self.handle_image_fetched(generation, src, image),
            AppEvent::FontFetched(generation, key, page, face) => self.handle_font_fetched(generation, &key, page, face.map(|f| *f)),
            AppEvent::ShapeResult(generation, laid, shape_cache, incomplete, next_id, adopt) => {
                self.next_para_id = self.next_para_id.max(next_id);
                self.handle_shape_result_job(generation, laid, shape_cache, incomplete, adopt.map(|a| (a, next_id)))
            }
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
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if event_flushes_pointer(&event) {
            self.idle_trim.activity();
            self.last_input = Some(std::time::Instant::now());
            self.flush_pointer_move();
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    let (w, h) = page_viewport_hb((size.width, size.height), self.hbar);
                    crate::js::set_window_viewport(w, h);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(fs) = &mut self.frame_stats {
                    fs.input(crate::frame_stats::InputKind::Key, std::time::Instant::now());
                }
                // QKEYS2: the `:` line takes its keys first (never the page, releases dropped);
                // everything else goes through `route_key` (Input / Normal mode).
                let pressed = event.state == ElementState::Pressed;
                if self.vim_mode != VimMode::Normal {
                    if pressed {
                        self.handle_key_event(&event);
                    }
                } else {
                    self.route_key(&event.logical_key, Some(event.physical_key), pressed, event.repeat, std::time::Instant::now());
                }
                // `:q`/`:quit` (`dispatch_command`) only set this flag: `event_loop.exit()` has to
                // happen here, where there is an `&ActiveEventLoop` (see `App::should_quit`).
                if pressed && self.should_quit {
                    event_loop.exit();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(fs) = &mut self.frame_stats {
                    fs.input(crate::frame_stats::InputKind::Move, std::time::Instant::now());
                }
                // QM1k1n6a: only the newest position of a turn is processed
                // (once, in `about_to_wait`, or first thing when a button,
                // wheel or key event arrives).
                self.queue_pointer_move((position.x as f32, position.y as f32));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(fs) = &mut self.frame_stats {
                    fs.input(crate::frame_stats::InputKind::Button, std::time::Instant::now());
                }
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
                        if let Some(pos) = self.last_mouse_pos
                            && self.scrollbar_press(pos)
                        {
                            self.mouse_down_pos = None;
                            return;
                        }
                        self.mouse_down_pos = self.last_mouse_pos;
                        if let Some(pos) = self.last_mouse_pos {
                            self.press_switches_to_input(pos);
                            self.js_mouse_at("mousedown", pos);
                            self.press_state(pos);
                            self.text_mouse_press(pos);
                        }
                    }
                    ElementState::Released => {
                        if self.sb_drag.take().is_some() || self.sb_hold.take().is_some() {
                            self.mouse_down_pos = None;
                            self.request_redraw_now();
                            return;
                        }
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
                        // QNAV9: the `<a href>` around the clicked element, for a hit
                        // outside any link text (block link padding, an `<img>` in a link).
                        let mut node_href: Option<String> = None;
                        let allowed = match clicked {
                            Some(up) => {
                                let node = self.element_at(up);
                                let target = if node == 0 { None } else { find_node_by_id(&self.document, node) };
                                let control = target.as_ref().and_then(enclosing_control);
                                node_href = target.as_ref().and_then(enclosing_link_href);
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
                                let doc_x = up.0 + self.scroll_x;
                                let doc_y = up.1 + self.scroll_offset;
                                crate::paint::with_scroll_x(self.scroll_x, || {
                                    hit_test_cursor(
                                        laid, doc_x, doc_y, &self.shape_cache, &self.scroll_offsets,
                                        self.scroll_offset, self.page_size().1 as f32,
                                    )
                                })
                                .1
                            }
                            _ => None,
                        };
                        self.mouse_down_pos = None;
                        let href = href.map(str::to_string).or(node_href);
                        if let (Some(href), true, false) = (href, allowed, in_control) {
                            self.start_navigation(&href);
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(fs) = &mut self.frame_stats {
                    fs.input(crate::frame_stats::InputKind::Wheel, std::time::Instant::now());
                }
                self.last_wheel = Some(std::time::Instant::now());
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x * crate::config::WHEEL_LINE_PX, y * crate::config::WHEEL_LINE_PX),
                    MouseScrollDelta::PixelDelta(pos) => (pos.x as f32, pos.y as f32),
                };
                let (dx, dy) = wheel_axes(dx, dy, self.modifiers.shift_key());
                if dy != 0.0 && self.select_popup_wheel(dy) {
                    return;
                }
                // QEL7: the cancelable `wheel` event goes to the page first; preventDefault stops the scroll.
                if self.dispatch_wheel(dx, dy) {
                    return;
                }
                // Routes to the deepest `overflow: scroll`/`auto` region
                // under the last known mouse position, if any — falls
                // through to the page's own top-level scroll (the
                // pre-v62 behavior, unchanged) whenever the cursor isn't
                // over one, or hasn't moved over the window at all yet.
                // QYSCR1c: the chain of regions under the cursor, innermost
                // first; the first one that can use the tick takes it, else
                // it goes to the viewport (unless that is `overflow: hidden`).
                let chain = self.last_mouse_pos.zip(self.cached.as_ref()).map_or_else(Vec::new, |((mx, my), laid)| {
                    crate::paint::hit_test_scroll_chain(
                        laid, mx + self.scroll_x, my + self.scroll_offset, &self.scroll_offsets, self.scroll_offset,
                        self.page_size().1 as f32,
                    )
                });
                // QXSCR1: the x part chains the same way, on its own axis.
                if dx != 0.0 {
                    self.wheel_x(&chain, dx);
                }
                let line_wheel = matches!(delta, MouseScrollDelta::LineDelta(..));
                match chain.iter().find(|info| dy != 0.0 && info.user && region_takes_wheel(info, &self.scroll_offsets, dy)) {
                    Some(info) => {
                        let current = self.scroll_offsets.get(&info.id).copied().unwrap_or(0.0);
                        let next = (current - dy).clamp(0.0, info.max_scroll());
                        self.scroll_offsets.insert(info.id, next);
                        if info.node != 0 {
                            self.node_scroll.insert(info.node, next);
                            self.node_scroll_dirty = true;
                        }
                    }
                    None if dy == 0.0 || self.viewport_user_scroll_blocked() => {}
                    None => {
                        // QSMOOTH1 D2/D3: real notches ease; touchpad pixel deltas stay instant.
                        if line_wheel {
                            self.scroll_by(-dy);
                        } else {
                            self.set_scroll_now((self.scroll_offset - dy).max(0.0));
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

/// QM1k1n6a: events that must see the latest pointer position, so a queued
/// move is processed first (everything but moves, redraws and modifier changes).
pub(crate) fn event_flushes_pointer(event: &WindowEvent) -> bool {
    !matches!(event, WindowEvent::CursorMoved { .. } | WindowEvent::RedrawRequested | WindowEvent::ModifiersChanged(_))
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

/// QM1k1n6c: whether a video frame may be damage-presented into the kept page. Only the
/// kept page's size must match the surface; `layout_dirty` is deliberately not an input.
pub(crate) fn video_damage_ok(kept: (u32, u32), kept_len: usize, surface: (u32, u32)) -> bool {
    kept == surface && kept_len == surface.0 as usize * surface.1 as usize
}

/// QPASTE1: a status-bar paste: newlines, tabs and other control characters become a space and
/// trailing whitespace is dropped (a copied URL often ends in a newline; the bar is one line).
pub(crate) fn clean_paste_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' && chars.peek() == Some(&'\n') {
            continue;
        }
        out.push(if c.is_control() { ' ' } else { c });
    }
    out.truncate(out.trim_end().len());
    out
}

/// QPASTE1: Ctrl-v, Ctrl-Shift-v and Shift-Insert (KEYS.md "Paste").
pub(crate) fn is_paste_chord(key: &Key, mods: &ModifiersState) -> bool {
    match key {
        Key::Character(s) => {
            s.eq_ignore_ascii_case("v") && mods.control_key() && !mods.alt_key() && !mods.super_key()
        }
        Key::Named(NamedKey::Insert) => mods.shift_key() && !mods.control_key() && !mods.alt_key(),
        _ => false,
    }
}
