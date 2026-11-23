#!/bin/bash
# Run web-host-cef in a memory-capped systemd scope (covers every CEF child process).
# For development and the C1 check; the shell (C2) will start the host itself.
# Usage: web/host-cef/run.sh [--x11] [host options and Chromium switches...]
#   e.g. run.sh --url https://example.com/
#        run.sh --proxy 127.0.0.1:3128 --control /path/control.sock
#   Wayland by default (needs WAYLAND_DISPLAY); --x11 uses DISPLAY (e.g. a nested Xwayland).
#   No network namespace: web enforces nothing (web/DECISIONS.md "No enforcement in web").
# Env:
#   WEB_HOST_PROFILE  throwaway profile/cache dir (default: web/host-cef/.qwork/profile, git-ignored)
#   WEB_HOST_UNIT     systemd scope name (default: web-host-cef-<pid>); stop the whole tree with
#                     `systemctl --user stop <unit>.scope`
#   WEB_HOST_BIN      binary (default: web/host-cef/target/debug/web-host-cef)
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${WEB_HOST_BIN:-$HERE/target/debug/web-host-cef}
PROFILE=${WEB_HOST_PROFILE:-$HERE/.qwork/profile}
UNIT=${WEB_HOST_UNIT:-web-host-cef-$$}
OZONE=wayland
if [ "${1:-}" = --x11 ]; then OZONE=x11; shift; fi
mkdir -p "$PROFILE"
# No core files: a crashed renderer (chrome://crash) otherwise stalls for ~45 s while
# systemd-coredump takes its dump, and the crash is only reported after that.
ulimit -c 0
echo "run.sh: unit $UNIT.scope, profile $PROFILE, ozone $OZONE" >&2
# FLATPAK_SANDBOX_DIR: without it Chromium's xdg-portal setup asks systemd (D-Bus
# StartTransientUnit) to move the browser process into its own
# app-org.chromium.Chromium-<pid>.scope, i.e. OUT of this memory-capped scope.
# components/dbus/xdg/systemd.cc skips that when this variable (or SNAP) is set;
# nothing else in libcef reads it (web/spike/README.md, W1).
exec systemd-run --user --scope -q --unit="$UNIT" -p MemoryMax=6G -p MemorySwapMax=0 \
  env FLATPAK_SANDBOX_DIR="$PROFILE" \
  "$BIN" --profile="$PROFILE" --ozone-platform="$OZONE" "$@"
