//! QINC1a (R60 section 10): the seeded differential fuzz harness for the incremental style + layout
//! engine, plus the tests of `style_diff` and the comparators.
//!
//! QINC1a laid every step out TWICE from scratch and asserted that the comparators find no difference
//! (determinism of the from-scratch path, including `positioned` order, which is why `tree_order` is
//! skipped). QINC1f: the second run is now the incremental restyle. Every mutation is written to the
//! write log the way a script's native would (`incr_log::note`), the check-mode reconcile is OFF, and
//! the shadow update inside each build must reproduce every style the from-scratch build resolved
//! (`cmp_styles` semantics) and its generated content. QINC1g: the kept box tree is compared after every
//! step; QINC1h1: so is the engine's relayout of it (`cmp_laid` against the from-scratch laid tree).
//!
//! Size: `SCRATCHY_FUZZ_N` cases (default 1 000 in release, 200 in debug; plus `REGRESSION_SEEDS`);
//! `SCRATCHY_FUZZ_SEED` offsets the seeds.

use super::*;
use crate::incr_cmp::{cmp_boxes, cmp_laid, cmp_styles, root_path, Names};
use crate::incr_diff::{style_changes, style_diff, StyleDiff};

// ---------- seeded generator ----------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        // splitmix64 scramble so adjacent seeds give unrelated streams
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Rng((z ^ (z >> 31)) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() >> 33) as usize % n.max(1)
    }
    fn pct(&mut self, p: usize) -> bool {
        self.below(100) < p
    }
    fn pick<T: Copy>(&mut self, s: &[T]) -> T {
        s[self.below(s.len())]
    }
}

const SELECTORS: &[&str] = &[
    ".c0", ".c1", ".c2", ".c3", ".c4", "div", "p", "span", "section", "ul > li", "div .c1", ".c0 + .c2", ".c1 ~ .c3", "li:nth-child(odd)",
    ":nth-child(2n+1)", ".c2:hover", "div:first-child", ".c0 > .c1", "span:not(.c1)", "#i1", "#i2", ".c4::before", "p::before", ".c3::after",
    "a:hover", "div > span", "b", "section .c0",
];

const DECLS: &[&str] = &[
    "display:none", "display:block", "display:inline", "display:inline-block", "display:flex", "display:grid", "display:contents",
    "display:table", "display:list-item", "display:inline-flex", "float:left", "float:right", "clear:both", "position:absolute;top:5px;left:10px",
    "position:fixed;bottom:0;right:3px", "position:relative;left:3px;top:2px", "position:sticky;top:0", "z-index:-1", "z-index:2", "z-index:5",
    "opacity:.5", "opacity:0", "width:50%", "width:120px", "width:auto", "width:fit-content", "width:max-content", "height:30px", "min-height:20px",
    "max-width:100px", "margin:5px 10px", "margin:0 auto", "margin-top:-4px", "padding:6px", "padding:0 12px", "border:2px solid red", "color:#c33",
    "background:#abc", "font-size:20px", "font-size:1.5em", "font-weight:bold", "line-height:1.8", "text-align:center", "text-align:right",
    "overflow:hidden", "overflow:auto", "white-space:pre", "white-space:nowrap", "flex-direction:column", "flex:1", "flex-wrap:wrap",
    "justify-content:space-between", "gap:4px", "grid-template-columns:1fr 2fr", "grid-template-columns:repeat(2,50px)", "--x:10px;width:var(--x)",
    "--x:30px;height:var(--x)", "counter-increment:n;", "vertical-align:middle", "text-indent:10px", "translate:5px 3px", "text-decoration:underline",
    "visibility:hidden", "box-shadow:2px 2px 3px #000", "border-radius:4px", "text-transform:uppercase", "word-spacing:3px", "box-sizing:border-box",
    "content:\"x\"", "content:counter(n) \".\"", "list-style-type:square",
];

/// QINC1k1: fractional heights, far-down spacers and `%` heights of out-of-flow boxes against auto-height
/// containing blocks (translation invariance in y), mixed in only by [`qinc1k1_fuzz_fractional_heights_far_down`]
/// so the default window's cases (and [`REGRESSION_SEEDS`]) stay what they were.
const YDRIFT_DECLS: &[&str] = &[
    "height:32.1px", "height:17.3px", "height:4097.7px", "position:relative",
    "position:absolute;inset:0;height:100%", "position:absolute;top:0;bottom:0;height:50%", "margin-top:3.3px",
    "padding:2.7px 0", "min-height:33.3%",
];

