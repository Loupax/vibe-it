# wire2 — prompt log

Append-only log of the exact prompt that produced each change, in order.
The design discussion that led here is in ../web/PROMPTS.md (engine discussion).

---

## Plan (2026-11-23, TODO only)

> Here's the todo I want you to create: 1. save the current state of wire in it's own branch/tag. We start Wire 2 which is going to be a proxy from the beginning. No browser semantics, only stream data between server and browser. Again, should not buffer unless necessary. We should enable scripting later, maybe using an embedded language. Leave that for later, just keep in mind this is something we'll have to do.

> I think we are ready. Add a TODO for v2 scratchy to use this document for its rewrite

> Create a PR for adding the relevant TODO items for our new plan

> You know what, let's not over complicate things. Wire 2 should be on its own folder, wire2

Result: wire2/ with README.md, TODO.txt (W2-SPEC pointing at the spec, W2-1
plain forwarding proxy, W2-2 TLS interception, W2-3 WebSockets in decrypted
tunnels, W2-F filters per the spec) and this file. wire/ is unchanged. The
`wire-v1` branch made earlier for the in-place plan is no longer needed.

---

## Spec copied, wire2 active (2026-11-23)

> 1. Park the wire tasks in general. We only work on wire2 now. [...] 4. Yes, copy the spec. wire2 is active

Result: wire2/SPEC.md (the user's design doc, copied by the coordinator; its one
diagram redrawn as ASCII). W2-SPEC done; wire/ parked.

---

## W2-1 plain forwarding proxy (2026-11-23, agent run by the coordinator)

> You are the agent for item W2-1 in the vibe-it repository (project `wire2/`, a new Rust crate). [...]
> Your step, W2-1: the plain forwarding proxy, i.e. the spec's "zero --filter flags" behaviour, built
> so the later steps fit (W2-2 TLS interception, W2-3 upgrades, W2-F the filter chain with hub socket,
> handshake, Proxy-Flow). [...] Async or threads: your call, but the spec's performance section
> (splice(2) for socket-to-socket pipes, never a lock across I/O, >= 2,000 concurrent flows, < 100 MB
> RSS at 500 flows) must stay reachable; explain the choice in a wire2/NOTES.md. [...] Tests: the spec
> acceptance criteria that apply to this step [...] Also a memory/throughput measurement vs direct
> over loopback for tunnels (spec target >= 95 % with no filters) [...]

Result: the `wire2` crate (binary `wire2`, tokio). CONNECT tunnels spliced through a shared pipe
pool; plain http:// streamed one request per connection with `Connection: close`, hop-by-hop and
Proxy-* headers removed, framing kept (chunked passed through with an end scanner); host
normalisation (idna); 502/504 upstream errors with Happy Eyeballs dialing; flow registry, access log,
SIGUSR1 dump; refuses root and a non-loopback listen without --allow-remote. Integration tests for the
applicable acceptance criteria (TLS through a tunnel with no CA, curl, [::1], normalisation, 502/504,
Proxy-* stripping, 2 GiB transfers with flat RSS, WebSocket echo), a perf test, a CI job, NOTES.md.

---

## W2-2 TLS interception (2026-11-23, agent run by the coordinator)

> You are the agent for item W2-2 in the vibe-it repository (project `wire2/`). [...] W2-2, TLS
> interception, shaped by the spec rather than TODO's older wording: the spec decrypts lazily, only for
> hosts an http-layer filter claims, and creates the CA only when the first http filter completes its
> handshake. Filters arrive in W2-F, so build the machinery now and the policy hook it plugs into [...]
> The decrypt decision is one function per CONNECT [...] Until W2-F there are no filters, so in
> production it is always "no" and the proxy behaves exactly as W2-1 (no CA file created — keep that
> test green). Implement `--no-decrypt` [...] For tests, a test-only way to force decryption for given
> hosts [...] Also do W2-1a first if you need it [...] Tests: decrypted GET/POST through a local TLS
> origin with a test root [...] 2 GiB through the decrypted path with flat memory, and the spec's
> decrypted throughput target (>= 50 % of direct) [...] A browser check: headless Chromium with a
> throwaway profile under .qwork/ trusting the test CA [...]

Result: `decrypt::Policy` (one decision per CONNECT; `add_http_claim` is W2-F's hook and creates the
CA), `ca::Ca` (CA on demand in `--ca-dir`, key 0600, reload on restart, per-host leaves cached in
memory), the decrypted tunnel (TLS to the browser with ALPN http/1.1, keep-alive, each request through
`proxy::forward`, the one request path shared with plain http; upstream TLS verified against the system
roots via rustls-native-certs, failures 502), `--no-decrypt` and `--ca-dir`, the TEST-ONLY
`--test-decrypt` and `--test-resolve` (W2-1a, included). Response bodies are pipelined across two
tasks, with a per-chunk yield so tokio's LIFO slot does not pin both to one core: decrypted throughput
went from 47 % to 52-54 % of direct TLS. Tests in tests/w2_2_*.rs, a headless Chrome check, NOTES.md.
