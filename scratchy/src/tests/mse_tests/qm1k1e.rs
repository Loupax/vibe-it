// QM1k1e: JS SourceBuffer (src/mse_prelude.js + __mse_append/abort/remove/buffered natives).
use super::qm1k1d::{attached, Js};
use super::super::*;
use super::{init_seg, seg4, TS};

fn arr(b: &[u8]) -> String {
    format!("[{}]", b.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","))
}

/// An attached element with `ms`, a video buffer `sb` and the page-level helpers
/// `INIT`, `S0`, `S1` (Uint8Arrays) and an `ev(target, names...)` logger.
fn with_buffer() -> Js {
    let js = attached();
    js.run(&format!(
        "var INIT = new Uint8Array({}), S0 = new Uint8Array({}), S1 = new Uint8Array({});\
         var sb = ms.addSourceBuffer('video/mp4; codecs=\"avc1.64001f\"');\
         var evs = function (t, tag) {{ ['updatestart', 'update', 'updateend', 'error', 'abort'].forEach(function (n) {{ t.addEventListener(n, function () {{ console.log(tag + n + ' ' + t.updating); }}); }}); }};\
         evs(sb, '');",
        arr(&init_seg(1, TS, 3600)),
        arr(&seg4(0, 40)),
        arr(&seg4(4, 40)),
    ));
    js.log.borrow_mut().clear();
    js
}

#[test]
fn qm1k1e_two_appends_full_event_order_and_updating_flag() {
    let js = with_buffer();
    js.run("sb.appendBuffer(INIT); console.log('sync ' + sb.updating);");
    assert_eq!(js.lines(), ["log:sync true", "log:updatestart true", "log:update false", "log:updateend false"]);
    js.log.borrow_mut().clear();
    js.run("sb.appendBuffer(S0); console.log('sync ' + sb.updating);");
    assert_eq!(js.lines(), ["log:sync true", "log:updatestart true", "log:update false", "log:updateend false"]);
    assert_eq!(js.val("sb.updating"), "false");
    // the bytes were parsed: init + one 4 s segment
    assert_eq!(js.val("[sb.buffered.length, sb.buffered.start(0), sb.buffered.end(0)].join()"), "1,0,4");
}

#[test]
fn qm1k1e_state_errors_while_updating_and_after_removal() {
    let js = with_buffer();
    js.run("sb.appendBuffer(INIT);");
    js.run("sb.appendBuffer(S0);");
    js.run("sb.appendBuffer(S0); globalThis.r = [errName(function () { sb.appendBuffer(S1) }), errName(function () { sb.remove(0, 1) }), errName(function () { ms.duration = 5 }), errName(function () { ms.endOfStream() }), errName(function () { sb.timestampOffset = 1 }), errName(function () { sb.mode = 'segments' }), errName(function () { sb.changeType('video/mp4') }), sb.updating].join();");
    assert_eq!(js.val("r"), "InvalidStateError,InvalidStateError,InvalidStateError,InvalidStateError,InvalidStateError,InvalidStateError,InvalidStateError,true");
    js.step(50.0);
    // removed buffer: everything but the plain getters throws InvalidStateError
    js.run("ms.removeSourceBuffer(sb);");
    assert_eq!(
        js.val("[errName(function () { sb.appendBuffer(S1) }), errName(function () { sb.remove(0, 1) }), errName(function () { sb.abort() }), errName(function () { sb.buffered }), errName(function () { sb.timestampOffset = 1 }), sb.updating].join()"),
        "InvalidStateError,InvalidStateError,InvalidStateError,InvalidStateError,InvalidStateError,false"
    );
    // argument errors
    let js = with_buffer();
    assert_eq!(js.val("[errName(function () { sb.appendBuffer() }), errName(function () { sb.appendBuffer('x') }), errName(function () { sb.appendBuffer({}) }), errName(function () { sb.remove(1) }), sb.updating].join()"), "TypeError,TypeError,TypeError,TypeError,false");
}

