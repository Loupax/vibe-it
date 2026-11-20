//! QINC1a (R60 section 10): comparators between two style / box / laid results, with a readable
//! first-difference report (node `tag#id.class`, path, field, both values). Test-only for now: the
//! differential fuzz harness (`tests/incr_fuzz_tests.rs`) compares two from-scratch layouts; the
//! incremental engine's shadow mode (QINC1f onward) compares engine against from-scratch.
//!
//! What is deliberately NOT compared: `ComputedStyle::node`/`tree_order` (identity and resolve order,
//! R60 4.5; the `positioned` ORDER is compared instead), paragraph ids (`LaidContent::Text(id)` and
//! `ScrollInfo::id` are per-layout numbers; the shaped glyphs they point to are compared through the
//! shape caches).
#![allow(dead_code)]

use super::*;
use crate::incr_diff::style_changes;
use std::fmt::Debug;

/// Tolerance for positions and sizes (R60 section 10).
pub(crate) const EPS: f32 = 0.01;

/// `node_id -> "tag#id.class1.class2"` for every element of a DOM, for reports.
#[derive(Default)]
pub(crate) struct Names(HashMap<usize, String>);

impl Names {
    pub(crate) fn from_dom(root: &Handle) -> Names {
        let mut n = Names::default();
        n.walk(root);
        n
    }

    fn walk(&mut self, h: &Handle) {
        if let NodeData::Element { name, attrs, .. } = &h.data {
            let mut s = name.local.to_string();
            for a in attrs.borrow().iter() {
                match a.name.local.as_ref() {
                    "id" if !a.value.is_empty() => s.push_str(&format!("#{}", a.value)),
                    "class" => {
                        for c in a.value.split_whitespace() {
                            s.push_str(&format!(".{c}"));
                        }
                    }
                    _ => {}
                }
            }
            self.0.insert(node_id(h), s);
        }
        for c in h.children.borrow().iter() {
            self.walk(c);
        }
    }

    pub(crate) fn of(&self, node: usize) -> String {
        if node == 0 {
            "(anon)".to_string()
        } else {
            self.0.get(&node).cloned().unwrap_or_else(|| format!("(node {node:#x})"))
        }
    }
}

/// The first difference found.
#[derive(Debug)]
pub(crate) struct Mismatch {
    /// `html/body/div#a/p.x[2]`-style path of the box (anonymous boxes are `(anon)`).
    pub(crate) path: String,
    pub(crate) field: String,
    pub(crate) a: String,
    pub(crate) b: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at {}: {}: {} vs {}", self.path, self.field, self.a, self.b)
    }
}

fn mm(path: &str, field: impl Into<String>, a: impl Debug, b: impl Debug) -> Mismatch {
    Mismatch { path: path.to_string(), field: field.into(), a: format!("{a:?}"), b: format!("{b:?}") }
}

fn child_path(names: &Names, parent: &str, i: usize, node: usize) -> String {
    format!("{parent}/{}[{i}]", names.of(node))
}

fn near(a: f32, b: f32) -> bool {
    a == b || (a - b).abs() <= EPS || (a.is_nan() && b.is_nan())
}

fn near_f(path: &str, field: &str, a: f32, b: f32) -> Result<(), Mismatch> {
    if near(a, b) { Ok(()) } else { Err(mm(path, field, a, b)) }
}

/// Two styles, minus `node`/`tree_order`: the first changed field.
pub(crate) fn cmp_styles(path: &str, a: &ComputedStyle, b: &ComputedStyle) -> Result<(), Mismatch> {
    match style_changes(a, b).into_iter().next() {
        None => Ok(()),
        Some(c) => Err(Mismatch { path: path.to_string(), field: format!("style.{}", c.field), a: c.old, b: c.new }),
    }
}

fn cmp_image(path: &str, field: &str, a: &Option<Arc<DecodedImage>>, b: &Option<Arc<DecodedImage>>) -> Result<(), Mismatch> {
    match (a, b) {
        (None, None) => Ok(()),
        (Some(x), Some(y)) if x.width == y.width && x.height == y.height && x.pixels == y.pixels => Ok(()),
        (x, y) => Err(mm(path, field, x.as_ref().map(|i| (i.width, i.height)), y.as_ref().map(|i| (i.width, i.height)))),
    }
}

