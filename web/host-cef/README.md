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

    web/tools/nest.sh                           # headless dwl, prints e.g. wayland-1
    WAYLAND_DISPLAY=wayland-1 web/host-cef/run.sh --url https://example.com/
    WAYLAND_DISPLAY=wayland-1 grim shot.png     # see it
    web/tools/nest.sh --stop

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

With `--proxy-log` the run also fails when the proxy log shows any host but the test
pages, the external page and the test's unresolvable host (C-BGNET, "Chromium's own
traffic is off" below); `--idle SECONDS` adds a wait after the first page loads in which
the proxy must log nothing; `--allow-host`, `--host-arg` (extra host arguments, e.g.
`--allow-background-traffic` for a control run) and `--headless` (no display; the find
step needs the nested compositor, headless alone fails it) are the other options.
fwd_proxy.py logs `CONNECT host:port opened` when a tunnel starts (and the closing line
when it ends), so a late request shows up when it is made.

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
IndexedDB, Cache Storage, an "app shell" service worker that answers navigations, and
an HTTP cache entry (`/cached.js`, `max-age`); `/check` reports what a fresh load sees,
and the server records the cookies it gets and whether `/cached.js` was fetched again
(it was not if it came from the HTTP cache).
Four host processes on one profile: forget site-a by a subdomain URL while its page is
shown; restart and check again; `--no-window --forget-site site-b.test`; restart and
check site-b. Headless by default (no display needed), `--wayland` for the nested
compositor:

    web/host-cef/test/forget.py --out .qwork/forget
    WAYLAND_DISPLAY=wayland-1 web/host-cef/test/forget.py --out .qwork/forget-wl --wayland

## The third-party cookie check

`test/cookies.py` checks that third-party cookies are blocked by default
(PROTOCOL.md "Third-party cookies off"). It serves `site-a.test`, `site-b.test` and
`site-c.test` over https from one local server (self-signed certificate,
`--host-resolver-rules`, `--ignore-certificate-errors`), and runs two fresh hosts: the
default, and `--allow-third-party-cookies` as the control. site-a embeds a site-b
iframe and image that try to set and read cookies; site-b at top level must keep its
cookies and gain none; a `Partitioned` cookie and localStorage in the frame must work
and stay in the top-level site's partition (site-c does not see site-a's). Headless
by default:

    web/host-cef/test/cookies.py --out .qwork/cookies

The verdict logic is unit-tested in `test/test_cookies.py`.

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
  Live (NEST-HEADLESS): real Ctrl/middle/Shift clicks arrive through `OnOpenURLFromTab`
  (`open-url-from-tab Drop:` in the log), and Chromium does not submit a form on
  Ctrl+Enter at all (no keypress), so nothing reaches the host there.
  Ctrl+Shift+click has the same disposition as `target=_blank`, so the renderer
  cancels that click on links (`CTRL_SHIFT_JS`, `preventDefault` only). There is no context menu at all (C-NOMENU, user 2026-11-23: its useful
  actions move to `:` commands, C-CMDS): `OnBeforeContextMenu` empties the model and
  `RunContextMenu` returns handled after `callback.cancel()`. Both are needed: with only the
  model cleared, Chrome style still opens a popup (checked live, notab.py fails). The
  right-click still reaches the page as an ordinary `mousedown`/`contextmenu` event, so
  pages with their own right-click menus work; the host log prints
  `context menu: suppressed (N items cleared)`. Chrome keyboard commands (Ctrl+T, Ctrl+N, Ctrl+Shift+N, ...) stay blocked by
  the `CommandHandler`. Other paths: dropping a dragged link on the page navigates the
  page itself (no new tab); the PDF viewer's buttons and the DevTools shortcuts
  (F12, Ctrl+Shift+I) go through Chrome commands, which the `CommandHandler` blocks
  (not checked live: owed); no `KeyboardHandler` (on Wayland CEF's
  `os_event` is not an `XEvent`, the crate's wrapper crashed the host).
  Live check: `test/notab.py` (real input through `web/tools/inject`; written, not yet run
  green: the nested compositor had no display, live check owed).
- **Resizable window.** The window delegate says `can_resize`/`can_maximize` (C2): the
  cef crate's defaults (0) pin the window at its preferred 1200x800 (xdg_toplevel min =
  max size), and the shell's configure would be ignored.
- **Popups load in place.** `OnBeforePopup` cancels every popup and loads its URL in
  the page (PROTOCOL.md "One page per host").
- **Find.** A new CEF find session counts matches but activates none; the host steps
  once more (from `on_find_result`, once the count is final and above 0) so a match is
  active, and reports only that result; a count of 0 is reported as it is. Every new
  session first ends the old one (`stop_finding`): after a navigation CEF keeps the old
  session and never answered the same text again (C5, found live). `match_case` goes to
  CEF's `matchCase`.