#[test]
fn qm1k1e_buffer_source_kinds_append_the_right_bytes() {
    let js = with_buffer();
    // ArrayBuffer, DataView, a subarray with a non-zero byteOffset (padding before and after).
    js.run(
        "var pad = function (u8) { var big = new Uint8Array(u8.length + 11); big.fill(0xee); big.set(u8, 5); return big; };\
         sb.appendBuffer(INIT.buffer.slice(INIT.byteOffset, INIT.byteOffset + INIT.byteLength));",
    );
    js.run("var bigS0 = pad(S0); sb.appendBuffer(new DataView(bigS0.buffer, 5, S0.length));");
    js.run("var bigS1 = pad(S1); var sub = bigS1.subarray(5, 5 + S1.length); console.log('off ' + sub.byteOffset); sb.appendBuffer(sub);");
    js.run("");
    assert!(js.lines().contains(&"log:off 5".to_string()), "{:?}", js.lines());
    assert_eq!(js.val("[sb.buffered.length, sb.buffered.start(0), sb.buffered.end(0)].join()"), "1,0,8");
    let feed = js.engine.media.as_ref().unwrap().borrow().mse.only().unwrap();
    let (init, segs) = feed.with_store(|st| (st.buffers[0].init_bytes().map(|b| b.to_vec()), st.buffers[0].segments().iter().map(|s| (*s.data).clone()).collect::<Vec<_>>()));
    assert_eq!(init.unwrap(), init_seg(1, TS, 3600));
    assert_eq!(segs, vec![seg4(0, 40), seg4(4, 40)]);
}

#[test]
fn qm1k1e_a_box_split_across_appends_is_reassembled() {
    let js = with_buffer();
    js.run("sb.appendBuffer(INIT);");
    js.run("sb.appendBuffer(S0.subarray(0, 20));");
    assert_eq!(js.val("sb.buffered.length"), "0");
    js.run("sb.appendBuffer(S0.subarray(20));");
    assert_eq!(js.val("sb.buffered.end(0)"), "4");
}

#[test]
fn qm1k1e_quota_exceeded_at_a_lowered_cap_throws_synchronously() {
    let js = with_buffer();
    let feed = js.engine.media.as_ref().unwrap().borrow().mse.only().unwrap();
    let init_len = init_seg(1, TS, 3600).len();
    feed.with_store(|st| st.set_max_bytes(init_len + seg4(0, 40).len() + 10));
    js.run("sb.appendBuffer(INIT);");
    js.run("sb.appendBuffer(S0);");
    assert_eq!(js.val("sb.buffered.length"), "1");
    js.log.borrow_mut().clear();
    // nothing is evictable (inside keep-ahead of currentTime 0): sync QuotaExceededError, not updating, no events
    assert_eq!(js.val("errName(function () { sb.appendBuffer(S1) }) + ' ' + sb.updating"), "QuotaExceededError false");
    js.step(50.0);
    assert!(js.lines().is_empty(), "{:?}", js.lines());
    assert_eq!(js.val("sb.buffered.end(0)"), "4");
}

#[test]
fn qm1k1e_remove_and_buffered() {
    let js = with_buffer();
    js.run("sb.appendBuffer(INIT);");
    js.run("sb.appendBuffer(S0);");
    js.run("sb.appendBuffer(S1);");
    assert_eq!(js.val("sb.buffered.length + ':' + sb.buffered.start(0) + '-' + sb.buffered.end(0)"), "1:0-8");
    // duration NaN: TypeError
    js.run("ms.duration = 8;");
    js.log.borrow_mut().clear();
    js.run("sb.remove(0, 4); console.log('sync ' + sb.updating);");
    assert_eq!(js.lines(), ["log:sync true", "log:updatestart true", "log:update false", "log:updateend false"]);
    assert_eq!(js.val("sb.buffered.length + ':' + sb.buffered.start(0) + '-' + sb.buffered.end(0)"), "1:4-8");
    assert_eq!(js.val("[errName(function () { sb.remove(-1, 2) }), errName(function () { sb.remove(9, 10) }), errName(function () { sb.remove(2, 2) }), errName(function () { sb.remove(NaN, 2) }), errName(function () { sb.remove(0, NaN) })].join()"), "TypeError,TypeError,TypeError,TypeError,TypeError");
    js.run("sb.remove(0, Infinity);");
    assert_eq!(js.val("sb.buffered.length"), "0");
    // before a duration exists remove() is a TypeError
    let js = with_buffer();
    assert_eq!(js.val("errName(function () { sb.remove(0, 1) })"), "TypeError");
}

