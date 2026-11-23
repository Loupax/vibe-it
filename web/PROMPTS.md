# web — prompt log

Append-only log of the exact prompt that produced each change, in order.

---

## Engine discussion (2026-11-23, no code)

> Do not implement anything, I want to discuss something. The way scratchy is implemented now, can we make its rendering engine a replaceable component, or is it too coupled with everything. I'm thinking of swapping it WebKit but I still want the overall contract of using wire for betwork

> How compliant is servo?

> What other engines are there?

> If I want to use scratchy as my daily driver I'd really need to get something as feature complete as possible, and I think focusing on the navigator part is going to be a full time job anyway. Let's focus for feature completeness and wire design compatibility second on this discussion.

> write the CEF + wire spike plan into a doc

Outcome: scratchy's `App` mixes shell and engine state, so the engine is not a
replaceable component; swapping it means keeping the shell and replacing the engine
code. WebKitGTK cannot answer http(s) requests from our own code, so wire would need
to become a TLS-intercepting proxy. With completeness first, the choice is Chromium
through CEF, whose resource handler can answer every http(s) request from wire, with
an empty network namespace as the enforcement. The spike plan became TODO.txt.

## Project start (2026-11-23, docs only)

> Let's pause development on scratchy to not waste tokens. I suggest we create a new project in vibe-it which will be out more feature complete browser

Answers: name it `web` for now; docs only (no code yet); pause scratchy with a freeze
note (code and TODOs stay as they are).

**What changed:** new `web/` with README.md, DECISIONS.md, TODO.txt (the spike, W1-W7)
and this file. scratchy marked paused in its DECISIONS.md, the coordinator prompt and
STATE.md, and the root README.

## W1 Bare CEF window (2026-11-23)

> W1 Bare CEF window (coordinator launch of web/TODO.txt W1)

**What changed:** new crate `web/spike/` (own workspace root, not in CI): one binary
with the `cef` crate 156.7.0 (CEF 156.0.31, Chromium 156.0.8078.100) that opens one
Views window and loads `--url`; `run.sh` starts it in a 6G memory-capped systemd scope
with a throwaway profile; README.md has the fetch/build/run commands and what W1
learned (process model, sandbox, Chromium moving itself out of the scope, startup, RSS).
example.com and en.wikipedia.org load on Wayland and X11 (nested dwl and its Xwayland).

## W2 Empty network namespace (2026-11-23)

> W2 Empty network namespace (coordinator launch of web/TODO.txt W2)

**What changed:** `web/spike/run.sh` now starts the binary as `systemd-run ... unshare -n
--map-user=$UID --map-group=$GID env ... web-spike` (the scope wraps the namespace), so the
whole CEF tree runs with only a down `lo`; `--net` opts out. Plain `unshare -rn` maps us to
uid 0 and Chromium then refuses to start without `--no-sandbox`, hence the uid-preserving map.
Chromium's sandbox stays ON. example.com and en.wikipedia.org fail with
`ERR_INTERNET_DISCONNECTED` ~100-200 ms after the browser is created, nothing crashes, the
window shows (Wayland and X11, nested dwl). README.md has the evidence and findings.

## W3 Answer requests from wire (2026-11-23)

> W3 Answer requests from wire (coordinator launch of web/TODO.txt W3)

**What changed:** the spike's client now has a `RequestHandler`: every http(s) request
gets a `ResourceHandler` that sends it to wire's Unix socket (`WEB_SPIKE_WIRE`, passed by
`run.sh`) and streams the answer back (one thread per request, async `read`).
Chromium follows redirects itself (`X-Wire-Max-Redirects: 0`) and owns cookies (no
`X-Wire-Jar`); wire errors map to Chromium network errors; Blob/File/stream uploads,
which CEF does not expose, fail instead of being sent empty. wire: `X-Wire-Max-Redirects: 0`
uses the shared no-follow agents, so connections are reused (was one TLS handshake per
request). example.com and en.wikipedia.org load inside the namespace; wire's log has every
request (2/2 and 48/48). Findings (Chromium's HTTP cache stores nothing, bypass paths in
a net log, load times, upload gap) in spike/README.md.

## Architecture C (2026-11-23, docs only)

> I'm happy with how wire2 is going so it looks like we have no reason to enforce the connectivity layer to a single app on the application level. Which enables me to pick whatever engine I want for web. we might as well pick webkit or blink but what options do we have in general? Can we make the engine hot swappable?

> wire2 is a web proxy, the documentation is not merged yet

