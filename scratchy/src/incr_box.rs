//! QINC1g (R60 sections 4.2, 7, 14.4): the persistent box tree, in SHADOW mode.
//!
//! The engine keeps the `LayoutBox` tree of the last build. An update (after QINC1f's restyle) takes
//! the elements whose box content may have changed (`incr_style::take_box_marks`: box owners marked
//! `BOX`, elements whose style / pseudo styles / matches / ambient inputs changed, new elements) plus
//! image arrivals and media size changes, and rebuilds every element box on the path from those
//! elements to the root with today's builder, reading styles and matches from the element table
//! (hooks in `resolve_style_for` / the match memo). Every element-owned box OFF that path is moved over
//! from the old tree unchanged (looked up by node in a pool that is filled lazily: rebuilding a box
//! first detaches its old element-owned descendants, through anonymous boxes and inline-block items).
//! Anonymous boxes, inline runs, markers, table rows, flex/grid anonymous items, generated boxes and
//! first-line/letter are rebuilt with their container. Never reused (rebuilt with their container):
//! `<html>`/`<body>` (the canvas background and viewport overflow are taken from them by the root
//! fix-ups, which are redone on the new root), form controls and media (live state, intrinsic sizes),
//! boxes with inline content (leaves whose parent can rewrite `text-overflow`), and list items whose
//! parent's child list changed (the marker counts siblings).
//!
//! Per element instead of per build (R60 14.4): the image `src`s / CSS image urls a box looked up
//! (`note_src`, attributed to the innermost element box being built: an arriving image rebuilds
//! exactly the boxes that read it), the video intrinsic sizes a box read, generated content (the
//! engine's own walk), the `display: none` roots (`incr_style::display_none_roots`). The rebuild leaves
//! the build's own tables (`WANTED_*`, `DISPLAY_NONE_ROOTS`) alone.
//!
//! `bver` (bumped on every rebuild of an element box) and `ldirty` (rebuilt boxes and their
//! ancestors, for QINC1h1's relayout) are side tables by node.
//!
//! Shadow: tests and `SCRATCHY_INCR_CHECK=1` compare the kept tree with the from-scratch tree after
//! every build (`cmp_boxes`); a difference panics in tests, and in release is logged, counted and the
//! kept tree restarts from the from-scratch one. `SCRATCHY_INCR=1` alone updates without comparing.
use super::*;
use crate::incr_cmp::{cmp_boxes, Names};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

const OFF: u8 = 0;
/// A from-scratch build in shadow mode: record what each box reads.
const RECORD: u8 = 1;
/// The engine's incremental rebuild.
const REBUILD: u8 = 2;

thread_local! {
    static MODE: Cell<u8> = const { Cell::new(OFF) };
    static REC: RefCell<Recording> = RefCell::new(Recording::default());
    static RB: RefCell<Option<Rebuild>> = const { RefCell::new(None) };
    static ENGINE: RefCell<Option<BoxEngine>> = const { RefCell::new(None) };
}

/// What the boxes built in one build read, per element box (key 0: the root's own content).
#[derive(Default)]
struct Recording {
    stack: Vec<usize>,
    srcs: HashMap<usize, Vec<Box<str>>>,
    /// Element boxes built (rebuild only).
    built: Vec<usize>,
    videos: HashMap<usize, Option<(u32, u32)>>,
}

struct Rebuild {
    /// Old element-owned boxes by node, detached from the old tree.
    pool: HashMap<usize, LayoutBox>,
    /// Elements whose box is rebuilt (the touched elements and all their ancestors).
    path: HashSet<usize>,
    kids: HashSet<usize>,
    rebuilt: u64,
    reused: u64,
}

pub(crate) struct BoxEngine {
    doc_key: usize,
    tree: LayoutBox,
    vh: Option<u32>,
    srcs: HashMap<usize, Vec<Box<str>>>,
    videos: HashMap<usize, Option<(u32, u32)>>,
    /// The image cache as of the last update: key -> `Arc` address.
    images: HashMap<Box<str>, usize>,
    bver: HashMap<usize, u64>,
    /// `bver` of every box built by the last full start.
    base_ver: u64,
    next_ver: u64,
    ldirty: HashSet<usize>,
    updates: u64,
    pub(crate) stats: BoxStats,
}

