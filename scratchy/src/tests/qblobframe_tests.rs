// QBLOBFRAME: `iframe.src = URL.createObjectURL(new Blob([html], {type: 'text/html'}))` loads a same-origin document
// from the blob's bytes (async, through the QDOM5c iframe load path); revoked or non-HTML blobs fail cleanly (`load` fires).
use super::*;

#[allow(clippy::let_and_return)]
fn run(js: &str) -> Vec<String> {
    let fake = QfFake::new(vec![]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval("t", js);
    for _ in 0..20 {
        qf7_drive(&engine, &fake);
        std::thread::sleep(std::time::Duration::from_millis(5));
        engine.run_due();
    }
    let out = log.borrow().clone();
    out
}

#[test]
fn qblobframe_html_blob_loads_same_origin() {
    let l = run("var u = URL.createObjectURL(new Blob(['<!doctype html><p id=x>hi</p><script>window.ran = 1<\\/script>'], {type: 'text/html;charset=utf-8'}));\
         var f = document.createElement('iframe'); f.src = u; f.onload = function () {\
           var w = f.contentWindow; console.log('loaded', w.location.href === u, w.document.getElementById('x') !== null, w.ran, w.location.origin === location.origin, f.contentDocument !== null); };\
         document.body.appendChild(f); console.log('sync', f.contentDocument.getElementById('x'));");
    assert!(l.contains(&"log:sync null".to_string()), "{l:?}");
    assert!(l.contains(&"log:loaded true true 1 true true".to_string()), "{l:?}");
}

#[test]
fn qblobframe_revoked_and_non_html_fail_cleanly() {
    let l = run("var mk = function (blob, revoke, tag) { var u = URL.createObjectURL(blob); var f = document.createElement('iframe'); f.src = u;\
           f.onload = function () { console.log(tag, f.contentDocument && f.contentDocument.body && f.contentDocument.body.textContent === '', f.contentWindow.location.href); };\
           if (revoke) URL.revokeObjectURL(u); document.body.appendChild(f); };\
         mk(new Blob(['<p>x</p>'], {type: 'text/html'}), true, 'revoked');\
         mk(new Blob(['<p>x</p>'], {type: 'text/plain'}), false, 'plain');");
    assert!(l.contains(&"log:revoked true about:blank".to_string()), "{l:?}");
    assert!(l.contains(&"log:plain true about:blank".to_string()), "{l:?}");
}
