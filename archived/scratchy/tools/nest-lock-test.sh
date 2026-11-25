#!/bin/bash
# QTOOL1 test: a nested dwl started by nest.sh must not inherit the caller's lock fd.
# Holds a (private) lock on fd 9, runs nest.sh, releases its own lock, then the lock
# must be takeable while dwl still runs. Needs a Wayland session (nested dwl). Kills only its dwl PID.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
D=$(mktemp -d "${TMPDIR:-/tmp}/nest-lock-test.XXXXXX")
LOCK=$D/test.lock
exec 9>"$LOCK"
flock 9
"$HERE/nest.sh" "$D/nest" >/dev/null || { echo FAIL: nest.sh; exit 1; }
PID=$(cat "$D/nest/dwl.pid")
exec 9>&-   # close only (flock -u would release every dup, hiding the bug)
rc=0
if flock -n "$LOCK" true; then echo "PASS: lock free while dwl ($PID) runs"
else echo "FAIL: dwl holds the lock"; rc=1; fi
# only the PIDs we started (timeout, and its dwl child)
kill $(pgrep -P "$PID") "$PID" 2>/dev/null
rm -rf "$D"
exit $rc
