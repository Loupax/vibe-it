# scratchy — regressions: FIXED sections moved out of REGRESSIONS.md (2026-11-19)

Unchanged text. REGRESSIONS.md keeps a stub heading with the same title. R numbers are unique
across both files.

## R33: A test fails on a clean checkout of main: `inline_flex_lays_its_children_out_side_by_side_not_stacked` (review, 2026-11-17) — FIXED (QCI0)

**Status: FIXED by QCI0.** Root cause (a REAL flex bug, found by forcing another sans-serif: Lato and Noto Sans CJK JP reproduce it here, Liberation Sans and 0xProto Propo happen to pass): `parse_flex_shorthand` defaulted an omitted basis to `auto`, so `flex: 1` was `1 1 auto` and the split depended on each item's max-content width (the 0.13 px was "x" vs "y"). Per CSS Flexbox the omitted basis is `0%`: now `FlexBasis::Len(Percent(0))`. New test `flex_number_shorthand_has_zero_basis_so_split_ignores_glyph_widths`; font-dependent `q30d_paint_value_placeholder_password_caret_selection` made relative; `shrink_to_fit_table...` under 0xProto Propo filed as QCI0a. Original report: Found by the 2026-11-17 review of main (ec6a0e6, again at
7e007e6) in a fresh Linux container: `cargo test --release` -> 877 passed,
1 failed, 2 ignored. The failing assertion: two `flex: 1` items in a 100 px
`inline-flex` box, `children[0].w` = 49.871094, expected 50.0. The container's
`fc-match sans-serif` is Inter; the test (244a8f4, 2026-11-11) presumably passes
on the user's machine with other fonts, and the repo has no CI, so nothing ran it
elsewhere. The 0.13 px is about the width difference of "x" and "y", which
suggests `flex: 1` (basis 0) is not resolving from a zero basis, or the
automatic minimum leaks in; not root-caused. Repro: `cargo test --release --lib
inline_flex_lays`. Fix or make font-independent in QCI0, then CI (QCI1).

### Not a regression, recorded here for agents: Q26 `letter-spacing` "cosmic-text bug"

The 2026-11-12/13 finding that cosmic-text 0.19.0 wraps far too early with
letter-spacing was a unit mismatch on scratchy's side: `Attrs::letter_spacing`
takes EM ("Set letter spacing (tracking) in EM", attrs.rs), the attempt passed
px. Standalone repro (cosmic-text =0.19.0, "Wide letter spacing ", 16 px font,
900 px, `Wrap::WordOrGlyph`): spacing 1/2/3/4/5/8 passed raw -> 1/1/2/2/3/4
lines (the recorded numbers exactly); passed as px/16 -> 1 line each, width
160.27 + 20 x spacing. Q26 is READY in TODO.txt.

## R20: www.youtube.com is a blank window (Opus investigator, 2026-11-15) — blank window FIXED by QY1 (ebea7ae)

Investigation only, no code changed. Fix items are QY1–QY7 at the top of
the TODO.txt QUEUE. Build: release build of `feada0e` (= origin/main at
the time, built in an isolated worktree); live runs in a nested dwl
(`tools/nest.sh`, one scratchy window, `grim`, screenshots viewed);
headless renders with the WPT runner recipe from R12–R19. Machine load
was 9–26 during the run, so wall-clock numbers are inflated; CPU times
are given where they matter.

### What the user sees, and what the network does

- `scratchy --socket $XDG_RUNTIME_DIR/wire.sock https://www.youtube.com`:
  "Loading…" for ~3–4 s, then a completely white window, forever (checked
  at 15 s and 30 s). stderr has exactly 4 `js error` lines (below).
- `--no-js`: the same white window, no stderr. `:set nojs` + `:refresh`
  (typed with `tools/inject`): white; no new JS errors after the refresh.
  So **the blank window is not caused by JavaScript.**
- Network is fine: `curl -sIL` and a raw `GET` through wire both get one
  `200 text/html; charset=utf-8`, no redirect, no consent.youtube.com
  hop (EU/DE IP, no cookies), ~1.0 MB of HTML (curl with a Chrome UA
  0.92 MB, without one 0.87 MB). Headers include
  `content-security-policy: require-trusted-types-for 'script'`
  (scratchy ignores CSP; harmless here). Set-Cookie: `YSC`,
  `__Secure-YEC`, `VISITOR_PRIVACY_METADATA` etc.; wire stores none,
  which does not matter for this page.
