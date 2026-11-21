#!/bin/bash
# QINC1k (R60 sections 10, 12): ONE run of the incremental-engine gate recipes.
#
# Usage: tools/incr-gate/run.sh KIND MODE PAGE TAG
#   KIND  window   (nested dwl + tools/inject + SCRATCHY_FRAME_STATS + grim screenshots)
#         headless (--perf-report --budget-ms 20000 --seconds S)
#   MODE  off      (SCRATCHY_INCR=0: the old full-relayout path; the engine is the default since QINC1l)
#         incr     (SCRATCHY_INCR=1, same as the default)
#         check    (SCRATCHY_INCR=1 SCRATCHY_INCR_CHECK=1: every engine update is re-done from scratch
#                   and compared; differences print `scratchy: incr-check: MISMATCH ...`)
#   PAGE  ytr (YouTube results, R58 recipe), ytw (YouTube watch G1AeBgalFZk, R55 recipe),
#         wiki (Wikipedia Rust, R55 recipe), github, bbc (bbc.com/news), social (social.loupax.com/discover),
#         hover (tools/incr-gate/hover.html: script-free, hover-heavy, no forced layouts)
#         imggeom (tools/incr-gate/img-geom.html: QINC1k5's image-size script-geometry probes, no input;
#                  the probes print `img-geom: ...` lines into the log)
#   TAG   run name; files go to $OUT/<TAG>.{log,stats,json,rss,pre.png,end.png,meta}
#
# Env: SCRATCHY  scratchy binary under test (COPY the installed one aside first: the coordinator
#                reinstalls ~/.cargo/bin/scratchy after every merge). Default: scratchy on PATH.
#      WIRE      wire binary (default: wire on PATH). Every run starts its OWN wire on its own socket
#                (fresh cookie jar), never the user's /run/user/1000/wire.sock.
#      INJECT    tools/inject binary (default tools/inject/target/release/inject)
#      OUT       output dir (default ${TMPDIR:-/tmp}/incr-gate; NOT tmpfs for long series is fine: files are small)
#      SOCK      nested dwl socket (window runs; default: start one with tools/nest.sh into $OUT/nest and
#                leave it running for later runs; gate.sh starts one per series)
#      PERF_LOCK lock file (default /tmp/scratchy-perf.lock); each run holds it. PERF_LOCK= skips.
#      MEMCAP    systemd-run scope cap (default 6G); every scratchy runs inside such a scope.
#      REPLAY    QREC3: base dir of page recordings (<REPLAY>/<page>/, made with RECORD=). The run's wire
#                is started `--replay <REPLAY>/<page> --replay-rules $RULES --replay-delay 5ms` (never the
#                network); headless runs also get `--deterministic=<epoch>` (epoch = mtime of the
#                recording's index.jsonl, ms; override with EPOCH=). hover/imggeom have no network: they
#                ignore REPLAY. Misses are counted into <TAG>.meta (replay_misses=N): more than a handful
#                makes the run invalid. Recordings stay LOCAL (R64, DECISIONS Q5): never commit one.
#      RECORD    QREC3: base dir; the run's wire is started `--no-cookies --record <RECORD>/<page>` (a
#                fresh jar, so no personal cookie is recorded; dirs mode 0700). Use a window run with the
#                page's input plan so hover/play requests are recorded too.
#      RULES     replay-rules file (default tools/incr-gate/replay-rules.txt).
# Input coordinates are written for a 960x1006 window and passed to inject as 960 1006, so on any
# other window size they land at the same FRACTION of the window (the pages are responsive).
# QINC1k3: INJECT_W / INJECT_H override that reference size: when the nested dwl gets a smaller tile
# (e.g. 960x316) and the page keeps its top layout, INJECT_H=<window height> makes the points absolute
# (the wiki plan's links are all above y 300; a point outside the extent is the compositor's to clamp).
# Only PIDs this script starts are ever killed.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
KIND=${1:?usage: run.sh KIND MODE PAGE TAG}
MODE=${2:?}
PAGE=${3:?}
TAG=${4:?}
SCRATCHY=${SCRATCHY:-scratchy}
WIRE=${WIRE:-wire}
INJECT=${INJECT:-$HERE/../inject/target/release/inject}
OUT=${OUT:-${TMPDIR:-/tmp}/incr-gate}
LOCK=${PERF_LOCK-/tmp/scratchy-perf.lock}
MEMCAP=${MEMCAP:-6G}
RULES=${RULES:-$HERE/replay-rules.txt}
REPLAY=${REPLAY:-}
RECORD=${RECORD:-}
RW=${INJECT_W:-960}
RH=${INJECT_H:-1006}
ulimit -c 0
mkdir -p "$OUT"

