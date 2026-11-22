#[allow(unused_imports)]
use super::*;

// ---- QY6e: Custom Elements.

#[test]
fn qy6e_define_upgrades_existing_runs_ctor_once_and_connected() {
    let log = run_in_html(
        "<html><body><x-a id=a1 k=v></x-a><div><x-a id=a2></x-a></div></body></html>",
        r#"let n = 0;
         class XA extends HTMLElement {
           static get observedAttributes() { return ['k']; }
           constructor() { super(); n++; this.tag = 'ctor:' + this.id; }
           connectedCallback() { console.log('conn ' + this.id); }
           attributeChangedCallback(a, o, v) { console.log('attr ' + this.id + ' ' + a + ' ' + o + ' ' + v); }
         }
         const a1 = document.getElementById('a1');
         console.log(a1 instanceof XA, a1 instanceof HTMLElement, a1.tag);
         customElements.define('x-a', XA);
         console.log(n, a1.tag, document.getElementById('a1') === a1, a1 instanceof XA);
         console.log(customElements.get('x-a') === XA, customElements.getName(XA), customElements.get('x-b'));"#,
    );
    assert_eq!(
        log,
        [
            "log:false true undefined",
            "log:attr a1 k null v",
            "log:conn a1",
            "log:conn a2",
            "log:2 ctor:a1 true true",
            "log:true x-a undefined",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6e_create_element_new_and_innerhtml_construct() {
    let log = run_in_html(
        "<html><body><div id=host></div></body></html>",
        r#"let made = [];
         class XB extends HTMLElement {
           constructor() { super(); made.push('ctor'); }
           connectedCallback() { made.push('conn'); }
           disconnectedCallback() { made.push('disc'); }
         }
         customElements.define('x-b', XB);
         const e = document.createElement('x-b');
         console.log(e instanceof XB, e.localName, made.join());
         const n = new XB();
         console.log(n instanceof XB, n.localName, n.parentNode === null, made.join());
         made.length = 0;
         const host = document.getElementById('host');
         host.innerHTML = '<p><x-b></x-b></p>';
         console.log(host.querySelector('x-b') instanceof XB, made.join());
         made.length = 0;
         host.appendChild(e);
         console.log(made.join());
         made.length = 0;
         host.removeChild(e);
         console.log(made.join());
         made.length = 0;
         host.innerHTML = '';
         console.log(made.join());
         const frag = document.createDocumentFragment();
         frag.appendChild(document.createElement('x-b'));
         made.length = 0;
         host.appendChild(frag);
         console.log(made.join());
         made.length = 0;
         const c = host.firstChild.cloneNode(true);
         console.log(c instanceof XB, made.join());"#,
    );
    assert_eq!(
        log,
        [
            "log:true x-b ctor",
            "log:true x-b true ctor,ctor",
            "log:true ctor,conn",
            "log:conn",
            "log:disc",
            "log:disc",
            "log:conn",
            "log:true ctor",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6e_attribute_changed_only_for_observed() {
    let log = run_in_html(
        "<html><body><x-c id=c></x-c></body></html>",
        r#"class XC extends HTMLElement {
           static get observedAttributes() { return ['title', 'class']; }
           attributeChangedCallback(a, o, v) { console.log(a + ':' + o + '>' + v); }
         }
         customElements.define('x-c', XC);
         const c = document.getElementById('c');
         c.setAttribute('title', 'a');
         c.setAttribute('lang', 'x');
         c.setAttribute('title', 'b');
         c.className = 'k';
         c.classList.add('m');
         c.removeAttribute('title');
         c.removeAttribute('title');
         c.toggleAttribute('title');
         c.id = 'd';"#,
    );
    assert_eq!(
        log,
        [
            "log:title:null>a",
            "log:title:a>b",
            "log:class:null>k",
            "log:class:k>k m",
            "log:title:b>null",
            "log:title:null>",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6e_when_defined_resolves_and_errors() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"class XD extends HTMLElement {}
         customElements.whenDefined('x-d').then(function (c) { console.log('resolved ' + (c === XD)); });
         customElements.whenDefined('x-d').then(function () { console.log('second'); });
         console.log('before');
         customElements.define('x-d', XD);
         customElements.whenDefined('x-d').then(function (c) { console.log('already ' + (c === XD)); });
         customElements.whenDefined('bad').catch(function (e) { console.log('rej ' + e.name); });
         const names = [];
         const tryDef = function (n, C, o) { try { customElements.define(n, C, o); names.push('ok'); } catch (e) { names.push(e.name); } };
         tryDef('nohyphen', class extends HTMLElement {});
         tryDef('Has-Upper', class extends HTMLElement {});
         tryDef('font-face', class extends HTMLElement {});
         tryDef('x-d', class extends HTMLElement {});
         tryDef('x-e', XD);
         tryDef('x-f', 5);
         console.log(names.join());"#,
    );
    assert_eq!(
        log,
        [
            "log:before",
            "log:SyntaxError,SyntaxError,SyntaxError,NotSupportedError,NotSupportedError,TypeError",
            "log:resolved true",
            "log:second",
            "log:already true",
            "log:rej SyntaxError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6e_upgrade_and_is_and_attach_shadow() {
    let log = run_in_html(
        "<html><body><button id=b is=x-btn></button></body></html>",
        r#"class XBtn extends HTMLButtonElement { constructor() { super(); this.mark = 'built'; } connectedCallback() { console.log('conn'); } }
         customElements.define('x-btn', XBtn, { extends: 'button' });
         const b = document.getElementById('b');
         console.log(b instanceof XBtn, b.mark);
         const d = document.createElement('div');
         d.innerHTML = '<x-g></x-g>';
         class XG extends HTMLElement { constructor() { super(); this.up = 1; } }
         customElements.define('x-g', XG);
         console.log(d.firstChild.up);
         customElements.upgrade(d);
         console.log(d.firstChild.up);
         try { d.attachShadow({ mode: 'open' }); console.log(d.shadowRoot.mode); } catch (e) { console.log(e.name); }
         try { new HTMLElement(); } catch (e) { console.log(e.name); }"#,
    );
    assert_eq!(
        log,
        [
            "log:conn",
            "log:true built",
            "log:undefined",
            "log:1",
            "error:scratchy: js: not implemented: shadow root rendering (QSD6); shadow content is not rendered, only the host's light children",
            "log:open",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6e_zero_cost_without_define() {
    let log = run_in_html(
        "<html><body><div id=h></div></body></html>",
        r#"const h = document.getElementById('h');
         for (let i = 0; i < 20; i++) { const e = document.createElement('x-z'); e.setAttribute('a', i); h.appendChild(e); }
         h.innerHTML = '<x-z></x-z>'; h.textContent = 'x'; h.removeChild(h.firstChild);
         console.log(__ce_stats.hooks);
         customElements.define('x-y', class extends HTMLElement {});
         h.appendChild(document.createElement('div'));
         console.log(__ce_stats.hooks > 0);"#,
    );
    assert_eq!(log, ["log:0", "log:true"], "{log:?}");
}

#[test]
fn qy6e_document_write_upgrades() {
    let out = qa13_run(
        "<html><body><script>\
         class XW extends HTMLElement { connectedCallback() { console.log('w conn'); } }\
         customElements.define('x-w', XW);\
         document.write('<x-w></x-w>');\
         console.log('end');</script></body></html>",
    );
    assert_eq!(out, ["log:w conn", "log:end"], "{out:?}");
}

// ---- QY6a: MutationObserver ----

/// Shared JS helpers: `d(record)` summarises a record as
/// `type|target|added|removed|prev|next|attr|old`; `watch(node, opts)` returns
/// the observer and pushes one string per delivered callback (records joined
/// by ` ; `) onto `out`; `step(f, g, ...)` runs each function in its own
/// microtask turn, after the previous delivery.
const QY6A_JS: &str = r#"
const n = x => x ? (x.id || x.nodeName.toLowerCase()) : '-';
const ns = l => Array.from(l).map(n).join(',') || '-';
const d = r => [r.type, n(r.target), ns(r.addedNodes), ns(r.removedNodes), n(r.previousSibling), n(r.nextSibling), r.attributeName, r.oldValue].map(String).join('|');
const out = [];
function watch(t, o) { const mo = new MutationObserver(l => out.push(l.map(d).join(' ; '))); mo.observe(t, o); return mo; }
function step() { const fs = Array.from(arguments); const run = () => { const f = fs.shift(); if (f) { f(); Promise.resolve().then(run); } else Promise.resolve().then(() => globalThis.__fin && __fin()); }; Promise.resolve().then(run); }
const mk = (t, id) => { const e = document.createElement(t); if (id) e.id = id; return e; };
const $ = id => document.getElementById(id);
"#;

fn qy6a_run(body: &str) -> Vec<String> {
    run_in_html(
        "<html><body><div id=h><i id=a></i><b id=b></b></div><span id=s>text</span><!--c--></body></html>",
        &format!("{QY6A_JS}\n{body}"),
    )
}

/// Runs `body` and returns the delivered callbacks (one entry each).
fn qy6a_out(body: &str) -> Vec<String> {
    let mut log = qy6a_run(&format!("globalThis.__fin = () => console.log(out.join('\\n'));\n{body}"));
    assert_eq!(log.len(), 1, "{log:?}");
    let l = log.remove(0);
    let l = l.strip_prefix("log:").unwrap_or(&l).to_string();
    if l.is_empty() {
        Vec::new()
    } else {
        l.split('\n').map(String::from).collect()
    }
}

#[test]
fn qy6a_zero_cost_before_observe() {
    let log = qy6a_run(
        r#"const h = $('h');
         for (let i = 0; i < 20; i++) { const e = mk('p'); e.setAttribute('a', i); h.appendChild(e); }
         h.innerHTML = '<p></p>'; h.textContent = 'x'; h.removeChild(h.firstChild);
         console.log(__ce_stats.hooks);
         const mo = new MutationObserver(() => {});
         console.log(__ce_stats.hooks);
         mo.observe(h, { childList: true });
         h.appendChild(mk('u'));
         console.log(__ce_stats.hooks > 0);"#,
    );
    assert_eq!(log, ["log:0", "log:0", "log:true"], "{log:?}");
}

#[test]
fn qy6a_child_list_write_paths() {
    let o = qy6a_out(
        r#"watch($('h'), { childList: true });
         step(() => $('h').appendChild(mk('p', 'p1')),
              () => $('h').insertBefore(mk('p', 'p2'), $('a')),
              () => $('h').removeChild($('b')),
              () => $('h').replaceChild(mk('p', 'p3'), $('a')),
              () => $('p1').remove());"#,
    );
    assert_eq!(
        o,
        [
            "childList|h|p1|-|b|-|null|null",
            "childList|h|p2|-|-|a|null|null",
            "childList|h|-|b|a|p1|null|null",
            "childList|h|p3|a|p2|p1|null|null",
            "childList|h|-|p1|p3|-|null|null",
        ]
    );
}

#[test]
fn qy6a_multi_node_apis_make_one_record() {
    let o = qy6a_out(
        r#"watch($('h'), { childList: true });
         step(() => $('h').append(mk('p', 'p1'), 'txt', mk('p', 'p2')),
              () => $('h').prepend(mk('q', 'q1'), mk('q', 'q2')),
              () => $('a').before(mk('r', 'r1'), mk('r', 'r2')),
              () => $('b').after(mk('t', 't1'), mk('t', 't2')),
              () => $('a').replaceWith(mk('u', 'u1'), mk('u', 'u2')),
              () => $('h').replaceChildren(mk('v', 'v1')),
              () => { const f = document.createDocumentFragment(); f.appendChild(mk('w', 'w1')); f.appendChild(mk('w', 'w2')); $('h').appendChild(f); });"#,
    );
    assert_eq!(
        o,
        [
            "childList|h|p1,#text,p2|-|b|-|null|null",
            "childList|h|q1,q2|-|-|a|null|null",
            "childList|h|r1,r2|-|q2|a|null|null",
            "childList|h|t1,t2|-|b|p1|null|null",
            "childList|h|u1,u2|a|r2|b|null|null",
            "childList|h|v1|q1,q2,r1,r2,u1,u2,b,t1,t2,p1,#text,p2|-|-|null|null",
            "childList|h|w1,w2|-|v1|-|null|null",
        ]
    );
}

#[test]
fn qy6a_inner_html_and_text_content_one_record() {
    let o = qy6a_out(
        r#"watch($('h'), { childList: true });
         step(() => { $('h').innerHTML = '<p id=p1></p><p id=p2></p>'; },
              () => { $('h').textContent = 'plain'; },
              () => { $('h').textContent = ''; },
              () => { $('h').innerHTML = ''; });"#,
    );
    // `#text` is the one text node textContent created; an empty -> empty write queues nothing.
    assert_eq!(
        o,
        [
            "childList|h|p1,p2|a,b|-|-|null|null",
            "childList|h|#text|p1,p2|-|-|null|null",
            "childList|h|-|#text|-|-|null|null",
        ]
    );
}

#[test]
fn qy6a_outer_html_replaces_with_one_record() {
    let o = qy6a_out(
        r#"watch($('h'), { childList: true });
         step(() => { $('a').outerHTML = '<p id=x1></p><p id=x2></p>'; });"#,
    );
    assert_eq!(o, ["childList|h|x1,x2|a|-|b|null|null"]);
}

#[test]
fn qy6a_move_records_removal_and_insertion() {
    let o = qy6a_out(
        r#"const s = $('s'); watch($('h'), { childList: true }); watch(s, { childList: true });
         step(() => s.appendChild($('a')));"#,
    );
    assert_eq!(o, ["childList|h|-|a|-|b|null|null", "childList|s|a|-|#text|-|null|null"]);
}

#[test]
fn qy6a_attribute_records_and_old_value() {
    let o = qy6a_out(
        r#"const a = $('a'); watch(a, { attributes: true, attributeOldValue: true });
         step(() => a.setAttribute('x', '1'),
              () => a.setAttribute('x', '2'),
              () => a.removeAttribute('x'),
              () => a.removeAttribute('nope'),
              () => a.toggleAttribute('hidden'),
              () => { a.className = 'k'; },
              () => { a.classList.add('m'); },
              () => { a.id = 'a2'; },
              () => { a.dataset.foo = 'bar'; },
              () => { a.style.color = 'red'; });"#,
    );
    assert_eq!(
        o,
        [
            "attributes|a|-|-|-|-|x|null",
            "attributes|a|-|-|-|-|x|1",
            "attributes|a|-|-|-|-|x|2",
            "attributes|a|-|-|-|-|hidden|null",
            "attributes|a|-|-|-|-|class|null",
            "attributes|a|-|-|-|-|class|k",
            "attributes|a2|-|-|-|-|id|a",
            "attributes|a2|-|-|-|-|data-foo|null",
            "attributes|a2|-|-|-|-|style|null",
        ]
    );
}

#[test]
fn qy6a_old_value_only_when_requested() {
    let o = qy6a_out(
        r#"const a = $('a'); a.setAttribute('x', '1'); watch(a, { attributes: true });
         step(() => a.setAttribute('x', '2'));"#,
    );
    assert_eq!(o, ["attributes|a|-|-|-|-|x|null"]);
}

#[test]
fn qy6a_attribute_filter() {
    let o = qy6a_out(
        r#"const a = $('a'); watch(a, { attributeFilter: ['x', 'y'], attributeOldValue: true });
         step(() => { a.setAttribute('z', '1'); a.setAttribute('x', '1'); a.setAttribute('w', '1'); a.setAttribute('y', '1'); });"#,
    );
    assert_eq!(o, ["attributes|a|-|-|-|-|x|null ; attributes|a|-|-|-|-|y|null"]);
}

#[test]
fn qy6a_character_data_records() {
    let o = qy6a_out(
        r#"const t = $('s').firstChild; watch(t, { characterData: true, characterDataOldValue: true });
         step(() => { t.data = 'one'; },
              () => { t.nodeValue = 'two'; },
              () => { t.textContent = 'three'; },
              () => t.appendData('+'),
              () => t.insertData(0, '>'),
              () => t.deleteData(0, 1),
              () => t.replaceData(0, 2, 'ZZ'));"#,
    );
    assert_eq!(
        o,
        [
            "characterData|#text|-|-|-|-|null|text",
            "characterData|#text|-|-|-|-|null|one",
            "characterData|#text|-|-|-|-|null|two",
            "characterData|#text|-|-|-|-|null|three",
            "characterData|#text|-|-|-|-|null|three+",
            "characterData|#text|-|-|-|-|null|>three+",
            "characterData|#text|-|-|-|-|null|three+",
        ]
    );
}

#[test]
fn qy6a_character_data_methods_edit_the_text() {
    let log = qy6a_run(
        r#"const t = document.createTextNode('hello');
         t.appendData(' world'); t.insertData(0, '>'); t.deleteData(1, 2); t.replaceData(0, 1, '<<');
         console.log(t.data, t.substringData(2, 3));
         try { t.insertData(99, 'x'); } catch (e) { console.log(e.name); }"#,
    );
    assert_eq!(log, ["log:<<llo world llo", "log:IndexSizeError"], "{log:?}");
}

#[test]
fn qy6a_document_write_records_insertions() {
    let out = qa13_run(
        "<html><body><p id=a>x<script id=sc>\
         var mo = new MutationObserver(function (l) { l.forEach(function (r) { console.log(r.type, r.target.id, r.addedNodes[0].id, r.addedNodes.length, r.previousSibling.id, r.nextSibling && r.nextSibling.id); }); });\
         mo.observe(document.getElementById('a'), { childList: true });\
         document.write('<b id=w>hi</b><u id=v></u>');</script><i id=z></i></p></body></html>",
    );
    assert_eq!(out, ["log:childList a w 2 sc z"], "{out:?}");
}

#[test]
fn qy6a_subtree_observation() {
    let o = qy6a_out(
        r#"const h = $('h'); const mo = watch(h, { subtree: true, childList: true, attributes: true });
         step(() => $('a').appendChild(mk('p', 'p1')),
              () => $('p1').setAttribute('q', '1'),
              () => { $('s').appendChild(mk('z', 'z1')); });"#,
    );
    assert_eq!(o, ["childList|a|p1|-|-|-|null|null", "attributes|p1|-|-|-|-|q|null"]);
}

#[test]
fn qy6a_without_subtree_only_direct_children() {
    let o = qy6a_out(
        r#"const h = $('h'); watch(h, { childList: true, attributes: true });
         step(() => $('a').appendChild(mk('p', 'p1')), () => $('a').setAttribute('q', '1'), () => h.setAttribute('q', '1'));"#,
    );
    assert_eq!(o, ["attributes|h|-|-|-|-|q|null"]);
}

#[test]
fn qy6a_transient_observers_for_removed_subtree() {
    let o = qy6a_out(
        r#"const h = $('h'), a = $('a'); watch(h, { subtree: true, attributes: true });
         const kid = mk('p', 'p1'); a.appendChild(kid);
         step(() => { h.removeChild(a); a.setAttribute('x', '1'); kid.setAttribute('y', '1'); },
              () => { a.setAttribute('after', '1'); });"#,
    );
    // The removal inside the delivery turn: both writes still reach the observer
    // (transient registrations); once delivered they are dropped.
    assert_eq!(o, ["attributes|a|-|-|-|-|x|null ; attributes|p1|-|-|-|-|y|null"]);
}

#[test]
fn qy6a_batching_microtask_order_and_this() {
    let log = qy6a_run(
        r#"const h = $('h');
         const mo = new MutationObserver(function (recs, obs) { console.log('cb', recs.length, this === mo, obs === mo); });
         mo.observe(h, { childList: true, attributes: true });
         queueMicrotask(() => console.log('micro before'));
         h.appendChild(mk('p')); h.setAttribute('x', '1'); h.appendChild(mk('p'));
         console.log('sync done');
         Promise.resolve().then(() => console.log('micro after'));"#,
    );
    assert_eq!(log, ["log:sync done", "log:micro before", "log:cb 3 true true", "log:micro after"], "{log:?}");
}

#[test]
fn qy6a_observers_notified_in_creation_order() {
    let log = qy6a_run(
        r#"const h = $('h');
         const m1 = new MutationObserver(() => console.log('first')), m2 = new MutationObserver(() => console.log('second'));
         m2.observe(h, { childList: true }); m1.observe(h, { childList: true });
         h.appendChild(mk('p'));"#,
    );
    assert_eq!(log, ["log:first", "log:second"], "{log:?}");
}

#[test]
fn qy6a_records_made_in_a_callback_deliver_in_the_next_batch() {
    let log = qy6a_run(
        r#"const h = $('h'); let k = 0;
         const mo = new MutationObserver(recs => { console.log('cb', recs.length); if (k++ === 0) h.appendChild(mk('q')); });
         mo.observe(h, { childList: true });
         h.appendChild(mk('p'));"#,
    );
    assert_eq!(log, ["log:cb 1", "log:cb 1"], "{log:?}");
}

#[test]
fn qy6a_disconnect_and_take_records() {
    let log = qy6a_run(
        r#"const h = $('h');
         const mo = new MutationObserver(recs => console.log('delivered', recs.length));
         mo.observe(h, { childList: true });
         h.appendChild(mk('p'));
         const t = mo.takeRecords();
         console.log('taken', t.length, t[0].type, mo.takeRecords().length);
         h.appendChild(mk('p'));
         mo.disconnect();
         h.appendChild(mk('p'));
         const m2 = new MutationObserver(recs => console.log('m2', recs.length));
         m2.observe(h, { childList: true });
         h.appendChild(mk('p'));
         m2.disconnect();
         Promise.resolve().then(() => console.log('end'));"#,
    );
    assert_eq!(log, ["log:taken 1 childList 0", "log:end"], "{log:?}");
}

#[test]
fn qy6a_reobserve_replaces_options_and_other_targets_keep_theirs() {
    let o = qy6a_out(
        r#"const h = $('h'), a = $('a');
         const mo = new MutationObserver(l => out.push(l.map(d).join(' ; ')));
         mo.observe(h, { childList: true }); mo.observe(a, { attributes: true });
         mo.observe(h, { attributes: true });
         step(() => { h.appendChild(mk('p')); h.setAttribute('x', '1'); a.setAttribute('y', '1'); });"#,
    );
    assert_eq!(o, ["attributes|h|-|-|-|-|x|null ; attributes|a|-|-|-|-|y|null"]);
}

#[test]
fn qy6a_observe_validates_options() {
    let log = qy6a_run(
        r#"const mo = new MutationObserver(() => {}), h = $('h');
         const t = o => { try { mo.observe(h, o); console.log('ok'); } catch (e) { console.log(e.name); } };
         t({}); t({ subtree: true }); t({ attributes: false });
         t({ attributeOldValue: true }); t({ attributeFilter: ['a'] }); t({ characterDataOldValue: true });
         t({ attributes: false, attributeOldValue: true }); t({ childList: true, attributeFilter: ['a'], attributes: false });
         t({ characterData: false, characterDataOldValue: true, childList: true });
         try { mo.observe(); } catch (e) { console.log(e.name); }
         try { mo.observe({}, { childList: true }); } catch (e) { console.log(e.name); }
         try { new MutationObserver(); } catch (e) { console.log(e.name); }
         try { new MutationObserver(1); } catch (e) { console.log(e.name); }
         try { MutationObserver(() => {}); } catch (e) { console.log(e.name); }"#,
    );
    assert_eq!(
        log,
        [
            "log:TypeError", "log:TypeError", "log:TypeError", "log:ok", "log:ok", "log:ok", "log:TypeError", "log:TypeError",
            "log:TypeError", "log:TypeError", "log:TypeError", "log:TypeError", "log:TypeError", "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6a_globals_and_record_shape() {
    let log = qy6a_run(
        r#"console.log(window.MutationObserver === MutationObserver, WebKitMutationObserver === MutationObserver, typeof MutationRecord);
         console.log(Object.prototype.toString.call(new MutationObserver(() => {})), String(MutationRecord.prototype[Symbol.toStringTag]));
         const mo = new MutationObserver(recs => {
           const r = recs[0];
           console.log(r instanceof MutationRecord, Object.prototype.toString.call(r), r.type, r.attributeNamespace, Array.isArray(r.addedNodes), r.removedNodes.length, r.target === $('a'));
         });
         mo.observe($('a'), { attributes: true }); $('a').setAttribute('k', 'v');"#,
    );
    assert_eq!(
        log,
        [
            "log:true true function",
            "log:[object MutationObserver] MutationRecord",
            "log:true [object MutationRecord] attributes null true 0 true",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6a_callback_error_does_not_stop_other_observers() {
    let log = qy6a_run(
        r#"const h = $('h');
         const m1 = new MutationObserver(() => { throw new Error('boom'); }), m2 = new MutationObserver(() => console.log('second ran'));
         m1.observe(h, { childList: true }); m2.observe(h, { childList: true });
         h.appendChild(mk('p'));"#,
    );
    assert!(log.iter().any(|l| l == "log:second ran"), "{log:?}");
    assert!(log.iter().any(|l| l.starts_with("error:") && l.contains("MutationObserver callback") && l.contains("boom")), "{log:?}");
}

#[test]
fn qy6a_custom_elements_and_observers_share_the_hooks() {
    let log = qy6a_run(
        r#"class XQ extends HTMLElement { connectedCallback() { console.log('conn'); } }
         customElements.define('x-q', XQ);
         const mo = new MutationObserver(l => console.log('records', l.length));
         mo.observe($('h'), { childList: true });
         $('h').appendChild(mk('x-q'));"#,
    );
    assert_eq!(log, ["log:conn", "log:records 1"], "{log:?}");
}

// ---- QY6g: IntersectionObserver / ResizeObserver / PerformanceObserver ----

/// Like `run_with_timers` but on a parsed document, so observers get targets.
fn qy6g_run(code: &str) -> Vec<String> {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let html = "<html><body><div id=a></div><img id=i width=40 height=30>\
                <div id=hid style='display:none'><p id=inner></p></div><div id=b></div></body></html>";
    let engine = crate::js::start_scripts_clocked(
        &parse_html(html),
        &[src(&format!("const $ = id => document.getElementById(id);\n{code}"))],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    for _ in 0..1000 {
        let Some(due) = crate::event_loop::headless_next_due(&engine) else { break };
        clock.set(clock.get().max(due));
        crate::event_loop::headless_turn(&engine, clock.get());
    }
    let v = out.borrow().clone();
    v
}

#[test]
fn qy6g_io_initial_entry_delivered_once_visible() {
    let log = qy6g_run(
        r#"const io = new IntersectionObserver(function (es, o) {
             console.log('cb', es.length, this === io, o === io);
             es.forEach(e => console.log(e.target.id, e.isIntersecting, e.intersectionRatio, e instanceof IntersectionObserverEntry,
               e.boundingClientRect instanceof DOMRectReadOnly, e.rootBounds.width, e.rootBounds.height, typeof e.time));
           });
           io.observe($('a')); io.observe($('a')); io.observe($('i'));
           console.log('sync');"#,
    );
    assert_eq!(
        log,
        ["log:sync", "log:cb 2 true true", "log:a true 1 true true 1024 768 number", "log:i true 1 true true 1024 768 number"],
        "{log:?}"
    );
}

#[test]
fn qy6g_io_hidden_and_detached_report_not_intersecting() {
    let log = qy6g_run(
        r#"const io = new IntersectionObserver(es => es.forEach(e => console.log(e.target.id || 'det', e.isIntersecting, e.intersectionRatio)));
           io.observe($('inner')); io.observe($('hid')); io.observe(document.createElement('div')); io.observe($('b'));"#,
    );
    assert_eq!(log, ["log:inner false 0", "log:hid false 0", "log:det false 0", "log:b true 1"], "{log:?}");
}

#[test]
fn qy6g_io_unobserve_disconnect_take_records() {
    let log = qy6g_run(
        r#"const io = new IntersectionObserver(es => console.log('A', es.map(e => e.target.id).join()));
           io.observe($('a')); io.observe($('b')); io.unobserve($('a'));
           const i2 = new IntersectionObserver(es => console.log('B'));
           i2.observe($('a')); i2.disconnect();
           const i3 = new IntersectionObserver(es => console.log('C'));
           i3.observe($('a')); const r = i3.takeRecords(); console.log('took', r.length, i3.takeRecords().length);"#,
    );
    assert_eq!(log, ["log:took 1 0", "log:A b"], "{log:?}");
}

#[test]
fn qy6g_io_arguments_and_properties() {
    let log = qy6g_run(
        r#"const t = f => { try { f(); return 'ok'; } catch (e) { return e.name; } };
           console.log(t(() => new IntersectionObserver(() => {}, { threshold: 2 })),
                       t(() => new IntersectionObserver(() => {}, { threshold: [0, -1] })),
                       t(() => new IntersectionObserver(() => {}, { rootMargin: '5em' })),
                       t(() => new IntersectionObserver(() => {}, { rootMargin: '1px 2px 3px 4px 5px' })),
                       t(() => new IntersectionObserver()),
                       t(() => new IntersectionObserver(1)),
                       t(() => IntersectionObserver(() => {})),
                       t(() => new IntersectionObserver(() => {}).observe({})),
                       t(() => new IntersectionObserver(() => {}, { root: 5 })),
                       t(() => new IntersectionObserver(() => {}, { threshold: [0.5, 0.25] })));
           const o = new IntersectionObserver(() => {}, { root: $('a'), rootMargin: '10px 20%', threshold: [1, 0.5] });
           console.log(o.root === $('a'), o.rootMargin, o.thresholds.join(), new IntersectionObserver(() => {}).thresholds.join(), new IntersectionObserver(() => {}).rootMargin,
                       new IntersectionObserver(() => {}, { threshold: 0.3 }).thresholds[0]);
           const r = new DOMRect(1, 2, 3, 4); console.log(r.right, r.bottom, JSON.stringify(DOMRectReadOnly.fromRect({ x: 5 })));"#,
    );
    assert_eq!(
        log,
        [
            "log:RangeError RangeError SyntaxError SyntaxError TypeError TypeError TypeError TypeError TypeError ok",
            "log:true 10px 20% 10px 20% 0.5,1 0 0px 0px 0px 0px 0.3",
            "log:4 6 {\"x\":5,\"y\":0,\"width\":0,\"height\":0,\"top\":0,\"right\":5,\"bottom\":0,\"left\":5}",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6g_io_one_batch_per_observer_in_creation_order() {
    let log = qy6g_run(
        r#"const x = new IntersectionObserver(es => console.log('x', es.length));
           const y = new IntersectionObserver(es => console.log('y', es.length));
           y.observe($('a')); x.observe($('a')); x.observe($('b')); y.observe($('b')); y.observe($('i'));"#,
    );
    assert_eq!(log, ["log:x 2", "log:y 3"], "{log:?}");
}

#[test]
fn qy6g_performance_observer_is_inert() {
    let log = qy6g_run(
        r#"let n = 0;
           const po = new PerformanceObserver(() => n++);
           po.observe({ entryTypes: ['longtask', 'resource'] }); po.observe({ type: 'paint', buffered: true });
           const t = f => { try { f(); return 'ok'; } catch (e) { return e.name; } };
           console.log(t(() => po.observe({})), t(() => po.observe({ entryTypes: [], type: 'x' })), t(() => new PerformanceObserver()),
                       PerformanceObserver.supportedEntryTypes.length, po.takeRecords().length, typeof po.disconnect());
           setTimeout(() => console.log('n', n), 5);"#,
    );
    assert_eq!(log, ["log:TypeError TypeError TypeError 0 0 undefined", "log:n 0"], "{log:?}");
}

#[test]
fn qy6g_zero_cost_before_use() {
    // No timer exists until the first observe().
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = crate::js::start_scripts_clocked(
        &parse_html("<div id=a></div>"),
        &[src("new IntersectionObserver(() => {}); new ResizeObserver(() => {}); new PerformanceObserver(() => {}).observe({ entryTypes: ['mark'] });")],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(Rc::new(std::cell::Cell::new(0.0))),
    )
    .unwrap();
    assert!(engine.next_due_ms().is_none(), "{:?}", out.borrow());
    assert!(out.borrow().is_empty(), "{:?}", out.borrow());
}

// ---- QY57: IntersectionObserver with real geometry ----

/// Like `qy6g_run` with a caller-supplied body (viewport 1024x768).
fn qy57_run(body: &str, code: &str) -> Vec<String> {
    crate::js_geom::TEST_FIXED_COST_MS.with(|c| c.set(Some(1.0))); // QFLK2
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let html = format!("<html><body style='margin:0'>{body}</body></html>");
    let engine = crate::js::start_scripts_clocked(
        &parse_html(&html),
        &[src(&format!("const $ = id => document.getElementById(id);\n{code}"))],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    for _ in 0..1000 {
        let Some(due) = crate::event_loop::headless_next_due(&engine) else { break };
        clock.set(clock.get().max(due));
        crate::event_loop::headless_turn(&engine, clock.get());
    }
    out.borrow().clone()
}

#[test]
fn qy57_css_sized_target_without_attributes_has_real_rects() {
    let log = qy57_run(
        "<div style='height:40px'></div><div id=t style='width:200px;height:100px'></div>",
        r#"const io = new IntersectionObserver(es => es.forEach(e => {
             const b = e.boundingClientRect, i = e.intersectionRect;
             console.log(e.isIntersecting, e.intersectionRatio, b.x, b.y, b.width, b.height, i.y, i.width, i.height, e.rootBounds.width, e.rootBounds.height);
           }));
           io.observe($('t'));"#,
    );
    assert_eq!(log, ["log:true 1 0 40 200 100 40 200 100 1024 768"], "{log:?}");
}

#[test]
fn qy57_offscreen_becomes_visible_after_document_scroll() {
    let log = qy57_run(
        "<div style='height:2000px'></div><div id=t style='height:100px'></div><div style='height:2000px'></div>",
        r#"const io = new IntersectionObserver(es => es.forEach(e =>
             console.log(e.isIntersecting, e.intersectionRatio, Math.round(e.boundingClientRect.top), e.intersectionRect.height)));
           io.observe($('t'));
           setTimeout(() => window.scrollTo(0, 1900), 50);
           setTimeout(() => window.scrollTo(0, 1950), 100);
           setTimeout(() => window.scrollTo(0, 0), 150);"#,
    );
    // initial: below the fold; 1900: fully visible; 1950: no crossing (still ratio 1, so no entry); 0: gone.
    assert_eq!(log, ["log:false 0 2000 0", "log:true 1 100 100", "log:false 0 2000 0"], "{log:?}");
}

/// QY57r: like `qy57_run` but with the whole document given.
fn qy57r_run(html: &str, code: &str) -> Vec<String> {
    crate::js_geom::TEST_FIXED_COST_MS.with(|c| c.set(Some(1.0)));
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html(html),
        &[src(&format!("const $ = id => document.getElementById(id);\n{code}"))],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    for _ in 0..1000 {
        let Some(due) = crate::event_loop::headless_next_due(&engine) else { break };
        clock.set(clock.get().max(due));
        crate::event_loop::headless_turn(&engine, clock.get());
    }
    out.borrow().clone()
}

#[test]
fn qy57r_body_overflow_propagates_to_viewport_and_does_not_clip() {
    let code = "new IntersectionObserver(es => es.forEach(e => console.log(e.isIntersecting, e.intersectionRatio, e.intersectionRect.height))).observe($('t'));";
    // html visible: body's overflow:scroll goes to the viewport; a zero-height body must not clip.
    let log = qy57r_run(
        "<html><body style='margin:0;overflow:scroll;height:0'><div id=t style='height:100px'></div></body></html>",
        code,
    );
    assert_eq!(log, ["log:true 1 100"], "{log:?}");
    // html's own overflow is the viewport's too.
    let log = qy57r_run(
        "<html style='overflow:hidden'><body style='margin:0'><div id=t style='height:100px'></div></body></html>",
        code,
    );
    assert_eq!(log, ["log:true 1 100"], "{log:?}");
    // Not propagated (html overflow:hidden): body is a real clip with its small box.
    let log = qy57r_run(
        "<html style='overflow:hidden'><body style='margin:0;overflow:hidden;height:20px'><div id=t style='height:100px'></div></body></html>",
        code,
    );
    assert_eq!(log, ["log:true 0.2 20"], "{log:?}");
}

#[test]
fn qy57_scrolling_container_and_explicit_root() {
    let body = "<div id=box style='height:200px;overflow:auto'><div style='height:600px'></div><div id=t style='height:50px'></div><div style='height:900px'></div></div>";
    // Implicit root: the overflow ancestor clips the target while it is scrolled out.
    let log = qy57_run(
        body,
        r#"const io = new IntersectionObserver(es => es.forEach(e => console.log('imp', e.isIntersecting, e.intersectionRatio)));
           io.observe($('t'));
           setTimeout(() => { $('box').scrollTop = 500; }, 50);"#,
    );
    assert_eq!(log, ["log:imp false 0", "log:imp true 1"], "{log:?}");
    // Explicit root: the root's own box is the bounds (200px high).
    let log = qy57_run(
        body,
        r#"const io = new IntersectionObserver(es => es.forEach(e => console.log('root', e.isIntersecting, e.rootBounds.height)), { root: $('box') });
           io.observe($('t'));
           setTimeout(() => { $('box').scrollTop = 500; }, 50);"#,
    );
    assert_eq!(log, ["log:root false 200", "log:root true 200"], "{log:?}");
}

#[test]
fn qy57_root_margin_extends_the_root() {
    let body = "<div style='height:800px'></div><div id=t style='height:100px'></div>";
    let log = qy57_run(
        body,
        r#"new IntersectionObserver(es => es.forEach(e => console.log('none', e.isIntersecting))).observe($('t'));
           new IntersectionObserver(es => es.forEach(e => console.log('px', e.isIntersecting, e.rootBounds.top, e.rootBounds.bottom)), { rootMargin: '50px' }).observe($('t'));
           new IntersectionObserver(es => es.forEach(e => console.log('pct', e.isIntersecting, e.rootBounds.bottom)), { rootMargin: '10%' }).observe($('t'));"#,
    );
    assert_eq!(log, ["log:none false", "log:px true -50 818", "log:pct true 844.8"], "{log:?}");
}

#[test]
fn qy57_thresholds_fire_at_crossings() {
    let log = qy57_run(
        "<div style='height:2000px'></div><div id=t style='height:100px'></div><div style='height:2000px'></div>",
        r#"const io = new IntersectionObserver(es => es.forEach(e => console.log(e.isIntersecting, e.intersectionRatio.toFixed(2))), { threshold: [0, 0.5, 1] });
           io.observe($('t'));
           const at = (ms, y) => setTimeout(() => window.scrollTo(0, y), ms);
           at(50, 1250);   // top at 750: 18px visible (0.18): crosses 0
           at(100, 1280);  // top at 720: 48px (0.48): same band, no entry
           at(150, 1330);  // top at 670: 98px (0.98): crosses 0.5
           at(200, 1400);  // fully visible: crosses 1
           at(250, 0);     // gone"#,
    );
    assert_eq!(log, ["log:false 0.00", "log:true 0.18", "log:true 0.98", "log:true 1.00", "log:false 0.00"], "{log:?}");
}

#[test]
fn qy57_unobserve_and_disconnect_stop_delivery() {
    let log = qy57_run(
        "<div style='height:2000px'></div><div id=t style='height:100px'></div><div id=u style='height:100px'></div><div style='height:2000px'></div>",
        r#"const a = new IntersectionObserver(es => es.forEach(e => console.log('a', e.target.id, e.isIntersecting)));
           const b = new IntersectionObserver(es => es.forEach(e => console.log('b', e.target.id, e.isIntersecting)));
           a.observe($('t')); a.observe($('u')); b.observe($('t'));
           setTimeout(() => { a.unobserve($('t')); b.disconnect(); window.scrollTo(0, 1900); }, 50);"#,
    );
    // Both get the initial (false) entries; after unobserve/disconnect only `u` reports.
    assert_eq!(log, ["log:a t false", "log:a u false", "log:b t false", "log:a u true"], "{log:?}");
}

#[test]
fn qy57_take_records_and_dom_writes_recheck() {
    let log = qy57_run(
        "<div id=t style='height:100px'></div>",
        r#"const io = new IntersectionObserver(es => es.forEach(e => console.log('cb', e.isIntersecting, e.boundingClientRect.height)));
           io.observe($('t'));
           const r = io.takeRecords();
           console.log('took', r.length, r[0].isIntersecting, io.takeRecords().length);
           setTimeout(() => { $('t').style.marginTop = '900px'; }, 50);
           setTimeout(() => { $('t').style.marginTop = '0'; }, 400);"#,
    );
    // A layout change moves the target out of and back into view.
    assert_eq!(log, ["log:took 1 true 0", "log:cb false 100", "log:cb true 100"], "{log:?}");
}

// ---- QY53e1: deferred, rate-limited IO rechecks after DOM-only changes ----

/// `qy57_run` that also returns how many forced layouts ran.
fn qy53e1_run(body: &str, code: &str) -> (Vec<String>, usize) {
    // QFLK2: the recheck deadline is 4x the measured (CPU) layout cost; pin it so load cannot move delivery times.
    crate::js_geom::TEST_FIXED_COST_MS.with(|c| c.set(Some(1.0)));
    crate::js_geom::GEOM_LAYOUTS.with(|c| c.set(0));
    let log = qy57_run(body, code);
    (log, crate::js_geom::GEOM_LAYOUTS.with(|c| c.get()))
}

#[test]
fn qy53e1_observe_delivers_initial_entry_with_one_layout() {
    let (log, layouts) = qy53e1_run(
        "<div id=t style='height:100px'></div>",
        "new IntersectionObserver(es => es.forEach(e => console.log(e.isIntersecting))).observe($('t'));",
    );
    assert_eq!(log, ["log:true"], "{log:?}");
    assert_eq!(layouts, 1);
}

#[test]
fn qy53e1_writes_that_move_nothing_are_rate_limited() {
    let (log, layouts) = qy53e1_run(
        "<div id=t style='height:100px'></div><div id=o></div>",
        r#"new IntersectionObserver(es => es.forEach(e => console.log(e.isIntersecting))).observe($('t'));
           for (let i = 1; i <= 10; i++) setTimeout(() => $('o').setAttribute('data-x', String(i)), i * 30);"#,
    );
    assert_eq!(log, ["log:true"], "{log:?}");
    // Undeferred this is 1 + 10. The deadline is >= 100 ms and not moved by later writes: 300 ms of writes.
    assert!((1..=4).contains(&layouts), "layouts {layouts}");
}

#[test]
fn qy53e1_moved_target_is_delivered_by_the_deadline() {
    let (log, layouts) = qy53e1_run(
        "<div id=t style='height:100px'></div>",
        r#"new IntersectionObserver(es => es.forEach(e => console.log(Math.round(performance.now()), e.isIntersecting))).observe($('t'));
           setTimeout(() => { $('t').style.marginTop = '900px'; }, 50);"#,
    );
    assert_eq!(log.len(), 2, "{log:?}");
    assert_eq!(log[0], "log:17 true"); // QEL3: the first frame boundary
    let (t, v) = log[1].strip_prefix("log:").unwrap().split_once(' ').unwrap();
    assert_eq!(v, "false");
    let t: i64 = t.parse().unwrap();
    assert!((150..=2100).contains(&t), "delivered at {t} ms: after the write (50) + the 100 ms floor, not immediately");
    assert_eq!(layouts, 2);
}

#[test]
fn qy53e1_script_read_after_a_write_feeds_io_without_a_second_layout() {
    let (log, layouts) = qy53e1_run(
        "<div id=t style='height:100px'></div>",
        r#"new IntersectionObserver(es => es.forEach(e => console.log(Math.round(performance.now()), e.isIntersecting))).observe($('t'));
           setTimeout(() => { $('t').style.marginTop = '900px'; $('t').offsetWidth; }, 50);"#,
    );
    // The read laid the page out; IO re-evaluates at the next rendering update (QEL3: frame boundaries 17 / 67 ms), no deadline wait.
    assert_eq!(log, ["log:17 true", "log:67 false"], "{log:?}");
    assert_eq!(layouts, 2);
}

#[test]
fn qy53e1_scroll_keeps_the_immediate_path() {
    let (log, _) = qy53e1_run(
        "<div style='height:2000px'></div><div id=t style='height:100px'></div><div style='height:2000px'></div>",
        r#"new IntersectionObserver(es => es.forEach(e => console.log(Math.round(performance.now()), e.isIntersecting))).observe($('t'));
           setTimeout(() => window.scrollTo(0, 1900), 50);"#,
    );
    assert_eq!(log, ["log:17 false", "log:67 true"], "{log:?}");
}

#[test]
fn qy57_zero_area_target_touching_the_root_intersects() {
    let log = qy57_run(
        "<div style='height:768px'></div><div id=t style='height:0'></div><div style='height:100px'></div>",
        r#"new IntersectionObserver(es => es.forEach(e => console.log(e.isIntersecting, e.intersectionRatio))).observe($('t'));"#,
    );
    assert_eq!(log, ["log:true 1"], "{log:?}");
}

// ---- QY6f: template.content, importNode.

#[test]
fn qy6f_parsed_template_content_innerhtml_and_outerhtml() {
    let log = run_in_html(
        "<html><body><template id=t><p class=x>a</p><b>b</b></template><div id=d></div></body></html>",
        r#"const t = document.getElementById('t');
         console.log(t.childNodes.length, t.content.childNodes.length, t.content === t.content, t.content.nodeType);
         console.log(t.innerHTML);
         console.log(t.outerHTML);
         console.log(t.content.querySelector('p.x').textContent, t.content.querySelectorAll('*').length);
         console.log(document.querySelector('p.x'), t.content.ownerDocument === document, t.content.ownerDocument.nodeType);
         t.innerHTML = '<i>z</i>';
         console.log(t.content.childNodes.length, t.content.firstChild.localName, t.innerHTML);
         console.log(t instanceof HTMLTemplateElement);"#,
    );
    assert_eq!(
        log,
        [
            "log:0 2 true 11",
            "log:<p class=\"x\">a</p><b>b</b>",
            "log:<template id=\"t\"><p class=\"x\">a</p><b>b</b></template>",
            "log:a 2",
            "log:null false 9",
            "log:1 i <i>z</i>",
            "log:true",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6f_created_template_clone_import_and_append_fragment() {
    let log = run_in_html(
        "<html><body><div id=d></div></body></html>",
        r#"const t = document.createElement('template');
         const p = document.createElement('p'); p.textContent = 'hi';
         t.content.appendChild(p);
         console.log(t.content.childNodes.length, t.childNodes.length, t.innerHTML, p.ownerDocument === t.content.ownerDocument);
         const c = t.cloneNode(true);
         console.log(c.content !== t.content, c.content.childNodes.length, c.content.firstChild !== p, c.innerHTML);
         const shallow = t.cloneNode(false);
         console.log(shallow.content.childNodes.length);
         const fc = t.content.cloneNode(true);
         console.log(fc.childNodes.length, fc.firstChild.ownerDocument === t.content.ownerDocument);
         const imp = document.importNode(t.content, true);
         console.log(imp.childNodes.length, imp.firstChild.ownerDocument === document, t.content.childNodes.length);
         const d = document.getElementById('d');
         d.appendChild(imp);
         console.log(d.innerHTML, imp.childNodes.length);
         document.body.appendChild(t.content);
         console.log(t.content.childNodes.length, document.body.querySelectorAll('p').length, p.ownerDocument === document);"#,
    );
    assert_eq!(
        log,
        [
            "log:1 0 <p>hi</p> true",
            "log:true 1 true <p>hi</p>",
            "log:0",
            "log:1 true",
            "log:1 true 1",
            "log:<p>hi</p> 0",
            "log:0 2 true",
        ],
        "{log:?}"
    );
}

#[test]
fn qy6f_content_is_inert_no_custom_element_upgrade() {
    let log = run_in_html(
        "<html><body><template id=t><x-q></x-q></template></body></html>",
        r#"let n = 0;
         class XQ extends HTMLElement { constructor() { super(); n++; } }
         customElements.define('x-q', XQ);
         const t = document.getElementById('t');
         const e = t.content.firstChild;
         t.innerHTML = '<x-q></x-q>';
         const cl = t.content.cloneNode(true);
         console.log(n, e instanceof XQ, t.content.firstChild instanceof XQ, cl.firstChild instanceof XQ);
         const im = document.importNode(t.content, true);
         console.log(n, im.firstChild instanceof XQ);
         document.body.appendChild(t.content);
         console.log(n, document.body.lastChild instanceof XQ);"#,
    );
    assert_eq!(log, ["log:0 false false false", "log:1 true", "log:2 true"], "{log:?}");
}

#[test]
fn qy6f_template_is_not_rendered() {
    let doc = parse_html("<html><body><p>shown</p><template><p>TPLTEXT</p></template><template id=u></template></body></html>");
    run_scripts_in(&doc, &[src("document.getElementById('u').appendChild(document.createTextNode('OWNKID'));")], Rc::new(|_, _| {}));
    let tree = build_layout_tree(&doc, 1920.0);
    let mut texts = Vec::new();
    q19_texts(&tree, &mut texts);
    assert!(texts.iter().any(|(t, _)| t.contains("shown")), "{texts:?}");
    assert!(!texts.iter().any(|(t, _)| t.contains("TPLTEXT") || t.contains("OWNKID")), "{texts:?}");
}

// ---- QSD0: fragment wrappers must not assign query methods per instance.

#[test]
fn qsd0_readonly_fragment_query_methods_do_not_break_wrappers() {
    let log = run_in_html(
        "<html><body><div id=a><p class=c>x</p></div></body></html>",
        r#"'use strict';
         Object.defineProperty(DocumentFragment.prototype, 'querySelectorAll',
           { value: function () { return ['page']; } });
         Object.freeze(DocumentFragment.prototype);
         const f = document.createDocumentFragment();
         const p = document.createElement('p');
         f.appendChild(p);
         console.log(f.querySelectorAll('p')[0], f.querySelector('p') === p, f.getElementsByTagName('p').length);
         console.log(document.querySelectorAll('p.c').length, document.getElementById('a').querySelector('.c').tagName);"#,
    );
    assert_eq!(log, ["log:page true 1", "log:1 P"], "{log:?}");
}

// ---- QSD1: document.domain, Document as ParentNode, getElementsByName.

#[test]
fn qsd1_document_domain_and_parent_node() {
    set_page_url(Url::parse("https://gist.github.com/a/b").ok().as_ref());
    let log = run_in_html(
        "<html><body><input name=n><p name=n></p><i name=m></i></body></html>",
        r#"const r = [];
         r.push(document.domain);
         document.domain = 'gist.github.com'; r.push(document.domain);
         document.domain = 'github.com'; r.push(document.domain);
         for (const bad of ['com', 'example.org', 'ub.com']) {
           try { document.domain = bad; r.push('no throw'); } catch (e) { r.push(e.name + ':' + (e instanceof DOMException)); }
         }
         r.push(document.firstElementChild.tagName, document.lastElementChild.tagName, document.childElementCount, document.children.length, document.children[0] === document.documentElement);
         const g = document.getElementsByName('n');
         r.push(g.length, g[0].tagName, document.getElementsByName('zz').length);
         console.log(r.join('|'));"#,
    );
    set_page_url(None);
    assert_eq!(
        log,
        ["log:gist.github.com|gist.github.com|github.com|SecurityError:true|SecurityError:true|SecurityError:true|HTML|HTML|1|1|true|2|INPUT|0"],
        "{log:?}"
    );
}

// ---- QSD1b: crypto.getRandomValues / randomUUID / subtle placeholder / isSecureContext.

#[test]
fn qsd1b_crypto_random_values_uuid_and_subtle_placeholder() {
    set_page_url(Url::parse("https://github.com/").ok().as_ref());
    let log = run_in_html(
        "<html><body></body></html>",
        r#"const r = [];
         r.push(window.crypto === globalThis.crypto, crypto instanceof Crypto, crypto.subtle instanceof SubtleCrypto, isSecureContext);
         const a = new Uint8Array(64); const b = crypto.getRandomValues(a);
         r.push(a === b, a.some(function (x) { return x !== 0; }));
         const u = new Uint32Array(4); crypto.getRandomValues(u); r.push(u.some(function (x) { return x !== 0; }));
         const id1 = crypto.randomUUID(), id2 = crypto.randomUUID();
         r.push(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id1), id1 !== id2);
         try { crypto.getRandomValues(new Float32Array(2)); r.push('no'); } catch (e) { r.push(e.name + ':' + (e instanceof DOMException)); }
         try { crypto.getRandomValues(new Uint8Array(65537)); r.push('no'); } catch (e) { r.push(e.name); }
         try { crypto.getRandomValues(new Uint8Array(65536)); r.push('ok'); } catch (e) { r.push(e.name); }
         try { crypto.getRandomValues([1, 2]); r.push('no'); } catch (e) { r.push(e.name); }
         console.log(r.join('|'));
         crypto.subtle.digest('SHA-256', new Uint8Array(1)).then(function () { console.log('resolved'); }, function (e) { console.log('rejected:' + e.name); });"#,
    );
    set_page_url(None);
    assert_eq!(log.len(), 3, "{log:?}");
    assert_eq!(
        log[0],
        "log:true|true|true|true|true|true|true|true|true|TypeMismatchError:true|QuotaExceededError|ok|TypeMismatchError"
    );
    assert!(log[1].starts_with("error:") && log[1].contains("not implemented: SubtleCrypto.digest"), "{log:?}");
    assert_eq!(log[2], "log:rejected:NotSupportedError");
}

#[test]
fn qsd1b_is_secure_context_follows_scheme_and_loopback() {
    let mut got = Vec::new();
    for u in ["https://example.org/", "http://example.org/", "http://localhost:8080/", "http://127.0.0.1/", "http://[::1]/"] {
        set_page_url(Url::parse(u).ok().as_ref());
        got.push(run_in_html("<html><body></body></html>", "console.log(String(isSecureContext))").remove(0));
    }
    set_page_url(None);
    assert_eq!(got, ["log:true", "log:false", "log:true", "log:true", "log:true"]);
}

// ---- QSD2: getRootNode, isConnected, compareDocumentPosition.

#[test]
fn qsd2_get_root_node_and_is_connected() {
    let log = run_in_html(
        "<html><body><div id=a><p id=b>x</p></div><template id=t><i id=ti></i></template></body></html>",
        r#"const r = [];
         const a = document.getElementById('a'), b = document.getElementById('b');
         r.push(b.getRootNode() === document, b.getRootNode({ composed: true }) === document, document.getRootNode() === document, b.isConnected, document.isConnected);
         const d = document.createElement('div'), s = document.createElement('span'); d.appendChild(s);
         r.push(s.getRootNode() === d, d.getRootNode() === d, s.isConnected, d.isConnected);
         const f = document.createDocumentFragment(), q = document.createElement('q'); f.appendChild(q);
         r.push(q.getRootNode() === f, f.getRootNode() === f, q.isConnected, f.isConnected);
         document.body.appendChild(d);
         r.push(s.getRootNode() === document, s.isConnected);
         const ti = document.getElementById('t').content.firstChild, tc = document.getElementById('t').content;
         r.push(ti.getRootNode() === tc, ti.isConnected, document.getElementById('ti'));
         const d2 = document.implementation.createHTMLDocument('t');
         r.push(d2.body.getRootNode() === d2, d2.body.isConnected, d2.createElement('u').isConnected);
         console.log(r.join('|'));"#,
    );
    assert_eq!(
        log,
        ["log:true|true|true|true|true|true|true|false|false|true|true|false|false|true|true|true|false||true|true|false"],
        "{log:?}"
    );
}

#[test]
fn qsd2_compare_document_position() {
    let log = run_in_html(
        "<html><body><div id=a><p id=b>x</p><p id=c>y</p></div><span id=e></span></body></html>",
        r#"const r = [], $ = function (i) { return document.getElementById(i); };
         const a = $('a'), b = $('b'), c = $('c'), e = $('e');
         r.push(a.compareDocumentPosition(a), a.compareDocumentPosition(b), b.compareDocumentPosition(a));
         r.push(b.compareDocumentPosition(c), c.compareDocumentPosition(b), b.compareDocumentPosition(e), e.compareDocumentPosition(b));
         r.push(document.compareDocumentPosition(b), b.compareDocumentPosition(document));
         const x = document.createElement('x'), y = document.createElement('y');
         const dx = x.compareDocumentPosition(y), dy = y.compareDocumentPosition(x);
         r.push((dx & 1) !== 0, (dx & 32) !== 0, dx === 37 || dx === 35, (dx & 6) !== (dy & 6));
         r.push(b.compareDocumentPosition(x) & 33, Node.DOCUMENT_POSITION_CONTAINED_BY, Node.prototype.DOCUMENT_POSITION_PRECEDING);
         try { a.compareDocumentPosition(null); r.push('no'); } catch (err) { r.push(err.name); }
         console.log(r.join('|'));"#,
    );
    assert_eq!(
        log,
        ["log:0|20|10|4|2|4|2|20|10|true|true|true|true|33|16|2|TypeError"],
        "{log:?}"
    );
}

const QSD4_HTML: &str = "<html><head><style>p{color:#0000ff}</style></head><body><p id=p>x</p></body></html>";

#[test]
fn qsd4_adopted_sheet_changes_used_colour_after_style() {
    let (c, l, _) = qsd4_color(QSD4_HTML, "var s=new CSSStyleSheet(); s.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets=[s];");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)), "{l:?}");
    let (c, _, a) = qsd4_color(QSD4_HTML, "");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)));
    assert_eq!(a, "", "zero cost when unused");
}

#[test]
fn qsd4_important_and_layer_interplay() {
    let html = "<html><head><style>p{color:#0000ff !important}</style></head><body><p>x</p></body></html>";
    let (c, ..) = qsd4_color(html, "var s=new CSSStyleSheet(); s.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets=[s];");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)));
    let (c, ..) = qsd4_color(QSD4_HTML, "var s=new CSSStyleSheet(); s.replaceSync('p{color:#ff0000 !important}'); document.adoptedStyleSheets=[s];");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)));
    let (c, ..) = qsd4_color(QSD4_HTML, "var s=new CSSStyleSheet(); s.replaceSync('@layer a { p{color:#ff0000} }'); document.adoptedStyleSheets=[s];");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)));
}

#[test]
fn qsd4_adoption_order_insert_delete_and_clear() {
    let (c, l, _) = qsd4_color(QSD4_HTML, "var a=new CSSStyleSheet(), b=new CSSStyleSheet(); a.replaceSync('p{color:#ff0000}'); b.replaceSync('p{color:#00ff00}'); document.adoptedStyleSheets=[a,b];");
    assert_eq!(c, Some(TextColor::rgb(0, 255, 0)), "later adopted sheet wins: {l:?}");
    let (c, l, _) = qsd4_color(QSD4_HTML, "var a=new CSSStyleSheet(); a.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets=[a]; a.insertRule('p{color:#00ff00}',1); console.log(a.cssRules.length);");
    assert_eq!(c, Some(TextColor::rgb(0, 255, 0)), "{l:?}");
    assert_eq!(l[0], "log:2");
    let (c, ..) = qsd4_color(QSD4_HTML, "var a=new CSSStyleSheet(); a.replaceSync('p{color:#ff0000} p{color:#00ff00}'); document.adoptedStyleSheets=[a]; a.deleteRule(1);");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)));
    let (c, _, a) = qsd4_color(QSD4_HTML, "var a=new CSSStyleSheet(); a.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets=[a]; document.adoptedStyleSheets=[];");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)));
    assert_eq!(a, "");
    let (c, ..) = qsd4_color(QSD4_HTML, "var a=new CSSStyleSheet(); a.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets.push(a); a.disabled=true;");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)), "disabled sheet does not apply");
}

