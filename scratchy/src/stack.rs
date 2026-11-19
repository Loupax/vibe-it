//! QSTK1: stack growth for the deep recursive walks.
//!
//! The box build (`build_box_for_element`), measure (`measure_inner`), the negative-z hoist
//! (`walk_negative_z`), paint (`paint_inner`) and hit test (`hit_test_inner`) recurse once per
//! nesting level with large frames (~14-20 KB a level in release), so a few hundred nested
//! elements overflowed the 8 MB main thread (headless, the window, and forced layout from a V8
//! native all run there) and ~150 overflowed the 2 MB shaping worker: SIGABRT, the whole browser
//! gone. Each of those functions now runs its body through `grow`: when less than `RED_ZONE` of
//! the current stack is left, the call continues on another stack segment (switched with `psm`,
//! the stack-switching crate under rustc's `stacker`), on any thread, so depth is bounded by
//! memory only. Above the red zone the check is a stack-pointer read and a compare.
//!
//! Unlike `stacker`, segments are kept for reuse (`KEEP` per thread): a fresh mmap per crossing
//! page-faulted every touched stack page again on every forced layout (+286 k faults, +50 %
//! forced-layout time on the 1 000-deep big-css fixture).

use std::cell::{Cell, RefCell};

/// Stack that must be left before a guarded call runs on the current stack: covers the unguarded
/// frames between two guarded calls plus leaf work (text shaping, image decoding).
const RED_ZONE: usize = 512 * 1024;
/// Usable size of each extra segment.
const SEGMENT: usize = 16 << 20;
/// Segments a thread keeps mapped for the next deep walk; deeper walks map more and unmap them after.
const KEEP: usize = 2;

thread_local! {
    /// Lowest usable address of the stack this thread is running on now (0 = unknown: never grow).
    static LIMIT: Cell<usize> = Cell::new(os_stack_limit());
    static SPARE: RefCell<Vec<Segment>> = const { RefCell::new(Vec::new()) };
}

/// The current thread's own stack bottom, from pthread (the main thread included).
fn os_stack_limit() -> usize {
    // SAFETY: pthread_getattr_np fills an attr we init/destroy ourselves.
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return 0;
        }
        let mut addr: *mut libc::c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        let rc = libc::pthread_attr_getstack(&attr, &mut addr, &mut size);
        libc::pthread_attr_destroy(&mut attr);
        if rc == 0 { addr as usize } else { 0 }
    }
}

/// An mmap'd stack: one guard page (PROT_NONE) below `SEGMENT` usable bytes.
struct Segment {
    map: *mut libc::c_void,
    page: usize,
}

impl Segment {
    fn new() -> Option<Segment> {
        // SAFETY: plain anonymous mapping, owned by the returned value (munmap in Drop).
        unsafe {
            let page = usize::try_from(libc::sysconf(libc::_SC_PAGESIZE)).ok().filter(|p| *p > 0)?;
            let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_STACK;
            let map =
                libc::mmap(std::ptr::null_mut(), SEGMENT + page, libc::PROT_READ | libc::PROT_WRITE, flags, -1, 0);
            if map == libc::MAP_FAILED {
                return None;
            }
            let seg = Segment { map, page };
            (libc::mprotect(map, page, libc::PROT_NONE) == 0).then_some(seg)
        }
    }

    fn base(&self) -> *mut u8 {
        // SAFETY: within the mapping (it is SEGMENT + page bytes long).
        unsafe { (self.map as *mut u8).add(self.page) }
    }
}

impl Drop for Segment {
    fn drop(&mut self) {
        // SAFETY: unmaps exactly the mapping `new` made; no frame lives on it any more.
        unsafe { libc::munmap(self.map, SEGMENT + self.page) };
    }
}

/// QOBS7: spare segments this thread keeps mapped, and the virtual bytes they span
/// (`MAP_NORESERVE`: only the pages a deep walk touched are resident).
pub(crate) fn spare_segments() -> (usize, usize) {
    let n = SPARE.with(|s| s.borrow().len());
    (n, n * SEGMENT)
}

/// Runs `f`, on another stack segment if the current stack has less than `RED_ZONE` left.
#[inline(always)]
pub(crate) fn grow<R>(f: impl FnOnce() -> R) -> R {
    let sp = psm::stack_pointer() as usize;
    if sp.saturating_sub(LIMIT.with(Cell::get)) >= RED_ZONE {
        f()
    } else {
        let mut f = Some(f);
        let mut ret = None;
        on_segment(&mut || ret = f.take().map(|f| f()));
        ret.expect("stack::grow callback ran")
    }
}

#[cold]
#[inline(never)]
fn on_segment(cb: &mut dyn FnMut()) {
    let Some(seg) = SPARE.with(|s| s.borrow_mut().pop()).or_else(Segment::new) else {
        // No memory for a segment: run in place, as before QSTK1.
        return cb();
    };
    let old = LIMIT.with(|l| l.replace(seg.base() as usize));
    // SAFETY: the segment is page-aligned, SEGMENT is a multiple of the page size, it stays mapped
    // until after `on_stack` returns, and the callback cannot unwind out of it (caught here).
    let panic = unsafe {
        psm::on_stack(seg.base(), SEGMENT, || std::panic::catch_unwind(std::panic::AssertUnwindSafe(cb)).err())
    };
    LIMIT.with(|l| l.set(old));
    SPARE.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() < KEEP {
            s.push(seg);
        }
    });
    if let Some(p) = panic {
        std::panic::resume_unwind(p);
    }
}
