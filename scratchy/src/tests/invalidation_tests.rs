//! QINC1d: the invalidation map (R60 section 5): one test per selector shape, then the soundness property test.
use super::*;
use crate::invalidation::{ChildChange, InvalidationMap, Mark, StateKind};

fn all_elements(doc: &Handle) -> Vec<Handle> {
    let mut out = Vec::new();
    let mut stack = vec![doc.clone()];
    while let Some(n) = stack.pop() {
        for c in n.children.borrow().iter().rev() {
            stack.push(c.clone());
        }
        if matches!(n.data, NodeData::Element { .. }) {
            out.push(n);
        }
    }
    out
}

fn by_id(doc: &Handle, id: &str) -> Handle {
    all_elements(doc).into_iter().find(|e| element_attr(e, "id").is_some_and(|v| *v == *id)).unwrap_or_else(|| panic!("no #{id}"))
}

fn set_attr(h: &Handle, name: &str, val: Option<&str>) {
    if let NodeData::Element { attrs, .. } = &h.data {
        let mut a = attrs.borrow_mut();
        a.retain(|x| &*x.name.local != name);
        if let Some(v) = val {
            a.push(html5ever::Attribute { name: html5ever::QualName::new(None, html5ever::ns!(), html5ever::LocalName::from(name)), value: v.into() });
        }
    }
}

fn detach(h: &Handle) -> usize {
    let p = parent_handle(h).unwrap();
    let mut ch = p.children.borrow_mut();
    let i = ch.iter().position(|c| Rc::ptr_eq(c, h)).unwrap();
    ch.remove(i);
    h.parent.set(None);
    i
}

fn attach(parent: &Handle, child: &Handle, index: usize) {
    parent.children.borrow_mut().insert(index, child.clone());
    child.parent.set(Some(Rc::downgrade(parent)));
}

fn map(css: &str) -> InvalidationMap {
    InvalidationMap::build(&parse_stylesheet(css), 1)
}

fn ids(doc: &Handle, v: &[(Handle, Mark)]) -> Vec<String> {
    let mut o: Vec<String> = v.iter().map(|(h, _)| element_attr(h, "id").map(|s| s.to_string()).unwrap_or_else(|| format!("<{}>", element_tag(h)))).collect();
    let _ = doc;
    o.sort();
    o
}

fn set_class(h: &Handle, new: Option<&str>) -> Option<String> {
    let old = element_attr(h, "class").map(|v| v.to_string());
    set_attr(h, "class", new);
    old
}

const PAGE: &str = "<body><div id=p class=a><span id=c1 class=b><i id=g class=b></i></span><span id=c2 class=b></span><em id=e></em></div>\
<div id=q><span id=q1 class=b></span></div></body>";

#[test]
fn shape_descendant() {
    let m = map(".a .b{color:red}");
    let d = parse_html(PAGE);
    let p = by_id(&d, "p");
    set_class(&p, Some("x"));
    // `.a` removed from #p: every `.b` below it, not the one under #q nor #p itself.
    assert_eq!(ids(&d, &m.for_class(&p, Some("a"), Some("x"))), ["c1", "c2", "g"]);
}

#[test]
fn shape_child() {
    let m = map(".a > .b{color:red}");
    let d = parse_html(PAGE);
    let p = by_id(&d, "p");
    assert_eq!(ids(&d, &m.for_class(&p, Some("a"), None)), ["c1", "c2"]);
}

#[test]
fn shape_adjacent() {
    let m = map(".a + .b{color:red}");
    let d = parse_html(PAGE);
    let c1 = by_id(&d, "c1");
    assert_eq!(ids(&d, &m.for_class(&c1, Some("a"), None)), ["c2"]);
}

#[test]
fn shape_general_sibling_then_descendant() {
    let m = map(".a ~ .b .c{color:red}");
    let d = parse_html("<body><u id=s class=a></u><u id=t class=b><i id=x class=c></i></u><u id=w class=b><i id=y class=c></i><i id=z></i></u></body>");
    let s = by_id(&d, "s");
    assert_eq!(ids(&d, &m.for_class(&s, Some("a"), None)), ["x", "y"]);
}

#[test]
fn shape_attribute() {
    let m = map("[x] .b{color:red}");
    let d = parse_html(PAGE);
    let p = by_id(&d, "p");
    assert_eq!(ids(&d, &m.for_attr(&p, "x", None, Some("1"))), ["c1", "c2", "g"]);
    assert!(m.for_attr(&p, "y", None, Some("1")).is_empty());
}

