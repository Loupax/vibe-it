//! QINC1g (R60 sections 4.2, 7): the persistent box tree in shadow mode. Every build in the suite
//! already compares the kept tree with the from-scratch tree (a difference panics inside the build);
//! these tests feed the engine through the write log ONLY (no reconcile) and pin how much of the tree
//! an update rebuilds and what it moves over unchanged.
use super::*;
use crate::incr_box::{self, BoxStats};
use crate::incr_log::{self, Change, Val};
use crate::incr_style::{self, Seam};

struct LogOnly(Seam);

impl LogOnly {
    fn new() -> LogOnly {
        incr_style::reset();
        incr_box::reset();
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
        incr_box::reset();
    }
}

fn build_with(doc: &Handle, cache: &HashMap<String, Arc<DecodedImage>>) -> (LayoutBox, BoxStats) {
    let tree = crate::images::build_layout_tree_with_resources(doc, 800.0, Some(600.0), "", cache);
    (tree, incr_box::with_engine(|e| e.expect("box engine").stats.clone()))
}

fn build(doc: &Handle) -> BoxStats {
    build_with(doc, &HashMap::new()).1
}

fn page(css: &str, body: &str) -> Handle {
    parse_html(&format!("<html><head><style>{css}</style></head><body>{body}</body></html>"))
}

