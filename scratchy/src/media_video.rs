//! QM1g/QM1g1: painting a `<video>` and the video-rect-only redraw.
//!
//! The pure parts (rect clipping, letterbox math, the sentinel overlay mask,
//! frame-to-rect compositing, the visibility rules) live here and are tested
//! in the default build; libmpv itself is in `media_mpv`.
//!
//! How the box is painted: the layout tree has an ordinary replaced box for a
//! `<video>` (`BoxContent::Image` of an EMPTY-pixel `DecodedImage`, the video
//! placeholder; its `mask` field carries the poster). `paint` calls
//! [`paint_box`] for it. During a full `App::redraw` that fills the visible
//! (clipped) part of the box with [`SENTINEL`] (top byte 0xFE: `blend` always
//! writes a top byte of 0, so ANY later paint over a pixel destroys the
//! sentinel). After everything else is painted, [`finish`] scans each rect:
//! the pixels that are still the sentinel are exactly the video pixels that no
//! other content covers (z-order, overflow, clipping, scroll: all for free).
//! They become the box's overlay mask, and the newest frame (or the poster, or
//! black) is copied through it. A new frame later only needs [`update_box`]
//! (copy through the cached mask) plus `present_with_damage`.

// `wake` is only called by the `media` backend, `set_boxes_enabled`/`fit_rect` by tests.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::css::ObjectFit;
use crate::layout::DecodedImage;

/// Top byte 0xFE: the page buffer is 0RGB and `blend` always yields a top byte of 0.
pub(crate) const SENTINEL: u32 = 0xFE0A_0B0C;
const BLACK: u32 = 0;

pub(crate) type FrameMap = HashMap<usize, Arc<VideoFrame>>;
pub(crate) type SizeMap = HashMap<usize, (u32, u32)>;

/// One decoded video frame in the page buffer's pixel layout (0RGB).
pub(crate) struct VideoFrame {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
    /// QM1g2a1: where `px` goes back to when the last holder drops the frame.
    pub pool: Option<Arc<crate::media_render::FramePool>>,
}

impl VideoFrame {
    pub(crate) fn new(w: u32, h: u32, px: Vec<u32>) -> VideoFrame {
        VideoFrame { w, h, px, pool: None }
    }
}

impl Drop for VideoFrame {
    fn drop(&mut self) {
        if let Some(p) = self.pool.take() {
            p.put(std::mem::take(&mut self.px));
        }
    }
}

/// What the backend should render: the video box's content size in device
/// pixels and how the picture sits in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VideoTarget {
    pub w: u32,
    pub h: u32,
    pub fit: ObjectFit,
}

// ------------------------------------------------------------ enabling

thread_local! {
    static ENABLED_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Test seam: force `<video>` boxes on/off on this thread (None = automatic).
pub(crate) fn set_boxes_enabled(v: Option<bool>) {
    ENABLED_OVERRIDE.with(|c| c.set(v));
}

/// True when `<video>` becomes a replaced box: only with the `media` feature
/// and a loadable libmpv. Otherwise the default build keeps today's layout
/// exactly (fallback content inline).
pub(crate) fn boxes_enabled() -> bool {
    if let Some(v) = ENABLED_OVERRIDE.with(|c| c.get()) {
        return v;
    }
    #[cfg(feature = "media")]
    {
        crate::media_mpv::libmpv_available()
    }
    #[cfg(not(feature = "media"))]
    {
        false
    }
}

/// The default `<video>` box size when neither CSS, attributes nor the stream give one.
pub(crate) const DEFAULT_W: u32 = 300;
pub(crate) const DEFAULT_H: u32 = 150;

thread_local! {
    /// Intrinsic video size by element node id, set by the media layer.
    static INTRINSIC: RefCell<HashMap<usize, (u32, u32)>> = RefCell::new(HashMap::new());
    /// The newest frame by element node id (what `paint` shows).
    static FRAMES: RefCell<HashMap<usize, Arc<VideoFrame>>> = RefCell::new(HashMap::new());
}

pub(crate) fn set_intrinsic(sizes: HashMap<usize, (u32, u32)>) {
    let changed = INTRINSIC.with(|m| std::mem::replace(&mut *m.borrow_mut(), sizes) != *m.borrow());
    if changed {
        // QINC1j: a media size arrived / changed (the box engine diffs the sizes it read).
        crate::incr_frame::note_ambient(crate::incr_log::Ambient::MediaSize);
    }
}
pub(crate) fn intrinsic(node: usize) -> Option<(u32, u32)> {
    INTRINSIC.with(|m| m.borrow().get(&node).copied())
}
pub(crate) fn set_frames(frames: HashMap<usize, Arc<VideoFrame>>) {
    FRAMES.with(|m| *m.borrow_mut() = frames);
}
pub(crate) fn frame_of(node: usize) -> Option<Arc<VideoFrame>> {
    FRAMES.with(|m| m.borrow().get(&node).cloned())
}

// ------------------------------------------------------------ wake-up

type WakeFn = Box<dyn Fn() + Send + Sync>;
static WAKE: OnceLock<Mutex<Option<WakeFn>>> = OnceLock::new();
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);

