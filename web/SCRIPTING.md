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
The runtime has no QuickJS `std`/`os` modules and no module loader: no files, no network,
no timers, no `import`. Each file is evaluated as one classic script, in strict mode (user, 2026-11-24: so that
user modules, ES modules and always strict, can come later without breaking a script); the
global and the profile script share one context, so the profile script sees what the global
one defined.

## Files and profiles

| file | runs |
|---|---|
| `$XDG_CONFIG_HOME/web/init.js` (default `~/.config/web/init.js`) | always, first |
| `$XDG_CONFIG_HOME/web/profiles/NAME/init.js` | with `--profile NAME`, after the global one |

- `web --profile NAME` picks the profile. `NAME` is `[A-Za-z0-9_-]+`. Its state directory
  defaults to `$XDG_STATE_HOME/web/profiles/NAME/` (its own cookies, history, engine
  profile); `--state-dir` still overrides it. Without `--profile`: the global script only,
  and the state directory as today (STASH.md).
- A missing file is not an error. A profile with no `init.js` runs on the global script.
- `web --profile NAME URL` hands the URL to the running instance of *that* profile
  (C-SINGLE finds it through the state directory, so this follows from the line above).
- No session restore (user, 2026-11-24): a profile's `ready` handler opens the buffers it
  wants every time; nothing remembers what was open last time.

## API (2026-11-24, user)

Everything lives on one global object, `web`.

### `web.cmd(line)`

Runs `line` as if typed after `:` and Enter (a leading `:` is dropped). It is the only way
a script acts on the browser: whatever a `:command` can do, a script can do, and nothing
more. Errors show in the status bar as the command's would.

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

Maps a key sequence in a mode. `mode`: `"n"` (NORMAL) only, for now. `rhs`: a string
starting with `:` (run as `web.cmd` would), or a function (called with no arguments).
A user mapping takes precedence over a built-in binding or sequence (config.rs `BINDINGS`,
`KEY_SEQUENCES`) for the same keys; mapping the same keys again replaces the mapping.

`keys` notation (vim's): a character stands for itself (`"gh"`, `"G"`); `<C-x>` is Ctrl
with `x`; `<lt>` is `<`. Any length. While the keys typed so far are a prefix of a longer
mapping, the shell waits up to `MAP_TIMEOUT_MS` (config.rs, as vim's `timeoutlen`) for the
next key; on a timeout or a key that matches nothing, the pending keys are handled as
they would have been without the mapping. Other keys (`<A-...>`, `<Space>`, arrows, ...)
come when a mapping needs them.

### `web.on("ready", fn)`

Calls `fn` once per start of the shell, when the engine host is connected and the first
buffer exists (the URL argument's, as today). This is where default buffers are opened
(`web.cmd("new https://...")`). What the event passes to `fn` is decided as it gets used
(user, 2026-11-24); today it passes nothing. A URL later handed over by `web URL`
(C-SINGLE) does not fire it again.

## Errors and limits

- A script that fails (syntax error, uncaught exception) stops at that point; what it set
  up before the error stays. web starts anyway. The status bar shows
  `init.js:LINE: message` (with the profile's path for the profile script); the full
  error goes to the log.
- An error in a mapped function, a user command or the `ready` handler shows in the
  status bar the same way.
- Every call into the script (the file itself, a mapping, a command, `ready`) has a time
  budget, `SCRIPT_BUDGET_MS` (config.rs); QuickJS's interrupt handler ends a call that
  overruns it with an error, so a stray loop cannot freeze the shell.
- `web.cmd` calls nested through user commands have a depth limit (an error, not a hang).
