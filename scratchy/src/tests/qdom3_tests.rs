#[allow(unused_imports)]
use super::*;

/// QDOM3: ParentNode.moveBefore pre-move checks and return value.
#[test]
fn qdom3_move_before_checks() {
    let (log, _) = mutate(
        "<html><body><div id=a><p id=p>x</p><b id=b></b>t</div><div id=c></div></body></html>",
        "var a = document.getElementById('a'), c = document.getElementById('c'), p = document.getElementById('p'), b = document.getElementById('b');\
         var err = function (f) { try { f(); return 'ok'; } catch (e) { return e.name; } };\
         console.log(typeof a.moveBefore, 'moveBefore' in document.createTextNode('x'), 'moveBefore' in document, 'moveBefore' in document.createDocumentFragment(), a.moveBefore.length);\
         console.log(err(function () { a.moveBefore(p); }), err(function () { a.moveBefore({}, null); }), err(function () { a.moveBefore(p, {}); }));\
         console.log(err(function () { p.moveBefore(a, null); }), err(function () { a.moveBefore(a, null); }), err(function () { c.moveBefore(p, b); }));\
         console.log(err(function () { c.moveBefore(document.createElement('i'), null); }), err(function () { c.moveBefore(document.createDocumentFragment(), null); }));\
         console.log(err(function () { document.moveBefore(document.body, null); }), err(function () { var m = document.createComment('m'); document.insertBefore(m, document.documentElement); document.moveBefore(m, null); }));\
         console.log(a.moveBefore(p, p), a.firstChild === p, a.moveBefore(p, b), a.firstChild === p);\
         console.log(c.moveBefore(p, null), c.firstChild === p, a.childNodes.length, a.moveBefore(a.lastChild, a.firstChild), a.firstChild.nodeType);",
    );
    assert_eq!(
        log,
        [
            "log:function false true true 2",
            "log:TypeError TypeError TypeError",
            "log:HierarchyRequestError HierarchyRequestError NotFoundError",
            "log:HierarchyRequestError HierarchyRequestError",
            "log:HierarchyRequestError ok",
            "log:undefined true undefined true",
            "log:undefined true 2 undefined 3",
        ],
        "{log:?}"
    );
}

/// QDOM3: a move re-runs no insertion steps (script, iframe) and keeps focus; live Ranges and NodeIterators
/// are updated as for a removal + insertion.
#[test]
fn qdom3_move_before_keeps_state_and_updates_live_objects() {
    let (log, _) = mutate(
        "<html><body><div id=a><p id=p>x<span id=s>y</span></p><b id=b></b></div><div id=c></div>\
         <input id=i><iframe id=f srcdoc='<p>f</p>'></iframe></body></html>",
        "var a = document.getElementById('a'), c = document.getElementById('c'), p = document.getElementById('p'), b = document.getElementById('b'), s = document.getElementById('s');\
         var i = document.getElementById('i'), f = document.getElementById('f'); i.focus();\
         var range = new Range(); range.setStart(s, 0); range.setEnd(b, 0);\
         var it = document.createNodeIterator(a, NodeFilter.SHOW_ELEMENT); it.nextNode(); it.nextNode(); it.nextNode();\
         var scr = document.createElement('script'); scr.textContent = 'window.ran = (window.ran || 0) + 1'; c.appendChild(scr);\
         var ranBefore = window.ran, fw = f.contentWindow, inner = f.contentDocument;\
         document.body.moveBefore(i, f); c.moveBefore(scr, null); c.moveBefore(p, null); document.body.moveBefore(f, i);\
         console.log(range.startContainer === a, range.startOffset, range.endContainer === b, range.endOffset, it.referenceNode.id);\
         console.log(document.activeElement === i, window.ran, ranBefore, f.contentWindow === fw, f.contentDocument === inner);\
         console.log(p.parentNode === c, s.parentNode === p, i.nextSibling === f, f.previousSibling === i);\
         range.setStart(c, 0); range.setEnd(c, 1); a.moveBefore(b, null); c.moveBefore(p, scr); a.moveBefore(b, null);\
         console.log(range.startOffset, range.endOffset, c.firstChild === p);",
    );
    assert_eq!(
        log,
        [
            "log:true 0 true 0 a",
            "log:true 1 1 true true",
            "log:true true false false",
            "log:0 2 true",
        ],
        "{log:?}"
    );
}

/// QDOM3: one MutationObserver record pair, and custom element reactions.
#[test]
fn qdom3_move_before_mutation_records_and_custom_elements() {
    let (log, _) = mutate(
        "<html><body><div id=a><p id=p></p></div><div id=c></div></body></html>",
        "var a = document.getElementById('a'), c = document.getElementById('c'), p = document.getElementById('p'), ev = [];\
         var mk = function (n, move) { customElements.define(n, move\
           ? class extends HTMLElement { connectedMoveCallback() { ev.push(n + ' move'); } connectedCallback() { ev.push(n + ' conn'); } disconnectedCallback() { ev.push(n + ' disc'); } }\
           : class extends HTMLElement { connectedCallback() { ev.push(n + ' conn'); } disconnectedCallback() { ev.push(n + ' disc'); } }); };\
         mk('x-plain', false); mk('x-move', true);\
         var x1 = document.createElement('x-plain'), x2 = document.createElement('x-move'); x1.appendChild(x2); a.appendChild(x1);\
         ev.length = 0;\
         var recs = [];\
         var mo = new MutationObserver(function (l) { recs.push(l.map(function (r) { return r.target.id + ':+' + r.addedNodes.length + '-' + r.removedNodes.length; }).join(',')); });\
         mo.observe(a, { childList: true }); mo.observe(c, { childList: true }); mo.observe(p, { childList: true });\
         c.moveBefore(x1, null);\
         console.log(ev.join());\
         Promise.resolve().then(function () { console.log(recs.length, recs[0]); });",
    );
    assert_eq!(log, ["log:x-plain disc,x-plain conn,x-move move", "log:1 a:+0-1,c:+1-0"], "{log:?}");
}
