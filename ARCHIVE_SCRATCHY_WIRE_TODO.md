# TODO A-ARCHIVE1: move scratchy/ and wire/ into archived/

For: a Haiku agent. Size: small, mechanical. Item id: `A-ARCHIVE1`.

User, 2026-11-25: "scratchy and wire are deprecated and no longer relevant." Move both projects
into a new `archived/` folder. Take what the central docs say about them out of those docs, and keep it
for the record in matching files under `archived/`.

Follow coordinator/AGENT_RULES.md (branch, staging files by name, PR, CI, `READY` reply). Branch:
`archive/a-archive1-scratchy-wire`. Commit message: `archive: A-ARCHIVE1 move scratchy and wire to archived/`.

## Rules for this item

- Use `git mv` for every move, so history follows the files.
- Do not edit any file inside `archived/scratchy/` or `archived/wire/`. They are frozen as they are.
- Ignore every `PROMPTS.md` (root, scratchy's, wire's, wire2's): they are deprecated and being
  removed. Do not edit them, do not copy them, do not mention them in the archive docs. They move
  with their folder; that is fine.
- Do not edit `coordinator/STATE.md` (an append-only log; old entries stay as written) or any
  `DECISIONS.md` / `TODO*.txt` outside archived/.
- Do not change source code, tests or scripts (`*.rs`, `*.py`, `*.sh`, `Cargo.*`). Comments in
  code that mention scratchy or wire stay. The one non-doc file you change is the CI workflow
  (step 5), because CI would break without it.
- When you move text into an archive file, copy it word for word. Only fix relative links (step 6).
- When a step below says "remove X", remove only X. Do not reword the text around it beyond
  what the step gives.

## Steps

### 1. Move the folders

```bash
mkdir archived
git mv scratchy archived/scratchy
git mv wire archived/wire
```

### 2. `archived/README.md` (new) from the root `README.md`

Create `archived/README.md`:

```
# archived

Projects that are no longer developed (user, 2026-11-25: "scratchy and wire are deprecated and
no longer relevant"). Kept for the record; nothing here is built, tested or installed. Their
sections below were moved here from the root README.md.

## Projects
```

Then MOVE (cut from the root README.md, paste here) these parts of the root `README.md`, in order:

1. Under "## Projects": the `scratchy/` bullet (starts ``- [`scratchy/`](scratchy/) — **paused 2026-11-23``,
   5 lines) and the `wire/` bullet (starts ``- [`wire/`](wire/) — a small daemon``, 4 lines).
2. The whole section `## How they fit together` (heading and both paragraphs).
3. The whole section `## Usage` (heading, the ```` ```sh ```` block, and the line
   ``Inside `scratchy`, `:help` lists the keys and commands.``).

In the root README.md, also change the end of the `snag/` bullet from
``a one-shot way to get a first page for `scratchy`.`` to
``a one-shot way to get a first page for the archived `scratchy`.``

Then add this bullet at the end of the root README's "## Projects" list:

```
- [`archived/`](archived/) — deprecated projects kept for the record (`scratchy`, `wire`).
```

### 3. `archived/COORDINATOR_PROMPT.md` (new) from the root `COORDINATOR_PROMPT.md`

Create `archived/COORDINATOR_PROMPT.md` starting with:

```
# Coordinator prompt: the scratchy and wire parts

Moved out of the root COORDINATOR_PROMPT.md on 2026-11-25, when scratchy and wire were archived.
Kept for the record; none of it applies any more.
```

Then MOVE these from the root `COORDINATOR_PROMPT.md`:

1. The PAUSED banner: the first paragraph (3 lines, starts `**PAUSED (user, 2026-11-23): scratchy`)
   plus the blank line after it.
2. The two table rows that start ``| `wire/` |`` and ``| `scratchy/` |``. In the archive file put
   them under a copy of the table's two header lines (`| project | status | ...` and `|---|...`).

Then EDIT the root `COORDINATOR_PROMPT.md` in place (do not copy these into the archive):

3. In "## Start by reading", the bullet ``scratchy/DECISIONS.md "Direction and process": ...``:
   change the path `scratchy/DECISIONS.md` to `archived/scratchy/DECISIONS.md`. Keep the bullet;
   those process decisions still apply to every project.
4. In "## Work", the bullet about `done.py`: the text `(the default` ends one line and
   `project is scratchy).` starts the next. Replace those two pieces with `(always pass --project).`
5. In the merge bullets: in the sentence that starts `Conflicts go back to the authoring agent, except`,
   remove `scratchy's PROMPTS.md, REGRESSIONS.md, add-only NOTES.md and the mod lines in
   tests/mod.rs, and ` so it reads `except the keep-both cases: the root PROMPTS.md.`
   Remove the sentence part ` (the `changes` job; a wire change also runs scratchy and its WPT shards)`
   and put back ` (the `changes` job)`.
6. The bullet that starts `Then pull and run `coordinator/ci_then_push.sh --install <crate>``:
   the text `` `scratchy` (the`` ends one line and `` default), `wire` or `web` (runs web/install.sh into
   ~/.local) for PRs that change them, `none` `` starts the next. Replace those pieces with `` `wire2` or `web` (runs web/install.sh into ~/.local) for PRs that change them (always pass
   --install: the script's default is the archived scratchy), `none` ``. Keep the rest of the bullet.
7. In the progress-page bullet, the line `  scratchy's sections stay, marked paused.` follows a line
   ending in `kept as a short record);`. Remove that whole line and change the `);` at the end of
   the line before it to `).`

Copy into the archive file, under a heading `## Old wording (before 2026-11-25)`, the original
text of every line you changed in 3..7, so the old instructions are on record.

### 4. `archived/AGENT_RULES.md` (new) from `coordinator/AGENT_RULES.md`

Create `archived/AGENT_RULES.md` starting with:

```
# Agent rules: the scratchy and wire parts

Moved out of coordinator/AGENT_RULES.md on 2026-11-25, when scratchy and wire were archived.
Kept for the record; none of it applies any more.
```

Then MOVE these whole sections (heading through the last line before the next `## `) from
`coordinator/AGENT_RULES.md` into it, in this order:

1. `## New tests go in their own file (2026-11-22: ...)` (scratchy's src/tests layout)
2. `## wire's HTTP cache (once W-CACHE1 lands)`
3. `## WPT checkout`

Leave every other section and the numbered rules 1..12 where they are. Then make only these edits in
`coordinator/AGENT_RULES.md`:

- Line 1: `# Rules for every scratchy agent. These OVERRIDE scratchy/AGENTS.md where they differ.` becomes
  `# Rules for every agent. These OVERRIDE a project's AGENTS.md where they differ.`
  Copy the old line 1 into the archive file under `## Old title`.
- In rule 3, `NEVER edit scratchy/TODO.txt or scratchy/TODO-DONE.txt` becomes
  `NEVER edit any TODO.txt or TODO-DONE.txt`.
- In "## No conflict markers, ever", `-- scratchy coordinator` becomes `-- . ':!archived'`.

### 5. CI: `.github/workflows/ci.yml`

Every job that builds scratchy or wire must go, or CI fails (a `.github/` change runs every job).

1. First copy the file as it is now, for the record: `cp .github/workflows/ci.yml archived/ci.yml`
   and add it with `git add archived/ci.yml`. (Outside `.github/`, GitHub never runs it.)
2. In `.github/workflows/ci.yml`:
   - `changes` job: remove the output line `scratchy: ${{ steps.f.outputs.scratchy }}`, the lines
     `sc=false; has '^(scratchy|wire)/' && sc=true`, `[ $sc = true ] && crates+=('"scratchy"')`,
     `has '^wire/' && crates+=('"wire"')` and `echo "scratchy=$sc" | tee -a "$GITHUB_OUTPUT"`.
     In the comment above the job, remove the sentence `scratchy's tests and the WPT shards run
     wire, so a wire change runs them too;`.
   - `test` job: remove every step whose `if:` is `matrix.crate == 'scratchy'` or
     `always() && matrix.crate == 'scratchy'` (whole step, from its `- name:` to the line before the
     next `- `). Remove the `V8_VERSION`, `V8_ARCHIVE_NAME` and `RUSTY_V8_ARCHIVE_SHA256` lines and
     then the now-empty `env:` key of the job, and the 3-line `# QV8-5` comment above `test:`.
     In the `Swatinem/rust-cache` step, remove the line `wire` under `workspaces:`.
     On the step `- name: Test` remove the line `if: matrix.crate != 'scratchy'`.
   - Remove the whole `wpt:` job and the comment block right above it (from `# QCISPLIT:` to the
     end of the file).
3. Check: `grep -n -i -E 'scratchy|wire\b|v8' .github/workflows/ci.yml` prints nothing (`wire2`
   may appear only as `wire2`), and
   `python3 -c 'import yaml,sys; yaml.safe_load(open(".github/workflows/ci.yml"))'` exits 0
   (if PyYAML is missing, say so in the PR and rely on CI).

### 6. Links in the active projects' docs

Only these links point into the moved folders. Change each `../scratchy/` to `../archived/scratchy/`
and `../wire/` to `../archived/wire/`; change nothing else on the line:

- `web/README.md` lines 24-25 (`../scratchy/` and `../scratchy/DECISIONS.md`)
- `web/KEYS.md` lines 3 and 6
- `web/STASH.md` line 3
- `web/TODO.txt` line 16 (`../scratchy/tools/nest.sh`) — this one TODO.txt edit is allowed, it is a path only
- `wire2/README.md` line 53 (`../wire/`)

In the archive files you created, fix relative links so they still resolve from `archived/`:
`](scratchy/` and `](wire/` stay as they are (the folders are next to the file now).

Check: `grep -rnE '\.\./(scratchy|wire)/' --include='*.md' --include='*.txt' web wire2 snag` prints nothing.

### 7. Finish

- Delete this file: `git rm ARCHIVE_SCRATCHY_WIRE_TODO.md`.
- `git status --short` must show only: the two renames, `README.md`, `COORDINATOR_PROMPT.md`,
  `coordinator/AGENT_RULES.md`, `.github/workflows/ci.yml`, the files in step 6, the new
  `archived/README.md`, `archived/COORDINATOR_PROMPT.md`, `archived/AGENT_RULES.md`,
  `archived/ci.yml`, and the deleted `ARCHIVE_SCRATCHY_WIRE_TODO.md`.
- Open the PR. The body lists each step and what you changed, then a section "Follow-ups (not done
  here)" with exactly these, for the coordinator:
  - `coordinator/ci_then_push.sh` still defaults `--install` to scratchy.
  - `coordinator/done.py` still defaults `--project` to scratchy; `coordinator/move_closed.py`
    still reads `scratchy/TODO.txt`.
  - `coordinator/merge_pr.sh` still has scratchy-specific conflict rules (PROMPTS/REGRESSIONS/
    areas.txt, the `--areas-only` rebuild).
  - Code comments in `snag/`, `web/` and `wire2/` still mention scratchy, wire and
    `/tmp/scratchy-perf.lock` (the lock path is still in use; renaming it is a separate decision).
- Wait for CI (`changes`, `snag`, `wire2`, `web/shell` should run and pass; no scratchy or wire
  check should appear), then reply `READY #<pr> A-ARCHIVE1 — scratchy and wire moved to archived/`.
