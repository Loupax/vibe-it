#!/usr/bin/env python3
"""C6c bench: frame pacing in the headless nested dwl (web/tools/nest.sh), never on the user's display.

    web/shell/test/c6c_bench.py --out .qwork/c6c [--web PATH] [--modes idle,raf,css,video]

For each mode (test/pages/c6c.html: idle, raf = requestAnimationFrame only, css = a CSS
animation, video = a generated 30 fps WebM, scroll = a long page scrolled 40 px every frame
(C6d)) it starts the shell on the nest, lets the page measure rAF (and video frame) intervals
for 10 s, and samples the shell's CPU time (utime+stime of the shell process) over the same
window, and (C6d) the engine's: its GPU process, its renderers and all of it (every
descendant of the shell). Prints one line per mode. --wayland-debug (a check, not a
measurement: the log costs CPU) runs the shell with WAYLAND_DEBUG=server and adds which
buffers the page made (wl_shm pools or zwp_linux_dmabuf buffers). Run it under
`flock /tmp/scratchy-perf.lock` (the harness does not take the lock itself) and note `uptime`.
Needs ffmpeg for the video mode."""

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

WEB = live.WEB


def cpu_s(pid):
    with open(f"/proc/{pid}/stat") as f:
        parts = f.read().rsplit(")", 1)[1].split()
    return (int(parts[11]) + int(parts[12])) / os.sysconf("SC_CLK_TCK")


def descendants(pid):
    """{pid: kind} of every descendant of `pid` (kind: CEF's --type=..., else "host")."""
    kids = {}
    for d in os.listdir("/proc"):
        if not d.isdigit():
            continue
        try:
            with open(f"/proc/{d}/stat") as f:
                ppid = int(f.read().rsplit(")", 1)[1].split()[1])
            kids.setdefault(ppid, []).append(int(d))
        except (OSError, ValueError, IndexError):
            pass
    out, todo = {}, list(kids.get(pid, []))
    while todo:
        p = todo.pop()
        try:
            with open(f"/proc/{p}/cmdline", "rb") as f:
                # Chromium's helpers rewrite their title: one string with spaces, not argv.
                argv = f.read().replace(b"\0", b" ").split()
        except OSError:
            continue
        kind = "host"
        for arg in argv:
            if arg.startswith(b"--type="):
                kind = arg[7:].decode()
        # Children forked from a zygote keep its command line: their main thread's name says
        # what they are: the GPU process has a GpuWatchdog (and VizCompositorTh) thread, a
        # renderer a Compositor thread.
        try:
            names = set()
            for task in os.listdir(f"/proc/{p}/task"):
                with open(f"/proc/{p}/task/{task}/comm") as f:
                    names.add(f.read().strip())
            if "GpuWatchdog" in names or "VizCompositorTh" in names:
                kind = "gpu-process"
            elif "Compositor" in names:
                kind = "renderer"
        except OSError:
            pass
        out[p] = kind
        todo.extend(kids.get(p, []))
    return out


def engine_cpu(pid):
    """CPU seconds of the shell's descendants: the GPU process, the renderers, all of them."""
    by = {"gpu-process": 0.0, "renderer": 0.0, "all": 0.0}
    for p, kind in descendants(pid).items():
        try:
            c = cpu_s(p)
        except (OSError, ValueError, IndexError):
            continue
        by["all"] += c
        if kind in by:
            by[kind] += c
    return by


def buffer_kinds(path):
    """The page's buffers, from the shell's WAYLAND_DEBUG=server log (requests it received)."""
    shm = dmabuf = 0
    try:
        with open(path, errors="replace") as f:
            for line in f:
                # wayland-backend's format: "[time][rs] <- wl_shm@8.create_pool, (3, 52, 2304)".
                if re.search(r"<- wl_shm@\d+\.create_pool,", line):
                    shm += 1
                elif re.search(r"<- zwp_linux_buffer_params_v1@\d+\.create(_immed)?,", line):
                    dmabuf += 1
    except OSError:
        pass
    return f" | buffers: {shm} wl_shm pools, {dmabuf} dmabufs"


def run_mode(a, mode, base, wl, out, x11=None):
    state = os.path.join(out, "state-" + mode)
    shutil.rmtree(state, ignore_errors=True)
    env = dict(os.environ, WAYLAND_DISPLAY=wl)
    env.pop("WAYLAND_SOCKET", None)
    env.pop("DISPLAY", None)
    if a.wayland_debug:
        env["WAYLAND_DEBUG"] = "server"
    if x11:
        env.pop("WAYLAND_DISPLAY")
        env["DISPLAY"] = x11
    unit = f"web-c6c-bench-{os.getpid()}-{mode}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/c6c.html#{mode}"]
    proc = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(out, f"web-{mode}.log"))
    try:
        log.wait(r'"title":"c6c ' + mode + r' running', timeout=60)
        c0, e0, t0 = cpu_s(proc.pid), engine_cpu(proc.pid), time.monotonic()
        m = log.wait(r'"title":"c6c ' + mode + r' done([^"]*)"', timeout=30)
        c1, e1, t1 = cpu_s(proc.pid), engine_cpu(proc.pid), time.monotonic()

        def pct(d):
            return 100 * d / (t1 - t0)
        eng = " ".join(f"{k} {pct(e1[k] - e0[k]):5.1f}%" for k in ("gpu-process", "renderer", "all"))
        line = f"{mode:6s} shell CPU {pct(c1 - c0):5.1f}% | engine {eng} | over {t1 - t0:.1f}s |{m[1]}"
        with open(os.path.join(out, f"procs-{mode}.txt"), "w") as f:
            for p, kind in sorted(descendants(proc.pid).items()):
                try:
                    with open(f"/proc/{p}/cmdline", "rb") as c:
                        f.write(f"{p} {kind} {c.read().replace(b'\0', b' ').decode(errors='replace')}\n")
                except OSError:
                    pass
        if a.wayland_debug:
            log.file.flush()
            line += buffer_kinds(os.path.join(out, f"web-{mode}.log"))
        return line
    finally:
        subprocess.run(["systemctl", "--user", "stop", unit + ".scope"], check=False, capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(live.SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--modes", default="idle,raf,css,video,scroll")
    ap.add_argument("--wayland", help="C6d: run on this Wayland display instead of the nest.sh dwl (e.g. a GPU "
                    "nest from web/tools/gpu-nest.sh)")
    ap.add_argument("--wayland-debug", action="store_true",
                    help="C6d: WAYLAND_DEBUG=server in the shell; the line says wl_shm or dmabuf (not for measuring)")
    ap.add_argument("--x11", action="store_true", help="run the shell as an X11 window (the nest's Xwayland): no desktop frame callbacks")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    pages = os.path.join(out, "pages")
    os.makedirs(pages, exist_ok=True)
    shutil.copy(os.path.join(HERE, "pages", "c6c.html"), pages)
    webm = os.path.join(pages, "c6c.webm")
    if not os.path.exists(webm):
        subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30", "-t", "4",
                        "-c:v", "libvpx", "-b:v", "500k", "-pix_fmt", "yuv420p", webm], check=True)
    wl = a.wayland or subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True,
                                     text=True).stdout.strip()
    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=pages, log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    x11 = None
    if a.x11:
        # Xwayland starts lazily on the first X11 client, so its DISPLAY is known from the start.
        x11 = subprocess.run([os.path.join(WEB, "tools/nest.sh"), "--x11-display"], check=True, capture_output=True,
                             text=True).stdout.strip()
    for mode in a.modes.split(","):
        print(run_mode(a, mode, base, wl, out, x11), flush=True)


if __name__ == "__main__":
    main()
