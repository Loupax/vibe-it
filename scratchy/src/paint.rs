use super::*;


thread_local! {
    /// Accumulated `opacity` of the box `paint` is currently painting
    /// (Q17); `blend` scales every source alpha by it.
    pub(crate) static PAINT_OPACITY: Cell<f32> = const { Cell::new(1.0) };
}

pub(crate) fn blend(existing: u32, color: TextColor) -> u32 {
    let op = PAINT_OPACITY.with(|o| o.get());
    let a = if op >= 1.0 { color.a() as u32 } else { (color.a() as f32 * op).round() as u32 };
    if a == 255 {
        return (color.r() as u32) << 16 | (color.g() as u32) << 8 | (color.b() as u32);
    }
    let er = (existing >> 16) & 0xFF;
    let eg = (existing >> 8) & 0xFF;
    let eb = existing & 0xFF;
    let inv = 255 - a;
    let r = (color.r() as u32 * a + er * inv) / 255;
    let g = (color.g() as u32 * a + eg * inv) / 255;
    let b = (color.b() as u32 * a + eb * inv) / 255;
    (r << 16) | (g << 8) | b
}

/// How far `lighten`/`darken` blend toward white/black — CSS2.1 §8.5.3
/// doesn't mandate one exact ratio for a bevel border's two-tone
/// shading, so this picks a single reasonable, clearly-visible value
/// and applies it consistently both directions (`Groove`/`Ridge`/
/// `Inset`/`Outset` — see `bevel_colors`).
pub(crate) const BEVEL_RATIO: f32 = 0.5;

/// Blends `c` `ratio` of the way toward white, per channel, keeping its
/// own alpha unchanged. No existing color-lightening helper was found
/// anywhere in this file or in cosmic-text's own `Color`/`Attrs` API
/// (checked before writing this) — `blend` above is a src-over alpha
/// compositor, a different operation entirely.
pub(crate) fn lighten(c: TextColor, ratio: f32) -> TextColor {
    let mix = |ch: u8| -> u8 { (ch as f32 + (255.0 - ch as f32) * ratio).round().clamp(0.0, 255.0) as u8 };
    TextColor::rgba(mix(c.r()), mix(c.g()), mix(c.b()), c.a())
}

/// Blends `c` `ratio` of the way toward black, per channel, keeping its
/// own alpha unchanged — the darkening counterpart to `lighten`.
pub(crate) fn darken(c: TextColor, ratio: f32) -> TextColor {
    let mix = |ch: u8| -> u8 { (ch as f32 * (1.0 - ratio)).round().clamp(0.0, 255.0) as u8 };
    TextColor::rgba(mix(c.r()), mix(c.g()), mix(c.b()), c.a())
}

