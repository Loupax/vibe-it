#[allow(unused_imports)]
use super::*;

/// QDOM13: event handler IDL attributes (HTML "event handler IDL attributes").
fn qdom13_run(code: &str) -> Vec<String> {
    run_collecting(&[src(code)])
}

#[test]
fn qdom13_accessors_on_the_right_prototypes() {
    let log = qdom13_run(
        "var d = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'onclick');\
         console.log(typeof d.get, typeof d.set, d.enumerable, d.configurable, 'onclick' in Document.prototype, 'onclick' in Window.prototype);\
         console.log(Object.getOwnPropertyDescriptor(Document.prototype, 'onreadystatechange') !== undefined, 'onreadystatechange' in HTMLElement.prototype);\
         console.log('onbeforeunload' in Window.prototype, 'onbeforeunload' in HTMLElement.prototype, 'onbeforeunload' in HTMLBodyElement.prototype, 'onbeforeunload' in HTMLFrameSetElement.prototype);\
         console.log('ontouchstart' in window, 'ontouchstart' in document, 'ontouchstart' in document.body, 'onpointerdown' in window);\
         console.log(document.createElement('frameset') instanceof HTMLFrameSetElement, 'onwebkitanimationend' in document.body, 'onwebkittransitionend' in window);\
         var p = document.createElement('div'); console.log(p.onclick, document.onclick, window.onclick, window.onmessage);",
    );
    assert_eq!(
        log,
        [
            "log:function function true true true true",
            "log:true false",
            "log:true false true true",
            "log:false false false true",
            "log:true true true",
            "log:null null null null"
        ],
        "{log:?}"
    );
}

#[test]
fn qdom13_get_set_rules_and_dispatch_order() {
    let log = qdom13_run(
        "var d = document.createElement('div'); document.body.appendChild(d); var f = function (e) { console.log('h', this === d, e.type); };\
         d.addEventListener('click', function () { console.log('l1'); });\
         d.onclick = f; console.log(d.onclick === f);\
         d.addEventListener('click', function () { console.log('l2'); });\
         d.onclick = f; d.click();\
         d.onclick = 'str'; console.log(d.onclick); d.click();\
         d.onclick = {}; console.log(d.onclick); d.onclick = function () { console.log('h2'); }; d.click();\
         d.onclick = function () { return false; }; var e = new Event('click', { cancelable: true }); d.dispatchEvent(e); console.log(e.defaultPrevented);\
         d.onclick = null; console.log(d.onclick);",
    );
    assert_eq!(
        log,
        [
            "log:true", "log:l1", "log:h true click", "log:l2", "log:null", "log:l1", "log:l2", "log:null", "log:l1", "log:l2",
            "log:h2", "log:l1", "log:l2", "log:true", "log:null"
        ],
        "{log:?}"
    );
}

#[test]
fn qdom13_content_attribute_and_idl_override() {
    let (log, _) = mutate(
        "<html><body><div id=a onclick=\"console.log('attr', this.id, event.type)\"></div></body></html>",
        "var a = document.getElementById('a'); console.log(typeof a.onclick);\
         a.click(); a.onclick = function () { console.log('idl'); }; a.click();\
         a.setAttribute('onclick', \"console.log('attr2')\"); a.click(); console.log(typeof a.onclick);\
         a.removeAttribute('onclick'); console.log(a.onclick); a.click();",
    );
    assert_eq!(
        log,
        ["log:function", "log:attr a click", "log:idl", "log:attr2", "log:function", "log:null"],
        "{log:?}"
    );
}

