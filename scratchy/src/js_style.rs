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
    static EXTRA_CSS: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The external stylesheets of the current page (same text the layout
/// build receives as `extra_css`).
pub(crate) fn set_js_extra_css(css: &str) {
    EXTRA_CSS.with(|c| {
        let mut c = c.borrow_mut();
        if *c != css {
            *c = css.to_string();
        }
    });
}

thread_local! {
    static ADOPTED_CSS: RefCell<String> = const { RefCell::new(String::new()) };
}

/// QSD4: the document's adopted stylesheets (cascade after the page CSS).
pub(crate) fn set_js_adopted_css(css: &str) {
    ADOPTED_CSS.with(|c| *c.borrow_mut() = css.to_string());
}

/// The page's external plus adopted CSS (what the layout build receives as
/// `extra_css`).
pub(crate) fn page_css() -> String {
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

#[derive(Default)]
pub(crate) struct StyleCache {
    wgen: u64,
    valid: bool,
    css_source: String,
    rules: Option<Rc<Vec<StyleRule>>>,
    index: Option<RuleIndex>,
    map: HashMap<(usize, u8), ComputedStyle>,
    root_inherited: Option<InheritedStyle>,
    /// QA18c: the document the cached rules/styles belong to.
    root_ptr: usize,
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
        return s.custom_props.and_then(|m| m.get(prop)).cloned().unwrap_or_default();
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

impl StyleCache {
    /// Brings the rule set up to date for DOM generation `wgen`.
    fn refresh(&mut self, root: &Handle, wgen: u64, page: bool) {
        let rp = Rc::as_ptr(root) as usize;
        if self.valid && self.wgen == wgen && self.root_ptr == rp {
            return;
        }
        self.root_ptr = rp;
        self.wgen = wgen;
        self.valid = true;
        self.map.clear();
        self.root_inherited = None;
        let mut src = String::new();
        collect_style_css(root, &mut src);
        src.push('\n');
        // The fetched stylesheets belong to the page, not to a second document.
        if page {
            EXTRA_CSS.with(|c| src.push_str(&c.borrow()));
            src.push('\n');
            ADOPTED_CSS.with(|c| src.push_str(&c.borrow()));
        }
        if !js_enabled() {
            src.push_str("\n/* nojs */");
        }
        if crate::media_video::boxes_enabled() {
            src.push_str("\n/* video-box */");
        }
        if self.rules.is_none() || self.css_source != src {
            let mut rules = parse_ua_stylesheet(DEFAULT_CSS);
            if js_enabled() {
                rules.extend(parse_ua_stylesheet("noscript { display: none; }"));
            }
            if crate::media_video::boxes_enabled() {
                rules.extend(parse_ua_stylesheet("video { display: inline-block; object-fit: contain; }"));
            }
            rules.extend(parse_stylesheet(&src));
            self.index = Some(RuleIndex::build(&rules));
            self.rules = Some(Rc::new(rules));
            self.css_source = src;
        }
    }

    /// Style of `h` (optionally its `::before`/`::after`), or `None` if it
    /// is not an element connected to `root` (CSSOM: all-empty values).
    pub(crate) fn style_of(&mut self, root: &Handle, wgen: u64, h: &Handle, pseudo: Option<&str>, page: bool) -> Option<ComputedStyle> {
        self.refresh(root, wgen, page);
        let rules = self.rules.clone()?;
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
        let vw = current_viewport().0;
        // Make the cascade's selector index apply to our rule slice, then
        // restore whatever the layout build had registered.
        let prev = RULE_INDEX.with(|ri| ri.borrow_mut().take());
        if let Some(idx) = self.index.take() {
            RULE_INDEX.with(|ri| *ri.borrow_mut() = Some((rules.as_ptr() as usize, rules.len(), idx)));
        }
        let mut inh = match self.root_inherited {
            Some(r) => r,
            None => {
                let rfs = find_html_element(root)
                    .map(|html| resolve_style(&html, &rules, ROOT_STYLE, vw).font_size)
                    .unwrap_or(ROOT_STYLE.font_size);
                let r = InheritedStyle { root_font_size: rfs, ..ROOT_STYLE };
                self.root_inherited = Some(r);
                r
            }
        };
        let mut out = None;
        let last = chain.len().saturating_sub(1);
        for (i, n) in chain.iter().enumerate() {
            let key = (Rc::as_ptr(n) as usize, 0u8);
            let st = match self.map.get(&key).copied() {
                Some(s) => s,
                None => {
                    let s = resolve_style_for(n, &rules, inh, vw, None);
                    self.map.insert(key, s);
                    s
                }
            };
            if i == last {
                out = Some(match pseudo {
                    Some(p) => {
                        let pk = (key.0, if p == "before" { 1u8 } else { 2 });
                        match self.map.get(&pk).copied() {
                            Some(s) => s,
                            None => {
                                let s = resolve_style_for(n, &rules, st.inherited(), vw, Some(p));
                                self.map.insert(pk, s);
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
