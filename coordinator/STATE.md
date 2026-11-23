# Coordinator state (keep this file current; it survives reboots, /tmp does not)

Last update: 2026-11-19 ~20:50 (session 2 wave 1 launched). Durable dir: /home/loupax/src/vibe-it/.coordinator/ (git-ignored via .git/info/exclude).

## Helpers (in this dir)
- `python3 -I .coordinator/done.py /home/loupax/src/vibe-it <ITEM> <short-hash> "<note>"` — moves a finished item from
  scratchy/TODO.txt to scratchy/TODO-DONE.txt and marks its PERFORMANCE QUEUE line `DONE <hash>`. Commit locally.
- `bash .coordinator/merge_pr.sh <n>` — merge queue for ONE PR: rebase only if main gained code since its base (PROMPTS.md /
  REGRESSIONS.md conflicts keep both), wait for all 4 CI jobs, squash-merge. Exit 0 merged, 1 CI red (resume/redo the agent),
  2 unresolvable conflict, 3 other.
- `bash .coordinator/ci_then_push.sh` — waits for the newest main CI run; ONLY if green: pushes my local tracking commits
  (retries once) and runs `cargo install --path scratchy --force -j4` with RUSTY_V8_ARCHIVE.

## SESSION 2 (2026-11-19 ~20:45). Triage: .coordinator/TRIAGE.md. Agent rules: .coordinator/AGENT_RULES.md (give to every agent).
Max 5 agents (user 22:30, was 6; fewer conflicts). Running agents are not stopped; no launch until fewer than 5 are running. merge_pr.sh now ALWAYS rebases; PROMPTS.md/REGRESSIONS.md conflicts keep-both automatically (user 22:25, my recommendation); any other conflict or duplicate R-heading -> exit 2 -> back to the agent. Builds/installs under
`nice -n 19 ionice -c3`; perf measurements under `flock /tmp/scratchy-perf.lock`. QCUR1 #365 merged 6b3520e + archived.

NEXT FREE SLOT: app.rs is taken by QM1k1n5 -> pick from js_dom-free/css-free areas: QY53i3 (animation.rs), QMEM3b/QMEM3d (memory), QM1g2a (media; not alongside QYSK1 fix): why PR #369 causes MORE layouts (+8 %) and style_resolves (+13 %) on YT watch (forced_layout median 5321 vs 3820 ms, load 18-32); fix on the PR or recommend closing. Raw data: session-2 scratchpad out-{m,p}{1,2,3}.json.

MERGE ORDER NOW: nothing open. QUIET PHASE started 04:17: one agent at a time. Batch 1 (Opus): QY53m-m + QMEM3d-m + QY53n3m. Then QY60/QY54/QY33, QOBS9, QFLK3, QPROF1.
PLAN (coordinator 03:45): no new code launches; after QPT1f4 + QYTC1d finish, QUIET-MACHINE MEASUREMENT PHASE: ONE Opus agent at a time: QY53m-m, QMEM3d-m, QY53n3m, QY60, QY54, QY33, QOBS9, then QPROF1 (files the next perf items).
USER 22:20: QV8-6 REMOVE QuickJS -> next free slot (Sonnet), alone in js_engine*/Cargo.toml/ci.yml; agents may skip the local engine-quickjs suite from now (CI guards until QV8-6 merges). USER 22:40: QPT1 GO (Chrome model) right after QV8-6 merges: Opus design+split first (docs), then implement; QPT2-7 backlog. USER 22:25: keep-both for doc conflicts. QM1k-mouse re-tested 22:21 (still slow; re-test after QYTC1a). USER QUESTIONS PENDING: QM1k-mouse re-test, drop QM1h, QY20b/QY15c live checks, WHICH QPT1-7 (R52) go ahead of perf.

## Agents in flight (die with the session)
| item | model | state |
|---|---|---|
| (merged) QM1k2 research #376 -> 636d2a1 | Opus | was READY #376 (R52: iframe realm -> no PoToken; QPT1-7 filed), held on REGRESSIONS conflict |
| QYTC1 | Opus | READY #371 docs, merging. Filed QYTC1a (P1, AFTER QY53o1: same geom_bump_attr hook), QYTC1b, QYTC1c |
| QMEM3e #369 check | Sonnet | DONE: BLOCKED (PR slower: more layouts/resolves); #369 held |
| QYSK1 investigation | Opus | READY #375 docs (R51); filed QYSK1a + QYSK1b |
| QYSK1a | Sonnet | running (media_el.rs hold flag) |
| QYTC1a | Opus | running (js_dom.rs geom_bump_attr hook + GeomCache skeleton) |
| (merged) | QOBS6 #370 -> 1b83534 | — | done |
| QM1k1n5 | Sonnet | READY #374 (not reproduced, guard test + docs), merging |
| (merged) QOBS5c #379 -> f21cdb6 | | done |
| QMEM3e follow-up | Opus | DONE: no extra work (earlier gap = YT variance + stale base); #369 READY, merge queue after #373 |
| QY53i3 | Sonnet | READY #378, held on PROMPTS.md conflict |
| (merged) QY53o1 #373 -> 66fc252 | Sonnet | was READY #373 (gate met, Wikipedia forced 52->20 median), queued to merge after #374; then launch QYTC1a (P1) in the held slot |
| QV8-6 | Sonnet | running (remove QuickJS) |
| QAN1h2 | Sonnet | READY #372 rebased, merging; was READY #372 (candidates -68 %), merge queue after #373 (both touch style.rs) |
| QMEM3b | Sonnet | running (measure first: unexplained ~65 MB main-thread malloc) |
Old worktree .claude/worktrees/agent-a27aab8d4ec172c9d (QMEM3e) kept until #369 merges.

## Process (user decisions 2026-11-19; also in auto-memory coordinator-todo-ownership.md)
- Max 5 agents, worktree isolation, Sonnet small/mechanical, Opus cross-cutting/design/measure; never two on the same files.
- Agents never edit TODO.txt/TODO-DONE.txt; coordinator marks [WIP] on assign and moves done items to TODO-DONE.txt.
- AGENTS DO NOT MERGE: they wait for all 4 CI jobs green, delete build dirs, hand back with first line
  `READY #<n> <ITEM> — <one-line summary>` (or `DRAFT #<n> <ITEM> — ...` / `BLOCKED <ITEM> — ...`; user request).
  The coordinator merges with merge_pr.sh, archives, removes the worktree (`git worktree unlock` + `remove --force`, delete
  branch), then ci_then_push.sh. One status line to the user per merge.