fn set_attr(h: &Handle, name: &str, v: &str) {
    let old = element_attr(h, name).map(|s| s.to_string());
    crate::js_dom::set_attr(h, name, v);
    incr_log::note(node_id(h), Change::Attr { name: name.into(), old: Val::of(old.as_deref()), new: Val::of(Some(v)) });
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

fn set_text(el: &Handle, v: &str) {
    let text = el.children.borrow().iter().find(|c| matches!(c.data, NodeData::Text { .. })).cloned().expect("a text child");
    if let NodeData::Text { contents } = &text.data {
        *contents.borrow_mut() = v.into();
    }
    incr_log::note(node_id(&text), Change::Text);
}

/// A detached element parsed from `html` (its first body child).
fn element(html: &str) -> Handle {
    let d = parse_html(&format!("<html><body>{html}</body></html>"));
    let body = find_by_tag(&d, "body").unwrap();
    let c = body.children.borrow()[0].clone();
    body.children.borrow_mut().clear();
    c.parent.set(None);
    c
}

/// Every text item of the kept tree, in order.
fn kept_texts() -> Vec<String> {
    fn walk(b: &LayoutBox, out: &mut Vec<String>) {
        match &b.content {
            BoxContent::Blocks(v) | BoxContent::TableRow(v) => v.iter().for_each(|c| walk(c, out)),
            BoxContent::Inline(items) => {
                for i in items {
                    match i {
                        InlineItem::Text(t, _) => out.push(t.clone()),
                        InlineItem::Embedded(e) => walk(e, out),
                        _ => {}
                    }
                }
            }
            BoxContent::Image(_) => {}
        }
    }
    let mut out = Vec::new();
    incr_box::with_engine(|e| walk(e.unwrap().tree(), &mut out));
    out
}

#[test]
fn qinc1g_first_build_copies_and_a_clean_build_rebuilds_nothing() {
    let _g = LogOnly::new();
    let doc = page("", "<div id=a><p>x</p></div><div id=b><p>y</p></div>");
    assert!(build(&doc).full);
    let s = build(&doc);
    assert!(!s.full);
    assert_eq!((s.rebuilt, s.reused, s.path), (0, 0, 0), "{s:?}");
}

#[test]
fn qinc1g_insert_and_remove_block_children_reuse_the_siblings() {
    let _g = LogOnly::new();
    let doc = page("", "<div id=c><p id=p1>a</p><p id=p2>b</p></div><div id=d><p>z</p></div>");
    build(&doc);
    let c = by_id(&doc, "c");
    insert_at(&c, &element("<p id=n>new</p>"), 1);
    let s = build(&doc);
    // html, body, #c and the new p are built; p1, p2 and #d move over.
    assert_eq!(s.rebuilt, 4, "{s:?}");
    assert_eq!(s.reused, 3, "{s:?}");
    assert!(kept_texts().contains(&"new".to_string()));
    remove(&by_id(&doc, "p1"));
    let s = build(&doc);
    assert_eq!(s.rebuilt, 3, "{s:?}");
    assert_eq!(s.reused, 3, "n, p2 and #d: {s:?}");
    assert!(!kept_texts().contains(&"a".to_string()));
}

#[test]
fn qinc1g_insert_and_remove_inline_children() {
    let _g = LogOnly::new();
    let doc = page("", "<p id=p>one <b id=b>two</b> three</p><div id=o>other</div>");
    build(&doc);
    let p = by_id(&doc, "p");
    insert_at(&p, &element("<i>ins</i>"), 1);
    let s = build(&doc);
    assert_eq!(s.reused, 1, "#o moves over: {s:?}");
    assert!(kept_texts().iter().any(|t| t == "ins"), "{:?}", kept_texts());
    remove(&by_id(&doc, "b"));
    let s = build(&doc);
    assert_eq!(s.reused, 1, "{s:?}");
    assert!(!kept_texts().iter().any(|t| t.contains("two")));
}

#[test]
fn qinc1g_text_edit_in_an_inline_element_rebuilds_its_paragraph_only() {
    let _g = LogOnly::new();
    let doc = page("", "<div id=a><p id=p1>x <b id=b>y</b></p><p id=p2>z</p><p id=p3>w</p></div><div id=q><p>v</p></div>");
    build(&doc);
    set_text(&by_id(&doc, "b"), "changed");
    let s = build(&doc);
    // html, body, #a and #p1 (the paragraph holding the run) are rebuilt; p2, p3 and #q move over.
    assert_eq!(s.rebuilt, 4, "{s:?}");
    assert_eq!(s.reused, 3, "{s:?}");
    assert!(kept_texts().contains(&"changed".to_string()), "{:?}", kept_texts());
}

#[test]
fn qinc1g_display_changes_rebuild_the_container() {
    let _g = LogOnly::new();
    let doc = page(
        ".none{display:none} .block{display:block} .inline{display:inline} .contents{display:contents} \
         .flex{display:flex} .cell{display:table-cell}",
        "<div id=w>before <span id=x class=none>X <i>in</i></span> after<div id=s>sib</div></div><p id=o>o</p>",
    );
    build(&doc);
    let x = by_id(&doc, "x");
    for cls in ["block", "inline", "contents", "flex", "cell", "none", "cell", "block"] {
        set_attr(&x, "class", cls);
        let s = build(&doc); // the shadow comparison panics on any difference
        assert!(!s.full, "{cls}: {s:?}");
        assert!(s.reused >= 1, "{cls}: #o moves over: {s:?}");
    }
}

#[test]
fn qinc1g_list_renumbers_after_an_insert() {
    let _g = LogOnly::new();
    let doc = page("", "<ol id=l><li>a</li><li>b</li><li>c</li></ol><ul id=u><li>u</li></ul>");
    build(&doc);
    insert_at(&by_id(&doc, "l"), &element("<li>first</li>"), 0);
    let s = build(&doc);
    assert!(!s.full);
    let t = kept_texts();
    for want in ["1. ", "2. ", "3. ", "4. "] {
        assert!(t.iter().any(|x| x == want), "{want} in {t:?}");
    }
    assert!(s.reused >= 1, "the <ul> moves over: {s:?}");
}

#[test]
fn qinc1g_before_content_change() {
    let _g = LogOnly::new();
    let doc = page(".a::before{content:\"AAA\"} .b::before{content:\"BBB\";display:block}", "<p id=p class=a>t</p><p>o</p>");
    build(&doc);
    assert!(kept_texts().contains(&"AAA".to_string()));
    set_attr(&by_id(&doc, "p"), "class", "b");
    let s = build(&doc);
    assert!(!s.full);
    let t = kept_texts();
    assert!(t.contains(&"BBB".to_string()) && !t.contains(&"AAA".to_string()), "{t:?}");
}

#[test]
fn qinc1g_image_arrival_rebuilds_the_boxes_that_read_it() {
    let _g = LogOnly::new();
    let doc = page(
        ".bg{background-image:url(b.png);width:10px;height:10px}",
        "<p id=p>x <img src=a.png alt=alt> y</p><div class=bg id=g></div><p id=o>other</p>",
    );
    let empty = HashMap::new();
    build_with(&doc, &empty);
    let img = Arc::new(DecodedImage { width: 3, height: 2, pixels: vec![TextColor::rgb(1, 2, 3); 6], svg: None, mask: None });
    let mut cache: HashMap<String, Arc<DecodedImage>> = HashMap::new();
    cache.insert("a.png".into(), img.clone());
    let (_, s) = build_with(&doc, &cache); // compared with the from-scratch tree inside
    assert_eq!(s.images_changed, 1, "{s:?}");
    assert!(s.reused >= 2, "#g and #o move over: {s:?}");
    assert!(!kept_texts().contains(&"alt".to_string()));
    cache.insert("b.png".into(), img);
    let (_, s) = build_with(&doc, &cache);
    assert_eq!(s.images_changed, 1, "{s:?}");
    assert!(s.reused >= 2, "#p and #o move over: {s:?}");
}

#[test]
fn qinc1g_first_line_follows_a_text_edit() {
    let _g = LogOnly::new();
    let doc = page("p::first-line{color:red} p::first-letter{font-size:30px}", "<p id=p>hello world</p><p>other</p>");
    build(&doc);
    set_text(&by_id(&doc, "p"), "goodbye");
    let s = build(&doc);
    assert!(!s.full);
    assert_eq!(s.reused, 1, "{s:?}");
}

#[test]
fn qinc1g_inline_block_tables_flex_and_generated_boxes_are_reused_inside() {
    let _g = LogOnly::new();
    let doc = page(
        ".f{display:flex} .ib{display:inline-block} li::marker{color:red} .g::after{content:\"!\"}",
        "<div class=f id=f><div id=f1>a</div>text<div id=f2 class=g>b</div></div>\
         <p>x <span class=ib id=ib><b id=ibb>in</b></span> y</p>\
         <table id=t><tr><td id=c1>1</td><td id=c2>2</td></tr></table><ul><li id=li>l</li></ul>",
    );
    build(&doc);
    for id in ["f1", "ibb", "c1", "li"] {
        let h = by_id(&doc, id);
        let first = h.children.borrow().iter().find(|c| matches!(c.data, NodeData::Element { .. })).cloned();
        match first {
            Some(_) => set_attr(&h, "class", "z"),
            None => set_text(&h, "edited"),
        }
        let s = build(&doc);
        assert!(!s.full && s.reused >= 2, "{id}: {s:?}");
    }
}

#[test]
fn qinc1g_bver_and_ldirty_follow_the_rebuilt_path() {
    let _g = LogOnly::new();
    let doc = page("", "<div id=a><p id=p>x</p></div><div id=b>y</div>");
    build(&doc);
    let (a, p, b) = (node_id(&by_id(&doc, "a")), node_id(&by_id(&doc, "p")), node_id(&by_id(&doc, "b")));
    let (va, vp, vb) = (incr_box::bver(a), incr_box::bver(p), incr_box::bver(b));
    set_text(&by_id(&doc, "p"), "changed");
    build(&doc);
    assert!(incr_box::bver(a) > va && incr_box::bver(p) > vp, "rebuilt boxes get a new version");
    assert_eq!(incr_box::bver(b), vb, "a reused box keeps its version");
    assert!(incr_box::ldirty(a) && incr_box::ldirty(p) && !incr_box::ldirty(b));
}

#[test]
fn qinc1g_display_none_roots_per_element_match_the_build() {
    let _g = LogOnly::new();
    let doc = page(".h{display:none}", "<div class=h id=h1><p class=h>in</p></div><p>x<span class=h id=h2>s</span></p>");
    crate::images::DISPLAY_NONE_ROOTS.with(|r| *r.borrow_mut() = Some(Vec::new()));
    build(&doc);
    let built: HashSet<usize> =
        crate::images::DISPLAY_NONE_ROOTS.with(|r| r.borrow_mut().take().unwrap().iter().map(node_id).collect());
    let mine: HashSet<usize> = incr_style::display_none_roots().iter().map(node_id).filter(|k| built.contains(k) || {
        // `<head>` and its children are display:none roots the build never visits under <body> only
        true
    }).collect();
    assert!(built.contains(&node_id(&by_id(&doc, "h1"))) && built.contains(&node_id(&by_id(&doc, "h2"))));
    assert!(built.is_subset(&mine), "{built:?} vs {mine:?}");
}

#[test]
fn qinc1g_a_broken_reuse_is_caught() {
    // The comparison has teeth: a stale kept tree (a text change the log never describes, reconcile
    // off) is reported as a box mismatch, not silently kept.
    let _g = LogOnly::new();
    incr_style::set_seam(Seam { shadow: true, reconcile: false, panic: false });
    let doc = page("", "<p id=p>x</p><p>y</p>");
    build(&doc);
    let t = by_id(&doc, "p").children.borrow()[0].clone();
    if let NodeData::Text { contents } = &t.data {
        *contents.borrow_mut() = "unlogged".into();
    }
    build(&doc);
    let m = incr_style::take_mismatches();
    assert!(m.iter().any(|x| x.contains("boxes") && x.contains("unlogged")), "{m:?}");
}
