//! The status bar's contents (KEYS.md "Status bar"), ported from scratchy's
//! `paint_status_bar` / `status_layout`: the mode, the page's URL (elided in the middle)
//! and the last message on the right; while the `:` line is open it takes the whole strip
//! with a caret and the dimmed ghost. cosmic-text shapes and rasterizes the text,
//! tiny-skia fills the strip, and the pixels go to the compositor as a memory buffer
//! drawn in C2's reserved strip.

use cosmic_text::{Attrs, Buffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Rectangle, Transform};
use tiny_skia::{Paint, Pixmap, Rect};

use crate::config::{
    GHOST_FG, STATUS_BAR_BG, STATUS_BAR_FG, STATUS_BAR_FONT_PX, STATUS_BAR_GAP, STATUS_BAR_MIN_URL_W, STATUS_BAR_MODE_FG, STATUS_BAR_PAD_X,
};
use crate::keys::BarView;

/// The strings the normal (non-`:`) strip shows.
#[derive(Debug, PartialEq)]
pub struct StatusLayout {
    pub mode: String,
    pub url: String,
    pub message: String,
}

/// Keeps the head of `text` and ends it with `…` so that it measures at most `max_w`.
pub fn elide_end(text: &str, max_w: f32, measure: &mut dyn FnMut(&str) -> f32) -> String {
    if measure(text) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let build = |keep: usize| chars[..keep].iter().collect::<String>() + "\u{2026}";
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure(&build(mid)) <= max_w {
            lo = mid
        } else {
            hi = mid - 1
        }
    }
    if lo == 0 && measure("\u{2026}") > max_w {
        return String::new();
    }
    build(lo)
}

/// Keeps the start (scheme + host) and the end (path tail) of `text` with `…` between,
/// measuring at most `max_w`; the head gets the larger half.
pub fn elide_middle(text: &str, max_w: f32, measure: &mut dyn FnMut(&str) -> f32) -> String {
    if measure(text) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let build = |keep: usize| -> String {
        let tail = keep / 2;
        let head = keep - tail;
        chars[..head].iter().collect::<String>() + "\u{2026}" + &chars[n - tail..].iter().collect::<String>()
    };
    let (mut lo, mut hi) = (0usize, n.saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure(&build(mid)) <= max_w {
            lo = mid
        } else {
            hi = mid - 1
        }
    }
    if lo == 0 && measure("\u{2026}") > max_w {
        return String::new();
    }
    build(lo)
}

/// Pure layout of the normal strip: the mode is fixed; a message gets the width it needs,
/// at most `avail - mode - gap - gap - min_url`; the URL gets the rest, elided in the
/// middle; with no message it gets all the room. `measure` is the text advance.
pub fn status_layout(
    avail: f32,
    gap: f32,
    min_url_w: f32,
    mode: &str,
    url: &str,
    message: &str,
    measure: &mut dyn FnMut(&str) -> f32,
) -> StatusLayout {
    let mode_w = measure(mode);
    let rest = (avail - mode_w - gap).max(0.0);
    let mut msg = String::new();
    let mut msg_w = 0.0;
    if !message.is_empty() {
        let cap = (rest - gap - min_url_w).max(0.0);
        msg = elide_end(message, cap, measure);
        msg_w = measure(&msg);
    }
    let url_max = if msg.is_empty() { rest } else { (rest - msg_w - gap).max(0.0) };
    StatusLayout {
        mode: mode.to_string(),
        url: elide_middle(url, url_max, measure),
        message: msg,
    }
}

fn rgb(c: (u8, u8, u8)) -> TextColor {
    TextColor::rgb(c.0, c.1, c.2)
}

/// Draws the status bar into pixmaps; keeps the font system (loaded once, on first use).
pub struct Painter {
    fonts: FontSystem,
    swash: SwashCache,
    /// C6b: the scale of the strip being painted (sizes in config.rs are logical px).
    scale: f32,
}

impl Painter {
    pub fn new() -> Painter {
        Painter {
            fonts: FontSystem::new(),
            swash: SwashCache::new(),
            scale: 1.0,
        }
    }

    /// Whether any font was found (CI machines may have none; text then draws nothing).
    #[cfg(test)]
    pub fn has_fonts(&self) -> bool {
        self.fonts.db().faces().next().is_some()
    }

