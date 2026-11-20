//! QMEM3g: the sampling heap profiler (only built with `--features memprof`; run locally:
//! `cargo test --features memprof memprof_tests`). The whole test binary runs under the
//! counting allocator, so tests identify their own allocations by exact size and thread.
use crate::memprof;

const SAMPLE: usize = 4096;

fn setup() {
    memprof::start();
    memprof::set_sample_bytes(SAMPLE);
}

#[inline(never)]
fn alloc_known_site(n: usize) -> Vec<u8> {
    std::hint::black_box(vec![1u8; n])
}

fn sites_of_size(n: u64) -> Vec<memprof::Site> {
    memprof::snapshot().into_iter().filter(|s| s.sampled_bytes == n && s.samples == 1).collect()
}

#[test]
fn known_allocation_shows_up_with_its_call_site_and_free_removes_it() {
    setup();
    const N: usize = 7_654_321;
    let v = alloc_known_site(N);
    let sites = sites_of_size(N as u64);
    assert_eq!(sites.len(), 1, "one sampled site of the exact size");
    let s = &sites[0];
    assert_eq!(s.bytes, N as u64);
    let f = alloc_known_site as *const () as usize;
    // A return address inside the known function appears in the stack.
    assert!(s.frames.iter().any(|a| *a > f && *a < f + 1024), "frames {:x?} lack fn at {f:x}", s.frames);
    drop(v);
    assert!(sites_of_size(N as u64).is_empty(), "free removes the sample");
}

#[test]
fn sampling_rate_is_roughly_one_per_interval() {
    setup();
    // A dedicated thread: the byte accumulator is per thread, so other tests do not disturb it.
    let h = std::thread::spawn(|| {
        let mut keep: Vec<Box<[u8; 1000]>> = Vec::with_capacity(5000);
        for _ in 0..4096 {
            keep.push(Box::new(std::hint::black_box([7u8; 1000])));
        }
        let tid = unsafe { libc::syscall(libc::SYS_gettid) } as u32;
        let n: u64 = memprof::snapshot().iter().filter(|s| s.tid == tid && s.sampled_bytes % 1000 == 0).map(|s| s.samples).sum();
        drop(keep);
        n
    });
    let n = h.join().unwrap();
    // 4096 * 1000 bytes / 4096 interval = ~1000 samples.
    assert!((800..=1200).contains(&n), "samples {n}");
}