thread_local! {
    static YDRIFT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Kept below ~16 000 px of page: far beyond that the f32 ulp (0.002-0.008) makes a shifted box's
/// geometry differ from a fresh sum by more than the comparator's `EPS` (0.01) after a few roundings,
/// which is the absolute-coordinate limit (R60 14.1, QINC1n), not a translation-invariance bug.
const YDRIFT_SPACERS: &[&str] = &["100px", "4100.3px", "4300.7px", "8191.9px", "3.3px", "6143.9px"];

/// QINC1k1: a spacer `#ys` first (its height is what [`mutate_once`] moves), then a positioned auto-height box
/// holding an `inset:0; height:100%` child and rows of fractional heights mixed with random content, and the
/// random body around it.
fn ydrift_body(r: &mut Rng, body: String) -> String {
    let disp = r.pick(&["", "display:flex;flex-direction:column", "display:flex;flex-wrap:wrap", "display:grid", "display:flow-root"]);
    let mut rows = String::new();
    for _ in 0..(3 + r.below(20)) {
        if r.pct(25) {
            let n = 1 + r.below(3);
            rows.push_str(&gen_fragment(r, n));
        } else {
            rows.push_str(&format!("<div style=\"height:{}.{}px\">r</div>", 5 + r.below(40), r.below(10)));
        }
    }
    format!(
        "<div id=\"ys\" style=\"height:{}\"></div><div class=\"c{}\" style=\"position:relative;{disp}\"><div style=\"position:absolute;inset:0;height:100%\"></div>{rows}</div>{body}",
        r.pick(YDRIFT_SPACERS),
        r.below(5)
    )
}

/// A declaration; with [`YDRIFT`] on, 30 % of them from [`YDRIFT_DECLS`] (off: the RNG is used as before).
fn pick_decl(r: &mut Rng) -> &'static str {
    if YDRIFT.with(|y| y.get()) && r.pct(30) { r.pick(YDRIFT_DECLS) } else { r.pick(DECLS) }
}

const TAGS: &[&str] = &["div", "div", "div", "span", "span", "p", "ul", "li", "li", "a", "b", "section"];
const WORDS: &[&str] = &["lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit", "wide-word-that-is-long", "a", "ff", "Wi"];
const WIDTHS: &[f32] = &[320.0, 640.0, 1024.0, 1280.0];
/// QINC1k4: viewport heights, and declarations that read the viewport height (a `%`-bearing `calc()`
/// captures the whole viewport pair). Drawn from a SECOND generator per seed so the main stream (and
/// with it every page, mutation and `REGRESSION_SEEDS` case) is the one the earlier windows ran.
const HEIGHTS: &[f32] = &[312.0, 485.0, 700.0, 900.0];
const VH_SELECTORS: &[&str] = &[".c0::before", ".c3::after", "p::before", ".c1", "div", "span", "li::marker", ".c2"];
const VH_DECLS: &[&str] = &[
    "height:20vh", "min-height:10vmin", "width:calc(100% - 4px)", "height:calc(50% + 10vh)", "padding-top:2vmax",
    "margin-top:calc(10% - 1vh)", "width:calc(30vh - 20px)", "max-height:calc(100% - 5vh)", "font-size:calc(8px + 1vh)",
];

/// The QINC1k4 viewport-height rules of a case (empty for half the seeds).
fn gen_vh_css(h: &mut Rng) -> String {
    if !h.pct(50) {
        return String::new();
    }
    (0..(1 + h.below(3)))
        .map(|_| {
            let sel = h.pick(VH_SELECTORS);
            let content = if sel.ends_with("::before") || sel.ends_with("::after") { "content:\"v\";display:inline-block;" } else { "" };
            format!("{sel}{{{content}{}}}\n", h.pick(VH_DECLS))
        })
        .collect()
}

fn gen_css(r: &mut Rng) -> String {
    let mut css = String::new();
    for _ in 0..(4 + r.below(10)) {
        let sel = r.pick(SELECTORS);
        let mut body = String::new();
        for _ in 0..(1 + r.below(3)) {
            body.push_str(pick_decl(r));
            body.push(';');
        }
        css.push_str(&format!("{sel}{{{body}}}\n"));
    }
    css
}

fn gen_text(r: &mut Rng) -> String {
    (0..(1 + r.below(10))).map(|_| r.pick(WORDS)).collect::<Vec<_>>().join(" ")
}

fn gen_attrs(r: &mut Rng, next_id: &mut usize) -> String {
    let mut a = String::new();
    if r.pct(70) {
        let cls: Vec<String> = (0..(1 + r.below(2))).map(|_| format!("c{}", r.below(5))).collect();
        a.push_str(&format!(" class=\"{}\"", cls.join(" ")));
    }
    if r.pct(25) {
        *next_id += 1;
        a.push_str(&format!(" id=\"i{}\"", *next_id % 6));
    }
    if r.pct(15) {
        a.push_str(&format!(" style=\"{}\"", pick_decl(r)));
    }
    a
}

