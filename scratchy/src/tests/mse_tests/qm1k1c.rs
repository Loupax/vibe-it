// QM1k1c: MSE in the element core (MediaCore + FakeBackend, no JS, no mpv).
use super::{init_seg, seg4, TS};
use crate::media_el::*;
use crate::media_net::MediaMode;
use crate::mse_store::MseFeed;
use std::sync::Arc;

const PAGE: &str = "https://page.test/";

fn setup(tracks: usize) -> (MediaCore, FakeHandle, Arc<MseFeed>) {
    let f = fake_new();
    let mut c = MediaCore::new(Box::new(FakeBackend(f.clone())));
    let feed = MseFeed::with_fmp4_tracks(tracks);
    c.attach_mse(feed.clone(), f64::NAN, PAGE.into());
    (c, f, feed)
}

fn names(c: &mut MediaCore) -> Vec<String> {
    c.take_outs()
        .into_iter()
        .filter_map(|o| match o {
            Out::Fire(n) => Some(n.to_string()),
            Out::MseOpen(_) => Some("sourceopen".into()),
            Out::MseEnded(_) => Some("sourceended".into()),
            Out::MseClose(_) => Some("sourceclose".into()),
            Out::BackendStarted | Out::Unavailable => None,
            o => Some(format!("{o:?}")),
        })
        .collect()
}

fn pump(c: &mut MediaCore, now: f64) -> Vec<String> {
    c.pump(now, std::time::Duration::ZERO);
    names(c)
}

fn append(c: &mut MediaCore, feed: &MseFeed, bi: usize, data: &[u8], now: f64) {
    let o = feed.append(bi, data, 0.0).unwrap();
    c.mse_data_changed(now, o.new_generation);
}

fn calls(f: &FakeHandle) -> Vec<String> {
    f.borrow().calls.clone()
}

fn count(f: &FakeHandle, prefix: &str) -> usize {
    calls(f).iter().filter(|c| c.starts_with(prefix)).count()
}

fn has(n: &[String], s: &str) -> bool {
    n.iter().any(|x| x == s)
}

/// Both tracks with init + `segs` 4-second fragments from 0, the load issued.
fn started(segs: u64) -> (MediaCore, FakeHandle, Arc<MseFeed>) {
    let (mut c, f, feed) = setup(2);
    names(&mut c);
    append(&mut c, &feed, 0, &init_seg(1, TS, 3600), 0.0);
    append(&mut c, &feed, 1, &init_seg(1, TS, 3600), 0.0);
    for i in 0..segs {
        append(&mut c, &feed, 0, &seg4(i * 4, 200), 1.0);
        append(&mut c, &feed, 1, &seg4(i * 4, 100), 1.0);
    }
    (c, f, feed)
}

/// The backend answers the load: metadata, then the first frame.
fn settle(c: &mut MediaCore, f: &FakeHandle, now: f64) -> Vec<String> {
    f.meta(f64::NAN, 64, 64);
    f.first_frame();
    pump(c, now)
}

#[test]
fn qm1k1c_event_order_on_attach() {
    let (mut c, f, feed) = setup(2);
    assert_eq!(names(&mut c), ["loadstart", "sourceopen"]);
    assert_eq!(c.network_state, NETWORK_LOADING);
    assert!(c.mse_attached());
    // only one init: no metadata yet
    append(&mut c, &feed, 0, &init_seg(1, TS, 3600), 0.0);
    assert!(!has(&names(&mut c), "loadedmetadata"));
    assert_eq!(c.ready_state, HAVE_NOTHING);
    // both inits: loadedmetadata, but no backend load until data covers the start
    c.mse_set_duration(12.0);
    append(&mut c, &feed, 1, &init_seg(1, TS, 3600), 600.0);
    let n = names(&mut c);
    assert!(has(&n, "durationchange") && has(&n, "loadedmetadata"), "{n:?}");
    assert_eq!(c.ready_state, HAVE_METADATA);
    assert_eq!(count(&f, "load_mse"), 0, "{:?}", calls(&f));
    // video data only: still waiting for the audio track
    append(&mut c, &feed, 0, &seg4(0, 200), 700.0);
    assert_eq!(count(&f, "load_mse"), 0);
    append(&mut c, &feed, 1, &seg4(0, 100), 800.0);
    assert_eq!(count(&f, "load_mse"), 1, "{:?}", calls(&f));
    assert!(calls(&f).iter().any(|x| x == &format!("load_mse {} 0", feed.id)));
    names(&mut c);
    let n = settle(&mut c, &f, 900.0);
    assert_eq!(n, ["loadeddata", "canplay", "canplaythrough"], "{n:?}");
    assert_eq!((c.video_width, c.video_height), (64, 64));
    assert_eq!(c.duration, 12.0, "the backend's duration is ignored");
    assert_eq!(c.network_state, NETWORK_IDLE);
}

