// QCNV7: canvas curves (arc, arcTo, ellipse, quadraticCurveTo, bezierCurveTo, roundRect), pixel readback.
use super::*;

const DOC: &str = "<html><body><canvas id=c width=40 height=40></canvas></body></html>";

fn js(code: &str) -> Vec<String> {
    run_in_html(
        DOC,
        &format!(
            "var x = document.getElementById('c').getContext('2d');\
             function px(a, b) {{ return Array.from(x.getImageData(a, b, 1, 1).data).join(','); }}{code}"
        ),
    )
}

const RED: &str = "255,0,0,255";
const CLEAR: &str = "0,0,0,0";

#[test]
fn qcnv7_arc_full_circle_and_directions() {
    let l = js("x.fillStyle = '#f00'; x.beginPath(); x.arc(20, 20, 10, 0, 2 * Math.PI); x.fill();\
         console.log(px(20, 20), px(28, 20), px(2, 2), px(20, 32));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.moveTo(20, 20); x.arc(20, 20, 15, 0, Math.PI / 2); x.fill();\
         console.log(px(25, 25), px(10, 25), px(25, 10));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.moveTo(20, 20); x.arc(20, 20, 15, 0, Math.PI / 2, true); x.fill();\
         console.log(px(25, 25), px(10, 25), px(25, 10), px(30, 15));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.arc(20, 20, 10, 7, 0, true); x.fill(); console.log(px(20, 20));");
    assert_eq!(
        l,
        [
            format!("log:{RED} {RED} {CLEAR} {CLEAR}"),
            format!("log:{RED} {CLEAR} {CLEAR}"),
            format!("log:{CLEAR} {RED} {RED} {RED}"),
            format!("log:{RED}")
        ]
    );
}

#[test]
fn qcnv7_arc_errors_and_ignored() {
    let l = js("try { x.arc(0, 0, -1, 0, 1); } catch (e) { console.log(e.name, e.code, e instanceof DOMException); }\
         try { x.ellipse(0, 0, 1, -1, 0, 0, 1); } catch (e) { console.log(e.name); }\
         try { x.arcTo(0, 0, 1, 1, -5); } catch (e) { console.log(e.name); }\
         x.arc(0, 0, -1, NaN, 1); x.arc(Infinity, 0, -1, 0, 1); x.arcTo(0, 0, NaN, 1, -1); console.log('ok');\
         x.fillStyle = '#f00'; x.beginPath(); x.arc(NaN, 0, 10, 0, 7); x.rect(0, 0, 10, 10); x.fill(); console.log(px(5, 5));");
    assert_eq!(l, ["log:IndexSizeError 1 true", "log:IndexSizeError", "log:IndexSizeError", "log:ok", &format!("log:{RED}")]);
}

#[test]
fn qcnv7_arc_through_ctm() {
    let l = js("x.fillStyle = '#f00'; x.translate(20, 20); x.scale(2, 1); x.beginPath(); x.arc(0, 0, 5, 0, 7); x.fill();\
         console.log(px(28, 20), px(31, 20), px(20, 24), px(20, 26));\
         x.setTransform(1, 0, 0, 1, 0, 0); x.clearRect(0, 0, 40, 40); x.beginPath(); x.arc(5, 5, 4, 0, 7); x.translate(20, 20); x.fill();\
         console.log(px(5, 5), px(25, 25));");
    assert_eq!(l, [format!("log:{RED} {CLEAR} {RED} {CLEAR}"), format!("log:{RED} {CLEAR}")]);
}

#[test]
fn qcnv7_ellipse_rotation() {
    let l = js("x.fillStyle = '#f00'; x.beginPath(); x.ellipse(20, 20, 15, 4, 0, 0, 2 * Math.PI); x.fill();\
         console.log(px(32, 20), px(20, 30), px(20, 22));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.ellipse(20, 20, 15, 4, Math.PI / 2, 0, 2 * Math.PI); x.fill();\
         console.log(px(20, 32), px(30, 20), px(22, 20));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.ellipse(20, 20, 15, 4, 0, 0, Math.PI); x.fill();\
         console.log(px(20, 22), px(20, 17));");
    assert_eq!(l, [format!("log:{RED} {CLEAR} {RED}"), format!("log:{RED} {CLEAR} {RED}"), format!("log:{RED} {CLEAR}")]);
}

#[test]
fn qcnv7_arc_to() {
    // Rounded corner at (30, 10) between a line from (10, 10) to (30, 10) and one down to (30, 30).
    let l = js("x.fillStyle = '#f00'; x.beginPath(); x.moveTo(10, 10); x.arcTo(30, 10, 30, 30, 10); x.lineTo(30, 30); x.lineTo(10, 30); x.closePath(); x.fill();\
         console.log(px(15, 15), px(29, 29), px(29, 11), px(35, 20));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.moveTo(10, 10); x.arcTo(30, 10, 30, 30, 0); x.lineTo(30, 30); x.lineTo(10, 30); x.fill();\
         console.log(px(29, 11));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.moveTo(5, 5); x.arcTo(15, 5, 35, 5, 10); x.lineTo(5, 35); x.fill(); console.log(px(8, 8));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.arcTo(30, 10, 30, 30, 5); x.lineTo(5, 35); x.lineTo(30, 35); x.fill(); console.log(px(28, 30));");
    assert_eq!(l, [format!("log:{RED} {RED} {CLEAR} {CLEAR}"), format!("log:{RED}"), format!("log:{RED}"), format!("log:{RED}")]);
}

