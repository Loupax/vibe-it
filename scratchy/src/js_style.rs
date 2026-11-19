//! QA16: `getComputedStyle`. The JS engine cannot see `App`, so this runs
//! the cascade itself, on demand, for one element: the page's `<style>`
//! text plus the external CSS the App fetched (`set_js_extra_css`) are
//! parsed into rules (re-parsed only when that text changes), and
//! `resolve_style_for` is run down the ancestor chain, so inheritance and
//! `em`/`rem` work. There is NO layout here: `width`/`height` read back
//! the computed (specified) value, `auto` when unset, not the used px.
//! Results are cached per element until the next DOM write (`DomTable::wgen`).
use super::*;

thread_local! {
    /// QY52: bumped when the external / adopted CSS text changes, so the
    /// style cache can tell "unchanged" without comparing megabytes.
    static CSS_VER: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
    /// QY52 test seams: cascades run / stylesheet pieces parsed by `StyleCache`.
    static CASCADES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// QSTK1b test seam: ancestor-chain elements visited by `style_chain` calls.
    static CHAIN_STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static PARSES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static EXTRA_CSS: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The external stylesheets of the current page (same text the layout
/// build receives as `extra_css`).
/// QMEM3b: `(1, capacity bytes)` of the page's extra CSS text kept for `getComputedStyle` (the
/// capacity, not the length: slack counts as resident).
pub(crate) fn extra_css_mem() -> (usize, usize) {
    EXTRA_CSS.with(|c| {
        let c = c.borrow();
        (usize::from(!c.is_empty()), c.capacity())
    })
}

pub(crate) fn set_js_extra_css(css: &str) {
    EXTRA_CSS.with(|c| {
        let mut c = c.borrow_mut();
        if *c != css {
            *c = css.to_string();
            CSS_VER.with(|v| v.set((v.get().0 + 1, v.get().1)));
        }
    });
}

thread_local! {
    static ADOPTED_CSS: RefCell<String> = const { RefCell::new(String::new()) };
}

/// QSD4: the document's adopted stylesheets (cascade after the page CSS).
pub(crate) fn set_js_adopted_css(css: &str) {
    ADOPTED_CSS.with(|c| {
        let mut c = c.borrow_mut();
        if *c != css {
            *c = css.to_string();
            CSS_VER.with(|v| v.set((v.get().0, v.get().1 + 1)));
        }
    });
}

/// QY53h: the version of the external + adopted CSS; changes whenever
/// `page_css()` would return different text. Cheap (no string is built).
pub(crate) fn css_ver() -> (u64, u64) {
    CSS_VER.with(|v| v.get())
}

#[cfg(test)]
thread_local! {
    /// Test seam: how many times `page_css()` built the string.
    pub(crate) static PAGE_CSS_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The page's external plus adopted CSS (what the layout build receives as
/// `extra_css`).
pub(crate) fn page_css() -> String {
    #[cfg(test)]
    PAGE_CSS_BUILDS.with(|c| c.set(c.get() + 1));
    let mut s = EXTRA_CSS.with(|c| c.borrow().clone());
    ADOPTED_CSS.with(|c| {
        let c = c.borrow();
        if !c.is_empty() {
            s.push('\n');
            s.push_str(&c);
        }
    });
    s
}

/// One `<style>` element's effective text (QY52). Only documents other than the page use this
/// (QMEM3e: the page's rules are `rules_cache`'s, shared with the layout build).
type PieceKey = String;

struct Piece {
    key: PieceKey,
    /// How many rules this piece contributed to `StyleCache::rules`.
    count: usize,
    layer: bool,
    has: bool,
    /// Some selector reads earlier siblings' attributes (`+`, `~`, `:nth-child(of)`).
    sib: bool,
}

struct Entry {
    fp: u64,
    /// Set for styles that depend on the clock (animations): valid for this generation only.
    wgen: Option<u64>,
    style: ComputedStyle,
}

/// QSTK1b: per-element callback of a `style_of_chain` walk.
type ChainVisit<'a> = dyn FnMut(&Handle, &ComputedStyle) + 'a;

#[derive(Default)]
pub(crate) struct StyleCache {
    wgen: u64,
    css_ver: (u64, u64),
    flags: String,
    pieces: Vec<Piece>,
    ua_count: usize,
    /// `@layer` rank numbering spans sheets, so such pages are parsed as one text.
    concat: bool,
    /// QMEM3e: the page's rules are `rules_cache`'s (its generation, not an `Rc`: a second
    /// owner would make every layout-side splice a full re-parse); `rules`/`pieces` stay empty.
    shared: bool,
    shared_gen: Option<u64>,
    /// Some rule uses `:has(`, which reads descendants: no per-element reuse across writes.
    has_rules: bool,
    sib_rules: bool,
    ua_flags: (bool, bool),
    rules: Option<Rc<Vec<StyleRule>>>,
    index: Option<RuleIndex>,
    map: HashMap<(usize, u8), Entry>,
    root_inherited: Option<(u64, InheritedStyle)>,
    /// QA18c: the document the cached rules/styles belong to.
    root_ptr: usize,
}

fn mix(h: u64, v: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut s = std::collections::hash_map::DefaultHasher::new();
    h.hash(&mut s);
    v.hash(&mut s);
    s.finish()
}

/// Hashes what a selector can read of an element (tag, attributes, whether it
/// is `:empty`); returns whether it is a form control (live state outside the attributes).
fn hash_el(n: &Handle, h: &mut std::collections::hash_map::DefaultHasher, full: bool) -> bool {
    use std::hash::Hash;
    let NodeData::Element { name, attrs, .. } = &n.data else { return false };
    name.local.as_ref().hash(h);
    if full {
        for a in attrs.borrow().iter() {
            a.name.local.as_ref().hash(h);
            let v: &str = &a.value;
            v.hash(h);
        }
        n.children
            .borrow()
            .iter()
            .any(|c| match &c.data {
                NodeData::Element { .. } => true,
                NodeData::Text { contents } => !contents.borrow().is_empty(),
                _ => false,
            })
            .hash(h);
    }
    matches!(name.local.as_ref(), "input" | "option" | "select" | "fieldset" | "optgroup" | "textarea" | "button")
}

/// A fingerprint of what selector matching of `n` can read besides its
/// ancestors: its attributes and `:empty`-ness, its siblings (tags of all,
/// attributes of the earlier ones). The second value says "form control around".
fn ctx_hash(n: &Handle, sib: bool) -> (u64, bool) {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut formish = hash_el(n, &mut h, true);
    if let Some(p) = parent_handle(n) {
        let mut before = true;
        for c in p.children.borrow().iter() {
            if Rc::ptr_eq(c, n) {
                before = false;
                1u8.hash(&mut h);
            } else if matches!(c.data, NodeData::Element { .. }) {
                formish |= hash_el(c, &mut h, before && sib);
            }
        }
    }
    (h.finish(), formish)
}

fn interaction_hash() -> u64 {
    INTERACTION.with(|i| {
        let i = i.borrow();
        let mut acc = 0u64;
        for set in [&i.hover, &i.active, &i.focus_within] {
            let x = set.iter().fold(0u64, |a, v| a.wrapping_add((*v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)));
            acc = mix(acc, x);
        }
        acc = mix(acc, i.focus.map_or(0, |f| f as u64 + 1));
        acc = mix(acc, i.target.map_or(0, |f| f as u64 + 1));
        mix(acc, i.focus_visible as u64)
    })
}

/// Cascades run by `getComputedStyle` so far on this thread (test seam, QY52).
#[cfg(test)]
pub(crate) fn cascade_count() -> u64 {
    CASCADES.with(|c| c.get())
}

/// Ancestor-chain steps taken by computed-style walks so far on this thread (test seam, QSTK1b).
#[cfg(test)]
pub(crate) fn chain_steps() -> u64 {
    CHAIN_STEPS.with(|c| c.get())
}

/// Stylesheet pieces parsed so far on this thread (test seam, QY52).
#[cfg(test)]
pub(crate) fn parse_count() -> u64 {
    PARSES.with(|c| c.get())
}

/// Whether a selector reads descendants (`:has`) or earlier siblings' attributes.
fn sel_flags(sel: &selectors::parser::Selector<OurSelectorImpl>, has: &mut bool, sib: &mut bool) {
    use selectors::parser::{Combinator, Component};
    for c in sel.iter_raw_match_order() {
        match c {
            Component::Combinator(Combinator::NextSibling | Combinator::LaterSibling) | Component::NthOf(_) => *sib = true,
            Component::Has(_) => *has = true,
            Component::Negation(l) | Component::Is(l) | Component::Where(l) => {
                l.slice().iter().for_each(|s| sel_flags(s, has, sib));
            }
            _ => {}
        }
    }
}

/// `(uses :has(, reads earlier siblings)` over `rules`.
pub(crate) fn rule_flags(rules: &[StyleRule]) -> (bool, bool) {
    let (mut has, mut sib) = (false, false);
    for r in rules {
        r.selectors.slice().iter().for_each(|s| sel_flags(s, &mut has, &mut sib));
    }
    (has, sib)
}

fn parse_piece(text: &str) -> (Vec<StyleRule>, bool, bool, bool) {
    PARSES.with(|c| c.set(c.get() + 1));
    let rules = parse_stylesheet(text);
    let (has, sib) = rule_flags(&rules);
    (rules, text.contains("@layer"), has, sib)
}

/// Properties reported by `length`/`item(i)`; every other name returns `''`.
pub(crate) const PROPS: &[&str] = &[
    "display", "position", "visibility", "color", "background-color", "opacity", "z-index", "float", "clear",
    "overflow", "overflow-x", "overflow-y", "width", "height", "min-width", "max-width", "min-height", "max-height",
    "top", "right", "bottom", "left", "margin-top", "margin-right", "margin-bottom", "margin-left", "padding-top",
    "padding-right", "padding-bottom", "padding-left", "border-top-width", "border-right-width",
    "border-bottom-width", "border-left-width", "border-top-style", "border-right-style", "border-bottom-style",
    "border-left-style", "border-top-color", "border-right-color", "border-bottom-color", "border-left-color",
    "box-sizing", "font-size", "font-weight", "font-style", "font-family", "line-height", "text-align",
    "text-transform", "white-space", "cursor", "vertical-align",
];

fn kebab(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            if !o.is_empty() {
                o.push('-');
            }
            o.push(c.to_ascii_lowercase());
        } else {
            o.push(c);
        }
    }
    o
}

fn num(v: f32) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

fn px(v: f32) -> String {
    format!("{}px", num(v))
}

fn color(c: TextColor) -> String {
    if c.a() == 255 {
        format!("rgb({}, {}, {})", c.r(), c.g(), c.b())
    } else {
        format!("rgba({}, {}, {}, {})", c.r(), c.g(), c.b(), num(c.a() as f32 / 255.0))
    }
}

fn opt_color(c: Option<TextColor>) -> String {
    c.map(color).unwrap_or_else(|| "rgba(0, 0, 0, 0)".to_string())
}

fn box_len(l: BoxLength) -> String {
    match l {
        BoxLength::Px(v) => px(v),
        BoxLength::Percent(v) => format!("{}%", num(v)),
        BoxLength::Calc(..) => "auto".to_string(),
    }
}

fn opt_len(l: Option<BoxLength>) -> String {
    l.map(box_len).unwrap_or_else(|| "auto".to_string())
}

fn border_style(s: BorderStyle) -> &'static str {
    match s {
        BorderStyle::Solid => "solid",
        BorderStyle::Dashed => "dashed",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Double => "double",
        BorderStyle::Groove => "groove",
        BorderStyle::Ridge => "ridge",
        BorderStyle::Inset => "inset",
        BorderStyle::Outset => "outset",
        _ => "none",
    }
}

