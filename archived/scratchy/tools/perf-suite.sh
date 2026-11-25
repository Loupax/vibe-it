#!/usr/bin/env bash
# QV8-0: the repeatable performance suite (PERF.md has the context).
#
#   tools/perf-suite.sh                 full suite: 7 live pages x 3 runs x 2 budgets + corpus
#   RUNS=1 PAGES="https://en.wikipedia.org/wiki/Rust_(programming_language)" SKIP_CORPUS=1 tools/perf-suite.sh
#
# Environment (all optional):
#   SCRATCHY=path      binary (default target/release/scratchy)
#   WIRE_SOCKET=path   the user's normal wire socket (default /run/user/$UID/wire.sock)
#   RUNS=3             runs per page and mode; the result is the per-field MEDIAN
#   SECONDS_PER_PAGE=10  --seconds of the report (time the page keeps running after load)
#   PAGES="u1 u2 ..."  override the page list (space separated)
#   SKIP_CORPUS=1      skip the offline engine corpus
#   OUT_DIR=perf       where perf/<engine>-<commit>.json goes (gitignored)
#
# Two passes per page: `--budget-ms 20000` ("true cost": nothing is killed) and the
# default budget ("what the user sees"). Pages load through the user's normal wire
# socket with scratchy's normal User-Agent (NO override). The script never tries to
# pass a bot/JS challenge: a page that answers with one is recorded as `challenge`
# and skipped. Run it on a quiet machine; it is slow (about 7 x 3 x 2 x 15 s).
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
SOCK="${WIRE_SOCKET:-/run/user/$(id -u)/wire.sock}"
BIN="${SCRATCHY:-$HERE/target/release/scratchy}"
RUNS="${RUNS:-3}"
SECS="${SECONDS_PER_PAGE:-10}"
OUT_DIR="${OUT_DIR:-$HERE/perf}"
DEFAULT_PAGES="https://www.youtube.com/results?search_query=rust
https://www.youtube.com/
https://en.wikipedia.org/wiki/Rust_(programming_language)
https://social.loupax.com/discover
https://github.com/Loupax/vibe-it
https://old.reddit.com/
https://www.bbc.com/news"
PAGES="${PAGES:-$DEFAULT_PAGES}"

[ -x "$BIN" ] || { echo "perf-suite: no scratchy binary at $BIN (cargo build --release)" >&2; exit 2; }
[ -S "$SOCK" ] || { echo "perf-suite: no wire socket at $SOCK" >&2; exit 2; }
TMP="$(mktemp -d "${TMPDIR:-/tmp}/scratchy-perf.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

n=0
while IFS= read -r url; do
  [ -n "$url" ] || continue
  for mode in true_cost default; do
    flags=()
    [ "$mode" = true_cost ] && flags=(--budget-ms 20000)
    for run in $(seq 1 "$RUNS"); do
      n=$((n + 1))
      f="$TMP/page-$n.json"
      echo "perf-suite: [$mode run $run/$RUNS] $url" >&2
      if timeout $((SECS + 240)) "$BIN" --perf-report --socket "$SOCK" ${flags[@]+"${flags[@]}"} --seconds "$SECS" "$url" > "$f" 2> "$TMP/page-$n.err"; then
        printf '%s\t%s\t%s\n' "$url" "$mode" "$f" >> "$TMP/index.tsv"
      else
        echo "perf-suite:   failed, see $TMP/page-$n.err" >&2
        printf '{"outcome":"run_failed","challenge":false}' > "$f"
        printf '%s\t%s\t%s\n' "$url" "$mode" "$f" >> "$TMP/index.tsv"
      fi
    done
  done
done <<< "$PAGES"

if [ -z "${SKIP_CORPUS:-}" ]; then
  CDIR="$TMP/corpus"
  FETCH_ONLY=1 "$HERE/tools/perf-corpus.sh" "$CDIR" > /dev/null
  for run in $(seq 1 "$RUNS"); do
    echo "perf-suite: [corpus run $run/$RUNS]" >&2
    "$BIN" --perf-corpus "$CDIR" > "$TMP/corpus-$run.json"
  done
fi

mkdir -p "$OUT_DIR"
python3 -I - "$TMP" "$OUT_DIR" "$RUNS" "$SECS" <<'PY'
import json, sys, os, glob, statistics

tmp, out_dir, runs, secs = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]

def med(vals):
    """Per-field median of a list of JSON values of the same shape."""
    first = vals[0]
    if isinstance(first, dict):
        return {k: med([v[k] for v in vals if k in v]) for k in first}
    if isinstance(first, bool) or first is None or isinstance(first, str):
        return first
    if isinstance(first, (int, float)):
        m = statistics.median(vals)
        return round(m, 3) if isinstance(m, float) else m
    if isinstance(first, list):
        if first and isinstance(first[0], dict) and "name" in first[0]:
            names = [e["name"] for e in first]
            return [med([e for r in vals for e in r if e.get("name") == n]) for n in names]
        return first
    return first

pages = {}
meta = {}
for line in open(os.path.join(tmp, "index.tsv")) if os.path.exists(os.path.join(tmp, "index.tsv")) else []:
    url, mode, f = line.rstrip("\n").split("\t")
    try:
        data = json.load(open(f))
    except Exception:
        data = {"outcome": "run_failed", "challenge": False}
    pages.setdefault(url, {}).setdefault(mode, []).append(data)
    if "engine" in data and not meta:
        meta = {k: data[k] for k in ("engine", "engine_version", "commit")}

result_pages = {}
for url, modes in pages.items():
    result_pages[url] = {}
    for mode, datas in modes.items():
        ok = [d for d in datas if d.get("outcome") == "ok"]
        if any(d.get("challenge") for d in datas):
            status = "challenge"
        elif not ok:
            status = datas[0].get("outcome", "run_failed")
        else:
            status = "ok"
        entry = {"status": status, "runs": len(datas), "ok_runs": len(ok)}
        if status == "ok":
            entry["median"] = med(ok)
        result_pages[url][mode] = entry

corpus = None
cfiles = sorted(glob.glob(os.path.join(tmp, "corpus-*.json")))
if cfiles:
    corpus = med([json.load(open(f)) for f in cfiles])
    if not meta:
        meta = {k: corpus[k] for k in ("engine", "engine_version", "commit")}

if not meta:
    sys.exit("perf-suite: nothing was measured")
out = {"schema": 1, **meta, "runs_per_page": runs, "seconds_per_page": float(secs),
       "modes": {"true_cost": "--budget-ms 20000", "default": "default budget"},
       "pages": result_pages, "corpus": corpus}
name = "%s-%s.json" % (meta["engine"].lower(), meta["commit"])
path = os.path.join(out_dir, name)
with open(path, "w") as fh:
    json.dump(out, fh, indent=1)
print(path)
PY
