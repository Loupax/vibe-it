//! C6c: frame pacing. The shell renders at most once per outer frame and tells the page
//! (frame callbacks) to draw its next frame only after a render, so the page's cadence is the
//! outer window's. On a Wayland desktop winit already holds `RedrawRequested` back until the
//! desktop's frame callback for the window (smithay's `submit` calls `pre_present_notify`), so
//! the `Pacer` stays out of the way there (a timer on top could make a render miss that
//! callback and wait a whole extra frame). On X11 nothing holds a redraw back (the page's own
//! commit rate, e.g. 70 Hz, would drive the shell), so the `Pacer` spaces renders by one
//! interval of the monitor's refresh rate with a timer.
//! Nothing here runs while the page is idle: no dirty flag, no timer.

use std::time::{Duration, Instant};

/// The refresh rate assumed when the desktop reports none (millihertz).
pub const DEFAULT_REFRESH_MHZ: u32 = 60_000;

/// A refresh rate in millihertz that is usable: 0 (unknown) and absurd values fall back.
pub fn sane_refresh(mhz: Option<u32>) -> u32 {
    match mhz {
        Some(m) if (10_000..=1_000_000).contains(&m) => m,
        _ => DEFAULT_REFRESH_MHZ,
    }
}

pub struct Pacer {
    refresh_mhz: u32,
    /// Space the renders itself (X11); false when the desktop's frame callbacks do (Wayland).
    timed: bool,
    last_render: Option<Instant>,
}

impl Pacer {
    pub fn new(refresh_mhz: u32, timed: bool) -> Pacer {
        Pacer {
            refresh_mhz: sane_refresh(Some(refresh_mhz)),
            timed,
            last_render: None,
        }
    }

    pub fn refresh_mhz(&self) -> u32 {
        self.refresh_mhz
    }

    pub fn set_refresh(&mut self, mhz: u32) {
        self.refresh_mhz = sane_refresh(Some(mhz));
    }

    /// One outer frame.
    pub fn interval(&self) -> Duration {
        Duration::from_nanos(1_000_000_000_000 / self.refresh_mhz as u64)
    }

    /// How long a render wanted at `now` has to wait (None: render now).
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        if !self.timed {
            return None;
        }
        let last = self.last_render?;
        let min = self.interval();
        let since = now.saturating_duration_since(last);
        (since < min).then(|| min - since)
    }

    pub fn rendered(&mut self, now: Instant) {
        self.last_render = Some(now);
    }
}
