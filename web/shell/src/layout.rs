//! Where things go in the shell window (logical pixels, origin top left).
//!
//! The page's window fills the shell window minus the status bar at the bottom (KEYS.md
//! "Status bar": the page never overlaps it). In page fullscreen the page gets the whole
//! window and the bar is hidden.
//!
//! C6b: the shell window's scale factor (winit's: the desktop's output scale, possibly
//! fractional) maps logical to physical pixels. Everything here is logical except
//! `physical` and `bar_physical` (the framebuffer's pixels).

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::ConstraintAdjustment;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use smithay::wayland::shell::xdg::PositionerState;

/// The status bar's height (`config::STATUS_BAR_H`).
pub const BAR_H: i32 = crate::config::STATUS_BAR_H;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// The shell window, logical pixels.
    pub window: Size<i32, Logical>,
    pub fullscreen: bool,
    /// The shell window's scale factor (1, 1.25, 1.5, 2, ...), see `sanitize_scale`.
    pub scale: f64,
    /// The shell window's framebuffer, physical pixels.
    pub physical: Size<i32, Physical>,
}

/// A scale factor the outputs can carry: fractional-scale-v1 speaks in 120ths; a nonsense
/// value (0, NaN) is 1.
pub fn sanitize_scale(s: f64) -> f64 {
    if !s.is_finite() || s <= 0.0 {
        return 1.0;
    }
    ((s * 120.0).round() / 120.0).clamp(0.25, 8.0)
}

/// The logical size of a framebuffer of `phys` pixels at `scale`, rounded: desktops size a
/// fractionally scaled window as round(logical * scale), so this gets the logical size back.
pub fn logical_size(phys: Size<i32, Physical>, scale: f64) -> Size<i32, Logical> {
    let l = |v: i32| ((v as f64 / scale).round() as i32).max(1);
    (l(phys.w), l(phys.h)).into()
}

/// What a pointer position in the shell window is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Page,
    Bar,
    Outside,
}

impl Layout {
    /// A scale 1 layout (logical and physical sizes are the same).
    pub fn new(w: i32, h: i32) -> Layout {
        Layout {
            window: Size::from((w, h)),
            fullscreen: false,
            scale: 1.0,
            physical: Size::from((w, h)),
        }
    }

    /// The layout of a framebuffer of `phys` pixels at scale factor `scale`.
    pub fn from_physical(phys: Size<i32, Physical>, scale: f64) -> Layout {
        let mut l = Layout::new(1, 1);
        l.set_physical(phys, scale);
        l
    }

    /// The shell window got a new framebuffer size or scale; fullscreen is kept.
    pub fn set_physical(&mut self, phys: Size<i32, Physical>, scale: f64) {
        self.scale = sanitize_scale(scale);
        self.physical = phys;
        self.window = logical_size(phys, self.scale);
    }

    /// Where the status bar's pixels go in the framebuffer and how many there are. The bar
    /// is painted into a buffer of exactly this size and drawn 1:1 (crisp at a fractional
    /// scale). Each edge is rounded on its own, as smithay rounds its render elements, so the
    /// element covers exactly these pixels.
    pub fn bar_physical(&self) -> Option<(Point<i32, Physical>, Size<i32, Physical>)> {
        let bar = self.bar()?;
        let r = |v: i32| (v as f64 * self.scale).round() as i32;
        let (x0, y0) = (r(bar.loc.x), r(bar.loc.y));
        let (x1, y1) = (r(bar.loc.x + bar.size.w), r(bar.loc.y + bar.size.h));
        Some(((x0, y0).into(), ((x1 - x0).max(1), (y1 - y0).max(1)).into()))
    }

    fn bar_h(&self) -> i32 {
        if self.fullscreen {
            0
        } else {
            // A window shorter than the bar still leaves the page one pixel.
            BAR_H.min((self.window.h - 1).max(0))
        }
    }

    /// The page window's place and size; never smaller than 1x1 (a configure of 0 means
    /// "pick your own size" in xdg_shell).
    pub fn page(&self) -> Rectangle<i32, Logical> {
        let w = self.window.w.max(1);
        let h = (self.window.h - self.bar_h()).max(1);
        Rectangle::new((0, 0).into(), (w, h).into())
    }

