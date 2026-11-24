#!/usr/bin/env python3
"""C-EDITURL live check: `:go ` fills in the page's URL, selected (KEYS.md T129-T133), in the headless nested dwl
(web/tools/nest.sh), never on the user's display, with a TEMPORARY state directory under --out (never the user's),
a local 127.0.0.1 page, on a PRIVATE D-Bus session bus (web/tools/notify-probe/private-bus.conf):

    web/shell/test/c_editurl_live.py --out .qwork/editurl --inject web/tools/inject/target/release/inject

Steps: `:go ` shows the page's URL selected (screenshot highlight.png, looked at by the implementer, not
committed); Backspace wipes it and leaves `go ` (no refill); `:go `, End, typing the end of the URL and Enter
navigates there; `:go `, Enter alone reloads the same URL; `:new` fills the URL of the buffer left.
Exit 0 when every step passed."""

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
    if not os.environ.get("C_EDITURL_PRIVATE_BUS"):
        env = dict(os.environ, C_EDITURL_PRIVATE_BUS="1")
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
    base = f"http://127.0.0.1:{srv.server_address[1]}/c4.html?c=blue&n=one"
    ebase = re.escape(base)

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

    unit = f"web-c-editurl-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, base]
    proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    try:
        log = live.Log(proc, os.path.join(out, "web.log"))
        log.wait(r'"title":"[^"]+"', timeout=60)
        g = livelib.settle(log, lambda n: shot(f"start-{n}"))

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def fill():
            k = log.mark()
            inject(*keys_for(":go "), "wait:300")
            log.wait(r"bar: :go " + ebase + r"\| \[selected 3\.\.\d+\]$", k, 5)
            shot("highlight.png")
            inject("key:esc", "wait:200")
            return "`:go ` shows the page URL selected (see highlight.png)"

        step("T129 `:go ` fills the page URL, selected", fill)

        def wipe():
            inject(*keys_for(":go "), "wait:200")
            k = log.mark()
            inject("key:bs", "wait:200")
            log.wait(r"bar: :go \|$", k, 5)
            inject("key:esc", "wait:200")
            return "Backspace wipes it; `go ` stays"

        step("T130 Backspace wipes the selection, no refill", wipe)

        def edit_end():
            inject(*keys_for(":go "), "wait:200", "key:end")
            k = log.mark()
            inject(*keys_for("&z1"), "wait:200")
            log.wait(r"bar: :go " + ebase + r"&z1\|", k, 5)
            shot("edited.png")
            k = log.mark()
            inject("key:ret", "wait:300")
            log.wait(r'"url":"' + ebase + r'&z1"', k, 20)
            return "End, typed `&z1`, Enter navigated"

        step("T131 End, edit the end of the URL, Enter navigates", edit_end)

        def enter_alone():
            inject(*keys_for(":go "), "wait:200")
            k = log.mark()
            inject("key:ret", "wait:300")
            log.wait(r'"url":"' + ebase + r'&z1"', k, 20)
            return "Enter right after the fill navigates to the same URL"

        step("T132 Enter at once re-navigates", enter_alone)

        def new_blank():
            k = log.mark()
            inject(*keys_for(":new"), "key:del", "key:ret", "wait:500")
            log.wait(r"bar: :go " + ebase + r"&z1\| \[selected 3\.\.\d+\]$", k, 10)
            shot("new.png")
            inject("key:esc", "wait:200")
            return "`:new` opens `:go <left page URL>` selected (see new.png)"

        step("T104 `:new` fills the URL of the buffer left", new_blank)
    finally:
        if proc.poll() is None:
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
