// QDOM17: Document flavours and properties (tagName case per document type, charset aliases, nodeValue/textContent).
use super::*;

const XHTML: &str = "http://www.w3.org/1999/xhtml";

#[test]
fn qdom17_createelement_and_tagname_per_document_type() {
    let log = run_in_html(
        "<html><body></body></html>",
        &format!(
            "var h = document.implementation.createHTMLDocument('t'), x = document.implementation.createDocument(null, 'r'),\
             xh = document.implementation.createDocument('{XHTML}', 'html'), n = new Document();\
             function d(doc) {{ var e = doc.createElement('Div'); return [e.tagName, e.localName, e.nodeName, String(e.namespaceURI), doc.contentType].join('/'); }}\
             console.log(d(document)); console.log(d(h)); console.log(d(x)); console.log(d(xh)); console.log(d(n));\
             var a = document.createElement('p'); x.documentElement.appendChild(a); console.log(a.tagName, a.localName, a.nodeName);"
        ),
    );
    assert_eq!(
        log,
        [
            format!("log:DIV/div/DIV/{XHTML}/text/html"),
            format!("log:DIV/div/DIV/{XHTML}/text/html"),
            "log:Div/Div/Div/null/application/xml".to_string(),
            format!("log:Div/Div/Div/{XHTML}/application/xhtml+xml"),
            "log:Div/Div/Div/null/application/xml".to_string(),
            "log:p p p".to_string(),
        ],
        "{log:?}"
    );
}

#[test]
fn qdom17_charset_aliases_and_compat_mode() {
    let log = run_in_html(
        "<html><body></body></html>",
        "var h = document.implementation.createHTMLDocument(''), x = document.implementation.createDocument(null, null);\
         [document, h, x, new Document(), new DOMParser().parseFromString('<p>', 'text/html')].forEach(function (d) {\
           console.log(d.characterSet, d.charset, d.inputEncoding, d.compatMode); });\
         console.log(new DOMParser().parseFromString('<!doctype html><p>', 'text/html').compatMode);",
    );
    assert_eq!(
        log,
        [
            "log:UTF-8 UTF-8 UTF-8 CSS1Compat",
            "log:UTF-8 UTF-8 UTF-8 CSS1Compat",
            "log:UTF-8 UTF-8 UTF-8 CSS1Compat",
            "log:UTF-8 UTF-8 UTF-8 CSS1Compat",
            "log:UTF-8 UTF-8 UTF-8 BackCompat",
            "log:CSS1Compat",
        ],
        "{log:?}"
    );
}

#[test]
fn qdom17_nodevalue_and_textcontent() {
    let log = run_in_html(
        "<!doctype html><html><body>x</body></html>",
        "var t = document.createTextNode('a'); t.nodeValue = null; console.log(JSON.stringify(t.nodeValue), JSON.stringify(t.data));\
         var dt = document.doctype, h = document.implementation.createHTMLDocument('t'), f = document.createDocumentFragment();\
         console.log(document.nodeValue, document.textContent, dt.nodeValue, dt.textContent, h.textContent, f.nodeValue, JSON.stringify(f.textContent), document.body.nodeValue);\
         document.textContent = 'zz'; dt.textContent = 'zz'; dt.nodeValue = 'zz'; document.nodeValue = 'zz';\
         console.log(document.body.textContent, document.childNodes.length, dt.name);\
         f.textContent = 'k'; console.log(f.textContent, f.childNodes.length);",
    );
    assert_eq!(
        log,
        ["log:\"\" \"\"", "log:null null null null null null \"\" null", "log:x 2 html", "log:k 1"],
        "{log:?}"
    );
}
