# scratchy — regressions

## R1–R4: white screen past one viewport, hover-driven relayout storms, slow scrolling — ALL FIXED (2026-11-13)

**Discovered:** 2026-11-13T09:16:02+01:00 (investigation on `main` @ `530f741`)
**Reported symptoms (on https://en.wikipedia.org/wiki/Main_Page):**
page flickers seemingly without interaction; mouse-wheel scrolling is
painfully slow; scrolling down ~two pages gives a white screen, and
scrolling back up renders again (it's not a crash).

There are three separate bugs here. Two are regressions with a known
introducing commit. The third is an old hotspot that Q31 made hit much
more often. All numbers below were **measured** with a temporarily
instrumented release build (see "How this was measured"), not guessed.
Window was 1052×1050.

---

### R1 — White screen after scrolling past the first viewport (regression, Q14 `35c826a`) — FIXED

**Fix:** `Laid::ink` (`Ink`, computed once by `compute_ink` at the end of
`measure_document`) is the union of a box and everything it can paint
(visible overflow, positioned descendants, sticky ranges; clipping boxes
contribute only their own rect for in-flow content; `fixed` boxes are
not unioned into parents). `paint_inner` culls on it, and `hit_test_node`
(now `hit_test_inner`, returning `Option`) / `hit_test_scroll_region`
prune on it while a box itself only counts as a hit inside its own
border box. Trees not passed through `measure_document` carry
`Ink::UNKNOWN` and never prune. Tests: `paint_past_first_viewport_still_
draws_with_height_100_percent_body` (fails without the fix),
`hit_test_finds_link_below_first_viewport_with_height_100_percent_body`,
`zero_height_list_with_visible_children_still_paints_and_hit_tests`.
Not added: dedicated `fixed`/`sticky` cull tests (the formula is
unchanged for them; existing sticky/fixed tests still pass).

**Measured:** with synthetic scrolling at 30px/frame, `paint` took
~10ms/frame up to `scroll_offset ≈ 900`. From `scroll_offset ≥ ~1050`
it took **~7µs/frame**, so it was painting nothing at all. That's the
white screen.

**Root cause:** Wikipedia's CSS has `html, body { height: 100% }`. Since
Q14 (`35c826a`, "viewport units and initial-containing-block height"),
that resolves against the ICB, so `<html>` and `<body>` lay out at
**exactly h=1050** (the viewport). Their content overflows visibly down
to y≈3809 (in-flow) and y≈24443 (an absolutely positioned
`.vector-dropdown-content`). Q14 raised the *synthetic root's* `h` to
`in_flow_bottom` so scrolling still works, but left `<html>`/`<body>` at
1050.

`paint_inner` (search for `Entirely outside the viewport`) culls a
whole subtree with:

```rust
if draw_y + laid.h < 0.0 || draw_y > win_h as f32 { return; }
```

Its comment assumes "every descendant's y-range is contained within
this box's". **That invariant is false in CSS.** Visible overflow,
positioned descendants, negative margins and fixed-height boxes all
break it. Once `scroll_offset > 1050`, `<body>` is "off-screen" and the
entire page gets culled. Scrolling back up brings it back.

A dump of boxes whose descendants extend past their own bottom edge
found 93 violators on Main_Page, including `<html>`, `<body>`,
`.mw-page-container`, `main#content`, `#bodyContent`, flex headers with
dropdowns, and `ul`s with `h=0`. So this isn't only a `height:100%`
problem. Any page with a fixed-height wrapper hits it.

**Same bug, second place: hit testing.** `hit_test_node` (and
`hit_test_scroll_region`) prune with
`if x < laid.x || x >= laid.x + laid.w || y < laid.y || y >= laid.y + laid.h { return … }`.
On Wikipedia this means hover, the link cursor and **link clicks below
y=1050 never reach the target element**. Not observed by hand yet, but
it follows directly from the same geometry. Verify it while fixing.

**Fix plan:**
1. Add a precomputed descendant extent to `Laid`, e.g.
   `ink_top: f32, ink_bottom: f32` (and `ink_left/right` for the hit-test
   x check). Set it once when the `Laid` is built at the end of
   `measure_document` (or in one post-pass over the finished tree):
   union of own rect, every `Blocks`/`TableRow` child's extent and every
   `positioned` entry's extent. `in_flow_bottom` (near `measure_document`)
   is a similar walk to crib from. Don't recompute it per frame.
2. In `paint_inner`, cull on the extent instead of `laid.y/laid.h`:
   `if ink_bottom - scroll_offset < 0.0 || ink_top - scroll_offset > win_h { return; }`.
   Keep it conservative. If a box clips (`overflow` hidden/scroll/auto),
   its descendants are clipped to it anyway, but don't optimise that
   until it's correct.
   - `fixed` subtrees: `paint` is called with `scroll_offset = 0.0` for
     them, so the same formula holds. `sticky`: the caller already
     adjusts `scroll_offset` (`sticky_shift`), so it holds there too.
     Add a test for each anyway.
3. In `hit_test_node` / `hit_test_scroll_region`, prune on the extent
   instead of the border box. The *hit itself* must still require the
   point to be inside the element's own rect. Only the "can any
   descendant be under this point" early-out changes.
4. Fix the misleading comment in `paint_inner`.
5. Tests (keep them, per project rules): `html,body{height:100%}` with
   ~3 viewports of paragraphs. Paint at `scroll_offset = 2 × viewport`
   and assert non-white pixels exist (`paint_fixture`-style). Hit-test a
   link at y = 2.5 × viewport and assert the href comes back.
   Also test an `h=0` `ul` with visible children.

---

### R2 — Every hover change re-runs the whole style + layout pipeline (regression, Q31 `a4b93c4`) — FIXED (items 1–4)

**Fix:** (1) `AppEvent::ShapeResult` now carries `incomplete`; a restyle
(`restyle_requested`/`hold_old_frame` on `App`) keeps painting the previous
complete `Laid` + its `ShapeCache` and swaps once, when the new
generation completes, so no flicker and no scroll yanking (the clamp only
ever sees complete trees). Navigation (`replace_document`) clears the hold.
(2) Content-keyed shaping memo on the worker thread (`SHAPE_MEMO`, keyed
by hash of text + span styles + width + flags at the paragraph, word and
chunk sites); `ShapingRequest::reuse_shaping` keeps it across restyles of
the same document/width and clears it otherwise. (3) Hover restyles are
throttled (150ms apart, none while a restyle is in flight, 200ms quiet
after a wheel tick) via a deferred `AppEvent::HoverFlush`. (4) `HoverKeys`:
only hover-chain changes touching an element a `:hover` compound could
match restyle at all. Tests: `shape_memo_reuses_unchanged_paragraphs_
across_trees`, `hover_keys_cover_exactly_the_elements_a_hover_rule_can_
restyle`. **Not verified in the live window** (the app can't be driven
headlessly); the App-level hold/throttle wiring has no unit test.

**Measured:** with a synthetic hover sweep (cursor moving down the
page), every element change under the pointer logged
`INTERACTION changed=0x1 -> relayout`, then a full rebuild:
**`build_layout_tree` 1.09–1.15s on the main thread, every time**.
That's 13 relayouts in ~10s, and frame rate dropped to roughly one frame
per second. After each rebuild, the page's laid-out height jumped
(3939 → 2798) because the worker restarts shaping from an **empty**
`ShapeCache` (`spawn_shaping_worker`: `let mut shape_cache = ShapeCache::new()`
per request). The first progressive `ShapeResult` replaces the fully
shaped `self.cached` with a partial tree, so text below the shaped
region vanishes and refills. **That's the flicker.**

It looks like it happens "without interaction" because any tiny pointer
movement (or a compositor-sent motion event when the window gets focus
or the layout changes) that crosses an element boundary is enough.
Wikipedia has plenty of `:hover` rules, so `state_rules & STATE_HOVER`
is set. Also, when a partial `Laid` with a smaller `h` arrives,
`redraw` clamps `self.scroll_offset` to the smaller `max_scroll`, so
**the scroll position gets yanked upward** during a hover relayout.
Scrolling then feels sluggish or jumpy.

`set_interaction_state` sets `layout_dirty` + `keep_scroll_on_rebuild`
→ `redraw` → `build_layout_tree_with_resources` (full CSS parse +
`precompute_generated` + full cascade) → new `layout_generation` →
reshape from scratch. Q31's own docs say "pages without `:hover` rules
pay nothing". That's true, but real pages always have them.

**Fix plan (in order; ship 1+2 first, they're what fixes the reported
symptoms):**
1. **Never drop the old `cached` for a partial result of a restyle.**
   (User confirmed 2026-11-13 that hover effects appearing slightly late
   is fine in exchange for no flicker.)
   For interaction-driven rebuilds, keep painting the previous complete
   `Laid` until the new generation's worker reports `incomplete == false`.
   Then swap it in once. That needs `incomplete` sent in
   `AppEvent::ShapeResult`; today it isn't. Navigation and first load
   keep the current progressive behaviour. Also stop clamping
   `scroll_offset` against a partial tree's `h` (or only clamp once the
   generation is complete).
2. **Reuse shaping work across restyles.** Hover almost never changes
   text or fonts. Let `ShapingRequest` carry the previous generation's
   `ShapeCache` when the request is a restyle of the same document and
   width. The worker can then reuse every paragraph whose key is
   unchanged. The ids are positional (`next_id` restarts at 0 per tree),
   so reuse is only valid if the cached entry also matches on content.
   Key on (text, attrs/span styles, width) or store a hash in
   `ShapedParagraph` and verify before reusing. Measure: a hover relayout
   should go from multiple worker ticks to ~1.
3. **Throttle/coalesce hover restyles.** Don't restyle on every
   `CursorMoved`. Restyle at most once per N ms, and skip entirely while
   a wheel scroll is in progress, using the latest hover chain. Cheap
   and independent of 1–2.
4. (Bigger, later) Only restyle when the hovered chain actually matches
   a `:hover` selector differently. Record which elements
   could match a `:hover` rule (subject or ancestor compound) during the
   cascade. If neither the old nor the new hover chain contains one,
   don't relayout. On Wikipedia most hover changes are between
   elements no `:hover` rule targets, so this could remove most
   relayouts outright.

---

### R3 — `build_layout_tree_with_resources` costs ~1.1–1.5s on Wikipedia, on the main thread (pre-existing; amplified by R2) — FIXED (items 1, 3, 4; item 2 only partly)

**Measured after** (`SCRATCHY_BENCH_PAGE=<saved Wikipedia main page, 1.2MB
html> cargo bench --bench layout -- real_page`; that file carries its own
inline CSS, not the 218KB external sheet, so absolute numbers differ from
the table below): **1.27s -> 0.42s** for `build_layout_tree_sized`.
**Fix:** (1) parsed rules cached by css source (`RULES_CACHE`, which also
caches the `STATE_*` mask the parse records). (3) `RuleIndex`: rules
bucketed by id/class/tag of each selector's rightmost compound, separate
buckets for normal vs pseudo-element selectors, candidates merged in
source order, registered per build (`RULE_INDEX`, matched by slice
identity so other callers fall back to a full scan). (4) `benches/layout.rs`
gains `real_page` driven by `SCRATCHY_BENCH_PAGE` (page not committed, too
large). (2) `precompute_generated` still walks the whole document once;
it now benefits from the index (its `::before`/`::after` lookups are
nearly free) but was not folded into the main walk. Tests:
`rule_index_returns_the_same_matches_as_a_full_scan`,
`cached_stylesheet_still_reports_state_rules`.

**Measured** (same page, 218,654 bytes of CSS, 1146 rules):

| phase | time |
|---|---|
| `parse_stylesheet` (default + page CSS) | ~17ms |
| `precompute_generated` | 350–400ms |
| `build_children` (cascade + box tree) | 760–970ms |
| **total** | **1.1–1.5s** |

The pre-Q31 commit `b1e6a09` measured the same (~1.45–1.49s), so this
is **not new**. Before Q31 it ran only on load, resize, stdin chunk or
image arrival. Now every hover change triggers it (R2), and each
`AppEvent::ImageFetched` triggers one too. On Main_Page all 24 images
arrived in one batch and coalesced into one rebuild. A page whose
images trickle in gets a ~1.4s freeze per straggler.

**Fix plan:**
1. Cache the parsed `Vec<StyleRule>` on `App`, keyed by
   `(css_source, viewport_width)` or invalidated by
   `replace_document`/chunk/`extra_css` change. Restyles reuse it.
   Saves ~17ms (small, but trivial to do).
2. `precompute_generated` does a full second cascade over the document
   whenever *any* rule has `content`/counters/quotes, which is always
   true on real pages. Either fold it into the `build_children` walk or
   only resolve styles for elements that a counter/quote/content rule
   can actually match. Measure before and after.
3. Selector matching in the cascade: index rules by rightmost compound
   (id / class / tag / universal) like real engines do, so each element
   only tests candidate rules instead of all 1146. Likely the biggest
   win inside `build_children`. Measure first: split `build_children`
   into `resolve_style` time vs box-building time.
4. Add a `benches/` target that runs `build_layout_tree_with_resources`
   on a saved Wikipedia page + CSS (fetch once via `wire`/`snag`, commit
   as a fixture). Then this number gets tracked instead of rediscovered.

---

### Secondary observations (not the reported symptoms; lower priority)

- Static paint at scroll 0 during load was 15–30ms/frame on Main_Page,
  then ~10ms once settled. Acceptable, but watch it once R1's
  culling uses the larger extents (more subtrees pass the cull).
- `.vector-dropdown-content` (absolutely positioned, `h=32`, clipping)
  has descendants laid out to y≈24443. Likely a dropdown menu real
  browsers hide via the checkbox hack (`:checked ~ …` / `display:none`).
  Harmless while it clips, but it inflates any extent computed in R1.
  Check that it doesn't make the "scrollable height" or the cull useless.
- Q14's height-change rebuild was moved up to **R4** below (user decided
  2026-11-13 to fix it).

