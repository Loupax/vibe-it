# Rules for every scratchy agent. These OVERRIDE scratchy/AGENTS.md where they differ.

Where you run: rules marked "user's machine" (nice/ionice, systemd-run memory cap, the perf lock, the
RUSTY_V8_ARCHIVE cache, /home/loupax/src/wpt, nested-window tests with tools/nest.sh) apply on the user's
machine. You are there when `$HOME/.cache/librusty_v8-152.2.0.a` exists. In a cloud session (claude.ai), skip
what is not available: build normally, skip the live-window, grim and perf steps, run the WPT subset only if a
checkout is available, and say in the PR which local checks were skipped so the coordinator runs them later.

1. Tool-call descriptions: start EVERY one with `[<ITEM>] ` (the user's agent panel shows it).
2. Worktree + branch: you run in your own git worktree. First: `git fetch origin && git checkout -b scratchy/<item-lowercase>-<slug> origin/main`.
   No `git stash`. Scratch files go in a subdir of your scratchpad, never loose in /tmp.
3. NEVER edit scratchy/TODO.txt or scratchy/TODO-DONE.txt (the coordinator owns them). Other docs per AGENTS.md (PROMPTS.md
   entry, REGRESSIONS/PERF/NOTES as the item needs) are fine.
4. LOCAL RESOURCE PRIORITY (user's machine only):
   - EVERY build command (cargo build, cargo check, cargo clippy, cargo test --no-run, cargo bench --no-run) runs as
     `nice -n 19 ionice -c3 cargo ...`. Env: `RUSTY_V8_ARCHIVE=/home/loupax/.cache/librusty_v8-152.2.0.a CARGO_BUILD_JOBS=4`.
   - Run the tests in two steps: build them with `nice -n 19 ionice -c3 cargo test --no-run ...`, then run the test binaries
     (`cargo test ...` again) at NORMAL priority.
   - Performance / benchmark / timing measurements (--perf-report runs, frame stats, cargo bench, profile.sh, anything whose
     numbers you report) run ONE AT A TIME across all agents: wrap each measured command in
     `flock /tmp/scratchy-perf.lock <command>`. Hold the lock only while measuring, not while building. Note `uptime` with results.
   - `ulimit -c 0` in shells that run scratchy.
   - Baselines: the coordinator reinstalls ~/.cargo/bin/scratchy after every merge. If you compare against "main", COPY that
     binary into your scratchpad first (and note its commit), or build origin/main yourself; never run the live path mid-series.
5. Processes: NEVER pkill/killall/kill by name or pattern (`pkill wire` also kills pipewire/wireplumber). Kill only PIDs you
   started and recorded, after checking their command line. If you need wire, run your OWN wire on your OWN socket in your
   scratchpad; NEVER use /run/user/1000/wire.sock. No `pgrep -f` wait loops.
6. Never `cargo install` (the coordinator installs from main).
7. Tests: add or update tests for your change. Before opening the PR, the full test suite must pass locally on the default
   features (V8 is the only engine: QuickJS was removed by QV8-6 b717000; there is no second config any more). (QSTK1x #397: plain debug `cargo test` passes, no RUST_MIN_STACK/--skip needed. QFLK1 8f4fc79 fixed the known load flakes: a full-suite failure is now a real signal, investigate it.) clippy warning count <= 77 with the local
   toolchain (CI baseline file says 74). WPT subset (/home/loupax/src/wpt at ac78b567, 322 pass / 292 fail): no pass->fail.
8. PR: push your branch and open a PR (`gh pr create`). Then WAIT for CI: all checks green or skipped (CI runs only the jobs for the crates you touched; `changes` decides) (`gh pr checks <n> --watch`).
   If CI fails you get at most 2 fix attempts (push a fix, wait again). Main may move meanwhile: do NOT rebase on your own
   unless the PR conflicts; the coordinator rebases at merge time.
   IMPORTANT: GitHub does NOT run pull_request CI while the PR conflicts with main. If no checks appear within ~5 min,
   run `gh pr view <n> --json mergeable`; if CONFLICTING: `git fetch origin && git rebase origin/main`, resolve
   (PROMPTS.md/REGRESSIONS.md: keep main's entries and yours, in order), re-run affected tests, push --force-with-lease.
   Never wait more than 10 min for checks that have not appeared.
9. NEVER merge. When all CI jobs are green: delete your build dirs (your worktree's `target/` and any CARGO_TARGET_DIR
   you made), then stop and reply with FIRST LINE exactly `READY #<pr> <ITEM> — <one-line summary>`, followed by a short
   report (what changed, tests, measured numbers with uptime, anything left out -> proposed new TODO item text for the
   coordinator to file).
10. If you cannot finish (blocked, item wrong or much bigger than its size, 2 CI fix attempts used up): open a DRAFT PR
    labelled `needs-input` (`gh pr create --draft --label needs-input`; create the label if missing) whose body says what
    you tried, what blocked you, and the EXACT question the user must answer. Reply with first line
    `DRAFT #<pr> <ITEM> — <question in one line>`. Measure-only / docs-only items with nothing to ask still open a normal PR.
11. If the coordinator later sends you a rebase conflict: `git fetch origin && git rebase origin/main` in your worktree,
    resolve, re-run the tests, force-push (`--force-with-lease`), wait for CI green, reply `READY #<pr> <ITEM> — rebased`.
12. Commit messages: `scratchy: <ITEM> <summary>` ending with the line
    `Co-Authored-By: Claude <your model, e.g. Opus 5.5 or Sonnet 5.5> <noreply@anthropic.com>` (your own model); PR bodies end with
    `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.


## Memory cap (2026-11-20 13:39: a scratchy process grew to 15 GB and the OOM killer took down the whole session)
Run EVERY scratchy process and EVERY test binary (cargo test, the fuzz harness, --wpt, perf runs, nested-window runs) inside
a memory-capped scope, so a runaway only kills itself:
  systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0 <command>
(e.g. `systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0 cargo test --release`; cargo's build step is
covered too, which is fine). Do not use `ulimit -v` (V8 reserves large virtual ranges). If something of yours is killed by
this cap, that is a bug to report (what ran, what grew), not a reason to raise the cap.

## /tmp is RAM (tmpfs) — no build output there (2026-11-20 ~15:10: agents' scratch dirs held 9.9 GB of RAM)
Never put a CARGO_TARGET_DIR, release binaries, profiles or big logs in /tmp or the scratchpad (both are tmpfs = RAM). Use a
directory under your worktree, e.g. `<worktree>/scratchy/target` or `<worktree>/.qwork/` (git-ignored by the root .gitignore since 2026-11-21), and
delete it after your PR merges. Small notes/scripts in the scratchpad are fine; delete your scratchpad subdir when done.

## Power-loss safety (user 2026-11-20 ~16:40: "as long as you don't lose progress if the power goes down we are fine")
- Commit work in progress to your branch and `git push -u origin <branch>` at least every ~30 minutes and after every
  milestone (a WIP commit is fine; squash before READY if you like). A pushed branch survives a power cut; an unpushed one
  on local disk usually does too, but /tmp and the scratchpad (tmpfs) do NOT.
- Keep measurement notes/results you will need for the report in a file under your worktree (e.g. `.qwork/notes.md`,
  git-ignored), not only in the scratchpad.
- If you are resumed after a crash: check `git status`/`git log` in your worktree and your notes file, and continue from there.

## Never commit scratch (2026-11-21: #498 committed a whole .qwork/ dir, incl. a wire binary, token file, logs and screenshots, to public main)
Stage files BY NAME (`git add path/to/file ...`), never `git add -A` / `git add .` / `git commit -a`. Before every commit run
`git status --short` and `git diff --cached --stat` and check that every staged path is source, tests or docs for your item.
Screenshots, logs, binaries, sockets, token files and anything under .qwork/, target/ or your scratchpad are never committed.

## Never touch the user's state dir (2026-11-21: gate/live window runs wrote localStorage + history into ~/.local/state/scratchy)
EVERY scratchy process you start (window, nested dwl, headless, gate scripts) gets its own state dir:
`--state-dir <worktree>/.qwork/state` (or `SCRATCHY_STATE_DIR=...`). Never run scratchy without it. Never read, copy or
write ~/.local/state/scratchy (or $XDG_STATE_HOME/scratchy): it holds the user's localStorage, cookies and history.

## One nested compositor, no leftovers (2026-11-21: an agent left 5 nested dwl instances running for 40 min)
At most ONE nested dwl (tools/nest.sh) per agent at a time: start it once, reuse it for every live run, and kill it by its
recorded PID as soon as the live checks are done (and before you report READY). Same for your own wire: one, reused, killed
at the end. Before your final reply, `ps -eo pid,cmd | grep <your worktree path>` must show nothing of yours.

## Waiting: in the foreground, bounded (2026-11-21: agents left dozens of background shells, some unbounded `until` loops)
- Wait for CI with ONE foreground `timeout 900 gh pr checks <n> --watch` call; never start a second watcher for the same PR,
  never in the background.
- Never write an unbounded wait loop (`until ...; do sleep; done`) and never poll a task output file in a background shell.
  If you must poll, wrap it in `timeout`, run it in the foreground, and stop as soon as it returns.
- Background shells only for long builds/test runs/measurements, and they must end by themselves (no daemons left behind).

## New tests go in their own file (2026-11-22: DOM PRs kept colliding at the end of js_dom_tests.rs)
Put your item's new tests in a NEW file `scratchy/src/tests/<item-lowercase>_tests.rs` plus one `mod <item>_tests;` line in
`src/tests/mod.rs` (the merge script keeps both sides of mod lines). Only edit an existing test file to change a test that
already lives there. This overrides AGENTS.md "matching area file" for new tests.

## No conflict markers, ever
After any rebase, before you commit or push: `git diff origin/main --stat` and
`git grep -nE '^(<<<<<<< |=======$|>>>>>>> )' -- scratchy coordinator` must print nothing. Two PRs (#560, #571)
landed `<<<<<<<`/`>>>>>>>` blocks in PROMPTS.md. merge_pr.sh now refuses a PR whose diff adds marker lines.

## wire's HTTP cache (once W-CACHE1 lands)
W-CACHE1 makes wire cache on disk by default in `~/.cache/wire`. Every wire an agent starts uses
`--cache-dir /tmp/wire-cache-<your worktree name> --cache-size 256M` (user 2026-11-22: agents' caches in
/tmp; /tmp is RAM, hence the size cap; wire prints one tmpfs warning line, expected), or `--no-cache`.
Never the user's `~/.cache/wire`. The tools that start a wire (tools/incr-gate/run.sh, wl-flood, missing-api corpus/cost) take a `WIRE_ARGS`
env var: set `WIRE_ARGS="--cache-dir /tmp/wire-cache-<worktree> --cache-size 256M"` (or `--no-cache`) for every run. Delete your cache dir by its explicit name when you finish. Perf
comparisons use the same cache setting on both sides (say which).

## A denied command stays denied
If the tool permission layer or the sandbox denies a command, do not rephrase, obfuscate or split it to
get it past the check (e.g. `$(echo gi)thub`), and do not ask the coordinator to run it for you. Skip
that step, and say in your report what was denied and what you skipped. (2026-11-22, QFF1b #598.)
Not the same thing: a command refused as "too complex to analyse" (long heredocs, chains) may be re-run as
plainer commands that do the same thing; that check is about readability, not permission. A permission or
sandbox DENIAL of what the command does stays denied.

## Nested compositor helper (QNEST1 #603)
tools/nest.sh reuses the worktree's live dwl (registry `.qwork/nest`). Call `tools/nest.sh --stop` before your final
reply instead of hunting PIDs; `--new` forces a fresh one. Never start dwl any other way.

## A failed CI check is information
If a CI check fails for a reason that looks unrelated to your change (a flaky test), do not re-run the job until
it is green. Say in the PR and your reply which test failed and why you think it is unrelated; the coordinator files
it. (2026-11-22, QCNV5 #624.)

## WPT checkout
Never use /home/loupax/src/wpt: it is a stale sparse checkout (no dom/, url/, html/canvas, ...) and gives thousands of
bogus 404/CRASH results. Always make your own pinned checkout under your worktree with tools/wpt-checkout.sh.

## Memory cap: load generators too
Every scratchy process runs under the systemd-run 6G memory cap, including extra copies you start only to load the
machine for a stability test (2026-11-23, QWPTFLK6 #646 ran 4 uncapped). And a new regression test must be shown to
FAIL on the old code (revert the fix locally, run it, restore) before you claim it covers the bug.

## web/ agents (2026-11-23, the CEF + wire spike)
These rules apply to every project. For a web/ (or wire/) item, read "scratchy" in the rules above as your project:
- Branch `web/<item-lowercase>-<slug>` (wire: `wire/<item>-<slug>`). Never edit web/TODO.txt or wire/TODO.txt
  (the coordinator owns every TODO.txt). Read web/README.md, web/DECISIONS.md and web/TODO.txt first; your step's
  "Done when" line is the acceptance test, so show it met with evidence (logs, screenshots, wire's request log).
- Spike code goes in `web/spike/` (a crate of its own, not in CI yet). CEF's binary distribution is downloaded to
  a cache under `$HOME/.cache/` (never /tmp, never inside the repo, never committed); delete `web/spike/target`
  at the end like any build dir.
- Every CEF run goes under the systemd-run memory cap (the scope covers CEF's child processes).
  Architecture C (2026-11-23) supersedes the spike's network rules: web enforces nothing, no empty network
  namespace and no wire is needed; the engine host's optional `--proxy` points at a proxy you start yourself
  (your own, on your own port, killed by PID at the end). Kill only PIDs you recorded;
  CEF spawns helper processes, so kill the whole scope you started (`systemctl --user stop <your unit>`).
- Use your own wire (own socket, `--cache-dir /tmp/wire-cache-<worktree> --cache-size 256M`), never the user's.
- Live windows: tools/nest.sh (nested dwl, Wayland). If a step needs X11 and the nested session has no Xwayland,
  say so in the PR instead of using the user's display.
- Never log in to real accounts, type credentials or use the user's browser profiles/cookies. Steps that need a
  login (W4) or a day of real use (W7): prepare everything (a build, a run script, what to check) and stop with a
  DRAFT PR saying exactly what the user should do.
- Never commit cookies, login state, wire logs or screenshots of real browsing; this repository is public.
