//! QINC1h1 (R60 sections 4.3, 8.1-8.5): relayout from the last laid tree, in shadow mode. Every
//! `measure_document` of a shadowed build in the suite already compares the engine's relayout with
//! the from-scratch tree (a difference panics inside the layout); these tests feed the engine through
//! the write log ONLY and pin WHERE the relayout stops and what it reuses.
use super::*;
use crate::incr_layout::{self, LayoutStats};
use crate::incr_log::{self, Change, Val};
use crate::incr_style::{self, Seam};

struct LogOnly(Seam);

impl LogOnly {
    fn new() -> LogOnly {
        incr_style::reset();
        crate::incr_box::reset();
        incr_layout::reset();
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
        crate::incr_box::reset();
        incr_layout::reset();
    }
}

thread_local! {
    static FS: std::cell::RefCell<Option<FontSystem>> = const { std::cell::RefCell::new(None) };
}

/// Build + lay out from scratch (the shadow relayout runs and compares inside); the engine's stats.
fn lay(doc: &Handle) -> LayoutStats {
    let tree = crate::images::build_layout_tree_with_resources(doc, 800.0, Some(600.0), "", &HashMap::new());
    FS.with(|f| {
        let mut f = f.borrow_mut();
        let fs = f.get_or_insert_with(new_font_system);
        let mut cache = ShapeCache::new();
        let (mut next_id, mut inc) = (0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        measure_document(&tree, fs, 800.0, 600.0, &mut next_id, &mut cache, deadline, &mut inc);
        assert!(!inc);
    });
    incr_layout::with_engine(|e| e.expect("layout engine").stats.clone())
}

fn page(css: &str, body: &str) -> Handle {
    parse_html(&format!("<html><head><style>body{{margin:0}} {css}</style></head><body>{body}</body></html>"))
}

fn set_attr(h: &Handle, name: &str, v: &str) {
    let old = element_attr(h, name).map(|s| s.to_string());
    crate::js_dom::set_attr(h, name, v);
    incr_log::note(node_id(h), Change::Attr { name: name.into(), old: Val::of(old.as_deref()), new: Val::of(Some(v)) });
}

fn set_text(el: &Handle, v: &str) {
    let text = el.children.borrow().iter().find(|c| matches!(c.data, NodeData::Text { .. })).cloned().expect("a text child");
    if let NodeData::Text { contents } = &text.data {
        *contents.borrow_mut() = v.into();
    }
    incr_log::note(node_id(&text), Change::Text);
}

fn insert_at(parent: &Handle, child: &Handle, at: usize) {
    child.parent.set(Some(Rc::downgrade(parent)));
    parent.children.borrow_mut().insert(at, child.clone());
    let (prev, next) = incr_log::siblings(child);
    incr_log::note(node_id(parent), Change::ChildInsert { child: node_id(child), prev, next });
}

fn remove(child: &Handle) {
    let p = parent_handle(child).expect("attached");
    let (prev, next) = incr_log::siblings(child);
    p.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, child));
    child.parent.set(None);
    incr_log::note(node_id(&p), Change::ChildRemove { child: node_id(child), prev, next });
}

fn element(html: &str) -> Handle {
    let d = parse_html(&format!("<html><body>{html}</body></html>"));
    let body = find_by_tag(&d, "body").unwrap();
    let c = body.children.borrow()[0].clone();
    body.children.borrow_mut().clear();
    c.parent.set(None);
    c
}

const LONG: &str = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore \
                    et dolore magna aliqua ut enim ad minim veniam quis nostrud exercitation ullamco laboris nisi ut";

fn incremental(s: &LayoutStats) {
    assert!(!s.full, "an incremental update, not a full layout: {s:?}");
}

#[test]
fn qinc1h1_first_layout_is_full_and_an_unchanged_page_relayouts_nothing() {
    let _g = LogOnly::new();
    let doc = page("", "<div><p>x</p></div><div><p>y</p></div>");
    assert!(lay(&doc).full);
    let s = lay(&doc);
    incremental(&s);
    assert_eq!((s.remeasured, s.splices, s.root), (0, 0, false), "{s:?}");
}

