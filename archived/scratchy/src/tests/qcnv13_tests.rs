// QCNV13: drawImage from canvas and OffscreenCanvas sources, imageSmoothingEnabled / imageSmoothingQuality.
use super::*;

const DOC: &str = "<html><body><canvas id=c width=100 height=50></canvas></body></html>";

fn js(code: &str) -> Vec<String> {
    run_in_html(
        DOC,
        &format!(
            "var cv = document.getElementById('c'); var x = cv.getContext('2d');\
             function px(a, b, c) {{ return Array.from((c || x).getImageData(a, b, 1, 1).data).join(','); }}\
             function mk(w, h, f) {{ var o = document.createElement('canvas'); o.width = w; o.height = h; var q = o.getContext('2d'); f(q); return o; }}\
             function quad() {{ return mk(20, 20, function (q) {{ q.fillStyle = '#f00'; q.fillRect(0, 0, 10, 10); q.fillStyle = '#0f0'; q.fillRect(10, 0, 10, 10);\
               q.fillStyle = '#00f'; q.fillRect(0, 10, 10, 10); q.fillStyle = '#ff0'; q.fillRect(10, 10, 10, 10); }}); }}\
             function err(f) {{ try {{ f(); return 'none'; }} catch (e) {{ return e.name; }} }}{code}"
        ),
    )
}

fn near(line: &str, want: &[[i32; 4]], tol: i32) {
    let got: Vec<Vec<i32>> = line.trim_start_matches("log:").split(' ').map(|p| p.split(',').map(|v| v.parse().unwrap()).collect()).collect();
    assert_eq!(got.len(), want.len(), "{line}");
    for (g, w) in got.iter().zip(want) {
        assert!(g.iter().zip(w).all(|(a, b)| (a - b).abs() <= tol), "{line}: want {want:?}");
    }
}

#[test]
fn qcnv13_three_five_and_nine_arguments() {
    let l = js("var q = quad();\
         x.drawImage(q, 30, 5); console.log(px(31, 6), px(45, 6), px(31, 20), px(45, 20), px(29, 6), px(50, 6));\
         x.clearRect(0, 0, 100, 50); x.drawImage(q, 0, 0, 40, 40); console.log(px(5, 5), px(30, 5), px(5, 30), px(30, 30), px(41, 5));\
         x.clearRect(0, 0, 100, 50); x.drawImage(q, 10, 0, 10, 10, 60, 10, 20, 20); console.log(px(61, 11), px(79, 29), px(59, 11), px(81, 11));");
    near(&l[0], &[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255], [0, 0, 0, 0], [0, 0, 0, 0]], 2);
    near(&l[1], &[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255], [0, 0, 0, 0]], 2);
    near(&l[2], &[[0, 255, 0, 255], [0, 255, 0, 255], [0, 0, 0, 0], [0, 0, 0, 0]], 2);
}

#[test]
fn qcnv13_negative_sizes_and_source_clipping() {
    let l = js("var q = quad();\
         x.drawImage(q, 20, 0, -20, 10, 0, 0, 20, 10); console.log(px(5, 5), px(15, 5));\
         x.clearRect(0, 0, 100, 50); x.drawImage(q, 0, 0, 20, 20, 40, 40, -20, -20); console.log(px(25, 25), px(35, 25), px(25, 35), px(35, 35));\
         x.clearRect(0, 0, 100, 50); x.drawImage(q, -10, -10, 20, 20, 0, 0, 40, 40); console.log(px(5, 5), px(25, 25), px(39, 39), px(41, 41));\
         x.clearRect(0, 0, 100, 50); x.drawImage(q, 10, 10, 100, 100, 0, 0, 100, 100); console.log(px(5, 5), px(15, 15), px(50, 50));\
         x.clearRect(0, 0, 100, 50); x.drawImage(q, 0, 0, 0, 10, 0, 0, 20, 10); x.drawImage(q, 0, 0, 10, 10, 0, 0, 0, 10); console.log(px(5, 5));");
    near(&l[0], &[[255, 0, 0, 255], [0, 255, 0, 255]], 2); // negative sizes only reposition the rect (no mirroring)
    near(&l[1], &[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255]], 2);
    // The source rect (-10,-10,20,20) is clipped to (0,0,10,10): the red quarter lands on the lower right 20x20 of the dest.
    near(&l[2], &[[0, 0, 0, 0], [255, 0, 0, 255], [255, 0, 0, 255], [0, 0, 0, 0]], 2);
    near(&l[3], &[[255, 255, 0, 255], [0, 0, 0, 0], [0, 0, 0, 0]], 2);
    near(&l[4], &[[0, 0, 0, 0]], 0);
}