fn gen_node(r: &mut Rng, depth: usize, budget: &mut usize, next_id: &mut usize, out: &mut String) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    if r.pct(20) {
        out.push_str(&gen_text(r));
        return;
    }
    if r.pct(4) {
        out.push_str("<br>");
        return;
    }
    if r.pct(3) {
        out.push_str(&format!("<img src=\"{}\" width=\"{}\">", tiny_png_data_uri(2, 2), 8 + r.below(40)));
        return;
    }
    let tag = r.pick(TAGS);
    out.push_str(&format!("<{tag}{}>", gen_attrs(r, next_id)));
    if depth < 4 {
        for _ in 0..r.below(4) {
            gen_node(r, depth + 1, budget, next_id, out);
        }
    } else {
        out.push_str(&gen_text(r));
    }
    out.push_str(&format!("</{tag}>"));
}

fn gen_fragment(r: &mut Rng, budget: usize) -> String {
    let mut b = budget;
    let mut out = String::new();
    let mut id = r.below(6);
    while b > 0 {
        gen_node(r, 1, &mut b, &mut id, &mut out);
    }
    out
}

fn gen_page(r: &mut Rng) -> String {
    let n = 8 + r.below(20);
    let mut body = gen_fragment(r, n);
    if YDRIFT.with(|y| y.get()) {
        body = ydrift_body(r, body);
    }
    format!("<html><head><style>{}</style></head><body>{body}</body></html>", gen_css(r))
}

// ---------- DOM mutations (the Rust-side equivalent of a script's writes) ----------

fn all_elements(h: &Handle, out: &mut Vec<Handle>) {
    if let NodeData::Element { name, .. } = &h.data {
        if matches!(name.local.as_ref(), "style" | "script" | "head") {
            return;
        }
        out.push(h.clone());
    }
    for c in h.children.borrow().iter() {
        all_elements(c, out);
    }
}

fn all_texts(h: &Handle, out: &mut Vec<Handle>) {
    if matches!(&h.data, NodeData::Element { name, .. } if matches!(name.local.as_ref(), "style" | "script" | "head")) {
        return;
    }
    if matches!(h.data, NodeData::Text { .. }) {
        out.push(h.clone());
    }
    for c in h.children.borrow().iter() {
        all_texts(c, out);
    }
}

fn is_inside(anc: &Handle, mut n: Handle) -> bool {
    loop {
        if Rc::ptr_eq(anc, &n) {
            return true;
        }
        match parent_handle(&n) {
            Some(p) => n = p,
            None => return false,
        }
    }
}

/// Whether `h` is in a document (its root is a `Document` node): only such writes are logged, as the
/// natives do.
fn connected(h: &Handle) -> bool {
    let mut n = h.clone();
    while let Some(p) = parent_handle(&n) {
        n = p;
    }
    matches!(n.data, NodeData::Document)
}

fn detach_node(h: &Handle) {
    if let Some(p) = parent_handle(h) {
        let log = connected(&p);
        let (prev, next) = crate::incr_log::siblings(h);
        p.children.borrow_mut().retain(|c| !Rc::ptr_eq(c, h));
        if log {
            crate::incr_log::note(node_id(&p), crate::incr_log::Change::ChildRemove { child: node_id(h), prev, next });
        }
    }
    h.parent.set(None);
}

fn insert_node(parent: &Handle, child: &Handle, at: usize) {
    detach_node(child);
    child.parent.set(Some(Rc::downgrade(parent)));
    {
        let mut kids = parent.children.borrow_mut();
        let at = at.min(kids.len());
        kids.insert(at, child.clone());
    }
    if connected(parent) {
        let (prev, next) = crate::incr_log::siblings(child);
        crate::incr_log::note(node_id(parent), crate::incr_log::Change::ChildInsert { child: node_id(child), prev, next });
    }
}

/// `js_dom::set_attr` plus the log entry its native writes.
fn set_attr_logged(h: &Handle, name: &str, v: &str) {
    let old = element_attr(h, name).map(|s| s.to_string());
    crate::js_dom::set_attr(h, name, v);
    if old.as_deref() != Some(v) && connected(h) {
        crate::incr_log::note(
            node_id(h),
            crate::incr_log::Change::Attr {
                name: name.into(),
                old: crate::incr_log::Val::of(old.as_deref()),
                new: crate::incr_log::Val::of(Some(v)),
            },
        );
    }
}

/// A text node's data, plus its log entry.
fn set_text_logged(t: &Handle, v: &str) {
    if let NodeData::Text { contents } = &t.data {
        *contents.borrow_mut() = v.into();
    }
    if connected(t) {
        crate::incr_log::note(node_id(t), crate::incr_log::Change::Text);
    }
}

fn parse_fragment(html: &str) -> Vec<Handle> {
    let doc = parse_html(&format!("<html><body>{html}</body></html>"));
    let body = find_by_tag(&doc, "body").unwrap();
    let kids: Vec<Handle> = body.children.borrow().iter().cloned().collect();
    for k in &kids {
        detach_node(k);
    }
    kids
}

fn label(h: &Handle) -> String {
    Names::from_dom(h).of(node_id(h))
}