#[test]
fn shape_hover_and_focus_within() {
    let m = map(":hover .b{color:red} #p:focus-within{color:red}");
    let d = parse_html(PAGE);
    let q = by_id(&d, "q");
    assert_eq!(ids(&d, &m.for_state(&q, StateKind::Hover)), ["q1"]);
    let p = by_id(&d, "p");
    assert_eq!(ids(&d, &m.for_state(&p, StateKind::FocusWithin)), ["p"]);
    // `#p:focus-within` keys on #p: another element entering the state reaches nothing.
    assert!(m.for_state(&q, StateKind::FocusWithin).is_empty());
}

/// QINC1j-ab: the compound carrying `:hover` is checked on the element entering the state before its
/// descendants are walked: `:hover` on an ancestor without `.menu` marks nothing for `.menu:hover .icon`,
/// while a key-less `:hover .icon` still reaches the icons below it.
#[test]
fn hover_on_ancestor_filters_by_carrier_compound() {
    let page = "<body><div id=app><div id=menu class=menu><i id=i1 class=icon></i></div><span id=s><i id=i2 class=icon></i></span></div></body>";
    let d = parse_html(page);
    let app = by_id(&d, "app");
    let menu = by_id(&d, "menu");
    // only the `.menu` compound reads `:hover`
    let m = map(".menu:hover .icon{color:red} div.menu:hover > i{color:blue} #menu:hover i{color:green}");
    assert!(m.for_state(&app, StateKind::Hover).is_empty(), "#app has no .menu: no descendant can change");
    assert_eq!(ids(&d, &m.for_state(&menu, StateKind::Hover)), ["i1"]);
    // a descendant-dependent rule whose hover compound has no tag / id / class: the subtree is walked
    let m = map(".menu:hover .icon{color:red} :hover > .icon{color:blue}");
    assert_eq!(ids(&d, &m.for_state(&app, StateKind::Hover)), Vec::<String>::new(), "`:hover > .icon`: #app has no .icon child");
    let s = by_id(&d, "s");
    assert_eq!(ids(&d, &m.for_state(&s, StateKind::Hover)), ["i2"]);
    let m = map("[data-x]:hover .icon{color:red} :hover .icon{color:blue}");
    assert_eq!(ids(&d, &m.for_state(&app, StateKind::Hover)), ["i1", "i2"]);
    // an attribute write on the carrier is filtered by its classes, a class write is not
    let m = map(".menu[x] .icon{color:red}");
    assert!(m.for_attr(&app, "x", None, Some("1")).is_empty());
    assert_eq!(ids(&d, &m.for_attr(&menu, "x", None, Some("1"))), ["i1"]);
    let old = set_class(&app, Some("menu"));
    assert_eq!(ids(&d, &m.for_class(&app, old.as_deref(), Some("menu"))), ["i1", "i2"]);
}

/// QINC1j-ab: entries sharing a hop path are walked once and keep what passes any of their subject
/// filters; the result equals the union of the entries' own walks.
#[test]
fn grouped_walk_is_union_of_entries() {
    let page = "<body><div id=app><p id=p1 class=a></p><p id=p2 class=b></p><span id=s1 class=a></span><em id=e1></em></div></body>";
    let d = parse_html(page);
    let app = by_id(&d, "app");
    let m = map(":hover p.a{color:red} :hover .b{color:red} :hover span{color:red}");
    assert_eq!(ids(&d, &m.for_state(&app, StateKind::Hover)), ["p1", "p2", "s1"]);
    let m = map(":hover p.a{color:red} :hover *{color:red}");
    assert_eq!(ids(&d, &m.for_state(&app, StateKind::Hover)), ["e1", "p1", "p2", "s1"]);
}

#[test]
fn shape_structural() {
    let m = map("li:nth-child(2n){color:red}");
    let d = parse_html("<body><ul id=u><li id=l1></li><li id=l2></li><li id=l3></li></ul></body>");
    let u = by_id(&d, "u");
    let n = parse_html("<body><li id=l0></li></body>");
    let li0 = by_id(&n, "l0");
    detach(&li0);
    attach(&u, &li0, 0);
    let r = ids(&d, &m.for_children(&u, &ChildChange::Inserted(li0)));
    assert_eq!(r, ["l0", "l1", "l2", "l3"]);
}

