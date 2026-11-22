//! QAPI6b: the cost of `:has()` on pages that use it. Changes under an unstyled subtree (`display: none`, or a
//! subtree inserted in the same batch) are answered once per subtree (`InvalidationMap::for_has_any`) instead of
//! once per logged change, and one `SelectorCaches` is shared by every match of a build. Correctness is pinned
//! with the log-only engine (a mismatch with the from-scratch styles panics) and against expected colors.
use super::*;
use crate::incr_log::{self, Change, Val};
use crate::incr_style::{self, Seam, Stats};
use crate::invalidation::InvalidationMap;

struct LogOnly(Seam);

impl LogOnly {
    fn new() -> LogOnly {
        incr_style::reset();
        let _ = incr_log::drain();
        set_interaction(&InteractionState::default());
        LogOnly(incr_style::set_seam(Seam { shadow: true, reconcile: false, panic: true }))
    }
}

impl Drop for LogOnly {
    fn drop(&mut self) {
        incr_style::set_seam(self.0);
        set_interaction(&InteractionState::default());
        incr_style::reset();
    }
}

fn build(doc: &Handle) -> Stats {
    let _ = build_layout_tree_sized(doc, 800.0, 600.0);
    incr_style::with_engine(|e| e.expect("shadow engine").stats.clone())
}

fn page(css: &str, body: &str) -> Handle {
    parse_html(&format!("<html><head><style>{css}</style></head><body>{body}</body></html>"))
}

fn color(h: &Handle) -> TextColor {
    incr_style::with_engine(|e| e.unwrap().entry(h).expect("styled").style().unwrap().color)
}

fn red() -> TextColor {
    TextColor::rgb(255, 0, 0)
}

fn black() -> TextColor {
    TextColor::rgb(0, 0, 0)
}

fn set_attr(h: &Handle, name: &str, v: Option<&str>) {
    let old = element_attr(h, name).map(|s| s.to_string());
    match v {
        Some(v) => crate::js_dom::set_attr(h, name, v),
        None => {
            if let NodeData::Element { attrs, .. } = &h.data {
                attrs.borrow_mut().retain(|a| a.name.local.as_ref() != name);
            }
        }
    }
    incr_log::note_node(h, Change::Attr { name: name.into(), old: Val::of(old.as_deref()), new: Val::of(v) });
}

fn append(parent: &Handle, child: &Handle) {
    child.parent.set(Some(Rc::downgrade(parent)));
    parent.children.borrow_mut().push(child.clone());
    let (prev, next) = incr_log::siblings(child);
    incr_log::note_node(parent, Change::ChildInsert { child: node_id(child), prev, next });
}

fn remove(child: &Handle) {
    let p = parent_handle(child).expect("attached");
    let (prev, next) = incr_log::siblings(child);
    p.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, child));
    child.parent.set(None);
    incr_log::note_node(&p, Change::ChildRemove { child: node_id(child), prev, next });
}

fn element(html: &str) -> Handle {
    let d = parse_html(&format!("<html><body>{html}</body></html>"));
    let body = find_by_tag(&d, "body").unwrap();
    let c = body.children.borrow()[0].clone();
    body.children.borrow_mut().clear();
    c.parent.set(None);
    c
}

const CSS: &str = ".hide{display:none} .card:has(.on){color:red} h2:has(+ .hide .on){color:red} \
    .box:has(> .hide > li:nth-child(3)){color:red} :has(> .hide [data-x]) > p{color:red}";

const BODY: &str = "<div id=c class=card><ul id=l class=hide></ul></div>\
    <h2 id=h>t</h2><div class=hide><ul id=l2></ul></div>\
    <div id=b class=box><ul id=l3 class=hide></ul></div>\
    <section><div class=hide><ol id=l4></ol></div><p id=p>p</p></section>";

