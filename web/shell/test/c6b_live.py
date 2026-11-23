#!/usr/bin/env python3
"""C6b live check: HiDPI and fractional scale in the headless nested dwl (web/tools/nest.sh),
never on the user's display. Runs at the nest's scale:

    NEST_SCALE=1.5 web/shell/test/c6b_live.py --out .qwork/c6b-1.5 --inject .qwork/inject/release/inject
    NEST_SCALE=2   web/shell/test/c6b_live.py --out .qwork/c6b-2   --inject .qwork/inject/release/inject

(NEST_SCALE goes on to nest.sh, which replaces a running nest of another scale.) Steps: the
page's devicePixelRatio is the shell's scale and its CSS viewport the page window's logical
size; a click lands on the CSS pixel aimed at; the page is drawn at the output's resolution
(a 1 CSS px line is pure black device pixels with no grey from scaling); the bar is 24 logical
px tall with text in it; then the output's scale is changed while the shell runs (`inject
output`, wlr-output-management, as moving the window to another monitor) and the page follows
(devicePixelRatio, the same CSS size, clicks still land), and the scale is put back.
Screenshots go into --out (never commit them). Exit 0 when every step passed.

Needs what test/live.py needs, and PIL."""

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

SHELL = live.SHELL
WEB = live.WEB
Fail = live.Fail
BAR_BG = (24, 24, 24)
BAR_H = 24
# test/pages/c6b.html
BOX = (200, 150)  # the centre of #box, CSS px
HAIR_Y, HAIR_X = 120, 550  # a 1 px black line, CSS px
CUR = (200, 450)  # inside #cur (cursor: url(32x32 magenta png) 0 0), CSS px
CUR_PX = 32  # the cursor image's size in CSS px


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
    from PIL import Image

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"

    unit = f"web-c6b-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/c6b.html"]
    proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(out, "web.log"))
    results = []
    restore = None

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

    def title(pattern, since, timeout=10):
        return log.wait(r'host event: \{"event":"title","tab":\d+,"title":"' + pattern, since, timeout)

    try:
        log.wait(r'"title":"c6b dpr:', timeout=60)
        g0 = livelib.settle(log, shot)
        ww, wh = g0.ww, g0.wh
        geo = {}

        def measure(name, S):
            p = shot(name)
            ow, oh = live.output_size(p, S)
            geo.update(S=S, ow=ow, oh=oh, bx=(ow - ww) // 2, by=(oh - wh) // 2, png=p)
            return S

        def shell_has(S):
            got = live.shell_scale(log.lines)
            if abs(got - S) > 1e-6:
                raise Fail(f"the shell logged scale {got}, the output's is {S}")
            return f"window {ww}x{wh} logical at {S:g}"

        S0 = measure("start.png", live.nest_scale())
        print(f"output {geo['ow']}x{geo['oh']} at scale {S0}, shell window {ww}x{wh} at {geo['bx']},{geo['by']}")
        step("the shell window takes the output's scale", lambda: shell_has(S0))

        def inject(*cmds):
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + geo['bx']},{int(mm[2]) + geo['by']}" if mm else c)
            subprocess.run([a.inject, str(geo["ow"]), str(geo["oh"])] + args, env=nest_env, check=True)

        def dpr(since=0, timeout=10):
            # The page sets its title on load, before the shell window is configured (release builds
            # show 1200x800 first) and again on every resize. Wait for the latest title to match the
            # settled page window (livelib.settle gave ww x wh); fail with the latest one seen.
            S = geo["S"]
            pw, ph = ww, wh - BAR_H
            rx = re.compile(r'host event: \{"event":"title","tab":\d+,"title":"c6b dpr:([\d.]+) css:(\d+)x(\d+)')
            end = time.monotonic() + timeout
            t = None
            while True:
                with log.cv:
                    seen = [m for m in (rx.search(l) for l in log.lines[since:]) if m]
                t = seen[-1] if seen else None
                if t and abs(float(t[1]) - S) <= 1e-6 and (int(t[2]), int(t[3])) == (pw, ph):
                    break
                if time.monotonic() > end:
                    if not t:
                        raise Fail("no c6b title")
                    raise Fail(f"latest title: devicePixelRatio {t[1]} CSS {t[2]}x{t[3]}; "
                               f"the shell's scale is {S}, the page window {pw}x{ph} logical px")
                time.sleep(0.1)
            return f"devicePixelRatio {t[1]} = the shell's scale; CSS viewport {t[2]}x{t[3]} = the page window"

        step("devicePixelRatio is the output's scale", dpr)

        def click():
            k = log.mark()
            # The nest's virtual pointer keeps its position across runs. On a reused nest it can already
            # sit exactly on BOX (the last run's click), under a window that is new to it: moving to
            # the same point is no motion, the compositor never gives the new window pointer focus, and
            # the click goes nowhere. So arrive from elsewhere, as a real pointer does.
            inject("move:1,1", "wait:100", f"move:{BOX[0]},{BOX[1]}", "wait:150", "click", "wait:300")
            t = title(r"c6b click:(\d+),(\d+)", k)
            x, y = int(t[1]), int(t[2])
            if abs(x - BOX[0]) > 1 or abs(y - BOX[1]) > 1:
                raise Fail(f"the click landed at CSS {x},{y}, aimed at {BOX[0]},{BOX[1]} (scale {geo['S']})")
            return f"aimed at CSS {BOX[0]},{BOX[1]}, landed at {x},{y}"

        step("a click lands where aimed", click)

        def crisp():
            S, bx, by = geo["S"], geo["bx"], geo["by"]
            im = Image.open(shot("crisp.png")).convert("RGB")
            x = round((bx + HAIR_X) * S)
            y0 = round((by + HAIR_Y) * S)
            # From 2 device px above the line to just before the next one (2 CSS px further down).
            col = [im.getpixel((x, y)) for y in range(y0 - 2, y0 + round(2 * S))]
            black = sum(1 for c in col if c == (0, 0, 0))
            grey = [c for c in col if c not in ((0, 0, 0), (255, 255, 255))]
            if black == 0:
                raise Fail(f"no black line at device row ~{y0}: {col}")
            if grey:
                raise Fail(f"grey pixels at the 1 px line: the page was scaled, not drawn at {S}: {col}")
            if black < int(S):
                raise Fail(f"the 1 CSS px line is {black} device px at scale {S}")
            return f"a 1 CSS px line is {black} pure black device px, no grey (drawn at {S}, not stretched)"

        step("the page is drawn at the output's resolution", crisp)

        def cursor():
            """C6b-CURSOR: a CSS cursor: url() image is drawn CUR_PX logical px wide, i.e. CUR_PX * scale
            device px, at the pointer (hotspot 0 0), not 1x-sized or doubled by a buffer scale."""
            S, bx, by = geo["S"], geo["bx"], geo["by"]
            inject("move:1,1", "wait:100", f"move:{CUR[0]},{CUR[1]}", "wait:100", f"move:{CUR[0] + 1},{CUR[1]}",
                   f"move:{CUR[0]},{CUR[1]}", "wait:500")
            im = Image.open(shot("cursor.png")).convert("RGB")
            w, h = im.size
            px = im.load()
            xs, ys = [], []
            for y in range(h):
                for x in range(w):
                    r, g, b = px[x, y]
                    if r > 230 and g < 40 and b > 230:
                        xs.append(x)
                        ys.append(y)
            if not xs:
                raise Fail(f"no magenta cursor image in the screenshot ({os.path.join(out, 'cursor.png')}); "
                           f"cursor lines: {[l for l in log.lines if 'cursor:' in l][-3:]}")
            bw, bh = max(xs) - min(xs) + 1, max(ys) - min(ys) + 1
            want = round(CUR_PX * S)
            if abs(bw - want) > 2 or abs(bh - want) > 2:
                raise Fail(f"the cursor image is {bw}x{bh} device px at scale {S}, want {want}x{want} ({CUR_PX} CSS px)")
            ex, ey = round((bx + CUR[0]) * S), round((by + CUR[1]) * S)
            if abs(min(xs) - ex) > 2 or abs(min(ys) - ey) > 2:
                raise Fail(f"the cursor image starts at {min(xs)},{min(ys)}, the pointer is at {ex},{ey} (hotspot 0 0)")
            return f"a {CUR_PX} CSS px cursor image is {bw}x{bh} device px at {S:g}, at the pointer"

        step("a CSS cursor: url() image is drawn at its CSS size", cursor)

        def bar():
            S, bx, by = geo["S"], geo["bx"], geo["by"]
            im = Image.open(shot("bar.png")).convert("RGB")
            px = lambda v: round(v * S)  # noqa: E731
            x = px(bx + ww) - 3
            rows = [y for y in range(px(by + wh - BAR_H) - 6, px(by + wh) + 2) if im.getpixel((x, y)) == BAR_BG]
            want = px(BAR_H)
            if abs(len(rows) - want) > 1:
                raise Fail(f"bar {len(rows)} px tall at scale {S}, want {want}")
            top = rows[0]
            ink = sum(1 for xx in range(px(bx), px(bx + 120)) for y in range(top, top + want) if im.getpixel((xx, y)) != BAR_BG)
            if ink < 40 * S * S:
                raise Fail(f"only {ink} px of text in the bar's left")
            return f"bar {len(rows)} px = {BAR_H} logical px at {S}; {ink} px of text"

        step("the bar is 24 logical px with text", bar)

        def rescale():
            nonlocal restore
            S1 = 2.0 if S0 != 2.0 else 1.5
            ow, oh = geo["ow"], geo["oh"]
            restore = (round(ow * S0), round(oh * S0), S0)
            k = log.mark()
            subprocess.run([a.inject, "output", str(round(ow * S1)), str(round(oh * S1)), str(S1)], env=nest_env, check=True,
                           capture_output=True)
            log.wait(r"scale [\d.]+ -> %s\b" % re.escape(f"{S1:g}"), k)
            title(r"c6b dpr:%s css" % re.escape(f"{S1:g}"), k)
            time.sleep(0.5)
            measure("rescaled.png", S1)
            detail = dpr(k)
            clicked = click()
            crisp_ok = crisp()
            cur_ok = cursor()
            return f"{S0:g} -> {S1:g}: {detail}; {clicked}; {crisp_ok}; {cur_ok}"

        step("a scale change while running re-configures the page", rescale)

        def back():
            nonlocal restore
            w, h, s = restore
            k = log.mark()
            subprocess.run([a.inject, "output", str(w), str(h), str(s)], env=nest_env, check=True, capture_output=True)
            restore = None
            log.wait(r"scale [\d.]+ -> %s\b" % re.escape(f"{s:g}"), k)
            title(r"c6b dpr:%s css" % re.escape(f"{s:g}"), k)
            return f"back to {s:g}"

        if restore:
            step("and back", back)
    finally:
        if restore:
            w, h, s = restore
            subprocess.run([a.inject, "output", str(w), str(h), str(s)], env=nest_env, capture_output=True)
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
