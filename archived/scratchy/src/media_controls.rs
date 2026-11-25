//! QM1g2d: scratchy's own `controls` bar for `<video controls>` / `<audio controls>`.
//!
//! The bar is UA chrome painted by scratchy (mpv's OSC stays off): a 32 px strip
//! along the bottom of the element's content box, drawn as ordinary pixels over the
//! video picture (through the video box's overlay mask, so content painted above the
//! video stays on top). Everything here is pure (geometry, hit-testing, time text,
//! key mapping, pixel drawing); the state it shows comes from the element core
//! (`media_el::MediaCore`) via `JsEngine::media_controls_state`, and the actions go
//! back through the element's own `play()` / `currentTime` / `muted` / `volume`
//! (`__scratchy_media_ui` in js_dom.rs) so page listeners see the normal events.
//! Time text uses a tiny built-in 5x7 digit font (no system font dependency).

use winit::keyboard::{Key, NamedKey};

use crate::media_video::{IRect, VideoBoxInfo};

pub(crate) const BAR_H: i32 = 32;
/// While playing the bar stays this long after the pointer or a key acted on the box.
pub(crate) const HIDE_AFTER_MS: u64 = 2000;
/// Redraw rate of the time text / seek thumb while playing (timeupdate rate).
pub(crate) const REFRESH_MS: u64 = 250;
const VOL_W: i32 = 60;
const BTN_W: i32 = 32;
const GLYPH_ADVANCE: i32 = 12;

/// What the bar reads from the element core.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ControlsState {
    pub paused: bool,
    pub current: f64,
    pub duration: f64,
    pub buffered: Vec<(f64, f64)>,
    pub volume: f64,
    pub muted: bool,
    pub ended: bool,
}

impl Default for ControlsState {
    fn default() -> Self {
        ControlsState { paused: true, current: 0.0, duration: f64::NAN, buffered: Vec::new(), volume: 1.0, muted: false, ended: false }
    }
}

/// What decides whether the bar must be repainted: the state at the rate the bar can
/// show it (time and buffered ends quantised to 250 ms) plus the visibility.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DrawKey {
    paused: bool,
    t4: i64,
    duration_bits: u64,
    buffered4: Vec<(i64, i64)>,
    volume_bits: u64,
    muted: bool,
    ended: bool,
    pub(crate) visible: bool,
    pub(crate) big: bool,
}

impl ControlsState {
    /// The big centred play button: paused, nothing played yet.
    pub(crate) fn big_button(&self) -> bool {
        self.paused && self.current <= 0.0 && !self.ended
    }
    pub(crate) fn key(&self, visible: bool) -> DrawKey {
        let q = |v: f64| if v.is_finite() { (v * 4.0).floor() as i64 } else { i64::MIN };
        DrawKey {
            paused: self.paused,
            t4: q(self.current),
            duration_bits: self.duration.to_bits(),
            buffered4: self.buffered.iter().map(|&(a, b)| (q(a), q(b))).collect(),
            volume_bits: self.volume.to_bits(),
            muted: self.muted,
            ended: self.ended,
            visible,
            big: self.big_button(),
        }
    }
}

