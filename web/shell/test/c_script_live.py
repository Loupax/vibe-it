#!/usr/bin/env python3
"""C-SCRIPT live check (SCRIPTING.md, KEYS.md T115-T117): an init.js under a TEMPORARY XDG_CONFIG_HOME (in
--out, never the user's ~/.config) opens two buffers from `web.on("ready")`, its user command works from the
`:` line, and an error in a user command shows `init.js:LINE: message` in the bar. Local pages only
(127.0.0.1), in the headless nested dwl (web/tools/nest.sh), on a PRIVATE D-Bus session bus
(web/tools/notify-probe/private-bus.conf: no service activation).

    web/shell/test/c_script_live.py --out .qwork/c-script --inject .qwork/inject/release/inject

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
web.command("Open", (args) => web.cmd("new " + args[0]));
web.command("Boom", () => {
  throw new Error("boom from a user command");
});
web.map("n", "gb", () => {
  const b = web.buffers.list().find(b => b.url.endsWith("/second.html"));
  web.cmd(b ? "b " + b.number : "echo no second.html");
});
web.map("n", "gq", async () => {
  const bad = await web.cmd("b no-such-buffer-xyz");
  const good = await web.cmd("b 2");
  web.cmd("echo cmdresult " + bad.ok + " " + good.ok);
});
web.on("ready", () => {
  web.cmd("new http://HOSTPORT/second.html");
  web.cmd(":new http://HOSTPORT/c2.html");
});
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    if not os.environ.get("C_SCRIPT_PRIVATE_BUS"):
        env = dict(os.environ, C_SCRIPT_PRIVATE_BUS="1")
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
    os.makedirs(os.path.join(config, "web", "profiles", "default"), exist_ok=True)  # C-SCRIPT-MODULES: no --profile is default

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    hostport = f"127.0.0.1:{srv.server_address[1]}"
    start = f"http://{hostport}/pick.html"
    with open(os.path.join(config, "web", "profiles", "default", "init.js"), "w") as f:
        f.write(INIT_JS.replace("HOSTPORT", hostport))

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl, XDG_CONFIG_HOME=config)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    unit = f"web-c-script-live-{os.getpid()}"
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
        def t117():
            log.wait(r"init\.js: ran in \d+ ms", timeout=30)
            log.wait(r"script: ready \(1 handlers\)", timeout=60)
            log.wait(r"buffer 3 of 3 is current", timeout=30)
            log.wait(r'"url":"http://' + re.escape(hostport) + r'/second\.html"', timeout=30)
            log.wait(r'"url":"http://' + re.escape(hostport) + r'/c2\.html"', timeout=30)
            readies = [l for l in log.lines if "script: ready" in l]
            if len(readies) != 1:
                raise Fail(f"ready fired {len(readies)} times")
            return "ready opened second.html (buffer 2) and c2.html (buffer 3, current)"

        step("T117 init.js from XDG_CONFIG_HOME; ready opens two buffers", t117)

        log.wait(r'"title":"c2 ready"', timeout=30)
        g = livelib.settle(log, shot)
        print(f"output {g.ow}x{g.oh} at scale {g.scale}, shell window {g.ww}x{g.wh}")

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def command(line):
            k = log.mark()
            inject(*keys_for(":" + line), "key:del", "key:ret", "wait:200")
            return k

        def t115():
            k = command(f"Open http://{hostport}/c4.html")
            log.wait(r"script: :new http://" + re.escape(hostport) + r"/c4\.html", k, 10)
            log.wait(r"buffer 4 of 4 is current", k, 15)
            log.wait(r'"url":"http://' + re.escape(hostport) + r'/c4\.html"', k, 15)
            return ":Open URL ran the script, which ran :new URL: buffer 4"

        step("T115 a user command from the : line", t115)

        def t115_error():
            k = command("Boom")
            log.wait(r"\] bar: NORMAL \| .*init\.js:3: boom from a user command", k, 10)
            log.wait(r"script error: init\.js:3: boom from a user command", k, 5)
            return "bar: init.js:3: boom from a user command; the stack in the log"

        step("T115 an error in a user command", t115_error)

        def t116():
            k = log.mark()
            inject(*keys_for(":Op"), "wait:300")
            log.wait(r"\] bar: :Op\|en", k, 10)
            inject("key:esc", "wait:200")
            return "the ghost offers Open after :Op"

        step("T116 completion offers the user command", t116)

        def buflist():
            # C-SCRIPT-BUFLIST: buffer 4 is current; `gb` finds the buffer showing second.html by URL.
            k = log.mark()
            inject(*keys_for("gb"), "wait:300")
            log.wait(r"script: :b 2", k, 10)
            log.wait(r"buffer 2 of 4 is current", k, 15)
            return "web.buffers.list() found second.html (buffer 2) and :b 2 switched to it"

        step("C-SCRIPT-BUFLIST a mapping finds a buffer by URL and switches", buflist)

        def cmdresult():
            # C-CMDRESULT: `await web.cmd(...)` gets ok=false for a failing line and ok=true for a good one.
            k = log.mark()
            inject(*keys_for("gq"), "wait:300")
            log.wait(r"\] bar: NORMAL \| .*cmdresult false true", k, 10)
            return "await web.cmd: ok false for :b no-such-buffer-xyz, true for :b 2"

        step("C-CMDRESULT await web.cmd reports ok", cmdresult)
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
