//! QEL1 (R64 section 4, E1-E3): the rendering-steps driver, HTML "update the rendering", shared by the
//! App (window) and every headless mode (`--wpt-th-one`, `--js-report --run-timers`, `--perf-report`).
//!
//! One rendering update runs, in this order:
//!   1. resize events            `RenderingHost::run_resize_steps`
//!   2. scroll events            `run_scroll_steps` (QEL7: `scroll` per queued target, then `scrollend`)
//!   3. media-query `change`     `evaluate_media_queries` (QEL7: lists whose result flipped)
//!   4. CSS animation events     `update_animations`
//!   5. animation frame callbacks: the whole batch pending when the step starts, one timestamp, never
//!      split by the timer slice (`JsEngine::run_animation_frames`; rAF ids are their own space)
//!   6. style + layout           `update_style_layout` (the App hands DOM writes to the incremental
//!      engine and lays out in `redraw`; headless lays out here when CSS animations or a
//!      ResizeObserver observation need it, QANIMHL / QEL2b; other geometry reads force lazily)
//!   7. ResizeObserver / IntersectionObserver hooks `run_observers` (RO gather/broadcast on the step-6
//!      layout: the App's frame, `ro_geom.rs`; then the IO evaluation)
//!   8. paint                    `paint` (App only)
//!
//! When it runs: the App at frame time (a rAF is pending and its 60 Hz frame is due, the viewport
//! changed, or an animation event is due); headless on virtual 16.7 ms frames, only while a rAF is
//! pending, an observer wants one, or (QANIMHL) a page with `@keyframes` changed its DOM or has a CSS
//! animation event coming (`headless_frame_due`), so a page with none of these costs nothing extra.

use crate::js::JsEngine;

/// The frame interval (60 Hz) on the page's `__scratchy_now` clock (TIMERS_PRELUDE `FRAME`).
#[cfg(test)]
pub(crate) const FRAME_MS: f64 = 1000.0 / 60.0;

/// The steps of one rendering update that belong to the host. Defaults are the headless behaviour.
pub(crate) trait RenderingHost {
    fn engine(&self) -> Option<&JsEngine>;
    /// Step 1: fire `resize` when the viewport changed since the last update.
    fn run_resize_steps(&mut self) {}
    /// Step 2: pending `scroll` / `scrollend` events (QEL7).
    fn run_scroll_steps(&mut self) {
        if let Some(js) = self.engine() {
            js.run_scroll_steps(false);
        }
    }
    /// Step 3: MediaQueryList `change` events (QEL7).
    fn evaluate_media_queries(&mut self) {
        if let Some(js) = self.engine() {
            js.run_media_queries();
        }
    }
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

/// The headless host: no window, no paint, a fixed viewport; the page clock (`now`) is the CSS
/// animation clock (QANIMHL).
pub(crate) struct Headless<'a> {
    pub(crate) js: &'a JsEngine,
    pub(crate) now: f64,
}

/// QANIMHL: the headless CSS animation state of one page (kept in its `JsEngine`). The App has its own
/// `AnimTracker`, fed by its builds; here the style pass is the page's forced layout, run by the
/// rendering update's style step when the page defines `@keyframes` and its DOM changed.
#[derive(Default)]
pub(crate) struct HeadlessAnim {
    tracker: crate::AnimTracker,
    /// The geometry generation whose rules were checked for `@keyframes`, and the answer.
    checked: Option<(u64, bool)>,
    /// The geometry generation the last style step saw.
    styled: Option<u64>,
    /// When the DOM change that wants a style step was first noticed (page clock).
    dirty_since: Option<f64>,
    /// `anim_eval::animation_passes` at the last report taken.
    passes: u64,
    /// The page clock time at which the tracker next has something to say (an event, a phase change).
    next: Option<f64>,
    /// QEL7: when scroll events (or a `scrollend`) were first noticed owed (page clock).
    scroll_since: Option<f64>,
}

/// The first 60 Hz frame boundary strictly after `t` (TIMERS_PRELUDE `__scratchy_raf_due`'s rounding).
fn frame_after(t: f64) -> f64 {
    let frame = 1000.0 / 60.0;
    let mut due = ((t / frame).floor() + 1.0) * frame;
    while due <= t {
        due += frame;
    }
    due
}

impl HeadlessAnim {
    /// The DOM changed since the last style step and the page has `@keyframes` (an animation may have
    /// started, changed or stopped), or animations are live (one may have been removed).
    fn style_wanted(&mut self, js: &JsEngine, generation: u64) -> bool {
        if self.styled == Some(generation) {
            return false;
        }
        let has = match self.checked {
            Some((g, has)) if g == generation => has,
            _ => {
                let has = js.page_has_keyframes();
                self.checked = Some((generation, has));
                has
            }
        };
        // QANIMHLb: a page that declares transitions may start one on any DOM change.
        has || crate::transition::any_declared() || !self.tracker.is_idle() || crate::transition::live()
    }
}

