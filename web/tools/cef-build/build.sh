#!/bin/sh
# Build a CEF binary distribution with proprietary codecs (H.264/AAC in <video>/MSE, WebRTC
# H.264 via OpenH264) for web-host-cef, and lay it out where cef-dll-sys finds it via CEF_PATH.
# The output is for this machine only: NEVER commit, upload or publish it (codec patents,
# web/DECISIONS.md 2026-11-23 "H.264: build CEF ourselves"). See README.md next to this file.
#
#   web/tools/cef-build/build.sh [STEP ...]       (default: all)
#
# Steps, in order (each can be re-run alone; `all` runs them all):
#   sync     depot_tools + CEF + Chromium checkout (no history) at the pinned versions
#            (CEF's automate-git.py with --no-build --no-distrib), Chromium hooks, sysroot
#   gen      GN configuration out/Release_GN_x64 (GN_DEFINES below; Release only)
#   build    autoninja -j $JOBS cefsimple chrome_sandbox  (libcef.so and its resources)
#   distrib  make_distrib.py --minimal (no symbols, no docs, no archive)
#   install  copy the distribution to $OUT/<cef version>/cef_linux_x86_64 + archive.json,
#            the layout cef-dll-sys expects: then CEF_PATH=$OUT
#   clean-out  delete out/Release_GN_x64 (keeps the checkout; next build is from scratch)
#
# Env (defaults in brackets):
#   CEF_BUILD_ROOT [$HOME/.cache/web-cef-build]  everything goes here (never /tmp, never the repo)
#   CEF_BRANCH     [8078]        CEF release branch (= Chromium branch build number)
#   CEF_COMMIT     [e603d97dd5cb8b0e37d8f1daa2bc5c7eab1aa7a5]  CEF 156.0.31 (cef crate =156.7.0)
#   JOBS           [12]          parallel compile jobs (the user's cap for this build: 12)
#   LINKS          [1]           concurrent link jobs (GN concurrent_links)
#   MEM            [20G]         MemoryMax of every step's scope (user's exception: 20G, this build only)
#   EXTRA_GN_DEFINES             appended to GN_DEFINES
# Every step runs as  systemd-run --user --scope -p MemoryMax=$MEM -p MemorySwapMax=0
#                     nice -n 19 ionice -c3 ...   and logs to $CEF_BUILD_ROOT/logs/<step>-<time>.log
set -eu

ROOT=${CEF_BUILD_ROOT:-$HOME/.cache/web-cef-build}
CEF_BRANCH=${CEF_BRANCH:-8078}
CEF_COMMIT=${CEF_COMMIT:-e603d97dd5cb8b0e37d8f1daa2bc5c7eab1aa7a5}
JOBS=${JOBS:-12}
LINKS=${LINKS:-1}
MEM=${MEM:-20G}

