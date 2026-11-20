//! QINC1f (R60 sections 4.1 and 6): the incremental restyle, in SHADOW mode.
//!
//! One `Engine` per thread keeps an element table: per element (keyed by the node's `Rc` address and
//! held by a `Weak`, so the address cannot be reused while the entry exists) the stored matches of its
//! slots (element, `::before`, `::after`, `::first-line`, `::first-letter`, `::marker`), their
//! `ComputedStyle`s, the ambient inputs the box builder threads down (decoration, link, inline
//! background, vertical-align), whether a slot is animated, flags (`RESTYLE` re-match, `RECASCADE`
//! re-cascade from the stored match, `DESC_DIRTY` path bit, `BOX` box content must be rebuilt) and the
//! box owner (the element whose `LayoutBox` holds this element's content).
//!
//! An update takes the write log (`incr_log`), turns each entry into marks through the
//! `InvalidationMap` (plus the interaction-state diff, the animated elements and `vh` readers), and
//! walks top-down along `DESC_DIRTY`: a `RESTYLE` element is matched, a `RECASCADE` element (or one whose
//! parent's inherited output, custom properties, `rem` base or opacity changed) is cascaded from its
//! stored match, everything else is skipped. Early cut-off: a style equal to the stored one gives the
//! children nothing; a change of only non-inherited fields re-cascades only children whose cascade
//! reads the parent through a CSS-wide keyword. `style_diff` turns each change into `BOX` marks for the
//! next stage (QINC1g). The counter / quote walk runs over the stored matches (R60 6.4).
//!
//! Shadow mode: every `build_layout_tree_with_resources` (always under `cfg(test)`; in release only with
//! `SCRATCHY_INCR=1` or `SCRATCHY_INCR_CHECK=1`) runs the update AFTER the from-scratch build, in an
//! environment that leaves the build's results and every per-build side table untouched. With the
//! check (tests, `SCRATCHY_INCR_CHECK=1`) the build records every style it resolved and each one must
//! equal the engine's (`cmp_styles` semantics: `node`/`tree_order` ignored). Tests panic on a mismatch;
//! check mode prints one `scratchy: incr-check: MISMATCH` line per difference, counts
//! `incr.check_mismatch` and restarts the engine from scratch. The check also reconciles the table
//! with the DOM (attributes, child lists, text, form state) so writes that never reach the log (the
//! parser, Rust-side writes in tests) are still seen; they are counted as `incr.unlogged`.
use super::*;
use crate::incr_diff::{style_changes, style_diff, StyleDiff};
use crate::incr_log::{Change, FormKind, Val};
use crate::invalidation::{ChildChange, InvalidationMap, Mark, StateKind};
use crate::style::{match_slot, seed_match, slot_pseudo, MatchList};
use markup5ever_rcdom::Node;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};

const RESTYLE: u8 = 1;
const RECASCADE: u8 = 2;
const DESC_DIRTY: u8 = 4;
const BOX: u8 = 8;
/// Why an entry was restyled in the last update (mismatch reports): the parent's output changed.
const WHY_PARENT: u8 = 16;
/// ... or the update restyled everything.
const WHY_FULL: u8 = 32;

/// Pseudo slots after the element (memo slot numbers 1..=5, `style::pseudo_slot`).
const PSEUDOS: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Off,
    /// `SCRATCHY_INCR=1`: the engine runs, nothing compares (and nothing uses) its results yet.
    #[cfg_attr(test, allow(dead_code))]
    Run,
    /// Tests and `SCRATCHY_INCR_CHECK=1`: record the build's styles and compare.
    Check,
}

#[cfg(not(test))]
fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        let on = |k: &str| std::env::var(k).is_ok_and(|v| v == "1");
        if on("SCRATCHY_INCR_CHECK") {
            Mode::Check
        } else if on("SCRATCHY_INCR") {
            Mode::Run
        } else {
            Mode::Off
        }
    })
}

#[cfg(test)]
fn mode() -> Mode {
    static ENV_OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ENV_OFF.get_or_init(|| std::env::var("SCRATCHY_SHADOW").is_ok_and(|v| v == "0")) || !SEAM.with(|s| s.get().shadow) {
        Mode::Off
    } else {
        Mode::Check
    }
}

/// Test seams (R60 section 10): shadow on/off, reconcile on/off, panic on a mismatch or keep it.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct Seam {
    pub(crate) shadow: bool,
    pub(crate) reconcile: bool,
    pub(crate) panic: bool,
}

