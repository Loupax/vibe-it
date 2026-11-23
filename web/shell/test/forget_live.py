#!/usr/bin/env python3
"""P-FORGET-SHELL + C-SOCKDIR live check, inside the nested headless dwl (web/tools/nest.sh),
never on the user's display:

    web/shell/test/forget_live.py --out .qwork/forget-live --inject web/tools/inject/target/release/inject \
        --host-bin ~/.local/lib/web/current/web-host-cef

Uses host-cef/test/forget.py's three local test sites (site-a.test and sub.site-a.test are
forgotten, site-b.test is kept). Steps:
  1. shell 1: log in on all three sites; `web --forget-site site-a.test` while the shell runs
     (the running-instance path): exit 0, a `forgot` line on stdout, the shell's status bar
     shows `forgot ...` and reloads the page; site-a has nothing, site-b everything.
  2. `:forget-site` typed in the shell (no argument: the page's own site), same checks;
     then `:q`: the run directory and the pid file are gone.
  3. no instance: `web --forget-site site-a.test` runs the host itself (exit 0, `forgot` on
     stdout); a new shell shows site-a empty and site-b intact.
  4. SIGKILL that shell: its run directory stays; the next shell start removes it.
Every shell runs in a memory-capped systemd scope that is stopped at the end. Exit 0 = pass."""

import argparse
import functools
import http.server
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(HERE)), "host-cef", "test"))
import live  # noqa: E402
import livelib  # noqa: E402
import forget as fg  # noqa: E402
from c3_live import keys_for  # noqa: E402

