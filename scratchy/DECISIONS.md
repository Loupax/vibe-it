# scratchy — user decisions

Decisions the user has made, so they are not asked again. Full wording and context
are in TODO-ARCHIVE.txt (search the id), QUESTIONS.md, STASH.md, KEYS.md and
REGRESSIONS.md. A newer decision overrides an older one; add new ones at the top of
their section with the date.

## Direction and process

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

- **QSTASH (2026-11-19, STASH.md):** persistent localStorage, cookie file and
  history in a state directory; sessionStorage per window; `--forget-site`.
  Supersedes the earlier "nothing persisted" rule (QF-D7), which still applies to
  caches.

## Keys and UI

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

- WPT: no pass->fail. The only accepted failures are the two QA11a2 table tests.
  Acid2/Acid3 are information, never a gate.
- PR #232 (body background on the canvas) kept despite one accidental WPT pass->fail.
