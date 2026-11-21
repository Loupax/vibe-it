"""Move a finished item from scratchy/TODO.txt to scratchy/TODO-DONE.txt.
usage: python3 -I done.py <repo> <ITEM> <hash> [<note>]
Removes the detail entry (the `- [ ]` line and its continuation lines), appends it
ticked to TODO-DONE.txt under `=== from TODO.txt: <block header>`, and moves the
PERFORMANCE QUEUE list line along with it.
"""
import re, sys
repo, item, h = sys.argv[1:4]
note = sys.argv[4] if len(sys.argv) > 4 else ""
tp, dp = f"{repo}/scratchy/TODO.txt", f"{repo}/scratchy/TODO-DONE.txt"
L = open(tp).read().split("\n")
start = None
for i, l in enumerate(L):
    if re.match(rf"^\s*- \[ \] (\[WIP\] )?{re.escape(item)} ", l):
        start = i
        break
if start is None:
    sys.exit(f"{item}: detail entry not found")
end = start + 1
while end < len(L) and L[end].startswith("      ") and not re.match(r"^\s*- \[", L[end]):
    end += 1
header = next((L[j] for j in range(start, -1, -1) if L[j].startswith("--- ")), "--- (no header)")
entry = L[start:end]
entry[0] = re.sub(r"- \[ \] (\[WIP\] )?", f"- [x] ", entry[0], count=1)
entry[0] = entry[0].replace(f"{item} ", f"{item} DONE {h}{(' ' + note) if note else ''} ", 1)
del L[start:end]
# queue list line: "  N [WIP] ITEM [S] ..."
# user 2026-11-19: closed items leave TODO.txt entirely -> the queue list line moves to TODO-DONE.txt too
qmoved = []
for i, l in enumerate(L):
    if re.match(rf"^\s+\d+[a-z0-9]* (\[WIP\] )?{re.escape(item)} ", l):
        qmoved.append(l); L[i] = None
L = [l for l in L if l is not None]
entry += [f"  (queue line) {q.strip()}" for q in qmoved]
open(tp, "w").write("\n".join(L))
with open(dp, "a") as f:
    f.write(f"\n=== from TODO.txt: {header}\n" + "\n".join(entry) + "\n")
print(f"{item}: moved {len(entry)} lines")