---

### R4 — Height-only window changes trigger a full rebuild (regression, Q14 `35c826a`) — FIXED

**Fix:** `VIEWPORT_H_READ` is set while building when a `vh`/`vmin`/`vmax`
unit or a `%`-calc resolves, or (static tree walk,
`tree_reads_viewport_height`) when any box has a percentage
height/min/max-height, `position: fixed`, or an orphaned `position:
absolute`. `redraw` skips the rebuild for a height-only change when it is
clear (`reads_viewport_h`); otherwise the rebuild is a restyle (held old
frame + shaping memo). Test: `viewport_height_flag_only_set_when_layout_
can_depend_on_it`. Note: Wikipedia (`html,body{height:100%}`) still
rebuilds on a height change, but through the cheap restyle path.

Q14 made `redraw` rebuild whenever the window **height** changes
(`self.cached_height != height` in `need_rebuild`). On a tiling WM,
height-only changes are routine (another window opens, a bar toggles).
Each one is now a full ~1.4s main-thread rebuild plus a reshape from
scratch, which means freeze + flicker (same mechanics as R2).

Only a few things depend on viewport height: `vh`/`vmin`/`vmax`,
`%` heights that chain up to the ICB (`html,body{height:100%}`), and
`position: fixed` / orphaned-absolute containing blocks (`viewport_h`
in `measure_document`).

