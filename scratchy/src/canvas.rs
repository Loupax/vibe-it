//! QCNV1: the Canvas 2D backing store and its first API slice.
//!
//! Each `<canvas>` that asked for a `'2d'` context owns one [`Canvas`] here, keyed by an id allocated at `getContext`
//! (QCNV3; `BY_NODE` maps the element's node address to it for paint and damage only): a
//! tiny-skia `Pixmap` (premultiplied RGBA8) of the element's `width` x `height`, the drawing state (colours,
//! `globalAlpha`, `lineWidth`, the save/restore stack) and a cached straight-alpha `DecodedImage` of the bitmap for
//! paint. The JS side (`js_dom.rs`, the `CanvasRenderingContext2D` object) is thin: it validates arguments and
//! calls the `__cv_*` natives registered by [`register`].
//!
//! Paint: a canvas box is a replaced box whose content is an empty-pixel `DecodedImage` carrying [`MARK`] in
//! `svg` (`images::build_canvas_box`); `paint` calls [`image`] for it and blits the bitmap. A draw does not touch
//! the DOM: it only records the node in the dirty set ([`mark`]), which the App drains once per redraw
//! ([`take_dirty`]) and turns into a damage rect of that box (`incr_layout::canvas_damage`), so the redraw repaints
//! without a relayout and, on the QHOV1 damage path, only that rect.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tiny_skia::{BlendMode, Color, FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, Rect, Stroke, StrokeDash, Transform};

use crate::js_dom::DomTable;
use crate::js_engine::{native, Bytes, Ctx};
use crate::layout::DecodedImage;

/// `DecodedImage::svg` of a canvas placeholder (never a real SVG: placeholders have no pixels).
pub(crate) const MARK: &[u8] = b"scratchy:canvas";
/// Bitmaps above this many pixels are not allocated (the context then draws nothing, as when allocation fails).
const MAX_PIXELS: u64 = 1 << 26;
/// The default size of a canvas element without `width`/`height` attributes.
/// `Canvas::node` of an OffscreenCanvas bitmap (QCNV4): never a node address, so it is not in `by_node` and never dirty.
const NO_NODE: usize = 0;
pub(crate) const DEFAULT_W: u32 = 300;
pub(crate) const DEFAULT_H: u32 = 150;

pub(crate) fn is_placeholder(img: &DecodedImage) -> bool {
    img.svg.as_ref().is_some_and(|s| s.as_slice() == MARK)
}

/// A colour attribute: the straight RGBA to paint with and what the getter returns.
#[derive(Clone)]
pub(crate) struct Style {
    rgba: [u8; 4],
    ser: String,
}

impl Style {
    fn new(rgba: [u8; 4]) -> Style {
        Style { rgba, ser: serialize_color(rgba) }
    }
}

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone)]
struct DrawState {
    fill: Style,
    stroke: Style,
    shadow: Style,
    alpha: f32,
    line_width: f32,
    /// QCNV8: 0 butt, 1 round, 2 square.
    cap: u8,
    /// QCNV8: 0 miter, 1 round, 2 bevel.
    join: u8,
    miter_limit: f32,
    /// QCNV8: the dash list as set (already even length), and the offset.
    dash: Vec<f64>,
    dash_offset: f64,
    /// QCNV5: the current transformation matrix as (a, b, c, d, e, f): x' = a x + c y + e, y' = b x + d y + f.
    ctm: [f64; 6],
    /// QCNV6: the clipping region (coverage 0..255 per pixel); `None` = no clip. Shared between saved copies.
    clip: Option<Arc<Mask>>,
}

