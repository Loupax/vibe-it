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