/// `m:ss`, `h:mm:ss` from one hour, `--:--` for NaN / infinite / negative.
pub(crate) fn format_time(t: f64) -> String {
    if !t.is_finite() || t < 0.0 {
        return "--:--".to_string();
    }
    let s = t.floor() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// `current / duration`.
pub(crate) fn time_text(s: &ControlsState) -> String {
    format!("{} / {}", format_time(s.current), format_time(s.duration))
}

// ------------------------------------------------------------ geometry

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BarLayout {
    pub bar: IRect,
    pub play: IRect,
    /// Left edge of the time text, `None` when the box is too narrow for it.
    pub time_x: Option<i32>,
    pub seek: Option<IRect>,
    pub mute: Option<IRect>,
    pub vol: Option<IRect>,
}

/// The bar of a content box `rect` (window coordinates). `None` when the box is too
/// small to carry one. `time_chars` is the length of the time text.
pub(crate) fn layout(rect: IRect, time_chars: usize) -> Option<BarLayout> {
    let (rx, ry, rw, rh) = (rect.0, rect.1, rect.2 as i32, rect.3 as i32);
    if rw < 48 || rh < 20 {
        return None;
    }
    let bh = BAR_H.min(rh);
    let by = ry + rh - bh;
    let bar: IRect = (rx, by, rw as u32, bh as u32);
    let play: IRect = (rx, by, BTN_W as u32, bh as u32);
    let mut left = rx + BTN_W + 4;
    let time_x = (rw >= 160).then(|| {
        let x = left;
        left += time_chars as i32 * GLYPH_ADVANCE + 8;
        x
    });
    let mut right = rx + rw - 8;
    let vol = (rw >= 420).then(|| {
        let r: IRect = (right - VOL_W, by, VOL_W as u32, bh as u32);
        right -= VOL_W + 4;
        r
    });
    let mute = (rw >= 200).then(|| {
        let r: IRect = (right - 28, by, 28, bh as u32);
        right -= 32;
        r
    });
    let seek = (right - left >= 16).then(|| (left, by, (right - left) as u32, bh as u32));
    Some(BarLayout { bar, play, time_x, seek, mute, vol })
}

fn inside(r: IRect, x: i32, y: i32) -> bool {
    x >= r.0 && y >= r.1 && x < r.0 + r.2 as i32 && y < r.1 + r.3 as i32
}

/// Position of `x` along a track as 0..1 (the thumb centre runs between the track ends).
pub(crate) fn track_frac(track: IRect, x: i32) -> f64 {
    if track.2 == 0 {
        return 0.0;
    }
    ((x - track.0) as f64 / track.2 as f64).clamp(0.0, 1.0)
}

/// What a press on the bar hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Hit {
    Play,
    Seek(f64),
    Mute,
    Volume(f64),
    /// Elsewhere on the bar: consumed (UA chrome), does nothing.
    Bar,
}

pub(crate) fn hit(l: &BarLayout, x: i32, y: i32) -> Option<Hit> {
    if !inside(l.bar, x, y) {
        return None;
    }
    if inside(l.play, x, y) {
        return Some(Hit::Play);
    }
    if let Some(s) = l.seek.filter(|s| inside(*s, x, y)) {
        return Some(Hit::Seek(track_frac(s, x)));
    }
    if let Some(m) = l.mute.filter(|m| inside(*m, x, y)) {
        let _ = m;
        return Some(Hit::Mute);
    }
    if let Some(v) = l.vol.filter(|v| inside(*v, x, y)) {
        return Some(Hit::Volume(track_frac(v, x)));
    }
    Some(Hit::Bar)
}

/// An action on the element, performed through its own JS API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum UiOp {
    Toggle,
    /// Absolute `currentTime` in seconds.
    Seek(f64),
    Mute(bool),
    Volume(f64),
}

impl UiOp {
    /// `(op name, argument)` for `__scratchy_media_ui`.
    pub(crate) fn js(self) -> (&'static str, f64) {
        match self {
            UiOp::Toggle => ("toggle", 0.0),
            UiOp::Seek(t) => ("seek", t),
            UiOp::Mute(m) => ("mute", f64::from(u8::from(m))),
            UiOp::Volume(v) => ("volume", v),
        }
    }
}

/// The op a bar press asks for (`None` = nothing to do).
pub(crate) fn op_for_hit(h: Hit, s: &ControlsState) -> Option<UiOp> {
    match h {
        Hit::Play => Some(UiOp::Toggle),
        Hit::Seek(f) => (s.duration.is_finite() && s.duration > 0.0).then_some(UiOp::Seek(f * s.duration)),
        Hit::Mute => Some(UiOp::Mute(!s.muted)),
        Hit::Volume(f) => Some(UiOp::Volume(f)),
        Hit::Bar => None,
    }
}