case $PAGE in
  ytr) URL='https://www.youtube.com/results?search_query=rust'; SECS=36; HSECS=30 ;;
  ytw) URL='https://www.youtube.com/watch?v=G1AeBgalFZk'; SECS=56; HSECS=30 ;;
  wiki) URL='https://en.wikipedia.org/wiki/Rust_(programming_language)'; SECS=52; HSECS=20 ;;
  github) URL='https://github.com/'; SECS=40; HSECS=20 ;;
  bbc) URL='https://www.bbc.com/news'; SECS=40; HSECS=20 ;;
  social) URL='https://social.loupax.com/discover'; SECS=40; HSECS=20 ;;
  hover) URL="$HERE/hover.html"; SECS=32; HSECS=10 ;;
  imggeom) URL="$HERE/img-geom.html"; SECS=10; HSECS=5 ;;
  *) echo "run.sh: unknown page $PAGE" >&2; exit 2 ;;
esac
case $MODE in
  off) MENV=(SCRATCHY_INCR=0) ;;
  incr) MENV=(SCRATCHY_INCR=1) ;;
  check) MENV=(SCRATCHY_INCR=1 SCRATCHY_INCR_CHECK=1) ;;
  *) echo "run.sh: unknown mode $MODE" >&2; exit 2 ;;
esac

# QREC3: record / replay arguments for this run's wire; local-file pages never touch the network
WARGS=()
DET=()
case $PAGE in hover|imggeom) ;; *)
  if [ -n "$REPLAY" ] && [ -n "$RECORD" ]; then echo "run.sh: REPLAY and RECORD contradict each other" >&2; exit 2; fi
  if [ -n "$REPLAY" ]; then
    [ -s "$REPLAY/$PAGE/index.jsonl" ] || { echo "run.sh: no recording in $REPLAY/$PAGE" >&2; exit 2; }
    WARGS=(--replay "$REPLAY/$PAGE" --replay-rules "$RULES" --replay-delay "${REPLAY_DELAY:-5ms}")
    EPOCH=${EPOCH:-$(( $(stat -c %Y "$REPLAY/$PAGE/index.jsonl") * 1000 ))}
    DET=(--deterministic="$EPOCH")
  elif [ -n "$RECORD" ]; then
    mkdir -p -m 0700 "$RECORD" "$RECORD/$PAGE"
    WARGS=(--no-cookies --record "$RECORD/$PAGE")
  fi ;;
esac

PIDS=()
cleanup() {
  local p
  for p in "${PIDS[@]}"; do
    if [ -r "/proc/$p/cmdline" ]; then kill "$p" 2>/dev/null; fi
  done
}
trap cleanup EXIT

# own wire, own socket, per run
WSOCK="$OUT/wire-$$.sock"
rm -f "$WSOCK"
"$WIRE" --socket "$WSOCK" "${WARGS[@]}" >"$OUT/$TAG.wire.log" 2>&1 9>&- &
WPID=$!
PIDS+=($WPID)
i=0
while [ ! -S "$WSOCK" ]; do
  i=$((i+1))
  if [ $i -gt 50 ]; then echo "run.sh: wire did not start, see $OUT/$TAG.wire.log" >&2; exit 1; fi
  sleep 0.1
