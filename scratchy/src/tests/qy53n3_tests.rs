// QY53n3: a hover change that adds/removes OFFSET-class rules (translate, top/left/right/bottom)
// is patched by shifting the affected boxes' laid subtrees; the patched tree must paint and
// measure (positions, ink) like a full rebuild, and anything layout-affecting must fall back.
#[allow(unused_imports)]
use super::*;
use super::qy53n2_tests::{build, find_id, hover_state, render};

type Dump = Vec<(usize, u8, [f32; 4], [f32; 4])>;

fn dump(l: &Laid, out: &mut Dump) {
    out.push((l.style.node, l.style.pseudo, [l.x, l.y, l.w, l.h], [l.ink.l, l.ink.t, l.ink.r, l.ink.b]));
    match &l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().for_each(|c| dump(c, out)),
        LaidContent::MixedInline(items) => {
            for it in items {
                match it {
                    MixedPositioned::Embedded(b) => dump(b, out),
                    MixedPositioned::Word { x, y, w, h, .. } => out.push((usize::MAX, 0, [*x, *y, *w, *h], [0.0; 4])),
                }
            }
        }
        _ => {}
    }
    l.positioned.iter().for_each(|c| dump(c, out));
}

fn flat(l: &Laid) -> Dump {
    let mut d = Dump::new();
    dump(l, &mut d);
    d
}

fn same_geometry(a: &Laid, b: &Laid, what: &str) {
    let (da, db) = (flat(a), flat(b));
    assert_eq!(da.len(), db.len(), "{what}: box count");
    for (x, y) in da.iter().zip(&db) {
        assert_eq!((x.0, x.1), (y.0, y.1), "{what}: order");
        for k in 0..4 {
            assert!((x.2[k] - y.2[k]).abs() < 1e-3, "{what}: node {} box {:?} vs {:?}", x.0, x.2, y.2);
            assert!((x.3[k] - y.3[k]).abs() < 1e-3, "{what}: node {} ink {:?} vs {:?}", x.0, x.3, y.3);
        }
    }
}

fn patch_moved(
    doc: &Handle,
    laid: &mut Laid,
    cache: &mut ShapeCache,
    fs: &mut FontSystem,
    from: &InteractionState,
    to: &InteractionState,
) -> Result<bool, &'static str> {
    let keys = HOVER_KEYS.with(|k| k.borrow().clone());
    let changed: Vec<Handle> = from.hover.symmetric_difference(&to.hover).filter_map(|&id| find_node_by_id(doc, id)).collect();
    set_interaction(to);
    let diff = crate::style::hover_match_diff(&keys, &changed).ok_or("no diff")?;
    crate::hover_patch::paint_patch(doc, laid, cache, fs, &diff, from, to)
}

/// Enter and leave `#t` by patching; every patched tree must equal a rebuild's (boxes, ink, pixels).
/// Returns whether the enter patch moved a box, or its refusal.
fn enter_leave(css: &str, body: &str) -> Result<bool, &'static str> {
    let doc = parse_html(&format!(
        "<html><head><style>body{{margin:0;font-size:16px}}{css}</style></head><body>{body}</body></html>"
    ));
    let t = find_id(&doc, "t").expect("#t");
    let (off, on) = (InteractionState::default(), hover_state(&t));
    let mut fs = new_font_system();
    set_interaction(&off);
    let (mut laid, mut cache) = build(&doc, &mut fs);
    let (laid_off, _) = build(&doc, &mut fs);
    let before = render(&laid, &mut cache, &mut fs);
    let r = patch_moved(&doc, &mut laid, &mut cache, &mut fs, &off, &on);
    let moved = match r {
        Ok(m) => m,
        Err(e) => {
            assert_eq!(render(&laid, &mut cache, &mut fs), before, "a failed patch ({e}) changed the frame");
            same_geometry(&laid, &laid_off, "a failed patch changed the tree");
            set_interaction(&InteractionState::default());
            return Err(e);
        }
    };
    let patched = render(&laid, &mut cache, &mut fs);
    set_interaction(&on);
    let (rebuilt, mut rcache) = build(&doc, &mut fs);
    let full = render(&rebuilt, &mut rcache, &mut fs);
    same_geometry(&laid, &rebuilt, "enter");
    let diffs = patched.iter().zip(&full).filter(|(a, b)| a != b).count();
    assert_eq!(diffs, 0, "{css}: patched frame differs from a full rebuild in {diffs} pixels");
    // Leave again from the patched state, and compare with the unhovered build.
    let r2 = patch_moved(&doc, &mut laid, &mut cache, &mut fs, &on, &off);
    assert_eq!(r2.map(|_| ()), Ok(()), "leaving a patched hover patches back");
    same_geometry(&laid, &laid_off, "leave");
    assert!(render(&laid, &mut cache, &mut fs) == before, "leave: patched frame differs from the unhovered build");
    set_interaction(&InteractionState::default());
    Ok(moved)
}

