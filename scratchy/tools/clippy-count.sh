#!/bin/sh
# Standard clippy warning count (AGENTS.md checklist step 3).
# Usage: tools/clippy-count.sh [outfile]   (run from scratchy/)
# Counts DISTINCT warnings = unique (lint name, file:line:col) pairs from the
# JSON diagnostics of `cargo clippy --release --all-targets`, so a warning that
# is reported once per target (lib/bin/test) is counted once, and the
# "N warnings emitted" summary lines are ignored. Prints a per-lint breakdown
# and a final "TOTAL <n>". The list itself goes to $1 (default: stdout skipped).
out="${1:-/dev/null}"
cargo clippy --release --all-targets --message-format=json 2>/dev/null | python3 -c '
import sys, json, collections
seen = {}
for line in sys.stdin:
    try: m = json.loads(line)
    except ValueError: continue
    if m.get("reason") != "compiler-message": continue
    d = m["message"]
    if d.get("level") != "warning" or not d.get("code"): continue
    sp = [s for s in d["spans"] if s["is_primary"]]
    if not sp: continue
    s = sp[0]
    seen[(d["code"]["code"], "%s:%d:%d" % (s["file_name"], s["line_start"], s["column_start"]))] = 1
keys = sorted(seen, key=lambda k: (k[0], k[1]))
c = collections.Counter(k[0] for k in keys)
for l, n in c.most_common(): print("%4d %s" % (n, l))
print("TOTAL", len(keys))
open(sys.argv[1], "w").write("\n".join("%s %s" % k for k in keys) + "\n")
' "$out"
