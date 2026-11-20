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