fn overflow(o: Overflow) -> &'static str {
    match o {
        Overflow::Visible => "visible",
        Overflow::Hidden => "hidden",
        Overflow::Scroll => "scroll",
        Overflow::Auto => "auto",
    }
}

fn display(s: &ComputedStyle) -> &'static str {
    match s.display {
        Display::None => "none",
        Display::Inline => "inline",
        Display::Block if s.list_item => "list-item",
        Display::Block if s.flow_root => "flow-root",
        Display::Block if s.table_caption => "table-caption",
        Display::Block => "block",
        Display::Table => "table",
        Display::TableRow => "table-row",
        Display::TableCell => "table-cell",
        Display::TableGroup => "table-row-group",
        Display::Flex => "flex",
        Display::Grid => "grid",
        Display::InlineBlock => "inline-block",
        Display::InlineFlex => "inline-flex",
    }
}

/// The CSSOM serialisation of `prop` (dashed name) for `s`; `''` if unknown.
pub(crate) fn serialize(s: &ComputedStyle, prop: &str) -> String {
    let side = |p: &str| match p {
        "top" => 0,
        "right" => 1,
        "bottom" => 2,
        _ => 3,
    };
    if prop.starts_with("--") {
        return s.custom_props.and_then(|m| m.get(prop)).unwrap_or_default();
    }
    match prop {
        "display" => display(s).into(),
        "position" => match s.position {
            Position::Static => "static",
            Position::Relative => "relative",
            Position::Absolute => "absolute",
            Position::Fixed => "fixed",
            Position::Sticky => "sticky",
        }
        .into(),
        "visibility" => if s.visibility == Visibility::Hidden { "hidden" } else { "visible" }.into(),
        "color" => color(s.color),
        "background-color" => opt_color(s.background_color),
        "opacity" => num(s.opacity),
        "z-index" => s.z_index.map(|z| z.to_string()).unwrap_or_else(|| "auto".into()),
        "float" | "css-float" => match s.float {
            Float::None => "none",
            Float::Left => "left",
            Float::Right => "right",
        }
        .into(),
        "clear" => match s.clear {
            Clear::None => "none",
            Clear::Left => "left",
            Clear::Right => "right",
            Clear::Both => "both",
        }
        .into(),
        "overflow" | "overflow-x" | "overflow-y" => overflow(s.overflow).into(),
        "width" => opt_len(s.width),
        "height" => opt_len(s.height),
        "min-width" => s.min_width.map(box_len).unwrap_or_else(|| "0px".into()),
        "min-height" => s.min_height.map(box_len).unwrap_or_else(|| "0px".into()),
        "max-width" => s.max_width.map(box_len).unwrap_or_else(|| "none".into()),
        "max-height" => s.max_height.map(box_len).unwrap_or_else(|| "none".into()),
        "top" => opt_len(s.inset.top),
        "right" => opt_len(s.inset.right),
        "bottom" => opt_len(s.inset.bottom),
        "left" => opt_len(s.inset.left),
        "margin-top" => if s.margin_auto_v.0 { "auto".into() } else { box_len(s.margin.top) },
        "margin-bottom" => if s.margin_auto_v.1 { "auto".into() } else { box_len(s.margin.bottom) },
        "margin-left" => if s.margin_auto.0 { "auto".into() } else { box_len(s.margin.left) },
        "margin-right" => if s.margin_auto.1 { "auto".into() } else { box_len(s.margin.right) },
        "padding-top" => box_len(s.padding.top),
        "padding-right" => box_len(s.padding.right),
        "padding-bottom" => box_len(s.padding.bottom),
        "padding-left" => box_len(s.padding.left),
        p if p.starts_with("border-") && p.ends_with("-width") => px(s.border_width[side(&p[7..p.len() - 6])]),
        p if p.starts_with("border-") && p.ends_with("-style") => border_style(s.border_style[side(&p[7..p.len() - 6])]).into(),
        p if p.starts_with("border-") && p.ends_with("-color") => {
            let i = side(&p[7..p.len() - 6]);
            s.border_color[i].map(color).unwrap_or_else(|| color(s.color))
        }
        "box-sizing" => if s.box_sizing == BoxSizing::BorderBox { "border-box" } else { "content-box" }.into(),
        "font-size" => px(s.font_size),
        "font-weight" => s.font_weight.0.to_string(),
        "font-style" => match s.font_style {
            FontStyle::Normal => "normal",
            FontStyle::Italic => "italic",
            FontStyle::Oblique => "oblique",
        }
        .into(),
        "font-family" => s
            .font_family
            .0
            .iter()
            .map(|f| match f {
                FontFamilyItem::Name(n) => format!("\"{n}\""),
                FontFamilyItem::SansSerif => "sans-serif".into(),
                FontFamilyItem::Serif => "serif".into(),
                FontFamilyItem::Monospace => "monospace".into(),
                FontFamilyItem::Cursive => "cursive".into(),
                FontFamilyItem::Fantasy => "fantasy".into(),
            })
            .collect::<Vec<_>>()
            .join(", "),
        "line-height" => match s.line_height {
            LineHeight::Normal => "normal".into(),
            l => px(l.resolve(s.font_size, s.root_font_size)),
        },
        "text-align" => match s.text_align {
            TextAlign::Left => "left",
            TextAlign::Right => "right",
            TextAlign::Center => "center",
            TextAlign::Justify => "justify",
        }
        .into(),
        "text-transform" => match s.text_transform {
            TextTransform::None => "none",
            TextTransform::Uppercase => "uppercase",
            TextTransform::Lowercase => "lowercase",
            TextTransform::Capitalize => "capitalize",
        }
        .into(),
        "white-space" => match s.white_space {
            WhiteSpace::Normal => "normal",
            WhiteSpace::Pre => "pre",
            WhiteSpace::Nowrap => "nowrap",
            WhiteSpace::PreWrap => "pre-wrap",
            WhiteSpace::PreLine => "pre-line",
        }
        .into(),
        "cursor" => kebab(&format!("{:?}", s.cursor)),
        "vertical-align" => kebab(&format!("{:?}", s.vertical_align)),
        _ => String::new(),
    }
}


