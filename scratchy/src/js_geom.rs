//! QJ4: geometry reads for page script (`getBoundingClientRect`,
//! `offset*`, `client*`, `scroll*`). The JS engine cannot see `App`, so a
//! read with a stale geometry generation runs style + layout for the
//! current DOM itself (a browser's "forced synchronous layout"): the tree is
//! built with `build_layout_tree_with_resources` and measured with
//! `measure_document` on the JS thread (own `FontSystem`, own `ShapeCache`),
//! then every laid-out element box is recorded by node id. The result is
//! cached until the next DOM write (`DomTable` write generation), so a
//! script that only reads costs one layout; a read/write loop costs one
//! layout per read, which is counted and logged once.
//!
//! QINC1i: a read updates the incremental engine (`incr_engine.rs`, `refresh_engine`): no
//! from-scratch build, rects from the engine's kept laid tree, glyphs on the first text read, the
//! window's decoded images. The from-scratch layout below is the fallback when the engine has no
//! tree (QINC1m removed the old path's QYTC1a / QYTC1d / HiddenSnap / ATTR_SNAP skips and the
//! App-layout IntersectionObserver source, which the engine bypassed).
//!
//! Limits (from scratch): images are laid out without their decoded size (only
//! `width`/`height` attributes and CSS count), the viewport is the one the
//! JS window reports, inline (non-box) elements get the union of their
//! element descendants or else their nearest box ancestor, and `:hover`-type
//! state is not applied.
use super::*;

/// One element's box in document space.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct GeomRect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
    /// Inside a `position: fixed` subtree: viewport space, not scrolled.
    pub(crate) fixed: bool,
    /// Bottom/right of the box and everything it paints (`Laid::ink`).
    pub(crate) ink_r: f32,
    pub(crate) ink_b: f32,
}

/// QWPT4a: this thread's forced-layout font system, made now with `fonts` (font file bytes, e.g.
/// WPT's Ahem) loaded on top of the system fonts. Returns how many faces were added.
pub(crate) fn install_extra_fonts(fonts: Vec<Vec<u8>>) -> usize {
    let mut fs = new_font_system();
    let before = fs.db().len();
    for f in fonts {
        fs.db_mut().load_font_data(f);
    }
    let added = fs.db().len() - before;
    FONTS.with(|f| *f.borrow_mut() = Some(fs));
    FONTS_SYNC.with(|s| *s.borrow_mut() = crate::webfonts::FontSync::default());
    added
}

/// QFF1a: brings this thread's forced-layout font system up to the web-font registry (when it exists).
pub(crate) fn sync_forced_fonts() {
    FONTS.with(|f| {
        if let Some(fs) = f.borrow_mut().as_mut() {
            // QFF1a: web faces came or went: the engine's kept shapes used other fonts.
            if FONTS_SYNC.with(|s| crate::webfonts::sync(fs, &mut s.borrow_mut())) {
                crate::incr_style::drop_engines();
            }
        }
    });
}

thread_local! {
    /// Test-only: how many forced layouts (cache misses) ran on this thread.
    #[cfg(test)]
    pub(crate) static GEOM_LAYOUTS: Cell<usize> = const { Cell::new(0) };
    /// QINC1i5 test seam: engine forced layouts skip the check-mode oracle (a from-scratch layout that
    /// would also refill the shape memo, hiding what the engine path alone leaves in it).
    #[cfg(test)]
    pub(crate) static NO_ORACLE: Cell<bool> = const { Cell::new(false) };
    static FONTS: RefCell<Option<FontSystem>> = const { RefCell::new(None) };
    /// QFF1a: `FONTS`' position in the web-font log.
    static FONTS_SYNC: RefCell<crate::webfonts::FontSync> = RefCell::new(crate::webfonts::FontSync::default());
}

#[derive(Default)]
pub(crate) struct GeomCache {
    key: Option<(u64, u32, u32, (u64, u64))>,
    /// QFF1a: the web-font registry generation of the last layout (a face that arrived since lays out again).
    font_gen: u64,
    /// QANIMHLc: document clock (bits) of the layout when it styled a running animation or
    /// transition; such a layout is only fresh at that clock. `None`: nothing was animated, the key alone decides.
    anim_clock: Option<u64>,
    /// QP2: viewport of the previous refresh; a change clears the shape memo.
    last_vp: Option<(u32, u32)>,
    rects: HashMap<usize, GeomRect>,
    /// Size of the laid-out document (the root box).
    page: (f32, f32),
    forced: u32,
    forced_ms: u128,
    /// QY53e1: thread CPU ms of the most recent forced layout (0 before the first).
    last_cpu_ms: f64,
    logged: bool,
    /// QJ7: every laid-out glyph by the DOM text node it came from, in
    /// layout order: `(char, x, y, w, h)`, document space, `y`/`h` of the line.
    glyphs: HashMap<usize, Vec<Glyph>>,
    /// QINC1b (R60 4.4): next unused paragraph id; never restarts.
    next_para_id: usize,
    /// QINC1b: persists per owner, swept of ids no live `Laid` uses (none is kept here).
    shape_cache: ShapeCache,
    /// QINC1i: the rects came from the engine's kept laid tree; `glyphs` are collected from it on the
    /// first `text_rects` read (R60 9: lazy glyphs).
    glyphs_lazy: bool,
}

