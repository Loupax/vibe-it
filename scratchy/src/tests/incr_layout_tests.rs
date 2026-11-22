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
        LogOnly(incr_style::set_seam(Seam { reconcile: false, panic: true }))
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

/// QINC1h1, revised by QINC1k3: a box with floats coming in was never spliced alone (it walked up to the
/// BFC root). Since QINC1k3 its key holds the floats it was measured among (`Ck::fl`): re-measured
/// among the same floats with an unchanged parent view and the same floats placed, it is spliced.
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
    assert!(!s.root && s.levels == vec![0], "spliced at the box among the floats: {s:?}");
    assert_eq!(s.miss_not_cacheable, 0, "{s:?}");
}

/// QINC1k3: a box whose measure placed different floats (a taller float inside a non-BFC parent, whose
/// own height does not include it) cannot stop there: the floats reach its later siblings.
#[test]
fn qinc1k3_changed_floats_out_walk_up() {
    let _g = LogOnly::new();
    let doc = page(
        "#f{float:left;width:50px;height:50px}",
        "<div id=w><div id=f></div>above</div><div id=n>beside the float, maybe</div><section><div>x</div></section>",
    );
    lay(&doc);
    set_attr(&by_id(&doc, "f"), "style", "height:300px");
    let s = lay(&doc);
    incremental(&s);
    assert!(s.root || s.levels.iter().all(|l| *l >= 2), "walked past #f and #w: {s:?}");
}

