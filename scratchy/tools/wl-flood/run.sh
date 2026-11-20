#!/bin/bash
# QY54a1: does scratchy survive a pointer-motion flood while its main thread stalls?
# (R56 section 2 / R57 section 5.) The nested dwl drops a client once ~190-225
# pointer-motion events sit unread; this harness reproduces that, one line per run.
#
# Usage: tools/wl-flood/run.sh PAGE HZ SECONDS [RUNS]
#   PAGE     busyN    -> busy.html?ms=N   (a timer every 4 s busy-loops N ms)
#            stall_kN -> stall.html?k=N   (8000 rows + N forced layouts every 4 s; k0 = control)
#            or any URL / path to a page (a local .html path is passed to scratchy as is)
#   HZ       500 (tools/inject wait:2, ~460 moves/s measured) or 125 (wait:8);
#            any other number n uses wait = round(1000/n) ms
#   SECONDS  how long the pointer moves; motion starts 1 s after launch
#   RUNS     default 3
# Env: SCRATCHY  scratchy binary under test (default: scratchy on PATH; COPY the installed
#                binary aside first, the coordinator reinstalls it after merges)
#      INJECT    tools/inject binary (default tools/inject/target/release/inject;
#                build: cd tools/inject && cargo build --release)
#      WIRE      wire binary (default: wire on PATH); run.sh starts its OWN wire on its own socket
#      OUT       scratch dir (default ${TMPDIR:-/tmp}/wl-flood): nest/, wire.sock, runN.log, runN.stats
#      W, H      window size for absolute moves (default 1052 1050, the nested dwl window)
#      PERF_LOCK lock file (default /tmp/scratchy-perf.lock). Each run holds flock on it; set
#                PERF_LOCK= (empty) to skip.
# Output: one `# ... uptime` header, then one line per run:
#   PAGE HZ run N: survived S s (...); gap_max X ms ...
#   PAGE HZ run N: lost at T s (...); gap_max X ms ...
# lost = scratchy exited while the injector was still moving the pointer; the line says whether
# QB1's "lost the connection to the Wayland compositor" message was in the log. gap_max is the
# longest redraw gap in SCRATCHY_FRAME_STATS (the stall length as the window saw it).
# Only the PIDs this script started (own wire, own dwl, scratchy, inject) are ever killed.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
PAGE=${1:?usage: run.sh PAGE HZ SECONDS [RUNS]}
HZ=${2:?}
SECS=${3:?}
RUNS=${4:-3}
SCRATCHY=${SCRATCHY:-scratchy}
INJECT=${INJECT:-$HERE/../inject/target/release/inject}
WIRE=${WIRE:-wire}
OUT=${OUT:-${TMPDIR:-/tmp}/wl-flood}
W=${W:-1052}
H=${H:-1050}
LOCK=${PERF_LOCK-/tmp/scratchy-perf.lock}
ulimit -c 0
case $HZ in
  500) WAIT=2 ;;
  125) WAIT=8 ;;
  *) WAIT=$(( (1000 + HZ / 2) / HZ )) ;;
esac
mkdir -p "$OUT"
# scratchy opens a local file path as is (no query string), so the fixture's default
# parameter is baked into a copy in $OUT (the same files also take ?ms=N / ?k=N as a URL).
case $PAGE in
  busy[0-9]*)
    URL="$OUT/$PAGE.html"
    sed "s/\\[0, \"450\"\\]/[0, \"${PAGE#busy}\"]/" "$HERE/busy.html" >"$URL" ;;
  stall_k[0-9]*)
    URL="$OUT/$PAGE.html"
    sed "s/\\[0, \"18\"\\]/[0, \"${PAGE#stall_k}\"]/" "$HERE/stall.html" >"$URL" ;;
  *)  URL=$PAGE ;;
esac
PIDS=()
cleanup() {
  local p
  for p in "${PIDS[@]}"; do
    # only PIDs we started
    if [ -r "/proc/$p/cmdline" ]; then kill "$p" 2>/dev/null; fi
  done
}
trap cleanup EXIT

