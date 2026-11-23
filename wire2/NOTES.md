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
  (W2-F) and `--no-decrypt`, `--ca-dir` (W2-2) are recognised and refused with "not supported yet".
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

- No `--resolve`-style test hook, so tests use `localhost` instead of `www.example.test`.
- The access log counts heads in `up`/`down`; the spec does not say either way.
- Browser checks were headless only (Chrome `--dump-dom`, Firefox `--screenshot`); video seeking and
  an interactive WebSocket chat need a real session.