type Parsed = Vec<(PieceKey, Vec<StyleRule>, bool, bool, bool)>;

fn parse_all(keys: &[PieceKey]) -> Parsed {
    keys.iter()
        .map(|k| {
            let (r, layer, has, sib) = parse_piece(k);
            (k.clone(), r, layer, has, sib)
        })
        .collect()
}

impl StyleCache {
    /// QOBS7: `(entries, approx bytes)`: the per-element style map by capacity, the parsed
    /// rules (selector heap not counted) and the rule index (counted by entry only).
    pub(crate) fn mem_estimate(&self) -> (usize, usize) {
        let map = self.map.capacity() * (std::mem::size_of::<((usize, u8), Entry)>() + 1);
        let rules = self.rules.as_ref().map_or(0, |r| r.capacity() * std::mem::size_of::<StyleRule>() + r.iter().map(StyleRule::heap_bytes).sum::<usize>());
        (self.map.len(), map + rules + self.pieces.capacity() * std::mem::size_of::<Piece>() + self.flags.capacity())
    }

    /// Brings the rule set up to date for DOM generation `wgen`: only the
    /// stylesheet pieces that changed are re-parsed (QY52).
    fn refresh(&mut self, root: &Handle, wgen: u64, page: bool) {
        let rp = Rc::as_ptr(root) as usize;
        let ver = CSS_VER.with(|v| v.get());
        let ready = if page { self.shared_gen.is_some() } else { self.rules.is_some() };
        if ready && self.shared == page && self.wgen == wgen && self.root_ptr == rp && self.css_ver == ver {
            return;
        }
        if self.root_ptr != rp {
            self.root_ptr = rp;
            self.map.clear();
            self.root_inherited = None;
        }
        self.wgen = wgen;
        self.css_ver = ver;
        if self.shared != page {
            // Switching between the page (shared rules) and another document's own parse.
            self.shared = page;
            self.shared_gen = None;
            self.rules = None;
            self.pieces.clear();
            self.index = None;
            self.map.clear();
            self.root_inherited = None;
        }
        if page {
            // QMEM3e: the layout build's parsed rules (each piece is parsed once for both).
            let _ = crate::rules_cache::cached_rules_for(root, crate::rules_cache::Extra::Ver(ver));
            self.adopt_shared();
            return;
        }
        let mut keys: Vec<PieceKey> = Vec::new();
        collect_style_pieces(root, &mut keys);
        let mut flags = String::new();
        if !js_enabled() {
            flags.push_str("nojs;");
        }
        let same = self.pieces.len() == keys.len() && self.pieces.iter().zip(&keys).all(|(p, k)| p.key == *k);
        if self.rules.is_some() && self.flags == flags && same {
            return;
        }
        if self.rules.is_none() || self.flags != flags || !self.splice(&keys) {
            self.rebuild(keys, flags);
        }
        self.map.clear();
        self.root_inherited = None;
        self.has_rules = self.ua_flags.0 || self.pieces.iter().any(|p| p.has);
        self.sib_rules = self.ua_flags.1 || self.pieces.iter().any(|p| p.sib);
    }

