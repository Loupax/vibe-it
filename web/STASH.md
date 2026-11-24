# stash: what web keeps on disk

Carried over from `../scratchy/STASH.md` (2026-11-23, decision "Architecture C" in
DECISIONS.md). In scratchy, stash was the plain-file `localStorage` store plus the state
directory, forget-site and history. In web the engine owns page storage and cookies, so
what carries over is: the state directory, history (unchanged format), forget-site (now
through the engine), and the cookie partitioning goal. The plain-file `localStorage`
store is dropped; see the end of this file.

## State directory

- `--state-dir DIR` (exact) / `WEB_STATE_DIR` (a base); default `$XDG_STATE_HOME/web`
  (`~/.local/state/web` when unset). Created with mode 0700.
- `--profile NAME` (C-PROFILE, SCRIPTING.md "Files and profiles"): the base (`WEB_STATE_DIR`, else
  `$XDG_STATE_HOME/web`, else `~/.local/state/web`) gets `/profiles/NAME`, so each profile has its
  own history, bookmarks, engine profile (cookies, storage), lock and socket, and its own running
  web (C-SINGLE below finds it through this directory: `web --profile NAME URL` and
  `web --profile NAME --forget-site SITE` act on that profile only). `--state-dir DIR` is used as
  given and wins over everything, `--profile` included (the script still follows the profile).
  `WEB_STATE_DIR` is a base, not an exact path, so a profile under it stays distinct. The
  directories are created 0700, parents included. Without `--profile`: as above, unchanged.
- A tmpfs directory (e.g. `$XDG_RUNTIME_DIR/web`) gives state that survives web
  restarts but not a logout (as in scratchy).
- If the directory cannot be used, web logs once and runs with an in-memory profile
  (the engine's incognito mode) and no history.

```
DIR/
  history                 <- the shell's (format below)
  bookmarks               <- the shell's ("Bookmarks" below)
  .history.lock
  engines/
    cef/                  <- web-host-cef's profile: cookies, localStorage, IndexedDB, cache
    webkit/               <- web-host-webkit's data and cache directories
  profiles/NAME/          <- with --profile NAME the state directory is this one, same tree inside
                             (DIR is $XDG_STATE_HOME/web for the default profile only)
```

- Each engine host gets its own profile directory from the shell and must never fall
  back to a default one (`~/.config/...`), as the spike already does.
- The engine's files are in its own format (Chromium: LevelDB and SQLite; WebKit:
  SQLite) and are not meant to be edited by hand. Logins do not carry between engines.

## Cookies

The engine owns the jar (DECISIONS.md "Cookies"), persisted in its profile directory.
scratchy's jar was partitioned by top-level site (like Firefox's Total Cookie Protection).
The closest engine settings, set by each host at start (P-COOKIES, done for CEF):
- CEF: third-party cookies blocked (`profile.cookie_controls_mode` = 1, set through
  `CefRequestContext::SetPreference`; the host flag `--allow-third-party-cookies` undoes
  it for testing). CHIPS partitioned cookies and storage partitioning stay on, as
  checked by `host-cef/test/cookies.py`. Details: PROTOCOL.md "Third-party cookies off".
- WebKitGTK (E1): `WEBKIT_COOKIE_POLICY_ACCEPT_NO_THIRD_PARTY`, plus its built-in
  tracking prevention (ITP).
Not exactly scratchy's partitioning: a third party gets no cookie at all instead of its
own jar per top-level site (only `Partitioned` cookies and partitioned storage are
per-site), so embedded logins that need a third-party cookie break.

## Forget site

`web --forget-site <site or URL>` (and `:forget` later, if asked), item P-FORGET. It is
part of the engine-host interface, so every engine host implements it and the shell
does not know how.

- **Site** = registrable domain (eTLD+1) of the argument, as scratchy's `site_of`:
  `https://mail.example.co.uk/x` forgets `example.co.uk` and all its subdomains.
  **web:** both http and https (P-FORGET; cookies do not keep the schemes apart
  either), so the scheme of the argument does not matter.
- **What goes:** every cookie whose domain is the site or under it, and the storage of
  every origin under it: localStorage, sessionStorage, IndexedDB, Cache Storage, service
  workers, the HTTP cache entries. History is not touched (as in scratchy).
- **Protocol** (C1's control socket): shell->host `forget-site <site>`; host->shell
  `forgot <site> ok`, or `forgot <site> partial <what failed>` (one line, logged and
  shown in the status bar). Open tabs of that site are reloaded afterwards by the shell.
  Details and limits: PROTOCOL.md "forget-site".
- **Who runs it:** an engine locks its profile, so a second host on the same profile
  cannot start. If a web instance runs, `web --forget-site` sends the command to its host
  (through the shell); if none runs, it starts the host with `--no-window` for that one
  command and exits. Exit status 0 `ok`, 1 `partial`, 2 refused; stdout is the host's
  `forgot` event (one JSON line) on both paths.
- **Finding the running instance (P-FORGET-SHELL):** the shell writes its pid to
  `DIR/web.pid` and listens on `$XDG_RUNTIME_DIR/web-<pid>/forget.sock`, next to its control
  socket. `web --forget-site` reads the pid file, checks that the pid is a live `web`
  (`/proc/<pid>/comm`), connects, sends `forget-site <site>` and reads the host's answer.
  Chosen over a socket inside the state dir (socket paths are limited to ~107 bytes and
  state dirs can be deep) and over a lock file (a pid file is stale-checkable without
  holding anything). A stale pid file, a recycled pid or a refused connection mean "no
  instance": the command runs the host itself. The running shell shows `forgot ...` in its
  status bar and reloads every buffer whose page is on the site (C4).
- **`:forget-site [site]`** in the shell does the same for the page's own site (or the
  site given); it exists because the status line and the reload come for free.
