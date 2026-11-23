#!/bin/bash
# Start a nested dwl for live tests and print its socket name (e.g. wayland-1) on stdout.
# The dwl PID is written to $OUT/dwl.pid.
# Usage: web/tools/nest.sh [--windowed] [--new|--stop|--x11-display] [OUT_DIR]   (default: ${TMPDIR:-/tmp}/scratchy-nest)
#
# NEST-HEADLESS (default): dwl on the wlroots HEADLESS backend. No display is needed and no
# window appears on the user's desktop; the headless output still produces frames (60 Hz),
# so clients (CEF) keep getting frame callbacks, grim screenshots work (wlr-screencopy) and
# web/tools/inject works (virtual keyboard + virtual pointer). Stock dwl advertises no seat
# capabilities without a backend input device, so clients would never bind wl_pointer or
# wl_keyboard; the helper therefore runs a dwl built from the user's dwl source with
# nest/dwl-headless.patch (seat advertises pointer+keyboard, output size from
# NEST_HEADLESS_SIZE). It is built once into <worktree>/.qwork/nest-dwl/ (rebuilt when the
# source or the patch changes; takes a few seconds).
#   NEST_SIZE   headless output size WxH (default 960x1006, what notab.py/inject assume);
#               also written to the registry file `size`. Logical px: with NEST_SCALE the
#               output has NEST_SIZE * NEST_SCALE pixels (grim's screenshots are that big)
#   NEST_SCALE  headless output scale (C6b; default 1; fractional ok, e.g. 1.5 or 2), written to
#               the registry file `scale`. A live dwl with another scale is replaced.
#   NEST_REFRESH  headless output refresh rate in Hz (C6c; default unset = wlroots' 60), written to the
#               registry file `refresh`; a live dwl with another one is replaced. The shell's window
#               gets frames at this rate, so it is what the page's requestAnimationFrame follows.
#   DWL_SRC     dwl source tree to build from (default /home/loupax/src/dotfiles/dwl)
#   WLROOTS_DIR wlroots tree with install/lib/pkgconfig (default: $DWL_SRC/../wlroots)
#
# --windowed: the old mode, the installed dwl on the wayland backend: a window inside the
# current session (needs the user's display; use only when a human wants to watch).
#
# QNEST1: ONE nested dwl per worktree. The compositor is recorded in the registry dir
# <worktree>/.qwork/nest (env NEST_REG overrides): dwl.pid, disp (socket), mode, size, dwl.log.
# A second call finds the live dwl there (PID alive and its command line names this
# registry), prints its socket and reuses it, whatever OUT_DIR is (OUT_DIR/dwl.pid and
# OUT_DIR/disp are refreshed for callers). If the live one runs in the other mode it is
# stopped and a new one started.
#   --new   stop the recorded dwl (by PID) first, then start a fresh one
#   --stop  stop the recorded dwl by its PID, remove the pid file, print nothing
#   --x11-display  print the running dwl's nested Xwayland DISPLAY (e.g. :1; empty if dwl has no
#           Xwayland) (C2). Xwayland starts lazily on the first X11 client. Works in both modes:
#           the headless dwl has Xwayland too (X11 clients run inside it, no window anywhere).
#           A dwl started by an older copy of this script recorded none: --new.
# NEST_BACKEND=wayland is the same as --windowed (older callers).
# A pid file whose process is gone (or is not our dwl) is cleaned up.
# dwl is started with every inherited fd above 2 closed (QTOOL1): a nested dwl
# outlives the caller (up to an hour) and would otherwise keep, e.g., the
# `flock /tmp/scratchy-perf.lock` fd of a caller holding the perf lock.
# The headless dwl gets none of the session's display variables (WAYLAND_DISPLAY,
# WAYLAND_SOCKET, DISPLAY): check with `tr '\0' '\n' </proc/$(cat .qwork/nest/dwl.pid)/environ`.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
MODE=
WANT=headless
[ "${NEST_BACKEND:-}" = wayland ] && WANT=windowed
while [ $# -gt 0 ]; do
  case "$1" in
    --new|--stop|--x11-display) MODE=$1; shift;;
    --windowed) WANT=windowed; shift;;
    --headless) WANT=headless; shift;;
    *) break;;
  esac