Fail = live.Fail
WEB = live.WEB
SHELL = live.SHELL


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", required=True)
    ap.add_argument("--inject", required=True)
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)
    runtime = os.environ["XDG_RUNTIME_DIR"]

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env.pop("DISPLAY", None)
    shell_env.pop("WAYLAND_SOCKET", None)

    fg.Server.log = open(os.path.join(out, "server.log"), "w")
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), fg.Server)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    port = srv.server_address[1]
    switches = fg.chromium_switches(port)
    print(f"nested dwl {wl}; test sites at 127.0.0.1:{port}", flush=True)

    shells = []

    def start_shell(tag, url):
        unit = f"web-forget-live-{os.getpid()}-{tag}"
        cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
               "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
               url, "--"] + switches
        proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        log = live.Log(proc, os.path.join(out, f"web-{tag}.log"))
        shells.append(unit)
        return proc, log

    def web_cli(*args):
        r = subprocess.run([a.web, "--state-dir", state, "--host-bin", a.host_bin, *args] + ["--"] + switches,
                           capture_output=True, text=True, timeout=180, env=dict(os.environ, XDG_RUNTIME_DIR=runtime))
        print(f"  web {' '.join(args)}: exit {r.returncode} stdout {r.stdout.strip()!r} stderr {r.stderr.strip()[-200:]!r}",
              flush=True)
        return r

    ow = oh = None

    def inject(*cmds):
        subprocess.run([a.inject, str(ow), str(oh)] + list(cmds), env=nest_env, check=True)

    def command(line, wait=500):
        inject(*(keys_for(":" + line) + ["key:ret", f"wait:{wait}"]))

    def title_events(log, since):
        for l in log.lines[since:]:
            m = re.search(r"host event: (\{\"event\":\"title\".*)$", l)
            if m:
                yield json.loads(m[1])["title"]

    def go(log, url, prefix, timeout=40):
        since = log.mark()
        command(f"go {url}")
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            for t in title_events(log, since):
                if t.startswith(prefix):
                    return t
            time.sleep(0.2)
        raise Fail(f"{url}: no title {prefix!r}")

    def login(log, host):
        go(log, f"http://{host}/login", f"stored {host}")

    def check(log, host):
        with fg.Server.lock:
            hits = fg.Server.hits.get(host, 0)
        t = go(log, f"http://{host}/check", "check {")
        r = json.loads(t[len("check "):])
        with fg.Server.lock:
            r["server_cookie"] = fg.Server.cookies.get(host, "")
            r["http_cached"] = fg.Server.hits.get(host, 0) == hits
        return r

    def expect_a_gone_b_kept(log, session_a=False, session_b=True):
        fg.expect_none(fg.stored_nothing(check(log, "site-a.test"), "site-a.test", session=session_a, http=False), None)
        fg.expect_none(fg.stored_nothing(check(log, "sub.site-a.test"), "sub.site-a.test", session=False, http=False), None)
        fg.expect_none(fg.stored_everything(check(log, "site-b.test"), "site-b.test", session=session_b, http=False), None)

    def sock_dirs():
        return sorted(d for d in os.listdir(runtime) if re.fullmatch(r"web-\d+", d))

    def forgot_line(stdout):
        ev = json.loads(stdout.strip().splitlines()[-1])
        if ev.get("event") != "forgot" or ev["result"] != "ok" or ev["domain"] != "site-a.test":
            raise Fail(f"printed {ev}")
        return ev

    ok = False
    try:
        # ---- 1 + 2: a running instance
        proc, log = start_shell("1", "http://site-a.test/login")
        log.wait(r'"title":"stored site-a.test', timeout=90)
        def grab(name):
            p = os.path.join(out, name)
            subprocess.run(["grim", p], env=nest_env, check=True)
            return p
        geo = livelib.settle(log, grab)
        ow, oh = geo.ow, geo.oh
        pid_file = os.path.join(state, "web.pid")
        if open(pid_file).read().strip() != str(proc.pid):
            raise Fail(f"{pid_file} says {open(pid_file).read().strip()!r}, shell is {proc.pid}")
        run_dir = os.path.join(runtime, f"web-{proc.pid}")
        if not os.path.exists(os.path.join(run_dir, "forget.sock")):
            raise Fail("no forget.sock in the shell's run directory")
        login(log, "sub.site-a.test")
        login(log, "site-b.test")
        for h in ["site-a.test", "sub.site-a.test", "site-b.test"]:
            fg.expect_none(fg.stored_everything(check(log, h), h, http=False), None)
        print("  ok   all three sites have cookies and storage", flush=True)
        go(log, "http://site-a.test/other", ("other", "from the service worker"))
        since = log.mark()
        r = web_cli("--forget-site", "https://sub.site-a.test/some/page")
        if r.returncode != 0:
            raise Fail(f"running-instance path exited {r.returncode}")
        forgot_line(r.stdout)
        log.wait(r"command: .*forget-site", since, 5)
        log.wait(r"reloading site-a.test", since, 10)
        log.wait(r'"event":"load","tab":\d+,"phase":"end","status":\d+,"kind":"reload"', since, 20)
        shot = os.path.join(out, "bar-after-cli.png")
        subprocess.run(["grim", shot], env=nest_env, check=True)
        expect_a_gone_b_kept(log)
        print("  ok   running instance: forgot printed, page reloaded, site-a empty, site-b kept", flush=True)

        login(log, "site-a.test")
        login(log, "sub.site-a.test")
        go(log, "http://site-a.test/other", ("other", "from the service worker"))
        since = log.mark()
        command("forget-site", wait=1500)
        log.wait(r"bar: .*forgot http://site-a.test/other ok", since, 20)
        log.wait(r"reloading site-a.test", since, 10)
        subprocess.run(["grim", os.path.join(out, "bar-after-command.png")], env=nest_env, check=True)
        expect_a_gone_b_kept(log)
        print("  ok   :forget-site in the shell: status bar line, reload, site-a empty, site-b kept", flush=True)

        login(log, "site-a.test")
        command("q", wait=300)
        proc.wait(timeout=30)
        if os.path.exists(run_dir) or os.path.exists(pid_file):
            raise Fail("run directory or pid file left after :q")
        print("  ok   :q removed the run directory and the pid file", flush=True)

        # ---- 3: no instance
        r = web_cli("--forget-site", "site-a.test")
        if r.returncode != 0:
            raise Fail(f"no-instance path exited {r.returncode}")
        forgot_line(r.stdout)
        proc, log = start_shell("2", "http://site-a.test/check")
        log.wait(r'"title":"check ', timeout=90)
        livelib.settle(log, grab)
        expect_a_gone_b_kept(log, session_b=False)
        print("  ok   no instance: host forgot site-a (exit 0), site-b kept", flush=True)

        # ---- 4: SIGKILL, then the next start cleans up
        pid = proc.pid
        with open(f"/proc/{pid}/comm") as f:
            if f.read().strip() != "web":
                raise Fail(f"pid {pid} is not the shell")
        os.kill(pid, signal.SIGKILL)
        proc.wait(timeout=10)
        subprocess.run(["systemctl", "--user", "stop", f"{shells[-1]}.scope"], check=False, stderr=subprocess.DEVNULL)
        time.sleep(1)
        stale = os.path.join(runtime, f"web-{pid}")
        if not os.path.isdir(stale):
            raise Fail("the killed shell left no run directory (nothing to clean)")
        print(f"  info SIGKILLed shell {pid} left {stale}", flush=True)
        proc, log = start_shell("3", "http://site-b.test/check")
        log.wait(r"removed the stale run directory", timeout=60)
        log.wait(r'"title":"check ', timeout=90)
        if os.path.exists(stale):
            raise Fail(f"{stale} still there after the next start")
        if open(os.path.join(state, "web.pid")).read().strip() != str(proc.pid):
            raise Fail("pid file not rewritten")
        if f"web-{proc.pid}" not in sock_dirs():
            raise Fail("the live shell has no run directory")
        print("  ok   SIGKILLed shell's run directory removed at the next start; the live shells of other sessions stay", flush=True)
        command("q", wait=300)
        proc.wait(timeout=30)
        if f"web-{proc.pid}" in sock_dirs():
            raise Fail("run directory left after :q")
        ok = True
    except (Fail, KeyError, ValueError, OSError, subprocess.SubprocessError) as e:
        print(f"FAIL: {e}", flush=True)
    finally:
        for u in shells:
            subprocess.run(["systemctl", "--user", "stop", f"{u}.scope"], check=False, stderr=subprocess.DEVNULL)
        srv.shutdown()
    print("PASS" if ok else "FAILED", flush=True)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
