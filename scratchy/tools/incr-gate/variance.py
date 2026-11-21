#!/usr/bin/env python3
"""QREC3: per page and variant (live / rep) the min / median / max over the rounds of each summarize.py column.

Usage: python3 -I tools/incr-gate/variance.py OUT_DIR TAG_PREFIX
Tags: <prefix><page>-<variant>-<round> (gate.sh replay / replay-headless). Prints one line per page and
variant: per column `min / median / max (spread %)` where spread = (max - min) / median.
"""
import os
import re
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import summarize  # noqa: E402

WIN = ['builds n', 'builds ms', 'wrk ms', 'forced n', 'forced ms', 'p95', 'hov p95', 'gPS n', 'gPS med', 'worst ms', 'vid', 'rss MB']
HL = ['forced n', 'forced ms', 'max ms', 'incr_shadow ms', 'worst cb ms', 'rss MB']


def nums(cell):
    return [float(x) for x in re.findall(r'-?\d+(?:\.\d+)?', cell)]


def values(row, kind):
    cells = [c.strip() for c in row.strip().strip('|').split('|')]
    if kind == 'window':  # run mode builds wrk forced p95 hov gPS worst vid rss ...
        b, w, f, p, h, g, wo, v, r = (nums(cells[i]) for i in (2, 3, 4, 5, 6, 7, 8, 9, 10))
        flat = b[:2] + w[:1] + f[:2] + p[:1] + h[:1] + g[:2] + wo[:1] + v[:1] + r[:1]
        return dict(zip(WIN, flat)) if len(flat) == len(WIN) else {}
    f, mx, sh, wo, r = (nums(cells[i]) for i in (2, 3, 4, 5, 6))
    flat = f[:2] + mx[:1] + sh[:1] + wo[:1] + r[:1]
    return dict(zip(HL, flat)) if len(flat) == len(HL) else {}


def main():
    out, prefix = sys.argv[1], sys.argv[2]
    tags = sorted({f.rsplit('.', 1)[0] for f in os.listdir(out) if f.endswith('.meta') and f.startswith(prefix)})
    groups = {}
    cols = WIN
    for t in tags:
        m = re.match(re.escape(prefix) + r'(.+)-(live|rep)-(\d+)$', t)
        if not m:
            continue
        kind = summarize.meta(os.path.join(out, t + '.meta')).get('kind')
        r = summarize.window_row(out, t) if kind == 'window' else summarize.headless_row(out, t)
        if r:
            cols = WIN if kind == 'window' else HL
            groups.setdefault((m.group(1), m.group(2)), []).append(values(r[0], kind))
    print('| page | variant | runs | ' + ' | '.join(cols) + ' |')
    print('|' + '---|' * (3 + len(cols)))
    for (page, var), rows in sorted(groups.items()):
        cells = []
        for c in cols:
            v = [r[c] for r in rows if c in r]
            if not v:
                cells.append('-')
                continue
            md = statistics.median(v)
            sp = (max(v) - min(v)) / md * 100 if md else 0
            cells.append(f'{min(v):.0f} / {md:.0f} / {max(v):.0f} ({sp:.0f}%)')
        print(f'| {page} | {var} | {len(rows)} | ' + ' | '.join(cells) + ' |')


if __name__ == '__main__':
    main()
