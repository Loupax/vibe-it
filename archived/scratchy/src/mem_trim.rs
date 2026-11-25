//! QMEM3d: hand freed heap back to the OS. glibc keeps freed memory in its
//! arenas (R48: free_kept 111 MB of ~496 MB RSS on YouTube watch);
//! `malloc_trim(0)` releases it. It runs on a short-lived helper thread (it is
//! thread-safe and walks every arena, so it must not stall a frame), after a
//! navigation dropped the old page and once per idle period.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// No input and no video for this long = the window is idle.
pub const IDLE_AFTER: Duration = Duration::from_secs(5);

/// Trims requested since start (every request counts, also where trimming is not available).
static REQUESTS: AtomicUsize = AtomicUsize::new(0);

pub fn requests() -> usize {
    REQUESTS.load(Ordering::Relaxed)
}

/// Ask for a trim (asynchronous). Counted whether or not glibc is present.
pub fn request() {
    REQUESTS.fetch_add(1, Ordering::Relaxed);
    #[cfg(not(test))]
    spawn_trim();
}

#[cfg(all(not(test), target_os = "linux", target_env = "gnu"))]
fn spawn_trim() {
    let _ = std::thread::Builder::new().name("malloc-trim".into()).spawn(|| {
        // SAFETY: malloc_trim is thread-safe and takes a plain int.
        unsafe {
            libc::malloc_trim(0);
        }
    });
}

#[cfg(all(not(test), not(all(target_os = "linux", target_env = "gnu"))))]
fn spawn_trim() {}

/// Synchronous trim, for measurement. Returns how long it took.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn trim_now() -> Duration {
    let t = std::time::Instant::now();
    // SAFETY: see spawn_trim.
    unsafe {
        libc::malloc_trim(0);
    }
    t.elapsed()
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn trim_now() -> Duration {
    Duration::ZERO
}

/// Idle detection: fires at most once per idle period.
#[derive(Default)]
pub struct IdleTrim {
    trimmed: bool,
}

impl IdleTrim {
    /// Input or a navigation starts a new idle period.
    pub fn activity(&mut self) {
        self.trimmed = false;
    }

    /// True once when the time since the last input and the last video frame (None = never)
    /// both reach `IDLE_AFTER` and no video box is on screen.
    pub fn should_trim(&mut self, input_age: Option<Duration>, video_age: Option<Duration>, video_boxes: bool) -> bool {
        if self.trimmed || video_boxes {
            return false;
        }
        if input_age.is_some_and(|a| a < IDLE_AFTER) || video_age.is_some_and(|a| a < IDLE_AFTER) {
            return false;
        }
        self.trimmed = true;
        true
    }

    /// How long until the window could become idle; None when this period is already trimmed.
    pub fn wake_in(&self, input_age: Option<Duration>, video_age: Option<Duration>) -> Option<Duration> {
        if self.trimmed {
            return None;
        }
        let age = input_age.into_iter().chain(video_age).min().unwrap_or(IDLE_AFTER);
        Some(IDLE_AFTER.saturating_sub(age).max(Duration::from_millis(50)))
    }
}
