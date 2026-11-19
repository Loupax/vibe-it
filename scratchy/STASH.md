# stash: persistent web storage in scratchy (design, not built)

Status: PROPOSED (2026-11-19). It becomes DECIDED when the PR that adds this
file is merged. Items are QSTASH1-QSTASH5 in `TODO.txt`; the decision record is
QSTASH-D. It supersedes QF-D7 ("nothing persisted") for `localStorage` and
the cookie jar.

## Summary

- There is **no stash daemon**. stash is a directory layout plus a module in
  scratchy (`src/stash.rs`). The filesystem is the backend interface:
  tmpfs, ext4, sshfs and so on are the backends, chosen by where
  `--state-dir` points.
- **One file per key.** The filename is the key and the file's contents are
  the value, so there is no file format.
- **flock between scratchy instances**, one lock per top-level site.
- **wire keeps the cookie jar** and gains `--cookie-file PATH`. That is the
  only state behind a socket, because wire needs the jar on every redirect hop.
- `sessionStorage` is never written to disk.
- History is NOT part of this: it waits until something reads it.

How we got here: an in-memory daemon was considered first (like wire). It was
dropped because wire and a daemon both get restarted routinely, a "save now"
command adds an unsaved state to track, and the filesystem already provides
everything the daemon would: pluggable backends, atomic rename, locks, and
Unix permissions as authentication.

## The state directory

`--state-dir DIR` (default `$XDG_STATE_HOME/scratchy`, falling back to
`~/.local/state/scratchy`). Created with mode 0700; Unix permissions are the
only access control.

```
DIR/
  storage/
    .locks/
      https!loupax.com            <- flock target, never deleted; holds the generation number
    https!loupax.com/             <- top-level site (schemeful, as net_policy::site_of)
      https!social.loupax.com!443/  <- origin: scheme!host!port
        vimOff                    <- contains "1\n"
```

- **Top-level site:** the same partition key wire's cookie jar uses
  (`net_policy::site_of`, `scheme://registrable-domain`), written with `://`
  replaced by `!`. A third-party script runs in the page's origin, so it uses
  the page's directory; a cross-site iframe's origin is stored under the TOP
  site, so it is partitioned like cookies (Total Cookie Protection).
- **Origin:** `scheme!host!port`, always with an explicit port. `!` cannot
  appear in a host, so the name stays readable with no percent-encoding.
- **Opaque origins** (`file:`, `data:`, `about:`, sandboxed): never persisted.
  They keep today's in-memory Storage.

### Backends

| Wanted | `--state-dir` |
| --- | --- |
| Disk (default) | `$XDG_STATE_HOME/scratchy` |
| In memory, survives scratchy restarts, gone at logout | `$XDG_RUNTIME_DIR/scratchy` (tmpfs) |
| Throwaway session | a fresh `mktemp -d` |
| Another machine | sshfs, Syncthing: works for a single instance, locks may be weak (see Locking) |
| S3 | not planned. Mounts have weak locks and non-atomic rename, and a mount's traffic bypasses wire (AGENTS.md network rule): needs the user's OK |

## Keys and values

- **Key to filename.** A key is used as the filename as it is, except that
  these are percent-encoded (`%XX` of the UTF-8 bytes): `/`, `%`, NUL anywhere;
  a leading `.`; a trailing `~`. The empty key is stored as `%`. A key whose
  encoded name exceeds 200 bytes is stored as `%h<sha256 hex>`, whose first line is
  the percent-encoded key and the rest is the value.
- **Value to contents.** WTF-8 (UTF-8 that also allows lone surrogates, so any
  JS string round-trips exactly), followed by one `\n`.
- **Reading.** Take the bytes, remove exactly ONE trailing `\n` if there is
  one. So `echo 1 > vimOff`, `printf 1 > vimOff`, an nvim save (which adds
  a final newline) and scratchy's own write all read back as `1`; a value that
  really ends in `\n` is written as `...\n\n` and reads back correctly.
