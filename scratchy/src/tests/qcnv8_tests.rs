// QCNV8: canvas line styles (width, cap, join, miterLimit, dashes, offset), pixel readback.
use super::*;

const DOC: &str = "<html><body><canvas id=c width=60 height=60></canvas></body></html>";

fn js(code: &str) -> Vec<String> {
    run_in_html(
        DOC,
        &format!(
            "var x = document.getElementById('c').getContext('2d');\
             function px(a, b) {{ return Array.from(x.getImageData(a, b, 1, 1).data).join(','); }}{code}"
        ),
    )
}

const BLACK: &str = "0,0,0,255";
const CLEAR: &str = "0,0,0,0";

#[test]
fn qcnv8_defaults_and_invalid_values_ignored() {
    let l = js("console.log(x.lineWidth, x.lineCap, x.lineJoin, x.miterLimit, x.lineDashOffset, JSON.stringify(x.getLineDash()));\
         x.lineWidth = 0; x.lineWidth = -3; x.lineWidth = NaN; x.lineWidth = Infinity; console.log(x.lineWidth);\
         x.lineCap = 'round'; x.lineCap = 'bogus'; x.lineJoin = 'bevel'; x.lineJoin = 'ROUND'; console.log(x.lineCap, x.lineJoin);\
         x.miterLimit = 3; x.miterLimit = 0; x.miterLimit = -1; x.miterLimit = NaN; console.log(x.miterLimit);\
         x.lineDashOffset = 4; x.lineDashOffset = Infinity; console.log(x.lineDashOffset);");
    assert_eq!(l, ["log:1 butt miter 10 0 []", "log:1", "log:round bevel", "log:3", "log:4"]);
}

#[test]
fn qcnv8_dash_list_rules() {
    let l = js("x.setLineDash([1, 2, 3]); console.log(JSON.stringify(x.getLineDash()));\
         x.setLineDash([5, -1]); x.setLineDash([5, NaN]); x.setLineDash([Infinity]); console.log(JSON.stringify(x.getLineDash()));\
         var d = x.getLineDash(); d.push(9); console.log(JSON.stringify(x.getLineDash()));\
         x.setLineDash([]); console.log(JSON.stringify(x.getLineDash()));\
         try { x.setLineDash(); } catch (e) { console.log(e.name); }\
         try { x.setLineDash(5); } catch (e) { console.log(e.name); }");
    assert_eq!(l, ["log:[1,2,3,1,2,3]", "log:[1,2,3,1,2,3]", "log:[1,2,3,1,2,3]", "log:[]", "log:TypeError", "log:TypeError"]);
}

#[test]
fn qcnv8_save_restore() {
    let l = js("x.lineCap = 'square'; x.lineJoin = 'round'; x.miterLimit = 2; x.setLineDash([4]); x.lineDashOffset = 1; x.lineWidth = 5; x.save();\
         x.lineCap = 'butt'; x.lineJoin = 'miter'; x.miterLimit = 7; x.setLineDash([]); x.lineDashOffset = 0; x.lineWidth = 1; x.restore();\
         console.log(x.lineCap, x.lineJoin, x.miterLimit, x.lineDashOffset, x.lineWidth, JSON.stringify(x.getLineDash()));");
    assert_eq!(l, ["log:square round 2 1 5 [4,4]"]);
}

#[test]
fn qcnv8_caps() {
    let l = js("x.lineWidth = 10; x.beginPath(); x.moveTo(20, 30); x.lineTo(40, 30); x.stroke();\
         console.log(px(17, 30), px(43, 30));\
         x.clearRect(0, 0, 60, 60); x.lineCap = 'square'; x.beginPath(); x.moveTo(20, 30); x.lineTo(40, 30); x.stroke();\
         console.log(px(17, 26), px(43, 33), px(13, 30));\
         x.clearRect(0, 0, 60, 60); x.lineCap = 'round'; x.beginPath(); x.moveTo(20, 30); x.lineTo(40, 30); x.stroke();\
         console.log(px(17, 30), px(15, 24), px(13, 30));");
    assert_eq!(
        l,
        [
            format!("log:{CLEAR} {CLEAR}"),
            format!("log:{BLACK} {BLACK} {CLEAR}"),
            format!("log:{BLACK} {CLEAR} {CLEAR}")
        ]
    );
}

