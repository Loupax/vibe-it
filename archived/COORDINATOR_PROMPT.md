# Coordinator prompt: the scratchy and wire parts

Moved out of the root COORDINATOR_PROMPT.md on 2026-11-25, when scratchy and wire were archived.
Kept for the record; none of it applies any more.

**PAUSED (user, 2026-11-23): scratchy development is paused. Do not triage, launch agents or
merge scratchy items. Development moves to `web/` (see web/DECISIONS.md). Ask the user before
resuming.** (The drain of the three agents running at the pause finished on 2026-11-23.)

| project | status | work list | item ids |
|---|---|---|---|
| `wire/` | PARKED (user, 2026-11-23: "We only work on wire2 now") | wire/TODO.txt | W-NAME (e.g. W-POOL5) |
| `scratchy/` | PAUSED (see the banner above) | scratchy/TODO.txt | Q... |

## Old wording (before 2026-11-25)

- scratchy/DECISIONS.md "Direction and process": the process decisions there (merging, CI,
  `python3 -I coordinator/done.py --project=<dir> "$PWD" <ITEM> <sha> "#<pr>"` (the default
  project is scratchy).
  - It rebases onto the latest main. Conflicts go back to the authoring agent, except
    the keep-both cases: scratchy's PROMPTS.md, REGRESSIONS.md, add-only NOTES.md and the mod lines in
  - Push, wait for CI, squash-merge. CI runs only the jobs for crates the change touches (the
    `changes` job; a wire change also runs scratchy and its WPT shards); the script requires
  - Then pull and run `coordinator/ci_then_push.sh --install <crate>`: `scratchy` (the
    default), `wire` or `web` (runs web/install.sh into ~/.local) for PRs that change them, `none` when nothing installable changed
  (the Architecture C steps C1..C8 and their results, the spike kept as a short record);
  scratchy's sections stay, marked paused.
