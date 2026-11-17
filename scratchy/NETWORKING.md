# Networking in scratchy

scratchy does no networking itself. Every byte that comes from the network
(page loads, sub-resources, requests made by page JavaScript) goes through
[`wire`](../wire/), a separate daemon, over a Unix socket given with
`--socket`. Without `--socket` nothing is fetched. This file is the overview;
the code is `src/net_policy.rs` (the policy), `src/js_net.rs` (the queue and
CORS) and `src/net.rs` (the wire codec). `../wire/NOTES.md` has wire's side.

## Request flow (page JS)

```
fetch / XMLHttpRequest / sendBeacon
  -> __net_send        RequestPolicy::check_request (blocked here, or warned)
  -> job queue         CORS classified, beacon suppressed here, preflight cached
  -> thread            at most 6 at once, 64 MiB of responses in flight
  -> wire              one request per connection on the Unix socket
  -> response          CORS check, headers filtered, body handed to the page
```

A refused request still answers the page asynchronously (a network error), so
page code sees the same shape as a real failure. An aborted request is dropped
from the queue or its answer is discarded.

## What is allowed and blocked

| Check | Default |
| --- | --- |
| Scheme | `http`, `https` and `file` only; anything else is blocked |
| Methods | `CONNECT`, `TRACE`, `TRACK` blocked; a body on GET/HEAD blocked |
| Mixed content | `http:` from an `https:` page blocked (loopback is exempt) |
| Bad ports | the browser "restricted ports" list is blocked (e.g. 25, 6000) |
| Private targets | loopback, private, link-local, CGNAT, ULA, multicast, unspecified, reserved, and names ending in `.localhost`, `.local`, `.internal`: allowed with one warning line (`private_targets`) |
| `file:` targets | allowed with one warning line (`file_targets`) |
| URL length | 32 KiB |

`private_targets` and `file_targets` can each be `Block`, `Warn` (default) or
`Allow`. A request to the page's own origin never warns. For a DNS name that
resolves to a private address, wire reports the class in `X-Wire-Target-Class`
and scratchy warns from that. Redirect hops are checked again. Page JS cannot
set forbidden headers (`Cookie`, `Origin`, `Host`, `Sec-*`, ...) or any
`X-Wire-*` header: that is the control channel.

## CORS

Browser rules. Same-origin requests are `basic` and fully readable. A
cross-origin `cors` request is sent, then `Access-Control-Allow-Origin` (and
`-Credentials` when credentials are included) must match or the page gets a
network error. Non-simple requests send an `OPTIONS` preflight first; the
result is cached (key: origin, URL, credentials) for `Access-Control-Max-Age`,
capped at 2 h, 5 s when absent, 128 entries. A `no-cors` request gives an
opaque response. `Access-Control-Expose-Headers` decides which response
headers the page may read; `Set-Cookie` is never readable.

## Cookies

