# scratchy — regressions

## R5–R6: Q33a script runner bugs seen on https://wikipedia.org — OPEN (2026-11-15)

**Discovered:** 2026-11-15, user report (stderr from visiting
https://wikipedia.org with JS on, `main` @ `c1fb661`). Most of the reported
errors are just the missing DOM API (Q33b/c/d, see TODO.txt "JS on real
pages"). Two are bugs in the Q33a runner (`2646dd1`) itself, fixed here.
Mapping below was done by fetching the portal HTML and lining up line:col
from the error stacks with its 4 `<script>`s (2 inline, 2 external).

### R5 — Classic scripts run in strict mode (bug, Q33a `2646dd1`) — FIXED (e76d162)

Symptom: `js error: inline script: portalSearchDomain is not defined at
<eval> (eval_script:15:5)`. Line 15 of the portal's 2nd inline script is
`portalSearchDomain = 'wikipedia.org'`, an assignment to an undeclared
name. In sloppy mode that creates a global; in strict mode it throws a
ReferenceError. Everything after it in that script (`wmL10nVisible`,
the `setTimeout` call) never runs.

Root cause (confirmed in the crate source): `run_scripts_with` calls
`ctx.eval::<(), _>(code)`, and rquickjs 0.14's `Ctx::eval` uses
`EvalOptions::default()`, which has `strict: true`
(`rquickjs-core-0.14.0/src/context/ctx.rs`, `impl Default for
EvalOptions`). HTML classic scripts are sloppy unless they say
`"use strict"`. Strict mode also makes `this` `undefined` inside plain
function calls, which breaks old UMD/IIFE code (`(function(){ this.x = …
})()`), so some of the "cannot read/set property of undefined" errors in
the report may be this bug too, not missing DOM.

Fix: evaluate with `ctx.eval_with_options(code, opts)` where `opts` is
`EvalOptions::default()` with `strict = false` and `filename =
Some(<script name>)` (`EvalOptions` is `#[non_exhaustive]`, so build it
from `default()` and set fields). The prelude and the two FIRE_* snippets
can keep strict mode or not; keep it simple and use sloppy for all.
Tests (in `src/tests.rs`, via `run_scripts_with` with a collecting
`LogFn`):
- `x = 1; console.log(typeof x)` logs `number` and no error.
- `console.log((function(){ return this === globalThis })())` logs `true`.
- A script starting with `"use strict"; y = 1` still logs an error
  (strict stays opt-in).
Re-check: the portal no longer logs `portalSearchDomain is not defined`.

### R6 — Script errors lose the message and the script name (bug, Q33a `2646dd1`) — FIXED (a56d276)

Symptoms in the report:
- `js error: Uncaught     at <anonymous> (eval_script:1:32) …` — no
  error name or message at all. Source: `__scratchy_fire` in the
  `PRELUDE` logs `'Uncaught ' + e.stack`. QuickJS's `Error.stack` holds
  only the frames, not `Name: message` (V8 includes it, QuickJS doesn't),
  so a throwing `DOMContentLoaded`/`load` listener prints an empty
  message. (The external script ran `forEach` → `value` → `call` inside a
  load listener; the real message is lost, so it can't be triaged.)
- Every stack frame says `eval_script:L:C`, and every inline script is
  named just `inline script`, so with many inline scripts you can't tell
  which one failed (the report has 9 different `inline script` errors).
- `describe_error` prints `uncaught {value:?}` (a Rust Debug dump) when a
  script throws a non-Error value (`throw "x"`, `throw {…}`).

Fix:
- In `__scratchy_fire`, log `String(e)` (gives `TypeError: msg`) plus,
  when present, `'\n' + e.stack`.
- Name inline scripts by document position, e.g. `inline script #3`
  (1-based among all scripts, counted in `find_external_resources` order
  so it matches the page source), and pass that name as the eval
  `filename` (same change as R5), so frames read `inline script #3:15:5`.
- For a non-Error throw, convert the value with JS `String()` (or
  `JSON.stringify` for plain objects, like the console formatter does).
Tests:
- A `load` listener that throws `new TypeError('boom')` logs a line
  containing `TypeError: boom`.
- A failing second inline script's error line contains `inline script #2`.
- `throw "plain"` logs a line containing `plain` and no `Value(`/Debug
  formatting.

---

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
| W5 | Negative values that CSS says are invalid are accepted: `padding-right:-1px` shrinks the box, `max-height:-1px` clamps to 0 (should be ignored), negative `flex-basis`, negative `border-*-width`. | pre-Q box model | FIXED Q53 (4662a7f) |
| W6 | A block with `width:auto` whose borders+padding exceed the containing block is clamped to the container width; it should overflow (48px container, 2x32px borders paints 48 wide, expected 64). | pre-Q box model | FIXED Q54 (a4f43e4) |
| W7 | Vertical margins do not collapse through an empty block (`height:auto` with no content, or `height:0`): `margin-bottom:50` + empty div + `margin-top:30` gives 80, expected 50. | pre-Q box model | BROKEN |
| W8 | A float that follows inline content on the same line (`<div style=inline-block></div><div style=float:left>`) is placed below the line box instead of at the top of the current line beside it. | float 5fc83ce, Q34 | FIXED Q49 (ab0678c); leftovers: Q49a |
| W9 | Shrink-to-fit width (float, inline-block, table, abspos) is wider than the text: with Ahem 20px, `float:left` "X" is 40px wide instead of 20 (+1em); with sans 20px "PA" is +3px. Adjacent floats of text show a visible gap. | float 5fc83ce, inline-block 244a8f4 | Q43 (c8e334c) removed the extra trailing space; PARTIAL (Q43a) |
| W10 | `position:absolute` with `width:auto` and `left`/`right` auto/one side set stretches to the containing block edge (to x=800) instead of shrink-to-fit. | position bb97e60 | FIXED Q44 (0e42655); margins with left+right stretch remain (Q44a) |
| W11 | An abspos box (auto height) whose only children are floats gets height 0 and paints no background (an abspos box is a BFC and must contain floats). `overflow:hidden` on it hides the bug. | position bb97e60, Q34 | FIXED Q45 (2ce2b2a) |
| W12 | Flex items with an explicit `width` keep it when growing/shrinking: siblings move as if the item grew, but it paints at its specified width. `flex:1` items without `width` are fine. | flex 440e03b | FIXED 9abe87b (row containers) |
| W13 | Single-line flex container with a definite `height`: cross size is taken from the tallest item, not the container. `align-items:center|flex-end` aligns inside the tallest item. | flex 440e03b | FIXED Q47 (d0d8e67, row containers) |
| W14 | A block-level `display:table` element (no `float`, no `width`) fills the container and spreads its cells across it (two 40px cells land at x=0 and x=400); it should shrink-to-fit. Only the floated case works (Q34c acbcb00). | table 53bab7a, Q34c | FIXED Q50 (bf93cba) for plain tables; table-internal parts remain (Q50a) |
| W15 | A table's `width` is content-box here; browsers (and WPT refs) treat it as including the table's border+padding. `table{width:100%;border:1px}` paints 2px too wide. | table 53bab7a | FIXED Q51 (e09c450) |
| W16 | No `border-collapse`, `border-spacing` or `table-layout:fixed` support (NOTES.md says so). | table 53bab7a scope cut | PARTIAL |
| W17 | A `z-index:-1` positioned child of a non-stacking-context positioned parent paints above the parent's background; it should paint below it (in the ancestor stacking context's negative layer). | none explicit | FIXED Q55 (53e9727); in-flow relative negatives remain (Q55a) |
| W18 | `<img width="100%">` (percentage in the HTML attribute) is treated as unset; image paints at `height` x intrinsic aspect (20px for `height=20`). | image support | FIXED Q56 (bdbc410) |
| W19 | `background-image`/`mask-image` `url()` only decodes `data:` URIs; file/network URLs never load in `resolve_background_image`. | Q16 93209c2 (documented) | PARTIAL |
| W20 | Inline `<svg:svg xmlns:svg=...>` (prefixed) and `<object data=x.svg>` are not rendered. | Q12 af12757 (`<img>` SVG only) | NOT CLAIMED |
| W21 | `:has()` never matches; `:nth-child(n of S)`, `:host`, `:is/:where` with namespaces not handled. | Q2 66bdc60 lists neither | NOT CLAIMED |
| W22 | `var()`: (a) invalid-at-parse values (`var(--b,!)`, `;`, `!important !important`) are accepted; (b) invalid-at-computed-time resolves as "declaration dropped" instead of `unset`; (c) escaped custom property names (`--\30`) do not resolve; (d) adjacent `var(--b)var(--c)` fails. | Q13 b23fd53; Q57 dde81b5 | FIXED |
| W23 | `list-style-type:"string"` and `<ol reversed>` are ignored (shows `1.`, `2.`). | Q19f cfa71ec (counters only) | NOT CLAIMED |

#### CSS2/normal-flow (claimed: baseline box model, no Q item)

698 eligible (+53 skipped for JS) | pass 313 | fail 384 | 1 no-reference.
Flipped by the W1+W19 rewrite: 189, so about half of all failures are W1 alone
(`1in` squares). Residual after the rewrite: 205 failures.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W1 units | 189 flipped (`max-height-006`, `height-006`, `width-applies-to-*`) | `width:1in` ignored, box full width | NOT CLAIMED |
| W5 negative values | 13 mention one (`max-height-001`, `max-height-012`) | `max-height:-1px` collapses a 96px square to 0 | FIXED Q53 (4662a7f) |
| W6 over-constrained width | `blocks-011`, `blocks-012` | box 48px wide instead of 64 | FIXED Q54 (a4f43e4) |
| W7 empty-block margins | `margin-collapse-through-zero-height-block`, `margin-collapse-after-float-001` | gap 300 instead of 200 | BROKEN |
| W3 `<br>` | 17 mention it (`width-percentage-001` reference) | two images in the reference sit side by side | FIXED by Q41 (09b8e63) |
| W9 shrink-to-fit | `inline-block-non-replaced-width-001` | red strip: box one em too wide | BROKEN |
| W8 float after inline | `inlines-013` | float lands on the next line | PARTIAL Q49 (ab0678c); Q49a |
| W11 abspos + float | `block-formatting-context-height-001` (Q40a lead) | paints nothing | FIXED Q45 (2ce2b2a) |
| W20 `<object>` svg | `replaced-intrinsic-*` (5) | nothing drawn | NOT CLAIMED |
| ungrouped | 28 failures with none of the markers, plus `inline-table-zorder-*` (5), `inline-block-zorder-*` (3) | not yet root-caused | see Q40c |

#### CSS2/margin-padding-clear (claimed: baseline box model; float/clear 5fc83ce)

684 eligible (0 JS) | pass 277 | fail 406 | 1 no-reference. Flipped 198 (W1:
`1in`, `5em` margins). Residual 224.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W1 units | 198 flipped | `padding:1in` ignored | NOT CLAIMED |
| W5 negative padding | 33 mention a negative length (`padding-right-001`, `-012`) | `padding-right:-1px` shrinks the box by 1px | FIXED Q53 (4662a7f) |
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
| W8 float after inline | `floats-001` | blue float lands below the black box instead of at its left inner edge | FIXED Q49 (ab0678c) |
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
| W11 abspos + float child | `block-formatting-context-height-001` (normal-flow) | abspos box paints no background | FIXED Q45 (2ce2b2a) |
| W20 inline `svg:svg` | 26 mention svg (`absolute-replaced-width-009`, `-032`) | svg missing | NOT CLAIMED |
| W18 `<img width="100%">` | 11 | | FIXED Q56 (bdbc410) |
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
| W14 block `display:table` | many `border-*-applies-to`, `float-applies-to` | fills the container | FIXED Q50 (bf93cba); remainder Q50a |
| W15 table width | `block-formatting-contexts-001` reference | 2px too wide | FIXED Q51 (e09c450) |
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
| W12 explicit width ignored when flexing | `flex-grow-001/002/006-009`, `flex-shrink-001/002/003/006` (273 mention flex-grow/shrink) | item paints at `width` while siblings move | FIXED 9abe87b |
| W13 cross size | `align-self-002..016` (9); 147 mention align-* | items hug the top; `flex-end`/`center` are relative to the tallest item | FIXED Q47 (d0d8e67); `-015/-016` left (Q47a) |
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
| W18 `<img width="100%">` | 49 references (`background-001`: reference image paints as a small square) | | FIXED Q56 (bdbc410) |
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
| W22a invalid-at-parse accepted | `variable-declaration-11/12/13/23` | `--a:var(--b,!)` is accepted and wins (crimson); it must be dropped | FIXED (Q57 dde81b5) |
| W22b IACVT | `variable-declaration-24` | invalid at computed time falls back to an earlier declaration (red) instead of `unset` (inherited green) | FIXED (Q57 dde81b5) |
| W22c escaped names | `variable-declaration-31/42`, `variable-reference-21/24` | `--\30` and unicode names unresolved | FIXED (Q57 dde81b5) |
| W22d adjacent `var()var()` | `variable-declaration-53` | not substituted | FIXED (Q57 dde81b5) |
| `@supports` with var() | `variable-supports-*` 12 | conditions containing `var()` mis-evaluated | FIXED (Q57 dde81b5) |
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

### Q40c: the areas Q40b did not reach (2026-11-15)

Same method as Q40b (see "How the audit was run"). Whole-area counts come from throwaway
lists (every `rel=match|mismatch` test, not a `-ref`, not under `reference/` or `support/`),
run with `scratchy --wpt WPT --subset LIST --expected /dev/null -v`; each area also adds ~12 evenly
spaced tests to `wpt-subset.txt`. Q40b root causes W1-W23 above; new ones follow. Before starting, the
committed subset re-ran clean (187 pass / 203 fail / 6 skipped, no change from `wpt-expected.txt`).
Every root cause below was confirmed on a minimal inline test/reference pair rendered with the runner.

| id | what is wrong | claimed by | triage |
|----|---------------|------------|--------|
| W24 | `@layer` blocks are dropped whole (even a lone `@layer { #t{background:green} }` paints nothing); `revert-layer` is stored as `revert`, which behaves as `unset`. | none (style.rs notes "no cascade layers") | NOT CLAIMED |
| W25 | The `all` shorthand is not implemented (`div{background:red; all:initial}` still paints red). `all` is parsed only as a media type. | none | NOT CLAIMED |
| W26 | `@scope` is not implemented (`@scope{div{...}}` rules are dropped). | none | NOT CLAIMED |
| W27 | `revert` has no UA-origin step: `display:inline; display:revert` on a `<div>` stays inline (computes as `unset`, i.e. initial), expected `block`. | Q7 38ca872 (CSS-wide keywords; documented as unset) | PARTIAL |
| W28 | `@import` inside an inline `<style>` is never fetched (only wire-fetched stylesheets expand imports). Same with a plain file import and no media list. | Q18 3a7054b (documented scope cut) | PARTIAL |
| W29 | Every text node gets a trailing space (`inline_text` pushes `' '` after `normalize_whitespace`, which strips both ends), so an inline element boundary inside a word adds a gap: with Ahem 20px `<div>a<span>b</span></div>` paints `b` at x=40, not 20 (`ab` is 40 wide, the span form 60); `<span>a</span><span>b</span><span>c</span>` has two gaps. | inline layout (pre-Q); Q43 c8e334c trimmed only the last item of a trial pass | FIXED (Q59 510e93f) |
| W30 | `::first-line` is applied after line breaking: with `div::first-line{font-size:40px}` in a 160px Ahem box the line is broken at the unstyled size, so big words and small words share the first line and the line height ignores `line-height`/`font-size` of the pseudo. | Q19e2 0e57f9d | BROKEN |
| W31 | `::first-letter` does not take following punctuation: `T.est` and `(T)est` colour only `T.`/`(T` (the `.`/`)` stay unstyled). Leading punctuation (`"T`) works. | Q19e 17346a8 | BROKEN |
| W32 | `::marker` honours only `color` and `content`: `li::marker{font-size:40px}` is ignored (marker stays 20px), and `text-transform`, `letter-spacing`, `tab-size`, `hyphens`, `white-space`, `unicode-bidi`, `text-align`, `display:grid` do nothing. | Q19c 76a9d25 (scope: color, content) | PARTIAL |
| W33 | `letter-spacing` and `tab-size` are ignored everywhere (`letter-spacing:10px` on `ab` is 40px wide; `tab-size:2` still uses 8). `word-spacing` works. | Q26 parked | NOT CLAIMED |
| W34 | Multi-value `margin`, `padding`, `border-width`, `border-color`, `border-style`, `border-radius`, `inset` shorthands apply only the FIRST value to all four sides: `margin:5px 30px` renders exactly like `margin:5px` (same in a stylesheet), `margin:0 0 0 30px` has no left margin. Longhands work. The parse branches call `parse_length_percent` once and copy the value to every side. | box model (pre-Q); Q46a noted `margin: 0 10px` "seemed to give margin 0" | FIXED (6b8f773) for margin/padding/inset/-block/-inline; border-* already used `parse_box4`; `border-radius` per-corner is Q58a |
| W35 | `<q>` has no default quote marks: `<q>a</q>` renders `a` (UA stylesheet has no `q::before/::after{content:open-quote/close-quote}` and `quotes` has no language default). With explicit `quotes` + `content:open-quote` rules the marks and nesting work. | Q19 bb7c64d (`open-quote`/`quotes` parsed) | BROKEN |
| W36 | `clip-path` (basic shapes, `path()`, `url(#id)`, geometry boxes) and the `clip` property are not implemented: `clip-path:inset(0 50px 0 0)` paints the whole box; `clip:rect()` on an abspos box paints the whole box. `mask-mode`, `mask-composite`, `mask-clip`, `mask-origin` also do nothing. | none (Q29 312a345 is `mask-image` + size/position/repeat for icons only) | NOT CLAIMED |
| W37 | `text-shadow`, `text-emphasis-*`, `text-decoration-thickness`, `text-underline-offset`, `text-underline-position`, `text-decoration-skip-ink`/`-skip-spaces`, `text-decoration-inset` are all ignored (each paints exactly like the property absent; confirmed for `text-shadow:20px 0 red`, `thickness:10px;offset:10px`, `text-emphasis:dot`). `text-decoration-style: dotted` works. | `text-decoration-style`/`-color` item (PROMPTS v33; line, style, colour only) | NOT CLAIMED |
| W38 | `hsl()`/`hsla()`, `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()` and `color(...)` are parsed by cssparser-color but `to_text_color` (css.rs) only converts `CssColor::Rgba`, so the declaration is dropped: `background:hsl(0 100% 50%)` paints nothing, not red. `rgb()` (modern and legacy syntax), hex, named, `transparent` work. | none (colour parsing predates the Q list) | BROKEN (`hsl` is common on the web) |
| W39 | System colours (`ButtonBorder`, `ActiveBorder`... only `Canvas` works), `color-mix()`, relative colours (`rgb(from green r g b)`), `light-dark()` are unsupported. | none | NOT CLAIMED |
| W40 | A media query beginning with `not` never matches: the parser sets `never = true` for the `not` keyword instead of negating the query. `@media not all and (min-width:100000px)` and `@media not (min-width:100000px)` both skip their rules (expected: apply). | `@media` 1282285 | BROKEN |
| W41 | Only `min-width`, `max-width`, `prefers-color-scheme`, `prefers-reduced-motion` are evaluated. `height`, `aspect-ratio`, `orientation`, `resolution`, `hover`, `color-gamut`, `scripting`, range syntax (`(width >= 400px)`), `or`, nested `(( ) and ( ))` all make the query never match. | `@media` 1282285 (documented feature list) | PARTIAL |
| W42 | `@supports` grammar is lenient: `not (a) and not (b)` and `not (a) or (b)` (invalid, must not apply) apply their rules; `(unknown) or (color:green)` is treated as invalid instead of false-or-true; `not ()` and `(margin: 0 or padding: 0)` mis-evaluate; `selector(:has(a))` and `at-rule(@media)`, `font-tech()` are false. Plain `(prop:value)`, `and`, `or`, `not (prop:value)`, `selector(a > b)` work. | Q10 a4f4d91 | PARTIAL |
| W43 | Container queries (`@container`, `container-type`, container units) are not implemented. | none | NOT CLAIMED |
| W44 | `aspect-ratio` is not implemented (`width:100px;aspect-ratio:2/1` paints no height; `aspect-ratio:1` too). Also `contain-intrinsic-size`, `fit-content(<length>)`, `stretch`/`-webkit-fill-available`. | none (Q12 images keep their own intrinsic ratio only) | NOT CLAIMED |
| W45 | `width:min-content` counts the trailing collapsible space of each word: Ahem 20px `XX XX` gets 60px instead of 40 (the box is one em wider than its widest word). `max-content` and `fit-content` keywords are right on the same text. | Q20f 01a1a77 | BROKEN |
| W46 | HTML presentational attributes are not mapped to CSS: `<table border=5>` (no border), `<table width=100>`, `<td width=100>`, `<table bgcolor>`, `<body bgcolor/text>`, `<font color>` (text stays black), `<div align=center>`, `<p align=right>`, `<hr width/size/color/noshade>`, `<ol type=a>`. | none | NOT CLAIMED |
| W47 | A word wider than its line box is broken across lines glyph by glyph by default (cosmic-text `Wrap::WordOrGlyph`): Ahem 20px `XXXXXXXXXX` in a 100px box is two lines (40px tall); `overflow-wrap:normal` should overflow on one line. | Q20d 7cddd62 (`overflow-wrap`/`word-break` keywords) | BROKEN |
| W48 | `-webkit-line-clamp`/`line-clamp`/`block-ellipsis`, `scroll-marker`/`scroll-marker-group`, `overflow-clip-margin`, `text-overflow:<string>` are not implemented; `overflow-body-propagation` (body `overflow` propagated to the viewport) is not. | none (Q21 3a5e687 is `overflow-x/-y` + `text-overflow:ellipsis|clip`) | NOT CLAIMED |
| W49 | Named grid lines are dropped: `grid-template-columns:[a] 50px [b] 50px [c]` makes the whole declaration invalid (single auto column), so `grid-column:b` has no effect. Also `repeat(2,[x] 50px)`. | none (Q15c is numeric placement) | NOT CLAIMED |
| W50 | `display:inline-grid` and `inline-flex` lay out as block-level boxes (a following `inline-block` wraps to the next line instead of sitting beside the grid). | grid `display:grid` item; no explicit inline-grid claim | BROKEN |
| W51 | Grid alignment is missing: `justify-content`, `align-content`, `justify-items`, `align-items`, `justify-self` ignored (`justify-items:end` leaves a 20px item at x=0); items do not stretch to an explicit row height (`grid-template-rows:20px` + auto-height item paints nothing); `order` is ignored in grid; `grid-auto-flow:column`/`dense` is not implemented. | Q15 (not claimed); TODO.txt notes `grid-auto-flow: row` only | PARTIAL |
| W52 | Acid2 (`acid/acid2/test.html`, compared directly with `px-reference.html`; the wpt `reftest.html` wrapper needs JS and an iframe) renders only the fallback text ("Standards compliant... Take The Acid2 Test") where the smiley face belongs. The face is built from `<object>` data URIs and fixed/abspos stacking; `<object>` is not rendered (W20). | none | NOT CLAIMED |
| W53 | Clearance: a `clear`ed block's own `margin-top` is added below the float instead of being absorbed by clearance. Float 0-50px, then `clear:left;margin-top:10px` paints at y=60; the top border edge belongs at y=50. | float/clear 5fc83ce | BROKEN |
| W54 | Flex items have no automatic minimum size (`min-height:auto`/`min-width:auto`): in a column flex container with `height:10px`, an item with `height:100px` is shrunk to 10px (green 10px tall); with `flex-shrink:0` it stays 100px. The spec floors it at `min(content, specified size)`. | flex 440e03b, Q5 1000951 | BROKEN |
| W55 | An abspos box whose height comes from `top:0;bottom:0` is not a definite containing-block height for percentage-height children: `height:50%` inside it (block or column flex) resolves to auto and paints nothing; with an explicit `height:200px` it works. | position bb97e60 | BROKEN |
| W56 | Column-wrap flex containers: `max-height` is not used as the wrap limit (no wrapping at all with `flex-flow:column wrap;max-height:100px`), and with a definite `height` the second column lands at x=400 (columns spread over the container width) instead of next to the first at x=50. | Q23 4661363 / flex wrap (row only documented) | BROKEN |
| W57 | `caption-side` is treated as non-inherited (style.rs `caption_bottom`): `table{caption-side:bottom}` with a plain `<caption>` still draws the caption on top; only a declaration on the caption itself works. CSS2.1: inherited. | Q34c acbcb00 | BROKEN |
| W58 | Paint order: an inline-block or inline-table is painted in the block layer in tree order, so a LATER block's background paints over it. `<div><span style="display:inline-block;background:green...">` followed by a `margin-top:-40px` red block shows red; inline-level content must paint above in-flow block backgrounds. | inline-block 244a8f4 | BROKEN |
| W59 | A `position:relative` inline that contains a block (block-in-inline split) does not shift the block part by its offset. | position bb97e60 | BROKEN |
<!-- Q40c-table-end -->

#### css-cascade (claimed: Q7 38ca872 CSS-wide keywords, Q18 3a7054b `@import` for fetched sheets)

50 eligible (+10 skipped for JS) | pass 13 | fail 27. (No crashes.)

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W24 `@layer` / `revert-layer` | `revert-layer-001/002/003/005/006/007/009/010/011/012` (10), `layer-stylesheet-sharing`, `-important` (12) | layered rules vanish; the box is unpainted instead of green | NOT CLAIMED |
| W26 `@scope` | `scope-implicit-001..006-print` (6), `scope-pseudo-element`, `scope-ua-shadow-host` (8) | scoped rules dropped, box missing | NOT CLAIMED |
| W25 `all` shorthand | `all-prop-001`, `all-prop-002`, `all-prop-initial-visited` (3) | `all:initial` does not reset (red box stays) | NOT CLAIMED |
| W28 inline `@import` | `import-conditional-001`, `-002` (2) | imported sheets never applied | PARTIAL |
| W27 `revert` | `revert-val-001` (1) | `display:revert` on a div stays inline | PARTIAL |
| animations | `important-prop` (1) | `@keyframes` overriding normal declarations | NOT CLAIMED |

Passing tests include `all-prop-*-color`, `revert-val-002`, `unset-val-*`: they only check `inherit`/`initial`/`unset` on single properties.

#### css-pseudo (claimed: Q19 bb7c64d `::before/::after`, Q19c 76a9d25 `::marker`, Q19e 17346a8 `::first-letter`, Q19e2 0e57f9d `::first-line`)

110 eligible (+163 skipped: 163 `<script>`/testharness, mostly `::selection`/`::highlight`/`::target-text`) | pass 29 | fail 81.
Marker counts among failures: `marker` 36, `first-letter` 22, `first-line` 14, form pseudos (`slider-*`, `file-selector-button`, `placeholder`) + spelling/grammar-error 12.
Hidden behind W29/W34: re-running on a scratch copy with W29 (trailing space only when the source had one) and W34 patched flips 8 of the 81 (`first-letter-skip-*`, `first-letter-with-before-after`, `first-line-with-quote`, `marker-content-006`) and breaks 2 vacuous passes, so W29 is a real cause in this area but not the main one.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W32 `::marker` properties | `marker-*` 36 (`marker-letter-spacing`, `marker-text-align-001..003`, `marker-hyphens`, `marker-content-003/006/010/012/015/016/018/019`) | marker text keeps the list item's font; `display:grid` or `text-transform` on `::marker` ignored | PARTIAL (documented scope) |
| W31 `::first-letter` punctuation | `first-letter-trailing-punctuation`, `first-letter-with-quote`, `first-letter-hi-*` | only the letter (plus leading punctuation) is styled, trailing `.`/`)`/closing quote stay plain | BROKEN |
| `::first-letter` float/abspos/background | `first-letter-002..005`, `first-letter-background-image`, `first-letter-opacity-001` | not root-caused (float + `position:absolute` combos; a plain floated `::first-letter` on Ahem matches its `<span>` equivalent) | BROKEN (not confirmed on a minimal repro) |
| W30 `::first-line` | `first-line-line-height-001/002`, `first-line-with-before-after`, `first-line-opacity-001`, `first-line-below-float`, `first-line-with-out-of-flow*` (14) | text that fits at the base size stays on the enlarged first line; line box height unchanged | BROKEN |
| W29 inline boundary gap | refs of `first-letter-with-span`, `first-letter-skip-empty-span*` | `<span>a</span>bc` is 20px wider than `abc` | BROKEN |
| form/text pseudos | `slider-fill/thumb/track-*` 7, `file-selector-button-*` 2, `placeholder-input-number`, `spelling-error-001`, `grammar-error-001` | not rendered | NOT CLAIMED |
| `::before` as flex container | `before-as-flex-container` | passes on a minimal repro (`display:flex` on `::before`); fails on test details not reproduced | PARTIAL |

#### css-content (claimed: Q19 bb7c64d, Q19b baee567 `attr()`, Q19d c1507db `content:url()`)

55 eligible (+11 skipped for JS) | pass 9 | fail 46. (Before the runner fix for root-relative hrefs 12 passed: 3 passes were vacuous, both sides missing the same stylesheet.)

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W35 default quotes | `quotes-001..031`, `quotes-033/034`, `quotes-match-parent-001` (34) | `<q>` and `open-quote` text without author rules show no “ ” ‘ ’ | BROKEN |
| `content` on elements | `element-replacement*` 9 (`element-replacement.html`, `-root`, `-alt`) | `content: url()`/gradient on a normal element does not replace it | NOT CLAIMED (Q19d is pseudo-elements only) |
| `attr()` details | `attr-case-sensitivity-001`, `attr-chained-pseudo-001` | case-sensitivity rules, `::after::marker` chain | PARTIAL (Q19b) |
| `pseudo-element-inline-box` | 1 | inline box around generated content | not root-caused |

#### css-masking (claimed: Q29 312a345 `mask-image`/`-webkit-mask-image` with size/position/repeat)

287 eligible (+100 skipped for JS) | pass 47 | fail 240. After the runner fix, the 3 `bad-mask-image*` tests (reference `about:blank`) run and fail instead of CRASH (invalid mask URL must hide the element).

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W36 `clip-path`, `clip` | `clip-path/*` 123, `clip/*` 19 (`clip-path-inset-*`, `clip-path-circle-*`, `clip-rule-*`) | shape ignored, full box painted | NOT CLAIMED |
| W19 file/network mask URLs | `mask-image/mask-image-1a..4b`, `mask-position-*`, `mask-repeat-*`, `mask-size-*` (about 70 of 97) | `mask-image:url(m.svg)` + `mask-repeat:no-repeat` on a 100px box paints 100px (mask dropped); the same mask as a `data:` URI paints the 50px tile | PARTIAL (documented; Q35a) |
| `mask-mode`, `mask-composite`, `mask-clip`, `mask-origin`, `mask-opacity` | `mask-mode-*` 6, `mask-composite-*` 8, `mask-clip-*` 6, `mask-origin-*` 2, `mask-opacity-*` 5 | properties ignored | NOT CLAIMED |
| invalid mask URL | `bad-mask-image*`, `mask-image-svg-loading-error` | element must be fully masked, text still painted | NOT CLAIMED |

#### css-text-decor (claimed: text-decoration line/style/color, Q7c 042f538 CSS-wide keywords)

311 eligible (+28 skipped for JS) | pass 82 | fail 229. Hidden behind W29/W34: 8 flipped.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W37 `text-emphasis*` | 125 (`text-emphasis-position-property-*` 40, `-style-property-*` 32, `-line-height-*` 13) | no marks drawn | NOT CLAIMED |
| W37 `text-decoration-inset` | 37 | property ignored | NOT CLAIMED |
| W37 thickness / offset / underline-position | 21 (`text-underline-offset-*`, `text-decoration-thickness-*`) | always default thickness/position | NOT CLAIMED |
| W37 skip-ink / skip-spaces | 16 | | NOT CLAIMED |
| W37 `text-shadow` | 12 (`text-shadow/basic`, `-blur`, `-multiple-noblur`) | no shadow | NOT CLAIMED |
| propagation | `text-decoration-propagation*` 6, `text-decoration-subelements` 3 | `display:contents`/inline-block propagation differs | PARTIAL |
| dotted/ordering | `text-decoration-dotted-*` 2 | | PARTIAL (not root-caused) |

#### css-color (claimed: none explicitly; colour keywords, `#rgb[a]`, `rgb()`)

299 eligible (+12 skipped for JS) | pass 88 | fail 211. Hidden behind W29/W34/Q48a: flipped 0 under the W29/W34 patch.
Failures by name: `hsl`/`t424`/`t425` 42, predefined spaces (`display-p3`, `a98rgb`, `rec2020`, `xyz`...) 43, `lab/lch/oklab/oklch/hwb` 39, `relative-*`/`color-mix`/`light-dark` 23, `deprecated-sameas-*` 23 (system colours), `rgb-*`/`rgba-*`/`hex-*`/`currentcolor-*`/`t32-*` about 37.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W38 colour functions | `hsl-*`, `t424-hsl-*`, `hwb-*`, `lab-*`, `lch-*`, `oklab-*`, `oklch-*`, `*-p3*`, `a98rgb-*`, `rec2020-*` (about 120) | declaration dropped, box unpainted | BROKEN |
| W39 | `deprecated-sameas-*` 23, `relative-currentcolor-*` 19, `color-mix-*` 4 | | NOT CLAIMED |
| rgb/hex/currentcolor/opacity tests | `hex-001..004`, `rgb-001..008`, `rgba-*`, `currentcolor-001/002/004`, `t32-opacity-*` | not a colour bug: the tests stack `<p>` boxes whose adjacent vertical margins do not collapse (the engine has no margin collapsing, Q48a), so the square is 16px lower than the reference | BROKEN (known, Q48a) |

#### mediaqueries (claimed: `@media` min-width/max-width + prefers-* 1282285)

48 eligible (+10 skipped for JS) | pass 20 | fail 28. Viewport is the runner's 800x600.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W40 `not` | `negation-001/002`, `mq-invalid-media-type-001/004` (4) | rules inside `@media not ...` skipped | BROKEN |
| W41 unsupported features / MQ4 syntax | `aspect-ratio-*` 5, `device-aspect-ratio-*` 2, `mq-range-001`, `mq-negative-range-*` 2, `mq-gamut-*` 3, `mq-calc-*` 7, `mq-case-insensitive-001` | query never matches | PARTIAL |
| scripting / svg-as-image | `scripting-print-*` 2, `prefers-color-scheme-svg-as-image` | | NOT CLAIMED |
Many passes are vacuous (`(width > 1000px)` never matches either way); the confirmed matrix is in the W40/W41 repros.

#### css-conditional (claimed: Q10 a4f4d91 `@supports`; `@media` 1282285)

145 eligible (+28 skipped for JS) | pass 88 | fail 57.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W42 `@supports` grammar | `css-supports-019/023/029/030/032/033/034/040/041/042` (10), `at-supports-026/034..039/045/048` (9), `at-supports-at-rule-*` 4, `-selector-*` 6, `-namespace-*` 2, `-content-*` 2, `-font-format/tech` 2 | rules apply (or not) contrary to the spec for malformed conditions | PARTIAL |
| W43 container queries | `container-queries/*` 16 | `@container` blocks ignored | NOT CLAIMED |
| `@media` + `@font-face`/`@import` | `at-media-001/003`, `at-media-content-002/003` (4) | not root-caused (rules inside `@media all` with `@font-face`, `@import` placement) | not confirmed |

#### css-sizing (claimed: Q20f 01a1a77 `width/min-width: min-content|max-content|fit-content`, Q15a 1154431 grid tracks)

569 eligible (+43 skipped for JS) | pass 143 | fail 426. Hidden behind W29/W34: 6 flipped.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W44 `aspect-ratio` | `aspect-ratio/*` 205 (`block-aspect-ratio-*` 37, `flex-aspect-ratio-*` 48, `grid-aspect-ratio-*` 36, `replaced-element-*` 27, `abspos-*` 19) | height stays auto | NOT CLAIMED |
| W44 `stretch`, `fit-content()`, `contain-intrinsic-size` | `stretch/*` 66, `fit-content-length-percentage` 25, `contain-intrinsic-size/*` 24 | ignored/never parsed | NOT CLAIMED |
| `intrinsic-percent-*` | 39 | percentage-sized replaced/non-replaced inside shrink-to-fit parents | PARTIAL (shrink-to-fit, W9/Q43a) |
| W45 min-content | `block-size-with-min-or-max-content`, `*-with-min-or-max-content` 29 (partly), `border-box-and-max-content` | box one em too wide | BROKEN |
| `margin-collapse-with-indefinite-block-size` | 5 | Q48a | BROKEN (known) |

#### html/rendering (claimed: default.css UA sheet, no Q item for presentational hints)

272 eligible (+65 skipped for JS) | pass 145 | fail 127. Hidden behind W29/W34: 1 flipped.
Failures by directory: `the-fieldset-and-legend-elements` 21, `tables` 20, `the-select-element` (replaced + widgets) 15, `button-layout` 8, `embedded-content`/`attributes-for-embedded-content` 8, `lists` 6, `the-hr-element` 4, `phrasing-content` 3, other 22.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| form controls | `fieldset`/`legend` 21, `select` 15, `button-layout` 8, `form-controls` 2, `textarea` 2 | controls drawn as plain text/blocks | NOT CLAIMED (Q30 forms, later) |
| W46 presentational attributes | `tables/table-border-1`, `table-width*`, `table-cell-width*`, `table-bordercolor-001`, `table-align-float`, `the-hr-element-0/*` 4, `lists/ol-type-supported`, `ul-`, `li-type-`, `div-align`, `font-element-text-decoration-color` | attribute ignored | NOT CLAIMED |
| tables | `table-row-height`, `table-row-group-height`, `table-direction*`, `table-row-pagination-*-print` | row heights/direction ignored (W16 family, Q51a) | PARTIAL |
| embedded | `audio-controls`, `canvas-*`, `video-*` | no media elements | NOT CLAIMED |
| `br` | `br-and-wbr-with-non-inline-display`, `br-clear-presentational-hints` | | PARTIAL (Q41a) |

#### css-overflow (claimed: Q21 3a5e687 `overflow-x/-y`, `text-overflow:ellipsis`; Q24 00648b9 scroll containers)

559 eligible (+167 skipped for JS) | pass 98 | fail 461. (The 1 CRASH, `overflow-body-propagation-013`, had an `about:blank` reference; it now runs and fails.)

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W48 line-clamp | `line-clamp/*` 293 (`line-clamp-auto` 52, `block-ellipsis` 40, `webkit-line-clamp` 29) | no clamping | NOT CLAIMED |
| W48 `text-overflow:<string>` and ellipsis details | `text-overflow-string-*` 24, `text-overflow-0xx` 18 (`-004` inherit, `-006` edge of line box, `-012` grapheme clusters) | | NOT CLAIMED / PARTIAL |
| W47 long words wrap | `text-overflow-002/004` (test shows 3 stacked black rows instead of one clipped line) | unbreakable Ahem word is split over lines | BROKEN |
| W48 scroll markers, clip margin, body propagation | `scroll-markers/*` 44, `overflow-clip-margin-*` 21, `overflow-body-propagation-*` 7 | | NOT CLAIMED |
| `overflow:clip` combos | `clip-002..005` | `overflow:clip` + `overflow:visible` per axis | PARTIAL |

#### css-grid (only the Q15 subset: Q15a 1154431 track sizes, Q15b 9397a18 `grid-template`, Q15c a665a34 numeric placement, Q15d 86e1dcb `repeat(auto-fill|auto-fit)`)

Run over `grid-definition`, `placement`, `layout-algorithm`, `implicit-grids`, `grid-model` and the top-level tests (218 files): 190 eligible (+28 skipped for JS) | pass 79 | fail 111.
Not run (not claimed): `grid-lanes` 928, `subgrid` 102, `masonry`, `alignment` 167, `abspos` 153, `grid-items` 190, `parsing`, `animation`.
The claimed features pass on minimal repros: `repeat(2,50px)`, `1fr 2fr`, `minmax()`, `min-content` tracks with Ahem text, `auto-fill` + overflow wrapping, `gap`, `grid-template` strings, `grid-area` names, `grid-column:span 2`, `1/-1`.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W49 named lines | `grid-placement-using-named-grid-lines-*` 8, `grid-support-named-grid-lines-*` 2, `grid-repeat-integer-*` 5 (named lines inside `repeat`) | track list dropped | NOT CLAIMED |
| auto-fit details | `grid-auto-fit-*` 9, `grid-auto-repeat-*` 8, `flex-item-grid-container-auto-repeat` 2 | collapsed tracks, `fit-content()`, calc, aspect ratio | PARTIAL (Q15d covers the basic form) |
| W50 inline-grid / W51 | `grid-model/*-inline-*` 7, `display-inline-grid`, `grid-container-scrollbar*` 5, `grid-first-letter`, `grid-floats-*` | inline-grid block-level; scrollbars/floats | BROKEN / PARTIAL |
| `fr-unit`, `fr-unit-with-percentage`, baseline | `fr-unit*` 2, `grid-container-baseline-synthesized` 4 | | PARTIAL |

#### acid2 (not claimed)

1 test run by hand: `acid/acid2/test.html` against `acid/acid2/px-reference.html` (the committed `reftest.html` needs `<script>` and an iframe, so the runner skips it) | pass 0 | fail 1.
First divergence: the page shows the intro text and a small red bar instead of the face (W52). Not added to `wpt-subset.txt` (the runnable file lives outside the checkout list); revisit after `<object>` and iframe support.

#### Q40b residue ("ungrouped / not root-caused" rows), re-measured 2026-11-15

Re-run over `clear-clearance-calculation-*`, `block-in-inline-*`, `inline-table-zorder-*`, `caption-side-applies-to-*`, `flex-minimum-height-flex-items-*`, `percentage-heights-*`, `intrinsic-size/col-wrap-*`: 99 eligible (+16 skipped for JS) | pass 42 | fail 57 (col-wrap measured separately: 14 fail, 12 JS-skipped).

| group | counts | root cause (confirmed on a minimal repro) | triage |
|-------|--------|-------------------------------------------|--------|
| `clear-clearance-calculation-*` | 5 of 5 fail | W53 (clearance + margin) and the reference's `url()` background (W19); also Q48a | BROKEN |
| `block-in-inline-*` | 25 pass, 5 fail (`-007`, `-relpos-001/002`, `-self-collapsing-only-child`, `-zero-height-margin-collapse`), 3 skipped | the 3 margin cases are Q48a (no margin collapsing); the 2 relpos cases are W59. The Q40b "8 fail" were mostly fixed by Q41/Q49 etc. | BROKEN (Q48a / W59) |
| `inline-table-zorder-*` | 5 of 5 fail (inline-block variant fails too) | W58 | BROKEN |
| `caption-side-applies-to-*` | 5 pass, 11 fail (`-006..-017`, all `display:table*` hosts) | W57: the property sits on the table and is not inherited by the caption. The Q40b guess ("applies to non-captions changes layout") was wrong | BROKEN |
| `flex-minimum-height-flex-items-*` | 9 pass, 16 fail, 6 skipped | W54 | BROKEN |
| `percentage-heights-*` | 3 pass, 15 fail, 7 skipped | W55 (abspos `top/bottom:0`); the rest need `flex-basis:%` of an indefinite parent | BROKEN |
| `col-wrap-*` | 0 pass, 14 fail | W56 | BROKEN |
