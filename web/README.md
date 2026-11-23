# web

A keyboard-driven browser meant to be a daily driver. It connects like any browser by
default; `--proxy <host:port>` (optional, any HTTP forward proxy) sends everything
through a proxy instead. Keeping it off the network except through that proxy is left to
a separate wrapper (DECISIONS.md).

Status: **C1 (engine host) and C2 (the shell's compositor) done**: `web` shows one
page with mouse and keyboard, on a Wayland or an X11 desktop; no keys or status bar
contents of its own yet (C3). The CEF spike (`spike/`, TODO.txt
W1-W7) proved Chromium can run with no network of its own; the plan now is
Architecture C (DECISIONS.md, TODO.txt C1-C8):

- `web-shell`: a small Wayland compositor that draws the status bar and command line and
  owns all input (scratchy's keys, KEYS.md). It runs as one window on a Wayland or X11
  desktop.
- an engine host, picked at start: `web-host-cef` (Chromium, `host-cef/`) first; a
  lighter one (`web-host-webkit`, WebKitGTK) later. It shows one page in one window, the
  page only (tabs are deferred), and is driven by the shell over a control socket
  (`PROTOCOL.md`).

`web` succeeds [`scratchy`](../scratchy/), which is paused (see
`../scratchy/DECISIONS.md`). scratchy's own engine is not carried over; its shell is.

- `KEYS.md`: modes, bindings, completion (from scratchy).
- `STASH.md`: what web keeps on disk: state directory, history and `:go` ranking,
  forget-site, cookies (from scratchy).
- `PROTOCOL.md`: the shell <-> engine-host control protocol (v0).
- `host-cef/`: web-host-cef, its run script and the C1 check (`host-cef/README.md`).
- `shell/`: web-shell, the `web` binary: the compositor and the C2 check (`shell/README.md`).
- `tools/`: `nest.sh` (the nested dwl for live tests, also its Xwayland) and `inject`
  (virtual keyboard and pointer), copied from scratchy.
- `DECISIONS.md`: what has been decided and why.
- `TODO.txt`: the plan (C1-C8, E1) and the spike record (W1-W7), each step with a check that ends it.
- `PROMPTS.md`: the exact prompts that produced this project, in order.

## Install and uninstall

    web/install.sh [--prefix DIR] [--no-build]     # default prefix ~/.local, no root

Builds web/shell and web/host-cef in release mode (`nice -n 19 ionice -c3`; the CEF
download is cached under `$HOME/.cache/web-spike/cef`, see `host-cef/README.md`) and installs
`PREFIX/lib/web/<version>/` (web, web-host-cef and CEF's runtime files together),
`PREFIX/lib/web/current` (symlink to the newest version) and `PREFIX/bin/web` (symlink to
`../lib/web/current/web`). `web` finds `web-host-cef` next to its real executable, so no wrapper or
environment variable is needed. A reinstall writes a new version dir and switches `current`
atomically, so a running `web` keeps working; the last two versions are kept.
`WEB_BUILD_DIR=DIR` puts the cargo target dirs somewhere else (they are ~2 GB). The
coordinator runs it through `coordinator/ci_then_push.sh --install web`. Then:

    web https://example.com

Uninstall (state lives apart, in `~/.local/state/web`, and is kept unless you remove it):

    rm -f ~/.local/bin/web && rm -rf ~/.local/lib/web
    rm -rf ~/.local/state/web      # optional: profile, history, cookies