/// QCNV6: one segment of the current default path, in DEVICE space (points are mapped by the CTM when added).
/// QCNV7 adds `Quad`/`Cubic` here (arcs and ellipses become cubics) and `build` below turns them into tiny-skia calls.
#[derive(Clone, Copy, Debug)]
enum Seg {
    Move(f64, f64),
    Line(f64, f64),
    /// QCNV7: quadratic curve (control, end).
    Quad(f64, f64, f64, f64),
    /// QCNV7: cubic curve (control 1, control 2, end).
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

/// The current default path: segments plus whether it has a subpath yet.
#[derive(Default)]
struct PathData {
    segs: Vec<Seg>,
}

impl PathData {
    /// The point `(x, y)` after the CTM.
    fn map(m: &[f64; 6], x: f64, y: f64) -> (f64, f64) {
        (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
    }
    fn move_to(&mut self, m: &[f64; 6], x: f64, y: f64) {
        let (x, y) = Self::map(m, x, y);
        self.segs.push(Seg::Move(x, y));
    }
    /// lineTo: with no subpath yet it only starts one at the point.
    fn line_to(&mut self, m: &[f64; 6], x: f64, y: f64) {
        let (x, y) = Self::map(m, x, y);
        self.segs.push(if self.segs.is_empty() { Seg::Move(x, y) } else { Seg::Line(x, y) });
    }
    /// The start point of the last subpath.
    fn start(&self) -> Option<(f64, f64)> {
        self.segs.iter().rev().find_map(|s| if let Seg::Move(x, y) = s { Some((*x, *y)) } else { None })
    }
    /// closePath: closes the last subpath; the next segment starts again at its first point.
    fn close(&mut self) {
        let Some(start) = self.start() else { return };
        if matches!(self.segs.last(), Some(Seg::Close)) {
            return;
        }
        self.segs.push(Seg::Close);
        self.segs.push(Seg::Move(start.0, start.1));
    }
    /// rect: a closed subpath of four corners (even with zero size), then a new subpath at (x, y).
    fn rect(&mut self, m: &[f64; 6], x: f64, y: f64, w: f64, h: f64) {
        self.move_to(m, x, y);
        self.line_to(m, x + w, y);
        self.line_to(m, x + w, y + h);
        self.line_to(m, x, y + h);
        self.close();
    }
    /// The tiny-skia path in device space (None when it has no drawable segment).
    fn build(&self) -> Option<tiny_skia::Path> {
        let mut pb = PathBuilder::new();
        for s in &self.segs {
            match *s {
                Seg::Move(x, y) => pb.move_to(x as f32, y as f32),
                Seg::Line(x, y) => pb.line_to(x as f32, y as f32),
                Seg::Quad(a, b, x, y) => pb.quad_to(a as f32, b as f32, x as f32, y as f32),
                Seg::Cubic(a, b, c, d, x, y) => pb.cubic_to(a as f32, b as f32, c as f32, d as f32, x as f32, y as f32),
                Seg::Close => pb.close(),
            }
        }
        pb.finish()
    }
    /// QCNV8: the path for stroking: zero-length segments are removed ("prune") so a round/square cap does not
    /// draw a dot for them; a subpath left with no segment draws nothing (its close is dropped too).
    fn build_pruned(&self) -> Option<tiny_skia::Path> {
        let mut pb = PathBuilder::new();
        let mut cur = (f64::NAN, f64::NAN);
        let mut start = cur;
        let mut drawn = false;
        for s in &self.segs {
            let (kept, end) = match *s {
                Seg::Move(x, y) => {
                    cur = (x, y);
                    start = cur;
                    drawn = false;
                    continue;
                }
                Seg::Close => {
                    if drawn {
                        pb.close();
                    }
                    cur = start;
                    drawn = false;
                    continue;
                }
                Seg::Line(x, y) => ((x, y) != cur, (x, y)),
                Seg::Quad(a, b, x, y) => ((a, b) != cur || (x, y) != cur, (x, y)),
                Seg::Cubic(a, b, c, d, x, y) => ((a, b) != cur || (c, d) != cur || (x, y) != cur, (x, y)),
            };
            if kept {
                if !drawn {
                    pb.move_to(cur.0 as f32, cur.1 as f32);
                    drawn = true;
                }
                match *s {
                    Seg::Line(x, y) => pb.line_to(x as f32, y as f32),
                    Seg::Quad(a, b, x, y) => pb.quad_to(a as f32, b as f32, x as f32, y as f32),
                    Seg::Cubic(a, b, c, d, x, y) => pb.cubic_to(a as f32, b as f32, c as f32, d as f32, x as f32, y as f32),
                    _ => {}
                }
            }
            cur = end;
        }
        pb.finish()
    }
    /// The current point in device space (the end of the last segment).
    fn cur(&self) -> Option<(f64, f64)> {
        match *self.segs.last()? {
            Seg::Move(x, y) | Seg::Line(x, y) | Seg::Quad(_, _, x, y) | Seg::Cubic(_, _, _, _, x, y) => Some((x, y)),
            Seg::Close => self.start(),
        }
    }
    /// "Ensure there is a subpath for (x, y)": starts one at the point when the path is empty.
    fn ensure(&mut self, m: &[f64; 6], x: f64, y: f64) {
        if self.segs.is_empty() {
            self.move_to(m, x, y);
        }
    }
    fn quad(&mut self, m: &[f64; 6], cx: f64, cy: f64, x: f64, y: f64) {
        self.ensure(m, cx, cy);
        let (a, b) = Self::map(m, cx, cy);
        let (x, y) = Self::map(m, x, y);
        self.segs.push(Seg::Quad(a, b, x, y));
    }
    fn cubic(&mut self, m: &[f64; 6], c1: (f64, f64), c2: (f64, f64), p: (f64, f64)) {
        self.ensure(m, c1.0, c1.1);
        let (a, b) = Self::map(m, c1.0, c1.1);
        let (c, d) = Self::map(m, c2.0, c2.1);
        let (x, y) = Self::map(m, p.0, p.1);
        self.segs.push(Seg::Cubic(a, b, c, d, x, y));
    }
    /// ellipse()/arc(): a line to the arc's start point (or a new subpath there), then cubics up to the end point.
    /// All arguments are finite and the radii non-negative (the JS side checks). Angles in radians.
    #[allow(clippy::too_many_arguments)]
    fn ellipse(&mut self, m: &[f64; 6], x: f64, y: f64, rx: f64, ry: f64, rot: f64, start: f64, end: f64, ccw: bool) {
        use std::f64::consts::PI;
        let (sr, cr) = rot.sin_cos();
        let pt = |t: f64| {
            let (px, py) = (rx * t.cos(), ry * t.sin());
            (x + cr * px - sr * py, y + sr * px + cr * py)
        };
        let sweep = if !ccw {
            if end - start >= 2.0 * PI { 2.0 * PI } else { (end - start).rem_euclid(2.0 * PI) }
        } else if start - end >= 2.0 * PI {
            -2.0 * PI
        } else {
            -(start - end).rem_euclid(2.0 * PI)
        };
        let p0 = pt(start);
        self.line_to(m, p0.0, p0.1);
        if rx == 0.0 || ry == 0.0 {
            let e = pt(start + sweep);
            self.line_to(m, e.0, e.1);
            return;
        }
        let n = ((sweep.abs() / (PI / 2.0)) - 1e-9).ceil().max(1.0);
        let d = sweep / n;
        let k = 4.0 / 3.0 * (d / 4.0).tan();
        let deriv = |t: f64| {
            let (px, py) = (-rx * t.sin(), ry * t.cos());
            (cr * px - sr * py, sr * px + cr * py)
        };
        for i in 0..n as usize {
            let (t0, t1) = (start + d * i as f64, start + d * (i + 1) as f64);
            let (a, b) = (pt(t0), pt(t1));
            let (da, db) = (deriv(t0), deriv(t1));
            let end_pt = if i + 1 == n as usize { pt(start + sweep) } else { b };
            self.cubic(m, (a.0 + k * da.0, a.1 + k * da.1), (b.0 - k * db.0, b.1 - k * db.1), end_pt);
        }
    }
    /// arcTo(x1, y1, x2, y2, r), computed in user space from the current point.
    fn arc_to(&mut self, m: &[f64; 6], x1: f64, y1: f64, x2: f64, y2: f64, r: f64) {
        self.ensure(m, x1, y1);
        let (Some(inv), Some(cur)) = (invert(m), self.cur()) else {
            self.line_to(m, x1, y1);
            return;
        };
        let (x0, y0) = Self::map(&inv, cur.0, cur.1);
        let (v0x, v0y, v2x, v2y) = (x0 - x1, y0 - y1, x2 - x1, y2 - y1);
        let (l0, l2) = (v0x.hypot(v0y), v2x.hypot(v2y));
        let cross = v0x * v2y - v0y * v2x;
        if l0 == 0.0 || l2 == 0.0 || r == 0.0 || cross.abs() <= 1e-14 * l0 * l2 {
            self.line_to(m, x1, y1);
            return;
        }
        let (u0x, u0y, u2x, u2y) = (v0x / l0, v0y / l0, v2x / l2, v2y / l2);
        let cos = (u0x * u2x + u0y * u2y).clamp(-1.0, 1.0);
        let half = cos.acos() / 2.0;
        let t = r / half.tan();
        let (bx, by) = (u0x + u2x, u0y + u2y);
        let bl = bx.hypot(by);
        let dist = r / half.sin();
        let (cx, cy) = (x1 + bx / bl * dist, y1 + by / bl * dist);
        let (t1x, t1y, t2x, t2y) = (x1 + u0x * t, y1 + u0y * t, x1 + u2x * t, y1 + u2y * t);
        if ![cx, cy, t1x, t1y, t2x, t2y].iter().all(|v| v.is_finite()) {
            self.line_to(m, x1, y1);
            return;
        }
        let a1 = (t1y - cy).atan2(t1x - cx);
        let a2 = (t2y - cy).atan2(t2x - cx);
        let mut d = a2 - a1;
        while d > std::f64::consts::PI {
            d -= 2.0 * std::f64::consts::PI;
        }
        while d < -std::f64::consts::PI {
            d += 2.0 * std::f64::consts::PI;
        }
        self.line_to(m, t1x, t1y);
        self.ellipse(m, cx, cy, r, r, 0.0, a1, a1 + d, d < 0.0);
        // Land exactly on the second tangent point.
        if let Some(Seg::Cubic(.., ex, ey)) = self.segs.last_mut() {
            let (dx, dy) = Self::map(m, t2x, t2y);
            *ex = dx;
            *ey = dy;
        }
    }
    /// roundRect: `r` = [ul.x, ul.y, ur.x, ur.y, lr.x, lr.y, ll.x, ll.y], radii relative to the (x, y) corner.
    fn round_rect(&mut self, m: &[f64; 6], mut x: f64, mut y: f64, mut w: f64, mut h: f64, r: &[f64]) {
        use std::f64::consts::PI;
        let mut q = [[r[0], r[1]], [r[2], r[3]], [r[4], r[5]], [r[6], r[7]]];
        // Exactly one negative size mirrors the rectangle, so its subpath runs the other way round (nonzero fills cancel).
        let reversed = (w < 0.0) != (h < 0.0);
        if w < 0.0 {
            x += w;
            w = -w;
            q.swap(0, 1);
            q.swap(2, 3);
        }
        if h < 0.0 {
            y += h;
            h = -h;
            q.swap(0, 3);
            q.swap(1, 2);
        }
        let sums = [(w, q[0][0] + q[1][0]), (h, q[1][1] + q[2][1]), (w, q[2][0] + q[3][0]), (h, q[0][1] + q[3][1])];
        let mut scale = 1.0f64;
        for (len, sum) in sums {
            if sum > 0.0 {
                scale = scale.min(len / sum);
            }
        }
        if scale < 1.0 {
            for c in q.iter_mut() {
                c[0] *= scale;
                c[1] *= scale;
            }
        }
        let [ul, ur, lr, ll] = q;
        self.move_to(m, x + ul[0], y);
        if reversed {
            self.ellipse(m, x + ul[0], y + ul[1], ul[0], ul[1], 0.0, 1.5 * PI, PI, true);
            self.line_to(m, x, y + h - ll[1]);
            self.ellipse(m, x + ll[0], y + h - ll[1], ll[0], ll[1], 0.0, PI, 0.5 * PI, true);
            self.line_to(m, x + w - lr[0], y + h);
            self.ellipse(m, x + w - lr[0], y + h - lr[1], lr[0], lr[1], 0.0, 0.5 * PI, 0.0, true);
            self.line_to(m, x + w, y + ur[1]);
            self.ellipse(m, x + w - ur[0], y + ur[1], ur[0], ur[1], 0.0, 0.0, 1.5 * PI, true);
            self.close();
            return;
        }
        self.line_to(m, x + w - ur[0], y);
        self.ellipse(m, x + w - ur[0], y + ur[1], ur[0], ur[1], 0.0, 1.5 * PI, 2.0 * PI, false);
        self.line_to(m, x + w, y + h - lr[1]);
        self.ellipse(m, x + w - lr[0], y + h - lr[1], lr[0], lr[1], 0.0, 0.0, 0.5 * PI, false);
        self.line_to(m, x + ll[0], y + h);
        self.ellipse(m, x + ll[0], y + h - ll[1], ll[0], ll[1], 0.0, 0.5 * PI, PI, false);
        self.line_to(m, x, y + ul[1]);
        self.ellipse(m, x + ul[0], y + ul[1], ul[0], ul[1], 0.0, PI, 1.5 * PI, false);
        self.close();
    }
}

/// The inverse of an affine matrix (None when singular or not finite).
fn invert(m: &[f64; 6]) -> Option<[f64; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])])
}

