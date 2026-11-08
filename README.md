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
- [`wire/`](wire/) — a small daemon that owns real networking for
  `scratchy`'s live interactivity (link clicks, form submissions):
  reads one HTTP request per connection off a Unix socket, executes it
  for real (TLS, redirects, decompression), and hands back one
  normalized HTTP/1.1 response.
