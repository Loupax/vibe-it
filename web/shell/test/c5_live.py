#!/usr/bin/env python3
"""C5 live check: search (`/`, Enter, `n`, `N`, Esc, the messages) of web-shell through the
host's find, driven with injected input inside the headless nested dwl (web/tools/nest.sh),
never on the user's display.

    web/shell/test/c5_live.py --out .qwork/c5 --inject .qwork/inject/release/inject

The page (pages/c5.html) has six matches of "needle" (one is "Needle" with a capital) at known
heights and reports its scrollY in its title. What happened is read from the shell's log (the
status bar as `bar: ...`, every command sent as `command: ...`, every host event). Screenshots
go into --out (never commit them). Each step names the KEYS.md rows it shows. Exit 0 when every
step passed. Needs what c3_live.py needs."""

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
MATCHES = 6  # "needle" is in the page six times, case-insensitively


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

    unit = f"web-c5-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/c5.html"]
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
        log.wait(r'"title":"c5 y=0"', timeout=60)
        geo = livelib.settle(log, shot)
        ow, oh = geo.ow, geo.oh

        def inject(*cmds):
            subprocess.run([a.inject, str(ow), str(oh)] + list(cmds), env=nest_env, check=True)

        def bar(pattern, since, timeout=6):
            return log.wait(BAR + pattern, since, timeout)

        def last_y():
            for l in reversed(log.lines):
                m = re.search(r'"event":"title","tab":\d+,"title":"c5 y=(\d+)"', l)
                if m:
                    return int(m[1])
            return -1

        def settle_y():
            """scrollY once it stopped changing (smooth scrolling)."""
            time.sleep(0.4)
            prev = last_y()
            for _ in range(10):
                time.sleep(0.3)
                y = last_y()
                if y == prev:
                    return y
                prev = y
            return prev

        def bar_now():
            """The last status bar state logged (the shell logs only changes)."""
            for l in reversed(log.lines):
                if "] bar: " in l:
                    return l.split("bar: ", 1)[1]
            return ""

        def sent(since):
            return [l.split("command: ", 1)[1] for l in log.lines[since:] if "command: " in l]

        def search(query):
            k = log.mark()
            inject("key:slash", "wait:150", *keys_for(query), "key:ret", "wait:700")
            return k

        def key(name, wait=600):
            k = log.mark()
            if name == "N":
                inject("hold:s", "key:n", "release", f"wait:{wait}")
            else:
                inject(f"key:{name}", f"wait:{wait}")
            return k

        def step_index(k, want_wrap=None):
            m = bar(r"NORMAL \| (search hit \w+, continuing at \w+  )?\[(\d+)/(\d+)\]", k)
            wrap, i, n = m[1], int(m[2]), int(m[3])
            if n != MATCHES:
                raise Fail(f"{n} matches, want {MATCHES}")
            if want_wrap is True and not wrap:
                raise Fail(f"index {i}: no wrap message")
            if want_wrap is False and wrap:
                raise Fail(f"index {i}: unexpected {wrap!r}")
            return i, (wrap or "").strip()

        first = {}

        def t17():
            k = key("n")
            bar(r"NORMAL \| No previous search$", k)
            k = key("N")
            if bar_now() != "NORMAL | No previous search":
                raise Fail(bar_now())
            if sent(k):
                raise Fail("a find was sent with no query")
            return "n and N before any search: 'No previous search', nothing sent"
        step("T17: No previous search", t17)

        def t8_t23():
            k = log.mark()
            inject("key:slash", "wait:200")
            bar(r"/\|", k)
            shot("search-line.png")
            inject(*keys_for("needle"), "wait:200")
            bar(r"/needle\|", k)
            shot("search-typed.png")
            inject("key:ret", "wait:900")
            cmds = sent(k)
            if not any('"query":"needle","dir":"next"' in c for c in cmds):
                raise Fail(f"no find command: {cmds}")
            i, _ = step_index(k, want_wrap=False)
            first["i"] = i
            y = settle_y()
            shot("search-first.png")
            return f"/ opened the line (prompt /), 'needle' typed, Enter: find sent; CEF made match {i}/{MATCHES} current at scrollY={y} (page top was 0)"
        step("T8 T22 T23: / needle Enter -> [i/N]", t8_t23)

        def n_walk_wraps():
            # From the current match, n six times: every index once, one BOTTOM wrap.
            seen, wraps = [first["i"]], 0
            for _ in range(MATCHES):
                k = key("n")
                i, wrap = step_index(k)
                if wrap:
                    wraps += 1
                    if wrap != "search hit BOTTOM, continuing at TOP":
                        raise Fail(wrap)
                    if i != 1:
                        raise Fail(f"wrapped to {i}, not 1")
                seen.append(i)
            if wraps != 1:
                raise Fail(f"{wraps} wrap messages in {seen}")
            return f"n x{MATCHES}: indices {seen}, one 'search hit BOTTOM, continuing at TOP' (landing on 1)"
        step("T15: n steps and wraps at the bottom", n_walk_wraps)

        def N_wraps_top():
            k = key("N")
            i, wrap = step_index(k)
            if not (i == MATCHES and wrap == "search hit TOP, continuing at BOTTOM"):
                raise Fail(f"N from 1 gave {i} {wrap!r}")
            y = settle_y()
            k = key("N")
            i2, wrap2 = step_index(k, want_wrap=False)
            if i2 != MATCHES - 1:
                raise Fail(f"N gave {i2}")
            return f"N from 1 -> {i} with the TOP wrap message (scrollY {y}); N again -> {i2}, no wrap"
        step("T15: N steps back and wraps at the top", N_wraps_top)

        def scrolls_into_view():
            inject("key:g", "wait:100", "key:g", "wait:700")
            settle_y()
            k = key("n", wait=900)
            i, _ = step_index(k)
            y = settle_y()
            if y <= 0 and i != 1:
                raise Fail(f"match {i} not scrolled to, y={y}")
            return f"after gg, n -> match {i}, scrollY {y} (the engine scrolls the match into view, S4)"
        step("S4: the engine scrolls a match into view", scrolls_into_view)

        def not_found():
            k = search("zzzz")
            bar(r"NORMAL \| Pattern not found: zzzz$", k)
            for name in ("n", "N"):
                k = key(name)
                if not any('"query":"zzzz"' in c for c in sent(k)):
                    raise Fail(f"{name}: no find sent")
                if bar_now() != "NORMAL | Pattern not found: zzzz":
                    raise Fail(bar_now())
            shot("not-found.png")
            return "/zzzz, n and N: 'Pattern not found: zzzz'"
        step("T16 T23: Pattern not found", not_found)

        def esc_cancel():
            k = search("needle")
            step_index(k)
            before = log.mark()
            inject("key:slash", "wait:150", *keys_for("abc"), "wait:150")
            bar(r"/abc\|", before)
            inject("key:esc", "wait:400")
            bar(r"NORMAL \| \[\d+/\d+\]$", before)
            if any('"query":"abc"' in c for c in sent(before)):
                raise Fail("the cancelled search was run")
            k = key("n")
            if not any('"query":"needle"' in c for c in sent(k)):
                raise Fail("n did not use the kept query")
            step_index(k)
            return "/needle, then /abc Esc: no find sent, the bar is back to the old [i/N]; n still searches 'needle' (T25)"
        step("T25: Esc cancels the search line", esc_cancel)

        def empty_enter():
            before = log.mark()
            inject("key:slash", "wait:150", "key:ret", "wait:400")
            bar(r"NORMAL \| \[\d+/\d+\]$", before)
            if sent(before):
                raise Fail(f"commands sent: {sent(before)}")
            return "/ Enter with nothing typed: same as Esc, nothing sent (T24)"
        step("T24: empty Enter is Esc", empty_enter)

        def esc_normal():
            k = log.mark()
            inject("key:esc", "wait:500")
            bar(r"NORMAL \| $", k)
            if not any('"cmd":"find-clear"' in c for c in sent(k)):
                raise Fail("no find-clear")
            shot("highlights-cleared.png")
            k = log.mark()
            inject("key:esc", "wait:300")
            if sent(k):
                raise Fail("a second Esc sent something")
            k = key("n")
            i, wrap = step_index(k, want_wrap=False)
            return f"Esc in NORMAL: find-clear, the bar's [i/N] gone, still NORMAL; a second Esc sends nothing; n then searched again (kept query) -> {i}/{MATCHES}"
        step("T7: Esc in NORMAL clears the highlights, keeps the query", esc_normal)

        def smartcase():
            k = search("Needle")
            if not any('"query":"Needle","dir":"next","match_case":true' in c for c in sent(k)):
                raise Fail(f"no match_case: {sent(k)}")
            m = bar(r"NORMAL \| \[(\d+)/(\d+)\]", k)
            if m[2] != "1":
                raise Fail(f"{m[2]} matches for Needle, want 1")
            k = search("needle")
            step_index(k)
            return "/Needle -> 1 match (case sensitive: smartcase), /needle -> 6"
        step("smartcase: a capital matches case", smartcase)

        def fragment_navigation():
            # C5-FRAG: a same-document navigation gets no load callbacks; the host reports it
            # as `load ... same-document` and the shell ends the find session (find-clear).
            k = search("needle")
            step_index(k)
            k = log.mark()
            inject(*keys_for(":go " + base + "/c5.html#frag"), "key:ret", "wait:1200")
            log.wait(r'"event":"load","tab":\d+,"phase":"start","status":0,"kind":"same-document"', k)
            if not any('"cmd":"find-clear"' in c for c in sent(k)):
                raise Fail(f"no find-clear after the fragment change: {sent(k)}")
            bar(r"NORMAL \| $", k)
            k2 = key("n")
            i, wrap = step_index(k2, want_wrap=False)
            return f"#frag: same-document load -> find-clear, the bar's [i/N] gone, kept query; n searched again -> {i}/{MATCHES}"
        step("T31: a same-document navigation clears the highlights", fragment_navigation)

        def navigation():
            k = log.mark()
            inject(*keys_for(":go " + base + "/second.html"), "key:ret", "wait:1500")
            log.wait(r'"title":"c2 second page"', k)
            bar(r"NORMAL \| $", k)
            k2 = key("n")
            bar(r"NORMAL \| Pattern not found: needle$", k2)
            k3 = log.mark()
            inject("key:h", "wait:1500")
            log.wait(r'"title":"c5 y=\d+"', k3)
            k4 = key("n")
            i, wrap = step_index(k4, want_wrap=False)
            return f"after :go the old [i/N] was cleared and n on the other page says 'Pattern not found: needle' (kept query, T31); h back, n -> {i}/{MATCHES}"
        step("T31: navigation clears the highlights and keeps the query", navigation)

        def t23_from_the_top_of_the_screen():
            # A new document (after h) has no selection: where does the engine's first match lie
            # when the screen is already scrolled? The matches are at y=400, 1300, 2200, 2700, 3600, 5000.
            inject("key:esc", "wait:200")
            inject("hold:c", "key:d", "release", "wait:800", "hold:c", "key:d", "release", "wait:800")
            y = settle_y()
            k = search("needle")
            i, wrap = step_index(k)
            y2 = settle_y()
            shot("search-from-scrolled.png")
            return f"screen scrolled to y={y}; /needle made match {i}/{MATCHES} current (scrollY {y2}). KEYS T23 wants the first match at or below the top of the screen"
        step("T23: where the engine starts when the screen is scrolled", t23_from_the_top_of_the_screen)

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
