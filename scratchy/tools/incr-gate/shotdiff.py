#!/usr/bin/env python3
"""QINC1k: compare the flag-off and flag-on screenshots of a gate series.

Usage: python3 -I tools/incr-gate/shotdiff.py OUT_DIR SERIES [SHOT]
  SERIES  the tag prefix (e.g. `perf`); SHOT `pre` (default: before any input) or `end`.
For each page and round, diffs <SERIES>-<page>-off-<r>.<SHOT>.png with <SERIES>-<page>-incr-<r>.<SHOT>.png
and, as the noise floor, off round r with off round r+1. Prints the share of differing pixels and the
bounding box of the difference; writes <a>--<b>.diff.png (differences in red) next to them.
Live pages differ between any two loads (ads, thumbnails, timestamps): an off/incr difference matters
only where off/off is clean.
"""
import os
import sys

from PIL import Image, ImageChops


def diff(a, b, outp):
    try:
        ia = Image.open(a).convert('RGB')
        ib = Image.open(b).convert('RGB')
    except OSError:
        return None
    if ia.size != ib.size:
        return ('size', ia.size, ib.size)
    d = ImageChops.difference(ia, ib).convert('L').point(lambda v: 255 if v > 24 else 0)
    box = d.getbbox()
    n = sum(1 for v in d.getdata() if v)
    if box:
        red = Image.new('RGB', ia.size, (255, 0, 0))
        Image.composite(red, ia.point(lambda v: v // 3 + 170), d).save(outp)
    return (n / (ia.size[0] * ia.size[1]), box)


def main():
    out, series = sys.argv[1], sys.argv[2]
    shot = sys.argv[3] if len(sys.argv) > 3 else 'pre'
    tags = sorted({f[:-len(f'.{shot}.png')] for f in os.listdir(out) if f.startswith(series + '-') and f.endswith(f'.{shot}.png')})
    pages = sorted({t.split('-')[1] for t in tags})
    for p in pages:
        rounds = sorted({int(t.rsplit('-', 1)[1]) for t in tags if t.split('-')[1] == p})
        for r in rounds:
            off = os.path.join(out, f'{series}-{p}-off-{r}.{shot}.png')
            on = os.path.join(out, f'{series}-{p}-incr-{r}.{shot}.png')
            off2 = os.path.join(out, f'{series}-{p}-off-{r + 1}.{shot}.png')
            a = diff(off, on, os.path.join(out, f'{series}-{p}-{r}.{shot}.off--incr.diff.png'))
            b = diff(off, off2, os.path.join(out, f'{series}-{p}-{r}.{shot}.off--off{r + 1}.diff.png')) if os.path.exists(off2) else None
            fmt = lambda x: '-' if x is None else (f'{x[0] * 100:.2f}% bbox {x[1]}' if x[0] != 'size' else f'size {x[1]} vs {x[2]}')
            print(f'{p} r{r} {shot}: off vs incr {fmt(a)}; off vs off r{r + 1} {fmt(b)}')


if __name__ == '__main__':
    main()
