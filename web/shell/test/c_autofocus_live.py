#!/usr/bin/env python3
"""C-AUTOFOCUS live check (KEYS.md T29, T32, T13, T6, `gi`): only a focus the user caused switches to
INPUT; a field the page focuses itself is blurred ("parked") and NORMAL stays, `i` gives it the focus
back, `gi` focuses the last (else the first visible) text field. Headless nested dwl
(web/tools/nest.sh), never the user's display; a PRIVATE D-Bus session bus
(web/tools/notify-probe/private-bus.conf, no service activation), never the user's session bus.

    web/shell/test/c_autofocus_live.py --out .qwork/autofocus --inject .qwork/inject/release/inject

Pages (test/pages): autofocus.html (an autofocus field with a blur handler that shows a validation
message, a second field, a tall body), twofields.html (a read-only field, then two fields),
nofield.html, refocus.html (focuses its field again on every blur). What happened is read from the
shell's log (`bar:` lines, `command:` lines, host events) and the pages' titles. Steps:
  1. autofocus.html opens in NORMAL: the field was parked (one `blur-field`, the page saw one blur)
  2. `j` scrolls the page (the field does not take the arrow key), `gg` back to the top
  3. `i`: INPUT and `focus-field last`; typed text lands in the autofocused field
  4. a click into the second field: INPUT (T29); typing lands there
  5. `gi` after Esc Esc: the last field (the second one) again, INPUT
  6. twofields.html: `gi` focuses the first visible text field (not the read-only one); a click
     into the other, Esc Esc, `gi`: that one
  7. nofield.html: `gi` says so in the bar, NORMAL stays
  8. refocus.html: one blur, then the guard leaves the page's field focused; NORMAL, no blur loop
Exit 0 when every step passed. Needs what test/live.py needs."""

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
PARKED = r"editable focused: the page's own: parked"
KEPT = r"editable focused: the page's own again: kept"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    if not os.environ.get("C_AUTOFOCUS_PRIVATE_BUS"):
        # Again, inside a fresh private session bus that ends with us (as c_bell_live.py).
        env = dict(os.environ, C_AUTOFOCUS_PRIVATE_BUS="1")
        conf = os.path.join(WEB, "tools/notify-probe/private-bus.conf")
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

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"

    unit = f"web-autofocus-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/autofocus.html"]
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
        log.wait(r'"title":"af ', timeout=60)
        geo = livelib.settle(log, shot)
        ow, oh, bx, by = geo.ow, geo.oh, geo.bx, geo.by
        print(f"output {ow}x{oh}, shell window {geo.ww}x{geo.wh} at {bx},{by}; pages at {base}")

        def inject(*cmds):
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + bx},{int(mm[2]) + by}" if mm else c)
            subprocess.run([a.inject, str(ow), str(oh)] + args, env=nest_env, check=True)

        def bar(pattern, since, timeout=5):
            return log.wait(r"\] bar: " + pattern, since, timeout)

        def mode():
            last = [l for l in log.lines if "] bar: " in l]
            return "INPUT" if last and "] bar: INPUT |" in last[-1] else "NORMAL"

        def no_input(since, what):
            if any("] bar: INPUT |" in l for l in log.lines[since:]):
                raise Fail(what)

        def title(pattern, since, timeout=5):
            return log.wait(r'"event":"title","tab":\d+,"title":"' + pattern, since, timeout)

        def last_title(prefix):
            ts = [m[1] for m in (re.search(r'"event":"title","tab":\d+,"title":"(' + prefix + r'[^"]*)"', l) for l in log.lines) if m]
            return ts[-1] if ts else ""

        def count(pattern, since):
            return sum(1 for l in log.lines[since:] if re.search(r"\[web \+\d+ms\] " + pattern, l))

        def keys(text, wait=300):
            inject(*(keys_for(text) + [f"wait:{wait}"]))

        def esc_esc(since):
            inject("key:esc", "wait:80", "key:esc", "wait:400")
            bar(r"NORMAL \|", since)

        def go(page, ready):
            k = log.mark()
            inject(*(keys_for(f":go {base}/{page}") + ["key:ret", "wait:800"]))
            title(ready, k, timeout=10)
            return k

        def opens_parked():
            log.wait(PARKED, 0, timeout=10)
            log.wait(r'"event":"field-result","tab":\d+,"op":"blur","ok":true', 0)
            title(r"af y=0 a=\[\] b=\[\] blurs=\d+ focus=none", 0)
            no_input(0, "the autofocus switched to INPUT")
            n = count(r'command: \{"cmd":"blur-field"', 0)
            if n != 1:
                raise Fail(f"{n} blur-field commands, wanted 1")
            shot("af-normal.png")
            first = last_title("af ")
            # Activate the page window with a click on an empty spot (NORMAL: nothing to focus). In the
            # nest the session's first click does not reach the page; a second one is sent if needed.
            k = log.mark()
            inject("move:700,600", "wait:150", "click", "wait:800")
            note = ""
            if "md=0" in last_title("af "):
                inject("move:700,620", "wait:150", "click", "wait:800")
                note = " (the session's first click did not reach the page; a second one did)"
            title(r"af .* md=[1-9]", k)
            no_input(k, "a click on an empty spot switched to INPUT")
            return f"NORMAL, one blur-field; page: {first}{note}"
        step("1 T32: an autofocus page opens in NORMAL, the field parked", opens_parked)

        def j_scrolls():
            k = log.mark()
            inject("key:j", "wait:60", "key:j", "wait:60", "key:j", "wait:600")
            m = title(r"af y=([1-9]\d*) ", k)
            y = m[1]
            inject("key:g", "wait:40", "key:g", "wait:700")
            title(r"af y=0 ", k)
            if mode() != "NORMAL":
                raise Fail("mode changed")
            return f"j j j scrolled to y={y} (T13: the arrow key reached the page, not a field); gg back to 0"
        step("2 T13: j scrolls the page, not the parked field's caret", j_scrolls)

        def i_restores():
            k = log.mark()
            inject("key:i", "wait:400")
            log.wait(r'command: \{"cmd":"focus-field","which":"last"\}', k)
            bar(r"INPUT \|", k)
            log.wait(r'"event":"field-result","tab":\d+,"op":"last","ok":true', k)
            keys("xyz")
            title(r"af y=0 a=\[xyz\] b=\[\] blurs=\d+ focus=a", k)
            esc_esc(k)
            return "i -> INPUT + focus-field last; typed xyz into the autofocus field"
        step("3 T6: i gives the parked field the focus back", i_restores)

        def click_field():
            k = log.mark()
            inject("move:190,140", "wait:150", "click", "wait:600")
            log.wait(r"editable focused: INPUT", k)
            bar(r"INPUT \|", k)
            keys("q")
            title(r"af y=0 a=\[xyz\] b=\[q\] blurs=\d+ focus=b", k)
            esc_esc(k)
            return "a click into the second field: INPUT; typed q there"
        step("4 T29: a click into a field still enters INPUT", click_field)

        def gi_last():
            k = log.mark()
            inject("key:g", "wait:40", "key:i", "wait:600")
            log.wait(r'command: \{"cmd":"focus-field","which":"last"\}', k)
            bar(r"INPUT \|", k)
            keys("r")
            title(r"af y=0 a=\[xyz\] b=\[qr\] .*focus=b", k)
            esc_esc(k)
            return "gi (the field still had the focus after Esc Esc): INPUT, typed r into it"
        step("5 gi: the last field", gi_last)

        def gi_first():
            k = go("twofields.html", r"two one=\[\] two=\[\] focus=none")
            no_input(k, "loading a page switched to INPUT")
            k = log.mark()
            inject("key:g", "wait:40", "key:i", "wait:600")
            log.wait(r'"event":"field-result","tab":\d+,"op":"last","ok":false', k)
            log.wait(r'command: \{"cmd":"focus-field","which":"first"\}', k)
            bar(r"INPUT \|", k)
            keys("1")
            title(r"two one=\[1\] two=\[\] focus=one", k)
            esc_esc(k)
            k2 = log.mark()
            inject("move:190,220", "wait:150", "click", "wait:600")
            bar(r"INPUT \|", k2)
            esc_esc(k2)
            inject("key:g", "wait:40", "key:i", "wait:600")
            bar(r"INPUT \|", k2)
            keys("2")
            title(r"two one=\[1\] two=\[2\] focus=two", k2)
            esc_esc(k2)
            return "gi with no field focused yet: the first visible text field (read-only skipped); after a click into the other: that one"
        step("6 gi: the first visible field, then the last", gi_first)

        def gi_none():
            go("nofield.html", "nofield ready")
            k = log.mark()
            inject("key:g", "wait:40", "key:i", "wait:600")
            bar(r"NORMAL \| .*gi: no text field on this page", k)
            no_input(k, "gi with no field switched to INPUT")
            shot("gi-none.png")
            return "gi: no text field on this page; NORMAL"
        step("7 gi: no field", gi_none)

        def refocus():
            k = go("refocus.html", r"rf ")
            log.wait(PARKED, k, timeout=5)
            log.wait(KEPT, k, timeout=5)
            time.sleep(2.5)  # past CLICK_FOCUS_MS: no second blur may come
            blurs = count(r'command: \{"cmd":"blur-field"', k)
            if blurs != 1:
                raise Fail(f"{blurs} blur-field commands, wanted 1")
            t = last_title("rf ")
            if "blurs=1 focus=a" not in t:
                raise Fail(f"page says {t!r}")
            no_input(k, "the refocusing page switched to INPUT")
            return f"one blur, the page took the focus back, kept (guard), NORMAL; page: {t}"
        step("8 refocus guard: a page that refocuses on blur", refocus)

        def blur_handlers():
            k = go("autofocus.html", r"af ")
            log.wait(PARKED, k, timeout=5)
            t = title(r"af y=0 a=\[\] b=\[\] blurs=1 focus=none msg=\[required\]", k)
            no_input(k, "the autofocus switched to INPUT")
            shot("af-blur-handler.png")
            return f"with the page window active, parking runs the page's blur handler: {last_title('af ')}"
        step("9 the park runs the page's blur handlers (risk check)", blur_handlers)
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
