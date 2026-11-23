# web — user decisions

A newer decision overrides an older one; add new ones at the top with the date.

## 2026-11-23 Start `web`, a CEF-based browser; pause scratchy

From the engine discussion of 2026-11-23 (PROMPTS.md "Engine discussion").

- **Priority: feature completeness first, wire compatibility second.** The goal is a
  daily-driver browser; the browser shell is the full-time job, not the engine.
- **Engine: Chromium (Blink) through CEF, prebuilt binaries.** Servo (~62 % overall WPT
  score, Feb 2026) and Ladybird (pre-release) are not daily-driver complete. WebKitGTK
  has no hook to answer http(s) requests from our own code. Firefox has no desktop
  embedding API.
- **Bindings: the `cef` Rust crate** (Tauri team), so the shell stays in Rust.
- **Request path:** a `CefResourceRequestHandler` / `GetResourceHandler` answers every
  http(s) request by sending it to wire's Unix socket and streaming the response back.
- **Cookies: Chromium owns the jar** (proposed; to confirm in the spike). `document.cookie`
  reads Chromium's jar, so a wire-owned jar breaks sites. wire passes `Set-Cookie`
  through and the browser sends no `X-Wire-Jar`. wire's jar stays for other clients.
- **wire keeps:** TLS, DNS, redirects, decompression, request policy, cache, logging,
  and blocking (ads/trackers).
- **Chromium enforces** CORS, mixed content and Secure contexts; scratchy's versions
  (`js_net.rs`) are not ported.
- **Enforcement: an empty network namespace.** The whole CEF process tree runs with no
  network interfaces (`unshare -rn` or bubblewrap `--unshare-net`); wire runs outside it.
  A Unix socket addressed by file path still works across network namespaces, so wire
  is the only way out and any path the handler misses fails instead of leaking. This
  replaces scratchy's seccomp filter, which cannot cover CEF's child processes.
- **Known losses accepted up front:** WebRTC (own UDP sockets) cannot go through wire.
  WebSockets, QUIC, DNS prefetch, preconnect and service-worker fetches are checked in
  the spike (TODO.txt step 6).
- **First step is docs only;** no CEF download or code until the spike is started.
