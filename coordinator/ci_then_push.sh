#!/bin/bash
# Wait for the newest main CI run; push local tracking commits and reinstall ONLY if it succeeded.
# usage: ci_then_push.sh [--install <crate dir>|none]   (default: scratchy; e.g. --install wire, --install wire2, --install none)
INSTALL=scratchy
[ "$1" = --install ] && INSTALL=$2
cd "$(dirname "$0")/.." || exit 2
id=$(gh run list --branch main --limit 1 --json databaseId -q '.[0].databaseId')
timeout 2700 gh run watch "$id" --exit-status >/dev/null 2>&1
res=$(gh run view "$id" --json headSha,conclusion -q '"\(.headSha[0:7]) \(.conclusion)"')
echo "main CI: $res"
case "$res" in
  *" success") ;;
  *) echo "NOT pushing, NOT installing: main CI is not green"; exit 1 ;;
esac
git fetch -q origin && git rebase -q origin/main || { echo "rebase onto origin/main FAILED"; exit 1; }
git push -q origin main || { sleep 15; git fetch -q origin && git rebase -q origin/main && git push -q origin main; } || echo "PUSH FAILED twice (local tracking commits kept)"
git log --oneline -1 origin/main
[ "$INSTALL" = none ] && { echo "--install none: skipping cargo install"; exit 0; }
[ -f "$INSTALL/Cargo.toml" ] || { echo "no crate at $INSTALL/: skipping cargo install"; exit 0; }
# Reinstall only on the user's machine (the V8 archive cache exists there); cloud sessions skip it.
V8A=$HOME/.cache/librusty_v8-152.2.0.a
if [ -f "$V8A" ]; then
  RUSTY_V8_ARCHIVE=$V8A timeout 3000 nice -n 19 ionice -c3 cargo install --path "$INSTALL" --force -j4 2>&1 | tail -1
else
  echo "not the user's machine: skipping cargo install"; exit 0
fi
ls -la --time-style=+%H:%M ~/.cargo/bin/"$(basename "$INSTALL")"
