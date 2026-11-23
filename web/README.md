# web

A keyboard-driven browser meant to be a daily driver. It connects like any browser by
default; `--proxy <host:port>` (optional, any HTTP forward proxy) sends everything
through a proxy instead. Keeping it off the network except through that proxy is left to
a separate wrapper (DECISIONS.md).

Status: **planning.** The CEF spike (`spike/`, TODO.txt W1-W7) proved Chromium can run
with no network of its own; the plan now is Architecture C (DECISIONS.md, TODO.txt C1-C8):

- `web-shell`: a small Wayland compositor that draws the status bar and command line and
  owns all input (scratchy's keys, KEYS.md). It runs as one window on a Wayland or X11
  desktop.
- an engine host, picked at start: `web-host-cef` (Chromium) first; a lighter one
  (`web-host-webkit`, WebKitGTK) later. It shows the page only, one window per tab, and is driven by the shell
  over a control socket.

`web` succeeds [`scratchy`](../scratchy/), which is paused (see
`../scratchy/DECISIONS.md`). scratchy's own engine is not carried over; its shell is.

- `KEYS.md`: modes, bindings, completion (from scratchy).
- `STASH.md`: what web keeps on disk: state directory, history and `:go` ranking,
  forget-site, cookies (from scratchy).
- `DECISIONS.md`: what has been decided and why.
- `TODO.txt`: the plan (C1-C8, E1) and the spike record (W1-W7), each step with a check that ends it.
- `PROMPTS.md`: the exact prompts that produced this project, in order.
