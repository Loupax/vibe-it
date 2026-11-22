//! QINC1d (R60 section 5): the invalidation map. Built once per `rules_cache` generation from every selector of the
//! rule list, it answers "this DOM/state change happened: which elements may now match a different set of
//! selectors?" so an incremental restyle (QINC1f) can re-match only those. It generalizes three one-feature
//! answers that exist today and are NOT changed by this item: `js_geom::ClassDeps` (class writes), `style::HoverKeys`
//! (hover chains) and `style::AttrReaders` (attribute names a rule list reads).
//!
//! Model. Every selector is cut into compounds, right to left; the rightmost one is the *subject*. A feature (class,
//! id, attribute name, state pseudo-class, structural pseudo-class, sibling adjacency) found in compound `k` is stored
//! as an [`Entry`]: the path of combinators ([`Hop`]s) from an element carrying the feature to the subject element it
//! can influence, plus a [`Filter`] (the subject compound's tag / id / classes, Blink's descendant-set filter). A write
//! on element `E` then marks: start at `E`, walk the hops (descendants, children, next / later siblings...), keep the
//! elements passing the filter. No hops = `E` itself.
//!
//! Conservative where exactness is not worth it: `:is/:not/:where` contents are collected with their own hops prefixed
//! (exact reach, features treated as unconditional), `:nth-child(of S)` is structural with `S` carried by every
//! sibling, `:host`, `::slotted`, `::part` and unknown combinators make the map *global* (every call returns
//! `Mark::Global`). The filter only uses the subject's own tag/id/class, which a non-`class`/`id` write never changes.
//!
//! QAPI6 `:has(R)`: every element a relative selector `R` looks at is a descendant of the anchor (`:has(.a)`,
//! `:has(> .a .b)`) or a later sibling of it or a descendant of one (`:has(+ .a)`, `:has(~ .a .b)`). So a feature
//! anywhere in `R` reaches the anchors through ONE hop, whatever `R`'s own combinators are: [`Hop::Anc`] (the
//! carrier's ancestors) or, for a sibling-anchored `R`, [`Hop::AncSib`] (the earlier siblings of the carrier and of
//! each of its ancestors); the anchors are filtered by the tag/id/class of the compound holding the `:has`
//! (`Entry::anchor`), then the outer hops lead to the subject as usual. Inserting or removing ANY element can
//! change what `R` finds: [`Key::HasTree`] holds those paths, walked from the inserted node / the removal point.
//! A selector with its own combinators nested inside `R` (`:has(:is(.a .b))`) can break out above the anchor:
//! global.
#![allow(dead_code)]
use super::*;
use selectors::parser::{Combinator, NthType, Selector};

type Sel = Selector<OurSelectorImpl>;

/// What to do with an element a change reaches. Ordered by strength when the same element is reached twice.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Mark {
    /// Only its inherited/ambient inputs may differ (`style` attribute): re-cascade, matches stay valid.
    Recascade,
    /// Its matched rule set may differ: re-match, re-cascade.
    Restyle,
    /// The map cannot bound the effect (`:has`, ...): everything is dirty. Returned on the changed element.
    Global,
}

/// The state pseudo-classes scratchy parses (`OurPseudoClass`) that depend on mutable state.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum StateKind {
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    Target,
    Checked,
    Indeterminate,
    Disabled,
    Enabled,
    Link,
    Lang,
}

/// One step from an element carrying a feature towards the subject it influences.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) enum Hop {
    Child,
    Desc,
    Next,
    Later,
    /// Every other element child of the same parent (`:nth-child(of S)`).
    AnySib,
    /// QAPI6: every ancestor element (the anchors of a descendant-direction `:has()`).
    Anc,
    /// QAPI6: every earlier element sibling of the element and of each of its ancestors (the anchors of a
    /// sibling-direction `:has(+ ..)` / `:has(~ ..)`).
    AncSib,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct Filter {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    /// QAPI6: attribute names the compound requires (`[role=dialog]`, `[hidden]`). Only checked on elements a
    /// walk REACHES (never on the changed element, whose write may be to that very attribute).
    attrs: Vec<String>,
}

/// QAPI6: whether `h` has every attribute `f` requires.
fn has_attrs(h: &Handle, f: &Filter) -> bool {
    f.attrs.iter().all(|a| element_attr(h, a).is_some())
}

