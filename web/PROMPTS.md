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

## C6-LIVE: c6_live.py fails 4/8 in another agent's nest (2026-11-23)

> (coordinator) c6_live.py fails 4/8 on main in an agent's headless nest (copy takes more than the field, Ctrl-v into the page, page -> `:` line, primary selection), though #686 reported 8/8. Find out whether the test or the bridge is wrong, and fix it. Middle-click paste into the page is not supported (DECISIONS 2026-11-23): the primary step checks only what is.

**Finding: the test, not the bridge.** Main's c6_live.py passed 8/8 in 16 runs here (fresh nest, after c_enter_live in the same nest, the installed and a debug-built host, under a 24-thread CPU load, `:set caret`, `--x11`). The four failing steps are exactly the four that click into the textarea (`into_field`), and #690's first message ("the field value plus the next paragraph") is a Ctrl-A on the whole page. Main's test reproduces that 4/8 with the same messages whenever the click misses the field, e.g. with the window size the shell has before dwl tiles it (1280x800, so every click lands 160 px left and 103 px low, on the paragraph). Two test faults let that happen: the window size was read from the log right after the first `page window is`, which can still be the pre-tiling size; and when the bar was not INPUT after the click, the test pressed `i`, putting the shell in INPUT with nothing focused, so a missed click turned into four confusing failures instead of one clear one.

**What changed:** `test/c6_live.py`: waits for the shell window to be focused (new log line `shell window focused: <bool>` in main.rs) and for a settled geometry (the latest `window WxH` fits the output and the page window has the matching page size); `into_field` clicks an empty spot (blurs the field), then the textarea, and requires the host's `editable-focused true` (no `i` fallback; a miss fails with a screenshot and the geometry used). The primary step checks only that a selection in the page becomes the desktop's primary; the middle click and the page's `auxclick` listener are gone. No bridge change. Live: c6_live 10/10 Wayland, 5/5 `--x11`; c3_live, c5_live, c_enter_live pass.

