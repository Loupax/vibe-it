#!/bin/bash
# Run the web spike in a memory-capped systemd scope (covers every CEF child process).
# Usage: web/spike/run.sh [--x11] [URL] [extra Chromium/CEF switches...]
#   Wayland by default (needs WAYLAND_DISPLAY); --x11 uses DISPLAY (e.g. a nested Xwayland).
# Env:
#   WEB_SPIKE_PROFILE  throwaway profile/cache dir (default: web/spike/.profile, git-ignored)
#   WEB_SPIKE_UNIT     systemd scope name (default: web-spike-<pid>); stop the whole tree with
#                      `systemctl --user stop <unit>.scope`
#   WEB_SPIKE_BIN      binary (default: web/spike/target/debug/web-spike)
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${WEB_SPIKE_BIN:-$HERE/target/debug/web-spike}
PROFILE=${WEB_SPIKE_PROFILE:-$HERE/.profile}
UNIT=${WEB_SPIKE_UNIT:-web-spike-$$}
OZONE=wayland
if [ "${1:-}" = --x11 ]; then OZONE=x11; shift; fi
URL=${1:-https://example.com/}
[ $# -gt 0 ] && shift
mkdir -p "$PROFILE"
echo "run.sh: unit $UNIT.scope, profile $PROFILE, ozone $OZONE" >&2
# FLATPAK_SANDBOX_DIR: without it Chromium's xdg-portal setup asks systemd (D-Bus
# StartTransientUnit) to move the browser process into its own
# app-org.chromium.Chromium-<pid>.scope, i.e. OUT of this memory-capped scope.
# components/dbus/xdg/systemd.cc skips that when this variable (or SNAP) is set;
# nothing else in libcef reads it.
exec systemd-run --user --scope -q --unit="$UNIT" -p MemoryMax=6G -p MemorySwapMax=0 \
  env WEB_SPIKE_PROFILE="$PROFILE" FLATPAK_SANDBOX_DIR="$PROFILE" \
  "$BIN" --url="$URL" --ozone-platform="$OZONE" "$@"