#[cfg(test)]
thread_local! {
    static SEAM: Cell<Seam> = const { Cell::new(Seam { shadow: true, reconcile: true, panic: true }) };
    static MISMATCHES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn set_seam(s: Seam) -> Seam {
    SEAM.with(|c| c.replace(s))
}

/// Test seam: the mismatch reports collected while `Seam::panic` is off.
#[cfg(test)]
pub(crate) fn take_mismatches() -> Vec<String> {
    MISMATCHES.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// Check mode reconciles the table with the DOM; `SCRATCHY_INCR=1` alone trusts the log (no fingerprints).
fn reconcile_on() -> bool {
    #[cfg(test)]
    {
        SEAM.with(|s| s.get().reconcile)
    }
    #[cfg(not(test))]
    {
        mode() == Mode::Check
    }
}

thread_local! {
    /// The from-scratch build's resolved styles, `(node, memo slot)` -> style, while a checked build runs.
    static RECORD: RefCell<Option<HashMap<(usize, u8), ComputedStyle>>> = const { RefCell::new(None) };
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    /// Bumped by `css::reset_custom_props_arena`: stored styles hold handles into the old arena.
    static ARENA_EPOCH: Cell<u64> = const { Cell::new(0) };
}

pub(crate) fn arena_reset() {
    ARENA_EPOCH.with(|e| e.set(e.get() + 1));
}

/// Called at the start of `build_layout_tree_with_resources`: whether the shadow update runs after it
/// (and, in check mode, starts recording the build's styles).
pub(crate) fn begin_build() -> bool {
    match mode() {
        Mode::Off => false,
        Mode::Run => true,
        Mode::Check => {
            RECORD.with(|r| *r.borrow_mut() = Some(HashMap::new()));
            true
        }
    }
}

/// `resolve_style_for` reports each result here (a no-op unless a checked build is recording).
#[inline]
pub(crate) fn record(handle: &Handle, pseudo: Option<&str>, style: &ComputedStyle) {
    RECORD.with(|r| {
        if let Ok(mut r) = r.try_borrow_mut()
            && let Some(m) = r.as_mut()
            && let Some(slot) = crate::style::pseudo_slot(pseudo)
        {
            m.insert((node_id(handle), slot), *style);
        }
    });
}

// ---------- the element table ----------

/// The inputs the box builder threads down besides the inherited style (R60 4.1): what an element
/// passes to its children. Not part of any `ComputedStyle`; a change rebuilds boxes (QINC1g).
#[derive(Clone, Copy, PartialEq, Default, Debug)]
struct Ambient {
    deco: DecorationValue,
    /// Hash of the nearest link's `href` (0: none).
    href: u64,
    bg: Option<TextColor>,
    valign: VerticalAlign,
}

/// What the check-mode reconcile compares with the DOM.
struct Fp {
    class: Option<Box<str>>,
    id: Option<Box<str>>,
    /// Every other attribute: name and value hash.
    attrs: Box<[(html5ever::LocalName, u64)]>,
    /// Hash of the text children's data, in order.
    text: u64,
    /// Live form state selectors read: checked, indeterminate, option selected.
    state: u8,
}

pub(crate) struct Entry {
    node: Weak<Node>,
    /// False for `<script>`/`<style>` (never styled; kept for the walk and sibling selectors).
    styled: bool,
    flags: u8,
    /// Why it was processed in the last update (`RESTYLE`/`RECASCADE`/`WHY_*`).
    why: u8,
    m: [Option<MatchList>; 1 + PSEUDOS],
    style: Option<Box<ComputedStyle>>,
    pseudo: [Option<Box<ComputedStyle>>; PSEUDOS],
    reads_parent: bool,
    reads_vh: bool,
    animated: bool,
    box_owner: usize,
    /// What this element passes to its children.
    amb: Ambient,
    /// Keys of every child node (elements and text), in order.
    children: Box<[usize]>,
    fp: Option<Box<Fp>>,
    seen: u32,
}

impl Entry {
    fn new(h: &Handle, styled: bool) -> Entry {
        Entry {
            node: Rc::downgrade(h),
            styled,
            flags: 0,
            why: 0,
            m: Default::default(),
            style: None,
            pseudo: Default::default(),
            reads_parent: false,
            reads_vh: false,
            animated: false,
            box_owner: 0,
            amb: Ambient::default(),
            children: Box::new([]),
            fp: None,
            seen: 0,
        }
    }

    fn handle(&self) -> Option<Handle> {
        self.node.upgrade()
    }

    fn none(&self) -> bool {
        self.style.as_ref().is_none_or(|s| s.display == Display::None)
    }

    #[cfg(test)]
    pub(crate) fn style(&self) -> Option<&ComputedStyle> {
        self.style.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn box_owner(&self) -> usize {
        self.box_owner
    }
}

/// How a processed element's change reaches its children.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Policy {
    None,
    /// Only the ambient inputs changed: recompute them (box marks), no cascade.
    Ambient,
    /// Non-inherited fields changed: re-cascade the children whose cascade reads the parent.
    Readers,
    /// Inherited output / custom properties / `rem` base / opacity changed: re-cascade every child.
    All,
}

/// What the last update did (tests and the counters).
#[derive(Default, Clone, Debug)]
pub(crate) struct Stats {
    pub(crate) full: bool,
    pub(crate) rematched: u64,
    pub(crate) recascaded: u64,
    pub(crate) unlogged: u64,
    pub(crate) styled_fresh: u64,
    /// Box owners marked `BOX`.
    pub(crate) boxes: Vec<usize>,
    pub(crate) mismatches: usize,
}

impl Stats {
    pub(crate) fn restyled(&self) -> u64 {
        self.rematched + self.recascaded
    }
}

pub(crate) struct Engine {
    doc: Weak<Node>,
    doc_key: usize,
    /// The rule list's identity: `rules_cache` generation (a splice keeps the `Rc` and moves it) and
    /// address. Held as numbers only: a `Weak` would stop `rules_cache` splicing in place.
    rules_key: (u64, usize),
    vw: f32,
    vh: f32,
    arena: u64,
    inter: InteractionState,
    rfs: f32,
    table: HashMap<usize, Entry>,
    root_dirty: bool,
    /// The generated-content walk's last output (counters/quotes rule lists only).
    gen_out: Option<GeneratedOut>,
    epoch: u32,
    updates: u64,
    pub(crate) stats: Stats,
    // per-update scratch
    flagged: Vec<usize>,
    pathed: HashSet<usize>,
    kids_dirty: HashSet<usize>,
    fp_dirty: HashSet<usize>,
    covered_attr: HashSet<(usize, Box<str>)>,
    covered_kids: HashSet<usize>,
    full: bool,
}

/// Matches per slot of the element (`seed_match` input or `match_slot` output).
fn decls_of<'a>(rules: &'a [StyleRule], list: &MatchList) -> Vec<(u64, &'a Declarations)> {
    list.iter().map(|&(k, i, imp)| (k, if imp { rules[i as usize].important() } else { rules[i as usize].decls() })).collect()
}

fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn is_el(h: &Handle) -> bool {
    matches!(h.data, NodeData::Element { .. })
}

/// `tag#id.class` of an element, for reports.
pub(crate) fn label(h: &Handle) -> String {
    let NodeData::Element { name, attrs, .. } = &h.data else { return "(node)".into() };
    let mut s = name.local.to_string();
    for a in attrs.borrow().iter() {
        match a.name.local.as_ref() {
            "id" if !a.value.is_empty() => s.push_str(&format!("#{}", a.value)),
            "class" => a.value.split_whitespace().for_each(|c| s.push_str(&format!(".{c}"))),
            _ => {}
        }
    }
    s
}

fn path_of(h: &Handle) -> String {
    let mut parts = Vec::new();
    let mut cur = Some(h.clone());
    while let Some(n) = cur {
        if is_el(&n) {
            parts.push(label(&n));
        }
        cur = parent_handle(&n);
    }
    parts.reverse();
    parts.join("/")
}

fn owns_box(tag: &str, s: &ComputedStyle) -> bool {
    !matches!(s.display, Display::Inline | Display::None | Display::TableGroup)
        || matches!(tag, "img" | "svg" | "video" | "iframe" | "input" | "textarea" | "select" | "progress" | "meter" | "br")
}

/// The fields a child's cascade reads from its parent without a CSS-wide keyword: the inherited
/// properties (`InheritedStyle`), custom properties, the `rem` base and the accumulated opacity.
fn inherited_changed(a: &ComputedStyle, b: &ComputedStyle) -> bool {
    a.color != b.color
        || a.font_size != b.font_size
        || a.root_font_size != b.root_font_size
        || a.font_weight != b.font_weight
        || a.font_style != b.font_style
        || a.font_family != b.font_family
        || a.white_space != b.white_space
        || a.word_break != b.word_break
        || a.text_transform != b.text_transform
        || a.list_style_type != b.list_style_type
        || a.text_align != b.text_align
        || a.line_height != b.line_height
        || a.visibility != b.visibility
        || a.cursor != b.cursor
        || a.word_spacing != b.word_spacing
        || a.border_collapse != b.border_collapse
        || a.text_indent != b.text_indent
        || a.background_color != b.background_color
        || a.display != b.display
        || a.vertical_align != b.vertical_align
        || a.custom_props != b.custom_props
        || a.opacity != b.opacity
}

/// The style the builder hands to its children (`build_box_for_element` sets `canvas_src` first).
fn for_children(tag: &str, s: &ComputedStyle) -> InheritedStyle {
    let mut s = *s;
    s.canvas_src = match tag {
        "html" => 1,
        "body" => 2,
        _ => 0,
    };
    s.inherited()
}

fn child_ambient(h: &Handle, tag: &str, s: &ComputedStyle, parent: &Ambient) -> Ambient {
    let deco = resolve_own_decoration(s, parent.deco);
    let href = if matches!(tag, "a" | "area") {
        element_attr(h, "href").map_or(parent.href, |v| hash_str(&v) | 1)
    } else {
        parent.href
    };
    if owns_box(tag, s) {
        Ambient { deco, href, bg: None, valign: VerticalAlign::default() }
    } else {
        let valign = if s.vertical_align == VerticalAlign::Baseline { parent.valign } else { s.vertical_align };
        Ambient { deco, href, bg: s.background_color.or(parent.bg), valign }
    }
}

fn state_bits(h: &Handle) -> u8 {
    match element_tag(h) {
        "input" => u8::from(crate::forms::live_checked(h)) | (u8::from(crate::forms::live_indeterminate(h)) << 1),
        "option" => u8::from(crate::forms_select::option_is_selected(h)) << 2,
        _ => 0,
    }
}

fn fingerprint(h: &Handle) -> Fp {
    let (mut class, mut id, mut attrs) = (None, None, Vec::new());
    if let NodeData::Element { attrs: a, .. } = &h.data {
        for at in a.borrow().iter() {
            match at.name.local.as_ref() {
                "class" => class = Some(Box::<str>::from(&*at.value)),
                "id" => id = Some(Box::<str>::from(&*at.value)),
                _ => attrs.push((at.name.local.clone(), hash_str(&at.value))),
            }
        }
    }
    let mut th = std::collections::hash_map::DefaultHasher::new();
    for c in h.children.borrow().iter() {
        if let NodeData::Text { contents } = &c.data {
            contents.borrow().hash(&mut th);
        }
    }
    Fp { class, id, attrs: attrs.into(), text: th.finish(), state: state_bits(h) }
}

fn child_keys(h: &Handle) -> Box<[usize]> {
    h.children.borrow().iter().map(node_id).collect()
}

/// The `Val` of the log as the map's `old`/`new` (a too-long value is "some other value").
fn val_str(v: &Val) -> Option<&str> {
    match v {
        Val::Absent => None,
        Val::Set(s) => Some(s),
        Val::Big => Some("\u{1}big"),
    }
}

impl Engine {
    fn new(doc: &Handle) -> Engine {
        Engine {
            doc: Rc::downgrade(doc),
            doc_key: node_id(doc),
            rules_key: (u64::MAX, 0),
            vw: f32::NAN,
            vh: f32::NAN,
            arena: ARENA_EPOCH.with(Cell::get),
            inter: InteractionState::default(),
            rfs: f32::NAN,
            table: HashMap::new(),
            root_dirty: true,
            gen_out: None,
            epoch: 0,
            updates: 0,
            stats: Stats::default(),
            flagged: Vec::new(),
            pathed: HashSet::new(),
            kids_dirty: HashSet::new(),
            fp_dirty: HashSet::new(),
            covered_attr: HashSet::new(),
            covered_kids: HashSet::new(),
            full: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn entry(&self, h: &Handle) -> Option<&Entry> {
        self.table.get(&node_id(h))
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.table.len()
    }

    fn handle_of(&self, key: usize) -> Option<Handle> {
        self.table.get(&key).and_then(Entry::handle)
    }

    // ---------- marks ----------

    fn flag(&mut self, key: usize, f: u8) {
        if let Some(e) = self.table.get_mut(&key) {
            if e.flags == 0 {
                self.flagged.push(key);
            }
            e.flags |= f;
        }
    }

    /// Marks `h` and the path above it (`DESC_DIRTY`), so the traversal reaches it.
    fn mark(&mut self, h: &Handle, f: u8) {
        self.flag(node_id(h), f);
        let mut cur = parent_handle(h);
        while let Some(p) = cur {
            let k = node_id(&p);
            if k == self.doc_key {
                self.root_dirty = true;
                break;
            }
            if !self.pathed.insert(k) {
                break;
            }
            self.flag(k, DESC_DIRTY);
            cur = parent_handle(&p);
        }
    }

    fn apply(&mut self, marks: Vec<(Handle, Mark)>) {
        for (h, m) in marks {
            match m {
                Mark::Global => self.full = true,
                Mark::Restyle => self.mark(&h, RESTYLE),
                Mark::Recascade => self.mark(&h, RECASCADE),
            }
        }
    }

    /// `BOX` on the box owner of `key` (itself when unknown).
    fn box_mark(&mut self, key: usize) {
        let owner = self.table.get(&key).map_or(key, |e| e.box_owner);
        self.flag(owner, BOX);
        self.stats.boxes.push(owner);
    }

    /// Drops the entries of `h`'s subtree (removed from the document, or no longer rendered).
    fn drop_subtree(&mut self, h: &Handle, include_self: bool) {
        if include_self {
            self.table.remove(&node_id(h));
        }
        for c in h.children.borrow().iter() {
            if is_el(c) {
                self.drop_subtree(c, true);
            }
        }
    }

    // ---------- sources ----------

    /// The parent element of the text node `text` (a reverse scan of the stored child lists).
    fn text_parent(&self, text: usize, index: &mut Option<HashMap<usize, usize>>) -> Option<usize> {
        let ix = index.get_or_insert_with(|| {
            let mut m = HashMap::new();
            for (k, e) in &self.table {
                for c in e.children.iter() {
                    m.insert(*c, *k);
                }
            }
            m
        });
        ix.get(&text).copied()
    }

    fn removal(&mut self, map: &InvalidationMap, parent: &Handle, child: usize, prev: Option<usize>, next: Option<usize>) {
        let pk = node_id(parent);
        self.kids_dirty.insert(pk);
        self.box_mark(pk);
        if !self.table.contains_key(&child) {
            // a text node (or an element of an unrendered subtree): only `:empty` sees it
            let m = map.for_text(parent);
            self.apply(m);
            return;
        }
        let kids = parent.children.borrow();
        let pos = |k: usize| kids.iter().position(|c| node_id(c) == k);
        let index = next.and_then(pos).or_else(|| prev.and_then(pos).map(|i| i + 1)).unwrap_or(kids.len());
        drop(kids);
        let node = self.handle_of(child);
        let m = map.for_children(parent, &ChildChange::Removed { node: node.clone().unwrap_or_else(|| parent.clone()), index });
        self.apply(m);
        match node {
            Some(n) => self.drop_subtree(&n, true),
            None => {
                self.table.remove(&child);
            }
        }
    }

    fn insertion(&mut self, map: &InvalidationMap, parent: &Handle, child: &Handle) {
        let pk = node_id(parent);
        self.kids_dirty.insert(pk);
        self.box_mark(pk);
        let m = if is_el(child) { map.for_children(parent, &ChildChange::Inserted(child.clone())) } else { map.for_text(parent) };
        self.apply(m);
    }

    fn attr_change(&mut self, map: &InvalidationMap, h: &Handle, name: &str, old: Option<&str>, new: Option<&str>) {
        let m = map.for_attr(h, name, old, new);
        self.apply(m);
        self.box_mark(node_id(h));
        self.fp_dirty.insert(node_id(h));
    }

    fn process_log(&mut self, map: &InvalidationMap, entries: &[crate::incr_log::Entry]) {
        let mut text_index = None;
        for e in entries {
            match &e.change {
                Change::Attr { name, old, new } => {
                    let Some(h) = self.handle_of(e.node) else { continue };
                    self.covered_attr.insert((e.node, name.clone()));
                    self.attr_change(map, &h, name, val_str(old), val_str(new));
                }
                Change::ChildInsert { child, .. } => {
                    let Some(p) = self.handle_of(e.node) else { continue };
                    self.covered_kids.insert(e.node);
                    let c = p.children.borrow().iter().find(|c| node_id(c) == *child).cloned();
                    if let Some(c) = c {
                        self.insertion(map, &p, &c);
                    }
                }
                Change::ChildRemove { child, prev, next } => {
                    let Some(p) = self.handle_of(e.node) else { continue };
                    self.covered_kids.insert(e.node);
                    self.removal(map, &p, *child, *prev, *next);
                }
                Change::Text => {
                    let Some(pk) = self.text_parent(e.node, &mut text_index) else { continue };
                    let Some(p) = self.handle_of(pk) else { continue };
                    self.covered_kids.insert(pk);
                    self.kids_dirty.insert(pk);
                    self.box_mark(pk);
                    let m = map.for_text(&p);
                    self.apply(m);
                }
                Change::TextContent | Change::InnerHtml => {
                    let Some(p) = self.handle_of(e.node) else { continue };
                    self.covered_kids.insert(e.node);
                    self.kids_dirty.insert(e.node);
                    self.box_mark(e.node);
                    let now: Vec<Handle> = p.children.borrow().clone();
                    let now_keys: HashSet<usize> = now.iter().map(node_id).collect();
                    let old: Vec<usize> = self.table.get(&e.node).map(|x| x.children.to_vec()).unwrap_or_default();
                    for k in old.into_iter().filter(|k| !now_keys.contains(k)) {
                        match self.handle_of(k) {
                            Some(n) => self.drop_subtree(&n, true),
                            None => {
                                self.table.remove(&k);
                            }
                        }
                    }
                    let m = map.for_text(&p);
                    self.apply(m);
                    for c in now.iter().filter(|c| is_el(c)) {
                        let m = map.for_children(&p, &ChildChange::Inserted(c.clone()));
                        self.apply(m);
                    }
                }
                Change::Form(kind) => {
                    let Some(h) = self.handle_of(e.node) else { continue };
                    self.box_mark(e.node);
                    self.fp_dirty.insert(e.node);
                    let targets: Vec<Handle> = if *kind == FormKind::Reset {
                        let mut v = Vec::new();
                        collect_desc(&h, &mut v);
                        v
                    } else {
                        vec![h.clone()]
                    };
                    for t in targets {
                        self.fp_dirty.insert(node_id(&t));
                        if !matches!(kind, FormKind::Value | FormKind::Selection) {
                            let m = map.for_state(&t, StateKind::Checked);
                            self.apply(m);
                            let m = map.for_state(&t, StateKind::Indeterminate);
                            self.apply(m);
                        }
                    }
                }
                Change::Write | Change::Global => self.full = true,
                // The rule list's identity decides (a new list is a full re-match).
                Change::SheetText | Change::Adopted | Change::CssVer => {}
            }
        }
    }

    fn interaction(&mut self, map: &InvalidationMap) {
        let now = INTERACTION.with(|i| i.borrow().clone());
        let old = std::mem::replace(&mut self.inter, now.clone());
        let mut hits: Vec<(usize, StateKind)> = Vec::new();
        for (o, n, k) in [(&old.hover, &now.hover, StateKind::Hover), (&old.active, &now.active, StateKind::Active), (&old.focus_within, &now.focus_within, StateKind::FocusWithin)] {
            hits.extend(o.symmetric_difference(n).map(|id| (*id, k)));
        }
        if old.focus != now.focus || old.focus_visible != now.focus_visible {
            for id in [old.focus, now.focus].into_iter().flatten() {
                hits.push((id, StateKind::Focus));
                hits.push((id, StateKind::FocusVisible));
            }
        }
        if old.target != now.target {
            hits.extend([old.target, now.target].into_iter().flatten().map(|id| (id, StateKind::Target)));
        }
        for (id, k) in hits {
            if let Some(h) = self.handle_of(id) {
                let m = map.for_state(&h, k);
                self.apply(m);
            }
        }
    }

    /// Check mode: compares the table with the DOM and marks what the log did not describe.
    fn reconcile(&mut self, map: &InvalidationMap, doc: &Handle) {
        self.epoch = self.epoch.wrapping_add(1);
        let mut stack: Vec<Handle> = vec![doc.clone()];
        while let Some(p) = stack.pop() {
            let kids: Vec<Handle> = p.children.borrow().iter().filter(|c| is_el(c)).cloned().collect();
            for c in kids {
                let k = node_id(&c);
                let Some(e) = self.table.get_mut(&k) else { continue };
                e.seen = self.epoch;
                let descend = e.styled && !e.none();
                let Some(fp) = e.fp.take() else {
                    if descend {
                        stack.push(c);
                    }
                    continue;
                };
                let now = fingerprint(&c);
                let stored_kids = e.children.clone();
                // attributes
                let mut changes: Vec<(String, Option<String>, Option<String>)> = Vec::new();
                if fp.class != now.class {
                    changes.push(("class".into(), fp.class.as_deref().map(str::to_string), now.class.as_deref().map(str::to_string)));
                }
                if fp.id != now.id {
                    changes.push(("id".into(), fp.id.as_deref().map(str::to_string), now.id.as_deref().map(str::to_string)));
                }
                let old_attrs: HashMap<&html5ever::LocalName, u64> = fp.attrs.iter().map(|(n, v)| (n, *v)).collect();
                let new_attrs: HashMap<&html5ever::LocalName, u64> = now.attrs.iter().map(|(n, v)| (n, *v)).collect();
                for (n, v) in &new_attrs {
                    if old_attrs.get(n) != Some(v) {
                        let cur = element_attr(&c, n).map(|s| s.to_string());
                        changes.push((n.to_string(), old_attrs.get(n).map(|_| "\u{1}old".to_string()), cur));
                    }
                }
                for n in old_attrs.keys().filter(|n| !new_attrs.contains_key(*n)) {
                    changes.push((n.to_string(), Some("\u{1}old".to_string()), None));
                }
                let state_changed = fp.state != now.state;
                let text_changed = fp.text != now.text;
                for (name, o, n) in changes {
                    if !self.covered_attr.contains(&(k, name.as_str().into())) {
                        self.stats.unlogged += 1;
                    }
                    self.attr_change(map, &c, &name, o.as_deref(), n.as_deref());
                }
                if state_changed {
                    self.stats.unlogged += 1;
                    self.fp_dirty.insert(k);
                    self.box_mark(k);
                    for s in [StateKind::Checked, StateKind::Indeterminate] {
                        let m = map.for_state(&c, s);
                        self.apply(m);
                    }
                }
                // child list and text
                let cur_kids = child_keys(&c);
                if (cur_kids != stored_kids || text_changed) && !self.covered_kids.contains(&k) {
                    self.stats.unlogged += 1;
                    self.kids_dirty.insert(k);
                    self.box_mark(k);
                    if text_changed {
                        let m = map.for_text(&c);
                        self.apply(m);
                    }
                    if cur_kids != stored_kids {
                        let pre = cur_kids.iter().zip(stored_kids.iter()).take_while(|(a, b)| a == b).count();
                        let max_suf = cur_kids.len().min(stored_kids.len()) - pre;
                        let suf = cur_kids.iter().rev().zip(stored_kids.iter().rev()).take(max_suf).take_while(|(a, b)| a == b).count();
                        let next = cur_kids.get(cur_kids.len() - suf).copied();
                        for rk in &stored_kids[pre..stored_kids.len() - suf] {
                            self.removal(map, &c, *rk, None, next);
                        }
                        let ins: Vec<Handle> = c.children.borrow()[pre..cur_kids.len() - suf].to_vec();
                        for n in ins {
                            self.insertion(map, &c, &n);
                        }
                    }
                }
                drop(fp);
                // The DOM as seen now is the next comparison's basis.
                if let Some(e) = self.table.get_mut(&k) {
                    e.fp = Some(Box::new(now));
                    e.children = cur_kids;
                }
                if descend {
                    stack.push(c);
                }
            }
        }
        // Entries not reached are no longer in the rendered document.
        let ep = self.epoch;
        self.table.retain(|_, e| e.seen == ep);
    }

    // ---------- the traversal ----------

    #[allow(clippy::too_many_arguments)]
    fn visit_children(&mut self, cx: &Cx, parent: &Handle, inh: &InheritedStyle, amb: &Ambient, owner: usize, policy: Policy) {
        let kids: Vec<Handle> = parent.children.borrow().iter().filter(|c| is_el(c)).cloned().collect();
        for c in kids {
            let k = node_id(&c);
            let tag = element_tag(&c);
            if tag == "script" || tag == "style" {
                let fresh = !self.table.contains_key(&k);
                let full = self.full;
                let rec = reconcile_on();
                let e = self.table.entry(k).or_insert_with(|| Entry::new(&c, false));
                if fresh || full || e.flags & RESTYLE != 0 {
                    e.m = Default::default();
                }
                if fresh || full {
                    e.children = child_keys(&c);
                    e.fp = rec.then(|| Box::new(fingerprint(&c)));
                }
                e.box_owner = owner;
                e.seen = self.epoch;
                continue;
            }
            crate::stack::grow(|| self.visit(cx, &c, k, inh, amb, owner, policy));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(&mut self, cx: &Cx, h: &Handle, k: usize, inh: &InheritedStyle, pamb: &Ambient, powner: usize, policy: Policy) {
        let rules = cx.rules;
        let exists = self.table.contains_key(&k);
        if !exists {
            self.table.insert(k, Entry::new(h, true));
        }
        let tag = element_tag(h);
        let (flags, reads_parent, old_style, old_amb) = {
            let e = &self.table[&k];
            (e.flags, e.reads_parent, e.style.as_deref().copied(), e.amb)
        };
        let new_entry = !exists || old_style.is_none();
        let need_match = self.full || new_entry || flags & RESTYLE != 0;
        let need_cascade = need_match
            || flags & RECASCADE != 0
            || policy == Policy::All
            || (policy == Policy::Readers && reads_parent);
        let mut child_policy = Policy::None;
        let mut style = old_style;
        let was_none = old_style.is_none_or(|s| s.display == Display::None);
        if need_cascade {
            let mut why = flags & (RESTYLE | RECASCADE);
            if policy >= Policy::Readers && flags & (RESTYLE | RECASCADE) == 0 {
                why |= WHY_PARENT;
            }
            if self.full {
                why |= WHY_FULL;
            }
            let m0 = if need_match {
                match_slot(h, rules, cx.vw, 0)
            } else {
                let m = self.table[&k].m[0].clone().expect("a styled entry keeps its match");
                seed_match(h, rules, cx.vw, 0, m.clone());
                m
            };
            let vh_before = crate::dom::VIEWPORT_H_READ.with(|v| v.replace(false));
            let s = resolve_style_for(h, rules, *inh, cx.vw, None);
            let reads_vh = crate::dom::VIEWPORT_H_READ.with(|v| v.replace(vh_before));
            let wide = decls_of(rules, &m0).iter().any(|(_, d)| !d.wide.is_empty())
                || element_attr(h, "style").is_some_and(|v| ["inherit", "unset", "revert", "initial"].iter().any(|w| v.contains(w)));
            if need_match {
                self.stats.rematched += 1;
            } else {
                self.stats.recascaded += 1;
            }
            // pseudo slots: they inherit from the element's own style
            let mut pseudo: [Option<Box<ComputedStyle>>; PSEUDOS] = Default::default();
            let mut ms: [Option<MatchList>; 1 + PSEUDOS] = Default::default();
            ms[0] = Some(m0);
            let mut animated = crate::anim_eval::is_animated(&s, cx.vw);
            if s.display != Display::None {
                let own = for_children(tag, &s);
                for slot in 1..=PSEUDOS as u8 {
                    let eligible = match slot {
                        3 => cx.first_line,
                        4 => cx.first_letter,
                        5 => tag == "li" || s.list_item,
                        _ => true,
                    };
                    if !eligible {
                        continue;
                    }
                    let m = match self.table[&k].m[slot as usize].clone() {
                        Some(m) if !need_match => {
                            seed_match(h, rules, cx.vw, slot, m.clone());
                            m
                        }
                        _ => match_slot(h, rules, cx.vw, slot),
                    };
                    // `::before`/`::after` exist only with a string / generated / image `content`
                    // (`generated_content`): a universal `*::before{box-sizing:..}` styles nothing.
                    let has_box = !m.is_empty()
                        && (slot > 2
                            || matches!(
                                cascade_css(&decls_of(rules, &m), None, "content", |d| d.content.clone()),
                                Cascaded::Value(ContentDecl::Text(_) | ContentDecl::Parts(_) | ContentDecl::Image(_))
                            ));
                    if has_box {
                        let ps = resolve_style_for(h, rules, own, cx.vw, slot_pseudo(slot));
                        animated |= crate::anim_eval::is_animated(&ps, cx.vw);
                        pseudo[slot as usize - 1] = Some(Box::new(ps));
                    }
                    ms[slot as usize] = Some(m);
                }
            }
            if let Some(old) = old_style {
                if inherited_changed(&old, &s) {
                    child_policy = Policy::All;
                } else if !style_changes(&old, &s).is_empty() {
                    child_policy = Policy::Readers;
                }
                match style_diff(&old, &s) {
                    StyleDiff::Box => {
                        self.box_mark(k);
                        self.flag(powner, BOX);
                        self.stats.boxes.push(powner);
                    }
                    StyleDiff::Layout if !owns_box(tag, &s) => self.box_mark(k),
                    _ => {}
                }
                if s.display == Display::None && old.display != Display::None {
                    self.drop_subtree(h, false);
                }
            } else {
                self.flag(powner, BOX);
                self.stats.boxes.push(powner);
            }
            let e = self.table.get_mut(&k).expect("inserted above");
            e.why = why;
            e.m = ms;
            e.style = Some(Box::new(s));
            e.pseudo = pseudo;
            e.reads_parent = wide;
            e.reads_vh = reads_vh;
            e.animated = animated;
            style = Some(s);
        } else if let Some(e) = self.table.get_mut(&k) {
            e.why = 0;
        }
        let s = style.expect("styled above");
        let owner = if owns_box(tag, &s) { k } else { powner };
        let amb = child_ambient(h, tag, &s, pamb);
        if amb != old_amb && !new_entry {
            child_policy = child_policy.max(Policy::Ambient);
            self.flag(owner, BOX);
            self.stats.boxes.push(owner);
        }
        let refresh = new_entry || self.full || self.kids_dirty.contains(&k);
        let rec = reconcile_on();
        {
            let e = self.table.get_mut(&k).expect("inserted above");
            e.box_owner = owner;
            e.amb = amb;
            e.seen = self.epoch;
            if refresh {
                e.children = child_keys(h);
            }
            if rec && (refresh || e.fp.is_none() || self.fp_dirty.contains(&k)) {
                e.fp = Some(Box::new(fingerprint(h)));
            }
        }
        if s.display == Display::None {
            return;
        }
        let descend = self.full || new_entry || was_none || flags & DESC_DIRTY != 0 || child_policy != Policy::None;
        if descend {
            let inh2 = for_children(tag, &s);
            self.visit_children(cx, h, &inh2, &amb, owner, child_policy);
        }
    }

    /// One incremental update after a from-scratch build of `doc`.
    fn update(&mut self, doc: &Handle, rules: &Rc<Vec<StyleRule>>, vw: f32, vh: f32) {
        self.stats = Stats::default();
        self.updates += 1;
        let log = crate::incr_log::take();
        // A list that is not the cache's current one has no generation: always a full re-match.
        let rkey = (crate::rules_cache::generation_of(rules).unwrap_or(u64::MAX), Rc::as_ptr(rules) as usize);
        self.full = self.table.is_empty()
            || log.global
            || rkey != self.rules_key
            || rkey.0 == u64::MAX
            || vw.to_bits() != self.vw.to_bits()
            || self.arena != ARENA_EPOCH.with(Cell::get);
        self.rules_key = rkey;
        self.vw = vw;
        self.arena = ARENA_EPOCH.with(Cell::get);
        let map = crate::invalidation::current().filter(|m| !m.is_global());
        if map.is_none() {
            self.full = true;
        }
        if !self.full
            && let Some(map) = map.as_ref()
        {
            self.process_log(map, &log.entries);
            self.interaction(map);
            if reconcile_on() && !self.full {
                self.reconcile(map, doc);
            }
            // animated elements are re-cascaded on every update (the clock moved)
            let anim: Vec<usize> = self.table.iter().filter(|(_, e)| e.animated).map(|(k, _)| *k).collect();
            let vh_readers: Vec<usize> =
                if vh.to_bits() != self.vh.to_bits() { self.table.iter().filter(|(_, e)| e.reads_vh).map(|(k, _)| *k).collect() } else { Vec::new() };
            for k in anim.into_iter().chain(vh_readers) {
                if let Some(h) = self.handle_of(k) {
                    self.mark(&h, RECASCADE);
                }
            }
        } else {
            self.inter = INTERACTION.with(|i| i.borrow().clone());
        }
        self.vh = vh;
        self.stats.full = self.full;
        if self.full {
            self.epoch = self.epoch.wrapping_add(1);
        }
        let (first_line, first_letter) = crate::style::PSEUDO_FLAGS
            .with(|f| f.get())
            .map(|(_, _, a, b)| (a, b))
            .unwrap_or_else(|| (crate::style::rules_have_pseudo(rules, "first-line"), crate::style::rules_have_pseudo(rules, "first-letter")));
        let cx = Cx { rules, vw, first_line, first_letter };
        // `rem`: the `<html>` element's own font-size, cascaded against the root style first.
        let mut root_policy = Policy::None;
        let html = find_html_element(doc);
        let html_dirty = html.as_ref().is_some_and(|h| {
            self.full || self.table.get(&node_id(h)).is_none_or(|e| e.flags & (RESTYLE | RECASCADE) != 0 || e.style.is_none())
        });
        if html_dirty || self.rfs.is_nan() {
            let rfs = match &html {
                Some(h) => {
                    let stored = self.table.get(&node_id(h)).filter(|e| !self.full && e.flags & RESTYLE == 0).and_then(|e| e.m[0].clone());
                    match stored {
                        Some(m) => seed_match(h, rules, vw, 0, m),
                        None => {
                            match_slot(h, rules, vw, 0);
                        }
                    }
                    let vh_before = crate::dom::VIEWPORT_H_READ.with(Cell::get);
                    let f = resolve_style(h, rules, ROOT_STYLE, vw).font_size;
                    crate::dom::VIEWPORT_H_READ.with(|v| v.set(vh_before));
                    f
                }
                None => ROOT_STYLE.font_size,
            };
            if rfs.to_bits() != self.rfs.to_bits() {
                root_policy = Policy::All;
                self.root_dirty = true;
            }
            self.rfs = rfs;
        }
        let root_inh = InheritedStyle { root_font_size: self.rfs, ..ROOT_STYLE };
        if self.full || self.root_dirty || root_policy != Policy::None {
            self.visit_children(&cx, doc, &root_inh, &Ambient::default(), 0, root_policy);
        }
        self.root_dirty = false;
        if self.full {
            let ep = self.epoch;
            self.table.retain(|_, e| e.seen == ep);
        }
        self.generated(doc, rules, vw);
        // Entries whose DOM the log or the reconcile saw change but the traversal did not reach.
        let rec = reconcile_on();
        for k in self.kids_dirty.iter().chain(self.fp_dirty.iter()) {
            if let Some(e) = self.table.get_mut(k)
                && let Some(h) = e.handle()
            {
                e.children = child_keys(&h);
                if rec {
                    e.fp = Some(Box::new(fingerprint(&h)));
                }
            }
        }
        // clear the per-update state
        for k in std::mem::take(&mut self.flagged) {
            if let Some(e) = self.table.get_mut(&k) {
                e.flags = 0;
            }
        }
        self.pathed.clear();
        self.kids_dirty.clear();
        self.fp_dirty.clear();
        self.covered_attr.clear();
        self.covered_kids.clear();
        if !reconcile_on() && self.updates.is_multiple_of(64) {
            self.sweep();
        }
        self.full = false;
    }

    /// R60 6.4: the counter / quote walk over the stored matches; marks the elements whose generated
    /// text changed. Only for rule lists that use counters, quotes or dynamic `content`.
    fn generated(&mut self, doc: &Handle, rules: &[StyleRule], vw: f32) {
        let table = &mut self.table;
        let mut matcher = |h: &Handle, slot: u8| -> Vec<(u64, &Declarations)> {
            let list = match table.get_mut(&node_id(h)) {
                Some(e) => match &e.m[slot as usize] {
                    Some(m) => {
                        seed_match(h, rules, vw, slot, m.clone());
                        m.clone()
                    }
                    None => {
                        let m = match_slot(h, rules, vw, slot);
                        e.m[slot as usize] = Some(m.clone());
                        m
                    }
                },
                None => match_slot(h, rules, vw, slot),
            };
            decls_of(rules, &list)
        };
        let mut out = GeneratedOut::default();
        if !generated_walk(doc, rules, &mut matcher, &mut out) {
            self.gen_out = None;
            return;
        }
        if let Some(old) = self.gen_out.take() {
            let mut changed: HashSet<usize> = HashSet::new();
            for (k, v) in &out.text {
                if old.text.get(k) != Some(v) {
                    changed.insert(k.0);
                }
            }
            changed.extend(old.text.keys().filter(|k| !out.text.contains_key(k)).map(|k| k.0));
            for (k, v) in &out.element {
                if old.element.get(k) != Some(v) {
                    changed.insert(*k);
                }
            }
            changed.extend(old.element.keys().filter(|k| !out.element.contains_key(k)));
            for k in changed {
                self.box_mark(k);
            }
        }
        self.gen_out = Some(out);
    }

    /// Drops entries whose node is gone or no longer in the document (run mode has no reconcile).
    fn sweep(&mut self) {
        let doc = self.doc_key;
        self.table.retain(|_, e| {
            let Some(mut n) = e.handle() else { return false };
            loop {
                match parent_handle(&n) {
                    Some(p) if node_id(&p) == doc => return true,
                    Some(p) => n = p,
                    None => return false,
                }
            }
        });
    }

    /// Membership-based ageing of animation start times (R60 6.5): keeps the start time of every
    /// animation of an element the table lists as animated, whether or not it was restyled.
    #[allow(dead_code)] // the engine owns ageing once its results are used (QINC1i/j); tests call it now
    pub(crate) fn age_animations(&self) {
        let animated: HashSet<usize> = self.table.iter().filter(|(_, e)| e.animated).map(|(k, _)| *k).collect();
        crate::anim_eval::retain_starts(|node, _| animated.contains(&node));
    }

    /// Compares every style the from-scratch build resolved with the table: the mismatch reports.
    fn compare(&self, fresh: &HashMap<(usize, u8), ComputedStyle>) -> Vec<String> {
        let mut out = Vec::new();
        for (&(k, slot), fs) in fresh {
            let e = self.table.get(&k);
            let es = e.and_then(|e| if slot == 0 { e.style.as_deref() } else { e.pseudo[slot as usize - 1].as_deref() });
            let what = match es {
                None => Some(format!("no {} style in the table", slot_pseudo(slot).unwrap_or("element"))),
                Some(es) => style_changes(fs, es)
                    .into_iter()
                    .next()
                    .map(|c| format!("slot {} field {}: fresh {} vs incremental {}", slot_pseudo(slot).unwrap_or("element"), c.field, c.old, c.new)),
            };
            if let Some(w) = what {
                let path = e.and_then(Entry::handle).map_or_else(|| format!("(node {k:#x})"), |h| path_of(&h));
                let why = e.map_or(0, |e| e.why);
                out.push(format!(
                    "{path}: {w} (update {}, full {}, why: restyle {} recascade {} parent {} full {})",
                    self.updates,
                    self.stats.full,
                    why & RESTYLE != 0,
                    why & RECASCADE != 0,
                    why & WHY_PARENT != 0,
                    why & WHY_FULL != 0
                ));
                if out.len() >= 20 {
                    break;
                }
            }
        }
        out
    }
}

fn collect_desc(h: &Handle, out: &mut Vec<Handle>) {
    for c in h.children.borrow().iter() {
        if is_el(c) {
            out.push(c.clone());
            collect_desc(c, out);
        }
    }
}

struct Cx<'a> {
    rules: &'a [StyleRule],
    vw: f32,
    first_line: bool,
    first_letter: bool,
}

/// Saved per-build state the shadow run must not disturb.
struct Env {
    flags: (u8, bool, usize),
    perf: bool,
    anim: bool,
    #[cfg(test)]
    counters: [usize; 6],
}

impl Env {
    fn enter(rules: &Rc<Vec<StyleRule>>) -> Env {
        let flags = (
            crate::dom::STATE_RULES.with(Cell::get),
            crate::dom::VIEWPORT_H_READ.with(Cell::get),
            crate::style::STYLE_ORDER.with(Cell::get),
        );
        #[cfg(test)]
        let counters = [
            crate::style::MATCH_RUNS.with(Cell::get),
            crate::style::VAR_EXPANSIONS.with(Cell::get),
            crate::style::VAR_EXPAND_HITS.with(Cell::get),
            crate::css::CUSTOM_PROPS_MEMO_HITS.with(Cell::get),
            crate::css::BLOCKS_EXPANDED.with(Cell::get),
            crate::style::PSEUDO_SCANS.with(Cell::get),
        ];
        // The same per-build tables a build sets up (the build just cleared them).
        let derived = crate::style::RULE_DERIVED.with(|d| {
            let mut d = d.borrow_mut();
            let x = d.as_mut().filter(|x| Rc::ptr_eq(&x.rules, rules))?;
            Some((x.index.take(), x.first_line, x.first_letter))
        });
        if let Some((index, fl, flt)) = derived {
            if let Some(ix) = index {
                crate::style::RULE_INDEX.with(|ri| *ri.borrow_mut() = Some((rules.as_ptr() as usize, rules.len(), ix)));
            }
            crate::style::PSEUDO_FLAGS.with(|f| f.set(Some((rules.as_ptr() as usize, rules.len(), fl, flt))));
        }
        crate::style::MATCH_MEMO.with(|m| *m.borrow_mut() = Some(Default::default()));
        crate::style::VAR_EXPAND_MEMO.with(|m| *m.borrow_mut() = Some(Default::default()));
        crate::style::ANCESTOR_BLOOM.with(|b| *b.borrow_mut() = Some(Default::default()));
        crate::dom::CLASS_CACHE.with(|c| *c.borrow_mut() = Some(Default::default()));
        Env {
            flags,
            perf: crate::perf::suspend_forced(),
            anim: crate::anim_eval::set_readonly(true),
            #[cfg(test)]
            counters,
        }
    }

    fn exit(self, rules: &Rc<Vec<StyleRule>>) {
        if let Some((_, _, index)) = crate::style::RULE_INDEX.with(|ri| ri.borrow_mut().take()) {
            crate::style::RULE_DERIVED.with(|d| {
                if let Some(x) = d.borrow_mut().as_mut()
                    && Rc::ptr_eq(&x.rules, rules)
                {
                    x.index = Some(index);
                }
            });
        }
        crate::style::PSEUDO_FLAGS.with(|f| f.set(None));
        crate::style::MATCH_MEMO.with(|m| *m.borrow_mut() = None);
        crate::style::VAR_EXPAND_MEMO.with(|m| *m.borrow_mut() = None);
        crate::style::ANCESTOR_BLOOM.with(|b| *b.borrow_mut() = None);
        crate::dom::CLASS_CACHE.with(|c| *c.borrow_mut() = None);
        crate::anim_eval::set_readonly(self.anim);
        crate::perf::resume_forced(self.perf);
        crate::dom::STATE_RULES.with(|c| c.set(self.flags.0));
        crate::dom::VIEWPORT_H_READ.with(|c| c.set(self.flags.1));
        crate::style::STYLE_ORDER.with(|c| c.set(self.flags.2));
        #[cfg(test)]
        {
            let c = self.counters;
            crate::style::MATCH_RUNS.with(|x| x.set(c[0]));
            crate::style::VAR_EXPANSIONS.with(|x| x.set(c[1]));
            crate::style::VAR_EXPAND_HITS.with(|x| x.set(c[2]));
            crate::css::CUSTOM_PROPS_MEMO_HITS.with(|x| x.set(c[3]));
            crate::css::BLOCKS_EXPANDED.with(|x| x.set(c[4]));
            crate::style::PSEUDO_SCANS.with(|x| x.set(c[5]));
        }
    }
}

/// Called at the end of `build_layout_tree_with_resources` when `begin_build` said so: the shadow
/// update, then (check mode) the comparison with what the build resolved.
pub(crate) fn after_build(doc: &Handle, rules: &Rc<Vec<StyleRule>>, vw: f32, vh: f32) {
    let fresh = RECORD.with(|r| r.borrow_mut().take());
    let t0 = std::time::Instant::now();
    let t_phase = crate::perf::phase(crate::perf::Phase::IncrShadow);
    let env = Env::enter(rules);
    let report = ENGINE.with(|e| {
        let mut slot = e.borrow_mut();
        if slot.as_ref().is_none_or(|e| e.doc_key != node_id(doc) || e.doc.upgrade().is_none()) {
            *slot = Some(Engine::new(doc));
        }
        let eng = slot.as_mut().expect("just set");
        eng.update(doc, rules, vw, vh);
        let styled = fresh.as_ref().map_or(0, |f| f.keys().filter(|k| k.1 == 0).count() as u64);
        eng.stats.styled_fresh = styled;
        let mut mism = fresh.as_ref().map(|f| eng.compare(f)).unwrap_or_default();
        if fresh.is_some() {
            // R60 6.4: the counter / quote walk over stored matches equals the build's own walk.
            let text = crate::images::GENERATED_TEXT.with(|g| g.borrow().clone());
            let element = crate::images::ELEMENT_CONTENT.with(|g| g.borrow().clone());
            let mine = eng.gen_out.as_ref();
            let same = match mine {
                Some(g) => g.text == text && g.element == element,
                None => text.is_empty() && element.is_empty(),
            };
            if !same {
                let mut diffs: Vec<String> = Vec::new();
                let empty = GeneratedOut::default();
                let g = mine.unwrap_or(&empty);
                for (k, v) in &text {
                    if g.text.get(k) != Some(v) {
                        diffs.push(format!("{:#x}{}: fresh {v:?} vs incremental {:?}", k.0, if k.1 { "::after" } else { "::before" }, g.text.get(k)));
                    }
                }
                for (k, v) in &g.text {
                    if !text.contains_key(k) {
                        diffs.push(format!("{:#x}: only incremental {v:?}", k.0));
                    }
                }
                mism.push(format!("generated content differs (update {}): {}", eng.updates, diffs.join("; ")));
            }
        }
        eng.stats.mismatches = mism.len();
        (eng.stats.clone(), mism)
    });
    env.exit(rules);
    drop(t_phase);
    let (stats, mism) = report;
    use crate::perf::{incr_add, Incr};
    incr_add(Incr::Updates, 1);
    incr_add(Incr::RestyledMatched, stats.rematched);
    incr_add(Incr::RestyledCascaded, stats.recascaded);
    incr_add(Incr::StyledFresh, stats.styled_fresh);
    incr_add(Incr::Unlogged, stats.unlogged);
    incr_add(Incr::GlobalUpdates, u64::from(stats.full));
    crate::perf::incr_restyle_sample(stats.restyled(), stats.styled_fresh, t0.elapsed().as_nanos() as u64);
    if mism.is_empty() {
        return;
    }
    incr_add(Incr::CheckMismatch, mism.len() as u64);
    // The engine is no longer trusted: the next update starts from scratch.
    ENGINE.with(|e| *e.borrow_mut() = None);
    #[cfg(test)]
    {
        if SEAM.with(|s| s.get().panic) {
            panic!("incremental restyle differs from the from-scratch build:\n{}", mism.join("\n"));
        }
        MISMATCHES.with(|m| m.borrow_mut().extend(mism));
    }
    #[cfg(not(test))]
    for m in mism {
        diag!("scratchy: incr-check: MISMATCH {m}");
    }
}

/// QOBS7-style estimate of the element table: `(entries, bytes)` (match lists counted per entry
/// even when an `Rc` is shared, so an upper bound).
pub(crate) fn mem() -> (usize, usize) {
    ENGINE.with(|e| {
        let Ok(e) = e.try_borrow() else { return (0, 0) };
        let Some(e) = e.as_ref() else { return (0, 0) };
        let cs = std::mem::size_of::<ComputedStyle>();
        let mut bytes = e.table.capacity() * (std::mem::size_of::<Entry>() + std::mem::size_of::<usize>() + 8);
        for x in e.table.values() {
            bytes += x.style.as_ref().map_or(0, |_| cs);
            bytes += x.pseudo.iter().flatten().count() * cs;
            bytes += x.children.len() * std::mem::size_of::<usize>();
            bytes += x.m.iter().flatten().map(|m| 40 + m.len() * 16).sum::<usize>();
            bytes += x.fp.as_ref().map_or(0, |f| {
                std::mem::size_of::<Fp>()
                    + f.attrs.len() * 16
                    + f.class.as_ref().map_or(0, |s| s.len())
                    + f.id.as_ref().map_or(0, |s| s.len())
            });
        }
        (e.table.len(), bytes)
    })
}

/// Test / diagnostic access to the engine of this thread.
#[cfg(test)]
pub(crate) fn with_engine<R>(f: impl FnOnce(Option<&Engine>) -> R) -> R {
    ENGINE.with(|e| f(e.borrow().as_ref()))
}

/// Test seam: forget the engine (the next build starts it from scratch).
#[cfg(test)]
pub(crate) fn reset() {
    ENGINE.with(|e| *e.borrow_mut() = None);
}
