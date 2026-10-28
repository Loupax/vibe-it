# snag — prompt log

Append-only log of the exact prompt that produced each feature, in the order
it was built.

---

## v1 — fetch a page and inline every resource it references

scratchy (`../scratchy`) deliberately never performs networking itself —
fetching is always the caller's job. That makes testing scratchy against a
real, CSS-heavy page (Wikipedia specifically, as a rendering-fidelity
target) awkward: a plain `curl url -o page.html` only gets the HTML, not
the external stylesheets/images it references, so scratchy would render it
with none of the page's real styling at all (confirmed: scratchy's
`collect_style_css` only reads inline `<style>` tags, `<link
rel="stylesheet">` is ignored entirely). `snag` is the external bundler
tool `scratchy/NOTES.md` already named as the intended answer to this —
not yet built before now.

**Requirements:**
1. **CLI:** `snag <url> <output-file>` — fetch `<url>`, write one
   self-contained HTML file to `<output-file>`. Exactly 2 arguments;
   anything else prints a usage line to stderr and exits 1.
2. **Fetch the page**, parse it with `html5ever`/`markup5ever_rcdom` (same
   stack scratchy itself uses for parsing, used independently here — not a
   shared dependency between the two tools, consistent with each being its
   own focused crate).
3. **Inline every `<link rel="stylesheet">`** — fetch its CSS text and
   replace the `<link>` with an equivalent `<style>` element holding the
   final text, in the same document position (preserves cascade order).
   Resolve `@import` rules recursively (each resolved relative to *its
   own* URL, not the original page's), with a visited-set guard against
   duplicate/cyclic imports.
4. **Rewrite every CSS `url(...)`** (background images, `@font-face` —
   though scratchy has no `@font-face` support to use the latter yet) to a
   `data:` URI, fetched and base64-encoded, so the bundle stays one file
   with no sibling assets. Applies to both newly-inlined external
   stylesheets and the page's own pre-existing inline `<style>` blocks.
5. **Inline every `<img src>`** the same way (base64 `data:` URI),
   dropping `srcset` (unused — scratchy has no responsive-image-candidate
   selection, and nothing else reads it).
6. **Strip `<script>` elements entirely** — scratchy has no JS engine and
   already discards script contents on its own, so shipping the bytes
   anyway would be pure bloat.
7. **Resolve `<a href>` to an absolute URL** (relative links stop making
   sense once the page is moved into a bundle file elsewhere on disk) —
   not followed/fetched, since scratchy has no link navigation yet either,
   just normalized for whenever it does.
8. **A missing/unreachable sub-resource doesn't abort the bundle** — skip
   that one resource (leave the original, now-likely-broken URL in place)
   and keep going. Matches scratchy's own established lenient-parsing
   philosophy, applied here to a networked context instead of a parsing
   one.
9. **CSS `url(...)`/`@import` rewriting is a hand-rolled text scanner, not
   a round-trip CSS parser** — deliberately simpler than pulling in
   `cssparser` for a source-preserving rewrite it wasn't designed for.
   Correct for ordinary real-world CSS; a comment or string literal that
   happens to contain the literal text `url(`/`@import` could be
   misdetected (accepted, documented gap — not expected to come up on
   real pages, including Wikipedia's own machine-generated CSS).
10. **Verify end to end** against a real target: bundle a real Wikipedia
    article (`en.wikipedia.org`) and confirm scratchy renders visible,
    styled content from the result (not just that the file parses) — the
    actual point of building this tool.

**Deliverable:** new standalone crate `snag/` (own `Cargo.toml`,
`src/main.rs`) alongside `scratchy/` in the `vibe-it` monorepo, with its
own `PROMPTS.md`/`NOTES.md`/`TODO.txt` following the same per-project
conventions scratchy already established. Root `README.md` gains a `snag`
entry.

---

## v2 — stdin/stdout, for real Unix pipelines

v1 required a URL argument (to fetch the page itself) and an output-file
argument, which ruled out the exact kind of pipeline the whole tool
exists to enable: `curl -vs <url> | snag | scratchy`. scratchy already
reads HTML from stdin when given no arguments (v1 of scratchy itself);
snag needed the matching other end.

**Requirements:**
1. **`snag [url]`, always writing the bundled result to stdout** — no
   more output-file argument. `snag <url>` still fetches `<url>` itself,
   same as v1; `snag` with no argument reads HTML from stdin instead
   (the top-level page is assumed already-fetched by whatever's piping
   into it — `curl`, `cat`, anything). Sub-resources (stylesheets,
   images) are still always fetched over the network in both modes —
   only the *top-level* page's own fetch is what stdin mode skips.
2. **Resolving relative links without a URL argument**: stdin mode has
   no URL to resolve against, so one has to come from the document
   itself. `determine_base` tries, in order: `<base href>` (resolved
   against the explicit URL if one was given and the `href` is itself
   relative), then the explicit URL itself if `snag <url>` was used,
   then `<link rel="canonical" href>` — real sites (confirmed on both
   English and German Wikipedia) always emit this as an absolute URL,
   so no further fallback is attempted past it. No usable base at all
   (no URL, no `<base>`, no canonical link) is a hard error rather than
   silently leaving every relative link broken — a "bundle" that
   couldn't resolve anything wouldn't be worth producing.
3. **Verified against the exact target pipeline**: `curl -vs
   https://de.wikipedia.org/wiki/Arabischsprachige_Wikipedia | snag |
   scratchy` end to end — curl's HTML (146KB) piped into snag (which
   found the page's `<link rel="canonical">` and used it to resolve
   every relative stylesheet/image reference, producing a 1.4MB bundle)
   piped straight into scratchy, which rendered real, correctly-styled
   German Wikipedia content (navigation, search box, article chrome)
   with no intermediate file ever touching disk. Also re-verified `snag
   <url>` (single-arg, no stdin) still works unchanged against
   `example.com`, and both error paths (bad arg count; stdin input with
   no resolvable base) print a clear message and exit 1 rather than
   panicking or silently producing garbage.

**Deliverable:** `src/main.rs` (`main`'s argument handling reworked
around stdin/stdout, `find_base_href` replaced by `determine_base`/
`find_tag_href`/`find_canonical`).

---

## v3 — exit quietly on a closed downstream pipe

Found while confirming `snag` and `scratchy` are both installed on
`$PATH` (`cargo install --path .`, matching how `scratchy` was already
installed in an earlier session) and spot-checking the installed
pipeline with `| head` to preview output: `snag ... | head -c 200`
panicked with a `BrokenPipe` backtrace instead of exiting cleanly —
Rust doesn't restore the default `SIGPIPE` behavior, so a downstream
reader closing early surfaces as a normal `io::Error` on the next
write, and the final `serialize(...).expect(...)` turned that into a
panic.

**Requirements:**
1. Treat `ErrorKind::BrokenPipe` from the final stdout write as a quiet,
   successful exit (the standard Unix shape for `| head`/`| less`
   closing a pipe early) — any other write error still reports and
   exits 1, same as before.
2. **Verified**: `snag ... | head -c 200` now exits 0 with no panic/
   backtrace; the full, untruncated pipeline (`curl | snag | scratchy`)
   still works unchanged.

**Deliverable:** `src/main.rs` (final `serialize` call's error handling).

---

## v4 — progress reporting, so a slow run doesn't look hung

Both `snag` and `scratchy` got installed to `$PATH` (`cargo install
--path .`) for real `curl | snag | scratchy`-style use. Running the
exact target pipeline (`curl -vs <wikipedia-url> | snag | cargo run`)
looked like it did nothing — no visible window, no output, no error.
Direct investigation found it actually *was* working the whole time:
the fetch+bundle step genuinely takes several seconds to tens of
seconds (German Wikipedia's test page alone needed ~240 sub-resource
fetches — the bulk of the wall-clock cost), and `snag` printed nothing
at all while that happened, so a slow-but-working run was
indistinguishable from a hung one. `TODO.txt` had already flagged "no
progress reporting" as a known, deliberately-deferred gap before this —
this is the first real evidence it actually mattered.

**Requirements:**
1. **A line to stderr for each stylesheet being inlined**, and for the
   top-level page fetch in `snag <url>` mode — few enough of these in
   practice (~15 stylesheets on the Wikipedia test) that one line each
   is readable signal, not noise.
2. **A single, `\r`-updating counter for every other fetched
   sub-resource** (images, and anything pulled in via a CSS `url()`) —
   a real page can reference 200+ of these, so a line per fetch would
   scroll past unreadably fast; a single self-overwriting "inlined N
   resources so far..." line gives the same "it's still working"
   signal without the noise. Implemented as a plain `AtomicU32` counter
   (`fetch_as_data_uri` increments it and reprints) — the program is
   single-threaded, so this is really just a simple global counter, not
   a serious concurrency primitive.
3. **The two don't visually collide**: a stylesheet message can land
   mid-way through the previous stylesheet's own `url()` fetches (while
   the counter's `\r`-line is still active), so `progress_line` drops
   to a fresh line first whenever the counter is non-zero, rather than
   running the message text into whatever digit the counter last
   showed.
4. **Verified**: stderr captured and inspected with `cat -A` to confirm
   the `\r` structure is correct (each counter update genuinely
   overwrites in place on one terminal line — `^M` between each segment
   — while each stylesheet message starts its own clean line); stdout
   re-confirmed to stay pure HTML throughout (`head -c 60` on the
   output still starts with `<!DOCTYPE html>`, nothing from the
   progress output leaking into the bundle); full pipeline re-verified
   working end to end afterward.

**Deliverable:** `src/main.rs` (`RESOURCE_COUNT` static counter,
`progress_line` helper, call sites in `main` and `process_node`'s `link`
branch, the counter increment+print in `fetch_as_data_uri`).

---

## v5 — true streaming: no DOM, no buffering, byte-exact pass-through

User feedback on v1-v4's design: "I want it to do the following: For
every tag that doesn't fetch resources output it as is. For every tag
that fetch resources, fetch the resources and push the incoming body to
stdout. When done send the closing tag." Followed by: images should be
base64'd but "don't make assumptions or take action to enforce a
format — if the incoming html has a closing tag, you send it, if it
doesn't you don't" (i.e. mirror the source's own tag structure, don't
normalize it); `<script>` passes through unchanged now, not stripped;
and, mid-build: "ideally snag shouldn't buffer anything."

This is a different architecture from v1-v4, not a tweak: that version
parsed the *entire* document into an `html5ever`/`RcDom` tree, walked it
to fetch and mutate every resource, and only then serialized the *whole*
tree to stdout in one shot — meaning zero output until literally
everything (a real page's 200+ sub-resources) had already been fetched,
and a full in-memory tree held the whole time. `html5ever::serialize`
also normalizes structure as it writes (canonical quoting, `<!DOCTYPE
html>` casing, etc.), which is the opposite of the byte-exact
pass-through now being asked for.

**Requirements:**
1. **A hand-rolled streaming tag scanner replaces `html5ever`/
   `markup5ever_rcdom` entirely** (both dependencies dropped). Reads one
   tag at a time (quote-aware — a `>` inside a `"..."`/`'...'` attribute
   value doesn't end the tag; comment-aware — `<!-- ... -->` scans for
   the literal `-->` instead) and either copies it to stdout immediately
   byte-for-byte, or — for `<link rel=stylesheet>`, `<style>`, `<img
   src>` — fetches and substitutes, flushing right after. Every other
   tag, all plain text, the doctype, comments, and (per the explicit
   answer) `<script>` in full (src included, never fetched) pass through
   completely untouched — exact original quoting, self-closing slashes,
   attribute order, and whether a tag the spec calls void happened to
   have a stray closing tag in the source or not (mirrored, never
   fabricated or dropped).
2. **`<style>`/`<script>` are HTML5 "raw text" elements** — content
   isn't scanned as tags at all, only a literal `</style`/`</script`
   (case-insensitive) ends them, so `if (a < b)` inside a `<script>`
   can't be misread as a tag open.
3. **`<img src>` streams its replacement, never buffers the image**:
   `base64::write::EncoderWriter` wraps stdout directly, and
   `io::copy` pipes the HTTP response body straight through it — the
   image's bytes exist in memory only as whatever chunk is in flight,
   never as a whole file. (CSS-internal `url()`s, rewritten while
   splicing an in-memory CSS string, still buffer each referenced
   resource fully — there's no "stream into" target there the way an
   HTML attribute's output position gives one.)
4. **Base URL discovery for stdin mode, done honestly**: a relative
   `href`/`src` needs *something* absolute to resolve against, and
   `<link rel=canonical>` — confirmed on real Wikipedia output — comes
   *after* the page's own `<link rel=stylesheet>` tags in `<head>`, so
   there's no way to resolve the first stylesheet without having looked
   further ahead than that tag itself. `discover_base` is the one
   deliberate, bounded exception to "don't buffer anything": it reads
   (and holds) only the `<head>` section — a few KB on a real page,
   nowhere near whole-document size — scanning for `<base href>` first,
   `<link rel=canonical>` second, stopping at `</head>` or a 1 MiB
   safety cap either way. Once found (or not), the held bytes are
   replayed through the exact same streaming processor via
   `Read::chain`, so nothing already read is lost, and everything after
   `<head>` streams with no further buffering at all. A page with no
   base source isn't fatal (fixed after an initial build hit this:
   `example.com`, which fetches nothing, used to hard-fail for no
   reason) — it's a warning, and only an actual relative-URL resolution
   attempt with nowhere to resolve against falls back to passing that
   one tag through unresolved.
5. **Verified**: `curl -s example.com | snag` now produces byte-for-byte
   *identical* output to the source (confirmed with `diff` — nothing to
   fetch, nothing touched) where v1-v4 would have reformatted it through
   `html5ever`'s own serializer. A hand-written test page exercising a
   `<script>` containing literal `<`/`>` and a fake-looking tag inside a
   string, a void `<img>` with no closing tag, quoted attribute values
   containing `>`, and an HTML comment containing `>` all round-tripped
   correctly. A CSS/image streaming test (`httpbin.org`, self-closing
   `<img .../>`) confirmed attribute order and quoting around a rewritten
   `src` stays untouched, and that the piped-in image bytes decode to a
   valid image. The real Wikipedia pipeline re-verified end to end
   (`curl | snag | scratchy`) — same correct render as v1-v4, but
   `scratchy`'s window now appears within a few seconds instead of
   waiting for the entire ~240-resource fetch to finish first, since
   output starts flowing as soon as the first processed tag is ready.
   `BrokenPipe` handling (v3) re-verified unaffected by the rewrite.

**Deliverable:** `src/main.rs`, rewritten (the `RawTag`/tag-scanning
machinery, `discover_base`, `stream_body`, `stream_data_uri`'s streaming
base64 encoder; `process_css`/`inline_imports`/`rewrite_urls`/
`fetch_text`/`fetch_as_data_uri`/progress reporting carried over
unchanged, since CSS-text-level and `fetch_as_data_uri`'s own buffering
were already correctly scoped and didn't need to change). `Cargo.toml`
drops `html5ever`/`markup5ever_rcdom`.

---

## v6 — stream CSS-embedded resources too, not just `<img src>`

User noticed, after watching the live `"inlined N resources so far..."`
progress output: the resources being counted weren't actually appearing
on stdout as they were inlined — expected per v5's whole premise
("shouldn't buffer anything"). Found the gap directly: v5's `<img src>`
handling genuinely streamed (`stream_data_uri`, `base64::write::
EncoderWriter` wrapping stdout), but CSS `url()` handling (the majority
of resources on a real page — 227 of ~240 on the Wikipedia test, mostly
icons embedded as stylesheet background-images) still went through
`fetch_as_data_uri`, which fully built each resource as a base64
`String`, spliced it into an in-memory CSS string via two full
`inline_imports`/`rewrite_urls` passes, and only then had the *entire*
rewritten stylesheet written to stdout in one `write_all` — the deeper
buffering layer `TODO.txt` had already flagged as a known, accepted
exception, now actually removed rather than left as a documented
tradeoff.

**Requirements:**
1. **Merge `inline_imports`+`rewrite_urls`'s two sequential full-text
   passes into one single-pass scanner** that writes straight to an
   `out: &mut impl Write` as it goes, instead of building and returning
   a `String`. Plain CSS text between constructs is written immediately;
   `@import` recurses into `process_css` itself, now also writing
   directly to the same `out` rather than building a separate string to
   splice in; each `url(...)`'s resource routes through `stream_data_uri`
   (the same zero-buffer encoder `<img src>` already used) instead of
   the old `fetch_as_data_uri`, which is now unused and deleted entirely.
2. **A real resource is never built as a whole base64 string anywhere
   in the program anymore** — `stream_data_uri` is now the *only* path
   a fetched resource's bytes take to reach stdout, for both `<img src>`
   and every CSS `url()`, image or otherwise.
3. **Verified**: `curl -s example.com | snag` still produces
   byte-for-byte identical output (nothing regressed for the
   nothing-to-fetch case). Re-ran the real Wikipedia pipeline — same
   output size (1,455,084 bytes), same resource count (240), same
   correct render through `scratchy`; spot-checked that CSS-embedded
   icons (227 of them, confirmed via `grep -c 'url("data:...;base64,'`)
   are still valid, correctly-encoded images, not just that the count
   matched.

**Deliverable:** `src/main.rs` (`process_css` rewritten to the merged
single-pass streaming scanner; `inline_imports`/`rewrite_urls`/
`fetch_as_data_uri` all deleted, fully superseded).

---

## v7 — `SNAG_TIMING=1`: real numbers instead of guessing at a "hangup"

User report: "an obvious hungup even when I call snag directly with a
URL" (i.e. `snag <url>`, not piped through `curl`/`scratchy` — the
simplest possible invocation, nothing else in the loop). Rather than
guess at a cause, added real instrumentation first and measured —
`scratchy/NOTES.md` already documents this same measure-don't-guess
habit for that project's own performance questions, applied here to a
new one.

**Requirements:**
1. **`timed_get`**, one function every actual HTTP request in the
   program now goes through (replacing direct `agent.get(url).call()`
   at all 3 call sites: the top-level `snag <url>` fetch, `fetch_text`,
   `stream_data_uri`) — times each request, always accrues it into a
   running total (`REQUEST_COUNT`/`REQUEST_NANOS`, a couple of atomic
   adds, cheap enough to leave on unconditionally), and — only when the
   `SNAG_TIMING` environment variable is set — prints each request's
   method/status/latency/URL immediately via `progress_line` (so it
   can't visually collide with the `\r`-updating resource counter).
2. **A one-line summary at the very end of a run** (also gated on
   `SNAG_TIMING`): total requests, total time spent in HTTP calls,
   average per request, and total wall-clock time — letting a slow run
   be read as either "many fast requests, serially, add up" or "one
   request is actually stuck," which are very different problems.
3. **Measured the actual report directly**: `SNAG_TIMING=1 snag
   <wikipedia-url>` — 244 requests, 5235ms total in HTTP calls (21ms
   average), 5893ms wall clock. No individual request stood out (max
   117ms); sequential HTTP latency alone accounts for ~89% of the total
   time. Not a hang, not a stuck request, not a regression from the v5/
   v6 streaming rewrite — just 244 round trips, one at a time, each
   individually fast but adding up (the serial-fetching limitation
   `TODO.txt` already flagged, now with real numbers instead of a
   guess). A network with meaningfully higher per-request latency than
   this environment's would turn the identical pattern into something
   that genuinely reads as hung — concurrency is the next lever if that
   turns out to be the case, not attempted here.
4. **Verified no regression to default behavior**: `curl -s example.com
   | snag` (no `SNAG_TIMING` set) still produces byte-for-byte identical
   output — the per-request accounting is free enough to run always, but
   only prints anything when explicitly asked for.

**Deliverable:** `src/main.rs` (`timing_enabled`, `timed_get`,
`print_timing_summary`, `REQUEST_COUNT`/`REQUEST_NANOS` statics, all 3
`agent.get(...).call()` call sites switched to `timed_get`).

---

## v8 — push `<script defer>` to the end of the page

User question, explicitly framed as "not for now": would it make sense
to push `defer` scripts to the end of the page before inlining? Pointed
out it's a valid exception to "don't buffer anything" — not buffering
the document, just holding a few specific tags' bytes in memory until
later. Confirmed worth doing and asked for it directly: "do it now and
I think we are done with snag."

The reasoning that makes this sound rather than just a convenient
reordering: `defer` already means a real browser defers that script's
*execution* until after parsing finishes, in document order relative to
other deferred scripts — so moving its *position* to just before
`</body>` changes nothing about real-browser behavior anywhere this
bundle might end up. For `scratchy` specifically (no JS engine — a
script's content is dead weight either way) the only thing that
actually matters is *where in the byte stream* that dead weight sits:
moving it out of the way means scratchy's streaming renderer reaches
real visible content sooner instead of reading through however much
inert script came first in the source.

**Requirements:**
1. **`RawTag::is_deferred_script`**: `<script>` (opening, not closing)
   with `defer` present and `async` absent — `async` takes priority over
   `defer` per spec when both are set, and carries no ordering guarantee
   of its own, so a script with both isn't moved.
2. **Held back in a `deferred: Vec<Vec<u8>>`** (tag + raw-text content +
   closing tag, one entry per deferred script, in the order
   encountered) instead of being written at its original position —
   passed through `stream_body`'s signature, not a global, so each
   invocation starts clean.
3. **Flushed (in original relative order — real browsers run deferred
   scripts in document order, so this has to be preserved) right before
   `</body>`**, or at true end of input if `</body>` never shows up at
   all (handles a truncated/body-less document the same defensive way
   every other EOF case in this file already does).
4. **Verified**: a hand-written test page with four script shapes (plain
   blocking, `defer` with external `src`, inline `defer`, `defer="defer"`
   with external `src`, and `async` alongside a separate non-deferred
   script) — only the three real `defer` cases moved, in their original
   relative order, landing right before `</body>`; the blocking and
   `async` scripts stayed exactly where they were; content that
   originally came *after* a deferred script in the source correctly
   ended up *before* the deferred scripts in the output. The no-`</body>`
   fallback verified separately with a truncated document. Re-verified
   zero regression on both `example.com` (still byte-identical output)
   and `motherfuckingwebsite.com` (still byte-identical — neither page
   uses `defer`), and re-ran the real Wikipedia pipeline (which also has
   no `defer` scripts) to confirm an identical byte count to the
   pre-change baseline.

**Deliverable:** `src/main.rs` (`RawTag::is_deferred_script`,
`stream_body`'s new `deferred` parameter and its dispatch branch,
`flush_deferred`, both call sites in `main`).

This was flagged as likely the last `snag` feature for now — see
`NOTES.md` for where things stand.
