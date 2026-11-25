# web

A keyboard-driven browser meant to be a daily driver. It connects like any browser by
default; `--proxy <host:port>` (optional, any HTTP forward proxy) sends everything
through a proxy instead. Keeping it off the network except through that proxy is left to
a separate wrapper (DECISIONS.md).

Status: **C1 (engine host), C2 (the shell's compositor) and C3 (keys and command line)
done**: `web` shows one page with mouse and keyboard, on a Wayland or an X11 desktop,
with scratchy's modes, status bar, `:` command line, ghost completion and history
(KEYS.md), search (C5), the desktop's clipboard (C6), the picker, bookmarks and buffers (C4:
several pages in one host, vim's `:ls`/`:b`/`:bd`). The CEF spike (`spike/`, TODO.txt
W1-W7) proved Chromium can run with no network of its own; the plan now is
Architecture C (DECISIONS.md, TODO.txt C1-C8):

- `web-shell`: a small Wayland compositor that draws the status bar and command line and
  owns all input (scratchy's keys, KEYS.md). It runs as one window on a Wayland or X11
  desktop.
- an engine host, picked at start: `web-host-cef` (Chromium, `host-cef/`) first; a
  lighter one (`web-host-webkit`, WebKitGTK) later. It holds the pages (one per buffer, each in a
  window of its own, the page only; it hides every page but the shown one), and is driven by the shell over a control socket
  (`PROTOCOL.md`).

`web` succeeds [`scratchy`](../archived/scratchy/), which is paused (see
`../archived/scratchy/DECISIONS.md`). scratchy's own engine is not carried over; its shell is.

- `KEYS.md`: modes, bindings, completion (from scratchy).
- `SCRIPTING.md`: `init.js`, global and per profile (`--profile`): user commands, key mappings, start buffers.
- `STASH.md`: what web keeps on disk: state directory, history and `:go` ranking,
  forget-site (`web --forget-site`, `:forget-site`, how the running instance is found),
  cookies (from scratchy).
- `PROTOCOL.md`: the shell <-> engine-host control protocol (v1: several pages, C4).
- `host-cef/`: web-host-cef, its run script and the C1 check (`host-cef/README.md`).
- `shell/`: web-shell, the `web` binary: the compositor and the C2 check (`shell/README.md`).
- `tools/`: `nest.sh` (the nested dwl for live tests, also its Xwayland) and `inject`
  (virtual keyboard and pointer), copied from scratchy.
- `DECISIONS.md`: what has been decided and why.
- `TODO.txt`: the plan (C1-C8, E1) and the spike record (W1-W7), each step with a check that ends it.
- `PROMPTS.md`: FROZEN (user, 2026-11-24): the prompts up to then; add nothing, it is being removed.

## Install and uninstall

    web/install.sh [--prefix DIR] [--no-build]     # default prefix ~/.local, no root

Builds web/shell and web/host-cef in release mode (`nice -n 19 ionice -c3`; the CEF
download is cached under `$HOME/.cache/web-spike/cef`, see `host-cef/README.md`) and installs
`PREFIX/lib/web/<version>/` (web, web-host-cef and CEF's runtime files together),
`PREFIX/lib/web/current` (symlink to the newest version) and `PREFIX/bin/web` (symlink to
`../lib/web/current/web`). `web` finds `web-host-cef` next to its real executable, so no wrapper or
environment variable is needed. It also writes the menu entry
`${XDG_DATA_HOME:-~/.local/share}/applications/web.desktop` (absolute `Exec=PREFIX/bin/web %u`,
`StartupWMClass=web`, no icon, no MimeType: web is not made the default browser or a URL handler), so
the app menu and KRunner find it; the desktop session's PATH has no `~/.local/bin`. Launched from the
menu, the shell's log (stderr) goes to the session journal (`journalctl --user -b`). A reinstall writes a new version dir and switches `current`
atomically, so a running `web` keeps working; the last two versions are kept.
`WEB_BUILD_DIR=DIR` puts the cargo target dirs somewhere else (they are ~2 GB). The
coordinator runs it through `coordinator/ci_then_push.sh --install web`. Then:

    web https://example.com

### Using your own CEF build (H.264) on another PC

The default install uses the stock CEF download, which has no H.264/AAC. A CEF built with
proprietary codecs (`tools/cef-build/`, ~5 h) can be reused on other PCs without rebuilding:
only the CEF directory (~600 MB) moves; `web` and `web-host-cef` rebuild from this repo in minutes.
Keep the archive in private storage only: never in this public repo or its Releases (codec patents).

Pack it on the machine that built it:

    tar -C ~/.cache/web-cef-build -czf cef-156.0.31-codecs.tar.gz dist

Unpack it on the other PC and install (Rust toolchain needed; no Chromium checkout):

    mkdir -p ~/.cache/web-cef-build
    tar -C ~/.cache/web-cef-build -xzf cef-156.0.31-codecs.tar.gz
    CEF_PATH=$HOME/.cache/web-cef-build/dist web/install.sh

`CEF_PATH` must hold `<version>/cef_linux_x86_64/` (with its `archive.json`), and that version
must be the CEF release the `cef` crate in `host-cef/Cargo.toml` is made for: crate 156.7.0 =
CEF 156.0.31 today (the crate has its own numbering). After a crate bump the old archive no longer
fits: rebuild with `tools/cef-build/` (its README says how to find the matching CEF) and pack again.
`coordinator/ci_then_push.sh --install web` uses `~/.cache/web-cef-build/dist` automatically when
it exists. To check H.264 works: `python3 -I web/tools/codec-probe/serve.py 18731 /tmp/probe`,
then `web http://127.0.0.1:18731/probe.html?tag=x` (H264 should be listed on the page). Without `CEF_PATH`, `install.sh` goes back to the stock CEF.

Uninstall (state lives apart, in `~/.local/state/web`, and is kept unless you remove it):

    rm -f ~/.local/bin/web ~/.local/share/applications/web.desktop && rm -rf ~/.local/lib/web
    rm -rf ~/.local/state/web      # optional: profile, history, cookies
