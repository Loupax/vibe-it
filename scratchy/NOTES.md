# scratchy — session notes

Not part of the prompt log (`PROMPTS.md`) — this is working notes for
picking the project back up later. `PROMPTS.md` stays the append-only
record of exact feature prompts; this file is free-form and can be
edited/pruned as the project evolves.

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

## Working notes for next session

- **Verification tests are now kept permanently**, in `src/lib.rs`'s
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

**Resolving a `PendingPositioned` entry** (`resolve_positioned`) turns
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
of a negative deficit, single-pass, not the real spec's iterative
freeze-and-retry algorithm — see `TODO.txt`) and `distribute_justify_
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
