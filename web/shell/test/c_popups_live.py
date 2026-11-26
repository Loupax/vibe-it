#!/usr/bin/env python3
"""C-POPUPS live check: popups like Chrome (KEYS.md "Popups", T93-T98) in the headless nested dwl
(web/tools/nest.sh), never on the user's display. Real clicks through web/tools/inject.

    web/shell/test/c_popups_live.py --out .qwork/c-popups --inject .qwork/inject/release/inject

The opener is test/pages/popups.html (red): a button whose click calls window.open with window
features, a target=_blank link, a timer that calls window.open with no user gesture, and an iframe
(popframe.html) whose timer does the same. A popup is popchild.html (green): its title says whether it
has window.opener, it posts `hello from child <n>` to its opener, and the whole page is a button that
calls window.close(). The opener's title collects what it heard (`pp <window.open results> | <messages>`).

Steps: the two timer popups are blocked (the host's popup-blocked, main frame and iframe; window.open
returned null; the bar says `popup blocked: <url>`); `:popupSearch` lists both and Enter on the timer
one opens it with window.opener kept (the opener hears the child); `:popupOpen` opens the iframe's (the
iframe hears the child); a real click on the button opens a buffer whose child posts to its opener, and
a click on the child's window.close() kills only that buffer; a click on the target=_blank link opens a
buffer (no opener). Exit 0 when every step passed. Needs what test/live.py needs, and PIL."""

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
Fail = live.Fail
RED, GREEN = (255, 0, 0), (0, 255, 0)
# Page coordinates (popups.html) of the box centres.
AT = {"btn": (190, 82), "blank": (190, 202)}


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

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    # Empty config/state/data homes of its own: no user init.js (web.buffers.focusExisting is off).
    homes = {}
    for var in ("XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME"):
        homes[var] = os.path.join(out, var.lower())
        shutil.rmtree(homes[var], ignore_errors=True)
        os.makedirs(homes[var], mode=0o700)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl, **homes)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    opener_url = f"{base}/popups.html"
    child = lambda n: f"{base}/popchild.html?n={n}"  # noqa: E731

    unit = f"web-c-popups-live-{os.getpid()}"
    # No --disable-popup-blocking here: the host adds it itself (it is the popup blocker).
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, opener_url]
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

    try:
        log.wait(r'"title":"pp ', timeout=60)
        g = livelib.settle(log, shot)
        print(f"output {g.ow}x{g.oh} at scale {g.scale}, shell window {g.ww}x{g.wh}", flush=True)
        from PIL import Image

        def colour(name):
            """The colour in the page area, right of the boxes, from a screenshot."""
            time.sleep(0.4)
            im = Image.open(shot(name)).convert("RGB")
            x, y = (g.bx + g.ww * 2 // 3) * g.scale, (g.by + g.wh // 3) * g.scale
            return im.getpixel((int(x), int(y)))

        def shows(c, name):
            got = colour(name)
            if got != c:
                raise Fail(f"{name}: the page area is {got}, not {c}")

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def click(sel):
            x, y = AT[sel]
            inject(f"move:{g.bx + x},{g.by + y}", "wait:100", "click", "wait:300")

        def bar(since, pat, timeout=8, what="bar"):
            log.wait(r"(\] |\+)" + what + ": " + pat, since, timeout)

        def command(line, wait_for=None, timeout=8):
            """Runs `:line` (Delete hides a ghost, so what was typed runs); waits for a bar line."""
            k = log.mark()
            inject(*keys_for(":" + line), "key:del", "key:ret", "wait:200")
            if wait_for:
                bar(k, wait_for, timeout)
            return k

        def opener_title(pat, since, timeout=10):
            log.wait(r'host event: \{"event":"title","tab":' + str(tabs["opener"]) + r',"title":"pp [^"]*' + pat, since, timeout)

        def created(since, cause, timeout=15):
            """The tab-created of a popup of the opener since `since`: its tab."""
            m = log.wait(r'host event: \{"event":"tab-created","tab":(\d+),"opener":' + str(tabs["opener"]) +
                         r',"cause":"' + cause + '"', since, timeout)
            return int(m.group(1))

        def blank_opened(since, timeout=15):
            """C-FOCUSEXISTING: a plain target=_blank click is handed to the shell (`blank-link`), which
            opens it with `new-tab` (web.buffers.focusExisting is off: no init.js here): the new tab."""
            log.wait(r'host event: \{"event":"blank-link","tab":' + str(tabs["opener"]) + r',"url":"' +
                     re.escape(child("blank")) + '"', since, timeout)
            m = log.wait(r'host event: \{"event":"tab-created","tab":(\d+),"req":\d+\}', since, timeout)
            return int(m.group(1))

        tabs = {}

        def blocked():
            shows(RED, "opener.png")
            for l in log.lines:
                mm = re.search(r'host event: \{"event":"tab-created","tab":(\d+)\}', l)
                if mm:
                    tabs["opener"] = int(mm[1])
            if "opener" not in tabs:
                raise Fail("no first tab-created")
            t = tabs["opener"]
            log.wait(r'host event: \{"event":"popup-blocked","tab":' + str(t) + r',"popup":1,"url":"' +
                     re.escape(child("timer")) + r'","name":"tkid","frame":"main"', 0, 10)
            log.wait(r'host event: \{"event":"popup-blocked","tab":' + str(t) + r',"popup":2,"url":"' +
                     re.escape(child("frame")) + r'","name":"fkid","frame":"sub"', 0, 10)
            opener_title(r"timer=null", 0)
            opener_title(r"frame open=null", 0)
            bar(0, r"NORMAL \| popup blocked: " + re.escape(child("frame")))
            if any('"tab-created"' in l and '"opener"' in l for l in log.lines):
                raise Fail("a blocked popup created a page")
            return "timer window.open (main frame) and the iframe's: popup-blocked 1 and 2, window.open returned null, bar says so"

        step("T96 a popup without a gesture is blocked and the bar says so", blocked)

        def search():
            k = log.mark()
            inject(*keys_for(":popupSearch"), "key:del", "key:ret", "wait:300")
            bar(k, r">\| pick popups \[1/2 of 2\] popup 2")
            shot("popup-search.png")
            k = log.mark()
            inject(*keys_for("timer"), "wait:200")
            bar(k, r">timer\| pick popups \[1/1 of 2\] popup 1")
            k = log.mark()
            inject("key:ret", "wait:300")
            tab = created(k, "popup-open")
            log.wait(r'host event: \{"event":"title","tab":' + str(tab) + r',"title":"child timer opener=yes"', k, 15)
            opener_title(r"hello from child timer", k)
            log.wait(r"buffer 2 of 2 is current", k, 5)
            bar(k, r"NORMAL \| buffer 2: blocked popup opened from buffer 1")
            shows(GREEN, "timer-popup.png")
            k = command("bd", r"NORMAL \| buffer 2 killed")
            log.wait(r"buffer 1 of 1 is current", k, 5)
            return f"picker listed 2, `timer` + Enter: tab {tab} (popup-open), child has window.opener, the opener heard it"

        step("T98/T97 :popupSearch lists the blocked popups; Enter opens one with its opener", search)

        def popup_open():
            k = command("popupOpen")
            bar(k, r"NORMAL \| (opening blocked popup|buffer 3)")
            tab = created(k, "popup-open")
            log.wait(r'host event: \{"event":"title","tab":' + str(tab) + r',"title":"child frame opener=yes"', k, 15)
            opener_title(r"frame got hello from child frame", k)
            log.wait(r"buffer 3 of 2 is current", k, 5)
            shows(GREEN, "frame-popup.png")
            k = command("bd", r"NORMAL \| buffer 3 killed")
            log.wait(r"buffer 1 of 1 is current", k, 5)
            k = command("popupOpen", r"NORMAL \| web: no blocked popup in this buffer")
            return f"tab {tab}: the iframe's window.open re-run in the iframe; the child's opener is the iframe, which heard it"

        step("T97 :popupOpen re-runs the latest blocked popup in its frame (an iframe)", popup_open)

        def gesture():
            shows(RED, "opener-again.png")
            k = log.mark()
            click("btn")
            tab = created(k, "popup")
            tabs["click"] = tab
            log.wait(r'host event: \{"event":"title","tab":' + str(tab) + r',"title":"child click opener=yes"', k, 15)
            opener_title(r"click=win", k)
            opener_title(r"hello from child click", k)
            log.wait(r"buffer 4 of 2 is current", k, 5)
            bar(k, r"NORMAL \| buffer 4: popup from buffer 1")
            log.wait(r"buffer 4 shown: page window of tab " + str(tab), k, 10)
            shows(GREEN, "click-popup.png")
            return f"click: window.open returned a window, tab {tab} is buffer 4 (shown, green), its window.opener posted back"

        step("T93 a click's window.open becomes a buffer that posts to its opener", gesture)

        def close():
            k = log.mark()
            inject(f"move:{g.bx + g.ww // 2},{g.by + g.wh // 3}", "wait:100", "click", "wait:300")
            log.wait(r'host event: \{"event":"tab-closed","tab":' + str(tabs["click"]) + r"\}", k, 10)
            bar(k, r"NORMAL \| buffer 4 closed by the page")
            log.wait(r"buffer 1 of 1 is current", k, 5)
            if any('"cmd":"close-tab"' in l for l in log.lines[k:]):
                raise Fail("the shell sent a close-tab for a page that closed itself")
            log.wait(r'host event: \{"event":"title","tab":' + str(tabs["opener"]) + r',"title":"pp ', 0, 1)
            shows(RED, "after-close.png")
            return "window.close() in the popup: its page closed, buffer 4 gone, the opener (buffer 1) shown again"

        step("T95 window.close() in a popup kills only its buffer", close)

        def blank():
            k = log.mark()
            click("blank")
            tab = blank_opened(k)
            log.wait(r'host event: \{"event":"title","tab":' + str(tab) + r',"title":"child blank opener=no"', k, 15)
            log.wait(r"buffer 5 of 2 is current", k, 5)
            shows(GREEN, "blank-popup.png")
            if any('"url","tab":' + str(tabs["opener"]) in l for l in log.lines[k:]):
                raise Fail("the opener navigated")
            k = command("bd", r"NORMAL \| buffer 5 killed")
            log.wait(r"buffer 1 of 1 is current", k, 5)
            return f"target=_blank click: tab {tab} is buffer 5 (blank-link: the shell opened it, no window.opener); the opener stayed"

        step("T94 a target=_blank link opens a buffer", blank)

        def modifiers_in_normal():
            # C-NORMALMODS: NORMAL clicks carry the held modifiers, so they act as in INPUT: nothing opens.
            x, y = AT["blank"]
            for held in ("c", "s", "cs"):
                k = log.mark()
                inject(f"move:{g.bx + x},{g.by + y}", "wait:100", f"hold:{held}", "click", "release", "wait:1500")
                if any('"tab-created"' in l or '"event":"url"' in l for l in log.lines[k:]):
                    raise Fail(f"{held}-click on the target=_blank link opened or loaded something: {log.lines[k:]}")
            # C-MIDCLICK: a middle-click is `:new <url>` (an open-link, not a popup: no opener).
            k = log.mark()
            inject(f"move:{g.bx + x},{g.by + y}", "wait:100", "mclick", "wait:300")
            log.wait(r'host event: \{"event":"open-link","tab":' + str(tabs["opener"]) + r',"url":"' + re.escape(child("blank")), k, 10)
            log.wait(r"buffer 6 of 2 is current", k, 15)
            if any('"tab-created"' in l and '"opener"' in l for l in log.lines[k:]):
                raise Fail("a middle-click made a popup")
            k = command("bd", r"NORMAL \| buffer 6 killed")
            log.wait(r"buffer 1 of 1 is current", k, 5)
            # a plain click right after still opens a buffer (the modifiers were restored; blank-link)
            k = log.mark()
            click("blank")
            tab = blank_opened(k)
            log.wait(r"buffer 7 of 2 is current", k, 15)
            k = command("bd", r"NORMAL \| buffer 7 killed")
            log.wait(r"buffer 1 of 1 is current", k, 5)
            return "NORMAL: Ctrl/Shift/Ctrl+Shift-click on target=_blank open nothing; a middle-click opens buffer 6 (no opener); a plain click opens buffer 7"

        step("T92 modifier clicks in NORMAL (as INPUT: nothing opens)", modifiers_in_normal)

        def scroll_after_shift_click():
            # NORMAL's own keys stay unmodified after a Shift-click: G -> End scrolls, gg back, j scrolls.
            x, y = AT["btn"]
            k = log.mark()
            inject(f"move:{g.bx + x},{g.by + y + 600}", "wait:100", "hold:s", "click", "release", "wait:300")
            inject(*keys_for("G"), "wait:600")
            opener_title(r" y=[1-9]\d{3}", k)
            k = log.mark()
            inject(*keys_for("gg"), "wait:600")
            # no ` y=` any more: back at the top
            log.wait(r'host event: \{"event":"title","tab":' + str(tabs["opener"]) + r',"title":"pp (?:(?!y=)[^"])*"', k, 10)
            k = log.mark()
            inject(*keys_for("j"), "wait:600")
            opener_title(r" y=[1-9]", k)
            return "after a Shift-click, G scrolled to the end, gg back to the top, j scrolled"

        step("T21 NORMAL keys still scroll unmodified after a Shift-click", scroll_after_shift_click)

        def nav_clears():
            # A reload is another document: its timers block popups 3 and 4.
            k = command("refresh")
            log.wait(r'host event: \{"event":"popup-blocked","tab":' + str(tabs["opener"]) + r',"popup":4,', k, 15)
            k = log.mark()
            inject(*keys_for(":popupSearch"), "key:del", "key:ret", "wait:300")
            bar(k, r">\| pick popups \[1/2 of 2\] popup 4")
            inject("key:esc", "wait:200")
            # Another document again: popups 3 and 4 go with the old one; its own are 5 and 6.
            k = command("go " + opener_url + "?again")
            log.wait(r'host event: \{"event":"popup-blocked","tab":' + str(tabs["opener"]) + r',"popup":5,', k, 15)
            log.wait(r'host event: \{"event":"popup-blocked","tab":' + str(tabs["opener"]) + r',"popup":6,', k, 15)
            k = log.mark()
            inject(*keys_for(":popupSearch"), "key:del", "key:ret", "wait:300")
            bar(k, r">\| pick popups \[1/2 of 2\] popup 6")
            inject("key:esc", "wait:200")
            return "after another document only its own blocked popups are listed (2, not 4)"

        step("T96 another document clears the blocked list", nav_clears)

        def quit_all():
            k = command("q")
            code = proc.wait(timeout=30)
            if code != 0:
                raise Fail(f"web exited {code}")
            log.wait(r"engine host exited: exit status: 0", k, 5)
            return "web and its host exited 0"

        step("quit", quit_all)
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