> I can do that? how would that look? the browser and the shell being two different processes?

> I don't want to use an extension because I don't want to rely on the extension API. C looks closest to what I like, can you give me a more detailed view on this?

> Nothing stops us to "make" our own kiosk mode by implementing a bare-bones engine-only browser, right?

> Yes, let's go with C and carry over everything that can be carried from scratchy. I guess KEYS and CRATE?

> Let's say I want to compile an X version of this, would that still be possible? or are we stuck with wayland?

Outcome: a web proxy plus the empty network namespace makes the network path
engine-neutral. Options weighed for the shell: an extension (rejected: no reliance
on the extension API), remote control over CDP/BiDi (automation flags), a compositor
shell driving a stock browser, and finally a compositor shell driving our own engine
hosts. Chosen: Architecture C with own engine hosts (CEF first, WebKitGTK second), the
engine picked at start. X11: the shell runs on an X11 desktop as a winit window; the
engine hosts speak Wayland to the shell either way.

**What changed:** DECISIONS.md "Architecture C"; TODO.txt rewritten around C0-C8 and a
"From scratchy" carry-over list (W1-W7 kept as the spike record, W-UPLOAD1 moot);
KEYS.md copied from scratchy with **web:** notes; HISTORY.md from scratchy's STASH.md
history and ranking sections; README.md. No code.

## No proxy named in web (2026-11-23, docs only)

> remove references of Wire2 from web. as far as web is concerned, it's just a web proxy

**What changed:** web's docs name no particular proxy: the contract is "any HTTP forward
proxy (plain requests and CONNECT) on a Unix socket", given with `--proxy <socket path>`
(DECISIONS.md, TODO.txt C0/C1, README.md, root README). The C0 dependency on another
project's TODO item is gone. The prompts above are kept verbatim.

## STASH from scratchy (2026-11-23, docs only)

> @scratchy/STASH.md ,that's what I meant

(Answering "I guess KEYS and CRATE?" above: CRATE was STASH.md.)

**What changed:** HISTORY.md became STASH.md, mirroring scratchy's: state directory
(engine profiles under `DIR/engines/`), cookies (the engine's jar, third-party cookies
off as the closest match to scratchy's per-site partitioning), forget-site through the
engine hosts, history and ranking unchanged. scratchy's plain-file localStorage cannot
sit behind CEF's or WebKit's storage; the options are in STASH.md and TODO.txt P-STASH
(waiting for the user). New items P-COOKIES and P-STASH; DECISIONS.md, README.md updated.

## localStorage stays in the engine (2026-11-23, docs only)

> does it even make sense? maybe it doesn't.

> yes, drop it and record the decision. Have the engine deal with localStorage unless we have a use case for centralizing it

**What changed:** DECISIONS.md "The engine owns localStorage"; TODO.txt P-STASH dropped;
STASH.md's last section records the drop instead of listing options.

## Forget-site in the engine-host interface (2026-11-23, docs only)

> I suppose we could introduce an interface for -forget-site per engine

**What changed:** STASH.md "Forget site" defines it as an engine-host command
(`forget-site <site>` -> `forgot <site> ok|partial`): the site is scheme + registrable
domain, what is removed, who runs it (the running host, or one started with
`--no-window`, since an engine locks its profile), and how CEF and WebKitGTK would do it.
TODO.txt P-FORGET gets a done-when; C1's protocol notes the command.

## Blink first; PR (2026-11-23, docs only)

> open a PR for the web plan, let's begin with Blink so we can have something working as soon as possible. We can switch to a more lightweight engine later

**What changed:** DECISIONS.md "Blink first"; TODO.txt: C1b (web-host-webkit) moves
after C8 as E1, C2/C3/C8 checks use web-host-cef only; README.md. The plan goes up as a
pull request.

## No enforcement in web (2026-11-23, docs only)

> C0: launcher (namespace plus forwarder to the proxy).
>
> What's namespace on this context?

> don't enforce that. It's something we can do separately, right?

**What changed:** DECISIONS.md "No enforcement in web": web takes `--proxy <host:port>`
and hands it to the engine; the empty network namespace and the forwarder to a proxy's
Unix socket become a separate wrapper outside web. TODO.txt: C0 removed, C1 drops the
namespace from run.sh and checks against a local proxy; README.md and the root README.

## The proxy is optional (2026-11-23, docs only)

> also, just for ease of usage, do not have it require a proxy. It should support it but a missing --proxy flag should default to standard connectivity

**What changed:** DECISIONS.md "The proxy is optional"; TODO.txt header and C1's check
(runs once without `--proxy`, once with); README.md.

## C1 web-host-cef (2026-11-23)

> C1 web-host-cef (coordinator launch of web/TODO.txt C1)

Mid-item, relayed by the coordinator: "do not expose tab functionality yet (tabs are
deferred; maybe later as a vim-style buffers interface, undecided). ... the host shows
ONE page. Drop `new-tab` and `close-tab` from protocol v0. `window.open` and
target=_blank still go through OnBeforePopup, but instead of a new window they load
the target URL in the existing page".

