// QCNV5: canvas transforms (translate/rotate/scale/transform/setTransform/resetTransform/getTransform), pixel readback.
use super::*;

const DOC: &str = "<html><body><canvas id=c width=20 height=20></canvas></body></html>";

fn js(code: &str) -> Vec<String> {
    run_in_html(
        DOC,
        &format!(
            "var x = document.getElementById('c').getContext('2d');\
             function px(a, b) {{ return Array.from(x.getImageData(a, b, 1, 1).data).join(','); }}{code}"
        ),
    )
}

#[test]
fn qcnv5_translate_scale_fill_and_clear() {
    let l = js("x.fillStyle = '#f00'; x.translate(5, 5); x.scale(2, 2); x.fillRect(1, 1, 3, 3);\
         console.log(px(7, 7), px(6, 6), px(12, 12), px(13, 13));\
         x.clearRect(1, 1, 1, 1); console.log(px(7, 7), px(10, 10));");
    assert_eq!(l, ["log:255,0,0,255 0,0,0,0 255,0,0,255 0,0,0,0", "log:0,0,0,0 255,0,0,255"]);
}

#[test]
fn qcnv5_rotate_and_order() {
    let l = js("x.fillStyle = '#0f0'; x.translate(10, 10); x.rotate(Math.PI / 2); x.fillRect(0, 0, 5, 2);\
         console.log(px(9, 12), px(8, 14), px(11, 11), px(12, 10));");
    assert_eq!(l, ["log:0,255,0,255 0,255,0,255 0,0,0,0 0,0,0,0"]);
}

#[test]
fn qcnv5_stroke_rect_uses_ctm() {
    let l = js("x.strokeStyle = '#00f'; x.lineWidth = 2; x.scale(2, 1); x.strokeRect(5, 10, 3, 4);\
         console.log(px(10, 10), px(9, 10), px(12, 10), px(13, 10), px(12, 12));");
    // line width 2 scales to 4 px horizontally: the top edge spans x 8..16 and y 9..11.
    assert_eq!(l, ["log:0,0,255,255 0,0,255,255 0,0,255,255 0,0,255,255 0,0,0,0"]);
}

#[test]
fn qcnv5_transform_set_and_reset() {
    let l = js("x.fillStyle = '#f00'; x.transform(1, 0, 0, 1, 3, 0); x.transform(1, 0, 0, 1, 2, 0); x.fillRect(0, 0, 1, 1);\
         console.log(px(5, 0), px(0, 0));\
         x.setTransform(1, 0, 0, 1, 0, 10); x.fillRect(0, 0, 1, 1); console.log(px(0, 10));\
         x.setTransform({ e: 7, f: 7 }); x.fillRect(0, 0, 1, 1); console.log(px(7, 7));\
         x.setTransform({ m41: 1, m42: 2, m11: 2 }); console.log(JSON.stringify([x.getTransform().a, x.getTransform().e, x.getTransform().f]));\
         x.resetTransform(); console.log(x.getTransform().isIdentity);\
         x.setTransform(); console.log(x.getTransform().isIdentity);");
    assert_eq!(l, ["log:255,0,0,255 0,0,0,0", "log:255,0,0,255", "log:255,0,0,255", "log:[2,1,2]", "log:true", "log:true"]);
}

#[test]
fn qcnv5_nonfinite_ignored_and_argument_errors() {
    let l = js("x.translate(3, 0); x.translate(NaN, 1); x.scale(Infinity, 1); x.rotate(NaN); x.transform(1, 0, 0, 1, 0, Infinity);\
         x.setTransform(NaN, 0, 0, 1, 0, 0); x.setTransform({ a: NaN });\
         var t = x.getTransform(); console.log(t.a, t.b, t.e, t.f);\
         function e(f) { try { f(); return 'no'; } catch (er) { return er.name; } }\
         console.log(e(function () { x.translate(1); }), e(function () { x.setTransform(1, 2, 3); }), e(function () { x.setTransform({ a: 1, m11: 2 }); }), e(function () { x.setTransform(5); }));\
         var o = new OffscreenCanvas(4, 4).getContext('2d'); o.scale(2, 2); o.fillRect(0, 0, 1, 1);\
         console.log(Array.from(o.getImageData(1, 1, 1, 1).data).join(','), Array.from(o.getImageData(2, 2, 1, 1).data).join(','));");
    assert_eq!(l, ["log:1 0 3 0", "log:TypeError TypeError TypeError TypeError", "log:0,0,0,255 0,0,0,0"]);
}

#[test]
fn qcnv5_save_restore_and_reset() {
    let l = js("x.translate(4, 0); x.save(); x.translate(4, 0); console.log(x.getTransform().e); x.restore(); console.log(x.getTransform().e);\
         x.restore(); console.log(x.getTransform().e);\
         var c = document.getElementById('c'); x.scale(3, 3); c.width = 20; console.log(x.getTransform().isIdentity);");
    assert_eq!(l, ["log:8", "log:4", "log:4", "log:true"]);
}
