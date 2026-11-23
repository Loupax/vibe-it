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
[--downloads <dir>] [--forget-site <site> [--no-window]] [Chromium switches...]`

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
- `--forget-site <site>`: run `forget-site <site>` at start (below); the `forgot`
  event goes to the control socket as usual.
- `--no-window` (only with `--forget-site`): show no page and need no display
  (Chromium's headless platform): start, forget the site, print the `forgot` event as
  one JSON line on stdout, exit. Exit status 0 `ok`, 1 `partial`, 2 refused (bad
  site or options). This is how `web --forget-site` works when no web instance runs
  (the engine locks its profile, so it cannot run beside a running host).
- `--allow-third-party-cookies`: keep the engine's default cookie policy instead of
  blocking third-party cookies (testing only; "Third-party cookies off" below).
- Anything else starting with `--` is passed to Chromium unchanged.

## Commands (shell -> host)

| text form | JSON | what it does |
|---|---|---|
| `open <url>` | `{"cmd":"open","url":"example.com"}` | load a URL; no scheme means `https://` |
| `back` | `{"cmd":"back"}` | history back |
| `forward` | `{"cmd":"forward"}` | history forward |
| `reload`, `reload hard` | `{"cmd":"reload"}`, `{"cmd":"reload","hard":true}` | reload; `hard` bypasses the cache (`:refresh hard`; CEF ReloadIgnoreCache) |
| `stop` | `{"cmd":"stop"}` | stop the current load |
| `find <query> next\|prev [match_case]` | `{"cmd":"find","query":"needle","dir":"next"}`, optional `"match_case":true` (default false: case-insensitive; the shell's smartcase sets it when the query has a capital) | find in page; the same query again steps to the next/previous match; a new query starts at the match the engine picks from where the page is scrolled (C5 measured: from the top, the first match; scrolled down, the one nearest above the screen; not exactly "first at or below the top of the screen") |
| `find-clear` | `{"cmd":"find-clear"}` | end the find: highlights go, the active match stays selected |
| `zoom <n>` | `{"cmd":"zoom","factor":1.25}` | page zoom as a factor, 1.0 = 100 %, 0.25..5 |
| `forget-site <site>` | `{"cmd":"forget-site","site":"https://mail.example.com/"}` | forget a site: cookies and storage (below); answered by `forgot` |
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
| `forgot <site> ok`, `forgot <site> partial <what failed>` | `{"event":"forgot","site":"https://mail.example.com/","domain":"example.com","result":"ok","cookies":3,"origins":["http://example.com","https://example.com","https://mail.example.com"]}`, with `"result":"partial","failed":["storage https://x.example.com (timeout)"]` |
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

## forget-site

`forget-site <site>` (P-FORGET; STASH.md "Forget site") deletes what the engine keeps
for a site. Needs no page: it also works before the page exists and with `--no-window`.

- `site`: a site, host or http(s) URL. What is forgotten is its **registrable domain**
  (eTLD+1, scratchy's `site_of` with the same short public-suffix list) and **every
  host under it**, over **both http and https** (cookies do not keep the schemes
  apart either): `https://mail.example.co.uk/x` forgets `example.co.uk`,
  `www.example.co.uk`, `mail.example.co.uk`, ... A top-level domain or public suffix
  (`com`, `co.uk`, `github.io`) or a non-http(s) URL is refused with an `error` event.
- **What goes:** every cookie whose domain is the site or under it (HttpOnly and
  script cookies, host and `Domain=` cookies alike); for every origin under the site:
  localStorage, sessionStorage, IndexedDB, Cache Storage, service workers and the
  engine's other per-origin storage; the site's HTTP cache entries (web-host-cef: the
  whole HTTP cache, see below).
- **`forgot`** answers every `forget-site`, after everything is done and the cookie
  store is flushed to disk. `site` is the argument as sent (so the shell can pair
  them), `domain` what was forgotten, `cookies` how many were deleted, `origins` the
  origins whose storage was cleared. `result` is `ok`, or `partial` with `failed`
  listing what could not be cleared (`cookies`, `http-cache`, `storage <origin>
  (<why>)`). Text form: `forgot <site> ok` / `forgot <site> partial <failed,
  comma-separated>`. Several `forget-site`s are run one after the other, each with its
  own `forgot`.
- **Afterwards** the shell reloads its page if it shows the site (STASH.md); the
  host does not.

### What every engine host implements

The protocol side (parsing the site, the queue, the origin record, `forgot`,
`--forget-site`/`--no-window`) is engine-neutral. In web-host-cef it is
`host-cef/src/forget.rs`, and the engine's part is the `SiteForgetter` trait there:
`start(job)` gets the registrable domain and the origins under it that the page
loaded, deletes everything in "What goes" for the domain and every host under it,
flushes to disk and reports `cookies`, `origins` and `failed`. A host for another
engine gives its own implementation:

- **web-host-cef** (`host-cef/src/forget_cef.rs`):
  1. Cookies: `CefCookieManager::VisitAllCookies`, deleting every cookie whose
     domain is under the site. This step is needed: `Storage.clearDataForOrigin`'s
     `cookies` type removes only the origin's host cookies. Checked with
     `test/forget.py`: a `Domain=site-a.test` cookie survived it on every origin of
     the site. The cookie domains also name hosts the page never loaded.
  2. Origins: CEF cannot list the origins that have storage, so the host records the
     origin of every frame its page loads (`<profile>/web-host-origins`; forgotten
     origins are removed from it). It clears those under the site, plus http and
     https of the domain, of `www.` and of every cookie host.
  3. Storage: Chromium's `Storage.clearDataForOrigin` with `storageTypes: "all"` for
     each origin. The call goes **in process** through
     `CefBrowserHost::ExecuteDevToolsMethod`, and the result comes back through a
     `CefDevToolsMessageObserver`. There is no remote-debugging port or DevTools
     socket. It runs on a hidden windowless browser that stays on `about:blank` and
     loads nothing, so DevTools never attaches to the user's page (and it works with
     `--no-window`).
  4. sessionStorage: the frames of the page that show the site also run
     `sessionStorage.clear()`.
  5. HTTP cache: `CefRequestContext::ClearHttpCache` clears **all of it**, because
     no per-site removal is reachable:
     - Checked with `test/forget.py` (a `max-age` response, served from disk unless
       removed): `Storage.clearDataForOrigin` (`all`) and
       `Storage.clearDataForStorageKey` (`all`) both left the forgotten site's
       entry in place.
     - Their removal types cover storage and cookies, not the HTTP cache.
     - DevTools' only cache method, `Network.clearBrowserCache`, is global.
     - CEF's `ClearHttpCache` takes no filter.
     - Chromium does have filtered HTTP cache removal (`NetworkContext::ClearHttpCache`
       with a URL filter). The engine reaches it only through its browsing-data
       remover: Chrome's settings, extensions' `browsingData` API, or a site's own
       `Clear-Site-Data: "cache"` response header. CEF exposes none of these.
     - Chromium's HTTP cache is also partitioned by top-level site, so a per-site
       removal would also have to cover the site's entries cached under other
       top-level sites.
- **web-host-webkit** (E1, not built): `webkit_website_data_manager_fetch` with
  `WEBKIT_WEBSITE_DATA_ALL` returns `WebKitWebsiteData` records grouped by registrable
  domain (cookies, storage, IndexedDB, service workers, disk cache, ...); remove the
  records whose name is the domain with `webkit_website_data_manager_remove`, then
  report. WebKit can list what it stores, so it needs neither the origin record nor
  a separate cookie pass, and its disk cache entries go per domain.

#### Third-party cookies off (P-COOKIES)

Every engine host starts with **third-party cookies blocked**, the closest an engine
gets to scratchy's jar partitioned by top-level site. A cross-site iframe or
subresource can neither set nor read a cookie of its own site; the site as the
top-level page still has its cookies (including `SameSite=None` ones). No protocol
command: it is set at start, before the first page loads. A host flag
`--allow-third-party-cookies` restores the engine's default, for testing only (it is
the control in `host-cef/test/cookies.py`).