#[test]
fn qm1k1c_buffered_is_the_intersection_and_seekable_follows_duration() {
    let (mut c, _f, feed) = setup(2);
    append(&mut c, &feed, 0, &init_seg(1, TS, 3600), 0.0);
    append(&mut c, &feed, 1, &init_seg(1, TS, 3600), 0.0);
    append(&mut c, &feed, 0, &seg4(0, 10), 0.0);
    append(&mut c, &feed, 0, &seg4(4, 10), 0.0);
    append(&mut c, &feed, 1, &seg4(0, 10), 0.0);
    assert_eq!(c.buffered(), vec![(0.0, 4.0)]);
    assert!(c.seekable().is_empty(), "duration unknown");
    c.mse_set_duration(8.0);
    assert_eq!(c.seekable(), vec![(0.0, 8.0)]);
    // after endOfStream the shorter track's last range reaches the highest end
    c.mse_end_of_stream();
    assert_eq!(c.buffered(), vec![(0.0, 8.0)]);
}

#[test]
fn qm1k1c_progress_is_throttled() {
    let (mut c, _f, feed) = setup(1);
    names(&mut c);
    let mut seen = 0;
    for (i, t) in [0.0, 100.0, 200.0, 400.0, 500.0, 800.0].iter().enumerate() {
        let d = if i == 0 { init_seg(1, TS, 3600) } else { seg4(4 * (i as u64 - 1), 10) };
        append(&mut c, &feed, 0, &d, *t);
        seen += names(&mut c).iter().filter(|n| *n == "progress").count();
    }
    assert_eq!(seen, 3, "at 0, 400 and 800 ms");
}

#[test]
fn qm1k1c_seek_inside_the_generation_is_a_backend_seek() {
    let (mut c, f, _feed) = started(2);
    c.mse_set_duration(8.0);
    settle(&mut c, &f, 0.0);
    c.set_current_time(6.0);
    assert_eq!(count(&f, "seek"), 1, "{:?}", calls(&f));
    assert_eq!(count(&f, "load_mse"), 1);
    let n = pump(&mut c, 10.0);
    assert!(has(&n, "seeked"), "{n:?}");
}

#[test]
fn qm1k1c_seek_outside_the_buffer_waits_then_reloads_at_t() {
    let (mut c, f, feed) = started(1);
    c.mse_set_duration(60.0);
    c.set_muted(true);
    settle(&mut c, &f, 0.0);
    c.play(1);
    names(&mut c);
    c.set_current_time(20.0);
    let n = names(&mut c);
    assert_eq!(n, ["seeking", "waiting"], "{n:?}");
    assert_eq!(c.ready_state, HAVE_METADATA);
    assert!(c.seeking);
    assert_eq!(count(&f, "seek"), 0);
    assert_eq!(count(&f, "load_mse"), 1, "no reload before both tracks have data at 20");
    // only the video track covers 20: still waiting
    append(&mut c, &feed, 0, &seg4(20, 100), 5.0);
    assert_eq!(count(&f, "load_mse"), 1);
    append(&mut c, &feed, 1, &seg4(20, 100), 6.0);
    assert_eq!(count(&f, "load_mse"), 2, "{:?}", calls(&f));
    assert!(calls(&f).iter().any(|x| x == &format!("load_mse {} 20", feed.id)), "{:?}", calls(&f));
    assert_eq!(count(&f, "unload"), 1);
    names(&mut c);
    let n = settle(&mut c, &f, 7.0);
    assert!(has(&n, "seeked") && has(&n, "canplay"), "{n:?}");
    assert!(!has(&n, "loadeddata"), "loadeddata fires once: {n:?}");
    assert!(!c.seeking);
    assert_eq!(c.current_time, 20.0);
}

#[test]
fn qm1k1c_ended_only_after_end_of_stream() {
    let (mut c, f, _feed) = started(2);
    c.mse_set_duration(8.0);
    c.set_muted(true);
    settle(&mut c, &f, 0.0);
    c.play(1);
    names(&mut c);
    // the player runs dry at the end of the appended data: a stall, not `ended`
    f.push(BackendEvent::Eof);
    let n = pump(&mut c, 10.0);
    assert!(!has(&n, "ended"), "{n:?}");
    assert!(has(&n, "waiting"), "{n:?}");
    assert!(!c.ended());
    c.mse_end_of_stream();
    assert_eq!(names(&mut c), ["sourceended"], "duration already equals the buffered end");
    f.push(BackendEvent::Eof);
    let n = pump(&mut c, 20.0);
    assert!(has(&n, "ended"), "{n:?}");
    assert!(c.ended());
}