    /// QMEM3e: the layout cache's current rule list. The layout build (a forced layout, the App) can
    /// replace it between two reads without a DOM write this cache saw, so the generation is checked
    /// on every use: a different one drops the index and every cached style.
    fn adopt_shared(&mut self) -> Option<Rc<Vec<StyleRule>>> {
        let (rules, generation, has, sib) = crate::rules_cache::current()?;
        if self.shared_gen != Some(generation) {
            self.shared_gen = Some(generation);
            self.index = Some(RuleIndex::build(&rules));
            self.map.clear();
            self.root_inherited = None;
            self.has_rules = has;
            self.sib_rules = sib;
        }
        Some(rules)
    }

    /// Replaces only the changed middle of the rule list; false when it cannot be done safely.
    fn splice(&mut self, keys: &[PieceKey]) -> bool {
        if self.concat {
            return false;
        }
        let (old, new) = (self.pieces.len(), keys.len());
        let mut p = 0;
        while p < old && p < new && self.pieces[p].key == keys[p] {
            p += 1;
        }
        let mut sfx = 0;
        while sfx < old - p && sfx < new - p && self.pieces[old - 1 - sfx].key == keys[new - 1 - sfx] {
            sfx += 1;
        }
        if self.pieces[p..old - sfx].iter().any(|x| x.layer) {
            return false;
        }
        let parsed = parse_all(&keys[p..new - sfx]);
        if parsed.iter().any(|x| x.2) {
            return false;
        }
        let Some(v) = self.rules.as_mut().and_then(Rc::get_mut) else { return false };
        let start = self.ua_count + self.pieces[..p].iter().map(|x| x.count).sum::<usize>();
        let rem = self.pieces[p..old - sfx].iter().map(|x| x.count).sum::<usize>();
        let mut pieces = Vec::new();
        let mut rules = Vec::new();
        for (k, r, layer, has, sib) in parsed {
            pieces.push(Piece { key: k, count: r.len(), layer, has, sib });
            rules.extend(r);
        }
        v.splice(start..start + rem, rules);
        let tail = self.pieces.split_off(old - sfx);
        self.pieces.truncate(p);
        self.pieces.extend(pieces);
        self.pieces.extend(tail);
        self.index = Some(RuleIndex::build(v));
        true
    }