Storage partitioning (third-party localStorage, IndexedDB, caches keyed by the
top-level site) and partitioned cookies (CHIPS, `Partitioned`) stay on: they are
per-top-level-site by design, the thing scratchy's jar gave.

- **web-host-cef** (`page.rs` `apply_cookie_policy`): the preference
  `profile.cookie_controls_mode` = 1 (`content_settings::CookieControlsMode`:
  0 off, 1 block third-party, 2 incognito only = Chromium's default) set with
  `CefRequestContext::SetPreference` on the global request context in
  `OnContextInitialized`, before the first browser exists; read back and logged
  (`cookies: profile.cookie_controls_mode=1 ... now=Some(1)`). Checked in CEF 156 with
  `test/cookies.py`: a frame on site-a cannot read site-b's `sid`, and its
  `Set-Cookie`/`document.cookie` writes (`SameSite=None; Secure`) never reach site-b's
  own jar; site-b at top level keeps its cookies; a `Partitioned` cookie works in the
  third-party frame (header and script) and is seen only under the same top-level site;
  localStorage in the frame is partitioned by top-level site. With the flag the same
  frame reads and writes site-b's cookies (so the test can tell). Notes:
  - `profile.block_third_party_cookies` is not registered in CEF 156 (the old name);
    `profile.cookie_controls_mode` is the one that exists and takes effect.
  - `SetPreference` fails silently (returns false, no message, sets nothing) if the
    `error` out-parameter is `CefString::default()` in the Rust bindings; pass
    `CefString::from("")`.
  - Not used: a command-line switch (`--enable-features=ForceThirdPartyCookieBlocking`
    made no difference in a test run while the preference was still failing; not
    retried since).
- **web-host-webkit** (E1, not built): `WebKitCookieManager`
  (`webkit_website_data_manager_get_cookie_manager`) with
  `webkit_cookie_manager_set_accept_policy(WEBKIT_COOKIE_POLICY_ACCEPT_NO_THIRD_PARTY)`,
  and `webkit_website_data_manager_set_itp_enabled(TRUE)` (Intelligent Tracking
  Prevention); WebKit also partitions third-party storage by default. Check with the
  same test (WebKit's support for `Partitioned` cookies: unchecked).

Consequence to know: embedded logins (SSO iframes, some payment and comment widgets)
that need a third-party cookie break. `document.requestStorageAccess()` is
Chromium's own way for a frame to ask; not checked here.

**Limits (web-host-cef):**
- The whole HTTP cache is cleared (above).
- An origin under the site that the host never loaded and that has no cookie is not
  reached: e.g. one visited before the record existed, or only inside another site's
  iframe.
- Storage the site has as a third party inside other sites' pages (partitioned
  storage keys) is not reached.

## Not in v0 (engine default for now)

Downloads are saved without asking (see `--downloads`); permission prompts and JS
dialogs (`alert`, `confirm`, `prompt`, `beforeunload`) are the engine's own UI. Each
gets its own item later.

## Versioning

`hello` carries `protocol`. Adding fields or events is compatible (both sides ignore
what they do not know); changing or removing one bumps the version.
