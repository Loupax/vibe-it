# web spike

**Record only (C1, 2026-11-23):** the spike's code became the engine host
`web/host-cef/` (build, run and check: `web/host-cef/README.md`). W3's resource
handler and wire client (`handler.rs`, `wire.rs`) and run.sh's network namespace were
dropped there (Architecture C); they are in git history before C1. Paths and env names
below (`web/spike/run.sh`, `WEB_SPIKE_*`) describe the spike as it was.

Throwaway spike code for `web/TODO.txt` (W1..W7). Its own workspace root: it joins
no other build and is not in CI.

State: one CEF window (Views, Chrome runtime style) loads one URL. Since W2 the whole
process tree runs in an empty network namespace (`run.sh --net` opts out); since W3
every http(s) request is answered from wire's Unix socket (`WEB_SPIKE_WIRE`), so
pages load again inside the namespace.

## Versions

- `cef` crate `=156.7.0+156.0.31` (tauri-apps/cef-rs), `default-features = false`
- CEF `156.0.31+ge603d97`, Chromium `156.0.8078.100`, the `linux64_minimal` archive
  (downloaded by `cef-dll-sys`'s build script)
- Build needs cmake and ninja (for CEF's `libcef_dll_wrapper`) and a C++ compiler.

## Fetch CEF and build

CEF lives in a cache outside the repo. `CEF_PATH` tells `cef-dll-sys` where; on the
first build it downloads the archive (~380 MB .tar.bz2, ~1.9 GB with the extracted
tree) to `$CEF_PATH/156.0.31/cef_linux_x86_64/`:

    cd web/spike
    export CEF_PATH=$HOME/.cache/web-spike/cef
    CARGO_BUILD_JOBS=4 nice -n 19 ionice -c3 cargo build

`target/` is ~2 GB afterwards (libcef.so is copied in). Delete it when done:
`rm -rf web/spike/target`. Delete the CEF cache with `rm -rf $HOME/.cache/web-spike`.

build.rs works around two things in `cef-dll-sys` 156.7.0:
- it copies libcef.so and its resources to `OUT_DIR/../../..`, which with the
  current cargo build-dir layout (`target/debug/build/<crate>/<hash>/out`) is
  `target/debug/build/`, not the binary's directory. build.rs hard-links them from
  the CEF dir (`DEP_CEF_DLL_WRAPPER_CEF_DIR`; `cef-dll-sys` is a direct dependency
  for that, which also orders the build scripts) to `target/debug/`.
- it sets no rpath; build.rs adds `$ORIGIN`.

Unit tests (URL normalisation; W3's request encoding, wire head parsing, error
mapping): `cargo test` (same `CEF_PATH`).

## Run

    web/spike/run.sh [--x11] [--net] [URL] [extra Chromium switches]

- Runs the binary in `systemd-run --user --scope -p MemoryMax=6G -p MemorySwapMax=0`
  as unit `$WEB_SPIKE_UNIT` (default `web-spike-<pid>`). Stop the whole process tree
  with `systemctl --user stop <unit>.scope`.
- Profile/cache: `$WEB_SPIKE_PROFILE` (default `web/spike/.profile/`, git-ignored).
  The binary refuses to start without it, so CEF never falls back to `~/.config`.
  CEF's own log goes to `<profile>/cef.log`; the spike's lines go to stderr as
  `[web-spike +<ms>ms] ...` (title, main-frame load end with HTTP status, load
  errors, console messages).
- Network: by default the binary runs as `systemd-run --scope ... unshare -n
  --map-user=<uid> --map-group=<gid> env ... web-spike` (see W2 below). `--net` runs it
  on the host network instead, for debugging only.
- Wayland by default (`--ozone-platform=wayland`, needs `WAYLAND_DISPLAY`);
  `--x11` passes `--ozone-platform=x11` (needs `DISPLAY`).
- wire (W3): `WEB_SPIKE_WIRE=<socket path>` answers every http(s) request from that
  wire. Run your OWN wire outside the namespace (never the user's socket), e.g.
  `WIRE_TIMING=1 wire --socket $PWD/.qwork/wire.sock --cache-dir /tmp/wire-cache-<you>
  --cache-size 256M`; `WIRE_TIMING=1` makes wire log one `TIMING` line per request.
  Unset: no interception, so nothing loads inside the namespace.
- Debug env (optional): `WEB_SPIKE_LOG_REQUESTS=1` (every request CEF asks about),
  `WEB_SPIKE_LOG_HEADERS=1` (header names sent to wire), `WEB_SPIKE_DUMP=1` (main-frame
  URL, `document.cookie` and text on load end), `WEB_SPIKE_JS=<code>` (run in the main
  frame on every load end), `WEB_SPIKE_REPORT_MS=<ms>` (request counters that long after
  start).

In the nested compositor (never the user's display):

    scratchy/tools/nest.sh                    # prints e.g. wayland-1
    env -u DISPLAY WAYLAND_DISPLAY=wayland-1 WEB_SPIKE_UNIT=web-spike-w1 web/spike/run.sh https://example.com/
    # X11: the nested dwl has Xwayland; its display is the X socket dwl listens on
    # (`ss -xlp | grep X11-unix` shows dwl's PID next to it, e.g. :1)
    env -u WAYLAND_DISPLAY DISPLAY=:1 WEB_SPIKE_UNIT=web-spike-w1 web/spike/run.sh --x11 https://example.com/
    systemctl --user stop web-spike-w1.scope
    scratchy/tools/nest.sh --stop

## What W1 learned (for W2..W7)

- **Process model.** One binary for everything: CEF re-executes it with
  `--type=zygote|gpu-process|utility|renderer` and `execute_process` runs that
  helper. A page load gives ~10 processes: browser, two zygotes (sandboxed and
  `--no-zygote-sandbox`), GPU, the network service
  (`--type=utility --utility-sub-type=network.mojom.NetworkService
  --service-sandbox-type=none`), storage/audio utilities, renderer(s).
- **Sandbox.** Chromium's own Linux sandbox works unprivileged (user namespaces;
  no SUID `chrome-sandbox` needed): renderers and sandboxed utilities already sit in
  their own user, pid **and network** namespace. The network service, browser and
  GPU process do not; they are what W2's `unshare -rn` has to cover.
  `WEB_SPIKE_NO_SANDBOX=1` sets `no_sandbox` if nested namespaces under `unshare -rn`
  turn out to be a problem.
- **Chromium leaves the memory-capped scope unless told not to.** At startup its
  xdg-portal code (`components/dbus/xdg/systemd.cc`) asks systemd over D-Bus
  (`StartTransientUnit`) to move the browser process into its own
  `app-org.chromium.Chromium-<pid>.scope`. The browser process then escapes the
  `MemoryMax` cap and `systemctl --user stop <unit>` no longer stops it. It skips
  this when `FLATPAK_SANDBOX_DIR` or `SNAP` is set, so run.sh sets
  `FLATPAK_SANDBOX_DIR` (nothing else in libcef reads it). Naming our scope `app-*`
  does not help. Keep this for every later run script; the session D-Bus socket is a
  path, so it is still reachable inside `unshare -rn`.
- **Startup** (debug build, warm disk cache): `cef::initialize` to
  `on_context_initialized` ~100-270 ms; example.com main frame loaded at ~200-570 ms,
  en.wikipedia.org at ~600-750 ms after process start.
- **Memory** (Wikipedia main page, one tab): process RSS sum ~1.05-1.2 GB (shared
  libcef pages counted per process: browser ~245 MB, GPU ~235 MB, renderer
  ~230 MB, network service ~110 MB, zygotes 20-75 MB each); the scope's own
  `MemoryCurrent` ~300 MB.
- **Wayland noise:** `'--ozone-platform=wayland' is not compatible with Vulkan` and
  one `ContextResult::kTransientFailure` from the GPU process on start; harmless,
  the page renders.

## What W2 learned (empty network namespace)

Done-when met: both sites fail to load, nothing else in CEF crashes.

- **Command.** `systemd-run --user --scope ... unshare -n --map-user=$(id -u)
  --map-group=$(id -g) env ... web-spike`. The scope wraps the namespace, so the 6G cap
  and `systemctl --user stop` cover everything.
- **Not `unshare -rn`.** `-r` maps us to uid 0 inside the user namespace; Chromium then
  exits at startup with "Running as root without --no-sandbox is not supported". Mapping
  our own uid to itself keeps the new user namespace (which is what lets an unprivileged
  user create the network namespace) without becoming root. The Chromium sandbox stays
  ON: its nested user/pid/net namespaces for renderers work under this (sandboxed zygotes
  sit in a further net namespace of their own). `WEB_SPIKE_NO_SANDBOX` was not needed.
- **Evidence (Wayland, example.com; host net ns `net:[4026531833]`).** All 10 processes in
  the scope: browser, the `--no-zygote-sandbox` zygote, GPU (forked from it) and the network
  service (`--type=utility --utility-sub-type=network.mojom.NetworkService`) are in
  `net:[4026535244]`; the sandboxed zygotes and renderers/utilities are in
  `net:[4026535386]`. None is in the host's. `/proc/<pid>/net/dev` of every one lists
  only `lo` (down). Same on X11.
- **Scope.** All processes stay in `web-spike-w2.scope` (cgroup of each PID, `systemctl
  --user status`); `FLATPAK_SANDBOX_DIR` still stops Chromium's D-Bus move into
  `app-org.chromium.Chromium-<pid>.scope`; `systemctl --user list-units
  'app-org.chromium*'` stays empty. The session D-Bus, Wayland and X11 sockets are paths and
  work unchanged.
- **Failure.** `load error: https://example.com/ ERR_INTERNET_DISCONNECTED` ~100 ms after
  the browser was created (+239 ms after start; same for en.wikipedia.org at +194 ms):
  immediate, no timeout, because there is no interface up and no resolver. The window
  shows Chromium's "No internet" page in the nested dwl (grim), on Wayland and via Xwayland.
  No crash dumps, no "exited" lines in `chrome_debug.log`, no GPU restart, window stayed up
  (scope still active after 10 s). Only new noise: `Fontconfig error: Cannot load default
  config file` (harmless).
- **lo is down.** Nothing needed it. W3 must not rely on loopback inside the namespace:
  reach wire by its Unix socket path. If something later wants `127.0.0.1`, add
  `ip link set lo up` inside the namespace (not needed so far).
- **bubblewrap** (bubblewrap 0.12.0) is installed (`/usr/bin/bwrap`). `--unshare-net` would give the same
  empty namespace; it would add the ability to hide the filesystem (bind only the profile,
  sockets, no `~`), a stronger confinement than W2 asks for. Not used: `unshare` is
  enough, and bwrap's own user namespace would also hit the root/uid mapping question.
  Worth considering later for filesystem isolation.

## What W3 learned (answering requests from wire)

Done-when met: example.com and en.wikipedia.org load and render inside the empty
namespace, and wire's log (`WIRE_TIMING=1`: one `TIMING` line per request) has every
request the handler sent, URL for URL.

- **Code.** `src/handler.rs` (CEF glue), `src/wire.rs` (the protocol without CEF
  objects; unit tested). The client's `RequestHandler::resource_request_handler` returns
  a `ResourceRequestHandler` for http(s) only (other schemes: `None`, CEF handles them);
  its `resource_handler` returns one `ResourceHandler` per request. `open` copies method,
  URL, all headers and the upload, sets `handle_request = 0` and starts one thread per
  request: connect to wire's socket, write the request, read the head, then
  `callback.cont()`. `response_headers` maps the head. `read` is async (returns 1 with 0
  bytes); the same thread reads from the socket straight into CEF's buffer and calls
  `ResourceReadCallback::cont(n)`. Nothing is buffered beyond one read (<= 256 KiB);
  `cancel` shuts the socket down. No blocking IO on CEF's IO thread.
- **cef crate trap.** `ResourceRequestHandler::on_before_resource_load` defaults to
  `ReturnValue::default()` = `RV_CANCEL` in the Rust crate (CEF's C++ default is
  `RV_CONTINUE`). Without an override every request ends `ERR_ABORTED` before
  `resource_handler` is ever called. Check every crate default that returns an enum.
- **Request.** `METHOD absolute-url HTTP/1.1`, CEF's header map, `Content-Length` for a
  body, plus `X-Wire-Max-Redirects: 0` and `X-Wire-Timeout-Ms: 3600000` (wire's 20 s
  default covers the body too). Not forwarded: `Host` / `Content-Length` / `Connection`
  (wire's leg), `X-Wire-*` (ours), and `Accept-Encoding`: wire strips
  `Content-Encoding` from every answer and ureq only decodes gzip, so Chromium's
  `gzip, deflate, br, zstd` would bring back undecodable br/zstd bytes. Without it ureq
  asks for gzip and decodes it (checked: httpbin.org/gzip gives `gzipped: true`). No
  `X-Wire-Jar`.
- **What CEF's header map has:** Accept, Accept-Language, User-Agent (Chrome's,
  `Chrome/156.0.0.0`), sec-ch-ua*, Upgrade-Insecure-Requests, Origin (CORS), Cookie
  (from Chromium's jar), Content-Type. **Not in it:** `Referer` (CEF keeps it as
  `referrer_url()`, already reduced by the referrer policy; the handler adds it) and
  `Sec-Fetch-*` (the network service adds those after interception, so they are not
  sent at all: a fidelity loss for sites that check them; W7 will tell).
- **Response.** Status, reason, all headers except `X-Wire-*` (as a multimap, so several
  `Set-Cookie` stay separate), mime type and charset from `Content-Type`. Length =
  `X-Wire-Upstream-Length` when present (0 for HEAD/1xx/204/304), else -1 (read to EOF);
  a body that ends short of it fails with `ERR_CONTENT_LENGTH_MISMATCH`.
- **Errors, not fake 200s.** `X-Wire-Error` answers become network errors
  (`wire::error_for`): `1` + ureq's text -> `ERR_NAME_NOT_RESOLVED` / `CONNECTION_REFUSED`
  / `TIMED_OUT` / `CERT_*` / `SSL_PROTOCOL_ERROR` / else `CONNECTION_FAILED`; `policy` ->
  `BLOCKED_BY_CLIENT`; `truncated` -> `CONTENT_LENGTH_MISMATCH`; `only-if-cached` ->
  `CACHE_MISS`. wire not reachable -> `ERR_PROXY_CONNECTION_FAILED` (Chromium's "something
  is wrong with the proxy server" page). Checked: a main frame on `nx-w3-check.invalid`
  ends in `ERR_NAME_NOT_RESOLVED`; `fetch()` of it rejects with `TypeError: Failed to
  fetch`, as in stock Chrome.
- **Redirects: Chromium follows them, wire does not.** The handler sends
  `X-Wire-Max-Redirects: 0`, so wire answers the 3xx itself and CEF follows `Location`
  with a new request through the handler (`on_resource_redirect` fires). Why: the page
  URL, origin and `location.href` are then the final URL; Chromium applies CORS and
  mixed-content checks per hop, turns POST into GET where it should, sends the jar's
  cookies to each hop and stores each hop's `Set-Cookie`. If wire followed, CEF would
  believe the first URL answered (wrong origin for relative URLs, CORS and cookies; wire
  reports the final URL only in `X-Wire-Final-Url`) and intermediate `Set-Cookie` lines
  would be lost (no wire jar for this client). Checked: `http://en.wikipedia.org/wiki/Rust`
  -> 301 -> loads at the https URL; `https://httpbin.org/cookies/set?w3=hello` -> 302 ->
  `/cookies` shows `{"cookies":{"w3":"hello"}}` and `document.cookie` is `w3=hello`: the
  cookie set on the redirect hop is in Chromium's jar and was sent on the next hop.
- **wire change (in this PR, 4 lines).** Before, any `X-Wire-Max-Redirects` other than 5
  built a one-off ureq agent per request, i.e. a new TCP+TLS connection for every
  request. `0` now uses wire's shared no-follow agents (`manual_agent` /
  `public_manual_agent`, already there for the jar path), so connections are reused.
  Test `wire/src/no_follow_tests.rs` (5 requests, 1 connection; 5 connections before).
- **Cookies.** Chromium owns them (DECISIONS.md): `Set-Cookie` passes through wire
  untouched (no jar header), CEF stores it, `document.cookie` reads it, later requests
  carry `Cookie` in the header map. Persistence across restarts and real logins: W4.
- **Uploads.** `PDE_TYPE_BYTES` and `PDE_TYPE_FILE` elements are sent. Checked against
  httpbin.org/post: `fetch` with a string, ArrayBuffer and URLSearchParams body,
  `sendBeacon`, and a form POST navigation all arrive with the right body and
  `Content-Length`. **Blob, File (also inside FormData) and ReadableStream bodies are not
  exposed by CEF:** Chromium keeps them as data pipes and `CefPostData` gives an `EMPTY`
  element for each (`has_excluded_elements()` still says 0). Sending the rest would
  upload a silently broken body (a multipart form whose file part is empty), so the
  handler fails such a request with `ERR_NOT_IMPLEMENTED` (`fetch` rejects). This breaks
  file uploads (mail attachments, avatars, `<input type=file>` forms) and any
  `fetch(blob)`: a real gap for a daily driver (proposed item below).
- **Which requests reach the handler.** Main frame, subframes, scripts, styles, images,
  XHR/fetch, beacons/pings (`RT_PING`), favicons, and fetches from a dedicated worker
  (still through the browser's handler). **Not:** `blob:` and `data:` URLs (never asked;
  CEF serves them), WebSockets (`new WebSocket('wss://...')` fails at once with close code
  1006 and never reaches the handler; W6), service workers (not tested; W6), downloads
  (`is_download` is logged; not tested, W5). A Chromium net log (`--log-net-log=FILE`,
  a run.sh extra switch) lists what the network service tried itself, i.e. what
  bypassed the handler; all of it failed in the namespace (`-106
  ERR_INTERNET_DISCONNECTED`, `-105`, `-109`, ...): the component updater
  (`update.googleapis.com`, `clients2.google.com/time`), the spell-check dictionary
  download (`redirector.gvt1.com`), autofill (`content-autofill.googleapis.com`),
  `www.google.com` (search prefetch), and preconnects to `https://en.wikipedia.org/` and
  `https://thumb.wikimedia.org/`. None reached the network. W6: switch these off rather
  than route them.
- **Chromium's HTTP cache does not store handler-served responses** (TODO.txt open
  question). A second Wikipedia load with the same profile sent all 38 requests to wire
  again (wire's cache answered 24 hits and 8 revalidations), and the profile's
  `Default/Cache` stayed at 16 KB (an empty index). wire's cache does all the HTTP
  caching for this browser; Chromium only keeps its memory cache / bfcache.
- **Counts (fresh profile, empty wire cache, Wayland in the nested dwl).** example.com:
  CEF asked about 2 http(s) requests, the handler sent 2, wire logged 2 (the same URLs).
  en.wikipedia.org: 48 / 48 / 48 (the 301, the page, CSS/JS/images from two hosts, two
  POST beacons, the CentralAutoLogin 302), 2.4 MB of bodies. No other scheme reached the
  handler.
- **Load time** (process start to main-frame load end, debug build, median of 5 runs,
  each with an empty wire cache; uptime load 0.2-0.7). en.wikipedia.org: direct on the
  host network (`--net`, no wire) 498 ms; through wire with this PR's pooled no-follow
  agent 524 ms; through the old wire (one agent, so one TLS handshake, per request)
  793 ms (540-1067). example.com (median of 3): direct 209 ms, wire 237 ms, old wire
  253 ms. W1's direct numbers were ~600-750 ms (Wikipedia) and ~200-570 ms
  (example.com). Going through wire costs ~30 ms per page here.
- **Known limits.** A server that sends br without being asked (httpbin.org/brotli)
  arrives as garbage, because wire drops a `Content-Encoding` it did not undo (wire item
  below; real servers honour `Accept-Encoding: gzip`). X11 was not re-checked (no
  display code changed). https pages carry no certificate info in CEF (wire did the TLS;
  `SSLInfo` is empty), so the security indicator cannot show the real certificate.

### Proposed follow-ups

- web: uploads with Blob/File/stream bodies. `CefPostData` cannot carry them. Options:
  route only such requests through Chromium's own network stack via a proxy inside the
  namespace that forwards to wire (related to W6's CONNECT idea), find a CEF API that
  exposes the data pipe, or patch CEF. Needs a decision before W7: file uploads are
  daily-driver basics.
- wire: keep `Content-Encoding` when ureq did not undo it (today wire always strips it,
  so br/zstd bodies arrive undecodable), or enable ureq's `brotli` feature.
