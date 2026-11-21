//! QINC1i (R60 sections 3, 9): script forced layouts through the incremental engine.
//!
//! With `SCRATCHY_INCR=1` (or `SCRATCHY_INCR_CHECK=1`) a `GeomCache` refresh no longer builds and
//! measures the page from scratch: it runs ONE engine update (QINC1f restyle from the write log,
//! QINC1g box tree update, QINC1h1 relayout of the kept laid tree) and reads its rects and glyphs
//! from the engine's kept laid tree. The first update of a document (and one after a global
//! restyle: a new rule list, a viewport width, a `Global` log entry) builds from scratch with the
//! shadow update on, which starts the kept box tree again, then lays it out through the engine.
//!
//! API (next to QINC1j's `incr_frame`):
//! - [`forced_on`]: whether script forced layouts go through the engine.
//! - [`update`]: bring the engine's laid tree up to date with the DOM (no from-scratch layout), through
//!   the same checks and update as an App frame (`incr_frame`, QINC1j: `live_ready`, `is_current`,
//!   `live_update`) and a complete relayout on the main thread (`incr_layout::update_kept`).
//! - `incr_layout::with_laid` / `touched_roots`: read the result.
//! - [`sync_window_images`] / [`window_images`]: the window's decoded images, so forced layouts lay
//!   images out at their real size (user decision, R60 open question 2), exactly as the App frame
//!   does: the App hands its image cache over at every build of its own.
//!
//! Check mode (tests with the seam on, `SCRATCHY_INCR_CHECK=1`): `GeomCache` also lays the page out
//! from scratch (shadow update off) and compares (`cmp_laid`); a difference is reported
//! (`incr-check: MISMATCH`, `incr.check_mismatch`), every stage starts over and the from-scratch
//! result is used.
use super::*;
#[cfg(test)]
use std::cell::Cell;
use std::cell::RefCell;
use std::sync::Arc;

type Images = HashMap<String, Arc<DecodedImage>>;

