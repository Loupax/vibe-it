// QIFRNAV / QDOM5d: unload events on iframe navigation (beforeunload, pagehide, visibilitychange, unload in spec order,
// nested windows after their parent), the ignore-opens-during-unload counter, and document.open() erasing the Window's and
// the Document's listeners (a load handler that calls document.open()/close() no longer recurses forever).
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
fn qdom5d_navigation_fires_unload_events_in_order() {
    let l = run("var f = document.createElement('iframe'); f.srcdoc = '<p>a</p>'; var seq = [];\
         f.onload = function () {\
           if (f.__n) return; f.__n = 1; var w = f.contentWindow, d = f.contentDocument;\
           ['beforeunload', 'pagehide', 'unload'].forEach(function (t) { w.addEventListener(t, function (e) { seq.push(t + ':' + e.constructor.name + ':' + (t === 'pagehide' ? e.persisted : '')); }); });\
           d.addEventListener('visibilitychange', function () { seq.push('vis:' + d.visibilityState); });\
           f.srcdoc = '<p>b</p>'; setTimeout(function () { console.log(seq.join(',')); }, 20);\
         };\
         document.body.appendChild(f);");
    assert!(l.contains(&"log:beforeunload:BeforeUnloadEvent:,pagehide:PageTransitionEvent:false,vis:hidden,unload:Event:".to_string()), "{l:?}");
}

#[test]
fn qdom5d_open_is_ignored_while_unloading_and_nested_windows_unload() {
    let l = run("var f = document.createElement('iframe'); f.srcdoc = '<p>a</p><iframe id=c srcdoc=\"<b>c</b>\"></iframe>';\
         f.onload = function () {\
           if (f.__n) return; f.__n = 1; var w = f.contentWindow, d = f.contentDocument, out = [];\
           var c = d.getElementById('c');\
           var go = function () {\
             var cw = c.contentWindow;\
             cw.addEventListener('unload', function () { var n = d.childNodes.length; d.open(); out.push('parent-open-ignored:' + (d.childNodes.length === n && n > 0)); });\
             w.addEventListener('pagehide', function () { out.push('parent-pagehide'); });\
             cw.addEventListener('pagehide', function () { out.push('child-pagehide'); });\
             f.srcdoc = '<p>b</p>'; setTimeout(function () { console.log(out.join(',')); }, 20);\
           };\
           if (c.contentWindow) go(); else setTimeout(go, 20);\
         };\
         document.body.appendChild(f);");
    assert!(l.contains(&"log:parent-pagehide,child-pagehide,parent-open-ignored:true".to_string()), "{l:?}");
}

#[test]
fn qdom5d_document_open_erases_listeners_so_load_handlers_do_not_recurse() {
    let l = run("var f = document.createElement('iframe'); f.srcdoc = '<p>a</p>'; var n = 0;\
         f.onload = function () {\
           if (f.__n) return; f.__n = 1; var w = f.contentWindow, d = f.contentDocument;\
           w.addEventListener('load', function () { n++; if (n < 50) { d.open(); d.close(); } });\
           d.open(); d.close(); setTimeout(function () { console.log('loads', n); }, 20);\
         };\
         document.body.appendChild(f);");
    assert!(l.contains(&"log:loads 0".to_string()), "{l:?}");
}
