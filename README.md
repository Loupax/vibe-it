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
- [`wire2/`](wire2/) — a streaming forward proxy whose traffic passes through
  external filters (HTTP servers on Unix sockets), decrypting only hosts a filter
  claims. Planning only so far; see [`wire2/TODO.txt`](wire2/TODO.txt).
- [`snag/`](snag/) — fetches a page and inlines everything it links to
  (stylesheets, `@import`s, images) into one self-contained HTML file, a
  one-shot way to get a first page for the archived `scratchy`.
- [`archived/`](archived/) — deprecated projects kept for the record (`scratchy`, `wire`).
