# stash: what web keeps on disk

Carried over from `../scratchy/STASH.md` (2026-11-23, decision "Architecture C" in
DECISIONS.md). In scratchy, stash was the plain-file `localStorage` store plus the state
directory, forget-site and history. In web the engine owns page storage and cookies, so
what carries over is: the state directory, history (unchanged format), forget-site (now
through the engine), and the cookie partitioning goal. The plain-file `localStorage`
store does not carry over as is; see the end of this file.

## State directory

- `--state-dir DIR` / `WEB_STATE_DIR`; default `$XDG_STATE_HOME/web`
  (`~/.local/state/web` when unset). Created with mode 0700.
- A tmpfs directory (e.g. `$XDG_RUNTIME_DIR/web`) gives state that survives web
  restarts but not a logout (as in scratchy).
- If the directory cannot be used, web logs once and runs with an in-memory profile
  (the engine's incognito mode) and no history.

```
DIR/
  history                 <- the shell's (format below)
  .history.lock
  engines/
    cef/                  <- web-host-cef's profile: cookies, localStorage, IndexedDB, cache
    webkit/               <- web-host-webkit's data and cache directories
```

- Each engine host gets its own profile directory from the shell and must never fall
  back to a default one (`~/.config/...`), as the spike already does.
- The engine's files are in its own format (Chromium: LevelDB and SQLite; WebKit:
  SQLite) and are not meant to be edited by hand. Logins do not carry between engines.

## Cookies

The engine owns the jar (DECISIONS.md "Cookies"), persisted in its profile directory.
scratchy's jar was partitioned by top-level site (like Firefox's Total Cookie Protection).
The closest engine settings, set by each host at start:
- CEF: block third-party cookies (the `profile.cookie_controls_mode` preference); CHIPS
  partitioned cookies stay allowed.
- WebKitGTK: `WEBKIT_COOKIE_POLICY_ACCEPT_NO_THIRD_PARTY`, plus its built-in
  tracking prevention.
Neither is exactly scratchy's partitioning; the item is P-COOKIES in TODO.txt.

## Forget site

`web --forget-site <site or URL>` (and `:forget` later, if asked), item P-FORGET:
1. engine: remove the site's cookies, localStorage, IndexedDB, cache and service workers
   through the host (CEF: `CefCookieManager::DeleteCookies` plus the
   `Storage.clearDataForOrigin` method through `ExecuteDevToolsMethod`, to be checked that
   it sets no automation flag; WebKitGTK: `webkit_website_data_manager_remove` for the
   site's data records).
2. history is not touched (as in scratchy).

## History

Unchanged from scratchy, so `cp ~/.local/state/scratchy/history ~/.local/state/web/`
brings scratchy's visits along. **web: where a visit comes from:** the shell writes the
line; the engine host reports a main-frame load that ended with a response, its final
URL after redirects, its kind (new, reload, traverse, same-document) and the title at
load end (TODO.txt C1 `load` / `title` events). The rules below decide what is recorded.

The first reader is `:go` completion (KEYS.md "Completion", QCOMP-D). Settings
(`HISTORY`, on/off, and `HISTORY_MAX_LINES`) are constants in `src/config.rs`.

- **File:** `DIR/history`, mode 0600, one line per visit:
  `<unix time in ms>\t<url>\t<title>\n`. Plain text so scripts can read it:
  `cut -f2 history` = the URLs; `cut -f2 history | sort | uniq -c | sort -rn` = URLs by
  number of visits. The title is for scripts (scratchy's own completion matches URLs
  only); the time breaks ties and allows "forget the last hour" later.
- **What is a visit** (user, 2026-11-20): a top-level navigation the user or the page
  started and that loaded: a link click, `:go`, a page-script navigation
  (`location`, form submit). After redirects the FINAL URL is recorded. The `#fragment`
  is stripped (`page#a` and `page#b` are one URL). One line at the page's `load` (or
  when the load gives up), with `document.title` at that moment (empty if none).
- **Not a visit (not recorded):** `:refresh`/reload, history traversal (`h`/`l`,
  `history.back()`), same-document navigations (fragment-only, `pushState`), iframes,
  failed loads (no response), opaque origins (`file:`, `data:`, `about:`), `:help`. These
  would only inflate the counts.
- **Escaping:** tabs, newlines and other control characters in the title become a
  space; the URL is the serialised URL (no tabs or newlines). A line is cut at 4096
  bytes (the title is shortened), so a single `write` with `O_APPEND` is atomic and
  several scratchy processes can append at the same time without a lock.
- **Size:** at startup, when the file has more than `HISTORY_MAX_LINES` lines, scratchy
  keeps the newest ones. This is also what keeps the ranking current: counts only cover
  the last `HISTORY_MAX_LINES` visits, so sites no longer visited sink out of the top
  with no frecency formula. Trim: write a temp file + rename, under an exclusive flock on
  `DIR/.history.lock` (appenders do not lock: an append that lands between the read
  and the rename is lost, which is acceptable for history).
- **Hand edits** are fine; lines that do not parse are skipped by the built-in reader.

### Ranking (the built-in `:go` source, user 2026-11-20)

`complete_history_urls` reads the whole file each time (one pass; ~100k lines is tens of
ms; once per command line, KEYS.md "Completion") and returns two groups, each filtered by the word being completed (prefix match on
the URL with `https://`/`http://` and `www.` stripped; empty word = everything):

1. **Sites:** one entry per host, the site root `scheme://host/` (host as visited,
   `www.` kept), ranked by the number of visits to ANY page of that host. Its title: the
   newest title recorded for exactly that root URL, else the host. Typing `you` puts
   YouTube's home page first even if most visits were to watch pages.
2. **Pages:** one entry per URL, ranked by that URL's own visit count. A URL that is a
   site root already listed in group 1 is not repeated.

Ties (both groups): the most recent visit first. This is the order of the ghost
suggestion: the first match is shown, Tab walks down the list (KEYS.md T45-T53).

- `--forget-site` does not touch history (yet).

## Not carried: the plain-file localStorage store

scratchy's stash kept `localStorage` as one file per key under
`storage/<site>/<origin>/`, readable and editable by hand (e.g. `vimOff` for
social.loupax.com), with per-site locks and a generation counter. In web, CEF and
WebKitGTK keep localStorage in their own databases and offer no hook to put another
store behind it. What remains possible, each only if the user asks (open question in
TODO.txt P-STASH):
- **Read and write single keys from the shell** (`:stash get/set <origin> <key>`):
  CEF through the `DOMStorage` methods of `ExecuteDevToolsMethod`; WebKitGTK has no API
  for values, only by running a script in a page of that origin.
- **Replace `window.localStorage` with our own file-backed object**: WebKitGTK can do
  it from a web process extension (file access added to its sandbox); CEF cannot without
  synchronous renderer IPC. Pages can tell the difference, `storage` events and quotas
  become ours to implement, and the two engines would differ. Not recommended.
