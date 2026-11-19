// QM1k1n4: a second init segment (YouTube quality switch) must not strand playback. A
// generation at a time before the switch uses the init of the segments that cover that time and
// stops at the init change; `switch_point` tells the element where to load the next generation.
use super::*;

/// Init A + segments 0-20 s, then init B + segments from 20 s (a quality switch at 20 s).
fn two_init_buffer() -> (SourceBufferStore, Vec<u8>, Vec<u8>) {
    let mut b = buffer_with_init();
    let init_a = b.init_bytes().unwrap().to_vec();
    for t in [0, 4, 8, 12, 16] {
        b.append(&seg4(t, 40)).unwrap();
    }
    let init_b = init_seg(1, 48_000, 1024);
    assert!(b.append(&init_b).unwrap().new_generation);
    for t in [20u64, 24] {
        b.append(&frag(1, t * 48_000, Dur::Explicit(10, 19_200), 40, false)).unwrap();
    }
    (b, init_a, init_b)
}

#[test]
fn qm1k1n4_a_generation_before_the_switch_uses_the_old_init_and_its_segments() {
    let (mut b, init_a, init_b) = two_init_buffer();
    assert_eq!(ranges(&b), vec![(0, 28000)], "the buffered range spans both inits");
    // the reload after the second init asks for the current time 18 s: it must find 16-20 s
    assert!(b.start_generation(2, 18.0));
    assert_eq!(b.run_end(), Some(20.0), "the run covers 18 s (old init) and stops at the init change");
    let bytes = b.generation_bytes().unwrap();
    assert_eq!(&bytes[..init_a.len()], &init_a[..]);
    assert_eq!(b.switch_point(), Some(20.0), "the next segment belongs to the new init");
    // a generation at/after the switch uses the new init
    assert!(b.start_generation(3, 20.0));
    assert_eq!(&b.generation_bytes().unwrap()[..init_b.len()], &init_b[..]);
    assert_eq!(b.run_end(), Some(28.0));
    assert_eq!(b.switch_point(), None);
}

#[test]
fn qm1k1n4_the_second_init_before_its_media_leaves_the_old_media_playable() {
    let mut b = buffer_with_init();
    for t in [0, 4, 8] {
        b.append(&seg4(t, 40)).unwrap();
    }
    b.append(&init_seg(1, 48_000, 1024)).unwrap();
    assert!(b.start_generation(2, 5.0));
    assert_eq!(b.run_end(), Some(12.0), "playback continues on the old init meanwhile");
    assert_eq!(b.switch_point(), None, "no media of the new init yet");
}

#[cfg(feature = "media")]
mod real {
    use crate::media_el::{BackendEvent, MediaBackend};
    use crate::mse_store::MseFeed;
    use crate::tests::mse_mpv_tests::real::*;
    use std::time::Instant;

    /// Real mpv: quality A (0-10 s), the init of quality B is appended while playing at ~6 s and
    /// the element reloads at the current time (this used to open nothing and freeze), then B's
    /// media (10-20 s) arrives and the element switches at the init boundary. Time must pass 15 s.
    /// Skips without libmpv / ffmpeg.
    #[test]
    fn qm1k1n4_mpv_keeps_playing_across_a_second_init() {
        if !have_tools() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("qm1k1n4-{}", std::process::id()));
        let (Some((va, aa)), Some((vb, ab))) = (make_fixture(&dir.join("a"), 10, 0.0, 2, 10), make_fixture(&dir.join("b"), 10, 10.03, 2, 10)) else {
            eprintln!("SKIP: ffmpeg could not make the fixtures");
            return;
        };
        let feed = MseFeed::with_fmp4_tracks(2);
        Driver::inits(&feed, &va, &aa);
        for (bi, t) in [(0usize, &va), (1usize, &aa)] {
            for s in &t.segs {
                feed.append(bi, &s.2, 0.0).unwrap();
            }
        }
        let mut be = backend();
        let t0 = Instant::now();
        be.load_mse(feed.clone(), 0.0).unwrap();
        be.set_paused(false);
        let (mut phase, mut max_t, mut ok) = (0, 0.0f64, false);
        while t0.elapsed().as_secs_f64() < 60.0 && !ok {
            for e in be.poll(std::time::Duration::from_millis(20)) {
                let BackendEvent::Time(t) = e else { continue };
                let now = t;
                max_t = max_t.max(now);
                if phase == 0 && now >= 6.0 {
                    // the quality switch: init B on both buffers, then the element reloads at `now`
                    phase = 1;
                    feed.append(0, &vb.init, 0.0).unwrap();
                    feed.append(1, &ab.init, 0.0).unwrap();
                    be.load_mse(feed.clone(), now).unwrap();
                    be.set_paused(false);
                    for (bi, t) in [(0usize, &vb), (1usize, &ab)] {
                        for s in &t.segs {
                            feed.append(bi, &s.2, 0.0).unwrap();
                        }
                    }
                } else if phase == 1
                    && let Some(b) = feed.with_store(|s| s.switch_point())
                    && now >= b - 0.1
                {
                    phase = 2;
                    be.load_mse(feed.clone(), b).unwrap();
                    be.set_paused(false);
                }
                ok |= phase == 2 && now >= 15.0;
            }
            if be.frame_pending() {
                let _ = be.take_frame();
            }
        }
        assert!(ok, "playback froze: phase {phase}, max time {max_t}");
        drop(be);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
