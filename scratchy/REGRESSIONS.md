# scratchy — regressions

## R20: www.youtube.com is a blank window (Opus investigator, 2026-11-15) — blank window FIXED by QY1 (b75b39d)

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

### R20 — An earlier `position:absolute` box paints over later positioned boxes whatever their z-index (bug) — FIXED (QY1, b75b39d)

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

## R12–R19: Acid2 / Acid3 audit (Opus investigator, 2026-11-15) — OPEN

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

### R13 — `border-style` defaults to `solid` (bug) — FIXED (QA1)

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

### R17 — z-index only orders siblings of the same parent (bug) — FIXED (QY1, b75b39d)

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

## R8–R11: build-test pass on `main` @ `f3b06a8` (Opus investigator, 2026-11-15) — OPEN

**R11 DONE** (QB4), **R10 DONE** (QB6), see TODO.txt. R8, R9 still open.

Investigation only, no code changed. Fix items are QB1–QB10 at the top of
the TODO.txt QUEUE. Everything below was seen in a real window and
screenshotted, unless it says otherwise.

### How this was tested (reusable recipe)

The old `AUTOSCROLL`/`AUTOHOVER` env vars no longer exist in `src/`; they
were temporary instrumentation. Real input can be driven without touching
the user's desktop:

- Nested compositor: `env -u DISPLAY WLR_BACKENDS=wayland WLR_RENDERER=pixman
  setsid timeout 3600 dwl -s 'echo $WAYLAND_DISPLAY > disp' &`. This opens a
  dwl window (1052x1050 here) inside the user's dwl and prints its socket
  name (e.g. `wayland-1`). `WLR_BACKENDS=headless` also works and is fully
  invisible, but dwl then never advertises seat capabilities
  (`wl_seat.capabilities(0)`), so clients get no pointer or keyboard. Use the
  `wayland` backend when input is needed.
- Run scratchy in it: `WAYLAND_DISPLAY=wayland-1 timeout 120
  target/release/scratchy --socket /run/user/1001/wire.sock <url>`.
  Screenshot with `WAYLAND_DISPLAY=wayland-1 grim out.png`.
- Input: a ~150-line Rust tool (scratchpad only, not committed; QB10 asks
  to commit it). It uses `wayland-client` 0.31, `wayland-protocols-wlr`
  (`zwlr_virtual_pointer_v1`: `motion_absolute`, `button` 0x110,
  `axis_discrete`, `frame`) and `wayland-protocols-misc`
  (`zwp_virtual_keyboard_v1`: `keymap` from `xkbcomp -xkb` of
  `pc+us`/`evdev`, `key` with evdev codes, `modifiers` with Shift=1 and
  Control=4). It takes commands like `move:X,Y click scroll:N key:j key:c-d
  type::go\ URL key:ret wait:MS`. Both protocols are dwl globals.
- Never `pkill -f <pattern>` from a shell whose own command line contains
  the pattern: it kills that shell. Kill PIDs you started.

Tested (all in the nested dwl, release build of `f3b06a8`): `:go` by
typing; wikipedia.org, en.wikipedia Main_Page, SoHo_Weekly_News,
news.ycombinator.com, html.duckduckgo.com/html, github.com, bbc.com/news,
each with JS on and `--no-js`; `:set nojs` + `:refresh`; mouse-wheel and
`Ctrl+d` scrolling; hover sweeps; link clicks (portal language button,
"Commons" sister link); `h`/`l` history incl. rapid `l h l h`; `:help`;
window resize (tiling a second client, then closing it); unreachable host
via wire; missing wire socket; image loading after rapid navigation (images
present every time, no R7 recurrence); idle CPU after load (0% on HN,
portal, Main_Page, SoHo, JS on and off); stderr closed early. Not tested:
`Ctrl+v` paste, `Tab` completion, `<select>`, keyboard focus with `Tab`,
streaming stdin mode, HiDPI/fractional scale, X11.

### R8 — Main-thread stalls get scratchy disconnected by the compositor, then it panics (bug) — OPEN