/// One laid-out character: `(char, x, y, w, h)` in document space.
type Glyph = (char, f32, f32, f32, f32);

/// QJ7: records the glyphs of every paragraph under `l`, keyed by DOM text node.
fn walk_glyphs(l: &Laid, cache: &ShapeCache, out: &mut HashMap<usize, Vec<Glyph>>) {
    crate::stack::grow(|| walk_glyphs_body(l, cache, out)) // QSTK1x: recursive per nesting level
}

fn walk_glyphs_body(l: &Laid, cache: &ShapeCache, out: &mut HashMap<usize, Vec<Glyph>>) {
    fn para(cache: &ShapeCache, id: usize, ox: f32, oy: f32, out: &mut HashMap<usize, Vec<Glyph>>) {
        let Some(entry) = cache.get(&id) else { return };
        let guard = entry.lock().unwrap();
        let (buffer, _, _, _, _, _, _, spans) = &*guard;
        for run in buffer.layout_runs() {
            for g in run.glyphs {
                let tnode = spans.get(g.metadata).map_or(0, |s| s.2);
                if tnode == 0 {
                    continue;
                }
                let Some(text) = run.text.get(g.start..g.end) else { continue };
                let n = text.chars().count().max(1) as f32;
                let v = out.entry(tnode).or_default();
                for (i, c) in text.chars().enumerate() {
                    v.push((c, ox + g.x + g.w * i as f32 / n, oy + run.line_top, g.w / n, run.line_height));
                }
            }
        }
    }
    match &l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().for_each(|c| walk_glyphs(c, cache, out)),
        LaidContent::Text(id) => {
            let cx = l.x + l.style.border_left() + l.style.padding.left.px();
            let cy = l.y + l.style.border_top() + l.style.padding.top.px();
            para(cache, *id, cx, cy, out);
        }
        LaidContent::MixedInline(items) => {
            for it in items {
                match it {
                    MixedPositioned::Embedded(b) => walk_glyphs(b, cache, out),
                    MixedPositioned::Word { id, x, y, .. } => para(cache, *id, *x, *y, out),
                }
            }
        }
        LaidContent::Image(_) => {}
    }
    l.positioned.iter().for_each(|p| walk_glyphs(p, cache, out));
}

/// Records every element box under `l`; returns the (right, bottom) of the
/// union of `l` and all its descendant boxes (the scrollable overflow
/// extent; `Laid::ink` is clipped by `overflow: hidden`, so it can't be used).
fn walk(l: &Laid, fixed: bool, out: &mut HashMap<usize, GeomRect>) -> (f32, f32) {
    let fixed = fixed || l.fixed;
    let mut ext = (l.x + l.w, l.y + l.h);
    // The outermost box of an element wins (a wrapper may share its node).
    let own = l.style.node != 0 && !out.contains_key(&l.style.node);
    if own {
        out.insert(l.style.node, GeomRect { x: l.x, y: l.y, w: l.w, h: l.h, fixed, ink_r: ext.0, ink_b: ext.1 });
        if crate::ro_geom::collecting() {
            crate::ro_geom::note(l); // QEL2b
        }
    }
    let mut sub = |c: &Laid, ext: &mut (f32, f32)| {
        let e = crate::stack::grow(|| walk(c, fixed, out)); // QSTK1x: deep recursion
        ext.0 = ext.0.max(e.0);
        ext.1 = ext.1.max(e.1);
    };
    match &l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().for_each(|c| sub(c, &mut ext)),
        LaidContent::MixedInline(items) => {
            for it in items {
                if let MixedPositioned::Embedded(b) = it {
                    sub(b, &mut ext);
                }
            }
        }
        _ => {}
    }
    l.positioned.iter().for_each(|p| sub(p, &mut ext));
    if own && let Some(r) = out.get_mut(&l.style.node) {
        r.ink_r = ext.0;
        r.ink_b = ext.1;
    }
    ext
}

/// QP2/QY53o3a: safety cap on the paragraphs the JS thread's memo keeps AFTER a layout's sweep
/// dropped what it did not use (a layout's own entries are never capped: 7.5k on Wikipedia).
pub(crate) const SHAPE_MEMO_CAP: usize = 32768;

impl GeomCache {
    /// QOBS7: `((rect entries, bytes), (glyph entries = text nodes, bytes))`, by capacity.
    pub(crate) fn mem_estimate(&self) -> ((usize, usize), (usize, usize)) {
        let rects = (
            self.rects.len(),
            self.rects.capacity() * (std::mem::size_of::<(usize, GeomRect)>() + 1),
        );
        let glyphs = (
            self.glyphs.len(),
            self.glyphs.capacity() * (std::mem::size_of::<(usize, Vec<Glyph>)>() + 1) + self.glyphs.values().map(|v| v.capacity() * std::mem::size_of::<Glyph>()).sum::<usize>(),
        );
        (rects, glyphs)
    }

