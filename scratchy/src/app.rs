use super::*;

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
    pub(crate) url: Url,
}

pub(crate) enum AppEvent {
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
    StreamCss(u64, String),
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
        _ => None,
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
pub(crate) const COMMAND_NAMES: &[&str] = &["go", "help", "historyBack", "historyFront", "q", "quit", "refresh", "set"];

/// `:set`'s own one valid argument, completed the same way `COMMAND_
/// NAMES` completes a command name itself — see `command_completions`.
pub(crate) const SET_ARGS: &[&str] = &["novim", "vim"];

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
}

impl TextEditBuffer {
    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub(crate) fn insert(&mut self, c: char) {
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

    pub(crate) fn backspace(&mut self) {
        let Some(prev) = self.text[..self.cursor].chars().next_back() else { return };
        self.cursor -= prev.len_utf8();
        self.text.remove(self.cursor);
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
    /// Did the current layout read the viewport height (R4)?
    pub(crate) reads_viewport_h: bool,
    /// Which elements a `:hover` rule can restyle (R2.4).
    pub(crate) hover_keys: HoverKeys,
    /// Hover restyle throttle (R2.3).
    pub(crate) last_hover_restyle: Option<std::time::Instant>,
    pub(crate) last_wheel: Option<std::time::Instant>,
    pub(crate) hover_flush_scheduled: bool,
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
    /// The `--socket <path>` CLI flag, if given — where a `wire` daemon
    /// is expected to be listening. `None` means the user never opted
    /// in at all; `start_navigation` treats that identically to "gave a
    /// path but nothing's listening there" (see its own docs), just
    /// without even attempting a connection first.
    pub(crate) socket_path: Option<PathBuf>,
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
    pub(crate) image_cache: HashMap<String, Arc<DecodedImage>>,
    pub(crate) scroll_offset: f32,
    pub(crate) scroll_offsets: HashMap<usize, f32>,
}

impl App {
    pub(crate) fn new_static(document: Handle, proxy: EventLoopProxy<AppEvent>, socket_path: Option<PathBuf>) -> Self {
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
            last_mouse_pos: None,
            mouse_down_pos: None,
            interaction: InteractionState::default(),
            state_rules: 0,
            hover_node: 0,
            pending_fragment: None,
            keep_scroll_on_rebuild: false,
            restyle_requested: false,
            hold_old_frame: false,
            reads_viewport_h: true,
            hover_keys: HoverKeys { any: true, ..Default::default() },
            last_hover_restyle: None,
            last_wheel: None,
            hover_flush_scheduled: false,
            proxy,
            shaping_requests,
            base_url,
            history: Vec::new(),
            history_index: 0,
            nav_generation: 0,
            layout_generation: 0,
            socket_path,
            extra_css: String::new(),
            image_cache: HashMap::new(),
            vim_enabled: true,
            vim_mode: VimMode::Normal,
            command_line: TextEditBuffer::default(),
            modifiers: ModifiersState::empty(),
            completion: None,
            should_quit: false,
            pre_help_page: None,
        }
    }

    pub(crate) fn new_streaming(proxy: EventLoopProxy<AppEvent>, socket_path: Option<PathBuf>) -> Self {
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
            last_mouse_pos: None,
            mouse_down_pos: None,
            interaction: InteractionState::default(),
            state_rules: 0,
            hover_node: 0,
            pending_fragment: None,
            keep_scroll_on_rebuild: false,
            restyle_requested: false,
            hold_old_frame: false,
            reads_viewport_h: true,
            hover_keys: HoverKeys { any: true, ..Default::default() },
            last_hover_restyle: None,
            last_wheel: None,
            hover_flush_scheduled: false,
            proxy,
            shaping_requests,
            base_url: None,
            history: Vec::new(),
            history_index: 0,
            nav_generation: 0,
            layout_generation: 0,
            socket_path,
            extra_css: String::new(),
            image_cache: HashMap::new(),
            vim_enabled: true,
            vim_mode: VimMode::Normal,
            command_line: TextEditBuffer::default(),
            modifiers: ModifiersState::empty(),
            completion: None,
            should_quit: false,
            pre_help_page: None,
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
            surface
                .resize(NonZeroU32::new(width).unwrap(), NonZeroU32::new(height).unwrap())
                .unwrap();
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
            let tree = build_layout_tree_with_resources(&self.document, width as f32, Some(height as f32), &self.extra_css, &self.image_cache);
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
            self.layout_generation += 1;
            Self::queue_shaping_request(&self.shaping_requests, tree, width as f32, height as f32, self.layout_generation, reuse);
        } else if need_rebuild {
            window.request_redraw();
        }

        let mut pixels = surface.buffer_mut().unwrap();
        for pixel in pixels.iter_mut() {
            *pixel = 0x00FFFFFF;
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

        if self.vim_mode == VimMode::Command {
            paint_command_bar(&self.command_line, &mut self.font_system, &mut self.swash_cache, &mut pixels, width, height);
        }

        window.pre_present_notify();
        pixels.present().unwrap();
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
        thread::spawn(move || {
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
                            eprintln!(
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
    /// `image_cache` is cleared unconditionally for the same reason: a
    /// new document starts owing nothing to the previous one's fetched
    /// images (any background fetch still in flight for the *old* page
    /// is harmless — `AppEvent::ImageFetched`'s own `nav_generation`
    /// check drops it before it could repopulate this cache anyway).
    pub(crate) fn replace_document(&mut self, document: Handle, extra_css: String) {
        self.document = document;
        self.interaction = InteractionState::default();
        set_interaction(&self.interaction);
        self.hover_node = 0;
        self.extra_css = extra_css;
        self.image_cache.clear();
        self.layout_dirty = true;
        self.layout_generation += 1;
        self.cached = None;
        self.restyle_requested = false;
        self.hold_old_frame = false;
        self.shape_cache.clear();
        self.scroll_offsets.clear();
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
        self.replace_document(parse_html(&html), String::new());
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
                image_cache: self.image_cache.clone(),
                scroll_offset: self.scroll_offset,
                scroll_offsets: self.scroll_offsets.clone(),
            });
        }
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
        self.image_cache = saved.image_cache;
        self.scroll_offsets = saved.scroll_offsets;
        self.scroll_offset = saved.scroll_offset;
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

    pub(crate) fn update_hover(&mut self, node: usize) {
        if node == self.hover_node {
            return;
        }
        self.hover_node = node;
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
        self.set_interaction_state(st);
    }

    /// Mouse press: element + ancestors become `:active`, and the nearest
    /// focusable ancestor-or-self takes focus (not `:focus-visible`).
    pub(crate) fn press_state(&mut self, pos: (f32, f32)) {
        let node = self.element_at(pos);
        let target = if node == 0 { None } else { find_node_by_id(&self.document, node) };
        let mut focus = None;
        let mut cur = target.clone();
        while let Some(n) = cur {
            if is_focusable(&n) {
                focus = Some(n);
                break;
            }
            cur = parent_handle(&n);
        }
        let mut st = self.interaction.clone();
        st.active = target.map(|h| ancestor_ids(&h).into_iter().collect()).unwrap_or_default();
        self.set_interaction_state(st);
        self.set_focus_element(focus.as_ref(), false);
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

    /// Enter on a focused link follows it.
    pub(crate) fn activate_focus(&mut self) {
        let Some(f) = self.interaction.focus else { return };
        let Some(h) = find_node_by_id(&self.document, f) else { return };
        if matches!(element_tag(&h), "a" | "area") {
            let href = element_attr(&h, "href").map(|v| v.to_string());
            if let Some(href) = href {
                self.start_navigation(&href);
            }
        }
    }

    pub(crate) fn start_navigation(&mut self, href: &str) {
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
        self.push_history(url.clone());
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
        if self.history.is_empty() {
            if let Some(current) = self.base_url.clone().filter(|b| *b != url) {
                self.history.push(current);
            }
        } else {
            self.history.truncate(self.history_index + 1);
        }
        self.history.push(url);
        self.history_index = self.history.len() - 1;
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
        self.history_index = new_index;
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
        self.pending_fragment = url.fragment().map(str::to_string);
        let mut url = url;
        url.set_fragment(None);
        self.nav_generation += 1;
        let generation = self.nav_generation;
        self.show_message("Loading…");
        let proxy = self.proxy.clone();
        let socket_path = self.socket_path.clone();
        thread::spawn(move || {
            let request_text = build_get_request_text(&url);
            let url_for_log = url.clone();
            let outcome: Result<FetchedPage, String> = match &socket_path {
                Some(path) => {
                    let fetch_start = std::time::Instant::now();
                    let wire_result = fetch_raw_via_wire(path, &request_text);
                    if timing_enabled() {
                        eprintln!(
                            "scratchy: TIMING page_fetch={:>7.1}ms {url_for_log}",
                            fetch_start.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    match wire_result {
                        WireFetchRaw::Responded { content_type, body } => match dispatch_wire_response(content_type, body) {
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
                                let extra_css = process_external_resources(&proxy, path, &url, &html, generation);
                                if timing_enabled() {
                                    eprintln!(
                                        "scratchy: TIMING stylesheets={:>7.1}ms {url_for_log}",
                                        css_start.elapsed().as_secs_f64() * 1000.0
                                    );
                                }
                                Ok(FetchedPage { html, extra_css, url })
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
            self.handle_command_line_key(event);
            return;
        }
        if !self.vim_enabled {
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
            ("go", [url]) => self.start_navigation(url),
            ("historyBack", []) => self.navigate_history(-1),
            ("historyFront", []) => self.navigate_history(1),
            ("refresh", []) => self.refresh(),
            ("set", ["vim"]) => self.vim_enabled = true,
            ("set", ["novim"]) => self.vim_enabled = false,
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

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
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
                    }
                }
            }
            AppEvent::StreamCss(generation, css) => {
                if generation != self.nav_generation || css.is_empty() {
                    return;
                }
                self.extra_css = css;
                self.layout_dirty = true;
                self.layout_generation += 1;
            }
            AppEvent::NavResult(generation, result) => {
                // A result from a navigation the user has since clicked
                // past (a second link, before the first one's fetch
                // finished) — drop it silently rather than clobbering
                // whatever's already newer on screen. See `AppEvent::
                // NavResult`'s own docs.
                if generation != self.nav_generation {
                    return;
                }
                match result {
                    Ok(FetchedPage { html, extra_css, url }) => {
                        let dom = new_parser().read_from(&mut html.as_slice()).unwrap();
                        self.base_url = Some(resolve_page_base_url(&dom.document, url));
                        self.replace_document(dom.document, extra_css);
                    }
                    Err(message) => self.show_message(&message),
                }
            }
            AppEvent::ImageFetched(generation, src, image) => {
                // Same staleness guard `NavResult` already uses — a slow
                // image fetch that only finishes after the user has since
                // navigated elsewhere must never populate the *new*
                // page's `image_cache` just because it happens to share
                // the same raw `src` string. See `AppEvent::ImageFetched`'s
                // own docs.
                if generation != self.nav_generation {
                    return;
                }
                let Some(image) = image else { return };
                self.image_cache.insert(src, Arc::new(image));
                // A newly-arrived image can change this element's own
                // intrinsic size (no explicit CSS/HTML width/height —
                // see `resolve_img`), so the whole tree needs a real
                // rebuild, not just a repaint — the same `layout_dirty`
                // flag `redraw` already checks before deciding whether to
                // call `build_layout_tree_with_resources` again.
                self.layout_dirty = true;
                self.layout_generation += 1;
            }
            AppEvent::ShapeResult(generation, laid, shape_cache, incomplete) => {
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
                // Replaced wholesale on every tick this generation's
                // worker reports, not merged — see `App::shape_cache`'s
                // own docs for why that's always safe (every entry here
                // is a strict superset of the last delivery's, same
                // generation, same underlying tree).
                self.shape_cache = shape_cache;
                self.cached = Some(*laid);
                // Q31b: a fragment from a cross-page navigation waits for
                // the first layout that contains its target.
                if let Some(frag) = self.pending_fragment.clone()
                    && self.apply_fragment(&frag)
                {
                    self.pending_fragment = None;
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
                if event.state == ElementState::Pressed {
                    if self.vim_mode != VimMode::Command {
                        match event.logical_key {
                            Key::Named(NamedKey::Tab) => {
                                self.tab_focus(self.modifiers.shift_key());
                                if let Some(w) = &self.window {
                                    w.request_redraw();
                                }
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
                // `position` is window/screen space; `Laid.x`/`.y` are
                // document space (scroll isn't applied until paint time),
                // so the same `+ scroll_offset` adjustment `paint` itself
                // makes (in reverse) is needed here too.
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
                        self.mouse_down_pos = self.last_mouse_pos;
                        if let Some(pos) = self.last_mouse_pos {
                            self.press_state(pos);
                        }
                    }
                    ElementState::Released => {
                        self.release_state();
                        // Computed into a plain, owned `Option<&'static
                        // str>` (not borrowed from `self`) *before*
                        // calling `start_navigation` below, which needs
                        // `&mut self` — `laid`/`shape_cache`/
                        // `scroll_offsets` are only borrowed for this
                        // one `match`, not held across that call.
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
                        if let Some(href) = href {
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
