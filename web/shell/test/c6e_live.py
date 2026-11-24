#!/usr/bin/env python3
"""C6e live check: an input method types composed (preedit) text into a page field, through the
shell's text-input-v3 relay, inside the headless nested dwl (web/tools/nest.sh).

    web/shell/test/c6e_live.py --out .qwork/c6e --inject .qwork/inject/release/inject
    NEST_SCALE=2 web/shell/test/c6e_live.py --out .qwork/c6e-2 --inject .qwork/inject/release/inject

The input method: no fcitx5 or IBus with a Wayland frontend is installed and none installs without
root, so the test IME is `inject ime` (web/tools/inject/src/ime.rs), a scripted zwp_input_method_v2
client. The nest's dwl relays text-input-v3 <-> input-method-v2 (the C6E-NEST hunk of
web/tools/nest/dwl-headless.patch) and logs what it relays to its dwl.log as `nest-ime: ...`.
The chain: test IME -> dwl -> the shell window's own zwp_text_input_v3 -> the shell (ime.rs) ->
the page's zwp_text_input_v3 (Chromium, --enable-wayland-ime --wayland-text-input-version=3).

The page (pages/c6e.html) has a text field and an email field and reports their values and the
composition (compositionupdate / compositionend) in its title. Steps:
  1. a click into the field: INPUT; the IME is activated, with the field's content type, and the
     cursor rectangle (in the shell window's logical coordinates, checked against the field's box,
     the same numbers at any NEST_SCALE) reaches the desktop
  2. preedit: the page shows a composition (compositionupdate), nothing committed yet
  3. commit: the composed text replaces the preedit in the field (compositionend)
  4. surrounding text: the IME gets the field's text and cursor
  5. delete_surrounding + commit in one go (a correction)
  6. NORMAL (Esc Esc) clears a preedit and deactivates the IME; text it sends then is dropped
  7. back to INPUT (click): activated again, typing works again
  8. the email field: content type purpose email reaches the IME
Exit 0 when every step passed."""

import argparse
import functools
import os
import queue
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
BAR = r"\] bar: "
# text-input-v3 content purposes.
PURPOSE_NORMAL, PURPOSE_EMAIL = 0, 6


class TestIme:
    """`inject ime` on the nest: commands on stdin, the compositor's events on stdout."""

    def __init__(self, inject, env, path):
        self.p = subprocess.Popen([inject, "ime"], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT, text=True, bufsize=1)
        self.lines = []
        self.cv = threading.Condition()
        self.file = open(path, "w", buffering=1)
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.p.stdout:
            line = line.rstrip("\n")
            self.file.write(line + "\n")
            with self.cv:
                self.lines.append(line)
                self.cv.notify_all()

    def mark(self):
        with self.cv:
            return len(self.lines)

    def wait(self, pattern, since=0, timeout=6):
        rx = re.compile(pattern)
        end = time.monotonic() + timeout
        with self.cv:
            while True:
                for line in self.lines[since:]:
                    m = rx.search(line)
                    if m:
                        return m
                left = end - time.monotonic()
                if left <= 0:
                    raise Fail(f"the test IME never printed /{pattern}/ (got {self.lines[since:][-8:]})")
                self.cv.wait(left)

    def seen(self, pattern, since):
        with self.cv:
            return [l for l in self.lines[since:] if re.search(pattern, l)]

    def send(self, line):
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()

    def stop(self):
        if self.p.poll() is None:
            try:
                self.send("quit")
                self.p.wait(timeout=3)
            except (OSError, subprocess.TimeoutExpired):
                self.p.kill()


