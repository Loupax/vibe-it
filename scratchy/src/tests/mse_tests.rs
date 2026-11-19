// QM1k1a: src/mse_store.rs (fMP4 byte-stream reader, segment store, generations, cap).
use crate::mse_store::*;

// ---------- synthetic fMP4 builders ----------

fn bx(ty: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(ty);
    v.extend_from_slice(payload);
    v
}

fn be(n: u32) -> [u8; 4] {
    n.to_be_bytes()
}

/// ftyp + moov with one track (tkhd v0, mdhd v0) and a trex default duration.
pub(super) fn init_seg(track: u32, timescale: u32, trex_dur: u32) -> Vec<u8> {
    let mut tkhd = vec![0, 0, 0, 3];
    tkhd.extend_from_slice(&[0; 8]); // ctime, mtime
    tkhd.extend_from_slice(&be(track));
    tkhd.extend_from_slice(&[0; 68]);
    let mut mdhd = vec![0, 0, 0, 0];
    mdhd.extend_from_slice(&[0; 8]);
    mdhd.extend_from_slice(&be(timescale));
    mdhd.extend_from_slice(&[0; 8]);
    let mdia = bx(b"mdia", &bx(b"mdhd", &mdhd));
    let mut trak = bx(b"tkhd", &tkhd);
    trak.extend_from_slice(&mdia);
    let mut trex = vec![0, 0, 0, 0];
    trex.extend_from_slice(&be(track));
    trex.extend_from_slice(&be(1));
    trex.extend_from_slice(&be(trex_dur));
    trex.extend_from_slice(&[0; 8]);
    let mut moov = bx(b"trak", &trak);
    moov.extend_from_slice(&bx(b"mvex", &bx(b"trex", &trex)));
    let mut out = bx(b"ftyp", b"isom\0\0\0\0isomiso6");
    out.extend_from_slice(&bx(b"moov", &moov));
    out
}

/// How the fragment states its sample durations.
#[derive(Clone, Copy)]
enum Dur {
    /// trun with a duration per sample: (count, each).
    Explicit(u32, u32),
    /// trun without durations, tfhd default_sample_duration: (count, default).
    TfhdDefault(u32, u32),
    /// trun without durations and tfhd without a default: the trex default; (count).
    TrexDefault(u32),
}

/// [styp] + moof(mfhd, traf(tfhd, tfdt v1, trun)) + mdat(`payload` bytes).
fn frag(track: u32, tfdt: u64, d: Dur, payload: usize, with_styp: bool) -> Vec<u8> {
    let (tfhd_flags, default, trun_flags, count, each): (u32, u32, u32, u32, Option<u32>) = match d {
        Dur::Explicit(c, e) => (0x2_0000, 0, 0x101, c, Some(e)),
        Dur::TfhdDefault(c, def) => (0x2_0008, def, 0x1, c, None),
        Dur::TrexDefault(c) => (0x2_0000, 0, 0x1, c, None),
    };
    let mut tfhd = be(tfhd_flags).to_vec();
    tfhd.extend_from_slice(&be(track));
    if tfhd_flags & 0x8 != 0 {
        tfhd.extend_from_slice(&be(default));
    }
    let mut tfdt_b = vec![1, 0, 0, 0];
    tfdt_b.extend_from_slice(&tfdt.to_be_bytes());
    let mut trun = be(trun_flags).to_vec();
    trun.extend_from_slice(&be(count));
    trun.extend_from_slice(&be(0)); // data_offset
    if let Some(e) = each {
        for _ in 0..count {
            trun.extend_from_slice(&be(e));
        }
    }
    let mut traf = bx(b"tfhd", &tfhd);
    traf.extend_from_slice(&bx(b"tfdt", &tfdt_b));
    traf.extend_from_slice(&bx(b"trun", &trun));
    let mut moof = bx(b"mfhd", &[0; 8]);
    moof.extend_from_slice(&bx(b"traf", &traf));
    let mut out = Vec::new();
    if with_styp {
        out.extend_from_slice(&bx(b"styp", b"msdh\0\0\0\0msdh"));
    }
    out.extend_from_slice(&bx(b"moof", &moof));
    let data: Vec<u8> = (0..payload).map(|i| ((i * 31 + tfdt as usize) % 251) as u8).collect();
    out.extend_from_slice(&bx(b"mdat", &data));
    out
}

