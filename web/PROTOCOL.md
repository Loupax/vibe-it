# web — engine-host control protocol, v0

How the shell (web-shell, C2/C3) drives an engine host (web-host-cef, C1; later
web-host-webkit, E1). Engine-neutral: nothing here names a CEF or WebKit type.
Implemented in `host-cef/src/protocol.rs` (encoding, unit tested) and
`host-cef/src/page.rs` (meaning); checked end to end by `host-cef/test/drive.py`.

## One page per host

A host shows **one page** in **one window**, the page only (no toolbar, tab strip or
menus). Tabs are deferred (user, 2026-11-23; maybe later as a vim-style buffers
interface, undecided), so v0 has no `new-tab` / `close-tab`.

- `window.open(...)` and `target=_blank` links go through the host (CEF:
  `OnBeforePopup`), which cancels the popup and loads its URL **in the same page**.
  `window.open` returns `null` to the page. **Known loss:** pages that rely on the
  popup's `window.opener` / a handle to the popup (OAuth and payment login popups,
  "open in a small window" flows) do not work. Popups without a URL
  (`window.open()` then `document.write`) show nothing.
- **No user gesture opens a second tab or window** (user, 2026-11-23, C-NOTAB). Ctrl+click,
  middle-click, Shift+click, Ctrl+Shift+click on a link and Ctrl+Enter in a form do
  nothing at all: nothing opens and the page is not navigated. The context menu has
  no item that opens a tab, window, profile, view-source or DevTools. Script
  `window.open` and plain `target=_blank` still load in place (above).
- Events still carry the page's id (`tab`), so a later multi-page design needs no
  protocol change. Today it is always the same number. `tab-created` and
  `tab-closed` stay: `tab-created` is how the shell pairs the host with the window
  that appears (it is sent before that window is mapped), `tab-closed` says the page
  is gone and the host is exiting.

## Transport

- A Unix stream socket. **The shell listens**, and passes its path to the host:
  `web-host-cef --control <path> ...`. The host connects once at start; if it cannot,
  it exits with status 2.
- One JSON object per line (UTF-8, `\n`-terminated), both directions. Commands are
  tagged by `"cmd"`, events by `"event"`. Unknown fields are ignored (room to grow
  without a version bump); unknown commands are an `error` event.
- The host sends `hello` first. The shell must read events promptly (the host writes
  from its UI thread).
- When the shell closes the socket, the host quits (as on `quit`). Without
  `--control` the host runs standalone (development: `host-cef/run.sh`) and exits
  when its window is closed.

## Host command line

`web-host-cef --profile <dir> [--control <path>] [--proxy <host:port>] [--url <url>]
[--downloads <dir>] [Chromium switches...]`

- `--profile`: the engine profile (cookies, cache, localStorage; STASH.md "State
  directory"). Required, so the engine never falls back to a default location.
- `--proxy <host:port>` (or `http://host:port`): an HTTP forward proxy (plain requests
  and `CONNECT`) for every request; given to Chromium as `--proxy-server`. Optional:
  **without it the engine connects directly**; a missing `--proxy` is never an error.
  The host enforces nothing (DECISIONS.md "No enforcement in web"). Chromium still
  goes direct for loopback addresses unless told otherwise
  (`--proxy-bypass-list=<-loopback>`, passed through).
- `--url`: the first page (default `about:blank`).
- `--downloads`: where downloads are saved without asking (default
  `<profile>/downloads`). A download UI is a later item.
- Anything else starting with `--` is passed to Chromium unchanged.

## Commands (shell -> host)

| text form | JSON | what it does |
|---|---|---|
| `open <url>` | `{"cmd":"open","url":"example.com"}` | load a URL; no scheme means `https://` |
| `back` | `{"cmd":"back"}` | history back |
| `forward` | `{"cmd":"forward"}` | history forward |
| `reload`, `reload hard` | `{"cmd":"reload"}`, `{"cmd":"reload","hard":true}` | reload; `hard` bypasses the cache (`:refresh hard`; CEF ReloadIgnoreCache) |
| `stop` | `{"cmd":"stop"}` | stop the current load |
| `find <query> next\|prev` | `{"cmd":"find","query":"needle","dir":"next"}` | find in page; the same query again steps to the next/previous match; a new query starts at the first (`next`) or last (`prev`) match |
| `find-clear` | `{"cmd":"find-clear"}` | end the find: highlights go, the active match stays selected |
| `zoom <n>` | `{"cmd":"zoom","factor":1.25}` | page zoom as a factor, 1.0 = 100 %, 0.25..5 |
| `quit` | `{"cmd":"quit"}` | close the page (no `beforeunload` prompt) and exit |

Commands that need no answer get none; a command the host cannot use (not JSON,
unknown `cmd`, missing field, zoom out of range) gets an `error` event.

## Events (host -> shell)

| text form | JSON |
|---|---|
| `hello` | `{"event":"hello","protocol":0,"engine":"cef"}` |
| `tab-created <id>` | `{"event":"tab-created","tab":1}` |
| `tab-closed <id>` | `{"event":"tab-closed","tab":1}` |
| `url <id> <url>` | `{"event":"url","tab":1,"url":"https://example.com/"}` |
| `title <id> <text>` | `{"event":"title","tab":1,"title":"Example Domain"}` |
| `load <id> start\|end <status> <kind>` | `{"event":"load","tab":1,"phase":"end","status":200,"kind":"new"}`, optional `"error":"ERR_ABORTED"` |
| `load-error <id> <code> <text>` | `{"event":"load-error","tab":1,"code":-105,"name":"ERR_NAME_NOT_RESOLVED","text":"ERR_NAME_NOT_RESOLVED"}` |
| `editable-focused <id> true\|false` | `{"event":"editable-focused","tab":1,"focused":true}` |
| `find-result <id> <i> <n>` | `{"event":"find-result","tab":1,"index":2,"count":5}` |
| `crashed <id>` | `{"event":"crashed","tab":1,"reason":"TS_PROCESS_CRASHED (exit code 139)"}` |
| (error) | `{"event":"error","message":"..."}` |

- `tab-created`: once, **before the page's window is mapped**. The window's Wayland
  app_id (X11 WM_CLASS) is `web-host-cef`.
- `url`: the main frame's URL, sent when it changes (also for same-document
  navigations). For every load, the new `url` comes **before** its `load end`.
