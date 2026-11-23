# web — user decisions

A newer decision overrides an older one; add new ones at the top with the date.

## 2026-11-23 No middle-click paste in the page; GTK stays

User: "didn't know I hade gtk dependencies in the first place. it's ok tho, Drop middle-click paste in the
page and keep GTK as it is." Chromium (libcef) loads GTK at run time for desktop integration: theme, dark
mode via prefers-color-scheme, and GTK settings such as gtk-enable-primary-paste, which GNOME ships off and
which gates middle-click paste into pages. web keeps GTK; there is no middle-click paste into the page. #689 closed.
(Corrected 2026-11-23 after #695: the coordinator first wrote that middle-click paste works in the `:` and
`/` lines; it was never built. Those lines paste the clipboard only: Ctrl-v, Ctrl-Shift-v, Shift-Insert.)

## 2026-11-23 Enter in NORMAL goes to the page; caret browsing as an experiment

User: "do this. Enter in NORMAL forwards to the page. Expose caret browsing from a command mode command,
so I can experiment with it but don't change the defaults." Enter in NORMAL is delivered to the page
(after a find, Chromium focuses the matched link, so Enter follows it). `:set caret` / `:set nocaret`
toggle Chromium's caret browsing; off by default. The rest of caret mode stays parked.

## 2026-11-23 Search places the pointer, Enter clicks

User: "On Search Mode n/N moves the cursor at the beginning of the match. Enter sends a click event to
it. I think that's better defined behavior." `/`+Enter, `n`, `N` move the mouse pointer to the start of
the current match; Enter in NORMAL clicks at the pointer, wherever it is (coordinator's reading: always,
not only after a search). Replaces KEYS.md S7/T18-T20 ("click the match", "Match spans N links"). Pairs
with C-CMDS, whose commands act on the element under the pointer. Item C5-ENTER.

## 2026-11-23 No context menu; its actions become commands

User: "maybe we could disable it completely, and expose it via the command mode" and, after the
coordinator's proposal (commands act on the element under the pointer now, picked by link hints later;
`:yank` needs the clipboard of C6), "yes, go ahead". C-NOMENU removes the menu (right-click still reaches
the page); C-CMDS adds `:save`, `:save link`, `:save image`, `:yank link`, `:yank image`. DRM (C-DRM) and
H.264 (C-H264) are parked (user, same day).

## 2026-11-23 Focusing an editable switches to INPUT

User: "when input elements gain focus, we should automatically switch to Input Mode. No auto-return on
blur since the exit of the form context isn't that clear." Any `editable-focused true` from the host
switches to INPUT, whether a click, the keyboard or page script (`autofocus`, `el.focus()`) caused it.
This reverses KEYS.md T32 (a page could not pull the user out of NORMAL). Blur never changes the mode.
Known effect: pages that autofocus a field (search engines, login pages) open in INPUT. Done in C3-RCLICK.

## 2026-11-23 Popups return to the previous mode

User, on C3's rule that a click opening a popup (context menu, select dropdown) switches to INPUT: "I
think it makes sense as UX, but what do you think about switching back to the previous mode after
clicking an option?" Agreed: INPUT while the popup is open, back to the previous mode when it closes,
unless the click focused an editable field. Item C3-RCLICK.

## 2026-11-23 forget-site uses DevTools Storage.clearDataForOrigin

User (on #670): "no need for hacks, use Storage.clearDataForOrigin. Make sure that's done in an
extendable manner for other engines." An in-process DevTools method call from the engine host
(CefBrowserHost::ExecuteDevToolsMethod) is allowed for forget-site; this is not the automation
protocol that "No browser extension, no automation protocol" excludes (no remote-debugging port,
no CDP socket, nothing outside the host). The forget contract sits behind an engine-neutral
interface so E1 (WebKit) implements it with its own API.

## 2026-11-23 No tabs yet

User: "Do not bother with tabs for now. Do not expose tab functionality yet. Maybe we'll mimic
them using the buffers interface, but let's not commit now." C4 is deferred (not scheduled, no
design session). The engine host shows one page: protocol v0 has no `new-tab`/`close-tab`, and
`window.open`/target=_blank load in the same page (coordinator's reading, sent to C1; known loss:
popups that rely on `window.opener`, e.g. OAuth login popups). Events keep their `<id>` field so
a later buffers or tabs design needs no protocol change.

## 2026-11-23 The proxy is optional

User: "do not have it require a proxy. It should support it but a missing --proxy flag
should default to standard connectivity". Without `--proxy`, the engine uses its normal
networking; a missing flag is never an error or a warning.

## 2026-11-23 No enforcement in web

User: "don't enforce that. It's something we can do separately". web passes a proxy to
the engine (`--proxy <host:port>`, handed to the engine host; without it the engine
connects directly) and does nothing else about the network. Keeping a program off the
network except through a proxy (an empty network namespace plus a forwarder to the
proxy's socket, as the spike's W2 did) is a separate, engine-neutral wrapper that can be
put around web, or any other program, from outside. Overrides "Enforcement: an empty
network namespace" below and the namespace parts of "Architecture C". Paths that ignore
a proxy setting (WebRTC's UDP, for one) are not web's concern.

## 2026-11-23 Blink first

User: "let's begin with Blink so we can have something working as soon as possible. We
can switch to a more lightweight engine later". web-host-cef is the only engine host
through C8; web-host-webkit (or another lighter engine) is E1, after C8. The control
protocol stays engine-neutral so E1 needs no shell change.

## 2026-11-23 The engine owns localStorage; scratchy's stash store is dropped

User: "drop it and record the decision. Have the engine deal with localStorage unless we
have a use case for centralizing it". localStorage, sessionStorage and IndexedDB live in
each engine host's profile (STASH.md "State directory"), in the engine's own format.
scratchy's one-file-per-key store, its locks and generation counter are not carried
over. Reopen only with a concrete reason to keep page storage outside the engine (e.g.
sharing it between engines or editing it by hand).

## 2026-11-23 Architecture C: compositor shell, engine hosts, a web proxy

From the discussion after the spike (PROMPTS.md "Architecture C"). Overrides "Request
path" and "Engine" below; "Cookies: Chromium owns the jar" and "Enforcement: an empty
network namespace" still hold.

- **Network: a web proxy.** web needs one thing from the network side: an HTTP forward
  proxy (plain requests and `CONNECT`) reachable on a Unix socket. Which proxy that is
  is the user's choice, not web's. The engine host gets a proxy setting pointing at a
  forwarder inside its empty network namespace, which reaches that socket. Nothing
  engine-specific sits on the network path, so the namespace plus a proxy setting is
  the whole contract. W3's resource handler and its wire client leave the browser.
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
  STASH.md now live in web/ (KEYS.md, STASH.md: state directory, history, forget-site,
  cookies); the shell code to port is
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
