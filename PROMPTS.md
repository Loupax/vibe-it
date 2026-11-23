
## QA11a part 2: border width default medium, keywords, currentColor (BLOCKED, PR open)

> Land the border default/keyword/currentColor change with zero pass->fail.

**What changed:** css.rs `parse_border_width` (thin/medium/thick), style.rs unset width = 3px when a style is set, explicit `currentColor` and `inherit`/`initial` of an unset border colour resolve to the own colour, paint.rs unset colour paints as `color` (`with_current_border_color`). Tests `border_width_defaults_to_medium_and_keywords_parse`, `border_color_current_color_resolves_to_own_color`; `q53_negative_sizes_are_dropped` expectation corrected (dropped width with a style is medium, 43px not 40). WPT +7 new passes, 2 pass->fail (`table-rules-rows-vertical`, `row-group-margin-border-padding`: accidental passes needing QA11a2), so NOT merged.

## QW5: honour the HTML `hidden` attribute

> The `hidden` attribute is not honoured (social.loupax.com's `<p id="vimpagetags" hidden>` shows). Add the UA rule, JS toggle must relayout, test it, check real pages.

**What changed:** scratchy/src/default.css gets `[hidden]:not([hidden=until-found i]) { display: none }` (UA origin, so an author `display:` on the same element still wins) plus `area, basefont, datalist, noembed, noframes, param, rp, script, style { display: none }`. `el.hidden` already reflected the attribute (js_dom.rs), so `hidden=false` restyles through the normal dirty path. Tests `qw5_hidden_attribute_is_display_none_author_display_wins`, `qw5_js_hidden_toggle_relayouts_and_node_stays_in_dom`. Real pages: social.loupax.com/discover loses the tag list (stays in DOM), en.wikipedia Main_Page now shows its images instead of hidden alt-text stubs; wikipedia.org and HN render byte-identical; --js-report error counts unchanged on all 10 pages.

## QSD3: real ShadowRoot (B1, not rendered)

> Implement QSD3: attachShadow with valid host list and NotSupportedError on a second attach, kind-11 node in a Rust side table host->root, shadowRoot getter for open mode, getRootNode({composed}) host step, adoptedStyleSheets stored per root. No rendering, slots or retargeting.

**What changed:** js_dom.rs: `DomTable.shadow_of/shadow_host` plus natives `__d_shadow_set/of/host`; `ShadowRoot` (extends DocumentFragment: host, mode, delegatesFocus, slotAssignment, clonable, serializable, activeElement null, innerHTML, getElementById, adoptedStyleSheets stored per root and validated); `Element.attachShadow` (valid hosts, TypeError on bad init, NotSupportedError on invalid host/second attach) and `Element.shadowRoot` (open only); `getRootNode({composed})`, `isConnected` and custom element `ceIsConn` walk root -> host; custom element scan/collect descend into shadow trees of hosts under the node (`ceQueryDeep`); one log line per page `not implemented: shadow root rendering (QSD6)`. js.rs `dispatch`: a composed event continues from a shadow root to its host (no retargeting, QSD8). Tests qsd3_*; qy6e test updated. js-report before->after: github 26 -> 1 (the 1 is the new not-implemented log line, plus a timer budget line as before), wikipedia, en.wikipedia Main_Page, HN 0, youtube/old.reddit 1 unchanged; live screenshots of en.wikipedia, github, HN byte-identical to the previous build.

## QY44c: document.styleSheets, style/link .sheet, insertRule reaches the cascade

> Item QY44c (= QSD4a): CSS-in-JS (BBC styled-components, Guardian emotion speedy) needs `document.styleSheets`, `HTMLStyleElement.sheet`/`HTMLLinkElement.sheet` with cssRules/insertRule/deleteRule/ownerNode/disabled, and insertRule on a `<style>` sheet must restyle the page.

**What changed:** cssom_prelude.js: per-element CSSStyleSheet (WeakMap, rules parsed from the element text, ownerNode/href/title/media/disabled), `document.styleSheets` (fresh StyleSheetList of connected style and rel=stylesheet link elements in tree order, constructed/adopted sheets excluded), edits publish the sheet's effective CSS with `__d_sheet_text(id, origText, css)`; unmutated rules keep their raw text so nesting or odd syntax survives. images.rs: `STYLE_OVERRIDES` consulted by `collect_style_css` only while the element text still equals the text the sheet was made from (a later textContent change replaces the sheet and wins); cleared on navigation. Link sheets have empty cssRules and do not reach the cascade (open). Tests: qy44c_* in tests.rs. Measured: BBC headless loses the "client-side exception" and `length of undefined` errors; the next one is the 500 ms CPU budget trip in the timer phase.

## QAN1a: parse @keyframes and the animation longhands (no evaluation)

> Item QAN1a: parse `@keyframes`/`-webkit-keyframes` into a side registry honouring @media/@supports, the eight animation longhands and the shorthand with -webkit- aliases and inherit/initial, timing functions; store per element as an interned list. No evaluation, no behaviour change.

**What changed:** new src/animation.rs (types, parsers, interner, KeyframeBlockParser, registry); css.rs `AtPrelude::Keyframes`, `Declarations::anim`, `parse_stylesheet_with_keyframes`; style.rs `ComputedStyle::animations` (one interned pointer), cascade + `copy_prop` for inherit/initial; images.rs caches the registry next to RULES_CACHE. Tests `qan1a_*`.

## QV8-4: V8 startup snapshot (measure first, drop if < 10 ms)

> Implement QV8-4: measure page-start CPU and isolate+context+prelude setup on the engine-v8 build (3 pages, median of 5), prototype a V8 startup snapshot with the preludes baked in, measure again; drop it if the gain is under 10 ms per page start, otherwise implement it properly (QuickJS build unchanged, no stale runtime files, identical tests).

**What changed:** js_engine_v8.rs: natives carry their arena index (Integer data) instead of an External pointer; `Snapshot` (blob + name -> index map) built in-process on a thread of its own by running scratchy's startup over an empty document (`js::bake_engine`, `js::startup_engine` split out of `start_scripts_clocked`); page engines are created from it (`CreateParams::snapshot_blob`, one external reference: the trampoline) and `register` only re-attaches closures by name; `warm_up` (background, app start), `ensure_snapshot` (tests, report modes), `SCRATCHY_V8_SNAPSHOT=0` off switch, `without_snapshot` test seam. PRELUDE: the page-derived state moved into `__scratchy_apply_env` (+ `__scratchy_env_hooks`, used by history), media_prelude `__media_sweep`; install_dom skips the DOM prelude when prebuilt and keeps `__d_all` plain while baking. QuickJS: no-op stubs. Tests: js_snapshot_tests.rs (snapshot start equals slow start for 3 URLs, page state is the page's, independent engines and Math.random, DOM of the real document); the JS_FLAG_LOCK is now taken by qb8_start, qj2 and qj3 (a latent race with the noscript tests that the slower first engine exposed). Measured: PERF.md "QV8-4".

