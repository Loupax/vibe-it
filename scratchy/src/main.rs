use std::env;
use std::fs;
use std::io::{self, Read};
use std::num::NonZeroU32;
use std::rc::{Rc, Weak};
use std::thread;

use cosmic_text::{Attrs, Buffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser as CssParser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};
use cssparser_color::Color as CssColor;
use html5ever::driver::{parse_document, ParseOpts, Parser};
use html5ever::tendril::stream::Utf8LossyDecoder;
use html5ever::tendril::{ByteTendril, TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom, WeakHandle};
use precomputed_hash::PrecomputedHash;
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::{
    MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode,
    SelectorCaches,
};
use selectors::matching::{matches_selector_list, ElementSelectorFlags};
use selectors::parser::{
    NonTSPseudoClass, ParseRelative, Parser as SelectorsParser, PseudoElement, SelectorImpl,
    SelectorList, SelectorParseErrorKind,
};
use selectors::{Element as SelectorsElement, OpaqueElement};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

// ---------- CLI input ----------

enum Input {
    File(String),
    Stdin,
}

fn parse_args() -> Input {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.len() {
        0 => Input::Stdin,
        1 => Input::File(args[0].clone()),
        _ => {
            eprintln!("usage: scratchy [file]");
            std::process::exit(1);
        }
    }
}

fn new_parser() -> Utf8LossyDecoder<Parser<RcDom>> {
    parse_document(RcDom::default(), ParseOpts::default()).from_utf8()
}

// ---------- minimal selectors glue ----------

#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
struct Atom(String);

impl From<&str> for Atom {
    fn from(s: &str) -> Self {
        Atom(s.to_string())
    }
}

impl std::borrow::Borrow<str> for Atom {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Atom {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PrecomputedHash for Atom {
    fn precomputed_hash(&self) -> u32 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.0.hash(&mut h);
        h.finish() as u32
    }
}

impl cssparser::ToCss for Atom {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        dest.write_str(&self.0)
    }
}

/// Uninhabited: we support neither interactive pseudo-classes nor pseudo-elements.
#[derive(Clone, Debug, Eq, PartialEq)]
enum NoPseudoClass {}

impl cssparser::ToCss for NoPseudoClass {
    fn to_css<W: std::fmt::Write>(&self, _dest: &mut W) -> std::fmt::Result {
        match *self {}
    }
}

impl NonTSPseudoClass for NoPseudoClass {
    fn is_active_or_hover(&self) -> bool {
        match *self {}
    }
    fn is_user_action_state(&self) -> bool {
        match *self {}
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NoPseudoElement {}

impl cssparser::ToCss for NoPseudoElement {
    fn to_css<W: std::fmt::Write>(&self, _dest: &mut W) -> std::fmt::Result {
        match *self {}
    }
}

impl PseudoElement for NoPseudoElement {}

#[derive(Clone, Debug, PartialEq)]
struct OurSelectorImpl;

impl SelectorImpl for OurSelectorImpl {
    type ExtraMatchingData<'a> = ();
    type AttrValue = Atom;
    type Identifier = Atom;
    type LocalName = Atom;
    type NamespaceUrl = Atom;
    type NamespacePrefix = Atom;
    type BorrowedLocalName = str;
    type BorrowedNamespaceUrl = str;
    type NonTSPseudoClass = NoPseudoClass;
    type PseudoElement = NoPseudoElement;
}

#[derive(Debug)]
struct CssError;

impl From<SelectorParseErrorKind> for CssError {
    fn from(_: SelectorParseErrorKind) -> Self {
        CssError
    }
}

struct OurSelectorParser;

impl<'i> SelectorsParser<'i> for OurSelectorParser {
    type Impl = OurSelectorImpl;
    type Error = CssError;
}

/// A DOM node wrapped so we can implement the foreign `selectors::Element`
/// trait on it (Rust's orphan rules forbid implementing it directly on
/// `Handle`, which is itself a foreign type alias for `Rc<Node>`).
#[derive(Clone, Debug)]
struct El(Handle);

fn parent_handle(node: &Handle) -> Option<Handle> {
    let weak: Option<WeakHandle> = node.parent.take();
    let result = weak.as_ref().and_then(Weak::upgrade);
    node.parent.set(weak);
    result
}

fn element_attr<'a>(handle: &'a Handle, name: &str) -> Option<std::cell::Ref<'a, str>> {
    if let NodeData::Element { attrs, .. } = &handle.data {
        let attrs_ref = attrs.borrow();
        if attrs_ref.iter().any(|a| a.name.local.as_ref() == name) {
            return Some(std::cell::Ref::map(attrs_ref, |attrs| {
                attrs
                    .iter()
                    .find(|a| a.name.local.as_ref() == name)
                    .map(|a| a.value.as_ref())
                    .unwrap()
            }));
        }
    }
    None
}