/// What the last box update did (tests and counters).
#[derive(Default, Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct BoxStats {
    pub(crate) full: bool,
    pub(crate) rebuilt: u64,
    pub(crate) reused: u64,
    /// Elements on the rebuilt path.
    pub(crate) path: usize,
    /// Elements touched (before the path closure).
    pub(crate) touched: usize,
    pub(crate) images_changed: usize,
}

// ---------- builder hooks ----------

/// Called at the start of every `build_layout_tree_with_resources`.
pub(crate) fn begin_build(shadow: bool) {
    MODE.with(|m| m.set(if shadow { RECORD } else { OFF }));
    if shadow {
        REC.with(|r| *r.borrow_mut() = Recording::default());
    }
}

#[inline]
pub(crate) fn active() -> bool {
    MODE.with(Cell::get) != OFF
}

#[inline]
pub(crate) fn rebuilding() -> bool {
    MODE.with(Cell::get) == REBUILD
}

/// `build_box_for_element` in shadow mode: reuse a clean old box (rebuild), else build it as the
/// innermost box (what it reads is recorded under it).
pub(crate) fn build_hooked(h: &Handle, build: impl FnOnce() -> LayoutBox) -> LayoutBox {
    let k = node_id(h);
    let rb = rebuilding();
    if rb && let Some(b) = try_reuse(h, k) {
        return b;
    }
    REC.with(|r| {
        let mut r = r.borrow_mut();
        r.stack.push(k);
        if rb {
            r.built.push(k);
        }
    });
    let b = build();
    REC.with(|r| r.borrow_mut().stack.pop());
    if rb {
        RB.with(|x| {
            if let Some(x) = x.borrow_mut().as_mut() {
                x.rebuilt += 1;
            }
        });
    }
    b
}

/// An image `src` / CSS `url()` the box being built looks up in the image cache.
pub(crate) fn note_src(src: &str) {
    if !active() {
        return;
    }
    REC.with(|r| {
        let mut r = r.borrow_mut();
        let owner = r.stack.last().copied().unwrap_or(0);
        let v = r.srcs.entry(owner).or_default();
        if !v.iter().any(|s| &**s == src) {
            v.push(src.into());
        }
    });
}

/// A media box read its element's intrinsic size.
pub(crate) fn note_video(node: usize) {
    if !active() {
        return;
    }
    REC.with(|r| {
        r.borrow_mut().videos.insert(node, crate::media_video::intrinsic(node));
    });
}

pub(crate) fn stored_style(h: &Handle, pseudo: Option<&str>) -> Option<ComputedStyle> {
    if !rebuilding() {
        return None;
    }
    crate::incr_style::stored_style(h, crate::style::pseudo_slot(pseudo)?)
}

pub(crate) fn stored_match(h: &Handle, slot: u8) -> Option<crate::style::MatchList> {
    if !rebuilding() {
        return None;
    }
    crate::incr_style::stored_match(h, slot)
}

fn try_reuse(h: &Handle, k: usize) -> Option<LayoutBox> {
    RB.with(|rb| {
        let mut rb = rb.borrow_mut();
        let rb = rb.as_mut()?;
        let old = rb.pool.remove(&k)?;
        if !rb.path.contains(&k) && reusable(h, &old, &rb.kids) {
            rb.reused += 1;
            return Some(old);
        }
        harvest_content(old.content, &mut rb.pool);
        None
    })
}

fn reusable(h: &Handle, old: &LayoutBox, kids: &HashSet<usize>) -> bool {
    let tag = element_tag(h);
    if matches!(tag, "html" | "body" | "video" | "audio") || crate::forms::is_form_control_tag(tag) {
        return false;
    }
    if matches!(old.content, BoxContent::Inline(_)) {
        return false;
    }
    if (tag == "li" || old.style.list_item) && parent_handle(h).is_some_and(|p| kids.contains(&node_id(&p))) {
        return false;
    }
    true
}

/// Element-owned boxes are pooled by node; anything else (anonymous boxes, generated boxes, table
/// rows, which the builder makes directly) is opened and its element-owned descendants pooled.
fn harvest_box(b: LayoutBox, pool: &mut HashMap<usize, LayoutBox>) {
    if b.style.node != 0 && b.style.pseudo == 0 && b.style.display != Display::TableRow {
        pool.insert(b.style.node, b);
    } else {
        harvest_content(b.content, pool);
    }
}

fn harvest_content(c: BoxContent, pool: &mut HashMap<usize, LayoutBox>) {
    crate::stack::grow(|| harvest_content_body(c, pool))
}