#[test]
fn qsd4_api_surface() {
    let (_e, _d, log) = events_engine(
        QSD4_HTML,
        "var s=new CSSStyleSheet({media:'print',disabled:false});\
         s.replaceSync('/*c*/ @import url(x.css); a, b { color: red; margin: 0 !important } @media (min-width:1px){p{top:0}}');\
         var r=s.cssRules; console.log(r.length, r.item(0)===r[0], r[0].selectorText, r[0].type, r[0].style.color, r[0].style.getPropertyValue('margin'), r[0].style.getPropertyPriority('margin'));\
         console.log(r[0].cssText, '|', r[1].type);\
         console.log(s.media.mediaText, s.ownerNode, s.href, s.type, s.disabled, s.rules===s.cssRules);\
         try { document.adoptedStyleSheets = [{}]; } catch (e) { console.log('te', e instanceof TypeError); }\
         try { new CSSStyleSheet().insertRule('a{}', 5); } catch (e) { console.log(e.name); }\
         try { new CSSStyleSheet().insertRule('@import url(a.css)'); } catch (e) { console.log(e.name); }\
         try { new StyleSheet(); } catch (e) { console.log('ill', e instanceof TypeError); }\
         console.log(typeof CSSRule, typeof CSSRuleList, typeof CSSStyleRule, typeof MediaList, s instanceof StyleSheet, r[0] instanceof CSSStyleRule, r instanceof CSSRuleList);\
         var t=new CSSStyleSheet(); t.replace('p{color:red}').then(function(x){ console.log('rep', x===t, x.cssRules.length); });",
    );
    let l = log.borrow();
    assert!(l[0].contains("@import rules are not allowed"), "{l:?}");
    assert!(l[1].ends_with("2 true a, b 1 red 0 important"), "{l:?}");
    assert!(l[2].ends_with("a, b { color: red; margin: 0 !important; } | 4"), "{l:?}");
    assert!(l[3].ends_with("print null null text/css false true"), "{l:?}");
    assert!(l[4].ends_with("te true"), "{l:?}");
    assert!(l[5].ends_with("IndexSizeError"), "{l:?}");
    assert!(l[6].ends_with("SyntaxError"), "{l:?}");
    assert!(l[7].ends_with("ill true"), "{l:?}");
    assert!(l[8].ends_with("function function function function true true true"), "{l:?}");
    assert!(l[9].ends_with("rep true 1"), "{l:?}");
}

