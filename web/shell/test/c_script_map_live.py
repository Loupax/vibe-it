#!/usr/bin/env python3
"""C-SCRIPT-MAP live check (SCRIPTING.md "web.map", KEYS.md T118-T124): an init.js under a TEMPORARY
XDG_CONFIG_HOME (in --out, never the user's ~/.config) maps NORMAL keys; real key presses (web/tools/inject)
run them, a prefix waits and is replayed on the timeout, `gg` still scrolls to the top with `gh` mapped, and a
bad mapping is refused at map time. Local pages only (127.0.0.1), in the headless nested dwl
(web/tools/nest.sh), on a PRIVATE D-Bus session bus (web/tools/notify-probe/private-bus.conf).

    web/shell/test/c_script_map_live.py --out .qwork/c-script-map --inject .qwork/inject/release/inject

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

INIT_JS = """\
web.map("n", "gh", ":go http://HOSTPORT/second.html<CR>");
web.map("n", "ge", ":go ");
web.map("n", "jk", ":refresh<CR>");
web.map("n", "Q", () => web.cmd("go http://HOSTPORT/c4.html"));
web.map("n", "x", ":refresh<Esc>");
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    if not os.environ.get("C_SCRIPT_MAP_PRIVATE_BUS"):
        env = dict(os.environ, C_SCRIPT_MAP_PRIVATE_BUS="1")
        conf = os.path.join(PROBE, "private-bus.conf")
        os.execvpe("dbus-run-session", ["dbus-run-session", f"--config-file={conf}", "--", sys.executable, "-I"] + sys.argv, env)
    bus = os.environ.get("DBUS_SESSION_BUS_ADDRESS", "")
    if not bus or re.search(r"/run/user/\d+/bus", bus):
        print(f"refusing: not a private bus ({bus!r})", file=sys.stderr)
        return 2
    print(f"private bus: {bus}", flush=True)

    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)
    os.makedirs(state, mode=0o700)
    config = os.path.join(out, "config")
    os.makedirs(os.path.join(config, "web"), exist_ok=True)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    hostport = f"127.0.0.1:{srv.server_address[1]}"
    start = f"http://{hostport}/c2.html"
    with open(os.path.join(config, "web", "init.js"), "w") as f:
        f.write(INIT_JS.replace("HOSTPORT", hostport))

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl, XDG_CONFIG_HOME=config)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    unit = f"web-c-script-map-live-{os.getpid()}"
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

    try:
        def t124():
            log.wait(r"init\.js: ran in \d+ ms", timeout=30)
            m = log.wait(r"script error: (init\.js:5: web\.map: <Esc> in .*)", timeout=10)
            return m.group(1)

        step("T124 a bad rhs is refused at map time", t124)

        log.wait(r'"title":"c2 ready', timeout=30)
        g = livelib.settle(log, shot)
        print(f"output {g.ow}x{g.oh} at scale {g.scale}, shell window {g.ww}x{g.wh}")

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def title(pattern, since, timeout=10):
            return log.wait(r'host event: \{"event":"title","tab":\d+,"title":"' + pattern, since, timeout)

        def t120():
            k = log.mark()
            t0 = time.monotonic()
            inject("key:j", "wait:100")
            m = title(r"scrolled:(\d+)", k, 5)
            took = time.monotonic() - t0
            if int(m.group(1)) <= 0:
                raise Fail(f"j scrolled to {m.group(1)}")
            if took < 0.9:
                raise Fail(f"j scrolled after {took:.2f}s, before the MAP_TIMEOUT_MS wait")
            if any("map jk" in l for l in log.lines[k:]):
                raise Fail("jk ran")
            return f"j alone waited, then scrolled to {m.group(1)} ({took:.2f}s after the press, title timer 150 ms)"

        step("T119/T120 a prefix (j of jk) waits, the timeout replays it", t120)

        def t118_gg():
            k = log.mark()
            t0 = time.monotonic()
            inject("key:g", "wait:30", "key:g", "wait:100")
            title("scrolled:0", k, 5)
            took = time.monotonic() - t0
            if took > 0.9:
                raise Fail(f"gg took {took:.2f}s: it waited for the gh mapping")
            return f"gg scrolled to 0 at once with gh mapped ({took:.2f}s incl. the 150 ms title timer)"

        step("T120 gg is not delayed by the gh mapping", t118_gg)

        def t121_jk():
            k = log.mark()
            inject("key:j", "wait:30", "key:k", "wait:200")
            log.wait(r"map jk: :refresh", k, 5)
            title("c2 ready", k, 15)
            return "jk ran :refresh (the page loaded again)"

        step("T121 jk runs its line", t121_jk)

        def t122():
            k = log.mark()
            inject("key:g", "wait:30", "key:e", "wait:300")
            m = log.wait(r"\] bar: (:go .*)", k, 5)
            inject("key:esc", "wait:200")
            return f"ge opened the line: bar {m.group(1)!r}"

        step("T122 a rhs without <CR> opens the : line with its text", t122)

        def t123():
            k = log.mark()
            inject(*keys_for("Q"), "wait:300")
            log.wait(r"script: :go http://" + re.escape(hostport) + r"/c4\.html", k, 5)
            log.wait(r'"url":"http://' + re.escape(hostport) + r'/c4\.html"', k, 15)
            return "Q called the function, which ran :go .../c4.html"

        step("T123 a function rhs", t123)

        def t121_gh():
            k = log.mark()
            inject("key:g", "wait:30", "key:h", "wait:300")
            log.wait(r"map gh: :go http://", k, 5)
            log.wait(r'"url":"http://' + re.escape(hostport) + r'/second\.html"', k, 15)
            if any("Back" in l and "cmd" in l for l in log.lines[k:]):
                raise Fail("h went back too")
            return "gh ran :go .../second.html (not h's history back)"

        step("T118/T121 gh overrides g + h", t121_gh)
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
