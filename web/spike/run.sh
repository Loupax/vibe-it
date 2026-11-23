#!/bin/bash
# Run the web spike in a memory-capped systemd scope (covers every CEF child process).
# Usage: web/spike/run.sh [--x11] [--net] [URL] [extra Chromium/CEF switches...]
#   The whole process tree runs in an EMPTY network namespace (`unshare -rn`) by default;
#   --net opts out (host network, for debugging only).
#   Wayland by default (needs WAYLAND_DISPLAY); --x11 uses DISPLAY (e.g. a nested Xwayland).
# Env:
#   WEB_SPIKE_PROFILE  throwaway profile/cache dir (default: web/spike/.profile, git-ignored)
#   WEB_SPIKE_UNIT     systemd scope name (default: web-spike-<pid>); stop the whole tree with
#                      `systemctl --user stop <unit>.scope`
#   WEB_SPIKE_BIN      binary (default: web/spike/target/debug/web-spike)
#   WEB_SPIKE_WIRE     wire's Unix socket (W3): every http(s) request is answered from it.
#                      Unset: no interception, so nothing loads inside the namespace.
#                      Run your OWN wire outside the namespace, never the user's.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${WEB_SPIKE_BIN:-$HERE/target/debug/web-spike}
PROFILE=${WEB_SPIKE_PROFILE:-$HERE/.profile}
UNIT=${WEB_SPIKE_UNIT:-web-spike-$$}
OZONE=wayland
NETNS=1
while :; do
  case "${1:-}" in
    --x11) OZONE=x11; shift ;;
    --net) NETNS=0; shift ;;
    *) break ;;
  esac
done
URL=${1:-https://example.com/}
[ $# -gt 0 ] && shift
mkdir -p "$PROFILE"
echo "run.sh: unit $UNIT.scope, profile $PROFILE, ozone $OZONE, netns $NETNS, wire ${WEB_SPIKE_WIRE:-none}" >&2
WRAP=()
# Order matters: the systemd scope (memory cap) wraps the namespace, which wraps the binary.
# unshare -r maps us to root in a new user namespace (needed to create the net namespace
# unprivileged); -n gives an empty one (only a DOWN `lo`). File-path sockets (D-Bus session
# bus, Wayland, X11) still work across network namespaces.
[ "$NETNS" = 1 ] && WRAP=(unshare -n --map-user="$(id -u)" --map-group="$(id -g)")
# FLATPAK_SANDBOX_DIR: without it Chromium's xdg-portal setup asks systemd (D-Bus
# StartTransientUnit) to move the browser process into its own
# app-org.chromium.Chromium-<pid>.scope, i.e. OUT of this memory-capped scope.
# components/dbus/xdg/systemd.cc skips that when this variable (or SNAP) is set;
# nothing else in libcef reads it.
exec systemd-run --user --scope -q --unit="$UNIT" -p MemoryMax=6G -p MemorySwapMax=0 \
  "${WRAP[@]}" env WEB_SPIKE_PROFILE="$PROFILE" FLATPAK_SANDBOX_DIR="$PROFILE" WEB_SPIKE_WIRE="${WEB_SPIKE_WIRE:-}" \
  "$BIN" --url="$URL" --ozone-platform="$OZONE" "$@"
