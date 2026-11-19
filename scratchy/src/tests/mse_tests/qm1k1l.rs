// QM1k1l: `buffered` is empty after an MSE detach (the backend's cache time of the unloaded
// file must not leak through `MediaCore::buffered`).
use super::qm1k1d::attached;

#[test]
fn qm1k1l_buffered_is_empty_after_the_element_is_emptied() {
    let js = attached();
    js.run("ms.addSourceBuffer('video/mp4; codecs=\"avc1.64001f\"');");
    // the backend reports a cache range (as mpv's demuxer-cache-time would)
    js.fakes.borrow()[0].borrow_mut().buffered = vec![(0.0, 61.8)];
    js.run("v.removeAttribute('src'); v.load();");
    js.step(100.0);
    assert_eq!(js.val("v.buffered.length"), "0");
    assert_eq!(js.val("[v.readyState, v.currentTime].join()"), "0,0");
}

#[cfg(feature = "media")]
mod real {
    use super::super::super::mse_mpv_tests::real::{backend, have_tools, make_fixture, pump, Driver};
    use crate::media_el::{BackendEvent, MediaBackend};
    use crate::mse_store::*;
    use std::time::{Duration, Instant};

    #[test]
    fn qm1k1l_mpv_buffered_is_empty_after_unload_even_with_late_cache_events() {
        if !have_tools() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("qm1k1l-{}", std::process::id()));
        let Some((v, a)) = make_fixture(&dir, 10, 0.0, 4, 10) else {
            eprintln!("SKIP: ffmpeg could not make the fixture");
            return;
        };
        let feed = MseFeed::with_fmp4_tracks(2);
        Driver::inits(&feed, &v, &a);
        let mut drv = Driver::new(&v, &a, 6.0, 0.0);
        let mut be = backend();
        be.set_video_target(Some(crate::media_video::VideoTarget { w: 320, h: 240, fit: crate::css::ObjectFit::Contain }));
        let t0 = Instant::now();
        drv.tick(&feed, 0.0);
        be.load_mse(feed.clone(), 0.0).unwrap();
        be.set_paused(false);
        let played = pump(&mut be, &feed, &mut drv, t0, 20.0, |be, e| matches!(e, BackendEvent::Time(t) if *t > 0.5 && !be.buffered().is_empty()));
        assert!(played, "mpv never played with a non-empty buffered");
        be.unload();
        assert!(be.buffered().is_empty(), "right after unload: {:?}", be.buffered());
        // late property events of the stopped file must not bring the range back
        let t1 = Instant::now();
        while t1.elapsed() < Duration::from_millis(1500) {
            be.poll(Duration::from_millis(20));
            assert!(be.buffered().is_empty(), "late cache event: {:?}", be.buffered());
        }
        drop(be);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