    fn shaped(&mut self, text: &str, h: f32, color: TextColor) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(STATUS_BAR_FONT_PX * self.scale, h));
        buffer.set_size(None, Some(h));
        buffer.set_wrap(Wrap::None);
        buffer.set_text(text, &Attrs::new().color(color), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    /// The advance of `text`, px of the strip being painted (physical at a scale).
    pub fn measure(&mut self, text: &str) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let b = self.shaped(text, 24.0 * self.scale, rgb(STATUS_BAR_FG));
        b.layout_runs().next().map_or(0.0, |r| r.line_w)
    }

    /// Draws `text` at `x` (clipped at `x + max_w`); returns its width.
    fn text(&mut self, pm: &mut Pixmap, text: &str, x: f32, max_w: f32, color: TextColor) -> f32 {
        if text.is_empty() || max_w <= 0.0 {
            return 0.0;
        }
        let h = pm.height() as f32;
        let mut buffer = self.shaped(text, h, color);
        let w = buffer.layout_runs().next().map_or(0.0, |r| r.line_w);
        let clip = x + max_w;
        let mut paint = Paint {
            anti_alias: false,
            ..Paint::default()
        };
        buffer.draw(&mut self.fonts, &mut self.swash, color, |gx, gy, gw, gh, c| {
            let px = x + gx as f32;
            if c.a() == 0 || px + gw as f32 > clip + 0.5 {
                return;
            }
            paint.set_color_rgba8(c.r(), c.g(), c.b(), c.a());
            if let Some(r) = Rect::from_xywh(px, gy as f32, gw as f32, gh as f32) {
                pm.fill_rect(r, &paint, tiny_skia::Transform::identity(), None);
            }
        });
        w.min(max_w)
    }

    /// The x of the caret before byte `target` of `text` drawn at 0.
    fn caret_x(&mut self, text: &str, target: usize) -> f32 {
        let b = self.shaped(text, 24.0 * self.scale, rgb(STATUS_BAR_FG));
        b.layout_runs()
            .next()
            .map(|run| {
                run.glyphs
                    .iter()
                    .find(|g| g.start >= target)
                    .map(|g| g.x)
                    .unwrap_or_else(|| run.glyphs.last().map(|g| g.x + g.w).unwrap_or(0.0))
            })
            .unwrap_or(0.0)
    }

    /// The whole strip for `view`, `w` x `h` px (premultiplied RGBA), at scale 1.
    #[cfg(test)]
    pub fn paint(&mut self, view: &BarView, w: u32, h: u32) -> Option<Pixmap> {
        self.paint_scaled(view, w, h, 1.0)
    }

    /// The whole strip for `view`, `w` x `h` physical px at `scale` (C6b): the font size,
    /// paddings, gaps and caret are the logical sizes times `scale`, so text is rasterized
    /// at the output's resolution, not stretched.
    pub fn paint_scaled(&mut self, view: &BarView, w: u32, h: u32, scale: f32) -> Option<Pixmap> {
        self.scale = scale;
        let (pad, gap, min_url) = (STATUS_BAR_PAD_X * scale, STATUS_BAR_GAP * scale, STATUS_BAR_MIN_URL_W * scale);
        let mut pm = Pixmap::new(w.max(1), h.max(1))?;
        pm.fill(tiny_skia::Color::from_rgba8(STATUS_BAR_BG.0, STATUS_BAR_BG.1, STATUS_BAR_BG.2, 255));
        let avail = (w as f32 - 2.0 * pad).max(0.0);
        let fg = rgb(STATUS_BAR_FG);
        if let Some((prompt, text, cursor)) = &view.line {
            let display = format!("{prompt}{text}");
            let text_w = self.text(&mut pm, &display, pad, avail, fg);
            let caret = self.caret_x(&display, cursor + prompt.len_utf8());
            let mut paint = Paint::default();
            paint.set_color_rgba8(STATUS_BAR_FG.0, STATUS_BAR_FG.1, STATUS_BAR_FG.2, 255);
            if let Some(r) = Rect::from_xywh(pad + caret, 3.0 * scale, (1.5 * scale).round().max(1.0), (h as f32 - 6.0 * scale).max(1.0)) {
                pm.fill_rect(r, &paint, tiny_skia::Transform::identity(), None);
            }
            if !view.ghost.is_empty() {
                let x = pad + text_w;
                self.text(&mut pm, &view.ghost, x, (avail - text_w).max(0.0), rgb(GHOST_FG));
            }
            return Some(pm);
        }
        let lay = status_layout(
            avail,
            gap,
            min_url,
            &view.mode,
            &view.url,
            &view.message,
            &mut |t| self.measure(t),
        );
        let mode_w = self.text(&mut pm, &lay.mode, pad, avail, rgb(STATUS_BAR_MODE_FG));
        let url_x = pad + mode_w + gap;
        let msg_w = self.measure(&lay.message);
        let url_max = (w as f32 - pad - msg_w - if msg_w > 0.0 { gap } else { 0.0 } - url_x).max(0.0);
        self.text(&mut pm, &lay.url, url_x, url_max + 0.5, fg);
        if msg_w > 0.0 {
            let x = w as f32 - pad - msg_w;
            self.text(&mut pm, &lay.message, x, msg_w + 0.5, fg);
        }
        Some(pm)
    }
}

/// The bar as the compositor draws it: a memory buffer redrawn when its view or size changes.
pub struct Bar {
    painter: Option<Painter>,
    pub buffer: MemoryRenderBuffer,
    drawn: Option<(BarView, u32, u32, u32)>,
}

impl Bar {
    pub fn new() -> Bar {
        Bar {
            painter: None,
            buffer: MemoryRenderBuffer::new(Fourcc::Abgr8888, (1, 1), 1, Transform::Normal, None),
            drawn: None,
        }
    }

    /// Redraws the buffer if `view`, the size (physical px) or the scale changed.
    pub fn update(&mut self, view: &BarView, w: u32, h: u32, scale: f64) {
        let s120 = (scale * 120.0).round() as u32;
        if self.drawn.as_ref().is_some_and(|(v, dw, dh, ds)| v == view && *dw == w && *dh == h && *ds == s120) {
            return;
        }
        let painter = self.painter.get_or_insert_with(Painter::new);
        let Some(pm) = painter.paint_scaled(view, w, h, scale as f32) else { return };
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let mut ctx = self.buffer.render();
        ctx.resize((pw, ph));
        let _ = ctx.draw(|mem| {
            // tiny-skia's premultiplied RGBA bytes are DRM's ABGR8888 (little endian).
            mem.copy_from_slice(pm.data());
            Ok::<_, ()>(vec![Rectangle::from_size((pw, ph).into())])
        });
        ctx.update_opaque_regions(Some(vec![Rectangle::from_size((pw, ph).into())]));
        drop(ctx);
        self.drawn = Some((view.clone(), w, h, s120));
    }
}
