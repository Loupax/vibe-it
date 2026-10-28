# snag — working notes

## What this is for

`snag` exists purely to feed `scratchy` real, representative test pages
without giving scratchy itself any networking — scratchy's own stance
(see `scratchy/NOTES.md`, "Priority going forward: rendering, not
networking") is that fetching is always the caller's job. Before this
tool, the only option was `curl url -o page.html`, which gets the HTML
but none of its external CSS/images — scratchy's `collect_style_css`
only ever reads inline `<style>` tags, so a raw-saved page rendered with
*zero* of its real styling. `snag` closes that gap: one command producing
one self-contained HTML file scratchy can be pointed at directly.

## Design choices

- **Single self-contained output file**, not a directory of HTML +
  sibling assets. Matches the "single self-contained local HTML file"
  plan already written down in `scratchy/NOTES.md` before this tool
  existed, and keeps the whole point (`scratchy bundled.html`, one
  argument) simple.
- **CSS inlined as plain text** (`<style>` elements), not data URIs —
  no reason to base64-bloat text content when it can just be pasted in
  directly. **Images inlined as base64 `data:` URIs** instead, since
  there's no text form for binary image data and the point is staying
  one file.
- **Fonts are not specially fetched.** `@font-face` `src: url(...)`
  references go through the same generic CSS `url()` rewriter as any
  other CSS url — they *will* get base64-inlined if reachable — but
  scratchy has no `@font-face` support at all yet (only system fonts via
  `fontdb`), so a font that successfully inlines still does nothing once
  scratchy loads the bundle. Not worth special-casing (skipping) until
  scratchy can actually use one; harmless either way, just some dead
  weight in the bundle today.
- **A hand-rolled text scanner for CSS, not `cssparser`.** Reusing
  `cssparser` (already proven, used by both this tool's own HTML parsing
  stack and scratchy itself) was the first instinct, but it's a
  parser-consumer API — built for extracting meaning from CSS, not for
  reconstructing the original text with only specific tokens (`url(...)`,
  `@import`) surgically replaced and everything else byte-identical.
  Doing a real round-trip rewrite with it would've meant writing a
  parallel CSS serializer. A plain substring scan (`@import`, `url(`)
  with careful quote-aware boundary finding is simpler, and correct for
  essentially all real-world CSS — including Wikipedia's own, which is
  machine-generated/minified, not hand-written to contain adversarial
  comments. Known gap, accepted rather than engineered around: a comment
  or string literal containing the literal text `url(` or `@import`
  could be misdetected.
- **`<script>` stripped outright**, not just left unfetched. scratchy has
  no JS engine and already discards script contents wherever they came
  from, so carrying the bytes into the bundle is pure file-size waste for
  zero benefit (real pages can ship megabytes of JS).

## First real test: Wikipedia

Bundled `https://en.wikipedia.org/wiki/Rust_(programming_language)` end
to end and pointed scratchy at the result. It worked on the first real
attempt: the output file parsed, scratchy didn't crash on ~2MB of
real-world HTML+CSS, and — the part that actually mattered — the page
rendered with real Wikipedia styling applied, not scratchy's bare UA
stylesheet: the title, "Jump to content" link (properly blue/underlined),
and the full sidebar navigation (Main menu, Navigation, Contribute
sections with their real link lists) all appeared correctly styled. This
is strong, concrete evidence that scratchy's existing CSS support (color,
text-decoration, lists, box model, inheritance/cascade) already covers a
meaningful slice of a real, non-trivial, un-simplified page once it
actually *has* the real CSS to work with — the `<link>`-ignoring gap was
genuinely the first blocker, exactly as suspected before building this.

Numbers from that one run: ~2MB output file, 15 `<style>` blocks (1
original + ~14 newly-inlined external stylesheets), 0 `<script>` tags
left, 241 embedded `data:` images (mostly SVG icons — Wikipedia's chrome
leans heavily on inline SVG — plus PNG/JPEG), 0 real `rel="stylesheet"`
`<link>`s left un-inlined (only harmless MediaWiki metadata links
remained: `mw-deduplicated-inline-style`, `mw:PageProp/Category`,
`dns-prefetch`, `canonical`, etc. — none of them things scratchy reads
anyway). Fetch+bundle took ~7 seconds end to end.

**Expected and not yet investigated further**: the sidebar navigation
renders *above* the article content as one long single column, not
beside it — scratchy has no positioning model at all (no `float`, no
`position`, no flexbox/grid), and Wikipedia's actual two-column layout
depends entirely on those. This was the predicted next gap going into
this work, not a surprise from it. Images don't paint (scratchy has no
`<img>` box model/painting yet either) even though the bundle correctly
carries their bytes as data URIs ready for whenever that lands. Couldn't
scroll down to inspect further in this environment — no input-injection
tool available here (`ydotool`/`wtype`/`wlrctl` all absent) to synthesize
a scroll event, same limitation noted for cursor-icon testing in
`scratchy/NOTES.md`.

## Known limitations / explicitly out of scope for v1

See `TODO.txt` for the fuller list. Headline ones: no `srcset`/responsive
image handling (stripped, not processed), no handling of CSS inside a
`style="..."` attribute's own `url()` references (rare in practice — most
real inline styles are plain property:value pairs, not background images
— not hit on the Wikipedia test), no retry/backoff on a failed fetch (one
attempt, then skip), no size cap or progress reporting for pages with a
very large number of sub-resources.
