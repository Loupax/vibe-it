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
- **HiDPI and fractional scale** (C6b): the inner output takes the shell window's scale factor
  (winit's, from the desktop; its `Resized` events carry changes, e.g. the window moved to
  another monitor). The output's mode is the window's framebuffer in pixels and its scale the
  window's (`Scale::Fractional`: wl_output.scale gets it rounded up, xdg_output the logical
  size); every page surface gets fractional-scale-v1's preferred scale and wl_surface v6's
  preferred_buffer_scale (`State::send_scale`, on creation, on commit and after a change).
  Everything the shell lays out is logical (`layout.rs`: the page, the 24 px bar, pointer
  positions, popup constraints); the Space draws the page at the scale, and the bar is
  painted at the framebuffer's resolution (font, paddings and caret times the scale) and drawn
  1:1 (`Layout::bar_physical`). Each change is logged as `window WxH, page ..., scale S (PxQ
  px)` and `scale A -> B`. CEF 156 (Ozone/Wayland) binds wp_fractional_scale_manager_v1 and
  uses it: at 1.5 it draws 1.5x buffers (1434x1467 for a 956x978 page) with a wp_viewport
  destination of the logical size and never calls wl_surface.set_buffer_scale; at 1 it
  unsets the viewport. Without fractional-scale-v1 it takes the integer scale (wl_output /
  preferred_buffer_scale: 2 at 1.5) and still draws through the viewport, which the shell then
  shrinks. devicePixelRatio is the scale. An X11 shell window sees whatever winit's X11
  backend reports (1 under the nest's Xwayland, which then scales the window up). Tests
  `src/tests/c6b_tests.rs`, live check `test/c6b_live.py`.

Not in C2/C3/C5/C6 (later items): frame pacing, dmabuf, IME.

The desktop's keyboard layout (C-KEYMAP, `src/keymap.rs`): the shell's seat (and so the page and the
shell's own keysym bindings) uses the desktop's keymap, read on a worker thread: on Wayland a second
`wl_keyboard` on the window's own connection (keymap on start and on every change, plus the active
layout group of its `modifiers`), on X11 the XKB extension and libxkbcommon-x11 (`NewKeyboardNotify`,
`MapNotify`, group `StateNotify`). Without a desktop keymap it falls back to xkbcommon's default
(`XKB_DEFAULT_*` or us). Live check `test/c_keymap_live.py` (de, fr, layout groups, X11 with setxkbmap).

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
- `--forget-site SITE` (P-FORGET-SHELL, `src/forget.rs`): no window. If a web runs on the
  same state dir (pid in `DIR/web.pid`, a live `web`), the request goes to its
  `$XDG_RUNTIME_DIR/web-<pid>/forget.sock` and the host's `forgot` line comes back; else the
  host runs `--no-window --forget-site`. Exit 0 ok, 1 partial, 2 refused. In the shell,
  `:forget-site [site]` does it for the page's site, shows `forgot ...` and reloads the page.
  Check: `test/forget_live.py` (running instance, `:forget-site`, no instance, SIGKILL).
- At start the shell removes `web-<pid>` run directories whose pid is not a running web
  (C-SOCKDIR; a hard-killed shell leaves its directory), by explicit file names.
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
the bar), a `<select>` popup chosen with the keyboard, wheel scrolling, a `<select>` flush with the right edge (its dropdown
slid inside the window, closed by Esc and by a click outside), page
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

## The C6b check

