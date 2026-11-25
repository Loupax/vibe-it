// QCNV1: Canvas 2D foundation: getContext, the tiny-skia bitmap, the first API slice (pixel readback through
// getImageData), paint of the bitmap and the damage a draw records.
use super::*;

const CV: &str = "<html><body style='margin:0'><canvas id=c width=20 height=10></canvas></body></html>";

fn js(code: &str) -> Vec<String> {
    run_in_html(CV, &format!("var c = document.getElementById('c'); var x = c.getContext('2d'); function px(a, b) {{ return Array.from(x.getImageData(a, b, 1, 1).data).join(','); }} {code}"))
}

#[test]
fn qcnv1_get_context_types_and_identity() {
    let l = js("console.log(c.getContext('2d') === x, x.canvas === c, c.getContext('webgl'), c.getContext('webgl2'), c.getContext('bitmaprenderer'), c.getContext('2D'), x instanceof CanvasRenderingContext2D);");
    assert_eq!(l, ["log:true true null null null null true"]);
}

#[test]
fn qcnv1_fill_clear_stroke_and_alpha() {
    let l = js("console.log(px(0,0));\
         x.fillStyle = 'rgb(10, 20, 30)'; x.fillRect(2, 2, 4, 4); console.log(px(3,3), px(1,1), px(6,6));\
         x.clearRect(3, 3, 1, 1); console.log(px(3,3), px(4,4));\
         x.globalAlpha = 0.5; x.fillStyle = '#ff0000'; x.fillRect(10, 0, 2, 2); console.log(px(10,0));\
         x.globalAlpha = 1; x.strokeStyle = '#00f'; x.lineWidth = 2; x.strokeRect(8, 4, 8, 4); console.log(px(8,4), px(12,4));\
         x.fillRect(-5, -5, 100, 100); x.fillRect(0, 0, 0, 5); console.log(px(19, 9));");
    assert_eq!(l, ["log:0,0,0,0", "log:10,20,30,255 0,0,0,0 0,0,0,0", "log:0,0,0,0 10,20,30,255", "log:255,0,0,128", "log:0,0,255,255 0,0,255,255", "log:255,0,0,255"]);
}

#[test]
fn qcnv1_style_serialisation_and_invalid_values() {
    let l = js("console.log(x.fillStyle, x.strokeStyle); x.fillStyle = 'red'; console.log(x.fillStyle); x.fillStyle = 'not a colour'; console.log(x.fillStyle);\
         x.fillStyle = 'rgba(0, 128, 255, 0.5)'; console.log(x.fillStyle); x.fillStyle = 'hsl(120, 100%, 25%)'; console.log(x.fillStyle);\
         x.globalAlpha = 2; console.log(x.globalAlpha); x.globalAlpha = NaN; console.log(x.globalAlpha); x.globalAlpha = 0.25; console.log(x.globalAlpha);");
    assert_eq!(l, ["log:#000000 #000000", "log:#ff0000", "log:#ff0000", "log:rgba(0, 128, 255, 0.5)", "log:#008000", "log:1", "log:1", "log:0.25"]);
}

#[test]
fn qcnv1_save_restore() {
    let l = js("x.fillStyle = '#123456'; x.globalAlpha = 0.5; x.save(); x.fillStyle = 'red'; x.globalAlpha = 1; x.save(); x.fillStyle = 'blue';\
         x.restore(); console.log(x.fillStyle, x.globalAlpha); x.restore(); console.log(x.fillStyle, x.globalAlpha); x.restore(); console.log(x.fillStyle, x.globalAlpha);");
    assert_eq!(l, ["log:#ff0000 1", "log:#123456 0.5", "log:#123456 0.5"]);
}

#[test]
fn qcnv1_width_height_reset_bitmap_and_state() {
    let l = js("x.fillStyle = 'red'; x.fillRect(0, 0, 20, 10); console.log(px(5,5)); c.width = 20; console.log(px(5,5), x.fillStyle, c.getContext('2d') === x);\
         x.fillRect(0, 0, 5, 5); console.log(px(1,1)); c.height = 4; console.log(px(1,1), x.getImageData(0,0,20,10).data.length, c.height);\
         c.width = 'abc'; console.log(c.width);");
    assert_eq!(l, ["log:255,0,0,255", "log:0,0,0,0 #000000 true", "log:0,0,0,255", "log:0,0,0,0 800 4", "log:0"]);
}

