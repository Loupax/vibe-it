// QDOM5e: navigating an iframe's initial about:blank window (src/srcdoc set after insertion) keeps the Window and makes
// a NEW Document in it (the old one stays about:blank and non-active); removing an iframe fires its pagehide/unload
// events in a task, not synchronously.
use super::*;

#[allow(clippy::let_and_return)]
fn run(js: &str, fake: &Arc<QfFake>) -> Vec<String> {
    let (engine, log) = qf7_engine("https://page.test/", fake);
    engine.eval("t", js);
    for _ in 0..20 {
        qf7_drive(&engine, fake);
        std::thread::sleep(std::time::Duration::from_millis(5));
        engine.run_due();
    }
    let out = log.borrow().clone();
    out
}

#[test]
fn qdom5e_srcdoc_after_insertion_keeps_the_window_and_makes_a_new_document() {
    let fake = QfFake::new(vec![]);
    let l = run(
        "var f = document.createElement('iframe'); document.body.appendChild(f);\
         var w = f.contentWindow, d0 = f.contentDocument; w.keep = 7;\
         f.onload = function () {\
           var d1 = f.contentDocument; if (d1 === d0 && !d0.getElementById('x')) return;\
           console.log('r', f.contentWindow === w, d1 !== d0, w.document === d1, w.keep, d0.URL, d1.URL,\
             d0.getElementById('x') === null, d1.getElementById('x') !== null, d0.open() === d0, d0.readyState, d1.readyState);\
         };\
         f.srcdoc = '<p id=x>hi</p>';",
        &fake,
    );
    assert!(l.contains(&"log:r true true true 7 about:blank about:srcdoc true true true complete complete".to_string()), "{l:?}");
}

#[test]
fn qdom5e_same_origin_src_after_insertion_reuses_the_initial_window() {
    let fake = QfFake::new(vec![("/p.html", qf7_ok("<!doctype html><p id=x>hi</p>", &[("content-type", "text/html")]))]);
    let l = run(
        "var f = document.createElement('iframe'); document.body.appendChild(f);\
         var w = f.contentWindow, d0 = f.contentDocument; w.keep = 1;\
         f.onload = function () {\
           var d1 = f.contentDocument;\
           console.log('r', f.contentWindow === w, d1 !== d0, w.keep, d0.URL, d1.URL, d1.getElementById('x') !== null, w.location.href);\
         };\
         f.src = '/p.html';",
        &fake,
    );
    assert!(l.contains(&"log:r true true 1 about:blank https://page.test/p.html true https://page.test/p.html".to_string()), "{l:?}");
}

#[test]
fn qdom5e_document_open_ends_the_initial_about_blankness() {
    let fake = QfFake::new(vec![("/p.html", qf7_ok("<!doctype html><p>hi</p>", &[("content-type", "text/html")]))]);
    let l = run(
        "var f = document.createElement('iframe'); document.body.appendChild(f);\
         var w = f.contentWindow; w.keep = 1; f.contentDocument.open();\
         f.onload = function () { console.log('r', f.contentWindow === w, f.contentWindow.keep); };\
         f.src = '/p.html';",
        &fake,
    );
    assert!(l.contains(&"log:r false undefined".to_string()), "{l:?}");
}

#[test]
fn qdom5e_custom_elements_follow_the_new_document() {
    let fake = QfFake::new(vec![]);
    let l = run(
        "var f = document.createElement('iframe'); document.body.appendChild(f);\
         var w = f.contentWindow; class A extends w.HTMLElement {} w.customElements.define('x-a', A);\
         f.onload = function () {\
           var d = f.contentDocument, e = d.createElement('x-a'), p = d.getElementById('y');\
           console.log('r', e instanceof A, p instanceof A, w.customElements.get('x-a') === A);\
         };\
         f.srcdoc = '<x-a id=y></x-a>';",
        &fake,
    );
    assert!(l.contains(&"log:r true true true".to_string()), "{l:?}");
}

#[test]
fn qdom5e_removal_fires_unload_in_a_task() {
    let fake = QfFake::new(vec![]);
    let l = run(
        "var f = document.createElement('iframe'); f.srcdoc = '<p>a</p>'; var seq = [];\
         f.onload = function () {\
           if (f.__n) return; f.__n = 1; var w = f.contentWindow;\
           w.addEventListener('pagehide', function () { seq.push('pagehide'); });\
           w.addEventListener('unload', function () { seq.push('unload'); });\
           f.remove();\
           console.log('sync', seq.length, f.contentWindow === null, window.length);\
           setTimeout(function () { console.log('later', seq.join(',')); }, 30);\
         };\
         document.body.appendChild(f);",
        &fake,
    );
    assert!(l.contains(&"log:sync 0 true 0".to_string()), "{l:?}");
    assert!(l.contains(&"log:later pagehide,unload".to_string()), "{l:?}");
}
