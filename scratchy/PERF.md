# PERF.md: scratchy script performance, measured the same way on every engine

Purpose: the V8 migration (TODO.txt, QV8-0 .. QV8-m) starts and ends with the SAME
measurement. QV8-0 takes the QuickJS baseline; QV8-m reruns exactly the commands below on
V8 and fills the comparison. The harness is permanent code (`src/perf.rs`,
`tools/perf-suite.sh`, `tools/perf-corpus.sh`), the numbers are not: nothing here is
measured by the harness-writing step. The BASELINE tables are filled in by the QV8-0
measurement step at the drained main hash. Do not put guesses in them.

## How to run

All commands run from `scratchy/` with a release build (`cargo build --release`), on a
quiet machine (load average well under 1), with the user's normal wire daemon running.

```
# one page, one JSON object on stdout (diagnostics on stderr)
target/release/scratchy --perf-report --socket /run/user/$UID/wire.sock \
    --seconds 10 'https://en.wikipedia.org/wiki/Rust_(programming_language)'

# the same page with nothing killed (the "true cost" pass)
target/release/scratchy --perf-report --socket /run/user/$UID/wire.sock \
    --budget-ms 20000 --seconds 10 'https://en.wikipedia.org/wiki/Rust_(programming_language)'

# a local file instead of a URL
target/release/scratchy --perf-report some-page.html

# QY53i2: several pages = several navigations in ONE process, one JSON line each
# (cross-navigation memory: does a cache grow from page to page?)
target/release/scratchy --perf-report --socket /run/user/$UID/wire.sock --seconds 30 \
    'https://www.youtube.com/watch?v=G1AeBgalFZk' 'https://en.wikipedia.org/wiki/Rust_(programming_language)'

# the whole suite: 7 live pages x 3 runs x 2 budgets, plus the corpus; writes
# perf/<engine>-<commit>.json (perf/ is gitignored); slow, run it when nothing else is busy
tools/perf-suite.sh

# smoke run: one page, one run, no corpus
RUNS=1 SKIP_CORPUS=1 PAGES='https://en.wikipedia.org/wiki/Rust_(programming_language)' tools/perf-suite.sh

# the offline engine corpus alone (downloads pinned files through wire into a temp dir,
# verifies sha256, runs `scratchy --perf-corpus <dir>`)
tools/perf-corpus.sh [dir]
```

`--budget-ms N` is a permanent debug flag of every mode (normal window, `--js-report`,
`--perf-report`): the per-callback CPU budget in milliseconds, default 500. The page-load
budget of one script stays 5 s unless `N` is larger (then it is `N`). The promise-drain cap
is 4 x `N`.

Pages (live, through the user's normal wire socket with scratchy's normal User-Agent, no
override): YouTube `/results?search_query=rust`, YouTube home, the English Wikipedia
article `Rust_(programming_language)`, `social.loupax.com/discover`,
`github.com/Loupax/vibe-it`, `old.reddit.com`, `bbc.com/news`. Not DuckDuckGo. The suite
never tries to pass a bot or JS challenge: a page that answers with one is recorded as
`challenge` and skipped. Live pages drift, which is why the corpus exists.

What the load does: fetch the HTML (navigation request), the page's stylesheets (so forced
layouts run against the real CSS) and scripts through wire, run the scripts, fire
DOMContentLoaded and load, then keep running timers, promise jobs and network callbacks
for `--seconds` of real time (default 5; the suite uses 10). Images are not fetched.

## Profiling (perf)

A sampling profiler that attributes CPU to Rust functions AND page JS functions in one run
(QOBS1; background in REGRESSIONS.md R44). Local only: nothing is uploaded anywhere.

```
tools/profile.sh [--seconds N] [--ua UA] <url|file>     # default 30 s
tools/profile.sh 'https://en.wikipedia.org/wiki/Rust_(programming_language)'
```

