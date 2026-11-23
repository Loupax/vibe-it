// QCNV11: globalCompositeOperation (Porter-Duff operators, blend modes, the unbounded operators, clip, globalAlpha).
use super::*;

const DOC: &str = "<html><body><canvas id=c width=100 height=50></canvas></body></html>";

fn js(code: &str) -> Vec<String> {
    run_in_html(
        DOC,
        &format!(
            "var x = document.getElementById('c').getContext('2d');\
             function px(a, b) {{ return Array.from(x.getImageData(a, b, 1, 1).data).join(','); }}{code}"
        ),
    )
}

/// Every pixel in the space-separated `log:` line `line` is within `tol` of `want` (one RGBA per pixel).
fn near(line: &str, want: &[[i32; 4]], tol: i32) {
    let got: Vec<Vec<i32>> = line.trim_start_matches("log:").split(' ').map(|p| p.split(',').map(|v| v.parse().unwrap()).collect()).collect();
    assert_eq!(got.len(), want.len(), "{line}");
    for (g, w) in got.iter().zip(want) {
        assert!(g.iter().zip(w).all(|(a, b)| (a - b).abs() <= tol), "{line}: want {want:?}");
    }
}

#[test]
fn qcnv11_attribute_values() {
    let l = js("console.log(x.globalCompositeOperation);\
         x.globalCompositeOperation = 'xor'; x.globalCompositeOperation = 'XOR'; x.globalCompositeOperation = 'darker';\
         x.globalCompositeOperation = 'source-over\\0'; x.globalCompositeOperation = 'over'; console.log(x.globalCompositeOperation);\
         x.globalCompositeOperation = 'clear'; console.log(x.globalCompositeOperation);\
         x.globalCompositeOperation = 'luminosity'; x.save(); x.globalCompositeOperation = 'copy'; console.log(x.globalCompositeOperation);\
         x.restore(); console.log(x.globalCompositeOperation);\
         var o = new OffscreenCanvas(10, 10).getContext('2d'); o.globalCompositeOperation = 'multiply'; console.log(o.globalCompositeOperation);");
    assert_eq!(l, ["log:source-over", "log:xor", "log:clear", "log:copy", "log:luminosity", "log:multiply"]);
}

#[test]
fn qcnv11_porter_duff_solid() {
    // Backdrop cyan (opaque), source yellow (opaque) over the whole canvas: the WPT 2d.composite.solid.* values.
    let l = js("var ops = ['source-over', 'source-in', 'source-out', 'source-atop', 'destination-over', 'destination-in',\
         'destination-out', 'destination-atop', 'lighter', 'copy', 'xor', 'clear'];\
         ops.forEach(function (op) { x.globalCompositeOperation = 'source-over'; x.clearRect(0, 0, 100, 50);\
           x.fillStyle = 'rgba(0, 255, 255, 1)'; x.fillRect(0, 0, 100, 50);\
           x.globalCompositeOperation = op; x.fillStyle = 'rgba(255, 255, 0, 1)'; x.fillRect(0, 0, 100, 50);\
           console.log(px(50, 25)); });");
    let want = [
        [255, 255, 0, 255],
        [255, 255, 0, 255],
        [0, 0, 0, 0],
        [255, 255, 0, 255],
        [0, 255, 255, 255],
        [0, 255, 255, 255],
        [0, 0, 0, 0],
        [0, 255, 255, 255],
        [255, 255, 255, 255],
        [255, 255, 0, 255],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
    ];
    assert_eq!(l.len(), want.len());
    for (line, w) in l.iter().zip(&want) {
        near(line, &[*w], 2);
    }
}

