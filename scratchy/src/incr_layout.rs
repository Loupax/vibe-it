//! QINC1h1 (R60 sections 4.3, 8.1-8.5): relayout from the last laid tree, in SHADOW mode.
//!
//! The engine keeps the `Laid` tree it laid out last for QINC1g's kept box tree, with its own shape
//! cache and paragraph ids. Every element `Laid` it measures records its `Ck` (layout.rs): the
//! constraints it was measured at, where `measure_inner` returned it, the box's `bver`, the
//! out-of-flow boxes (`pending`) and z-index entries (`ZInfo::bubble`) its subtree sent past it, and
//! whether it can be reused at all (no floats in or out, unique keys) or a change inside it can stop
//! at it (`intr`: nothing above reads its intrinsic sizes).
//!
//! An update (after a shadowed build's from-scratch layout, `after_measure`):
//! 1. BOTTOM-UP from every element the box update touched (plus the boxes it rebuilt off the path),
//!    outermost first: re-measure the element's box alone at its old key (an out-of-flow box:
//!    `resolve_positioned` at its old containing block), its clean children served from the old tree
//!    (below), and compare what the parent's flow reads (border box, margins, first baseline, the
//!    collapse-through inputs, the `finish_z` values, sticky, z-order key, nothing bubbled). Equal:
//!    splice it in place of the old `Laid` (moved by the old box's later shift); nothing outside moves.
//!    Different, or the box cannot be judged alone (a float, inline-level, a flex/grid/table item, an
//!    ancestor reading its intrinsic size, out-of-flow descendants or z entries leaving it): one level
//!    up. Reaching the root -> 2.
//! 2. TOP-DOWN from the root (`measure_document_engine`) with reuse: the old tree is harvested along
//!    the dirty paths (every clean element `Laid` hanging off them is detached whole, by node), and
//!    `measure_inner` returns a clean box measured at the same key minus the origin as its old `Laid`
//!    shifted by the origin delta (`try_hit`), re-emitting its `pending` (at shifted static positions)
//!    and its z-index bubble (the entries are found in the harvest by key).
//!
//! A box is clean when its `bver` (QINC1g) is the one it was measured with.
//! Ink and the root's in-flow extent are recomputed only for re-measured boxes and their ancestors;
//! a reused subtree that moved has its ink shifted (`finish_root`). Offset-only style changes are the
//! bottom-up case: the box is re-measured (its children hit) and its parent view is unchanged.
//! Drift guard: a full layout every `DRIFT_UPDATES` updates. Touched roots: `touched_roots`.
//!
//! Shadow: the result is compared with the from-scratch tree (`cmp_laid`) in tests and with
//! `SCRATCHY_INCR_CHECK=1` (tests panic; release logs, counts `incr.check_mismatch` and starts over);
//! `SCRATCHY_INCR=1` alone relayouts without comparing; with neither nothing here runs.
use super::*;
use crate::incr_cmp::{cmp_laid, Names};
use crate::layout::{Ck, CkFloats, CkPending, FloatSnap, PSig};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

/// R60 8.5: repeated shifts accumulate f32 rounding; a full layout every this many updates resets it.
pub(crate) const DRIFT_UPDATES: u64 = 1000;

/// A bottom-up walk this many levels up goes to the root instead (each level locates its box from
/// the root: a long walk would be quadratic, and the top-down pass reuses everything clean anyway).
const MAX_WALK: usize = 24;

/// Values closer than this are the same (positions and sizes; the comparators allow 0.01 px).
const NEAR: f32 = 1e-3;

/// One `measure_inner` call's key (see `Ck`).
pub(crate) struct Key {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) ch: Option<f32>,
    pub(crate) amo: Option<f32>,
    pub(crate) fbw: Option<f32>,
    pub(crate) zskip: bool,
    pub(crate) floats_in: bool,
}

/// What the last relayout did (tests and counters).
#[derive(Default, Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct LayoutStats {
    /// Laid out from scratch (first layout, the box tree started over, viewport width, drift guard).
    pub(crate) full: bool,
    /// The bottom-up walk reached the root: top-down relayout with reuse.
    pub(crate) root: bool,
    pub(crate) hits: u64,
    pub(crate) miss_dirty: u64,
    pub(crate) miss_key: u64,
    /// QINC1k1: of `miss_key`, the boxes refused only because they are `y_sensitive` and moved in y.
    pub(crate) miss_y: u64,
    pub(crate) miss_floats: u64,
    pub(crate) miss_not_cacheable: u64,
    pub(crate) miss_no_old: u64,
    /// Boxes measured for real (element and anonymous).
    pub(crate) remeasured: u64,
    /// QINC1h2: intrinsic-size (min, max) lookups served from the cache / computed.
    pub(crate) intr_hits: u64,
    pub(crate) intr_misses: u64,
    /// QINC1h2: bottom-up steps under an intrinsic-size reader that stopped (natural widths unchanged).
    pub(crate) intr_stops: u64,
    /// Reused boxes that moved.
    pub(crate) shifted: u64,
    /// Bottom-up splices, and the levels each walked up before it stopped.
    pub(crate) splices: u64,
    pub(crate) levels: Vec<usize>,
    pub(crate) starts: usize,
    /// Why the update went top-down from the root.
    pub(crate) root_why: &'static str,
}

/// R60 9: what a relayout touched (QINC1i updates the rect map from it, QINC1o repaints it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum Touch {
    /// The subtree was laid out again (node 0: the whole tree).
    Remeasured,
    /// A reused subtree moved.
    Shifted,
}

struct Ctx {
    harvest: HashMap<(usize, u8), Laid>,
    reuse: bool,
    /// The box being re-measured bottom-up (its old `Laid` is in the tree, not the harvest).
    skip: usize,
    stats: LayoutStats,
    touched: Vec<(usize, Touch)>,
}

/// QINC1j: the kept laid tree, shared with the App's view of it (`view`). Copy on write: a mutable
/// access clones the tree only while a view still holds it (the App drops its handle before a frame
/// update; a forced layout between two frames pays one copy).
pub(crate) struct SharedLaid(std::rc::Rc<Laid>);

impl SharedLaid {
    fn new(l: Laid) -> SharedLaid {
        SharedLaid(std::rc::Rc::new(l))
    }

    fn into_inner(self) -> Laid {
        std::rc::Rc::unwrap_or_clone(self.0)
    }
}

impl std::ops::Deref for SharedLaid {
    type Target = Laid;
    fn deref(&self) -> &Laid {
        &self.0
    }
}

impl std::ops::DerefMut for SharedLaid {
    fn deref_mut(&mut self) -> &mut Laid {
        std::rc::Rc::make_mut(&mut self.0)
    }
}

/// QINC1j: what a worker layout of a kept-tree copy records in each `Ck::bver` (the worker thread
/// has no box engine): the box engine's `bver` table as of the copy.
#[derive(Clone)]
pub(crate) struct WorkerVer {
    pub(crate) base: u64,
    pub(crate) bver: std::sync::Arc<HashMap<usize, u64>>,
}

pub(crate) struct LayoutEngine {
    doc_key: usize,
    base_ver: u64,
    laid: SharedLaid,
    cache: ShapeCache,
    next_id: usize,
    vw: f32,
    vh: f32,
    /// The root's height before the in-flow extent grew it.
    root_h: f32,
    /// The root box's style (the canvas background and viewport overflow come from `<html>`/`<body>`).
    root_style: ComputedStyle,
    updates: u64,
    since_full: u64,
    live: usize,
    pub(crate) stats: LayoutStats,
    touched: Vec<(usize, Touch)>,
    /// Old parent views of boxes whose subtree an attempt of this update opened (see `attempt`).
    views: HashMap<usize, View>,
    /// QINC1j: an App frame could not update the tree (it went to the worker): kept only as the view
    /// the App shows meanwhile; the next update starts over.
    stale: bool,
    /// QINC1j: bumped on every change of `laid` (the App installs a new view when it moved).
    ver: u64,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
    static ENGINE: RefCell<Option<LayoutEngine>> = const { RefCell::new(None) };
    static WALKED: Cell<u64> = const { Cell::new(0) };
    /// Element boxes whose old `Laid` this update opened because they changed (a later lookup is a
    /// "dirty" miss, not "no old").
    static OPENED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    /// QINC1k10: harvest keys that held two `Laid`s in this update. Neither is reused, and nor is any
    /// later one: a third `put` (e.g. the old tree's stale entry, laid against its old containing block,
    /// arriving after a failed attempt's two fresh copies cancelled out) would otherwise be served alone.
    static AMBIGUOUS: RefCell<HashSet<(usize, u8)>> = RefCell::new(HashSet::new());
    /// QINC1j: set on the shaping worker while it lays out a kept-tree copy.
    static WORKER_VER: RefCell<Option<WorkerVer>> = const { RefCell::new(None) };
    static VER: Cell<u64> = const { Cell::new(0) };
}

thread_local! {
    /// QINC1h2: `cell_natural_width` results of element boxes: (node, as_child) -> (bver, min, max).
    /// Used only during an engine measure; cleared when the engine starts over.
    static NAT: RefCell<NatMap> = RefCell::new(HashMap::new());
}

type NatMap = HashMap<(usize, bool), (u64, f32, f32)>;

