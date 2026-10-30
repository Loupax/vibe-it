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
- **`<script>` passes through completely unchanged** (reversed from an
  earlier "strip it outright" decision — see v5: the user explicitly
  asked for scripts to stay, not be stripped). scratchy still has no JS
  engine and still never does anything with script content either way,
  so this is purely about matching the requested "every non-fetching
  tag is untouched" model, not about scratchy needing the bytes for
  anything.

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

## v5: rearchitected as a true streaming filter

Direct user feedback on v1-v4's whole design, not a bug report: "For
every tag that doesn't fetch resources output it as is. For every tag
that fetch resources, fetch the resources and push the incoming body to
stdout. When done send the closing tag." Then, specifically on images
(which have no text "body" to stream and no closing tag to speak of):
base64 them, but don't invent structure that wasn't in the source —
mirror whatever the original tag's closing-tag situation actually was.
Then, mid-build: "ideally snag shouldn't buffer anything."

This landed as a full rewrite, not a patch. v1-v4's whole design was
batch: parse the *entire* document into an `html5ever`/`RcDom` tree,
walk it to fetch and mutate every single resource, *then* serialize the
*whole* tree to stdout in one `html5ever::serialize` call. That meant
zero bytes of output until literally everything — a real page's 200+
sub-resources — had already been fetched, with the complete in-memory
tree held the whole time. It also meant output was never byte-identical
to input even where nothing changed: `html5ever::serialize` normalizes
as it writes (canonical attribute quoting, `<!DOCTYPE html>` casing,
etc.) — directly in tension with "mirror the source structure, don't
invent or drop a closing tag."

Replaced the whole `html5ever`/`markup5ever_rcdom`-based tree approach
with a hand-rolled streaming tag scanner (dropped both as dependencies
entirely) that reads one tag at a time and either copies it to stdout
immediately, byte-for-byte, or — for the three tags that actually fetch
something (`<link rel=stylesheet>`, `<style>`, `<img src>`) — fetches
and substitutes in place, flushing right after. `<style>`/`<script>`
needed their own care: HTML5 defines them as "raw text" elements whose
content isn't tag-scanned at all (only a literal `</style`/`</script`
ends them), which matters a lot for `<script>` specifically — real JS
routinely contains `<`/`>` (`if (a < b)`) that would otherwise be
misread as a tag open by a naive scanner.

The one piece that couldn't be made fully zero-buffer, for a real
structural reason rather than convenience: resolving any relative
`href`/`src` needs an absolute base URL to resolve *against*, and in
stdin mode (no URL argument) that has to come from the document itself
— `<base href>` or `<link rel=canonical>`. Checked where `<link
rel=canonical>` actually sits in a real page's `<head>`
(`curl -s <wikipedia-url> | grep -o '<link[^>]*>'`) and confirmed the
worst case directly: Wikipedia's own `<link rel=canonical>` comes
*after* its `<link rel=stylesheet>` tags, so there is no way to resolve
the first stylesheet without having looked further ahead in the
document than that tag itself — a genuine causality problem, not a
design shortcut. Solved with one small, bounded, explicitly-justified
exception: buffer just the `<head>` section (a few KB on a real page,
nothing like whole-document size) long enough to find a base, then
replay those exact bytes through the same streaming processor via
`Read::chain` before continuing live with zero further buffering. Also
caught and fixed along the way: a first version treated "no base found"
as fatal for the *whole run*, which broke `example.com` (fetches
nothing at all, so it never needed a base in the first place) — fixed
to warn and continue, only falling back to passing a tag through
unresolved at the exact point a resolution attempt actually has nowhere
to resolve against.

`<img src>` got the biggest win from the rewrite: `base64::write::
EncoderWriter` wraps stdout directly and `io::copy` streams the HTTP
response body straight through it, so an image's bytes are never held
in memory as a whole — not even the large ones. CSS-internal `url()`s
inside a stylesheet's text still buffered each referenced resource
fully at this point (splicing a rewritten reference into an in-memory
CSS string had no equivalent "stream directly into output" position) —
flagged as a narrower, clearly-scoped exception at the time, not a
loophole, but it didn't stay that way long (see v6, next).

Verified the headline claim directly: `curl -s example.com | snag`
produces output *byte-for-byte identical* to the source (checked with
`diff`) — nothing to fetch, nothing touched, not even re-quoted. A
hand-written stress-test page (a `<script>` with literal `<`/`>` and a
fake-looking tag inside a JS string, a void `<img>` with deliberately no
closing tag, a quoted attribute value containing `>`, an HTML comment
containing `>`) round-tripped correctly on every count. Re-verified the
real Wikipedia pipeline end to end — same correct render as before, but
`scratchy`'s window now appears within a few seconds rather than after
the entire resource fetch completes, which was the actual point.