impl SelectorsElement for El {
    type Impl = OurSelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(&*self.0)
    }

    fn parent_element(&self) -> Option<Self> {
        parent_handle(&self.0).filter(|p| matches!(p.data, NodeData::Element { .. })).map(El)
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        let parent = parent_handle(&self.0)?;
        let children = parent.children.borrow();
        let pos = children.iter().position(|c| Rc::ptr_eq(c, &self.0))?;
        children[..pos]
            .iter()
            .rev()
            .find(|c| matches!(c.data, NodeData::Element { .. }))
            .cloned()
            .map(El)
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let parent = parent_handle(&self.0)?;
        let children = parent.children.borrow();
        let pos = children.iter().position(|c| Rc::ptr_eq(c, &self.0))?;
        children[pos + 1..]
            .iter()
            .find(|c| matches!(c.data, NodeData::Element { .. }))
            .cloned()
            .map(El)
    }

    fn first_element_child(&self) -> Option<Self> {
        self.0
            .children
            .borrow()
            .iter()
            .find(|c| matches!(c.data, NodeData::Element { .. }))
            .cloned()
            .map(El)
    }

    fn is_html_element_in_html_document(&self) -> bool {
        true
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        match &self.0.data {
            NodeData::Element { name, .. } => name.local.as_ref() == local_name,
            _ => false,
        }
    }

    fn has_namespace(&self, ns: &str) -> bool {
        match &self.0.data {
            NodeData::Element { name, .. } => name.ns.as_ref() == ns,
            _ => false,
        }
    }

    fn is_same_type(&self, other: &Self) -> bool {
        match (&self.0.data, &other.0.data) {
            (
                NodeData::Element { name: a, .. },
                NodeData::Element { name: b, .. },
            ) => a.local == b.local && a.ns == b.ns,
            _ => false,
        }
    }

    fn attr_matches(
        &self,
        _ns: &NamespaceConstraint<&Atom>,
        local_name: &Atom,
        operation: &AttrSelectorOperation<&Atom>,
    ) -> bool {
        match element_attr(&self.0, &local_name.0) {
            Some(value) => operation.eval_str(&value),
            None => false,
        }
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &NoPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match *pc {}
    }

    fn match_pseudo_element(
        &self,
        pe: &NoPseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match *pe {}
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        false
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &Atom, case_sensitivity: CaseSensitivity) -> bool {
        match element_attr(&self.0, "id") {
            Some(value) => case_sensitivity.eq(value.as_bytes(), id.0.as_bytes()),
            None => false,
        }
    }

    fn has_class(&self, name: &Atom, case_sensitivity: CaseSensitivity) -> bool {
        match element_attr(&self.0, "class") {
            Some(value) => value
                .split_whitespace()
                .any(|c| case_sensitivity.eq(c.as_bytes(), name.0.as_bytes())),
            None => false,
        }
    }

    fn has_custom_state(&self, _name: &Atom) -> bool {
        false
    }

    fn imported_part(&self, _name: &Atom) -> Option<Atom> {
        None
    }

    fn is_part(&self, _name: &Atom) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        !self.0.children.borrow().iter().any(|c| match &c.data {
            NodeData::Element { .. } => true,
            NodeData::Text { contents } => !contents.borrow().trim().is_empty(),
            _ => false,
        })
    }

    fn is_root(&self) -> bool {
        self.parent_element().is_none()
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}

// ---------- CSS parsing ----------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Display {
    Block,
    Inline,
    None,
}

#[derive(Clone, Copy, Default)]
struct Declarations {
    color: Option<CssColor>,
    background_color: Option<CssColor>,
    display: Option<Display>,
    margin: Option<f32>,
    padding: Option<f32>,
    border_width: Option<f32>,
    border_color: Option<CssColor>,
}

struct StyleRule {
    selectors: SelectorList<OurSelectorImpl>,
    decls: Declarations,
}

