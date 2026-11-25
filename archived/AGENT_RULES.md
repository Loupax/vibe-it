# Agent rules: the scratchy and wire parts

Moved out of coordinator/AGENT_RULES.md on 2026-11-25, when scratchy and wire were archived.
Kept for the record; none of it applies any more.

## New tests go in their own file (2026-11-22: DOM PRs kept colliding at the end of js_dom_tests.rs)
Put your item's new tests in a NEW file `scratchy/src/tests/<item-lowercase>_tests.rs` plus one `mod <item>_tests;` line in
`src/tests/mod.rs` (the merge script keeps both sides of mod lines). Only edit an existing test file to change a test that
already lives there. This overrides AGENTS.md "matching area file" for new tests.

## wire's HTTP cache (once W-CACHE1 lands)
W-CACHE1 makes wire cache on disk by default in `~/.cache/wire`. Every wire an agent starts uses
`--cache-dir /tmp/wire-cache-<your worktree name> --cache-size 256M` (user 2026-11-22: agents' caches in
/tmp; /tmp is RAM, hence the size cap; wire prints one tmpfs warning line, expected), or `--no-cache`.
Never the user's `~/.cache/wire`. The tools that start a wire (tools/incr-gate/run.sh, wl-flood, missing-api corpus/cost) take a `WIRE_ARGS`
env var: set `WIRE_ARGS="--cache-dir /tmp/wire-cache-<worktree> --cache-size 256M"` (or `--no-cache`) for every run. Delete your cache dir by its explicit name when you finish. Perf
comparisons use the same cache setting on both sides (say which).

## WPT checkout
Never use /home/loupax/src/wpt: it is a stale sparse checkout (no dom/, url/, html/canvas, ...) and gives thousands of
bogus 404/CRASH results. Always make your own pinned checkout under your worktree with tools/wpt-checkout.sh.


## Old title

# Rules for every scratchy agent. These OVERRIDE scratchy/AGENTS.md where they differ.
