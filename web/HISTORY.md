# History

Carried over from `../scratchy/STASH.md` "History" and "Ranking" (2026-11-23, decision
"Architecture C" in DECISIONS.md). The format, rules and ranking are scratchy's; notes
marked **web:** are the only changes.

- **web: state directory.** `--state-dir DIR` / `WEB_STATE_DIR`; default
  `$XDG_STATE_HOME/web` (`~/.local/state/web` when unset), mode 0700. The file is the
  same format, so `cp ~/.local/state/scratchy/history ~/.local/state/web/` brings
  scratchy's visits along.
- **web: where a visit comes from.** The shell writes the line; the engine host reports
  the events on the control socket (web/TODO.txt C1): a main-frame load that ended with a
  response, its final URL after redirects, whether it was a reload, a history traversal
  or a same-document navigation, and the title at load end. The rules below decide what
  is recorded from them.
- **web: not carried:** scratchy's `localStorage` store (the rest of STASH.md) and
  `--forget-site`. The engine owns page storage and cookies; forgetting a site becomes an
  engine-host command (web/TODO.txt P-FORGET).

## File and rules

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
