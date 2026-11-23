# web-shell (`web`)

The browser's shell (web/TODO.txt C2; Architecture C in `../DECISIONS.md`): a small
Wayland compositor built on smithay, shown as **one window** on the user's desktop
through winit (a Wayland window on a Wayland desktop, an X11 window on an X11 one). It
starts the engine host (`web-host-cef`, the only one for now) with `WAYLAND_DISPLAY` set
to its own socket, so the page is a Wayland client of the shell on either desktop, and
drives it over the control socket it listens on (`../PROTOCOL.md`).

The binary is called `web`, the name the user types (`web --forget-site`, STASH.md,
already assumes it). The crate is its own workspace root, like host-cef; unlike
host-cef it is in CI (it needs only libxkbcommon to link).

## What C2 does

- **The page** is the host's toplevel, paired with its `tab-created` event (either can
  come first; `src/pairing.rs`). One page only (DECISIONS.md "No tabs yet"): the first
  parentless toplevel is the page; any other toplevel is shown centred over it as a
  dialog. The page is configured to the shell window minus the 24 px status bar at the
  bottom (KEYS.md "Status bar"), maximized and tiled (`src/layout.rs`). C2 draws the bar
  as a plain strip; C3 draws its contents. The window title is the page title.
- **Input**: every key goes to the focused window (the page, a dialog, or a popup that
  grabbed), every pointer event to the surface under it (`src/input.rs`). C3's modes
  hook into the keyboard filter there. When the shell window loses the desktop's
  focus, the page loses keyboard focus and its activated state.
- **Popups** (xdg_popup: `<select>` menus, context menus, tooltips) are placed by their
  positioner, constrained to the whole shell window (a menu may cover the bar).
  Chromium does not ask for popup grabs; it closes its menus itself (Esc, a click
  outside). Grabs are honoured if a client asks.
- **JS dialogs and permission bubbles** are drawn by CEF inside the page window (no
  surface of their own), so they need nothing from the shell.
- **Cursors**: named shapes (cursor-shape-v1, which Chromium uses) are passed to the
  desktop through winit; an image cursor (CSS `cursor: url(...)`) arrives as a surface
  and the shell draws it, hiding the desktop's cursor over the page.
- **Fullscreen**: when the page asks (`requestFullscreen`), the shell window goes
  fullscreen on the desktop and the page gets all of it (no bar). Leaving fullscreen
  from the page (Esc) or from the desktop's window manager both end it.
- **Lifetime**: closing the shell window, SIGTERM or SIGINT send `quit` to the host;
  the shell exits when the host has (killing it after 5 s). If the host exits on its
  own, the shell exits too.

Not in C2 (later items): the status bar's contents and keys (C3), clipboard, HiDPI and
fractional scale, dmabuf, IME (C6), the desktop's keyboard layout (the shell uses
xkbcommon's default, i.e. `XKB_DEFAULT_*` or us).

## Build and test

    cd web/shell
    CARGO_BUILD_JOBS=4 nice -n 19 ionice -c3 cargo build
    cargo test            # args, protocol events, layout and popup placement, pairing, host command line

## Run

    web [--engine cef] [--proxy HOST:PORT] [--state-dir DIR] [--host-bin PATH] [URL] [-- HOST-ARGS...]

- `--state-dir`: default `$WEB_STATE_DIR`, else `$XDG_STATE_HOME/web`; the CEF profile is
  `DIR/engines/cef/` (STASH.md "State directory").
- `--host-bin`: default `$WEB_HOST_CEF`, else `web-host-cef` next to `web`, else on PATH.
  For development: `--host-bin web/host-cef/target/debug/web-host-cef`.
- `--proxy` is optional and passed to the host; without it the engine connects directly.
- After `--`: passed to the host unchanged (Chromium switches).
- The control socket is `$XDG_RUNTIME_DIR/web-<pid>/control.sock`, removed at exit.
- Log lines go to stderr as `[web +<ms>ms] ...` (every host event, cursor changes,
  popups, fullscreen, the page window's size).

For development run it in a memory-capped scope (it starts CEF) and in the nested
compositor, never on the user's display:

    web/tools/nest.sh                                   # prints e.g. wayland-1
    WAYLAND_DISPLAY=wayland-1 systemd-run --user --scope -p MemoryMax=6G -p MemorySwapMax=0 \
        web/shell/target/debug/web --state-dir .qwork/state \
        --host-bin web/host-cef/target/debug/web-host-cef https://example.com/
    # as an X11 window: env -u WAYLAND_DISPLAY DISPLAY=$(web/tools/nest.sh --x11-display) ...
    web/tools/nest.sh --stop

## The C2 check

`test/live.py` browses `test/pages/` in the shell inside the nested dwl with injected
input (`web/tools/inject`): the page fills the window minus the bar, pairing, a click,
typing into a field, cursor shapes (pointer, crosshair, an image cursor, default over
the bar), a `<select>` popup chosen with the keyboard, wheel scrolling, a context menu
near the right edge (slid inside the window, closed by Esc and by a click outside), page
fullscreen and back, a JS alert, a permission bubble, a form submitted with Enter, a
link, `--proxy` passed through, and quitting on SIGTERM. Screenshots of the test page
land in `--out` (never commit them).

    (cd web/tools/inject && CARGO_TARGET_DIR=$PWD/../../../.qwork/inject cargo build --release)
    web/shell/test/live.py --out .qwork/c2-wayland --inject .qwork/inject/release/inject
    web/shell/test/live.py --out .qwork/c2-x11 --x11 --inject .qwork/inject/release/inject
    web/shell/test/live.py --out .qwork/c2-proxy --proxy --inject .qwork/inject/release/inject
    web/tools/nest.sh --stop

`--x11` runs the shell on the nested dwl's Xwayland (`web/tools/nest.sh --x11-display`),
so winit opens an X11 window while the engine host still speaks Wayland to the shell.
