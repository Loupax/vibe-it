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
//! Limits: images are laid out without their decoded size (only
//! `width`/`height` attributes and CSS count), the viewport is the one the
//! JS window reports, inline (non-box) elements get the union of their
//! element descendants or else their nearest box ancestor, and `:hover`-type
//! state is not applied.
use super::*;

/// One element's box in document space.
#[derive(Clone, Copy)]
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

thread_local! {
    static FONTS: RefCell<Option<FontSystem>> = const { RefCell::new(None) };
}

#[derive(Default)]
pub(crate) struct GeomCache {
    key: Option<(u64, u32, u32, usize)>,
    rects: HashMap<usize, GeomRect>,
    /// Size of the laid-out document (the root box).
    page: (f32, f32),
    forced: u32,
    forced_ms: u128,
    logged: bool,
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
    }
    let mut sub = |c: &Laid, ext: &mut (f32, f32)| {
        let e = walk(c, fixed, out);
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

impl GeomCache {
    /// Lays the page out again unless nothing changed since the last read.
    fn refresh(&mut self, page: &Handle, wgen: u64, vw: f32, vh: f32) {
        let css = crate::js_style::page_css();
        let key = (wgen, vw as u32, vh as u32, css.len());
        if self.key == Some(key) {
            return;
        }
        // One-off cost (system font scan), kept out of the forced-layout stats.
        FONTS.with(|f| {
            f.borrow_mut().get_or_insert_with(new_font_system);
        });
        let t0 = std::time::Instant::now();
        let cpu0 = crate::js::thread_cpu_nanos();
        let prev_vp = current_viewport();
        // A layout bug must not take the whole browser down with a page
        // script: on a panic the page simply has no geometry (all zeros).
        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let tree = build_layout_tree_with_resources(page, vw, Some(vh), &css, &HashMap::new());
            let mut cache = ShapeCache::new();
            let mut laid = None;
            // Shaping is progressive (a deadline per pass); keep going until
            // it completes or a hard cap, resuming from the same shape cache.
            FONTS.with(|f| {
                let mut f = f.borrow_mut();
                let fs = f.get_or_insert_with(new_font_system);
                loop {
                    let mut incomplete = false;
                    let mut next_id = 0usize;
                    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
                    laid = Some(measure_document(&tree, fs, vw, vh, &mut next_id, &mut cache, deadline, &mut incomplete));
                    if !incomplete || t0.elapsed() > std::time::Duration::from_secs(10) {
                        break;
                    }
                }
            });
            laid
        }));
        set_viewport(prev_vp.0, prev_vp.1);
        // The layout is the browser's work, not the script's: don't charge it
        // to the script's CPU budget (it would trip it on a big page).
        crate::js::pause_budget_since(cpu0);
        self.rects.clear();
        self.page = (0.0, 0.0);
        match done {
            Ok(Some(l)) => {
                self.page = (l.w, l.h);
                walk(&l, false, &mut self.rects);
            }
            Ok(None) => {}
            Err(_) => diag!("scratchy: js: layout panicked while reading geometry; element rects read as zero"),
        }
        self.key = Some(key);
        self.forced += 1;
        self.forced_ms += t0.elapsed().as_millis();
        if !self.logged && (self.forced >= 20 || self.forced_ms >= 1000) {
            self.logged = true;
            diag!("scratchy: js: {} forced layouts, {} ms (script reads geometry after DOM writes)", self.forced, self.forced_ms);
        }
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
        let r = self.lookup(h).or_else(|| self.union_below(h)).or_else(|| {
            // An inline element has no box: use the nearest box ancestor.
            let mut cur = parent_handle(h);
            while let Some(a) = cur {
                if matches!(a.data, NodeData::Element { .. })
                    && let Some(r) = self.lookup(&a)
                {
                    return Some(r);
                }
                cur = parent_handle(&a);
            }
            None
        });
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

    fn lookup(&self, h: &Handle) -> Option<GeomRect> {
        self.rects.get(&node_id(h)).copied()
    }

    /// Union of the boxes of the nearest element descendants that have one
    /// (an inline element, or `display: contents`).
    fn union_below(&self, h: &Handle) -> Option<GeomRect> {
        let mut acc: Option<GeomRect> = None;
        let mut stack: Vec<Handle> = h.children.borrow().iter().cloned().collect();
        while let Some(n) = stack.pop() {
            if !matches!(n.data, NodeData::Element { .. }) {
                continue;
            }
            if let Some(r) = self.lookup(&n) {
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
}
