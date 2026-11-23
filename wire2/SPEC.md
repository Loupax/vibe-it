# Scriptable Filtering Proxy — Feature Spec

2026-11-23 · the user. Copied verbatim from the user's design doc (2026-11-23) by the coordinator; the doc is private to the user, this file is the repo's copy. It wins over wire2/TODO.txt wherever they differ.

## Overview

Build a local HTTP forward proxy for one user's browser whose traffic passes through a chain of external filters. Each filter is a small HTTP server on its own Unix socket. Requests and responses stream through every filter in the chain, and each filter can forward, change or answer them. The proxy decrypts TLS only for hosts that a connected filter has asked to see in plaintext; everything else stays an opaque tunnel.

**Goals**

- Filters are separate programs in any language with an HTTP library. They see real, streamed HTTP requests and responses and can forward, change or answer them.
- Filters are chosen at runtime with `--filter` flags; command-line order is chain order. Nothing is hardcoded in the proxy.
- On startup each filter advertises its capabilities in a small handshake (layer, hosts, failure policy), and the proxy configures routing and decryption from that.
- Decryption is lazy. The proxy creates no CA and decrypts nothing until a filter that needs plaintext completes its handshake, and then only for the hosts that filter claims.
- The proxy never starts processes. Filters are started and supervised by something else (systemd socket units, a shell).
- Nothing is buffered unless a stage must: request and response heads are parsed, bodies and tunnels stream.

**Non-goals for v1**

- Multi-user or network-wide deployment, caching, authentication.
- Preventing the user from bypassing the proxy (e.g. by changing browser settings). A firewall rule or network namespace can enforce that separately.
- HTTP/2 or HTTP/3 between the browser and the proxy for decrypted hosts; v1 speaks HTTP/1.1 there.
- Protection against other processes running as the same user. With proxy and filters under one account, any process of that user can connect to the sockets or read the CA key; no file or token changes that. Use the dedicated `proxy` user (Security requirements) when that matters.

Reference filter shipped with v1: `pass`, which forwards everything unchanged. It needs no plaintext, so with only `pass` connected the proxy never decrypts.

## Architecture

Figure (architecture: proxy front, hub and upstream; http and tunnel filters).
Requests stream through filters; the proxy decrypts only claimed hosts.

```
                 proxy · no exec · unprivileged · CA created on first http filter
+-----------+   +--------------+------------------+-----------------+   +-------------+
|  Browser  |-->|    Front     |       Hub        |    Upstream     |-->|   Server    |
| 127.0.0.1 |   | decrypts only| routes each stage| dials the server|   | http/https  |
|   :8080   |   | claimed hosts| checks Proxy-Flow| verifies TLS    |   +-------------+
+-----------+   +--------------+------------------+-----------------+
                   | plaintext   ^ forward     | CONNECT     ^ forward
                   v             |             v             |
                +-----------------------+   +-----------------------+
                | http filters          |   | tunnel filters        |
                | sees decrypted reqs   |   | sees CONNECT + bytes  |
                | forward, change,      |   | refuse or close       |
                |   answer              |   |   tunnels             |
                | e.g. pass-decrypt     |   | e.g. pass             |
                |   (tests)             |   |                       |
                +-----------------------+   +-----------------------+
                   filters · systemd user units · one Unix socket each
```

Hosts no http filter claims are never decrypted; with no filters, Front splices straight to Upstream.


The browser talks only to the proxy. Each filter listens on its own Unix socket and is started by systemd, never by the proxy. A filter receives only the hosts it declared, at the layer it declared: **tunnel** filters see `CONNECT` requests and the encrypted bytes; **http** filters see decrypted requests and responses.

Path of a flow:

1. The browser sends the proxy `CONNECT host:443`, or a plain `http://` request.
2. If a connected http filter claims the host, the proxy terminates TLS and sends each decrypted request, in absolute form (`GET https://host/path`), through the http filters in chain order.
3. The proxy's upstream connector opens the server connection as `CONNECT host:port` through the tunnel filters that claim the host, then dials the server and, for https, opens TLS to it.
4. If no http filter claims the host, the browser's own `CONNECT` goes through the tunnel filters and the proxy splices the bytes. Nothing is decrypted.

