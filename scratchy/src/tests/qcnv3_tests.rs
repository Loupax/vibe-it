// QCNV3: Canvas 2D leftovers: width/height parsing, brand checks, colour parsing and serialisation (lab(), color(),
// color-mix(), relative colours, currentcolor, system colours), registry ids, and the same colour syntax in CSS.
use super::*;

const CV: &str = "<html><body style='margin:0'><canvas id=c width=20 height=10></canvas><canvas id=d width=5 height=5></canvas></body></html>";

fn js(code: &str) -> Vec<String> {
    run_in_html(CV, &format!("var c = document.getElementById('c'); var x = c.getContext('2d'); function px(a, b) {{ return Array.from(x.getImageData(a, b, 1, 1).data).join(','); }} {code}"))
}

/// Pixel (0,0) of the canvas after filling with `style`, as `r,g,b,a`.
fn fill_px(style: &str) -> String {
    js(&format!("x.fillStyle = {style:?}; x.fillRect(0, 0, 1, 1); console.log(px(0, 0));")).remove(0)
}

fn near(got: &str, want: [i32; 4], tol: i32) {
    let v: Vec<i32> = got.trim_start_matches("log:").split(',').map(|n| n.parse().unwrap()).collect();
    assert!(v.iter().zip(want).all(|(a, b)| (a - b).abs() <= tol), "{got} vs {want:?}");
}

#[test]
fn qcnv3_size_attributes_follow_the_integer_parsing_rules() {
    let cases = [("100.999", 100), ("1e2", 1), ("0x10", 0), ("+100", 100), ("  100", 100), ("100px", 100), ("100%", 100), ("-5", 300), ("abc", 300), ("0", 0), ("99999999999", 300)];
    for (attr, want) in cases {
        let l = js(&format!("c.setAttribute('width', {attr:?}); console.log(c.width, c.getAttribute('width'));"));
        assert_eq!(l, [format!("log:{want} {attr}")], "width={attr:?}");
    }
    // The IDL setter is ToUint32; a non-number is 0; above 2^31-1 the default.
    let l = js("c.width = 200 - Math.pow(2, 32); c.height = 'foo'; console.log(c.width, c.height); c.width = 2147483648; console.log(c.width, c.getAttribute('width'));");
    assert_eq!(l, ["log:200 0", "log:300 300"]);
}

#[test]
fn qcnv3_computed_width_comes_from_the_parsed_attribute() {
    let doc = "<html><body><canvas id=c class=output width='+100' height='100.999'></canvas></body></html>";
    let l = run_in_html(doc, "var c = document.getElementById('c'); console.log(c.width, c.height, getComputedStyle(c, null).getPropertyValue('width'), getComputedStyle(c).height);");
    assert_eq!(l, ["log:100 100 100px 100px"]);
}

#[test]
fn qcnv3_size_attribute_sets_the_bitmap_size() {
    let doc = "<html><body><canvas id=c width='20.7' height=' +8x'></canvas></body></html>";
    let l = run_in_html(doc, "var x = document.getElementById('c').getContext('2d'); var d = x.getImageData(0, 0, 100, 100); console.log(x.canvas.width, x.canvas.height);");
    assert_eq!(l, ["log:20 8"]);
}

#[test]
fn qcnv3_interface_shape_and_brand_checks() {
    let l = js("console.log(Object.prototype.toString.call(c), Object.prototype.toString.call(x), Object.prototype.toString.call(x.createImageData(1, 1)));\
         var d = Object.getOwnPropertyDescriptor(CanvasRenderingContext2D, 'prototype'); console.log(d.writable, d.enumerable, d.configurable);\
         function t(f) { try { f(); return 'no'; } catch (e) { return e.name; } }\
         var P = CanvasRenderingContext2D.prototype;\
         console.log(t(function () { P.createImageData.call(null, 1, 1); }), t(function () { P.createImageData.call({}, 1, 1); }), t(function () { P.fillRect.call(c, 0, 0, 1, 1); }),\
         t(function () { P.getImageData.call(undefined, 0, 0, 1, 1); }), t(function () { Object.getOwnPropertyDescriptor(P, 'fillStyle').set.call({}, 'red'); }),\
         t(function () { Object.getOwnPropertyDescriptor(P, 'canvas').get.call(P); }));\
         var m = Object.getOwnPropertyDescriptor(P, 'fillRect'); console.log(m.writable, m.enumerable, m.configurable);");
    assert_eq!(
        l,
        ["log:[object HTMLCanvasElement] [object CanvasRenderingContext2D] [object ImageData]", "log:false false false", "log:TypeError TypeError TypeError TypeError TypeError TypeError", "log:true true true"]
    );
}

