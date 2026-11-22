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
//! QINC1i: with `SCRATCHY_INCR=1` (or `_CHECK=1`) a read updates the incremental engine instead
//! (`incr_engine.rs`, `refresh_engine`): no from-scratch build, rects from the engine's kept laid
//! tree, glyphs on the first text read, the window's decoded images, and the QYTC1a / QYTC1d /
//! HiddenSnap / ATTR_SNAP skips and the App-layout IntersectionObserver source bypassed.
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
    added
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
    /// QY53l2: the `display: none` subtrees of the last forced layout on this thread.
    static HIDDEN: RefCell<Option<HiddenSnap>> = const { RefCell::new(None) };
    /// QY53o1: the attribute readers of the last forced layout's rule list, with the
    /// geometry generation and CSS version it ran at.
    static ATTR_SNAP: RefCell<Option<AttrSnap>> = const { RefCell::new(None) };
}

/// QY53o1: `((geometry generation, CSS version), readers)` of the last forced layout.
type AttrSnap = ((u64, (u64, u64)), std::rc::Rc<crate::style::AttrReaders>);

/// QY53o1: attributes no selector, `attr()` or box build reads (tooltips, scripting hooks).
fn attr_never_read_by_layout(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "title" || n == "role" || n == "draggable" || n == "spellcheck" || n == "tabindex" || n.starts_with("data-") || n.starts_with("aria-")
}