**User report:** stderr `Io error: Broken pipe (os error 32)` then
`thread 'main' panicked at src/lib.rs:197:34: called Result::unwrap() on an
Err value: ExitFailure(1)`.

**What the message is:** `Io error: …` is `wayland-backend`'s own
`WaylandError::Io` printed by its `log_error!` (it falls back to `eprintln!`
because scratchy doesn't enable the `log` feature) when writing to the
Wayland socket fails. `ExitFailure(1)` comes from winit's Wayland
`pump_events`: `self.connection.flush().is_err()` -> `set_exit_code(1)`;
`run_app` turns that into `Err(EventLoopError::ExitFailure(1))` and `run()`
in `lib.rs` does `event_loop.run_app(&mut app).unwrap()`. So the compositor
closed scratchy's connection. scratchy itself writes nothing to a pipe here.

**Reproduced exactly, twice:**
1. Kill the (nested) compositor while scratchy runs: identical two lines and
   the same panic.
2. Compositor alive (dwl, libwayland 1.24): a page whose `setTimeout`
   callback busy-loops 1.5 s, plus pointer motion over the window during the
   stall. 200 motion events: survives. 400 motion events: disconnected,
   same panic. The compositor drops a client whose socket buffer fills up
   because the client isn't reading. winit always binds
   `zwp_relative_pointer_v1` (`listen_device_events` is a no-op on Wayland),
   so each mouse move costs about 3 events.
3. Real page, no busy loop: en.wikipedia.org/wiki/Main_Page with the pointer
   moving every 2 ms (a 500 Hz mouse) during load and hover: **crashed 4/4
   runs**, 3 with JS on and 1 with `--no-js`. SoHo_Weekly_News and the
   portal survived the same test.

**Measured stalls** (gaps between consecutive `wl_pointer.motion` events
received, from `WAYLAND_DEBUG=1` timestamps, pointer moving every 20 ms,
`--no-js`, machine load ~8 on 8 cores, so absolute numbers are inflated):
portal ~180 ms per gap, SoHo ~530 ms, Main_Page 1.3–1.8 s. They recur
about every 2 s while the pointer moves. That is `HOVER_RESTYLE_INTERVAL`
pacing: every hover restyle runs `build_layout_tree_with_resources` (DOM
walk + cascade) synchronously in `App::redraw`. Other synchronous main-thread
work that can stall the same way: `handle_nav_result` (HTML parse +
`run_page_scripts`, up to `SCRIPT_BUDGET` = 5 s), and every timer or event
callback, each with its own 5 s budget.

**Fix:** QB1 (never panic; exit cleanly with a clear message) — DONE
(637fef5) — and QB3 (bound how long the main thread stays busy while
input is flowing) — DONE (dbf1610): hover restyles wait for the pointer to
be still 150 ms when the last build took >50 ms, timer/event callbacks
get 500 ms. Live (2 ms motion, 20 s, Main_Page): before 0/3 survive,
after 7/7 (JS on and off); busy-loop page: before dies, after survives.
Remaining stalls (the cascade itself, 350 ms Main_Page, 1.65 s SoHo first
build, when started by image arrival/JS/resize) go to Q32.

### R9 — JS DOM writes rebuild without the restyle path: periodic writes leave the page blank or missing text (bug, Q33c `80b5e0a`) — FIXED (QB2 d78587f)

**Repro:** a local copy of SoHo_Weekly_News with `<base href>` plus
`setInterval(function(){document.body.setAttribute('data-tick',
String(Date.now()))}, 300)` before `</body>`, loaded with `--socket`:
**the page stays completely blank** (screenshot after 25 s). With a 1000 ms
interval the header and infobox image show, but **every paragraph of body
text is missing**, and scratchy sits at ~150% CPU. The same file without the
interval renders fully. Real pages do this all the time (clocks, carousels,
ad rotators, "x minutes ago" updaters).

**Root cause:** `App::js_sync` sets `layout_dirty = true` and bumps
`layout_generation`, but never sets `restyle_requested`. So `redraw`
queues a plain rebuild: `restyle = false`, `hold_old_frame = false`,
`reuse = false`. The `AppEvent::ShapeResult` handler then shows every
*incomplete* progressive pass (`self.cached = Some(*laid)` when
`incomplete` and not holding). The `layout_generation` bump in `js_sync` also
makes the in-flight shaping result stale on arrival. When a DOM write lands
more often than a full shaping pass takes, no complete frame ever lands:
the screen shows an early partial pass (missing text) or nothing at all.
The hover path (`set_interaction_state`) already does this right: it sets
`restyle_requested` and `keep_scroll_on_rebuild`, and `hover_restyle_delay`
coalesces while `hold_old_frame || layout_dirty`.

Related, seen once: after `:set js` + rapid `l h l h` back to Main_Page,
the word "squat" was missing mid-sentence ("a type of toy with a / body")
for 20+ s. Not reproduced in 3 more tries or after `:refresh`. It looks like
the same "incomplete pass left on screen" symptom. Re-check after QB2.

**Fix:** QB2. `js_sync` now takes the restyle path and `layout_in_flight`
coalesces writes to one rebuild per complete shaping pass. Measured with the
recipe above (release, 20 s in): 300 ms timer: blank page, ~190% CPU before;
full article incl. paragraphs, ~100% CPU (one core, back-to-back rebuilds)
after. 1000 ms timer: header+image but no paragraph text, ~190% before; full
article, ~45-60% after. No-timer page: 0% idle. wikipedia.org renders with
globe. The `squat` symptom above was not re-checked live.

### R10 — The previous page's JS engine stays alive under `:help` (bug, Q33t `bec5507` / Q33d `46dfc45`) — DONE (QB6)

**Repro:** a page with `document.addEventListener('keydown', …)` (logs each
key, `preventDefault`s `j`) and a 500 ms `setInterval` writing a body
attribute. Run `:help`, press `j` four times: stderr logs `old page keydown
:` and `old page keydown j` x4, so the old page still gets the keys and can
cancel the vim bindings. Worse, the help page **renders with its whole
second half missing**: the Commands table, the "Also demonstrated" heading
and the list text are gone, and the image sits alone above the `<hr>`. It
stays that way. The same `:help` from that page with `--no-js` renders
completely. Mechanism: the old timers' DOM writes call `js_sync` -> R9
rebuild churn on the help document.

**Root cause:** `App::show_help` swaps the document via `replace_document`
but leaves `self.js` (the old page's `JsEngine`) in place. `about_to_wait`
keeps running its timers, `js_key`/`js_click` keep dispatching into it, and
its dirty flag keeps triggering rebuilds of the help page.
`restore_saved_page` has no engine to restore either, because `SavedPage`
has no `js` field.

**Fix:** QB6 (done): the engine now lives in `SavedPage.js` while help is shown.

### R11 — User report: wikipedia.org "Thank you for donating recently!" dialog shows and Close does nothing — DONE (QB4; part (c) = QB5, fixed)

**(a) Why the dialog was visible.** Not reproducible on `f3b06a8`, with JS
on or off. A full-window screenshot of https://wikipedia.org shows no
dialog, and a `:refresh` doesn't bring it back. Facts:
- The markup is `<dialog class="frb-iad-dialog">` with no `open` attribute.
  The page CSS never sets `display` on it, so the UA rule
  `dialog:not([open]) { display: none; }` (`src/default.css`, Q33c2
  `1b02dd5`) hides it.
- The page's JS never opens it on load. `showModal()` is only called from
  the click handler of the banner's "I already donated" link (`.frb-iad`),
  in `index-be66aee782.js` around column 23586. The banner itself only
  becomes visible (`banner--visible`) for geo US/CA/GB/IE/AU/NZ, `en`, in
  2026.
- Before Q33c2 (`1b02dd5`, committed 14:20) every closed `<dialog>` rendered
  in flow. Q33c2's own TODO entry describes exactly this dialog. The
  installed `~/.cargo/bin/scratchy` (14:43) contains the rule.
- So the user was almost certainly running a process started from a binary
  older than `1b02dd5`. A live window check on the current install closes
  this.

**(b) Why Close did nothing.** Close is a plain `<button autofocus
class="frb-dialog-action frb-iad-dialog-close">`, not in a `<form
method=dialog>`. The page wires it with
`querySelectorAll(".frb-iad-dialog-close").forEach(e =>
e.addEventListener("click", () =>
document.querySelector(".frb-iad-dialog").close()))`. Q33d's click dispatch
does reach buttons: in a fixture, a `<button>` click listener logged on a
real injected click. But `HTMLDialogElement` has no `close`, `showModal`,
`show`, `open` or `returnValue` in `js_dom.rs`'s `DOM_PRELUDE`. Fixture
output: `typeof showModal undefined typeof close undefined`. Clicking a link
whose handler calls `showModal()` logs `Uncaught TypeError: not a function`.
On the old binary the dialog also had no `open` attribute to remove, so
Close could never hide it. Fix: QB4 (dialog API plus modal rendering).

**(c) The portal's remaining `TypeError: not a function` at
`index-be66aee782.js:1:18546`** (the only `--js-report` error left on the
portal). It is not related to the Close button. Source around that column,
inside a `doWhenReady` (DOMContentLoaded) handler:
`let t=i("searchInput"),a=i("searchLanguage");if(t){void 0===t.autofocus?t.focus():window.scroll(0,0);…}a.addEventListener("change",…)`.
`autofocus` isn't reflected, so it is `undefined`, and that takes the
`t.focus()` branch. `Element.prototype.focus` doesn't exist, so the call
throws. Fixture: `typeof el.focus`, `el.autofocus` and `typeof
window.scroll` are all `undefined`. Because it throws, the rest of that
handler never runs: the `change` listener on the search-language `<select>`
and the `<link rel=preconnect>` for the chosen language. Fix: QB5 (`focus`/
`blur`, `autofocus` reflection, `window.scroll/scrollTo/scrollBy`).

### Other things seen (filed or noted)

- Unreachable host: wire answers `502` with a `text/plain` body (`wire:
  upstream request failed: … Dns Failed: … Name or service not known`).
  scratchy shows only `can't render this content type: "text/plain"`,
  because `dispatch_wire_response` rejects every MIME type except HTML and
  images. The user never sees the reason. QB7. (A missing wire socket is
  handled well: a clear message, and the request is printed to stdout.)
- bbc.com/news with JS: 13 of 14 errors are `Error: chunk path empty but
  not in a worker` from Next.js/turbopack chunks. The runtime reads
  `document.currentScript` (`if(e)return{src:e.getAttribute("src")}`), which
  doesn't exist. QB8. (The page still looks better with JS than without:
  without JS its nav drawer renders expanded.)
- stderr closed early (`scratchy … 2>&1 | head -1`, page logging every
  200 ms): exit code 101, a panic. `eprintln!` panics on `EPIPE`. Not the
  user's crash (that message is different), but the same family. QB9.
- Portal, JS on, after clicking "Read Wikipedia in your language": the
  language list renders as one long column. The CSS is
  `.langlist>ul{column-width:11.2rem}`, and CSS multi-column isn't
  implemented. With `--no-js` the list is inline rows, which reads better.
  It also covers the button's own text. Not queued as a bug, since
  multi-column was never claimed. A new feature item would be M-sized.
- Hover after a fast 50-point sweep: the final hovered link is correct
  once the coalesced `HoverFlush` lands, about 2 s later.
- Main_Page/SoHo/Commons header icons render as dark grey squares (mask
  images, Q35a, known). Tofu for Bengali/Tamil/etc. is a missing system
  font, not a scratchy bug. SoHo at a ~470 px tile width renders infobox
  rows as one run-together line ("TypeAlternative weeklyFounder…");
  unverified against a real browser, not filed.
- Error stacks for events dispatched from Rust say `at <eval> (timers:1:1)`,
  because `JsEngine::eval_value` names every eval `timers`. Cosmetic, folded
  into QB8.

---

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
