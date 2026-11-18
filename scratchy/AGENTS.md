# scratchy — agent instructions

## Before starting
- **FREEZE LIFTED (coordinator, 2026-11-18, after QV8-m at 92dd513/0c25634).** V8 is the default
  engine (QuickJS only behind `engine-quickjs` until QV8-6, which needs the user's OK). Work
  the TODO.txt queue again in order (READY items). QV8-5b (clippy in media_mpv.rs) comes first;
  the MSE build (QM1k4 onward, plan in TODO.txt QM1k-PLAN) is allowed now.
- Read `TODO.txt` (QUEUE section first), `NOTES.md`, `REGRESSIONS.md`.
- Take the first unchecked QUEUE item (during the freeze: QT1, then the first unchecked QV8 item).
  Don't reorder or merge items.
- If an item is wrong or much bigger than its size, stop and report. Don't guess.
- Find code by function/type name (`grep -rn 'fn <name>' src/`). Never trust line numbers.

## Per-item checklist
1. Implement.
2. Add a permanent test in the matching area file of `src/tests/` (the `#[cfg(test)] mod tests`;
   e.g. `cascade_tests.rs`, `box_layout_tests.rs`, `js_dom_tests.rs`, `fetch_tests.rs`, `forms_tests.rs`;
   shared helpers live in `src/tests/mod.rs`). A new area gets a new `src/tests/<area>_tests.rs` plus a
   `mod <area>_tests;` line in `src/tests/mod.rs`. Never delete tests.
3. `cargo check`, `cargo clippy`, `cargo test` — all clean. Clippy warnings are
   counted ONLY with `tools/clippy-count.sh [outfile]` (run from `scratchy/`; it runs
   `cargo clippy --release --all-targets --message-format=json` and counts DISTINCT
   warnings = unique (lint name, file:line:col), so per-target duplicates and the
   "N warnings emitted" lines don't inflate it; `--all-targets` so test code and
   benches are included). Quote that TOTAL, never a raw output line count. Your change
   must not raise it above the baseline, which lives in ONE file,
   `tools/clippy-baseline.txt` (a single number; CI reads it, and lowering it is a
   one-line change; the list of warnings goes to `outfile`, diff it against
   main's to find new ones). CI (`.github/workflows/ci.yml`: `scratchy` = the DEFAULT build, V8 + media, V8 archive cached and
   sha256-pinned; `wire`, `snag`: build + test; `scratchy-quickjs` = the QuickJS fallback,
   `--no-default-features --features engine-quickjs,media`, until QV8-6 removes it; it also
   runs the clippy count with `CLIPPY_FLAGS="--no-default-features --features engine-quickjs"`,
   the config baseline 78 was measured on; media_mpv.rs adds 12 uncounted warnings, QV8-5b) must be
   green on the PR before merge. Check all four in `statusCheckRollup`. All tests must pass on
   BOTH configs (run the second with its own target dir:
   `CARGO_TARGET_DIR=... cargo test --release --no-default-features --features engine-quickjs,media`).
   A plain `cargo build --release` / `cargo install --path .` gives V8 + media (libmpv is loaded
   by dlopen; without it installed the inert placeholder stays). Exactly one of `engine-v8` /
   `engine-quickjs` must be on (compile_error! otherwise). The first clean V8 build takes ~15 min.
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
- Tests must not depend on the system sans-serif (R33): `new_font_system()`
  resolves `sans-serif` through `fc-match`, so a font-specific number (a glyph
  width) fails on another machine. Assert relations, or load Ahem. To try a
  different font set locally: make a directory with only that font, write a
  minimal `fonts.conf` (`<dir>` to it and a private `<cachedir>`), and run
  `FONTCONFIG_FILE=/path/fonts.conf cargo test --release` (check with
  `FONTCONFIG_FILE=... fc-match sans-serif`). Recipe in tools/README.
- Performance claims need measurements (temporary `Instant` timing or
  `cargo bench`), not reasoning. Remove timing code before committing.
  Exception: the permanent `--perf-report` / `tools/perf-suite.sh` harness from QV8-0;
  use it for any before/after claim about script or forced-layout cost.
- Before killing a scratchy process, `pgrep -fa scratchy` and kill
  specific PIDs. The user may have their own instance running.
- Cross-references in tracking files use commit hashes, not `vN` labels.
- `PROMPTS.md` is append-only.
- Don't vendor third-party source (WPT, browser UA sheets). Use it as reference.
- scratchy never talks to the internet itself: no network crates, no TCP/UDP/DNS,
  only unix sockets to wire (user rule 2026-11-17, enforced by TODO.txt QNET1). New
  protocols (WebSocket, EventSource, media) go through wire. A dependency or
  library that wants its own network access needs the user's OK first.
  The `v8` crate downloading its prebuilt library at BUILD time is approved (user,
  2026-11-18, QV8-1); at run time V8 still never opens a socket.

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
- **MEDIA DESIGN AGREED 2026-11-17 (QM1-D D1-D8 in TODO.txt)**: in-page `<video>`/
  `<audio>` through libmpv IN-PROCESS as an OPTIONAL dependency (without it the
  inert placeholder stays), ALL bytes through wire (never the player's own network),
  scratchy-painted controls, no autoplay except muted or after a user gesture, at most
  2 players. MediaSource: GOAL since 2026-11-17 = video on all sites except DRM (EME
  stays out); the MSE build follows QM1k-PLAN in TODO.txt (user answered Q4 2026-11-18). QM1i ("open in mpv") is DROPPED. Only
  the QM1b..QM1h items may be built, in TODO.txt order and each with its own approval
  in the item; anything beyond them still follows the rules below.
- **FORMS: GO-AHEAD 2026-11-17 (user)**: the user only wanted to be present for the
  DESIGN of form controls, and will redesign once the functionality exists. Implement
  Tier 6 (Q30a-Q30e, Q30g, and the form parts of Q33e/QB11/QSD11) as REAL behaviour,
  in TODO.txt order, each item with its own tests: real boxes and UA styles, focus and
  click, checkbox/radio state, text editing in input/textarea, GET and POST
  submission through wire, `<select>`. Keep the perf rule in Tier 6 (no whole-layout
  rebuild per keystroke). The Forms bullet and the forms placeholder paragraph below
  are SUPERSEDED by this go-ahead (media rules still apply as agreed above).
- **Audio and video** (rules below apply except as agreed above): `<video>`, `<audio>`, `<source>`, `<track>`,
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
The same placeholder rule applies to FORMS: form controls (`<input>`,
`<textarea>`, `<select>`, `<button>`) may render as inert boxes that do
nothing (no text editing, no caret, no focus ring logic beyond what exists, no
dropdown UI), and form JS APIs may exist as plain DOM reflections, no-ops or
`not implemented` error logs (e.g. `form.submit()` logs and does nothing) so
page scripts don't die. No live form state, no value editing, no validation,
no submission/network POST, no cookies for forms (Q30/Q30e/Q30f wait for the
user's design).
If an item or page seems to need more than a placeholder, stop, write what it
needs in TODO.txt as a `[ ]` item marked "WAITING FOR USER (design first)", and
move on to something else.

## Acid2 / Acid3 are information, not a merge gate
The Acid2 and Acid3 results (including the saved per-test Acid3 harness and the
live http://acid3.acidtests.org/ score) must NEVER block an item or its merge.
Report the score in your PR/report for information, nothing more. Triage:
1. A test fails because of a feature that is planned later (it has a `[ ]` item
   in TODO.txt, e.g. Range QA19, real form state Q30, media): leave it alone
   until that item is implemented. Do not work around it and do not chase it.
2. A test fails because of a missing feature that is NOT yet listed anywhere:
   do not guess a fix. Note it in your report ("Acid3 Tnn fails on <what you
   saw>, not in TODO.txt") and the coordinator will have an Opus investigator
   (read-only, docs only) audit it and file action items (as the QA1..QA21
   audit did). Sonnet/Haiku agents then implement those items.
3. The score DROPS because of a bug in your own change: that is a normal
   regression; fix it before merging.
The WPT subset rule (no pass->fail) is separate and still blocks merges.