    fn rebuild(&mut self, keys: Vec<PieceKey>, flags: String) {
        let mut rules = parse_ua_stylesheet(DEFAULT_CSS);
        if js_enabled() {
            rules.extend(parse_ua_stylesheet("noscript { display: none; }"));
        }
        rules.extend(parse_ua_stylesheet("video { display: inline-block; object-fit: contain; }"));
        self.ua_count = rules.len();
        let (mut ua_has, mut ua_sib) = (false, false);
        for r in &rules {
            r.selectors.slice().iter().for_each(|s| sel_flags(s, &mut ua_has, &mut ua_sib));
        }
        self.ua_flags = (ua_has, ua_sib);
        let parsed = parse_all(&keys);
        self.concat = parsed.iter().any(|x| x.2);
        self.pieces.clear();
        if self.concat {
            // Layer ranks are numbered across sheets: parse the joined text as one.
            let mut src = String::new();
            for (k, _, _, has, sib) in &parsed {
                src.push_str(k);
                src.push('\n');
                self.pieces.push(Piece { key: k.clone(), count: 0, layer: true, has: *has, sib: *sib });
            }
            let _ = parse_stylesheet_into(&src, &mut rules);
        } else {
            for (k, r, layer, has, sib) in parsed {
                self.pieces.push(Piece { key: k, count: r.len(), layer, has, sib });
                rules.extend(r);
            }
        }
        self.flags = flags;
        self.index = Some(RuleIndex::build(&rules));
        self.rules = Some(Rc::new(rules));
    }

