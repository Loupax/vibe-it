//! QWPT7c: geometry of the elements inside an inline `<svg>`, computed from the DOM (the root svg is
//! one raster image box, so the layout has no boxes for its children). Used by one shared pair of
//! functions: `element_rect` (`getBoundingClientRect` of an SVG child) and `hit` (which child of the
//! svg is under a point, for the App's pointer and `elementFromPoint`/the testdriver).
//!
//! Model: the root's `viewBox` + `preserveAspectRatio` give the user-space -> document matrix; each
//! element's `transform` (translate/scale/rotate/matrix/skew) composes down the tree. Shapes: `rect`
//! (rounded corners ignored), `circle`, `ellipse` exact for hit tests; `line`, `polyline`, `polygon`,
//! `path` (the control-point box of the commands), `image` and `foreignObject` by their box;
//! `text`/`tspan` by an estimated advance (0.55 em per character, baseline at 0.8 em). `use`,
//! `defs`, `symbol`, gradients, clip paths, masks, markers and patterns have no geometry. Not done:
//! stroke geometry of a `fill: none` shape (its box is hit), `vector-effect`, nested `svg` viewBox.
//! `pointer-events: none` (attribute or `style`) skips an element and, inherited, its children;
//! `display: none` and `visibility: hidden` skip it too. Every other pointer-events value is `auto`.
use crate::{element_attr, node_id, Handle, NodeData};

type Rect = (f32, f32, f32, f32); // x0, y0, x1, y1

/// `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct M([f32; 6]);

const IDENT: M = M([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

impl M {
    /// `self` after `o` (`o` is applied to a point first).
    fn mul(self, o: M) -> M {
        let (a, b) = (self.0, o.0);
        M([
            a[0] * b[0] + a[2] * b[1],
            a[1] * b[0] + a[3] * b[1],
            a[0] * b[2] + a[2] * b[3],
            a[1] * b[2] + a[3] * b[3],
            a[0] * b[4] + a[2] * b[5] + a[4],
            a[1] * b[4] + a[3] * b[5] + a[5],
        ])
    }

    fn apply(self, x: f32, y: f32) -> (f32, f32) {
        let m = self.0;
        (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
    }

    fn inverse(self) -> Option<M> {
        let m = self.0;
        let det = m[0] * m[3] - m[1] * m[2];
        if det.abs() < 1e-9 || !det.is_finite() {
            return None;
        }
        let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
        Some(M([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])]))
    }

    /// The box of the transformed corners of `r`.
    fn bbox(self, r: Rect) -> Rect {
        let pts = [self.apply(r.0, r.1), self.apply(r.2, r.1), self.apply(r.2, r.3), self.apply(r.0, r.3)];
        pts.iter().fold((f32::MAX, f32::MAX, f32::MIN, f32::MIN), |a, p| (a.0.min(p.0), a.1.min(p.1), a.2.max(p.0), a.3.max(p.1)))
    }
}

fn union(a: Option<Rect>, b: Rect) -> Option<Rect> {
    Some(a.map_or(b, |a| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))))
}

fn local(el: &Handle) -> String {
    match &el.data {
        NodeData::Element { name, .. } => name.local.as_ref().to_string(),
        _ => String::new(),
    }
}

/// A number with an optional `px` unit; `None` for anything else (a `%`, a keyword).
fn parse_num(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s.strip_suffix("px").unwrap_or(s);
    s.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

fn num(el: &Handle, name: &str) -> Option<f32> {
    element_attr(el, name).and_then(|v| parse_num(&v))
}

fn numbers(s: &str) -> Vec<f32> {
    s.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).filter_map(parse_num).collect()
}

/// A property of `el`: its `style` declaration, else the presentation attribute.
fn prop(el: &Handle, name: &str) -> Option<String> {
    if let Some(st) = element_attr(el, "style") {
        for d in st.split(';') {
            if let Some((k, v)) = d.split_once(':')
                && k.trim().eq_ignore_ascii_case(name)
            {
                return Some(v.trim().to_ascii_lowercase());
            }
        }
    }
    element_attr(el, name).map(|v| v.trim().to_ascii_lowercase())
}

