//! C-FLICKER: the pure parts of the buffer sync fix and of the pixel probe the live check reads
//! (`sync::Held`, `syncobj::decide`, `probe`). The sync itself is checked live
//! (`test/c_flicker_live.py`, GPU nest).

use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use smithay::backend::renderer::sync::{Fence, Interrupted, SyncPoint};

use crate::probe::{self, Rect};
use crate::sync::Held;
use crate::syncobj;

/// A GPU fence the test signals by hand.
#[derive(Debug, Default)]
struct TestFence(Arc<AtomicBool>);

impl Fence for TestFence {
    fn is_signaled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn wait(&self) -> Result<(), Interrupted> {
        Ok(())
    }
    fn is_exportable(&self) -> bool {
        false
    }
    fn export(&self) -> Option<OwnedFd> {
        None
    }
}

fn fence() -> (SyncPoint, Arc<AtomicBool>) {
    let flag = Arc::new(AtomicBool::new(false));
    (SyncPoint::from(TestFence(flag.clone())), flag)
}

/// A buffer stand-in that records when it is let go (smithay's `Buffer` sends the release then).
struct Tracked(Arc<AtomicBool>);

impl Drop for Tracked {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn tracked() -> (Tracked, Arc<AtomicBool>) {
    let released = Arc::new(AtomicBool::new(false));
    (Tracked(released.clone()), released)
}

#[test]
fn a_drawn_buffer_is_released_only_after_the_gpu_finished_that_render() {
    let mut held: Held<Tracked> = Held::default();
    let (sync, done) = fence();
    let (buf, released) = tracked();
    held.push(sync, vec![buf]);
    held.collect();
    assert!(!released.load(Ordering::SeqCst), "released while the GPU may still read it");
    assert_eq!(held.pending(), 1);
    done.store(true, Ordering::SeqCst);
    held.collect();
    assert!(released.load(Ordering::SeqCst));
    assert_eq!(held.pending(), 0);
}

#[test]
fn renders_finish_independently() {
    let mut held: Held<Tracked> = Held::default();
    let (s1, d1) = fence();
    let (s2, _d2) = fence();
    let (b1, r1) = tracked();
    let (b2, r2) = tracked();
    held.push(s1, vec![b1]);
    held.push(s2, vec![b2]);
    d1.store(true, Ordering::SeqCst);
    held.collect();
    assert!(r1.load(Ordering::SeqCst));
    assert!(!r2.load(Ordering::SeqCst));
    assert_eq!(held.pending(), 1);
}

#[test]
fn a_render_without_page_buffers_holds_nothing() {
    // shm pages (copied into a texture at commit) and frames of the bar alone: no timer needed.
    let mut held: Held<Tracked> = Held::default();
    let (sync, _) = fence();
    held.push(sync, vec![]);
    assert_eq!(held.pending(), 0);
    let mut held: Held<Tracked> = Held::default();
    let (buf, released) = tracked();
    held.push(SyncPoint::signaled(), vec![buf]);
    held.collect();
    assert!(released.load(Ordering::SeqCst));
}

#[test]
fn explicit_sync_goes_with_dmabuf_and_a_syncobj_eventfd() {
    assert_eq!(syncobj::decide(true, false, || Ok(true)), Ok(()));
    // wl_shm only (no GPU, WEB_NO_DMABUF): nothing to sync, the device is not even asked.
    let asked = std::cell::Cell::new(false);
    assert!(syncobj::decide(false, false, || {
        asked.set(true);
        Ok(true)
    })
    .is_err());
    assert!(!asked.get());
    assert!(syncobj::decide(true, true, || Ok(true)).unwrap_err().contains("WEB_NO_SYNCOBJ"));
    assert!(syncobj::decide(true, false, || Ok(false)).unwrap_err().contains("eventfd"));
    assert_eq!(
        syncobj::decide(true, false, || Err("x: denied".into())),
        Err("x: denied".to_string())
    );
}

#[test]
fn probe_spec_parses_rectangles_or_turns_off() {
    assert_eq!(
        probe::parse("100,10,500,36;10,100,50,500"),
        Some(vec![
            Rect {
                x: 100,
                y: 10,
                w: 500,
                h: 36
            },
            Rect {
                x: 10,
                y: 100,
                w: 50,
                h: 500
            }
        ])
    );
    assert_eq!(probe::parse(" 1, 2, 3, 4 ;"), Some(vec![Rect { x: 1, y: 2, w: 3, h: 4 }]));
    for bad in ["", ";", "1,2,3", "1,2,3,4,5", "a,b,c,d", "1,2,0,4", "-1,2,3,4"] {
        assert_eq!(probe::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn probe_rectangles_map_to_gl_pixels_bottom_up() {
    let r = Rect {
        x: 100,
        y: 10,
        w: 500,
        h: 36,
    };
    // GL's origin is the bottom left: a strip 10 px below the top of a 978 px window.
    assert_eq!(probe::to_gl(r, 1.0, 960, 978), Some((100, 932, 500, 36)));
    // C6b: logical coordinates times the scale.
    assert_eq!(probe::to_gl(r, 1.5, 1440, 1467), Some((150, 1398, 750, 54)));
    // Clipped to the framebuffer; nothing left: None.
    assert_eq!(
        probe::to_gl(
            Rect {
                x: 900,
                y: 0,
                w: 100,
                h: 10
            },
            1.0,
            960,
            978
        ),
        Some((900, 968, 60, 10))
    );
    assert_eq!(
        probe::to_gl(
            Rect {
                x: 960,
                y: 0,
                w: 10,
                h: 10
            },
            1.0,
            960,
            978
        ),
        None
    );
}

#[test]
fn probe_reports_channel_ranges() {
    let px = [255, 0, 255, 255, 250, 10, 240, 0];
    assert_eq!(probe::min_max(&px), ([250, 0, 240], [255, 10, 255]));
    assert_eq!(probe::entry([250, 0, 240], [255, 10, 255]), "min 250,0,240 max 255,10,255");
}