#[test]
fn qcnv13_ctm_clip_alpha_and_operator() {
    let l = js("var q = quad();\
         x.translate(10, 10); x.scale(2, 2); x.drawImage(q, 0, 0); console.log(px(11, 11), px(45, 11), px(11, 45), px(45, 45), px(9, 11));\
         x.setTransform(1, 0, 0, 1, 0, 0); x.clearRect(0, 0, 100, 50);\
         x.save(); x.beginPath(); x.rect(0, 0, 5, 50); x.clip(); x.drawImage(q, 0, 0); x.restore(); console.log(px(2, 2), px(7, 2));\
         x.clearRect(0, 0, 100, 50); x.globalAlpha = 0.5; x.drawImage(q, 0, 0); console.log(px(2, 2)); x.globalAlpha = 1;\
         x.fillStyle = '#00f'; x.fillRect(0, 0, 100, 50); x.globalCompositeOperation = 'destination-over'; x.drawImage(q, 0, 0); console.log(px(2, 2), px(50, 2));\
         x.globalCompositeOperation = 'copy'; x.drawImage(q, 50, 0); console.log(px(2, 2), px(52, 2));");
    near(&l[0], &[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255], [0, 0, 0, 0]], 2);
    near(&l[1], &[[255, 0, 0, 255], [0, 0, 0, 0]], 0);
    near(&l[2], &[[255, 0, 0, 128]], 2);
    near(&l[3], &[[0, 0, 255, 255], [0, 0, 255, 255]], 0);
    // `copy` clears everything outside the drawn rect (within the clip), too.
    near(&l[4], &[[0, 0, 0, 0], [255, 0, 0, 255]], 2);
}

#[test]
fn qcnv13_shadow_of_a_canvas_source() {
    let l = js("var q = mk(10, 10, function (c) { c.fillStyle = '#0f0'; c.fillRect(0, 0, 10, 10); });\
         x.shadowColor = '#f00'; x.shadowOffsetX = 30; x.shadowOffsetY = 20; x.drawImage(q, 5, 5);\
         console.log(px(7, 7), px(37, 27), px(36, 20), px(46, 36));");
    near(&l[0], &[[0, 255, 0, 255], [255, 0, 0, 255], [0, 0, 0, 0], [0, 0, 0, 0]], 2);
}

#[test]
fn qcnv13_offscreen_sources_and_self_draw() {
    let l = js("var o = new OffscreenCanvas(10, 10); var oc = o.getContext('2d'); oc.fillStyle = '#0f0'; oc.fillRect(0, 0, 10, 10);\
         x.drawImage(o, 5, 5); console.log(px(6, 6), px(16, 6));\
         var o2 = new OffscreenCanvas(20, 20); var c2 = o2.getContext('2d'); c2.fillStyle = '#f00'; c2.fillRect(0, 0, 10, 20); c2.drawImage(o2, 10, 0); console.log(px(5, 5, c2), px(15, 5, c2));\
         x.fillStyle = '#0f0'; x.fillRect(0, 0, 50, 50); x.fillStyle = '#f00'; x.fillRect(50, 0, 50, 50); x.drawImage(cv, 50, 0); console.log(px(0, 0), px(99, 49));\
         x.clearRect(0, 0, 100, 50); x.drawImage(document.createElement('canvas'), 0, 0); console.log(px(5, 5));");
    near(&l[0], &[[0, 255, 0, 255], [0, 0, 0, 0]], 0);
    near(&l[1], &[[255, 0, 0, 255], [255, 0, 0, 255]], 0);
    near(&l[2], &[[0, 255, 0, 255], [0, 255, 0, 255]], 2);
    near(&l[3], &[[0, 0, 0, 0]], 0);
}

