//! QM1g2a: the video render worker. One thread per media backend owns the render
//! context (libmpv's `mpv_render_context` is not thread-safe: it is created, updated,
//! rendered with and freed on this one thread) and renders each frame into a fresh
//! buffer. The main thread only takes the newest finished frame out of a one-frame slot
//! (`take`), so a 30-40 ms software render under load no longer blocks input, JS and
//! layout.
//!
//! The slot holds at most one frame, the newest: a frame the main thread did not take
//! yet is replaced, never queued. Every change that makes a frame wrong (a new target
//! size or fit, a seek, a load or unload) bumps a generation and empties the slot; a
//! frame finished for an older generation is dropped instead of being published, so no
//! stale picture appears after a resize, seek or reload. Dropping the [`RenderWorker`]
//! stops the thread and joins it; the renderer is dropped (the render context freed) on
//! the worker thread, before the caller goes on (and, for mpv, before the core is
//! destroyed).
//!
//! Wake-ups: the renderer's update callback (any thread) calls [`Signal::notify`]; the
//! worker then asks the renderer whether a frame is due (`update`), renders it, puts it
//! in the slot and calls the host's wake function (the app's "video frame" user event).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{JoinHandle, ThreadId};

use crate::media_video::{VideoFrame, VideoTarget};

/// What the worker thread drives. Built by the factory ON the worker thread and dropped
/// there.
pub(crate) trait FrameRenderer {
    /// After a [`Signal::notify`]: true when a new frame is due.
    fn update(&mut self) -> bool;
    /// Renders the current frame at `t` into `px` (`t.w * t.h` pixels, zeroed; 0RGB, top
    /// byte clear); false on failure.
    fn render(&mut self, t: VideoTarget, px: &mut [u32]) -> bool;
    /// Consumes a due frame without rendering it (nobody looks at it).
    fn skip(&mut self);
}

/// QM1g2a1: recycles frame buffers between the worker and the main thread. A
/// [`VideoFrame`] returns its pixels here when its last holder drops it, so a steady
/// stream allocates nothing after warm-up (a fresh ~0.9 MB zeroed buffer per frame costs
/// page faults on the worker and a free on the main thread).
#[derive(Default)]
pub(crate) struct FramePool {
    free: Mutex<Vec<Vec<u32>>>,
    /// Test seam: buffers ever allocated.
    allocs: AtomicUsize,
}

/// Enough for the frame in the slot, one in the main thread's hands, one being rendered.
const POOL_KEEP: usize = 4;

