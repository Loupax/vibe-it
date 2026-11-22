//! QCSS-A: SVG paint properties (`fill`, `stroke`, `stroke-width`, `stroke-linecap`, `stroke-linejoin`,
//! `fill-opacity`, `stroke-opacity`, `currentColor`) from stylesheets reach inline SVG painting, with
//! inheritance, and presentation attributes keep lower priority than CSS.
use super::*;
use crate::incr_box;
use crate::incr_log::{self, Change, Val};
use crate::incr_style::{self, Seam};

fn page(css: &str, body: &str) -> Handle {
    parse_html(&format!("<html><head><style>{css}</style></head><body style='margin:0'>{body}</body></html>"))
}

fn images(b: &LayoutBox, out: &mut Vec<Arc<DecodedImage>>) {
    match &b.content {
        BoxContent::Image(i) => out.push(i.clone()),
        BoxContent::Inline(items) => {
            for i in items {
                match i {
                    InlineItem::Image { image, .. } => out.push(image.clone()),
                    InlineItem::Embedded(b) => images(b, out),
                    _ => {}
                }
            }
        }
        BoxContent::Blocks(bs) | BoxContent::TableRow(bs) => bs.iter().for_each(|b| images(b, out)),
    }
}

/// The pixel at (x, y) of the first inline SVG of `doc`.
fn px(doc: &Handle, x: usize, y: usize) -> TextColor {
    let tree = build_layout_tree(doc, 400.0);
    let mut v = Vec::new();
    images(&tree, &mut v);
    let i = v.first().expect("an svg image");
    i.pixels[y * i.width as usize + x]
}

const GREEN: TextColor = TextColor::rgba(0, 255, 0, 255);
const RED: TextColor = TextColor::rgba(255, 0, 0, 255);
const BLUE: TextColor = TextColor::rgba(0, 0, 255, 255);
const BLACK: TextColor = TextColor::rgba(0, 0, 0, 255);