/// One random mutation; returns what it did (for the failure report).
fn mutate_once(r: &mut Rng, doc: &Handle, width: &mut f32) -> String {
    let body = find_by_tag(doc, "body").unwrap();
    let mut els = Vec::new();
    all_elements(&body, &mut els);
    let target = els[r.below(els.len())].clone();
    if YDRIFT.with(|y| y.get())
        && r.pct(40)
        && let Some(ys) = els.iter().find(|e| element_attr(e, "id").is_some_and(|v| &*v == "ys"))
    {
        let v = format!("height:{}", r.pick(YDRIFT_SPACERS));
        set_attr_logged(ys, "style", &v);
        return format!("spacer #ys {v:?}");
    }
    match r.below(10) {
        0 | 1 => {
            let n = 1 + r.below(6);
            let html = gen_fragment(r, n);
            let kids = parse_fragment(&html);
            let at = r.below(target.children.borrow().len() + 1);
            for (i, k) in kids.iter().enumerate() {
                insert_node(&target, k, at + i);
            }
            format!("insert into {} at {at}: {html}", label(&target))
        }
        2 => {
            if Rc::ptr_eq(&target, &body) {
                return "remove: skipped (body)".into();
            }
            detach_node(&target);
            format!("remove {}", label(&target))
        }
        3 => {
            let dest = els[r.below(els.len())].clone();
            if is_inside(&target, dest.clone()) || Rc::ptr_eq(&target, &body) {
                return "move: skipped".into();
            }
            let at = r.below(dest.children.borrow().len() + 1);
            insert_node(&dest, &target, at);
            format!("move {} into {} at {at}", label(&target), label(&dest))
        }
        4 => {
            let mut texts = Vec::new();
            all_texts(&body, &mut texts);
            if texts.is_empty() {
                return "text: skipped".into();
            }
            let t = texts[r.below(texts.len())].clone();
            let new = gen_text(r);
            set_text_logged(&t, &new);
            format!("text -> {new:?}")
        }
        5 => {
            let v = format!("c{} c{}", r.below(5), r.below(5));
            set_attr_logged(&target, "class", &v);
            format!("class of {} = {v:?}", label(&target))
        }
        6 => {
            let v = pick_decl(r).to_string();
            set_attr_logged(&target, "style", &v);
            format!("style of {} = {v:?}", label(&target))
        }
        7 => {
            let v = format!("i{}", r.below(6));
            set_attr_logged(&target, "id", &v);
            format!("id of {} = {v:?}", label(&target))
        }
        8 => {
            if r.pct(30) {
                set_interaction(&InteractionState::default());
                return "hover cleared".into();
            }
            let st = InteractionState { hover: ancestor_ids(&target).into_iter().collect(), ..Default::default() };
            set_interaction(&st);
            format!("hover {}", label(&target))
        }
        _ => {
            if r.pct(50) {
                *width = r.pick(WIDTHS);
                format!("viewport width {width}")
            } else {
                // replace the stylesheet text
                let mut styles = Vec::new();
                fn find_styles(h: &Handle, out: &mut Vec<Handle>) {
                    if matches!(&h.data, NodeData::Element { name, .. } if name.local.as_ref() == "style") {
                        out.push(h.clone());
                    }
                    for c in h.children.borrow().iter() {
                        find_styles(c, out);
                    }
                }
                find_styles(doc, &mut styles);
                let css = gen_css(r);
                let text = styles.first().and_then(|s| s.children.borrow().first().cloned());
                if let Some(t) = text {
                    set_text_logged(&t, &css);
                }
                format!("stylesheet -> {css:?}")
            }
        }
    }
}

// ---------- from-scratch layout ----------

struct Run {
    tree: LayoutBox,
    laid: Laid,
    cache: ShapeCache,
    #[allow(dead_code)] // read by `compare_runs` callers again from QINC1g/h1
    complete: bool,
}

fn layout_from_scratch(doc: &Handle, width: f32, vh: f32, fs: &mut FontSystem) -> Run {
    let tree = build_layout_tree_sized(doc, width, vh);
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, fs, width, vh, &mut next_id, &mut cache, deadline, &mut incomplete);
    Run { tree, laid, cache, complete: !incomplete }
}

/// QINC1a: A and B both from scratch. Unused since QINC1f (the second run is the incremental
/// restyle, checked inside the build); QINC1g / QINC1h1 compare their incremental boxes / laid trees here.
#[allow(dead_code)]
fn compare_runs(doc: &Handle, a: &Run, b: &Run) -> Result<(), String> {
    let names = Names::from_dom(doc);
    let path = root_path(&names, a.tree.style.node);
    cmp_boxes(&names, &path, &a.tree, &b.tree).map_err(|m| format!("box tree {m}"))?;
    cmp_laid(&names, &path, &a.laid, &a.cache, &b.laid, &b.cache).map_err(|m| format!("laid tree {m}"))
}

