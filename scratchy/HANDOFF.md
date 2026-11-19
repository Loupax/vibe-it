# Coordinator hand-off (2026-11-19, workload moving to another machine)

Read scratchy/AGENTS.md, scratchy/TODO.txt (QUEUE), REGRESSIONS.md R42/R43, QUESTIONS.md, and the memory
index first. Newer than NEXT_PROMPT.md (which is stale: it says one agent, baseline 78, QuickJS).

## State
- main at 167c9c0 or later. V8 + media are the default build (`cargo install --path . --force`, add
  `RUSTY_V8_ARCHIVE=<librusty_v8.a>` to skip the 15-min V8 build; libmpv is dlopen'd at run time).
- Merged 2026-11-19: #306-#321 (QV8-4a/4b, MSE stack QM1k1b-e, k, l, i, g, n4, controls bar, QM1k1n1 keep
  decoding off-screen video, QY53a-d forced-layout cuts). Nothing in flight at hand-off.
- YouTube: first minute plays; page paints ~28 s (was 45-50), click->playing ~7 s. Remaining blockers in order:
  QY53h -> QY53e1 -> QY53g -> QY53e3 (Opus, live-window measure) -> QY53e2 -> QY53f; QM1k1n5 (SABR results reach
  JS 1-19 s late, measure first), QM1k1n6 (redraw starved; Opus design), QM1k1n7 (411). PoToken stop (QM1k2) is
  parked and ends every run.
- QM1k-mouse: user saw the video freeze while the mouse moves over the page. DEFERRED by the user: re-check after the
  QY53 items; only if it still happens, Opus investigates (likely QM1k1n6).

## Process (unchanged, user rules)
- Max TWO agents at a time. Keep assigning without asking until the user says pause or only user-input items remain.
- Agents: worktree isolation, background, branch scratchy/<item>-<slug>, PR, CI green on ALL four jobs
  (scratchy, wire, snag, scratchy-quickjs; "no checks" is not green), merge themselves with
  `gh pr merge <n> --merge`. On a classifier denial: leave the PR open and surface it to the user (do not merge for
  the agent). Opus investigates/measures only (docs commits to main); Sonnet implements.
- Per-item checklist: AGENTS.md (clippy <= tools/clippy-baseline.txt = 74, full cargo test on BOTH configs, WPT no
  pass->fail, PROMPTS.md entry, TODO tick + separate fill-in-hash commit, js-report 0 errors on 3 pages, viewed
  nested-dwl screenshot when style/paint changes).
- Agents delete their own build dirs after their PR merges (AGENTS.md "Clean up after your PR merges"). The disk
  filled up once (186 worktrees x 3-6 GB); check `df -h /` and prune merged worktrees if it creeps up.
- After merges the coordinator reinstalls (`cargo install --path . --force -j2`, background) and asks the user to
  restart scratchy. The user restarts wire (`X-Wire-Version: 3`, `X-Wire-Features: ua,ws`).

## Waiting on the user
Q1 CPU budget (recommend c), Q3 DuckDuckGo, Q5 WPT in CI, Q6 --profile, QM1g2e fullscreen, QV8-6 (remove
QuickJS), QM1k-r Reddit bot challenge (never solve/evade), QM1k-AB real-audio drift test, QM1k-mouse re-check.