/// The cached (min, max) natural width of element box `node` when it is still clean.
pub(crate) fn nat_get(node: usize, as_child: bool) -> Option<(f32, f32)> {
    let v = nat_peek(node, as_child);
    CTX.with(|c| {
        if let Ok(mut c) = c.try_borrow_mut()
            && let Some(c) = c.as_mut()
        {
            if v.is_some() {
                c.stats.intr_hits += 1;
            } else {
                c.stats.intr_misses += 1;
            }
        }
    });
    v
}

fn nat_peek(node: usize, as_child: bool) -> Option<(f32, f32)> {
    let cur = bver(node);
    if cur == u64::MAX {
        return None;
    }
    NAT.with(|n| n.borrow().get(&(node, as_child)).filter(|e| e.0 == cur).map(|e| (e.1, e.2)))
}

pub(crate) fn nat_put(node: usize, as_child: bool, v: (f32, f32)) {
    let cur = bver(node);
    if cur != u64::MAX {
        NAT.with(|n| {
            n.borrow_mut().insert((node, as_child), (cur, v.0, v.1))
        });
    }
}

/// The cached entries of `node` that were valid for `ver` (its version when it was last laid out).
fn nat_old(node: usize, ver: u64) -> [Option<(f32, f32)>; 2] {
    NAT.with(|n| {
        let n = n.borrow();
        let get = |c: bool| n.get(&(node, c)).filter(|e| e.0 == ver).map(|e| (e.1, e.2));
        [get(false), get(true)]
    })
}

fn nat_clear() {
    NAT.with(|n| n.borrow_mut().clear());
}

/// Entries of boxes that no longer exist or changed are dropped once the map is large.
fn nat_sweep() {
    NAT.with(|n| {
        let mut n = n.borrow_mut();
        if n.len() > 20_000 {
            n.retain(|k, e| e.0 == bver(k.0));
        }
    });
}

fn bver(node: usize) -> u64 {
    if let Some(v) = WORKER_VER.with(|w| w.borrow().as_ref().map(|w| w.bver.get(&node).copied().unwrap_or(w.base))) {
        return v;
    }
    crate::incr_box::bver(node).unwrap_or(u64::MAX)
}

fn next_ver() -> u64 {
    VER.with(|v| {
        v.set(v.get() + 1);
        v.get()
    })
}

fn same(a: Option<f32>, b: Option<f32>) -> bool {
    a.map(f32::to_bits) == b.map(f32::to_bits)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= NEAR || a.to_bits() == b.to_bits()
}

fn near_opt(a: Option<f32>, b: Option<f32>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => near(x, y),
        _ => false,
    }
}

// ---------- measure hooks (layout.rs) ----------

/// Counts one box measured for real during an engine measure.
pub(crate) fn note_remeasured() {
    CTX.with(|c| {
        if let Ok(mut c) = c.try_borrow_mut()
            && let Some(c) = c.as_mut()
        {
            c.stats.remeasured += 1;
        }
    });
}

/// Records how the element box `b` was measured on its fresh `laid`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    laid: &mut Laid,
    b: &LayoutBox,
    k: &Key,
    intr_anc: bool,
    intr_own: bool,
    floats: &Floats,
    floats_at: (usize, usize),
    emitted: &[PendingPositioned<'_>],
) {
    let mut cacheable = true;
    // QINC1k3: the floats it was measured among and the ones it placed (`Ck::fl`).
    let out = FloatSnap::of(floats, floats_at);
    let fl = (k.floats_in || !out.is_empty()).then(|| {
        let inl = if k.floats_in {
            snap_in(floats, floats_at)
        } else {
            std::sync::Arc::new(FloatSnap::default())
        };
        Box::new(CkFloats { inl, out })
    });
    let mut bubble = Vec::new();
    if let Some(z) = &laid.zb {
        for e in &z.bubble {
            let key = (e.style.node, e.style.pseudo);
            if key.0 == 0 || e.fixed || bubble.contains(&key) {
                cacheable = false;
            }
            bubble.push(key);
        }
    }
    let pending = emitted
        .iter()
        .map(|p| CkPending {
            ptr: p.b as *const LayoutBox as usize,
            node: p.b.style.node,
            pseudo: p.b.style.pseudo,
            fixed: p.fixed,
            sx: p.static_x,
            sy: p.static_y,
        })
        .collect();
    laid.ck = Some(Box::new(Ck {
        x: k.x,
        y: k.y,
        width: k.width,
        ch: k.ch,
        amo: k.amo,
        fbw: k.fbw,
        zskip: k.zskip,
        ret_x: laid.x,
        ret_y: laid.y,
        ret_w: laid.w,
        ret_h: laid.h,
        bver: bver(b.style.node),
        cacheable,
        x_sensitive: x_sensitive(laid),
        // set by `measure_inner_engine` right after (it saw `layout::Y_SENS` move)
        y_sensitive: false,
        intr: intr_anc || intr_own,
        intr_anc,
        pending,
        bubble,
        psig: PSig::of(&b.style),
        pos: None,
        ink_ok: false,
        ink_x: 0.0,
        ink_y: 0.0,
        ifb: 0.0,
        fl,
    }));
}

thread_local! {
    /// QINC1k3: the last incoming-float snapshot (consecutive boxes of a BFC share one).
    static LAST_SNAP: RefCell<Option<std::sync::Arc<FloatSnap>>> = const { RefCell::new(None) };
}

/// QINC1k3: the first `at` entries of `floats` as a shared snapshot.
fn snap_in(floats: &Floats, at: (usize, usize)) -> std::sync::Arc<FloatSnap> {
    LAST_SNAP.with(|l| {
        let mut l = l.borrow_mut();
        let (fl, fr) = (floats.left.borrow(), floats.right.borrow());
        if let Some(s) = l.as_ref()
            && crate::layout::same_entries(&s.left, &fl[..at.0])
            && crate::layout::same_entries(&s.right, &fr[..at.1])
        {
            return s.clone();
        }
        let s = std::sync::Arc::new(FloatSnap { left: fl[..at.0].to_vec(), right: fr[..at.1].to_vec() });
        *l = Some(s.clone());
        s
    })
}

/// See `Ck::x_sensitive` (element children answer from their own key; anonymous ones are looked into).
fn x_sensitive(l: &Laid) -> bool {
    if matches!(l.content, LaidContent::MixedInline(_)) {
        return true;
    }
    laid_kids(l).into_iter().any(|(_, c)| match &c.ck {
        Some(ck) => ck.x_sensitive,
        None => crate::stack::grow(|| x_sensitive(c)),
    })
}

#[cfg(test)]
fn box_inside(b: &LayoutBox, ptr: usize) -> bool {
    fn walk(b: &LayoutBox, ptr: usize) -> bool {
        let kids: Vec<&LayoutBox> = match &b.content {
            BoxContent::Blocks(v) | BoxContent::TableRow(v) => v.iter().collect(),
            BoxContent::Inline(items) => {
                items.iter().filter_map(|i| if let InlineItem::Embedded(e) = i { Some(&**e) } else { None }).collect()
            }
            BoxContent::Image(_) => Vec::new(),
        };
        kids.into_iter().any(|k| std::ptr::eq(k, ptr as *const LayoutBox) || crate::stack::grow(|| walk(k, ptr)))
    }
    crate::stack::grow(|| walk(b, ptr))
}

