# web-host-cef

The CEF (Chromium) engine host (web/TODO.txt C1). It shows one page in one window,
the page only, and is driven by the shell over a control socket: protocol v0,
`../PROTOCOL.md`. It grew out of the spike (`../spike/README.md` keeps the record of
W1-W7 and what CEF taught us; the code moved here).

Its own workspace root, not in CI (CEF is a ~380 MB download).

## Build

Same as the spike (`../spike/README.md` "Versions", "Fetch CEF and build"; same CEF
cache, so nothing is downloaded again):

    cd web/host-cef
    export CEF_PATH=$HOME/.cache/web-spike/cef
    CARGO_BUILD_JOBS=4 nice -n 19 ionice -c3 cargo build
    cargo test                      # protocol encoding, options, zoom, load kinds
    python3 -m unittest discover -s test   # the check's helpers (no CEF)

`target/` is ~2 GB; delete it when done.

## Run

    web/host-cef/run.sh [--x11] [--url URL] [--proxy HOST:PORT] [--control SOCKET] [Chromium switches]

run.sh puts the whole process tree in a memory-capped scope (`systemd-run --user
--scope -p MemoryMax=6G`, unit `$WEB_HOST_UNIT`, stop it with `systemctl --user stop
<unit>.scope`), keeps Chromium from moving itself out of that scope
(`FLATPAK_SANDBOX_DIR`, spike W1), turns core files off, and uses the throwaway profile
`$WEB_HOST_PROFILE` (default `web/host-cef/.qwork/profile`, git-ignored). There is no
network namespace any more: web enforces nothing (`../DECISIONS.md`). Host log lines go
to stderr as `[web-host-cef +<ms>ms] ...`, including every event and command;
`WEB_HOST_TRACE=1` adds the raw CEF callbacks.

In the nested compositor (never the user's display):

    scratchy/tools/nest.sh                      # prints e.g. wayland-1
    WAYLAND_DISPLAY=wayland-1 web/host-cef/run.sh --url https://example.com/
    scratchy/tools/nest.sh --stop

## The C1 check

`test/drive.py` plays the shell: it listens on the control socket, starts the host
through run.sh, serves `test/pages/` from 127.0.0.1, sends every command and waits for
the events each must give (open, back/forward, reload, same-document navigations,
editable focus, find, zoom, window.open and target=_blank loading in the same page,
reload hard, stop, an unresolvable host (load-error), an external page, bad commands, a renderer crash, quit),
then checks that every event type was seen and that `tab-created` went out before the
window was shown. With `--proxy` it routes everything, the local pages included,
through an HTTP forward proxy; `test/fwd_proxy.py` is a small one (plain + CONNECT,
one log line per request or tunnel).

    WAYLAND_DISPLAY=wayland-1 web/host-cef/test/drive.py --out .qwork/c1-direct
    test/fwd_proxy.py --log .qwork/proxy.log --pid-file .qwork/proxy.pid &   # prints its port
    WAYLAND_DISPLAY=wayland-1 web/host-cef/test/drive.py --out .qwork/c1-proxy \
        --proxy 127.0.0.1:PORT --proxy-log .qwork/proxy.log
    kill "$(cat .qwork/proxy.pid)"

`--out` gets events.jsonl (the transcript), host.log and fixture.log. The pages call
`window.open` and click a `target=_blank` link from script, so the check passes
`--disable-popup-blocking` (there is no input injection yet).

## The forget-site check

`test/forget.py` checks `forget-site` and `--no-window --forget-site` (PROTOCOL.md
"forget-site") against local sites it serves itself: `site-a.test`, `sub.site-a.test`
(forgotten) and `site-b.test` (kept), mapped to its server with
`--host-resolver-rules` and made secure contexts with
`--unsafely-treat-insecure-origin-as-secure`. A fake `/login` on each host sets
cookies (HttpOnly, script, and a `Domain=` one), localStorage, sessionStorage,
IndexedDB, Cache Storage and an "app shell" service worker that answers navigations;
`/check` reports what a fresh load sees and the server records the cookies it gets.
Four host processes on one profile: forget site-a by a subdomain URL while its page is
shown; restart and check again; `--no-window --forget-site site-b.test`; restart and
check site-b. Headless by default (no display needed), `--wayland` for the nested
compositor:

    web/host-cef/test/forget.py --out .qwork/forget
    WAYLAND_DISPLAY=wayland-1 web/host-cef/test/forget.py --out .qwork/forget-wl --wayland

## Notes

- **Chrome runtime style, no Chrome UI.** The page is a CEF Views `BrowserView` in our
  own window with `CEF_CTT_NONE` (no toolbar). Chrome-style keyboard shortcuts (Ctrl+T,
  Ctrl+N, Ctrl+F, Ctrl+L, ...) would open Chrome UI; a `CommandHandler` blocks every
  Chrome command, since the shell owns keys.
- **No new tab or window by any user gesture (C-NOTAB, `src/newtab.rs`).** `OnBeforePopup`
  and `OnOpenURLFromTab` decide by disposition (`popup_action`, unit-tested): foreground
  tab (`target=_blank`, `window.open`) and popup load in place; background tab (Ctrl+click,
  middle-click, Ctrl+Enter), new window (Shift+click), off-the-record, save-to-disk,
  split view, PiP and the rest are dropped (nothing opens, the page is not navigated).
  Ctrl+Shift+click has the same disposition as `target=_blank`, so the renderer
  cancels that click on links (`CTRL_SHIFT_JS`, `preventDefault` only). The context menu
  (`OnBeforeContextMenu`) loses `newtab::MENU_REMOVE`: open link in new tab / window /
  incognito / profile / split view, open image/video/audio in new tab, search the web
  (for text, image, video frame, Lens), view page source, Inspect and DevTools, in the
  menu and its submenus; the host log prints `context menu: removed [...]; remaining [...]`
  per menu. Chrome keyboard commands (Ctrl+T, Ctrl+N, Ctrl+Shift+N, ...) stay blocked by
  the `CommandHandler`. Other paths: dropping a dragged link on the page navigates the
  page itself (no new tab); the PDF viewer's buttons and the DevTools shortcuts
  (F12, Ctrl+Shift+I) go through Chrome commands, which the `CommandHandler` blocks
  (not checked live: owed); no `KeyboardHandler` (on Wayland CEF's
  `os_event` is not an `XEvent`, the crate's wrapper crashed the host).
  Live check: `test/notab.py` (real input through `web/tools/inject`; written, not yet run
  green: the nested compositor had no display, live check owed).
- **Popups load in place.** `OnBeforePopup` cancels every popup and loads its URL in
  the page (PROTOCOL.md "One page per host").
- **Find.** A new CEF find session counts matches but activates none; the host steps
  once more so the first (or last) match is active, and reports only the final count.
- **Same-document loads** get no CEF load callbacks; page.rs derives them from address
  changes outside a cross-document load (see its module comment).
- **Chromium's own traffic.** With `--proxy`, the proxy also sees what Chromium sends by
  itself at start: component updater (`update.googleapis.com`, `*.gvt1.com`,
  `edgedl.me.gvt1.com`), `clients2.google.com/time`, `www.google.com`,
  `content-autofill.googleapis.com` (the same list the spike's W3 net log found). Not
  switched off in C1 (proposed item).