/// D6 keys, only for a FOCUSED controls element: space toggles, left/right seek
/// -5 s/+5 s, up/down volume +-0.1.
pub(crate) fn key_op(key: &Key, s: &ControlsState) -> Option<UiOp> {
    let end = if s.duration.is_finite() { s.duration } else { f64::MAX };
    match key {
        Key::Named(NamedKey::Space) => Some(UiOp::Toggle),
        Key::Named(NamedKey::ArrowLeft) => Some(UiOp::Seek((s.current - 5.0).clamp(0.0, end))),
        Key::Named(NamedKey::ArrowRight) => Some(UiOp::Seek((s.current + 5.0).clamp(0.0, end))),
        Key::Named(NamedKey::ArrowUp) => Some(UiOp::Volume(((s.volume + 0.1) * 100.0).round() / 100.0).map_volume()),
        Key::Named(NamedKey::ArrowDown) => Some(UiOp::Volume(((s.volume - 0.1) * 100.0).round() / 100.0).map_volume()),
        _ => None,
    }
}

impl UiOp {
    fn map_volume(self) -> UiOp {
        match self {
            UiOp::Volume(v) => UiOp::Volume(v.clamp(0.0, 1.0)),
            o => o,
        }
    }
}

// ------------------------------------------------------------ drawing

/// A pixel sink limited to the visible, unobstructed part of the video box.
struct Canvas<'a> {
    dst: &'a mut [u32],
    win_w: u32,
    vis: IRect,
    mask: &'a [bool],
}

impl Canvas<'_> {
    fn put(&mut self, x: i32, y: i32, rgb: (u32, u32, u32), a: u32) {
        let v = self.vis;
        if x < v.0 || y < v.1 || x >= v.0 + v.2 as i32 || y >= v.1 + v.3 as i32 {
            return;
        }
        let mi = (y - v.1) as usize * v.2 as usize + (x - v.0) as usize;
        if !self.mask.get(mi).copied().unwrap_or(false) {
            return;
        }
        let Some(p) = self.dst.get_mut(y as usize * self.win_w as usize + x as usize) else { return };
        let inv = 255 - a;
        let r = (rgb.0 * a + ((*p >> 16) & 0xFF) * inv) / 255;
        let g = (rgb.1 * a + ((*p >> 8) & 0xFF) * inv) / 255;
        let b = (rgb.2 * a + (*p & 0xFF) * inv) / 255;
        *p = (r << 16) | (g << 8) | b;
    }
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: (u32, u32, u32), a: u32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, rgb, a);
            }
        }
    }
    fn disc(&mut self, cx: i32, cy: i32, r: i32, rgb: (u32, u32, u32), a: u32) {
        for yy in cy - r..=cy + r {
            for xx in cx - r..=cx + r {
                let (dx, dy) = (xx - cx, yy - cy);
                if dx * dx + dy * dy <= r * r {
                    self.put(xx, yy, rgb, a);
                }
            }
        }
    }
    /// A right-pointing triangle inside the `w` x `h` box at `(x, y)`.
    fn triangle(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: (u32, u32, u32), a: u32) {
        for yy in 0..h {
            let d = (2 * yy + 1 - h).abs(); // distance from the middle, in half rows
            let span = w * (h - d) / h;
            for xx in 0..span {
                self.put(x + xx, y + yy, rgb, a);
            }
        }
    }
}

const WHITE: (u32, u32, u32) = (255, 255, 255);
const BLACK: (u32, u32, u32) = (0, 0, 0);

/// 5x7 glyphs, one row per entry, bit 4 = leftmost column.
fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        _ => return None,
    })
}

fn draw_text(c: &mut Canvas, x: i32, y: i32, text: &str) {
    for (i, ch) in text.chars().enumerate() {
        let Some(g) = glyph(ch) else { continue };
        let gx = x + i as i32 * GLYPH_ADVANCE;
        for (row, bits) in g.iter().enumerate() {
            for col in 0..5 {
                if bits >> (4 - col) & 1 == 1 {
                    c.rect(gx + col * 2, y + row as i32 * 2, 2, 2, WHITE, 255);
                }
            }
        }
    }
}