/// QY53o1: may a write of attribute `name` on `x` skip the geometry bump? Only while the last
/// forced layout is current (no geometry bump or CSS change since) and its rule list has no
/// selector or `attr()` that reads `name` on an element of this tag.
pub(crate) fn attr_write_unread(x: &Handle, name: &str) -> bool {
    // QINC1i: the engine's invalidation sets decide what a write restyles (R60 1: ATTR_SNAP bypassed).
    if !matches!(x.data, NodeData::Element { .. }) || !attr_never_read_by_layout(name) || crate::incr_engine::forced_on() {
        return false;
    }
    ATTR_SNAP.with(|s| {
        let s = s.borrow();
        let Some(((g, css), readers)) = s.as_ref() else {
            return false;
        };
        *g == crate::js_dom::geom_gen() && *css == crate::js_style::css_ver() && !readers.reads(name, element_tag(x))
    })
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
    // QINC1i: a write inside a `display: none` subtree invalidates nothing rendered in the engine (R60 1:
    // HiddenSnap bypassed).
    if crate::incr_engine::forced_on() {
        return None;
    }
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

// ---------- QYTC1d: `class` writes that change no element's matched rules ----------

/// QYTC1d: where a class sits in a selector, as seen from the element that carries it: the
/// subject compound only touches that element; a compound left of a descendant/child combinator
/// reaches its subtree, left of a sibling combinator its later siblings' subtrees.
const CF_DESC: u8 = 1;
const CF_SIB: u8 = 2;
/// Inside `:has()` / `:nth-child(of)`, or nested where the reach is not tracked: give up.
const CF_UNBOUNDED: u8 = 4;

/// QYTC1d: `ClassDeps::by_class` key of the rules whose selectors read `class` as an attribute
/// (`[class*=x]`): they can change with any `class` write. (No class token is empty.)
const CLASS_ATTR_KEY: &str = "";

/// QYTC1d: per rule list (by its `rules_cache` generation; never the list's `Rc`, see `ImpGeo`),
/// every class a selector names -> `(index into sels, CF_* flags)`.
struct ClassDeps {
    generation: u64,
    /// The selectors of every rule some class (or `[class]`) entry points at.
    sels: Vec<SelectorList<OurSelectorImpl>>,
    by_class: HashMap<String, Vec<(u32, u8)>>,
    /// Some `content` reads `attr(class)` (or an `attr()` behind `var()`): no verdict.
    content_reads: bool,
}

/// QYTC1d: does this block's `content` read `attr(class)`? (`AttrReaders::scan_decls`' rules.)
fn content_reads_class(d: &Declarations) -> bool {
    let here = match &d.content {
        Some(ContentDecl::Parts(parts)) => parts.iter().any(|p| matches!(p, ContentPart::Attr(n) if n.eq_ignore_ascii_case("class"))),
        _ => d.var_decls.iter().any(|(n, v)| n.eq_ignore_ascii_case("content") && v.to_ascii_lowercase().contains("attr(")),
    };
    here || d.late.iter().any(content_reads_class)
}

thread_local! {
    static CLASS_DEPS: RefCell<Option<Rc<ClassDeps>>> = const { RefCell::new(None) };
}

fn note_classes(c: &Component<OurSelectorImpl>, follow: u8, rule: u32, out: &mut HashMap<String, Vec<(u32, u8)>>) {
    let mut add = |name: &str, f: u8| {
        let v = out.entry(name.to_string()).or_default();
        match v.last_mut() {
            Some(e) if e.0 == rule => e.1 |= f,
            _ => v.push((rule, f)),
        }
    };
    match c {
        Component::Class(a) => add(&a.0, follow),
        Component::AttributeInNoNamespaceExists { local_name_lower, .. } if local_name_lower.0.eq_ignore_ascii_case("class") => add(CLASS_ATTR_KEY, follow),
        Component::AttributeInNoNamespace { local_name, .. } if local_name.0.eq_ignore_ascii_case("class") => add(CLASS_ATTR_KEY, follow),
        Component::AttributeOther(a) if a.local_name_lower.0.eq_ignore_ascii_case("class") => add(CLASS_ATTR_KEY, follow),
        Component::Negation(l) | Component::Is(l) | Component::Where(l) => {
            for s in l.slice() {
                // A nested selector with its own combinators reaches other elements: count both ways.
                let comb = s.iter_raw_match_order().any(|c| matches!(c, Component::Combinator(_)));
                let f = if comb { follow | CF_DESC | CF_SIB } else { follow };
                for n in s.iter_raw_match_order() {
                    note_classes(n, f, rule, out);
                }
            }
        }
        Component::Has(l) => {
            for h in l.iter() {
                for n in h.selector.iter_raw_match_order() {
                    note_classes(n, CF_UNBOUNDED, rule, out);
                }
            }
        }
        Component::NthOf(d) => {
            for s in d.selectors() {
                for n in s.iter_raw_match_order() {
                    note_classes(n, CF_UNBOUNDED, rule, out);
                }
            }
        }
        // Not parsed today (like `:has()`/`:nth-child(of)`); unbounded should that change.
        Component::Host(Some(s)) | Component::Slotted(s) => {
            for n in s.iter_raw_match_order() {
                note_classes(n, CF_UNBOUNDED, rule, out);
            }
        }
        _ => {}
    }
}

fn class_deps_build(rules: &[StyleRule], generation: u64) -> ClassDeps {
    use selectors::parser::Combinator;
    let mut by_class: HashMap<String, Vec<(u32, u8)>> = HashMap::new();
    let mut content_reads = false;
    for (ri, rule) in rules.iter().enumerate() {
        if rule.may_declare_generated() && (content_reads_class(rule.decls()) || content_reads_class(rule.important())) {
            content_reads = true;
        }
        for sel in rule.selectors.slice() {
            let mut follow = 0u8;
            let mut it = sel.iter();
            loop {
                for c in it.by_ref() {
                    note_classes(c, follow, ri as u32, &mut by_class);
                }
                match it.next_sequence() {
                    None => break,
                    Some(Combinator::Descendant | Combinator::Child) => follow |= CF_DESC,
                    Some(Combinator::NextSibling | Combinator::LaterSibling) => follow |= CF_SIB,
                    Some(Combinator::PseudoElement) => {}
                    Some(_) => follow |= CF_UNBOUNDED,
                }
            }
        }
    }
    // Rule indices -> positions in `sels`.
    let mut pos: HashMap<u32, u32> = HashMap::new();
    let mut sels = Vec::new();
    for v in by_class.values_mut() {
        for e in v.iter_mut() {
            let ri = e.0;
            e.0 = *pos.entry(ri).or_insert_with(|| {
                sels.push(rules[ri as usize].selectors.clone());
                sels.len() as u32 - 1
            });
        }
    }
    ClassDeps { generation, sels, by_class, content_reads }
}

/// QYTC1d: `ClassDeps` of the rule list of the latest build on this thread; `None` when that
/// list is no longer `rules_cache`'s current one (as `current_important_geo`).
fn current_class_deps() -> Option<Rc<ClassDeps>> {
    let (rules, generation, _, _) = crate::rules_cache::current()?;
    if !crate::style::RULE_DERIVED.with(|d| d.borrow().as_ref().is_some_and(|x| Rc::ptr_eq(&x.rules, &rules))) {
        return None;
    }
    CLASS_DEPS.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(v) = c.as_ref()
            && v.generation == generation
        {
            return Some(v.clone());
        }
        let v = Rc::new(class_deps_build(&rules, generation));
        *c = Some(v.clone());
        Some(v)
    })
}

/// QYTC1d: what `class_write_pre` saw before a `class` write; `class_write_post` decides.
pub(crate) struct ClassProbe {
    deps: Rc<ClassDeps>,
    rules: Vec<u32>,
    els: Vec<Handle>,
    before: Vec<bool>,
}

/// QYTC1d: class probes allowed between two geometry reads (`GeomCache::refresh` refills it).
const CLASS_PROBES_PER_READ: u32 = 32;

thread_local! {
    static PROBE_BUDGET: Cell<u32> = const { Cell::new(CLASS_PROBES_PER_READ) };
}

/// QYTC1d: most selector matches one side of a probe may run (elements x selectors).
const CLASS_PROBE_MAX: usize = 20_000;

