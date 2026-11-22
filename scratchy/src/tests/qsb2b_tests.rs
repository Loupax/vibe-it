// QSB2b: horizontal scrollbars (window and overflow-x regions), the corner, axis-generic maths.
#[allow(unused_imports)]
use super::*;
use crate::layout::xk;
use crate::scrollbar::{self as sb, Axis, BarHit};
use std::time::Instant;

const SURFACE: (u32, u32) = (800, 624);

#[test]
fn qsb2b_x_track_is_the_bottom_strip_and_stops_short_of_the_corner() {
    let rect = (10.0, 20.0, 400.0, 100.0);
    assert_eq!(sb::track_rect_axis(Axis::X, rect, false), Some((10.0, 112.0, 400.0, 8.0)));
    assert_eq!(sb::track_rect_axis(Axis::X, rect, true), Some((10.0, 112.0, 392.0, 8.0)));
    assert_eq!(sb::track_rect_axis(Axis::Y, rect, true), Some((402.0, 20.0, 8.0, 92.0)));
    assert_eq!(sb::track_rect_axis(Axis::Y, rect, false), Some((402.0, 20.0, 8.0, 100.0)));
    assert_eq!(sb::corner_rect(rect), Some((402.0, 112.0, 8.0, 8.0)));
    assert_eq!(sb::track_rect_axis(Axis::X, (0.0, 0.0, 100.0, 5.0), false), None, "box shorter than the track");
}

#[test]
fn qsb2b_x_hit_test_uses_x_along_the_track() {
    let rect = (0.0, 0.0, 400.0, 100.0);
    // content 1000 through 400: thumb 156.8 long in a 392 px track (corner), at 0.
    let h = |px: f32, py: f32| sb::hit_axis(Axis::X, true, rect, 1000.0, 400.0, 0.0, px, py);
    assert_eq!(h(10.0, 96.0), Some(BarHit::Thumb));
    assert_eq!(h(300.0, 96.0), Some(BarHit::TrackAfter));
    assert_eq!(h(300.0, 50.0), None, "above the strip");
    assert_eq!(h(396.0, 96.0), None, "the corner square is no track");
    assert_eq!(sb::hit_axis(Axis::X, false, rect, 400.0, 400.0, 0.0, 10.0, 96.0), None, "nothing to scroll");
    // At the end the thumb sits at the far end: a press at the start is before it.
    assert_eq!(sb::hit_axis(Axis::X, true, rect, 1000.0, 400.0, 600.0, 10.0, 96.0), Some(BarHit::TrackBefore));
}

#[test]
fn qsb2b_page_viewport_loses_the_track_height_while_the_bar_shows() {
    let (w, h) = page_viewport(SURFACE);
    assert_eq!(page_viewport_hb(SURFACE, false), (w, h));
    assert_eq!(page_viewport_hb(SURFACE, true), (w, h - crate::config::TRACK_W as u32));
}

fn app_with(html: &str) -> Option<App> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    let doc = parse_html(html);
    let mut app = App::new_static(doc, event_loop.create_proxy(), None);
    app.surface_size = SURFACE;
    let tree = build_layout_tree(&app.document, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut incomplete) = (0usize, false);
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut font_system, 800.0, 600.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    app.cached = Some(Rc::new(laid));
    Some(app)
}

const WIDE: &str = "<html><body style='margin:0'><div style='width:2000px;height:10px'></div></body></html>";
const WIDE_TALL: &str = "<html><body style='margin:0'><div style='width:2000px;height:3000px'></div></body></html>";

#[test]
fn qsb2b_window_hbar_shows_for_a_wide_page_only() {
    let Some(app) = app_with(WIDE) else { return };
    assert!(App::want_hbar(&app.cached, 800));
    let Some(narrow) = app_with("<html><body style='margin:0'><p>hi</p></body></html>") else { return };
    assert!(!App::want_hbar(&narrow.cached, 800));
    let Some(hidden) = app_with("<html><body style='margin:0;overflow-x:hidden'><div style='width:2000px;height:10px'></div></body></html>") else { return };
    assert!(!App::want_hbar(&hidden.cached, 800), "a hidden viewport x has no bar");
}

