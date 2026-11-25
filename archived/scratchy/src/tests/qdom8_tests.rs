// QDOM8: ProcessingInstruction, CDATASection, XMLDocument, Node constants.
use super::*;

#[test]
fn qdom8_pi_cdata_xmldocument_constants() {
    let log = run_in_html(
        "<html><body><div id=d></div></body></html>",
        "var x = document.implementation.createDocument(null, 'r'), pi = x.createProcessingInstruction('tg', 'da ta'), cd = x.createCDATASection('a<b');\
         console.log(pi.nodeType, pi.nodeName, pi.target, pi.data, pi.nodeValue, pi.textContent, pi.length, pi.ownerDocument === x);\
         console.log(pi instanceof ProcessingInstruction, pi instanceof CharacterData, Object.getPrototypeOf(pi) === ProcessingInstruction.prototype);\
         console.log(cd.nodeType, cd.nodeName, cd.data, cd instanceof CDATASection, cd instanceof Text, Object.getPrototypeOf(cd) === CDATASection.prototype, cd.ownerDocument === x);\
         pi.data = 'z'; pi.appendData('!'); cd.replaceData(0, 1, 'Q'); console.log(pi.data, cd.data, pi.cloneNode().target, pi.cloneNode().data, cd.cloneNode().nodeType);\
         x.documentElement.appendChild(cd); x.documentElement.appendChild(pi); x.documentElement.appendChild(x.createTextNode('t'));\
         console.log(x.documentElement.textContent, x.documentElement.childNodes.length, x.documentElement.cloneNode(true).childNodes[0].nodeType);\
         console.log(pi.isEqualNode(x.createProcessingInstruction('tg', 'z!')), pi.isEqualNode(x.createProcessingInstruction('th', 'z!')), cd.isEqualNode(x.createTextNode('Q<b')), cd.isEqualNode(x.createCDATASection('Q<b')));\
         var d = document.getElementById('d'); d.appendChild(document.createProcessingInstruction('a', 'b')); console.log(d.innerHTML);\
         function thr(f) { try { f(); return 'no'; } catch (e) { return e.name; } }\
         console.log(thr(function () { document.createProcessingInstruction('1a', 'x'); }), thr(function () { document.createProcessingInstruction('a', 'x?>'); }),\
           thr(function () { document.createProcessingInstruction('a'); }), thr(function () { document.createCDATASection('x'); }),\
           thr(function () { x.createCDATASection('a]]>b'); }), thr(function () { x.createCDATASection('a]]b'); }));\
         var n = new Document(), h = document.implementation.createHTMLDocument('t');\
         console.log(Object.getPrototypeOf(n) === Document.prototype, n instanceof XMLDocument, x instanceof XMLDocument, x instanceof Document, Object.getPrototypeOf(x) === XMLDocument.prototype, h instanceof XMLDocument, h instanceof Document, document instanceof XMLDocument);\
         console.log(thr(function () { new XMLDocument(); }), n.createCDATASection('ok').nodeType, thr(function () { h.createCDATASection('x'); }), h.createElement('P').tagName, x.createElement('P').tagName);\
         console.log([Node.CDATA_SECTION_NODE, Node.ENTITY_REFERENCE_NODE, Node.ENTITY_NODE, Node.PROCESSING_INSTRUCTION_NODE, Node.DOCUMENT_TYPE_NODE, Node.NOTATION_NODE, Node.DOCUMENT_POSITION_CONTAINED_BY].join(), document.NOTATION_NODE, d.CDATA_SECTION_NODE, JSON.stringify(Object.getOwnPropertyDescriptor(Node, 'ENTITY_NODE')));\
         var p2 = new DOMParser().parseFromString('<r><?pi some data?>x</r>', 'application/xml'); var q = p2.documentElement.firstChild;\
         console.log(q.nodeType, q.target, q.data, p2 instanceof XMLDocument);",
    );
    assert_eq!(
        log,
        [
            "log:7 tg tg da ta da ta da ta 5 true",
            "log:true true true",
            "log:4 #cdata-section a<b true true true true",
            "log:z! Q<b tg z! 4",
            "log:Q<bt 3 4",
            "log:true false false true",
            "log:<?a b>",
            "log:InvalidCharacterError InvalidCharacterError TypeError NotSupportedError InvalidCharacterError no",
            "log:true false true true true false true false",
            "log:TypeError 4 NotSupportedError P P",
            "log:4,5,6,7,10,12,16 12 4 {\"value\":6,\"writable\":false,\"enumerable\":true,\"configurable\":false}",
            "log:7 pi some data true",
        ],
        "{log:?}"
    );
}