## QV8-5b: media_mpv.rs clippy warnings, clippy count on the default config

> Implement QV8-5b: fix the 12 clippy warnings in src/media_mpv.rs (11x E0133, 1x missing_transmute_annotations) without changing behaviour, move the CI clippy count job to the default config (V8 + media), re-measure the baseline and set it only if <= 78.

**What changed:** media_mpv.rs: `st_*` stream callbacks wrap their bodies in `unsafe {}`, `cast_sym` (transmute_copy) replaces the unannotated transmute. Also fixed the trivial remaining warnings of the default config (dead_code x2, unused variable, unreachable pattern, needless_borrow, map_clone, unnecessary_map_or, manual_is_multiple_of). ci.yml: clippy step now in the `scratchy` job, removed from `scratchy-quickjs`; clippy-baseline.txt 78 -> 74 (default config measured 94 -> 83 -> 74). Checked: full cargo test (1042 pass), WPT no change, --js-report 0 errors on 3 pages, a `<video src>` mp4 playing through libmpv in nested dwl.

## QM1k1i: SourceBuffer sequence mode, timestampOffset, appendWindow

> Implement QM1k1i: `mode = 'sequence'`, `timestampOffset`, `appendWindowStart/End` with spec semantics at segment granularity; the offset shifts store times and mpv must see shifted times (decide by a test with the real mpv, else restrict to segments mode + log).

