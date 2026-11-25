// QCNV12: canvas shadows (shadowColor, shadowBlur, shadowOffsetX/Y; drawn before the shape with the operator).
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
fn qcnv12_attributes() {
    let l = js("console.log(x.shadowBlur, x.shadowOffsetX, x.shadowOffsetY);\
         x.shadowBlur = 1; x.shadowBlur = -2; x.shadowBlur = Infinity; x.shadowBlur = NaN; x.shadowBlur = 'no'; console.log(x.shadowBlur);\
         x.shadowBlur = true; console.log(x.shadowBlur); x.shadowBlur = false; console.log(x.shadowBlur);\
         x.shadowOffsetX = -3.5; x.shadowOffsetY = 2; x.shadowOffsetX = -Infinity; x.shadowOffsetY = NaN; console.log(x.shadowOffsetX, x.shadowOffsetY);\
         x.save(); x.shadowBlur = 7; x.shadowOffsetX = 9; x.restore(); console.log(x.shadowBlur, x.shadowOffsetX);\
         var o = new OffscreenCanvas(5, 5).getContext('2d'); o.shadowOffsetY = 4; console.log(o.shadowOffsetY);");
    assert_eq!(l, ["log:0 0 0", "log:1", "log:1", "log:0", "log:-3.5 2", "log:0 -3.5", "log:4"]);
}

#[test]
fn qcnv12_offset_shadow_from_outside_the_canvas_and_global_alpha() {
    // 2d.shadow.alpha.3: the shape is above the canvas, only its shadow lands on it, at half alpha.
    let l = js("x.fillStyle = '#f00'; x.fillRect(0, 0, 100, 50); x.shadowColor = '#00f'; x.shadowOffsetY = 50;\
         x.globalAlpha = 0.5; x.fillRect(0, -50, 100, 50); console.log(px(50, 25));\
         x.globalAlpha = 1; x.shadowColor = 'rgba(0, 0, 0, 0)'; x.fillStyle = '#0f0'; x.fillRect(0, -50, 100, 50); console.log(px(50, 25));");
    near(&l[0], &[[127, 0, 127, 255]], 2);
    near(&l[1], &[[127, 0, 127, 255]], 2);
}

#[test]
fn qcnv12_offsets_ignore_the_ctm_and_clip_applies() {
    // 2d.shadow.transform.2 and 2d.shadow.clip.2.
    let l = js("x.fillStyle = '#f00'; x.fillRect(0, 0, 100, 50); x.shadowOffsetY = 50; x.shadowColor = '#0f0';\
         x.rotate(Math.PI); x.fillRect(-100, 0, 100, 50); console.log(px(50, 25));\
         x.setTransform(1, 0, 0, 1, 0, 0); x.shadowOffsetY = 0; x.shadowColor = 'transparent';\
         x.fillStyle = '#f00'; x.fillRect(0, 0, 50, 50); x.fillStyle = '#0f0'; x.fillRect(50, 0, 50, 50);\
         x.save(); x.beginPath(); x.rect(0, 0, 50, 50); x.clip(); x.shadowColor = '#f00'; x.shadowOffsetX = 50;\
         x.fillStyle = '#0f0'; x.fillRect(0, 0, 50, 50); x.restore(); console.log(px(25, 25), px(75, 25));");
    near(&l[0], &[[0, 255, 0, 255]], 0);
    near(&l[1], &[[0, 255, 0, 255], [0, 255, 0, 255]], 0);
}

#[test]
fn qcnv12_blur_profile() {
    // A shadow covering x < 50 (shape at x -100..-50, offset 100), sigma = 4: opaque inside, ~half at the edge, ~0 far out.
    let l = js("x.shadowColor = '#000'; x.shadowOffsetX = 100; x.shadowBlur = 8; x.fillRect(-150, -50, 100, 150);\
         console.log(px(25, 25), px(50, 25), px(75, 25));");
    let a: Vec<i32> = l[0].trim_start_matches("log:").split(' ').map(|p| p.rsplit(',').next().unwrap().parse().unwrap()).collect();
    assert!(a[0] >= 250, "{l:?}");
    assert!((a[1] - 128).abs() <= 16, "{l:?}");
    assert!(a[2] <= 3, "{l:?}");
}

#[test]
fn qcnv12_shadow_uses_the_composite_operator() {
    // 2d.shadow.composite.1/.2 (xor), .3 (destination-out, shadow far away), 2d.shadow.enable.blur (destination-atop).
    let l = js("x.fillStyle = '#f00'; x.fillRect(0, 0, 100, 50); x.globalCompositeOperation = 'xor'; x.shadowColor = '#f00';\
         x.shadowOffsetX = 100; x.fillStyle = '#0f0'; x.fillRect(-100, 0, 200, 50); console.log(px(50, 25));\
         x.globalCompositeOperation = 'source-over'; x.shadowOffsetX = 0; x.shadowColor = 'transparent'; x.clearRect(0, 0, 100, 50);\
         x.fillStyle = '#f00'; x.fillRect(0, 0, 100, 50); x.globalCompositeOperation = 'xor'; x.shadowColor = '#f00'; x.shadowBlur = 1;\
         x.fillStyle = '#0f0'; x.fillRect(-10, -10, 120, 70); console.log(px(50, 25));\
         x.globalCompositeOperation = 'source-over'; x.shadowBlur = 0; x.shadowColor = 'transparent'; x.clearRect(0, 0, 100, 50);\
         x.globalCompositeOperation = 'destination-atop'; x.shadowColor = '#0f0'; x.shadowBlur = 0.1; x.fillStyle = '#f00';\
         x.fillRect(0, 0, 100, 50); console.log(px(50, 25));");
    near(&l[0], &[[0, 255, 0, 255]], 2);
    near(&l[1], &[[0, 255, 0, 255]], 2);
    near(&l[2], &[[0, 255, 0, 255]], 0);
}

#[test]
fn qcnv12_strokes_paths_gradients_and_offscreen() {
    let l = js("x.shadowColor = '#00f'; x.shadowOffsetY = 25; x.strokeStyle = '#f00'; x.lineWidth = 10;\
         x.beginPath(); x.moveTo(0, 10); x.lineTo(100, 10); x.stroke(); console.log(px(50, 10), px(50, 35), px(50, 22));\
         x.clearRect(0, 0, 100, 50); x.shadowOffsetY = 0; x.shadowOffsetX = 50;\
         var g = x.createLinearGradient(0, 0, 50, 0); g.addColorStop(0, '#f00'); g.addColorStop(1, '#ff0');\
         x.fillStyle = g; x.beginPath(); x.rect(-50, 0, 50, 50); x.fill(); console.log(px(25, 25));\
         x.clearRect(0, 0, 100, 50); x.fillStyle = 'rgba(255, 0, 0, 0.5)'; x.shadowColor = '#0f0'; x.fillRect(-50, 0, 50, 50);\
         console.log(px(25, 25));\
         var o = new OffscreenCanvas(20, 20).getContext('2d'); o.shadowColor = '#0f0'; o.shadowOffsetX = 10; o.fillRect(0, 0, 10, 20);\
         console.log(Array.from(o.getImageData(15, 5, 1, 1).data).join(','));");
    near(&l[0], &[[255, 0, 0, 255], [0, 0, 255, 255], [0, 0, 0, 0]], 0);
    near(&l[1], &[[0, 0, 255, 255]], 0);
    // The shadow takes the shape's alpha (0.5) times the shadow colour.
    near(&l[2], &[[0, 255, 0, 128]], 1);
    near(&l[3], &[[0, 255, 0, 255]], 0);
}