fn parse_px_length(input: &mut CssParser) -> Result<f32, ()> {
    match input.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => Ok(*value),
        Token::Number { value, .. } if *value == 0.0 => Ok(0.0),
        _ => Err(()),
    }
}

struct DeclParser<'a> {
    decls: &'a mut Declarations,
}

impl<'i> DeclarationParser<'i> for DeclParser<'_> {
    type Declaration = ();
    type Error = CssError;

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut CssParser<'i>,
        _declaration_start: &ParserState,
    ) -> Result<Self::Declaration, ParseError<Self::Error>> {
        if name.eq_ignore_ascii_case("color") {
            if let Ok(c) = CssColor::parse(input) {
                self.decls.color = Some(c);
            }
        } else if name.eq_ignore_ascii_case("background-color") {
            if let Ok(c) = CssColor::parse(input) {
                self.decls.background_color = Some(c);
            }
        } else if name.eq_ignore_ascii_case("border-color") {
            if let Ok(c) = CssColor::parse(input) {
                self.decls.border_color = Some(c);
            }
        } else if name.eq_ignore_ascii_case("display") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.display = if ident.eq_ignore_ascii_case("block") {
                    Some(Display::Block)
                } else if ident.eq_ignore_ascii_case("inline") {
                    Some(Display::Inline)
                } else if ident.eq_ignore_ascii_case("none") {
                    Some(Display::None)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("margin") {
            if let Ok(v) = parse_px_length(input) {
                self.decls.margin = Some(v);
            }
        } else if name.eq_ignore_ascii_case("padding") {
            if let Ok(v) = parse_px_length(input) {
                self.decls.padding = Some(v);
            }
        } else if name.eq_ignore_ascii_case("border-width") {
            if let Ok(v) = parse_px_length(input) {
                self.decls.border_width = Some(v);
            }
        }
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for DeclParser<'_> {
    type Prelude = ();
    type AtRule = ();
    type Error = CssError;
}

impl<'i> QualifiedRuleParser<'i> for DeclParser<'_> {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = CssError;
}

impl<'i> RuleBodyItemParser<'i, (), CssError> for DeclParser<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

fn parse_declarations(source: &str) -> Declarations {
    let mut decls = Declarations::default();
    {
        let mut input = CssParser::new(source);
        let mut decl_parser = DeclParser { decls: &mut decls };
        let mut iter = RuleBodyParser::new(&mut input, &mut decl_parser);
        while iter.next().is_some() {}
    }
    decls
}

struct TopLevelParser;

impl<'i> AtRuleParser<'i> for TopLevelParser {
    type Prelude = ();
    type AtRule = StyleRule;
    type Error = CssError;
    // Default implementation rejects every at-rule, which is exactly
    // "skip @media/@import/@font-face/etc." for this stage.
}

impl<'i> QualifiedRuleParser<'i> for TopLevelParser {
    type Prelude = SelectorList<OurSelectorImpl>;
    type QualifiedRule = StyleRule;
    type Error = CssError;

    fn parse_prelude(
        &mut self,
        input: &mut CssParser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        SelectorList::parse(&OurSelectorParser, input, ParseRelative::No)
    }

    fn parse_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut CssParser<'i>,
    ) -> Result<Self::QualifiedRule, ParseError<Self::Error>> {
        let mut decls = Declarations::default();
        {
            let mut decl_parser = DeclParser { decls: &mut decls };
            let mut iter = RuleBodyParser::new(input, &mut decl_parser);
            while iter.next().is_some() {}
        }
        Ok(StyleRule {
            selectors: prelude,
            decls,
        })
    }
}

fn parse_stylesheet(css: &str) -> Vec<StyleRule> {
    let mut input = CssParser::new(css);
    let mut top = TopLevelParser;
    let mut rules = Vec::new();
    let mut stylesheet = StyleSheetParser::new(&mut input, &mut top);
    while let Some(result) = stylesheet.next() {
        if let Ok(rule) = result {
            rules.push(rule);
        }
    }
    rules
}

fn to_text_color(color: &CssColor) -> Option<TextColor> {
    match color {
        CssColor::Rgba(rgba) => Some(TextColor::rgba(
            rgba.red,
            rgba.green,
            rgba.blue,
            (rgba.alpha * 255.0).round() as u8,
        )),
        _ => None,
    }
}

