#[allow(unused_imports)]
use super::*;

/// QDOM10: `window.event` (DOM "inner invoke").
fn qdom10_run(code: &str) -> Vec<String> {
    run_collecting(&[src(code)])
}

#[test]
fn qdom10_event_is_current_during_listener_and_restored() {
    let log = qdom10_run(
        "var d = document.createElement('div'); document.body.appendChild(d); window.onerror = function () { return true; };\
         console.log(window.event);\
         d.addEventListener('a', function (e) { console.log(window.event === e); d.dispatchEvent(new Event('b')); console.log(window.event === e); });\
         d.addEventListener('b', function (e) { console.log(window.event === e); throw new Error('x'); });\
         d.addEventListener('c', function (e) { console.log(window.event === e); });\
         d.dispatchEvent(new Event('a')); console.log(window.event);\
         d.dispatchEvent(new Event('c')); console.log(window.event);",
    );
    assert_eq!(log, ["log:undefined", "log:true", "log:true", "log:true", "log:undefined", "log:true", "log:undefined"], "{log:?}");
}

#[test]
fn qdom10_shadow_tree_listeners_leave_window_event_alone() {
    let log = qdom10_run(
        "var h = document.createElement('div'); document.body.appendChild(h);\
         var sr = h.attachShadow({ mode: 'open' }); var s = document.createElement('span'); sr.appendChild(s);\
         s.addEventListener('x', function (e) { console.log('in', window.event); });\
         h.addEventListener('x', function (e) { console.log('host', window.event === e); });\
         s.dispatchEvent(new Event('x', { bubbles: true, composed: true }));\
         console.log(window.event);",
    );
    let log: Vec<String> = log.into_iter().filter(|l| l.starts_with("log:")).collect();
    assert_eq!(log, ["log:in undefined", "log:host true", "log:undefined"], "{log:?}");
}
