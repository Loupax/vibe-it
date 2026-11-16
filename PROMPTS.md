
## QA11a part 2: border width default medium, keywords, currentColor (BLOCKED, PR open)

> Land the border default/keyword/currentColor change with zero pass->fail.

**What changed:** css.rs `parse_border_width` (thin/medium/thick), style.rs unset width = 3px when a style is set, explicit `currentColor` and `inherit`/`initial` of an unset border colour resolve to the own colour, paint.rs unset colour paints as `color` (`with_current_border_color`). Tests `border_width_defaults_to_medium_and_keywords_parse`, `border_color_current_color_resolves_to_own_color`; `q53_negative_sizes_are_dropped` expectation corrected (dropped width with a style is medium, 43px not 40). WPT +7 new passes, 2 pass->fail (`table-rules-rows-vertical`, `row-group-margin-border-padding`: accidental passes needing QA11a2), so NOT merged.

## QW5: honour the HTML `hidden` attribute

> The `hidden` attribute is not honoured (social.loupax.com's `<p id="vimpagetags" hidden>` shows). Add the UA rule, JS toggle must relayout, test it, check real pages.

**What changed:** scratchy/src/default.css gets `[hidden]:not([hidden=until-found i]) { display: none }` (UA origin, so an author `display:` on the same element still wins) plus `area, basefont, datalist, noembed, noframes, param, rp, script, style { display: none }`. `el.hidden` already reflected the attribute (js_dom.rs), so `hidden=false` restyles through the normal dirty path. Tests `qw5_hidden_attribute_is_display_none_author_display_wins`, `qw5_js_hidden_toggle_relayouts_and_node_stays_in_dom`. Real pages: social.loupax.com/discover loses the tag list (stays in DOM), en.wikipedia Main_Page now shows its images instead of hidden alt-text stubs; wikipedia.org and HN render byte-identical; --js-report error counts unchanged on all 10 pages.

## QSD3: real ShadowRoot (B1, not rendered)

> Implement QSD3: attachShadow with valid host list and NotSupportedError on a second attach, kind-11 node in a Rust side table host->root, shadowRoot getter for open mode, getRootNode({composed}) host step, adoptedStyleSheets stored per root. No rendering, slots or retargeting.

**What changed:** js_dom.rs: `DomTable.shadow_of/shadow_host` plus natives `__d_shadow_set/of/host`; `ShadowRoot` (extends DocumentFragment: host, mode, delegatesFocus, slotAssignment, clonable, serializable, activeElement null, innerHTML, getElementById, adoptedStyleSheets stored per root and validated); `Element.attachShadow` (valid hosts, TypeError on bad init, NotSupportedError on invalid host/second attach) and `Element.shadowRoot` (open only); `getRootNode({composed})`, `isConnected` and custom element `ceIsConn` walk root -> host; custom element scan/collect descend into shadow trees of hosts under the node (`ceQueryDeep`); one log line per page `not implemented: shadow root rendering (QSD6)`. js.rs `dispatch`: a composed event continues from a shadow root to its host (no retargeting, QSD8). Tests qsd3_*; qy6e test updated. js-report before->after: github 26 -> 1 (the 1 is the new not-implemented log line, plus a timer budget line as before), wikipedia, en.wikipedia Main_Page, HN 0, youtube/old.reddit 1 unchanged; live screenshots of en.wikipedia, github, HN byte-identical to the previous build.