#[test]
fn qcnv11_porter_duff_transparent() {
    // 2d.composite.transparent.*: backdrop rgba(0,255,0,0.5), source rgba(0,0,255,0.75).
    let l = js("['source-atop', 'destination-over', 'xor', 'destination-atop', 'source-in'].forEach(function (op) {\
           x.globalCompositeOperation = 'source-over'; x.clearRect(0, 0, 100, 50);\
           x.fillStyle = 'rgba(0, 255, 0, 0.5)'; x.fillRect(0, 0, 100, 50);\
           x.globalCompositeOperation = op; x.fillStyle = 'rgba(0, 0, 255, 0.75)'; x.fillRect(0, 0, 100, 50);\
           console.log(px(50, 25)); });");
    let want = [[0, 64, 191, 128], [0, 146, 109, 223], [0, 64, 191, 128], [0, 128, 128, 191], [0, 0, 255, 96]];
    for (line, w) in l.iter().zip(&want) {
        near(line, &[*w], 3);
    }
}

#[test]
fn qcnv11_unbounded_operators_clear_outside_the_shape() {
    // The shape covers only the left half; the right half is cleared by the unbounded operators, kept by the others.
    let l = js("['source-in', 'source-out', 'destination-in', 'destination-atop', 'copy', 'source-atop', 'xor', 'multiply'].forEach(function (op) {\
           x.globalCompositeOperation = 'source-over'; x.clearRect(0, 0, 100, 50);\
           x.fillStyle = '#0f0'; x.fillRect(0, 0, 100, 50);\
           x.globalCompositeOperation = op; x.fillStyle = '#f00'; x.fillRect(0, 0, 50, 50);\
           console.log(px(75, 25)); });");
    let want = [[0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0], [0, 255, 0, 255], [0, 255, 0, 255], [0, 255, 0, 255]];
    for (line, w) in l.iter().zip(&want) {
        near(line, &[*w], 0);
    }
}

#[test]
fn qcnv11_clip_bounds_the_composite() {
    // 2d.composite.clip.*: nothing outside the clip changes, even for copy/source-in/clear.
    let l = js("['copy', 'source-in', 'destination-in', 'clear', 'xor'].forEach(function (op) {\
           x.save(); x.globalCompositeOperation = 'source-over'; x.fillStyle = '#0f0'; x.fillRect(0, 0, 100, 50);\
           x.globalCompositeOperation = op; x.beginPath(); x.rect(-20, -20, 10, 10); x.clip();\
           x.fillStyle = '#f00'; x.fillRect(0, 0, 50, 50); x.restore(); console.log(px(25, 25), px(75, 25)); });\
         x.fillStyle = '#0f0'; x.fillRect(0, 0, 100, 50); x.save(); x.beginPath(); x.rect(0, 0, 50, 50); x.clip();\
         x.globalCompositeOperation = 'source-in'; x.fillStyle = '#00f'; x.fillRect(0, 0, 10, 10); x.restore();\
         console.log(px(5, 5), px(25, 25), px(75, 25));");
    for line in &l[..5] {
        near(line, &[[0, 255, 0, 255], [0, 255, 0, 255]], 0);
    }
    near(&l[5], &[[0, 0, 255, 255], [0, 0, 0, 0], [0, 255, 0, 255]], 0);
}

#[test]
fn qcnv11_global_alpha_and_copy() {
    let l = js("x.fillStyle = '#0f0'; x.fillRect(0, 0, 100, 50); x.globalAlpha = 0.5; x.globalCompositeOperation = 'copy';\
         x.fillStyle = '#f00'; x.fillRect(0, 0, 50, 50); console.log(px(25, 25), px(75, 25));\
         x.globalAlpha = 1; x.globalCompositeOperation = 'source-over'; x.fillStyle = '#0f0'; x.fillRect(0, 0, 100, 50);\
         x.globalAlpha = 0.5; x.globalCompositeOperation = 'lighter'; x.fillStyle = '#f00'; x.fillRect(0, 0, 100, 50);\
         console.log(px(50, 25));");
    near(&l[0], &[[255, 0, 0, 128], [0, 0, 0, 0]], 1);
    near(&l[1], &[[128, 255, 0, 255]], 1);
}

