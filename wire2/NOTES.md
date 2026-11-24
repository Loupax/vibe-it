# wire2 — implementation notes

Design choices and measurements behind the code. The design itself is [`SPEC.md`](SPEC.md); this
file explains how the code meets it and where later steps plug in. Newest step last.

## W2-1 plain forwarding proxy (2026-11-23)

What exists: the spec's "zero `--filter` flags" behaviour. `CONNECT host:port` becomes an opaque
spliced tunnel; plain `http://` requests in absolute form are forwarded, one per browser connection,
answered with `Connection: close`. Nothing is decrypted and no CA exists.

### Binary name: `wire2`

The spec calls the binary `proxy` and says the CLI is the contract, not the name. A binary called
`proxy` in `~/.cargo/bin` is too generic (it shadows anything else of that name on `PATH`), and the
repository's tooling installs a crate by its directory name (`coordinator/ci_then_push.sh --install
wire2`). So the binary is `wire2`, with exactly the spec's flags. The systemd units (later) can still
be called `proxy.service` if the user prefers.

### Async (tokio), not a thread per connection

- The performance section wants >= 2,000 concurrent flows and < 100 MB RSS at 500 flows. A thread
  per direction would be 4,000 threads with their stacks; tokio tasks cost a few KiB each.
- splice(2) fits an event loop well: a direction waits for readiness (`Conn::ready`), then runs
  `splice` through `try_io`, which clears tokio's readiness flag only on `EAGAIN`. Because a
  direction always empties its pipe before reading again, an `EAGAIN` can only come from the socket,
  never from the pipe, so readiness tracking stays exact.
- Later steps need timeouts (`--stage-timeout`, `--connect-timeout`), cancellation (revocation, the
  per-flow cancel function), reconnect with backoff and many concurrent filter connections; all of
  that is cheap with tasks and `select!`, and painful with threads.
- Multi-threaded runtime (one worker per core by default, `TOKIO_WORKER_THREADS` overrides). The
  only shared state is the flow registry (a `Mutex<HashMap>` held only to insert/remove/snapshot)
  and the pipe pool (a `Mutex<Vec>` held only to push/pop). No lock is ever held across I/O.

### Tunnels: splice with a shared pipe pool

- Each direction moves bytes socket -> pipe -> socket inside the kernel, no userspace copy. splice
  returns the byte count, which feeds the per-direction counters for free.
- Pipes are 1 MiB (`F_SETPIPE_SZ`; the kernel may refuse, then the default 64 KiB stays). 256 KiB
  pipes measured 59 % of direct throughput (one session), 1 MiB 87-96 %.
- A direction takes a pipe from a pool only when its socket is readable and gives it back once the
  pipe is drained, so an idle flow holds no pipe at all (two sockets, nothing else). This matters
  because the kernel accounts pipe capacity per user (`fs.pipe-user-pages-soft`, 64 MiB by default):
  2,000 flows with two private pipes each would blow that budget and the kernel would silently shrink
  new pipes to 8 KiB. The pool keeps at most 32 idle pipes.
- Closing: an EOF on one side is passed on as a FIN (`shutdown(SHUT_WR)`); the other direction then
  gets 5 s (`HALF_CLOSE_GRACE`) to finish what is in flight, after which the whole flow is closed
  (SPEC: "when any stage or either end closes, the proxy closes the rest of the flow"). An error on
  either direction (reset, broken pipe) closes both at once. Browsers close their side when the
  server closes, so in practice flows end immediately; the grace only bounds idle half-open flows.
- Bytes the browser sends right behind the `CONNECT` head (an early ClientHello) are forwarded first.

### Plain `http://`

- The request head is parsed (httparse, 64 KiB limit, 30 s to arrive, else 431/408). Origin-form
  (`GET /`) and `https://` absolute-form requests get 400 (this is a proxy; https goes through
  CONNECT); so do URLs with credentials and unparsable hosts.
- Upstream request: origin-form target, `Host` rewritten from the URL (RFC 9112 3.2.2), hop-by-hop
  headers removed (`Connection` and the headers it names, `Keep-Alive`, `TE`, `Trailer`,
  `Proxy-Connection`, `Proxy-Authorization`), every `Proxy-*` header removed (the proxy sets none on
  this path yet), `Upgrade` removed unless the request is an upgrade, `Connection: close` added.
- Bodies are streamed in both directions with a 64 KiB buffer, framing unchanged: Content-Length is
  counted, chunked bodies pass through byte for byte while a small scanner (`ChunkScanner`) finds
  their end (extensions and trailers included), responses without length run until the origin
  closes. A Content-Length next to Transfer-Encoding is dropped (smuggling rule); conflicting
  Content-Length values are an error. The proxy does not wait for the origin to close after a framed
  response, so origins that keep the connection open work.
- The request body is sent concurrently with waiting for the response (an origin may answer early).
  1xx interim responses (100 Continue, 103 Early Hints) are relayed. `101` after an upgrade request
  turns the connection into a byte pipe (userspace copy, same closing rules as tunnels); browsers
  send `ws://` through `CONNECT`, so this path is for other clients.
- The response keeps its status (3xx included), Content-Encoding and framing; hop-by-hop and
  `Proxy-*` headers are removed and `Connection: close` added.

### Upstream connector