done

inj() { WAYLAND_DISPLAY="$SOCK" "$INJECT" $RW $RH "$@" >>"$OUT/$TAG.inject.log" 2>&1 9>&-; }
T0=0
# sleep until T seconds after launch
at() { local now; now=$(date +%s.%N); local d; d=$(awk -v a="$T0" -v b="$now" -v t="$1" 'BEGIN { d = a + t - b; print (d > 0 ? d : 0) }'); sleep "$d"; }
shot() { WAYLAND_DISPLAY="$SOCK" grim "$OUT/$TAG.$1.png" 2>>"$OUT/$TAG.inject.log"; }

# raster sweep: rows from y0 to y1 step dy, x from x0 to x1 step dx, wait ms per move
sweep() {
  awk -v x0="$1" -v x1="$2" -v y0="$3" -v y1="$4" -v dx="$5" -v dy="$6" -v w="$7" 'BEGIN {
    r = 0
    for (y = y0; y <= y1; y += dy) {
      if (r % 2 == 0) { for (x = x0; x <= x1; x += dx) printf "move:%d,%d wait:%d ", x, y, w }
      else { for (x = x1; x >= x0; x -= dx) printf "move:%d,%d wait:%d ", x, y, w }
      r++
    }
  }'
}
# dwell on each of the given points for MS: rounds pts...
dwell() {
  local ms=$1 rounds=$2; shift 2
  local r p s=""
  for r in $(seq 1 "$rounds"); do for p in "$@"; do s="$s move:$p wait:$ms"; done; done
  echo "$s"
}

# The input plans (window). Timings are seconds after launch; `pre` = the screenshot BEFORE any
# input (the flag-on / flag-off visual comparison), `end` = after the input.
plan() {
  case $PAGE in
    ytr)  # R58: hover the first result's thumbnail (inline preview starts), the second, then the top bar
      at 14; shot pre
      at 15; inj move:300,230 wait:100 move:305,232 wait:5800
      inj move:300,420 wait:100 move:305,422 wait:5800
      inj move:480,30 wait:5800
      at 34; shot end ;;
    ytw)  # R55 (positions re-taken on 960x1006, 2026-11-20): play click at 13 s, 6 rounds x 8 positions x 600 ms (picture x2, progress bar, play/pause,
          # settings, search button, menu, sign-in), parked, screenshot at 54 s
      at 12; shot pre
      at 13; inj move:488,342 click wait:300
      at 15
      # shellcheck disable=SC2046
      inj $(dwell 600 6 300,200 650,300 400,542 58,566 909,573 681,23 783,30 875,30)
      inj move:480,900 wait:200
      at 54; shot end ;;
    wiki) # R55: from 12 s, 6 rounds x 8 links x 600 ms (general-purpose, performance, type safety, concurrency,
          # memory safety, Talk, Edit, View history: Page Previews), then parked on body text
      at 11; shot pre
      at 12
      # shellcheck disable=SC2046
      inj $(dwell 600 6 240,261 60,287 180,287 270,287 410,287 96,169 602,169 670,169)
      inj move:600,700 wait:200
      at 50; shot end ;;
    github|bbc|social)
      at 12; shot pre
      at 13
      # shellcheck disable=SC2046
      inj $(sweep 40 920 80 960 60 70 40)
      inj move:480,500 scroll:5 wait:1500
      # shellcheck disable=SC2046
      inj $(sweep 40 920 80 960 60 70 40)
      inj scroll:-5 wait:1000
      at $((SECS - 3)); shot end ;;
    hover)
      at 3; shot pre
      at 4
      # shellcheck disable=SC2046
      inj $(sweep 10 950 10 900 40 30 16)
      # shellcheck disable=SC2046
      inj $(sweep 25 950 25 900 40 30 16)
      at $((SECS - 3)); shot end ;;
    imggeom)
      at 3; shot pre
      at $((SECS - 3)); shot end ;;
  esac
}

