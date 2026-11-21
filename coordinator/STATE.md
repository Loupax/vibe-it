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
