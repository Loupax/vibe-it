// QDOM6b: createElementNS keeps the qualified name; the null namespace is not HTML; validate and extract.
use super::*;

#[test]
fn qdom6b_create_element_ns() {
    let log = run_in_html(
        "<html><body></body></html>",
        "function thr(f) { try { f(); return 'no'; } catch (e) { return e.name; } }\
         var a = document.createElementNS(null, 'Foo'), b = document.createElementNS('', 'p'), c = document.createElementNS('http://www.w3.org/1999/xhtml', 'Span');\
         console.log(a.namespaceURI, a.localName, a.tagName, a.prefix, b.namespaceURI, b.localName, b.tagName);\
         console.log(c.namespaceURI, c.localName, c.tagName, c.prefix);\
         var d = document.createElementNS('urn:x', 'p:Bar'); console.log(d.namespaceURI, d.prefix, d.localName, d.tagName);\
         console.log(thr(function () { document.createElementNS(null, 'p:x'); }), thr(function () { document.createElementNS('urn:x', 'xml:x'); }),\
           thr(function () { document.createElementNS('urn:x', 'xmlns:x'); }), thr(function () { document.createElementNS('http://www.w3.org/2000/xmlns/', 'a:b'); }),\
           thr(function () { document.createElementNS('urn:x', '1a'); }), thr(function () { document.createElementNS('http://www.w3.org/2000/xmlns/', 'xmlns'); }));\
         var at = document.createAttributeNS('http://www.w3.org/XML/1998/namespace', 'xml:lang'); console.log(at.prefix, at.localName, at.name);\
         console.log(thr(function () { document.createAttributeNS(null, 'a:b'); }), thr(function () { document.createAttributeNS('urn:x', 'xmlns:a'); }), thr(function () { document.createAttributeNS('http://www.w3.org/2000/xmlns/', 'a'); }));",
    );
    assert_eq!(
        log,
        [
            "log:null Foo Foo null null p p",
            "log:http://www.w3.org/1999/xhtml Span SPAN null",
            "log:urn:x p Bar p:Bar",
            "log:NamespaceError NamespaceError NamespaceError NamespaceError InvalidCharacterError no",
            "log:xml lang xml:lang",
            "log:NamespaceError NamespaceError NamespaceError",
        ]
    );
}
