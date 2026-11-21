#!/bin/bash
# QMAPI1: JS-time cost of the report mode. For each page, ROUNDS interleaved pairs of
#   scratchy --perf-report        (normal)
#   scratchy --missing-api-report (report mode)
# through your OWN wire on your OWN socket, each run under the perf lock. Prints per page the summed
# raw_cpu total_ms of all callback kinds (the JS time) and the ratio.
#
# Usage: SCRATCHY=bin WIRE=bin OUT=dir tools/missing-api/cost.sh [ROUNDS] [SECONDS] URL...
set -u
SCRATCHY=${SCRATCHY:-scratchy}
WIRE=${WIRE:-wire}
OUT=${OUT:?set OUT to a scratch dir}
LOCK=${PERF_LOCK-/tmp/scratchy-perf.lock}
ROUNDS=${1:-3}
SECS=${2:-20}
shift 2
mkdir -p "$OUT"
ulimit -c 0
SOCK="${XDG_RUNTIME_DIR:-/tmp}/qmapi1-cost-$$.sock" # short path: sockets are limited to ~108 bytes
rm -f "$SOCK"
"$WIRE" --socket "$SOCK" >"$OUT/wire.log" 2>&1 9>&- &
WPID=$!
cleanup() { if [ -r "/proc/$WPID/cmdline" ]; then kill "$WPID" 2>/dev/null; fi; }
trap cleanup EXIT
for _ in $(seq 50); do [ -S "$SOCK" ] && break; sleep 0.1; done
run() { # MODE URL OUTFILE
  local flag=--perf-report
  [ "$1" = report ] && flag=--missing-api-report
  if [ -n "$LOCK" ]; then
    flock "$LOCK" systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0 "$SCRATCHY" $flag --budget-ms 20000 --socket "$SOCK" --seconds "$SECS" "$2" >"$3" 2>/dev/null
  else
    "$SCRATCHY" $flag --budget-ms 20000 --socket "$SOCK" --seconds "$SECS" "$2" >"$3" 2>/dev/null
  fi
}
i=0
for url in "$@"; do
  i=$((i + 1))
  for r in $(seq "$ROUNDS"); do
    if [ $((r % 2)) -eq 1 ]; then run normal "$url" "$OUT/p$i-n$r.json"; run report "$url" "$OUT/p$i-r$r.json"
    else run report "$url" "$OUT/p$i-r$r.json"; run normal "$url" "$OUT/p$i-n$r.json"; fi
  done
done
python3 -I - "$OUT" "$ROUNDS" "$#" "$@" <<'EOF'
import json, sys
out, rounds, n = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
urls = sys.argv[4:]
def js(path):
    try:
        j = json.load(open(path))
    except Exception:
        return None
    return sum(v["raw_cpu"]["total_ms"] for v in j["callbacks"].values()), j
for i in range(1, n + 1):
    a = [js(f"{out}/p{i}-n{r}.json") for r in range(1, rounds + 1)]
    b = [js(f"{out}/p{i}-r{r}.json") for r in range(1, rounds + 1)]
    a = [x[0] for x in a if x]; bb = [x for x in b if x]
    if not a or not bb:
        print(f"{urls[i-1]}: no data"); continue
    ma, mb = sorted(a)[len(a) // 2], sorted(x[0] for x in bb)[len(bb) // 2]
    m = bb[-1][1].get("missing_api", {})
    print(f"{urls[i-1]}: normal {ma:.1f} ms  report {mb:.1f} ms  ratio {mb / ma if ma else 0:.2f}  "
          f"(all: {[round(x,1) for x in a]} / {[round(x[0],1) for x in bb]}; members {len(m.get('members', []))}, ignored {m.get('ignored')})")
EOF
