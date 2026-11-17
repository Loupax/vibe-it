# wire

A small daemon that does real networking for `scratchy`. It reads one HTTP
request per connection off a Unix socket (absolute-form, e.g.
`GET https://example.com/path HTTP/1.1`), executes it (TLS, redirects,
decompression), and writes back one normalized HTTP/1.1 response. Each
connection gets its own thread. It must be started by hand.

```
wire [--socket <path>] [--print-socket] [--no-cookies]
```

- `wire` listens on `$XDG_RUNTIME_DIR/wire.sock` (falling back to
  `/tmp/wire-$UID.sock`).
- `--socket <path>` listens on `<path>` instead.
- `--print-socket` prints the path wire would bind and exits (for scripts:
  `scratchy --socket "$(wire --print-socket)" page.html`).
- `--no-cookies` disables the in-memory cookie jar (on by default, lost on
  restart, partitioned by top-level site).
- `WIRE_TIMING=1 wire` prints one timing line per request.

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
| `X-Wire-Features: ua,ws` | response | comma-separated optional features, on every response (absent on an older wire); `ua` = the User-Agent override above, `ws` = the WebSocket pipe |
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
