#!/bin/sh
# Install web (the shell) and web-host-cef (Chromium) with CEF's runtime files, no root.
#
#   web/install.sh [--prefix DIR] [--no-build]       (default prefix: $HOME/.local)
#
# Layout:
#   PREFIX/lib/web/<version>/   web, web-host-cef, libcef.so, *.pak, icudtl.dat, snapshots, locales/ ...
#   PREFIX/lib/web/current      symlink to the newest <version>
#   PREFIX/bin/web              symlink to ../lib/web/current/web
#
# Why a symlink and no wrapper: the shell looks for `web-host-cef` next to its own
# executable (std::env::current_exe resolves symlinks), and the host finds libcef.so and
# its resources next to itself (rpath $ORIGIN). So `web` on PATH needs no wrapper, no
# environment variable and no change to the shell.
#
# A running web is never broken by a reinstall: each install goes to a NEW version dir
# (staged under a hidden name, renamed into place when complete) and then `current` is
# switched with an atomic rename. The previous version stays (running browsers keep
# using it); older ones are removed.
#
# Env: CEF_PATH (CEF cache root, default $HOME/.cache/web-spike/cef, as web/host-cef/README.md),
#      WEB_BUILD_DIR (cargo target dirs go to $WEB_BUILD_DIR/shell and /host-cef; default:
#      <crate>/target). Nothing is built in /tmp.
set -eu

PREFIX=${HOME}/.local
BUILD=1
while [ $# -gt 0 ]; do
  case $1 in
    --prefix) PREFIX=${2:?--prefix needs a directory}; shift 2 ;;
    --prefix=*) PREFIX=${1#--prefix=}; shift ;;
    --no-build) BUILD=0; shift ;;
    -h|--help) sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "install.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done

HERE=$(cd "$(dirname "$0")" && pwd)
case $PREFIX in /*) ;; *) PREFIX=$(pwd)/$PREFIX ;; esac

if [ "$BUILD" = 1 ]; then
  export CEF_PATH=${CEF_PATH:-$HOME/.cache/web-spike/cef}
  export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
  for crate in shell host-cef; do
    if [ -n "${WEB_BUILD_DIR:-}" ]; then
      CARGO_TARGET_DIR=$WEB_BUILD_DIR/$crate
      export CARGO_TARGET_DIR
    else
      unset CARGO_TARGET_DIR
    fi
    echo "install.sh: building web/$crate (release)" >&2
    (cd "$HERE/$crate" && nice -n 19 ionice -c3 cargo build --release)
  done
fi

if [ -n "${WEB_BUILD_DIR:-}" ]; then
  SHELL_OUT=$WEB_BUILD_DIR/shell/release
  HOST_OUT=$WEB_BUILD_DIR/host-cef/release
else
  SHELL_OUT=$HERE/shell/target/release
  HOST_OUT=$HERE/host-cef/target/release
fi
for f in "$SHELL_OUT/web" "$HOST_OUT/web-host-cef" "$HOST_OUT/libcef.so"; do
  [ -f "$f" ] || { echo "install.sh: missing $f (build first)" >&2; exit 1; }
done

LIB=$PREFIX/lib/web
VER=$(date +%Y%m%d-%H%M%S)-$(git -C "$HERE" rev-parse --short HEAD 2>/dev/null || echo nogit)
mkdir -p "$LIB" "$PREFIX/bin"
STAGE=$LIB/.stage-$$
rm -rf "$STAGE"
mkdir "$STAGE"
trap 'rm -rf "$STAGE"' EXIT INT TERM

cp "$SHELL_OUT/web" "$STAGE/web"
cp "$HOST_OUT/web-host-cef" "$STAGE/web-host-cef"
# The CEF runtime: what build.rs links next to the host (chrome-sandbox is not needed:
# the host uses Chromium's user-namespace sandbox).
for f in "$HOST_OUT"/*.so "$HOST_OUT"/*.so.* "$HOST_OUT"/*.pak "$HOST_OUT"/*.dat "$HOST_OUT"/*.bin "$HOST_OUT"/*.json; do
  if [ -f "$f" ]; then cp "$f" "$STAGE/"; fi
done
cp -r "$HOST_OUT/locales" "$STAGE/locales"
chmod 755 "$STAGE/web" "$STAGE/web-host-cef"
for f in libcef.so icudtl.dat resources.pak v8_context_snapshot.bin; do
  [ -f "$STAGE/$f" ] || { echo "install.sh: CEF runtime file $f missing in $HOST_OUT" >&2; exit 1; }
done

mv "$STAGE" "$LIB/$VER"
trap - EXIT INT TERM
ln -sfn "$VER" "$LIB/.current-new"
mv -T "$LIB/.current-new" "$LIB/current"
ln -sfn ../lib/web/current/web "$PREFIX/bin/.web-new"
mv -T "$PREFIX/bin/.web-new" "$PREFIX/bin/web"

# Keep the new version and the one before it; remove older ones.
for d in $(ls -1 "$LIB" | grep -E '^[0-9]{8}-' | sort -r | tail -n +3); do
  rm -rf "${LIB:?}/$d"
done

echo "install.sh: installed $VER -> $PREFIX/lib/web/current, $PREFIX/bin/web"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "install.sh: note: $PREFIX/bin is not on PATH" >&2 ;; esac
