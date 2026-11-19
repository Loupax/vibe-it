// QM1k1i: SourceBuffer `mode = 'sequence'`, `timestampOffset`, `appendWindowStart/End`
// (src/mse_store.rs: set_sequence/set_timestamp_offset/set_append_window + Fmp4Reader::shift;
// src/mse_prelude.js; __mse_sb_cfg/__mse_sb_tso in src/js_media.rs).
use super::qm1k1d::{attached, Js};
use super::{approx, buffer_with_init, init_seg, ranges, seg4, TS};
use crate::mse_store::*;

fn arr(b: &[u8]) -> String {
    format!("[{}]", b.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","))
}

/// The start of the first media segment a fresh reader finds in `init + seg` (what mpv parses).
fn parsed_start(buf: &SourceBufferStore, seg: &[u8]) -> f64 {
    let mut r = Fmp4Reader::new();
    let mut bytes = buf.init_bytes().unwrap().to_vec();
    bytes.extend_from_slice(seg);
    r.feed(&bytes)
        .into_iter()
        .find_map(|p| match p {
            Parsed::Media { start, .. } => Some(start),
            _ => None,
        })
        .unwrap()
}

#[test]
fn qm1k1i_segments_mode_offset_shifts_store_times_and_the_bytes_mpv_reads() {
    let mut b = buffer_with_init();
    b.set_timestamp_offset(10.0);
    let o = b.append(&seg4(0, 40)).unwrap();
    assert_eq!((o.segments, o.dropped, o.unshifted), (1, 0, 0));
    assert_eq!(ranges(&b), [(10_000, 14_000)]);
    // the stored bytes carry the shifted tfdt: a reader of them sees 10 s too
    assert!(approx(parsed_start(&b, &b.segments()[0].data), 10.0));
    // back to no offset
    b.set_timestamp_offset(0.0);
    b.append(&seg4(4, 40)).unwrap();
    assert_eq!(ranges(&b), [(4_000, 8_000), (10_000, 14_000)]);
    assert!(approx(parsed_start(&b, &b.segments()[0].data), 4.0));
}

#[test]
fn qm1k1i_a_shifted_generation_serves_the_shifted_bytes() {
    let mut b = buffer_with_init();
    b.set_timestamp_offset(30.0);
    b.append(&seg4(0, 40)).unwrap();
    b.append(&seg4(4, 40)).unwrap();
    assert_eq!(ranges(&b), [(30_000, 38_000)]);
    assert!(b.start_generation(1, 30.0));
    let all = b.generation_bytes().unwrap();
    // init + both segments, parsed back: starts 30 and 34
    let mut r = Fmp4Reader::new();
    let starts: Vec<f64> = r.feed(&all).into_iter().filter_map(|p| if let Parsed::Media { start, .. } = p { Some(start) } else { None }).collect();
    assert_eq!(starts.len(), 2);
    assert!(approx(starts[0], 30.0) && approx(starts[1], 34.0), "{starts:?}");
}

#[test]
fn qm1k1i_sequence_mode_places_groups_back_to_back() {
    let mut b = buffer_with_init();
    b.set_sequence(true);
    // the first segment of a group lands at the group start (0), later ones keep the offset
    b.append(&seg4(100, 40)).unwrap();
    b.append(&seg4(104, 40)).unwrap();
    assert_eq!(ranges(&b), [(0, 8_000)]);
    assert!(approx(b.timestamp_offset(), -100.0));
    assert!(approx(parsed_start(&b, &b.segments()[1].data), 4.0));
    // abort() starts a new group at the group end (8 s) wherever the media claims to be
    b.abort();
    b.append(&seg4(0, 40)).unwrap();
    assert_eq!(ranges(&b), [(0, 12_000)]);
    assert!(approx(b.timestamp_offset(), 8.0));
    // timestampOffset in sequence mode places the NEXT group
    b.set_timestamp_offset(20.0);
    b.append(&seg4(50, 40)).unwrap();
    assert_eq!(ranges(&b), [(0, 12_000), (20_000, 24_000)]);
    b.append(&seg4(54, 40)).unwrap();
    assert_eq!(ranges(&b), [(0, 12_000), (20_000, 28_000)]);
}