#[test]
fn qcnv7_quadratic_and_bezier() {
    let l = js("x.fillStyle = '#f00'; x.beginPath(); x.moveTo(0, 0); x.quadraticCurveTo(40, 0, 40, 40); x.lineTo(0, 40); x.fill();\
         console.log(px(5, 35), px(38, 5), px(30, 5));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.moveTo(0, 20); x.bezierCurveTo(0, 0, 40, 0, 40, 20); x.lineTo(0, 20); x.fill();\
         console.log(px(20, 12), px(20, 25), px(2, 4));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.quadraticCurveTo(20, 5, 5, 30); x.lineTo(5, 5); x.fill(); x.stroke(); console.log('nosubpath');");
    assert_eq!(l, [format!("log:{RED} {CLEAR} {CLEAR}"), format!("log:{RED} {CLEAR} {CLEAR}"), "log:nosubpath".to_string()]);
}

#[test]
fn qcnv7_round_rect() {
    let l = js("x.fillStyle = '#f00'; x.beginPath(); x.roundRect(5, 5, 30, 30, 10); x.fill();\
         console.log(px(20, 20), px(6, 6), px(6, 20), px(20, 6));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.roundRect(5, 5, 30, 30, [10, 0]); x.fill();\
         console.log(px(6, 6), px(33, 6), px(6, 33), px(33, 33));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.roundRect(35, 5, -30, 30, { x: 12, y: 12 }); x.fill(); console.log(px(6, 6), px(20, 20));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.roundRect(5, 5, 30, 30, [8, 0, 0, 0]); x.fill();\
         console.log(px(6, 6), px(33, 6), px(33, 33), px(6, 33));");
    assert_eq!(
        l,
        [
            format!("log:{RED} {CLEAR} {RED} {RED}"),
            format!("log:{CLEAR} {RED} {RED} {CLEAR}"),
            format!("log:{CLEAR} {RED}"),
            format!("log:{CLEAR} {RED} {RED} {RED}")
        ]
    );
}

#[test]
fn qcnv7_round_rect_errors() {
    let l = js("var t = function (f) { try { f(); console.log('no'); } catch (e) { console.log(e.name); } };\
         t(function () { x.roundRect(0, 0, 10, 10, -1); });\
         t(function () { x.roundRect(0, 0, 10, 10, [1, 2, 3, 4, 5]); });\
         t(function () { x.roundRect(0, 0, 10, 10, []); });\
         t(function () { x.roundRect(0, 0, 10, 10, [{ x: 1, y: -1 }]); });\
         t(function () { x.roundRect(0, 0, 10, 10, 0n); });\
         t(function () { x.roundRect(0, 0, 10, 10, { foo: 'bar' }); });\
         t(function () { x.roundRect(NaN, 0, 10, 10, -1); });\
         t(function () { x.roundRect(0, 0, 10, 10, [NaN]); });\
         t(function () { x.roundRect(0, 0, 10); });");
    assert_eq!(l, ["log:RangeError", "log:RangeError", "log:RangeError", "log:RangeError", "log:TypeError", "log:no", "log:no", "log:no", "log:TypeError"]);
}

#[test]
fn qcnv7_round_rect_negative_size_reverses_winding() {
    // A mirrored (one negative size) rectangle runs the other way, so under nonzero it cancels an overlapping one.
    let l = js("x.fillStyle = '#f00'; x.beginPath(); x.roundRect(0, 0, 20, 20, [0]); x.roundRect(0, 20, 20, -20, [3]); x.fill(); console.log(px(10, 10));\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.roundRect(0, 0, 20, 20, [0]); x.roundRect(20, 20, -20, -20, [3]); x.fill(); console.log(px(10, 10));");
    assert_eq!(l, [format!("log:{CLEAR}"), format!("log:{RED}")]);
}

#[test]
fn qcnv7_offscreen_canvas() {
    let l = run_in_html(
        DOC,
        "var o = new OffscreenCanvas(40, 40), x = o.getContext('2d'); x.fillStyle = '#f00'; x.beginPath(); x.arc(20, 20, 10, 0, 7); x.fill();\
         var d = x.getImageData(20, 20, 1, 1).data, e = x.getImageData(2, 2, 1, 1).data; console.log(d[0], d[3], e[3]);\
         x.clearRect(0, 0, 40, 40); x.beginPath(); x.roundRect(0, 0, 40, 40, 20); x.fill(); console.log(x.getImageData(1, 1, 1, 1).data[3], x.getImageData(20, 20, 1, 1).data[3]);",
    );
    assert_eq!(l, ["log:255 255 0", "log:0 255"]);
}