**What changed:** the spike crate moved to `web/host-cef/` as `web-host-cef`
(`web/spike/README.md` stays as the record). W3's resource handler, the wire client
and run.sh's network namespace are gone; `--proxy <host:port>` becomes Chromium's
`--proxy-server` (optional). One page in one Views window, no Chrome toolbar, Chrome
commands blocked; popups load in place. Control socket protocol v0 (JSON lines),
documented in `web/PROTOCOL.md`; W5's download handler kept. `test/drive.py` (the C1
check, with a test forward proxy `test/fwd_proxy.py`) and Rust/Python unit tests.
DECISIONS.md "No tabs yet" (recorded by the coordinator) is what the host follows.

Later, relayed by the coordinator: "two small protocol additions that C3's status bar and
navigation need ... (1) `reload hard` ... (2) A network-level load failure must reach the
shell: add `load-error <id> <code> <text>`". `reload hard` was already in; `load-error`
added (sent before the `load end` that carries the same error), and both are in the check.

## C-NOTAB no new tabs or windows (2026-11-23)

> can we remove tab opening functionality in general, like from context menus and such?

**What changed:** `web/host-cef/src/newtab.rs` (disposition decision, context-menu removal
list, Ctrl+Shift cancel script), handlers in `page.rs` and `renderer.rs`; `test/notab.py`
and `test/pages/links.html` (live check); `web/tools/inject` (copy of scratchy's, plus
`hold:`/`release`); PROTOCOL.md and README notes.

## P-FORGET forget a site (2026-11-23)

> P-FORGET (coordinator launch of web/TODO.txt P-FORGET): add a `forget-site <site>`
> command to protocol v0 with a reply event (`forgot <site> ...`), implemented in
> web-host-cef with CEF's own APIs (cookies through CefCookieManager; localStorage,
> IndexedDB, cache storage, service workers, HTTP cache where possible) for the site and
> its subdomains. No automation protocol (CDP/BiDi): if the only way to clear origin
> storage is `ExecuteDevToolsMethod("Storage.clearDataForOrigin")`, stop and ask. Also
> `web-host-cef --forget-site <site> --no-window`. Do not touch web/shell/ (C2).
> Done when, with a local test site (two hostnames, a fake login that sets a cookie,
> localStorage and IndexedDB on the site and a subdomain), forget-site leaves no cookie
> or storage for it (checked from a fresh load) and another test site's intact.

**What changed:** `forget-site` / `forgot` in PROTOCOL.md (new "forget-site" section)
and `host-cef/src/protocol.rs` (site -> registrable domain, origins; unit tested);
`host-cef/src/forget.rs`: cookies through `CookieManager::visit_all_cookies`; storage
NOT through DevTools: a hidden windowless browser loads each origin's
`/__web-host-forget__`, answered by the host's own resource handler with a page that
clears localStorage, IndexedDB, Cache Storage and service workers from inside the
origin (a service worker answering it is bypassed with `reload_ignore_cache`). The host
records every frame origin it loads in `<profile>/web-host-origins`, since CEF cannot
list origins. sessionStorage of the page's frames on the site; the whole HTTP cache
(`ClearHttpCache`: CEF has no per-site one). `--forget-site` and `--no-window`
(headless ozone, prints `forgot` on stdout, exit 0/1/2). `test/forget.py` is the
check; STASH.md and host-cef/README.md updated. `web --forget-site` wiring is left to
the shell (proposed TODO item).

Rework, the user's decision on #670 relayed by the coordinator: "no need for hacks, use
Storage.clearDataForOrigin. Make sure that's done in an extendable manner for other
engines." And: "why does every forget clear the whole HTTP cache? I thought devtools
allow to clear the cache for a specific origin."

