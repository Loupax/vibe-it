#!/usr/bin/env python3
"""C3-RCLICK live check (KEYS.md T57, with T29 and T32): a popup opened by a click returns the
shell to the mode from before the click when it closes. Headless nested dwl (web/tools/nest.sh),
never the user's display.

    web/shell/test/c3_rclick_live.py --out .qwork/c3rc --inject .qwork/inject/release/inject

Page: test/pages/rclick.html (a select, a button that opens the select
from a script 1.5 s after its click). The mode is read from the shell's `bar:` log lines.
Needs what c3_live.py needs. Exit 0 when every step passed."""

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
sys.dont_write_bytecode = True
import live  # noqa: E402

SHELL = live.SHELL
WEB = live.WEB
Fail = live.Fail


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True)
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"

    unit = f"web-c3rc-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/rclick.html"]
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
        log.wait(r'"title":"rclick ready"', timeout=60)
        log.wait(r"page window is (\d+)x(\d+)", timeout=10)
        time.sleep(0.5)
        sizes = [re.search(r"\] window (\d+)x(\d+), page", l) for l in log.lines]
        ww, wh = [int(x) for x in [s for s in sizes if s][-1].groups()]
        ow, oh = live.png_size(shot("start.png"))
        bx, by = (ow - ww) // 2, (oh - wh) // 2
        print(f"output {ow}x{oh}, shell window {ww}x{wh} at {bx},{by}; pages at {base}")

        def inject(*cmds):
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + bx},{int(mm[2]) + by}" if mm else c)
            subprocess.run([a.inject, str(ow), str(oh)] + args, env=nest_env, check=True)

        def bar(pattern, since, timeout=5):
            return log.wait(r"\] bar: " + pattern, since, timeout)

        def no_bar(pattern, since, what):
            if any(re.search(r"\] bar: " + pattern, l) for l in log.lines[since:]):
                raise Fail(what)

        def to_normal():
            """Esc Esc until the bar says NORMAL (the start mode already is)."""
            k = log.mark()
            inject("key:esc", "wait:80", "key:esc", "wait:400")
            last = [l for l in log.lines if "] bar: " in l]
            if "NORMAL |" not in (last[-1] if last else ""):
                bar(r"NORMAL \|", k)

        def mode():
            last = [l for l in log.lines if "] bar: " in l]
            return "INPUT" if last and "] bar: INPUT |" in last[-1] else "NORMAL"

        def popup_open(k):
            return log.wait(r"popup after a click: INPUT", k)

        def start_normal():
            bar(r"NORMAL \| $", 0)
        step("start in NORMAL", start_normal)

        def open_select():
            k = log.mark()
            inject("move:150,220", "wait:150", "click", "wait:900")
            popup_open(k)
            bar(r"INPUT \|", k)
            return k

        def closed_back(k, what):
            log.wait(r"popup closed: back to the mode before the click", k)
            bar(r"NORMAL \|", k)
            if mode() != "NORMAL":
                raise Fail(f"not NORMAL after {what}")

        def select_esc():
            k = open_select()
            inject("key:esc", "wait:700")
            closed_back(k, "Esc")
            return "click on the select -> INPUT -> Esc -> NORMAL"
        step("T29 + T57: <select> closed with Esc", select_esc)

        def select_outside():
            k = open_select()
            inject("move:600,500", "wait:100", "click", "wait:700")
            closed_back(k, "a click outside")
            return "click on the select -> INPUT -> click outside -> NORMAL"
        step("T57: <select> closed by a click outside", select_outside)

        def select_option():
            k = log.mark()
            inject("move:150,220", "wait:150", "click", "wait:900")
            popup_open(k)
            bar(r"INPUT \|", k)
            shot("select-open.png")
            inject("key:down", "wait:200", "key:ret", "wait:700")
            log.wait(r'"title":"rclick select:1"', k)
            log.wait(r"popup closed: back to the mode before the click", k)
            if mode() != "NORMAL":
                raise Fail("not NORMAL after choosing an option")
            return "click on the select -> INPUT -> option b chosen -> NORMAL"
        step("T29 + T57: <select> option chosen", select_option)

        def page_popup():
            k = log.mark()
            inject("move:150,300", "wait:100", "click", "wait:3000")
            log.wait(r"popup at", k)
            no_bar(r"INPUT \|", k, "a popup the page opened itself switched to INPUT")
            # In NORMAL the keys are the shell's, so close it with a click outside (which is not a popup of its own).
            inject("move:600,500", "wait:100", "click", "wait:700")
            log.wait(r"popup closed", k)
            no_bar(r"INPUT \|", k, "closing the page's popup switched to INPUT")
            if mode() != "NORMAL":
                raise Fail("mode changed")
            return "the page opened the select 1.5 s after the click: NORMAL throughout (T32)"
        step("T29: a popup opened by the page (no click) does not change the mode", page_popup)

        def autofocus():
            to_normal()
            k = log.mark()
            inject("move:100,380", "wait:100", "click", "wait:2500")
            log.wait(r'"title":"focus ready"', k)
            log.wait(r'"editable-focused","tab":\d+,"focused":true', k)
            bar(r"INPUT \|", k)
            return "left NORMAL by following a link to a page with an autofocus input: INPUT on load (T32)"
        step("T32: autofocus enters INPUT", autofocus)

        def script_focus():
            to_normal()
            k = log.mark()
            inject("move:150,220", "wait:100", "click", "wait:600")  # the button: a click, nothing focused
            no_bar(r"INPUT \|", k, "the button click itself switched to INPUT")
            log.wait(r'"editable-focused","tab":\d+,"focused":true', k, timeout=5)
            bar(r"INPUT \|", k)
            return "focus() from a timer 2 s after a click: INPUT"
        step("T32: a script focus() enters INPUT", script_focus)

        def blur():
            k = log.mark()
            inject("move:150,300", "wait:100", "click", "wait:800")
            log.wait(r'"title":"focus blurred"', k)
            no_bar(r"NORMAL \|", k, "a blur switched back to NORMAL")
            if mode() != "INPUT":
                raise Fail("mode changed on blur")
            return "the field was blurred by script: still INPUT (no auto-return)"
        step("T32: a blur never changes the mode", blur)
    except Fail as e:
        results.append(("start", False, str(e)))
        print(f"  FAIL start: {e}")
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