fn class_probe_bits(p: &ClassProbe) -> Vec<bool> {
    let mut caches = SelectorCaches::default();
    let mut out = Vec::with_capacity(p.before.len());
    for e in &p.els {
        let el = crate::dom::El(e.clone());
        for &ri in &p.rules {
            for sel in p.deps.sels[ri as usize].slice() {
                let mode = if sel.pseudo_element().is_some() { MatchingMode::ForStatelessPseudoElement } else { MatchingMode::Normal };
                let mut ctx = MatchingContext::new(mode, None, &mut caches, QuirksMode::NoQuirks, NeedsSelectorFlags::No, MatchingForInvalidation::No);
                out.push(matches_selector(sel, 0, None, &el, &mut ctx));
            }
        }
    }
    out
}

fn class_tokens(v: Option<&str>) -> std::collections::HashSet<&str> {
    v.map(|v| v.split_ascii_whitespace().collect()).unwrap_or_default()
}

/// QYTC1d: cheap gate checked before anything else per `class` write: the last forced layout is
/// current (no geometry bump or CSS change since) and the probe budget is not used up.
pub(crate) fn class_probe_possible() -> bool {
    // QINC1i: class writes go through the engine's invalidation sets (R60 1: QYTC1d bypassed).
    !crate::incr_engine::forced_on()
        && PROBE_BUDGET.with(Cell::get) > 0
        && ATTR_SNAP.with(|s| matches!(s.borrow().as_ref(), Some(((g, css), _)) if *g == crate::js_dom::geom_gen() && *css == crate::js_style::css_ver()))
}

/// QYTC1d, before `x`'s `class` goes from `old` to `new` (`None` = absent): `Some` when the last
/// forced layout is current (no geometry bump or CSS change since), no `content` reads
/// `attr(class)`, and the rules naming a class that changes (plus those reading `class` as an
/// attribute) are bounded (not inside `:has()`/`:nth-child(of)`); it records which of their
/// selectors match each element the change can reach.
pub(crate) fn class_write_pre(x: &Handle, old: Option<&str>, new: Option<&str>) -> Option<ClassProbe> {
    if !matches!(x.data, NodeData::Element { .. }) || !class_probe_possible() {
        return None;
    }
    // A probe only pays off when a geometry read follows; a script that churns classes without
    // reading gets a few probes, then one bump, and every later write finds the cache stale.
    PROBE_BUDGET.with(|b| b.set(b.get().saturating_sub(1)));
    let deps = current_class_deps()?;
    if deps.content_reads {
        return None;
    }
    let (a, b) = (class_tokens(old), class_tokens(new));
    let mut rules: Vec<u32> = Vec::new();
    let mut follow = 0u8;
    for c in a.symmetric_difference(&b).chain(std::iter::once(&CLASS_ATTR_KEY)) {
        for &(ri, f) in deps.by_class.get(*c).map(Vec::as_slice).unwrap_or(&[]) {
            follow |= f;
            rules.push(ri);
        }
    }
    if follow & CF_UNBOUNDED != 0 {
        return None;
    }
    rules.sort_unstable();
    rules.dedup();
    let mut els = Vec::new();
    if !rules.is_empty() {
        let n_sel: usize = rules.iter().map(|&r| deps.sels[r as usize].slice().len()).sum();
        // `x` itself; its subtree when a descendant combinator reaches there; its later siblings'
        // subtrees for a sibling one (`.a ~ .b .c`). Elements below the siblings of `x`'s
        // descendants are `x`'s descendants, which `CF_SIB` alone also takes in (`.a .b ~ .c`).
        els.push(x.clone());
        let mut stack = Vec::new();
        if follow & (CF_DESC | CF_SIB) != 0 {
            stack.extend(x.children.borrow().iter().cloned());
        }
        if follow & CF_SIB != 0
            && let Some(p) = parent_handle(x)
        {
            let kids = p.children.borrow();
            let at = kids.iter().position(|k| Rc::ptr_eq(k, x))?;
            stack.extend(kids[at + 1..].iter().cloned());
        }
        while let Some(n) = stack.pop() {
            if !matches!(n.data, NodeData::Element { .. }) {
                continue;
            }
            stack.extend(n.children.borrow().iter().cloned());
            els.push(n);
            if els.len() * n_sel > CLASS_PROBE_MAX {
                return None;
            }
        }
    }
    let mut p = ClassProbe { deps, rules, els, before: Vec::new() };
    p.before = class_probe_bits(&p);
    Some(p)
}

/// QYTC1d, after the write: did every reached element keep every selector match? Then no
/// element's style changed and the cached layout is still the page's.
pub(crate) fn class_write_post(p: ClassProbe) -> bool {
    class_probe_bits(&p) == p.before
}

