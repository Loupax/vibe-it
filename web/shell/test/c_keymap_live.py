#!/usr/bin/env python3
"""C-KEYMAP live check: the shell types with the desktop's keyboard layout, not xkbcommon's default.
Inside the headless nested dwl (web/tools/nest.sh), never on the user's display.

    NEST_XKB_LAYOUT=de web/tools/nest.sh --new
    web/shell/test/c_keymap_live.py --out .qwork/keymap-de --inject .qwork/inject/release/inject --layout de [--x11]
    NEST_XKB_LAYOUT=fr ...  --layout fr
    (no NEST_XKB_LAYOUT: dwl's own list us,de,gr)  --layout group
    --x11 --layout x11      the shell is an X11 window; setxkbmap on the nested Xwayland changes the layout

The keys are injected as raw evdev codes (the physical keys: inject ev:CODE), so what the page
and the shell's own bindings see is decided only by the keymaps. The page (pages/keymap.html) reports
its textarea's value in its title. Layouts:
  de     Y and Z swap: physical Y types z; ':' is Shift+period, '/' is Shift+7
  fr     A and Q swap: physical A types q; ':' is the period key, '/' is Shift+period
  group  dwl's layouts us,de,gr: group 0 types yz, `inject group:1` (Alt+Shift's result) types zy
  x11    X11 window on the Xwayland (us); setxkbmap de, then fr, then us: each types its own letters
The shell's log says `desktop keymap: <name>` for each keymap it was given. Exit 0 when every step passed."""

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
BAR = r"\] bar: "
# evdev codes
Y, Z, A, Q, DOT, N7 = 21, 44, 30, 16, 52, 8


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--layout", required=True, choices=["de", "fr", "group", "x11"])
    ap.add_argument("--x11", action="store_true", help="run the shell as an X11 window (nested Xwayland)")
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True)
    a = ap.parse_args()
    if a.layout == "x11":
        a.x11 = True
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    nest_env.pop("DISPLAY", None)
    shell_env = dict(os.environ)
    shell_env.pop("WAYLAND_SOCKET", None)
    x_env = None
    if a.x11:
        disp = subprocess.run([os.path.join(WEB, "tools/nest.sh"), "--x11-display"], check=True,
                              capture_output=True, text=True).stdout.strip()
        if not disp:
            raise SystemExit("the nested dwl has no Xwayland (DISPLAY unknown; try web/tools/nest.sh --new)")
        shell_env.pop("WAYLAND_DISPLAY", None)
        shell_env["DISPLAY"] = disp
        x_env = dict(os.environ, DISPLAY=disp)
        x_env.pop("WAYLAND_DISPLAY", None)
        print(f"nested dwl {wl}, shell on its Xwayland {disp}")
    else:
        shell_env.pop("DISPLAY", None)
        shell_env["WAYLAND_DISPLAY"] = wl
        print(f"nested dwl {wl}, shell as a Wayland client")
    # The shell's own default layout must not be what makes it pass: a layout that is not us.
    shell_env.pop("XKB_DEFAULT_LAYOUT", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    page_url = f"http://127.0.0.1:{srv.server_address[1]}/keymap.html"

    unit = f"web-keymap-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, page_url]
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

    try:
        log.wait(r'"title":"km v=copied in the page 7 keys="', timeout=60)
        log.wait(r"page window is (\d+)x(\d+)", timeout=10)
        time.sleep(0.5)
        sizes = [re.search(r"\] window (\d+)x(\d+), page", l) for l in log.lines]
        ww, wh = [int(x) for x in [s for s in sizes if s][-1].groups()]
        shot = os.path.join(out, "start.png")
        subprocess.run(["grim", shot], env=nest_env, check=True)
        ow, oh = live.png_size(shot)
        bx, by = (ow - ww) // 2, (oh - wh) // 2

        def inject(*cmds):
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + bx},{int(mm[2]) + by}" if mm else c)
            subprocess.run([a.inject, str(ow), str(oh)] + args, env=nest_env, check=True)

        def bar_now():
            for l in reversed(log.lines):
                if "] bar: " in l:
                    return l.split("bar: ", 1)[1]
            return ""

        def to_normal():
            inject("key:esc", "wait:60", "key:esc", "wait:400")
            if not bar_now().startswith("NORMAL | "):
                raise Fail(f"Esc Esc left the bar at {bar_now()!r}")

        def into_field():
            # The textarea has autofocus: `i` from NORMAL gives the page's focused field the keys.
            to_normal()
            k = log.mark()
            inject("key:i", "wait:300")
            log.wait(BAR + r"INPUT \| ", k)
            inject("key:end", "wait:100")

        def last_value():
            for l in reversed(log.lines):
                m = re.search(r'"title":"km v=(.*?) keys=[^"]*"', l)
                if m:
                    return m[1]
            return ""

        def type_physical(codes, want):
            """Presses the physical keys `codes` in the page's field; the field must end with `want`."""
            before = last_value()
            k = log.mark()
            inject(*[f"ev:{c}" for c in codes], "wait:400")
            end = time.monotonic() + 6
            while time.monotonic() < end:
                v = last_value()
                if v.endswith(want) and v != before:
                    return v
                time.sleep(0.2)
            subprocess.run(["grim", os.path.join(out, "fail.png")], env=nest_env)
            titles = [l.split('"title":', 1)[1] for l in log.lines[k:] if '"title":"km' in l]
            raise Fail(f"physical keys {codes} typed into {before!r} gave {last_value()!r}, want ...{want!r}; titles {titles}")

        def line_opens(prefix, cmds):
            """After Esc Esc, `cmds` (a physical chord) opens the command / search line showing `prefix`."""
            to_normal()
            k = log.mark()
            inject(*cmds, "wait:300")
            log.wait(BAR + re.escape(prefix), k, timeout=6)
            inject("key:esc", "wait:300")

        def shift(code):
            return ["hold:s", f"ev:{code}", "release"]

        def current_name():
            m = [re.search(r"desktop keymap: (.*)$", l) for l in log.lines]
            m = [x for x in m if x]
            return m[-1][1] if m else None

        if a.layout in ("de", "fr", "group"):
            first = log.wait(r"desktop keymap: (.*)$", 0, timeout=10)[1]
            print(f"  the shell was given the desktop keymap: {first}")

        if a.layout == "de":
            def typing():
                into_field()
                return "physical Y,Z -> " + type_physical([Y, Z], "zy")
            step("de: the page's field types z,y for physical Y,Z", typing)
            step("de: ':' (Shift+period) opens the command line", lambda: line_opens(":", shift(DOT)))
            step("de: '/' (Shift+7) opens the search line", lambda: line_opens("/", shift(N7)))
        elif a.layout == "fr":
            def typing():
                into_field()
                return "physical A,Q -> " + type_physical([A, Q], "qa")
            step("fr: the page's field types q,a for physical A,Q", typing)
            step("fr: ':' (the period key) opens the command line", lambda: line_opens(":", [f"ev:{DOT}"]))
            step("fr: '/' (Shift+period) opens the search line", lambda: line_opens("/", shift(DOT)))
        elif a.layout == "group":
            def g0():
                into_field()
                return "group 0 (us): physical Y,Z -> " + type_physical([Y, Z], "yz")
            step("group 0 (us) types y,z", g0)

            def g1():
                k = log.mark()
                inject("group:1", "wait:400")
                log.wait(r"desktop layout group: 1", k, timeout=6)
                return "group 1 (de): physical Y,Z -> " + type_physical([Y, Z], "zy")
            step("group 1 (de) types z,y", g1)
            step("group 1 (de): ':' (Shift+period) opens the command line", lambda: line_opens(":", shift(DOT)))

            def g0_again():
                k = log.mark()
                inject("group:0", "wait:400")
                log.wait(r"desktop layout group: 0", k, timeout=6)
                into_field()
                return "back to group 0: physical Y,Z -> " + type_physical([Y, Z], "yz")
            step("back to group 0 (us) types y,z", g0_again)
        else:
            def x_set(layout):
                subprocess.run(["setxkbmap", "-display", x_env["DISPLAY"], layout], check=True)

            def x_layout(layout, y_z, a_q):
                k = log.mark()
                x_set(layout)
                log.wait(r"desktop keymap: ", k, timeout=10)
                time.sleep(0.5)
                into_field()
                v = type_physical([Y, Z], y_z)
                v2 = type_physical([A, Q], a_q)
                return f"setxkbmap {layout}: physical Y,Z,A,Q -> ...{v2[-4:]}"
            step("x11: the first keymap is the X server's", lambda: "keymap: " + str(log.wait(r"desktop keymap: (.*)$", 0, 10)[1]))
            step("x11: setxkbmap de -> Y,Z swap", lambda: x_layout("de", "zy", "aq"))
            step("x11: setxkbmap fr -> A,Q swap", lambda: x_layout("fr", "yw", "qa"))
            step("x11: setxkbmap de again: ':' (Shift+period) opens the command line",
                 lambda: (x_set("de"), time.sleep(1), line_opens(":", shift(DOT)))[-1])
            step("x11: setxkbmap us -> back to y,z,a,q", lambda: x_layout("us", "yz", "aq"))
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
