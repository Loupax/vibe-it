#!/usr/bin/env python3
"""C-SINGLE live check, inside the nested headless dwl (web/tools/nest.sh), never on the user's display:

    web/shell/test/single_live.py --out .qwork/single-live --inject web/tools/inject/target/release/inject \
        --host-bin web/host-cef/target/debug/web-host-cef

Steps (one state directory under --out, never the user's):
  1. shell A starts on page-a.
  2. `web URL` (B) with the same state dir: exit 0, prints what happened, A loads the URL and shows
     `opened ... (from another web)`; no second web-host-cef is started.
  3. `web` with no URL: exit 0, says that an instance (A's pid) runs, starts nothing.
  4. a web-host-cef started by hand on A's profile: clean error exit (3) with a message, no panic.
  5. A quits; a stale SingletonLock (dead pid) is removed with a log line and the next shell starts.
  6. a SingletonLock held by a live foreign process (a `sleep` of ours): `web` exits 2 with an error
     naming the profile and the pid; the lock stays; the sleep is killed by its pid at the end.
Every shell runs in a memory-capped systemd scope that is stopped at the end. Exit 0 = pass."""

import argparse
import http.server
import os
import re
import shutil
import socket
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True
sys.path.insert(0, HERE)
import live  # noqa: E402
from c3_live import keys_for  # noqa: E402

Fail = live.Fail
WEB = live.WEB


