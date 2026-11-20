// QY53n1: a hover change re-matches only the elements it can affect and skips the build
// when none of their matched rule sets changed.
#[allow(unused_imports)]
use super::*;

fn find_id(h: &Handle, id: &str) -> Option<Handle> {
    if element_attr(h, "id").is_some_and(|v| &*v == id) {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_id(c, id))
}

fn build(doc: &Handle) {
    crate::style::KEEP_MATCH.with(|k| k.set(true));
    build_layout_tree_with_resources(doc, 800.0, Some(600.0), "", &HashMap::new());
    crate::style::KEEP_MATCH.with(|k| k.set(false));
}

fn hover_state(h: &Handle) -> InteractionState {
    InteractionState { hover: ancestor_ids(h).into_iter().collect(), ..Default::default() }
}

/// `(enter_skips, leave_skips)`: does hovering / un-hovering `#t` leave every affected
/// element's matched rule set unchanged (the build is skipped)?
fn hover_skips(css: &str, body: &str) -> (bool, bool) {
    let doc = parse_html(&format!("<html><head><style>{css}</style></head><body>{body}</body></html>"));
    let t = find_id(&doc, "t").expect("#t");
    let chain: Vec<Handle> = ancestor_ids(&t).into_iter().filter_map(|id| find_node_by_id(&doc, id)).collect();
    set_interaction(&InteractionState::default());
    build(&doc);
    let keys = HOVER_KEYS.with(|k| k.borrow().clone());
    set_interaction(&hover_state(&t));
    let enter = crate::style::hover_matches_unchanged(&keys, &chain);
    // Build under the hover, then leave it.
    build(&doc);
    set_interaction(&InteractionState::default());
    let leave = crate::style::hover_matches_unchanged(&keys, &chain);
    set_interaction(&InteractionState::default());
    (enter, leave)
}

#[test]
fn qy53n1_hover_that_changes_no_matched_set_skips() {
    // `.y` is the subject; nothing inside `.x` is `.y`: the hovered element's own set is unchanged.
    assert_eq!(hover_skips(".x:hover .y{color:red}", "<div id=t class=x><p>a</p></div>"), (true, true));
    // A media-gated hover rule that does not apply.
    assert_eq!(hover_skips("@media print{a:hover{color:red}}", "<a id=t href=/>a</a>"), (true, true));
    // A sibling combinator whose only candidate precedes the hovered element.
    assert_eq!(hover_skips(".x:hover + .y{color:red}", "<p class=y>b</p><div id=t class=x>a</div>"), (true, true));
}

#[test]
fn qy53n1_hover_that_changes_a_matched_set_builds() {
    assert_eq!(hover_skips("a:hover{color:red}", "<a id=t href=/>a</a>"), (false, false));
    assert_eq!(hover_skips("#t:hover{color:red}", "<div id=t>a</div>"), (false, false));
    // Descendant combinator after :hover.
    assert_eq!(hover_skips(".x:hover .y{color:red}", "<div id=t class=x><p><span class=y>a</span></p></div>"), (false, false));
    // Child combinator.
    assert_eq!(hover_skips(".x:hover > .y{color:red}", "<div id=t class=x><span class=y>a</span></div>"), (false, false));
    // Sibling combinators.
    assert_eq!(hover_skips(".x:hover + .y{color:red}", "<div id=t class=x>a</div><p class=y>b</p>"), (false, false));
    assert_eq!(hover_skips(".x:hover ~ .y{color:red}", "<div id=t class=x>a</div><i></i><p class=y>b</p>"), (false, false));
    // Sibling then descendant.
    assert_eq!(hover_skips(".x:hover + .y span{color:red}", "<div id=t class=x>a</div><p class=y><span>b</span></p>"), (false, false));
    // Hover on an ancestor of the pointer's element.
    assert_eq!(hover_skips("li:hover{color:red}", "<ul><li><b id=t>a</b></li></ul>"), (false, false));
}

#[test]
fn qy53n1_nested_hover_is_unbounded() {
    assert_eq!(hover_skips(":not(.z):hover{color:red}", "<div id=t>a</div>"), (false, false));
    // A `:hover` behind a combinator inside `:not()` reaches other elements (`:has()` is not parsed at all).
    assert_eq!(hover_skips("p:not(.a :hover){color:red}", "<p><b id=t>a</b></p>"), (false, false));
}