    /// QY53e1: would a read at this viewport be served without a layout? Cheap: no layout, no CSS string.
    pub(crate) fn is_fresh(&self, wgen: u64, vw: f32, vh: f32) -> bool {
        self.key == Some((wgen, vw as u32, vh as u32, crate::js_style::css_ver())) && self.clock_ok()
    }

    /// QANIMHLc: a layout that styled a running animation or transition is only the page's at the
    /// clock it was made at (a headless frame or an App build moves the clock without a DOM write).
    fn clock_ok(&self) -> bool {
        self.anim_clock.is_none_or(|c| c == crate::anim_eval::read_clock_ms().to_bits())
    }

    fn note_anim_clock(&mut self) {
        self.anim_clock = crate::anim_eval::layout_animated().then(|| crate::anim_eval::document_clock_ms().to_bits());
    }

    /// QY53e1: CPU ms the last forced layout took.
    pub(crate) fn last_cost_ms(&self) -> f64 {
        // QFLK2: the IO recheck deadline is 4x this (min 100 ms); tests that assert delivery times pin it.
        #[cfg(test)]
        if let Some(c) = TEST_FIXED_COST_MS.with(|c| c.get()) {
            return c;
        }
        self.last_cpu_ms
    }

    /// QANIMHL: the headless rendering update's style + layout step: lays the page out when it changed
    /// since the last read (the same work and cache as a geometry read).
    pub(crate) fn style_pass(&mut self, page: &Handle, wgen: u64, vw: f32, vh: f32) {
        self.refresh(page, wgen, vw, vh);
    }

    /// QEL2b: the ResizeObserver step needs the boxes of targets the last layout did not record (a
    /// new `observe()`, or the recorded boxes are of an older state): lay out when the page changed, and
    /// once more when even a current layout did not cover them (engine: a no-op update + walk).
    pub(crate) fn ro_pass(&mut self, page: &Handle, wgen: u64, vw: f32, vh: f32) {
        self.refresh(page, wgen, vw, vh);
        if !crate::ro_geom::current(page, vw as u32, vh as u32) {
            self.key = None;
            self.refresh(page, wgen, vw, vh);
        }
    }

    /// Lays the page out again unless nothing changed since the last read.
    fn refresh(&mut self, page: &Handle, wgen: u64, vw: f32, vh: f32) {
        // QY53h: keyed on the CSS version counters; the CSS string is only built on a miss.
        let key = (wgen, vw as u32, vh as u32, crate::js_style::css_ver());
        if self.key == Some(key) && self.clock_ok() && self.font_gen == crate::webfonts::generation() {
            return;
        }
        self.layout_now(page, vw, vh, key);
        // QFF1a: after the layout: its own rule publish (a first parse) does not make it stale.
        self.font_gen = crate::webfonts::generation();
    }