#[test]
fn qsd4_get_computed_style_sees_adopted_sheets() {
    let (_e, _d, log) = events_engine(
        QSD4_HTML,
        "var p=document.getElementById('p'); console.log(getComputedStyle(p).color);\
         var s=new CSSStyleSheet(); s.replaceSync('p{color:#ff0000}'); document.adoptedStyleSheets=[s];\
         console.log(getComputedStyle(p).color);\
         s.replaceSync('p{color:#00ff00}'); console.log(getComputedStyle(p).color);\
         document.adoptedStyleSheets=[]; console.log(getComputedStyle(p).color);",
    );
    let l = log.borrow();
    assert_eq!(
        *l,
        ["log:rgb(0, 0, 255)", "log:rgb(255, 0, 0)", "log:rgb(0, 255, 0)", "log:rgb(0, 0, 255)"],
        "{l:?}"
    );
}

/// Every command-line option a parser accepts must be documented on the
/// `:help` page, and `CLI_OPTIONS` must list every `"--option"` literal the
/// parsers (dom.rs `parse_args`, js.rs `--js-report`, wpt.rs `--wpt`, lib.rs
/// dispatch) mention, so a new option cannot slip past this test.
#[test]
fn cli_options_are_documented_in_help() {
    for opt in crate::dom::CLI_OPTIONS {
        assert!(HELP_PAGE_HTML.contains(opt), "`{opt}` is a command-line option but is not documented in src/help.html");
    }
    for (file, src) in [
        ("dom.rs", include_str!("../dom.rs")),
        ("js.rs", include_str!("../js.rs")),
        ("wpt.rs", include_str!("../wpt.rs")),
        ("wpt_th.rs", include_str!("../wpt_th.rs")),
        ("lib.rs", include_str!("../lib.rs")),
        ("perf.rs", include_str!("../perf.rs")),
    ] {
        let mut rest = src;
        while let Some(i) = rest.find("\"--") {
            rest = &rest[i + 1..];
            let lit: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
            let closed = rest[lit.len()..].starts_with('"');
            if closed && lit[2..].starts_with(|c: char| c.is_ascii_alphabetic()) {
                assert!(crate::dom::CLI_OPTIONS.contains(&lit.as_str()), "{file} accepts `{lit}`, which is missing from dom::CLI_OPTIONS");
            }
        }
    }
}