- **Ignored when loading:** dotfiles (scratchy's `.tmp-*`, editor swap files)
  and names ending in `~` (editor backups). No real key can produce either name,
  thanks to the encoding above.
- **Editing by hand is supported.** Checked with vim (2026-11-19): a file
  containing a lone surrogate opens as utf-8 and the bytes survive an edit. The
  only change an editor makes is the final newline, which reading removes. A
  page loading at the exact moment of a non-atomic editor save may see a
  half-written value; reloading fixes it.

## Operations

| Storage call | Filesystem | Lock |
| --- | --- | --- |
| page load (before the first script runs) | list the origin directory, read each file | shared |
| `setItem` (batched per task) | write `.tmp-<pid>-<n>`, `fsync`, `rename` over the key | exclusive |
| `removeItem` (batched) | `unlink` | exclusive |
| `clear()` | unlink every key file in the origin directory | exclusive |
| forget site | `rm -r` the site directory, then increment the generation | exclusive |

- **During the page, no IO happens.** Scripts read and write a local copy
  (the `Map` in `globals_prelude.js`), which is filled at load.
- **Writes are batched.** Each task's set/remove ops are queued and handed to
  a writer thread when the task ends, so the main thread never waits on the
  lock or on `fsync`. A crash loses at most the ops queued but not yet written.
- **`localStorage.key(i)` order:** the key list is sorted once at load, so it
  is stable for the life of the page.
- **Size limit:** 5 MB per origin, counted as the bytes of keys plus values
  (browsers count UTF-16 units; the exact unit does not matter here).
  `setItem` over the limit throws `QuotaExceededError`, as browsers do. The
  check runs in the page against its local copy; the writer thread checks
  again against the files under the exclusive lock and logs (and skips) a
  write that another window pushed over the limit.
- **The state directory is unusable** (unwritable, missing, full): one log
  line `scratchy: stash: <dir>: <os error>, localStorage is in-memory for this
  session`, then today's behaviour.

## Locking (between scratchy instances)

- One lock file per top-level site, `storage/.locks/<site>`, taken with
  `File::lock_shared` (load) or `File::lock` (everything else). Rust std, no
  new dependency.
- The lock files live OUTSIDE the data directories and are never deleted.
  Otherwise forget-site's `rm -r` would delete a lock file while another
  window waits on it, and that window would then lock a deleted file and write
  into a freshly created directory.
- `flock` locks are released by the kernel when a process dies, including on
  a crash: no PID files, no stale locks.
- **Generation number.** The lock file contains a counter that forget-site
  increments. Each page remembers the generation it loaded with; on commit,
  under the lock, a mismatch means the site was forgotten: the page drops its
  queued ops and empties its local copy. Without this, a window still open on
  a forgotten site would write the data back.

What locking gives: a task's batch, `clear()` and forget-site are seen by
other scratchy windows as all or nothing.

What it does NOT give, by design:
- **Read-modify-write across windows** (`localStorage.n++` in two windows):
  each window reads its own copy loaded with the page, so one update is lost.
  The only fix would be holding a lock while page JS runs (the spec's old
  "storage mutex", dropped by every browser). Chrome and Firefox lose this
  update too.
- **`storage` events between windows.** Possible later with inotify if a page
  needs it.
- **Protection from editors.** flock is advisory; nvim does not take it.
- **Network filesystems.** NFS, sshfs and FUSE mounts range from emulated
  locks to none.

## sessionStorage

Never written to disk. Today each page gets a new V8 realm
(`startup_engine`), so `sessionStorage` is lost on every navigation, which is
wrong: it must live per window (one scratchy process) across navigations.
QSTASH4 keeps it in Rust and gives each page a copy, keyed by origin.

## Cookies (wire)

- `wire --cookie-file PATH`: loads the jar at start, saves it (write a temp
  file, rename) at most once a second while it has changes, and on SIGTERM.
- Cookies with no expiry date ("session cookies") are NOT saved: they die with
  wire, which is the browser session from the jar's point of view.
- The file is created with mode 0600. Its format is wire's own choice (one
  cookie per line is enough); scratchy never reads it.
- The control channel gets `X-Wire-Cookie-Op: clear-site` (with
  `X-Wire-Top-Site`), used by forget-site.
- Unchanged: the control channel still has no access token (user decision
  2026-11-17, wire/TODO.txt). With cookies on disk the stakes rise; the user
  may want to revisit that before QSTASH5 ships.

## Forget site

`scratchy --forget-site <site or URL>` (and later a key binding):
1. stash: take the site's exclusive lock, `rm -r storage/<site>`, increment
   the generation.
2. wire: `X-Wire-Cookie-Op: clear-site`.
scratchy is the only component that knows both stores exist; the two never
talk to each other. If wire is not reachable, step 1 still runs and the log
says the cookies were not cleared.

## Later, not part of this design

- History: an append-only `DIR/history` file written by scratchy, once a
  feature reads it (URL completion, restoring back/forward after a restart).
- `--profile NAME` (QF14b2 (d), skipped by the user 2026-11-19) becomes
  "a different `--state-dir` plus a different `--cookie-file`" if it is ever
  wanted.
- HTTP cache, IndexedDB, Cache API: not planned.