#[test]
fn qcnv11_blend_modes() {
    // Opaque backdrop (255,128,0), opaque source (128,128,128): the W3C separable and non-separable formulas.
    let l = js("['multiply', 'screen', 'darken', 'lighten', 'difference', 'exclusion', 'overlay', 'hard-light', 'luminosity', 'color'].forEach(function (op) {\
           x.globalCompositeOperation = 'source-over'; x.fillStyle = 'rgb(255, 128, 0)'; x.fillRect(0, 0, 100, 50);\
           x.globalCompositeOperation = op; x.fillStyle = 'rgb(128, 128, 128)'; x.fillRect(0, 0, 50, 50);\
           console.log(px(25, 25), px(75, 25)); });");
    let b = [255, 128, 0, 255];
    let want = [
        [128, 64, 0, 255],    // multiply
        [255, 192, 128, 255], // screen
        [128, 128, 0, 255],   // darken
        [255, 128, 128, 255], // lighten
        [127, 0, 128, 255],   // difference
        [127, 128, 128, 255], // exclusion
        [255, 128, 0, 255],   // overlay: hard-light(cs, cb) with cs ~0.5
        [255, 128, 0, 255],   // hard-light ~ screen/multiply at 0.5
        [215, 108, 0, 255],   // luminosity: SetLum(cb, Lum(cs)), then ClipColor
        [152, 152, 152, 255], // color: SetLum(gray, Lum(cb) = 0.596)
    ];
    assert_eq!(l.len(), want.len());
    for (line, w) in l.iter().zip(&want) {
        near(line, &[*w, b], 3);
    }
}

#[test]
fn qcnv11_strokes_paths_gradients_and_clear_rect() {
    let l = js("x.fillStyle = '#0f0'; x.fillRect(0, 0, 100, 50); x.globalCompositeOperation = 'source-in';\
         x.strokeStyle = '#00f'; x.lineWidth = 10; x.beginPath(); x.moveTo(0, 25); x.lineTo(100, 25); x.stroke();\
         console.log(px(50, 25), px(50, 5));\
         x.globalCompositeOperation = 'source-over'; x.fillRect(0, 0, 100, 50);\
         x.globalCompositeOperation = 'destination-out'; x.beginPath(); x.rect(0, 0, 50, 50); x.fill();\
         console.log(px(25, 25), px(75, 25));\
         x.globalCompositeOperation = 'source-over'; x.fillRect(0, 0, 100, 50);\
         var g = x.createLinearGradient(0, 0, 100, 0); g.addColorStop(0, '#f00'); g.addColorStop(1, '#f00');\
         x.globalCompositeOperation = 'destination-in'; x.fillStyle = g; x.fillRect(0, 0, 50, 50);\
         console.log(px(25, 25), px(75, 25));\
         x.globalCompositeOperation = 'source-over'; x.fillStyle = '#0f0'; x.fillRect(0, 0, 100, 50);\
         x.globalCompositeOperation = 'copy'; x.clearRect(0, 0, 10, 10); console.log(px(5, 5), px(50, 25));\
         x.globalCompositeOperation = 'copy'; x.strokeStyle = '#f00'; x.lineWidth = 2; x.strokeRect(10, 10, 20, 20);\
         console.log(px(10, 10), px(20, 20), px(80, 40));");
    near(&l[0], &[[0, 0, 255, 255], [0, 0, 0, 0]], 0);
    near(&l[1], &[[0, 0, 0, 0], [0, 255, 0, 255]], 0);
    near(&l[2], &[[0, 255, 0, 255], [0, 0, 0, 0]], 0);
    near(&l[3], &[[0, 0, 0, 0], [0, 255, 0, 255]], 0);
    near(&l[4], &[[255, 0, 0, 255], [0, 0, 0, 0], [0, 0, 0, 0]], 0);
}

#[test]
fn qcnv11_offscreen_canvas() {
    let l = js("var o = new OffscreenCanvas(20, 20).getContext('2d'); o.fillStyle = '#0f0'; o.fillRect(0, 0, 20, 20);\
         o.globalCompositeOperation = 'destination-atop'; o.fillStyle = '#f00'; o.fillRect(0, 0, 10, 20);\
         console.log(Array.from(o.getImageData(5, 5, 1, 1).data).join(','), Array.from(o.getImageData(15, 5, 1, 1).data).join(','));");
    near(&l[0], &[[0, 255, 0, 255], [0, 0, 0, 0]], 0);
}