#[test]
fn qm1k1e_parse_error_fires_error_and_ends_the_stream_with_decode() {
    let js = with_buffer();
    js.run("sb.appendBuffer(new Uint8Array([0, 0, 0, 4, 102, 111, 111, 111, 0, 0, 0, 0]));");
    assert_eq!(js.lines(), ["log:updatestart true", "log:error false", "log:updateend false", "log:sourceended", "log:error"]);
    assert_eq!(js.val("[ms.readyState, v.error && v.error.code].join()"), "ended,3");
}

#[test]
fn qm1k1e_abort_while_updating_and_in_the_wrong_state() {
    let js = with_buffer();
    js.run("sb.appendBuffer(INIT);");
    js.log.borrow_mut().clear();
    js.run("sb.appendBuffer(S0); sb.abort(); console.log('after abort ' + sb.updating);");
    assert_eq!(js.lines(), ["log:after abort false", "log:abort false", "log:updateend false"]);
    // abort when idle: no events
    js.log.borrow_mut().clear();
    js.run("sb.abort();");
    assert!(js.lines().is_empty(), "{:?}", js.lines());
    // abort resets the parser input: half a segment, abort, then a whole one parses cleanly
    js.run("sb.appendBuffer(S1.subarray(0, 30));");
    js.run("sb.abort(); sb.appendBuffer(S1);");
    assert_eq!(js.val("sb.buffered.end(sb.buffered.length - 1)"), "8");
    // abort on a MediaSource that is not open
    js.run("ms.endOfStream();");
    assert_eq!(js.val("errName(function () { sb.abort() })"), "InvalidStateError");
}

#[test]
fn qm1k1e_append_after_ended_reopens_the_source() {
    let js = with_buffer();
    js.run("sb.appendBuffer(INIT);");
    js.run("sb.appendBuffer(S0);");
    js.run("ms.endOfStream();");
    assert_eq!(js.val("ms.readyState"), "ended");
    js.log.borrow_mut().clear();
    js.run("sb.appendBuffer(S1); console.log('state ' + ms.readyState);");
    assert_eq!(js.lines()[0], "log:state open");
    assert!(js.lines().contains(&"log:sourceopen".to_string()), "{:?}", js.lines());
}

#[test]
fn qm1k1e_attributes_and_their_validation() {
    let js = with_buffer();
    assert_eq!(js.val("[sb.mode, sb.timestampOffset, sb.appendWindowStart, sb.appendWindowEnd, typeof sb.audioTracks, typeof sb.videoTracks, 'buffered' in sb, sb.buffered instanceof TimeRanges].join()"), "segments,0,0,Infinity,undefined,undefined,true,true");
    js.run("sb.timestampOffset = 2.5; sb.appendWindowStart = 1; sb.appendWindowEnd = 30;");
    assert_eq!(js.val("[sb.timestampOffset, sb.appendWindowStart, sb.appendWindowEnd].join()"), "2.5,1,30");
    assert_eq!(
        js.val("[errName(function () { sb.appendWindowStart = -1 }), errName(function () { sb.appendWindowStart = 30 }), errName(function () { sb.appendWindowEnd = 1 }), errName(function () { sb.appendWindowEnd = NaN }), errName(function () { sb.timestampOffset = Infinity })].join()"),
        "TypeError,TypeError,TypeError,TypeError,TypeError"
    );
    // abort resets the append window
    js.run("sb.abort();");
    assert_eq!(js.val("[sb.appendWindowStart, sb.appendWindowEnd].join()"), "0,Infinity");
    // 'sequence' is stored and logged as not implemented
    js.run("sb.mode = 'sequence';");
    assert_eq!(js.val("sb.mode"), "sequence");
    assert!(js.lines().iter().any(|l| l.contains("not implemented") && l.contains("sequence")), "{:?}", js.lines());
    js.run("sb.mode = 'bogus';");
    assert_eq!(js.val("sb.mode"), "sequence");
}

