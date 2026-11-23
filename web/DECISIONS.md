# web — user decisions

A newer decision overrides an older one; add new ones at the top with the date.

## 2026-11-23 Architecture C: compositor shell, engine hosts, wire2

From the discussion after the spike (PROMPTS.md "Architecture C"). Overrides "Request
path" and "Engine" below; "Cookies: Chromium owns the jar" and "Enforcement: an empty
network namespace" still hold.

- **Network: wire2**, the forward proxy (`../wire2`, on its own branch until merged).
  The engine host gets a proxy setting pointing at a forwarder inside its empty network
  namespace, which reaches wire2's Unix socket. Nothing engine-specific sits on the
  network path, so the namespace plus a proxy setting is the whole contract. W3's
  resource handler and wire 1 leave the browser; wire 1 stays for scratchy.
- **Shell: web-shell is a Wayland compositor** (smithay, a nested compositor shown as
  one window on the user's desktop). It draws the status bar and command line, owns all
  keys and pointer input, and forwards to the page what the mode allows.
- **Engine: separate engine-host programs, own protocol.** Each engine gets a small host
  with no browser UI (one window per tab, the page only) and a control socket speaking
  our protocol. `web-host-cef` first (from the spike), `web-host-webkit` (WebKitGTK)
  second. The engine is picked at start (`--engine`); no switching while running, and
  logins do not carry between engines (each keeps its own cookies).
- **Rejected:** a browser extension as the shell (the user does not want to rely on the
  extension API); driving a stock browser over CDP or WebDriver BiDi (automation flags,
  sites that detect it); off-screen rendering into the shell (rebuilds popups, IME,
  drag and drop that the engine does natively as a Wayland client).
- **X11:** web-shell runs on an X11 desktop too: its winit backend opens an X11 window
  there, and the engine hosts keep speaking Wayland to the shell. C2's check covers both.
- **Carry over from scratchy everything that can be carried** (user): KEYS.md and the
  history design now live in web/ (KEYS.md, HISTORY.md); the shell code to port is
  listed in TODO.txt "From scratchy".

## 2026-11-23 The spike is a success; testing stops

The user, after W1-W5 (W4 prepared, W5 ready) and with W6 in progress: "I consider that a
success. you can stop testing now". W6 was stopped; its partial work (switch lists, start of a
WebSocket proxy) is on branch `web/w6-bypass`. W4's real-site login and W7's day of browsing
are not run. Whether this is the go for the "After a go" items is asked separately.

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
