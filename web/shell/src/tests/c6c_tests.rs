//! C6c: frame pacing (`pacing.rs`).

use std::time::{Duration, Instant};

use crate::pacing::*;

#[test]
fn interval_follows_the_refresh_rate() {
    assert_eq!(Pacer::new(60_000, true).interval().as_micros(), 16_666);
    assert_eq!(Pacer::new(144_000, true).interval().as_micros(), 6_944);
    assert_eq!(Pacer::new(59_940, true).interval().as_micros(), 16_683);
}

#[test]
fn unknown_or_absurd_refresh_falls_back_to_60_hz() {
    assert_eq!(sane_refresh(None), DEFAULT_REFRESH_MHZ);
    assert_eq!(sane_refresh(Some(0)), DEFAULT_REFRESH_MHZ);
    assert_eq!(sane_refresh(Some(5)), DEFAULT_REFRESH_MHZ);
    assert_eq!(sane_refresh(Some(165_000)), 165_000);
}

#[test]
fn the_first_render_never_waits() {
    assert_eq!(Pacer::new(60_000, true).wait(Instant::now()), None);
}

#[test]
fn a_render_right_after_another_waits_for_the_spacing() {
    let t0 = Instant::now();
    let mut p = Pacer::new(60_000, true);
    p.rendered(t0);
    let w = p.wait(t0 + Duration::from_millis(2)).expect("too soon");
    // one 16.67 ms interval, minus the 2 ms already passed
    assert!(w > Duration::from_millis(14) && w < Duration::from_millis(15), "{w:?}");
    assert_eq!(p.wait(t0 + Duration::from_millis(17)), None);
    assert_eq!(p.wait(t0 + Duration::from_secs(5)), None, "an idle page renders at once");
}

#[test]
fn a_faster_output_shortens_the_wait() {
    let t0 = Instant::now();
    let mut p = Pacer::new(60_000, true);
    p.rendered(t0);
    p.set_refresh(144_000);
    assert_eq!(p.wait(t0 + Duration::from_millis(7)), None);
}

#[test]
fn on_wayland_the_desktops_frame_callbacks_pace_and_the_pacer_never_waits() {
    let t0 = Instant::now();
    let mut p = Pacer::new(60_000, false);
    p.rendered(t0);
    assert_eq!(p.wait(t0), None);
    assert_eq!(p.wait(t0 + Duration::from_millis(1)), None);
}
