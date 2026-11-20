//! QMEM3g: feature-gated sampling heap profiler (`cargo build --release --features memprof`).
//!
//! Off by default: without the feature this module is not compiled and the system allocator is
//! used untouched (zero cost). With it, a counting `#[global_allocator]` wraps `System`:
//! roughly one allocation per `MEMPROF_SAMPLE_KB` KB allocated (default 8, per thread, counted
//! in bytes) gets a backtrace and an entry in a fixed live-pointer table (static, so it never
//! allocates); `free` removes the entry. A dump lists live sampled bytes per (thread, stack).
//!
//! Environment (read once by `start()`):
//! - `MEMPROF_SAMPLE_KB=N`  sampling interval in KB (default 8).
//! - `MEMPROF_OUT=path`     where to dump (default `memprof.out` in the cwd).
//!
//! Dump triggers: SIGUSR2 (a watcher thread polls the flag), normal process exit (atexit), and
//! the end of a `--perf-report` run. Resolve with `tools/memprof-resolve.py` (see PERF.md).
//!
//! Each live sample stands for `max(size, interval)` bytes (the usual threshold-sampling
//! estimate); allocations of at least the interval are sampled every time and counted exactly.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering, Ordering::Relaxed};

const TAB: usize = 1 << 20; // live-pointer table slots
const NST: usize = 1 << 15; // distinct stacks
const DEPTH: usize = 32;
const DEFAULT_SAMPLE: usize = 8 * 1024;

#[derive(Clone, Copy)]
struct Ent {
    ptr: usize,
    size: usize,
    st: u32,
    tid: u32,
}
static mut TABLE: [Ent; TAB] = [Ent { ptr: 0, size: 0, st: 0, tid: 0 }; TAB];
static mut STACKS: [[usize; DEPTH]; NST] = [[0; DEPTH]; NST];
static LOCK: AtomicBool = AtomicBool::new(false);
static ON: AtomicBool = AtomicBool::new(false);
static SAMPLE: AtomicUsize = AtomicUsize::new(DEFAULT_SAMPLE);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static DUMP_REQ: AtomicBool = AtomicBool::new(false);
static PERF_DUMPED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static ACC: Cell<usize> = const { Cell::new(0) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

fn lock() {
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Relaxed).is_err() {
        std::hint::spin_loop();
    }
}
fn unlock() {
    LOCK.store(false, Ordering::Release);
}
fn slot(ptr: usize) -> usize {
    (ptr >> 4).wrapping_mul(0x9E37_79B9_7F4A_7C15) % TAB
}

pub struct Prof;

#[global_allocator]
static ALLOC: Prof = Prof;

fn record(ptr: *mut u8, size: usize) {
    if ptr.is_null() || !ON.load(Relaxed) {
        return;
    }
    let t = SAMPLE.load(Relaxed);
    let take = ACC
        .try_with(|a| {
            let v = a.get() + size;
            if v >= t {
                a.set(v % t);
                true
            } else {
                a.set(v);
                false
            }
        })
        .unwrap_or(false);
    if !take {
        return;
    }
    // backtrace() may allocate on its first call (loads libgcc): do not sample ourselves.
    if BUSY.try_with(|b| b.replace(true)).unwrap_or(true) {
        return;
    }
    let mut fr = [0usize; DEPTH];
    unsafe { libc::backtrace(fr.as_mut_ptr() as *mut *mut libc::c_void, DEPTH as i32) };
    let tid = unsafe { libc::syscall(libc::SYS_gettid) } as u32;
    let mut h: usize = 1469598103934665603;
    for f in fr.iter() {
        h = (h ^ *f).wrapping_mul(1099511628211);
    }
    lock();
    unsafe {
        let stacks = &mut *std::ptr::addr_of_mut!(STACKS);
        let mut sid = usize::MAX;
        let mut i = h % NST;
        for _ in 0..16 {
            if stacks[i][0] == 0 {
                stacks[i] = fr;
                sid = i;
                break;
            }
            if stacks[i] == fr {
                sid = i;
                break;
            }
            i = (i + 1) % NST;
        }
        let mut stored = false;
        if sid != usize::MAX {
            let tab = &mut *std::ptr::addr_of_mut!(TABLE);
            let mut j = slot(ptr as usize);
            for _ in 0..64 {
                if tab[j].ptr <= 1 {
                    tab[j] = Ent { ptr: ptr as usize, size, st: sid as u32, tid };
                    stored = true;
                    break;
                }
                j = (j + 1) % TAB;
            }
        }
        if !stored {
            DROPPED.fetch_add(1, Relaxed);
        }
    }
    unlock();
    let _ = BUSY.try_with(|b| b.set(false));
}

fn forget(ptr: *mut u8) {
    if !ON.load(Relaxed) {
        return;
    }
    lock();
    unsafe {
        let tab = &mut *std::ptr::addr_of_mut!(TABLE);
        let mut j = slot(ptr as usize);
        for _ in 0..64 {
            if tab[j].ptr == ptr as usize {
                tab[j].ptr = 1; // tombstone
                break;
            }
            if tab[j].ptr == 0 {
                break;
            }
            j = (j + 1) % TAB;
        }
    }
    unlock();
}