#[test]
fn qsb2b_window_hbar_sits_above_the_status_bar_and_the_page_is_shorter() {
    let Some(mut app) = app_with(WIDE) else { return };
    let full = app.page_size().1;
    app.hbar = true;
    let page = app.page_size();
    assert_eq!(page.1, full - 8);
    let (rect, content_w, client_w) = App::viewport_bar_x(&app.cached, page).expect("bar");
    assert_eq!(rect, (0.0, page.1 as f32, 800.0, 8.0));
    assert_eq!(rect.1 + rect.3, (SURFACE.1 - crate::config::STATUS_BAR_H) as f32, "ends where the status bar starts");
    assert_eq!((content_w, client_w), (2000.0, 800.0));
}

#[test]
fn qsb2b_window_hbar_track_click_pages_sideways_and_hold_repeats() {
    let Some(mut app) = app_with(WIDE) else { return };
    app.hbar = true;
    let y = (app.page_size().1 + 4) as f32;
    // thumb is 800 * 800/2000 = 320 px at 0: a press at x 600 is after it.
    assert!(app.scrollbar_press((600.0, y)));
    assert_eq!(app.scroll_x, 800.0, "a page is the window width");
    assert_eq!(app.scroll_offset, 0.0);
    assert!(app.sb_hold.is_some_and(|h| h.axis == Axis::X && h.dir == BarHit::TrackAfter));
    // The hold repeats while the pointer stays after the thumb.
    app.last_mouse_pos = Some((790.0, y));
    app.sb_hold = app.sb_hold.map(|h| crate::app::SbHold { next: Instant::now(), ..h });
    app.scrollbar_hold_tick();
    assert_eq!(app.scroll_x, 1200.0, "clamped to the page's end (2000 - 800)");
    // A press before the thumb pages back.
    app.sb_hold = None;
    assert!(app.scrollbar_press((5.0, y)));
    assert_eq!(app.scroll_x, 400.0);
}

#[test]
fn qsb2b_window_hbar_thumb_drag_is_instant_and_proportional() {
    let Some(mut app) = app_with(WIDE) else { return };
    app.hbar = true;
    let y = (app.page_size().1 + 4) as f32;
    assert!(app.scrollbar_press((100.0, y)));
    assert!(app.sb_drag.is_some_and(|d| d.axis == Axis::X));
    assert!(app.sb_hold.is_none(), "a thumb press is no hold");
    // track 800, thumb 320: free 480, ratio 1200/480 = 2.5.
    app.scrollbar_drag_move((140.0, y));
    assert_eq!(app.scroll_x, 100.0);
    app.scrollbar_drag_move((5000.0, 0.0));
    assert_eq!(app.scroll_x, 1200.0, "clamped, and the y of the pointer is ignored");
    assert_eq!(app.scroll_offset, 0.0);
}

#[test]
fn qsb2b_both_window_bars_leave_the_corner_to_the_square() {
    let Some(mut app) = app_with(WIDE_TALL) else { return };
    app.hbar = true;
    let page = app.page_size();
    let y = (page.1 + 4) as f32;
    // The x track stops 8 px short of the right edge: the corner is no bar.
    assert!(!app.scrollbar_press((797.0, y)), "corner square");
    // The vertical bar ends above the x bar: its last px is on the track.
    assert!(app.scrollbar_press((797.0, (page.1 - 2) as f32)));
    assert!(app.scroll_offset > 0.0);
    // Painting: track tint in the strip, in the corner too.
    let mut pixels = vec![0x00FF_FFFFu32; (SURFACE.0 * SURFACE.1) as usize];
    let rect = (0.0, page.1 as f32, 800.0, 8.0);
    crate::paint::paint_viewport_hbar(2000.0, 800.0, 0.0, true, &mut pixels, SURFACE.0, SURFACE.1, rect);
    let at = |x: u32, y: u32| pixels[(y * SURFACE.0 + x) as usize];
    assert_ne!(at(400, page.1 + 4), 0x00FF_FFFF, "the track is painted");
    assert_ne!(at(797, page.1 + 4), 0x00FF_FFFF, "the corner is painted");
    assert_eq!(at(400, page.1 - 1), 0x00FF_FFFF, "nothing above the strip");
}