/// Debug-compares every field of a `SpanStyle`. Destructures `a` WITHOUT `..` so a new field is a
/// compile error until it is compared here.
fn cmp_span(path: &str, i: usize, a: &SpanStyle, b: &SpanStyle) -> Result<(), Mismatch> {
    macro_rules! each {
        ($($f:ident),*) => {{
            let SpanStyle { $($f),* } = a;
            $( if format!("{:?}", $f) != format!("{:?}", b.$f) {
                return Err(mm(path, format!("item[{i}].span.{}", stringify!($f)), $f, &b.$f));
            } )*
        }};
    }
    each!(
        color, background_color, font_size, font_weight, font_style, font_family, text_decoration, decoration_style,
        vertical_align, line_height, visibility, cursor, word_spacing, word_break, white_space, href, node, tnode
    );
    Ok(())
}

/// Boxes, structurally: styles (minus `node`/`tree_order`), background image, content kind, inline
/// items' text and spans, children.
pub(crate) fn cmp_boxes(names: &Names, path: &str, a: &LayoutBox, b: &LayoutBox) -> Result<(), Mismatch> {
    cmp_styles(path, &a.style, &b.style)?;
    cmp_image(path, "background_image", &a.background_image, &b.background_image)?;
    match (&a.content, &b.content) {
        (BoxContent::Blocks(x), BoxContent::Blocks(y)) | (BoxContent::TableRow(x), BoxContent::TableRow(y)) => {
            if x.len() != y.len() {
                return Err(mm(path, "children.len", x.len(), y.len()));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                cmp_boxes(names, &child_path(names, path, i, p.style.node), p, q)?;
            }
            Ok(())
        }
        (BoxContent::Inline(x), BoxContent::Inline(y)) => {
            if x.len() != y.len() {
                return Err(mm(path, "inline items.len", x.len(), y.len()));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                match (p, q) {
                    (InlineItem::Text(s, ss), InlineItem::Text(t, ts)) => {
                        if s != t {
                            return Err(mm(path, format!("item[{i}].text"), s, t));
                        }
                        cmp_span(path, i, ss, ts)?;
                    }
                    (
                        InlineItem::Image { image: i1, style: s1, background_image: b1 },
                        InlineItem::Image { image: i2, style: s2, background_image: b2 },
                    ) => {
                        if (i1.width, i1.height) != (i2.width, i2.height) {
                            return Err(mm(path, format!("item[{i}].image size"), (i1.width, i1.height), (i2.width, i2.height)));
                        }
                        cmp_styles(path, s1, s2)?;
                        cmp_image(path, &format!("item[{i}].background_image"), b1, b2)?;
                    }
                    (InlineItem::Embedded(e1), InlineItem::Embedded(e2)) => {
                        cmp_boxes(names, &child_path(names, path, i, e1.style.node), e1, e2)?;
                    }
                    (
                        InlineItem::Spacer { w: w1, pad_w: p1, bg: g1, start: s1 },
                        InlineItem::Spacer { w: w2, pad_w: p2, bg: g2, start: s2 },
                    ) => {
                        if (w1, p1, g1, s1) != (w2, p2, g2, s2) {
                            return Err(mm(path, format!("item[{i}].spacer"), (w1, p1, g1, s1), (w2, p2, g2, s2)));
                        }
                    }
                    (p, q) => return Err(mm(path, format!("item[{i}].kind"), item_kind(p), item_kind(q))),
                }
            }
            Ok(())
        }
        (BoxContent::Image(x), BoxContent::Image(y)) => {
            if (x.width, x.height) == (y.width, y.height) {
                Ok(())
            } else {
                Err(mm(path, "image size", (x.width, x.height), (y.width, y.height)))
            }
        }
        (x, y) => Err(mm(path, "content kind", box_kind(x), box_kind(y))),
    }
}

fn item_kind(i: &InlineItem) -> &'static str {
    match i {
        InlineItem::Text(..) => "Text",
        InlineItem::Image { .. } => "Image",
        InlineItem::Embedded(_) => "Embedded",
        InlineItem::Spacer { .. } => "Spacer",
    }
}

fn box_kind(c: &BoxContent) -> &'static str {
    match c {
        BoxContent::Blocks(_) => "Blocks",
        BoxContent::Inline(_) => "Inline",
        BoxContent::TableRow(_) => "TableRow",
        BoxContent::Image(_) => "Image",
    }
}

fn laid_kind(c: &LaidContent) -> &'static str {
    match c {
        LaidContent::Blocks(_) => "Blocks",
        LaidContent::Text(_) => "Text",
        LaidContent::TableRow(_) => "TableRow",
        LaidContent::Image(_) => "Image",
        LaidContent::MixedInline(_) => "MixedInline",
    }
}