#[test]
fn qm1k1i_append_window_drops_segments_wholly_outside_and_keeps_partial_ones() {
    let mut b = buffer_with_init();
    b.set_append_window(5.0, 14.0);
    let o = b.append(&seg4(0, 40)).unwrap(); // 0-4: ends before the window
    assert_eq!((o.segments, o.dropped), (0, 1));
    b.append(&seg4(4, 40)).unwrap(); // 4-8 overlaps the start: kept whole (no frame trimming)
    b.append(&seg4(8, 40)).unwrap();
    b.append(&seg4(12, 40)).unwrap(); // 12-16 overlaps the end: kept
    let o = b.append(&seg4(16, 40)).unwrap(); // starts after the end
    assert_eq!((o.segments, o.dropped), (0, 1));
    assert_eq!(ranges(&b), [(4_000, 16_000)]);
    // the window applies to the SHIFTED times
    b.abort();
    b.set_timestamp_offset(100.0);
    b.set_append_window(105.0, 110.0);
    let o = b.append(&seg4(0, 40)).unwrap(); // shifted 100-104
    assert_eq!(o.dropped, 1);
    let o = b.append(&seg4(4, 40)).unwrap(); // shifted 104-108
    assert_eq!(o.segments, 1);
    // abort() resets the window
    b.abort();
    assert_eq!(b.append_window(), (0.0, f64::INFINITY));
}

#[test]
fn qm1k1i_an_unshiftable_segment_is_stored_unshifted_and_counted() {
    let mut b = buffer_with_init();
    b.set_timestamp_offset(-1.0); // would move the tfdt below zero
    let o = b.append(&seg4(0, 40)).unwrap();
    assert_eq!((o.segments, o.unshifted), (1, 1));
    // store and bytes still agree
    assert_eq!(ranges(&b), [(0, 4_000)]);
    assert!(approx(parsed_start(&b, &b.segments()[0].data), 0.0));
}

fn with_buffer() -> Js {
    let js = attached();
    js.run(&format!(
        "var INIT = new Uint8Array({}), S0 = new Uint8Array({}), S1 = new Uint8Array({}), S2 = new Uint8Array({});\
         var sb = ms.addSourceBuffer('video/mp4; codecs=\"avc1.64001f\"');",
        arr(&init_seg(1, TS, 3600)),
        arr(&seg4(0, 40)),
        arr(&seg4(4, 40)),
        arr(&seg4(8, 40)),
    ));
    js.run("sb.appendBuffer(INIT);");
    js.log.borrow_mut().clear();
    js
}

#[test]
fn qm1k1i_js_timestamp_offset_shifts_buffered() {
    let js = with_buffer();
    js.run("sb.timestampOffset = 20; sb.appendBuffer(S0);");
    assert_eq!(js.val("[sb.timestampOffset, sb.buffered.length, sb.buffered.start(0), sb.buffered.end(0)].join()"), "20,1,20,24");
    assert!(js.lines().is_empty(), "{:?}", js.lines());
}

#[test]
fn qm1k1i_js_sequence_mode_moves_the_group_to_the_group_end() {
    let js = with_buffer();
    js.run("sb.mode = 'sequence'; sb.appendBuffer(S1);");
    // S1 is 4-8 but the group starts at 0
    assert_eq!(js.val("[sb.mode, sb.timestampOffset, sb.buffered.start(0), sb.buffered.end(0)].join()"), "sequence,-4,0,4");
    js.run("sb.appendBuffer(S2);");
    assert_eq!(js.val("[sb.buffered.length, sb.buffered.end(0)].join()"), "1,8");
    // no `not implemented` line any more
    assert!(js.lines().is_empty(), "{:?}", js.lines());
    // a plain mode switch back keeps the offset
    js.run("sb.mode = 'segments';");
    assert_eq!(js.val("[sb.mode, sb.timestampOffset].join()"), "segments,-4");
}

