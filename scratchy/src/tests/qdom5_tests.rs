// QDOM5: Range content operations (clone/extract/delete/insertNode/surroundContents) and live range fix-ups.
use super::*;

const HDR: &str = "function thr(f) { try { f(); return 'no'; } catch (e) { return e.name; } }\
    function pt(r) { return r.startContainer.nodeName + ':' + r.startOffset + '-' + r.endContainer.nodeName + ':' + r.endOffset; }\
    function ser(n) { return n.nodeType === 3 ? n.data : n.nodeType === 1 ? '<' + n.localName + '>' + Array.prototype.map.call(n.childNodes, ser).join('') + '</' + n.localName + '>' : Array.prototype.map.call(n.childNodes, ser).join(''); }";

#[test]
fn qdom5_clone_extract_delete() {
    let log = run_in_html(
        "<html><body><div id=d><p id=a>Hello <b id=b>big</b> world</p><i id=c>tail</i></div></body></html>",
        &format!(
            "{HDR}\
             var d = document.getElementById('d'), a = document.getElementById('a');\
             var r = document.createRange(); r.setStart(a.firstChild, 2); r.setEnd(a.lastChild, 3);\
             console.log(ser(r.cloneContents()), ser(d), pt(r));\
             var f = r.extractContents(); console.log(ser(f), ser(d), pt(r), r.collapsed);\
             r = document.createRange(); r.setStart(d.firstChild.firstChild, 1); r.setEnd(document.getElementById('c').firstChild, 2);\
             r.deleteContents(); console.log(ser(d), pt(r));\
             r = document.createRange(); r.selectNodeContents(d); var f2 = r.extractContents(); console.log(ser(f2), ser(d), pt(r));\
             var t = document.createTextNode('abcdef'); d.appendChild(t); r = document.createRange(); r.setStart(t, 1); r.setEnd(t, 4);\
             console.log(ser(r.cloneContents()), ser(r.extractContents()), t.data, pt(r));\
             var dt = document.implementation.createDocumentType('html', '', ''); var doc2 = document.implementation.createDocument(null, 'x', dt);\
             r = doc2.createRange(); r.setStart(doc2, 0); r.setEnd(doc2, 2); console.log(thr(function () {{ r.cloneContents(); }}), thr(function () {{ r.deleteContents(); }}), thr(function () {{ r.extractContents(); }}));"
        ),
    );
    assert_eq!(
        log,
        [
            "log:llo <b>big</b> wo <div><p>Hello <b>big</b> world</p><i>tail</i></div> #text:2-#text:3",
            "log:llo <b>big</b> wo <div><p>Herld</p><i>tail</i></div> P:1-P:1 true",
            "log:<div><p>H</p><i>il</i></div> DIV:1-DIV:1",
            "log:<p>H</p><i>il</i> <div></div> DIV:0-DIV:0",
            "log:bcd bcd aef #text:1-#text:1",
            "log:HierarchyRequestError HierarchyRequestError HierarchyRequestError",
        ]
    );
}