    /// Style of `h` (optionally its `::before`/`::after`), or `None` if it
    /// is not an element connected to `root` (CSSOM: all-empty values).
    pub(crate) fn style_of(&mut self, root: &Handle, wgen: u64, h: &Handle, pseudo: Option<&str>, page: bool) -> Option<ComputedStyle> {
        self.style_of_chain(root, wgen, h, pseudo, page, None)
    }

    /// QSTK1b: is `display:none` on `h` or any element ancestor? One cascade walk down the chain
    /// (what `style_of` does once for the target), instead of one `style_of` per ancestor (O(depth^2)).
    pub(crate) fn display_none_in_chain(&mut self, root: &Handle, wgen: u64, h: &Handle, page: bool) -> bool {
        let mut none = false;
        self.style_of_chain(root, wgen, h, None, page, Some(&mut |_, st| none |= display(st) == "none"));
        none
    }

    /// QSTK1b: the `offsetParent` search in one cascade walk: the nearest element ancestor of `h`
    /// that is `body`, a table/td/th, or positioned. `None` when the chain cannot be resolved.
    pub(crate) fn offset_parent_in_chain(&mut self, root: &Handle, wgen: u64, h: &Handle, page: bool) -> Option<Handle> {
        let mut seen: Vec<(Handle, bool)> = Vec::new();
        self.style_of_chain(root, wgen, h, None, page, Some(&mut |n, st| seen.push((n.clone(), serialize(st, "position") != "static"))))?;
        seen.pop(); // the element itself
        while let Some((n, positioned)) = seen.pop() {
            let t = element_tag(&n);
            if positioned || matches!(t, "body" | "td" | "th" | "table") {
                return Some(n);
            }
        }
        None
    }

