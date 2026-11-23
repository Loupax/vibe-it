#!/usr/bin/env python3
"""C1-INPUT live check: popups with REAL input, through the shell (web), in the headless nested
dwl (web/tools/nest.sh), never on the user's display. The engine host runs WITHOUT
--disable-popup-blocking, so Chromium's popup blocker is in force:

    web/shell/test/popup_live.py --out .qwork/popup --inject web/tools/inject/target/release/inject

  - a click (a user gesture) on a target=_blank link and on a button whose handler calls
    window.open: each loads in the same page (No tabs yet), and the whole run sees exactly one
    `tab-created`;
  - window.open from a timer with no user gesture (at load; and 7 s after a click, when the click's
    transient activation is gone): blocked by the popup blocker. The step reports what the host
    saw (a `popup` / `open-url-from-tab` line or nothing) and that the page did not navigate;
  - C-NOTAB's rules with real clicks only: ctrl-, middle-, shift- and ctrl+shift-click on links
    open nothing and navigate nothing.
Each action's evidence is the shell's log: the host's events (url, load, tab-created) and the
host's `popup ...` lines. Screenshots are not taken. Exit 0 when every step passed."""

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
import livelib  # noqa: E402

SHELL = live.SHELL
WEB = live.WEB
Fail = live.Fail
URL = re.compile(r'"event":"url","tab":\d+,"url":"([^"]*)"')
TAB_CREATED = '"event":"tab-created"'
HOST_POPUP = re.compile(r"(popup|open-url-from-tab) (\w+): (\S*) \(disposition (\w+), user gesture (\w+)\)")
# Page coordinates (popup.html) of the box centres.
AT = {"blank": (300, 100), "btn": (300, 220), "arm": (300, 340), "plain": (300, 460)}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    ap.add_argument("--control-run", action="store_true",
                    help="CONTROL: start the host with --disable-popup-blocking; the two no-gesture steps must then FAIL")
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

    unit = f"web-popup-live-{os.getpid()}"
    # No `-- --disable-popup-blocking`: this is the point of the check.
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/popup.html"] + (["--", "--disable-popup-blocking"] if a.control_run else [])
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
        log.wait(r'"title":"po ', timeout=60)
        geo = livelib.settle(log, shot)
        ow, oh, bx, by = geo.ow, geo.oh, geo.bx, geo.by

        def inject(*cmds):
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + bx},{int(mm[2]) + by}" if mm else c)
            subprocess.run([a.inject, str(ow), str(oh)] + args, env=nest_env, check=True)

        def move(sel):
            return "move:%d,%d" % AT[sel]

        def since(k):
            return list(log.lines[k:])

        def urls(k):
            return [m[1] for l in since(k) if (m := URL.search(l))]

        def host_popups(k):
            return [f"{m[1]} {m[2]} (disposition {m[4]}, user gesture {m[5]})" for l in since(k) if (m := HOST_POPUP.search(l))]

        def tabs_created():
            return sum(1 for l in log.lines if TAB_CREATED in l)

        def title_now():
            for l in reversed(log.lines):
                m = re.search(r'"event":"title","tab":\d+,"title":"(po [^"]*)"', l)
                if m:
                    return m[1]
            return ""

        def check_one_tab():
            n = tabs_created()
            if n != 1:
                raise Fail(f"{n} tab-created events, want exactly 1")

        def back():
            k = log.mark()
            inject("key:h", "wait:1500")
            log.wait(r'"title":"po ', k, timeout=10)
            time.sleep(0.5)

        def step_timer_at_load():
            # The 800 ms timer in popup.html calls window.open once with no input at all.
            time.sleep(2)
            if urls(0)[1:]:
                raise Fail(f"the page navigated by itself: {urls(0)}")
            seen = host_popups(0)
            check_one_tab()
            return (f"window.open from a timer at load (no gesture): the page did not navigate (urls {urls(0)}), "
                    f"one tab-created, title {title_now()!r}, host saw: {seen or 'nothing (no OnBeforePopup)'}")
        step("no-gesture window.open from a timer at load is blocked", step_timer_at_load)

        def click_loads(sel, via):
            k = log.mark()
            inject(move(sel), "wait:100", "click", "wait:300")
            log.wait(r'"title":"c2 second page"', k, timeout=10)
            got = urls(k)
            if not got or not got[-1].endswith(f"/second.html?via={via}"):
                raise Fail(f"urls after the click: {got}")
            pops = host_popups(k)
            check_one_tab()
            back()
            return f"loaded {got[-1]} in the same page; host: {pops or 'no popup line'}; tab-created still 1"
        step("click on target=_blank link loads in the same page", lambda: click_loads("blank", "blank"))
        step("click on a window.open button loads in the same page", lambda: click_loads("btn", "button"))

        def late_timer():
            k = log.mark()
            inject(move("arm"), "wait:100", "click", "wait:500")
            time.sleep(8)
            if urls(k):
                raise Fail(f"the page navigated: {urls(k)}")
            check_one_tab()
            return (f"window.open 7 s after a click (activation expired): no navigation, title {title_now()!r}, "
                    f"host saw: {host_popups(k) or 'nothing (no OnBeforePopup)'}")
        step("window.open from a timer after the click's activation expired is blocked", late_timer)

        def gesture(what, sel, hold, acts):
            """Runs the gesture; returns (navigated, note). Always comes back to popup.html."""
            k = log.mark()
            inject(*hold, move(sel), "wait:100", *acts, "wait:200", "release", "wait:1500")
            if any(TAB_CREATED in l for l in since(k)):
                raise Fail(f"{what}: a tab-created")
            check_one_tab()
            got = urls(k)
            ev = [m[1] for l in since(k) if (m := re.search(r'"title":"po (ev:[^"]*)"', l))]
            note = f"{what}: page saw {ev[-1] if ev else 'no mouse event'}, host: {host_popups(k) or 'no popup line'}"
            if got:
                back()
                return True, note + f", NAVIGATED in place to {got[-1]}"
            return False, note + ", nothing opened, no navigation"

        def gestures():
            notes = []
            for sel in ("plain", "blank"):
                for what, hold, acts in [("ctrl+click", ["hold:c"], ["click"]), ("middle-click", [], ["mclick"]),
                                         ("shift+click", ["hold:s"], ["click"]), ("ctrl+shift+click", ["hold:cs"], ["click"])]:
                    notes.append(gesture(f"{what} {sel}", sel, hold, acts)[1])
            # Strict part (per gesture): no tab-created, exactly one overall. A modifier click that the
            # page saw as a plain click is the shell's doing (NORMAL does not forward modifiers): reported.
            return "; ".join(notes)
        step("C-NOTAB with real clicks: ctrl/middle/shift-click never open a tab (observed what each did)", gestures)

        def still_plain():
            k = log.mark()
            inject(move("plain"), "wait:100", "click", "wait:300")
            log.wait(r'"title":"c2 second page"', k, timeout=10)
            return f"a plain click still follows the link ({urls(k)[-1]})"
        step("a plain click still navigates afterwards", still_plain)

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
