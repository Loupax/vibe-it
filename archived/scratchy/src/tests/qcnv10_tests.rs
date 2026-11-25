// QCNV10: createLinearGradient / createRadialGradient / createConicGradient, addColorStop, gradients as fill/stroke styles.
use super::*;

const DOC: &str = "<html><body><canvas id=c width=100 height=100></canvas></body></html>";

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
fn qcnv10_linear_fill_and_extension() {
    let l = js("var g = x.createLinearGradient(25, 0, 75, 0); g.addColorStop(0, '#f00'); g.addColorStop(1, '#00f');\
         x.fillStyle = g; x.fillRect(0, 0, 100, 10);\
         console.log(px(5, 5), px(95, 5), px(50, 5));\
         console.log(x.fillStyle === g, Object.prototype.toString.call(g));");
    assert_eq!(l[0], "log:255,0,0,255 0,0,255,255 125,0,130,255");
    assert_eq!(l[1], "log:true [object CanvasGradient]");
}

#[test]
fn qcnv10_errors_and_degenerate() {
    let l = js("var g = x.createLinearGradient(0, 0, 10, 0);\
         try { g.addColorStop(1.5, 'red'); } catch (e) { console.log(e.name); }\
         try { g.addColorStop(0.5, 'nope'); } catch (e) { console.log(e.name); }\
         try { g.addColorStop(NaN, 'red'); } catch (e) { console.log(e.name); }\
         try { x.createRadialGradient(0, 0, -1, 0, 0, 1); } catch (e) { console.log(e.name); }\
         try { x.createLinearGradient(0, 0, Infinity, 0); } catch (e) { console.log(e.name); }\
         try { new CanvasGradient(); } catch (e) { console.log(e.name); }\
         var d = x.createLinearGradient(5, 5, 5, 5); d.addColorStop(0, 'red'); x.fillStyle = d; x.fillRect(0, 0, 20, 20); console.log(px(10, 10));\
         var n = x.createLinearGradient(0, 0, 10, 0); x.fillStyle = n; x.fillRect(0, 0, 20, 20); console.log(px(10, 10));");
    assert_eq!(l, ["log:IndexSizeError", "log:SyntaxError", "log:TypeError", "log:IndexSizeError", "log:TypeError", "log:TypeError", "log:0,0,0,0", "log:0,0,0,0"]);
}

#[test]
fn qcnv10_radial_conic_and_ctm() {
    let l = js("var r = x.createRadialGradient(50, 50, 0, 50, 50, 40); r.addColorStop(0, '#fff'); r.addColorStop(1, '#000');\
         x.fillStyle = r; x.fillRect(0, 0, 100, 100); console.log(px(50, 50), px(95, 50));\
         var c = x.createConicGradient(0, 50, 50); c.addColorStop(0, '#f00'); c.addColorStop(0.5, '#0f0'); c.addColorStop(1, '#f00');\
         x.fillStyle = c; x.fillRect(0, 0, 100, 100); console.log(px(90, 51), px(10, 49));\
         x.clearRect(0, 0, 100, 100); var g = x.createLinearGradient(0, 0, 10, 0); g.addColorStop(0, '#000'); g.addColorStop(1, '#f00');\
         x.fillStyle = g; x.translate(50, 0); x.fillRect(0, 0, 10, 10); x.setTransform(1, 0, 0, 1, 0, 0);\
         console.log(px(50, 5), px(59, 5), px(20, 5));\
         x.strokeStyle = g; x.lineWidth = 6; x.strokeRect(5, 50, 20, 20); console.log(px(5, 50) !== '0,0,0,0');");
    assert_eq!(l[0], "log:250,250,250,255 0,0,0,255");
    assert_eq!(l[1], "log:252,3,0,255 1,254,0,255");
    assert!(l[2].starts_with("log:13,0,0,255 2"), "{}", l[2]);
    assert!(l[2].ends_with(" 0,0,0,0"), "{}", l[2]);
    assert_eq!(l[3], "log:true");
}

#[test]
fn qcnv10_style_save_restore_and_offscreen() {
    let l = js("var g = x.createLinearGradient(0, 0, 1, 0); x.fillStyle = g; x.save(); x.fillStyle = 'red'; console.log(x.fillStyle);\
         x.restore(); console.log(x.fillStyle === g); x.fillStyle = {toString: function () { return 'bogus'; }}; console.log(x.fillStyle === g);\
         var o = new OffscreenCanvas(10, 10).getContext('2d'); var og = o.createLinearGradient(0, 0, 10, 0); og.addColorStop(0, 'lime'); og.addColorStop(1, 'lime');\
         o.fillStyle = og; o.fillRect(0, 0, 10, 10); console.log(Array.from(o.getImageData(5, 5, 1, 1).data).join(','));");
    assert_eq!(l, ["log:#ff0000", "log:true", "log:true", "log:0,255,0,255"]);
}
