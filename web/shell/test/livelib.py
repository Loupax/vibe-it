"""Shared helpers for the live checks (LIVE-GEOM): wait until dwl has tiled the shell window and the
page has taken the matching size, then return the geometry every click depends on.

The shell starts at 1280x800 and the first `page window is` line can belong to that size, so reading
the size 0.5 s after it put every click 160 px left and 103 px low (C6-LIVE). The settled test
(from c6_live, #695): the shell window is focused on the desktop, and the latest `window WxH, page
..width: PW, height: PH` fits the output and equals the latest `page window is PWxPH`.

C6b: all coordinates are logical; screenshots are logical px times the nest's scale (live.nest_scale)."""

import os
import re
import subprocess
import sys
import time
from collections import namedtuple

sys.dont_write_bytecode = True
import live  # noqa: E402

Fail = live.Fail
Geo = namedtuple("Geo", "ww wh ow oh bx by scale")

_WIN = re.compile(r"\] window (\d+)x(\d+), page .*width: (\d+), height: (\d+)")
_PAGE = re.compile(r"\] page window is (\d+)x(\d+)")


def settled_window(lines, ow, oh):
    """(ww, wh) once the latest `window WxH` fits the ow x oh output and the page window has the
    page size that goes with it, else None."""
    win = page = None
    for l in lines:
        m = _WIN.search(l)
        if m:
            win = [int(x) for x in m.groups()]
        m = _PAGE.search(l)
        if m:
            page = [int(x) for x in m.groups()]
    if win and page and win[0] <= ow and win[1] <= oh and win[2:] == page:
        return win[0], win[1]
    return None


def settle(log, shot, timeout=15, need_focus=True):
    """Wait for the first page window, the desktop focus and the settled size; return a Geo.
    `shot(name)` takes a grim screenshot and returns its path (the output size comes from it).
    Raises Fail naming the window lines seen when the size never settles."""
    log.wait(r"page window is (\d+)x(\d+)", timeout=30)
    if need_focus:
        # The first click must find the shell focused on the desktop, else the page window is not
        # activated and the click may not focus anything.
        log.wait(r"shell window focused: true", timeout=10)
    scale = live.nest_scale()
    p = shot("start.png")
    ow, oh = live.output_size(p, scale)
    end = time.monotonic() + timeout
    got = settled_window(log.lines, ow, oh)
    while not got and time.monotonic() < end:
        time.sleep(0.1)
        got = settled_window(list(log.lines), ow, oh)
    if not got:
        raise Fail(f"the shell window never settled to the {ow}x{oh} output: "
                   f"{[l for l in log.lines if '] window ' in l or 'page window is' in l]}")
    time.sleep(0.2)  # the settled size must hold
    again = settled_window(list(log.lines), ow, oh)
    if again != got:
        raise Fail(f"the shell window size moved after settling: {got} -> {again}")
    ww, wh = got
    return Geo(ww, wh, ow, oh, (ow - ww) // 2, (oh - wh) // 2, scale)

