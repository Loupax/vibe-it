//! QOBS5: the frame-stats aggregator (per-second roll-up, p95, off = no file).
use crate::frame_stats::{External, FrameStats, InputKind};
use std::time::{Duration, Instant};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn qobs5_off_writes_no_file() {
    let path = std::env::temp_dir().join(format!("qobs5-off-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    for spec in [None, Some(""), Some("0")] {
        assert!(FrameStats::open(spec, Instant::now()).is_none());
    }
    assert!(!path.exists());
    let on = FrameStats::open(path.to_str(), Instant::now());
    assert!(on.is_some() && path.exists());
    drop(on);
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn qobs5_rollup_counts_and_resets() {
    let t0 = Instant::now();
    let mut fs = FrameStats::for_test(t0);
    assert!(fs.roll(t0 + ms(999), External::default()).is_none());
    fs.redraw_start(t0 + ms(10));
    fs.redraw_start(t0 + ms(50));
    fs.redraw_start(t0 + ms(400));
    fs.build(ms(7));
    fs.shape_result(false);
    fs.shape_result(true);
    fs.hover_restyle();
    fs.video_wake();
    fs.video_rendered(5);
    fs.video_rendered(5);
    fs.video_painted(5);
    fs.damage_present();
    let ext = External { js_ns: 12_000_000, forced_n: 2, forced_ns: 3_000_000, worker_ns: 4_000_000, net: (3, 1_000_000, 5_000_000, 7_000_000) };
    let line = fs.roll(t0 + ms(1000), ext).expect("a line after 1 s");
    assert!(line.contains("\"redraws\":3"), "{line}");
    assert!(line.contains("\"gap_max_ms\":350.0"), "{line}");
    assert!(line.contains("\"builds\":{\"n\":1,\"ms\":7.00}"), "{line}");
    assert!(line.contains("\"shape\":{\"complete\":1,\"incomplete\":1,\"worker_ms\":4.00}"), "{line}");
    assert!(line.contains("\"js_ms\":12.00"), "{line}");
    assert!(line.contains("\"forced\":{\"n\":2,\"ms\":3.00}"), "{line}");
    assert!(line.contains("\"hover_restyles\":1"), "{line}");
    assert!(line.contains("{\"node\":5,\"rendered\":2,\"painted\":1}"), "{line}");
    assert!(line.contains("\"damage_presents\":1"), "{line}");
    assert!(line.contains("\"net\":{\"n\":3,\"p50_ms\":1.00,\"p95_ms\":5.00,\"max_ms\":7.00}"), "{line}");
    let line2 = fs.roll(t0 + ms(2000), External::default()).unwrap();
    assert!(line2.contains("\"redraws\":0") && line2.contains("\"v\":[]"), "{line2}");
}

#[test]
fn qobs5_input_latency_max_and_p95() {
    let t0 = Instant::now();
    let mut fs = FrameStats::for_test(t0);
    // 20 moves; the i-th waited (i+1) ms for the present at 100 ms.
    for i in 0..20u64 {
        fs.input(InputKind::Move, t0 + ms(99 - i));
    }
    fs.input(InputKind::Key, t0 + ms(100));
    fs.presented(t0 + ms(100));
    // 21 samples: 0 ms (key) and 1..=20 ms; p95 = the 20th sorted = 19 ms.
    fs.input(InputKind::Wheel, t0 + ms(500));
    let line = fs.roll(t0 + ms(1000), External::default()).unwrap();
    assert!(line.contains("\"move\":20,\"button\":0,\"key\":1,\"wheel\":1"), "{line}");
    assert!(line.contains("\"lat_max_ms\":20.00"), "{line}");
    assert!(line.contains("\"lat_p95_ms\":19.00"), "{line}");
    assert!(line.contains("\"pending\":1,\"oldest_pending_ms\":500.0"), "{line}");
}