/// The shaped paragraph behind `id`: every glyph's position (0.01 px) and the per-span side tables.
fn cmp_paragraph(path: &str, what: &str, ia: usize, ca: &ShapeCache, ib: usize, cb: &ShapeCache) -> Result<(), Mismatch> {
    let (Some(pa), Some(pb)) = (ca.get(&ia), cb.get(&ib)) else {
        return if ca.contains_key(&ia) == cb.contains_key(&ib) {
            Ok(())
        } else {
            Err(mm(path, format!("{what}.paragraph present"), ca.contains_key(&ia), cb.contains_key(&ib)))
        };
    };
    if Arc::ptr_eq(pa, pb) {
        return Ok(());
    }
    let ga = pa.lock().unwrap();
    let gb = pb.lock().unwrap();
    let (ba, wa, colors_a, va_a, flags_a, cur_a, deco_a, href_a) = &*ga;
    let (bb, wb, colors_b, va_b, flags_b, cur_b, deco_b, href_b) = &*gb;
    near_f(path, &format!("{what}.paragraph width"), *wa, *wb)?;
    for (field, x, y) in [
        ("colors", format!("{colors_a:?}"), format!("{colors_b:?}")),
        ("vertical_align", format!("{va_a:?}"), format!("{va_b:?}")),
        ("flags", format!("{flags_a:?}"), format!("{flags_b:?}")),
        ("cursors", format!("{cur_a:?}"), format!("{cur_b:?}")),
        ("decoration styles", format!("{deco_a:?}"), format!("{deco_b:?}")),
        ("hrefs", format!("{href_a:?}"), format!("{href_b:?}")),
    ] {
        if x != y {
            return Err(Mismatch { path: path.to_string(), field: format!("{what}.paragraph.{field}"), a: x, b: y });
        }
    }
    let (mut ra, mut rb) = (ba.layout_runs(), bb.layout_runs());
    let mut run = 0;
    loop {
        match (ra.next(), rb.next()) {
            (None, None) => return Ok(()),
            (Some(x), Some(y)) => {
                near_f(path, &format!("{what}.run[{run}].line_top"), x.line_top, y.line_top)?;
                near_f(path, &format!("{what}.run[{run}].line_height"), x.line_height, y.line_height)?;
                if x.glyphs.len() != y.glyphs.len() {
                    return Err(mm(path, format!("{what}.run[{run}].glyphs.len"), x.glyphs.len(), y.glyphs.len()));
                }
                for (gi, (g, h)) in x.glyphs.iter().zip(y.glyphs).enumerate() {
                    let at = format!("{what}.run[{run}].glyph[{gi}]");
                    if (g.start, g.end, g.glyph_id, g.metadata) != (h.start, h.end, h.glyph_id, h.metadata) {
                        return Err(mm(
                            path,
                            format!("{at}.id/range"),
                            (g.start, g.end, g.glyph_id, g.metadata),
                            (h.start, h.end, h.glyph_id, h.metadata),
                        ));
                    }
                    near_f(path, &format!("{at}.x"), g.x, h.x)?;
                    near_f(path, &format!("{at}.w"), g.w, h.w)?;
                }
                run += 1;
            }
            (x, y) => return Err(mm(path, format!("{what}.runs"), x.is_some(), y.is_some())),
        }
    }
}