/// QINC1k3: clean boxes among floats are reused when their parent is re-measured (same origin, same
/// floats coming in; the floats they placed are placed again for their later siblings).
#[test]
fn qinc1k3_clean_boxes_among_floats_are_reused() {
    let _g = LogOnly::new();
    let doc = page(
        "#f{float:right;width:80px;height:200px}",
        "<div id=w><div id=f></div><p>one beside it</p></div><p>two beside it</p><div><p>three beside it</p></div><div id=z>z</div>",
    );
    lay(&doc);
    let body = find_by_tag(&doc, "body").unwrap();
    let n = body.children.borrow().len();
    insert_at(&body, &element("<div>new last</div>"), n);
    let s = lay(&doc);
    incremental(&s);
    assert!(s.hits >= 3, "{s:?}");
    assert_eq!(s.miss_floats, 0, "{s:?}");
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

const ROW: &str = "#fx{display:flex;height:100px} #fx > div{flex:1} #fx.g{gap:3px}";
const ROW_HTML: &str = "<div id=fx><div id=a><div>aa aa</div></div><div id=b><div>bb bb</div></div><div id=c><div>cc cc</div></div></div><section><div>x</div></section>";

#[test]
fn qinc1h2_a_full_layout_fills_the_intrinsic_cache_and_an_unchanged_one_asks_nothing() {
    let _g = LogOnly::new();
    let doc = page(ROW, ROW_HTML);
    let s = lay(&doc);
    assert!(s.full, "{s:?}");
    let s = lay(&doc);
    incremental(&s);
    assert_eq!((s.intr_hits, s.intr_misses), (0, 0), "{s:?}");
}

#[test]
fn qinc1h2_a_change_on_the_container_reuses_every_items_intrinsic_sizes() {
    let _g = LogOnly::new();
    let doc = page(ROW, ROW_HTML);
    lay(&doc);
    // The row re-runs (its gap changed) but its items are clean: their (min, max) come from the cache.
    set_attr(&by_id(&doc, "fx"), "class", "g");
    let s = lay(&doc);
    incremental(&s);
    assert!(s.intr_hits >= 3, "{s:?}");
    assert_eq!(s.intr_misses, 0, "{s:?}");
}

#[test]
fn qinc1h2_a_change_inside_one_item_recomputes_only_that_item() {
    let _g = LogOnly::new();
    let doc = page(ROW, ROW_HTML);
    lay(&doc);
    let inner = by_id(&doc, "a").children.borrow().iter().find(|c| matches!(c.data, NodeData::Element { .. })).cloned().unwrap();
    set_text(&inner, "aa aa aa aa aa");
    let s = lay(&doc);
    incremental(&s);
    assert!(s.intr_hits >= 2, "#b and #c are served from the cache: {s:?}");
    assert!(s.intr_misses >= 1 && s.intr_misses <= 3, "only #a (and its inner box) are computed: {s:?}");
}

#[test]
fn qinc1h2_a_paint_only_change_under_an_intrinsic_reader_stops_at_the_box() {
    let _g = LogOnly::new();
    let doc = page(
        "#fx{display:flex} #w{display:inline-block}",
        "<div id=fx><div id=w><div id=p>some words</div></div><div>other</div></div><section><div>x</div></section>",
    );
    lay(&doc);
    set_attr(&by_id(&doc, "p"), "style", "color:red");
    let s = lay(&doc);
    incremental(&s);
    assert!(!s.root, "{s:?}");
    assert!(s.splices >= 1 && s.intr_stops >= 1, "the natural widths did not change: {s:?}");
    assert_eq!(s.levels, vec![0], "{s:?}");
}

#[test]
fn qinc1h2_a_text_change_that_changes_the_natural_width_goes_up() {
    let _g = LogOnly::new();
    let doc = page(
        "#fx{display:flex} #w{display:inline-block}",
        "<div id=fx><div id=w><div id=p>some words</div></div><div>other</div></div><section><div>x</div></section>",
    );
    lay(&doc);
    set_text(&by_id(&doc, "p"), "some more words than there were");
    let s = lay(&doc);
    incremental(&s);
    assert_eq!(s.intr_stops, 0, "{s:?}");
}

/// QINC1k1 (R60, QINC1k finding): a box whose auto height is the sum of fractional children is reused
/// (shifted) when content above it moves it far down; its `inset:0; height:100%` child resolves the
/// percentage against that height. Summed in document space the height depended on the box's y (f32 ulp
/// ~0.0005 above y 4096), so the shifted box and the from-scratch one differed in `style.height` (the
/// shadow comparison panics on a difference). `container` is the CSS of the box holding the 20 rows.
fn y_drift_case(container: &str) {
    let _g = LogOnly::new();
    let rows: String = (1..=20).map(|i| format!("<div class=row>{i}</div>")).collect();
    let doc = page(
        &format!(
            "#spacer{{height:100px}} .rel{{position:relative;{container}}} .row{{height:32.1px}} \
             .bg{{position:absolute;inset:0;height:100%}}"
        ),
        &format!("<div id=spacer></div><div class=rel><div class=bg></div>{rows}</div><div>after</div>"),
    );
    lay(&doc);
    let spacer = by_id(&doc, "spacer");
    for h in ["4100.3", "4100.3", "4300.7", "100", "8191.9", "12345.6", "100"] {
        set_attr(&spacer, "style", &format!("height:{h}px"));
        let s = lay(&doc);
        incremental(&s);
    }
}

#[test]
fn qinc1k1_a_shifted_auto_height_block_keeps_its_percentage_child_exact() {
    y_drift_case("");
}

#[test]
fn qinc1k1_y_drift_flex_column() {
    y_drift_case("display:flex;flex-direction:column");
}

#[test]
fn qinc1k1_y_drift_flex_row() {
    y_drift_case("display:flex");
}

#[test]
fn qinc1k1_y_drift_flex_row_wrapping() {
    // 3 rows of 100px per 333px line: 7 lines of 32.1px.
    y_drift_case("display:flex;flex-wrap:wrap;width:333px} .rel > .row{width:100px");
}

#[test]
fn qinc1k1_y_drift_grid() {
    y_drift_case("display:grid");
}

#[test]
fn qinc1k1_y_drift_table() {
    y_drift_case("display:table");
}

/// QINC1k1: a BFC root holding floats places them in document space (`Floats`), so its height is not
/// translation-invariant in y: the engine re-measures it when it moves (`miss_y`) instead of shifting it,
/// and the `height:100%` child of the positioned BFC root stays exact. At the same y it is still reused.
#[test]
fn qinc1k1_a_bfc_root_with_floats_is_reused_only_at_the_same_y() {
    let _g = LogOnly::new();
    let floats: String = (1..=7).map(|i| format!("<div class=fl>{i}</div>")).collect();
    let doc = page(
        "#spacer{height:100px} .rel{position:relative;display:flow-root} .fl{float:left;width:30px;height:32.1px} \
         .fl:nth-child(4n){clear:left} .bg{position:absolute;inset:0;height:100%}",
        &format!("<div id=spacer></div><div class=rel><div class=bg></div>{floats}</div><div id=after>after</div>"),
    );
    lay(&doc);
    let spacer = by_id(&doc, "spacer");
    let mut miss_y = 0;
    for h in ["4100.3", "4300.7", "100", "8191.9"] {
        set_attr(&spacer, "style", &format!("height:{h}px"));
        let s = lay(&doc);
        incremental(&s);
        miss_y += s.miss_y;
    }
    assert!(miss_y >= 4, "re-measured on every move: {miss_y}");
    // A change below it does not move it: reused.
    set_attr(&by_id(&doc, "after"), "style", "height:50px");
    let s = lay(&doc);
    incremental(&s);
    assert_eq!(s.miss_y, 0, "{s:?}");
}

/// QINC1k3 (fuzz seed 5832): a BFC root beside a float is placed by its parent from its min-content width
/// (Q34b: it drops below the float when it does not fit beside it), so a change inside it never stops
/// at it while floats come in, even when its own size would be unchanged.
#[test]
fn qinc1k3_bfc_root_beside_a_float_is_placed_by_its_parent() {
    let _g = LogOnly::new();
    let doc = page(
        "#f{float:left;width:300px;height:100px} #b{overflow:hidden;height:40px}",
        "<div id=f></div><div id=b><span id=t>short</span></div><p>after</p>",
    );
    lay(&doc);
    set_text(&by_id(&doc, "t"), &"x".repeat(120));
    let s = lay(&doc);
    incremental(&s);
    assert!(!s.levels.contains(&0) || s.root, "{s:?}");
}