// ---------- cascade ----------

#[derive(Clone, Copy)]
struct ComputedStyle {
    color: TextColor,
    background_color: Option<TextColor>,
    display: Display,
    margin: f32,
    padding: f32,
    border_width: f32,
    border_color: Option<TextColor>,
}

const BLOCK_TAGS: &[&str] = &[
    "html",
    "body",
    "div",
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "header",
    "footer",
    "section",
    "article",
    "nav",
    "aside",
    "figure",
    "figcaption",
    "table",
    "form",
    "hr",
];

fn default_display(tag: &str) -> Display {
    if BLOCK_TAGS.contains(&tag) {
        Display::Block
    } else {
        Display::Inline
    }
}

fn cascade_property<T: Clone>(
    matches: &[(u32, &Declarations)],
    inline: Option<T>,
    get: impl Fn(&Declarations) -> Option<T>,
) -> Option<T> {
    if inline.is_some() {
        return inline;
    }
    let mut best: Option<(u32, T)> = None;
    for &(specificity, decls) in matches {
        if let Some(v) = get(decls) {
            if best.as_ref().map_or(true, |(s, _)| specificity >= *s) {
                best = Some((specificity, v));
            }
        }
    }
    best.map(|(_, v)| v)
}

fn resolve_style(handle: &Handle, rules: &[StyleRule], inherited_color: TextColor) -> ComputedStyle {
    let tag = match &handle.data {
        NodeData::Element { name, .. } => name.local.as_ref(),
        _ => "",
    };

    let element = El(handle.clone());
    let mut caches = SelectorCaches::default();
    let mut matches: Vec<(u32, &Declarations)> = Vec::new();
    for rule in rules {
        let mut ctx = MatchingContext::new(
            MatchingMode::Normal,
            None,
            &mut caches,
            QuirksMode::NoQuirks,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::No,
        );
        if matches_selector_list(&rule.selectors, &element, &mut ctx) {
            let specificity = rule
                .selectors
                .slice()
                .iter()
                .map(|s| s.specificity())
                .max()
                .unwrap_or(0);
            matches.push((specificity, &rule.decls));
        }
    }

    let inline_decls = element_attr(handle, "style").map(|s| parse_declarations(&s));

    let color = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.color), |d| d.color)
        .and_then(|c| to_text_color(&c))
        .unwrap_or(inherited_color);
    let background_color = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.background_color),
        |d| d.background_color,
    )
    .and_then(|c| to_text_color(&c));
    let display = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.display), |d| d.display)
        .unwrap_or_else(|| default_display(tag));
    let margin = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin), |d| d.margin)
        .unwrap_or(0.0);
    let padding = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.padding), |d| d.padding)
        .unwrap_or(0.0);
    let border_width = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_width),
        |d| d.border_width,
    )
    .unwrap_or(0.0);
    let border_color = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_color),
        |d| d.border_color,
    )
    .and_then(|c| to_text_color(&c));

    ComputedStyle {
        color,
        background_color,
        display,
        margin,
        padding,
        border_width,
        border_color,
    }
}

fn anonymous_style(color: TextColor) -> ComputedStyle {
    ComputedStyle {
        color,
        background_color: None,
        display: Display::Block,
        margin: 0.0,
        padding: 0.0,
        border_width: 0.0,
        border_color: None,
    }
}

fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------- layout tree ----------

enum BoxContent {
    Blocks(Vec<LayoutBox>),
    Inline(Vec<(String, TextColor)>),
}

struct LayoutBox {
    style: ComputedStyle,
    content: BoxContent,
}

/// Flattens an inline-level subtree (text + nested inline elements) into
/// colored runs. Any block-level descendant is treated as inline too —
/// "block inside inline" is out of scope for normal-flow-only layout.
fn flatten_inline(
    handle: &Handle,
    rules: &[StyleRule],
    inherited_color: TextColor,
    out: &mut Vec<(String, TextColor)>,
) {
    match &handle.data {
        NodeData::Text { contents } => {
            let mut text = normalize_whitespace(&contents.borrow());
            text.push(' ');
            out.push((text, inherited_color));
        }
        NodeData::Element { name, .. } => {
            let tag = name.local.as_ref();
            if tag == "script" || tag == "style" {
                return;
            }
            let style = resolve_style(handle, rules, inherited_color);
            if style.display == Display::None {
                return;
            }
            for child in handle.children.borrow().iter() {
                flatten_inline(child, rules, style.color, out);
            }
        }
        _ => {
            for child in handle.children.borrow().iter() {
                flatten_inline(child, rules, inherited_color, out);
            }
        }
    }
}

