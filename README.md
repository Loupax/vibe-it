# vibe-it

Vibe-coded, suckless-inspired tools — drop-in replacements for things I use but
don't love. Features get added strictly on a per-need basis: no speculative
abstraction, no unused code paths, no building for hypothetical futures.

Prompts are no longer logged (user, 2026-11-24): the why behind a decision goes
in the project's `DECISIONS.md` (with the user's words), an item's result in its
PR and commit message. The existing `PROMPTS.md` files are frozen: add nothing
to them; they are being removed.

## Projects

- [`web/`](web/) — a keyboard-driven daily-driver browser: a small Wayland
  compositor shell (scratchy's keys) around an engine host (Chromium via CEF, or
  WebKitGTK), pointed at a web proxy. Planning only so far; see [`web/TODO.txt`](web/TODO.txt).
- [`scratchy/`](scratchy/) — **paused 2026-11-23 in favour of `web/`.** A web browser built from composed, focused Rust
  crates (own HTML/CSS parsing, layout, and rendering) rather than an existing
  browser engine. It runs page JavaScript (V8) with a DOM, timers, ES
  modules, and `fetch`/XHR/`sendBeacon`. Video/audio play through libmpv when it is installed.
  A plain `cargo build --release` / `cargo install --path .` gives V8 + media (first clean build
  ~15 min, ~63 MB stripped binary; V8 downloads a pinned prebuilt library at build time).
- [`wire2/`](wire2/) — a streaming forward proxy whose traffic passes through
  external filters (HTTP servers on Unix sockets), decrypting only hosts a filter
  claims. Planning only so far; see [`wire2/TODO.txt`](wire2/TODO.txt).
- [`snag/`](snag/) — fetches a page and inlines everything it links to
  (stylesheets, `@import`s, images) into one self-contained HTML file, a
  one-shot way to get a first page for `scratchy`.
- [`wire/`](wire/) — a small daemon that owns all real networking:
  reads one HTTP request per connection off a Unix socket, executes it
  for real (TLS, redirects, decompression), and hands back one
  normalized HTTP/1.1 response. It also owns the cookie jar.

## How they fit together

`scratchy` never does networking itself. Page loads, sub-resources, link
clicks and every request made by page JavaScript go through `wire`, over the
socket given with `--socket`, under a request policy (browser CORS rules, no
mixed content, bad ports blocked, a warning for loopback/private/`file:`
targets, size/time/count limits). Without `--socket` nothing is fetched.
See [`scratchy/NETWORKING.md`](scratchy/NETWORKING.md).

`wire` keeps an in-memory cookie jar, on by default (`--no-cookies` turns it
off). A request uses it only when `scratchy` asks, the jar is partitioned by
top-level site (like Firefox's Total Cookie Protection), and it is lost when
`wire` restarts.

## Usage

```sh
# start the networking daemon (binds $XDG_RUNTIME_DIR/wire.sock by default)
wire &

# open a page; clicking links and page scripts fetch through wire
scratchy --socket "$(wire --print-socket)" https://example.com/

# a one-shot bundled first page from snag
snag https://example.com/ | scratchy --socket "$(wire --print-socket)"

# scripts off
scratchy --no-js page.html
scratchy --socket "$(wire --print-socket)" --user-agent 'Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0' https://example.com/   # or SCRATCHY_USER_AGENT; needs a current wire

# headless: run a file's scripts and print console, errors, network, cookies
scratchy --js-report page.html
scratchy --js-report --socket "$(wire --print-socket)" --run-timers 2000 page.html

# headless: the web-platform-tests reftest subset (see scratchy/AGENTS.md)
scratchy --wpt /path/to/wpt
```

Inside `scratchy`, `:help` lists the keys and commands.