- Server HTML without JS: no `<noscript>` at all. `<body>` holds a hidden
  `iframe` (passive Google sign-in), `<ytd-app>` (contains the masthead
  shell: hidden search input, hamburger `<svg>`, two `display:none`
  logo links, 3 grey skeleton icons, then `display:none` footer links),
  then `#home-page-skeleton` (guide column + chip bar + 16 grey
  placeholder video cards) and a hidden `#watch-page-skeleton`. All
  real data is JSON in inline scripts (`ytInitialData`, 160 KB). For a
  logged-out EU visitor that data is NOT videos: it is a cookie-consent
  bump ("Bevor Sie zu YouTube weitergehen", "Alle akzeptieren" …) and a
  "Der YouTube-Verlauf ist deaktiviert" feed nudge. Chrome itself shows
  only those after JS; without JS Chrome shows the grey skeleton.
- Size: 43 scripts (34 inline, 9 external; the app script `kevlar_base`
  is 10.8 MB), 4 stylesheets including one of 3.8 MB (871 `@media`, 507
  `@supports`, 269 `@container`), snag bundle 9.8 MB. Headless render of
  the whole script-less page: 1.1 s CPU. Time to replace "Loading…": ~4 s
  with JS, ~3–4 s without (1 s grim polling).

### R20 — An earlier `position:absolute` box paints over later positioned boxes whatever their z-index (bug) — FIXED (QY1, ebea7ae)

**Why YouTube is blank.** The 3.8 MB stylesheet has
`ytd-app{…;background:var(--t3e41d7b17b187f69) /* #fff */;display:block;
left:0;min-height:100%;position:absolute;right:0;top:0}`. Without JS
`<ytd-app>` is empty apart from a `position:fixed` white masthead, so it is
a white sheet over the whole viewport. The skeleton that follows it is
`#home-page-skeleton{position:relative;z-index:0;margin:56px 0 0;…}`. In
CSS 2 Appendix E both are in the root stacking context's
"positioned, z-index auto/0" layer, painted in tree order, so the
later skeleton is on top (that is what a no-JS Chrome shows). scratchy
paints the `ytd-app` sheet last.

Bisect (headless, scripts stripped from the snag bundle; "blank" = the
800x600 render equals `about:blank`): the full page is blank; with the
3.8 MB `<style>` removed it shows content; with the `<ytd-app>` element
removed it shows content; with the fonts `<style>` or the skeleton
stylesheet removed it is still blank. With `<ytd-app>`
removed, JS ON, live window: the masthead line, the white guide column
and the grey skeleton cards render (single column, see QY7), i.e.
roughly what Chrome shows with JS off.

**Minimal repro** (all yellow today; the gray bar must show):
```html
<body style=margin:0>
<div style="position:absolute;top:0;left:0;right:0;min-height:100%;background:yellow"></div>
<div style="position:relative;z-index:0;background:gray;height:100px;margin-top:56px"></div>
```
Also wrong with `z-index:1` and with no `z-index`; a custom element
(`<x-app style=display:block;…>`) behaves the same; `height:100%` instead
of `min-height` the same; an abs box with no height correctly shows the
gray bar (nothing to cover it).

**Root cause.** `paint_inner` paints a box's in-flow children, then its
`positioned` list through `paint_z_layer`. An abs box whose containing
block is the initial containing block bubbles (`catch_or_bubble_
positioned`) to the root's `positioned` list, so it is painted after the
entire `<body>` subtree, including every relative/z-indexed box in it.
`is_explicit_stacking_child` only re-orders relative/sticky boxes that
have a z-index AND are direct children of the box that owns the
`positioned` list. This is R17 seen from the other side; QA9's planned fix
breaks ties "stacking children before positioned", which still loses here
(`ytd-app` z auto = 0 vs skeleton z 0), so QY1 replaces QA9 with a
tree-order tie-break.

### The four console errors (JS on), each read at its source location

