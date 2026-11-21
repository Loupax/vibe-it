use super::*;
use crate::scrollbar::{self as sb, BarHit};

// QSB2: hold-repeat rule and thumb colour state.
#[test]
fn qsb2_hold_repeats_only_while_the_pointer_stays_on_the_pressed_side() {
    assert!(sb::repeat_applies(BarHit::TrackAfter, Some(BarHit::TrackAfter)));
    assert!(sb::repeat_applies(BarHit::TrackBefore, Some(BarHit::TrackBefore)));
    assert!(!sb::repeat_applies(BarHit::TrackAfter, Some(BarHit::Thumb)), "thumb caught up");
    assert!(!sb::repeat_applies(BarHit::TrackAfter, Some(BarHit::TrackBefore)));
    assert!(!sb::repeat_applies(BarHit::TrackAfter, None), "pointer left the bar");
    assert!(!sb::repeat_applies(BarHit::Thumb, Some(BarHit::Thumb)));
}

#[test]
fn qsb2_thumb_colour_follows_hover_and_press_of_its_own_bar_only() {
    use crate::config::*;
    assert_eq!(sb::thumb_rgba(None, 3), SB_THUMB_RGBA);
    assert_eq!(sb::thumb_rgba(Some((3, false)), 3), SB_THUMB_HOVER_RGBA);
    assert_eq!(sb::thumb_rgba(Some((3, true)), 3), SB_THUMB_ACTIVE_RGBA);
    assert_eq!(sb::thumb_rgba(Some((4, true)), 3), SB_THUMB_RGBA);
}

// QSB1: pure scrollbar maths and hit tests.
const RECT: (f32, f32, f32, f32) = (0.0, 0.0, 400.0, 500.0);

#[test]
fn qsb1_thumb_length_follows_client_over_content_with_a_floor() {
    // content 1000 through 500: half the 500 px track.
    assert_eq!(sb::thumb(500.0, 1000.0, 500.0, 0.0), (0.0, 250.0));
    // at the end the thumb sits at the track's bottom.
    assert_eq!(sb::thumb(500.0, 1000.0, 500.0, 500.0), (250.0, 250.0));
    // huge content: the 16 px minimum.
    assert_eq!(sb::thumb(500.0, 1.0e6, 500.0, 0.0).1, 16.0);
    // nothing to scroll: fills the track.
    assert_eq!(sb::thumb(500.0, 300.0, 500.0, 0.0), (0.0, 500.0));
}

#[test]
fn qsb1_hit_test_thumb_track_and_outside() {
    // thumb at 0..250 (scroll 0): x must be in the right 8 px.
    assert_eq!(sb::hit(RECT, 1000.0, 500.0, 0.0, 395.0, 100.0), Some(BarHit::Thumb));
    assert_eq!(sb::hit(RECT, 1000.0, 500.0, 0.0, 395.0, 300.0), Some(BarHit::TrackAfter));
    assert_eq!(sb::hit(RECT, 1000.0, 500.0, 500.0, 395.0, 100.0), Some(BarHit::TrackBefore));
    assert_eq!(sb::hit(RECT, 1000.0, 500.0, 0.0, 391.0, 100.0), None, "left of the track");
    assert_eq!(sb::hit(RECT, 1000.0, 500.0, 0.0, 395.0, 501.0), None, "below the box");
    assert_eq!(sb::hit(RECT, 400.0, 500.0, 0.0, 395.0, 100.0), None, "nothing to scroll");
    assert_eq!(sb::track_rect(0.0, 0.0, 5.0, 100.0), None, "box narrower than the track");
}

#[test]
fn qsb1_drag_maths_uses_the_content_over_track_ratio_and_keeps_the_grab_offset() {
    // content 1000, client 500, track 500 -> thumb 250, free 250, max 500: 2 content px per pointer px.
    let ratio = sb::drag_ratio(500.0, 1000.0, 500.0);
    assert_eq!(ratio, 2.0);
    // grabbed at scroll 100 with the pointer at y=60 (anywhere on the thumb); a move to y=110 = +50 px -> +100.
    assert_eq!(sb::drag_scroll(100.0, 60.0, 110.0, ratio, 500.0), 200.0);
    // no jump on the press itself: zero delta keeps the position.
    assert_eq!(sb::drag_scroll(100.0, 60.0, 60.0, ratio, 500.0), 100.0);
    // dragging back up.
    assert_eq!(sb::drag_scroll(100.0, 60.0, 30.0, ratio, 500.0), 40.0);
}

#[test]
fn qsb1_drag_clamps_at_both_ends() {
    let ratio = sb::drag_ratio(500.0, 1000.0, 500.0);
    assert_eq!(sb::drag_scroll(100.0, 60.0, 10_000.0, ratio, 500.0), 500.0);
    assert_eq!(sb::drag_scroll(100.0, 60.0, -10_000.0, ratio, 500.0), 0.0);
    assert_eq!(sb::drag_ratio(500.0, 400.0, 500.0), 0.0, "thumb fills the track");
}

#[test]
fn qsb1_track_press_pages_and_clamps() {
    assert_eq!(sb::page_target(BarHit::TrackAfter, 0.0, 460.0, 500.0), 460.0);
    assert_eq!(sb::page_target(BarHit::TrackAfter, 460.0, 460.0, 500.0), 500.0);
    assert_eq!(sb::page_target(BarHit::TrackBefore, 100.0, 460.0, 500.0), 0.0);
    assert_eq!(sb::page_target(BarHit::Thumb, 100.0, 460.0, 500.0), 100.0);
}

#[test]
fn qsb1_region_bar_hit_uses_the_regions_own_box() {
    let html = "<html><body style='margin:0'><div style='overflow:auto;height:200px;width:200px'><div style='height:600px'></div></div></body></html>";
    let laid = measure_document_fixture(html, 800.0, 600.0);
    // the bar is the region's right 8 px: x 192..200.
    let chain = crate::paint::hit_test_scroll_bars(&laid, 196.0, 10.0, &HashMap::new(), 0.0, 600.0);
    assert_eq!(chain.len(), 1);
    let r = chain[0];
    assert_eq!(r.rect.2, 200.0);
    let h = sb::hit(r.rect, r.info.content_h, r.info.client_h, 0.0, 196.0, r.py);
    assert_eq!(h, Some(BarHit::Thumb));
    let below = crate::paint::hit_test_scroll_bars(&laid, 196.0, 190.0, &HashMap::new(), 0.0, 600.0)[0];
    assert_eq!(sb::hit(below.rect, below.info.content_h, below.info.client_h, 0.0, 196.0, below.py), Some(BarHit::TrackAfter));
    // outside the bar column: no hit, the page keeps the click.
    assert_eq!(sb::hit(r.rect, r.info.content_h, r.info.client_h, 0.0, 100.0, r.py), None);
}