**What changed:** mse_store.rs: `SourceBufferStore` keeps mode/offset/append window/group start+end; sequence mode places the first segment of a group at the group start (group end after `set_sequence(true)` or `abort()`, or the `timestampOffset` set), later segments keep the offset; segments wholly outside the append window are dropped (partial ones kept whole, no frame trimming); a non-zero offset is written into the stored bytes by `ByteStreamFormat::shift` (`Fmp4Reader` adds the offset to every `tfdt`; if it cannot, e.g. a negative result, the segment is stored unshifted and counted in `AppendOutcome::unshifted`, JS logs `not implemented`). js_media.rs `__mse_sb_cfg`/`__mse_sb_tso`; mse_prelude.js forwards the setters and reads `timestampOffset` back from Rust (sequence mode moves it); no more `sequence` not-implemented log. Tests `qm1k1i_*` incl. real mpv playing a clip made at 0 s with offset 20 at media time 20..30, avsync 0.000.

## QM1k1n2: re-measure the YouTube first-minute timeline after QY53a+b+c and QM1k1n1 (Opus, docs only)

> Do TODO.txt item QM1k1n2: re-measure the YouTube first-minute timeline now that QY53a+b+c (PR #317) and QM1k1n1 are merged, compare with R42 sections 4-5 and R43 (time to first frame, time to `playing`, forced-layout ms per layout, time to PoToken stop, main-thread starvation), write findings into REGRESSIONS.md and file priority-ranked action items in TODO.txt. Docs only.

**What changed:** REGRESSIONS.md R42 section 6 (before/after table, three live runs + two headless, throwaway probes reverted): click -> `playing` 6.9 s (was 37.8), box 3.1 s after the style write (was 23.4), page painted by ~28 s; at most 18.5 s of video because a second init segment (YouTube's quality switch) reloads mpv into an empty generation and the demuxer never opens; page fetch results reach JS 1-19 s after wire finished, so SABR fetches are aborted; main thread still 65-98 % forced layout, 3 redraws in 16 s while playing. TODO.txt: QM1k1n2 ticked; new QM1k1n4 (P1), QM1k1n5 (P1), QM1k1n6 (P2, Opus design), QM1k1n7 (P3); MSE status line updated.

## QY53e1: deferred, rate-limited IntersectionObserver rechecks (Sonnet)

> Do TODO.txt item QY53e1: a recheck caused only by a geometry-generation change must not force a layout. New native `__d_geom_fresh(vw, vh)` (GeomCache key == current key incl. `css_ver()`), fresh -> evaluate now, stale -> `ioDue = now + max(100 ms, 4 x last forced-layout CPU ms, cap 2000 ms)`, evaluated at the end of a task after which geometry is fresh or when `ioDue` passes (one timer). Scroll, resize, element scroll and `observe()` keep the immediate path. Accept: tests; Wikipedia Rust headless 30 s forced layouts from IO <= 3 and forced_layout.total_ms >= 40 % lower.

**What changed:** js_geom.rs `GeomCache::is_fresh` / `last_cost_ms` (CPU ms of the last forced layout); js_dom.rs natives `__d_geom_fresh`, `__d_geom_cost`; IO prelude splits the signature into a hard part (viewport, scroll, `gScrollGen`: immediate) and the geometry generation (deferred via `__scratchy_ioTick`, one `ioDueT` timer). Tests `qy53e1_*` in js_observers_tests.rs; `qy57_take_records_and_dom_writes_recheck` second write moved from 100 ms to 400 ms (the deadline is now at least 100 ms after the first write).

## QOBS3: forced-layout breakdown in --perf-report (Sonnet)

> Do TODO.txt item QOBS3: add to `forced_layout` in `--perf-report` the phases (css_collect, rule_index, precompute_generated, build_children, measure_document, glyph_walk, rect_walk; summing to total_ms within 5 %), style_resolves, elements, no_change (rect map equal to the previous layout), by_reason {io, script} and by_caller (top 10 page JS callers via the V8 stack, taken only when a layout runs). Zero cost when off; overhead <= 2 % when on.

**What changed:** perf.rs `Phase` timers (`phase()`, free unless a forced layout is being recorded), counters, `forced_detail`, JSON members; images.rs `build_layout_tree_with_resources` phase boundaries; style.rs `resolve_style_for` counter; js_geom.rs `refresh` (rect compare, tree/cache drops inside the phases, `rect_walk` now inside the paused/forced total); `pause_budget_since` returns the ns; js_dom.rs natives `__d_rect`/`__d_textrects` look up the caller, `__d_io` brackets `ioEval`; `js_engine::caller_frame` (V8 stack walk, QuickJS: None). Tests `qobs3_*` in perf_tests.rs. PERF.md glossary.

## QM1k1n6a: pointer-motion coalescing (Sonnet)

> Do TODO.txt item QM1k1n6a: keep only the newest CursorMoved position per event-loop turn; process it once in about_to_wait; flush a pending move first when a button, wheel or key event arrives. Tests, frame-stats gate (R46 recipe).

**What changed:** app.rs `queue_pointer_move` / `flush_pointer_move` / `process_pointer_move` (the old CursorMoved body), `event_flushes_pointer`, fields `pending_pointer_move`, `pointer_moves_processed`; tests in frame_budget_tests.rs.

## QMEM3c: honest memory estimates in --perf-report (Sonnet)

> Do TODO.txt item QMEM3c: rule caches report their real heap (selectors, boxes, Vec capacities), shape_memo counts cosmic-text buffers, peak_rss_bytes from VmHWM (never below rss_bytes), plus explained_bytes and a malloc {in_use, free_kept} block. Tests, PERF.md glossary, YouTube memory JSON before/after.

**What changed:** css.rs `StyleRule::heap_bytes` (selector components + media Vec), style.rs `rules_cache_mem` (index also read from `RULE_DERIVED`), layout.rs `shape_memo_mem` (per-char glyph estimate), perf.rs `peak_rss_bytes` (VmHWM), `malloc_stats` (mallinfo2), `walk_us`, perf_tests.rs `qmem3c_*`, PERF.md glossary.

## QM1k1n5: page network results reach JS late (Sonnet)

> Do TODO.txt item QM1k1n5: measure first where AppEvent::JsNet waits (60-90 s YouTube watch, nested dwl, own wire, QOBS6 `net` field); fix if it reproduces, else write up with numbers.

**What changed:** did not reproduce on 1b83534: 124 results, median ~41 ms, max 494 ms (startup). No code change; REGRESSIONS.md section 6a addendum; test `qm1k1n5_response_arriving_during_a_busy_timer_chain_is_delivered_between_slices` in fetch_tests.rs.

## QY57r: IntersectionObserver must not clip by a propagated body overflow (Sonnet)

> Do TODO.txt item QY57r: ioEval0 clips by every overflow ancestor, so YouTube's `body{overflow:scroll}` (956x0) gives every entry an empty intersectionRect and thumbnails never get a `src`. Reuse the layout rule (body overflow propagates to the viewport when html is visible) and treat html as the viewport, not a clip.

**What changed:** js_dom.rs `ioEval0` skips `document.body` as a clip when html's computed overflow is visible (same rule as layout.rs `VIEWPORT_ROOT_NODE`); html was already skipped. Test `qy57r_body_overflow_propagates_to_viewport_and_does_not_clip` in js_observers_tests.rs. YouTube results (nested dwl, own wire): real thumbnails paint; 2 core ytimg imgs got a `src` (the 2 on screen, others lazy), 88 imgs total, 4 with a src.


## QY54b: why inline-preview getPlayerSize forced layouts cost ~58 ms on YouTube results (Opus, measure only)

> Do TODO item QY54b: on YouTube results, hovering a result starts the inline-preview player; one promise job runs 12-15 `getPlayerSize` forced layouts of ~58 ms each in the window (vs ~5 ms headless on watch). Find out why each one costs ~58 ms: which forced-layout path, what invalidates between them, page size at that point, whether QYTC1a/QYTC1d-style in-place shifts or skips should apply and why they don't. --perf-report / SCRATCHY_FRAME_STATS, nested dwl + tools/inject hover, >= 3 runs. Docs only: a new REGRESSIONS.md R-section with method, numbers, cause and sized fix items.

**What changed:** docs only. REGRESSIONS.md R58 (12 window hover runs + 4 headless runs under the perf flock, throwaway logging never committed). Each read is a full-document forced layout (~4 900-6 500 elements; style ~80 %), invalidated by the player building its own DOM (45 % structural, 24 % size writes, 14 % first `style`); the ~5 ms on watch is the small document at read time, same cost per element. New: script-inserted `<link rel=stylesheet>` (YouTube's `www-player.css`) is never fetched, so the preview player has no CSS (wrong size and slot); a counterfactual probe shows that fixing it does not reduce the reads. Proposed QY54b1 (dynamic stylesheet links), QY54b2 (precompute_generated pre-pass), QY54b3 (subtree forced layout, absorbs QY53o2), QY54b4 (first `style` attribute in place).

QYSCR1a: YouTube results: the "Alle" chip has focus after load, so Space presses it (Sonnet)

> Do TODO item QYSCR1a: on YouTube results (/results?search_query=rust) the "Alle" filter chip (button.ytChipShapeButtonReset) has focus after load without any click, so Space presses it instead of scrolling. Find who focuses it (log focus() calls from page JS), check scratchy's own focus rules; if scratchy is the cause fix + tests; if YouTube calls focus() as in Chrome, close as docs-only. Real check in a nested dwl with grim, full suite, clippy, WPT.

**What changed:** scratchy never focuses on its own here: the only focus() is YouTube's kevlar `loadPage_` -> `Ph(parentComponent)`, which focuses the first DFS element that is focusable and has `offsetParent != null`. Chrome picks the masthead guide button (a fixed-position subtree); scratchy's `offsetParent` returned null for every descendant of a `position:fixed` box (it tested the "fixed" flag of the rect, not the element's own position), skipped the masthead and landed on the first chip. Fix in js_dom.rs `gOffParent`: null only for a fixed box itself. Test `qyscr1a_offset_parent_of_fixed_descendants`. REGRESSIONS.md R61 note. After the fix Space presses the guide button, as headless Chrome 156 does (verified by CDP).

QINC1i6: shape memo on for the main thread's engine relayouts (Opus)

> Do TODO item QINC1i6: with SCRATCHY_INCR=1 the main thread's shape memo stays off until the page's first script forced layout, so Main-frame relayouts on pages with no forced layouts re-shape every paragraph they re-measure. Turn it on for them, bounded by `memo_trim`.

**What changed:** `incr_layout::live_update` sets `SHAPE_MEMO_ENABLED` (the existing `memo_trim` bound after the relayout is unchanged). Flag off: `live_update` is never reached. Test `qinc1i6_main_frames_use_the_shape_memo_without_forced_layouts` (memo off before the first Main frame, on after, a memo hit on the 3rd frame; fails without the change). REGRESSIONS.md R60 entry with the gate.

QINC1k: incremental engine exactness sweep and gates (Opus)

> Do TODO item QINC1k (measure only + docs + durable recipe scripts): SCRATCHY_INCR_CHECK=1 headless and window on YouTube results / watch, Wikipedia, github.com, bbc.com/news, social.loupax.com/discover, 3 runs each, 0 mismatches; then SCRATCHY_INCR=1 vs flag off (same binary, 3 interleaved runs under flock): forced layout ms, App builds ms, lat_p95, getPlayerSize median, worst preview job, hover latency, RSS, refusal mix and floors; handle the bimodal regimes; commit re-runnable recipes under tools/; screenshots flag on/off; go/no-go for QINC1l in R60 (do not flip).

**What changed:** no product code. New `tools/incr-gate/` (run.sh one run, gate.sh check|perf|headless series, summarize.py tables, shotdiff.py on/off screenshot compare, hover.html script-free hover page, abs-drift.html minimised mismatch) + tools/README section. REGRESSIONS.md R60 "QINC1k DONE": 126 runs, 0 panics; check mode 0 mismatches except github.com in the window (sub-pixel `style.height` 642.0005 vs 642.0, y translation invariance, minimised); YouTube gates met (getPlayerSize 63-65 -> 4.1-5.7 ms, preview job 830-847 -> 159-181 ms, watch builds -86 %), Wikipedia builds -62 %, github.com App builds +54 % (box-update floor). Verdict NO-GO until proposed QINC1k1 (y invariance / comparator) and QINC1k2 (github box floor); also proposed QINC1k3, QHOV2; QINC1n not recommended.

QERR1: uncaught top-level script exceptions fire an ErrorEvent at window (Sonnet)

> Do TODO item QERR1: an uncaught exception in a top-level page script fires an ErrorEvent at window (message, filename, lineno, colno, error) so onerror and testharness see it (R64 E7), then the log line.

**What changed:** js.rs eval_as/eval_module call the prelude __scratchy_report_error (js_engine_v8.rs expose_pending + throw position); window.onerror gets the 5-argument form, true or preventDefault suppresses the log line; reentrancy guarded. Timers/rAF/observers (QEL4) not covered. Verified: --wpt-th-one on css/cssom/computed-style-003/-004 now gives harness ERROR; WPT subset no change (333/293); cargo test 1811 pass; clippy 77.

QKEYS1c: status-bar message full width, URL elided in the middle (Sonnet)

> Do TODO item QKEYS1c: status-bar messages were clipped to a third of the bar; the message gets the width it needs, the URL gets the rest and is elided in the middle.

**What changed:** paint.rs: pure `status_layout` (+ `elide_end`/`elide_middle`) decides the drawn mode/URL/message strings from a measure closure; `paint_status_bar` uses it. Message capped at bar - mode - min URL slice (160 px), URL middle-elided. Unit tests in `status_layout_tests`. Real check in a nested dwl: URL `file:///.../worktr...status/bar/test/page.html` plus the full message `scratchy: unknown command: :zzzz` (yy's "Copied to clipboard" lands with QREG1 #531).

QDOMATOM: atomic insertion post-connection steps (Sonnet)

> Do TODO item QDOMATOM: when one insertion adds several nodes (a DocumentFragment, a div moved in), the post-connection steps (script prepare/run, iframe setup) run in tree order after the whole insertion; scripts see all inserted siblings and an iframe has no contentWindow until its turn.

**What changed:** js_dom.rs `afterIns(c, kids, par)` (called by appendChild/insertBefore with the fragment's nodes captured before they leave it) collects the inserted scripts and iframes (shadow-including, tree order), marks the iframes pending (`pendIfr`, `frameOf` returns null), then sets up each iframe or prepares each script in order; only the parent script (text appended into it) is prepared afterwards instead of a global sweep, so a nested insertion runs just that script. Tests: src/tests/qdomatom_tests.rs. WPT dom/nodes: Node-appendChild-script-and-iframe 2/4 -> 4/4, three-scripts(-from-fragment) 0/1 -> 1/1. append() with several arguments stays non-atomic (QDOM16 #549).

QDOM19: current DOM name validation (Sonnet)

> Do TODO item QDOM19: whatwg/dom 2025 "valid element local name", "valid attribute local name", "valid doctype name" and `validate and extract` for createElement(NS), createAttribute(NS), setAttribute(NS), toggleAttribute, createDocumentType, createDocument.

**What changed:** js_dom.rs: `reElemName`/`reAttrName`/`reDoctypeName`/`rePrefix` replace the XML Name approximation for these APIs (`validName` stays for processing-instruction targets). `extractName(fn, owner, ns, q, isEl)` splits at the FIRST ':' (prefix = non-empty, no whitespace/NUL/'/'/'>'; local = valid element or attribute local name), then the NamespaceError rules; `createElementNS` and `createDocument` use it, so `a:b:c` is valid. `toggleAttribute` validates before looking at the attribute. Attribute local names reject `=`; doctype names may be empty. The pinned WPT (ac78b567) follows the current spec everywhere checked, no disagreement found. tests: src/tests/qdom19_tests.rs. WPT: DOMImplementation-createDocument 388->430/434, Document-createAttribute 20->33/36, createElementNS 177->203, attributes 45->48, name-validation.html 5/5 (not in the subset until QWPT6c #579 lands).

QAPI6: `:has()` in stylesheets and querySelector, with anchor invalidation in the engine (Opus)

> Do TODO item QAPI6: `:has()` with relative selector lists (descendant, `>`, `+`, `~` anchors), in stylesheets and in querySelector/matches/closest, plus correct invalidation: a DOM change inside the subject (or its siblings for `+`/`~`) re-matches the anchor elements, in both the incremental engine and the from-scratch path; a page without `:has` must pay nothing (measure). Tests in a new src/tests/qapi6_tests.rs.

**What changed:** dom.rs `parse_has` (matching is the selectors crate's; per-call caches). invalidation.rs: a feature anywhere in a relative selector reaches the anchors in one hop (`Hop::Anc` / `Hop::AncSib`), anchors filtered by the `:has` compound, then the outer hops; `Key::HasTree` for inserts/removals pruned by a subtree summary; `Filter::attrs` (attribute names, checked only on reached elements); batch walk memo per engine update; nested combinators inside `:has` stay global. incr_log keeps weak node handles while a `:has` map is current, so `Engine::unentried` replays changes under `display: none` (previously skipped). 16 tests incl. a `:has` soundness fuzz on the QINC1d generator. WPT: reftest has-style-sharing-001 newly passes; wpt-th dom/nodes +3 files; css/selectors :has testharness files (not in the subset) 1366 -> 2762 / 2897 subtests. Cost: no-`:has` pages unchanged (hover fixture, Wikipedia); github light runs unchanged, heavy runs restyle +8 %; YouTube results forced layouts ~+65 % (35 `:has` rules now apply).

QAPI6b: the cost of `:has()` on pages that use it (Opus)

> Do QAPI6b (coordinator, stacked on #573): profile the `:has` cost on YouTube results (+65 % forced layouts with #573), share one SelectorCaches per build, and anything else the profile shows; target YouTube results and github heavy within noise of main, no correctness loss.

**What changed:** temporary per-phase timers (removed) showed the engine's invalidation, not matching, was the cost: one update replayed 1 329 logged inserts into hidden lists (236 ms). `Engine::unentried` now answers once per unstyled subtree (`InvalidationMap::for_has_any`, `Key::HasAny`: every `:has` path, feature-agnostic) and skips subtrees inserted in the same batch; `style::SEL_CACHES` shares one `SelectorCaches` per build/update. Found on the way: removal pruning by the removed subtree's summary was unsound (a detached subtree changes without a log entry), now unpruned. On github.com the window runs showed whole-page restyles: temporary mark attribution (removed) traced them to `:has` inside `:not()` / `:is()` losing its anchor filter; the enclosing compound's filter now anchors it. Tests in `qapi6b_tests.rs`.

QIFRNAV + QDOM5d: unload events on iframe navigation; url-fragment load dependence (Sonnet)

> Do TODO items QDOM5d and QIFRNAV (one PR): fire beforeunload/pagehide/visibilitychange/unload on iframe navigation in spec order, find why url-fragment.window.html is load-dependent, and make an in-place iframe navigation create a new Document.

**What changed:** js_dom.rs: setting `src`/`srcdoc` of an iframe whose window finished loading queues two tasks (the unload task, then the navigation task, so a handler killed by the callback budget does not stop the navigation; hr-time/navigation-start-post-before-unload busy-waits 3 s in beforeunload). The unload task runs `__scratchy_unload_phase` in the old realm: beforeunload (BeforeUnloadEvent), then pagehide (persisted false), visibilitychange (visibilityState hidden) and unload, nested windows after their parent, with the per-document ignore-opens-during-unload counter raised (document.open() returns without effect). document.open() on an XML document throws InvalidStateError and erases the Window's and the Document's listeners (`__scratchy_erase_listeners`, js.rs): url-fragment.window.html was a stack overflow, a window `load` handler calling document.open()/close() re-ran itself forever, and where the stack ran out depended on load. Left out: the new-Document-in-the-same-Window swap (the realm's `doc` is a closure variable used everywhere), unload on iframe removal (must not run script synchronously), promise-job-incumbent (needs window.open).

QGEOM1: Geometry Interfaces Level 1, DOMMatrix for canvas (Sonnet)

> Do QGEOM1: DOMPoint/DOMPointReadOnly, DOMRect/DOMRectReadOnly, DOMQuad, DOMMatrix/DOMMatrixReadOnly (sequence and CSS transform string constructors, fromMatrix/fromFloat32Array/fromFloat64Array, multiply/translate/scale/rotate/skew/invert and Self variants, toString/toJSON), canvas getTransform returning a DOMMatrix and setTransform taking a DOMMatrix2DInit; add css/geometry as a wpt-th area.

**What changed:** new `src/geometry_prelude.js` (prepended to DOM_PRELUDE, so one copy per realm) with all the interfaces; the DOM prelude takes DOMRect from it through the hidden `__scratchy_geom` (getBoundingClientRect/getClientRects/IntersectionObserver rects are real DOMRect instances). `tools/wpt-subset.py` has the area `css/geometry` (28 files, shard `rest`); tests in `src/tests/qgeom1_tests.rs`.
