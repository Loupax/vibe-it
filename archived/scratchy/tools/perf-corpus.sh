#!/usr/bin/env bash
# QV8-0: offline engine corpus. Downloads the PINNED library files (never
# committed, AGENTS.md: no vendoring) through the user's wire socket, verifies
# their sha256, then runs `scratchy --perf-corpus <dir>` and prints its JSON.
#
#   tools/perf-corpus.sh [dir]        dir defaults to a fresh temp directory
#   SCRATCHY=path/to/scratchy         binary (default: target/release/scratchy)
#   WIRE_SOCKET=path                  wire socket (default: /run/user/$UID/wire.sock)
#   FETCH_ONLY=1                      download + verify, do not run scratchy
#   RECORD=1                          print "file sha256" lines instead of verifying
#                                     (only for re-pinning: edit the table below)
#
# Downloads use `curl --unix-socket` with an absolute-form request target, the
# wire convention (wire/README.md). No User-Agent is sent by curl (-H
# 'User-Agent:'), so wire uses its own honest default. QV8-m runs this same
# table, so both engines see identical bytes.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
SOCK="${WIRE_SOCKET:-/run/user/$(id -u)/wire.sock}"
BIN="${SCRATCHY:-$HERE/target/release/scratchy}"
DIR="${1:-$(mktemp -d "${TMPDIR:-/tmp}/scratchy-corpus.XXXXXX")}"
mkdir -p "$DIR"

# file | URL | sha256   (react is 18.x: 19 has no UMD build; three 0.159.0 is the
# last release that ships build/three.min.js)
TABLE='
moment-2.31.0.min.js|https://cdn.jsdelivr.net/npm/moment@2.31.0/min/moment.min.js|c111ad2e4447c253765e679bb4a40ba91a3e23d80b4b69d603a0fd25164cb9de
d3-7.9.2.min.js|https://cdn.jsdelivr.net/npm/d3@7.9.2/dist/d3.min.js|912111098ea99503a6f4967361d2bbdc9dbf58cf9c678c8e27e5b02d1a07f0c1
lodash-4.18.1.min.js|https://cdn.jsdelivr.net/npm/lodash@4.18.1/lodash.min.js|a8d7e6291ad80256f976ace90824a71018d2f706992c9107b20bdced97bee27b
three-0.159.0.min.js|https://cdn.jsdelivr.net/npm/three@0.159.0/build/three.min.js|7b1c5d75b28d9de15042e2b374f83566d8c7146697af8fdeb4558b0fb528a585
jquery-4.1.0.min.js|https://cdn.jsdelivr.net/npm/jquery@4.1.0/dist/jquery.min.js|86346f3e414b3b52466453171abcdda46e9396c1c3ca3a783d07aa2d645c3339
react-18.3.1.min.js|https://cdn.jsdelivr.net/npm/react@18.3.1/umd/react.production.min.js|d949f1c3687aedadcedac85261865f29b17cd273997e7f6b2bfc53b2f9d4c4dd
react-dom-18.3.1.min.js|https://cdn.jsdelivr.net/npm/react-dom@18.3.1/umd/react-dom.production.min.js|35f4f974f4b2bcd44da73963347f8952e341f83909e4498227d4e26b98f66f0d
'

[ -S "$SOCK" ] || { echo "perf-corpus: no wire socket at $SOCK" >&2; exit 2; }
bad=0
while IFS='|' read -r file url sha; do
  [ -n "$file" ] || continue
  out="$DIR/$file"
  if [ ! -s "$out" ]; then
    curl -sS --fail --unix-socket "$SOCK" --request-target "$url" -H 'User-Agent:' http://localhost/ -o "$out" \
      || { echo "perf-corpus: download failed: $url" >&2; rm -f "$out"; exit 2; }
  fi
  got="$(sha256sum "$out" | cut -d' ' -f1)"
  if [ -n "${RECORD:-}" ]; then echo "$file $got"; continue; fi
  if [ "$got" != "$sha" ]; then
    echo "perf-corpus: sha256 mismatch for $file: expected $sha got $got" >&2
    bad=1
  fi
done <<< "$TABLE"
[ "$bad" = 0 ] || exit 3
[ -z "${RECORD:-}" ] || exit 0
[ -z "${FETCH_ONLY:-}" ] || { echo "$DIR"; exit 0; }
"$BIN" --perf-corpus "$DIR"