/// QANIMHL: the time of the rendering update the page's CSS animations want (a frame boundary), `None`
/// when they want none. The page clock is read only when a DOM change is first noticed.
fn headless_anim_due(js: &JsEngine) -> Option<f64> {
    let generation = crate::js_dom::geom_gen();
    let mut a = js.anim.borrow_mut();
    let dirty = if a.style_wanted(js, generation) {
        Some(*a.dirty_since.get_or_insert_with(|| js.now_ms()))
    } else {
        None
    };
    // QANIMHLb: a transition a script's style read started owes its `transitionrun` at the next frame.
    let owed = (a.next.is_none() && crate::transition::owes_events()).then(|| js.now_ms());
    let since = [dirty, a.next, owed].into_iter().flatten().reduce(f64::min);
    // QEL7: queued `scroll` events and waiting `scrollend`s are owed at the next frame boundary.
    let scroll = if js.qel7_bits() & 3 != 0 {
        Some(*a.scroll_since.get_or_insert_with(|| js.now_ms()))
    } else {
        a.scroll_since = None;
        None
    };
    [since, scroll].into_iter().flatten().reduce(f64::min).map(frame_after)
}

impl RenderingHost for Headless<'_> {
    fn engine(&self) -> Option<&JsEngine> {
        Some(self.js)
    }
    fn run_scroll_steps(&mut self) {
        self.js.anim.borrow_mut().scroll_since = None;
        self.js.run_scroll_steps(false);
    }
    /// Step 4: the events of the animations the last style step reported, at the page clock.
    fn update_animations(&mut self) {
        let trans = crate::transition::live();
        let (t, tt) = {
            let mut a = self.js.anim.borrow_mut();
            if a.tracker.is_idle() && !trans {
                a.next = None;
                return;
            }
            // The animation timeline moves with the rendering updates (getComputedStyle between them
            // reads this frame's values). No frames are added just to move it.
            crate::anim_eval::set_document_clock(self.now);
            let t = if a.tracker.is_idle() { crate::AnimTick::default() } else { a.tracker.tick(self.now) };
            // QANIMHLb: CSS transitions; headless paints nothing, so no in-between frames.
            let tt = crate::transition::tick(self.now, f64::INFINITY);
            a.next = match (t.next, tt.next) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (x, y) => x.or(y),
            };
            (t, tt)
        };
        for ev in tt.events {
            let Some(node) = ev.node else { continue };
            let init = [
                ("bubbles", crate::js::EventField::Bool(true)),
                ("propertyName", crate::js::EventField::Str(ev.property.to_string())),
                ("elapsedTime", crate::js::EventField::Num(ev.elapsed_s)),
                ("pseudoElement", crate::js::EventField::Str(ev.pseudo)),
            ];
            self.js.dispatch_event(Some(&node), ev.kind, &init);
        }
        for ev in t.events {
            // An element nobody holds any more has nobody to tell.
            let Some(node) = ev.node else { continue };
            let init = [
                ("bubbles", crate::js::EventField::Bool(true)),
                ("animationName", crate::js::EventField::Str(ev.name)),
                ("elapsedTime", crate::js::EventField::Num(ev.elapsed_s)),
                ("pseudoElement", crate::js::EventField::Str(ev.pseudo)),
            ];
            self.js.dispatch_event(Some(&node), ev.kind, &init);
        }
    }
    /// Step 6: when the page has `@keyframes` and its DOM changed, the style pass (the forced layout a
    /// geometry read does) at the page clock; the animations it (or a script's geometry read since the
    /// last update) styled go to the tracker, whose events the next update's step 4 sends.
    fn update_style_layout(&mut self) {
        let generation = crate::js_dom::geom_gen();
        let wanted = self.js.anim.borrow_mut().style_wanted(self.js, generation);
        if wanted {
            crate::anim_eval::set_document_clock(self.now);
            self.js.animation_style_pass();
        }
        let mut a = self.js.anim.borrow_mut();
        if wanted {
            a.styled = Some(crate::js_dom::geom_gen());
            a.dirty_since = None;
        }
        let passes = crate::anim_eval::animation_passes();
        if passes != a.passes {
            a.passes = passes;
            let report = crate::anim_eval::take_animation_report();
            if !report.is_empty() || !a.tracker.is_idle() {
                a.tracker.after_build(0, report, self.now);
                a.next = Some(self.now);
            }
        }
        // QANIMHLb: a transition the pass (or a script's style read) started sends its events at the next update.
        if crate::transition::owes_events() {
            a.next = Some(a.next.map_or(self.now, |n| n.min(self.now)));
        }
        drop(a);
        // QEL2b: the frame's layout for the ResizeObserver step (one layout shared by style, layout and RO).
        self.js.observer_layout();
    }
}

/// Headless: the time of the next rendering update (a virtual 16.7 ms frame boundary), `None` when
/// nothing waits for one: animation frame callbacks, observers (`__scratchy_raf_due`) or CSS
/// animations (QANIMHL).
pub(crate) fn headless_frame_due(js: &JsEngine) -> Option<f64> {
    match (js.raf_due_ms(), headless_anim_due(js)) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Headless: runs the rendering update when its frame is due at `now`. Returns the callbacks run,
/// at least 1 when a frame ran (progress for the pumps), 0 when none was due.
pub(crate) fn headless_frame(js: &JsEngine, now: f64) -> usize {
    match headless_frame_due(js) {
        Some(due) if due <= now => update_the_rendering(&mut Headless { js, now }, Some(now)).max(1),
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
    crate::anim_eval::set_task_clock(Some(now));
    match js.run_due_counted() {
        0 => headless_frame(js, now),
        n => n,
    }
}
