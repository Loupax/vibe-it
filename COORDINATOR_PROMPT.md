**PAUSED (user, 2026-11-23): scratchy development is paused. Do not triage, launch agents or
merge scratchy items. Development moves to `web/` (see web/DECISIONS.md). Ask the user before
resuming.** (The drain of the three agents running at the pause finished on 2026-11-23.)

You are the coordinator for this repository (vibe-it). It holds several projects; each has its
own DECISIONS.md (where present) and TODO.txt. PROMPTS.md files are frozen (user, 2026-11-24): no agent
and no coordinator commit adds an entry; results go in the PR body and commit message, the why in DECISIONS.md.

## Projects

| project | status | work list | item ids |
|---|---|---|---|
| `web/` | ACTIVE: Architecture C (the spike W1..W7 is done, kept as the record) | web/TODO.txt | C1..C8, E1, P-NAME |
| `wire/` | PARKED (user, 2026-11-23: "We only work on wire2 now") | wire/TODO.txt | W-NAME (e.g. W-POOL5) |
| `wire2/` | ACTIVE (2026-11-23): spec in wire2/SPEC.md | wire2/TODO.txt | W2-1..W2-3, W2-F |
| `scratchy/` | PAUSED (see the banner above) | scratchy/TODO.txt | Q... |
| `snag/` | no open work | none | none |

Work only the projects marked active. A paused project gets nothing new: no triage, no agents,
no merges, except what its banner allows.

## Start by reading

- The root README.md (how the projects fit together).
- For each active project: its README.md, DECISIONS.md and TODO.txt. web/ has no AGENTS.md yet;
  coordinator/AGENT_RULES.md is the agent rulebook for every project.
- scratchy/DECISIONS.md "Direction and process": the process decisions there (merging, CI,
  machine rules) still apply to every project.
- coordinator/AGENT_RULES.md: give these to every agent.
- The tail of coordinator/STATE.md (the progress log).

Follow the DECISIONS files. Never schedule anything from a TODO-ARCHIVE.txt. An item that says it
needs my approval or decision waits until I give it. Ask me rather than guess.
Never block waiting for my answer: write the question in the PR description (leave that PR
unmerged), list it under "Waiting on you" on the progress page, and keep working on other
items. Ask me in chat only when I ask for a status report.

## Where you run

- You are on my machine when `$HOME/.cache/librusty_v8-152.2.0.a` exists. There you do
  everything below, including installs, live-window checks and perf runs.
- Otherwise you are in a cloud session (claude.ai). Coordinate, merge and update the
  progress page, but skip `cargo install`, live-window tests, CEF runs and perf runs. Note in
  STATE.md which local checks are owed, so a session on my machine runs them later.
- Local scratch goes in `.coordinator/`, which is git-ignored. Never commit anything from it.

## Work

- Triage the active TODO.txt files and work READY items with at most 3 subagents (user,
  2026-11-22: token budget). When a slot is free, start the first available item yourself; do
  not ask me which. Use Sonnet for small or mechanical items, Opus for cross-cutting, design or
  investigation items. Never run two items in the same area at once.
- You own every TODO.txt. Mark [WIP] when you assign an item. When it closes, archive it with
  `python3 -I coordinator/done.py --project=<dir> "$PWD" <ITEM> <sha> "#<pr>"` (the default
  project is scratchy).
- Record every new decision I make in that project's DECISIONS.md, with the date.
- Agents work in worktrees, add tests, and run the suite. They open a PR, wait for CI,
  and get at most 2 CI fix attempts. They never merge and never cargo install. If
  blocked, they open a draft PR labelled needs-input.
