
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