/// QCNV8: the pen for stroke/strokeRect from the line-style state (user space; the CTM is the stroke transform).
fn stroke_of(st: &DrawState) -> Stroke {
    let mut s = Stroke { width: st.line_width, miter_limit: st.miter_limit, ..Stroke::default() };
    s.line_cap = match st.cap {
        1 => LineCap::Round,
        2 => LineCap::Square,
        _ => LineCap::Butt,
    };
    s.line_join = match st.join {
        1 => LineJoin::Round,
        2 => LineJoin::Bevel,
        _ => LineJoin::Miter,
    };
    if !st.dash.is_empty() {
        let arr: Vec<f32> = st.dash.iter().map(|d| *d as f32).collect();
        // None (all zeros, overflow): the stroke stays solid.
        s.dash = StrokeDash::new(arr, st.dash_offset as f32);
    }
    s
}

impl Default for DrawState {
    fn default() -> DrawState {
        DrawState { fill: Style::new([0, 0, 0, 255]), stroke: Style::new([0, 0, 0, 255]), shadow: Style::new([0, 0, 0, 0]), alpha: 1.0, line_width: 1.0, cap: 0, join: 0, miter_limit: 10.0, dash: Vec::new(), dash_offset: 0.0, ctm: IDENTITY, clip: None }
    }
}