/// Lays out an already-parsed (and possibly script-mutated) document; the
/// returned height is the document height.
fn qw5_doc_height(doc: &Handle) -> f32 {
    let tree = build_layout_tree(doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    measure_document(&tree, &mut font_system, 500.0, 400.0, &mut next_id, &mut cache, deadline, &mut incomplete).h
}

#[test]
fn qw5_hidden_attribute_is_display_none_author_display_wins() {
    let h = |body: &str| qw5_doc_height(&parse_html(&format!("<html><body style=\"margin:0\">{body}</body></html>")));
    let base = h("<div style=\"height:20px\"></div>");
    assert!(base >= 20.0);
    assert_eq!(h("<div style=\"height:20px\"></div><p hidden style=\"height:50px\">x</p>"), base, "hidden p is not laid out");
    assert_eq!(h("<div style=\"height:20px\"></div><p hidden=\"\" id=a class=tags>x</p>"), base);
    // author display wins over the UA rule
    assert!(h("<div style=\"height:20px\"></div><p hidden style=\"display:block;height:50px;margin:0\">x</p>") >= base + 50.0);
    // until-found is not display:none from this rule
    assert!(h("<p hidden=\"until-found\" style=\"height:50px;margin:0\">x</p>") >= 50.0);
    assert_eq!(h("<div style=\"height:20px\"></div><datalist><option>a</option></datalist><noembed>zz</noembed>"), base);
}

#[test]
fn qw5_js_hidden_toggle_relayouts_and_node_stays_in_dom() {
    let doc = parse_html("<html><body style=\"margin:0\"><p id=t hidden style=\"margin:0;height:40px\"><a href=\"/x\">x</a></p></body></html>");
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = start_scripts(
        &doc,
        &[src("var t=document.getElementById('t'); console.log(t.hidden, document.querySelectorAll('#t a[href]').length);\
                window.show=function(){t.hidden=false;}; window.hide=function(){t.hidden=true;};")],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
    )
    .expect("engine");
    assert_eq!(out.borrow().as_slice(), ["log:true 1"]);
    assert_eq!(qw5_doc_height(&doc), 0.0, "hidden: nothing laid out");
    engine.eval("t", "show()");
    assert!(qw5_doc_height(&doc) >= 40.0, "after hidden=false it is laid out");
    engine.eval("t", "hide()");
    assert_eq!(qw5_doc_height(&doc), 0.0);
}

/// QW4: horizontal margin/padding on non-replaced inline elements advance
/// the line: three adjacent links differ in x by text width + margin +
/// padding on both sides.
#[test]
fn qw4_inline_margin_and_padding_advance_adjacent_links() {
    let html = r##"<!doctype html><meta charset=utf-8><style>a{margin-right:2rem;padding:0 1rem;background:#cde}</style>
<p><a href="#">one</a><a href="#">two</a><a href="#">three</a></p>"##;
    let laid = measure_fixture(html, 800.0);
    let run = find_mixed_inline(&laid).expect("mixed inline run not found");
    let words: Vec<(f32, f32)> = mixed_items(run)
        .iter()
        .filter_map(|i| match i {
            MixedPositioned::Word { x, w, .. } => Some((*x, *w)),
            _ => None,
        })
        .collect();
    assert_eq!(words.len(), 3, "three words");
    for pair in words.windows(2) {
        // end padding 16 + end margin 32 + next start padding 16 = 64
        let gap = pair[1].0 - (pair[0].0 + pair[0].1);
        assert!((gap - 64.0).abs() < 0.5, "gap between links should be 64px, got {gap}");
    }
    let spacers: Vec<f32> = mixed_items(run)
        .iter()
        .filter_map(|i| match i {
            MixedPositioned::Embedded(l) if l.style.background_color.is_some() => Some(l.w),
            _ => None,
        })
        .collect();
    assert!(spacers.iter().all(|w| (*w - 16.0).abs() < 0.5) && spacers.len() == 6, "padding backgrounds: {spacers:?}");
}

/// QW4: on a wrapped inline element the margin applies at the start of the
/// first fragment and the end of the last, not at every line.
#[test]
fn qw4_inline_margin_wraps_only_at_outer_edges() {
    let html = r##"<style>a{margin:0 20px}</style><div style="width:120px"><a href="#">aaaa bbbb cccc dddd</a></div>"##;
    let laid = measure_fixture(html, 800.0);
    let run = find_mixed_inline(&laid).expect("mixed inline run not found");
    let words: Vec<(f32, f32)> = mixed_items(run)
        .iter()
        .filter_map(|i| if let MixedPositioned::Word { x, y, .. } = i { Some((*x, *y)) } else { None })
        .collect();
    assert!(words.len() >= 2);
    assert!((words[0].0 - 28.0).abs() < 0.5, "first fragment starts after margin-left: {words:?}");
    let second_line_x = words.iter().filter(|p| (p.1 - words[0].1).abs() > 1.0).map(|p| p.0).fold(f32::MAX, f32::min);
    assert!((second_line_x - 8.0).abs() < 0.5, "wrapped line has no margin at its start: {second_line_x} {words:?}");
}

fn qy18_find_box(l: &Laid, w: f32, h: f32) -> Option<(f32, f32)> {
    if (l.w - w).abs() < 0.01 && (l.h - h).abs() < 0.01 {
        return Some((l.x, l.y));
    }
    let kids: Vec<&Laid> = match &l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().collect(),
        _ => Vec::new(),
    };
    kids.into_iter().chain(l.positioned.iter()).find_map(|k| qy18_find_box(k, w, h))
}

