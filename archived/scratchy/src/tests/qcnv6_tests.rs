// QCNV6: canvas paths (beginPath/moveTo/lineTo/closePath/rect, fill/stroke/clip with fill rules), pixel readback.
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
fn qcnv6_fill_triangle_and_rect() {
    let l = js("x.fillStyle = '#f00'; x.beginPath(); x.moveTo(0, 0); x.lineTo(10, 0); x.lineTo(0, 10); x.fill();\
         console.log(px(1, 1), px(8, 8));\
         x.beginPath(); x.rect(12, 12, 6, 6); x.fillStyle = '#0f0'; x.fill(); console.log(px(14, 14), px(11, 11));\
         x.beginPath(); x.rect(5, 19, -3, -3); x.fill(); console.log(px(3, 17));");
    assert_eq!(l, ["log:255,0,0,255 0,0,0,0", "log:0,255,0,255 0,0,0,0", "log:0,255,0,255"]);
}

#[test]
fn qcnv6_fill_rules() {
    let code = "x.fillStyle = '#f00'; x.beginPath(); x.rect(0, 0, 20, 20); x.rect(5, 5, 10, 10); x.fill(RULE); console.log(px(10, 10), px(2, 2));";
    assert_eq!(js(&code.replace("RULE", "")), ["log:255,0,0,255 255,0,0,255"]);
    assert_eq!(js(&code.replace("RULE", "'nonzero'")), ["log:255,0,0,255 255,0,0,255"]);
    assert_eq!(js(&code.replace("RULE", "'evenodd'")), ["log:0,0,0,0 255,0,0,255"]);
    let l = js("try { x.fill('bogus'); } catch (e) { console.log(e.name); } try { x.clip('x'); } catch (e) { console.log(e.name); }");
    assert_eq!(l, ["log:TypeError", "log:TypeError"]);
}

#[test]
fn qcnv6_stroke_and_close() {
    let l = js("x.strokeStyle = '#00f'; x.lineWidth = 2; x.beginPath(); x.moveTo(5, 5); x.lineTo(15, 5); x.lineTo(15, 15); x.stroke();\
         console.log(px(10, 5), px(15, 10), px(10, 10), px(5, 10));\
         x.clearRect(0, 0, 20, 20); x.beginPath(); x.moveTo(5, 5); x.lineTo(15, 5); x.lineTo(15, 15); x.closePath(); x.stroke();\
         console.log(px(10, 10), px(5, 10));");
    assert_eq!(l, ["log:0,0,255,255 0,0,255,255 0,0,0,0 0,0,0,0", "log:0,0,255,255 0,0,0,0"]);
}

#[test]
fn qcnv6_path_uses_ctm_when_added() {
    let l = js("x.fillStyle = '#f00'; x.beginPath(); x.translate(10, 0); x.rect(0, 0, 5, 5); x.translate(-10, 0); x.fill();\
         console.log(px(12, 2), px(2, 2));\
         x.clearRect(0, 0, 20, 20); x.setTransform(2, 0, 0, 2, 0, 0); x.strokeStyle = '#00f'; x.lineWidth = 1;\
         x.beginPath(); x.moveTo(2, 2); x.lineTo(8, 2); x.stroke();\
         console.log(px(8, 3), px(8, 4), px(8, 2), px(8, 5));");
    assert_eq!(l, ["log:255,0,0,255 0,0,0,0", "log:0,0,255,255 0,0,255,255 0,0,0,0 0,0,0,0"]);
}

#[test]
fn qcnv6_subpath_rules() {
    // lineTo with no subpath only starts one; moveTo alone draws nothing; non-finite arguments are ignored;
    // after closePath the next lineTo starts from the closed subpath's first point; rect starts a new subpath.
    let l = js("x.fillStyle = '#f00'; x.beginPath(); x.lineTo(2, 2); x.moveTo(NaN, 4); x.lineTo(Infinity, 3); x.fill(); x.stroke();\
         console.log(px(2, 2), px(3, 3));\
         x.beginPath(); x.moveTo(0, 0); x.lineTo(10, 0); x.lineTo(10, 10); x.closePath(); x.lineTo(0, 10); x.fill();\
         console.log(px(2, 8), px(8, 2), px(18, 18));\
         x.clearRect(0, 0, 20, 20); x.beginPath(); x.rect(0, 0, 4, 4); x.lineTo(9, 9); x.stroke(); x.fill();\
         console.log(px(1, 1), px(8, 8));\
         x.beginPath(); x.fill(); x.rect(NaN, 0, 4, 4); x.fill(); x.beginPath(); x.rect(6, 1, 0, 5); x.fill(); console.log(px(6, 2));");
    assert_eq!(l, ["log:0,0,0,0 0,0,0,0", "log:0,0,0,0 255,0,0,255 0,0,0,0", "log:255,0,0,255 0,0,0,255", "log:0,0,0,0"]);
}