/// Intersects `(x, y, w, h)` against `clip` (`None` = unclipped, passed
/// through unchanged) — `w`/`h` can come back `<= 0.0` for a rect
/// entirely outside the clip, which `fill_rect`'s own existing bounds
/// checks already treat as "paint nothing" (same as a rect entirely
/// outside the window).
pub(crate) fn intersect_clip(x: f32, y: f32, w: f32, h: f32, clip: Option<(f32, f32, f32, f32)>) -> (f32, f32, f32, f32) {
    let Some((cx, cy, cw, ch)) = clip else {
        return (x, y, w, h);
    };
    let x0 = x.max(cx);
    let y0 = y.max(cy);
    let x1 = (x + w).min(cx + cw);
    let y1 = (y + h).min(cy + ch);
    (x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// `clip` is the nearest ancestor's `overflow: hidden` content-box bounds
/// (screen space, post-scroll), same as `paint_buffer`'s own — applied
/// once here via `intersect_clip`, so every caller (directly, or through
/// `fill_rounded_rect`/`fill_rounded_ring`/`paint_border_edge`, all the
/// way up to `paint_box_decoration`) gets clipping for free without
/// needing its own `intersect_clip` call. `None` behaves exactly as
/// before this parameter existed (unclipped, window bounds only).
pub(crate) fn fill_rect(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: TextColor,
    clip: Option<(f32, f32, f32, f32)>,
) {
    if color.a() == 0 {
        return;
    }
    let (x, y, w, h) = intersect_clip(x, y, w, h, clip);
    let x0 = x.max(0.0) as i64;
    let y0 = y.max(0.0) as i64;
    let x1 = ((x + w).min(win_w as f32)) as i64;
    let y1 = ((y + h).min(win_h as f32)) as i64;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for py in y0..y1 {
        for px in x0..x1 {
            let idx = (py as u32 * win_w + px as u32) as usize;
            pixels[idx] = blend(pixels[idx], color);
        }
    }
}

/// Fills a rect with its 4 corners rounded by `radius`, hard-edged (no
/// anti-aliasing — matches `fill_rect`'s own fidelity level, so this
/// isn't a visual downgrade from how every other shape here already
/// paints). Row by row: each row is one horizontal span, its full width
/// except within `radius` of the top/bottom edge, where it's inset by
/// how far that row sits from the nearest corner's circular boundary.
/// How far a row at vertical distance `dy` from its nearest top/bottom
/// edge should be inset from each side, for a rect corner-rounded by
/// `r` — shared by `fill_rounded_rect` and `fill_rounded_ring` (the
/// ring needs this computed twice per row, once for its outer boundary
/// and once for its inner one, each with its own radius). `r <= 0`
/// (including a clamped-to-zero inner radius when the border is wider
/// than the corner radius itself) naturally degrades to `0.0` for every
/// `dy`, i.e. a plain square edge — no separate no-rounding case needed.
pub(crate) fn corner_inset(dy: f32, r: f32) -> f32 {
    if dy < r {
        r - (r * r - (r - dy).max(0.0).powi(2)).max(0.0).sqrt()
    } else {
        0.0
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_rounded_rect(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    color: TextColor,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    if r <= 0.0 {
        fill_rect(pixels, win_w, win_h, x, y, w, h, color, clip);
        return;
    }
    let y0 = y.round() as i32;
    let y1 = (y + h).round() as i32;
    for row in y0..y1 {
        let dy = (row as f32 - y).min((y + h) - row as f32 - 1.0);
        let inset = corner_inset(dy, r);
        fill_rect(pixels, win_w, win_h, x + inset, row as f32, (w - 2.0 * inset).max(0.0), 1.0, color, clip);
    }
}

/// Paints a rounded *border ring* — the area between an outer rounded
/// rect (`x, y, w, h, radius`) and an inner one inset by `border_width`
/// on every side (inner radius `(radius - border_width).max(0.0)`, same
/// "ring narrows along with the border" real CSS shape). Unlike the
/// "paint a bigger solid shape, then a smaller one on top" trick
/// `outline`/`box-shadow` get away with, that doesn't work here: nothing
/// guarantees the interior is opaque to safely paint over (no
/// `background-color`), so this tests each row's outer *and* inner
/// boundary directly instead (see `ComputedStyle::border_radius`'s own
/// former caveat, now resolved by this function). Each row is at most
/// two horizontal spans (left and right edge strips) once it's within
/// the inner rect's own vertical extent, or one full-width span (a
/// plain top/bottom edge strip) outside it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_rounded_ring(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    border_width: f32,
    color: TextColor,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let r_outer = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let (inner_x, inner_y) = (x + border_width, y + border_width);
    let (inner_w, inner_h) = (w - 2.0 * border_width, h - 2.0 * border_width);
    let r_inner = (radius - border_width).max(0.0).min(inner_w / 2.0).min(inner_h / 2.0).max(0.0);
    let has_hole = inner_w > 0.0 && inner_h > 0.0;

    let y0 = y.round() as i32;
    let y1 = (y + h).round() as i32;
    for row in y0..y1 {
        let dy_outer = (row as f32 - y).min((y + h) - row as f32 - 1.0);
        let outer_inset = corner_inset(dy_outer, r_outer);
        let (outer_left, outer_right) = (x + outer_inset, x + w - outer_inset);

        let row_f = row as f32;
        let within_inner_rows = has_hole && row_f >= inner_y && row_f < inner_y + inner_h;
        if !within_inner_rows {
            fill_rect(pixels, win_w, win_h, outer_left, row_f, (outer_right - outer_left).max(0.0), 1.0, color, clip);
            continue;
        }
        let dy_inner = (row_f - inner_y).min((inner_y + inner_h) - row_f - 1.0);
        let inner_inset = corner_inset(dy_inner, r_inner);
        let (inner_left, inner_right) = (inner_x + inner_inset, inner_x + inner_w - inner_inset);
        fill_rect(pixels, win_w, win_h, outer_left, row_f, (inner_left - outer_left).max(0.0), 1.0, color, clip);
        fill_rect(pixels, win_w, win_h, inner_right, row_f, (outer_right - inner_right).max(0.0), 1.0, color, clip);
    }
}

/// Paints one border edge's rect — same `(x, y, w, h)` rectangle
/// `fill_rect` itself takes (whichever of `w`/`h` is larger is the edge's
/// length *along* the box side; the smaller is its thickness *across*,
/// i.e. `border_width` — true for all 4 edges a `paint_box_decoration`
/// border call ever makes, so no separate orientation flag is needed).
/// `Solid` is one plain `fill_rect`, same as before this property
/// existed; `Dashed`/`Dotted`/`Double` instead tile smaller fills along
/// the edge's length. `None` never reaches here (`paint_box_decoration`
/// skips the whole border when it sees `BorderStyle::None`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_border_edge(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    style: BorderStyle,
    color: TextColor,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let horizontal = w >= h;
    let (along, across) = if horizontal { (w, h) } else { (h, w) };
    match style {
        BorderStyle::None => {}
        BorderStyle::Solid => fill_rect(pixels, win_w, win_h, x, y, w, h, color, clip),
        BorderStyle::Dashed => {
            let dash = (across * 3.0).max(2.0);
            let mut pos = 0.0;
            while pos < along {
                let seg = dash.min(along - pos);
                if horizontal {
                    fill_rect(pixels, win_w, win_h, x + pos, y, seg, across, color, clip);
                } else {
                    fill_rect(pixels, win_w, win_h, x, y + pos, across, seg, color, clip);
                }
                pos += dash * 2.0;
            }
        }
        BorderStyle::Dotted => {
            let dot = across.max(1.0);
            let mut pos = 0.0;
            while pos < along {
                let seg = dot.min(along - pos);
                if horizontal {
                    fill_rect(pixels, win_w, win_h, x + pos, y, seg, across, color, clip);
                } else {
                    fill_rect(pixels, win_w, win_h, x, y + pos, across, seg, color, clip);
                }
                pos += dot * 2.0;
            }
        }
        BorderStyle::Double => {
            let stripe = (across / 3.0).max(1.0);
            if horizontal {
                fill_rect(pixels, win_w, win_h, x, y, w, stripe, color, clip);
                fill_rect(pixels, win_w, win_h, x, y + h - stripe, w, stripe, color, clip);
            } else {
                fill_rect(pixels, win_w, win_h, x, y, stripe, h, color, clip);
                fill_rect(pixels, win_w, win_h, x + w - stripe, y, stripe, h, color, clip);
            }
        }
        BorderStyle::Groove | BorderStyle::Ridge | BorderStyle::Inset | BorderStyle::Outset => {
            // Never actually reached: `paint_box_decoration` always
            // routes these 4 bevel styles through `paint_border`
            // instead, which calls this function once per already-
            // light/dark-shaded sub-stripe/edge with `BorderStyle::Solid`
            // — see its own docs. Kept as a defensive plain-solid
            // fallback (not folded into a `_ =>` catch-all) so adding a
            // future real `BorderStyle` variant still gets a compile
            // error here, not a silently-wrong paint.
            fill_rect(pixels, win_w, win_h, x, y, w, h, color, clip)
        }
    }
}

/// Lighter/darker variants of a bevel border's own declared color — see
/// `lighten`/`darken` and `BEVEL_RATIO`. `None` for every non-bevel
/// `BorderStyle` (nothing to compute; `paint_border` falls back to a
/// plain flat-colored edge for those).
pub(crate) fn bevel_colors(style: BorderStyle, base: TextColor) -> Option<(TextColor, TextColor)> {
    match style {
        BorderStyle::Groove | BorderStyle::Ridge | BorderStyle::Inset | BorderStyle::Outset => {
            Some((lighten(base, BEVEL_RATIO), darken(base, BEVEL_RATIO)))
        }
        _ => None,
    }
}

/// True when pixel centre `(px, py)` lies inside the rect `(rx, ry, rw, rh)`
/// whose corners are elliptical with radii `radii` (tl, tr, br, bl).
pub(crate) fn in_rounded_rect(px: f32, py: f32, rx: f32, ry: f32, rw: f32, rh: f32, radii: [(f32, f32); 4]) -> bool {
    if px < rx || px >= rx + rw || py < ry || py >= ry + rh {
        return false;
    }
    let corners = [
        (rx + radii[0].0, ry + radii[0].1, px < rx + radii[0].0 && py < ry + radii[0].1, radii[0]),
        (rx + rw - radii[1].0, ry + radii[1].1, px >= rx + rw - radii[1].0 && py < ry + radii[1].1, radii[1]),
        (rx + rw - radii[2].0, ry + rh - radii[2].1, px >= rx + rw - radii[2].0 && py >= ry + rh - radii[2].1, radii[2]),
        (rx + radii[3].0, ry + rh - radii[3].1, px < rx + radii[3].0 && py >= ry + rh - radii[3].1, radii[3]),
    ];
    for (cx, cy, active, (rxr, ryr)) in corners {
        if active && rxr > 0.0 && ryr > 0.0 {
            let (dx, dy) = ((px - cx) / rxr, (py - cy) / ryr);
            if dx * dx + dy * dy > 1.0 {
                return false;
            }
        }
    }
    true
}

/// Paints a border whose four sides differ (width, color or style),
/// per pixel, hard-edged like the rest of the painters. Each border-band
/// pixel belongs to the side it is proportionally closest to
/// (distance / that side's width), which yields the diagonal mitered
/// joins real CSS draws at corners between unequal sides; a zero-width
/// side never claims pixels. The outer edge is rounded by `radius`
/// (clamped), the inner (padding) edge by `radius - width` per axis.
/// Each side's style is evaluated per pixel: solid, dashed/dotted
/// (along the side), double (outer/inner thirds) and the four bevel
/// styles (light/dark whole edge, or split halves for groove/ridge).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_per_side_border(
    s: &ComputedStyle,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let [bt, br, bb, bl] = s.border_width;
    let widths = [bt, br, bb, bl];
    let visible = |i: usize| widths[i] > 0.0 && s.border_style[i] != BorderStyle::None && s.border_color[i].is_some();
    if !(0..4).any(visible) {
        return;
    }
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let outer = [(r, r); 4];
    let (iw, ih) = (w - bl - br, h - bt - bb);
    let has_hole = iw > 0.0 && ih > 0.0;
    let clampr = |rx: f32, ry: f32| ((rx.max(0.0)).min(iw / 2.0).max(0.0), (ry.max(0.0)).min(ih / 2.0).max(0.0));
    let inner = [clampr(r - bl, r - bt), clampr(r - br, r - bt), clampr(r - br, r - bb), clampr(r - bl, r - bb)];
    let y0 = y.floor() as i32;
    let y1 = (y + h).ceil() as i32;
    for row in y0..y1 {
        let py = row as f32 + 0.5;
        let in_corner_rows = py < y + r || py >= y + h - r;
        let in_mid_rows = has_hole && py >= y + bt && py < y + h - bb && !in_corner_rows;
        // Middle rows only touch the left/right strips.
        let spans: [(i32, i32); 2] = if in_mid_rows {
            [(x.floor() as i32, (x + bl).ceil() as i32), ((x + w - br).floor() as i32, (x + w).ceil() as i32)]
        } else {
            [(x.floor() as i32, (x + w).ceil() as i32), (0, 0)]
        };
        for (c0, c1) in spans {
            for col in c0..c1 {
                let px = col as f32 + 0.5;
                if !in_rounded_rect(px, py, x, y, w, h, outer) {
                    continue;
                }
                if has_hole && in_rounded_rect(px, py, x + bl, y + bt, iw, ih, inner) {
                    continue;
                }
                let dist = [py - y, x + w - px, y + h - py, px - x];
                let mut side = None;
                let mut best = f32::INFINITY;
                for i in 0..4 {
                    if widths[i] > 0.0 {
                        let n = dist[i] / widths[i];
                        if n < best {
                            best = n;
                            side = Some(i);
                        }
                    }
                }
                let Some(i) = side else { continue };
                if !visible(i) {
                    continue;
                }
                let along = if i % 2 == 0 { px - x } else { py - y };
                if let Some(c) = border_pixel_color(s.border_style[i], s.border_color[i].unwrap(), i, widths[i], dist[i], along) {
                    fill_rect(pixels, win_w, win_h, col as f32, row as f32, 1.0, 1.0, c, clip);
                }
            }
        }
    }
}

/// Colour of one border pixel on side `side` (0=top,1=right,2=bottom,
/// 3=left) of width `bw`, `across` px in from the outer edge and `along`
/// px along the side; `None` means a gap (dash/dot/double middle).
pub(crate) fn border_pixel_color(style: BorderStyle, color: TextColor, side: usize, bw: f32, across: f32, along: f32) -> Option<TextColor> {
    let tl = side == 0 || side == 3;
    match style {
        BorderStyle::None => None,
        BorderStyle::Solid => Some(color),
        BorderStyle::Dashed => {
            let dash = (bw * 3.0).max(2.0);
            (along.rem_euclid(dash * 2.0) < dash).then_some(color)
        }
        BorderStyle::Dotted => {
            let dot = bw.max(1.0);
            (along.rem_euclid(dot * 2.0) < dot).then_some(color)
        }
        BorderStyle::Double => {
            let stripe = (bw / 3.0).max(1.0);
            (across < stripe || across >= bw - stripe).then_some(color)
        }
        BorderStyle::Inset | BorderStyle::Outset => {
            let (light, dark) = bevel_colors(style, color)?;
            let dark_side = (style == BorderStyle::Inset) == tl;
            Some(if dark_side { dark } else { light })
        }
        BorderStyle::Groove | BorderStyle::Ridge => {
            let (light, dark) = bevel_colors(style, color)?;
            let half = (bw / 2.0).max(0.5);
            let outer_half = across < half;
            // Groove: top/left outer-dark inner-light, bottom/right the reverse.
            let dark_here = (style == BorderStyle::Groove) == (tl == outer_half);
            Some(if dark_here { dark } else { light })
        }
    }
}

/// Paints all 4 edges of a border rect `(x, y, w, h)` with thickness
/// `bw`. Every style but the 4 bevel ones (`Groove`/`Ridge`/`Inset`/
/// `Outset`) just delegates to `paint_border_edge` 4 times with one flat
/// `color`, unchanged from before this function existed.
///
/// The bevel styles need per-edge (and, for `Groove`/`Ridge`, per-half-
/// edge) color, computed from `bevel_colors`: for every edge, "outer"
/// names the half/whole farther from this box's own content (the top of
/// the top edge, left of the left edge, bottom of the bottom edge, right
/// of the right edge) and "inner" the half closer to it.
/// - `Inset`/`Outset` paint each *whole* edge one color: `Inset`'s
///   top/left are uniformly dark and bottom/right uniformly light (the
///   box reads as pressed in, lit from the top-left); `Outset` is the
///   reverse.
/// - `Groove`/`Ridge` additionally split *each* edge's own thickness
///   into an outer and inner half (`half`, clamped to at least `0.5px`
///   so a 1px bevel border still shows two distinct rows/columns), so a
///   single edge itself shows a crease: `Groove`'s top/left edges are
///   outer-dark/inner-light and bottom/right outer-light/inner-dark
///   (the same "carved in" light direction as `Inset`, just applied
///   within each edge rather than across the whole edge); `Ridge` is
///   the reverse of `Groove`, edge-for-edge.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_border(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    bw: f32,
    style: BorderStyle,
    color: TextColor,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let Some((light, dark)) = bevel_colors(style, color) else {
        paint_border_edge(pixels, win_w, win_h, x, y, w, bw, style, color, clip);
        paint_border_edge(pixels, win_w, win_h, x, y + h - bw, w, bw, style, color, clip);
        paint_border_edge(pixels, win_w, win_h, x, y, bw, h, style, color, clip);
        paint_border_edge(pixels, win_w, win_h, x + w - bw, y, bw, h, style, color, clip);
        return;
    };
    match style {
        BorderStyle::Inset | BorderStyle::Outset => {
            let (top, left, bottom, right) = if style == BorderStyle::Inset {
                (dark, dark, light, light)
            } else {
                (light, light, dark, dark)
            };
            paint_border_edge(pixels, win_w, win_h, x, y, w, bw, BorderStyle::Solid, top, clip);
            paint_border_edge(pixels, win_w, win_h, x, y + h - bw, w, bw, BorderStyle::Solid, bottom, clip);
            paint_border_edge(pixels, win_w, win_h, x, y, bw, h, BorderStyle::Solid, left, clip);
            paint_border_edge(pixels, win_w, win_h, x + w - bw, y, bw, h, BorderStyle::Solid, right, clip);
        }
        BorderStyle::Groove | BorderStyle::Ridge => {
            // top/left outer/inner, bottom/right outer/inner.
            let (outer_tl, inner_tl, outer_br, inner_br) = if style == BorderStyle::Groove {
                (dark, light, light, dark)
            } else {
                (light, dark, dark, light)
            };
            let half = (bw / 2.0).max(0.5);
            // Top edge: outer = upper stripe, inner = lower stripe.
            paint_border_edge(pixels, win_w, win_h, x, y, w, half, BorderStyle::Solid, outer_tl, clip);
            paint_border_edge(pixels, win_w, win_h, x, y + half, w, bw - half, BorderStyle::Solid, inner_tl, clip);
            // Left edge: outer = left stripe, inner = right stripe.
            paint_border_edge(pixels, win_w, win_h, x, y, half, h, BorderStyle::Solid, outer_tl, clip);
            paint_border_edge(pixels, win_w, win_h, x + half, y, bw - half, h, BorderStyle::Solid, inner_tl, clip);
            // Bottom edge: inner = upper stripe, outer = lower stripe.
            paint_border_edge(pixels, win_w, win_h, x, y + h - bw, w, bw - half, BorderStyle::Solid, inner_br, clip);
            paint_border_edge(pixels, win_w, win_h, x, y + h - half, w, half, BorderStyle::Solid, outer_br, clip);
            // Right edge: inner = left stripe, outer = right stripe.
            paint_border_edge(pixels, win_w, win_h, x + w - bw, y, bw - half, h, BorderStyle::Solid, inner_br, clip);
            paint_border_edge(pixels, win_w, win_h, x + w - half, y, half, h, BorderStyle::Solid, outer_br, clip);
        }
        _ => unreachable!("bevel_colors only returns Some for the 4 bevel styles"),
    }
}

