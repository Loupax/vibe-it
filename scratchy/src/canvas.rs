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

use tiny_skia::{BlendMode, Color, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

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
    /// QCNV5: the current transformation matrix as (a, b, c, d, e, f): x' = a x + c y + e, y' = b x + d y + f.
    ctm: [f64; 6],
}

impl Default for DrawState {
    fn default() -> DrawState {
        DrawState { fill: Style::new([0, 0, 0, 255]), stroke: Style::new([0, 0, 0, 255]), shadow: Style::new([0, 0, 0, 0]), alpha: 1.0, line_width: 1.0, ctm: IDENTITY }
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
    /// The bitmap as paint wants it, rebuilt after a draw.
    image: Option<Arc<DecodedImage>>,
}

impl Canvas {
    fn new(node: usize, w: u32, h: u32) -> Canvas {
        let pixmap = if u64::from(w) * u64::from(h) <= MAX_PIXELS { Pixmap::new(w, h) } else { None };
        Canvas { node, w, h, pixmap, state: DrawState::default(), stack: Vec::new(), image: None }
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

/// Some canvas owns a bitmap (pages without one never look further).
pub(crate) fn any() -> bool {
    ANY.load(Ordering::Relaxed)
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
    match op {
        0 => {
            if let Some(r) = rect_of(*x, *y, *w, *h) {
                px.fill_rect(r, &paint_of(c.state.fill.rgba, c.state.alpha), ts, None);
            }
        }
        1 => {
            if let Some(r) = rect_of(*x, *y, *w, *h) {
                let p = Paint { blend_mode: BlendMode::Clear, anti_alias: true, ..Paint::default() };
                px.fill_rect(r, &p, ts, None);
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
                px.stroke_path(&path, &paint_of(c.state.stroke.rgba, c.state.alpha), &stroke, ts, None);
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
