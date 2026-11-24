//! C6b: HiDPI and fractional scale. The shell window's scale factor (winit's) maps the
//! logical layout (page, bar, pointer, popups) to the framebuffer; the bar is painted at the
//! framebuffer's resolution.

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::{Anchor, ConstraintAdjustment, Gravity};
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use smithay::wayland::shell::xdg::PositionerState;

use crate::config::STATUS_BAR_H;
use crate::keys::BarView;
use crate::layout::{logical_size, sanitize_scale, Layout, Region};
use crate::statusbar::Painter;

fn phys(w: i32, h: i32) -> Size<i32, Physical> {
    (w, h).into()
}

#[test]
fn scale_factors_are_kept_in_120ths_and_nonsense_is_1() {
    for s in [1.0, 1.25, 1.5, 1.75, 2.0, 3.0] {
        assert_eq!(sanitize_scale(s), s);
    }
    assert_eq!(sanitize_scale(1.2499999), 1.25);
    // fractional-scale-v1 can only say n/120.
    assert_eq!(sanitize_scale(1.0 + 1.0 / 240.0 + 1e-9), 1.0 + 1.0 / 120.0);
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(sanitize_scale(bad), 1.0, "{bad}");
    }
    assert_eq!(sanitize_scale(100.0), 8.0);
}

#[test]
fn logical_size_inverts_the_desktops_rounding() {
    // A desktop at scale s gives a window of logical size L round(L * s) pixels.
    for s in [1.0, 1.25, 1.5, 1.75, 2.0, 2.25] {
        for l in [1, 23, 24, 25, 480, 777, 956, 1002, 1920] {
            let p = (l as f64 * s).round() as i32;
            assert_eq!(
                logical_size(phys(p, p), s),
                Size::<i32, Logical>::from((l, l)),
                "scale {s}, logical {l}"
            );
        }
    }
    assert_eq!(
        logical_size(phys(0, 0), 2.0),
        Size::from((1, 1)),
        "never 0 (a configure of 0 means: pick a size)"
    );
}

#[test]
fn scale_1_is_the_old_layout() {
    let l = Layout::from_physical(phys(1000, 800), 1.0);
    assert_eq!(l, Layout::new(1000, 800));
    assert_eq!(l.bar_physical(), Some(((0, 776).into(), (1000, 24).into())));
}

#[test]
fn scale_2_page_and_bar_are_logical_bar_pixels_doubled() {
    let l = Layout::from_physical(phys(1920, 2012), 2.0);
    assert_eq!(l.window, Size::from((960, 1006)));
    // The page is configured in logical px (the client draws 2x buffers).
    assert_eq!(l.page(), Rectangle::new((0, 0).into(), (960, 1006 - STATUS_BAR_H).into()));
    // The bar keeps its logical height; its pixels are doubled and reach the bottom.
    assert_eq!(l.bar().unwrap().size.h, STATUS_BAR_H);
    let (loc, size) = l.bar_physical().unwrap();
    assert_eq!(loc, Point::from((0, 2 * (1006 - STATUS_BAR_H))));
    assert_eq!(size, Size::from((1920, 2 * STATUS_BAR_H)));
    assert_eq!(loc.y + size.h, 2012);
}

#[test]
fn fractional_scales_cover_the_framebuffer_without_gaps() {
    for s in [1.25, 1.5, 1.75] {
        for (lw, lh) in [(960, 1006), (1280, 800), (777, 555)] {
            let p = phys((lw as f64 * s).round() as i32, (lh as f64 * s).round() as i32);
            let l = Layout::from_physical(p, s);
            assert_eq!(l.window, Size::from((lw, lh)));
            let (loc, size) = l.bar_physical().unwrap();
            // The bar starts where the page's pixels end ...
            let page = l.page();
            assert_eq!(loc.y, ((page.loc.y + page.size.h) as f64 * s).round() as i32, "scale {s}");
            // ... spans the framebuffer's width and ends at its bottom edge.
            assert_eq!((loc.x, size.w), (0, p.w), "scale {s}");
            assert_eq!(loc.y + size.h, p.h, "scale {s} {lw}x{lh}");
            // Its pixel height is the logical height scaled (within the rounding).
            assert!((size.h as f64 - STATUS_BAR_H as f64 * s).abs() <= 1.0, "scale {s}: {size:?}");
        }
    }
    // 1.5 at the nest's default size: 1440x1509 px is 960x1006 logical, the bar 36 px tall.
    let l = Layout::from_physical(phys(1440, 1509), 1.5);
    assert_eq!(l.window, Size::from((960, 1006)));
    assert_eq!(l.bar_physical(), Some(((0, 1473).into(), (1440, 36).into())));
}

#[test]
fn fullscreen_at_a_scale_has_no_bar() {
    let mut l = Layout::from_physical(phys(2880, 1800), 1.5);
    l.fullscreen = true;
    assert_eq!(l.page().size, Size::from((1920, 1200)));
    assert_eq!(l.bar_physical(), None);
}