#[test]
fn qinc1h1_a_write_inside_an_abs_box_stops_at_it() {
    let _g = LogOnly::new();
    let doc = page(
        "#cb{position:relative;height:300px} #a{position:absolute;left:10px;top:20px;width:200px;height:80px}",
        "<div id=cb><div id=a><div id=t>short</div></div></div><div id=after><div>below</div></div>",
    );
    lay(&doc);
    set_text(&by_id(&doc, "t"), &format!("{LONG} {LONG}"));
    let s = lay(&doc);
    incremental(&s);
    assert!(!s.root, "the walk stopped below the root: {s:?}");
    assert!(s.splices >= 1, "{s:?}");
    // #t's height changed: one level up, the abs box (fixed size) is the boundary.
    assert!(s.levels.contains(&1), "{s:?}");
    assert!(s.levels.iter().all(|l| *l <= 1), "{s:?}");
}

#[test]
fn qinc1h1_an_auto_height_box_whose_height_does_not_change_stops_there() {
    let _g = LogOnly::new();
    let mut body = String::from("<div id=w><div id=p>hello world</div></div>");
    for i in 0..60 {
        body.push_str(&format!("<section><div>row {i}</div></section>"));
    }
    let doc = page("", &body);
    lay(&doc);
    set_text(&by_id(&doc, "p"), "hello there");
    let s = lay(&doc);
    incremental(&s);
    assert!(!s.root, "{s:?}");
    assert_eq!(s.levels, vec![0], "spliced at the paragraph itself: {s:?}");
    // The floor is the path, not the page: 60 sections are not walked.
    assert!(incr_layout::walked() < 30, "walked {}", incr_layout::walked());
}

#[test]
fn qinc1h1_a_growing_paragraph_moves_the_following_siblings() {
    let _g = LogOnly::new();
    let doc = page(
        "",
        "<div id=w><div id=p>short</div></div><section><div>one</div></section><section><div>two</div></section>",
    );
    lay(&doc);
    set_text(&by_id(&doc, "p"), LONG);
    let s = lay(&doc);
    incremental(&s);
    // The height reaches the root; the two sections are reused and moved down, not re-measured.
    assert!(s.root, "{s:?}");
    assert!(s.hits >= 2 && s.shifted >= 2, "{s:?}");
}

#[test]
fn qinc1h1_floats_in_force_a_walk_up() {
    let _g = LogOnly::new();
    let doc = page(
        "#f{float:left;width:50px;height:50px}",
        "<div id=w><div id=f></div><div id=p>beside the float</div></div><section><div>x</div></section>",
    );
    lay(&doc);
    set_text(&by_id(&doc, "p"), "beside it too");
    let s = lay(&doc);
    incremental(&s);
    assert!(s.root || !s.levels.contains(&0), "a box with floats coming in is never spliced alone: {s:?}");
    assert!(s.miss_not_cacheable >= 1, "{s:?}");
}

#[test]
fn qinc1h1_flex_item_text_change_reruns_the_container_and_reuses_the_siblings() {
    let _g = LogOnly::new();
    let doc = page(
        "#fx{display:flex;height:100px} #fx > div{flex:1}",
        "<div id=fx><div id=a>aa</div><div id=b><div>bb</div></div><div id=c><div>cc</div></div></div><section><div>x</div></section>",
    );
    lay(&doc);
    set_text(&by_id(&doc, "a"), "aaaa");
    let s = lay(&doc);
    incremental(&s);
    assert!(!s.root, "the fixed-height container stops the walk: {s:?}");
    assert!(s.levels.contains(&1), "{s:?}");
    assert!(s.hits >= 2, "#b and #c come from the old tree: {s:?}");
}

#[test]
fn qinc1h1_table_cell_change() {
    let _g = LogOnly::new();
    let doc = page(
        "td{border:1px solid}",
        "<table id=t><tr><td id=a>x</td><td id=b><div>y</div></td></tr><tr><td><div>z</div></td><td><div>w</div></td></tr></table><section><div>after</div></section>",
    );
    lay(&doc);
    set_text(&by_id(&doc, "a"), "a much longer cell text than before");
    let s = lay(&doc);
    incremental(&s);
    assert!(s.hits >= 1, "{s:?}");
}