fn build_box_for_element(handle: &Handle, rules: &[StyleRule], inherited_color: TextColor) -> LayoutBox {
    let style = resolve_style(handle, rules, inherited_color);
    let children: Vec<Handle> = handle.children.borrow().iter().cloned().collect();
    let has_block_child = children.iter().any(|c| match &c.data {
        NodeData::Element { name, .. } => {
            let tag = name.local.as_ref();
            if tag == "script" || tag == "style" {
                false
            } else {
                resolve_style(c, rules, style.color).display == Display::Block
            }
        }
        _ => false,
    });

    let content = if has_block_child {
        BoxContent::Blocks(build_children(handle, rules, style.color))
    } else {
        let mut runs = Vec::new();
        for child in &children {
            flatten_inline(child, rules, style.color, &mut runs);
        }
        BoxContent::Inline(runs)
    };

    LayoutBox { style, content }
}

/// Walks the direct children of `handle`, starting a new box for each
/// block-level child and grouping consecutive inline-level children
/// (including text) into one anonymous inline-run box.
fn build_children(handle: &Handle, rules: &[StyleRule], inherited_color: TextColor) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    let mut pending: Vec<(String, TextColor)> = Vec::new();

    for child in handle.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => {
                let mut text = normalize_whitespace(&contents.borrow());
                text.push(' ');
                pending.push((text, inherited_color));
            }
            NodeData::Element { name, .. } => {
                let tag = name.local.as_ref();
                if tag == "script" || tag == "style" {
                    continue;
                }
                let style = resolve_style(child, rules, inherited_color);
                if style.display == Display::None {
                    continue;
                }
                if style.display == Display::Block {
                    if !pending.is_empty() {
                        result.push(LayoutBox {
                            style: anonymous_style(inherited_color),
                            content: BoxContent::Inline(std::mem::take(&mut pending)),
                        });
                    }
                    result.push(build_box_for_element(child, rules, inherited_color));
                } else {
                    flatten_inline(child, rules, style.color, &mut pending);
                }
            }
            _ => {}
        }
    }
    if !pending.is_empty() {
        result.push(LayoutBox {
            style: anonymous_style(inherited_color),
            content: BoxContent::Inline(pending),
        });
    }
    result
}

fn build_layout_tree(document: &Handle) -> LayoutBox {
    let mut css_source = String::new();
    collect_style_css(document, &mut css_source);
    let rules = parse_stylesheet(&css_source);
    let root_color = TextColor::rgb(0, 0, 0);
    LayoutBox {
        style: anonymous_style(root_color),
        content: BoxContent::Blocks(build_children(document, &rules, root_color)),
    }
}

fn collect_text_only(handle: &Handle, out: &mut String) {
    if let NodeData::Text { contents } = &handle.data {
        out.push_str(&contents.borrow());
    }
    for child in handle.children.borrow().iter() {
        collect_text_only(child, out);
    }
}

fn collect_style_css(handle: &Handle, out: &mut String) {
    if let NodeData::Element { name, .. } = &handle.data {
        if name.local.as_ref() == "style" {
            collect_text_only(handle, out);
            out.push('\n');
            return;
        }
    }
    for child in handle.children.borrow().iter() {
        collect_style_css(child, out);
    }
}

// ---------- measure + paint ----------

/// A laid-out box: resolved border-box rect (absolute document coordinates,
/// not yet scroll-adjusted) plus either its laid-out children or its
/// already-shaped text buffer.
struct Laid {
    style: ComputedStyle,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    content: LaidContent,
}

enum LaidContent {
    Blocks(Vec<Laid>),
    Text(Buffer),
}