#[derive(Default)]
pub(crate) struct GeomCache {
    key: Option<(u64, u32, u32, (u64, u64))>,
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
    /// QYTC1a: the laid tree flattened in pre-order (every `Laid`, anonymous ones too), so a
    /// contained inline-style write can shift a subtree and redo its ancestors' extents exactly.
    skel: Vec<Skel>,
    /// QYTC1a: element node -> index of its own (outermost, first) `skel` entry.
    skel_of: HashMap<usize, u32>,
    /// QYTC1a: nodes with more than one `skel` entry (a wrapper sharing the node, generated boxes).
    skel_multi: std::collections::HashSet<usize>,
    /// QYTC1a: the rules of the last layout's list that declare `!important` `left`/`top`/
    /// `transform` (or an `!important` the scan cannot pin to a property); `None` = unknown.
    imp_geo: Option<ImpGeo>,
    /// QYTC1a: a subtree was shifted in place; glyph rects are not shifted, so a `text_rects`
    /// read lays out again.
    glyphs_stale: bool,
    /// QINC1b (R60 4.4): next unused paragraph id; never restarts.
    next_para_id: usize,
    /// QINC1b: persists per owner, swept of ids no live `Laid` uses (none is kept here).
    shape_cache: ShapeCache,
    /// QINC1i: the rects came from the engine's kept laid tree; `glyphs` are collected from it on the
    /// first `text_rects` read (R60 9: lazy glyphs).
    glyphs_lazy: bool,
}

/// QYTC1a: one `Laid` of the last forced layout (see `GeomCache::skel`).
#[derive(Clone, Copy)]
struct Skel {
    node: usize,
    /// Index of the parent `Laid` (`u32::MAX` for the root).
    parent: u32,
    /// One past the last index of this `Laid`'s subtree.
    end: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    /// `walk`'s extent (right, bottom) of this `Laid` and its subtree.
    ext: (f32, f32),
    flags: u8,
}

/// `position: absolute | fixed`.
const SK_ABS: u8 = 1;
/// `width` is not `auto` (nor a sizing keyword).
const SK_WIDTH: u8 = 2;
const SK_RIGHT_AUTO: u8 = 4;
const SK_BOTTOM_AUTO: u8 = 8;
/// A CSS animation runs on the box (its keyframes beat the inline style).
const SK_ANIM: u8 = 16;
/// The box is (inside) a `position: fixed` subtree (`walk`'s `fixed`).
const SK_FIXED_IN: u8 = 32;

fn skel_flags(s: &ComputedStyle) -> u8 {
    let mut f = 0;
    if matches!(s.position, Position::Absolute | Position::Fixed) {
        f |= SK_ABS;
    }
    if s.width.is_some() {
        f |= SK_WIDTH;
    }
    if s.inset.right.is_none() {
        f |= SK_RIGHT_AUTO;
    }
    if s.inset.bottom.is_none() {
        f |= SK_BOTTOM_AUTO;
    }
    if s.animations.is_some() || s.transitions.is_some() {
        f |= SK_ANIM;
    }
    f
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
/// QYTC1a: also appends every `Laid` to `skel` in pre-order (`parent` = the caller's index).
fn walk(l: &Laid, fixed: bool, out: &mut HashMap<usize, GeomRect>, skel: &mut Vec<Skel>, parent: u32) -> (f32, f32) {
    let fixed = fixed || l.fixed;
    let mut ext = (l.x + l.w, l.y + l.h);
    let me = skel.len() as u32;
    let flags = skel_flags(&l.style) | if fixed { SK_FIXED_IN } else { 0 };
    skel.push(Skel { node: l.style.node, parent, end: 0, x: l.x, y: l.y, w: l.w, h: l.h, ext, flags });
    // The outermost box of an element wins (a wrapper may share its node).
    let own = l.style.node != 0 && !out.contains_key(&l.style.node);
    if own {
        out.insert(l.style.node, GeomRect { x: l.x, y: l.y, w: l.w, h: l.h, fixed, ink_r: ext.0, ink_b: ext.1 });
    }
    let mut sub = |c: &Laid, ext: &mut (f32, f32)| {
        let e = crate::stack::grow(|| walk(c, fixed, out, skel, me)); // QSTK1x: deep recursion
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
    let end = skel.len() as u32;
    let e = &mut skel[me as usize];
    e.end = end;
    e.ext = ext;
    ext
}

/// QYTC1a: the rules of `rules` that declare `left`/`top`/`transform` `!important` (or an
/// `!important` `var()`/CSS-wide value, which is not pinned to a property here). Only blocks
/// containing a `!` are parsed.
fn rules_important_geo(rules: &[StyleRule]) -> Vec<u32> {
    (0..rules.len())
        .filter(|&i| {
            let r = &rules[i];
            if !r.may_declare_important() {
                return false;
            }
            let d = r.important();
            d.left.is_some() || d.top.is_some() || d.transform.is_some() || !d.var_decls.is_empty() || !d.wide.is_empty()
        })
        .map(|i| i as u32)
        .collect()
}

/// QYTC1a: the selectors of a rule list's `rules_important_geo` rules, with the list's
/// `rules_cache` generation. QY53f1d: never the list's `Rc` itself: a second owner makes
/// `rules_cache` re-parse the whole page CSS (4.4 MB on YouTube) on every `<style>` insert
/// instead of splicing in the new piece.
type ImpGeo = Rc<(u64, Vec<SelectorList<OurSelectorImpl>>)>;

/// QYTC1a: may one of those `!important` rules apply to element `x`? (`@media` is not
/// evaluated: a rule whose selector matches counts.)
fn important_geo_matches(imp: &ImpGeo, x: &Handle) -> bool {
    let el = crate::dom::El(x.clone());
    let mut caches = SelectorCaches::default();
    imp.1.iter().any(|sel| {
        let mut ctx = MatchingContext::new(MatchingMode::Normal, None, &mut caches, QuirksMode::NoQuirks, NeedsSelectorFlags::No, MatchingForInvalidation::No);
        matches_selector_list(sel, &el, &mut ctx)
    })
}

thread_local! {
    /// QYTC1a: `rules_important_geo` of the last rule list it ran for (keyed by its `rules_cache` generation).
    static IMP_GEO: RefCell<Option<ImpGeo>> = const { RefCell::new(None) };
    /// QYTC1a: in-place shifts on this thread; `__d_ggen` adds it so script-side caches keyed on
    /// the geometry generation (IntersectionObserver, Range glyph rects) still see a move.
    static GEOM_SHIFTS: Cell<u64> = const { Cell::new(0) };
}

/// QYTC1a: how many contained style writes moved a box in place on this thread.
pub(crate) fn geom_shifts() -> u64 {
    GEOM_SHIFTS.with(Cell::get)
}

/// QYTC1a: `rules_important_geo` for the rule list of the latest build on this thread.
/// `None` (every write lays out) when that build's list is no longer `rules_cache`'s current one.
fn current_important_geo() -> Option<ImpGeo> {
    let (rules, generation, _, _) = crate::rules_cache::current()?;
    if !crate::style::RULE_DERIVED.with(|d| d.borrow().as_ref().is_some_and(|x| Rc::ptr_eq(&x.rules, &rules))) {
        return None;
    }
    IMP_GEO.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(v) = c.as_ref()
            && v.0 == generation
        {
            return Some(v.clone());
        }
        let sels = rules_important_geo(&rules).into_iter().map(|i| rules[i as usize].selectors.clone()).collect();
        let v = Rc::new((generation, sels));
        *c = Some(v.clone());
        Some(v)
    })
}

