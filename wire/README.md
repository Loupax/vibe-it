# wire

A small daemon that does real networking for `scratchy`. It reads one HTTP
request per connection off a Unix socket (absolute-form, e.g.
`GET https://example.com/path HTTP/1.1`), executes it (TLS, redirects,
decompression), and writes back one normalized HTTP/1.1 response. Each
connection gets its own thread. It must be started by hand.

```
wire [--socket <path>] [--print-socket] [--no-cookies] [--cookie-file <path>]
```

- `wire` listens on `$XDG_RUNTIME_DIR/wire.sock` (falling back to
  `/tmp/wire-$UID.sock`).
- `--socket <path>` listens on `<path>` instead.
- `--print-socket` prints the path wire would bind and exits (for scripts:
  `scratchy --socket "$(wire --print-socket)" page.html`).
- `--no-cookies` disables the in-memory cookie jar (on by default, lost on
  restart, partitioned by top-level site).
- `--cookie-file <path>` keeps the jar on disk: loaded at start (expired cookies
  dropped; a corrupt file is one log line and an empty jar), saved at most once
  a second while it changes and on SIGTERM (temp file, fsync, rename, mode 0600).
  Format: a `# wire cookies v1` line, then one cookie per line, tab-separated,
  `%`-escaped. Cookies with no expiry date (session cookies) are not saved.
  Cannot be combined with `--no-cookies`.
- `WIRE_TIMING=1 wire` prints one timing line per request.

## Cookie control token

At every start wire writes a random token (32 bytes, 64 hex digits) to
`<socket path>.token` (e.g. `/run/user/1000/wire.sock.token`), mode 0600,
replacing the old one. Every cookie control request (any `X-Wire-Cookie-Op`:
`get`, `set`, `clear-site`) must send `X-Wire-Control-Token: <token>`; a
missing or wrong token is answered `403` (one log line, nothing forwarded
upstream). Ordinary requests that use the jar (`X-Wire-Jar: scratchy`) do not
need it.

## `X-Wire-*` headers

`X-Wire-*` request headers are the scratchy-to-wire control channel and are
never forwarded upstream. Without them wire behaves as a plain proxy.