#[test]
fn qy18_transform_parses_translate_only() {
    let t = |s: &str| parse_declarations(s).transform;
    assert!(t("transform: none") == Some(vec![]));
    assert!(t("transform: translateX(100%)") == Some(vec![(Length::Percent(1.0), Length::Px(0.0))]));
    assert!(t("transform: translate(10px, 20px)") == Some(vec![(Length::Px(10.0), Length::Px(20.0))]));
    assert!(t("transform: translate(5px)") == Some(vec![(Length::Px(5.0), Length::Px(0.0))]));
    assert!(t("transform: translateY(-50%)") == Some(vec![(Length::Px(0.0), Length::Percent(-0.5))]));
    assert!(t("transform: translate3d(1px, 2px, 3px)") == Some(vec![(Length::Px(1.0), Length::Px(2.0))]));
    // Anything with an unsupported function is treated as none.
    assert!(t("transform: rotate(45deg)") == Some(vec![]));
    assert!(t("transform: translateX(10px) scale(2)") == Some(vec![]));
    assert!(t("transform: bogus").is_none());
}

#[test]
fn qy18_translate_offsets_paint_position_and_hit_test() {
    let base = measure_fixture(
        r#"<html><body style="margin:0"><div style="position:relative;width:400px;height:200px"><div style="position:absolute;right:0;top:0;width:100px;height:50px;background:red">x</div></div></body></html>"#,
        400.0,
    );
    let (bx, by) = qy18_find_box(&base, 100.0, 50.0).expect("box");
    // 100% of its own width pushes it fully off the right edge of the 400px viewport.
    let moved = measure_fixture(
        r#"<html><body style="margin:0"><div style="position:relative;width:400px;height:200px"><div style="position:absolute;right:0;top:0;width:100px;height:50px;background:red;transform:translateX(100%)">x</div></div></body></html>"#,
        400.0,
    );
    let (mx, my) = qy18_find_box(&moved, 100.0, 50.0).expect("box");
    assert!((mx - (bx + 100.0)).abs() < 0.01 && (my - by).abs() < 0.01, "{bx},{by} -> {mx},{my}");
    assert!(mx >= 400.0, "drawer is off-screen");
    // translate(10px,20px) on an in-flow box moves it by exactly (10,20) and not the flow.
    let html = |t: &str| format!(
        r#"<html><body style="margin:0"><div id="a" style="width:100px;height:50px;{t}">a</div><div id="b" style="width:60px;height:30px">b</div></body></html>"#
    );
    let plain = measure_fixture(&html(""), 400.0);
    let tr = measure_fixture(&html("transform:translate(10px,20px)"), 400.0);
    let (px, py) = qy18_find_box(&plain, 100.0, 50.0).unwrap();
    let (tx, ty) = qy18_find_box(&tr, 100.0, 50.0).unwrap();
    assert!((tx - px - 10.0).abs() < 0.01 && (ty - py - 20.0).abs() < 0.01);
    let (p2x, p2y) = qy18_find_box(&plain, 60.0, 30.0).unwrap();
    let (t2x, t2y) = qy18_find_box(&tr, 60.0, 30.0).unwrap();
    assert!((p2x - t2x).abs() < 0.01 && (p2y - t2y).abs() < 0.01, "layout of siblings is unchanged");
    // Hit test follows the offset: the new spot hits <a>, the old corner no longer does.
    let doc = parse_html(&html("transform:translate(10px,20px)"));
    let tree = build_layout_tree(&doc, 1920.0);
    let mut fc = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut inc) = (0usize, false);
    let laid = measure(
        &tree, &mut fc, 0.0, 0.0, 400.0, None, &mut next_id, &mut cache,
        std::time::Instant::now() + std::time::Duration::from_secs(5), &mut inc, &mut Vec::new(),
        &Floats::default(), None,
    );
    let a = find_by_tag(&doc, "div").unwrap();
    let hit_new = hit_test_element(&laid, 10.0 + 90.0, 20.0 + 40.0, &cache, &HashMap::new(), 0.0, 600.0);
    assert_eq!(hit_new, node_id(&a), "the translated position hits the element");
    let hit_old = hit_test_element(&laid, 3.0, 3.0, &cache, &HashMap::new(), 0.0, 600.0);
    assert_ne!(hit_old, node_id(&a), "the original top-left corner no longer hits it");
}