struct Canvas {
    /// The element's node address (paint and damage know canvases by it).
    node: usize,
    w: u32,
    h: u32,
    pixmap: Option<Pixmap>,
    state: DrawState,
    stack: Vec<DrawState>,
    /// QCNV6: the current default path (not part of the saved state).
    path: PathData,
    /// The bitmap as paint wants it, rebuilt after a draw.
    image: Option<Arc<DecodedImage>>,
}

impl Canvas {
    fn new(node: usize, w: u32, h: u32) -> Canvas {
        let pixmap = if u64::from(w) * u64::from(h) <= MAX_PIXELS { Pixmap::new(w, h) } else { None };
        Canvas { node, w, h, pixmap, state: DrawState::default(), stack: Vec::new(), path: PathData::default(), image: None }
    }
}

static CANVASES: OnceLock<Mutex<Registry>> = OnceLock::new();
static DIRTY: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static ANY: AtomicBool = AtomicBool::new(false);
static PENDING: AtomicBool = AtomicBool::new(false);

/// The canvases by id, and the node-address -> id index for paint.
#[derive(Default)]
struct Registry {
    next: u32,
    by_id: HashMap<u32, Canvas>,
    by_node: HashMap<usize, u32>,
}

impl Registry {
    fn get(&self, id: u32) -> Option<&Canvas> {
        self.by_id.get(&id)
    }
    fn get_mut(&mut self, id: u32) -> Option<&mut Canvas> {
        self.by_id.get_mut(&id)
    }
    /// A bitmap with no element (OffscreenCanvas, QCNV4).
    fn create_offscreen(&mut self, w: u32, h: u32) -> u32 {
        self.next += 1;
        self.by_id.insert(self.next, Canvas::new(NO_NODE, w, h));
        self.next
    }
    /// A fresh canvas for `node`; a canvas the same node address had before (a freed element's) is dropped.
    fn create(&mut self, node: usize, w: u32, h: u32) -> u32 {
        if let Some(old) = self.by_node.remove(&node) {
            self.by_id.remove(&old);
        }
        self.next += 1;
        self.by_id.insert(self.next, Canvas::new(node, w, h));
        self.by_node.insert(node, self.next);
        self.next
    }
}

fn map() -> std::sync::MutexGuard<'static, Registry> {
    CANVASES.get_or_init(|| Mutex::new(Registry::default())).lock().unwrap_or_else(|e| e.into_inner())
}

