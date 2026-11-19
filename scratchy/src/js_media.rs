//! QM1f: the natives behind the JS HTMLMediaElement (`media_prelude.js`).
//! The JS side is thin (attribute reflection, event/promise delivery); every
//! algorithm lives in `media_el::MediaCore`. Control calls queue what the
//! page must see (events, promise settlements) and `__media_pump` hands it
//! over together with whatever the backend produced since.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crate::js_engine::{native, Bytes, Ctx};

use crate::media_el::{BackendCtx, Candidate, MediaHost, Out};
use crate::media_net::MediaMode;

pub(crate) const MEDIA_PRELUDE: &str = include_str!("media_prelude.js");
pub(crate) const MSE_PRELUDE: &str = include_str!("mse_prelude.js");

type Queue = Rc<RefCell<Vec<String>>>;

fn fmt_out(id: i32, o: Out) -> Option<String> {
    Some(match o {
        Out::Fire(n) => format!("{id} e {n}"),
        Out::SourceError(i) => format!("{id} s {i}"),
        Out::Resolve(t) => format!("{id} r {t}"),
        Out::Reject(t, n) => format!("{id} j {t} {n}"),
        // QM1k1d: the MediaSource events (`id` = the element; the page routes by feed id)
        Out::MseOpen(f) => format!("{id} o {f}"),
        Out::MseEnded(f) => format!("{id} n {f}"),
        Out::MseClose(f) => format!("{id} c {f}"),
        Out::Unavailable | Out::BackendStarted => return None,
    })
}

/// Queues the page-visible outs; a closed MediaSource's feed is dropped (its JS object makes
/// a fresh one when it is attached again).
fn queue_outs(host: &Rc<RefCell<MediaHost>>, q: &Queue, outs: Vec<crate::media_el::HostOut>) {
    for (_, o) in &outs {
        if let Out::MseClose(f) = o {
            host.borrow_mut().mse.drop_feed(*f);
        }
    }
    q.borrow_mut().extend(outs.into_iter().filter_map(|(i, o)| fmt_out(i, o)));
}

