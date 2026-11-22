#[allow(unused_imports)]
use super::*;

/// QDOM14: lookupNamespaceURI / lookupPrefix / isDefaultNamespace on every node kind, Element.prefix.
#[test]
fn qdom14_namespace_lookup() {
    let (log, _) = mutate(
        "<html><body><div id=a></div></body></html>",
        "var NS = 'urn:x', X = 'http://www.w3.org/2000/xmlns/';\
         var r = document.createElementNS(NS, 'p:root'); r.setAttributeNS(X, 'xmlns:q', 'urn:q'); r.setAttributeNS(X, 'xmlns', 'urn:d');\
         var c = document.createElementNS('urn:c', 'c'); r.appendChild(c); var t = document.createTextNode('t'); c.appendChild(t);\
         var at = r.getAttributeNode('xmlns:q'); var f = document.createDocumentFragment();\
         console.log(r.prefix, r.localName, r.tagName, c.prefix, c.localName);\
         console.log(r.lookupNamespaceURI('p'), r.lookupNamespaceURI('q'), r.lookupNamespaceURI(null), r.lookupNamespaceURI(''), r.lookupNamespaceURI('z'));\
         console.log(c.lookupNamespaceURI('p'), c.lookupNamespaceURI(null), t.lookupNamespaceURI('q'), t.lookupNamespaceURI(null), at.lookupNamespaceURI('p'));\
         console.log(r.lookupPrefix(NS), r.lookupPrefix('urn:q'), c.lookupPrefix('urn:q'), t.lookupPrefix(NS), r.lookupPrefix(null), r.lookupPrefix(''), r.lookupPrefix('nope'));\
         console.log(r.isDefaultNamespace('urn:d'), c.isDefaultNamespace('urn:c'), r.isDefaultNamespace(NS), r.isDefaultNamespace(null), r.isDefaultNamespace(''), document.createElement('i').isDefaultNamespace(null));\
         var H = 'http://www.w3.org/1999/xhtml';\
         console.log(f.lookupNamespaceURI(null), document.lookupNamespaceURI(null), document.lookupPrefix(H), document.isDefaultNamespace(H));\
         var d = document.implementation.createDocument(NS, 'p:doc'); console.log(d.lookupNamespaceURI('p'), d.lookupPrefix(NS), d.isDefaultNamespace(NS), d.createComment('x').lookupNamespaceURI('p'));\
         try { r.lookupNamespaceURI(); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        [
            "log:p root p:root null c",
            "log:urn:x urn:q urn:d urn:d null",
            "log:urn:x urn:c urn:q urn:c urn:x",
            "log:p q q p null null null",
            "log:true true false false false false",
            "log:null http://www.w3.org/1999/xhtml null true",
            "log:urn:x p false null",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

/// QDOM14: getElementsByTagName by qualified name (HTML elements in HTML documents compare lowercased) and
/// getElementsByTagNameNS.
#[test]
fn qdom14_get_elements_by_tag_name() {
    let (log, _) = mutate(
        "<html><body><div id=a><B>x</B></div></body></html>",
        "var a = document.getElementById('a'); var te = document.createElementNS('urn:x', 'te:st'); var up = document.createElementNS('urn:x', 'TE:ST');\
         a.appendChild(te); a.appendChild(up); var h = document.createElementNS('http://www.w3.org/1999/xhtml', 'x:Div'); a.appendChild(h);\
         console.log(a.getElementsByTagName('te:st').length, a.getElementsByTagName('TE:ST').length, a.getElementsByTagName('b').length, a.getElementsByTagName('B').length, a.getElementsByTagName('*').length);\
         console.log(document.getElementsByTagName('te:st')[0] === te, a.getElementsByTagName('x:div').length, a.getElementsByTagName('x:Div').length, document.getElementsByTagName('').length);\
         console.log(a.getElementsByTagNameNS('urn:x', 'st').length, a.getElementsByTagNameNS('urn:x', 'ST').length, a.getElementsByTagNameNS('*', 'st').length, a.getElementsByTagNameNS('urn:x', '*').length, document.getElementsByTagNameNS('http://www.w3.org/1999/xhtml', 'b').length);\
         var l = a.getElementsByTagName('te:st'); a.removeChild(te); console.log(l.length);",
    );
    assert_eq!(
        log,
        ["log:1 1 1 1 4", "log:true 1 1 0", "log:1 1 1 2 1", "log:0"],
        "{log:?}"
    );
}