#[test]
fn qm1k1i_js_append_window_and_abort() {
    let js = with_buffer();
    js.run("sb.appendWindowStart = 5; sb.appendWindowEnd = 13; sb.appendBuffer(S0);"); js.run("sb.appendBuffer(S1);"); js.run("sb.appendBuffer(S2);");
    // S0 (0-4) is before the window, the others overlap it
    assert_eq!(js.val("[sb.buffered.length, sb.buffered.start(0), sb.buffered.end(0)].join()"), "1,4,12");
    js.run("sb.abort();");
    assert_eq!(js.val("[sb.appendWindowStart, sb.appendWindowEnd].join()"), "0,Infinity");
    // invalid combinations still throw
    assert_eq!(js.val("(function () { try { sb.appendWindowStart = 5; sb.appendWindowEnd = 3; } catch (e) { return e.name } })()"), "TypeError");
}

#[cfg(feature = "media")]
mod real {
    use super::super::super::mse_mpv_tests::real::{backend, have_tools, make_fixture, pump, Driver};
    use crate::media_el::{BackendEvent, MediaBackend};
    use crate::mse_store::*;
    use std::time::Instant;

    /// The decision test: the real mpv sees the shifted times. A clip made at 0 s is appended
    /// with `timestampOffset = 20` (segments mode) and must play at media time 20..30 with
    /// the audio in sync.
    #[test]
    fn qm1k1i_mpv_plays_a_timestamp_offset_stream_at_the_shifted_time() {
        if !have_tools() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("qm1k1i-{}", std::process::id()));
        let Some((v, a)) = make_fixture(&dir, 10, 0.0, 4, 10) else {
            eprintln!("SKIP: ffmpeg could not make the fixture");
            return;
        };
        let feed = MseFeed::with_fmp4_tracks(2);
        feed.with_store(|st| {
            for b in st.buffers.iter_mut() {
                b.set_timestamp_offset(20.0);
            }
        });
        Driver::inits(&feed, &v, &a);
        let mut drv = Driver::new(&v, &a, 6.0, 0.0);
        let mut be = backend();
        be.set_video_target(Some(crate::media_video::VideoTarget { w: 320, h: 240, fit: crate::css::ObjectFit::Contain }));
        let t0 = Instant::now();
        drv.tick(&feed, 0.0);
        let ranges = feed.with_store(|st| st.buffers[0].buffered());
        assert!(ranges.first().is_some_and(|r| (r.0 - 20.0).abs() < 0.1), "store times shifted: {ranges:?}");
        be.load_mse(feed.clone(), 20.0).unwrap();
        be.set_paused(false);
        let (mut first, mut last, mut worst, mut loaded, mut errors, mut samples) = (None, 0.0f64, 0.0f64, false, Vec::new(), 0u32);
        let ended = pump(&mut be, &feed, &mut drv, t0, 40.0, |be, e| {
            match e {
                BackendEvent::Loaded { .. } => loaded = true,
                BackendEvent::Error(c, m) => errors.push((*c, m.clone())),
                BackendEvent::Time(t) => {
                    first.get_or_insert(*t);
                    last = *t;
                    if *t > 21.5
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
        assert!(first.unwrap() >= 19.9, "mpv time-pos starts at the shifted tfdt: {first:?}");
        assert!(last > 28.5, "played to the end at shifted time: {last}");
        assert!(samples > 10 && worst < 0.1, "avsync samples {samples}, worst {worst}");
        eprintln!("qm1k1i: mpv first time {first:?}, last {last:.2}, worst |avsync| {worst:.3}");
        drop(be);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