#[test]
fn qcnv3_image_data_edges() {
    let l = js("function t(f) { try { f(); return 'no'; } catch (e) { return e.name; } }\
         console.log(t(function () { x.createImageData(0.99, 10); }), t(function () { x.createImageData(10, 0.1); }), x.createImageData(-2.5, 3).width,\
         new ImageData(2, 2).pixelFormat, t(function () { new ImageData(new Uint8Array(100), 25); }), t(function () { new ImageData(null, 4, 4); }), t(function () { new ImageData(new Uint8Array(8), 1, 2); }),\
         t(function () { new ImageData(2, 2, { pixelFormat: 'unorm8' }); }));");
    assert_eq!(l, ["log:IndexSizeError IndexSizeError 2 rgba-unorm8 IndexSizeError TypeError TypeError TypeError"]);
}

#[test]
fn qcnv3_colour_serialisation() {
    let cases = [
        ("rgb(255, 0, 255)", "#ff00ff"),
        ("rgba(255, 0, 255, 1)", "#ff00ff"),
        ("#ff00ffff", "#ff00ff"),
        ("#ff00ff00", "rgba(255, 0, 255, 0)"),
        ("rgba(255, 0, 255,    0)", "rgba(255, 0, 255, 0)"),
        ("color(srgb 1 0 1)", "color(srgb 1 0 1)"),
        ("color(srgb 1 0 1 / 1)", "color(srgb 1 0 1)"),
        ("color(srgb 1 0 1/0)", "color(srgb 1 0 1 / 0)"),
        ("color(srgb 1 none 1)", "color(srgb 1 none 1)"),
        ("color(srgb none -1 2 / 3)", "color(srgb none -1 2)"),
        ("color(display-p3 0.96 0.76  \t 0.79)", "color(display-p3 0.96 0.76 0.79)"),
        ("lab(29% 39 20)", "lab(29 39 20)"),
        ("transparent", "rgba(0, 0, 0, 0)"),
        ("rgb(none none none)", "#000000"),
        ("rgb(300 none 400)", "#ff00ff"),
        ("hsl(120, 100%, 25%)", "#008000"),
        ("ThreeDDarkShadow", "#696969"),
    ];
    for (input, want) in cases {
        // fillStyle, strokeStyle and shadowColor share the parser and the serialisation.
        let l = js(&format!("['fillStyle', 'strokeStyle', 'shadowColor'].forEach(function (n) {{ x[n] = {input:?}; console.log(x[n]); }});"));
        assert_eq!(l, vec![format!("log:{want}"); 3], "{input}");
    }
    let l = js("console.log(x.shadowColor); x.save(); x.shadowColor = 'red'; x.restore(); console.log(x.shadowColor); x.fillStyle = 'lab(nope)'; console.log(x.fillStyle);");
    assert_eq!(l, ["log:rgba(0, 0, 0, 0)", "log:rgba(0, 0, 0, 0)", "log:#000000"]);
}

#[test]
fn qcnv3_colour_pixels() {
    // Readback of colours that need real conversion (the clamp to the sRGB gamut for out-of-range spaces).
    near(&fill_px("lab(50% 0 0)"), [119, 119, 119, 255], 1);
    near(&fill_px("oklch(0.5 0 0)"), [99, 99, 99, 255], 1);
    near(&fill_px("color(srgb 1 0 0)"), [255, 0, 0, 255], 0);
    near(&fill_px("color(display-p3 1 0 0)"), [255, 0, 0, 255], 0);
    near(&fill_px("color(srgb-linear 0.5 0.5 0.5)"), [188, 188, 188, 255], 1);
    near(&fill_px("color(srgb 0 1 0 / 0.5)"), [0, 255, 0, 128], 0);
}