/// R60 8.2: a clean element box measured at the same key minus the origin: its old `Laid` moved to
/// the new origin, with its `pending` and z-index bubble re-emitted. `None` = measure it.
pub(crate) fn try_hit<'a>(b: &'a LayoutBox, k: &Key, pending: &mut Vec<PendingPositioned<'a>>, floats: &Floats) -> Option<Laid> {
    CTX.with(|c| {
        let mut c = c.try_borrow_mut().ok()?;
        let c = c.as_mut()?;
        if !c.reuse || b.style.node == c.skip {
            return None;
        }
        let node = b.style.node;
        let Some(mut old) = c.harvest.remove(&(node, 0)) else {
            if OPENED.with(|o| o.borrow().contains(&node)) {
                c.stats.miss_dirty += 1;
            } else {
                c.stats.miss_no_old += 1;
            }
            return None;
        };
        let miss = {
            let ck = old.ck.as_ref().expect("harvested element Laids have a key");
            if ck.bver != bver(node) {
                Some(&mut c.stats.miss_dirty)
            } else if !ck.cacheable {
                Some(&mut c.stats.miss_not_cacheable)
            } else if match &ck.fl {
                // QINC1k3: among floats only at the same origin with the same floats.
                Some(fl) => {
                    ck.x.to_bits() != k.x.to_bits()
                        || ck.y.to_bits() != k.y.to_bits()
                        || if k.floats_in { !fl.inl.same_as(floats) } else { !fl.inl.is_empty() }
                }
                None => k.floats_in,
            } {
                Some(&mut c.stats.miss_floats)
            } else if ck.width.to_bits() != k.width.to_bits()
                || !same(ck.ch, k.ch)
                || !same(ck.amo, k.amo)
                || !same(ck.fbw, k.fbw)
                || ck.zskip != k.zskip
                || (ck.x_sensitive && ck.x.to_bits() != k.x.to_bits())
                || !ck.bubble.iter().all(|key| c.harvest.contains_key(key))
            {
                Some(&mut c.stats.miss_key)
            } else if ck.y_sensitive && ck.y.to_bits() != k.y.to_bits() {
                c.stats.miss_key += 1;
                Some(&mut c.stats.miss_y)
            } else {
                None
            }
        };
        if let Some(m) = miss {
            *m += 1;
            // Its clean descendants can still be reused.
            open(&mut old, &mut c.harvest);
            return None;
        }
        #[cfg(test)]
        for p in &old.ck.as_ref().expect("checked").pending {
            assert!(box_inside(b, p.ptr), "QINC1h1: a cached pending box is not inside its clean box");
        }
        let ck = old.ck.as_mut().expect("checked");
        // What `measure_inner` returned: a flex/table parent stretches it again; its own ink is
        // recomputed (its children's stays valid).
        old.w = ck.ret_w;
        old.h = ck.ret_h;
        ck.ink_ok = false;
        let (ox, oy) = (k.x - ck.x, k.y - ck.y);
        let (dx, dy) = (ck.ret_x + ox - old.x, ck.ret_y + oy - old.y);
        ck.x = k.x;
        ck.y = k.y;
        for p in ck.pending.iter_mut() {
            p.sx += ox;
            p.sy += oy;
        }
        let pend = ck.pending.clone();
        let keys = std::mem::take(&mut ck.bubble);
        shift_laid_tree(&mut old, dx, dy);
        let mut bag = Vec::with_capacity(keys.len());
        for key in &keys {
            let mut e = c.harvest.remove(key).expect("checked");
            if !e.fixed {
                shift_laid_tree(&mut e, dx, dy);
            }
            bag.push(Box::new(e));
        }
        // Document-order ties: an insertion before the box renumbered it (the subtree's own order
        // is unchanged; only what the dirty ancestors sort is re-keyed).
        old.style.tree_order = b.style.tree_order;
        for e in bag.iter_mut() {
            if let Some(t) = crate::incr_box::tree_order(e.style.node, e.style.pseudo) {
                e.style.tree_order = t;
            }
        }
        let ck = old.ck.as_mut().expect("checked");
        ck.bubble = keys;
        ck.ret_x = old.x;
        ck.ret_y = old.y;
        if !bag.is_empty() {
            old.zb.get_or_insert_with(Default::default).bubble = bag;
        }
        for p in pend {
            // SAFETY: `p.ptr` is a box inside `b`'s subtree when it was recorded. `b` is clean (same
            // `bver`): the box engine moved it into the kept tree as a whole, never rebuilt or cloned
            // it, so every descendant box still lives in the same heap buffers; `b` is borrowed for 'a.
            // (Tests check the address really is inside `b` above.)
            let pb: &'a LayoutBox = unsafe { &*(p.ptr as *const LayoutBox) };
            debug_assert!(pb.style.node == p.node && pb.style.pseudo == p.pseudo);
            pending.push(PendingPositioned { b: pb, fixed: p.fixed, static_x: p.sx, static_y: p.sy });
        }
        if let Some(fl) = &old.ck.as_ref().expect("checked").fl {
            fl.out.place_into(floats);
        }
        c.stats.hits += 1;
        if dx != 0.0 || dy != 0.0 {
            c.stats.shifted += 1;
            if c.touched.len() < 4096 {
                c.touched.push((node, Touch::Shifted));
            }
        }
        Some(old)
    })
}

// ---------- harvest ----------

/// Puts `l` into the harvest: a clean element `Laid` (with its bubbled entries taken out next to it)
/// or a pseudo-element `Laid` by key; anything else is opened.
fn put(l: Laid, h: &mut HashMap<(usize, u8), Laid>) {
    crate::stack::grow(|| put_body(l, h))
}

fn put_body(mut l: Laid, h: &mut HashMap<(usize, u8), Laid>) {
    if l.w.is_nan() {
        // The hole an attempt left in an owner's list (its entry went to the harvest).
        return;
    }
    let (node, pseudo) = (l.style.node, l.style.pseudo);
    let clean = node != 0 && pseudo == 0 && l.ck.as_ref().is_some_and(|ck| ck.bver == bver(node));
    if node != 0 && (pseudo != 0 || clean) {
        if clean && let Some(z) = l.zb.as_mut() {
            for e in std::mem::take(&mut z.bubble) {
                put(*e, h);
            }
        }
        if AMBIGUOUS.with(|a| a.borrow().contains(&(node, pseudo))) {
            open(&mut l, h);
            return;
        }
        if let Some(mut prev) = h.insert((node, pseudo), l) {
            AMBIGUOUS.with(|a| a.borrow_mut().insert((node, pseudo)));
            // Two `Laid`s for one key: reuse neither as a whole.
            let mut cur = h.remove(&(node, pseudo)).expect("just inserted");
            open(&mut prev, h);
            open(&mut cur, h);
        }
        return;
    }
    if node != 0 && pseudo == 0 {
        OPENED.with(|o| o.borrow_mut().insert(node));
    }
    open(&mut l, h);
}

/// Takes `l`'s children, inline boxes, out-of-flow list and bubble into the harvest (`l` stays an
/// empty shell).
fn open(l: &mut Laid, h: &mut HashMap<(usize, u8), Laid>) {
    crate::stack::grow(|| {
        WALKED.with(|w| w.set(w.get() + 1));
        match std::mem::replace(&mut l.content, LaidContent::Blocks(Vec::new())) {
            LaidContent::Blocks(v) | LaidContent::TableRow(v) => {
                for c in v {
                    put(c, h);
                }
            }
            LaidContent::MixedInline(items) => {
                for i in items {
                    if let MixedPositioned::Embedded(e) = i {
                        put(*e, h);
                    }
                }
            }
            other => l.content = other,
        }
        for p in std::mem::take(&mut l.positioned) {
            put(p, h);
        }
        if let Some(z) = l.zb.as_mut() {
            for e in std::mem::take(&mut z.bubble) {
                put(*e, h);
            }
        }
    })
}

// ---------- ink and the root extent ----------

/// Shifts the ink of a reused subtree that moved (and where its keys say the ink is valid).
fn shift_ink(l: &mut Laid, dx: f32, dy: f32) {
    crate::stack::grow(|| {
        l.ink.l += dx;
        l.ink.r += dx;
        l.ink.t += dy;
        l.ink.b += dy;
        if let Some(ck) = l.ck.as_mut() {
            ck.ink_x += dx;
            ck.ink_y += dy;
        }
        match &mut l.content {
            LaidContent::Blocks(v) | LaidContent::TableRow(v) => {
                for c in v {
                    shift_ink(c, dx, dy);
                }
            }
            LaidContent::MixedInline(items) => {
                for i in items {
                    if let MixedPositioned::Embedded(e) = i {
                        shift_ink(e, dx, dy);
                    }
                }
            }
            _ => {}
        }
        for p in l.positioned.iter_mut() {
            if !p.fixed {
                shift_ink(p, dx, dy);
            }
        }
    })
}

/// `compute_ink` + `in_flow_bottom` in one pass that stops at every subtree whose ink is still valid
/// (shifting it when the subtree moved). Returns (ink, in-flow bottom).
fn pass(l: &mut Laid) -> (Ink, f32) {
    crate::stack::grow(|| pass_body(l))
}

fn pass_body(l: &mut Laid) -> (Ink, f32) {
    if let Some(ck) = l.ck.as_ref()
        && ck.ink_ok
    {
        let (dx, dy) = (l.x - ck.ink_x, l.y - ck.ink_y);
        let ifb = ck.ifb;
        if dx != 0.0 || dy != 0.0 {
            shift_ink(l, dx, dy);
        }
        return (l.ink, ifb + l.y);
    }
    WALKED.with(|w| w.set(w.get() + 1));
    let mut ink = Ink { l: l.x, t: l.y, r: l.x + l.w, b: l.y + l.h };
    let clips = l.style.overflow != Overflow::Visible;
    let mut ifb = l.y + l.h;
    match &mut l.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            for c in children.iter_mut() {
                let (ci, cf) = pass(c);
                if clips {
                    continue;
                }
                ink.union(ci);
                if let Some(info) = &c.sticky {
                    ink.t = ink.t.min(info.cb_top);
                    ink.b = ink.b.max(info.cb_bottom);
                }
                ifb = ifb.max(cf);
            }
        }
        LaidContent::MixedInline(items) => {
            for item in items.iter_mut() {
                match item {
                    MixedPositioned::Embedded(e) => {
                        let (ei, ef) = pass(e);
                        if !clips {
                            ink.union(ei);
                            ifb = ifb.max(ef);
                        }
                    }
                    MixedPositioned::Word { x, y, w, h, .. } => {
                        if !clips {
                            ink.union(Ink { l: *x, t: *y, r: *x + *w, b: *y + *h });
                        }
                    }
                }
            }
        }
        LaidContent::Text(_) | LaidContent::Image(_) => {}
    }
    let extra = l.scroll.filter(|i| i.user).map_or(0.0, |i| i.max_scroll());
    for p in l.positioned.iter_mut() {
        let (pi, pf) = pass(p);
        if p.fixed {
            continue;
        }
        if !clips && p.style.position != Position::Fixed {
            ifb = ifb.max(pf);
        }
        let mut pi = pi;
        pi.t -= extra;
        ink.union(pi);
    }
    l.ink = ink;
    let (x, y) = (l.x, l.y);
    if let Some(ck) = l.ck.as_mut() {
        ck.ink_ok = true;
        ck.ink_x = x;
        ck.ink_y = y;
        ck.ifb = ifb - y;
    }
    (ink, ifb)
}

/// The end of `measure_document` for the engine's tree: the root's height grows to the in-flow
/// extent, then ink, both only through what changed.
pub(crate) fn finish_root(root: &mut Laid, measured_h: f32) {
    root.h = measured_h;
    let (_, extent) = pass(root);
    if extent > root.y + root.h {
        root.h = extent - root.y;
        pass(root);
    }
}