/// `clip` is the nearest ancestor's `overflow: hidden` bounds (screen
/// space, post-scroll) — deliberately the *ambient* clip from `paint`'s
/// own caller, not this box's own `overflow` (which only starts clipping
/// its *descendants*, same as before this parameter existed — see
/// `paint`'s own `child_clip` vs. this call's docs).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_box_decoration(
    s: &ComputedStyle,
    background_image: Option<&DecodedImage>,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    // Outer shadows: behind this box's own background/border, first
    // listed on top (so paint last-to-first), and never drawn inside the
    // border box itself.
    let border_radius = s.border_radius.resolve(w);
    for shadow in s.box_shadow.iter().rev().flatten().filter(|sh| !sh.inset) {
        paint_box_shadow(pixels, win_w, win_h, shadow, x, y, w, h, border_radius, clip);
    }
    // Rounds both background and border together, matching a real
    // rounded box's background-under-border stacking — see
    // `fill_rounded_ring`'s own docs for why the border ring needs its
    // own dedicated painting instead of reusing `fill_rounded_rect`
    // twice. Scoped to `BorderStyle::Solid`: a dashed/dotted/double
    // (or, since v63, bevel — `Groove`/`Ridge`/`Inset`/`Outset`) ring
    // would need the same curved-path tracing a solid ring does, just
    // repeated per dash/dot/stripe/shaded-half — a real `TODO.txt` gap,
    // not attempted here — so any of those combined with `border-radius`
    // fall back to the existing square-corner rendering rather than a
    // half-rounded, honestly-wrong mix of a rounded background behind
    // a square border.
    //
    // `border_radius` resolves its own `%` right here, against `w` —
    // this box's own border-box width, the real CSS basis for a
    // percentage `border-radius` (unlike `width`/`margin`/`padding`,
    // which resolve against the *containing block's* width — see
    // `BoxLength`'s own docs) — rather than in `measure`, since `w` is
    // already exactly the value needed and nothing else ever reads
    // this field before paint.
    // Uniform borders round only when solid (or absent); unequal per-side
    // borders always round, via `paint_per_side_border`.
    let uniform = s.border_width.iter().all(|w| *w == s.border_width[0])
        && s.border_style.iter().all(|t| *t == s.border_style[0])
        && s.border_color.iter().all(|c| *c == s.border_color[0]);
    let no_border = s.border_width.iter().all(|w| *w <= 0.0);
    let round = border_radius > 0.0;
    // Uniform solid borders keep the dedicated ring path; every other
    // style with a radius goes through the per-pixel per-side painter.
    let ring_path = uniform && (no_border || s.border_style[0] == BorderStyle::Solid || border_radius <= 0.0);
    if let Some(bg) = s.background_color {
        if round {
            fill_rounded_rect(pixels, win_w, win_h, x, y, w, h, border_radius, bg, clip);
        } else {
            fill_rect(pixels, win_w, win_h, x, y, w, h, bg, clip);
        }
    }
    // Layers on top of `background-color`, same real-CSS stacking; sized,
    // positioned and repeated per `bg_layout`, and clipped to the
    // rounded corners when `border-radius` is set. Origin/size use the
    // border box (real CSS uses the padding box by default).
    if let Some(image) = background_image {
        paint_background_image(image, &s.bg_layout, pixels, win_w, win_h, x, y, w, h, border_radius, clip);
    }
    if ring_path {
        if s.border_width[0] > 0.0 && s.border_style[0] != BorderStyle::None {
            if let Some(bc) = s.border_color[0] {
                let bw = s.border_width[0];
                if round {
                    fill_rounded_ring(pixels, win_w, win_h, x, y, w, h, border_radius, bw, bc, clip);
                } else {
                    paint_border(pixels, win_w, win_h, x, y, w, h, bw, s.border_style[0], bc, clip);
                }
            }
        }
    } else {
        paint_per_side_border(s, pixels, win_w, win_h, x, y, w, h, border_radius, clip);
    }
    // Outside the border box (not part of the box model — doesn't affect
    // `w`/`h` the way `border_width` does), same 4-rect ring shape as
    // border, just offset outward by its own width instead of inward.
    if s.outline_width > 0.0 {
        if let Some(oc) = s.outline_color {
            let ow = s.outline_width;
            let (ox, oy, ow_total, oh_total) = (x - ow, y - ow, w + 2.0 * ow, h + 2.0 * ow);
            fill_rect(pixels, win_w, win_h, ox, oy, ow_total, ow, oc, clip);
            fill_rect(pixels, win_w, win_h, ox, oy + oh_total - ow, ow_total, ow, oc, clip);
            fill_rect(pixels, win_w, win_h, ox, oy, ow, oh_total, oc, clip);
            fill_rect(pixels, win_w, win_h, ox + ow_total - ow, oy, ow, oh_total, oc, clip);
        }
    }
    // Inset shadows: inside the padding box, over the background.
    if s.box_shadow.iter().flatten().any(|sh| sh.inset) {
        let (bt, br, bb, bl) = (s.border_width[0], s.border_width[1], s.border_width[2], s.border_width[3]);
        let (px, py, pw, ph) = (x + bl, y + bt, w - bl - br, h - bt - bb);
        let pr = (border_radius - bt.max(br).max(bb).max(bl)).max(0.0);
        for shadow in s.box_shadow.iter().rev().flatten().filter(|sh| sh.inset) {
            paint_box_shadow(pixels, win_w, win_h, shadow, px, py, pw, ph, pr, clip);
        }
    }
}

/// Blurs `mask` (`mw` x `mh`, alpha 0..1) in place with three box-blur
/// passes per axis approximating a Gaussian of standard deviation `sigma`.
pub(crate) fn blur_mask(mask: &mut [f32], mw: usize, mh: usize, sigma: f32) {
    // Box width for 3 passes (the classic "boxes for gauss" formula).
    let wi = ((12.0 * sigma * sigma / 3.0 + 1.0).sqrt().floor() as usize).max(1);
    let r = wi / 2;
    if r == 0 {
        return;
    }
    let mut line = vec![0.0f32; mw.max(mh)];
    let box_pass = |src: &[f32], dst: &mut [f32], n: usize| {
        // dst[i] = mean of src[i-r..=i+r], zero outside.
        let inv = 1.0 / (2 * r + 1) as f32;
        let mut acc: f32 = src[..(r + 1).min(n)].iter().sum();
        for i in 0..n {
            dst[i] = acc * inv;
            if i + r + 1 < n {
                acc += src[i + r + 1];
            }
            if i >= r {
                acc -= src[i - r];
            }
        }
    };
    let mut tmp = vec![0.0f32; mw.max(mh)];
    for _ in 0..3 {
        for row in 0..mh {
            let base = row * mw;
            line[..mw].copy_from_slice(&mask[base..base + mw]);
            box_pass(&line[..mw], &mut tmp[..mw], mw);
            mask[base..base + mw].copy_from_slice(&tmp[..mw]);
        }
        for col in 0..mw {
            for row in 0..mh {
                line[row] = mask[row * mw + col];
            }
            box_pass(&line[..mh], &mut tmp[..mh], mh);
            for row in 0..mh {
                mask[row * mw + col] = tmp[row];
            }
        }
    }
}

/// Whether the pixel centre `(px, py)` lies inside the rounded rect.
pub(crate) fn in_uniform_rrect(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, radius: f32) -> bool {
    if px < x || px >= x + w || py < y || py >= y + h {
        return false;
    }
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let dy = (py - y).min(y + h - py);
    let inset = corner_inset(dy, r);
    px >= x + inset && px < x + w - inset
}

/// Paints one `box-shadow` layer for the border box `(x, y, w, h)` with
/// corner radius `radius`. Outer shadows skip the border box's own area;
/// inset ones are confined to `(x, y, w, h)`, which the caller passes as
/// the padding box. Blur is a 3-pass box blur of a coverage mask
/// (sigma = blur / 2); above `MAX_SHADOW_MASK` cells it degrades to a
/// hard edge.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_box_shadow(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    sh: &ResolvedBoxShadow,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    const MAX_SHADOW_MASK: f32 = 4_000_000.0;
    if sh.color.a() == 0 || w <= 0.0 || h <= 0.0 {
        return;
    }
    // The shadow shape: the box moved by the offset, grown (outer) or
    // shrunk (inset) by the spread.
    let g = if sh.inset { -sh.spread } else { sh.spread };
    let (sx, sy) = (x + sh.offset_x - g, y + sh.offset_y - g);
    let (sw, shh) = (w + 2.0 * g, h + 2.0 * g);
    let sr = if sh.inset { (radius - sh.spread).max(0.0) } else { (radius + sh.spread).max(0.0) };
    let mut blur = sh.blur;
    let mut m = (blur * 1.5).ceil() as i32;
    // The mask covers the region the shadow can touch: outer = the shape
    // plus the blur margin, inset = the padding box.
    let (rx, ry, rw, rh) = if sh.inset {
        (x.floor() as i32 - m, y.floor() as i32 - m, w.ceil() as i32 + 2 * m + 1, h.ceil() as i32 + 2 * m + 1)
    } else {
        (sx.floor() as i32 - m, sy.floor() as i32 - m, sw.max(0.0).ceil() as i32 + 2 * m + 1, shh.max(0.0).ceil() as i32 + 2 * m + 1)
    };
    let (mut rx, mut ry, mut rw, mut rh) = (rx, ry, rw, rh);
    if rw <= 0 || rh <= 0 {
        return;
    }
    if (rw as f32) * (rh as f32) > MAX_SHADOW_MASK {
        blur = 0.0;
        m = 0;
        if sh.inset {
            (rx, ry, rw, rh) = (x.floor() as i32, y.floor() as i32, w.ceil() as i32 + 1, h.ceil() as i32 + 1);
        } else {
            (rx, ry, rw, rh) = (sx.floor() as i32, sy.floor() as i32, sw.max(0.0).ceil() as i32 + 1, shh.max(0.0).ceil() as i32 + 1);
        }
        if (rw as f32) * (rh as f32) > MAX_SHADOW_MASK * 4.0 {
            return;
        }
    }
    let _ = m;
    let (mw, mh) = (rw as usize, rh as usize);
    let mut mask = vec![0.0f32; mw * mh];
    for j in 0..mh {
        for i in 0..mw {
            let (cx, cy) = ((rx + i as i32) as f32 + 0.5, (ry + j as i32) as f32 + 0.5);
            if in_uniform_rrect(cx, cy, sx, sy, sw, shh, sr) {
                mask[j * mw + i] = 1.0;
            }
        }
    }
    if blur > 0.0 {
        blur_mask(&mut mask, mw, mh, blur / 2.0);
    }
    let (cx0, cy0, cw, ch) = intersect_clip(0.0, 0.0, win_w as f32, win_h as f32, clip);
    let (lo_x, lo_y) = (cx0.max(0.0).floor() as i32, cy0.max(0.0).floor() as i32);
    let (hi_x, hi_y) = ((cx0 + cw).min(win_w as f32).ceil() as i32, (cy0 + ch).min(win_h as f32).ceil() as i32);
    for j in 0..mh as i32 {
        let py = ry + j;
        if py < lo_y || py >= hi_y {
            continue;
        }
        for i in 0..mw as i32 {
            let px = rx + i;
            if px < lo_x || px >= hi_x {
                continue;
            }
            let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
            let inside_box = in_uniform_rrect(cx, cy, x, y, w, h, radius);
            let cov = mask[j as usize * mw + i as usize];
            let alpha = if sh.inset {
                if !inside_box {
                    continue;
                }
                1.0 - cov
            } else {
                if inside_box {
                    continue;
                }
                cov
            };
            let a = (sh.color.a() as f32 * alpha).round() as u8;
            if a == 0 {
                continue;
            }
            let idx = (py as u32 * win_w + px as u32) as usize;
            pixels[idx] = blend(pixels[idx], TextColor::rgba(sh.color.r(), sh.color.g(), sh.color.b(), a));
        }
    }
}

/// Paints each span's background (if any) as a rect behind its glyphs,
/// before the glyphs themselves — a span can wrap across multiple lines,
/// so this walks every line's glyphs rather than assuming one rect per
/// span, merging each line's consecutive same-span glyphs into one rect
/// (glyphs carry their originating span's index via cosmic-text's
/// per-`Attrs` `metadata` field, set in `shape_spans`).
pub(crate) fn paint_span_backgrounds(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    backgrounds: &[Option<TextColor>],
    hidden: &[bool],
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    ox: i32,
    oy: i32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    buffer.shape_until_scroll(font_system, false);
    for run in buffer.layout_runs() {
        let mut i = 0;
        while i < run.glyphs.len() {
            let glyph = &run.glyphs[i];
            let is_hidden = hidden.get(glyph.metadata).copied().unwrap_or(false);
            let bg = if is_hidden { None } else { backgrounds.get(glyph.metadata).copied().flatten() };
            let start_x = glyph.x;
            let mut end_x = glyph.x + glyph.w;
            let mut j = i + 1;
            while j < run.glyphs.len() && run.glyphs[j].metadata == glyph.metadata {
                end_x = run.glyphs[j].x + run.glyphs[j].w;
                j += 1;
            }
            if let Some(color) = bg {
                fill_rect(
                    pixels,
                    win_w,
                    win_h,
                    start_x + ox as f32,
                    run.line_top + oy as f32,
                    end_x - start_x,
                    run.line_height,
                    color,
                    clip,
                );
            }
            i = j;
        }
    }
}