`scratchy --js-report --socket $XDG_RUNTIME_DIR/wire.sock page.html`, where
`page.html` is the raw wire response body with `<base
href="https://www.youtube.com/">` added (snag's bundle works too): 43
scripts run, 4 errors; the live window prints the same 4.

| # | where | message | what is missing | item |
|---|---|---|---|---|
| 1 | inline script #6 (`ytcsi`), `ytcsi.setStart(w.performance ? w.performance.timing.responseStart : null)` | `cannot read property 'responseStart' of undefined` | `performance.timing` (QuickJS has `performance.now` only) | QY2 |
| 2 | `web-animations-next-lite.min.js`, `createElementNS(xhtml,"canvas").getContext("2d")` | `TypeError: not a function` | `HTMLCanvasElement.getContext` (null is fine) | QY3 |
| 3 | `webcomponents-all-noPatch.js`, `document.createEvent("Event"); initEvent(…)` | `TypeError: not a function` | `document.createEvent` (QA15), then a chain, see QY4 | QY4 |
| 4 | `kevlar_base…` (10.8 MB app script), a CRC table loop near the top | `InternalError: interrupted` | compile alone takes 2.6 s CPU / 8.3–8.9 s wall here, and the 5 s wall-clock budget is armed before compiling | QY5 |

None of the four affects what is painted today (the DOM is never
changed). Fixing all four would not show content either: see below.

How the chain in QY4 was found: a JS shim script inserted before
YouTube's scripts (scratchpad only) defined the missing piece, then
`--js-report` was re-run on a page holding just the shim and the
polyfill. With `performance.timing`, a `createEvent` stub and a
`getContext` returning null, errors 1 and 2 disappear and the polyfill
moves on to `DocumentFragment is not defined` → `CharacterData` →
`ErrorEvent` → `cannot read property '__CE_registry' of undefined`
(`node.ownerDocument` missing) → `MutationObserver is not defined` →
`NodeFilter is not defined` → `document.createTreeWalker` not a function,
with `document.implementation.createHTMLDocument` on the same line.
The chain was stopped there: TreeWalker and a second document are
QA17/QA18-sized.

### YouTube: what the page needs (beyond R20)

- Content and the consent dialog are Polymer custom elements (`ytd-*`)
  rendered from `ytInitialData`: Web Components via the shipped polyfill
  (QY6, L), then `fetch`/XHR (Q33f) for anything past the first screen.
  Until then the best possible rendering is the skeleton.
- Even if QY5 let the 10.8 MB script run to its end, it freezes the main
  thread for the whole run (R8) and cannot build anything without QY6; do
  not raise the budget for it.
- `@container` queries (QY7) only change the skeleton's column count.
- Not filed (seen, no payoff now): `window.onerror` is never called for
  script errors (YouTube installs one to beacon errors, harmless), the
  masthead hamburger `<svg>` uses `viewBox` on a `<g>` (also odd in
  Chrome), `scrollbar-color`/`::-webkit-scrollbar` rules.

### Not tested / caveats

- No Chrome/Firefox on this machine: "Chrome shows the skeleton" is from
  the CSS 2 Appendix E paint order, not a side-by-side screenshot.
- Logged-in YouTube, non-EU consent flow, `/watch` pages, `/results`
  search pages and mobile `m.youtube.com` were not tried; video playback
  is out of scope.
- QY2–QY4's effect was measured with JS shims, not with real fixes; the
  real implementations can hit further errors.
- Wall times are from a machine at load 9–26 (other agents' builds).

## R12–R19: Acid2 / Acid3 audit (Opus investigator, 2026-11-15) — ALL FIXED (R12 QA3 bd2945f, R13 QA1 10e4006, R14 QA2 80395ee, R15 QA4 065b61b, R16 QA5 e9232f3, R17 QY1 ebea7ae, R18 QA6 e8fb789, R19 QA7 00530a0)

**Status (audit 2026-11-16):** every R12-R19 bug is fixed (hashes in the
heading and in each entry). The Acid2/Acid3 sections below also list
later QA items; their open leftovers (QA19 Range, QA21 `<object>`, QA*a
items) are tracked in TODO.txt. Acid scores are not a merge gate.

Investigation only, no code changed. Fix items are QA1–QA21 at the top of
the TODO.txt QUEUE (R17/QA9 is now QY1, see R20). Build: release build of `e7a090d` (before QB8 merged).
Every repro below was rendered headless with the WPT runner (a scratch
"checkout" dir holding `fonts/Ahem.ttf` and a one-line subset file per
fixture, each fixture `<!DOCTYPE html><link rel=match href=about:blank>…`,
`scratchy --wpt DIR --subset F --expected /dev/null --out OUT` saves the
800x600 PNG), and the colours were read off the PNG on a 12px grid. The
live screenshots are from a nested dwl (`tools/nest.sh`, one scratchy
window, `grim`).

### Acid2 (http://acid2.acidtests.org/)

What a user sees, live, through wire:
- Opening the URL: the intro ("Standards compliant? / Take The Acid2 Test
  and compare it to the reference rendering.") with no border around it
  (`border: solid thin` has no colour, so Q52/Q52c), and a black bar with
  a red bottom edge painted over the word "Standards" (should be hidden
  under the intro: R17, and red instead of yellow: R12).
- Opening `http://acid2.acidtests.org/#top`: identical, the page does not
  scroll to `#top` (R18).
- Clicking "Take The Acid2 Test": "Hello World!" at the right place, then
  a red rectangle from x=38 to the window edge and y=74 to 369 with face
  fragments in it. The reference page (`reference.html`, also rendered by
  scratchy, correctly) shows the yellow smiley at x=74..242, y=110..278.

The face, line by line (12px grid, headless render of the test with the
intro removed and `#top{margin-top:0}` so it needs no scrolling):

| face line (Acid2 CSS) | expected | seen | cause |
|---|---|---|---|
| whole picture | no background (`<link rel="appendix stylesheet" href="data:text/css,.picture{background:none}">`) | red block | R15 (`background:none` keeps red) and R16 (data: stylesheet sent to wire) |
| 1 scalp: `position:fixed`, min/max-height | black 48x12, yellow 6px bottom border | black 48x12 correct, bottom 6px red | `p.bad` (`margin-top:3em`, red border) sits on top of it: R12 |
| 2 abspos `blockquote` with float child | `##yyyy##` at y=120, x=108 | `yyyy##` at y=84, x=24 | R12 (margins 36px/60px ignored) + R19 (shrink-to-fit ignores the float, so the float overhangs the left border) |
| 3 forehead (`&nbsp;` line, 1x1 PNG bg) | `#yyyyyyyy#` | nothing (height 0) | R14 (NBSP-only line has no height); its PNG is also `%2F`-escaped base64 (R16) |
| 4-5 eyes: `<object>` fallback chain, fixed backgrounds, float vs block paint order | yellow with two eyes | red with a black/yellow left edge | `<object>` image not rendered (W20, QA21); both 2x2 PNGs fail to decode (R16); `background-attachment:fixed` unsupported (fixed by QA8) |
| 6-9 nose: float, min/max-height, `::before/::after` borders | at y=168 | correct shape, 12px too high | knock-on from the 0-height forehead (R14) |
| 10-11 smile: `.empty` margin collapsing, negative clearance, abspos `width:auto`, nested floats | `#y#yyyyyy#y#` / `#yy######yy#` | wrong shape ~60px low | Q48b + Q74 (margins through empty block, clearance; repro below) and R19 |
| 12 chin (`&nbsp;` line, fixed bg) | `#yyyyyyyy#` | nothing | R14 |
| 13 parser (`background: red pink` must be ignored) | yellow | pink | R15 |
| 14 table (`ul` as `display:table`, anonymous cells) | 4 black cells | 1 cell | Q50a (repro below) |

Not checked: the `:hover` rules on the nose (the face is too broken for a
meaningful hover check).

Repros (each confirmed headless):
- `div{background:red} div{background:none}` paints red (R15).
- `.a{position:absolute;top:0;left:0;margin:36px 0 0 60px;width:48px;height:12px}` paints at (0,0); the same with `position:fixed` too; static and relative boxes get the margin (R12).
- `.a{position:absolute;border-left:24px solid;border-right:24px solid}` containing `<div style="float:right;width:48px;height:12px;background:yellow">`: box is 48px wide (borders only) and the float hangs left over the left border (R19).
- `<div style="line-height:1em">&nbsp;</div>` is 0px tall; `x` instead of `&nbsp;` is 12px (R14).
- `background:url(data:image/png;base64,…4%2F58BAAT%2FAf9j…)` paints nothing; the same PNG with `/` instead of `%2F` paints (R16).
- `.i{position:relative;z-index:2;background:lime}` then a later `position:fixed` (or root-bubbled `absolute`) black box over it: black wins. A later `position:relative` sibling with no z-index loses, correctly (R17).
- `<div style=display:table>` with three `display:table-cell` children and no row: nothing renders. With a plain block between two cells only the block renders (Q50a).
- `.top{height:12px;margin-bottom:4em}` + `.nose` float (`margin:-2em 2em -1em`) + `.empty{margin:6.25em}` containing `<div style="margin:0 2em -6em 4em">` + `.smile{margin:5em 3em;clear:both}`: smile top at y=168, expected 37 (Q48b, Q74).
- `<p><table><tr><td></table><p class=bad>` parses correctly (`p + table + p` matches), so the scalp problem is R12, not the parser.

### R12 — Margins of `position:absolute`/`fixed` boxes are ignored (bug) — FIXED (QA3, bd2945f)

`resolve_positioned` places the border box at `cb_x + left` (or the
static position) and never adds `margin-left`/`margin-top`; `right`/`bottom`
never subtract the margin. Any abspos/fixed box with a margin is misplaced.
Fixed by QA3: `resolve_positioned` now adds the used margins (`left`/`top`: inset
+ margin; `right`/`bottom`: minus margin; static position + margin; `auto` margins
absorb free space after subtracting fixed ones). Acid2 live: scalp bottom border is
now yellow (was red) and line 2 moved to its right place; the rest of the face is
other items.

### R13 — `border-style` defaults to `solid` (bug) — FIXED (QA1, 10e4006)

`BorderStyle`'s `#[default]` variant is `Solid` and `compute_style` uses
`unwrap_or_default()`, so `border: 1px blue` or a bare `border-width`
paints a solid border. The CSS initial value is `none`. Seen live on Acid3:
`* { border: 1px blue }` outlines every element in blue. Fixed by QA1 (default is now `none`;
verified headless, a `*{border:1px blue}` fixture no longer outlines).

### R14 — `&nbsp;` is treated as collapsible white space (bug) — FIXED (QA2 80395ee)

`str::trim()` and `char::is_whitespace` include U+00A0. A line made only
of `&nbsp;` collapses to nothing and its block gets height 0. Sites:
`collect_real_children` (`has_text`), `measure_mixed_inline` (hang test),
the blank-word test in the line-box loop, `:empty`. Fix: QA2.

FIXED (QA2 80395ee): `is_css_ws`/`trim_css_ws` (space, tab, LF, CR, FF only)
replace the Unicode-whitespace calls on rendered text, including
`inline_text`/`normalize_whitespace`, which also used to fold runs of
`&nbsp;` into one space.

### R15 — `background` shorthand doesn't reset omitted parts, accepts junk (bug) — FIXED (QA4 065b61b)

`parse_background_shorthand` skips tokens it doesn't know and the
`background` arm only writes colour/image when given, so `background:none`
leaves an earlier colour, `background: red pink` gives pink, and `fixed`
is dropped silently. Fixed by QA4: the shorthand resets colour/image/position/size/repeat, parses attachment/box keywords (ignored; QA8 will render `fixed`), and any junk, second colour or colour before a comma drops the declaration.

### R16 — `data:` URLs: escaped base64 not decoded, and data: scripts/stylesheets go to wire (bug) — FIXED (QA5, e9232f3)

`decode_data_uri` base64-decodes without percent-decoding first, so
`%2F`/`%3D` payloads (all of Acid2's images) fail. `<script src=data:…>`
and `<link href=data:…>` are fetched through wire, which doesn't decode
them: Acid3's five data: scripts each log `SyntaxError: expecting ';'` at
1:16. Fixed by QA5: percent-decode then forgiving base64; `data:` scripts/stylesheets/@import decoded locally. Acid3 harness: no SyntaxError, T97 passes (29/100, was 28).

### R17 — z-index only orders siblings of the same parent (bug) — FIXED (QY1, ebea7ae)

`paint`/`paint_z_layer` merge `is_explicit_stacking_child` boxes only with
the same box's `positioned` list. An abs/fixed box that bubbled to the root
paints above `position:relative; z-index:2` content anywhere else. Acid2's
intro (`z-index:2`) is painted under the fixed scalp bar. Fix: QA9, now
superseded by QY1 (tree-order tie-break needed, see R20).

### R18 — A fragment in the first URL is ignored (bug) — FIXED (QA6, commit e8fb789)

`scratchy …/wiki/SoHo_Weekly_News#References` and
`http://acid2.acidtests.org/#top` open at the top of the page (seen live,
both `--no-js` and JS on). Clicking a same-document `#top` link works.
Suspected: `apply_fragment` returns true when the target isn't found, so
`pending_fragment` is cleared by an early layout. Cause: `fetch_url_inner` set `pending_fragment` and then showed "Loading…";
that placeholder's complete layout consumed the fragment (`apply_fragment`
returns true for a missing target). Fixed in QA6: held in `nav_fragment`
until the real page arrives. A first attempt that applied on the first
layout containing the target landed mid-page on Wikipedia (earlier content
still growing); it now re-applies until the first complete layout. Live:
Wikipedia `#References` and acid2 `#top` open at their targets.

### R19 — Shrink-to-fit width ignores floats (bug) — FIXED (QA7, 00530a0)

`resolve_positioned` sizes `width:auto` from `cell_natural_width`, which
doesn't count floated children: a box containing only a 48px float is 0px
wide (Acid2 line 2) or takes the whole available width (Acid2 smile).
Fix: QA7: a floated descendant with a px `width` now contributes it to the
intrinsic widths and adjacent floats sum (live: Acid2 head top correct). A
float beside inline text and non-float block/inline-block widths are left
for QA7a/QA7b.

Also seen, not filed separately: with no compositor (`WAYLAND_DISPLAY`
pointing at a dead socket) scratchy panics at `run()`'s
`EventLoop::with_user_event().build().unwrap()`
(`WaylandError(Connection(NoCompositor))`)
instead of printing one line; same shape as QB1, cheap to add to it.

### Acid3 (http://acid3.acidtests.org/)

**2026-11-16 update (real page, live window):** the real flow already ran
all 100 tests (instrumented copy: `update()` called for index 0..100 every
~11 ms, `load` fired, no `js error:` lines, no CPU-budget message), but the
page stayed on "JS ?": `Text.data = x` (`span.firstChild.data = score`)
was a silent no-op because the getter-only `CharacterData.prototype.data`
shadowed the settable `Node.prototype.data`. Fixed (branch
`scratchy/acid3-real-page-runs`): the live page now shows the score
(61/100 at that commit, bucket boxes coloured). Remaining: `Comment.data =`
is still a no-op (`__d_settext` ignores comments); the score is the official
flow's (some tests fail), not the harness's.

Live, through wire, JS on: the page shows "Acid3", "JS ?", the
instructions and "Scripting must be enabled to use this test." The score
never starts: the test is started by `<body onload="update()">`, and
inline `on*` attributes are not wired (QA12). Every element has a blue
outline (R13). stderr: the five `data:` scripts fail (R16), and the
`document.write(...)` script fails with `TypeError: not a function`
(QA13).

Because the real harness never starts, the score below comes from a
per-test harness: the saved page plus 100 extra `<script>`s, each running
`tests[i]()` in try/catch and printing `ACID3 Tnn PASS/FAIL <message>`
through `console.error`, then the score, run with
`scratchy --js-report acid3-harness.html` (each test is its own script, so
each gets its own 5 s budget; nothing hit it). This is NOT the official
flow: no timers between tests, no retry loop (T69 asks for a retry and is
counted as a fail), and nothing is rendered. Observed: **28/100** (same
with `--socket` and a `<base href=http://acid3.acidtests.org/>`).
With the `document.write` markup pasted in by hand (what QA13 would do):
36/100.

| bucket / tests | first failure seen | missing feature | item | cost |
|---|---|---|---|---|
| 0: T0 | `getComputedStyle' of undefined` | `document.defaultView`, `getComputedStyle` | QA16 | M |
| 1: T1-T3, T6, T9, T11-T13 | `contentDocument' of null` | the `document.write` iframe; then `iframe.contentDocument` | QA13, QA18 | S, L |
| 1: T4, T5 | `not a function` | `createNodeIterator`, `createTreeWalker` | QA17 | M |
| 1: T7-T13 | `not a function` / iframe | DOM Range | QA19 | L |
| 1: T8 | `createDocument' of undefined` | `document.implementation` | QA18 | L |
| 1: T10 | PASS | | | |
| 1: T14-T16 | `no <iframe> support`, `<map>` missing | `document.write` | QA13 | S |
| 2: T17 | `hasAttribute' of undefined` | `document.write` (`<map>`/`<form>`) | QA13 | S |
| 2: T18, T24, T28 | PASS | | | |
| 2: T19, T20, T22, T23, T25 | exception constants / no exception | `DOMException` codes, name validation | QA14 | S |
| 2: T21 | `'PREFIX:LOCALNAME'` | `createElementNS` uppercases | QA14 | S |
| 2: T26, T27 | `createDocument' of undefined` | `document.implementation` | QA18 | L |
| 2: T29 | `cloneNode'/'0' of undefined` | `document.write` table, `tBodies/rows/cells` | QA13, QA20 | S, M |
| 2: T30-T32 | `not a function` | `document.createEvent` + `init*Event` | QA15 | S |
| 3: T33-T44, T46, T47 | `contentDocument' of null` | iframe doc + `getComputedStyle(...).zIndex` | QA18, QA16 | L, M |
| 3: T45 | PASS | | | |
| 3: T48 | `getAttribute' of null` | `document.write` | QA13 | S |
| 4: T49-T51 | `initially: tBodies`, `not a function` | table DOM | QA20 | M |
| 4: T52-T59, T62 | `elements`/`name`/`type` wrong | form/input/select/option/button/label DOM | QA20 (Q33e) | M |
| 4: T60, T61 | PASS | | | |
| 4: T63, T64 | `getAttribute'/'match' of undefined` | `document.write` markup, more attribute reflection | QA13, QA20 | S, M |
| 5: T65 | `appendChild' of undefined` | `document.write` | QA13 | S |
| 5: T66-T68, T70 | PASS | | | |
| 5: T69 | retry | support files loaded into iframes/objects | QA18 | L |
| 5: T71-T73 | `missing document for test` | iframe documents | QA18 | L |
| 5: T74-T79 | `contentDocument failed … svg document` | SVG documents in iframe/object, SMIL, SVG fonts | not filed | L+ |
| 5: T80 | `'1' of undefined` | the iframes/objects to remove | QA13, QA18 | |
| 6: T81-T96 | PASS (all 16) | QuickJS | | |
| T97 | `expected 'one' but got 'fail'` | data: scripts | QA5 | S |
| T98 | `createDocumentType' of undefined` | `document.implementation` | QA18 | L |
| T99 | PASS | | | |

Cheap wins in payoff order: QA12 (the test runs at all), QA5 (T97),
QA13 (+8 measured), QA14 and QA15 (about 9 tests by their messages, not
measured), QA17 (T4/T5). Everything else waits on second documents
(QA18), which gates about 35 tests.

## R7 — Fetched images lost when scripts delay NavResult (bug) — FIXED (b40c62a)

**Discovered:** 2026-11-15, user report: navigating to https://wikipedia.org
with JS on loses the globe logo; `--no-js` renders it; a snag'd copy (data:
URIs) works.

Root cause: `process_external_resources` spawns image fetch threads and then
synchronously fetches scripts, so `AppEvent::ImageFetched` (current generation)
can be applied to `image_cache` before `AppEvent::NavResult`; `NavResult` ->
`replace_document` then did `image_cache.clear()`, dropping it. With no
scripts, NavResult wins the race.

Fix: `replace_document` no longer clears `image_cache`; the cache is cleared
where a navigation starts (`fetch_url_inner`), plus `show_message`/`show_help`.
Stale generations are still dropped by `ImageFetched`'s `nav_generation` check;
`restore_saved_page` still restores its saved cache. `NavResult`/`ImageFetched`
bodies moved to `App::handle_nav_result`/`handle_image_fetched` so a test can
drive them. Test: `r7_image_fetched_before_nav_result_survives_document_swap`.

## R5–R6: Q33a script runner bugs seen on https://wikipedia.org — FIXED (R5 e76d162, R6 a56d276; 2026-11-15)

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