/// QINC1f-fix: `for_child_list` compares the old and new element lists, so a batch of changes cannot hide an edge.
#[test]
fn child_list_marks_old_and_new_edges_and_changed_siblings() {
    let d = parse_html("<body><ul id=u><li id=l1></li><li id=l2></li><li id=l3></li></ul></body>");
    let u = by_id(&d, "u");
    let old: Vec<usize> = ["l1", "l2", "l3"].iter().map(|i| node_id(&by_id(&d, i))).collect();
    let n = parse_html("<body><li id=x></li><li id=y></li></body>");
    let (x, y) = (by_id(&n, "x"), by_id(&n, "y"));
    detach(&x);
    detach(&y);
    attach(&u, &x, 0);
    attach(&u, &y, 1); // x y l1 l2 l3: the old first is third
    // `for_children` on each insert, against this final DOM, never reaches l1
    let m = map("li:first-child{color:red}");
    let mut per: Vec<String> = ids(&d, &m.for_children(&u, &ChildChange::Inserted(x.clone())));
    per.extend(ids(&d, &m.for_children(&u, &ChildChange::Inserted(y.clone()))));
    assert!(!per.iter().any(|i| i == "l1"), "{per:?}");
    assert_eq!(ids(&d, &m.for_child_list(&u, &old)), ["l1", "x"]);
    assert_eq!(ids(&d, &map("li:last-child{color:red}").for_child_list(&u, &old)), Vec::<String>::new(), "last unchanged");
    assert_eq!(ids(&d, &map("li:only-child{color:red}").for_child_list(&u, &old)), ["l1", "x"]);
    // `+`: x (new), y (new) and l1 (prev changed); l2/l3 keep their previous sibling
    assert_eq!(ids(&d, &map("li + li{color:red}").for_child_list(&u, &old)), ["l1", "x", "y"]);
    // `~`: everything from the first difference
    assert_eq!(ids(&d, &map("li ~ li{color:red}").for_child_list(&u, &old)), ["l1", "l2", "l3", "x", "y"]);
    assert_eq!(ids(&d, &map("li:nth-child(2n){color:red}").for_child_list(&u, &old)), ["l1", "l2", "l3", "x", "y"]);
    // unchanged list: nothing
    let now: Vec<usize> = ["x", "y", "l1", "l2", "l3"].iter().map(|i| node_id(&by_id(&d, i))).collect();
    assert!(m.for_child_list(&u, &now).is_empty());
    // removal at the end: the new last (l2) and nothing in the common prefix
    detach(&by_id(&d, "l3"));
    let r = ids(&d, &map("li:last-child{color:red} li + li{color:blue} li ~ li{color:green}").for_child_list(&u, &now));
    assert_eq!(r, ["l2"]);
}

#[test]
fn shape_empty() {
    let m = map(":empty{color:red}");
    let d = parse_html("<body><div id=a></div></body>");
    let a = by_id(&d, "a");
    assert_eq!(ids(&d, &m.for_text(&a)), ["a"]);
}

#[test]
fn shape_not_and_is() {
    let m = map(":not(.a) .b{color:red} :is(.a,.q) em{color:red}");
    let d = parse_html(PAGE);
    let p = by_id(&d, "p");
    let r = ids(&d, &m.for_class(&p, Some("a"), None));
    for want in ["c1", "c2", "g", "e"] {
        assert!(r.contains(&want.to_string()), "{want} in {r:?}");
    }
    assert!(!r.contains(&"q1".to_string()));
}

#[test]
fn shape_has_is_global() {
    let m = map(".a:has(.b){color:red}");
    let d = parse_html(PAGE);
    if m.is_global() {
        let r = m.for_class(&by_id(&d, "p"), Some("a"), None);
        assert_eq!(r.len(), 1);
        assert!(r[0].1 == Mark::Global);
    } else {
        // The parser rejects `:has()`: the rule does not exist, so nothing can depend on it.
        assert_eq!(m.entry_count(), 0);
    }
}

#[test]
fn style_attr_is_recascade() {
    let m = map(".a{color:red}");
    let d = parse_html(PAGE);
    let p = by_id(&d, "p");
    let r = m.for_attr(&p, "style", None, Some("color:blue"));
    assert_eq!(r.len(), 1);
    assert!(r[0].1 == Mark::Recascade);
}

// ---------- soundness property test ----------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v[self.below(v.len())]
    }
}

const CLASSES: [&str; 4] = ["a", "b", "c", "d"];
const TAGS: [&str; 6] = ["div", "span", "p", "li", "a", "fieldset"];