fn harvest_content_body(c: BoxContent, pool: &mut HashMap<usize, LayoutBox>) {
    match c {
        BoxContent::Blocks(v) | BoxContent::TableRow(v) => {
            for b in v {
                harvest_box(b, pool);
            }
        }
        BoxContent::Inline(items) => {
            for it in items {
                if let InlineItem::Embedded(b) = it {
                    harvest_box(*b, pool);
                }
            }
        }
        BoxContent::Image(_) => {}
    }
}

// ---------- the update ----------

fn clone_items(items: &[InlineItem]) -> Vec<InlineItem> {
    items
        .iter()
        .map(|i| match i {
            InlineItem::Text(t, s) => InlineItem::Text(t.clone(), *s),
            InlineItem::Image { image, style, background_image } => {
                InlineItem::Image { image: image.clone(), style: *style, background_image: background_image.clone() }
            }
            InlineItem::Embedded(b) => InlineItem::Embedded(Box::new(clone_box(b))),
            InlineItem::Spacer { w, pad_w, bg, start } => InlineItem::Spacer { w: *w, pad_w: *pad_w, bg: *bg, start: *start },
        })
        .collect()
}

/// A deep copy of a box tree (the kept tree starts as a copy of a from-scratch build).
pub(crate) fn clone_box(b: &LayoutBox) -> LayoutBox {
    crate::stack::grow(|| LayoutBox {
        style: b.style,
        background_image: b.background_image.clone(),
        content: match &b.content {
            BoxContent::Blocks(v) => BoxContent::Blocks(v.iter().map(clone_box).collect()),
            BoxContent::TableRow(v) => BoxContent::TableRow(v.iter().map(clone_box).collect()),
            BoxContent::Inline(items) => BoxContent::Inline(clone_items(items)),
            BoxContent::Image(i) => BoxContent::Image(i.clone()),
        },
    })
}

fn snapshot(cache: &HashMap<String, Arc<DecodedImage>>) -> HashMap<Box<str>, usize> {
    cache.iter().map(|(k, v)| (Box::<str>::from(k.as_str()), Arc::as_ptr(v) as usize)).collect()
}

impl BoxEngine {
    fn start(doc: &Handle, fresh: &LayoutBox, vh: Option<f32>, cache: &HashMap<String, Arc<DecodedImage>>, rec: Recording, ver: u64) -> BoxEngine {
        BoxEngine {
            doc_key: node_id(doc),
            tree: clone_box(fresh),
            vh: vh.map(f32::to_bits),
            srcs: rec.srcs,
            videos: rec.videos,
            images: snapshot(cache),
            bver: HashMap::new(),
            base_ver: ver,
            next_ver: ver + 1,
            ldirty: HashSet::new(),
            updates: 0,
            stats: BoxStats { full: true, ..Default::default() },
        }
    }