/// The value of a `width`/`height` attribute by the HTML rules for parsing non-negative integers (leading ASCII
/// whitespace, an optional `+`, digits, anything after them ignored), or `dflt` when it is missing, not a number,
/// negative or above 2^31 - 1. `100.999` -> 100, `1e2` -> 1, `0x10` -> 0, `+100` -> 100.
pub(crate) fn parse_dim(v: Option<&str>, dflt: u32) -> u32 {
    let Some(v) = v else { return dflt };
    let v = v.trim_start_matches(['\t', '\n', '\x0c', '\r', ' ']);
    let v = v.strip_prefix('+').unwrap_or(v);
    let digits: String = v.chars().take_while(char::is_ascii_digit).collect();
    match digits.parse::<u64>() {
        Ok(n) if n <= 2_147_483_647 => n as u32,
        _ => dflt,
    }
}

/// A draw happened since the App last drained the dirty set.
pub(crate) fn pending() -> bool {
    PENDING.load(Ordering::Relaxed)
}

fn mark(node: usize) {
    // QCNV4: an OffscreenCanvas has no node (NO_NODE): nothing paints it.
    if node == NO_NODE {
        return;
    }
    let mut d = DIRTY.lock().unwrap_or_else(|e| e.into_inner());
    if !d.contains(&node) {
        d.push(node);
    }
    PENDING.store(true, Ordering::Relaxed);
}

/// The canvases drawn on since the last call (each once).
pub(crate) fn take_dirty() -> Vec<usize> {
    if !PENDING.swap(false, Ordering::Relaxed) {
        return Vec::new();
    }
    std::mem::take(&mut *DIRTY.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The bitmap of canvas `node` for paint, if it has a context.
pub(crate) fn image(node: usize) -> Option<Arc<DecodedImage>> {
    let mut m = map();
    let id = *m.by_node.get(&node)?;
    let c = m.get_mut(id)?;
    if c.image.is_none() {
        let px = c.pixmap.as_ref()?;
        let pixels = px
            .pixels()
            .iter()
            .map(|p| {
                let d = p.demultiply();
                cosmic_text::Color::rgba(d.red(), d.green(), d.blue(), d.alpha())
            })
            .collect();
        c.image = Some(Arc::new(DecodedImage { width: c.w, height: c.h, pixels, svg: None, mask: None }));
    }
    c.image.clone()
}

// ------------------------------------------------------------ colours

/// A CSS colour string as a [`Style`], or `None` when it does not parse (the assignment is then ignored). `cur` is
/// what `currentcolor` is (the canvas element's computed `color`, straight RGBA). Legacy sRGB colours serialise as
/// `#rrggbb`/`rgba()`; `color()`, `lab()`, `color-mix()` and relative colours keep their CSS form.
pub(crate) fn parse_color(s: &str, cur: [u8; 4]) -> Option<Style> {
    use cssparser::ToCss;
    let s = s.trim();
    let cur_css = cssparser_color::Color::Rgba(cssparser_color::RgbaLegacy::new(cur[0], cur[1], cur[2], f32::from(cur[3]) / 255.0));
    let mut p = cssparser::Parser::new(s);
    let parsed = crate::csscolor::parse(&mut p, Some(&cur_css)).ok().filter(|_| p.expect_exhausted().is_ok());
    let Some(c) = parsed else {
        let rgb = crate::csscolor::system_color(s)?;
        return Some(Style::new([rgb[0], rgb[1], rgb[2], 255]));
    };
    let rgba = crate::csscolor::to_srgb8(&c)?;
    if matches!(c, cssparser_color::Color::Rgba(_) | cssparser_color::Color::Hsl(_) | cssparser_color::Color::Hwb(_)) {
        Some(Style::new(rgba))
    } else {
        Some(Style { rgba, ser: c.to_css_string() })
    }
}

/// The serialisation of a `fillStyle`/`strokeStyle` getter: `#rrggbb`, or `rgba(r, g, b, a)` when not opaque.
pub(crate) fn serialize_color(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        // The shortest decimal (2, then 3 digits) that is still this alpha as 8 bits: 128 -> 0.5, not 0.502.
        let exact = f64::from(c[3]) / 255.0;
        let a = [100.0, 1000.0].iter().map(|k| (exact * k).round() / k).find(|r| (r * 255.0).round() as u8 == c[3]).unwrap_or(exact);
        format!("rgba({}, {}, {}, {})", c[0], c[1], c[2], a)
    }
}

// ------------------------------------------------------------ drawing

fn rect_of(x: f64, y: f64, w: f64, h: f64) -> Option<Rect> {
    if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
        return None;
    }
    let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
    let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
    Rect::from_xywh(x as f32, y as f32, w as f32, h as f32)
}

fn paint_of(c: [u8; 4], alpha: f32) -> Paint<'static> {
    let mut p = Paint::default();
    let a = f32::from(c[3]) / 255.0 * alpha;
    p.set_color(Color::from_rgba(f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, a.clamp(0.0, 1.0)).unwrap_or(Color::TRANSPARENT));
    p.anti_alias = true;
    p
}

fn transform_of(m: &[f64; 6]) -> Transform {
    Transform::from_row(m[0] as f32, m[1] as f32, m[2] as f32, m[3] as f32, m[4] as f32, m[5] as f32)
}