#[test]
fn qcnv6_clip_and_save_restore() {
    let l = js("x.fillStyle = '#f00'; x.save(); x.beginPath(); x.rect(5, 5, 10, 10); x.clip(); x.fillRect(0, 0, 20, 20);\
         console.log(px(10, 10), px(2, 2), px(16, 16));\
         x.beginPath(); x.rect(10, 0, 10, 10); x.clip(); x.fillStyle = '#0f0'; x.fillRect(0, 0, 20, 20);\
         console.log(px(12, 7), px(7, 7), px(12, 12));\
         x.restore(); x.fillStyle = '#00f'; x.fillRect(0, 15, 20, 5); console.log(px(2, 17), px(18, 17));\
         x.clearRect(0, 0, 20, 20); x.save(); x.beginPath(); x.rect(0, 0, 20, 20); x.rect(5, 5, 10, 10); x.clip('evenodd'); x.fillRect(0, 0, 20, 20);\
         console.log(px(10, 10), px(2, 2)); x.restore();\
         x.clearRect(0, 0, 20, 20); x.save(); x.beginPath(); x.clip(); x.fillRect(0, 0, 20, 20); console.log(px(2, 2)); x.restore();\
         x.save(); x.beginPath(); x.moveTo(0, 0); x.lineTo(20, 0); x.clip(); x.fillRect(0, 0, 20, 20); console.log(px(2, 2));");
    assert_eq!(
        l,
        [
            "log:255,0,0,255 0,0,0,0 0,0,0,0",
            "log:0,255,0,255 255,0,0,255 255,0,0,255",
            "log:0,0,255,255 0,0,255,255",
            "log:0,0,0,0 0,0,255,255",
            "log:0,0,0,0",
            "log:0,0,0,0"
        ]
    );
}

#[test]
fn qcnv6_clip_limits_stroke_fill_and_clear() {
    let l = js("x.beginPath(); x.rect(0, 0, 10, 20); x.clip(); x.beginPath(); x.moveTo(0, 10); x.lineTo(20, 10); x.lineWidth = 4; x.strokeStyle = '#00f'; x.stroke();\
         console.log(px(5, 10), px(15, 10));\
         x.fillStyle = '#f00'; x.fillRect(0, 0, 20, 4); console.log(px(5, 1), px(15, 1));\
         x.setTransform(1, 0, 0, 1, 0, 0); x.clearRect(0, 0, 20, 20); console.log(px(5, 10));");
    assert_eq!(l, ["log:0,0,255,255 0,0,0,0", "log:255,0,0,255 0,0,0,0", "log:0,0,0,0"]);
}

#[test]
fn qcnv6_reset_and_offscreen() {
    let l = js("x.beginPath(); x.rect(0, 0, 20, 20); x.clip(); var c = document.getElementById('c'); c.width = 20;\
         x.fillRect(0, 0, 20, 20); console.log(px(10, 10)); x.rect(0, 0, 4, 4); c.width = 20; x.fill(); console.log(px(1, 1));\
         var o = new OffscreenCanvas(10, 10).getContext('2d'); o.fillStyle = '#f00'; o.beginPath(); o.moveTo(0, 0); o.lineTo(10, 0); o.lineTo(0, 10); o.fill();\
         console.log(Array.from(o.getImageData(1, 1, 1, 1).data).join(','), Array.from(o.getImageData(8, 8, 1, 1).data).join(','));\
         o.beginPath(); o.rect(0, 0, 3, 3); o.clip(); o.clearRect(0, 0, 10, 10); console.log(Array.from(o.getImageData(1, 1, 1, 1).data).join(','), Array.from(o.getImageData(5, 1, 1, 1).data).join(','));");
    assert_eq!(l, ["log:0,0,0,255", "log:0,0,0,0", "log:255,0,0,255 0,0,0,0", "log:0,0,0,0 255,0,0,255"]);
}
