#!/usr/bin/env python3
"""QOBS8: diff two `--perf-report` objects or two `tools/perf-suite.sh` outputs.

  python3 -I tools/perf-diff.py old.json new.json [--threshold 5]

Prints a table of the numeric fields whose change exceeds the threshold (percent of
the old value), largest change first: page, field, old, new, change %. A field that
is 0 in old and non-zero in new shows `new` (infinite change) and sorts first.

Inputs: a `--perf-report` object (page `(report)`), or a perf-suite output (one page
per `pages.<url>.<mode>` median, plus `corpus`). Both sides must be the same kind.

Walking is generic, so new schema-2 fields (forced_layout, memory, ...) need no change
here. Every numeric leaf is compared by dotted path (booleans are not numbers).
Arrays of objects whose entries have a `name` (the corpus) are keyed by that name;
every other array (`longest`, number lists) is skipped: its entries are ordered by
cost, not identity, so an index compares unrelated callbacks. Fields present on only
one side, and pages that are not `ok` on both sides, are counted on stderr, not diffed.
Run-to-run noise is typically several percent: diff main vs main to see your floor.
"""
import argparse, json, math, sys


def flatten(node, prefix, out):
    if isinstance(node, bool):
        return
    if isinstance(node, (int, float)):
        out[prefix] = node
    elif isinstance(node, dict):
        for k, v in node.items():
            flatten(v, prefix + "." + k if prefix else k, out)
    elif isinstance(node, list):
        for e in node:
            if isinstance(e, dict) and isinstance(e.get("name"), str):
                flatten(e, (prefix + "." if prefix else "") + e["name"], out)


def sections(doc):
    """({page label: {field path: number}}, count of skipped page/mode entries)."""
    secs, skipped = {}, 0
    if isinstance(doc.get("pages"), dict):
        for url, modes in doc["pages"].items():
            for mode, ent in modes.items():
                if ent.get("status") != "ok" or "median" not in ent:
                    skipped += 1
                    continue
                f = {}
                flatten(ent["median"], "", f)
                secs["%s [%s]" % (url, mode)] = f
        if isinstance(doc.get("corpus"), dict):
            f = {}
            flatten(doc["corpus"], "", f)
            secs["(corpus)"] = f
    else:
        f = {}
        flatten(doc, "", f)
        secs["(report)"] = f
    return secs, skipped


def fmt(x):
    return ("%d" % x) if float(x).is_integer() and abs(x) < 1e15 else "%.4g" % x


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("--threshold", type=float, default=5.0, help="minimum change in percent (default 5)")
    a = ap.parse_args()
    docs = []
    for p in (a.old, a.new):
        try:
            with open(p) as fh:
                docs.append(json.load(fh))
        except (OSError, ValueError) as e:
            sys.exit("perf-diff: %s: %s" % (p, e))
    (so, sk_o), (sn, sk_n) = sections(docs[0]), sections(docs[1])
    if ("(report)" in so) != ("(report)" in sn):
        sys.exit("perf-diff: one input is a --perf-report, the other a perf-suite output")
    rows, only = [], 0
    for page, fo in so.items():
        fn = sn.get(page)
        if fn is None:
            only += len(fo)
            continue
        only += len(set(fo) ^ set(fn))
        for k in fo.keys() & fn.keys():
            o, n = fo[k], fn[k]
            if o == n:
                continue
            pct = math.inf if o == 0 else (n - o) / abs(o) * 100.0
            if abs(pct) > a.threshold:
                rows.append((pct, page, k, o, n))
    only += sum(len(f) for p, f in sn.items() if p not in so)
    rows.sort(key=lambda r: (-abs(r[0]), r[1], r[2]))
    print("perf-diff: %s -> %s (threshold %g%%)" % (a.old, a.new, a.threshold))
    if rows:
        cells = [(p, k, fmt(o), fmt(n), "new" if math.isinf(c) else "%+.1f%%" % c) for c, p, k, o, n in rows]
        hdr = ("page", "field", "old", "new", "change")
        w = [max(len(r[i]) for r in cells + [hdr]) for i in range(5)]
        for r in [hdr] + cells:
            print("  ".join(r[i].ljust(w[i]) if i < 2 else r[i].rjust(w[i]) for i in range(5)))
    else:
        print("no field changed by more than the threshold")
    print("perf-diff: %d changed; %d fields on one side only, %d page/mode entries not ok (skipped)"
          % (len(rows), only, sk_o + sk_n), file=sys.stderr)


main()
