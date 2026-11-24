#!/bin/bash
# C-WALLET: start web-host-cef on a PRIVATE D-Bus (private-bus.conf: no service activation) with
# optional fake KWallet / Secret Service on it, record every D-Bus message the host sends
# (dbus-monitor on the private bus), and show how the cookie was stored.
# Usage: WAYLAND_DISPLAY=wayland-N web/tools/wallet-probe/run.sh OUT_DIR TAG FAKES [SECONDS] [host args...]
#   FAKES: none | kwallet | secrets | both   (what the fake bus offers)
#   env XDG_CURRENT_DESKTOP=KDE|GNOME|... is passed through (Chromium picks the backend from it);
#   WEB_HOST_BIN: host binary (default: the installed ~/.local/lib/web/current/web-host-cef)
#   Host args go last, e.g. --password-store=basic. Profile: OUT_DIR/profile-TAG (reused if present:
#   a second run with another store shows what happens to cookies written by the first).
# Results: OUT/TAG.http.log (Cookie header seen by the server), OUT/TAG.dbus.log (monitor), TAG.fake.log, TAG.host.log, TAG.cookies.txt (sqlite dump).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$(realpath -m "$1"); TAG=$2; FAKES=$3; SECS=${4:-20}; shift 4 || shift $#
mkdir -p "$OUT"
if [ -z "$INNER" ]; then
  exec env INNER=1 dbus-run-session --config-file="$HERE/../notify-probe/private-bus.conf" -- "$0" "$OUT" "$TAG" "$FAKES" "$SECS" "$@"
fi
case "$DBUS_SESSION_BUS_ADDRESS" in *"/run/user/"*"/bus"*) echo "refusing: user's session bus" >&2; exit 1;; esac
echo "private bus: $DBUS_SESSION_BUS_ADDRESS (XDG_CURRENT_DESKTOP=$XDG_CURRENT_DESKTOP)" >&2
: > "$OUT/$TAG.dbus.log"; : > "$OUT/$TAG.fake.log"
dbus-monitor --session > "$OUT/$TAG.dbus.log" 2>&1 & MPID=$!
python3 -I "$HERE/fake_wallet.py" "$OUT/$TAG.fake.log" "$FAKES" & FPID=$!
PORT=$((20000 + RANDOM % 20000))
python3 -I "$HERE/serve_cookie.py" "$PORT" "$OUT/$TAG.http.log" & SPID=$!
sleep 1
UNIT=web-host-cef-wallet-$$
PROFILE=${WEB_HOST_PROFILE:-$OUT/profile-$TAG}
export WEB_HOST_BIN=${WEB_HOST_BIN:-$HOME/.local/lib/web/current/web-host-cef}
WEB_HOST_UNIT=$UNIT WEB_HOST_PROFILE=$PROFILE "$HERE/../../host-cef/run.sh" \
  --url "http://127.0.0.1:$PORT/" "$@" > "$OUT/$TAG.host.log" 2>&1 & HPID=$!
sleep "$SECS"
systemctl --user stop "$UNIT.scope" 2>/dev/null || true
wait $HPID 2>/dev/null || true
kill $SPID $FPID $MPID 2>/dev/null || true
wait 2>/dev/null || true
python3 -I - "$PROFILE" > "$OUT/$TAG.cookies.txt" <<'PY'
import sqlite3, sys, glob
for f in glob.glob(sys.argv[1] + '/Default/Cookies') + glob.glob(sys.argv[1] + '/Default/Network/Cookies'):
    c = sqlite3.connect('file:%s?mode=ro' % f, uri=True)
    for n, v, e in c.execute('select name, value, encrypted_value from cookies'):
        print(f, n, 'plain=%r' % v, 'enc_prefix=%r' % bytes(e[:3]), 'enc_len=%d' % len(e))
PY
cat "$OUT/$TAG.cookies.txt"
echo "== Cookie header the page request carried (readable stored cookie = decryption worked)"; cat "$OUT/$TAG.http.log"
echo "== fake log"; cat "$OUT/$TAG.fake.log"
echo "== dbus calls to wallet-ish names"; grep -E 'member=|destination=' "$OUT/$TAG.dbus.log" | grep -iE 'wallet|secret|keyring' | head -40
