#!/bin/bash
# args: W (qwork dir), runs scratchy on wikipedia, drags the window thumb for ~8 s
W=$1; I=$2
SCRATCHY_FRAME_STATS=$W/fs.jsonl systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0 timeout 110 target/release/scratchy --socket $W/wire.sock https://en.wikipedia.org/wiki/United_States >$W/s2.log 2>&1 &
echo $! > $W/s2.pid
sleep 15
grim $W/w1.png
args="move:955,8 down"
for i in $(seq 1 60); do y=$((8+i*15)); args="$args move:955,$y wait:25"; done
for i in $(seq 1 60); do y=$((920-i*15)); args="$args move:955,$y wait:25"; done
$I 960 1006 $args wait:300 up wait:500
sleep 2
grim $W/w2.png
