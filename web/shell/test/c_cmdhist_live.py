#!/usr/bin/env python3
"""C-CMDHIST live check: Up/Down in the `:` line and the history kept in DIR/cmdhistory (KEYS.md T125-T128,
STASH.md "Command history"), in the headless nested dwl (web/tools/nest.sh), never on the user's display, with
a TEMPORARY state directory under --out (never the user's), a local 127.0.0.1 page, on a PRIVATE D-Bus session
bus (web/tools/notify-probe/private-bus.conf):

    web/shell/test/c_cmdhist_live.py --out .qwork/cmdhist --inject web/tools/inject/target/release/inject

Steps: run two lines (`:nosuchone`, `:nosuchtwo`); the file holds them, mode 0600; `:q` ends web and is a line
too; web starts again on the same state directory; Up recalls the lines newest first (`q`, `nosuchtwo`,
`nosuchone`), Down walks back to the typed (empty) line; typing `nosuchone` first and Up recalls that line (a
prefix filter) and Down brings the typed text back. Exit 0 when every step passed."""

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
PROBE = os.path.join(WEB, "tools/notify-probe")
Fail = live.Fail


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    if not os.environ.get("C_CMDHIST_PRIVATE_BUS"):
        env = dict(os.environ, C_CMDHIST_PRIVATE_BUS="1")
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

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    page = f"http://127.0.0.1:{srv.server_address[1]}/c4.html?c=blue&n=one"

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
        unit = f"web-c-cmdhist-live-{os.getpid()}-{tag}"
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

        def run_line(line):
            inject(*keys_for(":" + line), "key:del", "key:ret", "wait:300")

        def first_run():
            run_line("nosuchone")
            run_line("nosuchtwo")
            path = os.path.join(state, "cmdhistory")
            with open(path) as f:
                got = f.read()
            if got != "nosuchone\nnosuchtwo\n":
                raise Fail(f"cmdhistory is {got!r}")
            mode = stat.S_IMODE(os.stat(path).st_mode)
            if mode != 0o600:
                raise Fail(f"mode {mode:o}")
            return "cmdhistory = nosuchone, nosuchtwo; mode 0600"

        step("T128 run lines: they are appended to DIR/cmdhistory", first_run)

        def quit_web():
            run_line("q")
            try:
                proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                raise Fail("`:q` did not end web")
            with open(os.path.join(state, "cmdhistory")) as f:
                got = f.read()
            if got != "nosuchone\nnosuchtwo\nq\n":
                raise Fail(f"cmdhistory is {got!r}")
            return "web ended by `:q`, which is the last line of the file"

        step("T128 `:q` is a line too; web exits", quit_web)
        time.sleep(1.0)
        proc, log, g = start("two")

        def recall():
            inject(*keys_for(":"), "wait:100")
            for want in ("q", "nosuchtwo", "nosuchone"):
                k = log.mark()
                inject("key:up", "wait:200")
                log.wait(r"bar: :" + want + r"\|$", k, 5)
            shot("oldest.png")
            for want in ("nosuchtwo", "q"):
                k = log.mark()
                inject("key:down", "wait:200")
                log.wait(r"bar: :" + want + r"\|$", k, 5)
            k = log.mark()
            inject("key:down", "wait:200")
            log.wait(r"bar: :\|$", k, 5)
            inject("key:esc", "wait:200")
            return "Up: q, nosuchtwo, nosuchone (no ghost); Down back to the empty line"

        step("T125 after a restart Up/Down walk the saved lines", recall)

        def prefix():
            inject(*keys_for(":nosuchtw"), "wait:100")
            k = log.mark()
            inject("key:up", "wait:200")
            log.wait(r"bar: :nosuchtwo\|$", k, 5)
            shot("prefix.png")
            k = log.mark()
            inject("key:down", "wait:200")
            log.wait(r"bar: :nosuchtw\|", k, 5)
            inject("key:esc", "wait:200")
            return "typed `nosuchtw`: Up recalls nosuchtwo (no ghost), Down brings the typed text back"

        step("T126 what was typed filters", prefix)
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
