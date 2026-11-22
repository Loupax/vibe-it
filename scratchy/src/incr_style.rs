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
    /// Default: the engine runs, nothing compares its results with a from-scratch build.
    #[cfg_attr(test, allow(dead_code))]
    Run,
    /// Tests and `SCRATCHY_INCR_CHECK=1`: record the build's styles and compare.
    Check,
}

#[cfg(not(test))]
fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| if std::env::var("SCRATCHY_INCR_CHECK").is_ok_and(|v| v == "1") { Mode::Check } else { Mode::Run })
}

/// Tests always check (QINC1m: the engine cannot be turned off any more).
#[cfg(test)]
fn mode() -> Mode {
    Mode::Check
}

/// Test seams (R60 section 10): reconcile on/off, panic on a mismatch or keep it.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct Seam {
    pub(crate) reconcile: bool,
    pub(crate) panic: bool,
}

#[cfg(test)]
thread_local! {
    static SEAM: Cell<Seam> = const { Cell::new(Seam { reconcile: true, panic: true }) };
    static MISMATCHES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// QINC1i4: check mode cascades every adopted style again and compares (off: adopt as a
    /// default run does; the comparisons with the build then see the adopted styles themselves).
    static ADOPT_VERIFY: Cell<bool> = const { Cell::new(true) };
}

/// Test seam: verify adopted styles (default) or take them as a default (non-check) run does.
#[cfg(test)]
pub(crate) fn set_adopt_verify(on: bool) -> bool {
    ADOPT_VERIFY.with(|c| c.replace(on))
}

/// Whether a full update cascades the styles it adopts anyway and compares them.
fn adopt_verify() -> bool {
    #[cfg(test)]
    {
        checking() && ADOPT_VERIFY.with(Cell::get)
    }
    #[cfg(not(test))]
    {
        checking()
    }
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

/// Check mode reconciles the table with the DOM; a default run trusts the log (no fingerprints).
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

/// QINC1i4: what the from-scratch build resolved for one `(node, memo slot)`: the style and whether
/// resolving it read the viewport height (`VIEWPORT_H_READ` within that one resolve).
/// Boxed: a restart moves the box into the element table (no second copy of every style).
pub(crate) struct Rec {
    style: Box<ComputedStyle>,
    vh: bool,
}

type Record = HashMap<(usize, u8), Rec>;

thread_local! {
    /// The from-scratch build's resolved styles, `(node, memo slot)` -> style, while a shadowed build
    /// runs: check mode compares them with the engine's; a full shadow update (a restart) ADOPTS them
    /// instead of cascading every element again (QINC1i4).
    static RECORD: RefCell<Option<Record>> = const { RefCell::new(None) };
    /// QINC1i4: `RECORD` is collecting (one `Cell` test per resolve when off).
    static RECORDING: Cell<bool> = const { Cell::new(false) };
    /// QAPI6c: a recording build's selector matching evaluated a height media query (`MediaQuery::reads_height`).
    /// Kept out of the per-resolve flag and folded into `VIEWPORT_H_READ` by `take_record`.
    static MATCH_VH: Cell<bool> = const { Cell::new(false) };
    /// The last record's size (the next one is sized for it: no rehash while the build runs).
    static RECORD_LEN: Cell<usize> = const { Cell::new(0) };
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    /// Bumped by `css::reset_custom_props_arena`: stored styles hold handles into the old arena.
    static ARENA_EPOCH: Cell<u64> = const { Cell::new(0) };
}

thread_local! {
    /// QINC1i: a from-scratch build that is the ORACLE of an engine update runs no shadow update.
    static SUPPRESS: Cell<bool> = const { Cell::new(false) };
}

/// QINC1i: runs `f` (a from-scratch build used to check an engine update) without the shadow update.
pub(crate) fn without_shadow<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            SUPPRESS.with(|s| s.set(self.0));
        }
    }
    let _restore = Restore(SUPPRESS.with(|s| s.replace(true)));
    f()
}

/// QINC1i: forgets every stage's kept state (a checked engine update differed from the from-scratch
/// result): the next update starts over from a from-scratch build.
pub(crate) fn drop_engines() {
    ENGINE.with(|e| *e.borrow_mut() = None);
    crate::incr_box::drop_engine();
    crate::incr_layout::drop_engine();
}

pub(crate) fn arena_reset() {
    ARENA_EPOCH.with(|e| e.set(e.get() + 1));
}

/// Called at the start of `build_layout_tree_with_resources`: whether the shadow update runs after it;
/// if so, starts recording the build's styles (check mode compares them, a restart adopts them).
pub(crate) fn begin_build() -> bool {
    if SUPPRESS.with(Cell::get) {
        return false;
    }
    RECORD.with(|r| *r.borrow_mut() = Some(HashMap::with_capacity(RECORD_LEN.with(Cell::get))));
    RECORDING.with(|r| r.set(true));
    MATCH_VH.with(|m| m.set(false));
    true
}

/// Stops recording and returns what the build resolved.
fn take_record() -> Option<Record> {
    RECORDING.with(|r| r.set(false));
    if MATCH_VH.with(|m| m.replace(false)) {
        crate::dom::note_viewport_height_read();
    }
    let r = RECORD.with(|r| r.borrow_mut().take());
    RECORD_LEN.with(|n| n.set(r.as_ref().map_or(0, HashMap::len)));
    r
}

/// QINC1i4: `resolve_style_for` calls this before resolving: while a build records, the viewport-height
/// read flag is cleared for the one resolve (the token holds the flag as it was); `None` otherwise.
#[inline]
pub(crate) fn record_begin() -> Option<bool> {
    if RECORDING.with(Cell::get) { Some(crate::dom::VIEWPORT_H_READ.with(|v| v.replace(false))) } else { None }
}