// ---------- locating a box ----------

#[derive(Clone, Copy, Debug)]
enum Step {
    Child(usize),
    Emb(usize),
    Pos(usize),
}

fn laid_kids(l: &Laid) -> Vec<(Step, &Laid)> {
    let mut out = Vec::new();
    match &l.content {
        LaidContent::Blocks(v) | LaidContent::TableRow(v) => out.extend(v.iter().enumerate().map(|(i, c)| (Step::Child(i), c))),
        LaidContent::MixedInline(items) => {
            for (i, it) in items.iter().enumerate() {
                if let MixedPositioned::Embedded(e) = it {
                    out.push((Step::Emb(i), &**e));
                }
            }
        }
        _ => {}
    }
    out.extend(l.positioned.iter().enumerate().map(|(i, p)| (Step::Pos(i), p)));
    out
}

fn step_ref(l: &Laid, s: Step) -> &Laid {
    match (s, &l.content) {
        (Step::Child(i), LaidContent::Blocks(v) | LaidContent::TableRow(v)) => &v[i],
        (Step::Emb(i), LaidContent::MixedInline(items)) => match &items[i] {
            MixedPositioned::Embedded(e) => e,
            MixedPositioned::Word { .. } => unreachable!("located an inline box"),
        },
        (Step::Pos(i), _) => &l.positioned[i],
        _ => unreachable!("located path"),
    }
}

fn step_mut(l: &mut Laid, s: Step) -> &mut Laid {
    match (s, &mut l.content) {
        (Step::Child(i), LaidContent::Blocks(v) | LaidContent::TableRow(v)) => &mut v[i],
        (Step::Emb(i), LaidContent::MixedInline(items)) => match &mut items[i] {
            MixedPositioned::Embedded(e) => e,
            MixedPositioned::Word { .. } => unreachable!("located an inline box"),
        },
        (Step::Pos(i), _) => &mut l.positioned[i],
        _ => unreachable!("located path"),
    }
}

/// The path from `l` to the `Laid` of element `target` (searching only anonymous boxes and boxes of
/// `target`'s ancestors: hoisting moves a `Laid` up, never sideways).
fn find_laid(l: &Laid, target: usize, chain: &HashSet<usize>, out: &mut Vec<Step>) -> bool {
    crate::stack::grow(|| {
        for (s, c) in laid_kids(l) {
            if c.style.pseudo != 0 {
                continue;
            }
            if c.style.node == target {
                out.push(s);
                return true;
            }
            if c.style.node == 0 || chain.contains(&c.style.node) {
                out.push(s);
                if find_laid(c, target, chain, out) {
                    return true;
                }
                out.pop();
            }
        }
        false
    })
}

fn box_kids(b: &LayoutBox) -> Vec<&LayoutBox> {
    match &b.content {
        BoxContent::Blocks(v) | BoxContent::TableRow(v) => v.iter().collect(),
        BoxContent::Inline(items) => items.iter().filter_map(|i| if let InlineItem::Embedded(e) = i { Some(&**e) } else { None }).collect(),
        BoxContent::Image(_) => Vec::new(),
    }
}

/// The element box of `target` in the box tree and its ancestor boxes.
fn find_box<'t>(b: &'t LayoutBox, target: usize, chain: &HashSet<usize>, path: &mut Vec<&'t LayoutBox>) -> Option<&'t LayoutBox> {
    crate::stack::grow(|| {
        for c in box_kids(b) {
            if c.style.pseudo != 0 {
                continue;
            }
            if c.style.node == target {
                path.push(b);
                return Some(c);
            }
            if c.style.node == 0 || chain.contains(&c.style.node) {
                path.push(b);
                if let Some(f) = find_box(c, target, chain, path) {
                    return Some(f);
                }
                path.pop();
            }
        }
        None
    })
}

fn ancestors(h: &Handle) -> HashSet<usize> {
    let mut out = HashSet::new();
    let mut cur = parent_handle(h);
    while let Some(p) = cur {
        out.insert(node_id(&p));
        cur = parent_handle(&p);
    }
    out
}

fn is_element(h: &Handle) -> bool {
    matches!(h.data, NodeData::Element { .. })
}

// ---------- bottom-up ----------

/// What a `Blocks` parent (and anything reading through it) sees of an in-flow child.
#[derive(Debug, Clone, Copy)]
struct View {
    w: f32,
    h: f32,
    m: [f32; 4],
    fb: Option<f32>,
    empty_blocks: bool,
    zfb: Option<Option<f32>>,
    zll: Option<Option<(f32, f32)>>,
    flow_emptied: bool,
    sticky: bool,
    /// z-index (document order is kept relative: a rebuilt box was not moved).
    key: i32,
}

impl View {
    fn of(l: &Laid, cache: &ShapeCache) -> View {
        let m = &l.style.margin;
        View {
            w: l.w,
            h: l.h,
            m: [m.top.px(), m.right.px(), m.bottom.px(), m.left.px()],
            fb: first_baseline(l, cache),
            empty_blocks: matches!(&l.content, LaidContent::Blocks(v) if v.is_empty()),
            zfb: l.zb.as_ref().and_then(|z| z.fb),
            zll: l.zb.as_ref().and_then(|z| z.last_line),
            flow_emptied: l.zb.as_ref().is_some_and(|z| z.flow_emptied),
            sticky: l.sticky.is_some(),
            key: stack_key(l).0,
        }
    }

    fn same(&self, o: &View) -> bool {
        let ll = |a: Option<Option<(f32, f32)>>, b: Option<Option<(f32, f32)>>| match (a, b) {
            (None, None) | (Some(None), Some(None)) => true,
            (Some(Some(p)), Some(Some(q))) => near(p.0, q.0) && near(p.1, q.1),
            _ => false,
        };
        let fbz = |a: Option<Option<f32>>, b: Option<Option<f32>>| match (a, b) {
            (None, None) => true,
            (Some(p), Some(q)) => near_opt(p, q),
            _ => false,
        };
        near(self.w, o.w)
            && near(self.h, o.h)
            && self.m.iter().zip(o.m).all(|(a, b)| near(*a, b))
            && near_opt(self.fb, o.fb)
            && self.empty_blocks == o.empty_blocks
            && fbz(self.zfb, o.zfb)
            && ll(self.zll, o.zll)
            && self.flow_emptied == o.flow_emptied
            && self.sticky == o.sticky
            && self.key == o.key
    }
}

/// Where a z-index entry of a re-measured box sits: (depth of its owner on the located path, index
/// in the owner's list, key, z-index).
type Slot = (usize, usize, (usize, u8), Option<i32>);

enum Attempt {
    /// Spliced.
    Done,
    /// Already laid out by this update (inside another start's re-measure).
    Current,
    /// The element has no box of its own: try this ancestor (not a level).
    NoBox(usize),
    /// Try this element (0: the root).
    Up(usize),
}

/// A box whose size (or whose children's sizes) comes from the intrinsic sizes of its content.
fn reads_intrinsic(b: &LayoutBox) -> bool {
    let s = &b.style;
    let out_of_flow = matches!(s.position, Position::Absolute | Position::Fixed);
    let stretched = out_of_flow && s.inset.left.is_some() && s.inset.right.is_some() && !s.margin_auto.0 && !s.margin_auto.1;
    matches!(s.display, Display::Table | Display::Flex | Display::InlineFlex | Display::Grid)
        || s.width_kw.is_some()
        || s.min_width_kw.is_some()
        || (s.width.is_none() && (s.float != Float::None || s.display == Display::InlineBlock || (out_of_flow && !stretched)))
}

/// Whether an ancestor reads the intrinsic size of a subtree that is laid out outside its measure:
/// above an out-of-flow box (the box itself or one between it and that ancestor) the measure-time
/// `intr` flag cannot see it.
fn intrinsic_above_out_of_flow(b: &LayoutBox, bpath: &[&LayoutBox]) -> bool {
    let oof = |x: &LayoutBox| matches!(x.style.position, Position::Absolute | Position::Fixed);
    let mut crossed = oof(b);
    for a in bpath.iter().rev() {
        if crossed && reads_intrinsic(a) {
            return true;
        }
        crossed |= oof(a);
    }
    false
}

/// The bubble of a spliced box (its entries go to their owner's list); a `ZInfo` left empty goes.
#[allow(clippy::vec_box)]
fn take_entries(l: &mut Laid) -> Vec<Box<Laid>> {
    let Some(z) = l.zb.as_mut() else { return Vec::new() };
    let b = std::mem::take(&mut z.bubble);
    if z.fb.is_none() && z.last_line.is_none() && !z.flow_emptied {
        l.zb = None;
    }
    b
}

impl LayoutEngine {
    fn far() -> std::time::Instant {
        std::time::Instant::now() + std::time::Duration::from_secs(3600)
    }

