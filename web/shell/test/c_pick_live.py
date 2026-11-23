#!/usr/bin/env python3
"""C-PICK live check: the picker (KEYS.md "Picker", T58-T68) in the headless nested dwl
(web/tools/nest.sh), never on the user's display.

    web/shell/test/c_pick_live.py --out .qwork/c-pick --inject .qwork/inject/release/inject

The state directory (--out/state) gets a seeded history file before the shell starts. Steps:
`:historySearch` opens the picker over the history (the bar logs `>| pick history [1/N of N] <url>`)
and the page window keeps its size; a grim screenshot shows the list drawn right above the bar
(header + rows, the selection lighter) at scale 1; typing filters; Ctrl-n / Down / Ctrl-p / Up move;
Enter loads the selected URL in the page; Esc closes without loading anything; then the output is
switched to scale 2 (`inject output`, as c6b_live.py does) and the list is drawn at twice the pixels,
and the scale is put back. Screenshots go into --out (never commit them). Exit 0 when every step
passed. Needs what test/live.py needs, and PIL."""

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
BAR_BG = (24, 24, 24)
BAR_H = 24
# config.rs PICKER_*
ROW_H = 22
MAX_ROWS = 12
PICK_BG = (36, 36, 36)
SEL_BG = (60, 72, 96)
PICK = re.compile(r"\] bar: >([^|]*)\| pick history \[(\d+)/(\d+) of (\d+)\] (\S*)")


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

    # The seeded history (STASH.md "History"): pick3 (a page that is never loaded) has more visits than
    # pick2, so `target` ranks it first and reaching pick2 takes a move.
    seed = [
        (1000, f"{base}/pick2.html", "Target two"),
        (1001, f"{base}/pick3.html", "Target three"),
        (1002, f"{base}/pick3.html", "Target three"),
        (1003, "https://alpha.invalid/", "Alpha home"),
        (1004, "https://beta.invalid/docs", "Beta docs"),
    ]
    with open(os.path.join(state, "history"), "w") as f:
        os.chmod(f.name, 0o600)
        for ms, url, title in seed:
            f.write(f"{ms}\t{url}\t{title}\n")

    unit = f"web-c-pick-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/pick.html"]
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

    try:
        log.wait(r'"title":"pick start"', timeout=60)
        g = livelib.settle(log, shot)
        ww, wh = g.ww, g.wh
        geo = dict(S=g.scale, ow=g.ow, oh=g.oh, bx=g.bx, by=g.by)
        print(f"output {g.ow}x{g.oh} at scale {g.scale}, shell window {ww}x{wh} at {g.bx},{g.by}")

        def inject(*cmds):
            subprocess.run([a.inject, str(geo["ow"]), str(geo["oh"])] + list(cmds), env=nest_env, check=True)

        def pick_state(since, filt=None, timeout=6):
            """The picker's logged state after `since` (filter, selected, matches, total, selected URL)."""
            pat = r"\] bar: >" + (re.escape(filt) if filt is not None else r"[^|]*") + r"\| pick history"
            log.wait(pat, since, timeout)
            time.sleep(0.15)
            for l in reversed(log.lines):
                if "] bar: " in l:
                    m = PICK.search(l)
                    if not m:
                        raise Fail(f"the bar is no longer the picker: {l}")
                    return m[1], int(m[2]), int(m[3]), int(m[4]), m[5]
            raise Fail("no bar line")

        def opens(since):
            # The shell's own lines only (the host logs the commands it gets too).
            return [m[1] for l in log.lines[since:] if (m := re.match(r'\[web \+\d+ms\] command: \{"cmd":"open","url":"([^"]+)"', l))]

        def open_picker():
            k = log.mark()
            inject(*keys_for(":historySearch"), "key:ret", "wait:300")
            return k, pick_state(k, "")

        def picker_drawn(name, rows):
            """The list right above the bar at the current scale: (rows + 1) * ROW_H logical px of the
            picker's background at the window's right edge, the first row selected, text in the header."""
            S, bx, by = geo["S"], geo["bx"], geo["by"]
            im = Image.open(shot(name)).convert("RGB")
            px = lambda v: round(v * S)  # noqa: E731
            x = px(bx + ww) - 3
            bar_top = px(by + wh - BAR_H)
            if im.getpixel((x, bar_top + 2)) != BAR_BG:
                raise Fail(f"no bar at device row {bar_top}: {im.getpixel((x, bar_top + 2))}")
            y = bar_top - 1
            colours = set()
            while y > 0 and im.getpixel((x, y)) in (PICK_BG, SEL_BG):
                colours.add(im.getpixel((x, y)))
                y -= 1
            got = bar_top - 1 - y
            want = (rows + 1) * ROW_H * S
            if abs(got - want) > 1:
                raise Fail(f"the list is {got} device px tall above the bar, want {want} ({rows} rows + header at {S})")
            top = y + 1
            sel_y = top + px(ROW_H) + px(ROW_H) // 2
            if im.getpixel((x, sel_y)) != SEL_BG:
                raise Fail(f"the first row is not selected: {im.getpixel((x, sel_y))} at device row {sel_y}")
            above = im.getpixel((x, max(top - 3, 0)))
            ink = sum(1 for xx in range(px(bx), px(bx + 200)) for yy in range(top, top + px(ROW_H))
                      if im.getpixel((xx, yy)) not in (PICK_BG, SEL_BG))
            if ink < 30 * S * S:
                raise Fail(f"only {ink} px of text in the header")
            return f"{got} device px = ({rows} rows + header) x {ROW_H} x {S:g}, on the bar; page above it {above}; header ink {ink} px"

        state0 = {}

        def t58_open():
            sizes0 = len([l for l in log.lines if "page window is" in l])
            k, (filt, sel, n, total, url) = open_picker()
            if (sel, n) != (1, total) or total < 5:
                raise Fail(f"picker [{sel}/{n} of {total}], want every history entry with the first selected")
            time.sleep(0.5)
            sizes1 = len([l for l in log.lines if "page window is" in l])
            if sizes1 != sizes0:
                raise Fail("the page window was resized when the picker opened")
            state0.update(total=total)
            return f"{total} history rows, first selected {url}; the page window kept its size"

        step("T58 :historySearch opens the picker over the seeded history", t58_open)

        step("the list is drawn above the bar at scale 1",
             lambda: picker_drawn("pick-scale1.png", min(state0.get("total", 0), MAX_ROWS)))

        def t59_t62():
            k = log.mark()
            inject(*keys_for("target"), "wait:200")
            filt, sel, n, total, url = pick_state(k, "target")
            if n != 2 or not url.endswith("/pick3.html"):
                raise Fail(f"`target`: {n} matches, first {url}; want 2, pick3 (more visits) first")
            k = log.mark()
            inject("key:down", "wait:200")
            _, sel, _, _, url = pick_state(k, "target")
            if sel != 2 or not url.endswith("/pick2.html"):
                raise Fail(f"Down: [{sel}] {url}")
            k = log.mark()
            inject("hold:c", "key:p", "release", "wait:200")
            _, sel, _, _, url = pick_state(k, "target")
            if sel != 1:
                raise Fail(f"Ctrl-p: [{sel}] {url}")
            k = log.mark()
            inject("hold:c", "key:n", "release", "wait:200")
            _, sel, _, _, url = pick_state(k, "target")
            if sel != 2 or not url.endswith("/pick2.html"):
                raise Fail(f"Ctrl-n: [{sel}] {url}")
            k = log.mark()
            inject("key:ret", "wait:300")
            log.wait(r'"title":"pick target two"', k, 10)
            o = opens(k)
            if o != [f"{base}/pick2.html"]:
                raise Fail(f"Enter sent {o}")
            log.wait(r"\] bar: NORMAL", k, 5)
            return f"`target` -> 2 rows, pick3 first; Down/Ctrl-p/Ctrl-n moved; Enter: open {o[0]}, the page loaded"

        step("T59-T62 type to filter, move, Enter loads the chosen URL", t59_t62)

        def t64_esc():
            k, _ = open_picker()
            k2 = log.mark()
            inject(*keys_for("alpha"), "wait:200")
            _, _, n, _, url = pick_state(k2, "alpha")
            if url != "https://alpha.invalid/":
                raise Fail(f"`alpha` selects {url}")
            k3 = log.mark()
            inject("key:esc", "wait:1500")
            log.wait(r"\] bar: NORMAL", k3, 5)
            if opens(k):
                raise Fail(f"Esc loaded {opens(k)}")
            if any('"event":"load"' in l for l in log.lines[k3:]):
                raise Fail("a load started after Esc")
            return "closed, NORMAL, no open command"

        step("T64 Esc closes without action", t64_esc)

        def scale2():
            nonlocal restore
            S0, S1 = geo["S"], 2.0 if geo["S"] != 2.0 else 1.0
            restore = (round(geo["ow"] * S0), round(geo["oh"] * S0), S0)
            k = log.mark()
            subprocess.run([a.inject, "output", str(round(geo["ow"] * S1)), str(round(geo["oh"] * S1)), str(S1)],
                           env=nest_env, check=True, capture_output=True)
            log.wait(r"scale [\d.]+ -> %s\b" % re.escape(f"{S1:g}"), k)
            time.sleep(1.0)
            p = shot("rescaled.png")
            ow, oh = live.output_size(p, S1)
            geo.update(S=S1, ow=ow, oh=oh, bx=(ow - ww) // 2, by=(oh - wh) // 2)
            _, (_, _, n, total, _) = open_picker()
            time.sleep(0.3)
            drawn = picker_drawn(f"pick-scale{S1:g}.png", min(n, MAX_ROWS))
            inject("key:esc", "wait:300")
            return f"scale {S1:g}: {drawn}"

        step("the list is drawn above the bar at scale 2", scale2)

        def back():
            nonlocal restore
            w, h, s = restore
            k = log.mark()
            subprocess.run([a.inject, "output", str(w), str(h), str(s)], env=nest_env, check=True, capture_output=True)
            restore = None
            log.wait(r"scale [\d.]+ -> %s\b" % re.escape(f"{s:g}"), k)
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
