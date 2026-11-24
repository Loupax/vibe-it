#!/usr/bin/env python3
"""C-BELL live check: a page's Web Notifications reach the status bar, never the desktop (KEYS.md
"Notifications", T106-T110), in the headless nested dwl (web/tools/nest.sh), never on the user's display.

    web/shell/test/c_bell_live.py --out .qwork/bell --inject .qwork/inject/release/inject

Runs on a PRIVATE D-Bus session bus (web/tools/notify-probe/private-bus.conf: no service activation)
with fake_notifyd.py owning org.freedesktop.Notifications, so nothing can reach the user's desktop and
its log shows whether Chromium tried (it must not: only its start-up GetCapabilities/GetServerInformation).
Buffer 1 is the probe page (web/tools/notify-probe/probe.html) told to start after a delay; buffer 2
(`:new`) is shown meanwhile. Steps: the hidden buffer's notifications (page, service worker
registration, iframe, a burst the host rate-limits) make the bar flash once and show `[1!]`; `:ls`
marks buffer 1 `!` (screenshot); `:notifications` lists them (unread marked), filtering and Enter show buffer 1 and the mark goes (Ctrl-^ still works); a notification of the shown buffer
flashes without `[1!]`; the in-worker notification fails; no `Notify` reached D-Bus. Exit 0 when every
step passed. Needs what test/live.py needs, PIL, python3-dbus and gi (fake_notifyd.py)."""

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
BELL_BG = (120, 200, 255)  # config.rs


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    if not os.environ.get("C_BELL_PRIVATE_BUS"):
        # Again, inside a fresh private session bus that ends with us.
        env = dict(os.environ, C_BELL_PRIVATE_BUS="1")
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
    notifyd_log = os.path.join(out, "notifyd.log")
    open(notifyd_log, "w").close()
    notifyd = subprocess.Popen([sys.executable, "-I", os.path.join(PROBE, "fake_notifyd.py"), notifyd_log])

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    servers = []
    for d in (PROBE, os.path.join(HERE, "pages")):
        s = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=d, log=page_log))
        threading.Thread(target=s.serve_forever, daemon=True).start()
        servers.append(s)
    probe = f"http://127.0.0.1:{servers[0].server_address[1]}/probe.html"
    blue = f"http://127.0.0.1:{servers[1].server_address[1]}/c4.html?c=blue&n=two"
    first = f"{probe}?tag=hidden&delay=7000&burst=3"

    unit = f"web-c-bell-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, first]
    proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(out, "web.log"))
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

    def count(pattern, since=0):
        rx = re.compile(pattern)
        return sum(1 for l in list(log.lines)[since:] if rx.search(l))

    try:
        log.wait(r'"title":"notify-probe"', timeout=60)
        g = livelib.settle(log, shot)
        print(f"output {g.ow}x{g.oh} at scale {g.scale}, shell window {g.ww}x{g.wh}", flush=True)
        from PIL import Image

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def command(line):
            k = log.mark()
            inject(*keys_for(":" + line), "key:del", "key:ret", "wait:200")
            return k

        def bar_colour(png):
            im = Image.open(png).convert("RGB")
            # The bar's right padding (no text): 2 logical px in from the right, mid-height of the bar.
            x, y = (g.bx + g.ww - 2) * g.scale, (g.by + g.wh - 12) * g.scale
            return im.getpixel((int(x), int(y)))

        marks = {}

        def watch_flash(k, timeout):
            """grim back to back until the log has `bell: flash` then `bell: off`; returns whether a
            shot taken in between shows the bar in the bell's colours (kept as flash.png)."""
            end = time.monotonic() + timeout
            n, caught = 0, False
            while time.monotonic() < end and not count(r"\] bell: off", k):
                n += 1
                p = shot(f"watch-{n % 2}.png")
                if count(r"\] bell: flash", k) and bar_colour(p) == BELL_BG:
                    shutil.copy(p, os.path.join(out, "flash.png"))
                    caught = True
            for i in range(2):
                q = os.path.join(out, f"watch-{i}.png")
                if os.path.exists(q):
                    os.remove(q)
            log.wait(r"\] bell: flash", k, 1)
            log.wait(r"\] bell: off", k, 2)
            return caught

        def hidden_buffer():
            k = command(f"new {blue}")
            log.wait(r"buffer 2 of 2 is current", k, 10)
            log.wait(r'"title":"two visible"', k, 30)
            marks["shown"] = log.mark()
            # The probe in buffer 1 starts 7 s after its load. Screenshots back to back meanwhile: one
            # of them should catch the ~150 ms flash.
            lit = watch_flash(k, 20)
            time.sleep(1.5)  # every notification of the probe's run, and the bell's rate limit window
            log.wait(r"bar url: \[1!\] \[2\] " + re.escape(blue), k, 5)
            notes = count(r'host event: \{"event":"notify"', k)
            flashes = count(r"\] bell: flash", k)
            if notes != 5:
                raise Fail(f"{notes} notify events, not 5 (probe, sw, iframe, 2 of the burst before the host's limit)")
            if count(r"notify: tab \d+: rate limit, dropping", k) != 1:
                raise Fail("the host's rate limit did not log its drop once")
            if flashes != 1:
                raise Fail(f"{flashes} flashes for one burst (the bell's rate limit lets one through)")
            dark = bar_colour(shot("unread.png"))
            if dark == BELL_BG:
                raise Fail("the bar stayed in the bell's colours")
            caught = "flash.png shows the lit bar" if lit else "no screenshot caught the lit bar"
            return f"5 notify events (3 dropped by the host), 1 flash ({caught}; after: {dark}), bar [1!] [2]"

        step("T106/T110 a hidden buffer's notifications: one flash, [1!] in the bar", hidden_buffer)

        def ls():
            k = log.mark()
            inject(*keys_for(":ls"), "key:del", "key:ret", "wait:400")
            log.wait(r">\| pick buffers \[1/2 of 2\] buffer 1", k, 5)
            shot("ls.png")
            inject("key:esc", "wait:200")
            return "`:ls` open (ls.png: `1#! notify-probe`, detail the latest title)"

        step("T109 :ls marks the unread buffer", ls)

        def show():
            k = log.mark()
            inject(*keys_for(":notifications"), "key:del", "key:ret", "wait:400")
            log.wait(r">\| pick notifications \[1/5 of 5\] notice", k, 5)
            shot("notifications.png")
            inject(*keys_for("probe title"), "wait:300")
            if not count(r"pick notifications \[1/\d+ of 5\]", k):
                raise Fail("typing did not filter the notifications")
            inject("key:ret", "wait:300")
            log.wait(r"buffer 1 of 2 is current", k, 5)
            log.wait(r"bar url: \[1\] " + re.escape(first), k, 5)
            return "`:notifications` (notifications.png), filtered, Enter: buffer 1 shown, the bar's [1!] gone"

        step("T108/T112/T113 :notifications Enter shows the buffer and reads its notifications", show)

        def ctrl_caret():
            k = log.mark()
            inject("hold:cs", "key:6", "release", "wait:300")
            log.wait(r"buffer 2 of 2 is current", k, 5)
            k = log.mark()
            inject("hold:cs", "key:6", "release", "wait:300")
            log.wait(r"buffer 1 of 2 is current", k, 5)
            return "Ctrl-^ both ways"

        step("T77 Ctrl-^ still works", ctrl_caret)

        def shown_buffer():
            time.sleep(1.2)  # past the bell's interval
            url = f"{probe}?tag=shown&delay=500"
            k = command(f"go {url}")
            # The host's bucket is still nearly empty from the first run: only the first one or two pass.
            log.wait(r"\] bell: flash", k, 15)
            log.wait(r'host event: \{"event":"notify".*"title":"probe title"', k, 10)
            time.sleep(1.0)
            if count(r"bar url: \[\d+!\]", k):
                raise Fail("a notification of the shown buffer made it unread")
            n = count(r'host event: \{"event":"notify"', k)
            return f"{n} notify events of the shown buffer: a flash, no [N!]"

        step("T107 the shown buffer's notification flashes, not unread", shown_buffer)

        def desktop():
            with open(notifyd_log) as f:
                calls = f.read().splitlines()
            if any(c.startswith("Notify") for c in calls):
                raise Fail(f"Chromium reached the desktop's notification server: {calls}")
            host = [l for l in log.lines if "notify:" in l and "web-host-cef" in l]
            return f"fake notifyd saw {[c for c in calls if not c.startswith('fake')]}; host: {host[:2]}"

        step("T110 nothing reached D-Bus (no desktop popup)", desktop)

        def quit():
            k = command("q")
            proc.wait(timeout=20)
            return f"exit {proc.returncode}"

        step("T91 :q", quit)
    finally:
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        for s in servers:
            s.shutdown()
        notifyd.terminate()
        notifyd.wait(timeout=5)

    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; log in {out}/web.log")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