/// The app installs what posts the "video frame ready" user event.
pub(crate) fn set_wake(f: Option<WakeFn>) {
    *WAKE.get_or_init(Default::default).lock().unwrap() = f;
}

/// Called from ANY thread (mpv's render-update callback). Coalesced: at most
/// one event is outstanding until the main thread calls [`wake_handled`].
pub(crate) fn wake() {
    if WAKE_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let sent = WAKE.get().is_some_and(|m| m.lock().unwrap().as_ref().map(|f| f()).is_some());
    if !sent {
        WAKE_PENDING.store(false, Ordering::SeqCst);
    }
}
pub(crate) fn wake_handled() {
    WAKE_PENDING.store(false, Ordering::SeqCst);
}

// ------------------------------------------------------------ pure helpers

/// An integer rect: `(x, y, w, h)`, window coordinates.
pub(crate) type IRect = (i32, i32, u32, u32);

/// The part of `rect` inside the window and the optional clip (both clip and
/// rect as float screen-space boxes, as `paint` passes them).
pub(crate) fn clip_rect(rect: IRect, clip: Option<(f32, f32, f32, f32)>, win_w: u32, win_h: u32) -> Option<IRect> {
    let (mut x0, mut y0) = (rect.0.max(0), rect.1.max(0));
    let (mut x1, mut y1) = ((rect.0 as i64 + rect.2 as i64).min(win_w as i64) as i32, (rect.1 as i64 + rect.3 as i64).min(win_h as i64) as i32);
    if let Some((cx, cy, cw, ch)) = clip {
        x0 = x0.max(cx.round() as i32);
        y0 = y0.max(cy.round() as i32);
        x1 = x1.min((cx + cw).round() as i32);
        y1 = y1.min((cy + ch).round() as i32);
    }
    (x1 > x0 && y1 > y0).then(|| (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// Where the picture lands inside a `bw` x `bh` box for a `vw` x `vh` source:
/// `(x, y, w, h)` relative to the box, centred (`object-fit`; the rest of the
/// box is the black letterbox). May exceed the box for `cover`/`none`.
pub(crate) fn fit_rect(bw: f32, bh: f32, vw: f32, vh: f32, fit: ObjectFit) -> (f32, f32, f32, f32) {
    if vw <= 0.0 || vh <= 0.0 || bw <= 0.0 || bh <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let (w, h) = match fit {
        ObjectFit::Fill => (bw, bh),
        ObjectFit::Contain => {
            let s = (bw / vw).min(bh / vh);
            (vw * s, vh * s)
        }
        ObjectFit::Cover => {
            let s = (bw / vw).max(bh / vh);
            (vw * s, vh * s)
        }
        ObjectFit::None => (vw, vh),
        ObjectFit::ScaleDown => {
            let s = (bw / vw).min(bh / vh).min(1.0);
            (vw * s, vh * s)
        }
    };
    ((bw - w) / 2.0, (bh - h) / 2.0, w, h)
}

/// The visibility rule: a video is rendered only while some of its box is
/// really on screen (not covered, not scrolled/clipped away) and the page
/// is shown (not parked behind `:help`).
pub(crate) fn wants_render(visible_pixels: usize, layered: bool, page_hidden: bool) -> bool {
    !page_hidden && (visible_pixels > 0 || layered)
}

/// Fills the visible part of a box with the sentinel.
pub(crate) fn fill(pixels: &mut [u32], win_w: u32, vis: IRect, color: u32) {
    for y in vis.1..vis.1 + vis.3 as i32 {
        let o = y as usize * win_w as usize;
        if let Some(row) = pixels.get_mut(o + vis.0 as usize..o + vis.0 as usize + vis.2 as usize) {
            row.fill(color);
        }
    }
}

/// The overlay mask of a rect: `true` where the pixel is still the sentinel
/// (nothing painted above the video there).
pub(crate) fn overlay_mask(pixels: &[u32], win_w: u32, vis: IRect) -> Vec<bool> {
    let mut m = Vec::with_capacity(vis.2 as usize * vis.3 as usize);
    for y in vis.1..vis.1 + vis.3 as i32 {
        let o = y as usize * win_w as usize + vis.0 as usize;
        match pixels.get(o..o + vis.2 as usize) {
            Some(row) => m.extend(row.iter().map(|&p| p == SENTINEL)),
            None => m.extend(std::iter::repeat_n(false, vis.2 as usize)),
        }
    }
    m
}

/// The bounding rect of the `true` cells of a mask over `vis`.
pub(crate) fn mask_bbox(mask: &[bool], vis: IRect) -> Option<IRect> {
    let (w, h) = (vis.2 as usize, vis.3 as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        let row = &mask[y * w..(y + 1) * w];
        if let (Some(a), Some(b)) = (row.iter().position(|&m| m), row.iter().rposition(|&m| m)) {
            x0 = x0.min(a);
            x1 = x1.max(b + 1);
            y0 = y0.min(y);
            y1 = y1.max(y + 1);
        }
    }
    (x1 > x0).then(|| (vis.0 + x0 as i32, vis.1 + y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// What a box shows: a frame, the poster composed to the box size, or black.
pub(crate) enum Src {
    Frame(Arc<VideoFrame>),
    Poster(Vec<u32>),
    Black,
}

/// The poster drawn into a `w` x `h` black buffer (letterboxed per `fit`).
pub(crate) fn compose_poster(poster: &DecodedImage, fit: ObjectFit, pos: ((f32, f32), (f32, f32)), w: u32, h: u32) -> Vec<u32> {
    let mut buf = vec![BLACK; w as usize * h as usize];
    crate::paint::blit_image(poster, fit, pos, &mut buf, w, h, 0.0, 0.0, w as f32, h as f32, None);
    buf
}

/// Picks the source for a box of `rect` size.
pub(crate) fn pick_src(frame: Option<Arc<VideoFrame>>, poster: Option<&DecodedImage>, fit: ObjectFit, pos: ((f32, f32), (f32, f32)), rect: IRect) -> Src {
    if let Some(f) = frame
        && f.w == rect.2
        && f.h == rect.3
    {
        return Src::Frame(f);
    }
    match poster {
        Some(p) if p.width > 0 && !p.pixels.is_empty() => Src::Poster(compose_poster(p, fit, pos, rect.2, rect.3)),
        _ => Src::Black,
    }
}

/// Copies `src` (a `rect`-sized picture) into `dst` (a window buffer) over
/// the cells of `mask` (None = the whole of `vis`). `vis` is inside `rect`.
pub(crate) fn composite(src: &Src, rect: IRect, vis: IRect, mask: Option<&[bool]>, dst: &mut [u32], win_w: u32) {
    let rw = rect.2 as usize;
    for row in 0..vis.3 as usize {
        let y = vis.1 as usize + row;
        let sy = (y as i32 - rect.1) as usize;
        let d0 = y * win_w as usize + vis.0 as usize;
        let sx0 = (vis.0 - rect.0) as usize;
        let Some(drow) = dst.get_mut(d0..d0 + vis.2 as usize) else { continue };
        let mrow = mask.map(|m| &m[row * vis.2 as usize..(row + 1) * vis.2 as usize]);
        let srow: Option<&[u32]> = match src {
            Src::Frame(f) => Some(&f.px[sy * rw + sx0..sy * rw + sx0 + vis.2 as usize]),
            Src::Poster(p) => Some(&p[sy * rw + sx0..sy * rw + sx0 + vis.2 as usize]),
            Src::Black => None,
        };
        // Fast path: a row nothing covers is one memcpy.
        if mrow.is_none_or(|m| m.iter().all(|&b| b)) {
            match srow {
                Some(sr) => drow.copy_from_slice(sr),
                None => drow.fill(BLACK),
            }
            continue;
        }
        for (i, d) in drow.iter_mut().enumerate() {
            if mrow.is_some_and(|m| m[i]) {
                *d = srow.map_or(BLACK, |s| s[i]);
            }
        }
    }
}

// ------------------------------------------------------------ paint hooks

/// One `<video>` box seen during a paint.
pub(crate) struct PaintedBox {
    pub node: usize,
    /// The content box, window coordinates, unclipped.
    pub rect: IRect,
    /// The visible part (clipped to ancestors and the window).
    pub vis: Option<IRect>,
    /// Painted into an offscreen layer (mask-image): drawn black, frames not kept.
    pub layered: bool,
    pub fit: ObjectFit,
    pub pos: ((f32, f32), (f32, f32)),
    pub poster: Option<Arc<DecodedImage>>,
}

struct PaintCtx {
    ptr: usize,
    len: usize,
    boxes: Vec<PaintedBox>,
}

thread_local! {
    static PAINT: RefCell<Option<PaintCtx>> = const { RefCell::new(None) };
}

/// Starts collecting `<video>` boxes of a paint into the window buffer `pixels`.
pub(crate) fn begin_paint(pixels: &[u32]) {
    PAINT.with(|p| *p.borrow_mut() = Some(PaintCtx { ptr: pixels.as_ptr() as usize, len: pixels.len(), boxes: Vec::new() }));
}

pub(crate) fn end_paint() -> Vec<PaintedBox> {
    PAINT.with(|p| p.borrow_mut().take().map(|c| c.boxes).unwrap_or_default())
}

/// Called by `paint` for a video placeholder box. `x,y,w,h` is the content
/// box (float, screen space).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_box(
    placeholder: &DecodedImage,
    node: usize,
    fit: ObjectFit,
    pos: ((f32, f32), (f32, f32)),
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    clip: Option<(f32, f32, f32, f32)>,
    opacity_one: bool,
) {
    let rect: IRect = (x.round() as i32, y.round() as i32, w.round().max(0.0) as u32, h.round().max(0.0) as u32);
    let vis = clip_rect(rect, clip, win_w, win_h);
    let poster = placeholder.mask.clone();
    let main = PAINT.with(|p| p.borrow().as_ref().is_some_and(|c| c.ptr == pixels.as_ptr() as usize && c.len == pixels.len())) && opacity_one;
    let Some(v) = vis else {
        // QM1k1n1: laid out but clipped away or off-screen. Recorded so the backend keeps
        // decoding a playing element (YouTube parks it at top:-360px until `playing`).
        PAINT.with(|p| {
            if let Some(c) = p.borrow_mut().as_mut() {
                c.boxes.push(PaintedBox { node, rect, vis: None, layered: false, fit, pos, poster });
            }
        });
        return;
    };
    if main {
        fill(pixels, win_w, v, SENTINEL);
    } else {
        // No frame substitution for this paint (offscreen layer, opacity, or a
        // plain `paint` call): draw the picture straight away.
        let src = pick_src(frame_of(node), poster.as_deref(), fit, pos, rect);
        composite(&src, rect, v, None, pixels, win_w);
    }
    PAINT.with(|p| {
        if let Some(c) = p.borrow_mut().as_mut() {
            c.boxes.push(PaintedBox { node, rect, vis: Some(v), layered: !main, fit, pos, poster });
        }
    });
}

/// What the app keeps per visible video box between redraws.
pub(crate) struct VideoBoxInfo {
    pub node: usize,
    pub rect: IRect,
    pub vis: IRect,
    /// Overlay mask over `vis` (empty for a layered box).
    pub mask: Vec<bool>,
    /// Bounding rect of the visible (unobstructed) pixels.
    pub bbox: Option<IRect>,
    pub layered: bool,
    pub fit: ObjectFit,
    /// Kept so a bar-only redraw (QM1g2d) can put the picture back under the bar.
    pub poster: Option<Arc<DecodedImage>>,
    pub pos: ((f32, f32), (f32, f32)),
}

impl VideoBoxInfo {
    /// What the backend should render for this box.
    pub(crate) fn target(&self) -> VideoTarget {
        VideoTarget { w: self.rect.2, h: self.rect.3, fit: self.fit }
    }
    pub(crate) fn visible_pixels(&self) -> usize {
        self.mask.iter().filter(|&&m| m).count()
    }
}

/// After a complete paint: turns the sentinels into pixels and returns the
/// per-box info for the rect-only path.
pub(crate) fn finish(boxes: Vec<PaintedBox>, pixels: &mut [u32], win_w: u32) -> Vec<VideoBoxInfo> {
    let mut out = Vec::new();
    for b in boxes {
        let Some(vis) = b.vis else { continue };
        if b.layered {
            out.push(VideoBoxInfo { node: b.node, rect: b.rect, vis, mask: Vec::new(), bbox: Some(vis), layered: true, fit: b.fit, poster: b.poster, pos: b.pos });
            continue;
        }
        let mask = overlay_mask(pixels, win_w, vis);
        let src = pick_src(frame_of(b.node), b.poster.as_deref(), b.fit, b.pos, b.rect);
        composite(&src, b.rect, vis, Some(&mask), pixels, win_w);
        let bbox = mask_bbox(&mask, vis);
        out.push(VideoBoxInfo { node: b.node, rect: b.rect, vis, mask, bbox, layered: false, fit: b.fit, poster: b.poster, pos: b.pos });
    }
    out
}

/// The rect-only update: copies `frame` into `dst` over the box's overlay
/// mask. Returns the damaged rect, or None when the frame does not fit the
/// box (the caller then does a full redraw) or nothing is visible.
pub(crate) fn update_box(info: &VideoBoxInfo, frame: &Arc<VideoFrame>, dst: &mut [u32], win_w: u32) -> Option<IRect> {
    if info.layered || frame.w != info.rect.2 || frame.h != info.rect.3 {
        return None;
    }
    let bbox = info.bbox?;
    composite(&Src::Frame(frame.clone()), info.rect, info.vis, Some(&info.mask), dst, win_w);
    Some(bbox)
}

/// QM1g2d: puts the picture (newest frame, else poster, else black) back into the part of
/// the box inside `sub` (through the overlay mask), e.g. under a bar that is going away or
/// being repainted. Returns the rect touched.
pub(crate) fn recomposite(info: &VideoBoxInfo, sub: IRect, dst: &mut [u32], win_w: u32) -> Option<IRect> {
    if info.layered || info.mask.is_empty() {
        return None;
    }
    let (x0, y0) = (sub.0.max(info.vis.0), sub.1.max(info.vis.1));
    let x1 = (sub.0 + sub.2 as i32).min(info.vis.0 + info.vis.2 as i32);
    let y1 = (sub.1 + sub.3 as i32).min(info.vis.1 + info.vis.3 as i32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let v: IRect = (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32);
    let mut m = Vec::with_capacity(v.2 as usize * v.3 as usize);
    for y in y0..y1 {
        let o = (y - info.vis.1) as usize * info.vis.2 as usize + (x0 - info.vis.0) as usize;
        m.extend_from_slice(&info.mask[o..o + v.2 as usize]);
    }
    let src = pick_src(frame_of(info.node), info.poster.as_deref(), info.fit, info.pos, info.rect);
    composite(&src, info.rect, v, Some(&m), dst, win_w);
    Some(v)
}
