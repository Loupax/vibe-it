// QCNV9: Path2D (constructors, path methods, addPath), fill/stroke/clip(path), isPointInPath/isPointInStroke.
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
fn qcnv9_constructors() {
    let l = js("var p = new Path2D(); var q = new Path2D(p); var r = new Path2D('M 10 10 L 30 10 L 30 30 Z');\
         console.log(Object.prototype.toString.call(p), typeof Path2D, Path2D.length);\
         try { Path2D(); } catch (e) { console.log(e.name); }\
         try { p.moveTo.call({}, 1, 2); } catch (e) { console.log(e.name); }\
         x.fill(r); console.log(px(25, 15), px(12, 28));\
         x.clearRect(0, 0, 60, 60); x.fill(new Path2D(r)); console.log(px(25, 15));\
         x.clearRect(0, 0, 60, 60); x.fill(new Path2D('garbage')); x.fill(new Path2D('L 10 10 L 20 20')); console.log(px(15, 12));\
         x.clearRect(0, 0, 60, 60); x.fill(new Path2D('M 10 10 h 20 v 20 h -20 z')); console.log(px(15, 15));\
         x.clearRect(0, 0, 60, 60); x.fill(new Path2D('M 10 10 h 20 v 20 h -20 z M 5 5 L oops')); console.log(px(15, 15));");
    assert_eq!(
        l,
        [
            "log:[object Path2D] function 0".to_string(),
            "log:TypeError".into(),
            "log:TypeError".into(),
            format!("log:{BLACK} {CLEAR}"),
            format!("log:{BLACK}"),
            format!("log:{CLEAR}"),
            format!("log:{BLACK}"),
            format!("log:{BLACK}"),
        ]
    );
}

#[test]
fn qcnv9_path_methods_and_ctm_at_use() {
    let l = js("var p = new Path2D(); p.rect(10, 10, 20, 20); x.translate(20, 20); x.fill(p);\
         console.log(px(15, 15), px(35, 35), px(55, 55));\
         x.setTransform(1, 0, 0, 1, 0, 0); x.clearRect(0, 0, 60, 60);\
         var e = new Path2D(); e.arc(30, 30, 10, 0, 7); e.closePath(); x.fill(e); console.log(px(30, 30), px(5, 5));\
         try { e.arc(0, 0, -1, 0, 1); } catch (er) { console.log(er.name); }\
         var r = new Path2D(); r.roundRect(5, 5, 20, 20, [4]); x.clearRect(0, 0, 60, 60); x.fill(r); console.log(px(5, 5), px(15, 15));\
         var n = new Path2D(); n.moveTo(NaN, 1); n.lineTo(1, Infinity); x.clearRect(0, 0, 60, 60); x.stroke(n); console.log(px(1, 1));");
    assert_eq!(
        l,
        [
            format!("log:{CLEAR} {BLACK} {CLEAR}"),
            format!("log:{BLACK} {CLEAR}"),
            "log:IndexSizeError".to_string(),
            format!("log:{CLEAR} {BLACK}"),
            format!("log:{CLEAR}"),
        ]
    );
}

#[test]
fn qcnv9_add_path() {
    let l = js("var a = new Path2D(); a.rect(0, 0, 10, 10); var b = new Path2D(); b.addPath(a, { e: 20, f: 20 }); x.fill(b);\
         console.log(px(25, 25), px(5, 5));\
         b.addPath(a); x.clearRect(0, 0, 60, 60); x.fill(b); console.log(px(5, 5), px(25, 25));\
         try { b.addPath(a, { a: 1, m11: 2 }); } catch (e) { console.log(e.name); }\
         try { b.addPath({}); } catch (e) { console.log(e.name); }\
         var c = new Path2D(); c.addPath(a, new DOMMatrix([2, 0, 0, 2, 0, 0])); x.clearRect(0, 0, 60, 60); x.fill(c); console.log(px(15, 15));\
         var d = new Path2D(); d.addPath(a, { a: NaN }); x.clearRect(0, 0, 60, 60); x.fill(d); console.log(px(5, 5));\
         var s = new Path2D(); s.rect(0, 0, 4, 4); s.addPath(s, { e: 10 }); x.clearRect(0, 0, 60, 60); x.fill(s); console.log(px(12, 2));");
    assert_eq!(
        l,
        [
            format!("log:{BLACK} {CLEAR}"),
            format!("log:{BLACK} {BLACK}"),
            "log:TypeError".to_string(),
            "log:TypeError".into(),
            format!("log:{BLACK}"),
            format!("log:{CLEAR}"),
            format!("log:{BLACK}"),
        ]
    );
}

