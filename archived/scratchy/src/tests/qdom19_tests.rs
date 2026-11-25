// QDOM19: whatwg/dom name productions (valid element/attribute/doctype name, validate and extract).
use super::*;

const THR: &str = "function thr(f) { try { f(); return 'no'; } catch (e) { return e.name; } }";

fn rep(s: &str, n: usize) -> String {
    format!("log:{}", vec![s; n].join(","))
}

#[test]
fn qdom19_element_names() {
    let log = run_in_html(
        "<html><body></body></html>",
        &format!(
            "{THR}\
             var ok = ['div', 'a<b', 'f}}oo', ':a', '_a', '\\u00e9', 'a:b:c', 'a\\u0300', ':-', '\\ud83c\\udd96x', 'a\\u007f'];\
             var bad = ['', '1a', '-a', '.a', ' a', 'a b', 'a>', 'a/b', 'a\\0', 'a\\n', ':\\t', '_ ', '\\u00e9 '];\
             console.log(ok.map(function (n) {{ return thr(function () {{ document.createElement(n); }}); }}).join());\
             console.log(bad.map(function (n) {{ return thr(function () {{ document.createElement(n); }}); }}).join());"
        ),
    );
    assert_eq!(log, [rep("no", 11), rep("InvalidCharacterError", 13)]);
}

#[test]
fn qdom19_attribute_names() {
    let log = run_in_html(
        "<html><body></body></html>",
        &format!(
            "{THR}\
             var e = document.createElement('div');\
             var ok = ['a', '1', '-', '*', ':', 'a:b', '\\u00e9', '\\\"', 'a<b'];\
             var bad = ['', 'a b', 'a=', '=', 'a/', 'a>', 'a\\0', 'a\\n', '\\t'];\
             function run(f) {{ return function (n) {{ return thr(function () {{ f(n); }}); }}; }}\
             console.log(ok.map(run(function (n) {{ e.setAttribute(n, 'v'); }})).join());\
             console.log(ok.map(run(function (n) {{ e.toggleAttribute(n); e.toggleAttribute(n, false); }})).join());\
             console.log(ok.map(run(function (n) {{ document.createAttribute(n); }})).join());\
             console.log(bad.map(run(function (n) {{ e.setAttribute(n, 'v'); }})).join());\
             console.log(bad.map(run(function (n) {{ e.toggleAttribute(n, false); }})).join());\
             console.log(bad.map(run(function (n) {{ document.createAttribute(n); }})).join());\
             console.log(bad.map(run(function (n) {{ e.setAttributeNS('urn:x', 'p:' + n, 'v'); }})).join());"
        ),
    );
    let (ok, bad) = (rep("no", 9), rep("InvalidCharacterError", 9));
    assert_eq!(log, [ok.clone(), ok.clone(), ok, bad.clone(), bad.clone(), bad.clone(), bad]);
}

#[test]
fn qdom19_ns_and_doctype() {
    let log = run_in_html(
        "<html><body></body></html>",
        &format!(
            "{THR}\
             var X = 'http://www.w3.org/XML/1998/namespace', N = 'http://www.w3.org/2000/xmlns/';\
             console.log(thr(function () {{ document.createElementNS('urn:x', 'a:b:c'); }}), thr(function () {{ document.createElementNS('urn:x', '=:b'); }}),\
               thr(function () {{ document.createElementNS('urn:x', ':b'); }}), thr(function () {{ document.createElementNS('urn:x', 'a:'); }}),\
               thr(function () {{ document.createElementNS('urn:x', '\\ud83c\\udd96:b'); }}), thr(function () {{ document.createElementNS('urn:x', 'a b:c'); }}));\
             console.log(thr(function () {{ document.createElementNS(null, 'a:b'); }}), thr(function () {{ document.createElementNS('urn:x', 'xml:b'); }}),\
               thr(function () {{ document.createElementNS(X, 'xml:b'); }}), thr(function () {{ document.createElementNS('urn:x', 'xmlns'); }}),\
               thr(function () {{ document.createElementNS(N, 'xmlns'); }}), thr(function () {{ document.createElementNS(N, 'a:b'); }}));\
             console.log(thr(function () {{ document.createAttributeNS('urn:x', 'a:1'); }}), thr(function () {{ document.createAttributeNS('urn:x', 'a:b=c'); }}),\
               thr(function () {{ document.createAttributeNS('urn:x', '=:b'); }}), thr(function () {{ document.createAttributeNS('urn:x', 'a:b:c'); }}),\
               thr(function () {{ document.createAttributeNS(N, 'xmlns:a'); }}), thr(function () {{ document.createAttributeNS(N, 'a'); }}));\
             var im = document.implementation, dts = ['', 'a:b', '1', '\\u00e9', '-', ':'];\
             console.log(dts.map(function (n) {{ return thr(function () {{ im.createDocumentType(n, '', ''); }}); }}).join());\
             console.log(['a b', 'a>', 'a\\0', 'a\\n', ' '].map(function (n) {{ return thr(function () {{ im.createDocumentType(n, '', ''); }}); }}).join());\
             console.log(thr(function () {{ im.createDocument('urn:x', 'a:b:c'); }}), thr(function () {{ im.createDocument(null, '1a'); }}), thr(function () {{ im.createDocument(null, 'a:b'); }}));\
             console.log(im.createDocument('urn:x', 'a:b:c').documentElement.localName);"
        ),
    );
    assert_eq!(
        log,
        [
            "log:no no InvalidCharacterError InvalidCharacterError no InvalidCharacterError".to_string(),
            "log:NamespaceError NamespaceError no NamespaceError no NamespaceError".to_string(),
            "log:no InvalidCharacterError no no no NamespaceError".to_string(),
            rep("no", 6),
            rep("InvalidCharacterError", 5),
            "log:no InvalidCharacterError NamespaceError".to_string(),
            "log:b:c".to_string(),
        ]
    );
}