    /// Lays the kept tree out from scratch (reuse off), recording every key.
    fn full(tree: &LayoutBox, fs: &mut FontSystem, vw: f32, vh: f32, req: &crate::incr_box::LayoutReq, next_id: usize) -> Option<LayoutEngine> {
        let mut cache = ShapeCache::new();
        let mut next = next_id;
        let mut inc = false;
        nat_clear();
        let (laid, root_h) = with_ctx(false, || {
            crate::layout::measure_document_engine(tree, fs, vw, vh, &mut next, &mut cache, LayoutEngine::far(), &mut inc)
        })
        .0;
        if inc {
            return None;
        }
        let live = cache.len();
        Some(LayoutEngine {
            doc_key: req.doc_key,
            base_ver: req.base_ver,
            laid: SharedLaid::new(laid),
            cache,
            next_id: next,
            vw,
            vh,
            root_h,
            root_style: tree.style,
            updates: 0,
            since_full: 0,
            live,
            stats: LayoutStats { full: true, ..Default::default() },
            touched: vec![(0, Touch::Remeasured)],
            views: HashMap::new(),
            stale: false,
            ver: next_ver(),
        })
    }

    /// One incremental update. `false` = it could not complete (start over).
    fn update(&mut self, tree: &LayoutBox, fs: &mut FontSystem, vh: f32, touched: Vec<usize>) -> bool {
        let mut why = "";
        if self.vh.to_bits() != vh.to_bits() {
            why = "viewport_height";
        } else if self.root_style != tree.style {
            why = "root_style";
        }
        let mut root = !why.is_empty();
        self.root_style = tree.style;
        self.vh = vh;
        let mut keys: Vec<usize> = Vec::new();
        let mut seen = HashSet::new();
        for k in touched {
            if k == 0 {
                if !root {
                    why = "root_content";
                }
                root = true;
            } else if seen.insert(k) {
                keys.push(k);
            }
        }
        // Outermost starts only (a re-measure covers everything inside it).
        let mut starts = Vec::new();
        for k in keys {
            let Some(h) = crate::incr_style::handle_of_key(k) else { continue };
            if !is_element(&h) {
                continue;
            }
            let mut cur = parent_handle(&h);
            let mut inside = false;
            while let Some(p) = cur {
                if seen.contains(&node_id(&p)) {
                    inside = true;
                    break;
                }
                cur = parent_handle(&p);
            }
            if !inside {
                starts.push(k);
            }
        }
        let starts_n = starts.len();
        self.views.clear();
        self.ver = next_ver();
        let mut inc = false;
        let (ok, ctx) = with_ctx(true, || {
            let mut failed: HashMap<usize, usize> = HashMap::new();
            let mut trace: Vec<String> = Vec::new();
            if !root {
                for s in starts {
                    let mut e = s;
                    let mut level = 0usize;
                    loop {
                        if e == 0 || level >= MAX_WALK {
                            why = if e == 0 { "walked_to_root" } else { "walk_too_long" };
                            root = true;
                            break;
                        }
                        let next = match failed.get(&e) {
                            Some(p) => Attempt::Up(*p),
                            None => self.attempt(tree, fs, e, &mut inc),
                        };
                        if crate::incr_style::trace_on() {
                            let r = crate::perf::UP_TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()));
                            let l = crate::incr_style::handle_of_key(e).map_or_else(|| "?".to_string(), |h| crate::incr_style::label(&h));
                            let what = match &next {
                                Attempt::Done => "done".to_string(),
                                Attempt::Up(_) => r.join("+"),
                                Attempt::NoBox(_) => "nobox".to_string(),
                                Attempt::Current => "current".to_string(),
                            };
                            trace.push(format!("{l}:{what}"));
                        }
                        if inc {
                            return false;
                        }
                        match next {
                            Attempt::Done => {
                                CTX.with(|c| {
                                    if let Some(c) = c.borrow_mut().as_mut() {
                                        c.stats.levels.push(level);
                                    }
                                });
                                break;
                            }
                            Attempt::Up(p) => {
                                failed.insert(e, p);
                                e = p;
                                level += 1;
                            }
                            Attempt::NoBox(p) => e = p,
                            Attempt::Current => break,
                        }
                    }
                    if crate::incr_style::trace_on() {
                        trace.push("|".to_string());
                    }
                    if root {
                        break;
                    }
                }
            }
            if crate::incr_style::trace_on() {
                diag!("scratchy: incr-trace: relayout starts {starts_n}, root {root} ({why}): {}", trace.join(" "));
            }
            if root {
                let mut old = std::mem::replace(&mut self.laid, SharedLaid::new(empty_laid())).into_inner();
                CTX.with(|c| {
                    if let Some(c) = c.borrow_mut().as_mut() {
                        open(&mut old, &mut c.harvest);
                        c.touched.push((0, Touch::Remeasured));
                    }
                });
                drop(old);
                let (laid, rh) = crate::layout::measure_document_engine(
                    tree, fs, self.vw, self.vh, &mut self.next_id, &mut self.cache, LayoutEngine::far(), &mut inc,
                );
                self.laid = SharedLaid::new(laid);
                self.root_h = rh;
            } else {
                finish_root(&mut self.laid, self.root_h);
            }
            !inc
        });
        self.stats = ctx.stats;
        self.stats.root = root;
        self.stats.root_why = why;
        self.stats.starts = starts_n;
        self.touched = ctx.touched;
        ok
    }

    /// R60 8.1: re-measure element `e`'s box alone at its old key; splice it when its parent view
    /// is unchanged, else say which element to try next.
    fn attempt(&mut self, tree: &LayoutBox, fs: &mut FontSystem, e: usize, inc: &mut bool) -> Attempt {
        let Some(h) = crate::incr_style::handle_of_key(e) else { return Attempt::Up(0) };
        let dom_up = || parent_handle(&h).filter(is_element).map_or(0, |p| node_id(&p));
        let chain = ancestors(&h);
        let mut bpath = Vec::new();
        let Some(nb) = find_box(tree, e, &chain, &mut bpath) else { return Attempt::NoBox(dom_up()) };
        let parent_el = bpath.iter().rev().find(|b| b.style.node != 0 && b.style.pseudo == 0).map_or(0, |b| b.style.node);
        let parent_box = *bpath.last().expect("find_box pushes the parent");
        let mut lpath = Vec::new();
        if !find_laid(&self.laid, e, &chain, &mut lpath) {
            note(|s| s.miss_no_old += 1);
            crate::perf::incr_up_reason("new_box");
            return Attempt::Up(parent_el);
        }
        let cur = bver(e);
        let abs = matches!(nb.style.position, Position::Absolute | Position::Fixed);
        // QINC1h2: the natural widths an intrinsic-size reader above may have read (old version).
        let mut intr_old: [Option<(f32, f32)>; 2] = [None, None];
        let mut under_intr = false;
        let mut was_intr = false;
        {
            let old = at(&mut self.laid, &lpath);
            let Some(ck) = old.ck.as_ref() else { return Attempt::Up(parent_el) };
            if ck.bver == cur {
                return Attempt::Current;
            }
            // Out-of-flow boxes are measured outside their ancestors' recursion, but some intrinsic
            // walks still see them (inline-level items, table columns): any ancestor box whose size
            // comes from its content's intrinsic size keeps the change from stopping below it.
            // The natural widths the readers above saw of this box (a flavor nobody asked for has no entry:
            // no reader depends on it); compared with the new ones after the re-measure.
            if ck.intr {
                intr_old = nat_old(e, ck.bver);
                under_intr = ck.intr_anc;
                was_intr = true;
            }
            let why = if intrinsic_above_out_of_flow(nb, &bpath) {
                "intrinsic_above_out_of_flow"
            } else if ck.intr && !ck.bubble.is_empty() {
                // Its z-index entries sit in owners above; a re-measure here mostly fails on them (YouTube).
                "intrinsic_ancestor"
            } else if !ck.cacheable {
                "not_cacheable"
            } else if !matches!(parent_box.content, BoxContent::Blocks(_)) {
                "inline_or_row_parent"
            } else if abs {
                if ck.pos.is_none_or(|p| p.fixed != (nb.style.position == Position::Fixed))
                    || old.style.position != nb.style.position
                    || old.style.z_index != nb.style.z_index
                {
                    "position_changed"
                } else if ck.intr_anc {
                    "intrinsic_ancestor"
                } else {
                    ""
                }
            } else if nb.style.float != Float::None {
                "float"
            } else if matches!(parent_box.style.display, Display::Table | Display::Flex | Display::InlineFlex | Display::Grid) {
                "flex_grid_table_parent"
            } else if ck.pos.is_some() || ck.psig != PSig::of(&nb.style) {
                "parent_read_style_changed"
            } else {
                ""
            };
            if !why.is_empty() {
                note(|s| s.miss_not_cacheable += 1);
                crate::perf::incr_up_reason(why);
                return Attempt::Up(parent_el);
            }
        }
        // The old views of the box and of every box above it, before anything below them is opened
        // (a failed attempt leaves an emptied shell; the first view taken of a box is its old one).
        {
            let mut l: &Laid = &self.laid;
            for s in &lpath {
                l = step_ref(l, *s);
                if l.style.node != 0 && l.style.pseudo == 0 && !self.views.contains_key(&l.style.node) {
                    self.views.insert(l.style.node, View::of(l, &self.cache));
                }
            }
        }
        let old_view = *self.views.get(&e).expect("just taken");
        // Its old content goes to the harvest (clean children are served from it).
        // Its z-index entries sit in an owner's list above it: found by key and moved to the harvest
        // (a hole with the same key stays, never reused); a splice puts the new ones in the holes.
        let keys = at(&mut self.laid, &lpath).ck.as_ref().expect("checked").bubble.clone();
        let old_pending = at(&mut self.laid, &lpath).ck.as_ref().expect("checked").pending.clone();
        // QINC1k3: the floats it was measured among (re-measured among the same ones) and placed.
        let old_fl = at(&mut self.laid, &lpath).ck.as_ref().expect("checked").fl.clone();
        let mut slots: Vec<Slot> = Vec::new();
        for key in &keys {
            let mut found = None;
            let mut l: &Laid = &self.laid;
            let mut owners = vec![(0usize, l)];
            for (d, s) in lpath[..lpath.len() - 1].iter().enumerate() {
                l = step_ref(l, *s);
                owners.push((d + 1, l));
            }
            for (d, o) in owners.iter().rev() {
                if let Some(i) = o.positioned.iter().position(|p| (p.style.node, p.style.pseudo) == *key) {
                    found = Some((*d, i, o.positioned[i].style.z_index));
                    break;
                }
            }
            let Some((d, i, z)) = found else {
                note(|s| s.miss_not_cacheable += 1);
                crate::perf::incr_up_reason("entry_not_found");
                return Attempt::Up(parent_el);
            };
            slots.push((d, i, *key, z));
        }
        for (d, i, _, _) in &slots {
            let slot = &mut at(&mut self.laid, &lpath[..*d]).positioned[*i];
            let mut hole = empty_laid();
            hole.style = slot.style;
            hole.w = f32::NAN;
            let entry = std::mem::replace(slot, hole);
            CTX.with(|c| {
                if let Some(c) = c.borrow_mut().as_mut() {
                    put(entry, &mut c.harvest);
                }
            });
        }
        let (ck_key, pos, later) = {
            let old = at(&mut self.laid, &lpath);
            CTX.with(|c| {
                if let Some(c) = c.borrow_mut().as_mut() {
                    open(old, &mut c.harvest);
                }
            });
            let ck = old.ck.as_ref().expect("checked");
            let later = match &ck.pos {
                Some(p) if abs => (old.x - p.ret_x, old.y - p.ret_y),
                _ => (old.x - ck.ret_x, old.y - ck.ret_y),
            };
            (Key { x: ck.x, y: ck.y, width: ck.width, ch: ck.ch, amo: ck.amo, fbw: ck.fbw, zskip: ck.zskip, floats_in: false }, ck.pos, later)
        };
        CTX.with(|c| {
            if let Some(c) = c.borrow_mut().as_mut() {
                c.skip = e;
            }
        });
        let mut out = Vec::new();
        let mut floats_ok = true;
        let mut new = {
            let _z = crate::layout::z_active_guard();
            let _i = crate::layout::intr_anc_guard(under_intr);
            let _s = ck_key.zskip.then(crate::layout::z_skip_guard);
            if abs {
                let p = pos.expect("checked");
                let item = PendingPositioned { b: nb, fixed: p.fixed, static_x: p.sx, static_y: p.sy };
                resolve_positioned(item, p.cb.0, p.cb.1, p.cb.2, p.cb.3, fs, &mut self.next_id, &mut self.cache, LayoutEngine::far(), inc, &mut out)
            } else {
                let fl = old_fl.as_ref().map_or_else(Floats::default, |f| f.inl.to_floats());
                let r = crate::layout::measure_at(
                    nb, fs, ck_key.x, ck_key.y, ck_key.width, ck_key.ch, &mut self.next_id, &mut self.cache, LayoutEngine::far(), inc,
                    &mut out, &fl, ck_key.amo, ck_key.fbw,
                );
                let at_in = old_fl.as_ref().map_or((0, 0), |f| (f.inl.left.len(), f.inl.right.len()));
                let placed = FloatSnap::of(&fl, at_in);
                floats_ok = match &old_fl {
                    Some(f) => placed.same(&f.out),
                    None => placed.is_empty(),
                };
                r
            }
        };
        CTX.with(|c| {
            if let Some(c) = c.borrow_mut().as_mut() {
                c.skip = 0;
            }
        });
        if *inc {
            return Attempt::Up(0);
        }
        shift_laid_tree(&mut new, later.0, later.1);
        let new_keys: Vec<(usize, u8, Option<i32>)> =
            new.zb.as_ref().map(|z| z.bubble.iter().map(|b| (b.style.node, b.style.pseudo, b.style.z_index)).collect()).unwrap_or_default();
        let same_entries = new_keys.len() == slots.len()
            && slots.iter().all(|(_, _, k, z)| new_keys.iter().any(|n| (n.0, n.1) == *k && n.2 == *z));
        let pending_ok = self.pending_unchanged(&old_pending, &out);
        // QINC1h2: what the readers above saw of this box (its natural widths) is unchanged.
        let intr_ok = {
            let mut ok = true;
            for (i, o) in intr_old.iter().enumerate() {
                if let Some(o) = o {
                    let n = crate::layout::natural_width_probe(nb, fs, &mut self.next_id, &mut self.cache, i == 1);
                    ok &= n.is_some_and(|n| n.0.to_bits() == o.0.to_bits() && n.1.to_bits() == o.1.to_bits());
                }
            }
            ok
        };
        let fits = pending_ok
            && floats_ok
            && intr_ok
            && same_entries
            && new.ck.as_ref().is_some_and(|ck| ck.cacheable)
            && if abs { stack_key(&new).0 == old_view.key } else { View::of(&new, &self.cache).same(&old_view) };
        if !fits {
            crate::perf::incr_up_reason(if !pending_ok {
                "pending_out_changed"
            } else if !floats_ok {
                "floats_out_changed"
            } else if !intr_ok {
                "intrinsic_size_changed"
            } else if !same_entries {
                "z_entries_changed"
            } else if new.ck.as_ref().is_none_or(|ck| !ck.cacheable) {
                "new_not_cacheable"
            } else {
                "parent_view_changed"
            });
            CTX.with(|c| {
                if let Some(c) = c.borrow_mut().as_mut() {
                    put(new, &mut c.harvest);
                }
            });
            return Attempt::Up(parent_el);
        }
        // Splice: the parent's flow is unchanged, so the old box's later shift applies as is; the
        // parent's sticky bounds are kept.
        let old = at(&mut self.laid, &lpath);
        if let (Some(o), Some(n)) = (&old.sticky, &mut new.sticky) {
            n.cb_top = o.cb_top;
            n.cb_bottom = o.cb_bottom;
        }
        let mut entries = take_entries(&mut new);
        *old = new;
        for (d, i, key, _) in &slots {
            let at_i = entries.iter().position(|b| (b.style.node, b.style.pseudo) == *key).expect("same entries");
            at(&mut self.laid, &lpath[..*d]).positioned[*i] = *entries.swap_remove(at_i);
        }
        // Paint order: the lists that took new entries (and the one holding the box itself) are
        // sorted again with current document-order keys (a move changes an entry's place).
        let mut lists: Vec<usize> = slots.iter().map(|s| s.0).collect();
        if matches!(lpath.last(), Some(Step::Pos(_))) {
            lists.push(lpath.len() - 1);
        }
        lists.sort_unstable();
        lists.dedup();
        for d in lists {
            let owner = at(&mut self.laid, &lpath[..d]);
            for p in owner.positioned.iter_mut() {
                if let Some(t) = crate::incr_box::tree_order(p.style.node, p.style.pseudo) {
                    p.style.tree_order = t;
                }
            }
            owner.positioned.sort_by_key(stack_key);
        }
        // The ancestors' ink and in-flow extents are recomputed (`finish_root`).
        let mut l: &mut Laid = &mut self.laid;
        for s in &lpath[..lpath.len() - 1] {
            l = step_mut(l, *s);
            if let Some(ck) = l.ck.as_mut() {
                ck.ink_ok = false;
            }
        }
        CTX.with(|c| {
            if let Some(c) = c.borrow_mut().as_mut() {
                c.stats.splices += 1;
                c.touched.push((e, Touch::Remeasured));
            }
        });
        if was_intr {
            CTX.with(|c| {
                if let Some(c) = c.borrow_mut().as_mut() {
                    c.stats.intr_stops += 1;
                }
            });
        }
        Attempt::Done
    }

}