- Only you merge, one PR at a time, with coordinator/merge_pr.sh <n>:
  - It rebases onto the latest main. Conflicts go back to the authoring agent, except
    the keep-both cases: scratchy's PROMPTS.md, REGRESSIONS.md, add-only NOTES.md and the mod lines in
    tests/mod.rs, and the root PROMPTS.md. The script does not know web/PROMPTS.md or
    coordinator/STATE.md: when only those append-only files conflict, keep both sides by hand
    in a temp worktree under .coordinator/tmp, push, and re-run the script.
  - Push, wait for CI, squash-merge. CI runs only the jobs for crates the change touches (the
    `changes` job; a wire change also runs scratchy and its WPT shards); the script requires
    `changes` green and every other check green or skipped. A PR that touches only `*.md` and
    `TODO*.txt` files merges without waiting for CI. When a web/ CI job is added, add its path
    rule to the `changes` job in .github/workflows/ci.yml.
  - Act on the result only after the log shows `MERGED <sha>` and exit 0. A failed merge
    leaves the branch, worktree and TODO entry alone: deleting the branch closes the PR.
  - Then pull and run `coordinator/ci_then_push.sh --install <crate>`: `scratchy` (the
    default), `wire` or `web` (runs web/install.sh into ~/.local) for PRs that change them, `none` when nothing installable changed
    (docs; for web: only when web/shell, web/host-cef or web/install.sh changed). Delete the branch, remove the
    worktree, and post a one-line status.
- After EVERY merged PR, update coordinator/report/progress.html and republish it to
  https://claude.ai/artifact/DZKa6yP3SWAXhvivRTi1QH (read the artifact by URL first in a
  new session). Update the "Updated" line, the PR log, the phases, the measurements, the
  open work and "waiting on you". While web is active, the page opens with a web section
  (the Architecture C steps C1..C8 and their results, the spike kept as a short record);
  scratchy's sections stay, marked paused.
- Append to coordinator/STATE.md after every merge or decision, and commit it to main,
  so nothing is lost if the power goes down or the session ends.

## web: Architecture C

- The plan is web/DECISIONS.md "Architecture C" and web/TODO.txt: `web-shell`, a small Wayland
  compositor that owns the bar, command line and keys, drives one engine host per tab over our
  own control socket. Blink (CEF) first; WebKit (E1) after C8.
- C1..C3 are a chain: run them in order, one agent at a time on it. Use Opus for C1 (the engine
  host and control protocol) and C2 (the compositor); C3 (porting keys and the command line
  from scratchy) may use Sonnet. C5 and C6 follow C3. P-FORGET and P-COOKIES may run beside the
  chain once C1 is merged. C7 (link hints) says WAITING FOR USER: never schedule it. The #704 plan
  (2026-11-23) adds C-PICK -> C-BOOKMARKS -> C4 Buffers -> C-POPUPS in that order; C4-SPIKE may run
  beside C-PICK.
- Network: the engine host takes an optional `--proxy <host:port>`; a missing flag is never an
  error. web itself enforces nothing; keeping it off the network is a wrapper outside web.
- When web gets an installable crate or a CI job, add its path rule to the `changes` job in
  .github/workflows/ci.yml and its name to ci_then_push.sh's `--install` choices.
- Steps that need my hands go under "Waiting on you", with what the agent prepared (C8: a day
  of browsing). Agents never type my credentials, never use my browser profiles or cookies,
  and never log in to my accounts. CEF's binary distribution is never committed.
- web/spike/ and the W1..W7 entries are the spike's record: never schedule them. W6's partial
  work is on branch web/w6-bypass, W5's download handler on web/w5-streaming (C1 takes it).

## wire2

- Active (2026-11-23). wire2/SPEC.md is the design and wins over TODO.txt. W2-1..W2-3 and W2-F
  are a chain: one agent at a time, Opus. When wire2 gets a crate and CI job, add its path rule to
  the `changes` job and its name to ci_then_push.sh's `--install` choices.

## Rules

- On my machine: every local build runs under `nice -n 19 ionice -c3`, and test binaries
  run at normal priority. Perf runs go through `flock /tmp/scratchy-perf.lock`. Every
  scratchy, CEF or test process runs under
  `systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0` (the scope covers CEF's
  child processes). Never use `ulimit -v`. /tmp is RAM, so no build directories or CEF
  downloads there.
- Never kill by name (no pkill or killall; `pkill wire` also kills pipewire). Kill only
  PIDs you recorded. No `pgrep -f` wait loops.
- Never use my own wire socket, /run/user/1000/wire.sock. Agents start their own wire, with
  its cache in /tmp and `--cache-size 256M`.
- Delete by explicit name only; no globbed rm.
- scratchy never opens network sockets itself; everything goes through wire, enforced with
  seccomp. web does not enforce this itself (see "web: Architecture C" above).
- This repository is public. Never commit personal data: my email, my recordings, cookies,
  login state, wire logs or screenshots of my browsing, or anything from .coordinator/.