const TXT: &str = "<p>before</p><div id=t class=c>card <b>text</b></div><p>after the card</p>";

#[test]
fn qy53n3_translate_hover_equals_rebuild() {
    assert_eq!(enter_leave(".c{background:#cde}.c:hover{transform:translate(8px,6px)}", TXT), Ok(true));
    assert_eq!(enter_leave(".c{background:#cde;width:100px}.c:hover{transform:translateY(-4px)}", TXT), Ok(true));
    // A percentage translate uses the box's own (unchanged) size.
    assert_eq!(enter_leave(".c{background:#cde;width:100px;height:30px}.c:hover{transform:translate(50%,20%)}", TXT), Ok(true));
    // Changing an existing translate.
    assert_eq!(enter_leave(".c{background:#cde;transform:translateX(3px)}.c:hover{transform:translateX(20px)}", TXT), Ok(true));
}

#[test]
fn qy53n3_relative_offset_hover_equals_rebuild() {
    assert_eq!(enter_leave(".c{position:relative;background:#cde}.c:hover{top:5px;left:7px}", TXT), Ok(true));
    assert_eq!(enter_leave(".c{position:relative;background:#cde;top:2px}.c:hover{top:-6px}", TXT), Ok(true));
    assert_eq!(enter_leave(".c{position:relative;background:#cde}.c:hover{right:9px;bottom:3px}", TXT), Ok(true));
}

#[test]
fn qy53n3_absolute_offset_hover_equals_rebuild() {
    let b = "<div style='position:relative;height:120px;margin-top:10px'><div id=t class=c>abs</div></div><p>after</p>";
    assert_eq!(enter_leave(".c{position:absolute;left:10px;top:20px;width:60px;background:#cde}.c:hover{left:40px;top:30px}", b), Ok(true));
    assert_eq!(enter_leave(".c{position:absolute;right:10px;top:20px;width:60px;background:#cde}.c:hover{right:40px}", b), Ok(true));
    // With a translate as well.
    assert_eq!(enter_leave(".c{position:absolute;left:10px;top:20px;width:60px;background:#cde}.c:hover{left:20px;transform:translateX(9px)}", b), Ok(true));
}

#[test]
fn qy53n3_paint_and_offset_combined() {
    assert_eq!(
        enter_leave(".c{position:relative;background:#cde}.c:hover{background-color:#fa0;color:#c00;top:4px;transform:translateX(6px)}", TXT),
        Ok(true)
    );
    // Paint on a descendant plus a translated block.
    assert_eq!(
        enter_leave(".c{background:#eee}.c:hover{transform:translate(3px,3px)}.c:hover a{color:red}", "<div id=t class=c>x <a href=/>link</a> y</div><p>z</p>"),
        Ok(true)
    );
}

#[test]
fn qy53n3_unhandled_offsets_fall_back() {
    // An inline-block in a line: the build's inline layout places it (no translate there).
    assert!(enter_leave("#t{display:inline-block;background:#cde}#t:hover{transform:translate(5px,5px)}", "<p>a <span id=t>box</span> b</p>").is_err());
    // Layout-affecting rules.
    assert!(enter_leave(".c:hover{width:200px;transform:translateX(3px)}", TXT).is_err());
    assert!(enter_leave(".c:hover{margin-left:9px}", TXT).is_err());
    assert!(enter_leave(".c:hover{display:none}", TXT).is_err());
    assert!(enter_leave(".c{position:relative}.c:hover{top:5px;padding:4px}", TXT).is_err());
    // A percentage inset needs the containing block.
    assert!(enter_leave(".c{position:relative}.c:hover{left:10%}", TXT).is_err());
    // An absolute box with auto width: shrink-to-fit follows the insets.
    let b = "<div style='position:relative;height:120px'><div id=t class=c>abs</div></div>";
    assert!(enter_leave(".c{position:absolute;left:10px;top:20px;background:#cde}.c:hover{left:40px}", b).is_err());
    // An absolute box changing the edge it hangs from (auto -> set needs the static position).
    assert!(enter_leave(".c{position:absolute;top:20px;width:50px;background:#cde}.c:hover{left:40px}", b).is_err());
    // A positioned descendant (hoisted to the stacking context's list at build time).
    let nested = "<div id=t class=c>a<span style='position:relative;left:2px'>b</span></div><p>x</p>";
    assert!(enter_leave(".c{background:#cde}.c:hover{transform:translateX(5px)}", nested).is_err());
    // Sticky insets.
    assert!(enter_leave(".c{position:sticky;top:0}.c:hover{top:9px}", TXT).is_err());
    // An offset rule on a pseudo-element.
    assert!(enter_leave(".c::after{content:'x';position:relative}.c:hover::after{top:3px}", TXT).is_err());
    // An animated element.
    assert!(enter_leave("@keyframes k{from{opacity:.5}to{opacity:1}}.c{animation:k 1s infinite}.c:hover{transform:translateX(5px)}", TXT).is_err());
}