- **Run directories (C-SOCKDIR):** `$XDG_RUNTIME_DIR/web-<pid>/` holds the control and
  forget sockets and the help page. At start the shell removes the `web-<pid>`
  directories whose pid is not a running `web` (a hard-killed shell leaves one); it
  deletes the files it makes by name, then the directory, never a glob.
- **Per engine:**
  - CEF (P-FORGET, done; user 2026-11-23: "use Storage.clearDataForOrigin"):
    - Cookies: `CefCookieManager::VisitAllCookies`, deleting those under the site.
    - Storage: `Storage.clearDataForOrigin` (`all`) through `ExecuteDevToolsMethod`,
      in process, on a hidden browser, for each origin. The origins come from the
      host's record of the origins it loaded plus the cookie hosts.
    - HTTP cache: cleared whole; no per-site removal is reachable.
    - Details: PROTOCOL.md "What every engine host implements".
  - WebKitGTK: `webkit_website_data_manager_fetch` returns records grouped by
    registrable domain; `webkit_website_data_manager_remove` those of the site, all
    data types (the same `SiteForgetter` contract, PROTOCOL.md).

## History

Unchanged from scratchy, so `cp ~/.local/state/scratchy/history ~/.local/state/web/`
brings scratchy's visits along. **web: where a visit comes from:** the shell writes the
line; the engine host reports a main-frame load that ended with a response, its final
URL after redirects, its kind (new, reload, traverse, same-document) and the title at
load end (TODO.txt C1 `load` / `title` events). The rules below decide what is recorded.
Built in C3 (`shell/src/history.rs`, `Visits`): a `load end` of kind `new` with a status
and no `error` records the last `url` with the title seen since that load's `start`; a
title that is only the URL (Chromium's stand-in for a page without one) is recorded empty.

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

## Bookmarks (C-BOOKMARKS)

`DIR/bookmarks`, next to `history`: plain UTF-8, one bookmark per line, `url<TAB>title`, in the
order they were added; meant to be editable by hand (a line without a tab is a URL with no title;
blank lines are skipped). Written by `:bookmarkAdd` (the current page; a URL already present gets
its title updated) and `:bookmarkDelete [url]`, with the same lock as the history,
as a temp file `.bookmarks.tmp-<pid>` renamed over the file (mode 0600). Control characters in a title
become spaces; a hand-edited duplicate URL keeps its first place and its last title; the rewrite drops blank lines. `:bookmarkSearch`
lists them in the picker; `:go`'s ghost source puts bookmarks before history. `--forget-site` does not
touch them (as history).

## Not carried: the plain-file localStorage store

Dropped (user, 2026-11-23; DECISIONS.md): the engine keeps localStorage (and
sessionStorage, IndexedDB) in its own profile. scratchy's one-file-per-key store existed
because scratchy had no storage of its own; it comes back only with a use case for
centralizing storage outside the engine.

## One web per state directory (C-SINGLE)

- A second `web [URL]` with the same state directory does not start a second engine host
  (Chromium locks the profile). It finds the running shell as `web --forget-site` does (`DIR/web.pid`,
  checked through `/proc/<pid>/comm`) and talks to the same socket,
  `$XDG_RUNTIME_DIR/web-<pid>/forget.sock` (the name is kept, so a shell started before C-SINGLE is
  still found; it is now the instance's command socket). One request line per connection:
  `forget-site <site>` (answer: the host's `forgot` event) or `open <url>` (answer
  `{"event":"opened","url":...}` or `{"event":"error","message":...}`). The shell loads the URL in its
  page and shows `opened <url> (from another web)`; the second `web` prints what happened and exits 0.
  A URL is read as `:go` reads it (no scheme: `https://`; relative forms are refused).
- `web` with no URL while one runs prints "already running (pid N ...)" and exits 0, starting
  nothing. It does not raise the window: a Wayland client cannot take focus without an activation
  token, and X11 has no route from here that works on every desktop.
- Profile lock: Chromium keeps `<profile>/SingletonLock`, a symlink to `<hostname>-<pid>`, with
  `SingletonCookie` and `SingletonSocket`. When no live web owns the state directory, the shell
  checks it before starting the host: a lock whose pid is dead is removed (those three files, by
  name, with a log line); a lock whose pid lives (or is on another host, or has an odd target) is an
  error naming the profile, the pid and what to do, exit 2, and the lock is never touched. If the host
  itself still gets a failed `initialize` it exits 3 with a message (PROTOCOL.md "Host exit statuses").
