#[allow(unused_imports)]
use super::*;

// ---------- QDOMATOM: post-connection steps run after the whole insertion, in tree order ----------

#[test]
fn qdomatom_fragment_scripts_see_all_inserted_nodes() {
    let (log, _body) = mutate(
        "<html><body><div id=host></div></body></html>",
        "var host = document.getElementById('host'), a = document.createElement('script'), b = document.createElement('script'), d = document.createElement('div');\
         window.d = d; window.b = b; window.host = host;\
         a.textContent = \"console.log('A', !!d.parentNode, !!b.parentNode, host.childNodes.length)\";\
         b.textContent = \"console.log('B', !!d.parentNode, host.childNodes.length)\";\
         var f = document.createDocumentFragment(); f.appendChild(a); f.appendChild(b); f.appendChild(d);\
         host.appendChild(f); console.log('done');",
    );
    assert_eq!(log, ["log:A true true 3", "log:B true 3", "log:done"], "{log:?}");
}

#[test]
fn qdomatom_iframe_waits_for_its_turn() {
    let (log, _body) = mutate(
        "<html><body></body></html>",
        "var s = document.createElement('script'), i = document.createElement('iframe'), s2 = document.createElement('script');\
         window.iframe = i;\
         s.textContent = \"console.log('s1', iframe.contentWindow ? 'win' : 'null')\";\
         s2.textContent = \"console.log('s2', iframe.contentWindow ? 'win' : 'null')\";\
         var df = document.createDocumentFragment(); df.appendChild(s); df.appendChild(i); df.appendChild(s2);\
         document.body.appendChild(df); console.log('after', i.contentWindow ? 'win' : 'null');\
         var div = document.createElement('div'), s3 = document.createElement('script'), i3 = document.createElement('iframe');\
         window.iframe = i3; s3.textContent = \"console.log('s3', iframe.contentWindow ? 'win' : 'null')\";\
         div.appendChild(s3); div.appendChild(i3); document.body.appendChild(div);",
    );
    assert_eq!(log, ["log:s1 null", "log:s2 win", "log:after win", "log:s3 null"], "{log:?}");
}

#[test]
fn qdomatom_nested_text_append_runs_only_that_script() {
    let (log, _body) = mutate(
        "<html><body></body></html>",
        "var s1 = document.createElement('script'), s2 = document.createElement('script'), s3 = document.createElement('script'), h = [];\
         window.h = h; window.s3 = s3;\
         s1.textContent = \"s3.appendChild(new Text(\\\"h.push('s3')\\\")); h.push('s1')\";\
         s2.textContent = \"h.push('s2')\";\
         var div = document.createElement('div'); div.appendChild(s1); div.appendChild(s2); div.appendChild(s3);\
         document.body.appendChild(div); console.log(h.join(','));",
    );
    assert_eq!(log, ["log:s3,s1,s2"], "{log:?}");
}