wire owns the jar. It is in memory, on by default, and lost when wire
restarts. `wire --no-cookies` disables it. Partitioned by top-level site
(Firefox's Total Cookie Protection): a third party sees only the cookies it set
under the same top site. A request uses the jar only when scratchy asks for
it, so snag and other clients are unaffected.

| Header | Direction | Meaning |
| --- | --- | --- |
| `X-Wire-Jar: scratchy` | request | use the jar for this request |
| `X-Wire-Top-Site` | request | the partition (top-level page's site) |
| `X-Wire-Nav: 1` | request | top-level navigation (SameSite=Lax cookies may go) |
| `X-Wire-Initiator-Site` | request | with Nav: who started the navigation (Strict) |
| `X-Wire-Cookies: on/off` | response | jar used / jar disabled by `--no-cookies` |
| `X-Wire-Cookie-Op: get/set` | request | the `document.cookie` channel (below) |

`document.cookie` talks to wire synchronously: `get` returns the visible
(non-HttpOnly) cookies, `set` takes the assignment string as the body. Each
call blocks the page for at most 2 s; on a failure (no wire, old wire,
cookies off, timeout) it logs once and reads `''` for the rest of the page.
`Set-Cookie` never reaches the page. Names and values are never logged.

## Form submission (Q30e)

A form GET or POST is a top-level navigation of the current tab (`App::form_navigate`
in `src/forms_submit.rs`): `RequestPolicy::check_request` runs on the action URL first
(https page -> http action is blocked with a log line, loopback exempt; a private target
warns), then the request goes through the same path as a link click with the cookie
headers of `CookieContext::navigation` (`X-Wire-Nav: 1`, `X-Wire-Initiator-Site` = the
page, so wire's SameSite rules see a POST as not "safe": Lax cookies are withheld on a
cross-site POST). A POST also sends `Content-Type`, `Content-Length` and `Origin`. wire
follows redirects (303/302 after POST become GET) and reports `X-Wire-Final-Url`, which
scratchy now reads for every top-level navigation: the page's base URL and its history
entry become the final URL, so back/refresh re-GET the result and never re-POST.

## Beacons

`navigator.sendBeacon`, `fetch(..., {keepalive: true})` and `no-cors` fetches
are analytics-shaped: by default they are logged in detail and NOT sent
(`beacons_send` is false). `sendBeacon` returns false when the queued bytes
would pass 64 KiB.

## Sync XHR

`XMLHttpRequest` with `async=false` blocks the engine thread, but for at most
5 s. It goes through the same policy.

## Limits (`RequestPolicy::default()`)

| Limit | Value |
| --- | --- |
| Response body | 16 MiB per request |
| In flight | 64 MiB of reserved response bytes, 6 concurrent requests |
| Requests per page | 1000 |
| Connect / total timeout | 30 s / 30 s |
| Legacy idle timeout (link clicks, sub-resources) | 60 s |
| Redirects | 5 |
| Sync XHR timeout | 5 s |
| `document.cookie` control call | 2 s |
| Preflight cache | 128 entries, max age cap 7200 s, default 5 s |
| Beacon queue | 64 KiB |
| URL length | 32 KiB |

## Log lines

On stderr:

```
scratchy: net warning: <page> -> <target> (<class>) allowed by policy
beacon <api> <method> <url> content-type=<ct> body=<n> bytes body[..200]="..." page=<page>: suppressed (not sent)
scratchy: page networking disabled: wire is too old (restart it)
```

`<class>` is `loopback`, `private`, `link-local`, `cgnat`, `ula`,
`multicast`, `unspecified`, `reserved`, `local-name` or `file`; `<api>` is
`navigator.sendBeacon`, `fetch keepalive` or `fetch no-cors`. `--js-report`
prints a `--- network (N requests, B bytes) ---` section with one line per
request: `METHOD url -> outcome bytes [ms] jar: scratchy|off`.

## Restarting wire after a rebuild

wire is started by hand and keeps running the old binary. After changing it:

```sh
cd wire && cargo install --path . --force
kill <old wire pid>        # find it with: pgrep -x wire
wire &
```

scratchy reads `X-Wire-Version` on every response. Version 2 added the policy
caps, 3 the cookie jar. A wire with no such header (older than 2) makes
scratchy fail closed: page networking is off for the rest of the page, with
the one line `scratchy: page networking disabled: wire is too old (restart
it)`. A wire older than 3 means cookies are unavailable (logged once). Every response
also carries `X-Wire-Features` (comma list; `ua` = per-request User-Agent
override via `X-Wire-User-Agent` or the request's own `User-Agent`; `ws` = the WebSocket pipe, `X-Wire-Upgrade: websocket`, QF19w1); scratchy
must see `ua` there before relying on a UA override (an older wire ignores it).

## Not implemented

- Cookie persistence (the jar is memory only; a file is a later flag).
- EventSource, Workers, Service Workers (WebSocket exists since QF19w2, below).
- Form submission is implemented (Q30e, below) except file contents, validation and `target` other than this tab.
- Media (`<video>`, `<audio>`): placeholders only.
- `<a ping>`.

## Enforcement (QNET1)

scratchy installs a seccomp filter at startup: only AF_UNIX sockets can be created,
so it cannot reach the network except through wire. See NOTES.md "QNET1" and the
`:help` page for how to verify it with `unshare -rn`.

## Wire unreachable (QWU1)

When wire cannot be reached, sub-resources (stylesheets, @import, scripts, images, CSS images, iframes, page-JS fetch/XHR, media) are skipped. Scratchy prints ONE stderr line per page load: `scratchy: wire not reachable at <path> (<os error>): N sub-resources skipped`. The OS error tells why: `No such file or directory, os error 2` = no socket file; `Connection refused, os error 111` = stale socket file (wire died). Approach: failures are counted (`note_wire_skip`); a watcher prints the line once no new failure arrived for 400 ms (the load settled), so N is the final count; later stragglers are not printed again. `begin_wire_load` starts a count per navigation; file:// pages without --socket never begin one, so never print. Page JS and media still log their own errors; they are only counted in N.

## WebSocket (QF19w2)

`new WebSocket(url, protocols)` goes through wire's upgrade pipe (QF19w1). scratchy builds the
handshake (random `Sec-WebSocket-Key`, version 13, `Sec-WebSocket-Protocol`, `Origin`, `Upgrade`/`Connection`
headers, `X-Wire-Upgrade: websocket`, the cap headers, the cookie headers of a credentialed fetch,
`X-Wire-User-Agent` when set) and sends it to wire's unix socket. wire dials (DNS/TCP/TLS/policy/jar),
answers `101` and copies bytes; the RFC 6455 framing (src/ws_frames.rs, by hand: `tungstenite` is not
in the offline registry) runs in scratchy over that unix socket only. The URL passes the same
`RequestPolicy` as a fetch (http(s) twin): an https page may not open `ws:` (loopback excepted), bad
ports, private targets. Every refusal still shows the page an `error` then `close` 1006 event
asynchronously, with one log line. An old wire (no `ws` in `X-Wire-Features`): the first socket fails
with one log line (`wire is too old for WebSocket`), later sockets fail at once without sending. Caps:
16 open sockets per page, 16 MiB per frame/message, 64 MiB of undelivered messages (then the reader
waits). Dropping the page's engine (navigation) sends close 1001 and shuts the sockets.
