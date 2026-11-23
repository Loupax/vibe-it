#!/bin/bash
# C6d: a headless nested compositor WITH the GPU, for dmabuf checks and GPU-path measurements.
# Prints its Wayland socket name (wayland-gpu-<worktree hash>) on stdout.
#
# web/tools/nest.sh's headless dwl renders with pixman (the user's wlroots is built without
# GLES/Vulkan) and offers no zwp_linux_dmabuf_v1, so the shell inside it renders with llvmpipe
# and cannot import dmabufs: the dmabuf path cannot be exercised there. This helper runs
# KWin's virtual backend instead (`kwin_wayland --virtual`): no display, no window on the
# desktop, but it renders on the GPU (GBM on the render node) and offers linux-dmabuf v5, like
# the user's own Plasma desktop. It runs in its own D-Bus session (dbus-run-session), so it
# never talks to the user's session, and in a memory-capped scope (stop: --stop).
# No input injection (web/tools/inject needs wlroots' virtual keyboard/pointer): use it for
# page loads and measurements; input checks stay on nest.sh.
#
# Usage: web/tools/gpu-nest.sh [--stop | --shot FILE.png]
#   --stop   stop the scope (kwin, its D-Bus session and the portals it started)
#   --shot   screenshot of the whole virtual screen (spectacle over KWin's ScreenShot2,
#            through the nest's own D-Bus session)
# Env: GPU_NEST_SIZE WxH (default 960x1006, as nest.sh)
# Registry: <worktree>/.qwork/gpu-nest (unit, disp, dbus, kwin.log).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(git -C "$HERE" rev-parse --show-toplevel 2>/dev/null || echo "$HERE/../..")
REG=$ROOT/.qwork/gpu-nest
TAG=$(echo "$ROOT" | sha256sum | cut -c1-8)
UNIT=web-gpu-nest-$TAG
DISP=wayland-gpu-$TAG
SIZE=${GPU_NEST_SIZE:-960x1006}
mkdir -p "$REG"
running() { systemctl --user is-active -q "$UNIT.scope" 2>/dev/null; }
case "${1:-}" in
  --stop)
    systemctl --user stop "$UNIT.scope" 2>/dev/null || true
    rm -f "$REG/disp" "$REG/dbus"
    exit 0;;
  --shot)
    running || { echo "gpu-nest.sh: not running" >&2; exit 1; }
    [ -n "${2:-}" ] || { echo "gpu-nest.sh: --shot FILE.png" >&2; exit 2; }
    env -u DISPLAY WAYLAND_DISPLAY="$DISP" DBUS_SESSION_BUS_ADDRESS="$(cat "$REG/dbus")" \
      timeout 20 spectacle --background --nonotify --fullscreen --output "$2" >>"$REG/shot.log" 2>&1
    exit 0;;
esac
if running && [ -S "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/$DISP" ]; then
  echo "$DISP"; exit 0
fi
systemctl --user stop "$UNIT.scope" 2>/dev/null || true
rm -f "$REG/disp" "$REG/dbus"
W=${SIZE%x*}; H=${SIZE#*x}
(
  for f in /proc/self/fd/*; do   # as nest.sh: keep no caller fd (e.g. the perf lock) open
    n=${f##*/}
    if [ "$n" -gt 2 ] 2>/dev/null; then eval "exec $n>&-" 2>/dev/null || true; fi
  done
  exec env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u DBUS_SESSION_BUS_ADDRESS \
    KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1 \
    systemd-run --user --scope -q --unit="$UNIT" -p MemoryMax=6G -p MemorySwapMax=0 \
    timeout 3600 dbus-run-session -- sh -c 'echo "$DBUS_SESSION_BUS_ADDRESS" > "$0/dbus"; exec "$@"' "$REG" \
    kwin_wayland --virtual --socket "$DISP" --width "$W" --height "$H" \
    --no-lockscreen --no-global-shortcuts --no-kactivities >"$REG/kwin.log" 2>&1
) &
i=0
while ! [ -S "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/$DISP" ] || ! [ -s "$REG/dbus" ]; do
  i=$((i+1))
  if [ $i -gt 100 ]; then echo "gpu-nest.sh: kwin did not start, see $REG/kwin.log" >&2; exit 1; fi
  sleep 0.1
done
echo "$DISP" > "$REG/disp"
echo "$DISP"