impl LayoutEngine {
    /// The out-of-flow boxes a re-measured box sends past itself are the same ones, at the same static
    /// positions, as before, and none of them changed: their laid boxes (in a containing block above,
    /// which the unchanged parent view keeps as it was) stand as they are.
    fn pending_unchanged(&self, old: &[CkPending], new: &[PendingPositioned<'_>]) -> bool {
        old.len() == new.len()
            && old.iter().zip(new).all(|(o, n)| {
                (o.node, o.pseudo, o.fixed) == (n.b.style.node, n.b.style.pseudo, n.fixed)
                    && near(o.sx, n.static_x)
                    && near(o.sy, n.static_y)
                    && o.pseudo == 0
                    && self.clean_laid(n.b)
            })
    }

    /// The laid box of element box `b` in the kept tree was measured from this very box (same `bver`).
    fn clean_laid(&self, b: &LayoutBox) -> bool {
        let node = b.style.node;
        let Some(h) = crate::incr_style::handle_of_key(node) else { return false };
        let mut path = Vec::new();
        if !find_laid(&self.laid, node, &ancestors(&h), &mut path) {
            return false;
        }
        let mut l: &Laid = &self.laid;
        for s in &path {
            l = step_ref(l, *s);
        }
        l.ck.as_ref().is_some_and(|ck| ck.bver == bver(node))
    }
}

fn at<'t>(root: &'t mut Laid, path: &[Step]) -> &'t mut Laid {
    let mut l = root;
    for s in path {
        l = step_mut(l, *s);
    }
    l
}

