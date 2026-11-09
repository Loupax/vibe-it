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

## How they fit together

```sh
# 1. start the networking daemon once (binds $XDG_RUNTIME_DIR/wire.sock
#    by default)
wire &

# 2. bundle a page with snag for the initial render, then open it in
#    scratchy, passing wire's socket (via `wire --print-socket`, rather
#    than hardcoding wire's own default-path logic a second time) so
#    clicking a link navigates live
snag https://example.com/ | scratchy --socket "$(wire --print-socket)"
```

`snag` only produces the *first* page — it's a one-shot bundler, not
involved in live clicks. With a reachable `wire` socket, clicking an
`<a href>` inside `scratchy` builds a real HTTP/1.1 request and fetches
it through `wire` directly, no `snag` involved — and, for a live-
navigated page specifically, scratchy also fetches that page's own
non-inlined images and external stylesheets the same way, since `snag`
never touches a page reached by clicking a link. Without `--socket` (or
if `wire` isn't reachable), a click just prints the request it would
have sent to stdout instead of fetching anything.
