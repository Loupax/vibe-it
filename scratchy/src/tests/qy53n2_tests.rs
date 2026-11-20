// QY53n2: a hover change that only adds/removes paint-only rules is patched into the
// installed Laid tree and shape cache; the patched frame must paint exactly like a full
// rebuild, and anything else must fall back (Err, nothing changed).
#[allow(unused_imports)]
use super::*;

const W: u32 = 300;
const H: u32 = 200;

fn find_id(h: &Handle, id: &str) -> Option<Handle> {
    if element_attr(h, "id").is_some_and(|v| &*v == id) {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_id(c, id))
}

fn hover_state(h: &Handle) -> InteractionState {
    InteractionState { hover: ancestor_ids(h).into_iter().collect(), ..Default::default() }
}

/// An App-like build under the installed interaction state: keeps the match memo,
/// measures, returns the tree and its shape cache.
fn build(doc: &Handle, fs: &mut FontSystem) -> (Laid, ShapeCache) {
    crate::style::KEEP_MATCH.with(|k| k.set(true));
    let tree = build_layout_tree_with_resources(doc, W as f32, Some(H as f32), "", &HashMap::new());
    crate::style::KEEP_MATCH.with(|k| k.set(false));
    let mut cache = ShapeCache::new();
    let (mut next_id, mut incomplete) = (0usize, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let laid = measure_document(&tree, fs, W as f32, H as f32, &mut next_id, &mut cache, deadline, &mut incomplete);
    assert!(!incomplete);
    (laid, cache)
}

fn render(laid: &Laid, cache: &mut ShapeCache, fs: &mut FontSystem) -> Vec<u32> {
    let mut swash = SwashCache::new();
    let mut pixels = vec![0xFFFF_FFFFu32; (W * H) as usize];
    paint(laid, fs, &mut swash, cache, &HashMap::new(), &mut pixels, W, H, 0.0, None);
    pixels
}

/// Hover change of `#t` from `from` to `to` on the installed tree: the diff and the patch result.
fn patch(
    doc: &Handle,
    laid: &mut Laid,
    cache: &mut ShapeCache,
    fs: &mut FontSystem,
    from: &InteractionState,
    to: &InteractionState,
) -> Result<(), &'static str> {
    let keys = HOVER_KEYS.with(|k| k.borrow().clone());
    let changed: Vec<Handle> = from.hover.symmetric_difference(&to.hover).filter_map(|&id| find_node_by_id(doc, id)).collect();
    set_interaction(to);
    let diff = crate::style::hover_match_diff(&keys, &changed).ok_or("no diff")?;
    crate::hover_patch::paint_patch(doc, laid, cache, fs, &diff, from, to)
}

/// Enter and leave `#t` by patching; each patched frame must equal a full rebuild's.
/// Returns the enter patch's result (Err = fell back; then nothing may have changed).
fn enter_leave(css: &str, body: &str) -> Result<(), &'static str> {
    let doc = parse_html(&format!(
        "<html><head><style>body{{margin:0;font-size:16px}}{css}</style></head><body>{body}</body></html>"
    ));
    let t = find_id(&doc, "t").expect("#t");
    let (off, on) = (InteractionState::default(), hover_state(&t));
    let mut fs = new_font_system();
    set_interaction(&off);
    let (mut laid, mut cache) = build(&doc, &mut fs);
    let before = render(&laid, &mut cache, &mut fs);
    let r = patch(&doc, &mut laid, &mut cache, &mut fs, &off, &on);
    let patched = render(&laid, &mut cache, &mut fs);
    if let Err(e) = r {
        assert_eq!(patched, before, "a failed patch ({e}) changed the frame");
        set_interaction(&InteractionState::default());
        return Err(e);
    }
    // Leave again from the patched state before any rebuild (the kept memo
    // must now hold the hovered sets, else this would be a "no change" skip).
    let r2 = patch(&doc, &mut laid, &mut cache, &mut fs, &on, &off);
    assert_eq!(r2, Ok(()), "leaving a patched hover patches back");
    assert!(render(&laid, &mut cache, &mut fs) == before, "leave: patched frame differs from the unhovered build");
    // Enter again, then compare with a real build under the hover.
    patch(&doc, &mut laid, &mut cache, &mut fs, &off, &on).expect("re-enter");
    let repatched = render(&laid, &mut cache, &mut fs);
    assert!(repatched == patched);
    set_interaction(&on);
    let (rebuilt, mut rcache) = build(&doc, &mut fs);
    let full = render(&rebuilt, &mut rcache, &mut fs);
    set_interaction(&InteractionState::default());
    assert!(full != before, "the hover rule must change the frame");
    let diffs = patched.iter().zip(&full).filter(|(a, b)| a != b).count();
    assert_eq!(diffs, 0, "{css}: patched frame differs from a full rebuild in {diffs} pixels");
    Ok(())
}

