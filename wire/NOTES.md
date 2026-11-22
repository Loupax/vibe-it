# wire — working notes

## What this is for

`scratchy` has a firm, long-standing design stance: it does no
networking of its own (see `scratchy/NOTES.md`, "rendering, not
networking"). Fetching has always been the caller's job — originally
exactly one caller, `snag`, which bundles a whole page (HTML +
stylesheets + images) into one self-contained file for `scratchy` to
parse. That model works great for "render one page I already fetched,"
and that's still `snag`'s entire job.

It breaks down the moment `scratchy` needs to react to something the
*user* does inside a running session — clicking a link, submitting a
form. Those need a real, individual HTTP request fired on demand, not a
whole-page bundle built ahead of time. `scratchy/TODO.txt`'s v65 item
bolted this onto `snag` directly (`snag --serve`, a line-protocol daemon
that takes a URL and returns a bundle) as a first cut, but a follow-up
design conversation (logged in `scratchy/TODO.txt`'s "TOP PRIORITY"
entry) decided that was the wrong shape going forward: `snag`'s own job
is bundling, not being a general-purpose HTTP client for `scratchy` to
drive interactively, and conflating the two was already showing seams
(a GET-only line protocol with no way to express a form's method/body/
headers).

`wire` is the fix: a tool with exactly one job, smaller than either of
its siblings, that owns real networking and nothing else. `scratchy`
still does zero networking. `snag` still does zero live interactivity.
`wire` is the one place an actual TCP/TLS connection to a real remote
server gets opened.

## Design choices

- **Proxy absolute-form request framing (`GET https://host/path
  HTTP/1.1`), not a bespoke protocol.** `wire` sits behind a bare Unix
  socket with no DNS/TLS-SNI of its own to infer a destination from —
  something has to tell it exactly where to connect. Standard HTTP
  proxy framing (RFC 7230 §5.3.2) already solves this with zero
  invented syntax, and it's symmetric with the daemon's own job
  description ("acts like an HTTP proxy, minus the TCP tunnel"). Using
  it also means `scratchy`'s own request-building code can reuse
  exactly the same shape for a GET link click and a GET/POST form
  submission — see `scratchy/TODO.txt`'s own "forms and link-clicks
  become the exact same code path" decision.

- **A normalized response, never a raw 1:1 relay — the one point
  actually debated before this tool existed at all** (see
  `scratchy/TODO.txt`'s own writeup of the conversation). The
  alternative — just forwarding the real server's bytes untouched —
  would leave chunked-decoding/redirect-following/gzip-decoding as new
  HTTP-client logic `scratchy` would have to grow itself, directly
  contradicting its "no networking" stance; the *transport* mess is
  real HTTP's problem to solve once, in one place, not scratchy's to
  re-solve per feature. So `wire` always reads the real response to
  completion — redirects already followed, body already decompressed —
  and re-frames it with a correct `Content-Length`, stripping
  `Transfer-Encoding`/`Content-Encoding` (both now meaningless: the
  body in hand is already the final, decoded bytes). Everything else
  about *what the bytes mean* (is this HTML? an image? something
  scratchy can't render at all?) stays scratchy's decision, made by
  reading exactly one header, `Content-Type` — `wire` doesn't try to
  guess or collapse different content kinds into one shape.

- **A real non-2xx is not a transport failure.** `ureq` represents
  these as two different error-shaped outcomes under the hood
  (`Err(Error::Status(code, response))` carries a genuine response;
  `Err(Error::Transport(e))` means no response ever came back at all),
  and `execute` treats them accordingly: a `Status` error goes through
  the *exact same* `normalize_response` path as a plain `Ok` — a 404
  forwards as a 404, not folded into some generic "it failed" shape —
  while only a real `Transport` failure (DNS, connect, TLS handshake,
  timeout) produces the synthesized `502`. Getting this distinction
  backwards would have been an easy, quiet bug: naively matching
  `Err(_) => synthetic_error_response(...)` compiles fine and passes a
  test that never happens to hit a real 404, which is exactly why one
  of this file's own tests (`a_real_4xx_from_upstream_forwards_as_itself_
  not_a_transport_error`) exists — to pin the distinction, not just the
  happy path.

- **One thread per connection, not a serial accept loop.** `snag
  --serve`'s own loop (serve one connection, then accept the next) was
  the obvious first precedent to reach for, but the TODO item that
  specified this daemon is explicit that concurrent, independent
  connections are an expected shape (a page with several forms/links'
  worth of future per-resource fetches, each its own one-shot
  connection) — not a hypothetical. `std::thread::spawn` per accepted
  connection, with `ureq::Agent` cloned into each (it's an `Arc` handle
  internally, so cloning is cheap and thread-safe), is the simplest
  thing that actually satisfies that requirement; nothing fancier (a
  thread pool, async I/O) is justified until a real page's connection
  count makes raw `thread::spawn` itself the bottleneck, which nothing
  here has shown yet.

- **Only `gzip` decompression, not `brotli` too.** Real CSS/browser
  precedent treats gzip as the dominant real-world `Content-Encoding`
  by a wide margin, and `ureq`'s `brotli` feature is a separate,
  additional dependency with its own compile cost — not justified
  before something real actually needs it, the same per-need-only
  philosophy this repo already applies everywhere else (see e.g.
  `scratchy/TODO.txt`'s own `box-shadow`/`gap`-percentage items for the
  same reasoning pattern). Easy to add later; `is_response_header_to_
  strip`/`normalize_response` don't care which encoding produced the
  decompressed bytes they're framing.

- **No request-side chunked support.** The one real client is
  `scratchy`, which always knows its own request body's length up front
  (a form's URL-encoded value string, built in memory, has a length
  the instant it exists) — so `Content-Length`-framed request bodies
  are the only case worth handling. Nothing currently needs a daemon
  that can *receive* a chunked request.

- **Response headers otherwise pass through verbatim, unfiltered.**
  `wire` doesn't try to decide which upstream headers matter —
  deciding that is explicitly scratchy's job (reading `Content-Type`
  and nothing else, per the TODO item's own contract). Stripping
  headers here beyond the three transport-framing ones above would be
  `wire` quietly making a product decision on scratchy's behalf.

- **`--print-socket` resolves and prints, never a second computation.**
  scratchy's own `--socket` is a strict opt-in with zero auto-discovery
  (see `scratchy/TODO.txt`'s own settled design) — something has to
  hand it a real path. Rather than document `wire`'s own `$XDG_RUNTIME_
  DIR`-or-`/tmp` fallback logic as a recipe for scripts to re-derive
  (wrong the instant that logic ever changes), `--print-socket` reuses
  the *exact* `path` value the real serving path would bind — same
  `unwrap_or_else(default_socket_path)` call, same explicit-`--socket`
  override honored in either argument order — and just prints it,
  exiting before any real work (bind, listen, build an HTTP client).
  The one thing this still can't answer is whether a `wire` is actually
  *running* at that path right now — it resolves the path, not the
  daemon's liveness; connecting (or not) is `scratchy`'s own job, with
  its own already-established fallback (`fetch_raw_via_wire`'s
  `Unreachable` case) for exactly that.

## What's deliberately not here yet

See `wire/TODO.txt` for the precise, maintained list — this section is
just the short version. `scratchy` now dials `wire` for its own live
link navigation and sub-resource fetching (v66/v67, both in a separate
session — see `scratchy/PROMPTS.md`), closing what used to be this
file's main open item. What's left, entirely on the *consuming* side:
form submission has no transport of its own yet (scratchy has no
`<button>`/focus/keyboard subsystem at all to drive one with), and
`snag --serve` itself hasn't been removed now that `wire` supersedes it
for live navigation — `snag`'s own, independent change to make.

## The scratchy<->wire control channel (QF6, protocol version 2)

Request headers scratchy MAY send (all optional; a request without them behaves
exactly as before; none is ever forwarded upstream: every `X-Wire-*` request
header is skipped in `is_request_header_to_skip`):

- `X-Wire-Max-Bytes: N` - the response body may be at most N bytes. Headers
  are already out once a body streams, so with this header wire buffers up to
  N+1 bytes (memory is bounded by the cap itself); over the cap the answer is
  `502` + `X-Wire-Error: too-large` and a short text/plain body.
- `X-Wire-Timeout-Ms: N` - overall upstream timeout (default 20 s).
- `X-Wire-Max-Redirects: N` - redirect limit (ureq default 5). A non-default
  value builds a one-off agent (no connection reuse for that request). Going
  over the limit is a transport error: the marked 502 below.
- `X-Wire-Policy: public-only` - refuse non-public targets: the resolver drops
  every non-public address BEFORE connecting (ureq connects only to the
  addresses the resolver returns, so DNS rebinding cannot swap it afterwards)
  and ureq resolves again for every redirect hop. Refusal is `403` +
  `X-Wire-Error: policy`; `file:` URLs get the same 403. Without the header
  wire connects to loopback/private/file targets (user decision: warn and
  allow, scratchy does the warning from the class header below).
  **Pooling (W-POOL1):** ureq calls the resolver only when it opens a NEW
  connection, so a pooled connection would skip the check. `public-only`
  requests therefore use their own agents (`Shared::public_agent` /
  `public_manual_agent`, own pools holding only connections the checking
  resolver approved); plain requests never share a pool with them. A request
  with a non-default `X-Wire-Max-Redirects` already uses a fresh one-off agent.
  The class headers survive reuse via `CLASS_MAP` (`host:port` -> class, filled
  by the resolver, max 1024 entries, 10 min TTL refreshed on lookup; a miss just
  omits the headers). Known limit: on the non-jar path ureq follows redirects
  itself and exposes no history, so an INTERMEDIATE redirect hop served from a
  pooled connection is not listed in `X-Wire-Hop-Classes` (first and last hop
  always are; the jar path follows redirects itself and is exact). The
  `public-only` check is not affected: its pool only has checked connections.

Response headers wire ALWAYS adds (upstream copies of `X-Wire-*` are dropped so
a server cannot spoof them):

- `X-Wire-Version` on every response, including errors and `file:`: `2` for
  this protocol, `3` since the cookie jar below (an old wire has none; a
  response without it, or below 2, makes scratchy turn page networking off
  and log "restart wire").
- `X-Wire-Final-Url` - the URL after redirects (the file URL for `file:`).
- `X-Wire-Target-Class` - the worst class among the resolved addresses of all
  hops (`public`, `loopback`, `private`, `link-local`, `cgnat`, `ula`,
  `multicast`, `unspecified`, `reserved`; `file` for files) and
  `X-Wire-Hop-Classes` - the comma list per hop. Same names as scratchy's
  `net_policy::ip_class`; `is_public_ip`/`ip_class` are copies, keep in sync.
  When a name resolves to a mix, the worst class is reported (conservative).
- `X-Wire-Error` on wire-made errors: `1` (transport failure; body keeps the
  old `wire: upstream request failed: ...` text), `too-large`, `policy`.
- Duplicate upstream headers (`Set-Cookie`) are now written one line each.

Per-request resolver state lives in a thread-local (`TRACKER`): each
connection has its own thread and ureq resolves on the calling thread.

## The cookie jar (QF14, protocol version 3)

Design: `scratchy/TODO.txt` QF-D2a (user, FINAL). Code: `src/cookies.rs` (the
jar, RFC 6265bis parsing/matching, its own unit tests) and `execute_with_jar`,
`cookie_control` in `src/main.rs`.

- **Header-gated, memory only.** The jar is ON unless wire runs with
  `--no-cookies`, but a request uses it only with `X-Wire-Jar: scratchy` (and
  not `X-Wire-Cookies: off`). One jar for all clients (`JAR_NAME`; a later
  `--profile` changes that one value). Without the header (snag, other clients)
  wire sends and stores nothing and behaves exactly as before, a client
  `Cookie` header included. The jar lives in a `Mutex` inside `Shared`; it is
  lost when wire restarts (QF-D7; `--cookie-file` is a later flag).
- **Partitioned (Total Cookie Protection).** Key = (partition, domain, path,
  name). `X-Wire-Top-Site` gives the partition (any URL or site; normalized to
  `scheme://registrable-domain`; missing = the request's own site). A third
  party sees only cookies it set under the same top site. `SameSite` as
  declared; unspecified = None. Strict: only when the request is same-site with
  the top site. Lax: also on a top-level navigation (`X-Wire-Nav: 1`, safe
  methods only). Optional `X-Wire-Initiator-Site` (with Nav): scratchy sends the
  destination as the top site for a navigation, which would make every
  navigation same-site; with the initiator, a cross-site navigation sends Lax
  but not Strict cookies. Sites compare scheme + registrable domain. The
  registrable domain uses a SMALL embedded suffix list (single labels plus
  ~100 common multi-label suffixes, no `psl` crate); a missing ccTLD second
  level would let siblings share a super cookie. One function
  (`is_public_suffix`) to replace.
- **Rules:** name/value, Expires (lenient RFC date parser), Max-Age (wins),
  400 day cap, Domain (IP hosts host-only, public suffix rejected unless equal
  to the host), Path (default path), Secure (set only from a secure context;
  sent over https or loopback), HttpOnly, SameSite, `Partitioned` ignored,
  `__Secure-`/`__Host-` prefixes (case-insensitive), malformed lines ignored
  (no `=`, empty name, control characters), plain http cannot overwrite a
  Secure cookie. Limits in ONE struct (`Limits`): 4096 bytes name+value, 1024
  per attribute value, 180 per (partition, domain), 3000 total; eviction =
  expired first, then least recently accessed.
- **Request path.** With the jar, wire follows redirects ITSELF (a second
  agent with redirects off): before every hop (initial and each redirect) the
  `Cookie` header is computed for that hop's URL (a client-supplied `Cookie` is
  dropped: the jar is authoritative), after every hop every `Set-Cookie` line is
  stored, and `Set-Cookie` lines are stripped from what scratchy gets back (the
  jar belongs to wire). 303 and POST+301/302 become GET; 307/308 keep method and
  body. `X-Wire-Final-Url`/hop classes behave as before; the timeout is per hop
  on this path.
- **Control channel for `document.cookie`:** `X-Wire-Cookie-Op: get` +
  `X-Wire-Cookie-Url: <page url>` + `X-Wire-Top-Site` + `X-Wire-Jar: scratchy`
  answers `200 text/plain` `name=value; name2=value2` (HttpOnly hidden; SameSite
  by the top site), no upstream request. `set` takes the `document.cookie = ...`
  string as the BODY and answers `204` (rejected cookies also answer 204, as a
  browser silently ignores them; script cannot create or overwrite HttpOnly,
  Secure only from https/loopback). Bad input: `400` + `X-Wire-Error:
  cookie-control`. Disabled jar: `404` + `X-Wire-Cookies: off`. Hook for the
  access token the user has not decided on: `control_authorized` (always true
  today).
- **Detection:** `X-Wire-Version: 3` = cookie-capable wire. Responses carry
  `X-Wire-Cookies: on` when the jar was used for that request (control channel
  included) and `X-Wire-Cookies: off` on every response when wire runs with
  `--no-cookies` (the jar header is then ignored and `Set-Cookie` passes
  through). A version >= 3 without the header on a request that did not ask for
  the jar just means "available, not used".
- **Logging:** names and values never reach a log line; `cookies::log_line`
  prints host and counts only, and only under `WIRE_TIMING=1`.

## The HTTP cache (W-CACHE1, protocol version 4, `X-Wire-Features: cache`)

Design: wire/TODO.txt W-CACHE1 (user decisions 2026-11-22: on disk only, on by
default, standard request headers). Code: `src/cache.rs`; hooked into
`execute_network`, which computes the jar decision and the effective
User-Agent, then hands `cache::handle` a closure that does the upstream call
(`send_upstream`, the old jar / non-jar paths unchanged).

- **The writer in between.** `cache::handle` does not touch ureq. It passes a
  `CacheWriter` as the response writer: it collects the response head as wire
  writes it, parses it (status, headers, `X-Wire-Final-Url`, class headers,
  `X-Wire-Error`), decides whether the answer is storable and from then on
  copies body bytes to the client and to a temp file. To wire's own
  conditional a `304` is swallowed (never reaches the client) and `handle`
  answers from the rewritten entry. Unsafe methods use the writer only to see
  the status for invalidation.
- **Completeness (W-MEDIA2 for the cache).** `stream_response` records the
  upstream `Content-Length` in the thread-local `BODY_INFO` (ureq drops it when
  it undid gzip) and `copy_body` / the max-bytes path mark a read error as
  `truncated`. The temp file is renamed only when the body is complete.
- **File format.** `WIRECACHE1 <20-digit body length>\n` (32 bytes, patched in
  place at the end, before the rename), one JSON metadata line, the body. A
  file whose length does not match, or that does not parse, is deleted on
  lookup (renames are atomic, so only an old format or a damaged disk gets
  there).
- **No index.** `Cache::approx` counts bytes (exact after a scan, plus own
  stores) only to decide when to scan; the evictor thread scans at start-up,
  when a store takes it over the limit and once a minute while over. Scan items
  are 64 bytes each (mtime, size, partition index, name as 32 raw bytes), freed
  after the pass; temp files older than an hour are deleted. Shared directory:
  every vanished file or directory during a scan or lookup is a miss / skipped,
  never an error; creating a temp file retries once when an eviction removed the
  empty partition directory meanwhile.
- **Write errors.** Any error creating or writing the temp file (full disk,
  read-only, a file where the partition directory should be) drops the store,
  logs ONE warning line (again only after a store has worked), and the response
  is still served from the network.
- **`X-Wire-Cache`** comes from the thread-local `CACHE_HDR` that
  `write_status_and_headers` prints (like `X-Wire-Cookies`): `bypass` at the
  start of every request while the cache is on, `miss` before a network call
  through the cache, `hit` / `stale` / `revalidated` when served from the store.
- **Not done (v1):** caching redirects or 3xx (needs W-POOL2's own redirect
  following), several variants of one URL (a `Vary` mismatch replaces the
  entry, as Chromium does), serving stale on upstream errors, updating an entry
  from a `HEAD` or from a 304 to the client's own conditional, `If-Range`
  (such requests skip the store). W-CACHE2 (clearing) builds on the partition
  directories.

## The WebSocket pipe (QF19w1, `X-Wire-Features: ws`)

Design: scratchy/TODO.txt QF19 (user, 2026-11-17: "wire as a pipe"). Code:
`src/ws.rs`, routed from `handle_connection` when a request carries
`X-Wire-Upgrade` (never for the cookie control channel; without the header an
Upgrade request is an ordinary request, as before). The framing lives in
scratchy (QF19w2); wire only does DNS/TCP/TLS/policy/cookies and copies bytes.

- **Own connection, never a pool.** `ureq` cannot hand back the raw stream of
  a `101`, so ws.rs resolves with `TrackingResolver` (public-only drops
  non-public addresses before connecting, W-POOL1-safe: no pooled connection can
  skip it), connects with `TcpStream`, and for `wss:` (and `https:`) wraps it in
  `rustls` (webpki roots, the same stack ureq uses; `rustls`/`webpki-roots` are
  direct dependencies now, same locked versions, ring provider, no new crate in
  the lock). `ws:`/`wss:` are mapped to `http:`/`https:` for the jar.
- **Handshake.** Same checks as a fetch: 403 policy, bad `X-Wire-User-Agent`
  400, jar `Cookie` computed for the handshake URL (client `Cookie` dropped,
  jar authoritative) and every `Set-Cookie` of the answer stored (also for a
  non-101) and stripped from what scratchy gets. `Origin` and `Sec-WebSocket-*`
  pass through; `Host`, `User-Agent` (X-Wire-User-Agent > own > default),
  `Connection: Upgrade`, `Upgrade: websocket` are set by wire; `X-Wire-*` never
  forwarded and upstream `X-Wire-*` dropped. Errors: 400 + `X-Wire-Error:
  bad-upgrade` (value not `websocket`, not GET, other scheme), 502 `1` for
  connect/TLS/handshake failure.
- **Not 101:** buffered answer (Content-Length, chunked decoded, or to EOF; cap
  `X-Wire-Max-Bytes` else 4 MiB, over it `502 too-large`), status and headers
  as upstream sent them minus `Content-Length`/`Transfer-Encoding`, no redirect
  following (`X-Wire-Max-Redirects` is ignored), connection closed.
- **Pipe.** Plain `ws:`: two threads, blocking copies, 16 KiB buffer; a slow
  reader blocks the writer (no unbounded buffering). `wss:`: rustls buffers
  plaintext, so one thread owns the TLS stream and polls (10 ms socket read
  timeout) while a second thread reads the client into a bounded channel (8
  chunks); idle cost is ~100 wakeups/s per open wss socket and client->upstream
  latency up to 10 ms. Closing: client EOF -> `close_notify`/write shutdown
  upstream, then up to 5 s grace for upstream to finish; upstream EOF/error ->
  both sockets shut down at once.
- **Caps:** `X-Wire-Timeout-Ms` (default 20 s) bounds connect + handshake only;
  no idle limit. Test coverage in `src/ws_tests.rs`, incl. a wss echo over a
  throwaway openssl-made certificate (skipped when `openssl` is missing).

## W-CACHE2: clearing
`Cache::clear_partition` is one `remove_dir_all` of `<sha256(top site)>/`; `clear_origin` scans one partition and
unlinks entries whose stored URL has the origin (skips `.tmp-` files). `apply_clear_site_data` (main.rs) runs per jar hop and
for the final non-jar response, before the body streams; `storable` refuses an answer with `Clear-Site-Data`. `Jar::clear_host`
removes the host's cookies, its parent-domain cookies and its subdomains' in that partition. Under `--no-cookies`, clear-site
still clears the cache partition. wire did not handle `Clear-Site-Data` before this item.