- The proxy resolves names itself (`getaddrinfo` on tokio's blocking pool), then dials with a
  simplified Happy Eyeballs (RFC 8305): address families alternate, the next address starts after
  250 ms or as soon as the previous attempt fails, the first connection wins. Without it, a host
  whose IPv6 address is unreachable from here held a browser for the whole `--dial-timeout` (found
  with headless Firefox: `*.neverssl.com`).
- DNS failure, refused or unreachable -> 502; nothing within `--dial-timeout` (DNS included) -> 504.
  Logged as `WARN flow=<n> host=<h> port=<p> upstream error, answering <code>: <cause>`.

### Host normalisation

`host::normalise_host`: brackets removed from IPv6 literals (and the address canonicalised, so
`[0:0::1]` is `::1`), one trailing dot removed, IPv4 kept, names through UTS 46 (`idna` crate:
case folding, mapping, punycode). Every log line and the registry use the normalised host; W2-F's
host globs and W2-2's `--no-decrypt` will match against the same string.

### Flow registry and access log

- Entry per flow (tunnel or plain-http connection): id, method, host, port, decrypted (always no
  for now), chain (empty until W2-F), open time, bytes up/down (atomics), status, cancel token.
- `Drop` of the flow handle removes the entry and writes the access line, so there is exactly one
  line per flow however it ends:
  `access flow=7 CONNECT host=example.com port=443 decrypted=no chain=- status=tunnel up=1929 down=6524 duration=0.083s`
  `status` is `tunnel`, the HTTP status, or `-` (the browser left before any answer). `up`/`down`
  count every byte written to the upstream / to the browser, heads included.
- `SIGUSR1` logs `registry: N open flow(s), M pooled pipe(s)` and one `registry flow=...` line per
  flow with its age and byte counts. `SIGTERM`/`SIGINT` cancel all flows and exit.
- Logs go to stderr, one `write(2)` per line (no lock), UTC timestamps. `--log-level debug` adds
  every request line, dial attempt and the reason each flow ended.

### Security bits in this step

- Refuses to start as uid 0 (euid or uid), also inside a user namespace (`unshare -r`).
- `--listen` on a non-loopback address needs `--allow-remote` (and then warns). `unix:<path>`
  listeners get mode 0600; a stale socket file is replaced, a live one is an error.
- Unknown flags are errors (exit 2). `--filter`, `--hub`, `--connect-timeout`, `--stage-timeout`
  (W2-F) are recognised and refused with "not supported yet" (`--no-decrypt`, `--ca-dir`: W2-2).
- `RLIMIT_NOFILE` is raised to the hard limit at start (two fds per flow).
- No exec anywhere; the systemd unit with `SystemCallFilter=~execve execveat` comes with the units.

### Where the later steps plug in

- W2-2 (TLS interception): in `proxy::tunnel`, after the host is normalised and before the `200`,
  decide per CONNECT (http filter claims host, not `--no-decrypt`). A decrypted tunnel then serves
  HTTP/1.1 requests through the same head/body code as `proxy::plain` (`copy_body`, framing,
  hop-by-hop), with the upstream connector opening verified TLS.
- W2-F (filters): `flow::FlowInfo::chain` is computed once at open. The tunnel-layer chain runs
  inside `upstream::dial`'s replacement (CONNECT through each tunnel filter, then the server); the
  http-layer chain sits between the parsed request and the upstream connector in `proxy::plain`.
  `Proxy-*` stripping already happens at that boundary; the proxy will add its own `Proxy-Flow`
  there. The registry's cancel token is the "cancel function that closes every connection".
- The proxy id is generated at start (logged on the `listening on` line) for `Proxy-Id`.

### Measurements (2026-11-23, Ryzen 7 5700X, Fedora, loopback, release build)

All under `flock /tmp/scratchy-perf.lock`, load average 0.5-1.5 (desktop session running).
`cargo test --release --test w2_1_perf -- --ignored --nocapture --test-threads 1`

| What | Result | Target |
|---|---|---|
| Tunnel vs direct, 4 GiB per run, median of 9 runs | 87.1 %, 88.7 %, 94.9 %, 96.3 % (four sessions; direct ~6.5-6.9 GiB/s, tunnel 5.8-6.5 GiB/s) | >= 95 % |
| RSS idle / 500 open tunnels / 2,000 open tunnels | 3.9 MB / 7.6 MB / 17.8 MB | < 100 MB at 500 |
| 2,000 tunnels echoing 64 KiB at once | all done in 0.13 s, RSS 18.0 MB | >= 2,000 without degradation |
| 2 GiB through a tunnel / plain http (Content-Length) | RSS +4 KiB / +4 KiB; 5.3 / 2.8 GiB/s; first bytes before the origin finished | flat memory |

The throughput ratio is noisy on this machine (individual runs of the direct baseline range from 5.6
to 7.4 GiB/s); the median is around the 95 % target, not reliably above it. Worker count
(`TOKIO_WORKER_THREADS` 1, 2, default 16) made no measurable difference.

### Known gaps (proposed as TODO items in the W2-1 PR)

- No `--resolve`-style test hook, so tests use `localhost` instead of `www.example.test`. (Done in
  W2-2 as W2-1a: the TEST-ONLY `--test-resolve`.)
- The access log counts heads in `up`/`down`; the spec does not say either way.
- Browser checks were headless only (Chrome `--dump-dom`, Firefox `--screenshot`); video seeking and
  an interactive WebSocket chat need a real session.

## W2-2 TLS interception (2026-11-23)

What exists: the machinery for the spec's lazy decryption, and the hook W2-F plugs into. Without
filters nothing calls that hook, so a production `wire2` still never decrypts and never creates a CA
(the W2-1 "no CA file" test stays green, and `no_claim_no_ca` checks it again with `--no-decrypt`).

### The decision: `decrypt::Policy`

- One question per CONNECT, `Policy::decide(host)` on the normalised host: is there an http-layer
  claim matching it, and is it not on `--no-decrypt`? `--no-decrypt` wins; that is logged once per
  host (`never decrypting <host>: on --no-decrypt <glob> (http filter <name> skipped for it)`), and
  so is the first decrypt of each host (`WARN decrypting <host> for the first time (claimed by <name>)`).
- Claims come from `Policy::add_http_claim(filter, hosts)`. W2-F calls it when an http filter
  completes its handshake (and will add a matching remove on disconnect). The first call creates the
  CA, so "the CA is created the first time an http filter completes its handshake" holds by
  construction. Claims are read under an `RwLock` held only for the glob scan, never across I/O.
- The decision is taken once, in `proxy::tunnel`, before anything is answered; a later claim affects
  new CONNECTs only.
- Globs (`host::normalise_glob`, `host::glob_match`): labels without `*` are normalised like hosts
  (lowercase, punycode, no trailing dot, canonical IPs); `*` matches any run of characters, dots
  included, so `*.example.com` matches `a.b.example.com` but not `example.com` (SPEC). W2-F's filter
  `hosts` lists will use the same two functions.

### The CA: `ca::Ca`

- Directory: `--ca-dir`, default `$XDG_DATA_HOME/proxy` (else `~/.local/share/proxy`) exactly as the
  spec says, although the binary is called `wire2`. Created with mode 0700.
- Files: `ca-key.pem` (ECDSA P-256, PKCS#8, mode 0600, written to a temporary name and renamed) and
  `ca.pem` (the one file the browser imports). A later start loads both; a key readable by group or
  others is refused with a message saying how to fix it. Creation logs a loud `WARN CREATED A NEW CA`.
- CA certificate: `CN=wire2 local CA <8 hex digits>` (per install, so two installs never collide in a
  trust store), `O=wire2 (local filtering proxy)`, CA with path length 0, key usages certSign + CRL
  sign, 10 years, random 128-bit serial. Not name-constrained (it must sign whatever hosts a filter
  claims).
- Reloading uses rcgen's `x509-parser` feature (`CertificateParams::from_ca_cert_pem`) and re-signs
  an in-memory copy with the same subject and key; leaves signed by it verify against the original
  `ca.pem` (`ca_reused_across_restarts`).
- Leaves: minted in memory on first use per host, never written to disk: one SAN (DNS name, or IP for
  an IP literal), EKU serverAuth, AKI, 30 days valid from yesterday, random serial, a fresh P-256 key
  each. Cached as a ready `rustls::ServerConfig` per host (re-minted after 7 days; the cache is cleared
  if it reaches 4,096 hosts). The server sends the leaf only, ALPN `http/1.1` only (SPEC: v1 speaks
  HTTP/1.1 to the browser for decrypted hosts).

### The decrypted path: `proxy::decrypted`

- Answers `200 Connection established` itself, then terminates TLS (30 s for the handshake; bytes
  that arrived behind the CONNECT head are fed to the handshake first). A browser that closes before
  sending a ClientHello (Chrome's speculative preconnects) is logged at debug only; a real handshake
  failure is logged at info with a hint to check that the browser trusts `ca.pem`.
- Then serves HTTP/1.1 requests on that connection with keep-alive toward the browser (120 s idle).
  Targets may be origin-form, `https://` absolute-form or `OPTIONS *`. A request for another host
  (`Host` header or absolute URL) gets `421 Misdirected Request`: the tunnel's certificate and upstream
  are for the CONNECT host only.
- Each request goes through `proxy::forward`, the same function plain `http://` requests use. It is
  the one place every request and response passes through; W2-F's http chain goes at its top. Same
  rules as W2-1: hop-by-hop and every `Proxy-*` header removed, `Host` from the URL, framing kept
  byte for byte (Content-Length, chunked with extensions, until-close), 1xx relayed, `101` turns the
  connection into a byte pipe (that last part is W2-3's to test).
- Upstream: `upstream::dial_tls` = the same resolver/Happy-Eyeballs dial as W2-1, then rustls with SNI,
  ALPN `http/1.1`, verified against the system roots. Roots come from `rustls-native-certs` 0.8:
  `SSL_CERT_FILE` / `SSL_CERT_DIR` when set, otherwise the distribution's store (on this Fedora:
  /etc/pki/tls/certs, 118 roots). Loaded once, on the first decrypted request. Never downgraded, no
  option to skip verification. A TLS failure answers `502` (body `wire2: host:port: upstream tls:
  ...`) and logs `WARN flow=<n> host=<h> port=<p> upstream error, answering 502: upstream tls: invalid
  peer certificate: ...`; DNS/refused -> 502 and dial or handshake timeout -> 504 as before.
- One upstream connection per request, sent with `Connection: close` (like the plain path). Reusing
  upstream connections is a follow-up (proposed W2-2a): each decrypted request costs a TCP + TLS
  handshake to the origin.
- Response keep-alive: the browser connection is kept only if the browser did not ask to close, the
  response's end is visible from its framing (not until-close), and the whole request body was read.
  Otherwise the proxy answers with `Connection: close` and closes after the response.
- Logs: one line per request, then the flow line when the connection ends:
  `access flow=10 req=1 GET https://example.com/ status=200 up=661 down=669 duration=0.061s`
  `access flow=10 CONNECT host=example.com port=443 decrypted=yes chain=- status=tunnel up=1169 down=2452 duration=0.194s`
  Byte counts of a decrypted flow are plaintext HTTP (heads + bodies), not TLS records.

### Pipelined response bodies (why decrypted throughput needed work)

First measurement: 47 % of direct. A profile showed every sample of the proxy on one worker thread:
decrypting the upstream and encrypting toward the browser ran in one task, so one core did both.
Response bodies now go through `proxy::pipelined_body`: a spawned task reads (decrypts) and frames the
body and hands 64 KiB buffers through a bounded channel (4 deep, buffers recycled) to the flow's task,
which writes (encrypts). That alone did not help: tokio puts a task woken by another into the waking
worker's LIFO slot, which other workers cannot steal, so the two halves still ran on one thread.
`Builder::disable_lifo_slot` would fix that globally but is `tokio_unstable`; instead the reading half
yields once per chunk (wakes itself, returns `Pending`), which requeues it on the stealable queue and
lets an idle worker take it. With that, decryption and encryption run on two cores (profile: samples
spread over 4 workers, ~2 cores busy). `Coalesce` makes each hop carry up to 64 KiB (tokio-rustls
returns one 16 KiB record per read). Plain `http://` uses the same path and went from 2.5-2.8 to
3.1 GiB/s. What remains is mostly kernel loopback TCP (three hops instead of one, and this machine's
nftables hooks run on each) and the two AES-GCM passes.

### Test-only flags

Both are absent from `--help`, start with `--test-`, and log a `WARN TEST-ONLY ...` line at start.
They are real flags rather than `cfg(test)` because the integration tests run the release binary.

- `--test-resolve GLOB=IP` (W2-1a, filed from #669 and done here): the upstream connector maps
  matching names to IP instead of asking DNS, so tests use `www.example.test` like the spec.
- `--test-decrypt GLOB` (REMOVED in W2-F, replaced by the test-only pass-decrypt filter): registered a
  claim named `test-decrypt` at startup, standing in for an
  http-layer filter until W2-F brings filters and the test-only pass-decrypt filter. It goes through
  `add_http_claim` like a real filter will (creates the CA, obeys `--no-decrypt`). Once W2-F's
  pass-decrypt exists, this flag can go (proposed in the PR).
- Upstream roots in tests are the test root through `SSL_CERT_FILE` (an existing convention of
  `rustls-native-certs`, not a hook of ours).

### Tests (tests/w2_2_*.rs; unit tests in ca.rs, decrypt.rs, host.rs, config.rs, proxy.rs)

- `decrypted_requests_and_cert_chain`: CA created at startup with a 0600 key and the loud log line;
  a client trusting only the origin's root rejects the proxy's leaf (UnknownIssuer); a client trusting
  only `ca.pem` accepts it (leaf only, ALPN http/1.1 although the client offers h2 first); then on one
  kept-alive connection: GET (origin sees origin-form, the tunnel's Host, no `Proxy-Flow` or
  `Keep-Alive`), POST with Content-Length (300 kB streamed), POST chunked, a chunked response with a
  chunk extension passed through byte for byte with `Connection: close`; per-request log lines with
  full https URLs; the flow line says `decrypted=yes`; first-decrypt logged once.
- `unclaimed_and_no_decrypt_hosts_stay_tunnels`: an unclaimed host and a claimed host on
  `--no-decrypt WWW.Example.Test.` both stay opaque (the origin's own certificate reaches the client);
  "never decrypting" logged.
- `bad_upstream_certificate_is_502`: an origin signed by an untrusted root, and a trusted certificate
  for the wrong name, each give 502 with the cause logged.
- `no_claim_no_ca`: www.example.test via `--test-resolve`, `--no-decrypt` given, no claim: tunnel,
  and no file at all under the proxy's data dir.
- `ca_reused_across_restarts`: same `--ca-dir` twice: no new CA, leaves chain to the first `ca.pem`.
- `w2_2_large`: 2 GiB through the decrypted path (flat memory, first bytes before the origin
  finished). `w2_2_perf` (ignored): decrypted vs direct TLS.

### Measurements (2026-11-23, Ryzen 7 5700X, Fedora, loopback, release build)

Under `flock /tmp/scratchy-perf.lock`, load average 0.7-1.9 (desktop session running).
`cargo test --release --test w2_2_perf -- --ignored --nocapture --test-threads 1`. Direct = a rustls
client fetching from a rustls HTTPS origin (origin on its own thread and runtime, so the baseline does
not depend on where tokio places two tasks: with both in one runtime it jumped between 1.7 and 3.1
GiB/s); decrypted = the same client through the proxy with `--test-decrypt`.

| What | Result | Target |
|---|---|---|
| Decrypted vs direct TLS, 2 GiB per run, median of 7 | 53.7 %, 51.9 % (direct 2.24-2.31 GiB/s, decrypted 1.20 GiB/s); before pipelining 47.1 % | >= 50 % |
| 2 GiB decrypted download (Content-Length) | RSS +0.2-0.6 MB, 1.07-1.45 GiB/s, first bytes before the origin finished | flat memory |
| Proxy RSS during decrypted runs | 7-8 MB | |

The spec's target is for pass-decrypt, which adds one filter stage (W2-F); these numbers are the
proxy's own decrypted path without it.

### Browser check (headless Chrome, 2026-11-23)

Chrome 156 headless, `HOME` and `--user-data-dir` under `.qwork/browser/` (a fresh NSS db there with
only the test CA added by `certutil`; the user's profiles and trust store untouched, nothing logged
in), `--proxy-server` to a wire2 started with `--ca-dir .qwork/browser/ca --test-decrypt example.com`.
`--dump-dom https://example.com/` gave `<title>Example Domain</title>`; the log showed
`req=1 GET https://example.com/ status=200` and `req=2 GET https://example.com/s.js status=200` on one
decrypted flow, upstream verified against the 118 system roots. `https://www.iana.org/...` (not
claimed) loaded through an opaque tunnel (`decrypted=no status=tunnel`), as did Chrome's own
background connections.

### Known gaps (proposed as TODO items in the W2-2 PR)

- No upstream connection reuse for decrypted requests (one TCP + TLS handshake per request).
- Request bodies (uploads) are copied in one task, so they run at single-core decrypt+encrypt speed.
- WebSockets inside decrypted tunnels: done in W2-3 (below).
- `--test-decrypt` should be removed once W2-F's pass-decrypt exists.

## W2-3 WebSockets inside decrypted tunnels (2026-11-23)

What exists: nothing new in the design. An upgrade request in a decrypted tunnel already went through
`proxy::forward` -> `exchange`, the code plain `http://` uses: `Upgrade` and `Connection: upgrade` kept
(every other hop-by-hop header and every `Proxy-*` removed), `Sec-WebSocket-*` untouched, a `101` is
relayed (with `Connection: upgrade`) and both directions become `copy_until_eof` (no frame parsing;
message-level filtering is W2-F's). A non-101 answer to an upgrade request is an ordinary response
(`Connection: close`, body framing kept). After the pipe ends the function returns `Next::Abort`, so the
decrypted keep-alive connection is never read for another request; the request line in the log says
`status=101` with the bytes of that exchange, then the flow line follows.

### Bug found by the tests: no flush on the until-close copy

`copy_body` in `UntilClose` mode wrote each chunk and flushed only at EOF. With a TLS stream on the
destination (the browser side of a decrypted tunnel, or the upstream TLS) tokio-rustls keeps the
encrypted tail of a `write_all` that did not fit in the socket buffer until the next write or a flush.
An upgraded connection can go quiet right after such a write, so the last ~16-130 KiB of a large message
sat in the proxy until the peer sent something else (a 32 MiB message stalled, about 1 run in 8; with the
fix removed the test fails again within 8 runs, with it 70+ runs passed). The loop now flushes after
every chunk (a no-op on plain TCP). The same loop serves until-close response bodies, where it costs
nothing measurable. The test helper `ws_write` had the same missing flush on its own TLS stream.

### Behaviour worth knowing

- Closing: when the origin closes, the browser side gets a TLS `close_notify` and the proxy waits up to
  5 s (`HALF_CLOSE_GRACE`, as for tunnels) for the browser to close its side; browsers do at once, so
  the access lines come immediately in practice (the tests drop their client like a browser).
- Chrome fragments large WebSocket messages itself (a 2 MiB send arrived as ~128 KiB frames); the byte
  copy does not care, a test server must reassemble.
- Memory: 2 x 32 MiB echoed through the decrypted pipe moved the proxy's RSS by 0.2-0.4 MB.

### Tests (tests/w2_3_websocket.rs)

All use `--test-decrypt`, `--test-resolve` and a wss origin with the test root:
- `decrypted_websocket_echo_and_close`: a plain request first on the same connection, then the upgrade;
  the origin sees `Upgrade`/`Connection: Upgrade`, `Sec-WebSocket-Key/Version/Protocol/Extensions`,
  the tunnel's Host and no `Proxy-Flow`; the browser sees 101 with Accept, Protocol and the origin's own
  header; text, small binary and 200 kB binary echoes; close handshake; EOF; another request on the dead
  connection gets nothing and the origin sees no third connection; the log has `status=101` and both byte
  counts (> 200 kB) and one flow line.
- `server_initiated_close_ends_flow`, `browser_hangup_ends_flow`: either side closing ends the flow.
- `non_101_answer_is_a_normal_response`: 401 with body to an upgrade request, `Connection: close`.
- `large_message_flat_memory`: two 32 MiB messages, RSS growth < 16 MiB.

ws:// through plain http (`plain_http_websocket_upgrade`) and wss:// through an opaque tunnel
(`websocket_echo_through_tunnel`) are the W2-1 tests, still green.

### Browser check (headless Chrome 156, 2026-11-23)

HOME, `--user-data-dir` and the NSS db under `.qwork/browser/` (only the proxy's `ca.pem` imported; the
user's profiles and trust store untouched; nothing logged in). A local Python wss server (a throwaway
cert chain signed by a test root, given to wire2 as `SSL_CERT_FILE`) serves a page that opens
`wss://www.example.test:PORT/ws` through `wire2 --test-decrypt --test-resolve`, sends a text message and a
2 MiB binary message, checks both echoes and closes. Result `RESULT OK`; log:
`access flow=23 req=1 GET https://www.example.test:18943/ws status=101 up=2098021 down=2097310 duration=0.134s`.
Direct (no proxy) Chrome behaved identically, which is how the test server's own fragmentation bug was
found. No measurements: nothing looked slow (2 MiB each way in 0.13 s through the proxy).

## W2-F filters (2026-11-23)

What exists: the spec's filter chain. `--filter unix:<path>` (repeatable, chain order; any other
scheme is a startup error), `--hub`, `--connect-timeout`, `--stage-timeout`; the handshake; routing per
flow; the hub with its `Proxy-Flow` check; tunnel and http layers; lazy decryption driven by http
filters; the failure table; socket trust and peer credentials; the reference filter `pass` and example
systemd units. `--test-decrypt` is gone; the tests use the test-only pass-decrypt filter.

### Crate layout: a library and two binaries

`src/lib.rs` holds every module; `src/main.rs` (`wire2`) and `src/bin/pass.rs` (`wire2-pass`) are
three-line `main`s. Why a second binary in this crate and not a workspace member: pass needs the same
head parser (`http.rs`), splice pump (`splice.rs`) and logger as the proxy; a separate crate would need
those moved into a third, shared crate, and the repository's tooling installs one crate per directory
(`ci_then_push.sh --install wire2` now installs both binaries). The library is also what lets the tests
run pass-decrypt from pass's own code (`wire2::pass::serve` with `layer: http`) without a test-only
flag in the shipped binary. The binary is `wire2-pass`, not `pass`: `pass` is the standard password
manager's name, and `~/.cargo/bin/pass` would shadow it. Its handshake name is `pass`, as the spec says.

### Filters and their supervisors (`filter.rs`)

- One supervisor task per `--filter` owns a `watch` with the filter's state: `Starting` (first
  handshake still being tried, at most `--connect-timeout`), `Up(caps)`, or `Down { last }` where `last`
  holds the capabilities of the last good handshake (`None`: never shook hands, or the last answer was
  rejected).
- Handshake: a new connection, `GET /hello` with `Host: proxy`, `Proxy-Id` (random per start, also the
  first half of every `Proxy-Flow`), `Proxy-Filter-Version: 1`, `Proxy-Hub: unix:<hub>`, then a `200`
  with JSON (body up to 64 KiB; Content-Length, chunked or until close). Validated: `name` non-empty, at
  most 64 characters without spaces, commas or control characters (it appears in log lines and in
  `chain=` lists), unique among filters that are up; `version` exactly 1; `layer` `tunnel` or `http`;
  `hosts` a list of globs (normalised like `--no-decrypt`, default `*`); `fail` `open` or `closed`
  (default closed); unknown fields ignored (forward compatible). An http filter's hosts go to
  `Policy::add_http_claim` before it counts as up, so the CA exists before the first decrypted CONNECT;
  `remove_http_claim` runs when it goes down.
- Retry: socket missing or silent -> retried with exponential backoff (100 ms doubling to 5 s) forever.
  At startup a filter that has not answered within `--connect-timeout` becomes `Down { last: None }`
  and an ERROR line says that every host is refused until its handshake succeeds. A rejected answer
  (non-200, bad JSON, wrong version/layer, duplicate name) is logged with the reason and also leaves the
  filter down with no known capabilities; it is retried the same way (a fixed filter is picked up).
- Down detection is passive: the data path calls `Filter::mark_down` when connecting fails or a
  connection closes before response headers; the supervisor then redials and repeats the handshake.
  This is also how a restarted filter is noticed: pass closes traffic from a proxy it has not shaken
  hands with, the proxy marks it down and asks again (one request gets 502 in that case).
- Flows arriving while a filter is still `Starting` wait for it (bounded by `--connect-timeout`), so the
  first requests after startup are routed by real capabilities instead of being refused.

### Routing (`Filters::route`, once per flow)

For each filter in `--filter` order: up and its hosts match -> added to the flow's tunnel or http
chain; down with known capabilities whose hosts match -> refuse the flow with 503 if it fails closed,
skip it if it fails open; down with unknown capabilities (before any handshake, or rejected) -> refuse,
whatever the host (SPEC: "Before any handshake this applies to all hosts"). A browser CONNECT that is not
decrypted drops the http chain (those filters never see it), and a down http filter cannot refuse a
CONNECT to a host on `--no-decrypt` (it would never have seen it). The chain is stored in the flow and
logged (`chain=` lists http filters, then tunnel filters, in the order a request meets them). Each hop
re-checks the filter's state: one that went down since the flow opened is skipped (fail open) or
answers 503 (fail closed). Plain `http://` requests to hosts on `--no-decrypt` still go through http
filters: nothing needs decrypting there.

### The path of a request

- `proxy::forward` (every parsed browser request) -> `http_from(stage 0)`. A filter stage gets a new
  Unix connection with the request head in absolute form plus `Proxy-Flow: <id>/<flow>`, the body
  streamed by the same `exchange` code as before (now taking the next hop's head and a byte-counter
  pair). Past the last http stage, `connect_from(stage 0)` opens the server connection: a tunnel stage
  gets `CONNECT host:port` with `Proxy-Flow`; a 2xx makes that connection the pipe; past the last tunnel
  stage the proxy dials the server; for decrypted flows TLS (verified, `upstream::tls`) runs over
  whatever `connect_from` returned, so tunnel filters only ever see the encrypted bytes. Plain `http://`
  requests therefore pass tunnel filters too (as `CONNECT host:80`), so a tunnel-layer blocker covers
  them.
- An undecrypted browser CONNECT is `connect_from(stage 0)` and then the same splice as before, now
  between the browser and the first filter's connection.
- The hub (`hub.rs`, mode 0600, created only when there are filters) accepts one request per
  connection, checks the peer's uid, then `Proxy-Flow`: issued by this proxy (`Proxy-Id` half), a flow
  that is still open, and a flow whose stage at that layer is waiting for a request (`Route::expect`
  arms it before a stage is sent its request, `Route::take` consumes it, a guard disarms it when the
  stage is over). A flow has at most one request in flight per layer (decrypted requests are served one
  after the other and each opens its own upstream connection), so one slot per layer suffices and a
  filter cannot forward the same request twice. Anything else is a 403 naming the reason. Then every
  `Proxy-*` header is stripped and the hub runs the next stage: `connect_from(stage + 1)` for a CONNECT
  (answering `200` and splicing), `http_from(stage + 1)` for anything else, with the filter's connection
  in the browser's place. A filter may change anything in a request except its destination: a changed
  scheme, host or port is a 502 (tunnel filters and the decrypt decision were chosen for the original).
- `Proxy-*` stripping: the browser's request (`build_request`, as before), every request arriving at
  the hub, and every response (`strip_hop_by_hop`). The proxy adds `Proxy-Flow` only on what it sends to
  a filter; `Proxy-Id` only on handshakes. A browser's own `GET /hello`, with or without a forged
  `Proxy-Id`, is ordinary traffic.
- Byte counts in the access log are the browser-facing hop's only (hub hops use scratch counters), so
  they mean the same with or without filters (W2-1c's question about heads stands).
- When the browser-facing flow ends, its cancel token is cancelled, which ends whatever the hub still
  runs for it.

### Failure handling

| Situation | Implementation |
|---|---|
| Socket missing at startup | `Starting` up to `--connect-timeout` (flows wait), then down, redialled every <= 5 s |
| Handshake rejected | WARN with the reason, down with no capabilities (refuses every host), retried |
| Down, fail closed | 503 `wire2: filter <name> is down (fail closed), so <host> is refused`; a CONNECT gets the same 503 |
| Down, fail open | skipped (DEBUG line) |
| Connect fails | marked down, then the rule above applies to this very request (503 or skip) |
| Closes before response headers | 502, marked down (WARN), redialled from 100 ms, handshake repeated |
| No headers within `--stage-timeout` | 504, filter stays up. The clock stops while the request is downstream of the stage (the hub reports `Downstream` when the stage forwards, `Answered` on the first byte back), so a slow origin behind a prompt filter is not a filter timeout |
| Malformed HTTP from a filter | 502, connection dropped, ERROR line |

A refused CONNECT (non-2xx) from a tunnel filter is answered to the browser with that status. An error
the hub produces (an upstream 502/504, a refusal) travels back through the filters like any response.

### Security bits

- Socket trust: before connecting to a filter and before creating the hub, the socket's directory must
  not be group/world-writable and must be owned by our euid; at startup a filter directory that exists
  and fails the check, or a hub directory that fails it, is a startup error (exit 1). A missing directory
  is only a filter that is down. Peer credentials: a filter's listening end must belong to our uid or
  root (a systemd-created socket), and so must every peer that connects to the hub.
- The hub's 0600 mode plus the uid check means only the proxy's own user can forward into it. Within
  that user nothing is protected (SPEC non-goal); `pass` remembers the hub per `Proxy-Id` so traffic
  goes back to the proxy that issued it.

### pass (`src/pass.rs`, binary `wire2-pass`)

Handshake answer `{"name":"pass","version":1,"layer":"tunnel","hosts":["*"],"fail":"open"}`, no flag
that changes it. Socket: systemd socket activation (`LISTEN_PID`/`LISTEN_FDS`, fd 3), else `--socket
PATH` created under umask 0177 and chmod 0600 (a stale socket is replaced, a live one refused).
Traffic: it reads one request head (keeping the raw bytes), connects to the hub of the proxy that
issued the `Proxy-Flow`, writes the head and whatever followed it, then splices both ways with the
proxy's splice pump; it never parses or buffers a body, so tunnels, upgrades and both body framings are
all the same byte pipe. `--verbose` logs every request; SIGTERM exits 0.

Units in `systemd/`: `pass.socket` (`ListenStream=%t/pass.sock`, `SocketMode=0600`, `Accept=no`),
`pass.service`, and `wire2.service` (`Wants=`/`After=pass.socket`, `SystemCallFilter=~execve
execveat`, `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome=read-only` with the CA directory
writable, `PrivateTmp`, `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`). Checked with
`systemd-analyze --user verify` (only complaint: the binaries are not installed yet). Not run as
installed units here (the coordinator installs binaries); the spec's execve check under the unit is
manual.

### Unix socket buffers

The first tunnel-through-pass measurement was 77 % of direct. Unix stream sockets get
`net.core.wmem_default` (208 KiB) of buffer, loopback TCP autotunes to megabytes, so each splice through
a filter hop moved at most ~208 KiB per wakeup. `conn::tune_unix` asks for 1 MiB SO_SNDBUF/SO_RCVBUF on
every filter and hub connection (both ends; the kernel caps it at `wmem_max`/`rmem_max`): 89-93 %. A
quick sweep: 512 KiB and 1 MiB alike, 2 MiB and 4 MiB worse (76-83 %). The hub's http hops use owned
socket halves instead of `tokio::io::split` (no lock between body directions).

### Acceptance criteria (SPEC), tests

| Criterion | Test | Result |
|---|---|---|
| No filters: curl through, no CA | w2_1_tunnel, w2_2_decrypt `no_claim_no_ca` (unchanged) | pass |
| Only pass: browsing works, no CA, one access line per tunnel | w2_f_filters `pass_tunnels_and_plain_http_no_ca`; headless Chrome below | pass |
| pass-decrypt creates the CA; new tunnels decrypted (filter sees `GET https://`), others tunnels, earlier tunnels untouched | `http_claim_creates_ca_new_tunnels_only` (a fake filter that re-handshakes from tunnel to http layer) and every w2_2/w2_3 test through pass-decrypt | pass |
| `--no-decrypt` wins | `no_decrypt_wins_over_pass_decrypt`, w2_2 `unclaimed_and_no_decrypt_hosts_stay_tunnels` | pass |
| http filter answers itself (451 + HTML), https and http | `http_filter_answers_itself` | pass |
| http filter rewrites a chunked body, valid framing | `http_filter_rewrites_chunked_body` (plain, and decrypted keep-alive with a second request) | pass |
| tunnel filter 403 refuses; closing ends the tunnel within 1 s | `tunnel_filter_refuses_and_revokes` | pass |
| non-matching filter never connected | `unmatched_filter_never_sees_the_flow` | pass |
| 2 GB through pass and pass-decrypt, flat memory, early first bytes | w2_f_large (tunnel and plain http through pass; proxy and pass RSS), w2_2_large (pass-decrypt; proxy and test-process RSS) | pass |
| WebSocket echo through pass and pass-decrypt | `websocket_through_pass` (tunnel and ws://), w2_3_websocket (all through pass-decrypt) | pass |
| Host normalisation, `[::1]:443` | w2_1 tests (unchanged; routing uses the same normalised host) | pass |
| plain http to a non-80 port | w2_1_http (unchanged; every W2-F test uses non-80 ports through filters) | pass |
| Upstream 502/504, bad certificate 502 | w2_1_tunnel, w2_2 `bad_upstream_certificate_is_502` (now through pass-decrypt: the 502 comes back through the filter) | pass |
| Killing a fail-closed filter -> 503, restart restores; fail-open down skipped | `fail_closed_503_recovery_and_fail_open_skip`, `before_any_handshake_everything_is_refused` | pass |
| Stage timeout 504 | `stage_timeout_504` (http and tunnel layer; a slow origin behind a prompt filter is not a timeout) | pass |
| Proxy-* from browser or filter never reach the next stage or the server | `proxy_headers_are_stripped` | pass |
| A page's `GET /hello` is traffic | `page_hello_is_traffic` (through the real pass code) | pass |
| Browser Proxy-Id/Proxy-Flow stripped; hub refuses unknown Proxy-Flow | `proxy_headers_are_stripped`, `hub_refuses_unknown_flows` | pass |
| Refuses root, world-writable socket dir, non-loopback `--listen` | `untrusted_socket_dirs_are_refused` (/tmp, root's /run, hub in /tmp), w2_1_cli | pass (root: the W2-1 check, not re-tested) |
| execve denied under the unit | manual | not run |
| Performance targets | w2_f_perf, w2_2_perf, below | see below |

Also: `pass_socket_activation` (pass under `systemd-socket-activate`, LISTEN_FDS), malformed/closed
filter -> 502 with re-handshake (`malformed_and_closed_filter_502`), unit tests for the handshake
validation, socket-dir checks, the hub's expectation slots and the stage clock (paused tokio time).

### Measurements (2026-11-23, Ryzen 7 5700X, Fedora, loopback, release build)

Each under `flock /tmp/scratchy-perf.lock`, load average 1.2-2.6 at the time (desktop session and other
agents' builds). `cargo test --release --test w2_f_perf -- --ignored --nocapture --test-threads 1` and
`--test w2_2_perf`.

| Metric | Result | Target |
|---|---|---|
| Added time to response headers, one pass stage | p50 +0.16 ms, p99 +0.19 ms (no filter p99 0.24 ms, pass p99 0.43 ms; 5,000 requests each) | p99 < 2 ms |
| Undecrypted tunnel through pass vs direct, 4 GiB x 9 | median 89.1 % (5188 vs 5824 MiB/s); earlier 1 GiB x 9 runs with the same build: 89.5, 91.1, 78.5 % | >= 90 % |
| Tunnel with no filters vs direct (same session) | 91.2 % | >= 95 % |
| Decrypted through pass-decrypt vs direct TLS, 2 GiB x 7 | 51.4 % (1.12 vs 2.17 GiB/s); 1 GiB runs 49.0, 49.8 % | >= 50 % |
| Proxy RSS at 500 / 2,000 tunnels through pass | 13.6 MB / 38.4 MB (pass 8.8 / 23.4 MB); 2,000 x 64 KiB echoed at once in 0.14 s | < 100 MB |
| 2 GiB through pass (tunnel / plain http) | proxy RSS +36 KiB / +312 KiB, pass +0 | flat |
| 2 GiB through pass-decrypt | proxy +1.0 MB, test process (filter, origin, client) +1.2 MB | flat |

The two throughput ratios sit at their targets, not clearly above: the direct baseline on this machine
swings between 5.3 and 7.5 GiB/s from run to run, and the no-filter tunnel measured 91 % in the same
session (W2-1 recorded 87-96 %). Comparing the two ratios (each against its own direct runs), one pass stage
costs about 2 points.
Decrypted through pass-decrypt was 52-54 % without the filter stage (W2-2); the stage costs 2-3 points.

### Browser check (headless Chrome 156, 2026-11-23)

Throwaway HOME and profile under `.qwork/`, nothing logged in, no CA imported: `wire2-pass --socket`
and `wire2 --filter unix:.../pass.sock`, `--dump-dom https://example.com/` -> `<title>Example
Domain</title>`; 23 flows (Chrome's own background connections included), every access line
`decrypted=no chain=pass`, pass logged 23 requests, no data directory (no CA) was created.

### Known gaps

- A filter restart is noticed only when the proxy next uses it (connect fails, or pass closes traffic
  from an unknown proxy). A filter whose hosts match no traffic keeps its old capabilities in the proxy
  until then; with systemd socket activation the socket never goes away, so changed capabilities show
  up after the first request that reaches the new process. A periodic re-handshake would close this.
- A duplicate name (or any rejected handshake) leaves that filter down with no capabilities, which
  blocks every host (fail closed); the log says why.
- (Fixed by W2-2a, below: one upstream connection per decrypted request meant one CONNECT per request.)
- The access log's byte counts include heads (W2-1c, open question, unchanged); the CA directory default
  is still `$XDG_DATA_HOME/proxy` (W2-2d, open question, unchanged).

## W2-2a upstream connection reuse (2026-11-23)

### Design

- One slot per flow: `FlowInfo::pool` holds the idle origin connection (TLS halves) of a decrypted flow.
  A browser connection's requests are sequential (HTTP/1.1, no pipelining), so one slot is the whole pool.
  The slot lives in the flow, not the filter or hub path: a request that went through http filters and
  came back through the hub reaches `http_from`'s origin step with the same `FlowInfo` and uses the same slot.
  Plain `http://` flows and upgrades never use it (`Hop::Origin { pool: false }`); plain requests still go
  out with `Connection: close`.
- Request side: `build_request` no longer adds `Connection: close` for decrypted flows (HTTP/1.1 default
  is keep-alive). Plain http and upgrades are as before.
- Reuse conditions (`exchange`, `origin_ok`): the origin answered HTTP/1.1 without `Connection: close`, the
  response end is visible from its framing (Content-Length, chunked, no body; not until-close), nothing
  was read past the body, and the whole request body had been sent. Then the two halves come back out of
  `pipelined_body` (the reader task now returns its reader) and are parked, whether or not the browser
  connection itself stays open (the hub's filter-facing leg is always `Connection: close`, the origin leg
  is independent).
- Before reuse, `still_open` polls the parked connection once with a zero timeout: data, EOF or an error
  while idle means the origin closed it, and a fresh connection is dialled without sending anything (so even
  a POST is safe then). A dead connection that looks alive (FIN not arrived yet) is the race the retry
  covers: the request was written, no response byte came (EOF or I/O error on the first read).
  `resendable(req)` (GET, HEAD, OPTIONS, no body) gives `Next::Stale` and one more attempt on a new connection
  (the old one is dropped); anything else gives `502` "upstream closed the kept connection; <method>
  request not resent" and the dead connection is dropped; the browser connection itself carries on (when a
  filter sits in the chain, the 502 reaches the browser as an ordinary response). A fresh connection is
  never retried.
- Tunnel filters: each origin connection is still dialled through `connect_from`, so a tunnel filter sees one
  `CONNECT` per upstream connection (four requests, one CONNECT in the test). A filter closing its pipe ends
  the parked connection (the idle check sees it, or a request in flight fails); the next request dials again
  and the filter is asked again, so it can refuse (`403` reaches the browser) or allow.
- Access log: unchanged, one `req=N` line per request plus the flow line.
- Not done: no expiry of a parked connection other than the browser connection's 120 s idle and the origin's
  own close; no pool shared across browser connections (Chrome opens up to 6 per host, each gets its own
  upstream connection, as before).

### Measurements (2026-11-23, release builds, loopback, `flock /tmp/scratchy-perf.lock`, load average 2.9-3.7)

`cargo test --release --test w2_2a_reuse -- --ignored --nocapture perf` (`W2_CONNS`, `W2_REQS`, `W2_RUNS`;
`WIRE2_TEST_BIN` picks the proxy binary: before = origin/main e68312f9). Small GETs through pass-decrypt to a
keep-alive TLS origin, 15 runs, median:

| Load | Before (fresh TCP+TLS per request) | After | Origin TCP connections (15 runs) |
|---|---|---|---|
| 50 requests on 1 decrypted connection | 2097 ms (41.9 ms/request) | 113 ms (2.25 ms/request) | 750 -> 15 |
| 50 requests on each of 6 connections | 2145 ms (7.2 ms/request) | 156 ms (0.52 ms/request) | 4500 -> 90 |

The "before" figure is dominated by something near 40 ms per fresh upstream connection (not isolated; a
delayed-ACK stall in the handshake flights is the suspicion), so read the ratios (~19x, ~14x) as an upper
bound for a real network; the point is that the handshake cost is gone.

### Browser check (headless Chrome 156, 2026-11-23)

Throwaway HOME, NSS db and `--user-data-dir` under `.qwork/browser/` (test CA only, nothing logged in).
A local HTTPS origin (python, HTTP/1.1 keep-alive, a test root as the proxy's `SSL_CERT_FILE`) serving a
page with 30 images and 10 scripts as `page.example.test:18443`; wire2 + pass-decrypt claiming
`*.example.test` (the ignored test `serve_for_browser`). `--dump-dom` gave `<title>loaded 30</title>`; 41
requests answered 200 over 12 decrypted flows (7 flows carried 2-6 requests each); the origin accepted 12
TCP connections (before: one per request, 41).

### Tests (tests/w2_2a_reuse.rs: 9, plus 2 ignored)

Reuse of one origin connection over 7 requests (CL, chunked), HEAD without body keeps framing, CL/chunked/PUT
request bodies (300 KB), origin `Connection: close` and idle close give a new connection for GET and POST,
dead-after-read gives GET retried once / POST 502 and not resent, tunnel filter: one CONNECT for four requests
and a revocation (`403`) after it closed, close then redial, plain http still `Connection: close`. Six of the
nine fail with the pool switched off (checked).

## W2-F1 / W2-FLK1: idle filter restarts

Design: a supervisor of an up filter repeats the handshake every `--filter-check` (default 2s). A held keep-alive
handshake connection was rejected: the reference `pass`, the test fakes and any third-party filter answer the
handshake with `Connection: close` (the spec does not require keep-alive), so a close would be no signal.
Outcomes of a check: same answer, nothing; connect/answer failure, `mark_down` (then the existing 100 ms-5 s redial,
fail-closed 503 / fail-open skip as before, no request fails); different answer, the capabilities (and the http
decrypt claim) are replaced for NEW flows only (a flow's chain is fixed when it opens); a wrong answer, down and
fail-closed for all hosts (as at startup). A filter that is down and comes back with other hosts is simply the
normal redial. Cost: one tiny request per filter per 2s; a socket-activated filter that is not running is started
by the check (same as by traffic). A restart faster than the interval, with the same answer, is invisible, which
is fine: the data path still marks down on a failed request as before.
Test harness: `proxy_with` passes `--filter-check 1h` first so tests that count handshakes stay exact; the new
tests pass `100ms`. New tests: idle_restart_is_noticed_without_traffic, idle_pass_restart_first_request_succeeds,
changed_handshake_applies_to_new_flows (the first two fail with the check at 1h, checked).

W2-FLK1: `malformed_and_closed_filter_502` and `stage_timeout_504` (same race, it flaked once in a full run) now
`wait_log` (5 s bound) instead of reading the log once after the response. w2_f_filters x20: 20/20 green (twice,
before and after fixing stage_timeout_504, which failed once in between), full suite x3 green.

### W2-F1: additions beyond SPEC.md (SPEC.md is the user's document and is not edited)

- Flag row: `--filter-check` | 2s | How often an up filter is re-handshaken, so a restart or changed capabilities are noticed without traffic. New capabilities apply to new flows only.
- Failure-handling row: Filter goes away while idle | Noticed by the periodic handshake (`--filter-check`): marked down and redialled as above, with no request failing. A changed answer replaces the capabilities for new flows only.

## W2-HUBCLOSE + W2-2b (2026-11-23)

W2-HUBCLOSE. The hub served a filter's request with `keep_alive = false`, so every response on a filter leg said
`Connection: close`, and the stage before it (which strips hop-by-hop headers) could not tell a proxy-made 502 from an
ordinary answer; the browser got keep-alive either way. Now the hub passes `keep_alive = true`: the leg's response says
`Connection: close` only when the proxy made it (`answer`: 502/503/504/...) or its framing needs a close (read until
EOF, request said close). `exchange` on a filter hop reads that token: a filter leg that closes ends the browser
connection too (`Connection: close` to the browser, then the close). A filter that answers itself with
`Connection: close` (as the fake does) also closes the browser connection, which is its right. Without a filter in
the chain nothing changed (`answer` already closed). Test: `proxy_made_error_through_a_filter_closes_the_browser_connection`
(keep-alive through the chain twice, then a 502 from an origin sending garbage: `Connection: close`, EOF; fails on
the old code, checked). `w2_2a_reuse::stale_reused_connection_retries_get_never_post` used to expect the browser
connection to survive a proxy-made 502; it now expects the close and a new connection (the new rule).

W2-2b. `exchange` owns the upstream write half in its `send` future and hands it back with the result. A request
body that is chunked or Content-Length >= 256 KiB (`pipelines`) goes through `pipelined_upload`: this task reads
(for a decrypted flow: decrypts) the browser's body and frames it, a spawned task owning the write half writes
(encrypts) to the origin, joined by the same `ChanWriter` / `PIPE_DEPTH` channel as responses (memory flat; the
writer is aborted if the exchange ends first, in which case the origin connection is not reused). A write failure
is a `Dst` error as before. Small bodies keep the single-task copy (a spawn costs more than it saves).
Tests: tests/w2_2b_upload.rs: `uploads_arrive_intact_length_and_chunked` (10 B, 100 KB, 5 MiB+7, 300 KB and a 3 MiB
chunked body on one browser connection, every byte checked at the origin, connection still usable),
`large_upload_decrypted_flat_memory` (2 GiB generated, every byte checked; proxy RSS 6428 -> 8436 KiB, test process
incl. pass-decrypt 7128 -> 10336 KiB, ~650 MiB/s), ignored `perf_upload_throughput_vs_direct`.
Measured (flock perf lock, 2 GiB x 5 runs, medians, load average ~3-4 from other work, 16:03 and 16:04):
before (origin/main) direct 813 MiB/s, decrypted 781 MiB/s = 96.1 %; after direct 764, decrypted 706 = 92.4 %.
No measurable gain: the uploader (one TLS-encrypting client task in the same test process) is the bottleneck for
both legs, so the proxy was already keeping up; the ratio difference is within the run-to-run noise of the direct
baseline. The benefit is structural (decrypt and encrypt overlap on two cores, so a faster uploader than this test
client is no longer capped by one proxy core) and is not shown by this harness; a client in its own process would.

## W2-2e shared upstream pool and idle expiry (2026-11-23)

Replaces W2-2a's one-slot-per-flow (`FlowInfo::pool`, removed) with `pool.rs`, one `UpPool` in `Ctx`.

- Key: (host, port, names of the flow's tunnel-filter chain). A connection parked by a flow is taken by the next
  request of any decrypted flow with the same key, LIFO (freshest first). The chain is in the key because a tunnel
  filter decided over one CONNECT for the whole connection: a connection that went through chain X is never given
  to a flow with chain Y (test: a filter's hosts change, NEW flows have another chain, and the two chains keep to
  their own connections). A dial that skipped a down fail-open tunnel filter (`FlowInfo::skipped`, set by
  `connect_from` at any stage, reset when a dial starts) is not parked, so a connection never reaches a flow
  whose chain names a filter the connection did not go through. Each upstream connection still went through
  exactly one CONNECT per tunnel filter of its chain, however many flows used it.
- Hub pipe lifetime: the hub's splice behind a tunnel filter used to end when the flow that dialled it ended
  (`flow.cancel`). That would kill a parked connection another flow is about to use, so each tunnel dial now has
  its own token (`Route::expect` returns it, `Route::take` hands it to the hub); the dialler holds a `DropGuard`
  with the connection (in `Dialed`, then in the pool entry) and the pipe ends when the connection is dropped, by
  the pool (expiry, cap, dead on take) or at the end of the exchange. A filter closing its pipe ends exactly that
  connection: the take-time `still_open` check sees it and the request goes to another parked connection or dials
  again, through the filters again (so a filter can refuse).
- Idle expiry: `--pool-idle`, default 30 s. Why: below the usual origin keep-alive timeouts (nginx 75 s,
  Go net/http and Caddy 90-120 s, HAProxy 50 s; Apache's 5 s is shorter, and the stale-race retry covers that),
  so the proxy normally closes first and idle sockets do not pile up; a page's burst of requests is far
  shorter. A janitor task sweeps every idle/5 (2 s at most), so a connection lives idle..idle+tick; `take` also
  refuses an entry older than the idle time, so the lag never matters for reuse. Closing is a TLS close_notify
  then the FIN (the test origin sees a clean EOF), done in a spawned task so no request waits for it.
  `--pool-idle 0` turns reuse off. The browser connection's own 120 s idle is unchanged and now independent.
- Caps: `--pool-per-host` default 6 (what a browser opens per host), `--pool-total` default 64. Over a cap the
  OLDEST idle connection is closed (per key first, then overall). 20 parallel browser connections need 20
  upstream connections, 6 stay parked and 14 are closed cleanly (test).
- Rules kept: W2-2a's idle check (`still_open`) on every take, the stale-reused retry (idempotent no-body
  GET/HEAD/OPTIONS once on a FRESH connection, never another parked one; never a POST), W2-HUBCLOSE's close
  rules (the exchange code is untouched).
- Beyond SPEC.md (SPEC.md not edited): three flags, `--pool-idle D` (30s), `--pool-per-host N` (6), `--pool-total N`
  (64). SIGUSR1 lists the pool since W2-2f.

### Measurements (2026-11-23 16:19, release, loopback, under `flock /tmp/scratchy-perf.lock`, load average 9.6 -> 8.9: other agents were running, so absolute times are noisy)

`cargo test --release --test w2_2a_reuse -- --ignored --nocapture perf` (the W2-2a harness: a new set of browser
connections per run, 15 runs, median; before = the wire2 installed by the coordinator after #688, copied to
.qwork; after = this branch). TCP = connections the origin accepted over the 15 runs.

| Load | Before | After |
|---|---|---|
| 50 requests on 1 connection | 157.8 ms, 17 TCP | 106.8 ms, 1 TCP |
| 50 requests on each of 6 parallel connections | 162.4 ms, 90 TCP | 116.3 ms, 6 TCP |
| 5 requests on each of 6 parallel connections (short-lived browser connections) | 92.8 ms, 91 TCP | 51.4 ms, 6 TCP |

The gain comes from browser connections that open and close: their origin connections are reused by the next
connections instead of being dialled (TCP + TLS) again.

### Browser check (headless Chrome, google-chrome, 2026-11-23)

Throwaway HOME / NSS db / `--user-data-dir` under `.qwork/browser/` (test CA only, nothing logged in); the W2-2a
page (30 images, 10 scripts, local python HTTPS keep-alive origin `page.example.test:18443`) through wire2 +
pass-decrypt (`serve_for_browser`). `--dump-dom`: `<title>loaded 30</title>`; the proxy logged 42 per-request
access lines over 12 decrypted flows; the origin accepted 6 TCP connections (W2-2a: 12; before W2-2a: 41).

### Tests (tests/w2_2e_pool.rs: 8)

Sequential sharing across browser connections (also after the first closed), idle expiry with clean close and a
fresh dial, `--pool-idle 0`, per-host cap under 20 parallel connections, overall cap, a tunnel filter closing
exactly the first connection it carried (`Action::CloseFirstAfter`, new in tests/common/filters.rs), different
chains never share, a parked connection outliving the flow that dialled it through a filter (fails if the hub
pipe is tied to the flow's token again, checked). Seven of the eight fail on the old binary (checked with
`WIRE2_TEST_BIN`); the filter-close test passes there too, it guards behaviour that must not change.

## W2-2f SIGUSR1 pool dump (2026-11-23)

SIGUSR1 now also logs the pool (after the filters and the flow registry), at info, one line per key, keys sorted
by host, port, chain:

    pool: 3 idle upstream connection(s) in 2 key(s), caps 6 per key and 64 total, idle expiry 30.000s
    pool host=www.example.test port=8443 chain=gate idle=2 ages=0.412s,0.215s

Ages are oldest first (the order they would be closed in); `chain=-` is the empty chain. The lock is held only
to copy the ages. Beyond SPEC.md (not edited). Closes the "SIGUSR1 does not list pooled connections" gap of W2-2e.
Test: `sigusr1_dumps_the_pool_one_line_per_key` in tests/w2_2e_pool.rs (two keys: chain [gate] with 2 parked
connections, empty chain with 1; waits for the log lines, bounded 5 s).

## W2-1b: tunnel throughput re-measured (2026-11-24)

Quiet machine (idle load about 0.3; the test run itself lifts the 1-minute figure to 1.5-2.7, no other agent was
measuring), kernel 7.2.11, release build, 4 GiB per run, median of 9 runs per session, each session under
`flock /tmp/scratchy-perf.lock`, tests `perf_tunnel_throughput_vs_direct` (w2_1_perf) and
`perf_tunnel_through_pass_vs_direct` (w2_f_perf), same commit as main at 6ce9faea. 11 sessions each:

| What | Per-session ratios, sorted | Median | Target |
|---|---|---|---|
| Tunnel, no filter, vs direct | 88.7 91.4 91.7 91.8 94.2 94.9 96.9 98.6 98.9 99.0 99.8 % (tunnel 6.0-6.4 GiB/s, direct 6.0-6.9) | 94.9 % | >= 95 % |
| Tunnel via wire2-pass vs direct | 75.3 77.6 77.7 78.6 79.0 79.4 79.9 80.8 82.8 85.6 88.4 % (pass 4.9-5.4 GiB/s) | 79.4 % | >= 90 % |

No-filter: at the target within noise (the direct baseline itself swings 6.0-6.9 GiB/s between sessions, so single
ratios range 89-100 %); the tunnel absolute speed is steady at 6.0-6.4 GiB/s. Nothing to fix.

Through pass: MISSES the target (W2-F measured 89 % on a busier machine, so that figure was luck or a
different kernel). The path is three splice hops (browser -> proxy -> pass -> hub -> origin) against one
direct, so about 5.2 GiB/s of the 6.5 direct is where it settles. Small fixes tried (env-var test hooks, not
kept), 3 sessions each, pass test:

| Variant | Ratios | 
|---|---|
| baseline (1 MiB unix buffers) | 77.9 78.9 80.7 % |
| + SPLICE_F_MORE on the pipe -> socket splice | 75.4 77.9 82.1 % (no gain) |
| 512 KiB unix buffers | 76.1 79.3 86.4 % (noise) |
| 2 MiB unix buffers | 56.2 61.1 67.1 % (worse) |
| 4 MiB + SPLICE_F_MORE | 63.7 64.6 66.6 % (worse) |

So neither SPLICE_F_MORE nor buffer size moves it; the remaining cost is the extra hops (wakeups and
cache traffic per 1 MiB chunk), not a tunable. A real gain would need fewer hops (for example pass handing
the hub connection's fd over to the proxy so a pure pass-through tunnel is one splice), which changes the
SPEC's chain design; proposed as a follow-up, not done here.
