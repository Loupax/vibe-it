//! QCNV1: the Canvas 2D backing store and its first API slice.
//!
//! Each `<canvas>` that asked for a `'2d'` context owns one [`Canvas`] here, keyed by the element's node id: a
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

use tiny_skia::{BlendMode, Color, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use crate::js_dom::DomTable;
use crate::js_engine::{native, Bytes, Ctx};
use crate::layout::DecodedImage;

/// `DecodedImage::svg` of a canvas placeholder (never a real SVG: placeholders have no pixels).
pub(crate) const MARK: &[u8] = b"scratchy:canvas";
/// Bitmaps above this many pixels are not allocated (the context then draws nothing, as when allocation fails).
const MAX_PIXELS: u64 = 1 << 26;
/// The default size of a canvas element without `width`/`height` attributes.
pub(crate) const DEFAULT_W: u32 = 300;
pub(crate) const DEFAULT_H: u32 = 150;

pub(crate) fn is_placeholder(img: &DecodedImage) -> bool {
    img.svg.as_ref().is_some_and(|s| s.as_slice() == MARK)
}

#[derive(Clone)]
struct DrawState {
    fill: [u8; 4],
    stroke: [u8; 4],
    alpha: f32,
    line_width: f32,
}

impl Default for DrawState {
    fn default() -> DrawState {
        DrawState { fill: [0, 0, 0, 255], stroke: [0, 0, 0, 255], alpha: 1.0, line_width: 1.0 }
    }
}

struct Canvas {
    w: u32,
    h: u32,
    pixmap: Option<Pixmap>,
    state: DrawState,
    stack: Vec<DrawState>,
    /// The bitmap as paint wants it, rebuilt after a draw.
    image: Option<Arc<DecodedImage>>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Canvas {
        let pixmap = if u64::from(w) * u64::from(h) <= MAX_PIXELS { Pixmap::new(w, h) } else { None };
        Canvas { w, h, pixmap, state: DrawState::default(), stack: Vec::new(), image: None }
    }
}

static CANVASES: OnceLock<Mutex<HashMap<usize, Canvas>>> = OnceLock::new();
static DIRTY: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static ANY: AtomicBool = AtomicBool::new(false);
static PENDING: AtomicBool = AtomicBool::new(false);

fn map() -> std::sync::MutexGuard<'static, HashMap<usize, Canvas>> {
    CANVASES.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

/// Some canvas owns a bitmap (pages without one never look further).
pub(crate) fn any() -> bool {
    ANY.load(Ordering::Relaxed)
}

/// A draw happened since the App last drained the dirty set.
pub(crate) fn pending() -> bool {
    PENDING.load(Ordering::Relaxed)
}

fn mark(node: usize) {
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
    let c = m.get_mut(&node)?;
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

/// A CSS colour string as straight RGBA, or `None` when it does not parse (the assignment is then ignored).
pub(crate) fn parse_color(s: &str) -> Option<[u8; 4]> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("currentcolor") {
        return Some([0, 0, 0, 255]);
    }
    let mut p = cssparser::Parser::new(s);
    let c = cssparser_color::Color::parse(&mut p).ok()?;
    p.expect_exhausted().ok()?;
    let t = crate::css::to_text_color(&c)?;
    Some([t.r(), t.g(), t.b(), t.a()])
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

/// `op`: 0 fillRect, 1 clearRect, 2 strokeRect (`a` = x, y, w, h).
fn draw(c: &mut Canvas, op: i32, a: &[f64]) {
    let (Some(px), [x, y, w, h]) = (c.pixmap.as_mut(), a) else { return };
    match op {
        0 => {
            if let Some(r) = rect_of(*x, *y, *w, *h) {
                px.fill_rect(r, &paint_of(c.state.fill, c.state.alpha), Transform::identity(), None);
            }
        }
        1 => {
            if let Some(r) = rect_of(*x, *y, *w, *h) {
                let mut p = Paint::default();
                p.blend_mode = BlendMode::Clear;
                p.anti_alias = true;
                px.fill_rect(r, &p, Transform::identity(), None);
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
                let stroke = Stroke { width: c.state.line_width, ..Stroke::default() };
                px.stroke_path(&path, &paint_of(c.state.stroke, c.state.alpha), &stroke, Transform::identity(), None);
            }
        }
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

/// The `__cv_*` natives of the JS context object. `id` is the element's DOM-table id.
pub(crate) fn register(ctx: &Ctx<'_>, table: Rc<RefCell<DomTable>>) {
    let node_of = {
        let t = table.clone();
        move |id: i32| t.borrow().get(id).map(|h| crate::dom::node_id(&h))
    };
    let attr_dim = {
        let t = table.clone();
        move |id: i32, name: &str, dflt: u32| -> u32 {
            let Some(h) = t.borrow().get(id) else { return dflt };
            crate::dom::element_attr(&h, name).and_then(|v| v.trim().parse::<i64>().ok()).filter(|v| *v >= 0).map_or(dflt, |v| v.min(i64::from(u32::MAX)) as u32)
        }
    };
    // Ensures the element has a bitmap (a context was asked for); true when it exists.
    let (n, a) = (node_of.clone(), attr_dim.clone());
    native!(ctx, "__cv_ctx", move |id: i32| -> bool {
        let Some(node) = n(id) else { return false };
        let mut m = map();
        m.insert(node, Canvas::new(a(id, "width", DEFAULT_W), a(id, "height", DEFAULT_H)));
        ANY.store(true, Ordering::Relaxed);
        true
    });
    // width/height was set: the bitmap and the state are reset.
    let (n, a) = (node_of.clone(), attr_dim);
    native!(ctx, "__cv_reset", move |id: i32| {
        let Some(node) = n(id) else { return };
        let mut m = map();
        if let Some(c) = m.get_mut(&node) {
            *c = Canvas::new(a(id, "width", DEFAULT_W), a(id, "height", DEFAULT_H));
            drop(m);
            mark(node);
        }
    });
    let n = node_of.clone();
    native!(ctx, "__cv_draw", move |id: i32, op: i32, a: Vec<f64>| {
        let Some(node) = n(id) else { return };
        if let Some(c) = map().get_mut(&node) {
            draw(c, op, &a);
            c.image = None;
            mark(node);
        }
    });
    // which: 0 fillStyle, 1 strokeStyle. Returns the serialised colour, or "" when `s` is not a colour.
    let n = node_of.clone();
    native!(ctx, "__cv_set_style", move |id: i32, which: i32, s: String| -> String {
        let (Some(node), Some(col)) = (n(id), parse_color(&s)) else { return String::new() };
        if let Some(c) = map().get_mut(&node) {
            if which == 0 {
                c.state.fill = col;
            } else {
                c.state.stroke = col;
            }
        }
        serialize_color(col)
    });
    let n = node_of.clone();
    native!(ctx, "__cv_get_style", move |id: i32, which: i32| -> String {
        let Some(node) = n(id) else { return String::new() };
        map().get(&node).map(|c| serialize_color(if which == 0 { c.state.fill } else { c.state.stroke })).unwrap_or_default()
    });
    // which: 0 globalAlpha, 1 lineWidth.
    let n = node_of.clone();
    native!(ctx, "__cv_set_num", move |id: i32, which: i32, v: f64| {
        let Some(node) = n(id) else { return };
        if let Some(c) = map().get_mut(&node) {
            if which == 0 {
                c.state.alpha = v as f32;
            } else {
                c.state.line_width = v as f32;
            }
        }
    });
    let n = node_of.clone();
    native!(ctx, "__cv_get_num", move |id: i32, which: i32| -> f64 {
        let Some(node) = n(id) else { return 0.0 };
        map().get(&node).map_or(0.0, |c| f64::from(if which == 0 { c.state.alpha } else { c.state.line_width }))
    });
    let n = node_of.clone();
    native!(ctx, "__cv_save", move |id: i32| {
        if let Some(node) = n(id)
            && let Some(c) = map().get_mut(&node)
        {
            let s = c.state.clone();
            c.stack.push(s);
        }
    });
    let n = node_of.clone();
    native!(ctx, "__cv_restore", move |id: i32| {
        if let Some(node) = n(id)
            && let Some(c) = map().get_mut(&node)
            && let Some(s) = c.stack.pop()
        {
            c.state = s;
        }
    });
    let n = node_of.clone();
    native!(ctx, "__cv_get_image", move |id: i32, x: i32, y: i32, w: i32, h: i32| -> Option<Bytes> {
        let node = n(id)?;
        let m = map();
        let c = m.get(&node)?;
        Some(Bytes(get_image_data(c, x, y, w.max(0) as u32, h.max(0) as u32)))
    });
    let n = node_of;
    native!(ctx, "__cv_put_image", move |id: i32, data: Bytes, g: Vec<f64>| {
        let Some(node) = n(id) else { return };
        if let Some(c) = map().get_mut(&node) {
            if let [dw, dx, dy, sx, sy, sw, sh] = g[..] { put_image_data(c, &data.0, dw.max(0.0) as u32, dx as i32, dy as i32, sx as i32, sy as i32, sw as i32, sh as i32); }
            c.image = None;
            mark(node);
        }
    });
}

/// Test seam: the straight RGBA of one pixel of canvas `node`.
#[cfg(test)]
pub(crate) fn pixel(node: usize, x: u32, y: u32) -> Option<[u8; 4]> {
    let m = map();
    let c = m.get(&node)?;
    let d = get_image_data(c, x as i32, y as i32, 1, 1);
    Some([d[0], d[1], d[2], d[3]])
}