#[test]
fn qcnv3_color_mix_and_relative_colours() {
    let cases = [
        ("color-mix(in srgb, red, blue)", "color(srgb 0.5 0 0.5)"),
        ("color-mix(in srgb, red, color(srgb 1 0 0))", "color(srgb 1 0 0)"),
        ("color-mix(in srgb, red 25%, blue)", "color(srgb 0.25 0 0.75)"),
        ("color-mix(in srgb, 25% red, blue 25%)", "color(srgb 0.5 0 0.5 / 0.5)"),
        ("color-mix(in srgb, rgb(255 0 0 / 0.5), blue)", "color(srgb 0.333333 0 0.666667 / 0.75)"),
        ("rgb(from red g r b)", "color(srgb 0 1 0)"),
        ("color(from color(srgb 0.25 0.5 0.75 / 0.5) srgb r g b / alpha)", "color(srgb 0.25 0.5 0.75 / 0.5)"),
        ("rgb(from rgb(10 20 30) calc(r) g b)", ""),
        ("color-mix(in nonsense, red, blue)", ""),
        ("color-mix(in srgb, red, blue, green)", ""),
    ];
    for (input, want) in cases {
        let l = js(&format!("x.fillStyle = '#123456'; x.fillStyle = {input:?}; console.log(x.fillStyle);"));
        let want = if want.is_empty() { "#123456" } else { want };
        assert_eq!(l, [format!("log:{want}")], "{input}");
    }
    // A polar space interpolates the hue the short way round; the result is still painted.
    let l = js("x.fillStyle = 'color-mix(in oklch, oklch(0.5 0.1 350), oklch(0.5 0.1 10))'; console.log(x.fillStyle.startsWith('oklch(0.5 0.1 '));");
    assert_eq!(l, ["log:true"]);
    near(&fill_px("color-mix(in srgb, red, blue)"), [128, 0, 128, 255], 0);
    near(&fill_px("rgb(from red g r b)"), [0, 255, 0, 255], 0);
}

#[test]
fn qcnv3_current_color_is_the_elements_computed_color() {
    let page = "<html><head><style>#s { color: #0f0 }</style></head><body style='color: #f00; margin: 0'><canvas id=c width=4 height=4></canvas><canvas id=s width=4 height=4></canvas><div style='color: #00f'><canvas id=i width=4 height=4></canvas></div></body></html>";
    let code = "function f(id, v) { var x = document.getElementById(id).getContext('2d'); x.fillStyle = '#123'; x.fillStyle = v; x.fillRect(0, 0, 4, 4); return Array.from(x.getImageData(0, 0, 1, 1).data).join(','); }\
         console.log(f('c', 'currentColor'), f('s', 'CURRENTCOLOR'), f('i', 'currentcolor'));\
         var c = document.getElementById('c'); var x = c.getContext('2d'); x.fillStyle = 'currentColor'; c.setAttribute('style', 'color: #0f0'); console.log(x.fillStyle);\
         x.fillStyle = 'currentColor'; console.log(x.fillStyle);\
         x.fillStyle = 'color-mix(in srgb, black, currentcolor)'; console.log(x.fillStyle);\
         x.fillStyle = 'rgb(from currentcolor g r b)'; console.log(x.fillStyle);\
         var d = document.createElement('canvas').getContext('2d'); d.fillStyle = '#f00'; d.fillStyle = 'currentColor'; console.log(d.fillStyle);";
    let l = run_in_html(page, code);
    assert_eq!(l, ["log:255,0,0,255 0,255,0,255 0,0,255,255", "log:#ff0000", "log:#00ff00", "log:color(srgb 0 0.5 0)", "log:color(srgb 1 0 0)", "log:#000000"]);
}

#[test]
fn qcnv3_fill_style_takes_the_string_form_of_objects() {
    let l = js("x.fillStyle = { toString: function () { return '#008000'; } }; console.log(x.fillStyle);\
         x.fillStyle = {}; console.log(x.fillStyle); x.fillStyle = 800000; console.log(x.fillStyle); x.fillStyle = null; console.log(x.fillStyle);\
         try { x.strokeStyle = { toString: function () { throw new RangeError('boom'); } }; console.log('no throw'); } catch (e) { console.log(e.name); }\
         x.fillStyle = [ 'blue' ]; console.log(x.fillStyle);");
    assert_eq!(l, ["log:#008000", "log:#008000", "log:#008000", "log:#008000", "log:RangeError", "log:#0000ff"]);
}

#[test]
fn qcnv3_canvases_are_independent_by_id() {
    let l = js("var d = document.getElementById('d').getContext('2d'); x.fillStyle = 'red'; x.fillRect(0, 0, 20, 10); d.fillStyle = 'blue'; d.fillRect(0, 0, 5, 5);\
         console.log(px(0, 0), Array.from(d.getImageData(0, 0, 1, 1).data).join(','));\
         var e = document.createElement('canvas'); e.width = 3; e.height = 3; var y = e.getContext('2d'); y.fillStyle = '#0f0'; y.fillRect(0, 0, 3, 3);\
         console.log(px(0, 0), Array.from(y.getImageData(0, 0, 1, 1).data).join(','), y.getImageData(0, 0, 9, 9).data.length);\
         e = null;");
    assert_eq!(l, ["log:255,0,0,255 0,0,255,255", "log:255,0,0,255 0,255,0,255 324"]);
}