/// `op`: 0 multiply the CTM by `m` on the right (transform/translate/rotate/scale), 1 replace it (setTransform), 2 reset.
fn set_ctm(c: &mut Canvas, op: i32, m: &[f64]) {
    let [a, b, cc, d, e, f] = c.state.ctm;
    c.state.ctm = match (op, m) {
        (0, [ma, mb, mc, md, me, mf]) => [
            a * ma + cc * mb,
            b * ma + d * mb,
            a * mc + cc * md,
            b * mc + d * md,
            a * me + cc * mf + e,
            b * me + d * mf + f,
        ],
        (1, [ma, mb, mc, md, me, mf]) => [*ma, *mb, *mc, *md, *me, *mf],
        _ => IDENTITY,
    };
}

/// `op`: 0 fillRect, 1 clearRect, 2 strokeRect (`a` = x, y, w, h).
fn draw(c: &mut Canvas, op: i32, a: &[f64]) {
    let (Some(px), [x, y, w, h]) = (c.pixmap.as_mut(), a) else { return };
    let ts = transform_of(&c.state.ctm);
    let mask = c.state.clip.as_deref();
    match op {
        0 => {
            if let Some(r) = rect_of(*x, *y, *w, *h) {
                px.fill_rect(r, &paint_of(c.state.fill.rgba, c.state.alpha), ts, mask);
            }
        }
        1 => {
            if let Some(r) = rect_of(*x, *y, *w, *h) {
                // DestinationOut with an opaque source clears and, unlike Clear, honours the clip mask (QCNV6).
                let p = Paint { blend_mode: BlendMode::DestinationOut, anti_alias: true, ..Paint::default() };
                px.fill_rect(r, &p, ts, mask);
            }
        }
        _ => {
            if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) || (*w == 0.0 && *h == 0.0) {
                return;
            }
            let (x0, y0, x1, y1) = (*x as f32, *y as f32, (*x + *w) as f32, (*y + *h) as f32);
            let mut pb = PathBuilder::new();
            pb.move_to(x0, y0);
            pb.line_to(x1, y0);
            pb.line_to(x1, y1);
            pb.line_to(x0, y1);
            pb.close();
            if let Some(path) = pb.finish() {
                let stroke = stroke_of(&c.state);
                px.stroke_path(&path, &paint_of(c.state.stroke.rgba, c.state.alpha), &stroke, ts, mask);
            }
        }
    }
}

/// QCNV6: `op` 0 beginPath, 1 moveTo, 2 lineTo, 3 closePath, 4 rect (`a` = the arguments, all finite).
fn path_op(c: &mut Canvas, op: i32, a: &[f64]) {
    let m = c.state.ctm;
    match (op, a) {
        (0, _) => c.path.segs.clear(),
        (1, [x, y]) => c.path.move_to(&m, *x, *y),
        (2, [x, y]) => c.path.line_to(&m, *x, *y),
        (3, _) => c.path.close(),
        (4, [x, y, w, h]) => c.path.rect(&m, *x, *y, *w, *h),
        // QCNV7: quadraticCurveTo, bezierCurveTo, arc/ellipse (rx, ry, rotation, start, end, ccw), arcTo, roundRect.
        (5, [cx, cy, x, y]) => c.path.quad(&m, *cx, *cy, *x, *y),
        (6, [a, b, cc, d, x, y]) => c.path.cubic(&m, (*a, *b), (*cc, *d), (*x, *y)),
        (7, [x, y, rx, ry, rot, s, e, ccw]) => c.path.ellipse(&m, *x, *y, *rx, *ry, *rot, *s, *e, *ccw != 0.0),
        (8, [x1, y1, x2, y2, r]) => c.path.arc_to(&m, *x1, *y1, *x2, *y2, *r),
        (9, [x, y, w, h, r @ ..]) if r.len() == 8 => c.path.round_rect(&m, *x, *y, *w, *h, r),
        _ => {}
    }
}

/// QCNV6: `op` 0 fill, 1 stroke, 2 clip; `evenodd` picks the fill rule (fill and clip).
fn paint_path(c: &mut Canvas, op: i32, evenodd: bool) {
    let rule = if evenodd { FillRule::EvenOdd } else { FillRule::Winding };
    let path = if op == 1 { c.path.build_pruned() } else { c.path.build() };
    let (w, h) = (c.w, c.h);
    if op == 2 {
        // The region is replaced by its intersection with the path (an empty path clips everything away).
        let mut m = match c.state.clip.as_deref() {
            Some(m) => m.clone(),
            None => {
                let Some(mut m) = Mask::new(w, h) else { return };
                m.data_mut().fill(255);
                m
            }
        };
        match path {
            Some(p) => m.intersect_path(&p, rule, true, Transform::identity()),
            None => m.clear(),
        }
        c.state.clip = Some(Arc::new(m));
        return;
    }
    let (Some(px), Some(path)) = (c.pixmap.as_mut(), path) else { return };
    let mask = c.state.clip.as_deref();
    if op == 0 {
        px.fill_path(&path, &paint_of(c.state.fill.rgba, c.state.alpha), rule, Transform::identity(), mask);
    } else {
        // The path is in device space; the pen is in user space, so stroke the user-space path through the CTM.
        let Some(inv) = invert(&c.state.ctm) else { return };
        let Some(user) = path.transform(transform_of(&inv)) else { return };
        let stroke = stroke_of(&c.state);
        px.stroke_path(&user, &paint_of(c.state.stroke.rgba, c.state.alpha), &stroke, transform_of(&c.state.ctm), mask);
    }
}

