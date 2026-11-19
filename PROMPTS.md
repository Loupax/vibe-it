
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