class DwlLog:
    """The nest's dwl.log (the relay's `nest-ime:` lines), read from an offset."""

    def __init__(self, path):
        self.path = path

    def mark(self):
        try:
            return os.path.getsize(self.path)
        except OSError:
            return 0

    def lines(self, since):
        with open(self.path, "rb") as f:
            f.seek(since)
            return [l for l in f.read().decode("utf-8", "replace").splitlines() if "nest-ime:" in l]

    def wait(self, pattern, since, timeout=6):
        rx = re.compile(pattern)
        end = time.monotonic() + timeout
        while True:
            for l in self.lines(since):
                m = rx.search(l)
                if m:
                    return m
            if time.monotonic() > end:
                raise Fail(f"dwl.log never showed /{pattern}/ (got {self.lines(since)[-6:]})")
            time.sleep(0.1)


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
    reg = os.environ.get("NEST_REG") or os.path.join(os.path.dirname(WEB), ".qwork", "nest")
    dwl = DwlLog(os.path.join(reg, "dwl.log"))
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    nest_env.pop("DISPLAY", None)
    shell_env = dict(nest_env)
    shell_env.pop("WAYLAND_SOCKET", None)
    print(f"nested dwl {wl} (scale {live.nest_scale()}), test IME: inject ime")

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    page_url = f"http://127.0.0.1:{srv.server_address[1]}/c6e.html"

    ime = TestIme(a.inject, nest_env, os.path.join(out, "ime.log"))
    unit = f"web-c6e-live-{os.getpid()}"
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

    def shot(name):
        p = os.path.join(out, name)
        subprocess.run(["grim", p], env=nest_env, check=True)
        return p

    try:
        ime.wait(r"^bound$", timeout=10)
        log.wait(r'"title":"c6e v= m= c=-"', timeout=60)
        geo = livelib.settle(log, shot)
        ow, oh, bx, by = geo.ow, geo.oh, geo.bx, geo.by

        def inject(*cmds):
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + bx},{int(mm[2]) + by}" if mm else c)
            subprocess.run([a.inject, str(ow), str(oh)] + args, env=nest_env, check=True)

        def title(v, m="", c=None, since=0, timeout=6):
            pat = r'"title":"c6e v=' + re.escape(v) + " m=" + re.escape(m) + " c=" + (re.escape(c) if c else r"[^\"]*") + '"'
            return log.wait(pat, since, timeout)

        def bar_now():
            for l in reversed(log.lines):
                if "] bar: " in l:
                    return l.split("bar: ", 1)[1]
            return ""

        def to_normal():
            inject("key:esc", "wait:60", "key:esc", "wait:400")
            if not bar_now().startswith("NORMAL | "):
                raise Fail(f"Esc Esc left the bar at {bar_now()!r}")

        def click_field(y):
            """From NORMAL, a click on the page's empty area, then on the field at height y: INPUT."""
            to_normal()
            inject("move:600,600", "wait:150", "click", "wait:300")
            k = log.mark()
            inject(f"move:200,{y}", "wait:150", "click", "wait:300")
            log.wait(r'"editable-focused","tab":\d+,"focused":true', k, timeout=4)
            log.wait(BAR + r"INPUT \| ", k)

        def ime_command(line, since_dwl):
            i = ime.mark()
            ime.send(line)
            ime.wait(r"^sent ", i)
            return dwl.wait(r"im commit", since_dwl)

        def activate():
            k, d, i = log.mark(), dwl.mark(), ime.mark()
            click_field(60)
            log.wait(r"ime: page text input enabled", k)
            log.wait(r"ime: desktop input method enabled", k)
            ime.wait(r"^activate$", i)
            ime.wait(r"^content_type \d+ %d$" % PURPOSE_NORMAL, i)
            # The test IME answers the activation (as fcitx5 does); Chromium then commits its caret.
            ime.wait(r"^answered activate", i)
            m = log.wait(r"ime: cursor rectangle \((-?\d+), (-?\d+), (\d+), (\d+)\) to the desktop", k)
            shell_rect = tuple(int(x) for x in m.groups())
            dwl.wait(r"rect=%d,%d,%d,%d" % shell_rect, d)
            x, y, w, h = shell_rect
            # #t is at 40,40 400x40 (+ borders) in the page, the page at the shell window's top left.
            if not (38 <= x <= 60 and 40 <= y and y + h <= 86 and h >= 10 and w >= 0):
                raise Fail(f"the cursor rectangle {shell_rect} is not at the field's caret (field 40,40 404x44)")
            return f"IME activated (content type normal), cursor rectangle {shell_rect} (shell-window logical px) reached the desktop"
        step("1 click into the field: IME activated, cursor rectangle", activate)

        def preedit():
            k, d = log.mark(), dwl.mark()
            ime_command("preedit 2 2 ni", d)
            title("ni", c="u:ni", since=k)
            log.wait(r'ime: preedit "ni" to the page', k)
            time.sleep(0.5)  # the frame after the title
            shot("preedit.png")
            return "the page shows the composition 'ni' (compositionupdate u:ni), preedit.png"
        step("2 preedit reaches the page", preedit)

        def commit():
            k, d = log.mark(), dwl.mark()
            ime_command("commit 你好", d)
            title("你好", c="e:你好", since=k)
            return "commit 你好 replaced the preedit (compositionend e:你好)"
        step("3 commit replaces the preedit", commit)

        def surrounding():
            # Chromium commits its new text with its next text-input commit, i.e. after the next
            # `done`: the IME asks once more (an empty commit), as a real one does on its next key.
            i, d = ime.mark(), dwl.mark()
            ime_command("nudge", d)
            ime.wait(r"^surrounding 6 6 你好$", i, timeout=4)
            return "the IME got surrounding text '你好', cursor 6 (bytes)"
        step("4 surrounding text reaches the IME", surrounding)

        def correction():
            k, d = log.mark(), dwl.mark()
            ime_command("compose 3 0 |界", d)
            title("你界", since=k)
            log.wait(r"ime: delete 3,0 to the page", k)
            return "delete_surrounding 3,0 + commit 界: '你好' -> '你界'"
        step("5 delete_surrounding + commit", correction)

        def normal():
            k, d = log.mark(), dwl.mark()
            ime_command("preedit 2 2 zz", d)
            title("你界zz", c="u:zz", since=k)
            i = ime.mark()
            k = log.mark()
            to_normal()
            ime.wait(r"^deactivate$", i)
            log.wait(r"ime: desktop input method disabled", k)
            title("你界", since=k)
            d = dwl.mark()
            ime.send("commit X")
            dwl.wait(r"im commit .*commit=\"X\".*\(no active text input\)", d)
            time.sleep(0.5)
            if any('c6e v=你界X' in l for l in log.lines[k:]):
                raise Fail("text the IME sent in NORMAL reached the page")
            return "Esc Esc: the preedit 'zz' is cleared, the IME deactivated, its 'X' dropped by the desktop"
        step("6 NORMAL clears the preedit and deactivates the IME", normal)

        def again():
            i = ime.mark()
            k = log.mark()
            click_field(60)
            ime.wait(r"^activate$", i)
            d = dwl.mark()
            ime_command("commit !", d)
            title("你界!", since=k)
            return "a click back into the field: activated again, commit '!' typed"
        step("7 back to INPUT: activated again", again)

        def email():
            i = ime.mark()
            click_field(160)
            ime.wait(r"^activate$", i)
            ime.wait(r"^content_type \d+ %d$" % PURPOSE_EMAIL, i)
            k, d = log.mark(), dwl.mark()
            ime_command("commit a@b", d)
            title("你界!", m="a@b", since=k)
            return "the email field: content type purpose email reached the IME; commit typed into it"
        step("8 content type of an email field", email)

    except Fail as e:
        results.append(("start", False, str(e)))
        print(f"  FAIL start: {e}")
    finally:
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        ime.stop()
        srv.shutdown()

    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; logs in {out}/ (web.log, ime.log)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
