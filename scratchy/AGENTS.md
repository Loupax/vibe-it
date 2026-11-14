# scratchy — agent instructions

## Before starting
- Read `TODO.txt` (QUEUE section first), `NOTES.md`, `REGRESSIONS.md`.
- Take the first unchecked QUEUE item. Don't reorder or merge items.
- If an item is wrong or much bigger than its size, stop and report. Don't guess.
- Find code by function/type name (`grep -n 'fn <name>' src/lib.rs`). Never trust line numbers.

## Per-item checklist
1. Implement.
2. Add a permanent test in `src/lib.rs` `#[cfg(test)] mod tests`. Never delete tests.
3. `cargo check`, `cargo clippy`, `cargo test` — all clean.
4. WPT subset (once Q40a lands): `scratchy --wpt /home/loupax/src/wpt` (run from `scratchy/`; reads
   `wpt-subset.txt`, compares with `wpt-expected.txt`, exits 1 on any change;
   `--update` rewrites expectations; `--out DIR` saves failing PNGs). A new failure is a
   regression and blocks the commit. A newly passing test: update the
   expectations file in the same commit.
5. `cargo install --path .`
6. Real `grim` screenshot of a real page, not only a fixture. For paint,
   hover or scroll changes, also run the AUTOSCROLL/AUTOHOVER recipe in
   `REGRESSIONS.md`.
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