**Fix plan:**
1. While building the tree, record whether anything actually read the
   viewport height. Use a thread-local flag like `STATE_RULES`, set when
   resolving a `Length::Vp` with a height component, when `icb_height`
   is used as a containing height, or when `viewport_h` is used for a
   fixed/orphaned-absolute box. Store it on `App` next to `state_rules`.
2. In `redraw`, only treat a height change as needing a rebuild if that
   flag is set. Otherwise just update `cached_height`, re-clamp
   `scroll_offset` and repaint.
3. When it *does* need a rebuild, treat it like a restyle: hold the old
   frame (R2.1) and reuse the shape cache (R2.2). Text wrapping depends
   only on width, so with R2.2 nearly every paragraph should come back
   from the cache.
4. Test: a page with no `vh`/`%`-height rules doesn't bump
   `layout_generation` on a height-only resize. A page with
   `html,body{height:100%}` does.

---

### Suggested order for the fixing session

1. **R1** (white screen + unclickable links). Self-contained, highest
   user impact, easy to test headlessly.
2. **R2.1 + R2.3** (keep old frame until restyle completes; throttle
   hover). Kills the flicker and the scroll-position yanking.
3. **R2.2** (reuse shape cache across restyles).
3b. **R4** (skip height-only rebuilds unless something reads the viewport
   height; otherwise route them through R2.1/R2.2).