    fn style_of_chain(
        &mut self,
        root: &Handle,
        wgen: u64,
        h: &Handle,
        pseudo: Option<&str>,
        page: bool,
        mut visit: Option<&mut ChainVisit<'_>>,
    ) -> Option<ComputedStyle> {
        self.refresh(root, wgen, page);
        let rules = if self.shared { self.adopt_shared()? } else { self.rules.clone()? };
        // Ancestor chain, outermost first; must end at the document.
        let mut chain = Vec::new();
        let mut cur = h.clone();
        loop {
            if Rc::ptr_eq(&cur, root) {
                break;
            }
            if matches!(cur.data, NodeData::Element { .. }) {
                chain.push(cur.clone());
            }
            cur = parent_handle(&cur)?;
        }
        if !matches!(h.data, NodeData::Element { .. }) {
            return None;
        }
        chain.reverse();
        #[cfg(test)]
        CHAIN_STEPS.with(|c| c.set(c.get() + chain.len() as u64));
        let vw = current_viewport().0;
        if self.map.len() > 50_000 {
            self.map.clear();
        }
        // QY52: a cached style is reused while the fingerprint of everything its
        // cascade can read is unchanged, so a write elsewhere does not invalidate it.
        let html = find_html_element(root);
        let root_key = mix(html.as_ref().map_or(0, |h| ctx_hash(h, false).0), vw.to_bits() as u64);
        let mut acc = mix(root_key, interaction_hash());
        // Make the cascade's selector index apply to our rule slice, then
        // restore whatever the layout build had registered.
        let prev = RULE_INDEX.with(|ri| ri.borrow_mut().take());
        if let Some(idx) = self.index.take() {
            RULE_INDEX.with(|ri| *ri.borrow_mut() = Some((rules.as_ptr() as usize, rules.len(), idx)));
        }
        let mut inh = match self.root_inherited {
            Some((k, r)) if k == root_key => r,
            _ => {
                let rfs = html
                    .as_ref()
                    .map(|html| {
                        CASCADES.with(|c| c.set(c.get() + 1));
                        resolve_style(html, &rules, ROOT_STYLE, vw).font_size
                    })
                    .unwrap_or(ROOT_STYLE.font_size);
                let r = InheritedStyle { root_font_size: rfs, ..ROOT_STYLE };
                self.root_inherited = Some((root_key, r));
                r
            }
        };
        let mut out = None;
        let last = chain.len().saturating_sub(1);
        for (i, n) in chain.iter().enumerate() {
            let (cx, formish) = ctx_hash(n, self.sib_rules);
            acc = mix(acc, cx);
            if self.has_rules || formish {
                acc = mix(acc, wgen);
            }
            let fp = acc;
            let key = (Rc::as_ptr(n) as usize, 0u8);
            let st = match self.map.get(&key) {
                Some(e) if e.fp == fp && e.wgen.is_none_or(|w| w == wgen) => e.style,
                _ => {
                    CASCADES.with(|c| c.set(c.get() + 1));
                    let s = resolve_style_for(n, &rules, inh, vw, None);
                    let w = s.animations.is_some().then_some(wgen);
                    self.map.insert(key, Entry { fp, wgen: w, style: s });
                    s
                }
            };
            if let Some(f) = visit.as_deref_mut() {
                f(n, &st);
            }
            if i == last {
                out = Some(match pseudo {
                    Some(p) => {
                        let pk = (key.0, if p == "before" { 1u8 } else { 2 });
                        match self.map.get(&pk) {
                            Some(e) if e.fp == fp && e.wgen.is_none_or(|w| w == wgen) => e.style,
                            _ => {
                                CASCADES.with(|c| c.set(c.get() + 1));
                                let s = resolve_style_for(n, &rules, st.inherited(), vw, Some(p));
                                let w = s.animations.is_some().then_some(wgen);
                                self.map.insert(pk, Entry { fp, wgen: w, style: s });
                                s
                            }
                        }
                    }
                    None => st,
                });
            }
            inh = st.inherited();
        }
        self.index = RULE_INDEX.with(|ri| ri.borrow_mut().take()).map(|(_, _, i)| i);
        RULE_INDEX.with(|ri| *ri.borrow_mut() = prev);
        out
    }
}
