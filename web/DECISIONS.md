# web — user decisions

A newer decision overrides an older one; add new ones at the top with the date.

## 2026-11-27 Yank the page's selection: `:yank` and `y`; the built-in `yy` goes (C-YANK)

Issue #771. User, 2026-11-27: "I want to introduce a yank command, so I can copy selections from normal
mode."

- **`:yank` (no argument), bound to `y`**, a plain one-key NORMAL binding like `j` or `n` (KEYS.md T54,
  T183-T186). `:yank link` / `:yank image` stay reserved for the parked C-CMDS.
- **The built-in `yy` goes; it is a user mapping now.** User: "I think we should also remove the built in
  yy and have it done in js only, since all the functionality is already there." Then: "go ahead and update
  it. I'll update my init files manually once it rolls out." The replacement:
  `web.map("n", "yy", ":let @+ = @%<CR>");`. With it, the mapping wait (T118) does what a `y` timeout
  would: `yy` copies the URL, a lone `y` yanks once the wait runs out. No new timer, no Y state. A deliberate
  exception to SCRIPTING.md's "how it behaves by default stays in `shell/src/config.rs`": a profile with no
  `init.js` has no `yy`. `:let @+ = @%` stays built in.
- **What is copied is whatever the page has selected now**: made with the mouse, or the active match that
  `/foo` Esc leaves selected (Chromium keeps it selected when the find ends; C-ENTER relies on that). vim
  would not copy a search match. User: "I suppose it would make sense if we add regex matching at some
  point. I say accept it."
- **A keyboard way to make a selection (visual mode) is out of scope**; it stays parked with C5-ENTER /
  QCURSOR-D.
- **The host reads the selection, the shell writes register `+`**, the path `:let @+ = @%` uses
  (PROTOCOL.md "Selection"). Not a Ctrl-C sent to the page (pages take Ctrl-C over, a focused field would
  get it, and the shell could not tell "copied" from "nothing selected"). Not the primary selection the
  shell already bridges (Chromium does not set primary for a find's match or a script's selection).

## 2026-11-27 Ctrl-^ is the command `:bufferAlternate`; no built-in key (C-BUFALT)

Issue #770. User, 2026-11-27: "Create a github issue for exposing this as a command so I can customize it.
Remove it from the built-in Ctrl-^ hotkey so the init file is the only source of truth. I'll edit my file
when the feature lands."

- **`:bufferAlternate`** shows the previously shown buffer (behaviour unchanged, KEYS.md T84). No alias.
- **The built-in Ctrl-^ / Ctrl-6 rows are gone.** Unmapped, the keys do nothing in NORMAL. `init.js` is the
  only source: `web.map("n", "<C-^>", ":bufferAlternate<CR>");`.

## 2026-11-26 Built-in modules `web:url` and `web:api` (C-SCRIPT-URL)

Issue #767. A mapping that jumps to the first Slack buffer, whatever its path, has to read the hostname of
each buffer's URL; `web.buffers.list()` gives `url` as a string and QuickJS has no `URL`, so the obvious
`new URL(b.url).hostname` threw `ReferenceError`, swallowed by a `catch`, and never matched. Spec:
SCRIPTING.md "Modules" ("Built-in modules"), "API", "`web:url`".

- **A `URL` class that behaves like the browser's**, so `URL.parse(b.url)?.hostname` works as people
  expect. `web.buffers.list()` is unchanged (no `host` field).
- **Importable, not a global** (user: "it has to be global? can't we make it importable?"):
  `import { URL } from "web:url"`.
- **A reserved `web:` prefix for built-in modules**, the pattern of Node's `node:url` (user: "how does node
  import from its stdlib?", then "update the issue to use web:url"). The resolver checks `web:` names
  before any file rule, so a built-in cannot be shadowed by a file and a file cannot be taken for one.
  Only the prefixed spelling: bare `"url"` stays a file import (refused as `not a .js file`); there is no
  list of reserved bare names. An unknown `web:` name is `no such built-in module`.
- **`web` is importable too, and stays a global** (user: "for now only focus on the URL and web itself. We
  should still be able to import it in case the user overwrites it and wants to reimport it").
  `import web from "web:api"` is the original object (kept in Rust, not looked up on `globalThis`), with
  one named export per key of `web`, read from the object so a later key is exported too. The global is
  not changed (still writable and configurable), so existing `init.js` files keep working; the precedent
  is Node's `process` global and `node:process`.
- **Backed by the Rust `url` crate** the shell already uses (the WHATWG parser), so scripts and the shell
  parse URLs the same way. No JS polyfill (`whatwg-url` and the like carry large IDNA tables and would be a
  second parser that could disagree). No new dependency, no rquickjs `macro` feature: `web:url` is a native
  module whose parse closure feeds a short embedded class; nothing is left on `globalThis`.
- **Read-only first**: getters only, no setters, no `searchParams`/`URLSearchParams`. `URL` and
  `URL.prototype` are frozen, as `web` is.
- Beyond the issue (C-SCRIPT-URL agent): an error thrown inside a built-in (`new URL("x")` uncaught) is
  shown at the user's line (`init.js:3: Invalid URL: x`): stack frames of `web:` modules are skipped
  when the bar's `FILE:LINE` is chosen, as native frames already were.
- Out of scope, to be filed separately if wanted: a global `URL`, bare `"url"`/`"web"` aliases, removing or
  locking down the `web` global, setters, `searchParams`, a `host` field on `web.buffers.list()`,
  `URLPattern`, URL normalization in `focusExisting`.

## 2026-11-26 Every clipboard type between the desktop and the page on Wayland (C-CLIPTYPES)

The bug (issue #764, seen by the user): screenshots taken with a dwl keybinding,

    area=$(slurp) && grim -g "$area" - | wl-copy

paste as the image into Chrome, but pasting into a page in `web` gives the **previous clipboard text**.
C6 carried text only: the shell read the desktop's clipboard as text through smithay-clipboard, which
fails on an image-only clipboard; that was taken for an empty clipboard, so the page kept its old
selection and pasted it.

User's choice among the options: **"B"**: on Wayland, carry **every MIME type** both ways, without the
shell reading the data. This reverses C6's "text only" (the shell README's "The clipboard", KEYS.md
"Paste") for Wayland.

- **Desktop → page**: the page is offered exactly the types the desktop's selection offers; when the page
  reads one, its fd goes to the desktop's offer (`receive`). The page's selection follows the desktop's
  `selection` events (sent on every focus gain), so there is no sync before a paste key any more.
- **Page → desktop**: the desktop is offered exactly the page's types; when a desktop client reads one, its
  fd goes to the page. Only while the shell window has the desktop's keyboard focus (a selection needs a
  recent input serial); otherwise the log says it is not carried.