pub(crate) fn paint_buffer(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    backgrounds: &[Option<TextColor>],
    valigns: &[VerticalAlign],
    // `visibility: hidden` per span (by the same `glyph.metadata` index as
    // `backgrounds`/`valigns`) — skips painting that span's background and
    // glyphs while its layout space (already reserved by `measure`, which
    // never looks at visibility) stays exactly as if it had painted.
    // Decoration lines (underline/strikethrough) aren't filtered by this —
    // `paint_decorations` draws per-run, not per-span — a known, narrow
    // simplification: nothing in `default.css` pairs `text-decoration`
    // with `visibility: hidden` today.
    hidden: &[bool],
    // Per-span `text-decoration-style` (by the same `glyph.metadata` index
    // as `backgrounds`/`valigns`/`hidden`) — see `paint_decorations`.
    decoration_styles: &[DecorationStyle],
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    offset_x: f32,
    offset_y: f32,
    // The nearest ancestor's `overflow: hidden` content-box bounds (screen
    // space, post-scroll), or `None` if nothing above this clips. Checked
    // per-pixel here (text has no shared `fill_rect` choke point to push
    // this into, unlike box decoration — see `fill_rect`'s own docs).
    clip: Option<(f32, f32, f32, f32)>,
) {
    let ox = offset_x.round() as i32;
    let oy = offset_y.round() as i32;
    paint_span_backgrounds(buffer, font_system, backgrounds, hidden, pixels, win_w, win_h, ox, oy, clip);
    let default_color = TextColor::rgb(0, 0, 0);
    // `buffer.draw()`'s own internals, copied out by hand (public:
    // `LegacyRenderer`; decoration lines via this file's own
    // `paint_decorations` instead of cosmic-text's `render_decoration`)
    // instead of called directly — neither offers a hook to nudge an
    // individual glyph's y position (`vertical-align: sub`/`super`) or
    // paint a dashed/dotted line instead of a solid one
    // (`text-decoration-style`), both needed per span (looked up by
    // `glyph.metadata`, the same per-span index `paint_span_backgrounds`
    // already reads it by).
    buffer.shape_until_scroll(font_system, false);
    let mut renderer = LegacyRenderer {
        font_system,
        cache: swash_cache,
        callback: |x: i32, y: i32, w: u32, h: u32, color: TextColor| {
            if color.a() == 0 {
                return;
            }
            for dy in 0..h as i32 {
                for dx in 0..w as i32 {
                    let px = x + dx + ox;
                    let py = y + dy + oy;
                    if px < 0 || py < 0 || px as u32 >= win_w || py as u32 >= win_h {
                        continue;
                    }
                    if let Some((cx, cy, cw, ch)) = clip {
                        let (pxf, pyf) = (px as f32, py as f32);
                        if pxf < cx || pyf < cy || pxf >= cx + cw || pyf >= cy + ch {
                            continue;
                        }
                    }
                    let idx = (py as u32 * win_w + px as u32) as usize;
                    pixels[idx] = blend(pixels[idx], color);
                }
            }
        },
    };
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            if hidden.get(glyph.metadata).copied().unwrap_or(false) {
                continue;
            }
            let y_shift = valigns
                .get(glyph.metadata)
                .copied()
                .unwrap_or_default()
                .y_shift(glyph.font_size);
            let physical_glyph = glyph.physical((0., run.line_y + y_shift), 1.0);
            let glyph_color = glyph.color_opt.unwrap_or(default_color);
            renderer.glyph(physical_glyph, glyph_color);
        }
        paint_decorations(&mut renderer, &run, default_color, decoration_styles);
    }
}

/// Reimplements cosmic-text's own `render_decoration`/`draw_decoration_span`
/// (`src/render.rs` in the vendored 0.19.0 source — not `pub`, so it can't
/// be called directly) by hand, using only `DecorationSpan`'s/
/// `GlyphDecorationData`'s public fields, so a `DecorationStyle` other than
/// `Solid` can paint a dashed/dotted/double/wavy line instead of a plain
/// solid stroke across the same extent — a hook the public `render_
/// decoration` doesn't offer. Mirrors its exact offset/thickness/color
/// formulas, so `DecorationStyle::Solid` still looks pixel-identical to
/// calling `render_decoration` directly; `Dashed`/`Dotted`/`Double`/`Wavy`
/// diverge (see `paint_decoration_line`'s own docs for each). Each span's
/// style is looked up by `decoration_styles[glyph.metadata]` off its first
/// glyph — the same per-span index every other per-span property
/// (backgrounds, valigns, ...) already uses, and applies uniformly to
/// whichever of underline/strikethrough/overline this span has active —
/// real CSS `text-decoration-style` is one value shared by every line a
/// `text-decoration-line` turns on, not set per-line.
///
/// Doubling (v60) no longer depends on cosmic-text's own native
/// `UnderlineStyle::Double` at all — this function never called cosmic's
/// own runtime renderer to begin with (only mirrored its formulas), so
/// there was never a real reason to special-case *only* underline the way
/// pre-v60 `parse_text_decoration` did. `UnderlineStyle::Double` is still
/// matched defensively (treated exactly like `Single` plus a forced
/// `Double` style) in case some other code path ever sets it independently
/// of this file's own `DecorationStyle`, but neither `parse_text_
/// decoration` nor the v60 longhands produce it anymore — `double` always
/// goes through `DecorationStyle::Double` now, which this function applies
/// the same way to strikethrough/overline too.
pub(crate) fn paint_decorations<R: Renderer>(
    renderer: &mut R,
    run: &cosmic_text::LayoutRun,
    default_color: TextColor,
    decoration_styles: &[DecorationStyle],
) {
    for span in run.decorations {
        let glyphs = &run.glyphs[span.glyph_range.clone()];
        if glyphs.is_empty() {
            continue;
        }
        let style = decoration_styles.get(glyphs[0].metadata).copied().unwrap_or_default();
        let deco = &span.data;
        let td = &deco.text_decoration;
        let font_size = span.font_size;

        // Min/max over all glyphs, not first/last — RTL paragraphs store
        // glyphs in right-to-left order (same reasoning as cosmic-text's
        // own `draw_decoration_span`).
        let mut x_min = f32::INFINITY;
        let mut x_max = f32::NEG_INFINITY;
        for g in glyphs {
            x_min = x_min.min(g.x);
            x_max = x_max.max(g.x + g.w);
        }
        let width = x_max - x_min;
        if width <= 0.0 {
            continue;
        }
        let x_start = x_min;

        match td.underline {
            UnderlineStyle::None => {}
            UnderlineStyle::Single | UnderlineStyle::Double => {
                let color = td.underline_color_opt.or(span.color_opt).unwrap_or(default_color);
                let thickness = (deco.underline_metrics.thickness * font_size).max(1.0).ceil();
                let y = run.line_y - deco.underline_metrics.offset * font_size;
                // See this function's own docs — native `Double` forces
                // doubling even if `style` itself isn't `Double`.
                let style = if td.underline == UnderlineStyle::Double {
                    DecorationStyle::Double
                } else {
                    style
                };
                paint_decoration_line(renderer, x_start, y, width, thickness, color, style);
            }
        }
        if td.strikethrough {
            let color = td.strikethrough_color_opt.or(span.color_opt).unwrap_or(default_color);
            let thickness = (deco.strikethrough_metrics.thickness * font_size).max(1.0).ceil();
            let y = run.line_y - deco.strikethrough_metrics.offset * font_size;
            paint_decoration_line(renderer, x_start, y, width, thickness, color, style);
        }
        if td.overline {
            let color = td.overline_color_opt.or(span.color_opt).unwrap_or(default_color);
            // Reuse underline thickness for overline, same as cosmic-text's
            // own `draw_decoration_span`.
            let thickness = (deco.underline_metrics.thickness * font_size).max(1.0).ceil();
            let y = (run.line_y - deco.ascent * font_size).max(run.line_top);
            paint_decoration_line(renderer, x_start, y, width, thickness, color, style);
        }
    }
}

/// Paints one decoration line across `(x_start, y)..(x_start + width, y +
/// thickness)` — the whole span width in one `Renderer::rectangle` call
/// for `Solid`, several short ones spaced across the same extent for
/// `Dashed`/`Dotted`, two parallel `Solid` strokes (offset by `thickness`
/// plus an equal gap, same formula cosmic-text's own native double-
/// underline already used) for `Double`, and a piecewise sine-wave trace
/// for `Wavy` (v60) — see each arm's own comment. `x_start`/`y` are in the
/// same float coordinate space `render_decoration` itself uses (rounded to
/// pixels by `Renderer` implementors, same as any other rectangle it
/// draws).
pub(crate) fn paint_decoration_line<R: Renderer>(
    renderer: &mut R,
    x_start: f32,
    y: f32,
    width: f32,
    thickness: f32,
    color: TextColor,
    style: DecorationStyle,
) {
    match style {
        DecorationStyle::Solid => {
            let w = width as u32;
            if w > 0 {
                renderer.rectangle(x_start as i32, y as i32, w, thickness as u32, color);
            }
        }
        DecorationStyle::Dashed | DecorationStyle::Dotted => {
            let (mark, gap) = if style == DecorationStyle::Dotted {
                (thickness.max(1.0), thickness.max(1.0) * 2.0)
            } else {
                (thickness.max(1.0) * 3.0, thickness.max(1.0) * 2.0)
            };
            let step = mark + gap;
            let mut offset = 0.0f32;
            while offset < width {
                let seg_w = mark.min(width - offset) as u32;
                if seg_w > 0 {
                    renderer.rectangle((x_start + offset) as i32, y as i32, seg_w, thickness as u32, color);
                }
                offset += step;
            }
        }
        DecorationStyle::Double => {
            let gap = thickness;
            paint_decoration_line(renderer, x_start, y, width, thickness, color, DecorationStyle::Solid);
            paint_decoration_line(renderer, x_start, y + thickness + gap, width, thickness, color, DecorationStyle::Solid);
        }
        DecorationStyle::Wavy => {
            // `Renderer` offers only axis-aligned rectangles (confirmed by
            // reading cosmic-text's own `Renderer` trait — just
            // `rectangle`/`glyph`, no path/curve primitive at all), so a
            // real sine curve is approximated as a sequence of narrow
            // rectangles, each centered on the curve's own `sin` value at
            // that x — the same "break one long line into several small
            // `rectangle` calls" trick `Dashed`/`Dotted` already use, just
            // with a continuously-varying y instead of a gap pattern.
            // Amplitude/wavelength are both derived from `thickness` (the
            // stroke's own line weight) so the wave scales with font size
            // the same way dash/dot spacing already does, rather than
            // using fixed pixel constants that would look disproportionate
            // at very small or very large text.
            let amplitude = (thickness * 1.5).max(2.0);
            let wavelength = (thickness * 8.0).max(8.0);
            let step = (thickness * 0.5).max(1.0);
            let seg_h = thickness.max(1.0);
            let mut x = 0.0f32;
            while x < width {
                let seg_w = step.min(width - x);
                let center = x + seg_w / 2.0;
                let phase = (center / wavelength) * std::f32::consts::TAU;
                let y_off = amplitude * phase.sin();
                let seg_w_px = seg_w.max(1.0) as u32;
                if seg_w_px > 0 {
                    renderer.rectangle((x_start + x) as i32, (y + y_off) as i32, seg_w_px, seg_h as u32, color);
                }
                x += step;
            }
        }
    }
}

/// Blits a decoded image into `(x, y, w, h)` — the laid content-box rect,
/// same positioning `paint`'s `Text` arm already uses for its own content
/// (padding/border already excluded by the caller). The picture is
/// scaled from the image's intrinsic size and placed per `object-fit`/`object-position` and sampled by
/// `sample_image` (bilinear, box-averaged when downscaling). Composited through
/// the same `blend` every other paint primitive uses, so a partially
/// transparent pixel (a PNG with alpha) shows whatever `paint_box_
/// decoration` already painted underneath (this box's own background,
/// if any — called first, same stacking a real browser uses).
#[allow(clippy::too_many_arguments)]
pub(crate) fn blit_image(
    image: &DecodedImage,
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
) {
    if image.width == 0 || image.height == 0 || w <= 0.0 || h <= 0.0 {
        return;
    }
    // Where the picture lands inside the content box (`object-fit`,
    // `object-position`); it may be smaller (contain/none) or larger
    // (cover/none) than the box, and is clipped to the box either way.
    let (iw, ih) = (image.width as f32, image.height as f32);
    let (rw, rh) = match fit {
        ObjectFit::Fill => (w, h),
        ObjectFit::Contain => {
            let sc = (w / iw).min(h / ih);
            (iw * sc, ih * sc)
        }
        ObjectFit::Cover => {
            let sc = (w / iw).max(h / ih);
            (iw * sc, ih * sc)
        }
        ObjectFit::None => (iw, ih),
        ObjectFit::ScaleDown => {
            let sc = (w / iw).min(h / ih).min(1.0);
            (iw * sc, ih * sc)
        }
    };
    if rw <= 0.0 || rh <= 0.0 {
        return;
    }
    let rx = x + pos.0 .0 * (w - rw) + pos.0 .1;
    let ry = y + pos.1 .0 * (h - rh) + pos.1 .1;
    let (bx, by, bw, bh) = intersect_clip(x, y, w, h, clip);
    let (cx, cy, cw, ch) = intersect_clip(rx, ry, rw, rh, Some((bx, by, bw, bh)));
    let x0 = cx.max(0.0) as i64;
    let y0 = cy.max(0.0) as i64;
    let x1 = ((cx + cw).min(win_w as f32)) as i64;
    let y1 = ((cy + ch).min(win_h as f32)) as i64;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (kx, ky) = (iw / rw, ih / rh);
    for py in y0..y1 {
        // Maps this destination row back to source space via the
        // *unclipped* rect, so clipping never shifts the sampling.
        let fy = (py as f32 + 0.5 - ry) * ky;
        for px in x0..x1 {
            let fx = (px as f32 + 0.5 - rx) * kx;
            let color = sample_image(image, fx, fy, kx, ky);
            if color.a() == 0 {
                continue;
            }
            let idx = (py as u32 * win_w + px as u32) as usize;
            pixels[idx] = blend(pixels[idx], color);
        }
    }
}

