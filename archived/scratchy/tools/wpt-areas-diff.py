#!/usr/bin/env python3
"""QWPT5: markdown table of wpt-th/areas.txt, with per-area deltas against a base copy.
Usage: tools/wpt-areas-diff.py NEW [BASE]    (BASE = areas.txt from origin/main; omitted = no diff)
Columns as written by `scratchy --wpt-th`: area files ran all-pass subtests pass pass% score."""
import sys


def rows(path):
    out = {}
    try:
        lines = open(path).read().splitlines()
    except OSError:
        return out
    for l in lines:
        if not l.strip() or l.startswith("#"):
            continue
        f = l.split()
        if len(f) == 8:
            out[f[0]] = [float(x) for x in f[1:]]
    return out


def d(new, old, fmt):
    if old is None:
        return fmt % new
    x = new - old
    return fmt % new if abs(x) < 1e-9 else (fmt % new) + " (" + (fmt.replace("%", "%+")) % x + ")"


new = rows(sys.argv[1])
base = rows(sys.argv[2]) if len(sys.argv) > 2 else {}
print("| area | files | all-pass | subtests | pass | pass % | score |")
print("|---|---:|---:|---:|---:|---:|---:|")
for a, v in new.items():
    b = base.get(a)
    files, ran, allp, sub, ps, pct, sc = v
    bv = b or [None] * 7
    print("| %s | %d | %s | %s | %s | %.1f | %s |" % (
        ("**TOTAL**" if a == "TOTAL" else a), files, d(allp, bv[2], "%d"), d(sub, bv[3], "%d"), d(ps, bv[4], "%d"), pct, d(sc, bv[6], "%.1f")))
if base:
    gone = [a for a in base if a not in new]
    if gone:
        print("\nAreas missing from the new table: " + ", ".join(gone))
