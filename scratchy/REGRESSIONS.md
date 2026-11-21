# scratchy — regressions

## R65: QAREA-DOM: the dom/* testharness failures of the QWPT6 baseline, grouped by root cause. Nineteen causes explain most of the gap; each is one missing method family or one wrong rule, and each becomes one fix item (QDOM1-QDOM19, 16 S + 3 M). Three things weigh more than any single DOM gap: 94 of the 194 dom/events entries load testdriver.js (score 1.4 %, wait for QWPT7); 16 of the 41 dom/ranges files are the tentative OpaqueRange API; and the core Range/traversal suites are not in the subset at all (timeout=long), and today they could not run anyway because dom/common.js needs `createCDATASection`. If every proposed item lands, the projected scores are about: nodes 42.4 -> ~65-70, events 23.0 -> ~37-41, ranges 17.1 -> ~33-38, other 55.2 -> ~80-85 (Opus, 2026-11-21, investigation, docs only) — OPEN: proposed QDOM1-QDOM19 (section 4), QWPT6b/QWPT6c (section 6)

### 1. How this was measured

- Binary: `~/.cargo/bin/scratchy` and `~/.cargo/bin/wire` as installed from main (237a0bf, installed 17:09), copied
  into the worktree. WPT: a private sparse checkout of the pin made with `tools/wpt-checkout.sh` (the shared
  /home/loupax/src/wpt has no `dom/`). I started my own wire on my own socket
  (`--no-cookies --mount http://web-platform.test:8000=CHECKOUT --mount-override /resources/testharnessreport.js=tools/wpt/testharnessreport.js`)
  and ran one `scratchy --wpt-th-one` per dom/* URL of `wpt-th/subset.txt`, 4 at a time, inside the 6 GB scope. I kept
  each URL's JSON, because the expected files hold only subtest names, not the failure messages.
  703 URLs, a few minutes of wall time at load average 18.
- The run reproduces the committed baseline exactly: nodes 380 ran / 42.4, events 194 / 23.0, ranges 41 / 17.1,
  observable 29 / 0.0, other 46 / 55.2 (same subtest counts as `wpt-th/areas.txt`).
- Classification: every failing subtest (or the harness message, for an ERROR with no subtests) goes to the FIRST
  cause whose rule matches. Rules are regexes on the message, plus the file path for families such as moveBefore,
  `.xhtml`, collections and testdriver users. Window named access is detected as "`X is not defined`, where the
  test's source has `id=X` or `name=X`".
- Projection: "upper bound" = the area score when every subtest assigned to the cause passes, all other subtests stay
  as they are. A harness-ERROR file the item unblocks is counted at 75 %. Targets in the items are set at about
  60-80 % of the bound, because a subtest unblocked by one fix often hits the next gap.
- A probe page (one testharness file, not committed) confirmed each cause directly (section 2.2).

### 2. Where the scores come from

#### 2.1 The big blocks (not DOM core bugs)

| block | entries | score now | what it is | belongs to |
|---|---|---|---|---|
| dom/events testdriver users | 94 of 194 | 1.4 % | `<script src=/resources/testdriver.js>` in HTML: 40 `non-cancelable-when-passive/*` (TIMEOUT 0/0), `scrolling/*`, `handler-count`, click/focus tests. `action_sequence() is not implemented` | QWPT7 |
| dom/events without testdriver | 100 | 43.3 % | real event-model gaps (QDOM4, QDOM10, QDOM13, QDOM15) | here |
| dom/nodes testdriver users | 5 | 0 % | moveBefore hover/pointer, fullscreen | QWPT7 |
| dom/ranges/tentative OpaqueRange | 16 of 41 | 0 % | `createValueRange()` on input/textarea: a WICG proposal not shipped anywhere | not now |
| dom/ranges core suite | 0 in subset | - | 31 Range-*/traversal files are `timeout=long`, so excluded. They also all ERROR today: `dom/common.js` `setupRangeTests` calls `new Document().createCDATASection()` | QDOM6+QDOM8, then QWPT6c |
| dom/observable (tentative) | 29 | 0 % | `Observable is not defined` (234 of 251), `EventTarget.when` | not now (section 5) |
| dom/nodes/moveBefore | 62 | ~0 % | `moveBefore is not a function` (54), plus window named access (`container is not defined`: tests use element ids as globals) | QDOM1 + QDOM3 |
| XML-served dom/nodes files | 46 | ~0 % | 13 `.svg` CRASH (not counted in "ran"), 18 `.xhtml/.xml` ERROR (`Unexpected token '<'`: XHTML parsed as HTML, `<![CDATA[` inside `<script>`), plus `.xml`/`.xhtml` iframes with `contentDocument.documentElement` null (Document-createElement(NS): 488 subtests) | QDOM11 |

The subset generator meant to leave out testdriver users, but `tools/wpt-subset.py` only checks
`// META: script=` metadata (`.any.js`/`.window.js`), not HTML `<script src>`. So 102 dom entries that need trusted
input are in the phase-1 baseline. Either outcome is fine; the coordinator should choose (QWPT6b, section 6).

#### 2.2 Probe (confirms the causes on the current main binary)

`typeof probeDiv` for `<div id=probeDiv>` -> "undefined"; `typeof frames[0]` with one iframe -> "undefined";
`new Text('x')`, `new Comment('x')`, `new DocumentFragment()`, `new Document()` -> TypeError "Error converting from js
'undefined' into type 'i32'" (the constructors in js.rs `ctor(...)` make an empty object with no `__id`);
`c=createComment('a'); c.data='b'` -> "a" (also `appendData`); `document.nodeValue`/`document.textContent` -> the whole
text instead of null; `document.charset`/`inputEncoding` -> undefined; `document.body.childNodes` -> "[object Array]",
`children.item` undefined; `typeof XMLDocument` undefined; `createDocument(null,null,null).createElement('DIV').tagName`
-> "DIV" (lowercased, then upper-cased as HTML); `createTextNode('x').before` undefined; `document.doctype` undefined
on the page document; `Text.prototype.splitText` undefined; `createProcessingInstruction`, `createCDATASection`,
`getSelection` undefined; `dispatchEvent(createEvent('Event'))` (uninitialized) -> true instead of InvalidStateError;
`window.event` during dispatch undefined; `document.body.onload = f` does not set `window.onload`; `className='a a b';
classList.add('c')` -> "a a b c" (no dedupe). Already fine: `Attr` objects are Nodes (nodeType 2), `baseURI`.

### 3. Root causes, ranked by score per unit of work

Files/subtests: the subtests assigned to the cause (first-match), per area. "Bound": area score gain if they all
passed (section 1). Size S = 1, M = 2 for the ranking. Prerequisites come before what they unlock.

| # | cause (evidence) | nodes | events | ranges | other | bound | size | item |
|---|---|---|---|---|---|---|---|---|
| 1 | No window named access: element ids/names are not globals, `frames[i]`/`window[i]` missing (`container is not defined`, `frames[0].document`) | 48 st / 26 f | 13 / 6 | 1 / 1 | 7 / 3 | +5.2 / +3.1 / +2.4 / +6.5 | S | QDOM1 |
| 2 | Collections are plain arrays: `childNodes`, `children`, `getElementsBy*` have no `item`/`namedItem`, are not NodeList/HTMLCollection, not live | 34 / 23 | - | - | 49 / 10 | +5.1 / +20.7 other | M | QDOM2 |
| 3 | `moveBefore` missing (state-preserving move) | 94 / 41 | - | - | - | +9.5 (needs QDOM1) | S | QDOM3 |
| 4 | Event flags: stop/cancel/initialized flags not reset or honoured, `returnValue`, `cancelBubble`, `initEvent` during dispatch, `composedPath()` after dispatch, listener exceptions not reported, `handleEvent` looked up once, `isTrusted` not an own property, `srcElement`, passive default for touch/wheel on window/document/html/body | - | 88 / 26 | - | - | +9.3 | S | QDOM4 |
| 5 | Range has no content operations (`cloneContents`, `extractContents`, `deleteContents`, `insertNode`, `surroundContents`; the rest of archived QA19) and rejects Attr boundary points | - | - | 42 / 9 | - | +16.3 | M | QDOM5 |
| 6 | Node constructors do not create nodes (`new Text/Comment/DocumentFragment/Document`) | 42 / 12 | 2 / 2 | - | - | +1.8 / +0.2, unlocks dom/common.js | S | QDOM6 |
| 7 | Comment data is immutable (rcdom `Comment.contents` is not a cell; `text_write_changes` returns false) | 49 / 8 | - | - | - | +0.8 | S | QDOM7 |
| 8 | No ProcessingInstruction / CDATASection nodes; `XMLDocument` missing; Node constants incomplete | 112 / 16 + 5 ERROR | 6 / 1 | 0 / 1 ERROR | 3 ERROR (traversal) | +2.6 / +0.5 / +1.8, unlocks 31 long files | S | QDOM8 |
| 9 | DOMTokenList is not an ordered set (no dedupe, `supports`, `value`, iteration) | 140 / 1 | - | - | 20 / 4 | +0.1 / +4.2 other | S | QDOM9 |
| 10 | `window.event` never set | - | 23 / 7 | - | - | +3.6 | S | QDOM10 |
| 11 | XML documents: top-level `.xhtml/.xml/.svg` responses and XML iframes are parsed as HTML | 566 / 15 + 18 ERROR + 13 CRASH | - | - | - | +7.1 | M | QDOM11 |
| 12 | `Text.splitText`, `wholeText`, `Node.normalize` missing | 14 / 5 | - | 4 / 1 | - | +1.0 / +2.4 ranges | S | QDOM12 |
| 13 | Event handler IDL attributes are plain expandos (no `null` default, no body/frameset -> window forwarding, no `onanimation*`/`ontransition*`, no webkit-prefixed legacy types) | - | 52 / 4 | - | - | +2.0 (+ unblocks the 40 passive files once QWPT7 lands) | S | QDOM13 |
| 14 | Namespace lookups missing (`lookupNamespaceURI`, `lookupPrefix`, `isDefaultNamespace`), `getElementsByTagNameNS` missing, `getElementsByTagName` goes through the selector parser (`'te:st' is not a valid selector`), `prefix` null | 169 / 7 | - | - | 2 / 1 | +1.3 / +0.6 | S | QDOM14 |
| 15 | Event interface gaps: subclass constructors ignore dictionaries (`view`, `detail`...), `Event.NONE` etc. constants, `createEvent` interface list, dispatching an uninitialized or in-flight event does not throw | - | 67 / 5 | - | - | +1.6 | S | QDOM15 |
| 16 | ChildNode methods only on Element (`before/after/replaceWith/remove` missing on CharacterData and DocumentType); `document.doctype` undefined on the page document; `replaceChildren` missing on Document | 95 / 10 | - | - | - | +1.2 | S | QDOM16 |
| 17 | Document flavours: every document is the HTML one (`Object.create(doc)`), so `createElement` lowercases in XML documents and `tagName` upper-cases; `nodeValue`/`textContent` of Document/DocumentType are not null; `charset`/`inputEncoding` missing | 364 / 12 | - | - | - | +1.2 | S | QDOM17 |
| 18 | Pre-insert validity: HierarchyRequestError rules for Document parents, fragments with 2 elements, doctype placement, text under Document | 58 / 8 | - | - | - | +0.8 | S | QDOM18 |
| 19 | Name validation is the old XML Name production (the DOM spec relaxed it in 2025: `createElement`, `createAttribute`, `createDocumentType`, `createElementNS`) | 129 / 6 | - | - | - | +0.3 | S | QDOM19 |

Cumulative upper bound in this order (nodes / events / ranges / other): after QDOM1 47.6 / 26.1 / 19.5 / 61.7;
QDOM2 52.6 / - / - / 82.4; QDOM3 58.0; QDOM4 events 34.9; QDOM5 ranges 35.9; QDOM6-8 62.9 / 35.2 / 37.7; QDOM9 other
86.5; QDOM10 events 37.7; QDOM11 69.5; QDOM12 70.5 / - / 40.1; QDOM13 events 39.8; QDOM14 71.4; QDOM15 events 41.4;
QDOM16-19 74.4. Realistic end state: about nodes 65-70, events 37-41 (until QWPT7), ranges 33-38 (OpaqueRange stays at
0), other 80-85. What stays unclassified after these items: about 9 points in nodes and events, scattered single files
(section 5 lists the larger ones).

Ranking notes. QDOM1 is cheap, and it also helps every other area: html/*, shadow-dom and uievents tests use element
ids as globals too; only dom was measured here. QDOM6 and QDOM8 are worth more than their subset numbers show: together they
unblock `dom/common.js`, which the 31 timeout=long Range-*/NodeIterator/TreeWalker files and 3 in-subset files
(Node-compareDocumentPosition, Node-contains, Node-properties) build on. The XML item QDOM11 is M because scripts must run in
an XML document (top-level, through `wpt_th`/`run_page`), not only parse (DOMParser already uses roxmltree via
`__d_load_xml`/`xml_to_dom`).

### 4. Proposed items (verbatim for TODO.txt)

```
- [ ] QDOM1 [S] (P1; Sonnet; js.rs, js_dom.rs; R65; after QWPT6) Window named access and indexed child windows. A
      WindowProperties object between `Window.prototype` and `EventTarget.prototype` (a Proxy in the prototype chain set
      up next to `Object.setPrototypeOf(g, Window.prototype)` in js.rs; own globals and `var`s still win). get/has: an
      element with that id, or an embed/form/img/object with that name (new native `__d_named(doc, name)` next to
      `__d_byid`, one walk, cached by the DOM write generation `wgen` because `typeof X` feature probes hit it), an iframe
      with that name -> its contentWindow. `window[i]` / `frames[i]` / `window.length` = child browsing contexts in tree
      order (the iframe realm code in js_dom.rs). Not enumerable. Tests in js_dom_tests.rs (id global, var shadows it,
      removed element disappears, frames[0].document). WPT: abort/abort-signal-timeout, abort/reason-constructor,
      nodes/Node-removeChild, Node-appendChild, querySelector-empty-id, querySelector-id-nth-child,
      remove-and-adopt-thcrash, remove-next-sibling-during-replace-with, events/mouse-event-retarget,
      EventListener-handleEvent-cross-realm, traversal/TreeWalker-acceptNode-filter-cross-realm,
      ranges/Range-extractContents-dynamic-end. Target: nodes 42.4 -> 45, events 23.0 -> 25, other 55.2 -> 60;
      re-run all areas (html/*, shadow-dom, uievents use id globals too) and record their new passes.
- [ ] QDOM2 [M] (P1; Sonnet; js_dom.rs, js.rs; R65; after QDOM1) Real NodeList and HTMLCollection objects.
      `childNodes`, `children`, `getElementsByTagName(NS)`, `getElementsByClassName`, `document.forms/images/links/
      anchors/scripts/embeds` (`coll`), `querySelectorAll` (static NodeList): instances of `NodeList` / `HTMLCollection`
      (prototype with `item`, `namedItem` (HTMLCollection), `length`, `forEach`/`entries`/`keys`/`values`/iterator
      (NodeList only), supported indices/names as own properties, `[LegacyUnenumerableNamedProperties]`, delete/define
      rules). Live: a Proxy that re-queries the native list when the DOM write generation `wgen` changed since the last
      access (one query per generation, not per index). Prelude callers that use Array methods on these (`t.children.forEach`
      in the table code, `kidsOf`, `wrapAll` results) switch to `Array.from` or an internal array helper. Check the perf
      gate pages (getElementsByTagName in loops) before/after with --perf-report. WPT: dom/collections/* (10 files),
      nodes/Element-children, ParentNode-children, ParentNode-properties, Node-childNodes, NodeList-live-mutations,
      live-collections-batched-mutations, getElementsByClassName-03..25, ParentNode-querySelectorAll-scope-change.
      Target: nodes -> +4, other -> +17.
- [ ] QDOM3 [S] (P1; Sonnet; js_dom.rs; R65; after QDOM1) `ParentNode.moveBefore(node, child)` on Element, Document,
      DocumentFragment: the pre-move checks (same shadow-including root, connected-ness equal, not a doctype/
      ancestor; HierarchyRequestError otherwise), then a move WITHOUT the removing and insertion steps: no script re-run,
      no iframe reload, focus/selection kept, custom elements get `connectedMoveCallback` (else disconnected+connected),
      one MutationObserver childList record pair, live Ranges and NodeIterators updated as for remove+insert. Native
      side: a variant of `insert_child` that skips the `log_inserted`/load hooks for the moved subtree. WPT:
      dom/nodes/moveBefore/* (62 entries; 28 are blocked only by the missing method and id globals). Target: nodes +5
      (bound +9.5; the rest needs CSS transitions, focus events, ElementInternals).
- [ ] QDOM4 [S] (P1; Sonnet; js.rs Event + ETP.dispatchEvent; R65) Event flags per DOM 2.x: stop propagation, stop
      immediate, canceled, initialized, dispatch flags. `cancelBubble` setter (true only), `returnValue` (get = !canceled,
      set false = preventDefault), `initEvent` is a no-op while dispatching and clears the stop/canceled flags,
      `composedPath()` empty after dispatch, flags cleared at the end of dispatch, setting cancelBubble before dispatch
      stops it. Listener exceptions are reported (window `error` event / `onerror`, `reportError` path) and dispatch
      continues. `handleEvent` is looked up at every invoke. `isTrusted` is an own unforgeable getter. `srcElement` is
      null before dispatch. Passive default true for touchstart/touchmove/wheel/mousewheel listeners on window, document,
      `<html>`, `<body>`. WPT: dom/events/Event-initEvent, Event-returnValue, Event-propagation, Event-cancelBubble,
      Event-dispatch-bubble-canceled, Event-dispatch-propagation-stopped, Event-stopPropagation-cancel-bubbling,
      Event-dispatch-throwing, EventListener-handleEvent, Event-isTrusted.any, event-src-element-nullable,
      window-composed-path, passive-by-default, AddEventListenerOptions-passive.any. Target: events 23.0 -> 30.
- [ ] QDOM5 [M] (P2; Sonnet; js_dom.rs Range block (QJ7) + native helpers; R65; finishes archived QA19) Range content
      operations: `cloneContents`, `extractContents`, `deleteContents`, `insertNode`, `surroundContents` (partially
      selected CharacterData split/clone, DocumentFragment result, HierarchyRequestError/InvalidStateError rules), live
      boundary-point updates on insert/remove/CharacterData changes/splitText/normalize (spec "live range pre-remove
      steps"), and Attr nodes as boundary points (nodes without `__id` today: Range-attribute-nodes). WPT in subset:
      ranges/Range-{clone,delete,extract}Contents-{around-shadow,in-ShadowRoot}, Range-attribute-nodes. Target: ranges
      17.1 -> 30. After QWPT6c also the core Range-* suite.
- [ ] QDOM6 [S] (P1; Sonnet; js.rs `ctor('Text'|'Comment'|'DocumentFragment'|'Document')`, js_dom.rs; R65) Node
      constructors that create nodes: `new Text(data = '')`, `new Comment(data = '')`, `new DocumentFragment()` (owner =
      the constructor's realm's document, `owners[]`), `new Document()` (an XML document: contentType
      `application/xml`, no doctype, origin/URL of the realm's document, createElement does NOT lowercase; reuse
      `makeDoc(false, ...)`). Calling without `new` still throws TypeError. WPT: nodes/Text-constructor,
      Comment-constructor, DocumentFragment-constructor, Document-constructor, Document-adoptNode, Node-cloneNode,
      insertion-removing-steps/*, events/Event-dispatch-bubbles-{true,false}. Target: nodes +1.5. Prerequisite for
      dom/common.js (QDOM8).
- [ ] QDOM7 [S] (P1; Sonnet; js_dom.rs `text_write_changes`, `cdReplace`, DomTable; R65) Mutable data for Comment
      (and, for QDOM8, ProcessingInstruction): rcdom's `Comment { contents }` is not a cell, so a data write replaces the
      node with a new one in its parent's child list and re-points the JS id in DomTable to it (id -> Handle and the
      reverse map), or keeps an override table read by `__d_text`, `serialize_node`, `clone_node_body`. MutationObserver
      characterData records as for Text. WPT: nodes/CharacterData-{data,appendData,insertData,deleteData,replaceData,
      substringData,surrogates,appendChild} (the Comment.* halves). Target: nodes +0.7.
- [ ] QDOM8 [S] (P1; Sonnet; js_dom.rs; R65; after QDOM6, QDOM7) ProcessingInstruction and CDATASection nodes:
      `document.createProcessingInstruction(target, data)` (target must match Name, data must not contain `?>`:
      InvalidCharacterError), `createCDATASection(data)` (NotSupportedError on HTML documents, `]]>` ->
      InvalidCharacterError); rcdom `NodeData::ProcessingInstruction` (nodeType 7, nodeName = target, `target`),
      CDATA = a Text node marked in DomTable (nodeType 4, `#cdata-section`, CDATASection.prototype); both in `__d_kind`,
      `__d_tag`, `mk` prototypes, clone, isEqualNode, textContent, serialization; CharacterData methods work on both
      (QDOM7). `XMLDocument` interface (prototype of createDocument/new Document results). Node constants:
      CDATA_SECTION_NODE, ENTITY_REFERENCE_NODE, ENTITY_NODE, NOTATION_NODE, DOCUMENT_POSITION_* on Node and
      instances. Not the WICG "PI attributes" proposal (processing-instruction-attributes*.html stay failing). WPT:
      nodes/Document-createProcessingInstruction, Document-createCDATASection, Node-textContent, Node-constants,
      CharacterData-appendChild, CharacterData-remove, rootNode, Node-isSameNode, Node-compareDocumentPosition,
      Node-contains, Node-properties, ranges/StaticRange-constructor, traversal/NodeIterator, NodeIterator-removal,
      TreeWalker, events/EventTarget-this-of-listener. Target: nodes +3, ranges +1.5, other +4.
- [ ] QDOM9 [S] (P2; Sonnet; js_dom.rs DOMTokenList block (`var DTP = DOMTokenList.prototype`, `toks`/`putToks`); R65) DOMTokenList per
      spec: an ordered set parsed from the attribute (dedupe, ASCII whitespace), add/remove/toggle/replace run the update
      steps (serialize the set; no attribute write when the element had none and the set is empty), `value`, `length`,
      `item`, iteration over the set, `supports()` throws TypeError without supported tokens, InvalidCharacterError /
      SyntaxError for whitespace / empty tokens, one DOMTokenList object per element (`[SameObject]`), relList etc. on
      the elements in lists/DOMTokenList-coverage-for-attributes. WPT: nodes/Element-classlist, lists/* (4 files).
      Target: other +4.
- [ ] QDOM10 [S] (P2; Sonnet; js.rs ETP.dispatchEvent, attrHandler; R65; after QDOM4) `window.event`: set to the
      current event while a listener of a node in the page's realm runs, restored afterwards (also after a throwing
      listener), `undefined` for listeners inside a shadow tree, unset outside dispatch. WPT: dom/events/event-global*,
      window-event-restored-after-throwing-onerror. Target: events +3.
- [ ] QDOM11 [M] (P2; Sonnet; wpt_th.rs, js.rs `run_page`, js_dom.rs `__d_load_xml`/`xml_to_dom`; R65) XML documents.
      A response whose MIME type is XML (`application/xhtml+xml`, `image/svg+xml`, `text/xml`, `application/xml`,
      `+xml`) is parsed with the existing roxmltree path, not html5ever, for the top-level document and for iframes. The
      document is not an HTML document (QDOM17 flag); `<script>` elements in the XHTML/SVG namespaces run in tree order
      (CDATA content, `href`/`xlink:href` for SVG). A well-formedness error shows the parsererror document. App rendering
      of such documents can stay as it is (box tree from the same DOM); only the parse path changes. WPT: the 18 ERROR
      `.xhtml/.xml` files and 13 CRASH `.svg` files in dom/nodes (Element-childElementCount-*, Element-firstElementChild-*,
      ...), Document-createElement(NS) (XML iframes: 488 subtests), Document-createProcessingInstruction-xhtml,
      Node-lookupPrefix.xhtml. Target: nodes +5 (13 SVG files also start counting).
- [ ] QDOM12 [S] (P2; Sonnet; js_dom.rs; R65) `Text.splitText(offset)` (IndexSizeError, new node after it, live range
      updates once QDOM5 has them), `Text.wholeText`, `Node.normalize()` (merge adjacent Text, drop empty, on Document and
      Element). WPT: nodes/Text-splitText, Text-wholeText, Node-normalize, Node-normalize-2, ranges/Range-mutations-
      normalize. Target: nodes +1, ranges +2.
- [ ] QDOM13 [S] (P2; Sonnet; js.rs attrHandler/`fwd`, js_dom.rs; R65) Event handler IDL attributes as accessors:
      GlobalEventHandlers/WindowEventHandlers/DocumentAndElementEventHandlers on HTMLElement, Document and Window
      (default null; setting a non-function stores null; the content attribute still compiles lazily). `<body>`/
      `<frameset>` `onload`/`onerror`/`onresize`/... get/set the Window's handler (WindowEventHandlers). Include
      `onanimationstart/iteration/end/cancel`, `ontransitionrun/start/end/cancel`, and the legacy
      `webkitAnimation*`/`webkitTransitionEnd` mapping (an unprefixed event with only a prefixed listener fires the
      prefixed type). Only the names Chrome desktop exposes: NO `ontouch*` on window/document (sites treat
      `'ontouchstart' in window` as "touch device"). Add `HTMLFrameSetElement`. WPT: dom/events/webkit-*-event (4),
      Body-FrameSet-Event-Handlers. Target: events +2 (and the 40 non-cancelable-when-passive files, which assign
      `document.body.onload`, then run once QWPT7 exists).
- [ ] QDOM14 [S] (P2; Sonnet; js_dom.rs; R65) Namespaces: `lookupNamespaceURI`, `lookupPrefix`, `isDefaultNamespace`
      (DOM "locate a namespace" over element ns/prefix and xmlns attributes), `Element.prefix` from the qualified name,
      `getElementsByTagNameNS(ns, local)` on Document/Element, and `getElementsByTagName(qualifiedName)` matched natively
      by qualified name (HTML elements in HTML documents compare lowercased; others exact; `*`), not through
      `__d_query` (the selector parser rejects `te:st`). Returns QDOM2 collections when that has landed. WPT:
      nodes/Node-lookupNamespaceURI, Node-lookupPrefix.xhtml (after QDOM11), case, Document-getElementsByTagName(NS),
      Element-getElementsByTagName(NS), collections/HTMLCollection-empty-name. Target: nodes +1.2.
- [ ] QDOM15 [S] (P2; Sonnet; js.rs Event ctors, js_dom.rs `doc.createEvent`; R65; after QDOM4) Event interfaces:
      UIEvent/MouseEvent/KeyboardEvent/FocusEvent/WheelEvent/InputEvent/CompositionEvent constructors read their init
      dictionaries (`view` must be a Window or null: TypeError otherwise, `detail`, modifier keys...), `Event.NONE/
      CAPTURING_PHASE/AT_TARGET/BUBBLING_PHASE` on the constructor and prototype, `createEvent` accepts the spec's
      interface list (BeforeUnloadEvent, CompositionEvent, DeviceMotionEvent, DeviceOrientationEvent, DragEvent, ...) and
      throws NotSupportedError otherwise, `dispatchEvent` of an uninitialized or already-dispatching event throws
      InvalidStateError. WPT: dom/events/Event-subclasses-constructors, EventTarget-dispatchEvent, Event-constructors.any,
      Event-constants, EventTarget-constructible.any. Target: events +1.5.
- [ ] QDOM16 [S] (P2; Sonnet; js_dom.rs; R65) ChildNode/ParentNode on every node kind: `before`, `after`,
      `replaceWith`, `remove` on CharacterData and DocumentType (today only on `ElementProto`), `replaceChildren`/`append`/`prepend` on Document, `Symbol.unscopables` for them;
      `document.doctype` on the page document (the page's DocumentType node). WPT: nodes/ChildNode-after,
      ChildNode-before, ChildNode-replaceWith, DocumentType-remove, Document-doctype, ParentNode-replaceChildren,
      remove-unscopable. Target: nodes +1.2.
- [ ] QDOM17 [S] (P2; Sonnet; js_dom.rs `makeDoc`, `__d_tag`, `doc.createElement`; R65; after QDOM8) Document
      flavours: a per-document "HTML document" flag (true for the page and createHTMLDocument, false for createDocument,
      new Document(), XML parses). `createElement` lowercases only in HTML documents (XHTML namespace for
      application/xhtml+xml, null otherwise); `tagName`/`nodeName` upper-case only for HTML-namespace elements in HTML
      documents and include the prefix; `nodeValue` and `textContent` null for Document and DocumentType (setting is a
      no-op); `charset` and `inputEncoding` aliases of `characterSet` on every document. WPT: nodes/DOMImplementation-
      createDocument (306 subtests), DOMImplementation-createDocumentType, Element-tagName, Element-tagName-adopt,
      Node-nodeValue, Node-textContent, DOMImplementation-createHTMLDocument. Target: nodes +1.2.
- [ ] QDOM18 [S] (P3; Sonnet; js_dom.rs insert path (`insertBefore`/`appendChild`/`replaceChild`/`append`/`prepend`/
      `replaceChildren`); R65) DOM "ensure pre-insert validity" and "replace" checks: HierarchyRequestError for a
      Document parent with a second element, a fragment with two elements or with text, a doctype after the element,
      text directly under a Document; NotFoundError for a wrong child; checks before any mutation (fragments keep their
      children on failure). WPT: nodes/Node-insertBefore, Node-replaceChild, ParentNode-append, ParentNode-prepend,
      ParentNode-replaceChildren, append-on-Document, prepend-on-Document. Target: nodes +0.8.
- [ ] QDOM19 [S] (P3; Sonnet; js_dom.rs `checkName`; R65) Current DOM name validation (whatwg/dom 2025): "valid element
      local name", "valid attribute local name", "valid doctype name", `validate and extract` for the NS variants
      (NamespaceError rules for xml/xmlns prefixes). Used by createElement(NS), createAttribute(NS), setAttribute(NS),
      toggleAttribute, createDocumentType, createDocument. WPT: nodes/DOMImplementation-createDocumentType,
      DOMImplementation-createDocument, Document-createElementNS, Document-createAttribute, Document-createElement,
      attributes, name-validation (timeout=long). Target: nodes +0.3 (many subtests in few files).
```

### 5. Not DOM core, or not worth doing now

- **dom/observable (29 files, 0 %)**: the Observable API is tentative in WPT and in no stable spec. It would be a
  self-contained JS prelude (Observable, Subscriber, ~20 operators, `EventTarget.prototype.when`, about 600 lines), so it is
  cheap to isolate, but no real site needs it. Recommendation: not now. If the area score must move, it is one
  separate M item, P3.
- **dom/events testdriver files (94 entries)**: wait for QWPT7. Building more event code first does not help them:
  they time out at `action_sequence()`. The 40 `non-cancelable-when-passive/*` files also need QDOM13 (they set
  `document.body.onload`).
- **OpaqueRange (dom/ranges/tentative, 16 files)** and **PI attributes** (`processing-instruction-attributes*.html`,
  891 subtests in 2 files, WICG declarative-partial-updates): proposals, not now.
- **Selectors** (`:has()`, `:host`, `:scope` inside `:not()`, `:invalid`, `:modal`, namespace attribute selectors,
  XML case rules: 14 dom/nodes files, about +2): the selectors crate integration; CSS area, not QDOM.
- **Selection API** (`getSelection` missing; 3 ranges files, moveBefore/selection-preserve): editing area; its own item
  when wanted.
- **iframes created from script** (`createElement('iframe'); src=...; appendChild`: load never fires, so
  Document-contentType/* (9 files), getElementsByClassName-31, Element-matches/webkitMatchesSelector (their selector
  iframes) TIMEOUT), and cross-realm checks (`node-creation-realm`, `node-realm-*`, `Event-timestamp-cross-realm-getter`,
  `event-listener-not-fully-active-*`): HTML navigation/realms, QEL or a new iframe item.
- **Scroll events / scrollend** (dom/events/scrolling, non-testdriver half), **Web Animations** (`element.animate`
  missing, 14 hits), **activation behaviour** (Event-dispatch-single-activation-behavior 52 subtests,
  Event-dispatch-click: checkbox/radio/details/form activation), **ElementInternals** (`attachInternals`), **popover**,
  **XPath** (`document.evaluate`/`XPathResult`: 2 dom/other files, 20 subtests; L-sized evaluator): other areas or later.
- **HTML scripting bug seen on the way**: a classic `<script src>` whose response is 404 is EXECUTED (wire's
  "Not found" body gives `SyntaxError: Unexpected identifier 'found'`, e.g. moveBefore/fullscreen-preserve). HTML says
  a non-ok status fires `error` and runs nothing. Proposed for the scripting area:
  `QSCR404 [S] (P2; Sonnet; js.rs script fetch) a classic/module script whose response status is not 2xx is not run;
  fire error on the element.`

### 6. Tooling items found here (for the coordinator)

```
- [ ] QWPT6b [S] (P2; Sonnet; tools/wpt-subset.py, wpt-th/; R65 2.1) The subset filter misses testdriver users
      that load it from HTML (`<script src="/resources/testdriver.js">`); it only reads `// META: script=`. 102 dom
      entries (94 dom/events) are in the phase-1 baseline although they need trusted input. Either drop them (as the
      phase-1 rule says) or move them to their own area `<area>/testdriver` so the DOM scores show DOM work; regenerate
      subset + expected + areas.txt.
- [ ] QWPT6c [S] (P2; Sonnet; tools/wpt-subset.py; R65 2.1; after QDOM6, QDOM8) Include the dom/* timeout=long files
      (46: the core Range-*, NodeIterator, TreeWalker, Document-characterSet-normalization-1/2, ParentNode-querySelector-
      All, NodeList-static-length-getter-tampered-*). With the virtual clock they are cheap: all 46 ran in 9 s at 4 jobs
      (load 54) with my own wire. Today 31 of them ERROR at `xmlDocument.createCDATASection`, so measure their wall time
      again after QDOM8 before adding them.
```

Raw per-URL JSON and the classification scripts were in the investigator's worktree (`.qwork/res`, scratchpad); they
are not committed (re-run: section 1).

## R63: QOOM1: a scratchy grew to 15 GB and took the session down (13:39, OOM killer; QPROF1's headless runs hit the 6 GB cap in 26-33 s). The page is the github.com home. Its hero section is re-rendered about once a second in scratchy, each time inserting two new `<video>` elements (an AV1 and an HEVC file) and removing the old ones. A removed media element was only paused: `__media_drop` existed but nothing called it, so each one kept its libmpv player (16 dav1d workers + 16 HEVC frame threads, decoded frames, demuxer cache: ~250 MB) until the page went away. Fix: a media element still out of the document when its removal's MutationObserver record is delivered is paused and its player destroyed; it stays inert until it is put back, then loads again; a destroyed player asks for a `malloc_trim` (Opus, 2026-11-20) — FIXED

**Reproduced** (own wire, `scratchy --perf-report --budget-ms 20000 --seconds 30 https://github.com/` inside
`systemd-run --user --scope -p MemoryMax=3G -p MemorySwapMax=0`, RSS and the per-thread names sampled every 0.5-1 s; main at
18d0527): 152 MB at 2 s, 834 MB at 3 s, 1.35 GB at 5 s, 2.6 GB at 8 s, killed at ~9 s, every time (5 runs). The thread
names give it away: one `scratchy-video` render worker, one `core`, 16 `dav1d-worker` and 16 `av:hevc:dfN` per player, and
the player count went 2, 3, 4, 6, 8, 10, 12, 14 in 7 s (3.8 GB at 14 players, ~270 MB each). A temporary log showed the
same two URLs (`hero-light-*.mp4`, `copilot-fusion-light-av1-crf60.mp4`) loaded again for new element ids every ~0.7 s,
and the old elements removed (`removedNodes`) each time. With libmpv replaced by the placeholder backend (temporary switch)
the same page stays flat at 336 MB for 30 s (the re-render loop needs the media events).

**What did NOT grow** (bisected first): the QM1g2a render slot holds at most one frame, the QM1g2a1 frame pool keeps at
most 4 buffers, so no frame queue builds up headless. QMEM3g memprof: Rust-allocated live heap 183 MB at 1.4 GB RSS
(`compute_custom_props` ~100 MB, the rest small). V8 heap 53 MB. `smaps`: the growth is malloc arenas (`[heap]` 934 MB
plus 62-267 MB anonymous mappings), i.e. C allocations by libmpv/ffmpeg/dav1d. A page-fault profile
(`perf record -e page-faults --call-graph dwarf`): 33 % `dav1d_decode_frame_init` memset, the rest HEVC frame pools,
lavf probing, set_sps: decoder start-up of new players, not one player growing.

**Cause.** media_prelude.js's removal hook only paused a removed element (`setTimeout(..., 0)`, then `__media_pause`).
The element's `MediaCore` (and with it the `MpvBackend`: the mpv core, its decoders and render worker) stayed in
`MediaHost.els` until the page was torn down. The D7 cap (at most 2 PLAYING) does not bound this: all of these were paused.

**Fix.**
1. media_prelude.js `gone`: when the removal record is delivered (microtask after the removing script, so a move inside one
   script is complete) and the element is not connected, it is paused, `__media_drop(id)` destroys its core and player
   (`MpvBackend::drop`, ~1.5-6 ms), and its `pause` event / play() rejections still go out on the next pump. The first
   attempt kept the old `setTimeout(0)` and dropped there: drops then lagged behind github's loop (16 drops for 34 loads,
   18 players at 13 s), hence the drop in the observer callback itself.
2. Inert while out of the document: github's code touches the removed elements (`pause()`, `muted`), and the element
   layer used to answer such a call with a fresh load. A dropped element now never loads while disconnected (`load()`, a
   new `src`, `play()` rejects `NotSupportedError`); `MediaHost` answers its id with a throwaway inert core (no backend)
   until `revive`. Put back in the document (or touched while connected), it is revived and loads again from the start.
   Spec difference: a removed element keeps its resource in browsers; here it reloads when reinserted.
3. `MediaHost::remove` asks for a `malloc_trim` (at most one per second): after 282 player drops glibc held 1.86 GB free in
   its arenas (`malloc.free_kept`) against 0.78 GB in use, and RSS crept from 0.8 to 2.8 GB in 160 s without the trim.

**After** (same recipe, final build): github.com home, 3G cap, players 2-4 at every sample.

| run | RSS | players | notes |
|---|---|---|---|
| main 18d0527, 30 s headless | 2.6 GB at 8 s, killed at ~9 s | 14 at 7 s | 5/5 runs |
| fix (drop only), 170 s headless | 0.8-1.3 GB, then creeping to 2.8 GB | 2-4 | free_kept 1.86 GB |
| fix + trim, 170 s headless | 0.8-1.3 GB | 2-4 | 278 players destroyed |
| final, 300 s headless (quiet machine, load 0.4) | 1.0-2.0 GB, no trend after ~3 min (1.6-2.0 GB at 170-310 s), peak 2.18 GB | 2-4 | malloc in_use 1.26 GB, V8 heap 197 MB at the end |
| experiment: final + `vd-lavc-threads=4`, 150 s | 0.9-1.3 GB | 2-4 | not committed (QOOM1b) |
| YouTube watch, nested dwl 960x1006, 20 min, click to play + pointer motion every 10 s | 495 MB at start, 617-665 MB from 2 min to 19.6 min | 1 | flat (load avg 17 at start: other agents' builds) |

YouTube watch does not remove its `<video>` (one player for the whole run), so it was not this bug: the 15 GB kill at
13:39 was a github.com run (QPROF1, R62). YouTube is flat at ~0.64 GB over 20 min.

The github.com page stays bounded but above the ~1 GB gate: it still starts two new decoders about once a second (the
re-render loop, QOOM1a), so at any moment 2-4 players exist plus the ones being torn down, and glibc's arenas churn
(`free_kept` 4.2 GB by mallinfo, mostly already trimmed). Without the loop the page holds its 2 hero players (~0.8 GB).

**Left out (proposed).**
- QOOM1a [S]: why github.com's hero re-renders every ~0.7 s in scratchy (a React error/retry loop? the
  `':not(:defined),react-partial,...' is not a valid selector` SyntaxError from `querySelectorAll` repeats in the log). It
  costs a player start-up (two decoders) per second even with this fix.
- QOOM1b [S, measure]: per-player memory: mpv's default `vd-lavc-threads` gives 16 dav1d + 16 HEVC threads and ~250 MB per
  player; Chrome caps decoder threads by resolution. Measure playback (YouTube 1080p/4K AV1) at 4/8 threads before capping.
- QOOM1c [S]: a cap on players that exist at once (not only playing ones, D7), releasing the oldest paused,
  out-of-view player transparently (reload on play), as a guard for pages that keep many `<video>` elements in the document.

## R62: QPROF1 part A: full re-profile of main bbc569d (after QOBS9). Layout and restyle are still the largest share (forced layout 24-57 % of the JS/main thread headless, App builds 34-49 % of the window main thread: all QINC1). Outside layout, four scratchy-side hotspots qualify: the iframe-window prelude walks the whole document on every DOM removal (12 % of YouTube watch, 18 % of YouTube results), IntersectionObserver evaluation re-cascades ancestor chains it does not need (10-12 % on YouTube), the MessagePort shim drains every message in one task (one 375-389 ms task on bbc.com), and `dlopen(libmpv)` runs on the JS thread at the first `<video>` (118-131 ms, ~10 % of Wikipedia headless); plus V8 compile on the JS thread (9-25 %), mpv's software scaler (~10 % of process CPU while playing), and a profiler artifact. github.com home grew to 5.4 GB in 23-29 s, reproduced 2 of 2 (QOOM1) (Opus, 2026-11-20, measure only, docs only) — OPEN: proposed QPROF1-a .. QPROF1-g (section 7); layout/restyle items covered by QINC1

**Status: measurement only, docs only. No source change.** Part B (the joint session with the user) is not part of this section.

### 0. Setup
- **Binary:** origin/main **bbc569d** (QOBS9 #444 on top of 0954866 #441), clean tree, `cargo build --release`
  (V8 + media) into the worktree's own target, copied aside; the `profiling` profile of the same tree
  (`-C force-frame-pointers=yes`) for `tools/profile.sh` and the window profiles. Reports say `"commit":"bbc569d9d3"`.
- **Machine:** the 5700X of PERF.md "Baseline (V8, 5888220, 5700X, 2026-11-20)". **Load:** quiet for the first
  headless series (11:29-11:41, 1-min load 0.2-1.5). From ~11:45 the coordinator started 4 QINC1 build agents; the
  rule then was "wait up to 3 min for load < 3, else run and record the load". Every measured run ran under
  `flock /tmp/scratchy-perf.lock` and (after 14:00) inside `systemd-run --user --scope -p MemoryMax=6G -p
  MemorySwapMax=0`. 1-min load at the start of each run is given with each table. CPU-share profiles are not
  affected much by load; the window runs at load 13-19 have inflated ms (said where it matters).
- **Wire:** own wire (built from the same tree) on a scratchpad socket. Default User-Agent everywhere; YouTube watch
  also twice with the R43 Chrome 131 UA for comparison with QOBS9.
- **Headless:** `--perf-report --budget-ms 20000`, YouTube watch 60 s, the rest 30 s, 2 runs each plus one
  `tools/profile.sh` run (perf `-F 999 -g` + V8 JIT map). github.com was run as `github.com/Loupax/vibe-it` (the
  QOBS9 page) because **github.com home is the QOOM1 runaway** (section 6).
- **Window:** own nested dwl (`tools/nest.sh`) placed by KWin (960x1006; the first run got 960x489), own wire,
  `SCRATCHY_FRAME_STATS`, `SCRATCHY_MEDIA_AO=null`. YouTube watch 80 s: click on the player at ~15 s (playback
  starts), pointer moving over the player for ~8 s at ~28-36 s (200 moves, 40 ms apart), then parked on the top bar.
  Wikipedia Rust 45 s: wheel 6 x 3 notches at ~13-17 s, pointer sweeping the text column (160 moves, 50 ms apart)
  at ~20-28 s, wheel back up. 3 runs each, the third under `perf record` (profiling binary).
- **Foreign input check:** every second's `input` counters are matched against the inject log. ytw-w1 had 182
  pointer moves at 12-14 s that were not mine (the user's pointer crossed the window before the play click):
  that run's first 15 s are dropped; its playback/pointer phase was clean. All other runs show only the single
  enter event at t=1 s.
- **Compared with:** R44 (8d7d692, before QOBS2-10/QY53/QY54/QINC1 design) and PERF.md's QOBS9 baseline
  (5888220 = PR #432, QY54b1). Main moved since by #435/#438 (link clicks, file:// URLs), #436 (iframe realm
  WebSockets), #437 (YouTube scrolling: body overflow propagation, absolute boxes in the page height, scroll keys),
  #440 (hidden viewport, scroll chaining) and #441 (offsetParent of a fixed box), plus docs. Headless numbers below
  are within run-to-run noise of QOBS9 on every page (section 1); none of these commits moved a measured hotspot.
  R44's three unfiled hotspots are gone: `El::has_class` 28-32 % then, 1.8 % (YT watch) / 2.8 % (Wikipedia) of
  the main thread now; `intern_custom_props` 5.3 % -> 0.2 %; `layout::memo_key` 5.5 % -> 0.1-0.7 % (`core::fmt`
  0.5-3.1 %). QOBS9's `find_base_href` 2.5 % and `descendants` 2.5 % self are now 2.0-3.2 % and 2.1-2.7 % self, but
  they are part of one 12-18 % inclusive cost (section 3.1).
- **Profile caveat (tooling, QPROF1-g):** perf `-F 999` is frequency mode PER THREAD, so a thread that barely runs
  gets a tiny period and is heavily oversampled. `js-watchdog` shows as 5-30 % of all samples (Wikipedia 30 %),
  but `/proc/<pid>/task/*/stat` says it used 0.13-0.39 s of CPU in an 80 s window run (< 1 % of the process). Short
  per-script fetch threads (bbc: 273 threads named `scratchy`) are inflated the same way, and `summary.txt`'s
  per-thread split is by `comm`, so they are counted with the main thread. **All shares below are of the MAIN
  thread only** (stacks rooted at `_start`; the headless JS thread is the main thread), never "all samples".

### 1. Headless runs (median of 2, (min-max))

| page | load | forced layouts n / ms (max) | IO-reason layouts n / ms | raw CPU ms | compile ms | load wall / CPU ms | RSS MB (peak) | JS heap MB | QOBS9 for comparison |
|---|---|---|---|---|---|---|---|---|---|
| YouTube watch G1AeBgalFZk, default UA, 60 s | 1.11, 0.48 | 114 (111-116) / 2630 (2543-2717) (80) | 28 / 875 | 7650 (7486-7815) | 303 | 3601 / 1027 | 450 (471) | 157 | (R43 UA only) |
| YouTube watch, R43 UA, 60 s | 2.87, 1.52 | 109 / 2552 and 168 / 7369 (bimodal) | 22-32 / 712-1293 | 7306, 14671 | 353 | 3895 / 1160 | 465 (487) | 155 | 175 / 6330, raw 12328, RSS 502 (513) |
| YouTube results `?search_query=rust`, 30 s | 0.98, 0.90 | 38 (34-42) / 983 (844-1121) (56) | 24 / 781 | 4542 (4380-4703) | 292 | 2607 / 677 | 360 (360) | 127 | 10 s suite: 39 / 1028, raw 4482, RSS 345 |
| en.wikipedia Rust, 30 s | 1.44, 1.44 | 4 / 685 (592-779) (287) | 3 / 554 | 1068 (940-1196) | 17 | 252 / 44 | 233 (233) | 18 | 4 / 659, raw 1023, RSS 244 |
| bbc.com/news, 30 s | 1.66, 1.26 | 14 / 395 (77-80 max) | 2 / 50 | 1136 | 59 | 1119 / 161 | 134 | 38 | 10 s: 14 / 416, raw 1196, RSS 142 |
| social.loupax.com/discover, 30 s | 1.59, 1.59 | 10 / 95 (45) | 0 | 140 | 0.5 | 366 / 149 | 57 | 3.6 | 10 / 83, raw 121, RSS 58 |
| github.com/Loupax/vibe-it, 30 s | 2.87, 1.85 | 0 | 0 | 124 | 0.2 (modules) | 1017 / 145 | 65 | 14 | raw 129, RSS 67 |
| github.com home | - | killed by the 6 GB cap (section 6) | | | | | | | |

No challenge page, no budget kill. js_errors: YouTube watch 2, github 10 (same as QOBS9), others 0. YouTube watch
with the R43 UA is bimodal: run 2 got the player that does the 1.44 s `(anonymous) player_es6` timer with 13
layouts (1312 ms) at 24.8 s (QOBS9's 0.9-1.4 s timer); run 1 and both default-UA runs did not.

Forced layouts by page caller (`by_caller`, per run): YT watch default UA `calculateCanCollapse` 16-17 x 939-998 ms,
`numberOfGhostCommentsChanged` 6 x 283-290, `getPlayerSize` 42 x 237-244, `attached` 2 x 45-47; R43-UA run 2:
`getPlayerSize` 78 x 2877, `calculateCanCollapse` 18 x 1314, `e6a` 18 x 1266. YT results `_.k9s` 5 x 61-62, `nO`
44-48. Wikipedia `scrollToActiveSection` 1 x 110-153 (the rest is scratchy's IO evaluation). bbc `(anonymous)` 10 x
233-244. social `clamp.js` (anonymous) 6 x 62-65.

Longest callbacks (raw ms, forced layouts in it / their ms, when):

| page | longest callbacks |
|---|---|
| YT watch | `inline script #28` 360-364 (27 / 121-126) @3.1 s; timer `d` web-animations-next-lite 341-342 (4 / 63-65) @6 s; ytmainappweb script 199-207 (0) @3.5 s; **observer_delivery 194-195 (0 layouts)** @6.3 s; timer `d` 163-164 (2 / 149) @13 s |
| YT results | ytmainappweb script 278-282 (1 / 14) @2.6 s; promise_job `event:yt-page-data-fetched` 267 (3 / 42) @3.1 s; **observer_delivery 158-167 (0)** @3.4 s; timer `bound` 139-149 (1 / 44-48) |
| Wikipedia | observer_delivery 392-495 (2 IO layouts / 370-467) @1-1.4 s; observer_delivery 111-175 (1 / 111-173); **timer `(anonymous) media:98:54` 118-131 (0 layouts)** @0.5-0.7 s |
| bbc | **timer `(anonymous) globals:114:17` 375-389 (1 / 77-80)** @1.5-2.7 s; observer_delivery 294-308 (12 / 283-296); promise_job 62-63 (0) `onScriptLoad` require.js |
| social | `clamp.js` 82-85 (6 / 62-65); `egg.js` 53-57 (4 / 30-33) |
| github repo | module `core-*.js` 68 (0); everything else < 25 |

The bold ones are >= 100 ms with no or little forced layout; their causes are in sections 3-4.

### 2. Where the main thread goes (profiles, headless)

Main-thread samples (perf `-F 999`, profiling build, SCRATCHY_V8_FLAGS set so no startup snapshot), assigned to the
FIRST matching bucket in this order: forced layout (`GeomCache::refresh`); iframe-window prelude (`sweepFrames`,
`syncWin`, `frameList`, `frameSrcOk`); App build; text shaping; paint; image decode; CSS parse; HTML parse; V8
compile/parse; V8 GC; native DOM bindings (`install_*::{closure}`, no layout under it); media; page/prelude JS
(everything else in V8). Loads 0.2-1.6.

| bucket, % of main thread | YT watch (7 482 samples) | YT results (4 850) | Wikipedia (1 113) | bbc (1 330) | social (328) | github repo (243) |
|---|---|---|---|---|---|---|
| forced layout (QINC1) | 31.9 | 23.4 | 56.8 | 29.2 | 52.1 | 0 |
| **iframe-window prelude** | **12.0** | **17.6** | 0.1 | 0.1 | 0 | 0 |
| native DOM bindings (no layout) | 16.7 | 20.7 | 19.7 (of it media load 9.6) | 3.4 | 4.3 | 4.5 |
| page / prelude JS | 17.5 | 15.2 | 5.2 | 29.2 | 9.8 | 11.9 |
| **V8 compile/parse** | **9.3** | **10.6** | 7.9 | **24.8** | 14.6 | 38.7 |
| V8 GC (main-thread part) | 7.6 | 6.9 | 1.6 | 6.6 | 1.5 | 1.2 |
| text shaping / paint / CSS parse / HTML parse | < 1 each | < 1.2 each | 3.1 / 0.2 / 0.4 / 1.8 | < 1.1 each | < 2 | < 1 |

Cross-checks by frame (inclusive, main thread): `StyleCache::style_of_chain` 12.6 % (watch) / 16.3 % (results), of
which from scratchy's IO prelude (`ioShown`, `ioEval0`, `ioPad`) ~10 % / ~12 %; `obsFlush|ioEval` 24.0 % / 37.5 %
(includes the IO forced layouts); V8 Heap/Scavenger/MarkCompact 11.4 % / 10.7 % (incl. GC triggered inside page
JS); `Compiler|Parser|Scanner` 12.2 % / 13.4 %. Wikipedia: `_dl_*|do_lookup_x|dlopen` 10.5 %, `media_mpv` 11.0 %
(the libmpv load). bbc: inside `globals:114:17` (the MessagePort flush) 38.8 % of the main thread.

Top 15 SELF, main thread (YT watch / YT results / Wikipedia / bbc):
- YT watch: memmove 5.0, `matches_complex_selector` 2.2, [unknown] 2.1, **`js_dom::descendants` 2.1**, SipHash
  `write` 2.0, memcmp 2.0, **`net::find_base_href` 2.0**, `unlikely` 1.7, `LocalNameStaticSet` deref 1.6, `u8to64_le`
  1.3, V8 `Scanner::Next` 1.3, `_int_malloc` 1.3, `do_lookup_x` 1.1, `rotate_left` (SipHash) 1.0,
  `Declarations::clone` 1.0.
- YT results: memmove 5.9, **`find_base_href` 3.2**, [unknown] 2.8, **`descendants` 2.7**, SipHash 2.4,
  `matches_complex_selector` 2.4, `unlikely` 1.9, memcmp 1.6, `is_element` 1.5, `Scanner::Next` 1.5,
  `LocalNameStaticSet` 1.5, `rotate_left` 1.5, `u8to64_le` 1.4, `_int_malloc` 1.4, `LoadIC_Megamorphic` 1.0.
- Wikipedia: **`do_lookup_x` 6.4** (libmpv relocation), [unknown] 5.8, memmove 3.1, `_int_malloc` 3.0,
  `_int_free_create_chunk` 2.2, `descendants` 1.6, `matches_complex_selector` 1.6, closure 1.5, free 1.4,
  `select_unpredictable` 1.3, `strcmp` 1.1 (dlopen), memcmp 1.1, `unlink_chunk` 1.0, `eq<CssWide>` 0.8, `unlikely` 0.8.
- bbc: [unknown] 6.2, `Declarations::clone` 4.4, `Scanner::Next` 3.1, memmove 2.3, `Builtin_ArrayConcat` 1.4,
  `LoadICGenericBaseline` 1.3, `LookupIterator` 1.2, `_int_malloc` 0.9, `eq<Length>` 0.8, free 0.8,
  `Declarations::eq` 0.8, `LoadIC_Megamorphic` 0.7, `ElementsAccessor` 0.7, `Runtime_GetProperty` 0.7, JS `ec` 0.6.

Top 15 INCLUSIVE (wrappers such as `Invoke`, trampolines, `run_due`, microtask entry dropped), main thread:
- YT watch: ytmainappweb JS 38.1, `GeomCache::rect_of`/`refresh` 32.0/31.9, `build_layout_tree_with_resources`
  27.7, `drain_jobs` 24.9, `obsFlush` 23.1, web-animations-next-lite `d` (and its two inner frames) 23.1, `ioEval`
  22.4, `gRect` 21.1, `scheduler.js K` 20.5, `gClient` 20.0, two more ytmainappweb frames 18.8 / 17.6,
  `resolve_style_for` 17.2.
- YT results: `obsFlush` 36.9, ytmainappweb JS 36.5, `ioEval` 36.1, `drain_jobs` 26.0, `PromiseFulfillReactionJob`
  25.0, `gRect` 23.5, `GeomCache::rect_of` 23.5, then `refresh`, `build_layout_tree_with_resources`, `sweepFrames`/
  `syncWin`/`frameList` (17.6), `style_of_chain` 16.3, `ioShown` 8.6, `Compiler` ~10.
- Wikipedia: `obsFlush` 58.2, `GeomCache::rect_of`/`refresh` 56.8, `ioEval` 47.3, `build_layout_tree_with_resources`
  36.0, `gRect` 32.5, `gViewRect` 24.7, `ioEval0` 24.6, centralNotice load.php JS 21.7, `build_children_with_prefix`
  .. `build_box_for_element_body` 17.9, media `doLoad` -> `MpvBackend` 11.0, `_dl_*` 10.5.
- bbc: Next.js chunk `L` 31.5, `GeomCache::rect_of`/`refresh` 29.2, **`globals:114:17` 28.3** (+ the optimized
  copy: 38.8 total), `P` 28.0, `build_layout_tree_with_resources` 23.7, `ld` 23.0, `iP` 22.3, `drain_jobs` 21.3,
  `obsFlush` 20.3, `gClient` 20.2, `build_children_with_prefix` .. `build_box_for_element_body` 19.4.
- social and github repo: too few samples (328 / 243) to rank; social is 52 % forced layout (clamp.js, egg.js),
  github repo 39 % V8 compile (module scripts).

### 3. Non-layout hotspots, with the cause

**3.1 The iframe-window prelude walks the whole document on every DOM removal (YT watch 12.0 %, YT results
17.6 % of the main thread).** js_dom.rs QPT1b/QPT1d prelude: `__d_remove` (every `removeChild`/`remove`),
`__d_sethtml` (`innerHTML=`) and `__d_settext` (`textContent=`) call `sweepFrames()`, which, once the page has
ever had a window-backed iframe (`ixN > 0`; YouTube does), calls `syncWin()` -> `frameList()` ->
`doc.getElementsByTagName('iframe')` (a full-document `js_dom::descendants` walk) and, per iframe,
`frameSrcOk(el)` -> `__d_iframe_page(id)` -> `net::find_base_url(&root)` -> `find_base_href`, a recursive walk of
the WHOLE document looking for `<base>` (YouTube has none, so it never stops early), then `syncWin` re-checks
names. So one DOM removal costs 1 + (number of iframes) full-document walks. Callers (nearest page JS):
ytmainappweb (Polymer `_setUnmanagedPropertyToNode`, `disconnectedCallback`, `F2`), webcomponents-all-noPatch
`m.Qa`. This is QOBS9's `find_base_href` 2.5 % + `descendants` 2.5 % self: inclusive it is 5x that. Not layout.
Still present on current main (e01440f QPT1-D1 kept `frameSrcOk`/`frameList`/`sweepFrames`). -> **QPROF1-a**.

**3.2 IntersectionObserver evaluation re-cascades ancestor chains (YT watch ~10 %, YT results ~12 % of the
main thread, outside forced layout).** `gRect` calls `ioShown(el)` -> `__d_cs_hidden` ->
`StyleCache::display_none_in_chain` -> `style_of_chain` (a cascade of the element and every ancestor) BEFORE
`__d_rect`, which already returns `[]` for an element without a box (display:none in the chain = no box), and
`ioEval0` reads `overflow-x`/`overflow-y` through `__d_cs` and `border-*-width` through `gPx` (`ioPad`) for every
ancestor of every target: more `style_of_chain` calls. `StyleCache` is keyed by the write generation, so after each
DOM write the chains are cascaded again. The forced layout that `__d_rect` runs computes the same styles for the
same elements. This is restyle work, but redundant: QINC1p (getComputedStyle from the element table) would make
it cheap after the engine flips; a prelude fix makes it go away now. -> **QPROF1-b**.

**3.3 The MessagePort shim runs a whole message queue in one task (bbc: one 375-389 ms task, 38.8 % of bbc's main
thread inside it).** globals_prelude.js `flush(p)`: one `setTimeout(0)` callback that `while (p.__q.length)`
dispatches `message` events, INCLUDING messages posted during the loop. A scheduler that yields through
`MessageChannel` (React's scheduler does: post a message, run ~5 ms of work, post again; bbc.com/news is a Next.js
page and the work under the flush is its chunk `L`/`P`/`ld`/`iP`) never yields in scratchy: all its slices run in
one callback, one 375-389 ms task (1 forced layout of 77-80 ms in it). In a browser each `postMessage` is its own
task, so timers, input and rendering run between slices. -> **QPROF1-c**.

**3.4 `dlopen(libmpv)` on the JS thread at the first media element (Wikipedia: one 118-131 ms timer
`media:98:54`, `do_lookup_x`/`_dl_*` 10.5 % of the headless main thread).** The article has a `<video>`; the media
prelude's `doLoad` creates the first `MpvBackend`, which loads libmpv and its ffmpeg libraries with symbol
relocation on the JS thread. Once per process, but the page's JS is blocked for ~120 ms during load (R44 already
saw `do_lookup_x` 1.4 % here). -> **QPROF1-d**.

**3.5 V8 compile on the JS thread (YT watch 9.3 %, YT results 10.6 %, bbc 24.8 %, github repo 38.7 % of the main
thread; `compile_cpu_ms` 290-350 per YouTube load, 59 bbc).** Every classic script is compiled synchronously in
`compile_script` (js_engine_v8.rs: `v8::Script::compile` without `CompileOptions`/cached data), after all scripts
were fetched. No code cache across loads of the same script (QY26 was the QuickJS version of this, parked), no
background (streaming) compile while the other scripts are still being fetched. -> **QPROF1-e** (measure-first).

**3.6 mpv's software scaler while a video plays (~10 % of the process's CPU in the window: `worker` thread
272-456 ticks of 3152-5375).** `vo=libmpv` with the `sw` render context: frames are converted and resized by zimg
in float (`depth_convert ... StoreF32`, `resize_line8_h_fp_avx2_xiter<f32, 6, 2>` = lanczos), off the main thread.
`MPV_OPTIONS` (media_net.rs) sets no `zimg-scaler`. Not a stall, but a steady ~5 % of a core for a 30 fps 640x360
video. -> **QPROF1-f**.

**3.7 Profiler artifact (tooling).** See section 0: `-F` per-thread frequency mode; `js-watchdog` is NOT a
hotspot (0.13-0.39 s CPU in 80 s by /proc). -> **QPROF1-g**.

Checked and not filed:
- **V8 GC** 6.6-7.6 % of the main thread on YouTube/bbc (V8 worker threads add more, but their sample share is
  inflated by the artifact above). JS heap 155-157 MB on YouTube watch, no nameable scratchy cause found in this
  data (no single allocation site); revisit after QPROF1-a/b remove their garbage (the frames prelude builds a fresh
  array per removal).
- **The YT observer_delivery 158-195 ms with 0 forced layouts** = scratchy's IO evaluation (`ioEval` without a layout
  because the rects were current) plus page MutationObserver callbacks; its non-page part is 3.1 + 3.2.
- **HTML parse, CSS parse, text shaping, paint, image decode, malloc**: each < 3.5 % of the main thread headless on
  every page (shaping 3.1 % on Wikipedia).
- **net delivery** (QOBS6): window `net` max 100-658 ms (p95 mostly < 10 ms); the large values sit behind App builds
  (main thread busy), i.e. QINC1j.

### 4. The live window

Per run (whole run; ms = main-thread CPU, inflated by load where load > 3; RSS from /proc at t = end - 2 s):

| run | load | App builds n / ms | JS forced layouts n / ms | page JS ms | paint ms | redraws/s | video frames rendered/s (whole) | input->present latency during the pointer phase: per-second p95 median / max | RSS MB (HWM) |
|---|---|---|---|---|---|---|---|---|---|
| YT watch w1 (first 15 s dropped: foreign pointer) | 14.5 | 163 / 7332 | 126 / 3521 | 10969 | 1179 | 3.1 | 22 | 64 / 93 | 496 (496) |
| YT watch w2 | 13.5 | 132 / 9537 | 143 / 6934 | 17908 | 2099 | 2.3 | 10.3 (24 s with none, below) | 117 / 148 | 649 (649) |
| YT watch w3 (perf) | 18.3 | 151 / 12437 | 155 / 8807 | 21775 | 2872 | 2.8 | 8.5 | 130 / 433 | 652 (652) |
| Wikipedia w1 | 2.9 | 47 / 4948 | 13 / 1909 | 2359 | 794 | 3.6 | - | 144 / 459 (hover); wheel 8-10 ms | 313 (335) |
| Wikipedia w2 | 2.3 | 40 / 5597 | 14 / 2722 | 3330 | 961 | 3.4 | - | 217 / 509 (hover) | 312 (333) |
| Wikipedia w3 (perf) | 18.9 | 31 / 8181 | 13 / 5090 | 6244 | 1154 | 2.5 | - | 322 / 1274 (hover) | 315 (336) |

(`lat_max_ms` of the whole run is not used: an input that needs no redraw stays pending until the next present, so
the pointer parked at 4 s shows "8 496 ms" on Wikipedia w1. The per-second p95 during the pointer phases is the
real number.)

**YouTube: the video loses frames while the pointer moves over the player (QM1k-mouse, measured).** Per second
(w1 / w2, video frames rendered): before the pointer phase 30 / 30; while the pointer moves over the player 9-18 /
5-8; after it 30 / -. In those seconds the main thread does 5-11 App builds (45-115 ms each, 0.3-0.6 s per second)
plus 290-430 ms of page JS (the player controls' hover handlers, R46/R50), and the video is painted only between
them. Window main-thread profile (w3): App build 34.4 %, forced layout 20.2 %, native bindings 15.5 % (of it
`style_of_chain` 8.5 %, `descendants` 4.6 %), page JS 7.4 %, paint 6.0 %, winit/wayland 5.5 %, iframe-window
prelude 4.3 %, V8 compile 2.7 %, GC 2.3 %. The frame drop is App builds + forced layouts on the main thread:
**covered by QINC1j/QINC1i**; QPROF1-a and QPROF1-b take ~9-13 % off the same thread.

**YouTube w2: no video frame for 24 s (37-61 s), then a second `<video>` node appears at 58 s and plays from
62 s.** No JS, no builds, no net results in that window; w1 and w3 did not show it. Cause not named from this data
(not layout, not a main-thread stall: the thread was idle). Not filed as a perf item; noted for the QM1k track
(see section 7, last bullet).

**Wikipedia:** scrolling is fine (wheel latency 8-10 ms, 1 redraw per tick). The hover sweep over the text gives
2-5 App builds per second at 100-165 ms each (page `mouseover` handlers write the DOM) and input->present p95
120-320 ms, and hovering a reference fires the Reference Tooltips gadget's `reallyShow` timer: **837-1075 ms with
6 forced layouts (822 ms)** (`getWidthOrHeight`, `offset`, `computeStyleTests`: jQuery). Window main thread (w3):
App build 49.0 %, forced layout 30.8 %, everything else < 4 % each. **All covered by QINC1i/QINC1j.**

### 5. Forced layout, App build and restyle (quantified; covered by QINC1)

| where | count / ms | QINC1 item |
|---|---|---|
| YT watch headless 60 s | 111-168 layouts, 2.5-7.4 s (calculateCanCollapse, getPlayerSize, numberOfGhostCommentsChanged, e6a; 22-32 IO) | QINC1i (+ f/g/h1) |
| YT results headless 30 s | 34-42, 0.84-1.12 s (24 IO) | QINC1i |
| Wikipedia headless 30 s | 4, 0.59-0.78 s (3 IO, max 260-314 ms) | QINC1i |
| bbc headless | 14, 0.39-0.40 s | QINC1i |
| YT watch window 80 s | App builds 132-163 = 7.3-12.4 s; forced 126-155 = 3.5-8.8 s; builds during pointer motion 5-11/s | QINC1j, QINC1i |
| Wikipedia window 45 s | App builds 31-47 = 4.9-8.2 s (100-165 ms each on hover); `reallyShow` 0.84-1.08 s (6 layouts) | QINC1j, QINC1i |
| IO evaluation's style reads (`style_of_chain` outside layout) | 10-12 % of the main thread on YouTube | QPROF1-b now; QINC1p later |

### 6. github.com home: runaway memory (QOOM1)

`--perf-report --budget-ms 20000 --seconds 30 https://github.com/` (own wire, default UA):
- 11:43:47 (batch run gh-1, load 10.6, no cap yet): stderr stopped at 11:44:35, the run never ended (a 30 s run) and
  no JSON was written; the process (pid 787397) was OOM-killed at 13:39 at 15.4 GB anon RSS (total-vm 44 GB) and took
  the whole session down. It was this run: the batch was still waiting on it.
- 14:08 (profiling binary, capped, RSS sampled every ~2-4 s): 6 MB at 0 s, 68 MB at 2 s, 689 MB at 4 s, 1.26 GB at 7 s,
  1.89 GB at 11 s, 2.89 GB at 15 s, 3.90 GB at 21 s, 5.38 GB at 29 s, then killed by the 6 GB cap (exit 137).
  ~185 MB/s, main thread at ~100 % CPU (utime 18.6 s at 29 s).
- 14:12 (same, perf attached at 4 s for 10 s): 756 MB at 4 s, 3.89 GB at 19 s, 5.38 GB at 23 s, killed. Busy threads:
  main 44.8 % of samples, `dav1d-worker` 16.7 %, `av:hevc:df*` ~10 % together, mpv `core` 5.5 %: the page's hero
  videos (AV1 and HEVC) are being decoded headless while memory grows. (The coordinator's note: the kill was
  triggered on `dav1d-worker`.)
- Reproduces 2 of 2 (3 of 3 counting the 13:39 kill). github.com/Loupax/vibe-it (no video) is normal: 65 MB.
- Not chased further (QOOM1 has its own agent); the remaining github.com home runs were skipped.

### 7. Proposed items (for the coordinator to file; PERFORMANCE QUEUE step 7)

- **QPROF1-a [S]** (P2, Sonnet; area jsdom prelude (js_dom.rs QPT1b/QPT1d frames block) + net.rs; coordinate with
  QPT1-D2 if it reworks the same block) Make the iframe-window bookkeeping proportional to iframe changes, not to
  DOM removals. Today every `__d_remove`/`__d_sethtml`/`__d_settext` calls `sweepFrames()` -> `syncWin()` ->
  `frameList()` = a full-document `getElementsByTagName('iframe')` plus, per iframe, `frameSrcOk` ->
  `__d_iframe_page` -> `find_base_url` = a full-document `<base>` walk (R62 section 3.1: 12.0 % of YouTube watch's
  main thread, 17.6 % of YouTube results'). Fix: (1) skip the sweep/sync when the removed or replaced subtree holds no
  `<iframe>` (one `__d_query(id, 'iframe')` on the subtree before the native removal, or a per-document live iframe
  count kept by the insert/remove natives) and when no frame exists; (2) cache the document base URL (invalidate on
  `<base>` insert/remove/href write, i.e. the same hook) instead of `find_base_href` per call; (3) make `frameSrcOk`
  not call `__d_iframe_page` for `about:blank`/empty/`data:` (already short-circuited) and memoise the pre-fetched
  page lookup per element+src. Gate: `tools/profile.sh` on YT watch 60 s and YT results 30 s: `sweepFrames|syncWin|
  frameList|frameSrcOk` < 1 % of the main thread, `find_base_href` and `descendants` self < 0.5 %; `--perf-report`
  raw CPU on YT results not higher. Tests: removing a non-iframe node does not rebuild the window list (test-only
  counter); removing an iframe still drops its window and `window.length` follows; a `<base href>` added later
  changes `iframe.src` resolution; QPT1d tests unchanged.
- **QPROF1-b [S]** (P2, Sonnet; area jsdom prelude (IO block: `gRect`, `ioShown`, `ioEval0`, `ioPad`) + js_dom.rs
  natives) Stop IntersectionObserver evaluation from cascading ancestor chains it does not need (R62 section 3.2:
  `style_of_chain` 10-12 % of the YouTube main thread from `ioShown`/`ioEval0`/`ioPad`). (1) `gRect`: call
  `__d_rect`/`__d_rect_r` first and treat `[]` as hidden; call `ioShown` only where a rect cannot answer (detached
  check stays, it is cheap). (2) `ioEval0`'s clip walk: get each ancestor's overflow and border widths from the laid
  box (one native returning [overflow-x, overflow-y, border l/t/r/b] from the GeomCache's box, no cascade) instead of
  three `__d_cs`/`gPx` calls. Must give identical IO entries. Gate: profile.sh YT results 30 s: `style_of_chain`
  under `ioShown|ioEval0|ioPad` < 2 % of the main thread; the R38/QY57 IO tests and the IO WPT-ish fixtures unchanged.
  Tests: display:none ancestor -> not intersecting (no cascade call, test-only counter); overflow:hidden ancestor
  still clips; border widths still inset the root. (QINC1p would also cover the style part later; this is cheaper
  now and independent of the engine flip.)
- **QPROF1-c [S]** (P3, Sonnet; area globals_prelude.js MessageChannel) One `message` event per task. `flush(p)`
  dispatches the whole queue, including messages posted while dispatching, in one timer callback, so a
  MessageChannel-yielding scheduler (React) never yields: bbc.com/news has one 375-389 ms task (R62 section 3.3).
  Fix: dispatch only the message at the head, then schedule the next task if more are queued (or: only the messages
  present when the flush started, each in its own task). Gate: bbc.com/news headless `longest` has no `globals:114`
  callback >= 100 ms (raw), total raw CPU within noise. Tests: two messages posted in a row arrive in two tasks (a
  `setTimeout(0)` queued between them runs in between); a message posted from a handler runs in a later task;
  `start()`/`close()` semantics unchanged.
- **QPROF1-d [S]** (P3, Sonnet; area media_mpv.rs (library load) + app/perf startup) Load libmpv off the JS thread.
  The first media element pays `dlopen` + relocation of libmpv/ffmpeg on the JS thread: Wikipedia Rust has a
  118-131 ms `media:98:54` timer and `_dl_*` 10.5 % of the headless main thread (R62 section 3.4). Fix: start the
  load on a background thread at startup when media is enabled (or as soon as the parser sees `<video>`/`<audio>`),
  and have `MpvBackend::ensure` wait for it only if it is not done. Gate: Wikipedia headless: no callback under
  `media:` >= 30 ms; `do_lookup_x` self < 1 % of the main thread. Tests: two players created concurrently share one
  load; without libmpv the placeholder still appears (the load fails once, quietly).
- **QPROF1-e [M]** (P3, Opus measure then Sonnet; area js_engine_v8.rs (`compile_script`) + js.rs script loading)
  V8 compile off the critical path. Compile is 9-11 % of the YouTube main thread, 25 % on bbc, `compile_cpu_ms`
  290-350 ms per YouTube load (R62 section 3.5). Two independent parts, measure each first: (1) a session code
  cache: `CompileOptions::ConsumeCodeCache` with `v8::script_compiler::CachedData` kept in memory per (final URL,
  content hash), produced after first run (`create_code_cache`), capped (e.g. 64 MB LRU); no disk (QF-D7); helps
  reloads/revisits and the iframe realms. (2) background compile of classic scripts while the others are still
  being fetched (V8 `ScriptStreamingTask` / `StartStreaming`), so the JS thread only finalises. Gate: YT results
  second navigation in one process (`--perf-report URL URL`): compile_cpu_ms of the second load <= 40 % of the
  first; first load's JS-thread compile time lower with (2). Tests: cached and uncached runs give the same results;
  a changed script body misses the cache.
- **QPROF1-f [S]** (P3, Sonnet; area media_net.rs `MPV_OPTIONS` / media_mpv.rs render size) Cheaper software
  scaling for video. mpv's `sw` render path converts and lanczos-resizes every frame in float with zimg: ~10 % of
  the process CPU while a 640x360 video plays (R62 section 3.6). Try `zimg-scaler=bilinear` (and `zimg-fast=yes`
  explicitly), and render at the video's own size when the box is not larger (scale in paint instead). Gate: window
  YT watch 30 s of playback: mpv `worker` thread CPU (/proc) -50 %, no visible quality loss on a grim screenshot
  at 100 % zoom; frames rendered/s unchanged. Tests: the option list contains the scaler setting; render size
  follows the rule.
- **QPROF1-g [S]** (P4, Haiku/Sonnet; area tools/profile.sh, tools/perf-fold.py, PERF.md) Profile shares that mean
  CPU: `-F 999` is per-thread frequency mode, so near-idle and short-lived threads are oversampled (js-watchdog
  shows 5-30 % of samples for < 1 % of the CPU; 273 short fetch threads on bbc). Use a fixed period
  (`-c 3000000` ~ 1 kHz at 3 GHz, or `-F` plus a per-thread CPU table from /proc in summary.txt), fold by tid (main
  thread separately: stacks rooted at `_start`), and say in PERF.md which numbers compare across threads. Gate: on a
  YT watch profile the summary's per-thread shares match `/proc/<pid>/task/*/stat` within 5 points.
- **Covered by QINC1 (no new item):** all forced layouts (QINC1i; YT `calculateCanCollapse`/`getPlayerSize`/`e6a`,
  Wikipedia IO and `reallyShow` 0.84-1.08 s, bbc 14 layouts), all App builds (QINC1j; YouTube 5-11 per second while
  the pointer moves over the player = the video frame drop to 5-18 fps, Wikipedia 100-165 ms per hover build), the
  style part of IO evaluation later (QINC1p; QPROF1-b now).
- **Not filed, for the QM1k track:** YT watch w2 had 24 s with no video frame (37-61 s) while the main thread was
  idle, then a second `<video>` node started at 58 s (section 4). Seen in 1 of 3 runs; the coordinator may want to add
  it to QM1k-mouse / QM1k1n4's notes rather than a new item until it reproduces.
- **QOOM1 (already filed):** github.com home grows ~185 MB/s to the cap with AV1/HEVC decoders busy (section 6).


## R61: QYSCR1: scrolling did not work on YouTube (user 2026-11-20). Three causes, all fixed: (1) `<body>`'s `overflow` was never propagated to the viewport in layout (only the stacking-boundary check and, since QY57r, IntersectionObserver knew the rule), so YouTube's `body{overflow-y:scroll}` made the 0 px tall body a scroll region and stopped the page-height walk at it; (2) the page height (`laid.h`, the top-level scroll limit) ignored absolutely positioned boxes, and YouTube's whole page lives in the absolute `ytd-app`, so the page was one viewport tall; (3) Space, PageDown/PageUp and the arrow keys were not bound at all. Fix: one propagation rule in the layout tree, the page height includes absolute boxes, a wheel tick a region cannot use goes to the page, and the usual scroll keys (Opus, 2026-11-20) — FIXED

**Reproduced** (baseline = installed main at 094b4b5 code, own nested dwl 960x316/489, own wire): on YouTube results
(`/results?search_query=rust`) and watch (`G1AeBgalFZk`) neither the wheel (5-10 notches over the page) nor Space,
PageDown or Down moved the page at all (grim before/after identical). Local fixtures:

| fixture | wheel before | wheel after |
|---|---|---|
| plain page, 60 rows | scrolls | scrolls |
| `body{overflow-y:scroll}`, rows in flow | **no** | scrolls |
| `#app{position:absolute;top:0;left:0;right:0}` holding the rows, body visible | **no** | scrolls |
| YouTube's shape: both of the above | **no** | scrolls |
| `html,body{height:100%} body{overflow:auto}` | scrolls (body region) | scrolls (viewport) |

**Causes.**
1. CSS Overflow 3 (overflow viewport propagation): the root's `overflow` applies to the viewport; when it is `visible`, the
   `<body>`'s does instead, and the element it came from then uses `visible`. scratchy kept the body's `overflow` on the body
   box. `layout::is_stacking_boundary` had a special case (`VIEWPORT_ROOT_NODE`) and js_dom.rs `ioEval0` another (QY57r), but
   `measure` still gave `<body>` a `ScrollInfo` (client height 0 on YouTube) and `in_flow_bottom` stopped at the body because
   its overflow was not `visible`. With rows in flow the body was as tall as its content, so `laid.h` was right, but
   `hit_test_scroll_region` returned the body region (max scroll 0) for every point, so the wheel went to a region that cannot
   move and never to the page.
2. `in_flow_bottom` (what `measure_document` grows the root's `h` to, and `redraw` clamps `scroll_offset` against) only walked
   in-flow children. YouTube's `ytd-app` is absolutely positioned (body 956x0, `ytd-search` 884x7687 in R56), so the maximum
   scroll was 0.
3. `resolve_binding` had only vim keys (`j`/`k`, `Ctrl-d`/`u`, `G`, `gg`, Home/End).

**Fix (one rule).**
- images.rs `propagate_viewport_overflow` (next to the canvas-background propagation, QW6) runs once per layout-tree build, so
  App frames and the JS thread's forced layouts (GeomCache) both get it: `<html>`'s non-visible `overflow` becomes `visible`
  on `<html>`; otherwise `<body>`'s does on `<body>`. Layout/paint containment (`contain: layout|paint|strict|content`,
  newly parsed as `ComputedStyle::contain_lp`) on `<html>` or `<body>` stops the body propagation (WPT
  overflow-body-propagation-011). The viewport is the page's own top-level scroll, so nothing else is needed: the body gets no
  `ScrollInfo`, clips nothing, is no stacking boundary (`VIEWPORT_ROOT_NODE` removed) and does not stop the page-height walk.
  `getComputedStyle` still reports the specified value.
- js_dom.rs `gViewportBody()` is the same rule for script: `gScrollable` (element `scrollTop`, `scrollIntoView` ancestors)
  and `ioEval0`'s clip exemption (QY57r) both use it, so `body.scrollTop = 50` does nothing and `scrollIntoView` scrolls the
  window.
- layout.rs `in_flow_bottom` also walks `positioned` boxes that are not `fixed` (scrollable overflow includes absolute boxes;
  fixed ones never scroll). A clipping box still contributes only its own edge.
- app.rs `region_takes_wheel`: a wheel tick that the region under the cursor cannot use (nothing to scroll, or already at its
  end in that direction) scrolls the page instead (one-level scroll chaining).
- app.rs `resolve_binding`: Space / PageDown = `Action::PageDown` (viewport less 40 px), Shift+Space / PageUp = `PageUp`,
  ArrowDown / ArrowUp = one 40 px line. Page `keydown` handlers, focused text controls/selects and focused buttons and
  checkboxes still see the keys first (unchanged order in `WindowEvent::KeyboardInput`).

**Tests** (interaction_tests.rs, `qyscr1_*`, 8): YouTube-shaped document is >= 1500 px tall with no scroll region and no
wheel target; in-flow body `overflow-y:scroll` is no region; `html{overflow:hidden}` leaves `<body>`'s `overflow:auto` on
the body; `contain:paint` on `<html>` stops the propagation; fixed boxes add no page height, an absolute box at `top:2000px`
does; `region_takes_wheel` chains at both ends and on a region that fits; key bindings and `page_step`; script:
`getComputedStyle(body).overflowY` stays `scroll`, `body.scrollTop = 50` reads 0, `scrollIntoView` scrolls the window.
Full suite 1398 pass. WPT: no pass->fail; the other 12 overflow-body-propagation reftests were added to the subset:
001, 005, 006 and 010 pass newly (fail on main), 002-004, 007-009, 011, 013 pass on both, 012 fails on both (body
`overflow:hidden` under `html{overflow:hidden}` with a 0x0 body: needs the body's clip of its border-box overflow), 014-016
need script (skipped, not added). 333 pass / 293 fail. Clippy 77 (local; no new warning).

**Real check** (own nested dwl + tools/inject, release build of this branch; grim shots viewed): YouTube results: wheel 5
notches scrolls ~200 px, PageDown one page, Down 40 px. Watch G1AeBgalFZk: wheel 10 notches over the player reaches the
title/channel row and "345 Kommentare", 2x PageDown reaches the comment list, wheel over the related column scrolls the page
further (related videos move with it). Home: logged out with history off YouTube shows one short card, nothing to scroll
(same in other browsers). Wikipedia (Rust article) and Hacker News scroll by wheel, PageDown/Space and Down as before.

**Left out (proposed items).**
- QYSCR1a [S]: on YouTube results the "Alle" filter chip (`button.ytChipShapeButtonReset`) has focus after load without any
  click, so Space presses that button (Q30e, correct for a focused button) instead of scrolling; PageDown/arrows work. Find
  out who focuses it (YouTube's `focus()` call vs scratchy's autofocus/focus rules; Chrome scrolls on Space there).
  **QYSCR1a DONE (Sonnet, 2026-11-20): cause found and fixed.** Scratchy never focuses by itself. The only `focus()` call is
  YouTube's kevlar `loadPage_` -> `Ph(parentComponent)` (focus the first DFS element that is focusable and has
  `offsetParent != null`, else the component itself). Chrome 156 (headless, CDP, `HTMLElement.prototype.focus` logged)
  focuses the masthead `yt-icon-button#guide-button > button`; scratchy's `offsetParent` returned null for every
  descendant of a `position:fixed` box (the rect's "fixed" flag was tested, not the element's own position), skipped the
  fixed masthead and found the first chip. Fix: js_dom.rs `gOffParent` returns null only for a fixed box itself (CSSOM
  View). After the fix scratchy focuses the guide button like Chrome, and Space then toggles the guide (as it does in
  headless Chrome: scrollY stays 0, scrollHeight changes 6839 -> 907), so Space scrolling right after load needs a click or
  Tab away first in BOTH browsers; "Chrome scrolls on Space there" does not hold for a fresh load with focus on the guide
  button. Seen in grim after Space: the toggled-guide layout glitches (chip bar gone, first thumbnail offset), not
  investigated here. Gate: full suite 1408 pass, clippy 77 (baseline file 74, local limit 77), WPT no change (333/293).
- QYSCR1b [S]: DONE (this PR, QYSCR1b+c): a propagated viewport `overflow: hidden` blocks wheel and scroll keys; `scrollTo`/`scrollBy`/`scrollIntoView` and `scrollTop` on `overflow: hidden` boxes still scroll (`ScrollInfo::user`, `Style::viewport_hidden`).
- QYSCR1c [S]: DONE (same PR): `paint::hit_test_scroll_chain`; a tick goes to the first region under the cursor that can move, then the page. Keys always target the viewport (no focused scroll region in scratchy). Absolutely positioned regions are still not wheel targets.
- QYSCR1d [S]: WPT overflow-body-propagation-012 (body clips its own overflow when `html{overflow:hidden}` takes the viewport).

## R60: QINC1 design: one incremental style + layout engine for App frames AND script forced layouts. Persistent per-element styles with a match cache, Blink/Stylo-style invalidation sets and early cut-off; a box tree kept between layouts and rebuilt per container; the last laid tree reused as the fragment cache (a clean box is moved, not re-measured; a dirty box is re-measured at its cached constraints and the change goes up only while the parent-visible result changes). Measured first (throwaway probe): between two consecutive forced layouts the median page restyles 3-33 elements of 261-1 128 styled ones (1-5 %), and touches 1-6 % of the boxes; today every one of them redoes 100 %. Projected: YouTube results `getPlayerSize` 54-80 ms -> ~2-5 ms (preview job 685-1 137 ms -> ~150-220 ms), YouTube watch / Wikipedia App builds 38-90 ms -> ~2-6 ms each, and a frame after a forced layout costs ~0. Size: XL (14 items, 4 of them L), landed in shadow mode first, then behind `SCRATCHY_INCR=1`, then flipped, then the special cases deleted. RECOMMENDED; supersedes R59/QY54b3a-f (Opus, 2026-11-20, design only, docs only) — OPEN: proposed QINC1a-QINC1m (section 15), open questions (section 16); QINC1k gate (2026-11-21): NO-GO for the flip until QINC1k1 + QINC1k2 (section 15, QINC1k entry); QINC1k1 done 2026-11-21 (y translation invariance); QINC1k3 done 2026-11-21 (Wikipedia App builds 13-14 % of flag off)

Inputs: R43 sections 4/8 (phase split, 84 % of styles reusable), R47 (hover restyles), R50 (QYTC1a/b/d), R55 (App builds per run:
Wikipedia 104 / 9.4 s, YouTube watch 268 / 10.1 s), R57 (main-thread stalls), R58 (results `getPlayerSize`: full layout,
~11 us/element, style ~80 %), R59 (subtree layout, translation invariance to 0.00024 px). Code read at 5888220: style.rs
(`resolve_style_for`, `matching_indices_for`, `MATCH_MEMO`, `HoverKeys`, `RuleDerived`, `stash_last_match`), images.rs
(`build_layout_tree_with_resources`, `build_box_for_element_body`, `collect_real_nodes`, `precompute_generated`), layout.rs
(`LayoutBox`, `Laid`, `measure_inner_body`, `resolve_positioned`, `hoist_negative_z`, `compute_ink`, `shift_laid_tree`), js_geom.rs
(`GeomCache::refresh`, `walk`, `style_write_in_place`, `ClassDeps`, `HiddenSnap`), js_dom.rs (25 `geom_bump*` call sites), app.rs
(`redraw`, `spawn_shaping_worker`, `apply_hover_cheap`), hover_patch.rs, rules_cache.rs, anim_eval.rs, js_style.rs.

### 0. Measured: how much really changes between two layouts

Throwaway probe (scratchpad export of main 5888220, never committed): every `resolve_style_for` records its (node, slot) ->
`ComputedStyle` (with `tree_order` zeroed); after every `GeomCache::refresh` the map is compared with the previous forced
layout's, plus a per-element DOM fingerprint (tag, attributes, child ids, text of text children) and the rect maps. Release
build, own wire on a scratchpad socket, headless `--perf-report` (default budget), one run per page, under
`flock /tmp/scratchy-perf.lock`, load 17.7 -> 6.5 (other agents building; the ms are high, the counts are not affected).
"styled" = (element, pseudo) pairs the build resolved (elements in `display: none` subtrees are never styled);
"restyled" = changed + new pairs; "dirty" = elements whose own attributes, child list or child text changed;
"anc" = distinct ancestors of the dirty and changed elements (the path a relayout walks); "rects touched" = resized + moved +
new rects / all rects.

| page (forced layouts) | ms per layout, median (max) | styled, median | restyled per layout: median / p90 / max | restyled share: median / p90 | dirty: median / p90 | anc: median / p90 | rects touched: median / p90 | layouts that changed no style |
|---|---|---|---|---|---|---|---|---|
| YouTube results 30 s (37) | 34.2 (77.5) | 1 128 (up to 2 871 of 6 924 elements) | 33 / 205 / 449 | 4.5 % / 23 % | 129 / 551 | 104 / 372 | 5.9 % / 35 % | 9 |
| YouTube watch 30 s (72) | 18.9 (45.1) | 261 (up to 886 of 3 101) | 3 / 61 / 288 | 1.2 % / 17 % | 5 / 57 | 14 / 63 | 0.9 % / 23 % | 8 |
| Wikipedia Rust 20 s (3) | 142.6 (146.3) | 10 071 (9 971 elements) | 6 / 75 / 75 | 0.1 % / 0.7 % | 22 / 69 | 25 / 55 | 5.8 % / 6.0 % | 0 |

- The big shares (p90 17-23 %) are YouTube appending a batch of results/recommendations (new = 100-443 elements), or a
  class flip on an ancestor that changes ~60-270 inherited styles (e.g. 270 changed after one `dirty` element). Both are real
  work an incremental engine also has to do; everything else is a few elements.
- Changed styles OUTSIDE every dirty subtree: 1 + 2 + 0 in 112 layouts. Sibling/`:has`-like reach is rare here (headless,
  no pointer: hover was R47's job: 1-2 elements on Wikipedia, 0-6 plus up to 27 descendants on YouTube).
- Phase totals of these runs (QOBS3): YouTube results 1 516 ms = precompute 329 + build_children 708 + measure 305 + walks
  31; watch 1 558 = 446 + 781 + 100 + 21; Wikipedia 757 = 171 + 308 + 216 + 28. So 70-80 % is style, all of it redone for
  1-5 % of real change. R43 section 8 saw the same from the other side (84 % of fingerprints unchanged, 2026-11-19); it judged
  incremental style "not worth an L" because forced layouts were then 3 % of YouTube's CPU. Since then R55 (App builds:
  9.4-10.1 s per run) and R58 (54-80 ms per results read, 11-13 per promise job) made the full rebuild the cost again.

App builds were not probed (needs the nested window). They are fed by the same DOM writes (R55: 143-220 of 213-234 builds per
YouTube run come from script writes) plus hover/focus (R47), so the forced-layout distribution above is the model for them.

### 1. Today: one full pipeline, run twice, and the special cases around it

Scripts run on the MAIN thread (the V8 isolate lives on the winit thread; only the watchdog and snapshot threads are
separate). So the "JS thread" of R58/R59 and the App are the same thread, on the same `Rc` DOM. Two consumers run the same
from-scratch pipeline (`build_layout_tree_with_resources`: css collect, rule index, `precompute_generated` (all matching),
`build_children` (cascade + boxes) -> `measure_document`):
- App frame (`App::redraw`, `layout_dirty`): build on main, measure on the shaping worker (progressive, `hold_old_frame`),
  result installed in `App::cached`. With the window's image cache.
- Script forced layout (`GeomCache::refresh`, key `(geom_gen, vw, vh, css_ver)`): build + measure on main, then `walk`
  (rects + QYTC1a skeleton) and `walk_glyphs`. WITHOUT images (an empty cache).
A forced layout and the App frame after it do the same work twice; nothing is kept between layouts except caches that
work around that:

| special case | what it does | in the engine |
|---|---|---|
| QYTC1a `style_write_in_place` + `Skel` (R50) | shift an abs box's laid subtree for `left`/`top`/translate writes, no bump | general: a style write restyles one element; an offset-only diff on an abs box re-places only that box (cache hit + shift). DELETED |
| QYTC1d `ClassDeps` class probes (+`PROBE_BUDGET`) | class write that changes no rule match: no bump | invalidation sets (section 5) + early cut-off. `ClassDeps` is the seed of the class part. DELETED |
| QY53l2 `HiddenSnap` | write inside a `display: none` subtree: no bump | elements without a box have no style entry; a write there invalidates nothing rendered. DELETED |
| QY53o1 `ATTR_SNAP` / `AttrReaders` | write to an attribute no selector reads: no bump | `AttrReaders` becomes the attribute part of the invalidation map; the snapshot goes. DELETED (the reader table stays) |
| QY53m no-op write skip | a write that changes nothing marks nothing | stays as is (the DOM layer only reports real changes) |
| QY53n2/n3 hover paint/offset patch (hover_patch.rs, R55, #424, #426) | patch `App::cached` for paint-only / offset-only hover rules | hover = a state invalidation; paint-only diff = style patched into the kept box and fragment, no relayout. DELETED |
| `MATCH_MEMO` (per build), QY54b2 note | each (element, pseudo) matched once per build | per-element match cache that survives builds (R43 item 7, QY53f2/f3, QY54b2b). The per-build memo goes |
| QY53n1c kept memo (`KEEP_MATCH`, `LAST_MATCH`, `with_kept_build`, `hover_match_diff`) | the App build's matches kept for the hover patch | subsumed by the match cache. DELETED |
| `HoverKeys` | which elements a hover change can restyle | becomes the state part of the invalidation map |
| `RuleDerived` / `rules_cache` generation / QY53f1 splice | rule list + index cached per generation; a `<style>` edit re-parses one piece | stays; a new generation = "re-match everything, keep what compares equal" (section 6) |
| QY54b1 link CSS (#432) | script-inserted sheets applied, one `css_ver` bump | stays; the bump is a stylesheet invalidation |
| R59 / QY54b3a-f (on hold) | dirty points + stash + subtree layout of one abs box for a forced read | superseded: section 8's bottom-up relayout is stage 2 for every box, not only abs boxes with definite size, and also for App frames. R59's translation-invariance prototype, its generated-content and animation-pass analysis are reused |
| QY53e1/e2 IO (`RENDERED`, render stamps) | IntersectionObserver reads the App's layout to avoid its own forced layout | one layout: IO reads the engine's current tree (QY53e2b moot) |

### 2. What real browsers do, and what we take

- **Blink:** `Node` flags `NeedsStyleRecalc` (local / subtree) + `ChildNeedsStyleRecalc` path bits, set by `StyleEngine`
  from `RuleFeatureSet` invalidation sets (class/id/attribute/pseudo-class -> self, descendant set filtered by the rightmost
  compound's features, sibling sets; `:nth-*` and `:has` have their own). Recalc walks the path bits; a recomputed style that is
  equal to the old one stops (`StyleRecalcChange` decides whether children inherit-only recalc, full recalc or nothing).
  `ComputedStyle::VisualInvalidationDiff` classifies a change as repaint / layout / reattach. Layout: `NeedsLayout` +
  `ChildNeedsLayout`, relayout boundaries (fixed size + overflow clip + not a flex/grid item), LayoutNG caches each
  `LayoutResult` per box keyed by the constraint space and reuses it when the box is clean and the space matches.
- **Gecko/Stylo:** restyle hints from the invalidation map, a traversal over dirty-descendant bits, style sharing; frames
  are reconstructed per container on "reframe" changes; `NS_FRAME_IS_DIRTY` / `HAS_DIRTY_CHILDREN` for reflow.
- **WebKit (legacy):** persistent mutable render objects that hold their own geometry; `setNeedsLayout` marks up to a
  relayout root; layout walks only marked objects.
Taken here: Blink's invalidation sets and recalc-change propagation; Blink's style diff classes; Gecko's per-container box
reconstruction; WebKit's "the last laid tree is the persistent geometry" (scratchy's `Laid` is already an owned, mutable
tree in absolute coordinates, so it is the natural cache); LayoutNG's constraint-space keys and its "stop when the result the
parent sees is unchanged". Not taken (yet): relative-coordinate shared fragments (section 14, QINC1n).

### 3. Decision: ONE engine, on the main thread

One `Engine` (new `src/incr*.rs`) per page document, owned by the main thread (a thread-local next to the DOM, like
`INTERACTION`), used by BOTH the App frame and `GeomCache` (and the IntersectionObserver recheck). Reasons:
- It can: DOM, V8, App build and forced layout already share the thread; no locks, no copies of the DOM.
- A forced layout's result IS the next frame. Today a script write is laid out twice (forced read, then the App build);
  with one engine the frame after a forced read costs ~0 when nothing changed since.
- Two engines would need two sets of dirty bits fed by the same writes and two caches to keep exact; the special cases of
  section 1 exist because of that split.
Consequences:
- **Forced layouts see decoded image sizes** (they use the window's image cache). Real browsers do; today JS reads 0x0 /
  alt-text sizes for images the window shows at full size. This changes script-visible geometry (open question 2).
- **Incremental work runs on main, synchronously and complete** (no deadline, as forced layouts already do). The shaping
  worker stays for "big" updates (first layout of a page, viewport width change, a new rule-list generation, restyled share
  over a threshold, kill switch): today's progressive path, whose result the engine adopts (section 9).
- Headless `--perf-report` (no App) gets the engine through `GeomCache`, so the gates can be measured headless too.

### 4. Data model

**4.1 Element data (side table).** `HashMap<node_id, ElementData>` with a `WeakHandle` per entry (node ids are `Rc`
addresses; a freed node's address can be reused: the weak check catches it, as `GeomCache` and the animation table do).
Only elements that get a box (or are `display: contents`) have an entry; an element in a `display: none` subtree has none.
Per element and slot (element, `::before`, `::after`, `::marker`):
- `matched: Rc<[(u64, u32, bool)]>` (the match cache: what `matching_indices_for` returns), with the rules generation;
- `style: ComputedStyle` (base + animated value; `tree_order` see 4.5) and `anim: bool`;
- the ambient inputs the builder threads down (`DecorationValue`, `href`, inline `bg`, `valign`): compared like inherited
  values;
- the element's generated-content entries (`GENERATED_TEXT`/`ELEMENT_CONTENT` today) and its counter state on exit when the
  rule list uses counters/quotes;
- flags: `RESTYLE` (re-match), `RECASCADE` (inputs changed, matches valid), `DESC_DIRTY` (path bit: some descendant has
  a flag), `BOX` (this element's box content must be rebuilt), plus `box_owner` (the element whose `LayoutBox` holds this
  element's content: itself for block-level/flex/grid/table/abs/inline-block boxes, the nearest such ancestor for inline
  elements and `display: contents`).
Memory: ~1.4 KB per styled element (`ComputedStyle` is 1 304 B): ~4 MB on YouTube results, ~14 MB on Wikipedia (open
question 5).

**4.2 Box tree (kept).** The engine keeps the `LayoutBox` tree it built last (today it is dropped after measuring) and
updates it in place. An element-owned box is identified by `style.node`. Anonymous boxes (inline runs, anonymous table
wrappers, flex anonymous items, the root wrapper) and everything an inline element contributes (`InlineItem` spans with
copied `SpanStyle`s, spacers) belong to their container and are rebuilt with it. `InlineItem::Embedded` (inline-block /
inline-flex) is element-owned and reusable. Each element-owned box gets `ldirty: Cell<bool>` (its subtree changed since
the last layout) and `bver: u64` (bumped on every rebuild).

**4.3 The last laid tree is the fragment cache.** Every element `Laid` records how it was measured:
`ck = { origin x, y; width; containing_height; auto_margin_box_override; forced_border_w; floats-in: empty | not; vw, vh,
root epoch }` and `cacheable` (complete shaping; one `Laid` for its node; see 8.2 for out-of-flow and hoisted descendants).
`measure` is translation-invariant in (x, y) (R59 section 7: 0.00024 px at y ~2 089), so a clean box measured with the same
`ck` minus the origin equals its old `Laid` shifted by the origin delta.

**4.4 Stable paragraph ids.** `LaidContent::Text(id)` / `MixedPositioned::Word { id }` index a per-layout `ShapeCache`
whose ids restart at 0 every layout (`next_id`). A reused `Laid` keeps its ids, so ids become engine-unique (a counter
that never restarts; the worker path gets a base) and the `ShapeCache` persists, swept of ids no live `Laid` uses.

**QINC1b DONE (Sonnet, 2026-11-20):** `App::next_para_id` and `GeomCache::next_para_id` never restart (`ShapingRequest::id_base`; `ShapeResult` returns the worker's next id; the worker takes max(base, its own counter); every pass of one build reuses the same ids so a pass resumes the one before). The shaping worker's `ShapeCache` and `GeomCache::shape_cache` persist; `layout::sweep_shape_cache` drops ids no live `Laid` names when the cache exceeds 2x the live count (worker: after a finished layout and at the start of every request, so a storm of superseded builds cannot grow it; a new document clears it; a panic clears it; GeomCache: no `Laid` outlives a forced layout, so it is emptied). Readers unchanged.

**QINC1c DONE (Sonnet, 2026-11-20):** `finish_z` runs at the end of every `measure_inner` during a `measure_document` (`Z_ACTIVE`; a bare `measure` still returns the unhoisted tree, which existing tests read): a stacking boundary absorbs the z-index entries its children bubbled (`Laid::zb.bubble`), any other box moves its relative/sticky in-flow children (and, if positioned, its out-of-flow ones) out and passes everything up; the root absorbs last, before `in_flow_bottom`. Findings for QINC1f-h: (1) the old pass never descended into a STATIC inline-block (only `relative`/`sticky` ones were hoisted), so nothing below one is hoisted; reproduced with `Z_SKIP` around its measure (probably a bug: z-indexed boxes inside an inline-block never reach their stacking context; fixing it changes trees, so it is a separate item); (2) moving an in-flow child out at the box's own finish would change what its PARENT reads (first baseline, last line extent of an inline run, collapse-through of an empty block), so the box records the pre-move values in `ZInfo` (`fb`, `last_line`, `flow_emptied`, shifted with the box); (3) table rows built by `measure_row_with_widths` bypass `measure_inner`: the table finishes them. `stack_key` ties use `style.tree_order` renumbered in document order by `images::assign_document_order` (one index per element/pseudo; anonymous boxes keep theirs). Verified: every `measure_document` in the test suite (1443 tests) is laid twice, bubble vs the old pass kept under `cfg(test)`, `cmp_laid` 0 differences, plus a check that ties order the same under the old resolve counter; 2500 extra fuzz cases (seed 7000) 0 differences. NOTE for anything that counts work in `cargo test`: under `cfg(test)` `measure_document` lays the page TWICE (the second with the old pass), so shaping, `SHAPE_MEMO` and `incr.log` counts are doubled; layouts that finished incomplete (deadline) and trees deeper than 120 levels are not compared. The bubble holds `Box<Laid>` (passing up moves pointers; the entry is unboxed once, by the owner), and a `ZInfo` that carries nothing is dropped (`zb = None`); a reused `Laid` with `zb.fb` set must keep it (`first_baseline` returns it).

**4.5 Paint order ties.** `stack_key` breaks z-index ties by `style.tree_order`, a global counter taken at resolve time.
A partially restyled tree would order new elements after old ones. Ties are taken from document order instead (an order
index the engine renumbers on structural change, or a DOM-position comparison inside the sort; only ties need it).

### 5. Invalidation: what marks what

`InvalidationMap` per rule-list generation (new `src/invalidation.rs`; generalizes `ClassDeps`, `HoverKeys` and `AttrReaders`,
which already answer this question for one feature each). From every selector: features (class, id, attribute name, state
pseudo-class: `:hover :active :focus :focus-within :focus-visible :checked :disabled :enabled :target :placeholder-shown
:link/:visited :default :indeterminate :required :optional :read-only/-write`) and where they sit:
- in the rightmost compound -> **self**;
- left of a descendant/child combinator -> **descendants**, filtered by the rightmost compound's class/id/tag (Blink's
  descendant set; "all descendants" when the rightmost compound has no such key);
- left of `+` / `~` -> **siblings** (adjacent or all following; then self or descendants of them);
- inside `:not/:is/:where` -> treated as if outside (conservative); inside `:nth-child(of S)` -> structural;
- structural flags: `:nth-*`, `:first/last/only-child`, `:*-of-type`, `:empty`; `:has()` (scratchy does not parse it; if a
  future parser does, the guard is "any `:has` in the list -> every write invalidates its ancestors' `:has` subjects", i.e.
  Global until designed).
Sources -> marks (the `IncrLog`, filled where `geom_bump*` / `JsEngine.dirty` / `layout_dirty` are set today):

| source | mark |
|---|---|
| attribute set/remove (name, old, new) | `class`/`id`/other: the map's sets for the changed tokens; `style`: `RECASCADE` on the element (+ the map if a selector reads `[style]`) |
| child inserted / removed / moved (parent, prev/next sibling) | new subtree: `RESTYLE` all; parent: `BOX` on its box owner; structural selectors: siblings per flag (`:nth-child` -> all children, `+`/`~` -> following siblings); `:empty` -> parent; `<style>`/`<link>`/`<base>`/`<slot>`/`<meta>` in the subtree -> CSS/Global as today (`subtree_sensitive`) |
| text node data / `textContent` / `innerHTML` | `BOX` on the text's box owner (+ the child-list rules) |
| form state (checked, value, selection) | `:checked`/`:placeholder-shown`/... state sets; the control's `BOX` (its painted value is box content) |
| hover / active / focus / target change | old vs new chain: the state sets of every element entering or leaving the state (HoverKeys' job today) |
| rule list generation (style splice, link CSS, adopted sheets, js on/off) | v1: `RESTYLE` everything (re-match, early cut-off keeps boxes and layout for equal styles). Later: invalidate only by the added/removed rules' features (Blink RuleSet invalidation), QINC1q |
| viewport width | full restyle (media queries, vw units) and full relayout: worker path |
| viewport height | elements whose style read `vh` / the ICB height (`VIEWPORT_H_READ` per element instead of per build) |
| image / CSS image decoded, video intrinsic size, font | `BOX` on the elements using that `src` (an index built while building) |
| animation tick | `RECASCADE` on the elements with a running animation (the engine knows them) |
| counters / quotes changed by a restyle | the generated walk (6.4) marks the elements whose generated text changed |
| unknown (adopted sheets without node, a write the log cannot describe, log overflow) | Global = everything dirty (today's behaviour) |

A soundness test drives this table (QINC1d): for random DOM/CSS fixtures and every mutation kind, the predicted set must
contain every element whose match really changed (brute force: match all before and after).

### 6. Incremental restyle

**6.1 Traversal.** Top-down from the root along `DESC_DIRTY`, carrying the parent's `InheritedStyle` and ambient inputs, the
ancestor bloom filter (`ANCESTOR_BLOOM` already keeps a chain) and the rule index. At element E:
- `RESTYLE`: match (`matching_indices_for`), store the match, cascade;
- `RECASCADE`, or the parent's inherited output / custom properties / `root_font_size` / ambient changed: cascade from the
  stored match (no matching: the expensive half, ~1.4 us x 3 per element, R58 QY54b2 note);
- neither: skip E (its style stands), visit only its dirty children.
**6.2 Early cut-off.** New style == stored style (excluding `tree_order`, `node`) -> E's children get nothing from E. If only
non-inherited fields changed, children are not re-cascaded either. Inherited fields, custom properties or ambient changed ->
children `RECASCADE` (not re-match).
**6.3 The diff decides the next stages** (`style_diff(old, new) -> None | Paint | Offset | Layout | Box`, an exhaustive
destructuring of `ComputedStyle` like `SpanStyle::hash_into`, so a new field fails to compile until it is classified):
`Box` (display, position, float, content, list-style, overflow's BFC-ness, table/flex/grid kinds, anything the builder
branches on) -> `BOX` on the box owner (and its parent's for display/position changes); `Layout` -> `ldirty` on the box (an
inline element's change -> `BOX` on its box owner, because `SpanStyle`s are copied into the run and the paragraph re-shapes);
`Offset` (relative/abs insets, translate) -> the box re-placed only (8.3); `Paint` -> the new style is written into the kept
`LayoutBox` and `Laid` (and the paragraph's span colours) without layout. `opacity` 1 <-> < 1 and `z-index` are `Layout`
(stacking boundaries change hoisting).
**6.4 Generated content and counters.** Rule lists without counters/quotes (a `RuleDerived` flag): `::before/::after`
`content` is per element, recomputed when the element is restyled. With counters/quotes: a document-order walk over the
STORED styles (no matching: a few hundred ns per element) recomputes counter values and marks the elements whose generated
text changed (`BOX`). This is R59's `generated_walk`, made cheap by the stored styles.
**6.5 Animations.** `finish_animation_pass` drops the start time of every animation not seen in the pass; an incremental
pass sees few elements. The engine owns the list of animated elements, re-cascades them on each tick, and drops a start
time only when its element lost the animation or left the document (R59 section 6's audit list applies: no
`stash_last_match`, no `VIEWPORT_ROOT_NODE` reset, no full `DISPLAY_NONE_ROOTS`, no whole-memo sweep).

### 7. Box tree update

For every element with `BOX` (deepest first is not needed; containers are independent): rebuild its box owner's content with
today's builder (`build_children` / `collect_real_nodes` / table / flex / grid / marker / first-line / first-letter /
ellipsis), reading styles from the element table instead of resolving, and REUSING the old `LayoutBox` of every element-owned
child that has no `BOX` in its subtree and the same inputs (moved out of the old content by node id). Anonymous boxes and
inline runs are rebuilt (they are cheap: no style resolution). A rebuilt box gets a new `bver`; it and all its ancestors
get `ldirty`. `propagate_canvas_background` is a root fix-up redone when `<html>`/`<body>` change. Whole-build
thread-locals (`STATE_RULES`, `VIEWPORT_H_READ`, `DISPLAY_NONE_ROOTS`, `WANTED_IMG_SRCS`, `WANTED_CSS_IMAGES`,
`STYLE_OVERRIDES`, the leaked `href` strings) become per element or are recomputed from the table. Equality with a fresh
build is checked structurally in shadow mode (section 10).

### 8. Incremental layout

**8.1 Bottom-up first, top-down when needed.** For each `ldirty` box B that has an old `Laid` with a cacheable `ck`
(outermost dirty boxes only): re-measure B alone at its old `ck` (in-flow box: `measure` at its old origin, width,
containing height, overrides; abs/fixed box: `resolve_positioned` at its old containing-block size, as R59 stage 2 does);
clean children inside B are reused (8.2). Then compare B's PARENT VIEW (border box w/h, finalized margins, first baseline,
last line extent, what margin collapsing through B reads, B's out-of-flow and hoisted bubbles) with the old one:
- equal -> splice B's new `Laid` into the tree in place of the old; nothing outside B moves (this is the "layout boundary",
  decided by the result, not by a static size-containment rule, so it also covers `height: auto` boxes whose height did not
  change);
- different -> mark B's parent `ldirty` and repeat one level up.
Floats-in not empty (B inside a block formatting context with floats), no old `Laid`, or a non-cacheable `ck` -> go up.
When the walk reaches the root (or a viewport/global change), run `measure_document` top-down with reuse (8.2): that is
still incremental, because every clean subtree is a move.
**8.2 Reuse inside a measure.** Before relayout the engine "harvests" the old tree along the dirty paths: every clean
element subtree hanging off a dirty path is detached whole into `node -> Laid` (O(path x siblings), no subtree walks).
`measure_inner` on a clean box whose harvested `Laid` has the same `ck` minus origin returns it shifted (`shift_laid_tree`,
O(subtree) only when the position changed; most writes do not move outside boxes). A clean subtree whose measure bubbled
`pending` out-of-flow descendants or hoisted z-index entries keeps those lists in its cache entry and re-emits them shifted
(requires the bottom-up hoisting of QINC1c; `PendingPositioned` borrows the `LayoutBox`, which the kept tree outlives).
Flex/grid/table: the container re-runs its algorithm when it is on the dirty path; items/cells that are clean hit the cache
for the measures whose `ck` matches (2 entries per box: flex measures items for the hypothetical and the final size) and an
**intrinsic-size cache** (`cell_natural_width` min/max per clean box, invalidated with `bver`) serves the content
contributions without walking the item.
**8.3 Offset-only diffs.** A box whose diff is `Offset`: re-place it (its own `Laid` is a cache hit at the new origin) and
the ancestors' extents; this is QYTC1a, but for every box kind the measure already handles (relative, abs with % insets,
translate), because the position is recomputed by the real code with the cached size.
**8.4 Post passes.** `compute_ink` only for re-measured or shifted `Laid`s and their ancestors; hoisting bottom-up
(QINC1c); `in_flow_bottom` and the root height from the root's children; sticky/scroll info is part of each `Laid`.
**8.5 Drift guard.** Repeated shifts accumulate f32 rounding (~0.25 ulp per shift at y ~8 000 = ~0.0002 px). A full layout
every 1 000 incremental updates (or on idle) resets it; tests compare at 0.01 px.

### 9. After layout: one tree, three readers

- **App:** `App::cached` becomes a view of the engine's tree (paint, hit test, forms, media controls, scroll, sticky read
  it). An incremental update installs at once (no worker round trip, no `hold_old_frame`: the update is complete). The
  worker path (section 3) sends a CLONE of the kept box tree (~1-2 ms at 2.7k boxes) and the engine adopts the returned
  `Laid` (with its `ck`s) and shape cache as its tree.
- **GeomCache:** the rect map is updated for touched roots (re-measured, shifted, removed) instead of a full `walk`;
  glyphs are computed on the first `text_rects`/Range read after a layout (today `walk_glyphs` runs on every forced layout).
  `Skel` goes (QYTC1a's shift is 8.3).
- **IntersectionObserver:** reads the current tree; an IO recheck with dirty state runs an incremental update instead of a
  full forced layout.
- **Paint invalidation:** v1 repaints the whole frame as today (2-5 ms on Wikipedia, R47). A damage-rect repaint (QAN1d's
  plan) can take the engine's touched-roots list later (QINC1o).

### 10. Correctness strategy

- **Shadow mode (tests, always on):** under `cfg(test)` every layout that the old path performs ALSO runs the engine's
  incremental update and compares (QINC1a comparators): every styled element's `ComputedStyle` (minus `tree_order`/`node`),
  the box tree structurally (styles, content kinds, inline item text + spans, children), every `Laid` (x/y/w/h/ink within
  0.01 px, `fixed`, `positioned` order, sticky/scroll info, glyph positions). A mismatch panics with the node's
  tag#id.class, the field, both values, and the marks that led there. So the WHOLE existing suite (js_geometry, observer,
  hover, forms, animation, box layout, ...) tests the engine from the day each stage lands, without any behaviour change.
  After the flip the roles swap: the engine's result is used, the from-scratch path is the oracle.
- **Differential fuzzing (tests):** a generator of small pages (classes, ids, attributes, sibling/descendant/child
  combinators, `:nth-child`, `:hover`, inherited and custom properties, `display` none/contents/inline/inline-block/flex/
  grid/table, floats, abs/fixed/sticky, z-index/opacity, counters, `::before`) and of mutation scripts (insert, remove,
  move, text, attribute, class, style, hover, viewport). Fixed seeds, ~200 cases in the normal suite (`SCRATCHY_FUZZ_N` for
  more); incremental == from scratch after every step. This is the main safety net for the combinations nobody writes tests
  for.
- **Real pages:** `SCRATCHY_INCR_CHECK=1` runs the from-scratch path after every incremental update, prints one
  `scratchy: incr-check: MISMATCH ...` diag line per difference, counts `incr.check_mismatch` in `--perf-report`, and uses
  the from-scratch result. `SCRATCHY_INCR=0|1` chooses the path (default 0 until QINC1l, then 1); `SCRATCHY_INCR=0` stays as
  the kill switch.
- **Counters (`--perf-report` `incr` object):** updates, restyled (re-matched / re-cascaded), boxes rebuilt, boxes
  re-measured, cache hits, misses by reason (dirty, key, floats, not cacheable, no old), bottom-up stops by level, worker
  fallbacks by reason, check mismatches. Every stage's gate reads these.
- **WPT subset** (static pages: exercises the first layout and the adoption path) and the full suite with the flag on and
  off at every integration item; no pass -> fail.

### 11. Migration path (main stays green throughout)

1. Infrastructure with NO behaviour change: comparators + `style_diff` + fuzz harness (full vs full) + counters (a), stable
   paragraph ids (b), bottom-up hoisting with document-order ties (c, must reproduce today's trees), invalidation map (d),
   write log (e).
2. Engine stages in SHADOW mode (results computed and compared in tests, never used): restyle (f), box tree (g), relayout
   (h1, h2). In release builds they run only with `SCRATCHY_INCR=1` / `SCRATCHY_INCR_CHECK=1`.
3. Integration behind `SCRATCHY_INCR=1` (default off): forced layouts (i), App frames (j).
4. Gate + exactness sweep on real pages (k). Flip the default (l) after the user's OK.
5. Soak, then delete the special cases of section 1 (m). The from-scratch path stays as the oracle, the fallback for big
   updates (worker) and the kill switch.
Any stage can stop without leaving dead weight on main except its own files: until (l) nothing user-visible depends on it.

### 12. Estimate (projections from section 0 and the R data; QINC1k measures the real numbers)

Per-element costs today (R58, section 0): ~1.4 us per match x 3 slots, cascade + box ~6-8 us, measure ~2 us (more with
shaping). Engine floor per update: harvest + splice + touched-rect update + ink on the path, ~0.2-1 ms on these pages
(the full rect walk it replaces is 0.5-1.0 ms on YouTube results).

| case | today | with the engine | why |
|---|---|---|---|
| YouTube results `getPlayerSize` (R58) | 54-80 ms each, 11-13 per preview job: job 685-1 137 ms | ~2-5 ms each (restyle tens of player elements + rebuild their containers + relayout inside the 440x248 abs wrapper, which stops the bottom-up walk); job ~120-220 ms (93-150 ms of it is not layout) | R58 section 2: all writes inside the preview; section 0: median 33 restyled |
| YouTube watch controls (R50 after QYTC1a/d) | ~1 forced layout/s at 30-80 ms (time text), `ytp-autohide` class flips build | ~1-2 ms each | text change = one paragraph re-shape inside an abs box |
| YouTube watch hover (R55: 18-36 hover builds per run left, 45-81 ms) | build | ~1-3 ms; paint-only hovers ~0.3 ms (no layout) | invalidation sets, `style_diff` |
| YouTube watch App builds (R55: 268 per run, 10.1 s) | ~38 ms each | ~1-4 ms each, ~0 when a forced layout already ran: ~0.3-1 s per run | section 0 watch: median 3 restyled of 261 |
| YouTube watch forced layouts (R55: 234 per run, 7.5 s) | ~32 ms | ~1-3 ms: ~0.3-0.7 s per run | same |
| Wikipedia App builds (R55: 104 per run, 9.4 s) | ~90 ms | ~2-6 ms (10k-element page: harvest/ink paths are short; Page Previews popup = one abs subtree): ~0.3-0.6 s | section 0 Wikipedia: median 6 restyled of 10 071 |
| Wikipedia forced layouts (21 per run, 2.6 s; probe 131-146 ms) | ~125 ms | ~2-6 ms | same |
| loading YouTube (results appended, 100-450 new elements per update) | 30-77 ms | ~5-15 ms (new elements must be styled: no shortcut) | p90 restyled 205 |
| first layout, resize, a new rule-list generation | full | full (worker), plus box/layout reuse where styles compare equal | by design |

Gates proposed for QINC1k: R58 recipe `getPlayerSize` median < 10 ms and worst preview job < 300 ms (QY54b3's gates); R55
Wikipedia and YouTube recipes: App `builds.ms` per run -80 % or more; 0 `incr-check` mismatches on 6 pages x 3 runs.

### 13. Size and risk, honestly

- **Size: XL.** 14 items (4 L, 9 M, 1 S) + 4 optional. The restyle (f), box (g) and relayout (h1) items each touch code paths
  that took dozens of items to get right. Calendar: wave 1 (a, b, d, e) in parallel, then c ∥ f, then g, h1, then i ∥ j ∥ h2,
  then k, l, m: about 7 sequential steps.
- **Risk: high for exactness, low for main's stability.** Exactness bugs are the expected failure (a missed invalidation =
  a stale style or rect). Shadow mode turns every one the suite or the fuzzer can reach into a failing test before anything
  is used, and `SCRATCHY_INCR_CHECK=1` finds the rest on real pages. Nothing user-visible changes before (l), and
  `SCRATCHY_INCR=0` remains.
- **Perf risk:** floors (harvest, ink, rect map) and non-cacheable subtrees could keep some updates at 5-10 ms instead of 1-3;
  the counters show which. Memory +4-14 MB (styles) + the kept box tree (~1.4 KB per box: ~4 MB on YouTube results).

### 14. What scratchy's data structures make hard

1. **Absolute coordinates in `Laid`.** A reused subtree that moved is shifted O(subtree). Fine for writes that move nothing
   outside (the common case: section 0, "rects touched" median 0.9-5.9 %), costly when content is inserted above a long page
   (everything below shifts: still ~ns per box, no allocation). The real fix is relative offsets + shared (`Rc`) fragments,
   which touches every reader of `Laid` (paint, hit test, js_geom, forms, media, hover, scroll, sticky): optional QINC1n,
   decided on QINC1k's numbers.
2. **1.3 KB `ComputedStyle` copied everywhere** (element table, `LayoutBox`, `Laid` (a finalized copy), `InheritedStyle.parent`).
   Keeping trees costs memory; sharing them (`Rc<ComputedStyle>`) is a separate refactor (not proposed now).
3. **Node identity is the `Rc` address.** Every table needs the `WeakHandle` check (pattern exists).
4. **Whole-build thread-locals.** `MATCH_MEMO`, `VAR_EXPAND_MEMO`, `ANCESTOR_BLOOM`, `CLASS_CACHE`, `GENERATED_TEXT`/
   `ELEMENT_CONTENT`, `ANIM_*` passes, `VIEWPORT_ROOT_NODE`, `DISPLAY_NONE_ROOTS`, `STATE_RULES`, `VIEWPORT_H_READ`,
   `WANTED_*`, `STYLE_OVERRIDES`, `next_style_order`: each assumes "one build sees every element". Each needs incremental
   semantics; this audit is most of f and g's risk.
5. **Inline content is copied into the container** (`SpanStyle`, spacers, `apply_first_line`): any change to an inline
   element rebuilds and re-shapes its whole paragraph (the shape memo makes unchanged paragraphs free). Correct, just coarser
   than a browser's line-level reuse.
6. **Out-of-flow and hoisting are bubbles with borrowed references** (`PendingPositioned<'a>` borrows the box) and a post-pass
   (`hoist_negative_z`). Both must become per-subtree cacheable (c, h1).
7. **Tables** re-run the whole table algorithm when anything inside changes (cells reuse their layout and intrinsic widths);
   **flex/grid** measure items several times with different constraints (2-entry cache). Floats are absolute-coordinate
   `Floats` lists: a box with floats coming in is re-measured, not reused.
8. **Progressive shaping** (deadline, `incomplete`) exists for cold loads; an incremental update must complete, so a big
   re-shape goes to the worker path.
9. **`App::cached` has ~45 readers** across app.rs, forms_text.rs, forms_select.rs and tests: moving ownership to the engine
   is mechanical but wide (j).

### 15. Proposed items (the coordinator files them; they supersede QY54b3a-QY54b3f, QY54b4, QY53o2, QY53f2, QY53f3, QY53n4, QY53n1d, QY53n2d, QYTC1a2, QYTC1e, QYTC1f, QY53l3, QY53e2b)

Areas (for scheduling, at most one agent per area at a time): **incr-new** = new `src/incr*.rs`/`src/invalidation.rs` +
their tests; **layout** = layout.rs; **style** = style.rs + anim_eval.rs + images.rs `precompute_generated`; **build** =
images.rs builder; **jsdom** = js_dom.rs + forms*.rs natives; **geom** = js_geom.rs; **app** = app.rs; **perf** = perf.rs.

- **QINC1a [M]** (P2, Sonnet; areas incr-new + perf; no behaviour change; no deps) Engine test infrastructure (R60 sections 6.3,
  10). (1) `style_diff(old, new) -> StyleDiff { None, Paint, Offset, Layout, Box }`: exhaustive destructuring of
  `ComputedStyle` (no `..`), one test per class with a field of each kind, `opacity` 1 <-> < 1 and `z-index` = Layout.
  (2) Comparators `cmp_styles`, `cmp_boxes` (structural: style minus `tree_order`/`node`, content kind, inline items text +
  spans, children), `cmp_laid` (x/y/w/h/ink 0.01 px, `fixed`, `positioned` order, sticky/scroll info, glyphs via the shape
  cache) with a readable first-difference report (node tag#id.class, path, field, both values). (3) Differential fuzz harness
  in `src/tests/incr_fuzz_tests.rs`: seeded generator of pages + CSS + mutation scripts (R60 section 10's list); today it
  lays out each step twice from scratch and asserts the comparators find NO difference (this proves determinism: it will
  hit `tree_order`, which the comparators must skip while `positioned` order is compared). ~200 cases by default,
  `SCRATCHY_FUZZ_N`. (4) `--perf-report` `incr` object with the counters of R60 section 10 (all zero for now). Tests: as listed;
  gate: full suite, clippy count, WPT unchanged.
- **QINC1b [M]** (P2, Sonnet; area layout (+ the shaping-worker loop in app.rs and `walk_glyphs` in js_geom.rs); no behaviour
  change; no deps) Stable paragraph ids (R60 4.4): `next_id` from a per-owner counter that never restarts (App, GeomCache;
  the worker request carries a base and returns the next value); the `ShapeCache` persists per owner and is swept of ids no
  live `Laid` references (after a finished layout, when it exceeds 2x the live count). Readers (paint, hit test,
  hover_patch, forms, `walk_glyphs`, Range) unchanged. Tests: two layouts of a changed page never give one id to different
  text; a reused id still paints its paragraph; sweep keeps every live id; existing paint/hover tests unchanged. Gate: full
  suite, WPT unchanged, `--perf-report` YouTube watch memory (`shape cache` bytes) within 10 % of main.
- **QINC1c [M]** (P2, Sonnet, Opus review; area layout; AFTER QINC1b; no behaviour change) Bottom-up hoisting and
  document-order ties (R60 4.5, 8.2, 14.6): `hoist_negative_z`'s post-pass becomes a bubble returned by `measure` (each
  stacking boundary absorbs its descendants' z-index entries when it finishes; non-boundary boxes pass them up, like
  `pending`); `stack_key` ties use document order (an index assigned by the builder in DOM order, not `next_style_order()`).
  Must produce IDENTICAL trees: a test helper runs the old post-pass and the new bubble on every box-layout / z-index /
  stacking / hoisting test page and on the fuzz harness and compares with `cmp_laid` (needs QINC1a; if a is not merged yet,
  a local comparator). Gate: full suite, WPT unchanged, Acid2 information only.
- **QINC1d [M]** (P2, Sonnet; area incr-new (reads css.rs/style.rs types; at most `pub(crate)` exports there); no deps)
  `InvalidationMap` (R60 section 5): built per `rules_cache` generation from every selector; features class / id / attribute /
  state pseudo-classes, positions self / descendants (rightmost-compound filter) / adjacent / general siblings, structural
  flags, `:not/:is/:where` conservative, `:has` -> Global; API `for_attr(el, name, old, new)`, `for_class`, `for_state(el,
  kind)`, `for_children(parent, change)` returning `Vec<(Handle, Mark)>`. Generalizes `ClassDeps` (js_geom.rs), `HoverKeys`
  and `AttrReaders` (style.rs) without changing them yet. Tests: per selector shape (`.a .b`, `.a > .b`, `.a + .b`, `.a ~ .b
  .c`, `[x] .b`, `:hover .b`, `#i:focus-within`, `:nth-child(2n)`, `:empty`, `:not(.a) .b`, `:is(.a,.b) .c`), and a SOUNDNESS
  property test: random fixtures x mutations, brute-force re-match of every element before/after, predicted marks must be a
  superset; report tightness (predicted / really changed) on the fuzz corpus. Gate: full suite.
  QINC1d DONE (src/invalidation.rs, tests/invalidation_tests.rs): per-feature entries = combinator hop path + subject tag/id/class filter (generalizes ClassDeps/HoverKeys/AttrReaders, which are unchanged); `:is/:not/:where` exact reach, `:nth-child(of)` structural, `:has`/host/part Global; soundness fuzz on an own generator, 5000 cases / 59k steps sound; tightness 1.45 (steps with a change), 3.12 (all steps). Not covered: `:root` (the tree root never changes); radio-group / select option peers only through `for_state(Checked)`.
- **QINC1e [M]** (P2, Sonnet; area jsdom; no behaviour change; no deps) The write log (R60 section 5 table): `incr::note(node,
  Change)` at every DOM-writing native (the 25 `geom_bump*` call sites and every site that marks the App dirty: attribute set/remove with
  name/old/new, child insert/remove/move with siblings, text data, `textContent`/`innerHTML`, form state, adopted CSS,
  `css_ver` bumps); bounded log (overflow -> Global), consumed by nothing yet; counters in `--perf-report` (`incr.log`: entries
  per layout, Global count). App-side sources (interaction, viewport, images, media sizes, animation ticks) come with QINC1j.
  Tests (table-driven, one per native): the expected entries; a no-op write (QY53m) logs nothing; a write to a detached node
  logs nothing. Gate: full suite, YouTube watch headless `--perf-report` JS ms within noise of main (flock, 3 runs).
  QINC1e DONE: src/incr_log.rs + `incr_log::note` in the js_dom.rs natives; `--perf-report` `incr.log` (sub-object of QINC1a's `incr`); the log is drained at each forced layout (nothing consumes it yet; QINC1f should take it there instead).
- **QINC1f [L]** (P2, Opus; area style + incr-new; AFTER QINC1a, d, e) Incremental restyle in SHADOW mode (R60 sections 4.1,
  6): the element table (node id + `WeakHandle`; per slot matched rules, `ComputedStyle`, ambient inputs, generated entries,
  animation membership; flags `RESTYLE`/`RECASCADE`/`DESC_DIRTY`/`BOX`; box owner), filled by a from-scratch build and
  updated from the write log + `InvalidationMap`; top-down traversal, re-match vs re-cascade, early cut-off, inherited /
  custom property / `rem` / ambient propagation, `style_diff` -> marks, generated walk over stored styles (counters/quotes
  flag), animation membership (no full-pass ageing). Shadow: under `cfg(test)` every `build_layout_tree_with_resources` also
  runs the update and `cmp_styles` against the build's styles (panic on mismatch); release: only with `SCRATCHY_INCR=1`/
  `_CHECK=1`. Tests (src/tests/incr_style_tests.rs): one per row of R60 section 5's table, inheritance cut-off (a colour
  change re-cascades children, a width change does not), custom property change through `var()`, `rem` after an `<html>`
  font-size change, counters through an inserted `li`, an animation outside the dirty set keeps its start time, node address
  reuse (1 000 create/remove); the fuzz harness on styles. Gate: full suite green in shadow mode (it now checks every build),
  WPT unchanged; headless `--perf-report` with `SCRATCHY_INCR_CHECK=1` on YouTube results/watch, Wikipedia: 0 style
  mismatches, `incr.restyled` median <= 10 % of styled.
  QINC1f DONE (Opus, 2026-11-20; `src/incr_style.rs`, tests `src/tests/incr_style_tests.rs`): element table per element
  (node key + `Weak`; matches of 6 slots kept as the memo's `Rc` lists, element + pseudo `ComputedStyle`s, `reads_parent` (a
  CSS-wide keyword reads the parent), `reads_vh`, animation membership, ambient inputs, box owner, child keys, check-mode
  fingerprint), filled by the engine's own full pass (first build, new rule-list generation by `rules_cache` generation +
  address, viewport width, Global, `:has`-style global map) and updated from the write log (taken at every shadowed build:
  App and forced layouts alike) through `InvalidationMap`, the interaction-state diff, animated elements and `vh` readers.
  Traversal top-down along `DESC_DIRTY`; `RESTYLE` = `match_slot` (the per-build memo as the match cache), `RECASCADE` /
  parent change = `seed_match` + `resolve_style_for` (no matching); early cut-off: equal style -> nothing below, only
  non-inherited fields -> only `reads_parent` children; `rem` from the `<html>` pre-cascade; `style_diff` -> `BOX` on the box
  owner (and the parent's for `Box` diffs); counters/quotes by the shared `generated_walk` over stored matches (changed
  generated text -> `BOX`). Membership ageing (`Engine::age_animations`, `anim_eval::retain_starts`) exists but the build's
  pass still ages (shadow mode). `::before/::after` are styled only with a `content` (as `generated_content`).
  Shadow: after every `build_layout_tree_with_resources` (tests always, `SCRATCHY_SHADOW=0` skips; release only with
  `SCRATCHY_INCR=1` / `SCRATCHY_INCR_CHECK=1`), in an environment that restores the build's per-build tables, flags, the
  tree-order counter, test counters and the animation registry (read-only evaluation). Check mode compares EVERY style the
  build resolved (slot by slot) and the generated content; it also reconciles the table with the DOM (attributes, child
  lists, text, live form state) and counts writes the log did not describe as `incr.unlogged` (0 on the three pages).
  Found on the way: Wikipedia's reduced-motion `*{animation-duration}` made every element "animated" (membership now needs
  a named animation with keyframes); a `Weak` on the rule list stopped `rules_cache` from splicing in place (the identity is
  now generation + address). A deliberately broken cut-off fails 10 suite tests (the check has teeth); a mismatch inside a
  JS forced layout is caught by `GeomCache`'s panic guard (rects read as zero), so it surfaces as a geometry failure.
  Numbers (release, own wire, headless `--perf-report`, `SCRATCHY_INCR_CHECK=1`, 3 runs each under the perf lock, load
  5.7 -> 0.3; `share` = restyled / styled per build, median over the run's builds):
  | page | builds | full | mismatches | unlogged | restyled median | styled median | share median / p90 | table entries / MB | engine ms total |
  |---|---|---|---|---|---|---|---|---|---|
  | YouTube results 30 s | 40 / 40 / 39 | 3 each | 0 | 0 | 110 / 94 / 110 | 1 130 / 1 117 / 1 184 | 5.1 / 5.1 / 6.2 % ; 31-73 % | 3 050 / 5.9 | 346-354 |
  | YouTube watch 30 s | 114 / 118 / 111 | 4 each | 0 | 0 | 32 / 36 / 31 | 313 / 466 / 313 | 6.2 / 7.0 / 5.9 % ; 20-26 % | 4 275 / 8.8 | 736-910 |
  | Wikipedia Rust 20 s | 4 each | 2 each | 0 | 0 | 70 (the 2 incremental: 3, 70) | 9 207 | 0.7 % ; 100 % (the full ones) | 9 305 / 20.2 | 243-247 |
  Wikipedia's two full builds are the first build and one new rule-list generation (a script-inserted stylesheet: v1 rule,
  QINC1q). Memory: the table estimate is 5.9 / 8.8 / 20.2 MB (element `ComputedStyle`s 12.2 MB of Wikipedia's); RSS on
  Wikipedia `SCRATCHY_INCR=1` vs off 256 vs 232 MB (+24 MB, a little above the accepted +8-18 MB; `Rc<ComputedStyle>`
  sharing would be the fix). Suite (debug, shadow mode on every build): 1 461 passed in 41-55 s vs main 50.7 s (load 5-18,
  noisy); the fuzz harness, the long pole, went 44 s -> 20 s because each step now builds once (its second run is the
  incremental restyle, fed by the log only, reconcile off) and has 564 incremental steps in 200 cases. WPT unchanged (333 /
  293, also under `SCRATCHY_INCR_CHECK=1`: 0 mismatches). Left for later: App-side sources in the log (QINC1j: until then the
  check-mode reconcile covers the parser), `:has`/global maps restyle everything, stylesheet changes re-match everything
  (QINC1q).
- **QINC1g [L]** (P2, Opus; area build; AFTER QINC1f) Persistent box tree in SHADOW mode (R60 sections 4.2, 7): the engine keeps
  the `LayoutBox` tree; containers with `BOX` rebuild their content from stored styles, reusing clean element-owned children
  by node; anonymous boxes, inline runs, markers, tables, flex/grid anonymous items, `display: contents`, inline-block,
  `::before/::after`, first-line/letter, replaced elements (image cache, media sizes, form controls), canvas background
  fix-up; `bver` + `ldirty` up to the root; whole-build thread-locals per R60 14.4 made per element. Shadow: every build
  also updates the kept tree and `cmp_boxes` with the fresh one. Tests (src/tests/incr_box_tests.rs): insert/remove block and
  inline children, text edit inside an inline element (paragraph rebuilt, siblings reused: count), display changes (none ->
  block -> inline -> contents -> flex -> table-cell runs), list renumbering after an insert, `::before` content change, image
  arrival, first-line; fuzz harness on boxes. Gate: full suite in shadow mode, WPT unchanged, `incr.boxes_rebuilt` on the
  three pages (headless, `_CHECK=1`, 0 mismatches).
  QINC1g DONE (Opus, 2026-11-20; `src/incr_box.rs`, tests `src/tests/incr_box_tests.rs`): the engine keeps the box tree
  (a copy of the from-scratch tree after a full restyle). An incremental update takes QINC1f's marks (box owners marked
  `BOX`, plus every element whose style, pseudo styles, match lists or ambient inputs changed and every new element:
  `incr_style::take_box_marks`), image arrivals (each element box records the image `src`s / CSS urls it looked up;
  a cache entry that appears / changes / goes rebuilds exactly those boxes) and video intrinsic-size changes, closes
  them over DOM ancestors (the PATH) and rebuilds the root with today's builder, reading styles and matches from the
  element table (rebuild-only hooks in `resolve_style_for` and the match memo; generated content from the engine's own
  walk). Every element box OFF the path is moved over from a pool filled lazily (rebuilding a box first detaches its old
  element-owned descendants through anonymous boxes, table rows and inline-block items). Always rebuilt with their
  container: `<html>`/`<body>` (the canvas-background and viewport-overflow fix-ups are redone on the new root and take
  from them), form controls and media (live state, intrinsic sizes; cheap leaves), boxes with inline content (leaves
  whose parent may rewrite `text-overflow`), list items whose parent's child list changed (the marker counts siblings).
  Document-order ties are renumbered on the kept tree (`renumber_document_order`, without QINC1c's test bookkeeping).
  `bver` / `ldirty` are side tables by node (rebuilt path; QINC1h1 reads them). Per element instead of per build:
  image srcs, video sizes, generated content, `display: none` roots (`incr_style::display_none_roots`); the rebuild
  leaves `WANTED_IMG_SRCS` / `WANTED_CSS_IMAGES` / `DISPLAY_NONE_ROOTS` alone. Design choice: the whole PATH is
  rebuilt (each ancestor's own content list, siblings moved), not only the box owner spliced in place: it needs no
  "where is this box" search through anonymous wrappers and makes box ownership imprecision harmless; cost is
  O(path x children). Check: `cmp_boxes` of the kept tree vs the from-scratch tree after every shadowed build (tests
  panic; release diag + `incr.check_mismatch` + restart from the fresh tree); `incr_cmp` is compiled in release and
  builds paths only on a difference. Fuzz: the harness also compares the kept tree after every step; 3 000 cases
  (`SCRATCHY_FUZZ_KEEP_GOING=1`): 7 998 incremental box rebuilds (42 823 boxes rebuilt, 33 992 moved), 0 box mismatches;
  it found 7 STYLE mismatches of QINC1f on seeds above the default 200 (462, 519, 733, 1956, 2080, 2084, 2475:
  first-child / nth-child / sibling invalidation after inserts and moves; QINC1f-fix proposed). Suite (debug, shadow
  on every build): 1 488 passed in 40.2 s (load 0.5). WPT unchanged (333 / 293), also under `SCRATCHY_INCR_CHECK=1`
  (0 mismatches). Clippy 77 (local limit 77, none in the new files).
  Numbers (release, own wire, headless `--perf-report --budget-ms 20000`, YouTube 30 s, Wikipedia 20 s, each run under
  the perf lock and the 6G scope, 1-min load 0.14-2.35; `rebuilt`/`moved` = element boxes per run):
  | page | check runs | box updates | rebuilt | moved | box ms total | mismatches | kept tree estimate | RSS: off / main INCR=1 / this INCR=1 |
  |---|---|---|---|---|---|---|---|---|
  | YouTube results | 3 | 31 / 33 / 30 | 3 780 / 3 825 / 3 793 | 1 864 / 1 923 / 1 855 | 17.9 / 18.9 / 17.7 | 0 | 5.7 MB (2 200 element boxes) | 348, 419 / 426, 436 / 442, 432 MB (noise > delta) |
  | YouTube watch | 3 | 114 / 106 / 106 | 6 245 / 6 020 / 6 016 | 3 205 / 2 774 / 2 720 | 40.5 / 37.6 / 38.3 | 0 | 7.5 MB (3 200) | 470, 468 / 505, 493 / 500, 494 MB (noise > delta) |
  | Wikipedia Rust | 3 | 2 each | 100 each | 50 each | 1.0 / 1.1 / 1.0 | 0 | 15.2 MB (1 595) | 226, 226 / 247, 254 / 273, 276 MB (+24 MB) |
  Per incremental update: ~120 boxes rebuilt / 0.6 ms (results), ~57 / 0.36 ms (watch), 50 / 0.5 ms (Wikipedia), vs
  1 600-3 200 element boxes per from-scratch build. `incr.unlogged` 0. MEMORY is above the accepted envelope on
  Wikipedia: the kept tree is 15.2 MB because `InlineItem` is 1 328 B (its `Image` variant carries a whole
  `ComputedStyle`; `SpanStyle` is 112 B) and Wikipedia has ~10k inline items; RSS +24 MB over QINC1f's INCR=1, +48 MB
  over off. Boxing `InlineItem::Image::style` (layout.rs, outside this item's area) would cut every inline item to
  ~120 B (both the kept and every fresh tree); proposed as QINC1g-mem. The copy lives only with `SCRATCHY_INCR*=1`
  until QINC1j makes it THE tree (then no second tree exists).
  QINC1f-fix DONE (Opus, 2026-11-20; `src/invalidation.rs`, `src/incr_style.rs`): the 7 fuzz seeds above the default
  window had two causes. (1) All seven: the engine replays the write log against the FINAL DOM, and `for_children`
  computed its structural edges per entry there ("first two / last two element children", the element after the change
  point). One batch inserting several elements in front of the old first child (a parsed fragment goes in one node at a
  time; seed 2084 after a leading text node) leaves the old first child third, outside every insert's edge: it kept
  `div:first-child` (`position`, `width:fit-content`, `gap`, `clear`, `white-space` in the seeds; appends + `:last-child`
  likewise). Fix: `InvalidationMap::for_child_list(parent, old)` compares the element children at the last restyle (the
  stored child keys that have entries, captured before the log is processed) with the current list: old and new first
  / last for `:first/last/only-child`, every element whose previous element sibling changed for `+`, everything from the
  first difference for `~`, everything for `:nth-*`; called once per changed parent after the per-entry marks (and in the
  check-mode reconcile). Moves (remove + insert) and stale logged `prev`/`next` need nothing special any more. (2) Seed
  2475, hidden behind (1) (it then failed in the BOX check one step later): removing an element whose node is freed at
  once (no handle) removed only its own entry; its descendants' entries stayed with child lists naming freed TEXT nodes,
  whose addresses new text nodes reuse, so `text_parent` found the stale parent and a text edit was lost (the kept box
  kept the old text). `drop_key` drops the descendants through the stored lists; `text_parent` accepts only a live
  parent that still holds the text node. Tests: `qinc1f_fix_*` (incr_style_tests.rs; the first-child, last-child and
  freed-subtree ones fail on main), `child_list_marks_old_and_new_edges_and_changed_siblings` (invalidation_tests.rs).
  Fuzz (release, `SCRATCHY_FUZZ_KEEP_GOING=1`, 6G scope, load 1.3-5): seeds 0.., 10 000.., 100 000.. x 5 000 cases =
  15 000 cases, 0 style and 0 box mismatches, 28.9 / 28.5 / 28.2 s (14 518 / 14 506 / 14 619 incremental restyle steps,
  ~13.3k incremental box rebuilds each). CI: the fuzz test now runs 1 000 cases in release (5.4 s locally, was 200) and
  200 in debug, plus the seven seeds as `REGRESSION_SEEDS`.
- **QINC1h1 [L]** (P2, Opus; area layout; AFTER QINC1b, c, g) Relayout from the last tree in SHADOW mode (R60 sections 4.3,
  8.1-8.5): `ck` + `cacheable` on element `Laid`s; harvest along dirty paths; reuse in `measure_inner` (shift by the origin
  delta); bottom-up re-measure at the old `ck` with the parent-view comparison and splice, else one level up; abs/fixed via
  `resolve_positioned` at the cached containing block; cached bubbles (pending, hoisted); offset-only diffs; `compute_ink` on
  touched paths; drift guard (full layout every 1 000 updates); touched-roots list. Shadow: every layout also relayouts the
  kept tree and `cmp_laid` with the fresh one. Tests (src/tests/incr_layout_tests.rs): a write inside an abs box stops at it
  (count), `height: auto` box whose height does not change stops there, a growing paragraph moves following siblings (shift
  count), floats in force a walk up, flex item text change (container re-runs, siblings hit), table cell change, fixed
  descendant added/removed inside a clean subtree, z-index child inside a reused subtree (paint order), sticky inside a
  reused subtree, scroll container content change; the fuzz harness on layout. Gate: full suite in shadow mode, WPT
  unchanged; headless `_CHECK=1` 0 mismatches on YouTube results/watch, Wikipedia; `incr` hit rate and bottom-up stop levels
  reported.
  QINC1h1 DONE (Opus, 2026-11-20; `src/incr_layout.rs`, tests `src/tests/incr_layout_tests.rs`): the engine keeps the
  `Laid` tree it laid out last for QINC1g's kept box tree, with its own shape cache and never-restarting paragraph ids.
  Every element `Laid` an engine measure produces carries `Laid::ck` (layout.rs `Ck`, `None` on every other path):
  origin, width, containing height, auto-margin / forced-border overrides, "inside a static inline-block", where
  `measure_inner` returned it (position and size: flex/table parents stretch it afterwards), the box's `bver`, the
  out-of-flow boxes it sent past itself (`pending`: box address + static position) and the keys of its z-index bubble,
  the box-style fields the parent's flow reads (`PSig`), `resolve_positioned`'s containing block, `intr` (an ancestor
  reads its intrinsic size: flex/grid/table containers, shrink-to-fit/keyword widths, flex items) and ink /
  in-flow-bottom bookkeeping. Hook: `measure_document` (after a complete from-scratch layout of the tree the last
  shadowed build returned, identified by its root child list's buffer) -> `incr_layout::after_measure`; nothing runs
  with the env vars unset. Update: (1) BOTTOM-UP from the elements the box updates touched since the last relayout
  plus the boxes they rebuilt off the path (outermost only): re-measure the box alone at its old key (out-of-flow:
  `resolve_positioned` at the cached containing block), clean children served from the old tree; splice when the
  parent view is unchanged (border box, margins, first baseline, collapse-through inputs, the `finish_z` values,
  sticky, z-index; out-of-flow boxes it sends past itself identical and clean; its z entries are swapped in their
  owner's list, which is re-sorted with current document-order keys), else one level up; (2) the root reached:
  TOP-DOWN `measure_document_engine` with reuse: the old tree is harvested along the dirty paths (clean = same
  `bver`; clean element `Laid`s detached whole by node, hoisted entries by (node, pseudo)) and `measure_inner` returns
  a clean box at the same key minus the origin as its old `Laid` shifted (`try_hit`), re-emitting `pending` and its
  bubble (re-keyed `tree_order`). Ink and the root's in-flow extent are recomputed only through re-measured boxes
  (`finish_root`; a moved reused subtree has its ink shifted). Drift guard: a full engine layout every 1 000 updates;
  touched roots: `incr_layout::touched_roots`. Offset-only changes are the bottom-up case (the box is re-measured with
  its children hits; an abs box is re-resolved at its containing block). Shadow: tests and `SCRATCHY_INCR_CHECK=1`
  compare with `cmp_laid` (tests panic; release `incr-check: MISMATCH laid ...` diag, `incr.check_mismatch`, start
  over); `SCRATCHY_INCR=1` alone relayouts without comparing. Not shadowed: App frames (measured on the shaping
  worker thread, which has no engine; their builds still feed the dirty list): QINC1j.
  Found on the way (all reproduce only when a laid box is reused, so they are guarded, not changed): (1) layout is NOT
  exactly translation-invariant in x where a mixed inline run is involved: `measure_mixed_inline`'s line width is
  `(x + w) - x` in document space, and a shrink-to-fit box whose text fits exactly wraps at another x (fuzz seed
  11814); boxes with such content are reused only at the same x (`Ck::x_sensitive`). (2) Out-of-flow boxes are
  measured outside their ancestors' recursion, yet some intrinsic walks see them (an inline-level fixed box in a
  `max-content` box, an abs box in a table cell): a change below an out-of-flow box never stops under an ancestor
  that reads intrinsic sizes. (3) flex/table containers mutate a child `Laid`'s size after `measure` (stretch): a hit
  restores the returned size. (4) `cmp_laid` recursed without stack growth (a 4 000-deep compare overflowed): now
  through `stack::grow`. (5) `renumber_document_order` returns its index (a reused entry is re-keyed with it).
  Tests: one per case of the item (abs box stops at itself, `height: auto` box stops at itself with a walk of < 30
  boxes on a 60-section page, growing paragraph moves the following siblings (2 shifted hits), floats in force a walk
  up, flex item text change (container re-runs, 2 sibling hits), table cell, fixed descendant in a clean subtree
  re-emitted / removed / added, z-index child in a reused subtree + an insertion before it, sticky in a reused subtree,
  scroll container stops the walk, offset-only change re-places the box); the fuzz harness compares the kept laid tree
  after every step and counts relayouts. Fuzz (release): 26 000 cases over 7 windows (seeds 0, 10 000 ... 60 000),
  76 000 incremental relayouts, ~310 000 hits, ~30 000 splices, 0 laid / box / style mismatches (the 7 QINC1f-fix
  regression seeds run by default and pass). Suite: 1 506 passed, debug 44.6 s (load 1.3 -> 8.9; QINC1g 40.2 s at
  0.5), release 37.9 s. WPT unchanged (333 / 293), also under `SCRATCHY_INCR_CHECK=1` (0 mismatches). Clippy 77.
  Numbers (release, own wire, headless `--perf-report --budget-ms 20000`, `SCRATCHY_INCR_CHECK=1`, YouTube 30 s,
  Wikipedia 20 s, each run under the perf lock and the 6G scope, 1-min load 4.2 -> 0.3; `incr ms / update` = engine
  relayout time over its incremental updates, `fresh ms / layout` = the from-scratch measure it shadows; hit rate =
  hits / (hits + misses), a miss being a box measured for real):
  | page | runs | mismatches | relayouts (full / top-down / spliced only) | splices | stops by level walked (0..6, 7+) | hits / misses (rate) | incr ms / update | fresh ms / layout | kept laid tree |
  |---|---|---|---|---|---|---|---|---|---|
  | YouTube results | 3 | 0 | 39 / 36 / 30 (full 3 each; top-down 15 / 14 / 16) | 20 / 18 / 11 | 0,0,1,0,0,0,0,19 (run 1) | 2 331 / 4 162 (0.36), 0.35, 0.34 | 2.8 / 2.8 / 2.6 | 7.0 / 6.8 / 6.5 | 5.0 MB (2 180 element `Laid`s) |
  | YouTube watch | 3 | 0 | 109 each (4 / 42) | 66 each | 34,8,12,1,2,0,0,9 | 2 468 / 6 188 (0.29), 0.29, 0.28 | 0.75 | 2.9 | 7.8 MB (3 220) |
  | Wikipedia Rust | 3 | 0 | 4 / 5 / 4 (2-3 full, 0-1 top-down) | 3 / 1 / 3 | 0,0,0,1,0,0,0,2 | 28 / 110 (0.20) | 0.5 | ~51 | 5.5 MB (1 595) |
  Why a step went up (watch, per run): an ancestor reads intrinsic sizes 366, flex/grid/table parent 127, out-of-flow
  boxes sent past it changed 96, z entries changed 22, float 16; the walks that reach the root (42 of 105 watch
  updates, 15 of 36 results) are flex chains (YouTube lays out with nested flex; the item says "the container
  re-runs"): QINC1h2's intrinsic-size cache is what lets them stop lower. Memory (`--perf-report` RSS, 2 runs each):
  Wikipedia off 214 / 215 MB, main e61945b `SCRATCHY_INCR=1` 258 / 265, this `SCRATCHY_INCR=1` 266 / 266; YouTube
  results off 334 / 333, main INCR=1 413 / 411, this INCR=1 417 / 423 (+1 to +12 MB over QINC1g, within run noise on
  YouTube; the kept laid tree's paragraphs are mostly shared with the shape memo and counted there). The copy lives
  only with `SCRATCHY_INCR*=1` until QINC1i/j make it THE tree.
- **QINC1h2 [M]** (P2, Sonnet; area layout; AFTER QINC1h1; can run beside QINC1i/j) Intrinsic-size cache and flex/grid/table
  reuse (R60 8.2): `cell_natural_width` (min, max) per clean box keyed by `bver`; 2-entry `ck` cache for flex items; table cell
  natural widths. Exactness by shadow mode; tests: a change outside a flex row reuses its items' intrinsic sizes (count), a
  change inside one item recomputes only that item. Gate: `incr` counters on YouTube watch (flex-heavy) show intrinsic hits;
  0 mismatches.
- **QINC1i [M/L]** (P2, Opus; areas geom + jsdom readers; AFTER QINC1h1; parallel with QINC1j) Script forced layouts through the
  engine behind `SCRATCHY_INCR=1` (R60 sections 3, 9): `GeomCache::refresh` -> engine update; rect map from touched roots;
  lazy glyphs; forced layouts use the window's image cache (thread-local handle the App sets; headless: its own decoded
  images); IntersectionObserver reads the engine's tree; QYTC1a/QYTC1d/HiddenSnap/ATTR_SNAP bypassed when the flag is on;
  `SCRATCHY_INCR_CHECK=1` diag + counter. Tests: the js_geometry / observer / Range suites run twice (flag off and on, a test
  seam), image sizes visible to JS with the flag. Gate: full suite both ways; R58 recipe (window, 3 runs, flock) with the flag:
  `getPlayerSize` median < 10 ms, worst preview-start promise job < 300 ms, `_CHECK=1` 0 mismatches.
  QINC1i DONE (Opus, 2026-11-20; `src/incr_engine.rs`, `js_geom.rs` `refresh_engine`, tests `src/tests/incr_forced_tests.rs`
  + the two suites re-run): with `SCRATCHY_INCR=1` (or `_CHECK=1`) a script forced layout runs ONE engine update and no
  from-scratch layout: the same checks and restyle + box update as an App frame (QINC1j's `incr_style::live_ready` /
  `is_current` / `live_update`, rebased onto #459; nothing changed since the engine's last update = no work), then
  `incr_layout::update_kept` (a complete relayout of the kept box tree on the main thread; from scratch through the engine
  when it cannot be incremental: stale App view, box tree started over, viewport width, drift guard). First layout of a
  document, and a restyle that cannot be incremental (new rule-list generation, viewport width, `Global` log entry, no
  table yet: `live_ready` says so before restyling): the from-scratch build with the shadow update on (re)starts the kept
  box tree (counter `incr.forced_restarts`), then the engine lays it out. Rects: a full
  `walk` of the kept laid tree (measured 0.19 ms per forced layout on YouTube results, `rect_walk` 8 ms / 43 layouts, so
  the touched-roots map was not worth its bookkeeping; `incr_layout::touched_roots` stays for QINC1o). Glyphs: collected
  from the kept tree on the first `text_rects` read after a layout. Images: the App hands its image cache over at each of
  its builds (`incr_engine::sync_window_images`, copied only when an entry changed; one line in app.rs); forced layouts of
  that document lay images out at their decoded size (so App builds and forced layouts feed the box engine the same
  images); headless and other documents: none (headless decoding NOT done: proposed QINC1i2). IntersectionObserver: the
  App-layout source (`rendered_ok`) is off with the flag, so its reads update the engine (cheap). Bypassed with the flag:
  QYTC1a `style_write_in_place`, QYTC1d class probes, QY53l2 HiddenSnap, QY53o1 ATTR_SNAP (every write bumps; the next read
  updates). Check mode (tests, `SCRATCHY_INCR_CHECK=1`): the oracle is today's from-scratch layout with the shadow update
  off (`incr_style::without_shadow`), compared with `cmp_laid`; a difference is reported (`incr-check: MISMATCH laid ...`,
  `incr.check_mismatch`), every stage is dropped and the from-scratch result is used. QOBS3 phases: engine restyle + box
  = `incr_shadow`, relayout = `measure_document`. Tests: 5 new; js_geometry (Range included) and observer suites re-run
  with the engine (`*_incr_forced_tests`, seam = thread name, or `SCRATCHY_TEST_INCR_FORCED=1` for the whole suite); the
  16 tests of the bypassed special cases return early there. Suite (rebased onto QINC1h2 + QINC1j): 1 711 passed with the
  flag off, 1 711 with `SCRATCHY_TEST_INCR_FORCED=1`. WPT 333 / 293, no change, flag off and `SCRATCHY_INCR=1`. Clippy 77.
  In check mode a forced layout skips QINC1j's "nothing changed" shortcut: the update's reconcile then also sees writes
  that bypass the log (a Rust-side text write in a test was otherwise reported by the oracle).
  Gate (R58 recipe, own nested dwl 1052x1050, own wire, `SCRATCHY_FRAME_STATS`, under the perf lock; per-layout ms from a
  throwaway `eprintln` in `perf::forced_detail`, not committed; 1-min load 0.3-7.5):
  | run | `getPlayerSize` reads | median ms | p90 | max | worst promise job after 15 s (preview start) |
  |---|---|---|---|---|---|
  | INCR=1, final code (41fdae0 on e219bee), 3 runs | 39 / 38 / 40 | 5.80 / 5.39 / 5.91 | 8.0-9.0 | 32-35 | 182 / 176 / 183 ms |
  | INCR=1, before the rebase onto QINC1h2/QINC1j, 6 runs (1 more dropped: a 500 ms page job killed at load 15, no preview) | 38-40 | 5.37-6.13 | 8.3-9.5 | 9.0-26.5 | 174-180 ms |
  | flag off (same binary), 3 runs | 26 / 27 / 26 | 62.6 / 61.4 / 75.8 | 64-81 | 65-87 | 825-1 040 ms |
  `_CHECK=1` in the window: 6 runs, 0 mismatches (forced layouts 18-99 ms there: the oracle and the reconcile run too; with
  QINC1j every App frame is checked as well, and one of the two final-code runs never started the preview).
  One outlier with the flag: a 619 ms IntersectionObserver forced layout at 28 s (pre-rebase run 2; observer delivery
  697 ms), not seen in the other 8 runs; not investigated (proposed QINC1i3).
  Headless (`--perf-report --budget-ms 20000`, own wire, perf lock, 6G scope, load 0.2-3.6; forced layout ms total per run):
  | page | off (3 runs) | main 109445c `INCR=1` | this `INCR=1`, final code (3 runs) | engine / restarts per run | `_CHECK=1` mismatches |
  |---|---|---|---|---|---|
  | YouTube results 30 s | 887-981 (35-36 layouts) | 1 358 / 1 984 | 525 / 634 / 585 (36-42) | all / 3 | 0 (6 runs) |
  | YouTube watch 30 s | 2 457-3 396 (108-141) | - | 1 289 / 1 103 / 1 020 (119-122) | all / 4 | 0 (6 runs) |
  | Wikipedia Rust 20 s | 567 / 600 (4) | 846 / 1 701 | 1 133 / 951 / 660 (4-5) | all / 2-3 | 0 (6 runs) |
  Wikipedia is SLOWER than off: 2-3 of its 4-5 forced layouts are restarts (new stylesheets), and a restart is a
  from-scratch build whose shadow update restyles everything again, plus the engine's own full layout (~330-450 ms vs
  244-262 ms for the plain path): proposed QINC1i4. Memory (`rss_bytes`): no growth over main with `INCR=1` (results
  368-431 vs 371-441 MB, Wikipedia 268-284 vs 285-286); vs the flag off +~40-50 MB on Wikipedia (the kept trees, QINC1f-h1).
- **QINC1j [L]** (P2, Opus; area app; AFTER QINC1h1; parallel with QINC1i) App frames through the engine behind
  `SCRATCHY_INCR=1` (R60 sections 3, 9): incremental updates install at once; worker path for first layout, width change, new
  rule-list generation, restyled share > 25 %, with the box tree cloned for it and the result adopted; `App::cached` -> view
  of the engine's tree for all readers; interaction state, viewport height, image / CSS image / media size arrival and
  animation ticks feed the write log; a forced layout since the last frame = no work for the frame; hover patch paths
  bypassed with the flag. Tests: App-level tests with the flag for hover (paint-only and layout hovers), focus, animation
  frames, image arrival, resize, link CSS arrival, a JS write + forced read + frame (the frame does no layout: counter);
  existing suite with the flag off. Gate: R55 recipes (Wikipedia, YouTube watch; nested window, 3 interleaved pairs, flock):
  App `builds.ms` per run, frame `lat_p95`, `_CHECK=1` 0 mismatches.
  QINC1j DONE (Opus, 2026-11-20; `src/incr_frame.rs`, glue in `incr_style.rs` / `incr_box.rs` / `incr_layout.rs`, App
  in `app.rs`; tests `src/tests/incr_frame_tests.rs`): with `SCRATCHY_INCR=1` every App frame goes through
  `incr_frame::frame`: **Current** (the engine's laid tree already shows the page: no DOM / CSS write in the log, same
  interaction state, viewport, image cache, media sizes, no running animation, nothing waiting for a relayout; a forced
  layout since the last frame lands here, whichever path updated the engine, so it works before and after QINC1i),
  **Main** (incremental restyle + box update + relayout on the main thread, installed at once: no worker round trip,
  no `hold_old_frame`), **Worker** (first layout, width change, new rule-list generation, Global log, arena reset:
  from-scratch build whose shadow stage restarts the kept trees; restyled share > 25 % AND >= 200 elements restyled,
  or the relayout cannot run incrementally: a COPY of the kept box tree). The worker lays an engine job out with
  `incr_layout::worker_measure` (every element `Laid` records its `Ck`, with the box engine's `bver` table shipped to
  the worker thread) and sends the tree back with the complete result; the engine adopts both when the kept tree has
  not changed since the copy (tree `EPOCH`), else `adopt_refused`. `App::cached` is `Option<Rc<Laid>>`: with the flag
  it is the engine's kept tree itself (copy on write: `SharedLaid`; the App drops its handle before a frame update, a
  forced layout between two frames pays one deep copy), with its shape cache cloned once per new version. Hover
  match-memo / paint-patch paths (QY53n1/n2/n3) are bypassed with the flag. App-side inputs log `Change::Ambient`
  (interaction, viewport, image, media size, animation tick); the engine diffs their state itself. A live rebuild
  fills the App's "wanted image / CSS image" tables (a shadow rebuild still leaves them alone). Live restyles create
  and report animation start times (`anim_eval::finish_report_only`) and age them by membership
  (`Engine::age_animations`). Check mode (`SCRATCHY_INCR_CHECK=1`, and every test) re-runs a from-scratch build +
  layout after each Main frame through the shadow comparisons; a mismatch shows the from-scratch layout. With frame
  stats on, the window prints its `incr` counters at exit (`scratchy: incr: {...}`); new counters
  `incr.frames_current/main/worker`, `worker_fallbacks` reasons. With the flag unset nothing of this runs.
  Tests (10, App-level, through a shared event-loop proxy; the worker result is laid out inline in tests): paint-only
  and layout hovers (Main, then Current), focus, animation frames at 250/500/750 ms, image arrival (only the reading
  boxes rebuilt), resize (height = Main, width = Worker + adoption, then Current and Main), link CSS arrival (new
  generation = Worker, then Main), a JS write + forced read + frame (Current: `updates`, `box_updates`,
  `layout_updates` unchanged by the frame; the frame shows the forced layout's tree), big restyle (Worker with the
  kept-tree copy, adopted), media-size logging, flag off = old path. Suite 1 516 passed (debug, 46 s), WPT unchanged
  (333 / 293), clippy 77.
  Gate (release, own wire per run, own nested dwl 960x1006, R55 input: Wikipedia Rust 52 s with 6 rounds x 8 links
  x 600 ms; YouTube watch G1AeBgalFZk 56 s, click on play then 6 rounds x 8 positions; flag off vs `SCRATCHY_INCR=1`
  of the SAME binary, interleaved, order flipped per round, every run under `flock /tmp/scratchy-perf.lock` and the 6G
  scope; 1-min load 0.2-2.8 for the pairs; `builds` = main-thread frame work, `worker` = shaping-worker ms;
  `lat_p95` = median over the seconds with input of the per-second p95):
  | page, pair | off: builds n / ms, worker ms, lat_p95 | INCR=1: builds n / ms, worker ms, lat_p95 |
  |---|---|---|
  | Wikipedia 1 | 104 / 9 633, 2 021, 217 ms | 151 / 3 852, 209, 45 ms |
  | Wikipedia 2 | 104 / 9 591, 1 987, 212 ms | 153 / 4 163, 225, 182 ms |
  | Wikipedia 5 | 104 / 9 719, 2 061, 216 ms | 152 / 4 199, 228, 185 ms |
  | Wikipedia 3, 4 (page stalled in both: 19-22 builds) | 19 / 1 523; 20 / 1 601 | 22 / 1 384; 20 / 715 |
  | YouTube, "active" runs (~310 builds off) | 312 / 11 470, 1 363, 36 ms; 308 / 11 986, 1 404, 29 ms | 503 / 5 071, 107, 30 ms; 360 / 6 075, 150, 82 ms |
  | YouTube, "quiet" runs (63-69 builds off) | 66 / 2 690; 63 / 2 426; 69 / 2 970 | 81 / 4 450; 89 / 5 432; 57 / 2 612 |
  Wikipedia: main-thread build time -57 % (9.6 s -> 3.9-4.2 s) and worker time -89 % (2.0 s -> 0.2 s); the frames
  are 147 Main + 5 Worker (first layout, 2 new generations, ...). YouTube runs fall in two regimes for both binaries
  (the same input; probably whether playback starts), so they do not pair: in the active regime INCR=1 halves the
  build time; in the quiet regime it is +50-80 % WORSE. Cause (temporary timing diag, removed): hovering in and out of
  `ytd-app` / `div#content.ytd-app` makes the invalidation map mark 7 510-7 556 descendants (`:hover` descendant
  sets), so the engine re-matches 4 061 of 4 289 styled elements, taking ~290 ms per such frame (~70 us per element,
  the from-scratch build does the WHOLE page in ~40 ms, ~10 us per element); 12-13 such frames per run go on to the
  worker (`restyled_share`). Forced layouts with INCR=1 are also slower (the shadow stage's update runs after each; and
  their empty image cache makes the next frame rebuild every image box: QINC1i's "window image cache" fixes that).
  `SCRATCHY_INCR_CHECK=1`: Wikipedia 3 runs, YouTube 2 runs: **0 mismatches** (YouTube: 33 Current / 127 Main / 13
  Worker frames in one run; Wikipedia: 18 Main / 3 Worker). RSS at the end: Wikipedia off 335-342 MB vs INCR=1
  347-364 MB; YouTube off 472-489 vs 498-563 MB (the kept trees; no second laid tree any more for App frames).
  Proposed: QINC1j-a (engine restyle cost: ~70 us per re-matched element vs ~10 us in the build: the 6 match slots
  per element and the per-update environment; a full / near-full restyle should cost what a build costs, or the
  engine should adopt the build's styles instead of re-resolving after a from-scratch build), QINC1j-b (invalidation
  precision: `:hover` on `ytd-app` marks 7.5k descendants; find the rules and filter the descendant set by the
  subject's features), QINC1j-c (App-level tests share one event loop per test process: ~20 existing App tests build
  their own and silently return when another test holds it; move them to the shared proxy in incr_frame_tests.rs).
  QINC1j-ab DONE (Opus, 2026-11-20; `src/invalidation.rs`, `src/incr_style.rs`, two lines in `images.rs`; tests in
  `invalidation_tests.rs`, `incr_style_tests.rs`): the quiet-regime YouTube slowdown was the INVALIDATION, not the
  re-match. Temporary timing diag (removed) on one hover-into-`ytd-app` frame: update 295 ms = marks 235 ms + visit 53 ms
  (re-match + cascade of 4 062 elements: ~13 us each, about what the build pays) + 6 ms counter walk. The selectors
  behind the 7.5k marks: `.ytLpFaqEntryViewModelHost:hover [class$=icon-hover-highlighting]` (subject without
  tag/id/class: every descendant), `.ytp-info-panel-detail-close:hover path`, ...: the map never checked the compound
  holding `:hover` on the element ENTERING the state, and walked the subtree once per entry. Changes (no style / box /
  layout semantics; the shadow compares and the fuzz harness keep them exact):
  (1) **carrier filters**: every `Entry` keeps the tag / id / classes of the compound that holds its feature; `apply`
  drops an entry whose carrier filter the changed element fails (same `filter_self` rule as before: not for a `class` /
  `id` write, which changes those features; a pseudo-element compound's state sits on the subject: no filter). A
  `:hover` on `ytd-app` now reaches only rules whose `:hover` compound can match `ytd-app`.
  (2) **one walk per hop path**: entries of a key that share a hop path are walked once, keeping the elements that pass
  any of their subject filters (`None` in the group = everything); node sets use `PtrHasher`.
  (3) **map build**: dedup by sorting each key's entries instead of a `HashSet<(Key, Entry)>` (YouTube: 26.5k
  selectors, 55k adds, 32k entries: ~40 -> ~25-30 ms per rule-list generation).
  (4) **counter / quote walk skipped** when nothing it reads changed (no DOM write in the log, no stored match list
  changed, no unlogged change found by the check-mode reconcile; `Stats::gen_walked`).
  (5) (QINC1i4, matches half) **the shadow update after a from-scratch build starts from the build's match memo**
  (`after_build(.., build_memo)`, `Env::enter_as(.., memo)`): it runs on the same DOM and state, so a restart's full
  restyle re-matches nothing; the cascade half (adopting the build's styles) is left for QINC1i4.
  Tests: `hover_on_ancestor_filters_by_carrier_compound` (hover on an ancestor with only `.menu:hover` rules marks
  nothing; key-less `:hover > .icon` / `:hover .icon` still walk; attribute write filtered by the carrier's class, a
  class write not), `grouped_walk_is_union_of_entries`, `qinc1j_ab_hover_on_an_ancestor_rematches_only_what_its_rules_reach`
  (60 menus: hover on `#app` re-matches 0, on `#m3` exactly 3), `qinc1j_ab_counter_walk_only_after_a_change_it_can_see`;
  8 new carrier shapes in the invalidation soundness fuzz (5 000 cases x 12 steps: 0 unsound, tightness 1.48 on steps
  with a change). Fuzz harness (release, `SCRATCHY_FUZZ_KEEP_GOING=1`): 7 windows x 3 000 cases (seeds 0 ... 60 000):
  0 mismatches. Suite 1 715 passed (debug, after rebasing on QINC1i #460).
  Gate (release, rebased on #460; own wire per run, own nested dwl 960x1006, QINC1j's R55 input; `yt` = click on play,
  `ytq` = the same without the click; flag off vs `SCRATCHY_INCR=1` of the SAME binary, interleaved, order flipped per
  round, every run under `flock /tmp/scratchy-perf.lock` and the 6G scope; 1-min load 0.2-1.2):
  | run | off: builds n / ms, worker ms | INCR=1: builds n / ms, worker ms |
  |---|---|---|
  | YouTube `yt` 1, 2, 3 | 86 / 3 317, 438; 95 / 3 074, 384; 66 / 2 601, 341 | 74 / 1 017, 47; 70 / 936, 45; 77 / 981, 39 |
  | YouTube `ytq` 1, 2, 3 | 62 / 2 482, 323; 75 / 2 459, 319; 93 / 3 084, 406 | 73 / 966, 47; 71 / 1 007, 40; 81 / 979, 38 |
  | Wikipedia 1, 2, 3 | 103 / 9 428, 2 212; 103 / 9 483, 2 204; 103 / 9 338, 1 959 | 196 / 7 790, 201; 212 / 8 279, 197; 212 / 8 402, 201 |
  Every YouTube run was in the QUIET regime this time (61-95 builds off; playback never started, with or without the
  click), so no active-regime pair exists; QINC1j's quiet runs were +50-80 % with INCR=1, now -60 to -70 % (2.4-3.3 s ->
  0.94-1.02 s), and forced layouts with INCR=1 are 0.55-0.63 s vs 2.6-2.9 s off (QINC1i). Before the rebase (QINC1j
  base, same input): YouTube off 2 338-2 623 vs INCR=1 907-1 134 ms; Wikipedia INCR=1 3 312-3 414 ms (QINC1j: 3 852-4 199).
  INCR=1 vs main #460 (same flag, interleaved): YouTube 854 / 964 ms vs 4 865 / 5 071 ms builds (13 `restyled_share`
  worker frames per run on main, 0 here; engine restyle 0.88 s vs 4.9 s); Wikipedia builds 8 209 / 8 127 vs 7 218 / 7 391
  ms but forced 1 541 / 1 469 vs 3 769 / 3 366 ms (builds + forced -11 %: 9.6-9.8 s vs 10.8-11.0 s; engine restyle
  2.1 s vs 3.9 s). Attribution (temporary env toggles in one binary, removed; 2 runs each): the counter-walk skip saves
  1.1 s of restyle (3.2 -> 2.05 s), the build-memo start 0.45 s (2.5 -> 2.05 s), carrier filters + grouped walks + map
  build the rest (3.9 -> 3.2 s). The work moved from forced layouts into App builds: with identical layout counters
  (`boxes_remeasured` 138 016 in every run) `layout_ms` is 6.4 s with the walk skipped vs 6.0-6.1 s with it (main
  5.7-5.8 s), apparently the walk warmed the DOM / style memory the next relayout reads. The Wikipedia INCR=1 builds went from ~3.4 s to ~7.2-8.4 s with QINC1i itself
  (`layout_ms` 1.2 s -> 5.8 s, `boxes_remeasured` 138k, `miss_not_cacheable` 75k): proposed QINC1i5.
  `SCRATCHY_INCR_CHECK=1`: `yt`, `ytq`, Wikipedia one run each after the rebase and one each before: **0 mismatches**.
  RSS at the end: YouTube off 485-522 MB vs INCR=1 501-518 MB; Wikipedia off 339-342 MB vs INCR=1 ~350-365 MB.
  Proposed: QINC1j-ab1 (incremental invalidation map: per selector-list contributions cached across rule-list
  generations (`SelectorList::thin_arc_heap_ptr`), so a `<style>` splice costs its own selectors instead of ~25-30 ms
  for YouTube's 26.5k), QINC1i4 remainder (adopt the build's styles, not only its matches, on a restart).
  QINC1i5 DONE (Opus, 2026-11-20; `js_geom.rs` `refresh_engine`, `layout.rs` `memo_trim`, one line in
  `incr_layout::live_update`; test in `incr_forced_tests.rs`): the Wikipedia `SCRATCHY_INCR=1` App build regression of
  QINC1i (#460) was SHAPING, not the relayout's reach: the layout counters were the same before and after (138k
  `boxes_remeasured`, 75k `miss_not_cacheable`, 133-134 walks to the root also on QINC1j's runs: 132k / 66k / 117), only
  the time per re-measured box grew ~5x. Cause: `refresh_engine` kept the from-scratch forced layout's memo bracket
  (`memo_track_begin` ... `memo_sweep(cap, keep = 1)`: keep only the keys THIS layout used). A from-scratch layout uses
  every paragraph of the page; an engine update re-measures a few boxes, so each forced layout (~100 per Wikipedia run
  with the QYTC1a / QYTC1d / HiddenSnap / ATTR_SNAP skips bypassed) emptied the main thread's shape memo, and the next App
  frame's relayout (same thread, same memo) shaped every paragraph of its root walk again. Fix: the engine path no
  longer tracks / sweeps; `layout::memo_trim(cap)` bounds the memo instead (over the cap: drop the entries no kept shape
  cache shares (`Arc::strong_count == 1`), then clear if still over), after each engine forced layout and each Main
  frame relayout. The from-scratch path and the worker are unchanged. No style / box / layout semantics.
  Test: `qinc1i5_engine_forced_layout_keeps_the_shape_memo` (30 paragraphs, a from-scratch read, then 3 small writes
  read through the engine with the check-mode oracle off (`js_geom::NO_ORACLE` seam: its from-scratch layout refills
  the memo itself): >= 30 memo entries remain; fails on main with 0-1). Suite 1 716 passed (debug), WPT 333 / 293 no
  change (flag off and `SCRATCHY_INCR=1`), clippy 77. Fuzz harness (release, `SCRATCHY_FUZZ_KEEP_GOING=1`): 7 windows x
  3 000 cases (seeds 0, 10 000, ... 60 000): 0 mismatches.
  Gate (release, QINC1j's R55 input and scripts: own wire per run, own nested dwl 960x1006, `SCRATCHY_FRAME_STATS`, 6G
  scope, every run under `flock /tmp/scratchy-perf.lock`; main = 30a23cb, fix = the same + this change, both
  `SCRATCHY_INCR=1`, interleaved, order flipped per round; 1-min load 1.8 at the start (4.3 just before), 0.3 at the end):
  | run | main: builds n / ms, forced n / ms, `layout_ms`, lat_p95, RSS | fix: builds n / ms, forced n / ms, `layout_ms`, lat_p95, RSS |
  |---|---|---|
  | Wikipedia 1 | 211 / 8 352, 104 / 1 735, 6 734, 110 ms, 359 MB | 212 / 4 356, 104 / 1 240, 1 626, 43 ms, 323 MB |
  | Wikipedia 2 | 212 / 8 214, 99 / 1 794, 6 549, 114 ms, 377 MB | 212 / 3 639, 105 / 825, 1 402, 36 ms, 329 MB |
  | Wikipedia 3 | 212 / 8 315, 104 / 1 303, 6 420, 105 ms, 380 MB | 212 / 3 635, 105 / 839, 1 455, 36 ms, 326 MB |
  | YouTube watch 1, 2, 3 | 105 / 927, 91 / 1 021, 74 / 997; forced 540-597 ms; 477-511 MB | 66 / 1 055, 71 / 831, 70 / 1 008; forced 515-691 ms; 505-524 MB |
  Wikipedia App builds 8.2-8.4 s -> 3.6-4.4 s (QINC1j's #459 runs: 3.9-4.2 s; run 1 at 4.36 s came right after a load
  of 4.3), forced layouts 1.3-1.8 s -> 0.8-1.2 s, per-second input p95 105-114 -> 36-43 ms, RSS -35 to -55 MB (fewer
  duplicate paragraphs: the re-shaped ones no longer replace shared memo entries). YouTube watch (quiet regime every
  run): no change (builds 0.83-1.06 s vs 0.93-1.02 s; RSS +10-30 MB, within run-to-run spread). `SCRATCHY_INCR_CHECK=1`
  (fix): Wikipedia 3 runs, YouTube 3 runs: **0 mismatches**.
  Left: the main thread's memo is OFF until the page's first script forced layout (`SHAPE_MEMO_ENABLED` is set only by
  `js_geom`), so on a page without forced layouts every Main frame re-shapes what it re-measures: proposed QINC1i6.
  QINC1i4 DONE (Opus, 2026-11-20; `src/incr_style.rs`, the record bracket in `style.rs` `resolve_style_for`, two
  counters in `perf.rs`; test in `incr_style_tests.rs`, fuzz knob in `incr_fuzz_tests.rs`): a restart (the full shadow
  update right after a from-scratch build: first layout, new rule-list generation, viewport width, Global log) no
  longer cascades every element again: it ADOPTS the build's styles. Every shadowed build now records (not only in
  check mode), per `(node, memo slot)`, the style it resolved (boxed) and whether THAT resolve read the viewport
  height (`record_begin` clears `VIEWPORT_H_READ` for the one resolve, `record` ORs the old value back, so the build's
  global flag is unchanged; the per-element bit is the engine's `reads_vh`). `Engine::style_of`: in a full update
  with a record, `SCRATCHY_INCR=1` moves the recorded box into the element table (no second copy; peak memory as
  before); check mode (tests, `SCRATCHY_INCR_CHECK=1`) cascades anyway and reports a difference (`adopted <slot>
  style field ...` / `reads vh`), so the old compare still holds. Matches come from the build's memo (#461); the rest
  of the per-element state (ambient, box owner, `reads_parent` from the match, animated, child keys, counters walk)
  is derived as before (cheap). The record map is pre-sized from the previous build's. Counters
  `incr.restyle_adopted` / `incr.restyle_resolved` (styles the engine cascaded itself, any update). Box tree: a
  restart already ADOPTS the build's tree (`BoxEngine::start` clones it, ~7 ms on Wikipedia); laid tree: a forced
  restart builds without measuring and the engine lays out once, an App Worker frame lays out once on the worker
  and the engine adopts it (QINC1j), so no restart lays out twice; nothing to change there.
  Wikipedia restart, temporary `Instant` diag (removed): engine visit 20-23 ms for 10 000 adopted styles (match
  memo + bookkeeping, ~2 us per element), counter walk 6-8 ms, box start 7-8 ms; main re-cascaded at ~65 ms.
  Tests: `qinc1i4_restart_adopts_the_builds_styles` (first build and a new rule-list generation: `resolved` 0,
  `adopted` >= 10 incl. `::before` / `::after` / `::marker`; the adopted vh bit makes a height change re-cascade the
  vh reader alone; the same with the verification off as `SCRATCHY_INCR=1` runs it; an attribute write adopts
  nothing). Fuzz (release, `SCRATCHY_FUZZ_KEEP_GOING=1`): 7 windows x 3 000 cases (seeds 0 ... 60 000, adopted styles
  verified) + 3 windows with `SCRATCHY_FUZZ_ADOPT_VERIFY=0` (adopted unverified; box / laid compares still run):
  0 mismatches. Suite 1 716 passed (debug), also with `SCRATCHY_TEST_INCR_FORCED=1`; one run of the plain suite
  failed `qinc1i_forced_layouts_see_the_windows_images` ("images are per document": the window-image key is a
  document's node address, which a new document can reuse), passed alone and on the rerun: proposed QFLK6. WPT
  333 / 293 unchanged, flag off and `SCRATCHY_INCR=1`. Clippy 77.
  Gate (release, headless `--perf-report --budget-ms 20000`, own wire, Wikipedia Rust 20 s, YouTube results
  (`?search_query=rust`) 30 s and watch G1AeBgalFZk 30 s, default UA; main 30a23cb vs this, both `SCRATCHY_INCR=1`,
  interleaved, order flipped per round, every run under the perf lock and the 6G scope; 1-min load 0.1-3.0;
  forced layout ms total (layouts / restarts), `incr_shadow` phase ms, engine restyle ms):
  | page | main 30a23cb, 3 runs | this, 3 runs |
  |---|---|---|
  | Wikipedia forced total | 742 (5/3), 734 (5/3), 832 (5/3) | 532 (4/2), 526 (4/2), 842 (5/3) |
  | Wikipedia `incr_shadow` / restyle | 215 / 195, 210 / 190, 246 / 225 | 81 / 69, 77 / 65, 148 / 129 |
  | Wikipedia per restart: shadow, restyle, `build_children` | 72, 65, 50; 70, 63, 50; 82, 75, 58 | 41, 35, 63; 39, 33, 61; 49, 43, 72 |
  | YouTube results forced total (layouts) | 397 (41), 365 (36), 459 (36) | 358 (37), 385 (41), 451 (36) |
  | YouTube watch forced total (layouts / restarts) | 1 253 (170/5), 812 (119/4), 885 (119/4) | 829 (120/4), 825 (120/4), 881 (118/4) |
  Per Wikipedia restart the shadow stage is 30-35 ms cheaper (~70 -> ~40 ms: the restyle halves, 10 000 styles
  adopted, ~108 resolved), the build ~12 ms dearer (the record: one box + insert per resolve), net ~-20 ms of
  ~250 ms. The page loads with 2 OR 3 restarts (new stylesheets arriving), which moves its total more than the
  change does (runs 1-2 vs 3). What restarts cost: Wikipedia: the 1-2 non-restart layouts are 1-3 ms each, so
  restarts are ~99 % of the INCR=1 forced time before and after (734-832 ms for 3 restarts before; 526-532 ms for 2,
  842 for 3 after). Remaining per restart (this): build ~60 + counters precompute ~37 + shadow ~40 + the engine's
  from-scratch layout (`layout_full_ms` 245 for 2 restarts, 325 for 3: the first is cold). YouTube: restarts happen
  on the early, small DOM (adopted 309 styles over 3 restarts on results, 1 126 over 4 on watch): build + precompute
  + engine layout + shadow ~15-20 ms of 360-460 (results) and ~40 ms of 810-880 (watch), ~5 %. The YouTube INCR=1
  forced time is the INCREMENTAL restyle: engine restyle 230-300 ms of 360-450 (results, 3 900-4 000 styles
  resolved) and 550-590 of 810-880 (watch, 8 000-8 200 resolved), ~65 %: proposed QINC1i6. `_CHECK=1` (this binary,
  one run per page, every adopted style re-cascaded and compared): **0 mismatches** (Wikipedia 20 072 adopted, results
  309, watch 1 126). RSS at the end: Wikipedia main 277-287 vs this 287-288 MB; results 362-439 vs 365-434; watch
  488-544 vs 500-526 (run noise; an unboxed record was +20 MB on Wikipedia, hence the boxes).
  After the rebase onto QINC1i5 (#462): Wikipedia, 3 interleaved pairs vs main dc6ba98 (`INCR=1`, load 0.8-7.8):
  `incr_shadow` 193 / 221 / 152 ms (2 / 3 / 2 restarts) -> 130 / 81 / 111 ms (3 / 2 / 3), per restart 97 / 74 / 76 ->
  43 / 41 / 37 ms; forced totals 622 / 669 / 490 -> 636 / 449 / 570 (the restart count differs per run).
  Proposed: QINC1i6 (YouTube incremental restyle cost: ~65 % of INCR=1 forced time, ~60-70 us per resolved style
  incl. per-update fixed costs such as the counter walk; profile and cut), QINC1i4b (restart hand-over leftovers:
  `BoxEngine::start` clones the build's tree that the forced restart then drops: move it, ~7 ms + the drop per
  Wikipedia restart; the record's per-resolve box + insert, ~12 ms per restart: a Vec in resolve order), QINC1i4c
  (layout side, for QINC1i5's owner: the engine's from-scratch layout of a restart starts a new `ShapeCache`
  (`LayoutEngine::full`), the plain path keeps `GeomCache::shape_cache` across layouts; suspected part of the
  restart's 120-160 ms layout vs ~50 ms for a warm plain measure), QFLK6 (`qinc1i_forced_layouts_see_the_windows_images`
  keys window images by a reusable node address).
  QINC1i6 DONE (Opus, 2026-11-20; one line in `incr_layout::live_update`; test `qinc1i6_main_frames_use_the_shape_memo_without_forced_layouts`
  in `incr_frame_tests.rs`, fails without the change): `live_update` turns the main thread's `SHAPE_MEMO_ENABLED` on, so Main
  frames memoise from the first one on (bounded by the existing `memo_trim`). Flag off: unchanged (`live_update` is only reached
  with the engine). No style / box / layout semantics. Suite 1 717 passed (debug), clippy 77, fuzz harness (release) 3 windows x
  3 000 cases (seeds 0, 10 000, 20 000): 0 mismatches.
  Gate (release, own nested dwl 960x1006, own wire, `SCRATCHY_INCR=1` both, main = the same tree with the one line disabled,
  interleaved, order flipped per round, under flock; 1-min load 0.1-1.0 at each start; Wikipedia Rust = 18 wheel notches, 400-move
  sweep, 18 back; YouTube watch = play click + pointer over the player): App builds ms / forced ms
  | run | main | fix |
  |---|---|---|
  | Wikipedia 1, 2, 3 | 2 013 / 567, 1 993 / 886, 615 / 712 | 2 051 / 566, 2 032 / 579, 442 / 734 |
  | YouTube watch 1, 2, 3 | 2 076 / 6 149, 2 300 / 1 892, 2 241 / 6 252 | 1 611 / 1 317, 2 318 / 6 079, 2 320 / 6 302 |
  The runs are bimodal per page (a Wikipedia run with 13-23 builds instead of ~100, YouTube quiet vs active forced layouts), so
  pairs are only comparable within a regime: not slower in any pair (Wikipedia 100-103 builds: +2 % / +2 %; quiet YouTube 1.6 s
  vs 1.9-2.3 s; active 2.3 s vs 2.1-2.3 s). These pages DO have forced layouts, so the memo was already on after the first one:
  the change targets pages without them and the gain shows in the unit test's counter, not here. `SCRATCHY_INCR_CHECK=1`
  (fix): Wikipedia 2 runs, YouTube 2 runs: **0 mismatches**.
- **QINC1k [M]** (P2, Opus, measure only, docs; AFTER QINC1i, j, h2) Exactness sweep and gates (R60 section 12):
  `SCRATCHY_INCR_CHECK=1` headless + window on YouTube results (R58 recipe), YouTube watch (R50/R55 recipes), Wikipedia (R55),
  github.com, bbc.com/news, social.loupax.com/discover: 0 mismatches (each mismatch = a P1 fix item); then `SCRATCHY_INCR=1`
  vs main, 3 interleaved runs each under flock: forced layout ms, App builds ms, `getPlayerSize` median, worst preview job,
  hover latency; floors and refusal mix from the counters; go/no-go for the flip written into R60; files QINC1n if the floors
  matter.
  QINC1k DONE (Opus, 2026-11-21, measure only + docs + recipes; binary = release build of main 42b99fd, copied aside):
  **verdict: NO-GO for flipping the default now; GO once QINC1k1 and QINC1k2 (below) land and
  `tools/incr-gate/gate.sh check` / `gate.sh perf` show 0 mismatches and github.com not slower than flag off.**
  The YouTube gates are met by a wide margin, Wikipedia improves a lot but misses the -80 % builds gate (-62 %),
  github.com is a REGRESSION in the window (App builds +54 %, input p95 2x), and one sub-pixel mismatch remains.
  Recipes (durable, committed): `tools/incr-gate/` (`run.sh` one run, `gate.sh check|perf|headless` the series,
  `summarize.py` / `shotdiff.py` the tables and the screenshot compare, `hover.html` a script-free hover page,
  `abs-drift.html` the minimised mismatch); tools/README "Incremental-engine gate recipes". Every run: own wire
  (fresh cookies), own nested dwl 960x1006 (inject coordinates are fractions of 960x1006), `SCRATCHY_MEDIA_AO=null`,
  `SCRATCHY_FRAME_STATS`, the 6G scope, `flock /tmp/scratchy-perf.lock`; flag off vs `SCRATCHY_INCR=1` of the SAME
  binary, interleaved, order flipped per round, 3 rounds; 1-min load 0.03-2.18 at each run's start. Inputs: ytr =
  R58 (thumbnail 1 at 15 s, thumbnail 2 at 21 s, top bar at 27 s); ytw = R55 (play click at 13 s, 6 rounds x 8
  positions x 600 ms, parked); wiki = R55 (6 rounds x 8 links x 600 ms: general-purpose, performance, type safety,
  concurrency, memory safety, Talk, Edit, View history); github / bbc / social = raster sweep (40 ms per move),
  5 wheel notches, sweep, back; hover = 2 raster sweeps at 16 ms per move.
  **Regimes:** every window run of these recipes landed in ONE regime: YouTube watch played in all 6 runs (video
  frames painted 420-440 off, 963-973 with the flag), Wikipedia had 102-104 builds off / 211-213 on in all runs
  (no 13-23-build run). Starting the input after the load settles (12-15 s) and dwelling 600 ms per point instead of
  streaming moves is what keeps them there; `summarize.py` prints `vid` (playback) and the build count per run so a
  run in the other regime is visible.
  (1) **Exactness** (`gate.sh check`: `SCRATCHY_INCR=1 SCRATCHY_INCR_CHECK=1`, headless AND window per page per
  round, 7 pages x 3 rounds = 42 runs): **0 mismatches on YouTube results, YouTube watch, Wikipedia, bbc.com/news,
  social.loupax.com/discover and the hover page; github.com: 0 headless, 14 / 13 / 13 in the window runs**, all the
  same element and field: `incr-check: MISMATCH laid at root/positioned[64]=div.Wrapper__H1gdC8.Background__x8lac6:
  style.height: Some(Px(642.0005)) vs Some(Px(642.0))` (from scratch vs engine). The element is `position:absolute;
  inset:0; height:100%` (GitHub's `.Background__x8lac6` + `.Wrapper__H1gdC8`) inside a positioned box whose auto
  height is `cursor_y - content_y` in DOCUMENT coordinates (layout.rs block path, `content_h`): summed at y > 4096
  the f32 ulp is ~0.0005, so the same box laid at another y gets 642.0005 instead of 642.0, and `finalize_box_lengths`
  writes that into the percentage height. The engine shifts the clean subtree (keeps the height it measured at the
  old y); the from-scratch layout re-sums at the new y. Geometry compares with `EPS`, `style` fields compare exactly,
  so only the style field reports. It is the y twin of QINC1h1's finding (1) (x in `measure_mixed_inline`).
  Minimised: `tools/incr-gate/abs-drift.html` (20 rows of 32.1 px under a relative box with an `inset:0;
  height:100%` child; a timer moves it from y 100 to 4100.3 and back): headless check mode reports 2 mismatches
  (`642.00195` vs `641.9999`, both directions). Visible effect: none (< 0.003 px). Proposed QINC1k1 below.
  No panic and no crash in any of the 126 runs (42 check + 42 window perf + 42 headless). The social.loupax.com
  window runs end with a kill: the page handles `:` itself (its own vim-like UI), so the harness's `:q` never reaches
  scratchy and the window's `incr` counters line is not printed (diag lines still are; headless runs have them).
  (2) **Flag off vs `SCRATCHY_INCR=1`, window** (per run, 3 interleaved pairs; `builds` = App frame work on the
  main thread, `forced` = script forced layouts, p95 = median over the seconds with input of the per-second
  input-to-present p95, `gPS` = median over the seconds of the mean ms per `getPlayerSize` read, worst = longest
  callback after 15 s (the preview-start promise job on ytr), RSS = VmRSS at the end):
  | page | off: builds ms | INCR=1: builds ms | off: forced n / ms | INCR=1: forced n / ms | p95 off / on | other | RSS MB off / on |
  |---|---|---|---|---|---|---|---|
  | YouTube results (R58) | 5 054 / 4 720 / 4 620 | 886 / 900 / 966 (-81 %) | 91-99 / 4 313-4 805 | 103-156 / 570-749 | 29-51 / 39-49 | gPS 65.1 / 63.4 / 64.1 -> **5.7 / 4.1 / 4.7 ms**; worst job 844 / 847 / 830 -> **181 / 159 / 177 ms** | 457-512 / 481-514 |
  | YouTube watch (R55, playing) | 12 351 / 12 646 / 12 274 | 1 735 / 1 694 / 1 845 (-86 %) | 259-283 / 14 491-15 811 | 687-703 / 5 297-5 431 | 84-89 / 35-39 | gPS 80.6-85.0 -> 7.2-7.5 ms; video frames painted 420-440 -> 963-973 | 578-599 / 514-596 |
  | Wikipedia Rust (R55) | 9 335 / 9 454 / 9 481 | 3 454 / 3 569 / 3 589 (-62 %) | 21-23 / 2 507-2 838 | 105-106 / 802-972 | 206-214 / 36 | builds 102-104 -> 211-213 (no coalescing behind the worker) | 314-339 / 320-338 |
  | github.com | 11 380 / 11 700 / 11 523 | **17 991 / 17 457 / 17 570 (+54 %)** | 146-147 / 17 420-17 673 | 202-206 / 5 645-5 982 | **170-175 / 344-350** | builds + forced 28.8-29.4 s -> 23.2-24.0 s | 1 245-1 637 / 1 747-1 831 |
  | bbc.com/news | 590 / 610 / 595 | 185 / 185 / 184 (-69 %) | 4-5 / 142-171 | 6 / 58-84 | 670-671 / 210-228 | | 223-238 / 226-228 |
  | social.loupax.com/discover | 41 / 41 / 41 | 17 / 17 / 18 | 12 / 89-93 | 12-14 / 55-63 | - (hovers change nothing: no presents) | | 156-159 / 154-156 |
  | hover.html (no forced layouts) | 160 / 158 / 161 (+ worker 138-139) | 67 / 70 / 70 (+ worker 16) | 0 | 0 | **137-142 / 155-164** | 6-7 redraws/s for 62 moves/s in BOTH modes | 108-109 / 108-109 |
  **Headless** (`--perf-report --budget-ms 20000`, 30 s YouTube, 20 s the others, 10 s hover; forced n / ms, RSS MB):
  ytr off 30-36 / 602-893 vs 29-42 / 290-384 (RSS 375-402 vs 368-414); ytw 115-123 / 2 593-3 077 vs 115-117 / 794-815
  (452-460 vs 477-491); Wikipedia 4 / 557-571 vs 5 / 565-606 (3 restarts per run are all of it: QINC1i4; RSS 222-229
  vs 279-281, the kept trees); github 119 / 14 386-14 570 vs 188-199 / 10 080-10 739 (more layouts run in the same
  20 s; `incr_shadow` 8.4-8.9 s of it; RSS ~1 580 vs 1 355-1 764); bbc 14-15 / 384-407 vs 15 / 154-155; social 10 /
  73-75 vs 10 / 48; hover 0 / 0.
  **Gates (R60 section 12):** `getPlayerSize` median < 10 ms: MET (4.1-5.7 ms, was 63-65); worst preview job < 300 ms:
  MET (159-181 ms, was 830-847); App builds -80 % on the R55 recipes: MET on YouTube watch (-86 %), NOT on Wikipedia
  (-62 %; input p95 206-214 -> 36 ms, forced -65 %); 0 `incr-check` mismatches on 6 pages x 3 runs: NOT MET
  (github.com window, sub-pixel, QINC1k1).
  (3) **Refusal mix and floors** (window `incr` counters per run with the flag): App frames Current / Main / Worker:
  ytr 29-30 / 71-106 / 3-5, ytw 216-227 / 177-205 / 3-4, Wikipedia 0 / 207-210 / 3-4, github 0 / 64-65 / 2, bbc 0-1 /
  52-55 / 5, hover 0 / 132-133 / 1. Worker reasons: `first_layout` 1, `restyle_not_incremental` 1-4 (new rule-list
  generations), `adopt_refused` 0-2, `restyled_share` 0-1. Forced restarts: ytr 3, ytw 3, Wikipedia 2-3, github 0.
  Relayouts walked to the root: ytr 94-138, ytw 65-68, Wikipedia 133 (every Main frame of the page), github 218 (all),
  hover 36-37. Engine time per run (restyle / relayout / box update, ms): Wikipedia 1 934-2 006 / 1 421-1 455
  (+147-159 full) / 88-89; ytw 4 531-4 733 / 811-830 / 404-416; ytr 764-943 / 143-193 / 54-78; **github 4 325-4 621
  / 1 469-1 525 / 16 994-17 194**. The floors that matter: (a) github's BOX update, ~63 ms per update (17.1 s over
  ~270 updates; headless ~25 ms per forced layout, 317 boxes rebuilt per update on a ~3 000-element page: ~75 us per
  box), which turns its App frames into a regression: QINC1k2; (b) Wikipedia's Main frames all walk to the root
  (~6.8 ms relayout + ~9.3 ms restyle per frame): QINC1k3. The shift cost of absolute coordinates (R60 14.1) is not
  among them (`layout_shifted` is small everywhere), so **QINC1n is NOT recommended on these numbers**; its exactness
  side (translation invariance) is covered more cheaply by QINC1k1.
  (4) **Hover latency:** the script-free page shows the same ~140 ms input-to-present p95 with and without the flag
  (+15-25 ms with it), with only 6-7 redraws per second for 62 pointer moves per second in both modes; App frame
  work is 2-4 ms per second with the flag (off: ~8 ms per second on the main thread + ~140 ms per run on the worker),
  so the cadence is set outside the engine (proposed QHOV2, not an incr item). The flag bypasses the QY53n hover
  patch / skip paths (`hover.patched` / `skipped` 0 with it).
  (5) **Visual differences** (`shotdiff.py`, a `pre` screenshot before any input and an `end` one after it, each
  off/on pair compared with the next off run as the noise floor): Wikipedia, github, hover: byte-identical pixels in
  all 3 pairs. bbc: only news items that changed between loads (also off vs off). YouTube watch: thumbnails / player
  state (off vs off differs as much); 2 of 3 flag-on runs showed YouTube's player error screen at 54 s (an off probe
  run also hit it once). YouTube results: in 2 of 3 flag-on runs (and 1 of 3 check runs) the Shorts shelf came right
  after the first result with five narrower items; off runs (0 of 3) and the third on run put it after the second
  result; the content is live, so not attributed. **social.loupax.com/discover: a real, repeatable difference (3 of 3
  pairs, off vs off identical):** the page's own line-number gutter numbers the lines beside the first article image
  4-19 with the flag off (the text after it starts at 20, `1/245`), and 3 / 4 with the flag on (`1/244`). The page
  computes the gutter from geometry reads; with the flag, forced layouts see the window's decoded image sizes (QINC1i),
  without it they do not, so the flag-on gutter (image = one line, the text that follows = line 4) looks like the
  intended one. The user should confirm (their site).
  (6) **Memory:** see the RSS columns: no growth with the flag on YouTube (results 481-514 vs 457-512 MB, watch
  514-596 vs 578-599), bbc, social and the hover page (within run spread); Wikipedia none in the window (320-338 vs 314-339), +55 MB
  headless (the kept style / box / laid trees); github +110-500 MB in the window (1 747-1 831 vs 1 245-1 637 MB; its
  off runs alone spread by 390 MB).
  Proposed (the coordinator files them):
  - **QINC1k1 [S]** (P1, area incr-new / layout comparator; AFTER nothing) Translation invariance in y for
    percentage heights: `tools/incr-gate/abs-drift.html` + github.com report `style.height` 642.0005 vs 642.0 (auto
    height summed in document coordinates, f32 ulp ~0.0005 at y > 4096, written into the child's resolved `height`
    by `finalize_box_lengths`). Options: compare resolved-length style fields in `cmp_laid` with the geometry `EPS`
    (the rect already compares that way; the engine's value is the more exact one), or sum block content heights
    relative to `content_y`. Test: the fixture as a check-mode test (0 mismatches), fuzz seeds with fractional row
    heights at y > 4096. Gate: `gate.sh check 3 github` 0 mismatches.
    QINC1k1 DONE (Opus, 2026-11-21; `layout.rs` content heights + `Ck::y_sensitive`, `incr_layout.rs` `try_hit`,
    `paint.rs` `fill_rect`, counter `incr.miss_y`): **translation invariance in y, an engine guard where layout
    itself is not invariant, and paint edges that ignore the rounding noise.** (a) The auto content height is now
    summed RELATIVE to the content box (`ry`, advanced by the same deltas as `cursor_y`, which still places the
    children unchanged) in the block arm of `measure_inner`, `measure_table`, `measure_grid` and `measure_flex` (row
    and column return their items' bottom edge relative to `content_y`; only `column-reverse` still measures from
    the laid boxes); mixed inline content already was relative. From-scratch output changes only in the last bits
    of those heights. (b) Floats stay in document space (`Floats`), so a BFC root holding floats, and a flow
    position taken from a float (clearance, float avoidance), are not invariant: such a box records
    `Ck::y_sensitive` (via the `layout::Y_SENS` counter, which a cache hit on a y-sensitive box bumps too, so its
    ancestors inherit it) and `try_hit` reuses it only at the same y (else `miss_y`, counted in `miss_key` too), the
    y twin of QINC1h1's `x_sensitive`. The Q49 float-hoist re-lay keeps its relative start. (c) `fill_rect` counts an
    edge within 1e-3 px below a pixel boundary as on it (`PX_SNAP`): with (a) alone, bbc.com's `bottom: 0` cookie
    banner ended at 484.99957 instead of 485.0 (its height is now the exact relative sum, its position still a
    shift at document-space magnitude) and truncation dropped its last row, a 1-row difference in every flag-off
    screenshot pair; main got 485.0 only because both values carried the same rounding. Not chosen: an `EPS`
    compare of resolved-length style fields (would hide the drift, not remove it).
    (d) Found by the new fuzz after main's QINC1k4 added `calc()` margins: Q48d's clearance test read the cleared
    box's not yet finalized `margin-top` with `px()` (a debug assertion; 0 in release, so `margin-top:10%` got
    clearance it should not); it now `resolve`s it against the content width
    (`qinc1k1_clearance_resolves_a_percentage_margin_top`).
    Results (release builds; "main" = origin/main 34241f2 built from the same tree, "main+snap" = that with (c)
    only, so main+snap vs branch isolates (a)/(b)): `abs-drift.html` headless check 2 mismatches -> 0
    (`layout_shifted` 2 -> 3: the engine no longer restarts after a mismatch); `gate.sh check 3 github` **0
    mismatches in 6 runs** (3 window + 3 headless; a control run of the 0bbbd0d main binary in the same session: 13
    in its window run, as in QINC1k; re-run on the final rebased binary 40d114b: 0 again, `miss_y` 0); `gate.sh check 1` ytr ytw wiki bbc social hover: 0 mismatches in 12 runs;
    `miss_y` = 0 on all 7 pages (no reuse lost: github headless 9 488-9 624 hits / 4 058-4 133 shifted per run, window
    ~16 000 / ~5 500; ytr 2 019, ytw 2 517, Wikipedia 219 headless). Fuzz: default window x 7 (`SCRATCHY_FUZZ_SEED` 0..6000, 1 000 each) 0 mismatches; new
    `qinc1k1_fuzz_fractional_heights_far_down` (spacer `#ys` moved by the mutations, a positioned auto-height box of
    fractional rows with an `inset:0; height:100%` child, float / clear / `%`-height declarations mixed in): main
    code 378 of 1 000 cases fail, this branch 0 of 7 000. WPT 333 / 293, no change. Flag-off screenshots
    (`shotdiff.py`, `pre` and `end` shots): main+snap vs branch x 3 rounds on bbc.com/news, github.com, Wikipedia
    and hover.html: byte-identical in every same-size pair (one Wikipedia pair differed in window size); main vs
    branch x 2 rounds on bbc, github, Wikipedia: byte-identical. (The nested dwl got only 960x144-178 px of the
    desktop in these series, so they cover the top of each page plus bbc's bottom-fixed banner; an earlier 960x1006
    series of the (a)-only build showed github / Wikipedia / hover identical and bbc's 1-row banner gap.)
    Found, NOT fixed (also on main): (1) far down a page (> ~20 000 px) the f32 ulp (0.002-0.008) lets a shifted
    box's geometry differ from a fresh layout by more than `EPS` (`ink.b` 65733.89 vs 65733.91): the
    absolute-coordinate limit (R60 14.1, QINC1n); the fuzz keeps its pages below ~16 000 px. (2) A fuzz case with an
    8 px `style.width` difference (50 % of 1008 vs of 1024) after moving an `li.c1.c4` under `div .c1{width:50%}`,
    not related to y: full page and steps in `tools/incr-gate/fuzz-width-504-512.txt` (not reproduced by a
    hand-written minimal test). Proposed (new item, the coordinator names it) **[S]** (P2, area incr-new): minimise
    and fix it. (Fixed by QINC1k10, below.)
  - **QINC1k2 [M]** (P1, area incr-new (incr_box); AFTER nothing) github.com box-update floor: with
    `SCRATCHY_INCR=1` the incremental box update takes 17.0-17.2 s per window run (~63 ms per update, ~75 us per
    rebuilt box; restyle 4.3-4.6 s, relayout 1.5 s), so App builds are +54 % and input p95 2x vs flag off. Profile it
    (a perf run without call graphs showed SipHash `RandomState` hashing ~3.5 % of the main thread; no attribution yet),
    then cut. Gate: `gate.sh perf 3 github`: App builds and p95 not above flag off.
  - **QINC1k3 [M]** (P2, area layout; AFTER QINC1k2) Wikipedia Main frames walk to the root every time (133 of
    210 per run) at ~16 ms engine time each (restyle ~9.3 + relayout ~6.8): find why the bottom-up walk does not stop
    (counters `layout_up_reasons` per frame) and whether the restyle can stay inside the hovered subtree; goal the
    -80 % builds gate (3.5 s -> <= 1.9 s per run).
  - **QHOV2 [S]** (P3, area app; not incr) hover cadence: on `tools/incr-gate/hover.html` scratchy presents 6-7
    frames per second for 62 pointer moves per second (input-to-present p95 ~140 ms) with the flag on or off and
    ~2-8 ms of frame work per second: find the throttle (pointer-move coalescing, frame pacing in the nested dwl, or
    the injector's roundtrips) and decide whether it is intended.
  - Not filed: QINC1n (see (3)).
  QINC1k2 DONE (Opus, 2026-11-21; `images.rs` `canonical_svg_ids` / `decode_inline_svg` / `INLINE_SVG_CACHE`):
  **the github.com box-update floor was inline-SVG rasterization, not the box engine.** A frame-pointer
  `perf record -g` of the window recipe (`profiling` build with `-C force-frame-pointers=yes`; the earlier dwarf
  profile could not unwind the main thread) put ~23 % of ALL samples under `incr_box::build_hooked` ->
  `build_box_for_element_body` -> `images::resolve_inline_svg` -> `rasterize_svg` -> resvg pattern fill (tiny-skia
  `highp::bicubic` 15.5 % self); temporary timers (removed) confirmed it: the builder (`build_children`) was 99.5 %
  of `box_ms`, harvest / renumber / image-table work < 0.3 %. Cause: github.com's card backgrounds are inline
  `<svg>` grids (`<pattern id="background-grid-pattern-_r_3d_">` + gradient, React `useId`) that the page REMOUNTS
  ~50 times per run (7 per set), each mount with a new id and so new markup, and every box build of an inline
  `<svg>` parsed and rasterized its serialized markup again. With the flag the grids are 956 x 647 / 896 / 1 315
  (the engine's geometry: 350 rasterizations, 16.5 s per run, 34-73 ms each); flag off the page sized them
  956 x 80 (289 rasterizations, 3.7 s) — the two modes give the page different geometry for those cards (see
  QINC1k4 below), which is why only the flag paid 17 s. Fix (engine performance only, no style/box/layout change):
  inline-SVG rasterizations go through `INLINE_SVG_CACHE` (key = serialized markup incl. the substituted
  `currentColor`; 1 024 entries / 64 MB of markup + pixels, cleared when over; `inline_svg_cache` in the
  perf-report memory list: 66 entries / 5.7 MB on github.com), and `canonical_svg_ids` first renames the
  fragment's own ids to `scratchy-svg-id-N` in definition order (a bijection; the renamed markup is what gets
  rendered; markup left unchanged when it has `<style>`, an id outside `[A-Za-z0-9_.:-]`, the prefix already, or an
  id still mentioned after renaming `id="x"` and `#x` references inside tags). Rasterizations per github run:
  350 -> 7 (flag) / 289 -> 5 (off). Boxes sharing identical markup now share one `Arc<DecodedImage>`.
  **Window gate (`gate.sh perf 3 github`, release build of this branch, 1-min load 5.0-15.7 at the runs' start):**
  | run | builds n / ms | forced n / ms | p95 | box ms | RSS MB |
  |---|---|---|---|---|---|
  | off 1 / 2 / 3 | 87 / 7 558, 87 / 7 601, 78 / 7 524 | 174 / 18 535, 175 / 18 564, 155 / 18 818 | 92 / 91 / 107 | - | 1 400 / 1 805 / 1 325 |
  | INCR=1 1 / 2 / 3 | 112 / 2 287, 79 / 2 174, 118 / 2 379 | 295 / 8 265, 180 / 5 300, 282 / 8 552 | 22 / 23 / 23 | 867 / 675 / 886 | 1 471 / 1 217 / 1 515 |
  vs #465: flag-on App builds 17.5-18.0 s -> **2.2-2.4 s (-69 % vs flag off, was +54 %)**, input p95 344-350 ->
  **22-23 ms (off 91-107)**, box update 17.0-17.2 s -> 0.68-0.89 s per run (~2.5 ms per update, 339 updates);
  flag off also improved (App builds 11.4-11.7 s -> 7.5-7.6 s, forced layouts no longer rasterize the grids
  either). RSS: no growth (flag on 1 217-1 515 MB vs off 1 325-1 805). **Other pages, 1 interleaved round**
  (builds ms off / INCR=1; p95 off / on): YouTube results 4 849 / 1 212 (p95 13 / 52, gPS 84.1 / 18.2 ms, this run
  had 37 reads; #465 4.1-5.7), YouTube watch (playing) 11 761 / 1 310 (93 / 18, gPS 6.1 ms), Wikipedia 2 327 / 611
  (both in the 24-25-build regime, not the 102-104 one of #465), bbc 784 / 202 (347 / 206), hover 203 / 127
  (134 / 145): all within or better than #465's table except ytr's gPS median in that single run (load 5.9);
  re-run at load 0.3-2.0, INCR=1, this branch vs base b924d48 alternated x2: gPS 5.3 / 5.1 vs 4.3 / 5.0 ms, builds
  1 053 / 950 vs 906 / 904 ms (more reads and frames in run a: 159 vs 99-107 forced), worst job 185 / 161 vs
  186 / 165 ms: same as #465 (gPS 4.1-5.7, builds 886-966). **Exactness:** `gate.sh check 1 github` twice: headless 0 mismatches; window
  35 / 53 lines, all the known QINC1k1 `style.height` 642.0005 one except, in the FIRST window run only, 20 style
  mismatches `slot after/before field width: ... Calc(.., (956.0, 312.0)) vs (956.0, 485.0)` on footer links and
  carousel indicators (pseudo-element calc carrying a viewport-like pair; not reproduced in 2 more window check runs
  of this binary, base binary 1 run: QINC1k1 only; this change touches no style code) -> QINC1k4. Fuzz harness
  (`SCRATCHY_FUZZ_SEED` 0..6000 x 1 000, release): 0 mismatches. Full suite, WPT (333 / 293, no change).
  Proposed:
  - **QINC1k4 [S]** (P2, area incr-style; AFTER nothing) An intermittent window check-mode style mismatch on
    github.com: `slot after field width: fresh Some(Calc(Sub(Sub(Pct(1.0), Px(0.0)), Px(0.0)), 14.0, 16.0,
    (956.0, 312.0))) vs incremental ...(956.0, 485.0)` (footer `a.Primer_Brand__Link-module__Link--default`, 18 lines,
    `why: ... full true`) and `slot before field width` (CarouselPagination indicator, 2 lines), seen in 1 of 3
    window check runs (1-min load 14.4). The pair in the calc looks like the viewport (vw, vh) the calc was
    resolved against: find which vh the pseudo slot captured, reproduce (a resize during a check run?), fix.
  - **QINC1k5 [S]** (P2, area geom / incr; measure first) github.com's card grids get height 80 px (the page's own
    measurement) with the flag off and 647-1 315 px with it: which is right? (R60 QINC1k (5) saw a similar flag-on /
    flag-off geometry difference on social.loupax.com: forced layouts with vs without the window's decoded image
    sizes.) Screenshots of the viewport are identical in both modes, the grids are lower on the page.
  QINC1k5 DONE (Opus, 2026-11-21, measure + explain only, no engine / layout change; binary = release build of
  main ce088f9, copied aside; reference = the local `google-chrome --headless=new --window-size=960,1006
  --dump-dom`, nothing installed): **both differences are the intended QINC1i behaviour and the flag-on
  geometry is the CORRECT one (matches Chrome); flag off is the pre-existing QJ4 limit "no decoded image sizes
  in script geometry".** Neither is a new flag-on bug; one flag-on timing gap and two pre-existing bugs (both
  modes) were found on the way (proposed items below). **Not a blocker for QINC1l.**
  Mechanism (both pages): script forced layouts lay `<img>` out from the window's decoded image cache with the
  flag (QINC1i, `incr_engine::sync_window_images`); flag off they get an empty map (`js_geom.rs` from-scratch
  path), so an `<img>` without `width` / `height` attributes is never its decoded size for script, not even
  long after `load`: it is the undecoded placeholder, a line-height-tall box (600 x 21 as a block, 0 x 21 as
  a flex item, in the fixture; Chrome: 0 x 0 for `alt=""` before load, 400 x 300 after). The painted layout
  is the same in both modes (App builds always had the images): only what script reads differs.
  Minimised: `tools/incr-gate/img-geom.html` + `img-400x300.png` (`run.sh window MODE imggeom TAG`; a
  `data:` URL would not show it: every layout decodes those inline, flag or not). Probes at script run, at
  the image's `load`, in the rAF after it, at 2 s (window, 2 runs per mode + 1 check-mode run, 1-min load
  3.8-14.3; check mode 0 mismatches):
  | probe | Chrome | flag off | `SCRATCHY_INCR=1` |
  |---|---|---|---|
  | A img rect at `load` | 400 x 300 | 600 x 21 | 600 x 21 (2 / 2 runs: STALE) |
  | A img rect in the rAF after `load` | 400 x 300 | 600 x 21 | 600 x 21 / 400 x 300 (racy) |
  | A img rect at 2 s; text below at y | 400 x 300; 398 | 600 x 21; 119 | 400 x 300; 398 |
  | B card rect at 2 s (github.com Product markup) | 700 x 350 | 700 x 71 | 700 x 350 |
  | B grid `<svg>` at mount (from the card's rect) | 700 x 50 (not loaded yet) | 700 x 80 | 700 x 80 |
  | B ResizeObserver entry; grid `<svg>` after it | 698 x 348; 698 x 349 | 0 x 0; removed | 0 x 0; removed |
  (1) **github.com card grids** (`BackgroundGrid` in the page's bundle): the `<svg>` `width` / `height`
  ATTRIBUTES are computed by script, `fI(w, h)` = cells of `w / ceil(w / 10)` px, `ceil(h / cell)` rows, from
  the parent `.Product__JCupxb`'s `getBoundingClientRect()` at mount and then from its ResizeObserver
  `contentRect`. The card is a flex column whose `.Content__TWy-bF` (padding 24, `flex: 1 0 auto`) holds an
  `<img loading=lazy>` without size attributes (`features-*-light-*.webp`) or the hero `<video>`; the grid
  itself is `position:absolute; inset:0` (no feedback). Flag off: 2 + 48 + the 21 px placeholder = 71 ->
  rounded up to cells = **80**; flag on: the image's laid height + 50 = **647-1 315**. Chrome (945 px wide
  layout viewport): grids 945 x 726 / 756 (hero), 765 x 725 / 725 / 536 x 5 (feature cards): hundreds of px,
  so flag on is right and 80 is the QJ4 limit. The flag-on heights were not re-measured per card here
  (QINC1k2's 647 / 896 / 1 315 at 956 wide vs Chrome's 765-wide feature cards: a painted-layout width
  difference present in BOTH modes, not attributable to the flag; proposed QINC1k8). **Pre-existing in both
  modes:** scratchy's ResizeObserver (QY6g stub) delivers ONE entry with the `width` / `height` ATTRIBUTE size,
  0 x 0 for a `<div>`, so `fI(0, 0)` -> cell 0 -> React drops the `<svg>` right after every mount: in scratchy
  the grids exist only between a mount and the next timer tick (that is what QINC1k2's ~50 rasterizations per
  run were), and the card backgrounds stay empty; Chrome keeps a 698 x 349 grid in the fixture. Proposed
  QRO1 below.
  (2) **social.loupax.com/discover gutter** (`internal/web/js/vim.js` `buildGutter`, read-only): an
  `article img` / `.player` / `h2` is a "zone" = ONE gutter row stretched to its measured height; between
  zones the rows are 1lh (21 px) and the gutter's top and row heights come from script geometry, painted
  over the real layout. Flag off the first preview image measures 21 px tall (the placeholder), so it is one
  21 px row (line 3) and lines 4-19 are 1lh rows laid where script thinks the following text is, i.e. beside
  the painted (~360 px) image; the text starts at 20. Flag on the zone is the decoded height: line 3 is the
  image, the text starts at 4. Chrome (same page, dump of the gutter): line 2 = the `h2` (47 px row), line 3 =
  the image (360 px row), text from 4 = flag on. The page author's comments say the same ("Each is one
  line, whatever it actually looks like").
  **Flag-on timing gap (not a regression vs off, which never sees the size):** `App::handle_image_fetched`
  inserts the image into `image_cache` and fires `load` (`image_settled`) BEFORE the next
  `rebuild_layout` hands the cache to the engine (`sync_window_images`, Q35b2's debounced relayout), so a
  forced layout in a `load` handler sees the old size in 2 / 2 runs and one in the next rAF in 1 / 2 (Chrome:
  the new size in both). vim.js rebuilds its gutter in the rAF after each preview `load`; on the live page
  the flag-on gutter came out right in 3 / 3 runs of #465 (a later rebuild saw the sizes), but that is
  timing. Proposed QINC1k6.
  Pinned: `incr_forced_tests::qinc1k5_card_height_from_the_windows_image` (github.com's card markup:
  engine 700 x 350 = Chrome, flag off < 100); `qinc1i_forced_layouts_see_the_windows_images` already pins
  the plain `<img>` case and `js_observers_tests::qy6g_resize_observer_one_initial_entry` the 0 x 0 entry.
  Proposed (the coordinator files them):
  - **QINC1k6 [S]** (P2, area app (+ incr_engine); AFTER nothing; not a layout change) Hand a newly decoded
    image to the engine before its `load` fires: in `App::handle_image_fetched`, call
    `incr_engine::sync_window_images` (and note the ambient change, already done) right after
    `image_cache.insert`, before `image_settled`, so a forced layout in a `load` handler or the next rAF
    sees the decoded size (Chrome does). Test: an App-level test with the flag: image arrives, the `load`
    handler reads the img rect = decoded size; `tools/incr-gate/img-geom.html` A.load / A.raf rows = 400 x
    300 in 3 / 3 window runs. Flag off unchanged.
  - **QRO1 [M]** (P2, area jsdom / geom; AFTER QINC1l (needs real geometry for script)) ResizeObserver with
    real sizes: the initial entry and later entries from the layout (content / border box per the `box`
    option), delivered after layout when an observed size changed (spec "gather active observations" at
    each rendering update), instead of the `width` / `height` attribute size (0 x 0 for most elements).
    github.com's card grids (and every React `useResizeObserver` layout) drop their content on the 0 x 0
    entry today. Test: `img-geom.html` B rows (698 x 348 entry, svg kept), the QY6g tests updated to the
    new semantics (behaviour, not deleted). Gate: github.com card backgrounds visible in a window
    screenshot; perf: RO delivery cost on YouTube / github in `--perf-report`.
  - **QINC1k7 [S]** (P3, area layout / images; AFTER the QINC1 freeze) An `<img>` with no decoded image yet
    and no size attributes is laid out as a line-height-tall box (600 x 21 as a block, 0 x 21 as a flex
    item); Chrome gives 0 x 0 for `alt=""` (HTML rendering 15.4.3: an image that "represents nothing") and
    an inline alt-text box otherwise. Affects script geometry before load (both modes) and flag-off script
    geometry forever. Check Chrome / Firefox for the non-empty-alt case first.
  - **QINC1k8 [S]** (P3, area layout; measure first) github.com feature-grid cards are 956 px wide in
    scratchy (QINC1k2's grids) and 765 px in Chrome at a 960 px window (hero 945 in both): find which rule
    (grid / container / max-width) differs. Both modes; painted layout.
  QINC1k4 DONE (Opus, 2026-11-21; `incr_style.rs` element visit): **the engine dropped the pseudo slots'
  viewport-height read flag.** `Entry::reads_vh` (which elements a height-only change re-cascades) was the
  ELEMENT slot's flag only; `style_of` returned each `::before` / `::after` / `::marker` / first-line /
  first-letter slot's flag too, and the visit discarded it (`.0`). A `%`-bearing `calc()` captures the whole viewport
  pair at cascade time (`BoxLength::from_length` -> `Calc(.., current_viewport())`, noting a height read), so a
  footer link's `::after { width: calc(100% - 0px - 0px) }` read the height while the link itself did not: when the
  nested window's height moved 312 -> 485 between two updates (most likely the nested window settling, that run was at load 14.4), the from-scratch
  build cascaded the pseudo at 485 and the engine kept the 312 one. Not adoption-specific (the QINC1i4 record keeps
  each slot's flag correctly; an incremental cascade lost it the same way). Fix: the element's flag is the OR of
  its own and its eligible pseudo slots' reads. Repro: `qinc1k4_pseudo_calc_viewport_height_change_recascades`
  (a `%` calc on `::before`, `%` + `vh` on `::after`; height changes right after a verified restart, after an
  incremental re-match, and after a restart adopted without verification) failed with exactly the github
  message shape (`slot before field width: fresh Calc(.., (800.0, 312.0)) vs incremental (800.0, 600.0)`).
  Fuzz: viewport-height steps (312 / 485 / 700 / 900, 30 % of steps) and a second sheet of `vh` / `%`-calc rules
  on elements and pseudo slots (half the seeds), both from a second per-seed generator so every existing page,
  mutation and `REGRESSION_SEEDS` case is unchanged; without the fix the debug window fails on 8+ seeds; with it
  `SCRATCHY_FUZZ_SEED` 0..6000 x 1 000 (release) + the default window + `SCRATCHY_FUZZ_ADOPT_VERIFY=0`: 0
  mismatches. (A vh-rule candidate `margin-left: calc(10% - 1vh)` was left out of the fuzz: on an inline element
  it trips a pre-existing debug assertion in the from-scratch builder, `images.rs` QW4 inline spacers call
  `BoxLength::px()` on a `%` / `calc` horizontal margin or padding; release resolves it as 0 -> QINC1k9 proposed.)
  **Gate** (release build of this branch, 1-min load 3.4-12.8): `gate.sh check 3 github`: headless 0 / 0 / 0, window
  1 / 34 / 28 lines, every one the known QINC1k1 `style.height 642.0005` line, 0 slot mismatches; `gate.sh check 1`
  ytr ytw wiki bbc social hover: 0 mismatches headless and window. social's window run ends `quit=killed` (`:q`
  not honoured within 10 s, twice) with this branch AND the installed main binary (ce088f9): pre-existing.
  Full suite, clippy 77, WPT 333 / 293 (no change).
  - **QINC1k9 [S]** (P3, area images / box builder; correctness, needs the QINC1 freeze's OK) `images.rs`
    `collect_real_nodes` (QW4 inline start / end spacers) calls `BoxLength::px()` on an inline element's horizontal
    margin / padding, which is a `Percent` / `Calc` for `margin-left: 10%` or `calc(10% - 1vh)`: debug builds panic
    (`unresolved percent`), release treats it as 0 px. Resolve against the containing block width (CSS: inline
    margins / padding percentages refer to the containing block's inline size).
- **QINC1l [S]** (P2, Sonnet; areas app + geom (one-line defaults) + NOTES.md; AFTER QINC1k + the user's OK) Flip the default:
  `SCRATCHY_INCR` defaults to 1; `SCRATCHY_INCR=0` = the old path; `cfg(test)` keeps comparing with the from-scratch path;
  NOTES.md section "Incremental style + layout engine" (data model, marks, kill switch, check mode).
- **QINC1m [M]** (P3, Sonnet; areas geom + jsdom + app + style + hover_patch.rs; AFTER QINC1l and >= 3 days of the user's use with
  no incr regression) Delete what the engine supersedes (R60 section 1 table): `Skel`/`style_write_in_place`/
  `style_write_delta`/`IMP_GEO`/`GEOM_SHIFTS`, `ClassDeps`/`class_write_pre/post`/`PROBE_BUDGET`, `HiddenSnap`/
  `children_write_pre/post`, `ATTR_SNAP`/`attr_write_unread`, hover_patch.rs, `KEEP_MATCH`/`LAST_MATCH`/`stash_last_match`/
  `with_kept_build`/`hover_match_diff`/`set_kept_match`, the per-build `MATCH_MEMO`, `RENDERED` if unused. Their tests are
  KEPT (rewritten against observable behaviour where they called a deleted function: same pages, same asserted rects /
  pixels / layout counts). `SCRATCHY_INCR=0` then means "from scratch every time" (slow, correct). Gate: full suite, WPT,
  clippy count does not rise, R58/R55 recipes not slower than after QINC1l.
- Optional, filed only on QINC1k's numbers: **QINC1n [L]** relative-coordinate shared fragments (R60 14.1); **QINC1o [M]**
  damage-rect repaint from the touched-roots list (with QAN1d); **QINC1p [S]** `getComputedStyle` from the element table for
  rendered elements (`StyleCache` only for the rest); **QINC1q [M]** stylesheet invalidation by the added/removed rules'
  features instead of a full re-match on a new generation.
  QINC1k6 DONE (2026-11-21): `handle_image_fetched` now calls `incr_engine::note_window_image` (one-entry
  incremental insert into the window's images + a JS geometry-generation bump; the rect cache was keyed on
  the write generation and served the pre-image layout) before `load` fires. imggeom fixture, window,
  SCRATCHY_INCR=1 x2: A.load and A.raf-after-load = 400x300, textTop 398 (flag off still 600x21, QJ4); check
  mode 1 run: 0 mismatches. Test `qinc1k6_load_handler_forced_layout_sees_the_decoded_image`.
  QINC1k10 DONE (2026-11-21; `incr_layout.rs` harvest `put_body`, no style / box / layout.rs change): the 504 vs 512
  of `tools/incr-gate/fuzz-width-504-512.txt` is `li.c1.c4::before` (`position:absolute; inset:0; width:50%`), not
  the li. Before the move its containing block is the viewport (1024); after it, `div.c0` (`position:relative`, 1008
  = 1024 minus the body margins). Reproduced for every div destination with children (the report's "into div at 1"
  does not name one; 36 of 38 replays failed, one of them as `style.opacity 0 vs 1`, the same cause). Cause: the
  bottom-up walk reached the root through failed attempts, each of which `put` its freshly measured box into the
  harvest with its z-index bubble entries (the `::before` at 504) beside it. A key that gets two `Laid`s is reused
  by neither (both are opened), but only pairwise: two fresh copies cancelled out, then the top-down pass opened the
  old root and put the OLD tree's copy (laid against the viewport, 512) under the same key. It was now the only
  one, so the `<html>` hit (a fresh attempt result whose `ck.bubble` names the key; `try_hit` only checks the key is
  present) took the stale entry. Fix: `AMBIGUOUS` (per update, cleared in `with_ctx`): once a key has held two
  `Laid`s, every later `put` for it is opened too, so it is never served. Tests: `qinc1k10_replay_fuzz_width_504_512`
  (replays the recorded page into each div, with and without the hover step; fails on main),
  `qinc1k10_harvest_key_with_two_laids_stays_unserved` (test hook `incr_layout::harvest_after_puts`), and
  `qinc1k10_moved_abs_pseudo_resolves_against_its_new_containing_block` (hand-made, 0-3 wrappers; like the earlier
  hand-made attempt it passes on main too: the harvest's exact order matters). Fuzz (release): default window x 7
  (`SCRATCHY_FUZZ_SEED` 0..6000, 1 000 each) + `REGRESSION_SEEDS` 0 mismatches; `qinc1k1_fuzz_fractional_heights_far_down`
  x 7 (seeds 0..3000, 500 each) 0 mismatches. Suite 1 736 passed (debug and release), clippy 77, WPT 333 / 293
  (no change). Gate (release build of this branch, 1-min load 3.9-7.1): `gate.sh check 1` ytr ytw wiki github bbc
  social hover: 0 mismatches in all 14 runs (headless and window); social's window run ends `quit=killed` as before
  (pre-existing, see QINC1k4 above).
  QINC1k3 DONE (Opus, 2026-11-21; `incr_style.rs` counter-walk dirtiness, `layout.rs` / `incr_layout.rs` `Ck::fl`;
  engine performance only, no style / box / layout semantics): **Wikipedia App builds with `SCRATCHY_INCR=1` are now
  13-14 % of flag off (gate <= 20 %), root walks 133-134 -> 60 per run, relayout 1.44 s -> 0.23 s, restyle 1.96-2.00 s ->
  0.89 s per run.** A temporary per-frame trace (removed) showed two causes. (a) Restyle: on a frame with any DOM write
  ~95 % of the ~10.7 ms was the document-wide counter / quote walk (`Engine::generated`, ~10 ms over Wikipedia's ~9 300
  elements: the page has `ol` / `ul` / `li` and reference counters), re-run on EVERY non-ambient log entry; the restyle
  itself touched 3-14 elements (popup fade classes, the aria-live region, the hovered link). Each table entry now keeps
  `gsig`, a hash of what `images::generated_walk` reads of it from the same matches (matched `display: none`, `quotes`, the
  element's own `content`, `counter-reset/set/increment`, `::before` / `::after` content parts, the implicit `list-item`
  counter of ol/ul/li; `GEN_ATTR` when a content reads `attr()`; 0 = the walk passes its state through unchanged). The walk
  runs when an entry's `gsig` changes, a non-zero one is inserted (also a moved existing subtree), removed or swept, an
  attribute of a `GEN_ATTR` element changes, a `<script>` / `<style>` appears, or a write lands where the engine keeps no
  entries (a node without an entry, or a child list under a computed `display: none` element: the walk follows matched
  rules only, so it may walk on under an inline `display:none`; for such an element `gsig` also matches the skipped
  `::before` / `::after` slots — fuzz seed 1752). (b) Relayout: every walk went up through `not_cacheable`: Wikipedia's
  thumbnails / infobox float inside the root BFC, so every box after the first float (`floats_in`) and every ancestor of
  one (`floats_out`) was uncacheable: the hovered paragraph's walk (`p#mwWg` ... `html`, 67 per run) and every popup
  append to `<body>` (60 per run) re-measured nearly the whole page. `Ck::fl` (`CkFloats`) now records the floats a box
  was measured among (an `Arc<FloatSnap>` shared by consecutive boxes of a BFC; the set is append-only) and the floats its
  measure placed. `try_hit` reuses such a box only at the same origin among bit-identical floats (`miss_floats`
  otherwise) and places its floats again; the bottom-up `attempt` re-measures it among the same floats and splices only
  when it placed the same ones (`floats_out_changed`). A BFC root among floats still goes up (`bfc_among_floats`): its
  parent places it beside the floats by its min-content width (Q34b) — fuzz seed 5832 found that when the first version
  exempted BFC roots. The 60 root walks left are `pending_out_changed` at `<body>` (a popup appended / removed changes its
  out-of-flow list) and are cheap now (the rest of the page is served from the cache).
  **Window gate** (`gate.sh perf 3 wiki`, nested tile 960x1006, release builds; main = installed e5f64e4 copied aside,
  same session; 1-min load 0.1-0.6):
  | binary | off: builds n / ms | INCR=1: builds n / ms | INCR / off | root | restyle ms | relayout ms (+full) | p95 off / on |
  |---|---|---|---|---|---|---|---|
  | main e5f64e4 | 102-103 / 9 283-9 343 | 212 / 3 528-3 607 | 38 % | 133-134 | 1 957-1 995 | 1 437-1 467 (+145-161) | 206-208 / 36-37 |
  | this branch | 103-104 / 9 440-9 562 | 212 / 1 217-1 350 | **13-14 %** | **60** | **885-887** | **231-237** (+146-160) | 158-211 / **17** |
  Up reasons per run (branch): `not_cacheable` 913-919 -> 0, `pending_out_changed` 6-8 -> 126-131 (the popup walks now
  go up for this reason instead), `intrinsic_ancestor` 139-144 (the Talk / Edit / View history
  tabs: stop below the root, QINC1h3), `float` 30 (the popup's floated settings button). The restyle left (~2.8 ms per
  update, 317-319 updates) is per-update fixed cost, not the walk (proposed item below). An earlier series on a
  960x489 tile (`INJECT_H=316`, absolute coordinates; 60-61 / 103-105 builds): off 5 305-5 350 ms, INCR=1 653-824 ms
  (12-15 %), 24 root walks. **Other pages** (`gate.sh perf 1 ytr ytw github bbc hover`, 960x1006, branch vs main
  e5f64e4, App builds ms INCR=1 (off); restyle ms): YouTube results 929 (5 079) vs 1 132 (4 470), restyle 749 vs 1 048,
  gPS 3.2 vs 4.0 ms; YouTube watch (playing both) 1 276 (12 724) vs 1 836 (12 516), restyle 1 572 vs 4 609, forced
  2 903 vs 5 256 ms; github 2 269 (7 426) vs 2 482 (7 423), restyle 6 813 vs 7 484; bbc 146 (585) vs 179 (540); hover
  71 (156) vs 71 (160): none worse, all within or better than QINC1k / QINC1k2. **Exactness:** `gate.sh check 1` on
  all 7 pages (headless + window, nested tile 960x316 at the time): 0 mismatches, 0 panics (social's window run
  `quit=killed` as before). Fuzz (release): `SCRATCHY_FUZZ_SEED` 0..9000 x 1 000 (`incr_fuzz`, incl.
  `qinc1k1_fuzz_fractional_heights_far_down` and QINC1k10's replay): 0 mismatches (the first version had 2 in 7 000:
  seeds 1752 and 5832, both fixed above and pinned as tests). Suite 1 742 passed (debug), clippy 77, WPT 333 / 293 (no
  change). Tests: `qinc1k3_counter_walk_skips_writes_it_cannot_see`, `qinc1k3_counter_walk_sees_under_an_inline_display_none`,
  `qinc1k3_counter_walk_sees_a_hidden_elements_new_before`, `qinc1k3_changed_floats_out_walk_up`,
  `qinc1k3_clean_boxes_among_floats_are_reused`, `qinc1k3_bfc_root_beside_a_float_is_placed_by_its_parent`; updated
  `qinc1h1_floats_in_force_a_walk_up` (now: spliced among the same floats) and `qinc1j_ab_counter_walk_only_after_a_change_it_can_see`
  (a `color`-only hover match change no longer walks). Recipe note: the nested dwl's tile size follows the desktop
  (960x230 / 316 / 489 / 1006 seen in this session); check the `pre` screenshot, `run.sh` now takes `INJECT_W` /
  `INJECT_H` (tools/README).
  Proposed (the coordinator names them):
  - **[S]** (P3, area incr_style) the ~2.8 ms per-update restyle floor on Wikipedia (9 300 entries, 3-14
    restyled): profile the fixed per-update costs (the animated-entries scan and `age_animations` over the table, the
    64-update `sweep`, invalidation-map lookups) and cut what scales with the table instead of the dirty set.
  - **[S]** (P3, area layout; with QINC1h3) `pending_out_changed` at `<body>` when a popup is appended /
    removed (126-131 per Wikipedia run): let the walk splice the containing block's positioned list when only an
    out-of-flow child changed (the in-flow view is unchanged), so these stop below the root.

**Schedule (up to 5 agents, one per area):** wave 1: a (incr-new + perf), b (layout), d (incr-new: own file
`src/invalidation.rs`; a and d share only `src/tests/mod.rs` lines), e (jsdom). Wave 2: c (layout, after b) ∥ f (style +
incr-new, after a, d, e). Wave 3: g (build). Wave 4: h1 (layout). Wave 5: i (geom) ∥ j (app) ∥ h2 (layout). Then k, l, m.
While f, g and h1 are in flight, other items should not change style.rs / images.rs builder / layout.rs measure semantics
except correctness fixes (open question 4).

### 16. Open questions for the user (with my recommendation)

1. **One engine for App frames and script forced layouts, on the main thread; the shaping worker only for big updates?**
   Recommend YES: it is what makes the frame after a forced read free and removes the reason for most special cases.
2. **Script geometry with real image sizes** (forced layouts use the window's decoded images; JS reads change on pages with
   images, toward what browsers return)? Recommend YES (one engine needs it; today's 0x0 images are a known limit, NOTES QJ4).
3. **Drop R59 / QY54b3a-f and the narrow items listed in section 15's heading**, replaced by QINC1? Recommend YES. QY54b1-style
   correctness items are not affected.
4. **Freeze other changes to style/box/layout semantics while QINC1f-h1 are in flight** (correctness fixes still allowed, each
   one rebased onto the shadow comparisons)? Recommend YES, otherwise every change has to be made twice.
5. **Memory:** +4-14 MB (element styles) plus the kept box tree (~4 MB on YouTube results) on big pages. Recommend ACCEPT;
   `Rc<ComputedStyle>` sharing can follow if QMEM measurements object.
6. **Test time:** shadow mode lays out twice in every layout-running test (the suite gets slower, maybe 1.3-2x in layout-heavy
   test files). Recommend ACCEPT (it is the main exactness net); `SCRATCHY_SHADOW=0` for a quick local run, never in CI.
7. **Flip criteria:** 0 `incr-check` mismatches on the 6 pages x 3 runs + the section 12 gates, then flip; delete the old special
   cases after >= 3 days of your normal use with no incr-related report. Recommend these.
8. **Relative-coordinate fragments (QINC1n) and damage-rect paint (QINC1o)** now or later? Recommend LATER, only if QINC1k shows
   the floors above ~2 ms on the gated pages.
9. **Models:** Opus for f, g, h1, i, j, k (cross-cutting, exactness-critical), Sonnet for a, b, c (with Opus review), d, e,
   h2, l, m. Recommend as stated.


**QINC1a DONE (Sonnet, 2026-11-20):** engine test infrastructure, no behaviour change. `src/incr_diff.rs`: `style_diff(old,new) -> StyleDiff {None,Paint,Offset,Layout,Box}` over ONE exhaustive destructuring of `ComputedStyle` (no `..`; a new field is a compile error until classified; `node`/`tree_order` ignored), `style_changes` for reports; `opacity` 1 <-> <1 and `z_index` are Layout. `src/incr_cmp.rs` (test-only): `cmp_styles`, `cmp_boxes`, `cmp_laid` (0.01 px, `fixed`, `positioned` order, sticky/scroll info, glyphs through the shape caches; paragraph ids and `ScrollInfo::id` are not compared) with a first-difference `Mismatch` (path of `tag#id.class[i]`, field, both values). `src/tests/incr_fuzz_tests.rs`: seeded generator of pages (CSS pool with combinators, :nth-child, :hover, custom properties, counters, ::before, display/float/position/z-index/opacity kinds) and mutations (insert, remove, move, text, class/id/style, hover, viewport, stylesheet text); each step lays out twice from scratch and compares boxes and laid; 200 cases by default (`SCRATCHY_FUZZ_N`, `SCRATCHY_FUZZ_SEED`), 43 s in a debug build; found no difference (the from-scratch path is deterministic). `--perf-report` has an `incr` object (`perf::Incr` counters, `bottom_up_stops_by_level`, `worker_fallbacks`), all zero until the engine lands. Only non-test side change: `Debug` derived on the `ComputedStyle` field types that lacked it (css.rs, style.rs), for readable reports.

## R59: QY54b3 design: lay out less than the whole page for a script geometry read. Two stages that share one piece: the JS thread records WHERE each geometry-changing DOM write landed ("dirty points") and keeps, from the last full forced layout, the inputs needed to restyle one box (its parent's `InheritedStyle`, its own `ComputedStyle`, its containing-block size). Stage 1 ("unchanged box"): when every dirty point is inside a box whose size and position cannot depend on its content (an "insulating" box), a `getBoundingClientRect`/`offset*` read of that box or of anything outside it is answered from the stale cache with NO layout. Stage 2 ("subtree layout"): when the read is inside such a box and the box is absolutely/fixed positioned, re-build and measure only that box at its cached containing block, translate it to its cached place and splice its rects, skeleton and glyphs into `GeomCache`. Both are exact by construction for the cases they accept, and both run under a check mode (tests always, `SCRATCHY_GEOM_CHECK=1` on real pages) that compares with a full layout. A throwaway prototype confirms the core claim of stage 2: a box built from its parent's inherited style and measured alone at a provisional origin, then translated, equals the full layout's boxes to 0.00024 px. Estimated win on the R58 recipe: stage 1 alone answers ~55-70 % of the `getPlayerSize` reads in ~0 ms (only after QY54b1 gives the player its CSS), which meets the median gate but probably not the 300 ms job gate; stages 1+2 answer ~all of them at ~3-8 ms each, which should meet both (~190-330 ms) if the non-layout part of the job stays at R58's 93-150 ms. RECOMMENDED, staged: QY54b3a-QY54b3f (Opus, 2026-11-20, design only, docs only) — OPEN: proposed QY54b3a-QY54b3f (section 11)

Inputs: R58 (the recipe, the per-read cost 54-80 ms, sections 1-2: where the writes land), R50/QYTC1a (`style_write_in_place`,
`Skel`, `SK_FIXED_IN`, `redo_ancestor_extents`), QYTC1d (`ClassDeps`, `CF_SIB`), QY53l2 (`HiddenSnap`), QY53o1 (`ATTR_SNAP`,
`attr_write_unread`), QY54b2 (#429, R58 note: no code change; the pre-pass does every element's match and `build_children` reuses it). Code read at
022830a: js_geom.rs `GeomCache::refresh`/`walk`/`walk_glyphs`/`style_write_in_place`, js_dom.rs `geom_bump*` and the
`__d_rect` / `gRect` JS readers, images.rs `build_layout_tree_with_resources`/`build_box_for_element`/`collect_real_nodes`/
`precompute_generated`, style.rs `matching_declarations_for` (MATCH_MEMO, ANCESTOR_BLOOM), layout.rs `measure_document`/
`resolve_positioned`/`hoist_negative_z`/`is_stacking_boundary`, anim_eval.rs `finish_animation_pass`.

### 1. What the cache does today and why a partial path is possible

`GeomCache` is keyed on `(geom_gen, vw, vh, css_ver)`. Any write that may change geometry bumps `geom_gen`
(`js_dom::geom_bump`), and the next read runs `refresh`: `build_layout_tree_with_resources` (css collect, rule index,
`precompute_generated`, `build_children` over the whole DOM) + `measure_document` + `walk_glyphs` + `walk` (rects and the
QYTC1a skeleton). `geom_bump` records nothing about WHERE the write was, so the only answer to "something changed" is the
whole page. QYTC1a/QYTC1d/QY53l2/QY53o1 avoid the bump for writes that provably change nothing (or only shift an abs box);
R58 section 2 shows that the preview player's writes are real changes (45 % structural, 24 % size, 14 % first `style`),
so they need a layout, but all of them stay inside the preview, never in the results list. Selector matching walks the real
DOM (`El(handle)`), so a box restyled on its own still sees its ancestors, siblings, `:nth-child` position and `:hover` state
exactly as a full build does; what a partial path must supply is the state a full build threads down from the root (the
parent's `InheritedStyle`: inherited properties, custom properties, `root_font_size`; the `DecorationValue` and `href`
carried through inline ancestors; the counters/quotes state of `precompute_generated`) and the box's containing block.

### 2. Dirty points (shared by both stages)

A thread-local `GEOM_DIRTY` next to `GEOM_GEN`: `Clean | Points(Vec<(WeakHandle, Kind)>) | Unknown`, reset by every full
or subtree layout. `geom_bump()` (unknown cause: adopted sheets, `__d_sheet_text`, anything without a node) sets `Unknown`;
the write paths call `geom_bump_at(node, kind)` instead, which pushes a point (cap 64, then `Unknown`):

| write | point | kind |
|---|---|---|
| attribute set/remove (`geom_bump_attr`, `geom_bump_if` on an element) | the element | `Attr(name)` |
| insert / remove / move (`__d_insert`, `__d_remove`), `textContent`, `innerHTML` (`geom_bump_children`) | the parent whose child list changed (both parents for a move) | `Children` |
| text node data | the text's parent element | `Children` |
| form state (`__d_setchecked`, `__d_settvalue`, `__d_selset`, select/option, reset) | the control | `Form` |
| a moved/inserted subtree containing `style`/`link`/`meta`/`base`/`slot` (QY53l2 `subtree_sensitive`) | - | `Unknown` |

A point whose node is no longer connected when the read comes is ignored: the remove/insert that disconnected or moved it
left its own point on the parent(s). QYTC1a in-place shifts do not bump and leave no point (the skeleton is already
up to date). The cost is one `Vec` push per bumping write.

### 3. The stash: what the last full layout keeps for later partial work

While `GeomCache::refresh` runs a full layout (a thread-local recorder flag; the App's builds never record), keep for every
element that passes the STATIC insulation test of section 4:

- `build_box_for_element_body` (or `collect_real_nodes` at its block arm) records `inherited` (the parent's
  `InheritedStyle`, which holds the parent's custom properties `Rc` and `root_font_size`), `decoration`, `href`, and the
  element's own resolved `ComputedStyle` (it derives `PartialEq` since QY53n2c);
- `resolve_positioned` records `(cb_w, cb_h)` for the box it places (`item.b.style.node`); `measure`'s block path records
  whether a percentage `height` resolved against a definite containing height;
- `GEOM_INTERACTION`: `js_style::interaction_hash()` (hover/active/focus/target) at the layout;
- `fixed_roots`: the nodes of the `fixed` boxes resolved at the document root (the only boxes whose `Laid` lives outside
  their DOM ancestors' `Skel` ranges besides z-index hoisting).

Keyed by node id with a `WeakHandle` (address reuse). A few hundred entries on YouTube results (the box must be
block-level, BFC-establishing, with a definite height), a few hundred bytes each. Replaced by the next full layout.

### 4. Which boxes qualify ("insulating" box E)

E's border box (x, y, w, h) and every box outside E's subtree can not change when only E's content changes:

1. An element with exactly one `Skel` entry (not in `skel_multi`), not inside an `OPAQUE_CONTAINERS` element, not
   replaced (`img`, `video`, `svg`, `canvas`, `iframe`, form controls), not `display: contents` (it has no box) and not
   inline-level (`inline-block`/`inline-flex` have a content baseline that aligns the line, `inline` is split into the
   line).
2. It establishes an independent formatting context: `position: absolute | fixed`, or `overflow` not `visible`, or
   `display: flex | grid`. (A plain block lets a first child's top margin collapse through it and floats escape from it;
   both move things outside.)
3. Its size does not depend on its content. Width: a length, or a percentage (the containing block is outside and
   unchanged), or `auto` for an in-flow block-level box (fill-available, content-independent); never `auto` for abs/fixed
   (shrink-to-fit). Height: a length, or a percentage that resolved in the last layout; never `auto`. No `min-*`/`max-*`
   content keywords. `position: sticky` and floats are out.
4. Its parent formats it as a block: E is abs/fixed (any parent), or E's parent box is a block container in normal flow.
   A flex/grid/table item is out: the container sizes items from their content (`min-width: auto`, `flex-basis: auto`).
5. No rule uses `:has()` (`rule_child_deps().0`; scratchy does not parse it today, R47: 0 rules on YouTube/Wikipedia,
   but the guard keeps a future parser honest).

Why nothing outside changes then: selectors reach from an element to its ancestors, its previous siblings and (for
`:nth-*`/`:last-child`) its siblings; an element outside E has no ancestor or sibling inside E, so a write strictly inside
E changes no outside element's matched rules (except via `:has()`, rule 5). Layout reaches outside E only through E's
margin box (fixed by rules 2-4) and, for abs/fixed descendants, through their containing block, which is never outside
the flow. What DOES change outside: the scrollable-overflow extents (`ink_r`/`ink_b` of E and its ancestors,
`walk`'s unclipped union), and the root's `in_flow_bottom` for an overflowing in-flow E. Neither changes a rect.

**Points ON E itself** (not strictly inside): an `Attr(name)` point qualifies only when no selector reads `name`
(`ATTR_SNAP.reads(name, tag)`, the QY53o1 table; `style` is normally unread) and E's re-resolved style is equal to the
stashed one in every field except `custom_props` (custom properties only reach E's own fields through `var()`, which the
re-resolved fields already reflect). A `Children` point on E qualifies with the same style comparison (it catches
`:empty`). A `class` write on E is refused in v1 (it can match `.c + .x` / `.c ~ .x` on outside siblings; QYTC1d's
`CF_SIB` could open it later). Re-resolving E's style needs the rule index installed (see section 6: one `resolve_style`,
well under 1 ms).

**Global refusals** (full layout as today): key differs in viewport or `css_ver`; `GEOM_DIRTY` is `Unknown`; the
interaction hash changed; the cached layout panicked or was incomplete; `GEOM_DIRTY` is `Clean` but the key differs (a
bump that recorded nothing: a bug guard).

**Choosing E**: take the deepest common ancestor-or-self L of the connected points; walk up from L to the first element
that passes the stash's static test (section 3) and the point rules above. For stage 2, keep walking to the first
abs/fixed one (section 6).

### 5. Stage 1: answer the read without a layout ("unchanged box")

`rect_of(target)` with a stale key: if E exists and the target is E or is NOT inside E, return the cached `x, y, w, h,
fixed` and leave the cache stale (key, points and stash untouched: the next read inside E still needs a layout, and the
points keep accumulating; they stay inside E or the next read falls back). The ink extents and the page size may be stale,
so only callers that use the border box may take this path: the JS side gets a second native `__d_rect_box` (4 values +
fixed) used by `gViewRect` (`getBoundingClientRect`, `getClientRects`), `offsetLeft/Top/Width/Height` and `gClient`
(`clientWidth/Height`); `scrollWidth/Height` (`gScrollSize`, which reads `ink_r/ink_b`/page size), `scrollIntoView` and
IntersectionObserver keep `__d_rect`. `text_rects` keeps the full path. The cost per served read is the LCA walk over at
most 64 points (~25 levels each) plus, if a point is on E, one style resolve: tens of microseconds.

Side effect to note: a served read leaves the cache stale, so an IntersectionObserver check that follows (it needs a fresh
cache or the App's `rendered` layout) pays the full layout that the read used to pay. In the R58 burst that is at most one
full layout per rendering step instead of 11-13 inside one promise job.

### 6. Stage 2: lay out only box B ("subtree layout")

B = the nearest abs/fixed insulating ancestor-or-self of L (rules 1-5 plus): B `is_stacking_boundary` in the last layout
(overflow not visible, or a `z-index`, or `opacity < 1`, or fixed), so z-index hoisting (`hoist_negative_z`) never moves a
descendant's `Laid` out of B; and no node of `fixed_roots` is now inside B or detached (a fixed descendant's `Laid` sits at
the document root, outside B's `Skel` range).

**Style.** A new `images::build_subtree_box(B, rules, stash)` does what `build_layout_tree_with_resources` does around
`build_children`, for one element: `set_viewport`, the cached rule list (`rules_cache::current()`; refuse if its
generation is not the stashed one), the `RULE_DERIVED` index installed in `RULE_INDEX`, fresh `MATCH_MEMO`,
`VAR_EXPAND_MEMO`, `ANCESTOR_BLOOM` (its `sync_to` already pushes B's real ancestor chain, so `.a:hover .b` with `.a`
outside B is filtered and matched exactly as in a full build), `CLASS_CACHE`, `PSEUDO_FLAGS`; then
`build_box_for_element(B, rules, stash.inherited, stash.decoration, stash.href, vw, &HashMap::new())`. The parent's
inherited style (including its custom properties and `root_font_size`) is exactly what the full build would pass, because
no point is outside B and CSS, viewport and interaction state are unchanged. Then B's new `ComputedStyle` must equal the
stashed one (minus `custom_props`), else refuse. What it must NOT do (the audit list for QY54b3d; each is a thread-local the
full build resets): no `finish_animation_pass` (its `retain(seen == pass)` would drop the start times of every animation
outside B, restarting them; merge B's `ANIM_REPORT` entries into `ANIM_LAST` instead), no `stash_last_match` (the App's kept
hover memo, QY53n1c), no `propagate_canvas_background`, no reset of `VIEWPORT_ROOT_NODE`, no `memo_sweep` of the shape memo
(it evicts every paragraph the subtree did not use, i.e. the whole page's), `DISPLAY_NONE_ROOTS` collected for B only.

**Generated content and counters.** `precompute_generated` fills `GENERATED_TEXT`/`ELEMENT_CONTENT` in one document-order
walk because counters and quotes flow in document order, also out of B into later content. It is also where every element's
match is computed first (QY54b2 note in R58: the pre-pass fills `MATCH_MEMO` and `build_children` reuses all of it, ~1.4 us per
match, 3 per element). So a whole-document walk costs the whole page's matching (10.5-13 ms on YouTube results) and is out of
the question for stage 2. Instead: factor the walk as `generated_walk(root, rules, vw, &mut State)` with `State` (counters,
quote depth, quotes) named and `Clone + PartialEq`. In the full layout the walk records, for every element whose cascaded
`position` (from the `own` match it already has) is `absolute`/`fixed`, the `State` on entry and on exit (a clone each, a few
hundred elements). The subtree pass runs `generated_walk(B, entry_state)` and compares its exit state with the recorded one:
different = a counter or quote inside B changed what follows B = refuse. Equal = every entry outside B is unchanged, so only
B's entries are replaced (the maps are cleared to B's entries for the subtree build, then restored from the full layout's
copy). A rule list with no counter/quote use at all (one `RuleDerived` flag) skips the state handling. This also makes the
pre-pass the place where B's matching happens once, as in a full build. (QY54b2b's cross-layout match memo, if built, makes
both paths cheaper; stage 2 does not depend on it.)

**Measure.** `resolve_positioned(PendingPositioned { b, fixed, static_x: 0, static_y: 0 }, 0, 0, cb_w, cb_h, ...)` with the
stashed containing-block SIZE at a provisional origin, then `shift_laid_tree` by (cached `x`, `y`) - (new `x`, `y`). The stashed
absolute coordinates of the containing block or static position cannot be used: an ancestor placed by `right`/`bottom`, a
relative offset or flex alignment is measured first and shifted afterwards, so the coordinates `resolve_positioned` saw
are pre-shift. B's position is a function of its unchanged style, its unchanged containing block and its unchanged static
position, so the cached `x, y` IS the new one; its size is definite and must come out equal to the cached `w, h` (else
refuse). Then `hoist_negative_z` on B as owner (B is a boundary) and nothing else; `bubble_out` must be empty (a new fixed
descendant escapes B: refuse); an incomplete shaping pass (deadline) refuses. The `ShapeCache` is a new one, as in `refresh`.

**Splice into `GeomCache`.** With i = `skel_of[B]` and the old range `[i, end)`:
1. remove `rects`, `skel_of`, `skel_multi` entries of every node owned by the old range (this also drops removed nodes,
   whose addresses may be reused by new nodes), and the `glyphs` of the text nodes laid in it: `walk_glyphs` records, per
   text node, the pre-order index of the `Laid` it came from (it visits the same `Laid`s in the same order as `walk`);
2. `tail = skel.split_off(end)`, `skel.truncate(i)`, `walk(newB, fixed_of_old_B, ...)` appends B's entries with correct
   absolute indices; for the tail add `delta = new_len - old_len` to `end` and to every `parent >= end`; add `delta` to
   `end` of B's ancestors (entries before i whose `end >= old end`); append the tail; rebuild `skel_of`/`skel_multi`
   (O(entries), ~3k on YouTube) and shift the tail's glyph indices;
3. insert B's new rects and glyphs; `redo_ancestor_extents(parent of B)`; `self.page` is unchanged (B is out of flow);
4. `HiddenSnap`: drop roots that are inside B or detached, add B's new display-none roots through the
   `hidden_snapshot` filter, set its `ggen`; `ATTR_SNAP`: set its `ggen`; `imp_geo` stays (same rule list);
5. update the stash for nodes inside B (their inherited styles, cb sizes); key = the current key; `GEOM_DIRTY = Clean`;
   `last_cpu_ms` = this cost; `--perf-report` counts it as a forced layout of kind `subtree` (see section 8).

A panic in any step: catch, drop the partial state, run the full layout. IntersectionObserver reads after a subtree layout
see a fresh cache, so it also removes the IO layout of stage 1's side effect.

### 7. Prototype (throwaway, never committed)

`scratchpad/qy54b3/qy54b3_proto_tests.rs` (a test module dropped into `src/tests/` for one debug run at 022830a, then removed):
a page with 2x30 result items around a `position: relative` slot (fractional margins/padding) holding an abs wrapper
`#w` (`left: 21.13px; top: 17.71px; width: 440px; height: 248px; overflow: hidden`, placed at y ~2 089) with a player inside:
`width: 100%; height: 93.5%`, `rem` font size, a custom property from `<body>` through `var()`, a `.list > :nth-child(3) .x`
rule whose `:nth-child` is decided OUTSIDE the box, `::before { content: attr(data-t) }`, a flex row with `gap: var(--gap)`
and an `<img>`, an abs overlay placed by `right`/`bottom` with a % width, an abs caption with % insets, and a table.
Full layout through `GeomCache::rect_of`; then, for `#w` alone: the parent's `InheritedStyle` by resolving the ancestor chain
(standing in for the stash), `precompute_generated`, `build_box_for_element(#w, ...)`, `resolve_positioned` at (0, 0) with the
slot's size, `shift_laid_tree` to the cached position. Result: **13 boxes, max |difference| 0.00024 px** (the translation's f32
rounding at y ~2 089; the 0.01 px tolerance is 40x that), no bubbled `fixed` entry, complete in one pass. Not covered by the
prototype (left for the tests of QY54b3d/e): z-index hoisting inside B, the splice itself, glyphs, counters, a write.

### 8. Exactness check mode and observability

- `#[cfg(test)]`: check mode is ALWAYS on. Every served read (stage 1) and every subtree layout (stage 2) also runs a full
  layout into a scratch `GeomCache` at the same key and compares: for stage 1 the served `x, y, w, h, fixed`; for stage 2
  every rect (same node set, `fixed` equal, `x, y, w, h, ink_r, ink_b` within 0.01 px, the QYTC1a test tolerance; the
  translation adds f32 rounding), every glyph list of a text node inside B (same chars, coordinates within 0.01 px), the
  skeleton's node sequence, `page`, and the hidden roots. A mismatch panics with the node's tag/id/class, the field, both
  values, B and the points. So every existing js_geometry/observer test that takes either path is also an exactness test.
- `SCRATCHY_GEOM_CHECK=1` (read once): the same comparison on real pages; a mismatch prints one `scratchy: geom-check:
  MISMATCH ...` diag line, counts `geom_check.mismatch` in `--perf-report`, and the full result is adopted. Animations
  outside B are the one expected source of noise (the cache already ignores animation time today: its key has no clock), so
  the line says whether the node runs an animation.
- `--perf-report`: `forced` gets `served` (stage 1 count), `subtree` (count, ms, median) and `partial_refused` by reason
  (unknown, outside, no_boundary, point_on_box, style_changed, size_changed, fixed_escape, generated_outside, css/vp/
  interaction, has_sel, incomplete). QY54b3a adds these counters (as "would") before any behaviour changes.

### 9. Estimate on the R58 recipe

From R58 sections 1-3 (no layout code changed yet; ranges because the data were logged without the player CSS):

| | reads per preview | full layout | after |
|---|---|---|---|
| R58 base (no player CSS) | 33-36 per run, 11-13 in the worst job | 54-80 ms each | - |
| reads whose writes all stay inside `#inline-preview-player` (or on it) | 24-26 of 33-36 (~70 %) | | stage 1 only if the player is insulating, i.e. AFTER QY54b1: without its CSS it is static, `height: auto` (R58 section 3), so stage 1 serves none of them today |
| reads whose writes reach the preview containers / inline controls | 9-13 (~30 %) | | stage 2 at the nearest abs boundary (the wrapper `.ytdVideoPreviewPlayerContainerWrapper`, abs 440x248 overflow hidden, or a qualifying preview container: QY54b3a finds out) |
| reads whose writes reach the results list | 0 | | - |

Per read: stage 1 ~0.05 ms. Stage 2: a subtree of a few hundred elements (matching ~1.4 us x 3 per element, R58 QY54b2 note, ~1-2 ms of it); R58's headless watch reads, which laid out a
WHOLE document of 144-472 elements (the player being built), cost 5.2-5.7 ms median (2.2-15.2), so ~3-8 ms. Worst
preview-start job today: 685-1 137 ms, of which 592-884 ms are `getPlayerSize` layouts and 93-150 ms are everything else.
- Stage 1 alone (after QY54b1; a full layout stays 54-80 ms, QY54b2 found nothing to cut): ~55-70 % served, 3-5 full
  layouts per job = ~160-400 ms + 93-150 ms = **~255-550 ms**: the median gate (< 10 ms) is met, the job gate (< 300 ms)
  only at the very good end.
- Stages 1 + 2: 11-13 reads at <= 8 ms (stage 2 ones) or ~0 ms = <= ~100 ms + 93-150 ms = **~190-250 ms**, plus one full
  layout if the first read of a preview follows the container's creation outside any boundary (+54-80 ms, so up to ~330 ms).
  The gate is reachable but not guaranteed; QY54b3a measures the real split before stage 2 is built.

### 10. Risk, and what not to do

Risk is moderate and mostly scratchy-specific coupling the CSS argument does not see: an abs box splits its parent's inline
run (`collect_real_nodes` flushes `pending` at a block-level child) - harmless, it depends on the box's existence, not its
content; shrink-to-fit and table/flex intrinsic sizes read descendants (excluded by rules 3-4); margin collapsing and float
escape (rule 2); the root's `in_flow_bottom` (not a rect). The check mode turns any wrong assumption into a failing test or a
diag line instead of a wrong answer, and both stages can be switched off (`SCRATCHY_GEOM_PARTIAL=0`). Stage 1 is small (no
layout code, no splice) and is the cheap first half. Not worth doing: a general incremental layout (dirty bits on boxes,
relayout of in-flow ancestors): XL, and R58 shows the reads here never need it. Alternatives if the design is rejected:
QY54b2 found nothing to cut in the full build; QY54b2b (a match memo kept across forced layouts) could take at most the
matching (~10-13 ms of 54-80) off each full layout, so the worst job stays > 500 ms; answering
`getPlayerSize` from the App's `rendered` layout (QY53e2) does not help because the player changed since the App's build.

### 11. Proposed items (coordinator files them; they supersede QY54b3 [L] and absorb QY53o2)

- **QY54b3a [M]** (P2, Sonnet; js_dom.rs `geom_bump*`, js_geom.rs, images.rs/layout.rs recorder hooks, perf.rs; NO behaviour
  change) Dirty points + stash + classifier (R59 sections 2-4, 8). `GEOM_DIRTY` (`Clean | Points(<=64 (WeakHandle, Kind)) |
  Unknown`) filled by `geom_bump_at` at every write path of the R59 section 2 table, `geom_bump()` = Unknown, reset by
  `refresh`. The full-layout recorder (on only inside `GeomCache::refresh`): per insulating candidate the parent's
  `InheritedStyle`, `decoration`, `href`, own `ComputedStyle`, cb size from `resolve_positioned`, %-height-resolved flag;
  `interaction_hash`; `fixed_roots`. `GeomCache::partial_verdict(target) -> Serve(E) | Subtree(B) | Refuse(reason)` applying
  R59 section 4 (+ section 6's B rules) without acting on it; `--perf-report` `forced.partial_would` {serve, subtree, refused by
  reason} plus, under the QOBS3 recorder, the tag#id.class of E/B. Tests (js_geometry_tests.rs): one per rule and per refusal
  reason (write outside, point on E with a read attribute, class on E, style change on E, auto height, flex item, float,
  plain block without BFC, display:contents, :has guard via a test seam, Unknown cap, detached point ignored, css/vp/interaction
  change); the verdict never changes a rect (all existing tests unchanged). Gate: R58 recipe (window, 3 runs, flock; with
  QY54b1 merged, else say so) reports per gPS read and per worst promise job the would-serve / would-subtree / refused split;
  write it into R59 as go/no-go: build QY54b3b if >= 50 % of gPS reads would be served, QY54b3c-e if serve + subtree >= 85 %.
  If neither, stop and report (the design is then not worth its cost on this page).
- **QY54b3b [M]** (P2, Sonnet; AFTER QY54b3a and QY54b1; js_geom.rs, js_dom.rs JS readers) Stage 1, "unchanged box" (R59
  section 5): `rect_of_box` serves `x, y, w, h, fixed` of E or of any element not inside E from the stale cache without a layout;
  new native `__d_rect_box` used by `gViewRect` (getBoundingClientRect/getClientRects), `offset*` and `gClient`; `scroll*`,
  `scrollIntoView`, `text_rects` and IntersectionObserver keep the full path. Check mode (R59 section 8): always on under
  `cfg(test)` (a scratch full layout per served read, panic on mismatch), `SCRATCHY_GEOM_CHECK=1` on real pages (diag line +
  `geom_check.mismatch` counter, full result used); `SCRATCHY_GEOM_PARTIAL=0` disables. Tests: served reads equal a fresh
  layout (insert/remove/text/style inside an abs box, an overflow:hidden in-flow box with definite height, a flex container
  with definite size; reads of E, a sibling after E, an ancestor), refused when the target is inside E, a read of
  `scrollHeight` after a served read lays out, a served read then a write outside then a read = full layout. Gate: full
  suite + WPT unchanged; `SCRATCHY_GEOM_CHECK=1` 0 mismatches on R58 recipe x3, YT watch 60 s, Wikipedia Rust with pointer
  (Page Previews); then without it, R58 recipe x3 (flock): gPS layouts per run and median ms, worst preview-start job.
- **QY54b3c [S]** (P2, Sonnet; images.rs `precompute_generated`; no behaviour change) Factor the walk as
  `generated_walk(root, rules, vw, &mut State)` (`State` named, `Clone + PartialEq`); while the full-layout recorder is on, record
  the entry and exit `State` of every element whose cascaded `position` is absolute/fixed; `RuleDerived` flag `doc_order` (a
  rule declares counter-*/quotes or `content` with counter()/counters()/quote keywords). Tests: generated content/counters/
  quotes tests unchanged; walking a recorded abs element's subtree from its entry state reproduces its entries and its
  exit state; `doc_order` false on a page with only `attr()` content.
- **QY54b3d [M]** (P2, Sonnet; AFTER QY54b3a/c; images.rs, anim_eval.rs) `build_subtree_box(B, rules, stash) -> Option<LayoutBox>`
  (R59 section 6 "Style" + "Generated content"): the per-build thread-locals of `build_layout_tree_with_resources` for one
  element (rule index, memos, bloom, class cache, pseudo flags, viewport, DISPLAY_NONE_ROOTS for B only), B's generated entries
  from its recorded entry state (refuse if its exit state differs), and NOT: `finish_animation_pass` (merge B's report into
  `ANIM_LAST` without ageing other start times), `stash_last_match`, `propagate_canvas_background`, `VIEWPORT_ROOT_NODE` reset.
  Refuse when B's new `ComputedStyle` (minus `custom_props`) differs from the stashed one. Not wired to GeomCache yet. Tests
  (images_tests.rs): the subtree box equals the matching box of a full build (a structural compare helper) on the R59 section 7
  fixture plus `.a:hover .b` with `.a` outside (interaction state set), `:nth-child` decided outside, `rem`/`vh`, inherited
  font/colour, a custom property from an ancestor, a counter running through B (exit state equal) and one changed inside B
  (refused), `display: none` inside B; an animation outside B keeps its start time across a subtree build.
- **QY54b3e [M]** (P2, Sonnet; AFTER QY54b3b/d; js_geom.rs) Stage 2 measure + splice (R59 section 6 "Measure"/"Splice"):
  `resolve_positioned` at a provisional origin with the stashed cb size, `shift_laid_tree` to the cached x/y, `hoist_negative_z`
  on B, refuse on size change / bubbled fixed / incomplete shaping / panic; splice `skel` (tail index fix-up), `rects`,
  `skel_of`/`skel_multi` rebuild, glyphs by owner index (`walk_glyphs` records the pre-order index), `redo_ancestor_extents`,
  `HiddenSnap`/`ATTR_SNAP` ggen, stash entries inside B; no `memo_sweep`; `--perf-report` kind `subtree` (n, ms). Check mode
  extended to every subtree layout (all rects, glyphs inside B, skeleton node sequence, page, hidden roots). Tests
  (js_geometry_tests.rs): subtree after insert/remove/text/style writes inside an abs box equals a fresh full layout (all rects
  + glyphs, 0.01 px), with a z-index child inside B, a new fixed descendant (refused), a removed fixed descendant (refused),
  node address reuse (create/remove 1 000 nodes inside B between reads), text_rects after a subtree layout, an IO observing a
  node inside B, a QYTC1a shift of B's ancestor before the subtree layout. Gate: full suite + WPT unchanged;
  `SCRATCHY_GEOM_CHECK=1` 0 mismatches over R58 recipe x3 + YT watch 60 s; then R58 recipe x3 under flock: gPS layouts median
  < 10 ms and worst preview-start promise job < 300 ms (QY54b3 overall gate; if missed, report the refusal mix and stop).
- **QY54b3f [S]** (P2, Opus, measure only, docs; AFTER QY54b3e) Real-page exactness sweep and QY53o2's gate:
  `SCRATCHY_GEOM_CHECK=1` headless + window on YouTube results/watch, Wikipedia Rust (pointer over links: Page Previews),
  github.com, bbc.com/news: 0 mismatches (any mismatch = a P1 fix item); partial verdict mix per page; Wikipedia Page Previews
  forced layouts per run (QY53o2 gate: forced.n <= 5, or the cost of the remaining ones). Close QY53o2 or file what is left.
- QY54b4 (first `style` attribute shifts in place) stays as filed and is independent: it moves the preview container in place,
  after which the writes inside it are stage 1/2 material.

## R58: QY54b: why each inline-preview `getPlayerSize` forced layout on YouTube results costs ~58 ms. Every one is a FULL-document forced layout: `GeomCache::refresh` has no partial path, so each read rebuilds the whole page, ~4 900-6 500 elements (2 150-2 770 boxes, the document is ~7.7k px tall). Style work is ~80 % of it (`precompute_generated` 10-22 ms + `build_children` 30-42 ms of 54-80 ms); text measuring is 10-15 ms. Before each read the player has changed its own DOM: 45 % of the reads follow a node insert/remove, 24 % a `style` size write that QYTC1a refuses, 14 % a first `style` attribute. In-place shifts and skips cannot cover that. The "~5 ms on watch" in R56 is not the window vs headless: on watch the player reads its size in the first 2-3.5 s, while the document has 144-472 elements. The cost per element is the same (headless results: 53 ms at 5 024 elements; headless watch later: 81-107 ms at ~7 000). NEW: YouTube inserts `www-player.css` from script, and scratchy never fetches a script-inserted `<link rel=stylesheet>`. So the preview player has no CSS: it is laid out static and auto-height (150-1 791 px tall in a 232 px slot) and paints in the wrong result slot with stray controls. Applying that sheet (throwaway probe) fixes the size and the paint, but the reads are NOT fewer (41-43 vs 33-36 per preview) and each costs the same (Opus, 2026-11-20, measure only, docs only) — OPEN: proposed QY54b1-QY54b4 (below)

**Setup.** Main 6cdc00a (the same code as d36a809 / 343f615 for everything this measures; the commits after it change only
`wl_drain` docs and TODO files). The binary is a release build (V8 + media) of my worktree at 6cdc00a, in my own target dir,
copied aside. It has THROWAWAY logging that was never committed and does not change behaviour:
(a) every `geom_bump` records its call site, the node it hit and the write (attribute name, old -> new `style` value, the
QYTC1a refusal point, whether a QYTC1d probe ran); (b) every `GeomCache::refresh` miss prints its CPU ms, the QOBS3 phase
split, `style_resolves` / `elements`, the rect and skeleton counts, the page caller and the bumps since the last layout;
(c) every `__d_rect` read that has to lay out prints the element, its rect before and after, and the deepest common ancestor of
the nodes written since the last layout; (d) `QMEAS_PROBE=<file>` runs a JS file before the page scripts (the R56 hook; unset
in every run except the probe runs below). Own wire (a copy of the installed binary) on a scratchpad socket, restarted for
every run. Own nested dwl (tools/nest.sh). The window size changed between launches (960x489 or 960x1006; the per-layout cost
does not depend on it: the page width is 956 in both). `SCRATCHY_FRAME_STATS`, `SCRATCHY_MEDIA_AO=null`, default UA.
**Every run was under `flock /tmp/scratchy-perf.lock`.** Other agents were active on the machine (QY54a3, QY53n1c builds). The 1-min
load average at the start and end of the runs was **0.28-3.31**. The run-to-run spread of the per-layout cost (median 54-80 ms) comes
from that, not from the page.

**Recipe (window).** `https://www.youtube.com/results?search_query=rust`. At 15 s (the page has loaded), tools/inject moves the pointer
onto the first result's thumbnail (300,230) and nudges it 5 px. YouTube starts the inline-preview player. 6 s later it moves to the
second result's thumbnail (300,420), and 6 s after that (first 3 runs: to the top bar). 20 s of input, 9 runs (hov2-4, rd1-3,
base1-3). **Headless:** `--perf-report --seconds 30` on watch G1AeBgalFZk (3 runs), `--seconds 10` on results (1 run, no input).

### 1. What one `getPlayerSize` read costs, and why

`getPlayerSize` (`g.k.getPlayerSize`, player_ias base.js:15479) reads `#inline-preview-player`'s rect.

| run | gPS layouts | per layout, median (min-max) | elements resolved | boxes | worst second (frame stats `longest`) |
|---|---|---|---|---|---|
| hov2 | 45 | 57.1 (54.1-61.8) ms | 4 999-5 177 | 2 144-2 219 | promise job 792 ms: 12 gPS = 688 ms |
| hov3 | 34 | 75.0 (73.3-84.4) | 5 209-5 401 | 2 240-2 322 | 958 ms: 11 gPS = 820 ms |
| hov4 | 51 | 72.3 (59.0-92.5) | 5 122-5 300 | 2 201-2 276 | 939 ms: 22 gPS = 1 623 ms in that second |
| rd1 | 34 | 55.0 (54.3-61.2) | 5 040-5 240 | 2 158-2 244 | 706 ms: 11 gPS = 603 ms |
| rd2 | 35 | 58.9 (57.1-65.5) | 5 243-5 424 | 2 243-2 320 | 747 ms: 11 gPS = 637 ms |
| rd3 | 35 | 53.7 (52.0-58.3) | 4 877-5 077 | 2 299-2 385 | 685 ms: 11 gPS = 592 ms |
| base1 | 33 | 71.4 (68.9-87.6) | 6 342-6 522 | 2 698-2 774 | 897 ms: 11 gPS = 781 ms |
| base2 | 36 | 58.8 (56.7-66.5) | 5 251-5 443 | 2 250-2 332 | 744 ms: 11 gPS = 635 ms |
| base3 | 35 | 78.9 (73.3-104.3) | 5 211-5 403 | 2 236-2 318 | 1 034 ms: 11 gPS = 884 ms |
| headless watch x3 | 42, 42, 42 | 5.2, 5.7, 5.2 (2.2-15.2) | **144-472** | 44-165 | (242-260 ms in all) |

That is R56's burst again: one promise job with 11-12 reads of 54-80 ms each (0.69-1.03 s). Phase medians per gPS layout
(QOBS3 phases, 6 runs rd/base): css_collect 0.4-0.9, rule_index 0.0, **precompute_generated 10.5-19.1**, **build_children
29.7-41.7**, **measure_document 9.8-14.6**, glyph_walk 0.5-0.8, rect_walk 0.5-1.0 ms. Style resolution is ~80 %. Headless
watch gPS layouts split the same way at 1/10-1/30 of the size: 1.6 / 3.0 / 0.1 ms.

**Reason / path.** All of them are the `script` reason (none is `io`). The path is the only one `GeomCache::refresh` has: a
full `build_layout_tree_with_resources` + `measure_document` of the whole page. No QYTC1a shift and no QYTC1d skip happened
before any of these reads. The cache was invalid at every read, because the player had changed its own DOM since the previous read.

**Is it the window?** No. The headless watch page reads `getPlayerSize` 42 times, all in the first 2.3-3.5 s while YouTube
builds the player before the rest of the page: 144-472 elements, hence ~5 ms. Later forced layouts on the same headless watch
page cost 29-40 ms median and up to 81-107 ms at 6 930-7 208 elements. Headless results without input: up to 53 ms at 5 024
elements. **The cost per element is the same (~11 us); R56's "~10x" is the element count at the moment of the read.**

### 2. What invalidates between the reads

The first write after each layout (the one that invalidated the cache), over the 207 gPS layouts of rd1-3 + base1-3:

| first invalidating write | count | share |
|---|---|---|
| structural: insert/remove/`textContent`/`innerHTML` inside the player (`.ytp-chrome-top-buttons`, `.ytp-overlay-*`, `.ytp-title-*`, `.ytp-overflow-panel-*`, `#inline-preview-player` itself) | 93 | 45 % |
| `style` write refused by QYTC1a at the delta step (`style_write_delta` = None): `<video>` `width`/`height` (300x150 -> 412x887 -> 412x1339 -> 412x232...), `<video>` `top` (the video is static, see section 3), the progress bar's `width: 0.15%`, `.caption-window` `max-width`/`bottom` | 50 | 24 % |
| `style` attribute set for the first time (QYTC1a needs an old value): `display: none` on `.ytp-spinner` / `.ytp-cued-thumbnail-overlay`, `width: 300px` on the video, `--yt-delhi-*` custom properties on the player, `top: 112px; left: 96px` on the abs preview container | 30 | 14 % |
| `aria-label` (a selector reads it, so QY53o1's skip does not apply) | 10 | 5 % |
| `style` through another native (`setAttributeNS`/`style` property path) | 9 | 4 % |
| `<video src>` | 6 | 3 % |
| `class` with no QYTC1d verdict (probe not possible) / probe ran and a rule changed | 6 / 1 | 3 % |
| `style` on a node without a skeleton entry (QYTC1a r5) | 3 | 1 % |

So it is YouTube building the preview player's DOM between its size reads: the overlays, title, chrome and caption window
are created, and the video size is set. The reads are not repeated because nothing changes. They are spaced by real changes.

**Where the writes land.** For each read I took the deepest common ancestor of every node written since the previous layout.
In 24-26 of 33-36 reads all writes were inside `#inline-preview-player` (or on it). In 9-13 they reached the preview's own
containers (`.ytdVideoPreviewMediaContainer`, `.ytdVideoPreviewLoaderVideoPreviewContainer`, `ytd-video-preview`
class), or the preview's inline controls (`yt-inline-player-controls`: the controls container, progress bar, time badge).
**No common ancestor named a results-list element.** (The logged chain stops 6 levels up, so "inside `#video-preview`" is
shown only for the chains that reach it.) The
player sits in `.ytdVideoPreviewPlayerContainerWrapper` (`position: absolute; width: 440px; height: 248px; overflow: hidden`)
inside `ytd-video-preview`. A subtree layout under that box would answer every one of these reads (section 4, QY54b3).

**Why QYTC1a/QYTC1d do not apply.** QYTC1a moves a box in place only for `left`/`top`/translate of an abs/fixed box, or for
paint-only fields. 45 % of the reads follow structural changes. The 24 % refused at the delta step are size changes (width,
height, max-width), or a `top` on a box that is not absolutely positioned (the `<video>` is static here because its CSS is
missing, section 3). The 14 % are a `style` attribute that did not exist before; QYTC1a refuses `old = None` (r1). Most of those
change layout anyway (`display: none`, a width). The exceptions are the abs container's `top`/`left` and maybe the
custom-property-only writes. QYTC1d had no verdict for the few class writes that came first. **At most ~1-2 reads per burst
could be saved by widening the in-place paths.** The cost has to come down in the full layout itself (QY54b2) or by laying out
less than the whole page (QY54b3).

### 3. NEW: script-inserted stylesheets are never applied (the preview player has no CSS)

The player's rect cycled between reads: 300x150 -> 412x434 -> 484 -> 903 -> 1 305 -> 886 -> 1 624 -> 1 339 -> 1 791 -> 684 ->
996 px tall, in a 232 px slot. Only 21-26 of 33-36 reads returned the previous rect. A probe of computed styles
(`QMEAS_PROBE`, 2 diagnostic runs) showed `.html5-video-player` as `position: static; height: auto; overflow: visible`, and its
`.ytp-chrome-top`, `.ytp-overlays-container`, `.html5-video-container` children as `static` (in flow). On the watch page the
same element is `position: relative; overflow: hidden; width/height 100%` (640x360). Cause: on results, `www-player.css` is not in
the HTML. A probe that runs before the page scripts sees 4 stylesheet links; YouTube inserts
`https://www.youtube.com/s/player/<hash>/www-player.css` into `<head>` at ~5.2 s (MutationObserver). Scratchy fetches only
the `<link rel=stylesheet>` of the parsed document (net.rs `fetch_stylesheets` over `resources.stylesheets`). A script-inserted
link is never fetched, never applied, and fires no `load`. The same is true of the two Google Fonts links inserted at ~4.6 s.

Counterfactual (throwaway probe `QMEAS_PROBE=probe4.js`: on that link's insertion it `fetch()`es the href and appends the
text as a `<style>`; 3 runs css1-3 interleaved with base1-3, the same binary):

| | base1-3 (main) | css1-3 (player CSS applied) |
|---|---|---|
| player rect unchanged between reads | 22, 26, 24 of 33, 36, 35 | **40, 41, 39 of 42, 43, 41** (412x232) |
| gPS layouts per run | 33, 36, 35 | 42, 43, 41 |
| per layout, median | 71.4, 58.8, 78.9 ms | 61.8, 63.2, 79.6 ms |
| worst promise job | 897, 744, 1 034 ms (11 gPS) | 865, 886, 1 137 ms (13 gPS) |
| first invalidating write | structural 45 %, size 24 %, new `style` 14 % | structural 42 %, new `style` 19 %, class with a rule change 10 %, size 10 % |
| screenshot (pointer on the 2nd result) | the preview plays over the **1st** result's thumbnail; a `1:34:30` label and a red dot float above it, at the filter bar | the preview plays over the **2nd** result, no stray controls |

So the missing sheet is a real rendering bug (wrong size, wrong slot, stray chrome) and should be fixed (QY54b1). **It is
not the cause of the freeze.** With the sheet the size is stable, but YouTube reads slightly more often and every read is still
a full ~60-80 ms layout.

### 4. Proposed items (coordinator files them)

- **QY54b1 [M]** (P1, Sonnet; net.rs/app.rs/js_dom.rs, correctness) A `<link rel=stylesheet href>` that a script inserts
  (or whose `href`/`rel` changes while connected) is never fetched or applied. YouTube results inserts `www-player.css` at ~5 s,
  so the inline-preview player has no CSS (R58 section 3: 412x684-1 791 instead of 412x232, it paints over the wrong result with
  stray controls). Fetch it through wire like a page subresource (async, not on the JS thread). When it arrives, add its text
  to the page's external CSS at the link's document position (one `css_ver` bump, so the JS thread's `page_css` and the App
  build both see it; `@import` one level as for parsed links). Fire `load` (or `error`) on the element. Removing the link (or
  `disabled = true`) removes its CSS. Media-attribute handling as for parsed links. Tests: a fixture page whose script
  appends a link to a local CSS file that makes a div `position: absolute; height: 10px`: the div's rect changes after the
  link's `load` fires; removing the link restores it; a 404 fires `error` and changes nothing; a link inserted twice applies
  once. Gate: YouTube results in the nested window, pointer on a result: `#inline-preview-player` computed `position: relative`,
  its rect stays 412x232-ish across reads (R58 throwaway logging or a probe), and grim shows the preview over the hovered
  result with no controls above it. Also check that the YT watch page and Wikipedia do not change (their links are parsed).
- **QY54b2 [S]** (P1, Sonnet; images.rs `precompute_generated`) The counters/quotes/`content` pre-pass costs 10-22 ms of every
  forced layout on YouTube results (18-27 %), and of every App build: its walk calls `matching_declarations_for` three times per
  element (element, `::before`, `::after`) against the FULL rule list, on top of `build_children`'s own matching. It only needs
  `display` (to prune `display: none` subtrees), `content`, `counter-*` and `quotes`. Match against a prefiltered rule list
  (the `may_declare_generated` rules plus the rules that declare `display`, cached per `rules_cache` generation; inline
  `style` still applies). That is exact, because a property's cascade depends only on the rules that declare it. Alternative if
  measuring shows matching is not the cost: memo the element match for `build_children`. Tests: the existing generated
  content / counters / quotes / Q19f tests unchanged; a test with a `display: none` set by a rule that declares nothing
  generated still prunes its counters. Gate: R58 recipe, `precompute_generated` median per gPS layout < 4 ms (from 10-22),
  total per layout -15 % or more; WPT no pass->fail.
- **QY54b3 [L]** (P2, Opus design first; supersedes/absorbs QY53o2) Subtree forced layout: when every node written since
  the last forced layout is inside a box that is a layout boundary in the last layout (abs/fixed positioned, definite
  `width` and `height`, `overflow` not visible or a size that cannot change from its content), and no CSS / viewport change
  happened, lay out only that subtree (style from its parent's cached computed style, measure at the cached containing block),
  splice its rects and skeleton into `GeomCache`, and leave the rest of the page as it is. On YouTube results the writes
  before a `getPlayerSize` read are all in the preview (player, its containers, its inline controls), and 70-75 % stay inside `#inline-preview-player` under
  `.ytdVideoPreviewPlayerContainerWrapper` (abs, 440x248, overflow hidden) (R58 section 2). The design must cover: which
  boxes qualify (a box whose own size depends on its content does not, nor does a parent with `display: contents`), hoisted
  descendants (QYTC1a's `SK_FIXED_IN` logic), the IO/`io` reads, `glyphs` for the subtree, counters/`::before` text that
  depends on document order (QY54b2's pass), and an exactness check (a test mode that compares a subtree layout with a full
  layout, rect by rect). Gate: R58 recipe, gPS layouts median < 10 ms (from 54-80) and the worst promise job of a preview
  start < 300 ms (from 685-1 137 ms); full-suite and WPT unchanged.
- **QY54b4 [S]** (P4, Sonnet; js_geom.rs `style_write_in_place`) Treat an absent old `style` attribute as an empty one, so a
  first `style="top: 112px; left: 96px"` on an abs box (YouTube's `.ytdVideoPreviewLoaderVideoPreviewContainer`) shifts in
  place like a change would. 14-19 % of the R58 reads follow a first `style`, but most of those change layout anyway (`display:
  none`, a width), so expect ~1 read per preview at most. Only after QY54b3's design (it may make this moot). Tests: as
  QYTC1a's, with the attribute absent before the write.
- No item for the QYTC1d class probes (6-7 of ~125 first writes had no verdict) or `aria-label` (a selector reads it): too
  rare to matter here.

### QY54b2 note: the prefiltered rule list does not pay (measured, no code change)

The premise of QY54b2 is that `precompute_generated`'s matching is *on top of* `build_children`'s own. It is not. Both use the same
per-layout `MATCH_MEMO` (set before the pre-pass runs), so the pre-pass pays the first, and only, match of every
(element, pseudo): the later passes hit the memo. Measured (temporary counters and `Instant`, release build of main 022830a plus the
counters, headless `--perf-report --seconds 15` on the YouTube results page, own wire, under flock, load 1-2.4, 45 forced layouts):
- Matching is 90 % of the pre-pass: walk 11.6-14.9 ms, of which `matching_declarations_for` 10.5-13.4 ms (3 calls per element:
  none, `::before`, `::after`). So the first half of the claim holds (the item's alternative is not needed).
- But `matching_indices_for` ran 7 745 times in the pre-pass and **once** more in the whole `build_children` (7 746 vs 7 745;
  8 454 vs 8 453). Everything `build_children` needs is already memoized by the pre-pass.
- Phase totals of the 45 layouts (main): precompute_generated 401 ms, build_children 780, measure_document 358, all 1 616 ms.

A matcher restricted to the rules that declare `display`/generated properties would make the pre-pass cheap (<4 ms), but its results
cannot seed the memo (they are partial), so `build_children` would run the full match for every element itself: the
~10.5-13 ms moves from one phase to the other, and the total goes up by the (small) cost of the extra restricted pass. The gate
"total per layout -15 %" cannot be met this way; the < 4 ms gate would be met only by relabelling work. Nothing was implemented.

What would reduce the total: the match itself (about 1.4 us per call, 7.7k calls a layout) must not be redone, i.e. a match memo that
survives from one forced layout to the next for elements whose own attributes, ancestors, preceding siblings and the rule
generation did not change (the same validity question as QY53n1c's hover memo, but for script forced layouts). With 45 % of
the reads following a node insert/remove, the unchanged part of the 5-6.5k elements is most of them. Proposed: QY54b2b (size M,
Opus design first: key = (node, rules generation, viewport width); invalidation by DOM mutation epoch per subtree/ancestor chain and
`:has`/sibling flags from `rules_cache::current()`; with `has`/`sib` rules present, drop the memo).

### QY54b1 note: script-inserted stylesheet links are applied (fixes section 3, with two remaining layout gaps)

What changed: a `<link rel=stylesheet>` that a script inserts, removes, or whose `href`/`rel`/`disabled` it changes is now
fetched through wire and applied at its document position, and fires `load`/`error` (`link_css.rs`, NOTES.md). Tested in the
nested dwl (960x1006 window), own wire, under flock, load 0.5-2.3, release build of the branch (a throwaway build of the same
code adds a `QMEAS_PROBE` script hook, as in this R; never committed). Recipe as above (pointer on result 1 at 15 s, result 2 at 21 s).

- YouTube results: the two Google Fonts links load at ~4.2-4.4 s and `www-player.css` at ~5.3 s. `#inline-preview-player`
  is `position: relative` and its rect is the same at every 250 ms read: 300x232 over result 1, then 300x232 over result 2.
  The preview plays over the hovered result; main's stray `1:34:31` label at the filter bar is gone (screenshots main vs branch).
- The width is 300, not section 3's 412: R58's own counterfactual (the sheet appended as a `<style>`, probe on this binary)
  also gives 300x232 today. The wrapper `.ytdVideoPreviewPlayerContainer` is `position: absolute; width: auto;
  aspect-ratio: var(--ytd-video-preview-aspect-ratio)` in the main CSS, and scratchy has no `aspect-ratio` for boxes, so it
  shrink-to-fits to the `<video>`'s 300 px. Not this item.
- A red dot remains at the preview's top-left corner: the inline controls' progress-bar playhead
  (`.ytProgressBarPlayheadProgressBarPlayheadDotHide { transform: scale(0) }`), placed at the top because its host
  `yt-inline-player-controls` (`position: absolute; inset: 0`) is laid out 0 px tall: scratchy keeps the content height of an
  absolutely positioned `height: auto` box with both `top` and `bottom` (CSS2.1 §10.6.4 says it fills the containing block).
- YouTube watch: it parses `www-player.css` and then inserts a second link to the same URL (written relatively). Browsers apply
  the copy again at its place and fire `load`. Doing that here blanked the player: after `load`, YouTube re-measures
  `#player-container` (`position: absolute; top: 0; bottom: 0` inside a padding-sized box), which scratchy lays out 0 px tall
  (the §10.6.4 gap again), and moves the video out of view. So a link whose resolved URL an earlier link already applies is not
  applied again and fires nothing (the parsed links are deduplicated the same way). A throwaway build with §10.6.4 implemented
  showed the player correctly sized but made `tp-yt-app-drawer`'s `#scrim` (inset 0, `visibility: hidden` inherited) cover the
  results page and take every hover: scratchy's hit test ignores `visibility: hidden` and has no `pointer-events`. Those three
  belong together in a follow-up item.
- YouTube watch (player region) and Wikipedia Rust are pixel-identical to main.

## R57: QY54a design: a helper thread keeps reading the Wayland socket while the main thread is busy, so the compositor never fills its send buffer for scratchy (R56 section 2: lost at ~190-225 unread motion events). libwayland supports this: several threads may read one `wl_display` (`wl_display_prepare_read_queue` / `wl_display_read_events`), and the events still go to winit's queue, which only the main thread dispatches. So input order and hit-testing do not change. winit 0.30.13 uses the system libwayland (`wayland-backend` `client_system`), gives the `wl_display*` through its raw display handle, and runs the same pattern itself in its `pump_events` notifier thread. A throwaway winit 0.30.13 + softbuffer 0.4.8 prototype in the nested dwl: without the thread, a 450 ms and a 3 s busy loop at ~460 Hz motion are lost at the first stall. With it, 3/3 busy450 and 3/3 busy3000 survive all stalls, no move is lost, the order is kept, and the cost is +0.08 s sys CPU per 15 s of motion (Opus, 2026-11-20, design only, docs only) — OPEN: proposed QY54a1-QY54a3 (below)

### 1. How scratchy talks to the compositor today

- lib.rs `run`: `EventLoop::with_user_event()`, then `event_loop.run_app(&mut app)`. On `Err(ExitFailure)` QB1 prints
  `lost the connection to the Wayland compositor` and exits 1 (`event_loop_exit_message`).
- Crates (Cargo.lock): winit 0.30.13, softbuffer 0.4.8, wayland-client 0.31.15, wayland-backend 0.3.17 (winit turns on
  `client_system`, so this is **libwayland-client** 1.26 here, not the pure-Rust backend), smithay-client-toolkit 0.19.2,
  calloop 0.13.0, calloop-wayland-source 0.3.0, raw-window-handle 0.6.2. scratchy itself has no direct wayland dependency and
  no code that touches the connection (grep `wayland|HasDisplayHandle` in src/: tests only).
- winit's Wayland loop is a calloop loop on the main thread. `WaylandSource::before_sleep` flushes, takes a read guard
  (`prepare_read`) and polls the socket together with the other sources. `before_handle_events` reads the socket **or drops
  the guard before any user callback runs** ("the guard isn't held whilst process_events calls occur"). `process_events`
  then dispatches the queue into `ApplicationHandler::window_event`/`user_event`/`about_to_wait`.
- So the socket is read only while the main thread sits in calloop. All of scratchy's work runs inside those callbacks:
  page scripts and timers (`about_to_wait` -> `run_due_slice`, `user_event` `StreamCss`/`JsNet`/...), forced layouts inside
  JS, App builds and paint (`RedrawRequested` -> `redraw`), HTML chunks, image decode results. While any of them runs, the
  compositor keeps writing `wl_pointer.motion` + `frame` + `relative_motion` into the socket. When the kernel buffer and then
  libwayland-server's per-client buffer are full (~190-225 moves on dwl), the server drops the client (R56 section 2).
- Pointer moves are already coalesced AFTER dispatch: `CursorMoved` only stores `pending_pointer_move` (QM1k1n6a). The newest
  position is processed once in `about_to_wait`, or first when a button/wheel/key event arrives (`event_flushes_pointer`).
  So a burst of thousands of moves after a stall costs a few µs each. That is not the problem; the bytes never reaching the
  client is.

### 2. Options

| option | feasible with today's crates? | invasiveness | risk | input order / hit-test |
|---|---|---|---|---|
| **A. drain thread**: a second thread does `prepare_read` -> `poll(fd)` -> `read_events` (no dispatch) on a second `Connection` made from winit's `wl_display*` | **yes, proven** (section 3). `RawDisplayHandle::Wayland` gives the `wl_display*`; `wayland_backend::client::Backend::from_foreign_display` + `wayland_client::Connection::from_backend` wrap it with a private, never-used event queue. winit's own `PumpEventNotifier` (event_loop/mod.rs, `pump_events` only) and calloop-wayland-source ("some other thread read events before us ... expected") are written for this. | ~150 lines in one new file + start/stop in app.rs; 2 direct deps already in Cargo.lock (no new crate, no network) | low. Teardown must join the thread before winit disconnects the display (section 4). | **unchanged**: libwayland puts every event in winit's queue in socket order; only the main thread dispatches; hit-testing happens at dispatch time as today |
| B. move JS + layout off the winit thread so the loop always runs | in principle | **XL**: the V8 isolate, the DOM (`Rc` everywhere), layout, `App` state and every `AppEvent` path would move to a second thread with a message protocol to the UI thread | high | needs a new input-forwarding design; hit-testing against a layout owned by another thread |
| C. drain from inside long work on the main thread (a watchdog thread sets a flag / `isolate.request_interrupt` every ~100 ms; the interrupt and checkpoints in layout/style/parse/decode call `prepare_read` + `read_events`, never dispatch) | yes (same foreign-`Connection` read, no re-entrancy into winit) | M, and it grows: every long non-JS stretch (forced layout, App build, stylesheet parse, image decode, paint, a future one) needs its own checkpoint | medium: one missed stretch = a disconnect, so the bug class stays open | unchanged (no dispatch) |
| D. abort or yield long JS / layout when input is pending | partly (JS only, via the budget/interrupt) | M | changes page semantics; does not bound non-interruptible work; the limit is ~0.4 s at 500 Hz, inside every sane budget | n/a |
| E. coalesce motion earlier | no: coalescing needs the bytes read first, and the server side is the compositor's | - | - | already done after dispatch (QM1k1n6a) |
| F. bigger buffers (client `SO_RCVBUF`, libwayland 1.23+ `wl_display_set_max_buffer_size`) | partly: the client call sizes the CLIENT's receive buffer; the limit that drops us is the server's per-client buffer (`wl_display_set_default_max_buffer_size`, the compositor's choice) | S | only moves the threshold; a 3 s forced layout still loses | n/a |

**Recommendation: A.** It is the only option that bounds every kind of main-thread stall (JS, forced layout, builds,
parse, decode, paint, and the next one) and needs no checkpoints. It leaves dispatch, input order and hit-testing exactly
where they are now. B is the long-term "UI thread" architecture, far bigger than this bug, and not needed for it. C is a
weaker A that has to be maintained.

### 3. Prototype (throwaway, scratchpad, not committed)

A standalone winit 0.30.13 + softbuffer 0.4.8 client built `--offline` against scratchy's Cargo.lock (so the same crate
versions). Every 4 s from t = 3.5 s it busy-loops `BUSY_MS` in `about_to_wait`. `DRAIN=1` starts the drain thread in
`resumed`. In `exiting` it stops the thread through a wake pipe and joins it. Own nested dwl (tools/nest.sh, libwayland
1.26). tools/inject draws a circle with `wait:2` between moves (R56 recipe; measured rate ~460 moves/s). Each run under
`flock /tmp/scratchy-perf.lock`, 1-min load average 5.4 at the first run, then 0.6-3.7 (other agents building; these are
pass/fail runs, not timings).

| run | drain | busy per stall | result |
|---|---|---|---|
| 1 | off | 450 ms | **lost at the first stall** (3.5 s): 2x `Io error: Broken pipe`, `run_app` -> `Err(ExitFailure(1))` |
| 1 | off | 3 000 ms | **lost at the first stall** |
| 3 | on | 450 ms | **3/3 survive** 16 s, 3 stalls each; 2 061-2 081 moves dispatched per 4 s period, the same as without stalls |
| 3 | on | 3 000 ms | **3/3 survive** 20 s, 3 stalls each; 3 273-3 283 moves per 7 s period |
| 2+2 | off / on | 0 (no stall) | 6 547-6 557 / 6 551-6 559 moves in 15 s (no loss); CPU user+sys **0.15-0.16 s / 0.25-0.27 s** |
| 1 | on | 3 000 ms, 15 s | 6 558 moves (= the no-stall count), **0 jumps > 30 px** between consecutive positions (the circle steps ~10 px; a reorder would jump) |
| 1 | on | compositor killed at 3 s | exits as today (`ExitFailure`, `exiting` ran, thread joined, no hang), but the thread logged ~29 extra `Io error: Broken pipe` lines before the main thread noticed: **the real thread must stop at the first error / POLLHUP** |

So the mechanism holds with the exact crates scratchy ships: the thread reads while the main thread is busy, the compositor
never sees a full buffer, and calloop dispatches what was read on the next turn (`before_sleep`'s `prepare_read` fails
because the queue is not empty, so it dispatches without polling). The thread also reads while the loop is merely in
dispatch callbacks (2 700 reads before the first stall), which costs ~0.1 s sys CPU per 15 s of 460 Hz motion (~0.7 % of
one core) and nothing when the pointer is still.

### 4. Design details (for QY54a2)

- **New file `src/wl_drain.rs`**, `pub(crate) struct WlDrain { stop: Arc<AtomicBool>, wake: OwnedFd (pipe write end),
  thread: Option<JoinHandle<()>> }`, `WlDrain::start(&ActiveEventLoop) -> Option<WlDrain>`, `fn stop(&mut self)` (set the
  flag, write one byte, join), `Drop` calls `stop`. `start` returns `None` for any non-`RawDisplayHandle::Wayland`
  handle (X11: no change) and when `SCRATCHY_WL_DRAIN=0` (A/B switch and safety valve; default on).
- **Getting the connection**: `event_loop.display_handle()?.as_raw()` -> `WaylandDisplayHandle.display` ->
  `unsafe { Backend::from_foreign_display(ptr) }` -> `Connection::from_backend`. The foreign backend makes its own empty
  `wl_event_queue` and never disconnects the display. SAFETY argument: winit's `Connection` owns the display and outlives
  the thread, because the thread is joined in `exiting` (below) and the foreign `Connection` is dropped on the thread
  before `join` returns.
- **Thread loop** (name `wl-drain`): `while !stop { let Some(g) = conn.prepare_read() else { break }; poll([conn fd,
  wake pipe], -1); if wake or stop { drop(g) /* cancel_read */; break } if POLLERR|POLLHUP { break } if POLLIN {
  match g.read_without_dispatch() { Ok(_) => {}, Err(WaylandError::Io(e)) if e.kind() == WouldBlock => {}, Err(_) => break } } }`.
  It never dispatches and creates no objects, so `prepare_read` on its private queue only fails on a broken display (then it
  stops). On any error it just stops; the main thread sees the same error in its own flush/dispatch and QB1 prints the
  one line as today.
- **Why it cannot deadlock** (libwayland 1.26 `read_events` / `cancel_read`): every thread that calls `prepare_read` adds
  one reader. The LAST reader to call `read_events` does the read, and the others wait for it. `cancel_read` also counts as
  done. calloop's guard is only held while the main thread sits in `poll` on the same fd, and it is released (read or
  cancelled) before any callback. So whenever the drain thread waits in `read_events`, the main thread is a
  prepared reader that will read or cancel within one poll wake-up (the fd is readable for both). When the main thread is
  busy, the drain thread is the only reader and reads at once.
- **Start/stop in app.rs**: start in `ApplicationHandler::resumed` right after the window and surface exist (field
  `App.wl_drain: Option<WlDrain>`). Add `fn exiting(&mut self, _: &ActiveEventLoop) { self.wl_drain.take(); }`. winit calls
  `exiting` (`Event::LoopExiting`) on a clean exit AND on the `ExitFailure` path, before `run_app` drops the event loop
  (verified in the prototype). **Panics**: scratchy unwinds (no `panic = "abort"`), and an unwind through `run_app` would
  drop the display while the thread may still poll it. Install a chained panic hook in `start` that stops the thread
  (through a `static` stop flag + wake fd). The hook runs before unwinding.
- **Memory**: events read during a stall wait in libwayland's client queue (~100-200 B per message). A 3 s stall at 500 Hz is
  ~4 500 messages (< 1 MB). A 60 s hang is ~15 MB. Acceptable; nothing to cap (the alternative is being disconnected).
- **Not changed**: dispatch order, `queue_pointer_move` coalescing, `event_flushes_pointer`, QCUR1 move throttling, key
  repeat (calloop timers on the main thread), `xdg_wm_base.ping` (still answered at the next dispatch; dwl does not kill on a
  late pong; a desktop may grey the window during a long stall, as for any busy client).
- Deps: `wayland-client = "0.31"` and `wayland-backend = { version = "0.3", features = ["client_system"] }` under
  `[target.'cfg(target_os = "linux")'.dependencies]`. Both are already in Cargo.lock at 0.31.15 / 0.3.17 with that feature
  (the prototype built `--offline`). No new crate, no network.

### 5. Proposed items (coordinator files them)

- **QY54a1 [S]** (P1, harness, Sonnet; tools/ only) Commit the R56 stall harness so QY54a2 and later changes can be gated
  the same way. `tools/wl-flood/`: `busy.html?ms=N` (a timer every 4 s busy-loops N ms, the R56 busyN fixture),
  `stall.html?k=N` (8 000 rows + a timer every 4 s doing N `style.width` write + `offsetHeight` read pairs, the R56 stall_kN
  fixture; k=0 is the no-stall control), and `tools/wl-flood/run.sh PAGE HZ SECONDS [RUNS]`. The script starts its own nested
  dwl (tools/nest.sh into a given dir), runs the given scratchy binary on the page with `SCRATCHY_FRAME_STATS=1`, drives a
  tools/inject circle at `wait:2` (500 Hz) or `wait:8` (125 Hz) from 1 s, and prints one line per run: `survived` /
  `lost at T s` (QB1's line in stderr) plus the frame-stats stall lengths. It kills only the PIDs it started. Document it in
  tools/README. Gate: on current main it reproduces R56: busy450 and stall_k18 at 500 Hz lost 3/3, stall_k0 and busy300 at
  500 Hz survive 3/3.
- **QY54a2 [M]** (P1, Sonnet; new src/wl_drain.rs, app.rs `resumed`/`exiting`, Cargo.toml) The drain thread as designed in
  REGRESSIONS.md R57 section 4: a `wl-drain` thread that does `prepare_read` / `poll` / `read_without_dispatch` on a
  `Connection` made from winit's `wl_display*` (`Backend::from_foreign_display`), never dispatches, stops at the first error
  or POLLHUP, is woken by a pipe, joined in `exiting`, and stopped by a chained panic hook. `SCRATCHY_WL_DRAIN=0` turns it
  off; non-Wayland handles get no thread. Tests (src/tests/wl_drain_tests.rs): (1) over a `socketpair`, a client
  `Connection` (`Connection::from_socket`, system backend) sends 300 `wl_display.sync` requests on a queue that is never
  dispatched; the test's "server" end writes 300 `wl_callback.done` (+ `delete_id`) events with a non-blocking socket. With
  a `WlDrain` on that display, all bytes are consumed (the server's writes never hit `EAGAIN` / `FIONREAD` on the client end
  goes to 0) while nothing is dispatched; then one `dispatch_pending` delivers the 300 callbacks in request order; without
  the drain the server side backs up. (2) `stop()` returns within 100 ms with the thread idle in poll, and after the server
  end closes (POLLHUP) the thread has exited by itself. (3) start with a non-Wayland handle (or `SCRATCHY_WL_DRAIN=0`) gives
  `None`. If the socketpair test turns out impractical (> ~200 lines), say so in the PR and gate on QY54a1 only. Gate (the
  QY54 overall gate): with the QY54a1 harness, **busy450 and stall_k18 at 500 Hz survive 3/3** in the nested dwl, and with
  `SCRATCHY_WL_DRAIN=0` the same binary is still lost 3/3 (proves the thread is what fixes it). Clicking a link after a
  stall_k18 stall still hits the link under the pointer (a click injected during the stall).
- **QY54a3 [S]** (P2, Opus measure, after QY54a2) Re-run R56 section 2's YouTube rows with the drain: results page at 500 Hz
  from 1 s and from 15 s, 3 runs each (lost 3/3 each in R56), plus the CPU cost: process CPU and frame-stats busy share for
  60 s of 500 Hz motion over a static fixture, drain on vs `SCRATCHY_WL_DRAIN=0`, 3 interleaved pairs under the perf flock
  (prototype: ~+0.1 s sys per 15 s). Gate: 0 lost of 6, CPU cost < 1 % of one core, numbers in R57. If the cost is higher,
  file an item that lets the thread sleep while the main thread is idle in calloop (a "main busy since" timestamp it checks).
- Not proposed: B (separate UI thread) is not needed for QY54. File it only if a later goal (a responsive scrollbar / input
  while a script runs) needs it; it would be an XL architecture change and **needs the user**. QY54b (why the results-page
  `getPlayerSize` layouts cost ~10x more) stays as filed: with QY54a2 those 0.7-0.98 s jobs no longer disconnect, but they
  still freeze input for that long.

### 6. QY54a2: the drain thread is in (gate MET on the fixtures; YouTube re-run is QY54a3)

`src/wl_drain.rs` as in section 4 (NOTES.md index has the architecture note). Differences from the section 4 sketch: the
wake "pipe" is a `UnixStream::pair` and `poll` is `rustix::event::poll` (rustix 1.1.6 with `event`, already in the lock
through wayland-backend), so the one `unsafe` is `Backend::from_foreign_display`. The deps sit in plain `[dependencies]`
(scratchy has no other target cfg anywhere). The panic hook only acts on panics of the thread that started the drain, and it
joins the thread (bounded, 1 s). scratchy catches some main-thread panics (js_geom's layout guard), so `about_to_wait`
starts a new drain when the hook stopped the old one and the app is still running. Cargo.lock gains only the three
dependency edges (no new package; built `--offline`).

Gate, release build of the PR branch, copied aside, `tools/wl-flood/run.sh PAGE 500 15 3` (own wire, own nested dwl,
perf flock per run). The nested dwl output was 960x489 that day (run.sh's default W/H 1052x1050 only scales the circle):

| page | drain | result | load avg (1 min) |
|---|---|---|---|
| busy450 | on | **survived 3/3** (7 024-7 026 moves dispatched, longest callback 446 ms) | 10.16 |
| stall_k18 | on | **survived 3/3** (6 730-6 812 moves, longest callback 1 524-1 538 ms) | 0.89 |
| busy450 | `SCRATCHY_WL_DRAIN=0` | lost 3/3 at 4.5 s (the first stall) | 0.90 |
| stall_k18 | `SCRATCHY_WL_DRAIN=0` | lost 3/3 at 5.9 s (the first stall) | 1.06 |

Click during a stall: stall.html (k=18) plus a fixed 300x250 `<a>` with a click handler. A 500 Hz circle runs until the
second stall starts, then one inject does `move` to the link's centre + `click` + a small 500 Hz circle inside the link.
3/3 runs: the click was injected 307-328 ms into a 1 336-1 354 ms stall, dispatched 3-4 ms after the stall ended, and
`e.target` was the link at the injected position (478,242 client px). With `SCRATCHY_WL_DRAIN=0` the same run is lost
during that stall. Normal use (Wikipedia Rust in the nested dwl, move/scroll, a link click navigates): `:q` exits rc 0 in
~370 ms. Killing the nested dwl gives rc 1 with the same three stderr lines as with the drain off (2x winit's
`Io error: Broken pipe` + QB1's one line); the thread logs nothing.

### 7. QY54a3: YouTube re-run and CPU cost with the drain (YouTube gate MET, CPU gate MISSED: +1.4 % of one core)

**Setup.** Release build (V8 + media) of origin/main 8c211e9 (#425, the drain is in), own CARGO_TARGET_DIR, copied aside.
Own wire (a copy of the installed binary) on a scratchpad socket and own nested dwl per run, `tools/wl-flood/run.sh` with
two scratchpad-only changes: the motion start is a variable (1 s or 15 s) and scratchy runs under a wrapper that reads
`/proc/<pid>/stat` (process and per-thread utime/stime) when run.sh stops it (`timeout --foreground`, so the sample is
taken before scratchy gets the signal). 500 Hz = `wait:2` circle. `SCRATCHY_FRAME_STATS` on every run, default UA,
`SCRATCHY_MEDIA_AO=null`. **Every run under `flock /tmp/scratchy-perf.lock`**; before each run the 1-min load was waited
down to < 2 (3 min at most); the load is in the tables. Another agent was building at times (spikes to 7-13 during some
runs, noted). Machine: Ryzen 7 5700X, 16 threads.

**(1) YouTube results** (`/results?search_query=rust`), 500 Hz. "load" = motion from 1 s for 60 s; "late" = motion from
15 s for 45 s (to 60 s). R56: lost 3/3 each.

| case | drain | result | longest callback | input->present max | load (1 min) at start |
|---|---|---|---|---|---|
| load | on | **survived 3/3** (27 396-27 756 moves dispatched) | 326-343 ms | 1.39 / 1.86 s (run 1: frame stats lost to a driver bug; harness line: gap 2.36 s, longest 334 ms) | 1.76, 1.66, 2.62 |
| late | on | **survived 3/3** (20 138-20 585 moves) | 882 / 1 053 / 1 061 ms (promise job) | 0.82 / 1.25 / 1.26 s | 1.98, 1.29, 1.32 |
| load | `SCRATCHY_WL_DRAIN=0` | lost 1/1 at 2.9 s (434 moves) | 293 ms | - | 1.07 |
| late | `SCRATCHY_WL_DRAIN=0` | lost 1/1 at 16.2 s (98 moves), on the first preview job (11 `getPlayerSize` layouts, 653 ms; job 727 ms) | 727 ms | - | 0.89 |

**0 lost of 6 with the drain; the drain-off baseline still loses at the same places as R56.** Drain thread CPU on YouTube:
0.34-0.56 s per 60 s (user 0.11-0.17, sys 0.23-0.39). Process CPU 57-65 s per 60 s (the page keeps one core busy:
frame-stats busy share builds + paint/present + page JS = 87 % (load runs) and 79-83 % (late runs, whole 60 s)).

**For QY54b (input freeze on YouTube results).** After load, with the pointer over the results, the inline-preview player
runs one promise job every ~2-3 s with **13-15 `g.k.getPlayerSize` forced layouts of ~61-63 ms each (0.91-0.95 s), the
job 0.98-1.06 s** (`event:abort (anonymous) fetch:793:51` or no source), and in between an `observer_delivery` /
`event:durationchange` timer second with 2-5 more `getPlayerSize` layouts and 5-11 builds. One second of late run 3 had
29 `getPlayerSize` layouts (1.81 s, two jobs). Over the 45 s: 223 / 355 / 317 `getPlayerSize` layouts, 12.5 / 22.0 /
19.6 s. **The longest input freeze is ~1.06 s (input->present max 1.26 s)**, repeating every 2-3 s. During load the longest
freeze is the page-script second at 2-3 s (redraw gap 1.4-2.7 s, input->present max 1.86 s), not `getPlayerSize`. The
first preview job (t ~16 s) is shorter (11 layouts, 0.65-0.72 s). The 5.6-7.9 s "redraw gap" at 15.5-16 s in the late runs
is the idle page before the motion starts (no redraw needed), not a freeze (input latency in that second 70-200 ms).

**(2) CPU cost**, a static fixture (60 `<p>` of text, no script, no hover rule), 60 s of 500 Hz motion from 1 s, process
CPU at the end (61 s of process life). Pairs 1-3 are the item's 3 interleaved pairs; pairs 4-5 were added on a quieter
machine because pairs 1 and 3 had load spikes during the drain-on run.

| pair (order) | drain on: process user+sys | of it: main / wl-drain | drain off: process | of it: main | delta | load start (on / off), spikes |
|---|---|---|---|---|---|---|
| 1 (on, off) | 2.01 s (23 454 moves) | 1.35 / 0.56 | 1.10 s (28 019) | 1.00 | +0.91 s | 0.58 (13.5 during) / 3.76 |
| 2 (off, on) | 1.66 s (28 007) | 1.13 / 0.42 | 1.12 s (28 008) | 1.03 | +0.54 s | 1.79 / 1.93 |
| 3 (on, off) | 2.07 s (24 334) | 1.39 / 0.53 | 1.04 s (28 089) | 0.95 | +1.03 s | 1.97 (7.4 during) / 1.95 |
| 4 (off, on) | 2.03 s (27 685) | 1.38 / 0.54 | 1.04 s (28 096) | 0.95 | +0.99 s | 0.37 (3.5 at end) / 0.99 |
| 5 (on, off) | 1.48 s (28 094) | 0.99 / 0.39 | 1.05 s (28 082) | 0.95 | +0.43 s | 1.97 / 0.59 |

- **Cost of the drain: +0.43 to +1.03 s per 60 s of 500 Hz motion = 0.7-1.7 % of one core** (pairs 1-3: +0.54-1.03 s,
  mean +0.83 s = **1.4 %**; all 5: mean +0.78 s = 1.3 %). That is **above the < 1 % gate** and higher than the prototype's
  +0.1 s per 15 s (~0.7 %). Without the drain the same motion costs 1.04-1.12 s (1.7-1.9 %), so the drain roughly doubles
  what pointer motion over an idle page costs.
- About half is the `wl-drain` thread itself (0.39-0.56 s, mostly sys: `poll` + `read`), the other half is **extra work on
  the main thread** (+0.04 to +0.44 s, user and sys both up): while the main thread is idle in calloop, both threads poll
  the same fd, so every motion batch wakes both; libwayland makes one of them read and the other wait or cancel, and the
  main thread then dispatches with more, smaller wake-ups. The cost is only paid while the main thread is idle, i.e. exactly
  when the drain is not needed.
- Frame-stats busy share is **0.0 %** with and without the drain (builds + paint/present + JS = 10-18 ms per 61 s; the
  static page does not redraw on motion). Event dispatch is not in frame stats, so the cost shows only in process CPU.
- With the pointer still the drain costs nothing (the thread sleeps in `poll`).

**Verdict.** QY54's user-visible goal is met: with the drain YouTube results at 500 Hz survives 6/6 (R56: 0/6), and the
baseline without it still loses 2/2. The CPU gate is missed (1.4 % vs < 1 %), so per section 5 the follow-up below is
proposed. The YouTube page still freezes input for ~1 s every 2-3 s while the pointer is over the results (QY54b).

Proposed item (coordinator files it):

- **QY54a4 [S]** (P2, Sonnet; src/wl_drain.rs + app.rs only) The wl-drain thread sleeps while the main thread is idle in
  calloop. Measured (R57 section 7): the drain adds +0.43-1.03 s per 60 s of 500 Hz motion over a static page (0.7-1.7 %
  of one core, mean 1.4 %; gate < 1 %), half in the thread's own poll/read and half as extra main-thread wake-ups, because
  both threads poll the Wayland fd while the main thread is idle. Fix: the main thread publishes a "busy since" timestamp
  (`AtomicU64`, monotonic ns; set at the start of each event-loop iteration in `ApplicationHandler::new_events`, set to 0
  at the end of `about_to_wait` just before returning to calloop). The thread does not poll the Wayland fd while the stamp
  is 0 or younger than ~50 ms; it waits on its wake socket with a ~50 ms timeout and re-checks, and only when the main
  thread has been busy longer than that does it do today's `prepare_read` / `poll` / `read_without_dispatch` until the
  stamp goes back to 0. 50 ms at 500 Hz is ~25 moves, far below the ~190-225 that get scratchy dropped. Keep stop/wake,
  POLLHUP and the panic hook as they are. Tests: the existing wl_drain_tests still pass with the main thread marked busy;
  new: with the stamp 0 the thread consumes nothing (the socketpair server's bytes stay unread) and with a stamp older
  than the threshold it drains. Gate (`tools/wl-flood/run.sh`, release build, perf flock): busy450 and stall_k18 at 500 Hz
  still survive 3/3, YouTube results at 500 Hz from 15 s survives 3/3, and on a static page 3 interleaved pairs of 60 s
  of 500 Hz motion, drain on vs `SCRATCHY_WL_DRAIN=0`, differ by < 0.6 s process CPU (< 1 % of one core).

## R57 section 8: QY54a4 (the wl-drain thread sleeps while the main thread is idle): CPU gate MET

Main publishes a "busy since" `AtomicU64` (`WlDrain::busy` in `ApplicationHandler::new_events`, also once in `resumed` after the
drain starts; `WlDrain::idle` as the last statement of `about_to_wait`, which has no early return). The stamp is non-zero for the
whole loop iteration, so a stall in any callback after `new_events` (window event, user event, timer slice, forced layout in
`about_to_wait`) is seen. The thread sleeps on its wake socket (`poll` with a 50 ms timeout, or the rest of the 50 ms threshold)
while the stamp is 0 or younger than 50 ms; once older it does `prepare_read` / `poll` (20 ms timeout, so it goes back to sleep
soon after main does) / `read_without_dispatch` as before. Stop, POLLHUP (only watched while main is busy; main sees the same
error in its own read) and the panic hook are unchanged. Not covered: a stall inside winit's own dispatch before `new_events`
(short, winit-internal).

Release build of the branch, own wire and nested dwl, `flock /tmp/scratchy-perf.lock` per run (run.sh holds it itself; the
60 s runs used the QY54a3 wrapper, which also samples per-thread CPU from /proc). Load (1 min) at the start: 0.2-3.2.

| gate | result |
|---|---|
| `run.sh busy450 500 20 3` | survived 3/3 (8885-9340 moves, longest callback 448 ms, redraw gap max 4.3 s) |
| `run.sh stall_k18 500 20 3` | survived 3/3 (8561-8676 moves, longest callback 1.39-1.53 s) |
| YouTube results, 500 Hz, motion from 15 s, 60 s | survived 3/3 (27 105-28 006 moves, longest callback 0.97-1.34 s) |

Static page, 60 s of 500 Hz motion, 3 interleaved pairs (process CPU = user + sys, whole run incl. startup):

| pair (order) | drain on | drain off (`SCRATCHY_WL_DRAIN=0`) | on - off | wl-drain thread CPU |
|---|---|---|---|---|
| 1 (on, off) | 1.02 s | 1.03 s | -0.01 s | 0.00 s |
| 2 (on, off) | 1.04 s | 1.03 s | +0.01 s | 0.00 s |
| 3 (on, off) | 1.05 s | 1.27 s | -0.22 s | 0.00 s |

On - off is -0.22 to +0.01 s per 60 s (gate < 0.6 s); R57 section 7 with the always-polling thread: +0.43 to +1.03 s (mean +0.83 s).
The thread itself rounds to 0.00 s (it wakes ~20 times a second while main is idle). Pair 3's "off" run is noise (the same
binary without the thread normally costs 1.03-1.12 s).

## R56: Quiet-machine re-measurement of QY60, QY54 and QY33 on V8 main. QY60: FIXED. On YouTube results the masthead is finished at 4.3-4.6 s (R38: 47-53 s with a 20 s budget), all 20 results are in by 3.3-4.4 s, the worst rAF wait is 623-634 ms (R38: 3.6 s) and the worst timer lateness 701-725 ms (R38: 4.1 s). Forced layout is 0.6-1.0 s per 10 s headless (QV8-m: 14.6 s). The main thread is saturated only for ~5 s of load (page JS ~0.8-0.95 s per second), then idle. NEW (FIXED by QY57r: ioEval0 no longer clips by a propagated body overflow; thumbnails now paint): the thumbnails still show `true`: scratchy's IntersectionObserver clips every target by `<body>` (YouTube's body is 956x0 with `overflow: scroll`), so every entry has an empty intersection and YouTube never sets `src`. QY54: STILL OPEN. The nested compositor drops scratchy when about 190-225 pointer-motion events queue up unread: 3/3 lost with a 450 ms busy loop at 500 Hz, which is INSIDE the 500 ms callback budget. YouTube results at 500 Hz: lost 3/3 during load and 3/3 after load, on the inline-preview player's `getPlayerSize` layouts (0.7-0.98 s in one promise job). At 125 Hz it survives 3/3. QY33: FIXED. In 21 headless runs (home, results, watch) and 9 window runs: 0 budget kills, and the largest callback after load is 130 / 233 / 264 ms budget clock (327 ms raw) (Opus, 2026-11-20, measure only, docs only) — OPEN: proposed QY57r, QY54a, QY54b (below)

**Setup (all three items).** Main b88b3db (its code is 86635e3: the commits after it change only TODO files). The binary is
a release build (V8 + media) of a `git archive` export of main in the scratchpad. It has ONE throwaway line, never committed:
when the env var `QMEAS_PROBE=<file>` is set, that JS file is run after the preludes and before the page scripts
(`start_scripts_clocked`). With the variable unset the binary is main. All probe-free numbers come from runs without the
variable. The injector (tools/inject) and the binary were built before any measurement, each with its own CARGO_TARGET_DIR.
Own wire (a copy of the installed binary) on a scratchpad socket, restarted for every run (fresh cookie jar). Own nested dwl
(tools/nest.sh), window **960x1006**, `SCRATCHY_MEDIA_AO=null`, `SCRATCHY_FRAME_STATS` on every window run, default UA.
**Every run was under `flock /tmp/scratchy-perf.lock`.** I was the only agent on the machine (16 threads, 31 GB).
The 1-min load average at the start of the runs was **0.09-0.90**. Headless = `--perf-report --socket <own wire>`, the
PERF.md suite command (`--seconds 10` for results, 30 for home and watch), budgets 500 and 20000 interleaved.
**Machine note:** R37/R38 and PERF.md QV8-m were measured on the old 8-thread Ryzen 5 PRO 2500U. Their absolute ms are not
comparable 1:1, but the changes below are 5-20x, far beyond a machine difference.

### 1. QY60: main-thread throughput on YouTube results — FIXED (close)

Page: `https://www.youtube.com/results?search_query=rust`, 60 s window, no input, 3 runs without the probe (frame stats) and
3 with a rAF/rIC/setTimeout latency probe (R38 style: it wraps the three functions before the page scripts run, and a 1 s
ticker logs when `ytd-masthead.masthead-finish` and the result renderers appear; t = 0 is engine start, after the HTML
arrived).

| | R38 (2026-11-18, QuickJS, 20 s budget) | now (3 probe runs) |
|---|---|---|
| `masthead-finish` (the grey circles go away) | 47.3 s, 52.8 s (not by 40 s in a 3rd run) | **4.6, 4.4, 4.3 s** |
| results stamped | 4 results by ~60-90 s | 3 at 1.1-1.2 s, 11-12 at 2.2 s, 18-20 at 3.2-3.4 s, 20 at 4.3-4.4 s |
| max rAF latency (request -> callback) | 3 619 ms | **633, 634, 623 ms** (all in the first 5 s) |
| rAF callbacks | ~0.6-0.7/s for 87 s | 31-45 in the first 5 s, then ~0.2/s (YouTube stops asking; nothing waits) |
| max setTimeout lateness | 4 101 ms | **710, 725, 701 ms**; 0 timers > 1 s late |
| max rIC latency | 3 350 ms | 408, 153, 316 ms |

Where the window's main thread goes (frame stats, whole 60 s, 3 probe-free runs):

| run | builds n / ms | page JS ms (forced n / ms) | paint ms | busy (builds+JS+paint) | busy in the first 10 s | redraws | longest single callback |
|---|---|---|---|---|---|---|---|
| 1 | 52 / 1 693 | 4 308 (31 / 797) | 208 | 6 210 ms (11 %) | 6 060 ms (61 %) | 83 | 273 ms |
| 2 | 54 / 1 797 | 4 691 (32 / 944) | 190 | 6 680 ms (11 %) | 6 507 ms (67 %) | 81 | 269 ms |
| 3 | 52 / 1 672 | 3 147 (33 / 904) | 211 | 5 032 ms (8 %) | 4 870 ms (52 %) | 82 | 267 ms |

Per second, the load looks the same in all 3 runs: from t = 3-4 s to t = 7-8 s page JS is 0.72-0.95 s of every second
(forced layout 0.1-0.36 s of that), then 1-2 s of App builds (0.47-0.74 s per second), and from t = 8-10 s the thread is idle
(js_ms < 30 per second). So during those ~5 s the thread is saturated, and **page JS outside forced layout dominates**
(2.1-3.6 s of the 3.0-4.5 s JS in the first 10 s). App builds come second (1.7-1.8 s). Forced layout is third (0.8-0.9 s,
31-33 layouts). No single callback goes over 273 ms. That is a ~5 s load, not a throughput problem. YouTube's scheduler gets
its rAF/rIC/timer ticks within 0.7 s at worst.

Headless (`--perf-report`, 10 s, 3 runs per budget; QV8-m for comparison):

| | QV8-m 8dfe2f9 (old machine) | now, budget 500 | now, budget 20000 |
|---|---|---|---|
| budget kills | 0 | 0, 0, 0 | 0, 0, 0 |
| largest callback, budget clock | 446-460 ms | 205, 233, 218 ms (promise jobs) | 199, 218, 219 ms |
| forced layout total (count) | 14 561-14 569 ms (23-25) | **993 (37), 896 (35), 715 (28)** | 1 025 (32), 636 (27), 801 (30) |
| forced layout share of JS CPU | 70 % | 17-22 % | 18-19 % |
| of it from scratchy's own IntersectionObserver (`io`) | - | 874 of 993 ms | 877 of 1 025 ms |
| JS errors | 0 | 0 | 0 |

(Run b500-1 got a different YouTube front-end, `base_module_set` instead of `kevlar_base`, load CPU 180 ms vs 660 ms. Its
forced-layout and callback numbers are in the same range.) The QY53 series (getPlayerSize, `::first-line` scan, IO recheck,
QY53m no-op builds) took forced layout from ~70 % of the JS thread to ~20 %. Most of what is left is the `io` reason.

**NEW, found here: the thumbnails still paint `true`** (grim at 57 s, all 3 runs, viewed). QY56 and QY57 are merged, and
TODO-DONE says real thumbnails showed after QY57. Throwaway probes (3 diagnostic runs, not measurements):
- 41-47 `img.ytCoreImageHost`, **0 with `src`** at 10 s and at 25 s.
- YouTube's lazy-image observer (`rootMargin: "10%"`, `threshold: .01`) got 835-882 entries over 10-11 observers.
  **Every one** had `isIntersecting: false` and `intersectionRect` 0,0,0,0. The first img's `boundingClientRect` was
  291,112 20x230: on screen, well inside `rootBounds` -95.6,-100.2 1147x1202.
- Walking up from that img, three ancestors have a clipping `overflow`: `a#thumbnail` (hidden, 291,112 20x230: contains
  the img), `ytd-search` (hidden, 72,56 884x7687: contains it) and **`body` (`overflow: scroll`, 0,0 956x0)**.
  js_dom.rs `ioEval0` clips by every ancestor whose computed `overflow-x/-y` is not visible (it skips only `gRoot`). So the
  0-height body box empties every intersection. In CSS, `body`'s `overflow` propagates to the viewport when the root
  element's is `visible` (CSS Overflow 3, overflow viewport propagation). Body then does not clip at all. Chrome shows these thumbnails.
- So YouTube's filter (`intersectionRect.height > 0`) drops every entry, `onViewportEntered` never runs, and no `src`
  is set. That is R38 section 1 again, now with a different cause. Not bisected: whether a later change made YouTube's
  body `overflow: scroll` reach `ioEval0`, or YouTube changed its CSS.

### 2. QY54: does the compositor still disconnect scratchy under today's stalls? — STILL OPEN (yes; cause narrowed)

R8 recipe: tools/inject moves the pointer in a circle over the window with a `wait:2` (500 Hz) or `wait:8` (125 Hz) between
moves, starting 1-2 s (fixtures, YT "load") or 15 s (YT "late") after launch. "lost" = scratchy exits with QB1's `lost the
connection to the Wayland compositor` (after two `Io error: Broken pipe`). Fixtures (scratchpad, not committed):
**stall_kN** = 8 000 rows plus a timer every 4 s that does N `style.width` write + `offsetHeight` read pairs. Each pair is one
whole-document forced layout of ~170-210 ms, which the callback budget does not count (QJ4). **busyN** = a tiny page whose
timer busy-loops N ms every 4 s (inside the 500 ms budget).

| page | motion | stall per timer (measured in the window) | result |
|---|---|---|---|
| stall_k0 (same DOM, no stall) | 500 Hz | 0 | 2/2 survive 30 s |
| busy100 / busy200 / busy300 / busy375 | 500 Hz | 100 / 200 / 300 / 375 ms | 2/2 survive each (7 stalls per run) |
| **busy450** | 500 Hz | 450 ms | **2/2 lost at the first stall** (3.5 s) |
| stall_k2 / k3 / k5 / k7 / k10 | 500 Hz | 582-590 / 738-755 / 1 050 / 1 360-1 405 / 1 814-1 827 ms | **2/2 lost each**, at the first stall |
| stall_k18 ("3 s forced layout", the item's fixture) | 500 Hz | 3 061-3 074 ms | **3/3 lost** |
| stall_k2 / k5 / k7 | 125 Hz | 313-587 / 841-1 052 / 1 124-1 346 ms | 1/1 survive each |
| **stall_k10 / stall_k18** | 125 Hz | 1 814-1 842 / 3 003-3 074 ms | **2/2 lost each** |
| YT results, motion from 1 s | 500 Hz | page load: HTML parse + page scripts (475 ms JS in that second; 2.0 s between redraws) | **3/3 lost at 2.4-2.7 s** |
| YT results, motion from 15 s (after load) | 500 Hz | see below | **3/3 lost at 17.2-19.7 s** |
| YT results, motion from 1 s | 125 Hz | longest per-second callback 272-279 ms; gap_max 2.0-2.4 s | **3/3 survive 60 s** |

**The threshold is a count of unread motion events, not a time:** survive at 375 ms x 500 Hz (~188 moves) and 1.35 s x
125 Hz (~168), lost at 450 ms x 500 Hz (~225) and 1.81 s x 125 Hz (~227). So it is lost at **~190-225 queued moves**,
whatever stalls the thread (JS busy loop or forced layout). That is the compositor's per-client send buffer (each move is
`wl_pointer.motion` + `frame` + `zwp_relative_pointer_v1.relative_motion`, R8). libwayland-server here is 1.26 (nested dwl).
The user's session compositor may allow a different count (not measured).

What this answers:
- **Yes, today's stalls still disconnect.** QB3's 500 ms callback budget is no bound at 500 Hz: a 450 ms callback that the
  budget allows is enough. Forced layout makes it worse (not counted, any length), but **a forced-layout cap alone would not
  fix it**, and neither would QY49-D option (d) (a budget is a time; the limit is ~0.4 s at 500 Hz, ~1.5 s at 125 Hz).
- **YouTube results after load (the realistic case): the pointer over a result starts YouTube's inline-preview player.**
  In the fatal second of all 3 "late" runs, one promise job did **12-15 `g.k.getPlayerSize` forced layouts of ~58 ms each
  (693-857 ms; the job 809-984 ms)** (frame stats `forced.by_caller` and `longest`). That is the R43 pattern on the results
  page. In headless watch-page runs `getPlayerSize` layouts are ~5 ms each (39-42 per run, 207-226 ms in all). The window's
  results-page layout costs ~10x more each (a 7 600 px tall document; not split further).
- At 125 Hz YouTube results survived 60 s with the pointer moving, but the main thread was busy 74-80 % of the minute
  (builds 415-491 / 18.0-19.8 s, page JS 23.9-27.9 s with 192-234 forced layouts / 9.5-11.9 s; process CPU 54-59 s of 60).
  Without input the same page is 8-11 %. Input-to-present latency max 1.15-1.57 s. That is QY53n/QYTC-type hover work and
  the preview player's layouts.

### 3. QY33: YouTube post-load timer callbacks over budget — FIXED (close)

R26's numbers (QuickJS, home page, first callback after load 764-1 416 ms, then 500-684 ms, 1-2 `timer stopped` + 2-4
`promise jobs stopped` per load) re-measured on V8. Headless `--perf-report`, 3 runs per (page, budget); "POST" = the
callback ended after the `load` event.

| page (s) | budget | kills | largest callback (budget clock / raw), kind | largest after load | load CPU |
|---|---|---|---|---|---|
| home (30) | 500 | 0, 0, 0 | 128, 128, 129 / same, observer delivery | same | 662-670 ms |
| home (30) | 20000 | 0, 0, 0 | 125-130, observer delivery | same | 658-660 ms |
| results (10) | 500 | 0, 0, 0 | 205-233 / 224-267, promise job | same | 180*, 659, 660 ms |
| results (10) | 20000 | 0, 0, 0 | 199-219 / 231-260 | same | 658-666 ms |
| watch G1AeBgalFZk (30) | 500 | 0, 0, 0 | 258, 258, 264 / 317, 321, 327, timer | same | 979-992 ms |

Window (frame stats, 60 s, no input): home 3 runs, results 6 runs (section 1): the longest callback of any second is
**252-279 ms** (a page-load classic script); 0 seconds with a callback > 500 ms; no `timer stopped` / `promise jobs stopped`
line in any stderr; 0 `js error` lines in the window runs (headless watch has 2 per run, both lottie-light: a null canvas context, not budget-related). **Gate "no callback > 500 ms on an idle simulated viewport": met with ~2x margin on
all three pages.** Caveat: today's home page (logged out, fresh jar) is YouTube's "Der YouTube-Verlauf ist deaktiviert"
card with no video grid. That is lighter than R26's page, so results and watch stand in for the heavy case. The kebab, the
"Anmelden" pill and the guide icons that R28 section 4 / QY33 lost to the budget all paint (grim, home and results).

Top callbacks today (none over budget; for the record, so a later regression can be named):
1. Watch: `d` in `web-animations-next-lite.min.js` (the animation polyfill's rAF tick, R26/R37), **317-327 ms raw / 258-264
   budget**, 4 forced layouts (59-64 ms), at ~5.5 s.
2. Results/home: the `yt-page-data-fetched` promise job (`(anonymous)` in ytmainappweb kevlar_base), 186-233 ms budget,
   3-4 forced layouts.
3. Results: `bound onYtAction_` promise job 205 ms; observer deliveries 126-193 ms (the largest on home).
4. Load-time scripts (own 5 s budget): watch `inline script #28` 337-350 ms raw with 25-27 forced layouts; kevlar_base 190-281 ms.

### Proposed items (coordinator files them)
- **QY57r [S]** (P1, js_dom.rs `ioEval0`; Sonnet) IntersectionObserver clips by `<body>` even when body's `overflow`
  propagates to the viewport, so on YouTube results (body `overflow: scroll`, box 956x0) every entry has an empty
  `intersectionRect`, YouTube never sets an img `src`, and every thumbnail paints `true` (R56 section 1). Do what CSS
  overflow propagation says: when the root element's computed `overflow` is `visible`, `body`'s overflow applies to the
  viewport and body does not clip (and the root element's own overflow never clips, as `gRoot` already does). If scratchy's
  cascade/layout also lacks this propagation, check the paint/scroll side too (out of scope unless it is wrong there).
  Tests: a target inside `<body style="overflow:scroll;height:0">` under a visible-overflow html reports a non-empty
  intersection; a target inside a `div` with `overflow:hidden; height:0` still reports empty; body clips when html has
  `overflow: hidden`. Gate: YouTube results in the nested window: imgs with `src` > 0 by 10 s and real thumbnails in grim.
- **QY54a [M]** (P1, Opus design first, then Sonnet; app.rs / winit Wayland) Keep reading the Wayland socket while the main
  thread is busy, so the compositor never sees a full client buffer. Measured (R56 section 2): it is lost at ~190-225 unread
  motion events (~0.4 s at 500 Hz, ~1.5 s at 125 Hz). That is inside today's 500 ms callback budget, so no budget or
  forced-layout cap can bound it. Design options to weigh: (1) a helper thread that drains the socket into the client-side
  queue (libwayland's `wl_display_prepare_read`/`wl_display_read_events` are thread-safe by design; get the `wl_display` from
  winit's raw display handle, and the main thread still dispatches); (2) pump winit events from inside long work (re-entrancy
  risk); (3) interrupt long JS/forced layouts when input is pending. Gate: stall_k18 (3 s forced layout) and busy450 with
  500 Hz motion survive 3/3 in the nested dwl (R56 table), with no change in input order.
- **QY54b [S]** (P2, Opus measure) YouTube results: the inline-preview player started by the pointer over a result runs one
  promise job with 12-15 `getPlayerSize` forced layouts of ~58 ms each (0.7-0.86 s). In headless watch runs the same reads
  cost ~5 ms each. Measure why a results-page layout costs ~10x more here (document size? not in place? the QYTC1a in-place
  path refused?) and file fixes. With the pointer moving at 125 Hz the page keeps the main thread 74-80 % busy for a minute
  (vs 8-11 % idle). Gate: numbers in R56 and fix items.
- No item for QY60's remaining load (~5 s saturated, page JS outside forced layout dominates): it is a normal page load
  now; QOBS9/QPROF1 re-profile it anyway.

## R55: Quiet-machine re-measurement of QY53m (#386), QMEM3d (#400) and QY53n3 (#416). QY53m: gate MET. With JS on, the App rebuilds that followed only no-op writes go from 54 to 0 per run on Wikipedia and from 123-155 to 0 on YouTube watch. That is builds 133 -> 104 (-22 %) and 12.2 -> 9.4 s on Wikipedia, 371 -> 268 (-28 %) and 14.3 -> 10.1 s on YouTube. The Wikipedia screenshots are byte-identical. QMEM3d: gate MET. After leaving YouTube watch for a small local page, RSS is 104-117 MB lower 3 s after the navigation and 119-135 MB lower after 35 s (gate >= 25 MB). QY53n3: gate met only trivially, NO EFFECT on YouTube watch. No YouTube hover change in two pointer recipes involves an OFFSET rule, so `hover.patched` is 0 before and after. The remaining hover builds are refused for other reasons: Layout-classified rules (73 % of builds with the yt input, 33 % with yt2: opacity, `border-radius`, `fill` and `background` through `var()`, `border` restating the same width/style), `hover_match_diff` = None (27 % / 45 %: the kept match memo is stale after a `<style>` splice, or an element was never rendered), and list items (yt2: 22 %) (Opus, 2026-11-20, measure only, docs only) — OPEN: proposed QY53n2c, QY53n2d, QY53n1c, QY53n1d, QMEM3l (below)

**Setup (all three items).** Main 3361054. Every binary is a release build (V8 + media) of a `git archive` export of main
into the scratchpad, plus THROWAWAY instrumentation that is identical in all variants (never committed): (a) per-`js_sync`
batch flags: a write at one of QY53m's seven value-checked sites marks the batch "real" or "no-op", and every other
`dirty.set(true)` site marks it "real". Two frame-stats counters come from that: `noop_batches` (js_sync turns whose writes
were ALL no-op) and `noop_rebuilds` (those turns that still queued a restyle). (b) On every hover build, `eprintln` of WHY the
cheap path refused it, plus the non-paint rules in the change set (selector + declared fields). (c) Env `QMEAS_NO_TRIM`
skips `mem_trim::spawn_trim` (the request is still counted). The "before" binaries are the same tree with ONE change undone:
**bm** = QY53m reverted (the seven `d.set(d.get() | changed)` back to `d.set(true)`); **bn** = QY53n3 reverted
(`patch_with_rules` refuses every effect but Paint again; then the offset path and the `moved` refresh in app.rs never
run). That restores the exact pre-change behaviour while keeping the counters identical. Separate CARGO_TARGET_DIRs. All
builds were done before the measurement and nothing else ran. Own wire (copied binary) on a scratchpad socket, restarted
per run (fresh cookie jar). Own nested dwl (tools/nest.sh), window **960x1006**. `SCRATCHY_MEDIA_AO=null`,
`SCRATCHY_FRAME_STATS`, default UA. **Every run under `flock /tmp/scratchy-perf.lock`.** Load average (1 min) at the start
and end of the runs was **0.14-1.38** (the only agent running). Before/after runs are interleaved, and the order flips every
round.

**Input (tools/inject, R47 recipe adapted to 960x1006).** *Wikipedia* Rust (programming language), JS on, 52 s: from
t = 12 s, 6 rounds x 8 links x 600 ms (`general-purpose`, `performance`, `type safety`, `concurrency`, `memory safety`,
Talk, Edit, View history), then the pointer is parked on body text, with a screenshot at 50 s. *YouTube* watch G1AeBgalFZk,
56 s: a click on the big play button at 13 s, then from 15 s 6 rounds x 8 positions x 600 ms (2 points in the picture,
the progress bar, the play/pause button, settings, search button, menu, "Anmelden"), then parked, with a screenshot at
54 s. *yt2* (item 3 only, 60 s): the same click, then 6 rounds x 10 positions (3 on the progress bar, CC, settings,
play/pause, "mehr", 2 recommendation thumbnails, a recommendation title).

### 1. QY53m-m: js_sync skips the restyle after no-op write batches (ee4897f #386) — gate MET

Whole-run totals from frame stats, 3 interleaved pairs per page.

| run | builds n / ms | noop_batches | noop_rebuilds | forced n / ms | js_ms | hover restyles (all patched on WP) |
|---|---|---|---|---|---|---|
| WP after 1 / 2 / 3 | 104 / 9 393, 104 / 9 441, 103 / 9 345 | 54, 54, 54 | **0, 0, 0** | 22 / 2 734, 21 / 2 639, 21 / 2 640 | 3 239, 3 144, 3 145 | 38, 38, 38 |
| WP bm (before) 1 / 2 / 3 | 132 / 12 177, 133 / 12 241, 133 / 12 274 | 54, 54, 54 | **54, 54, 54** | 22 / 2 820, 21 / 2 629, 21 / 2 636 | 3 336, 3 141, 3 147 | 26, 26, 26 |
| YT after 1 / 2 / 3 | 268 / 10 088, 267 / 10 056, 279 / 10 287 | 285, 273, 276 | **0, 0, 0** | 234 / 7 604, 233 / 7 541, 234 / 7 615 | 14 100, 14 075, 14 379 | 30, 31, 30 (built) |
| YT bm (before) 1 / 2 / 3 | 363 / 13 737, 375 / 14 336, 371 / 14 454 | 123, 155, 147 | **123, 155, 147** | 230 / 7 497, 193 / 5 826, 196 / 6 012 | 13 951, 12 204, 12 421 | 23, 25, 22 (built) |

Medians: **Wikipedia builds 133 -> 104 (-29, -22 %), 12 241 -> 9 393 ms (-2.85 s); YouTube 371 -> 268 (-103, -28 %),
14 336 -> 10 088 ms (-4.2 s).** The no-op rebuild requests drop to 0 on both pages. On Wikipedia the build count falls by 29,
not by the 54 requests, because the requests coalesce: a request that comes while a build is in flight, or in the same
turn as a real write, does not add a build (QB2). The gate ("builds.n and builds.ms drop by the no-op count") is met in
that sense: every build that only no-op writes caused is gone. Side effect, with the same input: with fewer pending JS
builds, more hover changes take QY53n2's patch path instead of waiting for a build. Wikipedia `hover_restyles` is 38 (all
patched) after vs 26 before. YouTube's `noop_batches` is higher after (273-285 vs 123-155) because nothing coalesces them
into builds any more; the count is per js_sync turn. **Screenshots:** the 6 Wikipedia end screenshots are byte-identical
(same md5) across before and after. On YouTube, outside the player box, they differ only in the recommendations column
(x >= 674), and that column differs just as much between two "after" runs (thumbnails load in a different order). Forced
layouts and page JS time do not change (QY53m does not touch the geometry generation).

### 2. QMEM3d-m: malloc_trim after navigation / idle (8ce75f2 #400) — gate MET

The "after" binary, with (`trim`) and without (`notrim`, `QMEAS_NO_TRIM=1`) the trim. YouTube watch: play at 13 s, a few
pointer moves until 18 s, idle until 40 s, then `:go <scratchpad>/pages/small.html` (a 2-line local page). VmRSS of the
scratchy process from /proc. "nav+N" = N s after the `:go` Enter. 4 trim requests per run (start, `:go`, 2 idle periods).

| run (order) | RSS on YT at 38 s | nav+3 | nav+6 | nav+10 | nav+15 | nav+25 | nav+35 |
|---|---|---|---|---|---|---|---|
| trim 1 (1st) | 509 | **306** | 305 | 280 | 199 | 199 | **199** |
| notrim 1 (2nd) | 513 | **423** | 423 | 331 | 331 | 328 | **328** |
| notrim 2 (1st) | 513 | **410** | 410 | 410 | 410 | 312 | **312** |
| trim 2 (2nd) | 476 | **310** | 310 | 289 | 289 | 193 | **193** |
| trim 3 (1st) | 519 | **314** | 314 | 289 | 289 | 193 | **193** |
| notrim 3 (2nd) | 517 | **420** | 420 | 420 | 420 | 325 | **325** |

(MB.) **With the trim, RSS is 104-117 MB lower at nav+3 (medians 310 vs 420) and 119-135 MB lower at nav+35 (193 vs 325).
Gate >= 25 MB: met by ~4-5x.** For scale: a fresh scratchy on the same small page is **98 MB** (3 runs: 97.6 / 97.5 /
97.5 MB after 20 s). So with the trim, ~95-100 MB is still held after leaving YouTube, and ~215-230 MB without it. In
BOTH variants a second step of ~90-100 MB is freed 10-25 s after the navigation (nav+10 -> nav+25), not at the
navigation. Something of the old page (the mpv player and its threads? the V8 isolate's deferred dispose? a cache TTL?)
lives on for 10-25 s: QMEM3l below.

### 3. QY53n3m: offset hover patch (09ea0d5 #416) — gate met only trivially; no effect on YouTube watch

Frame-stats hover counters, totals per run. "after" = main, "bn" = QY53n3 reverted. Three series: the item-1 series (yt
input), a v2 series (yt input, with per-build refusal reasons), and a v3 series (both inputs, with the reason of a
`hover_match_diff` = None split out).

| run | hover restyles | skipped | built | **patched** | builds n / ms |
|---|---|---|---|---|---|
| yt after (item-1 series) 1 / 2 / 3 | 30, 31, 30 | 0, 0, 0 | 30, 31, 30 | **0, 0, 0** | 268 / 10 088, 267 / 10 056, 279 / 10 287 |
| yt bn 1 / 2 / 3 | 32, 31, 29 | 0, 0, 0 | 32, 31, 29 | **0, 0, 0** | 281 / 10 360, 280 / 10 481, 266 / 10 565 |
| yt after (v2) 1 / 2 / 3 | 34, 31, 33 | 0, 0, 0 | 34, 31, 33 | **0, 0, 0** | 275 / 10 434, 272 / 10 348, 273 / 10 331 |
| yt bn (v2) 1 / 2 / 3 | 31, 27, 33 | 0, 0, 0 | 31, 27, 33 | **0, 0, 0** | 277 / 10 381, 269 / 10 374, 276 / 10 465 |
| yt2 after (v3) 1 / 2 / 3 | 39, 35, 39 | 10, 11, 10 | 29, 24, 29 | **0, 0, 0** | 346 / 11 933, 342 / 11 549, 340 / 11 870 |
| yt2 bn 1 / 2 / 3 | 55*, 39, 35 | 25*, 11, 8 | 30*, 28, 27 | **0, 0, 0** | 141* / 4 123, 335 / 11 619, 339 / 11 824 |

(* yt2 bn 1 is an outlier: 141 builds and 26 no-op batches vs ~550. YouTube served a different page state that run, so
leave it out.) Before and after are the same within noise: **QY53n3 patched no YouTube hover with either input.** Why each
hover BUILD was refused on main (the reason that the last cheap attempt before the build returned), totals over 3 runs
per input:

| refusal (main) | yt, v2 (98 builds) | yt, v3 (92 builds) | yt2, v3 (82 builds) |
|---|---|---|---|
| `a changed rule is not paint/offset-only` (Effect::Layout) | 72 | 67 | 27 |
| `hover_match_diff` = None: the kept match memo is for another rule list (YouTube spliced a `<style>` after the last build) | 26 (v2: one "no diff" bucket for both None reasons) | 15 | 19 |
| `hover_match_diff` = None: an affected element the last build never matched (not rendered) | (in the 26) | 10 | 18 |
| `a list item or form control` (QY53n2b's case) | 0 | 0 | 18 |
| any OFFSET refusal (positioned descendant, inline-level box, % / calc() inset, auto-width or auto-margin absolute, edge change, sticky/fixed, pseudo-element, document height) | **0** | **0** | **0** |

**None of the four candidate refusals named in the item
(positioned descendants, inline-level boxes, % insets, auto-width absolutes) occurs.** No YouTube hover change in these
recipes reaches `plan_moves` at all: no changed rule set holds an OFFSET-class rule. YouTube moves its progress-bar pieces
from JS `style` writes, not from `:hover` rules (R50). The Layout-classified rules in the refused change sets (each counted
once per build; yt + yt2, v2 + v3):

| rule (selector, declared fields) | builds | really layout? |
|---|---|---|
| `.ytSpecButtonShapeNextCallToAction.ytSpecButtonShapeNextOutline:hover:not(...)` {`background: var(..)`, border-color} | 53 | no: `background` through `var()` is classified by its name, and the shorthand is not in PAINT_PROPS |
| `yt-icon-button.ytd-masthead:hover, ...topbar...:hover` {`border-radius`, `background-color: var(..)`} | 49 | no: `border-radius` only paints (its clipping is a paint concern too) |
| `.ytSearchboxComponentSearchButton:hover` {background-color, `border-width`, border-color, `border-style`} | 53 | only if the width/style values differ; the hover rule restates the same `border: 1px solid` with another colour |
| `.ytp-chrome-controls .ytp-button:not(...):hover, ...` {`opacity`} | 60 | paint, except opacity 1 <-> < 1 starts/ends a stacking context (QYTC1f's caution) |
| `.ytp-cued-thumbnail-overlay:hover .ytp-large-play-button-bg, ...` {`fill: var(..)`} | 15 | no: SVG `fill` only paints |

The original QY53n3 gate ("hover builds whose differing rules are all PAINT/OFFSET -> 0") holds only because no hover
build has an all-PAINT/OFFSET change set by scratchy's `Effect` classification. R47's 36-of-58 count used a wider paint
list (border-radius, opacity, visibility, cursor as paint). So on YouTube, the offset widening candidates (re-hoisting
positioned descendants, % insets through the containing-block basis) are worth nothing today. The classification gaps
and the stale kept memo are what keep YouTube hovers on the build path.

### QY53n2c note (Sonnet, 2026-11-20): resolved-value classification, measured
Hover rules are now judged by the resolved `ComputedStyle` change (`Effect::Value`, hover_patch.rs `value_same`), and a list
item or form control no longer refuses the patch when its inherited text paint is the same in both states (the YouTube
buttons). Same R55 yt input (960x1006, own wire/dwl, `flock`, uptime load 1-25 at the starts, other agents building), 3
interleaved pairs, main (installed binary) vs the branch: hover restyles / built / **patched** per run: branch 22/10/**12**,
26/10/**16**, 22/12/**10** (38 patched of 70, 54 %); main 24/24/0, 26/26/0, 26/26/0 (all 76 built). Builds per run are 177-292
vs 220-311 (noisy: YouTube state). Gate was >= 40 patched over 3 runs: 38, narrowly missed. What still builds (debug run,
`eprintln` of the refusal, not committed): about a third `hover_match_diff` = None (stale memo after a `<style>` splice or an
element never rendered: QY53n1c/QY53n1d), the rest a build already pending (`layout_dirty`/`restyle_requested`, page JS busy)
and a few `.ytp-button` opacity rules (QY53n2d). Before the list/control relaxation the same recipe patched 6 per run.

### QY53n1c note (Opus, 2026-11-20): the kept memo was cleared by forced layouts, not by `<style>` splices
Design-first measurement (throwaway `eprintln` of why `with_kept_build` / `hover_match_diff` gave up, of every
`rules_cache` splice and of every build that dropped the kept memo; main b2599bb, same yt input as above, own
wire/dwl, `flock`, load 1.2-2.7). Splices happen only during the load (77 -> 22 467 -> 22 472 rules in the first
seconds); after that the rule list never changed. Every `hover_match_diff` = None but one was `LAST_MATCH` = None:
`stash_last_match` CLEARED the App's kept memo at the end of every build that was not the App's, i.e. every script
forced layout (`GeomCache`, 117 of them in one 58 s run vs 207 App builds). The one other None was an element the App
build never matched (`<svg>`, QY53n1d). So the options of the item (remap the memo across a splice, re-match only the
spliced sheet's rules, keep it when the splice adds nothing that can match) would have changed nothing on this page.
**Fix:** a non-App build leaves `LAST_MATCH` alone (it changes nothing on screen; the memo still describes the App's
tree, and every DOM write still makes the App dirty, which the hover path already refuses). Because a forced layout
can now re-derive `RULE_DERIVED` for a list that a splice changed in place (one rule for one: same pointer and length),
the kept memo is also keyed by `rules_cache`'s generation (`rules_cache::generation_of`): any splice or re-parse after
the App build -> None -> build, as before. A splice remap is NOT done (0 occurrences after load). Tests
`qy53n1c_forced_layout_keeps_hover_patching` (patch after a forced layout equals a rebuild; fails on main) and
`qy53n1c_style_splice_after_build_falls_back_to_build` (same-count edit and an added `:hover` rule; fails without the
generation key). **Result, 3 interleaved pairs** (same throwaway counter in both binaries, `flock`, load 0.6-2.7):
hover refusals with `hover_match_diff` = None main 21/17/3 = **41**, branch 0/0/2 = **2** (gate: drop >= 10);
`hover.patched` main 13/10/17 = 40, branch 20/20/19 = **59**; `hover.built` main 13/14/9 = 36, branch 6/6/6 = 18.
The rest of the hover builds: a build already pending (`layout_dirty`/`restyle_requested`) and 6 patch refusals per
run (an element not rendered or animated, a rule changing more than paint).
**Total builds (coordinator check).** In those 3 pairs the branch had MORE builds per run every time (main 214/209/268,
branch 264/276/314). Two more interleaved pairs on main 6cdc00a vs the branch rebased onto it, with a second throwaway
counter that is the same in both binaries: each App build logs WHY it ran (a bit per `layout_dirty` site: script DOM write
in `js_sync`, the script restyle deferred while a pass was in flight, animation tick, `set_interaction_state` = a hover
or focus build, image relayout, document/CSS arrival, `:checked`, resize), and every `GeomCache` forced layout is counted.
`flock`, load 0.7-1.3. App builds per run, by reason:

| run | all | script restyle after in-flight pass | script write | both | + image | hover/focus | document | forced layouts |
|---|---|---|---|---|---|---|---|---|
| main 4 | 213 | 111 | 65 | 18 | 4 | 13 | 2 | 272 |
| branch 4 | 150 | 80 | 39 | 18 | 6 | 5 | 2 | 202 |
| branch 5 | 219 | 117 | 69 | 18 | 4 | 9 | 2 | 229 |
| main 5 | 234 | 145 | 58 | 12 | 5 | 12 | 2 | 227 |

This time the branch has FEWER builds in both pairs (150 vs 213, 219 vs 234). Over all 5 pairs: main 1138, branch 1223
(+7 %), direction 3:2. The run-to-run spread is in the script-driven categories (script writes and script restyles
deferred behind an in-flight pass): 143-220 per run whichever binary. The change cannot touch these. It only decides
whether a hover goes to a patch or a build, and it does not set or clear any dirty flag. The one category it can affect,
hover/focus builds, is down on the branch (13+12 -> 5+9). No build ran without a reason bit (a dirty flag left set would
show as one): 0 in all 4 runs. A stale memo that led to a wrong patch and then a corrective build would show as an extra
hover/focus or script build right after a `patched` hover. Hover/focus builds went down, and the script categories vary as much between two runs of one binary
(main 198 vs 220, branch 143 vs 208). Hover in these runs: `hover_match_diff` = None main 20/13, branch 3/6; patched main 15/14,
branch 10/23. (Note: the frame-stats file of the first 2-pair attempt was appended to, because `inject` was missing and
those runs got no input. The App build counts above come from the per-run log, which is overwritten each run.)

### Proposed items (coordinator files them)
- **QY53n2c [S]** (P3, style.rs `decls_effect` + hover_patch.rs; Sonnet) Classify by the resolved VALUE change, not by the
  declared property, for the cases measured here: `border-radius`, SVG `fill`/`stroke`, the `background` shorthand (also
  through `var()`) when the computed change is only `background-color`, and `border-*-width`/`border-*-style` declarations
  whose computed values equal the old ones (compare the old and new ComputedStyle of the affected element: only paint
  fields may differ). The patch must still match a rebuild pixel for pixel (the QY53n2 tests style). Gate: YT watch, this
  section's yt input, under flock: `hover.patched` >= 40 of ~95 hover builds per 3 runs (the search button, masthead
  icon buttons, CTA button and play-button-bg sets), builds per run down by the same count.
- **QY53n2d [S]** (P4, after QYTC1f's design) `opacity` hover changes: patch when old and new are both < 1, or both 1 and
  nothing else changes, or the element has no positioned descendants and already forms a stacking context; otherwise
  build. 60 YT builds per 6 runs are the `.ytp-button` opacity rule.
- **QY53n1c [M]** (P3, style.rs `with_kept_build` / rules_cache; Opus design first) Keep the kept match memo valid across a
  QY53f1 `<style>` splice. YouTube splices a `<style>` after the last build, so the memo's rule list no longer matches and
  the hover builds (15-19 per 3 runs). Options: re-key the memo when the splice only appended rules that cannot match the
  affected elements (check the new rules' RuleIndex buckets), or keep a rule-index remap.
- **QY53n1d [S]** (P4) `hover_match_diff` -> None when an affected element was never matched by the last build (10-18 per
  3 runs): if it is inside a `display:none` subtree (no box), its match change cannot change the picture. Count it as
  skipped (exactness: only when an ancestor is display:none in the build's styles).
- **QMEM3l [S]** (P3, Opus measure first) Leaving YouTube frees ~100 MB of RSS 10-25 s AFTER the navigation, with or
  without the trim (table 2: nav+10 -> nav+25). Find what lives on (mpv player/threads, V8 isolate teardown, a cache TTL)
  and release it at the navigation. Today RSS after leaving YT is 193-199 MB vs 98 MB for a fresh start.
- The offset-patch widening candidates named in QY53n3m (re-hoisting positioned descendants, % insets) are NOT proposed:
  0 occurrences on YouTube watch. Revisit only if a page with `:hover` offset rules shows up.

## R54: QY53f1d: YouTube's cold stylesheet parse is no longer ~130 ms. Since QY53f1c it is ~35-55 ms on a quiet machine (the 4.4 MB sheet, 22 344 rules), and it now runs in the first `getComputedStyle`, not in a forced layout. But the same parse ran 3-4 MORE times per minute: QYTC1a's `!important` geometry check (js_geom.rs `IMP_GEO`) kept a clone of the rule list's `Rc`, so every `<style>` that YouTube inserts after a forced layout did a full re-parse instead of QY53f1's splice. FIXED here (the check keeps selectors + a generation, not the `Rc`): 4-5 full parses per 60 s -> 1. Design for the rest: parse the external CSS on a worker thread as soon as it is fetched (QY53f1d1) and build the RuleIndex once per rule list instead of twice (QY53f1d2). Lazy selector parsing is not worth it (Opus, 2026-11-20) — FIXED: the re-parses; OPEN: QY53f1d1-QY53f1d3 (proposed)

**Setup.** Main bd1ead7, own worktree release build (V8 + media), own wire on a scratchpad socket, default UA,
`SCRATCHY_MEDIA_AO=null`, https://www.youtube.com/watch?v=G1AeBgalFZk. Headless: `--perf-report --budget-ms 20000
--seconds 10|60`. Window: own nested dwl (tools/nest.sh), 45 s. Every measured run under `flock /tmp/scratchy-perf.lock`.
Other agents were building, so the machine load varied a lot (`uptime` load 1.5-30 on 16 cores; noted per table).
Throwaway instrumentation (reverted, never committed): an `Instant` around `rules_cache`'s full-parse branch that also logs
WHY it fell back (`cold`, `js-changed`, `concat`, `layered`, `rc-shared` = `Rc::get_mut` failed, with the strong count),
per-piece parse times, per-call totals in `QualifiedRuleParser::parse_prelude` (selectors) and `parse_block` (the
lazy-block token scan), and a bench mode that parses the dumped external CSS of the page 7 times in-process.

### 1. Where the cold parse goes (bench, the dumped 4 402 766-byte external CSS, load 1.5-1.9)
| step | ms (rounds 2-7) |
|---|---|
| cssparser tokenize-only of the whole sheet (recursing into every block) | 18.7-20.4 |
| `parse_stylesheet_into` (all of it) | **34.0-37.0** (round 1: 45.0) |
| - selector lists (`SelectorList::parse`, 22 344 rules) | 12.5-13.5 |
| - block token scan (finds the block end, `gen_hint`) | 12.2-13.2 |
| - `StyleRule::lazy` (copy the block text) | 1.3-1.4 |
| - rest (top-level tokenizing, `@media`/`@supports` preludes, pushes) | 8.0-9.3 |
| `rule_flags` | 1.2-1.8 |
| `RuleDerived::new` (RuleIndex, `:has`/`:empty`/pseudo scans, AttrReaders) | 13.1-17.6 |
| `HoverKeys::build` | 2.4-3.3 |
| dropping the parsed list | 3.0-3.4 |
| the same parse on a spawned thread, incl. spawn + join | 36.0-40.5 |
Declarations are NOT in this: QY53f1c (b276712) made them lazy, which is why the 126-132 ms of R43 section 12 is gone.
The tokenizer alone is ~55 % of the parse; selectors are ~35 %.

**In the page (headless, 10 s runs):** the first full parse is UA + 5 `<style>` pieces (under 5 KB together, 0.1 ms) +
the 4.4 MB external piece. External piece 37-47 ms, whole full parse 42-54 ms at load 18-27 (runs 4-5); 99-265 ms in
runs 1-3 at load 3-21 (others' builds starting: this is where "~130 ms" readings come from). It runs inside YouTube's
first `getComputedStyle` (`js_style::StyleCache::refresh` -> `cached_rules_for(Extra::Ver)`), so it shows in a
`classic_script`/`timer` callback, not in `forced_layout.max_ms` (37-145 ms in those runs, all style/box build).
The first forced layout after it then pays `rule_index` (RuleDerived + HoverKeys) 16-17 ms (34-96 ms under load), and
`StyleCache::adopt_shared` builds a SECOND RuleIndex of the same list (`RuleIndex::build`, ~13 ms) for getComputedStyle.
In the window the order differs: the App builds before the CSS arrives (77 UA rules), and the 4.4 MB piece is then
parsed by the splice path; same cost.

### 2. The extra full parses (the bug fixed here)
Every run had more full parses after the cold one, all `reason rc-shared strong 2`, each after YouTube inserted a
`<style>` (5 -> 8 -> 9 -> 10 -> 11 pieces). The second owner of the rule list was QYTC1a's `IMP_GEO` thread-local
(`Rc<(Rc<Vec<StyleRule>>, Vec<u32>)>`, "the `Rc` keeps the pointer unique"), set after every forced layout. With it,
`rules_cache`'s splice (`Rc::get_mut`) always fails and the whole 4.4 MB is re-parsed (QY53f1b's 126 hit / 3 splice /
1 full split from before QYTC1a became 1 cold + 3-4 full re-parses per minute). Fix (js_geom.rs
`current_important_geo`): the cache keeps the matching rules' `SelectorList`s (servo_arc clones, a few rules) and the
list's `rules_cache` generation, never the list's `Rc`; it returns `None` (= no in-place move, a normal layout) when
the latest build's list is no longer `rules_cache`'s current one. Test:
`qy53f1d_important_geo_check_keeps_style_inserts_incremental` (a `<style>` insert after a forced layout is spliced:
1 piece parsed, 0 full parses; an `!important left` rule that arrives in the inserted piece still blocks the in-place
move). It fails on main (full parse 1).

**Result, 3+3 interleaved 60 s headless runs** (main = instrumented bd1ead7, fix = same + this change):
| run | load | full parses | their ms | forced layouts | forced total ms | rule_index ms |
|---|---|---|---|---|---|---|
| main 1 | 14 | 4 | 422 | 92 | 3615 | 149 |
| fix 1 | 15 | **1** | 134 | 94 | 3765 | 140 |
| main 2 | 20 | 5 | 591 | 125 | 5695 | 190 |
| fix 2 | 24 | **1** | 57 | 91 | 1279 | 60 |
| main 3 | 11 | 4 | 161 | 94 | 1343 | 53 |
| fix 3 | 3 | **1** | 43 | 102 | 1527 | 50 |
Full parses 4-5 -> 1 in every pair; 120-530 ms of main-thread parse per minute removed (37-120 ms each, by load).
The other totals are dominated by YouTube's own spread (R43 section 14) and the load, not by this change.
Window (nested dwl, 45 s, load 7-9): main 2 extra full parses (41 ms each), fix 0; screenshots equal apart from
YouTube's recommendations.

### 3. What the first layout needs, and the options
- Every selector is needed before the first layout: the RuleIndex keys every rule by its rightmost compound and the
  first build matches the whole document. Declarations are already parsed on first use only (QY53f1c).
- **Lazy selectors** (key-scan the prelude tokens for the index, parse the full selector on first candidate match):
  saves at most the selector share, 12.5-18 ms, and the ~20 ms tokenizer floor stays. Five whole-list scans walk every
  selector (`rule_flags`, `scan_child_deps`, `scan_pseudo`, `AttrReaders`, `HoverKeys`) and would all need a lazy
  variant. L-size for ~13 ms: NOT recommended.
- **Off-thread parse of the external CSS (recommended, QY53f1d1):** the CSS text is complete when `fetch_stylesheets`
  returns (perf.rs headless, the App's navigation thread, `start_stream_resources`), while scripts are still being
  fetched (hundreds of ms to seconds on YouTube). Parse it there on a worker and hand the result to `rules_cache`:
  the main thread then pays nothing for the 4.4 MB (35-55 ms quiet, 100+ ms loaded, once per navigation). The bench's
  thread run shows the parse is just as fast off the main thread. What it needs: `StyleRule: Send` (Cell/OnceCell are
  Send; `Atom` is a `String`; selectors are servo_arc; check `Declarations` with a compile-time assert, it is only
  filled eagerly under `--css-debug`); the `@keyframes` registry holds `Rc`, so the worker returns the raw
  `KeyframesDef`s and the main thread builds the registry; the parse's `STATE_RULES` mask is returned and OR-ed in
  (as `with_mask` does); under `css_debug_enabled()` no preparse. `rules_cache` takes the preparsed rules when the
  external piece's text equals the preparsed text (a 4.4 MB `memcmp` is ~0.2 ms) and waits for an unfinished
  worker instead of parsing twice. Wrong-result risk is low: the same parser function runs, only on another thread.
- **Build the RuleIndex once per rule list (QY53f1d2):** `StyleCache::adopt_shared` (getComputedStyle) and
  `RuleDerived::new` (layout) each build a RuleIndex of the same list: ~13 ms quiet (35-95 ms loaded) wasted on
  each rule-list change (cold + every `<style>` insert).
- **Splices still rebuild all derived data (QY53f1d3, lower):** a 1-rule `<style>` insert rebuilds RuleDerived +
  HoverKeys for all 22 470 rules (~16-20 ms quiet). An index that shifts the rule numbers after the spliced range
  and adds/removes only the piece's rules would make an insert cost the piece. Measure the remaining splice count
  first (3-4 per minute on YouTube now).

### 4. Proposed items
- QY53f1d1 [M] (P3) parse the page's external CSS on a worker as soon as it is fetched; `rules_cache` adopts the
  result for the identical external piece (design above). Gate: headless YT, the first full parse's external piece
  <= 2 ms on the main thread, 5 runs; tests: preparsed list equals an in-thread parse (selectors via `to_css`,
  specificity, order, `@media`/`@supports`, layer, `!important`, keyframes, state mask); a text mismatch parses
  normally.
- QY53f1d2 [S] (P3) one RuleIndex per rule list: StyleCache uses the layout's RuleDerived index when the generation
  matches (or the derived data moves into `rules_cache`). Gate: `RuleIndex::build` runs once per generation on YT.
- QY53f1d3 [M] (P4) incremental RuleDerived/HoverKeys for a `<style>` splice. Measure first.
- Note for QFLK2: `qy53e2_app_layout_gives_the_same_entries_as_a_forced_layout_without_forcing` failed here alone
  at load ~30 and passed at low load: the IO deadline is `max(100, 4 * __d_geom_cost())` ms of WALL-clock layout
  cost, so a slow debug layout (> 27.5 ms) moves the entry from 150 to 160.

## R53: QPT1 design: a same-origin iframe gets its own V8 realm (a second `v8::Context` in the page's isolate, made from the startup snapshot), the way Chrome does it. This fixes R52: BotGuard reads real built-ins from `iframe.contentWindow`. Split into QPT1a-QPT1f (Opus, 2026-11-19, design only, docs only) — OPEN: QPT1a is the BotGuard slice

**Status: design only. No product change.** The user approved it on 2026-11-19 ("ok, lets do what Chrome does then"). This
was written from the source at b717000 (QV8-6, V8 is the only engine) plus one throwaway spike that was NOT committed:
two natives in `js_engine_v8.rs` (`__spike_ctx(mode)` makes a context and returns its global, `__spike_heap()` runs
`low_memory_notification` and then reads `used_heap_size`) and one `js_dom_tests.rs` test that drove them. Both were reverted
with `git checkout`. Debug test build (the Rust side is unoptimised, the V8 library is the prebuilt release one). Load average
23-37 (other agents building), so the times below are rough.

### 1. What exists today (b717000)
- **One isolate, one context per page.** `Engine` (`js_engine_v8.rs`) holds `context: v8::Global<v8::Context>`, and
  `Engine::with` always enters it. The startup snapshot (QV8-4) holds two contexts. The default context is EMPTY. Context
  **0** is the page context, baked by `js::bake_engine`: every native is registered and every prelude has run (PRELUDE, timers,
  url, globals, intl, fetch, websocket, DOM, cssom, media, mse), over an empty `about:blank` document. A page engine does
  `Context::from_snapshot(0)`, re-attaches the native closures by name (`register`: a baked native is a function whose data is
  its arena index, and the arena is per ISOLATE), and then applies the page state with `__scratchy_apply_env(env)`.
- **Natives are per isolate, not per context.** A native's closure captures the page's `DomTable`, `JsRequests`, net and
  ws state. A context deserialised from snapshot 0 gets its OWN function objects for every native, and those objects point at
  the SAME closures. The DOM natives take node ids (`__d_tag(id)`), and every document of the page lives in one `DomTable`
  (the page is id 0, and second documents come from `__d_newdoc`). So a second realm can drive the iframe document through the
  natives it already has.
- **The DOM prelude binds its realm to id 0.** `cache[0] = doc; Object.defineProperty(doc, '__id', { value: 0 })` is
  non-configurable. `inPage` tests `n.__id === 0`. `owners[..] || doc` falls back to the realm's document.
- **Event-loop hooks go into the page context only.** Rust calls `__scratchy_timers_next`/`__scratchy_timers_run_one`
  (timers), `__net_complete` (fetch/XHR), `__ws_ev*` (WebSocket) and `__scratchy_rej_flush` through `Engine::with`, which
  means the page context. `reject_callback` calls `__scratchy_rej_push` in the CURRENT context.
- **The iframe "window" (QA18b, `js_dom.rs` `frameOf`)** is `Object.create(null)` with ten own properties. Its `document` is
  `makeDoc()`, a parent-realm `Object.create(doc)` over a `__d_newdoc()` id. It is made lazily on the first
  `contentDocument`/`contentWindow` read, and only while the iframe is connected and its src is empty, `about:blank*`, `data:`
  or pre-fetched (QA18b1). Network iframes give `null`. Iframe content is never rendered, and no script inside an iframe runs.

### 2. Spike results (second context in the page's isolate)
| check | result |
|---|---|
| `Context::from_snapshot(0)` a 2nd..6th time in the page isolate | works, **~1.2-1.5 ms** each (`Context::new`, empty: 0.36 ms) |
| V8 heap per realm (used, after a full GC) | **~0.65 MB** (667 128 B for the first, 666 190 B each for the next four). The page context is 2.0 MB at the same point |
| drop every JS reference to 4 realms, then GC | heap −2.66 MB: all 4 freed. No Rust-side state was kept, so a realm dies like any object |
| no shared security token | `w.String` → **`TypeError: no access`**. V8's global proxy is access-checked, and with no access-check callback `MayAccess` passes only when the tokens match |
| same token (`c.set_security_token(page.get_security_token())`) | `typeof w.String` `function`, `w.String !== String`, `[] instanceof w.Array` **false**, `w.Array.isArray([])` true, a function made by `w.eval` returns `w.Array`s |
| interfaces in the realm (from the snapshot) | `HTMLElement`, `Navigator`, `SubtleCrypto`, `SVGRectElement`, `HTMLIFrameElement`, `navigator.userAgent`, and `w.Object.getOwnPropertyDescriptor` all present. `WebKitCSSMatrix` is undefined, in the page as well |
| `Object.prototype.toString.call(w)` / `String(w)` | `[object Object]`. It does not throw any more (today: TypeError). `Window` has no `Symbol.toStringTag`: QPT3 |
| the realm's `document` | **`__id` 0 = the PAGE document.** `w.eval('document.body.appendChild(...)')` showed up in the page. A second wrapper for the same node (`w.document.body !== document.body`) |
| the realm's `location`, `parent` | `about:blank` (the baked env), `w.parent === w` |
| `w.setTimeout(f, 0)` | **never fires.** The realm's timer list is never pumped. The page's own timer fired |
| promise job queued from the realm | runs: one microtask queue per isolate, as in a browser |
| `Promise.reject` in the realm | **never reported**: it went to the realm's own `__scratchy_rej_push` queue |
| internal names | the realm has 510 own global names, the page 519 (all `__d_*` etc., the QPT5 gap) |

So a context from snapshot 0 already gives BotGuard everything R52 section 2 lists (`String`, `Object`, the interface
constructors, `getOwnPropertyDescriptor` through the realm's `Object`). What it needs on top of that is a token, a document
binding, a frame binding and the event-loop forwarding below.

### 3. What Chrome/Blink does (the model)
- Every frame on a renderer's main thread shares ONE isolate. Each `LocalDOMWindow` has one `v8::Context` per world (main
  world here), made from Blink's V8 context snapshot (`V8ContextSnapshot`, `Context::FromSnapshot`). That is what
  `from_snapshot(0)` is here. The context's global proxy is the `WindowProxy`, and `contentWindow`, `defaultView`, `frames[i]`
  and `parent` all return it.
- Same-origin access goes through the fast path: Blink sets the same security token on same-origin contexts (derived from the
  origin, `SecurityOrigin::ToTokenForFastCheck`) and installs access-check callbacks on the `Window` global template for the
  cross-origin case. Scratchy only makes realms for same-origin frames, so one shared token is enough, with no access-check
  callback.
- An iframe gets its browsing context and an initial `about:blank` document (origin inherited from the parent) SYNCHRONOUSLY
  when it is inserted, so `contentWindow` is non-null right after `appendChild`. Removing the iframe detaches the frame
  (`WindowProxy::DisposeContext` / `DetachGlobal`): `contentWindow` becomes `null`, the window's timers are cancelled, and
  an old reference sees a closed window. Re-inserting the iframe makes a NEW browsing context.
- Microtasks: same-origin frames of one agent share a microtask queue. Timers belong to the window (its `ExecutionContext`)
  but run on the same event loop. A callback always runs in the realm of the function, because V8 enters the function's
  creation context.
- A DOM node has ONE wrapper per world. It is created in the realm of the node's document (its creation context), so
  `iframe.contentDocument.body instanceof w.HTMLElement` is true and `instanceof HTMLElement` (the parent's) is false.
- `data:` iframes have an OPAQUE origin in Chrome: `contentDocument` is `null` and `contentWindow` is a cross-origin proxy.
  `srcdoc` and `about:blank` inherit the parent's origin. **Scratchy differs:** QA18b gives `data:` iframes a same-origin
  document. QPT1 does NOT give them a realm (see decision D4).

### 4. Design
**D1. A realm is a second context in the page isolate, made from snapshot context 0.** New native
`__scratchy_realm_new()` (js_engine_v8.rs). It does `Context::from_snapshot(sc, 0)`, then
`c.set_security_token(page_token)`. The page token is a fixed value set on the page context in `Engine::new` (a string such
as the page origin, or the page's global). The native keeps a `v8::Global<Context>` in `State.realms: Vec<Option<..>>` (index =
realm id), enters the new context (`ContextScope`) to run `mark_htmldda`/`remove_global("__d_all")` there (each realm needs
its own `document.all`), and returns the realm's global proxy. No snapshot (SCRATCHY_V8_SNAPSHOT=0, SCRATCHY_V8_FLAGS, the
`without_snapshot` tests): it returns `undefined`, and `frameOf` keeps today's stand-in object (one log line). QPT1e then adds
the slow path. Cost: ~1.3 ms and ~0.65 MB V8 heap per realm (section 2). YouTube makes 2-3 iframes, so about 2 MB.

**D2. The realm is bound to its frame by a hidden hook baked into the preludes** (the QV8-4 rule: preludes read page
state only through hooks). The parent calls `w.__scratchy_realm_init(o)` with
`{ docId, env, parent, top, frameElement, name }`:
- `__scratchy_apply_env(env)` with the parent's env, but `href: 'about:blank'` (or the QA18b1 URL) and **`origin` = the
  parent's origin** (inherited, as in Chrome; `locSt.origin` is already a separate field). Viewport = the iframe's size
  (default 300x150).
- The DOM prelude gets `__d_bind_doc(id)`: `delete cache[doc.__id]; cache[id] = doc`, and `__id` redefined to `id`. To allow
  that, the baked `__id` becomes `configurable: true` until bound. It then rebuilds `<html><head><body>` when the document is
  empty, and sets `readyState` to `complete`. `inPage` compares with `doc.__id` instead of `0`.
- `parent`, `top`, `frameElement` (the parent-realm wrapper of the `<iframe>`), `name`, `opener: null` and `frames = self`
  become own data properties. The global's prototype stays the realm's own `Window.prototype`.
- Host forwarding (D5) is installed. Afterwards the hook deletes itself.

`frameOf(el)` then becomes: `d = __d_newdoc()`, optionally `__d_load_html(d, page)` (QA18b1),
`w = __scratchy_realm_new()`, `w.__scratchy_realm_init({...})`, and `frames[el.__id] = { doc: w.document, win: w }`. So
`contentDocument === contentWindow.document`, and `d.defaultView === w` is the realm document's own getter.

**D3. Which iframes get a realm:** connected iframes whose src is empty or `about:blank*`, and those with a pre-fetched
same-origin src (QA18b1; its scripts still do not run until QPT1f). Later also `srcdoc` (QPT1f). Network iframes stay `null`
(QA18b2). A realm is still made lazily on the first `contentWindow`/`contentDocument` read, so pages that never look pay
nothing. The only observable difference from Chrome's synchronous creation is `window.length`/`frames[i]`, which QPT1d
counts from the connected iframes themselves.

**D4. `data:` iframes keep today's QA18b stand-in (no realm).** Chrome gives them an opaque origin. That makes
`contentDocument` null, which is a behaviour change some pages may notice, so it needs the user's choice (question Q below).
BotGuard uses a src-less iframe (R52: `createElement('iframe')`, then `contentWindow.String`), so this does not block it.

**D5. One event loop. The realm's host queues forward to the parent's.** Rust only pumps the page context (timers, net,
ws, rejection flush). Instead of N pumps, `__scratchy_realm_init` points the realm's `setTimeout`/`setInterval`/
`clearTimeout`/`clearInterval`/`requestAnimationFrame`/`cancelAnimationFrame`/`requestIdleCallback`/`cancelIdleCallback`
at small wrappers over the PARENT's functions. The wrappers record the ids per realm, so that D6 can cancel them. They do
the same for `fetch`, `XMLHttpRequest`, `WebSocket` and `EventSource` (the parent's constructors, so `instanceof w.Response` is
false: a documented deviation). They also point the realm's `__scratchy_rej_push` at the parent's queue (or Rust's
`reject_callback` calls the page context's function, via a page-context `Global` kept in `State`). Callbacks still run in their
own realm, because V8 enters a function's creation context, which is also what Chrome does. Microtasks need nothing: the
queue is per isolate. Proper per-realm host queues (a `Realm` list in `JsEngine`, completions routed by realm id) stay a
backlog item (QPT1g). They are needed only if a page shows the forwarding to be wrong.

**D6. Lifetime.** A realm lives while its iframe is connected and keeps its src, and no longer. On removal (`afterRem`
hook next to `afterIns`), on a src/srcdoc change, and on navigation of the page, the parent:
1. cancels the realm's recorded timers/idle/rAF ids,
2. deletes `frames[el.__id]`,
3. calls `__scratchy_realm_drop(id)`, which drops the `Global<Context>`. Optionally the native also calls
   `Context::DetachGlobal` (not bound by the `v8` crate: an `extern "C"` on `_ZN2v87Context12DetachGlobalEv`, like
   `MarkAsUndetectable`), so an old `w` reference stops reaching the window, as in Chrome.

4. (QPT1b2) closes the WebSockets the realm created: a realm's `WebSocket` is its own wrapper class over the page's
   (so `new w.WebSocket()` is recorded per realm; the page's sockets are not); `__scratchy_realm_release` calls the page's
   `__ws_detach` on each: the connection is closed through wire (1001; a CONNECTING one is aborted) and the socket is
   forgotten, so no event (error/close) reaches the dropped realm. Tests: qf19w2_tests `qpt1b2_*`.

V8 frees the context once no JS reference is left (the spike freed 4 of 4). `contentWindow` is `null` while the iframe is
disconnected. Re-inserting the iframe makes a new realm. The engine drop frees everything. A soft cap of 64 live realms per
page logs once and makes the extra frames fall back to `null`, so a pathological page cannot use unbounded memory (Chrome
has no such cap, but it has process limits).

**D7. Cross-realm identity (QPT1c).** One wrapper per node across all realms of the page, created by the realm that owns
the node's document (Chrome's rule). The DOM prelude's `cache` becomes a page-wide object: the parent passes its own in
`realm_init`, and a realm table maps docId to that realm's `wrap`. `wrap(id)` returns the cached wrapper. If there is none, it
asks a native for the id's tree root (a document id, or the `owners` entry for detached trees) and delegates to that realm's
`wrap`. `instanceof` across realms then behaves as in Chrome. Built-ins are distinct per realm (`w.Array !== Array`) by
construction. `adoptNode`/`appendChild` of an iframe node into the page keeps its wrapper and its realm's prototype (Blink
keeps the wrapper too).
*Done in QPT1c* (NOTES.md "iframe realms"): as designed, plus a native `__d_root`, owners kept on removal/adoption, and a
purge of the dropped realm's wrappers (QPT1b's no-leak test stays green). MutationObserver registries were per realm until QPT1c2: the registry (`xr.mo`) is now page-wide, every realm's mutation hooks are
installed once any observer exists (`xr.hk`), records are built by the observer's realm (`st.rec`) and delivered in its microtask,
and a dropped realm's observers are disconnected on purge.

**D8. WindowProxy surface (QPT1d).** *Done in QPT1d* (NOTES.md "iframe realms"): as designed, plus natives `__scratchy_incumbent`/`__scratchy_enter`; `Context::DetachGlobal` was dropped so that a stale `w.closed` is true (a detached global throws `no access`). Deviation: `window[i]` of an iframe added through a subtree appears at the next sync.
- `window.frames === window`.
- `window.length` = the number of connected iframes in the document that have a window.
- `window[i]` and `window[name]` = their `contentWindow`. These are getters kept in sync on insert/remove: V8 has no global
  named/indexed interceptor here, because there is no global template.
- `w.postMessage(msg, targetOrigin)` is called on the TARGET realm's function. It clones with the target realm's
  `structuredClone` (so `e.data` is built from the target's intrinsics) and queues a task on the parent's timer queue. The
  task dispatches a `MessageEvent` on `w` with `source` = the calling realm's window (the incoming realm of the call, passed
  explicitly by a wrapper) and `origin` = the sender's origin.
- `w.closed` is true after D6. `w.name` reflects the iframe's `name` attribute. `srcdoc` is reflected on
  `HTMLIFrameElement.prototype`: BotGuard reads `getOwnPropertyDescriptor(iframe, 'srcdoc')`, R52 section 2.

**D9. Scripts inside the iframe (QPT1f).** Same-origin only: srcdoc, about:blank written by `document.open/write/close`, and
the pre-fetched QA18b1 document. The realm's own copy of `__scratchy_run_global` already compiles in the realm, because a
native runs in its function's creation context. The work is to:
- parse srcdoc into the realm's document,
- run its classic scripts in order through `w.__scratchy_run_global`, and external ones through the QY34 fetch path,
- fire the realm's DOMContentLoaded/load, then `load` on the `<iframe>`,
- support `document.open/write/close` on an iframe document, rebuilding the tree.

Network iframes stay QA18b2. Rendering iframe content is not part of QPT1.

### 5. Risks
- **Natives that assume the page context.** A native called from a realm runs with `ctx.global()` = that realm's global.
  The current natives use `ctx` only for values. `register` is only called at startup, and the Rust-to-JS calls go through
  `Engine::with`, which means the page. The QPT1a tests must cover `__scratchy_run_global` and `getComputedStyle` from a realm.
- **Page-only Rust checks.** `page_connected` (`t.get(0)`) ignores iframe documents for geometry bumps, which is right
  because they are not rendered. `__d_iframe_page` resolves against the page's base URL.
- **A missed host hook** (D5): a realm API that queues work with Rust and is not forwarded hangs silently. Grep every
  `call_global`/`eval_value` hook name in js.rs, js_net.rs, js_ws.rs, js_media.rs, and make the QPT1a test list exercise
  each forwarded API from a realm. Media elements in iframe documents are out of scope (media_el keys elements by page id).
- **Fingerprint.** The realm is the page's own snapshot, so it carries the same gaps as the page (`__*` globals,
  non-native `toString`: QPT2-QPT5). R52 showed that these do not stop the token today.
- **Snapshot coupling.** `__scratchy_realm_init` and `__d_bind_doc` must be baked. A prelude that reads page state at top
  level would bake the bake-time values into every realm. This is the QV8-4 rule again, now with two users of it.
- **Memory:** about 0.65 MB V8 heap per live realm, freed on drop. The D6 cap bounds the worst case.
- **The YouTube result is the real gate.** R52's stand-in got a token in 3 of 4 runs. The cause of the 1 failed run is not
  known.

### 6. Split (the coordinator files these; order = dependency order)
- **QPT1a [M] (P1) BotGuard slice: a real realm for src-less/about:blank (and pre-fetched same-origin) iframes.** D1, D2,
  D5, and the `frameOf` change. No snapshot: today's stand-in.
  - Files: js_engine_v8.rs (`__scratchy_realm_new`, `State.realms`, page token, htmldda per realm), js.rs (PRELUDE
    `__scratchy_realm_init` + the forwarding wrappers, register the native in `startup_engine`), js_dom.rs (DOM prelude
    `__d_bind_doc`, `inPage`, `frameOf`, `srcdoc` reflection).
  - Tests (js_dom_tests.rs): `typeof w.String`, `w.String !== String`, `w.String('x') === 'x'`, `[] instanceof w.Array`
    false, `w.Object.getOwnPropertyDescriptor` works, `typeof w.HTMLElement/Navigator/SubtleCrypto` function,
    `w.document === f.contentDocument`, `w.document.defaultView === w`, `w.document.body instanceof w.HTMLElement`, a node
    appended in the realm is NOT in the page, `w.parent === window`, `w.top === window`, `w.frameElement === f`,
    `w.location.href === 'about:blank'` with the parent's `origin`, `w.setTimeout` fires, `w.fetch` goes out through the
    page queue, an unhandled rejection in the realm is logged, `String(w)` does not throw.
  - The existing qa18b/qa18b1 tests keep passing unchanged.
  - LIVE gate (the R52 recipe, logged out, default UA): GenerateIT returns an integrity token in at least 2 of 3 runs, the
    87-byte PoToken appears, and SABR answers stay at status 1 past 64 s.
- **QPT1b [S] Realm lifetime.** D6: drop on removal, src change and page navigation. Cancel the realm's timers, a
  `__scratchy_realm_drop` native, `DetachGlobal`, the 64-realm cap, `contentWindow` null when disconnected, a new realm on
  re-insert.
  - Files: js_engine_v8.rs, js_dom.rs.
  - Tests: a timer set in a removed realm never fires. Create/remove 50 iframes, then a test-seam GC: the heap is back within
    1 MB of where it started (the spike: 0.65 MB per realm, all freed). `f.contentWindow === null` after `remove()`, and a new
    `w` after re-append.
- **QPT1c [M] Cross-realm wrapper identity.** D7: a page-wide wrapper cache plus delegation to the owner realm.
  - Files: js_dom.rs (DOM prelude `wrap`, `owners`), one native for the tree-root document id.
  - Tests: `f.contentDocument.body` read from the page `=== w.document.body`, and it is `instanceof w.HTMLElement` and not
    `instanceof HTMLElement`. Adopting/appending an iframe node into the page keeps its wrapper. `ownerDocument` across realms.
    A MutationObserver in the page sees the same wrapper as the realm.
- **QPT1d [S] WindowProxy surface.** D8: `frames`, `length`, `window[i]`, `window[name]`, cross-realm `postMessage`
  (`source`, `origin`, data from the target's intrinsics), `w.closed`, `w.name`.
  - Files: js_dom.rs, globals_prelude.js.
  - Tests: `window.length` counts 2 iframes, `window[0] === f.contentWindow`, `w.postMessage({a:1}, '*')` gives a
    `message` on `w` with `e.source === window` and `e.data instanceof w.Object`, and parent-to-child plus child-to-parent
    both work.
- **QPT1e [S] Realms without the startup snapshot.** Keep a native name -> arena index map in `State` always (today only
  while baking or attached). A realm made by `Context::new` gets new function objects for every native (same arena
  indexes), then runs the prelude list in that context. Split `startup_engine`'s prelude list into one function used by
  both.
  - Files: js_engine_v8.rs, js.rs.
  - Tests: the QPT1a tests again under `without_snapshot`. Measure the slow realm cost (expected ~30-50 ms, the QV8-4
    slow-start numbers).
- **QPT1f [M] Scripts in same-origin iframes.** D9: srcdoc parse and scripts, `document.open/write/close` on an iframe
  document, DOMContentLoaded/load in the realm, and then `load` on the element.
  - Files: js_dom.rs, js.rs.
  - Tests: a srcdoc script sees its own `document`/`window` and `parent.x`. A `document.write('<script>...')` into an
    about:blank iframe runs in the realm. The event order is: DCL in the realm, then load in the realm, then `load` on the
    `<iframe>`.
- **QPT1g [M] backlog, only if needed: per-realm host queues** (replaces D5's forwarding). `JsEngine` keeps a realm list,
  the timer pump asks each realm, and net/ws completions and rejection flushes are routed by realm id.

**Question for the user (D4, not blocking QPT1a):** Chrome gives `data:` iframes an opaque origin, so their
`contentDocument` is `null` and the parent cannot reach into them. Scratchy (QA18b) gives them a readable same-origin
document. Keep scratchy's behaviour, or match Chrome (then they get no realm and `contentDocument` becomes `null`)?

**QPT1-D1 (2026-11-20, user answered D4: match Chrome) DONE.** `data:` iframes now have an OPAQUE origin. Their document is
decoded (percent or base64; text/html only, other types load empty) and loaded into its own realm like a srcdoc, so its
scripts run, `location.origin` is `null`, `frameElement` is null, and `parent`/`top` are cross-origin views. The parent gets
`contentDocument === null` and `contentWindow` = a cross-origin WindowProxy (`xoOf` in js_dom.rs, a JS `Proxy`, one per window):
only postMessage, close, closed, focus, blur, frames, length, location (href set only), opener, parent, self, top, window are
readable (`parent`/`top` of an opaque frame give the real parent chain; the view of anything else is cross-origin); every other
get/set/define/delete throws `SecurityError`. postMessage across the boundary gives the receiver a cross-origin view as
`event.source`. Genuinely cross-origin (network) iframes had no path to reuse: they still give `null` (QA18b2), so this proxy is
new and is what a future real cross-origin frame should use. Limits: the opaque realm shares the page's native tables, so
isolation is by API surface (a script guessing raw node ids could still reach the page's natives, as for any realm); the
realm is not given its own storage/cookie partition. Existing tests updated: `js_dom_tests.rs` the QA18b iframe test (`dd.contentDocument.nodeType`
became `dd.contentDocument` = null) and the QPT1a nested-realm test (`typeof dd.contentWindow.String` became
`typeof dd.contentWindow.postMessage`, `contentDocument` null). New: `qpt1_d1_*` tests.

## R52: QM1k2: why scratchy gets no PoToken. BotGuard makes an `<iframe>` and reads `iframe.contentWindow.String` to get clean built-ins from a fresh realm. scratchy's iframe window (QA18b) is a null-prototype object with ten own properties, so `String` is `undefined`. BotGuard then skips its whole fingerprint pass and sends an early-exit answer (~2.9 KB). `GenerateIT` answers `[null,43200,null,"<fallback>"]`: NO integrity token, so no PoToken is minted, SABR stays at status 2 and then goes to 3 (R45). A throwaway stand-in that only gives that object the page's globals turns the answer into a real integrity token. YouTube then mints an 87-byte PoToken, and every SABR answer is status **1** (OK) for the whole 119 s run, past the old ~64 s stop (Opus investigator QM1k2, 2026-11-19, research only, docs only) — OPEN: proposed QPT1 (the fix, L) and QPT2-QPT8 (fingerprint gaps, not blocking today)

**Status: research only, docs only. No product change.** A temporary 6-line loader in `js.rs` (`QPROBE_JS=<file>` evaluated
after the preludes) and throwaway probe scripts in the scratchpad were used. The loader is reverted (`git checkout`). Release build
of main 6b3520e (V8 + media) in the item's worktree. Own wire on a scratchpad socket (fresh in-memory cookie jar per run, so
LOGGED OUT). DEFAULT user agent (`Mozilla/5.0 (X11; Linux x86_64) scratchy/0.1.0`), no disguise. Nested dwl window 960x1006,
`SCRATCHY_MEDIA_AO=null`, `SCRATCHY_V8_SNAPSHOT=0`, `--budget-ms 120000` (the probes are slow). URL
`https://www.youtube.com/watch?v=G1AeBgalFZk`. Comparison: the local Google Chrome 156 (fresh profile, a window in the same
nested dwl, driven over the DevTools protocol only to read the network log and the console; the same probe script injected
with `Page.addScriptToEvaluateOnNewDocument`). Machine load average 12-23 (other agents building), so every time below is
rough and is not a performance claim. Nothing was forged, replayed, borrowed or fetched: every token below was produced by
YouTube's own scripts running in the browser under test. The stand-in experiment (section 3) is diagnostic only and is NOT a fix.

### 1. The flow, as it runs today (both browsers)
- The player loads `//www.google.com/js/th/<hash>.js` with a `<script>` element (the BotGuard interpreter; no
  `/api/jnn/v1/Create` request, the challenge is already in the page). The interpreter `eval`s a 63 KB control-flow-flattened
  VM, which runs the challenge program. Then the player POSTs `/api/jnn/v1/GenerateIT` (XHR) with
  `["O43z0dpjhgX20SCx4KAo","$<botguard response>"]`.
- **Chrome:** 0.04-0.4 s from the th script to GenerateIT. The request body is ~2 030 B, and the answer is
  `["<88-char integrity token>",43200,100]` (5 of 5 runs, including 2 with the probe injected).
- **scratchy (main):** the request body is ~2 920 B, and the answer is **`[null,43200,null,"Mk…<108 chars>"]`** (11 of 11 runs):
  integrity token `null`, ttl 43200, and a 4th field = websafe fallback token. That is the "no attestation" answer.
- SABR (`videoplayback?…sabr=1`, protobuf body): `streamerContext` (field 19) field 2 is the PoToken slot. Before a token
  exists it holds a 10-byte placeholder `{4: <8 bytes>}` in BOTH browsers, and the answer carries UMP part 58 `08 02`
  (status 2, pending). After a valid integrity token, YouTube's minter fills it with an 87-byte token `{6: <85 bytes>}`
  (seen only in section 3). With only the placeholder, scratchy gets ~64 s of media and then status 3 (R45).

### 2. Which probes run, and where scratchy diverges
The probe logged every read of a window global, of `navigator`/`screen`/`performance`/`history`, and of selected `document`
and prototype members, plus calls to `getOwnPropertyDescriptor`/`getOwnPropertyNames`/`addEventListener`/`instanceof`. It
logged these only when the calling stack contains a `/js/th/` frame (BotGuard's VM runs in `eval` code whose stack names the th
script). Same probe in both browsers:

| step | Chrome | scratchy |
|---|---|---|
| setup reads: Date, eval, Array, Math, Object, Symbol, String, Error, Number, requestIdleCallback, Reflect, performance.now/timeOrigin, readyState, atob, hidden, body, createElement | same | same (no `trustedTypes`, so the VM also reads `window.Proxy`) |
| `createElement('iframe')`, `instanceof DocumentFragment` (false) / `Element` (true) | yes | yes |
| `iframe.contentWindow.String` | `function` (a fresh realm), 0.08 s later | **`undefined`** (5-24 s later, when the busy main thread gets to it), then `.replace` -> `undefined` |
| later: iframe `Object`, ~15 interface names (`WebKitCSSMatrix`, `SubtleCrypto`, `Navigator`, `HTMLElement`, `SVGRectElement`, …), `getOwnPropertyDescriptor` through the iframe's `Object` | yes | **never reached** |
| fingerprint: listeners on `<body>` (focusin/out, key*, input, copy/cut/paste, click, pointer*, mouse*, drag*, touch*, wheel), `navigator.webdriver/platform/connection/hardwareConcurrency/maxTouchPoints/keyboard/doNotTrack/plugins/userAgent/language(s)/deviceMemory/gpu/userActivation/mediaDevices`, `screen.*`, `devicePixelRatio`, `history.length`, `menubar/toolbar/locationbar/statusbar`, `performance.memory/getEntriesByType`, `chrome`, `Permissions`, `isSecureContext`, `opener`, `matchMedia`, `HTMLCanvasElement.prototype.getContext`, `getOwnPropertyDescriptor(navigator,'plugins'/'mimeTypes')` then the getter called on the prototype (expects `Illegal invocation`), `getOwnPropertyDescriptor(iframe,'srcdoc'/'contentWindow')`, `Function.prototype.toString` on the `HTMLAnchorElement` `hrefTranslate` getter (expects `function get hrefTranslate() { [native code] }`) and on `window` (expects TypeError), `outer*/inner*`, `hasFocus`, `getElementsByTagName` | yes | **never reached**: right after the `undefined` it reads `Promise`, `btoa` and sends |
| GenerateIT | token | `null` |

Gaps observed directly in scratchy (a local env page, scratchy vs Chrome; `env.html` in the scratchpad):
- **iframe window** (`js_dom.rs` `frameOf`): `Object.create(null)` with `document, window, self, parent, top, frameElement, name,
  length, location, getComputedStyle`. It has no built-ins and no interfaces, and `String(iframe.contentWindow)` THROWS
  `TypeError: Cannot convert object to primitive value` (Chrome: `[object Window]`).
- `navigator`: own data properties on the instance (Chrome: accessors on `Navigator.prototype`, `Object.keys(navigator)` is
  empty). `webdriver`/`plugins`/`mimeTypes`/`pdfViewerEnabled`/`deviceMemory`/`connection`/`permissions`/`mediaDevices`/
  `storage` are undefined. `cookieEnabled` is `false` although wire's jar works. `hardwareConcurrency` is 1 (this machine has 16).
- `Object.prototype.toString` gives `[object Object]` for `navigator`, `document`, `window` and `performance` (Chrome:
  `Navigator`, `HTMLDocument`, `Window`, `Performance`). `performance` is a plain object, not a `Performance`, and has no
  navigation entry.
- `Function.prototype.toString` of every prelude-made API (`fetch`, `setTimeout`, `addEventListener`, `createElement`,
  `performance.now`, `crypto.subtle.digest`, …) prints JS source (Chrome: `[native code]`). `createElement.name` is `''`.
  About 65 internal names (`__scratchy_*`, `__d_*`, `__mse_*`, `__net_*`, `__hist_*`) are visible to
  `Object.getOwnPropertyNames(window)`.
- `Intl.DateTimeFormat().resolvedOptions().timeZone` is `UTC` while `new Date().getTimezoneOffset()` is `-60`
  (Europe/Berlin). That is an inconsistent clock.
- `performance.now()` steps by ~1 µs (Chrome 100 µs). Canvas `getContext` is `null` and `toDataURL()` is `"data:,"`.
  `screen` equals the viewport. `menubar`/`toolbar`/… are undefined. `Event.isTrusted` is a writable own data property.
- Stack frames of inline scripts read `inline script #N:line:col` (Chrome: the document URL).

### 3. The deciding experiment (throwaway stand-in, diagnostic only)
The probe replaced the `contentWindow` getter. On first access it defined getters on the iframe window object for every name
the main window has (443 names), returning the MAIN window's values. Same realm, so this is NOT what a browser does.

| runs | iframe window | GenerateIT answer | SABR |
|---|---|---|---|
| light probe, no stand-in (2) | main | `null` token, body 2 919-2 920 B | (not played) |
| light probe + stand-in (3) | filled | **token** in 2 of 3 (body 1 687-1 692 B); 1 run `null` (body 3 031 B; why not known) | |
| deep probe + stand-in (1) | filled | **token**. The VM now runs the whole fingerprint pass of section 2 against scratchy's own values (`webdriver` undefined, `plugins` undefined, `hardwareConcurrency` 1, UA `scratchy/0.1.0`, no canvas, …) | |
| light probe + stand-in, clicked play (run 22) | filled | **token** at +10.4 s | 87-byte PoToken in `streamerContext`; **40 SABR answers, all `08 01` (status 1, OK), media 0.77-3.6 MB each, through +119 s**; no status 3 |

So the ONE thing between scratchy and a PoToken today is the iframe realm. The other gaps are reported to Google inside the
fingerprint, but they did not stop the token in this test. They may matter later, or for other Google properties.

### Fix list (sized; the coordinator files the items; honest only, no UA or identity changes)
- **QPT1 [L] (P1 for YouTube past 60 s) A real window realm for same-origin `about:blank`/`srcdoc`/`data:` iframes (V8 build).**
  `frameOf` makes a new `v8::Context` in the page's isolate (same security token), from the startup snapshot when available,
  and runs the same preludes. Its `document` is the iframe's existing second document (QA18b), and `parent`/`top`/`frameElement`
  point at the page. `contentWindow`/`defaultView` return that context's global, so `String(w)` is `[object Window]`, and
  `w.String !== window.String`, `w.Object`, `w.Navigator` and `w.HTMLElement` are its own. Prelude natives are installed per
  context (`install_dom`/`install_net` take a context + document). The iframe dies with the element/navigation. Files:
  js_engine_v8.rs (multi-context `Ctx`, `with_context`), js.rs (`start_scripts` split: per-isolate vs per-context), js_dom.rs
  (`frameOf`, per-document install), js_net.rs (shared executor), tests in js_dom_tests.rs: an iframe has its own `Array`
  (`[] instanceof w.Array` false), `w.String('x')`, `Object.prototype.toString.call(w)`, `w.document === iframe.contentDocument`,
  `w.parent === window`, and the window is freed after `iframe.remove()` plus a GC. QuickJS keeps today's object (or JS_NewContext;
  it goes away at QV8-6). Expected effect: GenerateIT returns an integrity token, YouTube mints a PoToken, SABR status 1, and
  playback goes past ~64 s (section 3). Risk: memory per iframe (one context each; YouTube makes 2-3), and the scripts
  of a network iframe still do not run (QA18b2).
- **QPT2 [S] `navigator` the WebIDL way (truthful values).** Accessors on `Navigator.prototype` that throw `TypeError: Illegal
  invocation` on a wrong receiver, `Symbol.toStringTag` `Navigator`, `webdriver: false` (scratchy is not automation-driven),
  `cookieEnabled: true` when wire's jar is on, `hardwareConcurrency` = `std::thread::available_parallelism()`, and empty
  `PluginArray`/`MimeTypeArray` with `pdfViewerEnabled: false` (scratchy has no PDF viewer). Files: js.rs PRELUDE (+ apply_env).
  Effect: BotGuard's plugins/mimeTypes descriptor probes see a real browser shape. Not needed for the token today.
- **QPT3 [S] `Symbol.toStringTag` and prototypes for window, document, performance, screen, history, location** (`Window`,
  `HTMLDocument`, `Performance`, …). `performance` becomes a `Performance` with a `PerformanceNavigationTiming` entry.
  Files: js.rs, cssom/globals preludes. Effect: `Object.prototype.toString` probes match a browser.
- **QPT4 [M] Built-ins print as native.** `Function.prototype.toString` returns `function <name>() { [native code] }` for
  functions that scratchy's preludes define (a registry filled at prelude time, in a native `toString` override; Firefox does the
  same for its self-hosted JS built-ins). Every API function also gets its WebIDL `name`/`length`. Files: js.rs, js_engine_v8.rs
  (one native), all `*_prelude.js` (registration). Effect: BotGuard's `hrefTranslate`-getter style checks pass. Honest: these ARE
  scratchy's built-ins.
- **QPT5 [M] Hide scratchy's internal globals** (~65 `__*` names): keep them in a prelude closure, or behind one non-enumerable
  private-symbol-keyed holder, so that `Object.getOwnPropertyNames(window)` lists only web APIs. Files: every prelude plus the
  Rust callers that read them by name.
- **QPT6 [S] One clock identity.** Intl's default time zone = the system zone (the one `Date` already uses; V8/ICU default tz from
  `TZ`/`/etc/localtime`), not UTC. Files: js_engine_v8.rs init (ICU default zone) or intl_prelude.js. Effect: removes an
  inconsistency that bot checks look for.
- **QPT7 [S] Coarsen `performance.now()` to 100 µs** (Chrome's cross-origin-isolation-off value; it is also a timing-attack
  mitigation). File: js.rs `perf.now`. Check that QAN1/animation and perf tests do not depend on µs steps.
- **QPT8 [L, existing plans] Canvas 2D, network-iframe scripts (QA18b2), `trustedTypes`, Permissions/NetworkInformation/
  mediaDevices.** These are probed in the fingerprint but are features, not bot-check fixes. Track them under their own items,
  not for the token.
- Not a new item: BotGuard's steps are slow in scratchy (th script -> GenerateIT 1-25 s instead of 0.04-0.4 s), because its
  `setTimeout`/`requestIdleCallback` steps queue behind page work. That is the existing main-thread/perf work (QY53*, QM1k1n5).
  The token still arrived in time (+10 s in run 22, while 64 s of unattested media had been allowed).

### Not established
Whether the stand-in run that still got `null` (1 of 3) was a different challenge or a timing effect. Whether Google will later
require the iframe built-ins to come from a separate realm (with the stand-in they are the main realm's, and the token was still
issued). Whether the PoToken holds for a whole long video, and for seeks and quality switches (run 22 played 119 s). What the
other fingerprint values (no `webdriver`, no plugins, no canvas) do to the token's score over time. No Chrome comparison was made
logged in, and none with Firefox.

## R51: QYSK1: a click on YouTube's progress bar "pauses, then continues from where it was". The click, YouTube's seek and scratchy's MSE seek all WORK; what fails is what happens while an out-of-buffer seek waits for data: MediaCore leaves mpv's OLD generation loaded and unpaused, so YouTube's `play()` after its scrub-pause resumes the old position's picture and sound while the element reports `seeking` at the target. The target's data often never comes (SABR attestation status 3 past ~64 s media time, QM1k2) or comes late (QM1k1n5). Plus a smaller bug: after an in-generation seek mpv sometimes delivers a stale `time-pos` right after `playback-restart`, so `seeking`/`seeked` report the pre-seek time (Opus investigator, 2026-11-19, docs only) — OPEN: proposed QYSK1a (P1, hold the old generation while an MSE seek waits) and QYSK1b (P2, drop the stale time-pos), section 4

**Status: investigation only, docs only; the instrumentation was reverted.** Main d7984e5 (QCUR1 6b3520e
included), own worktree release build (V8 + media, libmpv 0.41 / ffmpeg 8.1.3), own wire on a scratchpad
socket (never the user's), `SCRATCHY_MEDIA_AO=null`, `SCRATCHY_V8_SNAPSHOT=0`, default UA, URL
https://www.youtube.com/watch?v=G1AeBgalFZk (logged out, DE). Headless: `scratchy --perf-report --budget-ms
20000 --seconds 45..90`. Window: tools/nest.sh (nested dwl, 960x316) + tools/inject + grim. No timing claims
(no flock); `uptime` at the end: load 23 (other agents building), which slows but does not change results.

**Throwaway instrumentation (reverted, never committed):** (1) appended to `mse_prelude.js`, logging via
`__scratchy_err` so it also shows headless: wrapped `currentTime` setter (value, old value, buffered, 5-frame
stack), `play()`/`pause()` with stacks, media events (currentTime/readyState/seeking/buffered), `mousedown`/
`mouseup`/`click`/`pointer*` on `document` (capture, target, clientX/Y), `addEventListener` registrations on
`.ytp-progress*`, `appendBuffer` (size, top-level boxes, each moof's `tfdt` in seconds via the init's `mdhd`
timescale) + `buffered` after each append, `remove`/`abort`/`timestampOffset`, googlevideo `fetch`es with UMP
part counts and the part-58 payload, a 2 s TICK with `movie_player.getPlayerState()/getCurrentTime()`, and
for headless a scripted seek after `playing` (`&sk=<t>_<delay>[_ct|_click]`): `movie_player.seekTo(t)`,
`video.currentTime = t`, or a synthetic `mousedown`/`mouseup`/`click` on `.ytp-progress-bar` at the clicked x
with exactly the fields `App::js_mouse` sets. (2) Rust `eprintln!` in `MediaCore::set_current_time`,
`mse_seek` (in_generation, covers, live generations, run ends, per-track buffered), `mse_resolve` (backend
seek vs new generation), `handle` (every backend event; `Time` only while seeking or on a jump > 1.5 s), and
`MpvBackend::load_mse`/`seek`. (3) Local fixtures (scratchpad only): a 60 s testsrc2 h264 + 440 Hz AAC pair,
fragmented (`frag_keyframe+empty_moov+default_base_moof`, 2 s fragments); a page that appends 0-15 s, plays
and, on an out-of-buffer `seeking`, appends from the target on like YouTube (`?seek=T&at=S`; `noappend` =
the target's data never comes, i.e. a SABR refusal; it mimics YouTube's click: `pause()`, 700 ms,
`currentTime = T; play()`); a plain `<video controls src=plain.mp4>` page.

### 1. The click reaches YouTube and YouTube seeks (no input or hit-test bug)
- `.ytp-progress-bar` registers `mousedown`, `mouseover`, `touchstart`, `focus`, `blur` (no `pointerdown`);
  scratchy dispatches `mousedown`/`mouseup`/`click`, so YouTube's listener is reached.
- **Real click, nested window**, playing at ~17 s, click at x = 45 s: `mousedown` target
  `DIV.ytp-timed-markers-container` -> YouTube `pauseVideo` (stack `g.k.Zx < g.k.seekTo`: YouTube pauses while
  scrubbing = the user's "pauses briefly") -> `mouseup` -> `currentTime = 44.994` (`setCurrentTime < seekTo`)
  -> `play()`. Inside the live generation: `mse_resolve -> backend.seek`, `playback-restart` 0.65 s later,
  `seeked`, TICKs 45.1, 47.0. **Worked.**
- Headless synthetic click at 45 s (ct 10.8, buffered [0-32]): same calls; outside the buffer, so a NEW
  generation loaded when the SABR answer came (1.25 s, part 58 `08 02`, appends 42.67-64.00), `seeked` 45.04,
  `playing`. **Worked.**
- Headless `seekTo`: 30 (ct 8.3) and 55 (ct 1.3) reload at the target and play on (30.6 -> 40.5; 55.05 ->
  64.9). 150 in one run: SABR status `08 01` (OK) that time, appends 149.33-170.67, `seeked` 150.00. **Worked.**
- Plain `<video controls>` (scratchy's QM1g2d bar, window): click on the seek track at 75 % -> mpv seek ->
  `seeking`/`seeked` ct 45.03 -> 45.93, 46.93. **Works** (the release re-applies the drag position, so one
  click fires `seeking`/`seeked` twice: harmless, a QM1g2d1 note). So this is not a general seek bug.
- Local MSE fixture headless: out-of-buffer 5.9 -> 40 (data appended on `seeking`) reloads at 40 and plays
  40.84 ... 52.86; in-generation 2.4 -> 12 and 7.8 -> 1 are mpv seeks and play on.

### 2. Root cause (P1): while an MSE seek waits for data, mpv keeps playing the OLD generation
`MediaCore::mse_seek` (src/media_el.rs) sets `seeking`, `current_time = t`, readyState HAVE_METADATA, fires
`seeking`/`waiting`, stores `want = t`; when `t` is neither in the live generation nor covered,
`mse_resolve` returns to wait for appends. **Nothing tells the backend.** mpv still has the old generation
(its run goes to the old buffered end):
- while playing it just keeps playing;
- YouTube pauses on `mousedown` and calls `play()` after the seek: `start_playing` ->
  `backend.set_paused(false)` RESUMES THE OLD GENERATION at the old position.
`BackendEvent::Time` is ignored while `seeking`, so JS and YouTube's UI show the target while the window
shows and plays the old position: the user's report.

Evidence:
- Local fixture `noappend` with YouTube's pause/seek/play sequence: TICKs say `ct=40.00 rs=1 seeking=true`
  for 11 s while mpv's `time-pos` runs **4.02 -> 15.12** (old generation, to its end at 16).
- Headless YouTube run 1, `seekTo(150)` at ct 10.48 (buffered [0-42.67]): no media after the seek (10 small
  SABR answers ~2 s apart, then YouTube's `stopVideo` -> `abort`/`emptied` = the R45 stop); for those 20 s
  mpv played **10.68 -> 32.15** while JS said `seeking` at 150.
- Window, real click at 150 s at ct 61.2: SABR `08 03 10 0a` (status 3), no media; after YouTube's `play()`
  mpv played the old generation 61.2 -> 63.4 until the old run ran dry at 64 (`Buffering(true)`). Earlier in
  the video the old run has 20-50 s left, which is what the user saw.
- When the target's data DOES come, the old generation still fills the gap until the new one loads (1.2-5.7 s
  headless; in a live window fetch results reach JS 1-19 s late, QM1k1n5), so even a successful seek first
  "continues from where it was".

Why the data often never comes is not scratchy's: SABR serves unattested media only up to ~64 s of MEDIA
time (status 2), then status 3 (QM1k2, R45, PARKED): seeks to 30 or 55 got data up to 64.00 then status 3;
150 got none (run 1, window) or status 1 (one run, played fine). ~73 % of this 3:59 video's bar lies past 64 s.

### 3. Second bug (P2): a stale `time-pos` after `playback-restart` (in-generation seeks)
Local fixture, in-generation seek 2.38 -> 12, 6 runs: in 2 mpv sent `PLAYBACK_RESTART` and then, in the
same poll batch, `time-pos` **2.000**. `handle(Restart)` already cleared `seeking`, so `Time(2.0)` sets
`current_time = 2.0` and the page's `seeking` and `seeked` handlers both read `currentTime 2.00` (target
12); the next `time-pos` (12.04) corrects it ~60 ms later. A page that checks `currentTime` in `seeked`
sees the seek land at the old position. Not the cause of this report (the window click landed cleanly).
Side note: `seeking` is queued (`Out::Fire`) and can be dispatched after `playback-restart` already cleared
`seeking`, so its handler reads `video.seeking == false`; the spec queues it as a task too, noted only.

### 4. Proposed fixes (not implemented, per the item)
- **QYSK1a [S/M, P1] Hold the old generation while an MSE seek waits.** In `mse_seek`, when the target is
  not playable now (`issued && !in_generation(t)`), set `hold` on `MseAttach` and call
  `backend.set_paused(true)` (last frame stays on screen, like Chrome's frozen frame; no sound). While
  `hold`: `start_playing`/`pause`/autoplay change `self.paused` but do not call `backend.set_paused`;
  `handle` ignores the old generation's `Time`, `Eof` and `Buffering`. `mse_resolve` clears `hold` on both
  paths: in-generation (`backend.seek(t)` then `set_paused(self.paused)`, for when a contiguous append
  extends the run over `t`) and new generation (already calls `set_paused(self.paused)` after `load_mse`);
  a new load / detach clears it. Alternative: `unload` the old generation at once and set `issued = false`
  (simpler, but the picture disappears during the wait; not recommended). ~30-40 lines in media_el.rs; no
  mpv, wire or JS change.
  **Tests:** (1) FakeBackend, beside `qm1k1c_seek_outside_the_buffer_waits_then_reloads_at_t`
  (src/tests/mse_tests/qm1k1c.rs): play, `set_current_time(20)` outside the buffer -> last backend call
  `paused true`; `pause()` + `play()` (YouTube's sequence) -> no `paused false`; queued `Time(5.0)`, `Eof`,
  `Buffering(true)` leave `current_time` 20, `seeking` and readyState unchanged and fire no `waiting`;
  appending both tracks at 20 -> `load_mse .. 20` then `paused false`. (2) Real mpv in
  src/tests/mse_mpv_tests.rs (skips without libmpv/ffmpeg like the qm1k1b tests),
  `qysk1a_out_of_buffer_seek_holds_old_generation_then_resumes_at_target`: MediaCore + MpvBackend over the
  2-track fixture, append 0-16 s, play to ~3 s, seek to 40 with nothing appended for 2 s -> no `Time` >
  old + 0.5 reaches the core; then append 38-56 -> `Time` >= 40.5 within 3 s. (Before the fix the same
  scenario gives 4.02 -> 15.12, section 2.)
- **QYSK1b [S, P2] Drop the stale `time-pos` after a seek.** `handle(Restart)` keeps the target (e.g.
  `settle: Option<(f64, Instant)>`); while set, `Time(p)` with `|p - target| > 1.0` is ignored, for at most
  500 ms or until a `Time` near the target arrives. ~10 lines. **Test:** FakeBackend in-generation seek to
  12 with queue `[Restart, Time(2.0), Time(12.04)]` -> `seeked` dispatched with `current_time == 12.0`, and
  `current_time` never 2.0 (the real-mpv repro is racy, 2 of 6, so the fake test is the gate).
- Not scratchy's: the SABR refusal past ~64 s (QM1k2, user decision). With QYSK1a a refused seek shows a
  frozen frame + YouTube's spinner, then YouTube's own error card after its 10 retries, instead of the wrong
  part of the video playing. QM1k1n5 (late fetch results) sets how long a successful out-of-buffer seek waits.

## R50: QYTC1: why YouTube's visible player controls cost ~10 forced layouts/s. About 2/3 of them (and of their ms) come after nothing but YouTube's progress-bar `style` writes (`transform: translateX(..)` on the scrubber, `transform: scalex(..)` on the play progress, `left: Npx` on the absolutely positioned hover progress), ~5 times a second, each followed by a geometry read (`uib`, then `getPlayerSize`). Each read is a whole-document layout (436 boxes, 27-58 ms here). 12-13 % change no box at all; ~20 % follow `class` toggles (`ytp-autohide*`, suggested-action badges) (Opus, 2026-11-19, measure only) — FIXED: QYTC1a (result at the end of the section: ~1 forced layout/s); OPEN: QYTC1b, QYTC1c

**Status: measurement only. No product change; the trace counters were reverted.** Branch from main
d7984e5 (includes QCUR1 6b3520e), release build (V8 + media), own worktree, own wire on a scratchpad
socket, nested dwl (tools/nest.sh) + tools/inject, `SCRATCHY_MEDIA_AO=null`, `SCRATCHY_FRAME_STATS`.
URL https://www.youtube.com/watch?v=G1AeBgalFZk. Load average 10-28 during the runs (other agents were
building), so absolute ms are high; the ratios are what matter. Every measured run under the perf flock.

### Recipe
R46 recipe, one phase: after 16 s `inject 960 1006 move:346,261 click wait:10000` (play), then a
60 Hz circle INSIDE the picture for ~10 s (`move:x,y wait:16` x 600, centre 346,200, radius 60x40:
keeps the controls visible), then park at (10,990) for 8 s (controls hide). Numbers below are the
seconds with >= 40 pointer moves (9 per run). Throwaway trace (env var, never committed): per forced
layout in `GeomCache::refresh` the V8 caller frame, CPU ms, whether any rect changed and which
elements' rects changed; per `geom_bump` the write that caused it (attribute, element, old -> new).

### Main today (two untraced runs, then the two traced runs)
| run | forced/s (median) | ms per layout | forced ms/s | video painted/s | lat_p95 ms | js_ms/s |
|---|---|---|---|---|---|---|
| base1 | 10.5 | 40.5 | 461 | 22.1 | 95 | 657 |
| base2 | 11.1 | 42.3 | 480 | 22.8 | 82 | 647 |
| trace1 | 10.0 | 43.3 | 563 | 20.3 | 79 | 733 |
| trace2 | 8.7 | 58.0 | 526 | 11.9 | 131 | 711 |

Idle seconds after the controls hide: 0 forced layouts, video 30/30, js ~25-30 ms/s. So the whole
cost is the controls' update loop, as the item says; the pointer only keeps the controls shown.

### Who reads (callers; trace1 / trace2, controls phase)
| caller | layouts | CPU ms |
|---|---|---|
| `uib` player base.js:7776:162 | 51 / 41 | 2060 / 2343 |
| `g.k.getPlayerSize` player base.js:15507:316 | 51 / 36 | 2002 / 2056 |
| scratchy's IntersectionObserver recheck (`io`) | 13 / 11 | 539 / 636 |

Headless `--perf-report --seconds 15` of the same URL (load only, no controls): 76 forced layouts,
2 254 ms; `getPlayerSize` 46 (1 141 ms), `uib` 10; 34 of 76 changed no rect.

### What dirties the geometry between two reads
The progress-bar update runs ~5x/s and is always the same pair:
1. `div.ytp-scrubber-container` `style="transform: translateX(42.19px)"` -> `translateX(42.45px)` and
   `div.ytp-play-progress` `style="left: 0px; transform: scalex(0.0685); background-size: ..."` ->
   `scalex(0.0689)`; then `uib` reads -> layout. Rects that changed: the scrubber container and its 7
   descendants only (scratchy applies a translate as an offset of the box subtree, QY18; `scalex` is
   ignored by layout).
2. `div.ytp-hover-progress` `style="left: 42.19px; transform: scalex(0)"` -> `left: 42.45px; ...`; then
   `getPlayerSize` reads -> layout. Rects that changed: the hover progress and the ink extents of its 9
   ancestors (`#player-wrap` .. `.ytp-progress-list`); no other box moved.

Classified by the writes between two layouts (controls phase):
| what came before the layout | trace1 | trace2 |
|---|---|---|
| ONLY progress-bar `style` writes (scrubber / play / hover / load progress) | 81 layouts, 3 183 ms (70 %) | 57, 3 260 ms (65 %) |
| something else, and no rect changed (no-op) | 13, 509 ms (11 %) | 11, 625 ms (12 %) |
| something else (`class` toggles), rects changed | 21, 909 ms (19 %) | 20, 1 151 ms (23 %) |

The "something else" writes: `#movie_player` class `ytp-autohide-active` added AND removed again
before the next read (11+11 in trace2: a net no-op), `ytp-autohide`, `playing-mode`,
`buffering-mode`, the suggested-action badges' `ytp-suggested-action-badge-with-controls` /
`-expanded` and their `style="display: none" -> "display: none; width: 40px"`, plus a few
`removeChild`/`insertBefore`/`removeAttribute` inside the player.

### What one layout costs
Each forced layout is a whole-document build + measure on the JS thread (`GeomCache::refresh`), 436
element boxes. Headless phases over the 76 load-time layouts: build_children 43 %, precompute_generated
33 % (the ::before/::after match pass over every element), css_collect + rule_index 18 %,
measure_document 5 %, walks < 1 %. So a real browser's "~1 ms" is not reachable by making the full
layout faster; the reads must stop needing one.

### Proposed fix (not implemented: needs QY53o1's code, see the question)
- **QYTC1a [M] (P1)** Contained inline-style writes. A `style` attribute write whose parsed declarations
  (`css::parse_declarations_split`, normal and `!important` parts) differ from the old ones ONLY in
  (i) paint-only fields (`opacity`, `background_color/_position/_size`, ...), (ii) `transform` (only a
  translate moves boxes: shift the element's laid subtree by the px delta, `frac * w` for `%`), or
  (iii) `left`/`top` in px on a box that was `absolute`/`fixed` with a definite `width` (`right` auto)
  / `top` with `bottom` auto: shift its laid subtree by the delta. Conditions: no selector reads the
  `style` attribute and no `attr(style)`; no rule declares `!important` `transform`/`left`/`top`
  (checked once per rule list); the element has one box; nothing laid elsewhere belongs to its DOM
  subtree (a `fixed` descendant). Then update the cached rects in place instead of bumping the
  geometry generation: `GeomCache` keeps a flat pre-order skeleton of the laid tree (node, rect,
  extent, subtree end) so the subtree shift and the ancestors' ink extents are exact; glyph rects of
  the subtree are marked stale (a `text_rects` read lays out). Anything else falls back to the full
  layout. Expected on this recipe: the 65-70 % "progress-only" layouts disappear (forced ~10/s ->
  ~3-4/s). The hook must sit where `__d_setattr` decides to bump (`geom_bump_attr`): it needs the
  attribute name and the old value, and must replace the bump, not follow it. A DOM-snapshot diff
  inside js_geom.rs alone is not sound (form state, `sheet_text`, adopted sheets bump without a DOM
  attribute change). Tests: fixture with an abs `left` write and a translate write -> 0 extra forced
  layouts and the same rects as a fresh layout; `[style*=x]` rule / `!important` / `width: auto` /
  `display` change -> full layout.
- **QYTC1b [S] (P2, after QYTC1a)** MEASURED, NOT MERGED (implemented and tested, 0 restores on the YouTube
  recipe; code on branch `scratchy/qytc1b-ledger-code`, see "QYTC1b result" below). A write that puts an attribute back to its value at the last forced layout
  cancels its own bump.
- **QYTC1c [S] (P3, measure)** css_collect + rule_index are 18 % of every forced layout on the JS
  thread (~5 ms each here): check whether `RULE_DERIVED`'s index is rebuilt per layout there.

### QYTC1a result (2026-11-19, release build of the branch, main 66fc252 + QYTC1a)
`GeomCache` keeps a flat pre-order skeleton of the laid tree (every `Laid`, anonymous ones too:
node, box, `walk` extent, subtree end, parent, a few computed-style flags). `__d_setattr` of `style`
calls `GeomCache::style_write_in_place` before bumping: if the cache is the current layout, no
selector/`attr()` reads `style` (QY53o1's `AttrReaders`), no `!important` rule declaring
`left`/`top`/`transform` (or an `!important` `var()`/CSS-wide value) MATCHES the element, and the parsed
old/new declarations (normal parts; the `!important` parts must be equal and not set those three)
differ only in `opacity` (both declared and < 1: `opacity < 1` is a stacking boundary that changes
hoisting), `background-color` (both declared: an inherited inline background adds a spacer box),
`background-position/-size`, `transform`, `left`, `top`, then:
- nothing moves (paint fields, `scalex` beside or instead of a translate): no bump, on any box;
- on an `absolute`/`fixed` box with no running animation: a translate delta (`px` / `%` of the box),
  `left` in px (needs a definite `width` and `right: auto`), `top` in px (needs `bottom: auto`), both
  sides declared inline; the box's laid subtree shifts, PLUS the subtrees of its DOM descendants that
  `hoist_negative_z` moved into a stacking-context owner (they were placed inside it before hoisting;
  YouTube's scrubber knob is one); a descendant inside a `fixed` subtree makes it fall back. The
  ancestors' extents are redone bottom-up from their children's. Glyph rects are not shifted:
  `glyphs_stale` makes the next `text_rects` read lay out. A shift adds to a counter that `__d_ggen`
  adds to the geometry generation, so the IntersectionObserver recheck and the Range glyph cache see
  the move (the recheck is served from the fresh cache).
Anything else bumps as before. An abs box in normal flow cannot move a sibling: `in_flow_bottom`
(the root height) never enters `positioned` lists, so the page size stays exact; an in-flow
translate could change it, which is one reason in-flow boxes only take the no-move path.

R50 recipe (own nested dwl 960x1006, own wire, `SCRATCHY_MEDIA_AO=null`, `SCRATCHY_FRAME_STATS`,
seconds with >= 40 pointer moves), every run under the perf flock, load average 13-24 (other agents
building). "main" is the installed binary of main at e017cd0 (QY53o1 included), copied before the runs.
| run | forced/s (median) | ms per layout | forced ms/s | video painted/s | lat_p95 ms | js_ms/s |
|---|---|---|---|---|---|---|
| main base1 (load 24 -> 15) | 11.0 | 46.8 | 570 | 15.0 | 120 | 738 |
| main base2 (load 19 -> 16) | 17.0 | 33.5 | 594 | 20.0 | 78 | 760 |
| QYTC1a new1 (first build: no effect, see below) | 12.0 | 48.0 | 583 | 15.0 | 114 | 763 |
| QYTC1a new2 (load 23 -> 19) | 1.0 | 55.6 | 57 | 21.0 | 77 | 420 |
| QYTC1a new3 (load 16 -> 13) | 1.0 | 39.1 | 44 | 42.0 | 51 | 323 |

Forced layouts while the controls show: ~11-17/s -> ~1/s (gate was ~3-4/s), forced ms/s ~580 -> ~50,
script+layout ms/s ~750 -> ~320-420. new1 was the first version, which refused the write when ANY
rule of the list declared an `!important` `left`/`top`/`transform` (YouTube has such rules); the
check is per element now (selectors of those few rules matched against the written element). A
throwaway trace (reverted) of one run counted 306 in-place shifts, 160 no-move writes, 66 writes on
an already stale cache and ~20 fallbacks (`width` changes on resize, `display: none` boxes).

### QYTC1b result (2026-11-20, release builds, main 01545ab vs main + QYTC1b)
**Not merged:** implemented exactly and tested, but 0 restores were observed, so the per-write bookkeeping is not worth it. The code is kept on branch `scratchy/qytc1b-ledger-code` for reference.

`GeomCache::ledger`: while the cache is the current layout, each attribute write that would bump the
geometry generation (`__d_setattr/rmattr/setq/rmq`; not the ones skipped as hidden/unread, not in-place
style shifts) is recorded as (node, name, value at layout time, current value). The ledger is trusted
only while every bump since the layout came from a recorded write (`expected` generation) and the CSS
version is unchanged; any other bump (insert/remove, innerHTML, text, sheet, `setns`, a `style` write
that was not shifted in place) makes the next write drop it and the next layout starts a new one. When
all recorded attributes equal their layout-time values again the cache key is rebased onto the new
generation (the generation still moves, so every other generation-keyed cache stays conservative;
HIDDEN/ATTR_SNAP are rebased with it). Exact: `class="a b"` -> `"b a"` or absent -> `""` count as
different. Tests `qytc1b_*` (class add+remove x5 -> 1 layout and the same rects; add+remove+add, a
restore beside another changed attribute, innerHTML / insert / new sheet in between -> layout).

R50 recipe, window 960x230 this time (the nested output gave 960x230, so inject used 960 230 with the
click at (346,150) and the circle at 346+-60,150+-30; the 1006-high recipe clicked outside the window and
showed no controls), seconds with >= 40 pointer moves (9 per run), every run under the perf flock, load
average 13 -> 8 (base1), 3 (new1), 2 (base2), 0.4 -> 1.4 (new2):
| run | forced n (9 s) | forced ms | forced/s (median) | ms per layout | lat_p95 median ms | js_ms/s |
|---|---|---|---|---|---|---|
| main base1 | 42 | 1728 | 2 | 41.1 | 64 | 361 |
| QYTC1b new1 | 44 | 1841 | 2 | 41.8 | 57 | 363 |
| main base2 | 39 | 1586 | 3 | 40.7 | 55 | 352 |
| QYTC1b new2 | 45 | 2274 | 2 | 50.5 | 70 | 363 |
No difference. A throwaway trace of one run (reverted) counted 0 restores in ~1900 ledger events:
the ledger is almost always polluted before a restore can complete. After QYTC1a the layouts that remain
come in bursts of 9-10 per second that start from a stale cache: the progress `style` writes
(`transform: translateX(..)`, `left: 0px; transform: scalex(..)`, the tooltip's `left:`) plus
`aria-valuenow/valuetext`, and `class` writes on the player (`ytp-autohide`... with other classes
changing at the same time), none of which return to their layout-time value, so "every pending write
restored" never holds. The restore pattern R50 saw (11-13 %) was measured before QYTC1a and is a much
smaller share of what is left. Next step if wanted: let `style_write_in_place` also work when the only
bumps since the layout came from ledger-recorded writes whose boxes the shift does not touch
(it requires the cache to be the current layout today).

### QYTC1d result (2026-11-20, release builds, main 83a9d93 vs main + QYTC1d 9303ef0 + 91b9e73)
Instead of a ledger, the writes that are provably inert no longer move the geometry generation at all,
so the cache stays the current layout (and QYTC1a's in-place path and QY53o1's unread-attribute skip
keep working after them). `aria-*` writes were already inert (QY53o1) whenever the cache was current;
they only bumped because something else had made it stale first. The new case is `class`:
`js_geom::class_write_pre/post` around the write in `__d_setattr`/`__d_rmattr` compare, before and
after, whether each selector of the rules naming a changed class (plus the rules reading `class` as
an attribute: YouTube has `[class*=..]` selectors, so `AttrReaders` alone would refuse every class
write) matches each element the change can reach (the element; its subtree / later siblings' subtrees
when the class sits left of a combinator). No difference = no element's matched rules changed = the
cached layout is still the page's: no bump. Anything else (a rule starts/stops matching, even a
paint-only one; a class inside `:has()`/`:nth-child(of)`, which scratchy does not parse today;
`content: attr(class)`; a probe over 20 000 selector matches; a stale cache) bumps as before.
Tests `qytc1d_*` (mixed inert writes + in-place shift + aria, rects and ink extents equal to a fresh
layout; rule-changing classes, sibling/descendant reach, `[class*=]`, `attr(class)`, stale cache; JS
loop with 5 toggles + aria + style writes = 1 layout; the class index keeps `<style>` inserts spliced).

Trace of one run (throwaway, reverted): of the `class` writes made while the cache was current, about
a third were found inert (`playing-mode`/`paused-mode`/`buffering-mode`, `ytp-autohide-active` removed,
`ytp-large-width-mode`, `style-scope` on some elements, ...); the others really change matched rules
(`ytp-autohide` turns on `.ytp-autohide:not(.ytp-autohide-active) .ytp-title` etc.: paint-only
`opacity` rules, but exactness first), and a few probes hit the 20 000 cap (the player root's subtree
times the many `[class*=]` rules).

Measured: the nested dwl gave 960x1006 this time (not 960x230), so the R50 recipe ran with the click at
(346,261) and the circle 346+-60, 200+-40, as in QYTC1a. At this size the controls phase is already at
~1 forced layout/s, and the trace shows every one of them comes from the time display: YouTube replaces
the text node of `span.ytp-time-current` once a second (remove + insert), a real text change no
attribute rule can skip. Seconds with >= 40 pointer moves (9-10 per run), every run under the perf
flock, interleaved, load average 13-25 (other agents building):
| run | load (start -> end) | forced n | forced ms | ms per layout | lat_p95 median ms | js_ms/s median | whole-run forced n / ms |
|---|---|---|---|---|---|---|---|
| main base1 | 13.5 -> 23.3 | 11 | 495 | 45.0 | 66 | 283 | 107 / 3473 |
| QYTC1d new1 | 23.3 -> 17.0 | 9 | 327 | 36.4 | 56 | 270 | 104 / 2271 |
| main base2 | 15.7 -> 11.0 | 13 | 1062 | 81.7 | 87 | 359 | 129 / 5092 |
| QYTC1d new2 | 8.7 -> 9.8 | 12 | 972 | 81.0 | 88 | 398 | 126 / 4391 |
| main base3 | 9.8 -> 24.9 | 11 | 626 | 56.9 | 144 | 233 | 111 / 3677 |
| QYTC1d new3 | 24.9 -> 19.4 | 9 | 275 | 30.6 | 39 | 218 | 111 / 3072 |
Forced layouts in the controls phase 11-13 -> 9-12 per run, whole run 0-3 fewer: a small effect, at
the edge of the noise at this window size. The 960x230 bursts QYTC1b saw could not be reproduced here
(the window size is the user's tiling). Left: the per-second `ytp-time-current` text replacement
(proposed QYTC1e below); the `ytp-autohide` toggles, whose rules only set `opacity`/`visibility`-like
paint (a "flipped rules declare only box-neutral properties" tier, proposed QYTC1f, needs care:
`opacity < 1` makes a stacking context, which moves hoisted boxes).

**Probe cost (coordinator review of #414).** Headless `--perf-report`, throwaway counters (reverted),
every run under the perf flock, interleaved:
- Worst case, a local fixture (200 elements, each with 10 children; per rAF frame 3 class toggles on
  each, all inert: `.a .nomatch`, `.b > .nope`, a `[class*=zz]` rule; one forced read at the start,
  none after; 12 s). The first version probed EVERY write: 360 000 probes, 495-1 230 ms of probe
  time, JS 794-1 038 ms (main) -> 1 365-3 101 ms (+60 % or more), load 4-12. Fix: a probe budget
  of 32 class writes per geometry read (`PROBE_BUDGET`, refilled in `GeomCache::refresh`, which
  every read goes through); after it the write bumps, the cache is stale, and every later class write
  stops at the cheap `class_probe_possible` gate (budget + `ATTR_SNAP` current) BEFORE the
  `page_connected` walk. Final, 4 interleaved pairs at load 0.2-1.0: JS 794/819/811/807 ms (main)
  vs 824/814/832/806 ms (mean 808 -> 819, +1.4 %); process CPU 960/980/960/960 vs 980/970/1000/960
  (+1.3 %).
- github.com home (15 s, load 1.6-4.1): a single class write per load reaches the probe; JS
  16 962/17 205/21 509 ms (main) vs 17 233/17 487/21 768 (first, unbudgeted version) vs
  17 146/18 167/21 967 (budgeted): the run-to-run spread (forced layouts 13.4-17.5 s per run) hides
  any difference.
- YouTube watch page headless (15 s, load 0.5-3.7): 402 class writes, 11 probes, 17.6 ms probe time in
  total (mostly building `ClassDeps` for YouTube's rule list once), i.e. ~0.6 % of ~3.1 s JS;
  JS 3 305/3 280 ms (main) vs 3 090/3 115 ms (budgeted), forced layouts 86-87 -> 78-79.

## R49: QAN1h: GitHub restyle re-measured. R31's 15-18 s restyle is gone: a page of that size (github.com repo page, 1 809 elements, 16 220 rules) now builds in 126 ms headless and 210-250 ms in a window. But today's github.com HOME is a bigger page (~3 200 elements, 29 342 rules), and a full build of it still takes 0.65 s headless and 1.1-1.55 s in a window. That is the cost of every hover and every JS restyle (input-to-frame 1.3-1.6 s). About half of the build is selector matching (534 candidate rules per match call, 1 344 rules in the universal bucket, 2.3 % of full matches succeed). About a fifth is `var()` re-expansion: each element re-substitutes and re-parses each matched `var()` rule, 19 272 times per build for 694 distinct declaration blocks (Opus, 2026-11-19, measure only, docs only) — OPEN: proposed QAN1h1-QAN1h3 (below)

**Status: measurement only. No product change; the temporary counters were reverted.** Main **a116be4**,
release build (V8 + media) in the item's worktree, own wire on a `/tmp` socket, DEFAULT user agent, logged
out. Machine load average 11-17 during every run, because other agents were building. The absolute ms are
inflated, so compare the pages with each other and use the shares.

### Pages and recipe
- HTML fetched through the own wire at ~19:25: `github.com/` (537 KB, 37 stylesheet links) and
  `github.com/Loupax/vibe-it` (213 KB, 33 stylesheet links). Static copies were made with `<base href>`
  pointing at the live site, every `<script>` removed, and one inline driver script added. The
  stylesheets load live through wire.
- **Headless:** the driver toggles a class on `<body>` and reads `body.offsetWidth` 30 times. That gives
  30 forced full restyles, run as `scratchy --perf-report --budget-ms 20000 --seconds 10 page.html`, and
  the schema-2 `forced_layout.phases` are reported. The live URLs were also run with page JS on, 15 s.
- **Window:** nested dwl (`tools/nest.sh`; the output was 960x316 on this desktop) with
  `SCRATCHY_FRAME_STATS`. For hover, `tools/inject` moved the pointer between 6 targets and back to
  blank space every 2.5 s (12 hover changes). For a JS restyle, a second static copy toggles a class on
  `<body>` every 2 s, 8 times, and reads no geometry.
- **Profile:** `tools/profile.sh` with its own `PROF_TARGET` on the headless home driver page (33 007
  samples).
- **Counters:** a separate throwaway build (reverted with `git checkout`). It recorded per build: match
  calls, `RuleIndex` candidates, candidates that pass the media check, candidates that pass the ancestor
  bloom filter (that is, fully matched), and matches. It also recorded `expand_var_decls` calls and time,
  distinct `Declarations` with `var()`, plus bucket sizes and a 400-rule sample of the universal bucket.

### Numbers
| | github.com home | repo page | R31 copy (2026-11-17) |
|---|---|---|---|
| elements per build (style resolves / builds) | ~3 215 | ~1 809 | 2 077 |
| rules (with `var()`) | 29 342 (16 845) | 16 220 (9 191) | 16 192 |
| headless forced restyle, mean / max (30 runs) | **647 / 1 655 ms** | **126 / 326 ms** | 14 745-17 863 ms |
| per layout: css_collect + rule_index | 5 ms | 3 ms | |
| per layout: `precompute_generated` | 354 ms | 66 ms | |
| per layout: `build_children` | 270 ms | 44 ms | |
| per layout: `measure_document` | 16 ms | 12 ms | 53-55 ms |
| whole build (counter build, 3 builds) | 789-1 361 ms | 204-219 ms | |
| window: hover build (12 `hover_restyles`, 1 build each) | 1 135-1 442 ms, median ~1 330; input-to-present 1.29-1.59 s | not run | |
| window: JS restyle build (8 class toggles) | 1 118-1 552 ms, median 1 348 | 210-253 ms, median 233 | |
| live URL, page JS on, headless 15 s | 23 forced layouts = 25.0 s (mean 1 089, max 2 029 ms) | 0 forced layouts | |

On the live home page, the forced layouts come from two places. One is GitHub's `r`
`85486-d8bd5f8aaf4e9c5d.js:2:314766` (16 layouts, 18.3 s). The other is `C`
`landing-pages-901fb96639d41b66.js:426:64866` (7 layouts, 6.7 s). In two live-window runs of the home
page, the builds took 765-1 457 ms and one forced layout took 956 ms. One of those two runs drew no
frame between 5 s and 42 s, and pointer moves in that time were not counted. The second run painted
by 8 s. That stall was seen once and is not explained here.

**Match counters per build (home / repo):**

| | home | repo |
|---|---|---|
| match calls | 5 777 | 3 438 |
| candidates | 3 087 080 (534 per call) | 849 936 (247 per call) |
| pass the media check | 2 257 059 | 654 562 |
| pass the bloom filter (fully matched) | 1 275 933 | 264 694 |
| matched | 29 851 (2.3 %) | 6 457 (2.4 %) |

**Universal bucket (home / repo):** 1 344 / 604 normal rules and 61 / 45 pseudo rules. In the
400-rule home sample:
- 175 have an attribute-only rightmost compound (`[data-color-mode=auto][data-dark-theme*=light]`,
  `.x [data-component=SkeletonText]`, `[hidden]:not(...)`).
- 41 are `:root` theme blocks.
- 34 are `*` (`.y > *`).
- 26 have a `:is()`/`:where()` rightmost compound whose class is hidden inside it
  (`:where(.prc-Label-Label-qG-Zu):where([data-variant=success])`).
- 26 are other pseudo-classes and 14 are `:not(...)`.
- About 93 are rules where one unkeyed selector in a selector list pulls the WHOLE rule into
  `universal`, although its other selectors have class keys (`RuleIndex::build` sets
  `universal = true` for the rule).

**`var()` (home / repo):** per build, 19 272 / 2 841 `expand_var_decls` entries with `var()` run over
only 694 / 237 distinct `Declarations` blocks. The time is 156-272 ms / 34-39 ms per build (~20 % / ~17 %).
Every element substitutes every matched `var()` declaration again, then does `format!` and
`parse_declarations_split` again.

**Profile, home driver page:** 89.5 % of samples are in `GeomCache::refresh`. Inclusive shares of all
samples:
- `matching_indices_for` **49.1 %**. Inside it: `matches_complex_selector` 31.9 %,
  `class_list_contains` 8.2 % (plus `element_attr` 4.7 %), `RuleIndex::candidates` 5.3 %,
  `media_list_matches` 2.8 %, `selector_may_match` 1.8 %.
- `precompute_generated` **50.2 %**. This is mostly the first match of every element.
  `precompute_generated` matches each element for `display`/counters/`content`, and
  `build_children` then hits the QY53d memo, so the phase name hides that it is the matching phase.
- `build_children` 36.2 %. Inside it: `resolve_style_core` 29.3 %, of which `expand_var_decls`
  **17.2 %**, `cascade_css` 8.2 %, `compute_custom_props` 4.5 %, `Declarations::clone` 2.7 %.
- `measure_document` 1.7 %.
- malloc/free/memmove 10.5 %, spread over all of the above.

### Verdict
R31's case is fixed. The 15-18 s were a page of the repo page's size, which now builds in 126 ms headless
and ~230 ms in a window, about 70-100x faster. That is in line with Wikipedia (174-180 ms hover builds,
R47). Close R31's open question.

The github.com HOME page is NOT in line: 0.65 s headless and 1.1-1.55 s per window build, so every hover
and every JS restyle on it is a second-class stall. Its cost is style, not layout: 97 % of a forced
layout. The cost is spread as follows:
- matching: ~half;
- `var()` re-expansion: ~a fifth;
- the rest of the cascade: ~a tenth;
- allocation: spread over all of these.

QY53n1-QY53n5 (R47: hover without a tree build) remain the structural fix for hover. The items below cut
the cost of every build, including JS restyles and forced layouts, which QY53n does not cover.

### Proposed items (the coordinator files them)
- **QAN1h1 [S] (P2) Memoize `var()` expansion per build.**
  - What: key on (the `Declarations` pointer, the element's custom-properties handle, `CustomPropsRef`
    since QY53i2) and reuse the expanded `(Declarations, Declarations)` groups.
  - Where: `expand_var_decls`, style.rs; both call sites.
  - Lifetime: the memo lives only while a build runs, like `MATCH_MEMO`.
  - Correctness: equal handles mean equal variable tables. Check that `var()` fallbacks and
    `inherit`-dependent values do not read anything besides the lookup.
  - Expected: 19 272 -> ~700-2 000 expansions per home build, -15-20 % of a build.
  - Gate (home driver page, median of 3, interleaved with main): `forced_layout.phases.build_children`
    -30 %, `total_ms` -12 %.
  - Tests: siblings with different custom props, fallbacks, `!important` var groups (Q52b segments).
- **QAN1h2 [M] (P2) Fewer universal candidates in `RuleIndex`.** Three steps:
  - (a) Bucket an attribute-only rightmost compound by attribute local name. An element looks up its
    own attribute names, the way Stylo's selector map does.
  - (b) Look through a single-selector `:is()`/`:where()` in the rightmost compound for an id, class
    or tag key.
  - (c) Index each selector of a rule separately instead of moving the whole rule to `universal` when
    one of them is unkeyed. A rule may sit in several buckets; `candidates` already sorts and dedups.

  The bloom filter's `ancestor_hashes` stay per selector. Expected: universal 1 344 -> a few hundred,
  candidates per call 534 -> ~250 (estimated from the 400-rule sample, NOT measured).
  - Gate (home driver page, median of 3, interleaved with main): candidates per build -40 % (temporary
    counter) and `phases.precompute_generated` -25 %.
  - Tests: attribute-bucket matching incl. case-insensitive attribute values, `:where()` specificity
    unchanged, cascade order unchanged for a split rule.
- **QAN1h3 [S] (P3) Class lookup without re-splitting the attribute.**
  - Measured: `El::has_class` -> `element_attr` + `class_list_contains` is ~13 % of the home build.
    It splits the `class` string on every check, about 1.28 M full matches per build.
  - Fix: during a build, cache each element's class atoms or hashes, for example next to the
    `AncestorBloom` stack entry or in a per-build `HashMap<node ptr, SmallVec<Atom>>`.
  - Gate: `class_list_contains` + `element_attr` under 4 % of `GeomCache::refresh` samples in
    `tools/profile.sh` on the home driver page.
- Not proposed:
  - Re-checking the live window's 37 s no-frame stall on the live home page. It was seen once in two
    runs at load ~15. File it only if it repeats; recipe above.
  - Renaming the `precompute_generated` phase. Documenting it here is enough; QPROF1 should read it as
    "first matching".

### Not established
- Numbers on a quiet machine: load was 11-17.
- Logged-in GitHub pages: never logged in.
- The window at a full-size viewport: the nested output was 960x316, a narrow layout.
- The gains of QAN1h1/QAN1h2: estimates from the counters, no throwaway fix was measured.

## R48: QMEM3: where the YouTube watch RSS goes after QY53f1c. At ~488 MB at the end of a 60 s headless run (568-581 MB in a live window), the QOBS7 `memory` block explains only 149 MB (caches 32 + JS heap 117). Most of the rest is: V8's own overhead beyond `used` and its code range (~18 MB), glibc keeping ~85-94 MB of freed memory in 42 per-thread arenas, libmpv (-112 MB RSS when it is not loaded: 70 MB mapped libraries + ~42 MB in its threads), the scratchy binary (34 MB), and ~65 MB of main-thread malloc that no probe names. The rule caches are ~2.5x their estimates but now small (~40 MB both). No leak in the headless minute; the live window grows ~0.2 MB/s (Opus, measure only, 2026-11-19) — OPEN: proposed QMEM3a-QMEM3e (in the PR); related QY53i2, QMEM1

**Status: measurement only, docs only, no product change.** Base **b276712** (QY53f1c, StyleRule 4040 -> 72
bytes), own worktree release build (V8 + media), own wire on a scratchpad socket, DEFAULT user agent,
`SCRATCHY_MEDIA_AO=null`, https://www.youtube.com/watch?v=G1AeBgalFZk. Four headless runs
`scratchy --perf-report --socket S --budget-ms 20000 --seconds 60 URL` (f, g: default; h:
`MALLOC_ARENA_MAX=2`; i: libmpv not loaded) and one live nested-dwl window (tools/nest.sh), sampled from
outside for 60 s. A first pass on be7c460 (before QY53f1c) is summarised at the end.

### How this was measured (recipe; the probe code was NOT committed)
Temporary code in `perf.rs` `report_page`, right before `memory_json` (so it sees the same state as the
`memory` block), writing into a directory from an env var:
1. Copies of `/proc/self/smaps_rollup`, `/proc/self/smaps`, `/proc/self/status`, thread names
   (`/proc/self/task/*/comm`).
2. glibc `malloc_info(0, FILE*)` (XML: per-arena `system` size and free chunks, mmapped chunks), then
   `malloc_trim(0)`, then smaps_rollup + malloc_info again.
3. V8: `get_heap_statistics` (incl. `malloced_memory`, `peak_malloced_memory`), every
   `get_heap_space_statistics(i)`, `get_heap_code_and_metadata_statistics`.
4. DOM census: walk `dom.document` (incl. template contents): nodes, elements, text bytes, attributes;
   `size_of` of rcdom `Node` (120 B), `StyleRule` (72), `ComputedStyle` (1304).
5. Staged frees, each followed by `malloc_trim(0)`, with `mallinfo2()` (`uordblks + hblkhd`) and RSS after
   each: reset the `StyleCache` (reached through a temporary thread-local `Weak`); drop `RULES_CACHE` +
   `RULE_DERIVED` + `RULE_INDEX` (the derived index holds the same `Rc`, so dropping only `RULES_CACHE`
   frees just the piece text); clear `SHAPE_MEMO`.
6. "No libmpv": `load_lib()` in media_mpv.rs returns `None` under a temporary env var (the inert
   placeholder path, as on a machine without libmpv).
smaps mappings were classified by a small script: `[heap]` = glibc main arena; anonymous rw mappings
starting at a 64 MB-aligned 0x7f.. address = glibc thread arenas; anonymous mappings outside 0x7f..
(scattered 256 KB pages) = V8 heap pages; the 256 MB rwx region = V8 code range; other anonymous 0x7f..
= glibc mmapped chunks, thread stacks, mpv; file mappings by path. RSS throughout, never VSZ.

### Results (run f; g agrees within 7 MB)
End of run: VmRSS 488 / 489 MB (f / g), VmHWM 488 / 490 MB: the peak is the end. h (`MALLOC_ARENA_MAX=2`)
425 MB; i (no libmpv) 376 MB. The `memory` block: caches_bytes 32 MB (custom_props_interner 29-30,
mse_store 2, rules_cache 10.6, style_cache 7.7) + js_heap.used 117 MB = 149 MB "explained" = 31 %.

| Part (base b276712) | RSS MB | Evidence |
|---|---:|---|
| V8 heap pages + code range | 135 | 461 anon 256 KB pages (129 MB) + rwx code range 6 MB. `total_physical_size` 142 MB vs `used` 123 MB: ~18 MB is V8 slack (new space 16.8 MB mostly empty). Zones now 1 MB (`malloced_memory`); `peak_malloced_memory` 61 MB |
| glibc: freed but retained in arenas | 94 (g: 85) | malloc_info: 42 arenas, system 231 MB, free 94 MB (main 18; 41 thread arenas ~76: 15 V8 workers, mpv/ffmpeg threads, the shaping worker; 1.3-12 MB each, most with < 1 MB in use). `MALLOC_ARENA_MAX=2` (h): 2 arenas, free 24 MB, RSS **-63 MB**. `malloc_trim(0)` at the end: only -29 / -34 MB |
| libmpv: mapped library files | 70 | ~1300 file mappings (54.6 MB other libs + 15.8 MB libav*), absent in run i; mostly Shared_Clean (44-62 MB vs 5 MB in i): page cache, Pss lower |
| libmpv: anonymous (its threads' arenas, decoders) | ~42 | i vs f: RSS -112 MB = 70 MB files + 41 MB less thread-arena RSS (overlaps the "retained" row) |
| Files: scratchy binary | 34 | r-x 26 MB + r-- 7 MB |
| `custom_props_interner` (leaked, QY53i2) | 29 | QOBS7 estimate, not re-measured |
| Layout rules: `rules_cache` + `RULE_DERIVED` + `RULE_INDEX` | 20 | staged drop: malloc in-use -24 MB, RSS -20 MB. Estimate 10.6 MB (+ hover keys 0.02): 2.3x undercount (selector heap, index) |
| getComputedStyle rules: `StyleCache` (its own parse of the same CSS) | 20 | staged reset: in-use -21 MB, RSS -20 MB. Estimate 7.7 MB: 2.7x undercount |
| Page CSS strings | ~9 | `EXTRA_CSS` 4.4 MB + rules_cache piece text ~4.5 MB (inside the rows above / below) |
| Shape memo (this thread) | 1.5 | staged clear: in-use -4 MB, RSS -1.5 MB (QOBS7 estimate 0.3 MB). The shaping worker's copy is not reachable |
| DOM | ~4 | 9 011 nodes (4 855 elements, 3 415 texts) x 120 B + 1.3 MB text + 8 402 attributes (0.4 MB values) |
| MSE store | 2.4 | QOBS7 probe |
| Thread stacks, mpv misc mappings, other 0x7f.. chunks | ~11 | QSTK1 kept segments: 0; 26 threads, a few touched pages each; the 32 MB shaping stack is not resident |
| **Not attributed: malloc in use** | **~65** | malloc in use 148 MB (arenas 137 + mmapped 11) minus the rows above. Main arena alone holds 119 MB in use. Candidates (not measured): the fetched script sources and HTML kept until the end of `report_page`, cosmic-text FontSystem/fontdb (main thread + shaping worker), the JS DOM id table and bindings, net buffers |

Live nested-dwl window (b276712, same page): VmRSS 568 MB at 5 s, 581 MB at 60 s (+0.22 MB/s, steady),
+80-93 MB over headless: thread arenas 137 MB vs 87 MB (App, shaping and render threads), V8 pages 148 vs
129 MB, main arena 148 vs 137 MB, fonts 4.5 MB. No malloc_info was taken in the window (external probe only).

### Top 3 consumers QOBS7 cannot explain (b276712)
1. **libmpv: ~112 MB** (70 MB mapped libraries, mostly shared page cache, + ~42 MB anonymous in its
   threads). Needed while a video plays; loaded on any page with a media element.
2. **glibc arena retention: 85-94 MB** freed but kept in 42 arenas; `MALLOC_ARENA_MAX=2` returns 63 MB
   (forced-layout total of the capped run 4156 ms vs 2848 / 3109 ms default: ONE run, inside the noise
   seen on be7c460 (3170-4836 ms), so it needs a real 3+3 gate). `malloc_trim(0)` alone: ~30 MB.
3. **~65 MB of main-thread malloc in use that no probe names**, plus the rule caches being ~2.5x their
   estimates (40 MB real vs 18 MB estimated).
V8's slack (~18 MB beyond `used`) and the 34 MB binary are the remaining smaller items.

### QMEM3b: naming the ~65 MB of main-thread malloc (Sonnet, 2026-11-19, c. 98e32bd + probes)
Method: a TEMPORARY sampling heap profiler (never committed: a `#[global_allocator]` over `System` that
samples every ~8 KB of allocation, keeps a live-pointer table with a libc `backtrace()` per sample, dumps the
live samples at the end of `report_page`, resolved with `addr2line`; saved only in the agent's scratchpad),
profiling build (release + line tables), own wire, YouTube watch G1AeBgalFZk, `--budget-ms 20000 --seconds 60`,
`SCRATCHY_MEDIA_AO=null`, under the perf flock, load average 16-30 (other agents compiling). No heaptrack or
valgrind on this machine. A Rust allocator sees ONLY Rust allocations; V8 and libmpv call malloc directly.

At the end of the run (profiled run): glibc `in_use` 131 MB = **106 MB Rust** + **~25 MB C++** (V8 off-heap
malloc, libmpv/ffmpeg, ICU). The 106 MB of Rust, by owner (sampled, +-2 MB):

| Owner | Live MB | Was probed? |
|---|---:|---|
| `rules_cache` + `RuleIndex` + parsed blocks + keyframes (selector parse 15, rule parse 5, index 4.6, piece text 4.4, `ParsedBlock` 3, keyframes 2.3) | 23.2 | yes, est 15.6 + 1.9 (1.3x low) |
| custom-property interner (`compute_custom_props`) | 17.4 | yes, est 15.5 |
| `StyleCache` (getComputedStyle; its own parse of the same CSS, 4.4 MB of it `EXTRA_CSS`) | 15.7 | yes, est 12.7 |
| **fetched script sources** (`fetch_script_via_wire` 14.2 + inline `script_ref` 1.3) | 15.5 | NO, new `report_scripts` |
| **fetched stylesheet text, joined** (`fetch_stylesheets`: 4.4 MB of CSS in an 8.8 MB capacity) | 8.8 | NO, new `report_css` |
| **page HTML + DOM text** (`perf_report` string 5.8 incl. 4.4 capacity slack of the lossy UTF-8 copy; DOM nodes via html5ever/`js_dom` 7.1) | ~8 + 7 | NO, new `report_html`, `dom` |
| MSE store (fMP4 reader) | 4.4 | yes, est 3.2 |
| shaping (cosmic-text buffers, this thread) | 3.8 | partly (`shape_memo` 1.3) |
| V8 startup snapshot (`finish_bake`) | 1.7 | no (small; left) |

So the "~65 MB no probe names" is, of the 106 MB Rust: **~39 MB of source text and DOM nobody probed** (scripts
15.5, CSS 8.8 + the `EXTRA_CSS` copy, HTML ~6, DOM ~7) plus **~16 MB of probe underestimates** (rules 1.3x,
style cache 1.2x, interner 1.1x, MSE 1.4x, shaping 2.9x) plus **~25 MB that is not Rust** (V8's own malloc,
libmpv; `malloced_memory` of V8 was 1 MB zones + backing stores, the rest is libmpv/ffmpeg threads).

Which of the 39 MB is a live-window cost? The fetched **scripts and the page HTML are held only by
`perf_report`'s locals** until the report ends. A window moves them into `AppEvent::StreamCss` and drops them
once `run_page_scripts` returns (only module sources stay, in `ModuleStore`; YouTube is classic scripts). So
they are a harness artefact: ~23 MB of the 65 in a headless run, ~0 in a window. The CSS text (4.4 MB) and
the DOM (~3.3 MB by the new probe; rcdom is 120 B/node) live in both. The window's real remaining weight is
unprobed C++ (V8 + libmpv) and glibc retention (QMEM3a/3d).

**Simple bug found and fixed (smallest fix):** `fetch_stylesheets` joins the sheets with `push_str`, whose
doubling left 8.8 MB capacity for 4.4 MB of text (YouTube). `shrink_to_fit()` at the end: `report_css` 8.8 ->
4.4 MB, malloc `in_use` -4 MB (137 -> 133 MB, 2 runs each). VmRSS did not move beyond noise (430-433 -> 429; the
freed block goes to glibc's free list: `free_kept` +7 MB), so this is an allocator-level saving only until
QMEM3d trims. The window keeps this string for the page's life (`App.extra_css`), so it is the same 4.4 MB there.

**New probes** (`memory.caches`): `js_extra_css` (EXTRA_CSS capacity), `dom` (nodes + attribute/text bytes),
`report_html`, `report_scripts`, `report_css` (the harness's own held text; zero outside `--perf-report`);
`memory.rss_anon_bytes` (`RssAnon`). With them, YouTube watch: explained_bytes 209-214 MB of VmRSS 430-433 MB.
Gate "explained >= 60 % of (anonymous RSS - malloc free)" (RssAnon 346 - free 46 = 300 MB on YouTube): 212 / 300 = 71 %.

**Smaller page (Wikipedia Rust, `--seconds 30`, profiled run, load 15):** malloc in_use 106 MB = 86 MB Rust + ~20 MB C++.
Of the 86 MB, **~62 MB is cosmic-text** (`shape_fallback` 17.6, `layout_to_buffer` 12.6, attrs `RangeMap` 12.2,
`Buffer::set_text` 9.9, `ShapeLine::build` 6.3, `ShapeSpan` 1.8) allocated from `layout::measure_mixed_inline` /
`shape_or_resume_word`, i.e. the main thread's `SHAPE_MEMO` (7 694 paragraphs, ~8 KB each); the QOBS7 estimate
was 23.8 MB (2.6x low; YouTube 2.9x). `shape_memo_mem` is now scaled by 2.7 (calibrated, not exact). Before the
scale, explained was 58 MB / (RssAnon 181 - free 34 = 147) = 39 %, after it ~94 / 147 = 64 %. The rest of the page: DOM
4.4 MB (19 636 nodes), style/rule caches ~4.5 MB, the Utf8 lossy decoder + tendrils ~3 MB, `walk_glyphs` 2.4 MB.
Open question (not S): is a 8 KB-per-paragraph memo worth its hit rate on a 7.7k-paragraph page (QMEM3j [S] measure
`SHAPE_MEMO` hits vs bytes and cap it, same as QY53o3c).

Proposed (not done, > S): QMEM3g [M] a counting `#[global_allocator]` behind a cargo feature (`memprof`) plus
`tools/memprof-resolve.py`, so the next breakdown does not need a throwaway patch; QMEM3h [S] V8 off-heap
(`malloced_memory`, ArrayBuffer backing stores) and per-library mapped RSS as probes (names the ~25 MB C++);
QMEM3i [M] drop `StyleCache`'s second parse (already QMEM3e) and measure the 1.2-1.3x probe undercounts.

### QMEM3j: SHAPE_MEMO hits vs bytes, and the fix (Opus, 2026-11-20, base 777faf3, fix 80d5353)
Method: throwaway probes (not committed) at the end of `--perf-report`: malloc `in_use` before and after
replacing every memo buffer by a clone (a clone has exact capacities) and after clearing the memo; per-component
capacity slack read through cosmic-text's public `shape_opt`/`layout_opt`. Own wire, every measured run under
`flock /tmp/scratchy-perf.lock`, load average 4-24 (other agents building), so the timing numbers are noisy;
counts and bytes are exact.

**Hit rate (Wikipedia Rust, headless, JS thread).** 7 982 entries, 104 k chars (13 chars/entry: most are the
word/chunk shapes of mixed-inline paragraphs). 4 forced layouts: 62 985 hits / 7 984 misses (89 %); every key
misses once and then hits ~2x per layout. The shaping worker in a window: 8 213 entries, 97-99 % hits over 20-50
builds. YouTube watch: ~600 entries (headless, 98 % hits), ~150-300 in the worker. So nearly every entry is a hit
that matters: an entry or byte cap below the page's own set would turn the full-page scans into LRU thrash
(0 hits). The fix had to make each entry smaller, not drop entries.

**Where the bytes were (69.6 MB real for 7 982 entries = 8.7 KB each):**
- 26.6 MB capacity slack: `Buffer::lines` grows by push to capacity 4 (BufferLine is 328 B: 7.8 MB),
  `ShapeLine::spans` is reserved per BYTE of the line (6.0 MB), word glyph vectors (5.4 MB), layout glyph
  vectors grown by doubling (3.9 MB), word lists (0.8 MB).
- 11.8 MB attribute B-tree leaves: `set_rich_text(.., &Attrs::new(), ..)` stores every span that differs from
  the line default in a `RangeMap` (a BTreeMap: its first entry allocates an 11-slot leaf of ~1.5 KB), so even a
  one-span paragraph paid 1.5 KB.
- the rest (~31 MB) is the shaped/laid-out glyphs themselves (103 k ShapeGlyph at 96 B + 103 k LayoutGlyph at 88 B),
  texts, the paragraph tuple and malloc headers.

**Fix (layout.rs):** `shape_spans` passes the FIRST span's attrs as the line default (one-span lines keep no
B-tree; cosmic-text already resets an empty line to the span's attrs, so shaping is identical: tested against
the old default) and shrinks `lines`; `memo_put` replaces the new entry's buffer by its clone (exact
capacities; the entry was just made, nothing else holds it). The clone costs 16-25 ms CPU per cold Wikipedia
layout (8 k entries), inside the noise of the forced-layout totals (1.1-2.5 s per run here).
`shape_memo_mem` now sums those public capacities (29.5 MB estimated vs 31.4 MB real; the old x2.7 guess said
67 MB), and the report has `memory.shape_memo_lookups` {hits, misses}.

| Wikipedia Rust, headless `--perf-report --seconds 10`, 5 interleaved runs each | base 777faf3 | QMEM3j |
|---|---:|---:|
| memo real bytes (freed by clearing it) | 69.6 MB | **31.4 MB** |
| malloc `in_use` at the end (median) | 107.5 MB | **69.2 MB** (-38.3) |
| VmRSS at the end (median) | 281.0 MB | **243.5 MB** (-37.5) |
| memo hits / misses | (no counter) | 62 985 / 7 961 (same entries: 7 959 vs 7 982) |
| forced layouts, ms per layout (median) | 266 | 194 |
| slowest forced layout = the cold one (median) | 439 ms | 321 ms |
| measure_document per layout (median) | 81 ms | 61 ms |

Live window (nested dwl, own wire, 100 s, pointer moves over links every 10 s), Wikipedia, 4 interleaved pairs
with RSS sampling: VmRSS at the end base 383/396/393/392 MB, QMEM3j 345/321/315/319 MB (**-55 to -78 MB**: the
JS thread's memo and the worker's memo, which the App's displayed ShapeCache shares, both shrink). App builds:
per-build ms base 378/397/382/275, QMEM3j 369/256/149/275; worker shaping ms per build likewise not worse. Screenshots
at t=30 s byte-identical base vs QMEM3j (4 of 5 pairs; the 5th differs only because the nested output changed size between the two runs).
YouTube headless (3 pairs, --seconds 20): memo ~1.6 MB (was ~3.8 real), in_use/RSS within noise (V8 and libmpv
dominate), measure_document per layout 3.2 vs 3.5 ms (noise; the memo there is ~600 entries).

**QY53o3c answered (same work): yes, the worker's memo grew without bound on a long-lived page.** It was cleared
only on a navigation, resize or non-restyle rebuild (`reuse_shaping == false`), and YouTube's rebuilds are almost
all restyles: with the sweep disabled a 300 s YouTube watch window went from 57 entries (build 18) to 291 entries /
0.8 MB (build 249), +1.2 entries per build, ~3 KB/s (~10 MB/h; the clock and counters YouTube rewrites every
second each leave a dead shape). Fix (app.rs `spawn_shaping_worker`): the worker tracks the keys each build uses
and, when a build FINISHES (not superseded, not panicked), `memo_sweep(32768, keep = 8)` drops what none of the last
8 finished builds used (a hover toggled off, a write undone, still finds its shape); the JS thread keeps its
`keep = 1` sweep (QY53o3a).With the sweep, two YouTube windows (240 s, 300 s) held
84-93 entries (0.2 MB) from build 66 to 195, and 584 entries (1.6 MB, a different page state) flat from build 44
to 138. YouTube live RSS at the end: base 403 MB, QMEM3j 400 MB, sweep off 399 MB (the memo is too small there to
show in RSS); per-build ms 24 (base) vs 29 (QMEM3j) vs 30 (sweep off), worker ms per build 2.2 vs 2.2 vs 2.0: noise
(different build counts, 75 vs 190 vs 203, because YouTube's own timers differ per run).

### First pass on be7c460 (before QY53f1c), for the record
Same recipe, 5 headless runs + a live window: 673-677 MB headless, 731 MB live. The page's 22.6k rules were
held TWICE at 4040 B per `StyleRule`: StyleCache's `Vec` as one 173.8 MiB mmapped chunk with 86.9 MiB
resident (capacity ~2x len, so QOBS7's 183 MB `style_cache` counted ~95 MB of untouched capacity), and
layout's list as a second 86.9 MiB chunk; staged reset of StyleCache: RSS -106 MB. QY53f1c's 72-byte
`StyleRule` removed both chunks (-~190 MB RSS, consistent with its own 698 -> 507 MB). Everything else
(V8 ~143 MB, glibc retained 93 MB with -59 / -62 MB under `MALLOC_ARENA_MAX=2`, libmpv -101 MB, binary
34 MB) matched the b276712 numbers above.

## R47: QY53n: what one `:hover` restyle costs and how to make it cheap. On Wikipedia every hover change over a link is a whole App tree build (174-180 ms on a quiet machine) that changes the matched rules of 1-2 of 10 650 elements, and the change is ALWAYS paint-only (`a:hover` colour/underline; 37 of 37 hover builds). With page JS on, pointer moves over links cost far more elsewhere: 33-38 JS-triggered builds per run that change NO style (same-value `li style="display: none;"` writes, `title` toggles; 6.6-7.9 s) and 35 forced layouts (14.5-16.8 s). On YouTube hover builds are a minority (25-33 of 95-108 builds per run), and in the user's frozen session 6 of 55 (Opus, 2026-11-19, docs only) — OPEN: QY53n1-QY53n5, QY53m2, QY53o1-QY53o3 (proposed below; QY53o measured in the addendum at the end: the forced layouts are Page Previews' `getBoundingClientRect` and scratchy's own IntersectionObserver recheck, 63-86 % of them after nothing but a `title` toggle)

**Status: measured live, design + items, docs only (no code change; the instrumentation was reverted).**
Release build of main ad764fa (V8 + media), own worktree, own wire on a scratchpad socket (`$!`), nested
dwl (tools/nest.sh), DEFAULT user agent, `SCRATCHY_MEDIA_AO=null`, `SCRATCHY_FRAME_STATS` (QOBS5). The host
gave the nested window **956x312** in every run (re-tiling, not ours; inject size 960x316). **Machine load:**
other agents were compiling; load average 11-13 during the instrumented runs, 4.6-6.4 during the baseline
runs, so the absolute ms of the instrumented runs are ~1.3-2x high. Use the ratios and the baseline rows.
**Input (tools/inject, from t = 10-13 s):** 6 rounds of `move:` over 8 positions with `wait:600`, ~29 s.
Wikipedia Rust: links only (5 links in the lead paragraph, Talk, Edit, View history). YouTube watch
G1AeBgalFZk: a click on the big play button first, then 5 points inside the picture (the player shows its
controls) and the masthead's search button, menu and avatar buttons.
**Throwaway instrumentation (env `QY53N_DIAG`, never committed):** one line per App tree build in
`App::redraw` with its cause (a relevant `apply_hover` / a `request_js_restyle` since the last build), the
phase split of `build_layout_tree_with_resources` (css collect, rule index, `precompute_generated`,
`build_children`), the time inside `resolve_style_for` and inside real selector matching
(`matching_indices_for` on a `MATCH_MEMO` miss), element and pseudo resolve counts, and a DIFF of the
build's `MATCH_MEMO` against the previous build's (key = node pointer + pseudo slot): entries whose matched
rule set changed, new/gone entries, element descendants of the changed elements, and every rule in a
changed set's symmetric difference classified as PAINT-only or not by its `Declarations` (paint = color,
background-*, text-decoration-*, opacity, outline-*, box-shadow, border-*-color, border-radius, visibility,
cursor, also by name in `wide`/`var_decls`; custom properties count as not paint). Also one line per
`ShapeResult` handling and per JS DOM write (`__d_setattr` etc.: tag, attribute, same value or not).

### Runs (instrumented unless "base"; 50-66 s each)
| run | JS | builds n / ms | hover builds n / ms (median) | JS builds that changed NO style: n / ms | JS builds that changed styles or the DOM: n / ms | forced layouts n / ms | lat_p95 median / max ms |
|---|---|---|---|---|---|---|---|
| wp2 | on | 52 / 11 244 | 6 / 1 210 (213) | 38 / 7 851 | 2 / 448 | 35 / 15 730 | 485 / 1 306 |
| wp3 | on | 55 / 12 849 | 11 / 2 176 (183) | 38 / 7 875 | 0 | 35 / 16 757 | 547 / 2 082 |
| wp4 | on | 53 / 10 893 | 8 / 1 581 (213) | 33 / 6 645 | 7 / 1 262 | 35 / 14 492 | 427 / 1 703 |
| wpn1 | `--no-js` | 20 / 8 247 | 18 / 6 860 (406) | - | - | 0 | 551 / 1 366 |
| bn1 base | `--no-js` | 22 / 3 138 | 18 hover restyles: **174 ms of builds per hover_restyle** | - | - | 0 | 296 / 2 109 |
| bn2 base | `--no-js` | 21 / 3 235 | 18: **180 ms per hover_restyle** | - | - | 0 | 153 / 2 138 |
| yt1 | on | 95 / 6 963 | 25 / 2 261 (65) | 29 / 1 158 | 37 / 3 175 | 187 / 7 427 | 58 / 1 026 |
| yt2 | on | 108 / 5 258 | 33 / 1 534 (45) | 32 / 1 630 | 40 / 1 848 | 206 / 8 053 | 50 / 240 |

The rows do not add up to "builds": stylesheet-changing builds during load (2-4 per run) are left out.
wp1, the first run, had only 1 hover build: the page's mouseover JS kept a build in flight, so the
R2.3/QB3 coalescing deferred every hover; it is left out. "JS builds that changed NO style" = no element's
matched rule set changed and no element appeared or went: the rebuilt tree equals the previous one.
The user's session (fs-user-yt-1715.jsonl, 38 s, real mouse, 1 582 moves): 55 builds = 2 855 ms, **6**
hover restyles, js 23 807 ms, **382 forced layouts = 18 200 ms**, paint 639 ms.

### 1. Where one hover restyle's time goes (Wikipedia, 10 652 elements + 799 pseudo resolves per build)
All of it is the main-thread tree build: nothing is skipped for a hover. Medians of the wp3 hover builds
(183 ms there; 174-180 ms on the quiet machine):
- css collect + rule index (cached `Rc`): **~1 %**.
- `precompute_generated`: **45 %** (82 ms): a selector-match pass over EVERY element (counters, generated
  content, markers), which fills `MATCH_MEMO`. Selector matching itself (`matching_indices_for`, timed
  separately) is **36 %** of the build (67 ms) and happens mostly here.
- `build_children`: **55 %** (100 ms): `resolve_style_for` for every element and pseudo-element (cascade on
  the memoised matches, `var()`, values) is **43 %** of the build (79 ms, incl. a few memo misses); the DOM
  walk and box construction are the remaining **~12 %**.
- Shaping is NOT on the main thread: the worker re-runs `measure_document` (content-keyed `SHAPE_MEMO`, so
  only the hovered paragraph is re-shaped) in 100-200 ms of worker wall time, and R2.1 (`hold_old_frame`)
  keeps the old frame until that is complete, so the visible hover latency is build + worker (lat_p95
  median 153-551 ms). `handle_shape_result` costs 0.2-1.2 ms on the main thread; paint 2-5 ms per redraw
  at this window size.
YouTube watch (1 100-3 160 elements): hover builds 45-81 ms with the same split (precompute 33-46 %,
build_children 54-64 %).

### 2. How many elements change, and which properties
| page | hover builds | elements whose matched set changed | their element descendants | paint-only | offset-only (translate, `top`) | layout (border-width/style) | nothing changed |
|---|---|---|---|---|---|---|---|
| Wikipedia (wp3, wp4, wpn1) | 37 | 1-2 (of 10 650-11 130) | 1-6 | **37 (100 %)** | 0 | 0 | 0 |
| YouTube (yt1, yt2) | 58 | 0-6 (of 1 100-3 160) | up to 27 | 21 (36 %) | 15 (26 %, `.ytp-*` player rules) | 16 (28 %, `.ytSearchboxComponentSearchButton:hover`; 5 of them also offset) | 6 (10 %: HoverKeys let them through, the cascade did not change) |

Wikipedia's are `a:where(:not([role=button])):hover` (colour + underline) and, on the Talk/Edit/View history
tabs, `.vector-menu-tabs ... a:hover` plus their `::after` pseudo-element (3 of 11 and 5 of 8 hover builds
in wp3/wp4 changed a pseudo slot; 0 of 18 in the content-link run wpn1). YouTube's player rules are
descendant rules (`.ytp-progress-bar-container:hover .ytp-timed-markers-container`), so the affected set
includes descendants of the hovered element.

### 3. Hover builds are not the main cost of pointer moves with page JS on
- **Wikipedia:** while the pointer moves over links, the page's mouseover JS writes `style="display: none;"`
  to an `<li>` (no id/class) with the SAME value 154-165 times per run and toggles `title` on the hovered
  link (30 per run, value-changing; presumably Page Previews, not checked). Every batch sets the JS dirty
  flag, so 33-38 App builds per run (6.6-7.9 s) rebuild a tree equal to the previous one: **4x the hover
  builds**. Same-value writes are QY53m's case; `title` changes a value no selector reads (QY53m2 below).
  Bigger still: **35 forced layouts per run, 14.5-16.8 s** (~450 ms each, at 1024x768: R43 section 11
  Finding 0), all inside the pointer phase (none before or after it). Who reads is not established
  (QOBS5c's window attribution will tell).
- **YouTube:** hover builds 1.5-2.3 s per run vs JS builds 3.5-4.3 s, forced layouts 7.4-8.1 s, page JS
  15.1-15.2 s. The user's frozen session: 6 hover restyles, 55 builds (2.9 s), forced 18.2 s. **Making
  hover restyles cheap will not fix the YouTube freeze**; R46 (QM1k1n6a-c) and the forced-layout work
  (QY53) do. It does fix Wikipedia's hover feedback (174-180 ms build + worker per link), and it would
  patch 36 of 58 YouTube hover builds (paint/offset) and skip 6.

### 4. Design: a hover restyle without a tree build
Today: `apply_hover` -> `set_interaction_state` -> `layout_dirty` + `restyle_requested` -> full build ->
worker -> held old frame. New path, tried first in `apply_hover` (its fallback = today's path, unchanged):
1. **Affected set (QY53n1).** HoverKeys also records, per `:hover` compound, what follows it to the right:
   nothing (the subject), a descendant/child combinator, or a sibling combinator. Affected = the elements
   entering/leaving the chain that `may_match` (as today), plus their element descendants when a hover
   compound with that key is followed by a descendant/child combinator, plus following siblings (and their
   descendants) for sibling combinators. `any` (`:hover` nested in `:is()/:not()/:has()`) or an affected
   set above a cap (e.g. 2 000 elements) -> fallback.
2. **Exact check (QY53n1).** The App keeps the last build's per-element match result (the `MATCH_MEMO` the
   build already makes: node pointer + pseudo slot -> rule indices; moved out of the build instead of
   dropped; invalid when the rule-list `Rc`, the viewport or the DOM generation changes). Re-match only the
   affected elements and their before/after/marker slots under the new interaction state (memo off, ~10 us
   each). No set changed -> store the new chain: no build, no repaint (YouTube's 6 of 58).
3. **Classify once per rule list (QY53n2).** `RuleDerived` gets an effect class per rule: PAINT (the property
   list in the instrumentation paragraph above, by `Declarations` field and by name in `wide`/`var_decls`),
   OFFSET (translate-only `transform`; `top/left/right/bottom` of a `position: relative` box), else LAYOUT
   (anything else, custom properties, `content`, `anim`, `late` layers that are not paint-only). A hover
   change takes the highest class among the rules in the symmetric differences.
4. **PAINT: patch, no build, no shaping (QY53n2).** Conditions: no pseudo slot changed, no affected element
   has `animations`/transitions, no layout in flight (`!layout_in_flight && !layout_dirty && !hold_old_frame`;
   else fallback, see 6). Recompute the ComputedStyle of each affected element and its element descendants
   with `resolve_style` (the parent's InheritedStyle comes from resolving the ancestor chain root-down, depth
   ~10-20 resolves, so nothing new is stored per element). Patch the installed `Laid` tree: boxes whose
   `style.node` is in the set get the paint fields (colour, background colour, border colours, opacity,
   outline, box-shadow, visibility, cursor). Inline text: an `<a>` has no `Laid` box, its style lives in the
   paragraph's spans, so the shaping worker records per paragraph which element each span's text belongs
   to (a `Vec<usize>` beside the existing per-span vectors of `ShapedParagraph`); the App rewrites those
   spans' attrs (colour, decoration line and colour) in that paragraph's `Buffer`, re-runs cosmic-text's
   layout for that paragraph only at its existing width (colour and underline do not change advances), and
   updates the per-span vectors (backgrounds, hidden, cursors, decoration styles). Then `request_redraw`.
   Main-thread cost: the affected re-match + a handful of resolves + 1-2 paragraphs + one paint (2-5 ms
   here). The next real build recomputes everything from the DOM + interaction state, so an error in a
   patch never outlives the next build.
5. **OFFSET: patch (QY53n3).** As 4, plus `shift_laid_tree` of the affected boxes by the change of
   `apply_transform_offset` / the relative offset. Hit testing follows (it reads `Laid`).
6. **LAYOUT, pseudo slots, or a build in flight: today's path.** Later (QY53n4) a subtree re-box: keep the
   last `LayoutBox` tree on the main thread, rebuild only the affected element's box subtree with
   `build_children`, splice it in and send the tree to the worker with `reuse` (the main thread pays the
   subtree, not 10 650 resolves). A patch made while a build is in flight would be overwritten by the older
   ShapeResult, so QY53n4 also keeps a "patches since generation N" list and re-applies it on install.
7. **Gating (QY53n5).** R2.3's `HOVER_RESTYLE_INTERVAL` and QB3's `hover_idle_wait` (keyed on
   `last_build_time > 50 ms`) exist because each hover was a 130-170 ms build. A hover that takes the patch
   path skips them; a fallback keeps them. **Frame stats:** `hover_restyles` stays the count of relevant
   hover changes; new `hover` {patched, skipped, built} and `hover_patch_ms`, so the gates below can be read
   from frame stats.

**Interaction with QM1k1n6 (R46).** QM1k1n6a moves the hit test + `update_hover` to once per turn in
`about_to_wait`: that limits HOW OFTEN `apply_hover` runs; this design changes what one `apply_hover` costs.
They meet at `apply_hover`/`hover_restyle_delay` only, so QY53n1/n5 go after QM1k1n6a (same code). A patched
hover is a normal redraw for QM1k1n6b's frame budget (not JS). QM1k1n6c (composite video while
`layout_dirty`) and this design both avoid builds; a patched hover never sets `layout_dirty`, so
`video_tick` keeps its fast path during hovers.
**Interaction with QY53m.** QY53m skips builds after same-value write batches; Wikipedia's same-value
`li style` writes are exactly its case (33-38 builds, 6.6-7.9 s per run here, 4x the hover builds), so with
JS on it is worth more than QY53n on Wikipedia, and it removes the in-flight builds that would otherwise
push hovers to the fallback (point 4). Order: QY53m, then QY53n1/n2.

### Proposed items (coordinator files them)
- **QY53n1 [S]** (style.rs HoverKeys + app.rs `apply_hover`; after QM1k1n6a) affected set + exact re-match
  against the kept match memo; no build when no matched set changed. Tests: a `:hover` rule that HoverKeys
  admits but whose other compounds do not match -> `layout_dirty` stays false; a descendant rule
  (`.a:hover .b{color:red}`) puts `.b` in the affected set. Gate: YouTube watch, pointer over the player,
  60 s: frame stats `hover.skipped` > 0 and builds per hover_restyle drop by that count; Wikipedia
  screenshot of a hovered link unchanged.
- **QY53n2 [M]** (style.rs RuleDerived effect classes; layout.rs span->element vector; app.rs patch) the
  paint-only patch, design points 3-4. Tests: `a:hover{color:red;text-decoration:underline}` -> the patched
  frame's pixels equal a full rebuild's (fixture, `paint` into a buffer both ways); `a:hover{padding:5px}`
  -> fallback; `.x:hover::after{color:red}` -> fallback; an element with a transition -> fallback.
  **Gate:** Wikipedia Rust `--no-js`, 50 s, pointer moves only over links (this section's recipe), median
  of 3: frame stats `builds.ms / hover_restyles` **<= 20 % of today's 174-180 ms (<= 36 ms)**,
  `hover.patched` >= 80 % of hover_restyles, `input.lat_p95_ms` reported (today median 153-296 ms); the
  same run with JS on reported (after QY53m); nested-dwl screenshot of a hovered link identical to main's.
- **QY53n3 [S]** (after n2) offset-only patch (translate, relative offsets). Gate: YouTube watch, played,
  pointer over the picture and controls: hover builds whose differing rules are all PAINT/OFFSET -> 0
  (today 36 of 58), `hover.patched` reported.
- **QY53n4 [M, later]** subtree re-box for LAYOUT hovers + patch re-application after an in-flight build;
  only if n2/n3 leave hover builds that matter (here 16 of 58 YouTube hover builds, 0 on Wikipedia).
- **QY53n5 [S]** (app.rs `hover_restyle_delay`, frame_stats.rs, PERF.md; after QM1k1n6a and QY53n2)
  patched hovers skip the R2.3/QB3 coalescing; frame stats `hover` {patched, skipped, built} +
  `hover_patch_ms`. Gate: Wikipedia `--no-js`, pointer over links: `input.lat_p95_ms` median <= 50 ms.
- **QY53m2 [S]** (js_dom.rs write paths, coordinate with QY53l2; after QY53m) a value-changing write to an
  attribute no selector can read does not set the restyle dirty flag: the rule list exposes the attribute
  names its selectors test (`[attr]`), plus class/id/style and the attributes the cascade and box build read
  (width, height, hidden, bgcolor, align, dir, lang, type, value, checked, disabled, open, colspan/rowspan,
  src/alt/srcset for replaced content, ...); a write of any other attribute (`title`, `aria-*`/`data-*`
  unless a selector names it) does not restyle. Tests: a `title` write leaves `layout_dirty` false; with
  `[title]{...}` in the sheet it sets it. Gate: Wikipedia JS on, pointer over links: JS builds that change no
  style <= 2 per run (today 33-38, with QY53m's same-value ones).
- **QY53o [S, Opus measure]** Wikipedia with JS on: 35 forced layouts per run (14.5-16.8 s, ~450 ms each)
  only while the pointer moves over links; name the reader (QOBS5c window attribution, or `--perf-report`
  with a synthetic mouseover) and file the fix (likely Page Previews' geometry reads; R43 Finding 0 makes
  each one a 1024x768 layout).

### Not established
Who writes `li style="display: none;"` and the `title` toggles (inferred: Wikipedia's mouseover
extensions); who forces the 35 Wikipedia layouts (QY53o); the split of `resolve_style_for` into cascade vs
`var()`/value computation; whether `top` in YouTube's player hover rules is on relative boxes (QY53n3 must
check at patch time); numbers at a full-size window (the host gave 956x312).
(The writers and the forced-layout callers are now established: QY53o addendum below.)

### QY53o addendum: who forces Wikipedia's layouts while the pointer hovers links (Opus, 2026-11-19, docs only)

**Status: measured live, docs only (the instrumentation was reverted).** Release build of main 140bef3
(V8 + media) plus throwaway instrumentation (env `QY53O_DIAG`, never committed): one line per forced layout
in `GeomCache::refresh` with wall/CPU ms, the QOBS3 phase split (read from the window-mode recorder's
`fl.phase_ns`, which runs whenever `SCRATCHY_FRAME_STATS` is set), `no_change`, the old/new cache key, the
reading native (`__d_rect`/`__d_textrects`), its target element and the WHOLE V8 stack (40 frames, prelude
frames included), plus every geometry-generation bump since the previous layout (`#[track_caller]` on
`geom_bump`/`geom_bump_if`: which write native, which attribute, which element); and one line per
`__d_setattr`/`__d_setq`/`__d_rmq` with name, same-value flag and stack. Own worktree, own wire on a
scratchpad socket (`$!`), nested dwl (tools/nest.sh), DEFAULT UA, `SCRATCHY_MEDIA_AO=null`,
`SCRATCHY_FRAME_STATS`. The host gave the nested window **960x1006** (960x489 in the first run d1); the
page's JS still sees 1024x768 (R43 Finding 0). Load average 3-5 (quiet-ish).
**Input (tools/inject, from t = 12 s, 29 s):** 6 rounds of `move:` over 8 links IN THE ARTICLE BODY
(general-purpose programming language, performance, type safety, concurrency, memory safety, functional
programming, immutability, garbage collector) with `wait:600`. R47's recipe had 5 lead links + Talk/Edit/View
history; the tabs are not Page Previews targets (its selector is `#mw-content-text a[href][title]`), which is
why R47 saw 35 per run and this recipe 50-56 (5/8 of the positions).

**Baseline (frame stats, instrumentation off, b1-b3), pointer phase (t >= 12 s):**
| run | forced n / ms | builds n / ms | page JS ms | lat_p95 median / max ms |
|---|---|---|---|---|
| b1 | 51 / 17 174 | 61 / 10 076 | 18 739 | 514 / 1 063 |
| b2 | 52 / 16 485 | 71 / 10 861 | 18 058 | 493 / 886 |
| b3 | 54 / 15 609 | 83 / 11 461 | 17 082 | 510 / 902 |
| **median** | **52 / 16 485** | 71 / 10 861 | 18 058 | 510 / 902 |
Whole run (incl. load): forced 55-58 / 16.8-19.3 s. Of the 30 s of pointer phase, ~16.5 s are forced layouts.

**Callers (instrumented runs d2, d3; pointer phase; d1 at 960x489 had the same callers, 30 forced):**
| reader (function url:line:col) | read | d2 n / ms | d3 n / ms | `no_change` | what changed the geometry generation before it |
|---|---|---|---|---|---|
| Page Previews dwell handler, `(anon) load.php?modules=ext.cite.referencePreviews\|ext.math.popup\|ext.popups.main:34:41125` (called from the link-hover dispatcher `34:35849`) | `link.getBoundingClientRect()` (`ElementProto.getBoundingClientRect` -> `gViewRect` -> `gRect` -> `__d_rect`); the `offsetWidth`, `offsetHeight`, `getClientRects()` it reads next hit the fresh cache | 38 / 10 716 | 34 / 10 991 | 19 / 29 | **only `title` toggles: 19 / 29**; the rest: the previous preview's popup (`div.mwe-popups` inserted/removed at the end of `<body>`, its `class`/`style="left:..;top:.."`, its links' `href`) |
| scratchy's own IntersectionObserver recheck, `ioEval0 eval:3102` <- `obsFlush` (timer) on `h1#firstHeading` (Vector's sticky-header observer) | `gViewRect` -> `__d_rect` (the QY53e1 deferred recheck: `ioDue` = 4 x last layout CPU, so ~1 s after a geometry-generation bump) | 18 / 5 110 | 16 / 5 165 | 16 / 14 | **only `title` toggles: 16 / 14**; the rest: popup insert/remove/style |
| **total** | | **56 / 15 826** | **50 / 16 156** | 35 / 43 | **`title` only: 35 (10 354 ms) / 43 (13 968 ms) = 63-86 % of the ms** |
During load (t < 12 s) 4 more: IO on `#mw-content-text`/`#firstHeading` and Vector's TOC
`scrollToActiveSection` (`load.php?...skins.vector...:914:902`, via `offsetParent`/`offsetTop` -> `gOffParent`).
No forced layout came from `getComputedStyle` (it uses the style cache, not the geometry cache), from
`__d_textrects`, or from a viewport change (the key's viewport stayed 1024x768).

**The writers (both are Page Previews, `ext.popups.main`, fetched copy of the same version):**
- `title` toggles: its store listener (`me`) runs on every `activeLink` change: the old link gets its title
  back (`r.setAttribute("title", e)`) and the new one `t.setAttribute("title", "")` (to suppress the native
  tooltip). 95-103 `setAttribute('title')` per run in the pointer phase, each VALUE-CHANGING, so each bumps the
  geometry generation (`__d_setq` -> `geom_bump_if(changed)`). No stylesheet on the page tests `title` on an
  `<a>`: the only `[title]` selectors are `abbr[title]` (default.css and the skin CSS), no `attr(title)` (the
  page's `attr()` uses `data-mw-footnote-number`/`data-line`). So every layout after a title-only change
  recomputes a tree whose rects equal the previous one: all 35/43 have `no_change = true`.
- `li style="display: none;"`: the footer "Enable previews" link (`#footer-places` li) that the extension
  appends; its subscriber `34:21894` does `t.style.display = shouldShowFooterLink ? "" : "none"` on EVERY store
  change -> 264-272 writes per run (more than R47's 154-165: more preview targets in this recipe), **all
  same-value, and none bumped the geometry generation** (none appears in any layout's bump list: QY53l's
  same-value check holds). They still set the App's JS dirty flag on every batch: QY53m's case (App builds,
  not forced layouts).
- The popup changes are real DOM changes (a positioned `div.mwe-popups` appended to `<body>`, `left/top`
  writes, `class` flips for the fade-in, the preview link `href`); the hovered link's box does not move
  because of them, but scratchy's geometry cache has one global generation, so the next read lays out the
  whole page.

**Why each one costs 250-330 ms (median wall 255 ms in d2, 325 ms in d3; CPU = wall, nothing waits):** each
is a whole-document layout at 1024x768 (10.6k elements). Median phases d2 / d3 (ms): css_collect 1 / 1,
rule_index 0 / 0, precompute_generated 59 / 75 (23 %), build_children 72 / 91 (28 %), **measure_document
116 / 144 (45 %, even with the content-keyed SHAPE_MEMO warm: nothing in the text changed)**, glyph_walk
7 / 9, rect_walk 1 / 2. R47's ~450 ms were at load average 11-13.

**Proposed items (coordinator files them):**
- **QY53o1 [S]** (P1; js_dom.rs `geom_bump_if` + the predicate QY53m2 needs, style.rs rule list; NOT alongside
  QY53l2 (#351, same write paths); do it together with QY53m2 or right after it, one predicate for both) a
  value-changing write of an attribute that no selector and no `attr()` of the current rule list can read on
  that element does NOT bump the geometry generation (and, via QY53m2, does not restyle). Inert = not
  class/id/style, not an attribute the cascade or box build reads (QY53m2's list: width, height, hidden,
  bgcolor, align, dir, lang, type, value, checked, disabled, open, colspan/rowspan, src/alt/srcset, ...), and
  every compound selector that tests `[name]` has a type selector that is not this element's tag (so
  `abbr[title]` does not make `title` on `<a>` relevant); `attr(name)` anywhere -> relevant. Must be
  invalidated when the rule list changes (css_ver). Tests: `a.setAttribute('title','x')` after a layout ->
  `__d_geom_fresh` stays true and `GEOM_LAYOUTS` does not grow on the next `getBoundingClientRect`; with
  `a[title]{padding:5px}` or `content:attr(title)` it does; `abbr[title]` in the sheet + a write on `<a>` ->
  fresh. **Gate (this recipe, frame stats, pointer phase, median of 3): forced.n <= 22 (today 52) and
  forced.ms <= 6 500 (today 16 485);** the IO recheck count drops with it (it only runs after a bump).
- **QY53o2 [M, Opus design first]** (P3; js_geom.rs) scoped geometry invalidation: writes inside an
  out-of-flow subtree (absolutely/fixed positioned, containing block outside the read target's ancestors)
  do not invalidate the rects of elements outside that subtree; a read of such an element reuses the cached
  rects (the page size/scroll extent must still be right). Covers the 7-21 layouts per run after Page
  Previews' popup changes. Gate (after QY53o1): forced.n <= 5 per run on this recipe. Only if QY53o1 leaves
  more than ~3 s per run.
- **QY53o3 [S, Opus measure]** (P3) where `measure_document` spends 116-144 ms in a forced layout whose text
  did not change (SHAPE_MEMO warm): memo hit rate, per-paragraph buffer rebuilds, box measurement; a perf
  profile with `--perf-report` on a headless replay of this page plus a title write and a
  `getBoundingClientRect` loop. Feeds the general forced-layout cost (QY53 block), worth ~45 % of every
  remaining forced layout.
- The IO rechecks (16-18 per run, 5.1 s) are QY53e2's case (IO on the App's own layout, after Q33b2);
  QY53o1 alone removes 14-16 of them. The same-value footer-li writes are QY53m's case (App builds only).

**QY53o1 result (Sonnet, 2026-11-19, release build of this branch, own wire, nested dwl 960x316; inject size
must be the window size 960 316, not 960x1006; positions: 5 lead-paragraph links + 3 more, 6 rounds, wait:600;
machine load 8-14 because other agents were building, so absolute ms are high).** Pointer phase (t >= 12 s),
before (same code with the check disabled, one run) forced 52 / 10 125 ms; after, 3 runs: 30 / 5 616, 20 / 3 008,
0 / 0 (Page Previews did not engage in that run; builds 25), median **20 / 3 008 ms: gate (n <= 22, ms <= 6 500)
met**. What made it fail first: Wikipedia's `content: counter(...) var(--cite-...)` made the first version treat
every attribute as read; only an `attr(` inside a `var()` content now does. The only `title` readers on the page
are `abbr[title]` / `acronym[title]` (type-pinned), so a `title` write on `<a>` is inert. Remaining layouts are
the popup's real DOM changes (QY53o2).

### QY53o3 addendum: why `measure_document` costs ~125 ms per forced layout with unchanged text (Opus, 2026-11-19, docs only)

**Status: measured headless, docs only (the instrumentation was reverted). Root cause found: the JS
thread's shape memo is NOT warm. `js_geom.rs` `SHAPE_MEMO_CAP = 4096` (QP2, d5fe517) is checked at the
START of every `GeomCache::refresh`, and ONE Wikipedia Rust layout puts 7 508 entries in it, so the memo
is cleared before every forced layout and every layout reshapes the whole page (0 % cross-layout hits).**

**Recipe (headless, reproducible without a window):** profiling build of main 732438c
(`cargo build --profile profiling`, `-C force-frame-pointers=yes`, own `CARGO_TARGET_DIR`), own `wire`
on a scratchpad socket (`$!`). Page: a saved copy of https://en.wikipedia.org/wiki/Rust_(programming_language)
(1.06 MB) with its `<script>`s removed, `client-nojs` -> `client-js`, a `<base href>` to the live URL
(so the two `load.php` stylesheets come through wire) and one inline script at the end of `<body>`:
`t = document.querySelectorAll('#mw-content-text a[href][title]')[5]` then 12 times
`t.setAttribute('title', i % 2 ? '' : 'Memory safety'); t.getBoundingClientRect()`. Run:
`scratchy --perf-report --socket <own wire> --budget-ms 20000 --seconds 1 rust-loop.html`. Result: 12
forced layouts (`no_change` 11 of 12), 1024x768, ~9.7k elements; schema-2 `forced_layout.phases` per layout:
precompute_generated ~48, build_children ~54, **measure_document 130-136** (median of 3: 132.3), glyph_walk
~6, rect_walk ~1 ms, i.e. the live QY53o numbers (116-144) on a load average of 3-7. Throwaway
instrumentation (never committed): per refresh, the memo size before the cap check, `memo_get`
hits/misses, `shape_spans` calls/ms/bytes, `memo_key` calls per kind and ms, measure loop ms, `drop(tree)`
ms; plus an env override of the cap to test the fix. `perf record -e cycles:u -F 1999 -g` on a 40-layout
variant of the same page.

**Per forced layout (instrumented, recipe as above):**
| | baseline (cap 4096) | cap raised (1 000 000) |
|---|---|---|
| memo size before the cap check | 7 508 -> **cleared** | 7 508, kept |
| `memo_get` hits / misses | 5 741 / 7 508 (hits are only repeats WITHIN the layout: intrinsic-size trials + the real pass) | **13 249 / 0** |
| `shape_spans` calls / ms / text bytes | 7 508 / 128-174 / 98 637 | 0 / 0 / 0 |
| `memo_key` calls (kind 0 paragraph / 1 word / 2 mixed-inline chunk) | 960 / 3 432 / 9 337 | 480 / 3 432 / 9 337 |
| measure loop, warm layouts, median of 3 runs | **124-128 ms** | **19.0-19.7 ms** |
| `phases.measure_document` per layout (total / 12, includes the cold first) | 129.8 / 132.3 / 135.6 | 34.8 / 35.2 / 36.8 |
| whole forced layout, warm, median | 262-270 ms | 160-161 ms |
| `forced_layout.total_ms` / count (incl. the cold first) | 267 / 272 / 285 | 176 / 176 / 179 |
| `memory.caches.shape_memo` after the run | 7 508 entries, ~5.1 MB | the same (the baseline refills it every layout) |
| RSS | 233-243 MB | 236-237 MB |
The keys are stable across layouts: with the cap raised every lookup after the first layout hits (the
`title` toggle and layout repetition change no `SpanStyle`, width, `node`/`tnode` or text). The
first (cold) layout is unchanged (~200 ms measure). `drop(tree)` inside the measure phase is ~1 ms.

**perf, baseline (8 419 samples under `measure_document`):** 83 % inside `shape_spans` (cosmic-text
`ShapeLine::build` 69 %, harfrust shaping 20 %, font fallback iteration 17 %, `Buffer::new` 8 %); by call
site 42 % mixed-inline chunk shaping (memo kind 2), 31 % plain paragraphs (kind 0), 16 % per-word
fallback `shape_or_resume_word` (kind 1), 7 % mixed-inline trials inside `natural_width_impl`
(tables, floats, flex intrinsic sizes); `natural_width_impl` 15 % and `table_column_extents` 6 % inclusive.

**perf, cap raised (the ~19 ms that remain per warm layout; stacks without `shape_spans`):**
`measure_mixed_inline` 77 % inclusive (it is where most text is), `natural_width_impl` 36 % (intrinsic
min/max trial passes re-run every chunk lookup), **`memo_key` 21 %** (SipHash `DefaultHasher` over every
text and `SpanStyle`, ~3.5 ms per layout for 13 249 keys), `__memmove` 14 % (mostly the
`InlineItem::Text(text.to_string(), *span)` synth item built only to compute a kind-1/kind-2 key, and
`Measured` vec growth), `SHAPE_MEMO`/`ShapeCache` hash lookups and `Arc` clones ~15 %,
`word_widths_from_buffer` 9 %, `Instant::now()` deadline checks per chunk 5 %, `hoist_negative_z` 3 %,
`in_flow_bottom` <1 %. No float/table/grid pass dominates on its own; the per-box walk itself is small.

**What this means for the forced-layout budget:** with the memo kept, a warm Wikipedia forced layout is
~160 ms, of which `precompute_generated` + `build_children` (the App-side style/box work R47 described)
are ~85 %; `measure_document` drops from the largest phase to ~12 %. Combined with QY53o1 (fewer layouts)
the remaining layouts each cost ~105 ms less.

**Not established:** the App's shaping worker memo (app.rs `spawn_shaping_worker`) has no size cap at
all (cleared only when `reuse_shaping` is false); whether it grows without bound on a long-lived page
that keeps changing text (e.g. YouTube comments, live counters) was not measured. Sharing the worker's
memo with the JS thread would not help the first forced layout: the JS thread lays out at 1024x768
(R43 Finding 0) while the window is another width, so the keys differ.

**Proposed items (coordinator files them):**
- **QY53o3a [S]** (P1 within the QY53 block; js_geom.rs `refresh` + layout.rs `SHAPE_MEMO`; not alongside
  QY53l2 if it touches js_geom.rs) keep the JS thread's shape memo across forced layouts by replacing the
  fixed 4 096-entry clear-at-start with a per-layout sweep: tag each memo entry with the refresh
  generation of its last `memo_get`/`memo_put` and, AFTER each refresh, drop the entries the layout did
  not use (the memo then holds exactly one layout's working set; a viewport change still clears it). Keep
  a byte/entry safety cap that is only applied after the sweep (e.g. > 64k entries -> clear). Test (in
  `js_geom`/layout tests): a document with more than 4 096 distinct text runs, two refreshes after a
  `title` write -> the second shapes nothing (test counter of `shape_spans` calls) and the memo size is
  equal after both; a removed paragraph's entry is gone after the next refresh.
  **Gate (this recipe, `--perf-report`, median of 3): `forced_layout.phases.measure_document` / count
  <= 45 ms (today 130-136); warm-layout measure <= 25 ms; `memory.caches.shape_memo` bytes not above
  today's ~5.1 MB; `forced_layout.total_ms` / count at least 80 ms below main's, measured interleaved
  (instrumented runs here: 267-285 -> 176-179 ms).** Live check:
  the QY53o recipe's forced.ms per run drops by ~100 ms x forced.n.
- **QY53o3b [S]** (P3; layout.rs, after QY53o3a) cheaper memo keys on the hit path: a fast non-SipHash
  hasher (FxHash-style, already-present dependency or a 20-line local one) for `memo_key`, compute kind
  1/2 keys from `(&str, &SpanStyle)` directly instead of allocating `InlineItem::Text(text.to_string(), ..)`
  per chunk/word, and one `Instant::now()` deadline check per paragraph rather than per chunk. Test: equal
  keys for equal inputs, different for a changed `SpanStyle` field/width/text (existing memo tests keep
  passing). **Gate (same recipe, median of 3): warm-layout `measure_document` <= 14 ms after QY53o3a
  (~19 ms with QY53o3a alone).**
- (Optional, M, only if QY53o3a+b leave measure > 15 ms) cache `natural_width_impl` min/max results per
  box per layout generation keyed like the memo, so intrinsic trials (36 % of the warm remainder) are not
  redone for unchanged subtrees.

## R46: QM1k1n6 / QM1k-mouse: where a "pointer moving over YouTube" second goes. Moving the pointer is NOT what freezes the video; YouTube's player-controls JS does, when the pointer crosses its controls: each `mouseover`/`mouseout` on a `ytp-*` control costs 40-180 ms of page JS (~2 forced layouts of ~60 ms), the timers it schedules then run back to back in ONE `run_due` turn (up to 100 timers, no time limit: 1.2-2.5 s), and every queued `CursorMoved` is processed one by one. Two main-thread stalls under everything were scratchy's own and are FIXED here: the event loop spun at 20-40k turns/s on any page with JS (an `EventLoopProxy` clone dropped every turn wakes the loop), and mpv's software render SLEPT on the main thread until each frame's display time (~31 ms per frame = 93-100 % of every playing second) (Opus, 2026-11-19) — OPEN: QM1k1n6a-c, QOBS5c (proposed in the PR); FIXED: the spin and the render block (this PR)

**Status: measured live, two S fixes implemented (app.rs `pump_net`, media_mpv.rs `sw_render_params`),
design + items for the rest.** Release builds of main 7bf6875-era code (V8 + media), own worktree, own
wire on a scratchpad socket, nested dwl (tools/nest.sh, window 960x1006), tools/inject, DEFAULT user
agent, `SCRATCHY_MEDIA_AO=null`, `SCRATCHY_FRAME_STATS` (QOBS5). URL
https://www.youtube.com/watch?v=G1AeBgalFZk. Throwaway instrumentation (never committed): per-second
tallies of every `window_event`/`user_event` kind with wall ms, `js_dispatch` by event type (+ the
target's tag/class when >= 20 ms), `run_due` calls and ms, `video_tick` split (mpv render / buffer /
present) and why it fell back to a full redraw, which source requested a restyle, `StartCause` of
each turn and which `ControlFlow` deadline was chosen.

### Recipe
1. `tools/nest.sh <dir>`; own `wire --socket <dir>/wire.sock` (`$!`).
2. `SCRATCHY_FRAME_STATS=<dir>/frames.jsonl SCRATCHY_MEDIA_AO=null scratchy --socket <dir>/wire.sock <URL>`.
3. After 16 s: `inject 960 1006 move:346,261 click` (the big play button), wait 10 s (playback
   starts), then phases of `move:x,y wait:16` (60 Hz) for 8-10 s each: **video** = a circle inside the
   picture; **page** = a zig-zag over the whole window (player, title, buttons, recommendations);
   **bar** = back and forth along the controls bar (y 380-400: progress bar, play, volume, size
   buttons). Park the pointer at (10,990) between phases. YouTube stops ~30 s after the click
   (R45 attestation stop), so at most ~20 s of playing time per run.
4. Watch `grim` with a timeout: once (17:15-17:19) the nested dwl stopped presenting (grim hung,
   0 redraws); restarting the nested dwl fixed it, that run was redone.

### The user's own session (coordinator, real window, main ~17:10 incl. QY53f1, real mouse)
Smooth seconds (t 21-24, 30, 38): js 47-260 ms, forced 0-3, video 28-34 painted/s, even with
99-127 pointer moves/s. Freeze seconds: t 18.2 js 1901 ms, forced 25/1561 ms, video 1/1, 32 moves
pending; t 35.0 (dt 3343 ms) js 3244, forced 45/2896, video 1/1, lat_max 2775 ms, 133 moves; t 37.4
(dt 2407) js 2302, forced 32/2095, video 1/2, lat_max 2019 ms, 266 moves. Builds 57-60 ms (up to 12/s),
paint <= 14 ms. The same shape as the nested runs below.

### Where the second goes (main before this PR; instrumented runs r0, r1)
| situation | what the main thread did per wall second |
|---|---|
| page loaded, video not started (or stopped) | **30-37k event-loop turns/s**, each `run_due` = one V8 eval (~15 us): 545 ms CPU/s, frame stats `js_ms` 130-142; StartCause `WaitCancelled` 20-37k/s with NO event delivered; a 1-line page with one `setInterval`: 10 s CPU in 10 s. Cause: `App::pump_net` (called from `js_sync` every turn) did `self.proxy.clone()`; dropping a calloop channel `Sender` pings the loop (calloop 0.13 `impl Drop for Sender`), so every turn scheduled the next one |
| playing, pointer still | `u:video` 30/s = **970-1000 ms**: `mpv_render_context_render` took **31 ms per frame** (r1 t 29.1: 977 ms for 31 frames); buffer + present 1 ms. mpv's `MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME` defaults to 1 = sleep until the frame's display time, on the main thread. Video 30 rendered/30 painted, but every other task (page JS, input, net results, redraws) waited behind the sleeps |
| playing, pointer over the PICTURE (r0 move phase) | 62 `CursorMoved`/s cost 0.3-0.5 ms in total, 0-1 hit-node changes/s, 0-1 hover restyles; js 15-30 ms; video 30/30. **No freeze.** |
| playing, pointer over the page (r1 page phase) | 5-24 hit-node changes/s; most `mouseover`/`mouseout` < 1 ms, but those on player controls 100-480 ms per second; video 14-31 painted/s; lat_p95 33-473 ms |
| playing, pointer along the CONTROLS BAR (r1 bar phase) | 18 hit-node changes in one second = `mouseover` **2507 ms** (+ `mouseout` 4.5 ms), forced 39 / 2461 ms, video 1/1, 26 moves pending; then NO event-loop turn at all for ~10 s (the frame-stats roll stopped: the queued moves were still being processed one by one): the user's freeze |
| click -> first frame (start phase) | **one turn per second**: a single `run_due` ran 1.0-2.6 s of timers (forced 13-50/s, ~60-65 ms each), video painted 1-2/s; first frame 9.6 s (r0) and 11.8 s (r1) after the click (R45: "picture ~6 s late") |

Slow pointer events by target (r5, >= 20 ms, page and bar phases): `mouseover`/`mouseout` on
`div.ytp-progress-bar-padding` (in `ytp-chapter-hover-container`) 96-134 ms, `button.ytp-size-button`
176 ms, `button.ytp-play-button` 85 ms, `button.ytp-volume-icon` 43 ms, `div.ytp-volume-slider-handle`
43 ms, `div.ytp-progress-linear-live-buffer` 96 ms. Each is ~2 forced layouts of ~60 ms; the follow-up
timers (r5 t 31.1: 4 turns, `run_due` 1182 ms, forced 27/1060 ms with only 9 moves) carry most of a
burst. Which JS function forces them is NOT measured in the window (`by_caller`/`longest` exist only in
`--perf-report`, which runs headless); R43 found 88 % of YouTube's forced layouts in `getPlayerSize`,
and QY53l (4608cd4, merged during this run) cut those by 28 %.

Other facts: scratchy dispatches NO `mousemove`/`pointermove` to pages at all (Q33d2 open): pages
only see `mouseover`/`mouseout` on a change of the hit node. Each `CursorMoved` runs synchronously
in `window_event` (hit test, `controls_pointer`, `select_popup_hover`, `update_hover` -> JS, cursor
icon); winit delivers ALL queued motions before the turn's redraw and `about_to_wait`. `:hover`
restyles are already debounced (R2.3/QB3): 0-7/s, 30-60 ms builds each. `video_tick` falls back to a
full redraw (and its 55-60 ms tree build) whenever `layout_dirty` is set (`vt_fail_dirty` 1-9/s in
burst seconds). winit order per turn: user events (VideoFrame, JsNet, ShapeResult...) -> window events
(all queued moves) -> RedrawRequested (if the frame callback came) -> AboutToWait (`run_due`).

### Fixed in this PR (before = main binary, after = this branch; same recipe, same session)
- `pump_net` borrows the proxy (no clone per turn). Trivial page: 10 s CPU in 10 s -> 0 s. YouTube
  loaded, idle: frame stats `js_ms` 132-142 -> 8-12 per second; turns/s 30-37k -> 2 (idle) / 160-300
  (playing, 30 video wakes + timers).
- `sw_render_params` passes `BLOCK_FOR_TARGET_TIME = 0`: `u:video` 970-1000 -> 17-22 ms per second
  for the same 30 frames (0.6 ms per frame); rendered 30.0/s and painted 37/s (with full redraws)
  before and after, so mpv's pacing is unchanged. Whole run CPU 31 s -> 25 s (57 s wall).
- First frame after the click: 4.4 s before / 2.2 s after in the same session (r6: 4.8 s); earlier
  main runs 9.6 and 11.8 s. Too few runs and too much YouTube variance to claim a number; not a gate.
- Pointer bursts are NOT fixed by this: after the fix (r6) the page phase still had seconds with js
  1890 ms (`run_due` 1522 ms, `mouseover`/`mouseout` 382 ms) and video 9 painted; the bar phase 12
  painted/s in its worst second. In the clean before/after pair YouTube produced almost no hover work
  (hit-node changes 0-1/s in both), so they show no pointer difference either way: the bursts depend
  on the player's state (controls shown or not), which the recipe does not control.

### Design for the rest (QM1k1n6a-c; numbers above)
(a) **Pointer motion: one processing per turn.** `CursorMoved` only stores the newest position (and
the stats input stamp); the hit test, hover/`mouseover`/`mouseout`, controls drag and cursor icon run
once per turn in `about_to_wait` (before `run_due`), and are flushed first by any button, wheel or key
event, so those still see the right target. Browsers do the same (one hit test per frame). Saves: r1
bar second 26 queued moves -> 18 dispatches = 2.5 s; with coalescing at most one hit-node change per
turn, and the video frame (a user event) is handled at the start of every turn, so video gets a frame
between two controls dispatches instead of after all of them. A `mousemove` dispatch (Q33d2) must use
the same once-per-turn point when it is added.
(b) **Frame budget for timers.** `run_due` stops after a time slice (16 ms while a `<video>` is
playing or input is pending, 50 ms otherwise; at least one timer per turn) and sets `ControlFlow` to
"now" if more are due, so a redraw/video frame/input gets a turn at least every slice + one callback.
Saves: start phase 1 turn/s -> >= 15 turns/s; the post-hover timer bursts (1.2-2.5 s in one turn) are
split into ~60 ms pieces (one forced layout each), so video can be painted ~10-15/s through a burst
instead of 1/s. It does not shorten one long callback (a 180 ms `mouseover`): that is QY53 work.
(c) **Composite video without a relayout.** `video_tick` may present into the kept page while
`layout_dirty` is set, as long as the kept page matches the window size (the screen still shows that
page: R2.1 `hold_old_frame` keeps it until the new layout is shaped). Saves the 55-60 ms build per
fallback (`vt_fail_dirty` 1-9/s in bursts) and keeps frames flowing during restyles.
(d) **Attribution in the window** (QOBS5c): the per-second line names the top forced-layout callers
(`forced.by_caller`, top 3, ms) and the longest callback's source, so the next burst can be blamed on
a function without headless reruns.

### QM1g2a (2026-11-19): mpv frames rendered off the main thread
The "33-39 ms per frame" in QM1g2 (a) predates this section's `BLOCK_FOR_TARGET_TIME = 0` fix; the
main-thread render cost before QM1g2a was 3-7 ms per frame. QM1g2a moves the render context to a
worker thread (`media_render.rs`) and adds `video.tick_ms`/`video.fetch_ms` (main-thread ms per
second in `App::video_tick` / in getting the frames) to the frame stats. Before = this branch's first
commit (the stats only), after = the worker. Nested dwl 960x1006, own wire on its own socket,
`SCRATCHY_MEDIA_AO=null`, tools/inject: 6 s still, 10 s of 60 Hz moves in a circle over the picture,
10 s zig-zag over the page. Each run under `flock /tmp/scratchy-perf.lock`, before/after interleaved,
load average 36-70 (other agents building). Numbers are over the seconds with a rendered frame.

| run | main ms per frame (tick / of it fetch) | frames taken | painted/s median (min) | input lat p95 median / worst max (ms) |
|---|---|---|---|---|
| local 1280x720 30 fps H.264 -> 640x360, before 1/2/3 | 4.09/2.88, 4.54/3.09, 5.86/4.13 | 1143, 1131, 1152 | 30 (30, 23, 12) | 34.4/77.6, 34.2/54.9, 36.4/74.1 |
| local, after 1/2/3 | 0.96/0.00, 1.18/0.00, 1.08/0.00 | 1130, 1128, 1166 | 30 (22, 20, 27) | 33.1/42.7, 33.4/51.9, 32.2/42.5 |
| YouTube G1AeBgalFZk (click play, then the same phases), before 1/2 | 8.48/6.09, 9.98/6.88 | 116, 91 | 5 (2), 6 (3) | 419/984, 365/867 |
| YouTube, after 1/2 | 2.40/0.00, 2.15/0.00 | 200, 192 | 6 (4), 7 (3) | 284/548, 259/1984 |

What is left on the main thread per frame (~1-2.4 ms): copying the frame through the box mask into
`page_buf` and the softbuffer buffer, the controls bar and the damage present. Local video fps is
mpv's 30 either way; YouTube stays at 5-7 painted/s in both builds because its page JS (js_ms 220-310
per second here) holds the main thread, which is QY53/QYTC1 work, not rendering. The YouTube latency
spread is page-JS noise (one after run had a 1984 ms second); the local runs show no input-latency
change beyond the smaller worst case.

QM1g2a1 (2026-11-20): frame buffers are pooled (`FramePool`, media_render.rs). Measured on a quiet machine the whole main-thread `video_tick` is ~0.3 ms/frame for 640x360 in a 960x489 window (page_buf copy 0.13, present-buffer copy 0.13, present 0.01, everything else under 0.01); the 1-2.4 ms in the table above was machine load. Interleaved before/after, 3 pairs (tick ms/frame, load 1-28): 0.43/0.34/0.91 vs 0.28/0.44/0.91, no measurable difference; the pool removes the per-frame 0.9 MB zeroed allocation on the worker and the free on the main thread. YouTube not re-measured (its main thread is page-JS bound).

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

## R43: Where forced layout goes (QY53): on the YouTube watch page 88 % of it is the player's `getPlayerSize` reading `#movie_player.clientWidth`, 80 % of those layouts change no box at all, and 99.5 % of each one is STYLE and BOX BUILD (layout proper 0.4 %); half of that is one O(elements x rules) scan for `::first-line`/`::first-letter` rules. On Wikipedia it is scratchy's OWN IntersectionObserver evaluation (Opus investigator QY53, 2026-11-19) — OPEN: QY53a-QY53h (a-d DONE; QY53e designed in section 7, PARTIAL, split into QY53e1-e3); QY53l PARTIAL (section 10: getPlayerSize layouts -28 %, proposed QY53l2/l3); QY53e3 measured in section 11; QY53l2 PARTIAL (section 13: getPlayerSize layouts a further -8 %, the safe rule's ceiling; the rest are writes to VISIBLE elements)

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

### 8. QY53f re-measured after QY53g/h/e1: incremental style is NOT worth an L item now (Opus, 2026-11-19, docs only)
**Setup.** Main d889703 (QY53g a86d821, QY53h 4749910, QY53e1 c5098fa in; QY53i and QSTK1 not yet).
`tools/profile.sh` (QOBS1) with an own `PROF_TARGET` and an own wire on a scratchpad socket, DEFAULT user
agent, headless `--perf-report --budget-ms 20000`: YouTube watch G1AeBgalFZk 60 s, Wikipedia Rust 30 s. No
challenge or consent wall. Then the same `profiling` build with **throwaway instrumentation (reverted with
`git checkout`, never committed)**: one stderr line per forced layout with the thread-CPU phases of
`build_layout_tree_with_resources` (CSS collect + rules cache/parse + index, `precompute_generated`,
`build_children`, the rest), `measure_document`, glyph and rect walks; matching runs/ms and memo hits;
top-level `resolve_style_for` count/ms; the rect map compared with the previous forced layout (changed /
added / removed); and a StyleCache-style fingerprint of every element (`ctx_hash` chained down from the root,
as QY52's `StyleCache::style_of` does) compared with the previous layout, i.e. how many elements an
incremental style pass could have reused. Two instrumented runs per page; the second ran at load average
~14 (other agents' builds), so its absolute ms are ~1.6-2.5x higher; the SHARES agree between runs.
Fixtures: the QY53 "big-css" fixture generated by a script (20 000 `.c<i> .d<i>` rules, 200 `::before`,
1 000 NESTED divs, 50 x (setAttribute, offsetWidth); `ulimit -s 262144`, QSTK1) and a FLAT variant (same
CSS, 50 `<section>`s x 20 divs, same script).

**Totals (forced_layout from `--perf-report`):**
| page | run | forced layouts | total_ms | mean | max | R43 (b8d865f) |
|---|---|---|---|---|---|---|
| YouTube watch 60 s | perf profile / instr. 1 / instr. 2 (loaded) | 129 / 136 / 133 | 4 612 / 4 990 / 8 068 | 36 / 37 / 61 | 306 / 293 / ~420 | 170-313 layouts, 56-121 s |
| Wikipedia Rust 30 s | perf profile / instr. 1 / instr. 2 (loaded) | 4 / 4 / 4 | 1 702 / 1 330 / 3 271 | 425 / 333 / 818 | 513 / 408 / - | 8 layouts, 18.8-19.3 s |

YouTube forced layout is now ~5 s per 60 s (raw callback CPU 10.8 s, so forced layout is still ~half of the
page's JS-side CPU, but only ~8 % of one core); it was 101 s in R43 run A.

**Where a forced layout goes now (instrumented run 1; run 2 same shares within 2 points):**
| phase | YouTube (136 layouts) | Wikipedia (4) | big-css nested (50) | flat (50) |
|---|---|---|---|---|
| CSS collect + rules cache + parse + index | **1 150 ms (23 %)**, of which 4 layouts that RE-PARSE the 4.4 MB stylesheet = 1 018 ms | 42 (3 %) | 185 (14 %, the first layout) | 180 (19 %) |
| `precompute_generated` (= the matching; memo shared with the build) | 900 (18 %) | 275 (21 %) | 39 (3 %) | 48 (5 %) |
| `build_children` (cascade + box build) | 2 737 (55 %) | 324 (24 %) | 940 (73 %) | 518 (53 %) |
| `measure_document` (layout + shaping) | 167 (**3 %**) | 640 (**48 %**) | 102 (8 %) | 145 (15 %) |
| glyph + rect walk, rest | 35 (1 %) | 51 (4 %) | 21 | 85 |

perf (profile run, share of `GeomCache::refresh` samples), YouTube: `resolve_style_core` 47 %, of it
**`compute_custom_props` 29 %** (QY53i's target), `parse_stylesheet` 13 %, `matching_indices_for` 15 %,
`memcmp` 7 % (the 4.4 MB CSS string compared on every build), malloc/free/memcpy 32 % (spread over all of
these), `measure_document` 2.6 %. Wikipedia: `measure_document` 43 % (shaping 38 %), matching 18.5 %,
`precompute_generated` 22 %, `build_children` 23 %. The four slowest YouTube forced layouts (249-293 ms
each, i.e. `max_ms`) are ENTIRELY the re-parse (css phase 241-265 ms): YouTube inserts a `<style>`
element, the concatenated CSS text changes, and `RULES_CACHE` parses all 22 564 rules again.

**No-op layouts and reusable styles:**
| | YouTube run 1 / run 2 | Wikipedia | big-css nested | flat |
|---|---|---|---|---|
| forced layouts (after the first) that moved, added or removed NO element box | **73/135 (54 %) = 1.5 s (30 % of forced CPU)** / 74/132 (56 %) | 0/3 | 49/49 | 49/49 |
| elements whose StyleCache chain fingerprint is unchanged since the previous forced layout ("clean") | **84.0 % / 83.3 %** | 63.9 % | 47.9 % | 99.9 % |
| ... if an ancestor's `style` attribute did not dirty its descendants (inheritance cut-off) | 85.2 % / 85.0 % | 63.9 % | 47.9 % | 99.9 % |
| cost of computing those fingerprints naively (per element `ctx_hash`, O(siblings), SipHash) | **1 020 ms / 1 879 ms = 20-23 % of forced CPU** | 82 ms (6 %) | 22 ms | 68 ms |
| rules with `:has()` / with sibling combinators or `:nth-*(of)` | 0 / 88 | 0 / 90 | 0 / 0 | 0 / 0 |

The 80 % no-op rate of R43 is down to 54-56 % because QY53c (no-op/detached writes no longer bump the
geometry generation) removed the cheapest ones.

**Verdict.** Incremental style (keep `ComputedStyle`s per element across forced layouts, R43 item 7) is
**no longer worth an L item**. Upper bound on YouTube: style (precompute + `build_children`) is 3.6 s per
60 s; 84 % reusable -> at most ~3.0 s per 60 s, minus the fingerprint check (a naive one costs 1.0-1.9 s,
so a real implementation must hash incrementally, see below). QY53i alone removes most of
`compute_custom_props` (29 % of a forced layout = ~1.4 s), leaving ~1.8 s per 60 s (~3 % of one core) for
incremental style. Wikipedia: at most 0.4 s per 30 s (its layouts are half shaping). Compare: the CSS
re-parse alone is 1.0-1.7 s per 60 s on YouTube, sets every `max_ms`, and is an S/M fix. What is left of
forced layout on YouTube (~5 s per 60 s, max ~0.3 s) is no longer what starves the main thread (R42's
"busy 100 %"); QOBS4's named slow callbacks and the window (QOBS5, QY53e3) are the better next look.

**Cheaper alternatives, ranked (proposed items; the coordinator files them):**
1. Incremental stylesheet parse in the layout build (S/M): `RULES_CACHE` is keyed on the concatenated CSS
   text (4.4 MB on YouTube, built and `memcmp`ed on every build) and re-parses ALL of it when one `<style>`
   is added. Key it on pieces instead (each `<style>` element's effective text, `css_ver()` for the external
   and adopted CSS), the way QY52's `StyleCache::refresh`/`splice` already does for `getComputedStyle`:
   parse only the changed piece and splice it into the rule list; full parse when `@layer` numbering needs
   it (StyleCache's `concat` rule). Then rebuild `RuleDerived` (index, hover keys, first-line flags) as
   today. Ideally one shared parsed rule list for both the layout build and `StyleCache`.
2. QY53i (merged 4946d83 while this was written, after these runs): `compute_custom_props` was the largest
   single item inside a YouTube forced layout here (29 %); its PR measured it at 0.9 % of all CPU after.
3. Optional, after 1 and QY53i and only if QPROF1 still shows matching on top: a cross-build match memo
   (the cheap half of incremental style, design steps 1-2 below).

Not proposed now: reusing `ComputedStyle` or box subtrees across builds (design steps 3-4), and
incremental LAYOUT (YouTube `measure_document` 3 %; Wikipedia's 48 % is 4 layouts per 30 s).

**Design, for when style is on top again (QPROF1 decides; not to be built now).**
1. *Unit of reuse = the match list first, then the style.* Make `MATCH_MEMO` (QY53d) persistent across
   builds instead of dropping it after each one: it already maps (node pointer, pseudo slot, rules pointer +
   len, viewport width) to rule indices. Clear it wholesale when the rules `Rc` changes (any stylesheet
   change = `RULES_CACHE` miss, which also covers `css_ver()`), on navigation (new document root, as
   `StyleCache::refresh` does with `root_ptr`) and past a size cap. Entries not touched by a build are
   swept at its end (no leak, no stale pointer kept past one build).
2. *Validation by fingerprint, computed top-down in the build itself.* Each entry stores the element's
   chained selector-context fingerprint (QY52's `ctx_hash` chained from the root: tag, all attributes,
   `:empty`-ness, sibling tags, earlier siblings' attributes when sibling rules exist, plus the root key
   `vw` and `interaction_hash()`). A hit needs the stored fingerprint to equal the current one, so a node
   pointer reused by a new node is harmless (same fingerprint = same match result). The measured lesson:
   computed per element the naive way this is 20 % of a YouTube forced layout, so the build must pass the
   parent's fingerprint down (it already walks in document order, like `AncestorBloom`), hash each
   parent's child list ONCE (one tag-list hash, prefix hashes of the earlier siblings' attributes), and use
   a cheap hasher (FxHash-style, QY53k) instead of SipHash. Alternative: per-node dirty bits set by the
   DOM write paths; rejected for now because writes come from many places (JS DOM, parser streaming, App,
   form state, custom-element upgrade) and a missed one is a silent wrong style, while a fingerprint is
   self-validating.
   What the fingerprint must also cover: `:hover/:active/:focus/:focus-within/:target/:focus-visible`
   (the global `interaction_hash()`, as QY52; a hover change re-validates the whole page, acceptable since
   hover restyles go through the App, not forced layouts); live form state `:checked/:indeterminate/
   :disabled` (StyleCache's `formish` rule: such elements and their neighbours also mix in `wgen`);
   `:has()` (0 rules on YouTube/Wikipedia today; StyleCache then mixes `wgen` into EVERY element, i.e. no
   reuse at all; better: store per entry whether a `:has` rule was among its candidates and re-test only
   those rules on a hit); shadow trees (the chain must follow the parent the matcher uses: host / slot
   assignment, QSD); `:lang` (attribute chain, covered). Media queries: `vw` is in the key; a viewport
   change clears everything.
3. *ComputedStyle reuse (the expensive half).* Reuse an element's `ComputedStyle` when its match entry hit
   AND its parent's style was reused (by induction its `InheritedStyle`, which embeds the parent's whole
   `ComputedStyle`, is then identical) AND it has no animation (`animations.is_some()`: evaluated every
   build, as StyleCache keys those on `wgen`) AND its inline `style` is unchanged (it is in the
   fingerprint). Viewport-height units: clear when `vh` changes (`VIEWPORT_H_READ` only says some style
   read it). The "inheritance cut-off" (an ancestor's `style` write does not dirty descendants unless its
   inherited properties changed) needs `PartialEq` on `ComputedStyle`; measured gain over plain induction:
   1-2 points on YouTube, 0 elsewhere: not worth it. Counters/quotes/`attr()` (`GENERATED_TEXT`) are
   document-order state: keep `precompute_generated`'s counter walk, only its matching is memoised.
4. *Box subtrees:* not reused. `build_children` builds boxes from the styles plus inherited decoration,
   `href`, anonymous table/inline wrapping and generated text that depend on neighbours; with matching and
   cascade cached, what remains of box build was not separable in these profiles and is not worth the
   invalidation risk.

**Dependencies.** QY53i (4946d83) kept the leaked `&'static CustomProps` (memo on the hit path; the arena
was left out as QY53i2): `ComputedStyle` stays `Copy`, so a cross-build style cache (step 3) can store it
as is. If QY53i2 later frees the tables on navigation, that cache must be cleared at the same point and
hold whatever handle type QY53i2 introduces; the match memo (steps 1-2) holds no custom properties and is
unaffected. QSTK1: the design only assumes the build still
visits elements in document order on one thread (the persistent memo and `AncestorBloom` are thread-locals
keyed by `Rc` pointers, which cannot cross threads); an iterative traversal or a bigger stack does not
change it, and a depth cap means capped-away subtrees simply get no entries.

### Not established
Whether the App's own (main-thread, per-turn) relayout could serve geometry reads or IO directly
(headless `--perf-report` has no App layout, so the duplication with the window's layout was not
measured); the live-window effect on R42's playback timeline (no nested-dwl run); YouTube results
and home pages (R37/R38 numbers not re-taken); why YouTube's rule index yields 418 candidates (the
size of the universal bucket was not logged); whether `getPlayerSize`'s preceding writes are the
page's own or our prelude's (e.g. `styleSet` rewriting the whole `style` attribute with the same
text; ANSWERED in section 10: mostly our `styleSet` reordering); correctness of FIX4 beyond the measured runs (no tests); whether QY52's StyleCache could be
reused by the box build (item 7).

### 9. QY53k: SipHash and `StyleCache::style_of` re-measured: hashing is 3.4 % of big-css, an Fx hasher gains nothing measurable (Sonnet, 2026-11-19, docs only)

Fixture: the R43 big-css shape rebuilt as a standalone page (20 000 `.c<i> .d<i>` rules, 200 `::before`, 1 000
nested divs with a leaf `span.d1`, 50 x `setAttribute` on the outermost div + `getComputedStyle(leaf).color` +
`offsetWidth`), `--perf-report --budget-ms 20000 --seconds 3`, profiling-profile binary of main 0d5f402, perf
`cycles:u -F 999 -g`. Share of all samples (1 677): SipHash (any `hash::sip` frame) **3.4 %**: MATCH_MEMO
lookups/inserts (key `(usize, u8, usize, usize, u32)`) about half, the rest is `StyleCache::map`
(`(usize, u8)`), `RuleIndex::candidates` (page-keyed `String`s), `AncestorBloom`/`atom_hash`, layout's
`memo_key`, and `ctx_hash` itself (0.4 %: it only hashes the chain's attributes, and attribute values are short).
`StyleCache::style_of` inclusive 15.7 %, of which the first-call stylesheet parse/rebuild is ~7 % and the rest is
the 1 000-level cascade (`resolve_style_core`, inherent: the `setAttribute` on the outermost div changes every
descendant's chained fingerprint, so each `getComputedStyle` re-cascades the whole chain by design).

Tried: an in-crate `FxHasher` (rotate/xor/multiply, 8 bytes a step) for `StyleCache::map`, `MATCH_MEMO` and
`ctx_hash`/`hash_el`/`mix`. big-css `forced_layout.total_ms`, 5 interleaved runs (load ~6), main 1235 / 1115 /
1141 / 1180 / 1131 (median 1141), Fx 1188 / 1109 / 1138 / 1151 / 1157 (median 1151): no change (+0.9 %, inside
the 11 % run-to-run spread). Not merged: a non-DoS-resistant hasher for fingerprints would have been justifiable
(the fingerprint only decides reuse of a page's own cached style) but buys nothing here. (Note `ahash` is
already a dependency if a hasher is wanted later.) The YT watch profile was not taken (the live-page run was
denied by the permission classifier in this session).

What is left: nothing in hashing. The remaining big-css cost is the per-level cascade
(`resolve_style_core`, 26 % inclusive) and `ComputedStyle` copies (`memmove` 20 %, much of it rule-vector extends
in `parse_stylesheet_with_keyframes` on the first build and `Vec<StyleRule>` moves in `StyleCache::rebuild`).

### 10. QY53l: what dirties geometry between `getPlayerSize` reads (2026-11-19, measured + partial fix)
**Setup.** Main 68501e8, own worktree release build (V8 + media), own wire on a scratchpad socket, DEFAULT
UA, `--perf-report --budget-ms 20000 --seconds 60` on watch?v=G1AeBgalFZk, `SCRATCHY_MEDIA_AO=null`; load
average 8-20 (other agents' builds), so ms numbers are noisy and COUNTS are the metric. **Throwaway
instrumentation (reverted, never committed):** every `GEOM_GEN` bump recorded the write (native, element
`tag#id.class`, attribute, old and new value, top JS frame and first page frame via `caller_frame`); every
`GeomCache::refresh` miss printed which key part moved (geometry generation / viewport / CSS version), the
QOBS3 caller, whether the rect map changed, and the writes since the previous miss.

**What it reads.** `g.k.getPlayerSize` (player base.js:15474) reads `#movie_player.clientWidth` (R43
section 2), from the player's event pipeline after almost every state change; 66 of 67 of its layouts
were keyed by a geometry-generation bump (never viewport or CSS).

**What dirtied it (main, 67 getPlayerSize layouts, 46 no-change).** 24 of the 46 no-change layouts
followed ONLY `style` attribute writes, and nearly all of those were scratchy's own `styleSet` (the
`el.style.x = v` / `setProperty` path in js_dom.rs): it removed the property and APPENDED it again, so
setting a value the element already had rotated the declaration order (`left: 0px; transform:
scalex(0);` -> `transform: scalex(0); left: 0px;`), the `style` string changed, QY53c's same-value check
did not fire and the geometry generation moved. Writers: the progress bars (`.ytp-load-progress`,
`.ytp-play-progress`, `.ytp-hover-progress`: `left` and `transform` written in turn by base.js `Wba`/`nZ`
on every tick) and `#movie_player`'s custom properties (`--yt-delhi-*`, `--ytp-grid-*`, base.js `kZa`).
Not our media prelude (no QM1k1n1 parking write appeared), not `aria-*`/`data-*` (1 layout).

**Fix (QY53l).** `styleSet` updates an existing declaration in place (CSSOM: only a new property is
appended), so a same-value write produces the byte-identical attribute and QY53c skips the bump. No new
skip rule was needed; tests `qy53l_*` (js_geometry_tests.rs).

**Result, 3 interleaved runs each (main 68501e8 / branch):**
| | main | branch | change (median) |
|---|---|---|---|
| getPlayerSize layouts | 67 / 76 / 71 | 52 / 51 / 51 | 71 -> 51, **-28 %** |
| all forced layouts | 128 / 134 / 132 | 104 / 103 / 105 | 132 -> 104, -21 % |
| `no_change` | 70 / 76 / 73 | 45 / 44 / 47 | 73 -> 45, -38 % |
The item's gate (getPlayerSize halved) is NOT met: what is left is not a no-op write.

**What is left (instrumented branch run: 52 getPlayerSize layouts, 30 no-change), by the kinds of write
before each no-change layout:** tree inserts/removes only 9 (player chrome built piecewise: popups,
overlays, `svg` icon swaps, `<script>` into `<head>`; the new nodes get no box, i.e. they are in or are
`display:none` subtrees); `style` only 8 (5 are `.ytp-load-progress` `transform: scalex(..)`, a scale
transform that scratchy's layout ignores (only translate moves boxes), others `max-width`/`width` of hidden
chapter containers and `#movie_player` custom properties); class only 3 (`#movie_player` state classes,
which DO change rule matching); the rest mixes (attr+tree 2, attr+class+tree 2, class+style 2,
attr+style 2, attr 1, class+tree 1). Skipping all style/attr-only ones would leave ~41 (-42 %);
reaching half also needs the tree writes, i.e. knowing a write lands in a `display:none` subtree.
Proposed (coordinator files): **QY53l2 [M]** skip the geometry bump for writes whose target (insert:
the parent; remove: the old parent; attribute: the element) has a `display:none` ancestor-or-self
in the CURRENT GeomCache style (cheap: GeomCache keeps the set of node ids that had no box because of
`display:none` at the last layout; the write must not change `display` on that node itself and
`class`/`style`/`id` writes on the `display:none` element itself still bump); upper bound 9-13 of 51
getPlayerSize layouts. **QY53l3 [S, optional]** a `style` write whose changed declarations are only
properties that cannot move a box in scratchy (scale/rotate-only `transform`, `opacity`, colors,
`background-*`), and no selector tests the `style` attribute (RuleIndex): skip the bump; ~5-8 of 51,
must be revisited when scale transforms reach getBoundingClientRect.
### 11. QY53e3: App layout vs forced layout in a live window (Opus, 2026-11-19, docs only)
**Setup.** Main 15a3222 (QY53e1 c5098fa, QY53g/h/i, QY53f1 01d20dd, QOBS5 bf9aea1 in), own worktree release
build (V8 + media), own wire on a scratchpad socket, nested dwl (tools/nest.sh), DEFAULT user agent,
`SCRATCHY_MEDIA_AO=null`, `SCRATCHY_FRAME_STATS=<file>`, 66 s per run (`timeout`). Input through tools/inject
from t = 8 s (YouTube: 13 s, after a click on the player's play button in runs 3-4): 6 x (4 pointer moves over
links/controls, wheel 3 down, 2 moves, wheel 2 down, wheel 1 up), ~36 s. The nested output was 956x1002 in
Wikipedia run 1 and 956x485 in all other runs (the host compositor re-tiled the nested window; not ours).
**Throwaway instrumentation (reverted with `git checkout`, never committed):** one stderr line per GeomCache
miss (`F`: time, `geom_gen`, `css_ver`, viewport, `in_io`, thread-CPU ms, no-change), per App tree build in
`App::redraw` (`B`: `layout_generation`, `geom_gen`, `css_ver`, window size, build ms), per COMPLETE
`ShapeResult` (`S`: generation, current `layout_generation`, `geom_gen`/`css_ver` now), per `js_sync` that took
`dirty` (`D`), and from the IO prelude per `ioRecheck` (`IOR`) and per QY53e1 deferral start (`IOD`, with its
deadline). A "generation" below is the pair (`geom_gen`, `css_ver`).
QY53l (section 10, `styleSet` updates a declaration in place) was merged after these runs: it turns some formerly
value-changing `style` writes into same-value ones, so the no-op rebuild counts below can only grow on newer main.

**Runs.** Wikipedia Rust: 3 runs (wp1-3). YouTube watch G1AeBgalFZk: 4 runs; yt1/yt2 without a play click,
yt3/yt4 played (30 frames/s from ~18 s until YouTube's attestation stop at ~57 s, R45). **yt1 froze at
t = 29.5 s**: 0 redraws for the rest of the run while inputs piled up (`input.pending` 2 -> 22, wheel events
included) and page JS kept running (~135 ms/s); no App build after 21 s. That is the freeze signature of
QM1k-mouse / QM1k1n6 (not caused by the instrumentation: yt2-4 with the same binary did not freeze); yt1's
numbers below cover its first ~30 s only. No challenge or consent wall.

**Totals per run (frame stats and the lines; ms = main-thread ms):**
| run | App builds (n / ms / median ms) | forced layouts (n / ms; IO n / ms) | hover restyles | redraws |
|---|---|---|---|---|
| wp1 | 57 / 8 548 / 161 | 19 / 5 993; 13 / 4 091 | 23 | 159 |
| wp2 | 63 / 7 994 / 131 | 13 / 3 360; 11 / 2 862 | 22 | 166 |
| wp3 | 24 / 4 202 / 170 | 7 / 3 021; 6 / 2 517 | 13 | 80 |
| yt1 (froze) | 40 / 1 834 / 49 | 142 / 3 195; 29 / 1 008 | 3 | 64 |
| yt2 | 10 / 341 / 27 | 105 / 2 653; 5 / 175 | 1 | 35 |
| yt3 (played) | 80 / 3 817 / 46 | 228 / 7 398; 43 / 1 752 | 22 | 157 |
| yt4 (played) | 87 / 4 039 / 46 | 208 / 6 936; 31 / 1 368 | 25 | 172 |
**In a window on Wikipedia the App's own tree builds now cost MORE main-thread time than forced layout**
(4.2-8.5 s vs 3.0-6.0 s per run); on YouTube forced layout is still 1.7-1.9x the App builds in the played runs.

**Finding 0: the two layouts are never at the same viewport.** Every forced layout ran at 1024x768
(`DEFAULT_VIEWPORT`, js.rs: `innerWidth/innerHeight` are fixed, open item Q33b2), the App at the window size
(956x1002 / 956x485). So today no App layout can serve a script read or an IO evaluation, and no GeomCache tree
can serve the App, whatever the generation. Also: the App sizes images, GeomCache does not (section 7).

**(A) App tree builds at a generation GeomCache had already laid out:** counted against the latest forced layout
before the build. All builds: wp 22/57, 24/63, 12/24; yt 23/40, 4/10, 40/80, 38/87. But almost all of those are
REBUILDS of a generation the App had itself built just before (hover restyles, no-op writes, below). Counting only
the FIRST App build of each generation: **wp 1/22, 2/20, 0/7 (0-0.26 s per run); yt 2/18, 3/9, 5/35, 7/41
(0.07-0.26 s)**. Reusing the JS side's `LayoutBox` tree in the App is not worth an item (and needs Finding 0
fixed and image sizes in GeomCache first).

**(B) Forced layouts at a generation an App build had already built** (built / already completed): wp 10/8,
7/4, 4/4 of 19/13/7; yt 7/2, 1/0, 17/13, 19/16 of 142/105/228/208. Nearly all of them are IO evaluations; page
SCRIPT reads at an App-built generation: **wp 0, 1, 0; yt 0, 1, 4, 1** (completed: 0-1 per run). YouTube's script
reads (`getPlayerSize` etc.) come in bursts right after writes, before the App builds. Serving script reads from
the App's layout: not worth it.

**(C) Delay from an IO recheck to the next complete App layout covering its generation** (0 = the latest complete
App layout already had that generation):
| run | QY53e1 deferrals (`IOD`): n, median / p90 / max ms to an App layout | e1 deadline ms | App layout before the deadline | IO forced layouts: median / p90 / max ms to an App layout |
|---|---|---|---|---|
| wp1 | 11: 204 / 532 / 539 | 934-1 322 | 11 of 11 | 0 / 209 / 1 021 |
| wp2 | 9: 150 / 436 / 436 | 919-1 093 | 9 of 9 | 3 / 169 / 678 |
| wp3 | 4: 241 / 336 / 336 | 1 518-1 719 | 4 of 4 | 0 / 1 360 / 1 360 |
| yt1 | 20: 1 093 / 2 039 / 2 046 | 100-190 | 5 of 20 | 1 110 / 1 909 / 2 063 |
| yt2 | 4: 763 / 1 226 / 1 226 | 100-182 | 1 of 4 | 562 / 1 265 / 1 265 |
| yt3 | 35: 437 / 1 629 / 2 419 | 100-170 | 16 of 35 | 801 / 1 615 / 2 266 |
| yt4 | 33: 77 / 1 807 / 2 743 | 100-190 | 19 of 33 | 30 / 1 426 / 2 197 |
Wikipedia: the App's layout always arrives well inside QY53e1's deadline (its forced layouts cost 260-410 ms, so
the deadline is ~1-1.7 s), and at a median IO forced layout the App ALREADY had a complete layout of that
generation. YouTube: the deadline is 100-190 ms (forced layouts are cheap there) and the App's layout comes later
in about half the cases (the main thread is in page JS; the App defers a rebuild while one is in flight), with a
2-2.7 s tail.

**(D) Complete App layouts current at completion:** every complete `ShapeResult` was for the current
`layout_generation` (wp 57/57, 63/63, 24/24; yt 40/40, 10/10, 79/80, 87/87): the App never wastes a whole shaping
pass. The DOM generation had NOT moved during the build in **wp 42/57, 53/63, 21/24 (74-88 %)** and **yt 25/40,
4/10, 58/80, 68/87 (40-78 %)**; the rest are complete but one write batch behind, so QY53e2's "target missing
from `rendered` stays pending" rule will trigger often on YouTube.

**App rebuilds after only no-op writes** (`dirty` set by a write batch that moved neither `geom_gen` nor `css_ver`,
same window size, so the rebuilt tree is the same as the previous one): **wp 9/57 (1 408 ms), 18/63 (2 313 ms),
2/24 (336 ms); yt 19/40 (942 ms), 0/10, 23/80 (1 135 ms), 21/87 (880 ms)**. Builds at an unchanged generation
WITHOUT any JS write in between (hover restyles; frame stats `hover_restyles` 13-25 per Wikipedia run): **wp 26
(3 847 ms), 25 (3 197 ms), 15 (2 572 ms); yt 3, 1, 22 (1 147 ms), 25 (1 315 ms)**. On Wikipedia each hover change
over a link is a full 130-170 ms App tree build on the main thread, the largest single main-thread item there in
a window.

**Verdict on QY53e2.** Still worth doing, but NOT as specified: as written it would evaluate App rects (window
viewport, sized images) against an IO root of `innerWidth x innerHeight` = 1024x768 (Finding 0), i.e. a third
geometry. Do Q33b2 (real `innerWidth/innerHeight` from the window, QY10a's plumbing) first, or let QY53e2 take
the root rect from the App's viewport and run only when the two agree. Gain once that holds: Wikipedia IO =
6-13 forced layouts per 60 s, 2.5-4.1 s (68-85 % of forced-layout CPU in the window), all replaceable within the
deadline; YouTube: about half of 31-43 IO layouts (~0.8 s per 60 s). Keep QY53e1's deadline as the fallback.

**Proposed items (coordinator files them):**
- Q33b2 promoted to P1 and ordered BEFORE QY53e2 (S, existing). Gate: in a window `innerWidth/innerHeight` equal
  the App's viewport (unit test on the env update; live: a forced layout's viewport equals the window size);
  `--perf-report` headless unchanged (1024x768 there).
- QY53m [S] (after QM1k1n6, same function): `App::js_sync` skips `request_js_restyle` when no write of the batch
  changed a value (and `css_ver()` and `media_size_changed()` did not move). Key it on its own "value changed"
  generation (QY53c's same-value check only), NOT on `geom_gen()`: QY53l2/l3 (section 10) would stop bumping
  `geom_gen` for style-only and `display:none` writes that still need a repaint. Gate: Wikipedia Rust
  live 60 s with this section's input script, frame stats `builds.n` and `builds.ms` drop by the no-op count
  above (median of 3), screenshot unchanged; test: a same-value `setAttribute` leaves `layout_dirty` false.
- QY53n [M, Opus design] hover restyle without a whole App tree build (restyle and re-box only the subtrees whose
  `:hover` match changed, or keep the box tree and patch the styles of the HoverKeys-matched elements). Gate: frame
  stats on Wikipedia Rust with pointer moves only over links: `builds.ms` per `hover_restyles` <= 20 % of today's
  130-170 ms, `input.lat_p95_ms` reported. Overlaps QM1k1n6 (pointer coalescing caps how OFTEN this runs; this item
  is what each one costs) and QM1k-mouse.
- Not proposed: App reusing GeomCache's `LayoutBox` tree ((A): <= 7 first builds per run), script reads from the
  App's layout ((B): <= 4 per run).

**Not established:** why yt1 froze (QM1k-mouse/QM1k1n6 own that); the App build phases (only its total was
timed); whether Wikipedia's no-op write batches are the page's or our preludes'.

**QY53e2 result (Sonnet, 2026-11-20): the App-layout path works but its gate is NOT met.** In a window IntersectionObserver
rechecks now run on the App's own complete layout when the DOM/CSS stamp taken at the App's tree build still equals the page
(`JsEngine::rendered_geometry`); otherwise the forced path (QY53e1 deadline, unchanged) runs. Local fixture (write after the page
is quiet): 1 forced layout (the initial one) + 2 App-served rechecks delivered 46 ms after the write. Nested dwl, own wire, flock,
Wikipedia Rust 30 s x 3 (load 4-11, WITH the flat 2 s window deadline described below): IO forced layouts 2-4 per run, 0 App-served (gate was <= 1). YouTube watch 60 s: 23 IO forced
layouts, 1 App-served. Why: page scripts write the DOM through the whole load (geometry generation past 600 within the first
second), so every complete App layout is already one write behind and the stale rule rejects it (33 of 34 on YouTube, 3-4 of 4 on
Wikipedia). That deadline is what cut Wikipedia's IO layouts (R43 section 11 had 6-13); it was dropped by the coordinator: up
to 2 s late IO delivery is not worth it. The churn case is QY53e2b (use the last complete layout during churn).
### 12. QY53f1b: why YouTube's slowest forced layout did not get faster after QY53f1 (Sonnet, 2026-11-19, FIXED)

Throwaway logging (reverted): per forced layout the phase deltas and the `rules_cache` path. YT watch 60 s, default UA,
130 layouts: 126 `hit`, 3 `splice`, 1 `full` (the cold first build, no cache yet). The 5 slowest: #1 is that first build,
226 ms (css 200, rule_index 20, build_children 5); #2-#5 are cache hits of 45-54 ms (pre 13-15, build_children 27-34,
measure 3-4, css under 1 ms). The splices cost 34-44 ms (css 8, rule_index 16-18). So `max_ms` 250-305 is NOT the `@layer`
or shared-list fallback: it is the one unavoidable cold parse of the 4.4 MB sheet, 22 343 rules. Inside it: the parse
itself 126-132 ms, then 55-75 ms in `rules.extend(r)`: `size_of::<StyleRule>()` is 4040 bytes, so moving the parsed Vec into
the final one copies ~90 MB (memcpy plus page faults). Fix: `css::parse_stylesheet_into` parses straight into the target Vec
(`rules_cache::parse_piece` appends, no intermediate list), final list `shrink_to_fit`. Test:
`qy53f1b_parse_stylesheet_into_appends_and_matches_the_returning_form`.

Numbers (interleaved, `--seconds 6`, machine load 9-13 on 16 cores, so absolute values are inflated): `forced_layout.max_ms`
main 501 / 426 / 349 / 552 / 339 (median 426), branch 257 / 194 / 254 / 251 / 174 (median 251): -41 %. css_collect total median
main 477 -> 256 ms. A 60 s interleaved run (3 pairs) was too load-noisy: max_ms main 343 / 359 / 606, branch 257 / 401 / 316.

Left: the cold parse is still ~130 ms (cssparser + selector building, 4.4 MB), and the first rule_index build ~16-20 ms;
`js_style::StyleCache` has the same `rules.extend(parse_stylesheet(..))` move (js_style.rs:584). Proposed items: QY53f1c
(`StyleRule` 4 KB: box the cold fields / shrink `Declarations`, which would also cut rules_cache's 96 MB), QY53f1d (parse
the big external sheet lazily or off-thread).

### 13. QY53l2: skip the geometry bump for writes inside `display: none` subtrees (2026-11-19, merged, gate NOT met)
**Mechanism.** Every `GeomCache` forced layout records the elements the box build skipped because their
own `display` is `none` (images.rs `DISPLAY_NONE_ROOTS`, filled only during that build) and keeps them
as the "hidden roots" snapshot, with the geometry generation and CSS version it ran at
(js_geom.rs `HiddenSnap`). A DOM write skips `geom_bump` only while that snapshot is current (no bump
and no CSS change since the layout) and: an attribute write targets an element STRICTLY inside a
hidden root (never the root, never an ancestor; `style`/`link`/`meta`/`base`/`slot` elements never);
a child-list change (insert, remove, `textContent`, `innerHTML`, text-node data) has its parent at or
inside a hidden root, for a move BOTH ends (so moving a node out of, or a hidden node into, a visible
place still bumps), no moved node or HTML string carries a `style`/`link`/`meta`/`base`/`slot`, and,
when the parent IS the root and some rule uses `:empty`, the root's `:empty`-ness is unchanged. Roots
under `select`/`datalist`/`option`/`svg`/`math`/replaced elements/`template`/`slot`/`col`/
`textarea`/`noscript` are not trusted (their content is read outside the box build). Any `:has()`
rule disables the whole thing (scratchy's selector parser does not accept `:has()` yet, so this is
defensive). Reads of hidden nodes are unchanged (`ioShown` checks computed style, which keys on the
DOM write generation, not the geometry one).

**Result, YT watch, default UA, own wire.** Round 2 (the merge base), 5+5 interleaved, main b8d9761 /
branch dff4eca:
| | main | branch | change (median) |
|---|---|---|---|
| getPlayerSize layouts | 49 / 52 / 49 / 49 / 49 | 44 / 45 / (103) / 45 / 45 | 49 -> 45, **-8 %** |
| all forced layouts | 101 / 102 / 101 / 101 / 101 | 95 / 96 / (187) / 96 / 93 | 101 -> 95.5, -5 % |
| `no_change` | 40 / 42 / 40 / 40 / 40 | 34 / 35 / (87) / 35 / 34 | 40 -> 34.5, -14 % |
(branch run 3 got a different YouTube page variant, 1.88 MB HTML instead of 1.34 MB, with every count
doubled; excluded.) Round 1, 3+3, against main 7bf6875 (before QY53f1b 9415fb2): getPlayerSize
54 / 49 / 56 vs 50 / 44 / 45 (-17 %), `no_change` 45 / 40 / 44 vs 39 / 34 / 34; main itself dropped
to 49 with QY53f1b, so round 2 is the number that counts. 4 of 4 comparable pairs: 4-7 fewer
getPlayerSize layouts and 5-6 fewer no-change layouts per 60 s.
Gate (getPlayerSize -20 %) NOT met; merged as a safe net win. **What is left** (throwaway instrumentation, reverted: the writes
logged before each layout, run on the branch): of 71 no-change layouts, 44 follow only writes to
VISIBLE elements (`.ytp-chapter-hover-container`, `.ytp-load-progress` `transform: scalex()`,
`#movie_player` state classes, `video` attributes), 8 follow only writes inside an inline `<svg>` (7 more mix those with visible writes)
(`circle.ytp-timed-pie-countdown__inner`, icon `path`s: the svg is a replaced box sized by its own
attributes/CSS), the rest are writes on a hidden root itself (`class`/`style` that keep it hidden) or
inside a hidden subtree after an earlier real bump. Further cuts need QY53l3 (paint-only style
declarations) and possibly "attribute writes strictly inside an inline `<svg>` (not on the svg, no
`<style>`/`<foreignObject>`) keep its box" (needs a check that the svg's decoded intrinsic
size only reads the root's attributes; it alone would save at most 8 of 71).

### 14. QY53e1b: QY53e1's deferred IntersectionObserver recheck does NOT make YouTube worse (Sonnet, 2026-11-19, docs only)

Question: QY53e1 (c5098fa) cut Wikipedia's forced layouts 8 -> 4, but its YT watch sample (2+2 runs, loaded machine) was worse
(272/284 layouts vs 144/223). Is the deferral (`__d_geom_fresh` / `ioDue`) making YouTube worse (lazy-load observers firing late ->
more continuation loads -> more layouts), or was that noise?

Method: current main (573ab64, after QY53g/h/i/l, QY53f1/b/c, QM1k1n6/a/b), one release binary; the OLD immediate path was
restored by a scratch-only env switch (`SCRATCHY_IO_IMMEDIATE`: `__d_geom_fresh` returns true, so every geometry-generation change
nudges the recheck at once; never committed). 5+5 interleaved headless runs, `--perf-report --budget-ms 20000 --seconds 60`,
own wire, default UA, `SCRATCHY_MEDIA_AO=null`, https://www.youtube.com/watch?v=G1AeBgalFZk, machine load 6-8 on 16 cores.
The report has no request counter; "network callbacks" below = all non-script/timer/promise/event callbacks, the closest proxy
for fetch/XHR continuations.

| run (def = deferred, main; imm = old immediate path) | layouts | total_ms | no_change | io n / ms | script n / ms | net callbacks |
|---|---|---|---|---|---|---|
| def 1 | 100 | 4033 | 40 | 22 / 1706 | 78 / 2327 | 235 |
| def 2 | 100 | 2443 | 39 | 23 / 872 | 77 / 1570 | 234 |
| def 3 | 103 | 4796 | 42 | 21 / 2011 | 82 / 2786 | 236 |
| def 4 | 200 | 15779 | 106 | 25 / 2836 | 175 / 12943 | 284 |
| def 5 | 102 | 2836 | 42 | 24 / 1482 | 78 / 1354 | 240 |
| imm 1 | 105 | 2275 | 41 | 28 / 989 | 77 / 1286 | 245 |
| imm 2 | 104 | 2825 | 41 | 28 / 1113 | 76 / 1712 | 243 |
| imm 3 | 156 | 11750 | 74 | 21 / 1909 | 135 / 9840 | 268 |
| imm 4 | 105 | 3847 | 42 | 24 / 1407 | 81 / 2440 | 241 |
| imm 5 | 95 | 1765 | 42 | 17 / 572 | 78 / 1192 | 223 |

Medians: deferred 102 layouts / 4033 ms / no_change 42 / io 23 (1706 ms) / script 78 (2327 ms) / 236 net callbacks; immediate
105 / 2825 ms / 42 / io 24 (1113 ms) / script 78 (1712 ms) / 243. Layout COUNT, io count, script count, no_change and network
callbacks are the same within one or two units. The top caller is `g.k.getPlayerSize` (49-52 layouts) in 9 of 10 runs, both modes.
The total_ms medians differ by 1.2 s but the run-to-run spread within one mode is 2.4-15.8 s (deferred) and 1.8-11.8 s
(immediate), and the two long tails are one run in each mode (def 4: 200 layouts, `getPlayerSize` 137; imm 3: 156 layouts,
`getPlayerSize` 92) with `script` layouts, not `io`, doing the extra work: YouTube's own spread (the TODO's 170-313), not the IO
path. The `io` layouts are 21-25 deferred vs 17-28 immediate; the deferral does not shift lazy loading into more requests.

Visible content: nested-dwl screenshots at 30 s of both builds are BYTE-IDENTICAL (`cmp`): player, title, channel row, description
box and comment skeletons; the comments are still skeletons at 30 s in both (an existing state, not a deferral effect).

Verdict: noise; QY53e1 is not making YouTube worse. No code change. The deferral stays.


### 15. Q33b2v: live check of Q33b2 (window size -> innerWidth/innerHeight, forced-layout viewport) (Sonnet, 2026-11-20, docs only)
Main c63b17b release build, own wire on a scratchpad socket, nested dwl (tools/nest.sh, 956x1002 output), page = a 50%-width
box plus a script logging `innerWidth`, `innerHeight`, `documentElement.clientWidth`, the box's `getBoundingClientRect().width`
at script time, at `load`, at +500 ms and in every `resize` handler (console output + printed into the page). The window was
resized by opening/closing a second scratchy window in the nest (dwl tiles: 956 -> 428 -> 956 wide). Files (scratchpad, not
committed): q/p.html, q/p2.html, q/s1.log, q/s2.log, q/s3.log, q/shot1.png .. q/shot4.png.
- Window at rest (956x1002): iw=956 ih=1002 cw=956 half=478 (screenshot 1, orange box = half the page). HOLDS.
- Second window opened: first window iw=428 ih=1002 cw=428 half=214; the new window iw=524 half=262 (screenshot 2). One `resize`
  per change in each window (resize#2 for the old, resize#1 for the new). HOLDS.
- Second window closed: first window back to iw=956 half=478, exactly one more `resize` (resize#3; screenshot 3). HOLDS.
- ih changed 768 -> 1002 on the first configure and followed with it. Only width changes were exercised after that (dwl tiling).
- **Finding:** a script that reads the size synchronously at parse time or in `load` still sees the headless default
  (iw=1024 ih=768 half=512), because the first window configure arrives after the page's scripts started; one `resize` then
  fixes it (+500 ms reads 956x1002). Later reads and the `resize` handler are right. A real browser reports the real size
  from the first script. Proposed: Q33b2w (publish the surface size to the JS thread before the first page script runs, or
  hold the first script run until the first configure; no `resize` event for the initial correction).

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
### 6a. QM1k1n5 addendum (Sonnet, 2026-11-19, main 1b83534 + QOBS6): the delay does NOT reproduce

Own worktree release build (V8 + media), own wire, nested dwl, `SCRATCHY_FRAME_STATS`, `SCRATCHY_MEDIA_AO=null`,
youtube.com/watch?v=G1AeBgalFZk, play clicked at +18 s, ~25 s of moving pointer, 110 s window, under the perf lock
(uptime load average 7.7 at the start, 1.8 at the end; other agents were building). 124 page network results were
delivered (QOBS6 `net`): approximate median 41 ms, per-second p95 at most 187 ms except one second at startup
(+6 s) with max 494 ms, nothing after +12 s above 63 ms. Result to JS is 0-0.5 s after the transport finished,
not 1-19 s. Event-loop ordering is not at fault: `user_event(JsNet)` calls `deliver_net` directly (no queue in
scratchy behind timers), `about_to_wait` runs timers in a 16/50 ms slice (QM1k1n6b) and winit hands the next queued
user event to the next turn. The 1-19 s of run 3 were main-thread stalls (forced layouts, the 20-40k turns/s
spin, mpv sleeping in render), which QY53a-c, QM1k1n6 and QM1k1n6b removed. Playback in this run still ended
~5 s after the first frames (YouTube's own stop, R45), unrelated to delivery. Permanent guard:
`qm1k1n5_response_arriving_during_a_busy_timer_chain_is_delivered_between_slices` (fetch_tests.rs). The
`net` field now shows any regression of this directly. Not covered: wire-to-main-thread time is measured from
the transport thread's stamp, so time inside wire itself is not in `net`.


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

### QP3 addendum (2026-11-19): cost of a font-match cache miss: measured, closed as not worth it

`tools/profile.sh` (perf, 999 Hz so 1 sample ~ 1 ms CPU, all threads and all 3 FontSystems summed, DEFAULT user agent, own wire) on main e518ed0, inclusive samples of the miss path:

| page | `with_face_data` | `FontMatchKey::new` | `get_font_matches` | `shape_spans` (all shaping) |
|---|---|---|---|---|
| Wikipedia Rust (25 s run, 3117 samples) | 4 ms | 9 ms | 8 ms | 401 ms |
| YouTube watch G1AeBgalFZk (10240 samples) | 1 ms | 3 ms | 2 ms | 31 ms |

The whole miss path is <= ~10 ms per page (threshold was 20 ms), and the mmap part (`with_face_data`) is 1-4 ms. `make_shared_face_data` for every face in `new_font_system_with` would keep every system font mapped in all 3 FontSystems (RSS and startup cost) to save at most ~4 ms, so it was NOT done. No code change. Reopen only if a page with hundreds of distinct font attrs shows `get_font_matches` above 20 ms in a profile.

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
- (QAN1h, 2026-11-19: re-measured in R49. A page of this size now restyles in 126 ms headless and ~230 ms
  in a window. Today's bigger github.com home page takes 0.65-1.5 s; items QAN1h1-QAN1h3.)
  GitHub's 15-18 s restyle with cached rules is itself a problem (hover,
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

### QY53n1 addendum (Sonnet, 2026-11-20): exact hover affected-set implemented

HoverKeys now records per key what follows the `:hover` compound (descendant/child -> the hovered element's subtree; sibling -> later siblings and their subtrees) and `unbounded` for a `:hover` that reaches other elements. `:not(:hover)`/`:is(:hover)` on the same compound are keyed by that compound (YouTube has dozens, e.g. `ytd-compact-video-renderer:not(:hover) ...`; the earlier "count the word hover" test made every YouTube page unbounded). `:has()` is not parsed at all, so its rules never exist. YT watch (G1AeBgalFZk), pointer over the player and the page, nested dwl, own wire, load avg 10-22: 15 hover restyles = 7 skipped + 8 built (before: 16 restyles, 0 skipped); Wikipedia link hovers: 8 restyles, 8 built (a:hover changes the matched set, as designed).

### QY53n2 addendum (Opus, 2026-11-20): paint-only hover patch implemented

Design points 3-4 above, with two findings: (1) Wikipedia declares `animation-*` longhands on every element (no keyframes run), so "has `animations`" cannot be the refusal test; `hover_patch` refuses only an affected element whose animation names existing keyframes, and resolves the chain with the base cascade (`evaluate` would add records to the next build's animation report). (2) The tab links' `::after` bar is absolutely positioned, so it lands in an ancestor's `positioned` list, not under its element's box: boxes are told apart by the new `ComputedStyle::pseudo` slot, not by tree position. Wikipedia Rust `--no-js`, 8 links incl. the 3 tabs, 3 locked runs (load 0.4-8): 49 of 49 hover restyles patched in each run, builds per hover_restyle 4.3 / 7.3 / 7.4 ms (page-load builds only; main 144 ms at load 3-4), 3.9-9.9 ms per patch; screenshots equal main's. QHOV1: the hover underline paints on main as well (pointer held, screenshot after the frame). Not covered (fallback to a build): OFFSET rules (QY53n3), `::marker`/first-line/first-letter, list items and form controls (their text spans carry no element), `<html>`/`<body>`. `input.lat_p95_ms` median stays ~156 ms: the R2.3/QB3 coalescing delay still applies to patched hovers (QY53n5). After rebasing on QMEM3j (7fef1a9: a line's first span lives in the `AttrsList` defaults) the patch rewrites the defaults too (the pixel test caught it); re-measured at load 25-37: 51/51, 49/49, 49/49 patched, 10.6 / 5.5 / 26.7 ms of builds per hover_restyle, 13-24 ms per patch, screenshots still equal main's.


### QY53n5 addendum (Sonnet, 2026-11-20): patched/skipped hovers skip the coalescing delay

`App::update_hover` first tries `apply_hover_cheap` (same chain, chain no `:hover` rule reads, QY53n1 skip, QY53n2 paint patch); only a hover that needs a build goes through `hover_restyle_delay` (R2.3/QB3) and `apply_hover_build`. Cheap hovers are still coalesced once per event-loop turn by QM1k1n6a (`flush_pointer_move`). Wikipedia Rust `--no-js`, nested dwl 960x489, own wire, 8 links (5 TOC, Talk, Edit) x 6 rounds x 600 ms, under flock, before/after interleaved, 48 of 48 hover restyles patched in every run: `input.lat_p95_ms` median over pointer-phase seconds before 175.0 / 176.7 / 159.0 (median 175), after 27.7 / 20.8 / 14.2 (median 20.8); max 105 / 90 / 27 ms. Load average 14-35 (other agents building). Gate <= 50 ms met.


### QY53n3 addendum (Sonnet, 2026-11-20): offset hover patch
`hover_patch::paint_patch` now also accepts `Effect::Offset` rule changes (slot 0 only). Per changed element the shift is `offset_delta(old, new)` from the build's own resolved styles (old values checked against the tree): translate change (`px + frac * own w/h`, exact because the size is unchanged) plus, by `position`: static = insets ignored; relative = px `left-right` / `top-bottom` change; absolute = only with explicit width, no auto margins, px insets and the same placing edge (left wins over right, top over bottom) in old and new. Refused (build as before): percentage/calc insets, auto-width absolute boxes (shrink-to-fit follows the insets), auto -> set edge changes (needs the static position), sticky/fixed inset changes, pseudo-element offset rules, inline-level boxes (the build's line layout drops a translate on an inline-block, so a shift would differ from a rebuild), animated elements, any positioned DOM descendant (`hoist_negative_z` moves them out of the box's subtree and static positions of fixed/abs descendants follow the box at build time), and a move that changes the root's grown height (`in_flow_bottom`; the moves are undone). `shift_laid_tree` does not move `ink`, so `compute_ink` runs over the tree after the shifts; the App refreshes IntersectionObserver geometry and the cursor when a box moved. Tests compare boxes, ink and pixels of the patched tree with a rebuild for enter and leave. NOT measured: the YT watch gate (hover builds that are only PAINT/OFFSET -> 0; nested dwl + own wire under flock), see the follow-up in the PR.

### R36 addendum 2 (QY47b, Sonnet, 2026-11-20): the extra createElement wrapper costs no measurable page time on V8 -> close QY47b

QY47b proposed folding the `media_prelude.js` `document.createElement` wrapper (`String(n).toLowerCase()`, compare with `video`/`audio`, `ce.apply`) into the DOM prelude to save a JS frame. The motivation (QuickJS's 1 MB stack, R36 wall 2) is gone: V8's limit is the real thread stack (QV8-3b) and frames are far smaller. Measured whether the wrapper still costs anything.

Method: installed main binary (commit 4e807b4e31), `--perf-report --seconds 0` on a local page that calls `document.createElement("div")` 1,000,000 times in one inline script; `cur` = main as is, `extra` = the same page with ONE more identical wrapper layered on in the page (a faithful stand-in for the media wrapper being removed: String + toLowerCase + two compares + `apply`). 4 interleaved pairs, each run under `flock /tmp/scratchy-perf.lock`, load 0.38-0.47 (quiet).

| pair | cur (ms / 1M calls) | extra (ms / 1M calls) | delta |
|---|---|---|---|
| 1 | 993.7 | 1079.8 | +86 |
| 2 | 1002.8 | 1085.0 | +82 |
| 3 | 1003.7 | 1101.6 | +98 |
| 4 | 999.7 | 1104.9 | +105 |

So one wrapper costs about 85-105 ns per call (createElement itself is about 1.0 us). On YouTube results (about 24k elements, not all created through createElement: the parser builds most) that is at most ~2.5 ms of a multi-second load, below run-to-run noise of any page metric. No stack-depth benefit either (V8 stack is not the limit). The folding would also need a cross-prelude hook (the DOM prelude is evaluated before `observe()` exists), adding a check on the hot path anyway, and risks the QM1f lazy-observer behaviour. Verdict: NO MEASURABLE SAVING on V8; close QY47b without code. (The custom-elements `created`/`ceUpgrade` and script-creation wrappers are the same order of cost per call and are not worth touching either.)

**QINC1h2 DONE (Sonnet, 2026-11-20).** Intrinsic-size cache: `natural_width_impl` memoises (min, max) per (node, as_child) with the
box's `bver`, only during an engine measure (the shadow's from-scratch measure never uses it, so `cmp_laid` stays an independent
check); incomplete (deadline) results are not stored. Bottom-up: a box below an intrinsic reader (`Ck::intr`) is re-measured and
its old natural widths (entries valid for the old `bver`; a flavor nobody read has none) compared bit-exactly with the new ones;
equal + same parent view = splice. The re-measure runs under `intr_anc_guard` so the new `Ck`s keep `intr_anc` (found by fuzz seed
30608: without it a later step stopped below a shrink-to-fit fixed `html`). Boxes with z-index entries under a reader still go up
(without that check `z_entries_changed` rose 22 -> ~300 per run and the engine was 2x slower from failed attempts). Flex/grid/table
parents still always go up (the container stretches the item after measure; a splice would have to replay that).
Fuzz (release): 7 windows x 3 700 cases (seeds 0 ... 60 000) + seed 70 000: 0 mismatches. Suite 1 511 passed.
YouTube watch, 30 s, `SCRATCHY_INCR_CHECK=1`, 3 runs main (109445c) vs this, 0 mismatches all: walks reaching the root (of ~120
updates) 55/50/54 -> 52/64/51 (NOT fewer: the blockers are z entries, pending out-of-flow boxes and flex parents); "intrinsic_ancestor"
ups 480/449/481 -> 517/683/505 (it now also counts the z-entry precheck, which runs before the flex-parent rule: that one went
190/161/184 -> 13/18/13); intrinsic hits / misses ~1 550 / ~3 950 per run (0.28); intrinsic stops 7-13 per run; engine relayout
time `layout_ms` 86/81/92 -> 67/73/68. Counts vary by run (the page).

## R64: Conformance plan (QWPT1, Opus investigator, 2026-11-21)

Design only: no product code changed. Everything below is a proposal for the user to approve
before any implementation. Numbers were measured on main 8cb8361 (16-core machine, load 0-9),
WPT at the pinned commit ac78b567f5404b6f2dc0418ea380af3946a47fca, unless marked "estimate".

### 0. Summary (plain language)

- **WPT** (web-platform-tests) is the browsers' shared test suite. It has two kinds of tests:
  *reftests* (render two pages, compare pixels; scratchy runs 632 of them today) and
  *testharness.js tests* (a page runs JavaScript checks and reports PASS/FAIL per check; scratchy
  runs none of them today). At our pin WPT has 33,150 testharness files (40,524 with variants) and
  29,177 reftests.
- **It already almost works.** A throwaway probe (section 2.6) inlined 1,522 self-contained
  testharness tests from dom/, html/webappapis/, css/cssom*, the observers, uievents, url,
  encoding, webidl, hr-time and streams, and ran each one through today's `scratchy --js-report`.
  98 % of them reported a result. 10,253 of 23,389 checks pass (44 %); mean per-file score 31 %.
  The whole set took 39 s at 4 parallel jobs. Running WPT for real is mostly plumbing.
- **Plan:** a small static test server inside wire (scratchy stays socket-free), a `--wpt-th`
  runner (one process per test, a virtual clock so timeouts cost nothing), per-area expectation
  files gated in CI like today's reftests, and a committed per-area score table.
- **Event loop audit:** 15 concrete deviations from the HTML "update the rendering" model; the
  biggest: `requestAnimationFrame` is a 60 Hz timer, ResizeObserver never re-fires and reports
  0x0, IntersectionObserver is evaluated at timer boundaries, user scrolling fires no `scroll`
  events, exceptions in timers/rAF never reach `window.onerror`.
- **Missing-API report:** a report mode that puts one forwarding Proxy at the end of the DOM and
  window prototype chains; it sees every lookup of a name scratchy does not have (feature probes
  and calls) without changing any answer. Zero cost outside report mode.
- **Offline pages:** wire records a page load once (`--record`) and replays it (`--replay`), so the
  incr-gate pages stop changing under us and perf numbers can become budgets.
- **Order:** infrastructure, then DOM core, WebIDL + events, HTML event loop + rendering steps (with
  the observers), HTML scripting/parsing, CSSOM, geometry, encoding/url/streams, fetch/xhr, media
  (archived). Changes from the proposal are explained in 3.2.
- 8 user questions in section 7.

### 1. What exists today

- `src/wpt.rs` (Q40a): reftests only. `skip_reason` skips any file containing `<script`,
  `reftest-wait` or `testharness.js`. Reads `wpt-subset.txt` (664 lines), compares with
  `wpt-expected.txt`, exits 1 on any change. Measured: `scratchy --wpt /home/loupax/src/wpt` =
  **0.9 s wall** (pass 333, fail 293, crash 0, skipped 6; "no change"). It is NOT run in CI today
  (`.github/workflows/ci.yml` has build, test, clippy only).
- The local checkout `/home/loupax/src/wpt` is a blobless partial clone (`blob:none`, so file
  contents are fetched on demand) at the pin, sparse: `css fonts html/rendering images resources`.
- `--js-report [--run-timers MS]` (`js_report_for_html_timers`, `js_report_for_scripts_timers`)
  loads a page headless, runs its scripts and timers on a **simulated clock** (time jumps to the
  next due timer), and prints errors and `console.error` lines. The probe used this.
- CI (run 44868420059, a typical push): scratchy job 3 m 40 s: build 1 m 49 s (rust-cache warm),
  test 62 s, clippy 17 s. A clean local release build: 59 s wall, 12 m 17 s CPU.
- WPT publishes a `MANIFEST-<sha>.json` per commit (release `merge_pr_65461` for our pin, 6 MB
  gzip): every test's type, URL variants and `timeout=long`/`testdriver` flags. All counts below
  come from it, so nobody needs a full clone to plan.

### 2. Running testharness.js tests (item a)

#### 2.1 The checkout and the pin

Do not grow the user's `/home/loupax/src/wpt` by hand. A script (`tools/wpt-checkout.sh DIR`, item
QWPT2) makes a blobless, depth-1, sparse checkout of the pinned commit:

```
git clone --filter=blob:none --no-checkout --depth 1 --sparse https://github.com/web-platform-tests/wpt DIR
git -C DIR fetch --depth 1 --filter=blob:none origin <pin>
git -C DIR sparse-checkout set <dirs from tools/wpt-dirs.txt>
git -C DIR checkout --detach <pin>
```

Measured from this machine: 33 s, 227 MB on disk for the phase-1 + phase-2 list below plus all of
`html/` (62 MB of the 227 is `html/`; without the unused parts of `html/` it is ~90 MB).

**Keeping the pin:** the commit lives in ONE file, `tools/wpt-pin.txt` (AGENTS.md points at it
instead of repeating the hash). The script refuses a checkout whose `HEAD` differs; the runner
prints the pin in its summary line; CI caches the checkout with key `wpt-<pin>-<hash of
wpt-dirs.txt>`. Bumping the pin is its own PR (coordinator): new pin, re-run, regenerated
expectation files, the score table diff in the PR body. Never mixed with a fix.

Directories to add (testharness files / URLs incl. variants, from the manifest; size on disk):

| directory | testharness files | URLs | need no server or testdriver (URLs) | size | phase | why |
|---|---|---|---|---|---|---|
| dom | 745 | 809 | 654 | 4.9 MB | 1 | DOM core, events, ranges, traversal, abort; testharness itself depends on it |
| html/webappapis | 346 | 369 | 321 | 2.1 MB | 1 | timers, microtasks, rAF, event loops, error reporting, document.write |
| html/dom | 242 | 259 | 245 | 2.4 MB | 1 | document, elements, dataset, attributes reflection |
| html/semantics/scripting-1 | 552 | 597 | 523 | 3.9 MB | 1/2 | script processing model (119 files need Python handlers) |
| css/cssom | 274 | 274 | 272 | 1.7 MB | 1 | style sheets, getComputedStyle serialization |
| css/cssom-view | 239 | 239 | 232 | 1.5 MB | 1 | geometry: getBoundingClientRect, scroll, elementFromPoint |
| resize-observer | 37 | 37 | 36 | 0.2 MB | 1 | rendering step |
| intersection-observer | 159 | 159 | 151 | 0.8 MB | 1 | rendering step |
| uievents | 69 | 74 | 17 | 0.8 MB | 1 (57 need testdriver) | event constructors; input needs testdriver |
| webidl | 46 | 63 | 44 | 0.3 MB | 1 | binding rules every area relies on |
| url | 35 | 79 | 50 | 0.8 MB | 1 | |
| encoding | 153 | 1,343 | 1,263 | 20 MB | 1 | variants: 1,228 "long" legacy-encoding URLs, run nightly only |
| hr-time | 14 | 19 | 10 | 0.1 MB | 1 | |
| streams | 90 | 307 | 88 | 1.1 MB | 1 | 217 URLs are worker variants |
| webmessaging, requestidlecallback, domparsing | 138, 21, 97 | 167, 21, 97 | 116, 21, 96 | 1.7 MB | 1 | |
| custom-elements, shadow-dom | 212, 318 | 212, 318 | 207, 186 | 3.7 MB | 1 | |
| common, css/support, html/resources | - | - | - | ~1 MB | 1 | shared helper scripts the tests include (the probe found tests needing them) |
| fetch, xhr | 487, 332 | 948, 442 | 255, 345 | 10 MB | 2 | mostly need Python handlers (`.py`) |
| pointerevents | 188 | 262 | 14 | | 2 | needs testdriver |
| css/css-transitions, css/css-animations, web-animations | 128, 131, 144 | | 126, 132, 141 | | later | animation model |
| media, html/semantics/embedded-content/media-elements | | | | | never (archived) | DECISIONS 2026-11-21 |

"Need no server" in the table is the manifest-level filter (no `.https.`, `.sub.`, worker variant
or testdriver). A content scan of the phase-1 directories (`.py` references, cross-origin helpers,
`window.open`) puts the share that runs on a plain static server at 95-100 % for dom/*, css/cssom*,
the observers, uievents, url, encoding, webidl and streams; 82 % for webmessaging; 77 % for
html/semantics/scripting-1; 43 % for fetch/api; 35 % for xhr.

#### 2.2 How testharness.js reports, and how scratchy collects it

Every test page loads `/resources/testharness.js` and then `/resources/testharnessreport.js`.
The second file is the **vendor hook**: in the WPT repo it is nearly empty, and each browser's
runner serves its own version. testharness.js calls back into it:

- `add_result_callback(fn(test))`: after each check ("subtest"): `test.name`, `test.status`
  (0 PASS, 1 FAIL, 2 TIMEOUT, 3 NOTRUN, 4 PRECONDITION_FAILED), `test.message`.
- `add_completion_callback(fn(tests, harness_status, asserts))`: once, at the end. `harness_status.status`
  is 0 OK, 1 ERROR (an uncaught exception outside a subtest), 2 TIMEOUT (the file did not finish in
  10 s, or 60 s with `<meta name="timeout" content="long">`), 3 PRECONDITION_FAILED.
- `setup({...})` from the hook can set `explicit_timeout`, `timeout_multiplier`, `output: false`
  (do not build the HTML results table, which costs layout per test).

scratchy's runner serves its own `testharnessreport.js` (the server maps that one path to a file in
`tools/wpt/`), which calls `setup({output: false})` and registers a completion callback that hands
`JSON.stringify({harness, message, tests: [[name, status, message]...]})` to a native,
`__scratchy_wpt_done`, installed only in runner mode. The child prints it as one line and exits. The
probe did exactly this with `console.error('WPTRESULT ...')`; it worked for 1,496 of 1,522 files
(of the 26 without a result, 16 are standalone SVG/XHTML documents, which scratchy parses as HTML;
the other 10, e.g. the scroll-behavior and characterSet-normalization tests, are for the DOM and
geometry triage items).

#### 2.3 Timeouts and isolation

- **Virtual time.** The child runs the page on a virtual clock like `--js-report --run-timers`:
  when nothing can run now, time jumps to the next timer. A test that waits forever for an event
  therefore reaches testharness's own 10 s / 60 s timeout in milliseconds (measured: 111 files
  timed out in the probe at ~80 ms each). One rule is new: the clock must NOT jump while a
  network/file load is in flight (otherwise a slow load becomes a false TIMEOUT); the child
  advances only when the task queue is empty AND no load is pending.
- **Wall cap.** A CPU-bound test still burns real time (`Event-timestamp-safe-resolution.html`:
  5.1 s, a busy loop). The parent kills a child after 30 s of real time and records `TIMEOUT`.
- **One process per test**, a pool of `--jobs N` (default: number of CPUs, 4 on CI). Measured cost
  per file: median 81 ms, p90 131 ms, p99 165 ms, of which ~55-60 ms is process start (an empty
  page: 59 ms). This gives full isolation for free (localStorage, cookies, the many thread-locals,
  a V8 crash or a 6 GB memory cap only kill one test) and needs no "reset between tests" code.
  Reusing a process for many tests would be ~3x faster but needs a reliable reset of every global;
  not worth it at these numbers. Crash = the child died without a result: status `CRASH`.
- The viewport is 800x600 (WPT's default), fonts as in the reftest runner (Ahem added), and the
  window/document reports `visibilityState = visible`.

#### 2.4 Serving the files: scratchy opens no sockets

testharness tests need real URLs: root-relative paths (`/resources/testharness.js`), a real origin
(`http://web-platform.test:8000`, not `file:`: file pages have an opaque origin, which breaks
`postMessage` origins, storage, `document.domain` and same-origin checks), helper files generated
by the server (`/resources/WebIDLParser.js` is an alias, `x.any.js` is served as a generated
`x.any.html` wrapper), per-file headers (`x.html.headers`), `{{host}}`-style substitution in
`.sub.` files, and for some tests Python request handlers (`.py`).

Options:

1. **`file://`**: no. Wrong origin, no root-relative paths, no `.any.html` wrappers.
2. **In-process loader inside scratchy** (no wire): quick, but every resource path in scratchy
   (`fetch_scripts`, link CSS, images, iframes, `js_net` fetch/XHR) would need a second backend, and
   network semantics that live in wire (cookies, redirects, target classes) would not be tested.
3. **A static mount in wire (recommended for phase 1)**: `wire --mount http://web-platform.test:8000=DIR`
   (plus `www.`, `www1.`, `www2.` subdomains and port 8001 as aliases of the same tree so
   cross-origin tests have a second origin). wire already serves `file:` URLs (`serve_file` in
   wire/src/main.rs); this is the same reader behind an http origin, plus `.headers`, the
   `.any.js`/`.window.js` wrappers, the `WebIDLParser.js` alias, `.sub.` substitution of the
   handful of `{{...}}` keys, and a fixed override for `/resources/testharnessreport.js` and
   `/resources/testdriver-vendor.js`. `.py` -> `501` plus one log line, recorded as
   `NEEDS-SERVER` and excluded from the score. The runner starts its own private wire on its
   own socket (as tools/incr-gate/run.sh does; never /run/user/1000/wire.sock). scratchy keeps
   talking only to wire; wire opens no socket for a mounted origin.
4. **wptserve (phase 2)**: WPT's own Python server, started from the checkout (`./wpt serve`,
   needs `tools/` in the sparse list and Python 3, present on the CI image), listening on
   127.0.0.1. wire reaches it like any loopback target (policy class `Warn`) through a host map
   `wire --map-host '*.web-platform.test=127.0.0.1'`. Full fidelity, `.py` handlers included.
   Needed for fetch/xhr/cookies and 119 scripting-1 files. User decision Q2.
5. **https** (phase 3): wptserve serves TLS with its own test CA; wire would have to trust that CA
   for `*.web-platform.test` only. User decision Q8.

**Variants:** `.window.js` and the window variant of `.any.js` are feasible now (served as
generated HTML). `.worker.js`, `.any.worker.html`, `.sharedworker` and `.serviceworker` are NOT
feasible: scratchy has no `Worker` at all (archived, lower priority than MSE per DECISIONS). The
runner lists them as `NOT-RUN (no workers)`; they are not in any score.

**testdriver:** tests that need real input (`test_driver.click`, `send_keys`, `action_sequence`)
call `/resources/testdriver-vendor.js`, which each browser implements. Phase 2: scratchy's version
turns these into the same synthetic input the App gets from winit, which needs the headless
rendering loop (QEL1) for hit testing. Unlocks 77 dom/events files, 57 uievents, most of
pointerevents.

#### 2.5 Expectations files

Like `wpt-expected.txt`, but one file per top-level area so five agents improving different areas
do not conflict on one file:

```
scratchy/wpt-th/subset.txt          # paths, one per line (+ "# area:" comments); generated from wpt-dirs.txt
scratchy/wpt-th/expected/dom.txt    # generated by --update
scratchy/wpt-th/flaky.txt           # paths whose results differ between runs: run, never compared
scratchy/wpt-th/areas.txt           # the score table (2.7), generated, committed
```

Format of an expected file: one line per test file, then one indented line per subtest that does
NOT pass (passing subtests are implied, which keeps the file small and stable):

```
OK      131/134  dom/nodes/Node-cloneNode.html
  FAIL           Node.cloneNode on an SVG element keeps the namespace
TIMEOUT   0/1    html/webappapis/animation-frames/callback-exception.html
  TIMEOUT        requestAnimationFrame callback exceptions are reported to error handler
NEEDS-SERVER     fetch/api/basic/request-headers.any.js
```

Subtest names are used verbatim (tabs/newlines escaped); a repeated name gets ` #2`. Gate rules
(same spirit as the reftest rule "no pass->fail"):

- REGRESSION (blocks): a passing subtest no longer passes; a file status OK -> ERROR/TIMEOUT/CRASH;
  a file that ran now has no result.
- NEWLY PASSING: must be recorded (`--update`) in the same commit, as for reftests.
- CHANGED (subtests appear or disappear, e.g. names that contain random values): fix the name
  normalisation or move the file to flaky.txt with a one-line reason.
- New tests in the subset start as UNRECORDED and are recorded by `--update`.

#### 2.6 The probe (what was measured, how)

Not committed (throwaway scripts in the scratchpad). From the manifest, every testharness file under
dom, html/webappapis, css/cssom, css/cssom-view, resize-observer, intersection-observer, uievents,
url, encoding (minus the slow legacy-mb set), webidl, hr-time and streams whose source has no
`.py`, iframe, `window.open`, testdriver, fetch/XHR, worker, stylesheet link or module script: 1,522
files. Each `<script src>` was inlined from a sparse checkout of the pin, a completion hook added after
testharnessreport.js, and the page run with `scratchy --js-report --run-timers 30000` under the 6 GB
scope, 4 at a time. Results (subtests pass %; "score" = mean over files of passed/total, the wpt.fyi
way, so a file with 11,000 subtests does not outweigh 100 small files):

| area | files | no result | harness timeout | files all-pass | subtests | pass % | score |
|---|---|---|---|---|---|---|---|
| dom/nodes | 339 | 16 | 32 | 106 | 4,257 | 62 % | 43 % |
| dom/events (no testdriver) | 89 | 0 | 7 | 31 | 656 | 47 % | 49 % |
| dom/ranges | 58 | 0 | 1 | 9 | 264 | 8 % | 16 % |
| dom/traversal, lists, abort, collections, misc | 46 | 0 | 0 | 18 | 454 | 76 % | 52 % |
| dom/observable (tentative) | 25 | 0 | 0 | 0 | 242 | 0 % | 0 % |
| html/webappapis | 196 | 1 | 17 | 41 | 818 | 60 % | 25 % |
| ... of which timers | 14 | 0 | 3 | 10 | 23 | 70 % | 75 % |
| ... animation-frames | 11 | 0 | 2 | 9 | 15 | 60 % | 82 % |
| ... microtask-queuing | 3 | 0 | 1 | 2 | 8 | 88 % | 67 % |
| ... scripting (event loops, errors) | 74 | 0 | 7 | 13 | 192 | 39 % | 24 % |
| ... dynamic-markup-insertion (document.write/open) | 81 | 1 | 3 | 2 | 111 | 3 % | 3 % |
| css/cssom | 214 | 2 | 12 | 40 | 1,821 | 54 % | 30 % |
| css/cssom-view | 200 | 5 | 14 | 41 | 1,215 | 40 % | 25 % |
| resize-observer | 34 | 1 | 4 | 9 | 45 | 31 % | 31 % |
| intersection-observer (v1) | 85 | 0 | 2 | 16 | 118 | 31 % | 21 % |
| intersection-observer/v2 (occlusion) | 35 | 0 | 13 | 0 | 39 | 0 % | 0 % |
| uievents (no testdriver) | 16 | 0 | 1 | 6 | 114 | 61 % | 46 % |
| webidl | 38 | 0 | 3 | 8 | 269 | 55 % | 40 % |
| url | 25 | 0 | 1 | 15 | 562 | 86 % | 75 % |
| encoding (no legacy-mb) | 37 | 1 | 0 | 7 | 11,313 | 35 % | 37 % |
| streams | 79 | 0 | 4 | 3 | 1,191 | 20 % | 11 % |
| hr-time | 6 | 0 | 0 | 0 | 11 | 27 % | 15 % |
| **total** | **1,522** | **26** | **111** | **350** | **23,389** | **44 %** | **31 %** |

Time: 39 s wall at 4 jobs (150 s of CPU in total). Most frequent failures (subtests): 6,800 from
missing single-byte encodings in `TextDecoder` (windows-125x, ISO-8859-x, KOI8, macintosh: one gap),
317 `WritableStream is not defined`, 126 "byte streams are not implemented", 118 `TransformStream is
not defined`, 134 `Observable is not defined` (tentative spec), 106 `expected null but got ""`, 102
`document.charset` undefined, 101 `expected "DIV" but got "div"` (tagName case), 75
`document.createProcessingInstruction is not a function`. These are the kind of findings the area
triage items (section 6) turn into fixes. The probe ran headless with `--js-report`, which has no
rendering steps (rAF is a plain timer there): the real runner should run the App's rendering steps
headless (QEL1), so numbers for rAF/observers/scroll will move.

#### 2.7 CI cost

| step | time | how known |
|---|---|---|
| build | 0 extra | the scratchy job already builds the release binary |
| wire binary | ~5-20 s | the wire job builds in 5 s with a warm cache; the scratchy job would build it too |
| WPT checkout | ~5-10 s with actions/cache, ~35 s cold | measured cold locally (33 s, 227 MB) |
| reftests (`--wpt`) | ~2 s | measured 0.9 s locally |
| testharness phase 1, ~2,300 files | ~1.5-2.5 min on the 4-vCPU runner | measured 1,522 files = 150 CPU-s locally; estimate for the slower runner |
| expectations compare + score table | seconds | |

So **+2 to 3.5 minutes** on the 3 m 40 s scratchy job (or a separate parallel job: no wall-clock
increase but a second build, ~2 min of runner time). The 1,228 "long" legacy-encoding variants and
anything over 5 s go to a nightly/manual workflow, not to every PR. Risk to check in QWPT2: the
reftest expectations were recorded with the user's fonts; CI only has Liberation + Ahem.

### 3. The per-area report and the order of areas (item b)

#### 3.1 The report

`scratchy/wpt-th/areas.txt`, generated by every `--wpt-th` run and committed when it changes:

```
# wpt-th areas at WPT ac78b56; scratchy <commit>; score = mean over files of passed/total subtests
# area                       files  ran  all-pass  subtests   pass   pass%  score
dom/nodes                      339  323       106      4257   2659    62.5   43.0
dom/events                      89   89        31       656    309    47.1   49.0
...
TOTAL                         2312 2280       ...
```

- Headline number per area = **score** (wpt.fyi's per-test scoring); subtest pass % next to it.
- Areas = the directory groups of `wpt-th/subset.txt` (`# area:` comments), not raw directories,
  so "HTML event loop" can include resize-observer and intersection-observer.
- **Across PRs:** CI writes the table for the PR and a diff against main's committed table into the
  job summary (`$GITHUB_STEP_SUMMARY`), e.g. `dom/ranges 16.0 -> 22.5 (+12 subtests)`. The PR body
  quotes the changed rows. History is `git log -p scratchy/wpt-th/areas.txt`; a 20-line
  `tools/wpt-trend.sh AREA` prints the score per commit from it. No separate database.
- Each area item in TODO.txt states a target ("dom/ranges score >= 60") so "done" is measurable.

#### 3.2 Dependency order (with reasons)

The proposal was DOM core -> events -> HTML event loop / rendering steps -> CSSOM -> geometry ->
fetch/streams -> media. The evidence changes it slightly:

1. **Infrastructure** (runner, server, CI, expectations). Everything else is measured with it.
2. **DOM core** (dom/nodes, ranges, traversal, collections, lists, abort, domparsing): every test
   in every area builds its fixture with DOM calls, and testharness.js itself uses them, so DOM
   bugs leak into all other areas' numbers. Probe: ranges 16 %, collections 21 %, nodes 43 %.
3. **WebIDL bindings + events** (webidl, dom/events, uievents constructors; trusted input after
   testdriver). NEW: WebIDL moves up. The harness's own assertions (`assert_throws_dom`,
   `instanceof`, "Illegal invocation", property attributes) depend on binding rules, and
   idlharness tests in every area measure them. Probe: webidl 40 %.
4. **HTML event loop and rendering steps** (html/webappapis timers, microtask-queuing,
   animation-frames, scripting/event-loops and error reporting, PLUS resize-observer and
   intersection-observer). NEW: the observers belong here, not in "geometry": they are steps of
   "update the rendering", and section 4 shows their timing is the main defect (RO never re-fires,
   IO evaluated at timer boundaries). Their size/geometry part overlaps step 7.
5. **HTML scripting and parsing model** (html/semantics/scripting-1, html/webappapis/dynamic-markup-
   insertion, later html/syntax). NEW as a separate step: scratchy runs every script after the
   whole document is parsed (`run_page`, deviation E13), document.write tests score 3 %. This is a
   structural difference that real sites hit; it needs its own design item, after the event loop.
6. **CSSOM** (css/cssom): independent of the event loop; can run in parallel with 4-5. Probe 30 %.
7. **Geometry** (css/cssom-view): depends on layout correctness (the reftest work) and on scroll
   events (step 4). Probe 25 %.
8. **Encoding, URL, streams, hr-time**: pure JavaScript, no server, big cheap wins (one encoding
   gap = 6,800 subtests; WritableStream/TransformStream missing). NEW: streams move before fetch,
   because fetch bodies are streams. Can be done any time by a Sonnet in parallel.
9. **fetch / XHR / cookies / EventSource / WebSocket**: needs wptserve (phase 2) and https
   (phase 3); cookies and CORS partly live in wire.
10. **Media**: archived (DECISIONS 2026-11-21); out of the plan.

Steps 2-3 and 6 and 8 can overlap (different files, different agents); 4 depends on QEL1; 5 depends
on 4.

### 4. Event loop audit against HTML "update the rendering" (item c, read-only)

Terms: a **task** is one unit of work from a queue (a timer callback, an event from input, a
network result). After every task, and after every callback when the JavaScript stack becomes
empty, the browser runs a **microtask checkpoint** (all queued promise reactions,
`queueMicrotask`, MutationObserver deliveries). Roughly every 16.7 ms, when there is something to
show, the browser runs **"update the rendering"**, in this order: resize events -> scroll events
-> media-query `change` events -> CSS/Web animations and their events -> rAF callbacks (all with
the same timestamp) -> style + layout -> ResizeObserver (gather, deliver, repeat while deeper
elements changed) -> focus fix-up -> IntersectionObserver evaluation (callbacks are then queued as a
task) -> paint. A **forced layout** is script reading geometry (`getBoundingClientRect`,
`offsetWidth`) when style/layout are stale: the browser runs style+layout immediately.

What scratchy does (window mode, one winit loop iteration):

1. Input events from winit are dispatched to JS immediately (`App::window_event` -> `App::js_dispatch`
   -> `JsEngine::dispatch_event` -> `__scratchy_dn`), each as one task with a checkpoint after it.
2. `RedrawRequested` -> `App::redraw`: if dirty, `rebuild_layout` / `rebuild_live` (the incremental
   engine; big updates go to the shaping worker), then paint what is laid out, then present.
   `install_laid` calls `JsEngine::rendered_geometry` (IntersectionObserver may evaluate on this
   layout).
3. `App::about_to_wait`: `flush_pointer_move`; `sync_node_scroll` + `JsEngine::set_scroll` (scrollY
   only); `JsEngine::set_viewport` (`__scratchy_setvp`: fires `resize`); `JsEngine::run_due_slice`
   (all due timers, rAF callbacks and observer flushes, in due-time order, max 100 per turn, a 16
   or 50 ms slice: `timer_slice`); `App::js_sync` (DOM writes become `layout_dirty` and a redraw);
   `App::anim_tick` (CSS animation events); then sleep until the next deadline.

Headless (`--js-report`, `--perf-report` and the probe): only scripts, timers and network on a
simulated clock (`js_report_for_scripts_timers`); no rendering step at all.

Correct today: a checkpoint after every script, timer, rAF callback and observer flush
(`eval_value_as` -> `drain_after_task` -> `Engine::drain_jobs` -> `perform_microtask_checkpoint`);
MutationObserver delivered as a microtask in creation order (`moNotify`; WPT
MutationObserver-callback-order passes); a rAF requested inside rAF runs in a later frame (WPT
nested-raf-not-sync passes); queueMicrotask interleaving (WPT queue-microtask.window.js passes);
`resize` coalesced to one per frame; forced layouts exist and go through the same incremental
engine as the App's frames (QINC1i), which is the right model.

Deviations (E = event-loop finding; WPT evidence from the probe where it exists):

| # | Deviation | Where (functions) | HTML says | Evidence |
|---|---|---|---|---|
| E1 | rAF is a timer: due at the next 60 Hz boundary of the script clock, run by `__scratchy_timers_run_one` mixed with ordinary timers; not tied to a frame (runs while a layout is in flight, when nothing changed, when the window is hidden); a batch can be split by `timer_slice` / `MAX_TIMERS_PER_TURN`, with a paint in between | `TIMERS_PRELUDE` `add(.., raf)`, `JsEngine::run_due_slice`, `App::about_to_wait` | rAF callbacks are a per-document list run inside "update the rendering", all at once, before style/layout | design |
| E2 | rAF ids share the timer id counter; each rAF callback gets `now` at its own start, not the frame time | `TIMERS_PRELUDE` (`nextId`, `t.fn.apply(.., [now])`) | own id space; one timestamp per frame | WPT no-interference-timers FAIL ("expected 1 but got 3") |
| E3 | Rendering-step order: CSS animation events after rAF; no scroll events; no MQL change; RO/IO not after layout | `App::about_to_wait` (`run_due_slice` before `anim_tick`) | animations+events -> rAF -> layout -> RO -> IO | probe page below |
| E4 | ResizeObserver reports the `width`/`height` attributes (0x0 for a div) once, at `observe()`, via a setTimeout(0) task; never again on a size change; no depth loop, no "loop completed with undelivered notifications" error | `RO.prototype.observe` (`attrPx`), `obsSchedule`, `obsFlush` in js_dom.rs | gather sizes after layout every rendering update, deliver, repeat for deeper targets | probe: 100x50 div -> `ro:0x0`; width 100->200 px -> no callback; WPT RO score 31 % (archived QRO1) |
| E5 | IntersectionObserver evaluated at timer boundaries: `__scratchy_ioTick` runs before AND after every timer callback, a deferred `ioDueT` timer 100-2000 ms after a geometry-only change, or `__scratchy_ioRendered` after an App layout | `__scratchy_timers_run_one`, `ioNudge`, `ioRecheck`, `JsEngine::rendered_geometry` | evaluate once per rendering update (after RO, before paint) with the frame time; then queue a task for the callbacks | probe order below; WPT IO v1 score 21 % |
| E6 | Trusted events dispatched from Rust run every listener inside ONE eval: promise jobs queued by listener 1 run after the last listener | `JsEngine::dispatch_event` -> `__scratchy_dn` -> `dispatch` | a microtask checkpoint after each listener (stack empty) for UA-dispatched events | design; WPT's trusted variants need testdriver |
| E7 | Exceptions thrown by timer and rAF callbacks are logged, not "reported": no `error` event at window, `onerror` never called; same for observer callbacks | `__scratchy_timers_run_one` catch -> `__scratchy_err`; `obsErr` | "report the exception": ErrorEvent at the global, then console | WPT animation-frames/callback-exception and timers/errors TIMEOUT |
| E8 | Every setInterval is clamped to >= 4 ms | `TIMERS_PRELUDE` `add` (`if (repeat \|\| level > 5)`) | clamp only when the nesting level is > 5 | WPT timers/setinterval-settimeout-clamping FAIL |
| E9 | postMessage / MessageChannel messages are setTimeout(0) timers; requestIdleCallback is setTimeout(1) with a guessed 50 ms deadline | globals_prelude.js `tm`, `requestIdleCallback` | message tasks have their own task source; idle callbacks run in idle periods | design (no 4 ms clamping seen in 12 hops) |
| E10 | User scrolling (wheel, keys, scrollbar) only updates `scrollY`: no `scroll` event at window or elements, no `scrollend`, no `wheel` events; element `scroll` events only for script scrolls, as a timer task | `JsEngine::set_scroll`, `gScrollEvent` | scroll events fired in the rendering steps, coalesced per frame | WPT scrollend tests fail |
| E11 | `MediaQueryList` `change` never fires | js.rs `matchMedia` | evaluated in the rendering steps | code comment |
| E12 | `getComputedStyle` runs its own cascade with no layout: `width`/`height` return the specified value (`auto`), not the used px | js_style.rs (`StyleCache`) | resolved value of width/height = used value for rendered elements | part of the css/cssom failures |
| E13 | All classic scripts run after the whole document is parsed (scripts see the full DOM), then modules, DOMContentLoaded, iframes and load, synchronously | `run_page` | parser-blocking scripts run during parsing; DOMContentLoaded and load are separate tasks | WPT dynamic-markup-insertion 3 % |
| E14 | `visibilityState` is always `visible`; rAF and rendering never throttled | js.rs (`visibilityState` getter) | hidden documents skip rendering updates | design |
| E15 | Paint may show a partly shaped layout (progressive shaping on the worker), so painted geometry can differ from script geometry for a few frames | `App::redraw`, `install_laid` (incomplete ticks) | layout completes before paint | deliberate (perf); keep, document |

Probe (headless, simulated clock) for E3/E4/E5, one page that registers RO + IO on a 100x50 div,
then `setTimeout(0)`, then two rAFs, then resizes the div at 100 ms:

```
scratchy:  ro:0x0, io:true, timeout0, micro-after-timeout0, raf1, micro-after-raf1, raf2 (same ts), raf-next-frame, resized   (no second ro)
expected:  timeout0, micro-after-timeout0, raf1, micro-after-raf1, raf2, ro:100x50, io:true, raf-next-frame, ..., resized, ro:200x50
```

**QEL1 (2026-11-21): E1, E2, E3 fixed.** `src/event_loop.rs` `update_the_rendering` runs the steps in
the HTML order for the App and every headless mode (NOTES.md "Rendering steps"); rAF has its own list
and id space, one batch per frame with one timestamp, never split by the timer slice; rAF exceptions
are reported at the callback's global (the timer/observer part of E7 stays QEL4). Scroll and MQL
steps are hooks (E10/E11, QEL7); RO/IO hooks sit after style/layout (E4/E5, QEL2/QEL3). The probe
above now gives `ro:0x0, io:true, timeout0, micro-after-timeout0, raf1, micro-after-raf1, raf2 (same
ts), raf-next-frame, resized`: the task/microtask/rAF part is the HTML order (test
`qel1_order_probe_task_microtask_raf_batch`); the leading `ro:0x0`/`io:true` are E4/E5.

Forced layouts: geometry reads update the incremental engine on the main thread (`js_geom`
`refresh_engine`), cached until the next DOM write; that matches HTML's "update style and layout
when needed". The deviations above are about WHEN things run, not about forced layouts.

### 5. Missing-API report for real sites (item d)

Goal: for a real page, list the web APIs it touched that scratchy does not have, ranked by how
many sites need them, so area work follows real demand.

What to catch, and how cheaply (V8):

1. **Calls and reads of members that do not exist** (`el.animate(...)`, `navigator.userActivation`)
   and **feature probes** (`'IntersectionObserver' in window`, `typeof ResizeObserver`,
   `if (window.CSS && CSS.highlights)`). Mechanism: in report mode only, insert ONE forwarding
   `Proxy` at the very end of the prototype chains of window and of the DOM objects (between
   `EventTarget.prototype` and `Object.prototype`). A lookup reaches it only when the name was not
   found anywhere on the object or its DOM prototypes, i.e. exactly a miss. Its `get` and `has`
   traps record `(name, receiver interface, kind get|has, first calling script URL)` and forward to
   `Object.prototype`, so every answer the page gets is unchanged (`'x' in el` stays false, a read
   stays `undefined`). Global misses go the same way: an undeclared `Foo` or `typeof Foo` makes V8
   ask the global object's chain. No V8 API change and no snapshot change is needed (an
   alternative, a V8 non-masking named interceptor on the global template, would need the template
   baked into the startup snapshot and its callbacks in `external_refs`; the report mode can use the
   existing slow-start path instead, but the Proxy covers DOM prototypes too).
   Noise filter: pages read their own expando names (`el.__reactFiber...`); names are classified
   against a list of standard members generated from WPT's `interfaces/*.idl` (363 files at the
   pin; `resources/webidl2` parses them) into a committed JSON (interface -> members). Standard
   name = "missing API"; other names = ignored (counted only).
2. **Uncaught errors that name a missing API**: V8 messages `X is not a function`, `Cannot read
   properties of undefined (reading 'y')`, `Foo is not defined` are already logged; the report
   groups them by the named member.
3. **Existing stubs**: 22 `not implemented` log lines in the preludes (SubtleCrypto, javascript:
   URLs, iframe navigation, ...) count as "present but stubbed".
4. **CSS feature probes and drops** (always-on counters, they only run on a parse failure):
   declarations dropped for an unknown property or invalid value (by property name), rules dropped
   because a selector did not parse (by pseudo-class, e.g. `:has()`), unknown at-rules, `@supports`
   / `CSS.supports()` that returned false (by condition), unknown media features in `matchMedia`.

Cost: zero outside report mode by construction (nothing is installed). In report mode, every miss
on a DOM object goes through a Proxy trap, and V8 stops caching lookups through that chain, so JS
runs slower; the first item must measure it on the 7 gate pages (`--perf-report` JS time with and
without, acceptance: report mode at most 2x JS time, normal mode byte-identical behaviour). The
CSS counters are a few increments per dropped declaration (Wikipedia/YouTube drop at most
thousands per load): unmeasurable.

Output: `scratchy --missing-api-report [--socket S] [--seconds N] URL...` prints one JSON object per
page (globals, members, errors, stubs, css). `tools/missing-api/corpus.sh` runs a site list (the
gate pages plus a user list, through replay recordings when they exist) and merges into a table
"API -> number of sites, total hits, example URL", which goes into the area triage items.

### 6. Offline recorded pages for deterministic perf runs (item e)

Today `tools/incr-gate/run.sh` loads live YouTube, Wikipedia, github, bbc and social.loupax.com.
The pages change daily (new scripts, A/B tests, ads), so a slower run can be the site's fault, and
nothing can become a CI budget.

**Recording and replay in wire** (scratchy needs no change to use it, it already sends everything to
wire):

- `wire --record DIR`: for every request, store the response body once by content hash
  (`DIR/blobs/<sha256>`, zstd) and append one index line `{seq, t_ms, method, url, range,
  req_body_sha256, status, headers, body_sha256}` to `DIR/index.jsonl`. WebSocket: not recorded
  in v1 (replay refuses the upgrade; pages fall back or log).
- `wire --replay DIR`: answer from the index only, never the network. Matching, in order: exact
  `(method, url, request body hash, range)`; then the same with query parameters from a per-host
  ignore list removed (`tools/replay-rules.txt`, e.g. YouTube `rn`, `cpn`, `t`; cache-busters `_`,
  `cb`); then the n-th request to the same path. A range request inside a recorded range is
  served from that body. A miss answers 404 with `X-Wire-Replay: miss` and one log line; a gate
  run with more than a handful of misses is invalid. Latency: none, or a fixed per-request delay
  (`--replay-delay 5ms`), never the recorded one (recorded latency is the noise we remove).
  Cookies: the jar works as usual on replayed Set-Cookie headers; recordings are made logged out
  with a fresh jar so they hold no personal session.
- scratchy side (`--deterministic`, report modes only): V8 `--random-seed`, a seeded
  `crypto.getRandomValues`, `Date.now()` starting at the recording's time (pages check expiry
  timestamps), network results delivered in request order at turn boundaries (today fetch threads
  race).

**Size (estimate, to be measured by QREC3):** main HTML documents measured today: Wikipedia 160 KB,
bbc 64 KB, YouTube results 136 KB, watch 133 KB, social.loupax.com 9 KB. Subresources dominate:
YouTube's app script alone is ~10 MB of source (R23, R24), github's hero section
inserts two video files (R63). Estimate per page: Wikipedia 2-4 MB, bbc 4-8 MB, social < 1 MB,
github 10-25 MB (videos), YouTube results 15-25 MB, YouTube watch 20-30 MB plus 10-40 MB of video
for a 56 s run. Total ~60-130 MB raw, ~40-90 MB after zstd (video does not compress). The
repository is PUBLIC: copies of third-party pages and their scripts must not be committed (user
decision Q5).

**Budgets to set first** (file `perf-budgets.txt`: metric, page, mode, limit, where; checked by a
small `tools/budget-check.py` over the existing `--perf-report` JSON and `summarize.py` rows):

| budget | first limit | where | why |
|---|---|---|---|
| peak RSS, headless 20 s, github home | <= 1.5 GB | CI + local | R63/QOOM1 grew to 15 GB |
| peak RSS, headless 30 s, YouTube watch | baseline + 20 % | CI + local | memory regressions are cheap to catch |
| script forced layouts, headless: count per page | baseline + 10 % | CI + local | deterministic with replay + `--deterministic` |
| largest callback after load (ms, budget clock) | <= 300 ms YouTube pages | local (perf lock) | R56 measured 130-264 ms |
| App build ms per run, window, Wikipedia hover plan | baseline + 15 % | local (nested dwl) | QINC1k3 gate |
| hover input p95 (window, hover.html + wiki) | <= 50 ms | local | QY53n5 gate |
| YouTube results: masthead finished / 20 results in | <= 6 s / <= 6 s | local | R56: 4.3 s / 3.3-4.4 s |

CI (no compositor with input; the shared runners are noisy) checks only counts and memory, on
replayed pages if the user allows CI access to recordings (Q5), else on synthetic pages committed
to the repo (hover.html, img-geom.html, a generated "article with floats" page). Wall-clock
budgets stay local, under the perf lock, with 3 interleaved runs and the median.

### 7. User decisions (questions, with my recommendation)

- **Q1. Test server for phase 1.** (a) a static mount inside wire, (b) WPT's Python server from day
  one, (c) an in-process loader in scratchy. *Recommendation: (a)*: no Python, no new socket,
  covers 95-100 % of phase 1; (b) later for fetch/xhr.
- **Q2. May WPT's Python server (wptserve) run locally and in CI, with wire fetching it on
  127.0.0.1 through a `*.web-platform.test` host map?** It is needed for fetch/xhr/cookies (the
  `.py` handlers). *Recommendation: yes, in phase 2, only for `*.web-platform.test`.*
- **Q3. Should testharness regressions block merges like reftests** (no passing subtest may start
  failing; newly passing ones recorded in the same PR; a flaky list for unstable files)?
  *Recommendation: yes, from the first CI run, with `wpt-th/flaky.txt`.*
- **Q4. CI time.** OK to add ~2-3.5 min to the scratchy job (one job), or prefer a separate parallel
  job (no longer wait, ~2 min more runner time)? *Recommendation: one job first; split only if it
  grows past 5 min.*
- **Q5. Where do page recordings live?** The repo is public; recordings are copies of third-party
  pages. (a) local only (`~/.cache/scratchy/recordings`), CI budgets on synthetic in-repo pages;
  (b) a private repository or private release asset that CI downloads with a token; (c) no
  recordings, keep live pages. *Recommendation: (a) now; (b) only if CI budgets on real pages prove
  necessary.*
- **Q6. Approve the area order in 3.2** (WebIDL moved up with events; observers inside the event
  loop step; a separate scripting/parsing step; encoding/url/streams before fetch)?
  *Recommendation: yes.*
- **Q7. Headline metric** = per-file score (mean of passed/total per file, like wpt.fyi), with
  subtest pass % beside it? *Recommendation: yes; raw subtest counts let one encoding file with
  11,000 checks dominate.*
- **Q8. HTTPS tests** need wire to trust WPT's test CA, for `*.web-platform.test` only. Now, later,
  or never? *Recommendation: later (with fetch), never for any other host.*

### 8. Follow-up items (item f), ready to paste into TODO.txt

Sizes as in TODO.txt (S < ~200 lines, M = several functions / new plumbing; nothing here is L, the
larger pieces are already split). "Opus" = needs design judgement, "Sonnet" = clear spec.
Recommended order: QWPT2 -> QWPT3 -> QWPT4a -> QWPT4b -> QWPT5 -> QWPT6, with QMAPI1 and QREC1 in
parallel (other crates/files); then QEL1 and the area triage items in the 3.2 order.

```
- [ ] QWPT2 [S] (P1; Sonnet; tools/, ci.yml, AGENTS.md; R64 2.1, 2.7) WPT checkout + pin + reftests in CI.
      tools/wpt-pin.txt (one line: ac78b567f5404b6f2dc0418ea380af3946a47fca; AGENTS.md points to it),
      tools/wpt-dirs.txt (R64 2.1 table: phase-1 dirs + css fonts images resources common css/support
      html/resources), tools/wpt-checkout.sh DIR (blobless depth-1 sparse clone of the pin; refuses a
      checkout whose HEAD differs). CI: actions/cache keyed on pin + dirs hash, then the existing
      `scratchy --wpt` reftest run (0.9 s locally). FIRST check that wpt-expected.txt holds with CI's
      fonts (Liberation + Ahem); if not, report the differing tests and keep reftests local (do not
      edit expectations to match CI without the coordinator).
- [ ] QWPT3 [M] (P1; Sonnet; wire only; R64 2.4 option 3; WAITS FOR USER Q1) wire static mount.
      `wire --mount http://web-platform.test:8000=DIR` (+ www/www1/www2 subdomains and :8001 as
      aliases): files with Content-Type by extension, `<file>.headers` and `__dir__.headers`,
      generated wrappers for `x.any.js` (served at x.any.html) and `x.window.js` (x.window.html, META
      script/title/timeout lines honoured), alias /resources/WebIDLParser.js ->
      /resources/webidl2/lib/webidl2.js, `.sub.` substitution of {{host}}, {{domains[www]}},
      {{ports[http][0]}}, {{ports[http][1]}}; `--mount-override PATH=FILE` (testharnessreport.js,
      testdriver-vendor.js); `.py` -> 501 + one log line. No socket for a mounted origin. Tests in wire.
- [ ] QWPT4a [M] (P1; Opus; new src/wpt_th.rs, js.rs report path; R64 2.2, 2.3; after QWPT3) Test child.
      `scratchy --wpt-th-one --socket S URL`: headless page load through wire, the vendor
      testharnessreport.js (setup({output:false}), completion callback -> native __scratchy_wpt_done,
      installed only in this mode), virtual clock that advances only when no task is runnable AND no
      load is pending, 800x600 viewport, Ahem font; prints ONE JSON line {harness, message, tests}
      or nothing (parent says CRASH). Tests: fixture pages in a mini checkout (pass, fail, harness
      error, timeout -> TIMEOUT in < 1 s real time, a pending fetch does not time out).
- [ ] QWPT4b [M] (P1; Sonnet; src/wpt_th.rs; R64 2.3, 2.5, 3.1; after QWPT4a) Runner parent.
      `scratchy --wpt-th <checkout> [--jobs N] [--update] [--area A]`: starts a private wire with the
      mount (own socket, never /run/user/1000/wire.sock; kills only its PID), pool of children, 30 s
      real-time cap per child (kill own PID -> TIMEOUT), reads wpt-th/subset.txt, writes/compares
      wpt-th/expected/<area>.txt (format R64 2.5), honours wpt-th/flaky.txt, writes wpt-th/areas.txt.
      Exit 1 on REGRESSION/UNRECORDED/CHANGED. Worker variants listed as NOT-RUN.
- [ ] QWPT5 [S] (P1; Sonnet; ci.yml; after QWPT2, QWPT4b, QWPT6; WAITS FOR USER Q3, Q4) CI step: build
      wire in the scratchy job, run `--wpt-th --jobs 4`, fail on regressions, write the areas table
      and its diff vs main into $GITHUB_STEP_SUMMARY. Measure and report the added minutes.
- [ ] QWPT6 [S] (P1; Sonnet; wpt-th/; after QWPT4b) Phase-1 subset and baseline: generate
      wpt-th/subset.txt from tools/wpt-dirs.txt + the WPT manifest of the pin (skip legacy-mb encoding
      variants and timeout=long files: nightly), run 3 times, put files whose results differ into
      flaky.txt with a reason, record expectations, commit areas.txt. Report the table in the PR.
- [ ] QWPT7 [M] (P2; Opus; testdriver-vendor.js + natives; after QEL1) testdriver: test_driver.click,
      send_keys, action_sequence (pointer + key) as synthetic input through the App's input path in
      the headless rendering loop (hit testing on the real layout). Unlocks dom/events (77 files),
      uievents (57), pointerevents.
- [ ] QWPT8 [M] (P2; Sonnet; tools/, wire --map-host; WAITS FOR USER Q2) Phase 2 server: tools/wptserve.sh
      (python3 ./wpt serve from a checkout that includes tools/), wire `--map-host
      '*.web-platform.test=127.0.0.1'`, runner `--server wptserve`; add fetch/ and xhr/ to the subset.
- [ ] QWPT9 [S] (P3; Sonnet; wire; WAITS FOR USER Q8) https tests: wire trusts WPT's test CA for
      *.web-platform.test only; add .https. tests.
- [ ] QEL1 [M] (P1; Opus; new src/event_loop.rs, app.rs, js.rs TIMERS_PRELUDE; R64 section 4 E1-E3; after
      QWPT4b so the change is measured) Rendering-steps driver shared by the App and the headless
      runner: one function runs, per frame: resize -> scroll events -> MQL change -> animation events
      -> rAF batch (own id space, one timestamp, never split by the timer slice) -> style/layout through
      the engine -> [RO, IO hooks] -> paint (App only). The App calls it at frame time (when something
      is dirty or a rAF is pending); headless runs it on virtual 16.7 ms frames. Keep today's cost
      tricks. Tests: WPT html/webappapis/animation-frames all pass; order probe (R64 4) gives the
      HTML order.
- [ ] QEL2 [M] (P1; Sonnet; js_dom.rs RO, geometry; after QEL1; supersedes archived QRO1) ResizeObserver
      with real sizes (content/border/device-pixel boxes from the engine's layout), gathered and
      delivered in the rendering step, depth loop and the "ResizeObserver loop completed with
      undelivered notifications" error event. Gate: resize-observer score up, github.com card grids
      no longer dropped (QINC1k5 finding).
- [ ] QEL3 [S] (P2; Sonnet; js_dom.rs IO; after QEL1) IntersectionObserver evaluated once per rendering
      update (after RO, before paint) with the frame time, callbacks queued as a task; remove the
      per-timer __scratchy_ioTick calls; keep the App-layout reuse. Gate: intersection-observer score
      not lower, YouTube thumbnails still load (QY57r).
- [ ] QEL4 [S] (P2; Sonnet; js.rs TIMERS_PRELUDE, js_dom.rs obsErr) "Report the exception" for timer,
      rAF and observer callbacks (ErrorEvent at window, onerror, then the log line). Gate: WPT
      animation-frames/callback-exception and timers/errors pass.
- [ ] QEL5 [S] (P2; Sonnet; js.rs TIMERS_PRELUDE) Clamp setInterval to 4 ms only at nesting level > 5.
      Gate: WPT timers/setinterval-settimeout-clamping.
- [ ] QEL6 [S] (P2; Sonnet; js.rs dispatch, __scratchy_dn) Microtask checkpoint between listeners of
      UA-dispatched (trusted) events. Test: two click listeners, a promise in the first resolves
      before the second runs; synthetic el.click() keeps today's order.
- [ ] QEL7 [S] (P2; Sonnet; app.rs, js.rs; after QEL1) scroll events for user scrolling (window and
      elements, coalesced per frame in the rendering step), scrollend, wheel events; MQL change events.
- [ ] QEL8 [S] (P3; Sonnet; globals_prelude.js) postMessage/MessageChannel on their own task queue (not
      the timer table); requestIdleCallback only when the loop is idle, deadline from the next timer
      or frame. Measure bbc.com (R62 MessagePort finding) before/after.
- [ ] QEL9 [M] (P2; Opus; DESIGN ONLY first; after QEL1 and QWPT6) Script processing model: running
      parser-blocking scripts during parsing (scripts see the partial DOM), defer/async ordering,
      DOMContentLoaded/load as tasks. Measure html/semantics/scripting-1 and dynamic-markup-insertion
      first; write the design for the user.
- [ ] QMAPI1 [M] (P1; Sonnet; new src/missing_api.rs + prelude; R64 section 5; independent) Missing-API
      report mode `scratchy --missing-api-report [--socket S] [--seconds N] URL...`: one forwarding
      Proxy at the end of the window and DOM prototype chains (get/has traps record misses, answers
      unchanged), IDL member list generated from WPT interfaces/*.idl (tool script + committed JSON),
      V8 error-message grouping, existing `not implemented` stubs; JSON per page. Measure the report
      mode's JS-time cost on the 7 gate pages (acceptance <= 2x) and prove normal mode installs
      nothing (test).
- [ ] QMAPI2 [S] (P2; Sonnet; css.rs counters; after QMAPI1) CSS part of the report: dropped declarations
      by property, dropped rules by unparsed pseudo-class/element, unknown at-rules, false
      @supports/CSS.supports conditions, unknown media features.
- [ ] QMAPI3 [S] (P2; Sonnet; tools/missing-api/; after QMAPI1) corpus.sh over a site list (gate pages +
      a list the user provides), merged table "API -> sites, hits, example URL" into an R-section.
- [ ] QREC1 [M] (P2; Sonnet; wire only; R64 section 6; independent) wire --record DIR / --replay DIR
      (blobs by sha256 + zstd, index.jsonl, matching exact -> ignore-list -> n-th same path, ranges,
      404 + X-Wire-Replay: miss, --replay-delay, WebSocket refused in replay). Tests in wire.
- [ ] QREC2 [S] (P2; Sonnet; js_engine_v8.rs, js.rs; after QREC1) scratchy --deterministic (report
      modes): V8 --random-seed, seeded crypto.getRandomValues, Date epoch from the recording, network
      results delivered in request order.
- [ ] QREC3 [S] (P2; Sonnet; tools/incr-gate; after QREC1, QREC2; WAITS FOR USER Q5) Record the 7 gate
      pages; run.sh/gate.sh REPLAY=DIR; measure recording sizes and 3-run variance of the summarize.py
      columns live vs replay; write the numbers into R64.
- [ ] QBUD1 [S] (P2; Sonnet; perf-budgets.txt, tools/budget-check.py; after QREC3) First budgets (R64 6
      table); local check in the AGENTS.md checklist for perf-touching PRs; CI only counts + RSS on
      synthetic or (per Q5) replayed pages.
- [ ] QAREA-DOM [M] (P1; Opus; docs only; after QWPT6) Triage the dom/* failures of the baseline by root
      cause (probe hints: tagName case, document.charset, createProcessingInstruction, Range type
      checks, replaceWith/splitText missing), file S fix items with target scores. Same pattern
      later: QAREA-IDL, QAREA-EL, QAREA-CSSOM, QAREA-VIEW, QAREA-ENC (single-byte encodings: one gap,
      ~6,800 subtests), QAREA-STREAMS.
```

Recommended first three: **QWPT2** (pin, checkout, reftests in CI: small, useful at once, needs no
decision), **QWPT3** (wire mount; after Q1), **QWPT4a** (the test child). With those, QWPT4b/6 give
the first committed baseline, and QMAPI1/QREC1 can start any time in parallel.

How this was measured (for re-runs): the WPT manifest for the pin was downloaded from the release
`merge_pr_65461`; counts come from it plus `git ls-tree` of the pinned tree. The probe pages were
built by inlining scripts from a sparse checkout under the worktree and run with the worktree's
release build: `systemd-run --user --scope -q -p MemoryMax=6G -p MemorySwapMax=0 nice -n 19 ionice
-c3 ... scratchy --js-report --run-timers 30000 <page>`, 4 at a time. The order probe and the
empty-page startup time used the same binary. No live site was loaded by scratchy.