fn rand_class(r: &mut Rng) -> Option<String> {
    let n = r.below(3);
    if n == 0 {
        return None;
    }
    Some((0..n).map(|_| r.pick(&CLASSES)).collect::<Vec<_>>().join(" "))
}

fn rand_html(r: &mut Rng, depth: usize, ids: &mut usize) -> String {
    let tag = r.pick(&TAGS);
    let mut s = format!("<{tag}");
    if let Some(c) = rand_class(r) {
        s += &format!(" class=\"{c}\"");
    }
    if r.below(5) == 0 {
        *ids += 1;
        s += &format!(" id=i{}", *ids % 3 + 1);
    }
    if r.below(6) == 0 {
        s += " x=1";
    }
    if tag == "a" && r.below(2) == 0 {
        s += " href=\"/\"";
    }
    s += ">";
    if depth < 4 {
        for _ in 0..r.below(4) {
            match r.below(6) {
                0 => s += "t",
                1 => {
                    let ty = if r.below(2) == 0 { "checkbox" } else { "text" };
                    let extra = if r.below(3) == 0 { " checked" } else { "" };
                    s += &format!("<input type={ty}{extra}>");
                }
                _ => s += &rand_html(r, depth + 1, ids),
            }
        }
    }
    s += &format!("</{tag}>");
    s
}

const SHAPES: [&str; 48] = [
    ".K", "#i1", "[x]", "[x=1]", ".K .K", ".K > .K", ".K + .K", ".K ~ .K", ".K ~ .K .K", "[x] .K", ":hover .K", ".K:hover",
    "#i1:focus-within", "li:nth-child(2n)", ":empty", "p:empty", ":not(.K) .K", ":is(.K,.K) .K", ":where(.K .K) span",
    "div:first-child", "div:last-child > .K", "li:only-child", "li:nth-of-type(2)", ".K:checked", "input:disabled",
    ":disabled + .K", "a:link .K", ".K[x] + div > span", ":nth-child(odd) .K", "p:nth-last-child(2)", "div .K:not(.K)",
    "[class*=a]", "input[type=checkbox]:checked ~ .K", "fieldset:disabled input", ":lang(en) .K", ".K:focus", ".K:active .K",
    "span::before", ".K::after", ":nth-child(2 of .K)",
    // QINC1j-ab: carrier filters (the compound holding the state / attribute is keyed by tag, id, class)
    "div.K:hover .K", "#i1:hover > span", "li.K:active ~ .K", ".K:hover + .K .K", "span.K[x] .K", ":is(.K:hover) span",
    ".K:not(:hover) .K", "p.K:focus-within .K",
];

fn rand_css(r: &mut Rng) -> String {
    let mut css = String::new();
    for _ in 0..(6 + r.below(10)) {
        let mut sel = r.pick(&SHAPES).to_string();
        while let Some(i) = sel.find('K') {
            sel.replace_range(i..i + 1, r.pick(&CLASSES));
        }
        css += &format!("{sel}{{color:red}}\n");
    }
    css
}

type MatchSets = HashMap<usize, Vec<bool>>;

fn match_sets(doc: &Handle, rules: &[StyleRule]) -> MatchSets {
    let mut caches = SelectorCaches::default();
    let mut out = HashMap::new();
    for e in all_elements(doc) {
        let el = crate::dom::El(e.clone());
        let mut bits = Vec::new();
        for rule in rules {
            for sel in rule.selectors.slice() {
                let mode = if sel.pseudo_element().is_some() { MatchingMode::ForStatelessPseudoElement } else { MatchingMode::Normal };
                let mut ctx = MatchingContext::new(mode, None, &mut caches, QuirksMode::NoQuirks, NeedsSelectorFlags::No, MatchingForInvalidation::No);
                bits.push(matches_selector(sel, 0, None, &el, &mut ctx));
            }
        }
        out.insert(node_id(&e), bits);
    }
    out
}

fn chain(h: &Handle) -> Vec<Handle> {
    let mut v = vec![h.clone()];
    let mut c = h.clone();
    while let Some(p) = parent_handle(&c) {
        if matches!(p.data, NodeData::Element { .. }) {
            v.push(p.clone());
        }
        c = p;
    }
    v
}

#[derive(Default)]
struct Tally {
    steps: usize,
    changed: usize,
    predicted: usize,
    predicted_on_changed_steps: usize,
    global_steps: usize,
}

