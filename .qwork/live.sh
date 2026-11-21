#!/bin/bash
# Usage: live.sh CASE   (CASE = social | yt | scroll)
set -u
W=/home/loupax/src/vibe-it/.claude/worktrees/agent-aef3e66bd68617ae5
Q=$W/.qwork
BIN=$W/scratchy/target/release/scratchy
INJECT=$W/scratchy/tools/inject/target/release/inject
ulimit -c 0
if [ ! -S "$Q/wire.sock" ]; then
  "$Q/wire" --socket "$Q/wire.sock" >"$Q/wire.log" 2>&1 &
  echo $! > "$Q/wire.pid"
  sleep 1
fi
if [ ! -s "$Q/nest/disp" ] || ! kill -0 "$(cat "$Q/nest/dwl.pid" 2>/dev/null)" 2>/dev/null; then
  SOCK=$("$W/scratchy/tools/nest.sh" "$Q/nest")
  sleep 1
else
  SOCK=$(cat "$Q/nest/disp")
fi
echo "SOCK=$SOCK"
export WAYLAND_DISPLAY=$SOCK
inj() { "$INJECT" 960 1006 "$@"; }
shot() { grim "$Q/$CASE.$1.png"; }
CASE=$1
case $CASE in
  social) URL='https://social.loupax.com/discover' ;;
  yt|ytn) URL='https://www.youtube.com/watch?v=G1AeBgalFZk' ;;
  scroll) URL="file://$Q/long.html" ;;
esac
systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0 \
  env SCRATCHY_MEDIA_AO=null "$BIN" --socket "$Q/wire.sock" "$URL" >"$Q/$CASE.log" 2>&1 &
SPID=$!
echo $SPID > "$Q/$CASE.pid"
sleep 14
shot 0pre
case $CASE in
  social)
    inj key:j wait:300; shot 1input_j
    inj key:esc key:esc wait:500; shot 2normal
    inj key:colon wait:500; shot 3colon
    inj key:esc wait:300 key:j key:j key:j wait:500; shot 4normal_j
    ;;
  scroll)
    inj key:pgdn wait:500; shot 1pgdn
    inj key:space wait:500; shot 2space
    inj key:down key:down key:down wait:500; shot 3down
    inj key:end wait:500; shot 4end
    ;;
  ytn)
    inj key:esc key:esc wait:500; shot 1normal
    inj key:j key:j key:j key:j key:j wait:800; shot 2normal_j
    inj key:i wait:300 key:l wait:1200; shot 3input_l
    ;;
  yt)
    inj move:488,342 click wait:6000; shot 1play
    inj key:l wait:1500; shot 2l_input
    inj key:l wait:1500; shot 3l_input
    ;;
esac
inj key:esc key:esc wait:300 key:colon type:q key:ret wait:500
sleep 2
if kill -0 $SPID 2>/dev/null; then echo "still running, leaving PID $SPID"; fi
