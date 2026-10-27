use std::env;
use std::fs;
use std::io::{self, Read};
use std::num::NonZeroU32;
use std::rc::{Rc, Weak};
use std::thread;

use cosmic_text::{
    Attrs, Buffer, Color as TextColor, FontSystem, Metrics, Shaping, Style as FontStyle,
    SwashCache, Weight as FontWeight,
};
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
use winit::event::{MouseScrollDelta, WindowEvent};
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

fn element_tag(handle: &Handle) -> &str {
    match &handle.data {
        NodeData::Element { name, .. } => name.local.as_ref(),
        _ => "",
    }
}

/// The marker text for a `<li>`: `"<n>. "` when its parent is an `<ol>`,
/// where `<n>` is this element's 1-based position among the parent's
/// direct `<li>` children (document order, non-`<li>` children skipped
/// when counting) — otherwise (`<ul>`, or malformed/missing-parent
/// markup) a plain bullet. Only ever looks at the immediate parent, so
/// a nested list numbers independently of any list it's nested inside.
fn list_marker(handle: &Handle) -> String {
    let Some(parent) = parent_handle(handle) else {
        return "\u{2022} ".to_string();
    };
    if element_tag(&parent) != "ol" {
        return "\u{2022} ".to_string();
    }
    let children = parent.children.borrow();
    let index = children
        .iter()
        .filter(|c| element_tag(c) == "li")
        .position(|c| Rc::ptr_eq(c, handle))
        .map(|i| i + 1)
        .unwrap_or(1);
    format!("{index}. ")
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
    font_size: Option<f32>,
    font_weight: Option<FontWeight>,
    font_style: Option<FontStyle>,
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

fn parse_font_weight(input: &mut CssParser) -> Result<FontWeight, ()> {
    match input.next().map_err(|_| ())? {
        Token::Ident(ident) if ident.eq_ignore_ascii_case("normal") => Ok(FontWeight::NORMAL),
        Token::Ident(ident) if ident.eq_ignore_ascii_case("bold") => Ok(FontWeight::BOLD),
        Token::Number { value, .. } => Ok(FontWeight(value.round() as u16)),
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
        } else if name.eq_ignore_ascii_case("font-size") {
            if let Ok(v) = parse_px_length(input) {
                self.decls.font_size = Some(v);
            }
        } else if name.eq_ignore_ascii_case("font-weight") {
            if let Ok(w) = parse_font_weight(input) {
                self.decls.font_weight = Some(w);
            }
        } else if name.eq_ignore_ascii_case("font-style") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.font_style = if ident.eq_ignore_ascii_case("normal") {
                    Some(FontStyle::Normal)
                } else if ident.eq_ignore_ascii_case("italic") {
                    Some(FontStyle::Italic)
                } else if ident.eq_ignore_ascii_case("oblique") {
                    Some(FontStyle::Oblique)
                } else {
                    None
                };
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

/// The subset of computed style that inherits from parent to child
/// (`color`, and — from v6 — `font-size`/`font-weight`/`font-style`),
/// threaded through tree-building instead of re-deriving from scratch.
#[derive(Clone, Copy)]
struct InheritedStyle {
    color: TextColor,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
}

const ROOT_STYLE: InheritedStyle = InheritedStyle {
    color: TextColor::rgb(0, 0, 0),
    font_size: 16.0,
    font_weight: FontWeight::NORMAL,
    font_style: FontStyle::Normal,
};

#[derive(Clone, Copy)]
struct ComputedStyle {
    color: TextColor,
    background_color: Option<TextColor>,
    display: Display,
    margin: f32,
    padding: f32,
    border_width: f32,
    border_color: Option<TextColor>,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
}

impl ComputedStyle {
    fn inherited(&self) -> InheritedStyle {
        InheritedStyle {
            color: self.color,
            font_size: self.font_size,
            font_weight: self.font_weight,
            font_style: self.font_style,
        }
    }
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

/// Default (`font-size`, `font-weight`) per heading tag, standing in for
/// the sizing a full user-agent stylesheet would otherwise supply (there
/// isn't one — same reasoning as `default_display` above). Only consulted
/// as the cascade's fallback, so any author CSS still overrides it.
fn default_heading_font(tag: &str) -> Option<(f32, FontWeight)> {
    match tag {
        "h1" => Some((32.0, FontWeight::BOLD)),
        "h2" => Some((24.0, FontWeight::BOLD)),
        "h3" => Some((19.0, FontWeight::BOLD)),
        "h4" => Some((16.0, FontWeight::BOLD)),
        "h5" => Some((13.0, FontWeight::BOLD)),
        "h6" => Some((11.0, FontWeight::BOLD)),
        _ => None,
    }
}

/// Default (`margin`, `padding`) for `ul`/`ol`, standing in for the
/// `margin-block: 1em; padding-inline-start: 40px` a full user-agent
/// stylesheet would otherwise supply (there isn't one — same reasoning as
/// `default_display`/`default_heading_font` above), folded into scratchy's
/// existing single-scalar, all-sides margin/padding model rather than a
/// new per-side one. Only consulted as the cascade's fallback, so any
/// author CSS still overrides it.
fn default_list_spacing(tag: &str) -> Option<(f32, f32)> {
    match tag {
        "ul" | "ol" => Some((16.0, 40.0)),
        _ => None,
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

fn resolve_style(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle) -> ComputedStyle {
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
        .unwrap_or(inherited.color);
    let background_color = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.background_color),
        |d| d.background_color,
    )
    .and_then(|c| to_text_color(&c));
    let display = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.display), |d| d.display)
        .unwrap_or_else(|| default_display(tag));
    let list_spacing = default_list_spacing(tag);
    let margin = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin), |d| d.margin)
        .or_else(|| list_spacing.map(|(margin, _)| margin))
        .unwrap_or(0.0);
    let padding = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.padding), |d| d.padding)
        .or_else(|| list_spacing.map(|(_, padding)| padding))
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
    let heading_default = default_heading_font(tag);
    let font_size = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.font_size), |d| d.font_size)
        .or_else(|| heading_default.map(|(size, _)| size))
        .unwrap_or(inherited.font_size);
    let font_weight = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.font_weight),
        |d| d.font_weight,
    )
    .or_else(|| heading_default.map(|(_, weight)| weight))
    .unwrap_or(inherited.font_weight);
    let font_style = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.font_style),
        |d| d.font_style,
    )
    .unwrap_or(inherited.font_style);

    ComputedStyle {
        color,
        background_color,
        display,
        margin,
        padding,
        border_width,
        border_color,
        font_size,
        font_weight,
        font_style,
    }
}