## CLI and configuration

The proxy is a single binary, configured only by flags. Nothing in this spec depends on the implementation language (Rust is planned). The wire protocol and CLI are the only contracts. There is no config file in v1.

```
proxy --listen 127.0.0.1:8080 \
      --filter unix:$XDG_RUNTIME_DIR/pass.sock
```

| Flag | Default | Meaning |
|---|---|---|
| `--listen` | `127.0.0.1:8080` | Address for the browser to use as its HTTP proxy: `host:port` (loopback unless `--allow-remote` is set), or `unix:<path>` for a forwarder inside a network namespace. |
| `--filter` | none (repeatable) | Filter address. v1 accepts only `unix:<path>`. Command-line order is chain order. |
| `--hub` | `$XDG_RUNTIME_DIR/proxy-hub.sock` | The proxy's own socket that filters forward requests to (see Filter protocol). |
| `--no-decrypt` | none (repeatable) | Host glob never decrypted, even when an http filter claims it (banking, apps that pin certificates). |
| `--ca-dir` | `$XDG_DATA_HOME/proxy` | Where the CA is created on first need. `ca.pem` is what the user imports into the browser. |
| `--connect-timeout` | 5s | How long to keep retrying a filter socket and its handshake at startup before treating that filter as down. |
| `--stage-timeout` | 30s | How long a stage may take to send response headers. Streaming bodies have no total timeout. |
| `--dial-timeout` | 10s | Upstream TCP connect timeout. |
| `--log-level` | info | `debug` logs every hop of every request. |

With zero `--filter` flags the proxy forwards everything as opaque tunnels and never decrypts, which is useful for testing the tunnel path. Unknown address schemes (anything other than `unix:`) are a startup error, not a warning.

## Filter protocol

A filter is an HTTP/1.1 server on a Unix stream socket; the proxy is its client. A request is the handshake if and only if it carries a `Proxy-Id` header. Every other request is traffic, whatever its method or path: a page's own `GET /hello` without that header is forwarded toward the server like any other request. The proxy strips every `Proxy-*` header it did not set, so neither a page nor a filter can forge a handshake.

### Handshake

At startup, and after every reconnect, the proxy sends `GET /hello HTTP/1.1` with `Host: proxy`, `Proxy-Id: <id>`, `Proxy-Filter-Version: 1` and `Proxy-Hub: unix:<path of --hub>`. `Proxy-Id` is a random value the proxy creates at each start; it links the filter to this proxy instance, so a filter that sees a new id knows the proxy restarted and can drop state from the old one. The method and path are only for readability. The filter answers `200` with a JSON body:

```json
{"name": "pass", "version": 1, "layer": "tunnel", "hosts": ["*"], "fail": "open"}
```

| Field | Required | Values (v1) | Effect |
|---|---|---|---|
| `name` | yes | string | Used in logs. Must be unique across filters. |
| `version` | yes | `1` | Protocol version. The proxy rejects versions it doesn't support. |
| `layer` | yes | `tunnel`, `http` | `tunnel`: sees `CONNECT` requests and the encrypted bytes. `http`: sees decrypted requests and responses, and makes the proxy decrypt the https hosts it claims. |
| `hosts` | no (default: all) | glob list | Hosts routed through this filter. `*.example.com` does not match `example.com`. Matched on the normalised host, without the port. |
| `fail` | no (default `closed`) | `closed`, `open` | While the filter is down: `closed` refuses its hosts, `open` skips it. |

Any other answer (non-200, bad JSON, unknown version or layer, duplicate name) rejects the filter: the proxy logs why and treats it as down. A filter changes its capabilities by restarting; the proxy asks again on reconnect.

### Traffic

For each request routed through it, a filter gets a new connection carrying one request with a `Proxy-Flow` header: the proxy's `Proxy-Id`, a slash and the flow number (for example `Proxy-Flow: 7f3a9c/1842`). The hub refuses any request whose `Proxy-Flow` it did not issue. Per request the filter either:

- **forwards** it, changed or not, to the hub socket from the handshake, keeping `Proxy-Flow` unchanged, and streams the request body up and the response back; the proxy routes it to the next filter or to the server; or
- **answers** it itself with any response (block page, redirect, synthetic content).

Rules for every stage:

- Stream. Bodies are copied as they arrive; a stage buffers only what it chooses to inspect.
- Keep framing valid. A stage that changes a body fixes `Content-Length` or switches to chunked encoding.
- `CONNECT` (tunnel layer): a `2xx` from the next hop turns the connection into a byte pipe, which the filter copies both ways and may close at any time; that is how a filter revokes a tunnel. Any non-2xx answer refuses the tunnel.
- Upgrades (WebSocket) at the http layer: after `101` the connection becomes a byte pipe the same way.
- `Proxy-*` headers are the proxy's control channel. The proxy strips every `Proxy-*` header it did not set before a request reaches a filter, the hub or the server. `Proxy-Id` appears only on the handshake, and `Proxy-Flow` only on traffic the proxy issued.

## Routing and decryption

- **Host normalisation.** Before matching and logging, the host is lowercased, a trailing dot is removed, internationalised names are converted to punycode, and IPv6 literals lose their brackets. Without this, `Example.COM` or `example.com.` would slip past `*.example.com`.
- **Routing.** A flow's chain is the connected filters, in `--filter` order, whose layer applies and whose hosts match. It is computed once per flow; other filters never see the flow.
- **Lazy decryption.** A host is decrypted only if at least one connected http filter (handshake complete) claims it and it is not on `--no-decrypt`. The CA is created the first time an http filter completes its handshake, not at startup; with only tunnel filters connected, the proxy holds no key material. The decision is made per `CONNECT`, so a newly connected http filter affects new tunnels only and never interrupts open ones.
- **Decrypted hosts.** The proxy answers the browser's `CONNECT` with `200` itself and terminates TLS with a per-host certificate signed by its CA (cached), offering ALPN `http/1.1` only. Each request goes through the http chain, then the upstream connector. Upstream TLS is verified against the system roots; a failure answers `502` and is logged.
- **Plain `http://` requests** go through the http chain (no decryption needed), then the upstream connector. The port comes from the URL (80 when absent). v1 handles one request per browser connection and answers with `Connection: close`. Hop-by-hop headers are removed: `Proxy-Connection`, `Proxy-Authorization`, `Connection` and the headers it names, `Keep-Alive`, `TE`, `Trailer`.
- **Upstream errors.** The proxy resolves hosts itself (browsers send names to a proxy). DNS failure or a refused connection answers `502`; no connection within `--dial-timeout` answers `504`. Each is logged with the flow id, host and cause.
- **Connection reuse.** A tunnel filter's decision covers a whole tunnel, and a browser may send several hostnames through one tunnel (HTTP/2 reuse when one certificate covers them). Http filters see every request with its own host.

## Flow lifecycle

The proxy keeps a registry of open flows (tunnels and decrypted browser connections) for introspection and cleanup. Revocation is done by the filters themselves: a stage in a flow's path can close its connection at any time, for example a time-quota filter that counts the bytes it pipes.

- Registry entry per flow: flow id, host, port, whether it is decrypted, the chain, open time, a byte counter per direction, and a cancel function that closes every connection of the flow.
- When any stage or either end closes, the proxy closes the rest of the flow and removes the entry.
- Access log: when a flow closes, one line with flow id, host, port, decrypted or not, the chain, the status (or `tunnel`), bytes each way and duration. A decrypted flow also logs one line per request.
- Introspection: `SIGUSR1` dumps the registry (flow, host, age, bytes) to the log.

## Failure handling

Default behaviour is fail-closed: a filter that can't do its job refuses the hosts it covers.

