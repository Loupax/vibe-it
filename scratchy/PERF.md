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

## Baseline (QuickJS, <drained hash>, <date>)

Numbers are filled in by the QV8-0 measurement step. Machine, load average and the exact
commands go here too.

Per page, two rows (true cost = `--budget-ms 20000`, default = what the user sees); median of 3 runs.

| page | pass | loads? | kills | max callback (budget clock) | p95 callback (budget clock) | forced layout total | time to load (wall) | time to load (CPU) | RSS | JS heap | JS errors |
|---|---|---|---|---|---|---|---|---|---|---|---|
| YouTube results (rust) | true cost | | | | | | | | | | |
| YouTube results (rust) | default | | | | | | | | | | |
| YouTube home | true cost | | | | | | | | | | |
| YouTube home | default | | | | | | | | | | |
| en.wikipedia Rust (programming language) | true cost | | | | | | | | | | |
| en.wikipedia Rust (programming language) | default | | | | | | | | | | |
| social.loupax.com/discover | true cost | | | | | | | | | | |
| social.loupax.com/discover | default | | | | | | | | | | |
| github.com/Loupax/vibe-it | true cost | | | | | | | | | | |
| github.com/Loupax/vibe-it | default | | | | | | | | | | |
| old.reddit.com | true cost | | | | | | | | | | |
| old.reddit.com | default | | | | | | | | | | |
| bbc.com/news | true cost | | | | | | | | | | |
| bbc.com/news | default | | | | | | | | | | |

Corpus (QuickJS):

| script | mode | bytes | parse/compile CPU | first run CPU |
|---|---|---|---|---|
| moment 2.31.0 | bare | | | |
| d3 7.9.2 | bare | | | |
| lodash 4.18.1 | bare | | | |
| three 0.159.0 | bare | | | |
| jquery 4.1.0 | dom prelude | | | |
| react-dom 18.3.1 | dom prelude | | | |
| fib(25) | bare | | | |
| sort 200k objects | bare | | | |
| JSON round trip | bare | | | |

Optional reference (Chrome normal and `--js-flags=--jitless`, CPU of the busiest task from a
trace): not taken by the harness; filled in by hand when Chrome is available.

## QuickJS vs V8

Filled in by QV8-m: the same two tables for V8 at the merged hash, side by side with the
baseline above, with the ratio per row.