fn parse_transform(s: &str) -> M {
    let mut m = IDENT;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(|c: char| c == ',' || c.is_whitespace()).to_ascii_lowercase();
        let Some(close) = rest[open..].find(')') else { break };
        let a = numbers(&rest[open + 1..open + close]);
        rest = &rest[open + close + 1..];
        let t = match (name.as_str(), a.as_slice()) {
            ("translate", [x]) => M([1.0, 0.0, 0.0, 1.0, *x, 0.0]),
            ("translate", [x, y, ..]) => M([1.0, 0.0, 0.0, 1.0, *x, *y]),
            ("scale", [s]) => M([*s, 0.0, 0.0, *s, 0.0, 0.0]),
            ("scale", [x, y, ..]) => M([*x, 0.0, 0.0, *y, 0.0, 0.0]),
            ("rotate", [d]) => rot(*d, 0.0, 0.0),
            ("rotate", [d, cx, cy, ..]) => rot(*d, *cx, *cy),
            ("skewx", [d]) => M([1.0, 0.0, d.to_radians().tan(), 1.0, 0.0, 0.0]),
            ("skewy", [d]) => M([1.0, d.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            ("matrix", [a, b, c, d, e, f, ..]) => M([*a, *b, *c, *d, *e, *f]),
            _ => IDENT,
        };
        m = m.mul(t);
    }
    m
}

fn rot(deg: f32, cx: f32, cy: f32) -> M {
    let (s, c) = deg.to_radians().sin_cos();
    M([1.0, 0.0, 0.0, 1.0, cx, cy]).mul(M([c, s, -s, c, 0.0, 0.0])).mul(M([1.0, 0.0, 0.0, 1.0, -cx, -cy]))
}

fn own_transform(el: &Handle) -> M {
    element_attr(el, "transform").map_or(IDENT, |t| parse_transform(&t))
}

/// The user space -> document matrix of the root `<svg>` whose content box is `content` (x, y, w, h).
fn root_matrix(svg: &Handle, content: Rect) -> M {
    let (cx, cy, cw, ch) = (content.0, content.1, content.2, content.3);
    let vb = element_attr(svg, "viewBox").map(|v| numbers(&v)).filter(|v| v.len() == 4 && v[2] > 0.0 && v[3] > 0.0);
    let Some(vb) = vb else { return M([1.0, 0.0, 0.0, 1.0, cx, cy]) };
    let par = element_attr(svg, "preserveAspectRatio").map(|v| v.to_ascii_lowercase()).unwrap_or_default();
    let (sx, sy) = (cw / vb[2], ch / vb[3]);
    if par.trim_start().starts_with("none") {
        return M([sx, 0.0, 0.0, sy, cx - vb[0] * sx, cy - vb[1] * sy]);
    }
    let s = if par.contains("slice") { sx.max(sy) } else { sx.min(sy) };
    let frac = |min: &str, max: &str| if par.contains(min) { 0.0 } else if par.contains(max) { 1.0 } else { 0.5 };
    let (fx, fy) = (frac("xmin", "xmax"), frac("ymin", "ymax"));
    M([s, 0.0, 0.0, s, cx + (cw - vb[2] * s) * fx - vb[0] * s, cy + (ch - vb[3] * s) * fy - vb[1] * s])
}

fn is_group(tag: &str) -> bool {
    matches!(tag, "g" | "a" | "svg" | "switch")
}

fn no_geometry(tag: &str) -> bool {
    matches!(
        tag,
        "defs" | "symbol" | "lineargradient" | "radialgradient" | "clippath" | "mask" | "marker" | "pattern" | "style" | "title" | "desc"
            | "metadata" | "script" | "filter" | "use" | "stop"
    )
}

fn font_size(el: &Handle) -> f32 {
    let mut cur = Some(el.clone());
    while let Some(n) = cur {
        if let Some(v) = prop(&n, "font-size").and_then(|v| parse_num(&v)) {
            return v;
        }
        cur = crate::parent_handle(&n);
    }
    16.0
}

/// The boxes of the `text` element `root` and every element below it, in its user space, by node id.
fn text_boxes(root: &Handle) -> Vec<(usize, Rect)> {
    fn walk(n: &Handle, pen: &mut (f32, f32), fs: f32, out: &mut Vec<(usize, Rect)>) -> Option<Rect> {
        match &n.data {
            NodeData::Text { contents } => {
                let t = contents.borrow();
                let chars = t.split_whitespace().collect::<Vec<_>>().join(" ").chars().count() as f32;
                if t.trim().is_empty() || chars == 0.0 {
                    return None;
                }
                let w = chars * fs * 0.55;
                let r = (pen.0, pen.1 - fs * 0.8, pen.0 + w, pen.1 + fs * 0.2);
                pen.0 += w;
                Some(r)
            }
            NodeData::Element { .. } => {
                let tag = local(n);
                let fs = prop(n, "font-size").and_then(|v| parse_num(&v)).unwrap_or(fs);
                if let Some(x) = element_attr(n, "x").and_then(|v| numbers(&v).first().copied()) {
                    pen.0 = x;
                }
                if let Some(y) = element_attr(n, "y").and_then(|v| numbers(&v).first().copied()) {
                    pen.1 = y;
                }
                pen.0 += num(n, "dx").unwrap_or(0.0);
                pen.1 += num(n, "dy").unwrap_or(0.0);
                let mut acc = None;
                if matches!(tag.as_str(), "text" | "tspan" | "a" | "textpath") {
                    for c in n.children.borrow().iter() {
                        if let Some(r) = walk(c, pen, fs, out) {
                            acc = union(acc, r);
                        }
                    }
                }
                let r = acc.unwrap_or((pen.0, pen.1 - fs * 0.8, pen.0, pen.1 + fs * 0.2));
                out.push((node_id(n), r));
                Some(r)
            }
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut pen = (0.0, 0.0);
    walk(root, &mut pen, font_size(root), &mut out);
    out
}

fn path_box(d: &str) -> Option<Rect> {
    let mut acc: Option<Rect> = None;
    let mut add = |x: f32, y: f32| acc = union(acc, (x, y, x, y));
    let (mut cx, mut cy, mut sx, mut sy) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let bytes: Vec<char> = d.chars().collect();
    let mut i = 0;
    let mut cmd = 'M';
    let read = |i: &mut usize| -> Option<f32> {
        while *i < bytes.len() && (bytes[*i].is_whitespace() || bytes[*i] == ',') {
            *i += 1;
        }
        let st = *i;
        while *i < bytes.len() && (bytes[*i].is_ascii_digit() || matches!(bytes[*i], '+' | '-' | '.' | 'e' | 'E')) {
            if *i > st && matches!(bytes[*i], '+' | '-') && !matches!(bytes[*i - 1], 'e' | 'E') {
                break;
            }
            *i += 1;
        }
        bytes[st..*i].iter().collect::<String>().parse().ok()
    };
    loop {
        while i < bytes.len() && (bytes[i].is_whitespace() || bytes[i] == ',') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i].is_ascii_alphabetic() {
            cmd = bytes[i];
            i += 1;
            if cmd == 'z' || cmd == 'Z' {
                (cx, cy) = (sx, sy);
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        let n = match cmd.to_ascii_uppercase() {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            _ => return acc,
        };
        let mut a = [0.0f32; 7];
        for slot in a.iter_mut().take(n) {
            let Some(v) = read(&mut i) else { return acc };
            *slot = v;
        }
        match cmd.to_ascii_uppercase() {
            'H' => cx = a[0] + if rel { cx } else { 0.0 },
            'V' => cy = a[0] + if rel { cy } else { 0.0 },
            'C' => {
                add(a[0] + ox, a[1] + oy);
                add(a[2] + ox, a[3] + oy);
                (cx, cy) = (a[4] + ox, a[5] + oy);
            }
            'S' | 'Q' => {
                add(a[0] + ox, a[1] + oy);
                (cx, cy) = (a[2] + ox, a[3] + oy);
            }
            'A' => (cx, cy) = (a[5] + ox, a[6] + oy),
            _ => (cx, cy) = (a[0] + ox, a[1] + oy),
        }
        if cmd.eq_ignore_ascii_case(&'M') {
            (sx, sy) = (cx, cy);
            cmd = if rel { 'l' } else { 'L' };
        }
        add(cx, cy);
    }
    acc
}

/// The shape of a leaf element in its own user space.
enum Shape {
    Box(Rect),
    Ellipse(f32, f32, f32, f32),
}

impl Shape {
    fn rect(&self) -> Rect {
        match *self {
            Shape::Box(r) => r,
            Shape::Ellipse(cx, cy, rx, ry) => (cx - rx, cy - ry, cx + rx, cy + ry),
        }
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        match *self {
            Shape::Box(r) => x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3,
            Shape::Ellipse(cx, cy, rx, ry) => rx > 0.0 && ry > 0.0 && ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0,
        }
    }
}

fn leaf_shape(el: &Handle, tag: &str) -> Option<Shape> {
    let n = |k: &str| num(el, k).unwrap_or(0.0);
    let pad = if prop(el, "stroke").is_some_and(|s| s != "none") { prop(el, "stroke-width").and_then(|v| parse_num(&v)).unwrap_or(1.0) / 2.0 } else { 0.0 };
    let grow = |r: Rect| (r.0 - pad, r.1 - pad, r.2 + pad, r.3 + pad);
    match tag {
        "rect" | "image" | "foreignobject" => {
            let (w, h) = (n("width"), n("height"));
            (w > 0.0 && h > 0.0).then(|| Shape::Box(grow((n("x"), n("y"), n("x") + w, n("y") + h))))
        }
        "circle" => {
            let r = n("r");
            (r > 0.0).then(|| Shape::Ellipse(n("cx"), n("cy"), r + pad, r + pad))
        }
        "ellipse" => {
            let (rx, ry) = (n("rx"), n("ry"));
            (rx > 0.0 && ry > 0.0).then(|| Shape::Ellipse(n("cx"), n("cy"), rx + pad, ry + pad))
        }
        "line" => {
            let (x1, y1, x2, y2) = (n("x1"), n("y1"), n("x2"), n("y2"));
            let pad = pad.max(1.0);
            Some(Shape::Box((x1.min(x2) - pad, y1.min(y2) - pad, x1.max(x2) + pad, y1.max(y2) + pad)))
        }
        "polyline" | "polygon" => {
            let p = numbers(&element_attr(el, "points")?);
            let mut acc = None;
            for c in p.windows(2).step_by(2) {
                acc = union(acc, (c[0], c[1], c[0], c[1]));
            }
            acc.map(|r| Shape::Box(grow(r)))
        }
        "path" => path_box(&element_attr(el, "d")?).map(|r| Shape::Box(grow(r))),
        _ => None,
    }
}

fn hidden(el: &Handle) -> bool {
    prop(el, "display").is_some_and(|v| v == "none") || prop(el, "visibility").is_some_and(|v| v == "hidden" || v == "collapse")
}

fn children(el: &Handle) -> Vec<Handle> {
    el.children.borrow().iter().filter(|c| matches!(c.data, NodeData::Element { .. })).cloned().collect()
}

/// The box of `el` in its PARENT's user space (its own `transform` applied); `None` without geometry.
fn bbox_in_parent(el: &Handle, text: &mut Option<(usize, Vec<(usize, Rect)>)>) -> Option<Rect> {
    let tag = local(el).to_ascii_lowercase();
    if no_geometry(&tag) || hidden(el) {
        return None;
    }
    let t = own_transform(el);
    let inner = if is_group(&tag) {
        let mut acc = None;
        for c in children(el) {
            if let Some(r) = bbox_in_parent(&c, text) {
                acc = union(acc, r);
            }
        }
        acc?
    } else if matches!(tag.as_str(), "text" | "tspan" | "textpath") {
        text_rect(el, text)?
    } else {
        leaf_shape(el, &tag)?.rect()
    };
    Some(t.bbox(inner))
}

/// The box of the text element `el` in the user space of its `text` ancestor (`text` itself: the
/// space it sits in, before its own transform, which `bbox_in_parent` applies).
fn text_rect(el: &Handle, cache: &mut Option<(usize, Vec<(usize, Rect)>)>) -> Option<Rect> {
    let mut root = el.clone();
    while !local(&root).eq_ignore_ascii_case("text") {
        root = crate::parent_handle(&root)?;
    }
    let rid = node_id(&root);
    if cache.as_ref().is_none_or(|c| c.0 != rid) {
        *cache = Some((rid, text_boxes(&root)));
    }
    let id = node_id(el);
    let r = cache.as_ref()?.1.iter().find(|(n, _)| *n == id).map(|(_, r)| *r)?;
    // A tspan's box is in the text's space; the transforms between them (tspan has none) are ignored.
    Some(r)
}

/// The document-space box (x, y, w, h) of `el`, an element below the root `<svg>` `root` whose content
/// box is `content`; `None` when `el` has no geometry (`defs`, `use`, hidden, not below `root`).
pub(crate) fn element_rect(root: &Handle, content: (f32, f32, f32, f32), el: &Handle) -> Option<(f32, f32, f32, f32)> {
    let mut chain = Vec::new();
    let mut cur = crate::parent_handle(el);
    let mut found = false;
    while let Some(a) = cur {
        if std::rc::Rc::ptr_eq(&a, root) {
            found = true;
            break;
        }
        chain.push(a.clone());
        cur = crate::parent_handle(&a);
    }
    if !found {
        return None;
    }
    let mut m = root_matrix(root, (content.0, content.1, content.2, content.3));
    for a in chain.iter().rev() {
        m = m.mul(own_transform(a));
    }
    let r = bbox_in_parent(el, &mut None)?;
    let r = m.bbox(r);
    Some((r.0, r.1, r.2 - r.0, r.3 - r.1))
}

/// The deepest element below the root `<svg>` `root` under document point (`px`, `py`), or `None`
/// (then the svg itself is the hit).
pub(crate) fn hit(root: &Handle, content: (f32, f32, f32, f32), px: f32, py: f32) -> Option<Handle> {
    let m = root_matrix(root, content).inverse()?;
    let (x, y) = m.apply(px, py);
    let mut text = None;
    for c in children(root).iter().rev() {
        if let Some(h) = hit_in(c, x, y, false, &mut text) {
            return Some(h);
        }
    }
    None
}

fn hit_in(el: &Handle, x: f32, y: f32, none: bool, text: &mut Option<(usize, Vec<(usize, Rect)>)>) -> Option<Handle> {
    let tag = local(el).to_ascii_lowercase();
    if no_geometry(&tag) || hidden(el) {
        return None;
    }
    let none = prop(el, "pointer-events").map_or(none, |v| v == "none");
    let (x, y) = own_transform(el).inverse()?.apply(x, y);
    if is_group(&tag) {
        for c in children(el).iter().rev() {
            if let Some(h) = hit_in(c, x, y, none, text) {
                return Some(h);
            }
        }
        return None;
    }
    if matches!(tag.as_str(), "text" | "textpath") {
        for c in children(el).iter().rev() {
            if let Some(h) = hit_in(c, x, y, none, text) {
                return Some(h);
            }
        }
    }
    if none {
        return None;
    }
    if matches!(tag.as_str(), "text" | "tspan" | "textpath") {
        let r = text_rect(el, text)?;
        return (x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3).then(|| el.clone());
    }
    let shape = leaf_shape(el, &tag)?;
    if !shape.contains(x, y) {
        return None;
    }
    // `fill: none` paints only the stroke (the default `visiblePainted`): no stroke, no target. A
    // rect/circle/ellipse is then hit on its stroke ring; a path/polyline/polygon (no exact geometry
    // here) is not hit at all, so the svg underneath is.
    if prop(el, "fill").is_some_and(|f| f == "none") {
        let stroked = prop(el, "stroke").is_some_and(|s| s != "none");
        let sw = prop(el, "stroke-width").and_then(|v| parse_num(&v)).unwrap_or(1.0);
        let ring = match &shape {
            Shape::Box(r) if matches!(tag.as_str(), "rect" | "image" | "foreignobject") => {
                !Shape::Box((r.0 + sw, r.1 + sw, r.2 - sw, r.3 - sw)).contains(x, y)
            }
            Shape::Ellipse(cx, cy, rx, ry) if matches!(tag.as_str(), "circle" | "ellipse") => !Shape::Ellipse(*cx, *cy, rx - sw, ry - sw).contains(x, y),
            _ => false,
        };
        return (stroked && ring).then(|| el.clone());
    }
    Some(el.clone())
}
