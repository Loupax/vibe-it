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

thread_local! {
    /// Test-only: how many forced layouts (cache misses) ran on this thread.
    #[cfg(test)]
    pub(crate) static GEOM_LAYOUTS: Cell<usize> = const { Cell::new(0) };
    static FONTS: RefCell<Option<FontSystem>> = const { RefCell::new(None) };
    /// QY53l2: the `display: none` subtrees of the last forced layout on this thread.
    static HIDDEN: RefCell<Option<HiddenSnap>> = const { RefCell::new(None) };
}

/// QY53l2: which subtrees had no box at all in the last `GeomCache` layout
/// because their root's own `display` was `none`. A DOM write that lands
/// entirely inside one of them and cannot change that root's or anything
/// else's style changes no box, so it need not move the geometry generation
/// (`js_dom::geom_bump`). Valid only while nothing moved the geometry
/// generation or the CSS since that layout; replaced by every layout.
struct HiddenSnap {
    /// Geometry generation and CSS version the layout ran at.
    ggen: u64,
    css: (u64, u64),
    /// Hidden roots by node id (the `Weak` guards against a reused address).
    roots: HashMap<usize, WeakHandle>,
    /// Some selector uses `:has()`: a descendant write can change an ancestor's match.
    has_sel: bool,
    /// Some selector uses `:empty`: a child-list change of the root itself can change its match.
    empty_sel: bool,
}

/// QY53l2: a hidden root under one of these (or being one) is not trusted:
/// their content is read by something other than the box build (options for
/// a `<select>`'s width, SVG/MathML/replaced content, table columns), or it
/// is not rendered through the normal tree at all.
const OPAQUE_CONTAINERS: &[&str] = &[
    "select", "datalist", "optgroup", "option", "svg", "math", "object", "embed", "video", "audio", "canvas", "iframe",
    "picture", "template", "slot", "col", "colgroup", "textarea", "noscript",
];

/// QY53l2: elements whose presence, attributes or content change the page's
/// style (or document-level state) wherever they are, `display: none` or not.
const STYLE_SENSITIVE: &[&str] = &["style", "link", "meta", "base", "slot"];

fn tag_in(h: &Handle, list: &[&str]) -> bool {
    let t = element_tag(h);
    !t.is_empty() && list.iter().any(|x| t.eq_ignore_ascii_case(x))
}

/// Does `h` or a node below it have a `STYLE_SENSITIVE` tag?
fn subtree_sensitive(h: &Handle) -> bool {
    let mut stack = vec![h.clone()];
    while let Some(n) = stack.pop() {
        if tag_in(&n, STYLE_SENSITIVE) {
            return true;
        }
        stack.extend(n.children.borrow().iter().cloned());
    }
    false
}

/// `:empty` in the selector sense: no element child and no non-empty text.
fn sel_empty(h: &Handle) -> bool {
    !h.children.borrow().iter().any(|c| match &c.data {
        NodeData::Element { .. } => true,
        NodeData::Text { contents } => !contents.borrow().is_empty(),
        _ => false,
    })
}

/// Builds the snapshot from the roots the box build recorded.
fn hidden_snapshot(page: &Handle, ggen: u64, css: (u64, u64), found: Vec<Handle>, deps: Option<(bool, bool)>) -> Option<HiddenSnap> {
    let (has_sel, empty_sel) = deps?;
    if has_sel {
        return None;
    }
    let mut roots = HashMap::new();
    'roots: for r in found {
        let mut cur = Some(r.clone());
        let mut top = r.clone();
        while let Some(n) = cur {
            if tag_in(&n, OPAQUE_CONTAINERS) {
                continue 'roots;
            }
            cur = parent_handle(&n);
            top = n;
        }
        if Rc::ptr_eq(&top, page) {
            roots.insert(node_id(&r), Rc::downgrade(&r));
        }
    }
    Some(HiddenSnap { ggen, css, roots, has_sel, empty_sel })
}

/// The hidden root that is `n` or an ancestor of it, if the snapshot is current.
fn hidden_root_of(snap: &HiddenSnap, n: &Handle) -> Option<Handle> {
    let mut cur = Some(n.clone());
    while let Some(a) = cur {
        if let Some(w) = snap.roots.get(&node_id(&a))
            && w.upgrade().is_some_and(|r| Rc::ptr_eq(&r, &a))
        {
            return Some(a);
        }
        cur = parent_handle(&a);
    }
    None
}

