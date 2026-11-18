#[allow(unused_imports)]
use super::*;

fn qm1e_file(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 7 + i / 251) % 256) as u8).collect()
}

fn qm1e_src(url: &str, page: &str, mode: MediaMode) -> MediaSource {
    MediaSource { url: Url::parse(url).unwrap(), page: Url::parse(page).unwrap(), mode }
}

fn qm1e_open(w: &Qm1eWire, url: &str, page: &str, mode: MediaMode) -> Result<MediaStream, MediaNetError> {
    MediaStream::open(w.path.clone(), qm1e_src(url, page, mode), RequestPolicy::default())
}

fn qm1e_read_all(s: &mut MediaStream, mut want: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 65536];
    while want > 0 {
        let n = s.read(&mut buf[..want.min(65536)]).expect("read");
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        want -= n;
    }
    out
}

const QM1E_URL: &str = "https://cdn.example/v.mp4";
const QM1E_PAGE: &str = "https://page.example/";

#[test]
fn qm1e_206_gives_total_from_content_range_and_exact_sequential_bytes() {
    let file = qm1e_file(3 * 1024 * 1024 - 100);
    let w = qm1e_wire(Qm1eCfg { file: file.clone(), ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert_eq!(s.size(), Some(file.len() as u64));
    assert!(s.seekable());
    let got = qm1e_read_all(&mut s, file.len() + 10);
    assert!(got == file, "bytes differ");
    assert_eq!(s.requests, 3, "1 MiB chunks: 3 requests for ~3 MiB");
    assert_eq!(s.read_cb(&mut [0u8; 8]), 0, "EOF");
    for r in w.seen.lock().unwrap().iter() {
        assert!(r.contains("\r\nRange: bytes="), "{r}");
        assert!(r.contains("\r\nX-Wire-Max-Bytes: "), "{r}");
        assert!(r.contains("\r\nX-Wire-Timeout-Ms: 30000\r\n"), "{r}");
    }
    assert!(qf14b_has(&w.seen.lock().unwrap()[0], "Range: bytes=0-1048575"));
}

#[test]
fn qm1e_200_fallback_is_forward_only_and_backward_seek_restarts() {
    let file = qm1e_file(3 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg { file: file.clone(), ignore_range: true, ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert!(s.is_forward_only() && !s.seekable());
    assert_eq!(s.size(), None);
    assert_eq!(s.size_cb(), -1);
    // request 0: capped attempt (502 too-large), request 1: the streaming GET
    {
        let seen = w.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(!seen[1].contains("X-Wire-Max-Bytes"), "{}", seen[1]);
        assert!(seen[1].contains("X-Wire-Timeout-Ms: 3600000"), "{}", seen[1]);
    }
    assert!(qm1e_read_all(&mut s, 1000) == file[..1000]);
    s.seek(2_000_000).unwrap();
    assert!(qm1e_read_all(&mut s, 500) == file[2_000_000..2_000_500], "forward seek discards");
    s.seek(10).unwrap();
    assert!(qm1e_read_all(&mut s, 100) == file[10..110], "backward seek restarts from 0");
    assert_eq!(w.seen.lock().unwrap().len(), 3);
}

#[test]
fn qm1e_small_200_body_is_served_whole_and_seekable() {
    let file = qm1e_file(5000);
    let w = qm1e_wire(Qm1eCfg { file: file.clone(), ignore_range: true, ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert!(s.seekable());
    assert_eq!(s.size(), Some(5000));
    s.seek(4000).unwrap();
    assert!(qm1e_read_all(&mut s, 5000) == file[4000..]);
    s.seek(0).unwrap();
    assert!(qm1e_read_all(&mut s, 10) == file[..10]);
    assert_eq!(w.seen.lock().unwrap().len(), 1);
}

#[test]
fn qm1e_416_is_eof_even_when_the_file_shrinks() {
    let w = qm1e_wire(Qm1eCfg { file: vec![], ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert_eq!(s.size(), Some(0));
    assert_eq!(s.read(&mut [0u8; 16]).unwrap(), 0);

    let w = qm1e_wire(Qm1eCfg { file: qm1e_file(3 * 1024 * 1024), ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    w.cfg.lock().unwrap().file = qm1e_file(100);
    s.seek(2 * 1024 * 1024).unwrap();
    assert_eq!(s.size(), Some(3 * 1024 * 1024));
    // The file shrank: the next fetch answers 416 `bytes */100`.
    assert_eq!(s.read(&mut [0u8; 16]).unwrap(), 0);
    assert_eq!(s.size(), Some(100));
}

#[test]
fn qm1e_short_206_body_is_retried_once_from_the_new_position() {
    let big = qm1e_file(2 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg { file: big.clone(), truncate_once: true, ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    let got = qm1e_read_all(&mut s, big.len());
    assert!(got == big, "short body healed by one retry");
    // open + short second chunk + its retry
    assert_eq!(w.seen.lock().unwrap().len(), 3);
    let retry = w.seen.lock().unwrap()[2].clone();
    let line = retry.lines().find(|l| l.starts_with("Range:")).unwrap().to_string();
    let start: u64 = line["Range: bytes=".len()..].split('-').next().unwrap().parse().unwrap();
    assert!(start > CHUNK_SMALL && start < 2 * CHUNK_SMALL, "retry continues after the bytes that arrived: {line}");
}

#[test]
fn qm1e_short_206_on_a_seek_miss_also_heals() {
    // Every ranged answer from request #1 on is short: #1 truncated, retry (#2) also short.
    let big = qm1e_file(2 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg { file: big.clone(), truncate_once: true, ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    s.seek(CHUNK_SMALL).unwrap();
    // idx 1 is truncated, its retry (idx 2) is whole: heals. A third miss is whole too.
    assert!(qm1e_read_all(&mut s, 10) == big[CHUNK_SMALL as usize..CHUNK_SMALL as usize + 10]);
}

#[test]
fn qm1e_wrong_content_range_start_fails() {
    let w = qm1e_wire(Qm1eCfg { file: qm1e_file(5000), shift_start: 1, ..Default::default() });
    let e = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).err().expect("must fail");
    assert!(matches!(e, MediaNetError::BadRange(_)), "{e:?}");
}

#[test]
fn qm1e_redirect_is_pinned_and_a_cross_origin_later_final_url_fails() {
    let file = qm1e_file(3 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg {
        file: file.clone(),
        final_urls: vec![(0, "https://cdn2.example/real.mp4".into()), (1, "https://cdn2.example/real.mp4".into()), (2, "https://evil.example/x.mp4".into())],
        ..Default::default()
    });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut got = 0;
    while got < CHUNK_SMALL as usize {
        got += s.read(&mut buf[..CHUNK_SMALL as usize - got]).unwrap();
    }
    got = 0;
    while got < 10 {
        got += s.read(&mut buf[..10 - got]).unwrap();
    }
    // chunk 2 (request 1) is pinned to the first final URL
    let r1 = w.seen.lock().unwrap()[1].clone();
    assert!(r1.starts_with("GET https://cdn2.example/real.mp4 "), "{r1}");
    // chunk 3 (request 2) comes back from another origin
    let e = loop {
        match s.read(&mut buf) {
            Ok(0) => panic!("EOF before the failing chunk"),
            Ok(_) => {}
            Err(e) => break e,
        }
    };
    assert!(matches!(e, MediaNetError::MixedOrigin { .. }), "{e:?}");
}

#[test]
fn qm1e_no_cors_sends_the_jar_and_anonymous_sends_cookies_off_and_origin() {
    let w = qm1e_wire(Qm1eCfg { file: qm1e_file(100), ..Default::default() });
    qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    let r = w.seen.lock().unwrap()[0].clone();
    assert!(qf14b_has(&r, "X-Wire-Jar: scratchy") && qf14b_has(&r, "X-Wire-Top-Site: https://page.example"), "{r}");
    assert!(!r.contains("X-Wire-Cookies: off") && !r.contains("\r\nOrigin:"), "{r}");

    // CORS without ACAO: fails after the first response.
    let e = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::Anonymous).err().expect("no ACAO must fail");
    assert!(matches!(e, MediaNetError::Cors(_)), "{e:?}");
    let r = w.seen.lock().unwrap()[1].clone();
    assert!(qf14b_has(&r, "X-Wire-Cookies: off") && qf14b_has(&r, "Origin: https://page.example"), "{r}");
    assert!(!r.contains("X-Wire-Jar"), "{r}");

    w.cfg.lock().unwrap().acao = Some("*".into());
    qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::Anonymous).expect("ACAO * is fine without credentials");
    // use-credentials: the jar is on, `*` is not enough, exact origin + ACAC is.
    let e = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::UseCredentials).err().unwrap();
    assert!(matches!(e, MediaNetError::Cors(_)), "{e:?}");
    {
        let mut c = w.cfg.lock().unwrap();
        c.acao = Some("https://page.example".into());
        c.acac = true;
    }
    qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::UseCredentials).expect("exact ACAO + ACAC");
    let r = w.seen.lock().unwrap().last().unwrap().clone();
    assert!(qf14b_has(&r, "X-Wire-Jar: scratchy") && qf14b_has(&r, "Origin: https://page.example"), "{r}");
}

#[test]
fn qm1e_mixed_content_and_bad_ports_are_blocked_before_any_request() {
    let w = qm1e_wire(Qm1eCfg { file: qm1e_file(100), ..Default::default() });
    let e = qm1e_open(&w, "http://media.example/v.mp4", QM1E_PAGE, MediaMode::NoCors).err().unwrap();
    assert!(matches!(e, MediaNetError::Blocked(_)), "{e:?}");
    let e = qm1e_open(&w, "https://media.example:25/v.mp4", QM1E_PAGE, MediaMode::NoCors).err().unwrap();
    assert!(matches!(e, MediaNetError::Blocked(_)), "{e:?}");
    let e = qm1e_open(&w, "ftp://media.example/v.mp4", QM1E_PAGE, MediaMode::NoCors).err().unwrap();
    assert!(matches!(e, MediaNetError::Blocked(_)), "{e:?}");
    assert!(w.seen.lock().unwrap().is_empty(), "nothing reached wire");
}

#[test]
fn qm1e_chunks_grow_from_1_to_4_mib_and_shrink_back_after_a_seek() {
    let file = qm1e_file(12 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg { file: file.clone(), ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert_eq!(s.last_chunk_len, CHUNK_SMALL);
    qm1e_read_all(&mut s, CHUNK_SMALL as usize + 1); // second chunk fetched
    assert_eq!(s.last_chunk_len, CHUNK_SMALL);
    qm1e_read_all(&mut s, CHUNK_SMALL as usize); // third contiguous miss
    assert_eq!(s.last_chunk_len, CHUNK_LARGE, "4 MiB after two contiguous fetches");
    let r = w.seen.lock().unwrap().last().unwrap().clone();
    assert!(r.contains(&format!("Range: bytes={}-{}", 2 * CHUNK_SMALL, 2 * CHUNK_SMALL + CHUNK_LARGE - 1)), "{r}");
    // a seek far away -> back to 1 MiB
    s.seek(11 * 1024 * 1024).unwrap();
    let mut b = [0u8; 100];
    s.read(&mut b).unwrap();
    assert_eq!(s.last_chunk_len, CHUNK_SMALL);
    assert!(b == file[11 * 1024 * 1024..11 * 1024 * 1024 + 100]);
}

#[test]
fn qm1e_cache_hit_serves_without_a_request() {
    let file = qm1e_file(6 * 1024 * 1024);
    let w = qm1e_wire(Qm1eCfg { file: file.clone(), ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    assert_eq!(w.seen.lock().unwrap().len(), 1);
    // tail (moov-at-end), then back to 0: chunk 0 is still cached
    s.seek(file.len() as u64 - 4096).unwrap();
    assert!(qm1e_read_all(&mut s, 4096) == file[file.len() - 4096..]);
    assert_eq!(w.seen.lock().unwrap().len(), 2);
    s.seek(0).unwrap();
    assert!(qm1e_read_all(&mut s, 70000) == file[..70000]);
    s.seek(500_000).unwrap();
    assert!(qm1e_read_all(&mut s, 1000) == file[500_000..501_000]);
    assert_eq!(w.seen.lock().unwrap().len(), 2, "cache hits made no request");
}

#[test]
fn qm1e_cancel_stops_a_blocked_read_at_once() {
    let w = qm1e_wire(Qm1eCfg { file: qm1e_file(3 * 1024 * 1024), ..Default::default() });
    let mut s = qm1e_open(&w, QM1E_URL, QM1E_PAGE, MediaMode::NoCors).unwrap();
    w.cfg.lock().unwrap().hold = true;
    s.seek(2 * 1024 * 1024).unwrap();
    let h = s.cancel_handle();
    let t0 = std::time::Instant::now();
    let th = thread::spawn(move || {
        let r = s.read(&mut [0u8; 16]);
        (r, s)
    });
    thread::sleep(std::time::Duration::from_millis(300));
    h.cancel();
    let (r, mut s) = th.join().unwrap();
    assert_eq!(r, Err(MediaNetError::Cancelled));
    assert!(t0.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(s.read_cb(&mut [0u8; 16]), -1, "later reads fail without a request");
    assert_eq!(w.seen.lock().unwrap().len(), 2);
}

#[test]
fn qm1e_other_status_is_a_plain_error() {
    use std::os::unix::net::UnixListener;
    let path = std::env::temp_dir().join(format!("qm1e-{}-status.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path).unwrap();
    thread::spawn(move || {
        for st in l.incoming() {
            let Ok(mut st) = st else { return };
            let mut b = [0u8; 4096];
            let _ = Read::read(&mut st, &mut b);
            let _ = st.write_all(b"HTTP/1.1 404 Not Found\r\n\r\nnope");
        }
    });
    let e = MediaStream::open(path, qm1e_src(QM1E_URL, QM1E_PAGE, MediaMode::NoCors), RequestPolicy::default()).err().unwrap();
    assert_eq!(e, MediaNetError::Status(404));
}

// ---------- QM1f: HTMLMediaElement (media_el.rs state machine, fake backend) ----------

mod qm1f {
    use super::*;
    use crate::media_el::*;
    use crate::media_net::MediaMode;

    fn core() -> (MediaCore, FakeHandle) {
        let f = fake_new();
        (MediaCore::new(Box::new(FakeBackend(f.clone()))), f)
    }

    fn cand(u: &str, m: &str) -> Candidate {
        Candidate { url: u.into(), mime: m.into() }
    }

    fn load(c: &mut MediaCore, url: &str) {
        c.load(vec![cand(url, "")], true, MediaMode::NoCors, "https://page.test/".into());
    }

    /// Pumps and returns the event names (and settled promises) since the last call.
    fn drain(c: &mut MediaCore, now: f64) -> Vec<String> {
        c.pump(now, std::time::Duration::ZERO);
        flat(c)
    }

    fn flat(c: &mut MediaCore) -> Vec<String> {
        c.take_outs()
            .into_iter()
            .map(|o| match o {
                Out::Fire(n) => n.to_string(),
                Out::SourceError(i) => format!("source-error {i}"),
                Out::Resolve(t) => format!("resolve {t}"),
                Out::Reject(t, n) => format!("reject {t} {n}"),
                Out::Unavailable => "unavailable".into(),
                Out::BackendStarted => "started".into(),
            })
            .filter(|s| s != "started")
            .collect()
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// A muted element loaded and playing, past canplaythrough.
    fn playing() -> (MediaCore, FakeHandle) {
        let (mut c, f) = core();
        c.muted = true;
        load(&mut c, "https://cdn.test/v.mp4");
        f.meta(10.0, 640, 360);
        f.first_frame();
        drain(&mut c, 0.0);
        c.play(1);
        flat(&mut c);
        (c, f)
    }

    #[test]
    fn normal_play_event_order() {
        let (mut c, f) = core();
        c.muted = true;
        load(&mut c, "https://cdn.test/v.mp4");
        assert_eq!(flat(&mut c), strs(&["loadstart"]));
        assert_eq!((c.network_state, c.ready_state), (NETWORK_LOADING, HAVE_NOTHING));
        assert!(c.duration.is_nan());
        f.meta(10.0, 640, 360);
        assert_eq!(drain(&mut c, 0.0), strs(&["durationchange", "loadedmetadata"]));
        assert_eq!((c.ready_state, c.duration, c.video_width, c.video_height), (HAVE_METADATA, 10.0, 640, 360));
        c.play(1);
        assert_eq!(flat(&mut c), strs(&["play", "waiting"]));
        assert!(!c.paused);
        f.first_frame();
        assert_eq!(drain(&mut c, 0.0), strs(&["loadeddata", "canplay", "playing", "resolve 1", "canplaythrough"]));
        assert_eq!((c.ready_state, c.network_state), (HAVE_ENOUGH_DATA, NETWORK_IDLE));
        assert_eq!(f.borrow().calls.iter().filter(|s| s.starts_with("paused")).count(), 1);
    }

    #[test]
    fn timeupdate_is_throttled_to_4hz() {
        let (mut c, f) = playing();
        f.advance(0.1);
        assert_eq!(drain(&mut c, 1000.0), strs(&["timeupdate"]));
        f.advance(0.1);
        assert!(drain(&mut c, 1100.0).is_empty());
        f.advance(0.1);
        assert!(drain(&mut c, 1200.0).is_empty());
        f.advance(0.1);
        assert_eq!(drain(&mut c, 1260.0), strs(&["timeupdate"]));
        assert!((c.current_time - 0.4).abs() < 1e-9);
    }

    #[test]
    fn seek_fires_seeking_then_seeked() {
        let (mut c, f) = playing();
        c.set_current_time(5.0);
        assert!(c.seeking);
        assert_eq!(c.current_time, 5.0);
        assert_eq!(flat(&mut c), strs(&["seeking"]));
        assert_eq!(drain(&mut c, 0.0), strs(&["timeupdate", "seeked", "playing"]));
        assert!(!c.seeking);
        assert!(f.borrow().calls.contains(&"seek 5".to_string()));
        // clamped to the duration, negative to 0
        c.set_current_time(99.0);
        assert_eq!(c.current_time, 10.0);
        c.set_current_time(-3.0);
        assert_eq!(c.current_time, 0.0);
    }

    #[test]
    fn seek_before_metadata_is_applied_after_it() {
        let (mut c, f) = core();
        c.muted = true;
        load(&mut c, "https://cdn.test/v.mp4");
        flat(&mut c);
        c.set_current_time(3.0);
        assert_eq!(c.current_time, 3.0);
        assert!(flat(&mut c).is_empty());
        f.meta(10.0, 0, 0);
        let ev = drain(&mut c, 0.0);
        assert_eq!(ev, strs(&["durationchange", "loadedmetadata", "seeking"]));
        assert!(f.borrow().calls.contains(&"seek 3".to_string()));
    }

    #[test]
    fn end_of_media() {
        let (mut c, f) = playing();
        f.advance(20.0);
        assert_eq!(drain(&mut c, 5000.0), strs(&["timeupdate", "timeupdate", "pause", "ended"]));
        assert!(c.paused && c.ended());
        assert_eq!(c.current_time, 10.0);
        // play() after the end restarts from 0
        c.play(2);
        let ev = flat(&mut c);
        assert_eq!(ev, strs(&["seeking", "play", "playing", "resolve 2"]), "{ev:?}");
        assert_eq!(c.current_time, 0.0);
    }

    #[test]
    fn loop_seeks_to_start_instead_of_ending() {
        let (mut c, f) = playing();
        c.loop_ = true;
        f.advance(20.0);
        let ev = drain(&mut c, 5000.0);
        assert!(ev.contains(&"seeking".to_string()) && !ev.contains(&"ended".to_string()), "{ev:?}");
        assert!(!c.paused);
    }

    #[test]
    fn pause_events_and_promise() {
        let (mut c, _f) = core();
        c.muted = true;
        load(&mut c, "https://cdn.test/v.mp4");
        c.play(7);
        flat(&mut c);
        c.pause();
        assert_eq!(flat(&mut c), strs(&["timeupdate", "pause", "reject 7 AbortError"]));
        assert!(c.paused);
        c.pause(); // already paused: nothing
        assert!(flat(&mut c).is_empty());
    }

    #[test]
    fn volume_muted_rate_events() {
        let (mut c, f) = core();
        c.set_volume(0.5).unwrap();
        c.set_volume(0.5).unwrap();
        c.set_muted(true);
        c.set_rate(2.0).unwrap();
        assert_eq!(flat(&mut c), strs(&["volumechange", "volumechange", "ratechange"]));
        assert_eq!(c.set_volume(1.5), Err("IndexSizeError"));
        assert_eq!(c.set_volume(f64::NAN), Err("IndexSizeError"));
        assert_eq!(c.set_rate(0.0), Err("NotSupportedError"));
        assert_eq!(c.rate, 2.0);
        let calls = f.borrow().calls.clone();
        assert!(calls.contains(&"volume 0.5".to_string()) && calls.contains(&"muted true".to_string()) && calls.contains(&"rate 2".to_string()), "{calls:?}");
    }

    #[test]
    fn waiting_and_resume_on_buffering() {
        let (mut c, f) = playing();
        f.push(BackendEvent::Buffering(true));
        assert_eq!(drain(&mut c, 0.0), strs(&["waiting"]));
        assert_eq!(c.ready_state, HAVE_CURRENT_DATA);
        f.push(BackendEvent::Buffering(false));
        assert_eq!(drain(&mut c, 0.0), strs(&["canplay", "playing"]));
        assert_eq!(c.ready_state, HAVE_ENOUGH_DATA);
    }

    #[test]
    fn error_before_metadata_is_src_not_supported() {
        let (mut c, f) = core();
        load(&mut c, "https://cdn.test/v.bin");
        flat(&mut c);
        f.push(BackendEvent::Error(MEDIA_ERR_SRC_NOT_SUPPORTED, "no".into()));
        assert_eq!(drain(&mut c, 0.0), strs(&["error"]));
        assert_eq!(c.error.as_ref().map(|e| e.0), Some(4));
        assert_eq!(c.network_state, NETWORK_NO_SOURCE);
    }

    #[test]
    fn network_error_after_metadata() {
        let (mut c, f) = playing();
        f.push(BackendEvent::Error(MEDIA_ERR_NETWORK, "gone".into()));
        assert_eq!(drain(&mut c, 0.0), strs(&["error"]));
        assert_eq!(c.error, Some((2, "gone".into())));
        assert_eq!(c.network_state, NETWORK_IDLE);
    }

    #[test]
    fn reload_fires_abort_emptied_and_resets() {
        let (mut c, f) = playing();
        load(&mut c, "https://cdn.test/other.mp4");
        assert_eq!(flat(&mut c), strs(&["abort", "emptied", "loadstart"]));
        assert!(c.paused && c.duration.is_nan() && c.current_time == 0.0 && c.ready_state == HAVE_NOTHING);
        assert!(f.borrow().calls.contains(&"unload".to_string()));
    }

    #[test]
    fn source_selection_skips_unplayable_types_and_failed_loads() {
        let (mut c, f) = core();
        c.load(
            vec![cand("https://cdn.test/a.avi", "video/x-msvideo"), cand("https://cdn.test/b.mp4", "video/mp4; codecs=\"avc1.42E01E\""), cand("https://cdn.test/c.webm", "video/webm")],
            false,
            MediaMode::NoCors,
            "https://page.test/".into(),
        );
        assert_eq!(flat(&mut c), strs(&["loadstart", "source-error 0"]));
        assert_eq!(c.current_src, "https://cdn.test/b.mp4");
        f.push(BackendEvent::Error(MEDIA_ERR_SRC_NOT_SUPPORTED, "bad".into()));
        assert_eq!(drain(&mut c, 0.0), strs(&["source-error 1"]));
        assert_eq!(c.current_src, "https://cdn.test/c.webm");
        f.push(BackendEvent::Error(MEDIA_ERR_SRC_NOT_SUPPORTED, "bad".into()));
        assert_eq!(drain(&mut c, 0.0), strs(&["source-error 2"]));
        assert_eq!(c.network_state, NETWORK_NO_SOURCE);
        assert!(c.error.is_none(), "source failures are not a MediaError");
    }

    #[test]
    fn no_candidates_stays_empty() {
        let (mut c, _f) = core();
        c.load(vec![], false, MediaMode::NoCors, "https://page.test/".into());
        assert!(flat(&mut c).is_empty());
        assert_eq!(c.network_state, NETWORK_EMPTY);
    }

    #[test]
    fn autoplay_follows_d5() {
        // unmuted, no gesture: stays paused, no play event
        let (mut c, f) = core();
        c.autoplay = true;
        load(&mut c, "https://cdn.test/v.mp4");
        f.meta(10.0, 0, 0);
        f.first_frame();
        let ev = drain(&mut c, 0.0);
        assert!(!ev.contains(&"play".to_string()) && c.paused, "{ev:?}");
        // muted: autoplays
        let (mut c, f) = core();
        c.autoplay = true;
        c.muted = true;
        load(&mut c, "https://cdn.test/v.mp4");
        f.meta(10.0, 0, 0);
        f.first_frame();
        let ev = drain(&mut c, 0.0);
        assert!(ev.contains(&"play".to_string()) && ev.contains(&"playing".to_string()) && !c.paused, "{ev:?}");
        // unmuted but after a user gesture: autoplays
        let (mut c, f) = core();
        c.autoplay = true;
        c.activated = true;
        load(&mut c, "https://cdn.test/v.mp4");
        f.meta(10.0, 0, 0);
        f.first_frame();
        drain(&mut c, 0.0);
        assert!(!c.paused);
    }

    #[test]
    fn play_without_gesture_is_rejected_when_unmuted() {
        let (mut c, f) = core();
        load(&mut c, "https://cdn.test/v.mp4");
        f.meta(10.0, 0, 0);
        f.first_frame();
        drain(&mut c, 0.0);
        c.play(3);
        assert_eq!(flat(&mut c), strs(&["reject 3 NotAllowedError"]));
        assert!(c.paused);
        c.activated = true;
        c.play(4);
        assert_eq!(flat(&mut c), strs(&["play", "playing", "resolve 4"]));
    }

    #[test]
    fn can_play_type_table() {
        for (t, want) in [
            ("video/mp4", "maybe"),
            ("video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\"", "probably"),
            ("video/webm; codecs=\"vp8, vorbis\"", "probably"),
            ("video/webm; codecs=vp9", "probably"),
            ("audio/webm; codecs=opus", "probably"),
            ("audio/ogg; codecs=\"vorbis\"", "probably"),
            ("audio/mpeg", "maybe"),
            ("audio/flac", "maybe"),
            ("audio/wav", "maybe"),
            ("audio/wav; codecs=1", "probably"),
            ("video/mp4; codecs=\"hev9.0\"", ""),
            ("video/webm; codecs=\"avc1\"", ""),
            ("video/x-msvideo", ""),
            ("application/x-mpegURL", ""),
            ("", ""),
        ] {
            assert_eq!(can_play_type(t), want, "{t}");
        }
    }

    #[test]
    fn qm1k4_mse_is_type_supported_table() {
        for (t, want) in [
            // valid
            ("video/mp4", true),
            ("audio/mp4", true),
            ("video/mp4; codecs=\"avc1.640028\"", true),
            ("video/mp4; codecs=\"avc1.42001E, mp4a.40.2\"", true),
            ("video/mp4; codecs=\"avc3.4D401F\"", true),
            ("audio/mp4; codecs=\"mp4a.40.2\"", true),
            ("audio/mp4; codecs=\"mp4a.40.5\"", true),
            ("audio/mp4; codecs=\"mp4a.40.29\"", true),
            // valid twins of YouTube's probes
            ("video/mp4; codecs=\"avc1.640028\"; width=1920", true),
            ("video/mp4; codecs=\"avc1.640028\"; height=1080", true),
            ("video/mp4; codecs=\"avc1.640028\"; framerate=60", true),
            ("video/mp4; codecs=\"avc1.640028\"; bitrate=2000000", true),
            ("video/mp4; codecs=\"avc1.640028\"; eotf=bt709", true),
            ("video/mp4; codecs=\"avc1.640028\"; eotf=smpte2084", true),
            ("video/mp4; codecs=\"avc1.640028\"; cryptoblockformat=subsample", true),
            ("video/mp4; codecs=\"avc1.640028\"; decode-to-texture=true", true),
            ("video/mp4; codecs=\"avc1.640028\"; experimental=allowed", true),
            ("video/mp4; codecs=\"avc1.640028\"; tunnelmode=true", true),
            ("audio/mp4; codecs=\"mp4a.40.2\"; channels=2", true),
            ("audio/mp4; codecs=\"mp4a.40.2\"; channels=5.1", true),
            ("VIDEO/MP4 ; Codecs = \"avc1.640028\" ; unknownparam=whatever", true),
            // invalid probes
            ("video/mp4; codecs=\"avc1.640028\"; width=99999", false),
            ("video/mp4; codecs=\"avc1.640028\"; height=99999", false),
            ("video/mp4; codecs=\"avc1.640028\"; width=0", false),
            ("video/mp4; codecs=\"avc1.640028\"; width=abc", false),
            ("video/mp4; codecs=\"avc1.640028\"; framerate=9999", false),
            ("video/mp4; codecs=\"avc1.640028\"; bitrate=2000000000", false),
            ("video/mp4; codecs=\"avc1.640028\"; eotf=catavision", false),
            ("video/mp4; codecs=\"avc1.640028\"; cryptoblockformat=invalidformat", false),
            ("video/mp4; codecs=\"avc1.640028\"; decode-to-texture=nope", false),
            ("video/mp4; codecs=\"avc1.640028\"; experimental=invalid", false),
            ("video/mp4; codecs=\"avc1.640028\"; tunnelmode=nope", false),
            ("audio/mp4; codecs=\"mp4a.40.2\"; channels=99", false),
            ("audio/mp4; codecs=\"mp4a.40.2\"; channels=0", false),
            ("video/mp4; codecs=\"av99.0.0\"", false),
            ("video/mp4; codecs=\"avc1.640028, av99\"", false),
            ("video/mp4; codecs=\"avc1.640028,\"", false),
            ("video/mp4; codecs=\"mp4a.40.3\"", false),
            ("video/mp4; codecs=\"avc10\"", false),
            // listed but switched off
            ("video/mp4; codecs=\"vp09.00.10.08\"", false),
            ("video/mp4; codecs=\"av01.0.05M.08\"", false),
            ("audio/mp4; codecs=\"opus\"", false),
            ("audio/mp4; codecs=\"flac\"", false),
            ("video/webm", false),
            ("video/webm; codecs=\"vp9\"", false),
            ("video/webm; codecs=\"vp09.00.10.08\"", false),
            ("video/webm; codecs=\"vp8\"", false),
            ("audio/webm; codecs=\"opus\"", false),
            ("audio/webm; codecs=\"vorbis\"", false),
            // not MSE at all
            ("audio/mpeg", false),
            ("application/x-mpegURL", false),
            ("", false),
        ] {
            assert_eq!(mse_type_supported(t), want, "{t}");
        }
    }

    #[test]
    fn qm1k4_mse_codec_table_is_one_list_with_webm_off() {
        assert!(MSE_CODECS.iter().any(|(m, c, on)| *m == "video/webm" && *c == "vp9" && !on));
        assert!(MSE_CODECS.iter().any(|(m, c, on)| *m == "video/mp4" && *c == "av01" && !on));
        assert!(MSE_CODECS.iter().filter(|(_, _, on)| *on).all(|(m, _, _)| m.ends_with("/mp4")));
    }

    #[test]
    fn qm1k4_can_play_type_rejects_invalid_probe_params() {
        let base = "video/mp4; codecs=\"avc1.640028\"";
        for (suffix, want) in [
            ("", "probably"),
            ("; width=1920", "probably"),
            ("; framerate=60", "probably"),
            ("; eotf=bt709", "probably"),
            ("; unknown=x", "probably"),
            ("; width=99999", ""),
            ("; height=99999", ""),
            ("; framerate=9999", ""),
            ("; bitrate=2000000000", ""),
            ("; eotf=catavision", ""),
            ("; cryptoblockformat=invalidformat", ""),
            ("; decode-to-texture=nope", ""),
            ("; experimental=invalid", ""),
        ] {
            assert_eq!(can_play_type(&format!("{base}{suffix}")), want, "{suffix}");
        }
        assert_eq!(can_play_type("audio/mp4; codecs=\"mp4a.40.2\"; channels=99"), "");
        assert_eq!(can_play_type("audio/mp4; codecs=\"mp4a.40.2\"; channels=2"), "probably");
        assert_eq!(can_play_type("video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\"; bad=1; width=99999"), "");
    }

    #[test]
    fn placeholder_backend_fails_loudly_but_harmlessly() {
        let mut c = MediaCore::new(Box::new(UnavailableBackend));
        c.muted = true;
        load(&mut c, "https://cdn.test/v.mp4");
        assert_eq!(flat(&mut c), strs(&["loadstart", "unavailable", "error"]));
        assert_eq!(c.network_state, NETWORK_NO_SOURCE);
        assert_eq!(c.error.as_ref().map(|e| e.0), Some(MEDIA_ERR_SRC_NOT_SUPPORTED));
        c.play(9);
        assert_eq!(flat(&mut c), strs(&["unavailable", "reject 9 NotSupportedError"]));
        assert!(c.paused);
        assert!(!c.available());
    }

    fn host_with_fakes() -> (MediaHost, Rc<RefCell<Vec<FakeHandle>>>) {
        let made: Rc<RefCell<Vec<FakeHandle>>> = Default::default();
        let m = made.clone();
        set_backend_factory(Some(Rc::new(move |_| {
            let f = fake_new();
            m.borrow_mut().push(f.clone());
            Box::new(FakeBackend(f))
        })));
        let h = MediaHost::new(BackendCtx { socket: None, policy: Default::default() }, Rc::new(|_, _| {}));
        set_backend_factory(None);
        (h, made)
    }

    #[test]
    fn at_most_two_players_oldest_is_paused() {
        let (mut h, made) = host_with_fakes();
        h.activated = true;
        for id in 1..=3 {
            h.with(id, |c| c.load(vec![cand("https://cdn.test/v.mp4", "")], true, MediaMode::NoCors, "https://page.test/".into()));
            let f = made.borrow()[(id - 1) as usize].clone();
            f.meta(10.0, 0, 0);
            f.first_frame();
        }
        h.pump(0.0, std::time::Duration::ZERO);
        let mut paused_events = Vec::new();
        for id in 1..=3 {
            let (_, outs) = h.with(id, |c| c.play(id as u32));
            paused_events.extend(outs.into_iter().filter(|(_, o)| *o == Out::Fire("pause")));
        }
        assert_eq!(paused_events, vec![(1, Out::Fire("pause"))]);
        assert!(h.peek(1).unwrap().paused && !h.peek(2).unwrap().paused && !h.peek(3).unwrap().paused);
    }

    #[test]
    fn host_logs_placeholder_line_once() {
        set_backend_factory(Some(Rc::new(|_| Box::new(UnavailableBackend))));
        let logs: Rc<RefCell<Vec<String>>> = Default::default();
        let l = logs.clone();
        let mut h = MediaHost::new(BackendCtx { socket: None, policy: Default::default() }, Rc::new(move |lv, t| l.borrow_mut().push(format!("{lv}:{t}"))));
        set_backend_factory(None);
        assert!(!h.backend_available());
        for id in 1..=2 {
            h.with(id, |c| c.load(vec![cand("https://cdn.test/v.mp4", "")], true, MediaMode::NoCors, "https://page.test/".into()));
            h.with(id, |c| c.play(1));
        }
        assert_eq!(*logs.borrow(), vec!["error:scratchy: js: not implemented: media (libmpv not available)".to_string()]);
    }

    // ---- through the JS element wrapper ----

    struct Js {
        engine: JsEngine,
        clock: Rc<std::cell::Cell<f64>>,
        log: Rc<RefCell<Vec<String>>>,
        fakes: Rc<RefCell<Vec<FakeHandle>>>,
    }

    impl Js {
        fn new(html: &str, code: &str, placeholder: bool) -> Js {
            let fakes: Rc<RefCell<Vec<FakeHandle>>> = Default::default();
            let m = fakes.clone();
            if placeholder {
                set_backend_factory(Some(Rc::new(|_| Box::new(UnavailableBackend))));
            } else {
                set_backend_factory(Some(Rc::new(move |_| {
                    let f = fake_new();
                    m.borrow_mut().push(f.clone());
                    Box::new(FakeBackend(f))
                })));
            }
            set_page_url(Some(&Url::parse("https://page.test/").unwrap()));
            let log = Rc::new(RefCell::new(Vec::new()));
            let l = log.clone();
            let clock = Rc::new(std::cell::Cell::new(0.0));
            let engine = crate::js::start_scripts_clocked(&parse_html(html), &[src(code)], Rc::new(move |lv, t| l.borrow_mut().push(format!("{lv}:{t}"))), Some(clock.clone())).unwrap();
            set_backend_factory(None);
            Js { engine, clock, log, fakes }
        }

        /// Advances the page clock by `ms`, running due timers on the way.
        fn step(&self, ms: f64) {
            let end = self.clock.get() + ms;
            for _ in 0..2000 {
                match self.engine.next_due_ms() {
                    Some(d) if d <= end => {
                        self.clock.set(self.clock.get().max(d));
                        self.engine.run_due();
                    }
                    _ => break,
                }
            }
            self.clock.set(end);
        }

        fn lines(&self) -> Vec<String> {
            self.log.borrow().clone()
        }
    }

    #[test]
    fn js_events_in_order_and_promise_resolves() {
        let js = Js::new(
            "<video id=v src='/a.mp4' muted></video>",
            "var v = document.getElementById('v');\
             ['loadstart','durationchange','loadedmetadata','loadeddata','canplay','canplaythrough','play','playing','pause','seeking','seeked','ended','timeupdate','waiting']\
               .forEach(function (n) { v.addEventListener(n, function () { console.log('ev', n); }); });\
             console.log('init', v.readyState, v.networkState, v.paused, v.duration, v.muted);\
             v.play().then(function () { console.log('play resolved'); });",
            false,
        );
        js.step(100.0);
        let f = js.fakes.borrow()[0].clone();
        assert_eq!(f.borrow().loaded.as_ref().map(|r| r.url.clone()).as_deref(), Some("https://page.test/a.mp4"));
        f.meta(2.0, 320, 240);
        js.step(100.0);
        f.first_frame();
        js.step(100.0);
        f.advance(1.0);
        js.step(300.0);
        f.advance(5.0);
        js.step(100.0);
        let l = js.lines();
        assert_eq!(l[0], "log:init 0 0 true NaN true", "{l:?}");
        let names: Vec<&str> = l[1..].iter().map(|s| s.as_str()).collect();
        let want = [
            "log:ev loadstart", "log:ev play", "log:ev waiting", "log:ev durationchange", "log:ev loadedmetadata", "log:ev loadeddata", "log:ev canplay",
            "log:ev playing", "log:ev canplaythrough", "log:play resolved",
        ];
        assert_eq!(&names[..want.len()], &want, "{names:?}");
        let tail = &names[want.len()..];
        assert!(tail.iter().filter(|s| **s == "log:ev timeupdate").count() >= 2, "{tail:?}");
        assert_eq!(&tail[tail.len() - 2..], &["log:ev pause", "log:ev ended"], "{tail:?}");
        let r = js.engine.eval_value::<String>("t", "var v = document.getElementById('v'); [v.ended, v.paused, v.currentTime, v.duration, v.videoWidth, v.videoHeight, v.readyState].join()").unwrap();
        assert_eq!(r, "true,true,2,2,320,240,4");
    }

    #[test]
    fn js_current_time_set_and_state_getters() {
        let js = Js::new(
            "<video id=v src='/a.mp4' muted></video>",
            "var v = document.getElementById('v'); v.addEventListener('seeking', function(){console.log('seeking', v.seeking, v.currentTime)});\
             v.addEventListener('seeked', function(){console.log('seeked', v.seeking, v.currentTime)});",
            false,
        );
        js.step(50.0);
        let f = js.fakes.borrow()[0].clone();
        f.meta(8.0, 0, 0);
        f.first_frame();
        js.step(50.0);
        f.borrow_mut().auto_restart = false;
        js.engine.eval("t", "document.getElementById('v').currentTime = 4.5; try { document.getElementById('v').volume = 3 } catch (e) { console.log(e.name) }");
        js.step(100.0);
        assert!(js.lines().contains(&"log:IndexSizeError".to_string()));
        assert!(js.lines().contains(&"log:seeking true 4.5".to_string()), "{:?}", js.lines());
        f.first_frame();
        js.step(100.0);
        assert!(js.lines().contains(&"log:seeked false 4.5".to_string()), "{:?}", js.lines());
        assert!(f.borrow().calls.contains(&"seek 4.5".to_string()));
    }

    #[test]
    fn js_source_children_select_by_type() {
        let js = Js::new(
            "<video id=v muted><source src='/a.avi' type='video/x-msvideo'><source src='/b.webm' type='video/webm'></video>",
            "var v = document.getElementById('v'); var s = v.querySelectorAll('source');\
             s[0].addEventListener('error', function () { console.log('source 0 error'); });",
            false,
        );
        js.step(50.0);
        assert!(js.lines().contains(&"log:source 0 error".to_string()), "{:?}", js.lines());
        let f = js.fakes.borrow()[0].clone();
        assert_eq!(f.borrow().loaded.as_ref().map(|r| r.url.clone()).as_deref(), Some("https://page.test/b.webm"));
        let r = js.engine.eval_value::<String>("t", "document.getElementById('v').currentSrc").unwrap();
        assert_eq!(r, "https://page.test/b.webm");
    }

    #[test]
    fn js_play_without_gesture_unmuted_rejects_not_allowed() {
        let js = Js::new(
            "<video id=v src='/a.mp4'></video>",
            "var v = document.getElementById('v'); v.play().then(function(){console.log('ok')}, function(e){console.log('rejected', e.name)});",
            false,
        );
        js.step(50.0);
        let f = js.fakes.borrow()[0].clone();
        f.meta(8.0, 0, 0);
        f.first_frame();
        js.step(50.0);
        assert!(js.lines().contains(&"log:rejected NotAllowedError".to_string()), "{:?}", js.lines());
        // after a user gesture the same call works
        js.engine.eval("t", "__media_activate(); document.getElementById('v').play().then(function(){console.log('ok')});");
        js.step(50.0);
        assert!(js.lines().contains(&"log:ok".to_string()), "{:?}", js.lines());
    }

    #[test]
    fn js_placeholder_path() {
        let js = Js::new(
            "<video id=v src='/a.mp4'></video><audio id=a></audio>",
            "var v = document.getElementById('v');\
             v.addEventListener('error', function () { console.log('error', v.error.code, v.networkState); });\
             console.log('cpt', JSON.stringify(v.canPlayType('video/mp4')));\
             v.play().catch(function (e) { console.log('rejected', e.name); });\
             document.getElementById('a').play().catch(function (e) { console.log('rejected2', e.name); });\
             console.log('after');",
            true,
        );
        js.step(100.0);
        let l = js.lines();
        assert!(l.contains(&"log:cpt \"\"".to_string()), "{l:?}");
        assert!(l.contains(&"log:error 4 3".to_string()), "{l:?}");
        assert!(l.contains(&"log:rejected NotSupportedError".to_string()) && l.contains(&"log:rejected2 NotSupportedError".to_string()), "{l:?}");
        assert_eq!(l.iter().filter(|s| s.contains("not implemented: media (libmpv not available)")).count(), 1, "{l:?}");
        assert!(l.contains(&"log:after".to_string()));
    }

    #[test]
    fn js_audio_constructor_and_timeranges() {
        let js = Js::new(
            "",
            "var a = new Audio('/x.mp3'); console.log(a instanceof HTMLAudioElement, a.preload, a.src);\
             var r = a.buffered; console.log(r.length, typeof r.start); try { r.start(0) } catch (e) { console.log(e.name) }\
             console.log(HTMLMediaElement.HAVE_ENOUGH_DATA, a.HAVE_METADATA, typeof MediaError, MediaError.MEDIA_ERR_DECODE);",
            false,
        );
        js.step(50.0);
        assert_eq!(js.lines(), strs(&["log:true auto https://page.test/x.mp3", "log:0 function", "log:IndexSizeError", "log:4 1 function 3"]));
    }
}

// QM1f real backend (cargo feature `media`): SKIPS when libmpv or ffmpeg is missing.
#[cfg(feature = "media")]
mod qm1f_real {
    use super::*;
    use crate::media_el::*;
    use crate::media_net::MediaMode;

    fn make_clip(dir: &std::path::Path) -> Option<Vec<u8>> {
        let p = dir.join("c.mp4");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25", "-t", "2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .arg(&p)
            .status()
            .ok()?
            .success();
        if !ok {
            return None;
        }
        std::fs::read(&p).ok()
    }

    fn run(c: &mut MediaCore, now: &mut f64, names: &mut Vec<String>, until: impl Fn(&MediaCore) -> bool) {
        for _ in 0..400 {
            *now += 20.0;
            c.pump(*now, std::time::Duration::from_millis(20));
            for o in c.take_outs() {
                if let Out::Fire(n) = o {
                    names.push(n.to_string());
                }
            }
            if until(c) {
                return;
            }
        }
        panic!("timed out; events so far: {names:?}");
    }

    #[test]
    fn mpv_plays_seeks_and_ends_through_the_wire_channel() {
        if !crate::media_mpv::libmpv_available() {
            eprintln!("SKIP: libmpv.so.2 not available");
            return;
        }
        let dir = std::env::temp_dir().join(format!("qm1f-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let Some(file) = make_clip(&dir) else {
            eprintln!("SKIP: ffmpeg could not make a test clip");
            return;
        };
        let w = qm1e_wire(Qm1eCfg { file, ..Default::default() });
        let backend = crate::media_mpv::MpvBackend::new(BackendCtx { socket: Some(w.path.clone()), policy: Default::default() });
        let mut c = MediaCore::new(Box::new(backend));
        c.muted = true;
        c.load(vec![Candidate { url: "https://cdn.test/c.mp4".into(), mime: String::new() }], true, MediaMode::NoCors, "https://cdn.test/".into());
        let (mut now, mut names) = (0.0, Vec::new());
        run(&mut c, &mut now, &mut names, |c| c.ready_state >= HAVE_ENOUGH_DATA);
        assert!((c.duration - 2.0).abs() < 0.2, "duration {}", c.duration);
        assert_eq!((c.video_width, c.video_height), (320, 240));
        assert_eq!(&names[..4], ["loadstart", "durationchange", "loadedmetadata", "loadeddata"], "{names:?}");
        c.play(1);
        run(&mut c, &mut now, &mut names, |c| c.current_time > 0.3);
        c.set_current_time(1.5);
        run(&mut c, &mut now, &mut names, |c| !c.seeking);
        assert!(names.contains(&"seeking".to_string()) && names.contains(&"seeked".to_string()), "{names:?}");
        run(&mut c, &mut now, &mut names, |c| c.ended());
        assert!(names.contains(&"ended".to_string()) && c.paused, "{names:?}");
        // every byte came through the fake wire (D4): ranged requests only
        let seen = w.seen.lock().unwrap().clone();
        assert!(!seen.is_empty() && seen.iter().all(|r| r.to_ascii_lowercase().contains("range")), "{seen:?}");
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mpv_without_wire_is_a_media_error_not_a_hang() {
        if !crate::media_mpv::libmpv_available() {
            eprintln!("SKIP: libmpv.so.2 not available");
            return;
        }
        // no socket at all: nothing can be fetched, the element gets MEDIA_ERR_SRC_NOT_SUPPORTED
        let backend = crate::media_mpv::MpvBackend::new(BackendCtx { socket: None, policy: Default::default() });
        let mut c = MediaCore::new(Box::new(backend));
        c.load(vec![Candidate { url: "https://cdn.test/x.mp4".into(), mime: String::new() }], true, MediaMode::NoCors, "https://cdn.test/".into());
        let (mut now, mut names) = (0.0, Vec::new());
        run(&mut c, &mut now, &mut names, |c| c.error.is_some());
        assert_eq!(c.error.as_ref().map(|e| e.0), Some(4));
        assert_eq!(c.network_state, NETWORK_NO_SOURCE);
    }
}
