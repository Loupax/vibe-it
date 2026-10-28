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