fn svg(inner: &str) -> String {
    format!(r#"<svg width="8" height="8" viewBox="0 0 8 8">{inner}</svg>"#)
}

#[test]
fn qcss_a_stylesheet_fill_reaches_the_svg() {
    let d = page(".i{fill:#00ff00}", &svg(r#"<rect class="i" width="8" height="8"/>"#));
    assert_eq!(px(&d, 4, 4), GREEN);
    // Control: without the rule the shape is black.
    let d = page("", &svg(r#"<rect class="i" width="8" height="8"/>"#));
    assert_eq!(px(&d, 4, 4), BLACK);
}

#[test]
fn qcss_a_fill_inherits_from_html_ancestors_and_the_svg_root() {
    let d = page("div{fill:#0000ff}", &format!("<div>{}</div>", svg(r#"<g><rect width="8" height="8"/></g>"#)));
    assert_eq!(px(&d, 4, 4), BLUE);
    let d = page("svg{fill:#ff0000}", &svg(r#"<g><rect width="8" height="8"/></g>"#));
    assert_eq!(px(&d, 4, 4), RED);
}

#[test]
fn qcss_a_css_beats_presentation_attributes() {
    let d = page("rect{fill:#00ff00}", &svg(r#"<rect fill="red" width="8" height="8"/>"#));
    assert_eq!(px(&d, 4, 4), GREEN);
    // The attribute still beats inheritance, and still applies without CSS.
    let d = page("svg{fill:#00ff00}", &svg(r#"<rect fill="red" width="8" height="8"/>"#));
    assert_eq!(px(&d, 4, 4), RED);
    let d = page("", &svg(r#"<rect fill="red" width="8" height="8"/>"#));
    assert_eq!(px(&d, 4, 4), RED);
    // `!important` and inline style order.
    let d = page("rect{fill:#00ff00 !important}", &svg(r#"<rect style="fill:red" width="8" height="8"/>"#));
    assert_eq!(px(&d, 4, 4), GREEN);
    let d = page("rect{fill:#00ff00}", &svg(r#"<rect style="fill:red" width="8" height="8"/>"#));
    assert_eq!(px(&d, 4, 4), RED);
}

#[test]
fn qcss_a_current_color_resolves_against_color() {
    let d = page("svg{color:#ff0000} path{fill:currentColor}", &svg(r#"<path d="M0 0H8V8H0z"/>"#));
    assert_eq!(px(&d, 4, 4), RED);
    // `fill: currentColor` on an ancestor, `color` on another element in between.
    let d = page("div{fill:currentColor;color:#0000ff}", &format!("<div>{}</div>", svg(r#"<path d="M0 0H8V8H0z"/>"#)));
    assert_eq!(px(&d, 4, 4), BLUE);
}

#[test]
fn qcss_a_stroke_width_and_none_fill() {
    let d = page(
        "rect{fill:none;stroke:#ff0000;stroke-width:2}",
        &svg(r#"<rect x="1" y="1" width="6" height="6"/>"#),
    );
    assert_eq!(px(&d, 4, 4).a(), 0, "the fill is none");
    assert_eq!(px(&d, 1, 4), RED, "the stroke is painted");
    // A wider stroke from the same rule through `var()`.
    let d = page(
        ":root{--w:3px} rect{fill:none;stroke:#ff0000;stroke-width:var(--w)}",
        &svg(r#"<rect x="1" y="1" width="6" height="6"/>"#),
    );
    assert_eq!(px(&d, 1, 4), RED);
}

#[test]
fn qcss_a_opacity_linecap_and_linejoin() {
    let d = page("rect{fill:#00ff00;fill-opacity:.5}", &svg(r#"<rect width="8" height="8"/>"#));
    let p = px(&d, 4, 4);
    assert!(p.a() > 100 && p.a() < 150, "half transparent: {}", p.a());
    // A square cap extends the line by half the width past its end (butt does not).
    let line = r#"<line x1="3" y1="4" x2="5" y2="4"/>"#;
    let d = page("line{stroke:#ff0000;stroke-width:2;stroke-linecap:square}", &svg(line));
    assert_eq!(px(&d, 2, 4), RED);
    let d = page("line{stroke:#ff0000;stroke-width:2;stroke-linecap:butt}", &svg(line));
    assert_eq!(px(&d, 2, 4).a(), 0);
    // A mitre join fills the outer corner; a bevel cuts it.
    let path = r#"<path d="M1 7V1H7" />"#;
    let d = page("path{fill:none;stroke:#ff0000;stroke-width:2;stroke-linejoin:miter}", &svg(path));
    assert_eq!(px(&d, 0, 0), RED);
    let d = page("path{fill:none;stroke:#ff0000;stroke-width:2;stroke-linejoin:bevel}", &svg(path));
    assert!(px(&d, 0, 0).a() < 200, "bevel cuts the corner");
}

#[test]
fn qcss_a_css_wide_keywords_and_invalid_values() {
    let d = page("div{fill:#0000ff} rect{fill:inherit}", &format!("<div>{}</div>", svg(r#"<rect fill="red" width="8" height="8"/>"#)));
    // `inherit` on the rect reads the svg root, which inherited blue from the div.
    assert_eq!(px(&d, 4, 4), BLUE);
    let d = page("rect{fill:#00ff00;fill:bogus}", &svg(r#"<rect width="8" height="8"/>"#));
    assert_eq!(px(&d, 4, 4), GREEN, "an invalid value is dropped");
}

// ---------- the incremental engine ----------

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

fn set_attr(h: &Handle, name: &str, v: &str) {
    let old = element_attr(h, name).map(|s| s.to_string());
    crate::js_dom::set_attr(h, name, v);
    incr_log::note(node_id(h), Change::Attr { name: name.into(), old: Val::of(old.as_deref()), new: Val::of(Some(v)) });
}

fn built_pixel(doc: &Handle) -> TextColor {
    let tree = crate::images::build_layout_tree_with_resources(doc, 800.0, Some(600.0), "", &HashMap::new());
    let mut v = Vec::new();
    images(&tree, &mut v);
    v[0].pixels[4 * v[0].width as usize + 4]
}

/// The engine re-rasterizes the svg when an ancestor's `fill`/`color` or a descendant's class changes
/// (every update here is compared with a from-scratch build; a difference panics).
#[test]
fn qcss_a_engine_recascades_svg_children() {
    let _g = LogOnly::new();
    let doc = page(
        ".c{fill:#ff0000} #p{fill:#0000ff} #q{color:#00ff00} .cc{fill:currentColor}",
        &format!("<div id=p><div id=q>{}</div></div>", svg(r#"<g><rect id=r width="8" height="8"/></g>"#)),
    );
    assert_eq!(built_pixel(&doc), BLUE);
    let p = find_by_id(&doc, "p").unwrap();
    set_attr(&p, "style", "fill:#00ff00");
    assert_eq!(built_pixel(&doc), GREEN, "an ancestor's fill changed");
    set_attr(&p, "style", "");
    assert_eq!(built_pixel(&doc), BLUE);
    let r = find_by_id(&doc, "r").unwrap();
    set_attr(&r, "class", "c");
    assert_eq!(built_pixel(&doc), RED, "a descendant's class changed");
    set_attr(&r, "class", "cc");
    assert_eq!(built_pixel(&doc), GREEN, "currentColor follows the ancestor's color");
    let q = find_by_id(&doc, "q").unwrap();
    set_attr(&q, "style", "color:#0000ff");
    assert_eq!(built_pixel(&doc), BLUE, "an ancestor's color changed");
}

fn find_by_id(h: &Handle, id: &str) -> Option<Handle> {
    if matches!(h.data, NodeData::Element { .. }) && element_attr(h, "id").as_deref() == Some(id) {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_by_id(c, id))
}

