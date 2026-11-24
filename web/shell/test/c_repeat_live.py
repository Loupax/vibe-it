#!/usr/bin/env python3
"""C-REPEAT live check: `@:` and `@@` (KEYS.md T140-T146), in the headless nested dwl (web/tools/nest.sh), never on
the user's display, with a TEMPORARY state directory under --out (never the user's), a local 127.0.0.1 page, on a
PRIVATE D-Bus session bus (web/tools/notify-probe/private-bus.conf):

    web/shell/test/c_repeat_live.py --out .qwork/repeat --inject web/tools/inject/target/release/inject \
        --host-bin /path/to/web-host-cef

Steps: `@:` with no line ran says so in the bar; run `:refresh` (the page is requested again: counted in the
page server's log); `@:` requests it again; `@@` and a second `@@` again; `:@:` as a line again and the repeat
did not become the last line (the cmdhistory file holds `refresh` and `@:` only); web ends and starts again on
the same state directory, and `@:` repeats `refresh` at once. Exit 0 when every step passed."""

import argparse
import functools
import os
import re
import shutil
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
PROBE = os.path.join(WEB, "tools/notify-probe")
Fail = live.Fail


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    if not os.environ.get("C_REPEAT_PRIVATE_BUS"):
        env = dict(os.environ, C_REPEAT_PRIVATE_BUS="1")
        conf = os.path.join(PROBE, "private-bus.conf")
        os.execvpe("dbus-run-session", ["dbus-run-session", f"--config-file={conf}", "--", sys.executable, "-I"] + sys.argv, env)
    bus = os.environ.get("DBUS_SESSION_BUS_ADDRESS", "")
    if not bus or re.search(r"/run/user/\d+/bus", bus):
        print(f"refusing: not a private bus ({bus!r})", file=sys.stderr)
        return 2

    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)
    os.makedirs(state, mode=0o700)

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log_path = os.path.join(out, "pages.log")
    page_log = open(page_log_path, "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    page = f"http://127.0.0.1:{srv.server_address[1]}/c4.html?c=blue&n=one"

    def requests():
        with open(page_log_path) as f:
            return sum(1 for l in f if "c4.html" in l)

    results = []

    def step(name, fn):
        t0 = time.monotonic()
        try:
            detail = fn() or ""
            results.append((name, True, detail))
            print(f"  ok   {name} ({time.monotonic() - t0:.1f}s) {detail}", flush=True)
        except Fail as e:
            results.append((name, False, str(e)))
            print(f"  FAIL {name}: {e}", flush=True)

    def shot(n):
        p = os.path.join(out, n)
        subprocess.run(["grim", p], env=nest_env, check=True)
        return p

    def start(tag):
        unit = f"web-c-repeat-live-{os.getpid()}-{tag}"
        cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
               "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, page]
        proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        log = live.Log(proc, os.path.join(out, f"web-{tag}.log"))
        log.wait(r'"title":"[^"]+"', timeout=60)
        g = livelib.settle(log, lambda n: shot(f"{tag}-{n}"))
        return proc, log, g

    proc = None
    try:
        proc, log, g = start("one")

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def expect_requests(n, what):
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if requests() >= n:
                    break
                time.sleep(0.2)
            time.sleep(0.5)
            if requests() != n:
                raise Fail(f"{what}: {requests()} page requests, wanted {n}")

        def none_yet():
            k = log.mark()
            inject(*keys_for("@:"), "wait:300")
            log.wait(r"web: no previous command line", k, 5)
            return "bar: web: no previous command line"

        step("T142 `@:` before any line", none_yet)

        def refresh_line():
            base = requests()
            inject(*keys_for(":refresh"), "key:del", "key:ret", "wait:300")
            expect_requests(base + 1, ":refresh")
            return f"{base + 1} requests"

        step("`:refresh` reloads the page", refresh_line)
        n = {"v": 0}

        def at_colon():
            n["v"] = requests()
            inject(*keys_for("@:"), "wait:300")
            expect_requests(n["v"] + 1, "@:")
            shot("at-colon.png")
            return "@: reloaded"

        step("T141 `@:` repeats the last line", at_colon)

        def at_at():
            base = requests()
            inject(*keys_for("@@"), "wait:300")
            expect_requests(base + 1, "@@")
            inject(*keys_for("@@"), "wait:300")
            expect_requests(base + 2, "second @@")
            return "@@ twice reloaded twice"

        step("T143 `@@` repeats `@:`", at_at)

        def colon_form():
            base = requests()
            inject(*keys_for(":@:"), "key:del", "key:ret", "wait:300")
            expect_requests(base + 1, ":@:")
            inject(*keys_for("@:"), "wait:300")
            expect_requests(base + 2, "@: after :@:")
            with open(os.path.join(state, "cmdhistory")) as f:
                got = f.read()
            if got != "refresh\n@:\n":
                raise Fail(f"cmdhistory is {got!r}")
            return "`:@:` reloaded; `@:` after it still repeats refresh; file = refresh, @:"

        step("T146 `:@:` and the last line stays `refresh`", colon_form)

        def restart():
            # (not `:q`: that line would be the last one, and `@:` would run it)
            proc.terminate()
            try:
                proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                raise Fail("web did not end")
            return "web ended"

        step("web ends", restart)
        time.sleep(1.0)
        proc, log, g = start("two")

        def after_restart():
            base = requests()
            inject(*keys_for("@:"), "wait:300")
            expect_requests(base + 1, "@: after restart")
            return "@: right after the restart reloaded (refresh, from DIR/cmdhistory)"

        step("T141 `@:` right after a restart", after_restart)
    finally:
        if proc is not None and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()

    bad = [r for r in results if not r[1]]
    print(f"{len(results) - len(bad)}/{len(results)} steps passed")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