- `title`: sent when the title changes (not repeated for the same text).
- `load`: main-frame loads only; every `start` is followed by one `end` of the same
  kind.
  - `kind`: `new` (open, link, form, script navigation, popup loaded in place),
    `reload`, `traverse` (back/forward, also restores from the back-forward cache),
    `same-document` (fragment change, `history.pushState`/`replaceState`, back/forward
    between such entries).
  - `start` is sent when the new document commits (the first response is in), not
    when the request leaves; `status` is 0 on `start`.
  - `end`: `status` is the HTTP status (back-forward-cache restores report the
    status the page was loaded with); 0 for `same-document` and when no response
    arrived. `error` names the engine's error when the load failed (`ERR_NAME_NOT_RESOLVED`,
    ...; an error page is then shown) or was stopped by `stop` before it committed
    (`ERR_ABORTED`; then `start` and `end` come together, with the kind of the
    stopped navigation). A load replaced by another navigation before it committed
    gets no events of its own.
- `load-error`: a main-frame load failed at the network level (DNS, connection, TLS,
  proxy, an HTTP error the engine turns into a failure) or was stopped. Sent **in
  addition to** the load's events, always **before** its `load end`, whose `error` is
  the same `name`; so a shell can show the failure from `load-error` and keep its
  start/end bookkeeping on `load`. `code` is the engine's error number (Chromium's
  net error: -105 ERR_NAME_NOT_RESOLVED, -3 ERR_ABORTED, ...), `name` its symbolic
  name, `text` the engine's description (CEF gives the name again). Not sent for the
  engine's own silent retries, nor for a load aborted because another navigation
  replaced it (that one has its own events). Through a proxy, an unresolvable host is
  the proxy's answer: e.g. a 502, failed as -379 ERR_HTTP_RESPONSE_CODE_FAILURE.
- `editable-focused`: on change, when keyboard focus moves into or out of an
  editable element (text inputs, textareas, contenteditable) in any frame of the
  page. A new document starts with `false`. The shell uses it for INPUT mode
  (KEYS.md).
- `find-result`: one per `find` command, when the engine's count is final. `index`
  is the 1-based active match (0 when there are no matches), `count` the matches.
- `crashed`: the page's renderer process died (crash, killed, out of memory). The
  page stays; `open` or `reload` loads it again in a new renderer.

## Not in v0 (engine default for now)

Downloads are saved without asking (see `--downloads`); permission prompts and JS
dialogs (`alert`, `confirm`, `prompt`, `beforeunload`) are the engine's own UI. Each
gets its own item later. `forget-site <site>` is added by P-FORGET (STASH.md).

## Versioning

`hello` carries `protocol`. Adding fields or events is compatible (both sides ignore
what they do not know); changing or removing one bumps the version.