| Situation | Proxy behaviour |
|---|---|
| Filter socket missing at startup | Retry with backoff up to `--connect-timeout`, then mark it down and keep the proxy running. |
| Handshake rejected | Log why and mark it down. |
| Filter down, `fail: closed` | Its hosts get `503` with a short plain-text body naming the filter (a refused `CONNECT` for tunnels). Before any handshake this applies to all hosts. |
| Filter down, `fail: open` | It is skipped; the flow continues through the rest of the chain. |
| Filter connection fails or closes before response headers | That request gets `502`. The filter is marked down and redialled with exponential backoff (100 ms to 5 s), then the handshake repeats. |
| No response headers within `--stage-timeout` | That request gets `504`. The filter stays connected. |
| Malformed HTTP from a filter | That request gets `502`, the connection is closed, and the error is logged. |

A fail-closed filter that has not completed its handshake blocks everything. This is intentional: a missing blocker must not silently allow traffic, and the startup log must make this state obvious. Filters are expected to use systemd socket activation, so the proxy's first connection starts them and this state lasts only until their handshake.

## Security requirements

- **No process execution.** The proxy never calls exec/fork+exec. It ships with a systemd unit that sets `SystemCallFilter=~execve execveat`.
- **Unprivileged.** The proxy refuses to start as uid 0. If a low port is ever needed, use `CAP_NET_BIND_SERVICE` via the unit, never root.
- **Loopback only by default.** `--listen` on a non-loopback address requires `--allow-remote`, otherwise the user has an open proxy.
- **CA only on demand.** The CA is created only when the first http filter completes its handshake, in `--ca-dir`, with the key at mode 0600 and readable only by the proxy's user. The key is never exported; only `ca.pem` is for the browser. The proxy logs loudly when it creates the CA and the first time it decrypts each host.
- **Never-decrypt wins.** A host on `--no-decrypt` is never decrypted, whatever any filter claims; the http filter is skipped for it and this is logged once.
- **Plaintext only where asked.** Tunnel filters never see decrypted bytes. Http filters see only the hosts they claim. Filters that see plaintext see passwords and cookies, so they must be trusted like the proxy itself.
- **Upstream TLS is always verified** against the system roots and is never downgraded.
- **Socket trust.** Before connecting to a filter, and before creating the hub socket, the proxy checks that the socket's parent directory is owned by the proxy's uid and not group- or world-writable (e.g. `$XDG_RUNTIME_DIR`). If not, it refuses and logs why.
- **Peer credentials.** Filters may check who connected with `SO_PEERCRED` (pid, uid, gid, reported by the kernel and impossible to fake), for example that the peer is the proxy service's main process; the proxy checks its filters the same way. Recommended deployment: the proxy runs as a dedicated `proxy` system user, filter sockets accept only that user, and the CA key belongs to it, so processes in the user's normal account can neither impersonate the proxy nor read the key.
- **Hardening in the unit:** `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome=read-only` (with `--ca-dir` writable), `PrivateTmp`, `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`.

## Performance requirements

Undecrypted tunnels must cost little more than a plain splice. Decryption and extra stages cost what they cost, but must never buffer whole bodies.

| Metric | Target |
|---|---|
| Added time to response headers, one local `pass` stage | p99 < 2 ms |
| Undecrypted tunnel through `pass` vs. direct | ≥ 90 % throughput |
| Tunnel with no filters vs. direct | ≥ 95 % throughput |
| Decrypted transfer through pass-decrypt vs. direct | ≥ 50 % throughput |
| Concurrent open flows | ≥ 2,000 without degradation |
| Proxy memory at 500 flows | < 100 MB RSS |

Required implementation choices:

- Splice byte pipes with zero-copy kernel paths where both ends are sockets (Linux `splice(2)`); `splice` returns the bytes moved, so counting is free. Decrypted data needs a userspace copy; benchmark both paths.
- Never hold a lock across network I/O.
- Compute each flow's chain once, when it opens.
- Cache per-host certificates.

## Reference filter: pass

The only filter in v1 is `pass`: an HTTP server on a Unix socket that forwards every request to the hub unchanged, streams bodies both ways, and copies bytes both ways for tunnels and upgrades. It is the starting point for real filters.