/// QAPI6: `passes` plus the attribute names, for an element a walk reached.
fn passes_reached(h: &Handle, f: &Option<Rc<Filter>>) -> bool {
    passes(h, f) && f.as_ref().is_none_or(|f| has_attrs(h, f))
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct Entry {
    hops: Box<[Hop]>,
    filter: Option<Rc<Filter>>,
    /// QINC1j-ab: the tag / id / classes of the compound that holds the feature, checked on the element
    /// carrying it (the change point) before any hop is walked. A state change on an element that does
    /// not have them cannot change whether that compound matches (`:hover` on `ytd-app` does not reach
    /// `.menu:hover .icon`). Not checked for a `class` / `id` write, which changes those very features.
    carrier: Option<Rc<Filter>>,
    /// QAPI6: the tag / id / classes of the compound holding the `:has()`, checked on the anchors the `Anc` /
    /// `AncSib` hop reaches (`None` outside `:has`, or when that compound has none).
    anchor: Option<Rc<Filter>>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Key {
    Class(String),
    Id(String),
    Attr(String),
    State(StateKind),
    /// `:first-child`, `:last-child`, `:only-child`: carriers are the elements at the edges.
    First,
    Last,
    Only,
    /// Anything position dependent beyond the edges (`:nth-*`, `-of-type`): every element child.
    Nth,
    /// `:empty`: carrier is the parent.
    Empty,
    /// A `+` combinator: carrier is the element after the change point.
    SibNext,
    /// A `~` combinator: carrier is the element after the change point and every later sibling.
    SibLater,
    /// QAPI6: a `:has()`: carrier is an inserted element (or the removal point), whose anchors may now find a
    /// different set of elements.
    HasTree,
    /// QAPI6b: every entry whose path starts at a `:has` anchor hop (`Anc` / `AncSib`), carrier filters dropped:
    /// "something below this element changed", answered once per unstyled subtree (`for_has_any`).
    HasAny,
}

/// A child-list change, as the write log records it (R60 section 5). A move is a `Removed` then an `Inserted`.
pub(crate) enum ChildChange {
    /// `node` is now a child of the parent (at its current position).
    Inserted(Handle),
    /// `node` is no longer a child; it used to be at child-node index `index`.
    Removed { node: Handle, index: usize },
}

#[derive(Default)]
pub(crate) struct InvalidationMap {
    pub(crate) generation: u64,
    keys: HashMap<Key, Vec<Entry>>,
    /// Some selector uses `:host` and friends: no verdict.
    global: bool,
    /// QAPI6: some selector uses `:has()` (the `HasTree` key / `Anc` hops exist).
    has: bool,
    /// QAPI6b: some `HasAny` path reached its anchor hop through sibling hops (`for_has_any` starts at every sibling).
    has_any_wide: bool,
    /// Some `content: attr()` cannot be pinned to a name: every attribute write restyles its element.
    attr_all: bool,
    /// Quick gates.
    has_state_checked: bool,
    has_state_disabled: bool,
    /// QAPI6: `Some` between `begin_batch` and `end_batch` (one engine update): the walks done so far.
    batch: RefCell<Option<WalkMemo>>,
}

/// `(element, hash of the rest of a walk)` pairs (see `walk`).
type WalkMemo = HashSet<(usize, u64)>;

/// The result accumulator: one mark per element (the strongest), in first-reached order.
type NodeSet = HashSet<usize, std::hash::BuildHasherDefault<crate::dom::PtrHasher>>;

/// Entries of one key sharing a hop path: the path and their subject filters (`None` = no filter).
type HopGroup<'a> = (&'a [Hop], Option<Vec<&'a Rc<Filter>>>);

#[derive(Default)]
struct Marks {
    out: Vec<(Handle, Mark)>,
    idx: HashMap<usize, usize, std::hash::BuildHasherDefault<crate::dom::PtrHasher>>,
}

impl Marks {
    fn add(&mut self, h: &Handle, m: Mark) {
        match self.idx.get(&node_id(h)) {
            Some(&i) => {
                if self.out[i].1 < m {
                    self.out[i].1 = m;
                }
            }
            None => {
                self.idx.insert(node_id(h), self.out.len());
                self.out.push((h.clone(), m));
            }
        }
    }
}

fn is_el(h: &Handle) -> bool {
    matches!(h.data, NodeData::Element { .. })
}

fn el_children(h: &Handle) -> Vec<Handle> {
    h.children.borrow().iter().filter(|c| is_el(c)).cloned().collect()
}

fn attr_str(h: &Handle, name: &str) -> Option<String> {
    element_attr(h, name).map(|v| v.to_string())
}

fn passes(h: &Handle, f: &Option<Rc<Filter>>) -> bool {
    let Some(f) = f else { return true };
    if let Some(t) = &f.tag
        && !element_tag(h).eq_ignore_ascii_case(t)
    {
        return false;
    }
    if let Some(i) = &f.id
        && element_attr(h, "id").is_none_or(|v| *v != **i)
    {
        return false;
    }
    if !f.classes.is_empty() {
        let Some(c) = element_attr(h, "class") else { return false };
        if !f.classes.iter().all(|k| c.split_ascii_whitespace().any(|t| t == k)) {
            return false;
        }
    }
    true
}

/// QAPI6: an element's tag / id / class tokens read once, for checking it against many filters (`passes`).
struct Probe {
    tag: String,
    id: Option<String>,
    classes: Vec<String>,
}

impl Probe {
    fn of(h: &Handle) -> Probe {
        Probe {
            tag: element_tag(h).to_string(),
            id: element_attr(h, "id").map(|v| v.to_string()),
            classes: element_attr(h, "class").map(|c| c.split_ascii_whitespace().map(str::to_string).collect()).unwrap_or_default(),
        }
    }

    /// `passes(h, f)` for the element this probe read.
    fn passes(&self, f: &Option<Rc<Filter>>) -> bool {
        let Some(f) = f else { return true };
        f.tag.as_ref().is_none_or(|t| self.tag.eq_ignore_ascii_case(t))
            && f.id.as_ref().is_none_or(|i| self.id.as_ref() == Some(i))
            && f.classes.iter().all(|k| self.classes.iter().any(|t| t == k))
    }
}

/// Whether `h` passes at least one of `fs` (an empty list passes everything): `passes` for a group of
/// entries sharing one hop path, reading the element's tag / id / class once.
fn passes_any(h: &Handle, fs: &[&Rc<Filter>]) -> bool {
    if fs.is_empty() {
        return true;
    }
    let tag = element_tag(h);
    let id = element_attr(h, "id");
    let class = element_attr(h, "class");
    fs.iter().any(|f| {
        if let Some(t) = &f.tag
            && !tag.eq_ignore_ascii_case(t)
        {
            return false;
        }
        if let Some(i) = &f.id
            && id.as_ref().is_none_or(|v| **v != **i)
        {
            return false;
        }
        if !f.classes.is_empty() {
            let Some(c) = &class else { return false };
            if !f.classes.iter().all(|k| c.split_ascii_whitespace().any(|t| t == k)) {
                return false;
            }
        }
        has_attrs(h, f)
    })
}

fn later_siblings(h: &Handle, include_self: bool) -> Vec<Handle> {
    let Some(p) = parent_handle(h) else { return Vec::new() };
    let ch = p.children.borrow();
    let Some(pos) = ch.iter().position(|c| Rc::ptr_eq(c, h)) else { return Vec::new() };
    let from = if include_self { pos } else { pos + 1 };
    ch[from..].iter().filter(|c| is_el(c)).cloned().collect()
}

/// `hop` applied to every element of `cur`; the result is deduplicated and only holds elements passing
/// one of `filters` (the last hop of a group of entries; empty = no filter).
fn step(cur: &[Handle], hop: Hop, filters: &[&Rc<Filter>]) -> Vec<Handle> {
    let mut out: Vec<Handle> = Vec::new();
    let mut seen = NodeSet::default();
    let push = |h: &Handle, out: &mut Vec<Handle>| {
        if passes_any(h, filters) {
            out.push(h.clone());
        }
    };
    match hop {
        Hop::Child => {
            for e in cur {
                for c in el_children(e) {
                    if seen.insert(node_id(&c)) {
                        push(&c, &mut out);
                    }
                }
            }
        }
        Hop::Desc => {
            // A node already walked (as a descendant of an earlier root) has had its whole subtree visited.
            let mut stack: Vec<Handle> = Vec::new();
            for e in cur {
                if seen.contains(&node_id(e)) {
                    continue;
                }
                stack.push(e.clone());
                while let Some(n) = stack.pop() {
                    for c in n.children.borrow().iter().filter(|c| is_el(c)) {
                        if seen.insert(node_id(c)) {
                            push(c, &mut out);
                            stack.push(c.clone());
                        }
                    }
                }
            }
        }
        Hop::Next => {
            for e in cur {
                if let Some(n) = later_siblings(e, false).into_iter().next()
                    && seen.insert(node_id(&n))
                {
                    push(&n, &mut out);
                }
            }
        }
        Hop::Later => {
            for e in cur {
                for n in later_siblings(e, false) {
                    if seen.insert(node_id(&n)) {
                        push(&n, &mut out);
                    }
                }
            }
        }
        Hop::AnySib => {
            for e in cur {
                let Some(p) = parent_handle(e) else { continue };
                for n in el_children(&p) {
                    if !Rc::ptr_eq(&n, e) && seen.insert(node_id(&n)) {
                        push(&n, &mut out);
                    }
                }
            }
        }
        Hop::Anc => {
            // An ancestor already reached had its own ancestors walked too.
            for e in cur {
                let mut p = parent_handle(e);
                while let Some(a) = p.filter(is_el) {
                    if !seen.insert(node_id(&a)) {
                        break;
                    }
                    push(&a, &mut out);
                    p = parent_handle(&a);
                }
            }
        }
        Hop::AncSib => {
            let mut walked = NodeSet::default();
            for e in cur {
                let mut x = Some(e.clone());
                while let Some(h) = x.filter(is_el) {
                    if !walked.insert(node_id(&h)) {
                        break;
                    }
                    earlier_siblings(&h, |s| {
                        if seen.insert(node_id(s)) {
                            push(s, &mut out);
                        }
                    });
                    x = parent_handle(&h);
                }
            }
        }
    }
    out
}

/// QAPI6: `f` on every earlier element sibling of `h`.
fn earlier_siblings(h: &Handle, mut f: impl FnMut(&Handle)) {
    let Some(p) = parent_handle(h) else { return };
    for c in p.children.borrow().iter() {
        if Rc::ptr_eq(c, h) {
            break;
        }
        if is_el(c) {
            f(c);
        }
    }
}

/// QAPI6: the anchors a `:has()` can have after a child of `parent` at child-node index `index` was removed:
/// `parent` and its ancestors (`Anc`), or the earlier siblings of the removal point and of `parent` and of each of
/// its ancestors (`AncSib`).
fn removal_anchors(parent: &Handle, index: usize, hop: Hop) -> Vec<Handle> {
    let mut out = Vec::new();
    if hop == Hop::Anc {
        let mut p = Some(parent.clone());
        while let Some(a) = p.filter(is_el) {
            out.push(a.clone());
            p = parent_handle(&a);
        }
    } else {
        out.extend(parent.children.borrow().iter().take(index).filter(|c| is_el(c)).cloned());
        out.extend(step(std::slice::from_ref(parent), Hop::AncSib, &[]));
    }
    out
}

fn tokens(v: Option<&str>) -> HashSet<&str> {
    v.map(|v| v.split_ascii_whitespace().collect()).unwrap_or_default()
}

/// A `Filter` for the subject compound `comps`, `None` when it has no tag/id/class to key on.
fn subject_filter(comps: &[&Component<OurSelectorImpl>]) -> Option<Rc<Filter>> {
    let mut f = Filter { tag: None, id: None, classes: Vec::new(), attrs: Vec::new() };
    for c in comps {
        match c {
            Component::LocalName(n) => f.tag = Some(n.lower_name.0.to_ascii_lowercase()),
            Component::ID(i) => f.id = Some(i.0.clone()),
            Component::Class(k) => f.classes.push(k.0.clone()),
            Component::AttributeInNoNamespaceExists { local_name_lower, .. } => f.attrs.push(local_name_lower.0.to_ascii_lowercase()),
            Component::AttributeInNoNamespace { local_name, .. } => f.attrs.push(local_name.0.to_ascii_lowercase()),
            Component::AttributeOther(a) => f.attrs.push(a.local_name_lower.0.to_ascii_lowercase()),
            _ => {}
        }
    }
    (f.tag.is_some() || f.id.is_some() || !f.classes.is_empty() || !f.attrs.is_empty()).then(|| Rc::new(f))
}

/// QAPI6: the tags / ids / classes present in a subtree (an inserted or removed one).
#[derive(Default)]
struct TreeSummary {
    tags: HashSet<String>,
    ids: HashSet<String>,
    classes: HashSet<String>,
    attrs: HashSet<String>,
}

impl TreeSummary {
    fn of(root: &Handle) -> TreeSummary {
        let mut s = TreeSummary::default();
        let mut stack = vec![root.clone()];
        while let Some(n) = stack.pop() {
            if !is_el(&n) {
                continue;
            }
            s.tags.insert(element_tag(&n).to_ascii_lowercase());
            if let Some(i) = element_attr(&n, "id") {
                s.ids.insert(i.to_string());
            }
            if let Some(c) = element_attr(&n, "class") {
                s.classes.extend(c.split_ascii_whitespace().map(str::to_string));
            }
            if let NodeData::Element { attrs, .. } = &n.data {
                s.attrs.extend(attrs.borrow().iter().map(|a| a.name.local.as_ref().to_ascii_lowercase()));
            }
            stack.extend(n.children.borrow().iter().filter(|c| is_el(c)).cloned());
        }
        s
    }

    /// Whether some element of the subtree may pass `f` (classes may come from different elements: a superset).
    fn may_hold(&self, f: &Option<Rc<Filter>>) -> bool {
        let Some(f) = f else { return true };
        f.tag.as_ref().is_none_or(|t| self.tags.contains(t))
            && f.id.as_ref().is_none_or(|i| self.ids.contains(i))
            && f.classes.iter().all(|c| self.classes.contains(c))
            && f.attrs.iter().all(|a| self.attrs.contains(a))
    }
}

/// Walks `hops` from `cur` and marks what is reached `Restyle`: `filters` (any of; empty = all) on the last hop,
/// `anchor` on the elements an `Anc` / `AncSib` hop reaches (QAPI6).
/// `memo` (QAPI6, an engine update's batch): `(element, rest of the walk)` pairs already walked in this batch; the
/// DOM does not change while a batch is answered, so walking one again marks nothing new.
fn walk(mut cur: Vec<Handle>, hops: &[Hop], filters: &[&Rc<Filter>], anchor: &Option<Rc<Filter>>, memo: Option<&RefCell<Option<WalkMemo>>>, out: &mut Marks) {
    for (i, hop) in hops.iter().enumerate() {
        if let Some(m) = memo
            && let Some(set) = m.borrow_mut().as_mut()
        {
            use std::hash::{Hash, Hasher};
            let mut k = std::collections::hash_map::DefaultHasher::new();
            hops[i..].hash(&mut k);
            for f in filters {
                (Rc::as_ptr(f) as usize).hash(&mut k);
            }
            anchor.as_ref().map(|a| Rc::as_ptr(a) as usize).hash(&mut k);
            let k = k.finish();
            cur.retain(|h| set.insert((node_id(h), k)));
            if cur.is_empty() {
                return;
            }
        }
        let last = i + 1 == hops.len();
        cur = step(&cur, *hop, if last { filters } else { &[] });
        if anchor.is_some() && matches!(hop, Hop::Anc | Hop::AncSib) {
            cur.retain(|h| passes_reached(h, anchor));
        }
        if cur.is_empty() {
            break;
        }
    }
    for h in &cur {
        out.add(h, Mark::Restyle);
    }
}

struct Builder {
    /// The carrier filter of the compound being collected (`Entry::carrier`).
    carrier: Option<Rc<Filter>>,
    /// Deduplicated (sorted) per key at the end of the build: one hash per add instead of a set of
    /// `(Key, Entry)` pairs (QINC1j-ab: the map is rebuilt for every rule-list generation).
    keys: HashMap<Key, Vec<Entry>>,
    global: bool,
    /// QAPI6: 0 outside `:has()`; 1 in its relative selector; 2+ in a selector nested inside it.
    has_depth: u8,
    /// QAPI6: `Entry::anchor` of the `:has()` being collected.
    anchor: Option<Rc<Filter>>,
    has: bool,
}

impl Builder {
    fn add(&mut self, key: Key, hops: &[Hop], filter: &Option<Rc<Filter>>) {
        let e = Entry { hops: hops.into(), filter: filter.clone(), carrier: self.carrier.clone(), anchor: self.anchor.clone() };
        self.keys.entry(key).or_default().push(e);
    }

    /// `selector`, one level deeper when inside a `:has()`.
    fn nested(&mut self, sel: &Sel, hops: &[Hop], filter: &Option<Rc<Filter>>) {
        let d = self.has_depth;
        if d > 0 {
            self.has_depth = d.saturating_add(1);
        }
        self.selector(sel, hops, filter);
        self.has_depth = d;
    }

    /// Collect the features of `sel`. `outer` = hops from `sel`'s subject to the top-level subject (empty at the top).
    fn selector(&mut self, sel: &Sel, outer: &[Hop], filter: &Option<Rc<Filter>>) {
        let mut seqs: Vec<(Vec<&Component<OurSelectorImpl>>, Option<Combinator>)> = Vec::new();
        let mut it = sel.iter();
        loop {
            let comps: Vec<_> = it.by_ref().collect();
            let next = it.next_sequence();
            let done = next.is_none();
            seqs.push((comps, next));
            if done {
                break;
            }
        }
        // The pseudo-element compound (`::before`) hangs off the same element: the subject is the one after it.
        let mut s = 0;
        while s + 1 < seqs.len() && seqs[s].1 == Some(Combinator::PseudoElement) {
            s += 1;
        }
        // QAPI6: inside `:has()` every compound reaches the anchors through the one hop at the front of `outer`; a
        // selector nested in the relative selector with combinators of its own can match outside the anchor.
        if self.has_depth >= 2 && seqs.len() > s + 1 {
            self.global = true;
        }
        let flat = self.has_depth > 0;
        // Hops from compound `k` (k >= s) to the subject.
        let hops_from = |this: &mut Builder, k: usize| -> Vec<Hop> {
            if flat {
                return outer.to_vec();
            }
            let mut h = Vec::new();
            for j in (s..k).rev() {
                match seqs[j].1 {
                    Some(Combinator::Descendant) => h.push(Hop::Desc),
                    Some(Combinator::Child) => h.push(Hop::Child),
                    Some(Combinator::NextSibling) => h.push(Hop::Next),
                    Some(Combinator::LaterSibling) => h.push(Hop::Later),
                    Some(Combinator::PseudoElement) => {}
                    _ => this.global = true,
                }
            }
            h.extend_from_slice(outer);
            h
        };
        let saved = self.carrier.take();
        for k in 0..seqs.len() {
            let hops = if k < s { outer.to_vec() } else { hops_from(self, k) };
            // A pseudo-element compound's state pseudo-classes sit on the subject: no carrier filter.
            let car = if k < s { None } else { subject_filter(&seqs[k].0) };
            self.carrier = car.clone();
            // A sibling combinator left of compound `k` makes the sibling list matter for the element it matches.
            if k >= s && k + 1 < seqs.len() {
                let key = match seqs[k].1 {
                    Some(Combinator::NextSibling) => Some(Key::SibNext),
                    Some(Combinator::LaterSibling) => Some(Key::SibLater),
                    _ => None,
                };
                if let Some(key) = key {
                    self.add(key, &hops, filter);
                }
            }
            for c in &seqs[k].0 {
                self.component(c, &hops, filter);
                // a nested selector list sets its own carriers
                self.carrier = car.clone();
            }
        }
        self.carrier = saved;
    }

    fn component(&mut self, c: &Component<OurSelectorImpl>, hops: &[Hop], filter: &Option<Rc<Filter>>) {
        match c {
            Component::Class(a) => self.add(Key::Class(a.0.clone()), hops, filter),
            Component::ID(a) => self.add(Key::Id(a.0.clone()), hops, filter),
            Component::AttributeInNoNamespaceExists { local_name_lower, .. } => {
                self.add(Key::Attr(local_name_lower.0.to_ascii_lowercase()), hops, filter)
            }
            Component::AttributeInNoNamespace { local_name, .. } => self.add(Key::Attr(local_name.0.to_ascii_lowercase()), hops, filter),
            Component::AttributeOther(a) => self.add(Key::Attr(a.local_name_lower.0.to_ascii_lowercase()), hops, filter),
            Component::Empty => self.add(Key::Empty, hops, filter),
            Component::Nth(d) => {
                let key = match (d.is_function, d.ty) {
                    (false, NthType::Child) => Key::First,
                    (false, NthType::LastChild) => Key::Last,
                    (false, NthType::OnlyChild) => Key::Only,
                    _ => Key::Nth,
                };
                self.add(key, hops, filter);
            }
            Component::NthOf(d) => {
                self.add(Key::Nth, hops, filter);
                // `S` is evaluated on the siblings (and the element itself): their features move this element's index.
                let mut sib = vec![Hop::AnySib];
                sib.extend_from_slice(hops);
                for s in d.selectors() {
                    self.nested(s, hops, filter);
                    self.nested(s, &sib, filter);
                }
            }
            Component::NonTSPseudoClass(pc) => {
                let kind = match pc {
                    OurPseudoClass::Link => Some(StateKind::Link),
                    OurPseudoClass::Hover => Some(StateKind::Hover),
                    OurPseudoClass::Active => Some(StateKind::Active),
                    OurPseudoClass::Focus => Some(StateKind::Focus),
                    OurPseudoClass::FocusVisible => Some(StateKind::FocusVisible),
                    OurPseudoClass::FocusWithin => Some(StateKind::FocusWithin),
                    OurPseudoClass::Target => Some(StateKind::Target),
                    OurPseudoClass::Checked => Some(StateKind::Checked),
                    OurPseudoClass::Indeterminate => Some(StateKind::Indeterminate),
                    OurPseudoClass::Disabled => Some(StateKind::Disabled),
                    OurPseudoClass::Enabled => Some(StateKind::Enabled),
                    OurPseudoClass::Lang(_) => Some(StateKind::Lang),
                    OurPseudoClass::Visited | OurPseudoClass::Root | OurPseudoClass::Dir(_) => None,
                };
                if let Some(k) = kind {
                    self.add(Key::State(k), hops, filter);
                }
            }
            Component::Negation(l) | Component::Is(l) | Component::Where(l) => {
                for s in l.slice() {
                    self.nested(s, hops, filter);
                }
            }
            Component::Has(list) => {
                if self.has_depth > 0 {
                    // not valid CSS (the parser rejects it); stay sound anyway
                    self.global = true;
                    return;
                }
                self.has = true;
                // `selector` set the carrier to the compound holding this `:has`: its anchors must match it.
                let car = self.carrier.take();
                let saved = std::mem::replace(&mut self.anchor, car.clone());
                for rs in list.iter() {
                    let first = if rs.match_hint.is_descendant_direction() { Hop::Anc } else { Hop::AncSib };
                    let mut h = vec![first];
                    h.extend_from_slice(hops);
                    // `HasTree` carrier: the relative selector's own subject compound (an inserted / removed
                    // subtree without an element that can match it changes nothing but adjacency)
                    let subj: Vec<&Component<OurSelectorImpl>> = rs.selector.iter().collect();
                    self.carrier = subject_filter(&subj);
                    self.add(Key::HasTree, &h, filter);
                    self.carrier = None;
                    self.has_depth = 1;
                    self.selector(&rs.selector, &h, filter);
                    self.has_depth = 0;
                }
                self.anchor = saved;
                self.carrier = car;
            }
            Component::Host(_) | Component::Slotted(_) | Component::Part(_) => self.global = true,
            _ => {}
        }
    }

    /// `content: attr(n)` of a rule reads `n` on the rule's subject elements.
    fn content_reads(&mut self, d: &Declarations, filter: &Option<Rc<Filter>>, all: &mut bool) {
        match &d.content {
            Some(ContentDecl::Parts(parts)) => {
                for p in parts {
                    if let ContentPart::Attr(n) = p {
                        self.add(Key::Attr(n.to_ascii_lowercase()), &[], filter);
                    }
                }
            }
            _ => {
                if d.var_decls.iter().any(|(n, v)| n.eq_ignore_ascii_case("content") && v.to_ascii_lowercase().contains("attr(")) {
                    *all = true;
                }
            }
        }
        d.late.iter().for_each(|l| self.content_reads(l, filter, all));
    }
}

impl InvalidationMap {
    pub(crate) fn build(rules: &[StyleRule], generation: u64) -> InvalidationMap {
        let mut b = Builder { carrier: None, keys: HashMap::new(), global: false, has_depth: 0, anchor: None, has: false };
        let mut attr_all = false;
        for rule in rules {
            for sel in rule.selectors.slice() {
                // The subject compound: skip a leading pseudo-element compound.
                let mut it = sel.iter();
                let mut comps: Vec<&Component<OurSelectorImpl>> = it.by_ref().collect();
                let mut next = it.next_sequence();
                while next == Some(Combinator::PseudoElement) {
                    comps = it.by_ref().collect();
                    next = it.next_sequence();
                }
                let filter = subject_filter(&comps);
                b.selector(sel, &[], &filter);
                if rule.may_declare_generated() {
                    b.carrier = None;
                    b.content_reads(rule.decls(), &filter, &mut attr_all);
                    b.content_reads(rule.important(), &filter, &mut attr_all);
                }
            }
        }
        // QAPI6b: the feature-agnostic `:has` paths (see `Key::HasAny`); an entry whose path reaches the anchor hop
        // through sibling hops first (`:nth-child(of S)` inside `:has`) makes the start set every sibling.
        let mut any: Vec<Entry> = Vec::new();
        let mut any_wide = false;
        for v in b.keys.values() {
            for e in v {
                let Some(i) = e.hops.iter().position(|h| matches!(h, Hop::Anc | Hop::AncSib)) else { continue };
                any_wide |= i > 0;
                any.push(Entry { hops: e.hops[i..].into(), filter: e.filter.clone(), carrier: None, anchor: e.anchor.clone() });
            }
        }
        if !any.is_empty() {
            b.keys.insert(Key::HasAny, any);
        }
        for v in b.keys.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
        let has_state_checked = b.keys.contains_key(&Key::State(StateKind::Checked));
        let has_state_disabled = b.keys.contains_key(&Key::State(StateKind::Disabled)) || b.keys.contains_key(&Key::State(StateKind::Enabled));
        InvalidationMap {
            generation,
            keys: b.keys,
            global: b.global,
            has: b.has,
            has_any_wide: any_wide,
            attr_all,
            has_state_checked,
            has_state_disabled,
            batch: RefCell::new(None),
        }
    }

    /// QAPI6b: anything in the subtree of `top` (an element nobody styles: under `display: none`, or inserted in
    /// this batch) changed in any way. Only `:has()` can carry that to styled elements: the anchors above `top`
    /// (and the earlier siblings of it and of its ancestors), for every `:has` rule, whatever the feature. A
    /// superset of what the change's own feature entries would reach from inside the subtree, answered once per
    /// `top` instead of once per change (a script filling a hidden list logs thousands).
    pub(crate) fn for_has_any(&self, top: &Handle) -> Vec<(Handle, Mark)> {
        let mut out = Marks::default();
        if !self.has {
            return out.out;
        }
        let starts: Vec<Handle> = match parent_handle(top) {
            Some(p) if self.has_any_wide => el_children(&p),
            _ => vec![top.clone()],
        };
        for s in &starts {
            self.apply(&Key::HasAny, s, false, &mut out);
        }
        out.out
    }

    /// QAPI6: the calls until `end_batch` answer one batch of changes against one DOM state (an engine update):
    /// a walk already done in the batch is not repeated (`:has(..) .x` reaches whole subtrees from the anchors).
    pub(crate) fn begin_batch(&self) {
        *self.batch.borrow_mut() = Some(WalkMemo::default());
    }

    pub(crate) fn end_batch(&self) {
        *self.batch.borrow_mut() = None;
    }

    /// Some selector is not bounded (`:host`, ...): every call returns a single `Mark::Global`.
    pub(crate) fn is_global(&self) -> bool {
        self.global
    }

    /// QAPI6: some selector uses `:has()`: a change of an element nobody styles (under `display: none`) can
    /// still change a rendered anchor's match.
    pub(crate) fn has_has(&self) -> bool {
        self.has
    }

    pub(crate) fn entry_count(&self) -> usize {
        self.keys.values().map(Vec::len).sum()
    }

    /// `filter_self`: the change cannot have altered `from`'s own tag / id / classes, so the entries'
    /// carrier filters (and, for an entry without hops, the subject filter) are checked on it.
    fn apply(&self, key: &Key, from: &Handle, filter_self: bool, out: &mut Marks) {
        self.apply_if(key, from, filter_self, &|_| true, out);
    }

    /// `apply` over the entries `keep` accepts.
    fn apply_if(&self, key: &Key, from: &Handle, filter_self: bool, keep: &dyn Fn(&Entry) -> bool, out: &mut Marks) {
        let Some(entries) = self.keys.get(key) else { return };
        // Entries with the same hop path are walked once, keeping what passes any of their subject
        // filters (`None` in the group = no filter): a `:hover` on an ancestor walks its subtree once,
        // not once per rule.
        let mut groups: Vec<HopGroup> = Vec::new();
        let mut anchors: Vec<&Option<Rc<Filter>>> = Vec::new();
        let mut probe: Option<Probe> = None;
        for e in entries.iter().filter(|e| keep(e)) {
            if filter_self && (e.carrier.is_some() || (e.hops.is_empty() && e.filter.is_some())) {
                let pr = probe.get_or_insert_with(|| Probe::of(from));
                if !pr.passes(&e.carrier) {
                    continue;
                }
            }
            if e.hops.is_empty() {
                if !filter_self || probe.as_ref().is_none_or(|pr| pr.passes(&e.filter)) {
                    out.add(from, Mark::Restyle);
                }
                continue;
            }
            let g = match groups.iter().zip(&anchors).position(|(g, a)| *g.0 == *e.hops && **a == e.anchor) {
                Some(i) => &mut groups[i],
                None => {
                    groups.push((&e.hops, Some(Vec::new())));
                    anchors.push(&e.anchor);
                    groups.last_mut().expect("just pushed")
                }
            };
            match (&e.filter, &mut g.1) {
                (Some(f), Some(v)) => v.push(f),
                (None, f) => *f = None,
                (Some(_), None) => {}
            }
        }
        // QAPI6: the `:has` groups of one change share their first hop (the carrier's ancestors / their earlier
        // siblings): computed once, each group only filters it by its anchor compound.
        let mut firsts: [Option<Vec<(Handle, Probe)>>; 2] = [None, None];
        for ((hops, filters), anchor) in groups.into_iter().zip(anchors) {
            let filters = filters.unwrap_or_default();
            match hops.split_first() {
                Some((&first @ (Hop::Anc | Hop::AncSib), rest)) => {
                    let slot = usize::from(first != Hop::Anc);
                    let start = firsts[slot].get_or_insert_with(|| {
                        step(std::slice::from_ref(from), first, &[]).into_iter().map(|h| {
                            let p = Probe::of(&h);
                            (h, p)
                        }).collect()
                    });
                    let mut cur: Vec<Handle> = start
                        .iter()
                        .filter(|(h, p)| p.passes(anchor) && anchor.as_ref().is_none_or(|f| has_attrs(h, f)))
                        .map(|(h, _)| h.clone())
                        .collect();
                    if rest.is_empty() {
                        cur.retain(|h| passes_any(h, &filters));
                        for h in &cur {
                            out.add(h, Mark::Restyle);
                        }
                    } else {
                        walk(cur, rest, &filters, &None, Some(&self.batch), out);
                    }
                }
                _ => walk(vec![from.clone()], hops, &filters, anchor, Some(&self.batch), out),
            }
        }
    }

    /// QAPI6: a child of `parent` at child-node index `index` was removed: the `HasTree` paths from the
    /// anchors the removed subtree may have been found by.
    /// `node`: the removed subtree when known (its tags / ids / classes prune the entries).
    fn has_removed(&self, parent: &Handle, index: usize, node: Option<&Handle>, out: &mut Marks) {
        let Some(entries) = self.keys.get(&Key::HasTree) else { return };
        let sum = node.map(TreeSummary::of);
        // grouped like `apply`: (first hop, rest of the path, anchor filter) -> subject filters
        type Group<'a> = ((Hop, &'a [Hop], &'a Option<Rc<Filter>>), Option<Vec<&'a Rc<Filter>>>);
        let mut groups: Vec<Group> = Vec::new();
        for e in entries {
            if sum.as_ref().is_some_and(|s| !s.may_hold(&e.carrier)) {
                continue;
            }
            let Some((&first, rest)) = e.hops.split_first() else { continue };
            let key = (first, rest, &e.anchor);
            let g = match groups.iter().position(|g| g.0 == key) {
                Some(i) => &mut groups[i],
                None => {
                    groups.push((key, Some(Vec::new())));
                    groups.last_mut().expect("just pushed")
                }
            };
            match (&e.filter, &mut g.1) {
                (Some(f), Some(v)) => v.push(f),
                (None, f) => *f = None,
                (Some(_), None) => {}
            }
        }
        let mut starts: [Option<Vec<Handle>>; 2] = [None, None];
        for ((first, rest, anchor), filters) in groups {
            let filters = filters.unwrap_or_default();
            let slot = usize::from(first != Hop::Anc);
            let start = starts[slot].get_or_insert_with(|| removal_anchors(parent, index, first));
            let mut cur: Vec<Handle> = start.iter().filter(|h| passes_reached(h, anchor)).cloned().collect();
            if rest.is_empty() {
                cur.retain(|h| passes_any(h, &filters));
                for h in &cur {
                    out.add(h, Mark::Restyle);
                }
            } else {
                walk(cur, rest, &filters, &None, Some(&self.batch), out);
            }
        }
    }

    fn global_marks(&self, el: &Handle) -> Vec<(Handle, Mark)> {
        vec![(el.clone(), Mark::Global)]
    }

    /// Attribute `name` of `el` went from `old` to `new` (`None` = absent). Call after the write.
    pub(crate) fn for_attr(&self, el: &Handle, name: &str, old: Option<&str>, new: Option<&str>) -> Vec<(Handle, Mark)> {
        let name = name.to_ascii_lowercase();
        if old == new {
            return Vec::new();
        }
        if self.global {
            return self.global_marks(el);
        }
        let mut out = Marks::default();
        if name == "style" {
            out.add(el, Mark::Recascade);
        }
        if self.attr_all {
            out.add(el, Mark::Restyle);
        }
        // Own class / id keys change with those writes: the subject filter of a self entry must not apply.
        let own_key = name == "class" || name == "id";
        self.apply(&Key::Attr(name.clone()), el, !own_key, &mut out);
        match name.as_str() {
            "class" => {
                let (o, n) = (tokens(old), tokens(new));
                for t in o.symmetric_difference(&n) {
                    self.apply(&Key::Class((*t).to_string()), el, false, &mut out);
                }
            }
            "id" => {
                for v in [old, new].into_iter().flatten() {
                    self.apply(&Key::Id(v.to_string()), el, false, &mut out);
                }
            }
            "href" => self.apply(&Key::State(StateKind::Link), el, true, &mut out),
            "disabled" => self.disabled_carriers(el, &mut out),
            "checked" | "selected" | "type" | "multiple" | "value" => {
                self.apply(&Key::State(StateKind::Checked), el, true, &mut out);
                self.apply(&Key::State(StateKind::Indeterminate), el, true, &mut out);
                self.option_peers(el, &mut out);
            }
            "indeterminate" => self.apply(&Key::State(StateKind::Indeterminate), el, true, &mut out),
            "lang" => {
                self.apply(&Key::State(StateKind::Lang), el, true, &mut out);
                for d in step(std::slice::from_ref(el), Hop::Desc, &[]) {
                    self.apply(&Key::State(StateKind::Lang), &d, true, &mut out);
                }
            }
            _ => {}
        }
        out.out
    }

    /// `disabled` on `el` (or a child change of a `<fieldset>`): `:disabled`/`:enabled` of `el`, and of every form
    /// control below it when it is a fieldset.
    fn disabled_carriers(&self, el: &Handle, out: &mut Marks) {
        if !self.has_state_disabled {
            return;
        }
        let mut carriers = vec![el.clone()];
        if element_tag(el) == "fieldset" {
            carriers.extend(step(std::slice::from_ref(el), Hop::Desc, &[]).into_iter().filter(is_form_control));
        }
        for c in &carriers {
            self.apply(&Key::State(StateKind::Disabled), c, true, out);
            self.apply(&Key::State(StateKind::Enabled), c, true, out);
        }
    }

    /// `:checked` of the options of the same `<select>` as `el` (single selection / `multiple` changes).
    fn option_peers(&self, el: &Handle, out: &mut Marks) {
        if !self.has_state_checked {
            return;
        }
        let tag = element_tag(el);
        let mut select = None;
        if matches!(tag, "select" | "optgroup") {
            select = Some(el.clone());
        } else if tag == "option" {
            let mut cur = parent_handle(el);
            while let Some(p) = cur {
                if element_tag(&p) == "select" {
                    select = Some(p);
                    break;
                }
                cur = parent_handle(&p);
            }
        }
        if let Some(s) = select {
            for o in step(&[s], Hop::Desc, &[]).into_iter().filter(|o| element_tag(o) == "option") {
                self.apply(&Key::State(StateKind::Checked), &o, true, out);
            }
        }
    }

    /// `el` entered or left state `kind` (hover/active/focus chains: call once per element entering or leaving).
    /// `Checked`: also the radios of the same name and the options of the same select (selecting one deselects the
    /// others).
    pub(crate) fn for_state(&self, el: &Handle, kind: StateKind) -> Vec<(Handle, Mark)> {
        if self.global {
            return self.global_marks(el);
        }
        let mut out = Marks::default();
        self.apply(&Key::State(kind), el, true, &mut out);
        if kind == StateKind::Checked {
            self.option_peers(el, &mut out);
            if element_tag(el) == "input"
                && let Some(name) = attr_str(el, "name")
            {
                let mut top = el.clone();
                while let Some(p) = parent_handle(&top) {
                    top = p;
                }
                let mut peers = step(&[top], Hop::Desc, &[]);
                peers.retain(|p| element_tag(p) == "input" && attr_str(p, "name").as_deref() == Some(name.as_str()));
                for p in &peers {
                    self.apply(&Key::State(StateKind::Checked), p, true, &mut out);
                    self.apply(&Key::State(StateKind::Indeterminate), p, true, &mut out);
                }
            }
        }
        out.out
    }

    /// Class write convenience: `for_attr(el, "class", ..)`.
    pub(crate) fn for_class(&self, el: &Handle, old: Option<&str>, new: Option<&str>) -> Vec<(Handle, Mark)> {
        self.for_attr(el, "class", old, new)
    }

    /// QAPI6: a child of `parent` that may have been an element (the caller cannot tell) left child-node index
    /// `index`: the `:has()` anchors that may have found it.
    pub(crate) fn for_has_removal(&self, parent: &Handle, index: usize) -> Vec<(Handle, Mark)> {
        let mut out = Marks::default();
        if self.has && is_el(parent) {
            self.has_removed(parent, index, None, &mut out);
        }
        out.out
    }

    /// A text node of `parent` was inserted, removed or edited (`:empty` is the only selector that sees it).
    pub(crate) fn for_text(&self, parent: &Handle) -> Vec<(Handle, Mark)> {
        if self.global {
            return self.global_marks(parent);
        }
        let mut out = Marks::default();
        if is_el(parent) {
            self.apply(&Key::Empty, parent, true, &mut out);
        }
        out.out
    }

    /// A child of `parent` was inserted or removed. Includes the new subtree (every element `Restyle`).
    pub(crate) fn for_children(&self, parent: &Handle, change: &ChildChange) -> Vec<(Handle, Mark)> {
        if self.global {
            return self.global_marks(parent);
        }
        let mut out = Marks::default();
        let (node, from) = match change {
            ChildChange::Inserted(n) => {
                let pos = parent.children.borrow().iter().position(|c| Rc::ptr_eq(c, n));
                (n, pos.map_or(0, |p| p + 1))
            }
            ChildChange::Removed { node, index } => (node, *index),
        };
        if !is_el(parent) {
            return out.out;
        }
        if let ChildChange::Inserted(n) = change
            && is_el(n)
        {
            out.add(n, Mark::Restyle);
            for d in step(std::slice::from_ref(n), Hop::Desc, &[]) {
                out.add(&d, Mark::Restyle);
            }
            // QAPI6: the anchors that may now find the new subtree (only through an element in it that can
            // match a relative selector's subject compound; adjacency changes go through SibNext / SibLater)
            if self.has {
                let sum = TreeSummary::of(n);
                self.apply_if(&Key::HasTree, n, false, &|e| sum.may_hold(&e.carrier), &mut out);
            }
        }
        if let ChildChange::Removed { node, index } = change
            && is_el(node)
            && self.has
        {
            // QAPI6b: never pruned by the removed subtree's content: the engine answers a batch against the FINAL
            // DOM, and a detached subtree can be changed after its removal without a log entry (a removed card
            // that lost its `.on` class still was found by the anchors before the batch).
            self.has_removed(parent, *index, None, &mut out);
        }
        // `:empty` sees text too; nothing else does.
        self.apply(&Key::Empty, parent, true, &mut out);
        if !is_el(node) {
            return out.out;
        }
        if element_tag(parent) == "fieldset" {
            // The first legend is exempt from a disabled fieldset: any child change can move it.
            self.disabled_carriers(parent, &mut out);
        }
        if matches!(element_tag(parent), "select" | "optgroup") {
            self.option_peers(parent, &mut out);
        }
        let els = el_children(parent);
        let after: Vec<Handle> = parent.children.borrow().get(from..).unwrap_or(&[]).iter().filter(|c| is_el(c)).cloned().collect();
        let firsts: Vec<Handle> = els.iter().take(2).cloned().collect();
        let lasts: Vec<Handle> = els.iter().rev().take(2).cloned().collect();
        for h in &firsts {
            self.apply(&Key::First, h, true, &mut out);
            self.apply(&Key::Only, h, true, &mut out);
        }
        for h in &lasts {
            self.apply(&Key::Last, h, true, &mut out);
            self.apply(&Key::Only, h, true, &mut out);
        }
        if self.keys.contains_key(&Key::Nth) {
            for h in &els {
                self.apply(&Key::Nth, h, true, &mut out);
            }
        }
        if let Some(n) = after.first() {
            self.apply(&Key::SibNext, n, true, &mut out);
        }
        for h in &after {
            self.apply(&Key::SibLater, h, true, &mut out);
        }
        out.out
    }

    /// QINC1f-fix: the element children of `parent` went from `old` (their keys, `node_id`, at the last restyle) to
    /// the current list, through any number of inserts, removes and moves. [`Self::for_children`] answers ONE change
    /// against the DOM it sees; a batch of changes replayed against the final DOM misses carriers (two inserts at the
    /// front leave the old first child third, outside the "first two" edge; a removal whose logged siblings moved too
    /// loses its index). This compares the two lists directly: the old and new first / last element (`:first-child`,
    /// `:last-child`, `:only-child`), every element whose previous element sibling changed (`+`), every element from
    /// the first difference on (`~`: an element in the common prefix has the same earlier siblings), every element
    /// (`:nth-*`). It does not mark the inserted subtrees (`for_children` does).
    pub(crate) fn for_child_list(&self, parent: &Handle, old: &[usize]) -> Vec<(Handle, Mark)> {
        if self.global {
            return self.global_marks(parent);
        }
        let mut out = Marks::default();
        if !is_el(parent) {
            return out.out;
        }
        let els = el_children(parent);
        let now: Vec<usize> = els.iter().map(node_id).collect();
        if now.as_slice() == old {
            return out.out;
        }
        self.apply(&Key::Empty, parent, true, &mut out);
        let at = |k: Option<&usize>| k.and_then(|k| now.iter().position(|n| n == k)).map(|i| &els[i]);
        // edges: the old and the new first / last (an old one that left needs nothing)
        let mut firsts: Vec<&Handle> = Vec::new();
        let mut lasts: Vec<&Handle> = Vec::new();
        if old.first() != now.first() {
            firsts.extend(els.first());
            firsts.extend(at(old.first()));
        }
        if old.last() != now.last() {
            lasts.extend(els.last());
            lasts.extend(at(old.last()));
        }
        for h in &firsts {
            self.apply(&Key::First, h, true, &mut out);
        }
        for h in &lasts {
            self.apply(&Key::Last, h, true, &mut out);
        }
        // `:only-child` = first and last: it flips only where one of them does
        for h in firsts.iter().chain(&lasts) {
            self.apply(&Key::Only, h, true, &mut out);
        }
        if self.keys.contains_key(&Key::Nth) {
            for h in &els {
                self.apply(&Key::Nth, h, true, &mut out);
            }
        }
        let old_prev: HashMap<usize, Option<usize>> = old.iter().enumerate().map(|(i, k)| (*k, i.checked_sub(1).map(|j| old[j]))).collect();
        let pre = now.iter().zip(old).take_while(|(a, b)| a == b).count();
        for (i, h) in els.iter().enumerate().skip(pre) {
            let prev = i.checked_sub(1).map(|j| now[j]);
            if old_prev.get(&now[i]) != Some(&prev) {
                self.apply(&Key::SibNext, h, true, &mut out);
            }
            self.apply(&Key::SibLater, h, true, &mut out);
        }
        out.out
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<InvalidationMap>>> = const { RefCell::new(None) };
}

/// The map of the rule list `rules_cache` holds now (built once per generation); `None` before any build.
pub(crate) fn current() -> Option<Rc<InvalidationMap>> {
    let (rules, generation, _, _) = crate::rules_cache::current()?;
    CURRENT.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(m) = c.as_ref()
            && m.generation == generation
        {
            return Some(m.clone());
        }
        let m = Rc::new(InvalidationMap::build(&rules, generation));
        crate::incr_log::set_keep_nodes(m.has_has());
        *c = Some(m.clone());
        Some(m)
    })
}
