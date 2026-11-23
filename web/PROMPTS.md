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
