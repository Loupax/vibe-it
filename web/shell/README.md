# web-shell (`web`)

The browser's shell (web/TODO.txt C2, C3; Architecture C in `../DECISIONS.md`): a small
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
  bottom (KEYS.md "Status bar"), maximized and tiled (`src/layout.rs`). The window
  title is the page title.
- **Input**: a key goes to the focused window (the page, a dialog, or a popup that
  grabbed) when the key model lets it (C3, below); every pointer event goes to the
  surface under it (`src/input.rs`). When the shell window loses the desktop's
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

## What C3 adds (KEYS.md)

- **The key model** (`src/keys.rs`, pure, no Wayland): INPUT / NORMAL / COMMAND, Esc Esc,
  the NORMAL bindings and the `gg`/`yy` sequences, the `:` line with its ghost completion
  (`src/completion.rs`), the commands `:go`, `:refresh [hard]`, `:historyBack`,
  `:historyFront`, `:help`, `:let`, `:q`. The compositor's keyboard filter asks it about
  every key event (modifiers included) and forwards only what it allows; a release goes
  to the page exactly when its press did. Starts in NORMAL.
- **Settings** in `src/config.rs` (dwm `config.h`: edit and rebuild): `BINDINGS`,
  `KEY_SEQUENCES`, `SCROLL_INPUTS` (what each scroll binding sends the page: a key held
  while the binding is held, or wheel steps), `ESC_ESC_MS`, `START_MODE`,
  `CLICK_FOCUS_MS`, the bar's sizes and colours, `COMPLETERS`, history.
- **Scrolling** is real input: `j` presses Down in the page window until `j` is released
  (the engine repeats it), `G` End, `gg` Home, Ctrl-d PageDown, ...
- **INPUT by click** (T29): the host's `editable-focused true`, or a new popup (a select's
  menu), within `CLICK_FOCUS_MS` of a mouse press; later it was the page's own focus()
  (T32) and the mode stays.
- **The status bar** (`src/statusbar.rs`): mode, URL (from `url`), last message (load
  errors from `load-error`, crashes, command answers) or the `:` line with caret and
  ghost; cosmic-text + tiny-skia into a memory buffer drawn in the bar strip. Every
  change is logged as `bar: ...`.
- **History** (`src/history.rs`, STASH.md): `DIR/history`, one line per visit, decided
  from the host's `url`/`title`/`load` events; `:go` completes from it (ranked).
- **`:help`**: `src/help.html`, built in, written next to the control socket at start
  and loaded by the host as a `file://` URL; `:q` there goes back.
- **Search** (C5, `src/keys.rs` `SearchState`, KEYS.md Search): `/` opens the `/` line (the
  `:` line's editor, no ghost), Enter sends `find` (smartcase -> `match_case`), `n`/`N` step
  it, `find-result` gives `[i/N]` and the wrap messages (derived from the indices),
  `Pattern not found: <q>`, `No previous search`; Esc in NORMAL sends `find-clear` (the
  query stays); a cross-document load start forgets the session (T31). Tests
  `src/tests/search_tests.rs`, live check `test/c5_live.py`.
- **The clipboard** (C6, `src/clipboard.rs` the desktop's side, `src/selection.rs` the page's):
  the shell bridges the page's wl_data_device / zwp_primary_selection (on its own seat) and
  the desktop's clipboard and primary selection (copypasta: smithay-clipboard on the shell
  window's Wayland connection, x11-clipboard on X11; a worker thread, every wait bounded to
  1 s). A copy in the page is read (a text type) and written to the desktop; the desktop's
  text is offered to the page (a compositor selection served from memory) when the window
  gains focus, before a paste key reaches the page and before a middle click, only when it
  changed since it was last carried across. `yy`, `:let @+ = @%`, `@+` as a source, and
  paste (Ctrl-v, Ctrl-Shift-v, Shift-Insert) into the `:` and `/` lines use the desktop's
  clipboard directly. Text only. Tests `src/tests/c6_tests.rs`, live check
  `test/c6_live.py` (Wayland and `--x11`).
- **Enter in NORMAL** (C-ENTER, KEYS.md T18-T20): goes to the page; with a find open the shell
  sends `find-clear` first (Chromium then focuses the match's link) and the Enter 50 ms later
  (`Effect::Pause`, `input.rs`). **`:set caret|nocaret|caret?`** (C-CARETCMD): the host's
  `caret` command, Chromium's caret browsing, off at start. Tests `src/tests/c_enter_tests.rs`,
  live check `test/c_enter_live.py`.

Not in C2/C3/C5/C6 (later items): HiDPI and
fractional scale, frame pacing, dmabuf, IME, the desktop's keyboard layout (the shell uses
xkbcommon's default, i.e. `XKB_DEFAULT_*` or us).

## Build and test

    cd web/shell
    CARGO_BUILD_JOBS=4 nice -n 19 ionice -c3 cargo build
    cargo test            # args, protocol, layout and popups, pairing, host command line; C3: src/tests/ (KEYS.md rows)

## Run

    web [--engine cef] [--proxy HOST:PORT] [--state-dir DIR] [--host-bin PATH] [URL] [-- HOST-ARGS...]

- `--state-dir`: default `$WEB_STATE_DIR`, else `$XDG_STATE_HOME/web`; the CEF profile is
  `DIR/engines/cef/` (STASH.md "State directory").
- `--host-bin`: default `$WEB_HOST_CEF`, else `web-host-cef` next to `web`, else on PATH.
  For development: `--host-bin web/host-cef/target/debug/web-host-cef`.
- `--proxy` is optional and passed to the host; without it the engine connects directly.
- After `--`: passed to the host unchanged (Chromium switches).
- The control socket is `$XDG_RUNTIME_DIR/web-<pid>/control.sock`, removed at exit.
- Log lines go to stderr as `[web +<ms>ms] ...` (every host event and command, cursor
  changes, popups, fullscreen, the page window's size, the status bar).

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

## The C3 check

`test/c3_live.py` drives the keys for real in the headless nest (19 steps, each naming
its KEYS.md rows): starts in NORMAL with the bar drawn; `j`/`k`/`G`/`gg`/Ctrl-d/Home
scroll by real keys while the page never sees the bound keys; `/` opens the search line; `i`,
typing, Esc Esc (the page sees exactly the first Esc), a slow Esc Esc; a click into a
field enters INPUT, the page's own focus() does not; `:go`, `h`/`l`, the `:go` ghost from
history with Tab and Enter, `:refresh [hard]`, `:help` and `:q` on it, `:let`/`yy` (C6),
Esc cancelling the line, a load error in the bar, the history file, `:q` quitting.
Shifted characters are typed with a held Shift key (`hold:s`): inject's `type:` sends
modifier state alone, which the shell (smithay's winit backend) does not see.

    web/shell/test/c3_live.py --out .qwork/c3 --inject .qwork/inject/release/inject