#[test]
fn qm1k1c_end_of_stream_sets_the_duration_to_the_buffered_end() {
    let (mut c, _f, _feed) = started(2);
    c.mse_set_duration(100.0);
    names(&mut c);
    c.mse_end_of_stream();
    assert_eq!(names(&mut c), ["sourceended", "durationchange"]);
    assert_eq!(c.duration, 8.0);
}

#[test]
fn qm1k1c_duration_changes_fire_once_and_backend_durations_are_ignored() {
    let (mut c, _f, _feed) = setup(1);
    names(&mut c);
    c.mse_set_duration(30.0);
    assert_eq!(names(&mut c), ["durationchange"]);
    c.mse_set_duration(30.0);
    assert!(names(&mut c).is_empty());
    c.mse_set_duration(31.5);
    assert_eq!(names(&mut c), ["durationchange"]);
    c.handle(BackendEvent::DurationChanged(99.0), 0.0);
    assert!(names(&mut c).is_empty());
    assert_eq!(c.duration, 31.5);
}

#[test]
fn qm1k1c_a_new_media_source_closes_the_old_one_and_loads_fresh() {
    let (mut c, f, feed) = started(1);
    settle(&mut c, &f, 0.0);
    names(&mut c);
    let feed2 = MseFeed::with_fmp4_tracks(1);
    c.attach_mse(feed2.clone(), 7.0, PAGE.into());
    let n = names(&mut c);
    for want in ["abort", "emptied", "sourceclose", "loadstart", "sourceopen"] {
        assert!(has(&n, want), "{want} missing in {n:?}");
    }
    assert!(feed.with_store(|s| s.buffers.iter().all(|b| b.generation().is_none())), "the old feed is stopped");
    assert_eq!(c.duration, 7.0);
    assert_eq!(c.ready_state, HAVE_NOTHING);
    let before = count(&f, "load_mse");
    append(&mut c, &feed2, 0, &init_seg(1, TS, 3600), 0.0);
    append(&mut c, &feed2, 0, &seg4(0, 50), 1.0);
    assert_eq!(count(&f, "load_mse"), before + 1);
    assert!(calls(&f).iter().any(|x| x == &format!("load_mse {} 0", feed2.id)), "{:?}", calls(&f));
}

#[test]
fn qm1k1c_a_second_init_reloads_the_generation_at_the_current_time() {
    let (mut c, f, feed) = started(1);
    settle(&mut c, &f, 0.0);
    assert_eq!(count(&f, "load_mse"), 1);
    // a quality switch inside one MediaSource: a second init on the video buffer
    let o = feed.append(0, &init_seg(1, TS, 3600), 0.0).unwrap();
    assert!(o.new_generation);
    c.mse_data_changed(9.0, true);
    let loads = count(&f, "load_mse");
    append(&mut c, &feed, 0, &seg4(0, 300), 10.0);
    assert!(count(&f, "load_mse") >= loads, "{:?}", calls(&f));
    // the reload happens at the element's current time with the new init
    assert!(calls(&f).iter().filter(|x| x.starts_with("load_mse")).count() >= 2, "{:?}", calls(&f));
}

#[test]
fn qm1k1c_a_new_load_or_element_removal_detaches() {
    let (mut c, _f, feed) = started(1);
    names(&mut c);
    c.load(vec![Candidate { url: "https://x.test/a.mp4".into(), mime: String::new() }], true, MediaMode::NoCors, PAGE.into());
    assert!(has(&names(&mut c), "sourceclose"));
    assert!(!c.mse_attached());
    assert!(feed.with_store(|s| s.buffers.iter().all(|b| b.generation().is_none())));

    // removal through the host
    let fh = fake_new();
    let fh2 = fh.clone();
    set_backend_factory(Some(std::rc::Rc::new(move |_: &BackendCtx| -> Box<dyn MediaBackend> { Box::new(FakeBackend(fh2.clone())) })));
    let mut host = MediaHost::new(BackendCtx { socket: None, policy: Default::default() }, std::rc::Rc::new(|_, _| {}));
    set_backend_factory(None);
    let feed = host.mse.create();
    assert!(host.mse.get(feed.id).is_some());
    host.with(5, |c| c.attach_mse(feed.clone(), f64::NAN, PAGE.into()));
    let outs = host.remove(5);
    assert!(outs.iter().any(|(i, o)| *i == 5 && *o == Out::MseClose(feed.id)), "{outs:?}");
    host.mse.drop_feed(feed.id);
    assert!(host.mse.get(feed.id).is_none());
}
