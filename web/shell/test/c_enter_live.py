#!/usr/bin/env python3
"""C-ENTER + C-CARETCMD live check: Enter in NORMAL goes to the page (KEYS.md T18-T20) and
`:set caret` / `:set nocaret` / `:set caret?` switch Chromium's caret browsing at runtime.
Driven with injected input inside the headless nested dwl (web/tools/nest.sh), never on the
user's display.

    web/shell/test/c_enter_live.py --out .qwork/center --inject .qwork/inject/release/inject

The page (pages/center.html) reports in its title its scrollY, the selection's anchor
(`sel=<element id>:<offset>`, where caret browsing puts the caret), the focused element, the
Enters it saw and the button presses. What happened is read from the shell's log (bar, commands,
host events and the host's own `caret:` line). Steps marked "(observed)" only report what the
engine did. Screenshots go into --out (never commit them). Exit 0 when every step passed."""

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
from c3_live import keys_for  # noqa: E402

SHELL = live.SHELL
WEB = live.WEB
Fail = live.Fail
BAR = r"\] bar: "
RUNS = 10  # straight `/alpha` Enter Enter repetitions (the find-clear / Enter ordering)
TITLE = re.compile(r'"event":"title","tab":\d+,"title":"(ce [^"]*)"')


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

    unit = f"web-center-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/center.html"]
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
        log.wait(r'"title":"ce y=0 ', timeout=60)
        geo = livelib.settle(log, shot)
        ow, oh, ww, wh, bx, by = geo.ow, geo.oh, geo.ww, geo.wh, geo.bx, geo.by

        def inject(*cmds):
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + bx},{int(mm[2]) + by}" if mm else c)
            subprocess.run([a.inject, str(ow), str(oh)] + args, env=nest_env, check=True)

        def bar(pattern, since, timeout=6):
            return log.wait(BAR + pattern, since, timeout)

        def bar_now():
            for l in reversed(log.lines):
                if "] bar: " in l:
                    return l.split("bar: ", 1)[1]
            return ""

        def page():
            """The page's last report as a dict (y, sel, act, enter, btn)."""
            time.sleep(0.4)
            for l in reversed(log.lines):
                m = TITLE.search(l)
                if m:
                    return dict(kv.split("=", 1) for kv in m[1].split()[1:])
            raise Fail("no page title yet")

        def sent(since):
            # The shell's own lines only (the host logs the commands it gets too).
            return [m[1] for l in log.lines[since:] if (m := re.match(r"\[web \+\d+ms\] command: (.*)$", l))]

        def loads(since):
            return [l for l in log.lines[since:] if '"event":"load"' in l]

        def command(line, wait=600):
            inject(*(keys_for(":" + line) + ["key:ret", f"wait:{wait}"]))

        def search(query):
            k = log.mark()
            inject("key:slash", "wait:150", *keys_for(query), "key:ret", "wait:900")
            return bar(r"NORMAL \| (?:search hit \w+, continuing at \w+  )?\[(\d+)/(\d+)\]", k)

        def to_normal():
            if not bar_now().startswith("NORMAL"):
                inject("key:esc", "wait:80", "key:esc", "wait:400")
            inject("key:esc", "wait:300")  # T7: clear the highlights

        def t20():
            k = log.mark()
            before = page()
            inject("key:ret", "wait:800")
            p = page()
            if int(p["enter"]) != int(before["enter"]) + 1:
                raise Fail(f"the page saw no Enter: {before} -> {p}")
            if loads(k) or sent(k):
                raise Fail(f"Enter with nothing focused did something: {loads(k)} {sent(k)}")
            if p["btn"] != "0":
                raise Fail(f"button pressed: {p}")
            return f"the page got the Enter (keydown seen, act={p['act']}); no load, no command, mode {bar_now()!r}"
        step("T19: Enter with nothing focused reaches the page and does nothing", t20)

        def t7_then_enter():
            m = search("alpha")
            p = page()
            inject("key:esc", "wait:600")
            p2 = page()
            k = log.mark()
            inject("key:ret", "wait:300")
            log.wait(r'"title":"c2 second page"', k, timeout=10)
            k2 = log.mark()
            inject("key:h", "wait:1500")
            log.wait(r'"title":"ce ', k2)
            return f"/alpha: focus act={p['act']} while the find is open; Esc (T7, find-clear) -> act={p2['act']}; Enter followed the link"
        step("T7 T18: / finds a link, Esc, Enter follows it", t7_then_enter)

        def t18_link():
            # Straight `/alpha` Enter Enter, ten times: the shell sends find-clear, then (after
            # ENTER_AFTER_FIND_CLEAR_MS) the Enter; the Enter must land on the link every time.
            for i in range(RUNS):
                to_normal()
                m = search("alpha")
                p = page()
                if i == 0:
                    shot("link-found.png")
                k = log.mark()
                inject("key:ret", "wait:300")
                log.wait(r'"title":"c2 second page"', k, timeout=10)
                url = log.wait(r'"event":"url","tab":\d+,"url":"([^"]*)"', k)[1]
                if not url.endswith("/second.html"):
                    raise Fail(url)
                if sent(k) != ['{"cmd":"find-clear"}']:
                    raise Fail(f"the shell sent {sent(k)}")
                k2 = log.mark()
                inject("key:h", "wait:1500")
                log.wait(r'"title":"ce ', k2)
            return (f"/alpha -> [{m[1]}/{m[2]}] (focus act={p['act']} while the find is open); Enter: find-clear, "
                    f"then the Enter followed the link to {url}; {RUNS}/{RUNS} runs; h back")
        step(f"T18: / finds a link, Enter follows it (x{RUNS})", t18_link)

        def t18_button():
            to_normal()
            m = search("press me")
            p = page()
            inject("key:ret", "wait:800")
            p2 = page()
            if p2["btn"] == "0":
                raise Fail(f"button not pressed: focus after the find act={p['act']}, after Enter {p2}")
            return f"/press me -> [{m[1]}/{m[2]}], focus act={p['act']}; Enter pressed the button (btn={p2['btn']})"
        step("T18: / finds a button, Enter presses it", t18_button)

        def caret_query_off():
            to_normal()
            k = log.mark()
            command("set caret?")
            bar(r"NORMAL \| nocaret$", k)
            if sent(k):
                raise Fail(f"sent {sent(k)}")
            return ":set caret? -> nocaret (off by default), nothing sent"
        step(":set caret? at start", caret_query_off)

        caret_notes = {}

        def place(word):
            """Puts the caret (the selection) on `word` with the keyboard: `/word` Enter, then
            Esc (T7: find-clear keeps the match selected; caret browsing starts from there)."""
            to_normal()
            search(word)
            inject("key:esc", "wait:500")
            return page()

        def caret_on():
            inject("key:g", "key:g", "wait:600")
            k = log.mark()
            command("set caret")
            m = log.wait(r"caret: settings\.a11y\.caretbrowsing\.enabled=true: set=(\d) now=(\S+)", k)
            if m[1] != "1" or m[2] != "Some(true)":
                raise Fail(f"host: set={m[1]} now={m[2]}")
            if '"cmd":"caret","on":true' not in " ".join(sent(k)):
                raise Fail(f"{sent(k)}")
            bar(r"NORMAL \| caret browsing on", k)
            p0 = place("first caret")
            inject("key:j", "wait:500")
            p1 = page()
            inject("key:j", "wait:500")
            p2 = page()
            shot("caret-on.png")
            if loads(k):
                raise Fail(f"a load happened: {loads(k)}")
            if p1["sel"] == p0["sel"] and p2["sel"] == p1["sel"]:
                raise Fail(f"j did not move the caret: {p0} {p1} {p2}")
            caret_notes["j"] = f"/first caret + Esc -> sel={p0['sel']}; j -> {p1['sel']} (y={p1['y']}); j -> {p2['sel']} (y={p2['y']})"
            return f"host set the pref (set=1, read back true), no reload; {caret_notes['j']}"
        step(":set caret: caret browsing on without a reload, j moves the caret", caret_on)

        def caret_keys():
            obs = []
            for name, cmds in [("k", ["key:k"]), ("Right", ["key:right"]), ("Right", ["key:right"]),
                               ("Left", ["key:left"]), ("G", ["hold:s", "key:g", "release"]),
                               ("gg", ["key:g", "key:g"]), ("Down", ["key:down"]),
                               ("Ctrl-d", ["hold:c", "key:d", "release"]), ("Space", ["key:space"])]:
                inject(*cmds, "wait:500")
                p = page()
                obs.append(f"{name}: sel={p['sel']} act={p['act']} y={p['y']}")
            caret_notes["keys"] = "; ".join(obs)
            return "(observed) " + caret_notes["keys"]
        step("caret on: k, arrows, G, gg, Ctrl-d, Space (observed)", caret_keys)

        def caret_link_enter():
            place("first caret")
            inject("key:j", "wait:500")
            p = page()
            k = log.mark()
            inject("key:ret", "wait:300")
            try:
                log.wait(r'"title":"c2 second page"', k, timeout=4)
                went = True
                k2 = log.mark()
                inject("key:h", "wait:1500")
                log.wait(r'"title":"ce ', k2)
            except Fail:
                went = False
            return f"(observed) caret moved by j into the link: sel={p['sel']} act={p['act']}; Enter followed it: {went}"
        step("caret on: caret in a link, Enter (observed)", caret_link_enter)

        def caret_into_field():
            p0 = place("fourth line")
            k = log.mark()
            seen = []
            for _ in range(6):
                if bar_now().startswith("INPUT"):
                    break
                inject("key:j", "wait:500")
                p = page()
                seen.append(f"{p['sel']}/{p['act']}/y={p['y']}")
            focused = any(re.search(r'"editable-focused","tab":\d+,"focused":true', l) for l in log.lines[k:])
            mode = bar_now().split(" |")[0]
            shot("caret-field.png")
            to_normal()
            return (f"(observed) /fourth line + Esc -> {p0['sel']}; j down: {' -> '.join(seen)}; "
                    f"editable-focused true: {focused}; mode: {mode}")
        step("caret on: moving the caret into a text field (observed)", caret_into_field)

        def caret_off():
            to_normal()
            k = log.mark()
            command("set caret?")
            bar(r"NORMAL \| caret$", k)
            k = log.mark()
            command("set nocaret")
            m = log.wait(r"caret: settings\.a11y\.caretbrowsing\.enabled=false: set=(\d) now=(\S+)", k)
            if m[1] != "1" or m[2] != "Some(false)":
                raise Fail(f"host: set={m[1]} now={m[2]}")
            bar(r"NORMAL \| caret browsing off", k)
            inject("key:g", "key:g", "wait:600")
            p0 = place("first caret")
            inject("key:j", "wait:200", "key:j", "wait:200", "key:j", "wait:700")
            p1 = page()
            if int(p1["y"]) <= int(p0["y"]):
                raise Fail(f"j did not scroll with caret off: {p0} -> {p1}")
            if p1["sel"] != p0["sel"]:
                raise Fail(f"the selection moved with caret off: {p0} -> {p1}")
            return f":set caret? -> caret; :set nocaret -> host set false; j scrolls again (y {p0['y']} -> {p1['y']}), sel stays {p1['sel']}"
        step(":set nocaret: j scrolls again", caret_off)

        def field_match():
            to_normal()
            k = log.mark()
            m = search("fieldword")
            time.sleep(0.6)
            p = page()

            def focused(since):
                return any(re.search(r'"editable-focused","tab":\d+,"focused":true', l) for l in log.lines[since:])
            f1, mode1 = focused(k), bar_now().split(" |")[0]
            shot("field-found.png")
            k2 = log.mark()
            inject("key:ret", "wait:800")
            p2 = page()
            f2, mode2 = focused(k2), bar_now().split(" |")[0]
            shot("field-enter.png")
            to_normal()
            return (f"(observed) /fieldword -> [{m[1]}/{m[2]}]: act={p['act']}, editable-focused true: {f1}, mode {mode1}; "
                    f"Enter (find-clear + Enter): act={p2['act']} sel={p2['sel']}, editable-focused true: {f2}, mode {mode2}, "
                    f"page Enters {p['enter']}->{p2['enter']}")
        step("T29: a find match inside a text field (observed)", field_match)

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