const BOX: &str = "<html><body style='margin:0'><div style='overflow-x:auto;overflow-y:hidden;width:200px;height:100px'>\
    <div style='width:1000px;height:20px'></div></div></body></html>";

#[test]
fn qsb2b_region_hbar_click_and_drag_move_the_regions_x_offset() {
    let Some(mut app) = app_with(BOX) else { return };
    let id = {
        let laid = app.cached.clone().unwrap();
        crate::paint::hit_test_scroll_chain(&laid, 10.0, 10.0, &app.scroll_offsets, 0.0, 600.0)[0].id
    };
    // The box is 200 x 100: the bar is its bottom 8 px (y 92..100), track 200 (no y bar), thumb 40.
    assert!(!app.scrollbar_press((100.0, 50.0)), "the box body is not a bar");
    assert!(app.scrollbar_press((150.0, 96.0)));
    assert_eq!(app.scroll_offsets.get(&xk(id)).copied(), Some(200.0), "a page is the box width");
    assert!(!app.scroll_offsets.contains_key(&id), "y untouched");
    assert_eq!(app.scroll_x, 0.0, "the window did not move");
    app.sb_hold = None;
    // The thumb is at 0: press on it and drag 80 px right.
    app.scroll_offsets.insert(xk(id), 0.0);
    assert!(app.scrollbar_press((10.0, 96.0)));
    assert!(app.sb_drag.is_some_and(|d| d.axis == Axis::X));
    app.scrollbar_drag_move((90.0, 96.0));
    assert_eq!(app.scroll_offsets.get(&xk(id)).copied(), Some(400.0), "ratio 800/160 = 5");
}

#[test]
fn qsb2b_region_with_both_bars_has_a_corner_and_shortened_tracks() {
    let html = "<html><body style='margin:0'><div style='overflow:auto;width:200px;height:100px'>\
        <div style='width:1000px;height:500px'></div></div></body></html>";
    let Some(mut app) = app_with(html) else { return };
    // The corner (196, 96) is on no track; the y track ends at y 92, the x one at x 192.
    assert!(!app.scrollbar_press((196.0, 96.0)));
    assert!(app.scrollbar_press((196.0, 88.0)), "the y bar");
    app.sb_hold = None;
    assert!(app.scrollbar_press((188.0, 96.0)), "the x bar");
    // Painted: bar tint on the bottom strip and in the corner.
    let info = crate::paint::hit_test_scroll_chain(&app.cached.clone().unwrap(), 10.0, 10.0, &HashMap::new(), 0.0, 600.0)[0];
    let mut pixels = vec![0x00FF_FFFFu32; 300 * 200];
    crate::paint::paint_scrollbars(&info, (0.0, 0.0), (false, false), &mut pixels, 300, 200, (0.0, 0.0, 200.0, 100.0), None);
    let at = |x: usize, y: usize| pixels[y * 300 + x];
    assert_ne!(at(100, 96), 0x00FF_FFFF, "x track");
    assert_ne!(at(196, 50), 0x00FF_FFFF, "y track");
    assert_ne!(at(196, 96), 0x00FF_FFFF, "corner");
    assert_eq!(at(100, 50), 0x00FF_FFFF, "the body is untouched");
}

#[test]
fn qsb2b_hot_ids_of_the_two_axes_differ() {
    assert_ne!(sb::VIEWPORT_ID, sb::VIEWPORT_X_ID);
    assert_ne!(xk(7), 7);
}
