//! QMEM3a: cap glibc malloc arenas. Each thread that allocates gets its own
//! arena (up to 8 x cores), and freed memory stays in them (R48: 42 arenas
//! held 85-94 MB on YouTube watch). `mallopt(M_ARENA_MAX, n)` must run before
//! any thread is spawned. Override: `SCRATCHY_MALLOC_ARENAS=<n>`, 0 = leave
//! glibc's default.

/// Default cap when the env var is unset.
pub const DEFAULT_ARENAS: i32 = 2;

/// Parse the override: unset/garbage -> default, 0 -> None (do nothing).
pub fn arenas_from(value: Option<&str>) -> Option<i32> {
    match value.and_then(|v| v.trim().parse::<i32>().ok()) {
        Some(n) if n <= 0 => None,
        Some(n) => Some(n),
        None => Some(DEFAULT_ARENAS),
    }
}

/// Apply the cap. Returns the value set, or None when skipped.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn install() -> Option<i32> {
    let n = arenas_from(std::env::var("SCRATCHY_MALLOC_ARENAS").ok().as_deref())?;
    // SAFETY: mallopt is thread-safe and takes plain ints.
    let ok = unsafe { libc::mallopt(libc::M_ARENA_MAX, n) };
    (ok == 1).then_some(n)
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn install() -> Option<i32> {
    None
}