/// Straight RGBA of the `w` x `h` rect at `(x, y)` (may reach outside the bitmap: transparent black there).
fn get_image_data(c: &Canvas, x: i32, y: i32, w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; w as usize * h as usize * 4];
    let Some(px) = c.pixmap.as_ref() else { return out };
    for row in 0..h as i64 {
        let sy = i64::from(y) + row;
        if sy < 0 || sy >= i64::from(c.h) {
            continue;
        }
        for col in 0..w as i64 {
            let sx = i64::from(x) + col;
            if sx < 0 || sx >= i64::from(c.w) {
                continue;
            }
            let p = px.pixels()[(sy as usize) * c.w as usize + sx as usize].demultiply();
            let o = (row as usize * w as usize + col as usize) * 4;
            out[o..o + 4].copy_from_slice(&[p.red(), p.green(), p.blue(), p.alpha()]);
        }
    }
    out
}

/// Copies the `(sx, sy, sw, sh)` part of `data` (an image `dw` wide) onto the bitmap at `(dx + sx, dy + sy)`, no
/// compositing (`putImageData`).
#[allow(clippy::too_many_arguments)]
fn put_image_data(c: &mut Canvas, data: &[u8], dw: u32, dx: i32, dy: i32, sx: i32, sy: i32, sw: i32, sh: i32) {
    let (cw, ch) = (i64::from(c.w), i64::from(c.h));
    let Some(px) = c.pixmap.as_mut() else { return };
    let pixels = px.pixels_mut();
    for row in sy..sy + sh {
        let ty = i64::from(dy) + i64::from(row);
        if ty < 0 || ty >= ch {
            continue;
        }
        for col in sx..sx + sw {
            let tx = i64::from(dx) + i64::from(col);
            if tx < 0 || tx >= cw {
                continue;
            }
            let o = (row as usize * dw as usize + col as usize) * 4;
            let Some(s) = data.get(o..o + 4) else { continue };
            pixels[ty as usize * c.w as usize + tx as usize] = tiny_skia::ColorU8::from_rgba(s[0], s[1], s[2], s[3]).premultiply();
        }
    }
}

// ------------------------------------------------------------ natives