fn fuzz_case(seed: u64, fs: &mut FontSystem) {
    let mut r = Rng::new(seed);
    // QINC1k4: viewport heights and `vh` rules from their own stream (see `HEIGHTS`)
    let mut hr = Rng::new(seed ^ 0x5648_5648_5648_5648);
    let mut html = gen_page(&mut r);
    let vh_css = gen_vh_css(&mut hr);
    if !vh_css.is_empty() {
        // a second sheet: the stylesheet mutation replaces the first one only
        html = html.replacen("</style></head>", &format!("</style><style>{vh_css}</style></head>"), 1);
    }
    let doc = parse_html(&html);
    let mut width = r.pick(WIDTHS);
    let mut vh = 700.0;
    let mut log: Vec<String> = vec![format!("initial page: {html}")];
    set_interaction(&InteractionState::default());
    let _ = crate::incr_log::drain();
    // The log alone feeds the engine (no reconcile); a mismatch is collected, not panicked on.
    let saved = crate::incr_style::set_seam(crate::incr_style::Seam { shadow: true, reconcile: false, panic: false });
    let _ = crate::incr_style::take_mismatches();
    let mut incremental = 0;
    for step in 0..(3 + r.below(4)) {
        if step > 0 {
            for _ in 0..(1 + r.below(3)) {
                let what = mutate_once(&mut r, &doc, &mut width);
                log.push(format!("step {step}: {what}"));
            }
            if hr.pct(30) {
                vh = hr.pick(HEIGHTS);
                log.push(format!("step {step}: viewport height {vh}"));
            }
        }
        // A: from scratch; B: the incremental restyle the build's shadow run computed.
        let a = layout_from_scratch(&doc, width, vh, fs);
        let mut mism = crate::incr_style::take_mismatches();
        // (a style mismatch drops the style engine: report the mismatch, not the missing engine)
        let full = crate::incr_style::with_engine(|e| e.map(|e| e.stats.full)).unwrap_or(true);
        incremental += usize::from(!full);
        // QINC1g: the kept box tree after this step equals the from-scratch tree (the build checks it
        // too; this also covers a kept tree a future change stops checking inside the build).
        let bs = crate::incr_box::with_engine(|e| {
            let Some(e) = e else { return Default::default() };
            if mism.is_empty() {
                let names = Names::from_dom(&doc);
                if let Err(m) = cmp_boxes(&names, &root_path(&names, 0), &a.tree, e.tree()) {
                    mism.push(format!("kept box tree {m}"));
                }
            }
            e.stats.clone()
        });
        FUZZ_BOXES.with(|c| {
            let (n, rb, ru) = c.get();
            c.set((n + usize::from(bs.rebuilt > 0), rb + bs.rebuilt as usize, ru + bs.reused as usize));
        });
        // QINC1h1: the engine's relayout of the kept tree equals the from-scratch laid tree (the layout
        // checks it too, inside `measure_document`).
        if mism.is_empty() {
            crate::incr_layout::with_engine(|e| {
                // (a document whose tree has no children never got an engine)
                let Some(e) = e.filter(|e| e.doc_key() == node_id(&doc)) else { return };
                let names = Names::from_dom(&doc);
                if let Err(m) = cmp_laid(&names, &root_path(&names, 0), &a.laid, &a.cache, e.laid(), e.cache()) {
                    mism.push(format!("kept laid tree {m}"));
                }
                let s = &e.stats;
                FUZZ_LAYOUT.with(|c| {
                    let (n, h, sp) = c.get();
                    c.set((n + usize::from(!s.full), h + s.hits as usize, sp + s.splices as usize));
                });
            });
        }
        if !mism.is_empty() {
            let report = format!("incr fuzz mismatch: seed {seed} step {step} (width {width}, height {vh}):\n{}\n{}", mism.join("\n"), log.join("\n"));
            if env::var("SCRATCHY_FUZZ_KEEP_GOING").is_ok_and(|v| v == "1") {
                // a long local run: note the case and go on with the next one
                diag!("{}", report.lines().take(3).collect::<Vec<_>>().join(" | "));
                FUZZ_FAILED.with(|c| c.set(c.get() + 1));
                break;
            }
            set_interaction(&InteractionState::default());
            crate::incr_style::set_seam(saved);
            panic!("{report}");
        }
    }
    crate::incr_style::set_seam(saved);
    set_interaction(&InteractionState::default());
    FUZZ_INCREMENTAL.with(|c| c.set(c.get() + incremental));
}