`test/c6b_live.py` runs at the nest's scale (`NEST_SCALE`, passed on to `web/tools/nest.sh`,
which replaces a nest of another scale; the output then has NEST_SIZE times the scale pixels):
the shell takes the output's scale; the page's devicePixelRatio is the scale and its CSS
viewport the page window; a click lands on the CSS pixel aimed at; a 1 CSS px line is pure
black device pixels with no grey (drawn at the scale, not stretched); the bar is 24 logical px
with text; then the output's scale is changed while the shell runs (`inject output W H S`,
wlr-output-management) and the page follows (devicePixelRatio, CSS size, clicks, crispness),
and the scale is put back. live.py and c3_live.py run at any `NEST_SCALE` too (coordinates
logical; screenshot checks multiply by the nest's scale).

    NEST_SCALE=1.5 web/shell/test/c6b_live.py --out .qwork/c6b-1.5 --inject .qwork/inject/release/inject
    NEST_SCALE=2 web/shell/test/c6b_live.py --out .qwork/c6b-2 --inject .qwork/inject/release/inject
    NEST_SCALE=2 web/shell/test/live.py --out .qwork/c2-s2 --inject .qwork/inject/release/inject

## C6c: frame pacing

The page's frame callbacks (`wl_surface.frame` done) are sent only right after a shell render,
and a render happens at most once per outer frame (`src/pacing.rs`): on a Wayland desktop
winit holds `RedrawRequested` back until the desktop's frame callback for the window
(smithay's `submit` calls `pre_present_notify`; the EGL swap interval is 0, so swapping never
blocks), so the page runs at the desktop's rate (a 30 Hz output gives a 30 Hz
requestAnimationFrame); on X11 nothing holds a redraw back, so a timer spaces renders by one
interval of the monitor's refresh rate. The inner output reports the outer monitor's refresh
rate (winit; 60 Hz if unknown). An idle page costs nothing: no dirty flag, no timer.
`WEB_FRAME_STATS=1` logs `frames: N renders in 5 s`. `test/c6c_bench.py` measures cadence and
CPU (run it under `flock /tmp/scratchy-perf.lock`); `NEST_REFRESH=Hz` for web/tools/nest.sh sets
the headless output's refresh (default 60).

## C6d: dmabuf

The shell offers `zwp_linux_dmabuf_v1` (v5, one feedback tranche: every format/modifier its
GlesRenderer imports, main device = the renderer's DRM render node) and imports each client
dmabuf as a texture (`src/dmabuf.rs`, `DmabufHandler` in state.rs). Chromium's Ozone/Wayland
then composites on the GPU and hands the shell GPU buffers; without the global it falls back
to software compositing in its GPU process and wl_shm, and every frame is copied by the CPU
(and uploaded again by the shell). Nothing to configure in the engine: CEF switches by itself.
wl_shm stays offered. The global is NOT offered (log line `dmabuf: not offered, wl_shm only:
<why>`) when the shell's renderer is software (llvmpipe), imports no dmabufs, has no DRM
device, or `WEB_NO_DMABUF=1` is set; then everything is as before. A failed import is logged
and takes the global away for clients that bind later. The start log says `dmabuf: offered (N
formats on /dev/dri/renderD128)`, and the first imported page buffer is logged once.

Headless: web/tools/nest.sh's dwl renders with pixman and offers no dmabuf (the user's wlroots
is built without GLES/Vulkan), so the shell inside it is llvmpipe and stays on wl_shm.
`web/tools/gpu-nest.sh` runs KWin's virtual backend instead (no display, own D-Bus session,
memory-capped scope; it renders on the GPU and offers linux-dmabuf, like a Plasma desktop):
`c6c_bench.py --wayland $(web/tools/gpu-nest.sh)` exercises the dmabuf path headless,
`web/tools/gpu-nest.sh --shot f.png` screenshots it, `--stop` ends it. It has no input injection.
`c6c_bench.py` now also has a `scroll` mode (a long page scrolled 40 px per frame), reports the
engine's GPU-process / renderer / total CPU, and with `--wayland-debug` counts the page's
wl_shm pools and dmabufs. On the real desktop: `web/shell/test/c6d_desktop_check.sh` (PASS/FAIL
plus CPU with and without dmabuf).

Measured 2026-11-23 on the GPU nest (960x1006, release, % of one core, 2 runs averaged):

| mode   | shell before | shell dmabuf | engine GPU process before | dmabuf | engine total before | dmabuf |
|--------|-------------:|-------------:|--------------------------:|-------:|--------------------:|-------:|
| scroll |          7.5 |          3.0 |                      27.5 |    3.5 |                36.5 |    8.0 |
| video  |          2.7 |          1.6 |                       3.0 |    2.5 |                 8.8 |    6.6 |
| css    |          4.2 |          3.2 |                       2.4 |    3.8 |                 8.7 |    8.9 |