#[test]
fn qcnv9_fill_stroke_clip_overloads() {
    let l = js("var p = new Path2D(); p.rect(10, 10, 30, 30); p.rect(20, 20, 10, 10);\
         x.fill(p, 'evenodd'); console.log(px(25, 25), px(12, 12));\
         x.clearRect(0, 0, 60, 60); x.fill(p); console.log(px(25, 25));\
         x.clearRect(0, 0, 60, 60); x.fill('evenodd'); console.log(px(25, 25));\
         try { x.fill(p, 'bogus'); } catch (e) { console.log(e.name); }\
         try { x.fill({}, 'evenodd'); } catch (e) { console.log(e.name); }\
         try { x.stroke({}); } catch (e) { console.log(e.name); }\
         x.beginPath(); x.rect(0, 0, 5, 5); x.lineWidth = 4; x.stroke(p); console.log(px(10, 25), px(2, 2));\
         x.clearRect(0, 0, 60, 60); x.save(); x.clip(p); x.fillRect(0, 0, 60, 60); x.restore(); console.log(px(25, 25), px(5, 5));\
         x.clearRect(0, 0, 60, 60); x.save(); x.beginPath(); x.rect(0, 0, 5, 5); x.clip(p, 'evenodd'); x.fillRect(0, 0, 60, 60); x.restore(); console.log(px(25, 25), px(12, 12));");
    assert_eq!(
        l,
        [
            format!("log:{CLEAR} {BLACK}"),
            format!("log:{BLACK}"),
            format!("log:{CLEAR}"),
            "log:TypeError".to_string(),
            "log:TypeError".into(),
            "log:TypeError".into(),
            format!("log:{BLACK} {CLEAR}"),
            format!("log:{BLACK} {CLEAR}"),
            format!("log:{CLEAR} {BLACK}"),
        ]
    );
}

#[test]
fn qcnv9_is_point_in_path() {
    let l = js("x.beginPath(); x.rect(10, 10, 20, 20); x.rect(15, 15, 10, 10);\
         console.log(x.isPointInPath(20, 20), x.isPointInPath(20, 20, 'evenodd'), x.isPointInPath(12, 12), x.isPointInPath(5, 5));\
         console.log(x.isPointInPath(10, 10), x.isPointInPath(30, 20), x.isPointInPath(NaN, 1), x.isPointInPath(1, Infinity));\
         x.translate(100, 0); console.log(x.isPointInPath(20, 20), x.isPointInPath(-80, 20));\
         x.setTransform(1, 0, 0, 1, 0, 0);\
         var p = new Path2D(); p.rect(0, 0, 10, 10); x.translate(30, 30);\
         console.log(x.isPointInPath(p, 5, 5), x.isPointInPath(p, 35, 35), x.isPointInPath(p, 5, 5, 'evenodd'));\
         try { x.isPointInPath(p, 1); } catch (e) { console.log(e.name); }\
         try { x.isPointInPath(1); } catch (e) { console.log(e.name); }\
         try { x.isPointInPath(p, 1, 2, 'bad'); } catch (e) { console.log(e.name); }\
         var c = new Path2D(); c.arc(30, 30, 10, 0, 7); x.setTransform(1, 0, 0, 1, 0, 0); console.log(x.isPointInPath(c, 30, 30), x.isPointInPath(c, 45, 30));");
    assert_eq!(
        l,
        [
            "log:true false true false",
            "log:true true false false",
            "log:true false",
            "log:false true false",
            "log:TypeError",
            "log:TypeError",
            "log:TypeError",
            "log:true false",
        ]
    );
}

#[test]
fn qcnv9_is_point_in_stroke() {
    let l = js("x.lineWidth = 10; x.beginPath(); x.moveTo(10, 30); x.lineTo(50, 30);\
         console.log(x.isPointInStroke(30, 33), x.isPointInStroke(30, 36), x.isPointInStroke(52, 30), x.isPointInStroke(NaN, 30));\
         x.lineCap = 'round'; console.log(x.isPointInStroke(53, 30), x.isPointInStroke(56, 30));\
         x.lineCap = 'butt'; x.setLineDash([5, 5]); console.log(x.isPointInStroke(12, 30), x.isPointInStroke(17, 30));\
         x.setLineDash([]); x.scale(2, 1); console.log(x.isPointInStroke(15, 34), x.isPointInStroke(15, 36));\
         var p = new Path2D(); p.moveTo(0, 0); p.lineTo(10, 0); x.setTransform(1, 0, 0, 1, 0, 0); x.lineWidth = 2;\
         console.log(x.isPointInStroke(p, 5, 0.5), x.isPointInStroke(p, 5, 3));\
         try { x.isPointInStroke(1); } catch (e) { console.log(e.name); }");
    assert_eq!(
        l,
        [
            "log:true false false false",
            "log:true false",
            "log:true false",
            "log:true false",
            "log:true false",
            "log:TypeError",
        ]
    );
}