    fn layout_now(&mut self, page: &Handle, vw: f32, vh: f32, key: (u64, u32, u32, (u64, u64))) {
        let wgen = key.0;
        #[cfg(test)]
        GEOM_LAYOUTS.with(|c| c.set(c.get() + 1));
        // QANIMHLc: the last layout styled a running animation: this one is at the clock of the read
        // (headless frames are not added to move it, so the document clock may be a frame or more behind).
        if self.anim_clock.is_some() {
            crate::anim_eval::set_document_clock(crate::anim_eval::read_clock_ms());
        }
        // QINC1i: through the incremental engine; from scratch when it has no tree.
        if self.refresh_engine(page, vw, vh, key) {
            return;
        }
        let css = crate::js_style::page_css();
        // One-off cost (system font scan), kept out of the forced-layout stats.
        FONTS.with(|f| {
            f.borrow_mut().get_or_insert_with(new_font_system);
        });
        // QP2: content-keyed shape memo on this thread (enabled once). Entries
        // are keyed by text + styles + width, so a DOM write can never serve a
        // stale shape; a resize drops them (old widths), and the size is capped.
        crate::layout::SHAPE_MEMO_ENABLED.with(|m| m.set(true));
        let vp = (vw as u32, vh as u32);
        if self.last_vp != Some(vp) {
            crate::layout::memo_clear();
        }
        self.last_vp = Some(vp);
        crate::layout::memo_track_begin();
        let t0 = std::time::Instant::now();
        // QOBS3: the phase timers and counters run only while the recorder is on.
        let recording = crate::perf::begin_forced();
        let cpu0 = crate::js::thread_cpu_nanos();
        let prev_vp = current_viewport();
        // A layout bug must not take the whole browser down with a page
        // script: on a panic the page simply has no geometry (all zeros).
        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let tree = build_layout_tree_with_resources(page, vw, Some(vh), &css, &HashMap::new());
            let t_phase = crate::perf::phase(crate::perf::Phase::MeasureDocument);
            let cache = &mut self.shape_cache;
            let id_base = self.next_para_id;
            let end_id = &mut self.next_para_id;
            let mut laid = None;
            // Shaping is progressive (a deadline per pass); keep going until
            // it completes or a hard cap, resuming from the same shape cache.
            #[allow(unused_mut)]
            let (mut pass_ms, mut cap_s) = (500u64, 10u64);
            // QFLK1: tests that need the whole layout (not a time-boxed part of it) lift the wall-clock limits.
            #[cfg(test)]
            if TEST_NO_LAYOUT_LIMITS.with(|c| c.get()) {
                (pass_ms, cap_s) = (600_000, 600);
            }
            FONTS.with(|f| {
                let mut f = f.borrow_mut();
                let fs = f.get_or_insert_with(new_font_system);
                // QFF1a: web faces came or went: the engine's kept shapes used other fonts.
                if FONTS_SYNC.with(|s| crate::webfonts::sync(fs, &mut s.borrow_mut())) {
                    crate::incr_style::drop_engines();
                }
                loop {
                    let mut incomplete = false;
                    let mut next_id = id_base;
                    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(pass_ms);
                    laid = Some(measure_document(&tree, fs, vw, vh, &mut next_id, cache, deadline, &mut incomplete));
                    *end_id = (*end_id).max(next_id);
                    if !incomplete || t0.elapsed() > std::time::Duration::from_secs(cap_s) {
                        break;
                    }
                }
            });
            drop(tree);
            drop(t_phase);
            let t_phase = crate::perf::phase(crate::perf::Phase::GlyphWalk);
            let mut glyphs = HashMap::new();
            if let Some(l) = &laid {
                walk_glyphs(l, cache, &mut glyphs);
            }
            drop(t_phase);
            (laid, glyphs)
        }));
        set_viewport(prev_vp.0, prev_vp.1);
        crate::layout::memo_sweep(SHAPE_MEMO_CAP, 1);
        let t_phase = crate::perf::phase(crate::perf::Phase::RectWalk);
        let prev = std::mem::take(&mut self.rects);
        self.page = (0.0, 0.0);
        self.glyphs.clear();
        self.glyphs_lazy = false;
        match done {
            Ok((Some(l), glyphs)) => {
                self.page = (crate::layout::doc_scroll_width(&l), l.h); // QXSCR1: the scroll width
                // QEL2b: the walk also records the ResizeObserver targets' boxes.
                let ro = crate::ro_geom::begin_forced();
                walk(&l, false, &mut self.rects);
                if ro {
                    crate::ro_geom::end_forced(page, (wgen, key.3), vp.0, vp.1);
                }
                self.glyphs = glyphs;
            }
            Ok((None, ..)) => {}
            Err(_) => diag!("scratchy: js: layout panicked while reading geometry; element rects read as zero"),
        }
        // QINC1b: the laid tree is gone, so no id is live: the sweep empties the cache (as the
        // per-layout cache it replaces was dropped here).
        crate::layout::sweep_shape_cache(&mut self.shape_cache, &std::collections::HashSet::new());
        // QOBS3: did this layout change any box? (only compared while recording)
        let no_change = recording && self.key.is_some() && prev.len() == self.rects.len() && prev.iter().all(|(k, v)| self.rects.get(k) == Some(v));
        drop(prev);
        drop(t_phase);
        // The layout is the browser's work, not the script's: don't charge it
        // to the script's CPU budget (it would trip it on a big page).
        self.last_cpu_ms = crate::js::thread_cpu_nanos().saturating_sub(cpu0) as f64 / 1e6;
        let d = crate::js::pause_budget_since(cpu0);
        if recording {
            crate::perf::end_forced();
            crate::perf::forced_detail(d, no_change);
        }
        self.key = Some(key);
        self.note_anim_clock();
        self.forced += 1;
        self.forced_ms += t0.elapsed().as_millis();        if !self.logged && (self.forced >= 20 || self.forced_ms >= 1000) {
            self.logged = true;
            diag!("scratchy: js: {} forced layouts, {} ms (script reads geometry after DOM writes)", self.forced, self.forced_ms);
        }
    }

    /// QINC1i (R60 9): `refresh` through the incremental engine: one engine update (restyle from the
    /// write log, box tree update, relayout of the kept laid tree) with the window's images, rects
    /// from the kept laid tree, glyphs on the first `text_rects` read. Check mode (tests,
    /// `SCRATCHY_INCR_CHECK=1`) also lays the page out from scratch and compares; a difference is
    /// reported, every stage starts over and the from-scratch result is used. False when the engine
    /// produced no tree (nothing changed here: the caller lays out from scratch).
    fn refresh_engine(&mut self, page: &Handle, vw: f32, vh: f32, key: (u64, u32, u32, (u64, u64))) -> bool {
        let css = crate::js_style::page_css();
        FONTS.with(|f| {
            f.borrow_mut().get_or_insert_with(new_font_system);
        });
        crate::layout::SHAPE_MEMO_ENABLED.with(|m| m.set(true));
        let vp = (vw as u32, vh as u32);
        if self.last_vp != Some(vp) {
            crate::layout::memo_clear();
        }
        self.last_vp = Some(vp);
        // QINC1i5: no "used by this layout" sweep here: an engine update re-measures only what changed,
        // so its used set is a few keys, and sweeping the rest emptied the memo the App frames'
        // relayouts (same thread) shape from (Wikipedia: layout_ms 1.2 s -> 5.8 s). Trimmed below instead.
        let t0 = std::time::Instant::now();
        let recording = crate::perf::begin_forced();
        let cpu0 = crate::js::thread_cpu_nanos();
        let prev_vp = current_viewport();
        let images = crate::incr_engine::window_images(page);
        let checking = crate::incr_style::checking();
        #[cfg(test)]
        let checking = checking && !NO_ORACLE.with(Cell::get);
        let cache = &mut self.shape_cache;
        let next_para = &mut self.next_para_id;
        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            set_viewport(vw, vh);
            FONTS.with(|f| {
                let mut f = f.borrow_mut();
                let fs = f.get_or_insert_with(new_font_system);
                // QFF1a: web faces came or went: the engine's kept shapes used other fonts.
                if FONTS_SYNC.with(|s| crate::webfonts::sync(fs, &mut s.borrow_mut())) {
                    crate::incr_style::drop_engines();
                }
                let ok = crate::incr_engine::update(page, vw, vh, &css, &images, fs);
                // QINC1n: one engine update in `ENGINE_CHECK_EVERY` is checked in daily use too.
                let sampled = ok && crate::incr_evidence::sample();
                // The oracle: today's from-scratch layout, without the shadow update (it would update
                // the engine a second time).
                let fresh = (ok && (checking || sampled)).then(|| {
                    crate::incr_style::without_shadow(|| {
                        let tree = build_layout_tree_with_resources(page, vw, Some(vh), &css, &images);
                        let mut next_id = *next_para;
                        let mut inc = false;
                        let far = std::time::Instant::now() + std::time::Duration::from_secs(3600);
                        let t_phase = crate::perf::phase(crate::perf::Phase::MeasureDocument);
                        let l = measure_document(&tree, fs, vw, vh, &mut next_id, cache, far, &mut inc);
                        drop(t_phase);
                        *next_para = (*next_para).max(next_id);
                        l
                    })
                });
                (ok, fresh, sampled)
            })
        }));
        set_viewport(prev_vp.0, prev_vp.1);
        crate::layout::memo_trim(SHAPE_MEMO_CAP);
        let (ok, fresh, sampled) = match done {
            Ok(r) => r,
            Err(_) => {
                diag!("scratchy: js: incremental layout panicked while reading geometry; laying out from scratch");
                crate::incr_style::drop_engines();
                crate::incr_evidence::fallback(page, "panic");
                (false, None, false)
            }
        };
        if !ok {
            crate::layout::sweep_shape_cache(&mut self.shape_cache, &std::collections::HashSet::new());
            return false;
        }
        let t_phase = crate::perf::phase(crate::perf::Phase::RectWalk);
        let prev = std::mem::take(&mut self.rects);
        self.page = (0.0, 0.0);
        self.glyphs.clear();
        self.glyphs_lazy = false;
        // (box path, field) of the first difference; the check-mode report has the values too.
        let found = fresh.as_ref().and_then(|f| crate::incr_evidence::compare(page, f, &self.shape_cache));
        let mismatch = (checking && found.is_some()).then(|| {
            let m = fresh.as_ref().and_then(|f| {
                let names = crate::incr_cmp::Names::from_dom(page);
                crate::incr_layout::with_laid(|l, c| crate::incr_cmp::cmp_laid(&names, "root", f, &self.shape_cache, l, c).err().map(|m| m.to_string())).flatten()
            });
            format!("laid {} {}", m.unwrap_or_else(|| "no kept laid tree".into()), crate::incr_layout::describe_last())
        });
        #[cfg(test)]
        let found = found.or_else(|| (sampled && crate::incr_evidence::forced_mismatch()).then(|| ("root".to_string(), "test hook".to_string())));
        if sampled && let Some((path, field)) = &found {
            crate::incr_evidence::mismatch(page, path, field);
        }
        match (&found, &fresh) {
            (Some(_), Some(l)) => {
                // Not trusted any more: every stage starts over; this read uses the from-scratch layout.
                crate::incr_style::drop_engines();
                self.page = (crate::layout::doc_scroll_width(l), l.h); // QXSCR1: the scroll width
                let ro = crate::ro_geom::begin_forced();
                walk(l, false, &mut self.rects);
                if ro {
                    crate::ro_geom::end_forced(page, (key.0, key.3), vp.0, vp.1);
                }
                walk_glyphs(l, &self.shape_cache, &mut self.glyphs);
            }
            _ => {
                let rects = &mut self.rects;
                self.page = crate::incr_layout::with_laid(|l, _| {
                    let ro = crate::ro_geom::begin_forced();
                    walk(l, false, rects);
                    if ro {
                        crate::ro_geom::end_forced(page, (key.0, key.3), vp.0, vp.1);
                    }
                    (crate::layout::doc_scroll_width(l), l.h)
                })
                .unwrap_or((0.0, 0.0));
                self.glyphs_lazy = true;
            }
        }
        drop(fresh);
        crate::layout::sweep_shape_cache(&mut self.shape_cache, &std::collections::HashSet::new());
        let no_change = recording && self.key.is_some() && prev.len() == self.rects.len() && prev.iter().all(|(k, v)| self.rects.get(k) == Some(v));
        drop(prev);
        drop(t_phase);
        self.last_cpu_ms = crate::js::thread_cpu_nanos().saturating_sub(cpu0) as f64 / 1e6;
        let d = crate::js::pause_budget_since(cpu0);
        if recording {
            crate::perf::end_forced();
            crate::perf::forced_detail(d, no_change);
        }
        self.key = Some(key);
        self.note_anim_clock();
        self.forced += 1;
        self.forced_ms += t0.elapsed().as_millis();
        if !self.logged && (self.forced >= 20 || self.forced_ms >= 1000) {
            self.logged = true;
            diag!("scratchy: js: {} forced layouts, {} ms (script reads geometry after DOM writes)", self.forced, self.forced_ms);
        }
        if let Some(m) = mismatch {
            crate::incr_style::report_mismatches("forced layout through the engine differs from the from-scratch layout", vec![m]);
        }
        true
    }

    /// `[x, y, w, h, fixed, ink_r, ink_b, page_w, page_h]` of element `h` in
    /// document space; empty when it is not connected to the page or has no
    /// box at all (the caller has already ruled out `display:none`).
    pub(crate) fn rect_of(&mut self, page: &Handle, wgen: u64, h: &Handle, vw: f32, vh: f32) -> Vec<f64> {
        let mut top = h.clone();
        while let Some(p) = parent_handle(&top) {
            top = p;
        }
        if !Rc::ptr_eq(&top, page) || !matches!(h.data, NodeData::Element { .. }) {
            return Vec::new();
        }
        self.refresh(page, wgen, vw, vh);
        // QWPT7: an inline element without a box of its own and without boxed descendants (a
        // `<span>` of text) gets the union of its glyphs, not its line's block.
        let r = lookup_in(&self.rects, h).or_else(|| union_below_in(&self.rects, h)).or_else(|| self.glyph_union(h)).or_else(|| find_in(&self.rects, h));
        let Some(r) = r else { return Vec::new() };
        vec![
            r.x as f64,
            r.y as f64,
            r.w as f64,
            r.h as f64,
            if r.fixed { 1.0 } else { 0.0 },
            r.ink_r as f64,
            r.ink_b as f64,
            self.page.0 as f64,
            self.page.1 as f64,
        ]
    }
}

