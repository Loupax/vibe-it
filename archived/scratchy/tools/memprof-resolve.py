#!/usr/bin/env python3
"""QMEM3g: turn a `--features memprof` dump into a top-N table of live heap by function.

    python3 -I tools/memprof-resolve.py memprof.out [--top 10] [--frames 1] [--skip REGEX] [--threads]

Symbolizes with addr2line (binutils; the binary needs symbols: the default release profile keeps
them unless stripped; `--profile profiling` adds line tables). Frames are grouped by the first
--frames "interesting" frames: frames in the allocator, std/core/alloc and libc are skipped
(--skip adds more regexes). Output: est. live bytes, samples, share, function.
"""
import argparse, collections, re, subprocess, sys

DEFAULT_SKIP = r"^(alloc::|core::|std::|<alloc::|<core::|<std::|hashbrown::|scratchy::memprof::|<scratchy::memprof::|__rust|_?rust_|backtrace|__libc|malloc|calloc|realloc|posix_memalign|operator new|operator delete|cfree|\?\?)"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dump")
    ap.add_argument("--top", type=int, default=10)
    ap.add_argument("--frames", type=int, default=1, help="group by this many non-skipped frames")
    ap.add_argument("--skip", action="append", default=[], help="extra regex of function names to skip")
    ap.add_argument("--threads", action="store_true", help="also split by thread name")
    a = ap.parse_args()

    maps, threads, sites, base = [], {}, [], {}
    hdr = ""
    for line in open(a.dump):
        p = line.split()
        if not p:
            continue
        if p[0] == "MEMPROF":
            hdr = line.strip()
        elif p[0] == "MAP":
            lo, hi = (int(x, 16) for x in p[1].split("-"))
            maps.append((lo, hi, int(p[3], 16), p[-1]))
            if int(p[3], 16) == 0:
                base.setdefault(p[-1], lo)
        elif p[0] == "THREAD":
            threads[p[1]] = " ".join(p[2:])
        elif p[0] == "S":
            sites.append((p[1], int(p[2]), int(p[4]), [int(x, 16) for x in p[5:]]))

    def locate(addr):
        a1 = addr - 1  # return address: look up the call instruction
        for lo, hi, off, path in maps:
            if lo <= a1 < hi:
                # virtual address inside the ELF (what addr2line wants): offset 0 maps the
                # file start, so vaddr = address - load base (also right for PIE binaries
                # whose text segment vaddr differs from its file offset).
                return path, a1 - base.get(path, lo - off)
        return None, None

    want = collections.defaultdict(set)
    for _, _, _, fr in sites:
        for x in fr:
            path, off = locate(x)
            if path:
                want[path].add(off)
    name = {}
    for path, offs in want.items():
        offs = sorted(offs)
        try:
            out = subprocess.run(["addr2line", "-f", "-C", "-e", path] + [hex(o) for o in offs],
                                 capture_output=True, text=True, check=True).stdout.split("\n")
        except (OSError, subprocess.CalledProcessError) as e:
            print(f"addr2line failed for {path}: {e}", file=sys.stderr)
            continue
        for i, o in enumerate(offs):
            fn = out[2 * i] if 2 * i < len(out) else "??"
            name[(path, o)] = re.sub(r"::h[0-9a-f]{16}$", "", fn)

    skip = re.compile("|".join([DEFAULT_SKIP] + a.skip))

    def fname(addr):
        path, off = locate(addr)
        return name.get((path, off), "??") if path else "??"

    groups = collections.defaultdict(lambda: [0, 0])
    total = 0
    for tid, w, n, fr in sites:
        keep = []
        for x in fr:
            f = fname(x)
            if skip.search(f):
                continue
            keep.append(f)
            if len(keep) >= a.frames:
                break
        key = " <- ".join(keep) or "(no frame left after skip)"
        if a.threads:
            key = f"[{threads.get(tid, tid)}] " + key
        g = groups[key]
        g[0] += w
        g[1] += n
        total += w
    print(hdr)
    print(f"total estimated live bytes: {total} ({total / 1048576:.1f} MiB), {len(sites)} sites")
    print(f"{'MiB':>9} {'samples':>8} {'share':>6}  function")
    for key, (w, n) in sorted(groups.items(), key=lambda kv: -kv[1][0])[: a.top]:
        print(f"{w / 1048576:9.2f} {n:8d} {100 * w / max(total, 1):5.1f}%  {key}")


if __name__ == "__main__":
    main()
