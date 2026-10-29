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
