# web — engine-host control protocol, v1

How the shell (web-shell, C2/C3) drives an engine host (web-host-cef, C1; later
web-host-webkit, E1). Engine-neutral: nothing here names a CEF or WebKit type.
Implemented in `host-cef/src/protocol.rs` (encoding, unit tested) and
`host-cef/src/page.rs` (meaning); checked end to end by `host-cef/test/drive.py` (popups with real
clicks: `shell/test/c_popups_live.py`).

## Several pages (protocol 1, C4)

A host holds **any number of pages** (the shell's buffers, KEYS.md "Buffers"); v0 had one. Protocol 1
is v0 plus `new-tab`, `show-tab`, `close-tab`, an optional `tab` on page commands and `req` on
`tab-created`; every v0 field keeps its meaning. C-POPUPS added `popup-open`, `popup-blocked` and
`tab-created`'s `opener`/`cause` (compatible additions, still protocol 1). (Measured by C4-SPIKE, `notes/C4-SPIKE.md`.)

- **One window per page.** Each page is a browser with its own top-level window, the page only (no
  toolbar, tab strip or menus): CEF 156 cannot put two Chrome-style browsers in one window.
- **One page is shown.** The first page is shown; a page from `new-tab` starts hidden; `show-tab`
  shows one and hides the one shown before. The host hides a page itself (web-host-cef:
  `BrowserView::SetVisible(false)` in its window, which stays mapped): it is then a background page as
  in Chrome (requestAnimationFrame stops, timers at most 1/s, audio keeps playing, memory stays). A
  window the shell merely does not draw is NOT enough (the page keeps running at full speed, checked).
  The shell draws only the shown page's window and gives it input; it sizes every page window to the
  page area, so switching does not re-layout the page.
- **Pairing.** Every page is announced by `tab-created` before its window is mapped, and windows are
  mapped in the order of their `tab-created`s; the shell pairs each new parentless toplevel with the
  oldest announced tab that has no window yet (FIFO). A parentless toplevel that arrives while no tab
  is waiting is held: the next `tab-created` takes it (its event was read after the window); until
  then it is shown as a dialog. A toplevel with a parent is a dialog. All windows have the app_id
  `web-host-cef`.
- **Page ids.** `tab` is the engine's page id (CEF: the browser identifier); positive, never reused
  in a run, not contiguous (forget-site's hidden browser takes one too). The shell's buffer numbers
  are its own.
- **Closing.** `close-tab` closes one page; closing the last page does **not** end the host (the shell
  opens an `about:blank` page first, KEYS.md T82). `close-all` (C-CLOSE-POLITE) asks every page to
  close politely: each may confirm with `beforeunload`, a page whose user stays stays open, and the
  host exits once the last page has closed. `quit` (and SIGTERM to the host) is forced: every page
  closes without asking the user (the host answers `beforeunload` itself) and the host exits within
  about 2 s at most. Without
  `--control` (standalone) the host exits when its last page closes.
- **Popups are pages** (C-POPUPS, "Popups" below): a `window.open` or `target=_blank` with a user
  gesture becomes a new page (`tab-created` with `opener`); one without a gesture is blocked and
  reported (`popup-blocked`), and `popup-open` opens it later.
- **No new-tab gesture opens a page** (user, 2026-11-23, C-NOTAB; kept by DECISIONS.md "A picker,
  bookmarks, buffers as tabs, popups like Chrome": "Ctrl/middle-click on links still open nothing").
  Ctrl+click, Shift+click, Ctrl+Shift+click on a link and Ctrl+Enter in a form do
  nothing at all (middle-click: amended 2026-11-24, `open-link`, DECISIONS.md): nothing opens and the page is not navigated. There is no context
  menu at all (C-NOMENU): the host suppresses Chrome's menu for every context, the right-click
  still reaches the page as a normal `contextmenu` event.

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
[--downloads <dir>] [--forget-site <site> [--no-window]] [--notify-sites <host,...>] [Chromium switches...]`

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
- `--notify-sites <host,...>` (C-BELL): only these hostnames (comma-separated, exact: no subdomains;
  an empty value: none) may show Web Notifications; every other site sees permission `denied`.
  Without it every site may ("Notifications"). The shell passes config.rs `NOTIFY_SITES`.
- Anything else starting with `--` is passed to Chromium unchanged.
- web-host-cef always runs Chromium with `--disable-popup-blocking`: the host is the popup blocker
  ("Popups"); Chromium's own blocker would cancel popups without telling CEF.

## Commands (shell -> host)

| text form | JSON | what it does |
|---|---|---|
| `open <url>` | `{"cmd":"open","url":"example.com"}` | load a URL; no scheme means `https://` |
| `back` | `{"cmd":"back"}` | history back |
| `forward` | `{"cmd":"forward"}` | history forward |
| `reload`, `reload hard` | `{"cmd":"reload"}`, `{"cmd":"reload","hard":true}` | reload; `hard` bypasses the cache (`:refresh hard`; CEF ReloadIgnoreCache) |
| `stop` | `{"cmd":"stop"}` | stop the current load |
| `find <id> <query> next\|prev [match_case] [screen_top]` | `{"cmd":"find","id":1,"query":"needle","dir":"next"}`, `id` (optional, default 0) is echoed in the answering `find-result`, optional `"match_case":true` (default false: case-insensitive; the shell's smartcase sets it when the query has a capital), optional `"screen_top":true` | find in page; the same query again steps to the next/previous match; a new query starts at the match the engine picks from where the page is scrolled (C5 measured: from the top, the first match; scrolled down, the one nearest above the screen). With `screen_top` (new query only; C5b, KEYS.md T23) the host first collapses the page's selection at the first text at or below the top of the screen (page script, answered by a console line), so the first current match is the first one at or below the top of the screen; if none is, the first match is current and the `find-result` says `wrapped` |
| `find-clear` | `{"cmd":"find-clear"}` | end the find: highlights go, the active match stays selected. CEF/Chromium then focuses the link or control holding the active match (only now, not while the find is open; C-ENTER), so an Enter given to the page afterwards follows that link or presses that button |
| `zoom <n>` | `{"cmd":"zoom","factor":1.25}` | page zoom as a factor, 1.0 = 100 %, 0.25..5 |
| `caret on\|off` | `{"cmd":"caret","on":true}` | the engine's caret browsing (a text caret in the page that the arrow keys move; `:set caret`, C-CARETCMD). Off at every host start, also when an earlier run turned it on; takes effect at once, no reload. web-host-cef: Chrome's preference `settings.a11y.caretbrowsing.enabled` set with `CefRequestContext::SetPreference` on the global request context (`CefString::from("")` as the error out-parameter, see "Third-party cookies off"), read back and logged (`caret: ...=true: set=1 now=Some(true)`); a failure is an `error` event. Needs no page |
| `forget-site <site>` | `{"cmd":"forget-site","site":"https://mail.example.com/"}` | forget a site: cookies and storage (below); answered by `forgot` |
| `new-tab <req> [url]` | `{"cmd":"new-tab","req":3,"url":"example.com"}` | create a page (default `about:blank`; no scheme means `https://`), hidden; answered by `tab-created` with the same `req`. While quitting: an `error` |
| `show-tab <id>` | `{"cmd":"show-tab","tab":4}` | show page `id` (visible, keyboard focus), hide every other page; unknown id: `error` |
| `close-tab <id>` | `{"cmd":"close-tab","tab":4}` | close page `id` (no `beforeunload` prompt); answered by `tab-closed`; unknown id: `error`. Closing the last page does NOT end the host |
| `blur-field` | `{"cmd":"blur-field","tab":4}` | C-AUTOFOCUS: blur the page's focused text field (in the frame of the last `editable-focused true`, else the main frame); the page keeps it as its last field. Answered by `field-result` (`op` `blur`; `ok` false when no field had the focus). The page's blur handlers run |
| `focus-field last\|first` | `{"cmd":"focus-field","which":"last"}` | C-AUTOFOCUS: focus the last text field focused in the document (`last`: in the frame of the last `editable-focused true`, else the main frame; only if still connected, rendered, not disabled or read-only) or the first visible one (`first`: main frame; input text/search/url/tel/email/password/number, textarea, contenteditable; the first in the viewport, else the first in the document). Answered by `field-result` (`ok`: a field has the focus now) |
| `popup-open <id> <n>` | `{"cmd":"popup-open","tab":1,"popup":7}` | open blocked popup `n` of page `id` ("Popups"); answered by a `tab-created` with `opener` (`cause` `popup-open` or `popup-fresh`); an unknown page or popup (already opened, or forgotten at a new document): `error` |
| `close-all` | `{"cmd":"close-all"}` | C-CLOSE-POLITE: close **every** page politely: each runs its `beforeunload` (CEF `CloseBrowser(false)`), so a page may show its "Leave site?" dialog; a page whose user chooses to stay stays open and the host keeps running normally (`new-tab` etc. still work). `tab-closed` is sent for each page that closes; the host exits after the last one has closed. Sent again, it asks the remaining pages again |
| `quit` | `{"cmd":"quit"}` | close **every** page, forced (CEF `CloseBrowser(true)`). CEF 156 still runs `beforeunload` then and would show the dialog: the host answers it "Leave" itself while quitting (`beforeunload answered Leave without a dialog`). If a page still has not closed 2 s later (a dialog that was already on screen), the host ends anyway (`quit: N page(s) did not close; ending anyway`, then it exits with status 0 without CEF's shutdown). **SIGTERM to the host is the same as `quit`** (it logs `SIGTERM: forced quit`) |

**`tab` on page commands.** `open`, `back`, `forward`, `reload`, `stop`, `find`, `find-clear`,
`blur-field`, `focus-field` and `zoom` take an optional `"tab":<id>` (`{"cmd":"reload","tab":4}`): they act on that page, shown or
hidden; without it on the shown page. An unknown `tab` is an `error`. `forget-site` and `caret` are
host-wide.

Commands that need no answer get none; a command the host cannot use (not JSON,
unknown `cmd`, missing field, zoom out of range) gets an `error` event.

## Events (host -> shell)

| text form | JSON |
|---|---|
| `hello` | `{"event":"hello","protocol":1,"engine":"cef"}` |
| `tab-created <id>` | `{"event":"tab-created","tab":1}`; the answer to `new-tab`: `{"event":"tab-created","tab":4,"req":3}`; a popup: `{"event":"tab-created","tab":5,"opener":4,"cause":"popup"}` |
| `tab-closed <id>` | `{"event":"tab-closed","tab":1}` |
| `url <id> <url>` | `{"event":"url","tab":1,"url":"https://example.com/"}` |
| `title <id> <text>` | `{"event":"title","tab":1,"title":"Example Domain"}` |
| `load <id> start\|end <status> <kind>` | `{"event":"load","tab":1,"phase":"end","status":200,"kind":"new"}`, optional `"error":"ERR_ABORTED"` |
| `load-error <id> <code> <text>` | `{"event":"load-error","tab":1,"code":-105,"name":"ERR_NAME_NOT_RESOLVED","text":"ERR_NAME_NOT_RESOLVED"}` |
| `editable-focused <id> true\|false` | `{"event":"editable-focused","tab":1,"focused":true}` |
| `field-result <id> <op> <ok>` | `{"event":"field-result","tab":1,"op":"last","ok":true}` |
| `find-result <tab> <id> <i> <n>` | `{"event":"find-result","tab":1,"id":7,"index":2,"count":5}`, optional `"wrapped":true` (only after `screen_top`: no match at or below the top of the screen, `index` is the first match) |
| `crashed <id>` | `{"event":"crashed","tab":1,"reason":"TS_PROCESS_CRASHED (exit code 139)"}` |
| `popup-blocked <id> <n> <url>` | `{"event":"popup-blocked","tab":1,"popup":7,"url":"https://idp.example/auth","name":"login","frame":"sub","features":"popup,width=400,height=300"}` |
| `open-link <id> <url>` | `{"event":"open-link","tab":1,"url":"https://a.example/x"}` |
| `notify <id> <title> <body>` | `{"event":"notify","tab":1,"title":"New message","body":"Ana: lunch?","tag":"msg-42","origin":"https://chat.example.com"}` |
| `forgot <site> ok`, `forgot <site> partial <what failed>` | `{"event":"forgot","site":"https://mail.example.com/","domain":"example.com","result":"ok","cookies":3,"origins":["http://example.com","https://example.com","https://mail.example.com"]}`, with `"result":"partial","failed":["storage https://x.example.com (timeout)"]` |
| (error) | `{"event":"error","message":"..."}` |

- `tab-created`: once per page, **before the page's window is mapped**. The window's Wayland
  app_id (X11 WM_CLASS) is `web-host-cef`. `req` echoes the `new-tab` it answers (none for the
  first page). A page the host created for a popup has `opener` (the page it came from) and `cause`:
  `popup` (the page opened it with a user gesture), `popup-open` (`popup-open` re-ran a blocked
  one) or `popup-fresh` (`popup-open` loaded it fresh); it starts hidden like a `new-tab` page, and the
  shell shows it with `show-tab` (the shell switches to every popup).
- `popup-blocked`: a popup without a user gesture was cancelled ("Popups"). `popup` numbers the
  page's blocked popups (from 1, never reused in the page); `url` is `""` when the popup had none;
  `name` the target name the page gave; `frame` `main` or `sub` (an iframe asked); `features` the
  window features as `window.open` takes them, rebuilt by the host (`popup`, then `left`, `top`,
  `width`, `height` as given).
- `open-link` (C-MIDCLICK): the user middle-clicked a link (no modifier held) in page `tab`; `url` is its absolute
  `http:`/`https:`/`file:` target. The click's own navigation was cancelled in the renderer; the shell opens the URL as `:new <url>`
  (KEYS.md T105). Pages cannot send it: the renderer script holds the only reference to the function that does.
- `notify` (C-BELL): page `tab` (any frame of it) showed a Web Notification ("Notifications" below).
  `title`, `body`, `tag` as the page gave them (`tag` `""` when none), `origin` the notifying
  frame's origin (`null` when opaque). The host caps them (title 200, body 500, tag 100, origin 200
  characters; a cut one ends with `…`), turns control characters (newlines) into spaces, and
  rate-limits per page (a burst of 5, then one every 2 s; the rest is dropped, logged once per run).
- `tab-closed`: once per page, when its browser has closed (`close-tab`, `close-all`, `quit`).
- Every page event (`url`, `title`, `load`, `load-error`, `editable-focused`, `find-result`,
  `crashed`, `popup-blocked`, `notify`) carries the page's `tab`; hidden pages send theirs too.
- `url`: the main frame's URL, sent when it changes (also for same-document
  navigations). For every load, the new `url` comes **before** its `load end`.
- `title`: sent when the title changes (not repeated for the same text).
- `load`: main-frame loads only; every `start` is followed by one `end` of the same
  kind.
  - `kind`: `new` (open, link, form, script navigation),
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
- `editable-focused`: when keyboard focus moves into or out of an editable element
  (text inputs, textareas, contenteditable) in any frame of the page: `false` on a
  change only, `true` for every focus that lands in an editable, also straight from
  another one (C-AUTOFOCUS). A new document starts with `false`. The shell uses it for
  INPUT mode (KEYS.md T29, T32).
- `field-result` (C-AUTOFOCUS): the answer to `blur-field` (`op` `blur`) or `focus-field`
  (`op` `last` / `first`); `ok` whether a field was blurred / has the focus now. web-host-cef:
  a script installed in every frame's main world before any page script (`field.js`) keeps the
  document's last focused text field and returns the function the renderer runs; it is never on
  `window`, so pages cannot call it or forge the answer (a renderer process message, not a
  console line).
- `find-result`: one per `find` command (the latest, if several are pending), when the
  engine's count is final. `id` echoes the `find` it answers. `index` is the 1-based
  active match (0 when there are no matches), `count` the matches. The host sends no
  `find-result` when no `find` is pending (CEF reports `0 0` at a cross-document load
  start; `load start` and `find-clear` also end the pending request), and a shell
  ignores one whose `id` it is not waiting for.
- Find after a same-document navigation: no new event; the shell reacts to
  `load ... same-document` by sending `find-clear` (C5-FRAG).
- `crashed`: the page's renderer process died (crash, killed, out of memory). It concerns
  that page only; the page stays; `open` or `reload` loads it again in a new renderer.

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
- **Afterwards** the shell reloads every page that shows the site (STASH.md; `reload` with
  `tab`); the host does not.

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

## Popups (C-POPUPS)

Built by C-POPUPS from C4-SPIKE's proposal (`notes/C4-SPIKE.md`); DECISIONS.md "A picker, bookmarks,
buffers as tabs, popups like Chrome" ("Popups follow Chrome"). The shell's side is KEYS.md "Popups".

**The host is the popup blocker.** web-host-cef runs Chromium with `--disable-popup-blocking`, so every
`window.open` and `target=_blank` reaches the host (CEF `OnBeforePopup`, with the frame that asked, the
URL, the target name, the disposition, the window features and whether the page had a user gesture).
Chromium's own blocker would cancel gesture-less popups before CEF hears of them (no callback, C4-SPIKE).
By disposition (`host-cef/src/newtab.rs`) and gesture:

- **foreground tab and popup** (`window.open` with or without features, a `target=_blank` link or form)
  **with** a user gesture: a new page in a window of its own (`tab-created` with `opener`, `cause`
  `popup`). `window.opener` works in it, so login and payment popups can post back to their opener;
  `window.close()` in it closes only it (`tab-closed`, the shell kills its buffer). `target=_blank`
  links get no `window.opener` (Chromium's implicit `noopener`, as in Chrome); the engine does not
  tell a `target=_blank` click from a `window.open` call. Size and position features are ignored (the
  shell sizes every page).
- the same **without** a gesture: cancelled (the page's `window.open` returns `null`, as with Chrome's
  blocker), recorded in its page and reported as `popup-blocked`. A page forgets its blocked popups when
  it starts a cross-document load (reload included); the shell clears its list on the same `load start`.
- background tab, new window and the rest (Ctrl/Shift click, C-NOTAB): dropped, nothing opens. A middle-click is told apart in the
  renderer (`auxclick` button 1) and becomes `open-link` (C-MIDCLICK), never a popup.
- this tab (`CURRENT_TAB`, ...): loaded in the page that asked.

**`popup-open <id> <n>`** takes blocked popup `n` out of the page's record and:
- runs `window.open(url, name, features)` again **in the frame that asked** (main frame or iframe;
  web-host-cef: `CefFrame::ExecuteJavaScript` in that frame) with a one-shot allow for that frame and
  URL, so the host lets that one call through without a gesture: the new page (`cause` `popup-open`)
  has `window.opener` = that frame. The allow is used up by that popup; if no popup comes within 1 s
  (the page replaced `window.open`, the frame went meanwhile) the host loads the URL fresh instead;
- if the frame is gone (an iframe removed): loads the URL in a new page (`cause` `popup-fresh`, no
  opener).

Limits of the re-run: the page's original call already returned `null`, so the page never gets a
handle to the popup (flows that only need `window.opener` in the popup work; a page that polls
`w.closed` does not); it calls the page's current `window.open` (a page that replaced it gets its own
function called); a popup that had no URL (`window.open()` then `document.write`) opens blank. Not now
(DECISIONS.md): a per-site "always allow popups" list.

## Notifications (C-BELL)

A page's Web Notifications go to the shell as `notify` events and never to the desktop
(web/notes/C-BELL-SPIKE.md: CEF exposes only the permission prompt, and Chrome's runtime would show a
granted notification itself over D-Bus). web-host-cef:

- replaces, in every script context of every frame before any page script runs, `Notification`
  (once Blink has installed its own: C-BELL-CRASH; a frame's window taken in the same turn that
  created it, other than through `iframe.contentWindow`, still has Chromium's, which is `denied`
  and blocked), `ServiceWorkerRegistration
  .prototype.showNotification`/`getNotifications` and `permissions.query({name:'notifications'})`.
  An allowed site (every site, or `--notify-sites`) sees permission `granted` without a prompt;
  `new Notification(...)` fires `show` and `showNotification` resolves, and each one becomes a
  `notify`. Others see `denied`: the constructor fires `error`, `showNotification` rejects, nothing
  is sent. `close()` fires `close`; `getNotifications()` is always empty; `onclick` never fires.
- sets Chromium's own notification permission to BLOCK (and drops the profile's per-site
  exceptions) at start, and denies any notification permission prompt at once: no Chrome bubble,
  and nothing Chromium could show itself. Notifications from inside a worker (a service worker's own
  `self.registration.showNotification`, push) are not reached by the replacement: they fail
  (`denied`) and are lost silently.
- Pages that look can tell (`Notification.toString()` and the `HTMLIFrameElement.prototype
  .contentWindow` getter are not native code, `navigator` has an own `serviceWorker`), and
  "granted" everywhere differs from
  a typical Chrome profile.

## Not in the protocol (engine default for now)

Downloads are saved without asking (see `--downloads`); other permission prompts and JS
dialogs (`alert`, `confirm`, `prompt`, `beforeunload`) are the engine's own UI. Each
gets its own item later.

## Versioning

`hello` carries `protocol`. Adding fields or events is compatible (both sides ignore
what they do not know); changing or removing one bumps the version.

## Host exit statuses

0 normal; 2 bad command line or no control socket; 3 (C-SINGLE) CEF's `initialize` failed,
almost always because another browser process holds the profile (Chromium's process
singleton: the second process hands over its command line and `initialize` returns 0). The
host prints one line on stderr naming the profile and, if `<profile>/SingletonLock` exists, its
`<host>-<pid>` target; the shell logs that the host could not start. For `--forget-site` runs
the codes 0/1/2 are as above.
