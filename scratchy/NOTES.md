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

**Known compromise, deliberately left as-is:** `ul`/`ol`'s default
spacing in `default.css` (`margin: 16px; padding: 24px`) doesn't match
Firefox — real browsers give lists `margin-block: 1em` (vertical-only,
~16px) and `padding-inline-start: 40px` (horizontal-only, for the
marker indent) via their UA stylesheet, but scratchy's box model only
has one scalar margin/padding applied to all four sides, so any
padding value that indents enough horizontally also adds that much
unwanted vertical space (and vice versa) — 24px was tuned as a
compromise, not a fix. The real fix is a per-side margin/padding model
(declined twice in this session in favor of other priorities: once
over font-size/weight/style for v6, once over a list-only indent
hack). Agreed with the user: next PR(s) that touch `default.css`
should rework it against the actual WHATWG rendering spec (§15.3.7
"Lists" has the exact UA rules) properly, which likely means building
real per-side support rather than tuning more constants.

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
  `https://github.com/Loupax/vibe-it.git`) but nothing has been pushed
  since v3 — local `main` is several commits ahead. Pushing hasn't
  been requested; don't push without asking.
