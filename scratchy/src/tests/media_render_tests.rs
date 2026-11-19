//! QM1g2a: the video render worker (frame slot, generations, lifecycle) with a stub
//! renderer; the real-mpv test is in `media_tests.rs`.
use crate::css::ObjectFit;
use crate::media_render::{FrameRenderer, HostWake, RenderWorker, Signal};
use crate::media_video::VideoTarget;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(5);

fn tgt(w: u32, h: u32) -> Option<VideoTarget> {
    Some(VideoTarget { w, h, fit: ObjectFit::Contain })
}

#[derive(Default)]
struct Log {
    /// Threads the renderer was created, updated, rendered and dropped on.
    threads: Vec<(&'static str, ThreadId)>,
    due: u32,
    updates: u32,
    renders: u32,
    skips: u32,
    dropped: bool,
}

/// A renderer whose frames are "due" when the test says so; `render` can be held at a gate.
struct Stub {
    log: Arc<Mutex<Log>>,
    gate: Option<(Sender<()>, Receiver<()>)>,
}

impl Stub {
    fn note(&self, what: &'static str) {
        self.log.lock().unwrap().threads.push((what, std::thread::current().id()));
    }
}

impl FrameRenderer for Stub {
    fn update(&mut self) -> bool {
        self.note("update");
        let mut l = self.log.lock().unwrap();
        l.updates += 1;
        if l.due > 0 {
            l.due -= 1;
            true
        } else {
            false
        }
    }
    fn render(&mut self, t: VideoTarget) -> Option<Vec<u32>> {
        self.note("render");
        if let Some((started, go)) = &self.gate {
            let _ = started.send(());
            let _ = go.recv_timeout(WAIT);
        }
        let mut l = self.log.lock().unwrap();
        l.renders += 1;
        Some(vec![l.renders; (t.w * t.h) as usize])
    }
    fn skip(&mut self) {
        self.note("skip");
        self.log.lock().unwrap().skips += 1;
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.note("drop");
        self.log.lock().unwrap().dropped = true;
    }
}

struct Rig {
    w: RenderWorker,
    sig: Arc<Signal>,
    log: Arc<Mutex<Log>>,
    wakes: Receiver<()>,
}

impl Rig {
    /// `gate`: (render started -> test, test -> render may finish).
    fn new(gate: Option<(Sender<()>, Receiver<()>)>) -> Rig {
        let log = Arc::new(Mutex::new(Log::default()));
        let (sig_tx, sig_rx) = channel();
        let (wtx, wakes) = channel();
        let wtx = Mutex::new(wtx);
        let wake: HostWake = Arc::new(move || {
            let _ = wtx.lock().unwrap().send(());
        });
        let l = log.clone();
        let w = RenderWorker::spawn(
            move |sig| {
                let _ = sig_tx.send(sig);
                let s = Stub { log: l, gate };
                s.note("create");
                Some(Box::new(s) as Box<dyn FrameRenderer>)
            },
            wake,
        )
        .expect("worker starts");
        let sig = sig_rx.recv_timeout(WAIT).unwrap();
        Rig { w, sig, log, wakes }
    }
    /// One frame becomes due and the update callback fires.
    fn frame(&self) {
        self.log.lock().unwrap().due += 1;
        self.sig.notify();
    }
    fn wait_wake(&self) {
        self.wakes.recv_timeout(WAIT).expect("a frame was published");
    }
}

fn wait_until(f: impl Fn() -> bool) {
    let t0 = std::time::Instant::now();
    while !f() {
        assert!(t0.elapsed() < WAIT, "timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn qm1g2a_frames_are_rendered_and_the_renderer_lives_and_dies_on_the_worker_thread() {
    let r = Rig::new(None);
    r.w.set_target(tgt(4, 2));
    assert!(!r.w.pending() && r.w.take().is_none());
    r.frame();
    r.wait_wake();
    assert!(r.w.pending());
    let f = r.w.take().expect("frame");
    assert_eq!((f.w, f.h, f.px.len(), f.px[0]), (4, 2, 8, 1));
    assert!(!r.w.pending() && r.w.take().is_none(), "each frame is taken once");
    let me = std::thread::current().id();
    let on = r.w.rendered_on().unwrap();
    assert_ne!(on, me, "rendered off the calling thread");
    let log = r.log.clone();
    drop(r.w);
    let l = log.lock().unwrap();
    assert!(l.dropped, "dropping the worker drops the renderer before it returns");
    let kinds: Vec<&str> = l.threads.iter().map(|t| t.0).collect();
    assert_eq!(kinds.first(), Some(&"create"));
    assert_eq!(kinds.last(), Some(&"drop"));
    assert!(l.threads.iter().all(|t| t.1 == on), "create/update/render/drop on one thread: {:?}", l.threads);
}

#[test]
fn qm1g2a_slot_keeps_only_the_newest_frame() {
    let r = Rig::new(None);
    r.w.set_target(tgt(2, 2));
    for _ in 0..3 {
        r.frame();
        r.wait_wake();
    }
    let f = r.w.take().unwrap();
    assert_eq!(f.px[0], 3, "the third render, not a queued older one");
    assert!(r.w.take().is_none());
}

#[test]
fn qm1g2a_hidden_target_consumes_frames_without_rendering() {
    let r = Rig::new(None);
    r.w.set_target(None);
    r.frame();
    wait_until(|| r.log.lock().unwrap().skips == 1);
    // A visible frame after it proves the worker went past the hidden one.
    r.w.set_target(tgt(2, 1));
    r.frame();
    r.wait_wake();
    let l = r.log.lock().unwrap();
    assert_eq!((l.skips, l.renders), (1, 1), "the hidden frame was skipped, not rendered");
}

#[test]
fn qm1g2a_a_frame_rendered_for_the_old_size_is_dropped_after_a_resize() {
    let (started_tx, started_rx) = channel();
    let (go_tx, go_rx) = channel();
    let r = Rig::new(Some((started_tx, go_rx)));
    r.w.set_target(tgt(4, 4));
    r.frame();
    started_rx.recv_timeout(WAIT).unwrap(); // the worker is inside render(4x4)
    r.w.set_target(tgt(8, 2));
    go_tx.send(()).unwrap();
    r.frame();
    started_rx.recv_timeout(WAIT).unwrap();
    go_tx.send(()).unwrap();
    r.wait_wake();
    let f = r.w.take().unwrap();
    assert_eq!((f.w, f.h), (8, 2));
    assert!(r.wakes.try_recv().is_err(), "the 4x4 frame was never published");
    assert_eq!(r.log.lock().unwrap().renders, 2);
}

#[test]
fn qm1g2a_seek_flush_drops_the_waiting_frame_and_an_in_flight_one() {
    let (started_tx, started_rx) = channel();
    let (go_tx, go_rx) = channel();
    let r = Rig::new(Some((started_tx, go_rx)));
    r.w.set_target(tgt(2, 2));
    r.frame();
    started_rx.recv_timeout(WAIT).unwrap();
    go_tx.send(()).unwrap();
    r.wait_wake();
    assert!(r.w.pending());
    r.w.flush();
    assert!(!r.w.pending(), "the pre-seek frame is gone");
    r.frame();
    started_rx.recv_timeout(WAIT).unwrap();
    r.w.flush(); // a seek while that frame renders
    go_tx.send(()).unwrap();
    r.frame();
    started_rx.recv_timeout(WAIT).unwrap();
    go_tx.send(()).unwrap();
    r.wait_wake();
    assert_eq!(r.w.take().unwrap().px[0], 3, "only the post-seek frame is published");
    assert!(r.wakes.try_recv().is_err());
}

#[test]
fn qm1g2a_resize_with_a_picture_redraws_at_once_and_reset_forgets_the_picture() {
    let r = Rig::new(None);
    r.w.set_target(tgt(2, 2));
    r.frame();
    r.wait_wake();
    r.w.take().unwrap();
    let updates = r.log.lock().unwrap().updates;
    r.w.set_target(tgt(6, 3));
    r.wait_wake();
    let f = r.w.take().unwrap();
    assert_eq!((f.w, f.h), (6, 3), "the current frame again, at the new size");
    assert_eq!(r.log.lock().unwrap().updates, updates, "a redraw, not a new frame");
    // A load: nothing to redraw until the next frame.
    r.frame();
    r.wait_wake();
    r.w.reset();
    assert!(!r.w.pending() && r.w.take().is_none());
    r.w.set_target(tgt(3, 3));
    // Hidden then shown: no redraw either (the picture may be gone with vid=no).
    r.w.set_target(None);
    r.w.set_target(tgt(5, 5));
    r.frame();
    r.wait_wake();
    let renders = r.log.lock().unwrap().renders;
    assert_eq!(renders, 4, "no redraw after reset or after hiding");
    assert_eq!(r.w.take().unwrap().w, 5);
}

#[test]
fn qm1g2a_a_renderer_that_cannot_be_created_fails_the_spawn() {
    let wake: HostWake = Arc::new(|| {});
    assert!(RenderWorker::spawn(|_sig| None, wake).is_none());
}