done
OUT=${1:-${TMPDIR:-/tmp}/scratchy-nest}
ROOT=$(git -C "$HERE" rev-parse --show-toplevel 2>/dev/null || echo "$HERE/../..")
REG=${NEST_REG:-$ROOT/.qwork/nest}
SIZE=${NEST_SIZE:-960x1006}
SCALE=${NEST_SCALE:-1}
REFRESH=${NEST_REFRESH:-0}
# The output's pixels: the logical NEST_SIZE times the scale, rounded (as a desktop sizes a scaled output).
PXSIZE=$(echo "$SIZE $SCALE" | awk '{ split($1, a, "x"); printf "%dx%d", int(a[1] * $2 + 0.5), int(a[2] * $2 + 0.5) }')
mkdir -p "$REG"
# live_pid: print the recorded PID if it is our dwl (cmdline names $REG/disp), else clean the files.
live_pid() {
  local p
  p=$(cat "$REG/dwl.pid" 2>/dev/null) || return 1
  if [ -n "$p" ] && [ -r "/proc/$p/cmdline" ] && tr '\0' ' ' <"/proc/$p/cmdline" | grep -qF "$REG/disp"; then
    echo "$p"; return 0
  fi
  rm -f "$REG/dwl.pid" "$REG/disp" "$REG/xdisp" "$REG/mode" "$REG/size" "$REG/scale"
  return 1
}
stop_nest() {
  local p i
  p=$(live_pid) || return 0
  kill "$p" 2>/dev/null || true   # timeout forwards TERM to dwl; without timeout it is dwl itself
  i=0
  while [ -d "/proc/$p" ] && [ $i -lt 50 ]; do sleep 0.1; i=$((i+1)); done
  rm -f "$REG/dwl.pid" "$REG/disp" "$REG/xdisp" "$REG/mode" "$REG/size" "$REG/scale"
}
publish() {  # refresh the caller's OUT dir
  mkdir -p "$OUT"
  if [ "$(cd "$OUT" && pwd)" != "$(cd "$REG" && pwd)" ]; then
    cp "$REG/dwl.pid" "$OUT/dwl.pid"; cp "$REG/disp" "$OUT/disp"
  fi
}
# build_dwl: print the path of the patched dwl, building it when missing or stale.
build_dwl() {
  local src=${DWL_SRC:-/home/loupax/src/dotfiles/dwl}
  local wlr=${WLROOTS_DIR:-$src/../wlroots}
  local patch=$HERE/nest/dwl-headless.patch
  local dir=$ROOT/.qwork/nest-dwl
  local stamp
  if [ ! -f "$src/dwl.c" ]; then
    echo "nest.sh: no dwl source at $src (set DWL_SRC)" >&2; return 1
  fi
  stamp=$(cat "$src"/*.c "$src"/*.h "$src/config.mk" "$src/Makefile" "$patch" | sha256sum | cut -c1-16)
  if [ -x "$dir/dwl" ] && [ "$(cat "$dir/stamp" 2>/dev/null)" = "$stamp" ]; then
    echo "$dir/dwl"; return 0
  fi
  rm -rf "$dir/src"
  mkdir -p "$dir/src"
  cp "$src"/*.c "$src"/*.h "$src/config.mk" "$src/Makefile" "$dir/src/"
  [ -d "$src/protocols" ] && cp -r "$src/protocols" "$dir/src/"
  if ! patch -s -d "$dir/src" -p1 <"$patch" >&2; then
    echo "nest.sh: $patch does not apply to $src/dwl.c; update the patch" >&2; return 1
  fi
  if ! nice -n 19 make -s -C "$dir/src" WLROOTS_DIR="$wlr" VERSION=nest-headless dwl >"$dir/build.log" 2>&1; then
    echo "nest.sh: building the headless dwl failed, see $dir/build.log" >&2; return 1
  fi
  cp "$dir/src/dwl" "$dir/dwl"
  echo "$stamp" >"$dir/stamp"
  echo "$dir/dwl"
}
if [ "$MODE" = --stop ]; then stop_nest; exit 0; fi
if [ "$MODE" = --x11-display ]; then
  live_pid >/dev/null || { echo "nest.sh: no nested dwl running" >&2; exit 1; }
  cat "$REG/xdisp" 2>/dev/null
  exit 0
fi
[ "$MODE" = --new ] && stop_nest
if live_pid >/dev/null && [ -s "$REG/disp" ]; then
  if [ "$(cat "$REG/mode" 2>/dev/null || echo windowed)" = "$WANT" ] && { [ "$WANT" = windowed ] || [ "$(cat "$REG/scale" 2>/dev/null || echo 1)" = "$SCALE" ] && [ "$(cat "$REG/refresh" 2>/dev/null || echo 0)" = "$REFRESH" ]; }; then
    publish
    cat "$REG/disp"
    exit 0
  fi
  stop_nest
fi
# QREC3b: NEST_TIMEOUT (seconds, default 3600; 0 = none): gate.sh runs longer than an hour and kills its own dwl by PID.
NEST_TIMEOUT=${NEST_TIMEOUT-3600}
[ "$NEST_TIMEOUT" = 0 ] && NEST_TIMEOUT=
rm -f "$REG/disp" "$REG/dwl.pid" "$REG/xdisp" "$REG/mode" "$REG/size" "$REG/scale" "$REG/refresh"
if [ "$WANT" = headless ]; then
  DWL=$(build_dwl)
  ENV=(env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET WLR_BACKENDS=headless WLR_RENDERER=pixman
       WLR_HEADLESS_OUTPUTS=1 NEST_HEADLESS_SIZE="$PXSIZE" NEST_HEADLESS_SCALE="$SCALE" NEST_HEADLESS_REFRESH="$REFRESH")
else
  DWL=dwl
  ENV=(env -u DISPLAY WLR_BACKENDS=wayland WLR_RENDERER=pixman)
fi
(
  for f in /proc/self/fd/*; do
    n=${f##*/}
    if [ "$n" -gt 2 ] 2>/dev/null; then eval "exec $n>&-" 2>/dev/null || true; fi
  done
  exec "${ENV[@]}" setsid ${NEST_TIMEOUT:+timeout $NEST_TIMEOUT} "$DWL" -s "echo \$DISPLAY > '$REG/xdisp'; echo \$WAYLAND_DISPLAY > '$REG/disp'" \
    >"$REG/dwl.log" 2>&1
) &
echo $! > "$REG/dwl.pid"
echo "$WANT" > "$REG/mode"
[ "$WANT" = headless ] && echo "$SIZE" > "$REG/size" && echo "$SCALE" > "$REG/scale" && echo "$REFRESH" > "$REG/refresh"
i=0
while [ ! -s "$REG/disp" ]; do
  i=$((i+1))
  if [ $i -gt 100 ] || ! [ -d "/proc/$(cat "$REG/dwl.pid")" ]; then
    echo "nest.sh: dwl did not start, see $REG/dwl.log" >&2
    exit 1
  fi
  sleep 0.1
done
publish
cat "$REG/disp"
