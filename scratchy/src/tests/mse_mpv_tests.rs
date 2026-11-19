// QM1k1b: the mpv MSE feed (`scratchy-mse://`), `MediaBackend::load_mse`, the ffmpeg
// sync-marker fixtures and the Rust fake MSE driver (no JS, no MediaSource, no element core:
// that is QM1k1c). Real-mpv tests SKIP without libmpv.so.2 or ffmpeg.
use crate::media_el::{FakeBackend, LoadError, MediaBackend, UnavailableBackend};
use crate::mse_store::*;

#[test]
fn qm1k1b_load_mse_default_is_refused_and_fake_records_it() {
    let feed = MseFeed::with_fmp4_tracks(1);
    let mut u = UnavailableBackend;
    assert_eq!(u.load_mse(feed.clone(), 0.0), Err(LoadError::Other("MSE not supported".into())));
    let h = crate::media_el::fake_new();
    let mut f = FakeBackend(h.clone());
    f.load_mse(feed.clone(), 12.5).unwrap();
    assert!(h.borrow().calls.iter().any(|c| c == &format!("load_mse {} 12.5", feed.id)), "{:?}", h.borrow().calls);
    assert!(h.borrow().loaded.as_ref().unwrap().url.starts_with("scratchy-mse://"));
    assert_eq!(h.borrow().position, 12.5);
}

