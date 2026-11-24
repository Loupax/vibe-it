#!/usr/bin/env python3
"""C-SCRIPT-ERRBAR + C-SOURCE + C-ECHO live check (SCRIPTING.md ":source", ":echo", KEYS.md T148-T157): under
a TEMPORARY XDG_CONFIG_HOME (in --out, never the user's ~/.config), in the headless nested dwl
(web/tools/nest.sh), on a PRIVATE D-Bus session bus, local pages only (127.0.0.1).

    web/shell/test/c_source_live.py --out .qwork/c-source --inject .qwork/inject/release/inject

Run 1: an init.js that throws: its `init.js:LINE: message` is still in the bar after the first page loaded;
the file is edited (the user command `Old` replaced by `New`, a `ready` handler that would open a buffer, no
`throw`) and `:source` says `sourced 1 file(s)`: `:New` works, `:Old` is gone, no extra buffer opened.
Run 2: an `:echo` from a `ready` handler at start is still in the bar after the page loaded.
Exit 0 when every step passed. Needs what test/live.py needs."""

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

INIT_1 = """\
web.command("Old", () => web.cmd("echo old command"));
throw new Error("startup boom");
"""

INIT_2 = """\
web.command("New", () => web.cmd("echo new command"));
web.on("ready", () => web.cmd("new http://HOSTPORT/c2.html"));
"""

INIT_3 = """\
web.on("ready", () => web.cmd("echo echo from init.js"));
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    if not os.environ.get("C_SOURCE_PRIVATE_BUS"):
        env = dict(os.environ, C_SOURCE_PRIVATE_BUS="1")
        conf = os.path.join(PROBE, "private-bus.conf")
        os.execvpe("dbus-run-session", ["dbus-run-session", f"--config-file={conf}", "--", sys.executable, "-I"] + sys.argv, env)
    bus = os.environ.get("DBUS_SESSION_BUS_ADDRESS", "")
    if not bus or re.search(r"/run/user/\d+/bus", bus):
        print(f"refusing: not a private bus ({bus!r})", file=sys.stderr)
        return 2
    print(f"private bus: {bus}", flush=True)

    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    config = os.path.join(out, "config")
    os.makedirs(os.path.join(config, "web"), exist_ok=True)
    init_path = os.path.join(config, "web", "init.js")

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    hostport = f"127.0.0.1:{srv.server_address[1]}"
    start = f"http://{hostport}/second.html"

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl, XDG_CONFIG_HOME=config)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)
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

    def shot(name):
        p = os.path.join(out, name)
        subprocess.run(["grim", p], env=nest_env, check=True)
        return p

    def last_bar(log, since):
        bars = [l for l in list(log.lines)[since:] if "] bar: " in l]
        return bars[-1] if bars else ""

    def launch(tag, source):
        with open(init_path, "w") as f:
            f.write(source.replace("HOSTPORT", hostport))
        state = os.path.join(out, "state-" + tag)
        shutil.rmtree(state, ignore_errors=True)
        os.makedirs(state, mode=0o700)
        unit = f"web-c-source-live-{tag}-{os.getpid()}"
        cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
               "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, start]
        proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        return unit, proc, live.Log(proc, os.path.join(out, f"web-{tag}.log"))

    def stop(unit, proc):
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass

    try:
        # ---- run 1: the error survives the load; :source reloads ----
        unit, proc, log = launch("a", INIT_1)
        try:
            log.wait(r"script error: init\.js:2: startup boom", timeout=30)
            log.wait(r'"title":"c2 second page', timeout=60)  # the page loaded
            g = livelib.settle(log, shot)
            print(f"output {g.ow}x{g.oh} at scale {g.scale}, shell window {g.ww}x{g.wh}", flush=True)
            time.sleep(1.0)

            def inject(*cmds):
                subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

            def command(line):
                k = log.mark()
                inject(*keys_for(":" + line), "key:del", "key:ret", "wait:300")
                return k

            def errbar():
                bar = last_bar(log, 0)
                if "init.js:2: startup boom" not in bar:
                    raise Fail(f"the error is not in the bar after the load; last bar line: {bar!r}")
                shot("errbar.png")
                return "the bar still reads init.js:2: startup boom after the page loaded"

            step("ERRBAR a script error survives the first page load", errbar)

            def old_works():
                k = command("Old")
                log.wait(r"\] echo: old command", k, 10)
                return ":Old ran (before the edit)"

            step("the old user command works", old_works)

            def source():
                with open(init_path, "w") as f:
                    f.write(INIT_2.replace("HOSTPORT", hostport))
                k = command("source")
                log.wait(r"\] bar: NORMAL \| sourced 1 file\(s\)", k, 10)
                shot("sourced.png")
                k2 = command("New")
                log.wait(r"\] echo: new command", k2, 10)
                k3 = command("Old")
                log.wait(r"\] bar: NORMAL \| web: unknown command: :Old", k3, 10)
                time.sleep(1.0)
                opened = [l for l in list(log.lines)[k:] if "script: ready" in l or "buffer 2" in l]
                if opened:
                    raise Fail(f"ready fired again or a buffer opened: {opened}")
                return "sourced 1 file(s); :New works, :Old is unknown, ready did not fire (no new buffer)"

            step("SOURCE reloads: new command in, old one out, no extra buffer", source)

            def source_err():
                with open(init_path, "w") as f:
                    f.write(INIT_1)
                k = command("source")
                log.wait(r"\] bar: NORMAL \| init\.js:2: startup boom", k, 10)
                time.sleep(1.0)
                return "an error at :source shows as at start"

            step("SOURCE an error is reported as at start", source_err)
        finally:
            stop(unit, proc)

        # ---- run 2: an :echo from a script at start survives the load ----
        unit, proc, log = launch("b", INIT_3)
        try:
            log.wait(r"\] echo: echo from init\.js", timeout=60)
            log.wait(r'"title":"c2 second page', timeout=60)
            time.sleep(1.5)

            def echobar():
                bar = last_bar(log, 0)
                if "echo from init.js" not in bar:
                    raise Fail(f"the echo is not in the bar after the load; last bar line: {bar!r}")
                shot("echobar.png")
                return "the bar still reads `echo from init.js` after the page loaded"

            step("ECHO an :echo from a script at start survives the load", echobar)
        finally:
            stop(unit, proc)
    finally:
        srv.shutdown()

    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; logs in {out}/web-*.log")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