#[test]
fn qcnv13_argument_errors() {
    let l = js("var z = document.createElement('canvas'); z.width = 0; var zo = new OffscreenCanvas(5, 0);\
         console.log(err(function () { x.drawImage(null, 0, 0); }), err(function () { x.drawImage(undefined, 0, 0); }),\
           err(function () { x.drawImage(0, 0, 0); }), err(function () { x.drawImage('', 0, 0); }), err(function () { x.drawImage(document.createElement('p'), 0, 0); }),\
           err(function () { x.drawImage({}, 0, 0); }));\
         console.log(err(function () { x.drawImage(z, 0, 0); }), err(function () { x.drawImage(zo, 0, 0, 5, 5); }),\
           err(function () { x.drawImage(quad()); }), err(function () { x.drawImage(quad(), 1); }), err(function () { x.drawImage(quad(), 1, 2, 3, 4, 5, 6); }),\
           err(function () { x.drawImage(quad(), 0, 0, 1, 1, 0, 0, 1, 1); }));\
         x.drawImage(quad(), NaN, 0); x.drawImage(quad(), 0, Infinity, 5, 5); x.drawImage(quad(), 0, 0, 10, 10, 0, 0, NaN, 10); console.log(px(5, 5));\
         console.log(err(function () { x.drawImage(z, NaN, 0); }));\
         console.log(CanvasRenderingContext2D.prototype.drawImage.length);");
    assert_eq!(l[0], "log:TypeError TypeError TypeError TypeError TypeError TypeError");
    assert_eq!(l[1], "log:InvalidStateError InvalidStateError TypeError TypeError TypeError none");
    assert_eq!(l[2], "log:0,0,0,0");
    assert_eq!(l[3], "log:none");
    assert_eq!(l[4], "log:3");
}

#[test]
fn qcnv13_smoothing_attributes_and_pixels() {
    let l = js("console.log(x.imageSmoothingEnabled, x.imageSmoothingQuality);\
         x.imageSmoothingEnabled = 0; x.imageSmoothingQuality = 'high'; x.imageSmoothingQuality = 'nope'; x.imageSmoothingQuality = 'HIGH';\
         console.log(x.imageSmoothingEnabled, x.imageSmoothingQuality);\
         x.save(); x.imageSmoothingEnabled = true; x.imageSmoothingQuality = 'medium'; x.restore(); console.log(x.imageSmoothingEnabled, x.imageSmoothingQuality);\
         var o = new OffscreenCanvas(2, 2).getContext('2d'); o.imageSmoothingEnabled = false; console.log(o.imageSmoothingEnabled);\
         var s = mk(2, 1, function (q) { q.fillStyle = '#000'; q.fillRect(0, 0, 1, 1); q.fillStyle = '#fff'; q.fillRect(1, 0, 1, 1); });\
         x.imageSmoothingEnabled = false; x.drawImage(s, 0, 0, 2, 1, 0, 0, 20, 10); console.log(px(2, 5), px(7, 5), px(12, 5), px(17, 5));\
         x.imageSmoothingEnabled = true; x.clearRect(0, 0, 100, 50); x.drawImage(s, 0, 0, 2, 1, 0, 0, 20, 10); console.log(px(2, 5), px(10, 5), px(17, 5));");
    assert_eq!(l[0], "log:true low");
    assert_eq!(l[1], "log:false high");
    assert_eq!(l[2], "log:false high");
    assert_eq!(l[3], "log:false");
    // Nearest: hard edge at the middle; bilinear: a blend between the two pixel centres (5 and 15).
    near(&l[4], &[[0, 0, 0, 255], [0, 0, 0, 255], [255, 255, 255, 255], [255, 255, 255, 255]], 0);
    let v: Vec<i32> = l[5].trim_start_matches("log:").split(' ').map(|p| p.split(',').next().unwrap().parse().unwrap()).collect();
    assert!(v[0] < 30 && v[2] > 225 && v[1] > 60 && v[1] < 200, "{l:?}");
}