#[test]
fn a_scale_change_keeps_fullscreen_and_the_logical_size() {
    // Moving the window from a scale 1 monitor to a scale 2 one: winit reports the same
    // logical window with twice the pixels.
    let mut l = Layout::from_physical(phys(1280, 800), 1.0);
    l.fullscreen = true;
    l.set_physical(phys(2560, 1600), 2.0);
    assert!(l.fullscreen);
    assert_eq!((l.window, l.scale), (Size::from((1280, 800)), 2.0));
    l.fullscreen = false;
    l.set_physical(phys(1600, 1000), 1.25);
    assert_eq!(l.window, Size::from((1280, 800)));
    assert_eq!(l.page().size, Size::from((1280, 800 - STATUS_BAR_H)));
}

#[test]
fn pointer_positions_are_logical_at_any_scale() {
    // smithay's winit backend gives the pointer as a fraction of the window; the shell maps
    // it onto the logical window (`position_transformed(layout.window)`), so the regions,
    // the surface lookup and the page all see logical coordinates.
    let l = Layout::from_physical(phys(1920, 2012), 2.0);
    let at = |fx: f64, fy: f64| -> Point<f64, Logical> { (fx * l.window.w as f64, fy * l.window.h as f64).into() };
    // The physical pixel (1900, 1990) is in the bar; (1900, 1900) is on the page.
    assert_eq!(l.region(at(1900.0 / 1920.0, 1990.0 / 2012.0)), Region::Bar);
    assert_eq!(l.region(at(1900.0 / 1920.0, 1900.0 / 2012.0)), Region::Page);
    let p = at(1900.0 / 1920.0, 1900.0 / 2012.0);
    assert_eq!((p.x, p.y), (950.0, 950.0));
}

#[test]
fn popups_are_constrained_in_logical_px_at_a_scale() {
    // C2-POPUP's case at scale 2: a select flush right in a 956 px (logical) window, 1912 px.
    let l = Layout::from_physical(phys(1912, 1600), 2.0);
    let target = l.popup_target(l.page().loc, (0, 0).into());
    assert_eq!(target, Rectangle::new((0, 0).into(), (956, 800).into()));
    let pos = PositionerState {
        rect_size: (410, 120).into(),
        anchor_rect: Rectangle::new((880, 100).into(), (68, 24).into()),
        anchor_edges: Anchor::BottomLeft,
        gravity: Gravity::BottomRight,
        constraint_adjustment: ConstraintAdjustment::None,
        ..Default::default()
    };
    let g = l.constrain_popup(pos, target);
    assert!(g.loc.x >= 0 && g.loc.x + g.size.w <= 956, "{g:?}");
}

fn view(mode: &str) -> BarView {
    BarView {
        mode: mode.into(),
        url: String::new(),
        message: String::new(),
        line: None,
        ghost: String::new(),
        flash: false,
    }
}

/// The ink's bounding box (x0, y0, x1, y1), pixels that differ from the top left one.
fn ink(pm: &tiny_skia::Pixmap) -> Option<(u32, u32, u32, u32)> {
    let bg = pm.pixel(0, 0).unwrap();
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for y in 0..pm.height() {
        for x in 0..pm.width() {
            if pm.pixel(x, y).unwrap() != bg {
                b = Some(match b {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    b
}

#[test]
fn the_bar_is_painted_at_the_scale_not_stretched() {
    let mut p = Painter::new();
    let h = STATUS_BAR_H as u32;
    // Sizes: the pixmap is exactly the physical strip.
    let one = p.paint_scaled(&view("NORMAL"), 400, h, 1.0).unwrap();
    let two = p.paint_scaled(&view("NORMAL"), 800, 2 * h, 2.0).unwrap();
    let frac = p.paint_scaled(&view("NORMAL"), 600, 36, 1.5).unwrap();
    assert_eq!((one.width(), one.height()), (400, h));
    assert_eq!((two.width(), two.height()), (800, 2 * h));
    assert_eq!((frac.width(), frac.height()), (600, 36));
    if !p.has_fonts() {
        eprintln!("no fonts on this machine: text checks skipped");
        return;
    }
    // The text is rasterized at twice the size (font px, padding), not drawn at 1x.
    let (x0_1, y0_1, x1_1, y1_1) = ink(&one).unwrap();
    let (x0_2, y0_2, x1_2, y1_2) = ink(&two).unwrap();
    let r = |a: u32, b: u32| b as f64 / a.max(1) as f64;
    assert!((1.8..=2.2).contains(&r(x0_1, x0_2)), "left padding {x0_1} -> {x0_2}");
    assert!(
        (1.8..=2.2).contains(&r(x1_1 - x0_1, x1_2 - x0_2)),
        "text width {} -> {}",
        x1_1 - x0_1,
        x1_2 - x0_2
    );
    assert!(
        (1.7..=2.3).contains(&r(y1_1 - y0_1, y1_2 - y0_2)),
        "glyph height {} -> {}",
        y1_1 - y0_1,
        y1_2 - y0_2
    );
    // At 1.5 the glyphs are 1.5 times as tall: rasterized at that size, not at 1x and stretched.
    let (_, y0_f, _, y1_f) = ink(&frac).unwrap();
    assert!(
        (1.3..=1.7).contains(&r(y1_1 - y0_1, y1_f - y0_f)),
        "glyph height at 1.5: {}",
        y1_f - y0_f
    );
}