impl FramePool {
    /// A zeroed buffer of `len` pixels.
    fn get(&self, len: usize) -> Vec<u32> {
        let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
        free.retain(|b| b.len() == len); // the target size changed: the old ones are useless
        if let Some(mut b) = free.pop() {
            drop(free);
            b.fill(0);
            return b;
        }
        drop(free);
        self.allocs.fetch_add(1, Ordering::Relaxed);
        vec![0u32; len]
    }
    pub(crate) fn put(&self, b: Vec<u32>) {
        let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
        if free.len() < POOL_KEEP && !b.is_empty() {
            free.push(b);
        }
    }
    #[cfg(test)]
    pub(crate) fn allocs(&self) -> usize {
        self.allocs.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
struct State {
    target: Option<VideoTarget>,
    epoch: u64,
    /// The renderer's update callback fired.
    woken: bool,
    /// Render the current frame again (the target changed while a picture exists).
    redraw: bool,
    quit: bool,
    slot: Option<Arc<VideoFrame>>,
    /// A frame was published since the last `reset` (a redraw has something to show).
    has_frame: bool,
    /// Test seam: the thread that rendered the last published frame.
    #[cfg_attr(not(test), allow(dead_code))]
    rendered_on: Option<ThreadId>,
}

/// Shared between the host, the worker and the renderer's update callback.
#[derive(Default)]
pub(crate) struct Signal {
    pool: Arc<FramePool>,
    st: Mutex<State>,
    cv: Condvar,
}

impl Signal {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// The renderer has something new (mpv's render-update callback; any thread, never
    /// called with the state lock held by the worker since the worker never calls the
    /// renderer while holding it).
    pub(crate) fn notify(&self) {
        self.lock().woken = true;
        self.cv.notify_one();
    }
}

pub(crate) type HostWake = Arc<dyn Fn() + Send + Sync>;

pub(crate) struct RenderWorker {
    sig: Arc<Signal>,
    th: Option<JoinHandle<()>>,
}

impl RenderWorker {
    /// Starts the worker; `make` builds the renderer on the worker thread (None = failed:
    /// the thread ends and `spawn` returns None). `wake` is called after each published
    /// frame.
    pub(crate) fn spawn<F>(make: F, wake: HostWake) -> Option<RenderWorker>
    where
        F: FnOnce(Arc<Signal>) -> Option<Box<dyn FrameRenderer>> + Send + 'static,
    {
        let sig = Arc::new(Signal::default());
        let (tx, rx) = std::sync::mpsc::sync_channel::<bool>(1);
        let s = sig.clone();
        let th = std::thread::Builder::new()
            .name("scratchy-video".into())
            .spawn(move || {
                let Some(r) = make(s.clone()) else {
                    let _ = tx.send(false);
                    return;
                };
                let _ = tx.send(true);
                run(r, &s, &*wake);
            })
            .ok()?;
        if rx.recv() == Ok(true) {
            Some(RenderWorker { sig, th: Some(th) })
        } else {
            let _ = th.join();
            None
        }
    }

    /// What to render (None: consume frames without rendering). A change drops the
    /// slot's frame and, when a picture exists, re-renders the current one at once.
    pub(crate) fn set_target(&self, t: Option<VideoTarget>) {
        let mut st = self.sig.lock();
        if st.target == t {
            return;
        }
        st.target = t;
        flush(&mut st);
        if t.is_none() {
            // Hidden: mpv may drop the video track (`vid=no`), so there is no picture to
            // redraw once it is shown again; the next frame comes from mpv.
            st.has_frame = false;
        } else if st.has_frame {
            st.redraw = true;
            drop(st);
            self.sig.cv.notify_one();
        }
    }

    /// A seek: the frame waiting in the slot (and any being rendered) is from before it.
    pub(crate) fn flush(&self) {
        flush(&mut self.sig.lock());
    }

    /// A load or unload: as `flush`, and there is no picture to redraw until the next frame.
    pub(crate) fn reset(&self) {
        let mut st = self.sig.lock();
        flush(&mut st);
        st.has_frame = false;
        st.redraw = false;
    }

    pub(crate) fn pending(&self) -> bool {
        self.sig.lock().slot.is_some()
    }

    /// The newest finished frame (each frame is returned once).
    pub(crate) fn take(&self) -> Option<Arc<VideoFrame>> {
        self.sig.lock().slot.take()
    }

    /// Test seam: how many frame buffers were allocated so far.
    #[cfg(test)]
    pub(crate) fn buffer_allocs(&self) -> usize {
        self.sig.pool.allocs()
    }

    /// Test seam: the thread that rendered the last published frame.
    #[cfg(test)]
    pub(crate) fn rendered_on(&self) -> Option<ThreadId> {
        self.sig.lock().rendered_on
    }
}

fn flush(st: &mut State) {
    st.epoch += 1;
    st.slot = None;
}

impl Drop for RenderWorker {
    fn drop(&mut self) {
        self.sig.lock().quit = true;
        self.sig.cv.notify_one();
        if let Some(th) = self.th.take() {
            let _ = th.join();
        }
    }
}

fn valid(t: &VideoTarget) -> bool {
    t.w > 0 && t.h > 0 && t.w <= 8192 && t.h <= 8192
}

fn run(mut r: Box<dyn FrameRenderer>, sig: &Signal, wake: &(dyn Fn() + Send + Sync)) {
    loop {
        let (target, epoch, woken, redraw) = {
            let mut st = sig.lock();
            while !st.woken && !st.redraw && !st.quit {
                st = sig.cv.wait(st).unwrap_or_else(|e| e.into_inner());
            }
            if st.quit {
                break;
            }
            let w = std::mem::take(&mut st.woken);
            let rd = std::mem::take(&mut st.redraw);
            (st.target, st.epoch, w, rd)
        };
        // The renderer is only called with the lock released (its update callback takes it).
        let due = woken && r.update();
        if !due && !redraw {
            continue;
        }
        let Some(t) = target.filter(valid) else {
            if due {
                r.skip();
            }
            continue;
        };
        let mut px = sig.pool.get(t.w as usize * t.h as usize);
        if !r.render(t, &mut px) {
            sig.pool.put(px);
            continue;
        }
        let mut st = sig.lock();
        if st.epoch != epoch || st.target != Some(t) || st.quit {
            drop(st);
            sig.pool.put(px);
            continue; // resized, seeked, reloaded or stopping meanwhile: stale
        }
        st.slot = Some(Arc::new(VideoFrame { w: t.w, h: t.h, px, pool: Some(sig.pool.clone()) }));
        st.has_frame = true;
        st.rendered_on = Some(std::thread::current().id());
        drop(st);
        wake();
    }
    drop(r); // the render context is freed here, on the thread that created it
}
