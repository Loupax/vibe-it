#!/usr/bin/env bash
# QOBS1: one-command sampling profile (perf + V8's JIT map). PERF.md "Profiling (perf)".
#
#   tools/profile.sh [--seconds N] [--ua UA] <url|file>
#
# Builds the `profiling` cargo profile (release + line tables) with frame pointers into
# ${PROF_TARGET:-$HOME/.cache/scratchy-prof-target} (never the repo's target/), runs
# `perf record -e cycles:u -F 999 -g` on `--perf-report --budget-ms 20000`, and writes
# perf.data, report.json, folded.txt, summary.txt (and flame.svg if inferno-flamegraph is
# on PATH) into perf/profile-<commit>-<slug>/ (gitignored).
# Environment: WIRE_SOCKET (default /run/user/$UID/wire.sock), PROF_TARGET,
# RUSTY_V8_ARCHIVE (reused when set; else the local gn_out archive if present).
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
SECS=30
UA=""
TARGET_ARG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --seconds) SECS="${2:?--seconds needs a number}"; shift 2 ;;
    --ua) UA="${2:?--ua needs a string}"; shift 2 ;;
    -h|--help) sed -n 2,13p "$0"; exit 0 ;;
    -*) echo "profile.sh: unknown option $1" >&2; exit 2 ;;
    *) TARGET_ARG="$1"; shift ;;
  esac
done
[ -n "$TARGET_ARG" ] || { echo "usage: tools/profile.sh [--seconds N] [--ua UA] <url|file>" >&2; exit 2; }

# Preflight (never runs sudo).
if ! command -v perf >/dev/null 2>&1; then
  echo "profile.sh: perf not found: install perf (dnf install perf)" >&2; exit 2
fi
PARANOID="$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo 4)"
if [ "$PARANOID" -gt 1 ]; then
  echo "profile.sh: perf_event_paranoid is $PARANOID (call graphs need <= 1); run (not persistent): echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid" >&2
  exit 2
fi
command -v python3 >/dev/null 2>&1 || { echo "profile.sh: python3 not found" >&2; exit 2; }

SOCK="${WIRE_SOCKET:-/run/user/$(id -u)/wire.sock}"
case "$TARGET_ARG" in
  http://*|https://*) PAGE="$TARGET_ARG"; SLUG_SRC="$TARGET_ARG"
    [ -S "$SOCK" ] || { echo "profile.sh: no wire socket at $SOCK (set WIRE_SOCKET)" >&2; exit 2; } ;;
  *) [ -f "$TARGET_ARG" ] || { echo "profile.sh: no such file: $TARGET_ARG" >&2; exit 2; }
     PAGE="$(realpath "$TARGET_ARG")"; SLUG_SRC="$(basename "$TARGET_ARG")" ;;
esac

PROF_TARGET="${PROF_TARGET:-$HOME/.cache/scratchy-prof-target}"
LOCAL_V8="$HERE/target/release/build/gn_out/obj/librusty_v8.a"
if [ -z "${RUSTY_V8_ARCHIVE:-}" ] && [ -f "$LOCAL_V8" ]; then
  export RUSTY_V8_ARCHIVE="$LOCAL_V8"
fi

echo "profile.sh: building (profiling profile) into $PROF_TARGET" >&2
(cd "$HERE" && CARGO_TARGET_DIR="$PROF_TARGET" RUSTFLAGS="-C force-frame-pointers=yes" \
  cargo build --profile profiling >&2)
BIN="$PROF_TARGET/profiling/scratchy"
[ -x "$BIN" ] || { echo "profile.sh: build produced no $BIN" >&2; exit 1; }

COMMIT="$(git -C "$HERE" rev-parse --short HEAD 2>/dev/null || echo nocommit)"
SLUG="$(printf '%s' "$SLUG_SRC" | sed -E 's#^https?://##; s#[^A-Za-z0-9]+#-#g; s#^-+|-+$##g' | cut -c1-40)"
OUT="$HERE/perf/profile-$COMMIT-${SLUG:-page}"
mkdir -p "$OUT"
FOLD="$HERE/tools/perf-fold.py"

# V8's --perf-basic-prof writes /tmp/perf-<pid>.map; remember the existing ones so only
# this run's are removed afterwards.
BEFORE="$(ls /tmp 2>/dev/null | grep -E '^perf-[0-9]+\.map$' || true)"

ARGS=(--perf-report --budget-ms 20000 --seconds "$SECS")
[ -S "$SOCK" ] && ARGS+=(--socket "$SOCK")
[ -z "$UA" ] || ARGS+=(--user-agent "$UA")

echo "profile.sh: recording $SECS s of $PAGE" >&2
# cwd = $OUT because
# the V8 flags also write isolate-0x...-v8.log files into the CWD.
(cd "$OUT" && SCRATCHY_V8_FLAGS='--perf-basic-prof --interpreted-frames-native-stack' \
  perf record -e cycles:u -F 999 -g -o perf.data -- "$BIN" "${ARGS[@]}" "$PAGE" > report.json 2> run.err) \
  || { echo "profile.sh: perf record failed, see $OUT/run.err" >&2; exit 1; }

(cd "$OUT" && perf script -i perf.data -F comm,tid,ip,sym 2>/dev/null | python3 -I "$FOLD" fold folded.txt)
python3 -I "$FOLD" summary "$OUT/folded.txt" > "$OUT/summary.txt"
if command -v inferno-flamegraph >/dev/null 2>&1; then
  inferno-flamegraph < "$OUT/folded.txt" > "$OUT/flame.svg" 2>/dev/null || true
fi

# Clean up this run's JIT maps and isolate logs, by exact name.
AFTER="$(ls /tmp 2>/dev/null | grep -E '^perf-[0-9]+\.map$' || true)"
for f in $AFTER; do
  case " $(echo $BEFORE) " in *" $f "*) ;; *) rm -f -- "/tmp/${f:?}" ;; esac
done
for f in $(cd "$OUT" && ls | grep -E '^isolate-.*-v8\.log$' || true); do rm -f -- "${OUT:?}/${f:?}"; done

echo "profile.sh: done: $OUT" >&2
echo "  summary.txt folded.txt report.json perf.data; query: python3 -I tools/perf-fold.py incl|callers|jscallers $OUT/folded.txt REGEX" >&2
sed -n '/== top 40 inclusive/,/^$/p' "$OUT/summary.txt" | head -12
