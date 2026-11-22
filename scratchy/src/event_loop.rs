//! QEL1 (R64 section 4, E1-E3): the rendering-steps driver, HTML "update the rendering", shared by the
//! App (window) and every headless mode (`--wpt-th-one`, `--js-report --run-timers`, `--perf-report`).
//!
//! One rendering update runs, in this order:
//!   1. resize events            `RenderingHost::run_resize_steps`
//!   2. scroll events            `run_scroll_steps` (hook; user-scroll events are QEL7)
//!   3. media-query `change`     `evaluate_media_queries` (hook; QEL7)
//!   4. CSS animation events     `update_animations`
//!   5. animation frame callbacks: the whole batch pending when the step starts, one timestamp, never
//!      split by the timer slice (`JsEngine::run_animation_frames`; rAF ids are their own space)
//!   6. style + layout           `update_style_layout` (the App hands DOM writes to the incremental
//!      engine; headless has nothing to do: geometry reads force the engine lazily, as before)
//!   7. ResizeObserver / IntersectionObserver hooks `run_observers` (RO gather/broadcast, then the IO evaluation)
//!   8. paint                    `paint` (App only)
//!
//! When it runs: the App at frame time (a rAF is pending and its 60 Hz frame is due, the viewport
//! changed, or an animation event is due); headless on virtual 16.7 ms frames, only while a rAF is
//! pending (`headless_frame_due`), so a page without rAF costs nothing extra.

use crate::js::JsEngine;

/// The frame interval (60 Hz) on the page's `__scratchy_now` clock (TIMERS_PRELUDE `FRAME`).
#[cfg(test)]
pub(crate) const FRAME_MS: f64 = 1000.0 / 60.0;

/// The steps of one rendering update that belong to the host. Defaults are the headless behaviour.
pub(crate) trait RenderingHost {
    fn engine(&self) -> Option<&JsEngine>;
    /// Step 1: fire `resize` when the viewport changed since the last update.
    fn run_resize_steps(&mut self) {}
    /// Step 2: pending scroll events (hook).
    fn run_scroll_steps(&mut self) {}
    /// Step 3: MediaQueryList `change` events (hook).
    fn evaluate_media_queries(&mut self) {}
    /// Step 4: CSS animation events and the restyle they need.
    fn update_animations(&mut self) {}
    /// Step 6: style and layout of what the steps above changed.
    fn update_style_layout(&mut self) {}
    /// Step 7: observer hooks.
    fn run_observers(&mut self) {
        if let Some(js) = self.engine() {
            js.observer_hook();
        }
    }
    /// Step 8: paint.
    fn paint(&mut self) {}
}

/// Runs one rendering update. `raf_ts` = the frame timestamp (ms on the page clock) when this update
/// is a 60 Hz animation frame: the rAF batch runs only then, so an update that is only for a
/// `resize` or a CSS animation event in between frames does not run rAF callbacks early (that
/// doubled a rAF loop's rate in the window). Returns how many animation frame callbacks ran.
pub(crate) fn update_the_rendering(host: &mut impl RenderingHost, raf_ts: Option<f64>) -> usize {
    host.run_resize_steps();
    host.run_scroll_steps();
    host.evaluate_media_queries();
    host.update_animations();
    let rafs = match (host.engine(), raf_ts) {
        (Some(js), Some(ts)) => js.run_animation_frames(ts),
        _ => 0,
    };
    host.update_style_layout();
    host.run_observers();
    host.paint();
    rafs
}

/// The headless host: no window, no paint, a fixed viewport, no CSS animation clock.
pub(crate) struct Headless<'a>(pub(crate) &'a JsEngine);

impl RenderingHost for Headless<'_> {
    fn engine(&self) -> Option<&JsEngine> {
        Some(self.0)
    }
}

/// Headless: the time of the next rendering update (a virtual 16.7 ms frame boundary), `None` when
/// nothing waits for one.
pub(crate) fn headless_frame_due(js: &JsEngine) -> Option<f64> {
    js.raf_due_ms()
}

/// Headless: runs the rendering update when its frame is due at `now`. Returns the callbacks run,
/// at least 1 when a frame ran (progress for the pumps), 0 when none was due.
pub(crate) fn headless_frame(js: &JsEngine, now: f64) -> usize {
    match headless_frame_due(js) {
        Some(due) if due <= now => update_the_rendering(&mut Headless(js), Some(now)).max(1),
        _ => 0,
    }
}

/// Headless: the earliest of the next timer and the next rendering frame (ms on the page clock).
/// The frame is asked first: `next_due_ms` checks for deferred promise jobs after its own eval, so
/// it must be the last eval before the caller jumps the clock.
pub(crate) fn headless_next_due(js: &JsEngine) -> Option<f64> {
    let frame = headless_frame_due(js);
    match (js.next_due_ms(), frame) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Headless, one turn at virtual time `now`: the due timers (tasks) first; when none ran, the
/// rendering update if its frame is due. Returns the callbacks run (0 = nothing was due).
pub(crate) fn headless_turn(js: &JsEngine, now: f64) -> usize {
    match js.run_due_counted() {
        0 => headless_frame(js, now),
        n => n,
    }
}