## v6: the CSS-url() buffering v5 left behind turned out to matter

Flagged in v5 as a deliberately narrow, accepted exception — then caught
almost immediately after, by the user actually watching the new
`"inlined N resources so far..."` progress counter and noticing the
resources it counted weren't showing up on stdout as it went. They
were right: on the Wikipedia test page, 227 of ~240 resources are CSS
`url()` references (mostly icons embedded as stylesheet
background-images), and v5's `<img src>` path streamed genuinely, but
the CSS path didn't — `fetch_as_data_uri` fully built each one as a
base64 `String`, spliced it into an in-memory CSS string across two
full-text passes (`inline_imports` then `rewrite_urls`), and only once
an *entire stylesheet* was done did its whole rewritten text get written
to stdout in one piece. So the vast majority of what the progress
counter was counting wasn't actually reaching stdout incrementally at
all — exactly the gap the user's question caught.

Fixed by merging the two full-text passes into one single-pass scanner
that writes directly to `out` as it scans (plain CSS text between
constructs written immediately, `@import` recursing into the same
streaming `process_css` rather than building a separate string to
splice in) and routing every CSS `url()`'s resource through
`stream_data_uri` — the exact same zero-buffer encoder `<img src>`
already used — instead of the old buffering `fetch_as_data_uri`, which
is now gone entirely. A fetched resource's bytes now reach stdout the
same way everywhere in the program, image tag or stylesheet reference,
with nothing in between holding a whole one in memory. Re-verified the
Wikipedia pipeline produced the identical byte count and resource count
as before, and spot-checked that CSS-embedded icons are still valid,
correctly-encoded images, not just that the numbers matched.

## v7: measuring a reported "hangup" instead of guessing at it

User report: an obvious hang-up even running `snag <url>` directly —
no `curl`, no `scratchy`, nothing else in the loop that could be at
fault. Rather than theorize (serial fetching already being a known,
documented limitation made it tempting to just assume that was the
whole story), added real instrumentation first: `SNAG_TIMING=1`, which
times every HTTP request through one shared `timed_get` function (the
only place any request in the program is actually made), always
accruing the total/count cheaply, and — only when asked for — printing
each request's latency plus a final summary.

Ran it against the exact case reported: `SNAG_TIMING=1 snag
<wikipedia-url>`. Result: 244 requests, 5235ms total time spent in HTTP
calls (21ms average), 5893ms wall clock — so ~89% of total time is
accounted for by request latency, and no single request stood out (the
slowest was 117ms, nothing close to "hung"). In this environment, at
least, this isn't a hang or a stuck request or a regression from the
v5/v6 streaming rewrite — it's genuinely just 244 sequential round
trips, individually fast, adding up because nothing fetches
concurrently yet (already flagged in `TODO.txt` before this, now
measured rather than assumed). A network with meaningfully higher
per-request latency than this one would turn the exact same pattern
into something that reads as actually hung, which is the most likely
explanation for what was reported — the tool now exists to confirm that
on whatever network it's actually run from, rather than guessing from
here.

## v8: `<script defer>` reordering, and probably the last snag feature

A genuinely good idea the user raised unprompted, explicitly flagged as
"not for now" at first — asked whether it'd make sense to push `defer`
scripts to the end of the page before inlining. The reasoning holds up:
`defer` already tells a real browser to run that script after parsing
finishes, in document order among other deferred scripts, so relocating
it to just before `</body>` is a complete no-op for real-browser
behavior. For `scratchy`, which has no JS engine at all, a script's
*content* was always dead weight regardless of position — the only
thing that actually mattered was where in the byte stream that dead
weight sat, potentially in front of real visible content the streaming
renderer wants to reach as soon as possible.

The user pre-empted the obvious objection before it was even raised:
this isn't "buffering" in the sense `snag`'s whole v5 rewrite moved away
from — it's holding onto a handful of specific tags' bytes, not the
document as a whole, the same bounded-exception shape as `discover_base`
already uses for `<head>` scanning. Implemented exactly that way: a
`deferred: Vec<Vec<u8>>` threaded through `stream_body`, one entry per
`<script defer>` (not `async` — takes priority over `defer` per spec and
has no ordering guarantee of its own to preserve), flushed in original
relative order right before `</body>` (or at true EOF if the document
never has one, same defensive shape as every other truncated-input case
in this file).

