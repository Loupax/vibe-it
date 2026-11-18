# Questions for the user (coordinator, 2026-11-18)

Answer inline (edit this file and push, or reply in the session). Short answers are fine:
"Q1: c". Where I give a RECOMMENDED default, saying "defaults" for the whole file is OK.
Everything else in the queue is being worked on without you; these are the only items that
need your input. Evidence for each is in the file/section named.

## Q1. CPU budget policy for scripts (QY49-D; REGRESSIONS.md R37 section 3)
**DEFERRED (user, 2026-11-18): decided after the V8 migration, from the QV8-m numbers.**
Real pages need 0.5-1.7 s of CPU in a single callback, always while the page loads (YouTube: a
1.66 s promise job; DuckDuckGo: a 500+ ms timer). Today every callback gets 500 ms and is then
killed (after QY46 each promise job has its own 500 ms; timers have one 500 ms for the whole
callback). The 500 ms exists because long main-thread stalls while the pointer moved got scratchy
disconnected by the compositor (R8/QB3). Note: forced layout is not counted against the budget
and already stalls up to 3.9 s, so 500 ms is not a bound on the stall today.
- (a) Only make callbacks cheaper (QY51, QY52, QY53 are doing this). Pages that need more keep dying.
- (b) A larger budget always (e.g. 2 s).
- (c) A larger budget (2-5 s) only while the page is loading, 500 ms afterwards. **RECOMMENDED**
- (d) Input-aware: large budget unless input arrived in the last ~250 ms.
Also: what counts as "loading" for (c): until the `load` event plus 2 s? (my default)

## Q2. Settings rules so nested-dwl runs are never blocked
You gave a standing OK for agents' nested-dwl runs (saved in memory), but the permission
classifier is separate and denied one run once. I can add allow rules to
`.claude/settings.json` (today only `Bash(gh pr merge:*)`): `Bash(tools/nest.sh:*)`,
`Bash(grim:*)`, `Bash(WAYLAND_DISPLAY=*:*)` (the last is wide: any command with that prefix).
Add them? (yes / yes but drop the wide one / no, keep approving)

## Q3. DuckDuckGo (QY55)
The investigator's one automated load got DuckDuckGo's bot challenge ("select all squares with a
duck", the grid empty). You earlier said it did not show the challenge for you.
- Does scratchy show you search RESULTS on DuckDuckGo now, or the challenge, or nothing?
- Does a normal browser on the same IP show results?
If you get results, I will have an agent profile the tripping timer in a session you run (no
more automated searches, never solving the challenge).

## Q4. MSE for video (QM1k) - ANSWERED 2026-11-18 (user), all items
1. ~60 s of YouTube playback is OK as the first milestone (PoToken research later).
2. Start with mp4 (avc1/mp4a), but PLAN to enable the others (WebM/Opus/VP9/AV1) later: keep the
   design codec-agnostic (mpv does the demuxing).
3. Audio into mpv: ANSWERED, option A (user 2026-11-18; revisit with B if desyncs appear). (A) a second
   input stream (`audio-files`) because mpv does all container work and new codecs cost nothing;
   (B) remuxing in scratchy is the fallback if A has sync/seek problems; the plan must include an
   early test that decides it.
4. 64 MB per player to start; revisit after testing.
5. Native HLS: later.
6. Workers/WebAssembly (Twitch): INCLUDE, at lower priority than the MSE work.
Timing: the V8 freeze still applies; the QM1k plan (Opus, docs only) starts after QV8-m unless the
user says to run it earlier. **PLAN WRITTEN 2026-11-18** (TODO.txt QM1k-PLAN, R34 addendum); the
build items start after the freeze. One follow-up: Q7.

## Q7. MSE: may scratchy read container TIMING headers? (QM1k1a; R34 addendum)
`SourceBuffer.buffered` must be correct the moment `updateend` fires (YouTube decides what to fetch
from it), so it cannot wait for mpv. The plan reads only box headers of the appended fMP4 (timescale,
fragment start `tfdt`, sample durations in `trun`; later WebM cluster times): no codec data, no
samples, mpv still does all demuxing and decoding. New codecs still cost nothing; a new CONTAINER
(WebM) costs a small header reader (QM1k1h).
- (a) Yes, read the timing headers. **RECOMMENDED**
- (b) No: take `buffered` from mpv (it lags behind `updateend`; players may refetch or stall).

## Q5. WPT in CI (QCI2)
Run the WPT subset in GitHub Actions: sparse-checkout only the directories `wpt-subset.txt`
uses, pinned to your WPT commit. Download size is small but non-zero. Do it? (yes / no / later)

## Q6. `--profile` jar name (QF14b2 (d))
Cookie jars are in memory and named `scratchy` today. Do you want a `--profile NAME` flag so
different windows use separate jars, or skip it for now? (my default: skip)

## What I do while you are away (no answer needed)
**Changed 2026-11-18 (user): V8 migration first.** Finish and merge what is already running,
then start nothing else from the old queue. Next: QT1 (split src/tests.rs, pure move, in
the quiet window right after the drain; user 2026-11-18), then QV8-0 (QuickJS baseline with a permanent
`--perf-report` harness), QV8-1 (V8 spike, stop-and-report gates), QV8-2 (engine seam),
QV8-3 (V8 backend), QV8-4 (snapshot), QV8-5 (V8 default), QV8-m (same measurements on V8,
before/after table in PERF.md), then lift the freeze. Details: TODO.txt top block.
Paused until then (kept READY): QY53, QY54, QY57-QY61, QAN1e, QM1k5, QY15a, QY33, Q30e2/Q30d2/
Q30d3/Q30g2/Q30a2, QM1f1/QM1g2, QY44d/e, QSD5, QC2. Q3-Q6 above still stand.
Reinstall scratchy after merges; I will not restart your wire.