fn note(f: impl FnOnce(&mut LayoutStats)) {
    CTX.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            f(&mut c.stats);
        }
    });
}

/// QINC1k10 test hook: puts one pseudo-element `Laid` per key into a fresh harvest (as an update would)
/// and returns the keys it still serves.
#[cfg(test)]
pub(crate) fn harvest_after_puts(keys: &[(usize, u8)]) -> Vec<(usize, u8)> {
    AMBIGUOUS.with(|a| a.borrow_mut().clear());
    let mut h = HashMap::new();
    for k in keys {
        let mut l = empty_laid();
        (l.style.node, l.style.pseudo) = *k;
        put(l, &mut h);
    }
    AMBIGUOUS.with(|a| a.borrow_mut().clear());
    let mut out: Vec<_> = h.into_keys().collect();
    out.sort();
    out
}

fn empty_laid() -> Laid {
    Laid {
        style: anonymous_style(ROOT_STYLE),
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
        background_image: None,
        content: LaidContent::Blocks(Vec::new()),
        positioned: Vec::new(),
        fixed: false,
        sticky: None,
        scroll: None,
        ink: Ink::UNKNOWN,
        zb: None,
        ck: None,
    }
}

/// Clears `ENG_ON` / the context even when a measure panics.
struct OnGuard;

impl Drop for OnGuard {
    fn drop(&mut self) {
        crate::layout::ENG_ON.with(|c| c.set(false));
        CTX.with(|c| *c.borrow_mut() = None);
    }
}

/// Runs `f` as an engine measure; returns its result and the context it left.
fn with_ctx<R>(reuse: bool, f: impl FnOnce() -> R) -> (R, Ctx) {
    CTX.with(|c| {
        *c.borrow_mut() = Some(Ctx { harvest: HashMap::new(), reuse, skip: 0, stats: LayoutStats::default(), touched: Vec::new() })
    });
    crate::layout::ENG_ON.with(|c| c.set(true));
    OPENED.with(|o| o.borrow_mut().clear());
    AMBIGUOUS.with(|a| a.borrow_mut().clear());
    let guard = OnGuard;
    let r = f();
    let ctx = CTX.with(|c| c.borrow_mut().take()).expect("set above");
    drop(guard);
    (r, ctx)
}

// ---------- the shadow stage ----------

/// Called after every complete `measure_document`: when `b` is the from-scratch tree of the last
/// shadowed build, relayout the engine's kept tree and (check mode) compare with `fresh`.
pub(crate) fn after_measure(b: &LayoutBox, fs: &mut FontSystem, vw: f32, vh: f32, fresh: &Laid, fresh_cache: &ShapeCache) {
    if crate::layout::ENG_ON.with(Cell::get) {
        return;
    }
    let Some(mut req) = crate::incr_box::layout_request(b) else { return };
    let t0 = std::time::Instant::now();
    WALKED.with(|w| w.set(0));
    let eng = ENGINE.with(|e| e.borrow_mut().take());
    let next_id = eng.as_ref().map_or(0, |e| e.next_id);
    let eng = crate::incr_box::with_tree(|tree| {
        let req = &mut req;
        let tree = tree?;
        let restart = req.full
            || eng.as_ref().is_none_or(|e| {
                e.stale || e.doc_key != req.doc_key || e.base_ver != req.base_ver || e.vw.to_bits() != vw.to_bits() || e.since_full + 1 >= DRIFT_UPDATES
            });
        if restart {
            return LayoutEngine::full(tree, fs, vw, vh, req, next_id);
        }
        let mut e = eng.expect("checked");
        if e.update(tree, fs, vh, std::mem::take(&mut req.touched)) {
            e.updates += 1;
            e.since_full += 1;
            Some(e)
        } else {
            None
        }
    });
    let ns = t0.elapsed().as_nanos() as u64;
    nat_sweep();
    let Some(mut eng) = eng else { return };
    finish_update(&mut eng, ns);
    if crate::incr_style::checking() && cmp_laid(&Names::default(), "root", fresh, fresh_cache, &eng.laid, &eng.cache).is_err() {
        let names = req.doc.upgrade().map(|d| Names::from_dom(&d)).unwrap_or_default();
        let m = cmp_laid(&names, "root", fresh, fresh_cache, &eng.laid, &eng.cache).err().map(|m| m.to_string()).unwrap_or_default();
        let s = &eng.stats;
        let report = format!(
            "laid {m} (update {}, full {}, root {}, starts {}, splices {}, levels {:?}, hits {}, remeasured {})",
            eng.updates, s.full, s.root, s.starts, s.splices, s.levels, s.hits, s.remeasured
        );
        // No longer trusted: the next update starts over (before a test panics on the report).
        drop(eng);
        crate::incr_style::report_mismatches("incremental relayout differs from the from-scratch layout", vec![report]);
        return;
    }
    ENGINE.with(|e| *e.borrow_mut() = Some(eng));
}

/// Shape-cache sweep and counters after one relayout.
fn finish_update(eng: &mut LayoutEngine, ns: u64) {
    // Shape cache: drop paragraphs no live `Laid` uses once it is twice the live set.
    if eng.cache.len() > 2 * eng.live.max(64) {
        let mut live = HashSet::new();
        crate::layout::collect_live_ids(&eng.laid, &mut live);
        crate::layout::sweep_shape_cache(&mut eng.cache, &live);
        eng.live = live.len();
    }
    use crate::perf::{incr_add, Incr};
    let s = &eng.stats;
    incr_add(Incr::LayoutUpdates, 1);
    incr_add(Incr::LayoutFull, u64::from(s.full));
    incr_add(Incr::LayoutRoot, u64::from(s.root && !s.full));
    if s.root && !s.full {
        crate::perf::incr_root_reason(s.root_why);
    }
    incr_add(Incr::BoxesRemeasured, s.remeasured);
    incr_add(Incr::IntrHits, s.intr_hits);
    incr_add(Incr::IntrMisses, s.intr_misses);
    incr_add(Incr::IntrStops, s.intr_stops);
    incr_add(Incr::CacheHits, s.hits);
    incr_add(Incr::MissDirty, s.miss_dirty);
    incr_add(Incr::MissKey, s.miss_key);
    incr_add(Incr::MissY, s.miss_y);
    incr_add(Incr::MissFloats, s.miss_floats);
    incr_add(Incr::MissNotCacheable, s.miss_not_cacheable);
    incr_add(Incr::MissNoOld, s.miss_no_old);
    incr_add(Incr::LayoutShifted, s.shifted);
    incr_add(Incr::LayoutSplices, s.splices);
    for l in &s.levels {
        crate::perf::incr_stop_at(*l);
    }
    crate::perf::incr_layout_ns(ns, s.full);
}

// ---------- QINC1j: App frames (live) ----------

/// QINC1j: relayout the kept tree for an App frame, on the main thread. `false` = it cannot be done
/// incrementally (no engine, the box tree started over, viewport width, drift guard): the caller
/// lays the kept tree out on the worker; the old tree stays as the App's view until one is adopted.
pub(crate) fn live_update(fs: &mut FontSystem, vw: f32, vh: f32) -> bool {
    let Some(mut req) = crate::incr_box::take_live_request() else { return false };
    let t0 = std::time::Instant::now();
    WALKED.with(|w| w.set(0));
    let Some(mut e) = ENGINE.with(|e| e.borrow_mut().take()) else { return false };
    // QINC1i6: the main thread's shape memo serves these relayouts from the first one on (it was on
    // only after the page's first script forced layout); bounded by `memo_trim` below.
    crate::layout::SHAPE_MEMO_ENABLED.with(|m| m.set(true));
    let restart = req.full
        || e.stale
        || e.doc_key != req.doc_key
        || e.base_ver != req.base_ver
        || e.vw.to_bits() != vw.to_bits()
        || e.since_full + 1 >= DRIFT_UPDATES;
    let ok = !restart
        && crate::incr_box::with_tree(|tree| match tree {
            Some(tree) => e.update(tree, fs, vh, std::mem::take(&mut req.touched)),
            None => false,
        });
    if !ok {
        e.stale = true;
        ENGINE.with(|x| *x.borrow_mut() = Some(e));
        return false;
    }
    e.updates += 1;
    e.since_full += 1;
    finish_update(&mut e, t0.elapsed().as_nanos() as u64);
    ENGINE.with(|x| *x.borrow_mut() = Some(e));
    // QINC1i5/i6: the main thread's memo is bounded here too.
    crate::layout::memo_trim(crate::js_geom::SHAPE_MEMO_CAP);
    true
}

/// QINC1j: the engine's tree is laid out at `vw` x `vh` from the current kept box tree (see
/// `incr_box::is_current` for whether anything waits for a relayout).
pub(crate) fn is_current(vw: f32, vh: f32) -> bool {
    let Some(base) = crate::incr_box::base() else { return false };
    ENGINE.with(|e| {
        e.try_borrow().ok().is_some_and(|e| {
            e.as_ref().is_some_and(|e| {
                !e.stale && e.vw.to_bits() == vw.to_bits() && e.vh.to_bits() == vh.to_bits() && (e.doc_key, e.base_ver) == base
            })
        })
    })
}

/// QINC1j: the App's view of the kept laid tree.
pub(crate) struct LiveView {
    pub(crate) laid: std::rc::Rc<Laid>,
    /// Empty when the caller already shows this version.
    pub(crate) cache: ShapeCache,
    pub(crate) ver: u64,
}

