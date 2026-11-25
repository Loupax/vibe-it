// QGEOM2: DOMRectList (getClientRects) and structured clone of the geometry classes.
#[allow(unused_imports)]
use super::*;

fn logs_of(log: &[String]) -> Vec<&str> {
    log.iter().filter_map(|l| l.strip_prefix("log:")).collect()
}

#[test]
fn qgeom2_rect_list_is_not_an_array() {
    let l = run_in_html(
        "<html><body><div id=x>x</div><div id=n style='display:none'></div></body></html>",
        "var d = document.getElementById('x').getClientRects();\
         console.log('DOMRectList' in window, d instanceof Array, Array.isArray(d), d instanceof DOMRectList,\
           Object.prototype.toString.call(d), d.length, d[1], d[-1], d.item(-1), d.item(1), d.item(0) === d[0]);\
         console.log(Object.prototype.toString.call(d[0]), [...d].length, Array.from(d).length, Object.keys(d).join());\
         d[0] = 5; console.log(d[0] instanceof DOMRect);\
         try { new DOMRectList(); } catch (e) { console.log(e.name); }\
         try { DOMRectList.prototype.item.call({}, 0); } catch (e) { console.log(e.name); }\
         console.log(document.getElementById('n').getClientRects().length);\
         var r = document.createRange(); r.selectNodeContents(document.getElementById('x'));\
         console.log(r.getClientRects() instanceof DOMRectList);",
    );
    assert_eq!(l[0], "log:true false false true [object DOMRectList] 1 undefined undefined null null true", "{l:?}");
    assert_eq!(l[1], "log:[object DOMRect] 1 1 0");
    assert_eq!(l[2], "log:true");
    assert_eq!(l[3], "log:TypeError");
    assert_eq!(l[4], "log:TypeError");
    assert_eq!(l[5], "log:0");
    assert_eq!(l[6], "log:true");
}

#[test]
fn qgeom2_structured_clone_geometry() {
    let l = run_in_html(
        "<html><body><div id=x>x</div></body></html>",
        "var p = new DOMPoint(1, -0, Infinity, NaN); p.foo = 1; var c = structuredClone(p);\
         console.log(c !== p, Object.prototype.toString.call(c), c.x, Object.is(c.y, -0), c.z, c.w, 'foo' in c);\
         var ro = structuredClone(new DOMPointReadOnly(1, 2)); console.log(Object.prototype.toString.call(ro));\
         try { ro.x = 9; } catch (e) {} console.log(ro.x);\
         var r = structuredClone(new DOMRect(1, 2, 3, 4)); console.log(Object.prototype.toString.call(r), r.right, r.bottom);\
         r.x = 7; console.log(r.x);\
         var q = new DOMQuad({x: 1}, {x: 2}, {x: 3}, {x: 4}); var qc = structuredClone(q);\
         console.log(qc.p3.x, qc.p3 !== q.p3, Object.prototype.toString.call(qc.p1));\
         var m = new DOMMatrix([1, -0, Infinity, NaN, 5, 6]); m.m13 = -0; var mc = structuredClone(m);\
         console.log(mc.is2D, Object.is(mc.m13, 0), mc.e, mc.f, mc.c, Object.prototype.toString.call(mc));\
         var m3 = structuredClone(new DOMMatrixReadOnly([1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16])); console.log(m3.is2D, m3.m44, Object.prototype.toString.call(m3));\
         var nested = structuredClone({a: [new DOMRect(1, 1, 1, 1)]}); console.log(nested.a[0].width);\
         try { structuredClone(document.getElementById('x').getClientRects()); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(l[0], "log:true [object DOMPoint] 1 true Infinity NaN false", "{l:?}");
    assert_eq!(l[1], "log:[object DOMPointReadOnly]");
    assert_eq!(l[2], "log:1");
    assert_eq!(l[3], "log:[object DOMRect] 4 6");
    assert_eq!(l[4], "log:7");
    assert_eq!(l[5], "log:3 true [object DOMPoint]");
    assert_eq!(l[6], "log:true true 5 6 Infinity [object DOMMatrix]");
    assert_eq!(l[7], "log:false 16 [object DOMMatrixReadOnly]");
    assert_eq!(l[8], "log:1");
    assert_eq!(l[9], "log:DataCloneError");
}

#[test]
fn qgeom2_post_message_clones_geometry() {
    crate::js::set_page_url(Url::parse("https://qgeom2.test/p.html").ok().as_ref());
    let log = run_with_timers(
        "var ch = new MessageChannel();\
         ch.port2.onmessage = function (e) { var d = e.data; console.log(Object.prototype.toString.call(d.r), d.r.width, Object.prototype.toString.call(d.m), d.m.e, d.q.p2.x); };\
         ch.port1.postMessage({r: new DOMRect(0, 0, 9, 1), m: new DOMMatrix().translate(4, 5), q: new DOMQuad({x: 1}, {x: 2})});\
         try { ch.port1.postMessage(document.body.getClientRects()); } catch (e) { console.log('list', e.name); }\
         window.addEventListener('message', function (e) { console.log('win', Object.prototype.toString.call(e.data), e.data.x); });\
         window.postMessage(new DOMPoint(3, 4), '*');",
    );
    let l = logs_of(&log);
    assert!(l.contains(&"list DataCloneError"), "{log:?}");
    assert!(l.contains(&"[object DOMRect] 9 [object DOMMatrix] 4 2"), "{log:?}");
    assert!(l.contains(&"win [object DOMPoint] 3"), "{log:?}");
}
