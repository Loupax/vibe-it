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