/// Samples `image` at source-space point `(fx, fy)` (pixel centres at
/// +0.5). `kx`/`ky` are source pixels per destination pixel: when
/// downscaling by 2x or more the whole footprint is box-averaged (capped
/// at 16x16 taps), otherwise a 4-tap bilinear blend. Alpha-weighted so a
/// transparent neighbour doesn't bleed its colour into the result.
pub(crate) fn sample_image(image: &DecodedImage, fx: f32, fy: f32, kx: f32, ky: f32) -> TextColor {
    let (w, h) = (image.width as i64, image.height as i64);
    let at = |x: i64, y: i64| image.pixels[(y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize];
    let mut acc = [0.0f32; 4];
    let mut add = |c: TextColor, wt: f32| {
        let a = c.a() as f32 * wt;
        acc[0] += c.r() as f32 * a;
        acc[1] += c.g() as f32 * a;
        acc[2] += c.b() as f32 * a;
        acc[3] += a;
    };
    let total = if kx >= 2.0 || ky >= 2.0 {
        let (sx0, sx1) = (fx - kx / 2.0, fx + kx / 2.0);
        let (sy0, sy1) = (fy - ky / 2.0, fy + ky / 2.0);
        let nx = (kx.ceil() as i64).clamp(1, 16);
        let ny = (ky.ceil() as i64).clamp(1, 16);
        for j in 0..ny {
            let yy = sy0 + (sy1 - sy0) * (j as f32 + 0.5) / ny as f32;
            for i in 0..nx {
                let xx = sx0 + (sx1 - sx0) * (i as f32 + 0.5) / nx as f32;
                add(at(xx.floor() as i64, yy.floor() as i64), 1.0);
            }
        }
        (nx * ny) as f32
    } else {
        let (gx, gy) = (fx - 0.5, fy - 0.5);
        let (x0, y0) = (gx.floor(), gy.floor());
        let (tx, ty) = (gx - x0, gy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        add(at(x0, y0), (1.0 - tx) * (1.0 - ty));
        add(at(x0 + 1, y0), tx * (1.0 - ty));
        add(at(x0, y0 + 1), (1.0 - tx) * ty);
        add(at(x0 + 1, y0 + 1), tx * ty);
        1.0
    };
    if acc[3] <= 0.0 {
        return TextColor::rgba(0, 0, 0, 0);
    }
    TextColor::rgba(
        (acc[0] / acc[3]).round().min(255.0) as u8,
        (acc[1] / acc[3]).round().min(255.0) as u8,
        (acc[2] / acc[3]).round().min(255.0) as u8,
        (acc[3] / total).round().min(255.0) as u8,
    )
}

/// The painted tile size of a `width` x `height` image under `layout`
/// in a `w` x `h` positioning area (`background-size` / `mask-size`).
pub(crate) fn bg_tile_size(width: u32, height: u32, layout: &BgLayout, w: f32, h: f32) -> (f32, f32) {
    let (iw, ih) = (width as f32, height as f32);
    let dim = |d: BgDim, basis: f32| match d {
        BgDim::Auto => None,
        BgDim::Px(v) => Some(v),
        BgDim::Pct(p) => Some(basis * p),
    };
    match layout.size {
        BgSize::Cover | BgSize::Contain => {
            let sx = w / iw;
            let sy = h / ih;
            let sc = if layout.size == BgSize::Cover { sx.max(sy) } else { sx.min(sy) };
            (iw * sc, ih * sc)
        }
        BgSize::Explicit(dw, dh) => match (dim(dw, w), dim(dh, h)) {
            (None, None) => (iw, ih),
            (Some(a), None) => (a, a * ih / iw),
            (None, Some(b)) => (b * iw / ih, b),
            (Some(a), Some(b)) => (a, b),
        },
    }
}

/// `mask-image` (Q29): paints the box via `paint_inner` and then blends
/// the result back toward what was underneath wherever the mask's alpha
/// is below 1, over the whole border box (descendants included). The
/// coverage comes from painting the (white-with-alpha) mask through the
/// background tiling code onto a black scratch buffer. An SVG mask is
/// re-rasterized at its tile size so it stays sharp.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_masked(
    laid: &Laid,
    mask: &DecodedImage,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    cache: &mut ShapeCache,
    scroll_offsets: &HashMap<usize, f32>,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    scroll_offset: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let (x, y, w, h) = (laid.x, laid.y - scroll_offset, laid.w, laid.h);
    let (cx, cy, cw, ch) = intersect_clip(x, y, w, h, clip);
    let x0 = cx.max(0.0).floor() as i64;
    let y0 = cy.max(0.0).floor() as i64;
    let x1 = ((cx + cw).min(win_w as f32)).ceil() as i64;
    let y1 = ((cy + ch).min(win_h as f32)).ceil() as i64;
    let go = |pixels: &mut [u32], font_system: &mut FontSystem, swash_cache: &mut SwashCache, cache: &mut ShapeCache| {
        paint_inner_with_opacity(laid, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, scroll_offset, clip);
    };
    if x1 <= x0 || y1 <= y0 || w <= 0.0 || h <= 0.0 || mask.width == 0 || mask.height == 0 {
        // Nothing visible to mask; a box whose mask is empty paints nothing.
        return;
    }
    let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut before = Vec::with_capacity(rw * rh);
    for py in y0..y1 {
        let base = (py as u32 * win_w) as usize + x0 as usize;
        before.extend_from_slice(&pixels[base..base + rw]);
    }
    go(pixels, font_system, swash_cache, cache);
    let layout = &laid.style.mask_layout;
    let sharp;
    let mask = match &mask.svg {
        Some(bytes) => {
            let (tw, th) = bg_tile_size(mask.width, mask.height, layout, w, h);
            let (tw, th) = (tw.round() as u32, th.round() as u32);
            match rasterize_svg(bytes, Some((tw.max(1), th.max(1)))) {
                Some(img) => {
                    sharp = whiten_mask(img);
                    &sharp
                }
                None => mask,
            }
        }
        None => mask,
    };
    let mut cov = vec![0u32; rw * rh];
    let prev = PAINT_OPACITY.with(|o| o.replace(1.0));
    paint_background_image(mask, layout, &mut cov, rw as u32, rh as u32, x - x0 as f32, y - y0 as f32, w, h, 0.0, None);
    PAINT_OPACITY.with(|o| o.set(prev));
    for j in 0..rh {
        let base = ((y0 as usize + j) * win_w as usize) + x0 as usize;
        for i in 0..rw {
            let a = cov[j * rw + i] & 0xFF;
            if a == 255 {
                continue;
            }
            let old = before[j * rw + i];
            let new = pixels[base + i];
            let mix = |sh: u32| {
                let o = (old >> sh) & 0xFF;
                let n = (new >> sh) & 0xFF;
                ((o * (255 - a) + n * a) / 255) << sh
            };
            pixels[base + i] = mix(16) | mix(8) | mix(0);
        }
    }
}

/// Paints a background image per `layout` (Q16): `background-size`
/// (`cover`/`contain`/explicit/auto, with a missing axis following the
/// image's aspect), `background-position` (a fraction of the free space
/// plus an offset, per axis), and `background-repeat` per axis. The
/// painted area is clipped to the box's rounded corners (`radius`).
/// Nearest-neighbour sampling.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_background_image(
    image: &DecodedImage,
    layout: &BgLayout,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    if image.width == 0 || image.height == 0 || w <= 0.0 || h <= 0.0 {
        return;
    }
    let (iw, ih) = (image.width as f32, image.height as f32);
    let (tw, th) = bg_tile_size(image.width, image.height, layout, w, h);
    if tw < 0.5 || th < 0.5 {
        return;
    }
    let ox = x + layout.pos_x.0 * (w - tw) + layout.pos_x.1;
    let oy = y + layout.pos_y.0 * (h - th) + layout.pos_y.1;
    let (cx, cy, cw, ch) = intersect_clip(x, y, w, h, clip);
    let x0 = cx.max(0.0) as i64;
    let y0 = cy.max(0.0) as i64;
    let x1 = ((cx + cw).min(win_w as f32)) as i64;
    let y1 = ((cy + ch).min(win_h as f32)) as i64;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    for py in y0..y1 {
        let mut dy = py as f32 - oy;
        if layout.repeat.y {
            dy = dy.rem_euclid(th);
        } else if dy < 0.0 || dy >= th {
            continue;
        }
        let sy = ((dy / th * ih) as u32).min(image.height - 1);
        let (lo, hi) = if r > 0.0 {
            let ry = ((py as f32 - y).min((y + h) - py as f32 - 1.0)).max(0.0);
            let inset = corner_inset(ry, r);
            ((x + inset).ceil() as i64, (x + w - inset).floor() as i64)
        } else {
            (x0, x1)
        };
        for px in x0.max(lo)..x1.min(hi) {
            let mut dx = px as f32 - ox;
            if layout.repeat.x {
                dx = dx.rem_euclid(tw);
            } else if dx < 0.0 || dx >= tw {
                continue;
            }
            let sx = ((dx / tw * iw) as u32).min(image.width - 1);
            let color = image.pixels[(sy * image.width + sx) as usize];
            let idx = (py as u32 * win_w + px as u32) as usize;
            pixels[idx] = blend(pixels[idx], color);
        }
    }
}

/// Default layout (intrinsic size, `0 0`, repeating) — kept for the
/// unit test of the basic tiling behaviour.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn tile_image(image: &DecodedImage, pixels: &mut [u32], win_w: u32, win_h: u32, x: f32, y: f32, w: f32, h: f32, clip: Option<(f32, f32, f32, f32)>) {
    paint_background_image(image, &BgLayout::default(), pixels, win_w, win_h, x, y, w, h, 0.0, clip);
}

/// `overflow: scroll`/`auto`'s own minimal scrollbar indicator (v62) —
/// a flat, simple painting primitive in the same spirit as this file's
/// other decorations (box-shadow/border): a translucent track plus a
/// translucent thumb, both hard-edged `fill_rect`s, no rounded ends, no
/// hover states. Deliberately an *overlay*, not a reserved-space
/// classic scrollbar — painted inside this box's own border box without
/// narrowing `content_w` for it at layout time at all (same "scrolling
/// never touches layout" reasoning `ScrollInfo`'s own docs already
/// give, extended to this box's own width too — an overlay-scrollbar OS
/// convention this matches, not just a shortcut). `Scroll` always
/// paints the track+thumb, even with nothing to scroll yet (thumb fills
/// the whole track); `Auto` only paints anything once `max_scroll() >
/// 0.0` — the one real, documented difference between the two keywords
/// (see `Overflow`'s own docs). Clipped by the *ambient* `clip`, not
/// `child_clip` — this is this box's own chrome, not descendant
/// content, same treatment `paint_box_decoration` already gets.
/// **Interactive drag-to-scroll on the thumb itself is explicitly out
/// of scope** (see TODO.txt) — wheel-scroll is this feature's own
/// interactive core; dragging the indicator is a documented cut.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_scrollbar(
    info: &ScrollInfo,
    own_scroll: f32,
    show_when_not_overflowing: bool,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    box_x: f32,
    draw_y: f32,
    box_w: f32,
    box_h: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let max_scroll = info.max_scroll();
    if max_scroll <= 0.0 && !show_when_not_overflowing {
        return;
    }
    const TRACK_W: f32 = 8.0;
    const MIN_THUMB_H: f32 = 16.0;
    if box_w < TRACK_W || box_h <= 0.0 {
        return;
    }
    let track_x = box_x + box_w - TRACK_W;
    let track_y = draw_y;
    let track_h = box_h;
    let ratio = if info.content_h > 0.0 { (info.client_h / info.content_h).clamp(0.0, 1.0) } else { 1.0 };
    let thumb_h = (track_h * ratio).clamp(MIN_THUMB_H.min(track_h), track_h);
    let thumb_y = if max_scroll > 0.0 { track_y + (own_scroll / max_scroll) * (track_h - thumb_h) } else { track_y };
    fill_rect(pixels, win_w, win_h, track_x, track_y, TRACK_W, track_h, TextColor::rgba(0, 0, 0, 28), clip);
    fill_rect(pixels, win_w, win_h, track_x, thumb_y, TRACK_W, thumb_h, TextColor::rgba(0, 0, 0, 120), clip);
}