case $ROOT in /tmp/*|/dev/shm/*) echo "build.sh: CEF_BUILD_ROOT must not be in tmpfs" >&2; exit 2 ;; esac

DL=$ROOT/src                      # automate-git --download-dir: chromium/, depot_tools/
DT=$DL/depot_tools
SRC=$DL/chromium/src
OUTDIR=out/Release_GN_x64
DIST_OUT=$ROOT/dist               # CEF_PATH
LOGS=$ROOT/logs
TOOLS=$ROOT/tools
mkdir -p "$DL" "$LOGS" "$TOOLS" "$DIST_OUT"

# Release-only, no symbols, sysroot (Debian bullseye from Chromium's hooks: no Fedora -devel
# packages needed), codecs on. Not an official build: official means ThinLTO + CFI + PGO, whose
# libcef link is not expected to fit the 20G cap with room to spare; see README.md "Choices".
GN_DEFINES="is_official_build=false is_debug=false dcheck_always_on=false \
symbol_level=0 blink_symbol_level=0 v8_symbol_level=0 \
proprietary_codecs=true ffmpeg_branding=\"Chrome\" rtc_use_h264=true \
use_sysroot=true use_remoteexec=false concurrent_links=$LINKS \
disable_fieldtrial_testing_config=true \
use_partition_alloc_as_malloc=false enable_backup_ref_ptr_support=false \
${EXTRA_GN_DEFINES:-}"
export GN_DEFINES
export GN_OUT_CONFIGS=Release_GN_x64
export CEF_ARCHIVE_FORMAT=tar.bz2
# depot_tools: never self-update in the middle of a build, no metrics.
export DEPOT_TOOLS_UPDATE=0 DEPOT_TOOLS_METRICS=0 DEPOT_TOOLS_REPORT_BUILD=0
export PATH="$DT:$PATH"
# siso (Chromium's builder) writes its glog files to $TMPDIR: ~200 MB per half hour of
# build, and /tmp is RAM here. Keep them, and every tool's temp files, on disk.
export TMPDIR=$ROOT/tmp
mkdir -p "$TMPDIR"
# depot_tools' vpython puts its virtualenvs (~1.3 GB) in ~/.cache/vpython-root.<uid> by default.
export VPYTHON_VIRTUALENV_ROOT=$ROOT/vpython-root

# run STEP CMD...: capped, niced, logged; prints the log path.
run() {
  step=$1; shift
  log=$LOGS/$step-$(date +%Y%m%d-%H%M%S).log
  echo "build.sh: $step -> $log" >&2
  {
    echo "# $(date -Is) $step: $*"
    echo "# GN_DEFINES=$GN_DEFINES"
  } >"$log"
  start=$(date +%s)
  set +e
  systemd-run --user --scope -q --unit="web-cef-build-$step-$$" \
    -p MemoryMax="$MEM" -p MemorySwapMax=0 \
    nice -n 19 ionice -c3 "$@" >>"$log" 2>&1
  rc=$?
  set -e
  echo "# $(date -Is) $step exit $rc after $(( $(date +%s) - start ))s" >>"$log"
  echo "build.sh: $step exit $rc after $(( $(date +%s) - start ))s" >&2
  return $rc
}

step_sync() {
  # First run: clones depot_tools (then frozen with --no-depot-tools-update), CEF at
  # $CEF_COMMIT and Chromium at the version in CEF's CHROMIUM_BUILD_COMPATIBILITY.txt,
  # without history, applies CEF's patches and runs the hooks (sysroot, clang, ...).
  # Later runs (new CEF_COMMIT for a security release) update both in place.
  # automate-git.py itself is taken from the pinned CEF commit, fresh each time.
  curl -sSfL -o "$TOOLS/automate-git.py" \
    "https://raw.githubusercontent.com/chromiumembedded/cef/$CEF_COMMIT/tools/automate/automate-git.py"
  upd=
  [ -d "$DT/.git" ] && upd=--no-depot-tools-update
  run sync python3 "$TOOLS/automate-git.py" --download-dir="$DL" --branch="$CEF_BRANCH" \
    --checkout="$CEF_COMMIT" --x64-build --no-chromium-history $upd --no-build --no-distrib
  # gn/autoninja wrappers need depot_tools' own python (python3_bin_reldir.txt), which a
  # checkout with DEPOT_TOOLS_UPDATE=0 never bootstraps. Its luci-auth error is harmless.
  [ -f "$DT/python3_bin_reldir.txt" ] || run bootstrap "$DT/ensure_bootstrap" || true
  [ -f "$DT/python3_bin_reldir.txt" ] || { echo "build.sh: depot_tools bootstrap failed" >&2; exit 1; }
}

step_gen() {
  run gen sh -c "cd '$SRC/cef' && python3 tools/gclient_hook.py"
  grep -v '^#' "$SRC/$OUTDIR/args.gn" >&2 || true
}

step_build() {
  run build sh -c "cd '$SRC' && autoninja -j '$JOBS' -C '$OUTDIR' cefsimple chrome_sandbox"
}

step_distrib() {
  rm -rf "$SRC/cef/binary_distrib"
  run distrib sh -c "cd '$SRC/cef/tools' && python3 make_distrib.py \
    --output-dir='$SRC/cef/binary_distrib' --ninja-build --x64-build --minimal \
    --allow-partial --no-symbols --no-docs --no-archive"
}

step_install() {
  d=$(ls -d "$SRC"/cef/binary_distrib/cef_binary_*_linux64_minimal | head -n 1)
  [ -d "$d" ] || { echo "build.sh: no distribution under $SRC/cef/binary_distrib" >&2; exit 1; }
  name=$(basename "$d")                            # cef_binary_<ver>+g<hash>+chromium-<ver>_linux64_minimal
  ver=$(echo "$name" | sed 's/^cef_binary_\([^+]*\)+.*/\1/')
  dest=$DIST_OUT/$ver/cef_linux_x86_64
  rm -rf "$dest.new"
  mkdir -p "$dest.new"
  # Same flattening as download-cef's extract_target_archive: Release/ + Resources/ +
  # CMakeLists.txt, cmake/, include/, libcef_dll/, CREDITS.html in one directory.
  cp -a "$d/Release/." "$dest.new/"
  cp -a "$d/Resources/." "$dest.new/"
  cp -a "$d/CMakeLists.txt" "$d/cmake" "$d/include" "$d/libcef_dll" "$dest.new/"
  [ -f "$d/CREDITS.html" ] && cp -a "$d/CREDITS.html" "$dest.new/"
  # cef-dll-sys checks archive.json's name for the version; sha1 is not checked.
  printf '{\n  "type": "minimal",\n  "name": "%s.tar.bz2",\n  "sha1": "local-build-proprietary-codecs"\n}' \
    "$name" >"$dest.new/archive.json"
  rm -rf "$dest"
  mv "$dest.new" "$dest"
  echo "build.sh: installed $dest" >&2
  echo "build.sh: use it with  CEF_PATH=$DIST_OUT" >&2
}

step_clean_out() { rm -rf "${SRC:?}/$OUTDIR"; }

[ $# -gt 0 ] || set -- all
for s in "$@"; do
  case $s in
    all) step_sync; step_gen; step_build; step_distrib; step_install ;;
    sync) step_sync ;;
    gen) step_gen ;;
    build) step_build ;;
    distrib) step_distrib ;;
    install) step_install ;;
    clean-out) step_clean_out ;;
    -h|--help) sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "build.sh: unknown step $s" >&2; exit 2 ;;
  esac
done