/// Laid trees: x/y/w/h/ink within 0.01 px, `fixed`, `positioned` (order and content), sticky and scroll
/// info, content kind and glyphs (through the shape caches), styles minus `node`/`tree_order`.
pub(crate) fn cmp_laid(names: &Names, path: &str, a: &Laid, ca: &ShapeCache, b: &Laid, cb: &ShapeCache) -> Result<(), Mismatch> {
    cmp_styles(path, &a.style, &b.style)?;
    for (field, x, y) in [("x", a.x, b.x), ("y", a.y, b.y), ("w", a.w, b.w), ("h", a.h, b.h)] {
        near_f(path, field, x, y)?;
    }
    for (field, x, y) in [("ink.l", a.ink.l, b.ink.l), ("ink.t", a.ink.t, b.ink.t), ("ink.r", a.ink.r, b.ink.r), ("ink.b", a.ink.b, b.ink.b)] {
        near_f(path, field, x, y)?;
    }
    if a.fixed != b.fixed {
        return Err(mm(path, "fixed", a.fixed, b.fixed));
    }
    cmp_image(path, "background_image", &a.background_image, &b.background_image)?;
    match (&a.sticky, &b.sticky) {
        (None, None) => {}
        (Some(x), Some(y)) => {
            for (field, p, q) in [("static_top", x.static_top, y.static_top), ("cb_top", x.cb_top, y.cb_top), ("cb_bottom", x.cb_bottom, y.cb_bottom)] {
                near_f(path, &format!("sticky.{field}"), p, q)?;
            }
            if x.top.map(f32::to_bits) != y.top.map(f32::to_bits) && !matches!((x.top, y.top), (Some(p), Some(q)) if near(p, q)) {
                return Err(mm(path, "sticky.top", x.top, y.top));
            }
            if x.bottom.map(f32::to_bits) != y.bottom.map(f32::to_bits) && !matches!((x.bottom, y.bottom), (Some(p), Some(q)) if near(p, q)) {
                return Err(mm(path, "sticky.bottom", x.bottom, y.bottom));
            }
        }
        (x, y) => return Err(mm(path, "sticky", x.is_some(), y.is_some())),
    }
    match (&a.scroll, &b.scroll) {
        (None, None) => {}
        (Some(x), Some(y)) => {
            near_f(path, "scroll.content_h", x.content_h, y.content_h)?;
            near_f(path, "scroll.client_h", x.client_h, y.client_h)?;
            if (x.node, x.user) != (y.node, y.user) {
                return Err(mm(path, "scroll.node/user", (x.node, x.user), (y.node, y.user)));
            }
        }
        (x, y) => return Err(mm(path, "scroll", x.is_some(), y.is_some())),
    }
    if a.positioned.len() != b.positioned.len() {
        return Err(mm(path, "positioned.len", a.positioned.len(), b.positioned.len()));
    }
    for (i, (p, q)) in a.positioned.iter().zip(&b.positioned).enumerate() {
        // The order of `positioned` is the paint order (z-index, then document order): compared by position.
        cmp_laid(names, &format!("{path}/positioned[{i}]={}", names.of(p.style.node)), p, ca, q, cb)?;
    }
    match (&a.content, &b.content) {
        (LaidContent::Blocks(x), LaidContent::Blocks(y)) | (LaidContent::TableRow(x), LaidContent::TableRow(y)) => {
            if x.len() != y.len() {
                return Err(mm(path, "children.len", x.len(), y.len()));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                cmp_laid(names, &child_path(names, path, i, p.style.node), p, ca, q, cb)?;
            }
            Ok(())
        }
        (LaidContent::Text(x), LaidContent::Text(y)) => cmp_paragraph(path, "text", *x, ca, *y, cb),
        (LaidContent::Image(x), LaidContent::Image(y)) => {
            if (x.width, x.height) == (y.width, y.height) {
                Ok(())
            } else {
                Err(mm(path, "image size", (x.width, x.height), (y.width, y.height)))
            }
        }
        (LaidContent::MixedInline(x), LaidContent::MixedInline(y)) => {
            if x.len() != y.len() {
                return Err(mm(path, "mixed.len", x.len(), y.len()));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                match (p, q) {
                    (MixedPositioned::Word { id: i1, x: x1, y: y1, w: w1, h: h1 }, MixedPositioned::Word { id: i2, x: x2, y: y2, w: w2, h: h2 }) => {
                        for (field, u, v) in [("x", x1, x2), ("y", y1, y2), ("w", w1, w2), ("h", h1, h2)] {
                            near_f(path, &format!("mixed[{i}].word.{field}"), *u, *v)?;
                        }
                        cmp_paragraph(path, &format!("mixed[{i}]"), *i1, ca, *i2, cb)?;
                    }
                    (MixedPositioned::Embedded(e1), MixedPositioned::Embedded(e2)) => {
                        cmp_laid(names, &child_path(names, path, i, e1.style.node), e1, ca, e2, cb)?;
                    }
                    (p, q) => {
                        let k = |m: &MixedPositioned| if matches!(m, MixedPositioned::Word { .. }) { "Word" } else { "Embedded" };
                        return Err(mm(path, format!("mixed[{i}].kind"), k(p), k(q)));
                    }
                }
            }
            Ok(())
        }
        (x, y) => Err(mm(path, "content kind", laid_kind(x), laid_kind(y))),
    }
}

/// Root path for `cmp_boxes` / `cmp_laid`.
pub(crate) fn root_path(names: &Names, node: usize) -> String {
    names.of(node)
}