fn with_current_snap<R>(f: impl FnOnce(&HiddenSnap) -> Option<R>) -> Option<R> {
    HIDDEN.with(|s| {
        let s = s.borrow();
        let snap = s.as_ref()?;
        if snap.has_sel || snap.roots.is_empty() || snap.ggen != crate::js_dom::geom_gen() || snap.css != crate::js_style::css_ver() {
            return None;
        }
        f(snap)
    })
}

/// QY53l2: may an attribute write on element `x` skip the geometry bump?
/// Only when `x` lies strictly inside a hidden subtree (never the hidden root
/// itself or an ancestor: its own `class`/`style`/`hidden`/... decide its
/// `display`) and `x` is not a style-bearing element.
pub(crate) fn attr_write_hidden(x: &Handle) -> bool {
    if !matches!(x.data, NodeData::Element { .. }) || tag_in(x, STYLE_SENSITIVE) {
        return false;
    }
    let Some(p) = parent_handle(x) else { return false };
    with_current_snap(|snap| hidden_root_of(snap, &p).map(|_| ())).is_some()
}

/// QY53l2: what `children_write_pre` saw; `children_write_post` decides.
pub(crate) struct HiddenChildren {
    /// `:empty`-ness of the hidden root before the write, when the write
    /// changes the root's own children and some rule uses `:empty`.
    root_empty: Option<bool>,
}

/// QY53l2, before a change to `p`'s child list (insert, remove, text or HTML
/// replace; `moved` = the nodes that leave or enter it): `Some` when `p` is a
/// hidden root or lies inside one, and no moved node carries style.
pub(crate) fn children_write_pre(p: &Handle, moved: &[Handle]) -> Option<HiddenChildren> {
    if !matches!(p.data, NodeData::Element { .. }) || tag_in(p, STYLE_SENSITIVE) {
        return None;
    }
    with_current_snap(|snap| {
        let r = hidden_root_of(snap, p)?;
        if moved.iter().any(subtree_sensitive) {
            return None;
        }
        let root_empty = (snap.empty_sel && Rc::ptr_eq(&r, p)).then(|| sel_empty(p));
        Some(HiddenChildren { root_empty })
    })
}

/// QY53l2, after the change: may it skip the geometry bump?
pub(crate) fn children_write_post(p: &Handle, pre: Option<HiddenChildren>) -> bool {
    match pre {
        None => false,
        Some(HiddenChildren { root_empty: None }) => true,
        Some(HiddenChildren { root_empty: Some(e) }) => sel_empty(p) == e,
    }
}

/// QY53l2: whether an HTML string may create a style-bearing element.
pub(crate) fn html_sensitive(html: &str) -> bool {
    let l = html.to_ascii_lowercase();
    STYLE_SENSITIVE.iter().any(|t| l.contains(&format!("<{t}")))
}

#[derive(Default)]
pub(crate) struct GeomCache {
    key: Option<(u64, u32, u32, (u64, u64))>,
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
}

/// One laid-out character: `(char, x, y, w, h)` in document space.
type Glyph = (char, f32, f32, f32, f32);