// QY53n2c: rules judged by the resolved value change, not the declared property.
#[test]
fn qy53n2c_value_judged_hover_rules_patch() {
    // border-radius (paint only).
    assert_eq!(enter_leave(".c{background:#cde;width:80px;height:30px}.c:hover{border-radius:12px;background-color:#fa0}", TXT), Ok(false));
    // A `background` shorthand whose computed change is only the colour, also through var().
    assert_eq!(enter_leave(".c{--b:#fa0;background:#cde}.c:hover{background:var(--b)}", TXT), Ok(false));
    assert_eq!(enter_leave(".c{background:#cde}.c:hover{background:#fa0}", TXT), Ok(false));
    // Border width and style restated with the same value; only the colour differs.
    assert_eq!(enter_leave(".c{border:2px solid #333}.c:hover{border:2px solid #c00}", TXT), Ok(false));
    assert_eq!(enter_leave(".c{border:2px solid #333}.c:hover{border-width:2px;border-style:solid;border-color:#c00}", TXT), Ok(false));
    // A custom property declared in the hover rule is not judged by value.
    assert!(enter_leave(".c{border:2px solid #333}.c:hover{border:var(--w) solid #c00;--w:2px}", TXT).is_err());
    // QCSS-A: SVG fill/stroke reach the rasterised svg, so a hover rule setting them is not patched.
    let svg = "<p>a</p><svg id=t class=c width=20 height=20><rect width=10 height=10 /></svg><p>b</p>";
    assert!(enter_leave(":root{--f:red}.c:hover{fill:var(--f);stroke:var(--f)}.c{background:#eee}.c:hover{background-color:#ddd}", svg).is_err());
    // With an offset rule in the same hover.
    assert_eq!(enter_leave(".c{background:#cde}.c:hover{border-radius:6px;transform:translateX(5px)}", TXT), Ok(true));
}

#[test]
fn qy53n2c_real_layout_changes_still_build() {
    // A real border-width change.
    assert!(enter_leave(".c{border:2px solid #333}.c:hover{border:4px solid #c00}", TXT).is_err());
    assert!(enter_leave(".c{border:2px solid #333}.c:hover{border-width:3px}", TXT).is_err());
    assert!(enter_leave(".c:hover{width:200px;border-radius:5px}", TXT).is_err());
    assert!(enter_leave(".c{--w:10px}.c:hover{--w:30px;width:var(--w)}", TXT).is_err());
    assert!(enter_leave(".c{--w:10px;width:var(--w)}.c:hover{width:30px}", TXT).is_err());
    // A background image appearing.
    assert!(enter_leave(".c:hover{background:url(data:image/gif;base64,R0lGODlhAQABAAAAACw=) #fa0}", TXT).is_err());
    // Style changes that alter the box: display, border style none, font size.
    assert!(enter_leave(".c{border:2px solid #333}.c:hover{border-style:none}", TXT).is_err());
    assert!(enter_leave(".c:hover{opacity:.5}", TXT).is_err());
    assert!(enter_leave(".c:hover{font-size:20px}", TXT).is_err());
}

#[test]
fn qy53n2c_controls_and_list_items_patch_when_their_text_paint_is_unchanged() {
    // A button whose hover only changes its background and border: its label (no element node) keeps its paint.
    let b = "<p>a</p><button id=t class=c>Go</button><p>b</p>";
    assert_eq!(enter_leave(".c{background:#cde;border:1px solid #333}.c:hover{background:#fa0;border:1px solid #c00;border-radius:6px}", b), Ok(false));
    let li = "<ul><li id=t class=c>item</li><li>two</li></ul>";
    assert_eq!(enter_leave(".c{background:#cde}.c:hover{background-color:#fa0;border-radius:4px}", li), Ok(false));
    // A colour change reaches the label / marker, which cannot be patched.
    assert!(enter_leave(".c:hover{color:#c00}", b).is_err());
    assert!(enter_leave(".c:hover{color:#c00}", li).is_err());
    assert!(enter_leave(".c:hover{text-decoration:underline}", li).is_err());
}