#[test]
fn qapi6b_filling_hidden_lists_in_one_update_matches_from_scratch() {
    let _g = LogOnly::new();
    let doc = page(CSS, BODY);
    build(&doc);
    let (c, h, b, p) = (by_id(&doc, "c"), by_id(&doc, "h"), by_id(&doc, "b"), by_id(&doc, "p"));
    for id in ["c", "h", "b", "p"] {
        assert_eq!(color(&by_id(&doc, id)), black(), "#{id}");
    }
    // a script fills four hidden lists with 300 items in one batch; the last items carry the features
    let mut last = Vec::new();
    for id in ["l", "l2", "l3", "l4"] {
        let l = by_id(&doc, id);
        for i in 0..75 {
            let li = element("<li>x</li>");
            append(&l, &li);
            if i == 74 {
                last.push(li);
            }
        }
    }
    set_attr(&last[0], "class", Some("on"));
    set_attr(&last[1], "class", Some("on"));
    set_attr(&last[3], "data-x", Some("1"));
    let s = build(&doc);
    assert!(!s.full, "{s:?}");
    assert_eq!(color(&c), red());
    assert_eq!(color(&h), red());
    assert_eq!(color(&b), red(), "li:nth-child(3) under the hidden list");
    assert_eq!(color(&p), red());
    // the hidden subtrees themselves are not styled, so only the anchors (and their outer-hop subjects) re-match
    assert!(s.rematched < 40, "{s:?}");
    // and back, again in one batch: removals and attribute writes under the hidden lists
    set_attr(&last[0], "class", None);
    remove(&last[1]);
    set_attr(&last[3], "data-x", None);
    let l3 = by_id(&doc, "l3");
    let kids: Vec<Handle> = l3.children.borrow().iter().filter(|c| matches!(c.data, NodeData::Element { .. })).cloned().collect();
    for k in kids.iter().skip(2) {
        remove(k);
    }
    let s = build(&doc);
    assert!(!s.full, "{s:?}");
    for (h, id) in [(&c, "c"), (&h, "h"), (&b, "b"), (&p, "p")] {
        assert_eq!(color(h), black(), "#{id}");
    }
}

#[test]
fn qapi6b_subtree_inserted_and_filled_in_one_batch() {
    // the new subtree is inserted under a styled element and then filled: the writes inside it are under an
    // element the engine has no entry for yet
    let _g = LogOnly::new();
    let doc = page(".card:has(.on){color:red}", "<div id=c class=card><p>x</p></div>");
    build(&doc);
    let c = by_id(&doc, "c");
    let wrap = element("<div></div>");
    append(&c, &wrap);
    for _ in 0..50 {
        append(&wrap, &element("<span>s</span>"));
    }
    let on = element("<b>b</b>");
    append(&wrap, &on);
    set_attr(&on, "class", Some("on"));
    build(&doc);
    assert_eq!(color(&c), red());
    set_attr(&on, "class", None);
    build(&doc);
    assert_eq!(color(&c), black());
}

#[test]
fn qapi6b_for_has_any_reaches_every_has_anchor_above_the_subtree() {
    let rules = parse_stylesheet(".a:has(.x){color:red} h2:has(+ div){color:red} .a:has(.y) > p{color:red} .b .c{color:red}");
    let m = InvalidationMap::build(&rules, 1);
    let doc = parse_html("<html><body><div id=a1 class=a><h2 id=h>t</h2><div id=u><i></i></div><p id=p>p</p></div><div class=b><span class=c id=n></span></div></body></html>");
    let u = by_id(&doc, "u");
    let mut got: Vec<String> = m.for_has_any(&u).iter().filter_map(|(h, _)| element_attr(h, "id").map(|s| s.to_string())).collect();
    got.sort();
    got.dedup();
    // `.a` anchor above, `h2` as the earlier sibling (`+`), the `> p` subject; nothing from the non-`:has` rule
    assert_eq!(got, vec!["a1", "h", "p"]);
    // a page without `:has` answers nothing
    let m = InvalidationMap::build(&parse_stylesheet(".b .c{color:red}"), 1);
    assert!(m.for_has_any(&u).is_empty());
}

