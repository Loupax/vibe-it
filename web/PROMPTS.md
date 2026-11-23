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

## P-COOKIES third-party cookies off in web-host-cef (2026-11-23)

> (coordinator) Third-party cookies off in web-host-cef by default, the closest Chromium
> gets to scratchy's per-top-level-site jar. Find the mechanism that takes effect in CEF
> 156 and whether CHIPS and storage partitioning stay on. Engine-neutral in PROTOCOL.md
> (and WebKitGTK's way for E1); no new protocol command; a host flag to turn it off for
> testing is fine. Test with two hostnames served locally: a cross-site frame or
> subresource cannot set or read the other site's cookie, the site at top level can, and
> a Partitioned cookie still works in the third-party context.

**What changed:** `page.rs` sets `profile.cookie_controls_mode` = 1 on the global request
context in `OnContextInitialized` (`--allow-third-party-cookies` leaves Chromium's 2).
`profile.block_third_party_cookies` does not exist in CEF 156. `SetPreference` silently
fails with `CefString::default()` as its `error` argument (found by trying a control
preference). `test/cookies.py` (two runs, default and the flag as control, over https
because third-party cookies need `SameSite=None; Secure`) passes: nothing crosses, top
level keeps its cookies, CHIPS and localStorage stay per top-level site. Docs:
PROTOCOL.md "Third-party cookies off", STASH.md "Cookies", host-cef/README.

## C-INSTALL make web installable (2026-11-23)

> (user, via the coordinator) Make `web` installable for daily use, so `web https://example.com`
> works as an ordinary command. An install script (no root) builds web/shell and web/host-cef in
> release mode and installs the host and CEF runtime into one directory under a prefix (default
> ~/.local) and `web` into ~/.local/bin, atomically enough that a running web is not broken
> mid-copy. `coordinator/ci_then_push.sh --install web` calls it; uninstall instructions in
> web/README.md. Test in the headless nest with a prefix under .qwork/, never the real ~/.local.

**What changed:** `web/install.sh` installs into `PREFIX/lib/web/<version>/` (web, web-host-cef,
CEF runtime) and switches `lib/web/current` and `bin/web` (a symlink) atomically; no shell change
(the shell already finds `web-host-cef` next to its resolved executable). `ci_then_push.sh
--install web` and COORDINATOR_PROMPT.md updated. Checked: drive.py against the installed host
passes (19/19 events), and `web https://example.com` from PATH in the headless nest loads the page
(load end 200, grim screenshot).

## C3 keys and command line (2026-11-23)

> You are the agent for item C3 in the vibe-it repository (project `web/`). ... C3: port the
> keyboard model and command line from scratchy into web/shell: KEYS.md modes
> INPUT/NORMAL/SEARCH/COMMAND, Esc Esc (ESC_ESC_MS), the status bar (mode or the line being
> typed, current URL from the host's `url` event, last message; drawn by the shell with
> cosmic-text + tiny-skia as a shell surface in C2's reserved strip), T1-T34 and ghost
> completion T45-T53; commands `:go` (assumes https:// per scratchy QCOMP6), `:refresh`,
> `:refresh hard` (protocol `reload hard`), `:help` (scratchy's help.html rewritten for web,
> served by the host from a file), `:q`, `:let` with `@%` (`@+`/`yy` need C6's clipboard:
> leave them out or stub with a clear message); history (STASH.md file format) written from
> the host's load/url/title events, with the built-in `:go` source and ranking; scroll
> bindings as real input to the host window (a config.rs table mapping
> j/k/Ctrl-d/Ctrl-u/G/gg/Home/End to keys or wheel steps); `h`/`l` -> back/forward; load
> errors (`load-error`) shown in the status bar. Port from scratchy's src/config.rs,
> completion.rs, history.rs, registers.rs and the listed parts of app.rs as web/TODO.txt
> says; keep scratchy's code untouched (scratchy is paused). The dwm config.h rule:
> settings live in web/shell/src/config.rs. Search (C5) is a later item: `/`, n/N may be
> left unbound or show "search: not yet" -- say which.
> Done when (web/TODO.txt): every KEYS.md row C3 covers has a test named after the row (as
> in scratchy) and works by hand. Live checks: use web/tools/nest.sh ... NEVER open windows
> on the user's desktop ...

(coordinator launch of web/TODO.txt C3; the full prompt also carries the agent rules.)

**What changed:** web/shell: `keys.rs` (the key model: modes, Esc Esc, bindings and the
`gg`/`yy` sequences, the `:` line editor, the commands), `config.rs` (dwm config.h:
`BINDINGS`, `KEY_SEQUENCES`, `SCROLL_INPUTS`, `ESC_ESC_MS`, `START_MODE`,
`CLICK_FOCUS_MS`, status bar sizes and colours, `COMPLETERS`, history knobs),
`completion.rs`, `history.rs` and `registers.rs` ported from scratchy, `statusbar.rs` (the
bar drawn with cosmic-text + tiny-skia into a memory buffer in C2's strip), `help.html`
(`:help`, written next to the control socket, loaded by the host). The compositor's
keyboard filter asks the key model for every key; scroll bindings press a real key in the
page window while held; T29 from `editable-focused` or a popup soon after a click. `/`,
`n`, `N` say "search is not built yet" (C5); `@+`, `yy` and paste say they need the
clipboard (C6). Tests named after the KEYS.md rows in `shell/src/tests/`; live check
`shell/test/c3_live.py` (19 steps in the headless nest).

## C5 search (2026-11-23)

> C5: search through the host's find: `/` opens SEARCH mode with the query typed in the status bar (same line editor as `:`, paste as KEYS.md says where it already works), Enter runs it (T23: current = first match at/below the top of the screen if the engine allows, else the engine's first match; say what CEF gives you), Esc cancels (T25, restoring the previous kept query/highlights), empty Enter = Esc (T24), `n`/`N` next/previous with the wrap messages ("search hit BOTTOM, continuing at TOP" and TOP/BOTTOM) derived from find-result indices, `Pattern not found: <q>` and `No previous search` (T16, T17), `[i/N]` in the status bar, Esc in NORMAL clears highlights but keeps the query (T7), navigation clears highlights and keeps the query (T31). Enter on a match (S7, T18-T19) is NOT in C5 (its own item): leave Enter in NORMAL as it is. Fix anything in host-cef's find handling only if needed, minimal, and say so.
> Done when: every KEYS.md row C5 covers has a unit test named after the row (as C3 did), and a live check (extend web/shell/test/c3_live.py or add c5_live.py) in the headless nest (web/tools/nest.sh, headless by default; never windowed) shows search, n/N with wrap messages, not-found, Esc behaviours on a test page with several matches. CEF under the 6G memory-capped scope; state dirs under .qwork/. Update KEYS.md web notes for search and the shell's help.html. Append a C5 entry to web/PROMPTS.md. Another agent (C-BGNET) works in web/host-cef (switches/prefs); keep host-cef edits minimal.

(coordinator launch of web/TODO.txt C5; the full prompt also carries the agent rules.)

**What changed:** web/shell: `SearchState` and the SEARCH line in `keys.rs` (`/`, `n`, `N`, Esc, `find-result` -> `[i/N]`, wrap messages from the indices, not-found, no-previous-search, smartcase), `Command::Find`/`FindClear` in `protocol.rs`, `find-result` and cross-document load start wired in `state.rs`, help.html. web/host-cef (minimal, `page.rs`, `protocol.rs`): `find` takes an optional `match_case`; a new session ends the old one first (after a navigation CEF never answered the same text again) and the activating step now happens from `on_find_result` once the count is final, so a count of 0 is always reported (live: the old skip-one logic left `n` after a navigation unanswered). Tests: `shell/src/tests/search_tests.rs` named after the rows; live check `shell/test/c5_live.py` (12 steps, headless nest). What CEF gives for T23: from the top of the page match 1; with the screen scrolled to y=1710 it made the match above the screen (at 1300) current, not the first below the top, so exact T23 is proposed as a follow-up.

## C-BGNET switch off Chromium's background traffic in web-host-cef (2026-11-23)

> (coordinator) Switch off Chromium's background traffic in web-host-cef. Through the
> proxy a drive.py run reached update.googleapis.com, *.gvt1.com, clients2.google.com/time,
> www.google.com, content-autofill.googleapis.com. Find the switches/preferences that stop
> each and apply the smallest set that works, documented one by one in host-cef/README.md.
> Decide about the component updater (Widevine): keep page media working without DRM and
> state plainly what is lost. Done when a drive.py run through test/fwd_proxy.py shows only
> page traffic plus a run left idle 2-3 minutes showing no background requests; add a check
> that fails if a non-page host appears.

**What changed:** `protocol::quiet_switches` (applied in `main.rs`'s
`on_before_command_line_processing`): `--disable-sync`, `--disable-background-networking`,
`--component-updater=url-source=http://127.0.0.1:9/`, and `--disable-features=
NetworkTimeServiceQuerying,AimEnabled,PreconnectToSearch,AutofillServerCommunication`;
`--allow-background-traffic` turns them off. Found with a net log (`--log-net-log`) and
leave-one-out runs; `--disable-component-update` and `--disable-background-networking` do
not stop the update check at start (it is an on-demand install), so the updater is pointed
at a closed loopback port. CEF 156's feature names are the C++ constant without the `k`
(the `k` form is silently ignored), which cost several wrong guesses for AI Mode. The late
request (`safebrowsing.googleapis.com`, ~2 min) only showed in a 150 s idle run, which is
why the check has `--idle`. Widevine: CEF ships no CDM and `requestMediaKeySystemAccess`
says no for `com.widevine.alpha` even after 130 s with all background traffic on, so DRM
was already off; the switches lose nothing there. drive.py: host-list check, `--idle`,
`--host-arg`, `--headless`; fwd_proxy.py logs tunnel opens (unit tests for the log parser
caught a field-offset bug in my first version).

## C3-RCLICK popups return to the previous mode (2026-11-23)

> C3-RCLICK: a popup opened by a user click (context menu, select dropdown) switches to INPUT as today; when that popup closes the shell returns to the mode it had before the click, unless the host reported `editable-focused true` for that click. Nested popups count as one session. New KEYS.md row, unit test, help.html, live check in the headless nest.
> Scope addition (user, via coordinator): "when input elements gain focus, we should automatically switch to Input Mode. No auto-return on blur since the exit of the form context isn't that clear." So `editable-focused true` always switches to INPUT (reverses T32); a blur never changes the mode.
> Later note (coordinator): the user is removing the context menu (C-NOMENU), so the live check uses <select> popups.

**What changed:** web/shell: `Keys::popup_opened`/`popup_closed` (T57, `popup_return`, an open-popup counter for nested sessions) called from `new_popup`/`popup_destroyed`; `editable-focused true` always calls `user_focused_control` (T29/T32, no click window; it also cancels a pending return); KEYS.md T29/T32 rewritten, T57 and decision-log entry added; help.html updated; tests in `tests/c3_rclick_tests.rs` and a rewritten T32 test; live check `test/c3_rclick_live.py` (select Esc / outside / option, page popup without a click, autofocus, timer focus(), blur; 8 steps, plus right-click link menu and field menu cases seen passing before C-NOMENU). `c3_live.py`: T29/T32 steps updated; its steps after "T7 T8" were already stale on main (C5 made `/` a search) and are not fixed here.

## C-NOMENU: no context menu at all

> (coordinator, change of task from the user 2026-11-23, "maybe we could disable it
> completely, and expose it via the command mode" ... "yes, go ahead") The item began as
> C-NOTAB-ALLOW (context menu by allow-list); now: no context menu ever appears, for any
> context. Right-click must still reach the page as a normal `contextmenu`/mouse event.
> Remove the deny-list code and tests; keep the cargo fmt commit; the live check asserts
> no menu (no popup, host log line) and that a page's own `contextmenu` handler fires.

**What changed:** `page.rs`: `on_before_context_menu` clears the model, `run_context_menu`
cancels the callback and returns handled; `newtab.rs` lost `MENU_REMOVE`, the cleanup and
their tests. First I built the allow-list (committed in history: `MENU_ALLOW`, tested), then
dropped it. notab.py: for link, target=_blank link, image, selected text, page, editable
field and a page with its own `contextmenu` handler (new `#cust` box in links.html) it
takes a grim screenshot before and after the right-click, asserts the same pixels (masked:
the input's caret and the two links' focus rings, which change when focus moves), the host
log has no `removed [`, and the page saw `ev:contextmenu` (and `ev:custom`). Findings:
clearing the model alone is NOT enough on CEF 156 Chrome style (a popup still opened,
12k pixels changed); `run_context_menu` returning handled is what stops it. CEF's
`sub_menu_at` returns Some for ordinary items, so submenu detection must use `type_at`.
Negative runs (menu back on) fail the check as intended.

## C5-FIXES: same-document find clear, stale find-result, c3_live refresh (2026-11-23)

> C5-FRAG: same-document navigations (#fragment, pushState) must clear the find session (T31). C5-STALE: tag find requests and results so a stale `find-result 0 0` is never taken as an answer. C3-LIVE-REFRESH: bring c3_live.py back to passing after C5 made `/` a search. (coordinator bundle)

**What changed:** the host already sent `load ... same-document`, so no new event: the shell now sends `find-clear` on it (`Keys::navigated_same_document`; the effect is applied from `host_event`, which the live run showed was missing: effects queued outside a key were never sent). `find` carries an id, `find-result` echoes it; the host keeps `find_pending` and drops results when none is pending (cleared by load start and find-clear); the shell ignores results whose id it is not waiting for and drops older waiting requests when the latest is answered. Unit tests `keys_t16_c5_stale_*`, `keys_t15_c5_stale_*`, `keys_t31_a_same_document_*`. PROTOCOL.md and KEYS.md T31 updated. Live: c3_live.py 19/19 (the `/` step now checks the search line; `#` added to the typed shifted keys; page-window wait 30 s in the three live scripts), c5_live.py 13/13 with a new fragment step, c3_rclick_live.py 8/8, all in the headless nest.

## C6 desktop integration: the clipboard (2026-11-23)

> C6: desktop integration. 1. Clipboard and primary selection bridged between the engine host
> (a client of our nested compositor: wl_data_device / primary-selection on the shell's seat) and
> the host desktop (winit window on Wayland or X11; smithay's selection support on the inner side
> and a desktop-clipboard crate on the outer side; pick what works on both and say why). Copy in
> the page -> desktop clipboard; desktop clipboard -> paste in the page. Then enable what C3
> stubbed: `yy` and `:let @+ = @%`, `@+` readable in `:let`, paste into the `:` and `/` lines;
> register `*` only if KEYS.md asks. 2. HiDPI and fractional scale. 3. Frame timing. 4. dmabuf.
> 5. IME only if cheap. If too big for one PR, split: 1 first, propose 2-5 as TODO text.
> Done when: tests named after the KEYS.md rows pass; a live check in the headless nest copies
> from a page and pastes into the `:` line and vice versa through a clipboard the test controls
> (the nested dwl's, never the user's).

(coordinator launch of web/TODO.txt C6; the full prompt also carries the agent rules.)

**What changed:** split as allowed: this PR is part 1 (clipboard), 2-5 are proposed as C6b-C6e.
web/shell: `clipboard.rs` (the desktop's side: copypasta, i.e. smithay-clipboard on the shell
window's own Wayland connection and x11-clipboard on X11, on a worker thread with 1 s bounded
waits; arboard / wl-clipboard-rs were not taken because on Wayland they need data-control,
which GNOME lacks, while smithay-clipboard uses core wl_data_device with the window's own
focus), `selection.rs` (the page's side: data device and primary selection handlers, focus
follows the keyboard, a page copy is read after smithay stores it, the desktop's text is
offered lazily at focus-in, on a paste key in INPUT and on a middle click), register `+` and
paste in `keys.rs`/`registers.rs`. Found on the way: smithay calls `new_selection` before it
stores the selection (so the read is deferred to after dispatch), and a thread started before
the main loop blocks SIGTERM took the signal and killed the shell (live.py's SIGTERM step
caught it; the clipboard thread now blocks SIGTERM/SIGINT). Tests `src/tests/c6_tests.rs`;
live check `test/c6_live.py` (8 steps, Wayland via wl-copy/wl-paste on the nested dwl and
`--x11` via xsel on its Xwayland); c3_live.py's `:let`/`yy` stub step updated; inject
learned `key:ins`. Not working: middle-click paste inside the page (CEF does none, even of its
own selection).

## C-ENTER + C-CARETCMD: Enter in NORMAL goes to the page; `:set caret` (2026-11-23)

> (user) "do this. Enter in NORMAL forwards to the page. Expose caret browsing from a command mode command, so I can experiment with it but don't change the defaults."
> (coordinator) Enter press and release delivered to the page in NORMAL; rewrite KEYS.md T18-T20 (no shell-side click, no "Match spans N links"; the engine decides). `:set caret` / `:set nocaret` / `:set caret?` through a new protocol command, default off, not persisted; leave j/k/gg/G as they are and describe how they behave with caret on.

**What changed:** shell: a `ToPage` action bound to Enter / keypad Enter with no modifier (`config::BINDINGS`, `Mods::None`); with no find open the real press and release reach the page (T19). Live, Chromium did NOT focus the matched link while the find was open (`act=body`); it does when the find ends (CEF `StopFinding` keeping the selection = our `find-clear`). So with a find open Enter sends `find-clear` (as T7) and then gives the page a synthesized Enter, held until the real key is released (T18). Sent at once, the Enter beat the focus change in 1 of 4 runs; a new `Effect::Pause` (ordered effect queue in `input.rs`, one-shot calloop timer) puts `ENTER_AFTER_FIND_CLEAR_MS` = 50 between them: 10/10. `:set caret|nocaret|caret?` (`Keys::set_option`, `COMPLETERS` row `set` from `SET_ARGS`), protocol `caret` command; host: `settings.a11y.caretbrowsing.enabled` via `SetPreference` on the global context, also set false at start (the preference is saved in the profile). Takes effect without a reload. KEYS.md T18-T21, "Caret browsing", decision log; PROTOCOL.md; help.html. Tests `tests/c_enter_tests.rs` (they fail with the Enter rows removed); live check `test/c_enter_live.py` 11/11; c3_live 19/19, c5_live 13/13, c3_rclick_live 8/8.

## 2026-11-23 C2-POPUP (select dropdowns kept inside the window)
Agent task from the coordinator, found by #689: Chromium's select popups send an xdg_positioner
with no constraint adjustment, so smithay's `get_unconstrained_geometry` did nothing and a
`<select>` flush right opened its dropdown at x=673..1083 in a 956 px window. `Layout::constrain_popup`
ORs SlideX|SlideY|FlipY into the positioner's adjustment before computing the geometry
(FlipY so a dropdown that does not fit below a select at the bottom edge opens above it; the slide
catches what is still out); `unconstrain_popup` calls it. Unit test in layout.rs (right and bottom
edge, no adjustment). live.py's stale context-menu step (C2-LIVE-REFRESH) became the
`<select id="sel2">` flush right in pages/c2.html, taken from #689's branch (live.py and c2.html
only; main's permission Block coordinate kept). live.py 16/16 on Wayland and `--x11`; the
step fails (15/16, 673..1083) with the fix reverted.

## 2026-11-23 P-FORGET-SHELL + C-SOCKDIR (`web --forget-site`, `:forget-site`, stale run directories)
Agent task from the coordinator: `web --forget-site <site>` through a running web for the same
state dir, else `web-host-cef --no-window --forget-site`; exit with the host's code; a
`:forget-site` command; remove stale `$XDG_RUNTIME_DIR/web-<pid>` dirs at start.
Mechanism: the shell writes `DIR/web.pid` and listens on `web-<pid>/forget.sock` (short path
whatever the state dir; pid checked through `/proc/<pid>/comm == web`, so a stale file or a
recycled pid falls back to the standalone host). The client gets the host's `forgot` JSON line
(or the `error` event for a refusal); the shell shows `forgot ... ok (N cookies)` and reloads
a page on the forgotten domain. `:forget-site` kept (same plumbing, no extra cost). Tests:
args, `forget.rs` (stale dirs, pid file, client/socket), `tests/p_forget_shell_tests.rs`; live
`shell/test/forget_live.py` 7/7 in the headless nest (running instance, `:forget-site`,
no instance, SIGKILL then next start; other sessions' live shells were left alone).