/// Paints vim mode's `:`-command bar — chrome drawn on top of the
/// already-laid-out page, outside the DOM/layout tree entirely, same
/// role `paint_scrollbar` already plays for the scrollbar overlay
/// (that's the existing precedent TODO.txt's own vim-mode plan points
/// at for "where chrome gets painted"). Bottom of the window, not top —
/// matches real vim's own command line, and Vimium/qutebrowser's bars,
/// rather than reading as a browser address bar (a placement TODO.txt
/// explicitly left open; settled here as bottom).
pub(crate) fn paint_command_bar(
    command_line: &TextEditBuffer,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
) {
    const BAR_H: f32 = 24.0;
    const PAD_X: f32 = 4.0;
    let bar_y = (win_h as f32 - BAR_H).max(0.0);
    fill_rect(pixels, win_w, win_h, 0.0, bar_y, win_w as f32, BAR_H, TextColor::rgba(24, 24, 24, 255), None);

    // The leading `:` is purely this bar's own display convention
    // (vim's own prompt character) — `command_line.text` itself never
    // stores it, so every real cursor byte offset into `text` needs a
    // `+1` shift to land on the matching glyph in `display`.
    let display = format!(":{}", command_line.text);
    let metrics = Metrics::new(14.0, BAR_H);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(Some((win_w as f32 - PAD_X).max(0.0)), Some(BAR_H));
    buffer.set_wrap(Wrap::None);
    let mut attrs = Attrs::new();
    attrs.color_opt = Some(TextColor::rgb(225, 225, 225));
    buffer.set_text(&display, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    let target = command_line.cursor + 1;
    let caret_x = buffer
        .layout_runs()
        .next()
        .map(|run| {
            run.glyphs
                .iter()
                .find(|g| g.start >= target)
                .map(|g| g.x)
                .unwrap_or_else(|| run.glyphs.last().map(|g| g.x + g.w).unwrap_or(0.0))
        })
        .unwrap_or(0.0);

    // One span (`Attrs::new()`'s own default `metadata: 0`), so every
    // per-span array `paint_buffer` expects just needs that one entry.
    let backgrounds = [None];
    let valigns = [VerticalAlign::default()];
    let hidden = [false];
    let decoration_styles = [DecorationStyle::default()];
    paint_buffer(
        &mut buffer, font_system, swash_cache, &backgrounds, &valigns, &hidden, &decoration_styles, pixels, win_w,
        win_h, PAD_X, bar_y + (BAR_H - 20.0) / 2.0, None,
    );

    fill_rect(pixels, win_w, win_h, PAD_X + caret_x, bar_y + 3.0, 1.5, BAR_H - 6.0, TextColor::rgb(225, 225, 225), None);
}

/// Real CSS2 §9.9.1: `position: relative`/`sticky` establishes its own
/// descendant stacking context only when it also has an *explicit*
/// `z-index` (`z-index: auto` — by far the common case — does not; such
/// a box keeps today's plain paint-position-only treatment, painted in
/// ordinary document order alongside its non-positioned siblings, same
/// as before this feature existed). `absolute`/`fixed` are deliberately
/// excluded here even though they always *do* establish a stacking
/// context in real CSS: they're already out of flow and already have
/// their own, separate, correctly-ordered home (`Laid::positioned`) —
/// this predicate exists only to find the *in-flow* boxes that need
/// pulling out of `paint`'s/`hit_test_cursor`'s ordinary per-child loop
/// to be merged into that same z-ordered presentation instead.
pub(crate) fn is_explicit_stacking_child(l: &Laid) -> bool {
    matches!(l.style.position, Position::Relative | Position::Sticky) && l.style.z_index.is_some()
}

/// Paints one CSS2.1 Appendix E z-ordered layer for a box: either the
/// negative-stack-level slice (layer 2 — "child stacking contexts with
/// negative stack levels", painted *before* this box's own background/
/// border) or the zero-and-positive slice (layers 6+7 combined — see
/// `Laid::positioned`'s own docs for why merging those two is harmless
/// here), painted at the very end, same spot this box's out-of-flow
/// descendants have always painted at. Merges two different kinds of
/// descendant stacking context that both need to compete by the same
/// `z-index` ordering: `positioned` (this box's own out-of-flow
/// `absolute`/`fixed` catches, already resolved elsewhere) and
/// `stacking_children` (direct `Blocks`/`TableRow` children pulled out
/// by `is_explicit_stacking_child` — still genuinely in normal flow,
/// contributing to this box's size, just reordered for *paint* purposes
/// only). Ties (same explicit `z-index`, or both `auto`/unset as `0`)
/// keep `stacking_children` before `positioned` — an arbitrary but
/// stable, documented tie-break for a narrow case (two things anchored
/// to the very same box, at the very same explicit level) rather than a
/// claim about real document order across two differently-sourced
/// lists.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_z_layer(
    positioned: &[Laid],
    stacking_children: &[&Laid],
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    cache: &mut ShapeCache,
    // `v62`: passed straight through to every recursive `paint` call
    // below unmodified — a positioned/explicit-stacking entry is NOT
    // itself shifted by this box's own `overflow: scroll`/`auto` (see
    // TODO.txt: a documented, narrower cut from the general case, same
    // shape as several other out-of-flow-content limitations already
    // accepted elsewhere in this file) — but each entry may still
    // independently be (or contain) its own scrollable region, which is
    // why this still needs forwarding rather than dropping entirely.
    scroll_offsets: &HashMap<usize, f32>,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    scroll_offset: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let mut entries: Vec<&Laid> = stacking_children.to_vec();
    entries.extend(positioned.iter());
    entries.sort_by_key(|l| l.style.z_index.unwrap_or(0));
    for item in entries {
        if matches!(item.style.position, Position::Relative | Position::Sticky) {
            // Same sticky-vs-relative scroll-offset trick `paint`'s own
            // ordinary `Blocks`/`TableRow` loop uses — a plain `relative`
            // box's offset is already permanently baked into its own
            // `x`/`y` (no `sticky` field), so this is a no-op shift (`dy
            // == 0`) for it, and the real per-frame shift for `sticky`.
            let child_scroll = match &item.sticky {
                Some(info) => scroll_offset - sticky_shift(info, item.h, scroll_offset, win_h as f32),
                None => scroll_offset,
            };
            paint(item, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, child_scroll, clip);
        } else {
            // An out-of-flow `positioned` entry — same `fixed` handling
            // `paint`'s own old positioned-list loop always used.
            paint(
                item, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h,
                if item.fixed { 0.0 } else { scroll_offset }, clip,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    laid: &Laid,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    cache: &mut ShapeCache,
    scroll_offsets: &HashMap<usize, f32>,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    scroll_offset: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    if let Some(mask) = laid.background_image.as_ref().and_then(|b| b.mask.clone()) {
        paint_masked(laid, &mask, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, scroll_offset, clip);
        return;
    }
    paint_inner_with_opacity(laid, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, scroll_offset, clip);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_inner_with_opacity(
    laid: &Laid,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    cache: &mut ShapeCache,
    scroll_offsets: &HashMap<usize, f32>,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    scroll_offset: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let prev = PAINT_OPACITY.with(|o| o.replace(laid.style.opacity));
    paint_inner(laid, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, scroll_offset, clip);
    PAINT_OPACITY.with(|o| o.set(prev));
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_inner(
    laid: &Laid,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    cache: &mut ShapeCache,
    // `overflow: scroll`/`auto`'s own *live* per-element scroll amounts
    // (v62), keyed by `ScrollInfo::id` — see that type's own docs for
    // why this lives outside `Laid` entirely. Read fresh here (via
    // `own_scroll`, just below), never written — `App`'s own event
    // handling is the only thing that ever updates it.
    scroll_offsets: &HashMap<usize, f32>,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    // The *ambient* scroll offset — `0.0` for a `position: fixed`
    // subtree (see the `laid.positioned` loop below and `Laid::fixed`'s
    // own docs), the real page scroll offset for everything else. Not
    // read directly off `laid` itself: a `fixed` box's own descendants
    // (normal content *inside* it) need to keep seeing whatever value
    // this call was given, which is exactly what naturally happens —
    // every recursive `paint` call below just forwards this same
    // parameter unchanged, so passing `0.0` once, at the top of a
    // `fixed` subtree, is all it takes for the whole subtree to render
    // scroll-independent; nothing *inside* that subtree needs its own
    // `fixed` check.
    scroll_offset: f32,
    // The nearest ancestor's `overflow: hidden` bounds (screen space,
    // post-scroll), if any — `None` means nothing above this clips.
    // Reaches every paint call this box makes: text/span-backgrounds
    // (`paint_buffer`) *and*, since v40, this box's own background/
    // border/outline/shadow (`paint_box_decoration`) — both funnel
    // through the one shared `fill_rect` primitive, which applies it.
    clip: Option<(f32, f32, f32, f32)>,
) {
    let s = laid.style;
    let draw_y = laid.y - scroll_offset;
    if laid.ink.b - scroll_offset < 0.0 || laid.ink.t - scroll_offset > win_h as f32 {
        // Entirely outside the viewport. `ink` is the union of this
        // box's rect and everything it can paint (visible overflow,
        // positioned descendants, negative margins...) — NOT just the
        // border box, since descendants are not contained in their
        // parent's rect in CSS. Skip the whole subtree instead of
        // clipping per-pixel, so a redraw costs O(visible content).
        return;
    }

    // This box's own `overflow: hidden` (if any) only starts clipping
    // its *descendants* — its own background/border below isn't clipped
    // by it, same as an ancestor's `clip` never reaches that call either.
    // Computed before any painting happens below (it's pure geometry —
    // moving it ahead of the negative-z-index layer, which now needs to
    // paint *before* this box's own background, doesn't change what it
    // computes).
    let child_clip = if matches!(s.overflow, Overflow::Hidden | Overflow::Scroll | Overflow::Auto) {
        let (cx, cy, cw, ch) = intersect_clip(laid.x, draw_y, laid.w, laid.h, clip);
        Some((cx, cy, cw, ch))
    } else {
        clip
    };

    // `overflow: scroll`/`auto`'s own live scroll amount (v62), looked
    // up fresh every call and re-clamped against `max_scroll()` at the
    // read site (self-healing if a relayout ever shrinks `content_h`
    // out from under a stale, larger stored offset, rather than
    // trusting the stored value is already in range). `0.0` for every
    // other box, including a `scroll`/`auto` box with nothing stored
    // for its id yet (the common "never scrolled" starting state).
    let own_scroll = laid
        .scroll
        .map_or(0.0, |info| scroll_offsets.get(&info.id).copied().unwrap_or(0.0).clamp(0.0, info.max_scroll()));
    // Only this box's own *descendants* see the extra shift — this
    // box's own painted position (`draw_y` above) deliberately still
    // uses the un-adjusted `scroll_offset`, since scrolling a box moves
    // its *content*, never the box itself (unlike `sticky`/`fixed`,
    // which shift the whole subtree including the box). Composes
    // naturally with nested scrolling and with a nested `position:
    // sticky`'s own `sticky_shift` basis (both read whatever ambient
    // `scroll_offset` is lexically in scope at their own recursion
    // depth) the same way `fixed` forcing it to `0.0` already composes
    // — no special-casing needed for either. **Deliberately not
    // threaded into the `paint_z_layer` calls below** (layers 2/6+7 —
    // this box's own out-of-flow/explicitly-stacked descendants): see
    // TODO.txt, a documented, narrower cut.
    let content_scroll_offset = scroll_offset + own_scroll;

    // Real CSS2.1 Appendix E stacking order, split at the zero boundary.
    // **Layer 2 (negative stack levels) paints *above* this box's own
    // background/border (layer 1), not below it** — the spec's own
    // bottommost-first list is 1 (background), 2 (negative), 3-5 (normal
    // content), 6-7 (zero/positive) — so a negative-`z-index` descendant
    // is only ever hidden *behind this box's own normal (layer 3+)
    // content*, never behind this box's own background fill. (Checked
    // directly against https://www.w3.org/TR/CSS21/zindex.html, which
    // lists background before negative contexts in its "bottommost
    // first" order — i.e. negative paints on top of it. This also
    // matches the well-known real-world experience of `z-index: -1`:
    // it only visibly disappears "behind everything" when the element
    // it's negative *relative to* has no background of its own to begin
    // with, not because negative z-index literally paints beneath any
    // box's own background.) `laid.positioned` is already sorted
    // ascending by `z-index`, so `partition_point` finds the zero
    // boundary with no extra sort. Direct `Blocks`/`TableRow` children
    // that establish their own stacking context (`is_explicit_stacking_
    // child` — `position: relative`/`sticky` with an explicit `z-index`)
    // are pulled out the same way, from whichever list this box's own
    // content actually is (empty for every other `LaidContent` variant —
    // nothing but `Blocks`/`TableRow` has a child list to pull from).
    let neg_split = laid.positioned.partition_point(|l| l.style.z_index.unwrap_or(0) < 0);
    let (neg_positioned, nonneg_positioned) = laid.positioned.split_at(neg_split);
    let (neg_stacking_children, nonneg_stacking_children): (Vec<&Laid>, Vec<&Laid>) = match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => children
            .iter()
            .filter(|c| is_explicit_stacking_child(c))
            .partition(|c| c.style.z_index.unwrap_or(0) < 0),
        _ => (Vec::new(), Vec::new()),
    };

    // Layer 1: this box's own background/border.
    // `visibility: hidden` still reserves this box's layout space (laid
    // out exactly as normal above) but skips its own background/border —
    // a descendant box with its own `visibility: visible` is a separate
    // `Laid` with its own independently-resolved `style.visibility` via
    // real CSS inheritance, so it still recurses and paints normally.
    if s.visibility == Visibility::Visible {
        // Clipped by the *ambient* `clip` from an ancestor's `overflow:
        // hidden` — deliberately not `child_clip` below, since a box's
        // own `overflow` only clips its descendants' overflowing
        // content, never its own background/border/outline/shadow,
        // same as real CSS.
        paint_box_decoration(&s, laid.background_image.as_deref(), pixels, win_w, win_h, laid.x, draw_y, laid.w, laid.h, clip);
    }

    // Layer 2: negative stacking contexts — above this box's own
    // background/border (just painted), below everything else (the
    // normal-content `match` below, and the zero/positive layer at the
    // end of this function) — unconditional, same reasoning as that
    // final layer (each entry's own recursive `paint` call checks *its
    // own* `style.visibility`).
    paint_z_layer(
        neg_positioned, &neg_stacking_children, font_system, swash_cache, cache, scroll_offsets, pixels, win_w,
        win_h, content_scroll_offset, child_clip,
    );

    match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            // Layers 3/4/5: ordinary in-flow content, in document order
            // (floats already sorted to the end of this same list by
            // `measure`'s own `BoxContent::Blocks` arm — see v54). Any
            // child pulled out above (`is_explicit_stacking_child`) is
            // skipped here — it's painted by one of the two `paint_z_
            // layer` calls instead, never both (every child is painted
            // exactly once).
            for child in children.iter().filter(|c| !is_explicit_stacking_child(c)) {
                // A `position: sticky` child's own `Laid::x`/`.y` are
                // still exactly its static in-flow position (never
                // permanently shifted, unlike `relative` — see `Position::
                // Sticky`'s own docs) — so its real, current painted
                // position is computed fresh, right here, from the live
                // `scroll_offset`, via `sticky_shift`. Folding that `dy`
                // into the *ambient* scroll offset passed down into this
                // one recursive call (rather than mutating `child` or
                // threading a separate offset parameter) reuses the exact
                // trick `fixed` already established: every descendant
                // inside this subtree (`draw_y = y - scroll_offset`, and
                // any nested `positioned`/`sticky` box of its own) sees
                // the adjusted value and shifts correctly with no extra
                // per-node check needed anywhere inside it.
                let child_scroll = match &child.sticky {
                    Some(info) => {
                        content_scroll_offset - sticky_shift(info, child.h, content_scroll_offset, win_h as f32)
                    }
                    None => content_scroll_offset,
                };
                paint(
                    child, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, child_scroll,
                    child_clip,
                );
            }
        }
        LaidContent::Text(id) => {
            // Not yet shaped (still mid-way through a budgeted measure pass)
            // just means nothing to paint here yet — it'll appear once a
            // later frame finishes shaping it.
            if let Some(entry) = cache.get(id) {
                let mut guard = entry.lock().unwrap();
                let (buffer, _, backgrounds, valigns, hidden, _, decoration_styles, _) = &mut *guard;
                let content_x = laid.x + s.border_left() + s.padding.left.px();
                let content_y = draw_y + s.border_top() + s.padding.top.px();
                paint_buffer(
                    buffer, font_system, swash_cache, backgrounds, valigns, hidden, decoration_styles, pixels,
                    win_w, win_h, content_x, content_y, child_clip,
                );
            }
        }
        LaidContent::Image(image) => {
            let content_x = laid.x + s.border_left() + s.padding.left.px();
            let content_y = draw_y + s.border_top() + s.padding.top.px();
            let content_w = (laid.w - s.border_h() - s.padding.left.px() - s.padding.right.px()).max(0.0);
            let content_h = (laid.h - s.border_v() - s.padding.top.px() - s.padding.bottom.px()).max(0.0);
            blit_image(image, s.object_fit, s.object_pos, pixels, win_w, win_h, content_x, content_y, content_w, content_h, child_clip);
        }
        LaidContent::MixedInline(positioned) => {
            // Don't reveal this run's text word by word as each one
            // happens to finish shaping (`measure_mixed_inline` shapes
            // one `Buffer` per word, not per run — needed so its own
            // manual wrap pass has each word's real width to wrap
            // around the image with, but it means a tick's own
            // `AppEvent::ShapeResult` can land with only *some* of a
            // run's words shaped). On a real page with many short
            // inline-image-mixed runs (Wikipedia's own inline icons —
            // flags, lock/edit badges — are a common real case), that
            // showed up as text visibly trickling in a few characters
            // at a time each tick, instead of appearing in whole
            // paragraph-sized chunks the way ordinary text already
            // does. Gate painting on the *whole* run having finished —
            // images still paint immediately regardless, since those
            // never go through `cache` at all.
            let text_ready = positioned.iter().all(|item| match item {
                // An embedded box's own nested text (if any) manages its
                // own cache-readiness independently, the normal way any
                // other recursively-painted `Laid` already does — this
                // check is only about the *outer* run's own directly-
                // owned word buffers, so `Embedded` is "ready" here
                // unconditionally, same as `Image` always was.
                MixedPositioned::Embedded(_) => true,
                MixedPositioned::Word { id, .. } => cache.contains_key(id),
            });
            for item in positioned {
                match item {
                    // A full, ordinary `Laid` — recurses through the
                    // exact same path a `Blocks`/`TableRow` child does,
                    // so it gets box-decoration/clipping/culling for
                    // free (see `MixedPositioned`'s own docs).
                    MixedPositioned::Embedded(img_laid) => {
                        paint(
                            img_laid, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h,
                            scroll_offset, child_clip,
                        );
                    }
                    MixedPositioned::Word { id, x, y, .. } => {
                        if !text_ready {
                            continue;
                        }
                        if let Some(entry) = cache.get(id) {
                            let mut guard = entry.lock().unwrap();
                            let (buffer, _, backgrounds, valigns, hidden, _, decoration_styles, _) = &mut *guard;
                            paint_buffer(
                                buffer, font_system, swash_cache, backgrounds, valigns, hidden, decoration_styles,
                                pixels, win_w, win_h, *x, *y - scroll_offset, child_clip,
                            );
                        }
                    }
                }
            }
        }
    }

    // `overflow: scroll`/`auto`'s own minimal scrollbar indicator
    // (v62) — this box's own chrome, painted in screen space (`draw_y`,
    // never shifted by `own_scroll` — the indicator itself doesn't
    // scroll, only the content behind it does) and clipped by the
    // *ambient* `clip`, not `child_clip`, same treatment the background/
    // border decoration above gets. `Scroll` always shows it; `Auto`
    // only once content actually overflows (see `paint_scrollbar`'s own
    // docs for the real difference between the two keywords).
    if let Some(info) = &laid.scroll {
        paint_scrollbar(
            info, own_scroll, s.overflow == Overflow::Scroll, pixels, win_w, win_h, laid.x, draw_y, laid.w, laid.h,
            clip,
        );
    }

    // Layers 6+7: this box's own out-of-flow `position: absolute`/
    // `fixed` descendants (`laid.positioned`, already sorted ascending
    // by `z-index` when it was built), merged with any direct `Blocks`/
    // `TableRow` child establishing its own non-negative-`z-index`
    // stacking context (`nonneg_stacking_children` above) — painted
    // *after* this box's own normal content, so they legitimately paint
    // on top of it and of each other. Unconditional (not gated on `s.
    // visibility`, unlike this box's own decoration above): visibility
    // is independent per element, and each entry's own recursive `paint`
    // call already checks *its own* `style.visibility`. Reuses `child_
    // clip` — the same bounds a normal descendant would be clipped to —
    // since each entry's containing block genuinely *is* this exact box
    // (real CSS: an `overflow: hidden` ancestor clips an absolutely
    // positioned descendant whenever that ancestor is at or inside its
    // containing block, which is always true here by construction).
    paint_z_layer(
        nonneg_positioned, &nonneg_stacking_children, font_system, swash_cache, cache, scroll_offsets, pixels, win_w,
        win_h, content_scroll_offset, child_clip,
    );
}

/// Finds the `CursorStyle` that should show at document-space `(x, y)` —
/// `y` already has any scroll offset added in (document space, same
/// space `Laid.x`/`.y` themselves are in, *not* the screen space mouse
/// events arrive in — see the `CursorMoved` handler). Falls back to a
/// box's own (already-inherited) `style.cursor` wherever no more
/// specific glyph match is found underneath it, same "ambient" layering
/// `paint`'s own box-vs-descendant painting already relies on. `cursor`
/// is real CSS inheritance (see `CursorStyle`'s own docs), so the
/// per-glyph `cursors` vector already reflects the correctly-cascaded
/// value for whichever span that glyph came from — no separate
/// propagation logic needed here either.
/// `scroll_offset` is the real, ambient page scroll offset — needed
/// here (unlike most of this function, which works in plain document
/// space throughout) only to translate into/out of a `position: fixed`
/// descendant's own viewport-space coordinates (see `Laid::fixed`'s own
/// docs — same translation `paint` does by passing `0.0` down instead,
/// but `hit_test_cursor` isn't recursive the same uniform way, since a
/// `fixed` descendant can appear at any depth's own `positioned` list),
/// and to compute a `position: sticky` child's own live, current
/// position (`sticky_shift` — see the `Blocks`/`TableRow` arm below).
/// `viewport_h` is only needed for *that*: `sticky_shift`'s own
/// `bottom`-offset math. `scroll_offsets` (v62) is the live per-element
/// `overflow: scroll`/`auto` state — see `ScrollInfo`'s own docs —
/// consulted only in the ordinary `Blocks`/`TableRow` loop below (the
/// "above" stacking/positioned-descendant loop deliberately isn't, same
/// documented cut `paint`'s own `paint_z_layer` calls just took).
///
/// Returns both the `CursorStyle` to show (what this function originally
/// returned, pre-link-navigation) and the `href` of whatever link, if
/// any, is under this exact point — link navigation (v65) reuses this
/// exact glyph-precision walk rather than a second, parallel one, since
/// it's answering the same underlying question ("which glyph is under
/// this point, and what span does it belong to") for a different event
/// (`WindowEvent::MouseInput`'s click handler instead of `CursorMoved`'s
/// hover). `href` has no box-level fallback the way `cursor` does
/// (`laid.style.cursor` always has *some* real cascaded value; there's
/// no equivalent "ambient box href" — only a per-glyph `SpanStyle::href`
/// exists at all, see its own docs), so every arm that falls back to
/// `laid.style.cursor` for the cursor half pairs it with a plain `None`
/// for the href half.
/// The element under a point as a `node_id` (0 if none), for Q31's
/// hover/active/focus tracking. Same walk as `hit_test_cursor`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hit_test_element(
    laid: &Laid, x: f32, y: f32, cache: &ShapeCache, scroll_offsets: &HashMap<usize, f32>, scroll_offset: f32,
    viewport_h: f32,
) -> usize {
    hit_test_node(laid, x, y, cache, scroll_offsets, scroll_offset, viewport_h).2
}

/// Anonymous boxes carry node 0: fall back to the enclosing element's id.
pub(crate) fn hit_test_fb(r: (CursorStyle, Option<&'static str>, usize), parent: usize) -> (CursorStyle, Option<&'static str>, usize) {
    if r.2 == 0 { (r.0, r.1, parent) } else { r }
}

/// `hit_test_node` without the element id (the original API).
#[allow(clippy::too_many_arguments)]
pub(crate) fn hit_test_cursor(
    laid: &Laid, x: f32, y: f32, cache: &ShapeCache, scroll_offsets: &HashMap<usize, f32>, scroll_offset: f32,
    viewport_h: f32,
) -> (CursorStyle, Option<&'static str>) {
    let (c, h, _) = hit_test_node(laid, x, y, cache, scroll_offsets, scroll_offset, viewport_h);
    (c, h)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn hit_test_node(
    laid: &Laid,
    x: f32,
    y: f32,
    cache: &ShapeCache,
    scroll_offsets: &HashMap<usize, f32>,
    scroll_offset: f32,
    viewport_h: f32,
) -> (CursorStyle, Option<&'static str>, usize) {
    hit_test_inner(laid, x, y, cache, scroll_offsets, scroll_offset, viewport_h).unwrap_or((CursorStyle::Auto, None, 0))
}

/// `None` = nothing in this subtree is under the point. Pruning uses
/// `Laid::ink` (the union of the box and all its descendants), not the
/// border box, since descendants can overflow it; the box *itself* only
/// counts as a hit when the point is inside its own border box.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hit_test_inner(
    laid: &Laid,
    x: f32,
    y: f32,
    cache: &ShapeCache,
    scroll_offsets: &HashMap<usize, f32>,
    scroll_offset: f32,
    viewport_h: f32,
) -> Option<(CursorStyle, Option<&'static str>, usize)> {
    if !laid.ink.contains(x, y) {
        return None;
    }
    let inside = x >= laid.x && x < laid.x + laid.w && y >= laid.y && y < laid.y + laid.h;
    let own = (laid.style.cursor, None, laid.style.node);
    // Same `is_explicit_stacking_child` pull-out `paint` uses, merged
    // with `laid.positioned` for one combined reversed-priority check
    // below — a direct `Blocks`/`TableRow` child establishing its own
    // stacking context visually paints outside the plain document-order
    // loop (same as an out-of-flow descendant), so it must win a hit
    // test the same way. **Deliberately not split by sign the way
    // `paint`'s own layering now is** — `paint`'s negative layer sits
    // *below* normal content (see its own docs), but a negative entry is
    // still checked here at the *same* (first/highest) priority as a
    // non-negative one: a narrow, documented approximation (see
    // TODO.txt) rather than a full mirror of that split, since a
    // negative stacking context's hit-test priority is rarely
    // observable in practice (it only wins a hit at all when nothing in
    // the normal-content layer happens to cover the same pixel).
    // Q24: this box's own live scroll shifts its stacking/positioned
    // descendants too, same as `paint`'s `content_scroll_offset`.
    let own_scroll = laid.scroll.map_or(0.0, |info| {
        scroll_offsets.get(&info.id).copied().unwrap_or(0.0).clamp(0.0, info.max_scroll())
    });
    let y = y + own_scroll;
    let scroll_offset = scroll_offset + own_scroll;
    let stacking_children: Vec<&Laid> = match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            children.iter().filter(|c| is_explicit_stacking_child(c)).collect()
        }
        _ => Vec::new(),
    };
    let mut above: Vec<&Laid> = stacking_children.clone();
    above.extend(laid.positioned.iter());
    above.sort_by_key(|l| l.style.z_index.unwrap_or(0));
    // Checked first, in reverse (highest `z-index`/last-painted first),
    // so a hit lands on whatever's actually on top. `.rev()` over an
    // ascending-`z-index` list, not a second sort.
    for item in above.iter().rev() {
        if matches!(item.style.position, Position::Relative | Position::Sticky) {
            // Mirror image of `paint`'s own trick (see the `Blocks`/
            // `TableRow` loop below): a sticky item's document-space
            // rect is shifted by `dy` from its stored (static) `x`/`y`;
            // a plain `relative` item has no `sticky` field at all, so
            // `dy` is always `0` for it (its offset is already baked
            // into `x`/`y` permanently).
            let dy = item.sticky.as_ref().map_or(0.0, |info| sticky_shift(info, item.h, scroll_offset, viewport_h));
            if let Some(r) = hit_test_inner(item, x, y - dy, cache, scroll_offsets, scroll_offset, viewport_h) {
                return Some(hit_test_fb(r, laid.style.node));
            }
        } else {
            let (ix, iy, inner_scroll) = if item.fixed { (x, y - scroll_offset, 0.0) } else { (x, y, scroll_offset) };
            if let Some(r) = hit_test_inner(item, ix, iy, cache, scroll_offsets, inner_scroll, viewport_h) {
                return Some(hit_test_fb(r, laid.style.node));
            }
        }
    }
    match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            // `overflow: scroll`/`auto` (v62): this box's own live
            // scroll amount shifts every *ordinary* descendant's own
            // effective position the same way `paint`'s own
            // `content_scroll_offset` does — see that computation's own
            // docs for the full derivation. Composes with a nested
            // sticky/scrollable descendant for free, same mechanism.
            for child in children.iter().filter(|c| !is_explicit_stacking_child(c)) {
                // Mirror image of `paint`'s own trick: a sticky child's
                // own *document-space* rect, for hit-testing purposes, is
                // shifted by `dy` from its stored (static) `x`/`y` — so
                // test against that shifted rect, but recurse with `y`
                // translated *back* (`y - dy`), since every descendant
                // position inside `child` is still stored relative to
                // its own static, unshifted position.
                if let Some(info) = &child.sticky {
                    let dy = sticky_shift(info, child.h, scroll_offset, viewport_h);
                    if let Some(r) = hit_test_inner(child, x, y - dy, cache, scroll_offsets, scroll_offset, viewport_h) {
                        return Some(hit_test_fb(r, laid.style.node));
                    }
                    continue;
                }
                if let Some(r) = hit_test_inner(child, x, y, cache, scroll_offsets, scroll_offset, viewport_h) {
                    return Some(hit_test_fb(r, laid.style.node));
                }
            }
            inside.then_some(own)
        }
        LaidContent::Text(id) => {
            if !inside {
                return None;
            }
            if let Some(entry) = cache.get(id) {
                let guard = entry.lock().unwrap();
                let (buffer, _, _, _, _, cursors, _, hrefs) = &*guard;
                let content_x = laid.x + laid.style.border_left() + laid.style.padding.left.px();
                let content_y = laid.y + laid.style.border_top() + laid.style.padding.top.px();
                let local_x = x - content_x;
                let local_y = y - content_y;
                for run in buffer.layout_runs() {
                    if local_y < run.line_top || local_y >= run.line_top + run.line_height {
                        continue;
                    }
                    for glyph in run.glyphs {
                        if local_x >= glyph.x && local_x < glyph.x + glyph.w {
                            let (href, node) = hrefs.get(glyph.metadata).copied().unwrap_or((None, 0));
                            return Some((
                                cursors.get(glyph.metadata).copied().unwrap_or(laid.style.cursor),
                                href,
                                if node != 0 { node } else { laid.style.node },
                            ));
                        }
                    }
                }
            }
            Some(own)
        }
        // No finer-grained hit test within an image — just this box's
        // own (already-cascaded) cursor, same fallback the other arms
        // use once nothing more specific matches underneath. (An
        // `<img>` directly inside an `<a>` isn't itself clickable this
        // way — `href` only ever reaches a `SpanStyle`, never an image's
        // own box — see `SpanStyle::href`'s own docs; a deliberate,
        // narrow scope cut, same spirit as other "images don't carry
        // the per-span machinery text does" gaps already in TODO.txt.)
        LaidContent::Image(_) => inside.then_some(own),
        LaidContent::MixedInline(positioned) => {
            for item in positioned {
                match item {
                    MixedPositioned::Embedded(img_laid) => {
                        if let Some(r) = hit_test_inner(img_laid, x, y, cache, scroll_offsets, scroll_offset, viewport_h) {
                            return Some(hit_test_fb(r, laid.style.node));
                        }
                    }
                    MixedPositioned::Word { id, x: wx, y: wy, w, h } => {
                        if x < *wx || x >= *wx + *w || y < *wy || y >= *wy + *h {
                            continue;
                        }
                        if let Some(entry) = cache.get(id) {
                            let guard = entry.lock().unwrap();
                            let (buffer, _, _, _, _, cursors, _, hrefs) = &*guard;
                            let local_x = x - wx;
                            let local_y = y - wy;
                            for run in buffer.layout_runs() {
                                if local_y < run.line_top || local_y >= run.line_top + run.line_height {
                                    continue;
                                }
                                for glyph in run.glyphs {
                                    if local_x >= glyph.x && local_x < glyph.x + glyph.w {
                                        let (href, node) = hrefs.get(glyph.metadata).copied().unwrap_or((None, 0));
                                        return Some((
                                            cursors.get(glyph.metadata).copied().unwrap_or(laid.style.cursor),
                                            href,
                                            if node != 0 { node } else { laid.style.node },
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            inside.then_some(own)
        }
    }
}

/// Finds the deepest `overflow: scroll`/`auto` box whose own border box
/// contains document-space point `(x, y)` — what `WindowEvent::
/// MouseWheel` (v62) needs to route a wheel tick to the right scroll
/// region instead of always the page's own top-level scroll. Same
/// coordinate conventions as `hit_test_cursor` (`y` already has the
/// live ambient scroll added in, same as the `CursorMoved` handler
/// already does for that function).
///
/// Deliberately narrower than `hit_test_cursor`'s own walk: only
/// ordinary `Blocks`/`TableRow` children are descended into (including
/// the same `position: sticky` translation `hit_test_cursor` needs,
/// since a sticky child with no explicit `z-index` still appears in
/// that ordinary list) — a `position: relative`/`sticky` child that
/// *does* establish its own explicit stacking context, or any
/// `position: absolute`/`fixed` descendant, is not walked into at all
/// (see TODO.txt: such content inside a scroll region doesn't actually
/// scroll with it either, the same documented cut `paint`'s own
/// `paint_z_layer` calls took, so there's nothing additional to find by
/// walking into it here).
///
/// Bubbles the innermost match up via `Option::or` — if a deeper call
/// finds no scrollable box at all, falls back to whichever scrollable
/// *ancestor* (if any) contains the point, exactly real "wheel bubbles
/// to the nearest scrollable ancestor" behavior; `None` all the way up
/// means "nothing scrollable under the cursor at all," the caller's own
/// cue to fall back to the page's own top-level scroll.
pub(crate) fn hit_test_scroll_region(
    laid: &Laid,
    x: f32,
    y: f32,
    scroll_offsets: &HashMap<usize, f32>,
    scroll_offset: f32,
    viewport_h: f32,
) -> Option<ScrollInfo> {
    if !laid.ink.contains(x, y) {
        return None;
    }
    let inside = x >= laid.x && x < laid.x + laid.w && y >= laid.y && y < laid.y + laid.h;
    let self_info = if inside { laid.scroll } else { None };
    let (LaidContent::Blocks(children) | LaidContent::TableRow(children)) = &laid.content else {
        return self_info;
    };
    // Same `own_scroll` shift `hit_test_cursor`'s own `Blocks`/
    // `TableRow` arm applies, for the same reason — see that
    // computation's own docs.
    let own_scroll =
        laid.scroll.map_or(0.0, |info| scroll_offsets.get(&info.id).copied().unwrap_or(0.0).clamp(0.0, info.max_scroll()));
    let child_y = y + own_scroll;
    let child_scroll_offset = scroll_offset + own_scroll;
    for child in children.iter().filter(|c| !is_explicit_stacking_child(c)) {
        if matches!(child.style.position, Position::Absolute | Position::Fixed) {
            // Never actually appears here (see `measure`'s own `Blocks`
            // arm — diverted to `own_pending`/`positioned` instead),
            // defensive only.
            continue;
        }
        if let Some(info) = &child.sticky {
            let dy = sticky_shift(info, child.h, child_scroll_offset, viewport_h);
            if let Some(sub) =
                hit_test_scroll_region(child, x, child_y - dy, scroll_offsets, child_scroll_offset, viewport_h)
            {
                return Some(sub);
            }
            continue;
        }
        if let Some(sub) = hit_test_scroll_region(child, x, child_y, scroll_offsets, child_scroll_offset, viewport_h) {
            return Some(sub);
        }
    }
    self_info
}