#[test]
fn qcnv8_joins_and_miter_limit() {
    // Sharp corner at (30, 40): the miter tip reaches far below, a bevel does not.
    let path = "x.lineWidth = 6; x.beginPath(); x.moveTo(10, 10); x.lineTo(30, 40); x.lineTo(50, 10); x.stroke();";
    let l = js(&format!(
        "x.lineJoin = 'miter'; x.miterLimit = 100; {path} console.log(px(30, 43));\
         x.clearRect(0, 0, 60, 60); x.miterLimit = 1; {path} console.log(px(30, 43));\
         x.clearRect(0, 0, 60, 60); x.lineJoin = 'bevel'; {path} console.log(px(30, 43));\
         x.clearRect(0, 0, 60, 60); x.lineJoin = 'round'; {path} console.log(px(30, 41), px(30, 44));"
    ));
    assert_eq!(l, [format!("log:{BLACK}"), format!("log:{CLEAR}"), format!("log:{CLEAR}"), format!("log:{BLACK} {CLEAR}")]);
}

#[test]
fn qcnv8_dash_and_offset() {
    let l = js("x.lineWidth = 4; x.setLineDash([10, 10]); x.beginPath(); x.moveTo(0, 30); x.lineTo(60, 30); x.stroke();\
         console.log(px(5, 30), px(15, 30), px(25, 30), px(35, 30));\
         x.clearRect(0, 0, 60, 60); x.lineDashOffset = 10; x.beginPath(); x.moveTo(0, 30); x.lineTo(60, 30); x.stroke();\
         console.log(px(5, 30), px(15, 30), px(25, 30));\
         x.clearRect(0, 0, 60, 60); x.lineDashOffset = 0; x.setLineDash([0, 0]); x.beginPath(); x.moveTo(0, 30); x.lineTo(60, 30); x.stroke();\
         console.log(px(5, 30));");
    assert_eq!(
        l,
        [
            format!("log:{BLACK} {CLEAR} {BLACK} {CLEAR}"),
            format!("log:{CLEAR} {BLACK} {CLEAR}"),
            format!("log:{BLACK}")
        ]
    );
}

#[test]
fn qcnv8_through_ctm_and_stroke_rect() {
    let l = js("x.scale(2, 1); x.lineWidth = 5; x.strokeRect(10, 20, 10, 10);\
         console.log(px(12, 25), px(22, 25), px(30, 25), px(36, 25));\
         x.setTransform(1, 0, 0, 1, 0, 0); x.clearRect(0, 0, 60, 60); x.lineWidth = 4; x.setLineDash([6, 6]); x.strokeRect(10, 10, 40, 40);\
         console.log(px(13, 10), px(19, 10), px(25, 10));");
    assert_eq!(l, [format!("log:{CLEAR} {BLACK} {CLEAR} {BLACK}"), format!("log:{BLACK} {CLEAR} {BLACK}")]);
}

#[test]
fn qcnv8_offscreen() {
    let l = run_in_html(
        DOC,
        "var x = new OffscreenCanvas(60, 60).getContext('2d'); x.lineWidth = 10; x.lineCap = 'square'; x.setLineDash([5, 5, 5]);\
         console.log(x.lineCap, JSON.stringify(x.getLineDash()), x.miterLimit);\
         x.setLineDash([]); x.beginPath(); x.moveTo(20, 30); x.lineTo(40, 30); x.stroke(); x.lineDashOffset = 5;\
         console.log(Array.from(x.getImageData(17, 26, 1, 1).data).join(','), x.lineDashOffset);",
    );
    assert_eq!(l, ["log:square [5,5,5,5,5,5] 10", "log:0,0,0,255 5"]);
}
