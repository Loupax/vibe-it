#!/bin/sh
# QWPT8 (R64 phase 2, DECISIONS Q2): start/stop WPT's own Python server (wptserve) from the pinned checkout.
# Usage (from scratchy/):
#   tools/wptserve.sh [start] CHECKOUT [--port N] [--wire PATH]
#       start (or reuse) the server AND a wire for it, print `wptserve: ready port=P pid=N wire-pid=M`
#   tools/wptserve.sh --stop CHECKOUT      stop the server and the wire this script started for CHECKOUT
# Then run `scratchy --wpt-th CHECKOUT --server wptserve ...` (it reads the state below and starts nothing: scratchy
# runs under QNET1's seccomp filter, which its children inherit, so python and a TCP-dialling wire cannot be its children).
# CHECKOUT is made by tools/wpt-checkout.sh (wpt-dirs.txt includes WPT's tools/, which wptserve needs).
#
# What runs: `serve.main()` of CHECKOUT/tools/serve with WPT's bundled third_party libraries on sys.path
# (tools/localpaths.py): no `./wpt` launcher, no virtualenv, no pip, no network. Python 3 only.
# Ports: the HTTP port pair P, P+1 (the mount's PORT / PORT+1 convention: `{{ports[http][0|1]}}`) is chosen from a
# hash of the checkout path (18000-27999) and probed free, so concurrent worktrees get their own; `--port N` forces
# P. Only HTTP on 127.0.0.1 (https, h2, ws, wss, dns and h3 are off: QWPT9). Reached only through wire's
# `--map-host '*.web-platform.test=127.0.0.1'`; nothing is added to /etc/hosts.
# wire: its OWN private one (never /run/user/1000/wire.sock), `--no-cookies --no-cache --map-host
# '*.web-platform.test=127.0.0.1'` and `--mount-override /resources/testharnessreport.js=tools/wpt/testharnessreport.js`
# (the runner's results hook replaces WPT's empty one), on a socket in a mktemp dir (unix socket paths are short).
# State (all inside CHECKOUT/.wptserve/, never the user's state dir): pid, port, config.json, log, wire.pid,
# wire.sock.path, wire.log.
# The server is started in its own session, so stopping it is by the recorded PID and its process group only.
set -eu

mode=start
case "${1:-}" in
  --stop) mode=stop; shift ;;
  start) shift ;;
esac
checkout="${1:?usage: tools/wptserve.sh [start|--stop] CHECKOUT [--port N] [--wire PATH]}"
shift
port=""
wire="${SCRATCHY_WIRE:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --port) port="${2:?--port needs a number}"; shift 2 ;;
    --wire) wire="${2:?--wire needs a path}"; shift 2 ;;
    *) echo "wptserve: unexpected argument $1" >&2; exit 1 ;;
  esac
done
here="$(cd "$(dirname "$0")" && pwd)"
[ -f "$checkout/tools/serve/serve.py" ] || { echo "wptserve: $checkout has no tools/serve (run tools/wpt-checkout.sh)" >&2; exit 1; }
checkout="$(cd "$checkout" && pwd)"
state="$checkout/.wptserve"
pidf="$state/pid"

# Is $1 a live wptserve we started (command line carries our `import localpaths` bootstrap)?
ours() {
  [ -r "/proc/$1/cmdline" ] && tr '\0' ' ' < "/proc/$1/cmdline" | grep -q 'serve.main()'
}

wire_ours() {
  [ -r "/proc/$1/cmdline" ] && tr '\0' ' ' < "/proc/$1/cmdline" | grep -q -- '--map-host'
}

if [ "$mode" = stop ]; then
  if [ -f "$state/wire.pid" ]; then
    wpid="$(cat "$state/wire.pid")"
    if wire_ours "$wpid"; then
      kill "$wpid" 2>/dev/null || true
      n=0
      while [ $n -lt 30 ] && kill -0 "$wpid" 2>/dev/null; do sleep 0.1; n=$((n + 1)); done
      kill -KILL "$wpid" 2>/dev/null || true
    fi
    wdir="$(dirname "$(cat "$state/wire.sock.path" 2>/dev/null || echo /nonexistent/x)")"
    case "$wdir" in /tmp/wptserve-wire.*) rm -f "$wdir"/wire.sock "$wdir"/wire.sock.token; rmdir "$wdir" 2>/dev/null || true ;; esac
    rm -f "$state/wire.pid" "$state/wire.sock.path"
  fi
  [ -f "$pidf" ] || { echo "wptserve: not running (no $pidf)"; exit 0; }
  pid="$(cat "$pidf")"
  if ours "$pid"; then
    kill -INT "$pid" 2>/dev/null || true
    n=0
    while [ $n -lt 50 ] && kill -0 "$pid" 2>/dev/null; do sleep 0.1; n=$((n + 1)); done
    # The server was started with setsid: its pid is the process group id. Anything left (its spawned
    # http servers) dies with the group.
    kill -TERM -- "-$pid" 2>/dev/null || true
    n=0
    while [ $n -lt 30 ] && kill -0 "$pid" 2>/dev/null; do sleep 0.1; n=$((n + 1)); done
    kill -KILL -- "-$pid" 2>/dev/null || true
  fi
  rm -f "$pidf" "$state/port"
  echo "wptserve: stopped pid=$pid"
  exit 0
