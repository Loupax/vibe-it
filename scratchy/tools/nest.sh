#!/bin/bash
# Start a nested dwl (Wayland backend, a window inside the current session)
# and print its socket name (e.g. wayland-1) on stdout. The dwl PID is
# written to $OUT/dwl.pid; kill that PID (only that one) when done.
# Usage: tools/nest.sh [OUT_DIR]   (default: ${TMPDIR:-/tmp}/scratchy-nest)
# dwl is started with every inherited fd above 2 closed (QTOOL1): a nested dwl
# outlives the caller (up to an hour) and would otherwise keep, e.g., the
# `flock /tmp/scratchy-perf.lock` fd of a caller holding the perf lock.
# The wayland backend is required for input: headless dwl advertises no
# seat capabilities, so clients get no pointer or keyboard.
set -e
# QREC3b: NEST_TIMEOUT (seconds, default 3600; 0 = none): gate.sh runs longer than an hour and kills its own dwl by PID.
NEST_TIMEOUT=${NEST_TIMEOUT-3600}
[ "$NEST_TIMEOUT" = 0 ] && NEST_TIMEOUT=
OUT=${1:-${TMPDIR:-/tmp}/scratchy-nest}
mkdir -p "$OUT"
rm -f "$OUT/disp"
(
  for f in /proc/self/fd/*; do
    n=${f##*/}
    if [ "$n" -gt 2 ] 2>/dev/null; then eval "exec $n>&-" 2>/dev/null || true; fi
  done
  exec env -u DISPLAY WLR_BACKENDS=wayland WLR_RENDERER=pixman \
    setsid ${NEST_TIMEOUT:+timeout $NEST_TIMEOUT} dwl -s "echo \$WAYLAND_DISPLAY > '$OUT/disp'" \
    >"$OUT/dwl.log" 2>&1
) &
echo $! > "$OUT/dwl.pid"
i=0
while [ ! -s "$OUT/disp" ]; do
  i=$((i+1))
  if [ $i -gt 100 ]; then
    echo "nest.sh: dwl did not start, see $OUT/dwl.log" >&2
    exit 1
  fi
  sleep 0.1
done
cat "$OUT/disp"
