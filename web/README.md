# web

A keyboard-driven browser meant to be a daily driver. It keeps the vibe-it networking
contract: the browser runs in an empty network namespace and reaches the network only
through [`wire2`](../wire2/), a forward proxy.

Status: **planning.** The CEF spike (`spike/`, TODO.txt W1-W7) proved Chromium can run
with no network of its own; the plan now is Architecture C (DECISIONS.md, TODO.txt C0-C8):

- `web-shell`: a small Wayland compositor that draws the status bar and command line and
  owns all input (scratchy's keys, KEYS.md). It runs as one window on a Wayland or X11
  desktop.
- an engine host, picked at start: `web-host-cef` (Chromium) or `web-host-webkit`
  (WebKitGTK). It shows the page only, one window per tab, and is driven by the shell
  over a control socket.

`web` succeeds [`scratchy`](../scratchy/), which is paused (see
`../scratchy/DECISIONS.md`). scratchy's own engine is not carried over; its shell is.

- `KEYS.md`: modes, bindings, completion (from scratchy).
- `HISTORY.md`: the history file and `:go` ranking (from scratchy).
- `DECISIONS.md`: what has been decided and why.
- `TODO.txt`: the plan (C0-C8) and the spike record (W1-W7), each step with a check that ends it.
- `PROMPTS.md`: the exact prompts that produced this project, in order.
