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

- **A single self-contained HTML stream**, not a directory of HTML +
  sibling assets. Matches the "single self-contained local HTML file"
  plan already written down in `scratchy/NOTES.md` before this tool
  existed. As of v2, "file" is really "stream" — `snag` writes to
  stdout, not a path, so the natural shape is a Unix filter
  (`curl url | snag | scratchy`, or `snag url > bundle.html` whenever a
  real file is wanted) rather than a tool that owns its own output
  location.
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

## v2: real pipelines (`curl | snag | scratchy`)

v1 took `snag <url> <output-file>` — functional, but it couldn't do the
thing that actually motivated building this tool: compose with `curl`
and `scratchy` as a plain Unix pipeline, with nothing touching disk.
Fixed by making `snag` a proper filter — `snag [url]`, always writing to
stdout; with no `url` argument, it reads the top-level page's HTML from
stdin instead of fetching it itself (sub-resource fetching — the actual
networking this tool exists for — is unaffected either way).

The one real design question this raised: without a URL argument,
*something* still has to anchor every relative link in the page (a
stylesheet `href="/w/load.php?..."`, an image `src="/thumb/..."`) to an
absolute URL. `<base href>` is the obvious first choice when present,
but most pages (including Wikipedia) don't declare one. The fallback
that made this actually work: `<link rel="canonical">`, which MediaWiki
(and most real sites generally) always emits with an absolute `href` —
confirmed on both the English and German Wikipedia test pages. A page
with neither is a hard error rather than a silent no-op bundle, since
producing a "bundle" that can't actually resolve any of its links isn't
meaningfully better than the raw unprocessed page `curl` alone would've
given.

Verified against the literal pipeline this was built for: `curl -vs
https://de.wikipedia.org/wiki/Arabischsprachige_Wikipedia | snag |
scratchy`. Worked end to end on the first attempt after the rewrite —
curl's 146KB of HTML piped straight into `snag`, which found the page's
canonical link, fetched and inlined everything, piped the 1.4MB bundle
straight into `scratchy`, which rendered real, correctly-styled German
Wikipedia navigation/search/article chrome. No file ever touched disk in
the full three-stage pipeline.

## v3/v4: installing it, and the "this does nothing" scare

Installed both `snag` and `scratchy` to `$PATH` (`cargo install --path
.`) so the pipeline reads as plain commands, not `./target/debug/...`
paths. Found and fixed one rough edge immediately: piping `snag`'s
output into something that closes early (`| head`) panicked with a
`BrokenPipe` backtrace instead of exiting quietly — Rust doesn't
restore default `SIGPIPE` handling, so a closed downstream pipe shows
up as a normal `io::Error` on the next write. Fixed by treating
`BrokenPipe` specifically as a quiet success exit (v3).

Then: running the literal target pipeline — `curl -vs
<german-wikipedia-url> | snag | cargo run` — appeared to do nothing at
all. No window, no output, no error, nothing. Investigated directly
rather than guessing: `ps aux` (not `pgrep`, which kept missing it —
worth remembering, `pgrep -f` can be surprisingly flaky about catching
a process in the narrow window right after it's spawned) showed the
process genuinely *was* running, and a screenshot a bit later showed it
had, in fact, rendered the correct page perfectly. The pipeline was
never broken — it just takes a real, noticeable amount of wall-clock
time (German Wikipedia's test page needed on the order of 240
sub-resource fetches) with `snag` printing *nothing* the entire time,
making a slow-but-working run indistinguishable from a hung one. This
is exactly the "no progress reporting" gap `TODO.txt` had already
flagged as deliberately deferred, now with concrete evidence it
mattered — fixed in v4 with a line per stylesheet inlined plus a
single `\r`-updating counter for every other fetched resource (not a
line per image — a real page can have 200+, which would be its own
kind of unreadable).

## Known limitations / explicitly out of scope

See `TODO.txt` for the fuller list. Headline ones: no `srcset`/responsive
image handling (stripped, not processed), no handling of CSS inside a
`style="..."` attribute's own `url()` references (rare in practice — most
real inline styles are plain property:value pairs, not background images
— not hit on the Wikipedia test), no retry/backoff on a failed fetch (one
attempt, then skip), no size cap or progress reporting for pages with a
very large number of sub-resources.
