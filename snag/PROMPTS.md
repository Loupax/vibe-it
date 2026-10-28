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