/// QYTC1a: the summed translate of a parsed `transform` list as `(px, fraction)` per axis, in the
/// order `resolve_style` sums it; `None` when a length is not `px`/`%` (it needs a font size).
fn translate_sum(list: &[(Length, Length)]) -> Option<((f32, f32), (f32, f32))> {
    let (mut px, mut py, mut fx, mut fy) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (x, y) in list {
        for (l, p, f) in [(x, &mut px, &mut fx), (y, &mut py, &mut fy)] {
            match l {
                Length::Percent(v) => *f += v,
                Length::Px(v) => *p += v,
                _ => return None,
            }
        }
    }
    Some(((px, fx), (py, fy)))
}

/// QYTC1a: how far a `style` write from `old` to `new` moves an absolutely/fixed positioned box
/// (`flags`, `w`, `h` of the last layout), or `None` when it may change anything else.
fn style_write_delta(old: &str, new: &str, flags: u8, w: f32, h: f32) -> Option<(f32, f32)> {
    let (on, oi) = crate::css::parse_declarations_split(old);
    let (nn, ni) = crate::css::parse_declarations_split(new);
    // `var()` values are re-parsed per element; the inline `!important` part must not change and
    // must not set what the normal part is allowed to change (it would win over it).
    if !on.var_decls.is_empty() || !nn.var_decls.is_empty() || oi != ni || oi.left.is_some() || oi.top.is_some() || oi.transform.is_some() {
        return None;
    }
    // Everything but the fields handled below must be identical.
    let rest = |d: &Declarations| {
        let mut d = d.clone();
        d.opacity = None;
        d.background_color = None;
        d.background_position = None;
        d.background_size = None;
        d.transform = None;
        d.left = None;
        d.top = None;
        d
    };
    if rest(&on) != rest(&nn) {
        return None;
    }
    // `opacity < 1` makes a stacking context, into which layout hoists z-indexed boxes: stay below 1.
    if on.opacity != nn.opacity && !matches!((on.opacity, nn.opacity), (Some(a), Some(b)) if a < 1.0 && b < 1.0) {
        return None;
    }
    // A background colour on an inline descendant (`inherit`) adds a box: keep one set both times.
    if on.background_color != nn.background_color && (on.background_color.is_none() || nn.background_color.is_none()) {
        return None;
    }
    let (mut dx, mut dy) = (0.0f32, 0.0f32);
    if on.transform != nn.transform {
        // An undeclared side would come from the cascade, which is not known here.
        let (Some(a), Some(b)) = (&on.transform, &nn.transform) else { return None };
        let (a, b) = (translate_sum(a)?, translate_sum(b)?);
        dx += (b.0.0 + b.0.1 * w) - (a.0.0 + a.0.1 * w);
        dy += (b.1.0 + b.1.1 * h) - (a.1.0 + a.1.1 * h);
    }
    if on.left != nn.left {
        // `left` places the box only while `right` is `auto`; a definite width keeps the
        // shrink-to-fit width (which depends on `left`) out.
        let (Some(Length::Px(a)), Some(Length::Px(b))) = (on.left, nn.left) else { return None };
        if flags & SK_WIDTH == 0 || flags & SK_RIGHT_AUTO == 0 {
            return None;
        }
        dx += b - a;
    }
    if on.top != nn.top {
        let (Some(Length::Px(a)), Some(Length::Px(b))) = (on.top, nn.top) else { return None };
        if flags & SK_BOTTOM_AUTO == 0 {
            return None;
        }
        dy += b - a;
    }
    Some((dx, dy))
}