    /// The status bar strip, none in fullscreen.
    pub fn bar(&self) -> Option<Rectangle<i32, Logical>> {
        let h = self.bar_h();
        (h > 0).then(|| Rectangle::new((0, self.window.h - h).into(), (self.window.w.max(1), h).into()))
    }

    pub fn region(&self, p: Point<f64, Logical>) -> Region {
        let inside = |r: Rectangle<i32, Logical>| {
            p.x >= r.loc.x as f64 && p.y >= r.loc.y as f64 && p.x < (r.loc.x + r.size.w) as f64 && p.y < (r.loc.y + r.size.h) as f64
        };
        if inside(self.page()) {
            Region::Page
        } else if self.bar().is_some_and(inside) {
            Region::Bar
        } else {
            Region::Outside
        }
    }

    /// Where a dialog-like toplevel (one that is not the page) goes: centred over the page,
    /// its top left kept inside the page.
    pub fn dialog_loc(&self, size: Size<i32, Logical>) -> Point<i32, Logical> {
        let page = self.page();
        let x = page.loc.x + ((page.size.w - size.w) / 2).max(0);
        let y = page.loc.y + ((page.size.h - size.h) / 2).max(0);
        (x, y).into()
    }

    /// The xdg_positioner constraint area for a popup: the whole shell window (a menu may
    /// cover the status bar), in the coordinates of the popup's parent surface.
    /// `root_loc`: where the popup's toplevel is in the shell window; `parent_offset`:
    /// the parent's offset inside that toplevel (0 for a popup on the toplevel itself,
    /// the parent popup's position for nested menus).
    pub fn popup_target(&self, root_loc: Point<i32, Logical>, parent_offset: Point<i32, Logical>) -> Rectangle<i32, Logical> {
        let mut r = Rectangle::new((0, 0).into(), self.window);
        r.loc -= root_loc;
        r.loc -= parent_offset;
        r
    }