#[test]
fn qapi6b_removed_subtree_changed_after_removal_still_restyles_the_anchor() {
    // the engine answers a batch against the final DOM: a removed card that loses `.on` while detached (no log
    // entry: it is not connected) was still found by the anchor before the batch
    let _g = LogOnly::new();
    let doc = page(".box:has(.on){color:red}", "<div id=b class=box><div id=card><i id=i class=on>i</i></div></div>");
    build(&doc);
    let b = by_id(&doc, "b");
    assert_eq!(color(&b), red());
    let card = by_id(&doc, "card");
    let i = by_id(&doc, "i");
    remove(&card);
    crate::js_dom::set_attr(&i, "class", "off");
    build(&doc);
    assert_eq!(color(&b), black());
}

#[test]
fn qapi6b_has_inside_not_is_where_soundness_fuzz() {
    // the anchor of a `:has` inside `:not()` / `:is()` / `:where()` is the enclosing compound's element
    const SHAPES: [&str; 10] = [
        ".K:not(:has(.K)) .K", "div:where(:has(+ .K)) span", ".K:is(:has(> .K)) > .K", "li.K:not(:has(~ .K))", ":is(.K:has(.K), p) .K",
        ".K:not(:has(.K)) .K .K", "span:not(.K:has(.K))", ".K:not(:hover) .K", "p:is(:has(.K:first-child)) ~ .K", ".K:where(:not(:has(:empty)))",
    ];
    let n: u64 = std::env::var("SCRATCHY_FUZZ_N").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let mut t = super::invalidation_tests::Tally { steps: 0, changed: 0, predicted: 0, predicted_on_changed_steps: 0, global_steps: 0 };
    for seed in 1..=n {
        super::invalidation_tests::fuzz_case_with(seed, 12, &mut t, &SHAPES);
    }
    diag!("qapi6b :not/:is(:has) fuzz: {} steps, {} global, changed {}, predicted {}", t.steps, t.global_steps, t.changed, t.predicted);
    assert!(t.changed > 0);
    assert_eq!(t.global_steps, 0);
}

#[test]
fn qapi6b_has_inside_not_is_anchored_by_the_enclosing_compound() {
    let m = InvalidationMap::build(&parse_stylesheet(".acc:not(:has(.open)) .vis{color:red}"), 1);
    let doc = parse_html("<html><body><div id=outer><div id=a class=acc><i id=i></i><p class=vis id=v1></p></div></div><p class=vis id=v2></p></body></html>");
    let i = by_id(&doc, "i");
    let mut got: Vec<String> = m.for_has_any(&i).iter().filter_map(|(h, _)| element_attr(h, "id").map(|s| s.to_string())).collect();
    got.sort();
    // only the `.acc` anchor's `.vis` descendants, not every ancestor's (body would reach #v2)
    assert_eq!(got, vec!["v1"]);
}

#[test]
fn qapi6b_shared_selector_caches_never_outlive_a_build() {
    // nth-child and :has results are cached per build; a DOM write between two builds must be seen
    let _g = LogOnly::new();
    let doc = page("li:nth-child(2){color:red} .card:has(> .on){color:red}", "<ul><li id=a>a</li><li id=b>b</li></ul><div id=c class=card><i id=i>i</i></div>");
    build(&doc);
    let (a, b, c) = (by_id(&doc, "a"), by_id(&doc, "b"), by_id(&doc, "c"));
    assert_eq!(color(&b), red());
    assert_eq!(color(&c), black());
    remove(&a);
    set_attr(&by_id(&doc, "i"), "class", Some("on"));
    build(&doc);
    assert_eq!(color(&b), black(), "b is now the first child");
    assert_eq!(color(&c), red());
}
