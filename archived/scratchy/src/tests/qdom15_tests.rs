// QDOM15: Event interfaces: constructors, init dictionaries, createEvent table, constants.
use super::*;

#[test]
fn qdom15_constants_and_call_without_new() {
    let log = run_in_html(
        "<html><body></body></html>",
        "function thr(f) { try { f(); return 'no'; } catch (e) { return e.name; } }\
         var e = document.createEvent('Event');\
         console.log([Event.NONE, Event.CAPTURING_PHASE, Event.AT_TARGET, Event.BUBBLING_PHASE].join(), [Event.prototype.NONE, Event.prototype.BUBBLING_PHASE, e.AT_TARGET].join(), document.createEvent('CustomEvent').CAPTURING_PHASE);\
         var d = Object.getOwnPropertyDescriptor(Event, 'AT_TARGET'); console.log(d.writable, d.enumerable, d.configurable);\
         console.log(thr(function () { Event('x'); }), thr(function () { UIEvent('x'); }), thr(function () { new Event(); }), thr(function () { new Event('x', 7); }),\
           thr(function () { new Event(Symbol()); }), thr(function () { new MouseEvent('x', 3); }));",
    );
    assert_eq!(log, ["log:0,1,2,3 0,3,2 1", "log:false true false", "log:TypeError TypeError TypeError TypeError TypeError TypeError"]);
}

#[test]
fn qdom15_ui_event_init_dictionaries() {
    let log = run_in_html(
        "<html><body></body></html>",
        "function thr(f) { try { f(); return 'no'; } catch (e) { return e.name; } }\
         var u = new UIEvent('u'); console.log(u.view, u.detail, 'view' in u);\
         u = new UIEvent('u', { view: window, detail: 7.9 }); console.log(u.view === window, u.detail);\
         console.log(thr(function () { new UIEvent('x', { view: 7 }); }), thr(function () { new UIEvent('x', { view: {} }); }), thr(function () { new UIEvent('x', { view: null }); }));\
         var m = new MouseEvent('m', { clientX: 40, screenY: 3.5, button: 2, buttons: 5, ctrlKey: true, relatedTarget: document, view: window });\
         console.log(m.clientX, m.screenY, m.button, m.buttons, m.ctrlKey, m.shiftKey, m.relatedTarget === document, m.view === window, m instanceof UIEvent);\
         console.log(thr(function () { new MouseEvent('m', { relatedTarget: 5 }); }), thr(function () { new MouseEvent('m', { clientX: Infinity }); }), thr(function () { new FocusEvent('f', { relatedTarget: {} }); }));\
         var w = new WheelEvent('w', { deltaY: 3.1, deltaMode: 2, clientX: 1 }); console.log(w.deltaX, w.deltaY, w.deltaMode, w.clientX, w instanceof MouseEvent);\
         var k = new KeyboardEvent('k', { key: 'a', code: 'KeyA', location: 3, repeat: true, keyCode: 65, altKey: true, isComposing: true });\
         console.log(k.key, k.code, k.location, k.repeat, k.keyCode, k.altKey, k.isComposing, k.which, k.charCode);\
         var c = new CompositionEvent('c', { data: 12, detail: 2 }); console.log(c.data === '12', c.detail, new CompositionEvent('c').data === '', c instanceof UIEvent);\
         var i = new InputEvent('i', { data: 'x', isComposing: true, inputType: 'insertText' }); console.log(i.data, i.isComposing, i.inputType, new InputEvent('i').data);",
    );
    assert_eq!(
        log,
        [
            "log:null 0 true",
            "log:true 7",
            "log:TypeError TypeError no",
            "log:40 3.5 2 5 true false true true true",
            "log:TypeError TypeError TypeError",
            "log:0 3.1 2 1 true",
            "log:a KeyA 3 true 65 true true 65 0",
            "log:true 2 true true",
            "log:x true insertText null",
        ]
    );
}

#[test]
fn qdom15_create_event_table() {
    let log = run_in_html(
        "<html><body></body></html>",
        "function thr(f) { try { f(); return 'no'; } catch (e) { return e.name; } }\
         var names = ['BeforeUnloadEvent','CompositionEvent','CustomEvent','DeviceMotionEvent','DeviceOrientationEvent','DragEvent','Event','Events','FocusEvent','HashChangeEvent','HTMLEvents','KeyboardEvent','MessageEvent','MouseEvent','MouseEvents','StorageEvent','SVGEvents','TextEvent','UIEvent','UIEvents'];\
         var out = names.map(function (n) { var e = document.createEvent(n), e2 = document.createEvent(n.toUpperCase()); return Object.getPrototypeOf(e).constructor.name + (e.isTrusted === false && e.type === '' && e.eventPhase === 0 && e2.constructor === e.constructor ? '' : '!'); });\
         console.log(out.join());\
         console.log(thr(function () { document.createEvent('TouchEvent'); }), thr(function () { document.createEvent('ErrorEvent'); }), thr(function () { document.createEvent('constructor'); }), thr(function () { document.createEvent(''); }), thr(function () { document.createEvent(); }));\
         var ev = document.createEvent('Event'); console.log(thr(function () { document.dispatchEvent(ev); }));\
         ev.initEvent('q', true, false); console.log(document.dispatchEvent(ev), thr(function () { document.dispatchEvent(ev); }));\
         document.addEventListener('r', function (e) { try { document.dispatchEvent(e); } catch (x) { console.log('re:' + x.name); } });\
         document.dispatchEvent(new Event('r'));",
    );
    assert_eq!(
        log,
        [
            "log:BeforeUnloadEvent,CompositionEvent,CustomEvent,DeviceMotionEvent,DeviceOrientationEvent,DragEvent,Event,Event,FocusEvent,HashChangeEvent,Event,KeyboardEvent,MessageEvent,MouseEvent,MouseEvent,StorageEvent,Event,TextEvent,UIEvent,UIEvent",
            "log:NotSupportedError NotSupportedError NotSupportedError NotSupportedError NotSupportedError",
            "log:InvalidStateError",
            "log:true no",
            "log:re:InvalidStateError",
        ]
    );
}
