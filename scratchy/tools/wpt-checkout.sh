#!/bin/sh
# Blobless, depth-1, sparse checkout of the pinned WPT commit (R64 2.1).
# Usage (from scratchy/): tools/wpt-checkout.sh DIR
# Pin: tools/wpt-pin.txt. Directories: tools/wpt-dirs.txt. Refuses a checkout whose HEAD differs from the pin.
set -eu
dir="${1:?usage: tools/wpt-checkout.sh DIR}"
here="$(cd "$(dirname "$0")" && pwd)"
pin="$(tr -d '[:space:]' < "$here/wpt-pin.txt")"
dirs="$(grep -v '^[[:space:]]*#' "$here/wpt-dirs.txt" | grep -v '^[[:space:]]*$')"
if [ ! -d "$dir/.git" ]; then
  git clone -q --filter=blob:none --no-checkout --depth 1 --sparse \
    https://github.com/web-platform-tests/wpt "$dir"
fi
if [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null || true)" != "$pin" ]; then
  git -C "$dir" fetch -q --depth 1 --filter=blob:none origin "$pin"
fi
# shellcheck disable=SC2086
git -C "$dir" sparse-checkout set $dirs
git -C "$dir" checkout -q --detach "$pin"
head="$(git -C "$dir" rev-parse HEAD)"
if [ "$head" != "$pin" ]; then
  echo "wpt-checkout: $dir is at $head, the pin is $pin" >&2
  exit 1
fi
echo "wpt-checkout: $dir at $pin"
