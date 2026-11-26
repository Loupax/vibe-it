#!/usr/bin/env python3
"""C6 live check: the clipboard bridge between the page and the desktop, inside the headless
nested dwl (web/tools/nest.sh), never on the user's display or clipboard.

    web/shell/test/c6_live.py --out .qwork/c6 --inject .qwork/inject/release/inject [--x11]

"The desktop" is the nested dwl: its clipboard is set and read with wl-copy / wl-paste on the
nested WAYLAND_DISPLAY (data-control, so they take no focus), or with xsel on the nested
Xwayland with --x11 (the shell is then an X11 window). The page (pages/c6.html) has a
textarea and reports its value in its title. What happened is read from the shell's log (the
status bar as `bar: ...`, the bridge as `clipboard: ...`, every host event). Each step names
the KEYS.md rows it shows. Exit 0 when every step passed. Needs what c3_live.py needs.

C6-LIVE: the steps that type into the textarea first make sure the page really focused it (the
host's `editable-focused true` after the click). They used to press `i` when the bar was not
INPUT after the click, which put the shell in INPUT with nothing focused, so a click that missed
the field showed up as four unrelated failures (a whole-page Ctrl-A copy, nothing pasted, nothing
typed). Middle-click paste into the page is not supported (web/DECISIONS.md 2026-11-23), so the
primary step checks only that a selection in the page becomes the desktop's primary.

C-CLIPTYPES (Wayland only; --x11 runs the text steps): every type is carried. The page also has a
`copy` handler (text/plain and text/html) and a `paste` handler that reports a paste without text
(an image) in its title as `c6 paste types=... file=<type>:<size> v=<value>`. The extra steps: an
image-only desktop clipboard (`wl-copy -t image/png`) pastes the image into the page and no old
text (page or `:` line); a copy in the page lists text/html and the marker type in `wl-paste -l`;
yy then a paste in the page; the page's primary selection with its types. Run it with a private
D-Bus (`dbus-run-session --config-file=web/tools/notify-probe/private-bus.conf -- ...`)."""

import argparse
import functools
import os
import re
import shutil
import subprocess
import sys
import threading
import struct
import time
import zlib

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


