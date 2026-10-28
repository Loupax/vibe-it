# vibe-it

Vibe-coded, suckless-inspired tools — drop-in replacements for things I use but
don't love. Features get added strictly on a per-need basis: no speculative
abstraction, no unused code paths, no building for hypothetical futures.

Every feature is preceded by the exact prompt that generated it, logged
verbatim in that project's `PROMPTS.md`, in order.

## Projects

- [`scratchy/`](scratchy/) — a web browser built from composed, focused Rust
  crates (own HTML/CSS parsing, layout, and rendering) rather than an existing
  browser engine.
- [`snag/`](snag/) — fetches a page and inlines everything it links to
  (stylesheets, `@import`s, images) into one self-contained HTML file, so
  `scratchy` (which never does networking itself) has something real to
  render.
