#!/bin/bash
# C-BELL spike: load probe.html in web-host-cef on a PRIVATE D-Bus session bus with a fake
# notification server on it, so nothing can reach the user's desktop notifications and the run
# shows whether Chromium calls org.freedesktop.Notifications.
# Usage: WAYLAND_DISPLAY=wayland-N web/tools/notify-probe/run.sh OUT_DIR TAG [SECONDS] [host args...]
#   (start the nested compositor first: web/tools/nest.sh). The host's own options go after
#   SECONDS (default 25), e.g. `--notify-sites=127.0.0.1` (C-BELL). Env PROBE_QUERY is appended to the
#   page's query (`&burst=8`, `&delay=3000`). Results: OUT_DIR/TAG.json (the
#   page's findings), OUT_DIR/TAG.host.log, OUT_DIR/TAG.notifyd.log, OUT_DIR/TAG.http.log.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$(realpath -m "$1"); TAG=$2; SECS=${3:-25}; shift 3 || shift $#
mkdir -p "$OUT"
if [ -z "$INNER" ]; then
  # Re-run inside a fresh private session bus (dbus-run-session ends it when we exit).
  # private-bus.conf: no service activation, or the bus starts the real xdg-desktop-portal and
  # the first page load hangs (seen with the default session config).
  exec env INNER=1 dbus-run-session --config-file="$HERE/private-bus.conf" -- "$0" "$OUT" "$TAG" "$SECS" "$@"
fi
case "$DBUS_SESSION_BUS_ADDRESS" in *"/run/user/"*"/bus"*) echo "refusing: user's session bus" >&2; exit 1;; esac
echo "private bus: $DBUS_SESSION_BUS_ADDRESS" >&2
python3 -I "$HERE/fake_notifyd.py" "$OUT/$TAG.notifyd.log" & NPID=$!
PORT=$((20000 + RANDOM % 20000))
python3 -I "$HERE/serve.py" "$PORT" "$OUT" > "$OUT/$TAG.http.log" 2>&1 & SPID=$!
sleep 1
UNIT=web-host-cef-notify-$$
PROFILE=${WEB_HOST_PROFILE:-$OUT/profile-$TAG}
WEB_HOST_UNIT=$UNIT WEB_HOST_PROFILE=$PROFILE "$HERE/../../host-cef/run.sh" \
  --url "http://127.0.0.1:$PORT/probe.html?tag=$TAG${PROBE_QUERY:-}" "$@" > "$OUT/$TAG.host.log" 2>&1 & HPID=$!
sleep $((SECS / 2))
grim "$OUT/$TAG.png" 2>/dev/null || true   # what the nested output shows mid-run (a prompt?)
sleep $((SECS - SECS / 2))
systemctl --user stop "$UNIT.scope" 2>/dev/null || true
wait $HPID 2>/dev/null || true
kill $SPID $NPID 2>/dev/null || true
wait 2>/dev/null || true
cp "$PROFILE/cef.log" "$OUT/$TAG.cef.log" 2>/dev/null || true
echo "== $TAG.json"; cat "$OUT/$TAG.json" 2>/dev/null || echo "(no result)"; echo
echo "== notifyd"; cat "$OUT/$TAG.notifyd.log"
