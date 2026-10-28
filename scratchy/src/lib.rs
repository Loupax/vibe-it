use std::env;
use std::fs;
use std::io::{self, Read};
use std::num::NonZeroU32;
use std::rc::{Rc, Weak};
use std::thread;

use cosmic_text::{
    Align as CosmicAlign, Attrs, Buffer, Color as TextColor, Family as CosmicFamily, FontSystem,
    LegacyRenderer, Metrics, Renderer as _, Shaping, Style as FontStyle, SwashCache,
    TextDecoration, UnderlineStyle, Weight as FontWeight, Wrap, render_decoration,
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

/// One-shot batch parse of a complete HTML string into a DOM root — the
/// `benches/` crate's entry point into the engine, so it doesn't need its
/// own `html5ever`/`TendrilSink` plumbing just to get a `Handle` to hand to
/// `build_layout_tree`.
pub fn parse_html(html: &str) -> Handle {
    new_parser().read_from(&mut html.as_bytes()).unwrap().document
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

    /// The `selectors` crate defaults this to `false` — without it,
    /// `:is(...)` fails to *parse* at all, silently dropping the whole
    /// selector (and so the whole rule) it appears in, same as any other
    /// unparseable selector. Found via `default.css`'s nesting-depth-based
    /// `list-style-type` rules (`:is(ol, ul, menu, dir) :is(ul, menu,
    /// dir) { list-style-type: circle; }` etc., since v11) silently never
    /// matching anything — which also means the v9 "nested lists have no
    /// top/bottom margin" `:is(...):is(...)` rule never actually worked
    /// either, now a real visible bug since v14 made `em` margins active.
    fn parse_is_and_where(&self) -> bool {
        true
    }
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
/// `style_type` is the `<li>`'s own *cascaded* `list-style-type` (inherited
/// from its `ul`/`ol` ancestor in the normal CSS sense, by the time this is
/// called — see `build_box_for_element`) — this no longer hardcodes "count
/// only if the parent tag is literally `ol`"; an explicit `list-style-type:
/// decimal` on a `<ul>` counts too, same as any real browser.
fn list_marker(handle: &Handle, style_type: ListStyleType) -> String {
    match style_type {
        ListStyleType::Disc => "\u{2022} ".to_string(),
        ListStyleType::Circle => "\u{25E6} ".to_string(),
        ListStyleType::Square => "\u{25AA} ".to_string(),
        ListStyleType::Decimal => {
            let Some(parent) = parent_handle(handle) else {
                return "1. ".to_string();
            };
            let children = parent.children.borrow();
            let index = children
                .iter()
                .filter(|c| element_tag(c) == "li")
                .position(|c| Rc::ptr_eq(c, handle))
                .map(|i| i + 1)
                .unwrap_or(1);
            format!("{index}. ")
        }
    }
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
    /// `display: table` — a block-level container whose content is laid
    /// out as table rows rather than normal block/inline flow.
    Table,
    /// `display: table-row` — a `tr`: its own box stacks vertically like
    /// a block (inside the table), but its content lays its cells out
    /// horizontally rather than stacking them.
    TableRow,
    /// `display: table-cell` — a `td`/`th`: behaves like a normal block
    /// container for its own content (text/inline/nested blocks all work
    /// unchanged), just laid out horizontally by its row rather than
    /// vertically by a parent block.
    TableCell,
    /// `table-row-group`/`table-header-group`/`table-footer-group`
    /// (`tbody`/`thead`/`tfoot`) and `table-column-group`/`table-column`
    /// (`colgroup`/`col`) — purely structural wrappers with no box of
    /// their own; `thead`/`tbody`/`tfoot` are transparently walked
    /// through for the `tr`s inside, `colgroup`/`col` carry no visual
    /// content in this simplified (equal-width-column) table layout, so
    /// walking through them finds nothing and they contribute no box
    /// either way.
    TableGroup,
}

/// A parsed-but-not-yet-resolved length: `px` is an absolute value, `em` is
/// relative to a font-size that isn't known until cascade time (an
/// element's own computed font-size, for margin/padding/border-width; its
/// *parent's* computed font-size, for font-size itself — see
/// `resolve_style`). `rem` and `%` remain unsupported, same as any other
/// unparseable value: no real use for either in `default.css` today (see
/// TODO.txt), so not adding them until something needs them.
///
/// `Larger`/`Smaller` are `font-size`'s own two relative keywords —
/// deliberately not reachable through the general `parse_length`/
/// `set_length` used by every other length-ish property (`margin:
/// larger` isn't meaningful), only through `font-size`'s own dedicated
/// parse branch — but resolve through the exact same `resolve_style`
/// code path as `Em` already does for `font-size` (against the
/// *parent's* computed font-size), so no change needed there. `1.2`
/// isn't a spec'd exact value (the real spec steps through an
/// absolute-size keyword table) — just the common approximation
/// browsers converge on, and the same ratio `default.css`'s own
/// heading sizes roughly follow.
#[derive(Clone, Copy)]
enum Length {
    Px(f32),
    Em(f32),
    Larger,
    Smaller,
}

impl Length {
    fn resolve(self, font_size: f32) -> f32 {
        match self {
            Length::Px(v) => v,
            Length::Em(v) => v * font_size,
            Length::Larger => font_size * 1.2,
            Length::Smaller => font_size / 1.2,
        }
    }
}

/// `line-height`'s three value forms. Kept symbolic (not resolved to a
/// pixel value) all the way through `InheritedStyle`/`ComputedStyle` —
/// unlike every other length-ish property here, `line-height`'s inherited
/// value is the *declared* form, not a flattened pixel number: a bare
/// `<number>` (`Multiplier`) or `normal` re-resolves against each
/// descendant's own font-size, not the declaring element's (the one real
/// CSS property where this distinction is observable — e.g. `body {
/// line-height: 1.5 }` should still give `h1` a line-height scaled to
/// `h1`'s own, larger font-size, not `body`'s). Only resolved to an
/// actual pixel value at the point something needs one
/// (`SpanStyle::from`, for `cosmic_text::Metrics`).
#[derive(Clone, Copy, Default)]
enum LineHeight {
    #[default]
    Normal,
    Multiplier(f32),
    Len(Length),
}

impl LineHeight {
    /// `1.25` for `Normal` matches cosmic-text's own prior hardcoded
    /// default (`Metrics::new(font_size, font_size * 1.25)`), so a page
    /// that never declares `line-height` renders exactly as before this
    /// feature existed.
    fn resolve(self, font_size: f32) -> f32 {
        match self {
            LineHeight::Normal => font_size * 1.25,
            LineHeight::Multiplier(m) => font_size * m,
            LineHeight::Len(l) => l.resolve(font_size),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Declarations {
    color: Option<CssColor>,
    background_color: Option<CssColor>,
    display: Option<Display>,
    margin_top: Option<Length>,
    margin_right: Option<Length>,
    margin_bottom: Option<Length>,
    margin_left: Option<Length>,
    padding_top: Option<Length>,
    padding_right: Option<Length>,
    padding_bottom: Option<Length>,
    padding_left: Option<Length>,
    border_width: Option<Length>,
    border_color: Option<CssColor>,
    border_style: Option<BorderStyle>,
    font_size: Option<Length>,
    font_weight: Option<FontWeight>,
    font_style: Option<FontStyle>,
    font_family: Option<FontFamily>,
    text_decoration: Option<TextDecoration>,
    white_space: Option<WhiteSpace>,
    list_style_type: Option<ListStyleType>,
    text_align: Option<TextAlign>,
    vertical_align: Option<VerticalAlign>,
    line_height: Option<LineHeight>,
    visibility: Option<Visibility>,
    outline_width: Option<Length>,
    outline_color: Option<CssColor>,
    box_shadow: Option<BoxShadow>,
    border_radius: Option<Length>,
}

/// Just `normal` (the default — current behavior, collapse+wrap) and `pre`
/// (`default.css`'s only real use, on `pre`/`listing`/`xmp`/`plaintext`) —
/// no `pre-wrap`/`pre-line`/`nowrap`, nothing declares them.
#[derive(Clone, Copy, PartialEq, Default)]
enum WhiteSpace {
    #[default]
    Normal,
    Pre,
}

/// Just the 4 keywords `default.css` actually declares (`disc`/`decimal`
/// directly on `ul`/`ol`, `circle`/`square` for nested lists by depth) — no
/// `none`, no the-many-other-counter-styles (roman numerals, alpha, ...).
/// `Disc` as `#[default]` matches the real CSS initial value.
#[derive(Clone, Copy, PartialEq, Default)]
enum ListStyleType {
    #[default]
    Disc,
    Circle,
    Square,
    Decimal,
}

/// The 4 keywords `text-align` actually has (no `start`/`end` — nothing
/// declares them, and they're writing-mode-relative, which scratchy has
/// no notion of). `Left` as `#[default]` matches the real CSS initial
/// value (for left-to-right content, which is all scratchy renders).
#[derive(Clone, Copy, PartialEq, Default)]
enum TextAlign {
    #[default]
    Left,
    Right,
    Center,
    Justify,
}

impl TextAlign {
    /// `None` for `Left` rather than `Some(CosmicAlign::Left)` — avoids
    /// relying on cosmic-text's own default matching ours, and `None` is
    /// what `shape_spans` always passed before this property existed, so
    /// the common case takes the exact code path it always has.
    fn to_cosmic(self) -> Option<CosmicAlign> {
        match self {
            TextAlign::Left => None,
            TextAlign::Right => Some(CosmicAlign::Right),
            TextAlign::Center => Some(CosmicAlign::Center),
            TextAlign::Justify => Some(CosmicAlign::Justified),
        }
    }
}

/// Just the 2 non-default keywords `default.css` declares (`sub`/`sup`'s
/// own `vertical-align: sub`/`super`) — no `top`/`middle`/`bottom`/
/// `text-top`/`text-bottom`/length/percentage, nothing needs them.
/// `Baseline` (`#[default]`, the real CSS initial value) is also what
/// `vertical-align: inherit` on `default.css`'s `tr`/`td`/`th` resolves
/// to in practice: `inherit` isn't a keyword any property here parses
/// (see TODO.txt-adjacent note in `collect_children`), so those
/// declarations just drop — fine, since nothing above them in the
/// cascade ever sets a non-Baseline value for `inherit` to meaningfully
/// carry anyway. Unlike `text-align`, this is *not* threaded through
/// `InheritedStyle` — `vertical-align` genuinely isn't a spec-inherited
/// property; see `collect_children`'s `valign` parameter for how it
/// actually propagates (ambient, like `background-color`/
/// `text-decoration`, not real inheritance).
#[derive(Clone, Copy, PartialEq, Default)]
pub enum VerticalAlign {
    #[default]
    Baseline,
    Sub,
    Super,
}

impl VerticalAlign {
    /// Pixels to shift a glyph's baseline by (positive = down), scaled to
    /// that glyph's own font size so it still looks right if `font-size`
    /// ever varies within a sub/sup span. Not real font-metric
    /// `subscriptYOffset`/`superscriptYOffset` values (most fonts don't
    /// expose those through `fontdb`/`cosmic-text` at all) — just the
    /// same rough fractions browsers fall back to when a font doesn't
    /// supply its own.
    fn y_shift(self, font_size: f32) -> f32 {
        match self {
            VerticalAlign::Baseline => 0.0,
            VerticalAlign::Sub => font_size * 0.2,
            VerticalAlign::Super => font_size * -0.35,
        }
    }
}

/// Just `visible`/`hidden` — no `collapse` (table row/column-specific,
/// and `default.css` never uses it). Unlike `display: none`, `visibility:
/// hidden` still reserves layout space and — the whole reason it's a
/// distinct property — a descendant can set `visibility: visible` to
/// become visible again despite a hidden ancestor, so this genuinely is
/// CSS-inherited (threaded through `InheritedStyle`, like `text-align`,
/// not the ambient-propagation pattern `vertical-align` uses — an
/// element's own `inherited.visibility` at the point a text node or box
/// is built already reflects the correctly-cascaded value, descendant
/// overrides included, with no extra propagation parameter needed).
#[derive(Clone, Copy, PartialEq, Default)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
}

/// Only the five CSS generic family keywords — no named families
/// (`"Georgia"`, `Helvetica`, ...) and no comma-separated fallback lists,
/// since nothing in `default.css` or any test page so far has needed
/// either (see TODO.txt). `SansSerif` is the default so every element
/// without a `font-family` rule renders exactly as before this feature —
/// matches cosmic-text's own `Attrs::new()` default.
#[derive(Clone, Copy, PartialEq, Default)]
enum FontFamily {
    #[default]
    SansSerif,
    Serif,
    Monospace,
    Cursive,
    Fantasy,
}

impl FontFamily {
    fn to_cosmic(self) -> CosmicFamily<'static> {
        match self {
            FontFamily::SansSerif => CosmicFamily::SansSerif,
            FontFamily::Serif => CosmicFamily::Serif,
            FontFamily::Monospace => CosmicFamily::Monospace,
            FontFamily::Cursive => CosmicFamily::Cursive,
            FontFamily::Fantasy => CosmicFamily::Fantasy,
        }
    }
}

struct StyleRule {
    selectors: SelectorList<OurSelectorImpl>,
    decls: Declarations,
}

fn parse_length(input: &mut CssParser) -> Result<Length, ()> {
    match input.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => Ok(Length::Px(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("em") => Ok(Length::Em(*value)),
        Token::Number { value, .. } if *value == 0.0 => Ok(Length::Px(0.0)),
        _ => Err(()),
    }
}

/// Parses a length into `*target`, leaving it untouched (so an earlier
/// declaration in the cascade isn't clobbered) on anything unparseable —
/// `rem`/`%`/`auto`/etc. all fall through this way today, same as any other
/// not-yet-supported value.
fn set_length(input: &mut CssParser, target: &mut Option<Length>) {
    if let Ok(v) = parse_length(input) {
        *target = Some(v);
    }
}

/// `line-height`'s own grammar — a bare `<number>` (any value, not just
/// `0` — unlike `parse_length`'s unitless-zero special case, this is the
/// property's single most common real-world form) alongside `normal` and
/// the usual `px`/`em` lengths.
fn parse_line_height(input: &mut CssParser) -> Result<LineHeight, ()> {
    match input.next().map_err(|_| ())? {
        Token::Ident(ident) if ident.eq_ignore_ascii_case("normal") => Ok(LineHeight::Normal),
        Token::Number { value, .. } => Ok(LineHeight::Multiplier(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => {
            Ok(LineHeight::Len(Length::Px(*value)))
        }
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("em") => {
            Ok(LineHeight::Len(Length::Em(*value)))
        }
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

/// Parses the `text-decoration` shorthand down to just its line keywords
/// (`underline`/`line-through`/`overline`/`none`) — cosmic-text already
/// knows how to paint each (`buffer.draw` draws them automatically once
/// `Attrs::text_decoration` is set, using real font underline/strikethrough
/// metrics), so there's no geometry to hand-roll here, only parsing. Style
/// keywords (`dotted`/`dashed`/`wavy`/`double`/`solid`, e.g. `abbr[title]`'s
/// `text-decoration: dotted underline`) and an explicit color are accepted
/// in the value but not tracked — every decoration this draws is a plain
/// single line in the text's own color, never dotted/wavy/a different
/// color. Returns `Err` only if no line keyword was found at all, so a
/// value that's pure noise still drops like any other unsupported value.
fn parse_text_decoration(input: &mut CssParser) -> Result<TextDecoration, ()> {
    let mut deco = TextDecoration::new();
    let mut found = false;
    loop {
        match input.next() {
            Ok(Token::Ident(ident)) => {
                if ident.eq_ignore_ascii_case("underline") {
                    deco.underline = UnderlineStyle::Single;
                    found = true;
                } else if ident.eq_ignore_ascii_case("line-through") {
                    deco.strikethrough = true;
                    found = true;
                } else if ident.eq_ignore_ascii_case("overline") {
                    deco.overline = true;
                    found = true;
                } else if ident.eq_ignore_ascii_case("none") {
                    found = true;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    if found {
        Ok(deco)
    } else {
        Err(())
    }
}

/// Just the 4 keywords that paint visibly differently from a plain
/// solid stroke with this file's existing pixel-fill primitives —
/// `groove`/`ridge`/`inset`/`outset` would need real two-tone bevel
/// shading to look like anything but `solid` (see
/// `border_style_from_keyword`, which folds all four into `Solid`
/// rather than giving them their own inert variants).
#[derive(Clone, Copy, PartialEq, Default)]
enum BorderStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    Double,
    /// Same outcome as `hidden` — no border renders regardless of
    /// `border-width`/`border-color` — scratchy has no `display: table`
    /// border-conflict-resolution rules that would ever need to tell
    /// `none` and `hidden` apart.
    None,
}

fn border_style_from_keyword(kw: &str) -> BorderStyle {
    if kw.eq_ignore_ascii_case("dashed") {
        BorderStyle::Dashed
    } else if kw.eq_ignore_ascii_case("dotted") {
        BorderStyle::Dotted
    } else if kw.eq_ignore_ascii_case("double") {
        BorderStyle::Double
    } else if kw.eq_ignore_ascii_case("none") || kw.eq_ignore_ascii_case("hidden") {
        BorderStyle::None
    } else {
        // solid, groove, ridge, inset, outset — see `BorderStyle`'s docs.
        BorderStyle::Solid
    }
}

/// The `border` shorthand: `<border-width> || <border-style> ||
/// <border-color>`, each 0-1 times, any order. Same lenient philosophy
/// as `parse_text_decoration`: an unrecognized token is skipped rather
/// than failing the whole value, so one bad component doesn't throw away
/// a width/color that did parse.
/// `(width, style, color)` — what `border`'s own shorthand grammar
/// parses into, each component optional.
type BorderShorthand = (Option<Length>, Option<BorderStyle>, Option<CssColor>);

fn parse_border_shorthand(input: &mut CssParser) -> Result<BorderShorthand, ()> {
    const STYLE_KEYWORDS: &[&str] = &[
        "none", "hidden", "dotted", "dashed", "solid", "double", "groove", "ridge", "inset", "outset",
    ];
    let (width, style_kw, color) = parse_border_like_shorthand(input, STYLE_KEYWORDS)?;
    Ok((width, style_kw.map(border_style_from_keyword), color))
}

/// `outline`'s own grammar — same `<width> || <style> || <color>` shape
/// as `border`, but `outline-style` additionally allows `auto` (the
/// browser-native focus ring, not a real style keyword) and has no
/// `hidden` (that's `border-style`-only). Not tracked here either,
/// unlike `border-style` (v30) — outlines always paint as a plain solid
/// stroke regardless of declared style; consumed and discarded.
fn parse_outline_shorthand(input: &mut CssParser) -> Result<(Option<Length>, Option<CssColor>), ()> {
    const STYLE_KEYWORDS: &[&str] = &[
        "none", "auto", "dotted", "dashed", "solid", "double", "groove", "ridge", "inset", "outset",
    ];
    let (width, _style_kw, color) = parse_border_like_shorthand(input, STYLE_KEYWORDS)?;
    Ok((width, color))
}

/// Shared implementation behind `parse_border_shorthand`/
/// `parse_outline_shorthand` — identical grammar shape
/// (`<width> || <style> || <color>`, any order, each 0-1 times), only the
/// recognized style keywords differ. Returns whichever keyword string
/// (borrowed from `style_keywords`) actually matched, if any — callers
/// decide whether/how to interpret it (`border`'s wrapper maps it to a
/// `BorderStyle`; `outline`'s just discards it). An unrecognized token is
/// skipped rather than failing the whole value (same lenient philosophy
/// as `parse_text_decoration` — one bad component shouldn't throw away a
/// width/color that did parse).
/// `(width, matched style keyword, color)` — see `parse_border_like_shorthand`'s own docs.
type BorderLikeShorthand<'k> = (Option<Length>, Option<&'k str>, Option<CssColor>);

fn parse_border_like_shorthand<'k>(
    input: &mut CssParser,
    style_keywords: &[&'k str],
) -> Result<BorderLikeShorthand<'k>, ()> {
    let mut width = None;
    let mut style = None;
    let mut color = None;
    let mut found = false;
    loop {
        let state = input.state();
        if let Ok(v) = parse_length(input) {
            width = Some(v);
            found = true;
            continue;
        }
        input.reset(&state);
        if let Ok(Token::Ident(ident)) = input.next() {
            if let Some(&kw) = style_keywords.iter().find(|k| ident.eq_ignore_ascii_case(k)) {
                style = Some(kw);
                found = true;
                continue;
            }
        }
        input.reset(&state);
        if let Ok(c) = CssColor::parse(input) {
            color = Some(c);
            found = true;
            continue;
        }
        input.reset(&state);
        match input.next() {
            Ok(_) => {}
            Err(_) => break,
        }
    }
    if found {
        Ok((width, style, color))
    } else {
        Err(())
    }
}

/// A parsed (not-yet-resolved) `box-shadow` value. Deliberately narrower
/// than the real property: no `inset` (an inner shadow would need an
/// entirely different paint — clipped to the box's own content area,
/// not drawn outside it — so `parse_box_shadow` rejects the whole
/// declaration rather than render it as an outer shadow, which would
/// look wrong, not just incomplete), no blur (`blur-radius` is parsed,
/// so it doesn't make the declaration drop, but intentionally discarded
/// — every shadow paints as a hard-edged solid rect), no comma-separated
/// multiple shadows (just the first/only one), and `color` is required
/// (real CSS defaults a missing color to `currentColor`; nothing in
/// `default.css` or a real test page has needed that yet, so it isn't
/// implemented — see TODO.txt).
#[derive(Clone, Copy)]
struct BoxShadow {
    offset_x: Length,
    offset_y: Length,
    spread: Length,
    color: CssColor,
}

/// `<offset-x> <offset-y> <blur-radius>? <spread-radius>? <color>` — a
/// `<color>` is accepted at any position in the token stream (real CSS
/// allows it first or last), same token-loop style as the shorthand
/// parsers above, but *not* lenient about unrecognized tokens: an
/// unexpected token (most likely `inset`, or two colors) drops the whole
/// declaration rather than guessing at which length is which.
fn parse_box_shadow(input: &mut CssParser) -> Result<BoxShadow, ()> {
    let mut lengths: Vec<Length> = Vec::new();
    let mut color: Option<CssColor> = None;
    loop {
        let state = input.state();
        if let Ok(v) = parse_length(input) {
            lengths.push(v);
            continue;
        }
        input.reset(&state);
        if color.is_none() {
            if let Ok(c) = CssColor::parse(input) {
                color = Some(c);
                continue;
            }
            input.reset(&state);
        }
        match input.next() {
            Ok(_) => return Err(()),
            Err(_) => break,
        }
    }
    if lengths.len() < 2 {
        return Err(());
    }
    let color = color.ok_or(())?;
    Ok(BoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        // lengths[2] (blur-radius), if present, is read past above (so it
        // doesn't fall through to the reject-unrecognized-token branch)
        // but never stored — no blur rendering.
        spread: lengths.get(3).copied().unwrap_or(Length::Px(0.0)),
        color,
    })
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
        } else if name.eq_ignore_ascii_case("border-style") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.border_style = Some(border_style_from_keyword(ident));
            }
        } else if name.eq_ignore_ascii_case("border") {
            if let Ok((w, st, c)) = parse_border_shorthand(input) {
                if let Some(w) = w {
                    self.decls.border_width = Some(w);
                }
                if let Some(st) = st {
                    self.decls.border_style = Some(st);
                }
                if let Some(c) = c {
                    self.decls.border_color = Some(c);
                }
            }
        } else if name.eq_ignore_ascii_case("display") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.display = if ident.eq_ignore_ascii_case("block") {
                    Some(Display::Block)
                } else if ident.eq_ignore_ascii_case("inline") {
                    Some(Display::Inline)
                } else if ident.eq_ignore_ascii_case("none") {
                    Some(Display::None)
                } else if ident.eq_ignore_ascii_case("table") {
                    Some(Display::Table)
                } else if ident.eq_ignore_ascii_case("table-row") {
                    Some(Display::TableRow)
                } else if ident.eq_ignore_ascii_case("table-cell") {
                    Some(Display::TableCell)
                } else if matches!(
                    ident.to_ascii_lowercase().as_str(),
                    "table-row-group" | "table-header-group" | "table-footer-group" | "table-column-group"
                        | "table-column"
                ) {
                    Some(Display::TableGroup)
                } else if ident.eq_ignore_ascii_case("table-caption") {
                    // No separate caption-box behavior implemented (no
                    // caption-side/alignment handling) — a plain block is
                    // the right simplification: it still renders, in its
                    // source position, just without special placement.
                    Some(Display::Block)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("margin") {
            if let Ok(v) = parse_length(input) {
                self.decls.margin_top = Some(v);
                self.decls.margin_right = Some(v);
                self.decls.margin_bottom = Some(v);
                self.decls.margin_left = Some(v);
            }
        } else if name.eq_ignore_ascii_case("margin-top") {
            set_length(input, &mut self.decls.margin_top);
        } else if name.eq_ignore_ascii_case("margin-right") {
            set_length(input, &mut self.decls.margin_right);
        } else if name.eq_ignore_ascii_case("margin-bottom") {
            set_length(input, &mut self.decls.margin_bottom);
        } else if name.eq_ignore_ascii_case("margin-left") {
            set_length(input, &mut self.decls.margin_left);
        } else if name.eq_ignore_ascii_case("margin-block-start") {
            set_length(input, &mut self.decls.margin_top);
        } else if name.eq_ignore_ascii_case("margin-block-end") {
            set_length(input, &mut self.decls.margin_bottom);
        } else if name.eq_ignore_ascii_case("margin-inline-start") {
            set_length(input, &mut self.decls.margin_left);
        } else if name.eq_ignore_ascii_case("margin-inline-end") {
            set_length(input, &mut self.decls.margin_right);
        } else if name.eq_ignore_ascii_case("margin-block") {
            if let Ok(v) = parse_length(input) {
                self.decls.margin_top = Some(v);
                self.decls.margin_bottom = Some(v);
            }
        } else if name.eq_ignore_ascii_case("margin-inline") {
            if let Ok(v) = parse_length(input) {
                self.decls.margin_left = Some(v);
                self.decls.margin_right = Some(v);
            }
        } else if name.eq_ignore_ascii_case("padding") {
            if let Ok(v) = parse_length(input) {
                self.decls.padding_top = Some(v);
                self.decls.padding_right = Some(v);
                self.decls.padding_bottom = Some(v);
                self.decls.padding_left = Some(v);
            }
        } else if name.eq_ignore_ascii_case("padding-top") {
            set_length(input, &mut self.decls.padding_top);
        } else if name.eq_ignore_ascii_case("padding-right") {
            set_length(input, &mut self.decls.padding_right);
        } else if name.eq_ignore_ascii_case("padding-bottom") {
            set_length(input, &mut self.decls.padding_bottom);
        } else if name.eq_ignore_ascii_case("padding-left") {
            set_length(input, &mut self.decls.padding_left);
        } else if name.eq_ignore_ascii_case("padding-block-start") {
            set_length(input, &mut self.decls.padding_top);
        } else if name.eq_ignore_ascii_case("padding-block-end") {
            set_length(input, &mut self.decls.padding_bottom);
        } else if name.eq_ignore_ascii_case("padding-inline-start") {
            set_length(input, &mut self.decls.padding_left);
        } else if name.eq_ignore_ascii_case("padding-inline-end") {
            set_length(input, &mut self.decls.padding_right);
        } else if name.eq_ignore_ascii_case("padding-block") {
            if let Ok(v) = parse_length(input) {
                self.decls.padding_top = Some(v);
                self.decls.padding_bottom = Some(v);
            }
        } else if name.eq_ignore_ascii_case("padding-inline") {
            if let Ok(v) = parse_length(input) {
                self.decls.padding_left = Some(v);
                self.decls.padding_right = Some(v);
            }
        } else if name.eq_ignore_ascii_case("border-width") {
            if let Ok(v) = parse_length(input) {
                self.decls.border_width = Some(v);
            }
        } else if name.eq_ignore_ascii_case("outline-width") {
            if let Ok(v) = parse_length(input) {
                self.decls.outline_width = Some(v);
            }
        } else if name.eq_ignore_ascii_case("outline-color") {
            if let Ok(c) = CssColor::parse(input) {
                self.decls.outline_color = Some(c);
            }
        } else if name.eq_ignore_ascii_case("outline") {
            if let Ok((w, c)) = parse_outline_shorthand(input) {
                if let Some(w) = w {
                    self.decls.outline_width = Some(w);
                }
                if let Some(c) = c {
                    self.decls.outline_color = Some(c);
                }
            }
        } else if name.eq_ignore_ascii_case("box-shadow") {
            if let Ok(v) = parse_box_shadow(input) {
                self.decls.box_shadow = Some(v);
            }
        } else if name.eq_ignore_ascii_case("border-radius") {
            set_length(input, &mut self.decls.border_radius);
        } else if name.eq_ignore_ascii_case("font-size") {
            let state = input.state();
            let keyword = input.next().ok().and_then(|t| match t {
                Token::Ident(ident) if ident.eq_ignore_ascii_case("larger") => Some(Length::Larger),
                Token::Ident(ident) if ident.eq_ignore_ascii_case("smaller") => Some(Length::Smaller),
                _ => None,
            });
            if let Some(v) = keyword {
                self.decls.font_size = Some(v);
            } else {
                input.reset(&state);
                if let Ok(v) = parse_length(input) {
                    self.decls.font_size = Some(v);
                }
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
        } else if name.eq_ignore_ascii_case("font-family") {
            // Only a single bare generic keyword, no comma-separated
            // fallback list and no quoted/named families — see
            // `FontFamily`'s doc comment. A quoted family name's string
            // token fails `expect_ident` here, so e.g. `"Georgia", serif`
            // drops the whole declaration rather than falling back to
            // `serif`, same as any other unsupported value.
            if let Ok(ident) = input.expect_ident() {
                self.decls.font_family = if ident.eq_ignore_ascii_case("sans-serif") {
                    Some(FontFamily::SansSerif)
                } else if ident.eq_ignore_ascii_case("serif") {
                    Some(FontFamily::Serif)
                } else if ident.eq_ignore_ascii_case("monospace") {
                    Some(FontFamily::Monospace)
                } else if ident.eq_ignore_ascii_case("cursive") {
                    Some(FontFamily::Cursive)
                } else if ident.eq_ignore_ascii_case("fantasy") {
                    Some(FontFamily::Fantasy)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("text-decoration") {
            if let Ok(deco) = parse_text_decoration(input) {
                self.decls.text_decoration = Some(deco);
            }
        } else if name.eq_ignore_ascii_case("white-space") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.white_space = if ident.eq_ignore_ascii_case("normal") {
                    Some(WhiteSpace::Normal)
                } else if ident.eq_ignore_ascii_case("pre") {
                    Some(WhiteSpace::Pre)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("list-style-type") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.list_style_type = if ident.eq_ignore_ascii_case("disc") {
                    Some(ListStyleType::Disc)
                } else if ident.eq_ignore_ascii_case("circle") {
                    Some(ListStyleType::Circle)
                } else if ident.eq_ignore_ascii_case("square") {
                    Some(ListStyleType::Square)
                } else if ident.eq_ignore_ascii_case("decimal") {
                    Some(ListStyleType::Decimal)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("text-align") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.text_align = if ident.eq_ignore_ascii_case("left") {
                    Some(TextAlign::Left)
                } else if ident.eq_ignore_ascii_case("right") {
                    Some(TextAlign::Right)
                } else if ident.eq_ignore_ascii_case("center") {
                    Some(TextAlign::Center)
                } else if ident.eq_ignore_ascii_case("justify") {
                    Some(TextAlign::Justify)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("vertical-align") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.vertical_align = if ident.eq_ignore_ascii_case("baseline") {
                    Some(VerticalAlign::Baseline)
                } else if ident.eq_ignore_ascii_case("sub") {
                    Some(VerticalAlign::Sub)
                } else if ident.eq_ignore_ascii_case("super") {
                    Some(VerticalAlign::Super)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("line-height") {
            if let Ok(v) = parse_line_height(input) {
                self.decls.line_height = Some(v);
            }
        } else if name.eq_ignore_ascii_case("visibility") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.visibility = if ident.eq_ignore_ascii_case("visible") {
                    Some(Visibility::Visible)
                } else if ident.eq_ignore_ascii_case("hidden") {
                    Some(Visibility::Hidden)
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
    font_family: FontFamily,
    white_space: WhiteSpace,
    list_style_type: ListStyleType,
    text_align: TextAlign,
    line_height: LineHeight,
    visibility: Visibility,
}

const ROOT_STYLE: InheritedStyle = InheritedStyle {
    color: TextColor::rgb(0, 0, 0),
    font_size: 16.0,
    font_weight: FontWeight::NORMAL,
    font_style: FontStyle::Normal,
    font_family: FontFamily::SansSerif,
    white_space: WhiteSpace::Normal,
    list_style_type: ListStyleType::Disc,
    text_align: TextAlign::Left,
    line_height: LineHeight::Normal,
    visibility: Visibility::Visible,
};

/// Per-side box-model values. Physical (top/right/bottom/left), not logical
/// — scratchy only ever lays out horizontal-tb/LTR, so block-start/end and
/// inline-start/end collapse onto top/bottom and left/right respectively;
/// logical property names are expanded to these physical sides at parse
/// time (`DeclParser::parse_value`), same as `margin`/`padding` shorthands
/// are. `border_width` stays a single uniform scalar — nothing in
/// `default.css` sets a per-side border, so there's no real use for it yet.
#[derive(Clone, Copy, Default)]
struct Sides {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

#[derive(Clone, Copy)]
struct ComputedStyle {
    color: TextColor,
    background_color: Option<TextColor>,
    display: Display,
    margin: Sides,
    padding: Sides,
    border_width: f32,
    border_color: Option<TextColor>,
    border_style: BorderStyle,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    font_family: FontFamily,
    /// The cascaded-but-not-yet-propagated value — `None` means nothing
    /// declared on *this* element, in which case the nearest ancestor's
    /// decoration (if any) still shows through, same "ambient, not real
    /// CSS inheritance" threading as `background_color` uses (see
    /// `collect_children`) — text-decoration isn't a spec-inherited
    /// property either, but a UA still draws an ancestor's decoration
    /// through descendant inline boxes that don't override it.
    text_decoration: Option<TextDecoration>,
    white_space: WhiteSpace,
    list_style_type: ListStyleType,
    text_align: TextAlign,
    /// Not inherited (see `VerticalAlign`'s own docs) — propagates the
    /// same ambient way `text_decoration` does, via `collect_children`'s
    /// `valign` parameter, not through `inherited()`/`InheritedStyle`.
    vertical_align: VerticalAlign,
    /// Still symbolic (see `LineHeight`'s own docs) — only resolved to a
    /// pixel value in `SpanStyle::from`, where a concrete number is
    /// finally needed.
    line_height: LineHeight,
    visibility: Visibility,
    /// Not inherited (same as `border_width`/`border_color`) and doesn't
    /// affect layout at all (unlike `border`, which is part of the box
    /// model) — drawn one `outline_width` ring *outside* the border box,
    /// in `paint_box_decoration`, with no `outline-offset` gap (nothing
    /// needs one yet).
    outline_width: f32,
    outline_color: Option<TextColor>,
    /// `None` means no `box-shadow` declared (or an unsupported form —
    /// see `BoxShadow`'s own docs). Not inherited.
    box_shadow: Option<ResolvedBoxShadow>,
    /// Only rounds the plain background fill (`paint_box_decoration`) —
    /// combined with a visible border (`border_width > 0`), it's ignored
    /// and both fall back to square corners. A correctly-rounded *ring*
    /// needs real per-pixel inner/outer-boundary testing (the "paint a
    /// bigger solid shape, then a smaller one on top" trick this file
    /// otherwise favors doesn't work once the interior isn't guaranteed
    /// opaque — nothing to safely "restore" where the ring should leave
    /// a hollow center) — not worth building without `default.css` or a
    /// real page combining the two (see TODO.txt). Not inherited.
    border_radius: f32,
}

/// `BoxShadow` with its lengths resolved to pixels (against the
/// element's own font-size, same timing as `border-width`/`padding`'s
/// own `em` resolution) and its color converted to a paintable
/// `TextColor` — the form `paint_box_decoration` actually draws.
#[derive(Clone, Copy)]
struct ResolvedBoxShadow {
    offset_x: f32,
    offset_y: f32,
    spread: f32,
    color: TextColor,
}

impl ComputedStyle {
    fn inherited(&self) -> InheritedStyle {
        InheritedStyle {
            color: self.color,
            font_size: self.font_size,
            font_weight: self.font_weight,
            font_style: self.font_style,
            font_family: self.font_family,
            white_space: self.white_space,
            list_style_type: self.list_style_type,
            text_align: self.text_align,
            line_height: self.line_height,
            visibility: self.visibility,
        }
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
        .unwrap_or(Display::Inline);
    // Resolved before margin/padding/border-width: an `em` on any of those
    // is relative to *this* element's own font-size (not its parent's),
    // so font-size has to be cascaded and resolved first. An `em` on
    // font-size itself is relative to the parent's (`inherited.font_size`)
    // — there's no self-reference, since font-size can only ever be `em`
    // relative to the value it's inheriting from, never its own result.
    let font_size = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.font_size), |d| d.font_size)
        .map(|l| l.resolve(inherited.font_size))
        .unwrap_or(inherited.font_size);
    let margin = Sides {
        top: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_top), |d| d.margin_top)
            .map(|l| l.resolve(font_size))
            .unwrap_or(0.0),
        right: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_right), |d| d.margin_right)
            .map(|l| l.resolve(font_size))
            .unwrap_or(0.0),
        bottom: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_bottom), |d| d.margin_bottom)
            .map(|l| l.resolve(font_size))
            .unwrap_or(0.0),
        left: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_left), |d| d.margin_left)
            .map(|l| l.resolve(font_size))
            .unwrap_or(0.0),
    };
    let padding = Sides {
        top: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.padding_top), |d| d.padding_top)
            .map(|l| l.resolve(font_size))
            .unwrap_or(0.0),
        right: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.padding_right), |d| d.padding_right)
            .map(|l| l.resolve(font_size))
            .unwrap_or(0.0),
        bottom: cascade_property(
            &matches,
            inline_decls.as_ref().and_then(|d| d.padding_bottom),
            |d| d.padding_bottom,
        )
        .map(|l| l.resolve(font_size))
        .unwrap_or(0.0),
        left: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.padding_left), |d| d.padding_left)
            .map(|l| l.resolve(font_size))
            .unwrap_or(0.0),
    };
    let border_width = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_width),
        |d| d.border_width,
    )
    .map(|l| l.resolve(font_size))
    .unwrap_or(0.0);
    let border_color = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_color),
        |d| d.border_color,
    )
    .and_then(|c| to_text_color(&c));
    let border_style = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_style),
        |d| d.border_style,
    )
    .unwrap_or_default();
    let font_weight = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.font_weight),
        |d| d.font_weight,
    )
    .unwrap_or(inherited.font_weight);
    let font_style = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.font_style),
        |d| d.font_style,
    )
    .unwrap_or(inherited.font_style);
    let font_family = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.font_family),
        |d| d.font_family,
    )
    .unwrap_or(inherited.font_family);
    let text_decoration = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.text_decoration),
        |d| d.text_decoration,
    );
    let white_space = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.white_space),
        |d| d.white_space,
    )
    .unwrap_or(inherited.white_space);
    let list_style_type = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.list_style_type),
        |d| d.list_style_type,
    )
    .unwrap_or(inherited.list_style_type);
    let text_align = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.text_align),
        |d| d.text_align,
    )
    .unwrap_or(inherited.text_align);
    // Not inherited — no `inherited.vertical_align` to fall back to;
    // the initial value (`Baseline`, also `VerticalAlign::default()`)
    // applies to any element that doesn't declare it itself.
    let vertical_align = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.vertical_align),
        |d| d.vertical_align,
    )
    .unwrap_or_default();
    let line_height = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.line_height),
        |d| d.line_height,
    )
    .unwrap_or(inherited.line_height);
    let visibility = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.visibility),
        |d| d.visibility,
    )
    .unwrap_or(inherited.visibility);
    let outline_width = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.outline_width),
        |d| d.outline_width,
    )
    .map(|l| l.resolve(font_size))
    .unwrap_or(0.0);
    let outline_color = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.outline_color),
        |d| d.outline_color,
    )
    .and_then(|c| to_text_color(&c));
    let box_shadow = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.box_shadow),
        |d| d.box_shadow,
    )
    .and_then(|bs| {
        to_text_color(&bs.color).map(|color| ResolvedBoxShadow {
            offset_x: bs.offset_x.resolve(font_size),
            offset_y: bs.offset_y.resolve(font_size),
            spread: bs.spread.resolve(font_size),
            color,
        })
    });
    let border_radius = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_radius),
        |d| d.border_radius,
    )
    .map(|l| l.resolve(font_size))
    .unwrap_or(0.0);

    ComputedStyle {
        color,
        background_color,
        display,
        margin,
        padding,
        border_width,
        border_color,
        border_style,
        font_size,
        font_weight,
        font_style,
        font_family,
        text_decoration,
        white_space,
        list_style_type,
        text_align,
        vertical_align,
        line_height,
        visibility,
        outline_width,
        outline_color,
        box_shadow,
        border_radius,
    }
}