/// QAPI6c: selector matching evaluates a rule's media list through this. A height media query decides
/// WHICH rules match, not what a matched declaration computes to: the engine re-matches in full when
/// the height changes and some rule's media reads it (`Engine::update`), so the read is no element's
/// own `reads_vh`. Matches are memoized per build, and an element can be matched inside its one resolve
/// in the build (`display: none`: resolved once) but before its resolve in the engine (memo hit): the
/// read would land in the build's per-resolve flag only ("adopted element style reads vh: build true vs
/// engine false"). While a build records, the read is kept aside (`MATCH_VH`) and reaches the build's
/// global flag at `take_record`; otherwise it is the plain global note, as before.
#[inline]
pub(crate) fn match_media(f: impl FnOnce() -> bool) -> bool {
    if !RECORDING.with(Cell::get) {
        return f();
    }
    let before = crate::dom::VIEWPORT_H_READ.with(|v| v.replace(false));
    let ok = f();
    if crate::dom::VIEWPORT_H_READ.with(|v| v.replace(before)) {
        MATCH_VH.with(|m| m.set(true));
    }
    ok
}

/// `resolve_style_for` reports each result here with `record_begin`'s token (a no-op unless a build
/// records); the viewport-height read flag gets back its old value OR this resolve's read.
#[inline]
pub(crate) fn record(handle: &Handle, pseudo: Option<&str>, style: &ComputedStyle, token: Option<bool>) {
    let Some(before) = token else { return };
    let vh = crate::dom::VIEWPORT_H_READ.with(|v| v.replace(before || v.get()));
    RECORD.with(|r| {
        if let Ok(mut r) = r.try_borrow_mut()
            && let Some(m) = r.as_mut()
            && let Some(slot) = crate::style::pseudo_slot(pseudo)
        {
            m.insert((node_id(handle), slot), Rec { style: Box::new(*style), vh });
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
    /// QINC1k3: what the counter / quote walk reads of this element (`gen_sig`; 0 = nothing).
    gsig: u64,
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
            // `<script>` / `<style>` keep no matches: the walk may read anything of them.
            gsig: if styled { 0 } else { GEN_OPAQUE },
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
    /// QINC1j-ab: the counter / quote walk ran (skipped when nothing it reads changed).
    pub(crate) gen_walked: bool,
    /// QINC1i4: styles (element and pseudo slots) a full update after a from-scratch build took from
    /// the build instead of cascading them again; `resolved`: styles it cascaded itself (any update).
    pub(crate) adopted: u64,
    pub(crate) resolved: u64,
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
    /// QINC1g: elements whose box content may have changed in this update (box owners marked `BOX`,
    /// elements whose style / pseudo styles / matches / ambient changed, new entries). The box stage
    /// rebuilds every box on the path from these to the root.
    touched: Vec<usize>,
    /// QINC1g: elements whose child list or text changed in this update (list markers count siblings).
    kids_changed: HashSet<usize>,
    /// QAPI6b: the unstyled subtrees `unentried` answered in this update.
    has_any_done: HashSet<usize>,
    /// QAPI6b: elements inserted under a styled parent in this update's log.
    inserted_now: HashSet<usize>,
    /// QINC1j-ab: what the counter / quote walk reads changed in this update (a DOM write, a stored match,
    /// an entry); otherwise its output is the last one and the document-wide walk is skipped.
    gen_dirty: bool,
    /// QINC1i4 check mode: adopted styles that differ from the engine's own cascade.
    adopt_mism: Vec<String>,
}

/// QINC1k3: `gsig` of an element the walk reads in full (unknown matches).
const GEN_OPAQUE: u64 = 1;
/// QINC1k3: `gsig` bit: a `content` of the element reads its attributes (`attr()`).
const GEN_ATTR: u64 = 1 << 63;

/// QINC1k3: what `images::generated_walk` reads of an element, from the same matches it is given
/// (slots 0-2): `display: none`, `quotes`, the element's own `content`, the counter properties, the
/// `::before` / `::after` `content` parts, and the `ol` / `ul` / `li` implicit `list-item` counter. 0 when
/// none of them is present (the walk passes its state through such an element unchanged), else a hash
/// (with `GEN_ATTR` when a `content` reads `attr()`). An update whose elements all keep their `gsig`,
/// with no element of non-zero `gsig` inserted, moved or removed and no attribute of a `GEN_ATTR`
/// element changed, leaves the walk's output as it was: the document-wide walk is skipped.
fn gen_sig(tag: &str, rules: &[StyleRule], ms: &[Option<MatchList>; 1 + PSEUDOS]) -> u64 {
    use crate::css::{ContentDecl, ContentPart};
    use crate::style::{cascade_css, Cascaded};
    let mut parts: Vec<String> = Vec::new();
    let mut attr = false;
    let mut reads_attr = |p: &[ContentPart]| attr |= p.iter().any(|x| matches!(x, ContentPart::Attr(_)));
    if let Some(m) = &ms[0] {
        let own = decls_of(rules, m);
        if matches!(cascade_css(&own, None, "display", |d| d.display), Cascaded::Value(Display::None)) {
            parts.push("none".into());
        }
        if let Cascaded::Value(q) = cascade_css(&own, None, "quotes", |d| d.quotes.clone()) {
            parts.push(format!("q{q:?}"));
        }
        match cascade_css(&own, None, "content", |d| d.content.clone()) {
            Cascaded::Value(ContentDecl::Text(t)) => parts.push(format!("t{t:?}")),
            Cascaded::Value(ContentDecl::Parts(p)) => {
                reads_attr(&p);
                parts.push(format!("p{p:?}"));
            }
            _ => {}
        }
        for (n, v) in [
            ("r", cascade_css(&own, None, "counter-reset", |d| d.counter_reset.clone())),
            ("s", cascade_css(&own, None, "counter-set", |d| d.counter_set.clone())),
            ("i", cascade_css(&own, None, "counter-increment", |d| d.counter_increment.clone())),
        ] {
            if let Cascaded::Value(ops) = v {
                parts.push(format!("{n}{ops:?}"));
            }
        }
    }
    for slot in [1usize, 2] {
        if let Some(m) = &ms[slot]
            && let Cascaded::Value(ContentDecl::Parts(p)) = cascade_css(&decls_of(rules, m), None, "content", |d| d.content.clone())
        {
            reads_attr(&p);
            parts.push(format!("{slot}{p:?}"));
        }
    }
    if matches!(tag, "ol" | "ul" | "li") {
        parts.push(tag.into());
    }
    if parts.is_empty() {
        return 0;
    }
    let h = hash_str(&parts.join("\u{1}")) & !GEN_ATTR;
    h.max(2) | if attr { GEN_ATTR } else { 0 }
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
        || a.svg != b.svg
        || a.cursor != b.cursor
        || a.word_spacing != b.word_spacing
        || a.letter_spacing != b.letter_spacing
        || a.direction != b.direction
        || a.border_spacing != b.border_spacing
        || a.border_collapse != b.border_collapse
        || a.text_indent != b.text_indent
        || a.background_color != b.background_color
        || a.display != b.display
        || a.vertical_align != b.vertical_align
        || a.custom_props != b.custom_props
        || a.opacity != b.opacity
        || a.text_shadow != b.text_shadow
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
            touched: Vec::new(),
            kids_changed: HashSet::new(),
            has_any_done: HashSet::new(),
            inserted_now: HashSet::new(),
            gen_dirty: false,
            adopt_mism: Vec::new(),
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
        self.touched.push(key);
        self.touched.push(owner);
    }

    /// `BOX` on `owner` because of a change of `key` (QINC1g: both are touched).
    fn box_mark_owner(&mut self, key: usize, owner: usize) {
        self.flag(owner, BOX);
        self.stats.boxes.push(owner);
        self.touched.push(key);
        self.touched.push(owner);
    }

    /// Drops the entries of `h`'s subtree (removed from the document, or no longer rendered).
    fn drop_subtree(&mut self, h: &Handle, include_self: bool) {
        if include_self && let Some(e) = self.table.remove(&node_id(h)) {
            self.gen_dirty |= e.gsig != 0;
        }
        for c in h.children.borrow().iter() {
            if is_el(c) {
                self.drop_subtree(c, true);
            }
        }
    }

    /// QINC1f-fix: drops the entry `k` and, through the stored child lists, its descendants' entries, for a removed
    /// element whose node is already gone (no handle to walk). Left behind, those entries kept child lists naming
    /// freed TEXT nodes, whose addresses a new text node can reuse: `text_parent` then found the stale parent and a
    /// later edit of the new text was lost.
    fn drop_key(&mut self, k: usize) {
        let mut stack = vec![k];
        while let Some(k) = stack.pop() {
            if let Some(e) = self.table.remove(&k) {
                self.gen_dirty |= e.gsig != 0;
                stack.extend(e.children.iter().copied());
            }
        }
    }

    // ---------- sources ----------

    /// The parent element of the text node `text` (a reverse scan of the stored child lists). A text node's address
    /// can be reused after it is freed, so a stale list may name it too: only a live entry whose node still holds
    /// `text` counts (QINC1f-fix).
    fn text_parent(&self, text: usize, index: &mut Option<HashMap<usize, Vec<usize>>>) -> Option<usize> {
        let ix = index.get_or_insert_with(|| {
            let mut m: HashMap<usize, Vec<usize>> = HashMap::new();
            for (k, e) in &self.table {
                for c in e.children.iter() {
                    m.entry(*c).or_default().push(*k);
                }
            }
            m
        });
        ix.get(&text)?.iter().copied().find(|k| self.handle_of(*k).is_some_and(|h| h.children.borrow().iter().any(|c| node_id(c) == text)))
    }

    fn removal(&mut self, map: &InvalidationMap, parent: &Handle, child: usize, prev: Option<usize>, next: Option<usize>) {
        let pk = node_id(parent);
        // QINC1k3: the walk may see under a `display: none` element (it reads matched rules only).
        self.gen_dirty |= self.table.get(&pk).is_some_and(Entry::none);
        self.kids_dirty.insert(pk);
        self.box_mark(pk);
        let kids = parent.children.borrow();
        let pos = |k: usize| kids.iter().position(|c| node_id(c) == k);
        let index = next.and_then(pos).or_else(|| prev.and_then(pos).map(|i| i + 1)).unwrap_or(kids.len());
        drop(kids);
        if !self.table.contains_key(&child) {
            // a text node (or an element of an unrendered subtree): only `:empty` sees it, and `:has()` (QAPI6)
            let m = map.for_text(parent);
            self.apply(m);
            let m = map.for_has_removal(parent, index);
            self.apply(m);
            return;
        }
        let node = self.handle_of(child);
        let m = map.for_children(parent, &ChildChange::Removed { node: node.clone().unwrap_or_else(|| parent.clone()), index });
        self.apply(m);
        match node {
            Some(n) => self.drop_subtree(&n, true),
            None => self.drop_key(child),
        }
    }

    fn insertion(&mut self, map: &InvalidationMap, parent: &Handle, child: &Handle) {
        let pk = node_id(parent);
        // QINC1k3: the walk may see under a `display: none` element (it reads matched rules only).
        self.gen_dirty |= self.table.get(&pk).is_some_and(Entry::none);
        self.kids_dirty.insert(pk);
        self.box_mark(pk);
        let m = if is_el(child) { map.for_children(parent, &ChildChange::Inserted(child.clone())) } else { map.for_text(parent) };
        self.apply(m);
        // QINC1k3: an element the walk reads that moved here (new elements are seen when visited).
        if !self.gen_dirty && is_el(child) {
            let mut stack = vec![child.clone()];
            while let Some(n) = stack.pop() {
                if self.table.get(&node_id(&n)).is_some_and(|e| e.gsig != 0) {
                    self.gen_dirty = true;
                    break;
                }
                stack.extend(n.children.borrow().iter().filter(|c| is_el(c)).cloned());
            }
        }
    }

    fn attr_change(&mut self, map: &InvalidationMap, h: &Handle, name: &str, old: Option<&str>, new: Option<&str>) {
        // QINC1k3: `content: attr(..)` reads it.
        self.gen_dirty |= self.table.get(&node_id(h)).is_some_and(|e| e.gsig & GEN_ATTR != 0);
        let m = map.for_attr(h, name, old, new);
        self.apply(m);
        self.box_mark(node_id(h));
        self.fp_dirty.insert(node_id(h));
    }

    /// The element children of `pk` at the last restyle (its stored child keys that have an entry: text nodes never
    /// do). Read before the log's removals drop entries.
    fn old_el_children(&self, pk: usize) -> Option<Vec<usize>> {
        let e = self.table.get(&pk)?;
        Some(e.children.iter().copied().filter(|k| self.table.contains_key(k)).collect())
    }

    /// QINC1f-fix: the structural / sibling marks of every parent whose child list changed, from its old and new
    /// element lists (the per-entry `for_children` calls replay each change against the FINAL DOM, which loses the
    /// old edges after several inserts / moves in one batch).
    fn child_lists(&mut self, map: &InvalidationMap, old: HashMap<usize, Vec<usize>>) {
        for (pk, old) in old {
            let Some(p) = self.handle_of(pk) else { continue };
            let m = map.for_child_list(&p, &old);
            self.apply(m);
        }
    }

    /// QAPI6: a logged change of a node without an entry (under `display: none`, or a text node whose parent has
    /// none). Nothing it styles is rendered, but with a `:has()` rule a rendered anchor above it may now match
    /// differently: the marks of the change, from the node's handle the log kept.
    fn unentried(&mut self, map: &InvalidationMap, e: &crate::incr_log::Entry) {
        if !map.has_has() {
            return;
        }
        let h = match crate::incr_log::taken_node(e.node) {
            Some(Some(h)) => h,
            // freed: out of the document, its removal was logged on a live parent
            Some(None) => return,
            // not kept (the log ran before this map existed)
            None => {
                self.full = true;
                return;
            }
        };
        // QAPI6b: nothing below the topmost entry-less ancestor `top` is styled, so only the `:has` anchors above
        // it can change: one feature-agnostic answer per `top` and update (`for_has_any`), however many changes
        // the log holds there.
        let mut top = h;
        loop {
            let Some(p) = parent_handle(&top) else {
                // detached: out of the document, its removal was logged on a live parent
                return;
            };
            let pk = node_id(&p);
            if pk == self.doc_key || self.table.contains_key(&pk) {
                break;
            }
            top = p;
        }
        // A `top` inserted under its styled parent in this batch did not exist before it: that insertion's own
        // `HasTree` marks (from the final subtree) already cover every change inside it.
        if self.inserted_now.contains(&node_id(&top)) || !self.has_any_done.insert(node_id(&top)) {
            return;
        }
        let m = map.for_has_any(&top);
        self.apply(m);
    }

    fn process_log(&mut self, map: &InvalidationMap, entries: &[crate::incr_log::Entry]) {
        let mut text_index = None;
        let mut old_lists: HashMap<usize, Vec<usize>> = HashMap::new();
        for e in entries {
            if let Change::ChildInsert { child, .. } = e.change
                && self.table.contains_key(&e.node)
            {
                self.inserted_now.insert(child);
            }
            if matches!(e.change, Change::ChildInsert { .. } | Change::ChildRemove { .. } | Change::TextContent | Change::InnerHtml)
                && !old_lists.contains_key(&e.node)
                && let Some(old) = self.old_el_children(e.node)
            {
                old_lists.insert(e.node, old);
            }
        }
        for e in entries {
            match &e.change {
                Change::Attr { name, old, new } => {
                    let Some(h) = self.handle_of(e.node) else {
                        // QINC1k3: a node without an entry (under a `display: none` element) may still be walked.
                        self.gen_dirty = true;
                        self.unentried(map, e);
                        continue;
                    };
                    self.covered_attr.insert((e.node, name.clone()));
                    self.attr_change(map, &h, name, val_str(old), val_str(new));
                }
                Change::ChildInsert { child, .. } => {
                    let Some(p) = self.handle_of(e.node) else {
                        // QINC1k3: see above.
                        self.gen_dirty = true;
                        self.unentried(map, e);
                        continue;
                    };
                    self.covered_kids.insert(e.node);
                    let c = p.children.borrow().iter().find(|c| node_id(c) == *child).cloned();
                    if let Some(c) = c {
                        self.insertion(map, &p, &c);
                    }
                }
                Change::ChildRemove { child, prev, next } => {
                    let Some(p) = self.handle_of(e.node) else {
                        // QINC1k3: see above.
                        self.gen_dirty = true;
                        self.unentried(map, e);
                        continue;
                    };
                    self.covered_kids.insert(e.node);
                    self.removal(map, &p, *child, *prev, *next);
                }
                Change::Text => {
                    let Some(p) = self.text_parent(e.node, &mut text_index).and_then(|pk| self.handle_of(pk)) else {
                        self.unentried(map, e);
                        continue;
                    };
                    let pk = node_id(&p);
                    self.covered_kids.insert(pk);
                    self.kids_dirty.insert(pk);
                    self.box_mark(pk);
                    let m = map.for_text(&p);
                    self.apply(m);
                }
                Change::TextContent | Change::InnerHtml => {
                    let Some(p) = self.handle_of(e.node) else {
                        // QINC1k3: see above.
                        self.gen_dirty = true;
                        self.unentried(map, e);
                        continue;
                    };
                    self.covered_kids.insert(e.node);
                    self.kids_dirty.insert(e.node);
                    self.gen_dirty |= self.table.get(&e.node).is_some_and(Entry::none);
                    self.box_mark(e.node);
                    let now: Vec<Handle> = p.children.borrow().clone();
                    let now_keys: HashSet<usize> = now.iter().map(node_id).collect();
                    let old: Vec<usize> = self.table.get(&e.node).map(|x| x.children.to_vec()).unwrap_or_default();
                    for k in old.into_iter().filter(|k| !now_keys.contains(k)) {
                        match self.handle_of(k) {
                            Some(n) => self.drop_subtree(&n, true),
                            None => self.drop_key(k),
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
                    let Some(h) = self.handle_of(e.node) else {
                        // QINC1k3: `:checked` matches of an element without an entry (see above).
                        self.gen_dirty = true;
                        self.unentried(map, e);
                        continue;
                    };
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
                // QINC1j: the engine diffs App-side inputs itself (interaction, vh, images, clock).
                Change::Ambient(_) => {}
            }
        }
        self.child_lists(map, old_lists);
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
                        let old_els = self.old_el_children(k).unwrap_or_default();
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
                        let m = map.for_child_list(&c, &old_els);
                        self.apply(m);
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
        let mut lost = false;
        self.table.retain(|_, e| {
            let keep = e.seen == ep;
            lost |= !keep && e.gsig != 0;
            keep
        });
        self.gen_dirty |= lost;
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
                // QINC1k3: a new `<script>` / `<style>` (`GEN_OPAQUE`) may change what the walk reads.
                self.gen_dirty |= fresh;
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
            let (sb, mut reads_vh) = self.style_of(cx, h, k, 0, inh);
            let s = *sb;
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
                        // QINC1k4: a pseudo slot's read of the viewport height (`vh`, or a `%` calc that
                        // captures the viewport pair) re-cascades its element on a height change too
                        let (ps, ps_vh) = self.style_of(cx, h, k, slot, &own);
                        reads_vh |= ps_vh;
                        animated |= crate::anim_eval::is_animated(&ps, cx.vw);
                        pseudo[slot as usize - 1] = Some(ps);
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
                        self.box_mark_owner(k, powner);
                    }
                    StyleDiff::Layout if !owns_box(tag, &s) => self.box_mark(k),
                    _ => {}
                }
                if s.display == Display::None && old.display != Display::None {
                    self.drop_subtree(h, false);
                }
            } else {
                self.box_mark_owner(k, powner);
            }
            // QINC1g: any change of what the builder reads for this element (its style, a pseudo
            // slot's style or presence, a match list: `content`, `background-image`) touches its box.
            let gsig = {
                let e = &self.table[&k];
                let gsig = if e.m == ms && !need_match {
                    e.gsig
                } else if s.display == Display::None {
                    // The walk matches the `::before` / `::after` slots the cascade skipped (it reads
                    // matched rules only, so it may walk on under an element hidden by inline style).
                    let mut all = ms.clone();
                    for slot in 1..=2u8 {
                        all[slot as usize] = Some(match_slot(h, rules, cx.vw, slot));
                    }
                    gen_sig(tag, rules, &all)
                } else {
                    gen_sig(tag, rules, &ms)
                };
                if gsig != e.gsig {
                    self.gen_dirty = true;
                }
                let changed = old_style.is_none_or(|o| !same_style(&o, &s))
                    || e.pseudo.iter().zip(pseudo.iter()).any(|(a, b)| match (a, b) {
                        (None, None) => false,
                        (Some(a), Some(b)) => !same_style(a, b),
                        _ => true,
                    })
                    || e.m != ms;
                if changed {
                    self.touched.push(k);
                }
                gsig
            };
            let e = self.table.get_mut(&k).expect("inserted above");
            e.why = why;
            e.gsig = gsig;
            e.m = ms;
            e.style = Some(sb);
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
            self.box_mark_owner(k, owner);
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

    /// The style of `(h, memo slot)` inheriting `inh` (its match is seeded) and whether resolving it
    /// read the viewport height. QINC1i4: a full update right after a from-scratch build ADOPTS what the
    /// build resolved (same DOM, rules, state and inherited input; check mode cascades anyway and
    /// compares).
    fn style_of(&mut self, cx: &Cx, h: &Handle, k: usize, slot: u8, inh: &InheritedStyle) -> (Box<ComputedStyle>, bool) {
        let adopt = cx.adopt.filter(|_| self.full);
        if !cx.verify
            && let Some(r) = adopt.and_then(|a| a.borrow_mut().remove(&(k, slot)))
        {
            self.stats.adopted += 1;
            return (r.style, r.vh);
        }
        let vh_before = crate::dom::VIEWPORT_H_READ.with(|v| v.replace(false));
        let s = resolve_style_for(h, cx.rules, *inh, cx.vw, slot_pseudo(slot));
        let reads_vh = crate::dom::VIEWPORT_H_READ.with(|v| v.replace(vh_before));
        let rec = adopt.map(RefCell::borrow);
        match rec.as_ref().and_then(|a| a.get(&(k, slot))) {
            None => self.stats.resolved += 1,
            Some(r) => {
                self.stats.adopted += 1;
                if !same_style(&r.style, &s) || r.vh != reads_vh {
                    let what = style_changes(&r.style, &s).into_iter().next().map_or_else(
                        || format!("reads vh: build {} vs engine {reads_vh}", r.vh),
                        |c| format!("field {}: build {} vs engine {}", c.field, c.old, c.new),
                    );
                    self.adopt_mism.push(format!("{}: adopted {} style {what}", path_of(h), slot_pseudo(slot).unwrap_or("element")));
                }
            }
        }
        (Box::new(s), reads_vh)
    }

    /// One incremental update after a from-scratch build of `doc`. `adopt`: what that build resolved
    /// (a full update takes it instead of cascading again; `None` for an App frame's live update).
    fn update(&mut self, doc: &Handle, rules: &Rc<Vec<StyleRule>>, vw: f32, vh: f32, adopt: Option<&RefCell<Record>>) {
        self.stats = Stats::default();
        self.updates += 1;
        self.touched.clear();
        self.kids_changed.clear();
        self.has_any_done.clear();
        self.inserted_now.clear();
        let log = crate::incr_log::take();
        // A list that is not the cache's current one has no generation: always a full re-match.
        let rkey = (crate::rules_cache::generation_of(rules).unwrap_or(u64::MAX), Rc::as_ptr(rules) as usize);
        self.full = self.table.is_empty()
            || log.global
            || rkey != self.rules_key
            || rkey.0 == u64::MAX
            || vw.to_bits() != self.vw.to_bits()
            // QCSS1: a media query on the viewport height (`(height >= 600px)`, orientation, aspect-ratio)
            // changes WHICH rules match; matches are cached per element, so such a list re-matches in full.
            || (vh.to_bits() != self.vh.to_bits() && rules.iter().any(|r| r.media.iter().any(|m| m.reads_height())))
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
            // QINC1k3: a DOM write dirties the counter / quote walk only through what it reads
            // (`gen_sig`: an entry's `gen` changed, a non-zero `gsig` inserted / moved / removed, an
            // attribute of a `GEN_ATTR` element).
            // QAPI6: one DOM state for the whole batch: the map skips walks it already did
            map.begin_batch();
            self.process_log(map, &log.entries);
            self.interaction(map);
            if reconcile_on() && !self.full {
                self.reconcile(map, doc);
                if self.stats.unlogged > 0 {
                    self.gen_dirty = true;
                }
            }
            map.end_batch();
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
        let cx = Cx { rules, vw, first_line, first_letter, adopt, verify: adopt_verify() };
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
        if self.full || self.gen_dirty || self.gen_out.is_none() {
            self.stats.gen_walked = true;
            self.generated(doc, rules, vw);
        }
        self.gen_dirty = false;
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
        self.kids_changed = self.kids_dirty.iter().copied().collect();
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
        let mut lost = false;
        self.table.retain(|_, e| {
            let keep = e.handle().is_some_and(|mut n| loop {
                match parent_handle(&n) {
                    Some(p) if node_id(&p) == doc => break true,
                    Some(p) => n = p,
                    None => break false,
                }
            });
            lost |= !keep && e.gsig != 0;
            keep
        });
        // QINC1k3: the next update walks again (an entry the walk read left without a log entry).
        self.gen_dirty |= lost;
    }

    /// Membership-based ageing of animation start times (R60 6.5): keeps the start time of every
    /// animation of an element the table lists as animated, whether or not it was restyled.
    pub(crate) fn age_animations(&self) {
        let animated: HashSet<usize> = self.table.iter().filter(|(_, e)| e.animated).map(|(k, _)| *k).collect();
        crate::anim_eval::retain_starts(|node, _| animated.contains(&node));
        // QANIMHLb: transitions (and the base style they compare with) of every element still listed.
        crate::transition::retain(|node| self.table.contains_key(&node));
    }

    /// Compares every style the from-scratch build resolved with the table: the mismatch reports.
    fn compare(&self, fresh: &Record) -> Vec<String> {
        let mut out = Vec::new();
        for (&(k, slot), rec) in fresh {
            let fs = &*rec.style;
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

/// Equal styles minus identity (`node`, `tree_order`): cheap, and conservative (a NaN field is a change).
fn same_style(a: &ComputedStyle, b: &ComputedStyle) -> bool {
    let (mut x, mut y) = (*a, *b);
    x.node = 0;
    y.node = 0;
    x.tree_order = 0;
    y.tree_order = 0;
    x == y
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
    /// QINC1i4: the from-scratch build's styles (a full update adopts them).
    adopt: Option<&'a RefCell<Record>>,
    /// Check mode: cascade an adopted style anyway and compare.
    verify: bool,
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
    /// `live`: an App frame's update (QINC1j): animation evaluation creates and reports start times.
    /// `memo`: the match memo to start from (the from-scratch build's own, for the shadow update that
    /// follows it on the same DOM and state); `None` = empty.
    fn enter_as(rules: &Rc<Vec<StyleRule>>, live: bool, memo: Option<crate::style::MatchMemo>) -> Env {
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
        crate::style::MATCH_MEMO.with(|m| *m.borrow_mut() = Some(memo.unwrap_or_default()));
        crate::style::VAR_EXPAND_MEMO.with(|m| *m.borrow_mut() = Some(Default::default()));
        crate::style::ANCESTOR_BLOOM.with(|b| *b.borrow_mut() = Some(Default::default()));
        crate::dom::CLASS_CACHE.with(|c| *c.borrow_mut() = Some(Default::default()));
        crate::style::SEL_CACHES.with(|c| *c.borrow_mut() = Some(Default::default()));
        Env {
            flags,
            perf: crate::perf::suspend_forced(),
            anim: crate::anim_eval::set_readonly(!live),
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
        crate::style::SEL_CACHES.with(|c| *c.borrow_mut() = None);
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
pub(crate) fn after_build(
    doc: &Handle,
    rules: &Rc<Vec<StyleRule>>,
    vw: f32,
    viewport_height: Option<f32>,
    image_cache: &HashMap<String, std::sync::Arc<DecodedImage>>,
    tree: &LayoutBox,
    build_memo: Option<crate::style::MatchMemo>,
) {
    let vh = viewport_height.unwrap_or(1080.0);
    let fresh = take_record();
    let styled = fresh.as_ref().map_or(0, |f| f.keys().filter(|k| k.1 == 0).count() as u64);
    let fresh = fresh.map(RefCell::new);
    let check = checking();
    let t0 = std::time::Instant::now();
    let t_phase = crate::perf::phase(crate::perf::Phase::IncrShadow);
    let env = Env::enter_as(rules, false, build_memo);
    let report = ENGINE.with(|e| {
        let mut slot = e.borrow_mut();
        if slot.as_ref().is_none_or(|e| e.doc_key != node_id(doc) || e.doc.upgrade().is_none()) {
            *slot = Some(Engine::new(doc));
        }
        let eng = slot.as_mut().expect("just set");
        eng.update(doc, rules, vw, vh, fresh.as_ref());
        eng.stats.styled_fresh = styled;
        let mut mism = std::mem::take(&mut eng.adopt_mism);
        if check && let Some(f) = fresh.as_ref() {
            mism.extend(eng.compare(&f.borrow()));
        }
        if check && fresh.is_some() {
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
    let style_ns = t0.elapsed().as_nanos() as u64;
    // QINC1g: the box stage, in the same environment (stored styles and matches, rule index).
    let box_mism = crate::incr_box::update(doc, rules, vw, viewport_height, image_cache, tree, report.1.is_empty());
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
    incr_add(Incr::RestyleAdopted, stats.adopted);
    incr_add(Incr::RestyleResolved, stats.resolved);
    crate::perf::incr_restyle_sample(stats.restyled(), stats.styled_fresh, style_ns);
    if !mism.is_empty() {
        // The engine is no longer trusted: the next update starts from scratch.
        ENGINE.with(|e| *e.borrow_mut() = None);
        report_mismatches("incremental restyle differs from the from-scratch build", mism);
    }
    if !box_mism.is_empty() {
        report_mismatches("incremental box tree differs from the from-scratch build", box_mism);
    }
}

// ---------- QINC1j: App frames (live) ----------

/// QINC1j: the element table can be updated for `doc` without a full restyle (same rule list,
/// viewport width and custom-property arena; the log is not Global; the invalidation map exists).
pub(crate) fn live_ready(doc: &Handle, rules: &Rc<Vec<StyleRule>>, vw: f32) -> bool {
    let rkey = (crate::rules_cache::generation_of(rules).unwrap_or(u64::MAX), Rc::as_ptr(rules) as usize);
    let ok = ENGINE.with(|e| {
        e.try_borrow().ok().is_some_and(|e| {
            e.as_ref().is_some_and(|e| {
                e.doc_key == node_id(doc)
                    && e.doc.upgrade().is_some()
                    && !e.table.is_empty()
                    && rkey.0 != u64::MAX
                    && e.rules_key == rkey
                    && e.vw.to_bits() == vw.to_bits()
                    && e.arena == ARENA_EPOCH.with(Cell::get)
            })
        })
    });
    ok && !crate::incr_log::pending().1 && crate::invalidation::current().is_some_and(|m| !m.is_global())
}

/// QINC1j: nothing the restyle reads changed since its last update (no DOM / CSS write in the log,
/// same interaction state and viewport height, no running animation). Call after `live_ready`.
pub(crate) fn is_current(vh: f32) -> bool {
    if crate::incr_log::has_writes() {
        return false;
    }
    ENGINE.with(|e| {
        e.try_borrow().ok().is_some_and(|e| {
            e.as_ref().is_some_and(|e| {
                e.vh.to_bits() == vh.to_bits()
                    && INTERACTION.with(|i| *i.borrow() == e.inter)
                    && !e.table.values().any(|x| x.animated)
            })
        })
    })
}

/// QINC1j: what an App frame's restyle did.
pub(crate) struct LiveStyle {
    /// Restyled / styled elements (the worker takes the relayout above `incr_frame::WORKER_SHARE`).
    pub(crate) share: f32,
    pub(crate) restyled: u64,
}

/// QINC1j: an App frame's restyle + box update, results USED (R60 section 9). `Err`: the engine
/// restyled everything (`global_restyle`, by design) or the box tree cannot be updated (QINC1n: a
/// fallback reason); the caller builds from scratch.
/// Animation start times are created and reported as a build's pass does, and aged by membership.
pub(crate) fn live_update(
    doc: &Handle,
    rules: &Rc<Vec<StyleRule>>,
    vw: f32,
    viewport_height: Option<f32>,
    image_cache: &HashMap<String, std::sync::Arc<DecodedImage>>,
) -> Result<LiveStyle, &'static str> {
    let vh = viewport_height.unwrap_or(1080.0);
    let t0 = std::time::Instant::now();
    let env = Env::enter_as(rules, true, None);
    let stats = ENGINE.with(|e| {
        let mut slot = e.borrow_mut();
        let eng = slot.as_mut()?;
        eng.update(doc, rules, vw, vh, None);
        eng.age_animations();
        Some((eng.stats.clone(), eng.table.len() as u64))
    });
    crate::anim_eval::finish_report_only();
    let style_ns = t0.elapsed().as_nanos() as u64;
    let full = stats.as_ref().is_none_or(|s| s.0.full);
    let boxes_ok = !full && crate::incr_box::live_update(doc, rules, vw, viewport_height, image_cache);
    if !boxes_ok {
        // The kept box tree missed this update's marks: it starts over from the next build.
        crate::incr_box::drop_engine();
    }
    env.exit(rules);
    let Some((stats, styled)) = stats else { return Err("no_style_engine") };
    use crate::perf::{incr_add, Incr};
    incr_add(Incr::Updates, 1);
    incr_add(Incr::RestyledMatched, stats.rematched);
    incr_add(Incr::RestyledCascaded, stats.recascaded);
    incr_add(Incr::GlobalUpdates, u64::from(stats.full));
    incr_add(Incr::RestyleResolved, stats.resolved);
    crate::perf::incr_restyle_sample(stats.restyled(), styled, style_ns);
    if !boxes_ok {
        return Err(if full { "global_restyle" } else { "box_update_failed" });
    }
    Ok(LiveStyle { share: if styled == 0 { 1.0 } else { stats.restyled() as f32 / styled as f32 }, restyled: stats.restyled() })
}

/// Check mode: counts `incr.check_mismatch`; tests panic (or collect, `Seam::panic` off), release
/// prints one `scratchy: incr-check: MISMATCH` diag line per difference.
pub(crate) fn report_mismatches(what: &str, mism: Vec<String>) {
    crate::perf::incr_add(crate::perf::Incr::CheckMismatch, mism.len() as u64);
    #[cfg(test)]
    {
        if SEAM.with(|s| s.get().panic) {
            panic!("{what}:\n{}", mism.join("\n"));
        }
        MISMATCHES.with(|m| m.borrow_mut().extend(mism));
    }
    #[cfg(not(test))]
    {
        let _ = what;
        for m in mism {
            diag!("scratchy: incr-check: MISMATCH {m}");
        }
    }
}

/// Whether the shadow stages compare with the from-scratch build (tests, `SCRATCHY_INCR_CHECK=1`).
pub(crate) fn checking() -> bool {
    mode() == Mode::Check
}

// ---------- QINC1g: what the box stage reads from the element table ----------
// (accessors only, so the table's storage can change without touching the box stage)

/// What the last update means for the box tree.
pub(crate) struct BoxMarks {
    /// The update restyled everything without describing what changed (the box tree starts over).
    pub(crate) full: bool,
    pub(crate) touched: Vec<usize>,
    pub(crate) kids_changed: HashSet<usize>,
}

pub(crate) fn take_box_marks() -> Option<BoxMarks> {
    ENGINE.with(|e| {
        let mut e = e.try_borrow_mut().ok()?;
        let e = e.as_mut()?;
        Some(BoxMarks { full: e.stats.full, touched: std::mem::take(&mut e.touched), kids_changed: std::mem::take(&mut e.kids_changed) })
    })
}

fn with_entry<R>(h: &Handle, f: impl FnOnce(&Entry) -> Option<R>) -> Option<R> {
    ENGINE.with(|e| {
        let e = e.try_borrow().ok()?;
        let x = e.as_ref()?.table.get(&node_id(h))?;
        if x.node.as_ptr() != Rc::as_ptr(h) {
            return None;
        }
        f(x)
    })
}

/// The stored style of `(h, memo slot)`.
pub(crate) fn stored_style(h: &Handle, slot: u8) -> Option<ComputedStyle> {
    with_entry(h, |e| if slot == 0 { e.style.as_deref().copied() } else { e.pseudo.get(slot as usize - 1)?.as_deref().copied() })
}

/// The stored match of `(h, memo slot)`.
pub(crate) fn stored_match(h: &Handle, slot: u8) -> Option<MatchList> {
    with_entry(h, |e| e.m.get(slot as usize)?.clone())
}

/// The live node of a table key.
pub(crate) fn handle_of_key(k: usize) -> Option<Handle> {
    ENGINE.with(|e| e.try_borrow().ok()?.as_ref()?.handle_of(k))
}

pub(crate) fn has_entry(k: usize) -> bool {
    ENGINE.with(|e| e.try_borrow().ok().and_then(|e| e.as_ref().map(|e| e.table.contains_key(&k))).unwrap_or(false))
}

/// The `rem` base of the last update.
pub(crate) fn root_font_size() -> Option<f32> {
    ENGINE.with(|e| e.try_borrow().ok()?.as_ref().map(|e| e.rfs).filter(|f| !f.is_nan()))
}

/// R60 14.4 per element: the elements the build skips as `display: none` roots (the table keeps an
/// entry for each, none for their descendants).
#[allow(dead_code)] // the forced-layout path reads it from QINC1i on (`HiddenSnap`); tests now
pub(crate) fn display_none_roots() -> Vec<Handle> {
    ENGINE.with(|e| {
        let Ok(e) = e.try_borrow() else { return Vec::new() };
        let Some(e) = e.as_ref() else { return Vec::new() };
        e.table.values().filter(|x| x.styled && x.style.as_ref().is_some_and(|s| s.display == Display::None)).filter_map(Entry::handle).collect()
    })
}

/// Runs `f` with the engine's generated content (counters / quotes / dynamic `content`, R60 6.4) in
/// the builder's `GENERATED_TEXT` / `ELEMENT_CONTENT` instead of the from-scratch build's.
pub(crate) fn with_engine_generated<R>(f: impl FnOnce() -> R) -> R {
    let mine = ENGINE.with(|e| e.borrow_mut().as_mut().and_then(|e| e.gen_out.take()));
    let had = mine.is_some();
    let g = mine.unwrap_or_default();
    let saved_text = crate::images::GENERATED_TEXT.with(|t| t.replace(g.text));
    let saved_el = crate::images::ELEMENT_CONTENT.with(|t| t.replace(g.element));
    let r = f();
    let text = crate::images::GENERATED_TEXT.with(|t| t.replace(saved_text));
    let element = crate::images::ELEMENT_CONTENT.with(|t| t.replace(saved_el));
    if had {
        ENGINE.with(|e| {
            if let Some(e) = e.borrow_mut().as_mut() {
                e.gen_out = Some(GeneratedOut { text, element });
            }
        });
    }
    r
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
