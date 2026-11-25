//! QWPT7c/QWPT7e: geometry of the elements inside an inline `<svg>`, computed from the DOM (the root svg
//! is one raster image box, so the layout has no boxes for its children). Used by one shared set of
//! functions: `element_rect` (`getBoundingClientRect` of an SVG child) and `hit` / `hit_all` (which
//! child of the svg is under a point, for the App's pointer, `elementFromPoint` and `elementsFromPoint`).
//!
//! Model: the root's `viewBox` + `preserveAspectRatio` give the user-space -> document matrix; each
//! element's `transform` (translate/scale/rotate/matrix/skew) composes down the tree, a nested `svg`
//! contributes its x/y and viewBox (and clips to its viewport), and `use` is hit through the element
//! it references. Shapes: `rect` (rounded corners ignored), `circle`, `ellipse`, `line`, `polyline`,
//! `polygon` and `path` (curves and arcs flattened) have a fill region (nonzero or `fill-rule:
//! evenodd`) and a stroke region (`stroke-width` around the outline); `image` and `foreignObject` are
//! boxes. `pointer-events` is honoured for none, bounding-box, visiblePainted/auto, visibleFill,
//! visibleStroke, visible, painted, fill, stroke and all: the computed value from the stylesheet when the
//! engine registered a hook (`set_pe_hook`), else the attribute / `style`. `text`/`tspan` use an
//! estimated advance (0.55 em per character, baseline at 0.8 em). Not done: `vector-effect`, line
//! caps and joins, rounded `rect` corners, markers, clip paths, masks.
//! `display: none` and `visibility: hidden` (attribute or `style`) skip an element.
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
    // A nested `<svg>` positions and scales its content (x, y, viewBox) instead.
    if local(el).eq_ignore_ascii_case("svg") {
        return nested_matrix(el);
    }
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
            | "metadata" | "script" | "filter" | "stop"
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

/// A subpath: its points (curves flattened) and whether it was closed with `z`.
type Sub = (Vec<(f32, f32)>, bool);