# own wire on its own socket
WSOCK="$OUT/wire.sock"
rm -f "$WSOCK"
"$WIRE" --socket "$WSOCK" >"$OUT/wire.log" 2>&1 &
PIDS+=($!)
i=0
while [ ! -S "$WSOCK" ]; do
  i=$((i+1))
  if [ $i -gt 50 ]; then echo "run.sh: wire did not start, see $OUT/wire.log" >&2; exit 1; fi
  sleep 0.1
done

# own nested dwl
SOCK=$("$HERE/../nest.sh" "$OUT/nest") || exit 1
PIDS+=("$(cat "$OUT/nest/dwl.pid")")

# a circle of 60 points around the window centre, enough moves for SECONDS
CMDS=$(awk -v w="$W" -v h="$H" -v s="$SECS" -v wait="$WAIT" 'BEGIN {
  n = int(s * 1000 / wait) + 100; pi = 3.14159265
  for (i = 0; i < n; i++) {
    a = 2 * pi * (i % 60) / 60
    printf "move:%d,%d wait:%d ", w / 2 + 0.3 * w * cos(a), h / 2 + 0.3 * h * sin(a), wait
  }
}')

run_one() {
  local r=$1 log="$OUT/run$r.log" stats="$OUT/run$r.stats" ilog="$OUT/run$r.inject.log"
  rm -f "$log" "$stats"
  local t0 t1 sp ip alive_end
  t0=$(date +%s.%N)
  env WAYLAND_DISPLAY="$SOCK" SCRATCHY_FRAME_STATS="$stats" SCRATCHY_MEDIA_AO=null \
    timeout $((SECS + 30)) "$SCRATCHY" --socket "$WSOCK" "$URL" >"$log" 2>&1 &
  sp=$!
  PIDS+=($sp)
  sleep 1
  # shellcheck disable=SC2086
  WAYLAND_DISPLAY="$SOCK" timeout "$SECS" "$INJECT" "$W" "$H" $CMDS >"$ilog" 2>&1 &
  ip=$!
  PIDS+=($ip)
  # until the injector ends (SECONDS) or scratchy exits
  while kill -0 $ip 2>/dev/null && kill -0 $sp 2>/dev/null; do sleep 0.1; done
  t1=$(date +%s.%N)
  if kill -0 $sp 2>/dev/null; then alive_end=1; else alive_end=0; fi
  kill $ip 2>/dev/null; wait $ip 2>/dev/null
  if [ $alive_end = 1 ]; then kill $sp 2>/dev/null; fi
  wait $sp 2>/dev/null
  local stall="no frame stats" sent=0 T
  if [ -s "$stats" ]; then
    local gap cb
    gap=$(grep -o '"gap_max_ms":[0-9.]*' "$stats" | cut -d: -f2 | awk '{ if ($1 + 0 > m) m = $1 + 0 } END { printf "%.0f", m }')
    cb=$(grep -o '"longest":{"ms":[0-9.]*' "$stats" | cut -d: -f3 | awk '{ if ($1 + 0 > m) m = $1 + 0 } END { printf "%.0f", m }')
    stall="stall: redraw gap max $gap ms, longest callback $cb ms"
    sent=$(grep -o '"move":[0-9]*' "$stats" | cut -d: -f2 | awk '{s += $1} END {print s + 0}')
  fi
  T=$(awk -v a="$t0" -v b="$t1" 'BEGIN { printf "%.1f", b - a }')
  if [ $alive_end = 1 ]; then
    echo "$PAGE ${HZ}Hz run $r: survived ${SECS} s of motion (moves dispatched $sent); $stall"
  elif grep -q "lost the connection" "$log"; then
    echo "$PAGE ${HZ}Hz run $r: lost at $T s after launch (moves dispatched $sent); $stall"
  else
    echo "$PAGE ${HZ}Hz run $r: scratchy exited at $T s WITHOUT the lost-connection message, see $log (moves dispatched $sent); $stall"
  fi
}

echo "# $(date -Is) page=$PAGE url=$URL hz=$HZ wait=${WAIT}ms secs=$SECS runs=$RUNS uptime:$(uptime | sed 's/.*load average/ load average/')"
for r in $(seq 1 "$RUNS"); do
  if [ -n "$LOCK" ]; then exec 9>"$LOCK"; flock 9; fi
  run_one "$r"
  if [ -n "$LOCK" ]; then flock -u 9; exec 9>&-; fi
  sleep 1
done
