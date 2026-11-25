// QGEOM1: Geometry Interfaces (DOMPoint, DOMRect, DOMQuad, DOMMatrix) and canvas getTransform/setTransform.
use super::*;

fn js(code: &str) -> Vec<String> {
    run_in_html("<html><body><canvas id=c width=20 height=20></canvas><div id=d></div></body></html>", code)
}

#[test]
fn qgeom1_point_and_rect() {
    let l = js("var p = new DOMPoint(1, 2); p.z = 3; console.log(JSON.stringify(p), p.w, Object.prototype.toString.call(p));\
        var r = new DOMRect(10, 20, -5, 4); console.log(r.left, r.right, r.top, r.bottom, JSON.stringify(DOMRectReadOnly.fromRect({width: 3})));\
        var ro = new DOMPointReadOnly(1); try { ro.x = 5; console.log(ro.x); } catch (e) { console.log('throw'); }\
        console.log(document.getElementById('d').getBoundingClientRect() instanceof DOMRect);\
        try { DOMPoint(); } catch (e) { console.log(e.name); }");
    assert_eq!(l[0], "log:{\"x\":1,\"y\":2,\"z\":3,\"w\":1} 1 [object DOMPoint]");
    assert_eq!(l[1], "log:5 10 20 24 {\"x\":0,\"y\":0,\"width\":3,\"height\":0,\"top\":0,\"right\":3,\"bottom\":0,\"left\":0}");
    assert_eq!(l[2], "log:1");
    assert_eq!(l[3], "log:true");
    assert_eq!(l[4], "log:TypeError");
}

#[test]
fn qgeom1_matrix_construct_and_ops() {
    let l = js("var m = new DOMMatrix(); console.log(m.is2D, m.isIdentity, String(m));\
        m = new DOMMatrix([1, 2, 3, 4, 5, 6]); console.log(String(m), m.a, m.m21, m.m42);\
        console.log(String(new DOMMatrix('translate(10px, 20px) scale(2)')), new DOMMatrix('translate3d(1px,2px,0px)').is2D);\
        var t = new DOMMatrix().translate(5, 6).scale(2); console.log(String(t), t.isIdentity);\
        var p = t.transformPoint({x: 1, y: 1}); console.log(p.x, p.y, p.z, p.w);\
        console.log(String(t.inverse().multiply(t)));\
        var s = new DOMMatrix([1, 0, 0, 1, 0, 0]); console.log(s.translateSelf(3, 4) === s, s.e, s.f);\
        var inv = new DOMMatrix([0, 0, 0, 0, 0, 0]).invertSelf(); console.log(inv.is2D, isNaN(inv.a));\
        try { String(inv); } catch (e) { console.log(e.name); }\
        try { new DOMMatrix('foo'); } catch (e) { console.log(e.name); }\
        try { new DOMMatrix([1, 2, 3]); } catch (e) { console.log(e.name); }\
        console.log(String(DOMMatrix.fromFloat64Array(new Float64Array([1,0,0,1,7,8]))), new DOMMatrix([1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16]).toString().slice(0, 8));\
        console.log(new DOMMatrix().skewX(45).rotate(0).c.toFixed(6), String(new DOMMatrix().flipY()), new DOMMatrix().toFloat32Array().length);\
        var r = new DOMMatrix().rotate(90); console.log(r.a.toFixed(3), r.b);\
        console.log(new DOMMatrix().rotate(10, 20, 30).is2D, new DOMMatrix().rotate(30).is2D);\
        try { DOMMatrix.fromMatrix({a: 1, m11: 2}); } catch (e) { console.log(e.name); }\
        try { DOMMatrix.fromMatrix({is2D: true, m13: 1}); } catch (e) { console.log(e.name); }\
        console.log(DOMMatrix.fromMatrix({m33: 2}).is2D, DOMMatrix.fromMatrix({a: 3}).m11);");
    let want = [
        "log:true true matrix(1, 0, 0, 1, 0, 0)",
        "log:matrix(1, 2, 3, 4, 5, 6) 1 3 6",
        "log:matrix(2, 0, 0, 2, 10, 20) false",
        "log:matrix(2, 0, 0, 2, 5, 6) false",
        "log:7 8 0 1",
        "log:matrix(1, 0, 0, 1, 0, 0)",
        "log:true 3 4",
        "log:false true",
        "log:InvalidStateError",
        "log:SyntaxError",
        "log:TypeError",
        "log:matrix(1, 0, 0, 1, 7, 8) matrix3d",
        "log:1.000000 matrix(1, 0, 0, -1, 0, 0) 16",
        "log:0.000 1",
        "log:false true",
        "log:TypeError",
        "log:TypeError",
        "log:false 3",
    ];
    assert_eq!(l, want);
}

#[test]
fn qgeom1_quad() {
    let l = js("var q = DOMQuad.fromRect({x: 1, y: 2, width: 3, height: 4}); var b = q.getBounds();\
        console.log(b.x, b.y, b.width, b.height, q.p3.x, q.p3.y, q.p1 === q.p1);\
        console.log(JSON.stringify(new DOMQuad({x: 1}).toJSON().p1));\
        console.log(new DOMPoint(2, 3).matrixTransform(new DOMMatrix().scale(2)).x);");
    assert_eq!(l, ["log:1 2 3 4 4 6 true", "log:{\"x\":1,\"y\":0,\"z\":0,\"w\":1}", "log:4"]);
}

#[test]
fn qgeom1_canvas_transform_is_dommatrix() {
    let l = js("var x = document.getElementById('c').getContext('2d'); x.translate(3, 4);\
        var t = x.getTransform(); console.log(t instanceof DOMMatrix, t.e, t.f, t.is2D);\
        t.e = 100; console.log(x.getTransform().e);\
        x.setTransform(new DOMMatrix().scale(2)); console.log(String(x.getTransform()));\
        x.setTransform({a: 1, m11: 1, e: 9}); console.log(x.getTransform().e);\
        try { x.setTransform({a: 1, m11: 2}); } catch (e) { console.log(e.name); }\
        x.setTransform(new DOMMatrix('translate3d(5px,6px,7px)')); console.log(x.getTransform().e);\
        x.setTransform(1, 0, 0, 1, NaN, 0); console.log(x.getTransform().e);");
    assert_eq!(l, ["log:true 3 4 true", "log:3", "log:matrix(2, 0, 0, 2, 0, 0)", "log:9", "log:TypeError", "log:5", "log:5"]);
}

#[test]
fn qgeom1_inverse_3d_round_trips() {
    let l = js("var a = DOMMatrix.fromMatrix({m11:1,m12:-0.5,m13:0.5,m21:0.5,m22:2,m23:-0.5,m33:1,m41:10,m42:20,m43:10,is2D:false});\
        var r = a.multiply(a.inverse()); console.log(JSON.stringify(Array.from(r.toFloat64Array()).map(function (v) { return Math.round(v * 1e9) / 1e9; })), r.isIdentity, JSON.stringify(Array.from(r.toFloat64Array())));");
    assert_eq!(l, ["log:[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1] true [1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]"]);
}
