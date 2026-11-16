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
| `X-Wire-Version` | response | protocol version (currently 3), on every response |
| `X-Wire-Final-Url`, `X-Wire-Target-Class`, `X-Wire-Hop-Classes` | response | URL after redirects, address classes |
| `X-Wire-Error` | response | on wire-made errors (`1`, `too-large`, `policy`, `cookie-control`) |
| `X-Wire-Cookies: on/off` | response | jar used / disabled |

After rebuilding, restart wire: `cargo install --path . --force`, kill the old
process, start it again. scratchy detects an old wire by `X-Wire-Version`.

Details: [`NOTES.md`](NOTES.md). Overview from scratchy's side:
[`../scratchy/NETWORKING.md`](../scratchy/NETWORKING.md).
