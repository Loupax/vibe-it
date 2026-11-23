# scratchy — user decisions

Decisions the user has made, so they are not asked again. Full wording and context
are in TODO-ARCHIVE.txt (search the id), QUESTIONS.md, STASH.md, KEYS.md and
REGRESSIONS.md. A newer decision overrides an older one; add new ones at the top of
their section with the date.

## Direction and process

- **2026-11-23 Drain before the pause (user, with #650):** the three agents already running (QCNV13, QWPT7e,
  QTYPE1) finish, and the coordinator merges their PRs. No new item starts after them.
- **2026-11-23 scratchy is PAUSED.** No new scratchy work is scheduled, to save tokens: no agents,
  no merges of new items. Development moves to `web/` (a CEF-based browser; see `../web/DECISIONS.md`).
  Code, TODO.txt and open PRs stay as they are. Only the user un-pauses it.
- **2026-11-22 The incr-gate page `github` is renamed `gh-home`** (recording dir too), because the
  sandbox read the name in gate commands as a git command.
- **2026-11-22 Remove the old pipeline now (QINC1m).** The user gave the go before the planned
  2026-11-24 check. The from-scratch layout stays as the engine's first-layout/fallback path and
  test oracle; `SCRATCHY_INCR_CHECK` stays as a debug option.
- **2026-11-22 Agent-written entries in the user's state dir stay** (history, localStorage); no
  cleanup.
- **2026-11-22 Web fonts are rendered.** `@font-face` (QFF1a ttf/otf/woff, then QFF1b woff2) is
  un-archived and scheduled; fonts are fetched through wire like CSS images and scoped per page.
- **2026-11-22 No GitHub Support purge of the cached PR refs.** The scratch files still reachable
  through PR #496/#498 refs were checked: test logs, a throwaway wire binary and its dead
  socket token, logged-out screenshots of public pages. Nothing personal; leave them.
- **2026-11-23 CI runs only what a change touches** (user): a `changes` job picks the jobs by path
  (wire changes also run scratchy and its WPT shards; `.github/` runs everything; web/, wire2/,
  coordinator/ and root docs run nothing). Skipped checks count as passing for merge_pr.sh.
- **2026-11-23 merge_pr.sh docs-only fast path** (user): a PR whose whole diff touches only `*.md` and
  `TODO*.txt` files merges right after the rebase push, without waiting for CI. Main's CI still runs on
  the merge commit.
- **2026-11-22 merge_pr.sh keeps both sides of NOTES.md conflicts when both sides only add
  lines** (neither deletes or changes a line of the common base). Any other NOTES.md conflict goes
  back to the agent.