{
  echo "tag=$TAG kind=$KIND mode=$MODE page=$PAGE url=$URL"
  echo "scratchy=$SCRATCHY ($(sha256sum "$(command -v "$SCRATCHY")" | cut -c1-12))"
  echo "start=$(date -Is) uptime_start=$(uptime | sed 's/.*load average: //')"
} >"$OUT/$TAG.meta"

if [ -n "$LOCK" ]; then exec 9>"$LOCK"; flock 9; fi
echo "locked=$(date -Is) uptime_locked=$(uptime | sed 's/.*load average: //')" >>"$OUT/$TAG.meta"

if [ "$KIND" = headless ]; then
  systemd-run --user --scope -q -p MemoryMax="$MEMCAP" -p MemorySwapMax=0 \
    env "${MENV[@]}" "$SCRATCHY" "${DET[@]}" --perf-report --socket "$WSOCK" --seconds "$HSECS" --budget-ms 20000 "$URL" \
    >"$OUT/$TAG.json" 2>"$OUT/$TAG.log" 9>&-
  echo "exit=$?" >>"$OUT/$TAG.meta"
else
  if [ -z "${SOCK:-}" ]; then
    SOCK=$("$HERE/../nest.sh" "$OUT/nest") || exit 1
    sleep 1
  fi
  rm -f "$OUT/$TAG.stats" "$OUT/$TAG.inject.log"
  # QINC1n: a state directory per run (localStorage, history, engine-check.log), never the user's.
  rm -rf "$OUT/$TAG.state"
  T0=$(date +%s.%N)
  systemd-run --user --scope -q -p MemoryMax="$MEMCAP" -p MemorySwapMax=0 \
    env WAYLAND_DISPLAY="$SOCK" SCRATCHY_FRAME_STATS="$OUT/$TAG.stats" SCRATCHY_MEDIA_AO=null SCRATCHY_STATE_DIR="$OUT/$TAG.state" "${MENV[@]}" \
    "$SCRATCHY" --socket "$WSOCK" "$URL" >"$OUT/$TAG.log" 2>&1 9>&- &
  SPID=$!
  PIDS+=($SPID)
  plan
  at "$SECS"
  if kill -0 $SPID 2>/dev/null; then
    grep -E '^(VmRSS|VmHWM)' "/proc/$SPID/status" >"$OUT/$TAG.rss" 2>/dev/null
    # Esc Esc: a page that focused a field leaves the window in INPUT mode (QKEYS2), where one Esc
    # goes to the page and `:q` would be typed into it (the run then ends with quit=killed).
    inj key:esc wait:60 key:esc wait:200 key:colon type:q key:ret wait:200
    i=0
    while kill -0 $SPID 2>/dev/null && [ $i -lt 100 ]; do sleep 0.1; i=$((i+1)); done
    if kill -0 $SPID 2>/dev/null; then
      echo "quit=killed" >>"$OUT/$TAG.meta"
      kill $SPID 2>/dev/null
    else
      echo "quit=clean" >>"$OUT/$TAG.meta"
    fi
  else
    echo "quit=died-early" >>"$OUT/$TAG.meta"
  fi
  wait $SPID 2>/dev/null
  echo "exit=$?" >>"$OUT/$TAG.meta"
fi
echo "end=$(date -Is) uptime_end=$(uptime | sed 's/.*load average: //')" >>"$OUT/$TAG.meta"
if [ -n "$LOCK" ]; then flock -u 9; exec 9>&-; fi
kill $WPID 2>/dev/null
wait $WPID 2>/dev/null
rm -f "$WSOCK"
if [ -n "$REPLAY" ]; then
  echo "replay_misses=$(grep -c 'X-Wire-Replay: miss\|replay miss' "$OUT/$TAG.wire.log") replay=$REPLAY/$PAGE" >>"$OUT/$TAG.meta"
fi
