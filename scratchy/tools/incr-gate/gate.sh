#!/bin/bash
# QINC1k (R60 section 12): the incremental-engine gate series, built on tools/incr-gate/run.sh.
#
# Usage: tools/incr-gate/gate.sh SERIES [RUNS] [PAGES...]
#   SERIES  check     SCRATCHY_INCR_CHECK=1 (+ SCRATCHY_INCR=1): headless AND window run per page per round
#                     (exactness: 0 `incr-check: MISMATCH` lines is the gate)
#           perf      window: flag off vs SCRATCHY_INCR=1 of the SAME binary, interleaved, order flipped per round
#           headless  the same pairs headless (--perf-report: forced layouts, rss_bytes, incr counters)
#           replay    QREC3: window, SCRATCHY_INCR=1, LIVE vs REPLAY of the same pages, interleaved, order
#                     flipped per round (tags replay-<page>-live-<r> / replay-<page>-rep-<r>); needs REPLAY=DIR
#                     (recordings, see run.sh). Also prints the variance.py 3-run min/median/max table.
#           replay-headless  the same pair headless (tags replayhl-...)
#   RUNS    rounds (default 3)
#   PAGES   default: ytr ytw wiki github bbc social hover (see run.sh)
# Env: as run.sh (SCRATCHY, WIRE, INJECT, OUT, PERF_LOCK, MEMCAP). Every run holds the perf lock for itself only.
# Starts ONE nested dwl for the series (window runs) and kills only that PID at the end.
# At the end prints the summarize.py tables for the series' runs (tags <series>-<page>-<mode>-<round>).
#
# Example (the QINC1k sweep; copy the binary first, it must not change mid-series):
#   cp target/release/scratchy $Q/scratchy-under-test
#   (cd tools/inject && cargo build --release)
#   SCRATCHY=$Q/scratchy-under-test WIRE=$Q/wire OUT=$Q/gate tools/incr-gate/gate.sh check
#   SCRATCHY=$Q/scratchy-under-test WIRE=$Q/wire OUT=$Q/gate tools/incr-gate/gate.sh perf
#   SCRATCHY=$Q/scratchy-under-test WIRE=$Q/wire OUT=$Q/gate tools/incr-gate/gate.sh headless
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
SERIES=${1:?usage: gate.sh check|perf|headless [RUNS] [PAGES...]}
RUNS=${2:-3}
shift
[ $# -gt 0 ] && shift
PAGES=("$@")
[ ${#PAGES[@]} -eq 0 ] && PAGES=(ytr ytw wiki github bbc social hover)
OUT=${OUT:-${TMPDIR:-/tmp}/incr-gate}
export OUT
mkdir -p "$OUT"
DWL=
case $SERIES in replay|replay-headless) [ -n "${REPLAY:-}" ] || { echo "gate.sh: $SERIES needs REPLAY=DIR" >&2; exit 2; } ;; esac
if [ "$SERIES" != headless ] && [ "$SERIES" != replay-headless ]; then
  SOCK=$(NEST_TIMEOUT=0 "$HERE/../nest.sh" "$OUT/nest") || exit 1
  DWL=$(cat "$OUT/nest/dwl.pid")
  export SOCK
  sleep 1
fi
cleanup() { if [ -n "$DWL" ] && [ -r "/proc/$DWL/cmdline" ]; then kill "$DWL" 2>/dev/null; fi; }
trap cleanup EXIT

echo "# $(date -Is) series=$SERIES runs=$RUNS pages=${PAGES[*]} scratchy=${SCRATCHY:-scratchy} uptime: $(uptime | sed 's/.*load average: //')"
for r in $(seq 1 "$RUNS"); do
  for p in "${PAGES[@]}"; do
    case $SERIES in
      check)
        "$HERE/run.sh" headless check "$p" "check-$p-hl-$r"
        "$HERE/run.sh" window check "$p" "check-$p-win-$r" ;;
      perf|headless)
        kind=window; [ "$SERIES" = headless ] && kind=headless
        if [ $((r % 2)) = 1 ]; then order="off incr"; else order="incr off"; fi
        for m in $order; do "$HERE/run.sh" "$kind" "$m" "$p" "$SERIES-$p-$m-$r"; done ;;
      replay|replay-headless)
        kind=window; pre=replay
        if [ "$SERIES" = replay-headless ]; then kind=headless; pre=replayhl; fi
        if [ $((r % 2)) = 1 ]; then order="live rep"; else order="rep live"; fi
        for m in $order; do
          if [ $m = rep ]; then "$HERE/run.sh" "$kind" incr "$p" "$pre-$p-rep-$r"
          else REPLAY= "$HERE/run.sh" "$kind" incr "$p" "$pre-$p-live-$r"; fi
        done ;;
      *) echo "gate.sh: unknown series $SERIES" >&2; exit 2 ;;
    esac
    echo "# round $r page $p done $(date -Is)"
  done
done
case $SERIES in replay) PFX=replay- ;; replay-headless) PFX=replayhl- ;; *) PFX=$SERIES- ;; esac
python3 -I "$HERE/summarize.py" "$OUT" "$PFX"
case $SERIES in replay*) python3 -I "$HERE/variance.py" "$OUT" "$PFX" ;; esac