#[test]
fn qcnv3_registry_replaces_a_canvas_at_the_same_node() {
    // A second getContext on the same element (after the first context object is dropped) never happens in JS, but
    // `create` for an address that already has a canvas must replace it, not leak the old id.
    let l = js("x.fillStyle = 'red'; x.fillRect(0, 0, 20, 10); console.log(px(1, 1), c.getContext('2d') === x);");
    assert_eq!(l, ["log:255,0,0,255 true"]);
}

fn span_color(style: &str) -> TextColor {
    let doc = parse_html(&format!("<html><body><span style='{style}'>x</span></body></html>"));
    let tree = build_layout_tree(&doc, 800.0);
    find_first_span_color(&tree).expect("no span found")
}

#[test]
fn qcnv3_css_colour_functions() {
    let near = |c: TextColor, want: [u8; 3], tol: i32| {
        assert!([(c.r(), want[0]), (c.g(), want[1]), (c.b(), want[2])].iter().all(|&(a, b)| (i32::from(a) - i32::from(b)).abs() <= tol), "{:?} vs {want:?}", (c.r(), c.g(), c.b()));
    };
    near(span_color("color: lab(50% 0 0)"), [119, 119, 119], 1);
    near(span_color("color: oklab(0.5 0 0)"), [99, 99, 99], 1);
    near(span_color("color: lch(50% 0 0)"), [119, 119, 119], 1);
    near(span_color("color: oklch(0.5 0 120)"), [99, 99, 99], 1);
    near(span_color("color: color(srgb 0 1 0)"), [0, 255, 0], 0);
    near(span_color("color: color(display-p3 0 1 0)"), [0, 255, 0], 0);
    near(span_color("color: color(srgb-linear 1 1 1)"), [255, 255, 255], 0);
    near(span_color("color: color(xyz-d65 0.9505 1 1.089)"), [255, 255, 255], 1);
    near(span_color("color: color-mix(in srgb, red, blue)"), [128, 0, 128], 0);
    near(span_color("color: color-mix(in srgb, red 75%, blue)"), [191, 0, 64], 0);
    near(span_color("color: color-mix(in lab, white, black)"), [119, 119, 119], 2);
    near(span_color("color: rgb(from blue b g r)"), [255, 0, 0], 0);
    near(span_color("color: hsl(from red h s calc(l))"), [0, 0, 0], 255); // calc() in relative colours is not supported: only no crash
    // Invalid values drop the declaration: the colour falls back to the inherited default (opaque black here).
    near(span_color("color: color-mix(in srgb, red)"), [0, 0, 0], 0);
    near(span_color("color: lab(nope)"), [0, 0, 0], 0);
    assert_eq!(span_color("color: color(srgb 1 0 0 / 0.5)").a(), 128);
}

#[test]
fn qcnv3_csscolor_conversions_round_trip() {
    use crate::csscolor::{convert, Col, Space};
    let red = Col { space: Space::Srgb, c: [Some(1.0), Some(0.0), Some(0.0)], a: Some(1.0) };
    for sp in [Space::SrgbLinear, Space::P3, Space::A98, Space::Rec2020, Space::ProPhoto, Space::Xyz50, Space::Xyz65, Space::Lab, Space::Lch, Space::Oklab, Space::Oklch, Space::Hsl, Space::Hwb] {
        let back = convert(&convert(&red, sp), Space::Srgb);
        let v: Vec<f64> = back.c.iter().map(|c| c.unwrap()).collect();
        assert!((v[0] - 1.0).abs() < 1e-4 && v[1].abs() < 1e-4 && v[2].abs() < 1e-4, "{sp:?}: {v:?}");
    }
    let lab = convert(&red, Space::Lab);
    assert!((lab.c[0].unwrap() - 54.29).abs() < 0.05 && (lab.c[1].unwrap() - 80.8).abs() < 0.1, "{lab:?}");
    let ok = convert(&red, Space::Oklab);
    assert!((ok.c[0].unwrap() - 0.6279).abs() < 1e-3 && (ok.c[1].unwrap() - 0.2249).abs() < 1e-3, "{ok:?}");
}
