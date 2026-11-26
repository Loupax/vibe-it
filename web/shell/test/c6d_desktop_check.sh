#!/bin/bash
# C6d: the dmabuf check on the user's REAL desktop (agents never run this: it opens windows
# on the session's display). One command, about two minutes, prints PASS/FAIL and numbers:
#
#     web/shell/test/c6d_desktop_check.sh
#
# Needs release builds (it says how if they are missing) and a Wayland session (Plasma here).
# 1. one scroll run with WAYLAND_DEBUG=server: the shell must say "dmabuf: offered" and
#    "first dmabuf imported", and the page must have made dmabuf buffers (not only wl_shm);
# 2. css/video/scroll with dmabuf, then the same with WEB_NO_DMABUF=1 (the old wl_shm path):
#    shell CPU and the engine's GPU-process / renderer / total CPU, % of one core.
# Every run is memory-capped (6G) and gets its own state dir under .qwork/c6d-desktop
# (never ~/.local/state). Several shell windows open and close one after another; leave them be.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(git -C "$HERE" rev-parse --show-toplevel)
WEB=${WEB_BIN:-$ROOT/web/shell/target/release/web}
HOST=${WEB_HOST_BIN:-$ROOT/web/host-cef/target/release/web-host-cef}
OUT=$ROOT/.qwork/c6d-desktop
if [ -z "$WAYLAND_DISPLAY" ]; then
  echo "c6d_desktop_check: no WAYLAND_DISPLAY (run it in the desktop session)" >&2; exit 2
fi
for b in "$WEB" "$HOST"; do
  if [ ! -x "$b" ]; then
    echo "c6d_desktop_check: $b is missing; build first:" >&2
    echo "  (cd web/shell && cargo build --release)" >&2
    echo "  (cd web/host-cef && CEF_PATH=\$HOME/.cache/web-spike/cef cargo build --release)" >&2
    exit 2
  fi
done
rm -rf "$OUT"
mkdir -p "$OUT"
# Empty config/state/data homes: the shells must not read the user's real init.js (C-LIVE-XDG).
XDG_TMP=$(mktemp -d "${TMPDIR:-/tmp}/wxdg-XXXXXX")
trap 'rm -rf "$XDG_TMP"' EXIT
export XDG_CONFIG_HOME=$XDG_TMP/c XDG_STATE_HOME=$XDG_TMP/s XDG_DATA_HOME=$XDG_TMP/d
mkdir -p "$XDG_CONFIG_HOME" "$XDG_STATE_HOME" "$XDG_DATA_HOME"
bench() { python3 "$HERE/c6c_bench.py" --web "$WEB" --host-bin "$HOST" --wayland "$WAYLAND_DISPLAY" "$@"; }
echo "== 1. does the page hand dmabufs? ($(uptime))"
line=$(bench --out "$OUT/check" --modes scroll --wayland-debug | tail -1)
echo "$line"
log=$OUT/check/web-scroll.log
grep -E '^\[web \+[0-9]+ms\] dmabuf:' "$log" || true
ok=1
grep -q 'dmabuf: offered' "$log" || { echo "FAIL: the shell did not offer dmabuf (see the 'dmabuf: not offered' reason above)"; ok=0; }
grep -q 'first dmabuf imported' "$log" || { echo "FAIL: no dmabuf from the page was imported"; ok=0; }
case "$line" in
  *" 0 dmabufs"*) echo "FAIL: the page made no dmabuf buffers"; ok=0;;
esac
if grep -q 'dmabuf: import failed' "$log"; then echo "FAIL: an import failed:"; grep 'dmabuf: import failed' "$log"; ok=0; fi
echo "== 2. CPU with dmabuf ($(uptime))"
bench --out "$OUT/on" --modes css,video,scroll
echo "== 3. CPU with wl_shm (WEB_NO_DMABUF=1) ($(uptime))"
WEB_NO_DMABUF=1 bench --out "$OUT/off" --modes css,video,scroll
if [ $ok = 1 ]; then echo "PASS: dmabuf offered, imported and used (logs in $OUT)"; else echo "FAIL (logs in $OUT)"; exit 1; fi