fn anonymous_style(inherited: InheritedStyle) -> ComputedStyle {
    ComputedStyle {
        color: inherited.color,
        background_color: None,
        display: Display::Block,
        margin: 0.0,
        padding: 0.0,
        border_width: 0.0,
        border_color: None,
        font_size: inherited.font_size,
        font_weight: inherited.font_weight,
        font_style: inherited.font_style,
    }
}

fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------- layout tree ----------

/// Per-span style for one piece of inline text — everything a text run
/// needs from its nearest ancestor's computed style to be shaped
/// correctly, carried independently of the anonymous inline box's own
/// style since a single run can mix spans from different elements
/// (e.g. plain text next to a `<b>`).
#[derive(Clone, Copy)]
struct SpanStyle {
    color: TextColor,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
}

impl From<InheritedStyle> for SpanStyle {
    fn from(s: InheritedStyle) -> Self {
        SpanStyle {
            color: s.color,
            font_size: s.font_size,
            font_weight: s.font_weight,
            font_style: s.font_style,
        }
    }
}

enum BoxContent {
    Blocks(Vec<LayoutBox>),
    Inline(Vec<(String, SpanStyle)>),
}

struct LayoutBox {
    style: ComputedStyle,
    content: BoxContent,
}

fn build_box_for_element(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle) -> LayoutBox {
    let style = resolve_style(handle, rules, inherited);
    let prefix = if element_tag(handle) == "li" {
        Some((list_marker(handle), style.inherited().into()))
    } else {
        None
    };
    LayoutBox {
        style,
        content: BoxContent::Blocks(build_children_with_prefix(handle, rules, style.inherited(), prefix)),
    }
}

