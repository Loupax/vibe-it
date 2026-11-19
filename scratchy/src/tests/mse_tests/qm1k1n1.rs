// QM1k1n1: a PLAYING element whose box is laid out but clipped or off-screen keeps the video
// track decoding (no `vid=no`); the picture is there as soon as the box comes into view.
#[cfg(feature = "media")]
mod real {
    use super::super::super::mse_mpv_tests::real::{backend, have_tools, make_fixture, pump, Driver};
    use crate::media_el::{BackendEvent, MediaBackend};
    use crate::mse_store::*;
    use std::time::{Duration, Instant};

    const T: crate::media_video::VideoTarget = crate::media_video::VideoTarget { w: 320, h: 240, fit: crate::css::ObjectFit::Contain };

    /// utime + stime of this process in seconds (all threads, so mpv's decoder threads count).
    fn proc_cpu() -> f64 {
        let s = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
        let rest = s.rsplit(')').next().unwrap_or("");
        let f: Vec<&str> = rest.split_whitespace().collect();
        let g = |i: usize| f.get(i).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0);
        (g(11) + g(12)) / 100.0
    }

    /// Plays hidden (no target) for 3 s with or without `keep`, then brings the box in and
    /// reports (video track selected while hidden, seconds from the box appearing to the
    /// first rendered frame).
    fn scenario(keep: bool) -> Option<(bool, f64)> {
        let dir = std::env::temp_dir().join(format!("qm1k1n1-{}-{keep}", std::process::id()));
        let Some((v, a)) = make_fixture(&dir, 12, 0.0, 4, 12) else {
            eprintln!("SKIP: ffmpeg could not make the fixture");
            return None;
        };
        let feed = MseFeed::with_fmp4_tracks(2);
        Driver::inits(&feed, &v, &a);
        let mut drv = Driver::new(&v, &a, 8.0, 0.0);
        let mut be = backend();
        let t0 = Instant::now();
        drv.tick(&feed, 0.0);
        // the page parks the video off-screen: laid out, no visible target
        be.set_video_target(None);
        be.set_video_keep(keep);
        be.load_mse(feed.clone(), 0.0).unwrap();
        be.set_paused(false);
        let mut loaded = false;
        assert!(
            pump(&mut be, &feed, &mut drv, t0, 20.0, |_, e| {
                loaded |= matches!(e, BackendEvent::Loaded { .. });
                loaded && matches!(e, BackendEvent::Time(t) if *t > 0.5)
            }),
            "never played"
        );
        // re-apply (as the next redraw would) now that the file is open
        be.set_video_target(None);
        let hold = Instant::now();
        let cpu0 = proc_cpu();
        pump(&mut be, &feed, &mut drv, t0, 60.0, |_, _| hold.elapsed() > Duration::from_secs(3));
        let cpu = proc_cpu() - cpu0;
        let video_on = be.prop_f64("current-tracks/video/id").is_some();
        eprintln!("qm1k1n1: keep={keep} hidden for 3 s: video track selected {video_on}, process CPU {cpu:.2} s (mpv threads + test pump)");
        // the box comes into view
        let t1 = Instant::now();
        be.set_video_target(Some(T));
        let mut got = None;
        while t1.elapsed() < Duration::from_secs(10) {
            drv.tick(&feed, t0.elapsed().as_secs_f64());
            be.poll(Duration::from_millis(5));
            if be.frame_pending()
                && let Some(f) = be.take_frame()
            {
                got = Some((f.w, f.h, t1.elapsed().as_secs_f64()));
                break;
            }
        }
        drop(be);
        let _ = std::fs::remove_dir_all(&dir);
        let (w, h, dt) = got.expect("no frame after the box came into view");
        assert_eq!((w, h), (320, 240));
        eprintln!("qm1k1n1: keep={keep}: box in view -> first rendered frame after {dt:.3} s");
        Some((video_on, dt))
    }

    #[test]
    fn qm1k1n1_mpv_keep_leaves_the_video_track_on_while_hidden_and_the_frame_is_immediate() {
        if !have_tools() {
            return;
        }
        let Some((on, dt)) = scenario(true) else { return };
        assert!(on, "keep: the video track must stay selected while the box is hidden");
        assert!(dt < 1.0, "first frame {dt} s after the box appeared");
        if let Some((off_on, off_dt)) = scenario(false) {
            assert!(!off_on, "without keep a hidden element drops the video track (vid=no)");
            eprintln!("qm1k1n1: keep {dt:.3} s vs vid=no {off_dt:.3} s");
        }
    }
}