#[test]
fn qdom5_insert_surround() {
    let log = run_in_html(
        "<html><body><div id=d>abcdef<i>x</i></div></body></html>",
        &format!(
            "{HDR}\
             var d = document.getElementById('d'), t = d.firstChild;\
             var r = document.createRange(); r.setStart(t, 2); r.setEnd(t, 2);\
             var s = document.createElement('s'); s.textContent = 'S'; r.insertNode(s);\
             console.log(ser(d), pt(r), d.childNodes.length);\
             r = document.createRange(); r.setStart(d.lastChild.firstChild, 0); r.setEnd(d.lastChild.firstChild, 1);\
             var u = document.createElement('u'); r.surroundContents(u); console.log(ser(d), pt(r));\
             r = document.createRange(); r.setStart(d.firstChild, 1); r.setEnd(d.lastChild, 0);\
             var w = document.createElement('w'); console.log(thr(function () {{ r.surroundContents(w); }}), thr(function () {{ r.surroundContents(document); }}), thr(function () {{ r.insertNode(document.createAttribute('q')); }}));\
             var fr = document.createDocumentFragment(); fr.appendChild(document.createElement('e')); fr.appendChild(document.createElement('f'));\
             r = document.createRange(); r.setStart(d, 1); r.collapse(true); r.insertNode(fr); console.log(ser(d), pt(r));\
             var c = document.createComment('c'); r = document.createRange(); r.setStart(c, 0); console.log(thr(function () {{ r.insertNode(s); }}));\
             var at = document.createElement('div'); at.setAttribute('x', 'abc'); var an = at.getAttributeNode('x');\
             r = document.createRange(); r.selectNodeContents(an); console.log(thr(function () {{ r.insertNode(s); }}), r.collapsed, ser(r.cloneContents()));\
             var dc = document.implementation.createDocument(null, 'x'); r = dc.createRange(); r.setStart(dc, 0); console.log(thr(function () {{ r.insertNode(dc.createElement('y')); }}));"
        ),
    );
    assert_eq!(
        log,
        [
            "log:<div>ab<s>S</s>cdef<i>x</i></div> #text:2-DIV:2 4",
            "log:<div>ab<s>S</s>cdef<i><u>x</u></i></div> I:1-I:2",
            "log:InvalidStateError InvalidStateError HierarchyRequestError",
            "log:<div>ab<e></e><f></f><s>S</s>cdef<i><u>x</u></i></div> DIV:1-DIV:3",
            "log:HierarchyRequestError",
            "log:HierarchyRequestError true ",
            "log:HierarchyRequestError",
        ]
    );
}

#[test]
fn qdom5_live_ranges() {
    let log = run_in_html(
        "<html><body><div id=d>abcdef<b>x</b><i>y</i></div></body></html>",
        &format!(
            "{HDR}\
             var d = document.getElementById('d'), t = d.firstChild, b = d.childNodes[1], i = d.childNodes[2];\
             var r = document.createRange(); r.setStart(t, 4); r.setEnd(d, 3);\
             t.deleteData(1, 2); console.log(pt(r), t.data);\
             t.insertData(0, 'ZZ'); console.log(pt(r));\
             t.replaceData(0, 100, 'q'); console.log(pt(r));\
             d.insertBefore(document.createElement('p'), b); console.log(pt(r));\
             d.removeChild(d.firstChild.nextSibling); console.log(pt(r));\
             r.setStart(b.firstChild, 1); b.remove(); console.log(pt(r));\
             var t2 = document.createTextNode('0123456789'); d.insertBefore(t2, d.firstChild);\
             var r2 = document.createRange(); r2.setStart(t2, 3); r2.setEnd(t2, 8); var r3 = document.createRange(); r3.setStart(d, 1); r3.setEnd(d, 1);\
             var n = t2.splitText(5); console.log(pt(r2), pt(r3), n.data, t2.data);\
             d.normalize(); console.log(pt(r2), pt(r3), d.firstChild.data, d.childNodes.length);\
             var r4 = document.createRange(); r4.setStart(d.firstChild, 2); r4.setEnd(i, 0); d.textContent = 'new'; console.log(pt(r4));\
             var r5 = document.createRange(); r5.setStart(d.firstChild, 1); r5.setEnd(d.firstChild, 2); d.firstChild.data = 'x'; console.log(pt(r5));"
        ),
    );
    assert_eq!(
        log,
        [
            "log:#text:2-DIV:3 adef",
            "log:#text:4-DIV:3",
            "log:#text:0-DIV:3",
            "log:#text:0-DIV:4",
            "log:#text:0-DIV:3",
            "log:DIV:1-DIV:2",
            "log:#text:3-#text:3 DIV:2-DIV:2 56789 01234",
            "log:#text:3-#text:8 #text:10-#text:10 0123456789q 2",
            "log:DIV:0-DIV:0",
            "log:#text:0-#text:0",
        ]
    );
}