4. **R3.1–R3.4** (make the restyle itself cheap), each measured.
5. **R2.4** (selective `:hover` invalidation).

Each step: `cargo check`, `cargo clippy`, `cargo test`, keep the new
tests, `cargo install --path .`, then re-run the measurement below
against Wikipedia Main_Page to confirm the numbers moved.

---

### How this was measured (reproduce before and after each fix)

Done in a throwaway `git worktree` (not committed). Temporary
`eprintln!` + `Instant` lines were added to:
- `App::redraw`: log each rebuild (reason: `layout_dirty`, width/height
  change, new generation) and `build_layout_tree_with_resources`
  duration. Log per frame: `paint` duration, `scroll_offset`, `cached.h`,
  generation.
- `user_event`: `ImageFetched` and `ShapeResult` (generation, cache size,
  `laid.h`).
- `set_interaction_state`: when it marks `layout_dirty`.
- `build_layout_tree_with_resources`: phase split (parse,
  `precompute_generated`, `build_children`).
- Synthetic input via env vars, since the window can't be driven
  externally: `AUTOSCROLL=1` (request a redraw every frame; after 15s
  add 30px to `scroll_offset` per frame) and `AUTOHOVER=1` (after 15s
  call `update_hover(element_at((300, y)))` with `y` sweeping the window
  each frame).
- `DUMP=1`: one-off walk of the `Laid` tree printing every box whose
  descendants extend below its own bottom edge, with tag/id/class (via
  `find_node_by_id`). That's how R1's cause was pinned to
  `<html>`/`<body>` h=1050.

Run with `timeout 45 target/release/scratchy --socket /run/user/1001/wire.sock https://en.wikipedia.org/wiki/Main_Page 2> log`.
Use `--release`, since the installed binary is a release build. Remove
all instrumentation before committing.

---

## WPT audit (Q40, opened 2026-11-14)

Findings from running `/home/loupax/src/wpt` reftests against features
TODO.txt marks done. Format per area: counts, failure groups by root
cause (example tests, wrong render), claiming Q item + hash, triage
(BROKEN / PARTIAL / NOT CLAIMED). See Q40b in TODO.txt.

### How the audit was run (Q40b)

- Runner: `scratchy --wpt /home/loupax/src/wpt --subset LIST` at 800x600 with Ahem
  loaded. Counts below are over EVERY eligible reftest in the area (a file with
  `rel=match|mismatch`, not a `-ref`, no `<script>`/`reftest-wait`), taken from
  throwaway lists, not from the committed `wpt-subset.txt`. The committed subset
  gets ~12 evenly spaced tests per area as a regression guard.
- Two runner fixes were needed first, both in `src/wpt.rs` (not engine bugs):
  `<![CDATA[` markers in `.xht` `<style>` corrupted the first rule (b86c983),
  and root-relative reference hrefs (`/css/reference/...`) counted as CRASH (6d523e5).
  Counts here are after both. There are no crashes left in the audited areas.
- "pass" can be vacuous: a test and its reference that both render wrong the same
  way match. Pass counts are an upper bound on correctness. Fail counts are exact.
- "flipped" = tests that fail raw but pass when a scratch copy of the test tree
  has (a) `in/cm/mm/pc` lengths rewritten to `px` and (b) `url(file)` images
  inlined as `data:` URIs. It measures how much a failure area is hidden behind
  W1 and W19 below. The copy lives outside the repo.
- "marker counts" (`tests mentioning X among the failures`) are grep upper bounds,
  not root-cause counts. A root cause is only listed as a group after a minimal
  repro was rendered and the wrong output seen.