/// Walks the children of `handle`, starting a new box for each block-level
/// one and grouping consecutive inline-level content (including text) into
/// anonymous inline-run boxes. Recurses *through* inline wrapper elements
/// (carrying their resolved color forward) rather than flattening them
/// outright — a block element nested inside something that looks inline at
/// first glance (routine in real-world markup, e.g. a `<div>` wrapping a
/// nav `<ul>`) still gets its own box this way, instead of being absorbed
/// into one giant text run. (An earlier version flattened unconditionally;
/// on a real Wikipedia page that merged the entire navigation chrome into a
/// single ~17,000-character run, which took 7+ seconds to shape by itself.)
fn build_children(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle) -> Vec<LayoutBox> {
    build_children_with_prefix(handle, rules, inherited, None)
}

/// Same as `build_children`, but seeds the first inline run with `prefix`
/// (used for a `<li>`'s marker) before any of `handle`'s own children.
fn build_children_with_prefix(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    prefix: Option<(String, SpanStyle)>,
) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    let mut pending: Vec<(String, SpanStyle)> = Vec::new();
    if let Some(p) = prefix {
        pending.push(p);
    }
    collect_children(handle, rules, inherited, &mut pending, &mut result);
    if !pending.is_empty() {
        result.push(LayoutBox {
            style: anonymous_style(inherited),
            content: BoxContent::Inline(pending),
        });
    }
    result
}

fn collect_children(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    pending: &mut Vec<(String, SpanStyle)>,
    result: &mut Vec<LayoutBox>,
) {
    for child in handle.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => {
                let mut text = normalize_whitespace(&contents.borrow());
                text.push(' ');
                pending.push((text, inherited.into()));
            }
            NodeData::Element { name, .. } => {
                let tag = name.local.as_ref();
                if tag == "script" || tag == "style" {
                    continue;
                }
                let style = resolve_style(child, rules, inherited);
                if style.display == Display::None {
                    continue;
                }
                if style.display == Display::Block {
                    if !pending.is_empty() {
                        result.push(LayoutBox {
                            style: anonymous_style(inherited),
                            content: BoxContent::Inline(std::mem::take(pending)),
                        });
                    }
                    result.push(build_box_for_element(child, rules, inherited));
                } else {
                    collect_children(child, rules, style.inherited(), pending, result);
                }
            }
            _ => {}
        }
    }
}