    /// Keys of the boxes that read an image whose cache entry appeared, changed or went away.
    fn image_changes(&mut self, cache: &HashMap<String, Arc<DecodedImage>>) -> Vec<usize> {
        let same = cache.len() == self.images.len()
            && cache.iter().all(|(k, v)| self.images.get(k.as_str()) == Some(&(Arc::as_ptr(v) as usize)));
        if same {
            return Vec::new();
        }
        let now = snapshot(cache);
        let mut changed: HashSet<&str> = HashSet::new();
        for (k, v) in &now {
            if self.images.get(k) != Some(v) {
                changed.insert(k);
            }
        }
        for k in self.images.keys() {
            if !now.contains_key(k) {
                changed.insert(k);
            }
        }
        let out: Vec<usize> =
            self.srcs.iter().filter(|(_, v)| v.iter().any(|s| changed.contains(&**s))).map(|(k, _)| *k).collect();
        self.stats.images_changed = changed.len();
        drop(changed);
        self.images = now;
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        doc: &Handle,
        rules: &Rc<Vec<StyleRule>>,
        vw: f32,
        vh: Option<f32>,
        cache: &HashMap<String, Arc<DecodedImage>>,
        mut touched: Vec<usize>,
        kids: HashSet<usize>,
    ) {
        self.updates += 1;
        self.stats = BoxStats::default();
        self.ldirty.clear();
        touched.extend(self.image_changes(cache));
        for (k, size) in &self.videos {
            if crate::media_video::intrinsic(*k) != *size {
                touched.push(*k);
            }
        }
        self.stats.touched = touched.len();
        // The path: every touched element and its ancestors (key 0 = the root's own content).
        let mut path: HashSet<usize> = HashSet::new();
        let mut root = false;
        for k in touched {
            if k == 0 {
                root = true;
                continue;
            }
            if path.contains(&k) {
                continue;
            }
            let Some(h) = crate::incr_style::handle_of_key(k) else { continue };
            path.insert(k);
            let mut cur = parent_handle(&h);
            while let Some(p) = cur {
                if !path.insert(node_id(&p)) {
                    break;
                }
                cur = parent_handle(&p);
            }
            root = true;
        }
        let vh_bits = vh.map(f32::to_bits);
        if !root && vh_bits == self.vh {
            return;
        }
        self.vh = vh_bits;
        self.stats.path = path.len();
        let old = std::mem::replace(
            &mut self.tree,
            LayoutBox { style: anonymous_style(ROOT_STYLE), background_image: None, content: BoxContent::Blocks(Vec::new()) },
        );
        let mut pool = HashMap::new();
        harvest_content(old.content, &mut pool);
        let ldirty: HashSet<usize> = path.iter().copied().collect();
        RB.with(|r| *r.borrow_mut() = Some(Rebuild { pool, path, kids, rebuilt: 0, reused: 0 }));
        REC.with(|r| *r.borrow_mut() = Recording::default());
        MODE.with(|m| m.set(REBUILD));
        let rfs = crate::incr_style::root_font_size().unwrap_or(ROOT_STYLE.font_size);
        let mut tree = crate::incr_style::with_engine_generated(|| {
            let root_inherited = InheritedStyle { root_font_size: rfs, ..ROOT_STYLE };
            let mut root_style = anonymous_style(root_inherited);
            root_style.icb_height = vh;
            LayoutBox {
                style: root_style,
                background_image: None,
                content: BoxContent::Blocks(build_children(doc, rules, root_inherited, DecorationValue::default(), None, vw, cache)),
            }
        });
        MODE.with(|m| m.set(OFF));
        crate::images::renumber_document_order(&mut tree);
        crate::images::propagate_canvas_background(&mut tree);
        crate::images::propagate_viewport_overflow(&mut tree);
        self.tree = tree;
        let rb = RB.with(|r| r.borrow_mut().take()).expect("set above");
        let rec = REC.with(|r| std::mem::take(&mut *r.borrow_mut()));
        // What the rebuilt boxes read replaces what their old versions read.
        self.srcs.remove(&0);
        for k in &rec.built {
            self.srcs.remove(k);
            self.bver.insert(*k, self.next_ver);
            self.next_ver += 1;
        }
        self.srcs.extend(rec.srcs);
        self.videos.extend(rec.videos);
        self.ldirty = ldirty;
        self.stats.rebuilt = rb.rebuilt;
        self.stats.reused = rb.reused;
        // Old pool entries left over were removed from the tree.
        if self.updates.is_multiple_of(64) {
            self.srcs.retain(|k, _| *k == 0 || crate::incr_style::has_entry(*k));
            self.videos.retain(|k, _| crate::incr_style::has_entry(*k));
            self.bver.retain(|k, _| crate::incr_style::has_entry(*k));
        }
    }
}