fi

mkdir -p "$state"
if [ -f "$pidf" ] && ours "$(cat "$pidf")"; then
  echo "wptserve: ready port=$(cat "$state/port") pid=$(cat "$pidf") wire-pid=$(cat "$state/wire.pid" 2>/dev/null || echo none) (already running)"
  exit 0
fi
if [ -z "$wire" ]; then
  for c in "$here/../../wire/target/release/wire" "$(command -v wire || true)"; do
    if [ -n "$c" ] && [ -x "$c" ]; then wire="$c"; break; fi
  done
fi
case "$wire" in /*|"") ;; *) [ -e "$wire" ] && wire="$(cd "$(dirname "$wire")" && pwd)/$(basename "$wire")" ;; esac
[ -n "$wire" ] && [ -x "$wire" ] || { echo "wptserve: no wire binary: pass --wire PATH or set SCRATCHY_WIRE" >&2; exit 1; }
"$wire" --help 2>&1 | grep -q -- '--map-host' || { echo "wptserve: $wire has no --map-host (QWPT8): rebuild wire" >&2; exit 1; }

free_pair() {
  python3 -I - "$1" <<'PY'
import socket, sys
p = int(sys.argv[1])
for q in (p, p + 1):
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", q))
    except OSError:
        sys.exit(1)
    finally:
        s.close()
PY
}
if [ -z "$port" ]; then
  h="$(printf %s "$checkout" | cksum | cut -d' ' -f1)"
  port=$((18000 + (h % 5000) * 2))
  n=0
  while ! free_pair "$port"; do
    port=$((port + 2)); n=$((n + 1))
    [ $n -lt 200 ] || { echo "wptserve: no free port pair" >&2; exit 1; }
  done
elif ! free_pair "$port"; then
  echo "wptserve: ports $port and $((port + 1)) are not free" >&2; exit 1
fi

cat > "$state/config.json" <<EOF
{"server_host": "127.0.0.1", "check_subdomains": false,
 "ports": {"http": [$port, $((port + 1))], "http-local": [], "http-public": [], "https": [], "https-local": [],
           "https-public": [], "h2": [], "ws": [], "wss": [], "webtransport-h3": [], "websocket-h3": [], "dns": []}}
EOF

cd "$checkout"
ulimit -c 0
PYTHONPATH="$checkout/tools" setsid python3 -c "
import sys, localpaths
sys.argv = ['serve', '--config', '$state/config.json', '--no-h2', '--no-websocket-h3']
from serve import serve
sys.exit(serve.main())" > "$state/log" 2>&1 < /dev/null &
pid=$!
echo "$pid" > "$pidf"
echo "$port" > "$state/port"

n=0
while [ $n -lt 600 ]; do
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "wptserve: exited early, log:" >&2; tail -20 "$state/log" >&2; rm -f "$pidf"; exit 1
  fi
  code="$(python3 -I - "$port" <<'PY' 2>/dev/null || true
import http.client, sys
try:
    c = http.client.HTTPConnection("127.0.0.1", int(sys.argv[1]), timeout=2)
    c.request("GET", "/resources/testharness.js", headers={"Host": "web-platform.test:%s" % sys.argv[1]})
    print(c.getresponse().status)
except Exception:
    pass
PY
)"
  if [ "$code" = 200 ]; then
    wdir="$(mktemp -d /tmp/wptserve-wire.XXXXXX)"
    "$wire" --socket "$wdir/wire.sock" --no-cookies --no-cache --map-host '*.web-platform.test=127.0.0.1' \
      --mount-override "/resources/testharnessreport.js=$here/wpt/testharnessreport.js" \
      > "$state/wire.log" 2>&1 < /dev/null &
    wpid=$!
    echo "$wpid" > "$state/wire.pid"
    echo "$wdir/wire.sock" > "$state/wire.sock.path"
    m=0
    while [ ! -S "$wdir/wire.sock" ] && [ $m -lt 100 ]; do sleep 0.1; m=$((m + 1)); done
    [ -S "$wdir/wire.sock" ] || { echo "wptserve: wire did not open its socket, log:" >&2; tail -5 "$state/wire.log" >&2; exit 1; }
    echo "wptserve: ready port=$port pid=$pid wire-pid=$wpid"
    exit 0
  fi
  sleep 0.1; n=$((n + 1))
done
echo "wptserve: not ready after 60 s, log:" >&2; tail -20 "$state/log" >&2
exit 1