/// QP2/QY53o3a: safety cap on the paragraphs the JS thread's memo keeps AFTER a layout's sweep
/// dropped what it did not use (a layout's own entries are never capped: 7.5k on Wikipedia).
pub(crate) const SHAPE_MEMO_CAP: usize = 32768;

impl GeomCache {
    /// QOBS7: `((rect entries, bytes), (glyph entries = text nodes, bytes))`, by capacity.
    pub(crate) fn mem_estimate(&self) -> ((usize, usize), (usize, usize)) {
        let rects = (
            self.rects.len(),
            self.rects.capacity() * (std::mem::size_of::<(usize, GeomRect)>() + 1)
                + self.skel.capacity() * std::mem::size_of::<Skel>()
                + self.skel_of.capacity() * (std::mem::size_of::<(usize, u32)>() + 1),
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
        // QYTC1d: a geometry read: class writes after it may be probed again.
        PROBE_BUDGET.with(|b| b.set(CLASS_PROBES_PER_READ));
        if self.key == Some(key) && self.clock_ok() {
            return;
        }
        #[cfg(test)]
        GEOM_LAYOUTS.with(|c| c.set(c.get() + 1));
        // QANIMHLc: the last layout styled a running animation: this one is at the clock of the read
        // (headless frames are not added to move it, so the document clock may be a frame or more behind).
        if self.anim_clock.is_some() {
            crate::anim_eval::set_document_clock(crate::anim_eval::read_clock_ms());
        }
        // QINC1i: through the incremental engine (`SCRATCHY_INCR=1`); from scratch when it has no tree.
        if crate::incr_engine::forced_on() && self.refresh_engine(page, vw, vh, key) {
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
        // QY53l2: a layout replaces the hidden-subtree snapshot; collect the new one.
        HIDDEN.with(|h| *h.borrow_mut() = None);
        ATTR_SNAP.with(|h| *h.borrow_mut() = None);
        crate::images::DISPLAY_NONE_ROOTS.with(|r| *r.borrow_mut() = Some(Vec::new()));
        // A layout bug must not take the whole browser down with a page
        // script: on a panic the page simply has no geometry (all zeros).
        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let tree = build_layout_tree_with_resources(page, vw, Some(vh), &css, &HashMap::new());
            let deps = crate::style::rule_child_deps();
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
            (laid, glyphs, deps)
        }));
        set_viewport(prev_vp.0, prev_vp.1);
        crate::layout::memo_sweep(SHAPE_MEMO_CAP, 1);
        let found = crate::images::DISPLAY_NONE_ROOTS.with(|r| r.borrow_mut().take()).unwrap_or_default();
        let t_phase = crate::perf::phase(crate::perf::Phase::RectWalk);
        let prev = std::mem::take(&mut self.rects);
        self.page = (0.0, 0.0);
        self.glyphs.clear();
        self.glyphs_stale = false;
        self.glyphs_lazy = false;
        self.imp_geo = None;
        self.skel.clear();
        self.skel_of.clear();
        self.skel_multi.clear();
        match done {
            Ok((Some(l), glyphs, deps)) => {
                self.page = (crate::layout::doc_scroll_width(&l), l.h); // QXSCR1: the scroll width
                walk(&l, false, &mut self.rects, &mut self.skel, u32::MAX);
                // QEL2b: the ResizeObserver targets' boxes of this layout.
                crate::ro_geom::forced_layout(page, &l, (wgen, key.3), vp.0, vp.1);
                for (i, e) in self.skel.iter().enumerate() {
                    if e.node != 0 && self.skel_of.insert(e.node, i as u32).is_some() {
                        self.skel_multi.insert(e.node);
                    }
                }
                // `insert` kept the last index; the element's own entry is its first (outermost).
                for (i, e) in self.skel.iter().enumerate().rev() {
                    if e.node != 0 && self.skel_multi.contains(&e.node) {
                        self.skel_of.insert(e.node, i as u32);
                    }
                }
                self.imp_geo = current_important_geo();
                self.glyphs = glyphs;
                let snap = hidden_snapshot(page, wgen, key.3, found, deps);
                HIDDEN.with(|h| *h.borrow_mut() = snap);
                if let Some(r) = crate::style::rule_attr_readers() {
                    ATTR_SNAP.with(|a| *a.borrow_mut() = Some(((wgen, key.3), r)));
                }
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
        HIDDEN.with(|h| *h.borrow_mut() = None);
        ATTR_SNAP.with(|h| *h.borrow_mut() = None);
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
        self.glyphs_stale = false;
        self.glyphs_lazy = false;
        self.imp_geo = None;
        self.skel.clear();
        self.skel_of.clear();
        self.skel_multi.clear();
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
        let mut skel = Vec::new();
        match (&found, &fresh) {
            (Some(_), Some(l)) => {
                // Not trusted any more: every stage starts over; this read uses the from-scratch layout.
                crate::incr_style::drop_engines();
                self.page = (crate::layout::doc_scroll_width(l), l.h); // QXSCR1: the scroll width
                walk(l, false, &mut self.rects, &mut skel, u32::MAX);
                walk_glyphs(l, &self.shape_cache, &mut self.glyphs);
                crate::ro_geom::forced_layout(page, l, (key.0, key.3), vp.0, vp.1);
            }
            _ => {
                let rects = &mut self.rects;
                self.page = crate::incr_layout::with_laid(|l, _| {
                    walk(l, false, rects, &mut skel, u32::MAX);
                    crate::ro_geom::forced_layout(page, l, (key.0, key.3), vp.0, vp.1);
                    (crate::layout::doc_scroll_width(l), l.h)
                })
                .unwrap_or((0.0, 0.0));
                self.glyphs_lazy = true;
            }
        }
        drop(skel);
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
        let r = find_in(&self.rects, h);
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

    /// QYTC1a: a `style` attribute write on `x` (from `old` to `new`, both present) that only
    /// moves an absolutely/fixed positioned box (`left`/`top` in px, a translate) or changes
    /// paint-only fields: shifts its laid subtree and redoes its ancestors' extents in place and
    /// returns true, so the caller does not bump the geometry generation. False (bump) unless the
    /// cache is the current layout (`ggen`, CSS version), no selector or `attr()` reads `style`,
    /// no rule declares `!important` `left`/`top`/`transform`, the box has no running animation
    /// and no box of its DOM subtree lies outside its laid subtree inside a `fixed` one (boxes
    /// hoisted into a stacking-context owner moved with it and are shifted too).
    pub(crate) fn style_write_in_place(&mut self, x: &Handle, old: Option<&str>, new: Option<&str>, ggen: u64) -> bool {
        let (Some(old), Some(new)) = (old, new) else { return false };
        // QINC1i: an offset-only write re-places the box in the engine (R60 8.3: QYTC1a bypassed).
        if crate::incr_engine::forced_on() {
            return false;
        }
        let Some(key) = self.key else { return false };
        if key.0 != ggen || key.3 != crate::js_style::css_ver() || !matches!(x.data, NodeData::Element { .. }) {
            return false;
        }
        let unread = ATTR_SNAP.with(|s| matches!(s.borrow().as_ref(), Some(((g, css), r)) if *g == ggen && *css == key.3 && !r.reads("style", element_tag(x))));
        if !unread {
            return false;
        }
        let Some(&i) = self.skel_of.get(&node_id(x)) else { return false };
        if self.imp_geo.as_ref().is_none_or(|imp| important_geo_matches(imp, x)) {
            return false;
        }
        let e = self.skel[i as usize];
        // Without SK_ABS a `left`/`top` change is refused below (it may be a relative offset), and
        // only writes that move nothing pass (paint-only fields, a scale beside the translate).
        let abs = e.flags & SK_ABS != 0;
        let Some((dx, dy)) = style_write_delta(old, new, if abs { e.flags } else { 0 }, e.w, e.h) else { return false };
        if !dx.is_finite() || !dy.is_finite() {
            return false;
        }
        if dx == 0.0 && dy == 0.0 {
            return true;
        }
        if !abs || e.flags & SK_ANIM != 0 {
            return false;
        }
        // The laid subtrees that move: the element's own, plus those of its DOM descendants that
        // layout hoisted out of it into a stacking-context owner after it was placed (they moved
        // with it: `resolve_positioned` shifts before `hoist_negative_z`). A `fixed` descendant
        // did not (its containing block is the viewport), so it makes the write fall back.
        let mut roots: Vec<(u32, u32)> = vec![(i, e.end)];
        let mut stack = vec![x.clone()];
        while let Some(n) = stack.pop() {
            if matches!(n.data, NodeData::Element { .. }) {
                let id = node_id(&n);
                let entries: Vec<u32> = if self.skel_multi.contains(&id) {
                    self.skel.iter().enumerate().filter(|(_, s)| s.node == id).map(|(k, _)| k as u32).collect()
                } else {
                    self.skel_of.get(&id).copied().into_iter().collect()
                };
                for k in entries {
                    if roots.iter().any(|&(a, b)| (a..b).contains(&k)) {
                        continue;
                    }
                    let s = self.skel[k as usize];
                    if (e.flags | s.flags) & SK_FIXED_IN != 0 {
                        return false;
                    }
                    roots.push((k, s.end));
                }
            }
            stack.extend(n.children.borrow().iter().cloned());
        }
        for &(lo, hi) in &roots {
            for j in lo..hi {
                let s = &mut self.skel[j as usize];
                s.x += dx;
                s.y += dy;
                s.ext = (s.ext.0 + dx, s.ext.1 + dy);
                let s = *s;
                if s.node != 0
                    && self.skel_of.get(&s.node) == Some(&j)
                    && let Some(r) = self.rects.get_mut(&s.node)
                {
                    (r.x, r.y, r.ink_r, r.ink_b) = (s.x, s.y, s.ext.0, s.ext.1);
                }
            }
        }
        // Each chain is redone bottom-up after every subtree moved, so a chain that stops early
        // (an unchanged extent) leaves nothing stale another chain does not redo.
        for &(root, _) in &roots {
            self.redo_ancestor_extents(self.skel[root as usize].parent);
        }
        self.glyphs_stale = true;
        GEOM_SHIFTS.with(|g| g.set(g.get() + 1));
        true
    }

    /// QYTC1a: recomputes `walk`'s extent of `p` and its ancestors from their children's, up to
    /// the first one that does not change.
    fn redo_ancestor_extents(&mut self, mut p: u32) {
        while p != u32::MAX {
            let pe = self.skel[p as usize];
            let mut ext = (pe.x + pe.w, pe.y + pe.h);
            let mut c = p + 1;
            while c < pe.end {
                let ce = &self.skel[c as usize];
                ext = (ext.0.max(ce.ext.0), ext.1.max(ce.ext.1));
                c = ce.end;
            }
            if ext == pe.ext {
                break;
            }
            self.skel[p as usize].ext = ext;
            if pe.node != 0
                && self.skel_of.get(&pe.node) == Some(&p)
                && let Some(r) = self.rects.get_mut(&pe.node)
            {
                (r.ink_r, r.ink_b) = ext;
            }
            p = pe.parent;
        }
    }

    /// Test-only: every element rect of the cache, by node id.
    #[cfg(test)]
    pub(crate) fn all_rects(&self) -> Vec<(usize, [f32; 6], bool)> {
        let mut v: Vec<_> = self.rects.iter().map(|(k, r)| (*k, [r.x, r.y, r.w, r.h, r.ink_r, r.ink_b], r.fixed)).collect();
        v.sort_by_key(|e| e.0);
        v
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

/// QY53e2: the App's own complete layout, as a read-only rect map for IntersectionObserver.
struct Rendered {
    stamp: RenderStamp,
    vp: (u32, u32),
    rects: HashMap<usize, GeomRect>,
}

/// What a layout was built from: the geometry generation (plus in-place shifts) and the CSS versions.
pub(crate) type RenderStamp = (u64, (u64, u64));

thread_local! {
    static RENDERED: RefCell<Option<Rendered>> = const { RefCell::new(None) };
}

/// QY53e2: the stamp of the DOM/CSS as they are now; the App records it when it builds a tree.
pub(crate) fn render_stamp() -> RenderStamp {
    (crate::js_dom::geom_gen() + geom_shifts(), crate::js_style::css_ver())
}

/// QY53e2: keeps the rects of the App's complete layout `l`, built at `stamp` for a `vw` x `vh` window.
pub(crate) fn set_rendered(l: &Laid, stamp: RenderStamp, vw: u32, vh: u32) {
    let mut rects = HashMap::new();
    walk(l, false, &mut rects, &mut Vec::new(), u32::MAX);
    RENDERED.with(|r| *r.borrow_mut() = Some(Rendered { stamp, vp: (vw, vh), rects }));
}

pub(crate) fn clear_rendered() {
    RENDERED.with(|r| *r.borrow_mut() = None);
}

/// QY53e2: is the kept App layout of the page as it is now (same DOM/CSS stamp) at this viewport?
pub(crate) fn rendered_ok(vw: u32, vh: u32) -> bool {
    // QINC1i: IntersectionObserver reads the engine's tree (a recheck runs an incremental update).
    if crate::incr_engine::forced_on() {
        return false;
    }
    RENDERED.with(|r| r.borrow().as_ref().is_some_and(|x| x.stamp == render_stamp() && x.vp == (vw, vh)))
}

/// QY53e2: `GeomCache::rect_of`'s `[x, y, w, h, fixed, ink_r, ink_b, page_w, page_h]` from the App layout
/// (page size 0); empty when the element has no box or no valid App layout is kept.
pub(crate) fn rendered_rect(h: &Handle) -> Vec<f64> {
    RENDERED.with(|r| {
        let r = r.borrow();
        let Some(r) = r.as_ref() else { return Vec::new() };
        let Some(b) = find_in(&r.rects, h) else { return Vec::new() };
        vec![b.x as f64, b.y as f64, b.w as f64, b.h as f64, if b.fixed { 1.0 } else { 0.0 }, b.ink_r as f64, b.ink_b as f64, 0.0, 0.0]
    })
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
        if self.glyphs_stale {
            // QYTC1a: a box moved in place since the layout; its glyphs did not.
            self.key = None;
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