/// QJ7: records the glyphs of every paragraph under `l`, keyed by DOM text node.
fn walk_glyphs(l: &Laid, cache: &ShapeCache, out: &mut HashMap<usize, Vec<Glyph>>) {
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

/// QP2: most shaped paragraphs the JS thread's memo keeps (checked per refresh).
const SHAPE_MEMO_CAP: usize = 4096;

impl GeomCache {
    /// QOBS7: `((rect entries, bytes), (glyph entries = text nodes, bytes))`, by capacity.
    pub(crate) fn mem_estimate(&self) -> ((usize, usize), (usize, usize)) {
        let rects = (self.rects.len(), self.rects.capacity() * (std::mem::size_of::<(usize, GeomRect)>() + 1));
        let glyphs = (
            self.glyphs.len(),
            self.glyphs.capacity() * (std::mem::size_of::<(usize, Vec<Glyph>)>() + 1) + self.glyphs.values().map(|v| v.capacity() * std::mem::size_of::<Glyph>()).sum::<usize>(),
        );
        (rects, glyphs)
    }

    /// QY53e1: would a read at this viewport be served without a layout? Cheap: no layout, no CSS string.
    pub(crate) fn is_fresh(&self, wgen: u64, vw: f32, vh: f32) -> bool {
        self.key == Some((wgen, vw as u32, vh as u32, crate::js_style::css_ver()))
    }

    /// QY53e1: CPU ms the last forced layout took.
    pub(crate) fn last_cost_ms(&self) -> f64 {
        self.last_cpu_ms
    }

    /// Lays the page out again unless nothing changed since the last read.
    fn refresh(&mut self, page: &Handle, wgen: u64, vw: f32, vh: f32) {
        // QY53h: keyed on the CSS version counters; the CSS string is only built on a miss.
        let key = (wgen, vw as u32, vh as u32, crate::js_style::css_ver());
        if self.key == Some(key) {
            return;
        }
        let css = crate::js_style::page_css();
        #[cfg(test)]
        GEOM_LAYOUTS.with(|c| c.set(c.get() + 1));
        // One-off cost (system font scan), kept out of the forced-layout stats.
        FONTS.with(|f| {
            f.borrow_mut().get_or_insert_with(new_font_system);
        });
        // QP2: content-keyed shape memo on this thread (enabled once). Entries
        // are keyed by text + styles + width, so a DOM write can never serve a
        // stale shape; a resize drops them (old widths), and the size is capped.
        crate::layout::SHAPE_MEMO_ENABLED.with(|m| m.set(true));
        let vp = (vw as u32, vh as u32);
        if self.last_vp != Some(vp) || crate::layout::SHAPE_MEMO.with(|m| m.borrow().len() > SHAPE_MEMO_CAP) {
            crate::layout::SHAPE_MEMO.with(|m| m.borrow_mut().clear());
        }
        self.last_vp = Some(vp);
        let t0 = std::time::Instant::now();
        // QOBS3: the phase timers and counters run only while the recorder is on.
        let recording = crate::perf::begin_forced();
        let cpu0 = crate::js::thread_cpu_nanos();
        let prev_vp = current_viewport();
        // QY53l2: a layout replaces the hidden-subtree snapshot; collect the new one.
        HIDDEN.with(|h| *h.borrow_mut() = None);
        crate::images::DISPLAY_NONE_ROOTS.with(|r| *r.borrow_mut() = Some(Vec::new()));
        // A layout bug must not take the whole browser down with a page
        // script: on a panic the page simply has no geometry (all zeros).
        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let tree = build_layout_tree_with_resources(page, vw, Some(vh), &css, &HashMap::new());
            let deps = crate::style::rule_child_deps();
            let t_phase = crate::perf::phase(crate::perf::Phase::MeasureDocument);
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
            drop(tree);
            drop(t_phase);
            let t_phase = crate::perf::phase(crate::perf::Phase::GlyphWalk);
            let mut glyphs = HashMap::new();
            if let Some(l) = &laid {
                walk_glyphs(l, &cache, &mut glyphs);
            }
            drop(cache);
            drop(t_phase);
            (laid, glyphs, deps)
        }));
        set_viewport(prev_vp.0, prev_vp.1);
        let found = crate::images::DISPLAY_NONE_ROOTS.with(|r| r.borrow_mut().take()).unwrap_or_default();
        let t_phase = crate::perf::phase(crate::perf::Phase::RectWalk);
        let prev = std::mem::take(&mut self.rects);
        self.page = (0.0, 0.0);
        self.glyphs.clear();
        match done {
            Ok((Some(l), glyphs, deps)) => {
                self.page = (l.w, l.h);
                walk(&l, false, &mut self.rects);
                self.glyphs = glyphs;
                let snap = hidden_snapshot(page, wgen, key.3, found, deps);
                HIDDEN.with(|h| *h.borrow_mut() = snap);
            }
            Ok((None, ..)) => {}
            Err(_) => diag!("scratchy: js: layout panicked while reading geometry; element rects read as zero"),
        }
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
        self.forced += 1;
        self.forced_ms += t0.elapsed().as_millis();        if !self.logged && (self.forced >= 20 || self.forced_ms >= 1000) {
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
        match self.glyphs.get(&node_id(h)) {
            Some(g) => align_text(&contents.borrow(), g),
            None => Vec::new(),
        }
    }
}
