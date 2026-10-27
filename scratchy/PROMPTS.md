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
