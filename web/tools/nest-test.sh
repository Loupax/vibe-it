#!/bin/bash
# NEST-HEADLESS: checks web/tools/nest.sh's default (headless) mode on a registry of its own:
# it starts, records an Xwayland DISPLAY (--x11-display), has none of the session's display variables, is reused by a second call,
# screenshots at NEST_SIZE with grim, takes injected input without error (needs
# web/tools/inject built: cd web/tools/inject && cargo build --release; skipped otherwise),
# and --stop ends it. Exit 0 when all pass. Opens no window anywhere.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(git -C "$HERE" rev-parse --show-toplevel)
export NEST_REG=$ROOT/.qwork/nest-test
export NEST_SIZE=640x480
fail=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; fail=1; }
"$HERE/nest.sh" --stop
disp=$("$HERE/nest.sh" "$NEST_REG/out") || { echo "FAIL: nest.sh did not start"; exit 1; }
pid=$(cat "$NEST_REG/dwl.pid")
[ "$(cat "$NEST_REG/mode")" = headless ] && ok "mode headless ($disp)" || bad "mode is $(cat "$NEST_REG/mode")"
dwl=$(pgrep -P "$pid" | head -1)
if tr '\0' '\n' <"/proc/${dwl:-$pid}/environ" | grep -qE '^(WAYLAND_DISPLAY|WAYLAND_SOCKET|DISPLAY)='; then
  bad "dwl has a display variable of the session"
else
  ok "dwl environment has no WAYLAND_DISPLAY/WAYLAND_SOCKET/DISPLAY"
fi
[ "$("$HERE/nest.sh" "$NEST_REG/out")" = "$disp" ] && [ "$(cat "$NEST_REG/dwl.pid")" = "$pid" ] && ok "second call reuses it" || bad "second call started another"
x=$("$HERE/nest.sh" --x11-display); [ -n "$x" ] && ok "--x11-display: $x (Xwayland in the headless dwl)" || bad "--x11-display printed nothing"
shot=$NEST_REG/shot.png
if WAYLAND_DISPLAY=$disp grim "$shot" && file "$shot" | grep -q "640 x 480"; then ok "grim screenshot 640x480"; else bad "grim: $(file "$shot")"; fi
inj=$ROOT/web/tools/inject/target/release/inject
if [ -x "$inj" ]; then
  WAYLAND_DISPLAY=$disp "$inj" 640 480 move:10,10 click hold:c key:a release type:ab && ok "inject ran" || bad "inject failed"
else
  echo "skip: inject not built"
fi
"$HERE/nest.sh" --stop
[ -d "/proc/$pid" ] && bad "--stop left PID $pid" || ok "--stop ended it"
rm -f "$shot" "$NEST_REG/out/dwl.pid" "$NEST_REG/out/disp"
exit $fail
