//! QSB1: the scrollbar's geometry, hit test and drag maths as pure functions
//! (no `App`, no layout), shared by painting (`paint::paint_scrollbar`, the
//! viewport bar in `App::redraw`) and by the pointer handling in `app.rs`.
//!
//! A bar is an 8 px overlay track along the right edge of a box (the window or an
//! `overflow: scroll|auto` region). The thumb length is `track * client / content`
//! (at least 16 px) and its offset is `scroll / max_scroll * (track - thumb)`.

// Tunables live in `config.rs` (QCONF1).
pub(crate) use crate::config::{MIN_THUMB_H, TRACK_W};

/// Where on a bar a press landed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BarHit {
    Thumb,
    /// On the track before (above) the thumb: page up.
    TrackBefore,
    /// On the track after (below) the thumb: page down.
    TrackAfter,
}

/// A box's border box `(x, y, w, h)`; the track is its right `TRACK_W` px.
/// `None` when the box is too small to hold a bar.
pub(crate) fn track_rect(bx: f32, by: f32, bw: f32, bh: f32) -> Option<(f32, f32, f32, f32)> {
    (bw >= TRACK_W && bh > 0.0).then_some((bx + bw - TRACK_W, by, TRACK_W, bh))
}

/// `(offset, length)` of the thumb inside a track `track_h` tall, for content
/// `content_h` seen through `client_h` at `scroll`. Without overflow the thumb
/// fills the track.
pub(crate) fn thumb(track_h: f32, content_h: f32, client_h: f32, scroll: f32) -> (f32, f32) {
    let max_scroll = (content_h - client_h).max(0.0);
    let ratio = if content_h > 0.0 { (client_h / content_h).clamp(0.0, 1.0) } else { 1.0 };
    let len = (track_h * ratio).clamp(MIN_THUMB_H.min(track_h), track_h);
    let off = if max_scroll > 0.0 { scroll.clamp(0.0, max_scroll) / max_scroll * (track_h - len) } else { 0.0 };
    (off, len)
}

/// Hit test of `(px, py)` against the bar of the box `rect` (`(x, y, w, h)`).
/// `None` outside the track or when there is nothing to scroll.
pub(crate) fn hit(rect: (f32, f32, f32, f32), content_h: f32, client_h: f32, scroll: f32, px: f32, py: f32) -> Option<BarHit> {
    if content_h - client_h <= 0.0 {
        return None;
    }
    let (tx, ty, tw, th) = track_rect(rect.0, rect.1, rect.2, rect.3)?;
    if px < tx || px >= tx + tw || py < ty || py >= ty + th {
        return None;
    }
    let (off, len) = thumb(th, content_h, client_h, scroll);
    let rel = py - ty;
    Some(if rel < off {
        BarHit::TrackBefore
    } else if rel >= off + len {
        BarHit::TrackAfter
    } else {
        BarHit::Thumb
    })
}

/// Content px scrolled per pointer px while dragging the thumb: the content/track
/// ratio, `max_scroll / (track - thumb)`. `0.0` when the thumb fills the track.
pub(crate) fn drag_ratio(track_h: f32, content_h: f32, client_h: f32) -> f32 {
    let max_scroll = (content_h - client_h).max(0.0);
    let (_, len) = thumb(track_h, content_h, client_h, 0.0);
    let free = track_h - len;
    if free > 0.0 { max_scroll / free } else { 0.0 }
}

/// The scroll position after the pointer moved from `start_ptr` to `ptr` (y) while
/// dragging a thumb grabbed at scroll `start_scroll`. Measured from the press, so
/// the thumb keeps its offset under the pointer. Clamped to `0..=max_scroll`.
pub(crate) fn drag_scroll(start_scroll: f32, start_ptr: f32, ptr: f32, ratio: f32, max_scroll: f32) -> f32 {
    (start_scroll + (ptr - start_ptr) * ratio).clamp(0.0, max_scroll.max(0.0))
}

/// The scroll position after a press on the track: a page up or down (`page` px),
/// clamped. `Thumb` leaves it unchanged.
pub(crate) fn page_target(hit: BarHit, scroll: f32, page: f32, max_scroll: f32) -> f32 {
    let t = match hit {
        BarHit::TrackBefore => scroll - page,
        BarHit::TrackAfter => scroll + page,
        BarHit::Thumb => scroll,
    };
    t.clamp(0.0, max_scroll.max(0.0))
}

/// A window bar: its box, the content height and the visible height.
pub(crate) type ViewportBar = ((f32, f32, f32, f32), f32, f32);
