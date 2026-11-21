//! QINC1f (R60 sections 4.1, 5, 6): the incremental restyle in shadow mode. Every build in the suite
//! already compares the engine with the from-scratch styles (a mismatch panics inside the build); these
//! tests feed the engine through the write log ONLY (the check-mode reconcile is off) and pin how much
//! it redoes: one test per row of R60 section 5's table, the early cut-off, custom properties, `rem`,
//! counters, animation membership and node address reuse.
use super::*;
use crate::incr_log::{self, Change, Val};
use crate::incr_style::{self, Seam, Stats};

/// Log-only engine for the test's duration (panics on a mismatch stay on).
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
    build_vh(doc, 600.0)
}

fn build_vh(doc: &Handle, vh: f32) -> Stats {
    let _ = build_layout_tree_sized(doc, 800.0, vh);
    incr_style::with_engine(|e| e.expect("shadow engine").stats.clone())
}

fn page(css: &str, body: &str) -> Handle {
    parse_html(&format!("<html><head><style>{css}</style></head><body>{body}</body></html>"))
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
    incr_log::note(node_id(h), Change::Attr { name: name.into(), old: Val::of(old.as_deref()), new: Val::of(v) });
}

fn append(parent: &Handle, child: &Handle) {
    child.parent.set(Some(Rc::downgrade(parent)));
    parent.children.borrow_mut().push(child.clone());
    let (prev, next) = incr_log::siblings(child);
    incr_log::note(node_id(parent), Change::ChildInsert { child: node_id(child), prev, next });
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

fn style_of(h: &Handle) -> ComputedStyle {
    incr_style::with_engine(|e| *e.unwrap().entry(h).expect("styled").style().unwrap())
}

const KIDS: &str = "<div id=p><span id=a>a</span><span id=b>b</span><span id=c>c</span></div><div id=q><i>x</i></div>";

// ---------- section 5, one test per row ----------

#[test]
fn qinc1f_first_build_is_full_and_a_clean_rebuild_restyles_nothing() {
    let _g = LogOnly::new();
    let doc = page("#p span{color:red}", KIDS);
    let s = build(&doc);
    assert!(s.full);
    assert!(s.restyled() >= 7, "{s:?}");
    let s = build(&doc);
    assert!(!s.full, "{s:?}");
    assert_eq!(s.restyled(), 0, "nothing changed: {s:?}");
}

#[test]
fn qinc1f_class_write_rematches_only_what_the_map_names() {
    let _g = LogOnly::new();
    let doc = page(".on span{color:red} .x{font-weight:bold}", KIDS);
    build(&doc);
    set_attr(&by_id(&doc, "p"), "class", Some("on"));
    let s = build(&doc);
    assert!(!s.full);
    // the three spans (descendant set of `.on span`); p's own match cannot change (no `.on` subject); q untouched.
    assert_eq!(s.rematched, 3, "{s:?}");
    assert_eq!(style_of(&by_id(&doc, "a")).color, TextColor::rgb(255, 0, 0));
    set_attr(&by_id(&doc, "p"), "class", None);
    let s = build(&doc);
    assert_eq!(s.rematched, 3, "{s:?}");
    assert_eq!(style_of(&by_id(&doc, "a")).color, TextColor::rgb(0, 0, 0));
}

#[test]
fn qinc1f_id_and_other_attribute_writes() {
    let _g = LogOnly::new();
    let doc = page("#z{color:blue} [data-k=\"1\"]{color:green}", KIDS);
    build(&doc);
    set_attr(&by_id(&doc, "b"), "id", Some("z"));
    let s = build(&doc);
    assert_eq!(s.rematched, 1, "{s:?}");
    let b = find_by_tag(&by_id(&doc, "p"), "span").unwrap();
    set_attr(&b, "data-k", Some("1"));
    let s = build(&doc);
    assert_eq!(s.rematched, 1, "{s:?}");
    assert_eq!(style_of(&b).color, TextColor::rgb(0, 128, 0));
    // An attribute no selector reads restyles nothing (its box is marked for the box stage).
    set_attr(&b, "title", Some("t"));
    let s = build(&doc);
    assert_eq!(s.restyled(), 0, "{s:?}");
    assert!(s.boxes.contains(&node_id(&by_id(&doc, "p"))), "the span's box owner is p: {s:?}");
}

#[test]
fn qinc1f_style_attribute_recascades_without_matching() {
    let _g = LogOnly::new();
    let doc = page("", KIDS);
    build(&doc);
    set_attr(&by_id(&doc, "a"), "style", Some("width:10px;display:inline-block"));
    let s = build(&doc);
    assert_eq!((s.rematched, s.recascaded), (0, 1), "{s:?}");
    assert_eq!(style_of(&by_id(&doc, "a")).display, Display::InlineBlock);
}

#[test]
fn qinc1f_child_insert_remove_and_move() {
    let _g = LogOnly::new();
    let doc = page("span:last-child{color:red} span+span{font-weight:bold}", KIDS);
    build(&doc);
    let p = by_id(&doc, "p");
    let n = element("<span id=n><b>new</b></span>");
    append(&p, &n);
    let s = build(&doc);
    assert!(!s.full);
    // the new span and its <b>, plus the old last child (`:last-child`)
    assert!(s.rematched >= 3 && s.rematched <= 5, "{s:?}");
    assert_eq!(style_of(&n).color, TextColor::rgb(255, 0, 0));
    assert_eq!(style_of(&by_id(&doc, "c")).color, TextColor::rgb(0, 0, 0));
    let before = incr_style::with_engine(|e| e.unwrap().len());
    remove(&n);
    let s = build(&doc);
    assert!(!s.full);
    assert_eq!(style_of(&by_id(&doc, "c")).color, TextColor::rgb(255, 0, 0));
    assert_eq!(incr_style::with_engine(|e| e.unwrap().len()), before - 2, "the removed subtree left the table");
    // move: `a` to the end
    let a = by_id(&doc, "a");
    remove(&a);
    append(&p, &a);
    let s = build(&doc);
    assert!(!s.full && s.rematched <= p.children.borrow().len() as u64 + 1, "{s:?}");
    assert_eq!(style_of(&a).color, TextColor::rgb(255, 0, 0));
}

#[test]
fn qinc1f_text_data_text_content_and_inner_html() {
    let _g = LogOnly::new();
    let doc = page("i:empty{display:block;width:5px}", "<p id=t>hello</p><i id=e></i>");
    build(&doc);
    let t = by_id(&doc, "t");
    let text = t.children.borrow()[0].clone();
    if let NodeData::Text { contents } = &text.data {
        *contents.borrow_mut() = "changed".into();
    }
    incr_log::note(node_id(&text), Change::Text);
    let s = build(&doc);
    assert_eq!(s.restyled(), 0, "text data restyles nothing here: {s:?}");
    assert!(s.boxes.contains(&node_id(&t)), "the paragraph's box is marked: {s:?}");
    // textContent on the empty <i>: it stops matching `:empty`
    let e = by_id(&doc, "e");
    let tn = parse_html("<html><body>x</body></html>");
    let x = find_by_tag(&tn, "body").unwrap().children.borrow()[0].clone();
    x.parent.set(Some(Rc::downgrade(&e)));
    e.children.borrow_mut().push(x);
    incr_log::note(node_id(&e), Change::TextContent);
    let s = build(&doc);
    assert_eq!(s.rematched, 1, "{s:?}");
    assert_eq!(style_of(&e).display, Display::Inline);
    // innerHTML replacing the children with elements
    let frag = element("<b id=in>z</b>");
    e.children.borrow_mut().clear();
    frag.parent.set(Some(Rc::downgrade(&e)));
    e.children.borrow_mut().push(frag.clone());
    incr_log::note(node_id(&e), Change::InnerHtml);
    let s = build(&doc);
    assert!(!s.full && s.rematched <= 2, "the new <b> (and <i> for `:empty`): {s:?}");
}

#[test]
fn qinc1f_form_state_checked() {
    let _g = LogOnly::new();
    let doc = page("input:checked + span{color:red}", "<input id=c type=checkbox><span id=s>s</span>");
    build(&doc);
    let c = by_id(&doc, "c");
    crate::forms::set_checked(&c, true);
    incr_log::note(node_id(&c), Change::Form(crate::incr_log::FormKind::Checked));
    let s = build(&doc);
    assert!(!s.full && s.rematched == 1, "the span after the checkbox: {s:?}");
    assert_eq!(style_of(&by_id(&doc, "s")).color, TextColor::rgb(255, 0, 0));
    crate::forms::set_checked(&c, false);
}

#[test]
fn qinc1f_hover_focus_and_target_are_state_diffs() {
    let _g = LogOnly::new();
    let doc = page("#a:hover{color:red} #b:focus{color:blue} #c:target{color:green} #q:hover i{color:red}", KIDS);
    build(&doc);
    let a = by_id(&doc, "a");
    set_interaction(&InteractionState { hover: ancestor_ids(&a).into_iter().collect(), ..Default::default() });
    let s = build(&doc);
    // a, plus the <i> of `#q:hover i` (the map keys a descendant set by its subject only: conservative)
    assert!(s.rematched >= 1 && s.rematched <= 2, "{s:?}");
    assert_eq!(style_of(&a).color, TextColor::rgb(255, 0, 0));
    let b = by_id(&doc, "b");
    let c = by_id(&doc, "c");
    set_interaction(&InteractionState { focus: Some(node_id(&b)), target: Some(node_id(&c)), ..Default::default() });
    let s = build(&doc);
    // a leaves hover, b gains focus, c becomes the target
    assert!((3..=4).contains(&s.rematched), "{s:?}");
    let q = by_id(&doc, "q");
    set_interaction(&InteractionState { hover: ancestor_ids(&q).into_iter().collect(), ..Default::default() });
    let s = build(&doc);
    assert_eq!(style_of(&find_by_tag(&q, "i").unwrap()).color, TextColor::rgb(255, 0, 0));
    assert!(!s.full, "{s:?}");
}

#[test]
fn qinc1f_new_rule_list_and_viewport_width_are_full_updates() {
    let _g = LogOnly::new();
    let doc = page("span{color:red}", KIDS);
    build(&doc);
    let style = find_by_tag(&doc, "style").unwrap();
    let text = style.children.borrow()[0].clone();
    if let NodeData::Text { contents } = &text.data {
        *contents.borrow_mut() = "span{color:blue}".into();
    }
    incr_log::note(node_id(&text), Change::Text);
    let s = build(&doc);
    assert!(s.full, "a new rule-list generation re-matches everything: {s:?}");
    let _ = build_layout_tree_sized(&doc, 500.0, 600.0);
    assert!(incr_style::with_engine(|e| e.unwrap().stats.full), "viewport width");
}

#[test]
fn qinc1f_viewport_height_recascades_only_vh_readers() {
    let _g = LogOnly::new();
    let doc = page("#a{display:block;height:10vh}", KIDS);
    build_vh(&doc, 600.0);
    let s = build_vh(&doc, 700.0);
    assert!(!s.full);
    assert_eq!((s.rematched, s.recascaded), (0, 1), "{s:?}");
}

#[test]
fn qinc1f_unknown_write_is_global() {
    let _g = LogOnly::new();
    let doc = page("", KIDS);
    build(&doc);
    incr_log::note(0, Change::Global);
    assert!(build(&doc).full);
}

// ---------- early cut-off, custom properties, rem ----------

#[test]
fn qinc1f_inherited_change_recascades_children_a_width_change_does_not() {
    let _g = LogOnly::new();
    let doc = page("", KIDS);
    build(&doc);
    let p = by_id(&doc, "p");
    set_attr(&p, "style", Some("color:red"));
    let s = build(&doc);
    assert_eq!((s.rematched, s.recascaded), (0, 4), "p and its three spans: {s:?}");
    assert_eq!(style_of(&by_id(&doc, "b")).color, TextColor::rgb(255, 0, 0));
    set_attr(&p, "style", Some("color:red;width:100px"));
    let s = build(&doc);
    assert_eq!((s.rematched, s.recascaded), (0, 1), "a non-inherited change stops at p: {s:?}");
    // ... unless a child reads the parent through `inherit`
    set_attr(&by_id(&doc, "c"), "style", Some("width:inherit;display:inline-block"));
    build(&doc);
    set_attr(&p, "style", Some("color:red;width:120px"));
    let s = build(&doc);
    assert_eq!(s.recascaded, 2, "p and the `inherit` reader: {s:?}");
}

#[test]
fn qinc1f_custom_property_change_reaches_var_users() {
    let _g = LogOnly::new();
    let doc = page("span{width:var(--w);display:inline-block}", KIDS);
    build(&doc);
    set_attr(&by_id(&doc, "p"), "style", Some("--w:33px"));
    let s = build(&doc);
    assert_eq!((s.rematched, s.recascaded), (0, 4), "{s:?}");
    assert_eq!(style_of(&by_id(&doc, "a")).width, Some(BoxLength::Px(33.0)));
}

#[test]
fn qinc1f_rem_follows_the_html_font_size() {
    let _g = LogOnly::new();
    let doc = page("span{display:inline-block;width:2rem}", KIDS);
    build(&doc);
    let html = find_by_tag(&doc, "html").unwrap();
    set_attr(&html, "style", Some("font-size:20px"));
    let s = build(&doc);
    assert!(!s.full);
    assert_eq!(s.rematched, 0, "only re-cascades: {s:?}");
    assert_eq!(style_of(&by_id(&doc, "a")).width, Some(BoxLength::Px(40.0)));
}

// ---------- counters, animations, address reuse ----------

#[test]
fn qinc1f_counters_follow_an_inserted_li() {
    let _g = LogOnly::new();
    let doc = page("ol{counter-reset:n} li{counter-increment:n} li::before{content:counter(n) \". \"}", "<ol id=o><li id=x>a</li><li id=y>b</li></ol>");
    build(&doc);
    let o = by_id(&doc, "o");
    let li = element("<li id=z>new</li>");
    insert_at(&o, &li, 0);
    let s = build(&doc);
    assert!(!s.full);
    // the inserted li is styled; the two old ones are not, but their generated text changed
    assert_eq!(s.rematched, 1, "{s:?}");
    for id in ["x", "y"] {
        assert!(s.boxes.contains(&node_id(&by_id(&doc, id))), "{id}: {s:?}");
    }
    let text = crate::images::GENERATED_TEXT.with(|g| g.borrow().get(&(node_id(&by_id(&doc, "y")), false)).cloned());
    assert_eq!(text.as_deref(), Some("3. "));
}

#[test]
fn qinc1f_animation_outside_the_dirty_set_keeps_its_start_time() {
    let _g = LogOnly::new();
    let doc = page("@keyframes k{from{opacity:0}to{opacity:1}} #a{animation:k 1000ms linear}", KIDS);
    set_document_clock(100.0);
    build(&doc);
    let a = by_id(&doc, "a");
    let start = crate::anim_eval::start_of(node_id(&a), false, 0, "k");
    assert_eq!(start, Some(100.0));
    set_document_clock(400.0);
    set_attr(&by_id(&doc, "c"), "style", Some("color:red"));
    let s = build(&doc);
    // c (written) and a (animated: re-cascaded on every update); a's start stands
    assert_eq!(s.recascaded, 2, "{s:?}");
    assert_eq!(crate::anim_eval::start_of(node_id(&a), false, 0, "k"), Some(100.0));
    let op = style_of(&a).opacity;
    assert!((op - 0.3).abs() < 0.01, "{op}");
    // membership ageing keeps it; dropping the animation drops it
    incr_style::with_engine(|e| e.unwrap().age_animations());
    assert_eq!(crate::anim_eval::start_of(node_id(&a), false, 0, "k"), Some(100.0));
    set_attr(&a, "style", Some("animation:none"));
    build(&doc);
    incr_style::with_engine(|e| e.unwrap().age_animations());
    assert_eq!(crate::anim_eval::start_of(node_id(&a), false, 0, "k"), None);
    set_document_clock(0.0);
}

#[test]
fn qinc1f_node_address_reuse_never_confuses_entries() {
    let _g = LogOnly::new();
    let doc = page(".k{color:red} div > .k{font-weight:bold}", "<div id=host></div>");
    build(&doc);
    let host = by_id(&doc, "host");
    let base = incr_style::with_engine(|e| e.unwrap().len());
    for i in 0..1000 {
        let cls = if i % 2 == 0 { "k" } else { "j" };
        let n = element(&format!("<span class={cls}>{i}</span>"));
        append(&host, &n);
        let s = build(&doc);
        assert!(!s.full);
        let want = if i % 2 == 0 { TextColor::rgb(255, 0, 0) } else { TextColor::rgb(0, 0, 0) };
        assert_eq!(style_of(&n).color, want, "iteration {i}");
        remove(&n);
        drop(n);
        build(&doc);
        assert_eq!(incr_style::with_engine(|e| e.unwrap().len()), base, "iteration {i}: the table stays bounded");
    }
}

#[test]
fn qinc1f_box_owner_of_inline_content_is_its_block() {
    let _g = LogOnly::new();
    let doc = page("", "<p id=p><span id=s><b id=b>x</b></span></p>");
    build(&doc);
    let owner = |id: &str| incr_style::with_engine(|e| e.unwrap().entry(&by_id(&doc, id)).unwrap().box_owner());
    assert_eq!(owner("b"), node_id(&by_id(&doc, "p")));
    assert_eq!(owner("p"), node_id(&by_id(&doc, "p")));
}

#[test]
fn qinc1f_script_writes_feed_the_engine_through_the_log() {
    // Real natives write the log; the reconcile (on, as in the suite) must find nothing unlogged.
    incr_style::reset();
    let log = run_in_html(
        "<html><head><style>.on span{color:red}</style></head><body><div id=p><span>a</span><span>b</span></div><div><i>q</i></div></body></html>",
        "var p=document.getElementById('p'); var w=p.offsetWidth; p.className='on'; w=p.offsetWidth; console.log('ok');",
    );
    assert_eq!(log, vec!["log:ok".to_string()], "{log:?}");
    let s = incr_style::with_engine(|e| e.expect("engine").stats.clone());
    assert!(!s.full, "{s:?}");
    assert_eq!(s.unlogged, 0, "{s:?}");
    assert_eq!(s.rematched, 2, "the two spans of `.on span`: {s:?}");
    incr_style::reset();
}

#[test]
fn qinc1f_a_bare_animation_duration_is_not_animation_membership() {
    // Wikipedia's reduced-motion style `* { animation-duration: ... }` made every element "animated"
    // (re-cascaded on every update) until membership required a named animation with keyframes.
    let _g = LogOnly::new();
    let doc = page("*{animation-duration:1ms;animation-iteration-count:1}", KIDS);
    build(&doc);
    set_attr(&by_id(&doc, "c"), "style", Some("color:red"));
    let s = build(&doc);
    assert_eq!((s.rematched, s.recascaded), (0, 1), "{s:?}");
}

// ---------- QINC1f-fix: the fuzz seeds above the default window (462, 519, 733, 1956, 2080, 2084, 2475) ----------

/// The minimal form of all seven seeds: one batch inserts several elements in front of the old first child (a
/// parsed fragment goes in one node at a time). Replayed against the final DOM, each insert's "first two element
/// children" edge held only new nodes, so the old first child kept its `:first-child` rule.
#[test]
fn qinc1f_fix_two_inserts_before_the_first_child_restyle_the_old_first() {
    let _g = LogOnly::new();
    let doc = page("div:first-child{color:red}", "<section id=p>t<div id=old>o</div></section>");
    build(&doc);
    assert_eq!(style_of(&by_id(&doc, "old")).color, TextColor::rgb(255, 0, 0));
    let p = by_id(&doc, "p");
    // seed 2084: after a leading text node; seeds 462/519/733/1956/2080/2475: at 0 (a text first in 462)
    insert_at(&p, &element("<i>a</i>"), 1);
    insert_at(&p, &element("<b>b</b>"), 2);
    let s = build(&doc); // the shadow check panics on a stale `:first-child`
    assert!(!s.full, "{s:?}");
    assert_ne!(style_of(&by_id(&doc, "old")).color, TextColor::rgb(255, 0, 0));
}

/// The mirror case: two appends after the old last child (two earlier siblings keep it out of the "first two" edge,
/// which would re-match it by accident).
#[test]
fn qinc1f_fix_two_appends_after_the_last_child_restyle_the_old_last() {
    let _g = LogOnly::new();
    let doc = page("span:last-child{color:red}", "<div id=p><span>f</span><span>g</span><span id=old>o</span></div>");
    build(&doc);
    assert_eq!(style_of(&by_id(&doc, "old")).color, TextColor::rgb(255, 0, 0));
    let p = by_id(&doc, "p");
    append(&p, &element("<span>a</span>"));
    append(&p, &element("<span>b</span>"));
    let s = build(&doc);
    assert!(!s.full, "{s:?}");
    assert_ne!(style_of(&by_id(&doc, "old")).color, TextColor::rgb(255, 0, 0));
}

/// `:only-child` after two appends to a single child.
#[test]
fn qinc1f_fix_two_appends_to_an_only_child() {
    let _g = LogOnly::new();
    let doc = page("span:only-child{font-weight:bold}", "<div id=p><span id=old>o</span></div>");
    build(&doc);
    assert_eq!(style_of(&by_id(&doc, "old")).font_weight, FontWeight::BOLD);
    let p = by_id(&doc, "p");
    append(&p, &element("<span>a</span>"));
    append(&p, &element("<span>b</span>"));
    build(&doc);
    assert_ne!(style_of(&by_id(&doc, "old")).font_weight, FontWeight::BOLD);
}

/// `:nth-child`, `+` and `~` after a move inside the parent plus inserts in the same batch (seed 2080's
/// `:nth-child(2n+1)` / `.c1 ~ .c3` sheet).
#[test]
fn qinc1f_fix_move_and_inserts_in_one_batch_keep_sibling_rules() {
    let _g = LogOnly::new();
    let doc = page(
        ":nth-child(2n+1){color:red} .a + .c{font-weight:bold} .b ~ .d{text-transform:uppercase}",
        "<div id=p><i id=a class=a>a</i><i id=b class=b>b</i><i id=c class=c>c</i><i id=d class=d>d</i></div>",
    );
    build(&doc);
    let p = by_id(&doc, "p");
    let b = by_id(&doc, "b");
    remove(&b);
    insert_at(&p, &b, 3); // a c d b
    insert_at(&p, &element("<i>x</i>"), 0);
    insert_at(&p, &element("<i>y</i>"), 0); // y x a c d b
    let s = build(&doc);
    assert!(!s.full, "{s:?}");
    assert_eq!(style_of(&by_id(&doc, "c")).font_weight, FontWeight::BOLD, ".a + .c now holds");
}

/// Seed 2475 (behind the first-child cause): removing an element whose node is freed at once left its
/// descendants' entries in the table, with child lists naming freed text nodes. A new text node can reuse such an
/// address; the text edit's parent lookup then found the stale entry and the edit was lost (a kept box kept the old
/// text). The removal now drops the descendants through the stored lists.
#[test]
fn qinc1f_fix_removing_a_freed_subtree_drops_its_descendant_entries() {
    let _g = LogOnly::new();
    let doc = page("", "<div id=p><a id=gone><i><span>x<b>y</b></span></i></a><div id=q>q</div></div>");
    build(&doc);
    let before = incr_style::with_engine(|e| e.unwrap().len());
    let gone = by_id(&doc, "gone");
    remove(&gone);
    drop(gone); // the last strong reference: only the table's `Weak` is left
    build(&doc);
    let after = incr_style::with_engine(|e| e.unwrap().len());
    assert_eq!(before - after, 4, "a, i, span and b leave the table ({before} -> {after})");
    // a text edit after the removal reaches its own parent
    let q = by_id(&doc, "q");
    let t = q.children.borrow()[0].clone();
    if let NodeData::Text { contents } = &t.data {
        *contents.borrow_mut() = "new text".into();
    }
    incr_log::note(node_id(&t), Change::Text);
    let s = build(&doc);
    assert!(!s.full, "{s:?}");
    assert!(s.boxes.contains(&node_id(&q)), "the edit marks q's box: {s:?}");
}

// ---------- QINC1j-ab: invalidation precision and per-update cost ----------

/// YouTube watch (R60 "QINC1j DONE"): `:hover` on `ytd-app` marked ~7 500 descendants because the map
/// never checked the compound carrying `:hover` on the element entering the state. A hover on an
/// ancestor now re-matches only what its rules can change; hovering the `.menu` itself still reaches
/// its icons.
#[test]
fn qinc1j_ab_hover_on_an_ancestor_rematches_only_what_its_rules_reach() {
    let _g = LogOnly::new();
    let mut body = String::from("<div id=app>");
    for i in 0..60 {
        body += &format!("<div class=menu id=m{i}><span class=icon>i</span><span class=label>l</span><b>x</b></div>");
    }
    body += "</div>";
    let doc = page(".menu:hover .icon{color:red} .menu:hover > b{font-weight:bold} .card:hover span{color:blue} #m3:hover .label{color:green}", &body);
    let s = build(&doc);
    assert!(s.full);
    let app = by_id(&doc, "app");
    set_interaction(&InteractionState { hover: ancestor_ids(&app).into_iter().collect(), ..Default::default() });
    let s = build(&doc);
    assert!(!s.full, "{s:?}");
    assert_eq!(s.rematched, 0, "html, body and #app carry no `.menu` / `.card` / `#m3`: nothing can change: {s:?}");
    let m3 = by_id(&doc, "m3");
    set_interaction(&InteractionState { hover: ancestor_ids(&m3).into_iter().collect(), ..Default::default() });
    let s = build(&doc);
    // #m3's icon, label and <b> (its own three rules), nothing in the other 59 menus
    assert_eq!(s.rematched, 3, "{s:?}");
    assert_eq!(style_of(&find_by_tag(&m3, "span").unwrap()).color, TextColor::rgb(255, 0, 0));
}

/// QINC1j-ab: the document-wide counter / quote walk runs only when what it reads changed (a DOM write, a
/// stored match); a hover that changes no match list leaves its output as it was. Every build compares the
/// engine's generated content with the from-scratch walk, so a skipped walk that should have run fails here.
#[test]
fn qinc1j_ab_counter_walk_only_after_a_change_it_can_see() {
    let _g = LogOnly::new();
    let doc = page(
        "ol{counter-reset:n} li{counter-increment:n} li::before{content:counter(n) \". \"} .h:hover{color:red} li.h:hover{counter-increment:n 5}",
        "<ol id=o><li id=x>a</li><li id=y class=h>b</li><li id=z>c</li></ol><p id=p class=h>p</p>",
    );
    assert!(build(&doc).gen_walked);
    // nothing changed: no walk
    let s = build(&doc);
    assert!(!s.gen_walked && s.rematched == 0, "{s:?}");
    // a hover on elements whose rules cannot change (html, body, the ol): nothing re-matched, no walk
    let o = by_id(&doc, "o");
    set_interaction(&InteractionState { hover: ancestor_ids(&o).into_iter().collect(), ..Default::default() });
    let s = build(&doc);
    assert!(s.rematched == 0 && !s.gen_walked, "{s:?}");
    // QINC1k3: a hover that changes a match list but nothing the walk reads (`gsig`: `color` only) does
    // not walk either
    let p = by_id(&doc, "p");
    set_interaction(&InteractionState { hover: ancestor_ids(&p).into_iter().collect(), ..Default::default() });
    let s = build(&doc);
    assert!(s.rematched >= 1 && !s.gen_walked, "{s:?}");
    // hovering the li changes the counters: walked, and the next li's text follows
    let y = by_id(&doc, "y");
    set_interaction(&InteractionState { hover: ancestor_ids(&y).into_iter().collect(), ..Default::default() });
    let s = build(&doc);
    assert!(s.gen_walked, "{s:?}");
    let text = crate::images::GENERATED_TEXT.with(|g| g.borrow().get(&(node_id(&by_id(&doc, "z")), false)).cloned());
    assert_eq!(text.as_deref(), Some("7. "), "x 1, y 1 + 5, z 7");
}

// ---------- QINC1i4: a restart adopts the from-scratch build's styles ----------

fn set_style_text(doc: &Handle, css: &str) {
    let style = find_by_tag(doc, "style").unwrap();
    let text = style.children.borrow()[0].clone();
    if let NodeData::Text { contents } = &text.data {
        *contents.borrow_mut() = css.into();
    }
    incr_log::note(node_id(&text), Change::Text);
}

/// A restart (first build, a new rule-list generation) takes every style the build resolved, element
/// and pseudo slots, with its viewport-height read flag: the engine cascades none of them itself
/// (check mode still cascades each one to verify it, outside the `resolved` counter). Then the same
/// with the verification off, as `SCRATCHY_INCR=1` runs it; incremental updates keep resolving.
#[test]
fn qinc1i4_restart_adopts_the_builds_styles() {
    let _g = LogOnly::new();
    let css = "span{color:red} #a::before{content:'>'} #b::after{content:'<'} #c{display:block;height:10vh} li{color:blue}";
    let doc = page(css, &format!("{KIDS}<ul><li id=l>x</li></ul>"));
    let s = build(&doc);
    assert!(s.full, "{s:?}");
    assert_eq!(s.resolved, 0, "first build: every style adopted: {s:?}");
    assert!(s.adopted >= 10, "elements + ::before + ::after + ::marker: {s:?}");
    set_style_text(&doc, &css.replace("red", "green"));
    let s = build(&doc);
    assert!(s.full, "new rule-list generation: {s:?}");
    assert_eq!(s.resolved, 0, "restart: nothing re-resolved: {s:?}");
    assert_eq!(style_of(&by_id(&doc, "a")).color, TextColor::rgb(0, 128, 0));
    // the adopted viewport-height flag: a height change re-cascades #c alone
    let s = build_vh(&doc, 700.0);
    assert!(!s.full);
    assert_eq!((s.rematched, s.recascaded, s.resolved), (0, 1, 1), "{s:?}");
    // SCRATCHY_INCR=1: adopted without the verifying cascade; the box / layout shadows still compare
    let v = incr_style::set_adopt_verify(false);
    set_style_text(&doc, &css.replace("red", "navy"));
    let s = build_vh(&doc, 700.0);
    assert!(s.full && s.resolved == 0 && s.adopted >= 10, "{s:?}");
    assert_eq!(style_of(&by_id(&doc, "b")).color, TextColor::rgb(0, 0, 128));
    let s = build_vh(&doc, 640.0);
    assert_eq!((s.recascaded, s.resolved), (1, 1), "vh flag adopted without verification: {s:?}");
    set_attr(&by_id(&doc, "p"), "class", Some("x"));
    let s = build_vh(&doc, 640.0);
    assert!(!s.full && s.adopted == 0, "{s:?}");
    incr_style::set_adopt_verify(v);
}

/// QINC1k4: a `::before`/`::after` whose `calc()` keeps a `%` captures the viewport pair at cascade
/// time (`BoxLength::Calc`), so it reads the viewport height even when the element's own style does
/// not. The engine kept only the element slot's read flag: after a height change the pseudo kept the
/// old pair (github.com window check: `Calc(.., (956.0, 312.0)) vs (956.0, 485.0)`). Each build panics
/// on a mismatch with the from-scratch styles; covered after a restart (adopted, verified and not),
/// after an incremental re-match of the element, and with a `vh` leaf in the pseudo's calc.
#[test]
fn qinc1k4_pseudo_calc_viewport_height_change_recascades() {
    let _g = LogOnly::new();
    let css = "#a::before{content:'>';display:inline-block;width:calc(100% - 4px)} \
               #b::after{content:'<';display:inline-block;height:calc(50% + 10vh)} .x{color:red}";
    let doc = page(css, KIDS);
    let s = build_vh(&doc, 600.0);
    assert!(s.full && s.resolved == 0, "restart adopts: {s:?}");
    // height change right after the (verified) adoption: both pseudo owners re-cascade
    let s = build_vh(&doc, 312.0);
    assert!(!s.full);
    assert_eq!(s.recascaded, 2, "#a and #b re-cascade for their pseudos: {s:?}");
    // the elements re-matched incrementally (not adopted), then the height moves again
    set_attr(&by_id(&doc, "a"), "class", Some("x"));
    set_attr(&by_id(&doc, "b"), "class", Some("x"));
    let s = build_vh(&doc, 312.0);
    assert!(!s.full && s.rematched == 2, "{s:?}");
    let s = build_vh(&doc, 485.0);
    assert_eq!(s.recascaded, 2, "{s:?}");
    // SCRATCHY_INCR=1: a restart adopted without the verifying cascade keeps the pseudo's flag too
    let v = incr_style::set_adopt_verify(false);
    set_style_text(&doc, &css.replace("red", "navy"));
    let s = build_vh(&doc, 485.0);
    assert!(s.full && s.resolved == 0, "{s:?}");
    let s = build_vh(&doc, 640.0);
    assert_eq!(s.recascaded, 2, "vh flags of adopted pseudo slots: {s:?}");
    incr_style::set_adopt_verify(v);
}

// ---------- QINC1k3: the counter / quote walk runs only when what it reads changed ----------

fn gen_text(doc: &Handle, id: &str) -> Option<String> {
    crate::images::GENERATED_TEXT.with(|g| g.borrow().get(&(node_id(&by_id(doc, id)), false)).cloned())
}

/// Writes that touch nothing the walk reads (`gsig` 0: a `div` without counters / content / quotes)
/// skip it; an inserted, moved or removed `li`, a `content: attr()` attribute and a match change of a
/// counter property walk. Every build compares the engine's generated content with the from-scratch walk.
#[test]
fn qinc1k3_counter_walk_skips_writes_it_cannot_see() {
    let _g = LogOnly::new();
    let doc = page(
        "ol{counter-reset:n} li{counter-increment:n} li::before{content:counter(n) \". \"} \
         #t::after{content:attr(data-x)} .big{counter-increment:n 10}",
        "<div id=w><p id=t data-x=a>t</p></div><ol id=o><li id=x>a</li><li id=y>b</li></ol><div id=d></div>",
    );
    assert!(build(&doc).gen_walked);
    // a plain div inserted, an attribute of a plain element, a removal of a plain div: no walk
    let w = by_id(&doc, "w");
    append(&w, &element("<div class=plain><span>s</span></div>"));
    let s = build(&doc);
    assert!(!s.full && !s.gen_walked, "{s:?}");
    set_attr(&by_id(&doc, "d"), "title", Some("x"));
    let s = build(&doc);
    assert!(!s.gen_walked, "{s:?}");
    remove(&by_id(&doc, "d"));
    let s = build(&doc);
    assert!(!s.gen_walked, "{s:?}");
    // an attribute `content: attr()` reads: walked
    set_attr(&by_id(&doc, "t"), "data-x", Some("b"));
    let s = build(&doc);
    assert!(s.gen_walked, "{s:?}");
    // a new li: walked, renumbered
    let o = by_id(&doc, "o");
    insert_at(&o, &element("<li id=z>z</li>"), 0);
    let s = build(&doc);
    assert!(s.gen_walked, "{s:?}");
    assert_eq!(gen_text(&doc, "y").as_deref(), Some("3. "));
    // a moved li (an existing entry inserted elsewhere): walked
    let z = by_id(&doc, "z");
    remove(&z);
    append(&o, &z);
    let s = build(&doc);
    assert!(s.gen_walked, "{s:?}");
    assert_eq!(gen_text(&doc, "z").as_deref(), Some("3. "));
    // a class that adds a counter-increment: walked
    set_attr(&by_id(&doc, "x"), "class", Some("big"));
    let s = build(&doc);
    assert!(s.gen_walked, "{s:?}");
    assert_eq!(gen_text(&doc, "y").as_deref(), Some("11. "));
    // a removed li: walked
    remove(&by_id(&doc, "x"));
    let s = build(&doc);
    assert!(s.gen_walked, "{s:?}");
    assert_eq!(gen_text(&doc, "y").as_deref(), Some("1. "));
}

/// Writes below a `display: none` element (no entries there), or to a node without an entry, walk: the
/// walk reads matched rules only and may see under it (found by fuzz seed 380: an inline `display:none`
/// on `body`, then a removal below it). Every build compares with the from-scratch walk.
#[test]
fn qinc1k3_counter_walk_sees_under_an_inline_display_none() {
    let _g = LogOnly::new();
    let doc = page(
        "li{counter-increment:n} li::before{content:counter(n) \".\"}",
        "<div id=h style=\"display:none\"><section id=s><li id=a>a</li></section></div><ol><li id=b>b</li></ol>",
    );
    build(&doc);
    // a write below the hidden div (no entry): walked
    set_attr(&by_id(&doc, "s"), "class", Some("x"));
    let s = build(&doc);
    assert!(s.gen_walked, "{s:?}");
    // a removal below it: walked
    remove(&by_id(&doc, "s"));
    let s = build(&doc);
    assert!(s.gen_walked, "{s:?}");
    // an insertion directly under the hidden div (its entry is `display: none`): walked
    append(&by_id(&doc, "h"), &element("<li id=c>c</li>"));
    let s = build(&doc);
    assert!(s.gen_walked, "{s:?}");
}

/// QINC1k3 (fuzz seed 1752): an element hidden by an inline `display:none` keeps no `::before` match (the
/// cascade skips its pseudo slots), but the walk matches them: a class that gives it a `::before` counter
/// changes the walk's output, so its `gsig` reads those slots too.
#[test]
fn qinc1k3_counter_walk_sees_a_hidden_elements_new_before() {
    let _g = LogOnly::new();
    let doc = page(
        "li{counter-increment:n} .c4::before{content:counter(n) \".\"}",
        "<section id=s class=c2 style=\"display:none\"><i>a</i></section><ol><li id=b class=c4>b</li></ol>",
    );
    build(&doc);
    set_attr(&by_id(&doc, "s"), "class", Some("c4"));
    let s = build(&doc);
    assert!(s.gen_walked, "{s:?}");
}

#[test]
fn qinc1l_flag_parsing_default_on() {
    use crate::incr_style::flag_on;
    assert!(flag_on(None));
    assert!(flag_on(Some("1")));
    assert!(flag_on(Some("")));
    assert!(!flag_on(Some("0")));
}
