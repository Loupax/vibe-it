#!/usr/bin/env python3
"""C3 live check: the keys, the status bar and the command line of web-shell, driven with
injected input inside the headless nested dwl (web/tools/nest.sh), never on the user's display.

    web/shell/test/c3_live.py --out .qwork/c3 --inject .qwork/inject/release/inject

Each step names the KEYS.md rows it shows. What happened is read from the shell's log (the
status bar is logged as `bar: ...` whenever it changes, every command sent as `command: ...`,
every host event) and from the test page, which reports its scroll position and the keys it
saw (keydown and keyup) in its title. Screenshots of the bar go into --out (never commit them).
The state directory is --out/state (its history file is checked at the end). Exit 0 when
every step passed.

Needs what test/live.py needs (C2): web/shell/target/debug/web, web/host-cef/target/debug/
web-host-cef, the inject tool, grim, dwl; and PIL for the bar pixels."""

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
import live  # noqa: E402  (the C2 check's page server and log follower)

SHELL = live.SHELL
WEB = live.WEB
Fail = live.Fail
BAR_BG = (24, 24, 24)
BAR_H = 24
SHIFTED = {":": ";", "?": "/", "_": "-", "@": "2", "+": "=", "%": "5"}


def keys_for(text):
    """inject commands typing `text`; shifted characters hold a real Shift key, because the
    shell's keymap sees modifiers only through key events (inject's `type:` sends modifier
    state alone, which smithay's winit backend does not pass on)."""
    cmds = []
    for ch in text:
        if ch == " ":
            cmds.append("key:space")
        elif ch in SHIFTED or ch.isupper():
            cmds += ["hold:s", f"key:{SHIFTED.get(ch, ch.lower())}", "release"]
        else:
            cmds.append(f"key:{ch}")
        cmds.append("wait:20")
    return cmds


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

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"

    unit = f"web-c3-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/c3.html"]
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
        log.wait(r'"title":"c3 y=0 ', timeout=60)
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

        def title(pattern, since, timeout=10):
            return log.wait(r'host event: \{"event":"title","tab":\d+,"title":"' + pattern, since, timeout)

        def last_title():
            for l in reversed(log.lines):
                m = re.search(r'"event":"title","tab":\d+,"title":"(c3 [^"]*)"', l)
                if m:
                    return m[1]
            return ""

        def page_saw():
            m = re.fullmatch(r"c3 y=(\d+) down=(.*) up=(.*)", last_title())
            if not m:
                raise Fail(f"bad title {last_title()!r}")
            return int(m[1]), [k for k in m[2].split(",") if k], [k for k in m[3].split(",") if k]

        def bar(pattern, since, timeout=5):
            return log.wait(r"\] bar: " + pattern, since, timeout)

        def command(line, wait=600):
            """`:` + line + Enter, typed for real."""
            inject(*(keys_for(":" + line) + ["key:ret", f"wait:{wait}"]))

        def scroll_y(since, cond, what, timeout=6):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                y, _, _ = page_saw()
                if cond(y):
                    return y
                time.sleep(0.2)
            raise Fail(f"scrollY {page_saw()[0]}: wanted {what}")

        def start_normal():
            bar(r"NORMAL \| $", 0)
            p = shot("bar-normal.png")
            from PIL import Image
            im = Image.open(p).convert("RGB")
            top = by + wh - BAR_H
            ink = sum(1 for x in range(bx, bx + ww // 3) for y in range(top, top + BAR_H) if im.getpixel((x, y)) != BAR_BG)
            if ink < 40:
                raise Fail(f"only {ink} text pixels in the left of the bar")
            bg = im.getpixel((bx + ww - 3, top + 2))
            if bg != BAR_BG:
                raise Fail(f"bar background {bg}")
            return f"starts in NORMAL; the bar has text ({ink} px of ink in its left third)"
        step("START_MODE: NORMAL, the status bar drawn", start_normal)

        def jk():
            k = log.mark()
            inject("key:j", "wait:700")
            y = scroll_y(k, lambda y: y > 0, "> 0 after j")
            _, down, up = page_saw()
            if "j" in down or "j" in up:
                raise Fail(f"the page saw j: {down} {up}")
            if "ArrowDown" not in down:
                raise Fail(f"no ArrowDown given to the page: {down}")
            inject("key:k", "wait:700")
            y2 = scroll_y(k, lambda v: v < y, f"< {y} after k")
            return f"j: y=0 -> {y} (the page got ArrowDown, never j); k: -> {y2}"
        step("T13 + T21: j / k scroll by real keys, the page never sees j/k", jk)

        def bottom_top():
            k = log.mark()
            inject("hold:s", "key:g", "release", "wait:900")
            y = scroll_y(k, lambda y: y > 3000, "the bottom after G")
            inject("key:g", "wait:100", "key:g", "wait:900")
            scroll_y(k, lambda y: y == 0, "0 after gg")
            inject("hold:c", "key:d", "release", "wait:900")
            y2 = scroll_y(k, lambda y: y > 300, "a page down after Ctrl-d")
            inject("key:home", "wait:900")
            scroll_y(k, lambda y: y == 0, "0 after Home")
            return f"G -> y={y}, gg -> 0, Ctrl-d -> {y2}, Home -> 0"
        step("T10 T11 T13: G, gg, Ctrl-d, Home", bottom_top)

        def search_not_yet():
            k = log.mark()
            inject("key:slash", "wait:300")
            bar(r"NORMAL \| web: search is not built yet", k)
            inject("key:esc", "wait:200")
            return "`/` says search is C5; Esc stays in NORMAL (T7)"
        step("T7 T8: / says not yet, Esc stays NORMAL", search_not_yet)

        def input_mode():
            k = log.mark()
            inject("key:i", "wait:300")
            bar(r"INPUT \|", k)
            inject("key:x", "wait:400")
            title(r'c3 y=0 down=[^"]*x up=[^"]*x', k)
            _, down, up = page_saw()
            if "i" in down or "i" in up:
                raise Fail(f"the page saw the i of T6: {down} {up}")
            k2 = log.mark()
            inject("key:esc", "wait:80", "key:esc", "wait:500")
            bar(r"NORMAL \|", k2)
            _, down2, up2 = page_saw()
            if down2.count("Escape") != 1 or up2.count("Escape") != 1:
                raise Fail(f"the page should see exactly the first Esc: {down2} / {up2}")
            inject("key:x", "wait:400")
            _, down3, _ = page_saw()
            if down3.count("x") != 1:
                raise Fail(f"x in NORMAL reached the page: {down3}")
            return "i -> INPUT (the page got x, never i); Esc Esc -> NORMAL, the page saw one Esc down and up"
        step("T6 T5 T1 T2 T34 T21: i, typing, Esc Esc", input_mode)

        def slow_esc():
            k = log.mark()
            inject("key:i", "wait:300", "key:esc", "wait:600", "key:esc", "wait:500")
            if any("bar: NORMAL" in l for l in log.lines[k:]):
                raise Fail("two Escs 600 ms apart entered NORMAL")
            inject("key:esc", "wait:100", "key:esc", "wait:400")
            bar(r"NORMAL \|", k)
            return "Esc, 600 ms, Esc stays INPUT (T3); then a quick Esc Esc enters NORMAL"
        step("T3: a slow second Esc is a new first Esc", slow_esc)

        def click_field():
            k = log.mark()
            inject("move:190,60", "wait:150", "click", "wait:600")
            log.wait(r"editable focused: INPUT", k)
            bar(r"INPUT \|", k)
            inject("type:ab", "wait:300")
            inject("key:esc", "wait:80", "key:esc", "wait:400")
            bar(r"NORMAL \|", k)
            return "a click into the text field switched to INPUT; Esc Esc back"
        step("T29: a click into a text field enters INPUT", click_field)

        def script_focus():
            inject("move:600,400", "wait:100", "click", "wait:400")  # blur the field
            k = log.mark()
            inject("move:550,60", "wait:100", "click", "wait:2500")
            log.wait(r'"editable-focused","tab":\d+,"focused":true', k)
            bar(r"INPUT \|", k)
            inject("key:esc", "wait:80", "key:esc", "wait:400")
            bar(r"NORMAL \|", k)
            inject("move:600,400", "wait:100", "click", "wait:400")
            return "the page focused the field 1.5 s after a button click: INPUT (T32, user 2026-11-23)"
        step("T32: the page's own focus() enters INPUT", script_focus)

        def go():
            k = log.mark()
            command(f"go {base}/second.html", wait=1200)
            log.wait(r'command: \{"cmd":"open","url":"' + re.escape(f"{base}/second.html") + '"', k)
            title("c2 second page", k)
            bar(r"NORMAL \| $", k)
            return ":go typed for real; the page loaded; still NORMAL (T31)"
        step("T9 T26 T27 T31: :go", go)

        def history_keys():
            k = log.mark()
            inject("key:h", "wait:1200")
            log.wait(r'command: \{"cmd":"back"\}', k)
            title("c3 ", k)
            k = log.mark()
            inject("key:l", "wait:1200")
            log.wait(r'command: \{"cmd":"forward"\}', k)
            title("c2 second page", k)
            return "h went back to c3.html, l forward to second.html"
        step("T14: h / l", history_keys)

        def ghost():
            k = log.mark()
            inject(*(keys_for(":go 127") + ["wait:300"]))
            g = bar(r":go 127\|(\.0\.0\.1:\d+/)", k)
            inject("key:tab", "wait:300")
            g2 = bar(r":go 127\|(\.0\.0\.1:\d+/\S+)", k)
            shot("bar-ghost.png")
            inject("key:ret", "wait:1200")
            want = "https://127" + g2[1]
            log.wait(r'command: \{"cmd":"open","url":"' + re.escape(want) + '"', k)
            return f"ghost {g[1]!r} (the site root first), Tab -> {g2[1]!r}, Enter opened {want} (https:// assumed, QCOMP6)"
        step("T45 T47 T50: the :go ghost from history", ghost)

        def back_to_c3():
            k = log.mark()
            command(f"go {base}/c3.html", wait=1200)
            title("c3 ", k)
        step("back to the test page", back_to_c3)

        def refresh():
            k = log.mark()
            command("refresh", wait=1200)
            log.wait(r'command: \{"cmd":"reload"\}', k)
            log.wait(r'"event":"load","tab":\d+,"phase":"end","status":200,"kind":"reload"', k)
            k = log.mark()
            command("refresh hard", wait=1200)
            log.wait(r'command: \{"cmd":"reload","hard":true\}', k)
            log.wait(r'"event":"load","tab":\d+,"phase":"end","status":200,"kind":"reload"', k)
            return ":refresh and :refresh hard reloaded the page"
        step(":refresh, :refresh hard", refresh)

        def help_page():
            k = log.mark()
            command("help", wait=1500)
            title("web help", k)
            shot("help.png")
            k = log.mark()
            command("q", wait=1200)
            log.wait(r'command: \{"cmd":"back"\}', k)
            title("c3 ", k)
            return ":help showed the help file; :q there went back"
        step(":help and :q on it", help_page)

        def let_yy():
            k = log.mark()
            command("let @+ = @%", wait=300)
            bar(r"NORMAL \| web: @\+ \(and yy\) need the clipboard", k)
            k = log.mark()
            command("let @% = @+", wait=300)
            bar(r"NORMAL \| web: register @% is read-only", k)
            k = log.mark()
            inject("key:y", "wait:50", "key:y", "wait:300")
            bar(r"NORMAL \| web: @\+ \(and yy\) need the clipboard", k)
            return ":let and yy say they need the clipboard (C6); @% is read-only"
        step(":let @+ = @%, yy (C6 stubs)", let_yy)

        def esc_cancels():
            k = log.mark()
            inject(*(keys_for(":q") + ["wait:200", "key:esc", "wait:400"]))
            bar(r"NORMAL \|", k)
            if proc.poll() is not None:
                raise Fail("Esc ran :q")
            return ":q then Esc: cancelled, still running"
        step("T28: Esc cancels the line", esc_cancels)

        def load_error():
            k = log.mark()
            command("go http://web-c3-check.invalid/", wait=2500)
            log.wait(r'"event":"load-error"', k)
            bar(r"NORMAL \| web: load failed: ERR_NAME_NOT_RESOLVED \(-105\)", k)
            shot("bar-error.png")
            return "the load error is in the status bar"
        step("load-error in the status bar", load_error)

        def history_file():
            with open(os.path.join(state, "history")) as f:
                lines = [l.rstrip("\n").split("\t") for l in f]
            urls = [l[1] for l in lines]
            # The ghost step's https:// guess failed (the server is plain http): no visit.
            want =[f"{base}/c3.html", f"{base}/second.html", f"{base}/c3.html"]
            if urls != want:
                raise Fail(f"history {urls}, want {want}")
            titles = [l[2] for l in lines]
            return f"{len(lines)} visits ({', '.join(u.rsplit('/', 1)[-1] for u in urls)}), titles {titles}; no reload, back/forward, help or failed load"
        step("history file (STASH.md)", history_file)

        def quit_():
            k = log.mark()
            command("q", wait=300)
            log.wait(r"quitting: :q", k)
            try:
                code = proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                raise Fail("the shell did not exit")
            if code != 0:
                raise Fail(f"exit {code}")
            return ":q quit web (exit 0)"
        step(":q quits", quit_)
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
