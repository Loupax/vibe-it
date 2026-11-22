#!/bin/bash
# QMAPI3: missing-API corpus. Runs `scratchy --missing-api-report` over a site list, ONE page at a time (each run
# under the perf lock and a memory cap, with a time limit), through ONE own wire on its own socket without cookies,
# then merges the per-page JSON into tables with merge.py:
#   API (Interface.member) -> number of sites, total hits, example URL     (JS members, sorted by sites then hits)
#   CSS (kind name)        -> number of sites, total hits, example URL
# plus the grouped errors. Per-page JSON stays in $OUT/<name>.json, the merged Markdown goes to stdout.
#
# Usage: SCRATCHY=bin WIRE=bin OUT=dir [SITES=file] [SECS=30] [LIMIT=120] tools/missing-api/corpus.sh
#   SITES   site list (default tools/missing-api/sites.txt next to this script), format in that file
#   SECS    --seconds per page (page time after load, default 30); LIMIT: hard `timeout` per page (default 120)
#   PERF_LOCK  lock file (default /tmp/scratchy-perf.lock; empty = none). MEMCAP (default 6G).
#   SCRATCHY_STATE_DIR must be set to a scratch dir (never the user's state dir). TOP: rows per table (default 40).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
SCRATCHY=${SCRATCHY:-scratchy}
WIRE=${WIRE:-wire}
OUT=${OUT:?set OUT to a scratch dir}
SITES=${SITES:-$HERE/sites.txt}
SECS=${SECS:-30}
LIMIT=${LIMIT:-120}
LOCK=${PERF_LOCK-/tmp/scratchy-perf.lock}
MEMCAP=${MEMCAP:-6G}
: "${SCRATCHY_STATE_DIR:?set SCRATCHY_STATE_DIR to a scratch dir, not the real state dir}"
export SCRATCHY_STATE_DIR
mkdir -p "$OUT" "$SCRATCHY_STATE_DIR"
ulimit -c 0
SOCK="${XDG_RUNTIME_DIR:-/tmp}/qmapi3-corpus-$$.sock" # short path: socket paths are limited to ~108 bytes
rm -f "$SOCK"
"$WIRE" --socket "$SOCK" --no-cookies >"$OUT/wire.log" 2>&1 9>&- &
WPID=$!
cleanup() { if [ -r "/proc/$WPID/cmdline" ]; then kill "$WPID" 2>/dev/null; fi; rm -f "$SOCK"; }
trap cleanup EXIT
for _ in $(seq 50); do [ -S "$SOCK" ] && break; sleep 0.1; done
[ -S "$SOCK" ] || { echo "corpus.sh: wire did not start, see $OUT/wire.log" >&2; exit 1; }

: >"$OUT/pages.tsv"
while read -r name url _; do
  case $name in ''|'#'*) continue ;; esac
  target=$url
  case $url in http://*|https://*) ;; *) target="$ROOT/$url" ;; esac
  echo "# $(date -Is) $name $url" >&2
  cmd=(systemd-run --user --scope -q -p MemoryMax="$MEMCAP" -p MemorySwapMax=0 timeout "$LIMIT"
       "$SCRATCHY" --missing-api-report --budget-ms 20000 --socket "$SOCK" --seconds "$SECS" "$target")
  if [ -n "$LOCK" ]; then flock "$LOCK" "${cmd[@]}" >"$OUT/$name.json" 2>"$OUT/$name.err"
  else "${cmd[@]}" >"$OUT/$name.json" 2>"$OUT/$name.err"; fi
  echo "$name rc=$? bytes=$(stat -c %s "$OUT/$name.json")" >&2
  printf '%s\t%s\n' "$name" "$url" >>"$OUT/pages.tsv"
done <"$SITES"
python3 -I "$HERE/merge.py" "$OUT/pages.tsv" "$OUT" "${TOP:-40}"
