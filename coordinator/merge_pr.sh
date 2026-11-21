#!/bin/bash
# Coordinator merge queue: merge ONE PR safely.
# usage: merge_pr.sh <pr-number>
# 1. Always rebase onto origin/main in a temp worktree; PROMPTS.md/REGRESSIONS.md conflicts keep both
#    (main first); a conflict in scratchy/wpt-th/areas.txt is rebuilt with `scratchy --wpt-th --areas-only` (QWPT10);
#    any other conflict or duplicate R-heading -> exit 2; if the head changed, force-push, wait CI.
# 2. Require all four checks green on the PR head, then squash-merge.
# Exit 0 = merged; 1 = CI red (resume the agent); 2 = conflict it could not resolve; 3 = other.
set -u
N=$1
REPO=$(cd "$(dirname "$0")/.." && pwd)
PAD=$REPO/.coordinator/tmp; mkdir -p "$PAD"   # local scratch, git-ignored
cd "$REPO" || exit 3
git fetch -q origin || exit 3
BR=$(gh pr view "$N" --json headRefName -q .headRefName) || exit 3
git fetch -q origin "$BR" || exit 3
HEAD=origin/$BR
BASE=$(git merge-base "$HEAD" origin/main)
CODE=$(git diff --name-only "$BASE" origin/main | grep -Ev '^(PROMPTS\.md|README\.md)$|^scratchy/(TODO\.txt|TODO-DONE\.txt|PROMPTS\.md|REGRESSIONS\.md|PERF\.md|NOTES\.md|AGENTS\.md|QUESTIONS\.md|HANDOFF\.md)$' | head -1)
# all checks SUCCESS (4 jobs today; 3 after QV8-6 drops scratchy-quickjs)
green() { [ "$(gh pr view "$N" --json statusCheckRollup -q '([.statusCheckRollup[]|select(.conclusion=="SUCCESS")]|length) as $ok | (.statusCheckRollup|length) as $all | if $all>=3 and $ok==$all then "y" else "n" end')" = y ]; }
waitci() { sleep 40; timeout 2700 gh pr checks "$N" --watch >/dev/null 2>&1; green; }
# user rule (session 2): ALWAYS rebase onto the latest main; ANY conflict -> exit 2 (back to the agent);
# a clean rebase that changed the head -> force-push and wait for CI again.
W=$PAD/mq-$N; rm -rf "$W"; git worktree prune
git worktree add -q -B "mq-$N" "$W" "$HEAD" || exit 3
# user 2026-11-19 ~22:25: conflicts ONLY in the append-only scratchy/PROMPTS.md / scratchy/REGRESSIONS.md are resolved
# keep-both (main's side first); any other conflicted file, or a duplicate `## R<n>:` heading, -> exit 2 (back to the agent).
keepboth() {
  python3 -I - "$1" <<'PYEOF'
import sys
p=sys.argv[1]; L=open(p).read().split('\n'); out=[]; i=0
while i<len(L):
    if L[i].startswith('<<<<<<< '):
        m=next(j for j in range(i,len(L)) if L[j]=='=======')
        e=next(j for j in range(m,len(L)) if L[j].startswith('>>>>>>> '))
        a=L[i+1:m]; b=L[m+1:e]
        out+=a+([''] if a and a[-1]!='' and b and b[0]!='' else [])+b; i=e+1
    else: out.append(L[i]); i+=1
open(p,'w').write('\n'.join(out))
PYEOF
}
# user 2026-11-20 ~14:50: scratchy/src/tests/mod.rs conflicts are resolved keep-both too, but ONLY when every line on
# both sides of every conflict hunk is a `mod <name>;` line (optionally `pub`/`#[cfg(...)]`) or blank; duplicates are
# dropped. Anything else in that file -> exit 2 (back to the agent). CI still gates the result.
modlines() {
  python3 -I - "$1" <<'PYEOF'
import sys,re
p=sys.argv[1]; L=open(p).read().split('\n'); out=[]; i=0
ok=re.compile(r'^\s*(#\[cfg\([^)]*\)\]\s*)?(pub(\(crate\))?\s+)?mod\s+\w+\s*;\s*(//.*)?$|^\s*$')
while i<len(L):
    if L[i].startswith('<<<<<<< '):
        m=next(j for j in range(i,len(L)) if L[j]=='=======')
        e=next(j for j in range(m,len(L)) if L[j].startswith('>>>>>>> '))
        a=L[i+1:m]; b=L[m+1:e]
        if not all(ok.match(x) for x in a+b): sys.exit(1)
        seen=set(); merged=[]
        for x in a+b:
            k=x.strip()
            if k and k in seen: continue
            seen.add(k); merged.append(x)
        out+=merged; i=e+1
    else: out.append(L[i]); i+=1
open(p,'w').write('\n'.join(out))
PYEOF
}
# QWPT10: scratchy/wpt-th/areas.txt is derived from wpt-th/{subset.txt,expected/,flaky.txt}: on a conflict take either
# side, then recompute it with `scratchy --wpt-th --areas-only`. Binary: $W/scratchy/target/release/scratchy if it exists,
# else build it there (nice -n 19 ionice -c3 cargo build --release, RUSTY_V8_ARCHIVE when present), else
# ~/.cargo/bin/scratchy only if it supports --areas-only. Echoes the binary used; returns 1 if none is usable.
AREAS=scratchy/wpt-th/areas.txt
SBIN=
areasbin() {
  if [ -z "$SBIN" ]; then
    local c=$W/scratchy/target/release/scratchy v8=$HOME/.cache/librusty_v8-152.2.0.a
    if [ ! -x "$c" ]; then
      echo "PR #$N: building scratchy for --areas-only (release build in $W/scratchy)" >&2
      ( cd "$W/scratchy" && if [ -f "$v8" ]; then export RUSTY_V8_ARCHIVE=$v8 CARGO_BUILD_JOBS=4; fi; nice -n 19 ionice -c3 cargo build --release >/dev/null 2>&1 )
    fi
    if [ -x "$c" ] && grep -qa -- '--areas-only' "$c"; then SBIN=$c
    elif [ -x "$HOME/.cargo/bin/scratchy" ] && grep -qa -- '--areas-only' "$HOME/.cargo/bin/scratchy"; then SBIN=$HOME/.cargo/bin/scratchy
    else return 1; fi
    echo "PR #$N: areas.txt rebuilt with $SBIN" >&2
  fi
  echo "$SBIN"
}
regen_areas() {
  local b; b=$(areasbin) || return 1
  git checkout -q --theirs -- "$AREAS" 2>/dev/null   # either side; the content is replaced below
  ( cd "$W/scratchy" && "$b" --wpt-th --areas-only --dir wpt-th >/dev/null 2>&1 ) || return 1
  git add "$AREAS"
}
if ! ( cd "$W" && git rebase -q origin/main >/dev/null 2>&1 ); then
  cd "$W"
  TRIES=0
  while git status | grep -q 'rebase in progress'; do
    TRIES=$((TRIES+1)); if [ $TRIES -gt 30 ]; then echo "PR #$N: rebase stuck"; git rebase --abort; cd "$REPO"; git worktree remove --force "$W"; git branch -D "mq-$N" >/dev/null; exit 3; fi
    U=$(git diff --name-only --diff-filter=U)
    BAD=$(echo "$U" | grep -Ev '^(scratchy/PROMPTS\.md|scratchy/REGRESSIONS\.md|scratchy/src/tests/mod\.rs|scratchy/wpt-th/areas\.txt)?$' | tr '\n' ' ')
    if [ -n "$BAD" ]; then
      echo "PR #$N: rebase CONFLICT in: $BAD"
      git rebase --abort; cd "$REPO"; git worktree remove --force "$W"; git branch -D "mq-$N" >/dev/null; exit 2
    fi
    for f in $U; do
      if [ "$f" = scratchy/src/tests/mod.rs ]; then
        if ! modlines "$f"; then
          echo "PR #$N: rebase CONFLICT in: $f (not only mod lines)"
          git rebase --abort; cd "$REPO"; git worktree remove --force "$W"; git branch -D "mq-$N" >/dev/null; exit 2
        fi
        git add "$f"
      elif [ "$f" = "$AREAS" ]; then
        if ! regen_areas; then
          echo "PR #$N: rebase CONFLICT in: $f (could not rebuild it with --areas-only: no usable scratchy binary)"
          git rebase --abort; cd "$REPO"; git worktree remove --force "$W"; git branch -D "mq-$N" >/dev/null; exit 2
        fi
      else
        keepboth "$f" && git add "$f"
      fi
    done
    [ -n "$U" ] && echo "PR #$N: keep-both resolved: $(echo $U)"
    GIT_EDITOR=true git rebase --continue >/dev/null 2>&1 || true
  done
  DUP=$(grep -oE '^## R[0-9]+:' scratchy/REGRESSIONS.md | sort | uniq -d | tr '\n' ' ')
  cd "$REPO"
  if [ -n "$DUP" ]; then
    echo "PR #$N: duplicate REGRESSIONS headings after keep-both: $DUP (back to the agent to renumber)"
    git worktree remove --force "$W"; git branch -D "mq-$N" >/dev/null; exit 2
  fi
fi
NEWH=$(cd "$W" && git rev-parse HEAD)
if [ "$NEWH" != "$(git rev-parse "$HEAD")" ]; then
  echo "PR #$N: rebased onto origin/main (main code moved: ${CODE:-no}), pushing"
  ( cd "$W" && git push -q -f origin "HEAD:$BR" ) || { git worktree remove --force "$W"; exit 3; }
  git worktree remove --force "$W"; git branch -D "mq-$N" >/dev/null
  waitci || { echo "PR #$N: CI NOT green after rebase"; exit 1; }
else
  echo "PR #$N: already on latest main"
  git worktree remove --force "$W"; git branch -D "mq-$N" >/dev/null
  green || waitci || { echo "PR #$N: CI NOT green"; exit 1; }
fi
gh pr merge "$N" --squash --delete-branch >/dev/null 2>&1
S=$(gh pr view "$N" --json state,mergeCommit -q '"\(.state) \(.mergeCommit.oid[0:7])"')
echo "PR #$N: $S"
case "$S" in MERGED*) exit 0;; *) exit 3;; esac
