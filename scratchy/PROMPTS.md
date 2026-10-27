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