class DesktopClipboard:
    """The nested desktop's clipboard, as the test controls it."""

    def __init__(self, x11, nest_env, x_env):
        self.x11, self.nest_env, self.x_env = x11, nest_env, x_env
        self.procs = []

    def set(self, text, primary=False):
        if text == "":
            if self.x11:
                subprocess.run(["xsel", "-p" if primary else "-b", "-c"], env=self.x_env, check=True)
            else:
                subprocess.run(["wl-copy", "--clear"] + (["-p"] if primary else []), env=self.nest_env, check=True)
            time.sleep(0.4)
            return
        if self.x11:
            p = subprocess.Popen(["xsel", "-p" if primary else "-b", "-i", "-n"], stdin=subprocess.PIPE, env=self.x_env)
            p.stdin.write(text.encode())
            p.stdin.close()
        else:
            p = subprocess.Popen(["wl-copy", "--foreground"] + (["-p"] if primary else []) + ["--", text],
                                 env=self.nest_env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.procs.append(p)
        time.sleep(0.4)

    def set_file(self, path, mime):
        """C-CLIPTYPES: the desktop's clipboard holds only `path`'s bytes as `mime` (Wayland)."""
        with open(path, "rb") as f:
            p = subprocess.Popen(["wl-copy", "--foreground", "-t", mime], stdin=f, env=self.nest_env,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.procs.append(p)
        time.sleep(0.4)

    def types(self, primary=False):
        r = subprocess.run(["wl-paste", "-l"] + (["-p"] if primary else []), env=self.nest_env,
                           capture_output=True, text=True, timeout=5)
        return r.stdout.split()

    def get_type(self, mime):
        r = subprocess.run(["wl-paste", "-n", "-t", mime], env=self.nest_env, capture_output=True, text=True, timeout=5)
        return r.stdout

    def get(self, primary=False):
        if self.x11:
            cmd = ["xsel", "-p" if primary else "-b", "-o"]
            env = self.x_env
        else:
            cmd = ["wl-paste", "-n"] + (["-p"] if primary else [])
            env = self.nest_env
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=5)
        return r.stdout

    def wait_for(self, want, primary=False, timeout=5):
        end = time.monotonic() + timeout
        got = None
        while time.monotonic() < end:
            got = self.get(primary)
            if got == want:
                return got
            time.sleep(0.2)
        raise Fail(f"the desktop's {'primary' if primary else 'clipboard'} holds {got!r}, want {want!r}")

    def stop(self):
        for p in self.procs:
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    p.kill()


def tiny_png(w=4, h=3):
    """C-CLIPTYPES: a small RGB PNG (stdlib only), what `grim - | wl-copy` puts on the clipboard."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\x00" + b"".join(bytes((x * 60, y * 80, 200)) for x in range(w)) for y in range(h))
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--x11", action="store_true", help="run the shell as an X11 window (nested Xwayland)")
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
        print(f"nested dwl {wl}, shell on its Xwayland {disp}, clipboard through xsel")
    else:
        shell_env.pop("DISPLAY", None)
        shell_env["WAYLAND_DISPLAY"] = wl
        print(f"nested dwl {wl}, shell as a Wayland client, clipboard through wl-copy/wl-paste")
    desk = DesktopClipboard(a.x11, nest_env, x_env)
    # The bridge's log lines: C-CLIPTYPES carries every type on Wayland, X11 stays text only.
    if a.x11:
        page_to_desktop_re = r"clipboard: page {} \(\d+ bytes\) to the desktop"
        desktop_to_page_re = r"clipboard: desktop {} \(\d+ bytes\) offered to the page"
    else:
        page_to_desktop_re = r"clipboard: page {} offered to the desktop \(\d+ types"
        desktop_to_page_re = r"clipboard: desktop {} offered to the page \(\d+ types"
    PAGE_TO_DESKTOP, DESKTOP_TO_PAGE = page_to_desktop_re, desktop_to_page_re  # noqa: N806

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    page_url = f"{base}/c6.html"

    unit = f"web-c6-live-{os.getpid()}"
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
        log.wait(r'"title":"c6 v=copied in the page 7"', timeout=60)
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

        def title(value, since, timeout=6):
            return log.wait(r'"title":"c6 v=' + re.escape(value) + '"', since, timeout)

        def chord(mods, key):
            return [f"hold:{mods}", f"key:{key}", "release", "wait:300"]

        def bar_now():
            """The last status bar state logged (the shell logs only changes)."""
            for l in reversed(log.lines):
                if "] bar: " in l:
                    return l.split("bar: ", 1)[1]
            return ""

        def to_normal():
            inject("key:esc", "wait:60", "key:esc", "wait:400")
            if not bar_now().startswith("NORMAL | "):
                raise Fail(f"Esc Esc left the bar at {bar_now()!r}")

        def into_field():
            # From NORMAL, a click on the page's empty area (blurs the textarea if it kept the
            # focus), then a click on the textarea: the page focuses it (the host reports
            # `editable-focused true`) and the shell switches to INPUT (T29). No `i` fallback:
            # INPUT with nothing focused would send Ctrl-A to the whole page.
            to_normal()
            inject("move:600,500", "wait:150", "click", "wait:300")
            k = log.mark()
            inject("move:200,80", "wait:150", "click", "wait:300")
            try:
                log.wait(r'"editable-focused","tab":\d+,"focused":true', k, timeout=3)
            except Fail:
                shot("into-field-miss.png")
                raise Fail(f"a click on the textarea (window {ww}x{wh} at {bx},{by} on a {ow}x{oh} output) "
                           "did not focus it: no editable-focused (into-field-miss.png)")
            bar(r"INPUT \| ", k)

        def page_to_desktop():
            desk.set("something else")
            into_field()
            k = log.mark()
            inject(*chord("c", "a"), *chord("c", "c"))
            log.wait(PAGE_TO_DESKTOP.format("clipboard"), k)
            desk.wait_for("copied in the page 7")
            return "Ctrl-A Ctrl-C in the page's textarea: the desktop's clipboard (wl-paste/xsel) holds the text"
        step("page -> desktop: a copy in the page reaches the desktop clipboard", page_to_desktop)

        def desktop_to_line():
            got = []
            for i, (mods, key) in enumerate([("c", "v"), ("cs", "v"), ("s", "ins")]):
                text = f"desk{i}.example/x"
                desk.set(text + "\n")
                to_normal()
                k = log.mark()
                inject(*keys_for(":go "), "wait:100", *chord(mods, key))
                bar(re.escape(":go " + text) + r"\|", k)
                got.append(f"{'+'.join({'c': 'Ctrl', 's': 'Shift'}[m] for m in mods)}-{key}")
                if i == 0:
                    shot("pasted-into-line.png")
                inject("key:esc", "wait:300")
            return f"{', '.join(got)} in the `:` line inserted the desktop's text, newline dropped (T26, Paste)"
        step("desktop -> `:` line: Ctrl-v, Ctrl-Shift-v, Shift-Insert (T26)", desktop_to_line)

        def desktop_to_search():
            desk.set("needle")
            to_normal()
            k = log.mark()
            inject("key:slash", "wait:200", *chord("c", "v"))
            bar(r"/needle\|", k)
            inject("key:esc", "wait:300")
            return "Ctrl-v in the `/` line inserted the desktop's text (T22)"
        step("desktop -> `/` line (T22)", desktop_to_search)

        def desktop_to_page():
            # Offered to the page when the desktop's selection changes (C-CLIPTYPES, Wayland) or on
            # the paste key (X11).
            k0 = log.mark()
            desk.set("pasted from the desktop 9")
            into_field()
            k = log.mark()
            inject(*chord("c", "a"), *chord("c", "v"))
            title("pasted from the desktop 9", k)
            log.wait(DESKTOP_TO_PAGE.format("clipboard"), k0)
            shot("pasted-into-page.png")
            return "Ctrl-A Ctrl-V in the textarea: the page's field holds the desktop's text"
        step("desktop -> page: Ctrl-v in INPUT pastes the desktop clipboard", desktop_to_page)

        def page_to_line():
            # The vice versa through the `:` line: what the page copied, pasted in the shell.
            into_field()
            inject(*chord("c", "a"), *keys_for("page word"), "wait:300")
            inject(*chord("c", "a"), *chord("c", "c"))
            desk.wait_for("page word")
            to_normal()
            k = log.mark()
            inject(*keys_for(":"), "wait:200", *chord("c", "v"))
            bar(re.escape(":page word") + r"\|", k)
            inject("key:esc", "wait:300")
            # C-CLIPTYPES 5: on Wayland the desktop's clipboard is the page's own source, read from
            # memory: no wait for the desktop (it would ask the shell's main thread, the one waiting).
            stuck = [line for line in log.lines[k:] if "did not answer" in line]
            if stuck:
                raise Fail(f"the `:` line paste waited for the desktop: {stuck[0]}")
            return "typed 'page word' in the page, copied it there, Ctrl-v in the `:` line shows it (no 'did not answer')"
        step("page -> `:` line", page_to_line)

        def yy():
            desk.set("old")
            to_normal()
            k = log.mark()
            inject("key:y", "wait:100", "key:y", "wait:400")
            bar(r"NORMAL \| Copied to clipboard", k)
            desk.wait_for(page_url)
            return f"yy: 'Copied to clipboard', the desktop holds {page_url}"
        step("T54 T55: yy copies the URL", yy)

        def let_plus():
            desk.set("old")
            k = log.mark()
            inject(*keys_for(":let @+ = @%"), "key:ret", "wait:400")
            bar(r"NORMAL \| Copied to clipboard", k)
            desk.wait_for(page_url)
            desk.set("")
            k = log.mark()
            inject(*keys_for(":let @+ = @+"), "key:ret", "wait:400")
            bar(r"NORMAL \| web: the clipboard is empty", k)
            desk.set("kept")
            k = log.mark()
            inject(*keys_for(":let @+ = @+"), "key:ret", "wait:400")
            bar(r"NORMAL \| Copied to clipboard", k)
            desk.wait_for("kept")
            return ":let @+ = @% copies the URL; :let @+ = @+ reads @+ ('the clipboard is empty' when it is)"
        step("Registers: :let @+ = @%, @+ readable", let_plus)

        def primary():
            # A selection in the page reaches the desktop's primary selection. Middle-click
            # paste into the page is not supported (DECISIONS.md 2026-11-23), so not checked.
            desk.set("old primary", primary=True)
            into_field()
            k = log.mark()
            inject(*chord("c", "a"), *keys_for("prim one"), "wait:200", *chord("c", "a"), "wait:400")
            log.wait(PAGE_TO_DESKTOP.format("primary selection"), k, timeout=6)
            desk.wait_for("prim one", primary=True)
            return "select-all in the page set the desktop's primary selection (wl-paste -p / xsel -p)"
        step("primary selection: page -> desktop", primary)

        # C-CLIPTYPES: every type on Wayland (X11 stays text only: these are skipped there).
        if not a.x11:
            png = os.path.join(out, "tiny.png")
            with open(png, "wb") as f:
                f.write(tiny_png())
            png_size = os.path.getsize(png)

            def image_to_page():
                # 1: an image-only desktop clipboard (`grim - | wl-copy`) pastes the image in the page.
                # Step 7: the offer carried to the page is not carried back to the desktop.
                k0 = log.mark()
                desk.set_file(png, "image/png")
                log.wait(DESKTOP_TO_PAGE.format("clipboard") + r": image/png\)", k0)
                into_field()
                k = log.mark()
                inject(*chord("c", "v"))
                log.wait(r"clipboard: the page reads the clipboard as image/png \(from the desktop\)", k)
                m = log.wait(r'"title":"c6 paste types=([^ "]*) file=image/png:(\d+)', k)
                if int(m[2]) != png_size:
                    raise Fail(f"the page got an image/png of {m[2]} bytes, the desktop's is {png_size}")
                bounced = [line for line in log.lines[k0:] if "page clipboard offered to the desktop" in line]
                if bounced:
                    raise Fail(f"the desktop's selection went back to the desktop: {bounced[0]}")
                return f"Ctrl-v in the field: paste event types={m[1]}, file image/png of {png_size} bytes; not carried back"
            step("C-CLIPTYPES desktop image -> page", image_to_page)

            def image_no_old_text():
                # 2: an image-only clipboard pastes no old text, in the page or in the `:` line.
                desk.set("old text 5")
                into_field()
                inject(*chord("c", "a"), *keys_for("field"), "wait:300")
                desk.set_file(png, "image/png")
                k = log.mark()
                inject(*chord("c", "v"))
                m = log.wait(r'"title":"c6 paste types=[^ "]* file=image/png:\d+ v=([^"]*)"', k)
                if m[1] != "field":
                    raise Fail(f"after the image paste the field holds {m[1]!r}, want 'field'")
                time.sleep(0.5)
                if any("old text 5" in line for line in log.lines[k:]):
                    raise Fail("the page pasted the old text")
                to_normal()
                k = log.mark()
                inject(*keys_for(":"), "wait:200", *chord("c", "v"), "wait:200", *keys_for("z"))
                bar(re.escape(":z") + r"\|", k)
                inject("key:esc", "wait:300")
                return "after an image wl-copy: the field still holds 'field', Ctrl-v in the `:` line inserts nothing"
            step("C-CLIPTYPES image-only clipboard: no old text", image_no_old_text)

            def rich_copy():
                # 3: the page's copy reaches the desktop with all its types (text/plain, text/html).
                desk.set("something else")
                into_field()
                inject(*chord("c", "a"), *keys_for("rich one"), "wait:300")
                k = log.mark()
                inject(*chord("c", "a"), *chord("c", "c"))
                log.wait(PAGE_TO_DESKTOP.format("clipboard"), k)
                desk.wait_for("rich one")
                types = desk.types()
                for want in ("text/html", "application/x-web-shell-own"):
                    if want not in types:
                        raise Fail(f"wl-paste -l lists {types}, no {want}")
                if not any(t.startswith("text/plain") for t in types):
                    raise Fail(f"wl-paste -l lists {types}, no text/plain")
                html = desk.get_type("text/html")
                if "<b>rich one</b>" not in html:
                    raise Fail(f"wl-paste -t text/html gave {html!r}")
                log.wait(r"clipboard: the desktop reads the clipboard as text/html \(from the page\)", k)
                return f"wl-paste -l: {', '.join(types)}; -t text/html: {html.strip()[:70]!r}"
            step("C-CLIPTYPES page rich copy -> desktop", rich_copy)

            def yy_to_page():
                # 4: yy (smithay-clipboard's own source on the desktop) reaches the page.
                desk.set("old")
                to_normal()
                inject("key:y", "wait:100", "key:y", "wait:400")
                desk.wait_for(page_url)
                into_field()
                k = log.mark()
                inject(*chord("c", "a"), *chord("c", "v"))
                title(page_url, k)
                log.wait(r"clipboard: the page reads the clipboard as \S+ \(from the desktop\)", k)
                return f"yy, then Ctrl-A Ctrl-v in the field: it holds {page_url}"
            step("C-CLIPTYPES yy -> paste in the page", yy_to_page)

            def primary_types():
                # 6: the page's primary selection is the shell's source on the desktop (with the marker).
                desk.set("old primary 2", primary=True)
                into_field()
                k = log.mark()
                inject(*chord("c", "a"), *keys_for("prim two"), "wait:200", *chord("c", "a"), "wait:400")
                log.wait(PAGE_TO_DESKTOP.format("primary selection"), k)
                desk.wait_for("prim two", primary=True)
                types = desk.types(primary=True)
                if "application/x-web-shell-own" not in types:
                    raise Fail(f"wl-paste -l -p lists {types}, no marker")
                return f"wl-paste -p holds 'prim two'; wl-paste -l -p: {', '.join(types)}"
            step("C-CLIPTYPES primary: page -> desktop with its types", primary_types)

    except Fail as e:
        results.append(("start", False, str(e)))
        print(f"  FAIL start: {e}")
    finally:
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        desk.stop()
        srv.shutdown()

    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; log in {out}/web.log")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