// ---- QSD3: real ShadowRoot (not rendered).

#[test]
fn qsd3_attach_shadow_basics() {
    let log = run_in_html(
        "<html><body><div id=h><b id=l>light</b></div><i id=bad></i></body></html>",
        r#"const r = [], h = document.getElementById('h');
         const sr = h.attachShadow({ mode: 'open', delegatesFocus: true });
         r.push(sr instanceof ShadowRoot, sr instanceof DocumentFragment, sr.nodeType, sr.host === h, sr.mode, sr.delegatesFocus, h.shadowRoot === sr, sr.activeElement);
         try { h.attachShadow({ mode: 'open' }); r.push('no'); } catch (e) { r.push(e.name); }
         try { document.getElementById('bad').attachShadow({ mode: 'open' }); r.push('no'); } catch (e) { r.push(e.name); }
         try { h.attachShadow({ mode: 'x' }); r.push('no'); } catch (e) { r.push(e.name); }
         const c = document.createElement('x-c'), cs = c.attachShadow({ mode: 'closed' });
         r.push(c.shadowRoot, cs.mode, cs.host === c);
         sr.innerHTML = '<p id=in>hi</p><span class=k>k</span>';
         r.push(sr.getElementById('in').localName, sr.querySelector('.k').localName, sr.querySelectorAll('*').length);
         r.push(document.querySelector('#in'), document.getElementById('in'), h.innerHTML, h.children.length, h.childNodes.length);
         r.push(sr.getRootNode() === sr, sr.firstChild.getRootNode() === sr, sr.getRootNode({ composed: true }) === document, sr.firstChild.getRootNode({ composed: true }) === document);
         r.push(sr.isConnected, sr.firstChild.isConnected, cs.isConnected);
         r.push(sr.adoptedStyleSheets.length);
         const sh = new CSSStyleSheet(); sr.adoptedStyleSheets = [sh]; r.push(sr.adoptedStyleSheets[0] === sh);
         try { sr.adoptedStyleSheets = [{}]; r.push('no'); } catch (e) { r.push(e.name); }
         console.log(r.join('|'));"#,
    );
    assert_eq!(
        log.last().map(String::as_str),
        Some("log:true|true|11|true|open|true|true||NotSupportedError|NotSupportedError|TypeError||closed|true|p|span|2|||<b id=\"l\">light</b>|1|1|true|true|true|true|true|true|false|0|true|TypeError"),
        "{log:?}"
    );
    assert_eq!(log.iter().filter(|l| l.contains("shadow root rendering")).count(), 1, "{log:?}");
}