thread_local! {
    /// Steps the engine answered incrementally (not a full restyle), for the run's report.
    static FUZZ_INCREMENTAL: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Cases that failed under `SCRATCHY_FUZZ_KEEP_GOING=1`.
    static FUZZ_FAILED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// QINC1g: (steps with an incremental box rebuild, boxes rebuilt, boxes reused).
    static FUZZ_BOXES: std::cell::Cell<(usize, usize, usize)> = const { std::cell::Cell::new((0, 0, 0)) };
    /// QINC1h1: (incremental relayouts, cache hits, bottom-up splices).
    static FUZZ_LAYOUT: std::cell::Cell<(usize, usize, usize)> = const { std::cell::Cell::new((0, 0, 0)) };
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// QINC1f-fix: seeds that once failed outside the default window, run on every default run (each has a named
/// minimal test in incr_style_tests.rs as well): `:first-child` / `:nth-child` / sibling marks after several
/// inserts in one batch (462 519 733 1956 2080 2084 2475), and a stale child list after a freed removal (2475).
const REGRESSION_SEEDS: &[u64] = &[462, 519, 733, 1956, 2080, 2084, 2475];

/// Default window: 1 000 cases in release (CI runs `cargo test --release`; ~6 s locally), 200 in a debug build
/// (~10x slower per case). `SCRATCHY_FUZZ_N` for more, `SCRATCHY_FUZZ_SEED` to move the window; with neither set,
/// [`REGRESSION_SEEDS`] run too.
#[test]
fn qinc1a_fuzz_two_from_scratch_layouts_never_differ() {
    let default_n = if cfg!(debug_assertions) { 200 } else { 1000 };
    let n = env_usize("SCRATCHY_FUZZ_N", default_n);
    let base = env_usize("SCRATCHY_FUZZ_SEED", 0) as u64;
    let custom = env::var("SCRATCHY_FUZZ_N").is_ok() || env::var("SCRATCHY_FUZZ_SEED").is_ok();
    // QINC1i4: `SCRATCHY_FUZZ_ADOPT_VERIFY=0` adopts the build's styles on full updates as
    // `SCRATCHY_INCR=1` does (no verifying cascade; the box and laid-tree compares still run).
    let verify = crate::incr_style::set_adopt_verify(!env::var("SCRATCHY_FUZZ_ADOPT_VERIFY").is_ok_and(|v| v == "0"));
    let mut fs = new_font_system();
    let t0 = std::time::Instant::now();
    let window = base..base + n as u64;
    let extra = REGRESSION_SEEDS.iter().copied().filter(|s| !custom && !window.contains(s));
    for seed in window.clone().chain(extra) {
        fuzz_case(seed, &mut fs);
    }
    let inc = FUZZ_INCREMENTAL.with(|c| c.replace(0));
    let (bsteps, rebuilt, reused) = FUZZ_BOXES.with(|c| c.replace((0, 0, 0)));
    let (lsteps, hits, splices) = FUZZ_LAYOUT.with(|c| c.replace((0, 0, 0)));
    diag!(
        "qinc1a/qinc1f/qinc1g/qinc1h1 fuzz: {n} cases in {:.1}s, {inc} incremental restyle steps, {bsteps} incremental box rebuilds ({rebuilt} boxes rebuilt, {reused} reused), {lsteps} incremental relayouts ({hits} cache hits, {splices} bottom-up splices)",
        t0.elapsed().as_secs_f64()
    );
    crate::incr_style::set_adopt_verify(verify);
    let failed = FUZZ_FAILED.with(|c| c.replace(0));
    assert_eq!(failed, 0, "{failed} failing cases (SCRATCHY_FUZZ_KEEP_GOING=1 printed them)");
    assert!(n < 20 || inc > n, "the fuzz must exercise incremental updates, not only full ones ({inc} for {n} cases)");
    assert!(n < 20 || (bsteps > n && reused > 0), "the fuzz must exercise incremental box rebuilds ({bsteps} for {n} cases, {reused} reused)");
    assert!(n < 20 || (lsteps > n && hits > 0 && splices > 0), "the fuzz must exercise incremental relayouts ({lsteps} for {n} cases, {hits} hits, {splices} splices)");
}

/// QINC1k1: the fuzz with [`YDRIFT_DECLS`] mixed in (fractional heights summed far down the page, moved by
/// mutations above them). 100 cases in debug, 500 in release; `SCRATCHY_FUZZ_N` / `SCRATCHY_FUZZ_SEED` as above
/// (seeds start at 1 000 000 + `SCRATCHY_FUZZ_SEED`).
#[test]
fn qinc1k1_fuzz_fractional_heights_far_down() {
    let n = env_usize("SCRATCHY_FUZZ_N", if cfg!(debug_assertions) { 100 } else { 500 });
    let base = 1_000_000 + env_usize("SCRATCHY_FUZZ_SEED", 0) as u64;
    let mut fs = new_font_system();
    YDRIFT.with(|y| y.set(true));
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            YDRIFT.with(|y| y.set(false));
        }
    }
    let _off = Off;
    for seed in base..base + n as u64 {
        fuzz_case(seed, &mut fs);
    }
    let failed = FUZZ_FAILED.with(|c| c.replace(0));
    assert_eq!(failed, 0, "{failed} failing cases (SCRATCHY_FUZZ_KEEP_GOING=1 printed them)");
}

