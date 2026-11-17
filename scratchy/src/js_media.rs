//! QM1f: the natives behind the JS HTMLMediaElement (`media_prelude.js`).
//! The JS side is thin (attribute reflection, event/promise delivery); every
//! algorithm lives in `media_el::MediaCore`. Control calls queue what the
//! page must see (events, promise settlements) and `__media_pump` hands it
//! over together with whatever the backend produced since.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use rquickjs::{Ctx, Function};

use crate::media_el::{BackendCtx, Candidate, MediaHost, Out};
use crate::media_net::MediaMode;

pub(crate) const MEDIA_PRELUDE: &str = include_str!("media_prelude.js");

type Queue = Rc<RefCell<Vec<String>>>;

fn fmt_out(id: i32, o: Out) -> Option<String> {
    Some(match o {
        Out::Fire(n) => format!("{id} e {n}"),
        Out::SourceError(i) => format!("{id} s {i}"),
        Out::Resolve(t) => format!("{id} r {t}"),
        Out::Reject(t, n) => format!("{id} j {t} {n}"),
        Out::Unavailable | Out::BackendStarted => return None,
    })
}

pub(crate) fn install_media<'js>(ctx: &Ctx<'js>, ctxb: BackendCtx, log: crate::media_el::MediaLog) -> Rc<RefCell<MediaHost>> {
    let host = Rc::new(RefCell::new(MediaHost::new(ctxb, log)));
    let queue: Queue = Default::default();
    let g = ctx.globals();
    macro_rules! native {
        ($name:expr, $f:expr) => {
            if let Ok(f) = Function::new(ctx.clone(), $f) {
                let _ = g.set($name, f);
            }
        };
    }
    // Runs `f` on the core, queues the page-visible outs.
    macro_rules! ctl {
        ($h:expr, $q:expr, $id:expr, $f:expr) => {{
            let (r, outs) = $h.borrow_mut().with($id, $f);
            $q.borrow_mut().extend(outs.into_iter().filter_map(|(i, o)| fmt_out(i, o)));
            r
        }};
    }
    let (h, q) = (host.clone(), queue.clone());
    native!("__media_load", move |id: i32, cands: Vec<String>, from_attr: bool, mode: String, page: String| {
        let cs: Vec<Candidate> = cands.chunks(2).map(|c| Candidate { url: c[0].clone(), mime: c.get(1).cloned().unwrap_or_default() }).collect();
        let mode = match mode.as_str() {
            "anonymous" => MediaMode::Anonymous,
            "use-credentials" => MediaMode::UseCredentials,
            _ => MediaMode::NoCors,
        };
        ctl!(h, q, id, |c| c.load(cs, from_attr, mode, page));
    });
    let (h, q) = (host.clone(), queue.clone());
    native!("__media_play", move |id: i32, tok: u32| {
        ctl!(h, q, id, |c| c.play(tok));
    });
    let (h, q) = (host.clone(), queue.clone());
    native!("__media_pause", move |id: i32| {
        ctl!(h, q, id, |c| c.pause());
    });
    let (h, q) = (host.clone(), queue.clone());
    native!("__media_seek", move |id: i32, t: f64| {
        ctl!(h, q, id, |c| c.set_current_time(t));
    });
    let (h, q) = (host.clone(), queue.clone());
    native!("__media_setvol", move |id: i32, v: f64| -> String { ctl!(h, q, id, |c| c.set_volume(v)).err().unwrap_or("").to_string() });
    let (h, q) = (host.clone(), queue.clone());
    native!("__media_setmuted", move |id: i32, m: bool| {
        ctl!(h, q, id, |c| c.set_muted(m));
    });
    let (h, q) = (host.clone(), queue.clone());
    native!("__media_setrate", move |id: i32, v: f64| -> String { ctl!(h, q, id, |c| c.set_rate(v)).err().unwrap_or("").to_string() });
    let (h, q) = (host.clone(), queue.clone());
    native!("__media_setdefrate", move |id: i32, v: f64| {
        ctl!(h, q, id, |c| c.set_default_rate(v));
    });
    let h = host.clone();
    native!("__media_setflag", move |id: i32, which: String, on: bool| {
        let mut h = h.borrow_mut();
        let c = h.core(id);
        match which.as_str() {
            "loop" => c.loop_ = on,
            "muted0" => c.muted = on,
            _ => c.autoplay = on,
        }
    });
    let h = host.clone();
    native!("__media_num", move |id: i32, key: i32| -> f64 {
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
    native!("__media_ranges", move |id: i32, kind: i32| -> Vec<f64> {
        let mut h = h.borrow_mut();
        let c = h.core(id);
        let v = if kind == 0 { c.buffered() } else { c.seekable() };
        v.into_iter().flat_map(|(a, b)| [a, b]).collect()
    });
    let h = host.clone();
    native!("__media_text", move |id: i32, key: i32| -> String {
        let mut h = h.borrow_mut();
        let c = h.core(id);
        if key == 0 { c.error.as_ref().map(|e| e.1.clone()).unwrap_or_default() } else { c.current_src.clone() }
    });
    let h = host.clone();
    native!("__media_activate", move || {
        h.borrow_mut().activated = true;
    });
    let h = host.clone();
    native!("__media_can_play", move |t: String| -> String {
        if h.borrow_mut().backend_available() { crate::media_el::can_play_type(&t).to_string() } else { String::new() }
    });
    let h = host.clone();
    native!("__media_drop", move |id: i32| {
        h.borrow_mut().remove(id);
    });
    let (h, q) = (host.clone(), queue);
    native!("__media_pump", move |now: f64, wait: f64| -> Vec<String> {
        let outs = h.borrow_mut().pump(now, Duration::from_millis(wait.clamp(0.0, 200.0) as u64));
        let mut q = q.borrow_mut();
        q.extend(outs.into_iter().filter_map(|(i, o)| fmt_out(i, o)));
        // Tell the page whether to keep polling: a trailing marker.
        let busy = h.borrow().busy();
        let mut v = std::mem::take(&mut *q);
        v.push(format!("-1 b {}", busy as u8));
        v
    });
    host
}