pub(crate) fn install_media<'js>(ctx: &Ctx<'js>, ctxb: BackendCtx, log: crate::media_el::MediaLog) -> Rc<RefCell<MediaHost>> {
    let host = Rc::new(RefCell::new(MediaHost::new(ctxb, log)));
    let queue: Queue = Default::default();
    // Runs `f` on the core, queues the page-visible outs.
    macro_rules! ctl {
        ($h:expr, $q:expr, $id:expr, $f:expr) => {{
            let (r, outs) = $h.borrow_mut().with($id, $f);
            queue_outs(&$h, &$q, outs);
            r
        }};
    }
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__media_load", move |id: i32, cands: Vec<String>, from_attr: bool, mode: String, page: String| {
        let cs: Vec<Candidate> = cands.chunks(2).map(|c| Candidate { url: c[0].clone(), mime: c.get(1).cloned().unwrap_or_default() }).collect();
        let mode = match mode.as_str() {
            "anonymous" => MediaMode::Anonymous,
            "use-credentials" => MediaMode::UseCredentials,
            _ => MediaMode::NoCors,
        };
        ctl!(h, q, id, |c| c.load(cs, from_attr, mode, page));
    });
    let (h, q) = (host.clone(), queue.clone());
    // QM1k1g: a blob: src naming a real Blob; its bytes are played from memory.
    native!(ctx, "__media_load_blob", move |id: i32, url: String, data: Bytes, page: String| {
        let bytes = std::sync::Arc::new(data.0);
        ctl!(h, q, id, |c| c.load_blob(url, bytes, page));
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__media_play", move |id: i32, tok: u32| {
        ctl!(h, q, id, |c| c.play(tok));
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__media_pause", move |id: i32| {
        ctl!(h, q, id, |c| c.pause());
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__media_seek", move |id: i32, t: f64| {
        ctl!(h, q, id, |c| c.set_current_time(t));
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__media_setvol", move |id: i32, v: f64| -> String { ctl!(h, q, id, |c| c.set_volume(v)).err().unwrap_or("").to_string() });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__media_setmuted", move |id: i32, m: bool| {
        ctl!(h, q, id, |c| c.set_muted(m));
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__media_setrate", move |id: i32, v: f64| -> String { ctl!(h, q, id, |c| c.set_rate(v)).err().unwrap_or("").to_string() });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__media_setdefrate", move |id: i32, v: f64| {
        ctl!(h, q, id, |c| c.set_default_rate(v));
    });
    let h = host.clone();
    native!(ctx, "__media_setflag", move |id: i32, which: String, on: bool| {
        let mut h = h.borrow_mut();
        let c = h.core(id);
        match which.as_str() {
            "loop" => c.loop_ = on,
            "muted0" => c.muted = on,
            _ => c.autoplay = on,
        }
    });
    let h = host.clone();
    native!(ctx, "__media_num", move |id: i32, key: i32| -> f64 {
        let mut h = h.borrow_mut();
        let c = h.core(id);
        match key {
            0 => c.network_state as f64,
            1 => c.ready_state as f64,
            2 => c.paused as u8 as f64,
            3 => c.seeking as u8 as f64,
            4 => c.current_time,
            5 => c.duration,
            6 => c.volume,
            7 => c.rate,
            8 => c.ended() as u8 as f64,
            9 => c.video_width as f64,
            10 => c.video_height as f64,
            11 => c.error.as_ref().map_or(0.0, |e| e.0 as f64),
            12 => c.default_rate,
            13 => c.muted as u8 as f64,
            14 => c.loop_ as u8 as f64,
            _ => f64::NAN,
        }
    });
    let h = host.clone();
    native!(ctx, "__media_ranges", move |id: i32, kind: i32| -> Vec<f64> {
        let mut h = h.borrow_mut();
        let c = h.core(id);
        let v = if kind == 0 { c.buffered() } else { c.seekable() };
        v.into_iter().flat_map(|(a, b)| [a, b]).collect()
    });
    let h = host.clone();
    native!(ctx, "__media_text", move |id: i32, key: i32| -> String {
        let mut h = h.borrow_mut();
        let c = h.core(id);
        if key == 0 { c.error.as_ref().map(|e| e.1.clone()).unwrap_or_default() } else { c.current_src.clone() }
    });
    let h = host.clone();
    native!(ctx, "__media_activate", move || {
        h.borrow_mut().activated = true;
    });
    let h = host.clone();
    native!(ctx, "__media_can_play", move |t: String| -> String {
        if h.borrow_mut().backend_available() { crate::media_el::can_play_type(&t).to_string() } else { String::new() }
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__media_drop", move |id: i32| {
        let outs = h.borrow_mut().remove(id);
        queue_outs(&h, &q, outs);
    });
    install_mse(ctx, &host, &queue);
    let (h, q) = (host.clone(), queue);
    native!(ctx, "__media_pump", move |now: f64, wait: f64| -> Vec<String> {
        let outs = h.borrow_mut().pump(now, Duration::from_millis(wait.clamp(0.0, 200.0) as u64));
        queue_outs(&h, &q, outs);
        let mut q = q.borrow_mut();
        // Tell the page whether to keep polling: a trailing marker.
        let busy = h.borrow().busy();
        let mut v = std::mem::take(&mut *q);
        v.push(format!("-1 b {}", busy as u8));
        v
    });
    host
}

/// QM1k1d: the natives behind `MediaSource` / `SourceBufferList` (`mse_prelude.js`). A feed is
/// created when a MediaSource is attached to an element, so a page that never uses MSE
/// creates nothing. Feed ids cross as numbers.
fn install_mse<'js>(ctx: &Ctx<'js>, host: &Rc<RefCell<MediaHost>>, queue: &Queue) {
    let h = host.clone();
    native!(ctx, "__mse_supported", move |t: String| -> bool { h.borrow_mut().backend_available() && crate::media_el::mse_type_supported(&t) });
    let h = host.clone();
    native!(ctx, "__mse_new", move || -> f64 { h.borrow_mut().mse.create().id as f64 });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__mse_attach", move |el: i32, feed: f64, dur: f64, page: String| {
        let Some(f) = h.borrow().mse.get(feed as u64) else { return };
        let ((), outs) = h.borrow_mut().with(el, |c| c.attach_mse(f, dur, page));
        queue_outs(&h, &q, outs);
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__mse_fail", move |el: i32, page: String| {
        let ((), outs) = h.borrow_mut().with(el, |c| c.fail_src(page, "the blob: URL was revoked or is unknown"));
        queue_outs(&h, &q, outs);
    });
    let h = host.clone();
    native!(ctx, "__mse_add_buffer", move |feed: f64, ty: String| -> i32 {
        let Some(f) = h.borrow().mse.get(feed as u64) else { return -1 };
        f.with_store(|st| st.add_buffer(crate::mse_store::SourceBufferStore::fmp4().with_type(&ty))) as i32
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__mse_remove_buffer", move |feed: f64, bi: i32, now: f64| {
        let Some(f) = h.borrow().mse.get(feed as u64) else { return };
        f.with_store(|st| st.remove_buffer(bi as usize));
        let r = h.borrow_mut().with_mse(feed as u64, |c| c.mse_data_changed(now, true));
        if let Some((_, outs)) = r {
            queue_outs(&h, &q, outs);
        }
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__mse_set_duration", move |feed: f64, d: f64| {
        let r = h.borrow_mut().with_mse(feed as u64, |c| c.mse_set_duration(d));
        if let Some((_, outs)) = r {
            queue_outs(&h, &q, outs);
        }
    });
    let h = host.clone();
    native!(ctx, "__mse_buffered_end", move |feed: f64| -> f64 {
        let Some(f) = h.borrow().mse.get(feed as u64) else { return 0.0 };
        f.with_store(|st| st.buffers.iter().filter_map(|b| b.buffered().last().map(|r| r.1)).fold(0.0, f64::max))
    });
    // QM1k1e: SourceBuffer natives. Result of append: 0 stored, 1 quota (nothing stored), 2 parse
    // error (what parsed before it is kept), -1 no such feed.
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__mse_append", move |feed: f64, bi: i32, data: Bytes, now: f64| -> i32 {
        let Some(f) = h.borrow().mse.get(feed as u64) else { return -1 };
        let t = h.borrow().mse_current_time(feed as u64);
        match f.append(bi as usize, &data.0, t) {
            Ok(o) => {
                let r = h.borrow_mut().with_mse(feed as u64, |c| c.mse_data_changed(now, o.new_generation));
                if let Some((_, outs)) = r {
                    queue_outs(&h, &q, outs);
                }
                if o.unshifted > 0 { 3 } else { 0 }
            }
            Err(crate::mse_store::MseError::QuotaExceeded) => 1,
            Err(crate::mse_store::MseError::Parse(_)) => {
                let r = h.borrow_mut().with_mse(feed as u64, |c| c.mse_data_changed(now, false));
                if let Some((_, outs)) = r {
                    queue_outs(&h, &q, outs);
                }
                2
            }
        }
    });
    // QM1k1i: op 0 mode (v = 1 sequence), 1 timestampOffset, 2 appendWindowStart, 3 appendWindowEnd.
    let h = host.clone();
    native!(ctx, "__mse_sb_cfg", move |feed: f64, bi: i32, op: i32, v: f64| {
        let Some(f) = h.borrow().mse.get(feed as u64) else { return };
        f.with_store(|st| {
            if let Some(b) = st.buffers.get_mut(bi as usize) {
                let (ws, we) = b.append_window();
                match op {
                    0 => b.set_sequence(v == 1.0),
                    1 => b.set_timestamp_offset(v),
                    2 => b.set_append_window(v, we),
                    _ => b.set_append_window(ws, v),
                }
            }
        });
    });
    let h = host.clone();
    native!(ctx, "__mse_sb_tso", move |feed: f64, bi: i32| -> f64 {
        let Some(f) = h.borrow().mse.get(feed as u64) else { return 0.0 };
        f.with_store(|st| st.buffers.get(bi as usize).map_or(0.0, |b| b.timestamp_offset()))
    });
    let h = host.clone();
    native!(ctx, "__mse_abort", move |feed: f64, bi: i32| {
        let Some(f) = h.borrow().mse.get(feed as u64) else { return };
        f.with_store(|st| {
            if let Some(b) = st.buffers.get_mut(bi as usize) {
                b.abort();
            }
        });
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__mse_remove", move |feed: f64, bi: i32, a: f64, b: f64, now: f64| {
        let Some(f) = h.borrow().mse.get(feed as u64) else { return };
        f.with_store(|st| st.buffers.get_mut(bi as usize).map(|s| s.remove(a, b)));
        let r = h.borrow_mut().with_mse(feed as u64, |c| c.mse_data_changed(now, false));
        if let Some((_, outs)) = r {
            queue_outs(&h, &q, outs);
        }
    });
    let h = host.clone();
    native!(ctx, "__mse_buffered", move |feed: f64, bi: i32| -> Vec<f64> {
        let Some(f) = h.borrow().mse.get(feed as u64) else { return Vec::new() };
        let v = f.with_store(|st| st.buffers.get(bi as usize).map(|b| b.buffered()).unwrap_or_default());
        v.into_iter().flat_map(|(a, b)| [a, b]).collect()
    });
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__mse_reopen", move |feed: f64| {
        let r = h.borrow_mut().with_mse(feed as u64, |c| c.mse_reopen());
        if let Some((_, outs)) = r {
            queue_outs(&h, &q, outs);
        }
    });
    // kind: 0 = no error, 1 = 'network', 2 = 'decode'. Returns the element's duration after.
    let (h, q) = (host.clone(), queue.clone());
    native!(ctx, "__mse_eos", move |feed: f64, kind: i32| -> f64 {
        let r = h.borrow_mut().with_mse(feed as u64, |c| {
            match kind {
                1 => c.mse_end_of_stream_error(crate::media_el::MEDIA_ERR_NETWORK),
                2 => c.mse_end_of_stream_error(crate::media_el::MEDIA_ERR_DECODE),
                _ => c.mse_end_of_stream(),
            }
            c.duration
        });
        match r {
            Some((d, outs)) => {
                queue_outs(&h, &q, outs);
                d
            }
            None => f64::NAN,
        }
    });
}
