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
