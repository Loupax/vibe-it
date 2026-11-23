//! C6h: the refresh interval the presentation feedback reports (`render`: `Refresh::fixed(pacer.interval())`).

use crate::pacing::Pacer;

#[test]
fn presentation_refresh_is_one_outer_frame() {
    // Chromium derives its BeginFrame interval from this value; 8333333 ns is what a 120 Hz
    // output's `presented` event carries (seen in WAYLAND_DEBUG).
    assert_eq!(Pacer::new(120_000, false).interval().as_nanos(), 8_333_333);
    assert_eq!(Pacer::new(60_000, false).interval().as_nanos(), 16_666_666);
}
