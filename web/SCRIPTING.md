# Scripting: init.js

A user's own setup (their commands, their keys, the buffers they start with) lives in a
JavaScript file read at start, in the spirit of nvim's `init.lua`. It sets up a *profile*,
not the app: what web is and how it behaves by default stays in `shell/src/config.rs`
(KEYS.md "Configuration"; DECISIONS.md 2026-11-24 "User scripts").

Rule for this file: **nothing is exposed to scripts unless the user asked for it.** Every
addition to the API below is a user decision, recorded here with its date.

## Engine

QuickJS (the `rquickjs` crate), embedded in web-shell. Nothing runs in the engine host: the
shell owns input and the command line, and a script never shares a process with a page.
The runtime has no QuickJS `std`/`os` modules: no network, no timers, no file access beyond
the module imports below. The profile's `init.js` is evaluated as an ES module, so it is always
strict (C-SCRIPT-MODULES, 2026-11-25; C-SCRIPT ran it as a strict classic script so that this could
come without breaking a script) and can `import` other modules (below, "Modules").

## Files and profiles

| file | runs |
|---|---|
| `$XDG_CONFIG_HOME/web/profiles/NAME/init.js` (default `~/.config/web/profiles/NAME/init.js`) | with `--profile NAME`; nothing else runs |
| `$XDG_CONFIG_HOME/web/profiles/default/init.js` | without `--profile` (it is `--profile default`) |