#[test]
fn qsd3_custom_element_in_shadow_and_event() {
    let log = run_in_html(
        "<html><body><div id=h></div></body></html>",
        r#"class XC extends HTMLElement { connectedCallback() { console.log('conn'); } disconnectedCallback() { console.log('disc'); } }
         customElements.define('x-c', XC);
         const h = document.getElementById('h'), sr = h.attachShadow({ mode: 'open' });
         const x = document.createElement('x-c'); sr.appendChild(x);
         const seen = [];
         h.addEventListener('ping', function (e) { seen.push('host:' + e.target.localName); });
         document.addEventListener('ping', function () { seen.push('doc'); });
         x.dispatchEvent(new Event('ping', { bubbles: true, composed: true }));
         x.dispatchEvent(new Event('ping', { bubbles: true }));
         console.log(seen.join(','));
         sr.removeChild(x);
         const d = document.createElement('div'), ds = d.attachShadow({ mode: 'open' }), y = document.createElement('x-c');
         ds.appendChild(y); document.body.appendChild(d);"#,
    );
    assert!(log.contains(&"log:host:x-c,doc".to_string()), "{log:?}");
    assert!(log.contains(&"log:disc".to_string()), "{log:?}");
    assert_eq!(log.iter().filter(|l| *l == "log:conn").count(), 2, "{log:?}");
}

// ---- QY21: define() looks up only the new name; scans use the native candidate walk.

#[test]
fn qy21_define_is_linear_and_upgrade_order_unchanged() {
    let mut html = String::from("<html><body><div id=root>");
    for i in 0..1500 {
        if i % 5 == 0 {
            html.push_str(&format!("<x-n{} id=e{}></x-n{}>", i % 600, i, i % 600));
        } else {
            html.push_str(&format!("<span id=e{}></span>", i));
        }
    }
    html.push_str("<button id=b is=x-btn></button></div></body></html>");
    let log = run_in_html(
        &html,
        r#"const order = [];
         for (let i = 0; i < 600; i++) {
           customElements.define('x-n' + i, class extends HTMLElement { connectedCallback() { order.push(this.id); } });
         }
         customElements.define('x-btn', class extends HTMLButtonElement { connectedCallback() { order.push('btn'); } }, { extends: 'button' });
         console.log(__ce_stats.defineQueries, order.length);
         const root = document.getElementById('root');
         const frag = document.createElement('div');
         for (let i = 0; i < 300; i++) { const e = document.createElement('x-n' + i); e.id = 'n' + i; frag.appendChild(e); }
         root.appendChild(frag);
         console.log(order.length, order[order.length - 1]);
         const t = document.createElement('div');
         t.innerHTML = '<x-n1 id=h1></x-n1><p><x-n2 id=h2></x-n2></p>';
         root.appendChild(t);
         console.log(order.slice(-2).join(','));"#,
    );
    // 601 defines -> exactly 601 one-name queries (not 601 growing selectors).
    assert_eq!(log[0], "log:601 301", "{log:?}");
    assert_eq!(log[1], "log:601 n299", "{log:?}");
    assert_eq!(log[2], "log:h1,h2", "{log:?}");
}

/// QY22(a): compile time is not charged to the page-load run budget. A fake
/// clock makes the compile "take" 4.99 s of a 5 s budget; a script that then
/// needs a few ms of run time still completes.
#[test]
fn qy22_compile_time_is_not_counted_against_the_run_budget() {
    use crate::js::{AFTER_COMPILE, FAKE_CLOCK};
    FAKE_CLOCK.with(|f| f.set(Some((1_000_000_000, 1_000_000))));
    AFTER_COMPILE.with(|c| c.set(Some(|| FAKE_CLOCK.with(|f| f.set(f.get().map(|(n, s)| (n + 4_990_000_000, s)))))));
    let log = run_collecting(&[src("var s=0;for(var i=0;i<300000;i++){s+=i}console.log('done');")]);
    FAKE_CLOCK.with(|f| f.set(None));
    AFTER_COMPILE.with(|c| c.set(None));
    assert!(log.iter().any(|l| l == "log:done"), "{log:?}");
    assert!(!log.iter().any(|l| l.starts_with("error:")), "{log:?}");
}

