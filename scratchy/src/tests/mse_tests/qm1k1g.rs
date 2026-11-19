// QM1k1g: `video.src = URL.createObjectURL(blob)` for a real Blob plays from memory.
use super::qm1k1d::{Js, HTML};

#[test]
fn qm1k1g_blob_src_reaches_the_backend_as_bytes_not_a_network_load() {
    let js = Js::new(
        HTML,
        "var v = document.getElementById('v'); var b = new Blob([new Uint8Array([1,2,3,4,5]), new Uint8Array([6])], { type: 'video/mp4' });\
         var u = URL.createObjectURL(b); v.src = u; globalThis.u = u;",
    );
    js.step(100.0);
    let f = js.fakes.borrow()[0].clone();
    let req = f.borrow().loaded.clone().expect("the backend got a load");
    assert_eq!(req.blob.as_ref().map(|b| b.as_slice()), Some(&[1u8, 2, 3, 4, 5, 6][..]));
    assert_eq!(req.url, js.val("u"));
    assert_eq!(js.val("v.currentSrc"), js.val("u"));
    // a plain network src carries no bytes
    let js = Js::new(HTML, "var v = document.getElementById('v'); v.src = 'https://x.test/a.mp4';");
    js.step(100.0);
    let f = js.fakes.borrow()[0].clone();
    assert!(f.borrow().loaded.clone().unwrap().blob.is_none());
}

#[test]
fn qm1k1g_revoked_blob_src_fails_and_never_reaches_the_backend() {
    let js = Js::new(
        HTML,
        "var v = document.getElementById('v'); v.addEventListener('error', function () { console.log('error ' + v.error.code); });\
         var u = URL.createObjectURL(new Blob([new Uint8Array([1])])); URL.revokeObjectURL(u); v.src = u;",
    );
    js.step(100.0);
    assert!(js.lines().contains(&"log:error 4".to_string()), "{:?}", js.lines());
    assert!(js.fakes.borrow().iter().all(|f| f.borrow().loaded.is_none()));
}

/// A whole faststart mp4 held in a Blob plays to `ended` with libmpv and no wire at all
/// (the backend has no socket, so any network read would be a MediaError).
#[cfg(feature = "media")]
#[test]
fn qm1k1g_blob_of_a_real_mp4_plays_to_ended_without_wire() {
    use crate::media_el::*;
    use crate::media_mpv::MpvBackend;
    use std::sync::Arc;
    if !crate::media_mpv::libmpv_available() {
        eprintln!("SKIP: libmpv.so.2 not available");
        return;
    }
    let dir = std::env::temp_dir().join(format!("qm1k1g-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join("c.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25", "-t", "2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
        .arg(&p)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let bytes = if ok { std::fs::read(&p).ok() } else { None };
    let _ = std::fs::remove_dir_all(&dir);
    let Some(bytes) = bytes else {
        eprintln!("SKIP: ffmpeg could not make a test clip");
        return;
    };
    let backend = MpvBackend::new(BackendCtx { socket: None, policy: Default::default() });
    let mut c = MediaCore::new(Box::new(backend));
    c.muted = true;
    c.load_blob("blob:https://page.test/1".into(), Arc::new(bytes), "https://page.test/".into());
    let mut now = 0.0;
    let mut names: Vec<String> = Vec::new();
    let step = |c: &mut MediaCore, now: &mut f64, names: &mut Vec<String>, until: &dyn Fn(&MediaCore) -> bool| {
        for _ in 0..600 {
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
    };
    step(&mut c, &mut now, &mut names, &|c| c.ready_state >= HAVE_ENOUGH_DATA);
    assert!(c.error.is_none(), "{:?} {names:?}", c.error);
    assert!((c.duration - 2.0).abs() < 0.2, "duration {}", c.duration);
    assert_eq!((c.video_width, c.video_height), (320, 240));
    c.play(1);
    step(&mut c, &mut now, &mut names, &|c| c.ended());
    assert!(names.contains(&"ended".to_string()) && c.paused, "{names:?}");
}
