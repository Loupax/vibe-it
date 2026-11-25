#!/bin/bash
# QNEST1 test: nest.sh keeps ONE nested dwl per worktree. Two calls (with different OUT dirs) give
# the same socket and PID; --new replaces it; --stop leaves none; a stale pid file is cleaned.
# Uses a private registry (NEST_REG) and the headless backend when there is no Wayland session.
# Kills only the PIDs recorded in its registry.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
D=$(mktemp -d "${TMPDIR:-/tmp}/nest-one-test.XXXXXX")
export NEST_REG=$D/reg NEST_TIMEOUT=120
[ -z "${WAYLAND_DISPLAY:-}" ] && export NEST_BACKEND=headless
rc=0
ck() { if eval "$2"; then echo "PASS: $1"; else echo "FAIL: $1"; rc=1; fi; }
alive() { [ -r "/proc/$1/cmdline" ]; }
S1=$("$HERE/nest.sh" "$D/a") || { echo "FAIL: first nest.sh"; rm -rf "$D"; exit 1; }
P1=$(cat "$D/a/dwl.pid")
S2=$("$HERE/nest.sh" "$D/b") || echo "FAIL: second nest.sh"
P2=$(cat "$D/b/dwl.pid" 2>/dev/null)
ck "same socket on second call ($S1)" '[ "$S1" = "$S2" ]'
ck "same PID on second call ($P1)" '[ "$P1" = "$P2" ]'
ck "one live dwl pid file" '[ "$(cat "$NEST_REG/dwl.pid")" = "$P1" ] && alive "$P1"'
S3=$("$HERE/nest.sh" --new "$D/c") || echo "FAIL: --new"
P3=$(cat "$D/c/dwl.pid" 2>/dev/null)
ck "--new gives a different PID" '[ -n "$P3" ] && [ "$P3" != "$P1" ]'
ck "--new stopped the old one" '! alive "$P1"'
"$HERE/nest.sh" --stop
ck "--stop: process gone" '! alive "$P3"'
ck "--stop: pid file removed" '[ ! -e "$NEST_REG/dwl.pid" ]'
echo 999999999 > "$NEST_REG/dwl.pid"
S4=$("$HERE/nest.sh" "$D/d") || echo "FAIL: after stale pid file"
P4=$(cat "$D/d/dwl.pid" 2>/dev/null)
ck "stale pid file replaced by a fresh dwl" '[ -n "$P4" ] && [ "$P4" != 999999999 ] && alive "$P4"'
"$HERE/nest.sh" --stop
ck "final --stop leaves none" '! alive "$P4"'
rm -rf "$D"
exit $rc