fn draw_speaker(c: &mut Canvas, r: IRect, muted: bool) {
    let cy = r.1 + r.3 as i32 / 2;
    let x = r.0 + 4;
    c.rect(x, cy - 3, 4, 6, WHITE, 255);
    // the cone: widens to the right
    for i in 0..6 {
        c.rect(x + 4 + i, cy - 3 - i, 1, 6 + 2 * i, WHITE, 255);
    }
    let tip = x + 10;
    if muted {
        for i in 0..10 {
            c.rect(tip + 3 + i, cy - 5 + i, 2, 2, WHITE, 255);
            c.rect(tip + 3 + i, cy + 4 - i, 2, 2, WHITE, 255);
        }
    } else {
        for (rad, _) in [(6i32, 0), (10, 0)] {
            for yy in -rad..=rad {
                for xx in 1..=rad {
                    let d2 = xx * xx + yy * yy;
                    if d2 <= rad * rad && d2 > (rad - 2) * (rad - 2) {
                        c.put(tip - 4 + xx, cy + yy, WHITE, 255);
                    }
                }
            }
        }
    }
}

fn draw_track(c: &mut Canvas, t: IRect, frac: f64, buffered: Option<(&[(f64, f64)], f64)>) {
    let cy = t.1 + t.3 as i32 / 2;
    let (x0, w) = (t.0, t.2 as i32);
    c.rect(x0, cy - 2, w, 4, WHITE, 77);
    if let Some((ranges, dur)) = buffered
        && dur.is_finite()
        && dur > 0.0
    {
        for &(a, b) in ranges {
            let (xa, xb) = (x0 + (a / dur).clamp(0.0, 1.0).mul_add(w as f64, 0.0) as i32, x0 + (b / dur).clamp(0.0, 1.0).mul_add(w as f64, 0.0) as i32);
            c.rect(xa, cy - 2, xb - xa, 4, WHITE, 140);
        }
    }
    let px = x0 + (frac.clamp(0.0, 1.0) * w as f64) as i32;
    c.rect(x0, cy - 2, px - x0, 4, WHITE, 255);
    c.disc(px, cy, 6, WHITE, 255);
}

/// Paints the bar (when `visible`) and the big play button over the picture of `info`.
/// Only the pixels of the video box that no other content covers are touched.
pub(crate) fn paint(dst: &mut [u32], win_w: u32, info: &VideoBoxInfo, s: &ControlsState, visible: bool) {
    if info.layered || info.mask.is_empty() {
        return;
    }
    let mut c = Canvas { dst, win_w, vis: info.vis, mask: &info.mask };
    let rect = info.rect;
    if s.big_button() && rect.2 >= 80 && rect.3 >= 80 {
        let (cx, cy) = (rect.0 + rect.2 as i32 / 2, rect.1 + rect.3 as i32 / 2);
        c.disc(cx, cy, 28, BLACK, 153);
        c.triangle(cx - 8, cy - 14, 24, 28, WHITE, 255);
    }
    if !visible {
        return;
    }
    let text = time_text(s);
    let Some(l) = layout(rect, text.chars().count()) else { return };
    c.rect(l.bar.0, l.bar.1, l.bar.2 as i32, l.bar.3 as i32, BLACK, 153);
    let cy = l.bar.1 + l.bar.3 as i32 / 2;
    // play triangle / pause bars
    if s.paused {
        c.triangle(l.play.0 + 11, cy - 7, 12, 14, WHITE, 255);
    } else {
        c.rect(l.play.0 + 10, cy - 7, 4, 14, WHITE, 255);
        c.rect(l.play.0 + 18, cy - 7, 4, 14, WHITE, 255);
    }
    if let Some(x) = l.time_x {
        draw_text(&mut c, x, cy - 7, &text);
    }
    if let Some(t) = l.seek {
        let frac = if s.duration.is_finite() && s.duration > 0.0 { s.current / s.duration } else { 0.0 };
        draw_track(&mut c, t, frac, Some((&s.buffered, s.duration)));
    }
    if let Some(m) = l.mute {
        draw_speaker(&mut c, m, s.muted || s.volume <= 0.0);
    }
    if let Some(v) = l.vol {
        draw_track(&mut c, v, if s.muted { 0.0 } else { s.volume }, None);
    }
}