- Both the clipboard and the primary selection (the latter when the desktop has
  `zwp_primary_selection_device_manager_v1`). Middle-click paste into the page stays unsupported
  (2026-11-23).
- Every source the shell sets on the desktop also offers `application/x-web-shell-own`, so the page's own
  selection coming back is not carried to the page again; the page never sees that type.
- **X11 stays text only**, exactly as before, and so does a Wayland desktop without `wl_data_device_manager`.
- The shell's own text uses (`yy`, `:let @+ = ...`, `@+`, paste into the `:` and `/` lines) still go
  through smithay-clipboard; while the desktop's clipboard is the page's own, they read the page's text
  from memory (asking the desktop would wait for the shell itself). The `+` register stays text.
- No size cap: the bytes never pass through the shell. Drag and drop is not carried.

## 2026-11-26 `focusExisting` also matches the URL a buffer's page was requested as (C-FOCUSEXISTING-REDIRECT)

User (issue #762), seen live: with `web.buffers.focusExisting = true`, clicking the same `target=_blank` link
twice opened two buffers. The journal:

```
[web-host-cef] event: blank-link 4 https://github.com/Loupax/vibe-it/issues/760
[web] command: {"cmd":"new-tab","req":5,"url":"https://github.com/Loupax/vibe-it/issues/760"}
...
[web-host-cef] event: url 6 https://github.com/Loupax/vibe-it/pull/760
```

GitHub redirects `/issues/760` to `/pull/760`; the first buffer showed `/pull/760`, which is not the link's
`/issues/760` byte for byte. Everything worked as designed; the match never saw the URL the buffer was asked
to load.

- **Still an exact match, no normalization**, but a link is matched against **two** URLs of each buffer: its
  current URL, and the **original URL of the page it shows now** (the URL requested before any redirects;
  CEF's `NavigationEntry::original_url()`, per navigation entry).
- **The original belongs to the page on screen.** Navigating elsewhere makes a new entry, so the old original
  stops matching: a link never sends you to a buffer that has moved on. Not matched: older history entries
  (back/forward pages), redirects done by the page's own JavaScript or `<meta refresh>` (new navigations).
- **How**: the host's `url` event gets an optional `original` (only when non-empty and different from `url`);
  the shell keeps the last one per buffer (every `url` event sets it, to none too) and `link_click` tests
  `url` or `original`. Not exposed to scripts (`web.buffers.list()` is unchanged). Spec: PROTOCOL.md, KEYS.md
  T170, SCRIPTING.md.

## 2026-11-26 `web.buffers.focusExisting`: a `_blank` link to an open page shows its buffer (C-FOCUSEXISTING)

User (issue #759): "catch clicks to links, and if I already have a buffer open to them, focus on that buffer
instead of opening a new one. I don't want this to be default behavior, but I think it makes sense to have it
as an option."

- **Opt-in**, set in `init.js`: `web.buffers.focusExisting = true`. Default `false`; boolean only (anything
  else is a script error, the value is kept); `:source` puts it back to `false`. Spec: SCRIPTING.md, KEYS.md T170.
- **Only real user clicks on links that would open a new buffer**: an unmodified left-click on a link whose
  target is `_blank` (`isTrusted`, button 0, no Ctrl/Shift/Alt/Meta, not `defaultPrevented`). Enter on a
  focused link is a trusted `click` too and counts. Nothing else: no `window.open`, no forms, no `web URL`
  handoff, no `:new`.
- **Middle-click is the way around it**: it always opens a new buffer, as before (C-MIDCLICK, T105).
- **Exact URL match only**: the link's absolute URL equals a buffer's current URL byte for byte. No fragment,
  query or trailing-slash normalization; a redirected buffer (http→https) does not match by its current URL alone. Accepted; amended 2026-11-26
  by "C-FOCUSEXISTING-REDIRECT" above: the URL the page was requested as matches too.
- **Several matches: the first in `:ls` order** (the lowest number). **The current buffer matches too**: then
  nothing opens, only the message. A switch is `:b N`'s (the old buffer becomes the alternate). The bar says
  `already open: [N] TITLE`.
- **How**: the renderer now catches those clicks always, whatever the setting (`newtab::BLANK_JS`, a `click`
  listener on `window` in the bubble phase, so a page's own `preventDefault` wins), cancels the popup and
  sends the new event `blank-link`; the shell decides. `open-link` stays for middle-clicks so the shell can
  tell the two apart. Links with `rel` containing `opener` or `download`, other schemes, and clicks a page
  stopped before `window` keep Chromium's popup path (PROTOCOL.md "Popups").
- **Known behaviour change with the setting off**: a `_blank` left-click now opens its buffer through the
  shell (`new-tab`) instead of Chromium's popup path. It looks the same (a new current buffer), but the new
  page gets no `Referer` header and no copy of the opener's `sessionStorage` (`window.opener` was already
  null: Chromium's implicit `noopener`). If that is not acceptable, the alternative is to inject `BLANK_JS`
  only while the setting is on, which needs the setting passed to the renderer processes; not built unless
  asked.

## 2026-11-26 Closing web: polite by default, SIGTERM forced, the shell never kills the host (C-CLOSE-POLITE)

User (issue #757): closing web with X on a page with `beforeunload`, choosing "Stay on page", closed web
anyway (the shell's 5 s deadline SIGKILLed the host).

- **Polite: the window's X button, `:q`/`:quit` (the help page's `:q` still goes back) and SIGINT
  (Ctrl-C).** The shell sends the new `close-all`; the host calls `close_browser(0)` on every page, so
  each may ask with its `beforeunload` dialog. **Follow Chrome: each page decides for itself:** a page
  whose user chooses "Stay" stays open (its buffer too) and web keeps running; the others close. The
  close is not aborted as a whole. Another X / `:q` / Ctrl-C asks the remaining pages again.
- **Forced: SIGTERM only.** The shell forwards it to the host as SIGTERM; the host (`term.rs`, a
  self-pipe handler installed after CEF's initialize) logs `SIGTERM: forced quit` and runs `quit`:
  `close_browser(1)` on every page. CEF 156 still runs `beforeunload` on a forced close and shows
  the dialog (seen in the headless nest), so **while quitting the host answers `beforeunload`
  "Leave" itself** (a JS dialog handler; beyond the issue's text, which only had a timer). A page
  still open 2 s later (a dialog that was already on screen) makes the host exit anyway with status
  0, without CEF's shutdown: with Chrome-style browsers `quit_message_loop` waits for every browser
  window, so the timer's `quit_message_loop` alone cannot end the loop.
- **The shell exits only when the host exits**, with the host's exit status (0 stays 0); the 5 s
  SIGKILL is gone. A host that exits unasked still makes the shell exit with at least 1.
- **Unchanged:** the host's forced quit when the shell closes the control socket.
- **Accepted edge case:** after a cancelled polite close, if the page left later closes by itself
  (`window.close()`), the host may exit (and web with it). No code for this.

## 2026-11-25 `web.cmd` returns a promise with the command's result (C-CMDRESULT)

User: "I think it's better to know if the command failed so we can handle this properly."

- **`web.cmd(line)` returns a Promise** settled, after the line has run, with `{ ok, message }`. It never
  rejects: a failed command is a value, so a script that ignores the promise stays silent and every existing
  script is unchanged.
- **Errors are signalled explicitly, not parsed.** Commands used to report an error only as a `web: ...`
  string put in the bar. Now every error path goes through `Keys::fail(message)`, which sets the bar text and
  marks the running command failed (`Keys.cmd_failed`); a script error (a user command that threw, a depth or
  chain stop) marks it too. `ok` is the opposite of that mark; nothing looks at a `web: ` prefix. A command
  that gives the bar no text leaves the bar as it was, and its `message` is `""`.
- **What counts as an error** (conservative: only what already showed a `web: ...` text or a script
  error): unknown command, usage, `no buffer ...`, no alternate buffer, `:b` matching none or several, a bad
  URL for `:go`/`:new`/`:openExternal`, no blocked popup, the bookmark/history commands that report `web: ...`
  (not bookmarked, not in the history, no state directory), `:let` failures, a picker with nothing to show
  (`web: the history is empty`), `:source` from a script. A command that only shows an ordinary message
  (`buffer 3 killed`, `Copied to clipboard`, `sourced 1 file(s)`) is ok.
- **A line a user command queues is its own line**: `Inner` calling `web.cmd("nope")` fails that line, not
  `Inner` (whose own result is `ok` unless it threw).
- **Order.** Lines still run after the current call returns, in the order queued. After each line its
  promise is settled in a budgeted call of its own; lines an `await` continuation queues there go to the end
  of the lines already queued, as a promise reaction would. `SCRIPT_DEPTH_MAX` (user commands nested through
  `web.cmd`) is unchanged.
- **New limit, `SCRIPT_CHAIN_MAX` = 1000**: lines run from one outermost list, continuations included. Without
  it `while (true) await web.cmd("...")` would never return to the shell (it ran no budgeted loop before).
  Past it the chain stops with an error in the bar, as the depth limit does.
- `:source` drops a promise that is still pending (a top-level `await web.cmd(...)` waiting for `ready`)
  with the old runtime, without an error.

## 2026-11-25 `web.buffers.list()` (C-SCRIPT-BUFLIST)

User: "I'd like an easy way to navigate back to [Slack] ... I think it's better to have an internal command
that ls uses then. maybe a simple list of just web.buffers?"

- **One source for the buffer list**: `buffers::infos()` gives `BufferInfo` rows in `:ls` order; the picker's
  `rows()` is built on it, and `web.buffers.list()` returns the same data, so they cannot disagree.
- **Access from a script: a snapshot, not a shared handle.** The scripts live inside `Keys`, so a script
  cannot borrow the buffers while `Keys` runs it. Before every call into JS (file, `ready`, command, mapping,
  picker mapping) `Keys::script_sync` saves the current buffer's URL and title and puts a fresh list in the
  script registry; `list()` copies it into frozen objects. Nothing is read until `ready` (`[]` before).
- **A new read API, so a new line in "Plugins are trusted code"** (SCRIPTING.md): every buffer's URL can now
  leave through `web.cmd`.
- A buffer opened with `web.cmd("new ...")` shows in `list()` from the next call; `web.cmd` lines run after
  the call returns (C-SCRIPT).

## 2026-11-25 One init.js per profile; reuse through ES modules (C-SCRIPT-MODULES)

Amends 2026-11-24 "User scripts". User: "I don't like how the 'default' profile init script gets inherited
by the rest of the profiles, it makes it very easy to accidentally add conflicting functionality. Remove
this, if the users want to reuse functionality they can create reusable ECMAScript modules". Spec:
SCRIPTING.md "Files and profiles", "Modules".

- **One init file per start.** `--profile NAME` runs `$XDG_CONFIG_HOME/web/profiles/NAME/init.js` and
  nothing else; there is no global script (`web/init.js` is no longer read).
- **`default` is only `--profile`'s default value.** No `--profile` is `--profile default`: its script is
  `profiles/default/init.js` (its own folder: "otherwise we imply that other profiles are modules to be
  imported") and its state directory `<base>/profiles/default` like any profile's (`--state-dir` is still
  exact; C-SINGLE and `--forget-site` follow). State left in the old base directory is not moved by web;
  the user moves it (STASH.md).
- **Reuse through ES modules.** `init.js` is evaluated as a module (always strict, as C-SCRIPT prepared for)
  and imports through our own resolver and loader on rquickjs's `loader` feature, not its
  FileResolver/ScriptLoader (they add a search path and extension guessing). `./`, `../` are relative to
  the importing file; a bare name to the profile's folder; an absolute path as written; no extension
  guessing. A module is named by its real path, so it runs once per runtime; `:source` makes a new
  runtime, so edited modules are read again.
- **Imports are confined to `$XDG_CONFIG_HOME/web/`** (user: "I expect a single user to have multiple
  profiles, so it makes sense to allow shared files between them"), checked on the real path (a symlink
  in a cloned repo cannot point out). `web/lib/` is the documented place for shared modules. Only regular
  `.js` files up to `SCRIPT_MODULE_MAX` (1 MiB): the read is Rust I/O, outside the script's time budget and
  memory limit. A refused import is an error naming the rule and the path; nothing is read.
- **Threat review.** A plugin is trusted code (SCRIPTING.md "Plugins are trusted code"): through
  `web.cmd` it can browse as the user (`:go` with the user's cookies, data out in a URL), run
  `:forget-site`/`:bookmarkDelete`, remap NORMAL keys and redefine other scripts' commands and mappings;
  every future read API (URL, buffers, history) becomes something a plugin can send out. The confinement
  only keeps `import` from becoming a way to read files outside the config directory (no FIFO, no
  device, nothing past `web/`).

## 2026-11-25 Deleting from a picker (C-PICKDEL, C-PICKDEL-MAP)

User: "I want to be able to delete items from a search result page ... in :ls ... delete buffers without
going in the buffer itself. From historySearch ... history items and similarly from bookmarks." Asked whether
it is built in or scriptable from init.js: both.

- **Ctrl-d** in the picker deletes the selected row of `:historySearch`, `:bookmarkSearch`, `:bufferList`/`:ls`
  (it was free: the filter line uses Ctrl-w and Ctrl-u). Other sources say there is nothing to delete.
- **A question first** (user: "ask for confirmation"): `... ? (y/n)` in the bar; `y` deletes, any other key
  cancels and does nothing else. Each source has a switch, `web.history.deleteConfirmation`,
  `web.bookmarks.deleteConfirmation`, `web.buffers.deleteConfirmation` (user: "expose ... variables with default
  true so we can switch it off"; buffers added by the user too). They are the first non-function values on
  `web`: later settings of a source go on its object. They cover the picker's Ctrl-d only; the commands never
  ask.
- **`:historyDelete [url]`**, new: removes every visit of exactly that URL. (2026-11-25, C-HISTREAL, user: "history search should only show real history rows":
  `:historySearch` no longer lists made-up site roots, so every row has lines to delete; a visited root is an
  ordinary row with its own count; `:go` keeps the roots first.) A root URL's row
  removes the root URL only, not the site's pages (`:forget-site` stays the tool for a whole site).
- After a delete the list is read again with the same filter and the selection stays in place.
- Killing the current buffer from `:ls` is allowed and does what `:bd` does.
- **Scripts**: `web.map("p", "<C-x>", fn)`, `fn(row)` on the selected row, before the built-in picker keys, so a
  profile can replace Ctrl-d or add its own keys. One Ctrl key only (letters type the filter), a function only.
  Split into its own item (C-PICKDEL-MAP) after C-PICKDEL, so each item is small.

## 2026-11-24 Script heap limit and unhandled rejections (C-SCRIPT-MEM)

- `SCRIPT_MEMORY_MAX` = 64 MiB through `Runtime::set_memory_limit`, no GC threshold change. A config script
  holds kilobytes, so 64 MiB is far above any use and far below what the 100 ms budget alone let a loop take
  (hundreds of MB). QuickJS raises a catchable `InternalError: out of memory`; the call fails like any
  script error. After a failed call the shell runs `run_gc` so cycles from the aborted allocation are freed;
  the runtime stays usable (tests: the same allocating command fails the same way twice, small ones work).
- Unhandled rejections use rquickjs's host promise rejection tracker: a rejection with no handler is
  recorded, a later `handled` notification removes it (by the promise's address), and what is left after the
  call and its pending jobs ran is reported once (bar: `FILE:LINE: unhandled rejection: msg`, log: with the
  stack). Reporting at the end of the call, not at rejection time, keeps `p = Promise.reject(); p.catch()`
  quiet. A call that also failed keeps its own error in the bar; the rejections are appended to the log text.

## 2026-11-24 Key mappings: vim's matcher, in front of the built-in keys (C-SCRIPT-MAP)

The implementer's choices (C-SCRIPT-MAP), not asked; KEYS.md T118-T124.

- **Zero cost without mappings.** The matcher (`Keys::normal_key_mapped`, keys.rs) is skipped
  entirely while no `web.map` exists, so NORMAL is the old code path; with mappings, a key that
  starts none of them goes to that path at once. Tests replay key streams (gg, gi, yy, Ctrl-^, held
  `j`, Esc Esc, the lines, a pseudo-random stream) against a shell without scripts, one with a script
  but no mapping, and one with unrelated mappings, and require the same effects, page deliveries and
  modes after every event (c_script_map_tests.rs).
- **The built-in two-key sequences keep their own machine.** KEY_SEQUENCES has no timer (a lone
  `g` waits forever); the mapping matcher sits in front of it with its own `MAP_TIMEOUT_MS`, and
  replays a pending key into the unchanged `resolve_sequence`. Unifying them would have changed
  when a lone `g` expires.
- **The second key of a half-typed built-in sequence is not mapped**, as vim reads a command's
  second key (`gg`'s) with mappings off. So mapping `gh` leaves `gg`, `gi` exactly as they were,
  not delayed; mapping `gg` itself replaces it (a mapping takes precedence over the sequence).
- **vim's resolution**: keys that start a longer mapping wait; a key that continues none (or the
  timeout) runs the longest mapping the pending keys start with, else hands the first one to the
  built-in keys and matches the rest again. A replayed key cannot reach the page any more (its
  event is gone); a scroll key it holds is let go at once if the key was released meanwhile.
- **Modifier presses are ignored by the matcher** (Shift for the `T` of `gT`), but stay in line
  and are replayed in order, so a replayed `g` followed by Shift behaves as it does today (Shift ends
  a pending `g`).
- **An OS repeat ends the wait** (a held key is not typed again). Keys after a mapping that opened
  the `:` line go into the line, in order (vim's typeahead). INPUT or focus loss drops pending keys.
- **No guard on `:`** (PR #735's question): a mapping of `:` hides the command line, as in vim.
  Esc cannot be expressed in the notation, so Esc Esc and the line's Esc cannot be lost.
- C-EDITURL's fill-in goes into `Keys::open_line_with`, the one place a line opens with text
  (`:new` with no argument and a mapping without `<CR>`).

## 2026-11-24 A mapping's `:` line runs with `<CR>`, otherwise it opens the line (vim's contract)

User: "I think we could have both 'prefill a command' and 'run a command' use the exact same interface";
"Yes, update them and drop :commandLine". `web.map`'s string rhs is one `:` line: ending in `<CR>` it runs,
without it the `:` line opens with that text typed. This changes SCRIPTING.md's earlier "a `:` string runs"
before C-SCRIPT-MAP is built, so nothing breaks. Only a trailing `<CR>` is a key: mappings never type keys,
so no `noremap`. `web.cmd` always runs. Items C-SCRIPT-MAP, C-EDITURL.

## 2026-11-24 Editing the page's URL in the `:` line

User: "I want an easy way to edit the currently visited page. I'm thinking `:go ` to automatically fill with
the currently open page, if it exists, and have it pre selected so the user can either wipe it clean, or move
the cursor to the end so they can alter the ending". Built into web, not a script: the script API has no hook
on `:` line edits and the line has no selection. Asked which commands and triggers: "Both go and new without
argument"; "do it for all commands that open URLs" (`go`, `new`, `bufferNew`, `openExternal`). Vim's Ctrl-R
`%` in the `:` line: "Let's skip this for now". A command that opens the `:` line with given text, so a
profile can map a key to it (qutebrowser's `:set-cmd-text`): "Yes, that sounds good"; later the same day
replaced by the map syntax (entry above), and dropped ("drop :commandLine"). Item C-EDITURL.

Built (implementer's choices, not asked; why): the selection is a byte range in `TextEditBuffer` (`sel`), cleared
by every edit, so no key can leave a stale range; the fill happens only where a trigger fired (typed Space, Tab,
`open_line_with`/`open_line_filled`), never by looking at the text afterwards, so Backspace/Ctrl-w/recalls
that reach `go ` cannot refill it. Up/Down while selected browse history filtered by the text before the
selection (`go `), not by the URL (which would match nothing); a match replaces the line as plain text. Tab and
unbound chords keep the selection. The `/` line has none. Colours `SELECTION_BG`/`SELECTION_FG` in config.rs.

## 2026-11-24 Command-line history: Up/Down with a prefix filter, kept on disk

User: "When I'm entering a command I should be able to use up or down keys to navigate history. If already
typed something, that should be used as filtering."; then "Keep the command history on disk, like vim". The `:`
lines run are kept in the state directory (`DIR/cmdhistory`, STASH.md once built) and survive a restart, as
vim's shada keeps its cmdline history. Item C-CMDHIST (TODO.txt).

Built (implementer's choices, not asked): `CMD_HISTORY_MAX` 1000; a line opened with text by a mapping or `:new`
filters by that text; the `/` line has none; the file is rewritten at start only when dedup or trim changed it.

## 2026-11-24 Repeating the last command: vim's @:

User: "We need a command that repeats the last performed command, similar to vims `.`"; then "Create a @: and
we can map it to . on the scripting layer if we want. Let's stay as closer to the vim contract as possible so
muscle memory takes over". So web gets vim's `@:` (and `@@`, `:@:`), repeating `:` lines only, not vim's `.`;
`.` stays unbound and a profile can map it (`web.map("n", ".", ":@:<CR>")`). It is built in because SCRIPTING.md's
API cannot see which line ran last. Items C-CMDHIST, C-REPEAT (TODO.txt).

Implementer's choices (C-REPEAT), not asked: `@@` before any `@:` is vim's E748 text; `:@:` and `:@@` lines
stay in the Up/Down list (vim records them) but `CmdHistory::last` skips them, which is vim's `:` register
keeping the repeated line, and it keeps a `:@:` line from repeating itself forever. `@@` also works after a
`:@:` (vim counts it as `@:`).

## 2026-11-24 Prompts are no longer logged

User: "I'm wondering if we should drop storing prompts altogether"; "update the relevant files so no prompt
gets updated anymore. Will remove it from main in a few days". No script or agent reads PROMPTS.md as input,
and it caused most merge conflicts (merge_pr.sh's keep-both rule). The why of a decision goes here, with the
user's words; an item's result goes in its PR body and commit message, large reports in `notes/`.
PROMPTS.md files are frozen until removed. Repo-wide: root README.md, COORDINATOR_PROMPT.md,
coordinator/AGENT_RULES.md, scratchy/AGENTS.md.

## 2026-11-24 web.cmd queues its line (C-SCRIPT)

The implementer's choice (C-SCRIPT), not asked. `web.cmd(line)` only appends the line to a queue; the shell
runs the lines a call queued right after that call into QuickJS returns. The alternative, running the command
inside `web.cmd`, needs the JS function to reach `Keys` (the shell's state) while `Keys` itself is in the middle
of calling into JS (`:Name` -> the user's function -> `web.cmd`): a `RefCell` borrowed across the call, or
`Keys` behind `Rc<RefCell<..>>` everywhere. With the queue no JS ever runs while the shell is borrowed and no
borrow is held across a call. Nothing is lost: a script cannot read anything back from a command (no getters,
SCRIPTING.md), and a queued line naming a user command runs that command's function before the next line, so
the order is the one synchronous calls would give. Lines queued at the top level of init.js wait for `ready`
(no engine host before it). Depth: user commands nested through `web.cmd` past `SCRIPT_DEPTH_MAX` stop the whole
chain with an error.

## 2026-11-24 Sticky script messages, :source, :echo (C-SCRIPT-ERRBAR, C-SOURCE, C-ECHO)

User's reports: "I don't see the error" (a throwing init.js), "do we have a :source command?", `console.log` for
testing, then `:echo`. The rest are the implementer's choices, not asked:

- **Sticky by text.** `Keys.sticky` holds the text of the last script error or `:echo`. A page load clears
  the message unless it is still exactly that text, and any key press empties `sticky`. Comparing the text
  (rather than a flag on the message) means a message that replaced it, or a flag that went stale, can never
  protect the wrong text, and none of the ~30 places that assign `message` need to know. Typed `:echo` is
  protected too (one rule; the user's next key, the Enter, comes before the command).
- **`:source` drops the whole `Scripts` value** (its Drop clears the persistent handles before the
  runtime) and loads the stored files into a new one, so there is no partial-unregister bookkeeping. The
  queue of `web.cmd` lines is drained after every call, so none is pending when `:source` runs; the files'
  held top-level lines run immediately instead of waiting for a `ready` that will not come again. If
  `ready` had not fired yet (a `:source` before the first tab) it still fires once, for the new handlers; if
  there was no runtime at start it never fires.
- **`:source` inside a script call is refused** with a counter on `run_script_lines` (every `web.cmd` line
  runs inside it), not by checking JS frames: the lines run after the call returned, so a reload would be
  safe, but the spec says error and a script that reloads itself is a loop waiting to happen.
- **Mapping timers:** keys waiting for a longer mapping stay pending and are matched against the new mappings
  (the draining loop may be running when a `:source<CR>` mapping fires); with none pending the timer is cleared.

## 2026-11-24 User scripts: init.js per profile plus a global one, QuickJS in the shell

User: "I want to be able to expose configuration similar to how nvim does it. I'm thinking using
javascript"; "Setting hotkeys to :commands, Defining :commands, Attaching functions to hotkeys. That way I
can set my default buffers for example. I think we should have one init script file per profile, and maybe a
global." Spec: SCRIPTING.md. (The global script is gone since 2026-11-25 "One init.js per profile; reuse
through ES modules".)

- **Scripts set up a profile, not the app.** This narrows "no runtime config file" (KEYS.md "Configuration",
  2026-11-20) rather than reversing it: config.rs stays where web's own defaults and knobs live, edited and
  rebuilt; a profile's commands, keys and start buffers live in its script. User: "that's basically for
  setting user profiles, not for configuring the app itself."
- **QuickJS, in web-shell, nothing in the engine host.** User: "Let's keep everything split from the engine.
  The smallest the better. Go with QuickJS." V8 (scratchy's) was too large for a config file; running the
  script in the CEF host would put it in the pages' process and tie it to one engine.
- **The smallest API, and nothing exposed until the user asks for it:** `web.cmd`, `web.command`,
  `web.map`, `web.on("ready")`. Scripts act only through `:commands`. Getters (current URL, buffers, mode)
  and `unmap` were proposed and left out until needed. What `ready` passes is decided as it gets used.
- **`--profile NAME`**: the profile's script and its own state directory. Without it, the global script only
  (superseded 2026-11-25 "One init.js per profile; reuse through ES modules": without it, `--profile default`).
  Built as C-PROFILE: the state directory is the base plus `/profiles/NAME`. `WEB_STATE_DIR` stays a
  base (so with `--profile` it gives `$WEB_STATE_DIR/profiles/NAME`; used as an exact path it would put
  every profile in one directory and one lock). Only `--state-dir` names the exact directory and beats
  the profile's. C-SINGLE and `--forget-site` already key off the state directory, so they follow with
  no code of their own. Names are `[A-Za-z0-9_-]+` because the name is a directory name (no `.`, no `/`).
- **No session restore.** User: "My goal is to replace that, and if we need it we might need to find a 'web'
  way to do it." A profile's `ready` handler opens its buffers at every start.

## 2026-11-24 Web Notifications go to the status bar, from every site by default

User: "We should plan a discreet notification feedback. Maybe a visual bell that causes the status bar to
flash? :ls should show which buffers have unread notifications"; "focus on catching notify events if
possible". Only a page's own Web Notifications count. The C-BELL spike (#727, notes/C-BELL-SPIKE.md) found
CEF exposes only the permission prompt and Chrome would send a granted notification straight to the desktop
over D-Bus; so the engine host replaces the notification API in every page and forwards each one to the
shell, and blocks Chromium's own notifications (no Chrome bubble, no desktop popup, even from a worker).
The shell flashes the bar, marks the buffer unread until it is shown, counts `[N!]` and marks `:ls`. No sound,
no desktop popups. **Default `NOTIFY_SITES = All`** (config.rs): picked by the coordinator per the spike's
recommendation, not by the user: every site sees permission "granted" without a prompt, so sites that
would ask first notify right away (the cost is noise in the bar, nothing leaves the machine), and "granted"
everywhere makes web a little more recognisable to sites that look. The user may switch to a list of
hostnames (`NotifySites::Only(&["chat.example.com"])`; every other site then sees "denied"). Notifications
sent from inside a worker (push, a service worker's own) are lost silently. Item C-BELL.

## 2026-11-24 :new opens a buffer

User: "we don't have an easy way to create a new buffer, add an action item for a :new command"; on the proposed
behaviour: "exactly what I wanted". `:new [url]` (alias `:bufferNew`) opens a buffer and makes it current, the
argument as `:go`'s; with no argument an about:blank buffer and the `:go ` prompt. The previous buffer becomes
the alternate (Ctrl-^). Item C-NEWBUF.

## 2026-11-23 H.264: build CEF ourselves; :openExternal as the escape hatch

User, after C-WEBEX (#718) showed Webex needs WebRTC H.264, which the prebuilt CEF lacks: "Let's do both.
Create :openExternal but let's also build chromium". C-EXTERNAL hands a page to another browser.
C-H264-BUILD builds CEF 156 with proprietary codecs (H.264/AAC, OpenH264) on this machine: the build may use
up to 20G of RAM (an exception to the 6G cap, for this build only; the machine has 31G), runs under nice/ionice
outside the repo (~/.cache/web-cef-build), and its output is never committed or published (codec patents).
C-H264 is unparked into C-H264-BUILD; DRM (C-DRM) stays parked.

## 2026-11-23 Modifiers reach the page on clicks in NORMAL

User: "we should pass the modifier keys on normal mod unless that breaks functionality". In NORMAL a
click carries the held modifiers (Ctrl/Shift-click behave as in INPUT: the host's no-new-tab rules apply),
while NORMAL's own bindings stay unaffected (the scroll keys it sends, e.g. G as End, must not arrive as
Shift+End). Item C-NORMALMODS.

## 2026-11-23 The picker's, bookmarks' and buffers' defaults are kept

User, on the choices agents made without asking ("they all look good right now"): the picker (C-PICK, KEYS.md
T58-T68: `>` prompt and `PICK history m/n` header, no wrap at the ends, Enter with no match does nothing, a
click on the list closes it and is kept from the page, words after the command prefill the filter, best
match on top) and bookmarks (C-BOOKMARKS: search newest first, delete by exact URL, a hand-edited file is
normalized on rewrite) stay as built; their *(proposed)* marks go.

## 2026-11-23 A picker, bookmarks, buffers as tabs, popups like Chrome

User, in a planning session (no code; PROMPTS.md "Picker, bookmarks, buffers, popups"). Overrides
"No tabs yet" and, for script popups and target=_blank, C-NOTAB.

- **Picker.** A list the shell draws over the page, above the status bar (the page is not resized),
  with fuzzy filtering, the same keys in every list (move, Enter runs the row, Esc closes) and a mode
  of its own. Each source gives rows of a label plus an action. It is for dedicated commands
  (`:historySearch`, `:bookmarkSearch`, `:bufferList`, `:popupSearch`; context-menu actions are a
  possible later source, C-CMDS stays parked). It never shows Tab candidates: "No menu" (KEYS.md,
  2026-11-20) was about `:go`'s completion and stays as it is.
- **Bookmarks.** A plain file `DIR/bookmarks` (`url<TAB>title`, editable by hand); `:bookmarkAdd`,
  `:bookmarkDelete [url]`, `:bookmarkSearch`; bookmarks also feed `:go`'s ghost, above history.
- **Buffers are the tabs.** One engine host holds several pages (the profile lock allows one host).
  Commands are camelCase with vim aliases (user: "go with b"): `:bufferList`/`:ls`,
  `:buffer`/`:b` (N or a partial name), `:bufferNext`/`:bn`, `:bufferPrev`/`:bp`, `:bufferKill`/`:bd`,
  `:bufferDescribe [text]` (no alias); Ctrl-^ for the previous buffer. Ghost completion offers the long
  names only. A buffer is searched and listed by title, hostname and an optional description the user
  sets ("keep it minimal": no page content, no snippets). The description stays across navigation and
  goes with the buffer. Killing the last buffer leaves an empty `about:blank` buffer ("The user doesn't
  know if the buffer they just killed is the last one"). Ctrl/middle-click on links still open nothing
  (middle-click amended 2026-11-24, see "Middle-click on a link opens a new buffer").
  Not now: searching page contents (maybe a separate feature), restoring buffers after a restart.
- **Popups follow Chrome** (user: "go with your suggestions"): a `window.open` from a user gesture opens
  a new buffer with `window.opener` kept (OAuth/payment logins work); one without a gesture is blocked,
  the status bar says `popup blocked: <url>` (or that it had no URL), and `:popupOpen` (latest) /
  `:popupSearch` (the buffer's list, cleared when it navigates away) open it in a new buffer. A
  `target=_blank` link opens a new buffer. Not now: a per-site "always allow popups" list.

## 2026-11-24 Middle-click on a link opens a new buffer (amends "Ctrl/middle-click on links still open nothing")

User (C-MIDCLICK): "Middle click on links should basically do a :new". A middle-click on a link now opens its URL in a
new buffer that becomes current, exactly as `:new <url>` does (T103: the old buffer is the alternate, Ctrl-^ returns),
in NORMAL and in INPUT (the new buffer starts in NORMAL). This reverses C-NOTAB for the middle button only: Ctrl-click,
Shift-click and Ctrl+Shift-click still open nothing, and middle-click on anything that is not a link is unchanged (no
paste, see below; no autoscroll). How the host tells a middle-click from Ctrl+click: Chromium gives both the same
disposition (NEW_BACKGROUND_TAB, no modifiers in `OnOpenURLFromTab`), so the host cannot there; the renderer's DOM can:
a middle-click is an `auxclick` with `button` 1 (Ctrl+click is a `click` with `button` 0 and `ctrlKey`). A script run
in every frame (`newtab::MIDDLE_JS`) cancels an unmodified middle-click on `a[href]`/`area[href]`, sends the link's
absolute URL to the host through a V8 function the script closes over (the page never sees it), and the host tells the
shell with `open-link`. The shell opens it like `:new` (no `window.opener`, no popup blocking: not a popup).

## 2026-11-23 No middle-click paste in the page; GTK stays

User: "didn't know I hade gtk dependencies in the first place. it's ok tho, Drop middle-click paste in the
page and keep GTK as it is." Chromium (libcef) loads GTK at run time for desktop integration: theme, dark
mode via prefers-color-scheme, and GTK settings such as gtk-enable-primary-paste, which GNOME ships off and
which gates middle-click paste into pages. web keeps GTK; there is no middle-click paste into the page. #689 closed.
(Corrected 2026-11-23 after #695: the coordinator first wrote that middle-click paste works in the `:` and
`/` lines; it was never built. Those lines paste the clipboard only: Ctrl-v, Ctrl-Shift-v, Shift-Insert.)

## 2026-11-23 Enter in NORMAL goes to the page; caret browsing as an experiment

User: "do this. Enter in NORMAL forwards to the page. Expose caret browsing from a command mode command,
so I can experiment with it but don't change the defaults." Enter in NORMAL is delivered to the page
(after a find, Chromium focuses the matched link, so Enter follows it). `:set caret` / `:set nocaret`
toggle Chromium's caret browsing; off by default. The rest of caret mode stays parked.

## 2026-11-23 Search places the pointer, Enter clicks

User: "On Search Mode n/N moves the cursor at the beginning of the match. Enter sends a click event to
it. I think that's better defined behavior." `/`+Enter, `n`, `N` move the mouse pointer to the start of
the current match; Enter in NORMAL clicks at the pointer, wherever it is (coordinator's reading: always,
not only after a search). Replaces KEYS.md S7/T18-T20 ("click the match", "Match spans N links"). Pairs
with C-CMDS, whose commands act on the element under the pointer. Item C5-ENTER.

## 2026-11-23 No context menu; its actions become commands

User: "maybe we could disable it completely, and expose it via the command mode" and, after the
coordinator's proposal (commands act on the element under the pointer now, picked by link hints later;
`:yank` needs the clipboard of C6), "yes, go ahead". C-NOMENU removes the menu (right-click still reaches
the page); C-CMDS adds `:save`, `:save link`, `:save image`, `:yank link`, `:yank image`. DRM (C-DRM) and
H.264 (C-H264) are parked (user, same day).

## 2026-11-24 Only a focus the user caused switches to INPUT

User (C-AUTOFOCUS): "moving to INPUT mode when input elements get focus confuses me [...] I'm thinking of not changing modes on autofocus, but should make sure that once I hit i, the autofocused input gets focus"; "include gi too". Amends "Focusing an editable switches to INPUT" (2026-11-23,
below): an `editable-focused true` switches to INPUT only when the user caused it, i.e. within
`CLICK_FOCUS_MS` of a mouse press, of a key the shell gave the page in NORMAL (Enter; the find +
Enter timed from the real press) or of `gi` (a click on "Reply" that makes the page focus a
textarea still counts). The page's own focus (`autofocus`, a late `el.focus()`) leaves the mode
alone, and the shell has the host blur the field and remember it ("parked"), so NORMAL's scroll
keys scroll the page; `i` focuses the parked field again. At most one blur per buffer per
`CLICK_FOCUS_MS` (pages that refocus on blur keep the focus). `gi` (as Vimium) focuses the last
text field, else the first visible one, and enters INPUT. Known effect: the blur runs the page's
blur handlers (a validation message, a closing suggestion list). KEYS.md T29, T32, T13, T6, T111.

## 2026-11-23 Focusing an editable switches to INPUT

**Amended 2026-11-24** ("Only a focus the user caused switches to INPUT", above): the page's own
focus no longer switches.

User: "when input elements gain focus, we should automatically switch to Input Mode. No auto-return on
blur since the exit of the form context isn't that clear." Any `editable-focused true` from the host
switches to INPUT, whether a click, the keyboard or page script (`autofocus`, `el.focus()`) caused it.
This reverses KEYS.md T32 (a page could not pull the user out of NORMAL). Blur never changes the mode.
Known effect: pages that autofocus a field (search engines, login pages) open in INPUT. Done in C3-RCLICK.

## 2026-11-23 Popups return to the previous mode

User, on C3's rule that a click opening a popup (context menu, select dropdown) switches to INPUT: "I
think it makes sense as UX, but what do you think about switching back to the previous mode after
clicking an option?" Agreed: INPUT while the popup is open, back to the previous mode when it closes,
unless the click focused an editable field. Item C3-RCLICK.

## 2026-11-23 forget-site uses DevTools Storage.clearDataForOrigin

User (on #670): "no need for hacks, use Storage.clearDataForOrigin. Make sure that's done in an
extendable manner for other engines." An in-process DevTools method call from the engine host
(CefBrowserHost::ExecuteDevToolsMethod) is allowed for forget-site; this is not the automation
protocol that "No browser extension, no automation protocol" excludes (no remote-debugging port,
no CDP socket, nothing outside the host). The forget contract sits behind an engine-neutral
interface so E1 (WebKit) implements it with its own API.

## 2026-11-23 No tabs yet

User: "Do not bother with tabs for now. Do not expose tab functionality yet. Maybe we'll mimic
them using the buffers interface, but let's not commit now." C4 is deferred (not scheduled, no
design session). The engine host shows one page: protocol v0 has no `new-tab`/`close-tab`, and
`window.open`/target=_blank load in the same page (coordinator's reading, sent to C1; known loss:
popups that rely on `window.opener`, e.g. OAuth login popups). Events keep their `<id>` field so
a later buffers or tabs design needs no protocol change.

## 2026-11-23 The proxy is optional

User: "do not have it require a proxy. It should support it but a missing --proxy flag
should default to standard connectivity". Without `--proxy`, the engine uses its normal
networking; a missing flag is never an error or a warning.

## 2026-11-23 No enforcement in web

User: "don't enforce that. It's something we can do separately". web passes a proxy to
the engine (`--proxy <host:port>`, handed to the engine host; without it the engine
connects directly) and does nothing else about the network. Keeping a program off the
network except through a proxy (an empty network namespace plus a forwarder to the
proxy's socket, as the spike's W2 did) is a separate, engine-neutral wrapper that can be
put around web, or any other program, from outside. Overrides "Enforcement: an empty
network namespace" below and the namespace parts of "Architecture C". Paths that ignore
a proxy setting (WebRTC's UDP, for one) are not web's concern.

## 2026-11-23 Blink first

User: "let's begin with Blink so we can have something working as soon as possible. We
can switch to a more lightweight engine later". web-host-cef is the only engine host
through C8; web-host-webkit (or another lighter engine) is E1, after C8. The control
protocol stays engine-neutral so E1 needs no shell change.

## 2026-11-23 The engine owns localStorage; scratchy's stash store is dropped

User: "drop it and record the decision. Have the engine deal with localStorage unless we
have a use case for centralizing it". localStorage, sessionStorage and IndexedDB live in
each engine host's profile (STASH.md "State directory"), in the engine's own format.
scratchy's one-file-per-key store, its locks and generation counter are not carried
over. Reopen only with a concrete reason to keep page storage outside the engine (e.g.
sharing it between engines or editing it by hand).

## 2026-11-23 Architecture C: compositor shell, engine hosts, a web proxy

From the discussion after the spike (PROMPTS.md "Architecture C"). Overrides "Request
path" and "Engine" below; "Cookies: Chromium owns the jar" and "Enforcement: an empty
network namespace" still hold.

- **Network: a web proxy.** web needs one thing from the network side: an HTTP forward
  proxy (plain requests and `CONNECT`) reachable on a Unix socket. Which proxy that is
  is the user's choice, not web's. The engine host gets a proxy setting pointing at a
  forwarder inside its empty network namespace, which reaches that socket. Nothing
  engine-specific sits on the network path, so the namespace plus a proxy setting is
  the whole contract. W3's resource handler and its wire client leave the browser.
- **Shell: web-shell is a Wayland compositor** (smithay, a nested compositor shown as
  one window on the user's desktop). It draws the status bar and command line, owns all
  keys and pointer input, and forwards to the page what the mode allows.
- **Engine: separate engine-host programs, own protocol.** Each engine gets a small host
  with no browser UI (one window per tab, the page only) and a control socket speaking
  our protocol. `web-host-cef` first (from the spike), `web-host-webkit` (WebKitGTK)
  second. The engine is picked at start (`--engine`); no switching while running, and
  logins do not carry between engines (each keeps its own cookies).
- **Rejected:** a browser extension as the shell (the user does not want to rely on the
  extension API); driving a stock browser over CDP or WebDriver BiDi (automation flags,
  sites that detect it); off-screen rendering into the shell (rebuilds popups, IME,
  drag and drop that the engine does natively as a Wayland client).
- **X11:** web-shell runs on an X11 desktop too: its winit backend opens an X11 window
  there, and the engine hosts keep speaking Wayland to the shell. C2's check covers both.
- **Carry over from scratchy everything that can be carried** (user): KEYS.md and the
  STASH.md now live in web/ (KEYS.md, STASH.md: state directory, history, forget-site,
  cookies); the shell code to port is
  listed in TODO.txt "From scratchy".

## 2026-11-23 The spike is a success; testing stops

The user, after W1-W5 (W4 prepared, W5 ready) and with W6 in progress: "I consider that a
success. you can stop testing now". W6 was stopped; its partial work (switch lists, start of a
WebSocket proxy) is on branch `web/w6-bypass`. W4's real-site login and W7's day of browsing
are not run. Whether this is the go for the "After a go" items is asked separately.

## 2026-11-23 Start `web`, a CEF-based browser; pause scratchy

From the engine discussion of 2026-11-23 (PROMPTS.md "Engine discussion").

- **Priority: feature completeness first, wire compatibility second.** The goal is a
  daily-driver browser; the browser shell is the full-time job, not the engine.
- **Engine: Chromium (Blink) through CEF, prebuilt binaries.** Servo (~62 % overall WPT
  score, Feb 2026) and Ladybird (pre-release) are not daily-driver complete. WebKitGTK
  has no hook to answer http(s) requests from our own code. Firefox has no desktop
  embedding API.
- **Bindings: the `cef` Rust crate** (Tauri team), so the shell stays in Rust.
- **Request path:** a `CefResourceRequestHandler` / `GetResourceHandler` answers every
  http(s) request by sending it to wire's Unix socket and streaming the response back.
- **Cookies: Chromium owns the jar** (proposed; to confirm in the spike). `document.cookie`
  reads Chromium's jar, so a wire-owned jar breaks sites. wire passes `Set-Cookie`
  through and the browser sends no `X-Wire-Jar`. wire's jar stays for other clients.
- **wire keeps:** TLS, DNS, redirects, decompression, request policy, cache, logging,
  and blocking (ads/trackers).
- **Chromium enforces** CORS, mixed content and Secure contexts; scratchy's versions
  (`js_net.rs`) are not ported.
- **Enforcement: an empty network namespace.** The whole CEF process tree runs with no
  network interfaces (`unshare -rn` or bubblewrap `--unshare-net`); wire runs outside it.
  A Unix socket addressed by file path still works across network namespaces, so wire
  is the only way out and any path the handler misses fails instead of leaking. This
  replaces scratchy's seccomp filter, which cannot cover CEF's child processes.
- **Known losses accepted up front:** WebRTC (own UDP sockets) cannot go through wire.
  WebSockets, QUIC, DNS prefetch, preconnect and service-worker fetches are checked in
  the spike (TODO.txt step 6).
- **First step is docs only;** no CEF download or code until the spike is started.