class Pages(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        name = self.path.strip("/").split("/")[0] or "root"
        body = f"<title>page-{name}</title><p>{name}</p>".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


def host_pids(state):
    """web-host-cef browser processes whose --profile is under `state` (comm is checked)."""
    out = []
    for p in os.listdir("/proc"):
        if not p.isdigit():
            continue
        try:
            with open(f"/proc/{p}/comm") as f:
                if f.read().strip() != "web-host-cef":
                    continue
            with open(f"/proc/{p}/cmdline", "rb") as f:
                cmd = f.read().decode(errors="replace").split("\0")
        except OSError:
            continue
        if any(a.startswith("--profile=" + state) for a in cmd) and not any(a.startswith("--type=") for a in cmd):
            out.append(int(p))
    return sorted(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(live.SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", required=True)
    ap.add_argument("--inject", required=True)
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)
    profile = os.path.join(state, "engines", "cef")
    runtime = os.environ["XDG_RUNTIME_DIR"]
    hostname = socket.gethostname()

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(nest_env)
    shell_env.pop("DISPLAY", None)
    shell_env.pop("WAYLAND_SOCKET", None)

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Pages)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    print(f"nested dwl {wl}; pages at {base}", flush=True)

    units = []
    sleeper = []

    def start_shell(tag, url):
        unit = f"web-single-live-{os.getpid()}-{tag}"
        cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
               "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, url]
        proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        units.append(unit)
        return proc, live.Log(proc, os.path.join(out, f"web-{tag}.log"))

    def web_cli(*args, timeout=60):
        r = subprocess.run([a.web, "--state-dir", state, "--host-bin", a.host_bin, *args], capture_output=True, text=True,
                           timeout=timeout, env=dict(shell_env, XDG_RUNTIME_DIR=runtime))
        print(f"  web {' '.join(args)}: exit {r.returncode} stdout {r.stdout.strip()!r} stderr {r.stderr.strip()[-300:]!r}", flush=True)
        return r

    def stop_units():
        for u in units:
            subprocess.run(["systemctl", "--user", "stop", f"{u}.scope"], check=False, stderr=subprocess.DEVNULL)

    ok = False
    try:
        # ---- 1
        proc, log = start_shell("a", f"{base}/a")
        log.wait(r'"title":"page-a"', timeout=90)
        log.wait(r"page window is (\d+)x(\d+)", timeout=30)
        lock = os.path.join(profile, "SingletonLock")
        if not os.path.islink(lock):
            raise Fail(f"no SingletonLock symlink at {lock}: {os.listdir(profile)}")
        print(f"  info A is pid {proc.pid}; {lock} -> {os.readlink(lock)}", flush=True)
        hosts = host_pids(state)
        if len(hosts) != 1:
            raise Fail(f"expected one host, found {hosts}")

        # ---- 2
        since = log.mark()
        r = web_cli(f"{base}/b")
        if r.returncode != 0 or "opened" not in r.stdout or str(proc.pid) not in r.stdout:
            raise Fail("second web with a URL: wrong exit or message")
        log.wait(r'"title":"page-b"', since, 30)
        log.wait(r"bar: .*opened http://127.0.0.1:\d+/b \(from another web\)", since, 10)
        subprocess.run(["grim", os.path.join(out, "bar-after-open.png")], env=nest_env, check=True)
        if host_pids(state) != hosts:
            raise Fail(f"hosts changed: {hosts} -> {host_pids(state)}")
        print("  ok   web URL: exit 0, A loaded page-b and shows the message, still one host", flush=True)

        # ---- 3
        before = set(os.listdir(runtime))
        r = web_cli()
        if r.returncode != 0 or "already running" not in r.stdout or str(proc.pid) not in r.stdout:
            raise Fail("second web without a URL: wrong exit or message")
        if host_pids(state) != hosts or set(os.listdir(runtime)) != before:
            raise Fail("web without a URL started something")
        print("  ok   web with no URL: exit 0, says A (pid) runs, started nothing", flush=True)

        # ---- 4: a host on the same profile, by hand
        cmd = [a.host_bin, f"--profile={profile}", "--url=about:blank"]
        r = subprocess.run(["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0"] + cmd,
                           capture_output=True, text=True, timeout=120, env=shell_env)
        print(f"  host on a locked profile: exit {r.returncode} stderr {r.stderr.strip()[-300:]!r}", flush=True)
        if r.returncode != 3 or "panick" in r.stderr or "cef initialize failed" not in r.stderr or profile not in r.stderr:
            raise Fail("host on a locked profile did not exit cleanly with its message")
        print("  ok   web-host-cef on a locked profile: exit 3 and a message naming the profile, no panic", flush=True)
        if host_pids(state) != hosts:
            raise Fail("A's host changed")

        # ---- 5: stale lock
        since = log.mark()
        inject_size = subprocess.run(["grim", "-"], env=nest_env, capture_output=True).stdout
        ow, oh = int.from_bytes(inject_size[16:20], "big"), int.from_bytes(inject_size[20:24], "big")
        subprocess.run([a.inject, str(ow), str(oh)] + keys_for(":q") + ["key:ret", "wait:300"], env=nest_env, check=True)
        proc.wait(timeout=30)
        stop_units()
        time.sleep(1)
        dead = subprocess.Popen(["true"])
        dead.wait()
        for f in ("SingletonLock", "SingletonCookie", "SingletonSocket"):
            try:
                os.remove(os.path.join(profile, f))
            except FileNotFoundError:
                pass
        os.symlink(f"{hostname}-{dead.pid}", lock)
        proc, log = start_shell("stale", f"{base}/a")
        log.wait(rf"removed the stale profile lock .*process {dead.pid} is gone", timeout=60)
        log.wait(r'"title":"page-a"', timeout=90)
        print(f"  ok   stale lock (dead pid {dead.pid}) removed with a log line; the shell started", flush=True)
        subprocess.run([a.inject, str(ow), str(oh)] + keys_for(":q") + ["key:ret", "wait:300"], env=nest_env, check=True)
        proc.wait(timeout=30)
        stop_units()
        time.sleep(1)

        # ---- 6: a live foreign holder
        sl = subprocess.Popen(["sleep", "300"])
        sleeper.append(sl)
        for f in ("SingletonLock", "SingletonCookie", "SingletonSocket"):
            try:
                os.remove(os.path.join(profile, f))
            except FileNotFoundError:
                pass
        os.symlink(f"{hostname}-{sl.pid}", lock)
        r = web_cli(f"{base}/a")
        if r.returncode != 2 or str(sl.pid) not in r.stderr or profile not in r.stderr or "panick" in r.stderr:
            raise Fail("live foreign lock: wrong exit or message")
        if not os.path.islink(lock) or os.readlink(lock) != f"{hostname}-{sl.pid}":
            raise Fail("the live lock was touched")
        if host_pids(state):
            raise Fail("a host was started anyway")
        print(f"  ok   lock held by live pid {sl.pid}: exit 2, error names the profile and the pid, lock untouched", flush=True)
        ok = True
    except Fail as e:
        print(f"FAIL: {e}", flush=True)
    finally:
        for s in sleeper:
            if s.poll() is None:
                s.kill()
                s.wait()
        stop_units()
        subprocess.run([os.path.join(WEB, "tools/nest.sh"), "--stop"], check=False)
    print("PASS" if ok else "FAIL", flush=True)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