- **2026-11-22 `:has()` (#573) is held until QAPI6b wins back its cost.** YouTube results forced
  layouts were +65 % with #573; #573 merges together with (or after) QAPI6b once the YouTube
  numbers are within noise of main.
- **2026-11-22 html/editing joins the WPT testharness subset (QWPT6d).** Adds an area to the
  R64 phase-1 subset (contentEditable, execCommand), then triage.
- **2026-11-21 Engine evidence before QINC1m** (user "yes"): QINC1n first (fallback log line, sampled check
  writing mismatches to `<state-dir>/engine-check.log`, `:engine` counts). ~3 days with an empty log and zero
  fallbacks is the go signal; the user still confirms QINC1m.
- **2026-11-21 Purge agent scratch from git history** (user): keep a backup of `.git` first, then rewrite history
  to drop the accidentally committed `.qwork/` and `scratchy/.qwork/` files (#496, #498) and force-push main.
- **2026-11-21 UI and persistence first.** The small UI and persistence items (keys, status bar,
  search, completion, paste, scrollbar, QSTASH) take free slots BEFORE the conformance items:
  they make daily use easier and do not depend on conformance. Conformance (QWPT2...) runs in
  slots those items leave free.
- **2026-11-21 Conformance plan R64 approved; all eight R64 §7 recommendations accepted.**
  Q1 phase-1 test pages served by a static mount in wire (no Python, no new socket).
  Q2 WPT's Python server (wptserve) may run locally and in CI in phase 2, reached by wire on
  127.0.0.1 only for `*.web-platform.test`. Q3 testharness results block merges like reftests (no
  passing subtest may start failing; new passes recorded in the same PR; `wpt-th/flaky.txt`), from
  the first CI run. Q4 the run goes in the existing scratchy CI job (+2-3.5 min); split only past
  5 min. Q5 page recordings stay local only (`~/.cache/scratchy/recordings`); CI budgets use
  synthetic in-repo pages. Q6 area order of R64 3.2. Q7 headline metric = mean per-file score
  (wpt.fyi style), subtest pass % beside it. Q8 HTTPS (WPT test CA trusted for
  `*.web-platform.test` only) later, with fetch; never for any other host.
- **2026-11-21 Conformance over sites.** Stop fixing individual sites. Make conformance
  measurable (WPT testharness.js per spec area), work spec areas in dependency order,
  keep perf budgets permanent, diagnose site breakage with tooling. First step:
  QWPT1 design memo, approved by the user before implementation.
- **2026-11-21 TODO cleanup.** Keep the UI work (modes, search, completion,
  persistence, history), the conformance plan, and a few reliability items. Archive
  the rest, media included (TODO-ARCHIVE.txt).
- **2026-11-21 Incremental engine is the default** (QINC1l, merged 906885b #473). `SCRATCHY_INCR=0` is
  the escape hatch until QINC1m deletes the old pipeline after ≥ 3 days of use.
  The from-scratch layout stays as the engine's fallback and the test oracle.
- **2026-11-20 Pause for architecture** once the layout engine is done (held 2026-11-21).
- **2026-11-20 Memory growth is not a blocker** for the engine work.
- **2026-11-20 R60 / QINC1 recommendations accepted.** One engine on the main
  thread; real image sizes for script geometry; match Chrome for data: iframes.
- **2026-11-18 JS engine is V8** (rusty_v8), replacing QuickJS (DuckDuckGo and
  YouTube callbacks were 0.5–1.7 s of interpreter CPU). QV8-6 (remove QuickJS)
  needs the user's OK.
- **2026-11-14 Rendering is "good enough"** for the user's purposes; style work is
  driven by real need, not by completeness.

## Network and privacy (wire)

- **2026-11-23 W-POOL4: wire never retries a non-idempotent request after a clean close (EOF) on a reused
  connection.** A rare failed POST (~1 in 1000 on a stale pooled connection) is preferred over a possible
  duplicate. Retries stay limited to W-POOL3's RST/EPIPE case, where the server provably never read the request.
- **scratchy opens no network sockets itself.** All bytes, media included, go
  through wire.
- **Cookies (QF-D2a, final):** a jar in wire, on by default, used only for requests
  that carry `X-Wire-Jar: scratchy`; one shared jar named `scratchy` (a
  `--profile NAME` is later). Third parties are partitioned by top-level site (Firefox
  Total Cookie Protection). SameSite as declared; missing = None. RFC 6265bis rules.
  `document.cookie` goes through a control channel; HttpOnly never exposed.
- **Cookie control-channel token (2026-11-21, replaces "none for now"):** add it before the cookie
  file (QSTASH5 #481) ships: wire writes a random token to a 0600 file next to its socket and
  requires it on cookie control requests; scratchy reads it at startup.
- **CORS:** browser rules (QF-D1).
- **Beacons** (`sendBeacon`, `keepalive`, `<a ping>`): log in detail, do not send
  (`beacons_send: false` policy flag).
- **Methods:** all HTTP methods with bodies, through wire.
- **Private and loopback targets, `file:`:** allowed with a warning line (policy
  values `Block | Warn | Allow`, default `Warn`).
- **Sync XHR:** really blocks, with its own shorter timeout (5 s) and log lines.
- **Limits:** every network number lives in one policy struct (`net_policy.rs`).
- **User-Agent:** scratchy may override wire's UA (`--user-agent`); pages may
  override user-agent defaults.
- **Tracker blocking (QTB1):** Disconnect list, third-party sub-resources to listed
  trackers are not requested; after the config file (QCFG1).
- **EventSource:** yes. **WebSocket:** "wire as a pipe" (design decided 2026-11-17;
  archived item QF19).

## Persistence

- **2026-11-21 localStorage quota** (user): 5 Mi UTF-16 code units of keys plus values per origin (Firefox's
  rule) is OK; the limit lives in `src/config.rs` with the other knobs (QCONF2).
- **QSTASH (2026-11-19, STASH.md):** persistent localStorage, cookie file and
  history in a state directory; sessionStorage per window; `--forget-site`.
  Supersedes the earlier "nothing persisted" rule (QF-D7), which still applies to
  caches.

## Keys and UI

- **2026-11-21 scratchy starts in NORMAL mode** (user), replacing KEYS D4's "starts in input mode (for now)".
  The start mode is a knob in `src/config.rs`; stickiness and click-into-field -> INPUT stay (QKEYS5).
- **2026-11-21 Copying via vim registers** (user, #528; details KEYS.md "Registers", TODO QREG-D): `:let @+ = @%`
  and `yy` copy the URL; clicking the status bar to copy dropped, mouse-selecting it deferred; a keyboard page cursor
  (QCURSOR-D) is the long-term way to copy text, design first.
- **2026-11-21 Horizontal scrolling** (user): real horizontal scrolling first (QXSCR1), driven by the mouse
  (horizontal wheel/touchpad, Shift+wheel) and ArrowLeft/ArrowRight; no hjkl-like letter bindings for it yet
  (h/l stay history). Horizontal scrollbars (QSB2b) after it.
- **2026-11-21 `:help` stays current** (user: "the help page is out of date"). QHELP1 updates it to every
  feature on main and adds tests that every binding and `:` command appears there; each later UI item updates it.
- **2026-11-20 QKEYS / QSEARCH / QCOMP design** (KEYS.md): input and normal mode,
  Esc Esc, `i`, status bar, `/` search with n/N and Enter, scriptable completion,
  history file, config in code. `o` in normal mode dropped ("`:go` should be enough").
- **Pages may cancel any key** (no reserved set) (QJ11). The user turns vim mode off
  on social.loupax.com via `localStorage.vimOff`.
- **Forms:** built (Tier 6); a redesign together with the user is still open.

## Media (archived 2026-11-21, decisions kept for when it returns)

- In-page video via libmpv, in process, optional dependency (`<video>` is an inert
  placeholder without it). All media bytes through wire.
- Autoplay never, except muted autoplay or `play()` after a user gesture.
  scratchy paints its own simple controls. `<audio>` uses the same backend, with at
  most 2 players playing at once.
- MSE plan answered 2026-11-18 (QUESTIONS.md Q4). PoToken (QM1k2) and native HLS
  (QM1k3) parked.
- Workers and WebAssembly in scope at lower priority than MSE (QUESTIONS.md Q4.6).

## Shadow DOM (2026-11-16, SD-D1..D4)

- Lightweight composed tree (B1 first); basic event retargeting; declarative shadow
  DOM only after rendering works; style scoping with per-root sheets plus
  `:host`/`::slotted` (`::part` and `:host-context` dropped).
- Translate-only `transform` (paint and hit-test offset, no layout change) (QY18).

## Testing

- **2026-11-21 Accepted reftest pass->fail** (user, option 1 on #527): css/printing/transition-in-media-print.tentative.html
  passed only because `@media not` was broken; correct `not` evaluation stays. Fix the right way with print media type
  for print reftests (QWPTPRINT).
- WPT: no pass->fail. The only accepted failures are the two QA11a2 table tests.
  Acid2/Acid3 are information, never a gate.
- PR #232 (body background on the canvas) kept despite one accidental WPT pass->fail.