- `.xht` parsed as HTML, so inline `svg:svg` and other namespaced XML do not work
  for the wrong reason in some tests (noted under W20).
- Not reproduced: the Q40a lead "Ahem `<p>` lands 2px lower than the same content
  in a `<div>`". With margins zeroed, `<p>` and `<div>` Ahem text land on the same
  row. The apparent offset is the UA `p` margin (1em). Dropped.

### Root causes found (referenced from the area sections)

Each has a minimal repro that was rendered with the runner. Claim = the Q/history
item that says the feature is done.

| id | what is wrong | claimed by | triage |
|----|---------------|------------|--------|
| W1 | `in`, `cm`, `mm`, `pc`, `Q` lengths are not parsed (declaration dropped). `width:1in` renders full width. | Q42 (b79a100) | FIXED (also `padding` rejects negative lengths) |
| W2 | `ex`, `ch`, `ic`, `lh`, `rlh`, `cap` units not parsed. | Q42 (b79a100) for `ex`/`ch` (half an em) | PARTIAL (rest: Q42a) |
| W3 | `<br>` is not implemented anywhere (`grep '"br"' src/*.rs` is empty). `a<br>b` renders `a b` in a block, float, inline-block, abspos. | Q41 (09b8e63) | FIXED (leftovers: Q41a) |
| W4 | `border: solid black` / `border-style: solid` with no width computes width 0; should be `medium` (3px). `border-bottom: solid` likewise. | Q4 3e80aae | BROKEN |
| W5 | Negative values that CSS says are invalid are accepted: `padding-right:-1px` shrinks the box, `max-height:-1px` clamps to 0 (should be ignored), negative `flex-basis`, negative `border-*-width`. | pre-Q box model | BROKEN |
| W6 | A block with `width:auto` whose borders+padding exceed the containing block is clamped to the container width; it should overflow (48px container, 2x32px borders paints 48 wide, expected 64). | pre-Q box model | BROKEN |
| W7 | Vertical margins do not collapse through an empty block (`height:auto` with no content, or `height:0`): `margin-bottom:50` + empty div + `margin-top:30` gives 80, expected 50. | pre-Q box model | BROKEN |
| W8 | A float that follows inline content on the same line (`<div style=inline-block></div><div style=float:left>`) is placed below the line box instead of at the top of the current line beside it. | float 5fc83ce, Q34 | BROKEN |
| W9 | Shrink-to-fit width (float, inline-block, table, abspos) is wider than the text: with Ahem 20px, `float:left` "X" is 40px wide instead of 20 (+1em); with sans 20px "PA" is +3px. Adjacent floats of text show a visible gap. | float 5fc83ce, inline-block 244a8f4 | Q43 (c8e334c) removed the extra trailing space; PARTIAL (Q43a) |
| W10 | `position:absolute` with `width:auto` and `left`/`right` auto/one side set stretches to the containing block edge (to x=800) instead of shrink-to-fit. | position bb97e60 | FIXED Q44 (0e42655); margins with left+right stretch remain (Q44a) |
| W11 | An abspos box (auto height) whose only children are floats gets height 0 and paints no background (an abspos box is a BFC and must contain floats). `overflow:hidden` on it hides the bug. | position bb97e60, Q34 | BROKEN (Q40a lead, confirmed) |
| W12 | Flex items with an explicit `width` keep it when growing/shrinking: siblings move as if the item grew, but it paints at its specified width. `flex:1` items without `width` are fine. | flex 440e03b | BROKEN |
| W13 | Single-line flex container with a definite `height`: cross size is taken from the tallest item, not the container. `align-items:center|flex-end` aligns inside the tallest item. | flex 440e03b | BROKEN |
| W14 | A block-level `display:table` element (no `float`, no `width`) fills the container and spreads its cells across it (two 40px cells land at x=0 and x=400); it should shrink-to-fit. Only the floated case works (Q34c acbcb00). | table 53bab7a, Q34c | BROKEN |
| W15 | A table's `width` is content-box here; browsers (and WPT refs) treat it as including the table's border+padding. `table{width:100%;border:1px}` paints 2px too wide. | table 53bab7a | BROKEN |
| W16 | No `border-collapse`, `border-spacing` or `table-layout:fixed` support (NOTES.md says so). | table 53bab7a scope cut | PARTIAL |
| W17 | A `z-index:-1` positioned child of a non-stacking-context positioned parent paints above the parent's background; it should paint below it (in the ancestor stacking context's negative layer). | none explicit | BROKEN |
| W18 | `<img width="100%">` (percentage in the HTML attribute) is treated as unset; image paints at `height` x intrinsic aspect (20px for `height=20`). | image support | BROKEN |
| W19 | `background-image`/`mask-image` `url()` only decodes `data:` URIs; file/network URLs never load in `resolve_background_image`. | Q16 93209c2 (documented) | PARTIAL |
| W20 | Inline `<svg:svg xmlns:svg=...>` (prefixed) and `<object data=x.svg>` are not rendered. | Q12 af12757 (`<img>` SVG only) | NOT CLAIMED |
| W21 | `:has()` never matches; `:nth-child(n of S)`, `:host`, `:is/:where` with namespaces not handled. | Q2 66bdc60 lists neither | NOT CLAIMED |
| W22 | `var()`: (a) invalid-at-parse values (`var(--b,!)`, `;`, `!important !important`) are accepted; (b) invalid-at-computed-time resolves as "declaration dropped" instead of `unset`; (c) escaped custom property names (`--\30`) do not resolve; (d) adjacent `var(--b)var(--c)` fails. | Q13 b23fd53 | BROKEN |
| W23 | `list-style-type:"string"` and `<ol reversed>` are ignored (shows `1.`, `2.`). | Q19f cfa71ec (counters only) | NOT CLAIMED |