/// QINC1j: the engine's current tree (`None`: no engine). `known` = the version the caller shows.
pub(crate) fn view(known: u64) -> Option<LiveView> {
    ENGINE.with(|e| {
        let e = e.try_borrow().ok()?;
        let e = e.as_ref()?;
        let cache = if e.ver == known { ShapeCache::new() } else { e.cache.clone() };
        Some(LiveView { laid: e.laid.0.clone(), cache, ver: e.ver })
    })
}

/// QINC1j: whether `l` is the engine's own tree.
pub(crate) fn owns(l: &std::rc::Rc<Laid>) -> bool {
    ENGINE.with(|e| e.try_borrow().ok().is_some_and(|e| e.as_ref().is_some_and(|e| std::rc::Rc::ptr_eq(&e.laid.0, l))))
}

/// QINC1j: the next paragraph id the engine hands out (the worker never goes below it).
pub(crate) fn next_id() -> usize {
    ENGINE.with(|e| e.try_borrow().ok().and_then(|e| e.as_ref().map(|e| e.next_id)).unwrap_or(0))
}

/// Clears the worker's `bver` table even when a measure panics.
struct VerGuard;

impl Drop for VerGuard {
    fn drop(&mut self) {
        WORKER_VER.with(|w| *w.borrow_mut() = None);
    }
}

/// QINC1j: the shaping worker lays out a kept-tree copy as the engine would (every element `Laid`
/// records its `Ck`, with the box versions of `ver`), so the main thread can adopt the result.
/// Returns the tree and the root's measured height.
#[allow(clippy::too_many_arguments)]
pub(crate) fn worker_measure(
    tree: &LayoutBox,
    fs: &mut FontSystem,
    vw: f32,
    vh: f32,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
    ver: &WorkerVer,
) -> (Laid, f32) {
    WORKER_VER.with(|w| *w.borrow_mut() = Some(ver.clone()));
    let _g = VerGuard;
    nat_clear(); // the worker thread's cache may hold another job's boxes
    with_ctx(false, || crate::layout::measure_document_engine(tree, fs, vw, vh, next_id, cache, deadline, incomplete)).0
}

/// QINC1j: a complete worker layout of the kept tree becomes the engine's tree.
#[allow(clippy::too_many_arguments)]
pub(crate) fn adopt(
    job: &crate::incr_box::WorkerJob,
    root_style: ComputedStyle,
    laid: std::rc::Rc<Laid>,
    root_h: f32,
    cache: ShapeCache,
    next_id: usize,
    vw: f32,
    vh: f32,
) -> u64 {
    // QINC1h2: the natural-width cache belonged to the tree the engine had (the worker filled its own).
    nat_clear();
    let live = cache.len();
    let e = LayoutEngine {
        doc_key: job.doc_key,
        base_ver: job.ver.base,
        laid: SharedLaid(laid),
        cache,
        next_id: next_id.max(self::next_id()),
        vw,
        vh,
        root_h,
        root_style,
        updates: 0,
        since_full: 0,
        live,
        stats: LayoutStats { full: true, ..Default::default() },
        touched: vec![(0, Touch::Remeasured)],
        views: HashMap::new(),
        stale: false,
        ver: next_ver(),
    };
    let ver = e.ver;
    ENGINE.with(|x| *x.borrow_mut() = Some(e));
    ver
}

/// QINC1i: relayouts the kept box tree now for a script forced layout (on the main thread, complete):
/// incrementally when the engine's tree can be updated, else from scratch through the engine (first
/// layout of a kept tree, box tree started over, viewport width, drift guard, a stale App view).
/// False when there is no kept tree or the layout could not complete (the caller lays out the old way).
pub(crate) fn update_kept(fs: &mut FontSystem, vw: f32, vh: f32) -> bool {
    if crate::layout::ENG_ON.with(Cell::get) {
        return false;
    }
    let Some(mut req) = crate::incr_box::take_live_request() else { return false };
    let t0 = std::time::Instant::now();
    WALKED.with(|w| w.set(0));
    let eng = ENGINE.with(|e| e.borrow_mut().take());
    let next_id = eng.as_ref().map_or(0, |e| e.next_id);
    let eng = crate::incr_box::with_tree(|tree| {
        let tree = tree?;
        let restart = req.full
            || eng.as_ref().is_none_or(|e| {
                e.stale || e.doc_key != req.doc_key || e.base_ver != req.base_ver || e.vw.to_bits() != vw.to_bits() || e.since_full + 1 >= DRIFT_UPDATES
            });
        if !restart {
            let mut e = eng.expect("checked");
            if e.update(tree, fs, vh, std::mem::take(&mut req.touched)) {
                e.updates += 1;
                e.since_full += 1;
                return Some(e);
            }
        }
        LayoutEngine::full(tree, fs, vw, vh, &req, next_id)
    });
    let ns = t0.elapsed().as_nanos() as u64;
    nat_sweep();
    let Some(mut eng) = eng else { return false };
    finish_update(&mut eng, ns);
    ENGINE.with(|e| *e.borrow_mut() = Some(eng));
    true
}

/// QINC1i: the kept laid tree and its shape cache (`None` before the first relayout).
pub(crate) fn with_laid<R>(f: impl FnOnce(&Laid, &ShapeCache) -> R) -> Option<R> {
    ENGINE.with(|e| {
        let e = e.try_borrow().ok()?;
        let e = e.as_ref()?;
        Some(f(&e.laid, &e.cache))
    })
}

/// QINC1i: a description of the last relayout (mismatch reports of the forced-layout check).
pub(crate) fn describe_last() -> String {
    ENGINE.with(|e| {
        e.try_borrow()
            .ok()
            .and_then(|e| {
                e.as_ref().map(|e| {
                    let s = &e.stats;
                    format!(
                        "(update {}, full {}, root {}, starts {}, splices {}, levels {:?}, hits {}, remeasured {})",
                        e.updates, s.full, s.root, s.starts, s.splices, s.levels, s.hits, s.remeasured
                    )
                })
            })
            .unwrap_or_default()
    })
}

/// QINC1i: forgets the kept laid tree (see `incr_style::drop_engines`).
pub(crate) fn drop_engine() {
    ENGINE.with(|e| *e.borrow_mut() = None);
    nat_clear();
}

/// QINC1j: an App frame went to the worker: the engine's tree is only the view shown meanwhile.
pub(crate) fn mark_stale() {
    ENGINE.with(|e| {
        if let Ok(mut e) = e.try_borrow_mut()
            && let Some(e) = e.as_mut()
        {
            e.stale = true;
        }
    });
}

/// R60 9: the subtrees the last relayout re-measured or moved (node 0: the whole tree).
#[allow(dead_code)] // read by QINC1i / QINC1o
pub(crate) fn touched_roots() -> Vec<(usize, Touch)> {
    ENGINE.with(|e| e.borrow().as_ref().map(|e| e.touched.clone()).unwrap_or_default())
}

/// `(element Laids, bytes)` of the kept laid tree and its shape cache.
pub(crate) fn mem() -> (usize, usize) {
    fn walk(l: &Laid, n: &mut usize, bytes: &mut usize) {
        *n += usize::from(l.style.node != 0);
        *bytes += std::mem::size_of::<Laid>();
        if let Some(ck) = &l.ck {
            *bytes += std::mem::size_of::<Ck>() + ck.pending.capacity() * std::mem::size_of::<CkPending>() + ck.bubble.capacity() * 16;
        }
        match &l.content {
            LaidContent::Blocks(v) | LaidContent::TableRow(v) => {
                for c in v {
                    walk(c, n, bytes);
                }
            }
            LaidContent::MixedInline(items) => {
                *bytes += items.capacity() * std::mem::size_of::<MixedPositioned>();
                for i in items {
                    if let MixedPositioned::Embedded(e) = i {
                        walk(e, n, bytes);
                    }
                }
            }
            _ => {}
        }
        for p in &l.positioned {
            walk(p, n, bytes);
        }
    }
    ENGINE.with(|e| {
        let Ok(e) = e.try_borrow() else { return (0, 0) };
        let Some(e) = e.as_ref() else { return (0, 0) };
        let (mut n, mut bytes) = (0, 0);
        crate::stack::grow(|| walk(&e.laid, &mut n, &mut bytes));
        // Paragraphs shared with the shape memo (most of them: a re-measure reuses its entries) are
        // counted there, not here.
        for v in e.cache.values() {
            if std::sync::Arc::strong_count(v) == 1
                && let Ok(g) = v.lock()
            {
                bytes += crate::layout::shaped_paragraph_bytes(&g);
            }
        }
        (n, bytes)
    })
}

/// Test / diagnostic access.
#[cfg(test)]
pub(crate) fn with_engine<R>(f: impl FnOnce(Option<&LayoutEngine>) -> R) -> R {
    ENGINE.with(|e| f(e.borrow().as_ref()))
}

#[cfg(test)]
impl LayoutEngine {
    pub(crate) fn doc_key(&self) -> usize {
        self.doc_key
    }
    pub(crate) fn laid(&self) -> &Laid {
        &self.laid
    }
    pub(crate) fn cache(&self) -> &ShapeCache {
        &self.cache
    }
}

/// Boxes the last relayout walked in its harvest and ink passes (tests: the floor is O(path)).
#[cfg(test)]
pub(crate) fn walked() -> u64 {
    WALKED.with(Cell::get)
}

#[cfg(test)]
pub(crate) fn reset() {
    ENGINE.with(|e| *e.borrow_mut() = None);
    nat_clear();
}
