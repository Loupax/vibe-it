//! C-FLICKER: a pixel probe for live checks. `WEB_PROBE=x,y,w,h[;x,y,w,h...]` (logical
//! shell-window coordinates) makes the shell read back those rectangles of every frame it
//! renders and log `probe: frame N | min r,g,b max r,g,b | ...` (one entry per rectangle).
//! A rectangle over a solid fixed element of a scrolling page must stay that colour in every
//! frame; a frame where the page's buffer was sampled before the engine finished drawing it
//! (a missing or torn header) shows up as a different min/max. Reading back costs a GPU
//! round trip per frame, so it is for checks only (`test/c_flicker_live.py`).

/// The env var.
pub const ENV: &str = "WEB_PROBE";

/// A rectangle in logical shell-window coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// `x,y,w,h;x,y,w,h`; None when empty or malformed (the probe is then off).
pub fn parse(spec: &str) -> Option<Vec<Rect>> {
    let mut out = Vec::new();
    for part in spec.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let n: Vec<i32> = part.split(',').map(|v| v.trim().parse().ok()).collect::<Option<_>>()?;
        match n[..] {
            [x, y, w, h] if w > 0 && h > 0 && x >= 0 && y >= 0 => out.push(Rect { x, y, w, h }),
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The rectangle in GL framebuffer pixels (origin bottom left) for a framebuffer `fb_h`
/// pixels high at `scale`, clipped to `fb_w` x `fb_h`; None when nothing is left.
pub fn to_gl(r: Rect, scale: f64, fb_w: i32, fb_h: i32) -> Option<(i32, i32, i32, i32)> {
    let x0 = ((r.x as f64) * scale).round() as i32;
    let y0 = ((r.y as f64) * scale).round() as i32;
    let x1 = (((r.x + r.w) as f64) * scale).round().min(fb_w as f64) as i32;
    let y1 = (((r.y + r.h) as f64) * scale).round().min(fb_h as f64) as i32;
    (x1 > x0 && y1 > y0).then(|| (x0, fb_h - y1, x1 - x0, y1 - y0))
}

/// Per-channel min and max of RGBA bytes (alpha ignored).
pub fn min_max(rgba: &[u8]) -> ([u8; 3], [u8; 3]) {
    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    for px in rgba.as_chunks::<4>().0 {
        for c in 0..3 {
            lo[c] = lo[c].min(px[c]);
            hi[c] = hi[c].max(px[c]);
        }
    }
    (lo, hi)
}

/// One rectangle's entry in the log line.
pub fn entry(lo: [u8; 3], hi: [u8; 3]) -> String {
    format!("min {},{},{} max {},{},{}", lo[0], lo[1], lo[2], hi[0], hi[1], hi[2])
}