fn shape_spans(font_system: &mut FontSystem, spans: &[(String, TextColor)], width: f32) -> (Buffer, f32) {
    let metrics = Metrics::new(16.0, 20.0);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(Some(width.max(0.0)), None);
    let rich: Vec<(&str, Attrs)> = spans
        .iter()
        .map(|(text, color)| {
            let mut attrs = Attrs::new();
            attrs.color_opt = Some(*color);
            (text.as_str(), attrs)
        })
        .collect();
    buffer.set_rich_text(rich, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    let mut height = 0.0f32;
    for run in buffer.layout_runs() {
        height = height.max(run.line_top + run.line_height);
    }
    (buffer, height)
}

fn measure(b: &LayoutBox, font_system: &mut FontSystem, x: f32, y: f32, width: f32) -> Laid {
    let s = b.style;
    let border_x = x + s.margin;
    let border_y = y + s.margin;
    let border_w = (width - 2.0 * s.margin).max(0.0);
    let content_x = border_x + s.border_width + s.padding;
    let content_y = border_y + s.border_width + s.padding;
    let content_w = (border_w - 2.0 * (s.border_width + s.padding)).max(0.0);

    match &b.content {
        BoxContent::Inline(spans) => {
            let (buffer, content_h) = if spans.is_empty() {
                (Buffer::new_empty(Metrics::new(16.0, 20.0)), 0.0)
            } else {
                shape_spans(font_system, spans, content_w)
            };
            let border_h = 2.0 * (s.border_width + s.padding) + content_h;
            Laid {
                style: s,
                x: border_x,
                y: border_y,
                w: border_w,
                h: border_h,
                content: LaidContent::Text(buffer),
            }
        }
        BoxContent::Blocks(children) => {
            let mut cursor_y = content_y;
            let mut laid_children = Vec::with_capacity(children.len());
            for child in children {
                let lc = measure(child, font_system, content_x, cursor_y, content_w);
                cursor_y += 2.0 * lc.style.margin + lc.h;
                laid_children.push(lc);
            }
            let content_h = cursor_y - content_y;
            let border_h = 2.0 * (s.border_width + s.padding) + content_h;
            Laid {
                style: s,
                x: border_x,
                y: border_y,
                w: border_w,
                h: border_h,
                content: LaidContent::Blocks(laid_children),
            }
        }
    }
}

fn blend(existing: u32, color: TextColor) -> u32 {
    let a = color.a() as u32;
    if a == 255 {
        return (color.r() as u32) << 16 | (color.g() as u32) << 8 | (color.b() as u32);
    }
    let er = (existing >> 16) & 0xFF;
    let eg = (existing >> 8) & 0xFF;
    let eb = existing & 0xFF;
    let inv = 255 - a;
    let r = (color.r() as u32 * a + er * inv) / 255;
    let g = (color.g() as u32 * a + eg * inv) / 255;
    let b = (color.b() as u32 * a + eb * inv) / 255;
    (r << 16) | (g << 8) | b
}

fn fill_rect(pixels: &mut [u32], win_w: u32, win_h: u32, x: f32, y: f32, w: f32, h: f32, color: TextColor) {
    if color.a() == 0 {
        return;
    }
    let x0 = x.max(0.0) as i64;
    let y0 = y.max(0.0) as i64;
    let x1 = ((x + w).min(win_w as f32)) as i64;
    let y1 = ((y + h).min(win_h as f32)) as i64;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for py in y0..y1 {
        for px in x0..x1 {
            let idx = (py as u32 * win_w + px as u32) as usize;
            pixels[idx] = blend(pixels[idx], color);
        }
    }
}

fn paint_box_decoration(s: &ComputedStyle, pixels: &mut [u32], win_w: u32, win_h: u32, x: f32, y: f32, w: f32, h: f32) {
    if let Some(bg) = s.background_color {
        fill_rect(pixels, win_w, win_h, x, y, w, h, bg);
    }
    if s.border_width > 0.0 {
        if let Some(bc) = s.border_color {
            let bw = s.border_width;
            fill_rect(pixels, win_w, win_h, x, y, w, bw, bc);
            fill_rect(pixels, win_w, win_h, x, y + h - bw, w, bw, bc);
            fill_rect(pixels, win_w, win_h, x, y, bw, h, bc);
            fill_rect(pixels, win_w, win_h, x + w - bw, y, bw, h, bc);
        }
    }
}

fn paint_buffer(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    offset_x: f32,
    offset_y: f32,
) {
    let ox = offset_x.round() as i32;
    let oy = offset_y.round() as i32;
    buffer.draw(
        font_system,
        swash_cache,
        TextColor::rgb(0, 0, 0),
        |x, y, w, h, color| {
            if color.a() == 0 {
                return;
            }
            for dy in 0..h as i32 {
                for dx in 0..w as i32 {
                    let px = x + dx + ox;
                    let py = y + dy + oy;
                    if px < 0 || py < 0 || px as u32 >= win_w || py as u32 >= win_h {
                        continue;
                    }
                    let idx = (py as u32 * win_w + px as u32) as usize;
                    pixels[idx] = blend(pixels[idx], color);
                }
            }
        },
    );
}

fn paint(
    laid: &mut Laid,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    scroll_offset: f32,
) {
    let s = laid.style;
    let draw_y = laid.y - scroll_offset;
    paint_box_decoration(&s, pixels, win_w, win_h, laid.x, draw_y, laid.w, laid.h);

    match &mut laid.content {
        LaidContent::Blocks(children) => {
            for child in children {
                paint(child, font_system, swash_cache, pixels, win_w, win_h, scroll_offset);
            }
        }
        LaidContent::Text(buffer) => {
            let content_x = laid.x + s.border_width + s.padding;
            let content_y = draw_y + s.border_width + s.padding;
            paint_buffer(buffer, font_system, swash_cache, pixels, win_w, win_h, content_x, content_y);
        }
    }
}

// ---------- windowing ----------

enum AppEvent {
    Chunk(Vec<u8>),
    Eof,
}

struct App {
    document: Handle,
    decoder: Option<Utf8LossyDecoder<Parser<RcDom>>>,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl App {
    fn new_static(document: Handle) -> Self {
        Self {
            document,
            decoder: None,
            window: None,
            surface: None,
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    fn new_streaming() -> Self {
        let decoder = new_parser();
        let document = decoder.inner_sink.tokenizer.sink.sink.document.clone();
        Self {
            document,
            decoder: Some(decoder),
            window: None,
            surface: None,
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    fn redraw(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        surface
            .resize(NonZeroU32::new(width).unwrap(), NonZeroU32::new(height).unwrap())
            .unwrap();

        let tree = build_layout_tree(&self.document);
        let mut laid = measure(&tree, &mut self.font_system, 0.0, 0.0, width as f32);

        let mut pixels = surface.buffer_mut().unwrap();
        for pixel in pixels.iter_mut() {
            *pixel = 0x00FFFFFF;
        }

        paint(
            &mut laid,
            &mut self.font_system,
            &mut self.swash_cache,
            &mut pixels,
            width,
            height,
            0.0,
        );

        pixels.present().unwrap();
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window_attrs = Window::default_attributes()
            .with_title("scratchy")
            .with_inner_size(LogicalSize::new(1024.0, 768.0));
        let window = Rc::new(event_loop.create_window(window_attrs).unwrap());
        let context = Context::new(window.clone()).unwrap();
        let surface = Surface::new(&context, window.clone()).unwrap();

        self.window = Some(window);
        self.surface = Some(surface);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Chunk(bytes) => {
                if let Some(decoder) = &mut self.decoder {
                    decoder.process(ByteTendril::from_slice(&bytes));
                    self.document = decoder.inner_sink.tokenizer.sink.sink.document.clone();
                }
            }
            AppEvent::Eof => {
                if let Some(decoder) = self.decoder.take() {
                    let dom = decoder.finish();
                    self.document = dom.document;
                }
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let input = parse_args();

    let event_loop = EventLoop::<AppEvent>::with_user_event().build().unwrap();

    let mut app = match input {
        Input::File(path) => {
            let html = fs::read_to_string(&path).unwrap_or_else(|e| {
                eprintln!("scratchy: {e}");
                std::process::exit(1);
            });
            let dom = new_parser().read_from(&mut html.as_bytes()).unwrap();
            App::new_static(dom.document)
        }
        Input::Stdin => {
            let proxy = event_loop.create_proxy();
            thread::spawn(move || {
                let mut stdin = io::stdin();
                let mut buf = [0u8; 8192];
                loop {
                    match stdin.read(&mut buf) {
                        Ok(0) => {
                            let _ = proxy.send_event(AppEvent::Eof);
                            break;
                        }
                        Ok(n) => {
                            if proxy.send_event(AppEvent::Chunk(buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => {
                            let _ = proxy.send_event(AppEvent::Eof);
                            break;
                        }
                    }
                }
            });
            App::new_streaming()
        }
    };

    event_loop.run_app(&mut app).unwrap();
}
