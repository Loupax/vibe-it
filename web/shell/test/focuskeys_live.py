#!/usr/bin/env python3
"""C-FOCUSKEYS + C3-SHIFTSYNC live check, inside the headless nested dwl (web/tools/nest.sh), never
on the user's display.

    web/shell/test/focuskeys_live.py --out .qwork/focuskeys --inject .qwork/inject-target/release/inject \
        --host-bin ~/.local/lib/web/current/web-host-cef

The page (pages/focuskeys.html) logs every keydown/keyup it gets with the modifier flags, and its
scrollY, in its title. Focus is moved away from the shell window with a second client: zenity (a GTK
window in the nested dwl; dwl gives a new window the keyboard, and gives it back when it closes).
  1. Alt held (a bare key event + modifiers) in INPUT, then focus leaves: the page must see keyup Alt
     (the shell releases what the page has pressed), and after the focus returns the late physical
     release must not reach the page again, and a typed `a` carries no Alt flag.
  2. Shift set by wl_keyboard.modifiers alone (inject mods:1, no key event): `;` opens the command
     line (`:`), `G` goes to the end, `gg` back.
  3. Shift held across a focus change (down before, still down after): the shifted `;` still opens
     the command line.
Exit 0 when every step passed."""

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
SHIFT, ALT, A, G, SEMI = 42, 56, 30, 34, 39


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
    nest_env.pop("DISPLAY", None)
    shell_env = dict(nest_env)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("XKB_DEFAULT_LAYOUT", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    page_url = f"http://127.0.0.1:{srv.server_address[1]}/focuskeys.html"

    unit = f"web-focuskeys-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, page_url]
    proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(out, "web.log"))
    results = []
    children = []  # processes of ours, killed by PID at the end

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
        log.wait(r'"title":"fk y=0 ev="', timeout=60)
        log.wait(r"page window is (\d+)x(\d+)", timeout=10)
        log.wait(r"shell window focused: true", timeout=10)
        time.sleep(0.5)
        shot = os.path.join(out, "start.png")
        subprocess.run(["grim", shot], env=nest_env, check=True)
        ow, oh = live.png_size(shot)

        def inject(*cmds):
            subprocess.run([a.inject, str(ow), str(oh)] + list(cmds), env=nest_env, check=True)

        def inject_bg(*cmds):
            p = subprocess.Popen([a.inject, str(ow), str(oh)] + list(cmds), env=nest_env)
            children.append(p)
            return p

        def bar_now():
            for l in reversed(log.lines):
                if "] bar: " in l:
                    return l.split("bar: ", 1)[1]
            return ""

        def title():
            for l in reversed(log.lines):
                m = re.search(r'"title":"fk y=(\d+) ev=([^"]*)"', l)
                if m:
                    return int(m[1]), [e for e in m[2].split(",") if e]
            return 0, []

        def wait_title(pred, what, timeout=8):
            end = time.monotonic() + timeout
            while time.monotonic() < end:
                if pred(*title()):
                    return
                time.sleep(0.15)
            raise Fail(f"{what}: the page's title is {title()}")

        def to_normal():
            inject("key:esc", "wait:60", "key:esc", "wait:400")
            if not bar_now().startswith("NORMAL | "):
                raise Fail(f"Esc Esc left the bar at {bar_now()!r}")

        def to_input():
            to_normal()
            k = log.mark()
            inject("key:i", "wait:300")
            log.wait(BAR + r"INPUT \| ", k)

        def focus_away(seconds=3):
            """A second client takes the keyboard from the shell window and gives it back
            (zenity closes itself after `seconds`). Returns the log mark from before."""
            k = log.mark()
            z = subprocess.Popen(["zenity", "--info", "--text=focus away", f"--timeout={seconds}"], env=dict(nest_env, GDK_BACKEND="wayland"),
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            children.append(z)
            log.wait(r"shell window focused: false", k, timeout=20)
            return k, z

        def focus_back(k, z):
            log.wait(r"shell window focused: true", k, timeout=20)
            try:
                z.wait(timeout=10)
            except subprocess.TimeoutExpired:
                z.terminate()

        def alt_held():
            to_input()
            wait_title(lambda y, ev: True, "start")
            before = len(title()[1])
            # Alt down (key event + modifiers), held for 9 s; the shell loses the keyboard meanwhile.
            p = inject_bg(f"kdown:{ALT}", "mods:8", "wait:9000", "mods:0", f"kup:{ALT}", "wait:200")
            wait_title(lambda y, ev: "d:Alt/A" in ev, "the page saw Alt go down")
            k, z = focus_away()
            # The shell released Alt in the page when the keyboard left it.
            log.wait(r"focus lost: released \d+ key\(s\) in the page", k, timeout=5)
            wait_title(lambda y, ev: "u:Alt/A" in ev or "u:Alt/" in ev, "the page saw Alt go up at the focus loss")
            focus_back(k, z)
            p.wait(timeout=20)
            time.sleep(0.5)
            ups = [e for e in title()[1] if e.startswith("u:Alt")]
            if len(ups) != 1:
                raise Fail(f"the page saw {len(ups)} Alt keyups, want exactly 1 (no keyup without its keydown): {title()[1]}")
            inject(f"ev:{A}", "wait:300")
            wait_title(lambda y, ev: any(e.startswith("d:a/") for e in ev[-3:]), "an `a` after the refocus")
            last = [e for e in title()[1] if e.startswith("d:a")][-1]
            if last != "d:a/":
                raise Fail(f"the `a` after the refocus carries modifiers: {last} (stuck Alt) {title()[1]}")
            return f"events {title()[1]}"

        def shift_alone_colon():
            to_normal()
            k = log.mark()
            inject("mods:1", "wait:250", f"kdown:{SEMI}", f"kup:{SEMI}", "wait:150", "mods:0", "wait:300")
            log.wait(BAR + r":\|", k, timeout=5)
            inject("key:esc", "wait:300")
            return "Shift by modifiers alone + `;` -> the command line"

        def shift_alone_G():
            to_normal()
            inject("mods:1", "wait:250", f"kdown:{G}", f"kup:{G}", "wait:150", "mods:0", "wait:300")
            wait_title(lambda y, ev: y > 3000, "G (Shift+g by modifiers alone) did not scroll to the end", timeout=6)
            y = title()[0]
            inject(f"ev:{G}", "wait:100", f"ev:{G}", "wait:600")
            wait_title(lambda y, ev: y < 50, "gg back to the top", timeout=6)
            return f"G -> y={y}, gg -> 0"

        def shift_across_focus():
            to_normal()
            k0 = log.mark()
            # Shift down (key + modifiers); the keyboard leaves and returns while it is held; then `;`.
            p = inject_bg(f"kdown:{SHIFT}", "mods:1", "wait:9000", f"kdown:{SEMI}", f"kup:{SEMI}", "wait:150", "mods:0", f"kup:{SHIFT}", "wait:200")
            time.sleep(0.5)
            k, z = focus_away(3)
            focus_back(k, z)
            # Shift is still down at the desktop; `;` comes at the end of the 9 s.
            k2 = log.mark()
            log.wait(BAR + r":\|", k2, timeout=12)
            p.wait(timeout=20)
            inject("key:esc", "wait:300")
            return "Shift held across a focus change -> `;` opens the command line"

        step("Alt held at focus loss: released in the page, nothing stuck after the refocus", alt_held)
        step("Shift set by wl_keyboard.modifiers alone: `;` opens the command line", shift_alone_colon)
        step("Shift set by wl_keyboard.modifiers alone: G goes to the end, gg back", shift_alone_G)
        step("Shift held across a focus change still makes `;` a `:`", shift_across_focus)
    except Fail as e:
        results.append(("start", False, str(e)))
        print(f"  FAIL start: {e}")
    finally:
        for c in children:
            if c.poll() is None:
                c.terminate()
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