/// The box stage of one shadowed build (called by `incr_style::after_build`, inside its environment):
/// update the kept tree and, in check mode, compare it with the from-scratch `fresh` tree. Returns the
/// mismatch reports.
pub(crate) fn update(
    doc: &Handle,
    rules: &Rc<Vec<StyleRule>>,
    vw: f32,
    vh: Option<f32>,
    cache: &HashMap<String, Arc<DecodedImage>>,
    fresh: &LayoutBox,
    style_ok: bool,
) -> Vec<String> {
    let t0 = std::time::Instant::now();
    MODE.with(|m| m.set(OFF));
    let rec = REC.with(|r| std::mem::take(&mut *r.borrow_mut()));
    let marks = crate::incr_style::take_box_marks();
    let mut eng = ENGINE.with(|e| e.borrow_mut().take());
    let doc_key = node_id(doc);
    let incremental = style_ok && marks.as_ref().is_some_and(|m| !m.full) && eng.as_ref().is_some_and(|e| e.doc_key == doc_key);
    let ver = eng.as_ref().map_or(1, |e| e.next_ver);
    let mut eng = match (incremental, eng.take(), marks) {
        (true, Some(mut e), Some(m)) => {
            e.update(doc, rules, vw, vh, cache, m.touched, m.kids_changed);
            e
        }
        _ => {
            let e = BoxEngine::start(doc, fresh, vh, cache, rec, ver);
            ENGINE.with(|x| *x.borrow_mut() = Some(e));
            return Vec::new();
        }
    };
    use crate::perf::{incr_add, Incr};
    if eng.stats.rebuilt + eng.stats.reused > 0 {
        incr_add(Incr::BoxUpdates, 1);
    }
    incr_add(Incr::BoxesRebuilt, eng.stats.rebuilt);
    incr_add(Incr::BoxesReused, eng.stats.reused);
    crate::perf::incr_box_ns(t0.elapsed().as_nanos() as u64);
    let mut out = Vec::new();
    if crate::incr_style::checking()
        && cmp_boxes(&Names::default(), "", fresh, &eng.tree).is_err()
    {
        let names = Names::from_dom(doc);
        if let Err(m) = cmp_boxes(&names, "", fresh, &eng.tree) {
            out.push(format!(
                "boxes {m} (update {}, touched {}, path {}, rebuilt {}, reused {})",
                eng.updates, eng.stats.touched, eng.stats.path, eng.stats.rebuilt, eng.stats.reused
            ));
        }
        // No longer trusted: start over from the from-scratch tree.
        let ver = eng.next_ver;
        eng = BoxEngine::start(doc, fresh, vh, cache, rec, ver);
    }
    ENGINE.with(|x| *x.borrow_mut() = Some(eng));
    out
}

/// `(element boxes, bytes)` of the kept tree (styles, items, text, side tables).
pub(crate) fn mem() -> (usize, usize) {
    fn walk(b: &LayoutBox, n: &mut usize, bytes: &mut usize) {
        *n += usize::from(b.style.node != 0);
        *bytes += std::mem::size_of::<LayoutBox>();
        match &b.content {
            BoxContent::Blocks(v) | BoxContent::TableRow(v) => {
                *bytes += (v.capacity() - v.len()) * std::mem::size_of::<LayoutBox>();
                for c in v {
                    walk(c, n, bytes);
                }
            }
            BoxContent::Inline(items) => {
                *bytes += items.capacity() * std::mem::size_of::<InlineItem>();
                for i in items {
                    match i {
                        InlineItem::Text(t, _) => *bytes += t.capacity(),
                        InlineItem::Embedded(e) => walk(e, n, bytes),
                        _ => {}
                    }
                }
            }
            BoxContent::Image(_) => {}
        }
    }
    ENGINE.with(|e| {
        let Ok(e) = e.try_borrow() else { return (0, 0) };
        let Some(e) = e.as_ref() else { return (0, 0) };
        let (mut n, mut bytes) = (0, 0);
        crate::stack::grow(|| walk(&e.tree, &mut n, &mut bytes));
        bytes += e.srcs.values().map(|v| 48 + v.iter().map(|s| 16 + s.len()).sum::<usize>()).sum::<usize>();
        bytes += (e.bver.capacity() + e.ldirty.capacity() + e.videos.capacity()) * 24;
        bytes += e.images.keys().map(|k| 32 + k.len()).sum::<usize>();
        (n, bytes)
    })
}

/// `bver` of the element box of `node` (R60 4.2: bumped on every rebuild).
#[allow(dead_code)] // read by QINC1h1/h2
pub(crate) fn bver(node: usize) -> Option<u64> {
    ENGINE.with(|e| e.borrow().as_ref().map(|e| e.bver.get(&node).copied().unwrap_or(e.base_ver)))
}

/// Whether the element box of `node` (or something below it) was rebuilt by the last update.
#[allow(dead_code)] // read by QINC1h1
pub(crate) fn ldirty(node: usize) -> bool {
    ENGINE.with(|e| e.borrow().as_ref().is_some_and(|e| e.ldirty.contains(&node)))
}

/// Test / diagnostic access.
#[cfg(test)]
pub(crate) fn with_engine<R>(f: impl FnOnce(Option<&BoxEngine>) -> R) -> R {
    ENGINE.with(|e| f(e.borrow().as_ref()))
}

#[cfg(test)]
impl BoxEngine {
    pub(crate) fn tree(&self) -> &LayoutBox {
        &self.tree
    }
}

#[cfg(test)]
pub(crate) fn reset() {
    ENGINE.with(|e| *e.borrow_mut() = None);
}
