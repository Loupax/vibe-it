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

## Clocks

- **raw CPU**: CPU time of the JS thread (`CLOCK_THREAD_CPUTIME_ID`).
- **budget clock**: raw CPU minus the time spent in forced layouts (a geometry read after
  a DOM write). This is what the callback budget counts (REGRESSIONS.md R37), so a kill
  happens when the BUDGET clock of one callback passes the budget.
- Spans nest (an observer delivery inside a timer, a script inserted by a network
  callback). Each span records its EXCLUSIVE time, so the totals add up with no double
  counting; a timer that only ran an observer delivery still counts as one timer with ~0.

## JSON field glossary (`--perf-report`)

| field | meaning |
|---|---|
| `schema` | JSON layout version (1) |
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
| `budget_kills` | sum of all `kills` |
| `forced_layout` | `count`, `total_ms`, `max_ms` of forced layouts (raw CPU, the part removed from the budget clock) |
| `compile_cpu_ms` | CPU spent compiling page scripts (a module's compile is inside its `module_script` run) |
| `engine_startup_cpu_ms` | CPU to create the runtime, install the DOM and run the preludes, before the first page script |
| `load.fetch_wall_ms`, `.fetch_cpu_ms` | start to "HTML, CSS and scripts in hand" |
| `load.dom_content_loaded_wall_ms`, `_cpu_ms`, `load.load_wall_ms`, `_cpu_ms` | start to the DOMContentLoaded / load event having run (start = before the HTML request) |
| `js_heap_bytes` | engine heap in use at the end (QuickJS `memory_used_size`) |
| `rss_bytes` | process resident set size at the end |
| `js_errors` | errors logged at level `error` (uncaught exceptions, killed callbacks, failed script loads) |
| `console_errors` | `console.error` calls |

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

Filled in by QV8-m: the same two tables for V8 at the merged hash, side by side with the
baseline above, with the ratio per row.
