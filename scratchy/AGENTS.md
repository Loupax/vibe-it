# scratchy — agent instructions

## Before starting
- Read `TODO.txt` (QUEUE section first), `NOTES.md`, `REGRESSIONS.md`.
- Take the first unchecked QUEUE item. Don't reorder or merge items.
- If an item is wrong or much bigger than its size, stop and report. Don't guess.
- Find code by function/type name (`grep -rn 'fn <name>' src/`). Never trust line numbers.

## Per-item checklist
1. Implement.
2. Add a permanent test in `src/tests.rs` (the `#[cfg(test)] mod tests`). Never delete tests.
3. `cargo check`, `cargo clippy`, `cargo test` — all clean.
4. WPT subset (once Q40a lands): `scratchy --wpt /home/loupax/src/wpt` (run from `scratchy/`; reads
   `wpt-subset.txt`, compares with `wpt-expected.txt`, exits 1 on any change;
   `--update` rewrites expectations; `--out DIR` saves failing PNGs). A new failure is a
   regression and blocks the commit. A newly passing test: update the
   expectations file in the same commit.
5. Do NOT run `cargo install --path .` (agents run concurrently and all would
   overwrite the same `~/.cargo/bin/scratchy`, so it ends up being some other agent's
   branch). Test with your own worktree's `target/release/scratchy`. The coordinator
   installs from up-to-date `main` after each merge.
6. Real `grim` screenshot of a real page, not only a fixture. For paint,
   hover or scroll changes, drive real input: `tools/nest.sh` (nested dwl)
   plus `tools/inject` (virtual pointer/keyboard); see `tools/README`.
   Recipe background is in `REGRESSIONS.md` "How this was tested".
7. Add a `PROMPTS.md` entry. Update `NOTES.md` if architecture changed.
8. Tick the item in `TODO.txt` (QUEUE and its Detail entry). Add a new
   narrower `[ ]` item for anything deliberately left out.
9. Commit, then a separate "fill in the hash" commit. Never amend.

## Rules
- Performance claims need measurements (temporary `Instant` timing or
  `cargo bench`), not reasoning. Remove timing code before committing.
- Before killing a scratchy process, `pgrep -fa scratchy` and kill
  specific PIDs. The user may have their own instance running.
- Cross-references in tracking files use commit hashes, not `vN` labels.
- `PROMPTS.md` is append-only.
- Don't vendor third-party source (WPT, browser UA sheets). Use it as reference.

## Background shells and waiting (don't leak processes)
- Never wait for a process with `while/until pgrep -f "<pattern>"; do sleep N; done`.
  `pgrep -f` matches full command lines, including the wait loop's own, so the
  loop finds itself and never exits. A run once left 24 such shells alive for
  up to 8 hours.
- Run long commands (`cargo build/test/install`, `--wpt`) with
  `run_in_background` (or in the foreground with `timeout`) and wait for the
  exit notification. Don't build a polling loop around them.
- If a wait loop is truly unavoidable, wait on a PID you own (`kill -0 $PID`)
  or use `pgrep -x`, and always wrap it in `timeout`.
- Never `pkill -f` with a pattern that also appears in your own command line;
  it can kill your own shell. Kill specific PIDs.
- Before you finish, check for background shells you started and kill them
  (specific PIDs only; never the user's scratchy/compositor).
- Don't spawn subagents from inside an agent task.

## Off limits until the user says so (design first)
Do NOT implement the real behaviour of these until the user has explicitly
asked and the design has been agreed. The user wants to understand how they
will work first (same stance as forms, Q30):
- **Audio and video**: `<video>`, `<audio>`, `<source>`, `<track>`,
  `HTMLMediaElement` playback (`play()`/`pause()`/`currentTime`/events),
  codecs, MediaSource/EME, Web Audio, `<canvas>` capture/WebRTC, autoplay.
- **Forms** (Q30 and children): live form state, text input widgets,
  form submission. QA20/Q33d1 only added DOM reflections and event stubs.

What IS allowed (placeholders so the rest of the page keeps working, and
nothing more):
- Render the element as an inert box (a poster image if it has one, else an
  empty replaced box with its width/height) that does nothing.
- Make the JS API exist but fail loudly and harmlessly: e.g. `video.play()`
  logs a clear `not implemented` error line (QJ2 log format) and returns a
  rejected promise / does nothing, instead of throwing `not a function` and
  killing the surrounding script.
No decoding, no audio output, no playback state machine, no controls UI.
If an item or page seems to need more than a placeholder, stop, write what it
needs in TODO.txt as a `[ ]` item marked "WAITING FOR USER (design first)", and
move on to something else.
