//! QINC1j (R60 sections 3, 9): App frames through the incremental engine, (default since QINC1l; `SCRATCHY_INCR=0` turns it off).
//!
//! One frame (`frame`) decides, in this order:
//! 1. **Current**: the engine's laid tree already shows the page as it is (a script forced layout
//!    since the last frame laid it out, or nothing changed): no style, box or layout work at all; the
//!    App shows the engine's tree (`incr_layout::view`).
//! 2. **Main**: incremental restyle + box update + relayout of the kept trees on the main thread,
//!    installed at once (no worker round trip, no `hold_old_frame`).
//! 3. **Worker**: the big updates go to the shaping worker, laid out the way the engine lays out
//!    (every element `Laid` records its `Ck`), and the engine adopts the result
//!    (`adopt_worker_result`): a copy of the kept box tree when the restyle was incremental but
//!    touched more than `WORKER_SHARE` of the styled elements (and `WORKER_MIN_RESTYLED`) or the relayout could not run (the laid
//!    tree is stale: box tree started over, viewport width, drift guard); a from-scratch build when
//!    the restyle cannot be incremental (first layout, viewport width, new rule-list generation,
//!    Global log entry, arena reset), whose shadow stage starts the kept trees from it.
//!
//! `SCRATCHY_INCR_CHECK=1` (and every test) checks each Main frame: a from-scratch build and layout
//! on the main thread run the engine's shadow comparisons (styles, boxes, laid tree); a mismatch is
//! reported (`incr.check_mismatch`, diag line / test panic) and the from-scratch layout is shown.
//!
//! The hook "a forced layout since the last frame = no work" only reads the engine's state, so it
//! holds whichever path updated the engine (today the forced layout's shadow stage; after QINC1i the
//! forced layout itself).
use super::*;
use crate::incr_box::WorkerJob;

/// Above this share of restyled / styled elements a frame's relayout goes to the worker ...
pub(crate) const WORKER_SHARE: f32 = 0.25;
/// ... when it restyled at least this many elements (a small page's relayout is cheap anyway).
pub(crate) const WORKER_MIN_RESTYLED: u64 = 200;

#[cfg(test)]
thread_local! {
    static LIVE_SEAM: Cell<bool> = const { Cell::new(false) };
}

/// Test seam: App frames use the engine (`SCRATCHY_INCR=1`). Returns the previous value.
#[cfg(test)]
pub(crate) fn set_live(on: bool) -> bool {
    LIVE_SEAM.with(|l| l.replace(on))
}

/// Whether App frames go through the engine (default; `SCRATCHY_INCR=0` off; tests: `set_live`).
pub(crate) fn live() -> bool {
    #[cfg(test)]
    {
        LIVE_SEAM.with(Cell::get)
    }
    #[cfg(not(test))]
    {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| crate::incr_style::engine_enabled())
    }
}

/// QINC1j: notes an App-side input change in the write log (only when frames use the engine).
pub(crate) fn note_ambient(a: crate::incr_log::Ambient) {
    if live() {
        crate::incr_log::note(0, crate::incr_log::Change::Ambient(a));
    }
}

/// What one App frame does.
pub(crate) enum Frame {
    /// Nothing to lay out: show the engine's tree.
    Current,
    /// The engine's tree was updated on the main thread: show it.
    Main,
    /// Check mode found a difference: show this from-scratch layout (the engine starts over).
    Fresh(Laid, ShapeCache),
    /// Lay `tree` out on the worker; with a job, the engine adopts the result.
    Worker(LayoutBox, Option<WorkerJob>),
}

/// The worker's layout of a kept-tree copy, returned for adoption (`AppEvent::ShapeResult`).
pub(crate) struct WorkerResult {
    pub(crate) tree: LayoutBox,
    pub(crate) job: WorkerJob,
    pub(crate) root_h: f32,
    pub(crate) vw: f32,
    pub(crate) vh: f32,
}