#### CSS2/normal-flow (claimed: baseline box model, no Q item)

698 eligible (+53 skipped for JS) | pass 313 | fail 384 | 1 no-reference.
Flipped by the W1+W19 rewrite: 189, so about half of all failures are W1 alone
(`1in` squares). Residual after the rewrite: 205 failures.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W1 units | 189 flipped (`max-height-006`, `height-006`, `width-applies-to-*`) | `width:1in` ignored, box full width | NOT CLAIMED |
| W5 negative values | 13 mention one (`max-height-001`, `max-height-012`) | `max-height:-1px` collapses a 96px square to 0 | BROKEN |
| W6 over-constrained width | `blocks-011`, `blocks-012` | box 48px wide instead of 64 | BROKEN |
| W7 empty-block margins | `margin-collapse-through-zero-height-block`, `margin-collapse-after-float-001` | gap 300 instead of 200 | BROKEN |
| W3 `<br>` | 17 mention it (`width-percentage-001` reference) | two images in the reference sit side by side | FIXED by Q41 (09b8e63) |
| W9 shrink-to-fit | `inline-block-non-replaced-width-001` | red strip: box one em too wide | BROKEN |
| W8 float after inline | `inlines-013` | float lands on the next line | BROKEN |
| W11 abspos + float | `block-formatting-context-height-001` (Q40a lead) | paints nothing | BROKEN |
| W20 `<object>` svg | `replaced-intrinsic-*` (5) | nothing drawn | NOT CLAIMED |
| ungrouped | 28 failures with none of the markers, plus `inline-table-zorder-*` (5), `inline-block-zorder-*` (3) | not yet root-caused | see Q40c |

#### CSS2/margin-padding-clear (claimed: baseline box model; float/clear 5fc83ce)

684 eligible (0 JS) | pass 277 | fail 406 | 1 no-reference. Flipped 198 (W1:
`1in`, `5em` margins). Residual 224.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W1 units | 198 flipped | `padding:1in` ignored | NOT CLAIMED |
| W5 negative padding | 33 mention a negative length (`padding-right-001`, `-012`) | `padding-right:-1px` shrinks the box by 1px | BROKEN |
| W7 empty-block margins | `margin-collapse-012`, `margin-collapse-022` | 20px gap instead of 60 | BROKEN |
| W14/W16 table internals | 52 mention `display:table-*` (`margin-applies-to-001..015`) | table drawn full width, no border-spacing | BROKEN/PARTIAL |
| W9/W10 | abspos/float references | | BROKEN |
| ungrouped | 43 with no marker (`margin-collapse-001`: inline margins in Ahem rows) | not yet root-caused | Q40c |

#### CSS2/box-display (claimed: baseline; inline-block 244a8f4; flow-root a4852af)

86 eligible (+34 JS) | pass 44 | fail 42. Flipped 6.

| group | tests | wrong render | triage |
|-------|-------|--------------|--------|
| `display:table-*` | `display-*` 12 (12 mention table internals) | W14 | BROKEN |
| containing block | `containing-block-*` 9 | abspos boxes stretch (W10) | BROKEN |
| block-in-inline | `block-in-inline-*` 8 (`block-in-inline-margin-with-leading-text`) | margins around the split differ | BROKEN (not confirmed on a minimal repro, see Q40c) |
| residue | 12 with no marker | | Q40c |

#### CSS2/floats + floats-clear (claimed: float 5fc83ce, Q34)

324 eligible (+12 JS) | pass 69 | fail 255. Flipped 37 (W1).
Marker counts among failures: 212 mention `float`, 57 `inline-block`, 40 `position:absolute`.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W8 float after inline | `floats-001` | blue float lands below the black box instead of at its left inner edge | BROKEN |
| W9 shrink-to-fit | `float-non-replaced-width-*` (6), `float-replaced-width-*`, `clear-applies-to-000` | `PA SS` instead of `PASS` | BROKEN |
| W14 table internals | `float-applies-to-*` (12) | `display:table-row-group; float:right` | BROKEN |
| W1 units | 37 flipped | | NOT CLAIMED |
| clearance | `clear-clearance-calculation-*` (5), `margin-collapse-clear-*` (10) | spacing after a cleared block differs; not root-caused | Q40c |
| ungrouped | 4 with no marker | | Q40c |

#### CSS2/positioning + css-position (claimed: position bb97e60, sticky 636f369)