/// QY22(a): a trip in the run phase reports compile and run time apart.
#[test]
fn qy22_interrupt_message_splits_compile_and_run_time() {
    use crate::js::{AFTER_COMPILE, FAKE_CLOCK};
    FAKE_CLOCK.with(|f| f.set(Some((1_000_000_000, 1_000_000))));
    AFTER_COMPILE.with(|c| c.set(Some(|| FAKE_CLOCK.with(|f| f.set(f.get().map(|(n, s)| (n + 3_000_000_000, s)))))));
    let log = run_collecting(&[src("for(;;){}")]);
    FAKE_CLOCK.with(|f| f.set(None));
    AFTER_COMPILE.with(|c| c.set(None));
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert_eq!(errs.len(), 1, "{log:?}");
    assert!(errs[0].contains("compiled in 3.0 s, ran 5.0 s"), "{log:?}");
    assert!(errs[0].contains("5 s CPU-time page-load budget"), "{log:?}");
}

/// QY22(a): hard cap on the source size.
#[test]
fn qy22_oversized_script_is_not_compiled() {
    let big = format!("console.log('ran');/*{}*/", "x".repeat(33 * 1024 * 1024));
    let log = run_collecting(&[src(&big), src("console.log('after');")]);
    assert!(log.iter().any(|l| l.contains("not compiled") && l.contains("32 MB cap")), "{log:?}");
    assert!(!log.iter().any(|l| l == "log:ran"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:after"), "{log:?}");
}

/// QY22(b): 1000 promise jobs queued behind a tripped busy loop log ONE
/// interrupt line (the task's), and the jobs get a fresh budget and run.
#[test]
fn qy22_promise_jobs_after_a_tripped_task_get_a_fresh_budget() {
    let log = run_with_timers(
        "var n=0;setTimeout(()=>{for(var i=0;i<1000;i++)Promise.resolve().then(()=>{n++});\
         for(;;){}},0);setTimeout(()=>console.log('n='+n),1);",
    );
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert_eq!(errs.len(), 1, "{log:?}");
    assert!(errs[0].contains("timer stopped: exceeded the 500 ms"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:n=1000"), "{log:?}");
}

/// QY22(b): jobs that themselves overrun the fresh budget are counted in
/// one line, not logged once each.
#[test]
fn qy22_tripped_job_drain_logs_one_line_with_a_count() {
    // QY22t: the budget is CPU time, so burn FAKE CPU (the test seam) instead of spinning on the
    // wall clock, which flaked under load (R41).
    let log = super::js_core_tests::qy46_run(
        // QY46: each job has its own 500 ms budget now, so each must overrun it by itself.
        "setTimeout(()=>{for(var i=0;i<4;i++)Promise.resolve().then(()=>{__test_burn(700);for(var k=0;k<3e6;k++){}});},0);",
        0,
        10_000,
    );
    // QY44a: a trip stops the drain and defers the rest, so one line per
    // turn that tripped (each names the kill and the deferral), never one per job.
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert!(!errs.is_empty() && errs.len() < 100, "{log:?}");
    assert!(errs.iter().all(|e| e.contains("promise jobs stopped") && e.contains("1 job killed")), "{log:?}");
    // QV8-3b: V8 drops the rest of the queue instead of deferring it (R39 C2).
    assert!(errs[0].contains("remaining jobs dropped"), "{log:?}");
}

/// QY44a (R29): after one promise job is killed by the budget the drain stops
/// and the later jobs run in a later turn with a fresh budget, so their
/// `finally` blocks run (none starts on an expired deadline).
#[test]
fn qy44a_tripped_job_does_not_kill_later_jobs() {
    let log = run_with_timers(
        "var stack=[],ran=[];\
         function withConsumer(tag,f){stack.push(tag);try{return f();}finally{stack.pop();}}\
         setTimeout(function(){\
           Promise.resolve().then(function(){var x=0;for(;;){x++;}});\
           Promise.resolve().then(function(){withConsumer('job2',function(){var s=0;for(var i=0;i<200000;i++)s+=i;ran.push('job2');});});\
           Promise.resolve().then(function(){withConsumer('job3',function(){var s=0;for(var i=0;i<200000;i++)s+=i;ran.push('job3');});});\
         },0);\
         setTimeout(function(){console.log('completed:['+ran.join(',')+'] stack:['+stack.join(',')+']');},50);",
    );
    // QV8-3b: on V8 the killed job takes the rest of the queue with it (R39 C2): job2 and job3
    // are dropped (never started, so no `finally` is skipped: the stack is still clean).
    assert!(log.iter().any(|l| l == "log:completed:[] stack:[]"), "{log:?}");
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert_eq!(errs.len(), 1, "{log:?}");
    assert!(errs[0].contains("1 job killed, remaining jobs dropped"), "{log:?}");
}

// ---- QY53e2: IntersectionObserver evaluates on the App's own layout in a window ----

const QY53E2_BODY: &str = "<style>.big{height:400px}.small{height:20px}#sc{height:100px;overflow:auto}#cl{height:60px;overflow:hidden}\
    #fx{position:fixed;top:700px;left:10px;width:50px;height:100px}#tr{height:40px;transform:translate(0,30px)}</style>\
    <div id=shift class=big></div>\
    <div id=sc><div style='height:80px'></div><div id=in1 style='height:50px'></div></div>\
    <div id=cl><div style='height:40px'></div><div id=in2 style='height:50px'></div></div>\
    <div id=tr></div><div id=fx></div><div id=gone style='display:none;height:10px'></div>\
    <div id=far style='margin-top:450px;height:30px'></div><span id=sp>inline</span>";

const QY53E2_CODE: &str = r#"const f = n => n.toFixed(1);
    const io = new IntersectionObserver(es => es.forEach(e => {
      const b = e.boundingClientRect, i = e.intersectionRect, r = e.rootBounds;
      console.log(Math.round(performance.now() / 10) * 10, e.target.id, e.isIntersecting, f(e.intersectionRatio), [b.x, b.y, b.width, b.height, i.x, i.y, i.width, i.height, r.width, r.height].map(f).join());
    }), { threshold: [0, 0.25, 0.5, 0.75, 1] });
    for (const id of ['in1', 'in2', 'tr', 'fx', 'gone', 'far', 'sp', 'shift']) io.observe($(id));
    setTimeout(() => { $('shift').className = 'small'; }, 50);"#;

/// mode 0: forced layout only; 1: the App's layout is handed over after the write; 2: the DOM changes
/// again after the App built its layout (stale).
fn qy53e2_run(mode: u8) -> (Vec<String>, usize) {
    // QFLK2: pin the measured layout cost (the IO recheck deadline is max(100, 4 x cost) ms).
    crate::js_geom::TEST_FIXED_COST_MS.with(|c| c.set(Some(1.0)));
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let doc = parse_html(&format!("<html><body style='margin:0'>{QY53E2_BODY}</body></html>"));
    crate::js_geom::GEOM_LAYOUTS.with(|c| c.set(0));
    let engine = crate::js::start_scripts_clocked(
        &doc,
        &[src(&format!("const $ = id => document.getElementById(id);\n{QY53E2_CODE}"))],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    let step = |until: f64| {
        for _ in 0..1000 {
            let Some(due) = crate::event_loop::headless_next_due(&engine) else { break };
            if due > until {
                break;
            }
            clock.set(clock.get().max(due));
            crate::event_loop::headless_turn(&engine, clock.get());
        }
    };
    step(60.0);
    if mode != 0 {
        let stamp = crate::js_geom::render_stamp();
        let tree = build_layout_tree_with_resources(&doc, 1024.0, Some(768.0), &crate::js_style::page_css(), &HashMap::new());
        let mut fs = new_font_system();
        let (mut cache, mut next_id, mut incomplete) = (ShapeCache::new(), 0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure_document(&tree, &mut fs, 1024.0, 768.0, &mut next_id, &mut cache, deadline, &mut incomplete);
        assert!(!incomplete);
        if mode == 2 {
            engine.eval("t", "document.getElementById('far').style.marginTop = '2000px';");
        }
        engine.rendered_geometry(&laid, stamp, 1024, 768);
    }
    step(1e9);
    let log = out.borrow().clone();
    (log, crate::js_geom::GEOM_LAYOUTS.with(|c| c.get()))
}

#[test]
fn qy53e2_app_layout_gives_the_same_entries_as_a_forced_layout_without_forcing() {
    if special_cases_bypassed() {
        return;
    }
    let (forced, forced_layouts) = qy53e2_run(0);
    let (app, app_layouts) = qy53e2_run(1);
    assert!(forced.iter().any(|l| l.starts_with("log:170 far true")), "initial entries plus the ones the write changed: {forced:?}");
    assert!(forced.iter().any(|l| l.starts_with("log:20 in1 ")) && forced.iter().any(|l| l.contains(" tr ")), "{forced:?}");
    // Same entries; the App-layout one is delivered right after the write instead of at the deadline.
    let untimed = |v: &[String]| v.iter().map(|l| l.split_once(' ').unwrap().1.to_string()).collect::<Vec<_>>();
    assert_eq!(untimed(&app), untimed(&forced), "IO entries must not depend on where the rects came from");
    assert!(app.iter().any(|l| l.starts_with("log:50 far true")), "{app:?}");
    assert_eq!(forced_layouts, 2, "initial + the deadline recheck");
    assert_eq!(app_layouts, 1, "the recheck used the App layout: only the initial layout was forced");
}

#[test]
fn qy53e2_stale_app_layout_falls_back_to_a_forced_layout() {
    let (log, layouts) = qy53e2_run(2);
    assert_eq!(layouts, 2, "a layout built before the last write is not used: {log:?}");
    // The second write pushed `far` far below the window: it never reads as visible.
    assert!(!log.iter().any(|l| l.contains(" far true")), "{log:?}");
}

#[test]
fn qy53e2_new_engine_drops_the_kept_app_layout() {
    if special_cases_bypassed() {
        return;
    }
    let doc = parse_html("<html><body><div id=a style='height:10px'></div></body></html>");
    let tree = build_layout_tree_with_resources(&doc, 1024.0, Some(768.0), &crate::js_style::page_css(), &HashMap::new());
    let mut fs = new_font_system();
    let (mut cache, mut next_id, mut incomplete) = (ShapeCache::new(), 0usize, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut fs, 1024.0, 768.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    crate::js_geom::set_rendered(&laid, crate::js_geom::render_stamp(), 1024, 768);
    assert!(crate::js_geom::rendered_ok(1024, 768));
    assert!(!crate::js_geom::rendered_ok(956, 485), "another viewport is not the same layout");
    let _e = start_scripts(&doc, &[src("1")], Rc::new(|_, _| {})).unwrap();
    assert!(!crate::js_geom::rendered_ok(1024, 768), "navigation (a new engine) clears it");
}

#[test]
fn qy53e2_window_without_an_app_layout_falls_back_to_forcing_at_the_normal_deadline() {
    crate::js::set_window_viewport(1024, 768);
    let (log, layouts) = qy53e1_run(
        "<div id=t style='height:100px'></div>",
        r#"new IntersectionObserver(es => es.forEach(e => console.log(Math.round(performance.now()), e.isIntersecting))).observe($('t'));
           setTimeout(() => { $('t').style.marginTop = '900px'; }, 50);"#,
    );
    crate::js::set_window_viewport(0, 0);
    // No App layout arrives (headless test): the recheck is forced at the normal deadline, as without a window.
    assert_eq!(log.len(), 2, "{log:?}");
    let t: i64 = log[1].strip_prefix("log:").unwrap().split_once(' ').unwrap().0.parse().unwrap();
    assert!((150..=2100).contains(&t) && t < 1900, "delivered at {t} ms");
    assert_eq!(layouts, 2);
}
