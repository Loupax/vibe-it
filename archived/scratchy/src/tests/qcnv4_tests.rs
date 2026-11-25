// QCNV4: OffscreenCanvas on the main thread: constructor, size, getContext('2d') sharing canvas.rs, reset on resize.
use super::*;

const DOC: &str = "<html><body><canvas id=c width=20 height=10></canvas></body></html>";

fn js(code: &str) -> Vec<String> {
    run_in_html(DOC, code)
}

#[test]
fn qcnv4_constructor_and_size() {
    let l = js("var o = new OffscreenCanvas(30, 20); console.log(o.width, o.height, Object.prototype.toString.call(o), o instanceof EventTarget);\
         o.width = 7.9; o.height = 0; console.log(o.width, o.height);\
         function t(f) { try { f(); return 'no'; } catch (e) { return e.name; } }\
         console.log(t(function () { OffscreenCanvas(1, 1); }), t(function () { new OffscreenCanvas(1); }), t(function () { new OffscreenCanvas(-1, 1); }), t(function () { new OffscreenCanvas(NaN, 1); }), t(function () { o.width = 4294967296; }));\
         console.log(t(function () { Object.getOwnPropertyDescriptor(OffscreenCanvas.prototype, 'width').get.call({}); }));");
    assert_eq!(l, ["log:30 20 [object OffscreenCanvas] true", "log:7 0", "log:TypeError TypeError TypeError TypeError TypeError", "log:TypeError"]);
}

#[test]
fn qcnv4_context_draws_and_is_cached() {
    let l = js("var o = new OffscreenCanvas(4, 4); var x = o.getContext('2d');\
         console.log(Object.prototype.toString.call(x), x === o.getContext('2d'), x.canvas === o, o.getContext('webgl'));\
         x.fillStyle = '#f00'; x.fillRect(1, 1, 2, 2);\
         console.log(Array.from(x.getImageData(1, 1, 1, 1).data).join(','), Array.from(x.getImageData(0, 0, 1, 1).data).join(','));\
         x.fillStyle = 'currentcolor'; console.log(x.fillStyle);");
    assert_eq!(l, ["log:[object OffscreenCanvasRenderingContext2D] true true null", "log:255,0,0,255 0,0,0,0", "log:#000000"]);
}

#[test]
fn qcnv4_resize_resets_bitmap_and_state() {
    let l = js("var o = new OffscreenCanvas(4, 4); var x = o.getContext('2d'); x.fillStyle = 'blue'; x.globalAlpha = 0.5; x.fillRect(0, 0, 4, 4);\
         o.width = 8; console.log(x.fillStyle, x.globalAlpha, Array.from(x.getImageData(0, 0, 1, 1).data).join(','), x.getImageData(0, 0, 100, 100).width);");
    assert_eq!(l, ["log:#000000 1 0,0,0,0 100"]);
}

#[test]
fn qcnv4_independent_of_the_element_canvas() {
    let l = js("var e = document.getElementById('c').getContext('2d'); var o = new OffscreenCanvas(20, 10).getContext('2d');\
         o.fillStyle = 'lime'; o.fillRect(0, 0, 20, 10);\
         console.log(Array.from(e.getImageData(0, 0, 1, 1).data).join(','), Array.from(o.getImageData(0, 0, 1, 1).data).join(','), e.canvas.tagName);\
         function t(f) { try { f(); return 'no'; } catch (er) { return er.name; } }\
         console.log(t(function () { CanvasRenderingContext2D.prototype.fillRect.call(o, 0, 0, 1, 1); }), typeof OffscreenCanvasRenderingContext2D, t(function () { new OffscreenCanvasRenderingContext2D(); }));");
    assert_eq!(l[0], "log:0,0,0,0 0,255,0,255 CANVAS");
}