CSS2/positioning: 513 eligible (+7 JS) | pass 154 | fail 359. Flipped 138 (W1).
css-position: 134 eligible (+119 JS) | pass 38 | fail 96. Flipped 0.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W1 units | 138 flipped | | NOT CLAIMED |
| W10 abspos auto width | `absolute-non-replaced-width-001`, `-018` | box fills the containing block instead of shrink-to-fit | BROKEN |
| W11 abspos + float child | `block-formatting-context-height-001` (normal-flow) | abspos box paints no background | BROKEN |
| W20 inline `svg:svg` | 26 mention svg (`absolute-replaced-width-009`, `-032`) | svg missing | NOT CLAIMED |
| W18 `<img width="100%">` | 11 | | BROKEN |
| css-position | `position-absolute-in-inline` 12, `inline-level-absolute-in-block-level-context` 6, `position-relative` 8, 38 mention `<br>` (W3), 6 `position-sticky-*` | abspos inside inline parents, static position | BROKEN (re-measure after W10) |
| vertical writing modes | ~30 `vrl-*`/`vlr-*` in css-position | | NOT CLAIMED |

#### CSS2/tables + css-tables (claimed: table 53bab7a, Q34c acbcb00; Q36 colspan/rowspan is in flight elsewhere)

CSS2/tables: 250 eligible (+123 JS) | pass 36 | fail 214.
css-tables: 145 eligible (+31 JS) | pass 43 | fail 102.
Measured before Q36 (colspan/rowspan) merged. Flipped 0 in both (units are not the issue here).

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W16 no `border-collapse`/`border-spacing`/`table-layout:fixed` | CSS2/tables: 56 `border-collapse`, 85 `table-layout`, 112 `border-spacing` (`collapsing-border-model-001`, `fixed-table-layout-*` ~50); css-tables: 40 `border-collapse` (`collapsed-borders-painting-order` 13) | table border and cell border both painted, widths from content | PARTIAL (documented scope cut) |
| anonymous table objects | `table-anonymous-objects-*` 61 (`-017`, `-059`) | loose cells/rows are not wrapped in an anonymous row/table; cells overlap | NOT CLAIMED (Q34c covers `display:table-caption` only) |
| W14 block `display:table` | many `border-*-applies-to`, `float-applies-to` | fills the container | BROKEN |
| W15 table width | `block-formatting-contexts-001` reference | 2px too wide | BROKEN |
| caption-side | `caption-side-applies-to-*` 11 | `caption-side` on non-captions changes layout | BROKEN (not confirmed on a minimal repro) |
| css-tables misc | `percentage-sizing-of-table-cell-children` 5, `table-intrinsic-size` 4, `subpixel-*` 5, `visibility-collapse-*` 4 | | NOT CLAIMED |

#### CSS2/linebox (claimed: Q9 f43f902 `vertical-align`; baseline `line-height`)

191 eligible (0 JS) | pass 16 | fail 175 (8% pass, the worst area). Flipped 13 (W1).
Clusters: `line-height-*` 60 (+13 `-applies-to`), `vertical-align-*` 49 (+13), `inline-formatting-context-*` 14.
Marker counts: 121 mention `position:absolute` (W10), 11 `ex` units (W2), 10 negative lengths (W5, `line-height:-1px`).
These tests put a reference box beside the test box with `font: 1in Ahem` and
absolute positioning, so they sit behind W1, W10, W2 and W5. Not root-caused
beyond that.

| group | tests | triage |
|-------|-------|--------|
| W10, W1, W2, W5 gating | most of the 175 | BROKEN / NOT CLAIMED |
| `vertical-align-*` (Q9), `line-height-*` | re-measure after W10/W1 | Q40c |

#### css-flexbox (claimed: `display:flex` 440e03b, Q5 1000951, Q23/Q23b/Q23c)

928 eligible (+70 JS) | pass 243 | fail 685 (26% pass). Flipped 0.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W12 explicit width ignored when flexing | `flex-grow-001/002/006-009`, `flex-shrink-001/002/003/006` (273 mention flex-grow/shrink) | item paints at `width` while siblings move | BROKEN |
| W13 cross size | `align-self-002..016` (9); 147 mention align-* | items hug the top; `flex-end`/`center` are relative to the tallest item | BROKEN |
| W5 negative basis | `flex-basis-003/004` | negative `flex-basis` honoured | BROKEN |
| writing modes, fragmentation | `flexbox-writing-mode` 15, `flexbox-break-request-*` 8 | | NOT CLAIMED |
| abspos children | `abspos/*` 9, `position-absolute-*` 6 | uses W10 | BROKEN |
| not root-caused | `flex-minimum-height-flex-items-*` 16, `percentage-heights` 15, `col-wrap` 14, `balance-*` 28, `intrinsic-size` 24, `aspect-ratio-*` 19, `flexbox-flex-basis-content-*` 8 | | Q40c |

#### CSS2/backgrounds + css-backgrounds (claimed: Q16 93209c2, Q25 3c7a36a)