// ---------- generator sanity ----------

#[test]
fn qinc1a_fuzz_generator_is_deterministic_and_varied() {
    let a = gen_page(&mut Rng::new(7));
    let b = gen_page(&mut Rng::new(7));
    let c = gen_page(&mut Rng::new(8));
    assert_eq!(a, b);
    assert_ne!(a, c);
    // every seeded page parses and has a body with content
    for s in 0..20 {
        let doc = parse_html(&gen_page(&mut Rng::new(s)));
        assert!(find_by_tag(&doc, "body").is_some());
    }
}

#[test]
fn qinc1a_fuzz_mutations_change_the_dom() {
    let mut r = Rng::new(3);
    let doc = parse_html(&gen_page(&mut r));
    let before = Names::from_dom(&doc);
    let mut w = 1024.0;
    let mut did = Vec::new();
    for _ in 0..40 {
        did.push(mutate_once(&mut r, &doc, &mut w));
    }
    set_interaction(&InteractionState::default());
    assert!(did.iter().any(|d| d.starts_with("insert")), "{did:?}");
    assert!(did.iter().any(|d| d.starts_with("class")), "{did:?}");
    let _ = before;
}

// ---------- comparators detect differences ----------

fn runs_of(html_a: &str, html_b: &str, width: f32) -> (Handle, Run, Handle, Run) {
    let mut fs = new_font_system();
    let da = parse_html(html_a);
    let db = parse_html(html_b);
    let a = layout_from_scratch(&da, width, 700.0, &mut fs);
    let b = layout_from_scratch(&db, width, 700.0, &mut fs);
    (da, a, db, b)
}

#[test]
fn qinc1a_cmp_laid_reports_node_path_field_and_values() {
    let (da, a, _db, b) = runs_of(
        "<body style='margin:0'><div id=a><p class=x style='height:10px'>hi</p></div></body>",
        "<body style='margin:0'><div id=a><p class=x style='height:30px'>hi</p></div></body>",
        400.0,
    );
    let names = Names::from_dom(&da);
    // The two documents are different DOMs, so node ids differ; the report still names the path by A's names where known.
    let m = cmp_laid(&names, "root", &a.laid, &a.cache, &b.laid, &b.cache).unwrap_err();
    let s = m.to_string();
    assert!(s.contains("style.height") || s.contains(": h:"), "{s}");
    assert!(s.starts_with("at root"), "{s}");
}

#[test]
fn qinc1a_cmp_laid_finds_a_position_difference_with_both_values() {
    let doc = parse_html("<body style='margin:0'><div id=a style='width:100px;height:20px'>x</div></body>");
    let mut fs = new_font_system();
    let a = layout_from_scratch(&doc, 400.0, 700.0, &mut fs);
    let mut b = layout_from_scratch(&doc, 400.0, 700.0, &mut fs);
    let names = Names::from_dom(&doc);
    assert!(compare_runs(&doc, &a, &b).is_ok());
    // nudge the deepest-first child by more than the tolerance
    fn first_leaf(l: &mut Laid) -> &mut Laid {
        if matches!(&l.content, LaidContent::Blocks(c) if !c.is_empty()) {
            let LaidContent::Blocks(c) = &mut l.content else { unreachable!() };
            return first_leaf(&mut c[0]);
        }
        l
    }
    first_leaf(&mut b.laid).x += 0.5;
    let m = cmp_laid(&names, "root", &a.laid, &a.cache, &b.laid, &b.cache).unwrap_err();
    assert_eq!(m.field, "x");
    assert!(m.a != m.b);
    // inside tolerance is equal
    first_leaf(&mut b.laid).x -= 0.495;
    assert!(cmp_laid(&names, "root", &a.laid, &a.cache, &b.laid, &b.cache).is_ok());
}

#[test]
fn qinc1a_cmp_boxes_sees_text_and_style_but_not_tree_order_or_node() {
    let doc = parse_html("<body><p class=x>hello <b>w</b></p></body>");
    let mut fs = new_font_system();
    let a = layout_from_scratch(&doc, 300.0, 700.0, &mut fs);
    let b = layout_from_scratch(&doc, 300.0, 700.0, &mut fs);
    let names = Names::from_dom(&doc);
    assert!(cmp_boxes(&names, "root", &a.tree, &b.tree).is_ok());
    // The same DOM, mutated: the text of the <b>, then the display of the <p>.
    let bt = find_by_tag(&doc, "b").unwrap();
    let t = bt.children.borrow()[0].clone();
    if let NodeData::Text { contents } = &t.data {
        *contents.borrow_mut() = "W".into();
    }
    let c = layout_from_scratch(&doc, 300.0, 700.0, &mut fs);
    let m = cmp_boxes(&names, "root", &a.tree, &c.tree).unwrap_err();
    assert!(m.field.contains("text"), "{m}");
    assert!(m.path.contains("p.x"), "{m}");
    crate::js_dom::set_attr(&find_by_tag(&doc, "p").unwrap(), "style", "display:flex");
    let d = layout_from_scratch(&doc, 300.0, 700.0, &mut fs);
    let m = cmp_boxes(&names, "root", &a.tree, &d.tree).unwrap_err();
    assert!(m.to_string().contains("display"), "{m}");
}