#[test]
fn qcnv1_image_data_roundtrip() {
    let l = js("var d = x.createImageData(2, 2); console.log(d.width, d.height, d.data.length, d.data instanceof Uint8ClampedArray, d.colorSpace, x.createImageData(-3, 2).width, x.createImageData(d).height);\
         d.data.set([255,0,0,255, 0,255,0,255, 0,0,255,255, 1,2,3,0]); x.putImageData(d, 4, 4); console.log(px(4,4), px(5,4), px(4,5), px(5,5));\
         x.putImageData(d, 0, 0, 0, 1, 1, 1); console.log(px(0,1), px(0,0));\
         var g = x.getImageData(18, 8, 4, 4); console.log(g.width, g.height, Array.from(g.data.slice(0, 4)).join(), g.data.length);\
         var i = new ImageData(new Uint8ClampedArray(8), 2); console.log(i.width, i.height);\
         x.fillStyle = 'rgba(255,0,0,0.5)'; x.fillRect(15, 0, 1, 1); x.putImageData(d, 15, 0); console.log(px(15, 0));");
    assert_eq!(l, ["log:2 2 16 true srgb 3 2", "log:255,0,0,255 0,255,0,255 0,0,255,255 0,0,0,0", "log:0,0,255,255 0,0,0,0", "log:4 4 0,0,0,0 64", "log:2 1", "log:255,0,0,255"]);
}

#[test]
fn qcnv1_errors() {
    let l = js("function t(f) { try { f(); return 'no'; } catch (e) { return e.name; } }\
         console.log(t(function () { x.getImageData(0, 0, 0, 1); }), t(function () { x.getImageData(0, 0, NaN, 1); }), t(function () { x.createImageData(0, 1); }),\
         t(function () { new ImageData(0, 1); }), t(function () { x.putImageData({}, 0, 0); }), t(function () { x.fillRect(1); }), t(function () { c.getContext(); }));");
    assert_eq!(l, ["log:IndexSizeError TypeError IndexSizeError IndexSizeError TypeError TypeError TypeError"]);
}

#[test]
fn qcnv1_paint_shows_the_bitmap() {
    let doc = parse_html("<html><body style='margin:0'><canvas id=c width=20 height=10></canvas></body></html>");
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    run_scripts_in(
        &doc,
        &[src("var x = document.getElementById('c').getContext('2d'); x.fillStyle = '#336699'; x.fillRect(0, 0, 10, 10); x.fillStyle = 'rgba(255,0,0,0.5)'; x.fillRect(10, 0, 10, 10);")],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
    );
    assert!(out.borrow().is_empty(), "{:?}", out.borrow());
    let tree = build_layout_tree(&doc, 200.0);
    let mut fs = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut incomplete) = (0usize, false);
    let laid = measure_document(&tree, &mut fs, 200.0, 200.0, &mut next_id, &mut cache, std::time::Instant::now() + std::time::Duration::from_secs(5), &mut incomplete);
    let mut swash = SwashCache::new();
    let mut pixels = vec![0u32; 200 * 200];
    paint(&laid, &mut fs, &mut swash, &mut cache, &HashMap::new(), &mut pixels, 200, 200, 0.0, None);
    assert_eq!(px_at(&pixels, 200, 3, 3), 0x336699);
    // half-transparent red over the (unpainted, 0) page buffer
    let p = px_at(&pixels, 200, 15, 5);
    assert!((p >> 16) & 0xFF >= 0x7F && (p >> 16) & 0xFF <= 0x80, "{p:#x}");
    // outside the 20 x 10 box nothing is painted
    assert_eq!(px_at(&pixels, 200, 30, 5), 0);
    // the draws recorded the canvas as dirty: the damage is that box only, no relayout needed
    let node = node_id(&find_by_tag(&doc, "canvas").unwrap());
    let dirty = vec![node];
    assert!(crate::canvas::pixel(node, 3, 3).is_some());
    let rects = crate::incr_layout::canvas_damage(&laid, &dirty);
    assert!(rects.is_some_and(|r| r.iter().any(|k| (k.r - k.l - 20.0).abs() < 0.5 && (k.b - k.t - 10.0).abs() < 0.5)));
}