**What changed:** the hidden-page mechanism is gone. Storage is now cleared with
`Storage.clearDataForOrigin` (`all`), called in process through `ExecuteDevToolsMethod`
on a hidden about:blank browser, and its results are awaited with a
`DevToolsMessageObserver`. The engine-neutral part is `forget.rs`, with a
`SiteForgetter` trait; the CEF implementation is `forget_cef.rs`. PROTOCOL.md "What
every engine host implements" covers both engines, including the WebKitGTK plan. The
protocol is unchanged.

Findings from experiments with test/forget.py:
- `clearDataForOrigin` leaves `Domain=` cookies, so the CookieManager pass stays.
- Neither `clearDataForOrigin` nor `clearDataForStorageKey` removes HTTP cache
  entries, and no per-site removal is reachable from CEF. The whole cache is still
  cleared, and PROTOCOL.md says why.

forget.py now also checks the HTTP cache (a `max-age` response).

## C2 web-shell (2026-11-23)

> C2, web-shell, the compositor minimum: a smithay compositor, winit backend (the shell is one window on the user's desktop), in a new crate web/shell/ (the user-facing binary; name it `web` unless you find a reason not to, and say why). It starts the engine host (`--engine cef`, the only one for now) with WAYLAND_DISPLAY = its own socket, owns the control socket the host connects to (`--control <path>`, see PROTOCOL.md), and passes the optional `--proxy` through. It shows the page's window filling the shell window minus a status-bar strip (C3 draws the bar; for C2 a plain reserved strip, or the current URL from the `url` event as plain text if trivial); forwards keyboard and pointer to it; places popups (xdg_popup: menus, selects, permission bubbles); passes cursor shapes to the host desktop; makes the shell window fullscreen when the page asks. One page only (No tabs yet): pair the host's single toplevel with `tab-created`.
> Done when (web/TODO.txt): browsing a site with mouse and typing works with web-host-cef, on a Wayland desktop AND an X11 desktop ... Test inside nested dwl (tools/nest.sh; copy scratchy's tools/nest.sh and tools/inject to web/tools/ ...) with injected input; for X11, use Xwayland inside the nested session if available ...

(coordinator launch of web/TODO.txt C2; the full prompt also carries the agent rules.)

**What changed:** new crate `web/shell/` (binary `web`, smithay 0.7 + winit): the page
window paired with `tab-created`, sized to the window minus a plain 24 px bar strip,
keyboard and pointer forwarded, xdg popups constrained to the window, cursor shapes to
the desktop (image cursors drawn by the shell), page fullscreen, quit on window close and
SIGTERM/SIGINT; unit tests and `test/live.py` (the C2 check, Wayland and X11, with and
without `--proxy`). `web/tools/nest.sh` (also records the nested Xwayland's DISPLAY) and
`web/tools/inject` copied from scratchy. host-cef: the window is now resizable (the cef
crate's default pinned it at 1200x800). CI: a `web/shell` leg (build, test, clippy).

## NEST-HEADLESS live tests with no display (2026-11-23)

> (user, relayed by the coordinator) the monitor moves to the work laptop. Live tests
> must run with no display: a nested compositor that needs no display and shows no
> window, yet takes injected input and grim screenshots and lets CEF render. Smallest
> working option; headless by default, the windowed mode behind a flag. Also: notab.py's
> injected Ctrl never reaches the page; fix it if it is in inject.

**What changed:** `web/tools/nest.sh` (a superset of scratchy/tools/nest.sh) runs dwl on
the wlroots headless backend by default; `--windowed` keeps the old mode. Only dwl is
installed (no sway/cage/labwc), and stock headless dwl advertises no seat capabilities
(no backend input device), so `web/tools/nest/dwl-headless.patch` (env-gated by
`NEST_HEADLESS_SIZE`) makes the seat advertise pointer+keyboard, gives it a keyboard so
clients always get a keymap (Chromium segfaults on `wl_keyboard.modifiers` without one),
falls back to that keyboard when inject's virtual keyboard goes away, and sets the
output size. nest.sh builds it from the user's dwl source into `.qwork/nest-dwl/`.
`web/tools/nest-test.sh` checks it. The "Ctrl never arrives" bug was not Ctrl: inject
sent pointer motions with a non-zero time, and dwl then reports the position from
before the move, so `move` + `click` clicked where the pointer had been; motions now
carry time 0. notab.py: dropped clicks log `open-url-from-tab Drop:`, Ctrl+Enter is
checked by the key reaching the page (Chromium never submits on it), focus events are
allowed, url-before-load-end lookups; links.html shows key names. AGENT_RULES "Nested
compositor helper" points agents at web/tools/nest.sh.
