# wire — prompt log

Append-only log of the exact prompt that produced each feature, in the order
it was built.

---

## v1 — the daemon itself: parse, execute, normalize

> You'll find a todo item on scratchy about building a new tool called
> wire. Start working on it

The item in question is `scratchy/TODO.txt`'s own "TOP PRIORITY" entry
(logged in a prior session, `32ee24c`, as a planning-only record of a
design conversation — see that commit's own message): `scratchy` wants
to stop talking to `snag --serve` for live navigation and instead speak
HTTP over a Unix socket to a new, separate daemon built specifically to
own real networking, so `snag` can narrow back down to its one actual
job (bundling a page to a single file) and `scratchy` can keep its own
"no networking of its own" design intact. That TODO entry already
settled every real design question in conversation before this session
(request framing, response normalization contract, one-connection-per-
request with concurrency expected, the `--socket` opt-in/fallback shape
on `scratchy`'s side) — this session's job was building the daemon
itself from that already-agreed spec, not re-deciding any of it.

**Scope for this first version, matching exactly what the TODO item
describes as `wire`'s own job** (the `scratchy`-side consumption —
replacing `fetch_from_snag`, wiring up `<button>`/form submission,
narrowing `snag`'s own CLI — is explicitly a separate, not-yet-started
slice of the same TODO item, left untouched here):

1. **Protocol**: one real HTTP/1.1 request per connection, read off a
   Unix domain socket, in the standard proxy absolute-form shape
   (`GET https://example.com/path HTTP/1.1`, RFC 7230 §5.3.2) — headers,
   then exactly `Content-Length` bytes of body if present. No chunked
   *request* support (the one real client, `scratchy`, always knows its
   own body length up front).
2. **Execution**: for real, via `ureq` (the exact crate `snag` already
   trusts) — TLS, redirect-following, and gzip decompression (the
   dominant real-world `Content-Encoding`; `brotli` isn't enabled, same
   per-need-only philosophy as everywhere else in this repo — nothing
   has needed it yet).
3. **Response normalization — the contract the TODO item itself settled
   as the one actually-debated point**: never a raw 1:1 byte relay.
   `Transfer-Encoding`/`Content-Encoding` are always stripped (the body
   has already been read to completion and decompressed by the time
   anything is written back) and a fresh, correct `Content-Length` is
   computed over the real, final body bytes. Every other upstream
   header passes through as `ureq` reports it. A real non-2xx status
   from the actual remote server (`ureq` surfaces these as
   `Err(Error::Status(..))`, not `Ok`) forwards as itself — a 404 stays
   a 404 — never collapsed into a generic failure. A genuine *transport*
   failure (DNS, connection refused, TLS handshake failure, timeout) —
   the one case where there's no real upstream response to normalize at
   all — synthesizes a `502 Bad Gateway` with a plain-text body naming
   the failure, so the daemon always hands back exactly one consistent
   response shape, never a hang or a dropped connection.
4. **Concurrency**: the TODO item is explicit that `scratchy` may open
   several *separate* one-shot connections at once (future per-resource
   fetches, each its own connection, no multiplexing on any single one)
   — unlike `snag --serve`'s serial one-at-a-time accept loop, `wire`
   spawns one thread per accepted connection.
5. **CLI**: `wire` (binds `$XDG_RUNTIME_DIR/wire.sock`, falling back to
   `/tmp/wire-$UID.sock` — the same per-user convention `snag --serve`'s
   own `socket_path` already established, just this tool's own
   filename) or `wire --socket <path>`. Stale-socket-file cleanup on
   bind, same precedent `snag --serve` already has, for the same reason
   (a crashed previous run leaves a useless file behind that would
   otherwise make every future bind fail forever).

**What landed**: `wire/Cargo.toml` (a new crate, `ureq` with its `gzip`
feature + `url`, same dependency shape `snag` already has minus
`base64`, which this tool has no use for). `wire/src/main.rs` —
`parse_request` (pure, no I/O beyond the passed-in reader: request
line, headers, `Content-Length` body), `execute` (the one function that
actually dials out, folding `ureq`'s `Ok`/`Err(Status)`/`Err(Transport)`
three-way split down into "always produce a `NormalizedResponse`"),
`normalize_response`/`synthetic_error_response`/`write_response`,
`run_server`/`handle_connection`/`default_socket_path`/`current_uid`
(the daemon plumbing, directly adapted from `snag --serve`'s own
precedent where the shape matches, diverging only where the TODO item's
spec explicitly calls for something different — per-connection threads
instead of a serial loop).

**10 new permanent tests**, following the same "no real network, no
real sockets outside this process" convention `snag`'s own test module
established: `parse_request` against a simple absolute-form `GET`, a
`POST` with a `Content-Length` body and extra headers, empty input
(`Ok(None)`, not an error), and a malformed request line (`Err`, not a
panic); `execute` against a tiny in-process loopback `TcpListener`
fixture for an ordinary response, a `Transfer-Encoding: chunked`
response (confirmed decoded and re-framed with a correct
`Content-Length`, with no `Transfer-Encoding` left in the forwarded
headers), a real gzip-compressed response (a literal, hand-built gzip
member — not produced by a compression crate on the test's own side, so
the test actually proves `wire`'s own `ureq` `gzip` feature is doing the
decoding — confirmed decompressed with `Content-Encoding` stripped), a
real upstream 404 (confirmed it forwards as a 404, not a transport
error — the exact distinction `ureq`'s `Error::Status`/`Error::
Transport` split exists to let this file get right), and a genuine
connection failure (nothing listening on `127.0.0.1:1`, confirmed a
synthesized `502` comes back rather than a hang or a panic); and one
real end-to-end Unix-socket round trip (`handle_connection` against a
real `UnixListener`/`UnixStream` pair, a real loopback fixture behind
it) confirming the full wire protocol — not just `execute` in
isolation — produces the expected framed response. `cargo build`/
`cargo clippy --all-targets` both clean, `cargo test` 10/10 passing.

**Live verification — what was and wasn't checked against the real
world, stated plainly, same discipline this repo's other `PROMPTS.md`
entries already hold to**: built a release binary, ran it as a real
long-lived daemon against a real Unix socket, and drove it with a
throwaway Python client (same tool `snag`'s own v9 verification used,
since this sandbox still has no `nc`/`socat`) for several real,
non-unit-test scenarios: `--help`, a missing `--socket` argument, an
unrecognized flag, a real `GET` against `127.0.0.1:1` (nothing
listening) confirmed to come back as a real `502` over the actual
socket — not just inside `execute`'s own unit test — and a stale-socket-
file rebind after a hard `pkill`, confirmed the daemon starts cleanly on
the second run against the same path. **What could not be verified
live, flagged rather than glossed over**: an actual fetch of a real
public Internet host (`https://example.com/`) failed in this sandbox —
checked both with the default build (plain DNS resolution failure, no
proxy configured) and, as a throwaway local experiment only, with
`ureq`'s `proxy-from-env` feature temporarily enabled and pointed at
this sandbox's own outbound HTTPS proxy (confirmed working, then
reverted — `wire`'s own shipped `Cargo.toml` never gained this feature;
baking ambient-env-var proxy auto-detection into every real-world user's
daemon by default would be a deliberate, unasked-for scope decision of
exactly the kind this repo's own `TODO.txt` entries flag for sign-off
rather than pick unilaterally, and `ureq` itself ships the feature
opt-in, off by default, for the same reason). Even with that proxy
configured, the sandbox's own network policy denies `CONNECT` to
arbitrary hosts outright (confirmed directly with `curl` against the
same host, independent of `wire`/`ureq` entirely — a `403` from the
sandbox's own proxy gateway, not anything this code could fix). So the
real end-to-end "fetch a real website through `wire`" path rests on the
loopback-fixture tests above (which exercise the exact same `execute`/
`normalize_response` code against a real TCP connection and a real,
if local, HTTP server) plus the synthesized-`502` and real-4xx-forwards-
as-itself live checks, not a literal public-internet round trip — worth
a real check next time a session's sandbox allows outbound traffic to
an arbitrary host.

**Deliverable**: `wire/Cargo.toml`, `wire/src/main.rs` (everything
above), `wire/PROMPTS.md` (this file), `wire/NOTES.md`, `wire/TODO.txt`
— `wire`'s first four files, all new.

## v2 — `--print-socket`

> Add a command in wire that just outputs the active socket so we can
> use that as an argument to scratchy. Update documentation accordingly.

scratchy's own `--socket <path>` flag is a strict opt-in with no
auto-discovery of its own (per `scratchy/TODO.txt`'s own settled
design — see its "TOP PRIORITY" item) — it never guesses where a `wire`
daemon might be listening, so something has to supply that path. Before
this, the only options were hardcoding `wire`'s own `$XDG_RUNTIME_DIR/
wire.sock`-or-`/tmp/wire-$UID.sock` fallback logic a second time in
every script that starts both tools, or passing an explicit `--socket`
to `wire` itself and remembering to reuse the exact same literal path
for `scratchy`. Neither is good: the first duplicates real logic
(wrong the moment `wire`'s own default ever changes), the second is
just manual bookkeeping.

`--print-socket`: resolves the socket path exactly the same way the
real serving path does (same `socket_path.unwrap_or_else(default_
socket_path)` call — not a second, parallel computation that could
silently drift from it), prints it to stdout, and exits immediately —
no bind, no listen, no real work done. Honors an explicit `--socket
<path>` if also given (in either argument order), so it always answers
"what path would this exact invocation use," not just "what's the
default." Lets a script do:

```sh
wire &
scratchy --socket "$(wire --print-socket)" page.html
```

without ever hardcoding `wire`'s own path logic.

Smoke-tested by hand (no automated CLI-argument-parsing test, matching
this file's own existing convention — `main`'s own argument loop was
never unit-tested either, only its pure helpers are): the bare default,
an explicit `--socket` before `--print-socket`, the same after it (flag
order doesn't matter), and confirmed the pre-existing `--socket` with no
path argument still errors the same way as before (this change touches
nothing about that path).

**Deliverable**: `wire/src/main.rs` (`print_socket` flag in `main`'s own
argument loop, `print_usage` updated), `wire/PROMPTS.md` (this entry),
`wire/NOTES.md`, and `README.md`'s own "How they fit together" example
updated to use it instead of hand-expanding `$XDG_RUNTIME_DIR` directly.


---

## v-file — `file://` URLs (scratchy Q22)

Added `serve_file`/`content_type_for_path` in `src/main.rs`: a `file://` request line is answered from disk (`200` + guessed `Content-Type`, `404` unreadable, `405` non-GET) without going through `ureq`. Test: `file_urls_are_served_from_disk_with_a_guessed_content_type`. Not done: directory listings, `HEAD`, `Content-Length`.

---

## v-qf6 — caps, class, public-only policy (scratchy QF6)

> Implement TODO.txt item QF6 [S] from the fetch/XHR DESIGN MEMO: wire honours `X-Wire-Max-Bytes`, `X-Wire-Timeout-Ms`, `X-Wire-Max-Redirects`; writes every duplicate response header (Set-Cookie) on its own line; reports the class of the address it connected to (`X-Wire-Target-Class`, per hop); `X-Wire-Policy: public-only` refuses non-public resolved addresses at every hop (403 + `X-Wire-Error: policy`, `file:` too); `X-Wire-Final-Url`; `X-Wire-Error: 1` on the synthesized 502; `X-Wire-Version: 2` on every response. Backwards compatible. Tests with loopback fixtures, a private test instance, docs.

Result: see wire/NOTES.md "The scratchy<->wire control channel". Tests: the QF6 block at the end of `src/main.rs`.

---

## v-qf14a — in-memory cookie jar (scratchy QF14, wire half)

> Implement the WIRE half of TODO.txt QF14 following QF-D2a exactly: `src/cookies.rs` (RFC 6265bis jar, partitioned by `X-Wire-Top-Site`, header-gated by `X-Wire-Jar: scratchy`, SameSite with `X-Wire-Nav`, limits struct, small embedded suffix list), per-hop store/send in the redirect loop (wire follows redirects itself when the jar is used), `Set-Cookie` stripped from answers when used, `document.cookie` control channel (`X-Wire-Cookie-Op get|set`), `X-Wire-Cookies: on|off`, `--no-cookies`, `X-Wire-Version: 3`, token hook only. Tests with loopback servers, a private instance on a temp socket.

Result: see wire/NOTES.md "The cookie jar". Added `X-Wire-Initiator-Site` (optional) so cross-site navigations can withhold Strict cookies. Not done: access token, real public suffix list, persistence.

## w-pool1 — policy and class headers on pooled connections

> W-POOL1 (security): `X-Wire-Policy: public-only` and class headers live in the resolver, which ureq skips on pooled connections. Implement (b) a separate agent/pool for public-only requests and (a) a bounded, expiring per-host:port class map filled by the resolver, correct for redirects and the jar path manual agent. Tests with a keep-alive loopback fixture; no protocol bump; no cargo install, own socket only.

Result: `Shared::public_agent`/`public_manual_agent`, `CLASS_MAP`, `reconcile_hops`; see wire/NOTES.md. Not done: intermediate pooled redirect hops on the non-jar path are not listed in `X-Wire-Hop-Classes` (ureq exposes no history); scheme is not part of the map key (resolver only sees host:port).

## qy39a — per-request User-Agent override

> QY39a (user decision 2026-11-17): let scratchy override wire's UA per request. The request's own `User-Agent` is forwarded; `X-Wire-User-Agent` (control header, never forwarded) takes precedence; neither -> wire's honest default. Validate (printable ASCII, max 512, no CR/LF -> 400 `X-Wire-Error: bad-user-agent`), apply on every hop, the jar path and the public-only pool. Additive `X-Wire-Features: ua` response header (version stays 3). Loopback-fixture tests, own socket only, no cargo install.

Result: `resolve_user_agent`, `write_bad_ua_response`, `WIRE_FEATURES` in `src/main.rs`; README header table; scratchy/NETWORKING.md line. Not done: the public-only test cannot reach a public host, so it checks the refusal and the shared request-builder path only; scratchy side is QY39b.

## qf19w1 — WebSocket pipe (scratchy QF19w1)

> QF19w1 (wire; design DECIDED 2026-11-17: wire is a pipe): a request with `X-Wire-Upgrade: websocket` goes through the same checks as any request (public-only W-POOL1-safe fresh connection, class headers, jar, UA, Origin/Sec-WebSocket-* passed), wire connects (TLS for wss), sends the handshake; on 101 returns the head and copies bytes both ways until either side closes; any other status is a buffered response, no redirects; not pooled; timeout covers the handshake only, no idle limit; `X-Wire-Features: ua,ws`. Tests over a std-only echo server; real check with a python3 client on my own socket; no cargo install.

Result: `src/ws.rs`, `src/ws_tests.rs`, NOTES.md "The WebSocket pipe". Direct deps `rustls`/`webpki-roots` (already locked). Not done: scratchy side (QF19w2).

## W-PERSIST1 filed — `--cookie-file` (scratchy design session QSTASH-D, docs only)

> do it in a PR so I can review it first

(The full design session's prompts are in scratchy/PROMPTS.md, "QSTASH-D".)

Result: wire/TODO.txt W-PERSIST1 (`--cookie-file PATH`, `X-Wire-Cookie-Op: clear-site`); design in scratchy/STASH.md "Cookies (wire)". No code.

## QWPT3 — `--mount` static origins (agent task)

> QWPT3 [M] wire static mount: `wire --mount http://web-platform.test:8000=DIR` (+ www/www1/www2 and the next port as aliases), `.headers` / `__dir__.headers`, generated `x.any.html` / `x.window.html` wrappers (META script/title/timeout/global), WebIDLParser.js alias, `.sub.` substitution, `--mount-override PATH=FILE`, `.py` -> 501, no socket, path traversal impossible and tested.

Result: `src/mount.rs` (+ unit tests), hook in `execute_with`, `mount_wire_tests` end-to-end test, README "Static mounts". Wrapper templates written from memory of wptserve (tools/ is not in the sparse WPT checkout); checked against R64 2.4. Not done: `.sub.headers`, `.asis`, Range, `__dir__` on wrappers (wptserve does not apply it either, as far as recalled).

## QREC1 — `--record` / `--replay` (agent task)

> QREC1 [M] wire --record DIR / --replay DIR (blobs by sha256 + zstd, index.jsonl, matching exact -> ignore-list -> n-th same path, ranges, 404 + X-Wire-Replay: miss, --replay-delay, WebSocket refused in replay). Tests in wire.

Result: `src/record.rs`, `src/record_tests.rs`, hooks in `execute_with` (new `execute_network` holds the old network half) and `handle_connection`, README section. New crates `zstd` (C libzstd compiled by the build, no sockets), `sha2` (pure Rust), `serde_json`; none opens a socket. Not done: Set-Cookie in replay (the jar path strips it before the recorder sees it), `tools/replay-rules.txt` content (QREC3), scratchy `--deterministic` (QREC2).

## W-CACHE1 — on-disk HTTP cache (agent task)

> Item: implement W-CACHE1 exactly as written in wire/TODO.txt (on-disk only RFC 9111 private cache, partitioned, streamed temp-file-then-rename storage, no in-memory index, eviction by mtime under --cache-size, Vary, revalidation, the six fetch `cache` modes via standard request headers, `public-only` policy check BEFORE lookup, never-stored cases, `X-Wire-Cache` response header stripped by scratchy, `X-Wire-Features: cache`, protocol version 4, the coordinator review notes (Authorization, immutable, shared dir, write errors) and the eviction check at a 256M cap). Also the scratchy side the item names: map fetch()'s `cache` option and reload/hard reload to the request headers, and strip `X-Wire-Cache`.

Result: `src/cache.rs` (+ `src/cache_tests.rs`), `--cache-dir` / `--cache-size` / `--no-cache`, `X-Wire-Version: 4`, README "HTTP cache", NOTES "The HTTP cache". scratchy: `cache_mode_headers` (net_policy.rs) for fetch's `cache`, `:refresh` = `max-age=0` on the document, `:refresh hard` = `no-cache` + `Pragma` on every request of the page (net.rs `set_hard_reload`); the WPT runner and the test wire pass `--no-cache`; the wire-starting tools scripts take `WIRE_ARGS`.

## W-CACHE2 — clearing the cache (agent task)

> Item: implement W-CACHE2 exactly as written in wire/TODO.txt (clear-site also removes the partition cache directory, Clear-Site-Data honoured, scratchy --forget-site confirmed by a test).

Result: `Cache::clear_partition`/`clear_origin`, `Jar::clear_host`, `apply_clear_site_data`, tests in `wire/src/cache_tests.rs` and `scratchy/src/tests/w_cache2_tests.rs`.