impl GeomCache {
    /// QWPT7: the `node_id` of the element under viewport point (`x`, `y`) of the page laid out at
    /// `vw` x `vh` and scrolled to (`sx`, `sy`); 0 when nothing is there. The same glyph-precise
    /// walk as the App's pointer (`paint::hit_test_element`) on the engine's kept laid tree (the
    /// tree this read just refreshed). Without one (the from-scratch fallback) the deepest
    /// recorded element box containing the point answers. Element scroll offsets are not applied
    /// (headless scrolling is script-only).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hit(&mut self, page: &Handle, wgen: u64, x: f32, y: f32, vw: f32, vh: f32, sx: f32, sy: f32) -> usize {
        self.refresh(page, wgen, vw, vh);
        let (dx, dy) = (x + sx, y + sy);
        let engine = crate::incr_layout::with_laid(|l, cache| {
            crate::paint::with_scroll_x(sx, || crate::paint::hit_test_element(l, dx, dy, cache, &HashMap::new(), sy, vh))
        });
        if let Some(n) = engine {
            return self.refine_inline(page, n, dx, dy);
        }
        let mut best: Option<(usize, usize)> = None; // (depth, node)
        for (&node, r) in &self.rects {
            let (px, py) = if r.fixed { (x, y) } else { (dx, dy) };
            if px < r.x || py < r.y || px >= r.x + r.w || py >= r.y + r.h {
                continue;
            }
            let Some(h) = find_node_by_id(page, node) else { continue };
            let depth = ancestor_ids(&h).len();
            if best.is_none_or(|(d, _)| depth > d) {
                best = Some((depth, node));
            }
        }
        match best {
            Some((_, n)) => self.refine_inline(page, n, dx, dy),
            None => 0,
        }
    }

    /// QWPT7: the union of the glyph boxes of the text below `h` (document space), with the
    /// `fixed` flag of the nearest boxed ancestor; `None` when it has no laid-out glyph.
    fn glyph_union(&mut self, h: &Handle) -> Option<GeomRect> {
        if self.glyphs_lazy {
            self.glyphs_lazy = false;
            let glyphs = &mut self.glyphs;
            crate::incr_layout::with_laid(|l, c| walk_glyphs(l, c, glyphs));
        }
        let mut acc: Option<(f32, f32, f32, f32)> = None;
        let mut stack: Vec<Handle> = h.children.borrow().iter().cloned().collect();
        while let Some(n) = stack.pop() {
            match &n.data {
                NodeData::Text { .. } => {
                    for g in self.glyphs.get(&node_id(&n)).map(Vec::as_slice).unwrap_or(&[]) {
                        if g.0.is_whitespace() && g.3 <= 0.0 {
                            continue;
                        }
                        let (l, t, r, b) = (g.1, g.2, g.1 + g.3, g.2 + g.4);
                        acc = Some(acc.map_or((l, t, r, b), |a| (a.0.min(l), a.1.min(t), a.2.max(r), a.3.max(b))));
                    }
                }
                NodeData::Element { .. } => stack.extend(n.children.borrow().iter().cloned()),
                _ => {}
            }
        }
        let fixed = find_in(&self.rects, h).is_some_and(|a| a.fixed);
        // Only collapsible white space (or nothing): the element sits at its insertion point, the
        // start of the next visible glyph in its block (else the end of the previous one).
        let (l, t, r, b) = match (acc, self.ink_only_ws(h)) {
            (Some(a), false) => a,
            _ => self.insertion_point(h)?,
        };
        Some(GeomRect { x: l, y: t, w: r - l, h: b - t, fixed, ink_r: r, ink_b: b })
    }

    fn ink_only_ws(&self, h: &Handle) -> bool {
        let mut stack: Vec<Handle> = h.children.borrow().iter().cloned().collect();
        while let Some(n) = stack.pop() {
            match &n.data {
                NodeData::Text { .. } => {
                    if self.glyphs.get(&node_id(&n)).is_some_and(|g| g.iter().any(|g| !g.0.is_whitespace())) {
                        return false;
                    }
                }
                NodeData::Element { .. } => stack.extend(n.children.borrow().iter().cloned()),
                _ => {}
            }
        }
        true
    }

    /// (left, top, right, bottom) of a zero-width box at the pen position of `h` (an inline element
    /// with no visible glyph): the first visible glyph after it inside its nearest boxed ancestor,
    /// else the end of the last one before it.
    fn insertion_point(&self, h: &Handle) -> Option<(f32, f32, f32, f32)> {
        let mut block = parent_handle(h)?;
        while lookup_in(&self.rects, &block).is_none() {
            block = parent_handle(&block)?;
        }
        // Text nodes of `block` in tree order, split at `h`.
        let (mut before, mut after, mut seen) = (Vec::new(), Vec::new(), false);
        let mut stack: Vec<Handle> = block.children.borrow().iter().rev().cloned().collect();
        while let Some(n) = stack.pop() {
            if Rc::ptr_eq(&n, h) {
                seen = true;
                continue;
            }
            match &n.data {
                NodeData::Text { .. } => {
                    if let Some(g) = self.glyphs.get(&node_id(&n)) {
                        if seen { after.push(g) } else { before.push(g) }
                    }
                }
                NodeData::Element { .. } => stack.extend(n.children.borrow().iter().rev().cloned()),
                _ => {}
            }
        }
        let vis = |g: &&Glyph| !g.0.is_whitespace();
        if let Some(g) = after.iter().find_map(|gs| gs.iter().find(vis)) {
            return Some((g.1, g.2, g.1, g.2 + g.4));
        }
        before.iter().rev().find_map(|gs| gs.iter().rev().find(vis)).map(|g| (g.1 + g.3, g.2, g.1 + g.3, g.2 + g.4))
    }

    /// The laid tree's hit is a box (a block, an inline formatting context); an inline element
    /// inside it (a link, a `<span>`) has no box of its own there. When document point (`x`, `y`)
    /// is on a glyph of a text node below `n`, the text's parent element is the hit.
    fn refine_inline(&mut self, page: &Handle, n: usize, x: f32, y: f32) -> usize {
        let Some(h) = (n != 0).then(|| find_node_by_id(page, n)).flatten() else { return n };
        if self.glyphs_lazy {
            self.glyphs_lazy = false;
            let glyphs = &mut self.glyphs;
            crate::incr_layout::with_laid(|l, c| walk_glyphs(l, c, glyphs));
        }
        let mut stack: Vec<Handle> = h.children.borrow().iter().rev().cloned().collect();
        let mut budget = 20_000usize;
        let mut found = None;
        while let Some(c) = stack.pop() {
            budget = budget.saturating_sub(1);
            if budget == 0 {
                break;
            }
            match &c.data {
                NodeData::Text { .. } => {
                    let on = self.glyphs.get(&node_id(&c)).is_some_and(|g| g.iter().any(|g| x >= g.1 && x < g.1 + g.3 && y >= g.2 && y < g.2 + g.4));
                    if on {
                        found = parent_handle(&c);
                    }
                }
                NodeData::Element { .. } => stack.extend(c.children.borrow().iter().rev().cloned()),
                _ => {}
            }
        }
        found.filter(|p| matches!(p.data, NodeData::Element { .. })).map_or(n, |p| node_id(&p))
    }
}