unsafe impl GlobalAlloc for Prof {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        record(p, l.size());
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        record(p, l.size());
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        forget(p);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            forget(p);
            record(q, n);
        }
        q
    }
}

/// One group of live sampled allocations that share a thread and a stack.
pub struct Site {
    pub tid: u32,
    /// Estimated live bytes: sum over samples of `max(size, interval)`.
    pub bytes: u64,
    /// Sum of the exact sizes of the sampled allocations.
    pub sampled_bytes: u64,
    pub samples: u64,
    pub frames: Vec<usize>,
}

/// Set the sampling interval in bytes (clamped to >= 1).
pub fn set_sample_bytes(n: usize) {
    SAMPLE.store(n.max(1), Relaxed);
}

/// Enable sampling (idempotent): reads the env, installs the SIGUSR2 + exit dump hooks once.
pub fn start() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Some(kb) = std::env::var("MEMPROF_SAMPLE_KB").ok().and_then(|v| v.parse::<usize>().ok()) {
            set_sample_bytes(kb * 1024);
        }
        ON.store(true, Relaxed);
        extern "C" fn on_sig(_: libc::c_int) {
            DUMP_REQ.store(true, Relaxed);
        }
        extern "C" fn at_exit() {
            // A --perf-report dump (taken while the page is alive) must not be overwritten
            // by the exit dump, which sees a mostly torn-down heap.
            if !PERF_DUMPED.load(Relaxed) {
                dump_default();
            }
        }
        unsafe {
            libc::signal(libc::SIGUSR2, on_sig as *const () as usize);
            libc::atexit(at_exit);
        }
        let _ = std::thread::Builder::new().name("memprof".into()).spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if DUMP_REQ.swap(false, Relaxed) {
                dump_default();
            }
        });
    });
}

/// Aggregate the live sampled allocations per (thread, stack), biggest first.
pub fn snapshot() -> Vec<Site> {
    // Phase 1 counts live entries; phase 2 copies them into a Vec allocated outside the lock
    // (an allocation inside the lock would deadlock on our own spin lock).
    lock();
    let n = unsafe { (*std::ptr::addr_of!(TABLE)).iter().filter(|e| e.ptr > 1).count() };
    unlock();
    let mut live: Vec<Ent> = Vec::with_capacity(n + 4096);
    lock();
    unsafe {
        for e in (*std::ptr::addr_of!(TABLE)).iter() {
            if e.ptr > 1 && live.len() < live.capacity() {
                live.push(*e);
            }
        }
    }
    unlock();
    let t = SAMPLE.load(Relaxed) as u64;
    let mut agg: std::collections::HashMap<(u32, u32), (u64, u64, u64)> = Default::default();
    for e in &live {
        let x = agg.entry((e.tid, e.st)).or_default();
        x.0 += (e.size as u64).max(t);
        x.1 += e.size as u64;
        x.2 += 1;
    }
    let mut out = Vec::with_capacity(agg.len());
    // Stack slots are write-once and were complete before any live entry referenced them, so
    // they are read without the lock (allocating while holding it would deadlock: every
    // dealloc takes the lock).
    let stacks = unsafe { &*std::ptr::addr_of!(STACKS) };
    for ((tid, st), (bytes, sampled_bytes, samples)) in &agg {
        let fr: Vec<usize> = stacks[*st as usize].iter().copied().take_while(|x| *x != 0).collect();
        out.push(Site { tid: *tid, bytes: *bytes, sampled_bytes: *sampled_bytes, samples: *samples, frames: fr });
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out
}

/// Write the dump text (format read by tools/memprof-resolve.py).
pub fn dump_to(path: &str) -> std::io::Result<()> {
    use std::io::Write;
    let sites = snapshot();
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(f, "MEMPROF 1 sample_bytes={} dropped={}", SAMPLE.load(Relaxed), DROPPED.load(Relaxed))?;
    if let Ok(m) = std::fs::read_to_string("/proc/self/maps") {
        for l in m.lines().filter(|l| l.contains('/')) {
            writeln!(f, "MAP {l}")?;
        }
    }
    if let Ok(rd) = std::fs::read_dir("/proc/self/task") {
        for e in rd.flatten() {
            let c = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default();
            writeln!(f, "THREAD {} {}", e.file_name().to_string_lossy(), c.trim())?;
        }
    }
    for s in &sites {
        let fr: Vec<String> = s.frames.iter().map(|x| format!("{x:x}")).collect();
        writeln!(f, "S {} {} {} {} {}", s.tid, s.bytes, s.sampled_bytes, s.samples, fr.join(" "))?;
    }
    f.flush()
}

/// Dump to `$MEMPROF_OUT` (default `memprof.out`); errors are reported on stderr.
pub fn dump_default() {
    let p = std::env::var("MEMPROF_OUT").unwrap_or_else(|_| "memprof.out".into());
    match dump_to(&p) {
        Ok(()) => eprintln!("memprof: dump written to {p}"),
        Err(e) => eprintln!("memprof: dump to {p} failed: {e}"),
    }
}

/// The end-of-`--perf-report` dump: like `dump_default`, and the exit dump is then skipped.
pub fn dump_perf_report() {
    dump_default();
    PERF_DUMPED.store(true, Relaxed);
}