#[test]
fn qy53n1_not_hover_is_keyed_by_its_compound() {
    // `.x:not(:hover)` is the hovered element's own state: bounded, and exact.
    assert_eq!(hover_skips(".x:not(:hover) .y{color:red}", "<div id=t class=x><p>a</p></div>"), (true, true));
    assert_eq!(hover_skips(".x:not(:hover) .y{color:red}", "<div id=t class=x><p class=y>a</p></div>"), (false, false));
    assert_eq!(hover_skips(".x:not(:hover){color:red}", "<div id=t class=x>a</div>"), (false, false));
    assert_eq!(hover_skips(".x:is(:hover){color:red}", "<div id=t class=x>a</div>"), (false, false));
}

#[test]
fn qy53n1_stale_memo_never_skips() {
    let doc = parse_html("<html><head><style>.x:hover .y{color:red}</style></head><body><div id=t class=x>a</div></body></html>");
    let t = find_id(&doc, "t").unwrap();
    let chain: Vec<Handle> = ancestor_ids(&t).into_iter().filter_map(|id| find_node_by_id(&doc, id)).collect();
    set_interaction(&InteractionState::default());
    // A build that does not keep its memo leaves nothing to compare with.
    build_layout_tree_with_resources(&doc, 800.0, Some(600.0), "", &HashMap::new());
    let keys = HOVER_KEYS.with(|k| k.borrow().clone());
    set_interaction(&hover_state(&t));
    assert!(!crate::style::hover_matches_unchanged(&keys, &chain));
    set_interaction(&InteractionState::default());
}

/// QY53n1c: a build that is not the App's (a script's forced layout) between the App build and
/// the hover keeps the App's match memo: the hover still patches, and the patch equals a rebuild.
#[test]
fn qy53n1c_forced_layout_keeps_hover_patching() {
    use super::qy53n2_tests as n2;
    let doc = parse_html(
        "<html><head><style>body{margin:0;font-size:16px}a:hover{color:red}</style></head>\
         <body><p>x <a id=t href=/>link</a> y</p></body></html>",
    );
    let t = n2::find_id(&doc, "t").unwrap();
    let (off, on) = (InteractionState::default(), n2::hover_state(&t));
    let mut fs = new_font_system();
    set_interaction(&off);
    let (mut laid, mut cache) = n2::build(&doc, &mut fs);
    let before = n2::render(&laid, &mut cache, &mut fs);
    // What `GeomCache` does for a geometry read: a plain build whose memo is not kept.
    build_layout_tree_with_resources(&doc, 640.0, Some(480.0), "", &HashMap::new());
    assert_eq!(n2::patch(&doc, &mut laid, &mut cache, &mut fs, &off, &on), Ok(()), "the hover is patched, not built");
    let patched = n2::render(&laid, &mut cache, &mut fs);
    set_interaction(&on);
    let (rebuilt, mut rcache) = n2::build(&doc, &mut fs);
    let full = n2::render(&rebuilt, &mut rcache, &mut fs);
    set_interaction(&off);
    assert!(full != before, "the hover rule must change the frame");
    assert!(patched == full, "patched frame differs from a full rebuild");
}

/// QY53n1c: a `<style>` change after the App build makes the kept memo unusable, also when the
/// spliced list keeps its pointer and length (one rule for one) and a forced layout re-derived
/// its index; and when the splice adds a matching `:hover` rule. The hover then builds.
#[test]
fn qy53n1c_style_splice_after_build_falls_back_to_build() {
    use super::qy53n2_tests as n2;
    for (id, text, added) in [("s", "a{color:green}", 0usize), ("s2", "a:hover{color:green}", 1)] {
        let doc = parse_html(
            "<html><head><style>a:hover{color:red}</style><style id=s>.u{color:blue}</style><style id=s2>/**/</style></head>\
             <body><p>x <a id=t href=/>link</a> y</p></body></html>",
        );
        let t = n2::find_id(&doc, "t").unwrap();
        let (off, on) = (InteractionState::default(), n2::hover_state(&t));
        let mut fs = new_font_system();
        set_interaction(&off);
        let _ = n2::build(&doc, &mut fs);
        let keys = HOVER_KEYS.with(|k| k.borrow().clone());
        let n0 = crate::rules_cache::rule_count();
        let s = n2::find_id(&doc, id).unwrap();
        let node = s.children.borrow()[0].clone();
        if let NodeData::Text { contents } = &node.data {
            *contents.borrow_mut() = text.into();
        }
        build_layout_tree_with_resources(&doc, 640.0, Some(480.0), "", &HashMap::new());
        assert_eq!(crate::rules_cache::rule_count(), n0 + added);
        let changed: Vec<Handle> = on.hover.iter().filter_map(|&id| find_node_by_id(&doc, id)).collect();
        set_interaction(&on);
        assert!(crate::style::hover_match_diff(&keys, &changed).is_none(), "{text}: a stale memo must not be used");
        set_interaction(&off);
    }
}