fn flatten_arc(out: &mut Vec<(f32, f32)>, from: (f32, f32), a: [f32; 7]) {
    let (x1, y1) = from;
    let (mut rx, mut ry, phi, fa, fs, x2, y2) = (a[0].abs(), a[1].abs(), a[2].to_radians(), a[3] != 0.0, a[4] != 0.0, a[5], a[6]);
    if (x1 == x2 && y1 == y2) || rx == 0.0 || ry == 0.0 {
        out.push((x2, y2));
        return;
    }
    let (sp, cp) = phi.sin_cos();
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let (x1p, y1p) = (cp * dx + sp * dy, -sp * dx + cp * dy);
    let lam = (x1p / rx).powi(2) + (y1p / ry).powi(2);
    if lam > 1.0 {
        let s = lam.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut co = if den == 0.0 { 0.0 } else { (num / den).max(0.0).sqrt() };
    if fa == fs {
        co = -co;
    }
    let (cxp, cyp) = (co * rx * y1p / ry, -co * ry * x1p / rx);
    let (cx, cy) = (cp * cxp - sp * cyp + (x1 + x2) / 2.0, sp * cxp + cp * cyp + (y1 + y2) / 2.0);
    let ang = |ux: f32, uy: f32, vx: f32, vy: f32| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let th1 = ang(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dth = ang((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
    if !fs && dth > 0.0 {
        dth -= std::f32::consts::TAU;
    } else if fs && dth < 0.0 {
        dth += std::f32::consts::TAU;
    }
    let n = ((dth.abs() / std::f32::consts::FRAC_PI_8).ceil() as usize).clamp(2, 32);
    for i in 1..=n {
        let t = th1 + dth * i as f32 / n as f32;
        let (st, ct) = t.sin_cos();
        out.push((cp * rx * ct - sp * ry * st + cx, sp * rx * ct + cp * ry * st + cy));
    }
    if let Some(l) = out.last_mut() {
        *l = (x2, y2);
    }
}

fn bezier(out: &mut Vec<(f32, f32)>, pts: &[(f32, f32)]) {
    const N: usize = 12;
    for i in 1..=N {
        let t = i as f32 / N as f32;
        let u = 1.0 - t;
        let p = if pts.len() == 4 {
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (a * pts[0].0 + b * pts[1].0 + c * pts[2].0 + d * pts[3].0, a * pts[0].1 + b * pts[1].1 + c * pts[2].1 + d * pts[3].1)
        } else {
            let (a, b, c) = (u * u, 2.0 * u * t, t * t);
            (a * pts[0].0 + b * pts[1].0 + c * pts[2].0, a * pts[0].1 + b * pts[1].1 + c * pts[2].1)
        };
        out.push(p);
    }
}

/// The subpaths of a path `d` (curves and arcs flattened); a parse error ends the path.
fn path_subpaths(d: &str) -> Vec<Sub> {
    let b: Vec<char> = d.chars().collect();
    let mut i = 0;
    let skip = |i: &mut usize| {
        while *i < b.len() && (b[*i].is_whitespace() || b[*i] == ',') {
            *i += 1;
        }
    };
    let read = |i: &mut usize| -> Option<f32> {
        skip(i);
        let st = *i;
        while *i < b.len() && (b[*i].is_ascii_digit() || matches!(b[*i], '+' | '-' | '.' | 'e' | 'E')) {
            if *i > st && matches!(b[*i], '+' | '-') && !matches!(b[*i - 1], 'e' | 'E') {
                break;
            }
            if *i > st && b[*i] == '.' && b[st..*i].contains(&'.') {
                break;
            }
            *i += 1;
        }
        b[st..*i].iter().collect::<String>().parse().ok()
    };
    let flag = |i: &mut usize| -> Option<f32> {
        skip(i);
        let c = *b.get(*i)?;
        *i += 1;
        match c {
            '0' => Some(0.0),
            '1' => Some(1.0),
            _ => None,
        }
    };
    let mut subs: Vec<Sub> = Vec::new();
    let (mut cx, mut cy, mut sx, mut sy) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut last_ctrl: Option<(f32, f32)> = None;
    let mut cmd = 'M';
    loop {
        skip(&mut i);
        if i >= b.len() {
            break;
        }
        if b[i].is_ascii_alphabetic() {
            cmd = b[i];
            i += 1;
            if cmd == 'z' || cmd == 'Z' {
                if let Some(s) = subs.last_mut() {
                    s.1 = true;
                }
                (cx, cy) = (sx, sy);
                last_ctrl = None;
                continue;
            }
        } else if cmd == 'z' || cmd == 'Z' {
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let up = cmd.to_ascii_uppercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        let n = match up {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            _ => break,
        };
        let mut a = [0.0f32; 7];
        let mut ok = true;
        for (k, slot) in a.iter_mut().enumerate().take(n) {
            let v = if up == 'A' && (k == 3 || k == 4) { flag(&mut i) } else { read(&mut i) };
            match v {
                Some(v) => *slot = v,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            break;
        }
        if up == 'M' {
            subs.push((vec![(a[0] + ox, a[1] + oy)], false));
            (cx, cy) = (a[0] + ox, a[1] + oy);
            (sx, sy) = (cx, cy);
            cmd = if rel { 'l' } else { 'L' };
            last_ctrl = None;
            continue;
        }
        if subs.is_empty() {
            break;
        }
        // A drawing command after `z` starts a new subpath at the start point.
        if subs.last().is_some_and(|s| s.1) {
            subs.push((vec![(cx, cy)], false));
        }
        let p0 = (cx, cy);
        let pts = &mut subs.last_mut().unwrap().0;
        match up {
            'L' => (cx, cy) = (a[0] + ox, a[1] + oy),
            'H' => cx = a[0] + if rel { cx } else { 0.0 },
            'V' => cy = a[0] + if rel { cy } else { 0.0 },
            'C' => {
                bezier(pts, &[p0, (a[0] + ox, a[1] + oy), (a[2] + ox, a[3] + oy), (a[4] + ox, a[5] + oy)]);
                last_ctrl = Some((a[2] + ox, a[3] + oy));
                (cx, cy) = (a[4] + ox, a[5] + oy);
            }
            'S' => {
                let c1 = last_ctrl.map_or(p0, |c| (2.0 * p0.0 - c.0, 2.0 * p0.1 - c.1));
                bezier(pts, &[p0, c1, (a[0] + ox, a[1] + oy), (a[2] + ox, a[3] + oy)]);
                last_ctrl = Some((a[0] + ox, a[1] + oy));
                (cx, cy) = (a[2] + ox, a[3] + oy);
            }
            'Q' => {
                bezier(pts, &[p0, (a[0] + ox, a[1] + oy), (a[2] + ox, a[3] + oy)]);
                last_ctrl = Some((a[0] + ox, a[1] + oy));
                (cx, cy) = (a[2] + ox, a[3] + oy);
            }
            'T' => {
                let c1 = last_ctrl.map_or(p0, |c| (2.0 * p0.0 - c.0, 2.0 * p0.1 - c.1));
                bezier(pts, &[p0, c1, (a[0] + ox, a[1] + oy)]);
                last_ctrl = Some(c1);
                (cx, cy) = (a[0] + ox, a[1] + oy);
            }
            'A' => {
                flatten_arc(pts, p0, [a[0], a[1], a[2], a[3], a[4], a[5] + ox, a[6] + oy]);
                (cx, cy) = (a[5] + ox, a[6] + oy);
            }
            _ => {}
        }
        if !matches!(up, 'C' | 'S' | 'Q' | 'T') {
            last_ctrl = None;
        }
        if matches!(up, 'L' | 'H' | 'V') {
            pts.push((cx, cy));
        }
    }
    subs
}

fn subs_bbox(subs: &[Sub]) -> Option<Rect> {
    let mut acc = None;
    for (pts, _) in subs {
        for p in pts {
            acc = union(acc, (p.0, p.1, p.0, p.1));
        }
    }
    acc
}

/// The geometry of a leaf element in its own user space (fill and stroke regions are separate).
enum Geo {
    Box(Rect),
    Ellipse(f32, f32, f32, f32),
    Poly { subs: Vec<Sub>, bbox: Rect, evenodd: bool },
}

fn winding(subs: &[Sub], x: f32, y: f32) -> bool {
    let mut wn = 0i32;
    for (pts, _) in subs {
        let n = pts.len();
        if n < 2 {
            continue;
        }
        for k in 0..n {
            let (a, b) = (pts[k], pts[(k + 1) % n]);
            let cross = (b.0 - a.0) * (y - a.1) - (x - a.0) * (b.1 - a.1);
            if a.1 <= y {
                if b.1 > y && cross > 0.0 {
                    wn += 1;
                }
            } else if b.1 <= y && cross < 0.0 {
                wn -= 1;
            }
        }
    }
    wn != 0
}

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// The even-odd rule: an odd number of crossings.
fn crossings_odd(subs: &[Sub], x: f32, y: f32) -> bool {
    let mut c = 0u32;
    for (pts, _) in subs {
        let n = pts.len();
        if n < 2 {
            continue;
        }
        for k in 0..n {
            let (a, b) = (pts[k], pts[(k + 1) % n]);
            if (a.1 <= y) != (b.1 <= y) {
                let t = (y - a.1) / (b.1 - a.1);
                if x < a.0 + t * (b.0 - a.0) {
                    c += 1;
                }
            }
        }
    }
    c % 2 == 1
}

impl Geo {
    /// The geometry's own box (no stroke).
    fn bbox(&self) -> Rect {
        match *self {
            Geo::Box(r) => r,
            Geo::Ellipse(cx, cy, rx, ry) => (cx - rx, cy - ry, cx + rx, cy + ry),
            Geo::Poly { bbox, .. } => bbox,
        }
    }

    fn in_fill(&self, x: f32, y: f32) -> bool {
        match self {
            Geo::Box(r) => x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3,
            Geo::Ellipse(cx, cy, rx, ry) => *rx > 0.0 && *ry > 0.0 && ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0,
            Geo::Poly { subs, evenodd, .. } => {
                if *evenodd {
                    crossings_odd(subs, x, y)
                } else {
                    winding(subs, x, y)
                }
            }
        }
    }

    /// The stroke of width `sw`: the band `sw / 2` either side of the outline.
    fn in_stroke(&self, x: f32, y: f32, sw: f32) -> bool {
        let h = sw / 2.0;
        match self {
            Geo::Box(r) => {
                let outer = x >= r.0 - h && x <= r.2 + h && y >= r.1 - h && y <= r.3 + h;
                let inner = x > r.0 + h && x < r.2 - h && y > r.1 + h && y < r.3 - h;
                outer && !inner
            }
            Geo::Ellipse(cx, cy, rx, ry) => {
                let f = |ax: f32, ay: f32| ay > 0.0 && ax > 0.0 && ((x - cx) / ax).powi(2) + ((y - cy) / ay).powi(2) <= 1.0;
                f(rx + h, ry + h) && !f(rx - h, ry - h)
            }
            Geo::Poly { subs, .. } => subs.iter().any(|(pts, closed)| {
                let n = pts.len();
                if n == 1 {
                    return seg_dist((x, y), pts[0], pts[0]) <= h;
                }
                let last = if *closed { n } else { n - 1 };
                (0..last).any(|k| seg_dist((x, y), pts[k], pts[(k + 1) % n]) <= h)
            }),
        }
    }
}

fn leaf_geo(el: &Handle, tag: &str) -> Option<Geo> {
    let n = |k: &str| num(el, k).unwrap_or(0.0);
    match tag {
        "rect" | "image" | "foreignobject" => {
            let (w, h) = (n("width"), n("height"));
            (w > 0.0 && h > 0.0).then(|| Geo::Box((n("x"), n("y"), n("x") + w, n("y") + h)))
        }
        "circle" => {
            let r = n("r");
            (r > 0.0).then(|| Geo::Ellipse(n("cx"), n("cy"), r, r))
        }
        "ellipse" => {
            let (rx, ry) = (n("rx"), n("ry"));
            (rx > 0.0 && ry > 0.0).then(|| Geo::Ellipse(n("cx"), n("cy"), rx, ry))
        }
        "line" => {
            let pts = vec![(n("x1"), n("y1")), (n("x2"), n("y2"))];
            let subs = vec![(pts, false)];
            let bbox = subs_bbox(&subs)?;
            Some(Geo::Poly { subs, bbox, evenodd: false })
        }
        "polyline" | "polygon" => {
            let p = numbers(&element_attr(el, "points")?);
            let pts: Vec<(f32, f32)> = p.windows(2).step_by(2).map(|c| (c[0], c[1])).collect();
            let subs = vec![(pts, tag == "polygon")];
            let bbox = subs_bbox(&subs)?;
            Some(Geo::Poly { subs, bbox, evenodd: prop(el, "fill-rule").is_some_and(|v| v == "evenodd") })
        }
        "path" => {
            let subs = path_subpaths(&element_attr(el, "d")?);
            let bbox = subs_bbox(&subs)?;
            Some(Geo::Poly { subs, bbox, evenodd: prop(el, "fill-rule").is_some_and(|v| v == "evenodd") })
        }
        _ => None,
    }
}

fn stroke_width(el: &Handle) -> f32 {
    prop(el, "stroke-width").and_then(|v| parse_num(&v)).unwrap_or(1.0)
}

fn stroked(el: &Handle) -> bool {
    prop(el, "stroke").is_some_and(|s| s != "none")
}

/// The box of a leaf in its own user space, with the stroke's half width around it (a `line` always
/// has at least one unit).
fn leaf_rect(el: &Handle, tag: &str) -> Option<Rect> {
    let r = leaf_geo(el, tag)?.bbox();
    let mut pad = if stroked(el) { stroke_width(el) / 2.0 } else { 0.0 };
    if tag == "line" {
        pad = pad.max(1.0);
    }
    Some((r.0 - pad, r.1 - pad, r.2 + pad, r.3 + pad))
}

fn hidden(el: &Handle) -> bool {
    prop(el, "display").is_some_and(|v| v == "none") || prop(el, "visibility").is_some_and(|v| v == "hidden" || v == "collapse")
}

fn children(el: &Handle) -> Vec<Handle> {
    el.children.borrow().iter().filter(|c| matches!(c.data, NodeData::Element { .. })).cloned().collect()
}

type TextCache = Option<(usize, Vec<(usize, Rect)>)>;

/// The element a `use` references (`href` / `xlink:href` to an id in the same document).
fn use_target(el: &Handle) -> Option<Handle> {
    let href = element_attr(el, "href")?.trim().to_string();
    let id = href.strip_prefix('#')?;
    let mut top = el.clone();
    while let Some(p) = crate::parent_handle(&top) {
        top = p;
    }
    let mut stack = vec![top];
    while let Some(n) = stack.pop() {
        if matches!(n.data, NodeData::Element { .. }) && element_attr(&n, "id").is_some_and(|v| &*v == id) {
            return Some(n);
        }
        stack.extend(n.children.borrow().iter().rev().cloned());
    }
    None
}

fn use_matrix(el: &Handle) -> M {
    M([1.0, 0.0, 0.0, 1.0, num(el, "x").unwrap_or(0.0), num(el, "y").unwrap_or(0.0)])
}

/// A nested `<svg>`'s transform into its parent's user space: its x/y position and viewBox.
fn nested_matrix(el: &Handle) -> M {
    let (x, y) = (num(el, "x").unwrap_or(0.0), num(el, "y").unwrap_or(0.0));
    match (num(el, "width"), num(el, "height")) {
        (Some(w), Some(h)) => root_matrix(el, (x, y, w, h)),
        _ => M([1.0, 0.0, 0.0, 1.0, x, y]),
    }
}

/// The viewport of a nested `<svg>` in its parent's space (`None` when it has no size, or paints overflow).
fn nested_viewport(el: &Handle) -> Option<Rect> {
    if prop(el, "overflow").is_some_and(|v| v == "visible" || v == "auto") {
        return None;
    }
    let (x, y) = (num(el, "x").unwrap_or(0.0), num(el, "y").unwrap_or(0.0));
    Some((x, y, x + num(el, "width")?, y + num(el, "height")?))
}

/// The box of `el` in its PARENT's user space (its own `transform` applied); `None` without geometry.
fn bbox_in_parent(el: &Handle, text: &mut TextCache, depth: u32) -> Option<Rect> {
    let tag = local(el).to_ascii_lowercase();
    if depth > 8 || (no_geometry(&tag) && tag != "use") || hidden(el) {
        return None;
    }
    let t = own_transform(el);
    let inner = if tag == "use" {
        let target = use_target(el)?;
        let ttag = local(&target).to_ascii_lowercase();
        let r = if matches!(ttag.as_str(), "symbol" | "svg") {
            let mut acc = None;
            for c in children(&target) {
                if let Some(r) = bbox_in_parent(&c, text, depth + 1) {
                    acc = union(acc, r);
                }
            }
            acc?
        } else {
            bbox_in_parent(&target, text, depth + 1)?
        };
        use_matrix(el).bbox(r)
    } else if is_group(&tag) {
        let mut acc = None;
        for c in children(el) {
            if let Some(r) = bbox_in_parent(&c, text, depth) {
                acc = union(acc, r);
            }
        }
        acc?
    } else if matches!(tag.as_str(), "text" | "tspan" | "textpath") {
        text_rect(el, text)?
    } else {
        leaf_rect(el, &tag)?
    };
    Some(t.bbox(inner))
}

/// The box of the text element `el` in the user space of its `text` ancestor (`text` itself: the
/// space it sits in, before its own transform, which `bbox_in_parent` applies).
fn text_rect(el: &Handle, cache: &mut TextCache) -> Option<Rect> {
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
/// box is `content`; `None` when `el` has no geometry (`defs`, hidden, not below `root`).
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
    let r = bbox_in_parent(el, &mut None, 0)?;
    let r = m.bbox(r);
    Some((r.0, r.1, r.2 - r.0, r.3 - r.1))
}

type PeHook = std::rc::Rc<dyn Fn(&Handle) -> Option<u8>>;

thread_local! {
    /// The computed `pointer-events` code (`style::pointer_events_code`) of an element, from the page's
    /// style cache (stylesheets). Set by the script engine; without one only attributes and `style` count.
    static PE_HOOK: std::cell::RefCell<Option<PeHook>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn set_pe_hook(h: Option<PeHook>) {
    PE_HOOK.with(|c| *c.borrow_mut() = h);
}

fn computed_pe(el: &Handle) -> Option<u8> {
    let h = PE_HOOK.with(|c| c.borrow().clone())?;
    h(el)
}

struct Hits {
    all: bool,
    text: TextCache,
    out: Vec<Handle>,
}

/// The deepest element below the root `<svg>` `root` under document point (`px`, `py`), or `None`
/// (then the svg itself is the hit).
pub(crate) fn hit(root: &Handle, content: (f32, f32, f32, f32), px: f32, py: f32) -> Option<Handle> {
    let mut h = Hits { all: false, text: None, out: Vec::new() };
    walk(root, content, px, py, &mut h);
    h.out.into_iter().next()
}

/// Every element below the root `<svg>` under the point, topmost first: the shapes hit (reverse paint
/// order), then their ancestors up to and including the root (what `elementsFromPoint` lists).
pub(crate) fn hit_all(root: &Handle, content: (f32, f32, f32, f32), px: f32, py: f32) -> Vec<Handle> {
    let mut h = Hits { all: true, text: None, out: Vec::new() };
    walk(root, content, px, py, &mut h);
    let leaves = std::mem::take(&mut h.out);
    let mut out: Vec<Handle> = Vec::new();
    let push = |n: Handle, out: &mut Vec<Handle>| {
        if !out.iter().any(|o| std::rc::Rc::ptr_eq(o, &n)) {
            out.push(n);
        }
    };
    for l in &leaves {
        push(l.clone(), &mut out);
    }
    for l in &leaves {
        let mut cur = crate::parent_handle(l);
        while let Some(a) = cur {
            if std::rc::Rc::ptr_eq(&a, root) {
                break;
            }
            push(a.clone(), &mut out);
            cur = crate::parent_handle(&a);
        }
    }
    out.push(root.clone());
    out
}

fn walk(root: &Handle, content: (f32, f32, f32, f32), px: f32, py: f32, h: &mut Hits) {
    let Some(m) = root_matrix(root, content).inverse() else { return };
    let (x, y) = m.apply(px, py);
    let inherit = prop(root, "pointer-events").and_then(|v| crate::style::pointer_events_code(&v)).unwrap_or(0);
    for c in children(root).iter().rev() {
        if hit_in(c, x, y, inherit, 0, h) {
            return;
        }
    }
}

/// Whether the leaf `el` (geometry `geo`) is the target at (`x`, `y`) given its `pointer-events` code.
fn leaf_hit(el: &Handle, tag: &str, geo: &Geo, x: f32, y: f32, pe: u8) -> bool {
    let sw = stroke_width(el);
    // The images and foreignObjects are boxes: always filled.
    let fill_on = matches!(tag, "image" | "foreignobject") || prop(el, "fill").is_none_or(|f| f != "none");
    let stroke_on = stroked(el) && sw > 0.0;
    let fill = |_: ()| geo.in_fill(x, y);
    let stroke = |_: ()| geo.in_stroke(x, y, sw);
    match pe {
        1 => false,
        2 => {
            let r = geo.bbox();
            x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
        }
        3 | 7 => fill(()),
        4 | 8 => stroke(()),
        5 | 9 => fill(()) || stroke(()),
        _ => (fill_on && fill(())) || (stroke_on && stroke(())),
    }
}

/// Visit `el` at point (`x`, `y`) in its parent's user space. `inherit` is the inherited attribute
/// `pointer-events` code. Returns true when the walk is done (the first hit, unless collecting all).
fn hit_in(el: &Handle, x: f32, y: f32, inherit: u8, depth: u32, h: &mut Hits) -> bool {
    let tag = local(el).to_ascii_lowercase();
    if depth > 8 || (no_geometry(&tag) && tag != "use") || hidden(el) {
        return false;
    }
    let attr_pe = prop(el, "pointer-events").and_then(|v| crate::style::pointer_events_code(&v)).unwrap_or(inherit);
    if tag == "svg" && nested_viewport(el).is_some_and(|v| x < v.0 || x > v.2 || y < v.1 || y > v.3) {
        return false;
    }
    let Some(inv) = own_transform(el).inverse() else { return false };
    let (x, y) = inv.apply(x, y);
    let pe_of = |el: &Handle| match computed_pe(el) {
        Some(c) if c != 0 => c,
        _ => attr_pe,
    };
    if tag == "use" {
        let Some(target) = use_target(el) else { return false };
        let (ux, uy) = use_matrix(el).inverse().map_or((x, y), |i| i.apply(x, y));
        let mut inner = Hits { all: false, text: None, out: Vec::new() };
        let ttag = local(&target).to_ascii_lowercase();
        let found = if matches!(ttag.as_str(), "symbol" | "svg") {
            children(&target).iter().rev().any(|c| hit_in(c, ux, uy, attr_pe, depth + 1, &mut inner))
        } else {
            hit_in(&target, ux, uy, attr_pe, depth + 1, &mut inner)
        };
        if found || !inner.out.is_empty() {
            h.out.push(el.clone());
            return !h.all;
        }
        return false;
    }
    if is_group(&tag) {
        for c in children(el).iter().rev() {
            if hit_in(c, x, y, attr_pe, depth, h) {
                return true;
            }
        }
        return false;
    }
    if matches!(tag.as_str(), "text" | "textpath") {
        for c in children(el).iter().rev() {
            if hit_in(c, x, y, attr_pe, depth, h) {
                return true;
            }
        }
    }
    if matches!(tag.as_str(), "text" | "tspan" | "textpath") {
        if pe_of(el) == 1 {
            return false;
        }
        let Some(r) = text_rect(el, &mut h.text) else { return false };
        if x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3 {
            h.out.push(el.clone());
            return !h.all;
        }
        return false;
    }
    let Some(geo) = leaf_geo(el, &tag) else { return false };
    if leaf_hit(el, &tag, &geo, x, y, pe_of(el)) {
        h.out.push(el.clone());
        return !h.all;
    }
    false
}