- **Same-document loads** get no CEF load callbacks; page.rs derives them from address
  changes outside a cross-document load (see its module comment).
- **Chromium's own traffic is off (C-BGNET).** Unswitched, Chromium talks to Google by
  itself, and a proxy sees it: component updater (`update.googleapis.com`,
  `edgedl.me.gvt1.com`), `clients2.google.com/time`, `www.google.com`,
  `content-autofill.googleapis.com`, the spellcheck dictionary (`redirector.gvt1.com`,
  `*.gvt1.com`), and after about two minutes `safebrowsing.googleapis.com`. The host
  adds these to Chromium's command line (`protocol::quiet_switches`, applied in
  `main.rs`; merged with a `--disable-features` you pass, and your own
  `--component-updater` wins). The smallest set that works, found by removing one at a
  time from a larger candidate list (each is needed: without it, the host named next
  shows up in the proxy log):
  - `--component-updater=url-source=http://127.0.0.1:9/`: the update check Chromium
    makes at start (`update.googleapis.com`, then `edgedl.me.gvt1.com` downloads of
    whatever the server offers). Its address is a closed loopback port, so every check
    fails at once, and loopback never goes through the proxy. `--disable-component-update`
    and `--disable-background-networking` do NOT stop it (tried, alone and together:
    the first check is an on-demand install, not the scheduled update).
  - `--disable-background-networking`: the Safe Browsing list update
    (`safebrowsing.googleapis.com`, first request after about two minutes; also what
    keeps the other background fetchers quiet later).
  - `--disable-sync`: the spellcheck dictionary download (`redirector.gvt1.com`,
    `en-us-10-2.bdic`). Found empirically; the switch name says sync, but removing it
    brings the download back and `--disable-spell-checking` does not stop it.
  - `--disable-features=NetworkTimeServiceQuerying`: `clients2.google.com/time/1/current`
    (Chromium's network time sync, used to judge certificate dates).
  - `--disable-features=AimEnabled`: `www.google.com/async/folae` (Google's AI Mode
    eligibility check at start). `AimServerRequestOnStartupEnabled` alone also works;
    `AimServerEligibilityEnabled` alone does not.
  - `--disable-features=PreconnectToSearch`: the TCP+TLS preconnect to `www.google.com`
    (the default search engine) at start.
  - `--disable-features=AutofillServerCommunication`: `content-autofill.googleapis.com`,
    queried when a page with a form loads.
  Feature names in CEF 156 are the C++ constant without its `k`
  (`kNetworkTimeServiceQuerying` -> `NetworkTimeServiceQuerying`); the `k` form is
  silently ignored. Tried without effect (not needed in the set): `--disable-domain-reliability`,
  `--no-pings`, `--disable-breakpad`, `--disable-component-update`, and the features
  `OptimizationHints`, `MediaRouter`, `Translate`, `CertificateTransparencyComponentUpdater`.
  `--allow-background-traffic` turns the whole set off (measuring, comparing).
  What is lost: **spellcheck** (no dictionary to download), Safe Browsing's list checks,
  Google's time sync, form-field prediction from Google's servers, AI Mode. **DRM
  (Widevine) loses nothing**: CEF's distribution ships no CDM, and with every switch off
  `requestMediaKeySystemAccess("com.widevine.alpha")` answers no, even 130 s after start
  (checked); only the component updater could ever have delivered one, so DRM video
  (Netflix, Spotify's web player) would need a CDM path and the updater switched back on.
  Clear Key (`org.w3.clearkey`) is available, VP9/WebM plays (`canPlayType` "probably").
  Unrelated to this item: the same probe says `video/mp4; codecs=avc1` (H.264) is NOT
  playable in this CEF build (no proprietary codecs), with or without the switches.
  Check: `test/drive.py ... --proxy ... --proxy-log ... --idle 150` fails when a host
  other than the test pages, the external page and the unresolvable test host appears in
  the proxy log (`--allow-host` adds one), or when the proxy logs anything in the 150 s
  after the first page loads (`proxy_log_hosts`/`foreign_hosts`, unit-tested). Results,
  before and after, are in the C-BGNET entry of `../PROMPTS.md`.

## Middle-click paste (C6f)

Blink pastes the primary selection on a middle click only when the desktop toolkit says
`PrimaryPasteEnabled`. Chromium loads GTK dynamically and reads `gtk-enable-primary-paste`, which
GNOME's schema defaults to false, so the click reached the page but nothing was pasted (the
`paste` event never fired). The host adds `--ui-toolkit=fallback` (whose answer is always yes)
unless the command line already has `--ui-toolkit`. Side effect: GTK theming is gone (a dark GTK
theme no longer makes pages prefer a dark colour scheme; the permission bubble is smaller).
`shell/test/c6_live.py`'s primary step checks the paste.