/// The box of element `h` in `rects`: its own, else the union of its nearest boxed descendants
/// (an inline element, `display: contents`), else the nearest box ancestor's.
fn find_in(rects: &HashMap<usize, GeomRect>, h: &Handle) -> Option<GeomRect> {
    lookup_in(rects, h).or_else(|| union_below_in(rects, h)).or_else(|| {
        let mut cur = parent_handle(h);
        while let Some(a) = cur {
            if matches!(a.data, NodeData::Element { .. })
                && let Some(r) = lookup_in(rects, &a)
            {
                return Some(r);
            }
            cur = parent_handle(&a);
        }
        None
    })
}

fn lookup_in(rects: &HashMap<usize, GeomRect>, h: &Handle) -> Option<GeomRect> {
    rects.get(&node_id(h)).copied()
}

/// Union of the boxes of the nearest element descendants that have one.
fn union_below_in(rects: &HashMap<usize, GeomRect>, h: &Handle) -> Option<GeomRect> {
    let mut acc: Option<GeomRect> = None;
    let mut stack: Vec<Handle> = h.children.borrow().iter().cloned().collect();
    while let Some(n) = stack.pop() {
        if !matches!(n.data, NodeData::Element { .. }) {
            continue;
        }
        if let Some(r) = lookup_in(rects, &n) {
            acc = Some(match acc {
                None => r,
                Some(a) => {
                    let (x, y) = (a.x.min(r.x), a.y.min(r.y));
                    let (rr, bb) = ((a.x + a.w).max(r.x + r.w), (a.y + a.h).max(r.y + r.h));
                    GeomRect { x, y, w: rr - x, h: bb - y, fixed: a.fixed, ink_r: a.ink_r.max(r.ink_r), ink_b: a.ink_b.max(r.ink_b) }
                }
            });
        } else {
            stack.extend(n.children.borrow().iter().cloned());
        }
    }
    acc
}