#[test]
fn qy53n2_link_colour_and_underline_patch_equals_rebuild() {
    assert_eq!(
        enter_leave(
            "a:hover{color:red;text-decoration:underline}",
            "<p>Some text with <a id=t href=/x>a link <b>bold</b> inside</a> and more text that wraps onto a second line.</p>"
        ),
        Ok(())
    );
}

#[test]
fn qy53n2_paint_patch_cases_equal_rebuild() {
    // A block whose colour inherits into nested inline text.
    assert_eq!(enter_leave("#t:hover{color:#00f}", "<div id=t>one <i>two</i> <span>three</span></div><p>other</p>"), Ok(()));
    // An inline background (ambient span background).
    assert_eq!(enter_leave("a:hover{background-color:#ff0}", "<p>x <a id=t href=/>link</a> y</p>"), Ok(()));
    // A descendant rule: the subject is below the hovered element.
    assert_eq!(enter_leave(".x:hover .y{color:green}", "<div id=t class=x><p>a <span class=y>b</span></p></div>"), Ok(()));
    // A block box's own background and border colour.
    assert_eq!(
        enter_leave("#t{border:3px solid #000;height:40px}#t:hover{background-color:#0f0;border-color:#f00}", "<div id=t>box</div>"),
        Ok(())
    );
    // `::before`/`::after`: their own paint-only rules, and inherited colour.
    assert_eq!(enter_leave(".x:hover::after{color:red}.x::after{content:'z'}", "<p class=x id=t>a</p>"), Ok(()));
    assert_eq!(enter_leave("a:hover{color:red}a::before{content:'!'}", "<p><a id=t href=/>link</a> x</p>"), Ok(()));
    // A block link with a generated bar below it (Wikipedia's tabs).
    assert_eq!(
        enter_leave(
            "a{display:block;color:#36c}a::after{content:'';display:block;height:3px}a:hover{color:#447}a:hover::after{background-color:#447}",
            "<div><a id=t href=/>Talk</a></div>"
        ),
        Ok(())
    );
    // A generated box of an inline element, and an absolutely positioned one
    // (it lands in an ancestor's `positioned` list, away from its element).
    assert_eq!(
        enter_leave("a::after{content:'';display:inline-block;width:5px;height:5px}a:hover::after{background-color:red}", "<p><a id=t href=/>link</a></p>"),
        Ok(())
    );
    assert_eq!(
        enter_leave(
            "li{position:relative;display:block}a::after{content:'';position:absolute;left:0;right:0;bottom:0;height:2px}a:hover{color:#447}a:hover::after{background-color:#447}",
            "<ul><li><a id=t href=/>Edit</a></li></ul>"
        ),
        Ok(())
    );
    // Colour through var().
    assert_eq!(enter_leave(":root{--h:#c00}a:hover{color:var(--h)}", "<p><a id=t href=/>link</a></p>"), Ok(()));
}

#[test]
fn qy53n2_non_paint_changes_fall_back() {
    // Layout-changing rule.
    assert!(enter_leave("a:hover{padding:5px}", "<p><a id=t href=/>link</a></p>").is_err());
    assert!(enter_leave("a:hover{font-weight:bold}", "<p><a id=t href=/>link</a></p>").is_err());
    // A `::marker` rule, and a list item's marker inheriting the colour.
    assert!(enter_leave("li:hover::marker{color:red}", "<ul><li id=t>a</li></ul>").is_err());
    assert!(enter_leave("li:hover{color:red}", "<ul><li id=t>a</li></ul>").is_err());
    // An animated element.
    assert!(enter_leave(
        "@keyframes k{from{color:red}to{color:blue}}a{animation:k 1s infinite}a:hover{color:green}",
        "<p><a id=t href=/>link</a></p>"
    )
    .is_err());
    // An offset-only rule is not patched (yet: QY53n3).
    assert!(enter_leave("#t{position:relative}#t:hover{top:2px}", "<div id=t>a</div>").is_err());
}

#[test]
fn qy53n2_effect_classes() {
    use crate::style::{decls_effect, Effect};
    let e = |css: &str| decls_effect(&parse_declarations_split(css).0);
    assert_eq!(e("color:red;text-decoration:underline"), Effect::Paint);
    assert_eq!(e("background-color:#fff;border-color:red;outline-color:blue"), Effect::Paint);
    assert_eq!(e("color:var(--x)"), Effect::Paint);
    assert_eq!(e("color:inherit"), Effect::Paint);
    assert_eq!(e("transform:translate(2px,0)"), Effect::Offset);
    assert_eq!(e("color:red;top:1px"), Effect::Offset);
    assert_eq!(e("padding:5px"), Effect::Layout);
    assert_eq!(e("--x:1px"), Effect::Layout);
    assert_eq!(e("width:var(--x)"), Effect::Layout);
    assert_eq!(e("opacity:.5"), Effect::Layout);
    assert_eq!(e("visibility:hidden"), Effect::Layout);
}
