# stash: persistent web storage

Status: design, not built. Decision QSTASH-D, items QSTASH1-QSTASH5 in
`TODO.txt`; wire's part is W-PERSIST1 in `../wire/TODO.txt`.

stash is the `localStorage` store: a directory layout plus `src/stash.rs`.
There is no daemon; scratchy reads and writes the files itself.

## State directory

- `--state-dir DIR` / `SCRATCHY_STATE_DIR`; default `$XDG_STATE_HOME/scratchy`
  (`~/.local/state/scratchy` when unset). Created with mode 0700.
- A tmpfs directory (e.g. `$XDG_RUNTIME_DIR/scratchy`) gives storage that
  survives scratchy restarts but not a logout.
- If the directory cannot be used, scratchy logs once
  (`scratchy: stash: <dir>: <os error>, localStorage is in-memory for this session`)
  and keeps localStorage in memory.

```
DIR/
  storage/
    .locks/
      https!loupax.com              <- lock file, holds the generation number
    https!loupax.com/               <- top-level site
      https!social.loupax.com!443/  <- origin
        vimOff                      <- one file per key; contents "1\n"
```

- **Site:** `net_policy::site_of` of the top-level page (the same partition as
  wire's cookie jar), with `://` written as `!`.
- **Origin:** `scheme!host!port`, port always explicit. A cross-site iframe's
  origin is stored under the top-level site.
- **Opaque origins** (`file:`, `data:`, `about:`, sandboxed) are never stored;
  their localStorage stays in memory.

## Keys and values

- **Filename = key**, with `%XX` (UTF-8 bytes) for: `/`, `%` and NUL
  anywhere, a leading `.`, a trailing `~`. The empty key is `%`. A key whose
  encoded name is over 200 bytes is stored as `%h<sha256 hex>`; that file's
  first line is the encoded key, the rest is the value.
- **Contents = value** encoded as WTF-8, followed by `\n`.
  - WTF-8 is UTF-8 that can also encode lone surrogates (unpaired UTF-16 halves,
    which JS strings may contain), so every JS string round-trips exactly.
  - The value must be taken from V8 as UTF-16 code units and encoded to WTF-8
    by stash. V8's UTF-8 conversions (`to_rust_string_lossy` and similar)
    replace lone surrogates with U+FFFD and must not be used for keys or values.
- **Reading** removes exactly one trailing `\n` if present, so files written
  by hand or saved by an editor read back correctly.
- **Loading ignores** dotfiles and names ending in `~` (temp, swap and backup
  files). Encoded keys never produce either.

## Operations

| Storage call | Files | Lock |
| --- | --- | --- |
| page start, before the first script | list the origin directory, read every key | shared |
| `setItem` | write `.tmp-<pid>-<n>`, fsync, rename over the key file | exclusive |
| `removeItem` | unlink the key file | exclusive |
| `clear()` | unlink every key file of the origin | exclusive |
| forget site | `rm -r` the site directory, increment the generation | exclusive |

- Page scripts read and write an in-memory copy filled at page start; no file
  IO happens while JS runs.
- `setItem` / `removeItem` / `clear()` are queued and handed to a writer
  thread at the end of each task; one task's ops are committed under one lock.
- `localStorage.key(i)` uses the key list sorted at load.
- **Limit:** 5 MB of keys plus values per origin. `setItem` beyond it throws
  `QuotaExceededError` (checked against the page's copy). The writer thread
  checks again under the lock and skips, with a log line, a write that would
  exceed it.

## Locking

- One lock per site: `storage/.locks/<site>`, `File::lock_shared` for loading,
  `File::lock` for everything else.
- Lock files are outside the data directories and are never deleted.
- **Generation:** the lock file holds a counter that forget-site increments.
  A page records the generation at load. When committing under the lock, a
  different generation means the site was forgotten: the queued ops are
  dropped and the page's copy is emptied.

Not provided: atomic read-modify-write across windows, `storage` events between
windows, and locking on filesystems where flock does not work.

## sessionStorage

Never written to disk. It is kept in memory per origin for the life of the
scratchy process, so it survives navigations; each new page gets a copy and
its ops are applied back.

## Cookies

wire owns the cookie jar (W-PERSIST1):
- `wire --cookie-file PATH` loads the jar at start, saves it (temp file +
  rename, mode 0600) at most once a second while it has changes, and on
  SIGTERM. Cookies without an expiry date are not saved.
- `X-Wire-Cookie-Op: clear-site` with `X-Wire-Top-Site` clears one partition.

## Forget site

`scratchy --forget-site <site or URL>`:
1. stash: exclusive lock, `rm -r storage/<site>`, increment the generation.
2. wire: `X-Wire-Cookie-Op: clear-site`. If wire is unreachable, step 1 still
   runs and the log says the cookies were not cleared.
