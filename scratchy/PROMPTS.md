# scratchy — prompt log

Append-only log of the exact prompt that produced each feature, in the order
it was built.

---

## v1 — minimal from-scratch renderer

A Rust binary that reads HTML from a file or stdin (in full, batch-style —
not streamed), strips all markup, and displays the plain text content —
word-wrapped — in a native Wayland window. No CSS, no styling, no
networking, no links, no scrolling, no incremental/streaming rendering.

**Requirements:**
1. **Input:** if given one CLI argument, treat it as a file path and read
   HTML from it. If given zero arguments, read HTML from stdin until EOF,
   then proceed — this is a one-shot batch read, not a stream. More than one
   argument → print `usage: scratchy [file]` to stderr, exit 1. Scratchy
   never performs networking itself — fetching is the caller's job
   (`curl url | scratchy`, `nc ... | scratchy`, etc.).
2. **Parsing:** parse the fully-read input with `html5ever` into a DOM tree.
3. **Content extraction:** walk the DOM and collect visible text content in
   document order, with `<script>`/`<style>` contents excluded. No CSS
   parsing, no cascade, no selectors.
4. **Layout:** word-wrap the extracted text to the window's current width
   using one fixed font, one fixed size, one color, via `cosmic-text`
   (shaping, line-breaking, and glyph rasterization).
5. **Window:** a single `winit` window targeting Wayland natively, default
   size 1024x768, resizable. Presentation via `softbuffer` (CPU pixel buffer
   blit) — no GPU/wgpu, no `tiny-skia` yet.
6. **Overflow:** if content is taller than the window, it's simply clipped —
   no scrolling.
7. **Closing the window exits the process cleanly.**
8. **Explicitly out of scope, do not implement:** CSS of any kind,
   hyperlinks/navigation, scrolling, images, forms, JavaScript, networking,
   incremental/streaming parsing or rendering (reading stdin as it arrives,
   repainting as content streams in — that's the *next* feature, logged
   separately).
9. **Project setup:** Cargo binary crate `scratchy`. Confirm current crate
   names/versions for `winit`, `softbuffer`, `html5ever`, `cosmic-text`
   against crates.io rather than assuming.
10. No automated tests for v1 — behavior is directly observable.

**Deliverable:** `main.rs` + `Cargo.toml` implementing exactly the above.

---

## v2 — stream stdin

Rather than reading stdin to completion before any parsing happens, read it
incrementally and repaint as content arrives — the behavior deliberately
deferred in v1.