fn anonymous_style(inherited: InheritedStyle) -> ComputedStyle {
    ComputedStyle {
        color: inherited.color,
        background_color: None,
        display: Display::Block,
        margin: Sides::default(),
        padding: Sides::default(),
        border_width: 0.0,
        border_color: None,
        border_style: BorderStyle::default(),
        font_size: inherited.font_size,
        font_weight: inherited.font_weight,
        font_style: inherited.font_style,
        font_family: inherited.font_family,
        text_decoration: None,
        white_space: inherited.white_space,
        list_style_type: inherited.list_style_type,
        text_align: inherited.text_align,
        vertical_align: VerticalAlign::default(),
        line_height: inherited.line_height,
        visibility: inherited.visibility,
        outline_width: 0.0,
        outline_color: None,
        box_shadow: None,
        border_radius: 0.0,
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
    background_color: Option<TextColor>,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    font_family: FontFamily,
    text_decoration: TextDecoration,
    vertical_align: VerticalAlign,
    line_height: f32,
    /// Real CSS inheritance (see `Visibility`'s own docs), so this is
    /// read straight off `s.visibility` — already the correctly-cascaded
    /// value for wherever this span's text node actually sits, descendant
    /// overrides included — not a separate ambient-propagation parameter.
    visibility: Visibility,
}

impl From<InheritedStyle> for SpanStyle {
    fn from(s: InheritedStyle) -> Self {
        SpanStyle {
            color: s.color,
            background_color: None,
            font_size: s.font_size,
            font_weight: s.font_weight,
            font_style: s.font_style,
            font_family: s.font_family,
            text_decoration: TextDecoration::new(),
            vertical_align: VerticalAlign::default(),
            line_height: s.line_height.resolve(s.font_size),
            visibility: s.visibility,
        }
    }
}

enum BoxContent {
    Blocks(Vec<LayoutBox>),
    Inline(Vec<(String, SpanStyle)>),
    /// A table row's cells, laid out horizontally instead of stacked.
    /// Only ever appears on a box whose own `style.display` is
    /// `Display::TableRow` — rows themselves are ordinary `Blocks`
    /// children of the table, stacking vertically like any other block.
    TableRow(Vec<LayoutBox>),
}

pub struct LayoutBox {
    style: ComputedStyle,
    content: BoxContent,
}

fn build_box_for_element(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle) -> LayoutBox {
    let style = resolve_style(handle, rules, inherited);
    if style.display == Display::Table {
        return LayoutBox {
            style,
            content: BoxContent::Blocks(build_table_children(handle, rules, style.inherited())),
        };
    }
    let prefix = if element_tag(handle) == "li" {
        Some((list_marker(handle, style.list_style_type), style.inherited().into()))
    } else {
        None
    };
    LayoutBox {
        style,
        content: BoxContent::Blocks(build_children_with_prefix(handle, rules, style.inherited(), prefix)),
    }
}

/// Walks a `display: table` element's children for its rows (and any
/// directly-placed block content, e.g. a `caption`) — flattening through
/// `thead`/`tbody`/`tfoot`/`colgroup`/`col` (`Display::TableGroup`), which
/// carry no box of their own in this simplified table layout. A nested
/// `<table>` is not flattened into: it's not `TableRow`/`TableGroup`
/// display, so it falls to the `Block` arm and gets its own box via the
/// normal `build_box_for_element` call, which re-enters this same
/// table-building path independently for its own rows.
fn build_table_children(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    for child in handle.children.borrow().iter() {
        let NodeData::Element { .. } = &child.data else { continue };
        let tag = element_tag(child);
        if tag == "script" || tag == "style" {
            continue;
        }
        let style = resolve_style(child, rules, inherited);
        match style.display {
            Display::None => {}
            Display::TableRow => {
                let cells = build_table_row_cells(child, rules, style.inherited());
                result.push(LayoutBox { style, content: BoxContent::TableRow(cells) });
            }
            Display::TableGroup => {
                result.extend(build_table_children(child, rules, style.inherited()));
            }
            Display::Block => {
                // e.g. `caption`, or any other block-level content placed
                // directly under `<table>`.
                result.push(build_box_for_element(child, rules, inherited));
            }
            // A stray `table-cell`/inline/`table` outside a row isn't
            // real-world table markup — skip rather than guess at it.
            _ => {}
        }
    }
    result
}

/// A `tr`'s direct `td`/`th` children, each built as a normal box (its own
/// interior content — text, inline elements, nested blocks — all work
/// unchanged via the ordinary `build_box_for_element` path; only how its
/// *parent* lays it out, horizontally instead of vertically, differs).
fn build_table_row_cells(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle) -> Vec<LayoutBox> {
    let mut cells = Vec::new();
    for child in handle.children.borrow().iter() {
        let NodeData::Element { .. } = &child.data else { continue };
        let style = resolve_style(child, rules, inherited);
        if style.display == Display::TableCell {
            cells.push(build_box_for_element(child, rules, inherited));
        }
    }
    cells
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
    collect_children(
        handle,
        rules,
        inherited,
        None,
        TextDecoration::new(),
        VerticalAlign::default(),
        &mut pending,
        &mut result,
    );
    if !pending.is_empty() {
        result.push(LayoutBox {
            style: anonymous_style(inherited),
            content: BoxContent::Inline(pending),
        });
    }
    result
}

/// Background color and text-decoration, painted/drawn for a run's glyphs,
/// are not CSS-inherited properties (an element's own background covers
/// only its own box; a UA draws an ancestor's decoration through, but
/// `text-decoration-line` itself isn't an inherited property either) — but
/// for inline elements split across a text run, the nearest enclosing
/// element's value is still the right thing to carry onto its descendants'
/// glyphs unless a nested element sets its own — so both are threaded
/// alongside `inherited` rather than folded into it. `decoration` is a
/// concrete `TextDecoration` (not `Option`), since "no decoration" already
/// has a representable off state within the type itself — an element can
/// use that to explicitly turn off an ancestor's decoration by declaring
/// `text-decoration: none`.
fn collect_children(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    bg: Option<TextColor>,
    decoration: TextDecoration,
    valign: VerticalAlign,
    pending: &mut Vec<(String, SpanStyle)>,
    result: &mut Vec<LayoutBox>,
) {
    for child in handle.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => {
                let mut span: SpanStyle = inherited.into();
                span.background_color = bg;
                span.text_decoration = decoration;
                span.vertical_align = valign;
                // `white-space: pre` preserves the text byte-for-byte
                // (literal newlines, runs of spaces, no fabricated
                // trailing space) instead of collapsing it.
                if inherited.white_space == WhiteSpace::Pre {
                    pending.push((contents.borrow().to_string(), span));
                } else {
                    let mut text = normalize_whitespace(&contents.borrow());
                    text.push(' ');
                    pending.push((text, span));
                }
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
                if matches!(style.display, Display::Block | Display::Table) {
                    if !pending.is_empty() {
                        result.push(LayoutBox {
                            style: anonymous_style(inherited),
                            content: BoxContent::Inline(std::mem::take(pending)),
                        });
                    }
                    result.push(build_box_for_element(child, rules, inherited));
                } else {
                    let child_bg = style.background_color.or(bg);
                    let child_decoration = style.text_decoration.unwrap_or(decoration);
                    let child_valign = if style.vertical_align == VerticalAlign::Baseline {
                        valign
                    } else {
                        style.vertical_align
                    };
                    collect_children(
                        child,
                        rules,
                        style.inherited(),
                        child_bg,
                        child_decoration,
                        child_valign,
                        pending,
                        result,
                    );
                }
            }
            _ => {}
        }
    }
}