/// One App frame (see the module docs). `fs` lays out on the main thread.
pub(crate) fn frame(
    doc: &Handle,
    vw: f32,
    vh: f32,
    css: &str,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
    fs: &mut FontSystem,
) -> Frame {
    use crate::perf::{incr_add, incr_worker_fallback, Incr};
    set_viewport(vw, vh);
    let rules = crate::rules_cache::cached_rules(doc, css);
    let mut reason = "restyle_not_incremental";
    if !crate::incr_style::live_ready(doc, &rules, vw) {
        reason = if crate::incr_box::base().is_none() { "first_layout" } else { "restyle_not_incremental" };
    } else if crate::incr_box::live_ready(doc) {
        if crate::incr_style::is_current(vh) && crate::incr_box::is_current(doc, Some(vh), image_cache) && crate::incr_layout::is_current(vw, vh) {
            // Only App-side notes can be left in the log (their state is unchanged).
            crate::incr_log::take();
            incr_add(Incr::FramesCurrent, 1);
            return Frame::Current;
        }
        match crate::incr_style::live_update(doc, &rules, vw, Some(vh), image_cache) {
            Some(st) if st.share <= WORKER_SHARE || st.restyled < WORKER_MIN_RESTYLED => {
                if crate::incr_layout::live_update(fs, vw, vh) {
                    incr_add(Incr::FramesMain, 1);
                    return check(doc, vw, vh, css, image_cache, fs);
                }
                reason = "relayout_not_incremental";
            }
            Some(_) => reason = "restyled_share",
            None => reason = "restyle_not_incremental",
        }
        if reason != "restyle_not_incremental"
            && let Some((tree, job)) = crate::incr_box::clone_for_worker()
        {
            crate::incr_layout::mark_stale();
            incr_add(Incr::FramesWorker, 1);
            incr_worker_fallback(reason);
            return Frame::Worker(tree, Some(job));
        }
    }
    let tree = build_layout_tree_with_resources(doc, vw, Some(vh), css, image_cache);
    let job = crate::incr_box::job_for_fresh(&tree);
    crate::incr_layout::mark_stale();
    incr_add(Incr::FramesWorker, 1);
    incr_worker_fallback(reason);
    Frame::Worker(tree, job)
}

/// Check mode (tests, `SCRATCHY_INCR_CHECK=1`): a from-scratch build and layout of the same state run
/// the shadow comparisons against the kept trees the frame just updated.
fn check(doc: &Handle, vw: f32, vh: f32, css: &str, image_cache: &HashMap<String, Arc<DecodedImage>>, fs: &mut FontSystem) -> Frame {
    if !crate::incr_style::checking() {
        return Frame::Main;
    }
    let before = crate::perf::incr_get(crate::perf::Incr::CheckMismatch);
    // Its animation pass sees every animated element at the same clock: start times stay.
    let tree = build_layout_tree_with_resources(doc, vw, Some(vh), css, image_cache);
    let mut cache = ShapeCache::new();
    let mut next_id = crate::incr_layout::next_id() + (1 << 20);
    let mut inc = false;
    let far = std::time::Instant::now() + std::time::Duration::from_secs(3600);
    let laid = measure_document(&tree, fs, vw, vh, &mut next_id, &mut cache, far, &mut inc);
    if crate::perf::incr_get(crate::perf::Incr::CheckMismatch) > before {
        crate::incr_layout::mark_stale();
        return Frame::Fresh(laid, cache);
    }
    Frame::Main
}

/// A complete worker result for a job: the engine adopts the worker's box-tree copy and its layout
/// when the kept tree has not changed since the copy. Returns the engine's view version (0: not
/// adopted).
pub(crate) fn adopt_worker_result(r: WorkerResult, laid: &Rc<Laid>, cache: &ShapeCache, next_id: usize) -> u64 {
    let root_style = r.tree.style;
    if !crate::incr_box::adopt_tree(&r.job, r.tree) {
        crate::perf::incr_worker_fallback("adopt_refused");
        return 0;
    }
    crate::incr_layout::adopt(&r.job, root_style, laid.clone(), r.root_h, cache.clone(), next_id, r.vw, r.vh)
}