thread_local! {
    /// The window's decoded images as of its last build: (the document, images). QFLK6: the document is
    /// a weak handle compared by identity. A bare node address is not an identity (a new document can
    /// reuse a dropped one's address); the weak handle keeps the allocation, so no new node can take it.
    static WINDOW_IMAGES: RefCell<(Option<WeakHandle>, Rc<Images>)> = RefCell::new((None, Rc::new(HashMap::new())));
    /// Test seam: `Some` forces script forced layouts through the engine (or not).
    #[cfg(test)]
    static SEAM: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Whether script forced layouts go through the engine (default; `SCRATCHY_INCR=0` turns it off; `SCRATCHY_INCR_CHECK=1` checks).
/// Tests: off unless the seam says so; a test whose thread name contains `incr_forced` (the
/// `*_incr_forced_tests` modules, which run whole suites a second time) has it on regardless. QINC1l:
/// on by default like shipping; `SCRATCHY_TEST_INCR_FORCED=0` runs the suite with the engine off
/// (the old full-relayout path; CI runs it that way too).
pub(crate) fn forced_on() -> bool {
    #[cfg(not(test))]
    {
        crate::incr_style::running()
    }
    #[cfg(test)]
    {
        static ENV_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let env_off = || *ENV_ON.get_or_init(|| std::env::var("SCRATCHY_TEST_INCR_FORCED").is_ok_and(|v| v == "0"));
        let on = SEAM.with(Cell::get).unwrap_or_else(|| !env_off() || std::thread::current().name().is_some_and(|n| n.contains("incr_forced")));
        on && crate::incr_style::running()
    }
}

/// Test seam: force the engine path on or off for this thread (`None`: by the thread's name).
#[cfg(test)]
pub(crate) fn set_forced_seam(on: Option<bool>) -> Option<bool> {
    SEAM.with(|s| s.replace(on))
}

/// Whether `w` is `doc` (and still alive).
fn is_doc(w: &Option<WeakHandle>, doc: &Handle) -> bool {
    w.as_ref().and_then(|w| w.upgrade()).is_some_and(|d| Rc::ptr_eq(&d, doc))
}

/// The App's image cache, handed over at each of its builds of `doc` (a no-op unless the engine path
/// is on; the map is copied only when an entry was added, replaced or removed).
pub(crate) fn sync_window_images(doc: &Handle, cache: &Images) {
    if !forced_on() {
        return;
    }
    WINDOW_IMAGES.with(|w| {
        let mut w = w.borrow_mut();
        let same = is_doc(&w.0, doc) && w.1.len() == cache.len() && cache.iter().all(|(k, v)| w.1.get(k).is_some_and(|x| Arc::ptr_eq(x, v)));
        if !same {
            *w = (Some(Rc::downgrade(doc)), Rc::new(cache.clone()));
        }
    });
}

/// QINC1k6: one image arrived (`handle_image_fetched`), before its `load` event fires: add it to the
/// window's images incrementally (full sync when the window has not built this document yet), so a
/// forced layout in a `load` handler lays the image out at its decoded size.
pub(crate) fn note_window_image(doc: &Handle, cache: &Images, src: &str, image: &Arc<DecodedImage>) {
    if !forced_on() {
        return;
    }
    let known = WINDOW_IMAGES.with(|w| {
        let mut w = w.borrow_mut();
        if !is_doc(&w.0, doc) {
            return false;
        }
        Rc::make_mut(&mut w.1).insert(src.to_string(), image.clone());
        true
    });
    if !known {
        sync_window_images(doc, cache);
    }
    // The cached rects (keyed on the write generation) were laid out without this image.
    crate::js_dom::geom_bump_image();
}

/// The images a forced layout of `doc` lays out with: the window's, when it last built this document
/// (headless and other documents: none).
pub(crate) fn window_images(doc: &Handle) -> Rc<Images> {
    WINDOW_IMAGES.with(|w| {
        let w = w.borrow();
        if is_doc(&w.0, doc) { w.1.clone() } else { Rc::new(HashMap::new()) }
    })
}

/// Brings the engine's laid tree up to date with `doc` at `vw` x `vh` (the caller has set the CSS
/// viewport). False when the engine could not produce a tree (the caller lays out from scratch).
pub(crate) fn update(doc: &Handle, vw: f32, vh: f32, extra_css: &str, images: &Images, fs: &mut FontSystem) -> bool {
    use crate::perf::{incr_add, Incr};
    let rules = crate::rules_cache::cached_rules(doc, extra_css);
    let mut incremental = false;
    // The App frame's checks and update (QINC1j), shared: nothing changed since the engine's last
    // update (an App frame or another forced layout laid it out) = no work at all.
    if crate::incr_style::live_ready(doc, &rules, vw) && crate::incr_box::live_ready(doc) {
        // Check mode updates anyway: its reconcile also sees writes that bypass the log (the parser,
        // Rust-side writes in tests), which the oracle would otherwise report as a mismatch.
        if !crate::incr_style::checking()
            && crate::incr_style::is_current(vh)
            && crate::incr_box::is_current(doc, Some(vh), images)
            && crate::incr_layout::is_current(vw, vh)
        {
            crate::incr_log::take();
            incr_add(Incr::ForcedEngine, 1);
            return true;
        }
        // QOBS3 phases: the engine's restyle + box update count as `incr_shadow`, its relayout as
        // `measure_document` (a from-scratch fallback build records its own build phases).
        let t_phase = crate::perf::phase(crate::perf::Phase::IncrShadow);
        incremental = crate::incr_style::live_update(doc, &rules, vw, Some(vh), images).is_some();
        drop(t_phase);
    }
    if !incremental {
        // First layout of the document or a global restyle: a from-scratch build with the shadow
        // update on (re)starts the kept box tree; the engine lays it out below.
        incr_add(Incr::ForcedRestarts, 1);
        drop(build_layout_tree_with_resources(doc, vw, Some(vh), extra_css, images));
        if !crate::incr_box::live_ready(doc) {
            return false;
        }
    }
    let t_phase = crate::perf::phase(crate::perf::Phase::MeasureDocument);
    let ok = crate::incr_layout::update_kept(fs, vw, vh);
    drop(t_phase);
    incr_add(Incr::ForcedEngine, u64::from(ok));
    ok
}
