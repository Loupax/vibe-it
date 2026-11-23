#!/bin/bash
# Start a nested dwl (Wayland backend, a window inside the current session)
# and print its socket name (e.g. wayland-1) on stdout. The dwl PID is
# written to $OUT/dwl.pid; kill that PID (only that one) when done.
# Usage: tools/nest.sh [--new|--stop|--x11-display] [OUT_DIR]   (default: ${TMPDIR:-/tmp}/scratchy-nest)
# (web/tools/nest.sh: copied from scratchy/tools/nest.sh for web C2; it also records the nested
# Xwayland's DISPLAY: `--x11-display` prints it, e.g. :1, empty if dwl has no Xwayland. Xwayland
# starts lazily on the first X11 client. A dwl started by scratchy's copy records no DISPLAY: --new.)
# QNEST1: ONE nested dwl per worktree. The compositor is recorded in the registry dir
# <worktree>/.qwork/nest (env NEST_REG overrides): dwl.pid, disp (socket), dwl.log. A second call
# finds the live dwl there (PID alive and its command line names this registry), prints its socket
# and reuses it, whatever OUT_DIR is (OUT_DIR/dwl.pid and OUT_DIR/disp are refreshed for callers).
#   --new   stop the recorded dwl (by PID) first, then start a fresh one
#   --stop  stop the recorded dwl by its PID, remove the pid file, print nothing
# NEST_BACKEND=headless is for tests only (headless dwl has no input).
# A pid file whose process is gone (or is not our dwl) is cleaned up.
# dwl is started with every inherited fd above 2 closed (QTOOL1): a nested dwl
# outlives the caller (up to an hour) and would otherwise keep, e.g., the
# `flock /tmp/scratchy-perf.lock` fd of a caller holding the perf lock.
# The wayland backend is required for input: headless dwl advertises no
# seat capabilities, so clients get no pointer or keyboard.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
MODE=
case "${1:-}" in --new|--stop|--x11-display) MODE=$1; shift;; esac
OUT=${1:-${TMPDIR:-/tmp}/scratchy-nest}
ROOT=$(git -C "$HERE" rev-parse --show-toplevel 2>/dev/null || echo "$HERE/..")
REG=${NEST_REG:-$ROOT/.qwork/nest}
mkdir -p "$REG"
# live_pid: print the recorded PID if it is our dwl (cmdline names $REG/disp), else clean the files.
live_pid() {
  local p
  p=$(cat "$REG/dwl.pid" 2>/dev/null) || return 1
  if [ -n "$p" ] && [ -r "/proc/$p/cmdline" ] && tr '\0' ' ' <"/proc/$p/cmdline" | grep -qF "$REG/disp"; then
    echo "$p"; return 0
  fi
  rm -f "$REG/dwl.pid" "$REG/disp" "$REG/xdisp"
  return 1
}
stop_nest() {
  local p i
  p=$(live_pid) || return 0
  kill "$p" 2>/dev/null || true   # timeout forwards TERM to dwl; without timeout it is dwl itself
  i=0
  while [ -d "/proc/$p" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i+1)); done
  rm -f "$REG/dwl.pid" "$REG/disp"
}
publish() {  # refresh the caller's OUT dir
  mkdir -p "$OUT"
  if [ "$(cd "$OUT" && pwd)" != "$(cd "$REG" && pwd)" ]; then
    cp "$REG/dwl.pid" "$OUT/dwl.pid"; cp "$REG/disp" "$OUT/disp"
  fi
}
if [ "$MODE" = --stop ]; then stop_nest; exit 0; fi
if [ "$MODE" = --x11-display ]; then
  live_pid >/dev/null || { echo "nest.sh: no nested dwl running" >&2; exit 1; }
  cat "$REG/xdisp" 2>/dev/null
  exit 0
fi
[ "$MODE" = --new ] && stop_nest
if live_pid >/dev/null && [ -s "$REG/disp" ]; then
  publish
  cat "$REG/disp"
  exit 0
fi
# QREC3b: NEST_TIMEOUT (seconds, default 3600; 0 = none): gate.sh runs longer than an hour and kills its own dwl by PID.
NEST_TIMEOUT=${NEST_TIMEOUT-3600}
[ "$NEST_TIMEOUT" = 0 ] && NEST_TIMEOUT=
rm -f "$REG/disp" "$REG/dwl.pid" "$REG/xdisp"
(
  for f in /proc/self/fd/*; do
    n=${f##*/}
    if [ "$n" -gt 2 ] 2>/dev/null; then eval "exec $n>&-" 2>/dev/null || true; fi
  done
  exec env -u DISPLAY WLR_BACKENDS=${NEST_BACKEND:-wayland} WLR_RENDERER=pixman \
    setsid ${NEST_TIMEOUT:+timeout $NEST_TIMEOUT} dwl -s "echo \$DISPLAY > '$REG/xdisp'; echo \$WAYLAND_DISPLAY > '$REG/disp'" \
    >"$REG/dwl.log" 2>&1
) &
echo $! > "$REG/dwl.pid"
i=0
while [ ! -s "$REG/disp" ]; do
  i=$((i+1))
  if [ $i -gt 100 ]; then
    echo "nest.sh: dwl did not start, see $REG/dwl.log" >&2
    exit 1
  fi
  sleep 0.1
done
publish
cat "$REG/disp"