- Never push tracking commits while a main CI run is in progress (cancel-in-progress); ci_then_push.sh handles it.
- Every agent prompt: start EVERY tool-call description with `[<ITEM>] ` (the user's agent panel shows it); branch from origin/main first; own wire on own socket (NEVER /run/user/1000/wire.sock); kill only own
  PIDs (never pkill/killall: `pkill wire` kills pipewire); `ulimit -c 0`; no git stash; scratchpad subdir; `nice -n 15` for
  cargo; RUSTY_V8_ARCHIVE=/home/loupax/.cache/librusty_v8-152.2.0.a; CARGO_BUILD_JOBS=4; clippy <= 77 (local toolchain;
  CI baseline file 74); WPT /home/loupax/src/wpt (ac78b567, 322 pass/292 fail); full tests on default (QuickJS removed b717000).
- Before a user test: user tells me; pause launches, ask agents to hold builds, wait for low `uptime`.
- Order: PERFORMANCE QUEUE (top of scratchy/TODO.txt QUEUE) -> QPROF1 (+ part B joint testing with the user) -> QYS1
  (YouTube search re-test, Opus) -> QM1k2 (attestation research, honest only) -> rest of the backlog.

## Next to assign (respect file conflicts)
FIRST (user 2026-11-19 ~20:30): QM1k2 RESEARCH (one slot) + QYTC1 (P1, YouTube controls-visible forced layouts; Opus measure first) + the #369 QMEM3e quiet-machine check.
QM1k2 RESEARCH (Opus, measure only, docs; one slot, no code conflicts): which YouTube
BotGuard probes run in scratchy, which fail/throw/look odd, why no valid PoToken; compare with a normal browser; output a
sized fix list in REGRESSIONS.md. Honest only (earn the token by running YouTube's own script faithfully; never forge,
replay, borrow or fetch tokens; never disguise scratchy as another browser). Then ask the user which fixes go ahead of perf.
QY53o1 (js_dom.rs geom_bump_if; after QCUR1 if it touches js_dom) -> QOBS6 then QM1k1n5 (app.rs; after QCUR1: media data
arrives late at startup) -> QOBS5c -> Q33b2 then QY53e2 -> QY53m then QY53m2 -> QY53n1..n5 (app.rs apply_hover) -> QY53l3
-> QMEM3b/d/e, QY53i3 -> QSTK1b, QY47b, QY53f1d, QM1g2(a) -> re-measures QY60/QY54/QY33/QOBS9 (quiet machine) -> QPROF1.
Conditional: QY53f2/f3, QY53o2, QY53n4, QY53o3c, QMEM3f.

## User's frame-stats runs (YT G1AeBgalFZk, real window): .coordinator/user-runs/
fs-user-yt-1715 (before: painted med 15.5, worst 1, lat_p95 201) ; fs2-user-yt-1845 (after QM1k1n6a: 30 fps, p95 62) ;
fs3-user-yt (same code) ; fs4-user-yt-n6b (after QM1k1n6b: >1 s gaps 11 -> 2; startup ~5 s with 0 rendered frames =
media data late -> QM1k1n5). SCRATCHY_FRAME_STATS APPENDS: ask the user for a new filename each run.

## Open user-facing items
- QuickJS: user will decide TOMORROW (2026-11-20) whether to remove it (QV8-6, approved earlier) and/or let agents skip the
  local QuickJS test suite (CI's scratchy-quickjs job still guards). Do NOT act on it before the user decides. How to compare:
  PERF.md "QV8-m" (V8 faster on every page) or build QuickJS into ~/.cache/scratchy-qjs and diff --perf-report runs.
- Ping the user to re-test (new fs file, quiet machine) after QCUR1 is merged + installed.
- Stale git stash `qy53f1-agent-tag` is the user's to drop.
08:xx user: "carry on". Plan agreed: finish queue (QY54a3, QY54b, QOBS9 quiet, QPROF1); perf follow-ups NOT started unless QPROF1 justifies them; then backlog.
09:45 user: do incremental layout properly in one go -> QINC1 Opus design running; QY54b3a-f on hold. QOBS9/QPROF1 after.
11:45 user: QINC1 GO (all recs), QPT1-D = opaque origin (QPT1-D1 filed). Wave 1: QINC1a,b,d,e. QPROF1-A running.
13:39 OOM: scratchy pid 787397 at 15 GB anon RSS killed; session died. Added memory-cap rule (systemd-run scope MemoryMax=6G). Resuming wave 1 + QPROF1.
14:10 user: usually has YouTube open -> QOOM1 P1 filed; takes NEXT free slot (before QPT1-D1).
14:40 USER: after the layout engine (QINC1) is done -> PAUSE implementation, hold an ARCHITECTURE DISCUSSION with the user before continuing.
16:30 user: engine memory (+24 MB Wikipedia) is NOT a blocker for deployment/flip; QINC1f-mem P3.
~16:35 USER LEFT: "Do thorough testing yourself once all the pending workers finish. Create a report and make an artifact so I can check it from outside."
PLAN while away: continue QINC1 chain g -> h1 -> (h2 || i || j) -> k (stop BEFORE QINC1l flip: needs user OK); QINC1f-mem only if a slot is idle.
No other implementation (pause rule). Then: my own thorough testing (suite, WPT, nested-dwl real pages YouTube/Wikipedia/github/bbc/social
with SCRATCHY_INCR off/on/check, screenshots looked at, memory under cap), then a report published as an Artifact (link for the user).

## 19:02 2026-11-20
- QINC1h1 merged #457 109445c (0 mismatches YT/Wiki; incr relayout 2.6 vs 6.8 ms results, 0.75 vs 2.9 watch, 0.5 vs 51 Wiki).
- Launched wave 5: QINC1h2 (Sonnet, ad41a41f7b8c7bbcb), QINC1i (Opus, a6884c0a3f12013d3), QINC1j (Opus, a0901a28898507700; incl. App-frame shadowing from #457).
- Next: merge them one at a time, then QINC1k, STOP before QINC1l; then own testing + report artifact.
- 20:12 QINC1h2 merged #458 ce886a5; QINC1h3 filed P3. Running: QINC1i, QINC1j.
- 20:36 QINC1j merged #459 e219bee; filed QINC1j-ab (P2 blocks k), j-c (P3). Worktree agent-a0901a28898507700 locked (pid 795104) - remove later.
- 20:36 launched QINC1j-ab (Opus a424f3d28ab0f4051). Running: QINC1i, QINC1j-ab. Then QINC1k.
- 21:14 QINC1i merged #460 9c55521. Filed QINC1i2/3/4, QFLK5. Running: QINC1j-ab (told about i4). Next: QINC1k after j-ab.
- 22:07 QINC1j-ab merged #461 30a23cb (YT INCR=1 builds ~1s vs 2.5-3.3 off). Launched QINC1i5 (Opus ac9c75cb693ed8ab8, Wiki INCR regression from #460) + QINC1i4 (Opus a5ab65a2ceae43329, adopt build styles). Then QINC1k; stop before l.
- 22:56 QINC1i5 merged #462 dc6ba98 (Wiki INCR builds 3.6-4.4s). Launched QINC1i6 (Sonnet a2b011781219d18d3). Running: i4, i6. Then QINC1k.
- 23:21 QINC1i4 merged #463 2618ee3. Filed i7, i4b, i4c, QFLK6 (none block k). Running: i6. Then QINC1k.
- 23:42 QINC1i6 merged #464 42b99fd. Launched QINC1k (Opus ac697452dc2550eb6): 6-site CHECK sweep + gates + durable recipes in tools/, go/no-go only. After: my own testing + report artifact; STOP before QINC1l.
- 11-21 01:14 QINC1k merged #465 b924d48: NO-GO (github +54% builds, 1 sub-px mismatch; Wiki -62% vs -80% gate). Data copied to .coordinator/qinc1k-data (screenshots, summaries). Launched QINC1k1 (Opus ad3bc14090e093db5) + QINC1k2 (Opus ace54cc04e137738d). Then QINC1k3 (after k1, same area), re-run gate.sh, then my testing + report. Note for user: social.loupax.com line-number gutter differs with flag (flag-on likely correct, user to confirm).
- 11-21 02:22 QINC1k2 merged #466 ce088f9 (github INCR builds 2.2-2.4s vs 7.5 off). Launched QINC1k4 (Opus acae36b08315b3a2f, slot vh mismatch) + QINC1k5 (Opus a091c55d8ea23c039, flag on/off geometry diffs, measure). Running: k1, k4, k5. Then k3, gate re-run, my testing + report.
- 11-21 02:48 QINC1k5 merged #467 b0b517b: flag-on geometry is CORRECT (Chrome-like) on github + social; not a blocker. Launched QINC1k6 (Sonnet a53746b1f4ebe7aa9). Running: k1, k4, k6. Then k3, gate re-run, my testing + report.
- 11-21 02:59 QINC1k4 merged #469 e560b44. #468 QINC1k1 open (agent still finishing). Running: k1, k6.
- 11-21 03:09 QINC1k6 merged #470 746586f. Running: k1 (#468).
- 11-21 04:04 QINC1k1 merged #468 5b420e6 (also paint edge snap + clearance calc fix). Launched QINC1k3 (Opus a365a8b3568d3e14f, Wiki root walks) + QINC1k10 (Opus a7475b9157dd9ce4b, fuzz width mismatch). Then gate re-run (6 sites) + my testing + report; STOP before QINC1l.
- 11-21 05:51 QINC1k3 merged #472 2798bb5. ALL QINC1 workers done except flip. NEXT: my own testing (suite, gate.sh check+perf all pages, screenshots), report artifact. STOP before QINC1l.
- 11-21 05:56 Own testing started: .coordinator/test/run_all.sh (bg bud4ohh5s): suite off/on, WPT off/incr/check, gate check/perf/headless x3 all 7 pages, shotdiff. Progress: .coordinator/test/progress.log. Then review screenshots + publish report artifact.

- 2026-11-21 07:08 Own testing DONE (2798bb5): suite 1742/0 off+forced; WPT 333/293 x3, 0 mism; gate check 0 mismatches 21 win + 21 hl runs, no panics; perf builds on/off 11-41%; hover p95 +~20ms (QHOV2). Report: .coordinator/report/qinc1-report.html = https://claude.ai/artifact/BASooXrKKsT8owWQBkTPFy. PAUSED: waiting for user OK on QINC1l + architecture discussion.
- 2026-11-21 11:14 Progress artifact https://claude.ai/artifact/DZKa6yP3SWAXhvivRTi1QH (src .coordinator/report/progress.html): UPDATE AFTER EVERY MERGED PR (user).
- 2026-11-21 11:21 Coordinator files moved into git: coordinator/ (scripts, AGENT_RULES, STATE, report/); .coordinator/ stays local (git-ignored). Prompt COORDINATOR_PROMPT.md is environment-aware (user machine vs claude.ai cloud).
- 2026-11-21 11:27 QINC1l merged #473 906885b: engine default; SCRATCHY_INCR=0 escape hatch; CI runs suite engine-off too (1743/0 both). QINC1m waits ~3 days of user use + OK.
- 2026-11-21 11:45 Reinstalled 11:31 from 933822c (engine default live). QWPT1 merged #474 d83093f: R64 conformance plan; 26 follow-ups in R64 s8 NOT filed yet; waiting for user answers Q1-Q8 (R64 s7).
- 2026-11-21 13:12 New session (user: "follow COORDINATOR_PROMPT.md"). Conformance still waits on R64 Q1-Q8. Launched ready non-conformance items in separate areas: QFLK4 (Sonnet, forms test timing), QFLK5 (Sonnet, V8 heap tests), QFLK6 (Sonnet, window images keyed by doc id), QJ4b (Sonnet, grid panic), QSTASH1 (Opus, src/stash.rs). Held: QINC1k9 (images, after QFLK6), QKEYS/QCONF/QPASTE (one UI-keys item at a time later).
- 2026-11-21 13:25 USER accepted all 8 R64 s7 recommendations -> DECISIONS.md. Filed the 26 R64 s8 items into TODO.txt (CONFORMANCE section). Next free slots go to QWPT2 (Sonnet), QWPT3 (Sonnet, wire), then QWPT4a (Opus, after QWPT3); conformance P1 items take priority over the remaining reliability/UI items.
- 2026-11-21 13:32 USER: small UI + persistence items first (they help daily use, independent of conformance); conformance fills remaining slots. Slot order: QPASTE1 (clipboard), QSTASH5 (wire), QSB1 (scrollbar), QCONF1 -> QKEYS1 -> QKEYS2 -> QSEARCH1-3 / QKEYS3-4 (keys area, one at a time), QSTASH2 after QSTASH1, QSTASH4, QCOMP chain after QCONF1; QWPT2/3 when a slot has no UI item in a free area.
- 2026-11-21 13:36 USER confirmed the UI/storage slot order (incl. the M items). Conformance after.
- 2026-11-21 13:50 Merged #475 QFLK5 ea2e80e, #476 QFLK6 bf99699, #477 QFLK4 4bc131b (real bug: input events were plain Event), #478 QJ4b e4ba938, #479 QSTASH1 4d6333e. Filed QFLK7, QFLK6b, QJ4c. One ci_then_push for all five (reinstall from latest main). Launched QPASTE1 (Sonnet), QSTASH5 (Sonnet, wire), QSB1 (Sonnet), QSTASH2 (Opus), QWPT2 (Sonnet, free slot: tools/ci). QCONF1 held until QSB1/QPASTE1 merge (same constants). For user: QSTASH1 quota counted as 5 Mi UTF-16 units (Firefox rule).
- 2026-11-21 14:20 USER: add the cookie control token BEFORE #481 (QSTASH5) ships -> DECISIONS.md; sent back to the QSTASH5 agent to extend #481 (wire token file <sock>.token 0600, X-Wire-Control-Token, scratchy reads it). Ready to merge: #480 QWPT2, #482 QPASTE1, #483 QSB1, #484 QSTASH2.
- 2026-11-21 14:40 Merged #480 QWPT2 9d4ff71, #482 QPASTE1 76ac76e, #483 QSB1 fa26a84, #484 QSTASH2 99f881d. Filed QSB2, QWIKI1; QSEARCH1 note (paste reuse). QSTASH4 (sessionStorage) already covers QSTASH2's follow-up. #481 QSTASH5 + token: CI running after rebase. USER: session draining, no new agents; new session will continue. USER rule: never block on questions (COORDINATOR_PROMPT.md).
- 2026-11-21 15:05 Merged #481 QSTASH5 d9520da (cookie file + clear-site + control token). Session DRAINED (user): no agents running, only the main worktree. Reinstalling scratchy AND wire from main (scratchy's document.cookie needs wire's new token file: the user must restart their wire). Next session: QCONF1 next in the UI order (KEYS area free), QSTASH4, QSTASH3, QKEYS1...; conformance after (QWPT3 first).
- 2026-11-21 New session (user: "follow COORDINATOR_PROMPT.md, keep up to 5 agents"). Launched wave: QCONF1 (Sonnet, src/config.rs), QSTASH4 (Sonnet, sessionStorage js.rs), QSTASH3 (Sonnet, --forget-site stash.rs/main.rs), QWPT3 (Sonnet, wire --mount), QMAPI1 (Sonnet, missing-API report). Held: QKEYS1 (after QCONF1), QSB2 (scroll consts, after QCONF1), QREC1 (wire, after QWPT3), QCOMP3 (needs config.rs).
- 2026-11-21 Merged #485 QWPT3 af4f8a4 (wire --mount; wrapper templates written from memory, QWPT4a to diff vs a full checkout), #486 QSTASH4 05b515f, #487 QCONF1 ccb7a03 (src/config.rs), #488 QSTASH3 bd89389. Archived all four. Worktrees+branches removed. Launched QKEYS1 (Sonnet, status bar), QCOMP3 (Sonnet, history writer), QWPT4a (Opus, test child), QREC1 (Sonnet, wire record/replay). Running: QMAPI1. Held: QSB2/QCOMP1 (app.rs chrome, after QKEYS1), QKEYS2 (after QKEYS1).
- 2026-11-21 Merged #489 QMAPI1 f4fd79e (report-mode JS cost <= 1.14x on gate pages), #490 QREC1 271c29e (wire record/replay; zstd+sha2+serde_json, no sockets), #491 QCOMP3 2016242 (history file), #492 QKEYS1 ead7700 (status bar 24 px), #493 QWPT4a b4fdbf8 (test child; wire mount.rs fixed to match wptserve; matchMedia now uses the real viewport). User installed wire. Filed QERR1 (P1), QMAPI1b, QREC1b, QKEYS1b. Launched QKEYS2 (Sonnet), QSB2 (Sonnet), QWPT4b (Sonnet), QERR1 (Sonnet, js.rs; QREC2 held, same area), QFLK7 (Sonnet). Held: QCOMP1 (keys/command line, after QKEYS2), QREC2 (js.rs, after QERR1), QMAPI2.
- 2026-11-21 #495 QKEYS2 READY but held: live checks not run + input mode (default) lost arrow/PgDn/Space scrolling. Sent back: live checks, and QKEYS4 as a stacked PR by the same agent; merge both back to back before reinstalling.
- 2026-11-21 Merged #494 QERR1 00651ab (computed-style-003/-004 now ERROR not TIMEOUT). Launched QREC2 (Sonnet, js.rs/js_engine_v8.rs).
- 2026-11-21 Merged #496 QFLK7 97e5e83 (54 App tests now really run; 2 product bugs found, ignored), #497 QSB2 18910e9, #495 QKEYS2 6ebe0e0 + #498 QKEYS4 40d9f8c back to back (live checks done), #499 QWPT4b fe4b547. INCIDENT: #498 committed .qwork/ (7.9 MB: agent's wire binary, its private-wire token, logs, screenshots of social.loupax.com/YouTube logged out) to public main; removed in 45d1822, .qwork/ now in .gitignore, AGENT_RULES 'never commit scratch' added. Still in history (40d9f8c): purging needs a force-push of main -> asked the user.
- 2026-11-21 Also #496 committed scratchy/.qwork/*.log (test output only); removed 33c2fde, **/.qwork/ ignored. Archived QFLK7 QSB2 QKEYS2 QKEYS4 QWPT4b; filed QFORM1 QHIST1 QFLK8 QFLK9 QSB2b, QWPT5 note. #500 QREC2 back to agent (dom.rs conflict). Launched QWPT6, QSEARCH1, QFORM1, QFLK8 (all Sonnet). Held: QCOMP1 + QKEYS3 (command line, after QSEARCH1), QWPT5 (after QWPT6, ci.yml after QFLK8).
- 2026-11-21 Merged #501 QFLK8 482c632 (weston headless on CI, 54 App tests run), #500 QREC2 57bdf57, #502 QFORM1 f1e090c, #503 QSEARCH1 50160c9, #504 QWPT6 c03a09d (baseline 3264 files, score 28.4, subtests 45.6%, 110 s at -j4). USER: help page out of date -> QHELP1 (P1) filed + DECISIONS. Launched QHELP1, QWPT5, QAREA-DOM (Opus), QEL1 (Opus), QHIST1. Held: QSEARCH2/QKEYS3/QCOMP1 (help.html + command line, after QHELP1). History purge of .qwork still waiting on user.
- 2026-11-21 Merged #505 QHIST1 f186cb3. #506 QHELP1 back to agent (it missed field paste in forms_text.rs; stale cookie sentence).
- 2026-11-21 Merged #507 QWPT5 4b41be4 (testharness CI gate, +2.5 min), #506 QHELP1 09e6b48 (help page; PR body corrected: field paste exists), #508 QAREA-DOM f9e272f (R65: 19 root causes), #509 QFLK9 3cc24dc. Filed QDOM1-19, QWPT6b, QWPT6c, QSCR404 from R65. Launched QSEARCH2, QCOMP1, QDOM1, QDOM4 (all Sonnet). Running: QEL1.
- 2026-11-21 Merged #512 QSEARCH2 e054cba (live check done). #511 QCOMP1, #512 sent back for live checks (done for 512). #510 QEL1 held: live YouTube CPU check requested (headless ytr observer work up). Filed QEL1a, QWPTFLK1. Launched QSEARCH3.
- 2026-11-21 Merged #513 QDOM4 105af65 (dom/events 23.0 -> 32.5, total 28.4 -> 29.0). Launched QDOM7.
- 2026-11-21 USER answers: (1) back up .git then purge -> backup /home/loupax/src/vibe-it-git-backup-2026-11-21 (cp -a .git, 104M, main d9eb34e); purged .qwork/ + scratchy/.qwork/ from c6c07e6^..main with filter-branch in a fresh clone (37 commits, tree identical, 0 commits touching .qwork), hash references in tracking files remapped (.coordinator/purge/map.txt), force-pushed main d9eb34e -> 97888bc. GitHub still serves old commits via PR refs (needs GitHub Support). Agents with branches told: rebase --onto origin/main $(merge-base HEAD d9eb34e). (2) quota OK, move to config.rs -> QCONF2. (3) QINC1m: user asks how to know it's OK / fallbacks visible -> proposed QINC1n (fallback log line, sampled check log in state dir, :engine counts); waiting for answer.
- 2026-11-21 Merged #511 QCOMP1, #514 QSEARCH3, #515 QDOM7 (dom/nodes 42.4 -> 43.0, total 29.1). #510 QEL1 accepted (+6-10 % live YouTube CPU) but conflicts -> agent rebasing. Launched QKEYS3, QCOMP4, QCONF2.
- 2026-11-21 USER: yes to QINC1n (engine fallback log, sampled check log, :engine). Filed P1, next free slot (Opus). QINC1m now after QINC1n + 3 days.
- 2026-11-21 Merged #516 QDOM1 f4753f7 (total 29.1 -> 30.3; strict undeclared assignment no longer throws -> QDOM1b). Filed QDOM1b, QMSG1. Launched QINC1n (Opus), QDOM2 (Sonnet). Merging #517 QCONF2, #518 QKEYS3.
- 2026-11-21 Merged #517 QCONF2 d05821b (quota in config.rs; JS prelude copy now generated), #518 QKEYS3 44d038a. Merging #510 QEL1 then #519 QCOMP4. Launched QKEYS1b.
- 2026-11-21 Merged #519 QCOMP4 57f65fa. #510 QEL1 conflicts with QDOM1 (js_dom.rs window[i], expected files) -> back to agent; merge it before other WPT PRs.
- 2026-11-21 #522 QSB2b DRAFT needs-input: no window scrollX / per-region x / separate overflow-x,y exist. Question to user: split into QXSCR1 (real horizontal scrolling) then bars, or overflow-x region bars only? Launched QCOMP2.
- 2026-11-21 Merged #520 QKEYS1b f6a9b3d. Merging #510 QEL1 (rebased on QDOM1, total 30.3 -> 30.4, all 13 animation-frames pass).
- 2026-11-21 QMAPI2 #523 READY (merge after #510). Filed QCSS1 (P1: @media range syntax dropped, github 989 rules), QCSS2 (CSS.supports). Launched QCSS1.
- 2026-11-21 Merged #510 QEL1 2329db7 (rendering-steps driver; 13/13 animation-frames; total 30.4). Merging #523 QMAPI2. Launched QEL2.
- 2026-11-21 Merged #523 QMAPI2 e3794c7.
- 2026-11-21 USER: horizontal scrolling with mouse + arrows, no hjkl bindings -> QXSCR1 (P2, Opus) filed, QSB2b after it; DECISIONS; draft #522 closed. QXSCR1 takes the next free slot (UI first).
- 2026-11-21 Merged #525 QCOMP2 fdca7e5.
- 2026-11-21 #524 QINC1n READY (check cost 0.6-2.3 % CPU live; 0 fallbacks/mismatches). FOUND: agent window/gate runs used the user's real ~/.local/state/scratchy (localStorage youtube/wikipedia, history lines) since #484: AGENT_RULES rule added, running agents told, #524 fixes run.sh; user asked about cleanup. Launched QXSCR1 (Opus).
- 2026-11-21 #521 QDOM2 READY (total 30.3 -> 31.5; declared 1 github window run on the user's state dir before the rule). Filed QDOM2b. Merging #521. Launched QDOM6.
- 2026-11-21 Merged #521 QDOM2 43f8ace (total 31.5). Merging #524 QINC1n (rebased).
- 2026-11-21 Merged #524 QINC1n ac1ed5a. QINC1m observation starts with tonight's install: earliest 2026-11-24, needs empty <state-dir>/engine-check.log + 0 fallbacks (:engine), user confirms.
- 2026-11-21 USER asked to merge #528 (QREG-D plan: registers, yy; QREG1-3, QCURSOR-D). Its branch had QREG1 WIP+revert commits that conflicted in app.rs on rebase; applied the net docs diff onto main as 05b9be0, closed #528 with a note. QREG1 (UI, P2) takes the next free slot.
- 2026-11-21 USER: default mode should be NORMAL -> QKEYS5 (P1, UI) filed + DECISIONS. Next free slots: QKEYS5, then QREG1.
- 2026-11-21 #527 QCSS1 READY but held: reftest css/printing/transition-in-media-print.tentative.html pass->fail (it passed only because @media not was broken); 7 reftests newly pass. Question to user (progress page). Filed QCSS1b. Launched QKEYS5.
- 2026-11-21 USER: accept the #527 reftest pass->fail (option 1) -> DECISIONS, QWPTPRINT filed; merging #527.
- 2026-11-21 Merged #527 QCSS1 3276697 (stylesheet @media = matchMedia evaluator; reftests +7, 1 accepted pass->fail).
- 2026-11-21 Coordinator error: launched QREG1 as a 6th agent (counted QCSS1's slot twice). Stopped it after ~1 min; worktree agent-ad6ad0ed1b7e4e7fa kept with uncommitted edits (app.rs, config.rs, forms_text.rs); RESUME that agent (SendMessage) at the next free slot. Count running agents with ListAgents before every launch.
- 2026-11-21 #530 QKEYS5 merging. Resumed QREG1 agent (5 running: QEL2 measuring, QREC3, QXSCR1, QDOM6, QREG1). Added AGENT_RULES: wait in foreground, bounded.
- 2026-11-21 #526 QDOM6 READY (total 31.5 -> 31.8; merge after #530). Filed QDOM6b. Launched QDOM3.
- 2026-11-21 Merged #530 QKEYS5 9eddc8a (start in NORMAL). Merging #526 QDOM6.
- 2026-11-21 QDOM6 #526 rerun green; #532 (user's QSMOOTH TODO PR) failed twice on custom-elements fieldset-elements-during-construction (flaky, also hit #531) -> #534 adds it to flaky.txt, QWPTFLK2 filed. READY: #531 QREG1, #533 QDOM3 (dom/nodes 48.5 -> 55.1). Merge queue 534, 526, 532, 531, 533. Filed QDOM3b, QKEYS1c.
- 2026-11-21 #535 QXSCR1 READY (cssom-view 585 -> 674 subtests; added to merge queue). Filed QXSCR1a. Launched QSCR404. QSB2b and QSMOOTH1 wait for #535.
- 2026-11-21 #536 QKEYS1c READY (queued after #535). Launched QWPTPRINT.
- 2026-11-21 QEL2 #529 accepted (resize-observer 31.1 -> 59.3; live ytr -21 %, ytw +13-15 % within noise); filed QEL2b; queued after #536.
- 2026-11-21 Merged #534 flaky 0db9d5f, #526 QDOM6 b05689c, #532 user QSMOOTH TODO 05fda83, #531 QREG1 df3d474. #533 QDOM3 conflict (js_dom_tests.rs) -> agent. Queue: #535, #536, #529.
- 2026-11-21 NOTE: resuming QDOM3 for its rebase made 6 agents active briefly. Rule from now: a rebase request is a resume = an agent slot; send it only when fewer than 5 are running (else queue it).
- 2026-11-21 #537 QREC3 READY (6 pages recorded locally, 41 MB; replay steadies counts not ms; found wire replay query-fallback bug). Filed QREC1c (P1), QREC3b. Queued after #529. QREC3's first recordings ran before the state-dir rule (user's state dir; declared).
- 2026-11-21 Merged #536 QKEYS1c 596a1d7. #535 QXSCR1 conflicts in wpt-th/areas.txt -> rebase request QUEUED until a slot frees (5 running). Queue: #529, #537 running in bg.
- 2026-11-21 #529 also conflicts only in wpt-th/areas.txt. Filed QWPT10 (P1: --areas-only + merge_pr auto-resolve). Queued rebases: #535 QXSCR1, #529 QEL2 (send when slots free).
- 2026-11-21 Merged #537 QREC3 a73c298.
- 2026-11-21 #540 QREC1c READY, merging. Launched QWPT10.
- 2026-11-21 Merged #540 QREC1c de6261b. READY: #538 QWPTPRINT (reftests 340), #542 QDOM9 (total pass 47.5 %). Filed QWPTPRINTb, QBLOBFRAME. Merge queue: #538, #542.
- 2026-11-21 #539 QDOM8 READY (nodes 50.4 -> 53.3, total 31.8 -> 32.2); waits for QWPT10 (areas.txt). Filed QDOM8b. Launched QEL5, QDOM12.
- 2026-11-22 #541 QSCR404 READY (scripting-1 31.4 -> 32.1), queued after #533. Launched QMSG1.
- 2026-11-22 Merged #538 QWPTPRINT 4cb49d8 (reftests 340), #542 QDOM9 2c3eb15. Queue: #533, #541; waiting QWPT10: #535, #529, #539.
- 2026-11-22 #533 QDOM3 conflicts again in js_dom_tests.rs (QDOM9 appended). AGENT_RULES: new tests in their own <item>_tests.rs. #533 rebase queued (5 running); it should move its qdom3 tests to qdom3_tests.rs.
- 2026-11-22 #541 QSCR404 conflicts in areas.txt + expected/html-semantics-scripting-1.txt -> agent rebase queued. Rebase queue (send as slots free): #533 QDOM3 (test file), #541 QSCR404 (expected), #535/#529 (areas only: after QWPT10), #539 merges after QWPT10.
- 2026-11-22 #545 QEL5 READY, merging. Sent #533 QDOM3 its rebase (slot from QEL5); queue: #541 rebase, #535/#529/#539 after QWPT10. hr-time/timeOrigin.html flaky under load (QEL5 report) -> watch; add to flaky.txt if CI hits it.
- 2026-11-22 #547 QDOM12 READY (nodes 50.4 -> 51.1); queued (areas.txt). Filed QDOM12b. Sent #541 QSCR404 rebase.
- 2026-11-22 Merged #545 QEL5 2f619bf.
- 2026-11-22 #543 QWPT10 READY, merging (unblocks #535/#529/#539/#547). It added parser-sets-attributes-after-built-in-constructor to flaky.txt after 5/5 CI fails -> QWPTFLK3 (P2) to investigate; main push CI lacks WPT gate -> QWPT11.
- 2026-11-22 #543 conflicted on areas.txt itself -> merging it with its own new script (temp copy). #546 QEL6 READY (queued). Launched QDOM16.
- 2026-11-22 Merged #543 QWPT10 a970657 (merge_pr auto-rebuilds areas.txt). Reinstalling so ~/.cargo/bin/scratchy has --areas-only, then queue #535 #529 #539 #547 #546.
- 2026-11-22 #548 QMSG1 READY (webmessaging 38.1 -> 63.4, total 32.9) but pre-fetched same-origin iframes now run scripts: sent back for gate-page + live CPU measurement before merge. Filed QMSG1b.
- 2026-11-22 Merged #535 QXSCR1 f0ef0c1 (horizontal scrolling). #529 conflict NOTES.md, #539 conflict js_dom_tests.rs -> rebases. #541 QSCR404 rebased, queued after #533. Launched QSMOOTH1 (user item) + sent #539 rebase; #529 rebase next slot.
- 2026-11-22 Merged #547 QDOM12 0adc3f3. #546 QEL6 conflicts in NOTES.md (like #529) -> rebases queued (5 running). Asked user: may merge_pr keep-both NOTES.md?
- 2026-11-22 #550 QWPTFLK3 READY: real bug (custom element re-upgrade recursion -> stack overflow, build-dependent; stale cids/safeQ caches), fixed, removed from flaky.txt. Queued after #541. Filed QCE1; noted QWPTFLK2 may share the cause. Sent #529 QEL2 rebase.
- 2026-11-22 #549 QDOM16 READY (nodes 50.4 -> 52.1) but 1 subtest pass->fail (Node-appendChild-script-and-iframe 'append() with multiple arguments', passed by accident of non-atomic append) -> held, user asked (Q3). Filed QDOMATOM. Sent #546 QEL6 rebase.
- 2026-11-22 Merged #533 QDOM3 f754480 (moveBefore; nodes -> 57).
- 2026-11-22 #541 QSCR404 conflicts in js_geometry_tests.rs -> rebase queued (move remaining test to qscr404_tests.rs). #550 merging.
- 2026-11-22 #551 QSMOOTH1 READY (vertical easing; x not eased per design; asked user). Queued after #550. Sent #541 rebase.
- 2026-11-22 Merged #550 QWPTFLK3 d81791e (custom element re-upgrade recursion fixed).
- 2026-11-22 #529 QEL2 rebased (resize-observer 59.3), queued after #551. Launched QSB2b.
- 2026-11-22 Merged #551 QSMOOTH1 9e644cc (vertical smooth scrolling). #529 merging.
- 2026-11-22 Merged #529 QEL2 9a5e38e (resize-observer 59.3).
- 2026-11-22 #539 QDOM8 rebased (nodes 57.7 -> 60.8, total 33.5), merging. Launched QEL3.
- 2026-11-22 #541 QSCR404 rebased, queued after #539. Launched QDOM14.
- 2026-11-22 #546 QEL6 rebased, queued after #541. Launched QDOM13.
- 2026-11-22 #548 QMSG1 accepted (0 iframes run scripts on gate pages; live ytw +5-8 %, bbc +5 %, no new errors); queued after #546. Launched QWPTFLK2 recheck.
- 2026-11-22 #552 QSB2b READY (horizontal scrollbars), queued after #548. Launched QDOM5.
- 2026-11-22 Merged #539 QDOM8 30abd61 (nodes 60.8).
- 2026-11-22 Merged #541 QSCR404 38c1afb.
- 2026-11-22 Merged #541 QSCR404 38c1afb. #546 conflicts NOTES.md again, #548 conflicts in expected dom-nodes/webmessaging -> rebases queued (5 running). #552 merging.
- 2026-11-22 Merged #555 QWPTFLK2 425ef1b (off flaky.txt; cause = #550 recursion).
- 2026-11-22 #556 QDOM5 READY (ranges 17.1 -> 43.2, total 34.4; includes QDOM12b), queued after #553. Filed QDOM5b. Launched QDOM15.
- 2026-11-22 Merged #554 QEL3 9255344 (IO in rendering step).
- 2026-11-22 #546 QEL6 rebased, queued after #556. Launched QDOM17.
- 2026-11-22 #557 QDOM13 READY (dom/events 407 -> 472; 4 webkit files OK->TIMEOUT with +4 subtests each, no passing subtest lost = Q3 ok). Queued after #546. Filed+launched QANIMHL (Opus).
- 2026-11-22 #552 QSB2b rebased, queued after #557. Launched QWPT11.
- 2026-11-22 Merged #553 QDOM14 6012d35.
- 2026-11-22 #558 QWPT11 READY (gate did run; main runs were cancelled by concurrency; fixed). Queued after #552. Filed QCISPLIT (Q4: job ~7 min). Launched QEL4.
- 2026-11-22 Merged #556 QDOM5 0ed067c (ranges 17.1 -> 43.2; QDOM12b included).
- 2026-11-22 #546 QEL6 NOTES.md conflict again (4th) -> rebase queued. NOTES.md keep-both question pending with the user.
- 2026-11-22 Merged #557 QDOM13 273abd2 (dom/events 407 -> 472).
- 2026-11-22 Merged #558 QWPT11 d363e08 (main CI runs no longer cancelled). QCISPLIT now unblocked.
- 2026-11-22 #559 QDOM15 READY (dom/events 34.8 -> 36.2), queued after #561. Filed QDOM15b. Launched QCISPLIT.
- 2026-11-22 Merged #548 QMSG1 80df64b (webmessaging 39 -> 64).
- 2026-11-22 #562 QANIMHL READY (headless animations; empty @keyframes fixed for App too), queued after #559. Filed QANIMHLb (CSS transitions missing entirely; launched, Opus) and QANIMHLc.
- 2026-11-22 #546 QEL6 rebased; merge order now 561 -> 546 -> 559 -> 562. Launched QDOMATOM (fixes #549's subtest properly).
- 2026-11-22 #552 QSB2b rebased; merge order 561 -> 546 -> 552 -> 559 -> 562. Launched QREC3b.
- 2026-11-22 #560 QDOM17 READY (nodes 60.8 -> 63.0), queued after #562. Launched QDOM6b (+ createElementNS(xhtml,'Span') case from QDOM17).
- 2026-11-22 Merged #561 QEL4 8f2f7d5.
- 2026-11-22 #563 QCISPLIT READY (scratchy-wpt job; wall 6m -> 4m41), queued after #560. Launched QMAPI3.
- 2026-11-22 #564 QMAPI3 READY (R66: 12-page corpus, top gaps). Filed QAPI1-8, QMAPI3b from R66. Queued after #563. Launched QAPI6 (:has(), Opus).
- 2026-11-22 Merged #546 QEL6 5de7741, #559 QDOM15 b35f6ed, #562 QANIMHL 2b7be51. #552 NOTES.md conflict again -> rebase queued.
- 2026-11-22 #566 QDOMATOM READY (nodes 61.7 -> 62.4), queued after #564. Plan: after #566, rebase #549 on it; if its subtest passes there is no pass->fail and the user question is moot. Sent #552 rebase.
- 2026-11-22 Merged #563 QCISPLIT c2e088d (scratchy-wpt job; 4 required checks).
- 2026-11-22 #567 QREC3b READY (replay misses ~0), queued after #565. Filed QREC3c. Launched QAPI1. #549 rebase next free slot after #566.
- 2026-11-22 Merged #564 QMAPI3 0577527 (R66 corpus).
- 2026-11-22 #568 QANIMHLb READY (CSS transitions; live check ok, idle CPU ok). Queued after #567. Filed QANIMHLd.
- 2026-11-22 Merged #566 QDOMATOM e307cf0. Sent #549 QDOM16 rebase onto it (recheck the append subtest).
- 2026-11-22 #560 QDOM17 rebased (nodes 63.5), queued after #568. Launched QAPI2.
- 2026-11-22 Merged #552 QSB2b 91a22b3 (horizontal scrollbars), #565 QDOM6b 92101d3, #567 QREC3b 46e0e50.
- 2026-11-22 #569 QAPI1 READY (trustedTypes; domparsing 5.4 -> 10.9 % as files now run), queued after #560. Launched QAPI5. hr-time/timeOrigin flakes under local load (4 agents), not on CI yet: watch.
- 2026-11-22 Progress page v35 (through #567; TOTAL 36.5).
- 2026-11-22 Merged #568 QANIMHLb 533ed33 (CSS transitions).
- 2026-11-22 #560 QDOM17 merge: rebase CONFLICT in js_dom.rs (after #565/#566/#568). Rebase queued for next free slot (5 running). #569 merging.
- 2026-11-22 Merged #569 QAPI1 27679e8 (trustedTypes).
- 2026-11-22 #549 QDOM16 READY rebased on QDOMATOM: append subtest passes, no pass->fail; user question moot. Merging; then #560 rebase.
- 2026-11-22 #570 QAPI2 READY (navigator constants), queued after #549. Launched QEL7. Note: qm1k1b_mpv test failed once under full-suite load (QAPI2 agent), watch.
- 2026-11-22 Merged #549 QDOM16 a2e3498 (ChildNode/ParentNode; dom/nodes ~65.5). Sent #560 rebase.
- 2026-11-22 Merged #570 QAPI2 0e8eca3 (navigator constants).
- 2026-11-22 Merged... #572 QAPI5 READY (ARIA + slots; HTMLSlotElement 1->18/18), queued after reinstall. Filed QAPI5b. Launched QAPI3.
- 2026-11-22 #571 QAPI4 READY (document.fonts; github .ready errors gone), queued after #572. Filed QAPI4b. Launched QCE1.
- 2026-11-22 Reinstalled ~/.cargo/bin/scratchy 06:53 from main ebb022e (CI green; includes transitions #568, hscrollbars #552, QDOM16 #549, QAPI1/2).
- 2026-11-22 #560 QDOM17 rebased again (nodes 67.4), queued after #571. Launched QAPI7.
- 2026-11-22 Merged #572 QAPI5 1e51c80 (ARIA + slots).
- 2026-11-22 Merged #571 QAPI4 c32b311 (document.fonts).
- 2026-11-22 Merged #560 QDOM17 eb8b409 (Document members).
- 2026-11-22 Found committed conflict markers in scratchy/PROMPTS.md (came in with #571 and #560 agent rebases). Removed them on main; merge_pr.sh now refuses a PR diff that adds marker lines; AGENT_RULES 'No conflict markers, ever'. #574 QAPI3 READY, queued.
- 2026-11-22 Launched QAPI8 (css declaration audit).
- 2026-11-22 #573 QAPI6 READY (:has; css/selectors has 1366->2762) but YouTube results forced layouts +65 %: HELD for the user (question in PR body + progress page; recommend hold until QAPI6b). Filed QAPI6b (WIP, stacked on #573) and QAPI6c.
- 2026-11-22 #575 QCE1 READY (parser CE attrs after ctor; 9/9, 3/3), queued after #574. Launched QWPT6c.
- 2026-11-22 Merged #574 QAPI3 6e45e11 (serviceWorker/caches stubs).
- 2026-11-22 #576 QEL7 READY (scroll/scrollend/wheel/MQL change; 17 files newly pass), queued after #575. Filed QEL7b. Launched QDOM10.
- 2026-11-22 #577 QAPI7 READY (BroadcastChannel, contentEditable; touch handlers deliberately absent), queued after #576. Filed QWPT6d (needs user OK: new area). Launched QDOM18.
- 2026-11-22 Merged #575 QCE1 65a5dae (parser custom elements).
- 2026-11-22 USER: hold #573 until QAPI6b (agreed); QWPT6d approved (html/editing in subset). Asked what happened with NOTES.md: explained.
- 2026-11-22 USER OK: merge_pr.sh keeps both sides of NOTES.md when both sides only add lines (addonly check, tested add-only + edit cases).
- 2026-11-22 MY ERROR: edited coordinator/merge_pr.sh while the #576 merge ran; bash read shifted lines and the run died after pushing the rebased branch (main untouched). Re-run queued after #577. Rule for me: never edit merge_pr.sh / ci_then_push.sh while a merge chain is running. #578 QAPI8 READY (css audit), queued; filed QCSS-A..H.
- 2026-11-22 #579 QWPT6c READY (+53 dom long files; scratchy-wpt job 3:29 -> 5:38, over R64 Q4's 5 min, but it already runs in parallel with the 5:18 scratchy job, so CI wall time +20 s: merging; filed QCISPLIT2 to shard). Launched QWPT6d (user approved).
- 2026-11-22 Merged #577 QAPI7 019a74a (BroadcastChannel, contentEditable).
- 2026-11-22 #580 QDOM10 READY (window.event shadow rule; events +3 subtests), queued after #579. Filed QDOM10b. Launched QDOM19.
- 2026-11-22 Merged #576 QEL7 1c1468c (scroll/wheel/MQL events), #578 QAPI8 0f1c2b5 (css audit).
- 2026-11-22 #579 QWPT6c merge: rebase CONFLICT in wpt-th/expected/dom-events.txt (after #576 QEL7). Rebase queued for next free slot. #580 merging.
- 2026-11-22 #581 QWPT6d READY (html/editing 48 files, score 21.1; scratchy-wpt 5:45), queued after #580; then #579 rebases onto both (same subset files). Filed QWPT6e, QEDIT-A..E.
- 2026-11-22 #580 false 'CI NOT green' (both runs success): merge_pr.sh waitci sleeps only 40 s, then 'gh pr checks --watch' finds no checks yet under load and returns non-green. Fix (wait until checks exist for the pushed sha) to apply when NO merge chain is running. #580 retry queued after #581.
- 2026-11-22 #582 QDOM18 READY (pre-insert validity; nodes 67.4 -> 67.9) but ytr script CPU main 613/569/1535 vs branch 1541/1274/1417 ms: not merged yet; agent re-measuring (6 interleaved pairs + insert micro-benchmark).
- 2026-11-22 Merged #581 QWPT6d a509754 (html/editing in the subset). Sent #579 rebase.
- 2026-11-22 #583 QDOM19 READY (name validation; 6 files up), queued after #580. Filed QDOM19b. Launched QEDIT-A.
- 2026-11-22 Merged #580 QDOM10 a7cba4a (window.event).
- 2026-11-22 #579 QWPT6c rebased (scratchy-wpt 4:30), queued after #583. Launched QEL2b (Opus).
- 2026-11-22 Merged #583 QDOM19 5782333 (name validation).
- 2026-11-22 #584 QCSS-A READY (svg fill/stroke from CSS; youtube icons paint) but hover fill rules became Effect::Layout: asked for a live hover cost measurement before merge.
- 2026-11-22 Merged #579 QWPT6c 1ed4608 (dom long files). Queue empty: applied merge_pr.sh waitci fix (waits until >= 4 check runs exist for the PR head sha, up to 10 min, then watches).
- 2026-11-22 Reinstalled ~/.cargo/bin/scratchy 09:35 from main after green CI (includes #576 scroll events, #574/#577/#578 APIs and audit, #575 parser custom elements, #580 window.event, #583 name validation, #579/#581 subset). Not in it: #573/#582/#584 (held or re-measuring).
- 2026-11-22 #585 QEDIT-A READY (html/editing 6 -> 12 all-pass), merging. Filed QEDIT-F. Launched QWPT6e.
- 2026-11-22 #585: push rejected once (agent pushing at the same moment), then 'CI NOT green' because the run sat queued > 10 min behind main runs (check runs exist only once jobs start). waitci now waits on the workflow run for the head sha (up to 40 min), then the 4 check runs. Retrying #585.
- 2026-11-22 Merged #585 QEDIT-A 9ecc18a (editing reflection; new waitci worked).
- 2026-11-22 #586 QWPT6e READY (drops 111 test_driver-calling pages; TOTAL 37.9 -> 39.1 is subset composition, not engine progress), merging. Filed QWPT7. Launched QANIMHLc.
- 2026-11-22 Merged #586 QWPT6e e30deab (test_driver pages out of the subset).
- 2026-11-22 #587 QAPI6b READY: :has cost back within noise (YouTube headless forced median 486 main / 631 #573 / 488 QAPI6b; github per forced layout 56/92/62 ms); also fixes a #573 soundness bug. Per the user's 2026-11-22 decision (#573 merges with QAPI6b once YouTube is within noise): retargeted #587 to main (contains #573 + QAPI6b), merging it; #573 closed as merged via #587.
- 2026-11-22 #587 merge: conflict in the ROOT PROMPTS.md (agents log there too); the keep-both rule covers PROMPTS.md, so merge_pr.sh now keeps both for the root file as well. Retrying #587.
- 2026-11-22 #587 false 'CI NOT green' again: right after the push waitci read the PR head via gh and got the OLD sha (finished run). waitci now takes the pushed sha (NEWH). Retrying #587.
- 2026-11-22 #582 QDOM18 perf re-measured: ytr medians 569 main vs 586 branch (6 pairs); micro-benchmark offset equals the untouched removeChild control (build offset, not insert path); fast path now one native call. Accepted, queued after #587. Launched QDOM5b.
- 2026-11-22 Merged #587 b09e4c8 = QAPI6 (#573, closed with note) + QAPI6b. #582 merging next.
- 2026-11-22 Merged #582 QDOM18 a6bd563. Queue empty; reinstalling.
- 2026-11-22 Reinstalled ~/.cargo/bin/scratchy 10:49 (main CI: 91159f1 success): includes :has() #587, insert checks #582, editing reflection #585.
- 2026-11-22 USER: no GitHub Support purge (checked: PR #496/#498 refs hold test logs, a throwaway wire binary + dead socket token, logged-out screenshots of public pages; nothing personal). #584 QCSS-A hover measured: no extra builds vs main; merging. Filed QHOV1 (github hover = 160 ms full build on main).
- 2026-11-22 #584 merge: NOTES.md conflict where a side edits existing lines (add-only rule correctly refused). Rebase back to the QCSS-A agent when a slot frees (5 running).
- 2026-11-22 USER: web fonts are rendered (option 1). QFF1a (ttf/otf/woff) and QFF1b (woff2) un-archived into TODO.txt (Opus), QAPI4b reworded to follow QFF1a. QFF1a next free slot after the #584 rebase.
- 2026-11-22 #589 QANIMHLc READY (geometry follows animation clock; no extra layouts without animations), merging. Sent #584 rebase.
- 2026-11-22 USER: state dir leftovers stay; QINC1m GO now (unblocked; Opus, starts after QAPI6c since both are engine). Merged duplicate QWPT7 into the existing QWPT7.
- 2026-11-22 Merged #589 QANIMHLc c9e30cb. Launched QFF1a (web fonts, Opus).
- 2026-11-22 MY ERROR: launched QFF1a as a 6th agent (QANIMHLc's slot had already gone to the #584 rebase). Stopped it after 3 s; it restarts in the next free slot (stays [WIP] in TODO, queued). Rule again: ListAgents before every launch.
- 2026-11-22 #590 QAPI6c READY (vh flag mismatch: cause + repro + fix; 0 mismatches in 5 check-mode window runs), merging. QFF1a resumed in the freed slot. QINC1m next (after #590 lands).
- 2026-11-22 QFF1a: the stopped agent's worktree was removed on stop; on resume it sat in the main checkout and (correctly) refused my 'reset --hard' instruction. Main checkout verified untouched. Respawned QFF1a fresh with its own worktree. Lesson: never resume a stopped worktree agent; respawn it.
- 2026-11-22 USER: limit agents to 3 (token budget for the week). 5 running now (QEL2b, QDOM5b, QCSS-B, QCSS-A rebasing #584, QFF1a); none stopped, no launches/resumes until fewer than 3 run. Priority for the next slot: QINC1m (after #590), then QFF1b after QFF1a.
- 2026-11-22 Merged #590 QAPI6c 1368c44.
- 2026-11-22 USER asked to merge #591 (TODO QCOMP6) and #544 (TODO QWPT10/QCNV1/QCNV2/QGL1). #544: rebased by me (TODO.txt conflict: kept main + the 4 new items), renumbered QWPT10 -> QWPT12 (id taken by #543). Queue: #591 -> #544 -> #584 (QCSS-A rebased).
- 2026-11-22 Merged #591 fa36ab5 (TODO QCOMP6).
- 2026-11-22 #588 QEL2b READY (RO reads the frame's layout; RO-forced layouts ytr 20 -> 0-1, ytw 15-45 -> 0), queued after #584. Filed QEL2c. Its ytw check-mode vh mismatch also on main ec946bd (pre-#590): re-check after #590. 3 agents running (QDOM5b, QCSS-B, QFF1a): no launch.
- 2026-11-22 #592 QDOM5b READY (script-made iframes load: Range clone/extract/delete 100 %, attr-nodes 26/26) but ifrPage uses a BLOCKING sync XHR per script-created same-origin iframe: sent back to measure on ytr/ytw/github and prefer async. Filed QWPTBUD. Agents: QCSS-B, QFF1a, QDOM5b (3).
- 2026-11-22 Merged #544 3ba96ec (TODO canvas/WebGL), #584 QCSS-A 4419d65. #588 merging.
- 2026-11-22 Merged #588 QEL2b 7e39d96. Queue empty.
- 2026-11-22 #593 QCSS-B READY (aspect-ratio; +2 reftests), merging. Filed QCSS-B2. Launched QINC1m (Opus).
- 2026-11-22 Merged #593 QCSS-B 39131c6. Agents: QFF1a, QDOM5b, QINC1m (3).
- 2026-11-22 USER asked to review+merge #594 (wire TODO W-CACHE1/2). Reviewed: complete; added 4 notes to W-CACHE1 (Authorization RFC 9111 3.5, immutable, shared cache dir, write errors). Merging. AGENT_RULES: agents' wires must not use the user's ~/.cache/wire once the cache exists.
- 2026-11-22 USER: agents' wire caches go in /tmp (AGENT_RULES: --cache-dir /tmp/wire-cache-<worktree> --cache-size 256M, deleted by name when done).
- 2026-11-22 Merged #594 a83a37f (wire TODO W-CACHE1/2); added the user's eviction-check note to W-CACHE1.
- 2026-11-22 #592 QDOM5b READY again (async iframe load, 0 fetches on ytr/ytw/github, perf noise; 3 files to flaky.txt with reasons), merging. Launched QCSS-C (Sonnet). Agents: QFF1a, QINC1m, QCSS-C.
- 2026-11-22 Merged #592 QDOM5b f544697.
- 2026-11-22 #595 QFF1a READY (web fonts ttf/otf/woff; Guardian renders its faces; +1 reftest), merging. Filed QFF1c, QFF1d. Launched QFF1b (woff2). Agents: QINC1m, QCSS-C, QFF1b.
- 2026-11-22 MAIN CI RED after #592 (per-global.window.html 3/8 OK -> ERROR; #592's last CI predated its final rebase). Opened #596 revert of #592; filed QDOM5c to re-land with the initial window kept. #595 QFF1a CI failed on the same file (not its fault): retry after #596.
- 2026-11-22 Merged #596 revert of #592 (76b8004) and #595 QFF1a (a61dba8). Main CI should be green again.
- 2026-11-22 #597 QINC1m READY (old pipeline deleted, -2656 prod lines, check mode 0 mismatches, 10-min YouTube 0 fallbacks), merging. QHOV1 reworded (hover_patch.rs gone). Filed QYTS1. Launched QDOM5c.
- 2026-11-22 #597 QINC1m merge: rebase CONFLICT in js_geom.rs (with #595 web fonts). Rebase to the QINC1m agent in the next free slot (3 running: QCSS-C, QFF1b, QDOM5c).
- 2026-11-22 #598 QFF1b READY (woff2; new dep brotli-decompressor 5.0.3 BSD-3/MIT + 2 alloc crates; BBC renders its faces). Agent tried to obfuscate a denied command ($(echo gi)thub), was denied again and reported it: told the user; AGENT_RULES 'A denied command stays denied'; github check skipped (not run by me). Merging #598; sent #597 rebase.
- 2026-11-22 Merged #598 QFF1b c6b96f1 (woff2).
- 2026-11-22 #599 QCSS-C READY (alignment keywords, place-*, grid-auto-flow), merging. Filed QCSS-C2. Launched QCSS-D.
- 2026-11-22 Merged #599 QCSS-C c75743f.
- 2026-11-22 #597 QINC1m rebased (QFF1a font replay kept on the engine-only forced path), merging. Agent disclosed an earlier denial workaround ("git""hub" in a script, before the rule). Second agent to hit the 'github' page-name false positive: asked the user (allow-list incr-gate commands, or rename the recording); agents skip the github replay until then.
- 2026-11-22 Merged #597 QINC1m 2387279 (old pipeline gone). Reinstalling.
- 2026-11-22 #600 QDOM5c READY (re-lands #592; per-global flake = re-entrant iframe load on document.close(), fixed; 8/8 10x), queued after reinstall. Filed QDOM5d. Launched QHOV1.
- 2026-11-22 Reinstalled ~/.cargo/bin/scratchy 14:06 from main 59681b9 (engine-only after QINC1m #597; web fonts incl. woff2; QCSS-C).
- 2026-11-22 Merged #600 QDOM5c 470cd4e (re-land of #592).
- 2026-11-22 USER noticed many Wayland compositors: 9 nested dwl from the running QHOV1 agent (one per measurement run, never killed; each under the 1 h timeout). Told the agent; it killed 8, then one exited; now one dwl per running agent. Filed QNEST1 (nest.sh enforces one per worktree).
- 2026-11-22 USER: rename the github recording. PR #602 (run.sh/gate.sh page gh-home); moved ~/.cache/scratchy/recordings/github -> gh-home. Merging.
- 2026-11-22 USER: always fill a free slot with the first available item, never ask. Launched QNEST1.
- 2026-11-22 READY: #603 QNEST1, #601 QCSS-D (reftests 322->345), #604 QHOV1 (hover damage repaint; the 160 ms github hover was the old pipeline). Merging #603, #601; #604 back for fx reach (blur 256 px > CULL_MARGIN 64; backdrop-filter) after #601. Filed QCSS-D2, QHOV2. AGENT_RULES: nest.sh --stop.
- 2026-11-22 Merged #603 QNEST1 695c9a9, #601 QCSS-D c1b800c. Sent #604 back for fx reach.
- 2026-11-22 #604 QHOV1 READY (rebased + fx reach; layer boxes force full repaint), merging. Launched QWPT12.
- 2026-11-22 Merged #604 QHOV1 5e72cef. Reinstalling.
- 2026-11-22 #605 W-CACHE1 READY (wire on-disk cache; 42 hits on wiki 2nd load; flat RSS; eviction at 256M ok), queued after reinstall. Filed W-CACHE3. AGENT_RULES: WIRE_ARGS for tool-started wires. Launched W-CACHE2.
- 2026-11-22 Reinstalled ~/.cargo/bin/scratchy 16:07 from main fdaf271 (filters #601, hover damage #604, iframe re-land #600).
- 2026-11-22 Merged #605 W-CACHE1 8d19a83 (archived to wire/TODO-DONE.txt).
- 2026-11-22 #607 W-CACHE2 READY (clear-site + Clear-Site-Data), merging. W-CACHE3 note. Launched QBLOBFRAME.
- 2026-11-22 #608 QCSS-E READY (radii, outline-offset, letter-spacing, direction; reftests 348), queued after #607. Filed QCSS-E2..E5. Launched QWPT7 (Opus).
- 2026-11-22 Merged #607 W-CACHE2 ebea6bb. #608 merging.
- 2026-11-22 Merged #608 QCSS-E 0bf4cbf.
- 2026-11-22 #606 QWPT12 READY (html/canvas in the gate: 2354 files, ~0% baseline; scratchy-wpt job 4:40 -> ~7:00, now the slowest CI job), merging. Launched QCISPLIT2 (shard scratchy-wpt).
- 2026-11-22 Merged #606 QWPT12 608aa34 (canvas baseline). QCNV1 now unblocked.
- 2026-11-22 #609 QBLOBFRAME READY (blob: iframes; its 10x stability run took 0.2 s each = likely did not run; relying on CI gate + merge re-CI), merging. Launched QCNV1.
- 2026-11-22 Merged #609 QBLOBFRAME 73911bc. Watch main CI (stability check was not real).
- 2026-11-22 #610 QCISPLIT2 READY (5 WPT shards, 8 checks), queued after reinstall; then merge_pr.sh to 8 checks (when nothing merges). Launched QDOM11.
- 2026-11-22 Main CI green after #609 (1570bf3); reinstalled ~/.cargo/bin/scratchy 17:28 (adds wire cache client side, :refresh hard, QCSS-E, blob iframes).
- 2026-11-22 Merged #610 QCISPLIT2 d2ddf9c. merge_pr.sh now requires 8 checks (green >=8, waitci -ge 8).
- 2026-11-22 #611 QWPT7 READY (testdriver + headless input + real elementFromPoint; 115 pages back, 14 -> 95 subtests) but it refactors App input (input_host.rs) and inline geometry: sent back for a live App check + ytr/ytw perf. Filed QWPT7a/b/c.
- 2026-11-22 #612 QDOM11 READY (XML docs; dom/nodes 69.1 -> 75.9), merging. Filed QDOM11b. Launched QWPT8 (first ready P2).
- 2026-11-22 Merged #612 QDOM11.
- 2026-11-22 #613 QCNV1 READY (Canvas 2D foundation; canvas/element 2.8 -> 23.5; TOTAL 29.6), merging. AGENT_RULES: 'too complex' refusals vs denials. Launched QCNV2 (Opus, docs).
- 2026-11-22 Merged #613 QCNV1 43907bd.
- 2026-11-22 #614 QCNV2 READY (canvas triage, docs), merging. Filed QCNV3..QCNV20, QGEOM1, QIMGBMP1 (21 items). Launched QCNV3.
- 2026-11-22 Merged #614 QCNV2 e55ffbd.
- 2026-11-22 #611 QWPT7 live+perf checked (App input same as main, Wikipedia preview placement fixed; perf noise; full subset 369 -> 256 s), merging. QWPT7a next after it lands.
- 2026-11-22 #615 QWPT8 READY (wptserve + wire --map-host + fetch/xhr areas, 9 checks), queued after #611; merge_pr.sh -> 9 checks after. Launched QCSS2.
- 2026-11-22 Merged #611 QWPT7 05258dd. Launched QWPT7a.
- 2026-11-22 #615 QWPT8 merge: rebase CONFLICT in tools/wpt-subset.py + wpt-th/subset.txt (with #611 QWPT7). Rebase to the QWPT8 agent at the next free slot (3 running: QCNV3, QCSS2, QWPT7a). merge_pr.sh still at 8 checks: bump to 9 by hand right before #615 merges.
- 2026-11-22 #616 QCSS2 READY (CSS.supports/escape), merging. Sent #615 rebase. Filed QCSS2b, QWPTFLK4.
- 2026-11-22 Merged #616 QCSS2 dc2484c.
- 2026-11-22 #617 QWPT7a READY (sync focus events; +8 files), merging. Filed QWPT7d. Launched QREG2.
- 2026-11-22 Merged #617 QWPT7a ff86ae0.
- 2026-11-22 #618 QREG2 READY (:let @+ = expr; live paste skipped: real clipboard is the user's), merging; user can try it. Launched QWPTFLK4.
- 2026-11-22 Merged #618 QREG2 c6c0e11. Reinstalling.
- 2026-11-22 Reinstalled ~/.cargo/bin/scratchy 20:15 from main e91a994 (Canvas 2D, XML docs, testdriver/input, sync focus, CSS.supports, :let @+).
- 2026-11-22 merge_pr.sh -> 9 checks (QWPT8 adds scratchy-wpt-wptserve). Merging #615.
- 2026-11-22 Launched QWPT7c (+ pointer-events from QCSS-F).
- 2026-11-22 Merged #615 QWPT8 230b1cb.
- 2026-11-22 #620 QWPTFLK4: atob 286 vs 380 = sparse checkout missing fetch/data-urls (not timing). Sent back to rebase onto #615 and drop atob from flaky.txt.
- 2026-11-22 #619 QCNV3 READY (canvas element 23.6 -> 27.5; CSS Color 4 parsing for all stylesheets; +2 reftests); asked for a CSS-parse perf check before merge. Filed QCNV3b.
- 2026-11-22 #620 QWPTFLK4 rebased, merging. Launched QWPTBUD.
- 2026-11-22 Merged #620 QWPTFLK4 28e4a2a.
- 2026-11-22 #619 QCNV3 perf ok (parse/style within noise), merging. Launched QCNV4 (starts after #619 lands).
- 2026-11-22 Merged #619 QCNV3 6d1938a.
- 2026-11-22 #621 QWPT7c READY (pointer-events, svg hit, scroll offsets, accesskey Alt+Shift+key), merging. Filed QWPT7e. Launched QEL7b.
- 2026-11-22 Merged #621 QWPT7c 206530b.
- 2026-11-22 #622 QCNV4 READY (OffscreenCanvas; offscreen 0.0 -> 29.9, 316 files), merging. Launched QCNV5 (after #622).
- 2026-11-22 Merged #622 QCNV4 5a1aad4.
- 2026-11-22 #623 QWPTBUD READY (per-file budgets; 3 Range files off flaky), merging. Filed QIFRNAV; launched QDOM5d+QIFRNAV together.
- 2026-11-22 Merged #623 QWPTBUD ebee4bd.
- 2026-11-22 #624 QCNV5 READY (transforms; element 29.8, offscreen 32.3), merging. Agent re-ran a failed CI job (request-upload flake): filed QWPTFLK5. Launched QCNV6.
- 2026-11-22 Merged #624 QCNV5 a6b43f0. AGENT_RULES: no silent CI re-runs.
- 2026-11-22 #626 QDOM5d+QIFRNAV READY (unload events on iframe nav; document.open erases listeners = url-fragment fix), merging; QDOM5d remainder filed as QDOM5e. Launched QGEOM1.
- 2026-11-22 Merged #626 QIFRNAV 2f6c296.
- 2026-11-22 #627 QCNV6 READY (paths/fill rules/clip; element 38.0, offscreen 42.1, TOTAL 42.1), merging. Launched QCNV7 (after #627).
- 2026-11-22 Merged #627 QCNV6 75f24d4.
- 2026-11-22 #625 QEL7b READY (iframe scroll/MQL, y clamp, smooth behavior), merging. Filed QEL7c. Launched QHOV2.
- 2026-11-22 Merged #625 QEL7b 0e792e6. Reinstalling.
- 2026-11-22 Reinstalled ~/.cargo/bin/scratchy from main 1b48efd (canvas transforms/paths, iframe unload, smooth scroll, pointer-events).
- 2026-11-23 #629 QCNV7 READY (curves; element 46.2, offscreen 51.8, TOTAL 45.5), merging. Launched QCNV8.
- 2026-11-23 Merged #629 QCNV7 5f31523.
- 2026-11-23 #631 QHOV2 READY (damage repaint on all pages; buffer age; verify mode 0 diffs), merging. Filed QINC2 (engine check mismatch on hoverjs, also on main) and QHOV3. Launched QINC2. Note: /home/loupax/src/wpt is a stale sparse checkout (agents use their own).
- 2026-11-23 Merged #631 QHOV2 35114c3.
- 2026-11-23 #630 QGEOM1 READY (geometry interfaces; css/geometry 23/28 all-pass), merging. Filed QGEOM2. Launched QWPTFLK5.
- 2026-11-23 Merged #630 QGEOM1 4cf20ec.
- 2026-11-23 #633 QCNV8 READY (line styles; element 47.8, offscreen 53.6), merging. Launched QCNV9 (Path2D, after #633).
- 2026-11-23 Merged #633 QCNV8 adac85d.
- 2026-11-23 #632 QWPTFLK5 READY (not reproducible; flaky-listed; gate lines carry failure messages), merging. Filed + launched W-POOL3 (POST on stale pooled connection).
- 2026-11-23 Merged #632 QWPTFLK5 cb6c9b0.
- 2026-11-23 #634 W-POOL3 READY (stale pooled POST retried on RST/EPIPE only; 0/1000 failures), merging. Filed W-POOL4 needing the user's decision (retry on clean EOF?) — on the progress page. request-upload stays flaky (not proven).
- 2026-11-23 Launched QWPT7b.
- 2026-11-23 Merged #634 W-POOL3 0542efc.
- 2026-11-23 #635 QWPT7b READY (App pointer/click/dblclick/auxclick/contextmenu/keypress; no native context menu), merging. Launched QGEOM2.
- 2026-11-23 Merged #635 QWPT7b 52a3995. Reinstalling.
- 2026-11-23 Reinstalled ~/.cargo/bin/scratchy 02:44 from main 5b9cfd9 (App pointer events, damage repaint everywhere, canvas line styles, DOMMatrix).
- 2026-11-23 #636 QCNV9 READY (Path2D; element score 48.9 -> 51.5, offscreen 54.9 -> 58.0, TOTAL 47.9; report quoted pass% not score), merging. QCNV3b += missing args. Launched QCNV10.
- 2026-11-23 Merged #636 QCNV9 981b8bf.
- 2026-11-23 #637 QINC2 READY (paragraph-id numbering across shaping ticks: NAT cache hits now consume ids; check mode 10/10 clean), merging. AGENT_RULES: never /home/loupax/src/wpt. Launched QDOM5e.
- 2026-11-23 Merged #637 QINC2 442162f.
- 2026-11-23 #638 QGEOM2 READY (DOMRectList, structured clone of geometry), merging. Filed + launched QWPT-IDL.
- 2026-11-23 Merged #638 QGEOM2 253a2f2.
- 2026-11-23 #640 QWPT-IDL READY (+3132 idlharness subtests, +2096 passing), merging. Launched QEDIT-B.
- 2026-11-23 Merged #640 QWPT-IDL 49c8316.
- 2026-11-23 #639 QCNV10 READY (gradients; element 55.8, offscreen 62.9, TOTAL 49.7), merging. Filed QCNV10b. AGENT_RULES trailer = own model. Launched QCNV11 (Opus).
- 2026-11-23 Merged #639 QCNV10 423be6a.
- 2026-11-23 #641 QEDIT-B READY (draggable + DataTransfer; 8/9 dnd files), merging. Filed QINC2b (qinc2 test is wall-clock sensitive; launched), QWPTFLK6, QEDIT-B2.
- 2026-11-23 Merged #641 QEDIT-B 3ee5c85. QDOM5e agent stopped without PR/report: resumed to finish. Launched QINC2b.
- 2026-11-23 #642 QINC2b READY (deterministic tick test; fails on the old bug; 52/52 under load), merging. Launched QWPTFLK6.
- 2026-11-23 #643 QDOM5e found as PR (agent report lost): new Document for initial-about:blank navigation, unload on removal; promise-job-incumbent not done. Queued after #642. Launched QWPTFLK6.
- 2026-11-23 QDOM5e report arrived (matches PR). Filed QINCUMB1. Note for QWPTFLK6: Range-insertNode/surroundContents time out under 4-way load even with budgets (also on main). Launched QEDIT-C.
- 2026-11-23 Merged #642 QINC2b 34fcc48.
- 2026-11-23 Merged #643 QDOM5e 54de567. Reinstalling.
- 2026-11-23 Reinstalled ~/.cargo/bin/scratchy 04:58 from main 28257b1.
- 2026-11-23 #644 QCNV11 DRAFT (compositing; element 58.2, offscreen 65.6) with 4 shadow subtests pass->fail (passed only because xor was ignored). Not asking the user: hold #644, QCNV12 stacked on it, merge together. Launched QCNV12 (Opus).
- 2026-11-23 #646 QWPTFLK6 READY (timeOrigin anchored on first read; 20/20 under load), merging. AGENT_RULES: cap load generators; prove tests fail on old code. Launched QEDIT-D.
- 2026-11-23 Merged #646 QWPTFLK6 9f404ad.
- 2026-11-23 #645 QEDIT-C READY (until-found, beforematch, details reveal), merging. Filed + launched QDET1 (closed <details> renders its body today).
- 2026-11-23 #645 merge FAILED (GitHub rejected the rebased push, EXIT=3); my cleanup then deleted the branch, closing the PR. Restored the branch at cfdb665, reopened #645, retrying. Cleanup now runs only after a MERGED line.
- 2026-11-23 Merged #645 QEDIT-C 3055227 (retry succeeded).
- 2026-11-23 #647 QCNV11+QCNV12 READY (compositing + shadows; total 49.8 -> 52.0), merging; #644 closed after. Launched QCNV13 (waits for #647 on main).
- 2026-11-23 Merged #647 QCNV11+QCNV12 fee8f36 (TOTAL 52.0); #644 closed as superseded.
- 2026-11-23 #649 QDET1 READY (details/summary), merging. Filed QWPT-INT (needs user approval: new WPT area) and QDET2. Launched QWPT7e.
- 2026-11-23 Merged #649 QDET1 5cd4f11. QWPT-INT added to Waiting on you.
- 2026-11-23 #648 QEDIT-D READY (tabIndex + focus fixup, 7 files), merging. Filed QTYPE1 (P1: typing into 2nd field drops chars, on main too; launched) and QEDIT-F.
- 2026-11-23 Merged #648 QEDIT-D 6f86b74. Reinstalling.
- 2026-11-23 Reinstalled ~/.cargo/bin/scratchy 07:03 from main 6e19ab9.
- 2026-11-23 USER: scratchy PAUSED (save tokens). No triage, launches or merges. Development moves to web/ (CEF + wire, docs only so far). Open PRs/agents left as they are.
- 2026-11-23 Merged #650 a1ba206 (web/ started, scratchy PAUSED). Draining QCNV13, QWPT7e, QTYPE1; their PRs get merged; no new launches.
- 2026-11-23 Merged #651 QTYPE1 a353c05 (drain). Filed QAUTOFOC1 (not scheduled: paused). Reinstalling.
- 2026-11-23 Reinstalled scratchy 07:45 from 1d5a317 (has #651). Merged #652 QCNV13 b608fa4 (drain). Filed QIMGERR1 (not scheduled). Coordinator prompt/rules/scripts now cover web/ and wire/ (d1409917). Draining: QWPT7e only.
- 2026-11-23 Merged #653 QWPT7e 4432661. DRAIN COMPLETE: no scratchy agents running, nothing scheduled. Filed QWPT7f (not scheduled). Final reinstall.
- 2026-11-23 Final reinstall: ~/.cargo/bin/scratchy 08:00 from main 4432661 (all drain PRs). scratchy idle.
- 2026-11-23 USER: W-POOL4 keep it safe (no retry after clean close). Closed to wire/TODO-DONE.txt, recorded in scratchy/DECISIONS.md (Network). Nothing waiting on the user.
- 2026-11-23 Session start (user's machine). web spike started: launched W1 (Opus, bare CEF window). wire: launched W-MEDIA2 (Sonnet; truncated vs complete body, matters for the spike's downloads/video, W5).
- 2026-11-23 Named wire's duplicate-header gap W-HDR1 and launched it (Sonnet; Chromium as a client makes it reachable). 3/3 slots.
- 2026-11-23 #655 W-HDR1 READY (repeated request headers merged: Cookie '; ', others ', '; all Set-Cookie lines already reached the client). wptserve shard failed once (request-forbidden-headers Via subtest), passed on re-run: flake, not filed (scratchy paused). merge_pr.sh green() now judges only the latest run per check (close/reopen left CANCELLED runs). Merging.
- 2026-11-23 Merged #656 W1 4cb0793 (CEF 156 window via the `cef` crate; loads example.com + Wikipedia on nested Wayland and Xwayland; FLATPAK_SANDBOX_DIR stops Chromium moving itself out of our capped scope). W1 marked done in web/TODO.txt (done.py does not know web's format). Launched W2 (Sonnet).
- 2026-11-23 #655 W-HDR1: wptserve shard failed again after rebase (request-forbidden-headers, Sec-Test subtest, "Failed to fetch"); agent could not reproduce on fetch/api x4 (full-shard repro was denied by the permission check; not run for it). Re-running the shard in CI on #655 to decide flake vs regression. #655 held.
- 2026-11-23 #654 W-MEDIA2 READY (X-Wire-Upstream-Length; 502 truncated on the buffered path when certain; streamed path cannot signal a cut with today's framing), merging.
- 2026-11-23 Merged #654 W-MEDIA2 c5b85e6; ~/.cargo/bin/wire reinstalled 08:57 (user's running wire needs a restart to get it). Archived W-MEDIA2 by hand (done.py has no wire/web format). Progress page: web section added, republished.
- 2026-11-23 Merged #657 W2 5ea040c (empty netns via unshare -n --map-user; sandbox on). Filed W-FS1 (optional bwrap filesystem hiding, under After a go). Launched W3 (Opus). #655: wptserve 2/5 failed on the PR, file never flaked on main; agent doing a capped A/B (branch wire vs main wire).
- 2026-11-23 #655 W-HDR1: capped A/B showed main's wire fails the same subtest (1/5) -> pre-existing flake. Rebased by hand (wire/PROMPTS.md keep-both), merged 0104f79. Main CI failed once (wptserve: xhr/responseText-status OK->CRASH), passed on re-run. wire reinstalled 09:33. Filed + launched W-FLK1 (Opus): the wptserve transport flakes.
- 2026-11-23 Merged #658 W3 a35f6ad (CEF requests answered from wire; wire's no-follow requests now pooled). wire reinstalled 09:46. Filed W-UPLOAD1 (web, decide before W7) and W-ENC1 (wire, br/zstd). Chromium's HTTP cache stores nothing handler-served (open question closed). Launched W4 prep (Sonnet; ends as a needs-input DRAFT for the user's login).
- 2026-11-23 #659 W4 DRAFT needs-input: user's real-site login checklist (run.sh --persistent; proven on fixtures + httpbin: persistence, session drop, HttpOnly/Secure/SameSite, wire jar empty). On the progress page under Waiting on you. Launched W5 (Sonnet) stacked on web/w4-cookies; merges after #659.
- 2026-11-23 Merged #660 W-FLK1 9b103f6 (cause: ureq 2 pools a connection holding a body sent after 204/304/HEAD; idempotent requests resent once). wire reinstalled 10:11. Filed W-POOL5 (needs user: patch ureq / ureq 3 / keep workaround), on Waiting on you. Launched W-ENC1 (Sonnet, brotli).
- 2026-11-23 Merged #661 W-ENC1 0b4f6e5 (brotli; client Accept-Encoding dropped; unknown encodings passed through). wire reinstalled 10:42. #662 W5 READY (CEF skip() range bug fixed; 1 GiB download; long-poll ok; no body-cap switch needed), stacked on #659: merges after the user's W4 check. Filed W-IDLE1 (optional). Launched W6 (Opus) stacked on #662.
- 2026-11-23 USER: spike is a success, stop testing. Stopped W6 (WIP pushed to web/w6-bypass), killed its recorded scopes/PIDs, removed its worktree. Recorded in web/DECISIONS.md. Asked whether this is the go; #659/#662 still open.
- 2026-11-23 USER: merge_pr.sh docs-only fast path (only *.md / TODO*.txt changed -> no CI wait; merge retried while GitHub computes mergeability). Recorded in scratchy/DECISIONS.md process section.
- 2026-11-23 Merged #663 wire2 plan 5738664 (user's PR, docs only: wire2/ README, TODO, PROMPTS). wire/ unchanged. wire-v1 branch left (PR says deletable; not deleted without the user's word).
- 2026-11-23 Merged #664 (user's PR, docs only, first docs-only fast-path merge: seconds) 0fb2e2e: web Architecture C plan (compositor shell + engine hosts + optional --proxy; Blink first; C1..C8, E1, P-FORGET, P-COOKIES; W-UPLOAD1 moot). Open for the user: merge #659/#662 as spike record or close them.
- 2026-11-23 USER closes #659 (W4) and #662 (W5) (moot under Architecture C). Branches kept; C1 notes to take the DownloadHandler from web/w5-streaming.
- 2026-11-23 USER: CI path filter. Merged #665 27d6e45: new `changes` job; scratchy/ -> scratchy+WPT, wire/ -> wire+scratchy+WPT, snag/ -> snag, .github/ -> all, others -> nothing. merge_pr.sh accepts SKIPPED, requires `changes` green. Probe #666 (web-only): 11 s run, closed. Recorded in scratchy/DECISIONS.md.
- 2026-11-23 USER: COORDINATOR_PROMPT.md updated: web = Architecture C (C1..C3 chain, C4/C7 wait for the user, no enforcement in web), wire2 planned/blocked on W2-SPEC, drain exception dropped, network rule narrowed to scratchy.
- 2026-11-23 Session start (user's machine). Launched C1 (Opus, web-host-cef + control protocol v0), W-MEDIA1 (Sonnet, file: Range + media types), W-POOL2 (Opus, wire follows redirects itself on every path). AGENT_RULES web/ section: spike netns/wire rule replaced by Architecture C (no enforcement, own --proxy).
- 2026-11-23 USER: wire PARKED ("we only work on wire2 now"). Parked W-MEDIA1 (draft #667, code+tests done, CI unfinished) and W-POOL2 (branch wire/w-pool2-redirects 697fef4d, WIP, no PR). W-MEDIA1 left /tmp/wire-w-media1-3588680 and -3591557 (40K each; its rm was denied, left for the user).
- 2026-11-23 USER: no tabs for now (C4 deferred; maybe buffers later). C1 told: one page, no new-tab/close-tab, popups load in the same page (known loss: window.opener popups). Recorded in web/DECISIONS.md "No tabs yet".
- 2026-11-23 USER: wire2 ACTIVE. Copied the spec doc into wire2/SPEC.md (diagram as ASCII); W2-SPEC done; wire2/DECISIONS.md started. Launched W2-1 (Opus). C7 hint design: explained to the user, awaiting their answer.
- 2026-11-23 USER: keep C7 (link hints), not dropped; user is thinking it over. Noted in web/TODO.txt with the open design questions.
- 2026-11-23 Status bar/navigation check vs KEYS.md: C1 protocol lacked `reload hard` (:refresh hard) and a network load-error event. Sent both to C1; noted in web/TODO.txt.
- 2026-11-23 Merged #668 C1 9b16bbe (web/host-cef, protocol v0 in web/PROTOCOL.md, one page, reload hard, load-error; drive.py passes with/without --proxy). Install: none. Filed C-BGNET, C-DL, C1-INPUT, C-PENDING. Parked wire worktrees removed (branches kept). Launched C2 (Opus) and P-FORGET (Opus; host side only, `web --forget-site` wiring waits for the shell).
- 2026-11-23 Progress page republished after #668 (web section is now Architecture C C1..C8 + P items; wire2 section added; waiting-on-you refreshed).
- 2026-11-23 Merged #669 W2-1 200820f (wire2 plain forwarding proxy). ~/.cargo/bin/wire2 installed 13:11. Filed W2-1a/b/c. Manual Chromium/Firefox browsing check -> Waiting on you.
- 2026-11-23 #670 P-FORGET READY but HELD: asks the user whether the hidden-page storage clearing counts as "not an automation protocol" (alternative: DevTools Storage.clearDataForOrigin) and notes every forget clears the whole HTTP cache. Filed P-FORGET-SHELL, P-FORGET-CACHE.
- 2026-11-23 USER: remove tab opening in general (context menus etc.). Filed C-NOTAB and launched it (Sonnet). Launched W2-2 (Opus).
- 2026-11-23 Progress page republished after #669 (+ #670 held, C-NOTAB, W2-2).
- 2026-11-23 USER on #670: use Storage.clearDataForOrigin (in-process DevTools call allowed for forget-site), engine-neutral interface; asked why the whole HTTP cache is cleared. Recorded in web/DECISIONS.md; P-FORGET agent resumed to rework #670 and verify per-origin HTTP cache clearing.
- 2026-11-23 USER: monitor moves to the work laptop. Filed NEST-HEADLESS (next free slot). Told C2, C-NOTAB, P-FORGET: live runs may stall without a display; mark them owed, don't debug. Note: 4 agents running (P-FORGET resumed for rework on top of 3); no launch until two finish.
- 2026-11-23 Merged #672 C-NOTAB 866ea8b (deny-list context menu, background/new-window dispositions dropped, Ctrl+Shift via injected capture listener). Live check owed (no display): re-run test/notab.py after NEST-HEADLESS. Filed C-NOTAB-ALLOW, C-KEYH. 3 agents (C2, W2-2, P-FORGET); NEST-HEADLESS next.
- 2026-11-23 Merged #670 P-FORGET fcc0eb5 (reworked per user: clearDataForOrigin in process, SiteForgetter trait; HTTP cache only clearable globally, verified). P-FORGET-CACHE moot. notab.py: injected Ctrl never arrives even in a working nested dwl (same on main) -> likely inject modifier bug, given to NEST-HEADLESS. Launched NEST-HEADLESS (Opus).
- 2026-11-23 #671 C2: wptserve shard failed after rebase (xhr/status-basic.htm subtest 9, "Failed to load", CHICKEN 402); C2 touches no scratchy/wire; re-run passed -> flake (likely the W-POOL5 class). Merged #671 C2 4e20d8b. Install none (web needs CEF runtime beside the binary: filed C-INSTALL). Filed C-KEYMAP, C-FOCUSKEYS, C-SOCKDIR.
- 2026-11-23 #673 NEST-HEADLESS READY (headless dwl via a small patch built from ~/src/dotfiles/dwl, inject motion time 0 fix; drive.py + notab.py PASS headless = C-NOTAB's owed live check). Merge CONFLICT nest.sh/PROMPTS.md with C2 -> sent back to the agent.
- 2026-11-23 Merged #674 W2-2 c8a5f49 (TLS interception machinery, lazy CA, W2-1a included). wire2 reinstalled 14:02. Filed W2-2a/b/c, W2-2d (user: CA dir name). Launched C3 (Opus) and W2-3 (Sonnet).
- 2026-11-23 Merged #673 NEST-HEADLESS 5864476 (rebased with C2's --x11-display). C-NOTAB live check done (notab.py PASS headless). Filed NEST-WINKEYMAP; scratchy/tools/inject has the same motion-time bug (fix when scratchy resumes). Running: C3, W2-3.
- 2026-11-23 Launched P-COOKIES (Sonnet). Progress page republished after #671/#673/#674.
- 2026-11-23 Merged #675 P-COOKIES 127f926. Filed P-COOKIES-2 (needs user). Launched C-BGNET (Sonnet; host-cef, C3 is in web/shell).
- 2026-11-23 USER: bump C-INSTALL -> next free slot (3/3 busy: C3, W2-3, C-BGNET).
- 2026-11-23 Merged #676 W2-3 b888097 (wss in decrypted tunnels; flush bug fixed). wire2 reinstalled. Launched C-INSTALL (Sonnet, bumped by user). wire2 next: W2-F (filters) after C-INSTALL/C-BGNET free a slot.
- 2026-11-23 Merged #678 C-INSTALL 3bce370. Installed web for the user: ~/.local/bin/web -> lib/web/20261123-143308-3bce370f (14:33). Launched W2-F (Opus).
- 2026-11-23 Merged #677 C3 f5e329c (PROMPTS.md-only conflict resolved by hand in .coordinator/tmp, keep-both). web reinstalled 14:39 (20261123-143909-f5e329ce). Filed C3-GOHTTP, C3-SHIFTSYNC, C3-RCLICK (user). Launched C5 (Sonnet).
- 2026-11-23 USER: popups (context menu, select) return to the previous mode when closed, unless a field got focus. Recorded in web/DECISIONS.md; C3-RCLICK ready (after C5: same area, web/shell keys).
- 2026-11-23 Merged #679 C5 f7a5cba; web reinstalled. Filed C5b, C5-FRAG, C5-STALE. Launched C3-RCLICK (Sonnet).
- 2026-11-23 USER: editable focus always switches to INPUT (script focus too; reverses T32), no return on blur. Recorded in web/DECISIONS.md; added to the running C3-RCLICK agent (same code).
- 2026-11-23 Merged #680 C-BGNET eb6b62e; web reinstalled. Filed C-DRM, C-H264 (user). C-BGNET agent briefly ran a check in another agent's nest (wayland-1): noted. Launched C-NOTAB-ALLOW (Sonnet; host-cef context menu, + cargo fmt of newtab.rs).
- 2026-11-23 USER: C-DRM and C-H264 parked. Asked about dropping the context menu for command-mode actions (discussing).
- 2026-11-23 USER: no context menu; actions as : commands. C-NOTAB-ALLOW agent repurposed to C-NOMENU; filed C-CMDS (after C-NOMENU; :yank after C6). C3-RCLICK told to test with selects. Recorded in web/DECISIONS.md.
- 2026-11-23 USER: Enter on a search match does nothing. Never filed after C5 (only mentioned). Filed C5-ENTER (S7, T18-T20), next web/shell slot after C3-RCLICK.
- 2026-11-23 USER: search moves the pointer to the match start; Enter clicks at the pointer (always, my reading, told the user). C5-ENTER rewritten; web/DECISIONS.md.
- 2026-11-23 Merged #681 C3-RCLICK 1068028; web reinstalled 15:15. Filed C3-LIVE-REFRESH. USER: unsure about moving the real pointer, prefers caret browsing -> C5-ENTER WAITING FOR USER (design with QCURSOR-D). Launched C6 (Opus).
- 2026-11-23 Merged #683 C-NOMENU 2d4da10 (no CI job for host-cef-only; local live check PASS); web reinstalled 15:18. C-CMDS held until the caret question is answered. Launched bundle C3-LIVE-REFRESH + C5-FRAG + C5-STALE (Sonnet).
- 2026-11-23 Merged #682 W2-F fe69303 (wire2's planned chain complete). wire2 + wire2-pass installed 15:21. Filed W2-F1, W2-F2 (user manual), W2-F3 folded into W2-1b. Launched W2-2a (Sonnet). USER: not Chromium's caret browsing (tried it in Chrome, bad) -> own caret, design pending (C5-ENTER note).
- 2026-11-23 USER: park everything caret-related (C5-ENTER, C-CMDS, CARET-1 idea). Discussed: Chromium caret drawn by the shell as a spike.
- 2026-11-23 Merged #684 C5-FIXES 95fc985; web reinstalled 15:33. Launched bundle C3-GOHTTP + C-SOCKDIR + cargo fmt drift (Sonnet). Waiting on the user re: Enter in NORMAL / caret always on (discussion, no changes).
- 2026-11-23 USER: Enter in NORMAL forwards to the page; :set caret/nocaret toggles Chromium caret browsing (default off). Filed C-ENTER + C-CARETCMD, launched (Opus: CEF caret pref unknown). Bundle C3-GOHTTP/C-SOCKDIR back to queue.
- 2026-11-23 Merged #685 W2-2a ae12729; wire2 reinstalled 15:44. Filed W2-2e, W2-HUBCLOSE, W2-FLK1. Launched W2-F1 + W2-FLK1 (Sonnet).
- 2026-11-23 #687 W2-F1+W2-FLK1 READY but edited wire2/SPEC.md: sent back to move the additions to NOTES/README. AGENT_RULES: user specs are read-only.
- 2026-11-23 Merged #686 C6 part 1 60d5aa0 (web reinstalled 15:55) and #687 W2-F1+W2-FLK1 390a956 (wire2 reinstalled 15:57). Filed C6b-C6f, C2-LIVE-REFRESH. Launched C6f+C2-LIVE-REFRESH (Sonnet) and W2-HUBCLOSE+W2-2b (Sonnet).
- 2026-11-23 Merged #688 W2-HUBCLOSE+W2-2b; wire2 reinstalled. Launched W2-2e (Sonnet).
- 2026-11-23 #689 C6f+C2-LIVE-REFRESH DRAFT: C6f fix (--ui-toolkit=fallback) drops GTK theming (dark mode to pages) -> asked user (a/b/c, recommend b). Shell popup-constraint bug found; agent's src edit was refused by the permission layer (out of the tests-only scope I gave), not worked around. Filed + launched C2-POPUP (Sonnet) with live.py refresh.
- 2026-11-23 USER: drop middle-click paste in the page, keep GTK. #689 closed (branch kept for C2-POPUP's cherry-pick). C6f dropped; web/DECISIONS.md.
- 2026-11-23 Merged #690 C-ENTER+C-CARETCMD; web reinstalled. Filed + launched C6-LIVE (Opus: c6_live 4/8 on main).
- 2026-11-23 #691 W2-2e: first merge FAILED (EXIT=3, GitHub rejected the rebased push), and my chained command ran cleanup anyway: deleted the branch (PR closed) and worktree. Restored the branch at a035445b from the local ref, reopened #691, retried: Merged 995d0c0. wire2 reinstalled. Lesson (again, cf. #645): merge, check EXIT and the MERGED line, THEN clean up, in separate commands. Filed W2-2f.
- 2026-11-23 Merged #692 C2-POPUP 9a37017; web reinstalled 16:30. Closed #689's worktree/branch. Launched C6b HiDPI (Opus) and W2-2f (Sonnet).
- 2026-11-23 Merged #693 W2-2f 1330ff6; wire2 reinstalled 16:39. W2-2c marked done (#682). wire2 agent work left: W2-1b (needs a quiet machine). Launched P-FORGET-SHELL + C-SOCKDIR (Sonnet).
- 2026-11-23 Merged #694 P-FORGET-SHELL+C-SOCKDIR 424db43 (revert-to-fail check skipped by the agent; live check covers it); web reinstalled. Launched C-KEYMAP (Sonnet).
- 2026-11-23 USER hit: second web with the same profile -> Chromium singleton lock -> host panic (their first web, pid 4129359, still running since 16:44). Filed C-SINGLE (next web slot).
- 2026-11-23 Merged #695 C6-LIVE 006e846 (test bug); web reinstalled. Corrected my web/DECISIONS.md error (middle-click paste in : and / lines was never built). Filed LIVE-GEOM, C6g (user's call). Launched C-SINGLE (Sonnet).
- 2026-11-23 Merged #696 C6b 79e8a9d; web reinstalled. Filed C6b-X11, C6b-CURSOR. Launched C6c frame pacing (Sonnet).
- 2026-11-23 Merged #697 C-SINGLE d83f8fe; web reinstalled. Launched LIVE-GEOM + host-cef fmt (Sonnet).
- 2026-11-23 Merged #698 C-KEYMAP 79478fd; web reinstalled. Found: the old nest dwl patch wrote the user's /tmp/dwl-keyboard-layout and signalled their someblocks on every agent nest run (fixed in #698). Filed C-KEYMAP-NAMES (optional). Launched C-FOCUSKEYS + C3-SHIFTSYNC (Sonnet).
- 2026-11-23 Merged #699 C6c 4767dbe; web reinstalled. Filed C6h (120 Hz rAF; renamed from the agent's C6g, taken). Launched C6d dmabuf (Opus).
- 2026-11-23 Merged #700 C-FOCUSKEYS+C3-SHIFTSYNC ec4c966 (PROMPTS-only conflict resolved by hand); web reinstalled. Launched C3-GOHTTP (Sonnet).
- 2026-11-23 Merged #701 LIVE-GEOM e698ca6 and #702 C3-GOHTTP bdb9efd (PROMPTS-only conflict by hand); web reinstalled. Filed LIVE-GEOM2. Launched C5b (Sonnet) and LIVE-GEOM2+NEST-WINKEYMAP (Sonnet).
- 2026-11-23 Merged #703 LIVE-GEOM2+NEST-WINKEYMAP 4479f3a (tools/tests; install none). Launched C1-INPUT (Sonnet).
- 2026-11-23 USER: merge #704 (their plan: picker, bookmarks, buffers as tabs, popups like Chrome). PROMPTS-only conflict resolved by hand; merged dd9fbb1 (docs-only fast path). C1-INPUT stopped (moot), WIP on web/c1-input, findings attached to C-POPUPS. COORDINATOR_PROMPT: plan order C-PICK -> C-BOOKMARKS -> C4 -> C-POPUPS, C4-SPIKE beside C-PICK. Launched C-PICK (Opus).
- 2026-11-23 Merged #706 C6d 985f223; web reinstalled. Filed C6d-DESKTOP (user), C3-LIVE-GOHTTP + C6b-LIVE-DPR (launched, Sonnet). Note: #702 left c3_live stale (2 steps) — test not updated in that PR.
- 2026-11-23 Merged #705 C5b 2243928; web reinstalled. Filed C5b-X (only if wanted). Launched C4-SPIKE (Opus) beside C-PICK per the #704 plan.
- 2026-11-23 Merged #707 C-PICK 60f7b98, BUT its branch was squashed onto an older main (soft reset), so the merge reverted #705 C5b, #706 C6d and coordinator TODO edits (dmabuf.rs, gpu-nest.sh, c5b/c6d tests, find_start...). Found from a TODO diff right after. Fixed with #708 (0ddeafa): tree = pre-merge main 22b45ea + exactly C-PICK's diff 5ac3434..60f7b98 (same 17-file set), PROMPTS keep-both; shell 232 tests + clippy clean; CI green. web reinstalled 18:24 with dmabuf + C5b + picker (verified in the binary). AGENT_RULES: squash only onto your own base; check three-dot diff. Launched C-BOOKMARKS (Sonnet).
- 2026-11-23 Merged #709 live fixes 6610f91 (tests only) and #710 C4-SPIKE 9229c23 (docs; PROMPTS conflict by hand). Spike findings attached to C4 and C-POPUPS. C4 waits for C-BOOKMARKS (plan order).
- 2026-11-23 Launched C6h + C6b-CURSOR (Sonnet). Third slot kept for C4 after C-BOOKMARKS.
- 2026-11-23 Merged #711 C-BOOKMARKS d9a3035 (PROMPTS conflict by hand); web reinstalled. Launched C4 Buffers (Opus) per plan.
- 2026-11-23 Merged #712 C6h+C6b-CURSOR f50e795; web reinstalled. Filed C6h-144 (optional), C-FMT (after C4). Holding the free slots: C4 touches most of web/shell (conflict risk); C-POPUPS waits for C4.
- 2026-11-23 Merged #713 C4 Buffers 335f5fe (31 web files, 2-dot = 3-dot); web reinstalled. Launched C-POPUPS (Opus) and C-FMT + test_notab fix (Sonnet).
- 2026-11-23 USER: all pending default choices kept (picker, bookmarks, wire2 W2-1c/W2-2d/W2-F calls). Recorded in web/ and wire2/ DECISIONS.md; C-FMT removes the (proposed) marks in KEYS.md.
- 2026-11-23 USER bug: flicker of fixed/absolute elements while scrolling (YouTube). Filed + launched C-FLICKER (Opus). Asked the user for A/B: WEB_NO_DMABUF=1 and Chromium's overlay delegation off.
- 2026-11-23 USER A/B: flicker gone with WEB_NO_DMABUF=1 (dmabuf path, NVIDIA). Sent to C-FLICKER (explicit sync / release timing).
- 2026-11-23 USER: Webex says shared content/video unsupported and needs OpenH264. Filed C-WEBEX (investigation).
- 2026-11-23 Merged #714 C-FMT 634925a and #715 C-POPUPS 2d48e31 (PROMPTS conflict by hand); web reinstalled 19:56. The #704 plan is complete. Filed C-NORMALMODS (user). Launched C-WEBEX (Opus).
- 2026-11-23 USER: pass modifiers to the page on clicks in NORMAL unless it breaks things. web/DECISIONS.md; launched C-NORMALMODS (Sonnet).
- 2026-11-23 Merged #717 C-NORMALMODS 9503f57; web reinstalled.
- 2026-11-23 Merged #716 C-FLICKER ac82479 (explicit/implicit sync for dmabufs); web reinstalled. Filed C-VIDEOPACE (only if needed). User to check YouTube on the real desktop.
- 2026-11-23 Merged #718 C-WEBEX 71ef452 (report). Filed C-H264-BUILD, C-EXTERNAL (user), C-SHARE-CHECK (user), C-SHARE-TAB (optional).
- 2026-11-23 USER: do both: :openExternal (C-EXTERNAL) and build CEF with proprietary codecs (C-H264-BUILD; 20G cap for that build only, outside the repo, never published). web/DECISIONS.md. Launched both.
- 2026-11-23 Merged #719 C-EXTERNAL 5e6c5f4; web reinstalled. C-H264-BUILD running (hours).
