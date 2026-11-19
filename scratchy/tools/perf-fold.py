#!/usr/bin/env python3 -I
"""QOBS1: fold `perf script` output and query the folded stacks (stdlib only).

Run with `python3 -I tools/perf-fold.py ...` (isolated mode). Input comes from
`perf script -i perf.data -F comm,tid,ip,sym`.

  fold [OUT]                  stdin = perf script; writes folded stacks
                              "comm;root;...;leaf COUNT" to OUT (default stdout)
  incl FOLDED REGEX...        inclusive samples (a stack counts once) per regex,
                              with the share of all samples
  callers FOLDED REGEX [N]    for stacks containing a frame matching REGEX: the N
                              (default 6) frames above its first match, aggregated;
                              Rust callers show as symbols
  jscallers FOLDED REGEX      for the same stacks: the nearest PAGE JS frame
                              (`JS:` frames that are not scratchy's own prelude,
                              i.e. not `eval:` / `timers:`) above the match
  summary FOLDED              per-thread share, top 30 self, top 40 inclusive, and
                              the page JS frames above GeomCache::refresh
"""
import re
import sys
from collections import Counter

PRELUDE = re.compile(r" (eval|timers):")
# thread/runtime plumbing that is on every stack; hidden from the summary's inclusive list
PLUMBING = re.compile(
    r"^(\[unknown\]|_start|main|start_thread|__GI___clone3|__libc_start\w*|std::rt::|std::panic::|"
    r"std::sys::|<std::thread::|core::ops::function|std::thread::|scratchy::run$)|catch_unwind|"
    r"__rust_begin_short_backtrace|lang_start|^\{closure#\d+\}")


def fold(out):
    stacks = Counter()
    frames = None
    comm = ""

    def flush():
        if frames:
            stacks[";".join([comm] + frames[::-1])] += 1

    for line in sys.stdin:
        line = line.rstrip("\n")
        if not line.strip():
            flush()
            frames = None
            continue
        if line[0] not in " \t":  # header: "comm tid"
            flush()
            s = line.strip()
            comm = (s.rsplit(None, 1)[0] if " " in s else s).replace(";", ":")
            frames = []
        elif frames is not None:
            parts = line.strip().split(None, 1)
            sym = parts[1] if len(parts) > 1 else "[unknown]"
            frames.append(sym.replace(";", ":"))
    flush()
    w = open(out, "w") if out else sys.stdout
    for s, n in stacks.most_common():
        w.write(f"{s} {n}\n")
    if out:
        w.close()
    total = sum(stacks.values())
    threads = Counter()
    for s, n in stacks.items():
        threads[s.split(";", 1)[0]] += n
    print(f"fold: {total} samples", file=sys.stderr)
    for t, n in threads.most_common(8):
        print(f"  {100 * n / max(total, 1):5.1f}%  {t}", file=sys.stderr)


def load(path):
    stacks = []
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            if not line:
                continue
            s, _, n = line.rpartition(" ")
            stacks.append((s.split(";"), int(n)))
    return stacks


def pct(n, total):
    return f"{100 * n / max(total, 1):5.1f}%"


def incl(path, regexes):
    stacks = load(path)
    total = sum(n for _, n in stacks)
    print(f"total samples: {total}")
    for rx in regexes:
        r = re.compile(rx)
        n = sum(c for fr, c in stacks if any(r.search(x) for x in fr[1:]))
        print(f"{pct(n, total)} {n:8d}  {rx}")


def callers(path, rx, depth):
    r = re.compile(rx)
    agg = Counter()
    total = 0
    for fr, c in load(path):
        body = fr[1:]
        for i, x in enumerate(body):
            if r.search(x):
                total += c
                agg[" <- ".join(reversed(body[max(0, i - depth):i + 1]))] += c
                break
    print(f"{rx}: {total} samples (chains: match <- caller <- ...)")
    for k, n in agg.most_common(25):
        print(f"{pct(n, total)} {n:7d}  {k}")


def jscallers(path, rx):
    r = re.compile(rx)
    agg = Counter()
    total = 0
    for fr, c in load(path):
        body = fr[1:]
        for i, x in enumerate(body):
            if r.search(x):
                total += c
                js = "[no page JS frame]"
                for y in reversed(body[:i]):
                    if y.startswith("JS:") and not PRELUDE.search(y):
                        js = y
                        break
                agg[js] += c
                break
    print(f"{rx}: {total} samples, nearest page JS frame above it")
    for k, n in agg.most_common(15):
        print(f"{pct(n, total)} {n:7d}  {k if len(k) <= 200 else k[:197] + '...'}")


def summary(path):
    stacks = load(path)
    total = sum(n for _, n in stacks)
    print(f"samples: {total}\n\n== per thread")
    th = Counter()
    for fr, c in stacks:
        th[fr[0]] += c
    for k, n in th.most_common(10):
        print(f"{pct(n, total)} {n:8d}  {k}")
    print("\n== top 30 self")
    self_ = Counter()
    for fr, c in stacks:
        self_[fr[-1]] += c
    for k, n in self_.most_common(30):
        print(f"{pct(n, total)} {n:8d}  {k}")
    print("\n== top 40 inclusive")
    inc = Counter()
    for fr, c in stacks:
        for x in set(fr[1:]):
            if not PLUMBING.search(x):
                inc[x] += c
    for k, n in inc.most_common(40):
        print(f"{pct(n, total)} {n:8d}  {k}")
    print()
    jscallers(path, r"GeomCache>?::refresh$")


def main():
    a = sys.argv[1:]
    try:
        if a[0] == "fold":
            fold(a[1] if len(a) > 1 else None)
        elif a[0] == "incl" and len(a) >= 3:
            incl(a[1], a[2:])
        elif a[0] == "callers" and len(a) >= 3:
            callers(a[1], a[2], int(a[3]) if len(a) > 3 else 6)
        elif a[0] == "jscallers" and len(a) == 3:
            jscallers(a[1], a[2])
        elif a[0] == "summary" and len(a) == 2:
            summary(a[1])
        else:
            raise IndexError
    except IndexError:
        print(__doc__, file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
