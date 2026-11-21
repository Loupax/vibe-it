"""Move every closed `- [x]` item out of scratchy/TODO.txt.
Stubs ending in `[archived: TODO-DONE.txt]` are deleted if TODO-DONE.txt has the item;
full entries (+ deeper-indented continuation lines, stopping at blank/`- [` lines) are appended to TODO-DONE.txt.
usage: move_closed.py <repo> [--apply]"""
import re, sys
repo=sys.argv[1]; apply='--apply' in sys.argv
tp,dp=f"{repo}/scratchy/TODO.txt",f"{repo}/scratchy/TODO-DONE.txt"
L=open(tp).read().split('\n'); D=open(dp).read()
ind=lambda s: len(s)-len(s.lstrip(' '))
keep=[]; moved=[]; stubs=0; stub_missing=[]; i=0
while i<len(L):
    l=L[i]; m=re.match(r'^(\s*)- \[x\] (\S+)',l)
    if not m: keep.append(l); i+=1; continue
    k=ind(l); j=i+1
    while j<len(L):
        if L[j].strip() and ind(L[j])>k and not re.match(r'^\s*- \[',L[j]): j+=1; continue
        # a blank line followed by more deeper-indented prose = same entry (except QM1k: its PLAN stays for open items)
        if not L[j].strip() and m.group(2)!='QM1k':
            n=j
            while n<len(L) and not L[n].strip(): n+=1
            if n<len(L) and ind(L[n])>k and not re.match(r'^\s*- \[',L[n]): j=n; continue
        break
    while j>i+1 and not L[j-1].strip(): j-=1
    block=L[i:j]
    if l.rstrip().endswith('[archived: TODO-DONE.txt]') and j==i+1:
        iid=m.group(2)
        if iid in D: stubs+=1
        else: stub_missing.append(iid); moved.append(block)
    else:
        moved.append(block)
    i=j
print(f"stubs deleted: {stubs}; stubs NOT found in TODO-DONE (moved instead): {stub_missing}")
print(f"full entries moved: {len(moved)-len(stub_missing)} ({sum(len(b) for b in moved)} lines)")
print(f"TODO.txt lines: {len(L)} -> {len(keep)}")
if apply:
    open(tp,'w').write('\n'.join(keep))
    with open(dp,'a') as f:
        f.write('\n=== Closed items moved out of TODO.txt by the coordinator 2026-11-19 (user: keep TODO.txt small) ===\n')
        for b in moved: f.write('\n'.join(b)+'\n')