// ---------- style_diff ----------

fn base_style() -> ComputedStyle {
    let doc = parse_html("<body><div>x</div></body>");
    let tree = build_layout_tree(&doc, 400.0);
    fn first_div(b: &LayoutBox) -> Option<ComputedStyle> {
        match &b.content {
            BoxContent::Blocks(c) => c.iter().find_map(first_div).or(Some(b.style)),
            _ => Some(b.style),
        }
    }
    first_div(&tree).unwrap()
}

#[test]
fn qinc1a_style_diff_identical_and_identity_fields_are_none() {
    let a = base_style();
    let mut b = a;
    assert_eq!(style_diff(&a, &b), StyleDiff::None);
    b.node = a.node.wrapping_add(8);
    b.tree_order = a.tree_order + 99;
    assert_eq!(style_diff(&a, &b), StyleDiff::None);
    assert!(cmp_styles("p", &a, &b).is_ok());
    assert!(style_changes(&a, &b).is_empty());
}

#[test]
fn qinc1a_style_diff_one_field_of_each_class() {
    let a = base_style();
    let mut b = a;
    b.color = TextColor::rgb(1, 2, 3);
    assert_eq!(style_diff(&a, &b), StyleDiff::Paint);
    let mut b = a;
    b.translate = Some((TranslateAxis { px: 5.0, frac: 0.0 }, TranslateAxis { px: 0.0, frac: 0.0 }));
    assert_eq!(style_diff(&a, &b), StyleDiff::Offset);
    let mut b = a;
    b.font_size = a.font_size + 3.0;
    assert_eq!(style_diff(&a, &b), StyleDiff::Layout);
    let mut b = a;
    b.display = if a.display == Display::Flex { Display::Block } else { Display::Flex };
    assert_eq!(style_diff(&a, &b), StyleDiff::Box);
    // the strongest class wins
    let mut b = a;
    b.color = TextColor::rgb(9, 9, 9);
    b.font_size = a.font_size + 1.0;
    b.float = Float::Left;
    assert_eq!(style_diff(&a, &b), StyleDiff::Box);
    let ch = style_changes(&a, &b);
    assert_eq!(ch.iter().map(|c| c.field).collect::<Vec<_>>(), vec!["color", "float", "font_size"]);
    let m = cmp_styles("div", &a, &b).unwrap_err();
    assert_eq!(m.field, "style.color");
}

#[test]
fn qinc1a_style_diff_opacity_and_z_index_are_layout() {
    let a = base_style();
    assert_eq!(a.opacity, 1.0);
    let mut b = a;
    b.opacity = 0.5;
    assert_eq!(style_diff(&a, &b), StyleDiff::Layout, "1 -> <1 creates a stacking boundary");
    assert_eq!(style_diff(&b, &a), StyleDiff::Layout, "<1 -> 1 removes it");
    let mut c = b;
    c.opacity = 0.6;
    assert_eq!(style_diff(&b, &c), StyleDiff::Paint, "<1 -> <1 only paints");
    let mut d = a;
    d.z_index = Some(2);
    assert_eq!(style_diff(&a, &d), StyleDiff::Layout);
    let mut e = d;
    e.z_index = Some(3);
    assert_eq!(style_diff(&d, &e), StyleDiff::Layout);
}

#[test]
fn qinc1a_style_diff_classifies_real_cascade_changes() {
    let mk = |css: &str| {
        let doc = parse_html(&format!("<style>{css}</style><body><div id=t>x</div></body>"));
        let tree = build_layout_tree(&doc, 400.0);
        fn find(b: &LayoutBox, node: usize) -> Option<ComputedStyle> {
            if b.style.node == node {
                return Some(b.style);
            }
            match &b.content {
                BoxContent::Blocks(c) | BoxContent::TableRow(c) => c.iter().find_map(|x| find(x, node)),
                _ => None,
            }
        }
        let t = find_by_tag(&doc, "div").unwrap();
        find(&tree, node_id(&t)).unwrap()
    };
    let base = mk("");
    assert_eq!(style_diff(&base, &mk("#t{color:red}")), StyleDiff::Paint);
    assert_eq!(style_diff(&base, &mk("#t{position:relative;left:4px}")), StyleDiff::Box);
    assert_eq!(style_diff(&base, &mk("#t{width:50px}")), StyleDiff::Layout);
    assert_eq!(style_diff(&base, &mk("#t{display:flex}")), StyleDiff::Box);
    assert_eq!(style_diff(&base, &mk("#t{opacity:.5}")), StyleDiff::Layout);
}