/// scratchy's built-in default stylesheet (see `src/default.css`) — stands
/// in for the user-agent stylesheet a real browser would otherwise supply.
/// Embedded at compile time rather than read from disk, so there's no
/// install-path/XDG-discovery problem for a single-binary tool.
const DEFAULT_CSS: &str = include_str!("default.css");

pub fn build_layout_tree(document: &Handle) -> LayoutBox {
    let mut css_source = String::new();
    collect_style_css(document, &mut css_source);
    let mut rules = parse_stylesheet(DEFAULT_CSS);
    rules.extend(parse_stylesheet(&css_source));
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
pub struct Laid {
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
    TableRow(Vec<Laid>),
}

/// A paragraph's shaped text, keyed by a stable id assigned during `measure`
/// in deterministic document order. Buffers live here — not inside `Laid` —
/// and persist across frames, so shaping a paragraph once is never repeated
/// (see `measure`'s budget/incomplete mechanism).
pub type ShapeCache = std::collections::HashMap<
    usize,
    (Buffer, f32, Vec<Option<TextColor>>, Vec<VerticalAlign>, Vec<bool>),
>;

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
pub fn new_font_system() -> FontSystem {
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

fn shape_spans(
    font_system: &mut FontSystem,
    spans: &[(String, SpanStyle)],
    width: f32,
    pre: bool,
    align: Option<CosmicAlign>,
) -> (Buffer, f32, Vec<Option<TextColor>>, Vec<VerticalAlign>, Vec<bool>) {
    let metrics = Metrics::new(16.0, 20.0);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(Some(width.max(0.0)), None);
    if pre {
        // `white-space: pre` also disables wrapping — content overflows
        // its box rather than breaking mid-line; literal `\n`s in the
        // text still start new lines regardless of wrap mode (cosmic-text
        // splits buffer lines on them either way).
        buffer.set_wrap(Wrap::None);
    }
    let rich: Vec<(&str, Attrs)> = spans
        .iter()
        .enumerate()
        .map(|(i, (text, span))| {
            let mut attrs = Attrs::new();
            attrs.color_opt = Some(span.color);
            attrs.weight = span.font_weight;
            attrs.style = span.font_style;
            attrs.family = span.font_family.to_cosmic();
            attrs.text_decoration = span.text_decoration;
            attrs.metadata = i;
            attrs = attrs.metrics(Metrics::new(span.font_size, span.line_height));
            (text.as_str(), attrs)
        })
        .collect();
    buffer.set_rich_text(rich, &Attrs::new(), Shaping::Advanced, align);
    buffer.shape_until_scroll(font_system, false);
    let mut height = 0.0f32;
    for run in buffer.layout_runs() {
        height = height.max(run.line_top + run.line_height);
    }
    let backgrounds = spans.iter().map(|(_, span)| span.background_color).collect();
    let valigns = spans.iter().map(|(_, span)| span.vertical_align).collect();
    let hidden = spans.iter().map(|(_, span)| span.visibility == Visibility::Hidden).collect();
    (buffer, height, backgrounds, valigns, hidden)
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
pub fn measure(
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
    let border_x = x + s.margin.left;
    let border_y = y + s.margin.top;
    let border_w = (width - s.margin.left - s.margin.right).max(0.0);
    let content_x = border_x + s.border_width + s.padding.left;
    let content_y = border_y + s.border_width + s.padding.top;
    let content_w = (border_w - 2.0 * s.border_width - s.padding.left - s.padding.right).max(0.0);

    match &b.content {
        BoxContent::Inline(spans) => {
            let id = *next_id;
            *next_id += 1;

            let content_h = if let Some((_, h, _, _, _)) = cache.get(&id) {
                *h
            } else if spans.is_empty() {
                cache.insert(
                    id,
                    (
                        Buffer::new_empty(Metrics::new(16.0, 20.0)),
                        0.0,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    ),
                );
                0.0
            } else if std::time::Instant::now() < deadline {
                let (buffer, h, backgrounds, valigns, hidden) = shape_spans(
                    font_system,
                    spans,
                    content_w,
                    s.white_space == WhiteSpace::Pre,
                    s.text_align.to_cosmic(),
                );
                cache.insert(id, (buffer, h, backgrounds, valigns, hidden));
                h
            } else {
                *incomplete = true;
                PLACEHOLDER_LINE_HEIGHT
            };

            let border_h = 2.0 * s.border_width + s.padding.top + s.padding.bottom + content_h;
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
                cursor_y += lc.style.margin.top + lc.style.margin.bottom + lc.h;
                laid_children.push(lc);
            }
            let content_h = cursor_y - content_y;
            let border_h = 2.0 * s.border_width + s.padding.top + s.padding.bottom + content_h;
            Laid {
                style: s,
                x: border_x,
                y: border_y,
                w: border_w,
                h: border_h,
                content: LaidContent::Blocks(laid_children),
            }
        }
        BoxContent::TableRow(cells) => {
            // Equal-width columns rather than real CSS auto table layout
            // (which sizes each column from its cells' min/max content
            // width) — a deliberate simplification, see TODO.txt.
            let n = cells.len().max(1) as f32;
            let cell_w = content_w / n;
            let mut cursor_x = content_x;
            let mut row_h = 0.0f32;
            let mut laid_cells = Vec::with_capacity(cells.len());
            for cell in cells {
                let lc = measure(cell, font_system, cursor_x, content_y, cell_w, next_id, cache, deadline, incomplete);
                row_h = row_h.max(lc.style.margin.top + lc.style.margin.bottom + lc.h);
                cursor_x += cell_w;
                laid_cells.push(lc);
            }
            let border_h = 2.0 * s.border_width + s.padding.top + s.padding.bottom + row_h;
            Laid {
                style: s,
                x: border_x,
                y: border_y,
                w: border_w,
                h: border_h,
                content: LaidContent::TableRow(laid_cells),
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

/// Fills a rect with its 4 corners rounded by `radius`, hard-edged (no
/// anti-aliasing — matches `fill_rect`'s own fidelity level, so this
/// isn't a visual downgrade from how every other shape here already
/// paints). Row by row: each row is one horizontal span, its full width
/// except within `radius` of the top/bottom edge, where it's inset by
/// how far that row sits from the nearest corner's circular boundary.
fn fill_rounded_rect(pixels: &mut [u32], win_w: u32, win_h: u32, x: f32, y: f32, w: f32, h: f32, radius: f32, color: TextColor) {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    if r <= 0.0 {
        fill_rect(pixels, win_w, win_h, x, y, w, h, color);
        return;
    }
    let y0 = y.round() as i32;
    let y1 = (y + h).round() as i32;
    for row in y0..y1 {
        let dy = (row as f32 - y).min((y + h) - row as f32 - 1.0);
        let inset = if dy < r {
            r - (r * r - (r - dy).max(0.0).powi(2)).max(0.0).sqrt()
        } else {
            0.0
        };
        fill_rect(pixels, win_w, win_h, x + inset, row as f32, (w - 2.0 * inset).max(0.0), 1.0, color);
    }
}

/// Paints one border edge's rect — same `(x, y, w, h)` rectangle
/// `fill_rect` itself takes (whichever of `w`/`h` is larger is the edge's
/// length *along* the box side; the smaller is its thickness *across*,
/// i.e. `border_width` — true for all 4 edges a `paint_box_decoration`
/// border call ever makes, so no separate orientation flag is needed).
/// `Solid` is one plain `fill_rect`, same as before this property
/// existed; `Dashed`/`Dotted`/`Double` instead tile smaller fills along
/// the edge's length. `None` never reaches here (`paint_box_decoration`
/// skips the whole border when it sees `BorderStyle::None`).
fn paint_border_edge(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    style: BorderStyle,
    color: TextColor,
) {
    let horizontal = w >= h;
    let (along, across) = if horizontal { (w, h) } else { (h, w) };
    match style {
        BorderStyle::None => {}
        BorderStyle::Solid => fill_rect(pixels, win_w, win_h, x, y, w, h, color),
        BorderStyle::Dashed => {
            let dash = (across * 3.0).max(2.0);
            let mut pos = 0.0;
            while pos < along {
                let seg = dash.min(along - pos);
                if horizontal {
                    fill_rect(pixels, win_w, win_h, x + pos, y, seg, across, color);
                } else {
                    fill_rect(pixels, win_w, win_h, x, y + pos, across, seg, color);
                }
                pos += dash * 2.0;
            }
        }
        BorderStyle::Dotted => {
            let dot = across.max(1.0);
            let mut pos = 0.0;
            while pos < along {
                let seg = dot.min(along - pos);
                if horizontal {
                    fill_rect(pixels, win_w, win_h, x + pos, y, seg, across, color);
                } else {
                    fill_rect(pixels, win_w, win_h, x, y + pos, across, seg, color);
                }
                pos += dot * 2.0;
            }
        }
        BorderStyle::Double => {
            let stripe = (across / 3.0).max(1.0);
            if horizontal {
                fill_rect(pixels, win_w, win_h, x, y, w, stripe, color);
                fill_rect(pixels, win_w, win_h, x, y + h - stripe, w, stripe, color);
            } else {
                fill_rect(pixels, win_w, win_h, x, y, stripe, h, color);
                fill_rect(pixels, win_w, win_h, x + w - stripe, y, stripe, h, color);
            }
        }
    }
}

fn paint_box_decoration(s: &ComputedStyle, pixels: &mut [u32], win_w: u32, win_h: u32, x: f32, y: f32, w: f32, h: f32) {
    // Painted first — behind this box's own background/border, same
    // stacking a real (non-inset) box-shadow has. A hard-edged solid
    // rect, not a blurred one (see `BoxShadow`'s own docs).
    if let Some(shadow) = s.box_shadow {
        fill_rect(
            pixels,
            win_w,
            win_h,
            x + shadow.offset_x - shadow.spread,
            y + shadow.offset_y - shadow.spread,
            w + 2.0 * shadow.spread,
            h + 2.0 * shadow.spread,
            shadow.color,
        );
    }
    if let Some(bg) = s.background_color {
        // Rounded only when there's no border to also round — see
        // `ComputedStyle::border_radius`'s own docs.
        if s.border_radius > 0.0 && s.border_width <= 0.0 {
            fill_rounded_rect(pixels, win_w, win_h, x, y, w, h, s.border_radius, bg);
        } else {
            fill_rect(pixels, win_w, win_h, x, y, w, h, bg);
        }
    }
    if s.border_width > 0.0 && s.border_style != BorderStyle::None {
        if let Some(bc) = s.border_color {
            let bw = s.border_width;
            paint_border_edge(pixels, win_w, win_h, x, y, w, bw, s.border_style, bc);
            paint_border_edge(pixels, win_w, win_h, x, y + h - bw, w, bw, s.border_style, bc);
            paint_border_edge(pixels, win_w, win_h, x, y, bw, h, s.border_style, bc);
            paint_border_edge(pixels, win_w, win_h, x + w - bw, y, bw, h, s.border_style, bc);
        }
    }
    // Outside the border box (not part of the box model — doesn't affect
    // `w`/`h` the way `border_width` does), same 4-rect ring shape as
    // border, just offset outward by its own width instead of inward.
    if s.outline_width > 0.0 {
        if let Some(oc) = s.outline_color {
            let ow = s.outline_width;
            let (ox, oy, ow_total, oh_total) = (x - ow, y - ow, w + 2.0 * ow, h + 2.0 * ow);
            fill_rect(pixels, win_w, win_h, ox, oy, ow_total, ow, oc);
            fill_rect(pixels, win_w, win_h, ox, oy + oh_total - ow, ow_total, ow, oc);
            fill_rect(pixels, win_w, win_h, ox, oy, ow, oh_total, oc);
            fill_rect(pixels, win_w, win_h, ox + ow_total - ow, oy, ow, oh_total, oc);
        }
    }
}

/// Paints each span's background (if any) as a rect behind its glyphs,
/// before the glyphs themselves — a span can wrap across multiple lines,
/// so this walks every line's glyphs rather than assuming one rect per
/// span, merging each line's consecutive same-span glyphs into one rect
/// (glyphs carry their originating span's index via cosmic-text's
/// per-`Attrs` `metadata` field, set in `shape_spans`).
fn paint_span_backgrounds(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    backgrounds: &[Option<TextColor>],
    hidden: &[bool],
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    ox: i32,
    oy: i32,
) {
    buffer.shape_until_scroll(font_system, false);
    for run in buffer.layout_runs() {
        let mut i = 0;
        while i < run.glyphs.len() {
            let glyph = &run.glyphs[i];
            let is_hidden = hidden.get(glyph.metadata).copied().unwrap_or(false);
            let bg = if is_hidden { None } else { backgrounds.get(glyph.metadata).copied().flatten() };
            let start_x = glyph.x;
            let mut end_x = glyph.x + glyph.w;
            let mut j = i + 1;
            while j < run.glyphs.len() && run.glyphs[j].metadata == glyph.metadata {
                end_x = run.glyphs[j].x + run.glyphs[j].w;
                j += 1;
            }
            if let Some(color) = bg {
                fill_rect(
                    pixels,
                    win_w,
                    win_h,
                    start_x + ox as f32,
                    run.line_top + oy as f32,
                    end_x - start_x,
                    run.line_height,
                    color,
                );
            }
            i = j;
        }
    }
}

fn paint_buffer(
    buffer: &mut Buffer,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    backgrounds: &[Option<TextColor>],
    valigns: &[VerticalAlign],
    // `visibility: hidden` per span (by the same `glyph.metadata` index as
    // `backgrounds`/`valigns`) — skips painting that span's background and
    // glyphs while its layout space (already reserved by `measure`, which
    // never looks at visibility) stays exactly as if it had painted.
    // Decoration lines (underline/strikethrough) aren't filtered by this —
    // `render_decoration` draws per-run, not per-span — a known, narrow
    // simplification: nothing in `default.css` pairs `text-decoration`
    // with `visibility: hidden` today.
    hidden: &[bool],
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    offset_x: f32,
    offset_y: f32,
) {
    let ox = offset_x.round() as i32;
    let oy = offset_y.round() as i32;
    paint_span_backgrounds(buffer, font_system, backgrounds, hidden, pixels, win_w, win_h, ox, oy);
    let default_color = TextColor::rgb(0, 0, 0);
    // `buffer.draw()`'s own internals, copied out by hand (both public:
    // `LegacyRenderer`, `render_decoration`) instead of called directly —
    // it offers no hook to nudge an individual glyph's y position, which
    // `vertical-align: sub`/`super` needs per span (looked up by
    // `glyph.metadata`, the same per-span index `paint_span_backgrounds`
    // already reads it by).
    buffer.shape_until_scroll(font_system, false);
    let mut renderer = LegacyRenderer {
        font_system,
        cache: swash_cache,
        callback: |x: i32, y: i32, w: u32, h: u32, color: TextColor| {
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
    };
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            if hidden.get(glyph.metadata).copied().unwrap_or(false) {
                continue;
            }
            let y_shift = valigns
                .get(glyph.metadata)
                .copied()
                .unwrap_or_default()
                .y_shift(glyph.font_size);
            let physical_glyph = glyph.physical((0., run.line_y + y_shift), 1.0);
            let glyph_color = glyph.color_opt.unwrap_or(default_color);
            renderer.glyph(physical_glyph, glyph_color);
        }
        render_decoration(&mut renderer, &run, default_color);
    }
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
    // `visibility: hidden` still reserves this box's layout space (laid
    // out exactly as normal above) but skips its own background/border —
    // a descendant box with its own `visibility: visible` is a separate
    // `Laid` with its own independently-resolved `style.visibility` via
    // real CSS inheritance, so it still recurses and paints normally.
    if s.visibility == Visibility::Visible {
        paint_box_decoration(&s, pixels, win_w, win_h, laid.x, draw_y, laid.w, laid.h);
    }

    match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            for child in children {
                paint(child, font_system, swash_cache, cache, pixels, win_w, win_h, scroll_offset);
            }
        }
        LaidContent::Text(id) => {
            // Not yet shaped (still mid-way through a budgeted measure pass)
            // just means nothing to paint here yet — it'll appear once a
            // later frame finishes shaping it.
            if let Some((buffer, _, backgrounds, valigns, hidden)) = cache.get_mut(id) {
                let content_x = laid.x + s.border_width + s.padding.left;
                let content_y = draw_y + s.border_width + s.padding.top;
                paint_buffer(
                    buffer, font_system, swash_cache, backgrounds, valigns, hidden, pixels, win_w, win_h,
                    content_x, content_y,
                );
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

/// The actual entry point — `src/main.rs` is just `fn main() {
/// scratchy::run(); }`. Split into a lib so `benches/` (a separate
/// compilation unit, unable to reach into a binary crate's `main.rs` at
/// all) can link against the engine directly, for real `cargo bench`
/// targets instead of hand-editing this file with temporary
/// `eprintln!`/`Instant` timing for every performance question (see
/// NOTES.md/TODO.txt — long agreed worth doing, not done until now).
pub fn run() {
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