#[test]
fn qinc1h1_fixed_descendant_inside_a_clean_subtree_is_re_emitted_added_and_removed() {
    let _g = LogOnly::new();
    let doc = page(
        "#fx{position:fixed;left:5px;top:6px;width:10px;height:10px}",
        "<div id=w><div id=p>short</div></div><div id=keep><div id=host><div id=fx></div></div></div>",
    );
    lay(&doc);
    // #keep is reused (and moved down); its fixed descendant bubbles past it to the root again.
    set_text(&by_id(&doc, "p"), LONG);
    let s = lay(&doc);
    incremental(&s);
    assert!(s.hits >= 1 && s.shifted >= 1, "{s:?}");
    remove(&by_id(&doc, "fx"));
    incremental(&lay(&doc));
    insert_at(&by_id(&doc, "host"), &element("<div id=fx2 style='position:fixed;right:0;bottom:0;width:4px;height:4px'></div>"), 0);
    incremental(&lay(&doc));
    set_text(&by_id(&doc, "p"), "short");
    let s = lay(&doc);
    incremental(&s);
    assert!(s.hits >= 1, "{s:?}");
}

#[test]
fn qinc1h1_z_index_child_inside_a_reused_subtree_keeps_its_paint_order() {
    let _g = LogOnly::new();
    let doc = page(
        "#keep{position:relative} #z{position:relative;z-index:2} #n{position:relative;z-index:-1}",
        "<div id=w><div id=p>short</div></div><div id=keep><div id=z>z</div><div id=n>n</div></div><div id=other style='position:relative;z-index:1'>o</div>",
    );
    lay(&doc);
    set_text(&by_id(&doc, "p"), LONG);
    let s = lay(&doc);
    incremental(&s);
    assert!(s.hits >= 1 && s.shifted >= 1, "{s:?}");
    // An insertion before them renumbers document order: the reused entries are re-keyed.
    insert_at(&by_id(&doc, "w"), &element("<div style='position:relative;z-index:2'>first</div>"), 0);
    incremental(&lay(&doc));
}

#[test]
fn qinc1h1_sticky_inside_a_reused_subtree() {
    let _g = LogOnly::new();
    let doc = page(
        "#st{position:sticky;top:0}",
        "<div id=w><div id=p>short</div></div><div id=keep><div id=st>s</div><div>t</div><div>u</div></div>",
    );
    lay(&doc);
    set_text(&by_id(&doc, "p"), LONG);
    let s = lay(&doc);
    incremental(&s);
    assert!(s.hits >= 1 && s.shifted >= 1, "{s:?}");
    set_attr(&by_id(&doc, "st"), "style", "top:5px");
    incremental(&lay(&doc));
}

#[test]
fn qinc1h1_scroll_container_content_change_stops_at_the_container() {
    let _g = LogOnly::new();
    let doc = page("#sc{overflow:auto;height:50px}", "<div id=sc><div id=p>short</div></div><section><div>after</div></section>");
    lay(&doc);
    set_text(&by_id(&doc, "p"), &format!("{LONG} {LONG} {LONG}"));
    let s = lay(&doc);
    incremental(&s);
    assert!(!s.root, "{s:?}");
    assert!(s.levels.contains(&1), "the paragraph grew, the fixed-height scroller did not: {s:?}");
}

#[test]
fn qinc1h1_offset_only_change_re_places_the_box() {
    let _g = LogOnly::new();
    let doc = page(
        "#r{position:relative} #a{position:absolute;width:50px;height:20px}",
        "<div id=r><div id=a style='left:1px;top:1px'><div>in</div></div><div>flow</div></div>",
    );
    lay(&doc);
    set_attr(&by_id(&doc, "a"), "style", "left:30px;top:7px");
    let s = lay(&doc);
    incremental(&s);
    assert_eq!(s.levels, vec![0], "re-resolved at its old containing block: {s:?}");
    // A relative offset: the parent's flow does not change.
    set_attr(&by_id(&doc, "r"), "style", "left:4px");
    let s = lay(&doc);
    incremental(&s);
    assert!(!s.root, "{s:?}");
}