#[test]
fn qdom13_body_forwards_window_handlers() {
    let log = qdom13_run(
        "var b = document.body, nop = function () {}; console.log(b.onload, window.onload);\
         b.onload = nop; console.log(window.onload === nop, b.onload === nop);\
         window.onresize = nop; console.log(b.onresize === nop); b.onresize = null; console.log(window.onresize);\
         var x = document.createElement('body'); window.onscroll = null; x.setAttribute('onscroll', 'return 1');\
         console.log(typeof window.onscroll, window.onscroll === x.onscroll);\
         x.removeAttribute('onscroll'); console.log(window.onscroll);\
         var fs = document.createElement('frameset'); fs.onblur = nop; console.log(window.onblur === nop, fs.onblur === nop); window.onblur = null;\
         var seen = []; window.onunload = function (e) { seen.push(this === window, e.type); }; window.dispatchEvent(new Event('unload')); console.log(seen.join());\
         document.body.onclick = nop; console.log(window.onclick, document.body.onclick === nop);",
    );
    assert_eq!(
        log,
        [
            "log:null null", "log:true true", "log:true", "log:null", "log:function true", "log:null", "log:true true",
            "log:true,unload", "log:null true"
        ],
        "{log:?}"
    );
}

#[test]
fn qdom13_body_attribute_runs_on_window_event() {
    let (log, _) = mutate(
        "<html><body onload=\"console.log('body-load', this === window)\" onhashchange=\"console.log('hc')\"></body></html>",
        "console.log(typeof window.onload, typeof document.body.onhashchange);\
         window.dispatchEvent(new Event('load')); window.dispatchEvent(new Event('hashchange'));\
         document.body.onhashchange = function () { console.log('idl-hc'); }; window.dispatchEvent(new Event('hashchange'));",
    );
    assert_eq!(
        log,
        ["log:function function", "log:body-load true", "log:hc", "log:idl-hc", "log:body-load true"],
        "{log:?}"
    );
}

#[test]
fn qdom13_window_onerror_five_arguments_and_cancel() {
    let log = qdom13_run(
        "window.onerror = function (m, f, l, c, e) { console.log(typeof m, typeof f, typeof l, typeof c, e instanceof Error); return true; };\
         var ev = new ErrorEvent('error', { cancelable: true, message: 'boom', error: new Error('x') }); window.dispatchEvent(ev); console.log(ev.defaultPrevented);",
    );
    assert_eq!(log, ["log:string string number number true", "log:true"], "{log:?}");
}

#[test]
fn qdom13_webkit_prefixed_handlers_are_not_aliases() {
    let log = qdom13_run(
        "var d = document.createElement('div'); document.body.appendChild(d);\
         d.onanimationend = function () {}; console.log(d.onwebkitanimationend, d.onanimationend !== null);\
         d.onwebkitanimationend = function () { console.log('prefixed handler'); };\
         console.log(d.onwebkitanimationend !== d.onanimationend);\
         d.onanimationend = null;\
         d.dispatchEvent(new AnimationEvent('webkitAnimationEnd')); d.dispatchEvent(new AnimationEvent('animationend'));\
         d.addEventListener('webkitanimationend', function () { console.log('lowercase listener'); });\
         d.dispatchEvent(new AnimationEvent('webkitAnimationEnd'));",
    );
    assert_eq!(
        log,
        ["log:null true", "log:true", "log:prefixed handler", "log:prefixed handler"],
        "{log:?}"
    );
}

/// QDOM13: scratchy never fires scrollend, so `window.onscrollend` stays undefined (tests wait for it when it exists).
#[test]
fn qdom13_no_scrollend_handler_names() {
    let log = qdom13_run("console.log(window.onscrollend, 'onscrollend' in document.body, window.onscroll);");
    assert_eq!(log, ["log:undefined false null"], "{log:?}");
}

#[test]
fn qdom13_handler_is_one_listener_entry() {
    let log = qdom13_run(
        "var d = document.createElement('div'); var n = 0; d.onclick = function () { n++; }; d.onclick = function () { n += 10; };\
         d.dispatchEvent(new Event('click')); console.log(n);\
         d.removeEventListener('click', d.onclick); d.dispatchEvent(new Event('click')); console.log(n);\
         d.removeEventListener('click', null); d.dispatchEvent(new Event('click')); console.log(n);",
    );
    assert_eq!(log, ["log:10", "log:20", "log:30"], "{log:?}");
}