/// The `__cv_*` natives of the JS context object. `id` is the element's DOM-table id (only `__cv_ctx`/`__cv_reset` need
/// it, to read the attributes and find the node); every other native takes the canvas id `__cv_ctx` returned.
pub(crate) fn register(ctx: &Ctx<'_>, table: Rc<RefCell<DomTable>>) {
    let node_dims = {
        let t = table.clone();
        move |id: i32| -> Option<(usize, u32, u32)> {
            let h = t.borrow().get(id)?;
            let dim = |n: &str, d: u32| parse_dim(crate::dom::element_attr(&h, n).as_deref(), d);
            Some((crate::dom::node_id(&h), dim("width", DEFAULT_W), dim("height", DEFAULT_H)))
        }
    };
    // A new bitmap for the element (a context was asked for); returns the canvas id, 0 when the element is gone.
    let nd = node_dims.clone();
    native!(ctx, "__cv_ctx", move |id: i32| -> i32 {
        let Some((node, w, h)) = nd(id) else { return 0 };
        ANY.store(true, Ordering::Relaxed);
        map().create(node, w, h) as i32
    });
    // QCNV4: a bitmap for `new OffscreenCanvas(w, h)` (no element; nothing paints it), its reset when `width`/`height`
    // is set, and its release when the JS object is collected.
    native!(ctx, "__cv_ctx_off", move |w: f64, h: f64| -> i32 { map().create_offscreen(w as u32, h as u32) as i32 });
    native!(ctx, "__cv_reset_off", move |cid: i32, w: f64, h: f64| {
        if let Some(c) = map().get_mut(cid as u32) {
            *c = Canvas::new(NO_NODE, w as u32, h as u32);
        }
    });
    native!(ctx, "__cv_free", move |cid: i32| {
        map().by_id.remove(&(cid as u32));
    });
    // width/height was set: the bitmap and the state are reset.
    native!(ctx, "__cv_reset", move |cid: i32, id: i32| {
        let Some((_, w, h)) = node_dims(id) else { return };
        let mut m = map();
        if let Some(c) = m.get_mut(cid as u32) {
            let node = c.node;
            *c = Canvas::new(node, w, h);
            drop(m);
            mark(node);
        }
    });
    native!(ctx, "__cv_draw", move |cid: i32, op: i32, a: Vec<f64>| {
        if let Some(c) = map().get_mut(cid as u32) {
            draw(c, op, &a);
            c.image = None;
            mark(c.node);
        }
    });
    // QCNV6: path building (see `path_op`; the JS side drops non-finite arguments) and fill/stroke/clip (`paint_path`).
    native!(ctx, "__cv_path", move |cid: i32, op: i32, a: Vec<f64>| {
        if let Some(c) = map().get_mut(cid as u32) {
            path_op(c, op, &a);
        }
    });
    native!(ctx, "__cv_paint", move |cid: i32, op: i32, evenodd: bool| {
        if let Some(c) = map().get_mut(cid as u32) {
            paint_path(c, op, evenodd);
            c.image = None;
            if op != 2 {
                mark(c.node);
            }
        }
    });
    // which: 0 fillStyle, 1 strokeStyle, 2 shadowColor. `cur` is the element's computed colour (only read when the
    // string needs it). Returns false when `s` is not a colour (the attribute keeps its value).
    native!(ctx, "__cv_set_style", move |cid: i32, which: i32, s: String, cur: String| -> bool {
        let cur = parse_color(&cur, [0, 0, 0, 255]).map_or([0, 0, 0, 255], |c| c.rgba);
        let Some(col) = parse_color(&s, cur) else { return false };
        if let Some(c) = map().get_mut(cid as u32) {
            match which {
                0 => c.state.fill = col,
                1 => c.state.stroke = col,
                _ => c.state.shadow = col,
            }
        }
        true
    });
    native!(ctx, "__cv_get_style", move |cid: i32, which: i32| -> String {
        map()
            .get(cid as u32)
            .map(|c| match which {
                0 => c.state.fill.ser.clone(),
                1 => c.state.stroke.ser.clone(),
                _ => c.state.shadow.ser.clone(),
            })
            .unwrap_or_default()
    });
    // which: 0 globalAlpha, 1 lineWidth.
    native!(ctx, "__cv_set_num", move |cid: i32, which: i32, v: f64| {
        if let Some(c) = map().get_mut(cid as u32) {
            if which == 0 {
                c.state.alpha = v as f32;
            } else {
                c.state.line_width = v as f32;
            }
        }
    });
    native!(ctx, "__cv_get_num", move |cid: i32, which: i32| -> f64 {
        map().get(cid as u32).map_or(0.0, |c| match which {
            0 => f64::from(c.state.alpha),
            1 => f64::from(c.state.line_width),
            n => c.state.ctm.get((n - 2) as usize).copied().unwrap_or(0.0),
        })
    });
    // QCNV5: transform/setTransform/resetTransform (see `set_ctm`); the JS side has already dropped non-finite input.
    // QCNV8: which 0 lineCap, 1 lineJoin, 2 miterLimit, 3 lineDashOffset (the JS side validated the value).
    native!(ctx, "__cv_set_ls", move |cid: i32, which: i32, v: f64| {
        if let Some(c) = map().get_mut(cid as u32) {
            match which {
                0 => c.state.cap = v as u8,
                1 => c.state.join = v as u8,
                2 => c.state.miter_limit = v as f32,
                _ => c.state.dash_offset = v,
            }
        }
    });
    native!(ctx, "__cv_get_ls", move |cid: i32, which: i32| -> f64 {
        map().get(cid as u32).map_or(0.0, |c| match which {
            0 => f64::from(c.state.cap),
            1 => f64::from(c.state.join),
            2 => f64::from(c.state.miter_limit),
            _ => c.state.dash_offset,
        })
    });
    // setLineDash/getLineDash: the list travels as a comma-separated string (the JS side validated and doubled it).
    native!(ctx, "__cv_set_dash", move |cid: i32, list: String| {
        if let Some(c) = map().get_mut(cid as u32) {
            c.state.dash = list.split(',').filter_map(|t| t.parse::<f64>().ok()).collect();
        }
    });
    native!(ctx, "__cv_get_dash", move |cid: i32| -> String {
        map().get(cid as u32).map(|c| c.state.dash.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",")).unwrap_or_default()
    });
    native!(ctx, "__cv_xf", move |cid: i32, op: i32, m: Vec<f64>| {
        if let Some(c) = map().get_mut(cid as u32) {
            set_ctm(c, op, &m);
        }
    });
    native!(ctx, "__cv_save", move |cid: i32| {
        if let Some(c) = map().get_mut(cid as u32) {
            let s = c.state.clone();
            c.stack.push(s);
        }
    });
    native!(ctx, "__cv_restore", move |cid: i32| {
        if let Some(c) = map().get_mut(cid as u32)
            && let Some(s) = c.stack.pop()
        {
            c.state = s;
        }
    });
    native!(ctx, "__cv_get_image", move |cid: i32, x: i32, y: i32, w: i32, h: i32| -> Option<Bytes> {
        let m = map();
        let c = m.get(cid as u32)?;
        Some(Bytes(get_image_data(c, x, y, w.max(0) as u32, h.max(0) as u32)))
    });
    native!(ctx, "__cv_put_image", move |cid: i32, data: Bytes, g: Vec<f64>| {
        if let Some(c) = map().get_mut(cid as u32) {
            if let [dw, dx, dy, sx, sy, sw, sh] = g[..] {
                put_image_data(c, &data.0, dw.max(0.0) as u32, dx as i32, dy as i32, sx as i32, sy as i32, sw as i32, sh as i32);
            }
            c.image = None;
            mark(c.node);
        }
    });
}

/// Test seam: the straight RGBA of one pixel of canvas `node`.
#[cfg(test)]
pub(crate) fn pixel(node: usize, x: u32, y: u32) -> Option<[u8; 4]> {
    let m = map();
    let c = m.get(*m.by_node.get(&node)?)?;
    let d = get_image_data(c, x as i32, y as i32, 1, 1);
    Some([d[0], d[1], d[2], d[3]])
}
