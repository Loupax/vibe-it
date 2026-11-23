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

## W4 Cookies in Chromium (2026-11-23)

> W4 Cookies in Chromium (coordinator launch of web/TODO.txt W4)

**What changed:** a persistent profile option for the spike (`run.sh --persistent` =
`~/.local/share/web-spike/profile`, mode 0700; `--profile DIR`; agents keep the throwaway
`.profile`), cookies flushed to disk on every load end, `WEB_SPIKE_COOKIES=1` (names and flags
only), and a local cookie/login fixture (`spike/tests/cookie_fixture.py`). Proven without a
real account: persistent cookies survive a restart, session cookies do not, HttpOnly / Secure /
SameSite and third-party cookies behave as in Chrome, and wire's jar stays empty with no
`X-Wire-Jar` sent. The real-site login is the user's step; the checklist is in spike/README.md
and the (draft) PR.

## W5 Streaming and large bodies (2026-11-23)

> W5 Streaming and large bodies (coordinator launch of web/TODO.txt W5)

**What changed:** the spike's `ResourceHandler` now implements `skip` (CEF applies `Range` to
the handler's stream itself; without it every range request that did not start at byte 0
failed and no seek ever finished), and the client has a `DownloadHandler` (saves under
`WEB_SPIKE_DOWNLOAD_DIR`, no prompt). Checked with a loopback fixture
(`spike/tests/stream_fixture.py`) and public files: a video seeks 8 times back and forth in
380 ms (local) / 660 ms median (Wikimedia 1080p), 150 MiB and 1 GiB downloads complete with
the right size/checksum and flat anonymous memory, a cut body is an interrupted download, a
65 s long-poll is not cut while pings keep flowing. Answered the TODO question: wire's body cap
needs no off switch (it is opt-in via `X-Wire-Max-Bytes`; the timeout switch exists).
