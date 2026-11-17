//! QAN1c: the App's bookkeeping of running CSS animations, settle-first.
//! After every build the tracker learns which animations were styled
//! (`AnimRecord`s); `tick` then reports the events due at the app clock
//! (`animationstart`/`iteration`/`end`/`cancel`), whether a restyle is
//! needed because an animation changed phase since the last build, and
//! the next clock time at which something changes. No in-between frames.
use super::*;
use std::collections::BTreeMap;

/// An animation event to dispatch at `node` (already gone = `None`).
pub(crate) struct AnimEvent {
    pub(crate) kind: &'static str,
    pub(crate) node: Option<Handle>,
    pub(crate) name: String,
    pub(crate) elapsed_s: f64,
    pub(crate) pseudo: String,
}

struct Live {
    rec: AnimRecord,
    started: bool,
    ended: bool,
    iter_seen: u32,
    last_iter_ms: Option<f64>,
    /// Phase the last build styled.
    built: AnimPhase,
}

#[derive(Default)]
pub(crate) struct AnimTracker {
    live: BTreeMap<AnimKey, Live>,
    generation: u64,
    pending: Vec<AnimEvent>,
}

#[derive(Default)]
pub(crate) struct AnimTick {
    pub(crate) events: Vec<AnimEvent>,
    pub(crate) restyle: bool,
    /// App clock (ms) of the next change, `None` when nothing is scheduled.
    pub(crate) next: Option<f64>,
}

const ITERATION_CAP_MS: f64 = 1000.0;

fn paused(a: &Animation) -> bool {
    a.play_state == AnimPlayState::Paused
}

fn elapsed(rec: &AnimRecord, now: f64) -> f64 {
    if paused(&rec.anim) { 0.0 } else { (now - rec.start).max(0.0) }
}

fn event(rec: &AnimRecord, kind: &'static str, elapsed_ms: f64) -> AnimEvent {
    AnimEvent {
        kind,
        node: rec.node.upgrade(),
        name: rec.key.3.clone(),
        elapsed_s: elapsed_ms / 1000.0,
        pseudo: rec.key.1.clone().unwrap_or_default(),
    }
}

fn active_duration(a: &Animation) -> f64 {
    let d = a.duration as f64 * a.iterations as f64;
    if d.is_nan() { 0.0 } else { d }
}

impl AnimTracker {
    pub(crate) fn is_idle(&self) -> bool {
        self.live.is_empty() && self.pending.is_empty()
    }

    /// Learns the animations a build (at document clock `build_ms`) styled.
    /// Started animations that are no longer styled are cancelled.
    pub(crate) fn after_build(&mut self, generation: u64, report: Vec<AnimRecord>, build_ms: f64) {
        if generation != self.generation {
            self.live.clear();
            self.pending.clear();
            self.generation = generation;
        }
        let mut present = std::collections::HashSet::new();
        for rec in report {
            let built = phase_of(&rec.anim, elapsed(&rec, build_ms));
            present.insert(rec.key.clone());
            match self.live.get_mut(&rec.key) {
                Some(l) => {
                    l.rec = rec;
                    l.built = built;
                }
                None => {
                    self.live.insert(rec.key.clone(), Live { rec, started: false, ended: false, iter_seen: 0, last_iter_ms: None, built });
                }
            }
        }
        let gone: Vec<AnimKey> = self.live.keys().filter(|k| !present.contains(*k)).cloned().collect();
        for k in gone {
            if let Some(l) = self.live.remove(&k)
                && l.started
                && !l.ended
            {
                let active = (build_ms - l.rec.start - l.rec.anim.delay as f64).clamp(0.0, active_duration(&l.rec.anim).max(0.0));
                self.pending.push(event(&l.rec, "animationcancel", active));
            }
        }
    }

    /// Events due at app clock `now`, restyle need, next deadline.
    pub(crate) fn tick(&mut self, now: f64) -> AnimTick {
        let mut out = AnimTick { events: std::mem::take(&mut self.pending), ..Default::default() };
        let mut next: Option<f64> = None;
        let soon = |t: f64, next: &mut Option<f64>| *next = Some(next.map_or(t, |n: f64| n.min(t)));
        for l in self.live.values_mut() {
            let a = l.rec.anim.clone();
            let el = elapsed(&l.rec, now);
            let phase = phase_of(&a, el);
            let delay = a.delay as f64;
            let dur = a.duration as f64;
            if !l.started && phase != AnimPhase::Before {
                l.started = true;
                out.events.push(event(&l.rec, "animationstart", (-delay).max(0.0).min(active_duration(&a))));
            }
            if phase == AnimPhase::Running && a.iterations.is_finite() && a.iterations > 1.0 && dur > 0.0 {
                let cur = ((el - delay) / dur).floor() as u32;
                if cur > l.iter_seen {
                    if l.last_iter_ms.is_none_or(|t| now - t >= ITERATION_CAP_MS) {
                        out.events.push(event(&l.rec, "animationiteration", cur as f64 * dur));
                        l.last_iter_ms = Some(now);
                    }
                    l.iter_seen = cur;
                }
            }
            if !l.ended && phase == AnimPhase::After {
                l.ended = true;
                out.events.push(event(&l.rec, "animationend", active_duration(&a)));
            }
            if phase != l.built {
                out.restyle = true;
                l.built = phase;
            }
            if paused(&a) || l.ended {
                continue;
            }
            let base = l.rec.start + delay;
            match phase {
                AnimPhase::Before => soon(base, &mut next),
                AnimPhase::Running => {
                    if a.iterations.is_finite() {
                        let mut t = base + active_duration(&a);
                        if a.iterations > 1.0 && dur > 0.0 {
                            let after_cap = l.last_iter_ms.map_or(0.0, |x| x + ITERATION_CAP_MS);
                            let b = ((l.iter_seen + 1) as f64).max(((after_cap - base) / dur).ceil());
                            if b < a.iterations as f64 {
                                t = t.min(base + b * dur);
                            }
                        }
                        soon(t, &mut next);
                    }
                }
                AnimPhase::After => {}
            }
        }
        out.next = next;
        out
    }
}
