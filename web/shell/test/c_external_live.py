#!/usr/bin/env python3
"""C-EXTERNAL live check: `:openExternal` and `:openExternal example.com` call $BROWSER with the right single
URL argument; a javascript: URL is refused (KEYS.md "Opening a page in another browser", T99-T102) in the
headless nested dwl (web/tools/nest.sh), never on the user's display and never with a real browser: $BROWSER is
a script in --out that records its argv, one line per argument, to --out/argv.

    web/shell/test/c_external_live.py --out .qwork/c-external --inject .qwork/inject/release/inject

Exit 0 when every step passed. Needs what test/live.py needs."""

import argparse
import functools
import os
import re
import shutil
import stat
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # no __pycache__ in the source tree
import live  # noqa: E402
import livelib  # noqa: E402
from c3_live import keys_for  # noqa: E402

SHELL = live.SHELL
WEB = live.WEB
Fail = live.Fail


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)
    os.makedirs(state, mode=0o700)

    argv_file = os.path.join(out, "argv")
    if os.path.exists(argv_file):
        os.remove(argv_file)
    fake = os.path.join(out, "fake-browser")
    with open(fake, "w") as f:
        f.write('#!/bin/sh\nprintf "%s\\n" "--call--" >> "$0.argv"\nfor a in "$@"; do printf "%s\\n" "$a" >> "$0.argv"; done\n')
    os.chmod(fake, os.stat(fake).st_mode | stat.S_IXUSR)
    argv_file = fake + ".argv"
    if os.path.exists(argv_file):
        os.remove(argv_file)

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl, BROWSER=f"/nonexistent/browser:{fake}")
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    hostport = f"127.0.0.1:{srv.server_address[1]}"
    start = f"http://{hostport}/pick.html"

    unit = f"web-c-external-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, start]
    proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(out, "web.log"))
    results = []

    def step(name, fn):
        t0 = time.monotonic()
        try:
            detail = fn() or ""
            results.append((name, True, detail))
            print(f"  ok   {name} ({time.monotonic() - t0:.1f}s) {detail}")
        except Fail as e:
            results.append((name, False, str(e)))
            print(f"  FAIL {name}: {e}")

    def shot(name):
        p = os.path.join(out, name)
        subprocess.run(["grim", p], env=nest_env, check=True)
        return p

    def calls():
        try:
            text = open(argv_file).read()
        except FileNotFoundError:
            return []
        return [c.splitlines() for c in text.split("--call--\n")[1:]]

    try:
        log.wait(r'"title":"pick start"', timeout=60)
        g = livelib.settle(log, shot)
        print(f"output {g.ow}x{g.oh} at scale {g.scale}, shell window {g.ww}x{g.wh}")

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def bar(since, pat, timeout=6):
            log.wait(r"\] bar: " + pat, since, timeout)
            time.sleep(0.1)
            for l in reversed(log.lines[since:]):
                if re.search(r"\] bar: " + pat, l):
                    return l
            raise Fail(f"no bar line {pat}")

        def command(line, wait_for, timeout=6):
            k = log.mark()
            inject(*keys_for(":" + line), "key:del", "key:ret", "wait:200")
            return bar(k, wait_for, timeout)

        def t99a():
            command("openExternal", r"NORMAL \| opened externally: " + re.escape(start) + re.escape(f" ({fake})"))
            time.sleep(0.3)
            if calls() != [[start]]:
                raise Fail(f"calls: {calls()}")
            return f"$BROWSER's second entry ran with the one argument {start}"

        step("T99/T101 :openExternal hands over the current URL", t99a)

        def t99b():
            command("openExternal example.com", r"NORMAL \| opened externally: https://example\.com/ ")
            time.sleep(0.3)
            if calls() != [[start], ["https://example.com/"]]:
                raise Fail(f"calls: {calls()}")
            return "https://example.com/ as the single argument"

        step("T99 :openExternal example.com", t99b)

        def t100():
            command("openExternal javascript:alert", r"NORMAL \| web: :openExternal: refused javascript")
            time.sleep(0.5)
            if len(calls()) != 2:
                raise Fail(f"calls: {calls()}")
            return "refused with a message, the browser was not run"

        step("T100 javascript: refused", t100)
    finally:
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        srv.shutdown()

    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; log in {out}/web.log")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