Verified with a hand-written page exercising all four real shapes
(blocking, external-src `defer`, inline `defer`, `defer="defer"`, plus
an `async` script to confirm it's correctly left alone) — order
preserved, non-deferred content correctly ended up ahead of the
relocated scripts in the output, and the no-`</body>` fallback checked
separately. Re-confirmed zero regression on both prior real-world test
pages (still byte-identical) and the Wikipedia pipeline (identical byte
count — it happens to have no `defer` scripts at all, so this change is
invisible on that specific page, but the mechanism is now proven on a
page that actually exercises it).

The user's own words closing this out: "I think we are done with snag."
Nothing currently blocks `scratchy`'s own next steps (`float`, image
painting) on anything further here — see `scratchy/TODO.txt` for where
that work picks back up.

## v9: `--serve`, a long-lived daemon for scratchy's link navigation

`scratchy` wants to support clicking an `<a href>` to navigate, but it
has zero networking code of its own by design (see `scratchy/NOTES.md`
— fetching is always the caller's job). Spawning a fresh `snag <url>`
subprocess per click would work, but pays full process startup every
time; a daemon `scratchy` can keep writing URLs to and reading bundles
back from amortizes that. This is the `snag`-side half of a two-part
plan worked out directly with the user — `scratchy`'s own click-handling
is a separate task.

**Architecture**: the one-shot `snag <url>` CLI branch used to inline
its fetch-then-stream logic directly in `main`. That logic is now
`bundle_url(url, agent, out)` — fetches `url`, runs `discover_base` on
the response body (the same bounded, one-time `<head>`-buffering trick
already used for stdin mode, just now also used here — see the
`<base href>` section below for why), replays the buffered head through
`Read::chain`, then calls `stream_body` exactly as before. Both the
plain CLI (`main`'s own `args.get(1)` branch) and `--serve`'s
per-connection handler call this one function — neither has its own copy
of the fetch/inline logic, so they can't drift apart.

**`--serve`'s own plumbing** (`run_server`/`handle_connection`,
std-only, no new crate dependency — `std::os::unix::net::{UnixListener,
UnixStream}` is sufficient): binds a Unix domain socket at
`socket_path()` — `$XDG_RUNTIME_DIR/snag.sock` when set (the
conventional per-user, per-login-session socket location on Linux;
systemd gives every login session a private tmpfs directory here,
auto-cleaned on logout, the same place `ssh-agent`/`gpg-agent`/
`pipewire` put their own sockets), falling back to `/tmp/snag-$UID.sock`
otherwise (still scoped per-user via the uid in the filename, just
without the logout auto-cleanup a real `XDG_RUNTIME_DIR` gives — the uid
itself comes from a bare `extern "C" { fn getuid() -> u32; }` FFI
declaration rather than pulling in the `libc` crate for one syscall,
since every Rust binary on Linux already links against the C library
that provides it). A stale socket file left behind by a crashed/killed
previous `--serve` is removed before binding (otherwise `bind` would
fail with "address in use" forever, for no good reason — a socket file
with nothing listening behind it is inert).

Then `listener.incoming()` is served one connection at a time, forever,
with no concurrency (matching this file's already-serial per-resource
fetch model; no reason to add a second kind of concurrency just for the
socket layer when the thing each connection does — `bundle_url` — is
itself entirely sequential already). Each connection: read one line (an
absolute URL) via `BufReader` over a `try_clone`'d handle (so reading
and writing can happen through two independent handles on the same
underlying socket without fighting over a single buffered reader/
writer), call `bundle_url` with the *original* `stream` as the output
`Write`, then return — dropping `stream` closes the connection, which is
the protocol's entire "done" signal (deliberately as simple as possible:
no length-prefixing, no explicit end marker, just "read until EOF," the
same semantics the one-shot CLI's own process exit already has today).
A bad URL or a failed top-level fetch is logged to stderr and the
connection just closes (possibly with zero bytes already written) —
*not* a `process::exit`, since that would be fatal to a daemon serving
other clients; this mirrors the lenient "skip the broken piece, keep
going" philosophy this file already applies to a single missing
sub-resource, now applied one level up, to the whole top-level fetch, 
specifically for the long-lived mode where a single process-wide exit
isn't an option. No caching across requests (explicitly out of scope
for v1, see `TODO.txt`) — every request still does the full independent
fetch+inline work.