**Requirements:**
1. **File input is unchanged** — still read fully upfront, same as v1 (no
   streaming benefit for a local file that's already entirely present).
2. **Stdin input becomes incremental:** read stdin in fixed-size byte chunks
   (not line-based — HTML isn't guaranteed to be newline-delimited) on a
   background thread, rather than blocking the whole program on a single
   `read_to_end`.
3. **Incremental parsing:** feed each chunk into html5ever's parser as it
   arrives, using its lower-level incremental API (`process()` per chunk,
   `finish()` at EOF) instead of v1's one-shot `read_from()`.
4. **Wake the event loop on new data:** the background thread reading stdin
   must notify the `winit` event loop when a new chunk has been parsed,
   using an `EventLoopProxy`/user event — the event loop must not block
   waiting on stdin itself, since it also has to keep handling window
   events (resize, close).
5. **Redraw on each chunk:** after each chunk is parsed, re-extract text
   from the (partially-built) DOM from scratch and re-run layout — no
   incremental diffing of only the new nodes. Simplicity over premature
   optimization, same as v1's approach.
6. **No loading indicator, no progress UI** — that's chrome, out of scope.
7. **EOF just means no more updates arrive** — no special "done" state or
   signal to the user.
8. Everything still out of scope per v1 (CSS, links, scrolling, networking)
   stays out of scope here too — this feature is only about *when*
   parsing/painting happens, not *what* gets parsed or painted.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`/
`Cargo.toml` implementing exactly the above.

---

## v3 — CSS parsing/cascade (color only)

**Reference material for this stage:**
- Selector syntax & specificity: CSS Selectors Level 4
  (https://www.w3.org/TR/selectors-4/)
- Cascade algorithm & inheritance: CSS Cascading and Inheritance Level 4
  (https://www.w3.org/TR/css-cascade-4/)
- Structural reference (read for approach, not copied): Servo's `style`
  crate, which solves this exact problem using the same
  `cssparser`/`selectors` crates this stage uses.

**Requirements:**
1. **Collect CSS source:** concatenate the text content of all `<style>`
   elements, in document order, into one stylesheet source. External
   stylesheets (`<link rel=stylesheet>`) stay out of scope — scratchy
   still never performs networking.
2. **Parse with `cssparser`** into simple qualified rules
   (`selector-list { declarations }`). Skip any at-rules encountered
   (`@media`, `@import`, `@font-face`, etc.) and any nested rules — out of
   scope.
3. From each declaration block, parse only the `color` property; every
   other declared property is parsed-and-discarded, not erred on.
4. **Implement `selectors`' `Element` trait** for our DOM node type:
   type/tag selectors, `#id`, `.class`, attribute selectors,
   descendant/child/sibling combinators. Interactive pseudo-classes
   (`:hover`, `:focus`, `:visited`) are out of scope — nothing is
   interactive yet.
5. Also parse each element's inline `style="..."` attribute for a `color`
   declaration, same parsing path.
6. **Resolve the cascade per element:** match all rules against it, sort
   by specificity then source order (later wins ties), apply the winner,
   then let an inline `style` attribute's `color` override that if present
   (inline always wins — no `!important` support needed since we don't
   implement `!important` at all).
7. **`color` inherits:** an element with no winning declaration takes its
   parent's computed color; the root default stays black, as in v1/v2.
8. **Rendering:** split the on-screen text into colored runs following
   each node's computed color, and paint each run with `cosmic-text`'s
   per-span attributes (`set_rich_text` or equivalent) instead of v1/v2's
   single uniform `set_text` call.
9. **Explicitly out of scope:** box model (margin/padding/border/
   background), `display`/layout, fonts/sizes/weights beyond color,
   `@media`/`@import`/`@font-face`, `!important`, multiple cascade
   origins, external stylesheets — all separate, later, logged features.
10. **Streaming is preserved:** as more stdin content and more `<style>`
    rules arrive, re-run cascade and repaint per chunk, same as v2
    already does.
11. **Project setup:** confirm current `cssparser`/`selectors` APIs
    against the installed crate source rather than assuming from
    memory — these two are lower-level and less stable than the v1/v2
    crates were.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`/
`Cargo.toml` implementing exactly the above.

---

## v4 — box model & block/inline layout

**Reference material:**
- CSS Box Model Module Level 3 (https://www.w3.org/TR/css-box-3/)
- CSS Display Module Level 3 — block/inline box generation, anonymous
  boxes (https://www.w3.org/TR/css-display-3/)
- CSS 2.1 Visual Formatting Model / normal flow
  (https://www.w3.org/TR/CSS2/visuren.html,
  https://www.w3.org/TR/CSS2/box.html)
- Structural reference (approach only, not copied): Servo's layout crate

**Requirements:**
1. **Extend the existing cascade** (from v3) to also resolve, per element:
   `display` (`block` | `inline` | `none`), `background-color`, `margin`,
   `padding`, `border-width`, `border-color`. Reuse the same
   selector-matching/specificity/source-order machinery already built —
   don't build a parallel system.
2. **These new properties do not inherit** (unlike `color`): each
   element's value is either explicitly set or its initial/default, never
   taken from its parent.
3. **Default `display` per tag**, since there's no full user-agent
   stylesheet — a small hardcoded table: `html, body, div, p, h1–h6, ul,
   ol, li, blockquote, pre, header, footer, section, article, nav, aside,
   figure, figcaption, table, form, hr` default to `block`; everything
   else defaults to `inline`. Author CSS can override this via the
   `display` property.
4. **`display: none`** means no box at all — the element and its whole
   subtree are skipped in layout and painting, not just invisible.
5. **Lengths:** only `px` and unitless `0` are supported for
   margin/padding/border-width. Percentages, `em`, `rem`, and other units
   are out of scope.
6. **No explicit sizing:** `width`/`height` properties are out of scope —
   every block box takes the full available content width of its
   containing block (normal `width: auto` behavior).
7. **Layout tree:** build a tree of boxes from the DOM. A block-level
   child starts a new box; consecutive inline-level children (including
   text) directly inside a box are grouped into one anonymous inline run,
   laid out as wrapped colored text (reusing the existing cosmic-text
   rich-text path from v3).
8. **Block layout (normal flow only):** child boxes stack vertically,
   each full container width. No margin collapsing — margins are summed
   naively, not collapsed per spec. No floats, no positioning
   (`relative`/`absolute`/`fixed`), no flexbox/grid.
9. **Painting:** for each box, paint `background-color` (if set) across
   its border box, then its border (if `border-width` > 0 — uniform
   solid rectangle strokes on all four sides, no per-side styling, no
   border-radius), then its content (text, or recurse into block
   children).
10. **Streaming is preserved:** re-run layout and repaint from scratch on
    each stdin chunk, same "simplicity over incremental diffing" policy
    as v2/v3.
11. **No scrolling yet** — content taller than the window is still
    clipped; that's v5.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`/
`Cargo.toml` implementing exactly the above.

---

## v5 — scrolling

**Requirements:**
1. Track a vertical scroll offset in pixels, clamped to
   `[0, max(0, total_content_height - window_height)]`.
2. **Input:** mouse wheel only (`WindowEvent::MouseWheel`, handling both
   winit's pixel-delta and line-delta variants). Keyboard scrolling
   (arrows/Page Up/Down/Home/End) is out of scope — a separate future
   feature.
3. **Painting:** shift every box's painted Y position by `-offset`;
   content above/below the viewport is clipped exactly as before.
4. **Re-clamp the offset** whenever the window resizes or content changes
   (new stdin chunks can change total content height).
5. **No scrollbar UI** — no visual indicator of scroll position. That's
   chrome, out of scope like the rest of the browser shell.

**Deliverable:** modifications to `scratchy`'s existing `main.rs`/
`Cargo.toml` implementing exactly the above.