/// What a layout was built from: the geometry generation and the CSS versions.
pub(crate) type RenderStamp = (u64, (u64, u64));

/// QY53e2: the stamp of the DOM/CSS as they are now; the App records it when it builds a tree.
pub(crate) fn render_stamp() -> RenderStamp {
    (crate::js_dom::geom_gen(), crate::js_style::css_ver())
}

/// Aligns a DOM text node's UTF-16 units with the glyphs laid out for it
/// (whitespace is collapsed in layout, `text-transform` may change case).
/// Returns 5 numbers per UTF-16 unit: `[valid, x, y, w, h]`; units with no
/// glyph (collapsed or trimmed whitespace) have `valid = 0`.
fn align_text(data: &str, g: &[Glyph]) -> Vec<f64> {
    let mut out = Vec::with_capacity(data.len() * 5);
    let mut j = 0usize;
    let ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}');
    for c in data.chars() {
        let mut hit = None;
        if ws(c) {
            if j < g.len() && ws(g[j].0) {
                hit = Some(j);
                j += 1;
            }
        } else {
            let lc = c.to_lowercase().next().unwrap_or(c);
            for (k, gk) in g.iter().enumerate().take((j + 4).min(g.len())).skip(j) {
                if gk.0 == c || gk.0.to_lowercase().next().unwrap_or(gk.0) == lc {
                    hit = Some(k);
                    j = k + 1;
                    break;
                }
            }
        }
        for _ in 0..c.len_utf16() {
            match hit {
                Some(k) => out.extend([1.0, g[k].1 as f64, g[k].2 as f64, g[k].3 as f64, g[k].4 as f64]),
                None => out.extend([0.0; 5]),
            }
        }
    }
    out
}

impl GeomCache {
    /// QJ7: `[valid, x, y, w, h]` per UTF-16 unit of text node `h` (document
    /// space); empty when it is detached or has no laid-out text.
    pub(crate) fn text_rects(&mut self, page: &Handle, wgen: u64, h: &Handle, vw: f32, vh: f32) -> Vec<f64> {
        let mut top = h.clone();
        while let Some(p) = parent_handle(&top) {
            top = p;
        }
        let NodeData::Text { contents } = &h.data else { return Vec::new() };
        if !Rc::ptr_eq(&top, page) {
            return Vec::new();
        }
        self.refresh(page, wgen, vw, vh);
        if self.glyphs_lazy {
            // QINC1i: glyphs of the engine's kept laid tree, collected on the first read after a layout.
            self.glyphs_lazy = false;
            let glyphs = &mut self.glyphs;
            crate::incr_layout::with_laid(|l, c| walk_glyphs(l, c, glyphs));
        }
        match self.glyphs.get(&node_id(h)) {
            Some(g) => align_text(&contents.borrow(), g),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
thread_local! { pub(crate) static TEST_FIXED_COST_MS: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
thread_local! { pub(crate) static TEST_NO_LAYOUT_LIMITS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