#[test]
fn qm1k1b_feed_stop_wakes_a_blocked_reader() {
    use std::sync::atomic::AtomicBool;
    let feed = MseFeed::with_fmp4_tracks(1);
    feed.append(0, &super::mse_tests::init_seg(1, 90000, 3000), 0.0).unwrap();
    let gid = feed.start_generation(0.0);
    let f2 = feed.clone();
    let t = std::thread::spawn(move || {
        let mut buf = vec![0u8; 16];
        f2.read(0, gid, 1 << 20, &mut buf, &AtomicBool::new(false))
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    let t0 = std::time::Instant::now();
    feed.stop();
    assert_eq!(t.join().unwrap(), FeedRead::Gone);
    assert!(t0.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn qm1k1b_feed_blocks_then_eof_after_end_of_stream_and_cancel_returns() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let feed = MseFeed::with_fmp4_tracks(1);
    let init = super::mse_tests::init_seg(1, 90000, 3000);
    feed.append(0, &init, 0.0).unwrap();
    let gid = feed.start_generation(0.0);
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let mut buf = vec![0u8; 16];
    let f2 = feed.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        f2.end_of_stream();
    });
    let t0 = std::time::Instant::now();
    let at = init.len() as u64; // past the init, no media yet
    assert_eq!(feed.read(0, gid, at, &mut buf, &cancel), FeedRead::Eof);
    assert!(t0.elapsed() >= std::time::Duration::from_millis(80));
    t.join().unwrap();
    cancel.store(true, Ordering::SeqCst);
    assert_eq!(feed.read(0, gid, at, &mut buf, &cancel), FeedRead::Gone);
}

#[cfg(feature = "media")]
pub(crate) mod real {
    use super::*;
    use crate::media_el::{BackendCtx, BackendEvent};
    use crate::media_mpv::MpvBackend;
    use std::path::Path;
    use std::time::{Duration, Instant};

    /// One fMP4 track cut into its init segment and media segments `(start, end, bytes)`.
    pub struct Track {
        pub init: Vec<u8>,
        pub segs: Vec<(f64, f64, Vec<u8>)>,
    }

    fn ffmpeg(args: &[&str]) -> bool {
        std::process::Command::new("ffmpeg").args(["-v", "error", "-y"]).args(args).status().map(|s| s.success()).unwrap_or(false)
    }

    /// ffmpeg's mov muxer always writes tfdt starting at 0 (even with -itsoffset/-copyts), so a
    /// fixture that starts at `offset` s gets its tfdt values shifted here (version 1 tfdt only).
    fn shift_tfdt(b: &mut [u8], offset: f64) {
        fn find(b: &[u8], from: usize, to: usize, ty: &[u8; 4]) -> Option<(usize, usize)> {
            let mut i = from;
            while i + 8 <= to {
                let n = u32::from_be_bytes(b[i..i + 4].try_into().ok()?) as usize;
                if n < 8 || i + n > to {
                    return None;
                }
                if &b[i + 4..i + 8] == ty {
                    return Some((i, n));
                }
                i += n;
            }
            None
        }
        let len = b.len();
        let Some((moov, mn)) = find(b, 0, len, b"moov") else { return };
        let Some((trak, tn)) = find(b, moov + 8, moov + mn, b"trak") else { return };
        let Some((mdia, dn)) = find(b, trak + 8, trak + tn, b"mdia") else { return };
        let Some((mdhd, _)) = find(b, mdia + 8, mdia + dn, b"mdhd") else { return };
        let ts = u32::from_be_bytes(b[mdhd + 20..mdhd + 24].try_into().unwrap()) as f64;
        let mut i = 0;
        while i + 8 <= len {
            let n = u32::from_be_bytes(b[i..i + 4].try_into().unwrap()) as usize;
            if n < 8 || i + n > len {
                return;
            }
            if &b[i + 4..i + 8] == b"moof"
                && let Some((traf, tn)) = find(b, i + 8, i + n, b"traf")
                && let Some((tfdt, _)) = find(b, traf + 8, traf + tn, b"tfdt")
                && b[tfdt + 8] == 1
            {
                let at = tfdt + 12;
                let v = u64::from_be_bytes(b[at..at + 8].try_into().unwrap()) + (offset * ts) as u64;
                b[at..at + 8].copy_from_slice(&v.to_be_bytes());
            }
            i += n;
        }
    }

    fn track_of(file: &Path, offset: f64) -> Option<Track> {
        let mut bytes = std::fs::read(file).ok()?;
        if offset != 0.0 {
            shift_tfdt(&mut bytes, offset);
        }
        let mut r = Fmp4Reader::new();
        let mut t = Track { init: Vec::new(), segs: Vec::new() };
        for p in r.feed(&bytes) {
            match p {
                Parsed::Init(b) => t.init = b,
                Parsed::Media { bytes, start, end } => t.segs.push((start, end, bytes)),
                Parsed::Error(_) => return None,
            }
        }
        (!t.init.is_empty() && !t.segs.is_empty()).then_some(t)
    }

    /// The sync-marker clip: video = a white frame (40 ms) at every whole second on black;
    /// audio = a 10 ms 1 kHz beep at every whole second. x264 + AAC, split into VIDEO-ONLY
    /// and AUDIO-ONLY fragmented mp4 (video fragments `vfrag` s, audio `afrag` s), with every
    /// timestamp shifted by `offset` s. None when ffmpeg is missing or fails.
    pub fn make_fixture(dir: &Path, secs: u32, offset: f64, vfrag: u32, afrag: u32) -> Option<(Track, Track)> {
        std::fs::create_dir_all(dir).ok()?;
        let src = dir.join("src.mp4");
        let (s, g) = (secs.to_string(), (vfrag * 25).to_string());
        let vf = "color=c=black:s=320x240:r=25,noise=alls=40:allf=t,drawbox=x=0:y=0:w=320:h=240:color=white:t=fill:enable='lt(mod(t,1),0.04)'";
        let af = "aevalsrc='0.8*sin(2*PI*1000*t)*lt(mod(t,1),0.01)':s=44100:c=mono";
        if !ffmpeg(&["-f", "lavfi", "-i", vf, "-f", "lavfi", "-i", af, "-t", &s, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", &g, "-keyint_min", &g, "-sc_threshold", "0", "-c:a", "aac", "-b:a", "64k", src.to_str()?]) {
            return None;
        }
        let (vp, ap) = (dir.join("v.mp4"), dir.join("a.mp4"));
        let frag = format!("{}", afrag as u64 * 1_000_000);
        if !ffmpeg(&["-i", src.to_str()?, "-map", "0:v", "-c", "copy","-movflags", "frag_keyframe+empty_moov+default_base_moof", vp.to_str()?]) {
            return None;
        }
        if !ffmpeg(&["-i", src.to_str()?, "-map", "0:a", "-c", "copy","-frag_duration", &frag, "-movflags", "empty_moov+default_base_moof", ap.to_str()?]) {
            return None;
        }
        Some((track_of(&vp, offset)?, track_of(&ap, offset)?))
    }

    pub fn have_tools() -> bool {
        if !crate::media_mpv::libmpv_available() {
            eprintln!("SKIP: libmpv.so.2 not available");
            return false;
        }
        if std::process::Command::new("ffmpeg").arg("-version").output().is_err() {
            eprintln!("SKIP: ffmpeg is not available");
            return false;
        }
        true
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("qm1k1b-{}-{name}", std::process::id()))
    }

    /// The fake MSE driver: appends init + fragments on a schedule (video first, audio
    /// `audio_delay` s late, each fragment `ahead` s before it is due) and ends the stream.
    pub struct Driver {
        items: Vec<(f64, usize, Vec<u8>)>, // (due s, buffer, bytes)
        next: usize,
        pub done: bool,
        /// Do not call `endOfStream` (a stalled-stream test).
        pub never_end: bool,
    }

    impl Driver {
        pub fn new(v: &Track, a: &Track, ahead: f64, audio_delay: f64) -> Driver {
            let mut items = Vec::new();
            for (bi, t, delay) in [(0usize, v, 0.0), (1usize, a, audio_delay)] {
                let first = t.segs[0].0;
                for (start, _, bytes) in &t.segs {
                    items.push(((start - first - ahead).max(0.0) + delay, bi, bytes.clone()));
                }
            }
            items.sort_by(|x, y| x.0.total_cmp(&y.0));
            Driver { items, next: 0, done: false, never_end: false }
        }

        /// No schedule and no `endOfStream`: the test appends by hand.
        pub fn manual() -> Driver {
            Driver { items: Vec::new(), next: 0, done: false, never_end: true }
        }

        pub fn inits(feed: &MseFeed, v: &Track, a: &Track) {
            feed.append(0, &v.init, 0.0).unwrap();
            feed.append(1, &a.init, 0.0).unwrap();
        }

        /// Appends everything due at driver time `t` (s).
        pub fn tick(&mut self, feed: &MseFeed, t: f64) {
            while self.next < self.items.len() && self.items[self.next].0 <= t {
                let (_, bi, bytes) = &self.items[self.next];
                feed.append(*bi, bytes, 0.0).unwrap();
                self.next += 1;
            }
            if self.next == self.items.len() && !self.done && !self.never_end {
                self.done = true;
                feed.end_of_stream();
            }
        }
    }

    pub fn backend() -> MpvBackend {
        // No socket: nothing can come through wire, so a played stream proves every byte was memory.
        MpvBackend::new(BackendCtx { socket: None, policy: Default::default() })
    }

    /// Polls `be` (ticking the driver) until `stop` returns true or `max_s` pass.
    pub fn pump(be: &mut MpvBackend, feed: &MseFeed, drv: &mut Driver, t0: Instant, max_s: f64, mut stop: impl FnMut(&MpvBackend, &BackendEvent) -> bool) -> bool {
        while t0.elapsed().as_secs_f64() < max_s {
            drv.tick(feed, t0.elapsed().as_secs_f64());
            for e in be.poll(Duration::from_millis(20)) {
                if stop(be, &e) {
                    return true;
                }
            }
            if be.frame_pending() {
                let _ = be.take_frame();
            }
        }
        false
    }

    #[test]
    fn qm1k1b_mpv_plays_a_growing_two_track_stream_to_the_end_at_media_time() {
        if !have_tools() {
            return;
        }
        let dir = tmp("play");
        // 10 s clip whose first fragment starts at 30 s (proves `rebase-start-time=no`)
        let Some((v, a)) = make_fixture(&dir, 10, 30.0, 4, 10) else {
            eprintln!("SKIP: ffmpeg could not make the fixture");
            return;
        };
        assert!((v.segs[0].0 - 30.0).abs() < 0.1, "fixture start {}", v.segs[0].0);
        let feed = MseFeed::with_fmp4_tracks(2);
        Driver::inits(&feed, &v, &a);
        let mut drv = Driver::new(&v, &a, 6.0, 0.0);
        let mut be = backend();
        be.set_video_target(Some(crate::media_video::VideoTarget { w: 320, h: 240, fit: crate::css::ObjectFit::Contain }));
        let t0 = Instant::now();
        drv.tick(&feed, 0.0);
        be.load_mse(feed.clone(), 30.0).unwrap();
        be.set_paused(false);
        let (mut first, mut last, mut worst, mut loaded, mut errors, mut samples) = (None, 0.0f64, 0.0f64, false, Vec::new(), 0u32);
        let ended = pump(&mut be, &feed, &mut drv, t0, 40.0, |be, e| {
            match e {
                BackendEvent::Loaded { .. } => loaded = true,
                BackendEvent::Error(c, m) => errors.push((*c, m.clone())),
                BackendEvent::Time(t) => {
                    first.get_or_insert(*t);
                    last = *t;
                    if *t > 31.5
                        && let Some(s) = be.prop_f64("avsync")
                    {
                        worst = worst.max(s.abs());
                        samples += 1;
                    }
                }
                _ => {}
            }
            matches!(e, BackendEvent::Eof)
        });
        assert!(errors.is_empty(), "{errors:?}");
        assert!(loaded && ended, "loaded {loaded} ended {ended} last time {last}");
        assert!(first.unwrap() >= 29.9, "time-pos starts at the first tfdt (no rebase): {first:?}");
        assert!(last > 38.5, "played to the end: {last}");
        assert!(samples > 10, "avsync was readable only {samples} times");
        assert!(worst < 0.1, "|avsync| {worst}");
        drop(be);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn qm1k1b_mpv_in_generation_seek_and_new_generation_reload_both_restart() {
        if !have_tools() {
            return;
        }
        let dir = tmp("seek");
        let Some((v, a)) = make_fixture(&dir, 10, 0.0, 4, 10) else {
            eprintln!("SKIP: ffmpeg could not make the fixture");
            return;
        };
        assert_eq!(v.segs.len(), 3, "10 s in 4 s fragments");
        let feed = MseFeed::with_fmp4_tracks(2);
        Driver::inits(&feed, &v, &a);
        // video fragments 0 and 2 only: the live run from 0 stops at 4 s, 9 s is outside it
        feed.append(0, &v.segs[0].2, 0.0).unwrap();
        feed.append(0, &v.segs[2].2, 0.0).unwrap();
        feed.append(1, &a.segs[0].2, 0.0).unwrap();
        let mut drv = Driver::manual();
        let mut be = backend();
        let t0 = Instant::now();
        be.load_mse(feed.clone(), 0.0).unwrap();
        be.set_paused(false);
        assert!(pump(&mut be, &feed, &mut drv, t0, 20.0, |_, e| matches!(e, BackendEvent::Time(t) if *t > 0.5)), "plays from 0");
        // in-generation seek (inside the run)
        be.seek(2.0);
        let mut restart = false;
        assert!(
            pump(&mut be, &feed, &mut drv, t0, 30.0, |_, e| {
                restart |= matches!(e, BackendEvent::Restart);
                restart && matches!(e, BackendEvent::Time(t) if *t >= 1.9)
            }),
            "in-generation seek reached (restart {restart})"
        );
        // out-of-generation: the run does not cover 9 s -> a new generation at 9 s
        assert!(feed.with_store(|s| s.buffers[0].run_end()).is_some_and(|e| e < 9.0), "9 s is outside the live run");
        let gen_before = feed.with_store(|s| s.buffers[0].generation().map(|g| g.id));
        let t1 = Instant::now();
        be.load_mse(feed.clone(), 9.0).unwrap();
        be.set_paused(false);
        let (mut got, mut loaded) = (None, false);
        assert!(
            pump(&mut be, &feed, &mut drv, t0, 60.0, |_, e| {
                // events of the old generation may still be queued: only count the new file's
                loaded |= matches!(e, BackendEvent::Loaded { .. });
                if let (true, BackendEvent::Time(t)) = (loaded, e) {
                    got = Some(*t);
                }
                loaded && matches!(e, BackendEvent::Time(t) if *t >= 9.0)
            }),
            "new generation reached 9 s: {got:?}"
        );
        let gen_after = feed.with_store(|s| s.buffers[0].generation().map(|g| g.id));
        assert!(gen_after > gen_before, "{gen_before:?} -> {gen_after:?}");
        eprintln!("qm1k1b: reload latency to first time >= 9 s: {:?}", t1.elapsed());
        drop(be);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn qm1k1b_mpv_unload_and_drop_return_at_once_while_a_read_is_blocked() {
        if !have_tools() {
            return;
        }
        let dir = tmp("cancel");
        let Some((v, a)) = make_fixture(&dir, 10, 0.0, 4, 10) else {
            eprintln!("SKIP: ffmpeg could not make the fixture");
            return;
        };
        for case in 0..2 {
            let feed = MseFeed::with_fmp4_tracks(2);
            Driver::inits(&feed, &v, &a);
            if case == 1 {
                // case 1: one fragment each, no endOfStream: playback stalls on a blocked read
                feed.append(0, &v.segs[0].2, 0.0).unwrap();
                feed.append(1, &a.segs[0].2, 0.0).unwrap();
            }
            // case 0: only the inits: the open itself is blocked
            let mut drv = Driver::manual();
            let mut be = backend();
            let t0 = Instant::now();
            be.load_mse(feed.clone(), 0.0).unwrap();
            be.set_paused(false);
            pump(&mut be, &feed, &mut drv, t0, 6.0, |_, _| false); // let it block
            let t1 = Instant::now();
            be.unload();
            let unload = t1.elapsed();
            let t2 = Instant::now();
            drop(be);
            let dropped = t2.elapsed();
            assert!(unload < Duration::from_secs(1) && dropped < Duration::from_secs(2), "case {case}: unload {unload:?} drop {dropped:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One scenario of the drift measurement: plays for `play_s` (or to the end) in real time,
    /// sampling mpv every 250 ms, and applies `actions` `(at wall s, Action)`.
    #[derive(Clone, Copy)]
    enum Act {
        Seek(f64),
        Reload(f64),
    }

    fn drift_scenario(name: &str, v: &Track, a: &Track, ahead: f64, audio_delay: f64, play_s: f64, actions: &[(f64, Act)]) -> bool {
        let feed = MseFeed::with_fmp4_tracks(2);
        Driver::inits(&feed, v, a);
        let mut drv = Driver::new(v, a, ahead, audio_delay);
        let mut be = backend();
        let t0 = Instant::now();
        drv.tick(&feed, 0.0);
        let mut reload_started: Option<Instant> = None;
        // A visible video box: without a target the backend switches the video off (`vid=no`)
        // and `avsync` / the drop counters do not exist.
        be.set_video_target(Some(crate::media_video::VideoTarget { w: 320, h: 240, fit: crate::css::ObjectFit::Contain }));
        be.load_mse(feed.clone(), 0.0).unwrap();
        be.set_paused(false);
        let (mut next_sample, mut next_act, mut ended) = (0.0f64, 0usize, false);
        let mut loaded = false;
        let mut load_at = Instant::now();
        let props = ["avsync", "total-avsync-change", "frame-drop-count", "decoder-frame-drop-count", "audio-pts"];
        println!("scenario {name}");
        println!("{:>7} {:>9} {:>9} {:>10} {:>7} {:>7} {:>10} {:>10}", "wall_s", "time-pos", "avsync", "tot-av-chg", "drops", "decdrop", "audio-pts", "apts-tpos");
        while t0.elapsed().as_secs_f64() < play_s + 5.0 && !ended {
            let wall = t0.elapsed().as_secs_f64();
            drv.tick(&feed, wall);
            if next_act < actions.len() && wall >= actions[next_act].0 {
                match actions[next_act].1 {
                    Act::Seek(t) => {
                        println!("# {wall:7.2} seek (in generation) to {t}");
                        be.seek(t);
                    }
                    Act::Reload(t) => {
                        println!("# {wall:7.2} reload (new generation) at {t}");
                        reload_started = Some(Instant::now());
                        be.load_mse(feed.clone(), t).unwrap();
                        be.set_paused(false);
                        loaded = false;
                        load_at = Instant::now();
                    }
                }
                next_act += 1;
            }
            let mut tpos = f64::NAN;
            for e in be.poll(Duration::from_millis(20)) {
                match e {
                    BackendEvent::Loaded { .. } => loaded = true,
                    BackendEvent::Time(t) => {
                        tpos = t;
                        if let (Some(s), true) = (reload_started, loaded) {
                            println!("# reload latency (load_mse -> first time-pos of the new generation): {:?} (loaded after {:?})", s.elapsed(), load_at.elapsed());
                            reload_started = None;
                        }
                    }
                    BackendEvent::Eof => ended = true,
                    BackendEvent::Error(c, m) => println!("# ERROR {c} {m}"),
                    _ => {}
                }
            }
            if be.frame_pending() {
                let _ = be.take_frame(); // the page would paint it; vo=libmpv stalls if nobody does
            }
            if wall >= next_sample {
                next_sample += 0.25;
                let p: Vec<f64> = props.iter().map(|n| be.prop_f64(n).unwrap_or(f64::NAN)).collect();
                let tp = be.prop_f64("time-pos").unwrap_or(tpos);
                println!("{wall:7.2} {tp:9.3} {:9.4} {:10.4} {:7} {:7} {:10.3} {:10.4}", p[0], p[1], p[2], p[3], p[4], p[4] - tp);
            }
        }
        drop(be);
        ended
    }

    /// QM1k-AB measurement, NOT an assertion: `cargo test --release -- --ignored mse_ab_drift_60s
    /// --nocapture`. Plays a 60 s sync-marker fixture (video 4 s fragments, audio 10 s, audio
    /// appended 2 s late) in real time with ao=null, in three scenarios: linear to the end;
    /// 5 seeks inside the buffer; 2 reloads outside the live generation. Asserts only that the
    /// linear one played to the end.
    #[test]
    #[ignore]
    fn mse_ab_drift_60s() {
        if !have_tools() {
            return;
        }
        let dir = tmp("drift");
        let Some((v, a)) = make_fixture(&dir, 60, 0.0, 4, 10) else {
            eprintln!("SKIP: ffmpeg could not make the fixture");
            return;
        };
        println!("load average at start: {}", std::fs::read_to_string("/proc/loadavg").unwrap_or_default().trim());
        let linear = drift_scenario("linear (audio appended 2 s late)", &v, &a, 6.0, 2.0, 62.0, &[]);
        let seeks: Vec<(f64, Act)> = [(4.0, 30.0), (8.0, 10.0), (12.0, 42.0), (16.0, 20.0), (20.0, 50.0)].iter().map(|&(w, t)| (w, Act::Seek(t))).collect();
        drift_scenario("5 seeks inside the buffer (all appended)", &v, &a, 1000.0, 0.0, 26.0, &seeks);
        drift_scenario("2 seeks outside the live generation (reloads)", &v, &a, 1000.0, 0.0, 22.0, &[(5.0, Act::Reload(25.0)), (13.0, Act::Reload(48.0))]);
        println!("load average at end: {}", std::fs::read_to_string("/proc/loadavg").unwrap_or_default().trim());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(linear, "the linear 60 s scenario did not play to the end");
    }

    /// Delegates to mpv and records every raw `Time` the backend reports (before the core
    /// filters them), so the test can see what the OLD generation does while a seek waits.
    struct Spy(MpvBackend, std::rc::Rc<std::cell::RefCell<Vec<f64>>>);

    impl MediaBackend for Spy {
        fn name(&self) -> &'static str {
            self.0.name()
        }
        fn available(&self) -> bool {
            self.0.available()
        }
        fn load(&mut self, r: &crate::media_el::LoadReq) -> Result<(), LoadError> {
            self.0.load(r)
        }
        fn load_mse(&mut self, f: std::sync::Arc<MseFeed>, s: f64) -> Result<(), LoadError> {
            self.0.load_mse(f, s)
        }
        fn unload(&mut self) {
            self.0.unload()
        }
        fn set_paused(&mut self, p: bool) {
            self.0.set_paused(p)
        }
        fn seek(&mut self, t: f64) {
            self.0.seek(t)
        }
        fn set_volume(&mut self, v: f64) {
            self.0.set_volume(v)
        }
        fn set_muted(&mut self, m: bool) {
            self.0.set_muted(m)
        }
        fn set_rate(&mut self, r: f64) {
            self.0.set_rate(r)
        }
        fn poll(&mut self, wait: Duration) -> Vec<BackendEvent> {
            let evs = self.0.poll(wait);
            for e in &evs {
                if let BackendEvent::Time(t) = e {
                    self.1.borrow_mut().push(*t);
                }
            }
            evs
        }
        fn buffered(&self) -> Vec<(f64, f64)> {
            self.0.buffered()
        }
        fn seekable(&self) -> Vec<(f64, f64)> {
            self.0.seekable()
        }
        fn active(&self) -> bool {
            self.0.active()
        }
    }

    /// QYSK1a: an out-of-buffer seek must not leave the old generation playing (R51).
    #[test]
    fn qysk1a_out_of_buffer_seek_holds_old_generation_then_resumes_at_target() {
        if !have_tools() {
            return;
        }
        let dir = tmp("hold");
        let Some((v, a)) = make_fixture(&dir, 60, 0.0, 4, 10) else {
            eprintln!("SKIP: ffmpeg could not make the fixture");
            return;
        };
        let times = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let feed = MseFeed::with_fmp4_tracks(2);
        let mut c = crate::media_el::MediaCore::new(Box::new(Spy(backend(), times.clone())));
        c.attach_mse(feed.clone(), f64::NAN, "https://page.test/".into());
        c.set_muted(true);
        c.mse_set_duration(60.0);
        let t0 = Instant::now();
        let ms = |t0: &Instant| t0.elapsed().as_secs_f64() * 1000.0;
        let app = |c: &mut crate::media_el::MediaCore, bi: usize, bytes: &[u8]| {
            let o = feed.append(bi, bytes, 0.0).unwrap();
            c.mse_data_changed(t0.elapsed().as_secs_f64() * 1000.0, o.new_generation);
        };
        app(&mut c, 0, &v.init);
        app(&mut c, 1, &a.init);
        // 0-16 s of video, 0-20 s of audio
        for i in 0..4 {
            app(&mut c, 0, &v.segs[i].2);
        }
        for i in 0..2 {
            app(&mut c, 1, &a.segs[i].2);
        }
        c.play(1);
        while c.current_time < 3.0 && t0.elapsed().as_secs_f64() < 30.0 {
            c.pump(ms(&t0), Duration::from_millis(20));
            c.take_outs();
        }
        assert!(c.current_time >= 3.0, "plays from 0: {}", c.current_time);
        let old = c.current_time;
        c.set_current_time(40.0);
        c.pause(); // YouTube's scrub-pause, then its play()
        c.play(2);
        times.borrow_mut().clear();
        let t1 = Instant::now();
        while t1.elapsed() < Duration::from_secs(2) {
            c.pump(ms(&t0), Duration::from_millis(20));
            c.take_outs();
        }
        assert!(c.seeking && c.current_time == 40.0, "still waiting at the target: {} {}", c.seeking, c.current_time);
        let leaked = times.borrow().iter().cloned().fold(f64::MIN, f64::max);
        assert!(leaked <= old + 0.5, "the old generation kept playing: time-pos reached {leaked} (was {old})");
        // data for 40 arrives: 36-56 s of video, 30-50 s of audio
        for i in 9..14 {
            app(&mut c, 0, &v.segs[i].2);
        }
        for i in 3..5 {
            app(&mut c, 1, &a.segs[i].2);
        }
        let t2 = Instant::now();
        while c.current_time < 40.5 && t2.elapsed().as_secs_f64() < 10.0 {
            c.pump(ms(&t0), Duration::from_millis(20));
            c.take_outs();
        }
        assert!(!c.seeking && c.current_time >= 40.5, "resumed at the target: seeking {} t {}", c.seeking, c.current_time);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
