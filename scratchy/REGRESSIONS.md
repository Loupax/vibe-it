# scratchy — regressions

## R45: QM1k-stop: YouTube's "Ein Fehler ist aufgetreten" overlay a few seconds into playback is the SABR attestation stop (`ump.spsrejectfailure` / `HTML5_SPS_UMP_STATUS_REJECTED`): after ~60-64 s of media has been served with STREAM_PROTECTION_STATUS 2 (pending), every SABR answer is status 3 (attestation required, 10 retries), and after the 10th YouTube calls `stopVideo()` and shows the error, with 27-40 s of media still buffered. Same with the default UA, the R43 Chrome UA and a second video; no MediaError, no SourceBuffer exception, no scratchy crash (Opus investigator QM1k-stop, 2026-11-19) — OPEN: QM1k2 (PARKED, user decides); no scratchy fix

**Status: investigation only, docs only.** Release build of main 42a5ef5 (V8 + media, libmpv 0.41),
own worktree, `RUSTY_V8_ARCHIVE` = local librusty_v8 152.2.0. Own wire on a scratchpad socket
(`WIRE_TIMING=1`, every line wall-clock stamped), never the user's. Nested dwl (tools/nest.sh; the
window was 960x489 or 960x1006, the inject size must match the screenshot), grim, tools/inject.
**DEFAULT user agent** (no `--user-agent`, as the user runs) except run 3. `SCRATCHY_MEDIA_AO=null`
except runs 4 and 7 (`pulse`, approved by the user, who listened). `SCRATCHY_MPV_LOG_FILE`,
`SCRATCHY_TIMING=1`. URL https://www.youtube.com/watch?v=G1AeBgalFZk (logged out, DE), and
Xa90OVJkBos (a still-image music video, the second video the user saw fail).
**Throwaway probe (appended to `mse_prelude.js`, reverted, never committed; runs 2-7 with
`SCRATCHY_V8_SNAPSHOT=0`):** media events on `document` (capture) with currentTime/readyState/
networkState/`video.error`/buffered; wrappers on every SourceBuffer/MediaSource method and setter
(logs any throw); `play`/`pause`/`load` with a JS stack; googlevideo `fetch`es: method, time to
resolve/reject, body size, UMP part counts and part 35/58 payloads (via `Response.clone()`); qoe/
stats beacons (`fetch`, `sendBeacon`, XHR); a 2 s tick with `movie_player.getPlayerState()`; when
`.ytp-error` appears, `movie_player.getDebugText(true)` (holds `debug_error`) and
`getStatsForNerds()`.

### Runs
| run | setup | click -> page `play()` | first status 3 | error overlay | `ct` at error | buffered at error |
|---|---|---|---|---|---|---|
| 1 | main binary, no probe, snapshot on | (not instrumented) | 107-B answers from player time 57 s (qoe `bwm`) | 52-67 s after the click (screenshots) | 16.27 (paused, see below) | 47.7 s ahead (qoe `bh`) |
| 2 | probe | 1.3 s | +14.4 s after the click | +43.2 s after the click | 36.8 | [0-64] |
| 3 | probe, R43 Chrome UA | (a pre-roll AD played first) | +46.7 s | +80.8 s | ~52 | [0-64] |
| 4 | probe, `ao=pulse` | CONTAMINATED: the user clicked once in the window (time unknown) | | | | |
| 5 | probe, Xa90OVJkBos | CONTAMINATED (same click), not used | | | | |
| 6 | probe, Xa90OVJkBos, clean | 3.8 s | +30.7 s | +57.0 s | ~47 | [0-105] |
| 7 | probe, `ao=pulse`, clean, the user listened | 1.2 s | +8.3 s | +30.3 s (15:38:19.5) | ~25 | [0-64] |

Every probed run ends with the SAME `debug_error`:
`{"errorCode":"ump.spsrejectfailure", ..., "HTML5_SPS_UMP_STATUS_REJECTED", "rn.18;rt.33;lb.107;
stall.0; ... req.sabr;rc.200"}` (run 2; runs 3, 6, 7 identical apart from counters). `video.error`
stayed `null` in every tick and event, no `error` event fired, no SourceBuffer/MediaSource call threw
(0 `THROW` lines), no `waiting`/`stalled`. The element ends by YouTube's own `stopVideo()` ->
`video.load()` (JS stack `g.k.stopVideo -> iHa -> g.k.load`) -> `abort` + `emptied`, and the overlay
is shown in the same task.

### What the SABR stream says (UMP part 58 = STREAM_PROTECTION_STATUS)
- Media answers carry `08 02` (status 2, attestation pending): run 2 four answers (2.44, 1.71, 3.57,
  1.54 MB), run 7 five (plus one 129-B answer). YouTube keeps playing on them.
- The first request after the buffer reaches **64.00 s** (G1AeBgalFZk, runs 2, 3, 7; Xa90OVJkBos run 6:
  60.06 s) is answered with **107 B, parts {35, 47, 58}, part 58 = `08 03 10 0a`** (status 3,
  attestation required, max retries 10). YouTube re-requests every ~2 s (`stp` in qoe `ctmp`), gets
  status 3 ten times (run 2: #116-#128, 10 answers in 24 s; run 7: 10 in 18.5 s), then gives up.
- Playback stays smooth on the buffer the whole time (run 2 ticks: ct 5.9 -> 35.9 at real speed,
  `state=1`), so the stop comes while 27-40 s of media are still buffered: nothing ran dry.
- The qoe beacons carry no `ec=`/`errorcode=`; the last one before the overlay has `vps=<t>:ER`,
  `bh=<t>:0.000` and `bwm=<t>:107:...` (each 107-byte answer is logged as a bandwidth sample). The
  error code is only in the player's own `debug_error` (above). No `/error_204` request was made.
- This is the PoToken/BotGuard attestation stop of R34/R42 (QM1k2, PARKED). R42 saw it at +92-196 s
  after the start; here it comes earlier because the page now starts faster (QY53a-h), so the
  ~64 s of unattested media is fetched sooner. Playback before the stop: 25-52 s of the video.

### Everything else seen (not the cause)
- **Chrome UA (run 3)** changes only what is served: a pre-roll ad played first (not with the
  default UA), one answer before the click had status `08 01` (OK), then the content stream went
  2 -> 3 -> the same error. So the UA does not avoid the stop.
- **SABR aborts (QM1k1n5)**: 1-5 per run (`AbortError` 7-15 s after the request). **`TypeError:
  Failed to fetch` (QM1k1n7)**: 2-7 per run = the `411 Length Required` POSTs (no ACAO header, so the
  CORS check fails; wire log runs 2-7: 6 x 411) plus `502` answers to `videoplayback` GETs on other
  googlevideo hosts (18 in runs 2-7; YouTube also sends GETs to a fake `rr5---sn-q4flrnez` URL with
  `expire=6159848608`, answered 403; which request each failure was: not checked). Neither ends playback: the error follows status 3 in every run, whatever the abort count.
- **Run 1 and run 4 pauses** (ct stuck at 13.5-13.6 for 30-50 s before the error, qoe `vps=PA`):
  YouTube's own `pauseVideo()` 2.3 s after a click that landed on an ALREADY-PLAYING video (in run 4
  the video had been started by the user's click, so the injected click paused it; in run 1 the
  window size changed between screenshot and click). Test-procedure artefact. With the pause, the
  status-3 retries still ran and the error came at the same point in the request sequence.
- **Picture late at the start / short freeze before the overlay (user's account of run 7)**:
  mpv decoded the first frame 0.22 s after `pause=no` (15:37:50.6), but logged `mpv_render_context_
  render() not being called or stuck` 29 times in the next 6 s and 12 times in the 4 s before the
  stop: scratchy's redraw did not run (R42 section 5 `top:-360px` parking until `playing` plus the
  QM1k1n6 redraw starvation), so audio led the picture by a few seconds. The "half-second hang"
  is YouTube's `stopVideo()` -> `load()` (15:38:18.48) -> `emptied` + overlay (15:38:19.51), i.e.
  the stop itself; mpv `stop` / `EOF code 4` at the same moment. No underrun, no `paused-for-cache`.
  (Runs 2 and 4 also logged one `Audio/Video desynchronisation detected!` right after the unpause.)
- **Run 4 (contaminated, real audio)**: the user heard sound with a still picture, then the sound
  froze, then the error: matches the log (user's click 15:29:08 -> `play`; injected click 15:29:20
  -> YouTube `pauseVideo()` 15:29:22 = "sound froze"; status 3 from 15:29:24; overlay 15:29:55).
- The user's own scratchy on Xa90OVJkBos failed the same way (coordinator report); run 6 reproduces
  it in a clean window with `ump.spsrejectfailure`.

### Answers
| question | answer |
|---|---|
| what stops playback | YouTube's player, on SABR STREAM_PROTECTION_STATUS 3 x10 (`ump.spsrejectfailure`): the PoToken/BotGuard attestation (QM1k2) |
| MediaError / `error` event | none (`video.error` null throughout) |
| SourceBuffer/MediaSource exception | none |
| SABR fetch aborts / late delivery (QM1k1n5) | present (1-5 aborts per run) but not the cause |
| buffer starvation | no: 27-40 s still buffered at the stop |
| mpv decode error / EOF | no; mpv `stop` only when the page unloads the element |
| redraw starvation (QM1k1n6) | present (late first picture, render stalls), not the cause |
| scratchy crash | no |
| default vs Chrome UA | same stop; the Chrome UA additionally gets a pre-roll ad |
| error beacon code | not in any query string; `debug_error.errorCode = ump.spsrejectfailure` |

### Not established
Whether status ever reaches 1 for the content stream (it did once for an ad answer in run 3 and,
in the contaminated run 5, during a long pause on Xa90OVJkBos: not reproduced cleanly, so not
claimed); what BotGuard/`GenerateIT` produce in scratchy and why the token is not accepted (that is
QM1k2's investigation, parked); the qoe POST bodies (wire logs only URLs).

## R44: Performance VISIBILITY audit: a sampling profiler (`perf` + V8's JIT map) attributes CPU to Rust functions AND page JS functions in one run, with no throwaway code. Profiles of main 8d7d692 confirm the QY53 picture, add three hotspots nobody had filed (`El::has_class` is 28-32 % of ALL CPU on both pages; custom-property interning 5.3 % on YouTube; shape-memo keys built with `format!("{:?}")` 5.5 % on Wikipedia), and show what the harness cannot see: the window (no frame/paint/input timing at all), WHICH callback was slow, and ~590 MB of native memory (Opus investigator, 2026-11-19) — OPEN: QOBS1-QOBS10, QP4, QY53i (TODO.txt block "PERFORMANCE VISIBILITY (QOBS)")

**Status: investigation only, docs only. No source change.** The user asked for more visibility on
performance bottlenecks so targeted action items can be filed, with local-only tools.

### 0. Setup (NEW machine; numbers are NOT comparable with PERF.md's tables)
- AMD Ryzen 7 5700X (8 cores / 16 threads), 31 GB RAM, Fedora 44, kernel 7.2.11, rustc 1.100.0.
  PERF.md's QV8-0 / QV8-m tables were taken on a Ryzen 5 PRO 2500U laptop (Gentoo). QOBS9 re-baselines.
- Binary 1 = `target/release/scratchy` of main 8d7d692 (normal release: symbols, no debuginfo, no frame
  pointers). Binary 2 = the SAME commit built for profiling into a target dir OUTSIDE the repo:
  ```
  RUSTY_V8_ARCHIVE=$HOME/.cache/librusty_v8-152.2.0.a CARGO_TARGET_DIR=$HOME/.cache/scratchy-prof-target \
  RUSTFLAGS="-C force-frame-pointers=yes" CARGO_PROFILE_RELEASE_DEBUG=line-tables-only cargo build --release
  ```
  (archive copied from `target/release/build/gn_out/obj/librusty_v8.a`; the build took 70 s on this machine.)
- Tools: `perf` 7.3.4 (Fedora package, installed by the user), `samply` 0.13.1 (`cargo install`, not used in
  the end: it needs `perf_event_paranoid <= 1` like perf's call graphs). The user set
  `/proc/sys/kernel/perf_event_paranoid` to 1 (was 2; NOT persistent, back to 2 on reboot). At 2, `perf record
  -e cycles:u` works but WITHOUT call graphs (`-g`, `--call-graph dwarf|lbr` all fail to open the event).
- Wire: the user's normal daemon (`/run/user/$UID/wire.sock`), headless `--perf-report --budget-ms 20000`.
  YouTube with the R43 Chrome 131 UA (user-approved there), `watch?v=G1AeBgalFZk`, 60 s; no challenge, no
  consent wall. Wikipedia `Rust_(programming_language)`, 30 s.

### 1. Recipe that worked (the core of QOBS1)
```
SCRATCHY_V8_SNAPSHOT=0 SCRATCHY_V8_FLAGS='--perf-basic-prof --interpreted-frames-native-stack' \
  perf record -e cycles:u -F 999 -g -o out.data -- $PROF_BIN --perf-report --socket ... --budget-ms 20000 --seconds 30 URL
perf script -i out.data | python3 fold.py out.folded   # folded stacks, per-thread totals
```
- `-g` = frame-pointer unwinding. With binary 1 (no frame pointers) `--call-graph dwarf` unwinds V8 frames but
  stops at the first Rust frame (every Rust leaf shows `[unknown]` as its caller): flat profiles only.
- `--perf-basic-prof` makes V8 write `/tmp/perf-<pid>.map`, so JIT frames appear as `JS:^getPlayerSize
  https://www.youtube.com/s/player/983e9c18/...` and scratchy's prelude as `JS:^gRect eval:2585:24`.
  `--interpreted-frames-native-stack` adds the interpreted (`~`) frames. Together they gave, in ONE run, the
  same "who forces the layout" table R43 needed a throwaway `new Error().stack` patch for (section 3).
- **CRASH (QOBS2):** both flags together WITH the default V8 startup snapshot segfault on Wikipedia 3/3
  (exit 139, `--seconds 3`); each flag alone 1/1 ok; both flags with `SCRATCHY_V8_SNAPSHOT=0` 3/3 ok; a tiny
  local page ok in every combination. Not investigated further (flags changed after the snapshot was baked
  is the obvious suspect; do not guess, QOBS2 measures).
- The fold / inclusive / caller scripts used are throwaway (scratchpad); QOBS1 makes them permanent.

### 2. Wikipedia Rust (30 s)
| run | binary | forced layouts | forced-layout total | max | observer_delivery raw CPU | RSS / JS heap |
|---|---|---|---|---|---|---|
| A, load avg 5 (a cargo build was running) | 1 | 8 | 10 381 ms | 1 718 ms | 10 667 ms | 305 / 21 MB |
| B, quiet, under perf | 2 | 8 | 4 306 ms | 593 ms | - | 312 / 25 MB |
(The 2.4x difference between A and B is the load, not the build flags: one-off, not a measurement of either.)

Profile B, inclusive share of ALL samples (all threads; the main thread is 99.2 %):
| frame | % |
|---|---|
| `obsFlush` (scratchy's IO prelude) | 91.7 |
| `GeomCache::refresh` (forced layout) | 91.2 |
| `build_layout_tree_with_resources` | 65.2 |
| `precompute_generated` (since QY53d it does the matching the box build then reuses) | 53.7 |
| `matching_declarations_for` | 53.3 |
| **`El::has_class` -> `str::SplitWhitespace::next`** | **31.7** (28.6 in `SplitWhitespace` itself) |
| `measure_document` (shaping + layout) | 22.8 |
| `build_children` | 10.1 |
| malloc/free/realloc | 7.8 |
| `core::fmt::*` | 6.8 |
| **`layout::memo_key`** (callers: `measure_mixed_inline`, `measure_inner`, `shape_or_resume_word`) | **5.5** |
| harfrust / cosmic-text shaping | 5.5 |
| V8 compile/parse | 3.9 |
| `do_lookup_x` (dynamic linker, from `dlopen` of libmpv mid-run: the article's video creates a player; mpv threads `lua/*`, `vo`, `av:vp9` sampled) | 1.4 |
`memo_key` (layout.rs) hashes `format!("{align:?}")` and `format!("{span:?}")` for EVERY text item of every
paragraph it looks up: a Debug string of the whole `SpanStyle` (colors, floats via `flt2dec`, enums), then
SipHash over it. That is most of the `fmt::` share. -> QP4.
Process start is not a factor: 60 ms wall, `engine_startup_cpu_ms` 27, 1 565 relocations.

### 3. YouTube watch (60 s)
`--perf-report`: ok, no challenge. 285 forced layouts, 32 300 ms, max 270 ms. Callbacks (raw CPU total / max):
timer 1 347 x, 24 198 / **5 519 ms**; promise_job 2 105 x, 5 283 / **2 823 ms**; observer_delivery 142 x,
5 335 / 291; classic_script 58 x, 1 818 / 1 553. RSS **714 MB**, JS heap 124 MB. js_errors 2.
The JSON names no callback: which timer took 5.5 s is not knowable from the harness (-> QOBS4).

Inclusive share of all samples (main thread 96.8 %, V8 worker threads 3.1 %):
| frame | % |
|---|---|
| `GeomCache::refresh` / `rect_of` | 87.3 |
| `build_layout_tree_with_resources` | 83.9 |
| `precompute_generated` | 63.8 |
| `matching_declarations_for` | 62.6 |
| **`El::has_class`** | **29.3** |
| `build_children` / `resolve_style_*` | 18.0 / 17.6 |
| obsFlush (IO) | 13.8 |
| malloc/free/realloc | 6.6 |
| **`css::intern_custom_props`** (from `compute_custom_props`) | **5.3** (BTreeMap clone 3.4, drop 2.2, SipHash of every key+value string ~1.6) |
| `js_style` computed style | 4.8 |
| V8 compile/parse | 4.6 |
| `RuleIndex`/`HoverKeys` (the `RULE_INDEX` thread-local borrow inside `matching_indices_for`) | 3.1 |
| `js_style::page_css` (QY53h) | 1.5 |
| `measure_document` | 1.6 |
| `Declarations::clone` | 1.0 |
Who forces the layouts (nearest page JS frames above `GeomCache::refresh`): `getPlayerSize` (via `Gl`,
`Va`) 43 %, `cV` > `b6a` (`offsetX` via `gOffset`) 13.6 %, scratchy's IO (`ioEval` > `gViewRect`) 10.6 %,
`numberOfGhostCommentsChanged` 2.3 %: the same ranking as R43 section 2, from one perf run.
`intern_custom_props` clones the parent's whole custom-property `BTreeMap<String,String>` plus the element's own
declarations, hashes the complete map to look it up, and `Box::leak`s every distinct map: memory that is
never freed, for the life of the process, across navigations. YouTube defines hundreds of `--yt-*` properties.

### 4. What the existing tools cannot show (the visibility gaps)
1. **No profiler recipe** in the repo: R43 wrote a SIGPROF sampler and reverted it; every investigator has to
   rebuild that. Section 1 does more with no code (QOBS1). The V8 flag crash (QOBS2) blocks the obvious
   recipe.
2. **Forced layouts are a count and a total** in `--perf-report` (`GeomCache::refresh` keeps `forced`,
   `forced_ms`). The per-phase split, the no-op rate (80 % on YouTube, R43) and the JS caller all needed
   throwaway code in R43 (QOBS3).
3. **Callbacks are aggregated per kind.** A 5.5 s timer and a 2.8 s promise job on YouTube have no name,
   script or line (QOBS4).
4. **The window is not measured at all.** `App::redraw` records only `last_build_time`; there is no count or
   time for redraws, shape-worker results, paint, present, video frames shown, or input-to-paint latency.
   QM1k1n6 (redraw starvation), QM1k-mouse (video freezes while the mouse moves), QY53e2/QY53e3 (App layout
   vs forced layout) and QM1k1n5 (network results 1-19 s late) each prescribe "temporary `Instant` stamps"
   (QOBS5, QOBS6).
5. **Memory:** `rss_bytes` at the end and the JS heap only. YouTube: 714 MB RSS vs 124 MB heap; ~590 MB native
   with no breakdown (layout trees, glyph maps, `CUSTOM_PROPS_INTERNER` leaks, `SHAPE_MEMO`, image/SVG caches,
   MSE store, libmpv). No peak RSS (QOBS7).
6. **Comparing runs is manual.** `perf/<engine>-<hash>.json` exists but nothing diffs two of them (QOBS8);
   no offline, deterministic forced-layout benchmark exists (the big-css fixture lives only inside tests;
   `benches/layout.rs` covers tree build + shaping of a synthetic page) (QOBS10).
7. **The baseline is from the old machine** (QOBS9).

## R43: Where forced layout goes (QY53): on the YouTube watch page 88 % of it is the player's `getPlayerSize` reading `#movie_player.clientWidth`, 80 % of those layouts change no box at all, and 99.5 % of each one is STYLE and BOX BUILD (layout proper 0.4 %); half of that is one O(elements x rules) scan for `::first-line`/`::first-letter` rules. On Wikipedia it is scratchy's OWN IntersectionObserver evaluation (Opus investigator QY53, 2026-11-19) — OPEN: QY53a-QY53h (a-d DONE; QY53e designed in section 7, PARTIAL, split into QY53e1-e3)

**Status: investigation only, docs only.** Main b8d865f, own worktree release build (V8 + media,
RUSTY_V8_ARCHIVE = existing librusty_v8.a, `nice -n 19 -j2`). Own wire on a scratchpad socket.
Headless `scratchy --perf-report --budget-ms 20000 --seconds 60|30` (the permanent harness,
"true cost" mode). YouTube with `--user-agent 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36
(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'` (user-approved), Wikipedia with the default UA
and with that UA (no difference). No challenge or consent wall on any run.
**Temporary instrumentation (never committed, reverted with `git checkout`):** one stderr line per
forced layout from `GeomCache::refresh`: wall time split into CSS collect, `RuleIndex`/`HoverKeys`
build, `precompute_generated`, box build (`build_children`) with the cascade (`resolve_style_for`),
`resolve_background_image` and inline-SVG rasterizing inside it, `measure_document`, glyph walk;
number of style resolves and DOM elements; selector-matching calls and time per pseudo and phase and
the mean number of candidate rules per call; per-tag exclusive box-build time; the rects compared with
the previous forced layout (changed / added / removed); every DOM write since the previous layout by
kind, whether the target was connected to the page, whether it was a no-op (same value); the perf
span kind; and the JS stack of the read that forced it (`new Error().stack`, 14 frames, captured in
`gRect` when `__d_wgen()` moved). Plus a leaf-IP sampling profiler (SIGPROF every 0.5 ms CPU,
recording only while a forced layout runs; symbolized with `nm`), and env toggles for three
throwaway fixes (FIX1/FIX2/FIX4 below) so the same binary measured before and after.
Caveat: YouTube runs differ run to run (170-313 forced layouts per 60 s): when the main thread gets
faster the page gets further (more DOM, more layouts). So fixes are compared PER STYLED ELEMENT
(cost of a forced layout divided by the number of style resolves in it), not by totals.

### 1. Totals
| page (60 s unless noted) | forced layouts | forced-layout CPU | of raw callback CPU | max | mean |
|---|---|---|---|---|---|
| YouTube watch G1AeBgalFZk, 5 baseline runs | 170 / 249 / 313 / 215 / 203 (30 s) | 68.7 / 74.9 / 120.8 / 100.4 / 55.7 s | 95 % (100.4 of 106.0 s, run A) | 0.70-1.42 s | 274-467 ms |
| en.wikipedia Rust_(programming_language), 30 s | 8 | 18.8-19.3 s | (R37/QV8-m: 93-94 %) | 2.65-2.70 s | 2.36-2.42 s |
| en.wikipedia Main_Page, 30 s, both UAs | **0** | 0 | 0 | - | - |
The "Wikipedia 93-94 %" of PERF.md is the Rust article (the perf-suite page); Main_Page never
forces a layout. Every forced layout was preceded by at least one DOM write (no viewport or CSS
change triggered one, except the first). Reads DO reuse one layout until the next write: 5.4
geometry reads per forced layout on YouTube (1164 reads, 215 layouts), 8.5 on Wikipedia.

### 2. Who forces them (JS stack of the read)
YouTube run A (215 layouts, 101 s):
| read | page caller | n | wall |
|---|---|---|---|
| `#movie_player.clientWidth` (`gRect<gClient<clientWidth`) | `Rtb.getPlayerSize` (player base.js), under the player's `publish`/`cueVideoByPlayerVars`/`Ctb.init` event pipeline | 187 | 87.1 s (86 %) |
| `div.offsetLeft` | `b6a` (base.js) | 10 | 2.8 s |
| `IntersectionObserver` targets | scratchy's own `obsFlush` -> `ioEval` -> `gViewRect` | 7 | 3.8 s |
| masthead/menu `clientWidth`, `offsetWidth`, `scrollHeight` | ytmainappweb `onStamperFinished`, `isOverflowing`, `updatePrimaryMenuWidth`, a ResizeObserver callback, `yt-desktop-body-width.js` | 11 | 7.3 s |
By perf span: timers 95, classic scripts 64, promise jobs 46, observer deliveries 10. Other runs:
same shape (`getPlayerSize` 118/170, 206/249, 238/301 layouts).
Wikipedia Rust: 7 of 8 forced layouts (16.9 of 18.9 s) are scratchy's own IntersectionObserver
evaluation (`obsFlush` timer -> `ioEval` -> `gViewRect` of `h1#firstHeading` / `#mw-content-text`,
Vector's sticky-header observer), re-run because a timer wrote 1-3 attributes (`ioSigOf` contains
`__d_wgen()`); 1 is `scrollToActiveSection` reading `a.offsetParent` (2.0 s).
DOM reads that force: `clientWidth` 193, `offsetLeft` 10, `getBoundingClientRect` via IO 7-12,
`offsetWidth` 3, `offsetParent` 1, `scrollHeight` 1. `getComputedStyle` never forces a layout
(it is style only, QY52) and `scrollTop` reads a cached value.

### 3. Dirty state before each forced layout
| | YouTube A (215) | Wikipedia (8) |
|---|---|---|
| writes since the previous layout: 1 / 2-4 / >= 5 | 61 / 26 / 128 | 2 / 3 / 3 |
| layouts after which NO element box moved, appeared or vanished | **171 (81.1 s, 80 %)** | 3 (7.3 s) |
| ... at most 5 boxes differ | 189 (88.9 s) | 5 (12.0 s) |
| would be a cache hit if only CONNECTED, VALUE-CHANGING writes invalidated | **73 (35.3 s, 34 %)**; other runs 88/301, 106/313 | 0 |
| ... plus ignoring attributes no selector mentions | 74 (35.7 s): no extra gain | 0 |
Most common YouTube trigger: ONE `setAttribute`/style write of the value the attribute already had
(46 of the redundant `getPlayerSize` layouts), then 7 `style` writes that changed no box (11), then
writes into detached subtrees (insert/setAttribute on a fragment before it is attached).

### 4. What one forced layout costs (it is style, not layout)
Per forced layout, share of wall time:
| segment | YouTube A | Wikipedia Rust |
|---|---|---|
| collect `<style>` CSS + rules cache check | 1.9 % | 0.8 % |
| `RuleIndex::build` + `HoverKeys::build` (rebuilt every time, same rules) | 8.5 % (40 ms) | 0.2 % |
| `precompute_generated` (a SECOND full selector-matching pass over the tree, for counters/quotes/`content`) | 17.5 % | 31.7 % (750 ms) |
| box build (`build_children`) | 71.2 % | 53.7 % |
| - of which cascade (`resolve_style_for`, incl. matching) | 14.2 % | 33.0 % |
| - of which `resolve_background_image` (a third matching per element) | 2.6 % | 3.6 % |
| - of which inline SVG rasterizing (5-7 per layout) | 0.3 % | 0 |
| `measure_document` (layout proper + shaping) | **0.4 %** | 13.0 % (300 ms) |
| glyph walk (QJ7) | 0 | 0.5 % |
Sampling profile (YouTube A, 28 768 samples inside forced layouts): 44 % in
`build_children_with_prefix` itself = the inlined `apply_first_line` / `apply_first_letter`, which
EACH scan EVERY selector of EVERY rule (`rules.iter().any(...)`) for EVERY element to learn whether
any `::first-line`/`::first-letter` rule exists: O(elements x rules) with YouTube's 3.8 MB of CSS;
14 % `matching_declarations_for`, 10 % `str::SplitWhitespace` (`El::has_class` splits the class
attribute on every check), 11 % libc (memcpy/alloc), 5 % `selectors` matching.
Matching: YouTube tests **418 candidate rules per element** on average (the rule index narrows
little), Wikipedia 83; ~50 µs per matching call. Wikipedia per layout: 11 413 style resolves for
9 947 elements; matching 634 ms in the box build + 452 ms duplicated in `precompute_generated` +
~460 ms for `::before`/`::after` (8.7k calls each, done in BOTH passes).
`page_css()` clones the 3.8 MB external CSS on every geometry read, hit or miss: 0.77 s per 60 s
on YouTube (0.8 %).

### 5. Throwaway fixes, measured (env toggles in the instrumented binary; reverted)
Cost per forced layout divided by style resolves in it (µs per styled element), YouTube 60 s:
| build | runs | total | idx | pre | box build | cascade |
|---|---|---|---|---|---|---|
| main (baseline) | yt2, yt3, A | **830-868** | 74-111 | 152-190 | 535-619 | 107-124 |
| FIX1: first-line/first-letter presence computed once per build | B | **490 (-44 %)** | 102 | 183 | 179 | 106 |
| FIX1 + FIX2: `RuleIndex`/`HoverKeys` kept while the rules `Rc` is the same | C, E | **366-399 (-55-58 %)** | 3-7 | 144-201 | 167-206 | 104-119 |
| FIX1 + FIX2 + FIX4: geometry key = connected value-changing writes only | D | 376 | 2 | 141 | 221 | 123 |
Wikipedia Rust (8 layouts, 11.4k resolves): main 212 µs/element (2.42 s per layout); FIX1+FIX2 198
(-7 %, 2.26 s); +FIX4 191 (7 layouts instead of 8, 2.18 s). FIX4 cannot be compared by count on
YouTube (the faster runs reach a bigger page); its counterfactual in section 3 (34 % of layouts
would be hits) is the number to use. With FIX1+FIX2 the profile is: matching 25 %, `has_class`
whitespace split 24 %, libc 16 %, `selectors` 11 %; `precompute_generated` is 39-50 % of what is left.

### 6. Ranked plan (measured upper bounds; YouTube watch = 101 s of forced layout per 60 s, run A)
| # | change | item | measured gain | effort |
|---|---|---|---|---|
| 1 | know once per build whether `::first-line`/`::first-letter` rules exist | QY53a | YouTube -44 % per element (measured, FIX1); Wikipedia ~-5 %; also every normal App relayout on big-CSS pages | S |
| 2 | keep `RuleIndex` + `HoverKeys` while the rule list is the same `Rc` | QY53b | YouTube a further -25 % (-55-58 % with 1, measured); ~0 on Wikipedia | S |
| 3 | geometry generation bumps only on connected, value-changing writes (no-op attribute/text/style writes, writes inside detached nodes skipped); IO signature uses it too | QY53c | upper bound 34 % of YouTube forced layouts (35 s of 101 s); 0-1 of 8 on Wikipedia | S |
| 4 | one matching per (element, pseudo) per build: `precompute_generated` and `resolve_background_image` reuse the box build's matches | QY53d | upper bound: Wikipedia 31.7 % + 3.6 %; YouTube 39-50 % of what is left after 1+2 | M |
| 5 | IntersectionObserver evaluation without its own forced layout | QY53e | Wikipedia up to 90 % (16.9 of 18.9 s); YouTube 4-9 % | M (Opus design first) |
| 6 | faster matching: allocation-free ASCII class check, per-build class/id cache, ancestor bloom filter, why 418 candidates on YouTube | QY53g | upper bound: matching + `has_class` ~50 % of what is left after 1+2 (profile) | M |
| 7 | incremental style: reuse `ComputedStyle`s of clean elements between forced layouts | QY53f | upper bound: 80 % of YouTube layouts changed no box (81 s); cascade+matching+pre are 70-85 % of a layout | L (Opus design) |
| 8 | `page_css()` without cloning 3.8 MB per read | QY53h | 0.8 % on YouTube | S |
Not worth doing first: incremental LAYOUT / dirty-subtree measure: layout proper is 0.4 % of a
YouTube forced layout and 13 % on Wikipedia; nothing to win until style is cheap.
Rough projection for YouTube (multiplying measured or upper-bound factors, NOT measured together):
1+2 (-57 %), 3 (-34 % of layouts), 4 (up to -40 % of the rest): 101 s -> ~17 s of forced layout per
60 s. That would still leave ~30 % of the main thread busy; 7 is what removes the rest.

### 7. QY53e design: IntersectionObserver without its own forced layout (Opus, 2026-11-19, 064eca5, PARTIAL)
**Partial:** the coordinator drained the machine before the live-window (nested dwl) runs, so the
App-vs-GeomCache duplication is still not measured (QY53e3). Headless numbers below are measured.
Main 9473913 (after QY53d), own worktree release build, own wire on a scratchpad socket, R43 mode
(`--perf-report --budget-ms 20000`, Wikipedia 30 s default UA, YouTube 60 s Chrome 131 UA). No
challenge or consent wall. **Throwaway instrumentation (reverted with `git checkout`):** one stderr
line per GeomCache miss (time, `geom_gen`, viewport, wall/CPU ms, whether `ioEval` was on the stack,
CSS length); one per `ioRecheck` and `ioEval` (and whether it queued an entry); one when a script
read later hit a layout IO had forced; one per App tree build (`geom_gen` at build, CPU ms) and per
complete `ShapeResult` (`geom_gen` at build vs now). The App lines are for QY53e3 (not run).

**How it works today (code reading).** `__scratchy_timers_run_one` calls `__scratchy_ioTick` before
and after EVERY timer; when `ioSigOf()` (geometry generation, viewport, window scroll, `gScrollGen`)
differs, `ioRecheck` (a 0 ms timer) marks every active observer `need` and `obsFlush` runs `ioEval`,
whose `gViewRect` -> `gRect` -> `__d_rect` -> `GeomCache::refresh` lays the whole page out when the
key moved. So any connected value-changing write followed by a timer turn = one full forced layout,
whether or not any observed box could have moved.

**Measured (headless, after QY53d):**
| page | runs | forced layouts | of them IO | IO CPU | IO evaluations that queued an entry |
|---|---|---|---|---|---|
| Wikipedia Rust 30 s | 3 | 8 / 8 / 8 (9.8 / 9.5 / 9.4 s, mean 1.17-1.23 s each) | 7 / 7 / 7 | 8.7 / 8.4 / 8.3 s (88-89 %) | run 3: 1 of 7 IO layouts (the first, initial `observe`); the 6 rechecks (~7 s) queued nothing |
| YouTube watch 60 s | 3 (3rd cut at ~60 s) | 142 / 217 / 194 (24.7 / 37.1 / 21.4 s; mean 110-174 ms) | 24 / 13 / 7 | 9.0 / 4.2 / 1.1 s (36 / 11 / 5 %) | run 3: 7 of 7; 4 of those 7 generations were read again by page script later (the layout was not wasted) |
Wikipedia's shape: after the load, a page timer writes 1-5 attributes about every 1.2 s for ~10 s;
each write -> `ioRecheck` -> a 1.1-1.3 s layout -> the next write lands during it -> again, so IO is
busy back to back and never changes its answer. YouTube's IO layouts come in bursts (every ~0.5 s for
10 s in run 1) and do change answers. Per-layout cost after QY53a-d: YouTube 110-174 ms mean (R43
main: 274-467), Wikipedia 1.17-1.23 s (R43: 2.36-2.42 s).

**Design.**
1. Deferred, rate-limited rechecks (QY53e1, headless and window). Only rechecks triggered by a
   geometry-generation change are deferred; `observe()`, `takeRecords()`, window/element scroll and
   viewport changes keep the immediate path. If GeomCache is already fresh (a script read laid out
   this generation), evaluate now for free. Otherwise evaluate at the first of: the end of a task
   after which the geometry became fresh, or a deadline `max(100 ms, 4 x last forced-layout CPU)`
   capped at 2 s, set when the first deferral starts and not moved by later writes. That bounds IO
   to ~20 % of the main thread and folds Wikipedia's 6 empty rechecks per 10 s into ~2. The
   freshness check must be cheap: `GeomCache::refresh` clones `page_css()` per call today, so QY53h
   (key on CSS version counters) comes first.
2. Use the App's layout in a window (QY53e2). The App lays out the same DOM on the same thread after
   every write batch (`js_sync` -> `request_js_restyle`, build in `App::redraw`, measure on the
   shaping worker, complete `ShapeResult` back on the main thread). On a complete, current
   `ShapeResult`, hand its `Laid` to GeomCache as a separate read-only `rendered` rect map (`walk()`,
   tagged with `geom_gen()` at build time) and run pending IO evaluations against it. That is what a
   browser does: IO runs in "update the rendering", after the frame's own layout. Staleness: at most
   one App layout behind (spec: IO has no timing guarantee). A target missing from `rendered` while
   the generation moved stays pending (inserted after the build). Script reads are NOT served from
   `rendered` (the App sizes images, GeomCache does not: two geometries; QY53e3 decides).
3. Headless: there is no App, so only design point 1 applies (unchanged behaviour otherwise).
**Expected gain:** Wikipedia Rust headless: IO layouts 7 -> ~2-3, forced-layout CPU about -50-60 %
(~5 of 9.5 s per 30 s); with 2 in a window, IO layouts -> 0-1. YouTube: up to the IO share (5-36 %
of forced-layout CPU) in a window, less headless because about half of YouTube's IO generations
are read by script anyway. **Risk:** observers (lazy load, infinite scroll, sticky headers) fire up to
the deadline later after DOM-only changes (scroll stays immediate); a wrong "not intersecting" entry
for targets inserted after the App build if the pending rule is missed (tested in QY53e2).

**QY53g / QY53f after QY53d:** still the right next items, re-ranked. Wikipedia's forced-layout time
is now 88 % IO, so QY53e1 (S) is the cheapest big win there; QY53h moves up because e1 needs its
cheap key. YouTube's forced layouts are 64-95 % script reads (`getPlayerSize`, R43 section 2) that
no IO change touches, and each still costs 110-174 ms of style: QY53g (faster matching) is the
lever for those and for every remaining IO layout, then QY53e2, then QY53f (L). Not re-profiled in
this run (the QY53g profile shares are R43 section 5, before QY53d). New order: QY53h -> QY53e1 ->
QY53g -> QY53e2 -> QY53f; QY53e3 before QY53e2 merges.

### Not established
Whether the App's own (main-thread, per-turn) relayout could serve geometry reads or IO directly
(headless `--perf-report` has no App layout, so the duplication with the window's layout was not
measured); the live-window effect on R42's playback timeline (no nested-dwl run); YouTube results
and home pages (R37/R38 numbers not re-taken); why YouTube's rule index yields 418 candidates (the
size of the universal bucket was not logged); whether `getPlayerSize`'s preceding writes are the
page's own or our prelude's (e.g. `styleSet` rewriting the whole `style` attribute with the same
text); correctness of FIX4 beyond the measured runs (no tests); whether QY52's StyleCache could be
reused by the box build (item 7).

## R42: YouTube through the MSE stack (QM1k-m2): the player script runs, picks avc1 + mp4a, attaches, appends and plays ~60 s of AUDIO, but NO PICTURE because YouTube adds the AUDIO SourceBuffer first and scratchy always makes SourceBuffer 0 the main (video) stream; with that swapped (throwaway patch) 640x360 video plays to the PoToken stop at ~60 s, then YouTube's own error card. The main thread is busy 100% of the time (294 forced layouts = 116 s of CPU in 60 s), so everything arrives 10-45 s late (Opus investigator QM1k-m2, 2026-11-19) — OPEN: QM1k1k (P1), QM1k1l, QM1k1m, QM1k-u, QY53 (now also blocks YouTube playback; measured in R43, fix items QY53a-QY53h); QM1k1n DONE 2026-11-19 (section 5: YouTube parks the `<video>` at `top:-360px` until `playing`, page design, not a CSS gap): QM1k1n1 DONE, QM1k1n3; QM1k1n2 DONE 2026-11-19 (section 6: start 3-6x faster after QY53a-c, but playback freezes at YouTube's first quality switch, a second init segment reloads into an empty generation): QM1k1n4 (P1), QM1k1n5 (P1, fetch results reach JS 1-19 s late), QM1k1n6, QM1k1n7

**Status: investigation only, docs only.** Main 5bf4e03 (QM1k1e merged), own worktree release build
(`nice -n 19 cargo build --release -j2`, RUSTY_V8_ARCHIVE = the existing local librusty_v8.a, 15 min),
V8 + media, libmpv 0.41 / ffmpeg 8.1.2. Own wire instance (`wire --socket <scratchpad>/wire.sock`,
`WIRE_TIMING=1`), never the user's. Nested dwl (tools/nest.sh, 1052x1050), grim, tools/inject.
`--user-agent 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko)
Chrome/131.0.0.0 Safari/537.36'` (user-approved for this test), `SCRATCHY_MEDIA_AO=null` (nothing
audible; audio is decoded, see below), `SCRATCHY_MPV_LOG_FILE`, `SCRATCHY_TIMING=1`. URL:
https://www.youtube.com/watch?v=G1AeBgalFZk (C64 "RoboCop 3" oscilloscope view, 3:59, logged out, DE).
**Instrumented binaries (throwaway, reverted, never committed):** (1) a logging wrapper appended to
`mse_prelude.js` (lines `MSELOG +<s since prelude>`: every `isTypeSupported` + answer,
`addSourceBuffer`, `appendBuffer` count/size/first box, `remove`/`abort`/`changeType`/
`endOfStream`/duration/`timestampOffset`/`mode` sets, element events, a 2 s TICK with currentTime/
readyState/paused/buffered/videoWidth, and for googlevideo `fetch`es: status, bytes, and a UMP part
count + STREAM_PROTECTION_STATUS (part 58) payload); (2) for run 2 also a 3-line change in
`MpvBackend::load_mse` that, when `QM2_SWAP` is set, loads SourceBuffer 1 as the main file and
SourceBuffer 0 as the external audio file. Nothing else differs from main.

### Runs
| run | binary | what happened |
|---|---|---|
| 1 | main + logging | audio-only playback 0-30 s, black/thumbnail picture, then `waiting` forever (stall, see 2) |
| 2 | main + logging + `QM2_SWAP=1` | 640x360 h264 video + AAC played 0-~64 s, PoToken stop, YouTube error card |
| headless | main + logging, `--perf-report --seconds 60` (same UA, own wire) | outcome ok, 0 kills, 0 JS errors; forced layout 294x, 115 922 ms total, max 1254 ms; raw CPU in callbacks 121 s (classic 24 s, timers 61 s, promise jobs 36 s) in 60 s simulated |
| local A | main + logging | the QM1k1e page (`src/tests/mse_tests/qm1k1e_page.html`, video SourceBuffer first) with a 10 s testsrc2+sine fixture from 127.0.0.1 (python http.server): plays, picture updates (screenshots show the burnt-in clock 4.64 s at 5 s), `ended` at 10.02. No regression of QM1k1e in a live window |
| local B | main + logging | the same page with the two `addSourceBuffer` lines swapped (audio first): mpv `No audio streams in file scratchy-mse://1/1/1` / `Can not open external file`, black 320x240 box, `videoWidth 0`, audio plays to `ended` 10.02. SAME BUG as YouTube, reproduced locally |
| local B + swap | `QM2_SWAP=1` | picture plays (clock 5.76 at 6 s), `vw=320x240`, mpv `Video --vid=1 (h264 320x240)` + `Audio '0' [external]` |
| Reddit | main + logging | r/oddlysatisfying: `?js_challenge=1&jsc_token=...` redirect, then "You've been blocked by network security." HARD STOP: not retried, nothing measured (R34 had the feed playing on 2026-11-17 through the user's wire) |

### 1. The MSE calls YouTube made (run 1 and 2 the same; full list in the logs)
- `isTypeSupported`: ~60 calls. Answered **true**: `video/mp4; codecs="avc1.42001E"` (+`decode-to-
  texture=true`), `avc1.4d401e` with each VALID probe (`width=640`, `height=360`, `framerate=30`,
  `bitrate=300000`, `eotf=bt709`, `cryptoblockformat=subsample`, `decode-to-texture=false`,
  `experimental=allowed`, `tunnelmode=true|false`), `avc1.4D401F; width=1280; height=720;
  framerate=30; bitrate=2317914`, `audio/mp4; codecs="mp4a.40.2"` (+`channels=2`). **false**: every
  INVALID twin (`width=99999`, `height=99999`, `framerate=9999`, `bitrate=2000000000`,
  `eotf=catavision`, `channels=99`, `cryptoblockformat=invalidformat`, `decode-to-texture=nope`,
  `experimental=invalid`), every `video/webm vp9`, `vp09.02.51...`, `av01.*`, `audio/webm opus`.
  So QM1k4's answers steer YouTube exactly as planned.
- `addSourceBuffer("audio/mp4; codecs=\"mp4a.40.2\"")` FIRST, then `addSourceBuffer("video/mp4;
  codecs=\"avc1.4D401E\"")`; `duration = 239`, later `238.9797732426304`. Never called in either run:
  `remove`, `abort`, `changeType`, `endOfStream`, `timestampOffset`/`mode`/`appendWindow*` sets.
- Appends (run 2): init 723 B (audio) / 741 B (video), both `ftyp`; then `moof` segments: video
  ~190-720 KB each (5.33 s), audio ~161 KB each (10 s); video #13 = 4.39 MB total, audio #8 = 1.13 MB
  total, `buffered` grew to `[0.00-64.00]`. No append threw; no `error` event on a SourceBuffer.
- SABR (run 2): POST `rrN---sn-....googlevideo.com/videoplayback`, `application/vnd.yt-ump`, 200.
  Media responses 1 498 125 B, 2 174 150 B, 1 851 251 B with part 58 payload `08 02` (status 2 =
  attestation pending); then from +195.8 s every response is 109 B, parts {35, 47, 58}, part 58 =
  `08 03 10 0a` (status 3 = attestation required, max retries 10), 10 such responses ~2 s apart, then
  the page emptied the element (`emptied`, ct 0) and showed **"Ein Fehler ist aufgetreten.
  Aktualisiere die Seite oder versuche es später noch einmal."** (screenshot). Same PoToken stop as R34
  (QM1k2, parked). No crash; scratchy kept running.
- 11 of the first 16 SABR fetches were rejected `AbortError` by the page (YouTube's own abort) 10-25 s
  after they were issued (one more, #5, got 105 B with parts {35, 57, 67} and no media); wire answered their headers in 0.5-1.2 s (`WIRE_TIMING`). One POST was
  `TypeError: Failed to fetch` with `js net: ... CORS: no Access-Control-Allow-Origin header` (an
  answer without ACAO; body not inspected). Most likely the aborts are the page timing out because its
  callbacks run late (section 3); not proven.

### 2. Blocker 1 (P1): the main mpv file is always SourceBuffer 0 (QM1k1k)
`MpvBackend::load_mse` loads `scratchy-mse://F/G/0` as the file and appends `.../1` to
`audio-files`. YouTube's order is audio, video, so mpv gets an audio-only main file and the VIDEO as
an "external audio file": mpv log run 1: `No audio streams in file scratchy-mse://1/1/1.` /
`Can not open external file scratchy-mse://1/1/1.`, only `Audio --aid=1 (aac 2ch 44100 Hz)`,
`video=eof`. JS saw `videoWidth 0x0` with readyState 4. The box showed YouTube's thumbnail, later
black with a pause button. Audio played 0 -> 29.95 s, then `waiting` forever (rs 2) because
`currentTime` ran past the intersected `buffered` end (21.33) while the video track was never read:
the audio-only mpv file kept going until its own data ended, and the video SourceBuffer stopped being
refilled (no further SABR media after +108 s). Swapping the two (run 2, local B + swap) fixes it.
Fix: choose the main stream by the SourceBuffer TYPE (`video/*` main, `audio/*` external), not by
index.

### 3. Blocker 2 (P1 for YouTube): the main thread is busy all the time (QY53)
- `ps`: scratchy at 98-101 % CPU on its main thread for the whole run (top -H: only the main thread
  busy; V8 workers, mpv threads idle). The 2 s TICK interval fired every 2-11 s, with gaps of 22-26 s.
- Headless `--perf-report` on the same URL (own wire, same UA, 60 s): forced layout **294 times,
  115.9 s total, max 1.25 s**; budget clock (forced layout excluded) only 2.7 s classic + 1.5 s timers
  + 1.6 s promise jobs. So ~96 % of the callback CPU is forced layout, as on YouTube results (R37/R38,
  QY53), but heavier here. `load` (DOMContentLoaded) 26 s wall / 24.6 s CPU.
- Effects measured live (run 2, seconds after the prelude started): MediaSource attached +5.2 s, but
  `addSourceBuffer` only at +16 s; first media append +118.7 s; `canplay` +128.9 s; click on the big
  play button at +139.5 s -> `play` event +159.2 s (20 s), `playing` +180.3 s (41 s). Run 1: click
  -> `play` 25-30 s. Screen white for the first ~45-50 s of both runs (window mapped, nothing painted).
- During playback (run 2) mpv logged `mpv_render_context_render() not being called or stuck` 19 times
  (~3.8 s, near the end) and once `Audio/Video desynchronisation detected!`: the frame is rendered on
  the main thread (QM1g), so a long callback freezes the picture while audio goes on. That is the only
  A/V drift symptom observed; `avsync` itself was not sampled (`ao=null`, QM1k1b finding 5).
- One CPU-budget kill in run 2: `js error: js: timer stopped: exceeded the 500 ms CPU-time callback
  budget` (which timer: not determined). None in run 1, none headless.

### 4. Other gaps seen
- **Video track off for the first ~41 s of playback (QM1k1m).** Run 2 mpv log: `vid="no"` 8.4 s after
  the load (before play), `vid="auto"` again only at mpv 77.1 s (= ~41 s after unpause at 36.0 s), then
  `VO: [libmpv] 640x360`, `first video frame after restart shown`. scratchy switches the video off
  when the element has no visible box target (QM1g, `set_video_target(None)`), although YouTube's player
  box was on screen the whole time (screenshots). Whether the `<video>` had a zero/hidden box until a
  late relayout, or the target was lost for another reason, is not determined. First real video pixels
  in a screenshot: 06:02:51, ~200 s after start (q06; a different oscilloscope frame from the
  thumbnail; frames differ between later screenshots).
- **QM1k1m measurement (2026-11-19, temporary logging, reverted):** the `<video>` was painted as a box only once (+26.1 s, 550x309, 166625 of 169950 px visible), then every later redraw reported ZERO video boxes (not "covered": covered boxes are listed), so `set_video_target(None)` and `vid=no` (as designed). The element has no painted box (display/visibility/size/clip) although YouTube's picture is on screen; cause not determined, item QM1k1n. EXPLAINED in section 5: the box exists but YouTube's inline `top: -360px` puts it entirely outside `#movie_player`'s `overflow:hidden` clip until the `playing` event. QM1k1l: fixed (the mpv backend forgot nothing on `unload`; cache/seekable events of a stopped file are ignored).
- **`buffered` after the element is emptied (QM1k1l).** Run 2, after `emptied` (ct 0, rs 0, the MSE
  detached): `buffered` = `[0.00-61.80]` on every TICK. With `self.mse` gone, `MediaCore::buffered`
  falls back to `MpvBackend::buffered`, i.e. `cache_end` (mpv demuxer-cache-time); a value left from the
  unloaded MSE file is the likely source (not verified in code paths beyond reading them).
- **Autoplay**: YouTube's unmuted autoplay did not start (expected: D-rules allow only muted or after a
  gesture); one click on the play button started it.
- **JS errors (both runs, same):** `lottie-light.js: TypeError: Cannot set properties of null (setting
  'fillStyle')` (canvas `getContext` returns null, known, QWK chain note) and `ReferenceError: lottie is
  not defined`; `js net: GET https://youtube.com/: CORS: Access-Control-Allow-Origin
  "https://www.youtube.com" does not match the page origin "null"` (a request whose origin is `null`;
  item QM1k-u); `tpc.googlesyndication.com/sodar/...js` connection refused (wire, harmless).
- No crash in any run (the R40/R41 snapshot crashes did not show).

### 5. QM1k1n (Opus investigator, 2026-11-19): why the `<video>` has no painted box until `playing`
**Answer: no CSS or layout gap in scratchy. YouTube's own player script parks the `<video>` above the
player (inline `top: -360px` = minus the player height) while the player is unstarted, cued or
paused-before-start, and moves it to `top: 0px` in its `playing` handler. Chrome gets the same
inline style; the difference is only TIMING (all three steps below are slow in scratchy) plus the
`vid=no` that QM1g applies to an off-screen box.** Not a chicken-and-egg: YouTube waits for the
`playing` EVENT, not for a frame, and scratchy fires `playing` with the video track off.

Setup: main 31acf1c, own worktree release build (V8 + media, RUSTY_V8_ARCHIVE = a local copy of the
existing librusty_v8.a), own wire (`--socket <scratchpad>/qm1k1n/wire.sock`), nested dwl 1052x1050,
grim, tools/inject, Chrome `--user-agent` (as R42), `SCRATCHY_MEDIA_AO=null`, `SCRATCHY_V8_SNAPSHOT=0`
(the JS probe sits in a prelude). Two runs of the R42 URL. **Throwaway probes (reverted, never
committed):** (1) appended to `mse_prelude.js`: every 3 s `getComputedStyle` (display, visibility,
opacity, position, width, height, left, top, overflow, transform, z-index) + `getBoundingClientRect` +
inline style of every `<video>` and all its ancestors; a MutationObserver on the video's `style` and on
`#movie_player`'s `class`; the media events; a wrapper logging each `HTMLMediaElement.play()` call; all
with wall-clock stamps. (2) Rust: `paint_inner` logged every video-placeholder box it reached
(position, clip), `media_video::paint_box` its clipped rect (`vis`), `App::redraw` walked the laid tree
for the video box and its ancestors (display, position, visibility, opacity, overflow, rect, ink,
cull), and `MpvBackend::set_video_target` logged Some/None changes, all with wall-clock stamps.

What the probes showed (run 1 and 2 the same):
- The `<video>` is ALWAYS in the laid tree and `paint` ALWAYS reaches it: `display:block`,
  `visibility:visible`, `opacity:1`, `position:absolute`, 640x360, no transform. Its inline style is
  `width: 640px; height: 360px; left: 0px; top: -360px;`, so the box is at y = 80 - 360 = -280 and
  `.html5-video-container` (`position:relative`, height 0) sits inside `#player-api` / `#movie_player`
  (`overflow:hidden`, 640x360 at 24,80). `paint_box` gets clip (24,80,640,360) and rect
  (24,-280,640,360): `clip_rect` = None, so the box is not listed (a fully clipped box is not
  "covered", it has no visible rect at all) -> `set_video_target(None)` -> `vid=no`. Exactly R42's
  "zero boxes".
- The one painted box QM1k1m saw (+26 s, 24,80 550x309) is the layout from BEFORE YouTube's first
  inline style landed (the CSS alone: `.html5-main-video{position:absolute;top:0;left:0;width:100%;
  height:100%}` in the 550 px wide `#player-wrap`). Run 2: target Some once ~32 s after the prelude
  started, None 0.29 s later (layout with the inline style). Later Some/None pairs only follow
  YouTube's own `top` writes, each one redraw-plus-relayout late (e.g. `top: 0px` written at +94.3 s,
  back to `-360px` at +107.6 s; the box target was Some from +109.1 s to +114.8 s, i.e. AFTER the
  page had parked it again).
- The code (YouTube `base.js`, player 983e9c18, `q9a`): `if (lX(a)) { ...centre the video... } else {
  p = -b.height; ...; p = new He(0, p, b.width, b.height) }` then `g.Se(video, left, top)`; `lX`
  returns false while `getAppState()===1`, the player has no video data, the player state is cued, or
  state flag 1024 is set. The page sets the same style in Chrome; it is the page's design, not a
  scratchy gap.
- YouTube moved the video to `top: 0px` in the SAME millisecond as the `playing` event (run 2:
  `playing` 1795075329.22, VSTYLE `top: 0px` 329.23). Run 1: `top: -360px` at the `play` event, `top:
  0px` at the first probe after `playing`. Run 2 also: YouTube's two early `play()` calls (before any
  click; scratchy rejects unmuted autoplay, D5) each set `top: 0px` and back to `-360px` 10-13 s later,
  followed by `emptied` + `loadstart` (the source was reloaded twice); `canplay` came at +156 s
  instead of +65 s in run 1. Whether Chrome with a fresh profile does the same: not checked (QM1k1n3).

Timeline of run 2 (wall seconds after the click on the big play button; mpv clock anchored at the
`set_video_target(Some)` = `vid="auto"` moment):
| step | at | gap |
|---|---|---|
| click (tools/inject) | 0 | |
| page `play()` called (click handler ran) | +12.6 | 12.6 s: main thread busy |
| mpv `pause="no"` | +12.6 | same moment |
| `play` event in JS | +18.4 | 5.8 s |
| `playing` event in JS, `top: 0px` written | +37.8 | 19.5 s after `play`, although media_el.rs fires `play` and `playing` in the same call (`start_playing` + `maybe_playing`); the page's other tasks run in between |
| next redraw (still the OLD laid tree, box at -280) | +51.1 | no redraw at all between +15.3 and +51.1 (36 s) |
| redraw with the new layout: box at 24,80, `set_video_target(Some)`, mpv `vid="auto"`, refresh seek to 29.95 | +61.3 | 23.4 s after the style write |
| mpv `first video frame after restart shown` | +76.9 | 15.6 s: NOT the re-enable cost; audio had run dry at media 29.95 (`Audio device underrun`, SABR not refilled) and mpv waited for data until EOF |
Run 1 (same shape, no wall clock on all steps): mpv `pause="no"` -> `vid="auto"` 49.6 s (media position
49.56 when the track came back), `vid="auto"` -> first frame **1.1 s** (data was there). So the
re-enable itself costs ~1 s and a refresh seek; the 40-60 s are the main thread (QY53 forced layouts:
the `playing` task waits behind the page's other tasks, and no redraw/relayout runs for tens of
seconds) plus the second layout pass needed after the style write.

Also seen: `getComputedStyle` reports `width=1%` / `height=1%` for `#movie_player` (`width:100%`) and
`#player` (QY58, already filed; YouTube sizes the player from `clientWidth`/`clientHeight`, so no
effect here). Run 2 ended in YouTube's error card ("Ein Fehler ist aufgetreten...", screenshot), the
known PoToken stop (QM1k2, parked). Screenshots: run 1 34 s after the click still the thumbnail with
the big red play button (audio playing, video parked off-screen); 77 s after the click the
oscilloscope picture with the pause button and the progress dot moved.

Items: QM1k1n1 (keep the video track decoding while a playing element's box is laid but clipped
away), QM1k1n2 (re-measure this timeline after QY53a-c), QM1k1n3 (YouTube's pre-click `play()` +
reloads vs Chrome). The rest is QY53.

**QM1k1n1 (2026-11-19, fixed): a PLAYING element whose box is laid out keeps decoding.** `paint_box` now records a laid box with nothing visible; `App::redraw` passes those nodes to the media host (`media_set_laid`); `MediaCore::keep_decoding` = laid && !paused; the mpv backend then leaves `vid=auto` (frames are consumed with `skip-rendering`, nothing is painted). Paused elements and elements with no box (display:none, disconnected) still get `vid=no`. Measured with real mpv (320x240, 25 fps, test fixture, 3 s hidden, process CPU incl. the test pump): keep 0.48 s (about 16 % of one core), `vid=no` 0.03 s. Box brought into view to first rendered frame: 0.040 s with keep, 0.086-0.106 s with `vid=no` (the refresh seek; on YouTube run 1 it was ~1.1 s with data buffered). Not measured: mpv's `render not being called` warnings (the backend does not read mpv's log; frames are consumed each poll), a 640x360 H.264 cost, the R42 URL itself (needs QY53 to get the main thread free: QM1k1n2). Nested-dwl screenshots of a local page (video `top:-360px` in a 640x360 `overflow:hidden` box, moved to `top:0` 6 s after `playing`): at 6 s only the orange container; 8 s later the picture with the burnt-in clock at 12.76 s (current, not stale).

Not established: what Chrome does with an off-screen, clipped, playing `<video>` (keeps decoding or
not; reasoning only); the cost in mpv CPU of decoding while off-screen (QM1k1n1 measures it); why
`playing` waits 19.5 s after `play` in detail (which page tasks run in between: not profiled, QY53's
domain); which of `lX`'s conditions flips first (player state only seen through `#movie_player`'s
classes, `unstarted-mode` / `paused-mode` / `playing-mode`); audio/A-V sync (ao=null).

### 6. QM1k1n2 (Opus investigator, 2026-11-19, commit 2163172): the timeline re-measured after QY53a+b+c and QM1k1n1
**Answer: everything up to `playing` is now 3-6x faster (click -> `playing` 6.9 s instead of 37.8 s
when data is buffered; box on screen 3.1 s after the style write instead of 23.4 s; page painted by
~28 s instead of 45-50 s), but no run played more than 18.5 s of video. The NEXT blocker is a
scratchy MSE bug: when YouTube appends a SECOND init segment (its ABR quality switch), the element
reloads mpv at `currentTime` with a generation that contains no segment (QM1k1n4, P1): mpv's demuxer
open waits forever and the picture freezes (run 3 at 18.47 s, run 2 before the click). Second:
page network results reach JS 2-19 s after wire finished them, so YouTube aborts most SABR fetches and
throws away 1-3.6 MB downloads (QM1k1n5, P1). Third: the main thread is still 65-98 % busy with forced
layouts (~300 ms each, ~3 per second) and the App redraws only every ~5 s during playback, so the
video can show at most a frame per redraw (QM1k1n6; QY53d/e/g/f).** The PoToken stop (QM1k2, parked)
still ends every run (status 3 at +92 s / +136 s after the start, earlier than R42's +196 s).

Setup: main b236dc1 (QY53a+b+c PR #317, QM1k1n1 93b7bc0, QM1k1k 97c9c77, QM1k1i), own worktree release
build (V8 + media, RUSTY_V8_ARCHIVE = local librusty_v8 copy, `nice -n 19 -j2`), own wire on a
scratchpad socket (`WIRE_TIMING=1`), nested dwl (tools/nest.sh, 1052x1050) + grim + tools/inject,
Chrome `--user-agent` (as R42), `SCRATCHY_MEDIA_AO=null`, `SCRATCHY_V8_SNAPSHOT=0`,
`SCRATCHY_MPV_LOG_FILE`. Same URL. No challenge or consent wall. **Throwaway probes (reverted, never
committed), all with wall-clock stamps:** JS (appended to `mse_prelude.js`): `play()` calls, media
events, `style` mutations of the `<video>`, `addSourceBuffer`, `appendBuffer` (#1-3 and every 10th),
a 2 s tick (currentTime, readyState, paused, `top`, videoWidth), and for googlevideo `fetch`es the
status, size, UMP part counts and part 58 (STREAM_PROTECTION_STATUS) payload. Rust: one line per
forced layout (`pause_budget_since`, CPU ms), per `App::redraw` (rebuild or not), per App tree build
(ms), per shaped layout installed, `set_video_target` Some/None, `set_video_keep`, mpv `vid`/`pause`
sets, each rendered video frame, and for page requests enqueue / start / wire done (status, bytes) /
abort / delivery to JS.
Runs: headless `--perf-report --budget-ms 20000 --seconds 60` x2 (R43 mode); live run 1 and 2: click
on the big play button after `canplay` (as R42 section 5); live run 3: click on the small play button
as soon as the player showed (+35.6 s, before any media data), to get playback before the PoToken stop.

**Before / after**
| measure | before (R42 sections 3+5, R43) | after (this run) |
|---|---|---|
| page painted (screenshot) | white for the first 45-50 s | painted at 28 s (run 1), 29 s (run 3); not bracketed tighter |
| headless DOMContentLoaded | 26 s wall / 24.6 s CPU | 8.0 / 8.3 s wall, 5.9 / 6.1 s CPU |
| headless forced layouts per 60 s | 294, 115.9 s, max 1.25 s (R42); 170-313, 55.7-120.8 s, mean 274-467 ms (R43) | 143, 43.9 s, mean 307 ms, median 136 ms, max 1.29 s; 241, 70.5 s, mean 293, median 395, max 0.83 s |
| live forced layouts, first 60 s | not measured | 179 / 38.3 s (mean 214 ms), 158 / 44.6 s (282 ms), 182 / 37.4 s (205 ms); while playing ~300 ms each, ~3 per second back to back |
| main thread busy with forced layout (share of each 10 s window) | ~100 % (ps, whole run) | 40-102 %, mostly 65-98 % |
| App's own tree build per relayout (`build_layout_tree_with_resources`, main thread) | not measured | mean 371-657 ms per run (37-44 builds per run) |
| `addSourceBuffer` after the start | +16 s | +8.9 s (run 1) |
| `canplay` after the start (no click) | +65 s (run 1) / +156 s (run 2) | +52.6 s (run 1), +129.8 s (run 2: reload, as QM1k1n3) |
| click -> page `play()` | 12.6 s | 2.0 s (run 2), 4.7 s (run 3), none in run 1 (see below) |
| click -> `play` event | 18.4 s | 3.2 s (run 2), 5.3 s (run 3) |
| `play` -> `playing` (+ `top: 0px` in the same ms) | 19.5 s | 3.8 s (run 2, data buffered); run 3 25.1 s, but it waited for DATA: `waiting` +7.2 s, first SABR answer delivered +16.4 s, `canplay` +25.2 s |
| style write -> box laid and on screen (`set_video_target(Some)`) | 23.4 s (no redraw for 36 s) | 3.1 s (run 2), 11.3 s (run 3: two redraws in between, each behind ~20 forced layouts) |
| box on screen -> first rendered frame | 15.6 s (data ran dry) | 4.0 s (run 3), then frozen; run 2 none (mpv had no file open) |
| longest time without a redraw | 36 s | 19.1-27.5 s |
| redraws while playing (run 3, +30 to +46 s) | not measured | 3 in 16 s; 1 rendered video frame in the whole run |
| SABR fetches aborted by the page | 11 of the first 16 | run 2: 18 of the first 22 issued were aborted before resolving; run 3: 7 of 17 |
| wire done -> result delivered to JS (run 3, 16 SABR POSTs) | headers 0.5-1.2 s (wire) | wire done after 0.01-0.95 s; delivered 1.1-19.1 s after the request (median ~5 s) |
| PoToken stop (part 58 `08 03 10 0a`) | +195.8 s after the start | +136.5 s (run 2, before any click), +92.5 s (run 3, 57 s after the click) |
| video played | ~64 s (R42 run 2) | 18.5 s (run 3), 0 (runs 1, 2) |

**What each run showed**
- Run 1 (click 89 s after the start, after `canplay`): 21 forced layouts back to back (410-830 ms each)
  for 7.5 s after the click, no `play()` call at all, then `top: 0px` + `emptied` and YouTube's error
  card ("Ein Fehler ist aufgetreten...", screenshot). The UMP probe was not in run 1's binary; run 2
  shows the same card follows the attestation-required status, so most likely the PoToken stop had
  already come before the click (not proven for run 1).
- Run 2: no click until +154.7 s. Statuses: two media answers with `08 02` (pending), then from
  +136.5 s every answer `08 03 10 0a` (required, 109 B). At +136.5 s (while the appends of the last
  media answer, 3.25 MB, were going on; the init-segment appends were not all logged, the probe
  logged #1-3 and every 10th) mpv `EOF code 4` + `loadfile scratchy-mse://3/2/1 start=0.000`: a
  reload, the same shape as run 3's new-init reload; the new file's demuxer probe ("Trying demuxers for level=normal") never
  finished (36 s, until the page emptied the element). Click at +154.7 s: `play()` +2.0 s, `play`
  +3.2 s, `playing` + `top: 0px` +6.9 s, box on screen +10.0 s; `currentTime` stayed 0.00, no frame;
  `emptied` + error card at +19.7 s.
- Run 3: click at +35.6 s. `play()` +4.7 s (mpv `pause=no`, QM1k1n1 `keep true` at once), `play`
  +5.3, `waiting` +7.2, first media answer delivered +16.4 (1.14 MB; earlier ones aborted),
  `loadedmetadata` +21.9, `canplay` +25.2 (ct 2.23: playing from the buffer before `playing`),
  `playing` + `top: 0px` +30.4, box laid +41.7, first rendered frame +45.7 (screenshot: the
  oscilloscope picture with the pause button, progress dot at ~18 s; the picture never changed
  after). At +46.0 s `appendBuffer` #10 = 741 B (a new INIT segment, `ftyp`; mpv: the video had been
  `Reinit context to 432x240` earlier, i.e. YouTube had switched quality), mpv `pause=yes/no`,
  `loadfile scratchy-mse://2/2/1 start=18.467` and again the demuxer probe never finished; ct stayed
  18.47 for the rest of the run (tick every 2-3 s) although appends went on. Status 3 from +57.0 s.

**Why the reload hangs (code reading, not instrumented):** `MediaCore::mse_data_changed` with
`new_init` sets `want = current_time`; `mse_resolve` reloads when `feed.covers(t)` is true, and
`covers` looks at ALL stored segments. But `SourceBufferStore::start_generation` takes the NEWEST
init and `extend_run` only uses segments with that init's `init_id` that cover `t`. The new init's
segments start later than `t` (or are not there yet), so the run is empty, `read_at` answers `Wait`
for every byte after the init, and mpv's lavf probe blocks. Chrome keeps playing the old quality until
the first segment of the new one. A local reproduction with two inits was not run.

**Why fetches are aborted (measured, run 3):** wire returns the full body in 0.01-0.95 s, but the
`AppEvent::JsNet` result waits on the main thread 1-18 s; in 5 of 16 cases the page's own abort ran
in the same turn just before the delivery (the abort timer fired first), throwing away 105 B-3.6 MB
(1.71, 2.44, 3.58 MB among them). Which queue the event waits in (winit user events vs scratchy's
timer and task pump in the same turn) was not instrumented. One SABR POST was answered `411 Length
Required` (1 564 B; which request, body empty or not: not checked).

**Main-thread picture during playback (run 3, +30 to +46 s after the click):** a forced layout every
~0.33 s (282-343 ms CPU each), 3 App redraws in 16 s, each followed by an App tree build of ~300 ms.
The `top: 0px` write at +30.4 s reached the screen at +41.7 s. Frames are rendered only inside
`App::redraw` (QM1g), so with this cadence a playing video can show about one frame per 5 s even
when mpv decodes; the QM1k-mouse report (video freezes on pointer moves) is the same starvation seen
from the user side (assumption, not measured).

Not measured: who forces the layouts during playback (no JS stacks this time; R43 says `getPlayerSize`
on `#movie_player.clientWidth`); A/V sync (ao=null); mpv CPU for the off-screen keep (QM1k1n1) at
640x360; Chrome's behaviour on the second init; whether the aborted downloads use up YouTube's
pre-attestation allowance sooner (the PoToken stop came earlier than in R42, cause not known).
Items: QM1k1n4 (P1), QM1k1n5 (P1), QM1k1n6 (P2), QM1k1n7 (P3); QY53d/e/g/f/h unchanged and still
needed (forced layout is still 65-98 % of the main thread).

### Answers to the item's questions
| question | answer |
|---|---|
| player script runs? | yes (base.js, SABR, BotGuard `/js/th/...` + `GenerateIT` as in R34) |
| avc1 + mp4a with our isTypeSupported answers? | yes (`avc1.4D401E` 360p + `mp4a.40.2`) |
| attach, append? | yes, both init + media segments, no append error |
| picture? | NO on main (track order, QM1k1k); YES with the swap, 640x360, after QM1k1m's ~41 s gap |
| 0-60 s? | audio: yes with the swap (mpv played 0 -> ~64 s = buffer end); on main only 0-30 s then stall |
| A/V sync | not measured (ao=null); one mpv desync warning during a 3.8 s render stall |
| PoToken stop | status 2 for 3 media responses, then status 3 x10, YouTube error card; clean, no crash, no `waiting` hang |
| CPU budget kills | 1 timer kill (run 2) |
| Reddit feed | NOT RE-MEASURED: js_challenge + "blocked by network security" (hard stop) |
| QM1k1e page live | plays to `ended` with picture (video-first order) |

### Not established
Audio audibility and real A/V sync (ao=null throughout; ao=pulse needs the user present); the
`avsync`/drop counters; which callbacks cause the 294 forced layouts on the watch page (QY53's job);
whether the AbortErrors are caused by late callbacks; why `vid=no` was set; whether YouTube calls
`getVideoPlaybackQuality`/`played`/`webkitDecodedFrameCount` (not instrumented; QM1k1f stays as filed);
anything after a valid PoToken; seeks and quality switches (not tried, the page was too slow to drive);
Reddit (blocked); the user's own network/UA conditions (own wire, Chrome UA).

## R41: after QV8-4a the default `cargo test --release` still aborts in ~23% of runs: `vector.h:415: libc++ Hardening assertion __n < size() failed` in `Isolate::New` of a snapshot engine made while a DEFAULT-snapshot isolate (`page: None` / `without_snapshot`) is alive; plus a CPU-vs-wall-clock flake in `qy22_tripped_job_drain_logs_one_line_with_a_count` (Opus investigator, 2026-11-19) — OPEN: QV8-4b, QY22t

### What reproduces, and how often
- Test binary of main 3185656 (default config: V8 + media), run directly 31 times
  (`nice -n 19`, default 8 test threads, LD_PRELOAD'ed SIGABRT/SIGSEGV backtrace catcher):
  **7/31 aborted** (batches: 4/10, 0/6, 3/15), each ~22-25 s into the run, exit 134 (abort) or
  139 (two threads hit it at the same moment; the second dies inside the abort). The message is
  always the one the other agent saw (`vector.h:415 ... vector[] index out of bounds`). That
  agent's log (`r_default.log`, agent-a5014959) has the same timing and the same in-flight tests.
  One more run failed `qy22_tripped_job_drain_logs_one_line_with_a_count` (below).
- Windowed browser (same commit, nested dwl, `timeout 5`, LD_PRELOAD catcher): R40 local copy
  10/10, a 1-line script page 10/10, a local HN copy 10/10 alive. **The browser is not affected**
  (see "Why the browser is safe").
- Deterministic reproduction (throwaway local test, NOT committed): in a fresh test process,
  `ensure_snapshot()`, then keep a bare engine (`EngineOpts { page: None, .. }`) alive and make a
  page engine (`qb8_start`): **3/3 abort** with the same message. Page engine first, then a bare
  engine: 3/3 fine. Bare engine dropped before the page engine: 3/3 fine.

### The failing test and the stack
- Every caught abort is on a `tests::perf_tests` thread (`qv8_0_budget_flag_kills_the_50ms_loop_only_when_lower`,
  `qv8_0_perf_report_json_shape`, `qv8_0_recorder_is_a_no_op_when_off`; twice two of them at
  once). Symbolized (llvm-symbolizer on the test binary):
  `Deserializer::ReadReadOnlyHeapRef <- Deserializer::ReadObject <-
  SharedHeapDeserializer::DeserializeStringTable <- Isolate::Init <- Snapshot::Initialize <-
  v8::Isolate::New <- js_engine::Engine::new <- js::startup_engine <- js::start_scripts_clocked <-
  perf::perf_report <- tests::perf_tests::qv8_report`.
- In flight at every crash: `qv8_0_perf_corpus_json_shape_on_a_tiny_script` (its `bare_eval` makes
  `page: None` engines) or it had just run; otherwise only tests that hold no isolate (the two
  `mse_mpv_tests::real` mpv tests, `qv8_4a_stress_many_fresh_processes` which runs children, paint
  tests). The libc++ is V8's own (in the static archive; libmpv and its deps use libstdc++), so mpv
  is a bystander.

### Cause (confirmed in the V8 152.2 source in ~/.cargo/registry)
- The read-only heap is per isolate GROUP (process-wide here) and is built from the read-only
  snapshot of the FIRST isolate created when none exists (`ReadOnlyHeap::SetUp`,
  `heap/read-only-heap.cc`); later isolates reuse it whatever blob they were given. The check that
  all isolates use the same read-only snapshot (`ReadOnlyArtifacts::VerifyChecksum`) is
  `#ifdef DEBUG` only, so release builds just read wrong offsets.
- When the LAST isolate goes away, `IsolateGroup::RemoveIsolate` does
  `read_only_artifacts_.reset()`: the next isolate rebuilds the read-only heap from ITS snapshot.
- scratchy has two blobs in one process: page engines use the startup snapshot baked by QV8-4
  (its read-only space has more pages: the creator adds read-only objects), while `page: None`
  engines (`perf::bare_eval`, `js_engine_tests`) and `without_snapshot` engines
  (`js_snapshot_tests::qv8_4_snapshot_engine_matches_the_slow_start` etc.) use V8's built-in
  blob. If a built-in-blob isolate is created at a moment when no isolate exists, the read-only
  heap is rebuilt from the built-in blob; a startup-snapshot isolate created while it lives
  references read-only page indices that do not exist -> `pages()[chunk_index]` out of bounds.
  Late in the suite (perf tests run near the end, alongside tests that hold no isolate) "no
  isolate alive" moments are common, hence ~23%.
- QV8-4a (ensure_snapshot in Engine::new) fixed the bake-vs-isolate race; this is a different
  bug: two snapshots mixed in one process. The "page engine first, bare second" order works only
  because the baked read-only space is a superset; it is still unsupported by V8.

### Why the browser is safe (today)
- Outside tests, the only `page: None` engine is `perf::bare_eval` (`--perf-corpus`), whose bare
  engine is dropped before the next engine is made (sequential, no overlap). The windowed browser
  makes only page engines; after the bake isolate is disposed the read-only heap is rebuilt from
  the startup snapshot and stays consistent. Any future code that makes a `page: None` engine next
  to a page (a worker, a bare eval context) would crash the browser the same way.

### Proposed fix (QV8-4b) and test
- One blob per process: when the startup snapshot exists, EVERY non-baking isolate is created from
  it. In the bake, make the creator's DEFAULT context an empty `v8::Context::new` (made before the
  preludes run) and add the page context with `SnapshotCreator::add_context` (index 0). Page
  engines (attached) use `v8::Context::from_snapshot(scope, 0, ..)`; bare (`page: None`) and
  `without_snapshot` engines use the same blob with `v8::Context::new` (the empty default context)
  and run the slow path exactly as today (delete Intl/queueMicrotask, BASE64_JS, preludes). With
  `SCRATCHY_V8_SNAPSHOT=0` or a failed bake nobody uses the blob, which is also consistent.
  (Rejected: a mutex or keeping one "keeper" isolate alive forever: it hides the order dependence,
  and V8 forbids mixed read-only snapshots anyway.)
- Test (engine-v8, `js_snapshot_tests.rs`, child process like QV8-4a so the process starts with no
  isolate): `ensure_snapshot()`, keep a bare engine alive, make a page engine and run a script;
  also a `without_snapshot` page engine alive next to a snapshot page engine; 10 child runs, none
  may die. Fails 100% on 3185656 (measured above). ACCEPTANCE: 20 consecutive full default
  `cargo test --release` runs with no abort (today ~5 of 20 would abort).

### Second flake: qy22_tripped_job_drain_logs_one_line_with_a_count (1/31 runs)
- Failed with an EMPTY log (`assert!(!errs.is_empty() ...)`, js_observers_tests.rs). Each promise
  job busy-waits 700 ms of WALL clock (`Date.now()`) but the V8 budget is the thread's CPU clock
  (`script_clock_nanos`, 500 ms per job, QY46). Under load (8 test threads at nice 19 plus other
  agents' builds) a job gets < 500 ms CPU in 700 ms wall, no job trips, nothing is logged.
  Not V8-crash related. Fix (QY22t): burn fake CPU with the test seam (`__test_burn(700)` per job
  under `FAKE_CLOCK`, as the QV8-0/QV8-3b tests do) instead of a wall-clock spin.

### Not established
- Whether the 4 crashes of the first batch (no backtrace yet) were all perf tests: their in-flight
  sets and timing match, but only 3 crashes (5 aborting threads) were symbolized.
- No core files or full register state (no gdb/lldb on this machine); the backtrace comes from
  glibc `backtrace()` in an LD_PRELOAD signal handler (scratchpad `v4b/btcatch.c`).
- Whether the `page_then_bare` order can ever misbehave (V8 calls it unsupported; 3/3 runs fine).

## R40: justinwhitney.com/HTML5Example: on current main all four `<video controls>` boxes are drawn (640x360, first frame each) but there is no controls bar and nothing starts playback; the "no video box" report was a `~/v8-target` binary built WITHOUT `media`; NEW: the V8 startup-snapshot bake races page isolates and crashes scratchy on fast (local) page loads (Opus investigator QM1n, 2026-11-19) — OPEN: QV8-4a (crash, blocker), QM1n1, QM1n2, QM1g2d, QM1g2e

**Status: investigation only, docs only.** Main 6583b38 (QV8-5b merged; V8 + media default),
release build (`nice -n 19 cargo build --release -j2`, own worktree, RUSTY_V8_ARCHIVE = a local
copy of the already-built librusty_v8.a, 15 min 54 s). The page fetched with one plain GET through
the user's wire (`/run/user/1001/wire.sock`, no UA override) into a local copy with
`<base href="https://justinwhitney.com/HTML5Example/">`; also loaded live. Nested dwl 1052x1050
(tools/nest.sh), grim, tools/inject. libmpv 2.5.0 (/usr/lib64/libmpv.so.2) present. Each run
`SCRATCHY_TIMING=1` (media range log lines). The page has NO scripts and no CSS beyond a font rule:
four `<video width=640 height=360 preload controls>`, 1-3 = one untyped `<source>` each (.mov,
.ogv, .webm), 4 = three typed `<source>`s (mp4 `avc1.42E01E, mp4a.40.2`, ogg `theora, vorbis`,
webm `vp8, vorbis`).

### 1. What current main draws (seen in the screenshots)
- Live URL, 12 s after start: video 1 (H.264) and video 2 (Ogg) are 640x360 boxes, each showing the
  first frame (trees against a blue sky, a translucent band with the label `H.264` / `Ogg`).
  Scrolled down: video 3 shows the `WebM`-labelled frame, video 4 the H.264 frame. No controls
  bar, no play button, no poster overlay, no focus ring on any of them.
- Click in the middle of video 3, wait 2 s, `space`, wait 2 s: the three grims are pixel-identical
  (PIL difference bbox `None`); nothing starts. As expected: no page script calls `play()`, and
  scratchy has no UI that does (QM1g2 (d)).
- The ranges in the log: each file's first 1 MiB (`bytes=0-1048575`, 206), video 1 and video 4
  both fetch the .mov (two elements = two players, correct), Ogg fetches its last 65307 bytes
  (duration seek), WebM its last 1386 bytes (cues) once its box is scrolled into view. Nothing
  else; no error lines.
- Per video:

| # | box | frame | source chosen | canPlayType for its type | ffmpeg here |
|---|---|---|---|---|---|
| 1 | 640x360 | yes | `.mov` (untyped, only one) | n/a (no type) | h264 + pcm_s16le (ffprobe on the first 300 KB: the .mov's audio is PCM, not the `mp4a.40.2` that video 4's type claims; harmless) |
| 2 | 640x360 | yes | `.ogv` | n/a | theora + vorbis (+ a skeleton stream) |
| 3 | 640x360 | yes (after scrolling it into view) | `.webm` | n/a | vp8 + vorbis |
| 4 | 640x360 | yes | `.mov` = the first candidate, as in Firefox/Chrome | `video/mp4; codecs="avc1.42E01E, mp4a.40.2"` = `probably` (`can_play_type`, media_el.rs); ogg `theora, vorbis` and webm `vp8, vorbis` also `probably` | all three decodable |

  `ffmpeg -decoders` here: h264, theora, vp8, vp9, vorbis, aac all present; demuxers mov, ogg,
  matroska/webm. So canPlayType's table matches what mpv can decode for this page.

### 2. Why the user's `~/v8-target` build drew NO box (root cause: built without `media`)
- The cargo fingerprints in `~/v8-target/release/.fingerprint` show the build history: the
  `bin-scratchy` built at **19:16:59** (2026-11-18) had features `["engine-v8"]` only; the
  report was committed at 19:21:53 (3d7fa0e) and the user's answer at 19:24:39 (00c1f34); the
  binary was rebuilt at **19:26:53** with `["engine-v8", "media"]` (what is there now).
- Without `media`, `media_video::boxes_enabled()` is false, `images.rs` never builds the video
  replaced box (`build_video_box` is only reached when it is true), and `<video>` lays out as a
  plain inline element whose content is whitespace and `<source>` elements: zero size. That is
  exactly "the elements take no space".
- The CURRENT `~/v8-target/release/scratchy` (V8 + media, 103ff22-era) on the same page, local
  copy and live: four 640x360 boxes, frames drawn (one local run showed box 1 black until the
  first scroll; not seen again in 3 local runs of main, so not filed).
- So it does not reproduce on main and no later commit "fixed" it; QV8-5 (21f8b6c) made `media`
  a default feature, so a plain build can no longer miss it. What remains true: with `media`
  OFF or with libmpv NOT INSTALLED, `<video width height>` still takes no space. AGENTS.md says
  the inert placeholder is "an empty replaced box with its width/height" (or the poster): today
  there is no placeholder at all in that case. Item QM1n1.

### 3. NEW, serious: main crashes on fast page loads (V8 snapshot bake races page isolates)
- Loading the LOCAL copy with main: the window often never appears; the process dies within
  1-5 s. 10 runs each (`timeout 5`, exit 124 = still alive = OK):

| page | default | `SCRATCHY_V8_SNAPSHOT=0` |
|---|---|---|
| local copy of this page | 8/10 crashed (exit 133 trap, 134 abort, 139 SIGSEGV) | 0/10 |
| 1-line page with one inline script | 1/10 (another batch: 2/6) | 0/10 |
| live URL (page fetch 190-920 ms) | 0/2 | not run |

- Messages: `vector.h:434: libc++ Hardening assertion !empty() failed: front() called on an empty
  vector` (abort), `# Fatal error # Check failed: IsFreeSpaceOrFiller(filler).` (trap), or a bare
  SIGSEGV. Symbolized (llvm-symbolizer on the release binary): (a) thread `v8-snapshot`:
  `ReadOnlyPage::ShrinkToHighWaterMark <- ReadOnlySpace::ShrinkPages <-
  ReadOnlyHeap::OnCreateHeapObjectsComplete <- SnapshotCreatorImpl::CreateBlob <-
  scratchy::js_engine::Engine::finish_bake`; (b) `MemoryAllocator::AllocateAlignedMemory <-
  AllocateReadOnlyPage <- ReadOnlySpace::EnsureSpaceForAllocation <- ... StringTable::LookupKey`.
- Cause: QV8-4 (f62d0b0) builds the startup snapshot on a background thread (`js_engine_v8.rs`
  `warm_up` -> `build_snapshot`, a `snapshot_creator` isolate), and a page engine created BEFORE
  the bake finishes starts "the slow way" with `v8::Isolate::new` on the main thread AT THE SAME
  TIME. V8's read-only heap is shared process-wide; the snapshot creator shrinks/rewrites
  read-only pages while the page isolate allocates from them. A network page loads after the
  ~60 ms bake is done (it waits for wire), so live pages rarely hit it; local files, `snag |
  scratchy`, `:help`, fast servers do. The 103ff22 binary (no snapshot) never crashed here.
- Tests do not see it because `snapshot_for_engine` calls the BLOCKING `ensure_snapshot` under
  `cfg(test)`, but engines with `page: None` skip `snapshot_for_engine` and can still be made on
  another test thread while one bakes (possible rare test flake; not observed).
- Item QV8-4a. Workaround until fixed: `SCRATCHY_V8_SNAPSHOT=0`.

### 4. Controls (QM1g2 (d), D6)
- `grep -rn controls src/` finds no handling of the `controls` attribute anywhere (only forms
  code); `media_video::paint_box` fills the box with the frame (or the SENTINEL for the
  video-only redraw) and nothing else. Click and space on a video do nothing (above). D6
  (decided 2026-11-17) says scratchy paints a simple bar (play/pause, seek, time, volume), mpv's
  OSC stays off, keys only while the video is focused (space pause, left/right seek, up/down
  volume), the page may cancel them (QJ11). Items QM1g2d (bar), QM1n2 (interim click toggle),
  QM1g2e (fullscreen, needs the user).
- Interim worth it? Yes, as a tiny item (QM1n2): Firefox and Chrome both toggle play/pause on a
  click on the picture of a `controls` video, so it is not throwaway (QM1g2d keeps it), it needs
  ~20 lines in the existing trusted-click path, and today a script-less `controls` video (this
  page, most old demo pages, many blogs) can never be started.

### Not determined
- Whether the WebM first frame was drawn before it was scrolled into view (off-screen boxes are
  not rendered by design; the tail range was fetched when it became visible).
- The user's QuickJS install not showing the WebM frame (their report): not reproduced; main
  showed it. Not chased (QuickJS goes away with QV8-6).
- Audio: not checked (nothing plays).

## R39: V8 fits scratchy: QV8-1 spike (v8 =152.2.0, V8 15.2.124.1) behind `engine-v8`: builds reproducibly, JIT runs under the QNET1 filter with NO filter change, CPU loops 6-15x faster, corpus load 2.2x; budget/stack/modules all workable; promise-job semantics change (Opus investigator QV8-1, 2026-11-18) — VERDICT: GO WITH CONDITIONS (QV8-2/QV8-3 text adjusted; conditions C1-C6 below)

**Status: investigation only.** Spike code on branch `scratchy/qv8-1-v8-spike` (commit bb90047,
pushed, NOT merged; the coordinator decides what to keep). Default builds unchanged: the
`v8` dependency is `optional`, the feature `engine-v8` is off, and without it `src/v8_spike.rs`
is not compiled. The spike adds `scratchy --v8-spike <mode>` (seccomp, corpus, corpus-qjs,
natives, budget, stack, jobs, modules, startup, memory) and one test
(`v8_spike_libmpv_plays_next_to_v8_under_netlock`, `--features media,engine-v8`). Machine as in
PERF.md's baseline (Ryzen 5 PRO 2500U, 4 cores/8 threads, 14 GB, rustc 1.96.1); measurements
un-niced with nothing else building (load ~1.0, top idle). All numbers are thread CPU
(`CLOCK_THREAD_CPUTIME_ID`) unless marked wall. Corpus bytes = the QV8-0 pinned files
(tools/perf-corpus.sh, sha256 verified).

### Gates (QV8-1 STOP conditions): none tripped
| gate | threshold | result |
|---|---|---|
| builds reproducibly | two clean builds in two target dirs, same V8 version and lib checksum, both binaries work | PASS: archive sha256 `b6683e9a...0fed3c` and `librusty_v8.a` sha256 `8b4ff8c1...8cc225` identical in both; both report V8 15.2.124.1-rusty; both pass `--v8-spike seccomp` and `natives` (11/11) |
| JIT under seccomp | Turbofan/Maglev/Sparkplug reachable, platform threads start, process not killed | PASS: all tiers reached under the filter; 7 `V8 DefaultWorker` threads, every thread `Seccomp: 2`; zero filter changes |
| clearly faster | >= 2x on the CPU loops AND >= 2x on the corpus (compile + first run, summed) | PASS: loops 6.1-14.8x; corpus 2.23x (145.6 -> 65.3 ms). Per script 1.2-3.1x (lodash 1.19x and moment/d3/react 1.7-1.85x are under 2x on their own) |

### 1. Build
| item | value |
|---|---|
| crate | `v8 = "=152.2.0"` (rusty_v8; newest on crates.io on 2026-11-18 per `cargo info v8`; builds with our pinned rustc 1.96.1, no nightly). V8 15.2.124.1. Default features (`use_custom_libcxx`: V8's own namespaced libc++, no clash with the system one); no pointer compression/sandbox |
| download | `librusty_v8_release_x86_64-unknown-linux-gnu.a.gz` from `github.com/denoland/rusty_v8/releases/download/v152.2.0/`: **39.8 MB** (39784686 B) gz, 198 MB unpacked `.a`. The build script fetches with deno, else python, else curl (build A used deno, build B python; both installed here) |
| pinning | Cargo.lock + `=152.2.0`; the build script also honours `RUSTY_V8_ARCHIVE` (local file/URL) + `RUSTY_V8_ARCHIVE_SHA256` (refuses any other archive) |
| clean build, `nice -n 19 -j2` (build A) | 10 min 45 s wall for all deps incl. v8, + ~1.5 min for the scratchy lib (it failed once on a spike type error and was rebuilt incrementally) = **~12 min 20 s**. The QuickJS-only default build at the same settings: 10 min 31 s. So V8 adds **~2 min** at -j2 (the C++ is prebuilt) |
| clean build, normal priority, default -j8 (build B, `--locked`, fresh copy of bb90047) | **21 min 51 s wall** (user 49 min). SLOWER than -j2 here: 4-core laptop with SMT, thermals, 14 GB RAM; do not read it as a per-core cost. CI runners will differ |
| incremental rebuild after a spike edit (-j2, nice) | 1 min 31-36 s (lib compile + link against the 198 MB archive) |
| binary, unstripped | QuickJS-only 17.7 MB -> with V8 82.3 MB |
| **binary, stripped** | QuickJS-only **14.8 MB** today (Q33-0 said 11.1 MB; the code grew) -> with V8 **62.8 MB (+48 MB, 4.2x)**; `size` text 14.05 -> 61.40 MB |
| target dir | +~400 MB per profile (`gn_out/obj/librusty_v8.a` 198 MB + `libv8-*.rlib` 205 MB) |

What CI would need (NOT changed): a clean job downloads 39.8 MB from GitHub releases per build
unless cached. `Swatinem/rust-cache` would cache the target dir including the ~400 MB of V8
artefacts (fits the 10 GB repo cache, slow to restore). Cleaner: `actions/cache` of the `.a.gz`
keyed on the crate version, plus `RUSTY_V8_ARCHIVE=<cached file>` and
`RUSTY_V8_ARCHIVE_SHA256=b6683e9afcb77fbd8cb2c3acf45d34d52029ab216b332522c95179c12f0fed3c`
(also a supply-chain pin). Runner disk (~14 GB free on ubuntu-latest) is enough; python3 and curl
exist on the runner (no deno needed). Extra CI time should be minutes (V8 is prebuilt), NOT
measured on a runner.

### 2. Runtime under QNET1 (seccomp)
`scratchy --v8-spike seccomp` (main() installs the filter first, as always):
- The default platform starts **7 `V8 DefaultWorker` threads** (nproc - 1), all `Seccomp: 2`
  (filter inherited). Process not killed, nothing logged.
- JIT: with `--allow-natives-syntax`, `%GetOptimizationStatus` of the hot function = 41
  (`Optimized | TurboFanned`); a fresh function walked through every tier: cold `Interpreted`
  (Ignition) -> `%CompileBaseline` = `Baseline` (Sparkplug) -> `Optimized|Maglevved` (Maglev) ->
  `Optimized|TurboFanned`. Concurrent Turbofan compiles on the worker threads, so they work
  under the filter.
- Hot loop `hot(1e5)` x200: JIT 1.72 ms/call steady (sum 337-349 ms) vs `--jitless` 3.53 ms/call
  (sum 699 ms). (Modulo-heavy loop; the CPU loops below show bigger JIT gains.)
- **Seccomp changes needed: NONE.** The filter is a deny-list for `socket`/`socketpair`
  (non-AF_UNIX) and `io_uring_setup`; V8 needs mmap/mprotect (W^X code pages), futex, clone,
  madvise, all allowed. Nothing V8 did failed or was refused.
- After V8 ran, on the main thread AND on a new thread: `socket(AF_INET/TCP)`,
  `socket(AF_INET6/UDP)`, `socket(AF_NETLINK)` = -1 EAFNOSUPPORT; `socket(AF_UNIX)` = ok; DNS for
  `example.com` fails ("Temporary failure in name resolution").
- **libmpv next to V8**: test `v8_spike_libmpv_plays_next_to_v8_under_netlock`
  (`cargo test --release --features media,engine-v8 v8_spike`): filter installed on the test
  thread first, a V8 isolate runs fib(30) (9.3 ms), then libmpv (dlopen `libmpv.so.2`) loads,
  plays, seeks and ends a 2 s H.264 clip through the fake-wire channel; V8 still runs after it;
  AF_INET still refused. Threads at the end: 7 V8 workers + mpv core/demux/vo/9 x av:h264/lua, in
  one process. PASS.
- RSS of a process that only started V8 and ran the hot loop: 31 MB.

### 3. One isolate on the JS thread: natives and corpus
**Natives.** A typed conversion layer (`FromArg` for i32/f64/bool/String/Vec<T>/Option<T>,
`IntoRet` for those and `()`), a `native!` macro with the SAME closure shape as js_dom.rs
(`|id: i32, name: String| -> Option<String> {...}`), and one trampoline `FunctionCallback` that
calls a boxed closure stored in the function's `External` data. Wired: `__scratchy_log`
(console.log), `__d_tag` (string getter), `__d_nid` (numeric), `__d_rect` (Vec<f64> out, a
forced layout that pauses the budget) + `__d_set_classes` (Vec<String> in), `__d_attr`
(Option<String>, None -> null) and `__d_run_script` (compiles and runs JS from inside a native,
as js.rs `install_script_natives` does). 11/11 checks pass, incl. a nested script calling natives
and a wrong argument type becoming a TypeError. Call cost, same closures via rquickjs in the same
binary:

| loop | V8 | QuickJS | ratio |
|---|---|---|---|
| 1M `__d_nid` (i32 -> f64) | 45.1 ms | 232.5 ms | 5.2x |
| 1M `__d_tag` (i32 -> String) | 92.3 ms | 294.6 ms | 3.2x |
| 100k `__d_attr` (i32, String -> Option<String>) | 18.5 ms | 46.1 ms | 2.5x |

**Corpus.** `--v8-spike corpus` (V8, each script in a fresh isolate, no code cache) vs
`--v8-spike corpus-qjs` (QuickJS `perf::bare_eval`, same binary), 3 alternating runs, medians,
ms CPU. Both engines run ALL scripts **bare** (PREAMBLE only; react-dom after react, not
measured), so jquery throws "jQuery requires a window with a document" on both (the spike has no
DOM prelude). PERF.md's baseline ran jquery/react-dom with the DOM prelude, so for those two only
the compile column compares with PERF.md. The QuickJS numbers match PERF.md (three 60.5 vs 59.5,
d3 35.8 vs 37.6 ms compile).

| script | bytes | QuickJS compile | QuickJS first run | V8 compile | V8 first run | QuickJS total | V8 total | speedup |
|---|---|---|---|---|---|---|---|---|
| d3 7.9.2 | 280622 | 35.81 | 3.28 | 11.40 | 10.46 | 39.78 | 21.82 | 1.82x |
| jquery 4.1.0 (bare, throws) | 78965 | 8.72 | 0.28 | 2.23 | 0.98 | 9.00 | 3.22 | 2.80x |
| lodash 4.18.1 | 73234 | 9.30 | 1.60 | 2.61 | 6.56 | 10.90 | 9.16 | 1.19x |
| moment 2.31.0 | 60887 | 5.62 | 0.52 | 1.57 | 2.05 | 6.13 | 3.62 | 1.69x |
| react 18.3.1 | 10751 | 1.19 | 0.05 | 0.35 | 0.33 | 1.24 | 0.67 | 1.85x |
| react-dom 18.3.1 (bare) | 131835 | 13.17 | 1.24 | 3.35 | 2.90 | 14.38 | 6.25 | 2.30x |
| three 0.159.0 | 668024 | 60.45 | 3.68 | 14.52 | 5.99 | 64.12 | 20.51 | 3.13x |
| **sum** | | | | | | **145.55** | **65.25** | **2.23x** |

Read with care: V8 compiles LAZILY (top level + preparse; inner functions compile on first
call), so part of its compile moves into "first run": V8's first-run column is HIGHER than
QuickJS's for every script. Only compile + first run compares; that is the gate number.

| CPU loop (QV8-0 text) | QuickJS first | QuickJS steady | V8 first | V8 steady | speedup first | speedup steady |
|---|---|---|---|---|---|---|
| fib(25) | 21.54 | 21.48 | 1.57 | 1.45 | 13.7x | 14.8x |
| sort 200k objects | 619.6 | 621.6 | 101.4 | 91.7 | 6.1x | 6.8x |
| JSON round trip | 332.6 | 276.2 | 46.0 | 39.0 | 7.2x | 7.1x |

(first = run 1; steady = median of runs 6-10 of the same compiled script in the same context.
PERF.md baseline 21.4 / 613 / 328: this binary's QuickJS matches.) V8's sort/JSON runs vary
82-106 / 38-56 ms between runs (GC); QuickJS's are flat.

**Memory.** One context with moment+d3+lodash+three+react loaded, after a GC:

| | JS heap | process RSS before -> after |
|---|---|---|
| V8 | 4.6 MB used (5.3-5.8 MB total) | 9.8 -> 36.7 MB (peak 41 MB) |
| QuickJS | 5.6 MB used | 9.8 -> 22.4 MB |

V8's heap for the same code is slightly smaller, but each process pays ~15-20 MB more fixed RSS
(code space, platform, worker stacks). The whole-corpus processes: V8 peak 166 MB (10 isolates
made and dropped), QuickJS 78 MB. YouTube-size heaps (77-99 MB on QuickJS) not measured.

### 4. Budget, stack, promise jobs
**Budget watchdog (works).** A `js-watchdog` thread wakes every 2 ms; while a callback is armed
(deadline in BUDGET-CLOCK ns, an AtomicU64) it calls `IsolateHandle::request_interrupt`. The
interrupt callback runs ON the JS thread, so it reads the same `script_clock_nanos()` (thread CPU
minus `pause_budget_since` time) that the QuickJS interrupt handler reads, and calls
`terminate_execution()` past the deadline. The caller then calls `cancel_terminate_execution()`.
`--v8-spike budget`, 500 ms budget:

| callback | result | budget clock ms | raw CPU ms |
|---|---|---|---|
| `for(;;){}` | killed | 501.9 | 501.9 |
| next callback | runs | 0.09 | 0.09 |
| `for(;;){ try{for(;;){}} catch{caught=1} finally{fin=1} }` | killed; `caught`/`fin` stay undefined (termination is uncatchable and skips finally, as QuickJS's kill does, QY22) | 500.6 | 500.6 |
| runaway recursion that catches its RangeError and recurses again | killed | 501.8 | 501.8 |
| next callback (globals from before intact) | runs | 0.12 | 0.12 |
| 20 x `__d_rect` with 40 ms forced layout each + 100 ms of script | **not killed** (pause excluded the layout) | 99.9 | 899.9 |
| 900 ms of plain script | killed | 500.5 | 500.5 |
| after the kills: a promise + microtask | runs | 0.15 | 0.16 |

Kill latency with the 2 ms tick: budgets 5/20/50/100 ms stopped at 5.98/20.66/51.72/101.44 ms.
Overhead: fib(30) 9.18 ms disarmed vs 9.16 ms armed (5 interrupts): none measurable.

**Stack limit (QY47a equivalent, works).** `CreateParams::set_stack_limit(addr)`, addr = current
stack position - min(4 MB, half of the thread's real stack) (the `js_stack_limit` formula):

| | main thread (8 MB) | 2 MB thread |
|---|---|---|
| f(1500), f(10000) | ok, ok | ok, ok |
| nested constructors (C extends B extends A, `new C(n-1)` inside, 300 deep) | ok | ok |
| runaway `h(){h()}` | catchable `RangeError: Maximum call stack size exceeded` at depth **13591** | at 13137 |
| 8 locals per frame | RangeError at depth 8667 | 8853 |
| uncaught runaway | RangeError reported, isolate keeps working | same |
| QuickJS, same runaway test, `js_stack_limit()` | depth 2381 | 595 |

V8 frames are ~6-20x smaller than QuickJS's; R36's "YouTube's RangeError swallowed" class goes
away. No crash in any case.

**Promise jobs one at a time: partly possible; the semantics change.**
- `MicrotasksPolicy::Explicit` works: nothing runs until `perform_microtask_checkpoint()`.
- There is NO public V8 API to run ONE microtask: a checkpoint drains all. A per-job budget is
  still possible with `Isolate::set_promise_hook`: `Before` fires before every promise reaction
  job (also every `await` resumption: 2 awaits = 2 Before). Cheap form: the hook only bumps a
  thread-local job counter; the 2 ms watchdog interrupt sees a new value and re-arms the deadline
  (per-job budget at 2 ms granularity). Measured: 6 jobs x 60 ms under a 100 ms per-job budget
  all run (drain 359 ms); a runaway job between two good ones is killed at 100.3 ms.
- **On termination V8 DROPS THE REST OF THE QUEUE.** Measured (A, runaway, C): C never runs, not
  in the next checkpoint either. Source: v8/src/execution/microtask-queue.cc
  `MicrotaskQueue::RunMicrotasks`: `if (isolate->is_execution_terminating()) { delete[]
  ring_buffer_; ... size_ = 0; ... return -1; }`. QuickJS today defers the remaining jobs to the
  next turn (QY44a). The drain cap (QY46) can only be enforced by terminating, so it drops them
  too: 20 x 50 ms jobs under a 400 ms cap -> 9 ran, the other 11 are gone.
- `queueMicrotask` is not in V8 (embedder API). A native over `Isolate::EnqueueMicrotask` is NOT
  seen by the hook (all such jobs share one budget: 80 ms + 80 ms + runaway were killed as a group
  at 101 ms and the second 80 ms job was lost). A JS `queueMicrotask(cb)` over
  `Promise.resolve().then(...)` IS seen (2 Before for 2 jobs).
- Cost, 200k chained `.then` in one checkpoint: no hook 6.7 ms; hook installed (counter form)
  17.7 ms; hook reading the CPU clock per job 155-158 ms (`CLOCK_THREAD_CPUTIME_ID` is a real
  syscall, ~0.7 us: never read the clock in the hook); QuickJS (`execute_pending_job` loop)
  68-69 ms. Even with the hook V8 is ~4x faster than today.

### 5. Startup, modules, architecture
**Isolate startup** (CPU, create + context + drop): empty isolate 2.3-2.6 ms; from a trivial
snapshot (427 KB blob, 200 functions) 2.5-2.9 ms; the same script without snapshot 2.5-2.6 ms;
QuickJS runtime+context+same script 0.43-0.59 ms. A trivial snapshot gains nothing; V8 costs ~2
ms more per page than QuickJS before any prelude. The real case (~198 KB of scratchy prelude
JS: `src/*_prelude.js` plus the JS in js.rs/js_dom.rs) is QV8-4's.
QV8-4 (2026-11-18, PERF.md "QV8-4"): measured and KEPT: startup CPU 30-53 ms -> 8-13 ms per page start with an
in-process snapshot of the real startup (natives baked with their arena index, closures re-attached per page).

**ES modules / import() / import maps: fit.** `--v8-spike modules`: a static graph through
`instantiate_module(resolve_cb)` (relative and bare specifiers, the bare one mapped by the
embedder), `import.meta.url` via `set_host_initialize_import_meta_object_callback`, dynamic
`import()` from a CLASSIC script (base = the script's resource name) via
`set_host_import_module_dynamically_callback`, top-level await (evaluate returns a promise), and a
missing module rejecting with a TypeError: all work. V8 has no import-map support of its own;
scratchy already resolves specifiers itself (js_modules::PageResolver), which maps 1:1 onto the
resolve callback. Module records need a URL <-> `v8::Global<Module>` table (identity hash ->
URL) instead of rquickjs's loader traits. The spike's dynamic import is synchronous; scratchy's
fetch-then-evaluate must resolve the promise later (QV8-3 detail).

**What makes QV8-2/QV8-3 hard (counted at 24343dc):**
- ~121 native registrations: js_dom.rs 80 (`native!`), js_media.rs 16, js.rs 14
  (`Function::new`), js_net.rs 6, js_ws.rs 5; 130 distinct `__*` global names in src. Nearly all
  take plain Rust types: the spike's FromArg/IntoRet covers them mechanically.
- Engine values inside natives: js_net.rs takes an `Object<'js>` request descriptor and returns
  `Object<'js>` completions; 8 `TypedArray` sites (js_net, js_ws: bodies/frames); js.rs's run-script
  native takes `Ctx` and evals nested code. The seam needs an object reader/builder and a bytes
  type. No `Persistent` is held across calls anywhere (nothing long-lived to port).
- 14 raw `rquickjs::qjs` calls (js.rs 9: compile-only eval, `JS_EvalFunction`, exceptions;
  perf.rs 4; js_dom.rs 1).
- rusty_v8's pinned-scope API (`v8::scope!`, `PinScope<'s,'i>`, new since 137) is strict about
  lifetimes; pin the exact version (`=`), upgrades are work.
- Error/stack text differs (QuickJS `describe_error` appends a stack; V8's `toString` has none
  and its frames are `at f (url:line:col)`): ~350 tests in the JS area files (js_*, fetch, xhr,
  perf) need the seam to normalise (QV8-3 already says so).
- Out of memory: QuickJS has `set_memory_limit` (a catchable error); V8 ABORTS THE PROCESS on heap
  OOM unless a near-heap-limit callback terminates the script. NOT tested in the spike.
- Native closures: rquickjs owns them; with V8 the boxes must live in a per-isolate arena dropped
  with the isolate (the spike leaks them).
- Process-wide init: `V8::initialize_platform` once per process, flags only before it. Tests
  (many threads) can each make isolates; fine, but flags like `--jitless` are global.

### Verdict: GO WITH CONDITIONS
V8 builds reproducibly with a pinned crate and archive hash, runs its full JIT under the QNET1
filter without any filter change, runs next to libmpv, and the budget/stack/module machinery
scratchy relies on has working V8 equivalents. Script EXECUTION (what kills YouTube and bbc, R36/
R37) is 6-15x faster; script LOADING ~2.2x (lazy compile); natives 2.5-5x cheaper to call.
Conditions (written into QV8-3's TODO text):
- **C1** Budget = watchdog thread + `request_interrupt` + `terminate_execution` /
  `cancel_terminate_execution` on the existing budget clock (design above), not wall time.
- **C2** Promise jobs: Explicit policy; per-job budget by a COUNTER-ONLY PromiseHook re-armed in
  the watchdog interrupt; `queueMicrotask` in JS over a promise reaction. ACCEPT that a killed job
  (or a drain-cap trip) drops the rest of the queue (not avoidable with V8's public API); adapt
  the QY44a/QY46 tests with a comment and log "remaining jobs dropped" instead of "deferred".
- **C3** Stack limit from the real thread stack via `CreateParams::set_stack_limit` (formula
  above).
- **C4** OOM: `add_near_heap_limit_callback` that terminates the script (and raises the limit a
  little so V8 can unwind), so a heap blow-up is a logged kill, not a process abort; prove it with
  a test.
- **C5** CI: a second job with `--features engine-v8`, the V8 archive cached and pinned by
  `RUSTY_V8_ARCHIVE_SHA256`.
- **C6** The user accepts the binary growing from 14.8 MB to ~63 MB stripped and ~2 ms more
  isolate startup per page (QV8-4 may win some back). Informational, not a blocker.

### Not verified
- Real pages: nothing in the spike runs the DOM prelude or a live page on V8; the page-level win
  is QV8-m's measurement.
- OOM behaviour (C4), GC pauses on YouTube-size heaps, heap limits.
- CI time and disk on a GitHub runner (reasoned from local numbers only).
- Bit-identical binaries (not attempted; Rust embeds paths). Reproducible here = same pinned
  archive and lib checksum, same V8 version, both binaries working.
- The normal-priority build timing is one run, slower than -j2 (laptop thermals/SMT).
- aarch64 (the filter supports it; a V8 prebuilt exists; not built).

## R38: YouTube results: every thumbnail shows `true` = an `<img>` whose src is never set (IntersectionObserver reports a 0x0 intersectionRect) and, once set, never fetched (script-set `img.src` is not loaded at all); the grey masthead circles are YouTube's own loading skeleton, shown until a slow main thread gets to stamp the buttons (Opus investigator QY47c, 2026-11-18) — OPEN: QY56, QY57, QY58, QY59, QY60 — QV8-m (8dfe2f9, V8 default): main thread cheaper (load CPU 2.63 -> 2.04 s, script budget clock -26%) but forced layout unchanged (14.6 s, 70% of JS CPU); thumbnails/images not re-tested

**QV8-m addendum (Opus, 2026-11-18, PERF.md "QV8-m"):** on V8 at 8dfe2f9 YouTube results load CPU is 2005-2035 ms (QuickJS 2591-2629, same hash, interleaved), compile 1.03-1.37 s -> 0.38 s, no budget kill at the default budget (QuickJS: 1 promise job), largest budget-clock callback 446-460 ms (a promise job; 8-11% under 500). Forced layout is NOT changed by the engine: 13.4-17.5 s per 10 s window, single layouts 1.1-1.5 s, single observer deliveries 1.5-1.9 s raw; its share of the JS thread rose to 70%. So the "slow main thread" behind the grey masthead circles (section 3) is now mostly forced layout (QY53, QY60). The `true` thumbnails (QY56/QY57) are not engine-related and were not re-tested (headless harness).

**Status: investigation only, docs only.** Main 3ba1d8e (QY47a, QY50, QY51, QY52 merged), release
build in an own worktree with TEMPORARY never-committed probes: an env var that evaluates a probe
JS file right after the media prelude (js.rs `start_scripts_clocked`), `--js-report` echoing
`PROBE` console lines to stderr, and `CALLBACK_BUDGET` overridable by an env var. Own wire on a
scratchpad socket for all traffic (pages POST), scratchy's default UA, nested dwl 1052x1050,
grim, `nice -n 19`. Page: https://www.youtube.com/results?search_query=rust (kevlar_base
`ytmainappweb` app, Shady DOM), headless (`--js-report --run-timers`, saved copy with `<base>`)
and live in the window. Probe = the R36 `Error.prototype.args` recipe plus setter/method hooks
for every way a `true` can become text (createTextNode, textContent/nodeValue/data/innerText/
innerHTML setters, append/prepend/before/after/replaceWith/insertAdjacentText, appendChild/
insertBefore of a `"true"` text node), `img.alt/src/srcset` setters and `setAttribute` on
`ytCoreImage` imgs, a wrapped `IntersectionObserver`, rAF/rIC/setTimeout latency counters, and
DOM dumps at 8/20/40 s.

### 1. The `true` is an `<img alt="true">` that never gets a `src`
- **No text node `true` exists.** A full walk of the document (and shadow roots) at 8, 20 and
  40 s found 0 text nodes whose text is `true`; none of the text hooks fired.
- The thumbnail DOM (outerHTML of the first `ytd-thumbnail`): `<a id="thumbnail" ...><yt-image
  class="style-scope ytd-thumbnail"><img class="ytCoreImageHost ytCoreImageFillParentHeight
  ytCoreImageFillParentWidth ytCoreImageContentModeScaleAspectFill" alt="true"
  style="background-color: transparent;"></yt-image>` + overlays (`41:33` badge, now-playing).
  **No `src` attribute.** Headless: 6 of 6 `ytCoreImage` imgs without src; window (20 s budget):
  9 of 9 at 8 s.
- `alt="true"` comes from YouTube: `setAttribute('alt', true)` with a BOOLEAN (stack: kevlar
  `yTP` <- `GFB`, the Wiz/idom attribute setter, for the `alt:function(){return
  q.config().alt}` binding of the core image component). A browser stringifies it the same
  way, so `alt="true"` exists in Chrome too; it is hidden there because the image loads.
- scratchy paints a src-less `<img>` as its alt text (images.rs `build_img_box`: no decoded image
  -> `InlineItem::Text(alt)`), so each thumbnail shows `true` at its top centre. Correct per se
  for a broken image, but the box ignores the img's own size (QY59).

**Why src is never set (measured).** The core image component loads lazily: `Fql(img, ...)`
does `removeAttribute("src")` and, when lazy, `$50(img, U)` = `img.onViewportEntered = U;
R3W.observe(img)`, where R3W is `new IntersectionObserver(cb, {rootMargin: "10%", threshold:
.01})` and `cb` does `entries.filter(e => e.intersectionRect.height > 0).forEach(e =>
e.target.onViewportEntered())`. `U` computes the URL (`_.m5o`, picks a source by
`getBoundingClientRect().width`) and sets `img.src`. The wrapped observer saw every img entry
with **`intersectionRect` = 0,0,0,0** (and `boundingClientRect` 0,0,0,0), `isIntersecting`
true for imgs connected and shown at observe time, false for the rest. Cause: js_dom.rs
IntersectionObserver (QY6g "honest minimal") builds both rects from the target's `width`/
`height` ATTRIBUTES (`roRect(attrPx(target,'width'), attrPx(target,'height'))`); these imgs
have none, so the height is 0, YouTube's filter drops every entry, `onViewportEntered` never
runs. The same img really has a box (`getBoundingClientRect` via QJ4: 17x249 at 309,112 in the
window, the alt-text box). Also: entries are delivered ONCE at observe time; a target that is
not shown then (false) is never re-reported when it becomes visible or is scrolled to, so even
with real rects, below-the-fold thumbnails could never load.

**Fix check (probe only).** A probe variant that rewrites each entry from the target's
`getBoundingClientRect()` (intersecting = height > 0 and within 1.1 x the viewport) made
YouTube set `src` on every visible img (7 `IMGSET src=https://i.ytimg.com/vi/<id>/hq720...jpg`
/ `oar2.jpg` lines; at 8 s 13 imgs, 6 without src = the below-the-fold ones). **The thumbnails
still painted `true`** (screenshot viewed, 120 s): the src was set but never fetched (section 2).

### 2. A `src` set (or an `<img>` inserted) by script is never fetched
The App fetches `<img src>` only from the initial parse (net.rs `find_external_resources` ->
`spawn_image_fetches` at load) plus CSS `url()` images the layout missed (`WANTED_CSS_IMAGES`,
Q35a). `resolve_img` only looks `src` up in `image_cache`; nothing records a miss, so a src
that appears later is never requested. Fixture (`<base href="https://i.ytimg.com/">`, own wire,
window 12 s, screenshot viewed): a static `<img src=.../hqdefault.jpg width=240 height=180>`
shows the thumbnail; the same markup without src plus `img.src = ...` in a script shows its alt
text "SCRIPT-ALT"; an img created by `createElement` + `src` + `appendChild` 500 ms later shows
"CREATED-ALT". Both alt boxes ignore `width=240 height=180` (QY59). The JS side also lacks the
image state pages check: `img.complete` and `naturalWidth` are `undefined` (probe dump), and no
`load`/`error` event reaches the page (`handle_image_fetched` only fills `image_cache`).
YouTube's `WqD` reads `img.complete`, its `on:load` handler sets the loaded class and
telemetry. Every lazy-loading site and every SPA that renders images from JSON hits this.

### 3. Grey masthead circles = YouTube's skeleton until the buttons are stamped
The three circles are YouTube's own placeholder: `<div id="masthead-skeleton-icons"
slot="masthead-skeleton">` with three `.masthead-skeleton-icon` (32x32, radius 50%,
`#e3e3e3`), served in the HTML; the inline CSS `ytd-masthead.masthead-finish
#masthead-skeleton-icons{display:none}` hides it once kevlar's masthead
`onRendererStamperFinished` adds `masthead-finish` (after the `#buttons` renderers are stamped).
scratchy matches the rule correctly: before the class `display=flex` (rect 783,12 225x32),
after it `display=none` (`matches()` true). The icons are inline `<svg>` in `yt-icon`
(kebab `M12 4a2 2 0 ...`, the Anmelden avatar path) and paint fine once stamped (screenshots).
So nothing is wrong with the masthead itself; the stamping comes LATE (probe, `#buttons`
children and the class change):

| run | budget | masthead-finish at | results |
|---|---|---|---|
| window 60 s | 500 ms (default) | between 8 and 20 s | none (1 promise job killed, QY49-D) |
| window 60 s | 20 s (local) | not by 40 s (screenshot at 60 s: 3 circles) | 4 results, `true` thumbnails |
| window 120 s, IO probe fix | 20 s | 52.8 s | chip bar, 2 results, Shorts shelf |
| window 90 s, IO probe fix | 20 s | 47.3 s (`#buttons` 0 -> 2 children) | 4 results |

Why late: YouTube's job scheduler (`_.NU`) runs its jobs from rAF/rIC/setTimeout ticks, and
the masthead buttons are a later, lower-priority job behind the result renders. Measured in
the 90 s run (20 s budget): rAF fired 16 times in the first 30 s and 61 times in 87 s (about
0.6-0.7 per second), max rAF latency 3619 ms; rIC max latency 3350 ms; setTimeout callbacks
up to 4101 ms late; the probe's own 5 s ticker ran every 5.1-7.4 s. In scratchy rAF is a 0 ms
timer (js.rs timers prelude) and rIC a 1 ms timer (globals_prelude.js), so these latencies are
the main thread being busy, not a rAF/rIC design problem. Where that time goes is R37's
territory (forced layout inside callbacks, QY53; layout per turn) and is NOT measured here.
In the 90 s screenshot (viewed) the circles were gone but the "Anmelden" icon, the kebab and
the guide icons (Startseite, Shorts, Abos, Mein YouTube) were missing while their labels were
there; in the 120 s run all of them were drawn. `yt-icon` fetches its SVGs
(`fonts.gstatic.com/s/i/youtube_outline_experimental/<name>/v*/24px.svg`) and inlines them, so
this is most likely the same lateness, **not verified** (the icon DOM was not dumped in that
run).

### 4. Also seen (not investigated)
- `getComputedStyle(el).width` for `width:100%` returns `"1%"` and for `50%` returns `"0.5%"`
  (fixture, `--js-report`): js_style.rs `box_len` prints the stored fraction with a `%`. A
  browser returns the used value in px for width/height of a laid-out element (`"300px"`).
- 4 swallowed errors (args recipe): `Error: Can't construct url for endpoint` with
  `keys:clickTrackingParams,commandMetadata,showDialogCommand` (the "Collab-Partner" dialog
  command of collab results). Probably harmless (a URL for a dialog command); unknown in Chrome.

**Root causes, ranked:** (1) script-set/inserted `<img src>` is never fetched, and images have
no `complete`/`naturalWidth`/`load` (QY56); (2) IntersectionObserver rects come from
attributes, not layout, and are never re-evaluated (QY57); together they make every lazy
thumbnail show its alt text `true`. (3) The masthead circles are a symptom of main-thread
throughput (QY60, with R37/QY53), not a masthead bug.

**Not verified:** the user's exact run (results drawn at the default budget; here the default
budget still kills one promise job, QY49-D); the real thumbnail images with both fixes (needs
QY56 + QY57 built: the probe can fake IO but not the fetch); the lockup/`yt-thumbnail-view-model`
data variant (this run served `ytd-video-renderer` + `ytd-thumbnail`; the core image component
is shared, so the same path is expected); whether the 90 s missing icons are lateness;
Chrome's behaviour for the `Can't construct url` errors.

**Filed:** QY56 [M] P1 (fetch script-set images + `complete`/`naturalWidth`/`load`/`error`),
QY57 [M] P1 (IntersectionObserver with layout rects and re-evaluation), QY58 [S] P3
(computed-style percent), QY59 [S] P3 (alt-text box keeps the img's size), QY60 [S] P2 (Opus:
where a YouTube results turn's time goes; rAF at 0.6/s). QY47c ticked.

## R37: DuckDuckGo search = a bot-challenge page plus a timer budget trip; YouTube now serves a new app that dies on a missing `document.scripts`; where callback CPU really goes (Opus investigator QY49, 2026-11-17) — OPEN: QY50-QY55, QY49-D (budget policy, WAITING FOR USER) — QV8-m (8dfe2f9, V8 default): YouTube results no longer trips at 500 ms (worst job 446-460 ms), bbc still trips (timer 520-536 ms, was 1335-1353); forced layout still 63-94% of JS CPU; QY49-D recommendation re-made from these numbers in PERF.md (still (c), 2 s while loading)

**QV8-m addendum (Opus, 2026-11-18, PERF.md "QV8-m"):** both engines measured at 8dfe2f9, interleaved, 7 pages x 3 runs x 2 budgets. Budget kills at the default 500 ms: QuickJS 2 pages (YouTube results promise job 623-655 ms, bbc timer 1335-1353 ms), V8 1 page (bbc timer 520-536 ms). V8 makes the budget clock 0.5-0.9x of QuickJS' per page and CPU loops 6-14x faster, but YouTube's largest page-load scripts got slightly slower (420/430 -> 488/497 ms, under their 5 s budget; not attributed). Section 2's forced-layout stalls are unchanged by the engine (Wikipedia 93-94% of JS CPU, 2.7-2.9 s single observer deliveries raw; YouTube results 70%). The natives listed in section 2 run in Rust, so V8 does not speed them up. DuckDuckGo was not measured (not in the suite). Section 3 recommendation re-made with V8 numbers in PERF.md: (c) with 2 s (largest V8 callback 536 ms), plus QY53 as the real lever for stalls.

**Status: investigation only, docs only.** Main b3120dd (QY46 + QY47a merged), release build
in an own worktree with TEMPORARY never-committed probes: `CALLBACK_BUDGET` overridable by an
env var; thread-CPU logging of every `eval_value` callback and every promise job >= 30 ms, with
the forced-layout share (the QJ4 `PAUSED_NANOS` delta) shown separately; a JS profiler loaded
before the DOM prelude that wraps the `__d_*` natives, every DOM prototype method/accessor,
`RegExp.exec/test`, `String.replace/split/match`, `JSON.*` with EXCLUSIVE CPU accounting, and
the timer prelude recording which timer function ran. Own wire on a scratchpad socket (all
traffic); nested dwl 1052x1050; un-niced scratchy; load average 0.4-1.0 on 8 cores (idle).
"Budget clock" below = thread CPU minus forced layout, which is what the 500 ms budget counts.

### 1. DuckDuckGo: a bot challenge, then investigation stopped
Run: direct load of https://duckduckgo.com/?q=rust (scratchy's default UA, NO override), 40 s,
default budget. DuckDuckGo answered with its anomaly page over the grey SERP shell:
"Unfortunately, bots use DuckDuckGo too. Please complete the following challenge to confirm
this search was made by a human. Select all squares containing a duck:" with an EMPTY grid (no
squares painted) and a Submit button (grim screenshot viewed). Per the task rules the
investigation of DuckDuckGo stopped here: no home -> type -> Enter run, no /html/ run, no
retries, no other UA, no attempt to solve or work around the challenge.
Callbacks measured in that one run (thread CPU):

| what | raw CPU | forced layout | budget clock | result |
|---|---|---|---|---|
| timer 1 | 769.5 ms | 333.2 ms | 436 ms | ran (64 ms under the line) |
| timer 2 | 159.4 ms | 151.0 ms | 8 ms | ran |
| timer 3 | 500.3 ms | 0 | 500 ms | **killed**: `js: timer stopped: exceeded the 500 ms CPU-time callback budget` |
| 2 promise jobs | 94, 132 ms | 0 | 94, 132 ms | ran |

So the user's "no results + two `timer stopped`" is very probably this challenge page (one
trip here, two in the user's run: timer 1 sits 64 ms under the line, machine load decides).
**Not measured** (would need more DuckDuckGo traffic): which DuckDuckGo function the killed
timer ran, what inside it is expensive, how much it really needs, and whether the empty duck
grid is caused by the killed timer or by something else (images, canvas). Whether the user also
gets the challenge in a normal browser on the same IP is unknown.

### 2. YouTube (comparison, QY33): a new front-end, and a missing `document.scripts`
YouTube now serves "ytmainappweb" (`/s/_/ytmainappweb/_/js/k=ytmainappweb.base_module_set...`,
plus `webcomponents-sd` = the Shady DOM polyfill), not the kevlar/Polymer app of R26/R36, for
both scratchy's default UA and the Firefox 140 UA (own wire). On main it dies at startup:
`Uncaught (in promise) TypeError: cannot read property 'Symbol.iterator' of undefined at from
(native) at cFU ... at W7_`: `cFU` does `for (let l of Array.from(document.scripts))` and
scratchy has no `document.scripts` (js_dom.rs defines `forms`, `images`, `links`, not
`scripts`). The window shows a white page with three grey circles (the QY47c "grey masthead
circles"). With a local `document.scripts` shim (`getElementsByTagName('script')`):
- budget 20 s: the results page renders (chip bar "Alle", "Weitere Informationen ...", "Filter",
  four results with titles, channels, descriptions, summary/chapter boxes; thumbnails still the
  literal `true`, QY47c). Screenshot viewed.
- default 500 ms: masthead fine (kebab, "Anmelden"), content area EMPTY: one promise job of
  **1659 ms** budget clock is killed (`promise jobs stopped ... 1 job killed`). So YouTube results
  are again blocked by the budget, now by a single job (QY46's per-job budget does not help).

Distribution, YouTube /results?search_query=rust, 60 s window, shim, no profiler (callbacks >= 30 ms CPU):

| run | kind | n | budget clock max / p50 | >= 400 / >= 500 ms | raw CPU max | forced-layout total |
|---|---|---|---|---|---|---|
| 20 s budget | timers | 56 | 494 / 56 | 1 / 0 | 2480 ms | 31.0 s of 35.6 s |
| 20 s budget | promise jobs | 44 | 1647 / 53 | 5 / 3 (1647, 936, 573) | 4440 ms | 6.1 s of 13.3 s |
| 500 ms | timers | 10 | 347 / 35 | 0 / 0 | 421 ms | 1.3 s |
| 500 ms | promise jobs | 23 | 1659 (killed) / 56 | 2 / 2 (1659, 501) | 1996 ms | 0.8 s |

Timer callbacks by function (20 s run): `d` = a requestAnimationFrame tick of an animation
polyfill (`u._animations.sort ...`), 29 calls, up to 494 ms (525 ms in the profiled run);
scratchy's OWN `obsFlush` (observer delivery timer, js_dom.rs prelude) 25 calls, up to 120 ms
budget clock but up to **2480 ms raw** CPU, almost all forced layout from the page's observer
callbacks reading geometry. Forced layout is excluded from the budget (QJ4), so these callbacks
never trip, but **the main thread stalls 2.5 s in one timer and up to 3.9 s in one promise job**
(`__d_rect` 3 calls = 3882 ms): exactly the R8 condition (compositor drops a client that
stops reading its socket) that QB3's 500 ms was meant to bound. Untested whether it disconnects.

**Where the budget clock goes** (profiled run, wrapper profiler; adds overhead, so read
proportions, not absolute ms). Biggest jobs/timers:
- `__d_cs` (getComputedStyle native): **~50 ms per call** right after DOM writes (10 calls =
  503 ms in one job; 199 calls = 316 ms in another). Counted against the budget. The QA16 cache
  is keyed on the DOM write generation, so every write followed by a read re-cascades.
- `__d_special` (`document.body`/`head`): collects ALL descendants of the document per call
  (js_dom.rs `__d_special`: `descendants(&root, &mut all)` even for `body`), 0.4-1.6 ms per call
  on this DOM: 69 calls = 102 ms of the 525 ms rAF tick; 462 calls = 116 ms; 158 calls = 255 ms.
- `__d_parent`: 24k-87k calls per callback (`isConnected`, our `moAttr` mutation-record parent
  walk under `__d_setattr` <- `writeStyle` <- `styleSet` for every style write, and the Shady
  DOM polyfill), 35-126 ms per callback.
- Page JS: the Shady DOM polyfill (`webcomponents-sd`) `insertBefore` 803 ms inclusive in a
  1037 ms job; ytmainappweb `renderImpl`/`GpG`/`QOC` tree walks (`nextSibling`, `tagName`,
  `nodeType`, `children` filters). RegExp/String/JSON were small (RegExp.exec 24-50 ms per job).
- A script-inserted external script (`__scratchy_run_global`, 1031 ms) runs under its own
  `SCRIPT_BUDGET` (top = true): not a budget problem.

### 3. Budget policy: the trade-off, plainly
R8/QB3: a page that stalls the main thread while the pointer moves gets scratchy disconnected
by the compositor (1.5 s busy loop + 400 motion events = disconnect; 2 ms pointer motion on
Wikipedia = 4/4 crashes before QB3). QB3 bounded every timer/event callback to 500 ms and
after it 7/7 survived. Raising the callback budget re-opens that window by the same amount.
Measured today: real sites need 0.5-1.7 s of budget clock in single callbacks (YouTube 1659 ms
job, DuckDuckGo 500+ ms timer); AND the 500 ms bound is already not a bound on the stall,
because forced layout (excluded) adds up to 3.9 s. Options:
(a) Reduce cost only (QY51-QY53): needed in any case, but no evidence it brings YouTube's
    1.6 s job under 500 ms; DuckDuckGo's cost is unmeasured.
(b) A separate larger timer/job budget (e.g. 2 s): sites work, but every stall up to 2 s is
    allowed at any time, including while the user moves the pointer (R8 risk back).
(c) A larger budget only while loading (e.g. 2-5 s until N s after the `load` event): covers
    both cases seen today (every trip measured happened while the page was still loading, in
    the first ~30 s after navigation); a slow callback after load still dies.
(d) Input-aware: the larger budget only while no input arrived in the last ~250 ms, 500 ms
    otherwise: targets R8's actual trigger (stall + input flood). More code; needs QY54's
    measurement to be worth it.
Recommendation: (c) now (smallest, matches every trip measured), plus QY51/QY52/QY53 to lower
costs, plus QY54 to measure whether the forced-layout stalls already disconnect; (d) only if
QY54 shows disconnects. This is a USER DECISION (QY49-D): it changes the R8 protection.

**Not verified:** DuckDuckGo beyond one direct load (see 1); the user's exact DuckDuckGo flow and
UA; what the YouTube 1659 ms job is (killed job not profiled; the profiled run's biggest pure
jobs were 1847/1752/1124 ms with profiler overhead); YouTube SPA search (only direct loads);
whether the stalls above disconnect from a real compositor under pointer motion (QY54).

**Filed:** QY50 [S] P1 (`document.scripts`), QY51 [S] P1 (`__d_special` without a full walk),
QY52 [M] P2 (`__d_cs` cost), QY53 [M] P2 (forced-layout stalls in observer callbacks), QY54 [S]
P2 (R8 re-test with today's stalls), QY55 [S] P3 (DuckDuckGo, with the user), QY49-D (budget
policy, WAITING FOR USER). QY49 ticked (investigation).

## R36: YouTube search: Enter and the search button "do nothing". The page's SPA navigation runs, then a CPU-budget trip kills it (wall 1); with the budget raised, the results page renders no results (wall 2). NOT QF11, NOT form submission (Opus investigator, 2026-11-17) — OPEN: QY46, QJ12, QF11 (refined), QY48 filed; wall 2 ROOT CAUSE FOUND (QY47 addendum below: QuickJS 1 MB stack limit, RangeError swallowed by YouTube) — OPEN: QY47a-d — QV8-m (8dfe2f9, V8 default): on V8 the stack limit comes from the real thread stack (QV8-3b), so wall 2's QuickJS 1 MB limit does not apply; the direct results load has no budget kill and 0 JS errors (no RangeError check beyond that); the SPA search flow (wall 1) was NOT re-tested

**QV8-m addendum (Opus, 2026-11-18, PERF.md "QV8-m"):** headless `--perf-report` of the DIRECT results URL on V8: 0 kills at the default budget (QuickJS same hash: 1 promise job killed), 0 JS errors, worst budget-clock callback 446-460 ms. Not re-tested: the user's flow (home -> type -> Enter, SPA navigation, wall 1) and whether results are DRAWN (headless; no screenshot in QV8-m). The QuickJS fallback still has the 1 MB stack limit until QV8-6 removes it.

**Status: investigation only, docs only.** User report 2026-11-17: `scratchy --socket "$(wire
--print-socket)" --user-agent '<Firefox 140 UA>' https://youtube.com`: autocomplete works
(suggestions appear while typing), but Enter in the search box and clicking the search button
do nothing.

**Verdict: a known class of problem (QY33: YouTube callbacks over the 500 ms CPU callback
budget), with a NEW concrete casualty, plus a second, unrelated wall behind it.** Hypotheses
from the task, in order:
1. `location.assign/href=` no-op (QF11): **ruled out.** YouTube never calls
   `location.assign/replace/reload`, never sets `location.href`, never calls `window.open` on
   submit. The only `location.href` writes seen came from scratchy's own `history` mirror
   (globals_prelude.js `apply()`, called by `pushState`/`replaceState`). QF11 is still a real gap
   (other sites), and that `apply()` detail is a trap for QF11 (see the item).
2. The SPA path runs and is killed by a budget: **CONFIRMED (wall 1).**
3. Q30e implicit submission never reaches YouTube: **ruled out.** Enter reaches the page and
   YouTube handles the submit; the button click is handled by YouTube before any submit.
4. YouTube refusing the UA: **ruled out for the Firefox UA**: `/youtubei/v1/search` answered 200
   and the result data reached the DOM. (Not run with wire's default UA.)

**Setup.** This worktree at b97054e (main), release build, default features, plus TEMPORARY
never-committed logging (a JS prelude evaluated after the media prelude in js.rs
`start_scripts`, wrapping `location.href/assign/replace/reload`, `history.pushState/
replaceState`, `window.open`, `fetch` (and its completion for search/browse/next URLs),
`XMLHttpRequest.open`, `Event.prototype.preventDefault` for submit/key/click,
`EventTarget.prototype.dispatchEvent` for types matching `navigat|search|submit|yt-action`,
window capture + bubble listeners for keydown/keypress/submit/click, and a DOM dump 3 s and
12 s after each `yt-navigate-finish`; Rust `diag!` lines in `text_control_key` (Enter),
`implicit_submit`, `request_submit`, `form_navigate` and the `reqs.nav` consumer; a per-promise-
job wall timer in `drain_jobs`; and `CALLBACK_BUDGET` made overridable by an env var). Nested
dwl (tools/nest.sh, 1052x1050) + tools/inject + grim, `nice -n 19`, user's wire, Firefox 140 UA,
https://www.youtube.com. Load: the machine was busy (user testing, other agents), so the job
times below are WALL ms of a niced process, not thread CPU.

**Run 1/2 (default budget): Enter.** Click into the box, `type:rust`, wait 5 s, `key:ret`:
```
PROBE capture keydown key=Enter keyCode=0 code= target=input.ytSearchboxComponentInput   (trusted)
PROBE rust Enter reached text_control_key (keydown not cancelled)
PROBE capture keypress key=Enter keyCode=0 ...   bubble-end keypress defaultPrevented=false
PROBE rust implicit_submit: default_button=None implicit_ok=true
PROBE capture submit target=form.ytSearchboxComponentSearchForm trusted=true
PROBE preventDefault submit on form.ytSearchboxComponentSearchForm @ w (kevlar_base ...:28878:19)
PROBE dispatchEvent yt-navigate on ytd-app
PROBE fetch https://www.youtube.com/youtubei/v1/search?prettyPrint=false
PROBE dispatchEvent yt-navigate-start ... "url":"/results?search_query=rust" ...
PROBE history.replaceState https://www.youtube.com/
PROBE history.pushState /results?search_query=rust
PROBE rust request_submit: submit event not cancelled = false      (so form_navigate is skipped: correct)
PROBE fetch done https://www.youtube.com/youtubei/v1/search?prettyPrint=false status=200
PROBE job took 355 ms (ok=true)
PROBE dispatchEvent yt-navigate-set-page-offset on ytd-page-manager#page-manager
PROBE job took 496 ms (ok=false)
scratchy: js error: js: promise jobs stopped: exceeded the 500 ms CPU-time callback budget (1 job killed, remaining jobs deferred to the next turn)
```
`yt-navigate-finish` never fires afterwards (it did fire for the initial home load). Screenshot
(viewed): the home page's "Der YouTube-Verlauf ist deaktiviert" card is gone, the content area is
white, the masthead still shows "rust" in the box, "Startseite" still highlighted. That is the
user's "nothing happens".

**Run 8 (default budget): the search BUTTON** (`move:772,22 click`): YouTube's own click handler
on `button.ytSearchboxComponentSearchButton` preventDefaults the click (no submit event at all,
correct for a JS-handled button) and starts the same path: `yt-navigate`, the search fetch,
`pushState /results?search_query=rust`, `fetch done ... status=200`, `PROBE job took 377 ms
(ok=true)`, `PROBE job took 114 ms (ok=false)`, `promise jobs stopped: exceeded the 500 ms
CPU-time callback budget (1 job killed ...)`. Screenshot (viewed): the suggestion dropdown (14
"rust ..." suggestions) stays open over a white content area; "Startseite" still selected.
Note the 377 + 114 ms: the jobs of ONE drain share ONE 500 ms budget (js.rs `drain_after_task`
arms `CALLBACK_BUDGET` once, then `drain_jobs` runs every pending job under it), so the job
that dies is whichever one crosses the line, not necessarily an expensive one.

**Run 3/4 (`CALLBACK_BUDGET` = 20 s via the temporary env var): wall 2.** Enter now reaches
`yt-navigate-finish` (url `/results?search_query=rust`, jobs 351 ms and 533 ms wall) and the
guide's "Startseite" icon switches to its outline (inactive) form, but the results area stays
white (screenshot viewed: only a small dark bar at about x 548-576, y 91 under the masthead).
DOM dump 3 s and 12 s after finish (identical): `ytd-search` 1 (rect 72,56 952x92), `ytd-browse`
hidden (`display:none`, `hidden`), `ytd-two-column-search-results-renderer` 904x60,
`ytd-section-list-renderer` text " ", `ytd-item-section-renderer` height 0 with children
`#header` (0 kids), `#spinner-container` (a paper spinner), `#contents` with **3
`yt-lockup-view-model` + 1 `grid-shelf-view-model`, each with 0 children**, `#continuations`
(0 kids); `ytd-video-renderer` count 0; `#secondary` (`ytd-secondary-search-container-renderer`)
holds the "Rust Videospiel ..." watch card text but is `display:none` at this width (expected).
So the search DATA arrived and was bound; the new "view model" result items render nothing.

**Run 5-7 (direct load of https://www.youtube.com/results?search_query=rust, 20 s budget):**
the SAME empty state without any submission: wall 2 is independent of search submission (it
will also hit the home feed when history is on and any other page whose items are
`yt-lockup-view-model`). On the first `yt-lockup-view-model`: custom element defined (class `Q`;
`grid-shelf-view-model`, `yt-thumbnail-view-model` also defined, `yt-lockup-metadata-view-model`
NOT defined), no `shadowRoot`, attrs `class="ytd-item-section-renderer lockup ..."`, own props
`rawProps, queuingData, owner, dispose, isWebComponentWrapper, isReparenting, hasRendered,
isLazyComponentFn, componentProps, setters, slotProps, _numRenders, _signalProps,
_signalValues`, `hasRendered = true`, `_numRenders = 1`, `queuingData = false`,
`isLazyComponentFn = false`, `rawProps` has `data` and 16 other keys, `innerHTML` length 0. No
`js error`, no `console.error`, no `Uncaught` line in the whole run (other than the probe's own
lines). So YouTube's component wrapper believes it rendered once and produced no DOM, silently.
Globals at that time: `scheduler` undefined, `requestIdleCallback` a function.

**Also seen (not investigated):** on every direct load `js net: GET https://youtube.com/: CORS:
Access-Control-Allow-Origin "https://www.youtube.com" does not match the page origin "null"`
(some context with an opaque origin fetches the bare host).

**Root cause, wall 1:** js.rs `drain_jobs` under one `CALLBACK_BUDGET` (500 ms CPU) per drain
(`drain_after_task`, also after `deliver_net`): YouTube's navigation continuation after the
`/youtubei/v1/search` response (render of `ytd-search`, set-page-offset, finish) needs more than
what is left of that shared budget on this machine; the uncatchable interrupt kills the job
mid-way (QY44a) and the router never fires `yt-navigate-finish`. Same mechanism as QY33/R28 (the
kebab icon), now with a user-visible casualty. **Root cause, wall 2:** unknown (QY47). [Found later: QuickJS's 1 MB stack limit; see the
QY47 addendum at the end of R36.]

**What a browser does that we do not, still unverified:** whether the per-job cost here is
mostly scratchy's own prelude (R29 found `ceIsConn`/`ceScan`/`isConnected`/`ioShown`
walks inside killed jobs) or YouTube's code. The job times above are wall ms under nice 19 on a
loaded machine, NOT thread CPU; QY46 starts by measuring thread CPU.

**Not verified:** wire's default UA (only the Firefox UA was run); an un-niced run on an idle
machine (the trip may be marginal there: 377 + 114 ms); the user's exact binary (this was main
b97054e + probes); any page after the results page (clicking a result).

**Filed:** QY46 [S] P1 (per-job budget in the promise drain, measure first), QY47 [M] Opus
investigate (`yt-lockup-view-model` renders nothing), QJ12 [S] (trusted KeyboardEvent
`keyCode/which/code` are 0/''), QF11 refined (not the cause; `apply()` trap; size), QY48 [S]
(pushState never reaches the App's URL/history).

### R36 addendum (QY47, Opus investigator, 2026-11-17): wall 2 = QuickJS's 1 MB stack limit; YouTube swallows the RangeError

**Root cause (measured): the JS stack runs out.** QuickJS's default max stack size is 1 MB
(`JS_DEFAULT_STACK_SIZE`; scratchy never calls `set_max_stack_size`, js.rs `start_scripts_clocked`).
Rendering a YouTube result item nests custom-element constructors synchronously (Polymer stamper
`stampDomArray_` -> `createComponent_` -> `document.createElement` -> scratchy `created` ->
`ceUpgrade` -> `Reflect.construct` -> YouTube "PolySi" wrapper constructor -> `mountComponent` ->
Wiz `render`/`Ct`/`W6y` -> `cTP` -> `document.createElement` -> ...) and needs about **1.15 MB** of
C stack on this page. Past 1 MB QuickJS throws `RangeError: Maximum call stack size exceeded`.
YouTube's Wiz wrapper (kevlar `r10`, and the PolySi `createElement()` try/catch) catches it, reports
it through `_.S$` (an error beacon; it logs to the console only when the `console_log_js_exceptions`
flag is on or the server is dev/staging, so scratchy prints NOTHING), leaves the wrapper empty
(`r10` returns its default `[[this], noop]`, which passes the "single root element" check, so
`hasRendered = true`, 0 children: exactly the R36 state), and `W7` rethrows when there is no owner
error boundary, which aborts the item-section stamping job: every item after the failing one is
never created.

**Setup.** This worktree at fda4bbc (main), release build, plus TEMPORARY never-committed patches
in js.rs: `SCRIPT_BUDGET` 60 s and `CALLBACK_BUDGET` 20 s (so QY46 does not mask it), an env var
that evaluates a probe JS file right after the media prelude, and env overrides for
`set_max_stack_size` / `set_memory_limit`. Own wire on a scratchpad socket, Firefox 140 UA.
Page: https://www.youtube.com/results?search_query=rust saved with `<base>` (headless
`--js-report --run-timers`) and loaded live in a nested dwl (window, 60 s, grim). Load average
~2 (idle machine), everything `nice -n 19`.

**How the swallowed error was found** (reusable trick): YouTube's catch handlers do
`e.args || (e.args = [])` before reporting, so a probe that defines an `args` accessor on
`Error.prototype` sees every error YouTube catches and reports, with its stack
(`Error.stackTraceLimit` raised; QuickJS caps a captured trace at 64 frames). First and only one
on this page: `RangeError Maximum call stack size exceeded`, args `["Error in web component
wrapper","video-summary-content-view-model"]` (the "Zusammenfassung" summary box inside the first
`ytd-video-renderer`); top frames `apply (native)` <- media prelude `document.createElement`
wrapper <- DOM prelude `created` wrapper <- kevlar `cTP` <- `W6y` <- `render` <- `Ct` x3 ...

**Measurements** (`depth()` = how many trivial self-recursive JS calls still fit = remaining stack
in "trivial frames"; one trivial QuickJS frame = about 1.75 KB of C stack on this build):

| run | stack limit | trivial frames at top level | min. remaining inside createElement of a `*-*` tag | swallowed RangeErrors | `grid-shelf-view-model` (empty) | `ytd-video-renderer` | elements |
|---|---|---|---|---|---|---|---|
| headless | 1 MB (default) | 599 | 196 | 1 | 1 (1) | 3 | 2848 |
| headless | 1.5 MB | ~900 | 221 | 0 | 11 (0) | 123 | 23732 |
| headless | 4 MB, heap 1 GB | 2402 | - | 0 | 11 (0) | 123 | 24397 |
| window 60 s | 1 MB (default) | 599 | 244 | 1 | 1 (1) | 3 | 2845 |
| window 60 s | 4 MB | 2402 | 1725 | 0 | 4 (0) | 16 | 5212 |

Peak use is therefore ~677 trivial frames = ~1.15-1.2 MB in both the window and headless runs, a
few percent over the limit. Screenshots (viewed): default stack = one result ("DAS NEUE
FORCE-WIPE-UPDATE ...") and nothing below it, no chip bar; 4 MB = chip bar ("Alle", "Weitere
Informationen zu diesen Ergebnissen", "Filter"), three results with titles, channel, description,
chapter/summary boxes, then the "Shorts" shelf (the `grid-shelf-view-model`) with its "Neu" badges.
The depth at top level is the same in the window and headless (599), so it is not a deep Rust
caller stack: rquickjs re-arms the stack top on every `ctx.with` entry.

**Data variant.** On 2026-11-17 evening YouTube served `videoRenderer` items (20) + 2
`gridShelfViewModel` (Shorts); R36's earlier run got `yt-lockup-view-model` items. The empty
`yt-lockup-view-model` state in R36 (`hasRendered = true`, `_numRenders = 1`, 0 children, no
error) is exactly what `r10`'s catch path leaves, and in this run the empty grid shelf after the
abort is `queuingData = true`, `hasRendered = false` (never reached). The lockup variant itself was
NOT reproduced today (not served), so "same cause for lockups" is strongly supported, not proven.

**Also seen with the 4 MB stack (not investigated):** (a) every result thumbnail is blank with
the literal text `true` at its top centre (a boolean rendered as text, or an `<img>`/`yt-image`
attribute reflection problem); (b) the masthead's right side ("Anmelden" button, settings kebab)
turns into three grey circles (placeholder/loading shapes) [(a) and (b): root causes in R38,
QY47c]; (c) headless (`--js-report`) keeps
loading continuations (30 item sections, 24k elements) and hits the 256 MB QuickJS heap limit
(`InternalError: out of memory`, then `Uncaught null` lines) with the stack at 2-4 MB; RSS
reached 1 GB with a 1 GB heap limit. The window loaded only 2 sections in 60 s, no OOM.

**Not verified:** the default 500 ms `CALLBACK_BUDGET` together with a bigger stack (QY46 not
merged); the SPA search path (only the direct load was run this time); the lockup data variant;
watch-page related items and the home feed; per-frame C stack cost on other compilers/opt levels.

**Filed:** QY47a [S] P1 (stack limit, Sonnet), QY47b [S] P3 (fewer createElement wrapper
frames, Sonnet, measure first), QY47c [S] P2 (Opus: `true` thumbnails + grey masthead circles),
QY47d [S] P3 (Opus/Sonnet: headless continuation runaway / 256 MB heap). QY47 ticked.

## R35: QCI0 (`flex: <number>` basis `auto` -> `0%`) on real pages: no visible change on 10 of 11 pages, BBC better; but the automatic minimum size is now 0 for every `flex: N` item (Opus investigator, 2026-11-17) — OPEN: QCI0b filed

**Status: investigation only, docs only.** BEFORE = fcfa57e (parent of 752aaa1), AFTER = a91dee3
(QCI0 merged), both release builds in their own target dirs. Pages saved 2026-11-17 with
`scratchpad/gate/fetch.py` through the user's wire (GET only, `<base>` added) and opened as files
(`--socket`, so stylesheets/images/scripts load live). Each page shot BEFORE, AFTER, BEFORE again in
a nested dwl (1052x1050, 20 s, then `pgdn pgdn`, second shot); BEFORE-vs-BEFORE2 is the noise
check. `flex: N` count = `flex:` followed by one or two bare numbers and no basis, in `<style>`,
`style=""` and linked sheets (fetched; `@import` and script-inserted CSS NOT counted).
Load average was 15-18 (other agents' builds) during the runs.

| page | `flex: N` uses | before = after? | difference seen | verdict |
|---|---|---|---|---|
| www.wikipedia.org | 0 | identical | - | - |
| en.wikipedia Main_Page | 1 (`flex:1 0`) | identical (BEFORE2 differs by 882 px: noise) | - | - |
| news.ycombinator.com | 0 | identical | - | - |
| github.com | 64 (`flex:1` x59, `flex:1!important` x5) | identical (AFTER = BEFORE2; only the first BEFORE run differs: load timing) | - | - |
| duckduckgo.com | 3 inline | identical | - | - |
| youtube.com | 528 (`flex:1` x509) | not conclusive -> no flex effect seen | first pair: AFTER lost every icon (guide, kebab, account glyph). A BEFORE re-run produced the SAME icon-less image byte for byte, and both binaries otherwise land on the same blank/partial states; logs show 2-6 "exceeded the 500 ms CPU-time callback budget" trips per run (load 17). JS budget noise, not layout. | same |
| bbc.com | 4 inline | DIFFERS (deterministic: BEFORE = BEFORE2) | masthead BBC logo moves 25 px left: BEFORE x 481-621 (25 px right of centre), AFTER 456-596 (centred in the 1052 px window). The real bbc.com centres the logo; the old `auto` basis gave the left flex:1 column (menu+search icon) more room than the empty right one. | **better** |
| theguardian.com/international | 4 inline | identical (AFTER = BEFORE2; first BEFORE differs: load timing) | - | - |
| lobste.rs | 2 (`flex: 1`) | identical | - | - |
| old.reddit.com (fetched, 200) | 7 | identical | - | - |
| social.loupax.com/discover | 1 inline | identical (BEFORE2 differs: noise) | - | - |
| repo fixtures | none use `flex:` (only src/help.html, no flex) | - | - | - |

WPT subset, both binaries (`--wpt /home/loupax/src/wpt --out`): pass 320 / fail 294 each, "no change
from wpt-expected.txt", and every one of the 588 written PNGs is byte-identical between the two.

**But a real regression exists that these pages do not show (fixtures `qci0b/pages/fx.html`,
`fx2.html` in the investigator's scratchpad; same in QCI0b):** layout.rs clamps a flex item's
automatic minimum size to its own flex basis (row: `(min_mb - margins).max(0.0).min(basis)`,
~line 4377; column: `lc.h.min(basis)`, ~line 4633). With basis `auto` that clamp was a no-op
(basis >= min-content); with QCI0's `0%` it makes the floor 0 for EVERY `flex: N` item, so
- row, 300 px box, `flex:1` items "Pneumonoultramicroscopicsilicovolcanoconiosis" | "short text
  here": BEFORE 342 px word + "short" column overflowing the box (what Chrome/Firefox draw: the
  word's min-content is the floor); AFTER 150/150, the word is broken mid-letters over 3 lines.
- column, `height:100px`, two `flex:1` items of 3 lines each: BEFORE 60+60 px, overflowing (real
  browsers: same, `min-height:auto` = content height); AFTER 50+50, "three" is overdrawn by "four"
  and "six" spills out of the box.
- row, `flex:1; width:50px` beside a `flex:none; width:280px` item in 300 px: BEFORE 50 px (real
  browsers: the specified-size suggestion caps the floor at 50), AFTER 20 px.
CSS Flexbox 4.5: the content-based minimum is min(content size suggestion, specified size
suggestion = the item's `width`/`height` if definite); `flex-basis` is not part of it. It also
applies only when the item is not a scroll container: `overflow` other than `visible` gives 0.
The row path ignores `overflow`, which QCI0 accidentally papered over: fixture `flex:1;
overflow:hidden; white-space:nowrap; text-overflow:ellipsis` + a 100 px item in a 300 px row was
WRONG before (item kept its full nowrap width, row overflowed by ~290 px) and is RIGHT after
(clipped with "..." at 200 px). So the fix must not simply drop the clamp. Fix: QCI0b.

Not verified: pages below the second viewport; logged-in pages; youtube under low load
(blocked by budget noise); script-inserted and `@import`ed CSS in the count; no real browser was
run (the "real browser" columns are from knowledge of the spec and of how these sites look).

## R34: MSE / streaming survey on real sites (YouTube, Reddit, Vimeo, BBC, Guardian, Twitch, X) (Opus investigator QM1k-m, 2026-11-17) — EVIDENCE for QM1k (plan with the user); QY45 READY; QM1k1-QM1k6 filed; QM1k PLANNED 2026-11-18 (addendum at the end; items QM1k4, QM1k1a-j, QM1k-AB, QWK0-QWK-m)

**Status: investigation only, nothing committed except docs.** Run on the user's machine through
the user's wire (`/run/user/1001/wire.sock`, `X-Wire-Features: ua`), never logged in, all audio
`ao=null`. Main @ b717e31 (QM1f built; QM1g frame painting NOT merged, so no screenshot can show
video pixels: the Reddit screenshots below show the poster).

**Headline:**
1. **YouTube needs MSE, nothing else.** The watch page's player response has NO `formats`
   (no progressive itag 18 any more) and `adaptiveFormats` WITHOUT `url`/`signatureCipher`: only
   `serverAbrStreamingUrl` (SABR). Same with and without `--user-agent` (Chrome 131 string).
   Without MediaSource the player shows "Dieses Video kann in deinem Browser nicht abgespielt
   werden" (`errorCode fmt.noneavailable`) and fetches no media at all.
2. **With MSE, YouTube DOES serve media to wire** (POST `videoplayback`, `application/vnd.yt-ump`,
   200), for both UAs, but **only ~60 s**: then UMP `STREAM_PROTECTION_STATUS` goes 2
   (attestation pending) -> 3 (attestation REQUIRED) and every further response is ~123 bytes
   with no media. BotGuard runs (`www.google.com/js/th/...js` 200, `POST /api/jnn/v1/GenerateIT`
   200, 128 bytes, request body 2443 bytes) but its token is not accepted (or not attached). This is
   PoToken: the open risk for YouTube (QM1k2).
3. **Reddit plays TODAY, without MSE, through QM1f**: the feed's `shreddit-player` picks the
   progressive muxed mp4 from its `packaged-media-json` (`packaged-media.redd.it/<id>/pb/m2-res_854p.mp4?...`),
   `<video src>`, muted autoplay: loadedmetadata, playing, ran 30.9 s to `ended`, seek to 20 ->
   `seeked` 62 ms later, `media: mpv`. It did this even with a MediaSource stub present.
4. **Most other players never reached media code in scratchy** (non-media blockers): Vimeo (dynamic
   `import()`, QY15a), Twitch (no `Worker`/`WebAssembly`), BBC (SMP player never renders, cause not
   found), X (no video in the logged-out profile), Guardian (click-to-play; long videos are YouTube).
5. **No EME call on any page measured** (`requestMediaKeySystemAccess` never called). DRM seen
   only in Vimeo's manifests (some videos: Widevine/PlayReady `cenc` DASH + FairPlay `SAMPLE-AES`
   HLS; others clear).

### Method (all throwaway, in the session scratchpad; nothing of it committed)
- Own worktree `cargo build --release --features media` with a LOCAL js.rs patch (reverted right
  after the build, binary copied to the scratchpad): when `QMKM_STUB` is set, two natives
  (`__qmkm_log` -> stderr, `__qmkm_env`) and a JS file evaluated right after media_prelude.js,
  before page scripts. The JS (`qmkm_stub.js`) has modes: `net` (logs fetch/XHR to media-ish URLs,
  canPlayType, EME, `<video>` creation/src/events; `window.MediaSource` stays undefined = today),
  `mse` (plus a logging MediaSource/SourceBuffer/SourceBufferList stub, `blob:` URLs for it,
  video element state for an attached MediaSource, appendBuffer sniffing: fMP4 top boxes + `tfdt`/
  `trun` time range, WebM EBML ids + Cluster timecode + last SimpleBlock time, TS sync byte; UMP part
  decoding of `application/vnd.yt-ump` responses via `Response.clone()`), `mse-mp4` (refuses webm),
  `QMKM_PLAY=1` (advance `currentTime` while buffered, so the player keeps buffering as if playing),
  `QMKM_ACT` (timed JS snippets: seek, quality switch, DOM dumps). It also polyfilled the missing
  `Element.setAttributeNS` family (YouTube's player dies without it, QY45).
- Headless: `--js-report --socket <wire> --run-timers N` on saved copies (with `<base>`), default
  UA and `--user-agent <Chrome 131>`. Live: one window at a time in a nested dwl (tools/nest.sh),
  tools/inject for clicks, grim screenshots (viewed). Plain GETs with a tiny Python wire client for
  manifests/segments (DASH/HLS playlists, Range checks). A Python reimplementation of Reddit's
  `js_challenge` was refused by the session's safety classifier and NOT pursued; Reddit was only
  ever loaded by scratchy itself.

### Per-site table
| Site (page) | Needs | Without MSE (today) | Formats / containers | Segment URLs, method, status | DRM | Seek / quality |
| --- | --- | --- | --- | --- | --- | --- |
| YouTube watch (aqz-KE-bpKQ, Big Buck Bunny) | MSE + UMP/SABR client (the page's own base.js does SABR; scratchy only needs MSE) + PoToken? | error card "can't be played in your browser" (`fmt.noneavailable`), no media request | MSE: 2 SourceBuffers, audio `audio/webm; codecs="opus"` (WebM: EBML/Segment/Info/Tracks init 259 B, then one ~160 KB Cluster per 10 s) + video `video/mp4; codecs="av01.0.01M.08"` (fMP4 ftyp+moov 700 B, then moof+mdat 4-7 s each); refusing webm gives `mp4a.40.2` + `avc1.4D401E` (all fMP4) | `POST https://rrN---sn-....googlevideo.com/videoplayback?...&sabr=1&rqh=1&c=WEB&n=...` (protobuf body, `fetch(Request)`), no Range, no `range=`; 200 `application/vnd.yt-ump`; first response 1,077,374 B (8 MEDIA_HEADER, 37 MEDIA, init metadata) = 0-20 s of both tracks; then 655 KB, 449 KB, 417 KB, 94 KB up to 60 s; then 121-123 B (status 3) forever. Plain GET of the same URL: 403 (empty UMP) | none (no EME call, no drm fields) | seek to 300 s: `currentTime` set, a new SABR POST, but status 3 -> no media (behaviour with a valid token unknown). Quality (`setPlaybackQualityRange('hd1080')`): the player made a NEW MediaSource, revoked the old blob URL, re-added both SourceBuffers (no `changeType`, no `remove`). No `remove()`, `timestampOffset`, `mode`, `endOfStream` in 2 min |
| Reddit feed (r/oddlysatisfying) | nothing new: `<video src>` progressive | PLAYS (QM1f + libmpv), muted autoplay, 3 players created, seek works | packaged muxed mp4 (`ftyp`, H.264+AAC, 4.79 MB/30 s at 854p), permutations by height; also DASH (`DASHPlaylist.mpd`, `SegmentBase` single-file CMAF per rep `CMAF_220..480.mp4`, separate `audio/mp4` mp4a.40.2) and HLS (`cf.v.redd.it/.../HLSPlaylist.m3u8`, CMAF, separate AUDIO group, subtitles) | packaged mp4: Range -> 206 (signed `e=`/`s=` query); `v.redd.it/<id>/DASHPlaylist.mpd` 200 unsigned; `v.redd.it/<id>/HLSPlaylist.m3u8` 403 without the `a=` token, 200 on `cf.v.redd.it` with `f=..&v=1` | none | seek: `seeking`->`seeked` 62 ms (QM1f). Quality: not triggered (feed player picks one permutation by size) |
| Reddit post page | — | NOT MEASURED: scratchy got a reCAPTCHA page ("Could not connect to the reCAPTCHA service") on `/comments/<id>/`; not attempted further | — | — | — | — |
| Vimeo player (player.vimeo.com/video/22439234 "The Mountain"; 76979871) | its player module (dynamic `import()`) | blank: the player never boots (`import(playerConfig.request.urls.js_module)` / `CSSLayerBlockRule` gate; QY15a); only Cloudflare's challenge "precursor" ran | config has `dash`, `hls`, `progressive: []` (no progressive); HLS: fMP4 (`EXT-X-MAP` init 803 B + moof segments ~1.7-1.9 MB/6 s, byte ranges encoded in the path `range/prot/<b64>` plus `range=` query), separate audio rendition; DASH mpd (64 KB) | GET, 200 `video/mp4`, Range 206 works | 76979871 ("The New Vimeo Player"): YES, DASH `cenc` with Widevine+PlayReady ContentProtection, HLS `#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://drm"` (FairPlay), init has `encv/sinf/tenc` = impossible. 22439234: clear | not measured |
| BBC News video (bbc.com/news/videos/c6grew79p7qdo, vpid p0pncywd) | SMP (bump-4.js + requirejs) | the player area is empty (screenshot); no `<video>` media, no mediaselector request | mediaselector JSON: HLS (USP, MPEG-TS segments `video/MP2T`, ~300 KB/7.68 s, muxed AAC), DASH (clear, `avc3` + separate `mp4a.40.5`), and in mediaset `mobile-tablet-main` PROGRESSIVE mp4 (`vod-pro-ww-live.akamaized.net/...`, Range 206, 2.08 MB) | all GET 200/206 | none (`availableTerritories` uk+nonUk) | not measured |
| Guardian video pages | click-to-play | headless: nothing until a click (not clicked); `/commentisfree/video/...` main media is a YouTube atom (`assetId N79jV1waf4g`, `hasYouTubeAtom`) = the YouTube route; short self-hosted clips on `/video` list HLS + mp4 sources | self-hosted: `uploads.guim.co.uk/...m3u8` (CMAF fMP4, single file with `EXT-X-MAP ... BYTERANGE`, separate audio) and two progressive `video/mp4` (16.0 MB, 13.8 MB, Range 206) | GET 200/206 | none | not measured |
| Twitch | Amazon IVS player in a `Worker` (`amazon-ivs-wasmworker.min.js` + `.wasm`, or `amazon-ivs-worker.min.js`) | `Worker=undefined WebAssembly=undefined`; no player (directory page; a live channel page not tested); also `ReferenceError: Path2D is not defined` in polyfills.js | (HLS, not measured) | `spade.twitch.tv/track` -> wire "Connection refused" | not measured | — |
| X (x.com/NASA, logged out) | — | profile header renders; no `<video>` created in 50 s; timelines need login | not measured | not measured | not measured | — |

### What YouTube asked (MSE mode, 35 distinct queries)
`MediaSource.isTypeSupported` for `video/webm; codecs="vp9"` (11x), with probe parameters
`width/height/framerate/bitrate/eotf/channels/cryptoblockformat/decode-to-texture/experimental/
tunnelmode` each once VALID and once INVALID (`eotf=catavision`, `width=99999`, `channels=99`,
`bitrate=2000000000`, `codecs=av99...`), plus vp09 HDR strings, av01 (0.04M, 0.05M, 0.13M), avc1
(42001E, 4D401F, 64002A), mp4a.40.2, opus. Chrome answers the invalid ones `false`; a real
isTypeSupported must do the same or the player believes in impossible formats (QM1k4).
`canPlayType` was only called with mp4/avc1 strings (all "probably" today), never with an HLS type.

### Recommended route per site
| Site | Route | Size / notes |
| --- | --- | --- |
| YouTube | FULL MSE (two SourceBuffers -> two stream_cb streams, fMP4 video + WebM-or-fMP4 audio; mpv `audio-files`/external track or one demuxer per stream) + the PoToken question | MSE core is the L item QM1k; whether more than ~60 s plays depends on PoToken (QM1k2). If attestation cannot be passed, YouTube = first minute only. Prefer answering mp4-only first (`mse-mp4`: avc1 + mp4a, both fMP4) so the first build needs only one container |
| Reddit feed | PROGRESSIVE (already works) | nothing to build for the feed; QM1g painting makes it visible. Post page: blocked by reCAPTCHA, not media |
| Vimeo | after QY15a: full MSE (DASH, clear) or native HLS (fMP4 + separate audio); IMPOSSIBLE for DRM videos | which one Vimeo's player picks was not observable (player never booted) |
| BBC | progressive mp4 exists (mobile mediaset) or native HLS (TS, muxed: simplest HLS case, one stream) | the SMP player must boot first (QM1k5) |
| Guardian | YouTube atoms = YouTube route; self-hosted: progressive mp4 (`<source>` list) works with QM1f if the player uses it | click needed; not verified |
| Twitch | impossible today (needs Workers + WebAssembly, a much larger decision) | its streams are HLS; a native-HLS route would still need the IVS player to run |
| X | unknown (logged out shows no video) | not measured |

Native HLS (scratchy fetches playlist + segments through wire and concatenates into ONE stream for
mpv) fits BBC (TS, muxed) directly; CMAF/fMP4 HLS with a separate AUDIO rendition (Reddit, Vimeo,
Guardian) needs two streams, the same as MSE. No measured page asked `canPlayType` for HLS, so a
native-HLS answer would only help players that check it (Twitch's error path does; BBC/Guardian
not observed).

### Open product questions (for the QM1k session)
1. PoToken: accept "YouTube plays ~60 s" as a first milestone, or require attestation research
   (QM1k2) before building MSE?
2. Codecs to admit in isTypeSupported: mp4-only (avc1/mp4a, simplest: both tracks fMP4) or also
   WebM/Opus/VP9/AV1 (YouTube's default choice was AV1 video + Opus/WebM audio)?
3. Audio: second stream to mpv (`audio-files` on a second stream_cb URL) vs remux both tracks into
   one container in scratchy (more code, one demuxer)?
4. Memory cap for appended data (YouTube appended 3.4 MB for 60 s at 360p; 1080p60 AV1 is ~10x).
5. Native HLS at all (BBC is the only measured case that would use plain TS HLS)?
6. Workers/WebAssembly for Twitch: out of scope for media, or a separate decision?

### NOT verified
Real video pixels (QM1g not merged); audio (always `ao=null`); YouTube with a real-time clock in a
live window (BotGuard may behave differently than under `--run-timers`' simulated clock); what
YouTube does after a VALID PoToken; the request body (protobuf) of SABR POSTs; YouTube live
streams/Shorts; Reddit post-page/HLS player (reCAPTCHA), Reddit quality switch; Vimeo, BBC, Twitch,
X players' actual MSE use (none booted); Guardian clicks; any EME on a page that needs it (none
seen); seek/quality on anything but YouTube (stub) and Reddit feed (seek only).

### Addendum 2026-11-18: the QM1k plan (Opus, source reading only; nothing built, run or loaded)
User answers (QUESTIONS.md Q4): 60 s of YouTube is the first milestone; mp4 (avc1/mp4a fMP4) first
but codec-agnostic; audio as a second mpv input (A), remux (B) only if an early test fails; 64 MiB
per player; native HLS later; Workers/WebAssembly in scope at lower priority. The plan and its
ordered items are in TODO.txt (QM1k-PLAN, QM1k4 -> QM1k1a..j, QM1k-AB, QM1k-B, QM1k-m2, QWK0..QWK-m).
Facts established from the tree and the machine (no process started):
- Byte path today: `media_mpv.rs` registers ONE stream_cb protocol (`scratchy-media`), and
  `st_open` opens a `media_net::MediaStream` (ranged GETs through wire, 1-4 MiB chunk cache) for
  the single `MediaSource {url,page,mode}` held in `Reg`; read/seek/size/cancel map 1:1 to it.
  MSE needs a second protocol whose reader is an in-memory, GROWING stream (blocking read on a
  Condvar, EOF only after endOfStream, size unknown). No wire/protocol change: MSE bytes already
  came through wire as page fetches (R34: SABR POSTs via `fetch`).
- `MpvBackend::load` always does `loadfile scratchy-media://1 replace`; mpv runs with
  `keep-open=yes`, `demuxer-max-bytes=32MiB`, `demuxer-max-back-bytes=16MiB` (MPV_OPTIONS), so an
  MSE player would hold its data twice: the plan lowers mpv's cache for MSE loads (16/8 MiB).
- Installed: libmpv 0.41 (`libmpv.so.2.5.0`; `loadfile` has the index argument since 0.38),
  ffmpeg 8.1.2 with dav1d and opus (AV1, Opus and the native VP9 decoder are available for the
  later codec flip, QM1k1j). `audio-files` is a path LIST option split on ':' and a
  `scratchy-mse://...` URL contains one, so the external audio track must be added with
  `change-list ... append`, a NODE array or `audio-add` (QM1k1b decides by test).
- mpv rebases timestamps to start at 0 by default (`rebase-start-time=yes`); MSE needs media time
  (YouTube seeks to 300 s and appends fragments with tfdt = 300 s), so MSE loads set
  `rebase-start-time=no` and `start=<t>` (to be confirmed by QM1k1b's test).
- JS seam: natives go through `js_engine::native!` on both engines; `js_engine::Bytes` accepts a
  `Uint8Array` only (V8: `v8::Uint8Array::copy_contents`, view offset honoured), so
  `appendBuffer(ArrayBuffer|DataView|view)` must be normalised in JS first.
- `URL.createObjectURL` (fetch_prelude.js) throws TypeError for anything but a Blob; the blob
  store (`__blob_lookup`) can hold a MediaSource too. `media_prelude.js` sends every src to
  `__media_load` as a URL; `srcObject` is a no-op getter/setter today.
- Not present today: `getVideoPlaybackQuality`, `progress`/`stalled`/`suspend` events, `played`
  ranges (always empty), canvas 2D (`getContext` returns null), `Path2D`, any Worker. V8's
  `init_v8` passes no flag that hides `WebAssembly` (`SCRATCHY_V8_FLAGS` only), so it should exist
  on the default build: NOT verified (QWK1 adds the test). Wasm streaming APIs need an embedder
  callback in V8; the plan polyfills them over `Response.arrayBuffer()`.
- `buffered` must be right at `updateend` (players decide what to fetch from it), so it cannot
  come from mpv's demuxer cache (asynchronous). The plan reads container box headers only (fMP4
  mdhd/tfdt/trun/tfhd/trex; WebM Info/Cluster/Block times later), never codec data: user
  confirmation asked in QUESTIONS.md Q7.
- Seeks: an MSE "file" is not one byte stream once the page seeks (YouTube appends fragments for
  t=300 s after those for 0-60 s). The plan's GENERATIONS (one mpv load = init + the contiguous
  run of segments from the seek point) avoid lavf seeking in an unsized fragmented stream; the
  cost is one reload per out-of-buffer seek, measured by QM1k-AB.
How option A vs B is decided: QM1k1b ships an `#[ignore]` test `mse_ab_drift_60s` (sync-marker
fixtures made by ffmpeg, split into video-only and audio-only fMP4, appended by a Rust fake MSE
driver in YouTube's order) that prints mpv `avsync`/drop counters/reload latency every 250 ms;
QM1k-AB (Opus) runs it 3x on a quiet machine and applies the pass criteria in TODO.txt.

## R33: A test fails on a clean checkout of main: `inline_flex_lays_its_children_out_side_by_side_not_stacked` (review, 2026-11-17) — FIXED (QCI0)

(Archived: full text in REGRESSIONS-FIXED.md.)

## R32: Every shaped paragraph wipes cosmic-text's font-match cache (`db_mut()` in `shape_spans`): 10-37x slower shaping everywhere, seen as "1 forced layouts, 2198 ms" on social.loupax.com/u/loupax (Opus investigator, 2026-11-17) — FIXED (QP1 6c1ae4e, QP2 d5fe517); QP3 (measurement) open

**Status: FIXED 2026-11-17: QP1 (6c1ae4e, the `db_mut()` fix: bench 3.4 s -> 42 ms, the forced-layout line is gone) and QP2 (d5fe517, shape memo on the JS thread); QP3 (cost of a font-match cache miss) is still open.** Original status: root cause found and measured, nothing committed. User report:
`scratchy --socket $(wire --print-socket) https://social.loupax.com/u/loupax`
prints `scratchy: js: 1 forced layouts, 2198 ms (script reads geometry after
DOM writes)`. The page is only 9.7 KB of HTML (29 posts, 2 inline `<style>`
blocks, no external CSS), so 2 s for ONE layout is far out of proportion.

### Who forces the layout

vim.js (loaded by egg.js `import("/vim.js")`) runs `buildGutter()` at load:
`document.documentElement.scrollHeight` plus one `getBoundingClientRect()` per
post (vim.js:99-104). The first read forces a layout (QJ4, `GeomCache::refresh`
in src/js_geom.rs); later reads hit the cache until the next DOM write. vim.js
then adds its gutter rows (a DOM write), and the next read lays out again. On
this page clamp.js does nothing (no `.skim` container); on /discover it reads
`scrollHeight` after each clamp write, which is one forced layout per clamped
post. The log line fires once at 20 layouts OR 1000 ms (`GeomCache::refresh`),
so the "1 forced layouts" in the report means a single slow layout, not a
read/write loop in the page.

### How this was measured (reusable)

- The sandbox could not run wire against the site (wire dials out directly; the
  sandbox only allows its HTTPS proxy), so: `curl` the page and its 9 scripts
  (vimflag, confirm, embed, clamp, egg, passkey, lists, upload, vim), then make
  `inlined.html`: `<base href="https://social.loupax.com/u/loupax">` after
  `<head>`, vimflag inlined in place, the 7 deferred scripts inlined at the
  end of `<body>` in order, and egg.js REPLACED by vim.js wrapped in
  `(function(){ ... })();` (it is a module; unwrapped, its top-level `bar`
  collides with another script's). Run `target/release/scratchy --js-report
  --socket <unused> inlined.html`: 8 scripts, 0 errors, and the report prints
  `3 forced layouts, 1549-1672 ms` (headless 1024x768; the user's window got 1
  layout of 2198 ms).
- TEMPORARY `Instant` timing in `GeomCache::refresh` (removed): tree build
  8-15 ms, glyph walk ~1 ms, `measure_document` 367-502 ms per layout (a third
  layout with vim.js's 325 gutter rows needed 2 deadline passes, 500 + 256 ms).
- TEMPORARY timing around `buffer.shape_until_scroll` in `shape_spans`
  (removed): 695 paragraphs, 359 ms of shaping. Even `"31"` (a gutter number,
  DejaVu Sans Mono, which IS installed) took 0.41-0.47 ms; the slowest were
  2.6-2.9 ms. `set_rich_text` is ~2 us. HarfBuzz-class shaping of 2 glyphs
  should take microseconds.
- `valgrind --tool=callgrind` of the same run: only 1.46 G instructions for the
  whole process (well under a second of CPU), so most of the wall time is
  syscalls. 4,814 `FontSystem::get_font_matches` calls, of which **1,387
  missed** the cache and rebuilt the full match list: **98,490
  `FontMatchKey::new`** calls (~71 faces per miss, 119 fonts installed in the
  sandbox). Each one with a weight different from the request calls
  `fontdb::Database::with_face_data`, and with fontdb's `memmap` feature (which
  cosmic-text enables) that is `File::open` + `mmap` + skrifa parse + `munmap` +
  close on the font FILE (fontdb-0.23.0 src/lib.rs `Source::with_data`).
  `try_statx`, `read_fonts::FileRef::new` and the `FontMatchKey` sort were all
  in the self-cost top 20.
- Logging every distinct `AttrsOwned` the page shapes (TEMPORARY, removed):
  only **16**, far under the cache's 256-entry limit, so the cache was not
  overflowing. It was being wiped.

### Root cause

`shape_spans` (src/layout.rs) resolves each span's family with
`span.font_family.resolve(font_system.db_mut(), attrs.weight, attrs.style)`.
cosmic-text 0.19's `FontSystem::db_mut()` is

```rust
pub fn db_mut(&mut self) -> &mut fontdb::Database {
    self.font_matches_cache.clear();
    &mut self.db
}
```

(cosmic-text-0.19.0 src/font/system.rs). It clears the cache because a caller
holding `&mut Database` may add or remove fonts. `FontFamily::resolve`
(src/css.rs) only needs `&fontdb::Database` (it calls `db.query`, and caches
its own answers in a thread-local). So EVERY span of EVERY paragraph wipes the
font-match cache, and the next shape rebuilds it from scratch, opening and
mmapping ~71 font files. It is the only `db_mut()` call in src/ outside tests.

### Measured fix (one word: `font_system.db_mut()` -> `font_system.db()`)

Applied locally, measured, then reverted:

| measurement | before | after |
|---|---|---|
| social /u/loupax forced layout, `measure_document`, headless | 367-502 ms | 6-12 ms |
| social /u/loupax, report line | `3 forced layouts, 1549-1672 ms` | not printed (all 4 layouts ~40 ms total) |
| `cargo bench --bench layout -- "measure, cold"`: 200 paragraphs + list + table | 810 ms [792-835] | 34.2 ms [33.5-35.0] |
| same: 200 paragraphs each with one inline icon | 348 ms [341-355] | 38.7 ms [37.7-39.8] |

The benches drive the same `measure` and `shape_spans` code the App's shaping
worker (`spawn_shaping_worker`) uses, so the fix speeds up EVERY page load,
resize and restyle in the window, not only geometry reads: a 9-24x faster cold
measure. It is very likely a large part of QY33's YouTube callback cost
(forced layouts of 1130-1511 ms) and of QJ4b's "~3.4 s on a 1.4 MB page"
note. Both need re-measuring after QP1.

### Secondary findings

1. **The JS thread does not use the content-keyed shape memo.**
   `SHAPE_MEMO_ENABLED` (src/layout.rs) is only set on the shaping worker
   thread (`spawn_shaping_worker`, src/app.rs). `GeomCache::refresh` builds a
   fresh id-keyed `ShapeCache` per layout, so each forced layout reshapes every
   paragraph. Enabling the memo on the JS thread (TEMPORARY) made a repeated
   layout of the same DOM 1.6-2.3 ms instead of 367 ms (before QP1), or
   1.7-2.5 ms instead of 6.4-6.6 ms (after QP1). After QP1 this matters only
   for pages that read geometry in a write/read loop (clamp.js on /discover).
   NOTE: do NOT persist the id-keyed `ShapeCache` across refreshes instead.
   Its keys are positional paragraph ids with no content check, so any DOM
   insert shifts the ids and serves the wrong text's shape (tried TEMPORARILY:
   faster, but unsafe).
2. **The memo would grow without bound on the JS thread.** The worker clears
   `SHAPE_MEMO` whenever a request has `reuse_shaping == false`; the JS thread
   has no such point. Keys include the width, so a resize leaves old entries
   behind. Clear it when `GeomCache`'s viewport key changes, or cap it.
3. **A cache miss is still expensive after QP1**: about 71 file
   open/mmap/munmap calls. Misses are rare once the cache is no longer wiped
   (16 distinct attrs on this page), but the first shape per distinct attrs on
   each `FontSystem` (the App's, the worker's, the JS thread's) still pays it.
   Not measured separately; see QP3.

---

## R31: CSS animations, transitions, Web Animations and `@font-face` do not exist; survey and design evidence (Opus investigator QAN1, 2026-11-17) — PARTLY BUILT (QAN1-0, QAN1a-c done); QAN1d0, QAN1d-i, QFF1a-c open in TODO.txt

**Status (audit 2026-11-17, main @ e4993c3):** QAN1-0, QAN1a (parse), QAN1b (f965c9a, evaluate; 9 more WPT passes, the PR #232 body test passes again) and QAN1c (4c45340, start/end restyles + events, settle-first, no frames) are DONE. QAN1-D DECIDED 2026-11-17 (reduce motion by default via new QAN1d0, infinite animations freeze after 5 s, 30 fps, none while hidden). Open: QAN1d0, QAN1d-QAN1i, QFF1a-QFF1c; perf follow-up parked as QAN1p. The "nothing built" status below is the original one.

**Status: evidence memo, nothing built.** Today `TopLevelParser::parse_prelude`
(css.rs) accepts only `@media`/`@supports`/`@layer`; `@keyframes` and
`@font-face` blocks are dropped, and `DeclParser` has no `animation*` or
`transition*` arm, so those declarations are dropped too. The JS side has the
`AnimationEvent`/`TransitionEvent` constructors (js.rs) but no
`Element.prototype.animate`, `getAnimations` or `document.timeline`.
`prefers-reduced-motion` is parsed and always answers `no-preference`.

### 1. Why `css/css-backgrounds/animations/background-color-animation-in-body.html` flipped in PR #232

The test puts `animation: bgcolor 1000000s cubic-bezier(0,1,1,0) -500000s` on
an EMPTY `<body>` (keyframes rgb(0,200,0) -> rgb(200,0,0)); the reference is
`body { background-color: rgb(100,100,0) }`. The negative delay puts the
animation at 50 % of its duration; cubic-bezier(0,1,1,0) at x = 0.5 gives
y = 0.5 with slope 0, so the value is rgb(100,100,0) and stays there.

- Before QW6 (dba5942): body background painted only on the body box, which
  is 0 px tall, so test and reference were both all white: an accidental PASS.
- After QW6: `propagate_canvas_background` (images.rs) moves the REFERENCE's
  body colour to the document root box; the test has no colour (animation
  dropped), so it stays white: FAIL. Seen in the runner's PNGs (`--out`,
  viewed): test = all white 800x600; reference = an olive rgb(100,100,0) band
  at y 0..7 (pixel-probed: y=7 olive, y=8 white), white below.
- Side finding: the band is only 8 px because the WPT runner
  (`wpt::Renderer::render`) paints the `Laid` tree onto a white buffer and does
  NOT do the whole-window canvas fill that `App::redraw` does since QW6. A real
  browser paints the whole 800x600 olive. It does not matter for this test
  (both sides would paint the same band), but the runner and the app disagree
  (item QAN1i).
- To pass: parse `@keyframes` + the `animation` shorthand (name, duration,
  `cubic-bezier()`, negative delay), evaluate the animation at document time 0
  in `resolve_style` (the runner's clock), interpolate `background-color` in
  sRGB, and let `propagate_canvas_background` take the animated body colour as
  it already does for a declared one. QAN1a + QAN1b are enough; no frames, no
  App changes.

### 2. WPT

- Subset (550 tests, `wpt-subset.txt`): 3 tests use `@keyframes`/`animation`:
  `background-color-animation-in-body` (FAIL; flips with QAN1a+b),
  `css-cascade/revert-layer-011` (FAIL; needs `@property` + `revert-layer`
  in a keyframe: out of scope), `css-masking/animations/clip-path-interpolation-
  shape-arc-direction-agnostic-radius` (FAIL; needs `clip-path: shape()`: out of
  scope). 2 tests have an inline `@font-face` (`css-values/ic-unit-003`,
  woff2; `css-conditional/at-supports-content-002`, Ahem.ttf inside
  `@supports`). About 60 more subset tests link `/fonts/ahem.css`, but the
  runner preloads `fonts/Ahem.ttf` into the fontdb (`new_font_system_with`), so
  they do not depend on `@font-face`.
- Whole checkout (`css/`, 25 767 reftests, scratchpad script): 233 reftests use
  `@keyframes`, only 73 of them without script/`reftest-wait` (the runner skips
  the rest). 33 use `transition`, 2 without script. Directories of the 73:
  filter-effects 22, css-animations 16, css-backgrounds 8, css-transforms 4,
  css-values 4, css-viewport 4, others 1-3. Properties animated in them:
  background-color 28, transform 22, filter 22, backdrop-filter 22,
  background 13, rotate/height/width 10, scale/translate 8, clip-path 6.
- Ran all 73 with main 70edcad (release, `--subset`, empty expectations):
  **36 PASS today, 37 FAIL.** The PASSes are accidental in the same way as the
  body test was (e.g. 22 filter-effects animation tests, the `with-table1..4`
  and `transparent-animation-in-body` background tests: the animation does
  nothing and the reference happens to match). Implementing animations can turn
  any of them into a pass->fail, so QAN1-0 adds all 73 to the subset FIRST.
- Candidates likely to flip FAIL->PASS with QAN1a+b (read, NOT run with an
  implementation): background-color-animation-in-body, -with-images,
  css-animations/animation-delay-011/012 (`::after`, `steps`/`step-end`,
  paused, `animation-delay: inherit`), animation-important-002 (steps(2,end)),
  animate-font-size-with-margin-override (fill both), css-cascade/important-prop
  (author !important beats the animation; `!important` inside a keyframe is
  ignored), css-conditional/at-media-content-003 and at-supports-content-003
  (`@keyframes` inside `@media`/`@supports`), css-values/vh-interpolate-px/
  -pct/-vh (width/height in vh and %), min-max-percentage-length-interpolation
  (needs `min()`/`max()` interpolation: maybe not).
- `@font-face` is a much larger WPT lever: 1 023 no-script reftests in `css/`
  declare an inline `@font-face` (css-text 321, WOFF2 298, css-fonts 227,
  css-writing-modes 105, css-values 21, ...). Formats referenced in `src: url()`
  across css/: .woff 1 019, .woff2 458, .ttf 449, .otf 291.

### 3. Real pages (fresh copies through the user's wire, jar `qan1-investigator`, harmless GETs)

Static CSS only: `<style>` blocks, `style=""` attributes and `<link
rel=stylesheet>` files (one `@import` level); CSS that page JS injects later
(YouTube's Polymer templates, BBC styled-components, Guardian emotion) is NOT
counted, so JS-heavy pages are under-counted.

| page | CSS bytes | `@keyframes` | `animation*` decls | `transition*` decls | `@font-face` (formats) | `prefers-reduced-motion` | JS mentions |
|---|---|---|---|---|---|---|---|
| youtube | 3.86 MB (5 files) | 254 (130 + 124 `-webkit-` duplicates, all in `ytmainappweb.base_module_set`) | 300 + 279 `-webkit-` | 458 + 281 `-webkit-` | 11 (truetype, Google Fonts css2: Roboto/YouTube Sans) | 49 | `.animate(` 3, `getAnimations` 7 (the web-animations polyfill), rAF 24 |
| github | 3.47 MB (30 files) | 85 | 226 | 398 | 6 (woff2, woff2-variations: Mona Sans, Hubot Sans, fallbacks) | 97 | `.animate(` 16 (several unguarded), `getAnimations` 3, `transitionend` 3, `FontFace`/`document.fonts` 2 |
| bbc | 82 KB inline | 0 (3 in JS) | 0 | 7 | 35 (woff2: BBC Reith Sans/Serif, `-fallback` local() faces) | 0 | rAF 4 |
| guardian | 711 KB (1 file + inline) | 0 (2 in JS) | 0 | 38 (19 `-webkit-`) | 30 (woff2/woff/truetype: Guardian Egyptian/Headline/Text Sans) | 0 | rAF 11 |
| wikipedia.org | 58 KB inline | 0 | 0 | 14 | 0 | 0 | 0 |
| en.wikipedia Main_Page | 220 KB | 0 | 3 | 22 | 0 | 1 | rAF 1 |
| duckduckgo | 65 KB inline | 4 | 8 | 12 | 6 (eot/woff2/woff/ttf/svg: DuckSans Product/Display) | 5 | `transitionend`/`animationend`/`animationstart` 3 each |
| lobste.rs | 47 KB (6 files) | 0 | 0 | 1 | 0 | 0 | 0 |
| news.ycombinator.com | 8 KB | 0 | 0 | 0 | 0 | 0 | 0 |

What is animated: inside `@keyframes`, YouTube `transform` 358 / `-webkit-
transform` 352 / `opacity` 320 / `scale` 22 / `rotate` 18; GitHub `opacity` 86,
`transform` 77, `clip-path` 18, `color` 6. Transitioned properties: YouTube
`opacity` 201, `transform` 246 (incl. prefixed), `background-color` 57,
`width` 50, `visibility` 43; GitHub `transform` 83, `opacity` 81,
`background-color` 57, `border-color` 32, `color` 29; Wikipedia/en.wiki/DDG
`color`, `background-color`, `border-color`, `box-shadow`, `max-height`,
`opacity`. Timing functions: `cubic-bezier()` dominates YouTube (467) and
GitHub (129), then `ease`, `linear`, `ease-in-out`; `steps()` 3, `step-end` 6.
So opacity + translate/transform + colours cover the bulk; layout properties
(`width`, `height`, `max-height`, `top`, `right`) are a minority.

Active at load (scratchy's own selector matcher run over the STATIC DOM of each
copy, temporary local test, `@media` conditions ignored, so an upper bound for
the static DOM and a lower bound for JS-built DOMs):
- duckduckgo: 4 elements, the loading spinner, `animation: .6s infinite`
  (transform/opacity).
- github: 8 elements: 6 `MonaHead__particle` (4 s, once) and 2
  `LogoSuite logobar-marqueeGroup` (`animation-iteration-count: infinite`,
  linear, `transform: translate(...)`: an endless logo marquee, i.e. a page
  that would never stop asking for frames).
- youtube static skeleton: 0 (its animations live on JS-built elements).
- bbc, guardian, en.wikipedia, wikipedia.org, lobste.rs, HN: 0.
- Elements matched by a non-zero `transition`: bbc 15, duckduckgo 5,
  github 103, guardian 23, wikipedia.org 21, en.wikipedia 2 335 (every element;
  a broad rule), HN/lobste.rs 0. Those only animate when a hover/focus/JS
  restyle changes a transitioned property.

Content that is hidden until an animation runs (a rule with an animation AND
`opacity:0`/`visibility:hidden` in the same block): github 4 (`.anim-fade-up`,
`.lp-Intro:before`, checkbox `:before`s), youtube 11 (paper-tooltip fade/slide,
reaction control panel, promo-panel hero/scrim). Today such content stays
invisible forever (the fill never applies). Same for code that waits for
`transitionend`/`animationend` to continue (DDG, GitHub, YouTube reference
them): those events never fire today.

### 4. Cost per frame (temporary local `Instant` test, reverted)

`build_layout_tree_with_resources` (DOM walk + cascade; `RULES_CACHE` hit) +
`measure_document` (shape memo on, as in the worker) + `paint` at 1280x800, on
the static copies (no JS, no images), median of 5 warm repeats, release build,
machine load 9-20 from other agents' builds (wall times are inflated, the
ratios hold):

| page | elements | restyle (build) | relayout (memo) | paint |
|---|---|---|---|---|
| github | 2 077 | 14 745 / 17 863 ms (2 runs) | 53-55 ms | 10-14 ms |
| en.wikipedia Main_Page | 2 335 | 1 594 / 2 934 ms | 157-244 ms | 9-21 ms |
| bbc | 2 635 | 1 728 ms | 74 ms | 16 ms |
| guardian | 4 964 | 1 107 ms | 68 ms | 4 ms |
| youtube (skeleton) | 417 | 266 ms | 0.4 ms | 3 ms |
| duckduckgo | 261 | 58 ms | 6 ms | 31 ms |
| lobste.rs | 838 | 54 ms | 15 ms | 3 ms |
| wikipedia.org | 1 196 | 33 ms | 21 ms | 5 ms |
| news.ycombinator.com | 814 | 12 ms | 44 ms | 2 ms |

Conclusions:
- A full restyle per animation frame is impossible on real pages (60 fps =
  16.7 ms; even HN's relayout is 44 ms). Today EVERY style change, including a
  hover, goes through that full rebuild (`App::redraw` -> `build_layout_tree_
  with_resources` -> worker `measure_document`); hover restyles are already
  throttled to one per 150 ms (`HOVER_RESTYLE_INTERVAL`) and wait for the
  pointer to stop when a build took > 50 ms (QB3).
- Headless paint of the whole frame is 2-31 ms here, but QM1c (R30, measured
  in the live app on the same day, 1048x1046 window, JS-built DOM, images)
  found a full `App::redraw` of en.wikipedia Main_Page at 67 ms median
  (`paint()` 55 ms, present ~4 ms) and United_States at 101 ms. So even a
  paint-only full repaint is 10-15 fps on Wikipedia. Animations of paint-only
  properties (opacity, colours, translate) must patch the already-laid tree
  AND repaint only the animated boxes' damage rects, keeping the rest of the
  last frame (the same mechanism QM1g1 needs for video: `Buffer::age()` +
  `present_with_damage`), never rebuild (QAN1d). Layout-affecting animations
  get at most a throttled rebuild, or jump straight to their end state on slow
  pages.
- GitHub's 15-18 s restyle with cached rules is itself a problem (hover,
  JS writes, any restyle) and is not explained here (QAN1h measures it).
- Not measured here: the live app's JS-built DOMs (bigger than these copies);
  for the live redraw/present cost see QM1c in R30.
- CPU risk: GitHub's infinite marquee and DDG's spinner would ask for frames
  forever. Even the paint-only path at 10-30 ms per frame is 30-90 % of a core
  at 30 fps. Needs a frame cap, a stop when the window is hidden/unfocused and
  a user decision on `prefers-reduced-motion` (QAN1-D).

### 5. Timers, rAF and budgets

- rAF is a JS timer due on the next 1000/60 ms boundary of `__scratchy_now`
  (TIMERS_PRELUDE, QY30); the App runs due timers in `about_to_wait` and sleeps
  with `ControlFlow::WaitUntil(js.next_deadline())`. Nothing ties rAF to an
  actual painted frame. CSS animations would be driven from Rust, not JS: they
  need their own next-frame deadline merged into that `WaitUntil`, and a tick
  right after `run_due` so a rAF style write and an animation step land in the
  same redraw.
- The 500 ms `CALLBACK_BUDGET` only applies to JS. The animation engine runs in
  Rust; only the events it fires (`animationend`, `transitionend`, ...) run JS,
  each under the normal budget via `App::js_dispatch`.
- `--js-report --run-timers` has a simulated clock and no style system; CSS
  animations do not exist there. Only the Web Animations JS surface (QAN1g)
  would need the clock (`document.timeline.currentTime` = `__scratchy_now`),
  so `finished` promises resolve in simulated time.
- YouTube already loads `web-animations-next-lite.min.js` (R20/R21), which
  implements `element.animate` with a rAF loop writing inline styles: each
  write is a JS DOM write and therefore a full restyle today. A native
  `animate` changes which code runs there: the polyfill checks
  `document.documentElement.animate([],0)` and for `play currentTime pause
  reverse ...`; a partial native API that passes that check would switch the
  polyfill off (QAN1g risk, measure YouTube before/after).

### How this was measured (reusable; nothing committed)

- Pages: `curl --unix-socket /run/user/1001/wire.sock -H 'X-Wire-Jar:
  qan1-investigator' --request-target <url> http://x/` for the page, every
  `<link rel=stylesheet>` (and one `@import` level) and up to 30 `<script
  src>`; scratchpad `qan1/fetch_pages.py`, counts `qan1/analyze.py`,
  `qan1/hidden_until_anim.py`.
- WPT: `qan1/scan_subset.py`, `qan1/scan_wpt.py` (static scans);
  `scratchy --wpt /home/loupax/src/wpt --subset <73 tests> --expected <empty>
  -v` and `--out` for the PNGs.
- Timing and "active at load": a temporary `#[cfg(test)] mod qan1_exp`
  (`qan1_frame_cost`, `qan1_active_at_load`, run with `cargo test --release
  --lib -- --ignored --nocapture`), deleted afterwards.

### Not verified

- No implementation was tried, so the FAIL->PASS candidate list in section 2 is
  read from the test sources, not run.
- Live app frame cost of my own (R30/QM1c has en.wikipedia only) and the JS-built DOMs of YouTube,
  GitHub, BBC and Guardian; no nested-dwl screenshots were taken (nothing
  visual changed, the only image evidence is the runner PNGs).
- Whether winit reports `Occluded`/focus loss on dwl (needed for the "stop
  animating when hidden" rule).
- Why GitHub's restyle takes 15 s (QAN1h).
- Which woff2 decoder crate fits (QFF1b).

## R30: Media bytes through wire: Range works unchanged; pooled connections skip `public-only` (Opus investigator QM1d/QM1c, 2026-11-17) — EVIDENCE for QM1c/QM1d/QM1e (all done); wire W-POOL1 FIXED (#248); W-POOL2, W-MEDIA1, W-MEDIA2, W-PERF1 OPEN

**Status (audit 2026-11-17, main @ e4993c3):** QM1e (93148a4, media byte channel through wire) and QM1f (242e468, HTMLMediaElement + libmpv backend behind the off-by-default `media` feature) are BUILT; wire W-POOL1 is FIXED (PR #248). Still open: QM1g/QM1g1 (paint, video-rect redraw), wire W-POOL2, W-MEDIA1, W-MEDIA2, W-PERF1. The "nothing implemented" status below is the original one.

**Status: investigation only, nothing implemented.** Verdict: the QM1e media
channel can be built on repeated ranged GETs over today's wire protocol (no
protocol change). One security gap in wire was found on the way (W-POOL1).

Setup (all throwaway, in the session scratchpad, nothing committed): a tiny
Range-capable Python server (`python3 -I`, `ThreadingHTTPServer`, HTTP/1.1
keep-alive, `TCP_NODELAY`, bound to 127.0.0.1 on a free port, killed by PID)
serving a 64 MiB random file and the QM1b test mp4 (7.0 MB, 1280x720 H.264, moov
at the END as ffmpeg writes by default); extra paths `/redir` (302),
`/setck` (302 + `Set-Cookie`), `/norange.bin` (ignores Range), `/gz.bin`
(compresses anyway). Python clients spoke raw HTTP to the user's running wire
(`/run/user/1001/wire.sock`, X-Wire-Version 3), one request per connection, GETs
only. The server logged client port, Range, Cookie and Accept-Encoding of every
upstream request. Other agents were compiling at the same time (8 threads busy),
so absolute times are pessimistic.

What wire does with a ranged request (source: `is_request_header_to_skip`,
`stream_response`, `execute_with_jar`; confirmed live):
| Case | Result through wire |
| --- | --- |
| `Range: bytes=0-1023`, `1000000-1262143`, `N-`, `-4096` | `206`, exact bytes, `Content-Range` and `Accept-Ranges` passed through, `Content-Length` stripped |
| range past the end | `416` + `Content-Range: bytes */67108864` |
| `HEAD` | `200`, no `Content-Length` (stripped), so HEAD cannot give the size: use `Content-Range` |
| plain GET 64 MiB, no caps | `200`, streamed (not buffered), 1.1-1.4 s |
| plain GET 64 MiB + `X-Wire-Max-Bytes: 16777216` | `502` `X-Wire-Error: too-large` after 0.28 s |
| 4 MiB range + 16 MiB cap (+ jar) | `206`, fine (the cap is per response, buffered up to N+1) |
| redirect + Range (shared agent and jar path) | Range re-sent on every hop, `206`, `X-Wire-Final-Url` = target |
| `/setck` + Range with the jar | cookie stored; next ranged GET with the same top site sends `mediack=1`; with another `X-Wire-Top-Site` sends none (partition kept) |
| server ignores Range | `200` with the whole 64 MiB |
| Accept-Encoding | none sent upstream when `Range` is set (ureq `add_accept_encoding` skips it), `gzip` otherwise: ranged bodies are never decompressed behind `Content-Range` |
| `file:` URL + Range | `200`, the WHOLE file (`serve_file` uses `std::fs::read`), `application/octet-stream` for media (W-MEDIA1) |

Latency and throughput, median per request (one new Unix connection each;
upstream connection reused by ureq's pool):
| Request | direct, keep-alive | direct, new TCP | via wire, plain | via wire, jar + caps |
| --- | --- | --- | --- | --- |
| 1 byte | 4.2 ms | 6.2 ms | 9.0 ms (p90 15.4) | 10.1 ms (p90 16.5) |
| 256 KiB | 6.3 ms, 41 MB/s | 9.3 ms, 30 MB/s | 11.7-14.0 ms, 19-23 MB/s | 12.9-13.4 ms, 19 MB/s |
| 1 MiB | 9.4 ms, 106 MB/s | 11.4 ms, 92 MB/s | 25.7-25.9 ms, 37 MB/s | 19.2-19.6 ms, 46-52 MB/s |
| 4 MiB | 16.9 ms, 200 MB/s | 27.5 ms, 145 MB/s | 49-60 ms, 59-83 MB/s | 48-52 ms, 69-75 MB/s |
Sequential and random offsets gave the same numbers (no seek penalty on a
ranged server). Wire adds ~5 ms per request and roughly halves bulk throughput
(8 KiB `copy_body` loop; W-PERF1, optional). A 1080p H.264 stream needs ~1 MB/s:
with 1 MiB chunks that is one request per second, ~2% of wall time, so a
persistent/streaming media endpoint is not worth a protocol change. On a real
remote server the request time is dominated by RTT (+ TLS only for a new pooled
connection); the first-byte delay per chunk is one RTT.

Connection reuse (an open wire/TODO item): ten ranged GETs arrived from ONE
upstream client port per agent (the shared agent and `manual_agent` each keep
their own pool), and a pooled connection was still reused after 65 s idle.
**W-POOL1 (security):** because the policy check and the target class live in
`TrackingResolver`, and ureq only resolves when it opens a new connection, a
reused connection skips both. Sequence on `http://localhost:<port>/`: cold pool
+ `X-Wire-Policy: public-only` -> `403 policy`; one plain request; the same
`public-only` request -> `206`, and no `X-Wire-Target-Class` on any repeat
request (shared agent and jar path alike). scratchy only sends `public-only`
with `private_targets = Block`; in the default Warn mode the effect is a missing
"resolves to a private address" warning on repeat requests.

Proof of concept, libmpv fed only by wire (throwaway ctypes script, `vo=null`,
`ao=null`, `config=no ytdl=no load-scripts=no osc=no input-default-bindings=no
load-unsafe-playlists=no`): `mpv_stream_cb_add_ro("scratchy-media")`, read_fn
served from a 1 MiB chunk cache, a miss = one ranged GET through wire with the
jar headers and `X-Wire-Max-Bytes`. Timeline: open -> GET 0-1 MiB (61 ms) ->
mpv seeks to 6978642 (the moov at the end, 7446 bytes) -> back to 48 ->
file-loaded 0.27 s after open, first frame (playback-restart) 0.54 s after open;
seek 12.5 -> two 1 MiB GETs (20, 24 ms) -> playback-restart at 12.533 0.78 s
later (decode from the previous keyframe: x264 default GOP 250 = 8.3 s); seek back
to 1.0 -> playback-restart 1.033. 9 requests / 8.4 MB for a 7.0 MB file with two
seeks; mpv asked read_fn for 131072 or 65536 bytes per call (79 calls), so a
per-stream chunk cache is needed. strace is not installed, so "no connect()
from mpv" was NOT checked (no network URL was ever given to mpv, only
`scratchy-media://1`).

QM1c, full-page repaint cost (temporary `Instant` timing around the fill,
`paint()` and `present()` in `App::redraw`, release build, reverted right after
the build; nested dwl from tools/nest.sh, window 1048x1046; live pages through
wire; wheel scrolls from tools/inject; rows after the page finished shaping):
| Page | redraws | total median / p90 / max | `paint()` median | white fill | present median / max |
| --- | --- | --- | --- | --- | --- |
| en.wikipedia Main_Page | 27 | 67.1 / 81.5 / 100.1 ms | 55.2 ms | 1.0 ms | 3.9 / 10.8 ms |
| en.wikipedia United_States | 28 | 101.2 / 107.6 / 109.8 ms | 89.2 ms | 1.0 ms | 4.1 / 7.9 ms |
About 7 ms per frame sits between the fill and `paint()` (the QW6 canvas-colour
blend over the whole buffer; not timed on its own). Load average was 15-21 on 8
threads (other agents compiling), so the numbers are upper bounds, but even at
half they are over a 33 ms (30 fps) budget and the redraw runs on the main
thread. Verdict: a video frame must not repaint the page; QM1g1 (keep the page
pixels, blit the video rect, `present_with_damage`; softbuffer 0.4.8 has
`Buffer::age()` and `present_with_damage`). United_States was blank in the text
column for the first ~45 s while shaping (screenshot), then complete; those
early redraws were excluded.

Recommendation (ranked; details and tests in TODO.txt QM1e):
1. Repeated ranged GETs, one wire connection each, 1 MiB chunks (4 MiB while
   reading sequentially), always with `Range` and `X-Wire-Max-Bytes: CHUNK`,
   validated against `Content-Range`; forward-only streaming fallback for
   servers that ignore Range. No wire protocol change.
2. (rejected for now) a streaming media endpoint / long-lived connection in wire:
   saves ~5 ms per request (<1% at 1 MiB chunks), costs a protocol version.
3. (rejected) a `X-Wire-Range` control header: the real `Range` header already
   works end to end, including redirects and the jar.
wire items filed (wire/TODO.txt): W-POOL1 (fix first, independent of media),
W-MEDIA1 (`file:` Range + media types; QM1e needs it for local media pages),
W-MEDIA2 (truncated bodies look complete; QM1e works around it with
Content-Range), W-PERF1 (optional throughput).

NOT verified: a real remote (TLS, CDN) media server and its RTT; mpv over a
slow link (stalls, `cancel_fn`); webm/vp9/av1; audio (every run used
`ao=null`); that mpv makes no connect() (no strace); a server that answers a
range with `Content-Encoding: gzip` (the test server compressed a slice, which
is not what real servers do); mixed-content and CORS behaviour of real media
sites.

## R29: new errors after QY34 (script-inserted scripts) are newly exposed gaps, plus a budget-trip amplifier in `drain_jobs` (Opus investigator QY44, 2026-11-17) — MOSTLY FIXED (QY44a-c done); QY44d, QY44e open

**Status (audit 2026-11-17, main @ e4993c3):** QY44a (budget trip no longer kills the drain), QY44b (`window.screen`) and QY44c (= QSD4a, CSSOM insertRule) are DONE. Open: QY44d (media textTracks placeholder), QY44e (blob:/data: script src).

**Status: open (no QY34 regression; fix forward, do not revert).** The QY34 agent
reported more errors after 281d281 (old.reddit 2 -> 103, guardian 0 -> 2, bbc a
different next.js exception, one YouTube run with 86 errors). Each was
reproduced and traced to the API the newly running code hits:

| page | before -> after (headless) | what the new code hits | QY34's fault? | item |
|---|---|---|---|---|
| old.reddit (cached "Welcome to Reddit" copy) | 1+2 -> 1+103 errors, 1 -> 6 requests | `window.screen` undefined: `Jp={height:window?.screen.height,...}` at module level in the `js/concat?...BMlnvMqNfR,...` chunk; every dependent SML module rejects (101x) | no | QY44b |
| guardian /international | 0 -> 2 timer errors, 10 -> 45 requests | `video.textTracks` undefined (SelfHostedVideo island, `3030.client.web...js:1:15283`); window also: `document.styleSheets.length` (emotion speedy, `frameworks.client.web...js:2:1107`) | no | QY44d, QY44c |
| bbc.com | 1 -> 1 timer error, 1 -> 11 requests | the SAME styled-components failure as before (`document.styleSheets`/`style.sheet` undefined, `13ad5orspgznk.js:1:13734`); before, the `/_error` route chunk never loaded ("Route did not complete loading"), now it loads and dies the same way | no | QY44c |
| youtube (cached copy) | 0 -> 85-86 errors (84x ``Error: sb`kCcqO``) in 4/4 plain runs at load 4-25 | a CPU-budget trip in one promise job kills every LATER job of the drain mid-way; kevlar's signal library loses its `finally` and stays "inside a reactive context" | no (pre-existing engine behaviour; QY34 adds CPU: player base.js, lazy chunks) | QY44a (+ QY33) |
| youtube (fresh copy) | 1 -> 2 trips, 0 `sb` errors in 4 runs | player `base.js` (3 MB) now executes | no | - |

QY34's own behaviour checked with a fixture (`qy34_semantics.html`, headless,
real wire): inline inserted script runs synchronously with
`document.currentScript` = itself, its exception is logged (`inline script
(inserted): Error: ...`) and the inserting script continues; a moved script is
not re-run; `src` runs later (async) and `load` fires after it ran
(`importShim` defined in onload); a 404 fires `error` and logs `failed to load
script (HTTP 404)`. No wrong order, no load-before-run, no swallowed errors.

### How this was measured (reusable; nothing committed)

- Binaries: this worktree's release builds of e6ad8b3 (QY34 merge, "after") and
  c17fb97 (its first parent, "before"), each with a LOCAL never-committed
  `COOKIE_JAR_NAME = "qy44-investigator"` (net_policy.rs). The report still
  prints `jar: scratchy` because that label is hard-coded (QY37 note); the
  wire header carried the investigator jar. A third local build (reverted)
  logged the stack of every budget-tripped job/callback (js.rs `drain_jobs`
  and `eval_value`, `QY44EXP ... trip stack`).
- Pages: fresh copies via `curl --unix-socket /run/user/1001/wire.sock -H
  'X-Wire-Jar: qy44-investigator' --request-target <url> http://x/` with a
  `<base href>` (guardian /international, www.bbc.com/, www.youtube.com/);
  old.reddit now answers wire with a 403 "blocked by network security" page
  and www.reddit.com with a JS challenge form, so the QY34 agent's cached
  copies (scratchpad `pages/https___old_reddit_com_.html`, 2026-11-16 23:53,
  and `pages/https___www_youtube_com_.html`) were reused. Reddit's concat
  chunks still answer scratchy (200) but 404 to a bare curl (headers differ;
  not investigated).
- Runs: `scratchy --js-report --run-timers 5000 --socket
  /run/user/1001/wire.sock copy.html`; probes injected right after `<base>`:
  a `__net_request` wrapper printing the reddit chunk around line 42 col
  67620; a `__scratchy_run_global` / `__mod_register` wrapper listing every
  inserted script run (YouTube: botguard `th/...js` 64 KB, player `base.js`
  2.7-3.0 MB 0.4-1.3 s wall, two kevlar lazy chunks); a `__net_request`
  filter blocking botguard (sb errors still in 1 of 2 runs).
- Windows: `tools/nest.sh` + `grim`, one scratchy window at a time on the
  file copies, 45-120 s. Machine load 4-25 (other agents' builds).
- Fixtures (scratchpad `qy44/fx/`): `trip_finally.html`, `trip_jobs_big.html`,
  `styledc.html`, `texttracks.html`, `screen.html`, `qy34_semantics.html`.

### 1. YouTube `sb\`kCcqO`: a trip kills the rest of the drain mid-way (QY44a)

kevlar `J3e` (signal write): ``ui[0]?.consumerAllowSignalWrites!==!1||Ko_(q)``,
`Ko_ = q => { throw Error("sb`" + q.debugName) }`; `kCcqO` is the
`disableTemplateUpdate` signal written by every `JSC$19014_renderImpl` (stack:
`moNotify` -> `N1W` -> `renderImpl` -> `AvP` -> `J3e` -> `Ko_`). A
MutationObserver microtask starts with an empty consumer stack in any browser,
so `ui[0]` must be a stale consumer left by an aborted computation.

Trip stacks (local logging build, cached copy): the second drain logs
`promise jobs stopped ... (5 jobs dropped)`; its jobs were interrupted in
`rootId`/`composedRootId` (`ceIsConn`, `ceScan`, `__d_insert`), `ioShown`,
an insert hook and, last, inside kevlar `producerRecomputeValue` -> `MF`; the
84 `sb` errors follow immediately. Mechanism, both parts shown with fixtures:

- QuickJS's interrupt is uncatchable and unwinds without running `catch` OR
  `finally` (quickjs.c, interpreter `exception:` label: the
  `JS_TAG_CATCH_OFFSET` scan is skipped for uncatchable errors).
  `trip_finally.html`: a timer `push; try { for(;;); } finally { pop }` leaves
  the stack at length 1. Unavoidable for the one job that really overruns.
- `drain_jobs` keeps executing queued jobs after a trip while the deadline is
  already expired, so EVERY following job (an unrelated page task) dies at its
  first interrupt poll. `trip_jobs_big.html` (job1 `for(;;)`, job2/job3 a
  200 000-iteration loop inside try/finally): main prints `jobs completed: []`,
  stale stack `[job2,job3]`, `(3 jobs dropped)`. With 2 000 iterations the
  jobs finish before the next poll and survive, which is why the damage is
  random.

Why only since QY34: before, the cached copy ran with 0 trips (0 errors, 99
requests); with QY34 the player `base.js` and kevlar's lazy chunks execute in
the same session and the timeline gets the extra CPU (trip stacks show mostly
scratchy prelude walks: QY33 note). 4/4 plain runs gave 84 `sb` errors at
load 4-25; with the run-listing probe 0/2 (timing). The fresh copy (different
build of the page) tripped twice per run and never hit a signal `finally`.

### 2. old.reddit: `window.screen` (QY44b)

Probe output, chunk `js/concat?prefix=en-US/&chunks=BMlnvMqNfR,C9D1KpOVTC,DGPisoaFEk`
line 42 around col 67620: `...,Jp={height:window?.screen.height,width:window?.screen.width},Yp=...`.
`screen.html`: `typeof screen` = undefined, `visualViewport` undefined. The
pre-QY34 `'width' of undefined` (data: module `buildEvent`:
`width:window.screen.width`) is the same gap. Also seen, harmless: the
always-loaded es-module-shims polyfill probes with `<script src=blob:...>`
(blocked: QY34's fetch path does not do blob:/data:) and finds no
`HTMLScriptElement.supports` (QY44e).

### 3. bbc / guardian: CSSOM and media placeholders (QY44c, QY44d)

`styledc.html`: `typeof document.styleSheets` = undefined, `style.sheet` =
undefined, `document.styleSheets.length` throws. BBC: styled-components'
`eq` sheet constructor (`e.sheet` or a `document.styleSheets` scan, else
`throw e_(17)`) runs on first render: the console.error `'length' of
undefined` + next.js "client-side exception" are in BOTH builds; QY34 only
lets the `/_error` route chunk load (headless) so the final error moves from
"Route did not complete loading" to the same TypeError. Guardian window
(120 s, after): emotion speedy `document.styleSheets.length` in an island.
`texttracks.html`: `textTracks`/`audioTracks`/`videoTracks` undefined.

### Screens (nested dwl, file copies, viewed)

- old.reddit (45 s): before and after pixel-identical: Reddit logo top left,
  a grey speech-bubble background pattern, no login form (shadow content is
  not rendered, QSD6).
- guardian (50 s; after also 120 s): pixel-identical: blue "You are viewing the
  International homepage / View the Europe homepage" bar with an X, "News"
  heading, the Live headline with a large protest photo, a row of four links,
  then two story cards (Nvidia, Microsoft). No Guardian masthead/logo at the
  top in either.
- bbc (50 s): same page in both (empty grey ad slot, BBC blocks, Home/News
  nav, three story columns); BEFORE has BBC's own dark cookie banner at the
  bottom ("Let us know you agree to cookies", "Yes, I agree", "No, take me to
  settings"); AFTER has no banner. After, the Sourcepoint CMP (inserted
  `wrapperMessagingWithoutDetection.js`) runs ("Messaging without detection
  successfully executed"); its message is not visible. Why the static banner
  goes away was not traced.
- youtube fresh copy (75 s): before and after the same: hamburger, logo + DE,
  wide search field, kebab, "An..." pill at the edge, mini guide with 4 icons
  and labels, the "Der YouTube-Verlauf ist deaktiviert" card with the
  overprinted subtitle line (QY43).

So: no page renders worse except the BBC consent banner change (arguably a
behaviour change, not a breakage); none renders better yet either, because
the code QY34 unlocks dies on QY44b/QY44c/QY44d.

### Not verified

- QY44a-QY44e fixes were not implemented locally (only the trip-stack logging).
- Whether YouTube's other lost work (kebab/icon jobs, QY33) also comes back
  with QY44a alone.
- Why reddit's concat chunks 404 to curl but 200 to scratchy, and whether the
  101 module rejections are the only shreddit failure (the login form needs
  shadow DOM rendering anyway).
- Why the BBC static cookie banner disappears with the CMP running, and
  whether a cross-origin consent iframe should be shown.
- Window logs show `wire answered 400 to the cookie control request` with a
  non-default jar name in both builds (also in R28); not investigated (QY37
  note).
- Other measured pages (wikipedia, HN, github, ddg, lobste.rs) were not re-run.

## R28: YouTube's remaining masthead/guide paint gaps: `document.all`, a 1e6 px max-content, pre-wrap newlines (Opus investigator QY35, 2026-11-17) — FIXED (QY40-QY43 done); the kebab icon is QY33 (open)

**Status (audit 2026-11-17, main @ e4993c3):** QY40 (fefde2b, `document.all`), QY41, QY42 and QY43 are all DONE. The one remaining gap (section 4, kebab icon dropped by the callback budget) is QY33, still open.

**Status: open (bugs, no regression).** QY35 asked for the cause of each
paint gap R26 listed once QY29/QY24/QY32 were merged. Measured on main
6249835 (QY29 + QY24 + QY32 merged; QY30/QY31 merged after the build, they
change only rAF due times and the headless report). Short answer:

| gap (R26) | on main | cause | item |
|---|---|---|---|
| no logo graphic | NOT reproduced: logo paints | - | - |
| search field a stub | yes | `width:100%` inline-block in the 40 px kebab button => max-content 1e6 => `#end` 1 000 024 px, `#center` 0 px | QY41 |
| no "Anmelden" | yes (at x = 1 000 129) | same as above; then clipped by 26 px (definite width ignored) | QY41, QY42 |
| mini-guide icons `display:none` | yes | no `document.all` => polymer-resin sets `hidden = "zClosurez"` | QY40 |
| subtitle overprint | yes | `measure_mixed_inline` never breaks at a preserved "\n" | QY43 |
| (new) kebab icon missing | yes | 500 ms callback-budget trip drops YouTube's icon job | QY33 note |

### How this was measured (reusable; nothing committed)

- Page: `curl --unix-socket /run/user/1001/wire.sock -H 'X-Wire-Jar:
  qy35-investigator' --request-target https://www.youtube.com/ http://x/`
  (kevlar_base es5 variant, as R26), copy with `<base
  href="https://www.youtube.com/">` and an inline probe `<script>` right
  after `<head>`; run as `scratchy --socket /run/user/1001/wire.sock
  copy.html` in `tools/nest.sh` (one window, 50-100 s, `grim`, screenshots
  viewed). Probe output = `console.error` lines on stderr.
- The binary was this worktree's release build with a LOCAL never-committed
  `COOKIE_JAR_NAME = "qy35-investigator"` (net_policy.rs) so the window runs
  did not write into the user's `scratchy` jar (QY37 is only for the
  report). Experiments (all reverted): `CALLBACK_BUDGET` 500 ms -> 30 s
  (js.rs); the QY41 patch (Embedded `%` width = auto in `trial`).
- Probes: (1) a tree dumper: tag/id/class, computed `display/position/
  width/height/min-width/white-space`, `getBoundingClientRect()`, for
  `ytd-topbar-logo-renderer`, `#center`/`#end.ytd-masthead`, the first
  `ytd-mini-guide-entry-renderer`, `ytd-feed-nudge-renderer`; (2) yt-icon
  controller state (`polymerController.icon/iconType/iconShapeData/...`);
  (3) a `window.fetch` wrapper logging icon SVG requests and a rAF counter;
  (4) a wrapped `hidden` setter / `setAttribute` on `yt-icon#icon` with
  `new Error().stack`, mapped back to kevlar.js lines.
- Fixtures (scratchpad): `prewrap.html` (window), `maxc.html` /
  `maxc2.html` / `docall.html` (headless `--js-report`, QJ4 geometry).
- Machine load average 9-12 throughout (other agents' builds).

### 1. Search field stub + no "Anmelden": max-content of 1 000 000 px (QY41, QY42)

Probe on main (t = 50 s):

```
div#center.ytd-masthead   disp=flex  rect=200,8 0x40
  yt-searchbox            disp=flex  rect=240,8 61x40      (overflows #center)
div#end.ytd-masthead      disp=flex  minw=225px rect=200,8 1000024x40
  div#buttons             disp=flex  rect=200,8 1000024x40
    ytd-topbar-menu-button-renderer  rect=200,8 999921x40
      a.yt-simple-endpoint (flex)    rect=200,8 999921x40
        yt-icon-button#button  w=40px rect=200,8 40x40
          button#button  inline-block w=100%  rect=208,16 24x24
    ytd-button-renderer ("Anmelden")  rect=1000129,8 95x40
```

`#end` is `flex:none`, so its width is its max-content, which comes out as
~1e6. Headless fixture, `flex:none; display:flex` wrapper, width read back:

| content | width |
|---|---|
| inline-block 40 px, empty | 40 |
| + absolute children (yt-interaction shape) | 40 |
| + inline-block child `width:100%` | **1 000 000** |
| block 40 px + inline-block child `width:100%` | **1 000 000** |
| inline-block child `width:50%` | **500 000** |
| inline-block child `width:20px` | **20** (should be 40) |
| wrapper not flex (`flex:none` block) | 40 |

Cause: `natural_width_impl` (layout.rs) runs `measure_mixed_inline` with
`UNBOUNDED` (1e6) for the max trial; its `Embedded` arm resolves the
inline-block's `width: N%` against that width (QY41). Separately, the
40 px box's own definite width is ignored because it is not empty and not a
float (`as_child` branch), so its content wins (QY42: 20 instead of 40).

Local QY41 patch (in `trial`, a `%`/calc width is `auto`): the fixture rows
read 40 (the `20px` row still 20); YouTube: `#center` 583 px, search box
543 px, `#end` 225 px; kebab `yt-icon-button` 18 px wide (QY42) so
`#buttons` is 95 px and "Anmelden" (95 px at x = 939) overflows `#end`
(783..1008) by 26 px. Screenshot: wide rounded search field + search
button, three-dot kebab, sign-in pill with a person icon and "An" cut at the
window edge. Media queries are not involved (`@media(max-width:656px)`
without a space parses and does not apply at 1024 px; fixture checked).

### 2. Mini-guide icons `display:none`: no `document.all` (QY40)

`yt-icon#icon.guide-icon` carries `hidden=""` while its entry's
`data.thumbnail` is undefined (`data` keys: navigationEndpoint, icon,
trackingParams, formattedTitle, accessibility, isPrimary). Template:
`<yt-icon id="icon" ... hidden="[[data.thumbnail]]" icon="[[data.icon.iconType]]">`.
The wrapped `hidden` setter received the STRING `"zClosurez"`, stack:
`_setUnmanagedPropertyToNode` <- `U2s` (Polymer binding effect, which calls
the sanitizer `$T(value, target, kind, node)` first) <- `gvv` <-
`_runEffectsForTemplate`. polymer-resin (`XBv`) returns a value unchanged
only `if (!T && T !== document.all)`. Fixture: `typeof document.all` =
"undefined", `'all' in document` = false, `undefined !== document.all` =
false. So for T = undefined resin falls through to the contract lookup and
returns its innocuous string; `hidden = "zClosurez"` => `[hidden]` =>
`display:none`. This hits EVERY falsy (undefined/null/false/0/"") property
binding through resin on every element, not only `hidden`.

Stand-in (page script before everything: `Object.defineProperty(document,
'all', {value: document.getElementsByTagName('*')})`, i.e. a TRUTHY object,
not the real fix): `hidden` stays false and the mini guide paints a house
(Startseite), the Shorts logo, Abos and Mein YouTube icons (viewed). The
real fix needs [[IsHTMLDDA]] (falsy, typeof "undefined"):
`JS_SetIsHTMLDDA` is in rquickjs-sys 0.14 (`quickjs.h`, x86_64 bindings).

### 3. Subtitle overprint: preserved newline on the per-word path (QY43)

The subtitle is `yt-formatted-string#subtitle[split-lines]`
(`white-space: pre-wrap`) with a `<span>` holding "... erhalten.\n\nWenn du
die Einstellung ändern möchtest, ... zuzustimmen. " and an
`<a class="yt-simple-endpoint">` "Weitere Informationen" whose computed
display is `inline-block`. Fixture (window, viewed), 400 px wide pre-wrap
div with the same text:

- A: link without padding: correct (two lines, empty line, last line).
- B: link with `padding:0 2px` (a Spacer item): line 1, a blank band,
  then "erhalten." with the following words painted on top of each other
  and the link right after, then "Wenn" alone on the last line: the same
  picture as YouTube.
- C: an inline-block `<span>` instead of the link: same overprint.
- D: no newline, padded link: correct.

So any paragraph that takes `measure_mixed_inline` (inline-block/padded
inline/`<img>`/float/word-spacing) breaks on preserved newlines. In its
`InlineItem::Text` arm the text is split only at ' ' (`word_byte_ranges`)
and widths are read from the first layout run only
(`word_widths_from_buffer`); a "\n" inside a word is never turned into a
`Measured::Break` (only a `<br>`'s BR_TEXT item is). Code reading; the fix
was not patched locally.

### 4. Kebab icon: dropped by the callback budget (QY33)

`yt-icon` (legacy wrapper, `kevlar_use_wil_icons` = true) resolves an icon
shape and, unless the SVG is in its cache, fetches it
(`https://fonts.gstatic.com/s/i/youtube_outline_experimental/<name>/vN/24px.svg`,
`credentials:"same-origin"`, after one rAF). On main the fetch wrapper saw
exactly three icon requests (search, menu, logo; all 200, text read fine,
`caches` undefined so no Cache Storage path); MORE_VERT and the mini-guide
icons were never requested (their controllers had `iconShapeData` but an
empty `span.yt-icon-shape`). Each run logged one `timer stopped` and one
`promise jobs stopped` (500 ms). With CALLBACK_BUDGET = 30 s (local): ~90
icon SVGs fetched and the kebab paints. With the budget raised AND the
`document.all` stand-in, the mini-guide icons paint too.

### 5. Logo: not reproduced

`ytd-logo > yt-icon#logo-icon` (93x20) > `span.yt-icon-shape` > `div` >
`svg#yt-ringo2-svg_yt6` with 7 paths: rect 72,18 93x20; screenshot shows the
red play button + "YouTube" wordmark + "DE". (R26's build lacked QY24/QY32
as merged code; not bisected which one fixed it.)

### Screens (viewed)

- main, 58 s: hamburger, logo, "DE", a tiny rounded search stub + search
  button at x ~280-325, nothing on the right; nudge card with the
  overprinted subtitle line; no mini guide yet (it stamps ~60-80 s in).
- main, 85 s: + mini-guide labels Startseite/Shorts/Abos/Mein YouTube, no
  icons.
- budget 30 s: + three-dot kebab at x ~222 (left, because `#end` starts at
  200 and is 1e6 wide).
- budget 30 s + `document.all` stand-in: + mini-guide icons.
- + local QY41 patch: wide search field, kebab and "Anmelden" pill at the
  right edge (clipped by ~26 px).

### Not verified

- QY40/QY42/QY43 fixes were not implemented locally (QY40: page stand-in
  only; QY43: code reading + fixture); QY41 was.
- Whether the kebab icon also paints with only a faster machine (no
  budget change) or after QY33's profiling.
- Layout viewport is 1024 px (`innerWidth`, `documentElement.clientWidth`)
  while the grim shot of the nested window is 1052 px wide, and the clipped
  pill drew past x = 1024: not investigated.
- Other `"zClosurez"` casualties of QY40 beyond the mini guide (not
  enumerated); YouTube logged-in, /watch and /results not tried.
- The wire cookie-control request answered 400 in these runs (`document.cookie`
  reads ''); probably the file-copy origin or the local jar name; not
  investigated.

## R26: YouTube is still a grey page: `Node.prototype.data` makes polymer-resin blank every renderer's data (Opus investigator, 2026-11-17) — MOSTLY FIXED; QY33, QY36, QY37, QY38 open

**Status (audit 2026-11-17, main @ e4993c3):** DONE: QY29 (7b48596, the blocker), QY30, QY31, QY32, QY34, QY35 (became R28), QY39a/b (User-Agent override). Open: QY33 (post-load callback budget, measure first), QY36 (getComputedStyle percentages), QY37 and QY38 (`--js-report` jar and stylesheets).

**Status: open (bugs and missing features, no regression).** User question:
"why is youtube still a gray page? I don't expect videos to play, but I'd
expect to see more UI." Answer: since QY21/QY22 the app script loads without
the 5 s trip and the app shell is stamped, but NO Polymer renderer ever
receives its `data`, so the home page is never rendered and the static grey
skeleton is never removed. Cause: scratchy defines `data` on
`Node.prototype` (every element has a `data` property). YouTube's
polymer-resin sanitizer (kevlar `XBv`) checks `name in <a plain element of
that tag>` for every property binding; for `data` that is true here (false
in browsers, where only CharacterData/`<object>`/`<data>` have it), so it
replaces every bound `data` value with its innocuous string `"zClosurez"`.
With that one property moved (stand-in), the browse page renders, the
skeleton is removed and the logged-out nudge appears. Two more paint bugs
then hide the masthead and the guide (QY32: a `position: fixed` box inside
an absolutely positioned box is dropped by layout) and the nudge title
(QY24: classList on `<yt-formatted-string>`).

### How this was measured (reusable; nothing committed)

- Page: `curl --unix-socket /run/user/1001/wire.sock -H 'X-Wire-Jar:
  qy28-investigator' --request-target https://www.youtube.com/ http://x/`,
  3 fetches 01:01, all three `kevlar_base` (10613 KB app script, the user's
  variant; no `base_module_set` this time), copy with `<base
  href="https://www.youtube.com/">` after `<head>`. Probes are inline
  `<script>`s after `<base>` that log with `console.error` from timers
  (element counts, custom element tags, `ytd-app` textContent, a full
  `outerHTML` dump at the end). Stand-ins (scratchpad only) are inline
  scripts in the copy; experiments are LOCAL, never-committed source
  patches in the investigator worktree (reverted).
- Builds, in this order: (a) the installed main binary (00:48); (b)
  41b07ea + a local QY21(1) patch (define() queries only the new name), as
  R24 did, because QY21 was not merged when this started; (c) main ee6240d
  after QY21 (959cd39), QJ4 (674231c), QJ5 and QY22 (ecd463a) merged
  mid-investigation; QY23 (5fee1a6) and QJ7 merged after (c) was measured.
- Temporary instrumentation (local only): per-eval and per-timer-callback
  thread-CPU ms, every global `__*` native wrapped in a JS timer, env
  overrides for SCRIPT_BUDGET/CALLBACK_BUDGET, a histogram of the timer
  callback sources, `ceReport` printing `String(e)` + tag + args.
- Window: `tools/nest.sh` + `grim` (screenshots viewed), one scratchy at a
  time, 60 s after start. Static check of the post-JS DOM: the probe's
  `outerHTML` dump, scripts stripped, rendered with `--no-js` (same
  stylesheets through wire).
- Machine: 8 cores, load average 9-21 (other agents' builds) throughout.

### Measurements

Load phase (`--js-report --run-timers`, page-load errors):

| build | load trips 5 s? | kevlar eval CPU | notes |
|---|---|---|---|
| (a) installed main | yes, 1/1 | - | 595 elements, 4 custom tags, 17 timer callbacks |
| (b) QY21(1) only | 2 of 3 runs trip, 1 of 3 not | 2139 ms (compile-only pass 1080 ms) at one time; 5035 ms (trip) / 6082-6679 ms with budget raised (compile-only 2676-2678 ms) at another | SAME binary, SAME page: thread-CPU cost per run varied ~2.5x with machine load (SMT/frequency contention), so a CPU-time budget is not machine-independent |
| (c) main ee6240d | no, 2/2 headless + 3/3 window | - | QY21 + QY22 removed the load wall |

Natives after load with (b): `__d_query` 1038-1647 calls, 372-1054 ms; the
`dom-module,array-selector,...` ce selector alone 53-383 calls, 25-582 ms;
after 5 s of timers 3101 calls 2613 ms (ce selector 1344 calls 2107 ms):
that is what QY21(2) (merged) removed.

Timer callbacks with main (c), CALLBACK_BUDGET raised to 60 s to see them:
first callback after load 1416 ms (764 ms in a later run), then 311, 684,
646, 505, 518, 501 ms (thread CPU). With the default 500 ms budget: 1-2
`timer stopped` + 2-4 `promise jobs stopped` lines per load (QY22's one-line
form; no cascade, no `out of memory`). QJ4's forced layout logged `11 forced
layouts, 1198 ms` headless and `2 forced layouts, 1511 ms` / `1 forced
layouts, 1130 ms` in the window.

The first ~2000 timer callbacks run ~500 different callbacks; the web
animations polyfill (`web-animations-next-lite.min.js`, rAF loop `function
d(q){...u._animations...}`) starts a rAF loop once renderers stamp.

### The chain, in the order it blocks the UI

1. (DONE, QY21+QY22) 5 s load trip. Gone on main (c).
2. **`data` on every element -> polymer-resin -> `"zClosurez"` (QY29).**
   Evidence: on (b)+60 s budgets, `ytd-browse.data` has the whole browse
   response (`responseContext,contents,header,...`), the bound child
   `ytd-two-column-browse-results-renderer` (template binding
   `data="[[data.contents.twoColumnBrowseResultsRenderer]]"`, node 12 of the
   browse template) has `polymerController.__data` with every OTHER bound
   property (`pageSubtype`, `isConstrainedList`, ...) but no `data`;
   re-assigning `browse.data` from a probe set the child's `data` to the
   string `"zClosurez"`. kevlar: `Uog` creates the polymer_resin policy
   whose `createHTML` returns `"zClosurez"` (`INNOCUOUS_STRING`); `XBv`'s
   `case "property": if (t in O) {...}` with `O =
   document.createElement(tag)` (or `polyresinuncustomized` for custom
   elements). In scratchy `'data' in document.createElement('div')` and
   `in` an HTMLUnknownElement are `true` (`Node.prototype.data`, js_dom.rs
   `defs(NodeProto, 'data', ...)`). Element.prototype also carries
   `checked`, `disabled`, `required` (browsers: only on form-control
   interfaces), the same trap for `disabled="[[...]]"` bindings.
   Stand-in (move the descriptor to CharacterData/Text/Comment): the
   skeleton is removed (`#home-page-skeleton` gone), `ytd-rich-grid-renderer`
   is stamped, the masthead gets "Navigation überspringen", "Anmelden";
   with geometry (fake stand-in on (b), or QJ4 on main in the window) the
   `ytd-feed-nudge-renderer` appears with the text YouTube serves this
   logged-out EU visitor.
3. **QY30: rAF loop spins at one instant in `--js-report`.** `add()` in
   TIMERS_PRELUDE: `delay = FRAME - (t % FRAME)` with FRAME = 1000/60; at
   t = 100 ms, `100 % FRAME` = 16.666666666666657, delay 1.1e-14, `t +
   delay` rounds to 100: the polyfill's rAF loop ran 200 040 callbacks at
   simulated 100 ms (sampled every 20000: always `raf 100 function
   d(q){...}`), so the report never got past 100 ms. Not reproduced live
   (real clock), not ruled out either.
4. **QY31: `--js-report` stops pumping after one tripped callback.**
   On main the timer phase ends at simulated 51-101 ms (34-44 callbacks of
   20000 ms) because `run_due_counted` returns 0 when its first eval trips
   and the pump treats 0 as "no due timer" (`if n == 0 { break }`).
5. **QY24 (existing, now confirmed visible):** 3x `cannot read property
   'split' of undefined at classes` on `<yt-formatted-string>` in the window
   (main c). With it, the nudge title keeps `is-empty` (CSS
   `yt-formatted-string[is-empty]:not(.use-shadow){display:none}`) and its
   `<yt-attributed-string hidden="true">`; with a local fix (read and WRITE
   the `class` attribute) the title is plain text and renders.
6. **QY32: `position: fixed` inside an absolutely positioned box is never
   laid out.** YouTube: `ytd-app` is `position:absolute`, the masthead
   (`#masthead-container.ytd-app{position:fixed;top:0;width:100%;
   z-index:2020;transform:translateY(0)}`) and `ytd-mini-guide-renderer`
   (fixed, z 2028) are inside it. Fixture (viewed): fixed boxes inside a
   static / relative parent paint, inside an absolute parent they vanish.
   `resolve_positioned` (layout.rs) measures the abs box with a fresh
   `nested_pending`; `resolve_or_bubble_positioned` always bubbles `fixed`
   entries, so they end up in `nested_pending`, which is only
   `debug_assert!`ed empty: release builds drop them (debug builds would
   panic). A local patch resolving them against the viewport made the
   fixture and YouTube's hamburger, search button and mini-guide appear.
7. Callback budget (QY33): see the numbers above; the window still painted
   the nudge with 1-2 trips, what the dropped work was is unknown.
8. Not blocking the home page: dynamically inserted `<script>` elements
   never run (QY34; YouTube inserts 3: the lazy kevlar chunk
   `exm=...m=QlbBce,De8mUc` (518 bytes), botguard `www.google.com/js/th/...`
   and the player `base.js`; adding the chunk statically changed nothing on
   the home page); `GET https://youtube.com/` CORS-blocked (QY27 asked:
   it is the apex URL, wire follows its 301 to www, the crossed-origin
   check compares `Origin: null` with ACAO `https://www.youtube.com`;
   ~1 MB fetched, nothing waits for it; a browser would fail it too because
   the 301 has no ACAO); 6 `googlevideo.com/videoplayback?itag=18` fetch()es
   (403 / upstream error; the player's prefetch, not a `<video>`); one POST
   `/youtubei/v1/feedback` 400 once the nudge renders.

What Chrome shows for this visitor: `ytInitialData` holds no videos; its
only content is a `feedNudgeRenderer` ("Der YouTube-Verlauf ist
deaktiviert", subtitle, "Weitere Informationen", button "Einstellung
aktualisieren") plus masthead and guide. So "more UI" = masthead, guide and
that card; videos would not appear even in Chrome without watch history.

### Screens (viewed)

- main (c), real URL, 60 s: white page, a thin empty masthead strip (two
  light horizontal lines), two big grey thumbnail boxes with a grey avatar
  circle and two grey text bars: the static `#home-page-skeleton`; no
  logo, no text. Same on (b) at ~20 s and ~28 s (that run: 2954
  `interrupted` promise-job lines in 36 s, 27 s CPU, RSS 444 MB, before
  QY22).
- Post-JS DOM of a stand-in run rendered `--no-js` without any stylesheet:
  "Zurück", hamburger, a huge YouTube logo, "DE", "Navigation
  überspringen", search and kebab icons, a huge account icon: the DOM
  content exists.
- Same DOM with YouTube's CSS: blank white except a search-box frame while
  the title still had `is-empty`; after the QY24 local fix: the nudge card
  (bold title, grey subtitle, blue "Weitere Informationen", grey pill
  button "Einstellung aktualisieren", thin border), nothing else.
- main (c) + local QY29/QY24/QY23/QY30 patches, real URL, default budgets:
  the grey skeleton is GONE, the nudge card is painted; no masthead, no
  guide. Adding the local QY32 patch: top-left hamburger, "DE" (no logo
  graphic), a collapsed search field stub with a search button, mini-guide
  labels "Startseite", "Shorts", "Abos", "Mein YouTube" without icons, no
  "Anmelden" button; the nudge subtitle has one line where words overprint
  ("relevanten Videos zu erhalten." followed by overlapping text, then
  "Wenn" alone on a line before the link). A plain `white-space: pre-wrap`
  fixture with the same text renders correctly, so that glitch is not a
  plain pre-wrap bug (QY35 bisects it).

### Other findings

- wire's User-Agent is `wire/0.1 (+https://github.com/Loupax/vibe-it)
  scratchy's real-networking daemon` (wire/src/main.rs); YouTube classifies
  it `cbrand=robot, cmodel=bot or crawler` (ytcfg `DEVICE`) and serves the
  ES5 bundle with `webcomponents-all-noPatch.js`, `fetch-polyfill.js`,
  `intersection-observer.min.js`. Everything above is about that variant.
  Whether to send a browser-like UA is a product question (QY39-D).
- `getComputedStyle` serialises percentages as fractions: `width:50%` ->
  `"0.5%"`, `100%` -> `"1%"` (QY36).
- `--js-report` sends the user's cookie jar name `scratchy`
  (`COOKIE_JAR_NAME`, net_policy.rs; report lines say `jar: scratchy`), so a
  diagnostic run writes into the user's in-memory wire jar (QY37). This
  investigation's headless and window runs did that for youtube.com.
- `--js-report` does not load `<link rel=stylesheet>`: getComputedStyle and
  QJ4's forced layout there see only UA + inline styles (ytd-app computes
  `display:inline` headless, `block` with the CSS inlined), so headless
  geometry differs from the window: with QJ4 on main the headless run did
  NOT stamp the nudge while the window did (QY38).

### Not verified

- Which of the four local patches the window needed: they were measured
  together (QY29 alone headless shows the grid; the live rAF effect is
  unknown).
- The QY32 local patch resolves nested fixed boxes against the viewport at
  the abs box; paint order/stacking against siblings was not checked.
- Logo, search width, sign-in button, mini-guide icons, subtitle overlap:
  causes not bisected (QY35).
- No Chrome on this machine: "what Chrome shows" is from ytInitialData.
- Logged-in YouTube, /watch, /results, `base_module_set` variant, and a
  browser UA were not tried. Video playback is out of scope (placeholders
  only); no `<video>` element exists on the home page.

## R27: Can scratchy embed mpv for `<video>`? (Opus investigator, 2026-11-17) — EVIDENCE for QM1; design DECIDED (QM1-D D1-D8), QM1b/QM1e/QM1f built

**Status (audit 2026-11-17, main @ e4993c3):** The media design is COMPLETE (QM1-D D1-D8, 2026-11-17): libmpv in-process, optional `media` feature, all bytes through wire, no MSE for now. Built: QM1b spike (6cab99c, tools/mpvspike), QM1e (93148a4), QM1f (242e468). Open: QM1a, QM1g, QM1g1; MSE is QM1k (with the user). The "investigation only" status below is the original one.

**Status: investigation only, nothing implemented (media is "Off limits" in
AGENTS.md until the user agrees a design).** Conclusion: an mpv *process*
cannot draw inside scratchy's window on Wayland; libmpv *inside* scratchy can,
by rendering each frame into a buffer scratchy owns. Recommendation and the
user's open questions are in TODO.txt QM1.

What is installed (checked 2026-11-17):
- mpv 0.41.0, libplacebo 7.360.1, FFmpeg 8.1.2. mpv build flags (from `mpv -v`):
  `x11=disabled`, `vaapi=disabled`, `vdpau=disabled`, `pipewire=disabled`,
  `pulse=enabled`, `wayland=enabled`, `vulkan=enabled`, `egl=enabled`,
  `zimg=disabled`, `libmpv=true`.
- libmpv: `pkg-config --modversion mpv` = 2.5.0, `/usr/lib64/libmpv.so.2.5.0`
  (2,423,368 bytes), headers `/usr/include/mpv/{client,render,render_gl,stream_cb}.h`
  (`MPV_CLIENT_API_VERSION` 2.5). render.h has `MPV_RENDER_API_TYPE_SW`
  (formats rgb0/bgr0/0bgr/0rgb, stride and pointer 64-byte aligned
  recommended, "extremely simple (but slow)") and `MPV_RENDER_API_TYPE_OPENGL`.
  stream_cb.h: `mpv_stream_cb_add_ro(protocol, open_fn)` with read/seek/size
  callbacks.
- `mpv --vo=help`: gpu-next, gpu, wlshm, dmabuf-wayland, libmpv, null, image,
  drm, kitty, tct. `--gpu-context=help`: waylandvk, wayland (EGL), drm,
  displayvk. `--hwdec=help`: only v4l2m2m-copy (no VA-API in mpv or ffmpeg:
  `--disable-vaapi`), so decoding is software on this machine.
- ffmpeg decoders: h264, hevc, vp8, vp9, av1 (native and libdav1d), aac, opus
  (native and libopus), vorbis, mp3, flac.
- ffplay 8.1.2 (SDL2 window, keyboard only, no control socket). vlc,
  gst-launch-1.0: not installed. Audio: `pactl info` = PulseAudio on PipeWire
  1.6.7; mpv `--ao=help`: pulse, alsa, null, pcm.
- CPU: AMD Ryzen 5 PRO 2500U, 8 threads.
- dwl 0.8-dev on wlroots 0.19. Globals it creates (symbols in /usr/bin/dwl,
  confirmed by mpv's registry log in a nested dwl): wl_compositor 6,
  wl_subcompositor 1, wp_viewporter 1, wp_single_pixel_buffer 1,
  wp_presentation 2, wp_fractional_scale 1, xdg_wm_base 6, xdg_activation,
  idle_inhibit, cursor_shape, layer_shell, xwayland, linux_dmabuf (created
  with the renderer). Not offered: xdg-foreign, color management, content-type.
  The nested pixman dwl from tools/nest.sh does NOT offer zwp_linux_dmabuf_v1
  v4 (`mpv --vo=dmabuf-wayland`: "Compositor doesn't support the
  zwp_linux_dmabuf_v1 (ver. 4) protocol!", VO init failed).
- scratchy: winit 0.30 + softbuffer 0.4, one wl_surface, no EGL/GL. `App::redraw`
  (src/app.rs) fills the buffer white, repaints the whole page and calls
  `present()` (no damage) on every redraw.

Tests run (throwaway files in the session scratchpad, nothing committed; every
mpv with `--no-config --ao=null`; windows only in a nested dwl from
`tools/nest.sh`, killed by its PID afterwards):
1. Test media: `ffmpeg -f lavfi -i testsrc2=size=1280x720:rate=30 -f lavfi -i
   sine=frequency=440 -t 20 -c:v libx264 -preset veryfast -pix_fmt yuv420p
   -c:a libopus -shortest t720.mp4` (7.2 MB) and the same at 1920x1080 without
   audio (13.5 MB).
2. Decode cost, `mpv --vo=null --length=10` (real time), bash `time`:
   720p30: wall 11.2 s, user 2.39 s, sys 0.24 s (~24% of one core);
   1080p30: wall 11.4 s, user 4.32 s, sys 0.25 s (~43%).
   1080p playing: mpv RSS 117 MB, 23 threads.
3. Software output cost, `WAYLAND_DISPLAY=<nested> mpv --vo=wlshm --length=10
   t720.mp4`: the window was tiled to 1048x1046, video scaled 1280x720 ->
   1048x589 in software: wall 12.0 s, user 4.67 s, sys 0.24 s, i.e. ~2.3 s
   (~23% of a core) more than vo=null. wlshm is the closest command-line proxy
   for libmpv's SW render (both scale and convert on the CPU into shared
   memory); the real SW render API was NOT measured (needs a program: QM1b).
4. Separate toplevel: in the nested dwl, mpv's window took the whole output
   (tiled; grim showed the letterboxed video filling the screen). A Wayland
   client cannot position its own toplevel; dwl only floats windows by
   compile-time rules (app_id) and the client still cannot place itself, so a
   second toplevel cannot sit over a `<video>` box or follow scrolling.
5. `--wid`: `mpv --vo=wlshm --wid=12345` under Wayland logs
   `Setting option 'wid' = '12345'` and then opens its own xdg_toplevel anyway
   (X11 support is compiled out). Wayland has no window ids; a wl_subsurface
   must be created on the parent's own client connection, so another process
   cannot become our subsurface. (mpv's own Wayland VOs use a subsurface +
   viewporter inside mpv's own toplevel, which shows dwl supports those for a
   single client.)
6. IPC (`mpv --idle --pause --vo=null --input-ipc-server=<sock>`, JSON lines from
   a small Python client): `observe_property time-pos`, `loadfile` -> events
   start-file, audio-reconfig, file-loaded, video-reconfig, playback-restart;
   get_property duration 20.0, width 1280, height 720, pause true, volume 100,
   seekable true, file-format mov/mp4, video-params/pixelformat yuv420p,
   eof-reached false; `set pause false` -> one time-pos event per frame
   (0.0333 s steps), time-pos 1.03 after 1 s; `seek 12.5 absolute` -> seek
   event, time-pos 12.5; `set volume 30` ok; `quit`. libmpv's client API exposes
   the same commands/properties/events in-process.
7. Pipe input: `cat t720.mp4 | mpv -` (moov atom at the end of the file, as
   ffmpeg writes by default) -> "Stream is not seekable", "Cannot seek backward
   in linear streams!", "error reading packet: Invalid data". A seekable byte
   source (Range reads) is required: stream_cb seek_fn, or an HTTP endpoint
   with Range.

Options (cost and size; ranking and reasons in TODO.txt QM1):
| Option | What | New code / deps | CPU per playing video (measured proxies) | Embeds? |
| --- | --- | --- | --- | --- |
| A1 | libmpv in-process, SW render into the `<video>` box, painted by scratchy | FFI to ~20 libmpv functions (or a -sys crate), dynamic link to the system libmpv (2.4 MB, already installed; ffmpeg libs shared), optional at build or run time | decode 24-43% of a core + SW scale ~23% + page blit/present (QM1c) | yes, with clipping/z-order/transform from the paint path |
| A2 | libmpv SW render into scratchy's own wl_subsurface + wp_viewporter | A1 + raw Wayland objects on winit's connection | decode + convert (compositor scales) | above or below the whole page only; clipping via viewport crop |
| B | libmpv OpenGL render into an EGL subsurface | A2 + EGL/GL context, GPU paths (not testable in the pixman nested dwl) | lowest | as A2 |
| C | spawn mpv as its own window (+ IPC) | spawn + optional socket | decode + GPU output | no: own dwl tile |
| D | ffmpeg/ffplay subprocess writing raw frames | our own A/V sync, audio, seeking | decode + pipe copy | as A1, much more code |
| E | vlc / GStreamer | not installed; GStreamer waylandsink embeds in-process too | n/a | n/a |

Security notes: a player handed a page URL fetches by itself: no wire policy
(private targets, bad ports, mixed content), no partitioned cookie jar, no
CORS, and mpv's ytdl hook would run youtube-dl/yt-dlp on it. With libmpv the
fix is to be the only byte source (stream_cb) and turn off mpv's extras
(`config=no`, `ytdl=no`, `load-scripts=no`, `osc=no`, default bindings off,
`load-unsafe-playlists=no`). For a separate mpv process (option C) the
equivalent is a loopback URL with a random token served by scratchy from wire,
plus the same flags. In-process libmpv means ffmpeg's demuxers/decoders parse
untrusted bytes in scratchy's process (QM1-D D2: helper process QM1h).

NOT verified: the libmpv SW render API's own cost (only the wlshm proxy); the
real (GPU) dwl's linux_dmabuf, `--vo=gpu`/`dmabuf-wayland` (QM1a); audio output
(every test used `--ao=null`); whether wire forwards `Range`/206 (QM1d);
scratchy's full-page repaint time per frame (QM1c); any webm/vp9/av1 file
(decoders are listed, not played); that `load-unsafe-playlists=no` plus
stream_cb alone stops every ffmpeg protocol (QM1e's strace test must show it).

## R24: YouTube's 5 s CPU wall is QY6e's own `define()` scan (Opus investigator, 2026-11-16) — MOSTLY FIXED (QY21-QY24 done); QY25, QY26, QY27 open

**Status (audit 2026-11-17, main @ e4993c3):** DONE: QY21 (959cd39, the quadratic define() scan), QY22, QY23, QY24. Open: QY25 (diagnostics), QY26 (bytecode cache, optional), QY27 (re-measure YouTube).

**Status: open (perf bug introduced by QY6e f887f67, plus budget-policy
gaps).** User report: `js error: ...k=ytmainappweb.kevlar_base...
stopped: exceeded the 5 s CPU-time page-load budget (10578 KB of source)`.

How it was measured (reusable; nothing committed):
- Page: `curl --unix-socket $(wire --print-socket) --request-target
  https://www.youtube.com/ http://x/` with `X-Wire-Jar: <own name>` (do
  not use the user's `scratchy` jar). YouTube A/B-serves two front ends:
  2 of 3 fetches got `kevlar_base` (10578 KB, the user's), 1 got
  `base_module_set` (1184 KB; loads in 0.2 s, dies on `Symbol.iterator of
  undefined` at m=base:1934:36). Saved copy with `<base
  href="https://www.youtube.com/">` after `<head>`, run with
  `scratchy --js-report --socket ... [--run-timers 5000] copy.html`.
- Temporary instrumentation in the worktree, reverted after: in
  `JsEngine::eval`, `JS_Eval(.., JS_EVAL_FLAG_COMPILE_ONLY)` then
  `JS_EvalFunction`, thread-CPU time around each; every global Rust native
  (`__d_*`, `__url_*`, `__cookie_*`, ...) re-wrapped in JS with a
  `thread_cpu_nanos` timer (per-native and per-selector totals); env
  overrides for SCRIPT_BUDGET / CALLBACK_BUDGET; per-timer-callback time.
  No perf/gdb on the box (eu-stack exists but ptrace_scope=1).

Numbers (thread CPU, load average 1-2, one run each unless noted):

| run | compile | run | host (natives) | JS | result |
|---|---|---|---|---|---|
| main, 5 s budget | 1418 ms | 3623 ms | 3036 ms (`__d_query` 751 calls 3027 ms) | 588 ms | trips at 5 s |
| main, budget 120 s | 1400 ms | 12748 ms | 11155 ms (`__d_query` 1646 calls 11109 ms) | 1592 ms | completes; 21 s wall for the report |
| define() queries only the new name, 5 s | 1390 ms | 1550 ms | 411 ms (`__d_query` 367 ms, 325 ceScan calls 227 ms) | 1139 ms | NO load error |
| same, bytecode round trip | 920 ms (+79 write +79 read) | 980 ms | 259 ms | 721 ms | bytecode 32339 KB, ran fine |

Every slow query had the same selector: all defined names joined with ','
(`dom-module,array-selector,custom-style,dom-bind,dom-if,dom-repeat,
iron-a11y-keys,...`), from `CER.prototype.define` (`ceSel`), and later
`ceScan`/`ceCollect`. 1227 elements at load, 1318 after kevlar, 1741 after
5 s of timers. Preludes: ~45 ms compile + ~5 ms run per page. getComputedStyle
(`__d_cs`) 67 calls 7.6 ms. No forced layouts exist (no geometry API).

After the define fix, timer phase (`--run-timers 5000`): 7 callbacks over
20 ms, the two biggest 465 and 491 ms (CALLBACK_BUDGET is 500 ms), so a
few trip; in a live window (`tools/nest.sh`) the trip then CASCADES: 6684
`Uncaught (in promise job) InternalError: interrupted` lines in 40 s
(`drain_jobs` keeps running queued jobs under the expired deadline), plus 4
`out of memory` (256 MB). With CALLBACK_BUDGET raised to 60 s locally: 548
callbacks, 96 requests (POST /youtubei/v1/guide 200, icon SVGs, 6
googlevideo `videoplayback` fetches), errors left: `no setter for property`
(Polymer assigns `this.style = "..."`, kevlar :8885:184; found only after
making `ceReport` print the message), `cannot read property 'split' of
undefined` in our classList `classes()`, one CORS-blocked `GET
https://youtube.com/`. The window still shows only the grey skeleton
(screenshot viewed: two grey thumbnail boxes with grey text bars).

## R25: social.loupax.com/discover: vim.js dies on getBoundingClientRect; clamp.js clamps every post; body background not on canvas (Opus investigator, 2026-11-16) — MOSTLY FIXED (QJ4-QJ9, QW6 done, QJ11 decided); QJ4b, QJ6b, QJ10 open

**Status (audit 2026-11-17, main @ e4993c3):** DONE: QJ4 (getBoundingClientRect), QJ5, QJ6, QJ7 (d48951f, Range text geometry), QJ8, QJ9, QW6; QJ11 DECIDED (pages may cancel any key). social /discover loads with 0 JS errors headless. Open: QJ4b (layout panic in `measure_grid`), QJ6b (horizontal scroll), QJ10 (Custom Highlight API).

**Status: open (missing features, not regressions).** User report:
`Uncaught (in promise) TypeError: not a function at vim.js:104:63 (map,
buildGutter vim.js:104:26, called at vim.js:176:1)`. Column 63 is
`.getBoundingClientRect()`: `Element.prototype.getBoundingClientRect` does
not exist (`querySelector` on `<article>` works).

How: `curl` of /discover through wire (logged-out HTML; in the window the
site gives scratchy's jar a guest account, same scripts), copy with
`<base href>`, `--js-report --run-timers 5000`; a local fixture probing
`typeof` of every API the site's 9 scripts use; then stand-in levels as
inline scripts after `<base>` (scratchpad only): (1) fake geometry on
Element.prototype (gBCR, getClientRects, offset*/client*/scroll*,
documentElement.scrollHeight), (2) no-op `scrollIntoView`, (3) a minimal
Range (setStart/setEnd/selectNodeContents/getClientRects/gBCR); then
synthetic `KeyboardEvent` keydowns dispatched from a timer.

Errors, today (clean build): exactly 1, vim.js:104:63 above. Other
scripts load with 0 errors, but clamp.js silently clamps 17/17 posts
(`b.scrollHeight` undefined), seen in the window as a "Read more" button
under every post, even 3-line ones. Behind the first wall: level 1 ->
`not a function` vim.js:260:29 (`scrollIntoView` in `placeLine`); level 2
-> `NotSupportedError: Range is not implemented` vim.js:309
(`computeLineChars`); level 3 -> 0 errors, load completes, gutter 148
rows (fake geometry), #vimbar/#vimgutter present. Keys (headless): j, j,
l, G, gg, Ctrl-D, }, n, `:`+"tag"+Enter (picker listed the tags), j, Esc,
`/`+search, `:feed` ("No feeds yet" for the guest), `:help` (opened;
with getter-only stand-in `scrollTop` it threw `no setter for property`
at vim.js:1083), Esc: all handled and preventDefault'ed. Not reached /
undefined: `window.open` (Enter on a post), `window.close` (`:q!`),
`Element.scrollBy` (help j/k), `document.activeElement` (undefined; after
`line.focus()` it is still undefined), `CSS.highlights`/`Highlight` and
`caretPositionFromPoint` (feature-detected: no error, but the cursor and
search highlights are invisible), `confirm` (submit only),
`PublicKeyCredential` (passkey forms only; handled message).
IntersectionObserver and matchMedia: not used by the site's scripts.
`w`/`e`/`b` exceeded the 500 ms callback budget with fake geometry (not
conclusive, re-test with QJ4/QJ7).

How vim.js is included: not in any template; `egg.js` (deferred, in the
layout, so every page, guest or logged in) runs `if
(!localStorage.vimOff) import("/vim.js")`. localStorage is memory-only
(QF-D7), so vim.js loads on every visit and `:set novim` never persists.

Visual (window, `tools/nest.sh`, screenshot viewed): header and nav links
run together (QW4), the hidden `#vimpagetags` tag list shows (QW5), every
post has "Read more", the page is a dark 40rem column between WHITE side
margins: the body background (`html.vim.catppuccin body {background:
#1e1e2e}`) is not propagated to the canvas (QW6; reproduced with a
one-line fixture), and an empty dark #vimbar strip at the bottom (its text
is set by `normal()`, which never runs).

## R23: Shadow DOM / template.content survey and design evidence (Opus investigator, 2026-11-16) — EVIDENCE for the TODO.txt "DESIGN MEMO (QSD)" block

**Status (audit 2026-11-16): partly outdated.** Since this was measured,
QY6f `template.content` (ac492ca), QSD0 (824954b), QSD1 (f704a73), QSD1b
crypto (e2929d5), QSD2 (0239f7b) and QSD4 constructable stylesheets
(ec80dfd) landed, so the github "appendChild of undefined", read-only
`querySelectorAll`, `document.domain` and `crypto.randomUUID` walls below
are gone. UNCHANGED: `attachShadow` still logs `not implemented` and
throws NotSupportedError (QSD3 waits for SD-D1), `display: contents`
(QSD5) and all rendering of shadow trees are missing. The per-site error
counts below have not been re-measured (QSD12).

Investigation only, no code changed. Build: release build of `5ca5619`
(origin/main after QY6g IO/RO), `target/release/scratchy` of an isolated
worktree. Memo, user checklist (SD-D1..D4) and items (QSD0-QSD12, QY6f)
are at the top of the TODO.txt QUEUE.

### How this was measured (reusable)

- Pages: `curl -sL --compressed -A '<Firefox 128 UA>'`, raw saved, then
  a COPY with `<base href="<effective url>">` right after `<head>`. Run:
  `scratchy --js-report --run-timers 5000 --socket /run/user/1001/wire.sock copy.html`.
- Static: every first-level `<script src>` downloaded (module imports
  not followed, except github's `f18-10a827ef5e40729a.js` chunk, which
  the error stacks name) and counted for `attachShadow`, `shadowRoot`,
  `adoptedStyleSheets`, `replaceSync`, `:host`, `::slotted`, `::part(`,
  `<slot`, `assignedNodes/Elements`, `slotchange`, `composedPath`,
  `getRootNode`, `attachInternals`, `shadowrootmode`, ShadyDOM, Lit.
  Failing positions read by line:column in the minified files.
- Stand-ins: a third copy with one inline `<script>` right after
  `<base>` (scratchpad only), in levels, each API logging its first use
  with `console.error('SD <api>')`:
  1. `template.content` (lazy fragment holding the template's children),
     template `innerHTML` get/set through it, deep `cloneNode` copying it.
  2. `attachShadow` returning a detached DocumentFragment with `host`,
     `mode`, `innerHTML`, `adoptedStyleSheets`, `getElementById`;
     `Element.shadowRoot` (open only). NOT rendered (needed only to get
     past startup; for REAL rendering this must become QSD6/QSD7).
  3. `getRootNode({composed})`, `isConnected` (startup only; real ones
     are QSD2, trivially correct).
  4. `CSSStyleSheet` ctor + `replaceSync/replace/insertRule`,
     `document.adoptedStyleSheets` (stored, not applied: real rendering
     needs QSD4).
  5. `attachInternals` (inert object). 6. slot `assignedNodes/Elements`
     returning `[]` (real: QSD10).
  q. work-around for the QSD0 bug: force `writable:true` when a page
     defines a value on `DocumentFragment.prototype`.
  x. non-shadow gaps: `document.domain`, `document.firstElementChild`,
     `document.getElementsByName`, `document.importNode`.
- Screenshots: `tools/nest.sh` nested dwl + `grim` after 15-20 s.

### Error locations (verified in the fetched sources)

| Page | Error | Source | What it needs |
|---|---|---|---|
| github (3 entries; repo page: 4) | `TypeError: cannot read property 'appendChild' of undefined` | f18-10a827ef5e40729a.js:223:27350, task-lists element module top level: `let f=document.createElement("template"); ... f.content.appendChild(p)` | `template.content` (QY6f), NOT shadow DOM |
| github (12x) | `not implemented: Element.attachShadow` in `relative-time` constructor | f18 223:10665 `l.set(this,this.shadowRoot?this.shadowRoot:this.attachShadow?this.attachShadow({mode:"open"}):this)` via the command polyfill wrapper f18 136:166006 (`i=e.prototype.attachShadow; e.prototype.attachShadow=function(e){let n=i.call(this,e)...}`) and the popover polyfill wrapper f18 285:1468 | attachShadow + shadowRoot (QSD3); "absent" does not work because the command polyfill wraps it unconditionally |
| github (1x) | same, in a connectedCallback | f18 136:687, `<tool-tip>`: `if(!this.shadowRoot){let e=this.attachShadow({mode:"open"}); e.appendChild(style).textContent=this.styles(), e.appendChild(slot)}` | QSD3 |
| github (after template.content) | `TypeError: 'querySelectorAll' is read-only` (all 3 entries) | scratchy `wrap()` -> `queries(o)` assigns per fragment instance after f18's popover polyfill did `Object.defineProperty(DocumentFragment.prototype,"querySelectorAll",{value(e){...}})` | QSD0 (scratchy bug) |
| github (next) | `Error: Unable to get document domain` | core-95dd3246eeba0b9e.js:136:26710 `let o=document.domain;if(null==o)throw` | QSD1 |
| github (next) | `cannot read property 'classList' of undefined` | core 2:114409 `document.firstElementChild.classList` | QSD1 |
| github (next) | `TypeError: not a function` | core 136:3547 `e.getElementById(t)\|\|e.getElementsByName(t)[0]` | QSD1 |
| github (last) | exception in `react-app` connectedCallback | core 136:16274 `crypto.randomUUID()` | QSD1b |
| youtube | `Failed to execute 'appendChild': parameter 1 is not of type 'Node'` | kevlar_base 10725:29 `document.head.appendChild(_.GZ().content)` (Polymer template) | QY6f |
| youtube (after QY6f stand-in) | `stopped: exceeded the 5 s CPU-time page-load budget (9533 KB of source)` | the app script | not shadow DOM (ShadyDOM forced) |

### github error count by stand-in level (`--js-report`, errors total)

baseline 29 -> level 1 (template.content) 29 (the error just moves to
QSD0) -> 1+q 32 -> 2+q 6 -> 9+q 6 -> 2+q+x 1 -> 9+q+x 1 (the remaining
one is `crypto.randomUUID`; timers: one callback over the 500 ms budget,
the collector POST blocked by CORS preflight). At 9+q+x the run logged
every stand-in as used at startup: template.content, attachShadow(open),
adoptedStyleSheets= (shadow root), new CSSStyleSheet + replaceSync,
document.adoptedStyleSheets=, getRootNode, isConnected, document.domain,
document.firstElementChild, document.getElementsByName.

### Per-site table (2026-11-16)

Counts: static (inline / first-level external scripts); errors =
`--js-report` errors total, baseline -> with stand-ins 9+q+x.

| Page | Needs shadow DOM at startup? | Static evidence | Errors | Notes |
|---|---|---|---|---|
| github.com | yes (imperative) | f18: attachShadow 14, shadowRoot 35, adoptedStyleSheets 4, `:host` 29, getRootNode 21, attachInternals 6, `.content` 44 | 29 -> 1 | relative-time x12 (no `datetime`: same text either way), tool-tip; template.content is the module killer |
| github.com/rust-lang/rust | yes | same chunk | 7 -> 1 | next: run-time module import `primer-react-*.js` (QY15a) |
| chromestatus.com | yes, entire UI | Lit + shoelace: `:host` 133, `::part(` 89, `::slotted` 46, `<slot` 125, slotchange 21 | 2 -> 0 | body is one `<chromedash-app>`; blank until shadow trees render (B2) and fetch data arrives |
| developer.mozilla.org | yes (Lit + DSD) | 17 `<template shadowrootmode>` (mdn-dropdown x10, search-button x2, color-theme, language-switcher, switch, button, homepage-search); ext: attachShadow 1, adoptedStyleSheets 2 | 2 -> 0 | `--no-js` screenshot readable (menu as a plain list, no Search button); shadow content = "Search" label + widget styling (`:host(:not([loaded],:focus-within)) slot[name=dropdown]{display:none}`) |
| salesforce.com (served /de/) | yes (LWC + DSD) | 185 DSD templates (hgf-button 104, nav panels 56, ...), 315 `<slot>`, ext: shadowRoot 148, `:host` 154, `::part(` 36, adoptedStyleSheets 15, attachInternals 7 | 43 -> 12 | 36% of body text is inside DSD (whole nav). `--no-js` window blank after 20 s: cause not isolated. Non-shadow errors: crypto.getRandomValues, consent/analytics objects |
| youtube.com | no (ShadyDOM forced) | attachShadow 19, `<slot` 69, ShadyDOM 67, `.content.cloneNode` 1008 | 1 -> 1 | needs template.content; next wall CPU budget |
| duckduckgo.com | no | inline polyfill `if(!Element.prototype.attachShadow) attachShadow=function(){let o=createElement("div");return this.appendChild(o),o}` | 0 | the flattened fallback never runs because scratchy's attachShadow exists (and throws) |
| bbc.com/news | no | 1 feature test for the "toucan" video player | 0 | media: off limits |
| theguardian.com | no | none | 0 | |
| lobste.rs | no | none | 0 | |
| en.wikipedia.org | no | none | 0 | |
| news.ycombinator.com | no | none | 0 | |
| web.dev | no | 1 `<template>` | 0 | |
| reddit | n/a | www.reddit.com and /r/rust return an 8 KB bot wall to curl; old.reddit login page (saved by R22): attachShadow 3, Lit, getRootNode 4 | 1 (nonce, not shadow) | not representative |

Score: 4 of 13 fetchable pages need Shadow DOM at startup (github,
chromestatus, MDN, salesforce); 1 more (youtube) needs template.content.

### Code facts used by the memo

- `src/js_dom.rs`: `ElementProto.attachShadow` logs + throws
  NotSupportedError; `ShadowRoot` is an `illegal()` interface object;
  `HTMLTemplateElement` exists but has no `content`; no `getRootNode`,
  `isConnected`, `document.importNode`, `document.domain`, Document
  ParentNode, main-document `getElementsByName`; `CSSStyleSheet` ctor
  throws TypeError; no `attachInternals`. `composedPath()` = the
  parentNode path; `Event.composed` is stored.
- `src/dom.rs` `impl SelectorsElement for El`: `parent_node_is_shadow_root`
  false, `containing_shadow_host` None: the `selectors` 0.41 hooks for
  `:host`/`::slotted`/`::part` exist but are off.
- `src/images.rs` `collect_real_children` reads `handle.children`
  (9 `children.borrow()` sites in images.rs, none in layout.rs/paint.rs):
  the single place a composed-children function plugs in.
  `collect_style_css` walks `children` only, so `<style>` inside
  template content never leaks into the document today.
- No `display: contents` (`Display` enum in css.rs).
- html5ever 0.39 tree builder calls `sink.attach_declarative_shadow`;
  markup5ever_rcdom 0.39 returns false (default trait method).

### What could not be tested

- Real rendering of any shadow tree (nothing renders one); all "visible
  gain" statements come from reading the components' source and the
  `--no-js` screenshots (MDN, salesforce).
- Whether GitHub's React islands render anything new once the modules
  run (they hit `crypto` and run-time imports next).
- Logged-in GitHub (repo pages with `datetime` relative-time elements),
  reddit (bot wall), salesforce's blank `--no-js` window (cause unknown).
- Performance of a composed-tree walk and of per-tree rule filtering.

## R22: fetch/XHR survey and design evidence (Opus investigator, 2026-11-16) — EVIDENCE for the TODO.txt "DESIGN MEMO (QF)" block

**Status (audit 2026-11-16): outdated as a description of scratchy/wire.**
The "wire's behaviour" section describes wire BEFORE QF6: wire now
answers `X-Wire-Version: 3`, honours the `X-Wire-*` caps/policy headers,
reports final URL and target class, and keeps an in-memory partitioned
cookie jar (QF6 ce2175a, QF14 4d62f87; see wire/NOTES.md). fetch (QF8),
XHR (QF9), preflight (QF13), sendBeacon (QF10) and cookies (QF14b
bc2fb00) exist now; the user's answers (TODO.txt "USER ANSWERS
2026-11-16") replaced the restrictive defaults. The page measurements
are still the baseline; re-measure with QF12.

Investigation only, no code changed. Build: release build of `faa1cbe`
(origin/main, after QY15 ES modules), `target/release/scratchy` of an
isolated worktree. The memo, user checklist (QF-D1..D9) and items
(QF1-QF19) are at the top of the TODO.txt QUEUE.

### How this was measured (reusable)

- Pages: `curl -sL --compressed -A '<Firefox 128 UA>'` saved raw, then a
  COPY with `<base href="<url>">` inserted right after `<head>` (for
  old.reddit, the URL it redirected to). Runs:
  `scratchy --js-report --run-timers 5000 --socket /run/user/1001/wire.sock copy.html`.
- Stand-ins: a second copy with one `<script>` inserted right after
  `<base>` (scratchpad only, never in the repo) that defines `fetch`,
  `Headers`, `Request`, `Response` (with a fake `body` stream),
  `XMLHttpRequest`, `Blob`, `File`, `FileReader`, `FormData`,
  `TextEncoder`/`TextDecoder`, `ReadableStream`, `TransformStream`,
  `WebSocket`, `EventSource`, `Worker`, `navigator.sendBeacon`. Each logs
  every call with `console.error('FX ...')` plus the calling frame, so
  the calls show up in the report. fetch/XHR answer after 50 ms (timer)
  with status 200 and body `{}`. No real network from page JS.
- Static: every inline script plus every first-level external
  `<script src>` of each page downloaded and grepped for network API
  patterns (module imports were not followed).
- CORS: `curl -H 'Origin: <page origin>'` on each observed endpoint.
- wire: raw requests written to `/run/user/1001/wire.sock` with a small
  Python client (absolute-form request line, like scratchy).

### What the pages call (stand-ins, `--run-timers 5000`)

| site | plain run: scripts / errors | network calls at startup (stand-in run) | what the response is used for |
|---|---|---|---|
| theguardian.com/europe | 17 / 7 (+2 timers) `ActiveXObject is not defined` | `new Request('')` (polyfill.io feature test); 10 fetches: `GET api.nextgen.guardianapps.co.uk/geolocation` (no options), 9x `GET ophan.theguardian.com/img/{1,2}?...` with `mode:'no-cors', keepalive:true, credentials:'include'` (7 at load, 2 in timers) | geolocation `.json()` -> CMP country (the plain run's `CMP initialised without country` console.error is gone with the stand-in); ophan responses never read. With fetch present: 0 errors |
| old.reddit.com (served `/login/?reason=lor2`) | 20 / 3 (customElements, a data: module, `fetch is not defined`) | 1 fetch `GET https://www.reddit.com/svc/events/preload-head?renderId=...&pageType=login` (no options) | not read in 5 s; `fetch is not defined` gone, customElements errors remain (QY6) |
| youtube.com | 39 / 2 (MutationObserver, `polyfillWrapFlushCallback`) | none (dies first); `new TextEncoder()` at startup | (R21, with 16 shims: 1 call to `/youtubei/v1/player` + 336 icon fetches) |
| github.com | 5 / 3 `TextEncoder is not defined` | none; with TextEncoder supplied the 3 errors become `customElements is not defined` | — |
| duckduckgo.com | 35 / 0 (16 console.error in timers, same with and without stand-ins: `TypeError: not a function` x8 + Next.js "client-side exception" x4 + "Route did not complete loading: /_error" x4; not network-related: `isEqualNode`, fixed since by QY16b) | `new ReadableStream()` (feature test; the page polyfills ReadableStream/TextEncoder itself when missing); `GET /country.json` (same-origin); 4x `sendBeacon` to `improving.duckduckgo.com` (2 with a 315-char string body, 2 URL-only) | `/country.json` `.json()` |
| bbc.com/news | 57 / 0 (+1 timer error `Route did not complete loading: /_error`, same with stand-ins) | 1 fetch `GET https://www.bbc.co.uk/userinfo` (no options) | `.json()` (signed-in state) |
| lobste.rs | 1 / 0 | none (the one module only rewrites `<time>` texts) | — |
| news.ycombinator.com | 1 / 0 | none at startup. On click (hn.js): vote = `XMLHttpRequest` `GET vote?id=..&how=up&auth=..`, collapse = XHR `GET collapse?id=..` (only when logged in), hide = `fetch('snip-story?..').then(r=>r.json())` | needs a session cookie; logged out there are no vote links |

Not used at startup by any page: `XMLHttpRequest`, `FormData`, `Blob`,
`File`, `FileReader`, `response.body`/`getReader`, `.text()`, `.blob()`,
`.arrayBuffer()`, `AbortSignal` on a fetch, `WebSocket`, `EventSource`,
`Worker`, sync XHR, any non-GET method. Every read used `.json()`.

### Static counts (inline / first-level external scripts; grep, not proof of use)

| pattern | guardian | old.reddit | youtube | github | ddg | bbc | hn |
|---|---|---|---|---|---|---|---|
| `fetch(` | 0/14 | 1/3 | 0/35 | 0/41 | 1/5 | 0/28 | 0/1 |
| `XMLHttpRequest` | 0/13 | 0/1 | 0/17 | 0/2 | — | 0/11 | 0/1 |
| `"POST"` | 0/6 | 0/6 | 0/47 | 0/12 | 0/1 | 0/33 | — |
| `credentials:"include"` | 0/1 | — | 0/3 | 0/1 | — | 0/9 | — |
| `mode:"no-cors"` | 0/2 | 0/2 | 0/3 | 0/1 | 0/1 | — | — |
| `withCredentials` | 0/2 | 0/3 | 0/56 | 0/1 | — | 0/22 | — |
| `getReader(` | — | — | 0/3 | — | 1/0 | 0/1 | — |
| `signal:` | — | 0/3 | 0/41 | 0/19 | 0/1 | 0/3 | — |
| `FormData` | 0/5 | 0/3 | 0/8 | 0/24 | 0/5 | 0/18 | — |
| `sendBeacon` | 0/2 | 0/3 | 0/19 | — | 13/5 | 0/15 | — |
| `EventSource` / `WebSocket` | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| sync `xhr.open(..., !1)` | 0/1 | — | — | — | — | — | — |

Read at the source:
- guardian's only sync XHR is polyfill.io's `navigator.sendBeacon`
  polyfill: `r.open("POST",t,!1)`. It is installed only when
  `sendBeacon` is missing.
- youtube's innertube client: `method:"POST", postParams, postBody,
  postBodyFormat:"JSON"` ("innertube xhrclient"), plus
  `fetch(x.url,{method:x.requestType, ..., credentials:"include" (when
  withCredentials), keepalive, signal})`. Its streaming reader is
  feature-tested: `t.body && typeof t.body.getReader === "function"`,
  else `handleResponse`.
- lobste.rs `application.js` (1.9 KB, logged out): no network calls.

### CORS headers of the observed endpoints (curl with `Origin:`)

| request | status, type | CORS |
|---|---|---|
| `api.nextgen.guardianapps.co.uk/geolocation` from theguardian.com | 200 application/json | `ACAO: *`, `ACA-Credentials: true` |
| `www.bbc.co.uk/userinfo` from www.bbc.com | 200 application/json | `ACAO: https://www.bbc.com`, `Vary: Origin` |
| `www.reddit.com/svc/events/preload-head` from old.reddit.com | 200 text/plain | none (and 6 `Set-Cookie`s: loid, session_tracker, csrf_token, token_v2, csv, edgebucket) |
| `duckduckgo.com/country.json` | 200 application/json | same-origin |
| `fonts.gstatic.com/s/i/...` (youtube icon host; a made-up path) | 404 | `ACAO: *` |

So browser CORS rules (QF-D1 option B) cost nothing on these pages.

### wire's behaviour (raw requests on the socket)

- `GET file:///etc/os-release` -> `200`, `application/octet-stream`,
  the file's contents. wire serves ANY file the user can read to
  anyone who can write to the socket. Page JS must never reach it with
  a `file:` URL.
- `GET http://127.0.0.1:1/` and `GET http://localhost:631/` -> `502`
  "Connection refused": wire does connect to loopback.
- `GET http://169.254.169.254/latest/meta-data/` -> no answer within 30
  s (client timeout). wire's 20 s agent timeout did not end it within
  30 s. scratchy's `fetch_raw_via_wire` has no read timeout at all.
- `GET http://lobste.rs/` -> `200` with the https page (redirect
  followed), but nothing in the response says the final URL.
- `HEAD https://duckduckgo.com/country.json` -> `200`, headers only.
  `OPTIONS https://www.bbc.co.uk/userinfo` with `Origin` -> `204` with
  `access-control-allow-*` headers (a preflight would work).
- Code read (wire/src/main.rs): every client header is forwarded except
  Host/Content-Length/Connection (so a page could set `Cookie` or
  `Host`-like headers if scratchy passed them through). Response
  headers are all forwarded except Content-Length/Transfer-Encoding/
  Content-Encoding (so `Set-Cookie` reaches scratchy). No cookie jar
  (ureq without its `cookies` feature). ureq 2.12.1: 5 redirects,
  POST->GET on 301/302/303, `Response::get_url()` has the final URL,
  and `AgentBuilder::resolver` exists (ureq resolves on every redirect
  hop, which is what QF6 relies on).

### Not tested / caveats

- No real network from page JS: each stand-in answered `{}` after 50
  ms. What the pages do with real data (guardian's consent banner once
  the CMP has a country, bbc with a signed-out `userinfo`) is unknown.
- youtube and github stop before their first request (QY6), so their
  real request pattern is static reading plus R21, not observation.
- `--js-report` runs with the copy's file:// URL as `location`
  (guardian's beacon carried `url=file:///tmp/...`). Origin-based
  behaviour in the pages may differ from the live site (QF7 fixes the
  report).
- old.reddit served its login wall to curl (logged out). Logged-in
  pages, HN voting and anything behind cookies were not exercised.
- Only startup plus 5 s of simulated timers. No clicks, scrolling or
  typing, so lazy-load and infinite-scroll requests are not in the
  table.
- Module imports below the first level were not grepped.
- The ddg `TypeError: not a function` console.errors (x8, also without
  stand-ins) were not traced here; QY16b (merged during this
  investigation, 15526fd) found them to be the missing
  `Node.isEqualNode` and fixed them. The survey build predates it.

## R21: Web Components / YouTube decision and real-site JS survey (Opus investigator, 2026-11-16) — EVIDENCE for the TODO.txt "DECISION MEMO (QY6)" block

**Status (audit 2026-11-16): outdated.** Almost everything listed as
missing below has been built: QY8-QY17, QY19, QY20, MutationObserver
(QY6a), customElements (QY6e), template.content (QY6f), IO/RO (QY6g),
fetch/XHR (QF8/QF9). Still missing from the lists here: translate
`transform` (QY18, waits for the user), container queries (QY7), real
layout geometry for JS (QY-IO-GEOMETRY). The memo's "no measured user
for native Shadow DOM" was superseded by R23. The D0 gate was closed by
the coordinator on 2026-11-16.

Investigation only, no code changed. The decision memo and the filed
items (QY8-QY20, plus re-split QY6) are the first block of the active
TODO.txt QUEUE. Build: release build of `0735ddd` (origin/main at the
start), `target/release/scratchy` of an isolated worktree.

### How this was measured (reusable)

- Pages: `curl -sL -A '<Chrome UA>' <url>` saved raw, then a COPY with
  `<base href="<url>">` inserted after `<head>` and run with
  `scratchy --js-report --socket /run/user/1001/wire.sock copy.html`
  (QJ3). External scripts come through wire.
- Missing-API chain: a shim `<script>` inserted right after `<base>` in
  the copy (scratchpad only, never in the repo) defined each missing
  API in JS; re-run; read the next first error at its source location
  (`python3 -c` slice of the script at the reported line:col). Each
  stub logged `USE <name>` the first time it was touched, so the report
  also lists what is used without throwing.
- `--js-report` drops the engine right after `load`, so timers,
  `requestAnimationFrame` and anything after them never run there. To
  see what an app does next, 60-200 copies of a "tail" script were
  appended before `</body>`: the first replaces `__scratchy_now` with a
  fake clock; each copy advances it 100 ms and calls
  `__scratchy_timers_run_one()` until nothing is due. Separate
  `<script>` elements matter: promise jobs only drain between scripts.
  A DOM summary (element count, custom-element tag histogram, app text)
  was logged via `console.error`. (QY14 makes this a real flag.)
- Live: nested dwl (`tools/nest.sh`), one scratchy window at a time,
  `grim` after 18-40 s, every screenshot viewed. Another agent started
  a second nested dwl mid-run, so screenshots from lobste.rs on are
  860x523 instead of 1052x1050.
- Static counts: every classic script (inline + external) and every
  `type=module` entry script (first level only, imports not followed)
  of each page downloaded with curl and grepped for API names; every
  linked stylesheet + inline `<style>` grepped for CSS features.

### What scratchy's JS environment lacks today (probe page, `0735ddd`)

Undefined globals: `customElements MutationObserver IntersectionObserver
ResizeObserver matchMedia localStorage sessionStorage indexedDB fetch
XMLHttpRequest requestIdleCallback MessageChannel AbortController URL
URLSearchParams TextEncoder TextDecoder structuredClone BroadcastChannel
crypto Worker postMessage history CSS DOMParser Intl SVGElement
ShadowRoot HTMLTemplateElement PointerEvent FormData Blob File
HTMLImageElement screen visualViewport HTMLScriptElement Range Selection
getSelection DOMRect Headers Request Response ReadableStream WebSocket
HTMLAnchorElement HTMLDivElement HTMLBodyElement HTMLUnknownElement
NamedNodeMap DOMTokenList CSSStyleSheet MediaQueryList Window top
parent frames opener origin isSecureContext open close alert confirm
prompt print focus blur name`. QuickJS has no `Intl` at all
(`(1234.5).toLocaleString()` is `1234.5`). Present: `WeakRef`,
`FinalizationRegistry`, `queueMicrotask`, `Promise`, `Proxy`, `Reflect`,
`Symbol`, `CustomEvent`, `HTMLElement`, `Element`, `Node`, `Event`,
`EventTarget`, `KeyboardEvent`, `MouseEvent`, `Image`, `atob/btoa`,
`getComputedStyle`, `requestAnimationFrame`, `performance`, `navigator`,
`DocumentFragment`, `HTMLFormElement`, `HTMLInputElement`,
`HTMLIFrameElement`. Elements lack `attachShadow getBoundingClientRect
getClientRects animate insertAdjacentHTML insertAdjacentElement
getAttributeNames attributes scrollIntoView dataset shadowRoot isConnected
getRootNode compareDocumentPosition offsetWidth clientWidth scrollHeight
tabIndex slot replaceChildren nextElementSibling`; `document` lacks
`importNode adoptNode createRange elementFromPoint activeElement hasFocus
fonts styleSheets scrollingElement`. `classList` has `add` etc. but is
not iterable. `<template>` contents are invisible to JS: `childNodes` is
empty, `innerHTML` is `''` and `outerHTML` is `<template id="t"></template>`
(the parser puts them in `template_contents`, which nothing exposes).

Performance: `nextSibling` is O(number of siblings) (it builds
`parentNode.childNodes` and does `indexOf`), so walking a child list with
`firstChild`/`nextSibling` is quadratic. Measured: 100 siblings 11 ms,
300 siblings 93 ms, 1000 siblings 689 ms; indexing `childNodes` for the
same 1000 is 0 ms. Every framework walks the DOM this way (QY8).

### YouTube (www.youtube.com, logged-out, DE)

Page today: 40 scripts run (R20 counted 43). The 7 external ones, in
order: `web-animations-next-lite.min.js`, `webcomponents-sd.js` (the
polyfill is now this one, not `webcomponents-all-noPatch.js`),
`scheduler.js`, `www-i18n-constants.js`, `spf.js`, `network.js`, and the
app script `kevlar_base` (9.8 MB); the rest are inline (`ytcfg`,
`ytcsi` ticks, `ytInitialData` JSON, the `ShadyDOM` config, the
`serverContract` that defines `getInitialData`). `--js-report`: 2 errors,
`webcomponents-sd.js: ReferenceError: MutationObserver is not defined`
and `kevlar_base…: ReferenceError: AbortController is not defined`.
Live window, JS on and `--no-js`: identical grey skeleton (masthead
line, 1-column grey video cards), no logo, no guide.

Two facts that change the QY6 plan:
1. The page sets `window.ShadyDOM={force:true,preferPerformance:true,
   noPatch:true}` and `ShadyCSS={disableRuntime:true}` in an inline
   script. YouTube never uses native Shadow DOM (`attachShadow`), even in
   Chrome: ShadyDOM renders shadow content into the light DOM. Native
   Shadow DOM / slots in the cascade are NOT needed for YouTube.
2. `webcomponents-sd.js` contains ShadyDOM + ShadyCSS but NO
   custom-elements polyfill: it reads `window.customElements.define`
   unconditionally. So `customElements` must be provided by scratchy
   (the old "polyfill path" in QY6 no longer exists).

Ordered list of what had to be supplied (JS shims) until `kevlar_base`
ran to its end without throwing, and its custom elements upgraded
without throwing:

| # | first error (script) | supplied | real-implementation size |
|---|---|---|---|
| 1 | `MutationObserver is not defined` (sd, load) | no-op MO that never fires | stub S / real M (QY6a) |
| 2 | `AbortController is not defined` (kevlar, top level) | AbortController/AbortSignal | S (QY13) |
| 3 | sd: `cannot read property 'prototype' of undefined` in `["Text","Comment","CDATASection","ProcessingInstruction"].forEach(...window[a].prototype)` | `CDATASection`, `ProcessingInstruction` interface objects | S (QY12) |
| 4 | kevlar: `cannot read property 'polyfillWrapFlushCallback' of undefined` (= `window.customElements`) | a JS `customElements` (define/get/whenDefined/upgrade; `HTMLElement` constructor that returns the element being upgraded; upgrade on define, `createElement`, `cloneNode`, `innerHTML`; connected/disconnected/attributeChanged callbacks from wrapped `appendChild/insertBefore/replaceChild/removeChild/append/...`/`setAttribute`) | real M (QY6e) |
| 5 | sd: `Window is not defined` | `Window` with `Window.prototype` holding `addEventListener/removeEventListener/dispatchEvent`, and `window`'s prototype set to it (first try with `Window.prototype = Object.prototype` polluted every object: kevlar then died on `'__shady___shady_dispatchEvent' is read-only`) | S (QY12) |
| 6 | kevlar: `HTMLTemplateElement is not defined`, then Polymer needs `template.content` | `HTMLTemplateElement` + a JS-side `content` fragment per template, template-aware `innerHTML` get/set, `cloneNode`/`importNode` copying content | real M, native (QY6f) |
| 7 | kevlar: `TypeError: not a function` at `window.matchMedia("(forced-colors: active)")` | `matchMedia` | S (QY10) |
| 8 | kevlar: `cannot read property 'Symbol.iterator' of undefined` at `Array.from(fragment.children)` | `children/firstElementChild/lastElementChild/childElementCount/append/prepend` on `DocumentFragment` | S (QY12) |
| 9 | kevlar: `HTMLDivElement is not defined` | `HTML<Tag>Element` interface objects | S (QY12) |
| 10 | kevlar: `cannot read property 'pushState' of undefined` | `history` | S (QY13 / Q33f) |
| 11 | kevlar: `Range is not defined` (TypeScript decorator metadata `design:paramtypes:[KeyboardEvent,Range]`) | value-only interface objects (Range, Selection, DOMRect, Event subclasses ...) | S (QY12) |
| 12 | kevlar: `XMLHttpRequest is not defined` (`XMLHttpRequest.prototype.fetch` read at top level) | XHR/fetch stubs that log and fail | L (Q33f) |
| 13 | kevlar: `cannot read property 'indexOf' of undefined` at `document.currentScript.src.indexOf(...)` | reflected `src`/`href`/... | S (QY11) |
| — | kevlar top level now runs clean. Errors inside `ytd-app` upgrade/connect: | | |
| 14 | `_parseTemplateChildNodes`: `cannot read property 'length' of undefined` (`node.attributes`) | `Element.attributes`, `getAttributeNames`, `hasAttributes` | S (QY11) |
| 15 | `value is not iterable` (`for (z of el.classList)`) | iterable `classList` | S (QY11) |
| 16 | `initVisibilityObserver`: `throw new TypeError("Fg")` when `window.IntersectionObserver` is missing | IntersectionObserver that reports every target as intersecting once | stub S / real M (needs `getBoundingClientRect`, Q33g) |

Used without throwing (feature-detected or after the above):
`MessageChannel` (scheduler), `window.postMessage`, `localStorage`,
`sessionStorage`, `requestIdleCallback`, `document.importNode`,
`Request`/`Response` constructors, `fetch`. Never touched at startup:
`indexedDB`, `serviceWorker`, `WebSocket`, `Worker`, `attachShadow`.

What it does next (fake clock, 20 s simulated, 805 timers run): calls
`window.getInitialData()` (it has the `ytInitialData` browse response),
stamps the app shell (1629 elements, 104 custom elements of ~40 types:
`ytd-app`, `ytd-masthead`, `yt-searchbox`, `tp-yt-app-drawer`,
`ytd-mini-guide-renderer`, `ytd-page-manager` holding a preloaded
`ytd-watch-flexy` with `ytd-player`, `ytd-comments`, ...,
`ytd-popup-container`), and starts 337 `fetch` calls: 1 to
`/youtubei/v1/player` and 336 for icon SVGs on `fonts.gstatic.com`
(`yt-icon` loads icons with `fetch`). It does NOT create `ytd-browse`,
the video grid or the EU consent dialog within the 20 s; the app's text
is `Zurück Suchen NaN / NaN`; three `Polymer::Attributes: couldn't decode
Array as JSON: [[shownItems]]` warnings suggest some binding still goes
wrong under the shims. The live window with ALL the shims (copy with
`location` overridden to `https://www.youtube.com/`) looks exactly like
`--no-js`: the grey skeleton. One extra live-only line: `Uncaught
exception in a promise job` with no message (QY14).

Conclusion: even with ~16 APIs supplied (two of them, customElements and
template.content, with real semantics), YouTube gains nothing visible.
The next walls are `fetch` (Q33f, L), a real MutationObserver, the
quadratic DOM walking (QY8), and then whatever the browse page needs
(unknown: layout of the stamped Polymer DOM was never reached).

### Survey of other JS-heavy pages (same build, 2026-11-16)

| site | scripts run / errors (`--js-report`) | first errors | live JS on | live `--no-js` |
|---|---|---|---|---|
| github.com | 0 / 0 | none: all 5 scripts are `type=module` (+ `importmap`, 2.7 MB first-level module code); scratchy skips modules | readable landing page; nav menus as a plain vertical list; the "You signed in with another tab or window" banner shows though it has `hidden` (Primer's `[hidden]{display:none!important}` is inside `@layer primer-css-base`, and scratchy drops `@layer` blocks); "Enter your email" placeholder overlaps "Sign up for GitHub" | identical to JS on |
| www.reddit.com | 1 / 1 | `URLSearchParams is not defined` | curl/wire get an 8 KB bot-challenge page; its script needs `URLSearchParams`, an async function, then `form.requestSubmit()` (POST, needs Q30e + cookies Q30f). Shows an orange circle at the top-left and the logo, forever | identical |
| old.reddit.com | 17 / 2 | inline #8 `customElements is not defined`; `fetch is not defined` | 302 to `/login/?reason=lor2` (logged-out visitors are sent to login). Logo + bubble background pattern; the login form (`faceplate-*`/`shreddit-*` custom elements) is missing. With shims: `could not load module 'data:...'` (dynamic `import()` from a classic script) | identical |
| www.bbc.com/news | 57 / 0 | none (live: `NGAS load timeout`, BBC's own ad loader) | good: logo, nav, headlines, teasers; images are grey placeholders; a grey vertical scrollbar strip at x≈318 from the hidden nav drawer (`visibility:hidden; transform:translateX(-100%)` with an `overflow-y:scroll` child) | worse: the nav drawer's link tree is expanded over the left third |
| duckduckgo.com | 35 / 4 | `matchMedia` not a function (theme script); `URL is not defined` x2 (turbopack runtime); `URLSearchParams is not defined`. With URL/matchMedia shims: `Intl is not defined` -> Next.js "client-side exception" (React app dies); with a stub Intl: `Intl.Locale(...).maximize` | logo, search box, but the settings side menu is permanently open on the right (`transform:translate(100%)` ignored) | BLANK: `<noscript><meta http-equiv=refresh content="0; url=https://html.duckduckgo.com/html"><style>body{display:none}</style></noscript>`; meta refresh is not implemented |
| lobste.rs | 0 / 0 | the one script (`time ago` texts) is `type=module` | fine | fine |
| theguardian.com | 15 / 1 | commercial bundle: `Automatic publicPath is not supported in this browser` (webpack reads `document.currentScript.src`; no reflected `src`) | fine (server-rendered); the 69 `gu-island` islands hydrate from `type=module` bundles that never run | identical |
| en.wikipedia.org, news.ycombinator.com | 4/0, 1/0 | already clean | fine | fine |

Static reference counts, classic/module code (grep, not proof of use):
customElements yt 38, gh 0/2, oldreddit 4, bbc 2; MutationObserver yt
32, gh 0/5, bbc 16, ddg 9, guardian 3; IntersectionObserver yt 46, gh
0/20, bbc 16, ddg 14, guardian 11/3; matchMedia yt 56, gh 0/30, ddg 10,
guardian 5; localStorage yt 34, bbc 87/40, ddg 30, guardian 9/22, wiki
6; fetch yt 742, gh 0/75, bbc 181/18, ddg 77, guardian 7/16;
URLSearchParams yt 11, gh 0/39, bbc 59/3, ddg 44, guardian 14/3, reddit
1; Intl yt 12, gh 0/12, ddg 90, bbc 2; attachShadow yt 19 (ShadyDOM's own
patch code), bbc 1, oldreddit 1, gh 0. No page uses declarative shadow
DOM (`shadowrootmode`: 0 on all 10).

CSS feature counts (linked + inline sheets): `@layer` gh 18 (others 0);
`@container` yt 273, gh 83, oldreddit 9; `:has(` gh 330, yt 38,
oldreddit 27, ddg 16, guardian 11, lobsters 8, wiki 4 (scratchy:
`#a:has(b)` does not match, verified with getComputedStyle); CSS
nesting (`&`) 0; `transform:` bbc 40, gh 843, yt 673 (unsupported,
deliberately not queued in 2026-11-14's re-plan). Verified unsupported
with a getComputedStyle probe: `:has()`, `@layer` (whole block dropped
in `TopLevelParser::parse_prelude`), nesting, `@container`, `transform`.

### Aggregate: what blocks the most sites

JS (sites where it is the first error or demonstrably stops the page,
of the 8 non-clean ones):
1. ES modules incl. `importmap` and dynamic `import()`: 4 (github,
   lobste.rs, guardian islands, old.reddit), plus bbc has 1 module.
2. `URL`/`URLSearchParams`: 2 erroring (ddg, reddit challenge), used by
   yt/gh/bbc/guardian.
3. `customElements` (+ `template.content`): 2 erroring (yt, old.reddit),
   gh's Catalyst elements behind modules.
4. `fetch`/XHR: 2 erroring (old.reddit, yt after shims), referenced by
   all big sites.
5. `matchMedia`: 2 (ddg, yt). Reflected `src` (`currentScript.src`): 2
   (guardian, yt). `Intl`: 1 erroring (ddg, kills the React app).
6. YouTube-only so far: `MutationObserver` (sd), `AbortController`,
   `Window`, `CDATASection`, HTML*Element interface objects, `Range` &
   co., `Element.attributes`, iterable `classList`, IntersectionObserver.
Layout/CSS (visible damage): `transform: translate` for off-canvas
drawers (2: ddg, bbc), `@layer` (1: github, also hides `[hidden]`
banners), `<meta http-equiv=refresh>` (ddg no-JS), scrollbar painted on
a `visibility:hidden` box (bbc). `:has()` (7 pages) and `@container` (3)
are widespread but their visible effect was not measured.

### Not tested / caveats

- Shims are not implementations: the YouTube chain is what a JS-level
  stand-in got past; real code can hit more. My `customElements` shim
  ran only through wrapped JS entry points (no parser-created upgrade
  hook beyond `define`'s `querySelectorAll`).
- The fake-clock pump runs timers in an order a real event loop would
  not (all due timers of a 100 ms slice, promise jobs only between
  slices); no input, no layout feedback (`getBoundingClientRect` absent).
- No real network from JS: every `fetch`/XHR was answered with a network
  error, so "what YouTube does with data" is unknown. The browse page /
  consent dialog may well need a successful fetch.
- No Chrome/Firefox on this machine; "what Chrome shows" is inferred.
- reddit.com/old.reddit.com served a challenge/login page to this IP and
  UA; a logged-in or non-challenged visitor gets different pages.
- BBC grey image placeholders: cause not investigated (lazy images via JS
  or `srcset`/`<picture>`).
- `:has()`/`@container` visual impact and GitHub's module graph beyond
  the first level were not measured.

## R20: www.youtube.com is a blank window (Opus investigator, 2026-11-15) — blank window FIXED by QY1 (ebea7ae)

(Archived: full text in REGRESSIONS-FIXED.md.)

## R12–R19: Acid2 / Acid3 audit (Opus investigator, 2026-11-15) — ALL FIXED (R12 QA3 bd2945f, R13 QA1 10e4006, R14 QA2 80395ee, R15 QA4 065b61b, R16 QA5 e9232f3, R17 QY1 ebea7ae, R18 QA6 e8fb789, R19 QA7 00530a0)

(Archived: full text in REGRESSIONS-FIXED.md.)

## R8–R11: build-test pass on `main` @ `f3b06a8` (Opus investigator, 2026-11-15) — ALL FIXED

**Status (audit 2026-11-16):** R8 FIXED (QB1 637fef5 no panic, QB3
dbf1610 bounded stalls; remaining cascade stalls are Q32), R9 FIXED (QB2
d78587f), R10 FIXED (QB6 c150951), R11 FIXED (QB4 a29738f, part (c) QB5
ac60599). (Older line: "R11 DONE (QB4), R10 DONE (QB6), see TODO.txt.
R8, R9 still open.")

Investigation only, no code changed. Fix items are QB1–QB10 at the top of
the TODO.txt QUEUE. Everything below was seen in a real window and
screenshotted, unless it says otherwise.

### How this was tested (reusable recipe)

The old `AUTOSCROLL`/`AUTOHOVER` env vars no longer exist in `src/`; they
were temporary instrumentation. Real input can be driven without touching
the user's desktop:

- Nested compositor: `env -u DISPLAY WLR_BACKENDS=wayland WLR_RENDERER=pixman
  setsid timeout 3600 dwl -s 'echo $WAYLAND_DISPLAY > disp' &`. This opens a
  dwl window (1052x1050 here) inside the user's dwl and prints its socket
  name (e.g. `wayland-1`). `WLR_BACKENDS=headless` also works and is fully
  invisible, but dwl then never advertises seat capabilities
  (`wl_seat.capabilities(0)`), so clients get no pointer or keyboard. Use the
  `wayland` backend when input is needed.
- Run scratchy in it: `WAYLAND_DISPLAY=wayland-1 timeout 120
  target/release/scratchy --socket /run/user/1001/wire.sock <url>`.
  Screenshot with `WAYLAND_DISPLAY=wayland-1 grim out.png`.
- Input: a ~150-line Rust tool (scratchpad only, not committed; QB10 asks
  to commit it). It uses `wayland-client` 0.31, `wayland-protocols-wlr`
  (`zwlr_virtual_pointer_v1`: `motion_absolute`, `button` 0x110,
  `axis_discrete`, `frame`) and `wayland-protocols-misc`
  (`zwp_virtual_keyboard_v1`: `keymap` from `xkbcomp -xkb` of
  `pc+us`/`evdev`, `key` with evdev codes, `modifiers` with Shift=1 and
  Control=4). It takes commands like `move:X,Y click scroll:N key:j key:c-d
  type::go\ URL key:ret wait:MS`. Both protocols are dwl globals.
- Never `pkill -f <pattern>` from a shell whose own command line contains
  the pattern: it kills that shell. Kill PIDs you started.

Tested (all in the nested dwl, release build of `f3b06a8`): `:go` by
typing; wikipedia.org, en.wikipedia Main_Page, SoHo_Weekly_News,
news.ycombinator.com, html.duckduckgo.com/html, github.com, bbc.com/news,
each with JS on and `--no-js`; `:set nojs` + `:refresh`; mouse-wheel and
`Ctrl+d` scrolling; hover sweeps; link clicks (portal language button,
"Commons" sister link); `h`/`l` history incl. rapid `l h l h`; `:help`;
window resize (tiling a second client, then closing it); unreachable host
via wire; missing wire socket; image loading after rapid navigation (images
present every time, no R7 recurrence); idle CPU after load (0% on HN,
portal, Main_Page, SoHo, JS on and off); stderr closed early. Not tested:
`Ctrl+v` paste, `Tab` completion, `<select>`, keyboard focus with `Tab`,
streaming stdin mode, HiDPI/fractional scale, X11.

### R8 — Main-thread stalls get scratchy disconnected by the compositor, then it panics (bug) — FIXED (QB1 637fef5 + QB3 dbf1610; leftover stalls: Q32)

**User report:** stderr `Io error: Broken pipe (os error 32)` then
`thread 'main' panicked at src/lib.rs:197:34: called Result::unwrap() on an
Err value: ExitFailure(1)`.

**What the message is:** `Io error: …` is `wayland-backend`'s own
`WaylandError::Io` printed by its `log_error!` (it falls back to `eprintln!`
because scratchy doesn't enable the `log` feature) when writing to the
Wayland socket fails. `ExitFailure(1)` comes from winit's Wayland
`pump_events`: `self.connection.flush().is_err()` -> `set_exit_code(1)`;
`run_app` turns that into `Err(EventLoopError::ExitFailure(1))` and `run()`
in `lib.rs` does `event_loop.run_app(&mut app).unwrap()`. So the compositor
closed scratchy's connection. scratchy itself writes nothing to a pipe here.

**Reproduced exactly, twice:**
1. Kill the (nested) compositor while scratchy runs: identical two lines and
   the same panic.
2. Compositor alive (dwl, libwayland 1.24): a page whose `setTimeout`
   callback busy-loops 1.5 s, plus pointer motion over the window during the
   stall. 200 motion events: survives. 400 motion events: disconnected,
   same panic. The compositor drops a client whose socket buffer fills up
   because the client isn't reading. winit always binds
   `zwp_relative_pointer_v1` (`listen_device_events` is a no-op on Wayland),
   so each mouse move costs about 3 events.
3. Real page, no busy loop: en.wikipedia.org/wiki/Main_Page with the pointer
   moving every 2 ms (a 500 Hz mouse) during load and hover: **crashed 4/4
   runs**, 3 with JS on and 1 with `--no-js`. SoHo_Weekly_News and the
   portal survived the same test.

**Measured stalls** (gaps between consecutive `wl_pointer.motion` events
received, from `WAYLAND_DEBUG=1` timestamps, pointer moving every 20 ms,
`--no-js`, machine load ~8 on 8 cores, so absolute numbers are inflated):
portal ~180 ms per gap, SoHo ~530 ms, Main_Page 1.3–1.8 s. They recur
about every 2 s while the pointer moves. That is `HOVER_RESTYLE_INTERVAL`
pacing: every hover restyle runs `build_layout_tree_with_resources` (DOM
walk + cascade) synchronously in `App::redraw`. Other synchronous main-thread
work that can stall the same way: `handle_nav_result` (HTML parse +
`run_page_scripts`, up to `SCRIPT_BUDGET` = 5 s), and every timer or event
callback, each with its own 5 s budget.

**Fix:** QB1 (never panic; exit cleanly with a clear message) — DONE
(637fef5) — and QB3 (bound how long the main thread stays busy while
input is flowing) — DONE (dbf1610): hover restyles wait for the pointer to
be still 150 ms when the last build took >50 ms, timer/event callbacks
get 500 ms. Live (2 ms motion, 20 s, Main_Page): before 0/3 survive,
after 7/7 (JS on and off); busy-loop page: before dies, after survives.
Remaining stalls (the cascade itself, 350 ms Main_Page, 1.65 s SoHo first
build, when started by image arrival/JS/resize) go to Q32.

### R9 — JS DOM writes rebuild without the restyle path: periodic writes leave the page blank or missing text (bug, Q33c `80b5e0a`) — FIXED (QB2 d78587f)

**Repro:** a local copy of SoHo_Weekly_News with `<base href>` plus
`setInterval(function(){document.body.setAttribute('data-tick',
String(Date.now()))}, 300)` before `</body>`, loaded with `--socket`:
**the page stays completely blank** (screenshot after 25 s). With a 1000 ms
interval the header and infobox image show, but **every paragraph of body
text is missing**, and scratchy sits at ~150% CPU. The same file without the
interval renders fully. Real pages do this all the time (clocks, carousels,
ad rotators, "x minutes ago" updaters).

**Root cause:** `App::js_sync` sets `layout_dirty = true` and bumps
`layout_generation`, but never sets `restyle_requested`. So `redraw`
queues a plain rebuild: `restyle = false`, `hold_old_frame = false`,
`reuse = false`. The `AppEvent::ShapeResult` handler then shows every
*incomplete* progressive pass (`self.cached = Some(*laid)` when
`incomplete` and not holding). The `layout_generation` bump in `js_sync` also
makes the in-flight shaping result stale on arrival. When a DOM write lands
more often than a full shaping pass takes, no complete frame ever lands:
the screen shows an early partial pass (missing text) or nothing at all.
The hover path (`set_interaction_state`) already does this right: it sets
`restyle_requested` and `keep_scroll_on_rebuild`, and `hover_restyle_delay`
coalesces while `hold_old_frame || layout_dirty`.

Related, seen once: after `:set js` + rapid `l h l h` back to Main_Page,
the word "squat" was missing mid-sentence ("a type of toy with a / body")
for 20+ s. Not reproduced in 3 more tries or after `:refresh`. It looks like
the same "incomplete pass left on screen" symptom. Re-check after QB2.

**Fix:** QB2. `js_sync` now takes the restyle path and `layout_in_flight`
coalesces writes to one rebuild per complete shaping pass. Measured with the
recipe above (release, 20 s in): 300 ms timer: blank page, ~190% CPU before;
full article incl. paragraphs, ~100% CPU (one core, back-to-back rebuilds)
after. 1000 ms timer: header+image but no paragraph text, ~190% before; full
article, ~45-60% after. No-timer page: 0% idle. wikipedia.org renders with
globe. The `squat` symptom above was not re-checked live.

### R10 — The previous page's JS engine stays alive under `:help` (bug, Q33t `bec5507` / Q33d `46dfc45`) — FIXED (QB6, c150951)

**Repro:** a page with `document.addEventListener('keydown', …)` (logs each
key, `preventDefault`s `j`) and a 500 ms `setInterval` writing a body
attribute. Run `:help`, press `j` four times: stderr logs `old page keydown
:` and `old page keydown j` x4, so the old page still gets the keys and can
cancel the vim bindings. Worse, the help page **renders with its whole
second half missing**: the Commands table, the "Also demonstrated" heading
and the list text are gone, and the image sits alone above the `<hr>`. It
stays that way. The same `:help` from that page with `--no-js` renders
completely. Mechanism: the old timers' DOM writes call `js_sync` -> R9
rebuild churn on the help document.

**Root cause:** `App::show_help` swaps the document via `replace_document`
but leaves `self.js` (the old page's `JsEngine`) in place. `about_to_wait`
keeps running its timers, `js_key`/`js_click` keep dispatching into it, and
its dirty flag keeps triggering rebuilds of the help page.
`restore_saved_page` has no engine to restore either, because `SavedPage`
has no `js` field.

**Fix:** QB6 (done): the engine now lives in `SavedPage.js` while help is shown.

### R11 — User report: wikipedia.org "Thank you for donating recently!" dialog shows and Close does nothing — FIXED (QB4 a29738f; part (c) = QB5 ac60599)

**(a) Why the dialog was visible.** Not reproducible on `f3b06a8`, with JS
on or off. A full-window screenshot of https://wikipedia.org shows no
dialog, and a `:refresh` doesn't bring it back. Facts:
- The markup is `<dialog class="frb-iad-dialog">` with no `open` attribute.
  The page CSS never sets `display` on it, so the UA rule
  `dialog:not([open]) { display: none; }` (`src/default.css`, Q33c2
  `1b02dd5`) hides it.
- The page's JS never opens it on load. `showModal()` is only called from
  the click handler of the banner's "I already donated" link (`.frb-iad`),
  in `index-be66aee782.js` around column 23586. The banner itself only
  becomes visible (`banner--visible`) for geo US/CA/GB/IE/AU/NZ, `en`, in
  2026.
- Before Q33c2 (`1b02dd5`, committed 14:20) every closed `<dialog>` rendered
  in flow. Q33c2's own TODO entry describes exactly this dialog. The
  installed `~/.cargo/bin/scratchy` (14:43) contains the rule.
- So the user was almost certainly running a process started from a binary
  older than `1b02dd5`. A live window check on the current install closes
  this.

**(b) Why Close did nothing.** Close is a plain `<button autofocus
class="frb-dialog-action frb-iad-dialog-close">`, not in a `<form
method=dialog>`. The page wires it with
`querySelectorAll(".frb-iad-dialog-close").forEach(e =>
e.addEventListener("click", () =>
document.querySelector(".frb-iad-dialog").close()))`. Q33d's click dispatch
does reach buttons: in a fixture, a `<button>` click listener logged on a
real injected click. But `HTMLDialogElement` has no `close`, `showModal`,
`show`, `open` or `returnValue` in `js_dom.rs`'s `DOM_PRELUDE`. Fixture
output: `typeof showModal undefined typeof close undefined`. Clicking a link
whose handler calls `showModal()` logs `Uncaught TypeError: not a function`.
On the old binary the dialog also had no `open` attribute to remove, so
Close could never hide it. Fix: QB4 (dialog API plus modal rendering).

**(c) The portal's remaining `TypeError: not a function` at
`index-be66aee782.js:1:18546`** (the only `--js-report` error left on the
portal). It is not related to the Close button. Source around that column,
inside a `doWhenReady` (DOMContentLoaded) handler:
`let t=i("searchInput"),a=i("searchLanguage");if(t){void 0===t.autofocus?t.focus():window.scroll(0,0);…}a.addEventListener("change",…)`.
`autofocus` isn't reflected, so it is `undefined`, and that takes the
`t.focus()` branch. `Element.prototype.focus` doesn't exist, so the call
throws. Fixture: `typeof el.focus`, `el.autofocus` and `typeof
window.scroll` are all `undefined`. Because it throws, the rest of that
handler never runs: the `change` listener on the search-language `<select>`
and the `<link rel=preconnect>` for the chosen language. Fix: QB5 (`focus`/
`blur`, `autofocus` reflection, `window.scroll/scrollTo/scrollBy`).

### Other things seen (filed or noted)

- Unreachable host: wire answers `502` with a `text/plain` body (`wire:
  upstream request failed: … Dns Failed: … Name or service not known`).
  scratchy shows only `can't render this content type: "text/plain"`,
  because `dispatch_wire_response` rejects every MIME type except HTML and
  images. The user never sees the reason. QB7. (A missing wire socket is
  handled well: a clear message, and the request is printed to stdout.)
- bbc.com/news with JS: 13 of 14 errors are `Error: chunk path empty but
  not in a worker` from Next.js/turbopack chunks. The runtime reads
  `document.currentScript` (`if(e)return{src:e.getAttribute("src")}`), which
  doesn't exist. QB8. (The page still looks better with JS than without:
  without JS its nav drawer renders expanded.)
- stderr closed early (`scratchy … 2>&1 | head -1`, page logging every
  200 ms): exit code 101, a panic. `eprintln!` panics on `EPIPE`. Not the
  user's crash (that message is different), but the same family. QB9.
- Portal, JS on, after clicking "Read Wikipedia in your language": the
  language list renders as one long column. The CSS is
  `.langlist>ul{column-width:11.2rem}`, and CSS multi-column isn't
  implemented. With `--no-js` the list is inline rows, which reads better.
  It also covers the button's own text. Not queued as a bug, since
  multi-column was never claimed. A new feature item would be M-sized.
- Hover after a fast 50-point sweep: the final hovered link is correct
  once the coalesced `HoverFlush` lands, about 2 s later.
- Main_Page/SoHo/Commons header icons render as dark grey squares (mask
  images, Q35a, known). Tofu for Bengali/Tamil/etc. is a missing system
  font, not a scratchy bug. SoHo at a ~470 px tile width renders infobox
  rows as one run-together line ("TypeAlternative weeklyFounder…");
  unverified against a real browser, not filed.
- Error stacks for events dispatched from Rust say `at <eval> (timers:1:1)`,
  because `JsEngine::eval_value` names every eval `timers`. Cosmetic, folded
  into QB8.

---

## R7 — Fetched images lost when scripts delay NavResult (bug) — FIXED (b40c62a)

(Archived: full text in REGRESSIONS-FIXED.md.)

## R5–R6: Q33a script runner bugs seen on https://wikipedia.org — FIXED (R5 e76d162, R6 a56d276; 2026-11-15)

(Archived: full text in REGRESSIONS-FIXED.md.)

## R1–R4: white screen past one viewport, hover-driven relayout storms, slow scrolling — ALL FIXED (2026-11-13)

(Archived: full text in REGRESSIONS-FIXED.md.)

## WPT audit (Q40, opened 2026-11-14)

Findings from running `/home/loupax/src/wpt` reftests against features
TODO.txt marks done. Format per area: counts, failure groups by root
cause (example tests, wrong render), claiming Q item + hash, triage
(BROKEN / PARTIAL / NOT CLAIMED). See Q40b in TODO.txt.

### How the audit was run (Q40b)

- Runner: `scratchy --wpt /home/loupax/src/wpt --subset LIST` at 800x600 with Ahem
  loaded. Counts below are over EVERY eligible reftest in the area (a file with
  `rel=match|mismatch`, not a `-ref`, no `<script>`/`reftest-wait`), taken from
  throwaway lists, not from the committed `wpt-subset.txt`. The committed subset
  gets ~12 evenly spaced tests per area as a regression guard.
- Two runner fixes were needed first, both in `src/wpt.rs` (not engine bugs):
  `<![CDATA[` markers in `.xht` `<style>` corrupted the first rule (b86c983),
  and root-relative reference hrefs (`/css/reference/...`) counted as CRASH (6d523e5).
  Counts here are after both. There are no crashes left in the audited areas.
- "pass" can be vacuous: a test and its reference that both render wrong the same
  way match. Pass counts are an upper bound on correctness. Fail counts are exact.
- "flipped" = tests that fail raw but pass when a scratch copy of the test tree
  has (a) `in/cm/mm/pc` lengths rewritten to `px` and (b) `url(file)` images
  inlined as `data:` URIs. It measures how much a failure area is hidden behind
  W1 and W19 below. The copy lives outside the repo.
- "marker counts" (`tests mentioning X among the failures`) are grep upper bounds,
  not root-cause counts. A root cause is only listed as a group after a minimal
  repro was rendered and the wrong output seen.
- `.xht` parsed as HTML, so inline `svg:svg` and other namespaced XML do not work
  for the wrong reason in some tests (noted under W20).
- Not reproduced: the Q40a lead "Ahem `<p>` lands 2px lower than the same content
  in a `<div>`". With margins zeroed, `<p>` and `<div>` Ahem text land on the same
  row. The apparent offset is the UA `p` margin (1em). Dropped.

### Root causes found (referenced from the area sections)

Each has a minimal repro that was rendered with the runner. Claim = the Q/history
item that says the feature is done.

| id | what is wrong | claimed by | triage |
|----|---------------|------------|--------|
| W1 | `in`, `cm`, `mm`, `pc`, `Q` lengths are not parsed (declaration dropped). `width:1in` renders full width. | Q42 (b79a100) | FIXED (also `padding` rejects negative lengths) |
| W2 | `ex`, `ch`, `ic`, `lh`, `rlh`, `cap` units not parsed. | Q42 (b79a100) for `ex`/`ch` (half an em) | PARTIAL (rest: Q42a) |
| W3 | `<br>` is not implemented anywhere (`grep '"br"' src/*.rs` is empty). `a<br>b` renders `a b` in a block, float, inline-block, abspos. | Q41 (09b8e63) | FIXED (leftovers: Q41a) |
| W4 | `border: solid black` / `border-style: solid` with no width computes width 0; should be `medium` (3px). `border-bottom: solid` likewise. | Q4 3e80aae | BROKEN |
| W5 | Negative values that CSS says are invalid are accepted: `padding-right:-1px` shrinks the box, `max-height:-1px` clamps to 0 (should be ignored), negative `flex-basis`, negative `border-*-width`. | pre-Q box model | FIXED Q53 (4662a7f) |
| W6 | A block with `width:auto` whose borders+padding exceed the containing block is clamped to the container width; it should overflow (48px container, 2x32px borders paints 48 wide, expected 64). | pre-Q box model | FIXED Q54 (a4f43e4) |
| W7 | Vertical margins do not collapse through an empty block (`height:auto` with no content, or `height:0`): `margin-bottom:50` + empty div + `margin-top:30` gives 80, expected 50. | pre-Q box model | BROKEN |
| W8 | A float that follows inline content on the same line (`<div style=inline-block></div><div style=float:left>`) is placed below the line box instead of at the top of the current line beside it. | float 5fc83ce, Q34 | FIXED Q49 (ab0678c); leftovers: Q49a |
| W9 | Shrink-to-fit width (float, inline-block, table, abspos) is wider than the text: with Ahem 20px, `float:left` "X" is 40px wide instead of 20 (+1em); with sans 20px "PA" is +3px. Adjacent floats of text show a visible gap. | float 5fc83ce, inline-block 244a8f4 | Q43 (c8e334c) removed the extra trailing space; PARTIAL (Q43a) |
| W10 | `position:absolute` with `width:auto` and `left`/`right` auto/one side set stretches to the containing block edge (to x=800) instead of shrink-to-fit. | position bb97e60 | FIXED Q44 (0e42655); margins with left+right stretch remain (Q44a) |
| W11 | An abspos box (auto height) whose only children are floats gets height 0 and paints no background (an abspos box is a BFC and must contain floats). `overflow:hidden` on it hides the bug. | position bb97e60, Q34 | FIXED Q45 (2ce2b2a) |
| W12 | Flex items with an explicit `width` keep it when growing/shrinking: siblings move as if the item grew, but it paints at its specified width. `flex:1` items without `width` are fine. | flex 440e03b | FIXED 9abe87b (row containers) |
| W13 | Single-line flex container with a definite `height`: cross size is taken from the tallest item, not the container. `align-items:center|flex-end` aligns inside the tallest item. | flex 440e03b | FIXED Q47 (d0d8e67, row containers) |
| W14 | A block-level `display:table` element (no `float`, no `width`) fills the container and spreads its cells across it (two 40px cells land at x=0 and x=400); it should shrink-to-fit. Only the floated case works (Q34c acbcb00). | table 53bab7a, Q34c | FIXED Q50 (bf93cba) for plain tables; table-internal parts remain (Q50a) |
| W15 | A table's `width` is content-box here; browsers (and WPT refs) treat it as including the table's border+padding. `table{width:100%;border:1px}` paints 2px too wide. | table 53bab7a | FIXED Q51 (e09c450) |
| W16 | No `border-collapse`, `border-spacing` or `table-layout:fixed` support (NOTES.md says so). | table 53bab7a scope cut | PARTIAL |
| W17 | A `z-index:-1` positioned child of a non-stacking-context positioned parent paints above the parent's background; it should paint below it (in the ancestor stacking context's negative layer). | none explicit | FIXED Q55 (53e9727); in-flow relative negatives remain (Q55a) |
| W18 | `<img width="100%">` (percentage in the HTML attribute) is treated as unset; image paints at `height` x intrinsic aspect (20px for `height=20`). | image support | FIXED Q56 (bdbc410) |
| W19 | `background-image`/`mask-image` `url()` only decodes `data:` URIs; file/network URLs never load in `resolve_background_image`. | Q16 93209c2 (documented) | PARTIAL |
| W20 | Inline `<svg:svg xmlns:svg=...>` (prefixed) and `<object data=x.svg>` are not rendered. | Q12 af12757 (`<img>` SVG only) | NOT CLAIMED |
| W21 | `:has()` never matches; `:nth-child(n of S)`, `:host`, `:is/:where` with namespaces not handled. | Q2 66bdc60 lists neither | NOT CLAIMED |
| W22 | `var()`: (a) invalid-at-parse values (`var(--b,!)`, `;`, `!important !important`) are accepted; (b) invalid-at-computed-time resolves as "declaration dropped" instead of `unset`; (c) escaped custom property names (`--\30`) do not resolve; (d) adjacent `var(--b)var(--c)` fails. | Q13 b23fd53; Q57 dde81b5 | FIXED |
| W23 | `list-style-type:"string"` and `<ol reversed>` are ignored (shows `1.`, `2.`). | Q19f cfa71ec (counters only) | NOT CLAIMED |

#### CSS2/normal-flow (claimed: baseline box model, no Q item)

698 eligible (+53 skipped for JS) | pass 313 | fail 384 | 1 no-reference.
Flipped by the W1+W19 rewrite: 189, so about half of all failures are W1 alone
(`1in` squares). Residual after the rewrite: 205 failures.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W1 units | 189 flipped (`max-height-006`, `height-006`, `width-applies-to-*`) | `width:1in` ignored, box full width | NOT CLAIMED |
| W5 negative values | 13 mention one (`max-height-001`, `max-height-012`) | `max-height:-1px` collapses a 96px square to 0 | FIXED Q53 (4662a7f) |
| W6 over-constrained width | `blocks-011`, `blocks-012` | box 48px wide instead of 64 | FIXED Q54 (a4f43e4) |
| W7 empty-block margins | `margin-collapse-through-zero-height-block`, `margin-collapse-after-float-001` | gap 300 instead of 200 | BROKEN |
| W3 `<br>` | 17 mention it (`width-percentage-001` reference) | two images in the reference sit side by side | FIXED by Q41 (09b8e63) |
| W9 shrink-to-fit | `inline-block-non-replaced-width-001` | red strip: box one em too wide | BROKEN |
| W8 float after inline | `inlines-013` | float lands on the next line | PARTIAL Q49 (ab0678c); Q49a |
| W11 abspos + float | `block-formatting-context-height-001` (Q40a lead) | paints nothing | FIXED Q45 (2ce2b2a) |
| W20 `<object>` svg | `replaced-intrinsic-*` (5) | nothing drawn | NOT CLAIMED |
| ungrouped | 28 failures with none of the markers, plus `inline-table-zorder-*` (5), `inline-block-zorder-*` (3) | not yet root-caused | see Q40c |

#### CSS2/margin-padding-clear (claimed: baseline box model; float/clear 5fc83ce)

684 eligible (0 JS) | pass 277 | fail 406 | 1 no-reference. Flipped 198 (W1:
`1in`, `5em` margins). Residual 224.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W1 units | 198 flipped | `padding:1in` ignored | NOT CLAIMED |
| W5 negative padding | 33 mention a negative length (`padding-right-001`, `-012`) | `padding-right:-1px` shrinks the box by 1px | FIXED Q53 (4662a7f) |
| W7 empty-block margins | `margin-collapse-012`, `margin-collapse-022` | 20px gap instead of 60 | BROKEN |
| W14/W16 table internals | 52 mention `display:table-*` (`margin-applies-to-001..015`) | table drawn full width, no border-spacing | BROKEN/PARTIAL |
| W9/W10 | abspos/float references | | BROKEN |
| ungrouped | 43 with no marker (`margin-collapse-001`: inline margins in Ahem rows) | not yet root-caused | Q40c |

#### CSS2/box-display (claimed: baseline; inline-block 244a8f4; flow-root a4852af)

86 eligible (+34 JS) | pass 44 | fail 42. Flipped 6.

| group | tests | wrong render | triage |
|-------|-------|--------------|--------|
| `display:table-*` | `display-*` 12 (12 mention table internals) | W14 | BROKEN |
| containing block | `containing-block-*` 9 | abspos boxes stretch (W10) | BROKEN |
| block-in-inline | `block-in-inline-*` 8 (`block-in-inline-margin-with-leading-text`) | margins around the split differ | BROKEN (not confirmed on a minimal repro, see Q40c) |
| residue | 12 with no marker | | Q40c |

#### CSS2/floats + floats-clear (claimed: float 5fc83ce, Q34)

324 eligible (+12 JS) | pass 69 | fail 255. Flipped 37 (W1).
Marker counts among failures: 212 mention `float`, 57 `inline-block`, 40 `position:absolute`.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W8 float after inline | `floats-001` | blue float lands below the black box instead of at its left inner edge | FIXED Q49 (ab0678c) |
| W9 shrink-to-fit | `float-non-replaced-width-*` (6), `float-replaced-width-*`, `clear-applies-to-000` | `PA SS` instead of `PASS` | BROKEN |
| W14 table internals | `float-applies-to-*` (12) | `display:table-row-group; float:right` | BROKEN |
| W1 units | 37 flipped | | NOT CLAIMED |
| clearance | `clear-clearance-calculation-*` (5), `margin-collapse-clear-*` (10) | spacing after a cleared block differs; not root-caused | Q40c |
| ungrouped | 4 with no marker | | Q40c |

#### CSS2/positioning + css-position (claimed: position bb97e60, sticky 636f369)

CSS2/positioning: 513 eligible (+7 JS) | pass 154 | fail 359. Flipped 138 (W1).
css-position: 134 eligible (+119 JS) | pass 38 | fail 96. Flipped 0.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W1 units | 138 flipped | | NOT CLAIMED |
| W10 abspos auto width | `absolute-non-replaced-width-001`, `-018` | box fills the containing block instead of shrink-to-fit | BROKEN |
| W11 abspos + float child | `block-formatting-context-height-001` (normal-flow) | abspos box paints no background | FIXED Q45 (2ce2b2a) |
| W20 inline `svg:svg` | 26 mention svg (`absolute-replaced-width-009`, `-032`) | svg missing | NOT CLAIMED |
| W18 `<img width="100%">` | 11 | | FIXED Q56 (bdbc410) |
| css-position | `position-absolute-in-inline` 12, `inline-level-absolute-in-block-level-context` 6, `position-relative` 8, 38 mention `<br>` (W3), 6 `position-sticky-*` | abspos inside inline parents, static position | BROKEN (re-measure after W10) |
| vertical writing modes | ~30 `vrl-*`/`vlr-*` in css-position | | NOT CLAIMED |

#### CSS2/tables + css-tables (claimed: table 53bab7a, Q34c acbcb00; Q36 colspan/rowspan is in flight elsewhere)

CSS2/tables: 250 eligible (+123 JS) | pass 36 | fail 214.
css-tables: 145 eligible (+31 JS) | pass 43 | fail 102.
Measured before Q36 (colspan/rowspan) merged. Flipped 0 in both (units are not the issue here).

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W16 no `border-collapse`/`border-spacing`/`table-layout:fixed` | CSS2/tables: 56 `border-collapse`, 85 `table-layout`, 112 `border-spacing` (`collapsing-border-model-001`, `fixed-table-layout-*` ~50); css-tables: 40 `border-collapse` (`collapsed-borders-painting-order` 13) | table border and cell border both painted, widths from content | PARTIAL (documented scope cut) |
| anonymous table objects | `table-anonymous-objects-*` 61 (`-017`, `-059`) | loose cells/rows are not wrapped in an anonymous row/table; cells overlap | NOT CLAIMED (Q34c covers `display:table-caption` only) |
| W14 block `display:table` | many `border-*-applies-to`, `float-applies-to` | fills the container | FIXED Q50 (bf93cba); remainder Q50a |
| W15 table width | `block-formatting-contexts-001` reference | 2px too wide | FIXED Q51 (e09c450) |
| caption-side | `caption-side-applies-to-*` 11 | `caption-side` on non-captions changes layout | BROKEN (not confirmed on a minimal repro) |
| css-tables misc | `percentage-sizing-of-table-cell-children` 5, `table-intrinsic-size` 4, `subpixel-*` 5, `visibility-collapse-*` 4 | | NOT CLAIMED |

#### CSS2/linebox (claimed: Q9 f43f902 `vertical-align`; baseline `line-height`)

191 eligible (0 JS) | pass 16 | fail 175 (8% pass, the worst area). Flipped 13 (W1).
Clusters: `line-height-*` 60 (+13 `-applies-to`), `vertical-align-*` 49 (+13), `inline-formatting-context-*` 14.
Marker counts: 121 mention `position:absolute` (W10), 11 `ex` units (W2), 10 negative lengths (W5, `line-height:-1px`).
These tests put a reference box beside the test box with `font: 1in Ahem` and
absolute positioning, so they sit behind W1, W10, W2 and W5. Not root-caused
beyond that.

| group | tests | triage |
|-------|-------|--------|
| W10, W1, W2, W5 gating | most of the 175 | BROKEN / NOT CLAIMED |
| `vertical-align-*` (Q9), `line-height-*` | re-measure after W10/W1 | Q40c |

#### css-flexbox (claimed: `display:flex` 440e03b, Q5 1000951, Q23/Q23b/Q23c)

928 eligible (+70 JS) | pass 243 | fail 685 (26% pass). Flipped 0.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W12 explicit width ignored when flexing | `flex-grow-001/002/006-009`, `flex-shrink-001/002/003/006` (273 mention flex-grow/shrink) | item paints at `width` while siblings move | FIXED 9abe87b |
| W13 cross size | `align-self-002..016` (9); 147 mention align-* | items hug the top; `flex-end`/`center` are relative to the tallest item | FIXED Q47 (d0d8e67); `-015/-016` left (Q47a) |
| W5 negative basis | `flex-basis-003/004` | negative `flex-basis` honoured | BROKEN |
| writing modes, fragmentation | `flexbox-writing-mode` 15, `flexbox-break-request-*` 8 | | NOT CLAIMED |
| abspos children | `abspos/*` 9, `position-absolute-*` 6 | uses W10 | BROKEN |
| not root-caused | `flex-minimum-height-flex-items-*` 16, `percentage-heights` 15, `col-wrap` 14, `balance-*` 28, `intrinsic-size` 24, `aspect-ratio-*` 19, `flexbox-flex-basis-content-*` 8 | | Q40c |

#### CSS2/backgrounds + css-backgrounds (claimed: Q16 93209c2, Q25 3c7a36a)

CSS2/backgrounds: 336 eligible (+3 JS) | pass 66 | fail 270. Flipped 98.
css-backgrounds: 488 eligible (+90 JS) | pass 105 | fail 383. Flipped 31.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W19 only `data:` URI images | 98 + 31 flipped (`background-053`, `background-position-*` 74) | image never drawn | PARTIAL (documented) |
| W18 `<img width="100%">` | 49 references (`background-001`: reference image paints as a small square) | | FIXED Q56 (bdbc410) |
| W1 units | `background-position-054` (`+0mm`) | | NOT CLAIMED |
| css-backgrounds | `background-size-*` 55, `background-origin/-clip` 12, `box-shadow-*` 12 | mostly need url images (W19); rest not root-caused | PARTIAL / Q40c |

#### CSS2/borders (claimed: Q4 3e80aae, Q4b c13703c, Q4c 96bb7b1)

504 eligible (+2 JS) | pass 242 | fail 262. Flipped 31 (W1: `border-*-width-*` with
`in/cm/pc`). 23 tests that passed raw fail after the rewrite: they passed only
because test and reference both ignored the unit.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W4 width-less border | `inline-replaced-width-001` (normal-flow), any `border: solid black` | border not painted | BROKEN |
| W5 negative width | `border-top-width-012` (`-1pt` must be invalid, i.e. medium); 15 mention negatives | | BROKEN |
| W16 border-collapse on internal table parts | `border-*-applies-to-*` (~109 mention table internals) | hollow square missing | PARTIAL |
| W1 units | 31 flipped | | NOT CLAIMED |
| ungrouped | 86 with no marker | not root-caused | Q40c |

#### css-lists (claimed: Q19c 76a9d25 `::marker`, Q19f cfa71ec counters)

155 eligible (+42 JS) | pass 57 | fail 98. Flipped 0.

| group | tests | wrong render | triage |
|-------|-------|--------------|--------|
| W23 `list-style-type:"str"`, `reversed` | `list-style-type-string-*` 8, `foo-counter-reversed` 11, `li-value-reversed` 8, `counter-reset-reversed-*` 7 | `1.` shown instead of the string; counters count up | NOT CLAIMED |
| counter scope | `counters` 6, `counters-scope` 4 | nesting scope differs | PARTIAL (not confirmed on a minimal repro) |
| `::marker` layout | `outside-marker-*`, `nested-marker-*`, `marker-*` ~12 | | PARTIAL |
| residue | 77 without marker | | Q40c |

#### selectors (claimed: Q2 66bdc60, Q31 a4b93c4)

120 eligible (+109 JS: most selector tests are testharness) | pass 43 | fail 77.

| group | tests | wrong render | triage |
|-------|-------|--------------|--------|
| W21 `:has()` | `has-style-sharing-*` 15 | `:has(> span)` and `:has(+ .x)` never match (minimal repro) | NOT CLAIMED |
| `:nth-child(n of S)` | `nth-child-of-*` ~10 | | NOT CLAIMED |
| `:host` / featureless | `featureless-*` 5 | | NOT CLAIMED |
| `:dir()` RTL | `dir-style-*` 3 | engine is LTR-only (Q2 says so) | PARTIAL |
| default namespace | `is-default-ns-*`, `not-default-ns-*` 6 | | NOT CLAIMED |
| `:lang()` | `lang-*` 10 | plain `:lang(en)`/`:lang(fr)` work on a minimal repro; the tests use wildcard/string forms | PARTIAL |

#### css-variables (claimed: Q13 b23fd53)

181 eligible (+2 JS) | pass 109 | fail 72. Flipped 0.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W22a invalid-at-parse accepted | `variable-declaration-11/12/13/23` | `--a:var(--b,!)` is accepted and wins (crimson); it must be dropped | FIXED (Q57 dde81b5) |
| W22b IACVT | `variable-declaration-24` | invalid at computed time falls back to an earlier declaration (red) instead of `unset` (inherited green) | FIXED (Q57 dde81b5) |
| W22c escaped names | `variable-declaration-31/42`, `variable-reference-21/24` | `--\30` and unicode names unresolved | FIXED (Q57 dde81b5) |
| W22d adjacent `var()var()` | `variable-declaration-53` | not substituted | FIXED (Q57 dde81b5) |
| `@supports` with var() | `variable-supports-*` 12 | conditions containing `var()` mis-evaluated | FIXED (Q57 dde81b5) |
| `var()` in `content`, shorthands | `variable-reference-12`, `vars-background-shorthand`, `vars-font-shorthand` | | PARTIAL |

#### css-values (claimed: Q11 33bd57f calc/min/max/clamp, Q14 35c826a viewport units, pt 17dd346)

170 eligible (+44 JS) | pass 52 | fail 118. Not mirrored.

| group | tests | wrong render | triage |
|-------|-------|--------------|--------|
| W2 units | `ch-unit` 13, `ic-unit` 13, `ex-unit` 2, `lh-unit` 2, `q-unit-case-insensitivity` 2 | declaration dropped | NOT CLAIMED |
| `calc-size()` | `calc-size-*` 20 | not parsed | NOT CLAIMED |
| `attr()` typed | `attr-style-sharing` 5 | | NOT CLAIMED |
| angle units, `calc` rounding | `angle-units` 5, `calc-rounding` 2 | | NOT CLAIMED |
| viewport units in odd contexts | 4 | | PARTIAL (Q14) |

### Q40c: the areas Q40b did not reach (2026-11-15)

Same method as Q40b (see "How the audit was run"). Whole-area counts come from throwaway
lists (every `rel=match|mismatch` test, not a `-ref`, not under `reference/` or `support/`),
run with `scratchy --wpt WPT --subset LIST --expected /dev/null -v`; each area also adds ~12 evenly
spaced tests to `wpt-subset.txt`. Q40b root causes W1-W23 above; new ones follow. Before starting, the
committed subset re-ran clean (187 pass / 203 fail / 6 skipped, no change from `wpt-expected.txt`).
Every root cause below was confirmed on a minimal inline test/reference pair rendered with the runner.

| id | what is wrong | claimed by | triage |
|----|---------------|------------|--------|
| W24 | `@layer` blocks are dropped whole (even a lone `@layer { #t{background:green} }` paints nothing); `revert-layer` is stored as `revert`, which behaves as `unset`. | none (style.rs notes "no cascade layers") | NOT CLAIMED |
| W25 | The `all` shorthand is not implemented (`div{background:red; all:initial}` still paints red). `all` is parsed only as a media type. | none | NOT CLAIMED |
| W26 | `@scope` is not implemented (`@scope{div{...}}` rules are dropped). | none | NOT CLAIMED |
| W27 | `revert` has no UA-origin step: `display:inline; display:revert` on a `<div>` stays inline (computes as `unset`, i.e. initial), expected `block`. | Q7 38ca872 (CSS-wide keywords; documented as unset) | PARTIAL |
| W28 | `@import` inside an inline `<style>` is never fetched (only wire-fetched stylesheets expand imports). Same with a plain file import and no media list. | Q18 3a7054b (documented scope cut) | PARTIAL |
| W29 | Every text node gets a trailing space (`inline_text` pushes `' '` after `normalize_whitespace`, which strips both ends), so an inline element boundary inside a word adds a gap: with Ahem 20px `<div>a<span>b</span></div>` paints `b` at x=40, not 20 (`ab` is 40 wide, the span form 60); `<span>a</span><span>b</span><span>c</span>` has two gaps. | inline layout (pre-Q); Q43 c8e334c trimmed only the last item of a trial pass | FIXED (Q59 510e93f) |
| W30 | `::first-line` is applied after line breaking: with `div::first-line{font-size:40px}` in a 160px Ahem box the line is broken at the unstyled size, so big words and small words share the first line and the line height ignores `line-height`/`font-size` of the pseudo. | Q19e2 0e57f9d | BROKEN |
| W31 | `::first-letter` does not take following punctuation: `T.est` and `(T)est` colour only `T.`/`(T` (the `.`/`)` stay unstyled). Leading punctuation (`"T`) works. | Q19e 17346a8 | BROKEN |
| W32 | `::marker` honours only `color` and `content`: `li::marker{font-size:40px}` is ignored (marker stays 20px), and `text-transform`, `letter-spacing`, `tab-size`, `hyphens`, `white-space`, `unicode-bidi`, `text-align`, `display:grid` do nothing. | Q19c 76a9d25 (scope: color, content) | PARTIAL |
| W33 | `letter-spacing` and `tab-size` are ignored everywhere (`letter-spacing:10px` on `ab` is 40px wide; `tab-size:2` still uses 8). `word-spacing` works. | Q26 parked | NOT CLAIMED |
| W34 | Multi-value `margin`, `padding`, `border-width`, `border-color`, `border-style`, `border-radius`, `inset` shorthands apply only the FIRST value to all four sides: `margin:5px 30px` renders exactly like `margin:5px` (same in a stylesheet), `margin:0 0 0 30px` has no left margin. Longhands work. The parse branches call `parse_length_percent` once and copy the value to every side. | box model (pre-Q); Q46a noted `margin: 0 10px` "seemed to give margin 0" | FIXED (6b8f773) for margin/padding/inset/-block/-inline; border-* already used `parse_box4`; `border-radius` per-corner is Q58a |
| W35 | `<q>` has no default quote marks: `<q>a</q>` renders `a` (UA stylesheet has no `q::before/::after{content:open-quote/close-quote}` and `quotes` has no language default). With explicit `quotes` + `content:open-quote` rules the marks and nesting work. | Q19 bb7c64d (`open-quote`/`quotes` parsed) | BROKEN |
| W36 | `clip-path` (basic shapes, `path()`, `url(#id)`, geometry boxes) and the `clip` property are not implemented: `clip-path:inset(0 50px 0 0)` paints the whole box; `clip:rect()` on an abspos box paints the whole box. `mask-mode`, `mask-composite`, `mask-clip`, `mask-origin` also do nothing. | none (Q29 312a345 is `mask-image` + size/position/repeat for icons only) | NOT CLAIMED |
| W37 | `text-shadow`, `text-emphasis-*`, `text-decoration-thickness`, `text-underline-offset`, `text-underline-position`, `text-decoration-skip-ink`/`-skip-spaces`, `text-decoration-inset` are all ignored (each paints exactly like the property absent; confirmed for `text-shadow:20px 0 red`, `thickness:10px;offset:10px`, `text-emphasis:dot`). `text-decoration-style: dotted` works. | `text-decoration-style`/`-color` item (PROMPTS v33; line, style, colour only) | NOT CLAIMED |
| W38 | `hsl()`/`hsla()`, `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()` and `color(...)` are parsed by cssparser-color but `to_text_color` (css.rs) only converts `CssColor::Rgba`, so the declaration is dropped: `background:hsl(0 100% 50%)` paints nothing, not red. `rgb()` (modern and legacy syntax), hex, named, `transparent` work. | none (colour parsing predates the Q list) | BROKEN (`hsl` is common on the web) |
| W39 | System colours (`ButtonBorder`, `ActiveBorder`... only `Canvas` works), `color-mix()`, relative colours (`rgb(from green r g b)`), `light-dark()` are unsupported. | none | NOT CLAIMED |
| W40 | A media query beginning with `not` never matches: the parser sets `never = true` for the `not` keyword instead of negating the query. `@media not all and (min-width:100000px)` and `@media not (min-width:100000px)` both skip their rules (expected: apply). | `@media` 1282285 | BROKEN |
| W41 | Only `min-width`, `max-width`, `prefers-color-scheme`, `prefers-reduced-motion` are evaluated. `height`, `aspect-ratio`, `orientation`, `resolution`, `hover`, `color-gamut`, `scripting`, range syntax (`(width >= 400px)`), `or`, nested `(( ) and ( ))` all make the query never match. | `@media` 1282285 (documented feature list) | PARTIAL |
| W42 | `@supports` grammar is lenient: `not (a) and not (b)` and `not (a) or (b)` (invalid, must not apply) apply their rules; `(unknown) or (color:green)` is treated as invalid instead of false-or-true; `not ()` and `(margin: 0 or padding: 0)` mis-evaluate; `selector(:has(a))` and `at-rule(@media)`, `font-tech()` are false. Plain `(prop:value)`, `and`, `or`, `not (prop:value)`, `selector(a > b)` work. | Q10 a4f4d91 | PARTIAL |
| W43 | Container queries (`@container`, `container-type`, container units) are not implemented. | none | NOT CLAIMED |
| W44 | `aspect-ratio` is not implemented (`width:100px;aspect-ratio:2/1` paints no height; `aspect-ratio:1` too). Also `contain-intrinsic-size`, `fit-content(<length>)`, `stretch`/`-webkit-fill-available`. | none (Q12 images keep their own intrinsic ratio only) | NOT CLAIMED |
| W45 | `width:min-content` counts the trailing collapsible space of each word: Ahem 20px `XX XX` gets 60px instead of 40 (the box is one em wider than its widest word). `max-content` and `fit-content` keywords are right on the same text. | Q20f 01a1a77 | BROKEN |
| W46 | HTML presentational attributes are not mapped to CSS: `<table border=5>` (no border), `<table width=100>`, `<td width=100>`, `<table bgcolor>`, `<body bgcolor/text>`, `<font color>` (text stays black), `<div align=center>`, `<p align=right>`, `<hr width/size/color/noshade>`, `<ol type=a>`. | none | NOT CLAIMED |
| W47 | A word wider than its line box is broken across lines glyph by glyph by default (cosmic-text `Wrap::WordOrGlyph`): Ahem 20px `XXXXXXXXXX` in a 100px box is two lines (40px tall); `overflow-wrap:normal` should overflow on one line. | Q20d 7cddd62 (`overflow-wrap`/`word-break` keywords) | BROKEN |
| W48 | `-webkit-line-clamp`/`line-clamp`/`block-ellipsis`, `scroll-marker`/`scroll-marker-group`, `overflow-clip-margin`, `text-overflow:<string>` are not implemented; `overflow-body-propagation` (body `overflow` propagated to the viewport) is not. | none (Q21 3a5e687 is `overflow-x/-y` + `text-overflow:ellipsis|clip`) | NOT CLAIMED |
| W49 | Named grid lines are dropped: `grid-template-columns:[a] 50px [b] 50px [c]` makes the whole declaration invalid (single auto column), so `grid-column:b` has no effect. Also `repeat(2,[x] 50px)`. | none (Q15c is numeric placement) | NOT CLAIMED |
| W50 | `display:inline-grid` and `inline-flex` lay out as block-level boxes (a following `inline-block` wraps to the next line instead of sitting beside the grid). | grid `display:grid` item; no explicit inline-grid claim | BROKEN |
| W51 | Grid alignment is missing: `justify-content`, `align-content`, `justify-items`, `align-items`, `justify-self` ignored (`justify-items:end` leaves a 20px item at x=0); items do not stretch to an explicit row height (`grid-template-rows:20px` + auto-height item paints nothing); `order` is ignored in grid; `grid-auto-flow:column`/`dense` is not implemented. | Q15 (not claimed); TODO.txt notes `grid-auto-flow: row` only | PARTIAL |
| W52 | Acid2 (`acid/acid2/test.html`, compared directly with `px-reference.html`; the wpt `reftest.html` wrapper needs JS and an iframe) renders only the fallback text ("Standards compliant... Take The Acid2 Test") where the smiley face belongs. The face is built from `<object>` data URIs and fixed/abspos stacking; `<object>` is not rendered (W20). | none | NOT CLAIMED |
| W53 | Clearance: a `clear`ed block's own `margin-top` is added below the float instead of being absorbed by clearance. Float 0-50px, then `clear:left;margin-top:10px` paints at y=60; the top border edge belongs at y=50. | float/clear 5fc83ce | BROKEN |
| W54 | Flex items have no automatic minimum size (`min-height:auto`/`min-width:auto`): in a column flex container with `height:10px`, an item with `height:100px` is shrunk to 10px (green 10px tall); with `flex-shrink:0` it stays 100px. The spec floors it at `min(content, specified size)`. | flex 440e03b, Q5 1000951 | BROKEN |
| W55 | An abspos box whose height comes from `top:0;bottom:0` is not a definite containing-block height for percentage-height children: `height:50%` inside it (block or column flex) resolves to auto and paints nothing; with an explicit `height:200px` it works. | position bb97e60 | BROKEN |
| W56 | Column-wrap flex containers: `max-height` is not used as the wrap limit (no wrapping at all with `flex-flow:column wrap;max-height:100px`), and with a definite `height` the second column lands at x=400 (columns spread over the container width) instead of next to the first at x=50. | Q23 4661363 / flex wrap (row only documented) | BROKEN |
| W57 | `caption-side` is treated as non-inherited (style.rs `caption_bottom`): `table{caption-side:bottom}` with a plain `<caption>` still draws the caption on top; only a declaration on the caption itself works. CSS2.1: inherited. | Q34c acbcb00 | BROKEN |
| W58 | Paint order: an inline-block or inline-table is painted in the block layer in tree order, so a LATER block's background paints over it. `<div><span style="display:inline-block;background:green...">` followed by a `margin-top:-40px` red block shows red; inline-level content must paint above in-flow block backgrounds. | inline-block 244a8f4 | BROKEN |
| W59 | A `position:relative` inline that contains a block (block-in-inline split) does not shift the block part by its offset. | position bb97e60 | BROKEN |
<!-- Q40c-table-end -->

#### css-cascade (claimed: Q7 38ca872 CSS-wide keywords, Q18 3a7054b `@import` for fetched sheets)

50 eligible (+10 skipped for JS) | pass 13 | fail 27. (No crashes.)

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W24 `@layer` / `revert-layer` | `revert-layer-001/002/003/005/006/007/009/010/011/012` (10), `layer-stylesheet-sharing`, `-important` (12) | layered rules vanish; the box is unpainted instead of green | NOT CLAIMED |
| W26 `@scope` | `scope-implicit-001..006-print` (6), `scope-pseudo-element`, `scope-ua-shadow-host` (8) | scoped rules dropped, box missing | NOT CLAIMED |
| W25 `all` shorthand | `all-prop-001`, `all-prop-002`, `all-prop-initial-visited` (3) | `all:initial` does not reset (red box stays) | NOT CLAIMED |
| W28 inline `@import` | `import-conditional-001`, `-002` (2) | imported sheets never applied | PARTIAL |
| W27 `revert` | `revert-val-001` (1) | `display:revert` on a div stays inline | PARTIAL |
| animations | `important-prop` (1) | `@keyframes` overriding normal declarations | NOT CLAIMED |

Passing tests include `all-prop-*-color`, `revert-val-002`, `unset-val-*`: they only check `inherit`/`initial`/`unset` on single properties.

#### css-pseudo (claimed: Q19 bb7c64d `::before/::after`, Q19c 76a9d25 `::marker`, Q19e 17346a8 `::first-letter`, Q19e2 0e57f9d `::first-line`)

110 eligible (+163 skipped: 163 `<script>`/testharness, mostly `::selection`/`::highlight`/`::target-text`) | pass 29 | fail 81.
Marker counts among failures: `marker` 36, `first-letter` 22, `first-line` 14, form pseudos (`slider-*`, `file-selector-button`, `placeholder`) + spelling/grammar-error 12.
Hidden behind W29/W34: re-running on a scratch copy with W29 (trailing space only when the source had one) and W34 patched flips 8 of the 81 (`first-letter-skip-*`, `first-letter-with-before-after`, `first-line-with-quote`, `marker-content-006`) and breaks 2 vacuous passes, so W29 is a real cause in this area but not the main one.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W32 `::marker` properties | `marker-*` 36 (`marker-letter-spacing`, `marker-text-align-001..003`, `marker-hyphens`, `marker-content-003/006/010/012/015/016/018/019`) | marker text keeps the list item's font; `display:grid` or `text-transform` on `::marker` ignored | PARTIAL (documented scope) |
| W31 `::first-letter` punctuation | `first-letter-trailing-punctuation`, `first-letter-with-quote`, `first-letter-hi-*` | only the letter (plus leading punctuation) is styled, trailing `.`/`)`/closing quote stay plain | BROKEN |
| `::first-letter` float/abspos/background | `first-letter-002..005`, `first-letter-background-image`, `first-letter-opacity-001` | not root-caused (float + `position:absolute` combos; a plain floated `::first-letter` on Ahem matches its `<span>` equivalent) | BROKEN (not confirmed on a minimal repro) |
| W30 `::first-line` | `first-line-line-height-001/002`, `first-line-with-before-after`, `first-line-opacity-001`, `first-line-below-float`, `first-line-with-out-of-flow*` (14) | text that fits at the base size stays on the enlarged first line; line box height unchanged | BROKEN |
| W29 inline boundary gap | refs of `first-letter-with-span`, `first-letter-skip-empty-span*` | `<span>a</span>bc` is 20px wider than `abc` | BROKEN |
| form/text pseudos | `slider-fill/thumb/track-*` 7, `file-selector-button-*` 2, `placeholder-input-number`, `spelling-error-001`, `grammar-error-001` | not rendered | NOT CLAIMED |
| `::before` as flex container | `before-as-flex-container` | passes on a minimal repro (`display:flex` on `::before`); fails on test details not reproduced | PARTIAL |

#### css-content (claimed: Q19 bb7c64d, Q19b baee567 `attr()`, Q19d c1507db `content:url()`)

55 eligible (+11 skipped for JS) | pass 9 | fail 46. (Before the runner fix for root-relative hrefs 12 passed: 3 passes were vacuous, both sides missing the same stylesheet.)

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W35 default quotes | `quotes-001..031`, `quotes-033/034`, `quotes-match-parent-001` (34) | `<q>` and `open-quote` text without author rules show no “ ” ‘ ’ | BROKEN |
| `content` on elements | `element-replacement*` 9 (`element-replacement.html`, `-root`, `-alt`) | `content: url()`/gradient on a normal element does not replace it | NOT CLAIMED (Q19d is pseudo-elements only) |
| `attr()` details | `attr-case-sensitivity-001`, `attr-chained-pseudo-001` | case-sensitivity rules, `::after::marker` chain | PARTIAL (Q19b) |
| `pseudo-element-inline-box` | 1 | inline box around generated content | not root-caused |

#### css-masking (claimed: Q29 312a345 `mask-image`/`-webkit-mask-image` with size/position/repeat)

287 eligible (+100 skipped for JS) | pass 47 | fail 240. After the runner fix, the 3 `bad-mask-image*` tests (reference `about:blank`) run and fail instead of CRASH (invalid mask URL must hide the element).

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W36 `clip-path`, `clip` | `clip-path/*` 123, `clip/*` 19 (`clip-path-inset-*`, `clip-path-circle-*`, `clip-rule-*`) | shape ignored, full box painted | NOT CLAIMED |
| W19 file/network mask URLs | `mask-image/mask-image-1a..4b`, `mask-position-*`, `mask-repeat-*`, `mask-size-*` (about 70 of 97) | `mask-image:url(m.svg)` + `mask-repeat:no-repeat` on a 100px box paints 100px (mask dropped); the same mask as a `data:` URI paints the 50px tile | PARTIAL (documented; Q35a) |
| `mask-mode`, `mask-composite`, `mask-clip`, `mask-origin`, `mask-opacity` | `mask-mode-*` 6, `mask-composite-*` 8, `mask-clip-*` 6, `mask-origin-*` 2, `mask-opacity-*` 5 | properties ignored | NOT CLAIMED |
| invalid mask URL | `bad-mask-image*`, `mask-image-svg-loading-error` | element must be fully masked, text still painted | NOT CLAIMED |

#### css-text-decor (claimed: text-decoration line/style/color, Q7c 042f538 CSS-wide keywords)

311 eligible (+28 skipped for JS) | pass 82 | fail 229. Hidden behind W29/W34: 8 flipped.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W37 `text-emphasis*` | 125 (`text-emphasis-position-property-*` 40, `-style-property-*` 32, `-line-height-*` 13) | no marks drawn | NOT CLAIMED |
| W37 `text-decoration-inset` | 37 | property ignored | NOT CLAIMED |
| W37 thickness / offset / underline-position | 21 (`text-underline-offset-*`, `text-decoration-thickness-*`) | always default thickness/position | NOT CLAIMED |
| W37 skip-ink / skip-spaces | 16 | | NOT CLAIMED |
| W37 `text-shadow` | 12 (`text-shadow/basic`, `-blur`, `-multiple-noblur`) | no shadow | NOT CLAIMED |
| propagation | `text-decoration-propagation*` 6, `text-decoration-subelements` 3 | `display:contents`/inline-block propagation differs | PARTIAL |
| dotted/ordering | `text-decoration-dotted-*` 2 | | PARTIAL (not root-caused) |

#### css-color (claimed: none explicitly; colour keywords, `#rgb[a]`, `rgb()`)

299 eligible (+12 skipped for JS) | pass 88 | fail 211. Hidden behind W29/W34/Q48a: flipped 0 under the W29/W34 patch.
Failures by name: `hsl`/`t424`/`t425` 42, predefined spaces (`display-p3`, `a98rgb`, `rec2020`, `xyz`...) 43, `lab/lch/oklab/oklch/hwb` 39, `relative-*`/`color-mix`/`light-dark` 23, `deprecated-sameas-*` 23 (system colours), `rgb-*`/`rgba-*`/`hex-*`/`currentcolor-*`/`t32-*` about 37.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W38 colour functions | `hsl-*`, `t424-hsl-*`, `hwb-*`, `lab-*`, `lch-*`, `oklab-*`, `oklch-*`, `*-p3*`, `a98rgb-*`, `rec2020-*` (about 120) | declaration dropped, box unpainted | BROKEN |
| W39 | `deprecated-sameas-*` 23, `relative-currentcolor-*` 19, `color-mix-*` 4 | | NOT CLAIMED |
| rgb/hex/currentcolor/opacity tests | `hex-001..004`, `rgb-001..008`, `rgba-*`, `currentcolor-001/002/004`, `t32-opacity-*` | not a colour bug: the tests stack `<p>` boxes whose adjacent vertical margins do not collapse (the engine has no margin collapsing, Q48a), so the square is 16px lower than the reference | BROKEN (known, Q48a) |

#### mediaqueries (claimed: `@media` min-width/max-width + prefers-* 1282285)

48 eligible (+10 skipped for JS) | pass 20 | fail 28. Viewport is the runner's 800x600.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W40 `not` | `negation-001/002`, `mq-invalid-media-type-001/004` (4) | rules inside `@media not ...` skipped | BROKEN |
| W41 unsupported features / MQ4 syntax | `aspect-ratio-*` 5, `device-aspect-ratio-*` 2, `mq-range-001`, `mq-negative-range-*` 2, `mq-gamut-*` 3, `mq-calc-*` 7, `mq-case-insensitive-001` | query never matches | PARTIAL |
| scripting / svg-as-image | `scripting-print-*` 2, `prefers-color-scheme-svg-as-image` | | NOT CLAIMED |
Many passes are vacuous (`(width > 1000px)` never matches either way); the confirmed matrix is in the W40/W41 repros.

#### css-conditional (claimed: Q10 a4f4d91 `@supports`; `@media` 1282285)

145 eligible (+28 skipped for JS) | pass 88 | fail 57.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W42 `@supports` grammar | `css-supports-019/023/029/030/032/033/034/040/041/042` (10), `at-supports-026/034..039/045/048` (9), `at-supports-at-rule-*` 4, `-selector-*` 6, `-namespace-*` 2, `-content-*` 2, `-font-format/tech` 2 | rules apply (or not) contrary to the spec for malformed conditions | PARTIAL |
| W43 container queries | `container-queries/*` 16 | `@container` blocks ignored | NOT CLAIMED |
| `@media` + `@font-face`/`@import` | `at-media-001/003`, `at-media-content-002/003` (4) | not root-caused (rules inside `@media all` with `@font-face`, `@import` placement) | not confirmed |

#### css-sizing (claimed: Q20f 01a1a77 `width/min-width: min-content|max-content|fit-content`, Q15a 1154431 grid tracks)

569 eligible (+43 skipped for JS) | pass 143 | fail 426. Hidden behind W29/W34: 6 flipped.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W44 `aspect-ratio` | `aspect-ratio/*` 205 (`block-aspect-ratio-*` 37, `flex-aspect-ratio-*` 48, `grid-aspect-ratio-*` 36, `replaced-element-*` 27, `abspos-*` 19) | height stays auto | NOT CLAIMED |
| W44 `stretch`, `fit-content()`, `contain-intrinsic-size` | `stretch/*` 66, `fit-content-length-percentage` 25, `contain-intrinsic-size/*` 24 | ignored/never parsed | NOT CLAIMED |
| `intrinsic-percent-*` | 39 | percentage-sized replaced/non-replaced inside shrink-to-fit parents | PARTIAL (shrink-to-fit, W9/Q43a) |
| W45 min-content | `block-size-with-min-or-max-content`, `*-with-min-or-max-content` 29 (partly), `border-box-and-max-content` | box one em too wide | BROKEN |
| `margin-collapse-with-indefinite-block-size` | 5 | Q48a | BROKEN (known) |

#### html/rendering (claimed: default.css UA sheet, no Q item for presentational hints)

272 eligible (+65 skipped for JS) | pass 145 | fail 127. Hidden behind W29/W34: 1 flipped.
Failures by directory: `the-fieldset-and-legend-elements` 21, `tables` 20, `the-select-element` (replaced + widgets) 15, `button-layout` 8, `embedded-content`/`attributes-for-embedded-content` 8, `lists` 6, `the-hr-element` 4, `phrasing-content` 3, other 22.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| form controls | `fieldset`/`legend` 21, `select` 15, `button-layout` 8, `form-controls` 2, `textarea` 2 | controls drawn as plain text/blocks | NOT CLAIMED (Q30 forms, later) |
| W46 presentational attributes | `tables/table-border-1`, `table-width*`, `table-cell-width*`, `table-bordercolor-001`, `table-align-float`, `the-hr-element-0/*` 4, `lists/ol-type-supported`, `ul-`, `li-type-`, `div-align`, `font-element-text-decoration-color` | attribute ignored | NOT CLAIMED |
| tables | `table-row-height`, `table-row-group-height`, `table-direction*`, `table-row-pagination-*-print` | row heights/direction ignored (W16 family, Q51a) | PARTIAL |
| embedded | `audio-controls`, `canvas-*`, `video-*` | no media elements | NOT CLAIMED |
| `br` | `br-and-wbr-with-non-inline-display`, `br-clear-presentational-hints` | | PARTIAL (Q41a) |

#### css-overflow (claimed: Q21 3a5e687 `overflow-x/-y`, `text-overflow:ellipsis`; Q24 00648b9 scroll containers)

559 eligible (+167 skipped for JS) | pass 98 | fail 461. (The 1 CRASH, `overflow-body-propagation-013`, had an `about:blank` reference; it now runs and fails.)

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W48 line-clamp | `line-clamp/*` 293 (`line-clamp-auto` 52, `block-ellipsis` 40, `webkit-line-clamp` 29) | no clamping | NOT CLAIMED |
| W48 `text-overflow:<string>` and ellipsis details | `text-overflow-string-*` 24, `text-overflow-0xx` 18 (`-004` inherit, `-006` edge of line box, `-012` grapheme clusters) | | NOT CLAIMED / PARTIAL |
| W47 long words wrap | `text-overflow-002/004` (test shows 3 stacked black rows instead of one clipped line) | unbreakable Ahem word is split over lines | BROKEN |
| W48 scroll markers, clip margin, body propagation | `scroll-markers/*` 44, `overflow-clip-margin-*` 21, `overflow-body-propagation-*` 7 | | NOT CLAIMED |
| `overflow:clip` combos | `clip-002..005` | `overflow:clip` + `overflow:visible` per axis | PARTIAL |

#### css-grid (only the Q15 subset: Q15a 1154431 track sizes, Q15b 9397a18 `grid-template`, Q15c a665a34 numeric placement, Q15d 86e1dcb `repeat(auto-fill|auto-fit)`)

Run over `grid-definition`, `placement`, `layout-algorithm`, `implicit-grids`, `grid-model` and the top-level tests (218 files): 190 eligible (+28 skipped for JS) | pass 79 | fail 111.
Not run (not claimed): `grid-lanes` 928, `subgrid` 102, `masonry`, `alignment` 167, `abspos` 153, `grid-items` 190, `parsing`, `animation`.
The claimed features pass on minimal repros: `repeat(2,50px)`, `1fr 2fr`, `minmax()`, `min-content` tracks with Ahem text, `auto-fill` + overflow wrapping, `gap`, `grid-template` strings, `grid-area` names, `grid-column:span 2`, `1/-1`.

| group | tests (examples) | wrong render | triage |
|-------|------------------|--------------|--------|
| W49 named lines | `grid-placement-using-named-grid-lines-*` 8, `grid-support-named-grid-lines-*` 2, `grid-repeat-integer-*` 5 (named lines inside `repeat`) | track list dropped | NOT CLAIMED |
| auto-fit details | `grid-auto-fit-*` 9, `grid-auto-repeat-*` 8, `flex-item-grid-container-auto-repeat` 2 | collapsed tracks, `fit-content()`, calc, aspect ratio | PARTIAL (Q15d covers the basic form) |
| W50 inline-grid / W51 | `grid-model/*-inline-*` 7, `display-inline-grid`, `grid-container-scrollbar*` 5, `grid-first-letter`, `grid-floats-*` | inline-grid block-level; scrollbars/floats | BROKEN / PARTIAL |
| `fr-unit`, `fr-unit-with-percentage`, baseline | `fr-unit*` 2, `grid-container-baseline-synthesized` 4 | | PARTIAL |

#### acid2 (not claimed)

1 test run by hand: `acid/acid2/test.html` against `acid/acid2/px-reference.html` (the committed `reftest.html` needs `<script>` and an iframe, so the runner skips it) | pass 0 | fail 1.
First divergence: the page shows the intro text and a small red bar instead of the face (W52). Not added to `wpt-subset.txt` (the runnable file lives outside the checkout list); revisit after `<object>` and iframe support.

#### Q40b residue ("ungrouped / not root-caused" rows), re-measured 2026-11-15

Re-run over `clear-clearance-calculation-*`, `block-in-inline-*`, `inline-table-zorder-*`, `caption-side-applies-to-*`, `flex-minimum-height-flex-items-*`, `percentage-heights-*`, `intrinsic-size/col-wrap-*`: 99 eligible (+16 skipped for JS) | pass 42 | fail 57 (col-wrap measured separately: 14 fail, 12 JS-skipped).

| group | counts | root cause (confirmed on a minimal repro) | triage |
|-------|--------|-------------------------------------------|--------|
| `clear-clearance-calculation-*` | 5 of 5 fail | W53 (clearance + margin) and the reference's `url()` background (W19); also Q48a | BROKEN |
| `block-in-inline-*` | 25 pass, 5 fail (`-007`, `-relpos-001/002`, `-self-collapsing-only-child`, `-zero-height-margin-collapse`), 3 skipped | the 3 margin cases are Q48a (no margin collapsing); the 2 relpos cases are W59. The Q40b "8 fail" were mostly fixed by Q41/Q49 etc. | BROKEN (Q48a / W59) |
| `inline-table-zorder-*` | 5 of 5 fail (inline-block variant fails too) | W58 | BROKEN |
| `caption-side-applies-to-*` | 5 pass, 11 fail (`-006..-017`, all `display:table*` hosts) | W57: the property sits on the table and is not inherited by the caption. The Q40b guess ("applies to non-captions changes layout") was wrong | BROKEN |
| `flex-minimum-height-flex-items-*` | 9 pass, 16 fail, 6 skipped | W54 | BROKEN |
| `percentage-heights-*` | 3 pass, 15 fail, 7 skipped | W55 (abspos `top/bottom:0`); the rest need `flex-basis:%` of an indefinite parent | BROKEN |
| `col-wrap-*` | 0 pass, 14 fail | W56 | BROKEN |

### QM1k1b findings (Sonnet, 2026-11-19): mpv 0.41 on a growing, unsized fMP4 stream (R34 QM1k plan, "first risk")
Built: `scratchy-mse://<feed>/<gen>/<track>` stream_cb (Condvar-blocking read, EOF only after endOfStream, cancel_fn wakes the reader), `MpvBackend::load_mse`, ffmpeg sync-marker fixtures, a Rust fake driver. Tests `qm1k1b_*` in src/tests/mse_mpv_tests.rs (real-mpv ones skip without libmpv/ffmpeg; they RAN locally: libmpv 0.41, ffmpeg 8.1.2).
- FAILURE 1 (reproduced, FIXED): with a seekable `seek_fn` and no size, mpv logged `stream level seek from 131072 to 2856706` and then sat in `Detected file format` for as long as no more bytes came (20 s until the test stopped it). Cause: libavformat's mov demuxer parses EVERY top-level atom at open (it skips the `mdat` payload with a seek to the end of what is known, then wants the next atom header), so open only returns at endOfStream/more data. With `seek_fn = None` (stream "not seekable") it opened, but then `Cannot seek in this stream` for every `seek` (`lavf: Cached seek not possible`) even with `demuxer-seekable-cache=yes`. FIX that keeps seek_fn: loadfile option `demuxer-lavf-o=fflags=+ignidx` (mov stops after the first moov+mdat). A first read after the init gives only the bytes held; mpv then blocks in read for more, which is fine (demux thread).
- `audio-files`: `change-list audio-files clr ""` then `change-list audio-files append scratchy-mse://F/G/1` works (the URL is one element, not split on ':'); the loadfile line `loadfile <url> replace -1 start=T,rebase-start-time=no,demuxer-max-bytes=16MiB,demuxer-max-back-bytes=8MiB,demuxer-lavf-o=fflags=+ignidx` is accepted. `time-pos` is media time: a fixture whose tfdt starts at 30 s plays 30.0 -> 40 s (no rebase).
- ffmpeg fact: its mov muxer ALWAYS writes tfdt from 0 (`-output_ts_offset`, `-itsoffset`, `-copyts` do nothing), so the fixture tfdt values are shifted in the test.
- The external audio track is opened synchronously by the load: with audio appended 2 s late, `time-pos` started ~2 s after load (first sample at 2.3 s wall). The element must (as planned) issue the load only when every SourceBuffer has data covering the start, not just its init.
- A forward seek to a time whose data is not appended blocks (no time-pos, nothing) until the data arrives: expected, and why QM1k1c must answer `waiting` there.
- Cancel: `unload()` and drop return in well under 1 s with the open blocked and with a playback read blocked (tests).
- Drift table (ao=null, release, machine load avg ~5 at start, 8 cores; run once): see the QM1k1b PR / TODO QM1k-AB. With ao=null `avsync` reads exactly 0.0000 always (the null audio output has no device clock), so it cannot judge sync; `audio-pts - time-pos` sat at -0.040 s (one video frame) in all samples, 0 decoder drops, at most 1 VO drop, reloads 50 ms and 256 ms to the first time-pos, in-generation seeks settle in ~0.5 s. QM1k-AB must use ao=pulse (user present) for a real A/V verdict.