CSS2/backgrounds: 336 eligible (+3 JS) | pass 66 | fail 270. Flipped 98.
css-backgrounds: 488 eligible (+90 JS) | pass 105 | fail 383. Flipped 31.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W19 only `data:` URI images | 98 + 31 flipped (`background-053`, `background-position-*` 74) | image never drawn | PARTIAL (documented) |
| W18 `<img width="100%">` | 49 references (`background-001`: reference image paints as a small square) | | BROKEN |
| W1 units | `background-position-054` (`+0mm`) | | NOT CLAIMED |
| css-backgrounds | `background-size-*` 55, `background-origin/-clip` 12, `box-shadow-*` 12 | mostly need url images (W19); rest not root-caused | PARTIAL / Q40c |

#### CSS2/borders (claimed: Q4 3e80aae, Q4b c13703c, Q4c 96bb7b1)

504 eligible (+2 JS) | pass 242 | fail 262. Flipped 31 (W1: `border-*-width-*` with
`in/cm/pc`). 23 tests that passed raw fail after the rewrite: they passed only
because test and reference both ignored the unit.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W4 width-less border | `inline-replaced-width-001` (normal-flow), any `border: solid black` | border not painted | BROKEN |
| W5 negative width | `border-top-width-012` (`-1pt` must be invalid, i.e. medium); 15 mention negatives | | BROKEN |
| W16 border-collapse on internal table parts | `border-*-applies-to-*` (~109 mention table internals) | hollow square missing | PARTIAL |
| W1 units | 31 flipped | | NOT CLAIMED |
| ungrouped | 86 with no marker | not root-caused | Q40c |

#### css-lists (claimed: Q19c 76a9d25 `::marker`, Q19f cfa71ec counters)

155 eligible (+42 JS) | pass 57 | fail 98. Flipped 0.

| group | tests | wrong render | triage |
|-------|-------|--------------|--------|
| W23 `list-style-type:"str"`, `reversed` | `list-style-type-string-*` 8, `foo-counter-reversed` 11, `li-value-reversed` 8, `counter-reset-reversed-*` 7 | `1.` shown instead of the string; counters count up | NOT CLAIMED |
| counter scope | `counters` 6, `counters-scope` 4 | nesting scope differs | PARTIAL (not confirmed on a minimal repro) |
| `::marker` layout | `outside-marker-*`, `nested-marker-*`, `marker-*` ~12 | | PARTIAL |
| residue | 77 without marker | | Q40c |

#### selectors (claimed: Q2 66bdc60, Q31 a4b93c4)

120 eligible (+109 JS: most selector tests are testharness) | pass 43 | fail 77.

| group | tests | wrong render | triage |
|-------|-------|--------------|--------|
| W21 `:has()` | `has-style-sharing-*` 15 | `:has(> span)` and `:has(+ .x)` never match (minimal repro) | NOT CLAIMED |
| `:nth-child(n of S)` | `nth-child-of-*` ~10 | | NOT CLAIMED |
| `:host` / featureless | `featureless-*` 5 | | NOT CLAIMED |
| `:dir()` RTL | `dir-style-*` 3 | engine is LTR-only (Q2 says so) | PARTIAL |
| default namespace | `is-default-ns-*`, `not-default-ns-*` 6 | | NOT CLAIMED |
| `:lang()` | `lang-*` 10 | plain `:lang(en)`/`:lang(fr)` work on a minimal repro; the tests use wildcard/string forms | PARTIAL |

#### css-variables (claimed: Q13 b23fd53)

181 eligible (+2 JS) | pass 109 | fail 72. Flipped 0.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W22a invalid-at-parse accepted | `variable-declaration-11/12/13/23` | `--a:var(--b,!)` is accepted and wins (crimson); it must be dropped | BROKEN |
| W22b IACVT | `variable-declaration-24` | invalid at computed time falls back to an earlier declaration (red) instead of `unset` (inherited green) | BROKEN |
| W22c escaped names | `variable-declaration-31/42`, `variable-reference-21/24` | `--\30` and unicode names unresolved | BROKEN |
| W22d adjacent `var()var()` | `variable-declaration-53` | not substituted | BROKEN |
| `@supports` with var() | `variable-supports-*` 12 | conditions containing `var()` mis-evaluated | BROKEN (Q10) |
| `var()` in `content`, shorthands | `variable-reference-12`, `vars-background-shorthand`, `vars-font-shorthand` | | PARTIAL |

#### css-values (claimed: Q11 33bd57f calc/min/max/clamp, Q14 35c826a viewport units, pt 17dd346)

170 eligible (+44 JS) | pass 52 | fail 118. Not mirrored.

| group | tests | wrong render | triage |
|-------|-------|--------------|--------|
| W2 units | `ch-unit` 13, `ic-unit` 13, `ex-unit` 2, `lh-unit` 2, `q-unit-case-insensitivity` 2 | declaration dropped | NOT CLAIMED |
| `calc-size()` | `calc-size-*` 20 | not parsed | NOT CLAIMED |
| `attr()` typed | `attr-style-sharing` 5 | | NOT CLAIMED |
| angle units, `calc` rounding | `angle-units` 5, `calc-rounding` 2 | | NOT CLAIMED |
| viewport units in odd contexts | 4 | | PARTIAL (Q14) |