One file per start (user, 2026-11-25: "I don't like how the 'default' profile init script gets inherited by
the rest of the profiles, it makes it very easy to accidentally add conflicting functionality"). There is no
global script: code that several profiles share goes in a module they both import (below).

- `web --profile NAME` picks the profile. `NAME` is `[A-Za-z0-9_-]+`. Its state directory
  defaults to `$XDG_STATE_HOME/web/profiles/NAME/` (its own cookies, history, engine
  profile); `--state-dir` still overrides it.
- `default` is special only as the value of `--profile` when it is not given: `web` and
  `web --profile default` are the same thing, with `profiles/default/init.js` and the state directory
  `$XDG_STATE_HOME/web/profiles/default/` like any profile (STASH.md). `$XDG_CONFIG_HOME/web/init.js` is
  not read any more.
- A missing file is not an error. A profile with no `init.js` runs nothing.
- `web --profile NAME URL` hands the URL to the running instance of *that* profile
  (C-SINGLE finds it through the state directory, so this follows from the line above).
- No session restore (user, 2026-11-24): a profile's `ready` handler opens the buffers it
  wants every time; nothing remembers what was open last time.

## Modules (C-SCRIPT-MODULES, 2026-11-25, user)

`init.js` imports other files with `import` (static) or `import()` (dynamic, a promise), as in any ES
module. Shared code goes in `$XDG_CONFIG_HOME/web/lib/`, the documented place for modules that
several profiles use:

```
~/.config/web/
  lib/common.js             export function startBuffers(urls) { ... }
  profiles/default/init.js  import { startBuffers } from "../../lib/common.js";
  profiles/work/init.js     import { startBuffers } from "../../lib/common.js";
```

```js
// ~/.config/web/lib/common.js
import { tag } from "./tag.js";          // a lib module imports its neighbours with ./
export function startBuffers(urls) {
  web.on("ready", () => urls.forEach(u => web.cmd("new " + u)));
}
```

- **Specifiers.** `./x.js` and `../x.js` are relative to the importing file. A bare name
  (`"common.js"`, `"sub/x.js"`) is relative to the profile's folder (`profiles/NAME/`), also inside a
  `lib/` module, which is why `lib/` modules import each other with `./`. An absolute path is used as
  written. The name is taken as it is: no extension is added, no search path, no `index.js`.
- **Confined to `$XDG_CONFIG_HOME/web/`** (user: "I expect a single user to have multiple profiles, so it
  makes sense to allow shared files between them"). The check is made on the real path, with symlinks
  resolved, so a symlink inside (in a cloned config repo, say) cannot point out of it.
- **Regular `.js` files only**, of at most `SCRIPT_MODULE_MAX` (config.rs, 1 MiB): no FIFO, device,
  folder, `.json` or other file. Reading a file is Rust I/O, outside the time budget and the memory
  limit, hence the size cap (it holds for `init.js` too).
- A refused import names the rule and the path, and nothing is read:
  `init.js: import "../../../x.js": outside /home/u/.config/web/: /home/u/x.js`. Other refusals:
  `no such file`, `not a .js file`, `not a regular file`, `larger than SCRIPT_MODULE_MAX`.
- A module is known by its real path, so a file imported twice (by two modules, or as `./x.js` and by
  its absolute path) runs once. `:source` makes a new runtime, so edited modules are read again.
- The whole import graph is loaded and evaluated in the one call that runs `init.js`: it all counts
  against `SCRIPT_BUDGET_MS`, and module code against `SCRIPT_MEMORY_MAX`.
- A top-level `await` works. One still waiting when nothing is left to run (it waits for a callback, as
  `await new Promise(r => web.on("ready", r))`) does not fail the start; the rest of the module runs
  when that callback settles it, and an error from it is reported as the file's error.
- **Module scope.** A module is not a classic script: top-level `var` and `function` declarations do
  not become globals (`globalThis.x` is undefined) and top-level `this` is `undefined`. This only
  matters for a script written as a classic one; use `export`/`import`, or `globalThis.x = ...` on
  purpose.

### Plugins are trusted code (C-SCRIPT-MODULES, 2026-11-25)

A module you import runs with everything `init.js` can do; import only code you would put in your
`init.js` yourself. Through `web.cmd` a script can browse as you (`:go` a URL with your cookies, and
put data in that URL), run `:forget-site` and `:bookmarkDelete`, remap NORMAL keys and redefine other
scripts' commands and mappings. Every read API added later (the current URL, the buffers, the history)
becomes something a plugin can send out the same way. (2026-11-25, C-SCRIPT-BUFLIST: `web.buffers.list()` is the
first read API; the URL, title and description of every open buffer can now leave through `web.cmd`, for
instance in `:go` with the data in the URL.) The confinement to `$XDG_CONFIG_HOME/web/`
stops a script from reading arbitrary files through `import`; it does not make a module safe.

## API (2026-11-24, user)

Everything lives on one global object, `web`.

### `web.cmd(line)`

Runs `line` as if typed after `:` and Enter (a leading `:` is dropped). It is the only way
a script acts on the browser: whatever a `:command` can do, a script can do, and nothing
more. Errors show in the status bar as the command's would.

**Return value (2026-11-25, user; C-CMDRESULT).** It returns a Promise that settles, after the line has run,
with `{ ok, message }`: `ok` is `false` when the command reported an error (an unknown command, its usage,
`:b` matching no buffer or several, a URL it refused, a user command that threw, ...), else `true`;
`message` is the text the command gave the bar (`""` when it gave none). The promise never rejects: a
failed command is a result, not an exception, so a script that ignores it behaves as before and no
unhandled rejection comes from it.

```js
// gs: go to the Slack buffer; open one when there is none.
web.map("n", "gs", async () => {
  const r = await web.cmd("b app.slack.com");
  if (!r.ok) web.cmd("new https://app.slack.com");
});
```

The line still runs after the current call returns, in order with the others. Once it has run, the promise
is settled in a call of its own (same time budget and error report as any call); an `await` that continues
there queues its own lines after the ones already queued. A top-level `await web.cmd(...)` in a file waits
for `ready` like the line itself. `:source` drops a promise still pending, silently. At most
`SCRIPT_CHAIN_MAX` (config.rs, 1000) lines run from one call, awaited continuations included, so an endless
`await` loop ends with an error in the bar. `message` is read as the bar had it when the line finished; the
errors a nested line (one a user command queued) had are that line's, not the command's.

(C-SCRIPT, 2026-11-24, forced by the implementation:) the line runs right after the current call
into the script returns, in order with the others it queued; a line naming a user command runs
that one's function there, so the order is as if each line had run at once. At the top level of
`init.js` there is no engine host yet: those lines run when `ready` fires, before its handlers.

### `web.command(Name, fn)`

Defines the command `:Name`. `Name` must start with an uppercase letter (as nvim's user
commands), so a user command never shadows a built-in one; defining the same name again
replaces it. `:Name a b` calls `fn(["a", "b"])` (the arguments split as built-in commands
split theirs). User commands are offered by the command-line completion with the built-ins.

### `web.map(mode, keys, rhs)`

Maps a key sequence in a mode. `mode`: `"n"` (NORMAL), or `"p"` (the picker, below). `rhs`: a string
starting with `:`, or a function (called with no arguments).

A string `rhs` is one `:` line, as in a vim mapping (user, 2026-11-24: "both prefill a
command and run a command use the exact same interface"):

- ending in `<CR>` (also `<Enter>`, `<Return>`; any case): the line runs, as `web.cmd` would.
  `web.map("n", "gh", ":go https://example.com<CR>")`.
- without it: the `:` line opens with the text typed, cursor at the end, and waits for the
  user, so it reaches the line's own behaviour (completion; the URL fill-in of `:go ` and the
  other URL commands, TODO.txt C-EDITURL). `web.map("n", "ge", ":go ")`.

Nothing else is a key in `rhs`: `<CR>` only at the very end, `<lt>` is a literal `<`, and
any other `<...>` or a `<CR>` before the end is an error when the mapping is made (more than
one command: use a function). Mappings never type keys, so one mapping cannot trigger another
and vim's `noremap` is not needed. `web.cmd` always runs its line.
A user mapping takes precedence over a built-in binding or sequence (config.rs `BINDINGS`,
`KEY_SEQUENCES`) for the same keys; mapping the same keys again replaces the mapping.

`keys` notation (vim's): a character stands for itself (`"gh"`, `"G"`); `<C-x>` is Ctrl
with `x`; `<lt>` is `<`. Any length. While the keys typed so far are a prefix of a longer
mapping, the shell waits up to `MAP_TIMEOUT_MS` (config.rs, as vim's `timeoutlen`) for the
next key; on a timeout or a key that matches nothing, the pending keys are handled as
they would have been without the mapping. Other keys (`<A-...>`, `<Space>`, arrows, ...)
come when a mapping needs them.

#### Mode `"p"`: keys in a picker (2026-11-25, user; C-PICKDEL-MAP)

`web.map("p", "<C-x>", fn)` runs `fn(row)` on the picker's selected row (KEYS.md T167-T168). In a picker the
letters type the filter, so a `"p"` mapping is exactly one Ctrl key, and its `rhs` is a function (a `:` line
could not name the row); anything else is an error when the mapping is made. It comes before the picker's
built-in keys, so mapping `<C-d>` replaces the built-in delete and its question (below). `row` is a plain
object: `source` (`"history"`, `"bookmarks"`, `"buffers"`, `"popups"`, `"notifications"`), `label`, `detail`,
and the one of `url` (history, bookmarks), `buffer` (buffers), `popup`, `notice` that applies. After `fn` and
its `web.cmd` lines have run, the list is read again (a deleted row disappears). No row selected: `fn` is not
called. A message the function's commands set (or its error) shows beside the filter line like the
picker's own (KEYS.md T169).

```js
// Ctrl-x deletes history rows without asking, and only history rows.
web.map("p", "<C-x>", row => {
  if (row.source === "history") web.cmd("historyDelete " + row.url);
});
```

### `web.history`, `web.bookmarks`, `web.buffers` (2026-11-25, user; C-PICKDEL)

Each has a setting, `deleteConfirmation`, `true` by default: whether the picker's Ctrl-d asks
`... ? (y/n)` before deleting a row of that source (KEYS.md T158-T166). Set it to `false` to delete at once:

```js
web.bookmarks.deleteConfirmation = false;
```

Only `true` or `false`; any other value is a script error and the setting keeps its value. `:source` puts all
three back to `true` before it runs the files again. The setting covers the picker's built-in Ctrl-d only: the
commands (`:historyDelete`, `:bookmarkDelete`, `:bufferKill`) never ask, typed or through `web.cmd`, and a
`"p"` mapping does what its function does. These objects are where later settings of a source go, never `web`
itself (user, 2026-11-25).

### `web.buffers.focusExisting` (2026-11-26, user; C-FOCUSEXISTING)

`false` by default. Set it to `true` and a plain left-click on a `target=_blank` link (or Enter on such a
link) whose absolute URL is, byte for byte, the current URL of an open buffer shows that buffer instead of
opening a new one (KEYS.md T170): the first such buffer in `:ls` order, switched to as `:b N` does (the one
left becomes the alternate, Ctrl-^ returns), and the bar says `already open: [N] TITLE`. When that buffer is
the one on screen nothing switches; only the message shows. No other match: a new buffer, as with the
setting off.

```js
web.buffers.focusExisting = true;
```

- Exact URLs only: a different `#fragment`, query or trailing slash is another page, and a buffer that was
  redirected (`http:` to `https:`) does not match the link's URL.
- Only those clicks: a middle-click (KEYS.md T105) always opens a new buffer, which is the way around it.
  `window.open`, forms, `:new`, `:go` and a `web URL` from outside never look for an open buffer.
- Only `true` or `false`; any other value is a script error and the setting keeps its value. `:source` puts
  it back to `false` before it runs the files again.

### `web.buffers.list()` (2026-11-25, user; C-SCRIPT-BUFLIST)

The open buffers, in `:ls` order (by number), as a new array of plain objects:

| field | |
|---|---|
| `number` | the shell's number (`:b N`) |
| `url`, `title`, `description` | strings (`description` is `:bufferDescribe`'s, `""` when none) |
| `current` | the buffer shown |
| `alternate` | the buffer Ctrl-^ goes to |
| `loading`, `crashed` | booleans |
| `unread` | how many notifications came while it was not shown |

It is a function, not a property: it reads the buffers at the moment of the call, and what it returned is
a snapshot that does not change afterwards. The objects are frozen, so a script changes a buffer through
`web.cmd` only. `:ls` is built from the same list, so the two never disagree. Before `ready` (the file's
top level) there are no buffers yet and it returns `[]`; `ready` handlers, commands and mappings see them.

`web.cmd` runs its lines after the current call returns, so a buffer a call opens with `web.cmd("new ...")`
is not in a `list()` made later in that same call; the next call sees it.

```js
// gs: go to the Slack buffer, or open it.
web.map("n", "gs", () => {
  const b = web.buffers.list().find(b => b.url.startsWith("https://app.slack.com/"));
  web.cmd(b ? "b " + b.number : "new https://app.slack.com");
});
```

### Commands for scripts: `:echo` and `:source`

Not on `web`: they are `:` commands (user, 2026-11-24), so a script reaches them through `web.cmd`.

`:echo TEXT` shows TEXT in the status bar (the rest of the line, verbatim) and writes
`[web] echo: TEXT` to the log. There is no `console` object; this is how a script shows a value
while the user is testing an `init.js`:

```js
web.command("Where", () => web.cmd("echo profile " + "work" + ", " + 2 + " buffers"));
web.on("ready", () => web.cmd("echo init.js ran"));
```

`:source` runs the script again, as at start but clean (user, 2026-11-24): every user command,
mapping and `ready` handler is dropped with the old QuickJS runtime, a new one runs the profile's
`init.js` again (the profile is the one the shell started with; C-SCRIPT-MODULES: that file only, and
every module it imports is read again) and the bar says `sourced 1 file(s)` (`sourced 0 file(s)` when
the profile has no `init.js`). `ready` handlers are registered again but not fired, so the start
buffers are not opened a second time; `web.cmd` lines at the top level of a file, which at start wait
for `ready`, run at once (the host is connected). Errors show as at start and what ran before the
error stays. `:source` is a command for the user: `web.cmd("source")` (a user command, a mapping
function, `ready`) is an error, because a reload from inside a call would drop the code that is
running; `web.map("n", "gs", ":source<CR>")` works, as the line is the user's. `:source FILE` does not
exist yet.

### `web.on("ready", fn)`

Calls `fn` once per start of the shell, when the engine host is connected and the first
buffer exists (the URL argument's, as today). This is where default buffers are opened
(`web.cmd("new https://...")`). What the event passes to `fn` is decided as it gets used
(user, 2026-11-24); today it passes nothing. A URL later handed over by `web URL`
(C-SINGLE) does not fire it again.

## Errors and limits

- A script that fails (syntax error, uncaught exception) stops at that point; what it set
  up before the error stays. web starts anyway. The status bar shows
  `init.js:LINE: message` (every profile's `init.js` is called `init.js` there); the full
  error goes to the log.
- An error inside an imported module shows that module's real path and line
  (`/home/u/.config/web/lib/common.js:3: message`). An import that fails (no such file, a refused
  specifier, a syntax error in the module) is the init file's error with the module's path in the
  message (`init.js: import "./x.js": no such file: /home/u/.config/web/profiles/work/x.js`); nothing
  of `init.js` runs then, as with a syntax error in it.
- An error in a mapped function, a user command or the `ready` handler shows in the
  status bar the same way.
- **Sticky messages** (C-SCRIPT-ERRBAR): a script's error, and the text of an `:echo`, stay in the bar
  until the user's next key. Loading a page clears the bar's message as always, except these (without
  this an `init.js` error at start was gone as soon as the first page loaded). After the next key such a
  message is an ordinary one again.
- Every call into the script (the file itself, a mapping, a command, `ready`) has a time
  budget, `SCRIPT_BUDGET_MS` (config.rs); QuickJS's interrupt handler ends a call that
  overruns it with an error, so a stray loop cannot freeze the shell.
- The script heap is limited to `SCRIPT_MEMORY_MAX` (config.rs, 64 MiB). A script that allocates past it
  gets QuickJS's `out of memory` error, shown like any other (`init.js:LINE: out of memory`); web keeps
  running, the garbage is collected, and later calls (commands, mappings, `ready`) work as before. A script
  can catch the error itself with `try`/`catch`.
- A promise that is rejected and has no handler by the time the call (and its promise jobs) finished is
  reported once, in the bar and the log: `init.js:LINE: unhandled rejection: message`. This covers `async`
  functions in commands, mappings and `ready`, and rejected promises in the file itself. A rejection that
  gets a `.catch` or `try`/`await` in time is not reported.
- `web.cmd` calls nested through user commands have a depth limit (an error, not a hang).