fn build_layout_tree(document: &Handle) -> LayoutBox {
    let mut css_source = String::new();
    collect_style_css(document, &mut css_source);
    let rules = parse_stylesheet(&css_source);
    LayoutBox {
        style: anonymous_style(ROOT_STYLE),
        content: BoxContent::Blocks(build_children(document, &rules, ROOT_STYLE)),
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
/// not yet scroll-adjusted) plus either its laid-out children or a stable id
/// identifying its shaped text in the shape cache (see `measure`).
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
    Text(usize),
}

/// A paragraph's shaped text, keyed by a stable id assigned during `measure`
/// in deterministic document order. Buffers live here — not inside `Laid` —
/// and persist across frames, so shaping a paragraph once is never repeated
/// (see `measure`'s budget/incomplete mechanism).
type ShapeCache = std::collections::HashMap<usize, (Buffer, f32)>;

/// Height assumed for a paragraph that hasn't been shaped yet, so content
/// below it has somewhere reasonable to sit until the real height arrives.
const PLACEHOLDER_LINE_HEIGHT: f32 = 20.0;

/// Asks the system's real fontconfig (via `fc-match`, the same resolution
/// path Firefox and other fontconfig-aware apps use) which actually-
/// installed family a generic name like "sans-serif" currently resolves
/// to. Returns `None` on any failure (command missing, non-zero exit,
/// empty output) rather than guessing.
fn fc_match_family(generic: &str) -> Option<String> {
    let output = std::process::Command::new("fc-match")
        .args(["-f", "%{family}", generic])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Builds a `FontSystem` whose generic family mapping (what `sans-serif`
/// etc. actually resolve to) comes from the system's real, installed
/// fonts, instead of `FontSystem::new()`'s hardcoded "Open Sans"/"Noto
/// Sans Mono"/"DejaVu Serif" — fonts that may not be installed at all, in
/// which case every glyph silently falls back to an arbitrary font that
/// merely happens to contain it (e.g. an emoji font for digits), rather
/// than the system's actual default. `fontdb::Database::load_system_fonts`
/// does its own naive parse of fontconfig's `<alias>` config and can
/// still land on an uninstalled family (measured on this machine: it
/// picks "FreeSans", which isn't installed either) — `fc-match` instead
/// resolves against the fonts that are actually present, so it's used to
/// override the three generic families explicitly when available.
fn new_font_system() -> FontSystem {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    if let Some(name) = fc_match_family("sans-serif") {
        db.set_sans_serif_family(name);
    }
    if let Some(name) = fc_match_family("monospace") {
        db.set_monospace_family(name);
    }
    if let Some(name) = fc_match_family("serif") {
        db.set_serif_family(name);
    }
    let locale = sys_locale::get_locale().unwrap_or_else(|| String::from("en-US"));
    FontSystem::new_with_locale_and_db(locale, db)
}

fn shape_spans(font_system: &mut FontSystem, spans: &[(String, SpanStyle)], width: f32) -> (Buffer, f32) {
    let metrics = Metrics::new(16.0, 20.0);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(Some(width.max(0.0)), None);
    let rich: Vec<(&str, Attrs)> = spans
        .iter()
        .map(|(text, span)| {
            let mut attrs = Attrs::new();
            attrs.color_opt = Some(span.color);
            attrs.weight = span.font_weight;
            attrs.style = span.font_style;
            attrs = attrs.metrics(Metrics::new(span.font_size, span.font_size * 1.25));
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

/// Lays out `b`, shaping text as it goes — but only until `deadline` passes.
/// Each paragraph gets a stable id (assigned in document order, so it's the
/// same id across repeated calls on the same tree) and its shaped result is
/// cached in `cache`, keyed by that id, so a paragraph already shaped on an
/// earlier call is never re-shaped. Once the deadline passes, not-yet-shaped
/// paragraphs get a placeholder height and `*incomplete` is set — the caller
/// is expected to call `measure` again (another frame, same `cache`) to pick
/// up where this call left off, until a full pass needs no new shaping at
/// all.
fn measure(
    b: &LayoutBox,
    font_system: &mut FontSystem,
    x: f32,
    y: f32,
    width: f32,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
) -> Laid {
    let s = b.style;
    let border_x = x + s.margin;
    let border_y = y + s.margin;
    let border_w = (width - 2.0 * s.margin).max(0.0);
    let content_x = border_x + s.border_width + s.padding;
    let content_y = border_y + s.border_width + s.padding;
    let content_w = (border_w - 2.0 * (s.border_width + s.padding)).max(0.0);

    match &b.content {
        BoxContent::Inline(spans) => {
            let id = *next_id;
            *next_id += 1;

            let content_h = if let Some((_, h)) = cache.get(&id) {
                *h
            } else if spans.is_empty() {
                cache.insert(id, (Buffer::new_empty(Metrics::new(16.0, 20.0)), 0.0));
                0.0
            } else if std::time::Instant::now() < deadline {
                let (buffer, h) = shape_spans(font_system, spans, content_w);
                cache.insert(id, (buffer, h));
                h
            } else {
                *incomplete = true;
                PLACEHOLDER_LINE_HEIGHT
            };

            let border_h = 2.0 * (s.border_width + s.padding) + content_h;
            Laid {
                style: s,
                x: border_x,
                y: border_y,
                w: border_w,
                h: border_h,
                content: LaidContent::Text(id),
            }
        }
        BoxContent::Blocks(children) => {
            let mut cursor_y = content_y;
            let mut laid_children = Vec::with_capacity(children.len());
            for child in children {
                let lc = measure(
                    child, font_system, content_x, cursor_y, content_w, next_id, cache, deadline,
                    incomplete,
                );
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
    laid: &Laid,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    cache: &mut ShapeCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    scroll_offset: f32,
) {
    let s = laid.style;
    let draw_y = laid.y - scroll_offset;
    if draw_y + laid.h < 0.0 || draw_y > win_h as f32 {
        // Entirely outside the viewport. Every descendant's y-range is
        // contained within this box's, so none of them can be visible
        // either — skip the whole subtree instead of just clipping
        // per-pixel, since that's what was making every redraw cost
        // O(whole document) instead of O(visible content).
        return;
    }
    paint_box_decoration(&s, pixels, win_w, win_h, laid.x, draw_y, laid.w, laid.h);

    match &laid.content {
        LaidContent::Blocks(children) => {
            for child in children {
                paint(child, font_system, swash_cache, cache, pixels, win_w, win_h, scroll_offset);
            }
        }
        LaidContent::Text(id) => {
            // Not yet shaped (still mid-way through a budgeted measure pass)
            // just means nothing to paint here yet — it'll appear once a
            // later frame finishes shaping it.
            if let Some((buffer, _)) = cache.get_mut(id) {
                let content_x = laid.x + s.border_width + s.padding;
                let content_y = draw_y + s.border_width + s.padding;
                paint_buffer(buffer, font_system, swash_cache, pixels, win_w, win_h, content_x, content_y);
            }
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
    scroll_offset: f32,
    /// Cached layout from the last `document`/width combination. Scrolling
    /// alone never invalidates this — relaying out and reshaping the whole
    /// page (expensive: real measurements showed full-document shaping can
    /// take seconds under full bidi/complex-script shaping, worse on large
    /// or RTL-heavy pages) on every mouse-wheel tick is what made scrolling
    /// feel laggy.
    cached: Option<Laid>,
    cached_width: u32,
    layout_dirty: bool,
    /// The box tree for the current layout session (built once from
    /// `document`+CSS, reused across every incremental `measure` tick until
    /// the document or width changes). Rebuilding this per tick — a full DOM
    /// walk plus cascade matching for every element — was itself costing
    /// hundreds of ms on large pages, swamping the shaping time budget
    /// before a single new paragraph even got shaped.
    tree: Option<LayoutBox>,
    /// Shaped paragraphs, keyed by the stable id `measure` assigns them.
    /// Outlives any single `measure` call — this is what lets layout be
    /// resumed across frames instead of reshaping the whole document in one
    /// blocking call (see `measure`'s deadline/incomplete mechanism).
    shape_cache: ShapeCache,
    /// Set when the last `measure` pass ran out of its time budget before
    /// shaping every paragraph. `redraw` keeps re-triggering itself while
    /// this is true, making progress a bit further each frame.
    layout_incomplete: bool,
    /// The size `surface.resize()` was last called with — skip the call
    /// when nothing changed instead of re-issuing it on every redraw.
    surface_size: (u32, u32),
    /// The width seen on the previous redraw, tracked unconditionally
    /// (whether or not a rebuild is needed). Tiling window managers
    /// commonly resize a window immediately after creating it (request
    /// size, then retile to the actual tile size) — committing to a
    /// rebuild as soon as one looks needed meant the very first layout
    /// session routinely started at a width that was already obsolete a
    /// tick later, throwing that work away. A rebuild only commits once
    /// the width has been the same for two ticks in a row.
    pending_width: Option<u32>,
}

impl App {
    fn new_static(document: Handle) -> Self {
        Self {
            document,
            decoder: None,
            window: None,
            surface: None,
            font_system: new_font_system(),
            swash_cache: SwashCache::new(),
            scroll_offset: 0.0,
            cached: None,
            cached_width: 0,
            layout_dirty: true,
            tree: None,
            shape_cache: ShapeCache::new(),
            layout_incomplete: false,
            surface_size: (0, 0),
            pending_width: None,
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
            font_system: new_font_system(),
            swash_cache: SwashCache::new(),
            scroll_offset: 0.0,
            cached: None,
            cached_width: 0,
            layout_dirty: true,
            tree: None,
            shape_cache: ShapeCache::new(),
            layout_incomplete: false,
            surface_size: (0, 0),
            pending_width: None,
        }
    }

    fn redraw(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        if self.surface_size != (width, height) {
            surface
                .resize(NonZeroU32::new(width).unwrap(), NonZeroU32::new(height).unwrap())
                .unwrap();
            self.surface_size = (width, height);
        }

        // Track the width seen on consecutive ticks unconditionally — not
        // just while a rebuild is pending — so even the very first rebuild
        // (layout_dirty from construction) waits for it to settle. The
        // first redraw after window creation routinely still reports the
        // requested size, not the tiling window manager's actual tile size
        // a tick later; building at the wrong one and then immediately
        // discarding it when the real size arrives was wasting real time
        // (measured: ~600ms of duplicate shaping on a small page).
        let width_matches_pending = self.pending_width == Some(width);
        self.pending_width = Some(width);
        let need_rebuild = self.layout_dirty || self.cached_width != width;

        if need_rebuild && width_matches_pending {
            self.tree = Some(build_layout_tree(&self.document));
            self.shape_cache.clear();
            self.cached_width = width;
            self.layout_dirty = false;
            self.layout_incomplete = true;
        } else if need_rebuild {
            window.request_redraw();
        }

        if self.tree.is_some() && (self.cached.is_none() || self.layout_incomplete) {
            // Each tick repaints the whole accumulated page regardless of
            // how much new shaping it did, and that fixed per-tick overhead
            // (paint+present+compositor wait) measures at ~50-60ms — so a
            // small budget mostly pays overhead rather than shaping. On a
            // 151-paragraph page: 80ms budget -> ~6-8 paragraphs/tick, 58
            // ticks, 7.1s total; 500ms -> ~30-34 paragraphs/tick, ~12
            // ticks, 6.2s total. On a much larger, RTL page: 16ms -> 306
            // ticks/17.0s; 80ms -> 90 ticks/13.0s; 150ms -> 53 ticks/12.6s
            // (diminishing returns past ~80ms there, since individual
            // complex-script paragraphs can themselves take hundreds of ms,
            // capping how much a larger budget can batch together). 500ms
            // trades a more noticeable per-tick pause for meaningfully less
            // total overhead; still far below the few-second range where a
            // pause starts reading as a freeze.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
            let mut incomplete = false;
            let mut next_id = 0usize;
            self.cached = Some(measure(
                self.tree.as_ref().unwrap(),
                &mut self.font_system,
                0.0,
                0.0,
                width as f32,
                &mut next_id,
                &mut self.shape_cache,
                deadline,
                &mut incomplete,
            ));
            self.layout_incomplete = incomplete;
        }
        let mut pixels = surface.buffer_mut().unwrap();
        for pixel in pixels.iter_mut() {
            *pixel = 0x00FFFFFF;
        }

        // `None` only on the very first tick or two, while we're still
        // waiting to see whether the width is about to change again (see
        // `pending_width`) — paint a blank frame rather than stall on it.
        if let Some(laid) = self.cached.as_ref() {
            let max_scroll = (laid.h - height as f32).max(0.0);
            self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll);

            paint(
                laid,
                &mut self.font_system,
                &mut self.swash_cache,
                &mut self.shape_cache,
                &mut pixels,
                width,
                height,
                self.scroll_offset,
            );
        }

        window.pre_present_notify();
        pixels.present().unwrap();

        if self.layout_incomplete {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window_attrs = Window::default_attributes()
            .with_title("scratchy")
            .with_inner_size(LogicalSize::new(1024.0, 768.0))
            .with_decorations(false);
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
                    self.layout_dirty = true;
                }
            }
            AppEvent::Eof => {
                if let Some(decoder) = self.decoder.take() {
                    let dom = decoder.finish();
                    self.document = dom.document;
                    self.layout_dirty = true;
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
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 40.0,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32,
                };
                self.scroll_offset -= dy;
                if self.scroll_offset < 0.0 {
                    self.scroll_offset = 0.0;
                }
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
