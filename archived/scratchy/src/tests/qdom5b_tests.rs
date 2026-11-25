// QDOM5b: live Range boundary-point fix-ups on the paths that bypass insertBefore/removeChild in JS
// (outerHTML, insertAdjacentHTML, innerHTML, document.write, cross-document adopt, other realms' wrappers),
// Attr nodes as Range boundary containers, and DOMException code 24.
use super::*;

const HDR: &str = "function pt(r) { return r.startContainer.nodeName + ':' + r.startOffset + '-' + r.endContainer.nodeName + ':' + r.endOffset; }";

#[test]
fn qdom5b_outer_inner_adjacent_html() {
    let log = run_in_html(
        "<html><body><div id=d><p>a</p><b id=b>cd</b><i id=i>ef</i></div></body></html>",
        &format!(
            "{HDR}\
             var d = document.getElementById('d'), b = document.getElementById('b'), i = document.getElementById('i');\
             var r1 = document.createRange(); r1.setStart(b.firstChild, 1); r1.setEnd(i.firstChild, 1);\
             var r2 = document.createRange(); r2.setStart(d, 2); r2.setEnd(d, 3);\
             b.outerHTML = '<u>1</u><u>2</u>';\
             console.log(pt(r1), pt(r2));\
             var u = d.childNodes[2], r3 = document.createRange(); r3.setStart(d, 2); r3.setEnd(d, 4);\
             u.insertAdjacentHTML('beforebegin', '<s>1</s><s>2</s>'); console.log(pt(r3));\
             var r4 = document.createRange(); r4.setStart(u, 0); r4.setEnd(u, 1);\
             u.insertAdjacentHTML('afterbegin', '<s>1</s><s>2</s>'); console.log(pt(r4));\
             var r5 = document.createRange(); r5.setStart(u, 3); r5.setEnd(u, 3);\
             u.insertAdjacentHTML('beforeend', '<s>3</s>'); console.log(pt(r5));\
             var r6 = document.createRange(); r6.setStart(d.firstChild.firstChild, 1); r6.setEnd(d, 3);\
             d.innerHTML = '<q>z</q>'; console.log(pt(r6));"
        ),
    );
    assert_eq!(
        log,
        ["log:DIV:1-#text:1 DIV:1-DIV:4", "log:DIV:2-DIV:6", "log:U:0-U:3", "log:U:3-U:3", "log:DIV:0-DIV:0"]
    );
}

#[test]
fn qdom5b_document_write_inserts_before_later_offsets() {
    let out = qa13_run(
        "<html><body><div id=x></div><script>var r=document.createRange(); r.setStart(document.body,1); r.setEnd(document.body,2);\
         document.write('<p>a</p><p>b</p>');\
         console.log(r.startContainer.nodeName, r.startOffset, r.endContainer.nodeName, r.endOffset);</script><i></i></body></html>",
    );
    // body children before the write: text/div/script (offsets 0..); the script sits at index 2 (a text before it), the
    // writes go after it, so offsets at or before the script are untouched.
    assert_eq!(out, ["log:BODY 1 BODY 2"]);
}

#[test]
fn qdom5b_cross_document_adopt() {
    let log = run_in_html(
        "<html><body><div id=d><p id=p>abc</p><i>x</i></div></body></html>",
        &format!(
            "{HDR}\
             var d = document.getElementById('d'), p = document.getElementById('p');\
             var other = document.implementation.createHTMLDocument('t');\
             var r = document.createRange(); r.setStart(p.firstChild, 2); r.setEnd(d, 2);\
             other.adoptNode(p);\
             console.log(pt(r), p.parentNode === null, p.ownerDocument === other);\
             var q = document.createElement('q'); q.textContent = 'zz'; d.insertBefore(q, d.firstChild);\
             var r2 = document.createRange(); r2.setStart(q.firstChild, 1); r2.setEnd(d, 2);\
             other.body.appendChild(q);\
             console.log(pt(r2), q.ownerDocument === other);"
        ),
    );
    assert_eq!(log, ["log:DIV:0-DIV:1 true true", "log:DIV:0-DIV:1 true"]);
}

#[test]
fn qdom5b_other_realm_wrappers_fix_up_ranges() {
    // A page-realm node moved into an iframe's document keeps its page-realm wrapper; removing its child through that
    // wrapper must still fix the iframe realm's live ranges.
    let log = run_in_html(
        "<html><body></body></html>",
        &format!(
            "{HDR}\
             var f = document.createElement('iframe'); document.body.appendChild(f);\
             var fd = f.contentDocument, p = document.createElement('p'); p.appendChild(document.createTextNode('abcd'));\
             fd.body.appendChild(p);\
             var r = fd.createRange(); r.setStart(p.firstChild, 3); r.setEnd(p, 1);\
             p.firstChild.deleteData(0, 2); console.log(pt(r));\
             p.insertBefore(document.createElement('b'), p.firstChild); console.log(pt(r));\
             p.removeChild(p.firstChild); console.log(pt(r));\
             p.removeChild(p.firstChild); console.log(pt(r));"
        ),
    );
    assert_eq!(log, ["log:#text:1-P:1", "log:#text:1-P:2", "log:#text:1-P:1", "log:P:0-P:0"]);
}

#[test]
fn qdom5b_attr_nodes_in_ranges_and_dom_exception_24() {
    let log = run_in_html(
        "<html><body><div id=d x=abc y=def></div></body></html>",
        &format!(
            "{HDR}\
             var d = document.getElementById('d'), a = d.getAttributeNode('x'), b = d.getAttributeNode('y');\
             console.log(a.parentNode, a.parentElement, a.isConnected, a.getRootNode() === a, a.contains(a), document.body.contains(a));\
             console.log(a.compareDocumentPosition(b), b.compareDocumentPosition(a), a.compareDocumentPosition(a), d.compareDocumentPosition(a), a.compareDocumentPosition(d));\
             var r = document.createRange(); r.selectNodeContents(a);\
             console.log(r.intersectsNode(a), String(r) === '');\
             var e = null; try {{ r.setStartBefore(a); }} catch (x) {{ e = x; }}\
             console.log(e.name, e.code);\
             try {{ r.surroundContents(document); }} catch (x) {{ e = x; }}\
             console.log(e.name, e.code, DOMException.INVALID_NODE_TYPE_ERR);"
        ),
    );
    assert_eq!(
        log,
        [
            "log:null null false true true false",
            "log:36 34 0 20 10",
            "log:true true",
            "log:InvalidNodeTypeError 24",
            "log:InvalidNodeTypeError 24 24",
        ]
    );
}

#[test]
fn qdom5b_delete_contents_over_a_doctype_does_not_throw() {
    let log = run_in_html(
        "<html><body></body></html>",
        &format!(
            "{HDR}\
             var dt = document.implementation.createDocumentType('html', '', ''); var d2 = document.implementation.createDocument(null, 'x', dt);\
             var r = d2.createRange(); r.setStart(d2, 0); r.setEnd(d2, 2); r.deleteContents(); console.log(pt(r), d2.childNodes.length);"
        ),
    );
    assert_eq!(log, ["log:#document:0-#document:0 0"]);
}