/// One fuzz case; returns the tally and panics on an unsound prediction.
fn fuzz_case(seed: u64, steps: usize, t: &mut Tally) {
    let mut r = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut n = 0;
    let html = format!("<body>{}{}</body>", rand_html(&mut r, 0, &mut n), rand_html(&mut r, 0, &mut n));
    let css = rand_css(&mut r);
    let doc = parse_html(&html);
    let rules = parse_stylesheet(&css);
    let m = InvalidationMap::build(&rules, 1);
    let mut st = crate::dom::InteractionState::default();
    set_interaction(&st);
    for step in 0..steps {
        let els = all_elements(&doc);
        let body = els.iter().find(|e| element_tag(e) == "body").unwrap().clone();
        let movable: Vec<Handle> = els.iter().filter(|e| !matches!(element_tag(e), "html" | "head" | "body")).cloned().collect();
        if movable.is_empty() {
            return;
        }
        let target = movable[r.below(movable.len())].clone();
        let before = match_sets(&doc, &rules);
        let kind = r.below(16);
        let mut marks: Vec<(Handle, Mark)> = Vec::new();
        let mut desc: String;
        match kind {
            0..=2 => {
                let old = element_attr(&target, "class").map(|v| v.to_string());
                let new = match kind {
                    0 => Some(format!("{} {}", old.clone().unwrap_or_default(), r.pick(&CLASSES))),
                    1 => old.as_ref().map(|o| o.split(' ').skip(1).collect::<Vec<_>>().join(" ")),
                    _ => rand_class(&mut r),
                };
                set_attr(&target, "class", new.as_deref());
                marks = m.for_class(&target, old.as_deref(), new.as_deref());
                desc = format!("class {old:?} -> {new:?}");
            }
            3 => {
                let old = element_attr(&target, "id").map(|v| v.to_string());
                let new = if r.below(3) == 0 { None } else { Some(format!("i{}", r.below(3) + 1)) };
                set_attr(&target, "id", new.as_deref());
                marks = m.for_attr(&target, "id", old.as_deref(), new.as_deref());
                desc = format!("id {old:?} -> {new:?}");
            }
            4 | 5 => {
                let (name, vals): (&str, &[Option<&str>]) = match r.below(6) {
                    0 | 1 => ("x", &[Some("1"), None, Some("2")]),
                    2 => ("disabled", &[Some(""), None]),
                    3 => ("checked", &[Some(""), None]),
                    4 => ("lang", &[Some("en"), None, Some("fr")]),
                    _ => ("href", &[Some("/"), None]),
                };
                let new = vals[r.below(vals.len())];
                let old = element_attr(&target, name).map(|v| v.to_string());
                set_attr(&target, name, new);
                marks = m.for_attr(&target, name, old.as_deref(), new);
                desc = format!("{name} {old:?} -> {new:?}");
            }
            6 => {
                set_attr(&target, "style", Some("color:blue"));
                marks = m.for_attr(&target, "style", None, Some("color:blue"));
                desc = "style".into();
            }
            7 | 8 => {
                // insert a random subtree under a random element
                let mut k = 100;
                let frag = parse_html(&format!("<body>{}</body>", rand_html(&mut r, 2, &mut k)));
                let node = el_first_body_child(&frag);
                let node = node.unwrap();
                detach(&node);
                let parent = if r.below(4) == 0 { body.clone() } else { target.clone() };
                let idx = r.below(parent.children.borrow().len() + 1);
                attach(&parent, &node, idx);
                marks = m.for_children(&parent, &ChildChange::Inserted(node));
                desc = format!("insert into {}", element_tag(&parent));
            }
            9 | 10 => {
                let parent = parent_handle(&target).unwrap();
                let index = detach(&target);
                marks = m.for_children(&parent, &ChildChange::Removed { node: target.clone(), index });
                desc = format!("remove {}", element_tag(&target));
                if kind == 10 {
                    // a move: reinsert elsewhere (not into itself: it is detached, so any element still in the tree is fine)
                    let els2 = all_elements(&doc);
                    let np = els2[r.below(els2.len())].clone();
                    if !matches!(element_tag(&np), "html" | "head") {
                        let idx = r.below(np.children.borrow().len() + 1);
                        attach(&np, &target, idx);
                        marks.extend(m.for_children(&np, &ChildChange::Inserted(target.clone())));
                        desc += " + move";
                    }
                }
            }
            11 => {
                // text node insert/remove
                let idx = r.below(target.children.borrow().len() + 1);
                let has_text = target.children.borrow().iter().position(|c| matches!(c.data, NodeData::Text { .. }));
                match has_text {
                    Some(i) if r.below(2) == 0 => {
                        let c = target.children.borrow()[i].clone();
                        target.children.borrow_mut().remove(i);
                        c.parent.set(None);
                    }
                    _ => {
                        let tn = markup5ever_rcdom::Node::new(NodeData::Text { contents: RefCell::new("t".into()) });
                        attach(&target, &tn, idx);
                    }
                }
                marks = m.for_text(&target);
                desc = "text".into();
            }
            12 | 13 => {
                // hover / active chain change
                let old = chain_set(if kind == 12 { &st.hover } else { &st.active });
                let new: HashSet<usize> = if r.below(4) == 0 { HashSet::new() } else { chain(&target).iter().map(node_id).collect() };
                if kind == 12 {
                    st.hover = new.clone();
                } else {
                    st.active = new.clone();
                }
                set_interaction(&st);
                for e in &els {
                    let id = node_id(e);
                    if old.contains(&id) != new.contains(&id) {
                        marks.extend(m.for_state(e, if kind == 12 { StateKind::Hover } else { StateKind::Active }));
                    }
                }
                desc = "hover/active".into();
            }
            _ => {
                // focus / focus-within chain
                let old_f = st.focus;
                let old_w = st.focus_within.clone();
                if r.below(4) == 0 {
                    st.focus = None;
                    st.focus_within.clear();
                } else {
                    st.focus = Some(node_id(&target));
                    st.focus_within = chain(&target).iter().map(node_id).collect();
                }
                st.focus_visible = r.below(2) == 0;
                set_interaction(&st);
                for e in &els {
                    let id = node_id(e);
                    if (old_f == Some(id)) != (st.focus == Some(id)) {
                        marks.extend(m.for_state(e, StateKind::Focus));
                        marks.extend(m.for_state(e, StateKind::FocusVisible));
                    }
                    if old_w.contains(&id) != st.focus_within.contains(&id) {
                        marks.extend(m.for_state(e, StateKind::FocusWithin));
                    }
                }
                desc = "focus".into();
            }
        }
        let after = match_sets(&doc, &rules);
        let global = marks.iter().any(|(_, mk)| *mk == Mark::Global);
        let predicted: HashSet<usize> = marks.iter().filter(|(_, mk)| *mk >= Mark::Restyle).map(|(h, _)| node_id(h)).collect();
        let mut changed = 0;
        let mut pred_existing = 0;
        for (id, b) in &before {
            let Some(a) = after.get(id) else { continue };
            if predicted.contains(id) {
                pred_existing += 1;
            }
            if a != b {
                changed += 1;
                assert!(global || predicted.contains(id), "UNSOUND seed {seed} step {step}: {desc}\nhtml: {html}\ncss: {css}");
            }
        }
        t.steps += 1;
        t.changed += changed;
        if global {
            t.global_steps += 1;
        } else {
            t.predicted += pred_existing;
            if changed > 0 {
                t.predicted_on_changed_steps += pred_existing;
            }
        }
    }
    set_interaction(&crate::dom::InteractionState::default());
}

fn chain_set(s: &HashSet<usize>) -> HashSet<usize> {
    s.clone()
}

fn el_first_body_child(doc: &Handle) -> Option<Handle> {
    let body = all_elements(doc).into_iter().find(|e| element_tag(e) == "body")?;
    let kids = body.children.borrow();
    kids.iter().find(|c| matches!(c.data, NodeData::Element { .. })).cloned()
}

#[test]
fn soundness_random_fixtures_and_mutations() {
    let n: u64 = std::env::var("SCRATCHY_FUZZ_N").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let mut t = Tally::default();
    for seed in 1..=n {
        fuzz_case(seed, 12, &mut t);
    }
    let tight = if t.changed > 0 { t.predicted as f64 / t.changed as f64 } else { 0.0 };
    let tight_changed = if t.changed > 0 { t.predicted_on_changed_steps as f64 / t.changed as f64 } else { 0.0 };
    diag!(
        "invalidation fuzz: {} cases x 12 steps = {} steps, {} global, really changed {}, predicted {} (tightness over all steps {:.2}, over steps with a change {:.2})",
        n, t.steps, t.global_steps, t.changed, t.predicted, tight, tight_changed
    );
    assert!(t.changed > 0, "the corpus never changed a match: generator is broken");
}