| Header | Direction | Meaning |
| --- | --- | --- |
| `X-Wire-Policy: public-only` | request | refuse non-public targets and `file:` (403) |
| `X-Wire-Max-Bytes`, `X-Wire-Timeout-Ms`, `X-Wire-Max-Redirects` | request | per-request caps |
| `X-Wire-Jar: scratchy` | request | use the cookie jar |
| `X-Wire-Top-Site`, `X-Wire-Nav`, `X-Wire-Initiator-Site` | request | cookie partition and SameSite context |
| `X-Wire-Cookie-Op`, `X-Wire-Cookie-Url` | request | the `document.cookie` control channel |
| `X-Wire-User-Agent: <ua>` | request | upstream `User-Agent` for this request (takes precedence over the request's own `User-Agent`); never forwarded as such |
| `User-Agent` | request | forwarded upstream when no `X-Wire-User-Agent` is given; with neither, wire sends its honest default `wire/0.1 (+https://github.com/Loupax/vibe-it) ...` |
| `X-Wire-Upgrade: websocket` | request | with `Connection: Upgrade` + `Upgrade: websocket`: the WebSocket pipe (below); without it an Upgrade request is an ordinary request |
| `X-Wire-Features: ua,ws,jarfile` | response | comma-separated optional features, on every response (absent on an older wire); `ua` = the User-Agent override above, `ws` = the WebSocket pipe, `token` = cookie control requests need the control token (below), `jarfile` = `--cookie-file` and the `X-Wire-Cookie-Op: clear-site` control op (with `X-Wire-Top-Site` and `X-Wire-Jar: scratchy`; removes every cookie of that top-level site's partition, first-party and embedded third-party, answers 204, saves at once) |
| `X-Wire-Version` | response | protocol version (currently 3), on every response |
| `X-Wire-Final-Url`, `X-Wire-Target-Class`, `X-Wire-Hop-Classes` | response | URL after redirects, address classes |
| `X-Wire-Error` | response | on wire-made errors (`1`, `too-large`, `policy`, `cookie-control`, `bad-user-agent`) |
| `X-Wire-Cookies: on/off` | response | jar used / disabled |

The chosen User-Agent must be printable ASCII (space allowed), at most 512
bytes; anything else is `400` + `X-Wire-Error: bad-user-agent` and is never
sent. It is applied on every redirect hop, on the jar path and on the
`public-only` pool.

`public-only` requests use their own connection pool, so reusing a pooled
connection can never skip the check; class headers are remembered per
host:port so reused connections still report them.

## WebSocket pipe (`X-Wire-Upgrade: websocket`)

wire is a pipe, not a WebSocket implementation: it never parses a frame.
The request is a `GET ws://...` / `wss://...` (or `http(s):`) in the usual
absolute form. It passes the same checks as any request (`X-Wire-Policy`,
class headers, cookie jar, User-Agent, `Origin`/`Sec-WebSocket-*` passed
through), always on a fresh connection (never pooled), through the checking
resolver. Upstream `101`: wire returns the 101 head, then copies bytes both
ways until either side closes. Any other answer (a 3xx is not followed) is a
normal buffered response and the connection closes. `X-Wire-Timeout-Ms` covers
the handshake only; there is no idle limit. Code: `src/ws.rs`.

After rebuilding, restart wire: `cargo install --path . --force`, kill the old
process, start it again. scratchy detects an old wire by `X-Wire-Version`.

Details: [`NOTES.md`](NOTES.md). Overview from scratchy's side:
[`../scratchy/NETWORKING.md`](../scratchy/NETWORKING.md).

## Static mounts (`--mount`, `--mount-override`)

```
wire --mount http://HOST[:PORT]=DIR [--mount ...] [--mount-override PATH=FILE ...]
```

Serves an http origin straight from a directory; no socket is opened for it and
nothing goes upstream. Built for scratchy's testharness.js runner (R64 2.4
option 3), where `DIR` is a web-platform-tests checkout and the origin is
`http://web-platform.test:8000`. Requests whose URL matches a mounted origin
are answered from the mount before anything else (cookie control requests
excepted), even with `X-Wire-Policy: public-only` (the operator opted in); the
response says `X-Wire-Target-Class: file`.

- Origin match: `HOST` plus `www.`, `www1.`, `www2.` of it, on `PORT` and
  `PORT+1` (so `web-platform.test:8000` also answers `:8001`, a second origin
  for cross-origin tests). Everything is the same tree.
- Methods GET and HEAD (others 405). Query strings are ignored. A directory
  serves its `index.html`.
- `Content-Type` by file extension (html, xhtml, css, js, json, svg, images,
  fonts, media, wasm; unknown = `application/octet-stream`).
- `__dir__.headers` of the file's own directory (only that one, as wptserve),
  then `<file>.headers` (it wins): lines `Name: value`, replacing same-named
  headers, so they can override `Content-Type`. `<x>.sub.headers`, when it
  exists, is used instead of `<x>.headers` and gets `.sub.` substitution.
  `.headers` files are never served themselves. A generated wrapper gets the
  headers of its `.js` file (its directory's `__dir__.headers`, `x.any.js.headers`).
- `x.any.html` is generated from `x.any.js` (window variant) and `x.window.html`
  from `x.window.js` when no real file of that name exists, with wptserve's
  templates (`AnyHtmlHandler`, `WindowHandler` in tools/serve/serve.py, checked
  at scratchy's WPT pin): `// META: script=PATH` adds a `<script src>`,
  `title=TEXT` a `<title>`, `timeout=long` a `<meta name=timeout content=long>`.
  META lines start at column 0 and end at the first other line. The first
  `global=` decides: empty = default (window + worker); otherwise a window
  variant only if one comma-separated item is `window` (else 404).
  Worker variants are not generated.
- `/resources/WebIDLParser.js` is an alias of `/resources/webidl2/lib/webidl2.js`.
- Paths with `.sub.` in them (a wrapper: its `.js` path) get `{{host}}` (the request's host),
  `{{domains[]}}`, `{{domains[www]}}`, `{{domains[www1]}}`, `{{domains[www2]}}`,
  `{{ports[http][0]}}` (= PORT) and `{{ports[http][1]}}` (= PORT+1) replaced;
  other `{{...}}` text is left alone.
- `--mount-override PATH=FILE`: requests for exactly `PATH` (any mounted
  origin) get `FILE`'s bytes, typed by `PATH`'s extension. For
  `/resources/testharnessreport.js` and `/resources/testdriver-vendor.js`.
- `.py` files answer 501 and log one line (`wire: mount: URL needs a Python
  handler`).
- Safety: the path is percent-decoded and refused (403) if any segment is `..`,
  contains `\`, NUL or an encoded `/`; the file's real path (symlinks
  resolved) must stay under `DIR`, else 403. Code: `src/mount.rs`.

```
wire --socket "$S" --mount http://web-platform.test:8000=$HOME/src/wpt \
     --mount-override /resources/testharnessreport.js=./my-report.js
```

## Record and replay (`--record`, `--replay`)

`wire --record DIR` stores every network response it serves (a mounted origin
is never recorded): the body once per content hash in `DIR/blobs/<sha256>`
(zstd) and one line per response in `DIR/index.jsonl` (`seq, t_ms, method, url,
range, req_body_sha256, status, reason, headers, body_sha256, body_len`).
Synthetic wire errors (`X-Wire-Error`) and WebSocket traffic are not recorded.
`wire --replay DIR` answers only from the recording and never opens a network
socket. Matching, in order: exact (method, url, request body hash, range); the
same with ignored query parameters removed (`--replay-rules FILE`, lines
`host name,name`, `*` for every host); a range request inside a recorded full
body or recorded `206` range (answered `206`); the n-th request to the same
host+path with the same query after the ignore list (the last recorded one once
they run out). A differing query is a miss unless the rules file has a line
`path-fallback HOST /PATH` (`*` for any host or path), which allows the n-th
request to that host+path whatever its query. A miss is `404` with
`X-Wire-Replay: miss` and one log line; hits carry `X-Wire-Replay: hit`.
`--replay-delay 5ms` sleeps per request (the recorded latency is never used).
A WebSocket upgrade is refused (`403`, `X-Wire-Replay: websocket-refused`).
Recordings may contain cookies and personal data: keep them local
(`~/.cache/scratchy/recordings`), never commit one. Known limit: the cookie
jar path strips `Set-Cookie` from answers, so replay does not feed the jar.