const TS: u32 = 90_000;

/// A 4-second fragment starting at `sec` (timescale 90000, 100 samples of 3600 ticks).
fn seg4(sec: u64, payload: usize) -> Vec<u8> {
    frag(1, sec * u64::from(TS), Dur::Explicit(100, 3600), payload, false)
}

fn buffer_with_init() -> SourceBufferStore {
    let mut b = SourceBufferStore::fmp4();
    let o = b.append(&init_seg(1, TS, 3600)).unwrap();
    assert!(!o.new_generation);
    b
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn ranges(b: &SourceBufferStore) -> Vec<(i64, i64)> {
    b.buffered().iter().map(|&(s, e)| ((s * 1000.0).round() as i64, (e * 1000.0).round() as i64)).collect()
}

// ---------- reader ----------

#[test]
fn qm1k1a_init_and_timing_from_container_headers() {
    let mut r = Fmp4Reader::new();
    let init = init_seg(1, TS, 3600);
    let ev = r.feed(&init);
    assert_eq!(ev, vec![Parsed::Init(init)]);
    // explicit per-sample durations (100 * 3600 ticks = 4 s) at tfdt 30 s, with an styp prefix
    let f = frag(1, 30 * u64::from(TS), Dur::Explicit(100, 3600), 50, true);
    match &r.feed(&f)[..] {
        [Parsed::Media { bytes, start, end }] => {
            assert_eq!(bytes, &f, "the segment keeps its styp+moof+mdat bytes");
            assert!(approx(*start, 30.0) && approx(*end, 34.0), "{start} {end}");
        }
        other => panic!("{other:?}"),
    }
    // tfhd default_sample_duration
    let f = frag(1, 34 * u64::from(TS), Dur::TfhdDefault(50, 7200), 10, false);
    match &r.feed(&f)[..] {
        [Parsed::Media { start, end, .. }] => assert!(approx(*start, 34.0) && approx(*end, 38.0)),
        other => panic!("{other:?}"),
    }
    // trex default (3600 ticks = 0.04 s) x 25 = 1 s
    let f = frag(1, 38 * u64::from(TS), Dur::TrexDefault(25), 10, false);
    match &r.feed(&f)[..] {
        [Parsed::Media { start, end, .. }] => assert!(approx(*start, 38.0) && approx(*end, 39.0)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn qm1k1a_missing_tfdt_continues_the_track() {
    // a moof without tfdt starts where the previous one on that track ended
    let mut r = Fmp4Reader::new();
    r.feed(&init_seg(1, TS, 3600));
    r.feed(&seg4(0, 10));
    let mut f = seg4(4, 10);
    // rebuild without tfdt: drop the tfdt box by renaming it to an ignored type
    let at = f.windows(4).position(|w| w == b"tfdt").unwrap();
    f[at..at + 4].copy_from_slice(b"free");
    match &r.feed(&f)[..] {
        [Parsed::Media { start, end, .. }] => assert!(approx(*start, 4.0) && approx(*end, 8.0), "{start} {end}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn qm1k1a_appends_split_at_every_byte_offset() {
    let init = init_seg(1, TS, 3600);
    let s0 = frag(1, 0, Dur::Explicit(100, 3600), 40, true);
    let s1 = frag(1, 4 * u64::from(TS), Dur::TfhdDefault(100, 3600), 33, false);
    let mut all = init.clone();
    all.extend_from_slice(&s0);
    all.extend_from_slice(&s1);
    let mut whole = SourceBufferStore::fmp4();
    whole.append(&all).unwrap();
    let expect = whole.buffered();
    assert_eq!(expect.len(), 1);
    for k in 0..=all.len() {
        let mut b = SourceBufferStore::fmp4();
        b.append(&all[..k]).unwrap();
        // buffered after the first half equals what whole boxes in all[..k] give
        let done_init = k >= init.len();
        let done_s0 = k >= init.len() + s0.len();
        let done_s1 = k >= all.len();
        let expect_now: Vec<(f64, f64)> = match (done_s0, done_s1) {
            (_, true) => vec![(0.0, 8.0)],
            (true, false) => vec![(0.0, 4.0)],
            _ => vec![],
        };
        assert_eq!(b.has_init(), done_init, "split {k}");
        assert_eq!(b.buffered(), expect_now, "split {k}");
        b.append(&all[k..]).unwrap();
        assert_eq!(b.buffered(), expect, "split {k}");
        assert_eq!(b.segments().len(), 2);
        assert_eq!(b.init_bytes().unwrap(), &init[..]);
    }
    // and one byte at a time
    let mut b = SourceBufferStore::fmp4();
    for byte in &all {
        b.append(std::slice::from_ref(byte)).unwrap();
    }
    assert_eq!(b.buffered(), expect);
}

#[test]
fn qm1k1a_parse_errors_are_errors_until_abort() {
    // media before init
    let mut b = SourceBufferStore::fmp4();
    assert!(matches!(b.append(&seg4(0, 5)), Err(MseError::Parse(_))));
    assert!(matches!(b.append(&init_seg(1, TS, 3600)), Err(MseError::Parse(_))), "stays failed");
    b.abort();
    assert!(b.append(&init_seg(1, TS, 3600)).is_ok());
    assert!(b.has_init());
    // mdat without moof
    assert!(matches!(b.append(&bx(b"mdat", &[1, 2, 3])), Err(MseError::Parse(_))));
    b.abort();
    // a size-0 top-level box, a size smaller than its header
    for bad in [vec![0, 0, 0, 0, b'm', b'o', b'o', b'f'], vec![0, 0, 0, 4, b'm', b'o', b'o', b'f']] {
        let mut b = buffer_with_init();
        assert!(matches!(b.append(&bad), Err(MseError::Parse(_))), "{bad:?}");
    }
    // trun shorter than its sample count, unknown track, zero timescale, no duration anywhere
    let mut b = buffer_with_init();
    let mut f = seg4(0, 5);
    let at = f.windows(4).position(|w| w == b"trun").unwrap();
    f[at + 8..at + 12].copy_from_slice(&be(100_000)); // sample_count
    assert!(matches!(b.append(&f), Err(MseError::Parse(_))));
    let mut b = buffer_with_init();
    assert!(matches!(b.append(&frag(9, 0, Dur::Explicit(1, 1), 1, false)), Err(MseError::Parse(_))));
    let mut b = SourceBufferStore::fmp4();
    assert!(matches!(b.append(&init_seg(1, 0, 1)), Err(MseError::Parse(_))));
    let mut b = SourceBufferStore::fmp4();
    b.append(&init_seg(1, TS, 0)).unwrap();
    assert!(matches!(b.append(&frag(1, 0, Dur::TrexDefault(3), 1, false)), Err(MseError::Parse(_))));
    // segments parsed before the error are kept
    let mut b = buffer_with_init();
    let mut data = seg4(0, 5);
    data.extend_from_slice(&bx(b"mdat", &[0]));
    assert!(b.append(&data).is_err());
    assert_eq!(b.buffered(), vec![(0.0, 4.0)]);
}

// ---------- store ----------

#[test]
fn qm1k1a_buffered_merges_within_the_gap_tolerance() {
    let mut b = buffer_with_init();
    b.append(&seg4(0, 10)).unwrap();
    b.append(&seg4(4, 10)).unwrap();
    assert_eq!(ranges(&b), vec![(0, 8000)]);
    // a 0.08 s hole merges, a 1 s hole does not
    b.append(&frag(1, 8 * u64::from(TS) + 7200, Dur::Explicit(10, 3600), 10, false)).unwrap();
    assert_eq!(ranges(&b), vec![(0, 8480)]);
    b.append(&seg4(10, 10)).unwrap();
    assert_eq!(ranges(&b), vec![(0, 8480), (10000, 14000)]);
    // out-of-order appends are kept sorted
    b.append(&seg4(20, 10)).unwrap();
    b.append(&seg4(14, 10)).unwrap();
    assert_eq!(ranges(&b), vec![(0, 8480), (10000, 18000), (20000, 24000)]);
}

#[test]
fn qm1k1a_remove_drops_only_segments_fully_inside() {
    let mut b = buffer_with_init();
    for s in [0, 4, 8, 12] {
        b.append(&seg4(s, 10)).unwrap();
    }
    assert_eq!(b.remove(1.0, 8.0), 1, "only [4,8) is fully inside [1,8]");
    assert_eq!(ranges(&b), vec![(0, 4000), (8000, 16000)]);
    assert_eq!(b.remove(0.0, f64::INFINITY), 3);
    assert!(b.buffered().is_empty());
    assert_eq!(b.segments().len(), 0);
    assert!(b.has_init(), "remove keeps the init segment");
}

#[test]
fn qm1k1a_overlap_replaces_whole_segments() {
    let mut b = buffer_with_init();
    b.append(&seg4(0, 10)).unwrap();
    b.append(&seg4(4, 10)).unwrap();
    // an identical-range re-append replaces the stored one (new bytes win)
    let newer = seg4(4, 77);
    b.append(&newer).unwrap();
    assert_eq!(b.segments().len(), 2);
    assert_eq!(&*b.segments()[1].data, &newer);
    // a segment straddling two old ones replaces both
    b.append(&frag(1, 2 * u64::from(TS), Dur::Explicit(100, 3600), 5, false)).unwrap();
    assert_eq!(b.segments().len(), 1);
    assert_eq!(ranges(&b), vec![(2000, 6000)]);
    // touching (abutting) segments do not overlap
    b.append(&seg4(6, 5)).unwrap();
    assert_eq!(b.segments().len(), 2);
}

#[test]
fn qm1k1a_second_init_flags_a_new_generation_and_old_media_stays_out_of_the_run() {
    let mut b = buffer_with_init();
    b.append(&seg4(0, 10)).unwrap();
    assert!(b.start_generation(1, 0.0));
    let first = b.generation_bytes().unwrap();
    assert_eq!(&first[..init_seg(1, TS, 3600).len()], &init_seg(1, TS, 3600)[..]);
    let init2 = init_seg(1, 48_000, 1024);
    let o = b.append(&init2).unwrap();
    assert!(o.new_generation);
    assert!(b.generation().is_none(), "the live generation ended; the caller starts a new one");
    // old-init media is still buffered, but a generation on the new init cannot use it
    assert_eq!(ranges(&b), vec![(0, 4000)]);
    assert!(b.start_generation(2, 0.0));
    assert_eq!(b.generation_bytes().unwrap(), init2);
    b.append(&frag(1, 4 * 48_000, Dur::Explicit(10, 4800), 20, false)).unwrap();
    // start 4 s: the run needs the segment covering 4 s, so wait: start the generation at 4
    assert!(b.start_generation(3, 4.0));
    let bytes = b.generation_bytes().unwrap();
    assert_eq!(bytes.len(), init2.len() + b.segments()[1].data.len());
}

#[test]
fn qm1k1a_generation_bytes_are_init_plus_the_contiguous_run() {
    let mut b = buffer_with_init();
    let init = b.init_bytes().unwrap().to_vec();
    let s: Vec<Vec<u8>> = [0, 4, 8, 20].iter().map(|&t| seg4(t, 30 + t as usize)).collect();
    // appended out of order, before the generation exists
    for i in [1, 2, 0, 3] {
        b.append(&s[i]).unwrap();
    }
    assert!(b.start_generation(7, 5.0));
    let mut want = init.clone();
    want.extend_from_slice(&s[1]);
    want.extend_from_slice(&s[2]);
    assert_eq!(b.generation_bytes().unwrap(), want, "starts at the segment covering 5 s; stops at the 12-20 s hole");
    assert_eq!(b.generation_len(), want.len() as u64);
    // a later segment that continues the run joins it
    let s12 = seg4(12, 9);
    b.append(&s12).unwrap();
    assert_eq!(b.run_end(), Some(16.0));
    want.extend_from_slice(&s12);
    assert_eq!(b.generation_bytes().unwrap(), want);
    // the one at 20 s joins only when 16-20 is filled
    b.append(&seg4(16, 9)).unwrap();
    assert_eq!(b.run_end(), Some(24.0));
    // reads: init, then run, then Wait at the end, Stale for another generation
    let mut buf = [0u8; 8];
    assert_eq!(b.read_at(7, 0, &mut buf), ReadAt::Bytes(8));
    assert_eq!(&buf[..], &init[..8]);
    assert_eq!(b.read_at(7, b.generation_len(), &mut buf), ReadAt::Wait);
    assert_eq!(b.read_at(8, 0, &mut buf), ReadAt::Stale);
    // a generation starting in a hole has an empty run until a covering segment arrives
    assert!(b.start_generation(9, 13_000.0));
    assert_eq!(b.run_end(), None);
}

#[test]
fn qm1k1a_base_offset_survives_front_eviction() {
    let mut b = buffer_with_init();
    let segs: Vec<Vec<u8>> = (0..4).map(|i| seg4(i * 4, 100)).collect();
    for s in &segs {
        b.append(s).unwrap();
    }
    b.start_generation(1, 0.0);
    let init_len = b.init_bytes().unwrap().len() as u64;
    let seg_len = segs[0].len() as u64;
    assert_eq!(b.generation_base(), init_len);
    b.remove(0.0, 4.0); // front of the run
    assert_eq!(b.generation_base(), init_len + seg_len);
    assert_eq!(b.generation_len(), init_len + 4 * seg_len, "virtual size does not shrink");
    let mut buf = vec![0u8; 16];
    assert_eq!(b.read_at(1, init_len + 2, &mut buf), ReadAt::Evicted);
    assert_eq!(b.read_at(1, init_len + seg_len, &mut buf), ReadAt::Bytes(16));
    assert_eq!(&buf[..], &segs[1][..16], "remaining bytes keep their virtual offsets");
    assert!(b.generation_bytes().is_none());
    // a hole in the middle truncates the run there
    b.remove(8.0, 12.0);
    assert_eq!(b.run_end(), Some(8.0));
    assert_eq!(b.generation_len(), init_len + 2 * seg_len);
    // appending the hole back extends the run again
    b.append(&seg4(8, 100)).unwrap();
    assert_eq!(b.run_end(), Some(16.0));
}

// ---------- cap and eviction ----------

#[test]
fn qm1k1a_eviction_drops_the_oldest_played_segments_first() {
    let init_len = init_seg(1, TS, 3600).len();
    let seg_len = seg4(0, 1000).len();
    let mut m = MseStore::new(init_len + 4 * seg_len);
    let bi = m.add_buffer(SourceBufferStore::fmp4());
    m.append(bi, &init_seg(1, TS, 3600), 0.0).unwrap();
    for t in [0, 4, 8, 12] {
        m.append(bi, &seg4(t, 1000), 0.0).unwrap();
    }
    assert_eq!(m.total_bytes(), init_len + 4 * seg_len);
    // currentTime 30: [0,4) and the later ones all ended > 10 s ago? only ends < 20 count.
    m.append(bi, &seg4(28, 1000), 30.0).unwrap();
    assert_eq!(ranges(&m.buffers[bi]), vec![(4000, 16000), (28000, 32000)], "only the OLDEST was evicted");
    m.append(bi, &seg4(32, 1000), 30.0).unwrap();
    assert_eq!(ranges(&m.buffers[bi]), vec![(8000, 16000), (28000, 36000)]);
    assert!(m.total_bytes() <= m.max_bytes());
}

#[test]
fn qm1k1a_quota_exceeded_when_nothing_may_be_evicted() {
    let init_len = init_seg(1, TS, 3600).len();
    let seg_len = seg4(0, 1000).len();
    let mut m = MseStore::new(init_len + 2 * seg_len);
    let bi = m.add_buffer(SourceBufferStore::fmp4());
    m.append(bi, &init_seg(1, TS, 3600), 0.0).unwrap();
    m.append(bi, &seg4(0, 1000), 0.0).unwrap();
    m.append(bi, &seg4(4, 1000), 0.0).unwrap();
    // currentTime 0: nothing is 10 s behind or 60 s ahead
    assert_eq!(m.append(bi, &seg4(8, 1000), 0.0), Err(MseError::QuotaExceeded));
    assert_eq!(ranges(&m.buffers[bi]), vec![(0, 8000)], "a refused append stores nothing");
    // the parser was not touched: the same bytes fit once something can be evicted
    assert!(m.append(bi, &seg4(8, 1000), 40.0).is_ok());
    // removing frees room too (the page's remove-and-retry)
    let mut m2 = MseStore::new(init_len + seg_len);
    let b2 = m2.add_buffer(SourceBufferStore::fmp4());
    m2.append(b2, &init_seg(1, TS, 3600), 0.0).unwrap();
    m2.append(b2, &seg4(0, 1000), 0.0).unwrap();
    assert_eq!(m2.append(b2, &seg4(4, 1000), 0.0), Err(MseError::QuotaExceeded));
    m2.buffers[b2].remove(0.0, 4.0);
    assert!(m2.append(b2, &seg4(4, 1000), 0.0).is_ok());
}

#[test]
fn qm1k1a_eviction_ahead_is_farthest_first_and_spares_the_live_run() {
    let init_len = init_seg(1, TS, 3600).len();
    let seg_len = seg4(0, 1000).len();
    let mut m = MseStore::new(init_len + 4 * seg_len);
    let bi = m.add_buffer(SourceBufferStore::fmp4());
    m.append(bi, &init_seg(1, TS, 3600), 0.0).unwrap();
    for t in [0, 4, 100, 200] {
        m.append(bi, &seg4(t, 1000), 0.0).unwrap();
    }
    m.start_generation(0.0); // the run holds [0,8)
    m.append(bi, &seg4(300, 1000), 0.0).unwrap();
    assert_eq!(ranges(&m.buffers[bi]), vec![(0, 8000), (100_000, 104_000), (300_000, 304_000)], "200 s (farthest unreached) went first");
    m.append(bi, &seg4(8, 1000), 0.0).unwrap(); // continues the run
    assert_eq!(ranges(&m.buffers[bi]), vec![(0, 12000), (100_000, 104_000)]);
    // the run's own segments are never evicted ahead, even when over the cap
    assert_eq!(m.append(bi, &seg4(12, 3000), 0.0), Err(MseError::QuotaExceeded));
}

#[test]
fn qm1k1a_behind_the_run_needs_mpv_to_have_read_past() {
    let init_len = init_seg(1, TS, 3600).len();
    let seg_len = seg4(0, 1000).len();
    let mut m = MseStore::new(init_len + 2 * seg_len);
    let bi = m.add_buffer(SourceBufferStore::fmp4());
    m.append(bi, &init_seg(1, TS, 3600), 0.0).unwrap();
    m.append(bi, &seg4(0, 1000), 0.0).unwrap();
    m.append(bi, &seg4(4, 1000), 0.0).unwrap();
    let g = m.start_generation(0.0);
    // far in the future by the page clock, but mpv has read nothing: the run is protected
    assert_eq!(m.append(bi, &seg4(40, 1000), 60.0), Err(MseError::QuotaExceeded));
    // mpv reads the first segment (init + seg 0): that one becomes evictable
    let mut buf = vec![0u8; init_len + seg_len];
    assert_eq!(m.buffers[bi].read_at(g, 0, &mut buf[..init_len]), ReadAt::Bytes(init_len));
    let mut sb = vec![0u8; seg_len];
    assert_eq!(m.buffers[bi].read_at(g, init_len as u64, &mut sb), ReadAt::Bytes(seg_len));
    assert!(m.append(bi, &seg4(40, 1000), 60.0).is_ok());
    assert_eq!(ranges(&m.buffers[bi]), vec![(4000, 8000), (40_000, 44_000)]);
    assert_eq!(m.buffers[bi].generation_base(), (init_len + seg_len) as u64);
}

#[test]
fn qm1k1a_the_cap_is_shared_by_all_source_buffers_of_a_media_source() {
    let init_len = init_seg(1, TS, 3600).len();
    let seg_len = seg4(0, 1000).len();
    let mut m = MseStore::new(2 * init_len + 3 * seg_len);
    let v = m.add_buffer(SourceBufferStore::fmp4());
    let a = m.add_buffer(SourceBufferStore::fmp4());
    m.append(v, &init_seg(1, TS, 3600), 0.0).unwrap();
    m.append(a, &init_seg(1, TS, 3600), 0.0).unwrap();
    m.append(v, &seg4(0, 1000), 0.0).unwrap();
    m.append(a, &seg4(0, 1000), 0.0).unwrap();
    m.append(v, &seg4(4, 1000), 0.0).unwrap();
    assert_eq!(m.append(a, &seg4(4, 1000), 0.0), Err(MseError::QuotaExceeded));
    // evicting across buffers: the oldest-ending segment overall goes first
    assert!(m.append(a, &seg4(4, 1000), 30.0).is_ok());
    assert_eq!(m.buffers[v].buffered(), vec![(4.0, 8.0)]);
    assert_eq!(m.buffers[a].buffered(), vec![(0.0, 8.0)]);
}

#[test]
fn qm1k1a_cap_default_and_env_override() {
    assert_eq!(MSE_MAX_BYTES, 64 * 1024 * 1024);
    assert_eq!(parse_max_bytes(None), MSE_MAX_BYTES);
    assert_eq!(parse_max_bytes(Some("12345")), 12345);
    assert_eq!(parse_max_bytes(Some(" 777\n")), 777);
    assert_eq!(parse_max_bytes(Some("nope")), MSE_MAX_BYTES);
    assert_eq!(parse_max_bytes(Some("0")), MSE_MAX_BYTES);
    assert_eq!(parse_max_bytes(Some("-5")), MSE_MAX_BYTES);
    // SCRATCHY_MSE_MAX_BYTES itself is read by mse_max_bytes (unset in CI)
    if std::env::var_os("SCRATCHY_MSE_MAX_BYTES").is_none() {
        assert_eq!(mse_max_bytes(), MSE_MAX_BYTES);
        assert_eq!(MseStore::from_env().max_bytes(), MSE_MAX_BYTES);
    }
}

// ---------- real ffmpeg output ----------

#[test]
fn qm1k1a_ffmpeg_fragmented_mp4_matches_ffprobe_duration() {
    let dir = std::env::temp_dir().join(format!("qm1k1a-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("f.mp4");
    let made = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=160x120:rate=25", "-t", "7", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "50", "-movflags", "frag_keyframe+empty_moov+default_base_moof"])
        .arg(&p)
        .status();
    if !matches!(made, Ok(s) if s.success()) {
        eprintln!("SKIP: ffmpeg is not available");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let probe = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(&p)
        .output();
    let Ok(probe) = probe else {
        eprintln!("SKIP: ffprobe is not available");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    let dur: f64 = String::from_utf8_lossy(&probe.stdout).trim().parse().expect("ffprobe duration");
    let data = std::fs::read(&p).unwrap();
    let mut b = SourceBufferStore::fmp4();
    for chunk in data.chunks(1000) {
        b.append(chunk).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(b.has_init());
    assert!(b.segments().len() >= 3, "{} fragments", b.segments().len());
    let r = b.buffered();
    assert_eq!(r.len(), 1, "{r:?}");
    assert!(r[0].0.abs() < 0.1, "starts near 0: {r:?}");
    assert!((r[0].1 - dur).abs() <= 0.04 + 1e-9, "buffered end {} vs ffprobe {dur}", r[0].1);
    // generation bytes = init + all fragments = the file without any trailing mfra/free
    b.start_generation(1, 0.0);
    let g = b.generation_bytes().unwrap();
    assert!(data.starts_with(&g), "init + fragments are the file's leading boxes");
}

mod qm1k1c;
mod qm1k1d;
mod qm1k1e;
mod qm1k1k;
mod qm1k1l;