    /// Returns `positioner` with SlideX|SlideY|FlipY added to its constraint adjustment. Chromium's
    /// select popups request none, which leaves smithay's `get_unconstrained_geometry` a no-op and
    /// the dropdown outside the window. Slide keeps it inside at the right/left/top/bottom edges;
    /// FlipY puts it above the anchor when it does not fit below (a select at the bottom edge), and
    /// the slide that follows catches what is still out.
    pub fn constrain_popup(&self, mut positioner: PositionerState, target: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        positioner.constraint_adjustment |= ConstraintAdjustment::SlideX | ConstraintAdjustment::SlideY | ConstraintAdjustment::FlipY;
        positioner.get_unconstrained_geometry(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::{Anchor, ConstraintAdjustment, Gravity};

    #[test]
    fn page_is_window_minus_bar() {
        let l = Layout::new(1000, 800);
        assert_eq!(l.page(), Rectangle::new((0, 0).into(), (1000, 776).into()));
        assert_eq!(l.bar(), Some(Rectangle::new((0, 776).into(), (1000, 24).into())));
    }

    #[test]
    fn fullscreen_page_fills_the_window() {
        let l = Layout {
            fullscreen: true,
            ..Layout::new(1920, 1080)
        };
        assert_eq!(l.page(), Rectangle::new((0, 0).into(), (1920, 1080).into()));
        assert_eq!(l.bar(), None);
        assert_eq!(l.region((5.0, 1079.0).into()), Region::Page);
    }

    #[test]
    fn tiny_windows_keep_a_page() {
        for (w, h) in [(0, 0), (1, 1), (10, 10), (10, 24), (10, 25)] {
            let p = Layout::new(w, h).page();
            assert!(p.size.w >= 1 && p.size.h >= 1, "{w}x{h}: {p:?}");
        }
        assert_eq!(Layout::new(10, 25).page().size.h, 1);
        assert_eq!(Layout::new(10, 25).bar().unwrap().size.h, 24);
    }

    #[test]
    fn regions() {
        let l = Layout::new(1000, 800);
        assert_eq!(l.region((0.0, 0.0).into()), Region::Page);
        assert_eq!(l.region((999.9, 775.9).into()), Region::Page);
        assert_eq!(l.region((500.0, 776.0).into()), Region::Bar);
        assert_eq!(l.region((500.0, 799.5).into()), Region::Bar);
        assert_eq!(l.region((500.0, 800.0).into()), Region::Outside);
        assert_eq!(l.region((-1.0, 10.0).into()), Region::Outside);
    }

    #[test]
    fn dialogs_are_centred_over_the_page() {
        let l = Layout::new(1000, 824);
        assert_eq!(l.dialog_loc((400, 200).into()), Point::from((300, 300)));
        // Larger than the page: pinned to its top left.
        assert_eq!(l.dialog_loc((2000, 2000).into()), Point::from((0, 0)));
    }

    fn positioner(anchor_rect: Rectangle<i32, Logical>, size: (i32, i32)) -> PositionerState {
        PositionerState {
            rect_size: size.into(),
            anchor_rect,
            anchor_edges: Anchor::BottomLeft,
            gravity: Gravity::BottomRight,
            constraint_adjustment: ConstraintAdjustment::FlipY | ConstraintAdjustment::SlideX | ConstraintAdjustment::SlideY,
            ..Default::default()
        }
    }

    #[test]
    fn select_near_the_bottom_flips_up_and_may_cover_the_bar() {
        let l = Layout::new(1000, 800);
        // A <select> 30 px tall at y=740 on the page; its 200 px menu does not fit below.
        let pos = positioner(Rectangle::new((100, 740).into(), (150, 30).into()), (150, 200));
        let target = l.popup_target(l.page().loc, (0, 0).into());
        let g = pos.get_unconstrained_geometry(target);
        assert_eq!(g, Rectangle::new((100, 540).into(), (150, 200).into()));
        // A menu that fits below the select (630..790), over the bar strip: left where it is.
        let pos = positioner(Rectangle::new((100, 600).into(), (150, 30).into()), (150, 160));
        assert_eq!(pos.get_unconstrained_geometry(target).loc, Point::from((100, 630)));
    }

    #[test]
    fn context_menu_at_the_right_edge_slides_left() {
        let l = Layout::new(1000, 800);
        let pos = positioner(Rectangle::new((950, 100).into(), (1, 1).into()), (220, 300));
        let g = pos.get_unconstrained_geometry(l.popup_target(l.page().loc, (0, 0).into()));
        assert_eq!(g.loc.x + g.size.w, 1000);
    }

    #[test]
    fn popup_without_constraint_adjustment_is_kept_inside() {
        let l = Layout::new(956, 800);
        let target = l.popup_target(l.page().loc, (0, 0).into());
        let mut pos = positioner(Rectangle::new((880, 100).into(), (68, 24).into()), (410, 120));
        pos.constraint_adjustment = ConstraintAdjustment::None;
        // The positioner alone leaves it outside (the bug) ...
        assert!(pos.get_unconstrained_geometry(target).loc.x + 410 > 956);
        let g = l.constrain_popup(pos, target);
        assert!(g.loc.x >= 0 && g.loc.x + g.size.w <= 956, "{g:?}");
        // Bottom edge: a dropdown that does not fit below stays inside the window.
        let mut pos = positioner(Rectangle::new((880, 770).into(), (68, 24).into()), (410, 200));
        pos.constraint_adjustment = ConstraintAdjustment::None;
        let g = l.constrain_popup(pos, target);
        assert!(g.loc.y >= 0 && g.loc.y + g.size.h <= 800, "{g:?}");
        assert!(g.loc.x + g.size.w <= 956, "{g:?}");
    }

    #[test]
    fn nested_popup_target_is_in_parent_coordinates() {
        let l = Layout::new(1000, 800);
        let t = l.popup_target((0, 0).into(), (300, 200).into());
        assert_eq!(t, Rectangle::new((-300, -200).into(), (1000, 800).into()));
        // A dialog's popup: its toplevel is not at the origin.
        let t = l.popup_target((300, 300).into(), (0, 0).into());
        assert_eq!(t.loc, Point::from((-300, -300)));
    }
}