#[test]
fn qm1k1e_change_type_validation() {
    let js = with_buffer();
    assert_eq!(
        js.val("[errName(function () { sb.changeType() }), errName(function () { sb.changeType('') }), errName(function () { sb.changeType('video/webm; codecs=\"vp9\"') }), errName(function () { sb.changeType('video/mp4; codecs=\"avc1.640028\"') })].join()"),
        "TypeError,TypeError,NotSupportedError,ok"
    );
    // a second init segment after changeType starts a new generation in the store
    js.run("sb.appendBuffer(INIT);");
    js.run("sb.changeType('video/mp4; codecs=\"avc1.640028\"'); sb.appendBuffer(INIT);");
    assert_eq!(js.val("sb.updating"), "false");
}

#[test]
fn qm1k1e_handlers_and_event_types() {
    let js = with_buffer();
    js.run("var got = []; ['updatestart', 'update', 'updateend', 'error', 'abort'].forEach(function (n) { sb['on' + n] = function (e) { got.push(e.type + ':' + (e.target === sb)); }; }); sb.appendBuffer(INIT);");
    assert_eq!(js.val("got.join()"), "updatestart:true,update:true,updateend:true");
}

/// The page from `qm1k1e_page.html` split into (html without script, script).
fn page_parts() -> (String, String) {
    let page = include_str!("qm1k1e_page.html");
    let (a, rest) = page.split_once("<script>").unwrap();
    let (code, b) = rest.split_once("</script>").unwrap();
    (format!("{a}{b}"), code.to_string())
}

#[cfg(feature = "media")]
#[test]
fn qm1k1e_local_page_plays_real_mp4_through_media_source_to_ended() {
    use super::super::mse_mpv_tests::real::{have_tools, make_fixture, Track};

    if !have_tools() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("qm1k1e-{}", std::process::id()));
    let Some((v, a)) = make_fixture(&dir, 3, 0.0, 1, 1) else {
        eprintln!("SKIP: ffmpeg could not make the fixture");
        return;
    };
    let join = |t: &Track| {
        let mut b = t.init.clone();
        for (_, _, s) in &t.segs {
            b.extend_from_slice(s);
        }
        b
    };
    let hdr = [("content-type", "video/mp4")];
    let ok = |b: Vec<u8>| Ok(WireResponse { status: 200, status_text: "OK".into(), headers: hdr.iter().map(|(x, y)| (x.to_string(), y.to_string())).collect(), body: b, error: None, final_url: None });
    let fake = QfFake::new(vec![("/v.mp4", ok(join(&v))), ("/a.mp4", ok(join(&a)))]);
    set_page_url(Some(&Url::parse("https://page.test/").unwrap()));
    crate::js_net::set_net_executor(fake.clone());
    let (html, code) = page_parts();
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let l = log.clone();
    let engine = crate::js::start_scripts(&parse_html(&html), &[src(&code)], Rc::new(move |lv, t| l.borrow_mut().push(format!("{lv}:{t}")))).unwrap();
    let mut sched = NetScheduler::new(&engine.net_policy());
    let t0 = std::time::Instant::now();
    let mut ended = false;
    while t0.elapsed() < std::time::Duration::from_secs(40) {
        crate::js_net::drive_report_net(&engine, &mut sched, &*fake);
        engine.run_due();
        if log.borrow().iter().any(|x| x.starts_with("log:ENDED") || x.starts_with("log:ERROR")) {
            ended = log.borrow().iter().any(|x| x.starts_with("log:ENDED"));
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let _ = std::fs::remove_dir_all(&dir);
    let l = log.borrow().clone();
    assert!(ended, "the page did not reach ended: {l:?}");
    let last = l.iter().find(|x| x.starts_with("log:ENDED")).unwrap();
    assert!(last.contains("of 3.0"), "{last}");
    assert!(l.contains(&"log:all appended, playing".to_string()), "{l:?}");
}
