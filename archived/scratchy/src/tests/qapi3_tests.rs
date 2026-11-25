#[allow(unused_imports)]
use super::*;

// ---------- QAPI3: navigator.serviceWorker / window.caches stubs ----------

fn nav(code: &str) -> Vec<String> {
    run_collecting(&[src(code)])
}

#[test]
fn qapi3_service_worker_container() {
    let log = nav(
        "var s = navigator.serviceWorker; var n = 0; s.addEventListener('message', function () { n++; });\
         s.dispatchEvent(new Event('message'));\
         console.log(s.controller, s.oncontrollerchange, s.onmessage, n, Object.prototype.toString.call(s), s instanceof EventTarget,\
           s === navigator.serviceWorker, 'serviceWorker' in navigator, typeof ServiceWorkerContainer, s.ready === s.ready);\
         var settled = false; s.ready.then(function () { settled = true; });\
         s.register('/sw.js').catch(function (e) { console.log('reg', e.name, e instanceof DOMException); });\
         s.getRegistrations().then(function (r) { console.log('regs', Array.isArray(r), r.length); });\
         s.getRegistration().then(function (r) { console.log('reg1', r); });\
         Promise.resolve().then(function () {}).then(function () {}).then(function () {}).then(function () { console.log('ready', settled); });",
    );
    assert_eq!(log[0], "log:null null null 1 [object ServiceWorkerContainer] true true true function true", "{log:?}");
    for want in ["log:reg SecurityError true", "log:regs true 0", "log:reg1 undefined", "log:ready false"] {
        assert!(log.iter().any(|l| l == want), "{want}: {log:?}");
    }
}

#[test]
fn qapi3_caches() {
    let log = nav(
        "console.log('caches' in window, Object.prototype.toString.call(caches), typeof CacheStorage, typeof Cache);\
         var fmt = function (r) { return r.map(function (v) { return Array.isArray(v) ? '[' + v.length + ']' : String(v); }).join(); };\
         caches.open('v1').then(function (c) {\
           console.log(Object.prototype.toString.call(c), c instanceof Cache);\
           return Promise.all([c.match('/a'), c.matchAll(), c.add('/a'), c.addAll(['/a']), c.put('/a', {}), c.delete('/a'), c.keys()]);\
         }).then(function (r) { console.log('cache', fmt(r)); });\
         Promise.all([caches.match('/a'), caches.has('v1'), caches.keys(), caches.delete('v1')]).then(function (r) {\
           console.log('storage', fmt(r));\
         });",
    );
    assert_eq!(log[0], "log:true [object CacheStorage] function function", "{log:?}");
    for want in [
        "log:[object Cache] true",
        "log:cache undefined,[0],undefined,undefined,undefined,false,[0]",
        "log:storage undefined,false,[0],false",
    ] {
        assert!(log.iter().any(|l| l == want), "{want}: {log:?}");
    }
}

#[test]
fn qapi3_open_never_persists_and_arguments_are_checked() {
    let log = nav(
        "caches.open('v1').then(function () { return caches.has('v1'); }).then(function (h) { console.log('has', h); });\
         caches.open().catch(function (e) { console.log(e.name); });\
         navigator.serviceWorker.register().catch(function (e) { console.log(e.name); });",
    );
    assert!(log.iter().any(|l| l == "log:has false"), "{log:?}");
    assert_eq!(log.iter().filter(|l| *l == "log:TypeError").count(), 2, "{log:?}");
}