What it does: builds the `profiling` cargo profile (release + line tables, no strip) with
`-C force-frame-pointers=yes` into `${PROF_TARGET:-$HOME/.cache/scratchy-prof-target}` (never
the repo's `target/`; reuses `RUSTY_V8_ARCHIVE` or the local `gn_out` archive), then runs
`perf record -e cycles:u -F 999 -g` on `--perf-report --budget-ms 20000` with
`SCRATCHY_V8_FLAGS='--perf-basic-prof --interpreted-frames-native-stack'` (scratchy skips the
V8 snapshot itself whenever `SCRATCHY_V8_FLAGS` is set, QOBS2). The wire socket is `$WIRE_SOCKET` or
`/run/user/$UID/wire.sock`. Output goes to `perf/profile-<commit>-<slug>/` (gitignored):
`perf.data`, `report.json` (the `--perf-report` JSON), `folded.txt`, `summary.txt`,
`flame.svg` (only if `inferno-flamegraph` is on PATH). The JIT maps (`/tmp/perf-<pid>.map`) and
the `isolate-*-v8.log` files the V8 flags write are removed afterwards.

Reading `summary.txt`: per-thread share of samples; top 30 SELF (where the CPU is); top 40
INCLUSIVE (a function plus everything it calls; a stack counts once); and the nearest PAGE JS
frames above `GeomCache::refresh` (frames starting `JS:` that are not scratchy's own
`eval:`/`timers:` prelude), i.e. which page function forces layouts. `[unknown]` callers mean the
binary lacked frame pointers (use profile.sh's binary, not `target/release`). Frames are
`JS:^name url:line:col` (optimized) or `JS:~name ...` (interpreted).

Querying a profile later (stdlib Python, run isolated; regexes are `re.search` on frame names,
which are `<scratchy::js_geom::GeomCache>::refresh` style, hence `GeomCache>?::refresh`):
```
python3 -I tools/perf-fold.py incl      perf/profile-.../folded.txt 'has_class' 'GeomCache>?::refresh'
python3 -I tools/perf-fold.py callers   perf/profile-.../folded.txt 'El::has_class' 8
python3 -I tools/perf-fold.py jscallers perf/profile-.../folded.txt 'GeomCache>?::refresh'
```

Needs `perf` (`dnf install perf`; the script exits 2 with that message if missing) and
`/proc/sys/kernel/perf_event_paranoid` <= 1, because `-g` call graphs fail at 2. The one-line
fix is not persistent (reset at reboot): `echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid`;
the script never runs sudo. A `-F 999` run costs roughly 5-10 % wall time, so compare profiles
with each other, not with unprofiled timings.

## Clocks

- **raw CPU**: CPU time of the JS thread (`CLOCK_THREAD_CPUTIME_ID`).
- **budget clock**: raw CPU minus the time spent in forced layouts (a geometry read after
  a DOM write). This is what the callback budget counts (REGRESSIONS.md R37), so a kill
  happens when the BUDGET clock of one callback passes the budget.
- Spans nest (an observer delivery inside a timer, a script inserted by a network
  callback). Each span records its EXCLUSIVE time, so the totals add up with no double
  counting; a timer that only ran an observer delivery still counts as one timer with ~0.

## Comparing two runs (`tools/perf-diff.py`)

**Every perf PR pastes `perf-diff` of main vs branch** (same machine, same pages, quiet box).

```
python3 -I tools/perf-diff.py old.json new.json [--threshold 5]
```

Takes two `--perf-report` objects or two `tools/perf-suite.sh` outputs and prints the numeric
fields that changed by more than the threshold (percent of old), largest first: page, field, old,
new, change. It walks the JSON generically (every numeric leaf by dotted path, so QOBS3/QOBS7
fields appear without changes); arrays of objects with a `name` are keyed by it, other arrays
such as `longest` are skipped (see `--help`). Diff main against main first to see the noise floor.

## JSON field glossary (`--perf-report`)

| field | meaning |
|---|---|
| `net_delivery` | QOBS6: `{count, p50_ms, p95_ms, max_ms}` of the delay between a page request result being complete and `deliver_net` delivering it. In the headless harness (`--perf-report`) the network is synchronous (result executed and delivered in the same loop turn), so the delay is ~0 (microseconds) there; the numbers are meaningful in the window (`SCRATCHY_FRAME_STATS` `net`) |
| `schema` | JSON layout version of `--perf-report`: **2** since QOBS4 (it adds `longest`, keeps every schema-1 field). QOBS3 and QOBS7 add THEIR fields to schema 2 (no further bump). `--perf-corpus` keeps its own `schema` 1 |
| `engine`, `engine_version` | JS engine name and version (QuickJS now; a V8 backend fills the same two fields) |
| `commit` | git commit the binary was built from (`-dirty` suffix for uncommitted changes), `unknown` outside a checkout |
| `target`, `title`, `page_bytes` | what was loaded; document title; HTML size |
| `budget_ms`, `seconds` | the `--budget-ms` and `--seconds` in force |
| `outcome` | `ok`, or `fetch_failed` (wire unreachable, unsupported type, no page); `error` holds the reason |
| `challenge` | true when the HTML looks like a bot/JS challenge page (the harness only reports it) |
| `callbacks.<kind>` | `classic_script`, `module_script`, `timer`, `promise_job`, `event`, `observer_delivery`, `net_callback` |
| `callbacks.<kind>.count` | callbacks run (an idle timer poll is not counted) |
| `callbacks.<kind>.kills` | callbacks stopped by the budget |
| `callbacks.<kind>.budget_clock`, `.raw_cpu` | `total_ms`, `max_ms`, `p50_ms`, `p95_ms` (nearest rank) in the budget clock and in raw CPU |
| `longest` | (schema 2) the 20 longest callbacks of the run by raw CPU, longest first (a bounded min-heap in the recorder). Each: `kind` (as `callbacks.<kind>`), `raw_ms` (exclusive raw CPU), `budget_ms` (exclusive budget clock), `forced_layouts` / `forced_ms` (forced layouts that ran directly in that callback, not in nested spans), `at_ms` (wall ms since the report started, when the callback ENDED), `source` |
| `longest[].source` | timers: `function-name url:line:col` of the callback's function (V8: `get_script_origin` and line/col, 1-based; `(anonymous)` for unnamed; QuickJS gives the function name only); scripts: the script name/URL; events: `event:<type>` / `DOMContentLoaded` / `load`; QOBS4b: a scratchy shim is looked through (`requestIdleCallback` names the page's callback), and a callback whose own source is a prelude wrapper (`media:108:22`, `globals:151:35`) or empty reports `event:<type> <slowest listener fn url:line:col>` when events were dispatched in it (V8 only; the dispatcher times each listener only while the recorder is on); `promise_job` and `net_callback`: empty (V8 exposes no reaction handler through rusty_v8, and a job has no single function to name) |
| `budget_kills` | sum of all `kills` |
| `forced_layout` | `count`, `total_ms`, `max_ms` of forced layouts (raw CPU, the part removed from the budget clock). (schema 2, QOBS3) Also: `phases` {`css_collect`, `rule_index` (RuleIndex + HoverKeys), `precompute_generated`, `build_children` (style cascade + box building, incl. the final tree passes), `measure_document` (incl. dropping the box tree), `glyph_walk`, `rect_walk` (incl. the `no_change` compare)}, each `total_ms` in thread CPU; they add up to `total_ms` within 5 %. `style_resolves` / `elements` = `resolve_style_for` calls / those for real elements (not `::before`/`::after`), summed over all layouts. `no_change` = layouts whose element-rect map equalled the previous layout's (wasted layouts). `by_reason` {`io`, `script`} each `count` + `total_ms`: `io` = the read came from scratchy's IntersectionObserver evaluation (a flag around `ioEval`), `script` = a page read. `by_caller` = the top 10 page callers by `total_ms`, each `{caller: "function url:line:col", count, total_ms}`: the first JS frame that is not a scratchy prelude, from `v8::StackTrace::current_stack_trace` taken only when a layout actually runs under `--perf-report` (not for `io` reads; QuickJS reports `[]`). |
| `compile_cpu_ms` | CPU spent compiling page scripts (a module's compile is inside its `module_script` run) |
| `engine_startup_cpu_ms` | CPU to create the runtime, install the DOM and run the preludes, before the first page script |
| `load.fetch_wall_ms`, `.fetch_cpu_ms` | start to "HTML, CSS and scripts in hand" |
| `load.dom_content_loaded_wall_ms`, `_cpu_ms`, `load.load_wall_ms`, `_cpu_ms` | start to the DOMContentLoaded / load event having run (start = before the HTML request) |
| `js_heap_bytes` | engine heap in use at the end (QuickJS `memory_used_size`) |
| `rss_bytes` | process resident set size at the end |
| `memory` | (schema 2, QOBS7) Taken ONCE at the end of the run, not per event. `rss_bytes`, `peak_rss_bytes` (QMEM3c: `VmHWM` of /proc/self/status, never below `rss_bytes`). `js_heap` {`used`, `total`, `external`} (V8 `get_heap_statistics`; QuickJS: `used` = memory_used_size, `total` = malloc_size, `external` null). `caches` = per cache `{entries, bytes}` for `custom_props_interner` (QY53i2: this thread's arena, one copy per distinct table, freed on navigation and at the start of each `--perf-report` page), `custom_props_memo` (QY53i), `shape_memo` (this thread's; the shaping worker's copy is not reachable), `rules_cache`, `rule_index_hover_keys`, `svg_sized_cache`, `image_cache` (always 0: a perf run has no App), `style_cache`, `mse_store` (media bytes in the MediaSource feeds), `geom_rects`, `geom_glyphs`. **`bytes` are ESTIMATES**: sums of String/Vec/HashMap capacities plus a fixed per-entry overhead, not allocator truth; QMEM3c: `rules_cache` counts each rule's selector components, media Vec and block text/parsed boxes, and the rule index (also when held by `RULE_DERIVED`); `shape_memo` counts per char one `ShapeGlyph` + one `LayoutGlyph` plus per-line overhead (cosmic-text's buffers are private, so approximate); interned atoms, and heap inside parsed `Declarations`, are not counted. `caches_bytes` = sum of the cache estimates; `explained_bytes` = `caches_bytes` + `js_heap.used`; compare it with `rss_bytes` for the share of RSS the listed parts explain. `walk_us` = how long this one end-of-run walk took. `malloc` {`in_use` (glibc `mallinfo2` uordblks+hblkhd), `free_kept` (fordblks: free bytes the allocator holds); null off glibc} shows the allocator share outside the listed caches. `stack_segments` {`kept` spare QSTK1 segments on this thread, `mapped_bytes` virtual size, MAP_NORESERVE: only touched pages are resident}. `media_players` = media elements with a core. Per-engine caches register a probe only when the recorder is on (zero cost otherwise). |
| `js_errors` | errors logged at level `error` (uncaught exceptions, killed callbacks, failed script loads) |
| `console_errors` | `console.error` calls |

QMEM3a (glibc arena cap): `main()` calls `mallopt(M_ARENA_MAX, 2)` before any thread exists (glibc Linux only, `src/malloc_arenas.rs`). On YT watch (headless, 60 s, 3 runs each) median `memory.rss_bytes` 508 MB -> 441 MB (-67 MB); 4 arenas gave 455 MB (-53 MB). `SCRATCHY_MALLOC_ARENAS=<n>` overrides the cap; `0` leaves glibc's default (use it to measure the old behaviour with the same binary).

A `module_script` span covers the `import()` entry and its drain: the promise jobs that load
and run the module graph are counted under it, not under `promise_job`.

`--perf-corpus <dir>` prints `{schema, engine, engine_version, commit, scripts:[...],
cpu_loops:[...]}`; each entry has `name`, `mode` (`bare` = plain engine without the DOM
prelude, `dom_prelude` = a page engine over an empty document; jquery and react-dom use
it, react-dom after react), `bytes`, `compile_cpu_ms` (parse/compile), `first_run_cpu_ms`
(running the script, promise jobs included) and `error` (null or the first line of the
exception). `cpu_loops`: `fib25`, `sort_200k_objects`, `json_round_trip`.

`tools/perf-suite.sh` output (`perf/<engine>-<commit>.json`): `pages.<url>.<true_cost|default>`
has `status` (`ok`, `challenge`, `fetch_failed`, `run_failed`), `runs`, `ok_runs` and
`median`, the per-field median of the ok runs (same shape as one report); `corpus` is the
per-field median of the corpus runs.

## Pinned corpus (never committed; downloaded at measurement time)

`tools/perf-corpus.sh` holds the table; QV8-m uses the identical bytes (sha256 checked).
moment 2.31.0, d3 7.9.2, lodash 4.18.1, three 0.159.0 (`build/three.min.js`, last release
that ships it), jquery 4.1.0, react 18.3.1 and react-dom 18.3.1 (UMD, 19 has none).

## Baseline (QuickJS, 2437284, 2026-11-18)

Taken by the QV8-0 measurement step (Opus), 2026-11-18 14:06-14:17 CET.

- **Binary:** `git checkout --detach 2437284` (main after PR #295, QT1), clean tree,
  `cargo build --release` (default features, so no `media`: Wikipedia logs one
  `not implemented: media (libmpv not available)` error), rustc 1.96.1. Every report says
  `"engine":"QuickJS"`, `"engine_version":"0.16.2 (rquickjs 0.14.0)"`, `"commit":"24372843cf"`
  (no `-dirty`).
- **Machine:** AMD Ryzen 5 PRO 2500U w/ Radeon Vega Mobile Gfx (4 cores / 8 threads,
  `nproc` 8), 14 GB RAM, Linux 6.18.41-gentoo-dist-bin, cpufreq governor `schedutil`. The
  user was away; nothing else ran (no builds, tests or other agents). Measurements un-niced.
- **Load average:** suite start 14:06:25 `0.30, 1.27, 1.83` (the 5/15-min values are the
  release build that finished at 14:04 and decaying); suite end 14:14:41 `0.44, 0.72, 1.33`;
  standalone corpus 14:16:44 `0.23, 0.54, 1.19` before and after. The 1-min load never went
  above ~0.5 from anything but the measurement itself.
- **Wire:** the user's normal daemon, `/run/user/1001/wire.sock`, scratchy's normal UA.
- **Commands** (from `scratchy/`):
  ```
  tools/perf-suite.sh            # RUNS=3, SECONDS_PER_PAGE=10, default 7 pages, corpus
  tools/perf-corpus.sh <dir>     # once more, standalone, same pinned bytes (sha256 ok)
  ```
  The suite was run from a scratchpad copy of `tools/perf-suite.sh` whose ONLY differences are
  a fixed `HERE=` path and no `trap 'rm -rf "$TMP"' EXIT`, so the 42 per-run JSON files survive
  (the committed script deletes them and keeps only medians; min/max below need them). Same
  binary, flags, timeouts and median code. Raw files are not committed.
- **No page answered with a bot/JS challenge** (`challenge:false` in all 42 runs, all
  `outcome:"ok"`). `old.reddit.com` served the NEW Reddit app (`<title>Welcome to Reddit`,
  `shreddit-app`, no old-reddit listing) to this client: measured as served, not a challenge.

Per page, two rows (true cost = `--budget-ms 20000`, default = 500 ms, what the user sees).
Each cell is the median of 3 runs, with (min-max) when the runs differ. "max callback" is the
largest single callback of any kind; "p95" is the highest per-kind p95 (the report has p95 per
kind, not over all callbacks). Times in ms, RSS/heap in MB.

| page | pass | loads? | kills | max callback, budget clock (kind) | max callback, raw CPU (kind) | highest per-kind p95, budget clock (kind) | forced layout total (count) | DOMContentLoaded wall / CPU | load wall / CPU | RSS | JS heap | JS errors | page bytes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| YouTube results (rust) | true cost | yes | 0 | 515 (514-520) (promise job) | 1431 (1423-1441) (promise job) | 169 (167-179) (observer) | 15615 (14612-17277) (25) | 4468 (4467-5202) / 2590 (2573-2710) | 4469 (4468-5202) / 2590 (2574-2711) | 420 (417-422) | 97.3 | 0 | 1631696 (1630748-1632003) |
| YouTube results (rust) | default | yes | **1** (promise job) | 502 (killed) | 1426 (1424-1429) (observer/promise job) | 185 (176-185) (observer) | 17048 (16916-17534) (26) | 4545 (4330-4809) / 2571 (2566-2584) | 4546 (4331-4809) / 2572 (2567-2584) | 423 | 98.9 | 1 (the kill) | 1631360 (1630651-1631608) |
| YouTube home | true cost | yes | 0 | 412 (412-413) (classic script) | 941 (918-948) (promise job) | 155 (observer) | 4198 (4126-4198) (20) | 4472 (4304-4652) / 2702 (2695-2702) | 4472 (4305-4653) / 2702 (2695-2702) | 394 | 76.5 | 0 | 1018469 (1017894-1019591) |
| YouTube home | default | yes | 0 | 423 (413-424) (classic script) | 956 (946-965) (promise job) | 155 (observer) | 3940 (3734-4216) (18) | 4258 (4204-4440) / 2678 (2676-2701) | 4259 (4205-4441) / 2679 (2676-2701) | 394 | 76.5 | 0 | 1018412 (1017777-1018559) |
| en.wikipedia Rust (programming language) | true cost | yes | 0 | 193 (190-205) (timer) | 2893 (2879-2920) (observer) | 31 (observer) | 10494 (10438-11941) (7) | 298 (289-418) / 158 (154-158) | 300 (291-420) / 160 (156-160) | 180 (180-187) | 13.1 | 1 (media) | 1065607 |
| en.wikipedia Rust (programming language) | default | yes | 0 | 195 (194-214) (timer) | 2890 (2885-2948) (observer) | 34 (31-34) (observer) | 11989 (11944-12044) (8) | 291 (284-298) / 139 (136-155) | 293 (286-300) / 141 (139-158) | 188 | 13.1 | 1 (media) | 1065607 |
| social.loupax.com/discover | true cost | yes | 0 | 51 (promise job) | 105 (promise job) | 51 (promise job) | 121 (10) | 410 (391-574) / 255 (224-255) | 411 (391-574) / 255 (224-255) | 31 | 3.0 | 0 | 35633 |
| social.loupax.com/discover | default | yes | 0 | 50 (promise job) | 105 (promise job) | 50 (promise job) | 121 (10) | 392 (364-405) / 223 (223-239) | 392 (365-405) / 223 (223-239) | 31 | 3.0 | 0 | 35633 |
| github.com/Loupax/vibe-it | true cost | yes | 0 | 179 (178-183) (module) | 179 (178-183) (module) | 37 (37-41) (timer) | 0 (0) | 1154 (1060-1785) / 538 (507-542) | 1157 (1062-1788) / 540 (509-544) | 53 | 11.8 | 4 | 212607 |
| github.com/Loupax/vibe-it | default | yes | 0 | 179 (178-183) (module) | 179 (178-183) (module) | 37 (37-38) (timer) | 0 (0) | 1095 (1088-1099) / 535 (516-548) | 1097 (1091-1101) / 537 (518-550) | 53 | 11.8 | 4 | 212607 |
| old.reddit.com (served new Reddit) | true cost | yes | 0 | 248 (236-250) (promise job) | 248 (236-250) (promise job) | 10 (module) | 75 (74-76) (1) | 373 (373-636) / 60 (58-61) | 374 (373-636) / 60 (58-61) | 109 | 13.4 | 8 | 340315 (340315-340325) |
| old.reddit.com (served new Reddit) | default | yes | 0 | 248 (246-251) (promise job) | 248 (246-251) (promise job) | 9 (module) | 76 (75-79) (1) | 431 (406-434) / 89 (86-91) | 432 (406-434) / 89 (86-91) | 109 | 13.4 | 8 | 340315 |
| bbc.com/news | true cost | yes | 0 | **1354** (1345-1357) (timer) | 1659 (1653-1680) (timer) | 31 (observer) | 2002 (1990-2109) (15) | 1113 (981-1792) / 550 (545-558) | 1113 (981-1792) / 550 (546-558) | 138 | 39.2 | 0 | 380662 (380347-384022) |
| bbc.com/news | default | yes | **1** (timer) | 500 (killed) | 1453 (1446-1459) (observer) | 23 (observer) | 1991 (1968-1999) (16) | 948 (915-1697) / 551 (513-553) | 949 (915-1698) / 552 (513-553) | 129 | 35.6 | 1 (the kill) | 380662 |

"loads?" = the HTML, CSS and scripts arrived, the scripts ran to `load`, no challenge. The
harness is headless: whether the content is DRAWN is not part of this table.

Whole-run totals (median of 3; the JS thread's CPU over the run = scripts + `--seconds 10`
after load, or less when the page goes idle):

| page | pass | callbacks | raw CPU total | budget clock total | forced layout share | compile CPU | fetch wall (HTML+CSS+scripts) |
|---|---|---|---|---|---|---|---|
| YouTube results | true cost | 4094 | 21275 | 5661 | 73% | 1048 | 2017 |
| YouTube results | default | 4090 | 23012 | 5891 | 74% | 1035 | 2113 |
| YouTube home | true cost | 6525 | 7809 | 3658 | 54% | 1278 | 1899 |
| YouTube home | default | 6355 | 7588 | 3648 | 52% | 1255 | 1688 |
| en.wikipedia Rust | true cost | 187 | 11285 | 798 | 93% | 127 | 205 |
| en.wikipedia Rust | default | 189 | 12828 | 823 | 93% | 126 | 207 |
| social.loupax.com/discover | true cost | 12 | 181 | 61 | 67% | 1 | 124 |
| github.com/Loupax/vibe-it | true cost | 186 | 434 | 434 | 0% | 0 (modules: inside `module_script`) | 706 |
| old.reddit.com | true cost | 2434 | 640 | 565 | 12% | 244 | 324 |
| bbc.com/news | true cost | 1427 | 4879 | 2872 | 41% | 473 | 579 |
| bbc.com/news | default | 912 | 3092 | 1097 | 64% | 483 | 442 |

(social.loupax.com, github and reddit default rows are within 2% of their true-cost rows.)

Variance: small. Across the 3 runs every CPU number (budget clock, raw CPU, compile, forced
layout per layout) is within ~5% except the forced-layout TOTALS (YouTube results 14.6-17.3 s,
Wikipedia 10.4-11.9 s: the number of forced layouts in the window changes by one). Wall times
vary more, mainly the first run of a page (cold wire/remote caches: YouTube results DCL 5.2 s
vs 4.5 s, bbc 1.8 s vs 1.0 s, github 1.8 s vs 1.1 s). Page bytes: YouTube results
1630651-1632003, YouTube home 1017777-1019591, bbc 380347-384022, reddit 340315-340325, the
others identical in all 6 runs. Run wall time (from file times): YouTube results 24-31 s,
YouTube home 15-18 s, Wikipedia 12-14 s, reddit 10.5 s, bbc 11-12 s, github 2.4-3.1 s,
social.loupax.com 0.4-0.6 s. The last two stop early because the pump loop in
`perf_report` ends when no timer is pending (the harness's network is synchronous, so nothing
else can still arrive); the page has gone idle, which is not a harness fault.

Corpus (QuickJS; `--perf-corpus`, 3 suite runs + 1 standalone `tools/perf-corpus.sh`; median of
the 4, range in brackets; ms of CPU; all sha256 matched, no `error`):

| script | mode | bytes | parse/compile CPU | first run CPU |
|---|---|---|---|---|
| moment 2.31.0 | bare | 60887 | 5.59 (5.56-5.60) | 0.52 (0.51-0.52) |
| d3 7.9.2 | bare | 280622 | 37.6 (35.1-40.9) | 3.22 (3.15-3.48) |
| lodash 4.18.1 | bare | 73234 | 9.26 (9.21-9.40) | 1.60 (1.59-1.62) |
| three 0.159.0 | bare | 668024 | 59.5 (59.1-59.6) | 3.67 (3.64-3.73) |
| jquery 4.1.0 | dom prelude | 78965 | 8.71 (8.66-8.78) | 0.97 (0.95-0.99) |
| react 18.3.1 | bare | 10751 | 1.16 | 0.05 |
| react-dom 18.3.1 | dom prelude (after react) | 131835 | 13.04 (13.01-13.12) | 1.76 (1.74-1.79) |
| fib(25) | bare | 91 | 0.04 | 21.4 (21.3-21.4) |
| sort 200k objects | bare | 208 | 0.05 | 613 (610-620) |
| JSON round trip | bare | 235 | 0.08 | 328 (325-334) |

Optional reference (Chrome normal and `--js-flags=--jitless`): **not taken**. No Chrome or
Chromium is installed on this machine (`which google-chrome chromium chrome chromium-browser
google-chrome-stable` finds nothing) and nothing was installed for it. So there is no "how much
of this is the interpreter" number from another engine; QV8-m's V8 run is the first one.

### What this says (only from the numbers above)

- **Unusable at the default budget on QuickJS: YouTube results and bbc.com/news**, each in all
  3 runs, by ONE callback: YouTube results has a promise job that needs 514-520 ms of budget
  clock (just over 500; killed at 502, so the default run loses that job's work), bbc has a
  timer that needs 1345-1357 ms (2.7x the budget). Both are script time in the budget clock,
  i.e. interpreter work plus native DOM calls made from JS (the harness cannot split those two),
  not forced layout.
- **Close to the line:** YouTube home's largest page script needs 412-424 ms of budget clock
  (raw 941-965 ms for its largest promise job); no kill, but only ~80 ms of margin on this
  CPU. YouTube's compile alone is 1.0-1.3 s CPU per load, before any page code runs.
- **Forced layout dominates the JS thread's total CPU on the heavy pages:** YouTube results
  15.6-17 s of ~21-23 s (73%), Wikipedia 10.5-12 s of 11-13 s (93%, ~1.5 s per layout of a
  1 MB article, single observer deliveries of 2.9 s raw), bbc 2.0 s, YouTube home 4.2 s. It is
  outside the budget (never causes a kill) but it is main-thread stall: single callbacks of
  1.4 s (YouTube results), 2.9 s (Wikipedia) and 1.7 s (bbc) raw CPU. A faster JS engine does
  not change this part; layout does.
- **Native cost vs interpreter cost:** the offline corpus is all interpreter + parser:
  fib(25) 21 ms, 200k object sort 613 ms, JSON round trip 328 ms; parsing three.min.js 60 ms.
  These are the numbers QV8-m compares directly.
- **Fine on QuickJS:** social.loupax.com (max callback 51 ms), github (179 ms, one module),
  Reddit as served (248 ms), Wikipedia's script part (max 193-214 ms budget clock).
- Memory: YouTube pages hold 77-99 MB of QuickJS heap and 394-424 MB RSS at the end.

### What the harness could NOT measure

| not measured | why |
|---|---|
| whether a page DRAWS its content (and how fast) | headless; no paint, no window layout, no screenshot. "loads?" only means scripts ran to `load` |
| layout and paint outside forced layouts | the window's own relayout/paint per turn is not in `--perf-report` |
| images | not fetched by the harness (by design), so image-driven work (QY56 late images) is absent |
| interpreter vs native DOM time inside the budget clock | the budget clock counts both; no per-function or per-native profile |
| GC time | not separated from script time |
| p95 over all callbacks | the report has p95 per kind only; the table gives the highest per-kind p95 |
| which page function a killed callback ran | no attribution in the report (R37 used temporary probes for that) |
| network-realistic wall times | the harness's network is synchronous (each request blocks the JS thread until done), so wall times are not the window's |
| work after the page goes idle or beyond `--seconds 10` | the pump stops at 10 s after load or when no timer is pending |
| Chrome / jitless reference | Chrome not installed (see above) |
| old.reddit.com's old listing | the server sent the new Reddit app to this client |

## QuickJS vs V8

### QV8-m: final comparison at 8dfe2f9 (both engines, same checkout, 2026-11-18 22:38-22:55 CET)

Taken by the QV8-m measurement step (Opus).

- **Binaries:** main 8dfe2f9 (PR #302, QV8-5), clean tree, rustc 1.96.1, both built from the same
  checkout with `nice -n 19 ... -j2` into separate target dirs BEFORE any measurement (builds finished
  22:38; nothing was built while measuring):
  V8 = `cargo build --release` (default = `engine-v8` + `media`), reports `"engine":"V8"`,
  `"engine_version":"15.2.124.1-rusty (v8 crate 152.2.0)"`, `"commit":"8dfe2f9392"`;
  QuickJS = `CARGO_TARGET_DIR=target/qjs cargo build --release --no-default-features --features
  engine-quickjs,media`, reports `"QuickJS"`, `"0.16.2 (rquickjs 0.14.0)"`, `"8dfe2f9392"`. No `-dirty`.
  Note: both builds have `media` (the QV8-0 baseline did not), so Wikipedia's `not implemented: media`
  error is gone on both.
- **Machine:** the same Ryzen 5 PRO 2500U (8 threads), schedutil. Measurements un-niced (as QV8-0). No
  other agent, build or test ran during the measurement (the last other commit on main, 4228de2, landed
  22:24). Firefox was open and idle (~1% CPU); a user scratchy that was busy at 22:15 had exited by 22:38.
- **Load average (1/5/15 min):** start 22:38:20 `1.64, 2.01, 1.80` (the tail of the two release builds,
  decaying), 22:44 1.39, 22:49 0.96, page runs end 22:53:22 `0.60, 0.82, 1.21`, corpus 22:53:23-22:53:29
  `0.60-0.63`. Highest 1-min value seen in the run 1.76 (includes the measurement itself, one busy core).
  Higher than the baseline's 0.3-0.5 in the first ~10 min; the interleaving below puts both engines in the
  same conditions, which is why the QuickJS column is re-measured here instead of reusing 2437284's.
- **Commands:** the exact `tools/perf-suite.sh` per-run invocation (`--perf-report --socket
  /run/user/1001/wire.sock [--budget-ms 20000] --seconds 10 <url>`, `timeout 250`, same 7 pages in the same
  order, RUNS=3, both budgets) from a scratchpad driver that runs every (page, budget, run) on BOTH binaries
  back to back, alternating which goes first, and keeps all 84 per-run JSON files. Corpus: the pinned
  files fetched by `FETCH_ONLY=1 tools/perf-corpus.sh` (all 7 sha256 match the table in the script), then
  `--perf-corpus` 3 times per engine interleaved (the suite's corpus runs) plus one standalone
  `SCRATCHY=<bin> tools/perf-corpus.sh` per engine = 4 runs per engine, as QV8-0. Raw files not committed.
- **No page answered with a bot/JS challenge** (`challenge:false`, `outcome:"ok"` in all 84 runs).
  old.reddit.com again served the new Reddit app (`Welcome to Reddit`).
- Each cell: median of 3 runs, (min-max) when they differ. The CPU still bounces between a fast and a
  slow state; with medians of interleaved runs this shows up only as the ranges.

#### Per page

"largest callback" = the largest single callback in the BUDGET clock that the 500 ms callback budget
applies to (timer, promise job, observer delivery, event, network callback; kind in brackets); the
largest classic/module page script is listed separately because a page-load script has its own 5 s
budget (never killed here). Ratio = V8 / QuickJS (below 1 = V8 cheaper). ms of CPU unless noted.

| page | pass | kills QJS -> V8 | largest callback, budget clock QJS -> V8 | largest page script QJS -> V8 | load wall QJS -> V8 | load CPU QJS -> V8 (ratio) | forced layout total QJS -> V8 (count) | JS errors QJS / V8 | RSS MB QJS -> V8 |
|---|---|---|---|---|---|---|---|---|---|
| YouTube results | true cost | 0 -> 0 | 647 (623-655) promise -> 446 (440-447) promise | 420 -> 488 (471-515) | 4324 -> 3834 | 2629 -> 2035 (0.77) | 14091 (24) -> 14561 (25) | 0 / 0 | 463 -> 485 |
| YouTube results | default | **1 (promise job) -> 0** | 501 killed -> 454 (453-460) promise | 400 -> 475 (469-476) | 3867 -> 3614 | 2591 -> 2005 (0.77) | 4111 (18) -> 14569 (23) (QJS lost the killed job's work) | 1 / 0 | 421 -> 483 |
| YouTube home | true cost | 0 -> 0 | 322 promise -> 143 timer | 430 -> 497 (491-497) | 3704 -> 3456 | 2740 -> 2210 (0.81) | 3974 (18) -> 3937 (19) | 0 / 0 | 399 -> 470 |
| YouTube home | default | 0 -> 0 | 342 promise -> 149 observer | 422 -> 497 (493-507) | 3819 -> 3223 | 2752 -> 2236 (0.81) | 4122 (19) -> 3808 (18) | 0 / 0 | 400 -> 467 |
| en.wikipedia Rust | true cost | 0 -> 0 | 198 timer -> 177 timer | - | 249 -> 232 | 157 -> 141 (0.90) | 12137 (8) -> 10230 (7) | 0 / 0 | 220 -> 264 |
| en.wikipedia Rust | default | 0 -> 0 | 205 timer -> 176 timer | - | 266 -> 249 | 172 -> 150 (0.87) | 11986 (8) -> 11189 (8) | 0 / 0 | 219 -> 265 |
| social.loupax.com/discover | true cost | 0 -> 0 | 50 promise -> <40 | 39 (V8) | 427 -> 433 | 252 -> 220 (0.87) | 126 (10) -> 129 (10) | 0 / 0 | 51 -> 79 |
| social.loupax.com/discover | default | 0 -> 0 | 51 promise -> <40 | 54 (V8) | 409 -> 390 | 252 -> 240 (0.95) | 125 (10) -> 124 (10) | 0 / 0 | 51 -> 77 |
| github.com/Loupax/vibe-it | true cost | 0 -> 0 | < 40 both | module 181 -> 118 | 954 -> 727 | 525 -> 291 (0.55) | 0 -> 0 | 4 / 4 | 51 -> 66 |
| github.com/Loupax/vibe-it | default | 0 -> 0 | < 40 both | module 187 -> 117 | 893 -> 741 | 530 -> 292 (0.55) | 0 -> 0 | 4 / 4 | 51 -> 67 |
| old.reddit.com (new Reddit) | true cost | 0 -> 0 | 215 promise -> 113 promise | 90 -> 102 | 392 -> 364 | 87 -> 51 (0.59) | 75 (1) -> 71 (1) | 8 / 8 | 126 -> 152 |
| old.reddit.com (new Reddit) | default | 0 -> 0 | 215 promise -> 92 promise | 89 -> 103 | 698 (378-902) -> 455 | 87 -> 51 (0.59) | 74 (1) -> 74 (1) | 8 / 8 | 126 -> 151 |
| bbc.com/news | true cost | 0 -> 0 | **1345 (1335-1353) timer -> 535 (520-536) timer** (0.40) | - | 777 -> 1401 (512-2087, network: fetch wall 272 -> 1121) | 526 -> 297 (0.56) | 2054 (15) -> 2113 (16) | 0 / 0 | 153 -> 205 |
| bbc.com/news | default | **1 (timer) -> 1 (timer)** | 500 killed -> 501 killed | - | 746 -> 538 | 525 -> 311 (0.59) | 2039 (16) -> 2103 (16) | 1 / 1 (the kill) | 144 -> 196 |

Whole-run totals (all callbacks; median of 3):

| page | pass | budget clock total QJS -> V8 | raw CPU total QJS -> V8 | forced layout share QJS -> V8 | compile CPU QJS -> V8 | engine startup QJS -> V8 |
|---|---|---|---|---|---|---|
| YouTube results | true cost | 8395 -> 6242 (0.74) | 22522 -> 20803 (0.92) | 63% -> 70% | 1033 -> 386 (0.37) | 32 -> 5.5 |
| YouTube results | default | 4676 -> 6170 (QJS killed work) | 8764 -> 20739 | 47% -> 70% | 1369 -> 379 | 32 -> 5.6 |
| YouTube home | true cost | 3496 -> 2380 (0.68) | 7382 -> 6316 (0.86) | 53% -> 63% | 1274 -> 495 (0.39) | 32 -> 5.6 |
| YouTube home | default | 3705 -> 2261 (0.61) | 8041 -> 6069 (0.75) | 52% -> 63% | 1286 -> 497 (0.39) | 32 -> 5.5 |
| en.wikipedia Rust | true cost | 835 -> 748 (0.90) | 12985 -> 10964 (0.84, one layout fewer) | 93% -> 93% | 110 -> 47 (0.43) | 51 -> 25 |
| en.wikipedia Rust | default | 823 -> 765 (0.93) | 12809 -> 11954 (0.93) | 94% -> 94% | 107 -> 43 | 65 -> 38 |
| social.loupax.com | true cost | 84 -> 77 | 209 -> 205 | 60% -> 63% | 0.9 -> 0.7 | 37 -> 8.2 |
| github | true cost | 442 -> 248 (0.56) | 442 -> 248 (0.56) | 0% | (modules) | 35 -> 8.5 |
| old.reddit.com | true cost | 601 -> 534 (0.89) | 677 -> 605 (0.89) | 11% -> 12% | 219 -> 65 (0.30) | 63 -> 12 |
| bbc.com/news | true cost | 2897 -> 1449 (0.50) | 4949 -> 3546 (0.72) | 41% -> 59% | 475 -> 124 (0.26) | 36 -> 8.3 |
| bbc.com/news | default | 1128 -> 1116 (both killed) | 3177 -> 3214 | 64% -> 65% | 487 -> 118 | 37 -> 8.0 |

Callback COUNTS are not comparable between the engines: V8 reports far fewer `promise_job`s (YouTube
results 447 vs 5149, reddit 169 vs 2365) because its promise hook counts jobs differently from QuickJS'
one-job-at-a-time loop (QV8-3b); totals and maxima are comparable, counts are not.

QuickJS today vs its own 2437284 baseline: same shape, but the YouTube results promise job grew from
514-520 to 623-655 ms (page drift and/or the QV8 seam; not separated), still killed at 500.

#### Corpus (`--perf-corpus`, 4 runs per engine, identical bytes by sha256, no `error` on either)

`--perf-corpus` has no budget (both "budgets" are the same run). ms of CPU, median (range).

| script | mode | bytes | compile QJS -> V8 | first run QJS -> V8 | compile + first run, ratio |
|---|---|---|---|---|---|
| moment 2.31.0 | bare | 60887 | 5.61 -> 1.59 | 0.51 -> 2.05 | 6.12 -> 3.64 (0.59) |
| d3 7.9.2 | bare | 280622 | 36.6 (34.6-48.1) -> 12.5 | 3.17 -> 10.8 | 39.8 -> 23.3 (0.59) |
| lodash 4.18.1 | bare | 73234 | 9.35 -> 2.64 | 1.59 -> 6.46 | 10.9 -> 9.10 (0.83) |
| three 0.159.0 | bare | 668024 | 59.5 -> 15.4 | 3.67 -> 5.81 | 63.2 -> 21.2 (0.34) |
| jquery 4.1.0 | dom prelude | 78965 | 8.81 -> 2.22 | 0.96 -> 5.45 | 9.77 -> 7.67 (0.79) |
| react 18.3.1 | bare | 10751 | 1.18 -> 0.33 | 0.05 -> 0.32 | 1.23 -> 0.65 (0.53) |
| react-dom 18.3.1 | dom prelude | 131835 | 13.2 -> 3.53 | 1.75 -> 3.92 | 14.9 -> 7.45 (0.50) |
| all 7 libraries | | | 134.3 -> 38.2 (0.28) | 11.7 -> 34.8 (2.97) | 146.0 -> 73.0 (**0.50**) |
| fib(25) | bare | 91 | 0.04 -> 0.03 | **21.4 -> 1.56 (13.7x faster)** | |
| sort 200k objects | bare | 208 | 0.05 | **614 -> 103 (6.0x)** | |
| JSON round trip | bare | 235 | 0.08 -> 0.06 | **329 -> 47.4 (6.9x)** | |

"Steady state": the harness runs each library once (compile + first run); there is no repeated-run
steady-state mode. The three CPU loops are the steady-state proxy (they run long enough for V8's JIT
tiers): 6-14x faster. V8 compiles lazily, so a library's first run costs MORE than on QuickJS (it
compiles the functions it calls then); compile + first run together is 2x cheaper.

#### QV8-4 snapshot effect on startup (V8 build, `SCRATCHY_V8_SNAPSHOT=0` vs default)

`engine_startup_cpu_ms` (isolate + context + natives + preludes) in the suite: V8 5.4-13.7 ms per page
(Wikipedia 25-41) vs QuickJS 32-70 ms. Direct A/B on the V8 binary, `--seconds 0`, 3 interleaved runs,
22:54 (load 0.33-1.0): Wikipedia 25.7 / 25.8 / 47.4 with snapshot vs 46.6 / 52.1 / 68.5 without; bbc
7.5 / 8.3 / 8.6 vs 53.3 / 29.0 / 28.8. So the snapshot saves ~20-45 ms per page start (as QV8-4
measured) and makes V8's startup 4-6x cheaper than QuickJS' on most pages. Why Wikipedia's startup is
25 ms with the snapshot when other pages take 8 ms was NOT investigated. RSS with/without the snapshot:
within 3 MB.

#### What is still slow, and why

1. **Forced layout, unchanged by the engine (as predicted).** Share of the JS thread's CPU on V8:
   Wikipedia 93-94%, YouTube results 70%, YouTube home 63%, bbc 59-65%. Single layouts of 1.1-1.5 s
   (YouTube results) and 1.56-1.75 s (Wikipedia); single callbacks of 1.6-1.9 s raw (YouTube results
   observer delivery), 2.7-2.9 s raw (Wikipedia observer delivery), 1.56 s (bbc observer delivery).
   These are outside the budget clock, so they never kill, but they are main-thread stalls 3-6x longer
   than the 500 ms budget. Because V8 made script cheaper, forced layout's share went UP on every heavy
   page. This is QY53's list from R37: our own `obsFlush` observer-delivery timer running the page's
   IntersectionObserver/ResizeObserver callbacks that read geometry after DOM writes; `__d_rect`
   (getBoundingClientRect, 3 calls = 3.9 s in one job in R37); a full relayout per write generation
   (does one callback's series of reads reuse one layout?). Natives in the budget clock from R37:
   `__d_cs` (fixed by QY52), `__d_special` (fixed by QY51), `__d_parent` (24k-87k calls per callback
   through `isConnected`, the `moAttr` parent walk for every style write, the Shady DOM polyfill), the
   Shady DOM `insertBefore`. These natives are Rust, so V8 does not make them faster; the harness cannot
   split native from JS time inside the budget clock.
2. **YouTube's big page scripts got slightly SLOWER on V8** in the budget clock: largest classic script
   420 -> 488 (YouTube results), 430 -> 497 (YouTube home). They run under the 5 s page-load budget, so
   they are not killed, but each is a ~0.5 s stall. Not attributed (no profiler in the harness): likely
   candidates are lazy compilation of a 1 MB+ script's functions on first call (the corpus shows the same
   first-run increase) and per-call native overhead through the V8 binding; this needs a profile.
3. **YouTube results' worst promise job is 446-460 ms on V8**: no kill now, but only 40-55 ms (8-11%)
   under the 500 ms budget on this CPU; a slower machine, the slow CPU state or more page code tips it.
4. **bbc.com/news still loses one timer at the default budget on V8** in 3/3 runs: it needs 520-536 ms
   of budget clock (was 1335-1353 on QuickJS, 2.5x cheaper, but still 4-7% over the line).

#### Memory

RSS at the end, V8 vs QuickJS: +15 to +70 MB per page (YouTube results +22/+62, YouTube home +68-71,
Wikipedia +44-46, social.loupax.com +26-28, github +15-16, reddit +25-26, bbc +52). JS heap at the end
is similar (YouTube 96-105 MB on both; Wikipedia 12.5 -> 20 MB; bbc 31-38 MB on both). GC time is not
separated from script time by the harness (not measured).

#### Recommendation for QY49-D (QUESTIONS.md Q1), from these numbers

**(c): a larger budget only while the page is loading, 2 s, until `load` + 2 s; 500 ms afterwards.**
- (a) alone is not enough: on V8 bbc still loses a timer (needs 520-536 ms) and YouTube results' worst
  job sits 8-11% under the line. The harness only covers navigation + 10 s after `load`, so every kill
  in the 84 runs is inside that window (the exact second of bbc's timer was not recorded); R37 found
  every trip it measured during the load too.
- 2 s (not 5 s) is enough: the largest single budget-clock callback measured on V8 is 536 ms, so 2 s is
  a ~4x margin; on QuickJS it would have been 1.35 s (still under 2 s), so the same value also covers the
  fallback build while it exists.
- (b) (2 s always) is not supported by any measurement: no trip has been seen after a page finished
  loading (but the harness does not watch long-lived pages or interaction, so this is unmeasured).
- (d) only if QY54 shows the compositor disconnects scratchy during these stalls. Note the R8 premise has
  weakened: forced layout (excluded from the budget) already stalls 1.5-2.9 s in one callback on V8, so
  the 500 ms budget is not what bounds stalls today; QY53 (forced layout) is the bigger lever for
  responsiveness than any budget value.
Caveat: whether bbc's timer runs before `load` + 2 s was not recorded (the kill log line has no time);
whoever implements (c) must check it on bbc and widen the window (e.g. `load` + 5 s) if it falls later.
This stays a user decision (it changes the R8 protection); the numbers no longer require (c), but
without it bbc keeps losing a timer and YouTube results is one slow day from losing its results again.

#### What QV8-m did NOT measure

| not measured | why |
|---|---|
| drawn output (whether the page's content appears) | headless harness; no nested-dwl screenshots in this step |
| interpreter vs native time in the budget clock; which function is slow in YouTube's 0.5 s scripts / bbc's timer | no profiler in the harness, no probes added (docs-only step) |
| GC time | not separated by the harness |
| steady-state runs of the corpus libraries | the harness has compile + first run only; CPU loops are the proxy |
| Chrome / jitless reference | Chrome not installed (as in QV8-0) |
| why V8 counts fewer promise jobs; why Wikipedia's V8 startup is 25 ms | not investigated |
| R8 disconnect under stall (QY54) | not in scope |
| DuckDuckGo | not in the suite (bot challenge, R37) |
| the baseline's quiet-machine conditions in the first ~10 min | load 1.0-1.76 while the build load decayed; mitigated by interleaving both engines |

### QV8-3b interim (V8 15.2.124.1 behind `engine-v8`, 103ff22 + QV8-3b, 2026-11-18)

Interim, NOT the final table (QV8-m writes that). `tools/perf-suite.sh` with
`SCRATCHY=target/v8/release/scratchy` (release, 7 pages x 3 runs x 2 budgets + corpus), run on a quiet
machine (load ~1.3 at the end, nothing else started by me), un-niced, same wire and pages as the
baseline. Modules are NOT implemented yet on V8 (QV8-3c): github.com and old.reddit.com use module
scripts and ran only 1 ms / 26 ms of callbacks (js errors 6 and 6 vs 4 and 8 on QuickJS; the individual
errors were not inspected), so their rows are not comparable.
Median of 3; ms of CPU unless noted.

| page | pass | kills (QuickJS) | max callback budget clock (QuickJS) | all callbacks raw CPU total (QuickJS) | forced layout total | load CPU | RSS MB | JS errors |
|---|---|---|---|---|---|---|---|---|
| YouTube results | true cost | 0 | 494 (515) | 23100 | 18555 | 2056 | 474 | 0 |
| YouTube results | default | 0 (1) | 554 (502 killed) | 22325 | 17958 | 2413 | 465 | 0 (1) |
| YouTube home | true cost | 0 | 499 (412) | 6725 | 4359 | 2229 | 469 | 0 |
| en.wikipedia Rust | default | 0 | 230 (195) | 12561 | 11683 | 165 | 240 | 1 (media) |
| bbc.com/news | true cost | 0 | 558 (1354) | 3705 | 2592 | 453 | 173 | 3 |
| bbc.com/news | default | 0 (1) | 395 (500 killed) | 2894 | 2061 | 310 | 174 | 3 (1) |

(Other rows are in the raw JSON, not committed.) Reading: the script-only part got cheaper where there is
no forced layout (bbc timer max 1354 -> 558, no kill at the default budget), but YouTube and Wikipedia are
dominated by forced layout (YouTube results 18.0 s of the 22 s), which the engine does not change, so
total CPU and load CPU did not improve on those pages. Corpus (bare engine; compile / first run CPU ms,
QuickJS in brackets): d3 13.1 / 10.4 (37.6 / 3.22), jquery 2.2 / 5.6 (8.71 / 0.97), lodash 2.6 / 6.4
(9.26 / 1.60), moment 1.6 / 2.0 (5.59 / 0.52), react 0.33 / 0.32 (1.16 / 0.05), react-dom 3.5 / 3.9
(13.04 / 1.76), three 15.2 / 5.8 (59.5 / 3.67), fib(25) 0.03 / 1.5 (0.04 / 21.4), sort 200k objects
0.05 / 99 (0.05 / 613), JSON round trip 0.06 / 47 (0.08 / 328). So parsing and CPU loops are 3-14x cheaper
(V8 compiles lazily, so first run of big libraries costs more than QuickJS' 0.5-3.7 ms), as in R39.
Single run per row for the corpus numbers here (the suite's median of 3). Not measured: the standalone
corpus run, Chrome reference, drawn output.

### QV8-4: V8 startup snapshot (V8 15.2.124.1 behind `engine-v8`, 2026-11-18)

What: a page engine's context is no longer built by running the ~198 KB of prelude JS (PRELUDE, timers,
url, globals, intl, fetch, websocket, DOM, cssom, media) after registering ~180 natives; it is
deserialized from a startup snapshot built IN-PROCESS by the same startup code over an empty
document (`js::bake_engine`, `js_engine_v8.rs` `Snapshot`), on a background thread
(`js_engine::warm_up`, started at app start; an engine made before it is ready starts the old way).
Nothing is stored on disk, so the snapshot cannot go stale (it is built from the very sources of the
running binary). The default (QuickJS) build is unchanged (no-op stubs in js_engine.rs).

Measured with `--perf-report --seconds 0 --socket wire.sock <local copy with <base>>`, the
`engine_startup_cpu_ms` field (engine_start -> engine_ready: isolate, context, natives, all preludes;
thread CPU), 5 runs per page, before and after interleaved in one loop, same machine, load average
1.2-1.5 at the start and end (nothing else of mine running; the machine bounces between a fast and a slow
CPU state, so the runs are bimodal: both modes are shown). ms:

| page | before (runs; median) | after (runs; median) |
|---|---|---|
| wikipedia.org | 30.3 31.8 47.5 52.7 52.7; 47.5 | 7.6 7.7 13.3 14.0 14.5; 13.3 |
| en.wikipedia Main_Page | 29.8 30.3 53.5 53.7 55.0; 53.5 | 8.9 12.6 13.0 13.0 13.3; 13.0 |
| news.ycombinator.com | 30.0 54.0 58.1 58.9 59.0; 58.1 | 13.3 13.4 13.6 13.8 13.8; 13.6 |

Gain: 22 ms (fast CPU state, 30 -> 8) to 40 ms (slow state, 53 -> 13) per page start. The whole page
`load_cpu_ms` median went 79 / 79 / 63 -> 64 / 52 / 21 (it includes the page's own scripts, so it is noisier).
Before, split (fast state, en.wikipedia): isolate+context+BASE64 5.3, PRELUDE 4.4, timers 0.2, url 0.6,
globals 1.4, intl 3.1, fetch 2.9, ws 0.4, DOM natives+DOM prelude 9.1, cssom 0.9, media 1.0.
Upper bound from a standalone experiment (stub natives, same preludes): 20.6 ms without snapshot,
4.9 ms from a 1.5 MB snapshot (empty isolate: 2.3). Costs: the snapshot is built once per PROCESS,
~60 ms of CPU on its own thread (`--js-report` of a one-line page: 104 ms wall with it, 45 ms with
`SCRATCHY_V8_SNAPSHOT=0`; the one-shot report modes build it first, so they pay it); the blob lives in
memory only. Decision: KEPT (>= 10 ms per page start).
Not measured: RSS (a snapshot-made isolate shares nothing with the next one), page load on slow disks
(there is no disk), many tabs.

## Offline benchmarks

### QOBS10: forced layout on the "big-css" fixture (`cargo bench --bench layout -- forced_layout`)

Offline and deterministic input (the fixture is generated in `benches/layout.rs`, never vendored; no network, no
script engine): 20 000 class rules (`.c<i> .d<i> { margin-left: 1px }`), 200 `::before` rules, 1 000 nested divs with
3 classes each. One iteration = write one attribute (`data-w` on the innermost div), then
`build_layout_tree_sized` (a thin wrapper of `build_layout_tree_with_resources`) + `measure_document`, i.e. what
`write attribute; read offsetWidth` costs. 1 000 nested divs run on the default criterion harness (no big-stack thread since QSTK1; same element and rule counts as the QY53 fixture).
Variants: `big_css`; `200 custom props + overrides` (200 `--p<i>` on `:root`, an inline override on every 10th div,
`var()` use; QY53i); `100 long text paragraphs` (100 x ~2 000 chars appended; QP4); `small <style> appended`
(a different small `<style>` appended to `<head>` before every build, so the stylesheet source changes and the
rule cache misses; the stylesheet re-parse cost QY53f1 will remove). CI only compiles it (`--all-targets` already
builds benches); no bench timing in CI.

Measured on main + this change (Ryzen 7 5700X, 16 threads, V8 build, release, 10 samples, 1 s warm-up, 5 s
measurement), while other agents were building (load average ~11.5, so spread is wide; criterion
[low median high]):

| variant | time per forced layout |
|---|---|
| big_css | [25.0 ms 28.6 ms 35.4 ms] |
| 200 custom props + overrides | [75.5 ms 100.8 ms 144.4 ms] |
| 100 long text paragraphs | [33.5 ms 37.7 ms 48.2 ms] |
| small `<style>` appended (re-parse) | [24.2 ms 25.4 ms 27.3 ms] |

Caveat: the appended-style variant is not measurably slower than plain big_css here (the fixture's CSS is ~0.6 MB,
much smaller than YouTube's 4.4 MB, and the run was noisy); re-take the table on an idle machine (QOBS9).
## Frame stats (QOBS5): what the live window does each second

`SCRATCHY_FRAME_STATS=<path>` (or `=1` for stderr) makes the window App append one JSON line per second of
wall time. Unset, empty or `0` = off: no file, no timing, no perf recorder (zero cost). On, the perf recorder
also runs in a window mode that keeps only running totals (no per-span samples).

    SCRATCHY_FRAME_STATS=/tmp/frames.jsonl scratchy --socket "$(wire --print-socket)" https://example.com/
    tail -f /tmp/frames.jsonl

Fields (all counts and ms are for that second; `dt_ms` is the real length of the interval):
- `t` seconds since start; `redraws` full redraws; `damage_presents` video-rect-only presents;
  `gap_max_ms` the longest gap between two redraw starts (carries across seconds).
- `builds` {n, ms}: App tree builds (DOM walk + cascade, main thread). `shape` {complete, incomplete, worker_ms}:
  shape-worker results received and the worker's wall time spent shaping.
- `paint_ms` fill + paint up to the present; `present_ms` `present()` / `present_with_damage()` (may include
  waiting for the compositor).
- `js_ms` page JS CPU ms (exclusive span time of scripts, timers, jobs, events, observers, net callbacks);
  `forced` {n, ms} forced layouts inside that JS.
- `forced.by_caller` (QOBS5c): the top 3 forced-layout callers of the second, `{caller, n, ms}`, slowest CPU first (same `function url:line:col` names as `--perf-report` `by_caller`); `[]` when there were none. `longest` {ms, kind, source} or `null`: the callback (script, timer, job, event, observer, net) with the most exclusive CPU that second and its source (same naming as `--perf-report` `longest`).
- `hover_restyles` hover changes that caused a rebuild.
- `net` {n, p50_ms, p95_ms, max_ms} (QOBS6): page network results delivered to JS this second and how long each waited between its transport finishing (the thread that ran the request sends `AppEvent::JsNet`, stamped then) and `deliver_net` running it on the main thread. Large values = the result sat in the event queue behind other main-thread work (QM1k1n5).
- `input` {move, button, key, wheel}: events received; `lat_max_ms` / `lat_p95_ms`: input event to the next
  present (any present); `pending` / `oldest_pending_ms`: inputs still without a present at the roll-up (an
  event that causes no redraw stays pending, so a large number is "no frame came", the freeze signature).
- `video` {wakes, v:[{node, rendered, painted}]}: `wakes` = mpv "frame ready" wakeups; per `<video>`: new frames
  rendered from mpv and presents that included the video (a full redraw repaints the newest frame, which may
  be the same one: painted can exceed rendered).

Overhead: see the QOBS5 PR (measured with and without the variable).

Idle floor (QOBS5b; fixed by QM1k1n6, #347, found independently here): an idle window must show `js_ms` ~0 and ~0 % CPU. It used to show ~125 ms `js_ms` per
second and ~98 % of a core on any page with a pending JS timer (Wikipedia, YouTube). Not a page timer and not a
recorder artifact: `App::pump_net` cloned the winit `EventLoopProxy` on every `about_to_wait` and dropped it, and the
drop pings the loop, so with `ControlFlow::WaitUntil` (a timer pending) the loop woke ~11 000 times a second; each wake
ran `run_due` / `next_deadline` evals that drain (empty) promise jobs, which the window recorder counted as ~2 us
`PromiseJob` spans (~125 ms/s of "JS"). Pages without timers use `ControlFlow::Wait`, which winit filters, so they
never showed it. Diagnose a future one with `SCRATCHY_FRAME_STATS` (`js_ms` steady at ~120 with zero redraws =
spinning loop, check CPU in `/proc/<pid>/stat`). After the fix: Wikipedia Rust idle `js_ms` 0.01 per second, CPU 3
ticks per 10 s (was ~980).