**`<base href>` emission** (the other half of this item, independent of
`--serve` itself, done in both modes): `discover_base` already computed
a usable base URL for resolving resource references internally; the gap
was that it never got written back into the emitted HTML, so a relative
`<a href>` — passed through completely untouched, unlike `link`/
`style`/`img` — had nothing to resolve against downstream. Fixed by
having `stream_body` write `<base href="...">` immediately after the
`<head>` tag it already passes through, computed from whatever `base`
value it was given (the discovered one in stdin mode, or the fetched
URL itself in `snag <url>`/`--serve` mode — unchanged from what already
resolved resources in each mode, so this is purely additive to the
*output*, not a change to *resolution* behavior) — unless the real page
already declares its own `<base href>` in `<head>`, tracked via a new
`has_own_base_tag` field on `discover_base`'s result.

That required one change to `discover_base` itself: it used to return
as soon as it found *any* usable base source (`<base href>` or
`<link rel=canonical>`, whichever came first in document order) and
never looked further. That's fine for picking the resolution value, but
wrong for answering "does this page have its own `<base href>`" — a
`<base>` tag appearing *after* a `<link rel=canonical>` in the markup
would never even be seen. `discover_base` now always scans through to
`</head>` (or the existing 1 MiB cap) regardless, tracking the
resolution `base` (first-found-of-either-type still wins, unchanged
priority) and `has_own_base_tag` (true the moment any `<base>` tag with
an `href` attribute is seen, anywhere in the head, independent of
whether that `href` happened to win the resolution race) separately.
Still a small, bounded, one-time buffer — the same shape as before, just
scanned a little further on the pages where the first version would
have stopped early.

The other consequence: `bundle_url` (the `snag <url>`/`--serve` path)
now calls `discover_base` too, which it never did before this change —
v5's own design note ("`snag <url>` already knows its base the instant
it starts — no lookahead needed, full streaming from byte one") was true
at the time, but answering "does this page already have its own
`<base href>`" inherently needs a look at `<head>` regardless of mode.
The buffer is the same small, bounded, one-time `<head>`-sized one
`discover_base` already used for stdin mode — not a return to v1-v4's
whole-document buffering — but it does mean `snag <url>` now holds a few
KB before the very first byte streams out, where it previously streamed
from byte one. Judged an acceptable, deliberate tradeoff for this
feature, not an oversight.

**Tests**: `snag` had no automated tests before this — every prior
feature was verified by hand against real pages. Added `#[cfg(test)]
mod tests` at the bottom of `main.rs` (the first one), matching the
in-file convention `scratchy/src/lib.rs` already established, rather
than inventing a different test setup for this project. Five tests:
`<base href>` correctly reflecting a discovered canonical link; a page
with its own `<base href>` not getting a second one injected (and
specifically not leaking the canonical link's own URL into an injected
one); a page with no base source at all getting no `<base>` tag and not
treating that as fatal; `--serve`'s output over a real Unix socket
confirmed byte-for-byte identical to calling `bundle_url` directly for
the same URL, using a tiny in-process loopback HTTP fixture
(`std::net::TcpListener` on `127.0.0.1:0`, one thread, same canned
response for every connection) rather than touching the real internet;
and the plain one-shot `bundle_url` path confirmed to emit `<base href>`
on its own. All run through `cargo test`, no manual steps needed for
these checks on future changes.

**Verified for real, by hand, beyond the automated tests**: installed
(`cargo install --path .`), ran `snag --serve &`, confirmed the socket
appeared at `$XDG_RUNTIME_DIR/snag.sock`. No `socat`/`nc`/`ncat`
available in this environment, so wrote a throwaway Python Unix-socket
client to connect, send a URL, and collect the response. Requested
`https://example.com` through it and diffed the result against a plain
`snag https://example.com` run moments later — byte-for-byte identical,
both carrying `<base href="https://example.com/">`. Sent a second
request for a different URL over the same still-running server to
confirm sequential-but-persistent serving (not exit-after-one-request).
Re-ran the real German Wikipedia pipeline through the plain one-shot CLI
to confirm it's unaffected beyond now also carrying a `<base href>`
reflecting the article's own URL. Also fed a hand-written page with its
own `<base href>` through stdin mode directly to confirm it passes
through unmodified with no second `<base>` injected and no spurious "no
base found" warning. Killed the test server afterward and confirmed its
socket file was gone.

## Known limitations / explicitly out of scope

See `TODO.txt` for the fuller list. Headline ones: no `srcset`/responsive
image handling (stripped, not processed), no handling of CSS inside a
`style="..."` attribute's own `url()` references (rare in practice — most
real inline styles are plain property:value pairs, not background images
— not hit on the Wikipedia test), no retry/backoff on a failed fetch (one
attempt, then skip), no size cap or progress reporting for pages with a
very large number of sub-resources.