## 2026-11-23 C6b (HiDPI and fractional scale)
Agent task from the coordinator: the shell's output was always scale 1 (and the window's
physical size was used as its logical size). Now `Layout` keeps the window's logical size, its
framebuffer size and the scale (winit's scale factor; `Resized` carries changes, e.g. a move
to another monitor); the inner output gets the framebuffer as its mode and `Scale::Fractional`;
fractional-scale-v1 is offered (with the existing wp_viewporter) and every page surface gets
its preferred scale and wl_surface v6's preferred_buffer_scale (on creation, on commit, after a
change). The Space draws the page at the scale; the status bar is painted at the framebuffer's
resolution (font, paddings, caret times the scale) and drawn 1:1 (`Layout::bar_physical`); the
image cursor surface is drawn at the scale; pixel wheel deltas are divided by it. Pointer
positions, popups (C2-POPUP's constraint) and the page's configure stay logical.
Found: CEF 156 binds wp_fractional_scale_manager_v1 and uses it (1.5x buffers with a viewport
destination of the logical size, never set_buffer_scale); without it, it uses the integer
scale (2 at 1.5) through the viewport. devicePixelRatio is the scale.
Tools: `NEST_SCALE` for web/tools/nest.sh (patched headless dwl takes NEST_HEADLESS_SCALE; the
output has NEST_SIZE times the scale pixels; a nest of another scale is replaced; nest-test.sh
checks it), `inject output W H S` (wlr-output-management) to change the scale while running.
Tests `src/tests/c6b_tests.rs`; live check `test/c6b_live.py` 7/7 at 1.5 and at 2 (with a
1.5 <-> 2 change while running); it fails on main (scale 1, devicePixelRatio 1). live.py 16/16
and c3_live.py 19/19 at scale 1 and 2 (made scale-aware: logical coordinates, screenshot
checks times the nest's scale), live.py --x11 16/16 at 1 and 2.

## C-SINGLE: a second `web` on the same state dir panicked the host (2026-11-23)

> (coordinator) With one `web` running, a second `web` with the same state dir started a second web-host-cef on the profile; Chromium's singleton printed "Opening in existing browser session", CEF initialize returned 0 and the host panicked at the assert. `web [URL]` should find the running instance and open the URL there; a stale or foreign lock should be a clear error, never a panic; the host should exit cleanly.

Mechanism: `forget.sock` became the instance's command socket (`open <url>` next to `forget-site`); `forget::run_second` is the second `web`'s end (URL: opens and exits 0; no URL: says "already running (pid)" and exits 0, no raise); `lock.rs` reads `<profile>/SingletonLock` (`<host>-<pid>`): dead pid removed by name with a log line, live/foreign/odd lock is an exit-2 error that names the profile and the pid; the host exits 3 with a message instead of the assert. Tests: `lock.rs`, `forget.rs` (request parsing, answers); live `shell/test/single_live.py` 5/5 in the headless nest (state under .qwork, nothing of the user's touched).

## 2026-11-23 C-KEYMAP (the shell follows the desktop's keyboard layout)
Agent task from the coordinator: the page got xkbcommon's default keymap, not the desktop's layout
(winit does not expose it). New `shell/src/keymap.rs`: a worker thread reads the desktop keymap and
sends it to the main loop, which gives it to the seat (`set_keymap_from_string`; smithay forwards it to
the page). Wayland: a second `wl_keyboard` on winit's own connection (as smithay-clipboard does),
`keymap` on start and on each change, and the `modifiers` group (Alt+Shift between the layouts of
one keymap, e.g. dwl's us,de,gr) applied with `set_layout`. X11: XKB extension on a connection of
its own, libxkbcommon-x11 (dlopen) for the core keyboard's keymap, `NewKeyboardNotify` / `MapNotify`
(debounced) and group `StateNotify`. keys.rs matches keysyms (`modified_sym`), so the bindings follow
the layout once the seat has it (a repeated identical keymap is ignored so it cannot reset modifiers).
Test tooling: the nest dwl patch honours `NEST_XKB_LAYOUT` / `NEST_XKB_OPTIONS` (headless only) and no
longer writes the user's /tmp/dwl-keyboard-layout or signals their someblocks on a layout change;
inject gets `ev:CODE` (raw evdev key), `group:N`, `chord:A,B`, and its virtual keyboard now uses the
desktop's keymap (it used to send us, which flipped the seat's keymap on every inject run).
Live `shell/test/c_keymap_live.py`: de 3/3, fr 3/3 (physical Y/Z, A/Q swap in the page's field,
`:` and `/` open the lines), layout groups 4/4 (us -> de -> us), X11 6/6 (setxkbmap de, fr, us on the
nested Xwayland, keys by XTEST); de and x11 fail with the keymap not given to the seat. c3_live 19/19.

## 2026-11-23 C6c (frame pacing)

Task (coordinator): pace the inner clients' frame callbacks to the outer window's frames, measure
rAF cadence / a CSS animation / a video and shell CPU (idle, animating) before and after under
the perf lock; say what the headless nest's refresh is.
Found: the shell already sent callbacks only after a render, and on Wayland winit already gates
`RedrawRequested` on the desktop's frame callback (smithay's submit calls pre_present_notify;
vsync is off, swap never blocks), so Wayland was paced (30 Hz nest -> 30 Hz rAF, before and
after). The unthrottled case is an X11 shell window: nothing gates, the page's commit rate
drove the shell (CSS animation 59-70 fps, sd 3 ms). Done: `src/pacing.rs` (a Pacer: on X11 a
timer spaces renders by one refresh interval; on Wayland it never waits, a timer there could
miss the desktop's frame callback and cost a whole frame); the inner output's wl_output mode
refresh is the outer monitor's (winit), not a constant 60; `WEB_FRAME_STATS=1`.
Tools: `test/c6c_bench.py` + `test/pages/c6c.html` (modes idle/raf/css/video, ffmpeg makes a
WebM), `NEST_REFRESH` for nest.sh (dwl-headless.patch takes NEST_HEADLESS_REFRESH).
Found, not fixed: at a 120 Hz output Chromium's rAF stays at 60 Hz (before and after; the shell
reports 120 in wl_output; the page commits at 60, so CEF caps its own rate).

## 2026-11-23 C-FOCUSKEYS + C3-SHIFTSYNC (stuck modifiers at focus loss; the desktop's modifiers are the truth)
> (coordinator) smithay's winit backend drops winit's synthetic key releases, so a key or modifier held when the shell window loses focus (Alt of Alt+Tab) can stay down in the page. And the shell learns held modifiers only from key events, so a modifier held when the window gains focus, or set by `wl_keyboard.modifiers` alone, is not seen by keys.rs: `:` and `G` fail.

Mechanism: `Keys::release_all` (focus-out) returns the keys the page has pressed (`forwarded`) and the ones only the shell saw (`down`) and forgets both and the scroll keys it holds; `State::release_pressed_keys` (keymap.rs) sends the releases to the seat before the keyboard focus is dropped (the page's keyup arrives while it still has the keyboard) and drops the shell's own keys from the seat's xkb state quietly. The Wayland watcher (keymap.rs) also forwards `wl_keyboard.modifiers` masks (X11: XKB StateNotify with the modifier state selected) as `Update::Mods`; `State::set_desktop_mods` turns the masks into a `ModifiersState` with a scratch xkb state and `set_modifier_state`s the seat, only while no key is down (the key events are the better source then, and the two arrive on different threads) and tells the page only in INPUT (NORMAL keeps its idea of the modifiers: a scroll key given to it is unmodified). winit's ModifiersChanged is not available (smithay's winit backend drops it), so there is no cross-check. inject gets `mods:BITS`, `kdown:CODE`, `kup:CODE`. Live `shell/test/focuskeys_live.py` (zenity in the nest takes and returns the keyboard): Alt held at focus loss -> the page's keyup Alt, no stuck Alt after (fails with the release off); Shift by modifiers alone -> `:` and `G`/`gg`; Shift held across a focus change -> `:`. Unit tests in `tests/c_focuskeys_tests.rs`.
## LIVE-GEOM: one settled-size wait for every live check (2026-11-23)

> (coordinator, from #695) live.py, c3_live, c3_rclick_live and c_enter_live read the window size 0.5 s after the first `page window is` line, before dwl had tiled the shell, as c6_live did before its fix: move c6_live's settled-size wait into a shared helper and use it in every live check. Also `cargo fmt` on web/host-cef/src/protocol.rs (separate commit).

New `shell/test/livelib.py`: `settle(log, shot)` waits for the first page window, `shell window focused: true`, then until the latest `window WxH, page ..width/height` fits the output and equals the latest `page window is`, and that the size holds 0.2 s; it returns `Geo(ww, wh, ow, oh, bx, by, scale)` (logical; the output size is the grim shot over the nest's scale) or fails naming the window lines seen. Used by live.py, c3_live, c3_rclick_live, c5_live, c_enter_live, c6_live, c6b_live and forget_live (single_live reads no size and clicks nothing). The only `i` presses left are c3_live's T6 tests of `i` itself; no fallback hides a missed click (c6_live's into_field fails with a screenshot path).


## 2026-11-23 C3-GOHTTP (`:go` ghost from an http:// history entry keeps http)
> (coordinator) A `:go` ghost taken from an http:// history entry typed without its scheme (`:go 127` Tab Enter) runs as https://, which fails on plain-http sites. Fix: when the run line's URL equals a history candidate's URL with scheme and www. stripped, run that candidate's own URL; otherwise assume https://. Typed schemes win.

Done in `keys.rs` (`:go` dispatch) with `completion::history_scheme_for` (first history entry in ghost order whose stripped URL equals the stripped word). Tests in `tests/completion_tests.rs` (`keys_t50_t53_go_http_...`, `keys_t50_go_https...`, `..._typed_scheme_wins`, `..._assumes_https`). KEYS.md note updated. No live check (pure logic).

## 2026-11-23 LIVE-GEOM2 + NEST-WINKEYMAP (focuskeys/c_keymap use settle(); the windowed nest gets the keymap fallback)

> (coordinator) focuskeys_live.py and c_keymap_live.py (written in parallel with LIVE-GEOM) do not use livelib's settle() yet. And in windowed nest mode a client started after an inject run ended gets no keymap and Chromium crashes on the first modifiers event (the headless patch handles it; windowed does not).

Both checks now call `livelib.settle(log, shot)` for the output size, window size and the desktop focus (no fallback size). `dwl-headless.patch`: the destroykeyboardgroup fallback (seat goes back to dwl's own keyboard group when a virtual keyboard goes away) is no longer gated on NEST_HEADLESS_SIZE; `NEST_NO_KEYMAP_FALLBACK=1` switches it off. `nest.sh --windowed` runs the patched dwl too (every other hunk stays gated on NEST_HEADLESS_SIZE). The windowed path itself is UNTESTED (agents never open a window on the user's desktop). Reproduction in the headless nest: an inject session ends, then the shell starts: with the fallback off the shell's first `desktop keymap:` line comes 11988 ms in (only when the next inject run creates a new virtual keyboard); with it on, 427 ms in, at start. The Chromium crash itself was not reproduced (the shell client tolerates a missing keymap).

## Picker, bookmarks, buffers, popups (2026-11-23, planning, docs only)

> Let me plan a few features with you about web. First, I want to create something similar to the "temporary buffer" vim has for showing results, tab completions etc. I want it to allow the user to search for bookmarks, their history etc. Then I also want to introduce bookmarks, by just adding the relevant commands in command mode. :bookmark add, :bookmark delete, :bookmark find (with search). finally I want to ad something similar to vims buffers which will play the role of tabs. the user must be able to navigate through buffers, search across them by name and contents and kill them
>
> Don't implement anything yet. how does that sound? be critical

> that decision was specific about the autocompletion of the :go command
>
> the temporary buffer should be used for dedicated historySearch, bookmarkSearch and even context menu related commands

> the context menu stays parked, I just mentioned  that as a potential usecase. use camelcase for not.

> what does window.open do in most browsers?

> the easy ones first
> Should buffer search cover only titles and URLs, or page contents too? Only titles for now, might add contents as a separate feature.
> What happens when you kill the last buffer: quit, or about:blank? about:blank. The user doesn't know if the buffer they just killed is the last one
>
> I think window.open should mimic the behavior of add blockers, which then allows you to open the link manually. Let's talk this through first

> Search is by titles only. One caveat: generic titles like "Login" or "Home" won't tell buffers apart, and matching the URL too would cost nothing. Your call.
>
> I think we should add some context from content. Not the entire page necessarily, but at least the hostname and maybe the first paragraph of text?

> keep it minimal for now then. Allow the user to set a short description (optional). Then the user can do fuzzy search using the page title, the hostname and possibly the description. Those should also appear in the list buffer

> go with b

> how do existing browsers deal with that?

> go with your suggestions

> yes. do that and create a PR

Outcome (DECISIONS.md "A picker, bookmarks, buffers as tabs, popups like Chrome"): a picker drawn
by the shell for dedicated commands (`:historySearch` first), fuzzy filtered, never showing Tab
candidates; bookmarks in a plain `DIR/bookmarks` file with `:bookmarkAdd`, `:bookmarkDelete`,
`:bookmarkSearch`, also feeding `:go`; buffers as tabs with camelCase commands and vim aliases,
searched by title, hostname and an optional user description, the last kill leaving `about:blank`;
popups like Chrome (gesture popups and target=_blank open a buffer with the opener kept, others are
blocked and opened by hand with `:popupOpen` / `:popupSearch`).

**What changed:** docs only. TODO.txt: C-PICK, C-BOOKMARKS, C4-SPIKE, C4 Buffers (replacing the
deferred Tabs), C-POPUPS and a "later" list. DECISIONS.md, KEYS.md ("Picker", "Buffers", decision
log), STASH.md ("Bookmarks"), PROTOCOL.md (note that buffers will change "One page per host").
