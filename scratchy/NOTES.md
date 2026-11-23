# scratchy — session notes

Not part of the prompt log (`PROMPTS.md`) — this is working notes for
picking the project back up later. `PROMPTS.md` stays the append-only
record of exact feature prompts; this file is free-form and can be
edited/pruned as the project evolves.

## Index: source files added 2026-11-14..16 (audit 2026-11-16)

The module list in "Source layout (Q38)" below predates these. Each is
explained in its own section near the end of this file.

- `src/js.rs`: the QuickJS engine, PRELUDE (window, events, timers,
  Navigator, matchMedia, location, document.cookie accessor) and the
  evaluation order of the JS preludes below.
- `src/js_dom.rs`: DOM_PRELUDE and the `__d_*` natives (nodes, mutation
  hooks, custom elements, MutationObserver, IO/RO, template.content,
  second documents and iframes).
- `src/js_style.rs`: `getComputedStyle` (QA16), cascade-only, no layout.
- `src/js_engine_v8.rs` (mounted as `js_engine`): the engine seam (QV8-2, V8 since QV8-6). The only file that names `v8`: natives (`native!`), values (`Bytes`, `JsObject`), compile/run, `Engine` (budget, job drain, module loader), error formatting.
- V8 startup snapshot (QV8-4, `js_engine_v8.rs` `Snapshot`, `js::bake_engine`, `js::startup_engine`): page engines are deserialized from a snapshot of scratchy's own startup over an empty document, built in-process on a background thread (`warm_up`; blocking `ensure_snapshot` in tests and the one-shot report modes; `SCRATCHY_V8_SNAPSHOT=0` disables; a non-empty `SCRATCHY_V8_FLAGS` also skips it with one stderr line, QOBS2: `--perf-basic-prof --interpreted-frames-native-stack` + snapshot jumped to a stale code address from `Builtins_LoadIC_NoFeedback`, SIGSEGV). Natives are baked as functions whose data is their arena index (`register` re-attaches the closure by name); per-page state is applied by `__scratchy_apply_env` (+ `__scratchy_env_hooks`, `__media_sweep`) after attaching. A prelude must NOT read page state (env, document, clock) at top level except through those hooks.
- `src/js_modules.rs`: ES module graph pre-fetch and import maps (QY15).
- `src/stack.rs` (QSTK1): `stack::grow(|| ...)` runs a recursive walk on a further stack segment
  (`psm::on_stack`, mmap'd with a guard page, 2 kept per thread for reuse) when less than 512 KB of
  the current stack is left. The per-nesting-level walks call it once per level through a thin
  wrapper around a `*_body` fn: `build_box_for_element`, `measure_inner`, `walk_negative_z`,
  `paint_inner`, `hit_test_inner`. Before it, ~375 nested elements overflowed the 8 MB main thread
  (headless, window, and forced layout from a V8 native all run there) and ~150 the 2 MB shaping
  worker (now spawned with 32 MB). A NEW recursive walk over the box/laid/DOM tree with a frame
  over ~400 bytes a level needs the same wrapper (`src/tests/deep_nesting_tests.rs` lays out a
  5 000-deep page on a 2 MB test thread; find the overflowing frame with gdb on that test).
- `src/wl_drain.rs` (QY54a2, design REGRESSIONS.md R57): the `wl-drain` THREAD, started in
  `App::resumed` after the window exists and joined in `App::exiting`. It reads winit's Wayland socket
  while the main thread is busy (a long callback, a forced layout), so the compositor never fills its
  send buffer and drops scratchy (R56: ~190-225 unread motion events). It holds a second
  `wayland_client::Connection` made from winit's `wl_display*` (`Backend::from_foreign_display`, the
  only `unsafe`; private, always-empty queue) and loops prepare_read / poll(socket, wake pipe) /
  `read_without_dispatch`. It NEVER dispatches: libwayland files each event on its owner's queue
  (winit's), and the main thread dispatches as before, so input order, coalescing and hit-testing are
  unchanged. It stops at the first error or POLLHUP (the main thread reports the loss as before, QB1),
  on `stop` (flag + pipe byte, join), and when the starting thread panics (a chained panic hook joins it
  before an unwind can drop the display; `about_to_wait` restarts it if the panic was caught).
  `SCRATCHY_WL_DRAIN=0` disables it (A/B switch); X11 gets no thread. Gate harness: tools/wl-flood.
- `src/js_net.rs`: page-JS network queue `NetState`, `__net_send`,
  CORS/preflight handling, cookie control natives (QF7, QF13, QF14b).
- `src/net_policy.rs`: `RequestPolicy` (every network limit and policy
  flag, `PrivatePolicy` for `private_targets`/`file_targets`), CORS
  helpers, beacon log, wire cap and cookie header builders (QF5, QF14b).
- `src/url_prelude.js`: `URL`/`URLSearchParams` over `__url_parse` (QY9).
- `src/globals_prelude.js`: history, AbortController, MessageChannel,
  Storage, structuredClone, TextEncoder/TextDecoder, rejection tracking
  (QY13, QY14, QY15b).
- `src/js_storage.rs` (QSTASH2): localStorage over `src/stash.rs` (STASH.md). `--state-dir DIR` /
  `SCRATCHY_STATE_DIR` / `$XDG_STATE_HOME/scratchy` opens a thread-local backend (stash + writer thread)
  in window mode; `--js-report` only with an explicit `--state-dir`; `--wpt`, `--perf-report` and tests
  never (in memory). At page start the prelude's `__scratchy_ls_fill` pulls the http(s) origin's pairs
  (`__scratchy_ls_load`, UTF-16 via `js_engine::Utf16`, never a UTF-8 conversion); setItem/removeItem/
  clear record ops (`__scratchy_ls_op`, after the JS-side 5 Mi code-unit quota check) and
  `JsEngine::storage_end_task` (end of `eval_as` / `drain_after_task`) hands one task's ops to the
  writer as one commit. A load first flushes the writer, so the next page reads the last one's writes.
  Same-origin iframe realms share the parent's areas; opaque origins stay in memory.
- `src/intl_prelude.js`: minimal en-US `Intl` (QY16).
- `src/cssom_prelude.js`: constructable stylesheets,
  `document.adoptedStyleSheets` (QSD4).
- `src/fetch_prelude.js`: Blob/File/FileReader, FormData, Headers/
  Request/Response/ReadableStream, `fetch`, `XMLHttpRequest`,
  `sendBeacon` (QF2-QF4, QF8-QF10).
- wire: `wire/src/cookies.rs` (the cookie jar) and the `X-Wire-*`
  protocol in `wire/NOTES.md`.

## Where things stand

v1 through v5 are built (static render → streaming stdin → CSS cascade →
box model/layout → mouse-wheel scrolling), followed by a round of
performance and correctness fixes discovered by testing real pages
(see commit log from `a0d6f23` onward — not tagged as a new version
since they're fixes to v4/v5's promised behavior, not new features):

- Scroll-triggered full relayout (fixed: layout is cached and only
  rebuilt when the document or width actually changes; scrolling just
  repaints at a new offset).
- A continuous redraw loop that fired forever with nothing triggering
  it (root cause: client-side window decorations marking themselves
  dirty, independent of our content — fixed by disabling decorations,
  which scratchy doesn't need anyway; compounded by calling
  `surface.resize()` unconditionally every frame — also fixed).
- Multi-second-to-tens-of-seconds freezes on real pages under full
  `Shaping::Advanced` (required for correct bidi/complex-script
  text — `Shaping::Basic` was measured and explicitly rejected, it
  breaks Arabic and other non-Latin content). Fixed with a resumable,
  time-budgeted layout pass that shapes a bit more each frame and
  caches each paragraph's shaped result by a stable id, so the window
  never blocks for more than one frame's budget at a time.
- A real box-tree bug, not just a perf issue: an element that looked
  inline at the top level but had a block element nested deeper inside
  (routine in real markup — e.g. a `<div>` wrapping a nav `<ul>`) was
  silently flattened into one giant text run instead of generating its
  own box. On a real Wikipedia page this merged the entire navigation
  chrome into one ~17,000-character run that alone took 7+ seconds to
  shape. Fixed by unifying block/inline tree-building into one
  recursive walk that respects block boundaries at any depth.
- The very first layout session was committing to a width the tiling
  window manager was about to immediately change, wasting one full
  shaping pass. Fixed by waiting for the width to hold steady across
  two consecutive frames before committing to a layout session at all.

Net result on a real, large Arabic Wikipedia article: 30s hard freeze
→ a few seconds, fully responsive and progressively rendering the
whole time.

v6 through v10 then built out more of the rendering side itself:
font-size/weight/style (v6), default heading sizing (v7), list
markers (v8), default list margin/padding (v9), and — since v4/v7/v9
each added their own hardcoded per-tag Rust table — v10 collapsed all
three into one `default.css` (embedded via `include_str!`, parsed
through the same v3 cascade, prepended before the document's own
rules). Also fixed a latent bug found while testing v6: cosmic-text's
`FontSystem::new()` hardcodes generic font families ("Open Sans" etc.)
that may not be installed, silently falling back per-glyph to
whatever font merely contains that glyph (measured: emoji font for
digits). Fixed by building our own `fontdb::Database` and asking
`fc-match` for the system's real installed default instead.

`default.css` was then rewritten properly against the real spec: fetched
the WHATWG rendering chapter in full, cross-checked against Firefox's
actual `html.css` (used only as a reference, not copied in — it's
MPL-2.0 and full of `-moz-*` syntax/forms/media rules scratchy has no
use for), and hand-curated the result. Most of it uses the real
property/selector names (logical properties, `display: table`, etc.)
even where currently inert, so it activates for free once that support
exists, rather than inventing scalar workarounds to revert later —
confirmed safe by hand (an unparseable rule just drops, doesn't poison
the rest of the sheet). Net effect: `ul`/`ol` lost the tuned-but-wrong
24px compromise and now have *zero* visible spacing again until
per-side margin/padding exists, which is the intended tradeoff, not a
regression to chase.

See `TODO.txt` for the running list of gaps this surfaced (per-side
box model, missing units/properties, etc.) — that file is now the
place to check/add to, not this prose.

v12 picked off `TODO.txt`'s first item: inline `background-color`
(`SpanStyle` gained a `background_color` field, threaded down
alongside — but separately from — `InheritedStyle` since background
doesn't truly inherit; painted as per-line rects behind each span's
glyphs via cosmic-text's `Attrs::metadata` tagging). Verified visually
against a test page with wrapped/nested `<mark>` spans via `grim`
screenshot — merges correctly across line wraps, nested elements
without their own background keep showing the ancestor's.

v13 picked off the next item: the per-side box model (`Sides { top,
right, bottom, left }` replacing the old single-scalar margin/padding,
logical property names expanding to physical sides at parse time).
Partway through this session `grim` started failing
(`failed to copy output eDP-1`) even run standalone with no scratchy
involved, and scratchy's own window stopped reaching winit's `Resumed`
event — a sandbox/compositor issue, not something this change caused
(confirmed: `grim` alone fails identically; `measure`'s per-side math
is pure arithmetic with no window dependency). Verified instead with a
temporary `#[cfg(test)]` module calling `resolve_style` on parsed DOM
nodes directly and asserting per-side values land correctly with no
cross-axis leakage — removed before finishing, same discipline as any
other scratch instrumentation in this project (see `set_px`/
eprintln-timing precedent below). If a future session hits the same
`grim`/winit stall, that's the repro to know about.

v14 added `em` length units (checked `default.css` first — it never
uses `rem` or `%`, so only `px`/`em` were implemented, not all three
TODO'd unit gaps). `Declarations`' length fields became a `Length`
enum (`Px`/`Em`), resolved in `resolve_style` once the cascade winner
is picked — `font-size` resolves first (against the *parent's*
font-size), then margin/padding/border-width resolve their own `em`
against that element's *own* just-computed font-size. This activates
most of `default.css`'s heading/paragraph/list margin-block values
that v13 left inert. Verified the same headless way as v13 (`grim`
still stalled) — temporary test asserting `h1`'s `.67em` resolves
against its own 32px, not the inherited 16px root default.

v15 picked off the next item: table layout. Deliberately scoped below
full CSS auto table layout — equal-width columns (no per-column
content-based sizing), no colspan/rowspan, no border-collapse — since
nothing exercised those yet; see `TODO.txt` for the honest list of
what's simplified. Structurally: `thead`/`tbody`/`tfoot`/`colgroup`/
`col` are transparent wrappers walked through to find `tr`s, each row
is an ordinary block-stacked child of the table whose own content
(`BoxContent::TableRow`) lays its cells out horizontally, and a nested
`<table>` correctly gets its own independent table-building pass
rather than having its row flattened into the outer table's. Verified
headlessly, this time running the *full* `build_layout_tree` +
`measure` pipeline (not just `resolve_style` like v13/v14) against a
fixture with a nested table-in-a-cell case — `grim`/winit are still
stalled in this sandbox.

v16 picked off `font-family`: only the 5 CSS generic keywords (checked
`default.css` first, same as v14 — it only ever uses bare
`monospace`, never a named family/fallback list, so that's all that's
implemented). Genuinely CSS-inherited, unlike v13's background-color,
so it threads through `InheritedStyle` the same simple way color/
font-size/weight/style already do. Verified past just the cascade,
matching the diligence the v6 `fc-match` bug called for: confirmed a
sans-serif span and a monospace span actually resolve to two
different `fontdb::ID`s when shaped side by side, not just two
different enum values. `grim`/winit remain stalled in this sandbox
(v13-v16 now) — worth checking fresh at the start of a future session
in case it's since recovered, but not spending more time on it this
session; headless `resolve_style`/`measure`/`shape_spans` tests have
caught everything they needed to so far.

v17 picked off `text-decoration` — turned out cosmic-text already has
full underline/strikethrough/overline support built into `Attrs`
(`Buffer::draw`, already called by `paint_buffer`, already calls its
own `render_decoration` internally using real font metrics), so this
was almost entirely CSS parsing + propagation, no new paint code.
Threaded the same ambient-propagation way as v12's background-color
(not CSS-inherited, but a UA still draws an ancestor's decoration
through descendants that don't override it) — same `collect_children`
mechanism, now carrying two propagated values instead of one.
Verified past the cascade into cosmic-text's own computed decoration
spans (not just scratchy's own `ComputedStyle` fields), confirming
propagation into a nested `<b>` and an explicit `text-decoration:
none` opt-out both actually affected what cosmic-text drew. `grim`/
winit remain stalled (v13-v17).

v18 picked off `white-space: pre`. Genuinely CSS-inherited (unlike
v12/v17's ambient-propagation pattern), so it threads through
`InheritedStyle` the simple way, same as v16's font-family. Two
effects, both gated on the same field: text extraction skips
`normalize_whitespace` entirely for `pre` content (byte-exact, no
fabricated trailing space), and `shape_spans` calls cosmic-text's own
`buffer.set_wrap(Wrap::None)` — decided at the box level (from the
anonymous inline box's own `ComputedStyle`), not per-span, since a
`<pre>`'s content is always its own box anyway. Verified through the
full pipeline at a deliberately narrow width, confirming cosmic-text's
own shaped `Buffer` kept exactly one layout run per source line
instead of wrapping. `grim`/winit remain stalled (v13-v18 now).

v19 picked off `list-style-type`, and along the way found a real,
previously-invisible bug worth flagging prominently: `default.css`'s
`:is(...)` selectors (the nesting-depth circle/square rules, *and*
v9's "nested lists have no top/bottom margin" rule) had never actually
matched anything, ever — `selectors` crate's `parse_is_and_where()`
defaults to `false` and `OurSelectorParser` never overrode it, so
every `:is(...)` selector silently failed to parse and the whole rule
dropped, same as any other unparseable selector. One-line fix
(`fn parse_is_and_where(&self) -> bool { true }`). The nested-margin
rule's breakage had been invisible since v9 because em-unit support
(v14) was the first thing that would have made its `margin-block: 0`
override visually distinguishable from "rule didn't match" at all —
worth remembering if something *else* written with real-spec selector
syntax ever seems to silently not apply. Also worth remembering: a
test that fails isn't always the code's bug — my first version of this
session's verification test called `resolve_style(li, rules,
ROOT_STYLE)` directly on a deeply-nested element, which skips every
real ancestor's cascaded values; had to fix the test (thread
`.inherited()` down the actual ancestor chain) before trusting what it
reported.

v20 closed out `TODO.txt`'s last item: `criterion`/`cargo bench`.
Required splitting `src/main.rs` into `src/lib.rs` (the engine, `pub
fn run()` plus a small, deliberately minimal `pub` surface — not a
blanket sweep) + a 3-line `src/main.rs` wrapper, since a `benches/`
crate can't reach into a binary's `main.rs` at all. `benches/
layout.rs` benchmarks `build_layout_tree` and `measure` separately
against a synthetic 200-paragraph+list+table fixture (no real-page
fixture available — still no bundler tool, see "Priority going
forward" below); confirmed it actually runs and produces real numbers
(~2.9ms / ~24.5ms), not just that it compiles. `cargo bench --bench
layout` (full run) or `cargo bench --bench layout -- --quick` (fast
sanity check) going forward, instead of hand-editing `main.rs` with
temporary `eprintln!`/`Instant` timing for every performance question.

**`TODO.txt` is now empty** (every item from the start of this
session resolved: inline background-color, the per-side box model,
`em` units, table layout, `font-family`, `text-decoration`,
`white-space: pre`, `list-style-type`, and now this) — next session
should treat the three narrower residual gaps noted inline in v14/
v15/v16/v17's `TODO.txt` wording (rem/%/unitless, table auto-layout/
colspan/border-collapse, named font families) as the closest thing to
a backlog, plus whatever a real test page surfaces fresh.

**`grim`/screenshot verification works again** — it turned out to be
the sandbox's monitor being powered off, not a real compositor/winit
bug (user's own diagnosis). Confirmed by re-running the v12-v20 test
page and screenshotting it for real: everything rendered as the
headless tests had already said it would, *except* one thing the
headless tests hadn't covered — `border: 2px solid #884400` (the
shorthand) rendered with no visible border at all, because only the
`border-width`/`border-color` longhands were ever parsed. Caught by
eye, confirmed with a quick headless test, filed as a `TODO.txt` item
(resolved in v21, below). Worth remembering: headless unit tests only
catch what they're written to check — a real rendered page can still
surface a gap nobody thought to assert on.

Also did a full HTML-rendering feature-gap survey (user asked "what
features are missing in regards to rendering html?") and wrote the
whole answer into `TODO.txt` as grouped, actionable items: `border`
shorthand, inert `vertical-align`/`text-align`, no positioning model
(float/position/z-index), no flexbox/grid, a cluster of unimplemented-
but-parsed-or-dropped CSS properties (cursor/line-height/letter-
spacing/opacity/box-shadow/border-radius/outline/visibility/overflow/
border-style/relative font sizes), no CSS custom properties, no
at-rules at all (`@media`/`@font-face`/`@keyframes`/...), stubbed-out
stateful pseudo-classes (`NonTSPseudoClass` is an empty enum), and no
forms/replaced-elements/JavaScript (consistent with "rendering, not
networking" below — most of these need either networking or a design
decision nobody's made yet).

User then asked to pick items off that survey autonomously, one
commit+push per feature, skip-and-note anything needing
clarification. **v21 resolved the `border` shorthand** (see
`PROMPTS.md`) — straightforward, no clarification needed, same
lenient "unparseable component skipped, not whole-value-dropped"
philosophy as `parse_text_decoration`.

**v22 resolved `text-align`.** Worth remembering for next time a
layout-affecting property comes up: check whether cosmic-text already
has it before hand-rolling geometry — it did here (`Buffer::
set_rich_text`'s `Option<Align>` parameter), the same way
`text-decoration` (v17) turned out to already be built in. Also
correctly categorized this one as real CSS inheritance (`InheritedStyle`,
like `white-space`/`list-style-type`), not the ambient-propagation
pattern `background-color`/`text-decoration` use — worth checking the
actual CSS spec's inherited-properties list before wiring a new
property through either path, since getting it backwards would be an
easy, quiet bug (e.g. inheriting something that shouldn't, or letting
a nested element silently lose a text-align it should've kept).
`vertical-align` is now the only property left in that TODO.txt item.

**v23 resolved `vertical-align` (`sub`/`super` only)**. Unlike v22,
cosmic-text has no built-in API for baseline shifting, and
`Buffer::draw()`'s convenience closure gives no per-glyph hook — had
to replace it in `paint_buffer` with a hand-copy of its own internals
(`LegacyRenderer`/`render_decoration`/`LayoutGlyph::physical` are all
public in cosmic-text, so this is "assembling the same pipeline by
hand," not reaching into private internals), adding a y-shift looked
up per glyph via `glyph.metadata` — the same mechanism
`paint_span_backgrounds` (v12) already uses to recover per-span data
after shaping flattens everything into one `Buffer`. Correctly
categorized this one as *not* inherited (unlike `text-align`) but
still needing the *ambient* propagation `background-color`/
`text-decoration` use, so a `<b>` nested inside `<sub>` still renders
shifted without needing its own `vertical-align` declaration. Verified
down to actual rasterized pixels (not just layout math) — rendered the
same glyph via `paint_buffer` at `Baseline` vs `Sub` into real pixel
buffers and diffed the topmost ink row.

With v23, every item from the original post-v20 feature-gap survey's
first two entries (`border` shorthand, vertical-align/text-align) is
resolved. The remaining survey items (positioning model, flexbox/grid,
the misc CSS property cluster, custom properties, at-rules, stateful
pseudo-classes, forms/replaced-elements/JS) are each considerably
bigger — next session should expect some of them to hit genuine design
decisions worth flagging rather than guessing (e.g. what `@media`
breakpoints should even mean for a window-resizable renderer with no
"device" concept, or how deep a flexbox subset is worth implementing).

**v24 picked `line-height` out of the misc-CSS-properties cluster** —
well-scoped on its own, with one real subtlety worth getting exactly
right: `line-height`'s inherited value must stay *symbolic* (a bare
number or `normal`, not a flattened pixel number) all the way down the
tree, because unlike every other length-ish property here, a
`<number>`/`normal` line-height re-resolves against each *descendant's
own* font-size, not the declaring element's. Got this wrong on the
first pass of reasoning (almost flattened it to a pixel value in
`InheritedStyle` like `font_size` itself does) before realizing that
would visibly squish every `h1`-`h6` the moment someone wrote `body {
line-height: 1.5 }` — a near-ubiquitous real-world CSS pattern, not a
contrived edge case. Caught before shipping by writing the test first
and thinking through what it should assert, not after. Worth
remembering: before resolving ANY inherited property to a concrete
value at cascade time, check whether the CSS spec's inheritance model
for that specific property is "inherit the computed pixel value"
(most properties) or "inherit the specified/symbolic value,
re-computed per descendant" (line-height's `<number>` case is the
textbook example of the latter, and the kind of thing that's easy to
get wrong by pattern-matching on how `font-size`/`margin`/etc. already
work in this codebase).

**v25 resolved `visibility: hidden`.** Correctly categorized as real
CSS inheritance (not ambient propagation, unlike `vertical-align`) —
the entire point of `visibility` as a distinct property from `display:
none` is that a descendant can un-hide itself by declaring `visibility:
visible`, and real inheritance already gives that for free with zero
extra wiring (unlike the ambient-propagation properties, which need
their own explicit "did this element override it" parameter threaded
through `collect_children`). Worth remembering as the general rule
this and v22/v24 all reinforce: **check whether a descendant can
override a property back to something other than what an ancestor
declared** — if yes, it's real inheritance; if the only way to change
it is to re-declare it on every element that needs a different value
(like `background-color`), it's ambient propagation. `text-decoration`
is the one property here that's genuinely neither by spec (not
inherited at all) but still gets the ambient treatment, because a UA
visually draws an ancestor's decoration through un-opted-out
descendants — that one was a deliberate, documented exception, not a
default to copy for new properties.

**v26 resolved `outline`.** The straightforward one of this run —
structurally identical to `border` (same non-inherited width/color
shape, same shorthand grammar modulo which style keywords are valid),
just painted outside the border box instead of as part of it, with no
box-model effect at all. Factored the border shorthand's parsing loop
out into a shared `parse_border_like_shorthand` rather than
duplicating it, since `outline`'s only real difference from `border`
is its style-keyword list (`auto` instead of `hidden`). Worth noting
for next time a "clearly outside the box model" paint-only property
comes up (`box-shadow` is the obvious next candidate already on
TODO.txt): verify it with an *actual* `measure()` call proving
box geometry is unaffected, not just that painting looks right — easy
to accidentally regress layout while only intending to change paint.

**v27 resolved `box-shadow`** — did take that `measure()`-proves-
layout-is-untouched advice from v26, same test shape reused. The
interesting decision here was scope, not mechanism: real `box-shadow`
has `inset`, blur, spread, `currentColor` fallback, and comma-
separated multiple shadows — implemented only offset+spread+solid
color, explicitly *rejecting* `inset` (parse failure, so it drops like
any unsupported value) rather than rendering it wrong as an outer
shadow. That distinction matters and is worth repeating as a pattern:
when a property has a variant that would require a fundamentally
different implementation to render *correctly* (inset shadows clip to
the content area; outer ones don't), reject that variant outright
instead of rendering it using the machinery built for the common case
— a wrong-but-plausible-looking render is worse than nothing, because
nothing is honestly "not supported" while wrong-but-plausible invites
trusting output that's actually incorrect.

**`letter-spacing` was attempted next and reverted (no commit) — a
real cosmic-text bug, not a scratchy one.** Full implementation
(parsing, real-CSS-inheritance cascade, `SpanStyle`/`shape_spans`
threading via cosmic-text's own documented `Attrs::letter_spacing()`)
passed every headless test written for it, including one that
measured shaped glyph x-advance directly. Only caught by the
established "spot-check visually, don't just trust unit tests"
habit (see the v12-v20-era `grim` note above): a real two-paragraph
test page showed `letter-spacing: 8px` shattering a short paragraph
into nearly one line per word. Isolated and measured precisely in a
headless test afterward (not scratchy's layout code at fault — this
reproduces with `shape_spans` called directly): cosmic-text 0.19.0's
word-wrap line-breaking starts firing far too early once
`letter-spacing` crosses roughly 2-3px, worsening smoothly with
larger values, while the actual rendered glyph positions are correct.
This is inside cosmic-text's own shaping/layout internals, not
reachable from scratchy's call site. Reverted cleanly (`git checkout
--`, nothing had been committed) rather than ship something that
visibly breaks paragraph wrapping — a regression far worse than "not
implemented." Documented in detail in `TODO.txt` (including the exact
reproduction numbers) for whoever picks this up: options are checking
a newer cosmic-text release (no network access this session, not
checked), finding an upstream fix/workaround, or accepting a lossy
scoped-down version (small-values-only, or disabling wrap whenever
letter-spacing is set) — all three need a judgment call this session
didn't have standing to make alone, hence skip-and-note per the user's
own instruction for this run rather than silently picking one.

**`opacity` was scoped out next without attempting it** — unlike
every other paint-only property landed this run, it can't be resolved
within `resolve_style`/`paint` alone: real `opacity` dims an entire
subtree as one group, which would need threading a new accumulated-
opacity parameter through all 6 of the mutually-recursive box-building
functions (`build_box_for_element`/`build_table_children`/
`build_table_row_cells`/`build_children`/`build_children_with_prefix`/
`collect_children`), not just `collect_children` the way every other
ambient property here has. And a simplified "multiply each
descendant's alpha ambiently" wouldn't actually match real group-
compositing semantics for overlapping semi-transparent content —
a correctness trade-off worth surfacing rather than picking silently.
Logged as its own `TODO.txt` item laying out the three real options
(full correct plumbing, a non-propagating simplified version, or the
lossy ambient-multiply version) for whoever decides.

**v28 resolved `border-radius`**, scoped to the background fill only.
Tried the same "paint a bigger solid rounded shape, then a smaller one
on top" nesting trick that made `outline`/`box-shadow` cheap, and it
genuinely doesn't work for a rounded *border ring* once the interior
might be hollow (no background): there's nothing to safely "restore"
in the no-bg case after a big solid fill overpaints it, unlike
outline/shadow where the thing underneath (background/border) is
always re-painted afterward regardless. Recognized this before writing
any ring code, not after — worth the general lesson: the nested-solid-
fill trick only works when every layer in the stack is *guaranteed*
to repaint over whatever's beneath it; the moment a layer can be
legitimately absent (no background declared), the trick needs real
boundary-membership math instead.

**v29 resolved `font-size: larger`/`smaller`.** Worth flagging this
one specifically: it's the first item this run that wasn't just
closing a `TODO.txt` line — `default.css` has declared `big {
font-size: larger }`/`small { font-size: smaller }` and `sub`/`sup`'s
own `font-size: smaller` since v6/v7, all silently inert this whole
time because the keyword forms were never parsed (same category of
bug as v19's `:is()` discovery, just lower-stakes — these are faithful
to the HTML spec, not a scratchy-specific override, so nobody had
reason to suspect them). Worth periodically grepping `default.css` for
other declarations that might still be silently inert, the way this
one and v19's were — `git grep` for property names against `TODO.txt`
items would surface candidates directly.

**v30 resolved `border-style`** (`dashed`/`dotted`/`double`/`none`/
`hidden`, falling back to `solid` for `groove`/`ridge`/`inset`/
`outset` — no bevel shading). Specifically checked `default.css`'s own
`hr { border-style: inset }` rule (already noted as inert-not-a-
regression in TODO.txt's wording before this landed) resolved to
`Solid` and produced identical pixels to before the property existed
— same "verify the fix doesn't silently break the one real-world rule
that already references it" habit v29 should have reinforced. Also
had to go back and actually capture the `border` shorthand's style
keyword instead of discarding it (v21 only ever recognized-and-
dropped it, to keep width/color parsing from tripping over it) —
worth remembering as a pattern: a property "recognized but discarded"
for one feature is often cheap to wire up for real once a later
feature needs it, since the parsing/recognition work is already done.

**v31 resolved `overflow`, scoped to text-only clipping** — the real
property would need a clip rect checked at every one of this file's
18+ separate `fill_rect`/`fill_rounded_rect` call sites (background,
border×4, outline×4, shadow, rounded-rect fills), which would have
meant touching the paint path of nearly every feature landed this
session this late in a long run — a real regression risk, not a
hypothetical one, given how many of those exact call sites were
written or changed in v21/v26/v27/v28/v30. Recognized that text
painting already funnels through exactly one shared per-pixel
callback (`paint_buffer`'s `LegacyRenderer`) plus one `fill_rect` call
(`paint_span_backgrounds`), so clipping there covers the single most
common real use (truncating overflowing text) at a fraction of the
risk. Worth the general lesson alongside border-radius's (v28): before
building a feature that needs to touch "every paint call site," check
whether the actual common use case can be served by a narrower
choke point instead — here, text happened to have exactly one.
Verified with a real end-to-end pipeline test (not just unit-level
math) specifically because clipping is the kind of bug that's easy to
get subtly wrong (off-by-one edges, wrong coordinate space, double-
counting an offset) in ways isolated unit tests on `intersect_clip`
alone wouldn't catch.

**v32 resolved `cursor` (`auto`/`pointer`)** — scratchy's first real
interactivity beyond mouse-wheel scroll (`WindowEvent::CursorMoved`,
previously unhandled). Fixes another currently-inert `default.css`
rule (`a { cursor: pointer }`, declared since early on, same category
as v29's `font-size: larger`/`smaller` discovery). The real design
decision here was recognizing that `<a>` is *inline* content — no
block-level `Laid` box of its own — so correct hit-testing needs
glyph-level precision (which exact glyph the mouse is over, via the
cached shaped `Buffer`'s `metadata`), not just block-level, the same
insight v23 (`vertical-align`) and v25 (`visibility`) already
established for per-span properties. Verified that precision directly
in a real pipeline test (an actual link glyph reports `Pointer`,
adjacent plain-text glyphs report `Auto`) rather than trusting it by
construction.

**Important limitation, explicitly flagged, not glossed over**: the
live `window.set_cursor` / `WindowEvent::CursorMoved` wiring itself
could only be smoke-tested (launched, confirmed no crash), not
visually verified — the OS cursor icon is compositor-rendered, never
touches scratchy's own pixel buffer, so `grim` is blind to it, and
this sandbox has no tool to synthesize a real mouse-move event. Every
other piece (hit-testing precision, the style→icon mapping) is
verified directly; only this last, thin, mechanical layer isn't. Noted
in `TODO.txt` for a real interactive check next time a human's at the
keyboard — worth remembering as a category: **any feature whose
correctness ultimately depends on OS/compositor-level state outside
scratchy's own framebuffer** (cursor icons, window titles, taskbar
state, etc.) hits this same verification ceiling in this environment.

**v33 resolved `text-decoration-style`/`-color`** — `dotted`/`dashed`/
`double`/an explicit color were already accepted in the `text-decoration`
shorthand's grammar since v17 but silently discarded; every decoration
painted as a plain solid line in the text's own color regardless. Fixed
`abbr[title] { text-decoration: dotted underline }` — another
currently-inert `default.css` rule, same category as v29/v32's
discoveries. The key constraint: cosmic-text 0.19's own
`render_decoration` function is `pub`, but the per-span function it
calls internally (`draw_decoration_span`) isn't, and the public
function offers no hook to vary painting style per span. Rather than
work around that, reimplemented the whole thing by hand using
`DecorationSpan`'s/`GlyphDecorationData`'s public fields (all public,
confirmed by reading cosmic-text's vendored source directly) —
mirroring cosmic-text's own offset/thickness/color formulas exactly so
`Solid` stays pixel-identical, and only branching to a hand-rolled
dash/dot tiling pattern for `Dashed`/`Dotted`. Same per-span
`glyph.metadata` lookup pattern used for every other per-span property
this session (backgrounds, valigns, hidden, cursors) — now a 7th
`ShapedParagraph` element.

**A real adjacent bug was found while visually verifying this, not
introduced by it**: the very first test (`text-decoration` declared
directly on a `<p>`) silently rendered with no decoration at all, while
the identical declaration on a nested `<span>` worked. Traced to
`collect_children`'s ambient `decoration` parameter being reseeded to
`DecorationValue::default()` every time a block starts its own
children (`build_children`/`build_children_with_prefix`), rather than
from that block's own resolved `style.text_decoration` — so a block's
*own direct text* never sees its *own* `text-decoration`, only inline
descendants do (since those re-check their own resolved style
independently of the ambient parameter). This has existed since v17;
it just never surfaced because every real `default.css` rule and prior
test page only ever declared `text-decoration` on inline elements
(`u`/`s`/`abbr`/`a`). A correct fix needs the same ambient-value
threading through `build_box_for_element`/`build_table_children`/
`build_table_row_cells`/`build_children`/`build_children_with_prefix`
that `opacity` (v28) was scoped out for already — recognized as the
same shape of invasive plumbing change, not attempted here either, and
documented in `TODO.txt` instead of guessed at. Re-tested with the
decoration moved onto a nested `<span>` (the realistic, default.css-
matching shape) to confirm the *new* style/color logic itself was
correct, independent of this pre-existing gap — it was: dotted,
dashed-blue, and double-red all rendered correctly in a real `grim`
screenshot.

**v34 resolved `rem` lengths** — the smaller, safer pick after v33,
deliberately avoiding the other two live `TODO.txt` candidates: named
`font-family` fallback lists would need an owned/leaked string living
inside `ComputedStyle`/`InheritedStyle`/`SpanStyle`, which are all
`#[derive(Clone, Copy)]` by design and threaded by value through every
recursive tree-building call — a real architecture question (Copy vs.
a `Box::leak` trick vs. a bigger refactor) worth more than a unilateral
pick; `word-spacing` has no cosmic-text 0.19 API at all to hook into
(confirmed by grepping its vendored source), a worse version of the
same risk that already burned v25's `letter-spacing` attempt. `rem`, by
contrast, fit the existing architecture cleanly: `Length` already had
an `Em` variant and a `resolve(font_size)` pattern threaded through
~14 call sites inside one function (`resolve_style`), so adding `Rem`
only needed a second, always-constant figure (`root_font_size`)
threaded alongside the existing one, not a new kind of plumbing.
Scoped `root_font_size` to a fixed `16.0` constant rather than a real
document root's own cascaded font-size (an `html { font-size: ... }`
override isn't respected) — flagged as its own `TODO.txt` item, since
nothing yet needs it and getting it right means resolving `<html>`
before everything else, not just copying a value down. Verified the
property's whole point directly: a `2rem` child nested inside a `40px`
parent resolves to `32px` (ignoring the parent), confirmed both by a
headless assertion and a real `grim` screenshot where the nested box
is visibly smaller than its parent, not the much-larger `80px` an
`em`-confused implementation would have produced.

**v35 resolved `font-weight: bolder`/`lighter`** — the last member of
the original misc-CSS-properties cluster worth picking up now;
`word-spacing`, the only one left, got its own narrower-scope
`TODO.txt` item instead, since grepping cosmic-text's vendored source
confirmed it has no `word_spacing` API at all (not even a buggy one —
`letter_spacing_opt` is genuinely the only tracking-style knob `Attrs`
exposes), meaning a correct implementation would mean hand-rolling
word-boundary spacing entirely outside cosmic-text's own shaping and
line-breaking, a worse-case version of exactly the risk that broke the
`letter-spacing` attempt. `font-weight: bolder`/`lighter`, by contrast,
mapped directly onto the same "resolve against the parent, not a fixed
value" shape `font-size`'s `larger`/`smaller` (v29) already established
— the only real new piece was the CSS spec's actual step table (not a
ratio) for how `bolder`/`lighter` land relative to an inherited weight.
Verified the one thing worth actually proving — that it's relative to
the *parent*, not a fixed absolute bump — with a cascade test showing
the identical `bolder` declaration resolves to `700` starting from a
normal root but `900` starting from an already-bold parent, and
visually confirmed a doubly-`bolder` nested span renders heavier than
a plain bold sibling.

**v36 resolved `rem`'s own narrower-scope gap**: it now resolves
against `<html>`'s real cascaded `font-size` (`build_layout_tree`
resolves it once up front via a new `find_html_element`, seeds
`root_font_size` from the result) instead of a fixed `16.0` constant.
Picked first off a short post-session prioritization pass (user asked
to check `TODO.txt` and prioritize) — the three items from this pass
that *do* need an architecture decision or user sign-off (named
`font-family` fallback lists, `opacity`, `letter-spacing`/
`word-spacing`) and the bigger survey items (positioning, flexbox/
grid, custom properties, at-rules, forms/JS) are explicitly still
deferred, not forgotten.

**v37 resolved `:link`/`:visited`** — same prioritization pass as v36.
`NonTSPseudoClass` was a stub uninhabited enum (`OurPseudoClass` now),
so these silently failed to *parse* at all before, same bug shape as
v19's `:is()` discovery. `:link` needed no interactivity, just a
structural check (`a`/`area` with `href`) that also happens to back
`selectors`' own `:any-link`; `:visited` always declines to match,
honestly reflecting that scratchy has no navigation/history.
`:hover`/`:focus`/`:active` still can't parse — they need real
interaction state, left as a narrower `TODO.txt` item.

**v38 resolved `border-radius` + a visible border** (`solid` only —
`dashed`/`dotted`/`double` still fall back to square corners, its own
narrower `TODO.txt` item now) — third and last item from this session's
prioritization pass. New `fill_rounded_ring` tests each row's outer and
inner rounded boundary directly, since the nested-solid-fill trick
other paint-only properties use doesn't work once the interior isn't
guaranteed opaque. Verified pixel-level (temp test) and visually.

**Significant finding while verifying v38, not previously in
`TODO.txt` at all**: `width`/`height` CSS properties aren't implemented
— confirmed by grep, no field for either anywhere. Every box is purely
content-sized today. This is a much bigger gap than anything else
closed this session (real design questions about how an explicit size
interacts with `measure`'s content-driven sizing) — logged as its own
`TODO.txt` item, worth a dedicated future slice rather than guessing at
now.

**v39 fixed the real `text-decoration` bug `v33` found but didn't fix**
— a block's own direct text (and nested block descendants) never saw
its own `text-decoration` declaration, only inline descendants did.
Turned out *not* to share `opacity`'s actual blocker despite
`TODO.txt` grouping them together: `opacity` needed a semantics
decision from the user; this one had one unambiguous correct behavior
the whole time (the same `unwrap_or` propagation already proven
correct for the inline case), just not yet threaded across block
boundaries. Threaded a `decoration: DecorationValue` parameter through
`build_box_for_element`/`build_table_children`/`build_table_row_cells`/
`build_children`/`build_children_with_prefix`. Verified with 4 temp
tests (own direct text, nested-block propagation, explicit `none`
opt-out, no sibling leak) plus a `grim` screenshot and a regression
check against inline decoration/links/lists/tables. Worth remembering
alongside `opacity`'s own entry: **"needs the same plumbing shape as
opacity" isn't the same claim as "has the same blocker as opacity"** —
check whether the semantics are actually ambiguous before assuming a
plumbing-heavy fix needs a design decision too.

**v40 extended `overflow: hidden` to clip a descendant's own box
decoration** (background/border/outline/shadow), not just text — the
last item from this session's prioritization pass. Rather than add
clip checks at each of the ~20 separate `fill_rect`/`fill_rounded_rect`
call sites (the real regression risk `TODO.txt` had flagged), pushed
clip-awareness into `fill_rect` itself and threaded a `clip` parameter
through every function that calls it, so nothing can miss it. Verified
with a pixel test, a full-pipeline test, and a `grim` screenshot (a
`box-shadow` offset -300px off a clipped ancestor, correctly clipped to
a small sliver). One false alarm along the way, caught before it went
anywhere: a live visual check using negative margin to push a child
outside its parent *looked* unclipped at first, but was actually
correct CSS — negative `margin-left` on a `width: auto` box also grows
its computed width, so most of it legitimately overlapped back into
the parent. Checked the real geometry headlessly before concluding
anything was wrong.

With v40, every item from this session's prioritization pass is
closed: `rem` root font-size (v36), `:link`/`:visited` (v37),
`border-radius` + border (v38), the block-level `text-decoration` bug
(v39), `overflow` box-decoration clipping (v40). The `width`/`height`
gap found while verifying v38 remains open, flagged for its own
dedicated future session — see its own `TODO.txt` entry.

**v42 resolved `width`/`height`** — the biggest gap found while
verifying v38, now closed. Content-box semantics throughout: an
explicit `width` replaces the container-derived width in `measure`
outright (including extending past the container when wider — not
clamped, same as real CSS); an explicit `height` overrides only the
box's own *reported* size for its parent's stacking, never skips
actually laying out children in full, so taller content spills past
the box (confirmed with the user as the deliberate choice — real CSS
`overflow: visible` default — rather than an always-clip simplification)
unless that element also sets `overflow: hidden`, which v40's existing
clipping already handles correctly with no new glue needed. First real
use of v41's permanent test suite: 6 tests added and kept, not written-
then-deleted.

With v42, every item flagged in this session (started by "check the
TODO... prioritize and get to work") is closed: `rem` root font-size
(v36), `:link`/`:visited` (v37), `border-radius` + border (v38), the
block-level `text-decoration` bug (v39), `overflow` box-decoration
clipping (v40), the test-suite policy reversal (v41), and `width`/
`height` (v42) — the one item v38 surfaced that didn't exist on the
list at session start. `TODO.txt` is left with only narrower-scope
residual items (named `font-family`, `%`/unitless lengths,
`box-sizing`/`min-*`/`max-*`, table auto-layout, `:hover`/`:focus`/
`:active`, dashed/dotted `border-radius`, `word-spacing`,
`letter-spacing`, `opacity`) plus the big, still-untouched survey items
(positioning, flexbox/grid, custom properties, at-rules, forms/
replaced-elements/JS) — see `TODO.txt` itself for the current list,
since this prose will drift out of date as more gets picked off.

## Priority going forward: rendering, not networking

Explicit decision: the next phase of work is about how scratchy
*renders* things (more CSS, layout correctness, probably links and/or
images eventually) — not about giving it its own networking. That
stays true to the original "scratchy never performs networking itself"
design (see `PROMPTS.md`, v1) and avoids scope creep into HTTP/TLS
concerns while the rendering pipeline is still young.

For testing against real-world pages without live network access
during dev loops, the plan is an **external tool** (not part of
scratchy, not yet built) that downloads a page and bundles it —
inlining whatever's needed — into a single self-contained local HTML
file, which can then just be passed to scratchy as a normal file
argument (`scratchy bundled.html`). This keeps scratchy itself free of
fetch/parse-multiple-resources complexity while still allowing
realistic test fixtures. Nothing built yet; just the agreed approach.

In the meantime, ad hoc testing against real pages this session used:
`curl -s URL -o /tmp/page.html` then `scratchy /tmp/page.html`.

## Source layout (Q38): `src/lib.rs` split into modules

`src/lib.rs` used to be one ~27k-line file. It is now a thin root plus
modules, moved verbatim (no behavior change). Find code with
`grep -rn 'fn <name>' src/`.

- `lib.rs` (~190 lines): imports, `mod` declarations, `pub use`/`pub(crate) use`
  re-exports of every module, and `pub fn run()`.
- `dom.rs`: CLI input/args, encoding sniffing, html5ever parse glue,
  `selectors` glue (`El`, `OurSelectorImpl`), interaction state, focus.
- `css.rs`: CSS parsing, value types (`ComputedStyle` and friends), media queries.
- `style.rs`: the cascade (`compute_style` and helpers).
- `images.rs`: image decoding, `data:` URIs, background-image/mask plumbing.
- `layout.rs`: `LayoutBox` tree, `measure`/`measure_document`, block, inline,
  float, flex, grid, table layout, text shaping, `Laid`.
- `paint.rs`: `paint`, backgrounds, borders, hit testing.
- `net.rs`: link navigation (`wire`) and external stylesheet/image resources.
- `app.rs`: the winit `App`, key bindings, command line, event loop handler.
- `tests/`: the `#[cfg(test)] mod tests` (`tests/mod.rs` = shared helpers, one `<area>_tests.rs` per area; never delete tests); `wpt.rs`: the WPT runner.

Every module starts with `use super::*;` and the root re-exports each
module with a glob, so all modules (and `tests.rs`) see each other's items
unqualified, exactly as when it was one file. Moving items across modules
required `pub(crate)` on items, struct fields and inherent methods. The
public (`pub`) surface used by `benches/` is unchanged. The `thread_local!`
statics are `pub(crate)` for the same reason.

## Working notes for next session

- **Verification tests are now kept permanently**, in `src/tests/` (QT1 split the old single `src/tests.rs`; was `src/lib.rs`'s
  `#[cfg(test)] mod tests` — reversed as of v41. Every version from
  v13 through v40 wrote a temporary `#[cfg(test)]` module, verified,
  then deleted it before committing, on a self-justification v13 wrote
  into `PROMPTS.md` that was never actually something the user asked
  for (see v41's own `PROMPTS.md` entry for the full story — they
  asked directly when this was decided, and it turned out it wasn't).
  Going forward: write the test, run it, **keep it** in `mod tests`
  rather than deleting it, unless told otherwise for a specific case.
  `cargo test` is now part of the standard per-feature loop alongside
  `cargo check`/`cargo clippy`/`cargo install --path .`.
- **Never use a broad `pkill -f "target/debug/scratchy"`** if there's
  any chance a persistent test instance is running (the user often
  keeps one open to interact with directly) — it matches and kills
  *every* scratchy process indiscriminately. This happened twice in
  this session by accident. Kill by specific PID instead
  (`kill -9 <pid>`, found via `pgrep -fa target/debug/scratchy`).
- Performance claims in this codebase were all settled by adding
  temporary `eprintln!`/`Instant` timing, measuring, then removing the
  instrumentation before committing — never by guessing. Several
  "obvious" theories turned out wrong under measurement (e.g. a
  suspected redraw-storm-from-event-queuing theory wasn't it; the
  real cause was client-side decoration state). Keep doing this.
- The user asked about setting up `criterion` for proper `cargo bench`
  targets instead of hand-editing `main.rs` for every measurement —
  agreed it's worth it, not yet done. Good candidate for next session
  if more perf tuning comes up.
- `cargo check` is fast (~0.5–1s) for a quick compile sanity check;
  `cargo build` takes several seconds (more after larger changes) —
  use `check` while iterating, `build` before actually running.
- The repo has a GitHub remote (`origin` →
  `https://github.com/Loupax/vibe-it.git`). Pushed v4-v20 (everything
  that had accumulated unpushed) plus the TODO.txt gap-survey commit
  in this session, after the user explicitly asked for it. User then
  authorized ongoing autonomous work: pick items off `TODO.txt`,
  **one commit and one push per feature**, and — since they won't be
  available to answer questions — skip anything that needs
  clarification and leave a note instead of guessing. This replaces
  the earlier "don't push without asking" caution for this specific,
  explicitly-authorized workflow; it isn't a standing license to push
  in general outside it.

## v43 — `box-sizing`/`min-*`/`max-*`-width/height

Resolved the narrower `box-sizing`/`min-width`/`max-width`/
`min-height`/`max-height` gap v42 left open (see PROMPTS.md for the
full design/implementation writeup) — no real design decision
surfaced this time, unlike v42's own height-overflow question, since
both `box-sizing`'s two keywords and the min/max clamp precedence are
exactly spec'd. Collapsed what used to be three separate inline
`border_h` formulas (one per `BoxContent` branch) plus the `border_w`/
`content_w` pair into a single `clamp_box_dim` helper — worth
remembering as the place to extend if a future `%`/`min-content`/
`max-content` value ever needs the same width/height machinery, rather
than re-duplicating the clamp logic per branch again.

`%` is still the next real gap on anything width/height-shaped —
explicitly deferred again here, same as v42, since nothing in
`default.css` or a real test page has needed it yet.

## v44 — structural pseudo-classes already worked

Picked off `TODO.txt` autonomously (standing workflow, see above).
`:not()`/`:nth-child()`/`:first-child` turned out to already work —
they're generic `selectors`-crate matching, not something scratchy
hand-implements like `:link`/`:visited`, and the `SelectorsElement`
methods they need (`prev_sibling_element`, `next_sibling_element`,
`is_root`, `is_empty`) were already there. Added 3 tests to confirm
rather than assume, per the standing "measure, don't guess" habit.
Only real snag: writing `<style>` inside `<body>` in a test (matching
every older pseudo-class test's style) makes `<style>` itself count as
the first child, which looked like a `:nth-child`/`:first-child` bug
until traced back to the test's own HTML structure — `<style>` belongs
in `<head>` for a child-index test like this one specifically.

Next up from the remaining `TODO.txt`: CSS custom properties
(`--foo`/`var()`) — fully spec'd, no design decision needed, and
common on real pages, unlike most of the other deliberately-deferred
items. `:hover`/`:active` state tracking is the other bounded
candidate after that (mouse position is already tracked for `cursor`;
needs mouse-button state + repaint-on-match-change wiring added).
Positioning (`float`/`position`) and flexbox/grid are the biggest real
wins for rendering actual pages but are large, invasive undertakings —
worth tackling after these smaller items, not before.

## v45 — CSS custom properties, `:root`-scoped only

Landed `--name`/`var()` without touching `resolve_style`'s per-element
cascade at all — real spec behavior inherits/cascades custom properties
per element, which would be a materially bigger plumbing job (on
`opacity`'s order, from the TODO.txt item it's still parked under).
Instead: one flat table of `--name` values harvested from whatever
rules match the real `<html>` element (`:root`, `html`, ...), then
`var()` substituted as a single whole-stylesheet text pass before the
normal parser ever runs. Good enough for the dominant real-world
pattern (design tokens declared once at the top) — not for per-subtree
variable overrides, which is now its own documented TODO.txt item
instead of the old flat "not implemented" one.

Added `:root` as a real pseudo-class along the way (free — just
delegates to the `is_root()` structural method already there for
other reasons), same kind of "turned out cheap once actually checked"
outcome as v44's structural pseudo-classes.

Real bug caught by the tests, not assumed away: gating the whole
substitution pass on "did we find any custom properties at all" is
wrong — `var(--undeclared, red)` needs its fallback resolved even when
*zero* `--name`s exist anywhere in the stylesheet. Gate on "does the
CSS text mention `var(` at all" instead.

Also, mid-session, the user asked directly to add `<img>` rendering to
`TODO.txt`. Split out from the old blanket "no replaced elements" item,
since unlike most of that item it doesn't actually need networking — a
local/`data:` URI `src` works today, and `snag` already rewrites real
pages into exactly that form. Not implemented, just tracked as its own
prioritized item now.

Next real candidates from `TODO.txt`: `:hover`/`:active` state tracking
(bounded — mouse position already tracked for `cursor`, needs mouse-
button state + repaint-on-match-change wiring), or `<img>` rendering
(needs an image-decoder crate decision — not chosen yet). Positioning
and flexbox/grid remain the biggest real wins but the biggest lifts.

## `<img>` rendering (v46) — scope decisions for future sessions

Implemented: decoding (`image` crate, PNG/JPEG/GIF/WebP, `data:` URI
only — see below), replaced-element sizing (intrinsic/CSS/HTML-attr,
aspect-ratio-preserving), painting (`blit_image`, nearest-neighbor), alt-
text fallback for a missing/broken `src`.

One scope cut worth remembering if a real page's images don't look
right:

1. **No local file `src`, only `data:` URIs.** The user explicitly
   narrowed this mid-planning ("drop file processing in general...
   treat files the same way as we treat network until further notice")
   — this isn't a technical limitation (the HTML file's own path is
   known for `Input::File`, nothing stops resolving a relative path
   against it), it's a deliberate policy choice to keep `<img src>`
   in the same bucket as the rest of the no-networking stance. Revisit
   only if asked to.

**Update, v48: real inline-level flow was built after all.** The v46
entry below used to say `<img>` always gets its own box, breaking
surrounding inline text — the user came back in the very next session
and asked for exactly that to be reversed ("Make `<img>` tags inline by
default"). The architectural reasoning from v46 was still right at the
time (true inline flow needs hand-rolled line-breaking since cosmic-
text has no embedded-object concept, and extending cosmic-text itself
isn't a promising path — it's a UI text-shaping library, not a
document-layout engine, same precedent as the `letter-spacing`
TODO.txt item treating it as a black box to work around, not patch) —
what changed was the user's call on whether that cost was worth paying
now, not the facts themselves. Confirmed again directly with the user
before starting: a cheap "tighter gap, still not real sharing"
approximation was offered as an alternative and declined in favor of
the full rewrite. `measure_mixed_inline` is the result — see TODO.txt's
`<img>` entry and PROMPTS.md's v48 write-up for exactly what it does
and its own (much narrower) list of simplifications. **Lesson for next
time:** a `TODO.txt` entry that says "revisit only if X turns out to
matter" is a note about *today's* priority call, not a permanent
architectural verdict — don't assume a past session's scope decision
is still the user's preference without checking, even when the
reasoning behind it still holds up.

**Confirmed, no code change: big unstyled `<img>`s overflowing the
viewport is correct, not a bug.** The user noticed this testing real
pages after v48 and asked directly. Real browsers don't implicitly cap
`<img>` width either — "images never overflow" on the real web almost
always comes from a page's own author CSS (`img { max-width: 100% }`
or a reset), not a UA default — so scratchy matches real behavior by
*not* adding that rule to `default.css`. Offered the pragmatic (non-
spec) alternative of adding it as a UA-level default anyway; the user
explicitly chose to keep matching real browser behavior instead. See
the `<img>` `TODO.txt` entry for the durable pointer — worth checking
before ever "fixing" this overflow unprompted.

## v49 — `%` lengths, and a new standing autonomous-loop convention

First item processed under a new standing instruction: pick the next
`TODO.txt` item, implement it fully (including writing/keeping
permanent tests, updating `TODO.txt`/`PROMPTS.md`/`NOTES.md`,
committing), then clear context and repeat until the list is empty —
distinct from the earlier "one commit+push per feature, skip anything
needing clarification" loop (still the standing policy for *pushing*;
this session didn't push, see the user's own instructions for why not
assumed here). Landed `%` lengths — the item that had been deferred
three times running ("not adding until something needs it") — since
the loop's whole premise is that deferral no longer applies once
something's sitting at the top of the list.

**Worth remembering architecturally**: `width`/`height`/`margin`/
`padding`/`border-radius` used to resolve straight to a final pixel
`f32` inside `resolve_style` (cascade time, before layout) — fine for
every length form that only needs a font-size, but wrong for `%`,
whose real basis is a containing-block dimension that `resolve_style`
genuinely cannot know yet. Rather than inventing a new deferred-value
pattern from scratch, this reused the one `LineHeight` already
established back in v24 (stay symbolic through `ComputedStyle`, resolve
only once a real basis exists) — now generalized into its own
`BoxLength` type, with a `.px()` accessor whose debug-assert pins the
"only ever read post-`measure`" invariant everywhere paint/hit-testing
code touches these fields. Worth checking this pattern first before
reaching for a parallel/shadow representation next time a CSS value's
real resolution timing doesn't line up with when this codebase
currently computes it.

**A real, caught-before-shipping footgun worth flagging for next
time**: `build_layout_tree`'s document root is a *synthetic* wrapper
box — `<html>` is laid out *inside* it, not literally the layout tree's
own root — and that same root `Laid`'s `h` doubles as "total scrollable
content height" in `redraw`'s own scroll-clamping. Wiring the window's
real inner height through as a viewport-height concept (so `html, body
{ height: 100% }` could resolve) looked like an easy, obviously-correct
addition and would have silently broken scrolling on every page taller
than one window — caught by actually tracing where `laid.h` gets read
elsewhere before shipping, not by the type system or a test. Flagged as
its own `TODO.txt` item (the root box's two roles need separating
first) rather than either shipping it broken or silently dropping the
idea with no trace.

## v50 — real auto table layout

Item 2 of the autonomous `TODO.txt` loop (v49 did `%` lengths). Replaced
v15's equal-width-column table layout with real CSS2.1 §17.5.2.2
automatic table layout — see `PROMPTS.md` for the full write-up. Two
things worth remembering architecturally:

**A table's own box-model entry can't go through `measure`'s normal
content dispatch, because real column sizing needs every row's cells up
front, before any one row is laid out.** The equal-width case got away
with letting each `TableRow` compute its own split purely from its own
`content_w` — a purely local decision. Real column widths are a
*table-wide* decision (the max across every row, for each column), so
`display: table` is special-cased at the very top of `measure`, before
the normal box-model preamble even runs, and owns its own rows directly
via `measure_table`/`measure_row_with_widths` rather than recursing back
through `measure`'s own `BoxContent::TableRow` arm. That arm is now an
explicitly-documented defensive fallback, not live code for any
well-formed table. Extracted `measure`'s own preamble into a shared
`box_model_entry` helper to support this without duplicating the margin/
padding/border/`clamp_box_dim` math a second time — worth reusing again
if a future feature needs a box's own box-model math computed somewhere
other than `measure`'s normal call path (the same shape of problem this
row-measuring case ran into).

**Computing a cell's natural (min/max) content width reuses `measure_
mixed_inline` (v48) at its two width extremes**, rather than a new,
separate measurement path: called at an effectively unconstrained width,
its existing greedy wrapper leaves everything on one line (the furthest
item's right edge is the max-content width); called at `0` width, its
own wrap condition forces every word/image onto its own line (the widest
resulting line is the min-content width — the widest unbreakable token).
Worth remembering as a general pattern: a hand-rolled greedy line-
breaker already written for one purpose (real inline flow) can often
answer "what's the natural width of this content" for free at its own
extremes, without a second bespoke measurement codepath. The real cost:
a table cell now gets shaped up to 3x (once at each extreme to find its
natural size, once more at its real column width) instead of once — the
two-pass cost v15's own original entry already anticipated real auto
layout would require, and not something worth optimizing away given it
only affects table cells, not ordinary paragraphs.

## v51 — named `font-family` fallback lists

Item 3 of the autonomous `TODO.txt` loop (v49 did `%` lengths, v50 did
table layout). Resolved the `Copy`-vs-`Vec` architecture question v34
explicitly parked rather than pick unilaterally: `FontFamily` is now a
leaked `&'static [FontFamilyItem]` (`Box::leak`, once per distinct
parsed `font-family` declaration, in `DeclParser`) instead of an owned
`Vec` — keeps `ComputedStyle`/`InheritedStyle`/`SpanStyle` exactly as
`Copy` as they were, at the cost of that memory never being freed for
the process's life. Judged acceptable, not a new category of problem:
the whole stylesheet is already reparsed from scratch on every relayout
with zero cross-redraw caching, so this is bounded by how many
*distinct* `font-family` declarations a document's CSS has (typically a
handful), not by element or redraw count, and dwarfed by the rest of
that reparse work. Worth remembering as the template next time a
variable-length/owned value needs to live in one of these three
`Copy`-by-design structs: leak-into-`'static` is a legitimate choice
when the alternative is a cascading `Copy`-to-`Clone` refactor across a
large fraction of the file for a property most pages don't even use —
not every non-`Copy` value needs `background-image`'s separate-ambient-
parameter treatment (that pattern only fits a property that *isn't*
really CSS-inherited; `font-family` genuinely is).

**How cosmic-text actually resolves (or fails to resolve) a named font,
confirmed by reading its vendored source, not assumed**: `cosmic_text::
Attrs` has no fallback-list concept at all — `Attrs::family` is a single
`Family<'a>`, never a slice — so a parsed list can never be handed to it
directly, regardless of how scratchy stores it. The real in-order
fallback logic ("try this name, fall through to the next if it's not
installed") lives one layer down, in `fontdb::Database::query`, which
already loops a `Query`'s `families` slice and returns the first one
with any installed face — it does not error on a missing name and does
not silently substitute an unrelated font. The only reason scratchy
can't just hand `fontdb` the whole list in one `Query` and be done: that
returns a matched font *ID*, not *which family in the list* produced
it, and `Attrs::family` needs the latter. So `FontFamily::resolve`
queries fontdb one candidate at a time, stopping at the first hit —
same observable fallback behavior, recovered one family at a time
instead of all at once. Also worth noting: `FontSystem` only exposes
`db_mut()`, no immutable `db()` getter — confirmed by reading its public
API rather than guessing a method name; `&mut Database` coerces to
`&Database` at the call site with no extra ceremony needed.

## v52 — a first positioning model (`position`/`z-index`): architecture

Picked off `TODO.txt`'s "No positioning model at all" item — the first
of the big, originally-deferred post-v20 survey items. Scoped to
`position: relative`/`absolute`/`fixed` and a real-but-narrow `z-index`
paint-order mechanism; `float`/`clear`, `position: sticky`, and full
CSS2.1 Appendix E stacking contexts are each their own, more precise
`TODO.txt` item now (see PROMPTS.md's v52 entry for the full write-up
of what's deferred and why). This is a significant enough structural
addition that the mental model is worth its own section here, not just
PROMPTS.md's narrative.

**The one fact that makes `relative` cheap: every `Laid`'s `x`/`y` is
already absolute document-space coordinates**, not a nested local
coordinate space — true since the very first version of `measure`/
`paint`, and the reason scroll itself is cheap (`paint` subtracts
`scroll_offset` once, at the very end, rather than threading an
accumulated offset through layout). A box's *reported size* (what its
parent's `cursor_y` stacks the next sibling against) and its *position*
are completely decoupled fields of the same `Laid`, so offsetting a
fully-built subtree after the fact can't disturb anything else — no
flow-side bookkeeping needed at all. `apply_relative_offset` runs once,
at the very end of `measure`'s own dispatch (after an in-flow box's
normal content *and* any `absolute` descendants resolved against it —
see below, since those need to move together with their relatively-
positioned containing block), and a new `shift_laid_tree` helper
recursively adds `(dx, dy)` to every absolute coordinate anywhere in
that subtree (normal `Blocks`/`TableRow` children, a `MixedInline`
run's own per-word/per-image positions, and a box's own out-of-flow
`positioned` list) with no re-measuring at all — pure geometry.

**Taking a box out of flow needed new plumbing, since nothing before
this could skip measuring a child at all.** `measure`'s own
`BoxContent::Blocks` arm is where this lives: a child whose `style.
position` is `Absolute`/`Fixed` is never measured in its parent's own
loop and never advances `cursor_y` — instead it's recorded as a
`PendingPositioned` (the box itself, whether it's `fixed`, and its own
"static position" fallback — the `x`/`y` an ordinary in-flow `measure`
call would have used, needed only when an offset is `auto`). The real
difficulty: its real containing block is the *nearest positioned
ancestor*, which the box's own immediate parent often isn't — could be
several `measure` call-frames further up, or might not exist at all.
Solved by threading a new `pending: &mut Vec<PendingPositioned<'a>>`
out-parameter through every one of `measure`'s own recursive call
sites (and `measure_table`/`measure_row_with_widths`/`lay_out_row_
cells`, so an `absolute` box nested inside a table cell's own content
still works — though a `<tr>`/`<td>`/`<caption>` itself declaring
`position` doesn't catch anything, a narrow documented scope cut):
each level either resolves an entry locally, via `resolve_or_bubble_
positioned` (if *this* box's own `position` isn't `Static` — the real
"nearest positioned ancestor" rule), or forwards it to its own caller's
`pending`, continuing the search one level further out. A `fixed`
entry always keeps bubbling regardless of any intervening `relative`/
`absolute` ancestor's own position — its containing block is always
the viewport (real spec behavior without CSS transforms, the only case
this engine has any notion of). Whatever reaches the very top (every
`fixed` box, plus an `absolute` with no positioned ancestor anywhere
above it) is resolved by a new top-level entry point, `measure_
document`, against the window's real `width`/`height` — this is a
*new*, narrower role than the existing `containing_height` parameter
(which still means "the nearest *in-flow* ancestor's own definite
content height," `None` by default — see v49's own note on not
conflating this with "total scrollable content height"): `viewport_h`
is passed down exactly once, straight into the root `measure` call's
`None` as before for *that* role, and separately into `measure_
document`'s own post-resolution step for out-of-flow boxes only.
`redraw` now calls `measure_document` instead of `measure` directly.

**Resolving a `PendingPositioned` entry** (`resolve_positioned`; QA3: the inset/static position names the *margin* edge, so the box's used margins are added to / subtracted from the border-box position, `auto` margins absorbing free space with insets on both sides) turns
it into a real `Laid` against a *now-concrete* containing block rect
(`cb_x`/`cb_y`/`cb_w`/`cb_h` — always known for real by this point,
unlike `measure`'s own `containing_height`, which can be indefinite for
an ordinary box). `top`/`left` win over `bottom`/`right` when both are
set on an axis (real CSS lets an author derive `width`/`height` from
having all of `left`+`right`/`top`+`bottom` set with the size left
`auto` — not implemented; this box's own size is always computed the
ordinary content-driven way against `cb_w`/`cb_h`, same as any in-flow
box). The one genuine trick needed: `right`/`bottom` need this box's
own just-*measured* size to solve ("containing block's far edge minus
my own width/height") — but measuring needs a starting position first.
So it's measured once at a provisional position (`left`/`top` if set,
else the static-position fallback) and, only if `right`/`bottom` turns
out to be the side that actually matters, shifted (`shift_laid_tree`
again) to its real final position — no second measurement.

**`position: absolute`/`fixed` also forces real CSS's own "computed
`display`" rule (spec §9.7)**: `resolve_style` sets `display: Block`
whenever `position` is `Absolute`/`Fixed` and `display` isn't already
`None`, applied once, right after both are cascaded. Without this, an
absolutely positioned `<span>` (default `display: inline`) would never
get its own `LayoutBox` at all — `collect_children`'s inline branch
would merge its text straight into the surrounding run, silently
dropping the whole element's positioning (and everything else about
it) — the same invisible-`default.css`-gap shape v19/v29/v32 each
already hit once. Because this is resolved once, at cascade time,
nothing downstream (`collect_children`'s block-vs-inline split,
`measure`'s own dispatch) needs a separate `position` check anywhere —
they can keep branching on `display` alone exactly as before.

**Paint order: `Laid` gained two new fields**, `positioned: Vec<Laid>`
(the out-of-flow descendants this box is the resolved containing block
for — empty for the overwhelming majority of boxes) and `fixed: bool`
(true only for a `position: fixed` box resolved at the document root).
`paint` grew one new step, after a box's own normal content: paint
every entry in its own `positioned` list, already sorted ascending by
`z-index` (`auto`/unset treated as `0`, ties in document order — a
stable sort needs no separate tie-break pass) — so an out-of-flow
descendant always paints *after*, and therefore visually on top of,
its own containing block's normal content, and higher-`z-index` peers
paint after lower ones. This is deliberately **not** full CSS2.1
Appendix E stacking-context ordering (see TODO.txt's own item for
exactly what's missing — negative `z-index` painting below its own
containing block, a `relative` box establishing its own separate
stacking context for unrelated overlapping content, etc.) — just
enough "paint in the right order" to make the common overlay/dropdown/
tooltip case correct, which was the task's own stated minimum bar.
`hit_test_cursor` mirrors this (checks `positioned` first, in reverse —
highest `z-index`/last-painted first — before falling into normal
content), so a click lands on whatever's actually visible on top.

**The one piece of real coordinate-space subtlety**: a `fixed` box's
own `Laid::x`/`y` are resolved in *viewport* space (against `measure_
document`'s own `(0, 0, width, viewport_h)` rect), not document space
the way literally every other `Laid` position in this file is — the
whole point, since that's what makes it immune to scroll. Reusing the
existing `scroll_offset` *parameter* (already threaded through every
recursive `paint` call) turned out to be enough: `paint` passes `0.0`
instead of the ambient `scroll_offset` only when recursing into a
`fixed` entry, and every normal recursive call below that just forwards
whatever it was given unchanged — so passing `0.0` once, at the top of
a `fixed` subtree, makes the *entire* subtree render scroll-independent
with no per-node "am I fixed" check needed anywhere inside it.
`hit_test_cursor` isn't structured the same uniform top-down way
(mouse coordinates arrive in document space, translated once at the
call site, not re-derived at every level the way `paint`'s `scroll_
offset` parameter is), so it needed the mirror-image translation
(document space → viewport space, and back) threaded through its own
recursive calls explicitly instead of getting it for free.

## v54 — `float`/`clear`: how a float interacts with the existing block/inline passes

Picked off `TODO.txt`'s `float`/`clear` item — the one v52 explicitly
split out of the positioning-model work as "a genuinely different,
separate layout algorithm... not just an extension of this positioning
work." That turned out right in one way (three different pieces of
`measure` needed to learn about floats, not one) and wrong in another
(the actual *mechanism* ended up much closer to `position`'s own
machinery than v52's writeup guessed, once `Floats` was modeled as
"read-only ambient state threaded through recursion," the same shape
`InheritedStyle` already uses, rather than anything resembling
`PendingPositioned`'s bubble-up search).

**Why a float needs no bubble-up search, unlike `absolute`/`fixed`.**
`position: absolute`'s containing block can be arbitrarily far up the
tree (the *nearest positioned ancestor*, found only by bubbling a
`PendingPositioned` value outward until something catches it — see
v52's own writeup). A float's containing block is never a search at
all: it's always just whichever `BoxContent::Blocks` loop happens to be
iterating over it, exactly the same box that would have contained it in
normal flow anyway. This single fact is why `float` needed a
*different*-shaped piece of new state than `position` did: a new
`Floats` type (`left`/`right`: `Vec<FloatEntry { top, bottom, edge }>`,
document-space coordinates, same convention every other position in
this file uses), passed into `measure` as a new `floats: &Floats`
parameter — read-only from any call's own point of view, grown only by
cloning it locally inside a `BoxContent::Blocks` arm as that specific
loop places its own direct float children. A normal (non-floated) child
recursing further down gets `&local_floats` (the up-to-date clone, so
floats placed earlier in *this* loop are visible to it); anything that
starts a new block formatting context — the document root
(`measure_document`), a float's own content, `position: absolute`/
`fixed` content (`resolve_positioned`), and a table cell's interior
(`lay_out_row_cells`) — gets a fresh, empty `Floats::default()` instead,
matching the real CSS rule that floats never escape into or out of one
of those.

**Placing a float, and the one real "chicken-and-egg" problem it has in
common with `absolute`'s own `right`/`bottom` solving.** A `width: auto`
float doesn't fill its available space the way an ordinary auto-width
block does — real CSS computes it via shrink-to-fit (CSS2.1 §10.3.5):
clamp the content's own natural (min/max) width against the *real*
containing block's width (not the narrower corridor other floats leave
— a float's *size* is never affected by other floats, only its
*position* is). This reuses `cell_natural_width` (v50's per-cell
min/max-content sizing for table columns) completely unchanged — it
was never actually table-specific, despite the name, just never called
from anywhere else before. The result has to *override* `measure`'s
ordinary "auto width = fill the available width passed in" computation
without disturbing that same call's `%`-margin/padding resolution,
which still needs the *real*, un-narrowed containing-block width as its
own basis — so `box_model_entry` (and `measure` itself) grew one new
parameter, `auto_margin_box_override: Option<f32>`, consulted only at
the exact one spot (`auto_border_w`'s computation) where "auto width
fills the container" would otherwise happen, leaving every other use of
`width`/`containing_height` in that same call untouched. `Float::Right`
has its own small version of the same "need this box's own just-
measured size before its final position is known" problem `resolve_
positioned`'s `right`/`bottom` solving already has (v52) — solved the
identical way: measured once at a provisional position, then shifted
into its real final position with `shift_laid_tree` (no re-measuring)
once its real width is known.

**The float never advances `cursor_y`, but its `Laid` still goes into
its parent's ordinary children list** — unlike `position: absolute`,
which removes its slot from that list entirely (its containing block
genuinely isn't this box's content box at all), a float's containing
block *is* this box's content box; it's only removed from the part of
flow that reserves *vertical stacking space*, so it still belongs in
the same `LaidContent::Blocks(Vec<Laid>)` every other child lives in —
just inserted (originally) at its own point in that list rather than
appended to the end, with no `cursor_y` bump before or after it.

**A real bug, found by looking at a screenshot rather than predicted
in advance**: a later, non-cleared, full-width sibling's own *box* is
never narrowed to avoid a float (only its *inline content* is — real
CSS floats only ever affect line-box geometry, never a block box's own
border/background position) — so that sibling's own opaque background
can legitimately overlap the float's area. Pushing the float's `Laid`
into the children list at its own document-order position meant the
later sibling, painted after it in that same order, painted *over*
most of it. Confirmed and fixed, not left as a hypothetical: `paint`
walks `LaidContent::Blocks` children in array order with no separate
z-ordering concept at all (unlike the `positioned` list, which v52
built real z-index sorting for) — so the fix is entirely in *list
order*, not a new paint-time concept: a `BoxContent::Blocks` loop now
collects its own float children into a side `Vec` and appends that
`Vec` to the normal children list only after every normal sibling has
already been pushed, so every float paints after (visually on top of)
every normal sibling sharing its list, matching CSS2.1 Appendix E's
real float-vs.-in-flow-block stacking order with no new field on `Laid`
and no change to `paint` itself at all.

**Inline text actually wrapping around a float — the part of this
feature with no real precedent to reuse directly, and also the part
that turned out cheapest once the right abstraction was found.**
cosmic-text's own single-`Buffer`-per-paragraph path (`measure`'s plain
`BoxContent::Inline` arm) hands the *whole* paragraph one fixed width
and lets cosmic-text pick every line break internally — there's no
hook to give it a *different* width per line, so a float-adjacent
paragraph can't go through that path at all once it needs lines next to
a float to be a different width than lines below it. `measure_mixed_
inline` (v48's hand-rolled greedy line-breaker, originally built only
for a run mixing text with an inline `<img>`) already produces output
one line at a time, with its own running `cursor_y` — turning its one
fixed `content_w` into a per-line `(left_bound, avail_w)` pair, resolved
once per line via a new `Floats::avail_bounds(y, outer_left,
outer_right)` call at that line's own starting `y`, was a local change
to an already-line-oriented loop, not a new line-layout algorithm.
`measure`'s own dispatch (which `BoxContent::Inline` arm to take) grew
one more trigger condition alongside "contains an image": now also
"this run starts before every float in its own BFC has ended"
(`!floats.is_empty() && content_y < floats.max_bottom()`), routing a
*pure-text* run through the per-word path too whenever it might need
per-line narrowing — the same function now genuinely serves three
different reasons to need it (an image present, a float present, or
both), not two.

**The one deliberate approximation this shares with `measure_mixed_
inline`'s own pre-existing scope**: both `Floats::avail_bounds` (one
`y` probe per line) and the per-float placement check in `measure`'s
`Blocks` arm use a *single point* in `y`, not the real height range a
line/float actually occupies, to decide what's "currently active" —
real but approximate, same category of simplification this file already
accepts elsewhere (e.g. `measure_mixed_inline`'s own per-word shaping
ignoring cross-word kerning). Checked against the common cases this
session's tests/screenshots cover and found to produce correct results
there; not proven correct for every possible float-height/line-height
combination.

**What's still open, see `TODO.txt` for the precise list**: CSS2.1
§9.5.1's full float-placement algorithm (retry lower down when a float
doesn't fit beside an earlier same-side one); the performance cost of
routing every pre-float-bottom paragraph through the slower per-word
path even once it's structurally past the one float that mattered;
and cursor-hit-test priority for a float/later-sibling overlap, which
still follows plain document order rather than the paint order this
session's own bugfix established.

## v55 — `position: sticky`: resolved at layout time, adjusted at paint time

Picked off `TODO.txt`'s `position: sticky` item — the one item in the
whole post-v20 positioning survey whose own `TODO.txt` text flagged a
real design conflict up front, not just a scope-narrowing choice: a
naive sticky implementation needs to know, on every single scroll tick,
whether this box's own would-be-static position has scrolled past some
threshold — a fundamentally different kind of input than `relative`/
`absolute`/`fixed` ever need (all three resolve once, at layout time,
and never change again until a real relayout happens). That's a direct
collision with this engine's own oldest, most load-bearing performance
invariant: **scrolling never touches layout** (see the v4/v5 writeup at
the top of this file — the entire reason scrolling here is cheap at
all, confirmed by real measurements back then, not assumed). Asked the
user directly, via `AskUserQuestion`, before writing any code: cheap
paint-time-only sticky, skip the item, or a fully spec-correct version
that re-measures on scroll. Chosen: paint-time-only — resolve the
static position/containing block once at layout time (the same way
`relative`/`absolute`/`fixed` already do), then clamp/adjust only the
*painted* position against the live scroll offset, entirely at paint
time, with zero relayout cost from scrolling alone. See PROMPTS.md's
v55 entry for the exact question asked and the full narrative.

**The key insight that makes this tractable at all: a sticky box is,
structurally, 95% `relative`.** It stays in normal flow (contributes to
its container's auto height, never displaces a sibling, no `Pending
Positioned`-style bubble-up search for a containing block — its
containing block is simply its own parent, the same ordinary
containing-block rule any in-flow box already follows). The *only*
thing it needs that `relative` doesn't is a *dynamic* offset instead of
a fixed one — so the natural design is: reuse everything `relative`
already has (in-flow placement, the `top`/`bottom` basis, the call
site inside `measure`'s own dispatch), and add exactly one new thing,
a small bundle of *scroll-independent* facts resolved once at layout
time that `paint` can turn into a *live* offset every frame for free.

**`StickyInfo` is that bundle** — a new `Laid::sticky: Option<
StickyInfo>` field, `None` for every box except `position: sticky`
ones. It holds: `static_top` (this box's own `laid.y`, captured
before anything could shift it — identical in spirit to `relative`'s
own basis), `top`/`bottom` (this box's own insets, resolved against
the same containing-block-height basis `relative`'s `%` offsets
already use — `apply_relative_offset`'s own `basis_h` parameter,
reused unchanged), and `cb_top`/`cb_bottom` (this box's real
containing block's — its *parent's content box*, not the padding box
`absolute`/`fixed` anchor against — document-space vertical extent,
the "don't escape your own parent" boundary real sticky semantics
need). The one real timing wrinkle: `cb_top`/`cb_bottom` genuinely
can't be known at the moment this box's own `measure` call returns —
its parent, the box that actually *is* its containing block, hasn't
finished stacking every sibling yet (could be several more below this
one). Solved the same two-phase way v52's `z-index`/`positioned`
machinery already handles a similar "resolve now, finalize once the
full picture exists" problem: `capture_sticky_info` (called right
alongside `apply_relative_offset`, same two call sites in `measure`/
`measure_table`) seeds a permissive placeholder; `patch_sticky_
containing_block`, called by the parent's own `BoxContent::Blocks`
loop (and `measure_table`'s own child-stacking loop) immediately after
it finishes — now knowing its own final `content_y`/`cursor_y` — fills
in the real values on every sticky child that loop just placed.

**One subtlety `shift_laid_tree` had to learn.** A sticky box's own
`StickyInfo` is captured in *absolute document-space* coordinates — but
an already-fully-measured subtree can still move afterward, when an
*ancestor* turns out to be `position: relative` with its own offset (or
`resolve_positioned`'s right/bottom-solving shifts a provisional
measurement into its final position) — `shift_laid_tree` is the one
existing helper both of those already use to move a subtree's absolute
coordinates without re-measuring anything. A sticky descendant caught
up in one of those shifts needs its `static_top`/`cb_top`/`cb_bottom`
moved by the same `dy`, or it'd keep stale, pre-shift bounds forever —
so `shift_laid_tree` now also adjusts `laid.sticky`'s own fields
alongside `laid.x`/`.y` themselves. (Ordering is safe by construction:
`capture_sticky_info`/`patch_sticky_containing_block` always run, for
any given sticky box, strictly before control could ever return to an
ancestor capable of shifting it — the two-phase resolution above always
completes within the single nested `measure` call that produced it.)

**The actual per-frame math, `sticky_shift`, is pure arithmetic over
already-resolved values — no shaping, no layout, nothing `measure`
needs to be involved in at all.** For `top`: `(scroll_offset + top)
.clamp(static_top, max_top) - static_top`, where `max_top = cb_bottom -
box_h`. Below `static_top`: not stuck yet, offset is `0`. Between the
two: tracks the live scroll offset exactly — this is "stuck." Above
`max_top`: clamped there and *stays* there regardless of how much
further the page scrolls, since `max_top` is itself a fixed document-
space coordinate — the box then simply scrolls away with the rest of
its container past that point, exactly like an ordinary static box
would ("stopped sticking"). `bottom` is the mirror image. This is
*informally* CSS Position L3 §6.5's real algorithm, scoped down per the
user's own sign-off: only `top`/`bottom` resolve at all (scratchy has
no horizontal scrolling to clamp a `left`/`right` offset against —
confirmed by grep, `Overflow` only ever clips, never scrolls, and the
only live `scroll_offset` anywhere in this file is vertical), and
"nearest scrolling ancestor" collapses to exactly one thing — the whole
document/viewport — since there's no nested `overflow: scroll`/`auto`
region anywhere in this engine to need its own version of this clamp.

**Wiring the live shift into `paint`/`hit_test_cursor` reuses, rather
than reinvents, `position: fixed`'s own v52 trick.** `fixed` makes an
entire subtree render scroll-independent by passing `0.0` instead of
the ambient `scroll_offset` into that one recursive `paint` call —
every descendant inside just forwards whatever it was given, so the
whole subtree shifts correctly with no per-node check anywhere inside
it. Sticky does the exact same thing with a different adjustment:
`paint`'s own `Blocks`/`TableRow` child loop computes `dy = sticky_
shift(...)` for a sticky child and recurses with `scroll_offset - dy`
instead of `scroll_offset` — inside that recursive call, `draw_y =
child.y - (scroll_offset - dy) = child.y - scroll_offset + dy`, exactly
the shifted position wanted, and every nested descendant (including a
`position: absolute` child anchored to the sticky box, painted via its
own `positioned` list using whatever ambient `scroll_offset` that
nested call sees) picks up the same shift for free. `hit_test_cursor`
needed the mirror-image version written out by hand instead — it isn't
uniformly recursive the same top-down way `paint` is (same reason
`fixed`'s own translation there already isn't, per v52's own writeup),
so each `Blocks`/`TableRow` child check now tests the child's *shifted*
rect (`child.y + dy`) but recurses with `y - dy` (translating back into
the child's own stored, unshifted coordinate space) — it also gained a
new `viewport_h` parameter, threaded through every recursive call,
purely for `sticky_shift`'s own `bottom`-offset math (which needs to
know the viewport's own height, not just the scroll offset).

**Why this genuinely can't cause a relayout, not just "shouldn't."**
`measure` and `Laid` never read `scroll_offset` anywhere — the type
signature itself is the proof, not a convention someone could
accidentally violate. `paint`/`hit_test_cursor` both take `&Laid`
(immutable), never `&mut Laid`, so calling either of them again at a
new scroll offset cannot, as a matter of the borrow checker, mutate the
layout tree at all. `App::redraw`'s own existing `cached: Option<Laid>`
mechanism (unchanged by this feature) already short-circuits a full
`measure` pass whenever only scroll changed, same as before `sticky`
existed. A permanent test (`scrolling_only_changes_the_painted_
position_never_the_laid_tree`) pins this down concretely anyway: builds
one `Laid` tree, paints it twice at very different scroll offsets, and
asserts the sticky box's own layout fields are byte-identical
afterward while the two painted pixel buffers genuinely differ.

**Scope, matching what the user signed off on and nothing more**: no
`left`/`right` sticky (no horizontal scroll to clamp against); real
CSS §9.7's computed-`display`-to-`block` rule deliberately *not*
extended to `sticky` (real spec only names `absolute`/`fixed`) — an
inline `position: sticky` element stays flattened into its surrounding
run with no box of its own, same pre-existing gap `relative` already
has on inline content; a `<tr>`/`<td>` itself declaring `position:
sticky` isn't supported, same pre-existing scope cut `absolute`/
`fixed` already have for table rows/cells since v52. **Not live-
verified interactively** — same category as v32's `cursor` gap and
v52's own `fixed`-while-scrolling gap: this sandbox has no tool to
synthesize a real mouse-wheel event (checked directly: no `wtype`/
`ydotool`/`wlrctl`, and the running compositor — `dwl`, no IPC —
exposes no scripting path for one either). A real test page's sticky
header rendered correctly at its pre-scroll static position via a real
`grim` screenshot; the live "sticks, then releases, while actually
scrolling" behavior is exercised only by the permanent headless tests
(which do call the real `paint`/`sticky_shift` code path, just with a
synthetic `scroll_offset` standing in for a real wheel event) — worth a
real interactive check next time a human's at the keyboard.

## v56 — real CSS2.1 Appendix E stacking-context semantics

Picked off `TODO.txt`'s `z-index`/stacking-order item — the one v52
explicitly left narrower than the full Appendix E model ("every
out-of-flow box anchored to the same containing block paints in one
ascending-`z-index` group, after that containing block's own normal
content"). Two real fixes, both paint-time-only (no change to `measure`
or `Laid`'s own layout fields beyond nothing at all — this entire
feature lives in `paint`/`hit_test_cursor`).

**Negative `z-index` paints between layers 1 and 3, not below layer 1.**
Found a genuine inconsistency in this session's own task wording while
implementing it: one part said negative `z-index` should paint "below
its own containing box's own background," another part gave the exact
Appendix E seven-layer list, which puts background at layer 1 and
negative-`z-index` contexts at layer 2 — i.e. *above* layer 1, not
below it. Resolved by trusting the cited authoritative source
(https://www.w3.org/TR/CSS21/zindex.html) over the paraphrase, not by
asking the user — this wasn't a design ambiguity, it was a factual
question the task's own more precise wording (and the spec itself)
already answered unambiguously. Caught by a failing test, not by
re-reading the prose carefully enough the first time: implemented
negative-paints-first literally, wrote a test pinning "negative loses
to the box's own background," watched it contradict the *other* test
(seven-layer order, built straight from the spec's own layer list)
that already correctly expected negative to beat background — the two
tests couldn't both be right, which is what surfaced the inconsistency
concretely rather than in the abstract. The real rule, confirmed: a
`z-index: -1` descendant only looks "hidden behind everything" when the
box it's negative *relative to* happens to have no background of its
own — colloquial CSS folklore conflates that common case with the
actual rule.

**`paint`'s per-box order is now**: (1) this box's own background/
border: `paint_box_decoration`, unchanged in content, just reordered
relative to the next step; (2) `paint_z_layer` for the *negative* half
of this box's combined z-ordered descendants; (3) the ordinary
`Blocks`/`TableRow` child loop (floats already sorted to the end of
that same list by `measure`'s own v54 logic — layers 3/4 for free, with
no new code); (4) `paint_z_layer` again, for the *non-negative* half —
same call this box's own out-of-flow `positioned` list has always
painted via, just now also carrying the new in-flow stacking children
(see below). `laid.positioned` is already sorted ascending by
`z-index` from where it's built (v52) — `partition_point` finds the
zero boundary with no extra sort; `paint_z_layer` is one new helper
function, not duplicated at each of the two call sites.

**`position: relative`/`sticky` + an explicit `z-index` establishes its
own descendant stacking context (CSS2 §9.9.1) — `z-index: auto` does
not, unaffected.** The key realization: such a box is *already* 95% in
the right place architecturally, the same way v55 found sticky was 95%
`relative` — it already catches its own `absolute`/`fixed` descendants
as their containing block (any `position != static` box does, since
v52, regardless of its own `z-index`), so a nested descendant's own
explicit `z-index` was already correctly scoped to compete only within
that same local group. The only missing piece was the box *itself*:
with an explicit `z-index`, it needs to compete by that value against
its *own* siblings and its parent's other out-of-flow descendants,
instead of painting at its plain document-order position in the
`Blocks`/`TableRow` array the way `z-index: auto` still does (and did,
unconditionally, before this session). A new predicate,
`is_explicit_stacking_child` (`position: relative`/`sticky` *and*
`z_index.is_some()`), finds these in a `Blocks`/`TableRow` child list;
`paint`'s normal loop filters them out (so nothing is ever painted
twice), and `paint_z_layer` merges them with this box's own `laid.
positioned` entries into one combined, sorted presentation — `paint_z_
layer` distinguishes the two kinds of entry by `item.style.position`
alone (an out-of-flow `positioned` entry is always `absolute`/`fixed`;
an in-flow stacking child is always `relative`/`sticky` — the two sets
never overlap, so no wrapper enum was needed), applying the right
scroll-offset trick for each (`sticky_shift` for a sticky child, the
`fixed`-immune-to-scroll check for an out-of-flow entry). The in-flow
stacking child is **not** taken out of flow — it still contributes to
its parent's size and never displaces a sibling, exactly like a plain
`relative` box always has; only its *paint-order slot* moves.

**Is "two unrelated in-flow subtrees overlapping" actually reachable in
scratchy today? Yes, but only at sibling granularity, not generally.**
This session's task explicitly asked this be checked before building
any generality for it. Float/clear (v54) only narrows content *around*
a float within one inline formatting context — it doesn't let two
unrelated block-level subtrees occupy the same screen region. But a
`position: relative` box's own offset (`top`/`left`, resolved since
v52) *can* move a box into visual overlap with a plain sibling in the
same parent's `Blocks` list, entirely independent of float — proven
directly by this session's own `relative_box_with_explicit_z_index_
establishes_its_own_stacking_context` test (a `top: 30px` shift makes
an earlier box overlap a later, ordinary static sibling). That's the
concrete, reachable shape of "two things overlapping that document
order alone would get wrong" — scoped to exactly that (same immediate
parent, one box's offset reaching into a sibling's natural rectangle),
not a general "any two subtrees anywhere on the page" search, which
genuinely doesn't arise from anything scratchy's layout model can
produce today.

**What's deliberately still scoped out, each its own `TODO.txt` item**:
a `relative`/`sticky` box's own stacking context is anchored at its
*immediate* containing block only — it doesn't bubble past an
intervening `position`-ed-but-`z-index:auto` ancestor the way real
nested stacking contexts would (that ancestor, per spec, shouldn't
establish its own context at all, so a descendant's `z-index` should
really compete one level further up); not reachable by anything real
yet, so not built. `hit_test_cursor`'s own negative-`z-index` priority
doesn't fully mirror `paint`'s corrected order either — a negative
entry is still checked at the same first-priority pass as a
non-negative one (restructuring the whole function to return `Option`
throughout, so a "found nothing more specific" fallback could be told
apart from "found this box's own ambient cursor," was judged a
disproportionate risk for a narrow, rarely-observable case — same
"pick the narrower, lower-risk option and document the gap precisely"
call v31's own `overflow` scoping and v54's own float-hit-test gap
already made).

**Testing**: 4 new permanent tests in `mod tests` — `negative_z_index_
paints_above_its_containing_boxs_background_but_below_normal_content`;
`seven_layer_stacking_order_within_one_containing_block` (layers 1, 2,
3, and 7 pinned directly via 3 sampled pixels in one fixture, document
order deliberately not matching paint order so the test can't pass by
accident); `relative_box_with_explicit_z_index_establishes_its_own_
stacking_context`; and `relative_box_with_auto_z_index_does_not_
establish_a_stacking_context` (the explicit regression pin for the
"auto is unaffected" half of the feature). One real snag while writing
them, same lesson v51/v52 already taught and worth restating since it
bit again here: the first draft of every fixture had a literal newline
between tags, each becoming its own whitespace-only anonymous inline
box that silently shifted every box below it — caught by dumping the
actual `Laid` tree with a throwaway debug test rather than assuming the
fixture HTML was fine because it had "worked before." Also visually
confirmed with a real `grim` screenshot against a page combining all of
this: a negative-`z-index` box correctly showing through below an
in-flow sibling that only covers *part* of its area, and a `relative`+
explicit-`z-index` box correctly winning an overlap against a later
static sibling.

## v57 — flexbox (`display: flex`), grid deferred

Picked off `TODO.txt`'s `display: flex`/`display: grid` item, scoped
deliberately to flexbox only — grid (track sizing, template areas,
implicit/explicit grids, `fr` units, grid placement) is a materially
larger, separate algorithm and was split into its own, entirely
deferred `TODO.txt` item rather than risk landing neither well, the
same latitude the `position`/`float` items already used for their own
oversized original TODO wording.

**Architecture: a third `box_model_entry`/`display`-dispatch special
case, parallel to `Table`'s.** `Display` gained a `Flex` variant;
`build_box_for_element` special-cases it (right next to its existing
`Table` special case) to call a new `build_flex_children` instead of
`build_children_with_prefix`; `measure`'s own top-of-function dispatch
special-cases it the same way `Table` already is, calling a new
`measure_flex` before the generic per-`BoxContent` match even runs
(exactly `measure_table`'s own reason: real flex sizing is a container-
wide decision made before any one item can be measured, not a per-child
decision the generic `BoxContent::Blocks` loop could make locally).
`collect_children`'s existing block-vs-inline split (`matches!(style.
display, Display::Block | Display::Table)`) grew a third arm,
`Display::Flex`, so a flex container nested inside otherwise-inline
content still gets its own box — same treatment `Table` already has.

**`build_flex_children` needs almost no code of its own**, because real
CSS "blockifies" every flex item (spec §4 — a flex item's outer display
type is always block-level, regardless of its own `display`) *for
free* here: `build_box_for_element` already builds a full `Blocks`-
content box for whatever handle it's given, regardless of that
element's own `style.display` — it only ever branches on `display` to
special-case `img`/`Table`/`Flex` themselves, never to decide "does
this get its own box at all." Only `collect_children`'s inline-run
grouping makes that decision, and `build_flex_children` bypasses it
entirely by calling `build_box_for_element` on every child directly
(skipping `script`/`style`/`display: none`, same as `build_table_
children`). So a bare `<span>` (default `display: inline`) placed
directly inside a flex container still gets its own real box, exactly
like a `<div>` would, with zero extra "force block" logic needed. The
one real, documented gap this bypass leaves: a *bare text node*
directly inside a flex container (no wrapping element at all) doesn't
become its own anonymous flex item the way real CSS's §4 would — only
real elements are collected, an explicit scope-narrowing choice (see
`TODO.txt`).

**`measure_flex` splits into `measure_flex_row`/`measure_flex_column`**
— the two directions turned out to have a meaningfully different shape,
not just "swap width and height," so sharing one body would have meant
more branching than two focused functions. Both share two small, pure
helpers: `resolve_flex_main_sizes` (CSS Flexbox §9.7's resolving-
flexible-lengths step — proportional `flex-grow` distribution of
positive free space, proportional *scaled-shrink-factor* distribution
of a negative deficit; since Q23c it is the real iterative freeze-and-
retry loop, so clamped items are frozen and their violation is
redistributed) and `distribute_justify_
content` (the `justify-content` keyword → `(start_offset, extra_
between)` math, identical regardless of which physical axis is "main").

**The one insight that made cross-axis stretch/grow/shrink nearly free,
on *either* axis:** this engine's own `height` property already doesn't
affect how a box's *children* are laid out — only the box's own
*reported* size to its parent (v42: an explicit `height` overrides only
`Laid::h`, content still lays out in full against the ordinary content-
driven height and may spill past it). That means wherever a flexed/
stretched *final* size lands on whichever axis is physically *height*
for a given direction — row's own cross axis (stretch), or column's
own main axis (`flex-grow`/`flex-shrink`) — it's a pure post-hoc
`Laid::h` overwrite after the one real `measure()` call that already
happened, never a second, more expensive re-layout pass. *Width*, by
contrast, genuinely does affect children (text reflows at a different
width), so wherever a flexed size lands on the *width* axis — row's own
main axis, column's own cross axis — the decided size has to be chosen
*before* the one real `measure()` call, not patched after.

This asymmetry shapes each direction's own two-step structure:

- **Row** (the common case, and the one with no provisional pass
  needed at all for its own main axis): each item's hypothetical main
  size (its *basis*) is computed up front from `flex-basis`/`width`/
  natural content size — the last case reusing `cell_natural_width`
  (v50's min/max-content measurement, built for table-column sizing,
  already reused once for `float`'s own shrink-to-fit basis — now a
  third, unrelated use of the exact same helper) — *without* a full
  `measure()` call, since width is already definite for a row container
  (`entry.content_w`). `resolve_flex_main_sizes` turns those bases into
  final per-item widths; *then* each item gets its one real `measure()`
  call at that decided width. Only *after* that call (now knowing each
  item's natural height) does `align-items: stretch` kick in — a
  post-hoc `Laid::h` overwrite to the line's own cross size (the max
  natural height among that line's items), exactly the free-overwrite
  case above.
- **Column**: the opposite order. Cross-axis (width) sizing is decided
  *before* the one real `measure()` call (stretch/explicit-width items
  measure at the full `content_w`, letting `box_model_entry`'s own
  explicit-wins-else-auto-fills logic do the right thing for free;
  non-stretch `width: auto` items get an explicit shrink-to-fit width
  from `cell_natural_width` instead — reused a *fourth* time). Main-axis
  (height) sizing is the mirror image of row's own cross axis: there's
  no "natural content *height*" helper analogous to `cell_natural_
  width` (height is already the axis a normal block measures for free),
  so each item's one real `measure()` call *is* how its hypothetical
  main size is learned (`lc.h`, already correct for both the auto-
  content and explicit-CSS-`height` cases, since `measure` resolves
  `height` the ordinary way regardless) — then the *final*, flexed size
  is a post-hoc `lc.h` overwrite, same free case as row's stretch.

**`flex-wrap` is genuinely real, but only for `row`/`row-reverse`.**
Items are greedily packed into lines (CSS Flexbox §9.3 — a line never
starts empty, so one item wider than the whole container still gets a
line to itself rather than being dropped), each line's own main-axis
sizing resolved completely independently via the same `resolve_flex_
main_sizes`/`distribute_justify_content` pair a single-line (`nowrap`)
container uses, then lines stack top-to-bottom by their own cross size
with `row-gap` between them — `wrap-reverse` just reverses the *list of
lines*, not the items within one. A `column`/`column-reverse` container
always lays out as a single column regardless of the declared `flex-
wrap` — real column wrapping would mean wrapping across the *width*
into side-by-side columns, judged a separate, rarer shape not worth
this session's remaining budget (see `TODO.txt`).

**`row-reverse`/`column-reverse` are a pure post-placement mirror, not
a reordering.** Every item is positioned exactly as the normal
direction would, then — once every item's final `x`/`y` (row) or `y`
(column) is known — mirrored within the container's own main-axis
extent via `shift_laid_tree` (the same helper `resolve_positioned`'s
own `right`/`bottom`-solving and `float: right`'s own placement already
use for an identical "measured at a provisional position, shift once
the real extent is known" reason). Items stay in plain document order
in the returned `Vec<Laid>` throughout — `order` isn't implemented, so
there's never a reason to reorder the underlying list itself, only to
mirror where each item visually lands.

**`position: absolute`/`fixed` children are filtered out of the flex
item list before any of the above runs** — exactly the same split
`measure`'s own `BoxContent::Blocks` arm already makes for an ordinary
block's out-of-flow children (collected into `own_pending`, resolved or
bubbled via the same `resolve_or_bubble_positioned` every other
container already uses), since real CSS §4 says they're not flex items
at all. A flex container is its own fresh BFC (`Floats::default()`
passed into every item's own `measure()` call), same category as a
table or a table cell's own interior.

**One real, deliberate narrowing worth flagging precisely**, found
while writing the permanent tests, not guessed at in advance: on a
`row`/`row-reverse` container, an item's final flexed width is passed
as `measure`'s own ordinary `width` parameter (the same convention
`lay_out_row_cells` already uses for a table cell's column-decided
width) — correct whenever the item's own CSS `width` is `auto` (the
common case for a flex item relying on content/`flex-grow`/`flex-
shrink`, since `box_model_entry`'s auto-fill then naturally lands on
exactly the passed-in value), but an item that *also* declares an
explicit, conflicting CSS `width` keeps that literal value instead —
`clamp_box_dim` ignores the passed-in basis entirely once `s.width` is
`Some`. This is the exact same accepted simplification `measure_
table`'s own per-cell-`width`-vs-column-width gap already has (`TODO.
txt`), not a new category of cut — confirmed only while writing the
`flex-shrink` test (the first draft declared explicit `width: 100px` on
each item and the shrink math silently did nothing at all, since the
items' own *basis* came from that explicit width, not their flex-basis
— fixed by switching the test to `flex-basis: 100px`, which doesn't set
`item_style.width` at all and so isn't subject to this gap). Column's
own equivalent (its main axis is *height*, which — per the asymmetry
above — is always a post-hoc `Laid::h` overwrite) has no such gap; the
override always applies there regardless of an explicit CSS `height`.

**Testing**: 10 new permanent tests covering exactly what this session's
own task called for — row vs. column direction (item ordering and
cross-axis alignment), `justify-content: space-between` pushing items
to opposite edges, `align-items: stretch` growing a shorter sibling to
match the tallest, `align-items: center` positioning without resizing,
`flex-grow`/`flex-shrink` genuinely proportional distribution (not just
"fits content" — verified with asymmetric 1:2 and 1:3 ratios producing
exactly the expected split, not merely "grew/shrank at all"), `flex-
wrap` starting a second line once items overflow, `gap` adding fixed
spacing on top of `justify-content`, and `position: absolute` correctly
pulling a child out of the flex item list into the container's own
`positioned` list instead. A few early drafts of these tests had real
bugs of their own, not just typos — worth recording as this session's
own version of the "test the test" lesson v19/v51/v56 already
established: an `align-items: center` assertion against an *absolute*
document-space `y` broke the moment `<body>`'s own 8px default margin
(`default.css`) shifted everything uniformly, fixed by asserting a
*relative* offset between siblings instead; a `justify-content`
assertion against a literal `x == 0.0` had the same problem, fixed by
giving the fixture's own `<body>` an explicit `margin: 0`.

**Visual verification.** Three real test pages (a row with `flex: 1 0
0`/`flex: 2 0 0` growing children, a centered `flex-direction: column`,
and a `flex-wrap: wrap` row forcing a second line) all rendered with
correct *text/geometry* positions via a real `grim` screenshot —
confirmed the row's items landed left-to-right at the right x-offsets,
the column's items stacked top-to-bottom with narrower items visibly
more indented (wider `align-items: center` offset) than wider ones,
and the wrap case put two items per line across two visibly separate
rows.

**Correction, found in the very next session:** the v57 work above
also reported *no* `background-color` painting at all in any of its
three screenshots, investigated via an unmodified-`main` A/B test on a
trivial unrelated `<div style="background: red">`, and concluded (in
good faith, having actually checked rather than assumed) that this was
a pre-existing environment/compositor limitation rather than a v57
regression. That conclusion was itself wrong: re-run immediately
afterward, the identical trivial `<div style="background-color:
#ff0000">` test rendered a clean, fully opaque red square via a real
`grim` screenshot — no limitation, no missing paint. Likely a one-off
timing/focus artifact in that specific sandbox session (e.g. a
screenshot grabbed before the first real paint, or the wrong/stale
window), not a reproducible bug of any kind. Left here instead of
silently deleting the original paragraph so a future session doesn't
waste time assuming `grim`-vs-background-color is a known limitation —
it isn't; re-verify live if it ever looks broken again rather than
trusting this note either way.

## v58 — `display: grid`

Picked off `TODO.txt`'s `display: grid` item — the one v57 explicitly
split out of flexbox's own scope. Architecturally this is a close
sibling of both `measure_table` (v50) and `measure_flex` (v57), reusing
pieces of each rather than growing fresh machinery:

**Same three-way special-case shape `Table`/`Flex` already
established.** `Display::Grid` is block-level in the outer flow
(`collect_children`'s existing `Display::Block | Display::Table |
Display::Flex` check grew a fourth arm), and both `build_box_for_
element` and `measure` special-case it *before* their generic
dispatch, exactly the way `Table`/`Flex` already do — real grid track
sizing is a container-wide decision (every column's width depends on
the whole track list and the container's own content width) that can't
be made per-child inside the generic `BoxContent::Blocks` loop.

**`build_grid_children` needed no code of its own at all** — it's
`build_flex_children`, called directly from the `Grid` branch. Real CSS
"blockifies" every flex *and* grid item identically (spec §4/§3: a flex
or grid item's outer display type is always block-level, regardless of
its own `display`), and `build_flex_children`'s own logic was already
generic enough ("call `build_box_for_element` on every direct element
child, skip `script`/`style`/`display: none`") that it needed no
flex-specific behavior to begin with — only its name and doc comment
needed updating to reflect the second caller. Worth remembering as the
general lesson here: before writing a near-duplicate "building" function
for a new container type, check whether the existing one's logic is
actually container-specific or just happened to be named after the
first container that needed it.

**Column tracks are a simpler problem than table columns ever were —
no content measurement needed at all.** `measure_table`'s whole reason
for `cell_natural_width`'s two-pass min/max-content measurement is that
real CSS auto table layout sizes columns *from their cells' own
content*. Grid's `grid-template-columns`, by contrast, only supports
plain fixed/`%`/`fr` tracks in this implementation (no `auto`/`min-
content`/`max-content` keywords — see `TODO.txt`) — so a column's width
is decided purely from the declared track list and the container's own
`content_w`, with zero shaping/measurement needed before any item can
be placed. This is the one piece of this feature that's *simpler* than
its table/flex prior art, not harder.

**`resolve_grid_tracks` is grid's own version of `resolve_flex_main_
sizes`** (v57) — same shape of problem (distribute a definite extent
across a list of fixed-and-flexible entries), same single-pass
simplification (no real spec iterative constraint resolution — nothing
here has a `min`/`max` to clamp against in the first place, since a
plain fixed/`fr` track has no such concept, so this one's actually
*simpler* than flexbox's own version): every non-`fr` track takes its
own resolved size first (`%` against `available`, the real containing-
block-width basis), then whatever's left over (floored at zero) splits
across the `fr` tracks in proportion to their own factor.

**Implicit auto-placement is a flat two-pass walk, not a real CSS Grid
Level-1 placement algorithm** (which also has to account for explicitly
placed items occupying cells out of order, named lines, spanning
items, etc. — all deliberately out of scope here, see `TODO.txt`):
collect every non-positioned child in document order, then bucket them
into rows of at most `num_cols` each via a single linear pass (`rows:
Vec<Vec<&LayoutBox>>`, a new row started whenever the current one has
filled). No cell-occupancy grid/bitmap of any kind exists — nothing
needs one yet, since every item is exactly 1×1 and placement is purely
sequential.

**The one real architectural subtlety, caught by a failing test before
anything was committed**: `measure_flex`'s own trick for computing a
container's final content height — derive it from the *laid items'*
own `y + h` extents, rather than tracking an accumulated cursor
directly, which conveniently also sidesteps a trailing-gap-after-the-
last-line bug — does **not** carry over to grid. It works for flexbox
because a line's cross size is always defined as exactly its tallest
item's own extent, so deriving from items is equivalent to deriving
from the line's own cross size. Grid breaks that equivalence: an
*explicit* `grid-template-rows` entry can declare a row taller than
every item placed in it (real CSS: the item still just sits at the
row's own top edge, same "stretch is default, but nothing forces an
item to actually fill the extra space" story items 6's own cut already
documents), and that extra space never appears in any item's own
measured `y + h` — only in the row's own `row_h`, which the items
themselves know nothing about. `measure_grid` instead tracks each row's
real bottom edge directly (`content_bottom`, updated once per row, set
*before* `row_gap` is added so the final value never includes a
trailing gap after the last row) rather than deriving it after the
fact. Caught immediately by `grid_explicit_row_track_sizes_the_row_
regardless_of_content` (first run: container height 10px instead of
the expected 80px) before this was committed — worth remembering
alongside v19/v51/v56's own "test the test" lessons: a pattern that's
correct for one container type isn't automatically correct for a
structurally similar one just because the shapes look alike on the
surface.

**Row-track `fr`/`%` sizing deliberately falls back to content-based
auto sizing, even when `grid-template-rows` explicitly names one** —
not a guess, a direct application of v49's own already-established
policy ("a percentage against an indefinite basis computes as if it
weren't declared") extended to `fr`: real CSS Grid distributes a
flexible row track's own share of the grid container's *own* definite
height, but for an ordinary `height: auto` grid container (the
overwhelmingly common real case) that height is itself almost never
known before layout — the identical "no free space to distribute"
situation flexbox's own `measure_flex_column` already has for an
auto-height column container's main axis. A **fixed** (`px`/`em`/
`rem`) row track needs no such basis and is sized exactly as declared
regardless, content overflowing past it if too short (the same
`overflow: visible`-by-default consequence an explicit CSS `height`
already has on any other box, since v42) — only `%`/`fr` row tracks
hit this gap. Column tracks never do, since a grid container's own
content *width* is always definite — which is exactly why `fr` was
prioritized there as this item's single most load-bearing primitive,
per the task's own wording, and landed fully real.

`gap`/`row-gap`/`column-gap` needed zero new code — confirmed generic
(not flexbox-specific) before writing anything, per this item's own
explicit instruction to check first: `ComputedStyle`'s `row_gap`/
`column_gap` fields and their `DeclParser` branches already existed
from v57 with no flexbox-specific framing, so `measure_grid` just reads
`s.row_gap`/`s.column_gap` directly, the same fields `measure_flex`
reads.

## v59 — `word-spacing`, and why it succeeded where `letter-spacing` didn't

Last item of the old misc-CSS-properties cluster. `TODO.txt`'s own
wording for this item (written back when `letter-spacing` was reverted)
assumed it would be *harder* than `letter-spacing`, since cosmic-text
0.19's `Attrs` has no `word_spacing` knob at all — `letter_spacing_opt`
is genuinely the only tracking-style field it exposes (confirmed again
by grepping its vendored source; unchanged since that was first noted).
That's true, but it turned out to be the wrong axis to worry about: the
actual blocker for `letter-spacing` was never "cosmic-text has no API
for this" (`Attrs::letter_spacing()` is a real, documented API, and
wiring it in worked fine in isolation) — it was that feeding spacing
into cosmic-text's own shaping made its *internal* wrap-width
computation get a word's width wrong, shattering a short paragraph into
one line per word. `word-spacing` never had the option to make that
specific mistake, since there's no `Attrs` knob to feed it into in the
first place — the real question was whether an entirely different,
hand-rolled approach could land the same visible effect without routing
through cosmic-text's shaping/wrap internals at all.

**It can, because v48/v54 already built the exact machinery needed,
for an unrelated reason.** `measure_mixed_inline` (v48, for inline
`<img>`) is a hand-rolled, word-by-word greedy line-breaker that shapes
each word independently (via `shape_spans`, one tiny one-word paragraph
at a time) and positions each word's own x-offset itself, entirely
outside cosmic-text's own paragraph-level `Buffer`/`Wrap` logic. v54
extended routing into this path to cover any plain-text run sharing a
BFC with an active float, not just an image. Since this path already
splits text into words and already does its own position arithmetic,
adding extra horizontal space between words there is **a position-
calculation change in scratchy's own code** — the same category of
change `float`'s own per-line corridor narrowing (v54) already is, not
a shaping change. cosmic-text's per-word `shape_spans` call still
measures each word at its own natural width, with zero knowledge that
`word-spacing` exists; only the number scratchy adds to that word's own
advance afterward changes. There's no cosmic-text-internal wrap-width
computation left in this path for that extra number to confuse, because
cosmic-text isn't doing any wrapping in this path at all — scratchy's
own greedy line-breaker is.

**The routing extension** needed to make this reachable mirrors v54's
own float-routing change almost exactly: `measure`'s `BoxContent::
Inline` match arm already had two disjuncts gating entry into
`measure_mixed_inline` ("contains an image," "an active float overlaps
this run's own starting `y`") — this added a third, "any span in this
run has a nonzero `word_spacing`," checked per-span (not against the
run's own anonymous-box style) since a run can mix spans from different
elements with different `word-spacing` values. `word-spacing: normal`
(represented as `0.0`, same as an unset value) never triggers this, so
an ordinary paragraph stays on the fast single-`Buffer` path with zero
added cost — the same "only pay for what you use" guarantee v48/v54
already established for images/floats.

**Where the spacing is actually added**, inside `measure_mixed_
inline`'s own word-measurement loop: a word's shaped width gets its
originating span's `word_spacing` added directly onto it, whenever that
word's own split token ends in the trailing space that marks a real
word boundary (the last word in a run — no trailing space — correctly
gets none). This is baked straight into the `Measured::Word`'s own
`width` rather than carried as a separate field threaded through every
downstream site: the wrap-check/cursor-advance in the line-breaker's
first pass, the final `text-align` offset math, and
`MixedPositioned::Word`'s own hit-test bounding box all already treat a
word's `width` as "how much horizontal room this word needs before the
next one starts," so one inflated number covers all three for free.
This is also, deliberately, the exact same treatment this function's
own docs already gave a word's *ordinary* trailing space before this
item existed (a documented, pre-existing simplification: "a line's
trailing space counts toward width... unlike a real UA which hangs it
past the edge") — extended uniformly rather than invented fresh.

**A real test-design trap, caught before shipping**: the first version
of the core geometry test tried to measure "the gap from word 1's own
right edge to word 2's left edge" and got exactly `0` — which looks like
a bug, but is the predictable consequence of baking the extra spacing
into word 1's own `width` (as above): the space is now *inside* that
word's own reported extent, not a separate quantity sitting after it.
Fixed by comparing word 2's absolute `x` position across two different
declared `word-spacing` values (`10px` vs `30px`) instead, and asserting
the delta is exactly `20px` — a cleaner, more direct proof that the
position math scales exactly with the declared value. Worth remembering
as a general lesson for this file's own "bake the extra into an existing
width field, don't add a new one" pattern (used again here after
`float`/`measure_mixed_inline` style precedent): a test that assumes the
extra quantity is still measurable as a separate "gap" after the fact
will quietly measure `0` and look like a regression, not a test bug,
unless you trace through where the number actually ended up.

**Verification deliberately modeled on `letter-spacing`'s own TODO.txt
entry** — reproducing its exact numbers, not just writing generic
tests, to make the comparison as direct as possible: the same 900px-wide
short-paragraph scenario that shattered `letter-spacing` into 2 lines at
3-4px and 4 lines at 8px still fits on exactly 1 line at both 2px and
8px of `word-spacing` here. A second test uses a longer, realistic
multi-sentence paragraph at a typical 600px width with a typical 3px
`word-spacing` value and confirms the resulting wrap still averages well
over 3 words per line, rather than degrading toward one-word-per-line.
5 new permanent tests total; all 108 (103 prior + 5 new) pass; `cargo
clippy` stayed clean (diffed its exact warning locations against a
stash of this change to confirm nothing new was introduced — the
pre-existing warning count shifted only by line-number offsets).
Visually confirmed with a real `grim` screenshot: three paragraphs
(`word-spacing: normal`/`20px`/`2px`) all wrap naturally across several
lines with no shattering, with visibly wider word gaps and correspondingly
more wrapped lines at the larger declared value — exactly the shape real
CSS `word-spacing` produces, not a flat/no-op rendering.

**Why this is the right place to note the contrast with `letter-
spacing` for future reference**: both properties are genuinely CSS-
inherited, length-only, no-cosmic-text-API-for-either properties — on
paper, close cousins. The difference that mattered wasn't the property
itself, it was *where* the extra spacing got applied: `letter-spacing`
fed it into cosmic-text's own per-glyph shaping (`Attrs::
letter_spacing()`), inside the exact subsystem responsible for its own
internal wrap-width math, and that's where the bug lived. `word-spacing`
never touches `Attrs`/`shape_spans`'s *shaping* at all — only the
position scratchy's own hand-rolled line-breaker already computes by
hand. The general lesson for a future length-ish text property: if
cosmic-text has a real, working API for it (`letter-spacing` did), that
doesn't automatically make it the safer implementation path — check
whether applying it earlier, in scratchy's own already-hand-rolled
position math (where one exists), avoids feeding cosmic-text's internal
layout algorithms something new to get wrong, even if that means a
slower code path for runs that use the property.

**v60 closed the remaining `text-decoration-style`/`-color` gaps left
over from v33** — `wavy`, `double` for strikethrough/overline, and
independent `text-decoration-line`/`-style`/`-color` longhands.

The wavy stroke is the one piece worth a real architecture note:
cosmic-text's `Renderer` trait (`src/render.rs` in the vendored 0.19.0
source) is genuinely minimal — `rectangle(x, y, w, h, color)` and
`glyph(...)`, nothing else. No path, no bezier, no polyline. So a sine
wave is built the same way `Dashed`/`Dotted` already turn one long line
into several small rectangles (v33) — except instead of a fixed gap
pattern, each narrow segment's own `y` is offset by `amplitude *
sin(2π · center_x / wavelength)`, where `center_x` is that segment's
own midpoint along the stroke. Segment width is `thickness * 0.5`
(floored at 1px) — narrow enough that the piecewise-rectangle
approximation reads as a smooth curve at normal font sizes, without
being so narrow it meaningfully hurts paint performance (a typical
decoration span is at most a few hundred pixels wide, so a few hundred
tiny `rectangle` calls per span is the realistic worst case, same order
of magnitude `Dotted` at small font sizes already produces). Both
amplitude and wavelength scale off `thickness` (the stroke's own line
weight, already derived from font size) rather than fixed pixel
constants — the same reasoning `Dashed`/`Dotted`'s own mark/gap sizing
already used, so the wave doesn't look disproportionately large or
cramped at very small or very large text.

**The `double` fix turned out to be a simplification, not new
complexity, once traced through carefully.** The pre-v60 code had
`double` bypass `DecorationStyle` entirely for underline — `parse_text_
decoration` set cosmic-text's own `deco.underline = UnderlineStyle::
Double` directly, relying on `paint_decorations`'s existing `match
td.underline { ... UnderlineStyle::Double => { two rectangles } }` arm.
That arm's own two-rectangle formula already does the real work; the
only reason it was underline-only is that cosmic-text's `TextDecoration`
type has no `Double` concept for strikethrough/overline, and the old
code piggybacked on cosmic's own enum instead of using this file's own
`DecorationStyle` (which *can* apply uniformly, since `paint_decoration_
line` is called identically for all three line types already). Re-
reading `paint_decorations`'s own v33 doc comment made this obvious: it
never actually calls cosmic-text's runtime renderer at all — it's a
complete hand-written reimplementation that only *mirrors* cosmic's
offset/thickness/color formulas for pixel-identical `Solid` output. So
there was no real dependency on cosmic's own `UnderlineStyle::Double` to
preserve — moving the "two rectangles, thickness + gap apart" logic into
`DecorationStyle::Double` (handled once, inside `paint_decoration_line`
itself, via a recursive call with `Solid`) and having `paint_decorations`
apply it the same way to underline/strikethrough/overline cost nothing
and removed a special case, rather than adding one. `UnderlineStyle::
Double` is still matched defensively in `paint_decorations` (treated as
"force `Double`, even if `style` itself isn't"), purely so nothing breaks
if some future code path ever sets it independently — but neither the
shorthand nor the new longhands produce it anymore.

**The independent longhands needed one real design decision: how to
combine three separately-cascaded components with the shorthand's own
single bundled value and the ambient-propagated decoration an element
inherits.** Resolved with `resolve_own_decoration(style, ambient)`,
which replaces the three `style.text_decoration.unwrap_or(decoration)`
call sites `build_box_for_element`/`build_table_children`/`collect_
children` already had: start from `ambient` or the shorthand's own full
replacement (unchanged from before), then let any declared `-line`/
`-style`/`-color` longhand override just its own one piece on top of
that. The combined *result* threads on as the new ambient value for that
element's children, exactly the way the shorthand's own combined value
already did — so no new parameter had to be threaded anywhere; only the
one combine step at each existing `unwrap_or` site changed. A `-color`
longhand with no line active anywhere (no shorthand, no `-line`
longhand, nothing ambient) correctly stays invisible — same "don't leave
a stray color on an inactive line" rule `parse_text_decoration` already
enforced for the shorthand's own inline color token, now also holding
across the three longhands combined.

**v61 extended `cursor` (v32) from `auto`/`pointer` to the full real CSS
keyword set** — the next item in the same autonomous `TODO.txt` loop,
right after v60. Worth recording the one fact that made this cheap: real
CSS's `cursor` keyword grammar and winit's own `CursorIcon` (the
`cursor_icon` crate, checked at its vendored 1.2.0 source) line up
almost name-for-name — every real keyword this item added had an exact
winit equivalent, so there was no mapping judgment call to make for any
of them. The one keyword that genuinely doesn't fit the "pick a
`CursorIcon`" model is `none` — real CSS defines it as "hide the cursor
entirely," which isn't an icon at all. Modeled that as `to_winit()`
returning `Option<CursorIcon>` (`None` = hide) rather than inventing a
placeholder icon or quietly dropping the keyword; the live handler
(`WindowEvent::CursorMoved`) now branches on this to call `window.
set_cursor_visible(false)`/`(true)` instead of `window.set_cursor(..)`
at exactly the hide/show transitions, keeping the existing "only touch
the OS cursor state when it actually changed" discipline `last_cursor`
was already there for (now `Option<CursorIcon>`, to carry the
hidden state too).

**No change needed anywhere in the hit-testing/propagation path** —
`hit_test_cursor`, the per-glyph `cursors: Vec<CursorStyle>` threading
through `shape_spans`, and `cursor`'s real CSS inheritance through
`InheritedStyle` were all already written generically against
"whichever `CursorStyle` this span/element resolves to," per v32's own
original design. Confirmed this rather than assumed it still held after
tripling the enum's size: a new full-pipeline test uses `cursor: grab`
(a keyword that didn't exist before this item) and checks `hit_test_
cursor` still reports it correctly at real glyph precision, alongside an
adjacent plain-text glyph still reporting `Auto` — same diligence v32's
own `Pointer`-vs-`Auto` test used, just against a keyword picked
specifically because nothing in the original implementation could have
been coincidentally tuned for it.

Also worth noting for whoever eventually tries to live-verify this (or
`position: sticky`'s own analogous gap, v55): every piece of `cursor`
that doesn't ultimately bottom out in an OS/compositor call is now
headlessly verified in as much depth as this codebase's testing
conventions go — parsing, cascade/inheritance, the `CursorStyle`→
`CursorIcon` mapping (including the `none` special case), and glyph-
precision hit-testing. The only thing still unverified live is the
mechanical, thin layer this sandbox has no tool to exercise end-to-end
(no synthetic mouse-move event, and `grim` can't see a compositor-drawn
cursor icon regardless) — not because nobody checked, but because
nothing in this environment *can* check it.

## v62 — `overflow: scroll`/`auto`: a per-element scroll offset, the same two-stage split `position: sticky` already established

Picked off `TODO.txt`'s next item: `overflow` (v31/v40) only ever
supported `visible`/`hidden` — this adds real `scroll`/`auto`, where an
overflowing element becomes its *own* independently-scrollable region
with a live, mouse-wheel-interactive scroll offset, not just a visual
clip.

**Why this didn't need an `AskUserQuestion` the way `position: sticky`
(v55) did.** v55's own writeup flagged a direct conflict with this
engine's oldest performance invariant — "scrolling never touches
layout" (see the v4/v5 writeup at the top of this file) — because a
naive sticky implementation needs to re-evaluate position on every
scroll tick. This item looked like it might hit the exact same wall
(a per-element scroll offset sounds like it obviously needs layout
involved), but it doesn't: `position: sticky`'s own split — resolve
*bounds* once at layout time (`StickyInfo`, stored in `Laid`), read a
*live* value fresh every `paint`/`hit_test_cursor` call (never stored
in `Laid`, so `measure` genuinely never touches it, a compile-time
guarantee since `paint`/`hit_test_cursor` take `&Laid` not `&mut Laid`)
— generalizes directly to a per-element scroll offset. The only real
difference: sticky's own "live value" is the page's single existing
`scroll_offset`, already a field that persists across frames; a
per-element scroll needs its *own* per-id persisted value instead,
which is the one genuinely new piece of state this item adds
(`App::scroll_offsets: HashMap<usize, f32>`). No design conflict
surfaced, so this proceeded without asking — a size/scope question
(same latitude `position`/`float`/flexbox/grid got), not a flagged
tradeoff.

**`ScrollInfo { id, content_h, client_h }`** is the `StickyInfo`
analogue, stored as `Laid::scroll: Option<ScrollInfo>`. `id` comes from
the *same shared `next_id` counter* `measure`'s own paragraph ids
already use for keying `ShapeCache` — a different, unrelated-by-value
namespace (scroll offsets live in their own separate `HashMap`), so no
new `&mut usize` parameter had to be threaded through every recursive
`measure` call just for this; reusing an existing thread was simpler
than inventing a parallel one. `content_h`/`client_h` are values
`measure`'s own `BoxContent::Blocks` arm already computes for its
ordinary height logic (`client_h` is literally that arm's own
`border_h - extra_h`) — `max_scroll()` is just `(content_h -
client_h).max(0.0)`, `0.0` exactly when nothing actually overflows
(the real distinction `auto` needs to know whether to show a
scrollbar at all).

**Scoped to one arm only, deliberately**: `measure`'s plain
`BoxContent::Blocks` arm assigns `ScrollInfo`; `measure_table`/
`measure_flex`/`measure_grid` (each their own separate tail, not this
arm) don't — so `overflow: scroll`/`auto` on a table/flex/grid
container's own box does nothing yet. Same "narrowest real choke point
over the general case" precedent `overflow: hidden`'s own v31/v40
scoping already set, not a new kind of cut.

**The live value lives in `App`, never in `Laid`.** `App::
scroll_offsets: HashMap<usize, f32>`, read fresh (and re-clamped
against `max_scroll()` at the read site — self-healing if a relayout
ever shrinks `content_h` out from under a stale, larger stored value,
rather than trusted blindly) by `paint`, `hit_test_cursor`, and the new
`hit_test_scroll_region`. Cleared alongside `shape_cache` whenever the
layout tree rebuilds (ids are only stable within one tree's lifetime,
same as `ShapeCache`'s own paragraph ids already were) — stale entries
after a rebuild would just be harmless dead weight otherwise, but
clearing avoids unbounded growth across many stdin chunks in streaming
mode.

**Composing with `fixed`/`sticky`'s own "ambient offset shifts a
subtree" trick, extended rather than reinvented.** `paint` computes
`own_scroll` (this box's own live, clamped amount) and folds it into
`content_scroll_offset = scroll_offset + own_scroll`, passed down
*only* into this box's own ordinary `Blocks`/`TableRow` children loop
— not into its own `draw_y`, since scrolling moves a box's *content*,
never the box itself (the one real structural difference from `sticky`/
`fixed`, which shift the *whole* subtree including the box). This is
exactly the same shape `fixed` (forcing the ambient to `0.0`) and
`sticky` (`scroll_offset - sticky_shift(...)`) already established —
just a different adjustment, applied once per scrollable box instead
of once per sticky child. A nested `position: sticky` descendant's own
`sticky_shift` call automatically picks up the adjusted ambient (it
just reads whatever `scroll_offset` is lexically in scope at its own
recursion depth) with zero special-casing needed — and, by the same
mechanism, a scroll region nested inside *another* scroll region
should compose correctly too, architecturally, though (see `TODO.txt`)
that specific case has no dedicated test yet.

**Clipping reuses `overflow: hidden`'s existing machinery completely
unchanged** — `Scroll`/`Auto` just join `Hidden` in the one `matches!`
check `paint`'s own `child_clip` computation already does. v31/v40's
"an element's own `overflow` clips its descendants, never itself" rule,
and the border-box (not strictly padding-box) clip bounds, both carry
over exactly as before — no new clip logic, just a wider match arm.

**Wheel-event routing (`hit_test_scroll_region`)** walks the `Laid`
tree the same shape `hit_test_cursor` (v32/v61) already does — same
coordinate conventions, same `position: sticky` child-rect translation
reused verbatim — but returns `Option<ScrollInfo>` (the deepest
scrollable *ancestor* under the point) instead of resolving by
inheritance the way `cursor` does. There's no "inherited scroll
region," so a miss at one level has to explicitly bubble to its own
caller via `Option::or` rather than falling back to an ambient value —
the real "wheel bubbles to the nearest scrollable ancestor, then to the
page" behavior. `App::last_mouse_pos` is new state: `winit`'s own
`WindowEvent::MouseWheel` carries no cursor position at all, so routing
needs whatever `WindowEvent::CursorMoved` last reported, remembered
separately — `None` (never moved over the window, or not over any
scrollable region) falls back to the pre-existing page-level
`scroll_offset` logic unchanged.

**Deliberately narrower than the general case, each cut precise, not
silently folded into "done"**: an explicitly-stacked (`position:
relative`/`sticky` with its own `z-index`) or out-of-flow (`position:
absolute`/`fixed`) descendant *inside* a scroll region doesn't scroll
along with it — `paint`'s own `paint_z_layer` calls and `hit_test_
cursor`'s/`hit_test_scroll_region`'s own "above" stacking-children
loops deliberately keep using the un-adjusted ambient, the same
documented cut shape several other features here already accepted for
similar out-of-flow edge cases.

**`auto`'s only real difference from `scroll`**: scrollbar visibility,
not clipping or scrollability — both clip+scroll identically whenever
content overflows (confirmed against the task's own framing before
writing any code, not assumed). `paint_scrollbar`'s own
`show_when_not_overflowing` flag (`s.overflow == Overflow::Scroll`)
is the entire difference: `scroll` always paints a track+thumb,
`auto` only once `max_scroll() > 0.0`. The indicator itself is a flat,
simple `fill_rect`-based primitive (same spirit as box-shadow/border/
outline), painted as an *overlay* — it doesn't narrow `content_w` at
layout time at all, matching the modern OS overlay-scrollbar
convention rather than a classic reserved-space one, and keeping
"scrolling never touches layout" extended to this box's own width too.
Drag-to-scroll on the thumb itself is explicitly out of scope — wheel
scrolling is this item's own interactive core.

Verified with 5 new permanent tests (the real pipeline, not just
`ScrollInfo`'s own arithmetic in isolation) plus a real `grim`
screenshot (clipping, and both the always-shown `scroll` scrollbar and
the correctly-absent `auto` one, at the pre-scroll/zero-offset
position). Two real bugs caught while writing the tests, not by luck:
`background: blue` (the shorthand — this engine only ever implemented
`background-color`, never the shorthand) silently dropped instead of
erroring, caught only because the test's own assertion failed for a
reason that didn't match the code path being tested; and this test
harness's own fresh `vec![0u32; ...]` pixel buffers default to *black*,
not white (`App::redraw`'s own render loop fills white explicitly
before painting — a from-scratch test buffer doesn't get that for
free), which made a naive `!= 0x00FFFFFF` "still unpainted" check
report false positives everywhere until the actual pixel values were
printed and inspected directly rather than trusted from the first
failure's own assertion message.

**Not live-verified interactively, same category as v32's `cursor`,
v55's `position: sticky`, and v61's extended cursor keywords**: this
sandbox still has no tool to synthesize a real mouse-wheel event, so
the actual "wheel over a region scrolls just that region, then the
page" behavior was only exercised by the permanent headless tests
(the real `paint`/`hit_test_scroll_region` code paths, just with a
synthetic scroll offset standing in for a live wheel event) — worth a
real interactive check next time a human's at the keyboard, same
standing note every prior scroll/cursor-adjacent item here has left.

## v64 — `@media`, scoped to viewport-width breakpoints

Reprioritized ahead of the ordinary autonomous `TODO.txt` loop because
of a real investigation: rendering actual `en.wikipedia.org` through
`snag | scratchy` and grepping the real fetched CSS found that the
page's entire desktop two-column skeleton comes from one `@media
screen and (min-width:1120px) { .mw-page-container-inner { display:
grid; grid-template-areas: ... } }` rule that never fired — `@media`
was dropped outright at parse time, and `grid-template-areas`/
`grid-area` placement was already out of scope from v58. Both got
user sign-off to jump the queue; this item is `@media` only (see
PROMPTS.md's v64 entry for the full story and TODO.txt for exactly
what's still deferred).

**Representation: a rule carries its own condition, not a separate
side table.** `StyleRule` gained `media: Vec<MediaQuery>` — empty (the
default for anything outside `@media`, unchanged from before this
item existed) always applies; non-empty applies if *any* entry matches
(`media_list_matches`, comma-separated = OR). Each `MediaQuery` is one
`and`-joined item: a `MediaType` (`All`/`Screen`/`Print`) plus optional
`min_width`/`max_width: Option<f32>` (resolved to a plain px number
*at parse time* — a media feature has no element/cascade context to
defer an `em`/`rem` resolution to the way `BoxLength` does elsewhere in
this file, so `media_feature_length_px` resolves immediately against
the same `16.0` UA-default root font-size `ROOT_STYLE` uses) and
optional `prefers_color_scheme`/`prefers_reduced_motion`. A `never:
bool` flag, once set, makes the whole entry permanently non-matching —
the single mechanism behind every "safely drop, don't crash or guess"
outcome in this item: an unrecognized media type, an unsupported
feature, an uncomputable value (`calc()`, this project has no
arithmetic evaluator anywhere — confirmed by grep before writing any
parsing code, not built for this item), or `not` (negation,
recognized syntactically but not actually evaluated) all just set
`never = true` and let the rest of parsing continue rather than
aborting. This mirrors real CSS Media Queries syntax directly, not
just an engineering convenience: the spec itself defines an individual
query that fails to parse as evaluating to "not all," so "safely never
matches" is the spec-accurate outcome, not a lossy approximation.

**Parsing: `TopLevelParser`'s `AtRuleParser` impl, previously relying
entirely on the default (reject-everything) trait methods, now has
real ones for `@media`.** `parse_prelude` recognizes the name
case-insensitively and hands off to `parse_media_query_list`, a
hand-written, deliberately permissive scanner over the already-
delimited prelude tokens (`and`/`only`/`not`/a known media type keyword
update `MediaQuery`'s fields in place; a `(` opens a `parse_nested_
block` call into `parse_media_feature`, which reuses `parse_length`
outright — checked first that it already accepts the needed unit forms
— for the `min-width`/`max-width` value grammar). Any single
unrecognized token at the top level is just skipped rather than
aborting the whole list, so a real, mostly-standard media query isn't
thrown away wholesale over one clause this engine doesn't understand.
`cssparser`'s own `parse_nested_block`/`parse_until_before` already
self-heal past a partially-consumed inner block (confirmed by reading
`cssparser` 0.38's vendored source before relying on it) — e.g. when
`parse_length` bails out on a `calc(` function token without
descending into it, the outer block-skip logic still consumes through
to the matching close-paren on its own, so there's no leftover-token
risk from an early `Err` return anywhere in this new code.

**One at-rule, several real rules — the one place the existing
single-`StyleRule`-per-item shape had to change.** `StyleSheetParser`
requires `AtRuleParser::AtRule` and `QualifiedRuleParser::QualifiedRule`
to be the *same* type (confirmed by reading its own trait bounds) —
previously both were plain `StyleRule`. An `@media` block can expand
into any number of real rules, so both became `Vec<StyleRule>` (an
ordinary qualified rule now just returns a one-element `Vec`), and
`parse_block` for the at-rule recurses with a fresh `TopLevelParser`
over the same block-delimited `input` — reusing the exact top-level
rule-list grammar instead of a second, parallel implementation — then
tags every rule to come out of that recursion with the block's own
condition (only if a *deeper*-nested `@media` hasn't already set one;
real CSS would AND nested conditions together, which this deliberately
doesn't attempt — rare enough in practice, and documented rather than
silently wrong). `parse_stylesheet` just flattens with `rules.extend`.

**Viewport-width-aware matching: one new parameter at the real choke
point, threaded out through everything already calling it.**
`matching_declarations` — already the single place every cascade call
in this file ran real selector matching through (`resolve_style`,
`resolve_background_image`, previously `collect_root_custom_props` had
its own separate, un-shared copy of the same loop, which also now
checks `media_list_matches`) — gained `viewport_width: f32` and skips
a rule outright (before the more expensive real selector match) when
its own condition doesn't currently hold. Every function already in
that cascade's call chain widened by this one parameter: `resolve_
style`, `resolve_background_image`, `build_box_for_element`, `build_
table_children`, `build_table_row_cells`, `build_flex_children`,
`build_children`/`build_children_with_prefix`, `collect_children`, and
`build_layout_tree` itself (now `pub fn build_layout_tree(document:
&Handle, viewport_width: f32)`). Mechanical, not clever — every one of
these functions already threaded `rules`/`inherited` the same way; this
is the identical shape of plumbing, one more value wide.

**No new resize-reaction mechanism — reused the one that already
existed for exactly this reason.** `App::redraw` already rebuilds
`build_layout_tree` from scratch every time the live window width
changes (`self.cached_width != width`), a mechanism that's existed
since before this item for `%`/`em`/`rem`/table/flex/grid measurement's
own sake (see v49/v50/flexbox/grid's own writeups). The only change at
that call site is passing `width as f32` through as the new second
argument — a resize now makes the *rule-matching* pick up a changed
media condition for free, through the exact same rebuild path that was
already making layout numbers change on resize. This was the task's
own explicit instruction (reuse the existing path, don't invent a new
one), and it held up cleanly: zero new state, zero new event handling.

**Testing**: 9 new permanent tests via the full `parse_html` + `<style>`
+ `build_layout_tree(&doc, width)` pipeline (not a lower-level unit
test of `MediaQuery::matches` in isolation) — a `find_box_with_
background` helper walks the box tree for the first resolved
`background-color`, the simplest reliable "did this rule actually
apply" signal. Covers: `min-width` matching at/above (inclusive at the
exact boundary) and not below; `max-width` as the precise inverse;
`and` genuinely requiring both conditions (not silently dropping the
width check when the type alone would already match); comma as real
OR (either branch sufficient, neither matching when neither holds);
`print` never matching even when paired with a `min-width:0px` that
would trivially hold on its own, isolating that it's the type
rejecting it; the Wikipedia-literal `calc()` case never matching at
any width tried, including one a correct evaluation would have
satisfied; an ordinary non-`@media` rule staying completely width-
insensitive (the cheapest possible regression check that this item's
new width-gating in `matching_declarations` didn't leak into the
unconditioned case); a same-document two-different-widths call
standing in for a real resize; and the `prefers-color-scheme: light`
default. All 135 pre-existing tests (plus the two `benches/layout.rs`
call sites) needed only a mechanical second argument added to their
existing `build_layout_tree(&doc)` calls (a wide `1920.0`, changing
nothing for any of them since none declare `@media`-scoped CSS).

**Live verification hit a real sandbox limitation, named rather than
worked around at length**: a real `@media (min-width:600px)`/
`(max-width:400px)` test page rendered correctly red through a live
`scratchy` process and `grim` screenshot, at this sandbox's actual dwl-
assigned window width (~1050px — past the 600px breakpoint, confirming
real end-to-end behavior, not just headless-test behavior). Getting a
genuinely narrow live window to see the inverse visually wasn't
possible here: dwl's tile layout keeps its master column at a fixed
width fraction regardless of client count (tried up to 5 simultaneous
scratchy windows — only the stack column's height shrinks per client,
confirmed by screenshot, not the master's width), and there's no wlr-
randr/swaymsg/wtype-equivalent in this sandbox to request or force a
specific size the WM would honor (same category of gap already noted
for synthesizing mouse events in v32/v55/v62). The headless test suite
above is the real authority for the narrow/boundary/inverse cases,
since it exercises the actual matching code path directly rather than
relying on the WM to cooperate.

## v65 — link navigation: click detection, href resolution, the socket thread, and the live DOM swap

scratchy's own half of the two-part link-navigation plan `@media`'s own
v64 entry ended on. `snag`'s half (`--serve`'s Unix-socket daemon, plus
both of `snag`'s modes emitting a real `<base href>`) already landed
separately as `snag`'s own v9 — this item treats that as a finished,
unmodified dependency (`snag/` isn't touched at all) and reads its
protocol straight off `snag/NOTES.md` rather than re-deriving it.

**Click detection reuses `cursor`'s hit-testing, not a parallel walk.**
`hit_test_cursor` (v32, extended v61) already answers "which glyph is
under this document-space point, and what `CursorStyle` does its span
resolve to" by walking the `Laid` tree down to per-glyph `metadata`
lookups into the shape cache. Rather than write a second recursive walk
for "...and what `href`, if any," the function's return type changed
from `CursorStyle` to `(CursorStyle, Option<&'static str>)` — every
existing early-return/fallback site got a second, usually-`None`,
`href` slot; only the two per-glyph lookup sites (`LaidContent::Text`,
`MixedPositioned::Word`) actually populate it, from a new 8th element
appended to the `ShapedParagraph` tuple (`Vec<Option<&'static str>>`,
parallel to the existing `cursors: Vec<CursorStyle>` vector, populated
by `shape_spans` from each span's own `SpanStyle::href`). `WindowEvent::
MouseInput`'s `Released(Left)` arm calls the exact same function
`CursorMoved`'s hover handler already calls, just reading the `.1` half
instead of `.0`, at a point established by a new, pure, directly
testable `is_click(down, up, threshold)` (5px `CLICK_DRAG_THRESHOLD_PX`)
comparing `Pressed`'s recorded position (`App::mouse_down_pos`, seeded
from `last_mouse_pos`, the same CursorMoved-maintained field `MouseWheel`
already relies on for its own position-less event) against `Released`'s.

**`href` threading is ambient propagation, not CSS inheritance** — same
category as `background-color`/`text-decoration`, not `cursor`/`text-
align`: it isn't a CSS property at all, just an HTML attribute that
needs to reach any descendant text with no more specific override of its
own. `SpanStyle` gained an `href: Option<&'static str>` field (leaked via
a new small `leak_str` helper, same `Box::leak`-to-stay-`Copy` tradeoff
`font-family`'s own named fallback lists already accepted — see that
item's own NOTES.md/TODO.txt entries for the full reasoning), threaded
as a new parameter alongside `decoration` through the same six functions
`text-decoration`'s own v39 block-level fix had to thread through:
`collect_children`, `build_children`/`build_children_with_prefix`,
`build_box_for_element`, `build_table_children`, `build_table_row_
cells`, `build_flex_children`. `collect_children` is the only one that
*computes* a new value (`child_href`, from an `a`/`area` element's own
`href` attribute, falling back to whatever ambient `href` is already in
scope via `.or(href)` so a nested element with no `href` of its own
keeps reporting its ancestor link's) — every other function just passes
the value straight through unchanged, the same shape `decoration`
already established.

**The socket fetch runs on a background thread, the same pattern
`run()`'s `Input::Stdin` case already uses.** `fetch_from_snag_at(path,
url)` (split from `fetch_from_snag`, which always targets the real
`snag_socket_path()`, purely so a test can point it at a mock listener
instead) does the whole round trip: `UnixStream::connect`, `writeln!`
the resolved URL, `read_to_end` until the server closes the connection —
`snag --serve`'s entire "done" signal, no length prefix (see `snag/
NOTES.md`'s v9 entry). Three outcomes become three distinct `Err`
strings (connection refused → "start it with `snag --serve`"; a clean
connection that closes with zero bytes → a fetch-failed message, since
that's `snag`'s own documented behavior for a failed top-level fetch,
not a degenerate success; any other I/O error → its own message) rather
than one generic "something went wrong." `App::start_navigation` spawns
exactly the `thread::spawn` + `event_loop.create_proxy().send_event(..)`
shape `Input::Stdin`'s stdin-reader thread already established — the one
real wrinkle is that `ActiveEventLoop` (unlike the top-level `EventLoop`
`run()` already has) has no `create_proxy` of its own (confirmed by
reading winit's vendored source, not assumed), so the proxy now has to
be created once in `run()` and threaded into `App` at construction time
(`App::new_static`/`new_streaming` both gained a `proxy: EventLoopProxy
<AppEvent>` parameter) instead of only existing inside the `Input::
Stdin` branch.

**The DOM swap reuses `run()`'s own initial-document-building path, not
a new one.** `AppEvent::NavResult(generation, result)`'s `Ok` branch
calls the identical `new_parser().read_from(&mut bytes.as_slice())`
`Input::File` already uses to build the very first `App::new_static`
document — link navigation's bundled bytes go through the same one-shot
HTML parse, not a second parser configuration. The actual swap — `self.
document = ..`, `layout_dirty = true`, clearing `tree`/`cached`/`shape_
cache`/`scroll_offsets`, resetting `scroll_offset`, requesting a redraw
— is factored into a new `App::replace_document`, used for three cases
alike: a successful navigation, the "Loading…" placeholder `start_
navigation` shows immediately (`App::show_message`, HTML-escaped via a
new `escape_html` since the text can be a clicked href or an `io::Error`
message — content scratchy doesn't control), and a failed navigation's
inline error. `base_url` (the page's own `<base href>`, parsed via a new
`find_base_url`/`find_base_href` — the latter the same recursive-search
shape `find_html_element` already established) is deliberately *not*
touched by `replace_document` itself, only by its callers that are
loading a real new page — a transient status message has no `<base>` of
its own, and clobbering `base_url` from it would make the real page's
own links briefly unresolvable while its replacement is loading.
`resolve_href(base: Option<&Url>, href: &str) -> Result<Url, String>`
does the actual join, via the `url` crate's real `Url::join` (a new
scratchy dependency — `snag` already depended on it, scratchy didn't) —
an already-absolute `href` resolves to itself regardless of `base`
(real browser behavior), a relative one joins against it, and a
relative one with no `base` at all reports a clear error rather than
guessing at a basis.

**Reentrancy**: a `u64` `App::nav_generation`, incremented on every
`start_navigation` call and carried on the `AppEvent::NavResult` its
thread eventually sends back, is checked against the *current*
generation on arrival — a result from a navigation the user has since
clicked past (started a second fetch before the first one's finished)
is silently dropped instead of overwriting newer, already-displayed
content. Not part of the originally-scoped feature, but a small, cheap
correctness fix for a real race the background-thread design
introduces on its own, not scope creep into history/back-forward (no
stack is built on top of it — just the one field a future one would
need).

**Testing convention note**: every other feature in this file's history
that touches `App`/the winit event loop at all has stayed entirely
unverified by `cargo test` (window/compositor state is inherently
outside what a headless unit test can observe — see v32/v55/v62's own
entries) — this item follows the same precedent rather than attempting
to construct a real `winit::event_loop::EventLoop` inside `#[cfg(test)]`
(technically possible via `EventLoopBuilderExtWayland::with_any_thread`,
since `cargo test` doesn't run on the main thread, but untried here: no
prior feature in this codebase has needed it, and doing so for the
first time wasn't judged worth the risk of a flaky permanent test over
what pure-function + live-process verification already covers well).
Every piece of this item that *can* be isolated into a plain function —
href resolution, click-vs-drag, the real socket protocol against a mock
listener, `<base>` discovery, HTML-escaping, and href recovery at glyph
precision through the real `hit_test_cursor` — is a permanent headless
test; only the thin `WindowEvent::MouseInput` → `start_navigation` wiring
itself rests on code review plus the live-process checks described in
`TODO.txt`'s own entry for this item.

## v66 — re-platformed link navigation onto `wire`

Scratchy's own half of the `wire` migration `scratchy/TODO.txt`'s "TOP
PRIORITY" entry described — `wire` itself (a separate crate, built in a
prior session: see `wire/NOTES.md`) already landed as a finished,
unmodified dependency; this item is purely scratchy's client side of
that new protocol, replacing v65's `snag --serve` client wholesale.

**Why a whole new transport instead of just pointing the old code at a
different socket**: the old protocol (`fetch_from_snag_at`: write an
absolute URL plus a newline, read until close) was deliberately
GET-only and bundle-shaped — it couldn't express a form's method or
body even in principle, and conflated "fetch a resource" with "bundle a
whole page," which is specifically `snag`'s job, not a general
networking daemon's. `wire` speaks real HTTP/1.1 instead (absolute-form
request line, real headers, a real body when there is one) specifically
so the *same* request-building path can someday cover both a link click
and a form submission — see `scratchy/TODO.txt`'s own "forms and
link-clicks become the exact same code path" decision. This item only
exercises the GET/link half of that; the form half still needs a
`<button>` box and focus/keyboard handling that don't exist yet.

**The response-dispatch design worth calling out**: rather than giving
scratchy a second "the whole document is just an image" rendering mode
for a navigated-to image, a fetched `image/*` response gets wrapped into
a one-element synthetic HTML document (`wrap_image_as_html`) and pushed
through the *exact same* `AppEvent::NavResult` → `new_parser().read_
from(...)` → `replace_document` path every other fetched page already
takes. This means `<img src="data:...">` decoding (`decode_data_uri`/
`decode_image_bytes`, built for v46's `<img>` support) is reused
verbatim with zero new rendering code — confirmed for real, not just
plausible-looking, by a live click-through test that rendered a real
generated PNG fetched over a real socket as a correctly-sized, correctly-
colored block (see `PROMPTS.md`'s own v66 entry for the exact steps).

**`WireFetchOutcome` exists specifically to keep two genuinely different
failure shapes from blurring into one `Result<_, String>`.** "Never
reached `wire` at all" (no socket configured, connection refused, or any
I/O failure before a complete response was read) is the one case
`scratchy/TODO.txt` says should *also* print the raw request to stdout —
there's something useful to hand the user for manual replay. "Reached
`wire`, got a real response back, but it's a content type scratchy can't
render" has nothing new to print — the response already arrived, the
user already knows what they clicked, so it's just an inline error. A
naive single `Result` would make this distinction easy to blur (e.g. a
careless `Err(_) => print_and_show_error(...)` catch-all) — keeping them
as genuinely different enum variants makes that collapse a compile error
instead of a silent behavior bug.

**Live verification went further than any previous session's sandbox
allowed — worth recording precisely, since it closes a gap v65's own
entry explicitly left open.** v65 could not synthesize a real mouse
click at all (no `ydotool`/`wtype`/`wlrctl` in that sandbox) and said so
plainly rather than overclaiming. This session's sandbox turned out to
have `xdotool` available via `apt`, plus `Xvfb` and ImageMagick's
`import` for a screenshot path once `libxkbcommon-x11-0` was installed
(winit's X11 backend needs it, confirmed by reading the actual panic:
`xkbcommon_dl::x11::xkbcommon_x11_handle` failing to `dlopen` it). That
combination made a *real*, full-stack, nothing-faked verification
possible for the first time: a real `wire` daemon, a real loopback HTTP
server standing in for a remote site, a real `scratchy` process, and a
real synthesized `X11` click on a real on-screen link — confirmed via
before/after screenshots that the page genuinely swapped in-process to
freshly-fetched remote content, for both the HTML case and the
image-wrapping case. Not reusable unattended (needs the sandbox to keep
offering these tools, and each piece — `wire`, the fixture server, the
target `scratchy` process — has to be started in a careful, verified
order; an earlier, messier attempt at this exact sequence hit a
transient "wire daemon not reachable" result from restarting processes
out of order across several `run_in_background` calls with their own
independent timeouts, not a real bug), but a genuine high-water mark for
this project's own "verify, don't assume" discipline, worth repeating
deliberately rather than treating as a one-off lucky sandbox.

## v67 — sub-resources for a live-navigated page: external CSS and non-inlined images

The gap this closes: `snag` has always inlined every stylesheet/image a
*bundled* page references, so scratchy's own rendering code never had
to fetch anything itself. v66's live link navigation broke that
assumption without anyone noticing at first — a page reached by
clicking never goes through `snag` at all, so its external references
just sat there, unresolved, with `resolve_img` quietly falling back to
`alt` text and `collect_style_css` never looking at `<link>` in the
first place. Found the same way most real gaps in this project get
found: by actually using the feature and noticing something missing,
not by reading the code first.

## Why CSS and images needed two different mechanisms, not one

The instinct going in was "treat both as 'fetch extra stuff for this
page,' one mechanism." That doesn't survive contact with how this
codebase's rendering pipeline actually works: `build_layout_tree` is a
single synchronous pass with no concept of "re-style once more data
shows up" — every property cascades once, from whatever rules exist at
call time. A stylesheet fetched *after* that first pass would need an
entirely new incremental-restyle capability this codebase doesn't have
and wasn't worth building just for this. An image is different: its
absence only ever affects one box's own intrinsic size (and only when
no explicit CSS/HTML width/height already pins it), so showing the page
without it and then re-laying-out once it arrives is both correct and
cheap — exactly what a real browser does. So: CSS blocks (fetched
synchronously, before the page is ever shown), images don't (fetched in
the background, the page updates incrementally as they land). Worth
recording because the two-mechanism design isn't obvious from the
feature description alone ("fetch a page's sub-resources") — it falls
directly out of a real constraint in how this specific engine lays
things out, not a general web-platform truth applied blindly.

## The `Rc`/`!Send` wall, and why it shaped `process_external_resources`'s shape

`markup5ever_rcdom::Handle` is `Rc<Node>` — not `Send`, by design (no
reason for it to be, since nothing before this item ever needed a DOM
to cross a thread boundary: every background fetch up to v66 only ever
carried raw bytes/strings back to the event-loop thread, which then did
its own, separate, main-thread parse). Finding a page's `<link>`/`<img>`
references, though, really does want a real DOM walk, not a hand-rolled
text scanner (`snag`'s own approach, chosen there for streaming reasons
that don't apply here — scratchy already has the whole response
buffered). The fix: parse a *second*, throwaway DOM, entirely inside the
background thread, walk it there, extract only plain owned `String`s
(and, for images, nothing synchronous at all — see below) and let that
DOM drop before the function returns. `AppEvent::ImageFetched` carries a
plain `DecodedImage` struct, not an `Rc<DecodedImage>`, for the exact
same reason one level down — `Rc::new` only happens once a fetched
image is back on the main thread, inside the event handler, never
before. Getting this backwards (e.g., trying to send the throwaway DOM
itself across the channel, or wrapping a fetched image in `Rc` before
sending it) would have been a compile error, not a runtime bug — Rust's
own `Send` bound caught it before it was ever a real mistake to debug.

## Why `image_cache` is keyed by the raw, unresolved `src` string

The alternative — resolving every `<img src>` to its real absolute URL
before caching, the "obviously more correct"-looking choice — would
have meant threading `base_url` through the exact same 8-function
`build_children`→…→`resolve_img` chain `image_cache` itself already
needed to join (`href`'s own v65 addition already did this once, for a
different reason: ambient href propagation into nested elements with no
href of their own). That's real, avoidable plumbing cost for a safety
property this codebase already gets for free a different way:
`App::replace_document` already clears every piece of per-page state on
every navigation (`scroll_offsets`, `shape_cache`, now `image_cache`
too) specifically *because* nothing from one page should ever leak into
the next — which means the same raw relative `src` string appearing on
two different pages can never collide in `image_cache`, since the first
page's cache is gone by the time the second page's own entries could
exist. Worth stating plainly since it's the kind of shortcut that looks
subtly wrong at a glance ("but what if two pages use the same relative
path for different images?") until the actual lifetime guarantee is
spelled out.

## External CSS `url()` images (Q35a)

`background-image`/`mask-image` `url()`s that are not `data:` are fetched
through the same `spawn_image_fetches` / `AppEvent::ImageFetched` /
`image_cache` path as `<img>`, keyed by the url text. Two pieces make the
key meaningful: `absolutize_css_urls` (net.rs, called at the end of
`expand_css_imports`) rewrites relative `url()`s of every fetched
stylesheet to absolute ones against the sheet's own URL, so the resolution
base survives concatenation; inline `style`/`<style>` urls stay raw and
resolve against the page base like `<img src>`. Discovery is miss-driven:
`resolve_background_image` (now takes `image_cache`) records every
uncached external url in a thread-local (`take_wanted_css_images`), and the
app drains it after each layout build (`fetch_wanted_css_images`), once per
url per page (`App::css_images_requested`, cleared with `image_cache`),
capped at `MAX_CSS_IMAGE_FETCHES` (64). A late arrival sets `layout_dirty`
like `<img>` (the image is baked into the `LayoutBox`).

## Live verification: proving the async/sync split, not just asserting it

The easy trap here would have been: click a link, take one screenshot
once everything's visibly settled, call it done — which on a fast
loopback fixture (sub-millisecond fetches) would have looked identical
whether images were genuinely async or secretly blocking the page. The
session caught this by deliberately slowing one fixture route down
(`time.sleep(1.5)` on the image response only) and taking a screenshot
*mid-flight* — 0.5s after the click, well before the artificially-
delayed image could have arrived. That screenshot is the actual
evidence the design works as intended: the page had already fully
navigated and the external stylesheet had already applied, with the
image slot simply empty, not the whole page frozen waiting on it. A
second screenshot after the delay showed the image appear on its own,
with zero further interaction — confirming the full `AppEvent::
ImageFetched` → cache-insert → `layout_dirty` → redraw loop actually
fires correctly from a real background thread into a real running event
loop, not just in a headless unit test calling the same functions
directly. Worth establishing as a pattern for any future async-UI
feature in this codebase: when a design's whole point is "doesn't block
on X," prove it by making X slow on purpose and watching for the block,
not by watching a fast path that could hide one either way.

## v68 — a wire-navigated page's own missing `<base>` tag

Found the same way most real gaps in this project surface: by actually
using the thing and hitting it, not by auditing the code first. Worth
recording precisely *why* v66/v67 missed this, since it's a useful
pattern to watch for again: both of those items were tested against
fixtures built for this project, which (consciously or not) always
either declared their own `<base>` or only ever used absolute hrefs —
exactly the two cases that were already correct. A page with neither —
no `<base>`, but a real relative href — is the actual majority case on
the real web, and nothing in either prior session's test suite or live
verification exercised it. `snag`'s own v9 already solved the same
problem for bundled pages (synthesize a `<base href>` from the real
fetched URL whenever the page doesn't declare one) specifically because
someone hit the identical gap there first — the fix here is the same
idea, just computed by scratchy itself instead of pre-baked by `snag`
into the bundle, since a `wire`-navigated page never passes through
`snag` at all. Worth remembering next time a similar "parity with a
`snag`-bundled page" gap gets fixed for the `wire` path: check `snag`'s
own `NOTES.md`/`PROMPTS.md` first for whether it already solved the
exact same class of problem once.

## v69 — why `cycle_command_completion` needs no separate "how much to
remove" bookkeeping

The one real subtlety in the Tab-completion work: cycling has to
replace a *different* amount of text on the first `Tab` (whatever
prefix the user actually typed) than on every `Tab` after it (the
previous candidate's own, usually-longer text), and the obvious-looking
fix — store the replaced range's end alongside `token_start` in
`CommandCompletion` — turns out to be unnecessary. `self.command_line.
cursor` already *is* that end, on both calls, for the same reason
either way: `apply_completion`'s own last line always leaves it at
`token_start + candidate.len()` right after inserting a candidate, so
the next call (whether that's "the first real completion" reading the
cursor the user's own typing left it at, or "cycle to the next match"
reading the cursor the *previous* completion left it at) can use
`token_start..cursor` as the replace range unconditionally. Spotted only
after writing a first draft that threaded a separate `replace_end`
computation through `CommandCompletion` — worth the few minutes it took
to notice `cursor` already carried the information, since the simpler
version is also the version with nothing to keep in sync (a hand-
computed `replace_end` that silently drifted from the real cursor would
have been a live bug no test here is shaped to catch, since every test
exercises `token_start_before`/`command_completions` in isolation, not
`cycle_command_completion`'s own stateful replace step — that one's
covered by code review and the live `xdotool` cycling screenshots in
`PROMPTS.md`'s v69 entry instead, not a unit test).

## v69 — `arboard` reads the `CLIPBOARD` selection, not `PRIMARY`

Worth stating plainly since X11 has two selections and it's easy to
test the wrong one by hand: `Ctrl`+`v` paste, same as every other
`Ctrl`+`v` on Linux, reads `CLIPBOARD` (an explicit copy — `Ctrl`+`c`,
or here, `xclip -selection clipboard`), not `PRIMARY` (whatever's
currently text-selected, pasted with a middle click). Live-verifying
this live caught nothing wrong — `arboard::Clipboard::get_text()`
already defaults to `CLIPBOARD` — but it's the kind of thing that looks
right from reading the code alone and is worth having actually run
once, with `xclip -selection clipboard` specifically (not a bare
`xclip`, which would have set `PRIMARY` and proven nothing about the
selection `paste_into_command_line` actually reads).

## Q13 — custom properties cascade per element

Replaces the v45 design (one `:root` table, whole-stylesheet textual
substitution before cascading). Declarations containing `var(` are kept
raw in `Declarations::var_decls`; `resolve_style` builds the element's
custom-property table (`compute_custom_props`: inherited from the parent
via `InheritedStyle::custom_props`, overlaid with its own `--x`), then
re-parses the deferred declarations with `var()` substituted and
inserts them next to their rule's entries in the match list, so the
existing `cascade_property` machinery needs no change. The table is an
interned, leaked `&'static BTreeMap` so `ComputedStyle` stays `Copy`;
elements that declare no custom properties just copy the parent's
pointer. Cost: each element matching a rule with `var()` re-parses those
declarations (resolve_style runs several times per element), which is
fine at current page sizes but the first thing to cache if profiling
points here.

QY53i: `compute_custom_props` is memoised (`CUSTOM_PROPS_MEMO`, thread
local) on (parent table address, the element's own winning `--x` raw
pairs sorted by name). Because interned tables are leaked, an address is
never reused, so the pair fully determines the result and a hit returns
the interned pointer without cloning the parent table, re-resolving
`var()` or SipHashing the whole combined table (YouTube watch tables are
~125 KB each; 98 % of calls are hits). The leak itself
is unchanged: freeing tables on navigation needs `ComputedStyle` /
`InheritedStyle` to stop being `Copy` (an `Rc`/handle instead of
`&'static`), which is a larger change than QY53i; the memo, like the
interner, lives as long as the thread.

## Q14 — viewport units and the initial containing block

Two separate roles used to share the root `Laid`'s `h`: the basis for
`%` heights, and the scrollable content height `redraw` clamps to. They
are now split. `ComputedStyle::icb_height` (set only on the synthetic
root box, from the window height) feeds `child_containing_height` and
nothing else, so the root's own height stays content-driven.
`measure_document` additionally raises the root's `h` to `in_flow_bottom`
(lowest in-flow descendant edge, not descending into overflow-clipping
boxes), because `html{height:100%}` ends at the window bottom while its
content overflows. `vw`/`vh`/`vmin`/`vmax` resolve from a thread-local
(`set_viewport`) written at the start of every style build; layout-time
`BoxLength::Calc` carries a copy since it resolves on the worker thread.
Height changes now trigger a rebuild (`App::cached_height`).

Q15a: grid content-sized tracks. `GridTrackSpec`/`TrackBreadth` gained
`Auto`/`MinContent`/`MaxContent`. `measure_grid` now places items before
sizing columns (placement only needs the track count), gathers
`cell_natural_width` (min, max) per content-sized column from its
single-column items, and passes that to `resolve_grid_tracks`. Order in
the resolver: fixed tracks, content tracks start at base and grow toward
their limit, `fr` tracks share what is left, and with no `fr` track the
`auto` tracks stretch equally. Single-pass, no spec iteration.

Q15c: numeric grid placement. New `GridLine` (`Auto`/`Line(i32)`/`Span(n)`),
four item longhands on `Declarations`/`ComputedStyle`
(`grid_row_start` etc.), set by `grid-row`/`grid-column`, the longhands and
the 4-value `grid-area` (a lone ident still means a named area). In
`measure_grid`, `resolve_grid_axis` turns an axis' lines into a fixed range
or an auto span; placement runs in three passes (named/fully fixed, one fixed
axis, cursor auto-placement with spans via a `free` cell check). A row span
still only contributes height to its first row. Named lines and implicit
columns beyond the template are not supported (clamped).

**Q17 `opacity`.** Implemented the "multiply alpha down the tree" option: `resolve_style` accumulates `ComputedStyle::opacity` (own times parent's, via `InheritedStyle::parent`), and `paint` sets a thread-local `PAINT_OPACITY` around `paint_inner` so `blend` scales every alpha. This avoided threading a parameter through the 6 box-building functions that the older `opacity` entry above feared. Double-blending of overlapping faded children is accepted; inline-span `opacity` is not applied (no per-span style slot).

**Q19 generated content.** Pseudo-element rules are matched by `matching_declarations_for` (a `pseudo: Option<&str>` parameter; `matching_declarations` is the `None` wrapper) using `matches_selector` per selector in `MatchingMode::ForStatelessPseudoElement`, because `matches_selector_list` in `Normal` mode always rejects selectors with a pseudo-element (Q3 relies on that). `resolve_style_for` is the pseudo-aware `resolve_style`. Generation happens only in `collect_children` (which now brackets `collect_real_children`), so it covers block, inline, list-item and flex/grid containers' inline runs, but not `build_flex_children` items or table internals. Cost: two extra rule scans per element; each is a cheap `pseudo_element()` check per selector, and it exits early when nothing matches.

**Q19b dynamic content.** `attr()`, counters and quotes depend on document order, which the recursive box builders do not follow uniformly (inline, block, flex, table paths), so `precompute_generated` does a separate pre-pass in `build_layout_tree_with_resources` that evaluates them into the thread-local `GENERATED_TEXT` keyed by (element pointer, is-after); `generated_content` only looks the string up. The pass is skipped entirely unless a rule uses such content or a counter property. `build_flex_children` emits `::before`/`::after` as blockified items.


**Q23c flex leftovers (update to the flex notes above).** `measure_flex_column`
now wraps: with `flex-wrap` set and a definite container height it breaks
items into side-by-side columns (greedy by hypothetical height), sizes each
column's cross extent from the widest margin box, and distributes the free
width with `distribute_align_content` (shared with `measure_flex_row`).
Wrapping items are measured once at fit-content width and stretched post hoc
(`Laid::w` overwrite, no re-measure, so no duplicate `pending` entries).
Column items get a content-based automatic minimum height (their measured
height, `overflow: visible` and no `min-height` only). `align-items: baseline`
in rows uses `first_baseline` (first shaped line's `line_y` found through the
shape cache, falling back to the border-box bottom) and the line cross size
accounts for baseline items' above/below extents.

## `mask-image` (Q29)

A mask rides in `DecodedImage::mask` on the same per-box `background_image`
bundle (a 0x0 placeholder when there is no background image), because
`ComputedStyle` is `Copy`. `paint` diverts masked boxes to `paint_masked`,
which paints the box normally and then lerps the border-box region back to a
pre-paint snapshot by the mask coverage (descendants are masked too). SVG
masks are re-rasterized at the tile size on every paint, uncached.

## Interaction state: `:hover` / `:active` / `:focus*` (Q31)

The `LayoutBox`/`Laid` tree holds no DOM handles, so element identity is
`node_id` (the `Rc<Node>` address). `ComputedStyle::node` (0 for anonymous
boxes) and `SpanStyle::node` (stored per glyph span next to the href in
`ShapedParagraph`) carry it; `hit_test_node` (wrapped by `hit_test_cursor`
and `hit_test_element`) returns it, falling back to the enclosing box's id.

State lives in `InteractionState` (hover / active / focus_within already
expanded to ancestor-inclusive id sets, plus `focus` and `focus_visible`).
`App` owns it and mirrors it into the `INTERACTION` thread-local, which
`match_non_ts_pseudo_class` reads (style matching is synchronous on the main
thread and has no other channel). `App::set_interaction_state` is the one
place state changes: it restyles (`layout_dirty` plus
`keep_scroll_on_rebuild`, so inner scroll offsets survive) only if a kind
that changed is mentioned by a stylesheet. The parser records that in the
`STATE_RULES` bitmask (`STATE_HOVER|ACTIVE|FOCUS`, reset at the start of each
`build_layout_tree_with_resources`, read into `App::state_rules`). A page with
no `:hover` rules pays nothing, and a hover that moves between elements whose
ancestor chain is identical changes nothing.

Events: `CursorMoved` -> `update_hover`; mouse press -> `press_state` (active
chain, and focus on the nearest focusable ancestor, not focus-visible);
release clears active; Tab/Shift+Tab -> `tab_focus` (focus-visible); Enter
on a focused link navigates.

Reusing it for forms (Q30): make the control focusable in `is_focusable`
(it covers links and `tabindex >= 0` only today, and `focusable_in_tab_order`
follows it), call `App::set_focus_element(Some(&handle), visible)` from a text
input's own click/Tab handling, and `:focus`/`:focus-within` work with no
matcher changes. A button needs nothing beyond `is_focusable`: hover/active
already apply to any element. `:disabled`/`:checked` stay in the matcher.

Measured (release, synthetic 5000-item list): a restyle rebuild is ~113 ms
(DOM walk + cascade; shaping runs on the worker afterwards); finding the
hovered handle from its id and building its ancestor chain is ~0.15 ms.
Not done: focus restoration across navigation.

Hover (QINC1m): a hover change whose chain no `:hover` rule can read (R2.4) needs no frame; any
other is an ordinary incremental engine frame (`incr_frame::frame`). The old pipeline's hover
shortcuts (QY53n1 match-memo diff, QY53n2/n3 `hover_patch` paint/offset patch, R47) were removed
with it (QINC1m); see git history before QINC1m for their design.

Hover paint (QHOV1): the engine's hover frame is ~1 ms (restyle of the few changed elements, one bottom-up
splice); the cost left was repainting the whole window. Each splice in `incr_layout::attempt` records the
document rect it changed (old ink and the new subtree's rect union, each padded by `paint_reach`: shadow,
outline, 4 px) plus the rects of its z-index entries; a top-down relayout, a failed update, a fixed/sticky/
scroll-region/masked box in the path or subtree, a mask, or more than 32 rects leaves the damage unknown
(`None`). `take_damage` is called once per paint of the engine's own tree (`owns`), so the next paint
starts from this one. `App::redraw` (page has `:hover/:active/:focus` rules, no video element/search/select
popup) keeps the page it painted (`DamagePaint::kept`, before the status bar) with a `PaintKey` (size,
scroll, region offsets, focus, hot scrollbar, bars, canvas colour); a frame with damage and the same key
repaints only those rects into `scratch` (`repaint_rect`: white + canvas colour, `paint` with
`PAINT_CULL` skipping boxes whose ink + a margin (max of 64 px and the tree's largest `box_reach` + 8, for a
neighbour's shadow / text-shadow / drop-shadow) misses the rect),
copies them into the kept page and the surface, repaints the status bar and `present_with_damage`s. More
than 8 rects or half the viewport: full paint. QCSS-D effects (`incr_layout::box_reach`): outline, box-shadow,
filter blur / drop-shadow (`fx::filter_reach`) and text-shadow widen the damage rects and the cull margin;
a LAYER box (`filter`, `backdrop-filter`, non-normal `mix-blend-mode`: its output depends on what is around
or behind it, and a clipped repaint would blur / blend against unpainted scratch) that a damage rect reaches
(ink + its reach) makes the whole frame a full paint (`app::layer_hit`; the engine reports `layer_fx` /
`reach` with the damage). A new way to paint past the border box must widen `box_reach`, or add its case to
qhov1_tests. `SCRATCHY_NO_DAMAGE=1` disables it. Measured (hover-big.html, 960x1006
nested window, 5000 elements): paint+present 3.0 -> 1.7 ms per redraw, 108 of 147 redraws damage-presented.

Damage repaint for any page (QHOV2): the gate `state_rules != 0 || canvas` is gone. Any frame of the engine's own
tree with known damage and an equal `PaintKey` patches the buffer: JS class toggles, small DOM writes, animation
frames. No kept copy of the page any more (it was a 4 MB copy per full paint): `DamagePaint::hist` holds the
`Damage` of the last `DAMAGE_HISTORY` (4) frames since the chain started, and a swap buffer of age `a`
(`pixels.age()`; 0 = unknown contents) is missing exactly the newest `a - 1` of them, so `damage_plan` merges those
rects (deduplicated) with this frame's and `redraw` repaints the union into the buffer (the present damage stays
this frame's rects: that is what changed on screen). A frame with known damage and the chain's key extends the
chain even when it was painted whole (too big, buffer too old), so a double-buffered window is patching from
the third frame; unknown damage, another key or an overlay (video, search highlight, select popup) restarts it
(`hist` empty: only this frame's buffer is current; the other buffers are older than the chain). Layer boxes /
fixed / sticky / scroll regions are unchanged: their damage is unknown (`None`), or `layer_hit` on the merged
rects, and the frame is a full paint. `SCRATCHY_DAMAGE_VERIFY=1` paints every patched frame whole as well and
prints `damage-verify: ... differs` (and counts `app::DAMAGE_VERIFY_BAD`) when the two differ.

JS DOM writes (QB2): `App::js_sync` is a restyle request like a hover
change (`layout_dirty` + `restyle_requested` + `keep_scroll_on_rebuild`).
`layout_in_flight` is true from the moment `redraw` queues a shaping pass
until its complete `ShapeResult`; writes in that window only set
`js_restyle_pending`, and the next rebuild starts when the pass completes.
Without it a timer faster than a full pass made every pass stale.

Image arrival (Q35b2): `handle_image_fetched` caches the image at once but
no longer rebuilds per arrival. It records `image_relayout_since` and arms
an `AppEvent::ImageFlush` timer for `IMAGE_RELAYOUT_DEBOUNCE` (80 ms, from
the first arrival, not extended). `flush_image_relayout` (timer, and the
complete `ShapeResult` in `handle_shape_result`) sets `layout_dirty` only
when the window has passed and no pass is in flight (`layout_in_flight` /
`hold_old_frame`), so a burst is one rebuild and no pass is made stale.
An arrival while a rebuild is already queued (first layout) adds nothing.
`replace_document` clears the pending state; stale-generation images are
still dropped first (R7). Pure decision: `image_relayout_wait`.

Main-thread stall limits (QB3, R8): the compositor drops a client that
doesn't read its socket while input piles up. `hover_idle_wait` (layout.rs)
makes a hover restyle wait for 150 ms of pointer stillness when the last
cascade build took >50 ms. `CALLBACK_BUDGET` (js.rs, 500 ms) bounds each
timer/event callback; only the page-load script run keeps `SCRIPT_BUDGET`
(5 s). The cascade itself still runs synchronously in `App::redraw`.
Both budgets are thread CPU time (QY5, `thread_cpu_nanos`), not wall
clock, so load or waiting doesn't kill scripts; a stop is logged as
`js: <script> stopped: exceeded the N CPU-time ... budget`.

### Fragments, scroll-into-view, focus ring (Q31b)

`InteractionState::target` (node id) backs `:target` (`STATE_TARGET` bit).
`fetch_url` short-circuits a URL whose fragment-stripped form equals
`base_url`: `App::apply_fragment` finds the element (`find_fragment_target`:
`id`, then `<a name>`; raw then percent-decoded), sets `target`, and scrolls
its top to the viewport top. A cross-page navigation keeps the fragment in
`nav_fragment` (never sent in the request; `fetch_url_inner`), promoted to
`pending_fragment` by `handle_nav_result` (not earlier: the "Loading…" page
lays out first and would consume it, QA6/R18), and applied in the
`ShapeResult` handler once a layout containing the target exists. A target
the document does not have is waited for while layouts are incomplete and
dropped on the first complete one.
`node_geometry(laid, cache, node)` is the shared "where is this element"
query: its own box, else one rect per line of its glyph runs (spans carry the
node id), plus whether an author `outline` is set. Tab uses it in
`scroll_focus_into_view` (minimum scroll, 24px margin); `redraw` uses it to
paint `paint_focus_ring` (2px blue with a white halo) for `:focus-visible`
focus unless the element already has an outline. Limits: ignores
`overflow: scroll` containers and sticky shifts.

### JS engine spike (Q33-0)

Measured, nothing shipped (no JS dependency in the scratchy crate). Throwaway
crates ran real libraries (moment, d3, lodash, three.js: 1.67MB) in `boa_engine`
0.22 and `rquickjs` 0.14 with stub globals. parse+exec: boa ~1.05s, QuickJS
~0.15s. CPU-bound loop: 1.1s vs 0.47s. RSS after ~1.9MB of JS: 126MB vs 55MB.
Engine-alone clean build: boa 3m43 wall / 21m CPU, QuickJS 50s (gcc only).
Stripped binary of a stub program: boa 13.0MB, QuickJS 1.66MB; scratchy itself
is 13.5MB (11.1MB stripped) and builds clean in 5m11 / 37m CPU. Recommendation:
rquickjs, pending the user's confirmation. Full table in TODO.txt, Detail
"JavaScript - engine choice". Caveat: the machine was loaded by other builds,
so wall times are noisy and jquery/react-dom were not usable without a DOM.

### Engine decision and script running (Q33a)

Decision (user, 2026-11-15, "Go with rquickjs"): the engine is `rquickjs`
0.14.0 (QuickJS, bundled C compiled by `cc`, no bindgen), pinned exactly in
Cargo.toml, always built in (no cargo feature; `--no-js`/`:set nojs` is the
runtime switch). boa is not used.

`src/js.rs`. `find_external_resources` also collects classic `<script>`s
(`ScriptRef::Inline`/`External`, document order; module/json/template types
skipped). `process_external_resources` now returns `PageResources { css,
scripts }`: external scripts are fetched concurrently through wire
(`fetch_script_via_wire`, same path as stylesheets) on the fetch thread and
joined in order; `FetchedPage` and `AppEvent::StreamCss` carry the
`Vec<ScriptSource>`. Scripts run on the main thread right after the document
is installed (`run_page_scripts`): fresh Runtime/Context per page, a JS
prelude (console -> stderr via native `__scratchy_log`, `window`/`document`
stubs holding only `DOMContentLoaded`/`load` listeners and `window.onload`),
each script evaluated separately (errors logged, never fatal), promise jobs
drained, then DOMContentLoaded and load. Guards: 5s total interrupt budget,
256MB memory limit. A page with no scripts never creates a runtime. File
pages without `--socket` run inline scripts only.

JS flag: process-wide `AtomicBool` (`js_enabled`). With JS off the HTML
parser runs with `scripting_enabled=false` so `<noscript>` children are real
elements, and the UA rule `noscript{display:none}` is only added when JS is
on (not in default.css; the rules cache key notes the flag). `:set nojs`
applies from the next page load (`:refresh`).

DOM read API (Q33b), `src/js_dom.rs`: `run_scripts_in(document, ..)` installs
native `__d_*` functions over a Rust table of `Handle`s (ids are indices,
document = 0) plus a JS prelude of cached per-node wrappers. Selectors use the
cascade's `OurSelectorParser`/`matches_selector_list`. Element listeners are
stored but never fired yet. Writes (Q33c): natives mutate the `Handle` tree
directly (own detach/insert helpers) and set `JsEngine.dirty`; `App::js_sync`
turns that into one `layout_dirty` per event-loop turn (after page load and
after timers). `style.<prop>` is a JS Proxy over the `style` attribute.

Timers (Q33t): `js::start_scripts` returns a `JsEngine` (runtime + context +
log + clock base) that `App.js` keeps for the page's lifetime; it is replaced
when a page's scripts run and cleared in `fetch_url_inner`, so stale timers die
with their page. `setTimeout/setInterval/clear*/requestAnimationFrame` are JS
over a timer table (`TIMERS_PRELUDE`); Rust only calls `__scratchy_timers_next`
and `__scratchy_timers_run_one`. `App::about_to_wait` runs due timers (max 100
per turn, each with a fresh 5s interrupt budget, promise jobs drained after
each) and sets `ControlFlow::WaitUntil(next)` or `Wait` (no wake-ups without
timers). Intervals and >5-nested timeouts clamp to 4ms. rAF is not a timer since QEL1
(see "Rendering steps"); it is not gated on window visibility yet (E14).

Events (Q33d): one `EventTarget` implementation in `PRELUDE` (WeakMap listener store keyed by the target object, shared by window/document/wrappers/bare targets) with capture/target/bubble `dispatch`; `on<type>` properties run after listeners. Rust fires trusted events with `JsEngine::dispatch_event` (-> `__scratchy_dn`, `None` target = body); `App::js_dispatch` also runs `js_sync`. Hooked: click (release and Enter on a focused link; `preventDefault` cancels navigation), keydown/keyup (before vim bindings, not in command mode), focus/blur/focusin/focusout in `set_focus_element`. DOMContentLoaded/load use the same dispatcher. input/change/submit natives wait for Q30 (Q33d1). QA12: inline `on<type>="..."` attributes are compiled lazily in `dispatch` (`attrHandler`: `new Function('event', src)`, cached per element+type in a WeakMap, recompiled when the attribute text changes) whenever the `on<type>` property is not a function; `<body>`/`<frameset>` window-forwarded types (`load`, `resize`, ...) are read from `<body>` when the event reaches the window. No `with` scope chain (only `this`, `event` and globals); `onerror`'s window signature is not special-cased.

Focus and scroll from JS (QB5): `JsEngine.reqs` (`JsRequests`) holds at most one pending focus/blur and one scroll request; natives `__d_focus`/`__d_scroll` only record, `App::js_sync` drains them (`set_focus_element` + `scroll_focus_into_view`, `scroll_offset`). The engine never touches App state directly.

Dialog (QB4): `wrap` gives `<dialog>` wrappers `HTMLDialogElement.prototype` (open/returnValue/show/showModal/close, all attribute-backed; `close` event fired synchronously). `showModal` also sets the internal `data-scratchy-modal` attribute because `:modal` isn't supported; `default.css` keys the fixed, viewport-centred rule off it. `resolve_positioned` centres a box with both insets set and an `auto` margin (horizontal and, via `ComputedStyle::margin_auto_v`, vertical). `hit_test_inner` no longer prunes on `ink` for a box holding `fixed` children (they are excluded from ink by design).

JS and `:help` (QB6): `SavedPage.js` parks the page's `JsEngine` while help is shown (`show_help` takes `self.js`, `restore_saved_page` puts it back and calls `js_sync`). Help and `show_message` pages run no JS (`self.js = None`), so timers, key/click listeners and the dirty flag cannot touch them. Timer deadlines are absolute, so overdue ones fire on return. Back/forward and `:refresh` refetch, so `fetch_url_inner` simply drops the engine; `navigate`-style loads drop `pre_help_page` (and its engine) too.

`document.currentScript` (QB8): `ScriptSource.index` is the script's position among runnable `<script>` elements; `start_scripts` resolves it to a DOM handle and brackets each script with `__scratchy_cs(id)` / `__scratchy_cs(-1)`. Scripts without an element (tests, index `None`) see null. `JsEngine::eval_value(label, code)` names its eval (`event:click`, `timer`) so stack traces say which dispatch ran.

## QY1 — stacking contexts (paint order)

`measure_document` ends with `hoist_negative_z`, which flattens each stacking
context: every positioned descendant (abs/fixed, and relative/sticky with or
without `z-index`) is moved into the `positioned` list of the nearest owner
(the root, `fixed`, positioned with `z-index`, sticky, `opacity<1`, or a
clipping `overflow` box; QYSCR1: `<html>`/`<body>` overflow is already `visible` in the layout tree: `images::propagate_viewport_overflow` gives it to the viewport, the page's top-level scroll, CSS Overflow 3). Lists are
sorted by `layout::stack_key` = (z-index, `ComputedStyle.tree_order`), where
`tree_order` is a counter bumped per `resolve_style` (document order).
`paint_z_layer` and `hit_test_inner` use the same key, so hit testing equals
paint order. The old `is_explicit_stacking_child` pull-out in `paint` is now
mostly a no-op (those children have already been hoisted).

## QA10 — anonymous table objects

Box building (images.rs), not layout, generates the missing table parts
(CSS 2.1 17.2.1). `build_table_nodes` walks a node list: consecutive
`table-cell` nodes become one anonymous `TableRow` box; a block/flex/grid/
table node arriving while such a run is open becomes an anonymous cell in the
same row (a `table-caption` or a block before the first cell stays a stacked
child, as before). `build_table_row_cells` wraps runs of non-cell content in
an anonymous cell (whitespace-only runs are dropped). In `collect_real_nodes`
a `table-row|cell|row-group` element outside a table collects its following
sibling table parts (plus whitespace/script/`display:none`) into a run and
emits one anonymous `Display::Table` box around `build_table_nodes` of it.
Real `<table>` markup takes the same paths unchanged. `paint_buffer` also
adds the fractional part of the run's x origin to each glyph position.

`document.write` (QA13): `__scratchy_cs` also resets a per-script insertion point; `document.write` parses its argument as a fragment in the context of the current script's parent (`parse_fragment_in`, shared with innerHTML) and splices the nodes after the script (then after the previous write's last node). Not stream-parsed: split tags across writes are not joined, written `<script>`s do not execute, after load it is a warned no-op.

getComputedStyle (QA16), `src/js_style.rs`: the engine cannot see `App`, so `__d_cs(id, pseudo, prop)` runs the cascade itself for one element (`StyleCache::style_of`: `resolve_style_for` from the outermost ancestor down, rules parsed from UA + `<style>` + the thread-local external CSS set by `set_js_extra_css`). Cache: per (element, pseudo) until the DOM write generation (`wgen`, bumped by each mutating native) changes; rules re-parsed only if their source text changed. `RULE_INDEX` is swapped in for the call and the layout build's is restored. No layout: width/height are computed values (`auto` when unset), not used px (QA16a).

Geometry reads (QJ4), `src/js_geom.rs`: `__d_rect(id, vw, vh)` returns an element's laid-out border box in document space. A read with a stale key (DOM write generation, viewport, external CSS length) runs a FORCED layout on the JS thread (`build_layout_tree_with_resources` + `measure_document`, thread-local `FontSystem`, own `ShapeCache`; `catch_unwind` so a layout panic gives zero rects instead of killing the browser), records every `Laid` box by `style.node`, and caches until the next DOM write. The layout's CPU time is excluded from the script budget (`js::pause_budget_since`, `script_clock_nanos`). The JS side (end of DOM_PRELUDE) builds `getBoundingClientRect` (document box minus `window.scrollY`; fixed subtrees are not scrolled), `getClientRects`, `offset*`, `client*`, `scroll*`; display:none/detached read as zeros. `JsEngine::set_scroll` mirrors the App's real scroll offset into `window.scrollY` before events and timers. Counts forced layouts and logs once at 20 or 1 s total. Limits: no decoded image sizes, inline elements use their descendants' union or nearest box ancestor, element `scrollTop/Left` only remembered (QJ6).

QINC1m: the old forced-layout path's skips (QYTC1a in-place `style` shifts, QYTC1d inert `class` probes,
QY53l2 `HiddenSnap`, QY53o1 unread-attribute writes) and the App-layout IntersectionObserver source (QY53e2,
`rendered_geometry`) are gone: the engine bypassed them (its invalidation sets decide what a write restyles), and
`GeomCache::refresh` from scratch is now only the engine's fallback (no tree / panic).

Multiple documents (QA18a): the page is DomTable id 0; `document.implementation.createHTMLDocument/createDocument` allocate another `NodeData::Document` root through `__d_newdoc` and register its id in `DomTable.docs` (any other Document-data node is a fragment). All nodes of all documents live in the one table and share wrappers/prototypes. The JS Document wrapper for a second document is `Object.create(document)`: every method/getter reads `this.__id`, so `getElementById`, `documentElement`, `head`, `body`, queries and child access just work per document. Ownership of detached nodes is a JS map (`owners`, set by the document's creators, `importNode` and `adoptNode`); an attached node's owner is its root document. The cascade (`__d_cs`) is still the page's only.

iframe documents (QA18b): `HTMLIFrameElement.contentDocument/contentWindow` make, on first access, a `makeDoc` document with html/head/body and a minimal window object (frames table keyed by the iframe's node id, in the prelude). Only for an iframe that is in the page and whose `src` is empty, `about:blank` or `data:` (the data content is ignored); anything else, including http(s), gives null until QA18b1. `load` is queued by a hook (`afterIns`) in `appendChild`/`insertBefore`, so iframes inserted by innerHTML or document.write get no event. Nothing is laid out or painted inside the iframe (still the Q52a replaced box), and the frame has no cascade (QA18c).

iframe src documents (QA18b1): `net::prefetch_iframes` runs where scripts are fetched (so on the fetch thread, before the JS engine starts) and keeps the HTML of same-origin http(s)/file iframes present in the markup in a process-wide map keyed by absolute URL; the JS engine cannot reach the network, so `frameOf` only looks the URL up (`__d_iframe_page`) and parses it into the frame's document (`__d_load_html`). Iframes made later by script, and streamed pages, get null. Scripts inside frames are not run. QA18c: `__d_cs` takes the cascade root from the node's top ancestor (page or a registered second document); `StyleCache` reseeds when the root changes, and the externally fetched CSS (`EXTRA_CSS`) is only applied to the page.

iframe realms (QPT1a, design R53): an iframe with an empty/`about:blank` src, or a pre-fetched one (QA18b1), gets a real window: `frameOf` calls the native `__scratchy_realm_new()` (js_engine_v8.rs), which deserialises the startup snapshot's page context 0 a second time in the page isolate (~1.3 ms, ~0.65 MB), gives it the caller's security token and its own `document.all`, and returns its global proxy. The parent then calls the baked hook `w.__scratchy_realm_init(o)` (PRELUDE in js.rs): about:blank env with the PARENT's origin, `__d_bind_doc(id)` rebinds the realm's `document` (baked as id 0) to the iframe's second document, `parent`/`top`/`frameElement`/`name`, and the host queues forward to the parent (Rust pumps only the page context): the timer table (`__scratchy_timers_fwd`; the callback still runs in its realm with its window as `this`), network requests (`__net_fwd`: the parent's request table, the realm's own `Response`), `WebSocket` (the parent's class), unhandled rejections (the page's flush calls the realm's, `__scratchy_rej_sub`). Navigation/history/scroll natives are no-ops that log in a realm. Natives called from a realm run in the realm's context and share the page's closures and `DomTable`. Realm lifetime (QPT1b, R53 D6): a frame's realm is dropped when its iframe leaves the document (the prelude wraps `__d_remove`/`__d_sethtml`/`__d_settext` and sweeps `frames`), when its `src`/`srcdoc` is set or removed, or with the page (the engine drop). `dropFrame` calls the realm's `__scratchy_realm_release` (nested frames, timers by `recv` via `__scratchy_timers_cancel`, pending requests by owner via `__net_cancel`, a last rejection flush, `__scratchy_rej_unsub`), then the native `__scratchy_realm_drop(w)` removes the `State.realms` entry (QPT1d: no `Context::DetachGlobal` any more, it made every read of a stale `w` throw `no access`, so `w.closed` could not be true; the context now lives while JS references the window, as in Chrome. `contentWindow` of a removed iframe is null, a re-inserted one gets a new realm). Cap `MAX_REALMS` = 64 live realms per page: the extra frame gets the stand-in (an older frame's window is never pulled from under its scripts), logged once. WebSockets a realm opened stay with the parent's class until closed. Wrapper identity (QPT1c, R53 D7): one wrapper per node across all realms of a page. The DOM prelude's `cache` (id -> wrapper), `owners` and the realm table `xr` (`byDoc`: document id -> that realm's wrapper maker `mk`; `n`: live iframe realms) are page-wide: the parent passes its own in `realm_init` (`o.dom`) and `__d_bind_doc` switches the realm to them. On a cache miss with `xr.n > 0`, `wrap` asks the native `__d_root(id)` for the tree root (a document, or a detached root whose document is `owners[root]`) and lets the realm that owns that document make the wrapper, with its prototypes (`f.contentDocument.body instanceof w.HTMLElement`, not the page's `HTMLElement`, also through a page getter). A node moved to another document keeps its wrapper and so its creation realm's prototypes (Blink does the same; Firefox re-parents the prototype). While realms exist, a removed node records the document it left in `owners` (`ownerDocument` across realms). A dropped realm (`__d_realm_purge` in `__scratchy_realm_release`) deletes the wrappers it made that are still in its documents (or detached), its documents and the `owners` entries naming them, so nothing keeps its context alive; a node moved into the page keeps its wrapper. MutationObserver registries stay per realm: an observer sees mutations made through its own realm's DOM methods only. WindowProxy surface (QPT1d, R53 D8): `length`/`window[i]`/`window[name]` follow the connected iframes of the realm's document that have a window, in document order (lazy windows count); the indexed/named properties are getters kept in sync by `syncWin` (iframe insert/removal, src/srcdoc/name changes, innerHTML containing `<iframe`, a `length` read); deviation: an iframe added through a subtree is counted by `length` at once but its `window[i]` property appears at the next sync; iframes with a network src have no window and are not counted. `closed` is a getter set by `__scratchy_realm_release`; `name` is the browsing context name (the iframe's name attribute, kept in step, settable from inside, which re-syncs the parent's `window[name]`). `w.postMessage` runs in the TARGET realm (clone with its intrinsics, its MessageEvent, async through the timer queue); `source`/`origin` come from the native `__scratchy_incumbent()` (the context Rust entered). Code the page runs INTO a realm (`w.eval`, a callback) is still the page as incumbent; `__scratchy_enter(w, f)` runs `f` with the realm entered (QPT1f's script runs enter their realm; timers/listeners of a realm do not yet). Scripts in iframe realms (QPT1f, R53 D9): `srcdoc` wins over `src` (an about:srcdoc window with the parent's origin and base URL). A srcdoc iframe loads as a task after insertion (or a `srcdoc` change, or innerHTML containing one), and the page's own srcdoc iframes load after its DOMContentLoaded and before its load (`__scratchy_frames_boot`, js.rs `run_page`): the parent calls the realm's `__scratchy_frame_load(html, top)` (DOM prelude `frameLoad`), which parses the document (scripting on), runs its scripts in document order (`sRun`: `__scratchy_run_global` inside `__scratchy_enter`, so the realm is the incumbent; an external classic script goes through the page's request queue and blocks the ones after it; modules are started in order), then DOMContentLoaded, nested srcdoc frames, the window `load`, and the parent's `load` on the <iframe> (`o.dom.onLoad`). It stops as soon as the window is closed. An iframe realm's document also supports `open()`/`write()`/`close()` from the parent or the realm: `open()` empties it, `write()` collects markup, `close()` runs it like a srcdoc load (deviation: nothing is parsed before `close()`); inside a parser script `document.write` inserts after it (QA13, as on the page). Script-inserted scripts: the eligible table is page-wide (`xr.s`), and the sweep (any realm's insertion) runs each connected one through `xr.prep[doc]`, the `sPrepare` of the realm whose document it is in (page: 0; a dropped realm's document has none, so its scripts never run). Custom elements: each realm's registry upgrades and connects only elements of its own document (`ceIsConn`, define's query), and `DomTable.ce` keeps the defined names per realm (keyed by the realm's document id, dropped with the realm). Deviation: CE reactions (like MutationObserver records, QPT1c2) follow the realm whose DOM method made the mutation. The pre-fetched QA18b1 documents still run no scripts. Without the snapshot (`SCRATCHY_V8_SNAPSHOT=0`, `SCRATCHY_V8_FLAGS`; QPT1e) `__scratchy_realm_new` makes a plain `Context::new` in the same isolate, re-binds every native registered so far by name (`State.live`: name -> arena index, same closures; a fresh function per context) and replays `Intl`/`queueMicrotask` removal, BASE64_JS and the page's prelude list (`js::realm_preludes()`: prelude, timers, url, globals, intl, fetch, websocket, DOM, cssom, media, mse; not the page-only env/media sweep), then `__scratchy_realm_init` as usual: ~4 ms and ~0.76 MB per realm against ~1-1.5 ms and ~0.67 MB with the snapshot. `data:` iframes keep the stand-in object (R53 D4). postMessage (QMSG1): `window.postMessage(msg, targetOrigin|{targetOrigin, transfer}, transfer)` parses a non-`*`/`/` targetOrigin as a URL (SyntaxError if it does not parse; compared by serialized origin, mismatch drops silently; `/` is the sender's origin, the default), validates `transfer` (MessagePorts only; DataCloneError for others/duplicates) and hands the ports to the event's `ports` (deviation: the same port objects, the sender keeps its reference). Delivery is an entry in the receiving window's explicit posted-message queue (`msgq`/`__scratchy_post_task` in globals_prelude.js; QEL8 moves it to its own task queue) and the listeners run with the receiving realm entered, so `e.source.postMessage` from a handler reports that window as source. Pre-fetched same-origin iframe documents (QA18b1) now load like a srcdoc (`frameHtml`): their scripts run in the realm, then DOMContentLoaded/load. Setting `src` to a document scratchy does not load fires `load` on the iframe at once.

## QY15 — ES modules (module graph pre-fetch)

`script_ref` now returns `ModuleInline`/`ModuleExternal`/`ImportMap` for
`<script type=module>`, `type=module src` and `type=importmap`; classic
scripts with `nomodule` are skipped. QuickJS' module loader is synchronous, so
the static import graph is fetched BEFORE the engine runs, in
`fetch_scripts_with` (src/js.rs; `fetch_scripts` plugs in `wire`): each
module entry's source is scanned (`js_modules::scan_import_specifiers`, a
comment/string/template/regex-aware token scan, not a parser; it also picks
literal dynamic `import('x')`, including from classic scripts), specifiers are
resolved (`resolve_specifier`: import map `imports` + `scopes` first, then
`/ ./ ../` and absolute URLs; bare specifiers with no map entry are an error),
and each level of new URLs is fetched in parallel, breadth-first, capped at
200 modules / 16 MB / depth 32 (github needs more than 5 MB). `data:` modules decode locally. The result is
ordinary `ScriptSource`s tagged by `ScriptKind` (`Classic`, `Module`,
`ModuleDep`, `ImportMap`, `ModuleError`), so `FetchedPage`/`StreamCss` and the
`--js-report` path are unchanged. The engine builds a `ModuleStore` (url ->
source, parsed import map) behind an rquickjs `Resolver`/`Loader` pair that
only reads it; an import of a URL that is not in the store (fetch failed, or a
computed run-time `import()`) fails with a clear error (QY15a: fetch at run
time). Execution: classic scripts first, in order; then module entries in
document order (defer semantics), each through `import(<url>)` so a module
that is also imported elsewhere is evaluated once, under the same 5 s CPU
budget; rejections log once as `<module url>: <message>`; then
DOMContentLoaded/load. Inline modules are named `<doc url>#inline-module-N`
(`import.meta.url` strips the tag). Top-level await works (module promise).

## QF2: Blob / File / FileReader / blob: store

src/fetch_prelude.js (evaluated after intl in js.rs; the home of the QF fetch/XHR
data classes). Blob bytes live in a WeakMap (Uint8Array, never mutated); File
extends Blob; FileReader is an EventTarget whose events fire from one
setTimeout(0). No I/O. The blob: URL store is a JS Map behind
URL.createObjectURL/revokeObjectURL; QF8/QF9 read it with the global
`__blob_lookup(url)` (Blob or undefined). `Blob.stream()` only exists if
ReadableStream exists at load time (QF4). UTF-8 helpers are private to the file.

## QF4: ReadableStream / Headers / Request / Response (data classes)

Same file (src/fetch_prelude.js), all inside the one IIFE so they share the Blob
byte helpers. No network. ReadableStream is minimal: a default reader
(`read/cancel/releaseLock/closed`), `locked`, `cancel`, `values()`/async
iteration, a controller (`enqueue/close/error/desiredSize`), `highWaterMark`
counted in chunks. `tee/pipeTo/pipeThrough`, BYOB, TransformStream and
WritableStream stay ABSENT on purpose (duckduckgo polyfills what is missing).
Headers keep a lowercased `[name, value]` list plus a guard
(none/request/response/immutable): iteration is sorted, values combined with
`, `, Set-Cookie kept separate. Request/Response keep their state in WeakMaps;
a body is `{bytes | src stream, lazily created stream, type, used}` and every
consumer (`text/json/arrayBuffer/bytes/blob/formData`) goes through one
`consume()` that rejects TypeError "body already used". Cloning a body that
came from a page-supplied stream throws (no tee).

INTERNAL HOOK for QF7/QF8: global `__make_response(status, statusText,
headersPairs, bodyBytes, url, redirected, type)` builds a real Response
(`headersPairs` = `[[name, value], ...]` in wire order). Headers get the
`immutable` guard; for `basic`/`cors` Set-Cookie/Set-Cookie2 are dropped;
`opaque`/`opaqueredirect`/`error` have status 0, no headers, no body, url ''.
CORS-safelisted filtering of `cors` headers is the caller's job.

## QY6e: Custom Elements hooks

All in the QY6e block at the end of `DOM_PRELUDE` (js_dom.rs). The mutation
natives are IIFE-local `var`s (`__d_insert/remove/sethtml/settext/setattr/
rmattr/write`), so the first `customElements.define` can rebind them to hooked
versions (and `createElement`/`createElementNS`/`cloneNode`); a page that never
defines an element runs the bare natives (`__ce_stats.hooks` counts hook calls,
0 until then). Hooks query the subtree with one selector built from the defined
names (`ceSel`; `tag[is]` for customized built-ins) and only touch matches.
Per-element state lives in `ceState[id]` (the definition, or -1 for a failed
upgrade) and `ceConnFlag[id]`; `ceNConn` lets removals skip the subtree query
when no custom element is connected. QWPTFLK3: `ceUpgrade` returns at once when
`ceState[id]` is set and sets it to -1 while the constructor runs (DOM "upgrade
an element" steps 1 and 4), so a constructor that inserts its own element is
not upgraded again from the insert hook (that used to recurse until V8's stack
overflowed, and the binary-dependent overflow point made WPT results differ
between builds). Upgrade runs `Reflect.construct(ctor)`
with the element pushed on the definition's construction stack; `HTMLElement`
(js.rs) forwards `new.target` to `ceNew`, which returns that element (or builds
a new one for `new X()`). Reactions are synchronous after the API call, not
microtask-queued. Only document id 0 is upgraded/connected. Not implemented:
adoptedCallback, `:defined`, shadow DOM (`attachShadow` logs `not implemented`
and throws NotSupportedError).

### QY6a: MutationObserver on the same hook layer

`hookInstall` (js_dom.rs, just before `ceInstall`) is the ONE place that rebinds
the mutation natives; it runs at the first `customElements.define` or the first
`MutationObserver.observe`, never otherwise (`__ce_stats.hooks` stays 0).
Each hook does the custom-element work only when `ceOn` and the record work only
when `moN > 0` (live registrations). Hooks call `moChild/moAttr/moChar`, which
walk the target's ancestors through `moReg[id]` (registrations by node id),
build per-observer `MutationRecord`s and queue ONE Promise job (`moNotify`,
observers sorted by creation id, `callback.call(observer, records, observer)`).
Multi-step APIs (`replaceChild`, `append/prepend/before/after/replaceWith/
replaceChildren`, the `outerHTML` setter) wrap their natives in `moBegin/moEnd`
so the pieces of one call merge into one childList record per target.
Transient registered observers: a removed node inherits its old ancestors'
subtree registrations until the next delivery to that observer. Cuts: writes
from the parser or Rust do not notify; `Comment.data =` is a native no-op
(QA18e) so it makes no record; `attributeNamespace` is null.

### QY6g: IntersectionObserver / ResizeObserver with honest minimal semantics

js_dom.rs, right after `MutationRecord`. JS cannot see layout (no
`getBoundingClientRect`, `offsetWidth`/`clientWidth` are 0, the window is a
fixed 1024x768, Q33b2), so these APIs exist to keep scripts running and report
only what can be said honestly:

- `observe(el)` queues ONE initial entry; a single `setTimeout(0)` flushes all
  pending observers (creation order, one callback per observer carrying all its
  entries, `callback.call(observer, entries, observer)`). After that NOTHING is
  ever delivered again (no scroll, resize or layout change fires anything).
  `unobserve`/`disconnect` drop pending entries; `takeRecords` drains them.
- IntersectionObserver: `isIntersecting: true`, ratio 1 ("visible once": a lazy
  loader loads its content; a reader-friendly choice, NOT a measurement),
  unless the target is detached or it or an ancestor has computed
  `display:none` (then false, ratio 0). `rootBounds` is the fixed viewport for
  an implicit root, zeros for an explicit one. Rects sit at (0,0) with the
  element's `width`/`height` attribute size, else 0. Spec validation:
  RangeError for thresholds outside 0..1, SyntaxError for a bad `rootMargin`,
  TypeError for a bad root/callback/target. `root`, `rootMargin` (normalised to
  4 values), `thresholds` (sorted, frozen).
- ResizeObserver: one entry per observed element (re-observing re-queues),
  sizes from the `width`/`height` attributes else 0x0 (a lie of omission; a
  real RO would not even fire for a 0x0 element). `contentRect`,
  `borderBoxSize`, `contentBoxSize`, `devicePixelContentBoxSize` carry the same
  numbers; `box` is validated, otherwise ignored.
- PerformanceObserver: inert. `observe()` validates its argument, nothing is
  ever delivered, `takeRecords()` is `[]`, `supportedEntryTypes` is `[]`.
- `DOMRectReadOnly` / `DOMRect` minimal (x/y/width/height, derived edges,
  `fromRect`, `toJSON`); no `DOMRectList`, no `DOMPoint`.
- Zero cost: no hooks, timers or state exist until the first `observe()`.
- The missing piece is the geometry bridge (TODO.txt QY-IO-GEOMETRY).

### Meta refresh (QY19)

After a fetched page arrives, `handle_nav_result` looks for the first `meta[http-equiv=refresh]` (`meta_refresh_action`). With JS off `<noscript>` children are elements (`new_parser`), with JS on they are text, so only the right one is found. A 0 s `url=` redirect never installs the page: it replaces the current history entry and fetches the target (`follow_meta_refresh`). A delayed one (<=10 s) installs the page and a sleeping thread (same shape as `schedule_image_flush`) sends `AppEvent::MetaRefresh(generation, url)`; a changed `nav_generation` cancels it. `App::meta_chain` (reset by link/`:go`/history/`:refresh`) caps chains at 3.

## Cascade layers (QY17)
`@layer` is parsed in `TopLevelParser` (`LayerState` registers every mentioned layer path and its prefixes in first-mention order; `parse_stylesheet` ranks them afterwards, sub-layers before their parent's own rules). `StyleRule::layer`: 0 unlayered, 1 user-agent (`parse_ua_stylesheet`), n>=2 author layer. The cascade key in `matching_declarations_for` is a u64: bit 62 important, bits 32..61 layer rank (`normal_key`; reversed for important by `important_key`/`to_important`), low bits specificity. `@import ... layer(x)` is expanded into `@layer x { ... }` text by `expand_css_imports`.

## FormData (QF3)

Same file, after Blob/File. Entries are `[name, value]` pairs in a WeakMap
(value: string or File). A Blob value becomes a File (name `blob` unless a
filename is given). Names and values are USVString-normalised (lone
surrogates to U+FFFD). Iteration (`entries/keys/values/Symbol.iterator`) is
live over the list, in insertion order. `new FormData(form)` never reads the
controls: it logs `scratchy: js: not implemented: FormData(form) (waits for
Q30e)` and stays empty (forms rule, AGENTS.md). A FormData request/response
body is serialised as multipart/form-data with a random boundary (QF13 partly
done, serialisation only). `formData()` still parses urlencoded bodies only,
so a multipart body rejects with TypeError (QF4a).

## Net policy and wire codec (QF5)

`src/net_policy.rs` is pure (no I/O, no JS): `RequestPolicy` is the ONE place for every network number and open decision (limits, timeouts, redirect limit, `sync_xhr_timeout_ms`, `beacons_send`, `private_targets`/`file_targets` as `PrivatePolicy`). `check_request(page, url, method, has_body)` returns a `PolicyOutcome` whose `warning` class is logged with `format_net_warning`; same-origin never warns; schemes, bad ports, mixed content (loopback exempt) and method/body rules apply in every mode; `check_redirect_hop` re-runs it per hop. CORS helpers (`classify_fetch`, `needs_preflight`, `check_cors_response`, `check_preflight_response`) and the forbidden header lists (a test pins them to fetch_prelude.js) are pure. `classify_beacon` + `format_beacon_log` serve the suppressed-beacon log. `is_public_ip` is mirrored by hand in wire (QF6). The codec in net.rs (`build_wire_request`, `parse_wire_response_full`, `fetch_full_via_wire`) was not called by anything when QF5 landed (QF7's `js_net.rs` uses it now); it can carry `X-Wire-Policy/Max-Bytes/Timeout-Ms/Max-Redirects` (`wire_cap_headers`). Wire already supports every method and Content-Length bodies; what it lacked then was added by QF6 (wire v2) and QF14 (v3).

## Page JS network queue (QF7)

JS never does I/O and cannot touch `App`. The flow:

1. `__net_send(desc) -> id` (native, src/js_net.rs `NetState::send`) checks, in order: page URL known, URL parses, `max_requests_per_page`, `check_request` (warning -> engine log level `net-warning`), `classify_fetch` (CORS kind), beacon classification (suppressed: logged in detail with level `net`, answered as an opaque empty response, never sent unless `beacons_send`), CORS preflight need (rejected as `not implemented (QF13 preflight)` until QF13; now an OPTIONS job is sent first, see "CORS preflight (QF13)"), body vs `max_in_flight_bytes`. It serialises with `build_wire_request` plus `wire_cap_headers` (`X-Wire-Policy` only when `private_targets == Block`; `Origin` for cors) and pushes a `NetJob` to `JsRequests.net`. Refused requests and suppressed beacons go to `JsRequests.net_done` and are delivered asynchronously like any result. `__net_abort(id)` forgets the request (a late result is ignored) and cancels it if queued (`net_abort`).
2. `App::js_sync` drains those into `App::net` (`NetScheduler`: `max_concurrent_requests` in flight, request bytes in flight under `max_in_flight_bytes`, FIFO) and `pump_net` starts one thread per job through the executor (`js_net::net_executor()`, a thread-local set by `App::new_*`/`--js-report`; tests inject a fake `NetExecutor`). The thread sends `AppEvent::JsNet(nav_generation, id, Result<WireResponse, NetError>)`. `handle_js_net` drops stale generations, frees the slot, pumps again, calls `JsEngine::deliver_net` and `js_sync`. While `:help` parks the engine results wait in `App::parked_net` and are delivered by `restore_saved_page`. `fetch_url_inner` resets the scheduler.
3. `deliver_net` -> `NetState::finish`: cross-origin `cors` responses go through `check_cors_response` (failure = network error, logged with the reason) and header filtering (safelisted + `Access-Control-Expose-Headers`); no-cors = opaque. Then JS `__net_complete(result)` (src/fetch_prelude.js `__net_request`) runs under `CALLBACK_BUDGET`, promise jobs drain.
4. `sync: true` runs the executor on the engine thread with `sync_xhr_timeout_ms` as the wire read timeout; the result is read with `__net_sync_result(id)`; the JS helper throws `NetworkError` on failure.
5. `--js-report`: `drive_report_net` runs the same queue on the calling thread after load and after each timer slice; `NetState.entries` gives the `--- network (N requests, B bytes) ---` section. The report's page URL is the `<base href>`.

Every number comes from `RequestPolicy` (src/net_policy.rs).

## wire protocol version 2 (QF6)

wire now answers `X-Wire-Version: 2` on every response (3 since QF14); an old wire sends none. As built (QF7a), there is no separate probe: page networking fails closed when an answer has no version or one below 2, and the cookie client stops sending jar headers when the version is below 3 (see "Old wire fails closed (QF7a)" and "Cookie client (QF14b-scratchy)"). Wire reports `X-Wire-Final-Url`, `X-Wire-Target-Class` (worst class over all hops), `X-Wire-Hop-Classes`, and marks its own errors with `X-Wire-Error` (`1` transport, `too-large`, `policy`; the `parse_wire_response_full` codec already consumes `x-wire-error`/`x-wire-final-url`, the class headers stay in `headers`). Over-cap bodies and redirect-limit overruns come back as a 502, not a truncated 200. Details: wire/NOTES.md.
## fetch (QF8)

`window.fetch` is pure JS at the end of src/fetch_prelude.js, a thin layer over `__net_request` (QF7): `new Request(input, init)` validates, then `data:` is decoded in JS and `blob:` is read from the QF2 store (both local, no policy, no network); http(s)/`file:` go to `__net_request` (policy, CORS, beacons, opaque no-cors); any other scheme rejects with TypeError. Bodies are buffered through the Request (`arrayBuffer()`, FormData is multipart). `redirect:"error"` rejects on `response.redirected`; `"manual"` acts as follow. Ignored: cache, referrer, referrerPolicy, integrity, priority. Credentials are accepted (`include` with ACAO `*` fails the CORS check in QF7); since QF14b the `credentials` mode decides whether the wire jar is used (see "Cookie client (QF14b-scratchy)"). Failures reject with `TypeError("Failed to fetch")`; the real reason is the QF7 console line.

## XMLHttpRequest (QF9)

`XMLHttpRequest` (plus `XMLHttpRequestEventTarget`, `XMLHttpRequestUpload`) is JS at the end of src/fetch_prelude.js over `__net_request` (api `xhr`), so policy, CORS, limits and logs are fetch's. A hidden `__xhr_internals` (forbidden-header list, body extraction, UTF-8 decode, response bytes) is shared from the QF4/QF5 closure; the forbidden request header list is the Headers one. Events: readystatechange 1 after open; async send fires loadstart, then (rs2, rs3, one progress when the body is non-empty, rs4, load, loadend); network/CORS/policy failure is rs4, error, loadend with status 0; abort is rs4, abort, loadend then readyState 0; `timeout` is a JS timer that cancels the request (rs4, timeout, loadend). Non-simple cross-origin requests get a CORS preflight like fetch (QF13; before QF13 they failed with a log line). Sync (`open(..,false)`): `send()` blocks in `__net_request` with `sync_xhr_timeout_ms`, fires rs4/load/loadend, and throws a NetworkError DOMException on failure; `responseType` or `timeout` on a sync XHR throws InvalidAccessError (at open and on the setters, as browsers do on a window). Since QF9a `responseType "document"` and `responseXML` return an inert second document through `DOMParser`'s machinery (`g.__parseDocument`, js_dom.rs): text/html (async only, `document` type only) is parsed by html5ever with scripting disabled (`__d_parse_html`); XML types go through roxmltree (`__d_load_xml`, well-formedness checked; a failure yields a Firefox-style `parsererror` document); other types give null. DOMParser documents have URL about:blank, `defaultView` null, scripts never run (parser-made scripts are never marked eligible, QY34). Limits: element prefixes are dropped from `tagName` (namespace kept, `xmlns:*` attributes kept), no doctype node for XML, PIs dropped, the document URL is not the page URL. Deviations: the body arrives whole; upload fires its events only at the end; open() user/password ignored; `withCredentials` = credentials `include` for the wire jar since QF14b. `navigator.sendBeacon` is QF10 (below).

## navigator.sendBeacon (QF10)

`Navigator` exists (js.rs; `navigator` inherits from `Navigator.prototype`); `sendBeacon` is JS in src/fetch_prelude.js over `__net_request` (api `beacon`), body from the hidden `__beacon_body` (the Request body extractor, synchronous). TypeError for a bad or non-http(s) URL; `false` when queued bytes + body exceed `RequestPolicy.beacon_queue_bytes` (read through `__net_beacon_limit`); otherwise `true`. The native `__net_send` does policy warnings and, with `beacons_send` false (default), only logs the beacon in detail and answers opaque; with it true the request goes out like any keepalive POST (same path, no second sender). `WebSocket` exists since QF19w2 (NOTES section "WebSocket (QF19w2)"); `EventSource`, `Worker`, `SharedWorker` stay undefined on purpose: feature detection then takes the page's fallback, and no surveyed page used them at startup. `<a ping>` is QF10a.

## Old wire fails closed (QF7a)

`NetState.finish` looks at `X-Wire-Version` of every answer: below 2 (or absent) means a wire from before QF6 that cannot report `X-Wire-Final-Url`, so the redirect/CORS/policy checks would run on guesses. Page networking is then OFF for the page: one `net-warning` line, the answer becomes an error, later `send()` calls are refused silently (outcome `blocked: ...` in the `--js-report` network section). Wire v2 works without cookies; v3 with. The check is off in test builds (`check_wire_version`; fakes send no version). `X-Wire-Target-Class` (worst class over the hops) warns for a DNS NAME that resolved to a non-public address (Warn mode); Block mode relies on wire's 403 `policy`. Per-hop CORS is QF7b. Response headers starting `x-wire-` never reach the page (`X-Wire-Cache`, W-CACHE1, included). W-CACHE1: fetch()'s `cache` mode becomes standard request headers (`cache_mode_headers` in net_policy.rs, added in `NetState::send` after the CORS classification so they never cause a preflight, only when the page set no `Cache-Control`/`Pragma`); `:refresh` / `location.reload()` sends `Cache-Control: max-age=0` on the document request (`net::with_reload_revalidate`), `:refresh hard` sets `net::set_hard_reload` so every request built by `build_wire_request_with` / `build_get_request_text_with` carries `Cache-Control: no-cache` + `Pragma: no-cache` until the next navigation that is not a hard reload. wire's cache itself: wire/NOTES.md "The HTTP cache".

## Cookie jar lives in wire (QF14, wire half)

wire (version 3) owns a memory-only cookie jar, partitioned by top-level site and selected per request with `X-Wire-Jar: scratchy` (scratchy sends the headers since QF14b-scratchy, below). With the jar, wire strips `Set-Cookie` from answers and ignores a client `Cookie` header; `document.cookie` goes through wire's control channel (`X-Wire-Cookie-Op` + `X-Wire-Cookie-Url`). scratchy detects the jar by `X-Wire-Cookies: on|off` plus the version. Details and limits: wire/NOTES.md "The cookie jar".

## QSD0: query methods on prototypes only

querySelector/All and getElementsByTagName/ClassName are installed once on Element.prototype, DocumentFragment.prototype and document (queries(P)); wrap() assigns nothing per instance, so a page may redefine or freeze them (GitHub popover polyfill does) without breaking fragment wrappers.

## Border width defaults (QA11a)

A border with a style but no width is `medium` (3px); `thin/medium/thick` are 1/3/5px; an unset or inherited border colour paints as the element's own `color` (currentColor). Two WPT tests that only passed while unset colours painted nothing (`row-group-margin-border-padding`, `table-rules-rows-vertical`) are accepted failures until QA11a2 (real border-collapse).

## Cookie client (QF14b-scratchy)

scratchy sends the jar headers wire (QF14) understands; ALL of them are built in `net_policy.rs` (`wire_cookie_headers`, `cookie_control_headers`, section "QF14b cookies"; a test greps the other sources for the literals). `CookieContext {top_site, kind}`: `Navigation{initiator}` (top site = the destination, `X-Wire-Nav: 1`, `X-Wire-Initiator-Site` only for a page-initiated navigation: link click, script, meta refresh; `:go`, startup, `:refresh` and history are browser-initiated = same-site), `Subresource` (image, stylesheet, script, module, iframe: always the jar), `Fetch{credentials, same_origin}` (omit, or same-origin mode on a cross-origin URL: `X-Wire-Cookies: off`, no jar; XHR `withCredentials` = include). Every sub-resource loader takes the page URL (`top`) next to `base`; `site_of(url)` = scheme + registrable domain with a COPY of wire's short suffix list (keep identical; wire normalizes again). A `file:` page has no cookie origin: no jar headers and `document.cookie` is ''.

`document.cookie` is an accessor on `Document.prototype` over natives `__cookie_get`/`__cookie_set` (js_net.rs `NetState::cookie_op`): a SYNCHRONOUS control request through the same `NetExecutor`, blocking the engine thread for at most `RequestPolicy::cookie_control_timeout_ms` (2 s). Any failure is logged once (`document.cookie: ...`) and it reads '' for the rest of the page; no `--socket` is silent. Second documents (iframe contentDocument, createHTMLDocument) read '' and ignore sets.

Old wire: `CookieSupport` learns from the first jar response (`X-Wire-Version >= 3` and `X-Wire-Cookies: on`); too old or `off` logs ONCE `scratchy: cookies unavailable: wire is too old (restart it)` / `wire has cookies disabled` and no jar header is sent afterwards. wire v2 strips every `X-Wire-*` request header (checked in git history, QF6 `is_request_header_to_skip`); a v1 wire (before QF6) would forward them upstream, but the first request of a session is the top-level navigation whose top site is its own destination, so only that site name leaks. Not done: `cookieStore`, the `crossorigin` attribute credentials mode for img/script/module (they always use the jar), dropping same-origin-mode credentials on a cross-origin redirect hop, the access token for the control channel (wire hook `control_authorized`), `--profile`.

## QY6f: template.content

html5ever fills `template_contents` (a Document-data node = kind 11 fragment); `template_content(h)` creates it on demand. `__d_template_content(id)` exposes it; the JS `content` getter maps it to ONE lazily made inert document (`makeDoc`, via `owners`), so ownerDocument is inert and custom elements in it are never upgraded (ceIsConn is false, and the cloneNode hook skips non-main documents). rcdom serializes only `children`, so `serialize_node` is our own walk that prints template content. innerHTML set parses into the content; deep clone copies it. appendChild on the template element itself still adds ordinary children (hidden by `template { display:none }`). Not done: shadowrootmode (QSD3/9).

## CORS preflight (QF13)

A cross-origin `cors` request that `needs_preflight` (non-safelisted method or header) first sends an `OPTIONS` job through the same scheduler (so it counts toward the per-page cap and in-flight limits): `Origin`, `Access-Control-Request-Method`, `Access-Control-Request-Headers` (lowercased, sorted, only the non-safelisted), no body, no jar (`X-Wire-Cookies: off`), `X-Wire-Max-Redirects: 0` (a redirecting preflight fails). `NetState.preflights` maps the preflight job id to the waiting actual request; `finish` routes the answer to `preflight_done`, which validates with `check_preflight_response`, fills `PreflightCache` (key = origin, URL, method, header set, credentials; expiry on the engine clock = the manual clock in tests; Max-Age default/cap/max entries from `RequestPolicy`) and releases the actual job (`take_released`, moved into `JsRequests.net` by `deliver_net`). A failure logs `CORS preflight failed: <url>: <what is missing>` and rejects/errors the page request; nothing else is sent. Sync XHR runs both inside one blocking call, the actual request getting the time that is left of `sync_xhr_timeout_ms`. Redirects: wire follows them itself, so a PREFLIGHTED request is sent with `X-Wire-Max-Redirects: 0` and a 3xx fails closed (QF13a: re-preflight the new URL per hop); for other cors requests only the final answer is checked, against `Origin: null` when `X-Wire-Final-Url` is another origin (intermediate hops cannot be inspected). `navigator.sendBeacon` is a keepalive POST with api `beacon` (logged, not sent by default).
## QSD1: document.domain

Getter is the location hostname. Setter accepts the same value or a dot-boundary suffix with at least one dot (no public suffix list: "co.uk" passes, "com" does not), remembers it for the getter, otherwise throws SecurityError. No origin relaxation happens. Document ParentNode accessors and getElementsByName live on the main `doc` object; created documents inherit them.

## QSD1b: crypto

`crypto` is a window getter over one `Crypto` instance; `getRandomValues` accepts integer typed arrays only (TypeMismatchError otherwise, QuotaExceededError above 65536 bytes) and fills them from `os_random_bytes` (getrandom(2) via libc). `randomUUID` builds a v4 id from 16 such bytes. `crypto.subtle` is a `SubtleCrypto` placeholder: every method logs `not implemented` and rejects with NotSupportedError. `isSecureContext` is true for https/wss and loopback hosts.

## QSD2: roots and document position

`getRootNode` walks `__d_parent` to the topmost node (document, detached element, DocumentFragment, template content fragment, second document); `composed` is ignored until QSD3 adds the shadow-host step. `isConnected` is "root is a Document" (so true inside a second document, false in template content). `compareDocumentPosition` compares parent chains; different roots give DISCONNECTED|IMPLEMENTATION_SPECIFIC plus PRECEDING/FOLLOWING by node id. Attribute nodes are not modelled.

## QSD4: constructable stylesheets

`src/cssom_prelude.js` keeps each constructed sheet's rules as text (rule splitter + declaration parser in JS; `@import`/`@charset` dropped with a console warning). `document.adoptedStyleSheets` is a Proxy over a plain array (index writes are type-checked and republish; the setter validates every item and copies). The adopted sheets' text (adoption order, disabled sheets skipped, `media` wrapped in `@media`) is published through `__d_adopted(text)` only when it changes: it sets `JsRequests.adopted_css`, the js_style thread-local (getComputedStyle) and bumps the DOM write generation. `App::js_sync` copies it to `App::adopted_css`; the layout build appends it after `extra_css` (`images::with_adopted_css`, borrowed when empty), so adopted rules are unlayered author rules that come after every `<style>`/`<link>` sheet. Cleared by `replace_document`. `baseURL` is stored but url() is not rebased. No ShadowRoot yet (QSD3/QSD7).

Stylesheet links after load (QY54b1), `src/link_css.rs`: `App::extra_css` is the parsed page's `<link rel=stylesheet>` sheets concatenated in document order (fetched before the first layout, `net::fetch_stylesheet_parts`, which also returns `SheetParts`: each link's key with the byte range of its text, or `None` when the fetch failed). `App::link_sheets` (`LinkSheets`) keeps that composition. A key is the link's `href` resolved against the page base (`link_key`; `data:` URIs as written). The DOM natives (`__d_insert`, `__d_remove`, `__d_sethtml`, `__d_settext`, `__d_write` and the `href`/`rel`/`disabled` attribute writers) set `JsRequests.links_changed` when a CONNECTED `<link>` is inserted, removed or changed; `App::js_sync` then calls `App::sync_link_sheets`: `LinkSheets::sync` walks the document's applying links (`rel` has `stylesheet`, `href` set, no `disabled`; `media` is ignored, as for parsed links) and rebuilds the text from known sheets (ranges of the current `extra_css`, or `spare` texts of links that were removed/disabled), so nothing is refetched or re-copied for an unchanged page. Unknown keys are fetched on one thread each through wire (`fetch_one_stylesheet`, `@import`s expanded like the parsed links) and come back as `AppEvent::LinkCss(nav_generation, key, Option<text>)` -> `settle` + sync again. A changed text replaces `extra_css` and `set_js_extra_css` (one `css_ver` bump: the JS thread's `page_css`/GeomCache/StyleCache and the App build see it at once), then `load` (text) / `error` (failed) is dispatched once per element and key, after the CSS is in place. Parsed links count as fired at install time (`LinkSheets::adopt`), so they still get no events. A key that an earlier link already applies is applied once, at its first link, and the later link fires nothing (also for parsed duplicates): see R58 "QY54b1 note" for why (YouTube watch). A page installed without parts (stream, `:help`, tests) keeps its whole `extra_css` as an opaque prefix whose links are not refetched; new sheets go after it. Still all link sheets cascade after every `<style>` element (`rules_cache`: style pieces, then `extra_css` as one piece, re-parsed whole when it changes). The headless `--perf-report` path (perf.rs) has no App, so it does not apply script-inserted links.

## Media via mpv (QM1 investigation, 2026-11-17; nothing built)

Design memo only; media stays placeholder-only until the user agrees a design
(AGENTS.md "Off limits"). Evidence: REGRESSIONS.md R27; questions and items:
TODO.txt QM1-D, QM1a-QM1j. An mpv process cannot be embedded on Wayland
(`--wid` is X11 only and compiled out of this mpv; a wl_subsurface must belong
to our own client connection; a second toplevel is tiled by dwl and cannot
place itself). The route that fits scratchy's CPU paint model is libmpv
in-process: software render (`MPV_RENDER_API_TYPE_SW`) of each frame into a
buffer sized to the `<video>` box, painted like an `<img>`, so scroll,
clipping, z-order and QY18 translate need no new machinery. Bytes come from
wire through libmpv's stream_cb (seekable reads, so the media channel needs
Range requests), which keeps the request policy, cookie partitioning and CORS.
`App::redraw` repaints and presents the whole page on every frame today, so
video needs a damage-only redraw (QM1c). MSE/EME are out of reach (YouTube
needs MSE).

QM1c/QM1d (2026-11-17, R30): the media byte channel is repeated ranged GETs
through wire (wire already forwards `Range` and passes 206/Content-Range; no
protocol change), served to libmpv's stream_cb from a 1 MiB chunk cache
(TODO.txt QM1e has the design). A full `App::redraw` is 67-101 ms on Wikipedia
pages, so video frames get their own rect-only redraw (QM1g1).

## QM1g/QM1g1: video paint and the rect-only redraw (2026-11-17)

Enabled only when `media_video::boxes_enabled()` (cargo feature `media` AND a
loadable libmpv); otherwise the default build lays `<video>` out exactly as before
(inline, fallback content). Enabled: a UA rule `video { display: inline-block;
object-fit: contain }` and `images::build_video_box` give a replaced box, built as an
`<img>`-like `BoxContent::Image` whose "image" is an EMPTY-pixel `DecodedImage` (the
sentinel `paint` recognises; its `mask` field carries the poster). Size: CSS or
`width`/`height` attributes, else the video's own size (`media_video::intrinsic`, set
from the engine before each layout build; `JsEngine::media_size_changed` makes
`js_sync` restyle when a size becomes known), else the poster's, else 300x150.
Posters are fetched like `<img>` (`net::find_external_resources`). A page with a
`<video>`/`<audio>` starts the JS engine even with no scripts (`run_page_scripts`).

Painting (`src/media_video.rs`): `paint` calls `paint_box`. During a full
`App::redraw` it fills the visible (clipped) part of the box with SENTINEL (top byte
0xFE; `blend` always writes a top byte of 0, so any later paint over a pixel
destroys it). After everything else is painted `finish` scans each rect: pixels still
equal to the sentinel are exactly the video pixels no other content covers (z-order,
overflow, clipping, scroll come free). That is the cached OVERLAY MASK (`Vec<bool>`
over the visible rect); the newest frame (or the poster composed to the box size, or
black) is copied through it (whole rows with one memcpy). `VideoBoxInfo` keeps rect,
mask, bounding rect. A box painted into another buffer (mask-image layer) or under
opacity is drawn black and flagged `layered` (no frame substitution, no rect-only
update: the app falls back to a full redraw for it).

Frames: `vo=libmpv` + `mpv_render_context` type `sw`, format `bgr0` (= 0RGB), size =
the box's content size in device pixels (mpv letterboxes: `keepaspect`, `panscan`,
`video-unscaled` per `object-fit`). QM1g2a: the render context lives on a render
worker thread (`media_render::RenderWorker`, thread `scratchy-video`, one per backend;
it creates, updates, renders with and frees the context, which is not thread-safe).
mpv's render-update callback (an mpv thread) only calls `Signal::notify`; the worker then
runs `update` and `render` into a fresh buffer, puts the `Arc<VideoFrame>` into a ONE-frame
slot (an unread older frame is replaced, never queued) and calls `media_video::wake()`, a
coalesced `AppEvent::VideoFrame` posted through the winit proxy; `MpvBackend::take_frame`
on the main thread only takes the slot. Staleness: a target change, seek, load or unload
bumps an epoch and empties the slot; a frame finished for an older epoch or target is
dropped, not published. A target change while a picture exists re-renders the current
frame at once (`redraw`; not after a hide, where `vid=no` may have dropped the picture).
`MpvBackend::drop` drops the worker first (it joins the thread, which frees the context)
and only then destroys the core. With no
visible box (`MediaHost::set_targets` gets only boxes with unobstructed pixels, empty
while `:help` parks the page) the worker consumes frames with SKIP_RENDERING (vo=libmpv
stalls playback if they pile up) and, once metadata is known and the file has an audio
track, `vid=no` stops decoding (a video-only file keeps decoding: `vid=no` ends it at
once). Audio: `ao=null` in tests, else `SCRATCHY_MEDIA_AO` or `pulse,null`; mpv only
opens the device while an element plays (D5 still blocks unmuted autoplay).

QM1g1 (`App::video_tick`): on `VideoFrame` it takes the waiting frames; when the
page is unchanged (no `layout_dirty`, same surface size, `AppVideo.page_buf` = a copy of
the last presented page, kept only while a video box is visible) it copies the frame
through each box's mask into `page_buf` and, when `buffer.age() == 1`, also into the
softbuffer buffer (else copies `page_buf` whole, ~4 MB), then
`present_with_damage(bounding rects)`. Anything else (frame of another size, layered
box, dirty layout) returns false and the normal full redraw runs. NO `pre_present_notify`
on this path: it arms winit's frame-callback gate, and a damage present every ~33 ms
starved the next full redraw (a scroll waited ~9 s in the nested dwl).

Limits (TODO QM1g2): (rendering left the main thread in QM1g2a; the main thread still
copies each frame into `page_buf` and the softbuffer buffer and presents it); a `<video>` inside a mask-image/opacity layer paints black; no
rounded-corner clip; `controls` are not drawn (QM1h-ish); paused elements keep their
libmpv handle (QM1f1 a); `webm`/`vp9` could not be tested (no encoder here).

## Media spike QM1b (tools/mpvspike, 2026-11-17)

Own Cargo project (like tools/inject; nothing in scratchy/src, not in
scratchy's Cargo.toml). Build: `cd tools/mpvspike && cargo build --release
--offline`; run `mpvspike FILE` or `mpvspike http://127.0.0.1:PORT/NAME.mp4`
(the http form feeds mpv ONLY through `scratchy-media://1` stream_cb with 1 MiB
ranged GETs; `rangeserver.py DIR PORT` is a loopback Range server;
`connectlog.c` is an LD_PRELOAD connect() logger since strace is missing).
Keys: space, l/h (+-5 s), s (seek 12.5), j/k and wheel (scroll), left/right.
It proved: libmpv SW render (bgr0) into a rect of a softbuffer page, wheel
scroll plus window-edge clipping, exact seeks (use `+exact`), 0.7-0.8 core and
90-96 MB RSS for 720p in 640x360, and that no connect() other than our own
reader's happens. Numbers and the mpv option list: TODO.txt QM1b RESULTS.

## Form controls: live state and painting (Q30c, Q30d)

Checkbox/radio state (`CHECK_LIVE`, forms.rs) and text state (`TEXT_LIVE`, forms_text.rs) are thread-local maps keyed by `node_id`, cleared on navigation. Neither is in the layout tree: the box only carries `ComputedStyle::check_widget` / `text_widget` and the painter (`paint_checkable`, `paint_text_control`) draws from the live state, so toggling or typing is a repaint. A text control is "dirty" once script or the user set its value; until then the `value` attribute (textarea: its text) rules. While an enabled text control is focused, `App::text_control_key` owns the keys after the page keydown handlers; Esc blurs and returns the keys to the vim bindings. `TextEditBuffer` (app.rs) is shared with the `:` command bar.

## QM1f: HTMLMediaElement (2026-11-17)

Three layers. `src/media_el.rs`: `MediaCore` is the HTML media element state
machine (resource selection from `src`/`<source>` with `type` filtering,
networkState/readyState, play()/pause()/seek/ended/loop, volume/muted/rate, the
event order, MediaError codes, D5 autoplay = muted or a user gesture, canPlayType)
over the `MediaBackend` trait; it emits `Out` items (events, promise settlements)
and never touches JS. `FakeBackend` (deterministic, scripted by tests) and
`UnavailableBackend` (placeholder: loadstart + `error` code 4, play() rejects
NotSupportedError, ONE `not implemented: media (libmpv not available)` line) live
there too, as does `MediaHost` (all elements of a page, D7: at most 2 playing, the
once-only log lines). `src/js_media.rs` + `src/media_prelude.js`: natives
`__media_*` and the thin JS wrapper (attribute reflection, `<source>` candidates,
MutationObserver hooks for src/child changes, events always dispatched from a timer
task, play() promises by token, `Audio`, `TimeRanges`, `MediaError`). The page JS
keeps a `setTimeout` pump while any backend is live. `src/media_mpv.rs` (cargo
feature `media`, libloading dlopen of `libmpv.so.2`): the QM1b options, `vo=null`
`ao=null` (env SCRATCHY_MEDIA_AO), `stream_cb` protocol `scratchy-media://` backed
by `media_net::MediaStream` (mpv never opens a URL itself), `eof-reached`/
`playback-restart`/`file-loaded`/`time-pos` mapped to `BackendEvent`s, seeks
`absolute+exact`. First real load logs `media: mpv`. QV8-5: `media` is a DEFAULT feature (libmpv is dlopen'd, so no build-time dependency; without
libmpv at run time the inert placeholder stays); `--no-default-features` drops it. `--js-report --run-timers` sets
`__media_wait=20` so the simulated clock still lets a real player advance.

## QNET1: scratchy cannot reach the internet (seccomp network lock)

`src/netlock.rs`, installed first thing in `main()` (the real binary only; library
and test code never install it, the filter is irreversible), before any thread or
library load. A hand-assembled seccomp-BPF program (libc `prctl`, PR_SET_NO_NEW_PRIVS
first): `socket()`/`socketpair()` with any domain other than AF_UNIX fail with
EAFNOSUPPORT; `io_uring_setup` fails with ENOSYS (io_uring can create sockets);
any other ABI (32-bit compat, x32) kills the process. x86_64 and aarch64 numbers.
Inherited by all threads and by libmpv/ffmpeg. No escape hatch. stderr line:
`scratchy: network: unix sockets only (wire)`. If the kernel refuses (or the OS/arch
is not Linux x86_64/aarch64) a warning is logged and scratchy continues unfiltered.
AF_NETLINK is NOT allowed (see TODO.txt QNET1 for the measurement). Installed for
every mode of the binary: windowed, `--js-report` and `--wpt` (they only use wire's
unix socket).

mpv defence in depth (media_net.rs `MPV_OPTIONS`): `demuxer-lavf-o` and
`stream-lavf-o` = `protocol_whitelist=%11%crypto,data` so ffmpeg cannot open nested
http/tcp/file URLs from an HLS playlist.

Guards (normal tests, src/tests/qnet1_tests.rs): `no_network_crate_in_normal_dependencies`
(`cargo tree -e normal`), `sources_do_not_use_inet_sockets` (src/ outside tests), and
the child-process probe `socket_filter_allows_only_unix_sockets`.

Verify from outside: `unshare -rn scratchy --socket /path/to/wire.sock URL` with wire
started OUTSIDE the namespace (filesystem unix sockets cross network namespaces):
it must behave exactly as before; `file://` pages need no wire at all. For a connect
log use `tools/mpvspike/connectlog.c` with LD_PRELOAD: only AF_UNIX connects appear.


## WebSocket (QF19w2)

Files: src/ws_frames.rs (SHA-1 for the accept key, masked frame encoder, incremental decoder, fragment
assembler with the RFC close codes 1002/1007/1009; cap 16 MiB), src/js_ws.rs (handshake, threads,
queue, natives, `JsEngine::pump_ws`), src/ws_prelude.js (the class). Design: wire is a pipe
(TODO QF19); scratchy frames. One reader thread per socket (handshake, then frames, with a 250 ms
read timeout to notice an abort or the 5 s wait for the server's close frame) and one writer thread
(masks with a fresh `getrandom` key, coalesces `Flushed` events for `bufferedAmount`). Events go
through `WsShared` (queue + one outstanding wake-up, `AppEvent::JsWs`) and `pump_ws` hands them to
JS 64 at a time, each as its own task under the callback budget (`__ws_ev` / `__ws_ev_bin`).
The engine owns the socket handles (`WsState`): dropping it (navigation, page replacement) sends
close 1001; a page parked behind `:help` keeps its sockets and its queued events (pumped on
restore). `--js-report` has no App: `drive_report_net` calls `drive_report_ws`, which waits up to
3 s for sockets still connecting. Tests: src/tests/qf19w2_tests.rs (framing units, and a REAL wire
built from ../wire into target/wire-for-tests with a local echo server; they skip when wire cannot
be built; `SCRATCHY_TEST_WIRE=/path/to/wire` overrides). Known limits: no extensions
(`extensions` is always ''), `on*` handlers run after the `addEventListener` ones, a message is
delivered whole (no streaming), sending is unbounded (no limit on queued outgoing bytes).

## QV8-5: V8 + media are the default build

`Cargo.toml` `default = ["engine-v8", "media"]`: a plain `cargo build --release` /
`cargo install --path .` gives V8 (`src/js_engine_v8.rs`, mounted as `js_engine`) and libmpv
video. QV8-6 (user 2026-11-19) removed QuickJS (`engine-quickjs`, `rquickjs`, the old
`src/js_engine.rs` backend, the `scratchy-quickjs` CI job); `git revert` of that commit brings
it back. `lib.rs` has a `compile_error!` if `engine-v8` is off. CI: `scratchy` (default, V8
archive cached + sha256-pinned, clippy count), `wire`, `snag`. Release binary ~63 MB stripped; first clean build ~15 min.

## QM1g2d: the controls bar (src/media_controls.rs)

`<video controls>` / `<audio controls>` get a 32 px bar painted by scratchy as ordinary pixels over the
video picture, through the box's overlay mask, AFTER `media_video::finish` (not in `paint_box`: the bar is
semi-transparent over the frame, so the frame must be composited first). State comes from the element
core (`JsEngine::media_controls_state`), actions go back through the element's JS API
(`__scratchy_media_ui`, a user gesture). `App::controls_refresh` (run from `about_to_wait`) repaints only
the bar rect when the quantised state (250 ms) or visibility changed (`recomposite` puts the picture back
under it), and returns the next deadline (hide after 2 s, 250 ms refresh while playing). Bar presses are
UA chrome: the page gets no mouse events for them; keys (D6) only act on the focused element, after the
page's keydown.

## QY53g: selector matching inside a layout build (src/style.rs, src/dom.rs)

`matching_indices_for` tests only the `RuleIndex` candidates of an element (buckets by the id / class /
tag of each selector's rightmost compound; for a pseudo-element selector such as `.a::before` the
compound LEFT of the pseudo-element, since `::before` is its own compound behind a pseudo-element
combinator). Inside a build (`ANCESTOR_BLOOM` set next to `MATCH_MEMO` in images.rs) a counting bloom
filter holds the tag, namespace, id and classes of the element's ancestors (`AncestorBloom`, a stack
synced to each element's parent chain, cheap in document order); a selector whose `AncestorHashes`
(precomputed per rule in `RuleIndex`) are not all in it is skipped without walking ancestors. Hashes
must equal `Atom::precomputed_hash` (`dom::atom_hash`). `El::has_class` splits on HTML ASCII whitespace
only, byte-wise (`dom::class_list_contains`). Outside a build (no index registered) every rule is
tested with no filter.

## QY53f1c: a style rule's declaration block is parsed on first use (src/css.rs)

`StyleRule` holds the selectors, `media`, `layer`, and the declaration block as its source text
(`TopLevelParser::parse_block` only tokenizes it, to find its end and to note whether it may declare
`content` / `counter-*` / `quotes`). `StyleRule::decls()` / `important()` parse it on the first call
(`parse_declarations_split`) into a boxed `Declarations` (~2 KB; the `!important` half is `None`, read as
an empty set, when the block has none) and drop the text. Only rules whose selector matched something are
ever parsed; `precompute_generated` asks `may_declare_generated()` first so it does not parse every block.
`SCRATCHY_CSS_DEBUG` / the test capture parse eagerly (they report ignored declarations at parse time).
`size_of::<StyleRule>()` is 72 bytes (was 4040: two inline `Declarations`); `heap_bytes()` feeds the
`rules_cache` / `style_cache` memory estimates.

## QY53i2: custom-property tables live in a per-thread arena freed on navigation (src/css.rs)

`ComputedStyle::custom_props` / `InheritedStyle::custom_props` are `Option<CustomPropsRef>`: a `Copy`
8-byte handle `{generation, index}` into this thread's `CUSTOM_PROPS_ARENA` (each distinct table stored
once as an `Rc<CustomProps>`, shared by the `tables` Vec and the dedupe `index` map), so the styles stay
`Copy`. Was a `Box::leak`ed `&'static CustomProps` per distinct table plus a second copy as the
interner's map key, never freed. Read with `handle.get("--x")` (a cloned `String`) or `handle.table()`
(an `Rc` clone; use it when the caller may intern meanwhile, the arena is a `RefCell`).
`reset_custom_props_arena()` drops every table and the QY53i memo (keyed by the parent HANDLE now;
equal handles of one generation = equal tables) and starts a new generation; it runs in
`App::replace_document` (every navigation, the help page, the loading/error message page) and at the
start of each `--perf-report` page (`--perf-report` takes several targets = several navigations in one
process). A handle of an older generation (a style that outlived its document, e.g. in the old engine's
GeomCache) reads as empty, never as another table. Generations come from one process-wide atomic, so a
handle read on another thread (the shaping worker gets styled trees but reads no custom properties) also
reads empty instead of aliasing. Within one document the arena only grows (bounded by the distinct
tables; an SPA that keeps one document forever keeps its tables). A cross-build `ComputedStyle` cache
(QY53f3) must be cleared in `reset_custom_props_arena` too.

## QY53i3: animation list sets in a per-thread arena (src/animation.rs)

Same scheme as QY53i2: `AnimRef` handle into `ANIM_ARENA`, one `Rc<AnimLonghands>` per distinct list set, a new generation (and the old tables freed) on `reset_anim_arena()`, which `reset_custom_props_arena()` calls. Any future cross-build style cache must be cleared there too.

## Missing-API report mode (QMAPI1, src/missing_api.rs)

`scratchy --missing-api-report` is `--perf-report` plus a `missing_api` member. In report mode only, `startup_engine` ends by evaluating `missing_api_prelude.js`: ONE `Proxy` (target `Object.prototype`) becomes the prototype of every DOM interface prototype whose parent was `Object.prototype` (EventTarget among them) and of the namespace objects, so a lookup that found nothing reaches its `get`/`has` trap, which records and forwards (answers unchanged). `has` has no receiver, so its interface is `?`. Names are matched against `missing_api_idl.json` (regenerate: `tools/missing-api/gen-idl.py > src/missing_api_idl.json`); non-standard names only count in `ignored`. Not covered: iframe realms. Cost measured with tools/missing-api/cost.sh: at most 1.14x JS time on the gate pages.

QMAPI2 adds a top-level `css` member next to `missing_api` (R64 5.4): `[{"kind","name","count","example"}]`, most hits first. Kinds: `declaration` (a property:value the cascade parser ignored, by property), `pseudo_class` / `pseudo_element` (unsupported pseudo in a stylesheet rule's selector; the rule is dropped; hooks in `OurSelectorParser`, counted only while `TopLevelParser` parses a rule prelude, not for querySelector), `at_rule` (unknown at-rule dropped), `supports_false` (an `@supports` condition that evaluated false, grouped by condition text), `media_feature` (unknown feature in an `@media` list, or one with no colon i.e. boolean/range form; the matchMedia evaluator's unknown names too). QCSS2: `CSS.supports()` exists now and shares the `@supports` evaluator (css.rs `supports_condition_text` / `supports_property_value`; `font-format()` is true for woff/woff2/truetype/opentype/collection, `font-tech()` is always false). Report mode only: `missing_api::enabled()` also makes rule bodies parse eagerly (as `SCRATCHY_CSS_DEBUG` does) so declarations can be checked; results are unchanged. QAPI8: `missing_api::declaration_is_handled` keeps custom properties, `transition*`/`animation*` and initial-value declarations (`height: auto`, `outline: none`) out of the `declaration` rows; extend its table when another property turns out to be handled outside `Declarations`; inline `style=` attributes re-parsed on restyle inflate counts.

## WPT testharness child (QWPT4a, src/wpt_th.rs; R64 2.2, 2.3)

`scratchy --wpt-th-one --socket S URL` runs ONE testharness.js page and is the child process of the
`--wpt-th` runner (QWPT4b). The parent starts its own private wire:

```
wire --socket S --mount http://web-platform.test:8000=CHECKOUT \
     --mount-override /resources/testharnessreport.js=scratchy/tools/wpt/testharnessreport.js
```

and runs one child per test URL (e.g. `http://web-platform.test:8000/dom/nodes/Node-cloneNode.html`,
or `.../x.any.html` / `x.window.html` for the generated wrappers).

**Output contract.** stdout is either exactly ONE line of JSON, or empty:

```
{"harness":"OK","message":null,"tests":[["name","PASS",null],["other","FAIL","assert_equals: expected 2 but got 1"]]}
```

- `harness`: `OK` | `ERROR` | `TIMEOUT` | `PRECONDITION_FAILED` (testharness.js `harness_status.status`).
- `message`: the harness message (string) or `null`.
- `tests`: `[name, status, message]` in testharness order; status `PASS` | `FAIL` | `TIMEOUT` |
  `NOTRUN` | `PRECONDITION_FAILED`; message `null` for a pass. Names are verbatim (the parent escapes
  them for the expected files). An unknown number would appear as `UNKNOWN(n)`.
- Exit status 0 with a line, 1 without (page not loadable, hook never called, stuck, caps hit),
  2 for usage errors. stderr has diagnostics (`scratchy --wpt-th-one: ...`, page `js error` and
  `console.error` lines) and is not part of the contract.
- Empty stdout = the parent records `CRASH` (also when the process died). The 30 s real-time cap
  (R64 2.3) is the parent's job; the child only stops by itself after 200,000 timer callbacks,
  3,600 s of virtual time, or 120 s without an awaited network result.

**How it reports.** `tools/wpt/testharnessreport.js` (scratchy's own file, served by the override)
calls `setup({output: false})` and an `add_completion_callback` that maps the numeric statuses to the
words above and calls the native `__scratchy_wpt_done(json)`. That native is installed only in this
mode (`JsEngine::install_wpt_done` through the `start_scripts_hooked` hook, after the preludes and
before the page's first script); elsewhere the hook file does nothing. The first call wins.

**Virtual clock.** The page runs on a manual `__scratchy_now` clock from 0. `wpt_th::pump` loops:
(1) network: new page requests go into a `NetScheduler`, started jobs run on threads (through the
`NetExecutor`, i.e. wire), finished ones are delivered; (2) timers and deferred promise jobs due at
the current virtual time run; (3) if a request (or a WebSocket) is in flight, wait for it in REAL
time, the clock does not move; (4) only when nothing is runnable and nothing is loading does the
clock jump to the next timer. So testharness's 10 s / 60 s timeout costs milliseconds, a slow fetch
never becomes a false TIMEOUT, and timers due "now" still run while a fetch is pending.

**Page setup.** The page is fetched through wire (2xx only) and decoded with the header charset; its
`<link rel=stylesheet>` sheets (for getComputedStyle), iframes and external scripts are fetched first,
as in `--js-report`. Viewport 800x600 (`innerWidth`/`innerHeight` and `matchMedia`, which now follows
the same viewport as `innerWidth` in every mode). Ahem: `<origin>/fonts/Ahem.ttf` is fetched through
wire and added to the forced-layout font system (`js_geom::install_extra_fonts`); missing = one
stderr line. `visibilityState` is `visible` (always, E14). The pump runs the rendering update on
virtual 16.7 ms frames through the shared driver (QEL1, "Rendering steps"). Not here yet: IO
in the rendering step (QEL3), stylesheets inserted after load. Trusted input: "testdriver (QWPT7)" below.

### testdriver (QWPT7; tools/wpt/testdriver-vendor.js, src/wpt_input.rs, src/input_host.rs)

The runner's wire also overrides `/resources/testdriver-vendor.js` with scratchy's vendor (embedded like the report hook).
It sets `test_driver_internal.in_automation` and implements `click`, `send_keys`, `action_sequence` (pointer
move/down/up, key down/up, wheel `scroll`, pause; element and `pointer` origins resolved at tick time), plus a small
`get_computed_role` / `get_computed_label` (tag/role table, aria-label(ledby), labels, alt, text). `bless` works through
`click`. Each call queues a job of TICKS and calls the native `__scratchy_td_wake` (installed only in `--wpt-th-one`,
`JsEngine::install_testdriver`). `wpt_th::pump` runs ONE tick per idle turn (after network, due timers and the rendering
frame) by evaluating `__scratchy_td_next()`: ops one per line (`m X Y`, `d B`, `u B`, `kd CPS`, `ku CPS`, `w X Y DX DY`,
`f ID`, `e ID`, `t MS`). The virtual clock then moves by the tick's duration, at least 16 ms (`MIN_TICK_MS`: a page
that sends keys in a loop until something happens still reaches its harness timeout). A job's promise resolves in the
`__scratchy_td_next` call after its last tick; a tick that throws (origin out of view) rejects it.

`wpt_input::Driver` runs the ops: the element under the pointer is `GeomCache::hit` (below), events are trusted
(`JsEngine::dispatch_event`), and the default actions are the App's own, shared through the `input_host::InputHost`
trait (the App implements it too): `click_activate` (checkbox/radio legacy pre-activation, reset/submit buttons,
select), `forward_label_click`, `implicit_submit`, `focus_events` (blur/focusout/focus/focusin with `relatedTarget`),
`forms_text::text_control_key` (keypress, beforeinput, edit, input; Enter, Backspace, arrows, ...), and the init field
builders `mouse_init`/`pointer_init`/`key_init` (`composed` set). Order of a press: `pointerdown` (first button;
cancelled = no compatibility mouse events until all buttons are up), `mousedown` (`detail` = click count), `:active`,
focus of the nearest click-focusable ancestor (App `press_state`; a press on nothing focusable blurs), `contextmenu`
(right button). Release: `pointerup`, `mouseup`, then `click` (main button, with activation and label forwarding) or
`auxclick` at the nearest common ancestor of the press and release targets; `dblclick` for a second click within
500 virtual ms. Moves fire out/leave then over/enter (pointer, then mouse) when the hit element changes, then
`pointermove`/`mousemove`; `:hover` follows (`js_dom::interaction_changed`: the next forced layout restyles). Keys:
WebDriver code points (U+E000 block: `webdriver_key`, left/right modifiers and numpad through the physical code),
modifiers held across ticks, `keydown` at the focused element (else body), then text editing, Tab focus order, Space on
a checkbox/radio, Enter/Space on a button, Enter on a link, else `keypress` for printable keys. `send_keys` focuses
the element first and puts a text control's caret at the end. `wheel`: a cancelable `WheelEvent` at the hit element,
then `__scratchy_td_wheel` scrolls the nearest scroll container (else the window). Page `focus()`/`blur()` requests
(QB5 `FocusReq`) are applied by the pump as `js_sync` applies them in the App, silently (QWPT7a: `set_focus_ev` /
`App::set_focus_element_ev` with `events` false): `HTMLElement.prototype.focus/blur` (js_dom.rs) fire blur/focusout then
focus/focusin themselves, synchronously, through `__scratchy_dn` (relatedTarget set; nothing fires when focus does not
change, the element is unfocusable or not connected). The native `__d_focus` updates the page-visible active element and
mirrors `:focus` into the style thread-local at once (`mirror_focus`), returns the previous element, and queues the request
the App/driver catches its own state up with.
QWPT7c: hit testing honours `pointer-events: none` (`ComputedStyle::pointer_events_none`, inherited; in `paint::hit_test_element`, so
the App and the driver share it), element scroll offsets (the script-side `scrollTop` map goes to `__d_hit`; `js_geom::regions_by_node`
maps node ids to region ids) and inline-svg children (`svg_geom.rs`, `js_geom::refine_svg_hit`, also the rect of an svg child).
`offsetX/Y` are computed on read for pointing-device events. Accesskey is Alt+Shift+key (`input_host::accesskey_activate`).
Not headless: navigation (link clicks, form submission fire their events only), the select dropdown, drag and drop, touch/pen pointers (reported as mouse), window rects.

**Hit testing (`document.elementFromPoint`, `elementsFromPoint`, ShadowRoot versions).** Native `__d_hit(x, y, vw, vh,
sx, sy)` -> `GeomCache::hit`: refreshes the forced layout, then `paint::hit_test_element` (the App pointer's walk:
z-order, positioned and fixed boxes, sticky) on the incremental engine's kept laid tree in document space (viewport
point + window scroll); without an engine tree, the deepest recorded box containing the point. The laid tree's hit is
a box; `refine_inline` then picks the inline element whose glyph is under the point (a link or span in a line).
JS: non-finite coordinates throw (WebIDL double), outside the viewport -> null / [], no hit -> the root element;
shadow hits are retargeted against the scope; `elementsFromPoint` = the hit and its ancestors whose border box
contains the point, then the root (no siblings painted underneath). No `pointer-events: none`, no SVG content
hits. Since QWPT7 an inline element with no box of its own and no boxed descendants has the union of its glyph
boxes as its rect (getBoundingClientRect/getClientRects/offset*), and one with only collapsible white space or nothing
sits at its pen position (zero width, the next visible glyph in its block, else the end of the previous).

## WPT testharness runner (QWPT4b, src/wpt_th_run.rs; R64 2.3, 2.5, 3.1)

`scratchy --wpt-th <checkout> [--jobs N] [--update] [--area A] [--wire PATH] [--dir wpt-th] [--timeout SECS] [--no-scope] [-v]`,
run from `scratchy/`. Reads `wpt-th/subset.txt` (paths relative to the checkout, one per line; `# area: NAME` comment lines
start an area, e.g. `# area: dom/nodes`), `wpt-th/flaky.txt` (first token of a line = path, rest is the reason) and
`wpt-th/expected/<area>.txt` (`/` in the area name becomes `-`); writes `wpt-th/areas.txt` and prints it on stdout.

- **Wire:** the runner starts its OWN `wire --socket <tmp>/wire.sock --no-cookies --mount http://web-platform.test:8000=CHECKOUT
  --mount-override /resources/testharnessreport.js=<tmp>/testharnessreport.js` (the vendor hook is embedded in the binary and
  written to the temp dir). Never `/run/user/1000/wire.sock`; on exit only that child PID is killed. Wire binary: `--wire PATH`,
  else `$SCRATCHY_WIRE`, else `wire` next to the scratchy executable, else `../wire/target/release/wire`, else `wire` on `$PATH`.
- **Children:** one `--wpt-th-one` per file, `--jobs` threads (default: CPUs). 30 s real-time cap (`--timeout`): the runner
  kills that child's own PID and records `TIMEOUT` (0/0); no output = `CRASH`. `*.worker.*`, `*.serviceworker.*`,
  `*.sharedworker.*` are not run: `NOT-RUN` (recorded, compared, not counted in the score).
- **Memory cap:** the runner re-execs itself once under `systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0`
  (`SCRATCHY_WPT_TH_MEM` overrides, `--no-scope` skips; no usable systemd-run = one stderr line and no cap). One scope for runner,
  wire and all children (a scope per child costs a D-Bus round trip per test). A runaway child is OOM-killed inside the cgroup and
  recorded `CRASH`. Core dumps are disabled for the whole tree.
- **Gate (exit 1):** REGRESSION (file status got worse; a subtest that was passing fails), UNRECORDED (file not in the
  expected file), CHANGED (subtest count differs, a non-pass changed kind, harness status changed sideways). NEWLY-PASSING
  is printed and does not block; `--update` records everything. Flaky files run and are printed, but are in neither the
  expected files nor the areas table.
- **areas.txt:** columns files, ran (got a result), all-pass, subtests, pass, pass%, score (mean per-file passed/total over ran
  files, Q7 headline). With `--area` only that area's row is replaced. The R64 "scratchy <commit>" header field is left out
  (it would change the committed file on every commit).
- **`--areas-only` (QWPT10):** `scratchy --wpt-th --areas-only [--dir wpt-th]` recomputes `areas.txt` from `subset.txt` +
  `expected/*.txt` + `flaky.txt` with no test run (no checkout, no wire, no memory scope) and prints it. Same rows as a full run:
  `files` counts every subset entry (flaky included), the rest counts non-flaky entries that have an expected record. Use it
  after resolving a merge: `areas.txt` is derived, never hand-merged. `coordinator/merge_pr.sh` does this when a rebase conflicts
  ONLY in `scratchy/wpt-th/areas.txt` (plus the keep-both files): it takes either side, runs `--areas-only` with
  `target/release/scratchy` of the temp worktree (built there with `nice -n 19 ionice -c3 cargo build --release` when missing,
  RUSTY_V8_ARCHIVE when present; else `~/.cargo/bin/scratchy` only if it supports `--areas-only`), `git add`s it and continues.

### Phase-1 subset and baseline (QWPT6; tools/wpt-subset.py, wpt-th/)

`wpt-th/subset.txt` is GENERATED: `python3 tools/wpt-subset.py CHECKOUT` (run from `scratchy/`). It needs WPT's own manifest code,
which the sparse checkout lacks: `git -C CHECKOUT sparse-checkout add tools` once (~12 MB). The script imports `tools/manifest` from the
checkout and builds `CHECKOUT/MANIFEST.json` in-process (no `./wpt` launcher, no venv, no pip, no sockets; ~1 min). It keeps every
testharness URL of the areas in the script, minus timeout=long outside dom/* (QWPT6c keeps the dom/* ones) and `encoding/legacy-mb-*` (nightly), testdriver users, `.https.`/`.sub.`,
worker variants, pages whose source CALLS the driver (`test_driver`, QWPT6e) and files whose source mentions a `.py` handler; skip counts go to stderr. Result at the pin: 3,254 URLs (3,365 before QWPT6e) in 24 areas incl. html/editing (3,312 before QWPT6c, which added 53 dom/* timeout=long URLs: dom/nodes 13, dom/events 7, dom/ranges 28, dom/other 5)
(`# area:` blocks in the script's AREAS order). Query variants (`?1-10`) are separate entries.
Baseline: 3,214 ran, 656 all-pass, 37,045 subtests, 45.6 % pass, mean per-file score 28.4 (table in `wpt-th/areas.txt`).
Wall time of the full run: 110 s at `--jobs 4` (quiet machine, load 5), 67 s at `--jobs 12`; first run 3:41 on a loaded machine
(load 30). Peak RSS of the runner tree ~290 MB. Four runs were identical, so `wpt-th/flaky.txt` is empty (comment only).
html/canvas (QWPT12): 2,354 URLs in three areas (`html/canvas/element` 1,276, `html/canvas/offscreen` 1,074, `html/canvas/other` 4; `.tentative.` included; `html/canvas` is in tools/wpt-dirs.txt). Skipped there: worker variants 1,040, `.https.`/`.sub.` 77, test_driver calls 5, testdriver 1, timeout=long 2. Baseline when added: 40 of 3,021 element subtests pass (1.3 %), offscreen 0 of 1,370 (`getContext` returns null, QY3); five runs of the three areas were identical, no flaky entries. Local time of the canvas files: ~50-85 s per big area at `--jobs 4`. The reftest gate is unchanged (canvas reftests draw with script).
html/editing (QWPT6d): 48 files; the filter also drops pages whose source mentions `testdriver` (`SRC_TESTDRIVER_AREAS` in wpt-subset.py; only this area).
QWPT6e (testdriver): `--wpt-th-one` has no trusted-input path (no App; `document.elementFromPoint` is a stub; testdriver-vendor.js throws `action_sequence() is not implemented`), so a vendor for click/send_keys/Actions would need hit testing, focus and default actions: not an S item. Instead `DRIVER_CALL` (`\btest_driver\b` in the source) drops every page that calls the driver, in all areas: 111 URLs (by area in PROMPTS.md QWPT6e). 138 pages only load testdriver.js and never call it; they stay (they run fine). Baseline now 3,209 ran, 969 all-pass, 85,795 subtests, 67.1 % pass, score 39.1. 25 s at `--jobs 4` locally.
QWPT7 (testdriver vendor): `DRIVER_CALL` now only drops a page that calls a `test_driver.<name>` outside `DRIVER_DONE` (click, send_keys,
action_sequence, Actions, bless, get_computed_label/role, set_test_context, message_test): 1 URL at the pin (get/set_window_rect). 111 URLs came back.
QWPT6c: the 53 dom/* timeout=long URLs take ~20 s at `--jobs 4` on their own (load ~25). Five runs: two files have run-dependent subtest counts (they hit the 10 s virtual harness clock partway) and are in `flaky.txt`: dom/ranges/Range-mutations-dataChange.html and Range-mutations-replaceData.html.

### Range fix-ups across realms, dynamic iframes (QDOM5b)

`xr.rl` (page-wide, shared by every realm) lists WeakRefs of the Range STATES (`{sc,so,ec,eo}`); the Range block's native wrappers (`__d_insert`, `__d_remove`, `__d_settext`, `__d_sethtml`, `__d_write`) all read it (`replaceChild` removes then inserts since QDOM18, so its fix-ups come in the spec's order for free). A wrapper keeps the natives of the realm that created it, so a per-realm list missed ranges of the other realm. Nothing runs when the list is empty. `ifrFetch(el)` / `ifrPage(el)` (iframe block): for a script-set or script-inserted iframe whose same-origin http(s) `src` is not prefetched, one ASYNC XHR per URL (`dynBusy` in flight, `dynPages` url -> [url, html] or [] when failed/non-HTML/> 1 MiB). While it runs (`ifrPending`) the iframe shows its initial about:blank window; on arrival the SAME window is navigated in place (QDOM5c: `loadFrame` calls the realm's `__scratchy_realm_nav(url)`: location parts, document URL, no about:blank base; only for a `src` present at insertion: a `src` set later drops the window as before, because browsers give the old Document back as non-active with URL about:blank and the Document object is reused here) and the page loads into it (`loadFrameLater`: scripts, `load`); a failure just fires `load`. QBLOBFRAME: a same-origin `blob:` src (`blob:<page origin>/<uuid>`) goes through the same path: `ifrKey` accepts it (the fragment dropped), `ifrFetch` reads the blob store with `__blob_lookup` (no XHR) and `blob.text()`; a URL revoked before insertion (browsers resolve the blob entry when navigation starts), an unknown one or a non-`text/html` type fires `load` with the about:blank window kept. Other-origin blob URLs are not loaded. Parsed iframes that were not prefetched are untouched. `contentWindow` and `window.navigator` keep their identity across the load (browsers reuse the initial window); the Document object is reused too (a browser makes a new one). The iframe's `load` reported while an iframe `load` handler of the same realm is running (its handler called document.open()/close()) goes to a task: such a handler recursed on the stack (the real cause of per-global.window.html's flaky ERROR: a stack overflow in testharness, then 'Timeout while running cleanup'). dom/ranges Range-insertNode/surroundContents run their 1840 cases in one callback and hit the 500 ms callback budget at a varying point: flaky.txt.

### XML documents (QDOM11)

A response whose MIME type is XML (`js_dom::is_xml_mime`: `application/xhtml+xml`, `image/svg+xml`, `text/xml`, `application/xml`, any `+xml`) is parsed by roxmltree, not html5ever: the `--wpt-th-one` top level (`wpt_th::run_one` -> `parse_xml_document`; `js::set_page_xml_mime` -> `run_page` evals `__scratchy_xml_doc(mime)` before the scripts) and iframes (`net::prefetch_iframes` stores `iframe_page_mime`; the dynamic `ifrFetch` (XHR and blob) puts the MIME as third element of its `[url, text, mime]` page; `frameOf`/`frameLoad` call native `__d_load_xml_page` and `__scratchy_xml_doc` in the frame realm). `__scratchy_xml_doc` makes the realm's document an XMLDocument (prototype, `contentType`, QDOM17 non-HTML flag via `nonHtmlDocs`, case-keeping `createElement`). `load_xml_into`: roxmltree tree (entities from an internal DTD work), a Doctype node from a hand scan of the prolog (roxmltree has none; comments/PIs of the internal subset are dropped from the children, the subset itself is not kept), and on a well-formedness error the Firefox-style `parsererror` document. Scripts: `js::script_namespace` accepts only XHTML and SVG `script` elements (also prefixed), SVG ones read `href`/`xlink:href`; CDATA arrives as ordinary text. Not done: the App path (`dispatch_wire_response` still rejects XML top-level types, so nothing changes on screen), `document.write` on an XML document, internal-subset serialisation.

### wptserve server (QWPT8; tools/wptserve.sh, wire --map-host, `--server wptserve`)

Phase 2 of R64 (DECISIONS Q2): WPT's own Python server for pages that need `.py` handlers. `tools/wptserve.sh start CHECKOUT [--port N] [--wire PATH]` runs `serve.main()` from the checkout (needs `tools/` in the sparse checkout: wpt-dirs.txt has it) with its bundled `third_party` libraries on sys.path through `tools/localpaths.py`: no `./wpt` launcher, no virtualenv, no pip, Python 3 of the system. Only HTTP on 127.0.0.1, ports P and P+1 (the mount's convention) chosen from a hash of the checkout path (18000-27999) and probed free; no check_subdomains, no https/h2/ws/dns (QWPT9). It starts its own wire (`--no-cookies --no-cache --map-host '*.web-platform.test=127.0.0.1' --mount-override /resources/testharnessreport.js=tools/wpt/testharnessreport.js`, socket in a `mktemp -d /tmp/wptserve-wire.*` dir) and keeps state in `CHECKOUT/.wptserve/` (pid, port, wire.pid, wire.sock.path, logs). `--stop CHECKOUT` sends SIGINT to the recorded server PID (SIGTERM would leave its spawned http processes behind), then TERM/KILL to its process group (it runs in its own session) and kills the recorded wire PID after checking its command line. The runner does NOT start them: it runs under QNET1's seccomp filter (no TCP socket in it or in any child), so `scratchy --wpt-th CHECKOUT --server wptserve` only reads the state and uses the script's wire socket. `--server wptserve` runs exactly the areas in `WPTSERVE_AREAS` (wpt_th_run.rs, same list as wpt-subset.py: fetch/api, fetch/other, xhr), the default run skips them, and naming one for the wrong server is exit 2. Subset: `.py` handler pages stay in for these areas, `.h2.` out; `.https.`/`.sub.`, worker variants, testdriver and timeout=long stay out as everywhere. CI: shard `scratchy-wpt-wptserve` starts the script before the runner step and stops it in an always() step; `rest` excludes the areas (9 required checks now).

### Window named access (QDOM1)

`WindowProperties` (js.rs, next to `Object.setPrototypeOf(g, Window.prototype)`) is a Proxy (get/has traps only) that is the prototype of `Window.prototype`; its target inherits `EventTarget.prototype`. A name that EventTarget.prototype/Object.prototype defines is never shadowed; own globals and `var`s win because they sit on the global itself; nothing is enumerable. The DOM prelude gives it a resolver (`__scratchy_set_named`): an index -> `winAt(i)` (child window), else native `__d_named(doc, name)` (first element in tree order whose id is `name`, or an embed/form/img/object/iframe with that `name`; one map per DOM write generation `wgen`, so `typeof X` probes are a hash lookup), an iframe resolves to its window (by its current window name), others to the element wrapper. Known V8 deviation: a strict-mode assignment to an undeclared name no longer throws ReferenceError (a Proxy in the global's chain counts as "found"); reads of undeclared names still throw.

### Script load failures (QSCR404)

A `<script src>` whose fetch fails (network error or a non-2xx status; `fetch_script_via_wire` checks `WireFetchRaw::Responded.status`) is never run. Parser-inserted: `fetch_scripts_with` makes a `ScriptKind::ClassicError` / `ModuleError` source and `run_page` fires `error` on the element (`__scratchy_script_error`) at its place in the order, no `load`. A static import the prefetch could not fetch is a `ScriptKind::ModuleMissing` source; a module whose static graph reaches one (`js_modules::static_graph_missing`) gets `error` and does not run. Literal `import()` targets are prefetched best-effort; a missing one only makes that `import()` reject. Script-inserted scripts (js_dom.rs `sFetch`/`sModuleGraph`) check the status in JS (QY34) and fire `error` the same way.


### Box alignment (QCSS-C)

css.rs `JustifyContent`/`AlignContent`/`AlignItems` store the specified keyword (getComputedStyle serialises it, `keyword()`); layout reads only `canon()`. `AlignItems` is also the type of `justify-items`/`justify-self` (`Left`/`Right`/`Legacy*` are justify-only; `Auto` exists only in `Declarations` and is turned into `None` by `resolve_style`). Parsing: `align_words` collects the identifiers, `parse_align_prefix`/`parse_content_prefix` read one value and report how many words it used (`place-*` split the words with `split_place`). `measure_grid`: `justify_self.unwrap_or(justify_items).canon()` sizes an auto-width item to fit-content when not stretch and the post-row pass offsets it (x in the column span, y in the row); there is no vertical stretch. `grid-auto-flow` (`GridAutoFlow {column, dense}`): column flow fills `explicit_rows.max(1)` rows then appends implicit auto columns after placement; dense resets the cursor per item. No new style_diff class: all new fields are Layout (`incr_diff.rs`).

### Box and text properties (QCSS-E)

- **Radii:** `ComputedStyle::border_radius` is a `CornerRadii` (css.rs): four `(horizontal, vertical)` `BoxLength` pairs (TL, TR, BR, BL), filled from the four `Declarations::border_radius` longhand slots; the `border-radius` shorthand (1-4 values, `/` for the vertical radii) writes all four. `%` is resolved at paint against the border box width (horizontal) and height (vertical) by `CornerRadii::resolve(w, h)`, which applies the overlap scaling of CSS Backgrounds 3 section 5.5 (`fit_radii`; all-circular-and-equal radii keep the old exact `min(r, w/2, h/2)` clamp, so uniform boxes paint pixel-identically). paint.rs paints `Radii` (`[(f32, f32); 4]`) through `rrect_insets`/`ellipse_inset` (fills, the solid ring, backgrounds images, inset/outer shadows via `in_rrect`) and `in_rounded_rect` (per-side borders). Transitions/animations interpolate each corner.
- **outline-offset:** `outline_offset` (px, not inherited, may be negative); the ring sits `offset` outside the border box. It is a Paint class in `style_diff`. `ComputedStyle::outline_reach()` (`outline_width + max(offset, 0)`) is how far the outline paints past the border box: the damage repaint's `paint_reach` (QHOV1) must use it (and `CULL_MARGIN` if an offset can exceed it).
- **transform-origin:** parsed and computed (`(TranslateAxis, TranslateAxis)`, initial `50% 50%`, getComputedStyle prints the specified form: no layout there), Paint class. The only transform painted is the summed translate, which does not depend on the origin, so nothing reads it yet; `transform_origin_px(w, h)` is for a future scale/rotate. `calc()` with a `%` is resolved against the font size like `translate()` does.
- **border-spacing:** inherited `(h, v)` px; `measure_table` puts `h` before, between and after the columns and `v` the same for rows (not under `border-collapse`), row boxes start one `h` in and span the cells, `settle_rowspans` adds the row gaps, a nested table's natural width adds the column gaps. The UA stylesheet has no `table { border-spacing: 2px }` (it would shift every table); `cellspacing` is not mapped. Layout class.
- **letter-spacing:** inherited px, `SpanStyle::letter_spacing` (hashed into the shape memo key), passed to cosmic-text as `Attrs::letter_spacing(px / font_size)` (cosmic-text takes EM: the Q26 "bug" was px passed as em). cosmic-text also adds the spacing after the last glyph of a line (upstream issue 542), which shifts centred/right-aligned text by one spacing. Layout class.
- **direction:** inherited `Direction`; the UA sheet maps `[dir=ltr|rtl i]`. It affects only inline content: `TextAlign` got `Start`/`End` (initial `start`, kept as declared in the computed value, `TextAlign::physical(dir)` resolves them in layout; `to_cosmic(dir)` passes an explicit `Left` only in an rtl element). The bidi paragraph level still comes from the first strong character (cosmic-text has no way to force it, and a prefixed RLM would shift every glyph offset that search/Range read), blocks, tables, flex rows and margins are not mirrored. Layout class.

### Iframe unload on navigation (QIFRNAV, QDOM5d)

Setting `src`/`srcdoc` of an iframe whose realm finished loading (`f.loaded`; the initial about:blank has nothing to unload) does not drop the window at once: `ifrAttr` queues an unload task and then a navigation task (two timers, so a handler killed by the callback budget does not stop the navigation; the old window stays `contentWindow` until the second task). The unload task calls `unloadFrame` -> the realm's `__scratchy_unload_phase('beforeunload')` then `('unload')`: beforeunload (BeforeUnloadEvent, not cancelable by UI), then pagehide (persisted false), visibilitychange at the document (visibilityState/hidden flip) and unload; each phase reaches the nested windows after the window's own events, with `ignoreOpens` (the spec's ignore-opens-during-unload counter, which `document.open()` checks) raised for the whole phase. Not fired: on iframe removal (the spec forbids synchronous script there), on realm teardown with the page, by document.open(). `document.open()` on an XML document throws InvalidStateError and erases the Window's and the Document's listeners/handlers (`__scratchy_erase_listeners`): url-fragment.window.html's load-dependent CRASH/0/2 was a stack overflow from a window `load` handler that calls open()/close() re-running itself. Not done (QDOM5d): in-place navigation making a NEW Document inside the kept Window (the DOM prelude's `doc` is a closure variable bound once by `__d_bind_doc`; a swap must rebind `cache`, `xr.byDoc`, `xr.prep`, `myDocs`, the custom-element registry, `frames[..].doc` and `g.document`).

### CI gate (QWPT5)

QCISPLIT2: the testharness gate is sharded by area into five matrix jobs: `scratchy-wpt-dom` (`--area 'dom/*'`), `scratchy-wpt-html` (`--area html/dom,html/editing,html/semantics/*,html/webappapis,custom-elements`), `scratchy-wpt-canvas-element` (`--area html/canvas/element`), `scratchy-wpt-canvas-other` (`--area 'html/canvas/*' --except html/canvas/element`, so new canvas areas are gated) and `scratchy-wpt-rest` (`--except` of all those lists, so any other new area is gated; it also runs the reftest gate). Each shard is ~200 s of summed file time. Required checks are now scratchy, snag, wire and those five (8; `scratchy-wpt` is gone). `--area`/`--except` take comma lists of exact areas or `prefix/*` globs and combine (area minus except); an exact name that matches no area is an error. A shard rewrites only its own areas' rows of `areas.txt` (the other rows stay as committed), so `areas.txt` is still the one file for all areas and the "CI result differs from committed" check is per shard. Why the CI step is slow: not one file (11 of 3254 took >= 2 s, 58 s of ~594 s summed, before canvas); it is ~0.18 s per file (child process + V8 start) divided by 4 jobs. Every run prints `SLOW` (10 slowest files) and `AREA-TIME` lines. A new top-level area that grows large should get its own shard.

QCISPLIT: the WPT gates run in their own CI job `scratchy-wpt` (parallel to `scratchy`, own release build of scratchy + wire); required checks are scratchy, snag, wire, scratchy-wpt. It runs `--wpt-th "$RUNNER_TEMP/wpt" --jobs 4 --no-scope --wire ../wire/target/release/wire` after the reftest step (no display needed; `--no-scope` because the runner has no systemd user bus). Regressions/unrecorded/changed files fail the job (DECISIONS Q3). Step summary: areas table with deltas vs origin/main's `wpt-th/areas.txt` (`tools/wpt-areas-diff.py NEW BASE`). A CI result that differs from the committed baseline is never re-recorded silently: timing-sensitive files go to `wpt-th/flaky.txt` with a reason, anything else is a question to the user.

### Scroll, scrollend, wheel, MediaQueryList change (QEL7)

Scroll events are QUEUED in JS (js_dom.rs `gScrollNote`: window = the Document as target, elements = the element; a Set
coalesces to one per target per update) and FIRED by rendering step 2 (`JsEngine::run_scroll_steps` ->
`__scratchy_scroll_steps(busy)`): `scroll` at each queued target (the Document's bubbles to the Window, an element's does not),
trusted. Sources: `scrollTo/By`, `scrollTop/Left`, `Element.scroll*` (queued when the offset changed), and the App mirror
(`JsEngine::set_scroll`, run at every `about_to_wait`/dispatch: a changed window offset queues the Document, `__scratchy_elscroll`
diffs the element offsets the wheel/scrollbar/keyboard changed and resolves nid -> element with `__d_by_nid`). `scrollend` fires for
a target in the first update where it did not scroll (the App passes `busy` = the viewport is easing, `scroll_target`, or a
thumb is held). A scroll made inside a rAF callback is reported by the next update. Rust learns what is owed without an eval from
the thread-local `js_dom::QEL7_BITS` (native `__d_qel7`: 1 scroll queued, 2 scrollend waiting, 4 a MediaQueryList is watched;
`JsEngine::qel7_bits`). App: `scroll_step_due` triggers an update (at most one per 16 ms, `SCROLL_STEP_GAP`; `scroll_wake` is
the wake-up); headless: `headless_anim_due` adds the next frame boundary after the first notice (`HeadlessAnim::scroll_since`).
QEL7b: iframe realms have no rendering update of their own; each reports its bits up (`__scratchy_q7_kid`) and the top realm's steps run the kids' scroll steps and `__scratchy_mql_run` (a frame's window/element scroll and MQL change are rendering-step events too; only the top realm writes `__d_qel7`). `scrollTo/By` clamp y to the document height in the top realm before reporting (frames have no layout: unclamped). `behavior: smooth` / `scroll-behavior: smooth` (window scroll* and scrollIntoView's window part) goes to the App as `ScrollReq.smooth` and reuses the QSMOOTH1 easing; `SMOOTH_OK` (set by the App when easing is on) keeps headless, frames and tau 0 instant; element containers and x are instant. `visualViewport` never fires `scroll` (offsets are 0). No scroll-snap in scratchy.
`wheel`: `App::dispatch_wheel` (MouseWheel handler, after the select popup) sends a trusted cancelable `WheelEvent` (pixel
deltas, `deltaY` > 0 = down, deltaMode 0) at the element under the pointer when a page has a wheel listener
(`pointer_listener_mask` bit 8); `preventDefault` skips the whole tick (window/document/body listeners are passive by default
since QDOM4). `onscrollend` handler names exist since QEL7.
MediaQueryList: lists get `change` (a `MediaQueryListEvent`, then `onchange`) from step 3 (`__scratchy_mql_run`) when the window
size changed and the result flipped; only lists with a `change` listener / `onchange` are watched (kept alive by the engine),
so `matchMedia(q)` in a loop costs nothing extra.

### Event handler IDL attributes (QDOM13)

js.rs `getH`/`setH`: one slot per (target, event type): `v` (IDL value: function or null; undefined = the content attribute applies), `asrc` (attribute text when `v` was set; a later change wins), `entry` (the listener-list entry registered when a non-null value was first set, so handlers keep registration order; a null set removes it). A content attribute never overridden is compiled lazily (cached per element) and runs after the listeners. `isFwdEl`: `<body>`/`<frameset>` forward `onblur/onerror/onfocus/onload/onresize/onscroll` and the Window-only names to the Window; the Window's attribute handler comes from the last body/frameset whose attribute was set (`__scratchy_attr_changed`, called by `setAttribute`/`removeAttribute`) or `document.body`. Accessor tables (`GLOBAL_H`, `WIN_ONLY_H`, ...) sit after the `Window` interface in js.rs; js_dom.rs installs them on `HTMLBodyElement`/`HTMLFrameSetElement`/`SVGElement` through `__scratchy_install_handlers`. No `ontouch*` and no `onscrollend*` names on purpose (see PROMPTS.md QDOM13). Trusted animation/transition events fall back to the `webkit*` listeners per node (`LEGACY` in `dispatch`). Known gap: CSS animation/transition events never fire in the `--wpt-th-one` runner, so dom/events/webkit-*-event ends TIMEOUT after 4 passing subtests each.

## Rendering steps (QEL1, src/event_loop.rs; R64 section 4 E1-E3)

`event_loop::update_the_rendering(host, ts)` is HTML "update the rendering", ONE function for the App
and every headless mode. Order: (1) resize events (2) scroll events (QEL7) (3) MediaQueryList
`change` (QEL7) (4) CSS animation events (5) the animation frame callbacks (6) style + layout
(7) ResizeObserver / IntersectionObserver hooks (8) paint. The host supplies the steps it has through
the `RenderingHost` trait; the defaults are headless (no resize, no animations, observers = RO then IO
evaluation, no paint).

**rAF list (TIMERS_PRELUDE).** `requestAnimationFrame` no longer uses the timer table: each realm
numbers its own ids from 1 (`clearTimeout(rafId)` and `cancelAnimationFrame(timerId)` do not cross),
the callbacks wait in one list (iframe realms forward to their parent's, keyed by realm, as timers
do; a dropped realm's callbacks are removed). `__scratchy_raf_due()` = the 60 Hz boundary of the page
clock strictly after the moment the list became non-empty (-1 when empty). `JsEngine::run_animation_frames(ts)`
takes the list as it is (`__scratchy_raf_begin`), then runs `__scratchy_raf_run_one` until it says
false: every callback gets the same `ts`, its own CPU budget and a microtask checkpoint after it; the
batch is never split by `timer_slice` or the 100-per-turn timer cap; a callback cancelled earlier in
the batch is skipped; callbacks requested during the batch wait for the next frame. An exception is
"reported" (ErrorEvent at the callback's own global, `onerror`; the log line only when nobody
cancelled it). Timer callbacks still only log (QEL4).

**App.** `about_to_wait`: pointer flush, scroll mirror, due timers (sliced, as before), `js_sync`, then
`App::rendering_update_if_due`: the update runs when a rAF's frame time has come, a `resize` is owed
(`viewport_changed`), or an animation event/change is due (`anim_wake`, set from `anim_tick`'s answer;
`-inf` after every build). The frame timestamp is the page clock at that moment. Step 5 runs only in an
update whose rAF frame is due: an update for a `resize` or a CSS animation event in between frames
skips it (running it there doubled a rAF loop's rate to ~120 Hz while a CSS animation ran). Step 6 is `js_sync`
(the frame's DOM writes become `layout_dirty` / an incremental restyle) and step 8 requests the redraw;
the layout itself and the paint stay in `redraw` on the RedrawRequested that follows (incremental
engine, shaping worker, progressive shaping, no forced layout per frame: the cost tricks are
unchanged; painting a partly shaped layout is E15, deliberate). The wake-up is the earliest of the next
timer, the next rAF frame and the next animation change.

**Headless.** `event_loop::headless_next_due` (earliest timer or rAF frame) and `headless_turn` (due
timers first; when none ran, the rendering update if its frame is due) drive `wpt_th::pump`, the
`--js-report --run-timers` pump and the test helper `run_with_timers`; `--perf-report` uses
`headless_frame` on its real-time loop. A page without rAF has no frames at all (no extra cost).

**Headless CSS animations (QANIMHL).** Before QANIMHL the headless host had no animation step at all:
the document clock (`anim_eval::DOC_CLOCK`) stayed 0, nothing fed an `AnimTracker`, so no
`animation*` event ever fired in `--wpt-th-one` (the App path did fire them, from its builds). Also an
EMPTY `@keyframes x {}` (the dom/events webkit-* tests) was skipped by `evaluate` and `is_animated`
in every mode, App included: it now runs (start/end/iteration/cancel events) and changes no value.
Now `event_loop::Headless { js, now }` keeps a `HeadlessAnim` in the `JsEngine`: step 6 (style),
when the page's rules define `@keyframes` (`JsEngine::page_has_keyframes`, checked once per geometry
generation) and the geometry generation moved since the last style step (or animations are live), sets
the document clock to the page clock and runs the page's forced layout (`__d_anim_pass` ->
`GeomCache::style_pass`, the same cache a geometry read uses, so it is a no-op when a script already
read geometry); the report of the pass (`anim_eval::animation_passes` tells a new pass from none)
goes to the tracker. Step 4 of the NEXT update ticks the tracker at the page clock and dispatches the
events (trusted, bubbling) before the rAF batch, so a rAF requested by an `animationend` handler runs
in that same update. `headless_frame_due` adds the animation due time (frame boundary after the
DOM change or after the tracker's next change). A page without `@keyframes`, or one whose DOM does
not change and has no animation event coming, owes no frame. `getComputedStyle`'s cache keys animated
entries on the clock as well as the write generation. Under `--perf-report` the step's layout counts
as `io` (rendering-update work, like the IntersectionObserver's), caller `(rendering update: CSS
animations)`; on GitHub that is one first layout (~480 ms) a script never asked for. Not here: CSS transitions (since QANIMHLb,
below), WAAPI
(`el.animate`). Geometry reads (QANIMHLc): `GeomCache::anim_clock` is the document clock of a layout whose
style pass styled a live animation (not yet ended) or transition (`anim_eval::layout_animated`); such a
layout is fresh only at `anim_eval::read_clock_ms` (the headless task clock, else the document clock), and a
relayout then runs at the read clock. A page with nothing running keeps the plain generation key.

**CSS transitions (QANIMHLb, src/transition.rs).** `transition` + the five longhands (`-webkit-`
aliased) parse into `Declarations::trans` and cascade into an interned `TransRef` (`ComputedStyle::
transitions`, arena reset with the animation arena). Engine: `transition::apply` runs at the end of
every `resolve_style_for` (one flag test when the element declares none and no transition exists).
Each element that declares transitions keeps its last BASE style (no transitions applied) as the
next resolve's before-change style; a changed base value of a transitionable property (the
`lerp_styles` set: opacity, colours, border widths/colours, margins, paddings, sizes, insets,
z-index, font-size, word-spacing, text-indent, flex-grow/shrink, translate-only `transform`,
`visibility`; table `build_props`) whose matching list entry has a combined duration > 0 starts a
transition at the document clock from the before-change value (or the running transition's current
value; the reverse of a running one is shortened per §3.1). The comparison is idempotent, so any
number of resolves of one state (getComputedStyle, a forced layout, the incremental engine's adopt /
check builds) agree; read-only resolves (shadow run) only apply. `display: none`, a property no
longer listed, removal (from-scratch pass ageing in `finish_animation_pass`, membership ageing in
`age_animations`) cancel. An inherited property moved by a transitioning ancestor starts nothing on
the child (Firefox's "animation-only restyle" rule; otherwise `* {transition: color}` restarts a
child transition every frame). Values: applied over the base where no CSS animation set the field.
`is_animated` is true while a transition runs, so the incremental engine re-cascades the element on
every update; the hover patch refuses an element that declares transitions (the real restyle starts
them). Events: `transition::tick(now, interval)` in step 4 of the rendering update (App `anim_tick`,
headless `update_animations`): run/start/end by phase change (pending -> before/active/after), cancel
events queued at the style change; an ended transition is dropped one tick later (a child's restyle
in between still sees it). App frames: an active transition asks for a restyle no more often than
every max(33 ms, 2 x last build), none in between after a build > 50 ms (start/end only, QAN1d's
rule for layout-affecting animations); headless asks for none in between (no paint), only at phase
changes. Nothing running = `transition::live()` false = no wake-up. Headless style step also runs
for pages that parsed a `transition` declaration (`any_declared`). Not here: `transition-behavior:
allow-discrete` for properties outside the table (display, content-visibility), `inherit` on the
longhands (treated as initial), `transform` functions other than translate, `getAnimations()`/
CSSTransition objects, the App's clock for a transition a script's getComputedStyle starts between
builds (the last build's time; headless uses the task's page clock, `anim_eval::set_task_clock`).
getComputedStyle keeps a snapshot for every element it resolves (`transition::with_watch`), so
"flush, then set a value and `transition` together" transitions; other elements keep one only while
they declare `transition` (a compact `Snap` of the transitionable fields, not a `ComputedStyle`).

## `:has()` (QAPI6, src/dom.rs, src/invalidation.rs, src/incr_style.rs, src/incr_log.rs)

Parsing and matching are the `selectors` crate's (`OurSelectorParser::parse_has`); every matching call builds its own
`SelectorCaches`, so a cached relative-selector result never outlives a DOM write. `:has(:has())` and `:has()` are parse
errors (SyntaxError in `querySelector`). The fast paths written for "no `:has` rule" keep their guards and simply turn off on
a `:has` page: `StyleCache` (`has_rules`), `HoverKeys::unbounded`
(only for a `:hover` inside `:has`), the incremental-layout rule 5 (R59).

Invalidation map: a feature anywhere inside a relative selector `R` reaches the anchors in ONE hop, `Hop::Anc` (the carrier's
ancestors) or for `:has(+ ..)`/`:has(~ ..)` `Hop::AncSib` (earlier siblings of the carrier and of each ancestor), because every
element `R` looks at is a descendant / later sibling (subtree) of the anchor; `R`'s own combinators are ignored (flattened). The
anchors are filtered by the tag/id/class of the compound holding the `:has` (`Entry::anchor`), then the outer hops lead to the
subject. `Key::HasTree` = an element inserted (walked from the new node) or removed (`removal_anchors`: the parent and its
ancestors, or the earlier siblings of the removal point and of the parent's chain), pruned by a `TreeSummary` of the subtree
(tags / ids / classes / attribute names): only a subtree that can hold an element matching `R`'s own subject compound can change
what `R` finds (adjacency changes go through `SibNext` / `SibLater`). Global only for a selector WITH combinators nested in `R`
(`:has(:is(.a .b))` can match through an `.a` above the anchor). Soundness fuzz with 14 `:has` shapes in `qapi6_tests.rs`
(same generator as QINC1d).

Cost on YouTube (the measured case): `:has(> [role=dialog]:not([aria-hidden=true]) .x) [dialog]` has no tag/id/class on the
anchor, so every `aria-hidden` write reached the whole page through `[Anc, Desc]`. Three things keep that bounded: `Filter::attrs`
(attribute names of a compound, checked only on elements a walk REACHES, never on the written element), the shared first hop per
change (`apply_if` computes the ancestors once for all `:has` groups), and the batch memo (`begin_batch`/`end_batch` around one
engine update: a `(element, rest of the walk)` already walked against this DOM state is not walked again).

Engine: a logged change of a node WITHOUT an entry (under `display: none`) used to be skipped (only `gen_dirty`); with a `:has`
rule it can still flip a rendered anchor (`.card:has(.hidden .on)`). `incr_log::note_node` keeps a `Weak` of every logged node
while the current map has `:has` (`set_keep_nodes`, set when the map is built; no `:has` = no map lookup, nothing kept), and
`Engine::unentried` answers it from that handle (a freed node is skipped: its removal was logged on a live parent; a node that
was not kept forces a full restyle).

QAPI6b (cost, measured on YouTube results): `unentried` no longer replays each change. Nothing below the topmost entry-less
ancestor `top` is styled, so only `:has` anchors above it can change: `InvalidationMap::for_has_any(top)` walks every `:has`
path (`Key::HasAny`: each entry's path from its `Anc`/`AncSib` hop, carrier filters dropped, i.e. feature-agnostic) once per
`top` and update, and a `top` inserted under its styled parent in the same log is skipped (that insertion's `HasTree` marks,
taken from the final subtree, cover everything inside; the subtree did not exist before the batch). One update had 1 329
logged inserts into hidden lists: 236 ms of replays before, one answer per list now. Removals are no longer pruned by the
removed subtree's summary: the batch is answered against the final DOM and a detached subtree can change without a log entry.
`style::SEL_CACHES`: one `SelectorCaches` shared by every match of a build or engine update (installed and dropped with
`CLASS_CACHE`; the DOM does not change meanwhile), so `:has` results per (anchor, relative selector) and `:nth-*` indices are
computed once per build instead of being rebuilt and dropped for every element. A `:has` inside `:not()` / `:is()` /
`:where()` is anchored by the ENCLOSING compound (`Builder::enclosing`, merged into the nested selector's subject compound
filter: it is the same element): `.acc:not(:has(.open)) .vis` walked from every ancestor before (github.com's RiverAccordion
rule re-matched the whole page, ~1 900 elements, on each removal), from `.acc` elements now.

## Engine evidence (QINC1n, src/incr_evidence.rs)

Daily-use evidence for the engine (it decided QINC1m, the old pipeline's removal). Both engine paths, App frames
(`incr_frame::frame`, `check`) and script forced layouts (`incr_engine::update`,
`js_geom::refresh_engine`), report to one thread-local counter set:
- update: the engine served the frame from its kept trees (App Current / Main / worker with an
  incremental restyle; a forced layout that updated or had nothing to do);
- restart (no log line): a from-scratch build that starts the kept trees over by design: first
  layout, viewport width, a new rule list, a global change (`incr_style::live_update` returns
  `Err("global_restyle")`), the relayout's own restart (drift guard, stale view);
- fallback (ONE `scratchy: engine: fallback <reason> on <url>` diag line per document, keyed by a
  weak handle): `box_update_failed` / `no_style_engine` from `live_update`, `no_box_tree` (a forced
  layout's build did not start the box engine), `relayout_failed` (`update_kept` false), `panic` (the
  forced layout's `catch_unwind`).
- sampled check: one engine update in `config::ENGINE_CHECK_EVERY` (200; 0 = off) is also laid
  out from scratch WITHOUT the shadow update (it would restart the engine) and compared with
  `incr_cmp::cmp_laid` against `incr_layout::with_laid` (`incr_evidence::compare`). A difference:
  one diag line, one `<state-dir>/engine-check.log` line (0600, `<unix ms>\t<url>\t<box path>
  <field>`, no values, so no page text), the engine starts over (`drop_engines`) and that frame
  uses the from-scratch layout (`Frame::Fresh`; the forced read walks the fresh tree). The URL is
  the App's status-bar URL (`set_page`, called per App frame) or `js::page_url()`.
- `:engine` puts `status_text()` in the status bar; `--perf-report` JSON has an `engine_counts` object;
  a window run with `SCRATCHY_FRAME_STATS` prints the counts at exit.
Test seams (`incr_evidence::with_seam`): interval, forced fallback / mismatch, state dir, printed
lines; tests default to checks off (they run in check mode anyway).

## QXSCR1: horizontal scrolling

**Style.** `ComputedStyle` has `overflow_x`/`overflow_y` (the old single `overflow` folded the two).
`style::overflow_axes` applies CSS Overflow 3: when either axis is `hidden`/`scroll`/`auto`, a
`visible` other axis computes to `auto` and a `clip` one to `hidden`. `Overflow::Clip` is new: it
clips its axis but is no scroll container (`ComputedStyle::scroll_container`), so `overflow-x: clip`
with `overflow-y: visible` clips only x (`paint`'s `child_clip` and `compute_ink` are per axis).
The viewport propagation (`propagate_viewport_overflow`) records `viewport_hidden` (y) and
`viewport_hidden_x` separately; `clip` on the viewport counts as hidden.

**Layout.** `ScrollInfo` gains `user_x` (x not `hidden`), `content_w`, `client_w`. The widths are
set by `compute_ink` (and the engine's `incr_layout::pass`), which now folds the content's union in
per axis (`fold_content_ink`) and gives the right edge of that union to the region. `compute_ink`
takes the `ShapeCache`: a `white-space: pre`/`nowrap` paragraph's widest line extends the ink
(`text_overflow_right`), so a nowrap line in an `overflow-x: auto` box is scrollable and wide text
widens the document. `layout::doc_scroll_width` is the root's width or its ink's right edge.

**Offsets.** No new maps: an x offset lives in the same `App::scroll_offsets` / `node_scroll` as the
y one, under the key `layout::xk(id)` (`X_KEY`, the top bit). Clearing, history save/restore and
`collect_scroll_regions` carry both. `ScrollInfo::offsets(map)` returns the clamped (y, x).
`App::scroll_x` is the window's (clamped in `redraw` against `doc_scroll_width`).

**Paint / hit test.** The ambient x shift is a thread-local, `paint::SCROLL_X` (like
`PAINT_OPACITY`), not a parameter, so the many `paint`/`hit_test_*` callers keep their signatures.
The App sets it with `paint::with_scroll_x(self.scroll_x, ..)` around the frame's paint and every
hit test; `paint` adds a region's own x offset for its content and z-layers and forces 0 for a
`fixed` subtree. Hit tests take document-space x (`pos.0 + scroll_x`) and add each region's x offset
for its children (like y); a fixed box subtracts the window's `scroll_x()`. Tests that call `paint`
directly get 0 by default. Focus ring, search highlights (`visible_rects`, `reveal_x`, `doc_left`),
the select popup anchor, the text caret click and `pageX` subtract or add `scroll_x`.

**Input.** `wheel_axes`: winit's x delta, or Shift turning a vertical wheel horizontal.
`App::wheel_x` routes the x part through the same scroll chain (first region that can move on x,
else the window unless `viewport_hidden_x`). `Action::ScrollLeft/Right` on ArrowLeft/ArrowRight
(normal-mode bindings and the input-mode UA scroll keys; a focused text field keeps its arrows).
No letter keys (DECISIONS 2026-11-21).

**Script.** `scrollX` is mirrored from the App (`JsEngine::set_scroll(x, y, els)`); `scrollTo`/
`scrollBy` clamp x to `scrollWidth - innerWidth` (a layout read, only for x > 0). Elements:
`scrollLeft` reads/writes `gElScrollX`, `__d_scrollel(id, v, xAxis)`, a `scroll` event like
`scrollTop`; `getBoundingClientRect` subtracts ancestors' x offsets; `scrollIntoView` handles
`inline` for scroll containers too. Horizontal scrollbars are QSB2b.

**ResizeObserver (QEL2, js_dom.rs).** Step 7 calls `__scratchy_roRun`: gather active observations (current size
differs from the last reported one; the first report is always due, last-reported starts at -1x-1), deliver those
deeper than the last delivered depth, repeat; observations left at a shallower depth raise an ErrorEvent
"ResizeObserver loop completed with undelivered notifications." (`__scratchy_report_error(..., message)`; log line only
if not cancelled). Sizes: border box from the layout rect (`gRect`, forced JS-side layout, lazy, cached until the next
DOM write), content box = minus padding and border (computed styles), 0 for display:none / detached / non-replaced
inline, inline/block swapped for vertical writing modes, device-pixel box = content x `devicePixelRatio` rounded.
Depth is the flat-tree depth (a shadow root continues at its host). `__scratchy_ro_want()` (geometry gen, viewport, DPR
or a new observation since the last run) makes `__scratchy_raf_due` ask for a rendering update even with no rAF.
(QEL2b replaced the layout source, below.)

**ResizeObserver on the frame's layout (QEL2b, src/ro_geom.rs).** `observe`/`unobserve`/`disconnect` register the
target with Rust (`__d_ro_track`, refcounted). Every complete layout records the targets' boxes `[bw, bh, cw, ch, cx, cy]`
from the `Laid` tree (outermost box; content = minus padding and border from `Laid.style`; non-replaced inline / no box =
none): every forced layout, inside its own rect walk (`js_geom::walk` -> `ro_geom::note` between `begin_forced`/`end_forced`,
no walk of its own), and the App's frame (`ro_geom::app_install` from `App::install_laid`, a walk of its own, only when the
observers wait for it, nothing is recorded, the viewport changed, or the DOM is as recorded, i.e. the layout changed with no
DOM write; a frame for DOM writes is recorded lazily by step 7 when the shown layout is of the DOM as it is now). `roBoxes` reads `__d_ro_box(id, vw, vh)`: the recorded box when it is of
this DOM state (render stamp, or the cache key's gen/CSS for in-place shifts) and viewport; otherwise (a callback wrote the
DOM inside the depth loop, or no layout of this state exists) it runs the layout itself, `io` reason, caller
`(rendering update: ResizeObserver)`. No `getComputedStyle` read per gather: `writing-mode` is read only when a target's
physical sizes moved (a writing-mode change with equal physical sizes is not reported). SVG child elements keep the old JS
path (`roBoxesJs`). Size changes with no DOM write: an App layout whose target boxes differ from the last recorded ones bumps
`__d_ro_epoch`, part of `roHardSig`, so `__scratchy_ro_want` asks for a rendering update (image decode, CSS
animation/transition, `:hover` restyle, viewport). App step 7 (`RenderingHost::run_observers`): when DOM writes since the
shown layout mean the frame's layout is still to come (`layout_dirty` / `layout_in_flight`), `ro_geom::set_wait(true)`:
`__scratchy_ro_want` says -1 (no update owed, no spin) until `install_laid` records that layout and ends the wait, then the
next update's step 7 gathers from it. So the App's own frame layout (main thread or worker, incremental engine) is the one RO
reads; RO forces nothing per frame. Effect: one painted frame shows the pre-callback state (layout -> paint -> RO callbacks ->
layout -> paint), where browsers run the callbacks before painting. Headless: step 6 (`Headless::update_style_layout` ->
`JsEngine::observer_layout` -> `__scratchy_ro_layout`) lays out when an observation may have changed, caller
`(rendering update: style and layout)`; step 7 reads it. A microtask checkpoint runs after each RO callback. Counters:
`--perf-report` `resize_observer` {runs, callbacks, app_collects, forced_collects, ro_layouts, changes}; window runs with
`SCRATCHY_FRAME_STATS` print them at exit.

**IntersectionObserver (QEL3, js_dom.rs).** Step 7 runs `__scratchy_ioRun` right after RO: when an observation is new
(`need`), the viewport / scroll position changed (`ioHardSig`) or the geometry generation moved, every observer that needs it
is evaluated once against one `performance.now()` (the update's time, shared by all entries), entries are queued and delivered
by ONE timer-queue task (`obsFlush`, ordered by observer creation). No IO timer remains: `__scratchy_ioTick`, `ioNudge` and
`ioDueT` are gone; `__scratchy_io_want` (like `ro_want`) feeds `__scratchy_raf_due`, so a page without rAF still gets its
frame. A geometry-only change with stale geometry waits `max(100, 4 x last layout cost)` ms (cap 2 s) before the update forces a
layout (QY53e1 rate limit, unchanged); a fresh geometry evaluates at the next frame. (The App-layout reuse, QY53e2, was
removed by QINC1m: the engine's forced layouts read the same kept tree.) Delivery is now at frame boundaries (first entry ~17 ms after `observe()` headless, not 0 ms).


**UA-dispatched events (QEL6).** `__scratchy_dn` (events born in Rust) tags the event with `__scratchy_ua`; `dispatch` runs a microtask checkpoint (`__scratchy_microtasks`) after each listener and handler callback of a tagged event only. Nested/script dispatches never checkpoint (script is on the stack).

`inline` for scroll containers too. Horizontal scrollbars: QSB2b below.

## QSB2b: horizontal scrollbars

`scrollbar::Axis` (Y/X); `track_rect_axis(axis, rect, corner)` (right / bottom `TRACK_W` strip, `corner` = the
other bar shows so the track stops short), `corner_rect`, `hit_axis`. The thumb/drag/page maths was already 1-D.
Hot ids: `VIEWPORT_ID`, `VIEWPORT_X_ID`, a region's `id` and `layout::xk(id)`. `SbFound/SbDrag/SbHold` carry the
axis; `scrollbar_set(target, axis, v)` writes `scroll_offset`/`scroll_x`/the region's `xk(id)` offset (horizontal
moves are instant; QSMOOTH1 eases y only).

Region bars (`paint::paint_scrollbars`): overlay, vertical as before, horizontal along the bottom when
`user_x` and `content_w > client_w` (or `overflow-x: scroll`); with both, the tracks stop 8 px short and a corner
square is painted. Hit tests use the same rule (both overflowing => corner).

Window bar: `App::hbar` (set in `redraw` from `want_hbar`: `doc_scroll_width > width` and the viewport `overflow-x`
not `hidden`). While it shows, `page_size()` / `page_viewport_hb` are `TRACK_W` shorter (layout height,
`innerHeight`, JS viewport, vertical bar); the strip above the status bar is filled with the canvas colour and
`paint_viewport_hbar` draws the bar, plus the corner when the vertical bar shows. A change of `hbar` redoes the frame.
### document.fonts (QAPI4)
`src/fonts_prelude.js` (after the CSSOM prelude; `FONTS_PRELUDE` in js.rs, in `realm_preludes` and `PRELUDE_SCRIPTS`). (QFF1a: faces are real now; see "Web fonts (QFF1a)" below.) `document.fonts` is a `FontFaceSet` whose members are the `@font-face` rules of `document.styleSheets` (a `FontFace` cached per rule object, status `unloaded`; re-read on every access, so rules added later appear) plus faces added by script. `FontFace.load()` / `fonts.load(font)` fetch the first `url()` source with the page `fetch` (so only wire sees it), discard the bytes and set `loaded`/`error`; `check(font)` is true when every face matching the font shorthand's families is `loaded` (true for unknown = system families). Set status/events: `loading` on the first pending face, `loadingdone`/`loadingerror` from a timer once none is pending; `ready` is a lazily created promise (do not create promises while a prelude runs: the leftover job keeps a dropped iframe realm alive, QPT1b nosnap heap test). Not done: matching by weight/style/unicode-range, `FontFace` in workers, loading `local()` fonts for real.

### Web fonts (QFF1a, src/webfonts.rs)
- **Rules:** css.rs `TopLevelParser` parses `@font-face` (also inside `@media`, which is kept on the rule, and true
  `@supports`) into `webfonts::FaceRule` (family, `src` list of `url()`+`format()` / `local()`, weight or weight range,
  style, `unicode-range`; `font-display` ignored = swap). `parse_stylesheet_into` stashes them in a thread-local; the rules
  cache keeps them per piece (`Piece::ff`) and publishes the page's list on every `cached_rules` (`publish_declared`, a
  pointer/value compare when unchanged). `@media` is evaluated at publish time.
- **Use:** `FontFamily::resolve` asks `webfonts::lookup` for each named family on a cache miss: a declared family marks
  the best face for the requested (weight, style) as wanted (style first, then weight; a face whose `unicode-range` misses
  Basic Latin only when nothing else fits; at most 64 per page), and a family with a loaded face resolves to its mangled
  per-page name `"<family> \u{2063}wf<page>"` (tried with `db.query` first, so a font system that has not synced yet
  falls through to the next family).
- **Loading:** `App::webfonts_tick` (start of `js_sync`, i.e. after every event batch and script task) drains the wanted
  faces: `local()` is looked up in the App's system fonts (bytes copied), `url()` sources are fetched in order on a thread
  per face through wire (`load_sources` / `fetch_font_bytes`: credentials same-origin, partition headers, `Origin` +
  ACAO check cross-origin, 2xx only; svg/eot skipped by `format()` or extension; `data:` decoded in place), woff 1.0
  is inflated to sfnt (flate2), the first face is parsed with a throwaway fontdb. `AppEvent::FontFetched` (nav
  generation + registry page guard) registers it: its `FaceInfo` gets the mangled family and the descriptor's weight
  (clamped into the range) and style.
- **Font systems:** the registry (one per App: `webfonts::handle`/`install`, the shaping worker installs its App's; any
  other thread gets its own, so parallel tests are isolated) keeps an append-only log of face adds/removes. `sync(fs,
  &mut FontSync)` replays it into a font system: App (`App::font_sync`), worker (each request), forced layout
  (`js_geom::FONTS_SYNC`, also `sync_forced_fonts`), reftest renderer. Same ops in the same order = the same fontdb ids
  everywhere (the worker shapes, the App paints); a removed slot replays as a dummy add + remove so a late joiner matches.
  A sync that removed faces rebuilds the `FontSystem` around its database (cosmic-text's font cache holds the bytes). Any
  change clears that thread's `FontFamily::resolve` cache and shape memo; the worker also drops its shape cache.
- **Relayout:** a face that arrives (`webfonts_tick`): App + forced-layout fonts synced, `incr_style::drop_engines()`
  (the kept trees were shaped with the fallback), `document.fonts` re-synced, `restyle_requested` and the image-arrival
  debounce (a burst of faces is one relayout; the old frame stays up until the new one is shaped). `GeomCache` also keys on
  `webfonts::generation()` (taken after the layout, so a first parse's publish does not make it stale).
- **Navigation:** `App::replace_document` calls `webfonts::new_page()` (every face removed, page number bumped) and syncs.
- **document.fonts:** a rule's `FontFace` key is `family|first source` (same as `FaceRule::key`); its status follows
  `__scratchy_font_state` (`__scratchy_fonts_sync` is called by the App when a load starts or ends). `FontFace.load()`
  fetches the first usable url through page `fetch` and hands the bytes to `__scratchy_font_add` (in `document.fonts`:
  used for rendering) or `__scratchy_font_check`; binary sources the same; `document.fonts.delete` of a script face calls
  `__scratchy_font_remove`. So `--wpt-th-one` gets script faces too, but not CSS-driven loads (no App).
- **Reftests (`--wpt`):** `Renderer::render_inner` starts a page per test, lays out, loads the wanted faces synchronously
  from the checkout (`load_wanted_now`), and lays out again (at most 3 rounds).
- **woff2 (QFF1b):** `src/woff2.rs`: table directory, brotli by the `brotli-decompressor` crate (safe, no sockets; read through `take(total + 1)` so output is capped at 32 MiB and must equal the declared sizes), glyf/loca transform rebuilt into plain TrueType glyphs (uncompressed 16-bit coordinates; a short loca that would overflow switches to long and `head.indexToLocFormat` is patched), hmtx transform (omitted lsb from glyph xMin). Collections and anything malformed return `None`: the face fails like any bad font and the next `src` / system fonts are used. No header size claim is used for allocation.
- Not done: `unicode-range` beyond picking one face per (weight, style) (a second subset is never used for
  the characters the first lacks); `size-adjust`/`ascent-override`; variable-font axes beyond cosmic-text's own weight
  handling; CSS-driven loads in `--wpt-th-one`.

### Pre-insert validity (QDOM18)
`appendChild`/`insertBefore`/`replaceChild` (js_dom.rs) call `cnCheck` ("ensure pre-insert validity", `replace` = the replace variant) before any native edit, then `rawIns` (`__d_insert` + the after-insertion hook). Fast path: an Element parent with an Element/Text/CDATA/PI/Comment/Fragment node skips `cnCheck` (the native insert refuses an inclusive ancestor); an `insertBefore` ref that is not a child still goes through `cnCheck` so the error order is spec (ancestor HierarchyRequestError before NotFoundError). The ParentNode/ChildNode mixins (QDOM16) use `cnCheck` + `rawIns` directly. Nothing is mutated and no mutation record is queued when a check throws.

### Inline SVG paint properties (QCSS-A)

An inline `<svg>` is serialized to XML and rasterised by resvg at box-build time. The seven paint properties (`style::SVG_KEYS`) are real inherited cascade properties (`ComputedStyle::svg`, interned `SvgProps`), so an HTML ancestor's `fill`/`color` reaches the svg. `images::serialize_inline_svg_styled` resolves the style of every svg descendant (not defs/gradients/clip containers) and writes the declared values into `style` attributes; the root also gets inherited values it has no presentation attribute for. The engine sees the descendants as ordinary elements: a change of their computed `svg` marks the owning svg box (`svg => B` in `incr_diff`), and an ancestor change re-cascades them through `inherited_changed`.

## The engine is the only pipeline (QINC1m, 2026-11-22)

The user said remove the old pipeline (DECISIONS.md 2026-11-22). Since QINC1m:
- App frames always go through `incr_frame::frame` (`App::rebuild_layout` -> `rebuild_live`), script forced
  layouts always through `incr_engine::update` (`GeomCache::refresh_engine`). There is no `SCRATCHY_INCR`
  any more (setting it changes nothing), no `incr_frame::live` / `incr_engine::forced_on` and no test seam that
  turns the engine off.
- The from-scratch layout (`build_layout_tree_with_resources` + `measure_document`) stays: the engine's
  first layout / restart (its shadow stage starts the kept trees), its fallback (`incr_evidence` fallback
  reasons, `GeomCache::refresh` when the engine has no tree, `Frame::Fresh`) and the oracle of check mode and of
  the sampled check (`engine-check.log`).
- `SCRATCHY_INCR_CHECK=1` (debug option): every engine update is compared with a from-scratch build and
  layout; a difference prints `scratchy: incr-check: MISMATCH ...`, counts `incr.check_mismatch` and the
  from-scratch result is used. Tests always run in check mode (a difference panics unless a test's
  `incr_style::Seam { panic: false }` collects it).
- `:engine`, `--perf-report`'s `engine_counts` and the exit counters with `SCRATCHY_FRAME_STATS` are unchanged
  (the exit counters now print for every window run with frame stats).
- Tests: the `*_incr_forced_tests` re-runs and `SCRATCHY_TEST_INCR_FORCED` / `SCRATCHY_SHADOW` are gone (the
  suite runs once, engine on); CI's `scratchy` job runs the suite once (the engine-off step is gone; the four
  required check names, scratchy / snag / wire / scratchy-wpt, are unchanged).
- `tools/incr-gate`: modes `incr` and `check` only (`off` and the `perf` series that compared against it
  were removed); compare against an older build with `SCRATCHY=` instead.

## tools/nest.sh keeps one compositor per worktree (QNEST1)
- The nested dwl is recorded in `<worktree>/.qwork/nest/` (`dwl.pid`, `disp`, `dwl.log`). A live recorded dwl
  (PID alive and its command line names that registry) is reused: nest.sh prints its socket and does not start
  another, whatever OUT_DIR the caller passes. `--new` stops it first, `--stop` stops it and removes the pid file,
  a stale pid file is cleaned. `NEST_TIMEOUT` remains the last-resort cap. Test: `tools/nest-one-test.sh`.
- incr-gate/run.sh no longer leaks one dwl per run; gate.sh still kills its dwl by PID at the end.

### filter, backdrop-filter, text-shadow, mix-blend-mode, clip-path (QCSS-D, src/fx.rs)

Five paint properties, all `P` (paint, no relayout) in `incr_diff::style_walk`; `text_shadow` is also compared in `incr_style::inherited_changed` (it inherits through `InheritedStyle::parent`). Declared values are `Fx<T>` (`None` or `Value`, so an explicit `none` wins the cascade like `ShadowDecl`) with unresolved `Length`s; `style.rs` resolves them (CSS-wide keywords through `cascade_css`/`apply_wide`, em/rem against the element's font size, `currentColor` against its `color`) into interned `&'static` lists (`ComputedStyle` is `Copy`; the tables are leaked and de-duplicated, so memory grows only with distinct values; animations snap `t` to 1/128 for that reason). `getComputedStyle` serialises the computed forms (`js_style.rs`). Out of scope: `url()` filters, `path()`/`url()` clip-paths, the non-separable `mix-blend-mode` modes (hue, saturation, color, luminosity: parsed, painted as normal). `inset()` keeps one radius for all corners; `at <position>` takes one or two items.

**Paint.** `paint` (paint.rs) checks `fx::has_effects` and calls `fx::paint_with_effects`, which wraps `paint_plain` (the old `paint`). The page buffer is opaque RGB, so an element that needs its own alpha (`filter`, `mix-blend-mode`) is painted twice into pooled window-sized scratch buffers, over black and over white, clipped to its region: `white - black = 255 (1 - alpha)` and `black = colour * alpha` give the premultiplied RGBA of the whole subtree. The filter chain runs on that layer (colour matrices on the unpremultiplied colour, `blur` through `blur_mask` per channel plane, `drop-shadow` = offset blurred alpha under the layer), then the layer is composited with the `clip-path` coverage (2x2 samples) and the blend function. `clip-path` alone paints normally and then takes back the uncovered pixels from a snapshot of the region. `backdrop-filter` filters the page pixels under the border box (read with the blur's reach, blur edge normalised by a coverage plane, written back inside the rounded border box) before the box paints. Region = ink + the chain's reach, clipped to the window. Bounds: sigma <= 40 px, reach <= 256 px, at most 8 functions, a region over `MAX_REGION_PX` (2.4 M pixels) paints without the effect. An effect box's `opacity` is baked into its layer (accumulated through `PAINT_OPACITY`): opacity then filter, not the spec's filter then opacity.

**text-shadow.** `paint_inner_with_opacity` sets the thread-local `fx::TEXT_SHADOW` to the box's list; `paint_buffer` replays its glyph walk per shadow into a coverage mask (offset, blurred, tinted) under the glyphs, so `color: transparent` text still shadows. One list per text block (no per-span shadows); decorations are not shadowed; sigma <= 16 px.

**Inline elements** have no box (the builder flattens them), so their `filter`/`backdrop-filter`/`mix-blend-mode` goes to the block boxes built inside them (`fx::with_inline_fx`, block-in-inline); text directly inside the inline element is not filtered (known gap).

**Animation.** `anim_eval::lerp_styles`/`merge_styles` interpolate `filter`/`backdrop-filter` (same function list, or `none` padded with identities) and `text-shadow`. Transitions of these (transition.rs `PropDef`s) and `clip-path` interpolation are not done.

## Canvas 2D (QCNV1)

`src/canvas.rs` keeps one `Canvas` per `<canvas>` that called `getContext('2d')` (global map keyed by node id, replaced at each `getContext`): a tiny-skia `Pixmap` (premultiplied RGBA8, `width` x `height`, none above 2^26 pixels), the draw state (`fillStyle`, `strokeStyle`, `globalAlpha`, `lineWidth`) with its save/restore stack, and a cached straight-alpha `DecodedImage` for paint. The JS object (`CanvasRenderingContext2D`, js_dom.rs next to QY3) validates arguments and calls `__cv_*` natives; it is created once per element (`el.__ctx2d`). Setting `width`/`height` (even to the same value) resets the bitmap and the state. Colours go through the CSS parser (`css::to_text_color`; `lab()`/`color()` are not handled: invalid values are ignored; `currentcolor` is black).

Layout/paint: `canvas` is `display: inline-block` and `images::build_canvas_box` makes a replaced box (width/height attributes, 300x150 default, CSS wins) whose content is an empty-pixel `DecodedImage` with `canvas::MARK` in `svg`. `paint` sees the mark and blits `canvas::image(node)` (object-fit fill). A draw never touches the DOM: it records the node in the dirty set and `App::js_sync` requests a redraw; `App::redraw` drains it (`canvas::take_dirty`), `incr_layout::canvas_damage` turns it into the box's document rect (None under a fixed/sticky/scroll-region/masked box: full paint) and adds it to the engine's damage, so the QHOV1 kept-page path repaints only that rect without a relayout. `canvas::any()` turns the damage path on for pages with a canvas even without `:hover` rules. Not done (QCNV2): paths, text, images, transforms, gradients, compositing modes, shadows, `toDataURL`/`toBlob`, OffscreenCanvas.

QCNV3 changes to the above: the registry is keyed by a canvas id allocated at `getContext` (`canvas::Registry`: `by_id`, plus `by_node` = node address -> id, used only by paint/damage; a new context for an address that already had a canvas replaces it), the natives take that id. Colours go through `csscolor::parse` (src/csscolor.rs): `cssparser-color` parses `lab()`/`lch()`/`oklab()`/`oklch()`/`color()` but cannot convert them, so csscolor converts (XYZ-D65 hub, CSS Color 4 matrices, out-of-gamut values clamp, no gamut mapping), and adds `color-mix()` (in any space, percentages, hue methods, premultiplied alpha) and relative colours (`rgb|hsl|hwb|lab|lch|oklab|oklch|color(from <color> ...)` with channel keywords, numbers, percentages, `none`; no `calc()`), which resolve at parse time to `Lab`/`ColorFunction` values. `css::parse_color` replaces `CssColor::parse` for every stylesheet colour, so the same syntax works in CSS when it has no `currentcolor`; `css::to_text_color` converts the new variants. Canvas getters serialise legacy sRGB (rgb/hsl/hwb/named/hex) as `#rrggbb`/`rgba()` and everything else in its CSS form (`color(srgb 1 0 1)`, `lab(29 39 20)`; relative and mixed colours as `color(srgb ...)`/their space). `currentcolor` is `getComputedStyle(canvas).color` read when the style is set (black when the canvas is not connected; not-rendered canvases are not handled: manual test 2d.fillStyle.parse.current.notrendered). System colours are a fixed light-scheme table (`csscolor::system_color`). Width/height follow the HTML rules for parsing non-negative integers (`canvas::parse_dim`, JS getter mirrors it); the setter is ToUint32 and stores 300/150 above 2^31-1. The context prototype is WebIDL-shaped (non-writable `CanvasRenderingContext2D.prototype`, enumerable writable configurable methods) and every method/accessor brand-checks `this` (TypeError). `shadowColor` is stored and saved/restored but has no effect on drawing (QCNV10 shadows). Lab/LCH lightness is clamped to 0..100 and Oklab/OKLCH to 0..1 when converting (css-color/lch-l-over-100-1.html). Results: html/canvas/element score 23.6 -> 27.5 (all-pass files 294 -> 345), html/canvas/other 50.0 -> 75.0 (color.window.html 0/15 -> 15/15; the whole file failed because `shadowColor` did not exist), offscreen unchanged 0.0; no other wpt-th area moved. Reftest gate: css/css-color/a98rgb-001.html and css/css-color/oklch-007.html newly pass (351 pass / 275 fail), no pass->fail. Left: colour objects (CSSRGB/CSSHSL, Typed OM), `calc()` in relative colours, `pixelFormat: 'rgba-float16'` (QCNV20), ImageData ctor.basics details beyond what is listed in PROMPTS.

QCNV4: `OffscreenCanvas` (main thread only; the `.worker.js` variants stay NOT-RUN) is a JS object next to the canvas element code in js_dom.rs; `getContext('2d')` makes a canvas.rs bitmap with no element (`__cv_ctx_off`: node 0, not in `by_node`, `mark` ignores it, so nothing paints it and it never enters the damage path). `OffscreenCanvasRenderingContext2D.prototype` holds copies of the `CanvasRenderingContext2D.prototype` descriptors (same functions, so the same brand registry and `__cv_*` natives), plus its own `canvas` getter. The bitmap is released by a FinalizationRegistry on the OffscreenCanvas. wpt-th html/canvas/offscreen: 0 -> 316 all-pass files, score 0.0 -> 29.9 (the QCNV2 ceiling was 278 files). Not done: transferToImageBitmap (QIMGBMP1), convertToBlob (QCNV16), transferControlToOffscreen.

QCNV5: transforms. The CTM (`DrawState.ctm`, six f64 a..f) is part of the saved state; `canvas::set_ctm` multiplies/replaces/resets it, `draw` hands it to tiny-skia as the `Transform` for fillRect/clearRect/strokeRect (line width goes through it too; putImageData ignores it). JS side (js_dom.rs next to `meth('save')`): ToNumber on all arguments first, then the call is ignored when any is non-finite. setTransform accepts 6 numbers or a plain object (a..f / m11, m12, m21, m22, m41, m42, mismatching pair = TypeError); getTransform returns a PLAIN OBJECT with the DOMMatrix fields (QGEOM1 then made it a real DOMMatrix and setTransform's dictionary a DOMMatrix2DInit). wpt-th: element 27.8 -> 29.8, offscreen 29.9 -> 32.3 (+26 all-pass files each).

QCNV6: paths. `canvas::PathData` holds the current default path as `Seg::{Move,Line,Close}` in DEVICE space (the CTM at the time a point is added maps it; QCNV7 curves add variants and a `build` arm). It is not part of the saved state; `beginPath` clears it and a width/height reset drops it. `fill` uses tiny-skia `fill_path` with FillRule Winding/EvenOdd; `stroke` maps the path back through the inverse CTM and strokes in user space with the CTM as the transform. `DrawState.clip` is an `Option<Arc<Mask>>` (saved/restored), `clip()` intersects it with the path (an empty path clips everything); fillRect/strokeRect/fill/stroke use it, putImageData does not. clearRect paints DestinationOut (tiny-skia's Clear ignores the mask). wpt-th: element 29.8 -> 38.0, offscreen 32.3 -> 42.1 (+105 all-pass files each).

QCNV7: curves. `Seg::Quad`/`Seg::Cubic`; arcs/ellipses (and roundRect corners) are cubics built in user space then mapped by the CTM (`PathData::ellipse`); arcTo works in user space from the current point mapped back through the inverse CTM. Natives: `__cv_path` ops 5 quad, 6 cubic, 7 arc/ellipse, 8 arcTo, 9 roundRect. wpt-th: element 38.0 -> 46.2, offscreen 42.1 -> 51.8.

QCNV8: line styles. `DrawState` holds cap/join (0..2 indices), miter limit, the dash list (already even length) and offset; all are saved/restored. `stroke_of` maps them to tiny-skia (`StrokeDash::new` returns None for an all-zero list: the stroke stays solid). Strokes run in user space with the CTM as transform, so widths, dashes and joins are all transformed. `PathData::build_pruned` removes zero-length segments before `stroke` only (the spec prunes them; otherwise round caps draw dots). wpt-th: element 22.7 -> 23.7 (score 46.2 -> 47.8), offscreen 42.9 -> 44.7 (51.8 -> 53.6); 41 more all-pass files in total (the QCNV2 table's 91 files overcounted: many line-styles tests also need gradients, text or drawImage).

## Canvas 2D triage after QCNV1 (QCNV2)

Baseline: origin/main 7a3b00c0 (QCNV1 #613 merged), wpt-th expected files `html-canvas-{element,offscreen,other}.txt`, WPT ac78b567. element 1,276 files / 2,818 subtests / 338 pass / score 23.5; offscreen 1,074 / 1,371 / 0 / 0.0; other 4 / 18 / 2 / 50.0. Failing (not all-pass) files: element 980 (2,485 failing subtests), offscreen 1,074 (1,375), other 2 (16).

**Method.** For every failing file, the inline script and body markup of the test source (pinned checkout) were matched against API tokens that scratchy does not implement yet (`beginPath|moveTo|...` = paths, `createImageBitmap` = ImageBitmap, `globalCompositeOperation` = compositing, an `<img>` that is not the "expected output" image = image source, ...). A file counts as unlocked by a step once every feature it uses is covered by that step or an earlier one, so the cumulative numbers assume each step is implemented correctly: they are a ceiling, not a forecast. Subtests = failing subtests of those files (an ERROR/TIMEOUT 0/0 file counts as 1). The score estimate adds each unlocked file's missing fraction to the wpt-th score (mean of passed/total over the files that ran with subtests: 1,267 element, 1,070 offscreen).

**Findings that shape the order.**
- The offscreen area is not Worker work. The 1,074 offscreen files wpt-th runs are the `.html` variants: `new OffscreenCanvas(w, h)` on the main thread, otherwise the same generated test as the element twin. The 1,040 `.worker.js` variants are NOT-RUN (`wpt_th_run::is_worker_variant`) and are in no count here. A main-thread OffscreenCanvas turns element passes into offscreen passes: 278 offscreen files (297 subtests) pass as soon as it exists, and every later step counts about twice.
- Most drawImage, compositing and pattern tests do not use `<img>`: they `fetch('/images/x.png')`, `.blob()`, `createImageBitmap(blob)`. ImageBitmap (platform work; `fetch().blob()` exists) is the main image source of the canvas tests. `<img>` sources also need the decoded pixels on the JS thread (today js_dom.rs `IMAGE_STATES` keeps only the size).
- 51 of the 125 element text files load the `CanvasTest` web font through `@font-face`/`FontFace`: canvas text needs the web-font registry (QFF1a, QAPI4) reachable from canvas text shaping.
- 6 manual/wide-gamut-canvas files hold 1,087 of element's 2,485 failing subtests (display-p3 ImageBitmap/pattern matrices). They dominate subtest counts and barely move the score.
- Proposals, all `.tentative.`: HTML-in-canvas (`drawElementImage`, `captureElementImage`, `onpaint`, `layoutsubtree`) 94 files / 207 subtests; text clusters and shaping (`shapeText`, `fillTextCluster`, glyph runs) 62 files / 378 subtests. Not filed.
- Residual QCNV1 gaps that use no missing API: 49 element files (canvas-host width/height attribute parsing and reflection 23 + toStringTag 1, `currentcolor` 7, colour objects / CSS Typed OM `CSSHSL` 5, `color-mix()`/relative colours 6, system colour, object `toString`, prototype descriptors, `|this|` checks, ImageData edge cases 7), html/canvas/color.window.html (15 subtests: all fail, even `rgb()` serialisation, so check first why the whole file fails) and historical.any.html (needs OffscreenCanvasRenderingContext2D).

**Failing files and subtests by directory:**

| directory | el files | el failing subtests | off files | off failing subtests |
|---|---|---|---|---|
| path-objects | 210 | 210 | 211 | 211 |
| fill-and-stroke-styles (colours, gradients, patterns) | 129 | 129 | 247 | 247 |
| text | 118 | 278 | 119 | 277 |
| compositing | 87 | 87 | 97 | 97 |
| shadows | 51 | 51 | 56 | 56 |
| drawing-images-to-the-canvas | 34 | 34 | 28 | 28 |
| canvas-host | 30 | 40 | 35 | 46 |
| line-styles | 29 | 29 | 33 | 33 |
| reset | 29 | 29 | 29 | 29 |
| transformations | 22 | 22 | 22 | 22 |
| layers | 17 | 22 | 20 | 25 |
| the-canvas-state | 15 | 26 | 20 | 56 |
| pixel-manipulation | 14 | 14 | 59 | 59 |
| drawing-rectangles-to-the-canvas | 13 | 13 | 32 | 32 |
| filters | 11 | 11 | 12 | 12 |
| wide-gamut-canvas, color-type, global-hdr-headroom | 13 | 35 | 7 | 12 |
| canvas-context, conformance-requirements, (root) | 8 | 18 | 20 | 20 |
| manual/draw-element-image (tentative) | 92 | 204 | 5 | 6 |
| manual/wide-gamut-canvas | 13 | 1,087 | 1 | 1 |
| manual/imagebitmap | 16 | 66 | - | - |
| manual/the-offscreen-canvas, convert-to-blob, draw-generic-family | - | - | 16 | 93 |
| other manual dirs (filters, drawing-images, paths, shadows, smoothing, ...) | 30 | 98 | 5 | 13 |
| video (media archived) | 1 | 1 | - | - |

**Missing features used by failing files** (a file using several counts under each):

| feature | el files | el subtests | off files | off subtests |
|---|---|---|---|---|
| OffscreenCanvas | 14 | 62 | 1,073 | 1,374 |
| paths (beginPath/moveTo/lineTo/rect/fill/stroke/clip) | 281 | 288 | 278 | 298 |
| text (fillText/strokeText/measureText/font/text*) | 140 | 323 | 136 | 302 |
| drawImage | 128 | 1,157 | 94 | 151 |
| transforms | 97 | 184 | 87 | 107 |
| globalCompositeOperation | 94 | 105 | 105 | 119 |
| ImageBitmap | 85 | 1,037 | 99 | 140 |
| curves (arc/arcTo/ellipse/bezier/quadratic) | 76 | 76 | 76 | 76 |
| createPattern | 76 | 216 | 58 | 70 |
| shadows | 68 | 85 | 71 | 94 |
| gradients | 65 | 65 | 63 | 63 |
| FontFace / document.fonts | 58 | 68 | 54 | 64 |
| line styles / dash | 56 | 70 | 59 | 79 |
| roundRect | 56 | 56 | 56 | 56 |
| img/svg/video element source | 103 | 921 | 9 | 33 |
| reset() | 39 | 62 | 33 | 33 |
| DOMMatrix / DOMPoint | 37 | 116 | 27 | 27 |
| context attributes (alpha, willReadFrequently, colorSpace) | 36 | 1,174 | 19 | 43 |
| colour space / float16 / HDR | 36 | 1,156 | 14 | 19 |
| isPointInPath / isPointInStroke | 30 | 30 | 30 | 30 |
| layers (beginLayer/endLayer) | 20 | 25 | 21 | 26 |
| filter / CanvasFilter | 20 | 30 | 16 | 16 |
| toDataURL / toBlob / convertToBlob | 12 | 46 | 6 | 31 |
| Path2D | 8 | 8 | 9 | 9 |
| Worker | 7 | 16 | 13 | 63 |
| imageSmoothing | 7 | 15 | 3 | 11 |
| drawFocusIfNeeded | 7 | 7 | 2 | 2 |

**Cumulative unlock in dependency order** (files newly passing at each step; score columns are ceilings):

| step | el files | off files | subtests (all areas) | el score | off score | non-canvas dependency |
|---|---|---|---|---|---|---|
| now | | | | 23.6 | 0.0 | |
| QCNV3 residual host/colour | 52 | 1 | 69 | 27.7 | 0.1 | CSS Color 4/5 parsing in css.rs for ~12 files |
| QCNV4 OffscreenCanvas (main thread) | 0 | 278 | 297 | 27.7 | 26.1 | none (Workers only for the NOT-RUN variants) |
| QCNV5 transforms | 25 | 25 | 50 | 29.7 | 28.3 | getTransform: QGEOM1 |
| QCNV6 paths, fill rules, clip | 72 | 72 | 144 | 35.3 | 35.0 | |
| QCNV7 curves, roundRect | 87 | 87 | 174 | 42.2 | 43.2 | |
| QCNV8 line styles, dash | 44 | 47 | 104 | 45.6 | 47.6 | |
| QGEOM1 DOMMatrix/DOMPoint | 25 | 25 | 50 | 47.5 | 49.9 | yes: Geometry Interfaces |
| QCNV9 Path2D, isPointInPath/Stroke | 32 | 32 | 64 | 50.1 | 52.9 | |
| QCNV10 gradients | 54 | 53 | 107 | 54.3 | 57.9 | |
| QCNV11 globalCompositeOperation | 45 | 52 | 101 | 57.8 | 62.7 | |
| QCNV12 shadows | 47 | 52 | 129 | 61.4 | 67.6 | |
| QCNV13 drawImage (canvas sources), smoothing | 21 | 17 | 39 | 62.9 | 69.2 | |
| QIMGBMP1 ImageBitmap | 51 | 58 | 118 | 67.0 | 74.6 | yes: ImageBitmap |
| QCNV14 img/svg sources, patterns | 93 | 55 | 192 | 74.3 | 79.7 | yes: decoded pixels on the JS thread |
| QCNV15a+b text | 97 | 95 | 221 | 81.8 | 88.6 | yes: web fonts in canvas shaping |
| QCNV16 toDataURL/toBlob/convertToBlob | 1 | 0 | 1 | 81.9 | 88.6 | |
| QCNV17 reset, context attributes, focus | 39 | 36 | 102 | 85.0 | 92.0 | |
| QCNV18 filter | 15 | 9 | 34 | 86.1 | 92.8 | url(): SVG filter effects |
| QCNV19 layers (proposal) | 20 | 21 | 51 | 87.7 | 94.8 | |
| QCNV20 colour spaces, float16, HDR | 35 | 14 | 1,173 | 90.5 | 96.0 | yes: colour management, Float16Array |
| not filed: Workers | 5 | 9 | 71 | 90.7 | 96.7 | Workers (+ 1,040 NOT-RUN worker variants) |
| not filed: text clusters (tentative) | 31 | 31 | 378 | 93.2 | 99.6 | proposal |
| not filed: HTML-in-canvas (tentative) | 89 | 5 | 207 | 100 | 100 | proposal |

Notes on the order: transforms come before paths because path points are transformed when added. OffscreenCanvas comes early because it is small and doubles the yield of every later step. toDataURL/toBlob unlocks almost nothing directly (its files also use images or colour spaces; the plain toDataURL tests live in html/semantics/embedded-content/the-canvas-element, not a wpt-th area), but QCNV14, QCNV20 and the QCNV1-noted `toDataURL -> <img>` tests need it, so it can move earlier. The ERROR/TIMEOUT files 2d.drawImage.svg, 2d.drawImage.zerosource.image, 2d.pattern.animated.gif and the four flaky manual/imagebitmap files (flaky.txt) belong to QIMGBMP1/QCNV14.

**Proposed TODO items** (for the coordinator to file; same text in the QCNV2 PR). Every item's test plan: unit tests in `src/tests/<item>_tests.rs` reading pixels back with getImageData, then `--wpt-th --area html/canvas/*` and `--update`, report the area scores before/after; no pass->fail anywhere.
- QCNV3 [S] (P2; Sonnet; js_dom.rs canvas section, canvas.rs colour parse, css.rs) Residual QCNV1 conformance: width/height reflection (rules for parsing non-negative integers: `100.999`, `1e2`, `0x10`, `+100`, leading whitespace, trailing junk; the attribute string stays verbatim; IDL setter ToUint32 and non-numbers -> 0; getComputedStyle width from the parsed value), `Object.prototype.toString` = `[object HTMLCanvasElement]`, CanvasRenderingContext2D.prototype descriptors, `|this|` brand checks (TypeError), createImageData truncation before the zero check, ImageData ctor basics, fillStyle/strokeStyle ToString of non-gradient objects (a throwing toString propagates), `currentcolor` = the canvas element's computed colour at set time (inherited, stylesheet, not rendered), system colours, colour serialisation (html/canvas/color.window.html), `color()`/`lab()`/`color-mix()`/relative colours (these need css.rs CSS Color 4/5 parsing: split out if css.rs lacks them). Also the QCNV1 note: key the canvas registry by an id allocated at getContext, not the node address. Not: colour objects/CSSHSL (Typed OM, proposal). Should fix ~49 element files + color.window.html (15 subtests); element 23.6 -> ~27.7.
- QCNV4 [S] (P2; Sonnet; js_dom.rs, canvas.rs) OffscreenCanvas on the main thread: constructor (`[EnforceRange] unsigned long long`), width/height (setting resets), getContext('2d') -> OffscreenCanvasRenderingContext2D sharing the context implementation (`canvas` returns the OffscreenCanvas; no paint), other types null, `commit` absent. Not: transferControlToOffscreen, convertToBlob (QCNV16), transferToImageBitmap (QIMGBMP1), Workers. Should fix 278 offscreen files (297 subtests) + historical.any.html; offscreen 0 -> ~26.
- QCNV5 [S] (P2; Sonnet; canvas.rs) Transforms: CTM in the saved state; translate/rotate/scale/transform/setTransform (6 numbers or a DOMMatrix2DInit dict)/resetTransform, non-finite arguments ignored; fillRect/strokeRect/clearRect draw through it (putImageData does not). getTransform waits for QGEOM1. Should fix 50 files (25+25).
- QCNV6 [M] (P2; Sonnet; canvas.rs, js_dom.rs) Paths: the current default path (beginPath, moveTo, lineTo, closePath, rect incl. negative sizes), fill(fillRule)/stroke()/clip(fillRule) with nonzero/evenodd, points mapped by the CTM when added, clip region in the saved state, stroke via tiny-skia `Stroke` with lineWidth, non-finite arguments ignored, empty subpaths. Should fix 144 files (72+72).
- QCNV7 [M] (P2; Sonnet; canvas.rs) Curves: arc (anticlockwise, sweeps over 2pi, negative radius IndexSizeError), arcTo (collinear/degenerate cases), ellipse (rotation), quadraticCurveTo, bezierCurveTo, roundRect (radii: number or DOMPointInit, 1-4 entries, RangeError/negative). Should fix 174 files (87+87).
- QCNV8 [S] (P2; Sonnet; canvas.rs) Line styles: lineCap, lineJoin, miterLimit, setLineDash/getLineDash (odd lists doubled, negative/non-finite rejected), lineDashOffset, through tiny-skia `StrokeDash`; stroke and strokeRect honour them. Should fix 91 files (104 subtests).
- QGEOM1 [M] (P2; Sonnet; new JS prelude next to DOMRect in js_dom.rs; platform, NOT canvas) Geometry Interfaces: DOMMatrixReadOnly/DOMMatrix (2D/3D, fromMatrix, fromFloat32/64Array, multiply, invert, translate/scale/rotate, is2D/isIdentity, string init through the CSS transform parser), DOMPointReadOnly/DOMPoint (matrixTransform). Then canvas getTransform, setTransform(DOMMatrix), roundRect(DOMPoint). Should fix 50 canvas files; css/geometry WPT is not a wpt-th area (adding it: separate QWPT item).
- QCNV9 [S] (P2; Sonnet; canvas.rs) Path2D (empty, copy, SVG path data via svgtypes, already in Cargo.lock), addPath(path, transform), fill/stroke/clip(path), isPointInPath/isPointInStroke (inverse CTM, fill rule, stroke geometry, both overloads). Should fix 64 files.
- QCNV10 [M] (P2; Sonnet; canvas.rs, js_dom.rs) Gradients: CanvasGradient from createLinearGradient, createRadialGradient (full two-point conical incl. r0 > 0 and the cone cases; check tiny-skia's RadialGradient covers it, else a per-pixel shader), createConicGradient (not in tiny-skia: per-pixel), addColorStop (IndexSizeError/SyntaxError), fill/strokeStyle getters return the object, gradient in user space at draw time. Should fix 107 files.
- QCNV11 [M] (P2; Opus; canvas.rs) globalCompositeOperation: the 26 values (Porter-Duff + blend modes) through tiny-skia BlendMode, invalid ignored; the unbounded operators (copy, source-in, source-out, destination-in, destination-atop) clear outside the shape: draw into a temp layer, composite the whole canvas. Should fix 97 files (101 subtests); 74 more compositing files also need QCNV13/QIMGBMP1/QCNV14.
- QCNV12 [M] (P2; Opus; canvas.rs, reuse fx.rs gauss_blur) Shadows: shadowColor/Blur/OffsetX/OffsetY state; drawn when the colour is not transparent and blur or offset is non-zero; offsets unaffected by the CTM, sigma = blur/2; via a temp layer composited under the shape with the current operator; for rects, paths, and later images/text. Should fix 99 files (129 subtests).
- QCNV13 [S] (P2; Sonnet; canvas.rs) drawImage with HTMLCanvasElement/OffscreenCanvas sources (3/5/9 arguments, source-rect clipping, negative sizes, zero-size source, self-draw, InvalidStateError for a 0x0 source, TypeError for bad types), imageSmoothingEnabled/Quality. Should fix 38 files.
- QIMGBMP1 [M] (P2; Sonnet; js_dom.rs/new prelude, images.rs decoders; platform, NOT canvas) ImageBitmap: createImageBitmap from Blob (images::decode_image_bytes), ImageData, canvas, OffscreenCanvas, ImageBitmap, img (after QCNV14's pixel access); crop rect; options (imageOrientation, premultiplyAlpha, colorSpaceConversion, resizeWidth/Height/Quality); close(); resolves in a task; errors; drawImage/createPattern accept it; OffscreenCanvas.transferToImageBitmap. Should fix 109 files (118 subtests) and may settle the four flaky manual/imagebitmap files.
- QCNV14 [M] (P2; Opus; js_dom.rs img state, images.rs, canvas.rs; needs image-pipeline work) Image element sources and patterns: drawImage(HTMLImageElement, SVGImageElement, SVG `<img>`) with the decoded RGBA reachable from the JS thread (today only sizes); not-yet-decoded draws nothing, broken -> InvalidStateError; animated GIF first frame; SVG intrinsic size; createPattern (repeat modes, setTransform) from canvas/ImageBitmap/img; origin-clean flag and SecurityError on read-back. Should fix 148 files (192 subtests).
- QCNV15a [M] (P2; Opus; canvas.rs, css.rs font shorthand, webfonts.rs) Canvas text state and metrics: `font` parse/serialise (default `10px sans-serif`; em/rem/ex/ch against the canvas element/root, incl. the 11 root-font-relative subtests), textAlign/textBaseline/direction (inherit), letterSpacing/wordSpacing/fontKerning/fontStretch/fontVariantCaps/textRendering/lang, measureText -> TextMetrics through cosmic-text, web fonts (CanvasTest via @font-face/FontFace) usable. QCNV15b [M] (P2; Opus) fillText/strokeText: glyph outlines into tiny-skia paths, maxWidth squeeze, alignment/baselines, shadows/composite apply. 15a+15b should fix 192 files (221 subtests); which half fixes which file depends on 15a's pixel-free subtests.
- QCNV16 [S] (P2; Sonnet; canvas.rs, js_dom.rs) toDataURL/toBlob/convertToBlob: PNG (tiny-skia encode_png on unpremultiplied data), JPEG with quality (image crate), other types -> PNG, 0-size -> `data:,`, toBlob callback as a task, convertToBlob promise, tainted -> SecurityError. Directly 1 file here; needed by QCNV20 and the `toDataURL -> <img>` tests (QCNV1 note).
- QCNV17 [S] (P2; Sonnet; canvas.rs, js_dom.rs) reset() (bitmap, state stack, path, CTM, clip), getContextAttributes, `{alpha: false}` (opaque black), willReadFrequently, desynchronized, isContextLost() false, drawFocusIfNeeded (focused fallback element). Should fix 75 files (102 subtests).
- QCNV18 [M] (P3; Opus; canvas.rs, fx.rs) ctx.filter: CSS filter functions on a temp layer (reuse fx.rs), `none`, invalid ignored, url(#id) SVG filters (or split off), CanvasFilter (tentative). Should fix 24 files (13 tentative).
- QCNV19 [M] (P3; Opus; canvas.rs) beginLayer/endLayer (a proposal): layer stack carrying alpha/composite/shadow/filter, errors for unbalanced calls and reset inside a layer. Should fix 41 files (51 subtests).
- QCNV20 [L] (P3; Opus; canvas.rs, css.rs; needs colour management) Colour spaces: getContext colorSpace 'display-p3', ImageData colorSpace conversion on get/put, colorType 'float16' + ImageData pixelFormat (needs Float16Array in V8), toBlob colour conversion, globalHDRHeadroom. Should fix 49 files (1,173 subtests, mostly 6 manual matrix files).
- Not filed: OffscreenCanvas in Workers (needs Worker support; DECISIONS lists Workers at lower priority) — 14 files here plus the 1,040 NOT-RUN worker variants; text clusters/shaping and HTML-in-canvas (tentative proposals; 156 files); colour objects/CSSHSL (Typed OM); video (media archived).

### QWPTBUD: per-file callback budgets for the runner (wpt-th/budgets.txt)
Range-insertNode, Range-surroundContents, Range-mutations-replaceData and Range-mutations-dataChange run all their cases in one callback, and the 500 ms thread-CPU cut fell at a load-dependent point. `wpt-th/budgets.txt` (`path ms reason`) gives a listed file's `--wpt-th-one` child a larger `--budget-ms` (the parent adds it per file; real-time kill limit at least budget + 45 s); every unlisted file, the App and every other mode keep 500 ms, so a runaway loop elsewhere still dies fast. (A first try, a blanket 60 s default for all runner children, made 6 NodeList-static-length-getter-tampered files and Event-timestamp-safe-resolution burn 30-60 s each: rejected.) `--wpt-th --budget-ms N` forwards N to every child without an entry. With 60 s: insertNode 1840/1840, surroundContents 1840/1840 and replaceData 573/1146 were identical in 10 sequential runs and 3 rounds of 4 parallel runs, so they left flaky.txt. dataChange (2808 subtests, 35-60 s of CPU) is stable alone but was cut at 60 s in 3 of 12 runs under 4 parallel runs, so it stays in flaky.txt and gets no budget.

QGEOM1: Geometry Interfaces (`src/geometry_prelude.js`, concatenated before the DOM prelude: `DOM_PRELUDE = concat!(include_str!(..), r#".."#)`). State lives in per-interface WeakMaps (brand checks throw `Illegal invocation`; the read-only classes share the stores of the mutable ones, `rw` flag). DOMMatrix keeps 16 numbers in m11..m44 order (column-major, P[r][c] = m[4c+r]) plus `is2D`; 2D x 2D products touch only a..f (no 0 x Infinity NaNs). Strings (constructor, setMatrixValue) go through a small CSS transform-list parser in the same file: px/absolute lengths, deg/rad/grad/turn, `calc()` of same-typed terms, comments; relative units, `var()` etc. are SyntaxErrors; any 3D function makes is2D false. 4x4 inverse is the cofactor form (a Gauss-Jordan one left 1e-17 noise that WPT's isIdentity check rejects). The DOM prelude takes `GEOM = g.__scratchy_geom` (and deletes it) for DOMRect construction and canvas getTransform/`DOMMatrix2DInit` fixup, so a page overwriting the globals cannot change them. `WebKitCSSMatrix` is an alias of DOMMatrix; `scaleNonUniform` exists (WPT DOMMatrix-002/003 want it). wpt-th area `css/geometry` (shard `rest`): not done: `DOMRectList` (getClientRects still returns an Array), structured clone of the geometry classes, canvas `addPath`/Path2D (QCNV9), idlharness.

### QWPTFLK5: request-upload.any.html (wptserve) and gate-line messages
`fetch/api/basic/request-upload.any.html` failed once on CI (PR #624, first attempt): "subtest was passing, now FAIL: Fetch with POST with Uint8Array body", green on a re-run without a change. Not reproduced locally against a pinned checkout's wptserve (release build, `--wpt-th-one` per run): 30 sequential runs (0.77-0.83 s each), 120 runs as 4 parallel loops (0.60-0.87 s), 300 runs as 12 parallel loops (mean 0.76 s, max 1.20 s, load ~8): all 450 byte-identical (16/18, the two recorded FAILs). 4,000+ POSTs straight through wire to echo-content.py (up to 8 threads, random gaps up to 3 s, interleaved with GETs of resources wptserve closes after) all came back 200 with the exact body, so wire's framing and pool are not the proven cause. Suspect, unproven: ureq 2.12 retries a request on a pooled connection the server closed only when the method is idempotent (`Unit::is_retryable`), so a POST that picks a connection wptserve just closed fails once as a transport error (fetch: TypeError); wptserve closes after every response without Content-Length (all static files). The file is in `flaky.txt`. To make the next occurrence diagnosable, REGRESSION/CHANGED subtest lines of the runner now carry the subtest's failure message (`FileResult.msgs`: diagnostics only, never recorded or compared). The other #615 flaky entries were not re-investigated: their symptoms (whole-file counts, NOTRUN/TIMEOUT) do not match a single failed POST.