- Handshake: `layer: tunnel`, all hosts, `fail: open`. It never asks for plaintext, so it never causes decryption, and it has no flags that change that. Decryption is tested with pass-decrypt, a separate test-only filter in the test suite (the same forwarding code with layer: http); it is not shipped.
- Listens on a systemd-activated socket (`LISTEN_FDS`, fd 3) or a `--socket <path>` flag, creating the socket with mode 0600.
- Never buffers a body. Logs to stderr; `--verbose` logs every request.
- Exits on SIGTERM.

Units: `pass.socket` (`ListenStream=%t/pass.sock`, `SocketMode=0600`, `Accept=no`) and `pass.service`. The proxy's own `.service` has `Wants=` / `After=` on `pass.socket`.

## Acceptance criteria

Every item has an automated test unless marked manual. Integration tests use a local upstream TLS server with a test root, the pass filter, the test-only pass-decrypt filter, and a scripted fake filter in the test suite (forward, answer, rewrite, delay, close, send malformed HTTP) that records what it receives.

- With no filters, `curl -x 127.0.0.1:8080 https://www.example.test` succeeds and no CA file exists.
- With only `pass` (tunnel layer), browsing works, no CA file exists, and the access log has one line per tunnel.
- Connecting pass-decrypt with hosts `*.example.test` creates the CA. New tunnels to `www.example.test` are decrypted (the fake filter sees `GET https://...`), other hosts stay tunnels, and tunnels opened earlier are untouched.
- A host on `--no-decrypt` is never decrypted, even when an http filter claims it.
- An http filter that answers a request itself (e.g. `451` with an HTML body) delivers exactly that response, for https and for plain `http://`.
- An http filter that rewrites a chunked response body delivers the changed body with valid framing.
- A tunnel filter that answers `403` to a `CONNECT` refuses the tunnel; one that closes an open tunnel ends it within 1 s.
- A filter whose hosts don't match a flow never gets a connection for it.
- A 2 GB download through pass and through pass-decrypt keeps proxy and filter memory flat, and its first bytes arrive before the upstream finishes sending.

- A WebSocket echo works through pass and through pass-decrypt.
- `WWW.Example.Test`, `www.example.test.` and a Unicode name whose punycode form matches are routed like `www.example.test`; `CONNECT [::1]:443` parses as host `::1`, port 443.
- A plain `http://` request to a non-80 port reaches that port without hop-by-hop headers and is answered with `Connection: close`.
- Upstream errors: unresolvable host and refused connection give `502`, no answer within `--dial-timeout` gives `504`, a bad upstream certificate gives `502`.
- Killing a fail-closed filter gives its hosts `503`; restarting it restores service without restarting the proxy. A fail-open filter that is down is skipped.
- A stage that never sends response headers gives `504` after `--stage-timeout`.
- `Proxy-*` headers sent by the browser or a filter that the proxy did not set never reach the next stage or the server.
- A page's `GET https://www.example.test/hello` reaches filters as traffic and is forwarded to the server; a filter never treats it as a handshake.
- `Proxy-Id` or `Proxy-Flow` sent by a browser is stripped and never reaches a filter; a request to the hub with a `Proxy-Flow` the proxy did not issue is refused.
- The proxy refuses to run as root, refuses a socket in a world-writable directory, and refuses a non-loopback `--listen` without `--allow-remote`.
- Under the shipped systemd unit, an execve from the proxy is denied (manual check with a debug build).
- The throughput and latency targets in Performance requirements are met (benchmarks over loopback).

## Out of scope / future work

The handshake carries a version, so these can be added without breaking v1 filters.

- HTTP/2 (and later HTTP/3) between the browser and the proxy for decrypted hosts.
- Keep-alive for plain `http://` browser connections.
- Changing a filter's capabilities without restarting it.
- Other transports: `tcp:` filters on loopback.
- Firewall or network-namespace integration so the proxy can't be bypassed by changing browser settings.
- Message-level WebSocket filtering (frames instead of a byte pipe).
- Example filters: `block-facebook` (tunnel layer, refuses `CONNECT` to Facebook hosts) and `youtube-quota` (tunnel layer, counts bytes on the tunnels it pipes and closes them once the daily quota is used). If built: define what counts as an active minute, and test HTTP/2 connection reuse across hostnames with each browser.

