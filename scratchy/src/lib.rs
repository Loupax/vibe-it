use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::num::NonZeroU32;
use std::rc::{Rc, Weak};
use std::thread;

use cosmic_text::{
    Align as CosmicAlign, Attrs, Buffer, Color as TextColor, Family as CosmicFamily, FontSystem,
    LegacyRenderer, Metrics, Renderer, Shaping, Style as FontStyle, SwashCache,
    TextDecoration, UnderlineStyle, Weight as FontWeight, Wrap,
};
use cssparser::{
    match_ignore_ascii_case, AtRuleParser, CowRcStr, DeclarationParser, ParseError,
    Parser as CssParser, ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
    StyleSheetParser, Token,
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
use winit::window::{CursorIcon, Window, WindowId};

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

/// `:hover`/`:focus`/`:active` still can't parse (there's no mouse-button
/// or keyboard-focus state to drive them — only `CursorMoved` exists, and
/// only for the `cursor` property, not selector matching) — only the two
/// link-state pseudo-classes, which need no interactivity at all: every
/// hyperlink scratchy ever renders is honestly "unvisited" (no navigation,
/// no history), so `:link` can match structurally (any `a`/`area` with an
/// `href`) and `:visited` can just always decline to match, same as a real
/// browser would render a link it has no visit record for.
/// `:root` (v45) just delegates to the `selectors` crate's own
/// `Element::is_root()` (already implemented for `El` for structural
/// matching generally) — same element `html` matches, but with `:root`'s
/// higher specificity. Doubles as the natural place real-world pages
/// declare CSS custom properties (`:root { --accent: ... }`), which is
/// exactly what `substitute_css_vars` looks for.
#[derive(Clone, Debug, Eq, PartialEq)]
enum OurPseudoClass {
    Link,
    Visited,
    Root,
}

impl cssparser::ToCss for OurPseudoClass {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        dest.write_str(match self {
            OurPseudoClass::Link => ":link",
            OurPseudoClass::Visited => ":visited",
            OurPseudoClass::Root => ":root",
        })
    }
}

impl NonTSPseudoClass for OurPseudoClass {
    fn is_active_or_hover(&self) -> bool {
        false
    }
    fn is_user_action_state(&self) -> bool {
        false
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
    type NonTSPseudoClass = OurPseudoClass;
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

    fn parse_non_ts_pseudo_class(
        &self,
        name: CowRcStr<'i>,
    ) -> Result<OurPseudoClass, ParseError<CssError>> {
        match_ignore_ascii_case! { &name,
            "link" => return Ok(OurPseudoClass::Link),
            "visited" => return Ok(OurPseudoClass::Visited),
            "root" => return Ok(OurPseudoClass::Root),
            _ => {}
        }
        Err(ParseError::custom(SelectorParseErrorKind::UnsupportedPseudoClassOrElement))
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
        pc: &OurPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match pc {
            // Every hyperlink scratchy renders is honestly "unvisited" —
            // see `OurPseudoClass`'s own docs.
            OurPseudoClass::Link => self.is_link(),
            OurPseudoClass::Visited => false,
            OurPseudoClass::Root => self.is_root(),
        }
    }

    fn match_pseudo_element(
        &self,
        pe: &NoPseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        match *pe {}
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    /// A hyperlink *source anchor*, in the CSS selectors sense (drives
    /// `:any-link` and, via `match_non_ts_pseudo_class`, `:link`) — an
    /// `a`/`area` with an `href`, not just any `<a>` (an anchor with no
    /// `href` is a plain fragment target, not a link, same as real
    /// browsers treat it).
    fn is_link(&self) -> bool {
        matches!(element_tag(&self.0), "a" | "area") && element_attr(&self.0, "href").is_some()
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
    /// content at all — real per-column sizing (`measure_table`) still
    /// derives every column's width from its cells' own content, not
    /// from a `<col width>`/CSS `width` hint on a `col` element, which
    /// isn't read — so walking through them finds nothing and they
    /// contribute no box either way.
    TableGroup,
    /// `display: flex` — a block-level container (in the outer flow,
    /// same as `Table`) whose direct children each become flex items,
    /// laid out per CSS Flexible Box Layout (`measure_flex`) instead of
    /// normal block/inline flow. `inline-flex` is not implemented (see
    /// TODO.txt) — a flex container is always block-level here.
    Flex,
}

/// `position`'s 5 implemented keywords. Not inherited (same category as
/// `border_width`/`overflow`/`cursor` — every element defaults to
/// `Static` regardless of what an ancestor declared).
///
/// `Static` is one of two in-flow values: `Relative`/`Absolute`/`Fixed`
/// all establish this box as a CSS "positioned element" — the real
/// spec term for the one thing all three share (an `Absolute`/`Fixed`
/// *descendant*'s containing block is the nearest *positioned*
/// ancestor, i.e. any of these three, not `Static`; see `measure`'s own
/// docs on `PendingPositioned`/`catch_or_bubble_positioned`). `Relative`
/// is also the only one that stays in normal flow — see `measure`'s own
/// `apply_relative_offset`.
///
/// `Sticky` is the other in-flow value, and deliberately resolved in two
/// separate stages, unlike every other variant here: `Relative`/
/// `Absolute`/`Fixed` all resolve fully at layout time and stay fixed
/// until the next relayout, but a sticky box's real final position
/// depends on the *live* scroll offset, which doesn't exist yet when
/// `measure` runs and changes every scroll tick — and scrolling in this
/// engine is deliberately cheap precisely because it never triggers a
/// relayout (see NOTES.md's v4/v5 writeup). So `measure` only resolves
/// what *doesn't* change per frame — this box's own would-be-static
/// position (exactly like `Relative`, it never leaves flow) and its own
/// containing block's bounds (`capture_sticky_info`/`patch_sticky_
/// containing_block`, stored in `Laid::sticky`) — and the actual
/// *painted* position is computed fresh every frame, from the ambient
/// scroll offset, in `paint`/`hit_test_cursor` (`sticky_shift`) — no
/// relayout, ever, from a scroll-offset change alone. Doesn't force
/// computed `display: block` the way `Absolute`/`Fixed` do (real CSS
/// §9.7 only names those two) — an inline `position: sticky` element
/// stays flattened into its surrounding run with no box of its own to
/// stick, the same pre-existing gap `Relative` already has on inline
/// content (not attempted here either, see TODO.txt).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

/// `float`'s 2 implemented keywords (plus `none`, the default — not a
/// variant here, same shape as `Position`'s own `Static`). Not inherited
/// (same category as `position` — every element defaults to `None`
/// regardless of what an ancestor declared). A floated box is taken out
/// of normal block-level flow (it doesn't advance its parent's own
/// `cursor_y` the way a normal sibling does — see `measure`'s own
/// `BoxContent::Blocks` arm) but, unlike `position: absolute`/`fixed`,
/// it still participates in the surrounding *inline* formatting context:
/// text in a later sibling genuinely wraps around it rather than
/// ignoring it entirely (see `Floats`'s own docs). Real CSS's own
/// "computed `display`" rule (spec §9.7) applies here too, same as
/// `position: absolute`/`fixed` — `resolve_style` forces `display:
/// block` whenever `float` isn't `None` (unless `display` is already
/// `none`), so e.g. a `<span style="float: left">` gets its own box
/// instead of being flattened into the surrounding inline run.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum Float {
    #[default]
    None,
    Left,
    Right,
}

/// `clear`'s 4 implemented keywords. Not inherited. Only ever consulted
/// for an ordinary in-flow block-level child (`measure`'s `BoxContent::
/// Blocks` arm checks it once per child, before that child is measured,
/// including a child that's itself floated — real CSS lets a float also
/// clear) — pushes the flow position down past the bottom of any
/// earlier float(s) *on the given side(s)* in the same block formatting
/// context (CSS2.1 §9.5.2), via `Floats::clear_y`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum Clear {
    #[default]
    None,
    Left,
    Right,
    Both,
}

/// `flex-direction`'s 4 keywords. `Row`/`RowReverse` make the *main*
/// axis horizontal (width) and the *cross* axis vertical (height);
/// `Column`/`ColumnReverse` swap them. The `*Reverse` variants don't
/// change document/paint order (`order` isn't implemented anyway — see
/// TODO.txt) — only *where* each item lands along the main axis
/// (`measure_flex` lays items out normally, then mirrors their main-axis
/// position within the container's main-axis extent for the reverse
/// variants, rather than reordering anything).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

impl FlexDirection {
    fn is_row(self) -> bool {
        matches!(self, FlexDirection::Row | FlexDirection::RowReverse)
    }

    fn is_reverse(self) -> bool {
        matches!(self, FlexDirection::RowReverse | FlexDirection::ColumnReverse)
    }
}

/// `flex-wrap`'s 3 keywords. Only genuinely implemented for `flex-
/// direction: row`/`row-reverse` (see `measure_flex`'s own docs) — a
/// `column`/`column-reverse` flex container always lays its items out
/// as a single column regardless of this value, a documented scope cut
/// (TODO.txt), since wrapping a *column* means wrapping across the
/// *width* into multiple side-by-side columns, a materially different
/// (and much rarer in real pages) shape than the classic "wrapped
/// multi-line row" this item prioritizes.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

/// `justify-content`'s keywords, main-axis distribution. `SpaceEvenly`
/// is CSS Box Alignment's newer addition (not in the original Flexbox
/// spec) — implemented anyway since it fell out of the same `leftover`/
/// `n` arithmetic as `SpaceBetween`/`SpaceAround` for free, despite the
/// task's own wording treating it as optional.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// `align-items`/`align-self`'s keywords, cross-axis alignment.
/// `Baseline` is accepted but resolves identically to `FlexStart` — real
/// baseline alignment needs each item's own font ascent threaded out of
/// `measure`'s box-model math (nothing here tracks a box's baseline
/// today, only cosmic-text's per-glyph shaping does, several layers
/// below `measure`), a disproportionate amount of new plumbing for a
/// value most real "flex row of items" pages don't actually rely on —
/// explicitly permitted as an approximation and documented (TODO.txt),
/// not silently dropped.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum AlignItems {
    #[default]
    Stretch,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
}

/// `flex-basis`'s value: either `auto` (the real initial value — fall
/// back to the item's own `width`/`height` on the main axis, or its
/// content size if that's also auto) or an explicit length/percentage.
/// Deferred the same way `width`/`margin`/etc. are (`Length`, not a
/// resolved `BoxLength`) — resolved against the flex container's own
/// main-axis content size, which `resolve_style` can't know at cascade
/// time; see `measure_flex`'s own docs for where that happens.
#[derive(Clone, Copy, Default)]
enum FlexBasis {
    #[default]
    Auto,
    Len(Length),
}

/// `top`/`right`/`bottom`/`left` ("inset" is this file's own shorthand
/// name for the group, following the real CSS logical-properties spec
/// term — there's no actual `inset` shorthand *property* parsed here,
/// same scope as `margin`'s four longhands before a shorthand existed).
/// `None` means `auto` (the initial value *and* "not declared" collapse
/// to the same thing, same simplification `width`/`height`'s own
/// `Option<BoxLength>` already accepts — see `set_length`'s own docs:
/// an explicit `top: auto` declaration can't currently re-override a
/// lower-specificity `top: 10px` rule back to auto, a pre-existing
/// limitation of this file's `set_length` convention, not a new one
/// introduced for this property). `BoxLength`, not a plain `f32`: `top`/
/// `bottom`'s `%` resolves against the containing block's *height*,
/// `left`/`right`'s against its *width* — both only known once a real
/// positioned ancestor (or the viewport, for `Fixed`/an orphaned
/// `Absolute`) is actually found, in `resolve_positioned`, same
/// deferred-until-a-real-basis-exists timing `BoxLength` already uses
/// for `width`/`margin`/etc.
#[derive(Clone, Copy, Default)]
struct Insets {
    top: Option<BoxLength>,
    right: Option<BoxLength>,
    bottom: Option<BoxLength>,
    left: Option<BoxLength>,
}

/// A parsed-but-not-yet-resolved length: `px` is an absolute value, `em` is
/// relative to a font-size that isn't known until cascade time (an
/// element's own computed font-size, for margin/padding/border-width; its
/// *parent's* computed font-size, for font-size itself — see
/// `resolve_style`), `rem` is relative to the document root's font-size
/// (see `root_font_size`'s own docs below).
///
/// `Percent` is the newest, and the odd one out: unlike every other
/// variant, *which* basis it resolves against isn't fixed by `Length`
/// itself — it depends on which property declared it. For `font-size`
/// and `line-height` (whose real CSS percentage basis is a font-size,
/// just like `Em`), `resolve`'s own `Percent` arm below handles it
/// directly, same timing as `Em`. For every box-dimension property
/// (`width`/`height` and their `min-`/`max-` forms, `margin`/`padding`,
/// `border-radius`) the basis is a *containing-block dimension* —
/// something `resolve_style`'s cascade can't know yet (it runs before
/// layout) — so those properties never call `Length::resolve` on a
/// `Percent` at all: `BoxLength::from_length` (below) diverts it,
/// unresolved, into `ComputedStyle`, where it's finally resolved against
/// the real basis once one exists — see `BoxLength`'s and `measure`'s own
/// docs. Stored as the raw fraction (`0.5` for `50%`, matching
/// `cssparser`'s own `Token::Percentage::unit_value`), not a `0..100`
/// percentage number.
///
/// `Larger`/`Smaller` are `font-size`'s own two relative keywords —
/// deliberately not reachable through the general `parse_length`/
/// `parse_length_percent`/`set_length` used by every other length-ish
/// property (`margin: larger` isn't meaningful), only through
/// `font-size`'s own dedicated parse branch — but resolve through the
/// exact same `resolve_style` code path as `Em` already does for
/// `font-size` (against the *parent's* computed font-size), so no change
/// needed there. `1.2` isn't a spec'd exact value (the real spec steps
/// through an absolute-size keyword table) — just the common
/// approximation browsers converge on, and the same ratio `default.css`'s
/// own heading sizes roughly follow.
#[derive(Clone, Copy)]
enum Length {
    Px(f32),
    Em(f32),
    /// Relative to the *document root's* computed font-size, not the
    /// declaring element's own — unlike every other variant here,
    /// resolving it needs a second font-size figure distinct from the
    /// `font_size: f32` every other `resolve` call already threads
    /// through `resolve_style`. Carried as `InheritedStyle::
    /// root_font_size`, set once at `ROOT_STYLE` and copied unchanged by
    /// every other element's `inherited()`/`anonymous_style` (it's a
    /// document-wide constant, not something that cascades per element).
    Rem(f32),
    /// The raw fraction (see this enum's own doc comment above for why
    /// `Length::resolve` only sometimes handles this variant itself).
    Percent(f32),
    Larger,
    Smaller,
}

impl Length {
    /// Only a valid call for `Percent` when `font_size` really is this
    /// value's intended basis (`font-size`'s own percentage, relative to
    /// the *parent's* font-size — the same `font_size` parameter `Em`
    /// already uses there — or `line-height`'s, relative to the
    /// *element's own* font-size, same timing `Em` already uses there
    /// too). Every box-dimension property instead calls
    /// `BoxLength::from_length`, which diverts `Percent` away before it
    /// ever reaches here — see this enum's own docs.
    fn resolve(self, font_size: f32, root_font_size: f32) -> f32 {
        match self {
            Length::Px(v) => v,
            Length::Em(v) => v * font_size,
            Length::Rem(v) => v * root_font_size,
            Length::Percent(p) => font_size * p,
            Length::Larger => font_size * 1.2,
            Length::Smaller => font_size / 1.2,
        }
    }
}

/// `width`/`height` (and their `min-`/`max-` forms), `margin`/`padding`
/// (all four physical sides), and `border-radius` all share this
/// representation in `ComputedStyle` instead of a plain `f32`, because
/// real CSS `%` on any of them resolves against a *containing-block*
/// dimension — something that isn't known at cascade time
/// (`resolve_style` runs once per element, independent of layout) and is
/// only known once layout actually reaches that box (`measure`, which
/// already receives the available width, and — since v49 — the
/// containing block's own definite height, as plain parameters). `Px`
/// covers every other length form (`px`/`em`/`rem`, all resolved eagerly
/// in `resolve_style`, since none of them need a layout-time basis) —
/// the name is about *when* this is finalized, not literally "pixels
/// from a `px` token." `Percent` is the raw fraction (`0.5` for `50%`,
/// matching `Length::Percent`), deferred until `measure` (via
/// `finalize_box_lengths`) has a real basis to resolve it against.
///
/// By the time a `ComputedStyle` is stored in a `Laid` (i.e. anywhere
/// `measure` has already run on it), every field of this type is
/// guaranteed `Px` — `measure` always finalizes a box's own lengths
/// before returning — which is what makes `px()` (below) safe to call
/// from `paint`/`hit_test_cursor`/the children-stacking code in `measure`
/// itself, none of which have a containing-block basis of their own to
/// resolve a `Percent` against.
#[derive(Clone, Copy)]
enum BoxLength {
    Px(f32),
    Percent(f32),
}

impl Default for BoxLength {
    fn default() -> Self {
        BoxLength::Px(0.0)
    }
}

impl BoxLength {
    /// Converts a cascade-winning `Length` into a `BoxLength`: `Percent`
    /// is carried through unresolved (its real basis is a containing-
    /// block dimension, not available yet — see this type's own docs);
    /// every other variant resolves immediately via `Length::resolve`,
    /// exactly as it always has (the `font_size`/`root_font_size` basis
    /// those need *is* available at cascade time).
    fn from_length(l: Length, font_size: f32, root_font_size: f32) -> Self {
        match l {
            Length::Percent(p) => BoxLength::Percent(p),
            other => BoxLength::Px(other.resolve(font_size, root_font_size)),
        }
    }

    /// Resolves against `basis` — the containing block's width, for
    /// every property except `height`/`min-height`/`max-height` (whose
    /// basis is the containing block's own definite *height*, when it
    /// has one — see `measure`'s `containing_height` parameter) and
    /// `border-radius` (whose basis is this box's *own* border-box
    /// width, per real CSS — see `paint_box_decoration`). `Px` ignores
    /// `basis` entirely, already final.
    fn resolve(self, basis: f32) -> f32 {
        match self {
            BoxLength::Px(v) => v,
            BoxLength::Percent(p) => basis * p,
        }
    }

    /// Reads an already-finalized value with no basis needed — see this
    /// type's own docs for the invariant that makes this safe wherever
    /// it's called. Debug-asserts (rather than silently resolving
    /// against a bogus implicit basis of `0.0`) if that invariant is
    /// ever violated, which would otherwise be a confusing, silent
    /// wrong-number-not-a-crash bug.
    fn px(self) -> f32 {
        match self {
            BoxLength::Px(v) => v,
            BoxLength::Percent(p) => {
                debug_assert!(false, "BoxLength::px() called on an unresolved {p} (fraction) percent");
                0.0
            }
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
    fn resolve(self, font_size: f32, root_font_size: f32) -> f32 {
        match self {
            LineHeight::Normal => font_size * 1.25,
            LineHeight::Multiplier(m) => font_size * m,
            LineHeight::Len(l) => l.resolve(font_size, root_font_size),
        }
    }
}

#[derive(Clone, Default)]
struct Declarations {
    /// `--name: <raw text>` custom-property declarations on this rule,
    /// in source order, name kept exactly as written (custom property
    /// names are case-*sensitive*, unlike every other property this file
    /// matches via `eq_ignore_ascii_case`). The raw value text is kept
    /// unparsed/unresolved here — `var()` substitution happens as a
    /// whole-stylesheet text pass in `substitute_css_vars`, not here; see
    /// its own docs for why and for the "`:root`/`html`-scoped only, no
    /// per-element override chain" simplification this implies.
    custom_props: Vec<(String, String)>,
    color: Option<CssColor>,
    background_color: Option<CssColor>,
    /// The raw `url(...)` argument, unresolved — decoding (a `data:`
    /// URI only, same `<img src>` policy) happens once per matching
    /// element in `resolve_background_image`, not here, since a
    /// decoded image can't live in `ComputedStyle` (it's `Copy`; see
    /// `resolve_background_image`'s own docs). `background-image: none`
    /// isn't recognized as its own value (parsing simply fails and
    /// leaves this `None`, same end state) — not a real gap today since
    /// nothing declares it, but see TODO.txt.
    background_image: Option<String>,
    display: Option<Display>,
    position: Option<Position>,
    float: Option<Float>,
    clear: Option<Clear>,
    top: Option<Length>,
    right: Option<Length>,
    bottom: Option<Length>,
    left: Option<Length>,
    z_index: Option<i32>,
    width: Option<Length>,
    height: Option<Length>,
    min_width: Option<Length>,
    max_width: Option<MaxLen>,
    min_height: Option<Length>,
    max_height: Option<MaxLen>,
    box_sizing: Option<BoxSizing>,
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
    font_weight: Option<FontWeightValue>,
    font_style: Option<FontStyle>,
    font_family: Option<FontFamily>,
    text_decoration: Option<DecorationValue>,
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
    overflow: Option<Overflow>,
    cursor: Option<CursorStyle>,
    flex_direction: Option<FlexDirection>,
    flex_wrap: Option<FlexWrap>,
    justify_content: Option<JustifyContent>,
    align_items: Option<AlignItems>,
    /// `None` here means "not declared" *and* the real `auto` keyword
    /// (an item's `align-self: auto` falls back to its container's own
    /// `align-items` — see `ComputedStyle::align_self`'s own docs) —
    /// same "both collapse to the same thing" simplification `z_index`/
    /// `top`/etc. already use for their own `auto` keyword.
    align_self: Option<AlignItems>,
    flex_grow: Option<f32>,
    flex_shrink: Option<f32>,
    flex_basis: Option<FlexBasis>,
    row_gap: Option<Length>,
    column_gap: Option<Length>,
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

/// Just `visible`/`hidden` — no `scroll`/`auto` (no scrollbar UI, no
/// scroll-region interactivity, just clipping). Not inherited. `Hidden`
/// clips all of a descendant's painting — text (`paint`'s `clip`
/// parameter, read by `paint_buffer`/`paint_span_backgrounds`) *and*
/// box decoration (background/border/outline/shadow, via `fill_rect`
/// itself applying `clip`, so every caller gets it for free) — but
/// never a box's *own* decoration against its *own* `overflow`, only a
/// descendant's (see `paint`'s own docs on `child_clip` vs. the ambient
/// `clip` it passes to `paint_box_decoration`), same as real CSS: an
/// element's `overflow` clips its content, not itself.
#[derive(Clone, Copy, PartialEq, Default)]
enum Overflow {
    #[default]
    Visible,
    Hidden,
}

/// `width`/`height`/`min-width`/`min-height` all resolve against the same
/// reference box `box-sizing` names — `ContentBox` (the default) excludes
/// padding/border, `BorderBox` includes them. Only these two real
/// keywords; `padding-box` isn't a valid CSS value. Not inherited (same
/// as `border_width`/`overflow`).
#[derive(Clone, Copy, PartialEq, Default)]
enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

/// `max-width`/`max-height`'s own grammar needs a value distinct from
/// "not declared" to represent `none` (the property's real initial
/// value — unconstrained) winning over an earlier, more specific rule's
/// actual length — unlike `min-width`/`min-height`, which have no such
/// keyword and reuse `Length` directly (their initial value, `0`, is
/// already what an absent declaration resolves to).
#[derive(Clone, Copy)]
enum MaxLen {
    None,
    Len(Length),
}

impl MaxLen {
    /// `None` here means "no constraint" (both the real initial value
    /// and an explicit `none`) — the same `Option<BoxLength>` shape
    /// `ComputedStyle::max_width`/`max_height` carry, so this is the
    /// only place that needs to know `MaxLen` exists at all. `Len`'s
    /// `Length` goes through `BoxLength::from_length`, same as every
    /// other box-dimension property — a `%` stays unresolved here too,
    /// finalized later in `measure`.
    fn resolve(self, font_size: f32, root_font_size: f32) -> Option<BoxLength> {
        match self {
            MaxLen::None => None,
            MaxLen::Len(l) => Some(BoxLength::from_length(l, font_size, root_font_size)),
        }
    }
}

/// Just `auto`/`pointer` — the only two keywords `default.css` needs
/// (`a { cursor: pointer }`; everything else defaults to `auto`). Real
/// CSS inheritance (`Auto` is the real spec initial value too), unlike
/// `vertical-align` — a `<span>` nested inside an `<a>` should show the
/// pointer cursor too without re-declaring it, which real inheritance
/// gives for free. Threaded per-span (`SpanStyle`/`shape_spans`'s
/// `Vec<CursorStyle>`, same pattern as `vertical_align`/`visibility`),
/// since the common real use (`<a>`) is inline content inside a block,
/// not a block-level box of its own — hit-testing needs to know which
/// *glyph* the mouse is over, not just which block.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum CursorStyle {
    #[default]
    Auto,
    Pointer,
}

impl CursorStyle {
    fn to_winit(self) -> CursorIcon {
        match self {
            CursorStyle::Auto => CursorIcon::Default,
            CursorStyle::Pointer => CursorIcon::Pointer,
        }
    }
}

/// One entry in a `font-family` fallback list — either a specific named
/// family (`"Georgia"`, `Helvetica Neue`) or one of the 5 CSS generic
/// keywords. `Name` holds a leaked `&'static str` — see `FontFamily`'s
/// own docs for why.
#[derive(Clone, Copy, PartialEq, Debug)]
enum FontFamilyItem {
    Name(&'static str),
    SansSerif,
    Serif,
    Monospace,
    Cursive,
    Fantasy,
}

impl FontFamilyItem {
    fn to_cosmic(self) -> CosmicFamily<'static> {
        match self {
            FontFamilyItem::Name(name) => CosmicFamily::Name(name),
            FontFamilyItem::SansSerif => CosmicFamily::SansSerif,
            FontFamilyItem::Serif => CosmicFamily::Serif,
            FontFamilyItem::Monospace => CosmicFamily::Monospace,
            FontFamilyItem::Cursive => CosmicFamily::Cursive,
            FontFamilyItem::Fantasy => CosmicFamily::Fantasy,
        }
    }
}

/// A full `font-family` value: an ordered fallback list of one or more
/// `FontFamilyItem`s — a real CSS comma-separated family list, e.g.
/// `"Georgia", Helvetica Neue, serif`, not just PROMPTS.md v16's original
/// bare-generic-keyword-only scope.
///
/// `&'static [FontFamilyItem]`, not an owned `Vec`/`Box<[_]>`:
/// `ComputedStyle`/`InheritedStyle`/`SpanStyle` are all
/// `#[derive(Clone, Copy)]` by design, threaded *by value* through every
/// recursive tree-building function (see NOTES.md v34's note on this
/// exact tradeoff, flagged there and deliberately deferred until now) —
/// a `Vec` field would force dropping `Copy` from all three, touching a
/// large fraction of this file for a property most pages never declare
/// at all. Each distinct parsed list (and each of its `Name` items'
/// strings) is instead leaked once, in `DeclParser`'s `font-family`
/// handling (`Box::leak`), into a `&'static` slice/str — itself
/// trivially `Copy` regardless of what it points to, so every struct
/// that already carried a plain `FontFamily` field keeps working
/// unchanged. The cost: memory for every *distinct* `font-family`
/// declaration's parsed list is never freed for the life of the
/// process. Accepted the same way this project already accepts
/// re-parsing the entire stylesheet from scratch on every relayout (no
/// cross-redraw caching at all yet) — bounded by how many distinct
/// `font-family` declarations exist in a document's CSS source
/// (typically a handful), not by how many elements reference them or
/// how many redraws happen, and dwarfed by the full-document reparse
/// work already redone on every one of those redraws. Worth revisiting
/// only if a page with many distinct `font-family` values redrawn many
/// times is ever shown to matter in practice.
#[derive(Clone, Copy, PartialEq)]
struct FontFamily(&'static [FontFamilyItem]);

impl Default for FontFamily {
    fn default() -> Self {
        // A plain array-literal reference to `Copy`, `Drop`-free data is
        // eligible for `'static` promotion — no leak needed for this,
        // the single most common case (no `font-family` declared at
        // all), matching cosmic-text's own `Attrs::new()` default.
        FontFamily(&[FontFamilyItem::SansSerif])
    }
}

impl FontFamily {
    /// Resolves this fallback list against the real font system,
    /// returning the single `cosmic_text::Family` to actually shape
    /// with. cosmic-text's own `Attrs` has no fallback-list concept —
    /// `Attrs::family` is one `Family`, not a slice — so a list can't be
    /// handed to it directly.
    ///
    /// Confirmed by reading `fontdb::Database::query`'s own source
    /// (vendored, `fontdb-0.23.0/src/lib.rs`): it already loops a
    /// `Query`'s `families` slice in order and returns the first one
    /// with any installed face, i.e. real fallback-list semantics live
    /// in `fontdb` already — it does **not** error or silently pick an
    /// arbitrary font for an unavailable name. The catch is that it
    /// reports back a matched font *ID*, not which *family name* in the
    /// list produced it — information `Attrs::family` itself needs. So
    /// rather than one `Query` naming every candidate at once, this
    /// queries one candidate at a time (via `std::slice::from_ref`) and
    /// stops at the first that resolves: observably the same in-order
    /// fallback behavior, while letting each step hand back its own
    /// `Family` directly. If nothing in the list resolves at all (every
    /// named family missing *and* even the trailing generic has no
    /// installed font), falls back to cosmic-text's own built-in
    /// default rather than shaping with an unmatched `Family`.
    fn resolve(self, db: &fontdb::Database, weight: FontWeight, style: FontStyle) -> CosmicFamily<'static> {
        for item in self.0.iter().copied() {
            let family = item.to_cosmic();
            let query = fontdb::Query {
                families: std::slice::from_ref(&family),
                weight,
                style,
                ..Default::default()
            };
            if db.query(&query).is_some() {
                return family;
            }
        }
        CosmicFamily::SansSerif
    }
}

struct StyleRule {
    selectors: SelectorList<OurSelectorImpl>,
    decls: Declarations,
}

/// `<length>` only — no `<percentage>` — the real CSS grammar for
/// `border-width`/`outline-width`/`box-shadow`'s offsets and radii, none
/// of which the spec allows a percentage on at all (unlike
/// `parse_length_percent`, below, used by every property whose real
/// grammar is `<length-percentage>`). A bare unitless number is only
/// ever valid here as the literal `0` — real CSS doesn't allow an
/// unparenthesized unitless non-zero number for any length-only
/// property like these (unlike `line-height`'s own dedicated grammar in
/// `parse_line_height`, where a bare number is the property's single
/// most common real-world form).
fn parse_length(input: &mut CssParser) -> Result<Length, ()> {
    match input.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => Ok(Length::Px(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("em") => Ok(Length::Em(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("rem") => Ok(Length::Rem(*value)),
        Token::Number { value, .. } if *value == 0.0 => Ok(Length::Px(0.0)),
        _ => Err(()),
    }
}

/// Same grammar as `parse_length` plus a `<percentage>` token — used by
/// every property whose real CSS grammar is `<length-percentage>`
/// rather than a plain `<length>`: `width`/`height` and their `min-`/
/// `max-` forms, `margin`/`padding` (and all their shorthand/logical
/// forms), `border-radius`, and `font-size` (`parse_length_percent`
/// itself has no font-size-vs-box-dimension opinion — that distinction
/// is `BoxLength::from_length`'s/`Length::resolve`'s job, at the point
/// each property's winning value is actually resolved). `unit_value` is
/// already the raw fraction (`cssparser`'s own convention — `0.5` for
/// `50%`), matching `Length::Percent`'s own documented representation.
fn parse_length_percent(input: &mut CssParser) -> Result<Length, ()> {
    match input.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => Ok(Length::Px(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("em") => Ok(Length::Em(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("rem") => Ok(Length::Rem(*value)),
        Token::Percentage { unit_value, .. } => Ok(Length::Percent(*unit_value)),
        Token::Number { value, .. } if *value == 0.0 => Ok(Length::Px(0.0)),
        _ => Err(()),
    }
}

/// Parses a length into `*target`, leaving it untouched (so an earlier
/// declaration in the cascade isn't clobbered) on anything unparseable —
/// `auto`/etc. still fall through this way, same as any other
/// not-yet-supported value. Used only by properties whose real grammar
/// is `<length-percentage>` (see `parse_length_percent`'s own docs) —
/// `border-width`/`outline-width` parse `parse_length` directly instead,
/// since real CSS doesn't allow a percentage on either.
fn set_length(input: &mut CssParser, target: &mut Option<Length>) {
    if let Ok(v) = parse_length_percent(input) {
        *target = Some(v);
    }
}

/// `max-width`/`max-height`'s own grammar: `none` (the real initial
/// value) alongside the usual `px`/`em`/`rem`/`%`/unitless-zero lengths —
/// same unparseable-leaves-target-untouched philosophy as `set_length`.
fn set_max_length(input: &mut CssParser, target: &mut Option<MaxLen>) {
    let state = input.state();
    let none_keyword = input.next().ok().and_then(|t| match t {
        Token::Ident(ident) if ident.eq_ignore_ascii_case("none") => Some(()),
        _ => None,
    });
    if none_keyword.is_some() {
        *target = Some(MaxLen::None);
        return;
    }
    input.reset(&state);
    if let Ok(v) = parse_length_percent(input) {
        *target = Some(MaxLen::Len(v));
    }
}

/// `z-index`'s own grammar: a plain `<integer>` — no `<length>`/`%`
/// forms at all, unlike every other property this file parses with
/// `set_length`. `auto` (the real initial value) is deliberately *not*
/// matched here — same `set_length`-wide "unparseable leaves the
/// cascade's existing value untouched" convention (see its own docs);
/// `ComputedStyle::z_index`'s `None` already covers both "never
/// declared" and "declared `auto`" identically, so there's nothing a
/// matched `auto` arm would do differently from just failing to parse.
fn parse_z_index(input: &mut CssParser) -> Result<i32, ()> {
    match input.next().map_err(|_| ())? {
        Token::Number { int_value: Some(v), .. } => Ok(*v),
        Token::Number { value, .. } => Ok(*value as i32),
        _ => Err(()),
    }
}

/// `line-height`'s own grammar — a bare `<number>` (any value, not just
/// `0` — unlike `parse_length`'s unitless-zero special case, this is the
/// property's single most common real-world form) alongside `normal` and
/// the usual `px`/`em`/`rem`/`%` lengths. A `%` here resolves against the
/// *element's own* font-size (`Length::resolve`'s `Percent` arm, same
/// timing `Em` already uses for `line-height` — see `LineHeight`'s own
/// docs for why that's `Len`, not `Multiplier`: unlike a bare number, a
/// percentage computes to a fixed absolute value at the declaring
/// element, not something each descendant re-resolves against its own
/// font-size).
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
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("rem") => {
            Ok(LineHeight::Len(Length::Rem(*value)))
        }
        Token::Percentage { unit_value, .. } => Ok(LineHeight::Len(Length::Percent(*unit_value))),
        _ => Err(()),
    }
}

/// `font-weight`'s value as declared — `bolder`/`lighter` are relative to
/// whatever weight this element would otherwise *inherit*, not a fixed
/// number, so (like `font-size`'s `Length::Larger`/`Smaller`, v29) they
/// can't resolve to a concrete `FontWeight` until `resolve_style` has
/// `inherited.font_weight` in hand. Unlike `LineHeight`, nothing further
/// needs to stay symbolic once resolved — the result becomes an ordinary
/// `FontWeight` in `ComputedStyle`/`InheritedStyle`, inherited normally
/// from there, since `bolder`/`lighter` only ever look at the *immediate*
/// parent's weight, not some original declaring ancestor's.
#[derive(Clone, Copy)]
enum FontWeightValue {
    Absolute(FontWeight),
    Bolder,
    Lighter,
}

impl FontWeightValue {
    /// The CSS Fonts step table for relative weights (CSS Fonts Level 4
    /// §font-weight-prop) — not a simple ratio the way `font-size`'s
    /// `larger`/`smaller` use, since real browsers converge on these
    /// specific breakpoints rather than a multiply.
    fn resolve(self, inherited: FontWeight) -> FontWeight {
        match self {
            FontWeightValue::Absolute(w) => w,
            FontWeightValue::Bolder => FontWeight(if inherited.0 < 350 {
                400
            } else if inherited.0 < 550 {
                700
            } else {
                900
            }),
            FontWeightValue::Lighter => FontWeight(if inherited.0 < 550 {
                100
            } else if inherited.0 < 750 {
                400
            } else {
                700
            }),
        }
    }
}

fn parse_font_weight(input: &mut CssParser) -> Result<FontWeightValue, ()> {
    match input.next().map_err(|_| ())? {
        Token::Ident(ident) if ident.eq_ignore_ascii_case("normal") => {
            Ok(FontWeightValue::Absolute(FontWeight::NORMAL))
        }
        Token::Ident(ident) if ident.eq_ignore_ascii_case("bold") => {
            Ok(FontWeightValue::Absolute(FontWeight::BOLD))
        }
        Token::Ident(ident) if ident.eq_ignore_ascii_case("bolder") => Ok(FontWeightValue::Bolder),
        Token::Ident(ident) if ident.eq_ignore_ascii_case("lighter") => Ok(FontWeightValue::Lighter),
        Token::Number { value, .. } => Ok(FontWeightValue::Absolute(FontWeight(value.round() as u16))),
        _ => Err(()),
    }
}

/// `text-decoration-style`'s value, so far as this file can actually
/// paint it differently from a plain solid stroke. `double` isn't a
/// variant here — cosmic-text already natively supports a double
/// *underline* (`UnderlineStyle::Double`), so `parse_text_decoration`
/// folds a `double` keyword straight into `deco.underline` instead of
/// going through this enum (it only ever affects underline this way —
/// strikethrough/overline have no double variant in cosmic-text 0.19,
/// a narrower scope worth documenting but not worth hand-rolling a
/// second rectangle for). `wavy` has no custom painting path either —
/// an actual sine-wave stroke needs real curve rasterization this file
/// doesn't have anywhere else — and falls back to `Solid`, same
/// "accepted but not visually distinct" narrowing as `border-style`'s
/// `groove`/`ridge`/`inset`/`outset` (see `border_style_from_keyword`).
/// `pub` — appears in the public `ShapedParagraph`/`ShapeCache` type
/// aliases, same reason `VerticalAlign`/`CursorStyle` are `pub` too.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum DecorationStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

/// What the `text-decoration` shorthand resolves to for one declaration —
/// `deco` (cosmic-text's own struct: which lines, double-underline, and
/// per-line color, the last two set directly inside
/// `parse_text_decoration` since resolving a color token needs no cascade
/// context) plus `style` (solid/dashed/dotted, painted by hand in
/// `paint_decorations` instead of cosmic-text's own renderer). Bundled as
/// one struct rather than two independently-cascaded fields because both
/// only ever come from the same one shorthand at the same time — there's
/// no `text-decoration-style`/`-color` longhand support, so there's never
/// a case where one half is set without the other.
#[derive(Clone, Copy, Default)]
struct DecorationValue {
    deco: TextDecoration,
    style: DecorationStyle,
}

/// Parses the `text-decoration` shorthand: line keywords
/// (`underline`/`line-through`/`overline`/`none`), a style keyword
/// (`solid`/`dashed`/`dotted`/`double`/`wavy` — see `DecorationStyle`'s
/// own docs for which of these actually paint differently), and an
/// explicit color, in any order (e.g. `abbr[title]`'s `text-decoration:
/// dotted underline`). An unrecognized token is skipped rather than
/// failing the whole value, same lenient philosophy as
/// `parse_border_like_shorthand`. Color and style are collected locally
/// and applied only after the loop ends, so token order doesn't matter —
/// `dotted underline` and `underline dotted` both work. Returns `Err`
/// only if no line keyword was found at all, so a value that's pure noise
/// still drops like any other unsupported value.
/// Parses a full `font-family` value: a comma-separated fallback list,
/// each entry either a specific family name — a quoted string
/// (`"Georgia"`) or one-or-more bare idents joined with spaces, per the
/// real CSS grammar (`Helvetica Neue`) — or one of the 5 bare generic
/// keywords (only recognized when the *entire* entry is a single ident
/// matching one exactly; `Helvetica Neue` can never collide with a
/// generic since no generic keyword is two words). Fails (dropping the
/// whole declaration, same convention as every other unsupported value
/// in this file) only if the value is empty or a comma has no entry on
/// one side of it — unlike v16's single-keyword grammar, an individual
/// *entry* is never dropped on its own and the rest kept, because a
/// fallback list's entire point is that a later entry is only tried if
/// an earlier one fails to resolve against the real font system at
/// paint time (see `FontFamily::resolve`), not at parse time.
fn parse_font_family(input: &mut CssParser) -> Result<Vec<FontFamilyItem>, ()> {
    let mut items = Vec::new();
    loop {
        if let Ok(s) = input.try_parse(|input| input.expect_string().map(|s| s.as_ref().to_string())) {
            items.push(FontFamilyItem::Name(Box::leak(s.into_boxed_str())));
        } else {
            let mut words: Vec<String> = Vec::new();
            loop {
                let state = input.state();
                match input.next() {
                    Ok(Token::Ident(ident)) => words.push(ident.as_ref().to_string()),
                    _ => {
                        input.reset(&state);
                        break;
                    }
                }
            }
            if words.is_empty() {
                return Err(());
            }
            let generic = if words.len() == 1 {
                let w = &words[0];
                if w.eq_ignore_ascii_case("sans-serif") {
                    Some(FontFamilyItem::SansSerif)
                } else if w.eq_ignore_ascii_case("serif") {
                    Some(FontFamilyItem::Serif)
                } else if w.eq_ignore_ascii_case("monospace") {
                    Some(FontFamilyItem::Monospace)
                } else if w.eq_ignore_ascii_case("cursive") {
                    Some(FontFamilyItem::Cursive)
                } else if w.eq_ignore_ascii_case("fantasy") {
                    Some(FontFamilyItem::Fantasy)
                } else {
                    None
                }
            } else {
                None
            };
            items.push(
                generic.unwrap_or_else(|| FontFamilyItem::Name(Box::leak(words.join(" ").into_boxed_str()))),
            );
        }
        let state = input.state();
        match input.next() {
            Ok(Token::Comma) => continue,
            _ => {
                input.reset(&state);
                break;
            }
        }
    }
    if items.is_empty() {
        Err(())
    } else {
        Ok(items)
    }
}

fn parse_text_decoration(input: &mut CssParser) -> Result<(TextDecoration, DecorationStyle), ()> {
    let mut deco = TextDecoration::new();
    let mut style = DecorationStyle::Solid;
    let mut is_double = false;
    let mut color = None;
    let mut found = false;
    loop {
        let state = input.state();
        if let Ok(Token::Ident(ident)) = input.next() {
            if ident.eq_ignore_ascii_case("underline") {
                deco.underline = UnderlineStyle::Single;
                found = true;
                continue;
            } else if ident.eq_ignore_ascii_case("line-through") {
                deco.strikethrough = true;
                found = true;
                continue;
            } else if ident.eq_ignore_ascii_case("overline") {
                deco.overline = true;
                found = true;
                continue;
            } else if ident.eq_ignore_ascii_case("none") {
                found = true;
                continue;
            } else if ident.eq_ignore_ascii_case("dashed") {
                style = DecorationStyle::Dashed;
                continue;
            } else if ident.eq_ignore_ascii_case("dotted") {
                style = DecorationStyle::Dotted;
                continue;
            } else if ident.eq_ignore_ascii_case("solid") {
                style = DecorationStyle::Solid;
                continue;
            } else if ident.eq_ignore_ascii_case("double") {
                is_double = true;
                continue;
            } else if ident.eq_ignore_ascii_case("wavy") {
                // Not supported — falls back to `Solid` (see
                // `DecorationStyle`'s own docs).
                continue;
            }
        }
        input.reset(&state);
        if let Ok(c) = CssColor::parse(input) {
            color = to_text_color(&c);
            continue;
        }
        input.reset(&state);
        match input.next() {
            Ok(_) => {}
            Err(_) => break,
        }
    }
    if is_double && deco.underline == UnderlineStyle::Single {
        deco.underline = UnderlineStyle::Double;
    }
    if let Some(c) = color {
        if deco.underline != UnderlineStyle::None {
            deco.underline_color_opt = Some(c);
        }
        if deco.strikethrough {
            deco.strikethrough_color_opt = Some(c);
        }
        if deco.overline {
            deco.overline_color_opt = Some(c);
        }
    }
    if found {
        Ok((deco, style))
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

fn flex_direction_from_keyword(ident: &str) -> Option<FlexDirection> {
    if ident.eq_ignore_ascii_case("row") {
        Some(FlexDirection::Row)
    } else if ident.eq_ignore_ascii_case("row-reverse") {
        Some(FlexDirection::RowReverse)
    } else if ident.eq_ignore_ascii_case("column") {
        Some(FlexDirection::Column)
    } else if ident.eq_ignore_ascii_case("column-reverse") {
        Some(FlexDirection::ColumnReverse)
    } else {
        None
    }
}

fn flex_wrap_from_keyword(ident: &str) -> Option<FlexWrap> {
    if ident.eq_ignore_ascii_case("nowrap") {
        Some(FlexWrap::NoWrap)
    } else if ident.eq_ignore_ascii_case("wrap") {
        Some(FlexWrap::Wrap)
    } else if ident.eq_ignore_ascii_case("wrap-reverse") {
        Some(FlexWrap::WrapReverse)
    } else {
        None
    }
}

fn parse_flex_wrap(input: &mut CssParser) -> Result<FlexWrap, ()> {
    let ident = input.expect_ident().map_err(|_| ())?;
    flex_wrap_from_keyword(ident).ok_or(())
}

/// `align-items`/`align-self`'s 5 keywords (minus `align-self`'s own
/// extra `auto`, handled separately — see its own `DeclParser` branch).
fn parse_align_keyword(input: &mut CssParser) -> Result<AlignItems, ()> {
    let ident = input.expect_ident().map_err(|_| ())?;
    if ident.eq_ignore_ascii_case("stretch") {
        Ok(AlignItems::Stretch)
    } else if ident.eq_ignore_ascii_case("flex-start") {
        Ok(AlignItems::FlexStart)
    } else if ident.eq_ignore_ascii_case("flex-end") {
        Ok(AlignItems::FlexEnd)
    } else if ident.eq_ignore_ascii_case("center") {
        Ok(AlignItems::Center)
    } else if ident.eq_ignore_ascii_case("baseline") {
        Ok(AlignItems::Baseline)
    } else {
        Err(())
    }
}

/// `flex-grow`/`flex-shrink`'s own grammar: a plain non-negative
/// `<number>` — real CSS actually clamps a negative value to `0` rather
/// than rejecting the whole declaration; matched here for the same
/// reason (`.max(0.0)`), not just convenience.
fn parse_nonneg_number(input: &mut CssParser) -> Result<f32, ()> {
    match input.next().map_err(|_| ())? {
        Token::Number { value, .. } => Ok(value.max(0.0)),
        _ => Err(()),
    }
}

/// `flex-basis`'s own grammar: `auto` (real initial value, falls back to
/// the item's own `width`/`height`/content size — see `FlexBasis`'s own
/// docs) alongside the usual `<length-percentage>` forms. `content` (a
/// newer keyword, forcing content-based sizing even if `width`/`height`
/// is set) isn't recognized — not what `auto`'s own fallback-to-`width`
/// behavior already covers for the common case, and nothing exercises
/// the distinction.
fn parse_flex_basis(input: &mut CssParser) -> Result<FlexBasis, ()> {
    let state = input.state();
    if let Ok(ident) = input.expect_ident()
        && ident.eq_ignore_ascii_case("auto")
    {
        return Ok(FlexBasis::Auto);
    }
    input.reset(&state);
    parse_length_percent(input).map(FlexBasis::Len)
}

/// The `flex` shorthand: `none` (`flex-grow: 0; flex-shrink: 0; flex-
/// basis: auto` — real spec value for "never grow or shrink, size from
/// content/width"), `auto` (`1 1 auto` — grow and shrink freely from
/// content/width), or `<'flex-grow'> <'flex-shrink'>? || <'flex-
/// basis'>` in either order (`flex: 1`, `flex: 1 0`, `flex: 1 200px`,
/// `flex: 200px 1` all valid). Unlike `border`/`outline`'s own lenient
/// shorthand loops, an unrecognized token here drops the whole
/// declaration (same as `box-shadow`) — flex-grow/flex-shrink are both
/// plain numbers, genuinely ambiguous to guess past if something odd
/// shows up, unlike width-vs-style-vs-color's more distinguishable
/// token shapes.
fn parse_flex_shorthand(input: &mut CssParser) -> Result<(f32, f32, FlexBasis), ()> {
    let state = input.state();
    if let Ok(ident) = input.expect_ident() {
        if ident.eq_ignore_ascii_case("none") {
            return Ok((0.0, 0.0, FlexBasis::Auto));
        }
        if ident.eq_ignore_ascii_case("auto") {
            return Ok((1.0, 1.0, FlexBasis::Auto));
        }
    }
    input.reset(&state);
    let mut numbers: Vec<f32> = Vec::new();
    let mut basis: Option<FlexBasis> = None;
    loop {
        let state = input.state();
        // Capped at 2 (`flex-grow`/`flex-shrink`) — otherwise a bare
        // unitless `0` meant as `flex-basis` (`parse_length_percent`'s
        // own literal-zero rule, same as any other length property)
        // would be mistaken for a third number instead, since a bare
        // number and a bare `0` length are lexically identical tokens.
        if numbers.len() < 2 {
            if let Ok(v) = parse_nonneg_number(input) {
                numbers.push(v);
                continue;
            }
            input.reset(&state);
        }
        if basis.is_none() {
            if let Ok(b) = parse_flex_basis(input) {
                basis = Some(b);
                continue;
            }
            input.reset(&state);
        }
        match input.next() {
            Ok(_) => return Err(()),
            Err(_) => break,
        }
    }
    if numbers.is_empty() && basis.is_none() {
        return Err(());
    }
    let grow = numbers.first().copied().unwrap_or(1.0);
    let shrink = numbers.get(1).copied().unwrap_or(1.0);
    Ok((grow, shrink, basis.unwrap_or(FlexBasis::Auto)))
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
        if name.starts_with("--") {
            // Custom property: grammar is "anything", so there's no typed
            // value to parse — just capture the raw source text of the
            // whole value (consuming every token so `parse_entirely`,
            // which wraps this call, doesn't see it as a parse error).
            let start = input.state();
            while input.next().is_ok() {}
            let raw = input.slice_from(start.position()).trim().to_string();
            self.decls.custom_props.push((name.to_string(), raw));
        } else if name.eq_ignore_ascii_case("color") {
            if let Ok(c) = CssColor::parse(input) {
                self.decls.color = Some(c);
            }
        } else if name.eq_ignore_ascii_case("background-color") {
            if let Ok(c) = CssColor::parse(input) {
                self.decls.background_color = Some(c);
            }
        } else if name.eq_ignore_ascii_case("background-image") {
            if let Ok(url) = input.expect_url() {
                self.decls.background_image = Some(url.to_string());
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
                } else if ident.eq_ignore_ascii_case("flex") {
                    Some(Display::Flex)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("position") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.position = if ident.eq_ignore_ascii_case("static") {
                    Some(Position::Static)
                } else if ident.eq_ignore_ascii_case("relative") {
                    Some(Position::Relative)
                } else if ident.eq_ignore_ascii_case("absolute") {
                    Some(Position::Absolute)
                } else if ident.eq_ignore_ascii_case("fixed") {
                    Some(Position::Fixed)
                } else if ident.eq_ignore_ascii_case("sticky") {
                    Some(Position::Sticky)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("float") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.float = if ident.eq_ignore_ascii_case("none") {
                    Some(Float::None)
                } else if ident.eq_ignore_ascii_case("left") {
                    Some(Float::Left)
                } else if ident.eq_ignore_ascii_case("right") {
                    Some(Float::Right)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("clear") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.clear = if ident.eq_ignore_ascii_case("none") {
                    Some(Clear::None)
                } else if ident.eq_ignore_ascii_case("left") {
                    Some(Clear::Left)
                } else if ident.eq_ignore_ascii_case("right") {
                    Some(Clear::Right)
                } else if ident.eq_ignore_ascii_case("both") {
                    Some(Clear::Both)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("top") {
            set_length(input, &mut self.decls.top);
        } else if name.eq_ignore_ascii_case("right") {
            set_length(input, &mut self.decls.right);
        } else if name.eq_ignore_ascii_case("bottom") {
            set_length(input, &mut self.decls.bottom);
        } else if name.eq_ignore_ascii_case("left") {
            set_length(input, &mut self.decls.left);
        } else if name.eq_ignore_ascii_case("z-index") {
            if let Ok(z) = parse_z_index(input) {
                self.decls.z_index = Some(z);
            }
        } else if name.eq_ignore_ascii_case("width") {
            set_length(input, &mut self.decls.width);
        } else if name.eq_ignore_ascii_case("height") {
            set_length(input, &mut self.decls.height);
        } else if name.eq_ignore_ascii_case("min-width") {
            set_length(input, &mut self.decls.min_width);
        } else if name.eq_ignore_ascii_case("min-height") {
            set_length(input, &mut self.decls.min_height);
        } else if name.eq_ignore_ascii_case("max-width") {
            set_max_length(input, &mut self.decls.max_width);
        } else if name.eq_ignore_ascii_case("max-height") {
            set_max_length(input, &mut self.decls.max_height);
        } else if name.eq_ignore_ascii_case("box-sizing") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.box_sizing = if ident.eq_ignore_ascii_case("content-box") {
                    Some(BoxSizing::ContentBox)
                } else if ident.eq_ignore_ascii_case("border-box") {
                    Some(BoxSizing::BorderBox)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("margin") {
            if let Ok(v) = parse_length_percent(input) {
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
            if let Ok(v) = parse_length_percent(input) {
                self.decls.margin_top = Some(v);
                self.decls.margin_bottom = Some(v);
            }
        } else if name.eq_ignore_ascii_case("margin-inline") {
            if let Ok(v) = parse_length_percent(input) {
                self.decls.margin_left = Some(v);
                self.decls.margin_right = Some(v);
            }
        } else if name.eq_ignore_ascii_case("padding") {
            if let Ok(v) = parse_length_percent(input) {
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
            if let Ok(v) = parse_length_percent(input) {
                self.decls.padding_top = Some(v);
                self.decls.padding_bottom = Some(v);
            }
        } else if name.eq_ignore_ascii_case("padding-inline") {
            if let Ok(v) = parse_length_percent(input) {
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
        } else if name.eq_ignore_ascii_case("overflow") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.overflow = if ident.eq_ignore_ascii_case("visible") {
                    Some(Overflow::Visible)
                } else if ident.eq_ignore_ascii_case("hidden") {
                    Some(Overflow::Hidden)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("cursor") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.cursor = if ident.eq_ignore_ascii_case("auto") {
                    Some(CursorStyle::Auto)
                } else if ident.eq_ignore_ascii_case("pointer") {
                    Some(CursorStyle::Pointer)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("flex-direction") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.flex_direction = if ident.eq_ignore_ascii_case("row") {
                    Some(FlexDirection::Row)
                } else if ident.eq_ignore_ascii_case("row-reverse") {
                    Some(FlexDirection::RowReverse)
                } else if ident.eq_ignore_ascii_case("column") {
                    Some(FlexDirection::Column)
                } else if ident.eq_ignore_ascii_case("column-reverse") {
                    Some(FlexDirection::ColumnReverse)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("flex-wrap") {
            if let Ok(w) = parse_flex_wrap(input) {
                self.decls.flex_wrap = Some(w);
            }
        } else if name.eq_ignore_ascii_case("flex-flow") {
            // `<'flex-direction'> || <'flex-wrap'>`, either order, each
            // 0-1 times — same lenient component-loop shape as `border`/
            // `outline`'s own shorthands (`parse_border_like_shorthand`),
            // just with this property's own two keyword sets instead.
            let mut direction = None;
            let mut wrap = None;
            loop {
                let state = input.state();
                let Ok(ident) = input.expect_ident_cloned() else { break };
                if direction.is_none()
                    && let Some(d) = flex_direction_from_keyword(&ident)
                {
                    direction = Some(d);
                    continue;
                }
                if wrap.is_none()
                    && let Some(w) = flex_wrap_from_keyword(&ident)
                {
                    wrap = Some(w);
                    continue;
                }
                input.reset(&state);
                break;
            }
            if let Some(d) = direction {
                self.decls.flex_direction = Some(d);
            }
            if let Some(w) = wrap {
                self.decls.flex_wrap = Some(w);
            }
        } else if name.eq_ignore_ascii_case("justify-content") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.justify_content = if ident.eq_ignore_ascii_case("flex-start") {
                    Some(JustifyContent::FlexStart)
                } else if ident.eq_ignore_ascii_case("flex-end") {
                    Some(JustifyContent::FlexEnd)
                } else if ident.eq_ignore_ascii_case("center") {
                    Some(JustifyContent::Center)
                } else if ident.eq_ignore_ascii_case("space-between") {
                    Some(JustifyContent::SpaceBetween)
                } else if ident.eq_ignore_ascii_case("space-around") {
                    Some(JustifyContent::SpaceAround)
                } else if ident.eq_ignore_ascii_case("space-evenly") {
                    Some(JustifyContent::SpaceEvenly)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("align-items") {
            if let Ok(a) = parse_align_keyword(input) {
                self.decls.align_items = Some(a);
            }
        } else if name.eq_ignore_ascii_case("align-self") {
            // `auto` (the real initial value) is deliberately *not*
            // matched here, same `set_length`/`parse_z_index`-wide
            // "unparseable leaves the cascade's existing value
            // untouched" convention — `self.decls.align_self: Option
            // <AlignItems>`'s `None` already covers both "never
            // declared" and "declared `auto`" identically (see
            // `ComputedStyle::align_self`'s own docs), so there's
            // nothing a matched `auto` arm would do differently from
            // just failing to parse.
            if let Ok(a) = parse_align_keyword(input) {
                self.decls.align_self = Some(a);
            }
        } else if name.eq_ignore_ascii_case("flex-grow") {
            if let Ok(g) = parse_nonneg_number(input) {
                self.decls.flex_grow = Some(g);
            }
        } else if name.eq_ignore_ascii_case("flex-shrink") {
            if let Ok(s) = parse_nonneg_number(input) {
                self.decls.flex_shrink = Some(s);
            }
        } else if name.eq_ignore_ascii_case("flex-basis") {
            if let Ok(b) = parse_flex_basis(input) {
                self.decls.flex_basis = Some(b);
            }
        } else if name.eq_ignore_ascii_case("flex") {
            if let Ok((grow, shrink, basis)) = parse_flex_shorthand(input) {
                self.decls.flex_grow = Some(grow);
                self.decls.flex_shrink = Some(shrink);
                self.decls.flex_basis = Some(basis);
            }
        } else if name.eq_ignore_ascii_case("gap") {
            // `gap: <row-gap> <column-gap>?` — one value sets both.
            if let Ok(row) = parse_length(input) {
                let col = parse_length(input).unwrap_or(row);
                self.decls.row_gap = Some(row);
                self.decls.column_gap = Some(col);
            }
        } else if name.eq_ignore_ascii_case("row-gap") {
            if let Ok(v) = parse_length(input) {
                self.decls.row_gap = Some(v);
            }
        } else if name.eq_ignore_ascii_case("column-gap") {
            if let Ok(v) = parse_length(input) {
                self.decls.column_gap = Some(v);
            }
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
                if let Ok(v) = parse_length_percent(input) {
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
            // A real comma-separated fallback list — see `parse_font_
            // family`'s own docs for the grammar and `FontFamily`'s for
            // why the parsed `Vec` gets leaked into a `&'static` slice
            // here rather than stored by value.
            if let Ok(items) = parse_font_family(input) {
                self.decls.font_family = Some(FontFamily(Box::leak(items.into_boxed_slice())));
            }
        } else if name.eq_ignore_ascii_case("text-decoration") {
            if let Ok((deco, style)) = parse_text_decoration(input) {
                self.decls.text_decoration = Some(DecorationValue { deco, style });
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

/// Collects every `--name` custom-property declaration from whichever
/// `rules` match the document's real `<html>` element — i.e. `:root`,
/// `html`, `html.foo`, or anything else whose selector happens to match
/// it, via the exact same real selector matching `resolve_style` uses,
/// not a hardcoded string check. This is the dominant real-world pattern
/// for custom properties (design tokens declared once at the top,
/// referenced everywhere via `var()`) and is deliberately the *only*
/// pattern supported: a `--name` declared on some other, non-root-
/// matching selector (e.g. `.dark-mode { --bg: ... }`, scoping a
/// variable to one subtree) isn't picked up here and plays no part in
/// `var()` substitution. Real CSS custom properties inherit per-element
/// like any other inherited property, with descendants able to override
/// them — that would mean threading a resolved-custom-props map through
/// `resolve_style`'s per-element cascade (plus a second, per-element
/// `var()` substitution pass, since an element's inherited custom props
/// aren't known until its ancestors are resolved) instead of this one
/// flat, whole-document table built once up front. Nothing in a real
/// test page needs anything but root-scoped tokens yet, so that more
/// invasive version is deferred, not attempted. Later same-name
/// declarations (by rule order, not specificity) win on a tie, matching
/// `cascade_property`'s own `>=` tie-break elsewhere in this file.
fn collect_root_custom_props(rules: &[StyleRule], html: &Handle) -> HashMap<String, String> {
    let element = El(html.clone());
    let mut caches = SelectorCaches::default();
    let mut props = HashMap::new();
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
            for (name, value) in &rule.decls.custom_props {
                props.insert(name.clone(), value.clone());
            }
        }
    }
    props
}

/// Replaces every `var(--name)` / `var(--name, <fallback>)` call
/// anywhere in `css` with the matching entry from `vars` — or, if
/// `--name` isn't in `vars`, the fallback text (itself recursively
/// substituted, so `var(--a, var(--b, red))` resolves correctly) — or,
/// with no fallback either, an empty string (same as everywhere else in
/// this file: an unresolvable value just makes that one declaration
/// silently fail to parse, not a hard error for the whole rule/block).
///
/// A plain balanced-paren-aware text scan, not a real CSS tokenizer:
/// correctly finds the *matching* close-paren even through a nested
/// function call in the fallback, but has no notion of CSS string or
/// comment context, so (unlike real `var()` substitution) it would still
/// fire inside a quoted string, e.g. `content: "var(--x)"`. Nothing in
/// `default.css` or a real test page does that, so not handled. Also:
/// `vars`' own values are used verbatim, never substituted against each
/// other — a `:root { --a: blue; --b: var(--a); }` won't resolve `--b`'s
/// `var(--a)` to `blue`, only a *fallback* ever gets this recursive
/// treatment. A real page defining one custom property in terms of
/// another would need that; nothing exercises it yet, and avoiding it
/// here sidesteps needing a cycle guard for a theoretical `--a: var(--a)`.
fn substitute_css_vars(css: &str, vars: &HashMap<String, String>) -> String {
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < bytes.len() {
        if css[i..].starts_with("var(") && is_var_call_boundary(bytes, i) {
            let call_start = i + "var(".len();
            if let Some(call_end) = matching_close_paren(bytes, call_start) {
                out.push_str(&resolve_var_call(&css[call_start..call_end], vars));
                i = call_end + 1;
                continue;
            }
        }
        let ch_len = css[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&css[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// `var(` shouldn't match as a trailing slice of a longer identifier
/// (`-var(`, `myvar(`) — require the preceding byte, if any, to not be
/// an identifier character.
fn is_var_call_boundary(bytes: &[u8], i: usize) -> bool {
    i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'-' || bytes[i - 1] == b'_')
}

/// `start` is the index of the first byte *after* an already-consumed
/// `(` — returns the index of its matching `)`, accounting for any
/// nested parens in between.
fn matching_close_paren(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1u32;
    for (offset, &b) in bytes[start..].iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// `inner` is a `var(...)` call's contents (everything between its outer
/// parens) — splits on the first *top-level* comma (not one nested
/// inside a fallback's own function call) into name/fallback, resolves
/// `name` against `vars`, or recursively substitutes and returns the
/// fallback if `name` isn't declared.
fn resolve_var_call(inner: &str, vars: &HashMap<String, String>) -> String {
    let bytes = inner.as_bytes();
    let mut depth = 0u32;
    let mut comma_at = None;
    for (idx, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                comma_at = Some(idx);
                break;
            }
            _ => {}
        }
    }
    let (name, fallback) = match comma_at {
        Some(idx) => (inner[..idx].trim(), Some(inner[idx + 1..].trim())),
        None => (inner.trim(), None),
    };
    if let Some(v) = vars.get(name) {
        return v.clone();
    }
    match fallback {
        Some(f) => substitute_css_vars(f, vars),
        None => String::new(),
    }
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
    /// For `rem` lengths (`Length::Rem`) — resolved once from the real
    /// `<html>` element's own cascaded `font-size` (`build_layout_tree`),
    /// then copied unchanged through every other element's `inherited()`/
    /// `anonymous_style` rather than re-derived per element the way real
    /// inheritance works for every other field here, since `rem` is
    /// document-root-relative, not parent-relative.
    root_font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    font_family: FontFamily,
    white_space: WhiteSpace,
    list_style_type: ListStyleType,
    text_align: TextAlign,
    line_height: LineHeight,
    visibility: Visibility,
    cursor: CursorStyle,
}

const ROOT_STYLE: InheritedStyle = InheritedStyle {
    color: TextColor::rgb(0, 0, 0),
    font_size: 16.0,
    root_font_size: 16.0,
    font_weight: FontWeight::NORMAL,
    font_style: FontStyle::Normal,
    font_family: FontFamily(&[FontFamilyItem::SansSerif]),
    white_space: WhiteSpace::Normal,
    list_style_type: ListStyleType::Disc,
    text_align: TextAlign::Left,
    line_height: LineHeight::Normal,
    visibility: Visibility::Visible,
    cursor: CursorStyle::Auto,
};

/// Per-side box-model values. Physical (top/right/bottom/left), not logical
/// — scratchy only ever lays out horizontal-tb/LTR, so block-start/end and
/// inline-start/end collapse onto top/bottom and left/right respectively;
/// logical property names are expanded to these physical sides at parse
/// time (`DeclParser::parse_value`), same as `margin`/`padding` shorthands
/// are. `border_width` stays a single uniform scalar — nothing in
/// `default.css` sets a per-side border, so there's no real use for it yet.
///
/// Fields are `BoxLength`, not a plain `f32` — real CSS resolves a `%`
/// margin/padding (on *every* side, even top/bottom — see
/// `finalize_box_lengths`) against the containing block's width, which
/// `margin`/`padding` on `ComputedStyle` can't know at cascade time. See
/// `BoxLength`'s own docs for exactly when/how that final resolution
/// happens.
#[derive(Clone, Copy, Default)]
struct Sides {
    top: BoxLength,
    right: BoxLength,
    bottom: BoxLength,
    left: BoxLength,
}

#[derive(Clone, Copy)]
struct ComputedStyle {
    color: TextColor,
    background_color: Option<TextColor>,
    display: Display,
    /// Not inherited (see `Position`'s own docs). Forces `display` to
    /// `Block` when `Absolute`/`Fixed` — real CSS's own "computed
    /// `display`" rule (spec §9.7) — applied once, in `resolve_style`,
    /// right after both are cascaded; everywhere else in this file
    /// (`collect_children`'s block-vs-inline split, `measure`) can keep
    /// branching on `display` alone with no separate `position` check,
    /// since an absolutely/fixed positioned element is *always*
    /// block-level by the time either of them sees it.
    position: Position,
    /// `top`/`right`/`bottom`/`left` — only meaningful (resolved against
    /// a real containing block) once `position` isn't `Static`; see
    /// `Insets`'s own docs. Not inherited.
    inset: Insets,
    /// `None` means `auto` (`z-index`'s real initial value) — paints no
    /// differently from an explicit `0` (see `catch_or_bubble_
    /// positioned`'s sort key), the one simplification this file's
    /// stacking-order support makes vs. the real spec (where `auto`
    /// additionally means "don't start a new stacking context" — a
    /// distinction this file doesn't model at all; see TODO.txt). Not
    /// inherited, and only ever consulted for a `position`-ed box (an
    /// in-flow box's own `z_index` is simply never read).
    z_index: Option<i32>,
    /// Not inherited (see `Float`'s own docs). Forces `display` to
    /// `Block` the same way `position: absolute`/`fixed` does — applied
    /// in `resolve_style` right alongside that existing rule, so nothing
    /// downstream needs its own separate `float` check for that part.
    /// Only ever consulted for an ordinary in-flow child in `measure`'s
    /// `BoxContent::Blocks` arm (an `absolute`/`fixed` box's own `float`
    /// is never read — real CSS says `position` wins and `float`
    /// computes to `none` in that case, but since that box is already
    /// routed through the entirely separate `PendingPositioned` path,
    /// there's nothing here that would read `float` for it anyway).
    float: Float,
    /// Not inherited. See `Clear`'s own docs.
    clear: Clear,
    /// `None` means `auto` (the default — content-driven sizing, same
    /// as before this property existed). `Some` is the specified
    /// width/height in pixels, in `box_sizing`'s reference box (content
    /// box by default, so unchanged from before `box_sizing` existed —
    /// padding/border still add outward on top of this for
    /// `ContentBox`, but are *carved out* of it for `BorderBox`): see
    /// `measure`'s own docs for how each is actually applied — they
    /// differ: `width` constrains the content box directly (and
    /// children's own available width), while `height` only overrides
    /// this box's own reported height for its *parent*'s stacking —
    /// content taller than an explicit `height` still lays out and
    /// paints in full, spilling past this box (real CSS `overflow:
    /// visible` default), clipped only if `overflow: hidden` is also
    /// set (already correct since v40, no extra plumbing needed here).
    /// `min_width`/`max_width`/`min_height`/`max_height` clamp this same
    /// box (see `clamp_box_dim`) — `None` means unconstrained on that
    /// side (`min`'s real initial value, `0`, needs no explicit
    /// constraint since every box is already floored at `0`; `max`'s
    /// real initial value is literally `none`).
    ///
    /// `BoxLength`, not a plain `f32`: a `%` here resolves against the
    /// containing block's width (`width`/`min_width`/`max_width`) or its
    /// own definite height (`height`/`min_height`/`max_height`, which
    /// may not exist — see `measure`'s `containing_height` parameter and
    /// `finalize_box_lengths`), neither of which `resolve_style` knows at
    /// cascade time.
    width: Option<BoxLength>,
    height: Option<BoxLength>,
    min_width: Option<BoxLength>,
    max_width: Option<BoxLength>,
    min_height: Option<BoxLength>,
    max_height: Option<BoxLength>,
    /// Not inherited (same as `overflow`/`border_width`). See
    /// `BoxSizing`'s own docs.
    box_sizing: BoxSizing,
    margin: Sides,
    padding: Sides,
    border_width: f32,
    border_color: Option<TextColor>,
    border_style: BorderStyle,
    font_size: f32,
    /// Passed straight through from `inherited.root_font_size`, not
    /// derived from this element's own `font_size` — see
    /// `InheritedStyle::root_font_size`'s own docs.
    root_font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    font_family: FontFamily,
    /// The cascaded-but-not-yet-propagated value — `None` means nothing
    /// declared on *this* element, in which case the nearest ancestor's
    /// decoration (if any) still shows through, same "ambient, not real
    /// CSS inheritance" threading as `background_color` uses (see
    /// `collect_children`) — text-decoration isn't a spec-inherited
    /// property either, but a UA still draws an ancestor's decoration
    /// through descendant inline boxes that don't override it. Bundles
    /// line flags, style (solid/dashed/dotted), and color together (see
    /// `DecorationValue`'s own docs) since all three only ever come from
    /// the one `text-decoration` shorthand.
    text_decoration: Option<DecorationValue>,
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
    /// Rounds both the background fill and (for `BorderStyle::Solid`
    /// only — see `fill_rounded_ring`'s own docs) the border itself, via
    /// `paint_box_decoration`. Not inherited. `BoxLength`, not a plain
    /// `f32`: real CSS resolves a `%` border-radius against the box's
    /// *own* border-box width (unlike `width`/`margin`/`padding`, whose
    /// basis is the *containing block's* width) — `paint_box_decoration`
    /// resolves it directly against its own `w` parameter, since that's
    /// already the exact value needed and it's never used anywhere
    /// `measure` would otherwise have to finalize it early.
    border_radius: BoxLength,
    /// Not inherited. See `Overflow`'s own docs for the clip's scope.
    overflow: Overflow,
    cursor: CursorStyle,
    /// The following flexbox fields are all not inherited (same category
    /// as `border_width`/`overflow`) — a flex container's own `display:
    /// flex` establishes a *new* flex formatting context for its direct
    /// children only; none of these properties reach past it to a
    /// grandchild (real CSS: `flex-direction` etc. simply aren't
    /// inherited properties at all). `flex_direction`/`flex_wrap`/
    /// `justify_content`/`align_items`/the two `*_gap` fields are only
    /// ever consulted on a `display: flex` box itself (`measure_flex`);
    /// `flex_grow`/`flex_shrink`/`flex_basis`/`align_self` are only ever
    /// consulted on that box's own *direct children* (a flex item reads
    /// its own cascaded value for these, regardless of what its own
    /// `display` is).
    flex_direction: FlexDirection,
    flex_wrap: FlexWrap,
    justify_content: JustifyContent,
    align_items: AlignItems,
    /// `None` means `auto` (the real initial value) — resolved against
    /// the *containing* flex container's own `align_items` only at
    /// layout time (`measure_flex`), not here, since `resolve_style`
    /// doesn't know whether this element's parent is even a flex
    /// container at all.
    align_self: Option<AlignItems>,
    flex_grow: f32,
    flex_shrink: f32,
    flex_basis: FlexBasis,
    /// Resolved eagerly (like `border_width`/`outline_width`) rather
    /// than deferred as a `BoxLength` — real CSS allows a `%` `row-gap`/
    /// `column-gap` (resolving against the flex container's own cross/
    /// main size respectively), but that's deliberately not supported
    /// here (see `TODO.txt`): only `px`/`em`/`rem` parse at all
    /// (`parse_length`, not `parse_length_percent`), so there's no
    /// deferred-percent case to carry through to `measure_flex` the way
    /// `width`/`margin` do.
    row_gap: f32,
    column_gap: f32,
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
            root_font_size: self.root_font_size,
            font_weight: self.font_weight,
            font_style: self.font_style,
            font_family: self.font_family,
            white_space: self.white_space,
            list_style_type: self.list_style_type,
            text_align: self.text_align,
            line_height: self.line_height,
            visibility: self.visibility,
            cursor: self.cursor,
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

/// Every rule in `rules` whose selector matches `handle`, paired with its
/// specificity — the real selector-matching work `resolve_style` itself
/// needs, factored out so `resolve_background_image` can reuse it
/// (instead of re-running `matches_selector_list` over every rule a
/// second time) rather than folding background-image into
/// `ComputedStyle` itself — see `resolve_background_image`'s own docs
/// for why it's resolved separately at all.
fn matching_declarations<'a>(handle: &Handle, rules: &'a [StyleRule]) -> Vec<(u32, &'a Declarations)> {
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
    matches
}

/// Resolves and decodes this element's `background-image`, if any, as
/// its own small cascade — not folded into `ComputedStyle` the way
/// `background-color` is, because `ComputedStyle` is `#[derive(Copy)]`
/// (cheaply duplicated everywhere a box's style is read: `let s = b.
/// style;` throughout `measure`/`paint`) and a decoded image's pixel
/// buffer can't be `Copy`. Threaded alongside a box instead, in its own
/// `Rc` — same reasoning, and the same `Rc<DecodedImage>` type, as
/// `BoxContent::Image` (see its own docs). Only a `data:` URI `url(...)`
/// decodes — same `<img src>` policy, see `decode_data_uri`'s own docs.
fn resolve_background_image(handle: &Handle, rules: &[StyleRule]) -> Option<Rc<DecodedImage>> {
    let matches = matching_declarations(handle, rules);
    let inline_decls = element_attr(handle, "style").map(|s| parse_declarations(&s));
    let url = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.background_image.clone()),
        |d| d.background_image.clone(),
    )?;
    decode_data_uri(&url).and_then(|b| decode_image_bytes(&b)).map(Rc::new)
}

fn resolve_style(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle) -> ComputedStyle {
    let matches = matching_declarations(handle, rules);
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
    // Not inherited (see `Position`'s own docs) — every element starts
    // from `Static` regardless of an ancestor's own `position`.
    let position = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.position), |d| d.position)
        .unwrap_or_default();
    // Not inherited (see `Float`'s own docs) — every element starts from
    // `None` regardless of an ancestor's own `float`.
    let float = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.float), |d| d.float)
        .unwrap_or_default();
    let clear = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.clear), |d| d.clear)
        .unwrap_or_default();
    // Real CSS's own "computed `display`" rule (spec §9.7): an
    // `absolute`/`fixed` element, *or* a floated one, is always
    // block-level, overriding whatever `display` the cascade otherwise
    // picked — *except* `display: none`, which still wins (there's no
    // box to be block-level about). Applied here, once, so every later
    // piece of this file that branches on `display` (`collect_children`,
    // `measure`) needs no separate `position`/`float` check of its own.
    let display = if (matches!(position, Position::Absolute | Position::Fixed) || float != Float::None)
        && display != Display::None
    {
        Display::Block
    } else {
        display
    };
    // Resolved before margin/padding/border-width: an `em` on any of those
    // is relative to *this* element's own font-size (not its parent's),
    // so font-size has to be cascaded and resolved first. An `em` on
    // font-size itself is relative to the parent's (`inherited.font_size`)
    // — there's no self-reference, since font-size can only ever be `em`
    // relative to the value it's inheriting from, never its own result.
    let font_size = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.font_size), |d| d.font_size)
        .map(|l| l.resolve(inherited.font_size, inherited.root_font_size))
        .unwrap_or(inherited.font_size);
    let width = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.width), |d| d.width)
        .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size));
    let height = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.height), |d| d.height)
        .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size));
    let min_width = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.min_width), |d| d.min_width)
        .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size));
    let min_height = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.min_height), |d| d.min_height)
        .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size));
    let max_width = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.max_width), |d| d.max_width)
        .and_then(|l| l.resolve(font_size, inherited.root_font_size));
    let max_height = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.max_height), |d| d.max_height)
        .and_then(|l| l.resolve(font_size, inherited.root_font_size));
    let box_sizing = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.box_sizing), |d| d.box_sizing)
        .unwrap_or_default();
    // Only ever actually resolved (`.resolve(basis)`) once a real
    // containing block is found for a `position`-ed box (`resolve_
    // positioned`) — see `Insets`'s own docs for why these stay
    // `BoxLength`, deferred the same way `width`/`margin`/etc. already
    // are, rather than resolved here against anything.
    let inset = Insets {
        top: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.top), |d| d.top)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size)),
        right: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.right), |d| d.right)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size)),
        bottom: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.bottom), |d| d.bottom)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size)),
        left: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.left), |d| d.left)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size)),
    };
    // Not inherited; `None` ("auto") is only ever actually consulted
    // for a `position`-ed box — see `ComputedStyle::z_index`'s own docs.
    let z_index = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.z_index), |d| d.z_index);
    let margin = Sides {
        top: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_top), |d| d.margin_top)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size))
            .unwrap_or_default(),
        right: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_right), |d| d.margin_right)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size))
            .unwrap_or_default(),
        bottom: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_bottom), |d| d.margin_bottom)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size))
            .unwrap_or_default(),
        left: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_left), |d| d.margin_left)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size))
            .unwrap_or_default(),
    };
    let padding = Sides {
        top: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.padding_top), |d| d.padding_top)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size))
            .unwrap_or_default(),
        right: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.padding_right), |d| d.padding_right)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size))
            .unwrap_or_default(),
        bottom: cascade_property(
            &matches,
            inline_decls.as_ref().and_then(|d| d.padding_bottom),
            |d| d.padding_bottom,
        )
        .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size))
        .unwrap_or_default(),
        left: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.padding_left), |d| d.padding_left)
            .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size))
            .unwrap_or_default(),
    };
    let border_width = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_width),
        |d| d.border_width,
    )
    .map(|l| l.resolve(font_size, inherited.root_font_size))
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
    .map(|v| v.resolve(inherited.font_weight))
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
    .map(|l| l.resolve(font_size, inherited.root_font_size))
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
            offset_x: bs.offset_x.resolve(font_size, inherited.root_font_size),
            offset_y: bs.offset_y.resolve(font_size, inherited.root_font_size),
            spread: bs.spread.resolve(font_size, inherited.root_font_size),
            color,
        })
    });
    let border_radius = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_radius),
        |d| d.border_radius,
    )
    .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size))
    .unwrap_or_default();
    let overflow = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.overflow), |d| d.overflow)
        .unwrap_or_default();
    let cursor = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.cursor), |d| d.cursor)
        .unwrap_or(inherited.cursor);
    let flex_direction = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.flex_direction),
        |d| d.flex_direction,
    )
    .unwrap_or_default();
    let flex_wrap = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.flex_wrap), |d| d.flex_wrap)
        .unwrap_or_default();
    let justify_content = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.justify_content),
        |d| d.justify_content,
    )
    .unwrap_or_default();
    let align_items = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.align_items),
        |d| d.align_items,
    )
    .unwrap_or_default();
    // Not inherited, and `None` here really does mean `auto` (see this
    // field's own docs) — unlike `align_items` above, there's no
    // `.unwrap_or_default()`, since `measure_flex` has to tell "not
    // declared/auto" apart from an explicit value to fall back to the
    // *container's* own `align_items` instead of a fixed default.
    let align_self = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.align_self),
        |d| d.align_self,
    );
    let flex_grow = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.flex_grow), |d| d.flex_grow)
        .unwrap_or(0.0);
    let flex_shrink = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.flex_shrink),
        |d| d.flex_shrink,
    )
    .unwrap_or(1.0);
    let flex_basis = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.flex_basis),
        |d| d.flex_basis,
    )
    .unwrap_or_default();
    let row_gap = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.row_gap), |d| d.row_gap)
        .map(|l| l.resolve(font_size, inherited.root_font_size))
        .unwrap_or(0.0);
    let column_gap = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.column_gap),
        |d| d.column_gap,
    )
    .map(|l| l.resolve(font_size, inherited.root_font_size))
    .unwrap_or(0.0);

    ComputedStyle {
        color,
        background_color,
        display,
        position,
        inset,
        z_index,
        float,
        clear,
        width,
        height,
        min_width,
        max_width,
        min_height,
        max_height,
        box_sizing,
        margin,
        padding,
        border_width,
        border_color,
        border_style,
        font_size,
        root_font_size: inherited.root_font_size,
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
        overflow,
        cursor,
        flex_direction,
        flex_wrap,
        justify_content,
        align_items,
        align_self,
        flex_grow,
        flex_shrink,
        flex_basis,
        row_gap,
        column_gap,
    }
}

fn anonymous_style(inherited: InheritedStyle) -> ComputedStyle {
    ComputedStyle {
        color: inherited.color,
        background_color: None,
        display: Display::Block,
        position: Position::Static,
        inset: Insets::default(),
        z_index: None,
        float: Float::default(),
        clear: Clear::default(),
        width: None,
        height: None,
        min_width: None,
        max_width: None,
        min_height: None,
        max_height: None,
        box_sizing: BoxSizing::default(),
        margin: Sides::default(),
        padding: Sides::default(),
        border_width: 0.0,
        border_color: None,
        border_style: BorderStyle::default(),
        font_size: inherited.font_size,
        root_font_size: inherited.root_font_size,
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
        border_radius: BoxLength::default(),
        overflow: Overflow::default(),
        cursor: inherited.cursor,
        flex_direction: FlexDirection::default(),
        flex_wrap: FlexWrap::default(),
        justify_content: JustifyContent::default(),
        align_items: AlignItems::default(),
        align_self: None,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        flex_basis: FlexBasis::default(),
        row_gap: 0.0,
        column_gap: 0.0,
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
    /// Ambient, same as `text_decoration` (see `DecorationValue`'s docs)
    /// — always set together with it, never independently.
    decoration_style: DecorationStyle,
    vertical_align: VerticalAlign,
    line_height: f32,
    /// Real CSS inheritance (see `Visibility`'s own docs), so this is
    /// read straight off `s.visibility` — already the correctly-cascaded
    /// value for wherever this span's text node actually sits, descendant
    /// overrides included — not a separate ambient-propagation parameter.
    visibility: Visibility,
    cursor: CursorStyle,
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
            decoration_style: DecorationStyle::default(),
            vertical_align: VerticalAlign::default(),
            line_height: s.line_height.resolve(s.font_size, s.root_font_size),
            visibility: s.visibility,
            cursor: s.cursor,
        }
    }
}

/// One piece of an inline run — either a text span (the only kind that
/// existed before v48) or an inline-level `<img>` (`display: inline`,
/// the default — a `display: block` `<img>` instead gets its own
/// `BoxContent::Image` box entirely, same as any other block element;
/// see `collect_children`'s own docs for that split). An inline image
/// carries its own `ComputedStyle` (for size/margin/border/background —
/// same fields a block-level replaced element uses, via the shared
/// `image_box_size`) and its own resolved `background_image`, since
/// both live outside `ComputedStyle` itself for the same reason
/// `LayoutBox.background_image` does (see its own docs) — an inline
/// `<img>` is still a full element that can have its own CSS
/// background-image/border/etc., not just its foreground pixels.
enum InlineItem {
    Text(String, SpanStyle),
    Image {
        image: Rc<DecodedImage>,
        style: ComputedStyle,
        background_image: Option<Rc<DecodedImage>>,
    },
}

enum BoxContent {
    Blocks(Vec<LayoutBox>),
    Inline(Vec<InlineItem>),
    /// A table row's cells, laid out horizontally instead of stacked.
    /// Only ever appears on a box whose own `style.display` is
    /// `Display::TableRow` — rows themselves are ordinary `Blocks`
    /// children of the table, stacking vertically like any other block.
    TableRow(Vec<LayoutBox>),
    /// A decoded `<img>` — see `build_img_box`. Wrapped in `Rc` so
    /// `measure` can hand `Laid` its own cheap reference (`LaidContent::
    /// Image`) without re-decoding or copying the pixel buffer on every
    /// layout pass.
    Image(Rc<DecodedImage>),
}

/// A fully decoded `<img>`, in the same packed-`u32`-pixel form `blend`/
/// `fill_rect` already paint everything else in (so `blit_image` is a
/// direct composite, no per-pixel format conversion at paint time).
/// `width`/`height` are the *intrinsic* pixel dimensions straight off the
/// decoded image — `measure`'s own `Image` arm is what applies any CSS/
/// HTML `width`/`height` override on top.
struct DecodedImage {
    width: u32,
    height: u32,
    pixels: Vec<TextColor>,
}

pub struct LayoutBox {
    style: ComputedStyle,
    /// `None` for an anonymous box (not a real element — nothing to
    /// resolve `background-image` against) or a real element that
    /// didn't declare one. See `resolve_background_image`'s own docs
    /// for why this lives here instead of on `style` itself.
    background_image: Option<Rc<DecodedImage>>,
    content: BoxContent,
}

// ---------- images ----------

/// Decodes a `data:` URI's base64 payload into raw bytes — the only
/// `<img src>` form this engine understands (see TODO.txt: no
/// networking, and no local file reads either, until that's revisited
/// — `snag` already rewrites real pages' `<img src>`s into exactly this
/// form). Returns `None` for anything else: a non-`data:` scheme, a
/// non-base64 payload (the alternate, URL-encoded `data:` text form —
/// not what `snag` emits), or a malformed one.
fn decode_data_uri(src: &str) -> Option<Vec<u8>> {
    let rest = src.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    if !meta.split(';').any(|p| p.eq_ignore_ascii_case("base64")) {
        return None;
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(payload.trim()).ok()
}

/// Decodes raw image bytes (already extracted from a `data:` URI) via
/// the `image` crate, converting straight to this file's own packed-
/// `u32` pixel form so nothing downstream needs to know `image`'s types
/// exist at all. `None` for anything `image` can't decode — an
/// unsupported format (only PNG/JPEG/GIF/WebP are compiled in, see
/// `Cargo.toml`), or truncated/corrupt data.
fn decode_image_bytes(bytes: &[u8]) -> Option<DecodedImage> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    let pixels = rgba.pixels().map(|p| TextColor::rgba(p[0], p[1], p[2], p[3])).collect();
    Some(DecodedImage { width, height, pixels })
}

/// An `<img width>`/`<img height>` HTML attribute, as a plain unitless
/// pixel integer (the only form the HTML spec gives these attributes —
/// no `em`/`%`/other CSS length syntax). Only consulted as a fallback
/// when the CSS cascade left the matching property `auto` (see
/// `build_img_box`) — a real UA attaches these as the lowest-priority
/// presentational hint, exactly as if `img { width: Npx }` were in the
/// UA stylesheet, so CSS always wins once any is declared.
fn img_size_attr(handle: &Handle, name: &str) -> Option<f32> {
    element_attr(handle, name)?.trim().parse::<f32>().ok().filter(|v| *v > 0.0)
}

/// Resolves `<img>`'s own box-model style (CSS `width`/`height`, falling
/// back to the HTML attributes — see `img_size_attr`'s own docs) and
/// decodes its `src`, shared by both places `<img>` can end up:
/// `build_img_box` (a `display: block` `<img>`, its own box) and
/// `collect_children`'s inline branch (the default `display: inline` —
/// pushed straight into the surrounding text run as an `InlineItem::
/// Image` instead, see its own docs). Only a `data:` URI `src` decodes;
/// anything else — missing, unsupported format, a local file path (not
/// yet supported, see TODO.txt), or corrupt data — returns `None`.
fn resolve_img(handle: &Handle, mut style: ComputedStyle) -> (ComputedStyle, Option<Rc<DecodedImage>>) {
    if style.width.is_none() {
        style.width = img_size_attr(handle, "width").map(BoxLength::Px);
    }
    if style.height.is_none() {
        style.height = img_size_attr(handle, "height").map(BoxLength::Px);
    }
    let decoded = element_attr(handle, "src").and_then(|src| decode_data_uri(&src)).and_then(|b| decode_image_bytes(&b));
    (style, decoded.map(Rc::new))
}

/// Builds `<img>`'s own box directly, bypassing the normal children walk
/// (it's a void element — nothing to recurse into) — only reached for a
/// `display: block`/`table` `<img>` (see `collect_children`'s own docs);
/// the default `display: inline` case never calls this. A `src` that
/// fails to decode falls back to rendering `alt` as plain text in the
/// image's place, same as a real browser's "fallback content" for a
/// replaced element that failed to load, rather than collapsing to a
/// blank 0×0 box.
fn build_img_box(handle: &Handle, style: ComputedStyle, background_image: Option<Rc<DecodedImage>>) -> LayoutBox {
    let (style, decoded) = resolve_img(handle, style);
    match decoded {
        Some(image) => LayoutBox { style, background_image, content: BoxContent::Image(image) },
        None => {
            let alt = element_attr(handle, "alt").map(|a| a.to_string()).unwrap_or_default();
            LayoutBox { style, background_image, content: BoxContent::Inline(vec![InlineItem::Text(alt, style.inherited().into())]) }
        }
    }
}

/// `decoration` is the *ambient* text-decoration this element inherited
/// from whatever called it — a block boundary alone doesn't interrupt
/// propagation (see `collect_children`'s own docs on why it's ambient,
/// not real CSS inheritance), so this can't just reset to
/// `DecorationValue::default()` the way it used to. Resolved against
/// this element's *own* declared value (`style.text_decoration`, if
/// any) right away, then threaded into every path that builds this
/// element's own children — same `unwrap_or` shape `collect_children`'s
/// inline branch already uses, just applied once per block too.
fn build_box_for_element(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle, decoration: DecorationValue) -> LayoutBox {
    let style = resolve_style(handle, rules, inherited);
    // Resolved once here (cheap to carry further — just an `Rc` clone —
    // vs. re-running `resolve_background_image`'s own selector-matching
    // pass again in every branch below).
    let background_image = resolve_background_image(handle, rules);
    // Only reached for a `display: block`/`table` `<img>` — the default
    // `display: inline` case is handled entirely inside `collect_children`
    // instead (pushed into the surrounding text run, never given its own
    // box via this function at all; see its own docs). A block-level
    // `<img>` is a void element (no real children to walk) and a
    // replaced element (its own box, built directly from the decoded
    // image rather than `BoxContent::Blocks`/`Inline`) — handled before
    // anything else here, same early-return shape `Display::Table`'s
    // own branch just below uses for the same reason (it builds its
    // `LayoutBox` directly too, rather than delegating further down).
    if element_tag(handle) == "img" {
        return build_img_box(handle, style, background_image);
    }
    let own_decoration = style.text_decoration.unwrap_or(decoration);
    if style.display == Display::Table {
        return LayoutBox {
            style,
            background_image,
            content: BoxContent::Blocks(build_table_children(handle, rules, style.inherited(), own_decoration)),
        };
    }
    // `display: flex` is block-level in the outer flow (same as `Table`,
    // just above) but lays its *content* out completely differently —
    // each direct child becomes a flex item (`build_flex_children`),
    // measured per `measure_flex` instead of normal block/inline flow.
    if style.display == Display::Flex {
        return LayoutBox {
            style,
            background_image,
            content: BoxContent::Blocks(build_flex_children(handle, rules, style.inherited(), own_decoration)),
        };
    }
    let prefix = if element_tag(handle) == "li" {
        Some((list_marker(handle, style.list_style_type), style.inherited().into()))
    } else {
        None
    };
    LayoutBox {
        style,
        background_image,
        content: BoxContent::Blocks(build_children_with_prefix(handle, rules, style.inherited(), prefix, own_decoration)),
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
fn build_table_children(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle, decoration: DecorationValue) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    for child in handle.children.borrow().iter() {
        let NodeData::Element { .. } = &child.data else { continue };
        let tag = element_tag(child);
        if tag == "script" || tag == "style" {
            continue;
        }
        let style = resolve_style(child, rules, inherited);
        // Resolved once here (not left to `build_box_for_element`, since
        // the `TableRow`/`TableGroup` arms build their `LayoutBox`
        // directly rather than delegating to it) — the `Block` arm below
        // passes the un-resolved `decoration` straight through instead,
        // since `build_box_for_element` already redoes this same
        // `unwrap_or` itself once it has the child's own `style`.
        let child_decoration = style.text_decoration.unwrap_or(decoration);
        match style.display {
            Display::None => {}
            Display::TableRow => {
                let cells = build_table_row_cells(child, rules, style.inherited(), child_decoration);
                let background_image = resolve_background_image(child, rules);
                result.push(LayoutBox { style, background_image, content: BoxContent::TableRow(cells) });
            }
            Display::TableGroup => {
                result.extend(build_table_children(child, rules, style.inherited(), child_decoration));
            }
            Display::Block | Display::Flex => {
                // e.g. `caption`, or any other block-level content placed
                // directly under `<table>` — a stray `display: flex`
                // element here is the same kind of edge case, handled
                // the same way (its own box via the normal dispatch,
                // which re-enters `build_flex_children` for it).
                result.push(build_box_for_element(child, rules, inherited, decoration));
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
fn build_table_row_cells(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle, decoration: DecorationValue) -> Vec<LayoutBox> {
    let mut cells = Vec::new();
    for child in handle.children.borrow().iter() {
        let NodeData::Element { .. } = &child.data else { continue };
        let style = resolve_style(child, rules, inherited);
        if style.display == Display::TableCell {
            cells.push(build_box_for_element(child, rules, inherited, decoration));
        }
    }
    cells
}

/// A `display: flex` element's direct children, each becoming its own
/// flex item — real CSS "blockifies" every flex item (spec §4: a flex
/// item's outer display type is always block-level, regardless of its
/// own `display`), which falls out *for free* here: `build_box_for_
/// element` already builds a full `Blocks`-content box for whatever it's
/// given regardless of that element's own `style.display` (it only ever
/// branches on `display` to special-case `img`/`Table`/`Flex`
/// themselves, never to decide "does this get its own box at all" — only
/// `collect_children`'s inline-run grouping makes that decision, which
/// this function bypasses entirely by calling `build_box_for_element` on
/// every child directly). So a `<span>` (default `display: inline`)
/// placed directly inside a flex container still gets its own real box
/// here, exactly like a `<div>` would — nothing extra needed to force
/// block-level behavior.
///
/// **Deliberately not attempted** (see `TODO.txt`): a bare text node
/// directly inside the flex container (no wrapping element) doesn't
/// become its own anonymous flex item the way real CSS would — only
/// element children are collected here, matching the task's own
/// explicitly-permitted scope-narrowing latitude; non-whitespace text
/// placed directly in a flex container (uncommon in real page chrome,
/// which almost always wraps flex items in an element) is silently
/// dropped rather than rendered. `position: absolute`/`fixed` children
/// are *not* filtered out here (unlike `build_table_children`'s own
/// `Display::None` skip) — they're still real children of this returned
/// list, but `measure_flex` itself (not this function) is what actually
/// takes them out of the flex algorithm, the same split `measure`'s own
/// `BoxContent::Blocks` arm already uses for an ordinary block's
/// absolutely/fixed positioned children.
fn build_flex_children(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle, decoration: DecorationValue) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    for child in handle.children.borrow().iter() {
        let NodeData::Element { .. } = &child.data else { continue };
        let tag = element_tag(child);
        if tag == "script" || tag == "style" {
            continue;
        }
        let style = resolve_style(child, rules, inherited);
        if style.display == Display::None {
            continue;
        }
        result.push(build_box_for_element(child, rules, inherited, decoration));
    }
    result
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
fn build_children(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle, decoration: DecorationValue) -> Vec<LayoutBox> {
    build_children_with_prefix(handle, rules, inherited, None, decoration)
}

/// Same as `build_children`, but seeds the first inline run with `prefix`
/// (used for a `<li>`'s marker) before any of `handle`'s own children.
fn build_children_with_prefix(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    prefix: Option<(String, SpanStyle)>,
    decoration: DecorationValue,
) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    let mut pending: Vec<InlineItem> = Vec::new();
    if let Some((text, span)) = prefix {
        pending.push(InlineItem::Text(text, span));
    }
    collect_children(
        handle,
        rules,
        inherited,
        None,
        decoration,
        VerticalAlign::default(),
        &mut pending,
        &mut result,
    );
    if !pending.is_empty() {
        result.push(LayoutBox {
            style: anonymous_style(inherited),
            background_image: None,
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
/// concrete `DecorationValue` (not `Option`), since "no decoration"
/// already has a representable off state within the type itself — an
/// element can use that to explicitly turn off an ancestor's decoration by
/// declaring `text-decoration: none`.
fn collect_children(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    bg: Option<TextColor>,
    decoration: DecorationValue,
    valign: VerticalAlign,
    pending: &mut Vec<InlineItem>,
    result: &mut Vec<LayoutBox>,
) {
    for child in handle.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => {
                let mut span: SpanStyle = inherited.into();
                span.background_color = bg;
                span.text_decoration = decoration.deco;
                span.decoration_style = decoration.style;
                span.vertical_align = valign;
                // `white-space: pre` preserves the text byte-for-byte
                // (literal newlines, runs of spaces, no fabricated
                // trailing space) instead of collapsing it.
                if inherited.white_space == WhiteSpace::Pre {
                    pending.push(InlineItem::Text(contents.borrow().to_string(), span));
                } else {
                    let mut text = normalize_whitespace(&contents.borrow());
                    text.push(' ');
                    pending.push(InlineItem::Text(text, span));
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
                // `<img>`'s own box-vs-inline-item split: a `display:
                // block`/`table` `<img>` (an explicit author override —
                // not the default) gets its own box, same as any other
                // block element. The *default* `display: inline` case
                // (v48 on) instead decodes right here and pushes an
                // `InlineItem::Image` straight into the surrounding text
                // run, with no flush — real inline-level flow, sharing
                // lines with adjacent text via `measure_mixed_inline`.
                // Before v48 this always flushed and broke the line
                // regardless of `display`, a deliberate (now reverted)
                // simplification — see TODO.txt/NOTES.md for why that
                // was the original call and what changed.
                if tag == "img" && !matches!(style.display, Display::Block | Display::Table | Display::Flex) {
                    let background_image = resolve_background_image(child, rules);
                    let (img_style, decoded) = resolve_img(child, style);
                    match decoded {
                        Some(image) => pending.push(InlineItem::Image { image, style: img_style, background_image }),
                        None => {
                            let alt = element_attr(child, "alt").map(|a| a.to_string()).unwrap_or_default();
                            let mut span: SpanStyle = inherited.into();
                            span.background_color = bg;
                            span.vertical_align = valign;
                            pending.push(InlineItem::Text(alt, span));
                        }
                    }
                } else if matches!(style.display, Display::Block | Display::Table | Display::Flex) {
                    if !pending.is_empty() {
                        result.push(LayoutBox {
                            style: anonymous_style(inherited),
                            background_image: None,
                            content: BoxContent::Inline(std::mem::take(pending)),
                        });
                    }
                    result.push(build_box_for_element(child, rules, inherited, decoration));
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

/// Finds the document's `<html>` element, if present — needed up front so
/// `rem` lengths can resolve against its real cascaded `font-size` (an
/// `html { font-size: ... }` override) instead of a fixed constant. A
/// shallow search would be enough for any well-formed document (html5ever's
/// `Document` node's only element child is `<html>`), but this walks the
/// whole tree so a malformed/fragment document still finds it if it's
/// nested unexpectedly, rather than silently falling back to the default.
fn find_html_element(handle: &Handle) -> Option<Handle> {
    if let NodeData::Element { name, .. } = &handle.data {
        if name.local.as_ref() == "html" {
            return Some(handle.clone());
        }
    }
    for child in handle.children.borrow().iter() {
        if let Some(found) = find_html_element(child) {
            return Some(found);
        }
    }
    None
}

pub fn build_layout_tree(document: &Handle) -> LayoutBox {
    let mut css_source = String::new();
    collect_style_css(document, &mut css_source);
    let html_handle = find_html_element(document);
    let mut rules = parse_stylesheet(DEFAULT_CSS);
    rules.extend(parse_stylesheet(&css_source));
    // CSS custom properties (v45): harvest `--name` values from whatever
    // rules in this first pass match the real `<html>` element (`:root`,
    // `html`, `html.foo`, ...) — see `collect_root_custom_props`'s own
    // docs for why only root-matching rules count — then re-parse both
    // stylesheets with every `var()` call textually substituted
    // (`substitute_css_vars`) so the normal per-property parsers above
    // see a plain value, never a `var()` they'd otherwise just silently
    // fail to recognize and drop. Gated on whether either stylesheet's
    // *text* mentions `var(` at all, not on whether `root_vars` actually
    // found anything — a `var(--undeclared, red)` with no matching
    // `--undeclared` anywhere still needs this pass to fall back to its
    // literal `red`, so an empty `root_vars` map doesn't mean nothing to
    // do. Skipped only for the common case of a page that never uses
    // `var()` at all, to avoid the extra reparse cost.
    if DEFAULT_CSS.contains("var(") || css_source.contains("var(") {
        let root_vars = html_handle
            .as_ref()
            .map(|html| collect_root_custom_props(&rules, html))
            .unwrap_or_default();
        let mut substituted = parse_stylesheet(&substitute_css_vars(DEFAULT_CSS, &root_vars));
        substituted.extend(parse_stylesheet(&substitute_css_vars(&css_source, &root_vars)));
        rules = substituted;
    }
    // `rem` lengths resolve against `<html>`'s own cascaded font-size, not
    // necessarily `ROOT_STYLE.font_size` — resolve it once up front (pure,
    // side-effect-free) so `root_font_size` carries the real value into
    // every other element's `resolve_style` call, including `<html>`'s own
    // second (otherwise-identical) resolution inside `build_children`.
    let root_font_size = html_handle
        .as_ref()
        .map(|html| resolve_style(html, &rules, ROOT_STYLE).font_size)
        .unwrap_or(ROOT_STYLE.font_size);
    let root_inherited = InheritedStyle {
        root_font_size,
        ..ROOT_STYLE
    };
    LayoutBox {
        style: anonymous_style(root_inherited),
        background_image: None,
        content: BoxContent::Blocks(build_children(document, &rules, root_inherited, DecorationValue::default())),
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
    /// Carried straight over from the `LayoutBox` this was measured
    /// from (an `Rc` clone — see its own docs for why it isn't on
    /// `style`), for `paint` to tile behind this box's own content.
    background_image: Option<Rc<DecodedImage>>,
    content: LaidContent,
    /// `position: absolute`/`fixed` descendants whose containing block
    /// is *this* box — empty for the overwhelming majority of boxes
    /// (no out-of-flow descendants at all). Resolved once, by
    /// `catch_or_bubble_positioned`, after this box's own normal
    /// (in-flow) content and final size are known; painted/hit-tested
    /// *after* that normal content (`paint`/`hit_test_cursor`), so they
    /// can legitimately overlap it — sorted ascending by `z-index`
    /// (`auto`/unset treated as `0`; see `ComputedStyle::z_index`'s own
    /// docs for the one real-spec distinction this drops), ties broken
    /// by document order, which is the closest this file gets to real
    /// CSS2.1 Appendix E stacking-context paint order without actually
    /// modeling stacking contexts themselves (see TODO.txt).
    positioned: Vec<Laid>,
    /// `true` only for a `position: fixed` box resolved at the very
    /// document root (`measure_document`) — never for anything inside
    /// some other box's own `positioned` list (a `fixed` descendant
    /// always bubbles all the way to the root; see `catch_or_bubble_
    /// positioned`'s own docs), and never for anything in `content`
    /// (an ordinary in-flow/`relative` descendant of a `fixed` box is
    /// just that — `fixed` is not itself ambiently inherited). Read
    /// only by `paint`, to skip subtracting the ambient scroll offset
    /// for this one subtree — the one thing that actually makes
    /// `fixed` stay put on screen while the page scrolls underneath it
    /// (see `paint`'s own docs). Does *not* affect positioning math
    /// itself (`resolve_positioned` already placed this box against
    /// the viewport, scroll-independent, before this flag is even set).
    fixed: bool,
    /// `Some` only for a `position: sticky` box — everything `sticky_
    /// shift` needs to compute this frame's painted position from the
    /// live scroll offset, resolved once here at layout time (never
    /// touched again by a scroll-offset change alone — see `Position::
    /// Sticky`'s own docs). `None` for every other box, including every
    /// other `position` value (`relative`'s own offset is instead baked
    /// permanently into `x`/`y` by `apply_relative_offset`, since it
    /// doesn't need to react to scroll at all).
    sticky: Option<StickyInfo>,
}

/// `position: sticky`'s own layout-time-resolved, scroll-independent
/// state — see `Position::Sticky`'s own docs for why this is split from
/// the actual per-frame position math (`sticky_shift`). Only `top`/
/// `bottom` are resolved — scratchy has no horizontal scrolling at all
/// (see NOTES.md's own scrolling writeup), so a `left`/`right` sticky
/// offset would have no live horizontal scroll position to clamp
/// against; deliberately not attempted, same "reject the part that
/// would need fundamentally different machinery" category `Position`'s
/// own former `sticky`-rejection doc comment already established for
/// this property as a whole. `top` wins over `bottom` when both are
/// set, same precedence `resolve_positioned` already gives `left`/`top`
/// over `right`/`bottom`.
#[derive(Clone, Copy)]
struct StickyInfo {
    /// This box's own would-be-static in-flow position, document-space
    /// — identical to `relative`'s own basis, and (unlike every other
    /// field here) already exactly `laid.y` by the time `capture_
    /// sticky_info` runs, since sticky never leaves flow and nothing
    /// has shifted it yet.
    static_top: f32,
    /// Resolved against this box's own containing block's height
    /// (`Insets`'s usual basis), same timing/basis `apply_relative_
    /// offset` already uses for `relative`'s own `top`/`bottom`. `None`
    /// means not declared (real CSS `auto`) — this box only sticks if
    /// at least one of `top`/`bottom` is set.
    top: Option<f32>,
    bottom: Option<f32>,
    /// This box's own containing block's content-box vertical extent,
    /// document-space — the real CSS containing block for anything
    /// that stays in normal flow (sticky included, same as `relative`),
    /// *not* the padding box `absolute`/`fixed` anchor against. Can't
    /// be known until the parent's own `Blocks`/table loop that placed
    /// this child finishes stacking every sibling (`patch_sticky_
    /// containing_block`, called right after that loop ends) — seeded
    /// with a permissive placeholder by `capture_sticky_info` in the
    /// meantime, which in practice is never actually read before being
    /// patched (every `Sticky` `Laid` is placed by exactly one such
    /// loop).
    cb_top: f32,
    cb_bottom: f32,
}

/// Computes this frame's sticky vertical offset — the `dy` to add to
/// `info.static_top` to get this box's real, current painted document-
/// space `y` — from the live, ambient `scroll_offset` (and the window's
/// own `viewport_h`, only needed for `bottom`'s own viewport-relative
/// math). Called fresh every `paint`/`hit_test_cursor` call, never
/// cached anywhere in `Laid` itself — the whole architectural point of
/// `Position::Sticky` being split the way it is: this can run every
/// single frame for free, with no relayout, because it's pure
/// arithmetic over values `measure` already resolved once.
///
/// Real sticky semantics, informally (CSS Position L3 §6.5): `top`
/// tries to keep this box's own top edge `top` px below the current
/// viewport's top edge (document space: `scroll_offset + top`) — but
/// never pulls it *above* its own static position (hasn't scrolled far
/// enough yet — stays in normal flow, `dy == 0`), and never pushes it
/// past its own containing block's bottom edge minus its own height
/// (would escape its own parent — "stops sticking" once that edge
/// scrolls past). `(scroll_offset + top).clamp(static_top, max_top)`
/// captures exactly this: below `static_top`, clamps up to it (not
/// stuck yet); between the two, tracks the viewport exactly (stuck);
/// above `max_top`, clamps down to it (stopped sticking — and since
/// `max_top` is a fixed document-space coordinate, the box then simply
/// scrolls away with the rest of its container past that point, same
/// as an ordinary static box would). `bottom` is the mirror image,
/// clamped the other way (`cb_top` is the edge it can't escape past).
/// Both `max_top`/`min_top` are additionally clamped against `static_
/// top` itself (`.max`/`.min`) as a pure defensive measure against
/// `f32::clamp`'s own `min <= max` requirement — real layout always
/// keeps a box within its own containing block, so this should never
/// actually change the result, just guarantee it can't panic if some
/// future caller's `cb_top`/`cb_bottom` ever disagreed with that
/// invariant.
fn sticky_shift(info: &StickyInfo, box_h: f32, scroll_offset: f32, viewport_h: f32) -> f32 {
    if let Some(top) = info.top {
        let want = scroll_offset + top;
        let max_top = (info.cb_bottom - box_h).max(info.static_top);
        return want.clamp(info.static_top, max_top) - info.static_top;
    }
    if let Some(bottom) = info.bottom {
        let want = scroll_offset + viewport_h - bottom - box_h;
        let min_top = info.cb_top.min(info.static_top);
        return want.clamp(min_top, info.static_top) - info.static_top;
    }
    0.0
}

/// `position: sticky`'s own layout-time half of the two-stage split
/// `Position::Sticky`'s own docs describe — called alongside `apply_
/// relative_offset` (same call sites, same `basis_h` parameter — only
/// `top`/`bottom`, whose basis is a containing-block *height*, are
/// resolved here; see `StickyInfo`'s own docs for why `left`/`right`
/// aren't). `laid.y` is still exactly this box's own static position at
/// this point — `apply_relative_offset` is a no-op for anything but
/// `Relative`, so nothing has shifted it. `cb_top`/`cb_bottom` are
/// seeded with a permissive placeholder here (this box's real
/// containing block — its *parent* — hasn't finished stacking every
/// sibling yet, so its final content-box extent isn't known); `patch_
/// sticky_containing_block`, called by that parent's own loop right
/// after it finishes, fills in the real values.
fn capture_sticky_info(laid: &mut Laid, s: &ComputedStyle, basis_h: Option<f32>) {
    if s.position != Position::Sticky {
        return;
    }
    let resolve_v = |bl: BoxLength| match bl {
        BoxLength::Px(v) => v,
        BoxLength::Percent(p) => basis_h.map(|h| h * p).unwrap_or(0.0),
    };
    laid.sticky = Some(StickyInfo {
        static_top: laid.y,
        top: s.inset.top.map(resolve_v),
        bottom: s.inset.bottom.map(resolve_v),
        cb_top: f32::MIN,
        cb_bottom: f32::MAX,
    });
}

/// Fills in the real `cb_top`/`cb_bottom` on every `position: sticky`
/// child a `Blocks`-like loop just finished stacking — the one thing
/// `capture_sticky_info` couldn't know yet (see its own docs). `cb_top`/
/// `cb_bottom` is this loop's own box's content-box vertical extent,
/// document-space — the real CSS containing block for anything that
/// stays in normal flow (sticky included, same as `relative`'s own
/// basis), now finally known (`content_y`/the loop's own final `cursor_
/// y`). Only ever touches a `Laid` this exact loop just placed
/// (`children`, by mutable reference) — never recurses: a sticky box
/// several levels deeper was already patched by whichever closer
/// ancestor's own loop placed *it*.
fn patch_sticky_containing_block(children: &mut [Laid], cb_top: f32, cb_bottom: f32) {
    for child in children {
        if let Some(info) = &mut child.sticky {
            info.cb_top = cb_top;
            info.cb_bottom = cb_bottom;
        }
    }
}

/// One already-placed float's margin-box extent, in absolute
/// document-space coordinates — the real spec term for this is a
/// float's "margin box" (CSS2.1 §9.5.1). `top`/`bottom` are the vertical
/// range it occupies (same convention `measure`'s own `cursor_y`
/// bookkeeping already uses: the position a normal sibling's flow would
/// have used, not shifted by this box's own margin). `edge` is the one
/// horizontal coordinate anything avoiding this float actually needs:
/// for a `Left` float, its own right margin edge (nothing else may start
/// left of this x); for a `Right` float, its own left margin edge
/// (nothing else may extend past this x) — see `Floats::avail_bounds`.
#[derive(Clone, Copy)]
struct FloatEntry {
    top: f32,
    bottom: f32,
    edge: f32,
}

/// Every float placed so far within one block formatting context (BFC) —
/// threaded through `measure`'s own recursion as a plain, read-only
/// `&Floats` (see `measure`'s own docs for exactly when a fresh, empty
/// one is started vs. an ancestor's forwarded unchanged), separately
/// from `pending: &mut Vec<PendingPositioned>`: a float's containing
/// block is always just "whichever block box's own `Blocks` loop placed
/// it" — unlike `absolute`/`fixed`, there's no search for some other
/// ancestor, so nothing here ever needs to *bubble* the way `Pending
/// Positioned` does. A `BoxContent::Blocks` arm clones whatever `Floats`
/// it was handed, grows the clone locally as it places its own direct
/// float children (so a later sibling, and anything nested inside an
/// earlier non-floated sibling, sees them too — real floats affect every
/// descendant inline formatting context in the same BFC, not just
/// immediate siblings), and passes a *fresh*, empty `Floats` into a
/// float's own content (and into any other box that starts a new BFC in
/// this file — `position: absolute`/`fixed` content, and a table cell's
/// own interior) — floats don't escape into, or leak in from outside,
/// one of those.
///
/// `pub` (fields stay private) purely so `measure`'s own public
/// signature can name this type — same reason `PendingPositioned` is
/// `pub` already: a bench/test caller outside this crate needs to
/// construct `Floats::default()` (the only thing it ever needs to do
/// with one) to call `measure` at all.
#[derive(Clone, Default)]
pub struct Floats {
    left: Vec<FloatEntry>,
    right: Vec<FloatEntry>,
}

impl Floats {
    fn is_empty(&self) -> bool {
        self.left.is_empty() && self.right.is_empty()
    }

    /// The lowest bottom edge among every float placed so far (on either
    /// side) — used as a cheap, conservative check (`measure`'s own
    /// `BoxContent::Inline` dispatch) for "has every float in this BFC
    /// already ended by the time this content starts," so a paragraph
    /// safely below every float can keep taking the fast, whole-buffer
    /// shaping path instead of the slower per-word one `avail_bounds`
    /// needs. `f32::MIN` (never `>=` any real `content_y`) when there
    /// are no floats at all.
    fn max_bottom(&self) -> f32 {
        self.left.iter().chain(self.right.iter()).map(|f| f.bottom).fold(f32::MIN, f32::max)
    }

    /// The horizontal range available for content at document-space `y`
    /// (a single probe at this exact `y`, not a check across a whole
    /// line's/box's real height range — the same real-but-approximate
    /// simplification a line's own height isn't known until every item
    /// on it is chosen; see `measure_mixed_inline`'s own docs), clamped
    /// within `[outer_left, outer_right]`. Only a float whose own
    /// `top..bottom` range actually contains `y` can narrow it — an
    /// already-finished (or not-yet-started) float has no effect.
    fn avail_bounds(&self, y: f32, outer_left: f32, outer_right: f32) -> (f32, f32) {
        let left_bound = self
            .left
            .iter()
            .filter(|f| f.top <= y && y < f.bottom)
            .map(|f| f.edge)
            .fold(outer_left, f32::max);
        let right_bound = self
            .right
            .iter()
            .filter(|f| f.top <= y && y < f.bottom)
            .map(|f| f.edge)
            .fold(outer_right, f32::min);
        (left_bound, right_bound.max(left_bound))
    }

    /// The flow position a `clear`-ing box must not start above: the
    /// lowest bottom edge among the floats `clear` actually names
    /// (`Left`/`Right`/`Both`), or `f32::MIN` if `clear` is `None` or
    /// there's nothing to clear — safe to `cursor_y.max(...)` against
    /// unconditionally, since `f32::MIN` never wins that `max`.
    fn clear_y(&self, clear: Clear) -> f32 {
        let mut y = f32::MIN;
        if matches!(clear, Clear::Left | Clear::Both) {
            y = self.left.iter().map(|f| f.bottom).fold(y, f32::max);
        }
        if matches!(clear, Clear::Right | Clear::Both) {
            y = self.right.iter().map(|f| f.bottom).fold(y, f32::max);
        }
        y
    }
}

/// A `position: absolute`/`fixed` descendant collected while its
/// immediate containing `Blocks`/table structure is being measured —
/// deliberately *not* measured there and then (see `measure`'s own
/// `BoxContent::Blocks` arm): real CSS takes it out of normal flow
/// entirely (no box reserved for it, no effect on where any sibling
/// ends up — `static_x`/`static_y` below exist only as its own
/// position's *fallback*, not something any sibling ever reads), and
/// its real containing block is the nearest *positioned* ancestor
/// (`Position::Relative`/`Absolute`/`Fixed`, i.e. anything but
/// `Static`) — which might be several levels further up the tree than
/// its immediate parent, not knowable until that ancestor (if any) is
/// actually reached. So this just carries everything `resolve_
/// positioned` will eventually need, and bubbles up through each
/// `measure`/`measure_table` call's own `pending` out-parameter
/// (`catch_or_bubble_positioned`) until something catches it.
pub struct PendingPositioned<'a> {
    /// The box itself — measured for real only once `resolve_
    /// positioned` has a real containing block to measure it against.
    b: &'a LayoutBox,
    /// `true` for `position: fixed`. Unlike `absolute`, a `fixed` box's
    /// containing block is always the viewport (not whatever positioned
    /// ancestor happens to be nearest) — real CSS spec behavior pre-
    /// transforms, the only case this engine has any notion of at all —
    /// so a `fixed` entry is never caught by `catch_or_bubble_
    /// positioned` at an intermediate box regardless of that box's own
    /// `position`; it always keeps bubbling until `measure_document`'s
    /// own root-level resolution.
    fixed: bool,
    /// Absolute document-space coordinates (same convention every
    /// other position in this file uses) of where this box would have
    /// been placed had it stayed in normal flow — i.e. exactly the
    /// `x`/`y` its parent's own `Blocks` loop would have fed an
    /// ordinary `measure` call for it. Used as-is whenever the
    /// corresponding `top`/`right`/`bottom`/`left` pair is entirely
    /// `auto` (real CSS's own "static position" fallback — see
    /// `resolve_positioned`), needing no translation into any
    /// containing block's coordinate space, since every `Laid` position
    /// in this file already lives in the same absolute document space
    /// regardless of which box is whose containing block.
    static_x: f32,
    static_y: f32,
}

/// Shifts `laid`'s own position, and every absolute document-space
/// coordinate anywhere inside its subtree (recursing into normal
/// `Blocks`/`TableRow` children, a `MixedInline` run's own per-word/
/// per-image positions, and this box's own `positioned` out-of-flow
/// descendants too), by `(dx, dy)` — the one operation both `position:
/// relative`'s own offset (`apply_relative_offset`) and `resolve_
/// positioned`'s own left/top-vs-right/bottom solving need: moving an
/// already-fully-laid-out subtree without re-measuring any of it (pure
/// geometry, no shaping/box-model work to redo). Never recurses into a
/// `positioned` entry that's `fixed` — impossible to reach from here
/// regardless (see `Laid::fixed`'s own docs: a `fixed` box's `Laid`
/// only ever exists in the document root's own `positioned` list, which
/// is never itself the subject of a `relative`-offset shift, since the
/// synthetic document root is always `Position::Static`), but checked
/// explicitly anyway rather than relying on that invariant silently.
///
/// Also shifts `laid.sticky`'s own document-space fields (`static_top`/
/// `cb_top`/`cb_bottom`), if any — needed because a `position: sticky`
/// descendant can itself be shifted from above (e.g. a `position:
/// relative` ancestor's own offset, or `resolve_positioned`'s right/
/// bottom-solving), and `capture_sticky_info`/`patch_sticky_containing_
/// block` already ran for it by the time that could ever happen (see
/// `StickyInfo`'s own docs) — without this, a sticky box nested inside
/// a shifted ancestor would keep stale pre-shift bounds, sticking (or
/// failing to) relative to the wrong document-space position.
fn shift_laid_tree(laid: &mut Laid, dx: f32, dy: f32) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    laid.x += dx;
    laid.y += dy;
    if let Some(info) = &mut laid.sticky {
        info.static_top += dy;
        info.cb_top += dy;
        info.cb_bottom += dy;
    }
    match &mut laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            for child in children {
                shift_laid_tree(child, dx, dy);
            }
        }
        LaidContent::MixedInline(items) => {
            for item in items {
                match item {
                    MixedPositioned::Word { x, y, .. } => {
                        *x += dx;
                        *y += dy;
                    }
                    MixedPositioned::Image(img) => shift_laid_tree(img, dx, dy),
                }
            }
        }
        LaidContent::Text(_) | LaidContent::Image(_) => {}
    }
    for item in &mut laid.positioned {
        if !item.fixed {
            shift_laid_tree(item, dx, dy);
        }
    }
}

/// `position: relative`'s own, much simpler effect vs. `absolute`/
/// `fixed`: stays in normal flow (already measured as if `Static` by
/// the time this is called — the caller's own `cursor_y` advance
/// already happened against this box's *unshifted* `h`/margins, so
/// flow is genuinely unaffected), just visually offset from where it
/// would otherwise have painted — and, since any `position`-ed box
/// (this one included) is a valid containing block for a nested
/// `absolute` descendant, this has to shift *everything* already
/// resolved against this box's own (pre-shift) padding box too
/// (`laid.positioned`, populated by `catch_or_bubble_positioned` before
/// this ever runs) — not just `laid`'s own normal content — so a
/// `position: absolute` child anchored to a `position: relative;
/// top: 20px` parent ends up anchored to the parent's real, shifted
/// position, not its pre-offset one. `top`/`left`/`right`/`bottom` all
/// apply (unlike `absolute`/`fixed`, where only one of each pair ever
/// matters — see `resolve_positioned`): real CSS lets `relative` use
/// both of an axis's offsets, with `top`+`bottom` (or `left`+`right`)
/// together simply summing with opposite sign when both are set,
/// unlike `absolute`'s left/top-wins precedence (there's no box edge
/// being solved for here, just an offset).
/// `basis_w` is this box's own containing block's content *width* —
/// exactly `measure`'s own `width` parameter, the same basis `box_
/// model_entry` already resolves this box's own `margin`/`width`
/// percentages against. `basis_h` is that containing block's content
/// *height*, only when that's definite — exactly `measure`'s own
/// `containing_height` parameter, same basis (and same `None`-means-
/// "percentage computes to auto" rule — see `finalize_box_lengths`'s
/// own docs) `height`'s own `%` already uses.
fn apply_relative_offset(laid: &mut Laid, s: &ComputedStyle, basis_w: f32, basis_h: Option<f32>) {
    if s.position != Position::Relative {
        return;
    }
    let resolve_v = |bl: BoxLength| match bl {
        BoxLength::Px(v) => v,
        BoxLength::Percent(p) => basis_h.map(|h| h * p).unwrap_or(0.0),
    };
    let dx = s.inset.left.map(|v| v.resolve(basis_w)).unwrap_or(0.0) - s.inset.right.map(|v| v.resolve(basis_w)).unwrap_or(0.0);
    let dy = s.inset.top.map(resolve_v).unwrap_or(0.0) - s.inset.bottom.map(resolve_v).unwrap_or(0.0);
    shift_laid_tree(laid, dx, dy);
}

/// Splits `own_pending` (gathered while measuring `box_for`'s own
/// normal, in-flow content — see `measure`'s `BoxContent::Blocks` arm)
/// into the entries this box becomes the real, resolved containing
/// block for (returned, ready to attach as this box's own `Laid::
/// positioned`) vs. ones that still need to keep bubbling further up
/// (pushed onto `bubble_out` — the caller's own `pending` parameter,
/// continuing the same search one level further out): a `fixed` entry
/// *always* keeps bubbling, regardless of `position` (its containing
/// block is always the viewport, only ever resolved at the document
/// root — see `PendingPositioned::fixed`'s own docs); an `absolute`
/// entry stops the moment it reaches a box whose own `position` isn't
/// `Static` — the real CSS "nearest positioned ancestor" rule. `pad_x`/
/// `pad_y`/`pad_w`/`pad_h` is this box's own *padding* box (border box
/// minus its own border width on each side) — the real spec containing
/// block for an absolutely positioned descendant.
#[allow(clippy::too_many_arguments)]
fn resolve_or_bubble_positioned<'a>(
    own_pending: Vec<PendingPositioned<'a>>,
    position: Position,
    pad_x: f32,
    pad_y: f32,
    pad_w: f32,
    pad_h: f32,
    font_system: &mut FontSystem,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
    bubble_out: &mut Vec<PendingPositioned<'a>>,
) -> Vec<Laid> {
    let mut resolved = Vec::new();
    for item in own_pending {
        if item.fixed || position == Position::Static {
            bubble_out.push(item);
        } else {
            resolved.push(resolve_positioned(
                item, pad_x, pad_y, pad_w, pad_h, font_system, next_id, cache, deadline, incomplete,
            ));
        }
    }
    // Ascending `z-index` (`auto`/unset as `0`), ties in document order
    // (the order `own_pending` was built in, since `Vec::sort_by_key` is
    // a stable sort) — see `Laid::positioned`'s own docs for why this is
    // the paint/hit-test order, not a real stacking-context ordering.
    resolved.sort_by_key(|l| l.style.z_index.unwrap_or(0));
    resolved
}

/// Measures `item.b` for real against a now-known, concrete containing
/// block (`cb_x`/`cb_y`/`cb_w`/`cb_h`, absolute document-space, always
/// *definite* — unlike `measure`'s own `containing_height`, which can
/// be `None` for an ordinary in-flow box, this is only ever called once
/// a real positioned ancestor or the viewport has been found, so there
/// is always a concrete basis) and positions it against that block's
/// `top`/`right`/`bottom`/`left`.
///
/// **Solving left/top-vs-right/bottom, and the one real scope cut
/// here**: real CSS lets an author set both `left`+`right` (or `top`+
/// `bottom`) with `width`/`height` left `auto`, deriving the missing
/// dimension from the gap between them — not implemented; `left`/`top`
/// always wins when both are set (same precedence this file already
/// gives a cascade's `width` over a derived one elsewhere), and this
/// box's own width/height is always computed the ordinary content-
/// driven way against `cb_w`/`cb_h`, exactly like any in-flow box would
/// be (see TODO.txt). Measured once at a *provisional* position (`left`/
/// `top` if set, else its own `static_x`/`static_y` fallback) and then,
/// since only *now* is its real size known, shifted (`shift_laid_tree`,
/// no re-measuring) to its real final position if `right`/`bottom` ends
/// up being the side that actually matters (needs this box's own just-
/// measured `w`/`h` to resolve "right edge minus my own width").
///
/// `item.b`'s own nested `absolute`/`fixed` descendants (if `item.b`'s
/// `BoxContent` is `Blocks`) are resolved entirely within this one
/// `measure` call — `item.b` is itself non-`Static` by construction
/// (only ever `Absolute`/`Fixed`), so nothing can bubble past it; the
/// disposable, always-empty-on-return `Vec` handed to `measure` here
/// reflects exactly that.
#[allow(clippy::too_many_arguments)]
fn resolve_positioned(
    item: PendingPositioned,
    cb_x: f32,
    cb_y: f32,
    cb_w: f32,
    cb_h: f32,
    font_system: &mut FontSystem,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
) -> Laid {
    let s = item.b.style;
    let left = s.inset.left.map(|v| v.resolve(cb_w));
    let right = s.inset.right.map(|v| v.resolve(cb_w));
    let top = s.inset.top.map(|v| v.resolve(cb_h));
    let bottom = s.inset.bottom.map(|v| v.resolve(cb_h));
    let provisional_x = left.map(|l| cb_x + l).unwrap_or(item.static_x);
    let provisional_y = top.map(|t| cb_y + t).unwrap_or(item.static_y);
    let mut nested_pending = Vec::new();
    // Positioned content establishes its own fresh BFC (same category
    // as a float's own content/a table cell's interior — see `Floats`'s
    // own docs): outer floats don't narrow an absolutely/fixed
    // positioned box's own content, and it can't itself be a `width:
    // auto` float (the two are mutually exclusive — real CSS computes
    // `float` to `none` whenever `position` is `absolute`/`fixed`).
    let no_floats = Floats::default();
    let mut laid = measure(
        item.b, font_system, provisional_x, provisional_y, cb_w, Some(cb_h), next_id, cache, deadline, incomplete,
        &mut nested_pending, &no_floats, None,
    );
    debug_assert!(nested_pending.is_empty(), "an absolute/fixed box is itself always a positioned ancestor");
    let final_x = if left.is_some() {
        provisional_x
    } else if let Some(r) = right {
        cb_x + cb_w - r - laid.w
    } else {
        item.static_x
    };
    let final_y = if top.is_some() {
        provisional_y
    } else if let Some(b) = bottom {
        cb_y + cb_h - b - laid.h
    } else {
        item.static_y
    };
    let (dx, dy) = (final_x - laid.x, final_y - laid.y);
    shift_laid_tree(&mut laid, dx, dy);
    laid.fixed = item.fixed;
    laid
}

enum LaidContent {
    Blocks(Vec<Laid>),
    Text(usize),
    TableRow(Vec<Laid>),
    Image(Rc<DecodedImage>),
    /// A `BoxContent::Inline` run that contains at least one inline
    /// `<img>` — real inline-level flow, laid out by `measure_mixed_
    /// inline` instead of the single-`Buffer`-per-paragraph path
    /// `Text(usize)` uses for plain text (see its own docs for why
    /// these need an entirely different approach).
    MixedInline(Vec<MixedPositioned>),
}

/// One already-positioned item within a laid-out `MixedInline` run —
/// absolute document-space coordinates, same convention every other
/// `Laid`/position in this file uses (scroll is applied once, at paint
/// time). A `Word` is one shaped word, rendered via the same `cache`/
/// `paint_buffer` machinery `LaidContent::Text` uses — just one word
/// per cache entry instead of one whole paragraph (see `measure_mixed_
/// inline`'s own docs for why). An `Image` is a complete, ordinary
/// `Laid` (its `BoxContent::Image` shape, `x`/`y` already absolute) —
/// `paint`/`hit_test_cursor` just recurse into it exactly like any
/// other child `Laid`, getting full box-decoration/clipping fidelity
/// for free instead of a second, parallel painting path.
enum MixedPositioned {
    Word { id: usize, x: f32, y: f32, w: f32, h: f32 },
    Image(Box<Laid>),
}

/// A paragraph's shaped text, keyed by a stable id assigned during `measure`
/// in deterministic document order. Buffers live here — not inside `Laid` —
/// and persist across frames, so shaping a paragraph once is never repeated
/// (see `measure`'s budget/incomplete mechanism).
/// A paragraph's shaped `Buffer` plus every per-span vector something later
/// needs to paint or hit-test it correctly, parallel to each other and to
/// the original `spans` list by index (via cosmic-text's own per-glyph
/// `metadata` field, set in `shape_spans`): `(height, backgrounds, vertical
/// aligns, per-span "visibility: hidden", per-span cursor style, per-span
/// text-decoration style)`.
pub type ShapedParagraph = (
    Buffer,
    f32,
    Vec<Option<TextColor>>,
    Vec<VerticalAlign>,
    Vec<bool>,
    Vec<CursorStyle>,
    Vec<DecorationStyle>,
);

pub type ShapeCache = std::collections::HashMap<usize, ShapedParagraph>;

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

/// Shapes `items`' text spans as one cosmic-text paragraph — any
/// `InlineItem::Image` entries are silently skipped (never actually
/// happens: every caller already guarantees an all-`Text` slice, either
/// the whole run in `measure`'s plain-text fast path, or a single
/// synthesized one-word slice in `measure_mixed_inline`'s per-word
/// shaping — see its own docs for why that path shapes word-by-word
/// instead of calling this once over the whole mixed run).
fn shape_spans(
    font_system: &mut FontSystem,
    items: &[InlineItem],
    width: f32,
    pre: bool,
    align: Option<CosmicAlign>,
) -> ShapedParagraph {
    let spans: Vec<(&str, &SpanStyle)> = items
        .iter()
        .filter_map(|item| match item {
            InlineItem::Text(text, span) => Some((text.as_str(), span)),
            InlineItem::Image { .. } => None,
        })
        .collect();
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
            attrs.family = span.font_family.resolve(font_system.db_mut(), attrs.weight, attrs.style);
            attrs.text_decoration = span.text_decoration;
            attrs.metadata = i;
            attrs = attrs.metrics(Metrics::new(span.font_size, span.line_height));
            (*text, attrs)
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
    let cursors = spans.iter().map(|(_, span)| span.cursor).collect();
    let decoration_styles = spans.iter().map(|(_, span)| span.decoration_style).collect();
    (buffer, height, backgrounds, valigns, hidden, cursors, decoration_styles)
}

/// Clamps a box-model dimension (`measure`'s own `border_w`/`border_h` —
/// what margins/siblings stack against) to `min`/`max`, honoring
/// `box_sizing`'s choice of reference box: per spec, `width`/`min-width`/
/// `max-width` (and the height equivalents) all resolve against the
/// *same* box `box-sizing` names, so a single `extra` (2× border-width
/// plus the relevant padding pair) converts every one of them into
/// border-box terms uniformly via `to_box`. `max` is applied before
/// `min` — real CSS clamp order — so an author's `min > max` mistake
/// resolves to `min`, never something smaller than it. `auto_box` is
/// the border-box dimension to start from when `specified` is `None`
/// (`auto`): the container-derived width, or `extra + content_h` for
/// height — still subject to `min`/`max` either way, same as real CSS
/// (a `min-width` wider than the container forces overflow).
/// `specified`/`min`/`max` are `BoxLength`, not a plain `Option<f32>` —
/// callers are expected to have already finalized them (via
/// `finalize_box_lengths`) before reaching here, so `.px()` is always
/// safe (see `BoxLength`'s own docs on that invariant).
fn clamp_box_dim(
    specified: Option<BoxLength>,
    min: Option<BoxLength>,
    max: Option<BoxLength>,
    auto_box: f32,
    box_sizing: BoxSizing,
    extra: f32,
) -> f32 {
    let to_box = |raw: f32| match box_sizing {
        BoxSizing::ContentBox => raw.max(0.0) + extra,
        BoxSizing::BorderBox => raw.max(0.0),
    };
    let mut box_dim = specified.map(|v| to_box(v.px())).unwrap_or(auto_box);
    if let Some(mx) = max {
        box_dim = box_dim.min(to_box(mx.px()));
    }
    if let Some(mn) = min {
        box_dim = box_dim.max(to_box(mn.px()));
    }
    box_dim.max(0.0)
}

/// Resolves every `BoxLength`-typed field of `s` (margin/padding on all
/// four sides, `width`/`min-width`/`max-width`, `height`/`min-height`/
/// `max-height`) in place, against its real CSS percentage basis — see
/// `BoxLength`'s own docs for why this can't happen any earlier than
/// `measure`. `width`/margin/padding (all four sides, even top/bottom —
/// real CSS always resolves a margin/padding `%` against the containing
/// block's *width*) resolve against `width`: the containing block's
/// width, which every box always has a definite value for. `height`/
/// `min-height`/`max-height` resolve against `containing_height`
/// instead — the containing block's own *content* height, which is only
/// `Some` when that's itself definite. When it's `None` (the containing
/// block's height depends on its own content — the common case for an
/// ordinary block with no explicit `height`), a `%` height/min-height/
/// max-height resolves to `None` too: real CSS Level 2's documented rule
/// for exactly this case ("if the height of the containing block is not
/// specified explicitly... the [percentage] value computes to 'auto'"
/// for `height`; equivalent to no constraint at all for `min-`/`max-
/// height`, since this engine already floors every box at `0` and
/// treats an absent `max` as unconstrained regardless). After this call,
/// every one of these fields is `BoxLength::Px` — the invariant `.px()`
/// (and every paint/hit-test call site that relies on it) depends on.
fn finalize_box_lengths(s: &mut ComputedStyle, width: f32, containing_height: Option<f32>) {
    let resolve_h = |v: Option<BoxLength>| -> Option<f32> {
        v.and_then(|bl| match bl {
            BoxLength::Px(px) => Some(px),
            BoxLength::Percent(p) => containing_height.map(|ch| ch * p),
        })
    };
    s.margin = Sides {
        top: BoxLength::Px(s.margin.top.resolve(width)),
        right: BoxLength::Px(s.margin.right.resolve(width)),
        bottom: BoxLength::Px(s.margin.bottom.resolve(width)),
        left: BoxLength::Px(s.margin.left.resolve(width)),
    };
    s.padding = Sides {
        top: BoxLength::Px(s.padding.top.resolve(width)),
        right: BoxLength::Px(s.padding.right.resolve(width)),
        bottom: BoxLength::Px(s.padding.bottom.resolve(width)),
        left: BoxLength::Px(s.padding.left.resolve(width)),
    };
    s.width = s.width.map(|l| BoxLength::Px(l.resolve(width)));
    s.min_width = s.min_width.map(|l| BoxLength::Px(l.resolve(width)));
    s.max_width = s.max_width.map(|l| BoxLength::Px(l.resolve(width)));
    s.height = resolve_h(s.height).map(BoxLength::Px);
    s.min_height = resolve_h(s.min_height).map(BoxLength::Px);
    s.max_height = resolve_h(s.max_height).map(BoxLength::Px);
}

/// The box-model quantities every `measure` content arm needs in common
/// (and, since real auto table layout landed, that a table row also needs
/// when it's laid out directly by `measure_table`/`measure_row_with_
/// widths` instead of through `measure`'s own generic dispatch) — margin/
/// padding/border resolution, this box's own border-box width (`clamp_
/// box_dim` against the explicit/min/max width), and the content-box
/// origin/width/height-basis it hands down to its own content. Factored
/// out of `measure`'s own preamble (unchanged otherwise) so it can be
/// called a second, independent time for a table row without duplicating
/// this math — rows measured via `measure_table` never go through
/// `measure` itself at all (see its own docs for why).
struct BoxModelEntry {
    border_x: f32,
    border_y: f32,
    border_w: f32,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    extra_h: f32,
    child_containing_height: Option<f32>,
}

/// `auto_margin_box_override`, when `Some`, replaces `width` as the
/// basis for this box's own *auto*-width result only (`width`/margin/
/// padding's real `%` basis — resolved just above, via `finalize_box_
/// lengths` — is untouched, still `width`, the real containing block;
/// an *explicit* `s.width` is untouched too, since `clamp_box_dim`
/// already ignores `auto_border_w` whenever `specified` is `Some`). The
/// one caller that passes `Some` is `measure`'s own `BoxContent::Blocks`
/// arm, placing a `width: auto` float: real CSS computes a float's
/// width via shrink-to-fit (CSS2.1 §10.3.5), not "fill the containing
/// block" — the value passed in is that shrink-to-fit result, in
/// margin-box terms (see `measure`'s own docs for how it's computed),
/// converted to border-box terms here using the margins this call just
/// resolved, the same way `width`'s own ordinary margin-box-to-border-
/// box conversion works one line below.
fn box_model_entry(
    s: &mut ComputedStyle,
    x: f32,
    y: f32,
    width: f32,
    containing_height: Option<f32>,
    auto_margin_box_override: Option<f32>,
) -> BoxModelEntry {
    finalize_box_lengths(s, width, containing_height);
    let margin_top = s.margin.top.px();
    let margin_right = s.margin.right.px();
    let margin_left = s.margin.left.px();
    let padding_top = s.padding.top.px();
    let padding_right = s.padding.right.px();
    let padding_bottom = s.padding.bottom.px();
    let padding_left = s.padding.left.px();
    let border_x = x + margin_left;
    let border_y = y + margin_top;
    let extra_w = 2.0 * s.border_width + padding_left + padding_right;
    let auto_border_w = match auto_margin_box_override {
        Some(mb) => (mb - margin_left - margin_right).max(0.0),
        None => (width - margin_left - margin_right).max(0.0),
    };
    let border_w = clamp_box_dim(s.width, s.min_width, s.max_width, auto_border_w, s.box_sizing, extra_w);
    let content_x = border_x + s.border_width + padding_left;
    let content_y = border_y + s.border_width + padding_top;
    let content_w = (border_w - extra_w).max(0.0);
    let extra_h = 2.0 * s.border_width + padding_top + padding_bottom;
    let child_containing_height = s.height.map(|h| match s.box_sizing {
        BoxSizing::ContentBox => h.px().max(0.0),
        BoxSizing::BorderBox => (h.px() - extra_h).max(0.0),
    });
    BoxModelEntry { border_x, border_y, border_w, content_x, content_y, content_w, extra_h, child_containing_height }
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
///
/// `containing_height` is this box's own containing block's *content*
/// height, when that's definite (`None` otherwise — e.g. an ordinary
/// `height: auto` ancestor) — the basis `finalize_box_lengths` resolves
/// a `%` `height`/`min-height`/`max-height` against. There's no real
/// viewport/initial-containing-block concept here (unlike a real UA,
/// whose initial containing block always has the window's own definite
/// height) — this crate's own top-level caller passes `None`, see its
/// own docs for why. A `%` height resolves correctly against any
/// *explicit* ancestor height further down the tree regardless — this
/// parameter only ever starts `None` at the very top.
#[allow(clippy::too_many_arguments)]
pub fn measure<'a>(
    b: &'a LayoutBox,
    font_system: &mut FontSystem,
    x: f32,
    y: f32,
    width: f32,
    containing_height: Option<f32>,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
    // `position: absolute`/`fixed` descendants that bubbled all the way
    // up out of this box's own subtree without finding a positioned
    // ancestor to resolve against (see `PendingPositioned`'s own docs) —
    // appended to, never read; the caller (another `measure` call one
    // level up, or `measure_document` at the very top) is the one that
    // either catches them (if *it's* positioned) or keeps bubbling them
    // further still.
    pending: &mut Vec<PendingPositioned<'a>>,
    // Every float already placed in this box's own block formatting
    // context (BFC) — read-only here (grown into a local clone only by
    // the `BoxContent::Blocks` arm below, which is the only place that
    // can ever place a *new* float) — see `Floats`'s own docs for
    // exactly what counts as "this box's own BFC": the document root
    // (`measure_document`), a float's own content, `position: absolute`/
    // `fixed` content (`resolve_positioned`), and a table cell's own
    // interior (`lay_out_row_cells`) all start a *fresh*, empty `Floats`
    // here rather than forwarding an ancestor's; an ordinary recursive
    // `BoxContent::Blocks` child instead gets the current, already-grown
    // local clone, so floats are visible to every descendant inline
    // formatting context in the same BFC, not just immediate siblings.
    floats: &Floats,
    // `Some` only when this call is placing a `float: left`/`right`
    // child whose own `width` is `auto` — the shrink-to-fit margin-box
    // width `measure`'s own `BoxContent::Blocks` arm already computed
    // for it via `cell_natural_width` — see `box_model_entry`'s own
    // docs for exactly how this overrides the ordinary "auto width
    // fills the container" result. `None` for every other call
    // (everything that isn't itself a `width: auto` float).
    auto_margin_box_override: Option<f32>,
) -> Laid {
    // `display: table` builds its own box model exactly the same way as
    // any other box (`box_model_entry`, just below — reused, not
    // special-cased) but lays its *content* out completely differently:
    // real per-column sizing from its cells' own min/max content width,
    // not the generic `Blocks` dispatch every other block-level box
    // falls into. Its `BoxContent` is still plain `Blocks` (unchanged
    // from `build_box_for_element`'s own docs — there's no dedicated
    // `BoxContent::Table` variant, since the `Blocks` shape — a table
    // row stacked among any other direct block child like a `caption` —
    // is already exactly right), so this has to be checked before the
    // `match` below, not as one more arm inside it. See `measure_table`'s
    // own docs for the real algorithm. A table establishes its own BFC
    // (same category as a table cell — see `lay_out_row_cells`'s own
    // docs), so `floats`/`auto_margin_box_override` have nothing to do
    // with it either way; `measure_table` doesn't take either.
    if b.style.display == Display::Table
        && let BoxContent::Blocks(children) = &b.content
    {
        return measure_table(
            b, children, font_system, x, y, width, containing_height, next_id, cache, deadline, incomplete, pending,
        );
    }
    // `display: flex` — same shape of special-case as `Table` just
    // above, and for the same reason: its direct children (flex items)
    // need a table-wide/container-wide decision (each item's final
    // main-axis size depends on every sibling's own basis and the
    // container's own free space) before any one item can be measured,
    // which doesn't fit `measure`'s own generic per-child `Blocks`
    // dispatch below. See `measure_flex`'s own docs for the real
    // algorithm. A flex container establishes its own fresh BFC (same
    // category as a table — see `lay_out_row_cells`'s own docs), so
    // `floats`/`auto_margin_box_override` have nothing to do with it
    // either way; `measure_flex` doesn't take either.
    if b.style.display == Display::Flex
        && let BoxContent::Blocks(children) = &b.content
    {
        return measure_flex(
            b, children, font_system, x, y, width, containing_height, next_id, cache, deadline, incomplete, pending,
        );
    }
    let mut s = b.style;
    let entry = box_model_entry(&mut s, x, y, width, containing_height, auto_margin_box_override);
    let border_x = entry.border_x;
    let border_y = entry.border_y;
    let border_w = entry.border_w;
    let content_x = entry.content_x;
    let content_y = entry.content_y;
    let content_w = entry.content_w;
    let extra_h = entry.extra_h;
    let child_containing_height = entry.child_containing_height;

    let mut laid = match &b.content {
        BoxContent::Inline(items)
            if items.iter().any(|i| matches!(i, InlineItem::Image { .. }))
                || (!floats.is_empty() && content_y < floats.max_bottom()) =>
        {
            // Real inline-level flow for a run that mixes text with at
            // least one inline `<img>` — a materially different (and
            // much more expensive) path than the plain-text case below,
            // so only taken when actually needed; see `measure_mixed_
            // inline`'s own docs. Also taken (new — the `float`/`clear`
            // item) for a *plain-text* run that starts before every
            // float in this BFC has already ended, since only this
            // hand-rolled per-word breaker can narrow/shift individual
            // lines around a float — cosmic-text's own whole-buffer
            // `Buffer`/`Wrap` path below has one fixed width for the
            // entire paragraph. Known cost, not attempted away: once
            // *any* float exists anywhere earlier in this BFC, every
            // paragraph that starts before the *lowest* float's bottom
            // edge pays this slower per-word path, even one that turns
            // out not to actually overlap any specific float once its
            // own line positions are known — see TODO.txt.
            let (content_h, positioned) = measure_mixed_inline(
                items, &s, font_system, content_x, content_y, content_w, containing_height, next_id, cache, deadline,
                incomplete, floats,
            );
            let border_h = clamp_box_dim(
                s.height,
                s.min_height,
                s.max_height,
                extra_h + content_h.max(0.0),
                s.box_sizing,
                extra_h,
            );
            Laid {
                style: s,
                x: border_x,
                y: border_y,
                w: border_w,
                h: border_h,
                background_image: b.background_image.clone(),
                content: LaidContent::MixedInline(positioned),
                positioned: Vec::new(),
                fixed: false,
                sticky: None,
            }
        }
        BoxContent::Inline(items) => {
            let id = *next_id;
            *next_id += 1;

            let content_h = if let Some((_, h, _, _, _, _, _)) = cache.get(&id) {
                *h
            } else if items.is_empty() {
                cache.insert(
                    id,
                    (
                        Buffer::new_empty(Metrics::new(16.0, 20.0)),
                        0.0,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    ),
                );
                0.0
            } else if std::time::Instant::now() < deadline {
                let (buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles) = shape_spans(
                    font_system,
                    items,
                    content_w,
                    s.white_space == WhiteSpace::Pre,
                    s.text_align.to_cosmic(),
                );
                cache.insert(id, (buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles));
                h
            } else {
                *incomplete = true;
                PLACEHOLDER_LINE_HEIGHT
            };

            // An explicit `height` (and `min-height`/`max-height` —
            // see `clamp_box_dim`) overrides only this box's own
            // reported size (what its *parent* stacks against) — the
            // content above was already shaped in full against
            // `content_w` regardless, so it still paints completely,
            // just potentially spilling past this box if `content_h`
            // (the real content height) is taller than the clamped
            // result (real CSS `overflow: visible` default;
            // `overflow: hidden` on this element clips it correctly
            // already, see v40).
            let border_h = clamp_box_dim(
                s.height,
                s.min_height,
                s.max_height,
                extra_h + content_h.max(0.0),
                s.box_sizing,
                extra_h,
            );
            Laid {
                style: s,
                x: border_x,
                y: border_y,
                w: border_w,
                h: border_h,
                background_image: b.background_image.clone(),
                content: LaidContent::Text(id),
                positioned: Vec::new(),
                fixed: false,
                sticky: None,
            }
        }
        BoxContent::Blocks(children) => {
            let mut cursor_y = content_y;
            let mut laid_children = Vec::with_capacity(children.len());
            // Collected here, not measured here: a `position: absolute`/
            // `fixed` child is taken out of normal flow entirely — no
            // `measure` call (its real containing block isn't this
            // box's content box at all — see `PendingPositioned`'s own
            // docs), no `cursor_y` advance (the whole point of "out of
            // flow": every other sibling lands exactly where it would
            // if this child didn't exist).
            let mut own_pending: Vec<PendingPositioned<'a>> = Vec::new();
            // Grown locally as this loop places its own direct float
            // children, so a later sibling (and anything nested inside
            // an earlier non-floated sibling) sees them too — see
            // `Floats`'s own docs for why this is a clone of the
            // inherited set, not a fresh one.
            let mut local_floats = floats.clone();
            // A float's own `Laid` is collected here, not pushed
            // straight into `laid_children` at its own point in the
            // loop, and appended only once every normal sibling has
            // been — real CSS paints a float *above* every later
            // in-flow, non-positioned sibling's own background/border in
            // the same block formatting context (CSS2.1 Appendix E:
            // floats are their own stacking level, above ordinary
            // in-flow block-level boxes), which matters here because a
            // later normal sibling's own box is never narrowed to avoid
            // a float — only its *inline* content is (see this arm's own
            // docs) — so its full-width background can legitimately
            // overlap a float's own area, and must not visually bury it.
            // Confirmed with a real `grim` screenshot before adding this:
            // without it, a later opaque-background sibling painted
            // straight over most of an unrelated, non-cleared float.
            let mut float_children: Vec<Laid> = Vec::new();
            for child in children {
                if matches!(child.style.position, Position::Absolute | Position::Fixed) {
                    own_pending.push(PendingPositioned {
                        b: child,
                        fixed: child.style.position == Position::Fixed,
                        static_x: content_x,
                        static_y: cursor_y,
                    });
                    continue;
                }
                // `clear` (CSS2.1 §9.5.2): pushes the flow position down
                // past the bottom of any earlier float(s) on the named
                // side(s) in this same BFC, *before* this child (float
                // or not) is measured. `clear_y` returns `f32::MIN`
                // (never wins the `max`) when there's nothing to clear.
                if child.style.clear != Clear::None {
                    cursor_y = cursor_y.max(local_floats.clear_y(child.style.clear));
                }
                if child.style.float != Float::None {
                    // Real CSS takes a float out of normal block-level
                    // flow entirely (no `cursor_y` advance — the next
                    // sibling lands exactly where it would if this
                    // child didn't exist) but still places it at the
                    // *current* flow position, narrowed to whatever
                    // horizontal room earlier same-BFC floats leave at
                    // that exact `y` (`avail_bounds`'s own "single
                    // probe, not a height range" simplification — see
                    // its own docs). Not attempted here, a documented
                    // scope cut (see TODO.txt): if this float's own
                    // width doesn't actually fit in that narrowed
                    // corridor (competing same-side floats too wide for
                    // the container), it's placed here anyway rather
                    // than dropped down to retry below the obstruction
                    // — it may visually overlap a competing float.
                    let (left_bound, right_bound) =
                        local_floats.avail_bounds(cursor_y, content_x, content_x + content_w);
                    // A `width: auto` float uses shrink-to-fit sizing
                    // (CSS2.1 §10.3.5), not "fill the container" the
                    // way an ordinary `width: auto` block does — reused
                    // from the exact min/max-content machinery real
                    // auto table-column sizing already uses (`cell_
                    // natural_width`, despite its table-specific name,
                    // works on any box). Clamped against this box's own
                    // real containing-block width (`content_w`), *not*
                    // the narrowed avail corridor just computed above —
                    // a float's shrink-to-fit basis is the ordinary
                    // containing block, same as any other box; only its
                    // final *position* (below), never its size, adjusts
                    // to avoid other floats.
                    let auto_override = if child.style.width.is_none() {
                        let (min_mb, max_mb) =
                            cell_natural_width(child, font_system, next_id, cache, deadline, incomplete);
                        Some(content_w.min(max_mb).max(min_mb.min(content_w)))
                    } else {
                        None
                    };
                    // A float establishes its own, fresh BFC for its
                    // own content (see `Floats`'s own docs) — outer
                    // floats don't affect text wrapping *inside* it, and
                    // it can have floats of its own that don't escape
                    // back out either.
                    let empty_floats = Floats::default();
                    // Measured at `left_bound` first regardless of
                    // side — for `Right`, shifted into its real final
                    // position below (`shift_laid_tree`, no
                    // re-measuring) once its real width is known, the
                    // same provisional-then-shift trick `resolve_
                    // positioned` already uses for `right`/`bottom`.
                    let mut lc = measure(
                        child, font_system, left_bound, cursor_y, content_w, child_containing_height, next_id,
                        cache, deadline, incomplete, &mut own_pending, &empty_floats, auto_override,
                    );
                    let margin_top = lc.style.margin.top.px();
                    let margin_bottom = lc.style.margin.bottom.px();
                    let margin_right = lc.style.margin.right.px();
                    let margin_left = lc.style.margin.left.px();
                    if child.style.float == Float::Right {
                        let dx = right_bound - margin_right - (lc.x + lc.w);
                        shift_laid_tree(&mut lc, dx, 0.0);
                    }
                    let top = cursor_y;
                    let bottom = cursor_y + margin_top + lc.h + margin_bottom;
                    match child.style.float {
                        Float::Left => {
                            let edge = lc.x + lc.w + margin_right;
                            local_floats.left.push(FloatEntry { top, bottom, edge });
                        }
                        Float::Right => {
                            let edge = lc.x - margin_left;
                            local_floats.right.push(FloatEntry { top, bottom, edge });
                        }
                        Float::None => unreachable!(),
                    }
                    float_children.push(lc);
                    continue;
                }
                let lc = measure(
                    child, font_system, content_x, cursor_y, content_w, child_containing_height, next_id, cache,
                    deadline, incomplete, &mut own_pending, &local_floats, None,
                );
                cursor_y += lc.style.margin.top.px() + lc.style.margin.bottom.px() + lc.h;
                laid_children.push(lc);
            }
            // Appended last, after every normal sibling — see
            // `float_children`'s own docs just above for why paint
            // order (not just `cursor_y`/flow) needs this.
            laid_children.extend(float_children);
            // This loop's own content box is the real CSS containing
            // block for a `position: sticky` direct child (same basis
            // `relative`'s own `%` offsets already resolve against —
            // see `patch_sticky_containing_block`'s own docs) — only
            // knowable now that every sibling has been stacked
            // (`cursor_y`'s final value for this loop).
            patch_sticky_containing_block(&mut laid_children, content_y, cursor_y);
            // Real CSS: an ordinary block box's own auto height is
            // computed from its in-flow content only — a float does
            // *not* grow its containing block's height (the classic
            // "needs a clearfix" behavior; see TODO.txt) — so this is
            // still purely `cursor_y`, never adjusted by anything in
            // `local_floats`, exactly as before this feature existed.
            let content_h = cursor_y - content_y;
            // See the `Inline` branch's own comment just above — same
            // "override only the reported size, children already laid
            // out in full regardless" behavior.
            let border_h = clamp_box_dim(
                s.height,
                s.min_height,
                s.max_height,
                extra_h + content_h.max(0.0),
                s.box_sizing,
                extra_h,
            );
            // Only now — `border_h` just above is the first point this
            // box's own final size is known — can `own_pending` (this
            // box's *own* direct out-of-flow children, plus anything
            // that bubbled up from a `Static` descendant's `measure`
            // call above) be resolved against this box's padding box,
            // if it's eligible to be one (`s.position != Static`) —
            // otherwise every one of them keeps bubbling to `pending`,
            // this call's own out-parameter from its caller.
            let positioned_children = resolve_or_bubble_positioned(
                own_pending,
                s.position,
                border_x + s.border_width,
                border_y + s.border_width,
                (border_w - 2.0 * s.border_width).max(0.0),
                (border_h - 2.0 * s.border_width).max(0.0),
                font_system,
                next_id,
                cache,
                deadline,
                incomplete,
                pending,
            );
            Laid {
                style: s,
                x: border_x,
                y: border_y,
                w: border_w,
                h: border_h,
                background_image: b.background_image.clone(),
                content: LaidContent::Blocks(laid_children),
                positioned: positioned_children,
                fixed: false,
                sticky: None,
            }
        }
        BoxContent::TableRow(cells) => {
            // Defensive-only fallback: equal-width columns, same as this
            // file's table layout used everywhere before real per-column
            // sizing (`measure_table`/`table_column_widths`) landed. Only
            // reached if a `TableRow` box somehow gets measured outside
            // its own `Table` parent's `measure_table` dispatch —
            // `build_table_children` never actually produces one that
            // way, so this isn't real-world markup, same "stray markup,
            // best-effort" philosophy `build_table_children` itself
            // already documents.
            let n = cells.len().max(1) as f32;
            let col_widths = vec![content_w / n; cells.len()];
            let (row_h, laid_cells) = lay_out_row_cells(
                cells, &col_widths, font_system, content_x, content_y, child_containing_height, next_id, cache,
                deadline, incomplete, pending,
            );
            // Same explicit-`height`-overrides-only-the-reported-size
            // behavior as the `Inline`/`Blocks` branches above.
            let border_h = clamp_box_dim(
                s.height,
                s.min_height,
                s.max_height,
                extra_h + row_h.max(0.0),
                s.box_sizing,
                extra_h,
            );
            Laid {
                style: s,
                x: border_x,
                y: border_y,
                w: border_w,
                h: border_h,
                background_image: b.background_image.clone(),
                content: LaidContent::TableRow(laid_cells),
                positioned: Vec::new(),
                fixed: false,
                sticky: None,
            }
        }
        BoxContent::Image(image) => {
            // `s` is already finalized (against this very box's own
            // incoming `width`/`containing_height`, exactly the right
            // basis for a replaced element's own box) — `image_box_size`
            // just reads it, no further resolution needed.
            let (border_w, border_h) = image_box_size(image, &s);
            Laid {
                style: s,
                x: border_x,
                y: border_y,
                w: border_w,
                h: border_h,
                background_image: b.background_image.clone(),
                content: LaidContent::Image(Rc::clone(image)),
                positioned: Vec::new(),
                fixed: false,
                sticky: None,
            }
        }
    };
    // `position: relative` is the only kind that stays in normal flow
    // (see `apply_relative_offset`'s own docs) — applied once here,
    // after everything above (including any `positioned` out-of-flow
    // descendants just resolved against this box's own pre-shift
    // padding box), so a nested `absolute` descendant ends up anchored
    // to this box's real, shifted position. `width`/`containing_height`
    // (this call's own incoming parameters, not any `content_`-prefixed
    // local) are this box's own containing block — the right basis for
    // its own `top`/`right`/`bottom`/`left` percentages, same as `box_
    // model_entry` already uses for `margin`/`width`/`height`'s.
    apply_relative_offset(&mut laid, &s, width, containing_height);
    // `position: sticky` is the other kind that stays in normal flow —
    // captures this box's own (still-static, since the call above is a
    // no-op for anything but `Relative`) position plus its `top`/
    // `bottom` insets against the same containing-block basis; see
    // `capture_sticky_info`'s own docs for why its containing-block
    // *bounds* aren't resolved here too (the caller's own loop patches
    // those in immediately after, via `patch_sticky_containing_block`).
    capture_sticky_info(&mut laid, &s, containing_height);
    laid
}

/// A replaced element's border-box size from its intrinsic decoded size
/// and its own `ComputedStyle` — shared by a block-level `<img>`
/// (`BoxContent::Image`, above) and an inline one mixed into a text run
/// (`measure_mixed_inline`), same CSS replaced-element sizing algorithm
/// either way: `width`/`height` both `auto` uses the intrinsic size
/// directly; only one `auto` scales it from the other via the intrinsic
/// aspect ratio; both set stretches independently (default `object-fit:
/// fill`, needs no extra work since that's just what `clamp_box_dim`
/// already does for every other box). Doesn't re-derive the aspect
/// ratio after `min`/`max-width/height` clamp the other axis — a
/// further edge case nothing has needed yet.
/// Assumes `s` is already finalized (see `BoxLength`'s own docs) —
/// every caller either just finalized it itself (`measure`'s own `s`)
/// or finalized a fresh copy right before calling this
/// (`measure_mixed_inline`, for an inline `<img>`'s own `style`).
fn image_box_size(image: &DecodedImage, s: &ComputedStyle) -> (f32, f32) {
    let intrinsic_w = (image.width.max(1)) as f32;
    let intrinsic_h = (image.height.max(1)) as f32;
    let aspect = intrinsic_h / intrinsic_w;
    let extra_w = 2.0 * s.border_width + s.padding.left.px() + s.padding.right.px();
    let extra_h = 2.0 * s.border_width + s.padding.top.px() + s.padding.bottom.px();
    if s.width.is_none() && s.height.is_some() {
        let border_h = clamp_box_dim(s.height, s.min_height, s.max_height, extra_h + intrinsic_h, s.box_sizing, extra_h);
        let content_h = (border_h - extra_h).max(0.0);
        let auto_border_w = extra_w + content_h / aspect;
        let border_w = clamp_box_dim(s.width, s.min_width, s.max_width, auto_border_w, s.box_sizing, extra_w);
        (border_w, border_h)
    } else {
        let border_w = clamp_box_dim(s.width, s.min_width, s.max_width, extra_w + intrinsic_w, s.box_sizing, extra_w);
        let content_w = (border_w - extra_w).max(0.0);
        let auto_border_h = if s.height.is_none() { extra_h + content_w * aspect } else { extra_h + intrinsic_h };
        let border_h = clamp_box_dim(s.height, s.min_height, s.max_height, auto_border_h, s.box_sizing, extra_h);
        (border_w, border_h)
    }
}

/// An image's own natural (intrinsic) content width, for `cell_natural_
/// width`'s real-auto-table-layout column sizing — deliberately simpler
/// than `image_box_size`: an explicit fixed `width` (`BoxLength::Px`,
/// e.g. from an HTML `width=` attribute or `img { width: 200px }`)
/// overrides the intrinsic size, same as `image_box_size`'s own normal
/// case; a `%` width is left alone and treated as if unset, since there's
/// no containing-block width yet to resolve it against while this image
/// is itself *being used* to size one (the real spec's own rule for a
/// replaced element's intrinsic-size contribution: a percentage that
/// can't yet be resolved just doesn't contribute one). Doesn't honor an
/// explicit `height` driving the width via aspect ratio the way `image_
/// box_size` does — a narrower simplification than that function's own,
/// worth revisiting only if a real page's table column sizing ever looks
/// wrong because of it (see TODO.txt).
///
/// `s` must be the *un*-finalized `ComputedStyle` (straight off the
/// `LayoutBox`, before any `finalize_box_lengths` call) — finalizing
/// first would already have resolved a `%` width down to a plain `Px`
/// against whatever basis was used, indistinguishable from a real fixed
/// `Px` by the time it got here, defeating the whole point of this
/// function's `Px`-vs-anything-else check.
fn image_natural_width(image: &DecodedImage, s: &ComputedStyle) -> f32 {
    match s.width {
        Some(BoxLength::Px(v)) => v.max(1.0),
        _ => image.width.max(1) as f32,
    }
}

/// Real inline-level flow for a `BoxContent::Inline` run that mixes text
/// with at least one inline `<img>` — text genuinely shares a line with
/// the image and wraps around it, unlike a plain-text run (which hands
/// the whole paragraph to cosmic-text's own `Buffer`/`Wrap` in one call
/// and lets it decide line breaks). cosmic-text has no concept of an
/// embedded non-text box mid-paragraph (confirmed against how Firefox's
/// `nsLineLayout` actually handles this — see NOTES.md), so this hand-
/// rolls a simple greedy line-breaker instead: splits every text item
/// into words (`split_inclusive(' ')`, each word keeping its own
/// trailing space for wrapping purposes — text is already whitespace-
/// normalized by the time it reaches here, see `collect_children`'s own
/// docs), shapes each word as its own tiny one-span paragraph via the
/// existing `shape_spans`/`ShapeCache` machinery (full fidelity for
/// color/background/decoration/cursor per word, for free — see
/// `MixedPositioned`'s own docs for why), and treats an image as one
/// atomic token sized via the shared `image_box_size`. Each token gets
/// a stable `ShapeCache` id exactly like a whole paragraph would,
/// participating in the same `deadline`/`incomplete` incremental-
/// shaping budget — not a special case, a *shaped word* is just a
/// smaller unit of the same thing.
///
/// **Known simplifications, not attempted here** (see TODO.txt): no
/// cross-word kerning/ligatures (each word shaped in isolation); no
/// real bidi reordering across words (each word's own internal shaping
/// is correct, but RTL text spanning multiple words next to an image
/// would appear in the wrong visual order — cosmic-text's own bidi only
/// operates within one `Buffer`, and this path shapes one `Buffer` per
/// word); `white-space: pre` isn't honored in a mixed run (no special
/// handling of embedded `\n`/multiple spaces — words are always split
/// on plain spaces); an inline `<img>`'s vertical margin is ignored
/// (horizontal margin reserves line space correctly, matching real
/// inline-level box behavior, but doesn't affect line height); an
/// image is bottom-aligned within its line (an approximation of real
/// CSS's baseline alignment — the actual font-metrics baseline offset
/// isn't threaded out here); only `text-align: left/right/center` are
/// applied per line (`justify` falls back to `left` — no inter-word
/// space stretching); a line's trailing space (every word's measured
/// width includes its own trailing space) counts toward line width for
/// alignment purposes, unlike a real UA which hangs trailing whitespace
/// past the edge.
#[allow(clippy::too_many_arguments)]
fn measure_mixed_inline(
    items: &[InlineItem],
    s: &ComputedStyle,
    font_system: &mut FontSystem,
    content_x: f32,
    content_y: f32,
    content_w: f32,
    // This paragraph box's own containing-block content height (see
    // `measure`'s own docs) — an inline `<img>` lives directly inside
    // this box's content, sharing the *same* containing block as the
    // box itself (there's no intermediate box of its own the way a
    // `Blocks`/`TableRow` child has), so a `%` on its own `height` needs
    // this exact value, unmodified, not a derived "child" one.
    containing_height: Option<f32>,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
    // Floats active in this run's own block formatting context — the
    // `float`/`clear` item's real target: each *line* this greedy
    // breaker produces gets its own horizontal corridor, narrowed/
    // shifted by whatever float(s) overlap that line's own starting
    // `y` (`Floats::avail_bounds`), instead of the one fixed `content_x`
    // .. `content_x + content_w` every line used before this existed.
    floats: &Floats,
) -> (f32, Vec<MixedPositioned>) {
    // Effectively unconstrained — combined with `pre: true` below (which
    // also forces `Wrap::None`), a word is never wrapped/truncated
    // internally by cosmic-text regardless of this value; only used as
    // the buffer's nominal width.
    const WORD_MEASURE_WIDTH: f32 = 1_000_000.0;

    #[allow(clippy::large_enum_variant)]
    enum Measured {
        Word { id: usize, width: f32, height: f32 },
        Image { laid_style: ComputedStyle, background_image: Option<Rc<DecodedImage>>, image: Rc<DecodedImage>, advance_w: f32, img_w: f32, img_h: f32 },
    }

    let mut measured: Vec<Measured> = Vec::new();
    for item in items {
        match item {
            InlineItem::Text(text, span) => {
                for word in text.split_inclusive(' ') {
                    if word.is_empty() {
                        continue;
                    }
                    let id = *next_id;
                    *next_id += 1;
                    let (width, height) = if let Some((buffer, h, _, _, _, _, _)) = cache.get(&id) {
                        (buffer.layout_runs().next().map(|r| r.line_w).unwrap_or(0.0), *h)
                    } else if std::time::Instant::now() < deadline {
                        let synth = [InlineItem::Text(word.to_string(), *span)];
                        let (buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles) =
                            shape_spans(font_system, &synth, WORD_MEASURE_WIDTH, true, None);
                        let w = buffer.layout_runs().next().map(|r| r.line_w).unwrap_or(0.0);
                        cache.insert(id, (buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles));
                        (w, h)
                    } else {
                        *incomplete = true;
                        (0.0, PLACEHOLDER_LINE_HEIGHT)
                    };
                    measured.push(Measured::Word { id, width, height });
                }
            }
            InlineItem::Image { image, style, background_image } => {
                // `style` hasn't been through `measure`'s own
                // `finalize_box_lengths` call (that's the *paragraph*
                // box's `s`, not this image's own style) — finalize a
                // copy against this run's own width (`content_w`, the
                // image's containing block width) and the paragraph's
                // own `containing_height` (see this function's own
                // docs) before reading anything out of it.
                let mut finalized_style = *style;
                finalize_box_lengths(&mut finalized_style, content_w, containing_height);
                let (img_w, img_h) = image_box_size(image, &finalized_style);
                measured.push(Measured::Image {
                    laid_style: finalized_style,
                    background_image: background_image.clone(),
                    image: Rc::clone(image),
                    advance_w: finalized_style.margin.left.px() + img_w + finalized_style.margin.right.px(),
                    img_w,
                    img_h,
                });
            }
        }
    }

    // Resolves the horizontal corridor available to a line that starts
    // at document-space `content_y + y_rel` — `content_w` (the full
    // run's own containing-block-derived width) when `floats` is empty
    // or nothing overlaps, narrowed/shifted otherwise. A single probe at
    // the line's own *top* `y` (not checked across its real height
    // range, which isn't known until every item on it is chosen) — a
    // real but approximate simplification, same category as every other
    // scope cut this function's own docs already list.
    let line_bounds = |y_rel: f32| -> (f32, f32) {
        let (left_bound, right_bound) = floats.avail_bounds(content_y + y_rel, content_x, content_x + content_w);
        (left_bound, (right_bound - left_bound).max(0.0))
    };

    // Pass 1: greedy-wrap into lines — each entry is (index into
    // `measured`, x offset within its line, before `text-align`). Each
    // line also carries its own `(left_bound, avail_w)`, resolved once
    // when the line starts (via `line_bounds`, above) rather than
    // assuming the whole run's fixed `content_x`/`content_w` the way
    // this looked before floats existed.
    let mut lines: Vec<(f32, f32, Vec<(usize, f32)>)> = Vec::new();
    let mut current: Vec<(usize, f32)> = Vec::new();
    let mut cursor_x = 0.0f32;
    let mut cursor_y = 0.0f32;
    let (mut left_bound, mut avail_w) = line_bounds(cursor_y);
    for (i, m) in measured.iter().enumerate() {
        let w = match m {
            Measured::Word { width, .. } => *width,
            Measured::Image { advance_w, .. } => *advance_w,
        };
        if !current.is_empty() && cursor_x + w > avail_w {
            let line_height = current
                .iter()
                .map(|&(i, _)| match &measured[i] {
                    Measured::Word { height, .. } => *height,
                    Measured::Image { img_h, .. } => *img_h,
                })
                .fold(0.0f32, f32::max);
            lines.push((left_bound, avail_w, std::mem::take(&mut current)));
            cursor_y += line_height;
            cursor_x = 0.0;
            let bounds = line_bounds(cursor_y);
            left_bound = bounds.0;
            avail_w = bounds.1;
        }
        current.push((i, cursor_x));
        cursor_x += w;
    }
    if !current.is_empty() {
        lines.push((left_bound, avail_w, current));
    }

    // Pass 2: resolve each line's height/`text-align` offset, emit
    // final absolute (document-space) positions.
    let mut positioned: Vec<MixedPositioned> = Vec::with_capacity(measured.len());
    let mut cursor_y = 0.0f32;
    for (left_bound, avail_w, line) in &lines {
        let line_height = line
            .iter()
            .map(|&(i, _)| match &measured[i] {
                Measured::Word { height, .. } => *height,
                Measured::Image { img_h, .. } => *img_h,
            })
            .fold(0.0f32, f32::max);
        let line_w = line
            .last()
            .map(|&(i, x)| {
                x + match &measured[i] {
                    Measured::Word { width, .. } => *width,
                    Measured::Image { advance_w, .. } => *advance_w,
                }
            })
            .unwrap_or(0.0);
        let align_offset = match s.text_align {
            TextAlign::Right => (avail_w - line_w).max(0.0),
            TextAlign::Center => (avail_w - line_w).max(0.0) / 2.0,
            TextAlign::Left | TextAlign::Justify => 0.0,
        };
        for &(i, x) in line {
            let final_x = left_bound + x + align_offset;
            match &measured[i] {
                Measured::Word { id, width, height } => {
                    positioned.push(MixedPositioned::Word { id: *id, x: final_x, y: content_y + cursor_y, w: *width, h: *height });
                }
                Measured::Image { laid_style, background_image, image, img_w, img_h, .. } => {
                    positioned.push(MixedPositioned::Image(Box::new(Laid {
                        style: *laid_style,
                        x: final_x + laid_style.margin.left.px(),
                        y: content_y + cursor_y + (line_height - img_h),
                        w: *img_w,
                        h: *img_h,
                        background_image: background_image.clone(),
                        content: LaidContent::Image(Rc::clone(image)),
                        positioned: Vec::new(),
                        fixed: false,
                        sticky: None,
                    })));
                }
            }
        }
        cursor_y += line_height;
    }

    (cursor_y, positioned)
}

/// The furthest right edge any item in a trial `measure_mixed_inline`
/// call reached — i.e. the actual content width that run needed, as
/// opposed to the `content_w` it was *given* (which `measure_mixed_
/// inline` itself never reports back directly, since its normal callers
/// already know their own box's width and only need line positions).
/// Used only by `cell_natural_width`'s own trial calls, both of which
/// pass `content_x: 0.0`, so an item's absolute `x` is already relative
/// to the run's own start.
fn mixed_inline_extent(positioned: &[MixedPositioned]) -> f32 {
    positioned
        .iter()
        .map(|p| match p {
            MixedPositioned::Word { x, w, .. } => x + w,
            MixedPositioned::Image(laid) => laid.x + laid.w,
        })
        .fold(0.0f32, f32::max)
}

/// This cell's own natural (intrinsic) content width range — the `(min,
/// max)` pair CSS2.1 §17.5.2.2's automatic table layout bases column
/// sizing on (see `table_column_widths`). `max` is this cell's content
/// laid out with no wrapping at all (its width if it had unlimited
/// space); `min` is the width of its single widest unbreakable token
/// (the narrowest it could ever be wrapped to without breaking a word or
/// shrinking an image). Both figures are border-box, including this
/// cell's own margin/border/padding, not just its content — the real
/// horizontal space it needs in its row.
///
/// Reuses `measure_mixed_inline`'s existing greedy word-wrapper for
/// *both* figures, run at its two extremes, rather than a separate
/// whole-buffer `shape_spans` call for `max` and a hand-rolled "longest
/// word" scan for `min`: calling it at an effectively unconstrained
/// width (`max`) leaves every word/image on one line, so the furthest
/// item's own right edge *is* the unwrapped width; calling it at `0`
/// width (`min`) forces every word/image onto its own line (the
/// wrapper's own `cursor_x + w > content_w` check is always true for a
/// second item on an empty-budget line), so the widest single resulting
/// line *is* the widest unbreakable token. This also means a cell whose
/// content mixes text and an inline `<img>` gets the same, consistent
/// treatment as a plain-text one, with no separate code path — see
/// `measure_mixed_inline`'s own docs for why one hand-rolled wrapper
/// already has to serve both shapes.
///
/// Participates in the same `deadline`/`cache`/`next_id` incremental-
/// shaping budget as everything else `measure` shapes (each trial call
/// shapes real words, just mapped into different lines) — a huge table
/// degrades the same resumable way a huge paragraph already does, not a
/// separate, unbounded pass.
fn cell_natural_width(
    cell: &LayoutBox,
    font_system: &mut FontSystem,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
) -> (f32, f32) {
    // Effectively unconstrained, same constant/reasoning as `measure_
    // mixed_inline`'s own `WORD_MEASURE_WIDTH`.
    const UNBOUNDED: f32 = 1_000_000.0;
    let raw_width = cell.style.width;
    let mut s = cell.style;
    // There's no real containing-block width yet — the whole point of
    // this pass is to find out how much width this cell *wants*, before
    // any column width exists to resolve a `%` margin/padding/border
    // against. `0.0` is CSS2.1's own fallback basis for exactly this
    // situation (a percentage that can't yet be resolved against a real
    // basis computes as if the basis were `0`), same treatment `image_
    // natural_width` gives a `%` width for the same underlying reason.
    finalize_box_lengths(&mut s, 0.0, None);
    let extra_w =
        s.margin.left.px() + s.margin.right.px() + 2.0 * s.border_width + s.padding.left.px() + s.padding.right.px();
    // Intrinsic sizing cares about the content only, never the author's
    // chosen alignment — force `Left` so a trial call's own `text-align:
    // center`/`right` offset (computed against *this trial's* `content_w`,
    // not any real one) can't inflate the measured extent.
    let mut trial_style = s;
    trial_style.text_align = TextAlign::Left;
    let (content_min, content_max) = match &cell.content {
        BoxContent::Inline(items) => {
            // No real containing block exists yet for this trial (see
            // this function's own docs) — same reasoning extends to
            // floats: there's no real BFC to inherit from either, so
            // both trials use a fresh, empty one.
            let no_floats = Floats::default();
            let (_, min_items) = measure_mixed_inline(
                items, &trial_style, font_system, 0.0, 0.0, 0.0, None, next_id, cache, deadline, incomplete,
                &no_floats,
            );
            let (_, max_items) = measure_mixed_inline(
                items, &trial_style, font_system, 0.0, 0.0, UNBOUNDED, None, next_id, cache, deadline, incomplete,
                &no_floats,
            );
            (mixed_inline_extent(&min_items), mixed_inline_extent(&max_items))
        }
        // A nested block stacks its children vertically — its own
        // natural width is the *widest* child, not their sum.
        BoxContent::Blocks(children) => children.iter().fold((0.0f32, 0.0f32), |(mn, mx), child| {
            let (cmn, cmx) = cell_natural_width(child, font_system, next_id, cache, deadline, incomplete);
            (mn.max(cmn), mx.max(cmx))
        }),
        // Only reached for a nested `<table>`'s own rows (never a real
        // table cell's direct content — see `build_table_row_cells`): a
        // row lays its own cells out horizontally, so its natural width
        // is their *sum* — composes correctly with the `Blocks` arm
        // above (a nested table's own natural width ends up as the max
        // over its rows of each row's own cells' summed width, a
        // reasonable approximation of running this same algorithm
        // recursively without actually doing so).
        BoxContent::TableRow(cells) => cells.iter().fold((0.0f32, 0.0f32), |(mn, mx), c| {
            let (cmn, cmx) = cell_natural_width(c, font_system, next_id, cache, deadline, incomplete);
            (mn + cmn, mx + cmx)
        }),
        BoxContent::Image(image) => {
            let w = image_natural_width(image, &ComputedStyle { width: raw_width, ..s });
            (w, w)
        }
    };
    (content_min + extra_w, content_max + extra_w)
}

/// The real per-column widths `measure_table` distributes a table's own
/// `content_w` into — CSS2.1 §17.5.2.2's automatic table layout, scoped
/// to what matters without colspan/rowspan (a row's `n`th cell always
/// maps to column `n`; a row with fewer cells than the table's column
/// count just leaves its later columns unfilled, handled already by
/// `lay_out_row_cells`'s own fallback). Each column's own `min`/`max` is
/// the largest of its cells' own `cell_natural_width` across every row —
/// a column has to be wide enough for its widest cell, in either
/// direction. Then:
///
/// - if every column's `max` fits in `content_w`, each grows
///   proportionally to its own share of the total `max` to fill the rest
///   (an auto-width table still fills its own box-model width exactly as
///   it always has — see `measure_table`'s own docs — so the extra space
///   has to go *somewhere*, and growing by `max`-share is what a real
///   `table-layout: auto` table does once content fits);
/// - else if even every column's `min` doesn't fit, each shrinks
///   proportionally to its own share of the total `min` instead, letting
///   the real content overflow rather than clamping to something
///   smaller still (no different from a `min-width` wider than its own
///   container elsewhere in this engine);
/// - otherwise (the ordinary case: `content_w` sits between the table's
///   total `min` and total `max`) each column interpolates between its
///   own `min` and `max`, in proportion to how much of the table's total
///   `max - min` "slack" it represents — the real spec's own rule for
///   this case.
fn table_column_widths(
    children: &[LayoutBox],
    content_w: f32,
    font_system: &mut FontSystem,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
) -> Vec<f32> {
    let num_cols = children
        .iter()
        .filter_map(|c| match &c.content {
            BoxContent::TableRow(cells) => Some(cells.len()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    if num_cols == 0 {
        return Vec::new();
    }
    let mut col_min = vec![0.0f32; num_cols];
    let mut col_max = vec![0.0f32; num_cols];
    for child in children {
        if let BoxContent::TableRow(cells) = &child.content {
            for (i, cell) in cells.iter().enumerate() {
                let (min_w, max_w) = cell_natural_width(cell, font_system, next_id, cache, deadline, incomplete);
                col_min[i] = col_min[i].max(min_w);
                // A column's `max` is never smaller than its own `min`
                // (can happen per-cell if a trial measurement landed on
                // an incomplete/placeholder shape under the deadline
                // budget — see `cell_natural_width`'s own docs) so the
                // distribution math below never has to handle `max <
                // min` for a single column.
                col_max[i] = col_max[i].max(max_w).max(col_min[i]);
            }
        }
    }
    let total_min: f32 = col_min.iter().sum();
    let total_max: f32 = col_max.iter().sum();

    if total_max <= content_w {
        if total_max <= 0.0 {
            return vec![content_w / num_cols as f32; num_cols];
        }
        let extra = content_w - total_max;
        col_max.iter().map(|&m| m + extra * (m / total_max)).collect()
    } else if total_min >= content_w {
        if total_min <= 0.0 {
            return vec![content_w / num_cols as f32; num_cols];
        }
        col_min.iter().map(|&m| content_w * (m / total_min)).collect()
    } else {
        let total_slack = total_max - total_min;
        (0..num_cols)
            .map(|i| {
                let slack = col_max[i] - col_min[i];
                col_min[i] + (content_w - total_min) * (slack / total_slack)
            })
            .collect()
    }
}

/// Lays a table row's cells out left-to-right at exactly `col_widths`
/// (one entry per cell — an equal share of the total if there are more
/// cells than entries, a defensive fallback that shouldn't come up in
/// practice since `table_column_widths`'s own `num_cols` is already the
/// *widest* row's cell count). Shared by `measure`'s own defensive
/// stray-`TableRow` fallback (equal-width `col_widths`) and `measure_row_
/// with_widths`'s real, column-sized case, so the two don't duplicate
/// the cell loop/row-height bookkeeping.
/// `pending` is simply forwarded to each cell's own `measure` call,
/// never caught here: per `TODO.txt`, a `<tr>`/`<td>` can't itself be a
/// "nearest positioned ancestor" in this engine, regardless of what
/// `position` it might declare — any `position: absolute`/`fixed` box
/// bubbling up from inside a cell's own content keeps bubbling straight
/// through the row/table structure to whatever real positioned
/// ancestor (if any) lies *outside* the table entirely.
#[allow(clippy::too_many_arguments)]
fn lay_out_row_cells<'a>(
    cells: &'a [LayoutBox],
    col_widths: &[f32],
    font_system: &mut FontSystem,
    content_x: f32,
    content_y: f32,
    child_containing_height: Option<f32>,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
    pending: &mut Vec<PendingPositioned<'a>>,
) -> (f32, Vec<Laid>) {
    let fallback_w = if col_widths.is_empty() { 0.0 } else { col_widths.iter().sum::<f32>() / col_widths.len() as f32 };
    let mut cursor_x = content_x;
    let mut row_h = 0.0f32;
    let mut laid_cells = Vec::with_capacity(cells.len());
    // A table cell's own interior is its own fresh BFC (same category
    // as a float's own content/positioned content — see `Floats`'s own
    // docs): floats don't escape a cell's own box, in either direction.
    let no_floats = Floats::default();
    for (i, cell) in cells.iter().enumerate() {
        let cell_w = col_widths.get(i).copied().unwrap_or(fallback_w);
        let lc = measure(
            cell, font_system, cursor_x, content_y, cell_w, child_containing_height, next_id, cache, deadline,
            incomplete, pending, &no_floats, None,
        );
        row_h = row_h.max(lc.style.margin.top.px() + lc.style.margin.bottom.px() + lc.h);
        cursor_x += cell_w;
        laid_cells.push(lc);
    }
    (row_h, laid_cells)
}

/// A `TableRow` box's own box model (`box_model_entry`, exactly like any
/// other box) plus its cells, laid out at `col_widths` instead of an
/// equal split — called directly by `measure_table` instead of through
/// `measure`'s own generic dispatch, since a row measured this way needs
/// the table's real per-column widths, which only `measure_table` (its
/// own `Table` parent) has computed.
#[allow(clippy::too_many_arguments)]
fn measure_row_with_widths<'a>(
    row: &'a LayoutBox,
    col_widths: &[f32],
    font_system: &mut FontSystem,
    x: f32,
    y: f32,
    width: f32,
    containing_height: Option<f32>,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
    pending: &mut Vec<PendingPositioned<'a>>,
) -> Laid {
    let BoxContent::TableRow(cells) = &row.content else {
        unreachable!("measure_row_with_widths is only ever called on a BoxContent::TableRow box, from measure_table");
    };
    let mut s = row.style;
    let entry = box_model_entry(&mut s, x, y, width, containing_height, None);
    let (row_h, laid_cells) = lay_out_row_cells(
        cells, col_widths, font_system, entry.content_x, entry.content_y, entry.child_containing_height, next_id,
        cache, deadline, incomplete, pending,
    );
    let border_h =
        clamp_box_dim(s.height, s.min_height, s.max_height, entry.extra_h + row_h.max(0.0), s.box_sizing, entry.extra_h);
    Laid {
        style: s,
        x: entry.border_x,
        y: entry.border_y,
        w: entry.border_w,
        h: border_h,
        background_image: row.background_image.clone(),
        content: LaidContent::TableRow(laid_cells),
        positioned: Vec::new(),
        fixed: false,
        sticky: None,
    }
}

/// A `display: table` element's own box model (`box_model_entry`, same
/// as any other box) plus its content: real CSS2.1 §17.5.2.2 automatic
/// table layout (`table_column_widths`), not the old equal-width-column
/// simplification (see `TODO.txt`'s previous wording for this item).
/// `children` is the table's own direct `Blocks` children — `TableRow`s
/// plus any plain-`Block` content like a `caption` — exactly what
/// `build_table_children` already produces; this table's own box-model
/// `width` (and therefore its `content_w`, what `table_column_widths`
/// distributes) is still derived the ordinary `clamp_box_dim` way, same
/// as before this feature existed — real CSS auto-width tables actually
/// shrink-to-fit their own content rather than filling their container,
/// but changing *that* is a separate, bigger scope than "size the
/// columns from their content" (this item's actual ask — see TODO.txt),
/// so an auto-width table still fills its container exactly as it did
/// with equal-width columns, just divided unevenly among them now.
///
/// Stacks `children` vertically exactly like `BoxContent::Blocks`
/// always has — a plain-`Block` child (e.g. a `caption`) via the
/// ordinary `measure`, a `TableRow` child via `measure_row_with_widths`
/// instead, passing the real per-column widths rather than an equal
/// split.
///
/// No colspan/rowspan, no border-collapse/cell borders, no
/// `caption-side` — all still explicitly out of scope, see `TODO.txt`.
#[allow(clippy::too_many_arguments)]
fn measure_table<'a>(
    b: &'a LayoutBox,
    children: &'a [LayoutBox],
    font_system: &mut FontSystem,
    x: f32,
    y: f32,
    width: f32,
    containing_height: Option<f32>,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
    pending: &mut Vec<PendingPositioned<'a>>,
) -> Laid {
    let mut s = b.style;
    let entry = box_model_entry(&mut s, x, y, width, containing_height, None);
    let col_widths = table_column_widths(children, entry.content_w, font_system, next_id, cache, deadline, incomplete);

    let mut cursor_y = entry.content_y;
    let mut laid_children = Vec::with_capacity(children.len());
    // A direct `<table>` child (a row, or a plain-`Block` child like a
    // `caption`) being itself `position: absolute`/`fixed` isn't
    // supported — same narrow, documented scope cut as a `<tr>`/`<td>`
    // itself declaring `position` (see `lay_out_row_cells`'s own docs)
    // — so, unlike the generic `Blocks` arm, nothing here checks `child.
    // style.position` before measuring it in place. Content *inside* a
    // row's cells is unaffected (see `lay_out_row_cells`): this table
    // still becomes the real containing block for an `absolute`
    // descendant buried inside a cell, exactly like any other
    // `position`-ed ancestor — only `own_pending` catches that here,
    // just like the generic `Blocks` arm.
    let mut own_pending: Vec<PendingPositioned<'a>> = Vec::new();
    // A table is its own fresh BFC (see `Floats`'s own docs) — a direct
    // `<table>` child (a row, or a plain-`Block` child like a
    // `caption`) never needs to see floats from outside the table.
    let no_floats = Floats::default();
    for child in children {
        let lc = if matches!(child.content, BoxContent::TableRow(_)) {
            measure_row_with_widths(
                child, &col_widths, font_system, entry.content_x, cursor_y, entry.content_w,
                entry.child_containing_height, next_id, cache, deadline, incomplete, &mut own_pending,
            )
        } else {
            measure(
                child, font_system, entry.content_x, cursor_y, entry.content_w, entry.child_containing_height,
                next_id, cache, deadline, incomplete, &mut own_pending, &no_floats, None,
            )
        };
        cursor_y += lc.style.margin.top.px() + lc.style.margin.bottom.px() + lc.h;
        laid_children.push(lc);
    }
    // Same patch `measure`'s own `Blocks` arm does, for a direct
    // `<table>` child (e.g. a `caption`) that's itself `position:
    // sticky` — a `<tr>`/`<td>` itself declaring `position` still isn't
    // supported at all (same pre-existing scope cut this function's own
    // docs already note for `absolute`/`fixed`).
    patch_sticky_containing_block(&mut laid_children, entry.content_y, cursor_y);
    let content_h = cursor_y - entry.content_y;
    let border_h = clamp_box_dim(
        s.height,
        s.min_height,
        s.max_height,
        entry.extra_h + content_h.max(0.0),
        s.box_sizing,
        entry.extra_h,
    );
    let positioned_children = resolve_or_bubble_positioned(
        own_pending,
        s.position,
        entry.border_x + s.border_width,
        entry.border_y + s.border_width,
        (entry.border_w - 2.0 * s.border_width).max(0.0),
        (border_h - 2.0 * s.border_width).max(0.0),
        font_system,
        next_id,
        cache,
        deadline,
        incomplete,
        pending,
    );
    let mut laid = Laid {
        style: s,
        x: entry.border_x,
        y: entry.border_y,
        w: entry.border_w,
        h: border_h,
        background_image: b.background_image.clone(),
        content: LaidContent::Blocks(laid_children),
        positioned: positioned_children,
        fixed: false,
        sticky: None,
    };
    apply_relative_offset(&mut laid, &s, width, containing_height);
    capture_sticky_info(&mut laid, &s, containing_height);
    laid
}

/// One flex item's resolved sizing inputs for `resolve_flex_main_sizes`
/// — border-box, margin excluded (same convention `measure`'s own
/// `cursor_y`/`Laid::h` bookkeeping already uses throughout this file:
/// margin is always a separate figure, added on top, never folded into
/// a box's own reported size).
struct FlexBasisInfo {
    basis: f32,
    grow: f32,
    shrink: f32,
    min: f32,
    max: f32,
}

/// CSS Flexible Box Layout's "resolving flexible lengths" step (CSS
/// Flexbox Level 1 §9.7), scoped to a single distribution pass — not
/// the real spec's iterative algorithm, which freezes any item whose
/// flexed size would violate its own `min`/`max` and re-distributes the
/// remaining free space again among the rest. `items[i].basis` (not
/// `main_size`) is the proportional weight for *shrinking*
/// (`flex_shrink * basis`, the real spec's own "scaled shrink factor"
/// formula — a larger item shrinks by more of the deficit, not an equal
/// share), and a straight `grow`-weighted share of the surplus for
/// *growing*. Each result is clamped to its own `[min, max]` — but,
/// since this is a single pass, a clamped item's own under/overshoot
/// isn't redistributed a second time among its unclamped siblings (see
/// `TODO.txt`) — fine for the common case (no unusually tight `min-`/
/// `max-width`/`height` relative to an item's own basis); a real
/// divergence only once several siblings' constraints interact.
fn resolve_flex_main_sizes(main_size: f32, gap: f32, items: &[FlexBasisInfo]) -> Vec<f32> {
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    let sum_basis: f32 = items.iter().map(|i| i.basis).sum();
    let free = main_size - sum_basis - gap * (n - 1) as f32;
    let mut sizes: Vec<f32> = if free > 0.0 {
        let sum_grow: f32 = items.iter().map(|i| i.grow).sum();
        if sum_grow > 0.0 {
            items.iter().map(|i| i.basis + free * (i.grow / sum_grow)).collect()
        } else {
            items.iter().map(|i| i.basis).collect()
        }
    } else if free < 0.0 {
        let sum_scaled: f32 = items.iter().map(|i| i.shrink * i.basis).sum();
        if sum_scaled > 0.0 {
            items
                .iter()
                .map(|i| (i.basis - free.abs() * (i.shrink * i.basis / sum_scaled)).max(0.0))
                .collect()
        } else {
            items.iter().map(|i| i.basis).collect()
        }
    } else {
        items.iter().map(|i| i.basis).collect()
    };
    for (size, item) in sizes.iter_mut().zip(items) {
        *size = size.clamp(item.min, item.max.max(item.min));
    }
    sizes
}

/// `justify-content`'s main-axis distribution — shared by `measure_
/// flex_row`/`measure_flex_column`, since the formula is identical
/// regardless of which physical axis is "main." Returns `(start_offset,
/// extra_between)`: `start_offset` is added once, before the first
/// item's own margin edge; `extra_between` is added once between each
/// consecutive pair, *on top of* the caller's own declared `gap` — this
/// function never sees `gap` at all (the caller already subtracted it
/// out of `leftover` before calling), so it only ever computes the
/// *additional* space `justify-content` itself contributes.
fn distribute_justify_content(leftover: f32, n: usize, justify: JustifyContent) -> (f32, f32) {
    if n == 0 || leftover <= 0.0 {
        return (0.0, 0.0);
    }
    match justify {
        JustifyContent::FlexStart => (0.0, 0.0),
        JustifyContent::FlexEnd => (leftover, 0.0),
        JustifyContent::Center => (leftover / 2.0, 0.0),
        JustifyContent::SpaceBetween if n > 1 => (0.0, leftover / (n - 1) as f32),
        JustifyContent::SpaceBetween => (0.0, 0.0),
        JustifyContent::SpaceAround => {
            let extra = leftover / n as f32;
            (extra / 2.0, extra)
        }
        JustifyContent::SpaceEvenly => {
            let extra = leftover / (n + 1) as f32;
            (extra, extra)
        }
    }
}

/// A `display: flex` element's own box model (`box_model_entry`, same
/// as any other box) plus its content: real CSS Flexible Box Layout
/// (https://www.w3.org/TR/css-flexbox-1/), scoped to the common "flex
/// row/column of items, growing/shrinking/justified/aligned" case — see
/// `measure_flex_row`/`measure_flex_column`'s own docs for exactly what
/// each direction implements, and `TODO.txt` for what's deliberately
/// cut (no `align-content`, no `order`, real baseline alignment
/// approximated as `flex-start`, `flex-wrap` only real for `row`/
/// `row-reverse`, no automatic-minimum-size/iterative constraint
/// resolution, and an item's own explicit `width`/`flex-basis` on a
/// *row* container can win over a flex-grown/shrunk size in one narrow
/// case — see `measure_flex_row`'s own docs — the same accepted
/// simplification `measure_table`'s own column-width-vs-cell-width gap
/// already has).
///
/// `position: absolute`/`fixed` children are taken out of the flex
/// algorithm entirely (real CSS §4: they're not flex items at all) —
/// the same split `measure`'s own `BoxContent::Blocks` arm already
/// makes for an ordinary block's out-of-flow children, anchored against
/// this box's own padding box exactly the same way. A flex container is
/// its own fresh BFC (same category as a table — see `lay_out_row_
/// cells`'s own docs): floats never escape into or out of it.
#[allow(clippy::too_many_arguments)]
fn measure_flex<'a>(
    b: &'a LayoutBox,
    children: &'a [LayoutBox],
    font_system: &mut FontSystem,
    x: f32,
    y: f32,
    width: f32,
    containing_height: Option<f32>,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
    pending: &mut Vec<PendingPositioned<'a>>,
) -> Laid {
    let mut s = b.style;
    let entry = box_model_entry(&mut s, x, y, width, containing_height, None);
    let no_floats = Floats::default();
    let mut own_pending: Vec<PendingPositioned<'a>> = Vec::new();
    let mut items: Vec<&'a LayoutBox> = Vec::with_capacity(children.len());
    for child in children {
        if matches!(child.style.position, Position::Absolute | Position::Fixed) {
            own_pending.push(PendingPositioned {
                b: child,
                fixed: child.style.position == Position::Fixed,
                static_x: entry.content_x,
                static_y: entry.content_y,
            });
            continue;
        }
        items.push(child);
    }
    let is_row = s.flex_direction.is_row();
    let main_gap = if is_row { s.column_gap } else { s.row_gap };
    let cross_gap = if is_row { s.row_gap } else { s.column_gap };
    let reverse = s.flex_direction.is_reverse();
    let mut laid_children = if is_row {
        measure_flex_row(
            &items, &s, &entry, font_system, next_id, cache, deadline, incomplete, &mut own_pending, &no_floats,
            reverse, s.flex_wrap, main_gap, cross_gap,
        )
    } else {
        measure_flex_column(
            &items, &s, &entry, font_system, next_id, cache, deadline, incomplete, &mut own_pending, &no_floats,
            reverse, main_gap,
        )
    };
    let bottom_extent = laid_children
        .iter()
        .map(|lc| lc.y + lc.style.margin.bottom.px() + lc.h)
        .fold(entry.content_y, f32::max);
    patch_sticky_containing_block(&mut laid_children, entry.content_y, bottom_extent);
    let content_h = bottom_extent - entry.content_y;
    let border_h = clamp_box_dim(
        s.height, s.min_height, s.max_height, entry.extra_h + content_h.max(0.0), s.box_sizing, entry.extra_h,
    );
    let positioned_children = resolve_or_bubble_positioned(
        own_pending,
        s.position,
        entry.border_x + s.border_width,
        entry.border_y + s.border_width,
        (entry.border_w - 2.0 * s.border_width).max(0.0),
        (border_h - 2.0 * s.border_width).max(0.0),
        font_system,
        next_id,
        cache,
        deadline,
        incomplete,
        pending,
    );
    let mut laid = Laid {
        style: s,
        x: entry.border_x,
        y: entry.border_y,
        w: entry.border_w,
        h: border_h,
        background_image: b.background_image.clone(),
        content: LaidContent::Blocks(laid_children),
        positioned: positioned_children,
        fixed: false,
        sticky: None,
    };
    apply_relative_offset(&mut laid, &s, width, containing_height);
    capture_sticky_info(&mut laid, &s, containing_height);
    laid
}

/// `flex-direction: row`/`row-reverse` — the primary case this item
/// prioritizes. The main axis (width) is always definite already
/// (`entry.content_w`, the flex container's own content-box width), so
/// every item's final main size is decided *before* its one real
/// `measure()` call — unlike the cross axis (height), which needs that
/// call to learn an item's natural size first.
///
/// `flex-wrap` is genuinely implemented here: items are greedily packed
/// into lines (CSS Flexbox §9.3 — a line never starts empty; an item
/// wider than the container alone still gets its own line rather than
/// being dropped or forced to share), each line's main-axis sizing
/// resolved independently (`resolve_flex_main_sizes`/`distribute_
/// justify_content`), then lines are stacked top-to-bottom by their own
/// cross size, `row-gap` between them. `wrap-reverse` only stacks the
/// *lines* in the opposite order — not the items within a line, and not
/// `order` (unimplemented — see `TODO.txt`).
///
/// Cross-axis (height) alignment/stretch is a pure post-hoc `Laid::h`/
/// position adjustment, never a second `measure()` call: this engine's
/// own `height` property already doesn't affect how a box's *children*
/// are laid out (v42 — only the box's own reported size to its parent),
/// so once an item's natural height is known from its one real
/// `measure` call, stretching it to the line's cross size is just
/// overwriting `laid.h` directly — safe specifically *because* of that
/// pre-existing engine property, not a flexbox-specific trick.
///
/// **The one narrow, documented gap**: an item's final main size is
/// passed as `measure`'s own ordinary `width` parameter (same
/// convention `lay_out_row_cells` already uses for a table cell's
/// column-sized width) — if the item's own CSS `width` is `auto`
/// (the common case for a flex item relying on content/`flex-grow`/
/// `flex-shrink`), `box_model_entry`'s own auto-fill logic naturally
/// lands on exactly this value, so growing/shrinking fully works. But
/// if the item *also* declares an explicit, conflicting CSS `width`,
/// `clamp_box_dim` uses that explicit value instead of the distributed
/// one regardless of what's passed in — same accepted simplification
/// `measure_table`'s own per-cell `width` gap already has (`TODO.txt`).
#[allow(clippy::too_many_arguments)]
fn measure_flex_row<'a>(
    items: &[&'a LayoutBox],
    s: &ComputedStyle,
    entry: &BoxModelEntry,
    font_system: &mut FontSystem,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
    pending: &mut Vec<PendingPositioned<'a>>,
    no_floats: &Floats,
    reverse: bool,
    wrap: FlexWrap,
    main_gap: f32,
    cross_gap: f32,
) -> Vec<Laid> {
    if items.is_empty() {
        return Vec::new();
    }
    struct Prep {
        basis: f32,
        margin_left: f32,
        margin_right: f32,
        min: f32,
        max: f32,
        grow: f32,
        shrink: f32,
    }
    let preps: Vec<Prep> = items
        .iter()
        .map(|item| {
            // A throwaway finalized copy, purely to read this item's own
            // margin/width/min/max against the *real* containing-block
            // width (`entry.content_w`, already definite for a row
            // container — unlike `cell_natural_width`'s own `0.0`-basis
            // trick, which only exists because a table's real column
            // width genuinely isn't known yet at that point).
            let mut item_style = item.style;
            finalize_box_lengths(&mut item_style, entry.content_w, entry.child_containing_height);
            let margin_left = item_style.margin.left.px();
            let margin_right = item_style.margin.right.px();
            let extra_w = 2.0 * item_style.border_width + item_style.padding.left.px() + item_style.padding.right.px();
            let to_box = |raw: f32| match item_style.box_sizing {
                BoxSizing::ContentBox => raw.max(0.0) + extra_w,
                BoxSizing::BorderBox => raw.max(0.0),
            };
            // `flex-basis`'s own real fallback chain (CSS Flexbox §9.2):
            // an explicit length/percentage first, else this item's own
            // `width` (if set), else its natural content size.
            let basis_from_decl = match item_style.flex_basis {
                FlexBasis::Len(Length::Percent(p)) => Some(entry.content_w * p),
                FlexBasis::Len(l) => Some(l.resolve(item_style.font_size, item_style.root_font_size)),
                FlexBasis::Auto => None,
            };
            let basis = basis_from_decl.unwrap_or_else(|| {
                if item_style.width.is_some() {
                    // Not clamped by `min`/`max` here — real CSS doesn't
                    // clamp the *basis* itself, only the final resolved
                    // size (`resolve_flex_main_sizes`, below).
                    clamp_box_dim(item_style.width, None, None, 0.0, item_style.box_sizing, extra_w)
                } else {
                    // Natural (max-content) width, reused from the exact
                    // min/max-content machinery real auto table-column
                    // sizing already uses (`cell_natural_width` — margin-
                    // box, hence the margin subtraction to get border-box).
                    let (_, max_mb) = cell_natural_width(item, font_system, next_id, cache, deadline, incomplete);
                    (max_mb - margin_left - margin_right).max(0.0)
                }
            });
            Prep {
                basis,
                margin_left,
                margin_right,
                min: item_style.min_width.map(|v| to_box(v.px())).unwrap_or(0.0),
                max: item_style.max_width.map(|v| to_box(v.px())).unwrap_or(f32::INFINITY),
                grow: item.style.flex_grow,
                shrink: item.style.flex_shrink,
            }
        })
        .collect();

    let mut lines: Vec<Vec<usize>> = Vec::new();
    if wrap == FlexWrap::NoWrap {
        lines.push((0..items.len()).collect());
    } else {
        let mut current: Vec<usize> = Vec::new();
        let mut current_extent = 0.0f32;
        for (i, prep) in preps.iter().enumerate() {
            let item_extent = prep.basis + prep.margin_left + prep.margin_right;
            let needed = if current.is_empty() { item_extent } else { current_extent + main_gap + item_extent };
            if !current.is_empty() && needed > entry.content_w {
                lines.push(std::mem::take(&mut current));
                current_extent = item_extent;
                current.push(i);
            } else {
                current_extent = needed;
                current.push(i);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        if wrap == FlexWrap::WrapReverse {
            lines.reverse();
        }
    }

    let mut laid: Vec<Option<Laid>> = (0..items.len()).map(|_| None).collect();
    let mut line_top = entry.content_y;
    for line in &lines {
        let line_infos: Vec<FlexBasisInfo> = line
            .iter()
            .map(|&i| FlexBasisInfo {
                basis: preps[i].basis,
                grow: preps[i].grow,
                shrink: preps[i].shrink,
                min: preps[i].min,
                max: preps[i].max,
            })
            .collect();
        let final_sizes = resolve_flex_main_sizes(entry.content_w, main_gap, &line_infos);
        let n = line.len();
        let used: f32 = line.iter().enumerate().map(|(k, &i)| final_sizes[k] + preps[i].margin_left + preps[i].margin_right).sum::<f32>()
            + main_gap * (n.saturating_sub(1)) as f32;
        let leftover = (entry.content_w - used).max(0.0);
        let (start_offset, extra_between) = distribute_justify_content(leftover, n, s.justify_content);

        let mut cursor_x = entry.content_x + start_offset;
        let mut line_cross = 0.0f32;
        for (k, &i) in line.iter().enumerate() {
            let item = items[i];
            let main_w = final_sizes[k];
            let x = cursor_x + preps[i].margin_left;
            let lc = measure(
                item, font_system, x, line_top, main_w, entry.child_containing_height, next_id, cache, deadline,
                incomplete, pending, no_floats, None,
            );
            line_cross = line_cross.max(lc.style.margin.top.px() + lc.h + lc.style.margin.bottom.px());
            cursor_x += preps[i].margin_left + main_w + preps[i].margin_right + main_gap + extra_between;
            laid[i] = Some(lc);
        }
        for &i in line {
            let lc = laid[i].as_mut().expect("just measured above");
            let align = items[i].style.align_self.unwrap_or(s.align_items);
            let margin_top = lc.style.margin.top.px();
            let margin_bottom = lc.style.margin.bottom.px();
            if align == AlignItems::Stretch && items[i].style.height.is_none() {
                lc.h = (line_cross - margin_top - margin_bottom).max(0.0);
            }
            let target_y = match align {
                AlignItems::Stretch | AlignItems::FlexStart | AlignItems::Baseline => line_top + margin_top,
                AlignItems::FlexEnd => line_top + (line_cross - lc.h - margin_bottom),
                AlignItems::Center => line_top + (line_cross - lc.h - margin_top - margin_bottom) / 2.0 + margin_top,
            };
            shift_laid_tree(lc, 0.0, target_y - lc.y);
        }
        line_top += line_cross + cross_gap;
    }

    let mut result: Vec<Laid> = laid.into_iter().map(|l| l.expect("every item measured exactly once")).collect();
    if reverse {
        for lc in &mut result {
            let new_x = 2.0 * entry.content_x + entry.content_w - lc.x - lc.w;
            shift_laid_tree(lc, new_x - lc.x, 0.0);
        }
    }
    result
}

/// `flex-direction: column`/`column-reverse`. The cross axis (width) is
/// always definite (`entry.content_w`, the same value an ordinary block
/// child already gets), so stretch-vs-shrink-to-fit is decided *before*
/// each item's one real `measure()` call — the opposite order from the
/// row case's own cross axis (height), which needs that call to learn
/// an item's natural size first. There's no "natural content width"
/// counterpart for *height* in this engine (width is the one axis
/// `cell_natural_width` can measure without a real layout pass), so
/// each item's own real `measure()` call *is* how its natural/
/// hypothetical main size is learned (`lc.h`, already correct for both
/// "auto, content-driven" and "explicit CSS `height`" for free, since
/// `measure` already resolves `height` the ordinary way) — then, like
/// row's cross-axis stretch, the *final* (grown/shrunk) main size is a
/// pure post-hoc `lc.h` overwrite, safe for the same reason (`height`
/// doesn't affect a box's own children here). Unlike row's own width
/// gap, this means column's `flex-grow`/`flex-shrink` has no equivalent
/// "loses to an explicit CSS value" narrowing — the override always
/// applies.
///
/// **`flex-wrap` is not implemented for a column container** (see
/// `FlexWrap`'s own docs) — always exactly one column, regardless of
/// the declared value; `cross_gap`/multi-column distribution never
/// comes up.
#[allow(clippy::too_many_arguments)]
fn measure_flex_column<'a>(
    items: &[&'a LayoutBox],
    s: &ComputedStyle,
    entry: &BoxModelEntry,
    font_system: &mut FontSystem,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
    pending: &mut Vec<PendingPositioned<'a>>,
    no_floats: &Floats,
    reverse: bool,
    main_gap: f32,
) -> Vec<Laid> {
    if items.is_empty() {
        return Vec::new();
    }
    struct ColItem {
        lc: Laid,
        basis: f32,
        min: f32,
        max: f32,
        grow: f32,
        shrink: f32,
        margin_top: f32,
        margin_bottom: f32,
    }
    let mut prepped: Vec<ColItem> = items
        .iter()
        .map(|item| {
            let align = item.style.align_self.unwrap_or(s.align_items);
            // Stretch (the default) or an explicit `width` both measure
            // at the full cross width — `box_model_entry`'s own
            // explicit-width-wins-else-auto-fills logic already does
            // the right thing for both for free. Only a non-stretch,
            // `width: auto` item needs an actual shrink-to-fit width
            // computed up front.
            let width_param = if align != AlignItems::Stretch && item.style.width.is_none() {
                let mut item_style = item.style;
                finalize_box_lengths(&mut item_style, entry.content_w, entry.child_containing_height);
                let margin_lr = item_style.margin.left.px() + item_style.margin.right.px();
                let (_, max_mb) = cell_natural_width(item, font_system, next_id, cache, deadline, incomplete);
                (max_mb - margin_lr).max(0.0).min(entry.content_w)
            } else {
                entry.content_w
            };
            let lc = measure(
                item, font_system, entry.content_x, entry.content_y, width_param, entry.child_containing_height,
                next_id, cache, deadline, incomplete, pending, no_floats, None,
            );
            let margin_top = lc.style.margin.top.px();
            let margin_bottom = lc.style.margin.bottom.px();
            let basis = match lc.style.flex_basis {
                FlexBasis::Len(Length::Percent(p)) => entry.child_containing_height.map(|h| h * p).unwrap_or(lc.h),
                FlexBasis::Len(l) => l.resolve(lc.style.font_size, lc.style.root_font_size),
                FlexBasis::Auto => lc.h,
            };
            let min = lc.style.min_height.map(|v| v.px()).unwrap_or(0.0);
            let max = lc.style.max_height.map(|v| v.px()).unwrap_or(f32::INFINITY);
            ColItem {
                lc,
                basis,
                min,
                max,
                grow: item.style.flex_grow,
                shrink: item.style.flex_shrink,
                margin_top,
                margin_bottom,
            }
        })
        .collect();

    let n = prepped.len();
    let sum_natural: f32 =
        prepped.iter().map(|p| p.basis + p.margin_top + p.margin_bottom).sum::<f32>() + main_gap * (n.saturating_sub(1)) as f32;
    // Real CSS: an auto-height (indefinite main size) flex container has
    // no free space to grow into at all — same "no extra space to
    // distribute" outcome an ordinary `height: auto` block already has
    // today (matches `entry.child_containing_height`'s own `None` case,
    // see `box_model_entry`'s own docs).
    let main_size = entry.child_containing_height.unwrap_or(sum_natural);
    let infos: Vec<FlexBasisInfo> = prepped
        .iter()
        .map(|p| FlexBasisInfo { basis: p.basis, grow: p.grow, shrink: p.shrink, min: p.min, max: p.max })
        .collect();
    let final_sizes = resolve_flex_main_sizes(main_size, main_gap, &infos);
    let used: f32 = prepped.iter().zip(&final_sizes).map(|(p, &f)| f + p.margin_top + p.margin_bottom).sum::<f32>()
        + main_gap * (n.saturating_sub(1)) as f32;
    let leftover = (main_size - used).max(0.0);
    let (start_offset, extra_between) = distribute_justify_content(leftover, n, s.justify_content);

    let mut cursor_y = entry.content_y + start_offset;
    for (item, (p, &final_h)) in items.iter().zip(prepped.iter_mut().zip(&final_sizes)) {
        p.lc.h = final_h;
        let align = item.style.align_self.unwrap_or(s.align_items);
        let target_x = match align {
            AlignItems::Stretch | AlignItems::FlexStart | AlignItems::Baseline => entry.content_x,
            AlignItems::FlexEnd => entry.content_x + entry.content_w - p.lc.w,
            AlignItems::Center => entry.content_x + (entry.content_w - p.lc.w) / 2.0,
        };
        let target_y = cursor_y + p.margin_top;
        let dx = target_x - p.lc.x;
        let dy = target_y - p.lc.y;
        shift_laid_tree(&mut p.lc, dx, dy);
        cursor_y += p.margin_top + final_h + p.margin_bottom + main_gap + extra_between;
    }

    let mut result: Vec<Laid> = prepped.into_iter().map(|p| p.lc).collect();
    if reverse {
        for lc in &mut result {
            let new_y = 2.0 * entry.content_y + main_size - lc.y - lc.h;
            shift_laid_tree(lc, 0.0, new_y - lc.y);
        }
    }
    result
}

/// The real top-level entry point for laying out a whole document —
/// `measure` itself, plus resolving whatever `position: absolute`/
/// `fixed` box bubbled all the way up without ever finding a real
/// positioned ancestor (see `PendingPositioned`'s own docs): a `fixed`
/// box always ends up here (its containing block is always the
/// viewport, never an intermediate ancestor), and so does an `absolute`
/// box with no positioned ancestor anywhere above it in the tree —
/// both resolved against the same `viewport_w`×`viewport_h` rect,
/// approximating the real CSS "initial containing block" (this engine
/// has no literal viewport-sized box of its own to use instead — see
/// NOTES.md's own write-up of this feature's architecture for exactly
/// what that approximates away, and TODO.txt for the one visible
/// consequence: an orphaned `absolute` box's containing block doesn't
/// grow/shrink with the document's real scrollable height the way the
/// actual initial containing block would). The two cases still end up
/// painted differently despite sharing this containing-block rect — see
/// `Laid::fixed`'s and `paint`'s own docs for how a `fixed` box stays
/// put on screen while scrolling, independent of this positioning math.
#[allow(clippy::too_many_arguments)]
pub fn measure_document(
    b: &LayoutBox,
    font_system: &mut FontSystem,
    width: f32,
    viewport_h: f32,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
) -> Laid {
    let mut pending = Vec::new();
    // The document root starts a fresh BFC — the real initial
    // containing block has no floats of its own yet either.
    let root_floats = Floats::default();
    let mut laid = measure(
        b, font_system, 0.0, 0.0, width, None, next_id, cache, deadline, incomplete, &mut pending, &root_floats, None,
    );
    if !pending.is_empty() {
        let mut resolved: Vec<Laid> = pending
            .into_iter()
            .map(|item| resolve_positioned(item, 0.0, 0.0, width, viewport_h, font_system, next_id, cache, deadline, incomplete))
            .collect();
        laid.positioned.append(&mut resolved);
        // Re-sort the combined list — the document root is always
        // `Position::Static` in practice (see `build_layout_tree`), so
        // `laid.positioned` is normally empty before this, but sorting
        // again regardless keeps the invariant "a `Laid`'s own
        // `positioned` is always in ascending `z-index` order" true
        // even if that ever changes.
        laid.positioned.sort_by_key(|l| l.style.z_index.unwrap_or(0));
    }
    laid
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

/// Intersects `(x, y, w, h)` against `clip` (`None` = unclipped, passed
/// through unchanged) — `w`/`h` can come back `<= 0.0` for a rect
/// entirely outside the clip, which `fill_rect`'s own existing bounds
/// checks already treat as "paint nothing" (same as a rect entirely
/// outside the window).
fn intersect_clip(x: f32, y: f32, w: f32, h: f32, clip: Option<(f32, f32, f32, f32)>) -> (f32, f32, f32, f32) {
    let Some((cx, cy, cw, ch)) = clip else {
        return (x, y, w, h);
    };
    let x0 = x.max(cx);
    let y0 = y.max(cy);
    let x1 = (x + w).min(cx + cw);
    let y1 = (y + h).min(cy + ch);
    (x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// `clip` is the nearest ancestor's `overflow: hidden` content-box bounds
/// (screen space, post-scroll), same as `paint_buffer`'s own — applied
/// once here via `intersect_clip`, so every caller (directly, or through
/// `fill_rounded_rect`/`fill_rounded_ring`/`paint_border_edge`, all the
/// way up to `paint_box_decoration`) gets clipping for free without
/// needing its own `intersect_clip` call. `None` behaves exactly as
/// before this parameter existed (unclipped, window bounds only).
fn fill_rect(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: TextColor,
    clip: Option<(f32, f32, f32, f32)>,
) {
    if color.a() == 0 {
        return;
    }
    let (x, y, w, h) = intersect_clip(x, y, w, h, clip);
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
/// How far a row at vertical distance `dy` from its nearest top/bottom
/// edge should be inset from each side, for a rect corner-rounded by
/// `r` — shared by `fill_rounded_rect` and `fill_rounded_ring` (the
/// ring needs this computed twice per row, once for its outer boundary
/// and once for its inner one, each with its own radius). `r <= 0`
/// (including a clamped-to-zero inner radius when the border is wider
/// than the corner radius itself) naturally degrades to `0.0` for every
/// `dy`, i.e. a plain square edge — no separate no-rounding case needed.
fn corner_inset(dy: f32, r: f32) -> f32 {
    if dy < r {
        r - (r * r - (r - dy).max(0.0).powi(2)).max(0.0).sqrt()
    } else {
        0.0
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_rounded_rect(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    color: TextColor,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    if r <= 0.0 {
        fill_rect(pixels, win_w, win_h, x, y, w, h, color, clip);
        return;
    }
    let y0 = y.round() as i32;
    let y1 = (y + h).round() as i32;
    for row in y0..y1 {
        let dy = (row as f32 - y).min((y + h) - row as f32 - 1.0);
        let inset = corner_inset(dy, r);
        fill_rect(pixels, win_w, win_h, x + inset, row as f32, (w - 2.0 * inset).max(0.0), 1.0, color, clip);
    }
}

/// Paints a rounded *border ring* — the area between an outer rounded
/// rect (`x, y, w, h, radius`) and an inner one inset by `border_width`
/// on every side (inner radius `(radius - border_width).max(0.0)`, same
/// "ring narrows along with the border" real CSS shape). Unlike the
/// "paint a bigger solid shape, then a smaller one on top" trick
/// `outline`/`box-shadow` get away with, that doesn't work here: nothing
/// guarantees the interior is opaque to safely paint over (no
/// `background-color`), so this tests each row's outer *and* inner
/// boundary directly instead (see `ComputedStyle::border_radius`'s own
/// former caveat, now resolved by this function). Each row is at most
/// two horizontal spans (left and right edge strips) once it's within
/// the inner rect's own vertical extent, or one full-width span (a
/// plain top/bottom edge strip) outside it.
#[allow(clippy::too_many_arguments)]
fn fill_rounded_ring(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    border_width: f32,
    color: TextColor,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let r_outer = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let (inner_x, inner_y) = (x + border_width, y + border_width);
    let (inner_w, inner_h) = (w - 2.0 * border_width, h - 2.0 * border_width);
    let r_inner = (radius - border_width).max(0.0).min(inner_w / 2.0).min(inner_h / 2.0).max(0.0);
    let has_hole = inner_w > 0.0 && inner_h > 0.0;

    let y0 = y.round() as i32;
    let y1 = (y + h).round() as i32;
    for row in y0..y1 {
        let dy_outer = (row as f32 - y).min((y + h) - row as f32 - 1.0);
        let outer_inset = corner_inset(dy_outer, r_outer);
        let (outer_left, outer_right) = (x + outer_inset, x + w - outer_inset);

        let row_f = row as f32;
        let within_inner_rows = has_hole && row_f >= inner_y && row_f < inner_y + inner_h;
        if !within_inner_rows {
            fill_rect(pixels, win_w, win_h, outer_left, row_f, (outer_right - outer_left).max(0.0), 1.0, color, clip);
            continue;
        }
        let dy_inner = (row_f - inner_y).min((inner_y + inner_h) - row_f - 1.0);
        let inner_inset = corner_inset(dy_inner, r_inner);
        let (inner_left, inner_right) = (inner_x + inner_inset, inner_x + inner_w - inner_inset);
        fill_rect(pixels, win_w, win_h, outer_left, row_f, (inner_left - outer_left).max(0.0), 1.0, color, clip);
        fill_rect(pixels, win_w, win_h, inner_right, row_f, (outer_right - inner_right).max(0.0), 1.0, color, clip);
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
#[allow(clippy::too_many_arguments)]
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
    clip: Option<(f32, f32, f32, f32)>,
) {
    let horizontal = w >= h;
    let (along, across) = if horizontal { (w, h) } else { (h, w) };
    match style {
        BorderStyle::None => {}
        BorderStyle::Solid => fill_rect(pixels, win_w, win_h, x, y, w, h, color, clip),
        BorderStyle::Dashed => {
            let dash = (across * 3.0).max(2.0);
            let mut pos = 0.0;
            while pos < along {
                let seg = dash.min(along - pos);
                if horizontal {
                    fill_rect(pixels, win_w, win_h, x + pos, y, seg, across, color, clip);
                } else {
                    fill_rect(pixels, win_w, win_h, x, y + pos, across, seg, color, clip);
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
                    fill_rect(pixels, win_w, win_h, x + pos, y, seg, across, color, clip);
                } else {
                    fill_rect(pixels, win_w, win_h, x, y + pos, across, seg, color, clip);
                }
                pos += dot * 2.0;
            }
        }
        BorderStyle::Double => {
            let stripe = (across / 3.0).max(1.0);
            if horizontal {
                fill_rect(pixels, win_w, win_h, x, y, w, stripe, color, clip);
                fill_rect(pixels, win_w, win_h, x, y + h - stripe, w, stripe, color, clip);
            } else {
                fill_rect(pixels, win_w, win_h, x, y, stripe, h, color, clip);
                fill_rect(pixels, win_w, win_h, x + w - stripe, y, stripe, h, color, clip);
            }
        }
    }
}

/// `clip` is the nearest ancestor's `overflow: hidden` bounds (screen
/// space, post-scroll) — deliberately the *ambient* clip from `paint`'s
/// own caller, not this box's own `overflow` (which only starts clipping
/// its *descendants*, same as before this parameter existed — see
/// `paint`'s own `child_clip` vs. this call's docs).
#[allow(clippy::too_many_arguments)]
fn paint_box_decoration(
    s: &ComputedStyle,
    background_image: Option<&DecodedImage>,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
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
            clip,
        );
    }
    // Rounds both background and border together, matching a real
    // rounded box's background-under-border stacking — see
    // `fill_rounded_ring`'s own docs for why the border ring needs its
    // own dedicated painting instead of reusing `fill_rounded_rect`
    // twice. Scoped to `BorderStyle::Solid`: a dashed/dotted/double
    // ring would need the same curved-path tracing a solid ring does,
    // just repeated per dash/dot/stripe — a real `TODO.txt` gap, not
    // attempted here — so those combined with `border-radius` fall
    // back to the existing square-corner rendering rather than a
    // half-rounded, honestly-wrong mix of a rounded background behind
    // a square border.
    //
    // `border_radius` resolves its own `%` right here, against `w` —
    // this box's own border-box width, the real CSS basis for a
    // percentage `border-radius` (unlike `width`/`margin`/`padding`,
    // which resolve against the *containing block's* width — see
    // `BoxLength`'s own docs) — rather than in `measure`, since `w` is
    // already exactly the value needed and nothing else ever reads
    // this field before paint.
    let border_radius = s.border_radius.resolve(w);
    let round = border_radius > 0.0 && (s.border_width <= 0.0 || s.border_style == BorderStyle::Solid);
    if let Some(bg) = s.background_color {
        if round {
            fill_rounded_rect(pixels, win_w, win_h, x, y, w, h, border_radius, bg, clip);
        } else {
            fill_rect(pixels, win_w, win_h, x, y, w, h, bg, clip);
        }
    }
    // Layers on top of `background-color`, same real-CSS stacking —
    // tiled at its own intrinsic size (`background-repeat: repeat`/
    // `background-position: 0 0`, the unqualified default — no
    // `background-size`/`-position`/`-repeat` longhand control yet, see
    // TODO.txt). Not clipped to `border-radius`'s rounded corners even
    // when `round` is true — same square-tile painting regardless, a
    // further simplification nothing has needed yet.
    if let Some(image) = background_image {
        tile_image(image, pixels, win_w, win_h, x, y, w, h, clip);
    }
    if s.border_width > 0.0 && s.border_style != BorderStyle::None {
        if let Some(bc) = s.border_color {
            let bw = s.border_width;
            if round {
                fill_rounded_ring(pixels, win_w, win_h, x, y, w, h, border_radius, bw, bc, clip);
            } else {
                paint_border_edge(pixels, win_w, win_h, x, y, w, bw, s.border_style, bc, clip);
                paint_border_edge(pixels, win_w, win_h, x, y + h - bw, w, bw, s.border_style, bc, clip);
                paint_border_edge(pixels, win_w, win_h, x, y, bw, h, s.border_style, bc, clip);
                paint_border_edge(pixels, win_w, win_h, x + w - bw, y, bw, h, s.border_style, bc, clip);
            }
        }
    }
    // Outside the border box (not part of the box model — doesn't affect
    // `w`/`h` the way `border_width` does), same 4-rect ring shape as
    // border, just offset outward by its own width instead of inward.
    if s.outline_width > 0.0 {
        if let Some(oc) = s.outline_color {
            let ow = s.outline_width;
            let (ox, oy, ow_total, oh_total) = (x - ow, y - ow, w + 2.0 * ow, h + 2.0 * ow);
            fill_rect(pixels, win_w, win_h, ox, oy, ow_total, ow, oc, clip);
            fill_rect(pixels, win_w, win_h, ox, oy + oh_total - ow, ow_total, ow, oc, clip);
            fill_rect(pixels, win_w, win_h, ox, oy, ow, oh_total, oc, clip);
            fill_rect(pixels, win_w, win_h, ox + ow_total - ow, oy, ow, oh_total, oc, clip);
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
    clip: Option<(f32, f32, f32, f32)>,
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
                    clip,
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
    // `paint_decorations` draws per-run, not per-span — a known, narrow
    // simplification: nothing in `default.css` pairs `text-decoration`
    // with `visibility: hidden` today.
    hidden: &[bool],
    // Per-span `text-decoration-style` (by the same `glyph.metadata` index
    // as `backgrounds`/`valigns`/`hidden`) — see `paint_decorations`.
    decoration_styles: &[DecorationStyle],
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    offset_x: f32,
    offset_y: f32,
    // The nearest ancestor's `overflow: hidden` content-box bounds (screen
    // space, post-scroll), or `None` if nothing above this clips. Checked
    // per-pixel here (text has no shared `fill_rect` choke point to push
    // this into, unlike box decoration — see `fill_rect`'s own docs).
    clip: Option<(f32, f32, f32, f32)>,
) {
    let ox = offset_x.round() as i32;
    let oy = offset_y.round() as i32;
    paint_span_backgrounds(buffer, font_system, backgrounds, hidden, pixels, win_w, win_h, ox, oy, clip);
    let default_color = TextColor::rgb(0, 0, 0);
    // `buffer.draw()`'s own internals, copied out by hand (public:
    // `LegacyRenderer`; decoration lines via this file's own
    // `paint_decorations` instead of cosmic-text's `render_decoration`)
    // instead of called directly — neither offers a hook to nudge an
    // individual glyph's y position (`vertical-align: sub`/`super`) or
    // paint a dashed/dotted line instead of a solid one
    // (`text-decoration-style`), both needed per span (looked up by
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
                    if let Some((cx, cy, cw, ch)) = clip {
                        let (pxf, pyf) = (px as f32, py as f32);
                        if pxf < cx || pyf < cy || pxf >= cx + cw || pyf >= cy + ch {
                            continue;
                        }
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
        paint_decorations(&mut renderer, &run, default_color, decoration_styles);
    }
}

/// Reimplements cosmic-text's own `render_decoration`/`draw_decoration_span`
/// (`src/render.rs` in the vendored 0.19.0 source — not `pub`, so it can't
/// be called directly) by hand, using only `DecorationSpan`'s/
/// `GlyphDecorationData`'s public fields, so a `DecorationStyle` other than
/// `Solid` can paint a dashed/dotted line instead of a plain solid stroke
/// across the same extent — a hook the public `render_decoration` doesn't
/// offer. Mirrors its exact offset/thickness/color formulas, so
/// `DecorationStyle::Solid` (including double-underline, which is cosmic's
/// own native `UnderlineStyle::Double`) still looks pixel-identical to
/// calling `render_decoration` directly; only `Dashed`/`Dotted` diverge.
/// Each span's style is looked up by `decoration_styles[glyph.metadata]`
/// off its first glyph — the same per-span index every other per-span
/// property (backgrounds, valigns, ...) already uses.
fn paint_decorations<R: Renderer>(
    renderer: &mut R,
    run: &cosmic_text::LayoutRun,
    default_color: TextColor,
    decoration_styles: &[DecorationStyle],
) {
    for span in run.decorations {
        let glyphs = &run.glyphs[span.glyph_range.clone()];
        if glyphs.is_empty() {
            continue;
        }
        let style = decoration_styles.get(glyphs[0].metadata).copied().unwrap_or_default();
        let deco = &span.data;
        let td = &deco.text_decoration;
        let font_size = span.font_size;

        // Min/max over all glyphs, not first/last — RTL paragraphs store
        // glyphs in right-to-left order (same reasoning as cosmic-text's
        // own `draw_decoration_span`).
        let mut x_min = f32::INFINITY;
        let mut x_max = f32::NEG_INFINITY;
        for g in glyphs {
            x_min = x_min.min(g.x);
            x_max = x_max.max(g.x + g.w);
        }
        let width = x_max - x_min;
        if width <= 0.0 {
            continue;
        }
        let x_start = x_min;

        match td.underline {
            UnderlineStyle::None => {}
            UnderlineStyle::Single => {
                let color = td.underline_color_opt.or(span.color_opt).unwrap_or(default_color);
                let thickness = (deco.underline_metrics.thickness * font_size).max(1.0).ceil();
                let y = run.line_y - deco.underline_metrics.offset * font_size;
                paint_decoration_line(renderer, x_start, y, width, thickness, color, style);
            }
            UnderlineStyle::Double => {
                let color = td.underline_color_opt.or(span.color_opt).unwrap_or(default_color);
                let thickness = (deco.underline_metrics.thickness * font_size).max(1.0).ceil();
                let gap = thickness;
                let y = run.line_y - deco.underline_metrics.offset * font_size;
                paint_decoration_line(renderer, x_start, y, width, thickness, color, style);
                paint_decoration_line(renderer, x_start, y + thickness + gap, width, thickness, color, style);
            }
        }
        if td.strikethrough {
            let color = td.strikethrough_color_opt.or(span.color_opt).unwrap_or(default_color);
            let thickness = (deco.strikethrough_metrics.thickness * font_size).max(1.0).ceil();
            let y = run.line_y - deco.strikethrough_metrics.offset * font_size;
            paint_decoration_line(renderer, x_start, y, width, thickness, color, style);
        }
        if td.overline {
            let color = td.overline_color_opt.or(span.color_opt).unwrap_or(default_color);
            // Reuse underline thickness for overline, same as cosmic-text's
            // own `draw_decoration_span`.
            let thickness = (deco.underline_metrics.thickness * font_size).max(1.0).ceil();
            let y = (run.line_y - deco.ascent * font_size).max(run.line_top);
            paint_decoration_line(renderer, x_start, y, width, thickness, color, style);
        }
    }
}

/// Paints one decoration line's rectangle — the whole span width in one
/// call for `Solid`, or several short ones spaced across the same extent
/// for `Dashed`/`Dotted`. `x_start`/`y` are in the same float coordinate
/// space `render_decoration` itself uses (rounded to pixels by `Renderer`
/// implementors, same as any other rectangle it draws).
fn paint_decoration_line<R: Renderer>(
    renderer: &mut R,
    x_start: f32,
    y: f32,
    width: f32,
    thickness: f32,
    color: TextColor,
    style: DecorationStyle,
) {
    match style {
        DecorationStyle::Solid => {
            let w = width as u32;
            if w > 0 {
                renderer.rectangle(x_start as i32, y as i32, w, thickness as u32, color);
            }
        }
        DecorationStyle::Dashed | DecorationStyle::Dotted => {
            let (mark, gap) = if style == DecorationStyle::Dotted {
                (thickness.max(1.0), thickness.max(1.0) * 2.0)
            } else {
                (thickness.max(1.0) * 3.0, thickness.max(1.0) * 2.0)
            };
            let step = mark + gap;
            let mut offset = 0.0f32;
            while offset < width {
                let seg_w = mark.min(width - offset) as u32;
                if seg_w > 0 {
                    renderer.rectangle((x_start + offset) as i32, y as i32, seg_w, thickness as u32, color);
                }
                offset += step;
            }
        }
    }
}

/// Blits a decoded image into `(x, y, w, h)` — the laid content-box rect,
/// same positioning `paint`'s `Text` arm already uses for its own content
/// (padding/border already excluded by the caller). Nearest-neighbor
/// scaled from the image's intrinsic size to `w`/`h` when they differ —
/// no bilinear/smoothing, matching this file's existing hard-edged
/// painting fidelity everywhere else (see TODO.txt). Composited through
/// the same `blend` every other paint primitive uses, so a partially
/// transparent pixel (a PNG with alpha) shows whatever `paint_box_
/// decoration` already painted underneath (this box's own background,
/// if any — called first, same stacking a real browser uses).
#[allow(clippy::too_many_arguments)]
fn blit_image(image: &DecodedImage, pixels: &mut [u32], win_w: u32, win_h: u32, x: f32, y: f32, w: f32, h: f32, clip: Option<(f32, f32, f32, f32)>) {
    if image.width == 0 || image.height == 0 || w <= 0.0 || h <= 0.0 {
        return;
    }
    let (cx, cy, cw, ch) = intersect_clip(x, y, w, h, clip);
    let x0 = cx.max(0.0) as i64;
    let y0 = cy.max(0.0) as i64;
    let x1 = ((cx + cw).min(win_w as f32)) as i64;
    let y1 = ((cy + ch).min(win_h as f32)) as i64;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for py in y0..y1 {
        // Maps this destination row back to a source row via the
        // *unclipped* box bounds (`x`/`y`/`w`/`h`, not `cx`/`cy`), so
        // clipping never shifts which part of the image is sampled.
        let sy = (((py as f32 + 0.5 - y) / h * image.height as f32) as i64).clamp(0, image.height as i64 - 1) as u32;
        for px in x0..x1 {
            let sx = (((px as f32 + 0.5 - x) / w * image.width as f32) as i64).clamp(0, image.width as i64 - 1) as u32;
            let color = image.pixels[(sy * image.width + sx) as usize];
            let idx = (py as u32 * win_w + px as u32) as usize;
            pixels[idx] = blend(pixels[idx], color);
        }
    }
}

/// Tiles a decoded image across `(x, y, w, h)` at its own intrinsic
/// size — real CSS's unqualified `background-image: url(...)` default
/// (`background-repeat: repeat`, `background-position: 0 0` — no
/// longhand control over either yet, see TODO.txt). Composited the same
/// `blend` way every other paint primitive is, tile by tile, so a
/// partially transparent tile shows whatever painted underneath it
/// (this box's own `background-color`, if any — `paint_box_decoration`
/// fills that first).
#[allow(clippy::too_many_arguments)]
fn tile_image(image: &DecodedImage, pixels: &mut [u32], win_w: u32, win_h: u32, x: f32, y: f32, w: f32, h: f32, clip: Option<(f32, f32, f32, f32)>) {
    if image.width == 0 || image.height == 0 || w <= 0.0 || h <= 0.0 {
        return;
    }
    let (cx, cy, cw, ch) = intersect_clip(x, y, w, h, clip);
    let x0 = cx.max(0.0) as i64;
    let y0 = cy.max(0.0) as i64;
    let x1 = ((cx + cw).min(win_w as f32)) as i64;
    let y1 = ((cy + ch).min(win_h as f32)) as i64;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (iw, ih) = (image.width as i64, image.height as i64);
    for py in y0..y1 {
        // `rem_euclid` (not `%`) so the tile pattern still lines up
        // correctly when the box's own origin (`x`/`y`) sits at a
        // fractional or negative offset from the pixel grid — a plain
        // `%` would give a negative remainder there and sample out of
        // bounds.
        let sy = ((py as f32 - y) as i64).rem_euclid(ih) as u32;
        for px in x0..x1 {
            let sx = ((px as f32 - x) as i64).rem_euclid(iw) as u32;
            let color = image.pixels[(sy * image.width + sx) as usize];
            let idx = (py as u32 * win_w + px as u32) as usize;
            pixels[idx] = blend(pixels[idx], color);
        }
    }
}

/// Real CSS2 §9.9.1: `position: relative`/`sticky` establishes its own
/// descendant stacking context only when it also has an *explicit*
/// `z-index` (`z-index: auto` — by far the common case — does not; such
/// a box keeps today's plain paint-position-only treatment, painted in
/// ordinary document order alongside its non-positioned siblings, same
/// as before this feature existed). `absolute`/`fixed` are deliberately
/// excluded here even though they always *do* establish a stacking
/// context in real CSS: they're already out of flow and already have
/// their own, separate, correctly-ordered home (`Laid::positioned`) —
/// this predicate exists only to find the *in-flow* boxes that need
/// pulling out of `paint`'s/`hit_test_cursor`'s ordinary per-child loop
/// to be merged into that same z-ordered presentation instead.
fn is_explicit_stacking_child(l: &Laid) -> bool {
    matches!(l.style.position, Position::Relative | Position::Sticky) && l.style.z_index.is_some()
}

/// Paints one CSS2.1 Appendix E z-ordered layer for a box: either the
/// negative-stack-level slice (layer 2 — "child stacking contexts with
/// negative stack levels", painted *before* this box's own background/
/// border) or the zero-and-positive slice (layers 6+7 combined — see
/// `Laid::positioned`'s own docs for why merging those two is harmless
/// here), painted at the very end, same spot this box's out-of-flow
/// descendants have always painted at. Merges two different kinds of
/// descendant stacking context that both need to compete by the same
/// `z-index` ordering: `positioned` (this box's own out-of-flow
/// `absolute`/`fixed` catches, already resolved elsewhere) and
/// `stacking_children` (direct `Blocks`/`TableRow` children pulled out
/// by `is_explicit_stacking_child` — still genuinely in normal flow,
/// contributing to this box's size, just reordered for *paint* purposes
/// only). Ties (same explicit `z-index`, or both `auto`/unset as `0`)
/// keep `stacking_children` before `positioned` — an arbitrary but
/// stable, documented tie-break for a narrow case (two things anchored
/// to the very same box, at the very same explicit level) rather than a
/// claim about real document order across two differently-sourced
/// lists.
#[allow(clippy::too_many_arguments)]
fn paint_z_layer(
    positioned: &[Laid],
    stacking_children: &[&Laid],
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    cache: &mut ShapeCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    scroll_offset: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let mut entries: Vec<&Laid> = stacking_children.to_vec();
    entries.extend(positioned.iter());
    entries.sort_by_key(|l| l.style.z_index.unwrap_or(0));
    for item in entries {
        if matches!(item.style.position, Position::Relative | Position::Sticky) {
            // Same sticky-vs-relative scroll-offset trick `paint`'s own
            // ordinary `Blocks`/`TableRow` loop uses — a plain `relative`
            // box's offset is already permanently baked into its own
            // `x`/`y` (no `sticky` field), so this is a no-op shift (`dy
            // == 0`) for it, and the real per-frame shift for `sticky`.
            let child_scroll = match &item.sticky {
                Some(info) => scroll_offset - sticky_shift(info, item.h, scroll_offset, win_h as f32),
                None => scroll_offset,
            };
            paint(item, font_system, swash_cache, cache, pixels, win_w, win_h, child_scroll, clip);
        } else {
            // An out-of-flow `positioned` entry — same `fixed` handling
            // `paint`'s own old positioned-list loop always used.
            paint(item, font_system, swash_cache, cache, pixels, win_w, win_h, if item.fixed { 0.0 } else { scroll_offset }, clip);
        }
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
    // The *ambient* scroll offset — `0.0` for a `position: fixed`
    // subtree (see the `laid.positioned` loop below and `Laid::fixed`'s
    // own docs), the real page scroll offset for everything else. Not
    // read directly off `laid` itself: a `fixed` box's own descendants
    // (normal content *inside* it) need to keep seeing whatever value
    // this call was given, which is exactly what naturally happens —
    // every recursive `paint` call below just forwards this same
    // parameter unchanged, so passing `0.0` once, at the top of a
    // `fixed` subtree, is all it takes for the whole subtree to render
    // scroll-independent; nothing *inside* that subtree needs its own
    // `fixed` check.
    scroll_offset: f32,
    // The nearest ancestor's `overflow: hidden` bounds (screen space,
    // post-scroll), if any — `None` means nothing above this clips.
    // Reaches every paint call this box makes: text/span-backgrounds
    // (`paint_buffer`) *and*, since v40, this box's own background/
    // border/outline/shadow (`paint_box_decoration`) — both funnel
    // through the one shared `fill_rect` primitive, which applies it.
    clip: Option<(f32, f32, f32, f32)>,
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

    // This box's own `overflow: hidden` (if any) only starts clipping
    // its *descendants* — its own background/border below isn't clipped
    // by it, same as an ancestor's `clip` never reaches that call either.
    // Computed before any painting happens below (it's pure geometry —
    // moving it ahead of the negative-z-index layer, which now needs to
    // paint *before* this box's own background, doesn't change what it
    // computes).
    let child_clip = if s.overflow == Overflow::Hidden {
        let (cx, cy, cw, ch) = intersect_clip(laid.x, draw_y, laid.w, laid.h, clip);
        Some((cx, cy, cw, ch))
    } else {
        clip
    };

    // Real CSS2.1 Appendix E stacking order, split at the zero boundary.
    // **Layer 2 (negative stack levels) paints *above* this box's own
    // background/border (layer 1), not below it** — the spec's own
    // bottommost-first list is 1 (background), 2 (negative), 3-5 (normal
    // content), 6-7 (zero/positive) — so a negative-`z-index` descendant
    // is only ever hidden *behind this box's own normal (layer 3+)
    // content*, never behind this box's own background fill. (Checked
    // directly against https://www.w3.org/TR/CSS21/zindex.html, which
    // lists background before negative contexts in its "bottommost
    // first" order — i.e. negative paints on top of it. This also
    // matches the well-known real-world experience of `z-index: -1`:
    // it only visibly disappears "behind everything" when the element
    // it's negative *relative to* has no background of its own to begin
    // with, not because negative z-index literally paints beneath any
    // box's own background.) `laid.positioned` is already sorted
    // ascending by `z-index`, so `partition_point` finds the zero
    // boundary with no extra sort. Direct `Blocks`/`TableRow` children
    // that establish their own stacking context (`is_explicit_stacking_
    // child` — `position: relative`/`sticky` with an explicit `z-index`)
    // are pulled out the same way, from whichever list this box's own
    // content actually is (empty for every other `LaidContent` variant —
    // nothing but `Blocks`/`TableRow` has a child list to pull from).
    let neg_split = laid.positioned.partition_point(|l| l.style.z_index.unwrap_or(0) < 0);
    let (neg_positioned, nonneg_positioned) = laid.positioned.split_at(neg_split);
    let (neg_stacking_children, nonneg_stacking_children): (Vec<&Laid>, Vec<&Laid>) = match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => children
            .iter()
            .filter(|c| is_explicit_stacking_child(c))
            .partition(|c| c.style.z_index.unwrap_or(0) < 0),
        _ => (Vec::new(), Vec::new()),
    };

    // Layer 1: this box's own background/border.
    // `visibility: hidden` still reserves this box's layout space (laid
    // out exactly as normal above) but skips its own background/border —
    // a descendant box with its own `visibility: visible` is a separate
    // `Laid` with its own independently-resolved `style.visibility` via
    // real CSS inheritance, so it still recurses and paints normally.
    if s.visibility == Visibility::Visible {
        // Clipped by the *ambient* `clip` from an ancestor's `overflow:
        // hidden` — deliberately not `child_clip` below, since a box's
        // own `overflow` only clips its descendants' overflowing
        // content, never its own background/border/outline/shadow,
        // same as real CSS.
        paint_box_decoration(&s, laid.background_image.as_deref(), pixels, win_w, win_h, laid.x, draw_y, laid.w, laid.h, clip);
    }

    // Layer 2: negative stacking contexts — above this box's own
    // background/border (just painted), below everything else (the
    // normal-content `match` below, and the zero/positive layer at the
    // end of this function) — unconditional, same reasoning as that
    // final layer (each entry's own recursive `paint` call checks *its
    // own* `style.visibility`).
    paint_z_layer(
        neg_positioned, &neg_stacking_children, font_system, swash_cache, cache, pixels, win_w, win_h, scroll_offset,
        child_clip,
    );

    match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            // Layers 3/4/5: ordinary in-flow content, in document order
            // (floats already sorted to the end of this same list by
            // `measure`'s own `BoxContent::Blocks` arm — see v54). Any
            // child pulled out above (`is_explicit_stacking_child`) is
            // skipped here — it's painted by one of the two `paint_z_
            // layer` calls instead, never both (every child is painted
            // exactly once).
            for child in children.iter().filter(|c| !is_explicit_stacking_child(c)) {
                // A `position: sticky` child's own `Laid::x`/`.y` are
                // still exactly its static in-flow position (never
                // permanently shifted, unlike `relative` — see `Position::
                // Sticky`'s own docs) — so its real, current painted
                // position is computed fresh, right here, from the live
                // `scroll_offset`, via `sticky_shift`. Folding that `dy`
                // into the *ambient* scroll offset passed down into this
                // one recursive call (rather than mutating `child` or
                // threading a separate offset parameter) reuses the exact
                // trick `fixed` already established: every descendant
                // inside this subtree (`draw_y = y - scroll_offset`, and
                // any nested `positioned`/`sticky` box of its own) sees
                // the adjusted value and shifts correctly with no extra
                // per-node check needed anywhere inside it.
                let child_scroll = match &child.sticky {
                    Some(info) => scroll_offset - sticky_shift(info, child.h, scroll_offset, win_h as f32),
                    None => scroll_offset,
                };
                paint(child, font_system, swash_cache, cache, pixels, win_w, win_h, child_scroll, child_clip);
            }
        }
        LaidContent::Text(id) => {
            // Not yet shaped (still mid-way through a budgeted measure pass)
            // just means nothing to paint here yet — it'll appear once a
            // later frame finishes shaping it.
            if let Some((buffer, _, backgrounds, valigns, hidden, _, decoration_styles)) = cache.get_mut(id) {
                let content_x = laid.x + s.border_width + s.padding.left.px();
                let content_y = draw_y + s.border_width + s.padding.top.px();
                paint_buffer(
                    buffer, font_system, swash_cache, backgrounds, valigns, hidden, decoration_styles, pixels,
                    win_w, win_h, content_x, content_y, child_clip,
                );
            }
        }
        LaidContent::Image(image) => {
            let content_x = laid.x + s.border_width + s.padding.left.px();
            let content_y = draw_y + s.border_width + s.padding.top.px();
            let content_w = (laid.w - 2.0 * s.border_width - s.padding.left.px() - s.padding.right.px()).max(0.0);
            let content_h = (laid.h - 2.0 * s.border_width - s.padding.top.px() - s.padding.bottom.px()).max(0.0);
            blit_image(image, pixels, win_w, win_h, content_x, content_y, content_w, content_h, child_clip);
        }
        LaidContent::MixedInline(positioned) => {
            for item in positioned {
                match item {
                    // A full, ordinary `Laid` — recurses through the
                    // exact same path a `Blocks`/`TableRow` child does,
                    // so it gets box-decoration/clipping/culling for
                    // free (see `MixedPositioned`'s own docs).
                    MixedPositioned::Image(img_laid) => {
                        paint(img_laid, font_system, swash_cache, cache, pixels, win_w, win_h, scroll_offset, child_clip);
                    }
                    MixedPositioned::Word { id, x, y, .. } => {
                        if let Some((buffer, _, backgrounds, valigns, hidden, _, decoration_styles)) = cache.get_mut(id) {
                            paint_buffer(
                                buffer, font_system, swash_cache, backgrounds, valigns, hidden, decoration_styles,
                                pixels, win_w, win_h, *x, *y - scroll_offset, child_clip,
                            );
                        }
                    }
                }
            }
        }
    }

    // Layers 6+7: this box's own out-of-flow `position: absolute`/
    // `fixed` descendants (`laid.positioned`, already sorted ascending
    // by `z-index` when it was built), merged with any direct `Blocks`/
    // `TableRow` child establishing its own non-negative-`z-index`
    // stacking context (`nonneg_stacking_children` above) — painted
    // *after* this box's own normal content, so they legitimately paint
    // on top of it and of each other. Unconditional (not gated on `s.
    // visibility`, unlike this box's own decoration above): visibility
    // is independent per element, and each entry's own recursive `paint`
    // call already checks *its own* `style.visibility`. Reuses `child_
    // clip` — the same bounds a normal descendant would be clipped to —
    // since each entry's containing block genuinely *is* this exact box
    // (real CSS: an `overflow: hidden` ancestor clips an absolutely
    // positioned descendant whenever that ancestor is at or inside its
    // containing block, which is always true here by construction).
    paint_z_layer(
        nonneg_positioned, &nonneg_stacking_children, font_system, swash_cache, cache, pixels, win_w, win_h,
        scroll_offset, child_clip,
    );
}

/// Finds the `CursorStyle` that should show at document-space `(x, y)` —
/// `y` already has any scroll offset added in (document space, same
/// space `Laid.x`/`.y` themselves are in, *not* the screen space mouse
/// events arrive in — see the `CursorMoved` handler). Falls back to a
/// box's own (already-inherited) `style.cursor` wherever no more
/// specific glyph match is found underneath it, same "ambient" layering
/// `paint`'s own box-vs-descendant painting already relies on. `cursor`
/// is real CSS inheritance (see `CursorStyle`'s own docs), so the
/// per-glyph `cursors` vector already reflects the correctly-cascaded
/// value for whichever span that glyph came from — no separate
/// propagation logic needed here either.
/// `scroll_offset` is the real, ambient page scroll offset — needed
/// here (unlike most of this function, which works in plain document
/// space throughout) only to translate into/out of a `position: fixed`
/// descendant's own viewport-space coordinates (see `Laid::fixed`'s own
/// docs — same translation `paint` does by passing `0.0` down instead,
/// but `hit_test_cursor` isn't recursive the same uniform way, since a
/// `fixed` descendant can appear at any depth's own `positioned` list),
/// and to compute a `position: sticky` child's own live, current
/// position (`sticky_shift` — see the `Blocks`/`TableRow` arm below).
/// `viewport_h` is only needed for *that*: `sticky_shift`'s own
/// `bottom`-offset math.
fn hit_test_cursor(laid: &Laid, x: f32, y: f32, cache: &ShapeCache, scroll_offset: f32, viewport_h: f32) -> CursorStyle {
    if x < laid.x || x >= laid.x + laid.w || y < laid.y || y >= laid.y + laid.h {
        return CursorStyle::Auto;
    }
    // Same `is_explicit_stacking_child` pull-out `paint` uses, merged
    // with `laid.positioned` for one combined reversed-priority check
    // below — a direct `Blocks`/`TableRow` child establishing its own
    // stacking context visually paints outside the plain document-order
    // loop (same as an out-of-flow descendant), so it must win a hit
    // test the same way. **Deliberately not split by sign the way
    // `paint`'s own layering now is** — `paint`'s negative layer sits
    // *below* normal content (see its own docs), but a negative entry is
    // still checked here at the *same* (first/highest) priority as a
    // non-negative one: a narrow, documented approximation (see
    // TODO.txt) rather than a full mirror of that split, since a
    // negative stacking context's hit-test priority is rarely
    // observable in practice (it only wins a hit at all when nothing in
    // the normal-content layer happens to cover the same pixel).
    let stacking_children: Vec<&Laid> = match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            children.iter().filter(|c| is_explicit_stacking_child(c)).collect()
        }
        _ => Vec::new(),
    };
    let mut above: Vec<&Laid> = stacking_children.clone();
    above.extend(laid.positioned.iter());
    above.sort_by_key(|l| l.style.z_index.unwrap_or(0));
    // Checked first, in reverse (highest `z-index`/last-painted first),
    // so a hit lands on whatever's actually on top. `.rev()` over an
    // ascending-`z-index` list, not a second sort.
    for item in above.iter().rev() {
        if matches!(item.style.position, Position::Relative | Position::Sticky) {
            // Mirror image of `paint`'s own trick (see the `Blocks`/
            // `TableRow` loop below): a sticky item's document-space
            // rect is shifted by `dy` from its stored (static) `x`/`y`;
            // a plain `relative` item has no `sticky` field at all, so
            // `dy` is always `0` for it (its offset is already baked
            // into `x`/`y` permanently).
            let dy = item.sticky.as_ref().map_or(0.0, |info| sticky_shift(info, item.h, scroll_offset, viewport_h));
            let (ix, iy) = (item.x, item.y + dy);
            if ix <= x && x < ix + item.w && iy <= y && y < iy + item.h {
                return hit_test_cursor(item, x, y - dy, cache, scroll_offset, viewport_h);
            }
        } else {
            let (ix, iy, inner_scroll) = if item.fixed { (x, y - scroll_offset, 0.0) } else { (x, y, scroll_offset) };
            if ix >= item.x && ix < item.x + item.w && iy >= item.y && iy < item.y + item.h {
                return hit_test_cursor(item, ix, iy, cache, inner_scroll, viewport_h);
            }
        }
    }
    match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            for child in children.iter().filter(|c| !is_explicit_stacking_child(c)) {
                // Mirror image of `paint`'s own trick: a sticky child's
                // own *document-space* rect, for hit-testing purposes, is
                // shifted by `dy` from its stored (static) `x`/`y` — so
                // test against that shifted rect, but recurse with `y`
                // translated *back* (`y - dy`), since every descendant
                // position inside `child` is still stored relative to
                // its own static, unshifted position.
                if let Some(info) = &child.sticky {
                    let dy = sticky_shift(info, child.h, scroll_offset, viewport_h);
                    let (cx, cy) = (child.x, child.y + dy);
                    if cx <= x && x < cx + child.w && cy <= y && y < cy + child.h {
                        return hit_test_cursor(child, x, y - dy, cache, scroll_offset, viewport_h);
                    }
                    continue;
                }
                if child.x <= x && x < child.x + child.w && child.y <= y && y < child.y + child.h {
                    return hit_test_cursor(child, x, y, cache, scroll_offset, viewport_h);
                }
            }
            laid.style.cursor
        }
        LaidContent::Text(id) => {
            if let Some((buffer, _, _, _, _, cursors, _)) = cache.get(id) {
                let content_x = laid.x + laid.style.border_width + laid.style.padding.left.px();
                let content_y = laid.y + laid.style.border_width + laid.style.padding.top.px();
                let local_x = x - content_x;
                let local_y = y - content_y;
                for run in buffer.layout_runs() {
                    if local_y < run.line_top || local_y >= run.line_top + run.line_height {
                        continue;
                    }
                    for glyph in run.glyphs {
                        if local_x >= glyph.x && local_x < glyph.x + glyph.w {
                            return cursors.get(glyph.metadata).copied().unwrap_or(laid.style.cursor);
                        }
                    }
                }
            }
            laid.style.cursor
        }
        // No finer-grained hit test within an image — just this box's
        // own (already-cascaded) cursor, same fallback the other arms
        // use once nothing more specific matches underneath.
        LaidContent::Image(_) => laid.style.cursor,
        LaidContent::MixedInline(positioned) => {
            for item in positioned {
                match item {
                    MixedPositioned::Image(img_laid) => {
                        if img_laid.x <= x && x < img_laid.x + img_laid.w && img_laid.y <= y && y < img_laid.y + img_laid.h {
                            return hit_test_cursor(img_laid, x, y, cache, scroll_offset, viewport_h);
                        }
                    }
                    MixedPositioned::Word { id, x: wx, y: wy, w, h } => {
                        if x < *wx || x >= *wx + *w || y < *wy || y >= *wy + *h {
                            continue;
                        }
                        if let Some((buffer, _, _, _, _, cursors, _)) = cache.get(id) {
                            let local_x = x - wx;
                            let local_y = y - wy;
                            for run in buffer.layout_runs() {
                                if local_y < run.line_top || local_y >= run.line_top + run.line_height {
                                    continue;
                                }
                                for glyph in run.glyphs {
                                    if local_x >= glyph.x && local_x < glyph.x + glyph.w {
                                        return cursors.get(glyph.metadata).copied().unwrap_or(laid.style.cursor);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            laid.style.cursor
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
    /// The OS cursor icon last actually set, so `CursorMoved` only calls
    /// `window.set_cursor_icon` when the hit-tested style actually
    /// changes, not on every mouse-move tick.
    last_cursor: CursorIcon,
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
            last_cursor: CursorIcon::Default,
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
            last_cursor: CursorIcon::Default,
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
            self.cached = Some(measure_document(
                self.tree.as_ref().unwrap(),
                &mut self.font_system,
                width as f32,
                // This is the one place `measure_document`'s own
                // `viewport_h` parameter is a real, definite window
                // dimension — used *only* for `position: fixed` (and an
                // orphaned `position: absolute`)'s own containing block
                // (see its own docs), never reused as the root box's
                // own `%`-height-resolution basis the way NOTES.md's v49
                // entry warned against: that's still a *completely
                // separate* `None` passed one level down, inside
                // `measure_document` itself, to the root box's own
                // `measure` call — this engine still has no viewport/
                // initial-containing-block *height* concept for an
                // ordinary `html, body { height: 100% }` pattern (see
                // TODO.txt), only for this one new, narrower role.
                height as f32,
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
                None,
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
            WindowEvent::CursorMoved { position, .. } => {
                // `position` is window/screen space; `Laid.x`/`.y` are
                // document space (scroll isn't applied until paint time),
                // so the same `+ scroll_offset` adjustment `paint` itself
                // makes (in reverse) is needed here too.
                if let (Some(laid), Some(window)) = (&self.cached, &self.window) {
                    let doc_x = position.x as f32;
                    let doc_y = position.y as f32 + self.scroll_offset;
                    let icon = hit_test_cursor(
                        laid, doc_x, doc_y, &self.shape_cache, self.scroll_offset, self.surface_size.1 as f32,
                    )
                    .to_winit();
                    if icon != self.last_cursor {
                        window.set_cursor(icon);
                        self.last_cursor = icon;
                    }
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

/// Persistent regression tests, kept from here on (not removed before
/// committing like `NOTES.md`'s earlier versions describe) — see the
/// `PROMPTS.md` entry explaining the switch. Organized by the feature
/// each test guards, roughly in the order those features landed.
#[cfg(test)]
mod tests {
    use super::*;

    // ---------- v36: `rem` resolves against `<html>`'s real font-size ----------

    fn find_deepest_nonzero_margin_top(b: &LayoutBox) -> Option<&LayoutBox> {
        if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                if let Some(found) = find_deepest_nonzero_margin_top(c) {
                    return Some(found);
                }
            }
        }
        if b.style.margin.top.px() != 0.0 {
            return Some(b);
        }
        None
    }

    #[test]
    fn rem_resolves_against_html_font_size() {
        let doc = parse_html(
            r#"<html style="font-size: 40px"><body><div style="margin-top: 2rem">x</div></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let div = find_deepest_nonzero_margin_top(&tree).expect("div not found");
        assert_eq!(div.style.margin.top.px(), 80.0, "2rem should resolve against html's 40px, giving 80px");
    }

    #[test]
    fn rem_defaults_to_16_without_html_override() {
        let doc = parse_html(r#"<html><body><div style="margin-top: 2rem">x</div></body></html>"#);
        let tree = build_layout_tree(&doc);
        let div = find_deepest_nonzero_margin_top(&tree).expect("div not found");
        assert_eq!(div.style.margin.top.px(), 32.0);
    }

    // ---------- v37: `:link`/`:visited` pseudo-classes ----------

    fn find_first_span_color(b: &LayoutBox) -> Option<TextColor> {
        if let BoxContent::Inline(items) = &b.content {
            if let Some(InlineItem::Text(_, s)) = items.first() {
                return Some(s.color);
            }
        } else if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                if let Some(found) = find_first_span_color(c) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn link_pseudo_matches_anchor_with_href() {
        let doc = parse_html(
            r#"<html><body><style>a:link { color: #ff0000; } a:visited { color: #00ff00; }</style><a href="x">hi</a></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(color, TextColor::rgb(0xff, 0, 0), ":link should match an <a href> and apply red");
    }

    #[test]
    fn link_pseudo_does_not_match_anchor_without_href() {
        let doc = parse_html(r#"<html><body><style>a:link { color: #ff0000; }</style><a>hi</a></body></html>"#);
        let tree = build_layout_tree(&doc);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_ne!(color, TextColor::rgb(0xff, 0, 0), ":link should not match an <a> without href");
    }

    #[test]
    fn visited_never_matches() {
        let doc = parse_html(
            r#"<html><body><style>a:visited { color: #00ff00; }</style><a href="x">hi</a></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_ne!(color, TextColor::rgb(0, 0xff, 0), ":visited should never match");
    }

    // ---------- v38: `border-radius` + a visible border (rounded ring) ----------

    #[test]
    fn rounded_bg_then_ring_combo() {
        const W: u32 = 40;
        const H: u32 = 40;
        let mut pixels = vec![0u32; (W * H) as usize];
        let bg = TextColor::rgb(0, 0, 255);
        let border = TextColor::rgb(255, 0, 0);
        fill_rounded_rect(&mut pixels, W, H, 0.0, 0.0, 40.0, 40.0, 10.0, bg, None);
        fill_rounded_ring(&mut pixels, W, H, 0.0, 0.0, 40.0, 40.0, 10.0, 5.0, border, None);
        let px = |x: u32, y: u32| pixels[(y * W + x) as usize];

        assert_eq!(px(0, 0), 0x000000, "corner should be untouched (cut by rounding)");
        assert_eq!(px(20, 2), 0x_FF0000, "top edge within border thickness should be red");
        assert_eq!(px(20, 20), 0x_0000FF, "center should show background through the ring's hole");
        assert_eq!(px(2, 20), 0x_FF0000, "left edge within border thickness should be red");
    }

    // ---------- v39: text-decoration reaching a block's own text ----------

    fn collect_decorations(b: &LayoutBox, out: &mut Vec<TextDecoration>) {
        if let BoxContent::Inline(items) = &b.content {
            for item in items {
                if let InlineItem::Text(_, s) = item {
                    out.push(s.text_decoration);
                }
            }
        }
        if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                collect_decorations(c, out);
            }
        }
    }

    #[test]
    fn block_own_direct_text_gets_its_own_decoration() {
        let doc = parse_html(r#"<html><body><p style="text-decoration: underline">text</p></body></html>"#);
        let tree = build_layout_tree(&doc);
        let mut decos = Vec::new();
        collect_decorations(&tree, &mut decos);
        assert!(decos.iter().any(|d| *d != TextDecoration::new()), "p's own direct text should be underlined: {:?}", decos);
    }

    #[test]
    fn decoration_propagates_through_nested_block_descendant() {
        let doc = parse_html(
            r#"<html><body><div style="text-decoration: underline"><div>nested</div></div></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let mut decos = Vec::new();
        collect_decorations(&tree, &mut decos);
        assert!(decos.iter().any(|d| *d != TextDecoration::new()), "nested block descendant should inherit ambient decoration: {:?}", decos);
    }

    #[test]
    fn nested_block_can_opt_out_of_decoration_with_none() {
        let doc = parse_html(
            r#"<html><body><div style="text-decoration: underline"><div style="text-decoration: none">nested</div></div></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let mut decos = Vec::new();
        collect_decorations(&tree, &mut decos);
        assert!(decos.iter().all(|d| *d == TextDecoration::new()), "explicit none should opt out: {:?}", decos);
    }

    #[test]
    fn sibling_block_does_not_leak_decoration() {
        let doc = parse_html(
            r#"<html><body><div style="text-decoration: underline">a</div><div>b</div></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let mut decos = Vec::new();
        collect_decorations(&tree, &mut decos);
        assert_eq!(decos.len(), 2, "expected exactly 2 spans: {:?}", decos);
        assert_ne!(decos[0], TextDecoration::new(), "first div should be underlined: {:?}", decos);
        assert_eq!(decos[1], TextDecoration::new(), "second (sibling) div should NOT inherit the first's decoration: {:?}", decos);
    }

    // ---------- v40: `overflow: hidden` clips box decoration ----------

    #[test]
    fn fill_rect_respects_clip() {
        let mut pixels = vec![0u32; 20 * 20];
        let color = TextColor::rgb(255, 0, 0);
        fill_rect(&mut pixels, 20, 20, 0.0, 0.0, 20.0, 20.0, color, Some((5.0, 5.0, 5.0, 5.0)));
        for y in 0..20u32 {
            for x in 0..20u32 {
                let painted = pixels[(y * 20 + x) as usize] != 0;
                let in_clip = (5..10).contains(&x) && (5..10).contains(&y);
                assert_eq!(painted, in_clip, "pixel ({x},{y}) painted={painted} in_clip={in_clip}");
            }
        }
    }

    #[test]
    fn nested_block_decoration_clipped_by_ancestor_overflow_hidden() {
        let doc = parse_html(
            r#"<html><body>
<div style="overflow: hidden; padding: 10px; background-color: #0000ff;">
  <div style="margin-left: 500px; background-color: #ff0000; padding: 5px;">inner</div>
</div>
</body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure(
            &tree, &mut font_system, 0.0, 0.0, 200.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
            &mut Vec::new(), &Floats::default(), None,
        );
        let mut swash_cache = SwashCache::new();
        let win_w = 800u32;
        let win_h = 200u32;
        let mut pixels = vec![0u32; (win_w * win_h) as usize];
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &mut pixels, win_w, win_h, 0.0, None);

        let has_red = pixels.iter().any(|&p| (p & 0x00FF0000) >> 16 == 0xFF && (p & 0xFF) == 0x00 && ((p >> 8) & 0xFF) == 0x00);
        assert!(!has_red, "inner div's red background should be fully clipped by ancestor overflow:hidden");
        let has_blue = pixels.contains(&0x0000FF);
        assert!(has_blue, "outer div's own background should still paint");
    }

    // ---------- v42: `width`/`height` CSS properties ----------

    fn measure_fixture(html: &str, available_width: f32) -> Laid {
        measure_fixture_h(html, available_width, None)
    }

    /// Same as `measure_fixture`, but also lets a test supply a definite
    /// containing-block height for the document root (see `measure`'s
    /// own `containing_height` docs) — needed to exercise a `%` `height`/
    /// `min-height`/`max-height` declared on a top-level element, which
    /// `measure_fixture`'s plain `None` (indefinite) deliberately can't
    /// resolve, matching a real UA only ever having one via its viewport.
    fn measure_fixture_h(html: &str, available_width: f32, containing_height: Option<f32>) -> Laid {
        let doc = parse_html(html);
        let tree = build_layout_tree(&doc);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        measure(
            &tree, &mut font_system, 0.0, 0.0, available_width, containing_height, &mut next_id, &mut cache, deadline,
            &mut incomplete, &mut Vec::new(), &Floats::default(), None,
        )
    }

    #[test]
    fn explicit_width_sets_content_box_width() {
        let laid = measure_fixture(
            r#"<html><body><div style="width: 100px; padding: 10px;">x</div></body></html>"#,
            500.0,
        );
        let div = find_by_width(&laid, 100.0).expect("div with explicit width not found");
        assert_eq!(div.w, 120.0, "content-box: 100px content + 10px padding on each side");
    }

    #[test]
    fn explicit_width_can_exceed_available_width() {
        let laid = measure_fixture(r#"<html><body><div style="width: 900px;">x</div></body></html>"#, 200.0);
        let div = find_by_width(&laid, 900.0).expect("div with explicit width not found");
        assert_eq!(div.w, 900.0, "an explicit width isn't clamped to the container, same as real CSS");
    }

    fn find_laid_by_margin_top(laid: &Laid, target: f32) -> Option<&Laid> {
        if laid.style.margin.top.px() == target {
            return Some(laid);
        }
        if let LaidContent::Blocks(children) = &laid.content {
            for c in children {
                if let Some(found) = find_laid_by_margin_top(c, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn explicit_width_constrains_children() {
        // The inner div has no `width` of its own to search by — marked
        // with a distinguishing `margin-top` instead (the only nonzero
        // margin anywhere in this fixture).
        let laid = measure_fixture(
            r#"<html><body><div style="width: 150px;"><div style="margin-top: 1px;">inner</div></div></body></html>"#,
            900.0,
        );
        let inner = find_laid_by_margin_top(&laid, 1.0).expect("inner div not found");
        assert_eq!(inner.w, 150.0, "a child with no width of its own should be laid out against its parent's explicit width");
    }

    fn find_by_height(laid: &Laid, target_h: f32) -> Option<&Laid> {
        if laid.style.height.map(BoxLength::px) == Some(target_h) {
            return Some(laid);
        }
        if let LaidContent::Blocks(children) = &laid.content {
            for c in children {
                if let Some(found) = find_by_height(c, target_h) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn find_by_width(laid: &Laid, target_w: f32) -> Option<&Laid> {
        if laid.style.width.map(BoxLength::px) == Some(target_w) {
            return Some(laid);
        }
        if let LaidContent::Blocks(children) = &laid.content {
            for c in children {
                if let Some(found) = find_by_width(c, target_w) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn explicit_height_overrides_reported_size_but_children_still_fully_laid_out() {
        let laid = measure_fixture(
            r#"<html><body><div style="height: 10px;"><p>line one</p><p>line two</p></div></body></html>"#,
            400.0,
        );
        let outer = find_by_height(&laid, 10.0).expect("div with explicit height not found");
        assert_eq!(outer.h, 10.0, "the div's own reported height should be exactly the explicit value");
        let LaidContent::Blocks(children) = &outer.content else {
            panic!("expected the div's content to be Blocks (two <p> children)");
        };
        assert_eq!(children.len(), 2, "both paragraphs should still be present, not truncated");
        for p in children {
            assert!(p.h > 0.0, "each paragraph should still be fully laid out with its own real height, not clipped away");
        }
    }

    #[test]
    fn explicit_height_larger_than_content_extends_the_box() {
        let laid = measure_fixture(r#"<html><body><div style="height: 300px;">x</div></body></html>"#, 400.0);
        let div = find_by_height(&laid, 300.0).expect("div with explicit height not found");
        assert_eq!(div.h, 300.0, "explicit height should win over the much-shorter natural content height");
    }

    // ---------- v43: box-sizing + min/max-width/height ----------

    #[test]
    fn box_sizing_border_box_shrinks_content_to_fit_padding_and_border() {
        let laid = measure_fixture(
            r#"<html><body><div style="box-sizing: border-box; width: 100px; padding: 10px; border: 5px solid black;">x</div></body></html>"#,
            500.0,
        );
        let div = find_by_width(&laid, 100.0).expect("div with explicit width not found");
        assert_eq!(
            div.w, 100.0,
            "border-box: the specified width is the box's total width, with padding/border carved out of it rather than added on top"
        );
    }

    #[test]
    fn box_sizing_border_box_applies_to_height_too() {
        let laid = measure_fixture(
            r#"<html><body><div style="box-sizing: border-box; height: 50px; padding: 10px;">x</div></body></html>"#,
            400.0,
        );
        let div = find_by_height(&laid, 50.0).expect("div with explicit height not found");
        assert_eq!(
            div.h, 50.0,
            "border-box: the specified height is the box's total height, not the content height with padding added on top"
        );
    }

    #[test]
    fn min_width_can_force_the_box_wider_than_its_container() {
        let laid = measure_fixture(
            r#"<html><body><div style="min-width: 300px; margin-top: 1px;">x</div></body></html>"#,
            100.0,
        );
        let div = find_laid_by_margin_top(&laid, 1.0).expect("div not found");
        assert_eq!(
            div.w, 300.0,
            "min-width should win even past the container's own available width, same as real CSS"
        );
    }

    #[test]
    fn max_width_clamps_the_box_narrower_than_an_explicit_width() {
        let laid = measure_fixture(
            r#"<html><body><div style="width: 500px; max-width: 200px;">x</div></body></html>"#,
            900.0,
        );
        let div = find_by_width(&laid, 500.0).expect("div with explicit width not found");
        assert_eq!(div.w, 200.0, "max-width should clamp the box below its own explicit width, same as real CSS");
    }

    #[test]
    fn min_width_wins_when_author_declares_min_greater_than_max() {
        let laid = measure_fixture(
            r#"<html><body><div style="width: 50px; min-width: 400px; max-width: 200px;">x</div></body></html>"#,
            900.0,
        );
        let div = find_by_width(&laid, 50.0).expect("div with explicit width not found");
        assert_eq!(
            div.w, 400.0,
            "when min-width exceeds max-width, the used value is min-width, never something smaller (real CSS clamp order: max first, then min overrides)"
        );
    }

    #[test]
    fn max_height_clamps_reported_height_but_children_still_fully_laid_out() {
        let laid = measure_fixture(
            r#"<html><body><div style="max-height: 10px; margin-top: 1px;"><p>line one</p><p>line two</p></div></body></html>"#,
            400.0,
        );
        let outer = find_laid_by_margin_top(&laid, 1.0).expect("div not found");
        assert_eq!(outer.h, 10.0, "max-height should clamp the box's own reported height even though height itself is auto");
        let LaidContent::Blocks(children) = &outer.content else {
            panic!("expected the div's content to be Blocks (two <p> children)");
        };
        assert_eq!(children.len(), 2, "both paragraphs should still be present, not truncated");
        for p in children {
            assert!(p.h > 0.0, "each paragraph should still be fully laid out with its own real height, not clipped away");
        }
    }

    // ---------- v44: structural pseudo-classes (`:not()`, `:nth-child()`, `:first-child`) ----------
    // These aren't `OurPseudoClass` variants at all — `:link`/`:visited` need custom
    // state scratchy tracks itself, but `:not()`/`:nth-child()`/`:first-child` are
    // built into the `selectors` crate's own matching, driven purely by the
    // `SelectorsElement` methods (`prev_sibling_element`, `next_sibling_element`,
    // `is_root`, ...) already implemented for `El`. Never actually verified before.

    fn find_all_span_colors(b: &LayoutBox, out: &mut Vec<TextColor>) {
        if let BoxContent::Inline(items) = &b.content {
            for item in items {
                if let InlineItem::Text(_, s) = item {
                    out.push(s.color);
                }
            }
        } else if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                find_all_span_colors(c, out);
            }
        }
    }

    #[test]
    fn not_pseudo_excludes_matching_element() {
        let doc = parse_html(
            r#"<html><head><style>p:not(.skip) { color: #ff0000; }</style></head><body><p>one</p><p class="skip">two</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let mut colors = Vec::new();
        find_all_span_colors(&tree, &mut colors);
        assert_eq!(colors.len(), 2, "expected two paragraphs' worth of spans");
        assert_eq!(colors[0], TextColor::rgb(0xff, 0, 0), ":not(.skip) should match the first <p>");
        assert_ne!(colors[1], TextColor::rgb(0xff, 0, 0), ":not(.skip) should not match the second <p>");
    }

    #[test]
    fn nth_child_matches_correct_position() {
        let doc = parse_html(
            r#"<html><head><style>p:nth-child(2) { color: #00ff00; }</style></head><body><p>one</p><p>two</p><p>three</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let mut colors = Vec::new();
        find_all_span_colors(&tree, &mut colors);
        assert_eq!(colors.len(), 3);
        assert_ne!(colors[0], TextColor::rgb(0, 0xff, 0), ":nth-child(2) should not match the 1st <p>");
        assert_eq!(colors[1], TextColor::rgb(0, 0xff, 0), ":nth-child(2) should match the 2nd <p>");
        assert_ne!(colors[2], TextColor::rgb(0, 0xff, 0), ":nth-child(2) should not match the 3rd <p>");
    }

    #[test]
    fn first_child_matches_only_first() {
        let doc = parse_html(
            r#"<html><head><style>p:first-child { color: #0000ff; }</style></head><body><p>one</p><p>two</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let mut colors = Vec::new();
        find_all_span_colors(&tree, &mut colors);
        assert_eq!(colors.len(), 2);
        assert_eq!(colors[0], TextColor::rgb(0, 0, 0xff), ":first-child should match the 1st <p>");
        assert_ne!(colors[1], TextColor::rgb(0, 0, 0xff), ":first-child should not match the 2nd <p>");
    }

    // ---------- v45: CSS custom properties (`:root`/`html`-scoped) + `var()` ----------

    #[test]
    fn root_custom_prop_resolves_via_var() {
        let doc = parse_html(
            r#"<html><head><style>:root { --c: #ff0000; } p { color: var(--c); }</style></head><body><p>hi</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(color, TextColor::rgb(0xff, 0, 0), "var(--c) should resolve to the :root-declared value");
    }

    #[test]
    fn html_selector_also_scopes_a_custom_prop() {
        let doc = parse_html(
            r#"<html><head><style>html { --c: #0000ff; } p { color: var(--c); }</style></head><body><p>hi</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(color, TextColor::rgb(0, 0, 0xff), "var(--c) should resolve via a plain `html` selector too");
    }

    #[test]
    fn var_fallback_used_when_prop_undeclared() {
        let doc = parse_html(
            r#"<html><head><style>p { color: var(--undefined, #00ff00); }</style></head><body><p>hi</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(color, TextColor::rgb(0, 0xff, 0), "an undeclared custom property should fall back to the fallback value");
    }

    #[test]
    fn nested_var_fallback_resolves() {
        let doc = parse_html(
            r#"<html><head><style>p { color: var(--a, var(--b, #112233)); }</style></head><body><p>hi</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(color, TextColor::rgb(0x11, 0x22, 0x33), "a nested var() inside a fallback should itself resolve");
    }

    #[test]
    fn non_root_custom_prop_is_not_picked_up() {
        let doc = parse_html(
            r#"<html><head><style>.scope { --c: #ff0000; } p { color: var(--c, #00ff00); }</style></head><body><p class="scope">hi</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(
            color,
            TextColor::rgb(0, 0xff, 0),
            "a custom property declared on a non-root-matching selector shouldn't be visible to var() (documented scope limit)"
        );
    }

    // ---------- v46: `<img>` rendering ----------

    /// Builds a solid-color PNG in memory and returns it as a `data:` URI
    /// — avoids hardcoding a base64 blob in test source; exercises the
    /// real `image`-crate decode path the same way a real `<img src>`
    /// would, just generated instead of fetched.
    fn tiny_png_data_uri(width: u32, height: u32) -> String {
        use base64::Engine;
        let img = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_fn(width, height, |_, _| {
            image::Rgba([255u8, 0, 0, 255])
        }));
        let mut bytes = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes))
    }

    fn find_image_laid(laid: &Laid) -> Option<&Laid> {
        if matches!(laid.content, LaidContent::Image(_)) {
            return Some(laid);
        }
        if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
            for c in children {
                if let Some(found) = find_image_laid(c) {
                    return Some(found);
                }
            }
        }
        // A default-`display: inline` `<img>` (the common case since
        // v48) lands inside a `MixedInline` run instead of its own
        // top-level `LaidContent::Image` — see `MixedPositioned`'s own
        // docs.
        if let LaidContent::MixedInline(items) = &laid.content {
            for item in items {
                if let MixedPositioned::Image(img_laid) = item {
                    return Some(img_laid);
                }
            }
        }
        None
    }

    #[test]
    fn img_with_no_css_or_attrs_uses_intrinsic_size() {
        let src = tiny_png_data_uri(40, 20);
        let html = format!(r#"<html><body><img src="{src}"></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        let img = find_image_laid(&laid).expect("img box not found");
        assert_eq!((img.w, img.h), (40.0, 20.0), "auto width/height should fall back to the decoded intrinsic size");
    }

    #[test]
    fn img_css_width_only_scales_height_by_intrinsic_aspect_ratio() {
        let src = tiny_png_data_uri(40, 20); // 2:1
        let html = format!(r#"<html><body><img src="{src}" style="width: 100px"></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        let img = find_image_laid(&laid).expect("img box not found");
        assert_eq!((img.w, img.h), (100.0, 50.0), "height: auto should scale proportionally from the explicit width");
    }

    #[test]
    fn img_css_height_only_scales_width_by_intrinsic_aspect_ratio() {
        let src = tiny_png_data_uri(40, 20); // 2:1
        let html = format!(r#"<html><body><img src="{src}" style="height: 50px"></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        let img = find_image_laid(&laid).expect("img box not found");
        assert_eq!((img.w, img.h), (100.0, 50.0), "width: auto should scale proportionally from the explicit height");
    }

    #[test]
    fn img_html_width_height_attrs_used_when_css_leaves_both_auto() {
        let src = tiny_png_data_uri(40, 20);
        let html = format!(r#"<html><body><img src="{src}" width="80" height="90"></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        let img = find_image_laid(&laid).expect("img box not found");
        assert_eq!((img.w, img.h), (80.0, 90.0), "HTML width/height attributes should size the image when no CSS size is declared");
    }

    #[test]
    fn img_css_size_overrides_html_attrs() {
        let src = tiny_png_data_uri(40, 20);
        let html = format!(r#"<html><body><img src="{src}" width="80" height="90" style="width: 10px; height: 10px"></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        let img = find_image_laid(&laid).expect("img box not found");
        assert_eq!((img.w, img.h), (10.0, 10.0), "a declared CSS size should win over the HTML width/height attributes");
    }

    fn find_blocks_containing_image(b: &LayoutBox) -> Option<&[LayoutBox]> {
        if let BoxContent::Blocks(children) = &b.content {
            if children.iter().any(|c| matches!(c.content, BoxContent::Image(_))) {
                return Some(children);
            }
            for c in children {
                if let Some(found) = find_blocks_containing_image(c) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn find_inline_run_with_image(b: &LayoutBox) -> Option<&[InlineItem]> {
        if let BoxContent::Inline(items) = &b.content {
            if items.iter().any(|i| matches!(i, InlineItem::Image { .. })) {
                return Some(items);
            }
        }
        if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                if let Some(found) = find_inline_run_with_image(c) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn img_with_default_display_shares_the_inline_run_with_surrounding_text() {
        // Pre-v48 behavior: a default-`display: inline` `<img>` always
        // broke surrounding text into separate sibling boxes. v48
        // reverses this — see TODO.txt/NOTES.md — so this now pins the
        // *opposite*: one `BoxContent::Inline` run holding text before,
        // the image, and text after, in source order, ready for real
        // inline-level flow in `measure_mixed_inline`.
        let src = tiny_png_data_uri(2, 2);
        let html = format!(r#"<html><body><p>before <img src="{src}"> after</p></body></html>"#);
        let doc = parse_html(&html);
        let tree = build_layout_tree(&doc);
        let items = find_inline_run_with_image(&tree).expect("<p> should be one inline run containing the image");
        assert_eq!(items.len(), 3, "text before/the image/text after should be 3 items in the SAME inline run");
        let InlineItem::Text(before, _) = &items[0] else { panic!("expected text before the image") };
        assert!(before.trim_start().starts_with("before"), "unexpected text before the image: {before:?}");
        assert!(matches!(items[1], InlineItem::Image { .. }));
        let InlineItem::Text(after, _) = &items[2] else { panic!("expected text after the image") };
        assert!(after.trim_start().starts_with("after"), "unexpected text after the image: {after:?}");
    }

    #[test]
    fn img_with_explicit_display_block_still_gets_its_own_box() {
        // The one case that should still behave like pre-v48: an author
        // explicitly overriding `<img>`'s default `display: inline` to
        // `block` is real block-level markup, not inline flow — same
        // treatment as any other block element, not folded into the
        // surrounding text run.
        let src = tiny_png_data_uri(2, 2);
        let html = format!(r#"<html><body><p>before <img src="{src}" style="display: block"> after</p></body></html>"#);
        let doc = parse_html(&html);
        let tree = build_layout_tree(&doc);
        let children = find_blocks_containing_image(&tree).expect("display:block <img> should still split into sibling boxes");
        assert_eq!(children.len(), 3);
        assert!(matches!(children[1].content, BoxContent::Image(_)));
    }

    fn find_inline_text(b: &LayoutBox) -> Option<&str> {
        if let BoxContent::Inline(items) = &b.content {
            for item in items {
                if let InlineItem::Text(text, _) = item {
                    return Some(text.as_str());
                }
            }
        }
        if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                if let Some(found) = find_inline_text(c) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn img_with_missing_src_falls_back_to_alt_text() {
        let doc = parse_html(r#"<html><body><img alt="a fallback"></body></html>"#);
        let tree = build_layout_tree(&doc);
        let text = find_inline_text(&tree).expect("no text found");
        assert!(text.contains("a fallback"), "a missing src should fall back to rendering alt text: {text:?}");
    }

    #[test]
    fn img_with_undecodable_src_falls_back_to_alt_text() {
        let doc = parse_html(r#"<html><body><img src="data:image/png;base64,not-valid-base64!!" alt="broken img"></body></html>"#);
        let tree = build_layout_tree(&doc);
        let text = find_inline_text(&tree).expect("no text found");
        assert!(text.contains("broken img"), "undecodable image data should fall back to rendering alt text: {text:?}");
    }

    // ---------- v47: `background-image` ----------

    #[test]
    fn tile_image_repeats_the_source_image_across_a_larger_box() {
        const W: u32 = 8;
        const H: u32 = 2;
        let mut pixels = vec![0u32; (W * H) as usize];
        let red = TextColor::rgb(255, 0, 0);
        let blue = TextColor::rgb(0, 0, 255);
        let image = DecodedImage { width: 2, height: 1, pixels: vec![red, blue] };
        tile_image(&image, &mut pixels, W, H, 0.0, 0.0, 8.0, 1.0, None);
        let px = |x: u32| pixels[x as usize] & 0x00FFFFFF;
        assert_eq!(px(0), 0xFF0000);
        assert_eq!(px(1), 0x0000FF);
        assert_eq!(px(2), 0xFF0000, "a 2px-wide image should repeat, not stretch, across a wider box");
        assert_eq!(px(3), 0x0000FF);
        assert_eq!(px(6), 0xFF0000);
        assert_eq!(px(7), 0x0000FF);
    }

    #[test]
    fn background_image_css_property_paints_behind_border() {
        let src = tiny_png_data_uri(4, 4); // solid red, see `tiny_png_data_uri`
        let html = format!(
            r#"<html><body><div style="width: 40px; height: 40px; border: 4px solid blue; background-image: url('{src}')"></div></body></html>"#
        );
        let doc = parse_html(&html);
        let tree = build_layout_tree(&doc);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure(
            &tree, &mut font_system, 0.0, 0.0, 200.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
            &mut Vec::new(), &Floats::default(), None,
        );
        let mut swash_cache = SwashCache::new();
        let (win_w, win_h) = (100u32, 100u32);
        let mut pixels = vec![0u32; (win_w * win_h) as usize];
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &mut pixels, win_w, win_h, 0.0, None);

        // `body { margin: 8px }` (default.css) offsets everything by 8px —
        // border-box spans roughly x/y 8..56 (4px border + 40px content +
        // 4px border), content (background-image) area roughly 12..52.
        let px = |x: u32, y: u32| pixels[(y * win_w + x) as usize] & 0x00FFFFFF;
        assert_eq!(px(30, 30), 0xFF0000, "background-image should paint inside the border");
        assert_eq!(px(9, 30), 0x0000FF, "border itself should still paint on top of/around the background-image");
    }

    #[test]
    fn no_background_image_declared_resolves_to_none() {
        let laid = measure_fixture(r#"<html><body><div style="width: 13px; height: 13px; background-color: #00ff00;"></div></body></html>"#, 200.0);
        let div = find_by_width(&laid, 13.0).expect("div not found");
        assert!(div.background_image.is_none(), "a div with no background-image declared shouldn't resolve one, even with a background-color set");
        assert!(div.style.background_color.is_some(), "sanity: background-color itself should still resolve normally");
    }

    // ---------- v48: real inline-level `<img>` flow ----------

    fn find_mixed_inline(laid: &Laid) -> Option<&Laid> {
        if matches!(laid.content, LaidContent::MixedInline(_)) {
            return Some(laid);
        }
        if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
            for c in children {
                if let Some(found) = find_mixed_inline(c) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn mixed_items(laid: &Laid) -> &Vec<MixedPositioned> {
        let LaidContent::MixedInline(items) = &laid.content else { panic!("not a MixedInline Laid") };
        items
    }

    fn mixed_item_xy(item: &MixedPositioned) -> (f32, f32) {
        match item {
            MixedPositioned::Image(img) => (img.x, img.y),
            MixedPositioned::Word { x, y, .. } => (*x, *y),
        }
    }

    #[test]
    fn mixed_inline_keeps_items_on_one_line_when_they_fit() {
        let src = tiny_png_data_uri(4, 4);
        let html = format!(r#"<html><body><p>x <img src="{src}" style="width: 10px; height: 10px"> y</p></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        let run = find_mixed_inline(&laid).expect("mixed inline run not found");
        let items = mixed_items(run);
        assert_eq!(items.len(), 3, "word, image, word");
        // One line's height is `max(word line-height, image height)` —
        // the word's own `Metrics::new(16.0, 20.0)` line-height (20px)
        // dominates a 10px-tall image, so the whole run's box should be
        // ~20px tall. Two separate lines would instead be ~40px+ (each
        // line at least 20px, image bottom-aligned within whichever is
        // taller) — a far coarser, implementation-detail-free way to
        // confirm "on one line" than comparing exact `y` offsets (the
        // word is top-aligned and the image bottom-aligned *within* a
        // shared line, see `measure_mixed_inline`'s own docs, so their
        // `y` values legitimately differ even on the same line).
        assert!(run.h < 25.0, "word+image should fit on one ~20px line, got height {}", run.h);
        let positions: Vec<(f32, f32)> = items.iter().map(mixed_item_xy).collect();
        assert!(positions[0].0 < positions[1].0, "image should sit after the first word, not stacked below it");
        assert!(positions[1].0 < positions[2].0, "trailing word should sit after the image");
    }

    #[test]
    fn mixed_inline_wraps_when_content_does_not_fit_on_one_line() {
        // Two 50px-wide images in a 60px container can't possibly share
        // a line (50 + 50 > 60) regardless of font metrics, so this
        // pins real wrapping deterministically without depending on how
        // wide any shaped word measures.
        let src = tiny_png_data_uri(4, 4);
        let html = format!(
            r#"<html><body><div style="width: 60px"><p><img src="{src}" style="width: 50px; height: 10px"><img src="{src}" style="width: 50px; height: 10px"></p></div></body></html>"#
        );
        let laid = measure_fixture(&html, 500.0);
        let run = find_mixed_inline(&laid).expect("mixed inline run not found");
        let items = mixed_items(run);
        assert_eq!(items.len(), 2);
        let positions: Vec<(f32, f32)> = items.iter().map(mixed_item_xy).collect();
        assert_ne!(positions[0].1, positions[1].1, "two 50px images in a 60px container should wrap onto separate lines");
        assert!(run.h >= 20.0 - 0.01, "two wrapped 10px-tall image lines should report roughly 20px total, got {}", run.h);
    }

    #[test]
    fn mixed_inline_respects_text_align_center() {
        let src = tiny_png_data_uri(4, 4);
        let left_html = format!(r#"<html><body><p><img src="{src}" style="width: 10px; height: 10px"></p></body></html>"#);
        let center_html = format!(r#"<html><body><p style="text-align: center"><img src="{src}" style="width: 10px; height: 10px"></p></body></html>"#);
        let left_laid = measure_fixture(&left_html, 500.0);
        let center_laid = measure_fixture(&center_html, 500.0);
        let (left_x, _) = mixed_item_xy(&mixed_items(find_mixed_inline(&left_laid).unwrap())[0]);
        let (center_x, _) = mixed_item_xy(&mixed_items(find_mixed_inline(&center_laid).unwrap())[0]);
        assert!(center_x > left_x, "text-align: center should shift a mixed line's content rightward compared to default left alignment");
    }

    // ---------- v49: `%` lengths, and confirming unitless-non-zero stays rejected ----------

    #[test]
    fn percent_width_resolves_against_containing_block_width() {
        // `body { margin: 8px }` (default.css) eats 16px total, so a
        // 416px window gives body a 400px content width — the div's own
        // containing block.
        let laid = measure_fixture(r#"<html><body><div style="width: 50%;">x</div></body></html>"#, 416.0);
        let div = find_by_width(&laid, 200.0).expect("div's width: 50% should resolve to 200px");
        assert_eq!(div.w, 200.0);
    }

    #[test]
    fn percent_margin_resolves_against_containing_block_width_on_every_side() {
        // Real CSS resolves a margin percentage against the containing
        // block's *width* on every side, even top/bottom — not against
        // the margined box's own (much narrower, unrelated) width. The
        // outer div pins the containing block's content width at
        // exactly 1000px so there's an unambiguous basis to check
        // against.
        let laid = measure_fixture(
            r#"<html><body><div style="width: 1000px;"><div style="width: 20px; margin-top: 10%; margin-left: 10%;">inner</div></div></body></html>"#,
            2000.0,
        );
        let inner = find_by_width(&laid, 20.0).expect("inner div not found");
        assert_eq!(
            inner.style.margin.top.px(),
            100.0,
            "margin-top: 10% should resolve against the containing block's 1000px width, not inner's own 20px"
        );
        assert_eq!(inner.style.margin.left.px(), 100.0);
    }

    #[test]
    fn percent_padding_top_resolves_against_containing_block_width_not_height() {
        // Same point as the margin test above, isolated for `padding`
        // specifically (a separate `Declarations`/cascade path) and
        // against a box whose own *height* could plausibly be
        // (wrongly) used as the basis instead, if padding-top's real
        // CSS basis (width, same as every other side) weren't followed.
        let laid = measure_fixture(
            r#"<html><body><div style="width: 1000px;"><div style="height: 10px; padding-top: 10%;">inner</div></div></body></html>"#,
            2000.0,
        );
        let inner = find_by_height(&laid, 10.0).expect("inner div not found");
        assert_eq!(
            inner.style.padding.top.px(),
            100.0,
            "padding-top: 10% must resolve against the containing block's 1000px width, not inner's own 10px height"
        );
    }

    #[test]
    fn percent_height_resolves_against_definite_containing_block_height() {
        let laid = measure_fixture(
            r#"<html><body><div style="height: 300px;"><div style="height: 50%;">inner</div></div></body></html>"#,
            500.0,
        );
        let inner = find_by_height(&laid, 150.0)
            .expect("inner div's height: 50% should resolve to 150px against its 300px containing block");
        assert_eq!(inner.h, 150.0);
    }

    #[test]
    fn percent_height_computes_to_auto_when_containing_block_height_is_indefinite() {
        // Neither `html` nor `body` declares an explicit height in
        // `default.css`, so the div's containing block height is
        // indefinite (content-driven) here — real CSS Level 2's
        // documented rule for exactly this case: the percentage
        // computes to `auto`, i.e. exactly as if `height` weren't
        // declared at all, not some value resolved against a bogus
        // implicit basis. Checked by comparing against the identical
        // markup with the declaration removed entirely — if `%` were
        // wrongly resolving against something (e.g. `0`, or this box's
        // own width), the two reported heights would differ.
        let with_percent = measure_fixture(
            r#"<html><body><div style="height: 50%; padding: 5px;">hello</div></body></html>"#,
            500.0,
        );
        let without = measure_fixture(r#"<html><body><div style="padding: 5px;">hello</div></body></html>"#, 500.0);
        assert_eq!(
            with_percent.h, without.h,
            "height: 50% against an indefinite containing block should have no effect at all, same as auto"
        );
    }

    #[test]
    fn unitless_nonzero_number_is_still_rejected_for_margin() {
        // This TODO item's other half: confirming a plain `<length>`
        // property (unlike `line-height`, which legitimately accepts a
        // bare number) still only accepts unitless *zero*, not a
        // general unitless number — `margin-top: 5` (no unit) must drop
        // the whole declaration, falling back to the real initial value
        // `0`, not be (wrongly) treated as `5px`.
        let laid =
            measure_fixture(r#"<html><body><div style="width: 123px; margin-top: 5;">x</div></body></html>"#, 500.0);
        let div = find_by_width(&laid, 123.0).expect("div not found");
        assert_eq!(div.style.margin.top.px(), 0.0, "a unitless non-zero margin-top must be dropped, not treated as 5px");
    }

    #[test]
    fn line_height_percent_resolves_against_own_font_size() {
        // `line-height`'s `%` form resolves against the *declaring*
        // element's own font-size (same timing as its `em` form, and
        // unlike its bare-`<number>` form, which stays symbolic and
        // re-resolves per descendant — see `LineHeight`'s own docs).
        let laid = measure_fixture(
            r#"<html><body><div style="font-size: 20px; line-height: 150%;">x</div></body></html>"#,
            500.0,
        );
        let div = find_by_laid_font_size(&laid, 20.0).expect("div with font-size: 20px not found");
        match div.style.line_height {
            LineHeight::Len(l) => assert_eq!(l.resolve(div.style.font_size, div.style.root_font_size), 30.0),
            _ => panic!("line-height: 150% should parse as LineHeight::Len(Length::Percent), not Normal/Multiplier"),
        }
    }

    fn find_by_laid_font_size(laid: &Laid, target: f32) -> Option<&Laid> {
        if laid.style.font_size == target {
            return Some(laid);
        }
        if let LaidContent::Blocks(children) = &laid.content {
            for c in children {
                if let Some(found) = find_by_laid_font_size(c, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn border_radius_percent_resolves_against_own_border_box_width() {
        // `border-radius`'s `%` basis is this box's own border-box
        // width (unlike `width`/`margin`/`padding`, whose basis is the
        // *containing block's* width) — resolved directly in
        // `paint_box_decoration` against its own `w`, not in `measure`.
        let laid = measure_fixture(
            r#"<html><body><div style="width: 200px; height: 50px; border-radius: 25%;">x</div></body></html>"#,
            500.0,
        );
        let div = find_by_width(&laid, 200.0).expect("div not found");
        assert_eq!(div.style.border_radius.resolve(div.w), 50.0, "border-radius: 25% of a 200px-wide box should resolve to 50px");
    }

    // ---------- table layout: real CSS auto table layout (min/max content width per column) ----------

    /// The first `LaidContent::TableRow`'s own cells, found by walking
    /// through any number of wrapping `Blocks` boxes (a `<table>`'s own
    /// content, or `<body>`/`<html>` above it) — same recursive-search
    /// shape as this file's other `find_*` test helpers.
    fn find_table_row_cells(laid: &Laid) -> Option<&[Laid]> {
        if let LaidContent::TableRow(cells) = &laid.content {
            return Some(cells);
        }
        if let LaidContent::Blocks(children) = &laid.content {
            for c in children {
                if let Some(found) = find_table_row_cells(c) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn table_columns_size_from_their_own_cell_content_not_an_equal_split() {
        // Two columns, deliberately very uneven content: the old equal-
        // width-column layout would give both exactly half of the
        // table's content width regardless. Real auto table layout
        // should size each column from its own cells' content instead.
        let laid = measure_fixture(
            r#"<table><tr><td>Hi</td><td>This is a much longer piece of cell text that genuinely needs a lot more horizontal room than the first column does</td></tr></table>"#,
            900.0,
        );
        let cells = find_table_row_cells(&laid).expect("table row not found");
        assert_eq!(cells.len(), 2, "two <td>s");
        assert!(
            cells[1].w > cells[0].w * 3.0,
            "the long-text column ({}) should end up substantially wider than the short one ({}), not an equal 450/450 split",
            cells[1].w,
            cells[0].w
        );
    }

    #[test]
    fn table_column_widths_still_fill_the_tables_own_content_width() {
        // Real per-column sizing changes how `content_w` is *divided*,
        // not the table's own box-model width — an auto-width table
        // still fills its container exactly as it did with equal-width
        // columns (see `measure_table`'s own docs on why changing that
        // is a separate, bigger scope). `body { margin: 8px }` (default.
        // css) eats 16px total, so a 900px window gives the table a
        // 884px content width.
        let laid = measure_fixture(
            r#"<table><tr><td>a</td><td>a much much longer second column of text here</td></tr></table>"#,
            900.0,
        );
        let cells = find_table_row_cells(&laid).expect("table row not found");
        let total: f32 = cells.iter().map(|c| c.w).sum();
        assert!((total - 884.0).abs() < 1.0, "columns should sum to the table's own 884px content width, got {total}");
    }

    #[test]
    fn table_narrow_container_shrinks_columns_proportionally_to_min_content_not_below_their_longest_word() {
        // A container too narrow for either column's own natural
        // (unwrapped) width forces both to shrink below their `max` —
        // but a column still shouldn't collapse to less than its own
        // widest unbreakable word (`min`-content): real CSS lets content
        // overflow rather than breaking a word, the same `min-width`-
        // wider-than-container behavior this engine already has
        // elsewhere (see `clamp_box_dim`). Both columns here are a
        // single long word each (own `min` == own `max`, so this
        // exercises the "even every column's `min` doesn't fit" shrink
        // branch, not the ordinary interpolate-between-min-and-max one).
        let laid = measure_fixture(
            r#"<table><tr><td>Supercalifragilisticexpialidocious</td><td>Pneumonoultramicroscopicsilicovolcanoconiosis</td></tr></table>"#,
            60.0,
        );
        let cells = find_table_row_cells(&laid).expect("table row not found");
        // The second word is longer than the first, so even shrunk
        // proportionally to each column's own min-content share, it
        // should still end up wider than the first column, not clamped
        // to some shared minimum.
        assert!(
            cells[1].w > cells[0].w,
            "the longer single-word column ({}) should still end up wider than the shorter one ({}) even once both are shrunk",
            cells[1].w,
            cells[0].w
        );
    }

    #[test]
    fn nested_table_in_a_cell_gets_its_own_independent_column_sizing() {
        // v15's own "deliberately pathological" case, still true after
        // real per-column sizing: a `<table>` nested inside a cell of an
        // outer table isn't flattened into the outer table's own row —
        // `cell_natural_width`'s `Blocks`/`TableRow` recursion has to
        // walk down into it without mistaking its rows for the outer
        // table's own columns.
        let laid = measure_fixture(
            r#"<table><tr><td>outer cell</td><td><table><tr><td>inner a</td><td>inner cell with much longer text in it</td></tr></table></td></tr></table>"#,
            900.0,
        );
        let outer_cells = find_table_row_cells(&laid).expect("outer table row not found");
        assert_eq!(outer_cells.len(), 2, "outer table still has exactly 2 columns, not the inner table's 2 flattened in");
        let inner_cells = find_table_row_cells(&outer_cells[1]).expect("nested table's own row not found inside the second outer cell");
        assert_eq!(inner_cells.len(), 2, "inner table's own row has its own 2 cells");
        assert!(
            inner_cells[1].w > inner_cells[0].w,
            "the inner table's own uneven columns ({}, {}) should still size from their own content",
            inner_cells[0].w,
            inner_cells[1].w
        );
    }

    // ---------- v51: named `font-family` fallback lists ----------

    fn find_first_span_font_family(b: &LayoutBox) -> Option<FontFamily> {
        if let BoxContent::Inline(items) = &b.content {
            if let Some(InlineItem::Text(_, s)) = items.first() {
                return Some(s.font_family);
            }
        } else if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                if let Some(found) = find_first_span_font_family(c) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn font_family_parses_quoted_multiword_and_generic_fallback_list() {
        // A quoted name (`'Georgia'`), an unquoted multi-word name
        // (`Helvetica Neue` — real CSS grammar joins bare idents with
        // spaces into one family name), and a trailing bare generic
        // keyword, all in one comma-separated list.
        let doc = parse_html(
            r#"<html><body><p style="font-family: 'Georgia', Helvetica Neue, monospace">x</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let family = find_first_span_font_family(&tree).expect("no span found");
        assert_eq!(
            family.0,
            [
                FontFamilyItem::Name("Georgia"),
                FontFamilyItem::Name("Helvetica Neue"),
                FontFamilyItem::Monospace,
            ]
            .as_slice(),
            "expected the quoted name, the joined multi-word name, and the trailing generic, in order"
        );
    }

    #[test]
    fn font_family_single_generic_keyword_still_works() {
        // v16's original scope (a single bare generic, no list at all)
        // must still parse to exactly one item — no regression from
        // generalizing the grammar to a full list.
        let doc = parse_html(r#"<html><body><code>x</code></body></html>"#);
        let tree = build_layout_tree(&doc);
        let family = find_first_span_font_family(&tree).expect("no span found");
        assert_eq!(family.0, [FontFamilyItem::Monospace].as_slice());
    }

    #[test]
    fn font_family_inherits_through_the_cascade() {
        // Real CSS inheritance (same family as color/font-size/etc.,
        // per PROMPTS.md v16) — a descendant with no `font-family` rule
        // of its own takes its ancestor's resolved fallback list, not
        // the default.
        let doc = parse_html(
            r#"<html><body><div style="font-family: 'Georgia', serif"><span>x</span></div></body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let family = find_first_span_font_family(&tree).expect("no span found");
        assert_eq!(family.0, [FontFamilyItem::Name("Georgia"), FontFamilyItem::Serif].as_slice());
    }

    fn collect_text_shape_ids(laid: &Laid, ids: &mut Vec<usize>) {
        match &laid.content {
            LaidContent::Text(id) => ids.push(*id),
            LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
                for c in children {
                    collect_text_shape_ids(c, ids);
                }
            }
            LaidContent::Image(_) | LaidContent::MixedInline(_) => {}
        }
    }

    #[test]
    fn font_family_fallback_resolves_through_the_real_font_system() {
        // Verified past the cascade, through to what actually gets
        // shaped — same diligence v16's own (since-removed, pre-v41)
        // test used: a field can cascade to the right enum value and
        // still not affect a single rendered glyph if the shaping path
        // never reads it correctly. This runs the full pipeline
        // (`build_layout_tree` + `measure`) on three paragraphs —
        // `monospace`, `sans-serif`, and a fallback list naming a family
        // that can't possibly be installed before a real `monospace`
        // keyword — and inspects the actually-shaped `cosmic_text::
        // Buffer`'s own `fontdb::ID` per paragraph (not scratchy's own
        // `FontFamily` value, which would only prove the enum carries
        // the right data, not that it reached a real font).
        // Deliberately no whitespace between the `<p>`s — each gets its
        // own anonymous inline box/shaped paragraph, but a newline
        // between them would be its own (4th, 5th, ...) whitespace-only
        // inline run with its own shape id too, breaking the assumption
        // that `collect_text_shape_ids` finds exactly one id per `<p>`.
        let html = r#"<html><body><p style="font-family: monospace">A</p><p style="font-family: sans-serif">A</p><p style="font-family: 'NoSuchFontXYZ123', monospace">A</p></body></html>"#;
        let doc = parse_html(html);
        let tree = build_layout_tree(&doc);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure(
            &tree, &mut font_system, 0.0, 0.0, 800.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
            &mut Vec::new(), &Floats::default(), None,
        );

        let mut ids = Vec::new();
        collect_text_shape_ids(&laid, &mut ids);
        assert_eq!(ids.len(), 3, "expected one shaped paragraph per <p>");

        let font_id_of = |id: usize| -> fontdb::ID {
            let (buffer, ..) = cache.get(&id).expect("missing shaped paragraph");
            buffer
                .layout_runs()
                .next()
                .and_then(|r| r.glyphs.first())
                .map(|g| g.font_id)
                .expect("no glyph was shaped")
        };

        let mono_id = font_id_of(ids[0]);
        let sans_id = font_id_of(ids[1]);
        let fallback_id = font_id_of(ids[2]);

        assert_ne!(mono_id, sans_id, "monospace and sans-serif should resolve to different fonts on this system");
        assert_eq!(
            fallback_id, mono_id,
            "an unavailable named family should fall through to the next entry in the list (monospace), not silently stay on cosmic-text's default sans-serif"
        );
    }

    // ---------- v52: positioning model (`position`/`z-index`) ----------

    /// Like `measure_fixture`, but through `measure_document` — the real
    /// top-level entry point, needed for anything that exercises `fixed`
    /// or an orphaned `absolute` (both only ever resolved at the document
    /// root; see `measure_document`'s own docs). Plain `measure_fixture`
    /// calls `measure` directly, which would silently *drop* either case
    /// (whatever bubbles out of its own disposable `pending` `Vec` is
    /// simply never resolved) — the right behavior for `measure` itself
    /// (it has no viewport to resolve them against), wrong for a test
    /// that wants to see the fully-resolved tree.
    fn measure_document_fixture(html: &str, width: f32, viewport_h: f32) -> Laid {
        let doc = parse_html(html);
        let tree = build_layout_tree(&doc);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        measure_document(&tree, &mut font_system, width, viewport_h, &mut next_id, &mut cache, deadline, &mut incomplete)
    }

    /// Searches the *pre-measure* `LayoutBox` tree (not a `Laid`) for a
    /// box whose own `position` is `pos` — used only by `absolute_
    /// position_computes_display_to_block`, where what's actually being
    /// tested is whether a box exists for it at all (see that test's own
    /// docs for why that's the right thing to check).
    fn find_box_by_position(b: &LayoutBox, pos: Position) -> Option<&LayoutBox> {
        if b.style.position == pos {
            return Some(b);
        }
        match &b.content {
            BoxContent::Blocks(children) | BoxContent::TableRow(children) => {
                for c in children {
                    if let Some(found) = find_box_by_position(c, pos) {
                        return Some(found);
                    }
                }
            }
            _ => {}
        }
        None
    }

    #[test]
    fn absolute_position_computes_display_to_block() {
        // A default-`display: inline` `<span>` with no block content of
        // its own is never given its own `LayoutBox` at all —
        // `collect_children` recurses straight into it, merging its text
        // into the surrounding run, so there would be *no* box anywhere
        // in the tree carrying its `position: absolute`, if the real
        // CSS "computed `display`" rule (spec §9.7) weren't applied. Its
        // mere existence here is the test.
        let doc = parse_html(r#"<html><body><span style="position: absolute; top: 0; left: 0;">text</span></body></html>"#);
        let tree = build_layout_tree(&doc);
        let span = find_box_by_position(&tree, Position::Absolute).expect(
            "an absolutely positioned <span> should still get its own box (real CSS computes its display to block, instead of being flattened into the surrounding inline run the way a default `display: inline` span would be)",
        );
        assert_eq!(span.style.display, Display::Block, "position: absolute should force computed display to block");
    }

    #[test]
    fn position_relative_offsets_paint_position_not_sibling_flow() {
        // Deliberately no whitespace between the `<div>`s — a literal
        // newline/indentation between block-level tags becomes its own
        // whitespace-only anonymous inline box, consuming real flow
        // height and defeating the exact-pixel deltas this test checks
        // (same lesson v51's own font-family test already hit).
        let laid = measure_fixture(
            r#"<html><body><div style="height: 20px;">first</div><div style="position: relative; top: 10px; left: 5px; height: 21px;">second</div><div style="height: 22px;">third</div></body></html>"#,
            400.0,
        );
        let first = find_by_height(&laid, 20.0).expect("first div not found");
        let second = find_by_height(&laid, 21.0).expect("second (relative) div not found");
        let third = find_by_height(&laid, 22.0).expect("third div not found");
        let first_x = first.x;
        let first_bottom = first.y + first.h;
        assert_eq!(second.y, first_bottom + 10.0, "a relative `top` offset should shift paint position");
        assert_eq!(second.x, first_x + 5.0, "a relative `left` offset should shift paint position");
        assert_eq!(
            third.y,
            first_bottom + second.h,
            "a later sibling's flow position must be exactly where it would land if the relative box had no offset at all — `position: relative` never affects normal flow"
        );
    }

    #[test]
    fn position_relative_percent_offset_resolves_against_containing_block() {
        let laid = measure_fixture(
            r#"<html><body><div style="width: 200px; height: 100px;"><div style="position: relative; top: 10%; left: 50%; height: 20px;">x</div></div></body></html>"#,
            400.0,
        );
        let outer = find_by_width(&laid, 200.0).expect("outer div not found");
        let (outer_x, outer_y) = (outer.x, outer.y);
        let inner = find_by_height(&laid, 20.0).expect("inner (relative) div not found");
        assert_eq!(inner.x, outer_x + 100.0, "left: 50% should resolve against the containing block's own 200px width");
        assert_eq!(inner.y, outer_y + 10.0, "top: 10% should resolve against the containing block's own 100px height");
    }

    #[test]
    fn position_absolute_removed_from_flow_and_anchored_to_positioned_ancestor() {
        // No inter-tag whitespace — same reason as the `relative` test
        // above (a whitespace-only text node would otherwise show up as
        // an extra `Blocks` child, breaking the exact `children.len()`
        // check below).
        let laid = measure_fixture(
            r#"<html><body><div style="position: relative; width: 300px; height: 100px; padding: 10px;"><div style="position: absolute; top: 5px; left: 5px; width: 50px; height: 50px;">abs</div><div style="height: 20px;">normal</div></div></body></html>"#,
            400.0,
        );
        let container = find_by_width(&laid, 300.0).expect("positioned container not found");
        let LaidContent::Blocks(children) = &container.content else { panic!("expected Blocks content") };
        assert_eq!(
            children.len(),
            1,
            "an absolutely positioned child must not occupy a slot in its container's normal flow"
        );
        let normal = &children[0];
        let content_y = container.y + container.style.padding.top.px();
        assert_eq!(
            normal.y, content_y,
            "the sibling after an absolutely positioned box should land exactly where it would if that box didn't exist"
        );

        assert_eq!(container.positioned.len(), 1, "the absolute child should resolve against its nearest positioned ancestor");
        let abs = &container.positioned[0];
        assert_eq!(abs.x, container.x + 5.0, "left resolves against the containing block's own padding edge");
        assert_eq!(abs.y, container.y + 5.0, "top resolves against the containing block's own padding edge");
        assert_eq!((abs.w, abs.h), (50.0, 50.0));
    }

    #[test]
    fn position_absolute_right_bottom_solve_from_box_own_size() {
        // No `left`/`top` at all — only `right`/`bottom`, which need
        // this box's own (just-measured) width/height to solve for its
        // final position (see `resolve_positioned`'s own docs).
        let laid = measure_fixture(
            r#"<html><body><div style="position: relative; width: 300px; height: 100px;">
<div style="position: absolute; right: 10px; bottom: 5px; width: 40px; height: 20px;"></div>
</div></body></html>"#,
            400.0,
        );
        let container = find_by_width(&laid, 300.0).expect("positioned container not found");
        let abs = container.positioned.first().expect("absolute child not resolved");
        assert_eq!(abs.x, container.x + 300.0 - 10.0 - 40.0, "right: 10px should solve against this box's own 40px width");
        assert_eq!(abs.y, container.y + 100.0 - 5.0 - 20.0, "bottom: 5px should solve against this box's own 20px height");
    }

    #[test]
    fn position_absolute_with_no_positioned_ancestor_and_fixed_both_bubble_to_document_root() {
        let laid = measure_document_fixture(
            r#"<html><body>
<div style="height: 3000px;"></div>
<div style="position: absolute; top: 5px; left: 5px; width: 10px; height: 10px;"></div>
<div style="position: fixed; top: 5px; left: 5px; width: 10px; height: 10px;"></div>
</body></html>"#,
            400.0,
            600.0,
        );
        assert_eq!(laid.positioned.len(), 2, "both an orphaned absolute and a fixed box should bubble all the way to the document root");
        let fixed_count = laid.positioned.iter().filter(|l| l.fixed).count();
        assert_eq!(fixed_count, 1, "exactly one of the two (the `position: fixed` one) should be flagged for scroll-independent painting");
        for item in &laid.positioned {
            assert_eq!(
                (item.x, item.y),
                (5.0, 5.0),
                "both resolve against the same viewport-sized containing block, regardless of the preceding 3000px-tall sibling"
            );
        }
    }

    #[test]
    fn position_fixed_anchors_to_viewport_not_wherever_it_sits_in_a_tall_document() {
        let laid = measure_document_fixture(
            r#"<html><body>
<div style="height: 3000px;"></div>
<div style="position: fixed; top: 10px; right: 20px; width: 50px; height: 30px;"></div>
</body></html>"#,
            400.0,
            600.0,
        );
        let fixed = laid.positioned.first().expect("fixed box not resolved");
        assert!(fixed.fixed);
        assert_eq!(fixed.y, 10.0, "top anchors against the 600px viewport, not the 3000px document");
        assert_eq!(fixed.x, 400.0 - 20.0 - 50.0, "right anchors against the 400px viewport width");
    }

    #[test]
    fn z_index_controls_paint_order_for_overlapping_absolute_boxes() {
        // The higher-`z-index` box (red) appears *first* in document
        // order but should still paint on top of the lower one (blue,
        // later in markup) wherever they overlap — document order alone
        // would give the opposite (wrong) answer here if `z-index`
        // weren't honored at all (blue painting last/on top).
        let html = r#"<html><body>
<div style="position: absolute; top: 0px; left: 0px; width: 40px; height: 40px; background-color: #ff0000; z-index: 1;"></div>
<div style="position: absolute; top: 10px; left: 10px; width: 40px; height: 40px; background-color: #0000ff; z-index: 0;"></div>
</body></html>"#;
        let doc = parse_html(html);
        let tree = build_layout_tree(&doc);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid =
            measure_document(&tree, &mut font_system, 200.0, 200.0, &mut next_id, &mut cache, deadline, &mut incomplete);
        let mut swash_cache = SwashCache::new();
        let (win_w, win_h) = (200u32, 200u32);
        let mut pixels = vec![0u32; (win_w * win_h) as usize];
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &mut pixels, win_w, win_h, 0.0, None);
        let px = pixels[20 * win_w as usize + 20];
        assert_eq!(
            px & 0x00FF_FFFF,
            0x00FF_0000,
            "the higher z-index box (red) should paint on top in the overlap region"
        );
    }

    // ---------- v53: `<head>`/`<title>`/`<meta>`/`<link>`/`<base>` are hidden by default ----------

    fn collect_all_text(b: &LayoutBox, out: &mut String) {
        match &b.content {
            BoxContent::Blocks(children) | BoxContent::TableRow(children) => {
                for c in children {
                    collect_all_text(c, out);
                }
            }
            BoxContent::Inline(items) => {
                for item in items {
                    if let InlineItem::Text(t, _) = item {
                        out.push_str(t);
                    }
                }
            }
            BoxContent::Image(_) => {}
        }
    }

    #[test]
    fn title_text_does_not_render() {
        let doc = parse_html(
            r#"<html><head><title>Secret Page Title</title><meta charset="utf-8"><link rel="stylesheet" href="x.css"></head><body>Visible Body Text</body></html>"#,
        );
        let tree = build_layout_tree(&doc);
        let mut text = String::new();
        collect_all_text(&tree, &mut text);
        assert!(!text.contains("Secret Page Title"), "title text must not render in the document body: {text:?}");
        assert!(text.contains("Visible Body Text"), "body text should still render normally: {text:?}");
    }

    // ---------- v54: `float`/`clear` ----------

    #[test]
    fn float_left_is_removed_from_normal_flow_and_positioned_at_the_containing_blocks_edge() {
        // No inter-tag whitespace — same reason every other exact-pixel
        // flow test in this file avoids it (a whitespace-only text node
        // would otherwise become its own `Blocks` child).
        let laid = measure_fixture(
            r#"<html><body><div style="width: 300px;"><div style="float: left; width: 50px; height: 80px;"></div><div style="height: 20px;">normal</div></div></body></html>"#,
            400.0,
        );
        let container = find_by_width(&laid, 300.0).expect("container not found");
        let LaidContent::Blocks(children) = &container.content else { panic!("expected Blocks content") };
        assert_eq!(
            children.len(),
            2,
            "a float still produces a real Laid box in its parent's children (unlike `position: absolute`, which removes its slot entirely) — it just doesn't advance the flow cursor"
        );
        let float_box = children.iter().find(|c| c.h == 80.0).expect("float box not found");
        let normal = children.iter().find(|c| c.h == 20.0).expect("normal sibling not found");
        assert_eq!(
            (float_box.x, float_box.y),
            (container.x, container.y),
            "an unobstructed `float: left` should sit at the containing block's own content-box left/top edge"
        );
        assert_eq!(float_box.w, 50.0, "an explicit float width should be honored exactly, same as any other box");
        assert_eq!(
            normal.y, container.y,
            "the sibling after a float should land exactly where it would if the float didn't exist at all — a float doesn't advance the flow cursor"
        );
        assert_eq!(
            normal.x, container.x,
            "an ordinary (non-floated) block's own border-box position is unaffected by a float — only its *inline* content narrows around one, not its own box geometry"
        );
    }

    #[test]
    fn float_right_is_positioned_against_the_containing_blocks_right_edge() {
        let laid = measure_fixture(
            r#"<html><body><div style="width: 300px;"><div style="float: right; width: 50px; height: 30px;"></div></div></body></html>"#,
            400.0,
        );
        let container = find_by_width(&laid, 300.0).expect("container not found");
        let LaidContent::Blocks(children) = &container.content else { panic!("expected Blocks content") };
        let float_box = children.iter().find(|c| c.h == 30.0).expect("float box not found");
        assert_eq!(
            float_box.x,
            container.x + 300.0 - 50.0,
            "an unobstructed `float: right` should sit flush with the containing block's own right edge"
        );
        assert_eq!(float_box.y, container.y);
    }

    #[test]
    fn clear_moves_a_box_below_an_earlier_float() {
        let laid = measure_fixture(
            r#"<html><body><div style="width: 300px;"><div style="float: left; width: 50px; height: 80px;"></div><div style="clear: left; height: 20px;">cleared</div></div></body></html>"#,
            400.0,
        );
        let container = find_by_width(&laid, 300.0).expect("container not found");
        let LaidContent::Blocks(children) = &container.content else { panic!("expected Blocks content") };
        assert_eq!(children.len(), 2);
        let float_box = children.iter().find(|c| c.h == 80.0).expect("float box not found");
        let cleared = children.iter().find(|c| c.h == 20.0).expect("cleared sibling not found");
        assert_eq!(
            cleared.y,
            float_box.y + float_box.h,
            "clear: left should push this box down to exactly the bottom of the earlier left float, instead of landing beside/under it like the uncleared sibling in the test above"
        );
    }

    #[test]
    fn float_paints_above_a_later_full_width_siblings_own_background() {
        // A later, non-cleared, full-width sibling's own box is never
        // narrowed to avoid an earlier float (only its *inline* content
        // is — see the geometry tests above), so its own background can
        // legitimately overlap the float's area. Real CSS still paints
        // the float on top in that overlap (CSS2.1 Appendix E: floats
        // are their own stacking level, above ordinary in-flow,
        // non-positioned block-level boxes) — found as a real, visible
        // bug via a `grim` screenshot before this test existed: without
        // special-casing paint order, the float (collected into its
        // parent's `Blocks` children at its own point in document order,
        // same as any other child) painted *underneath* the later green
        // sibling's own opaque background instead.
        let html = r#"<html><body><div style="width: 400px;"><div style="float: left; width: 150px; height: 150px; background-color: #ff0000;"></div><div style="background-color: #00ff00; height: 100px;"></div></div></body></html>"#;
        let doc = parse_html(html);
        let tree = build_layout_tree(&doc);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid =
            measure_document(&tree, &mut font_system, 400.0, 400.0, &mut next_id, &mut cache, deadline, &mut incomplete);
        let mut swash_cache = SwashCache::new();
        let (win_w, win_h) = (400u32, 400u32);
        let mut pixels = vec![0u32; (win_w * win_h) as usize];
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &mut pixels, win_w, win_h, 0.0, None);
        // `body { margin: 8px }` (default.css) offsets everything by
        // 8px — (50, 50) is well inside both the float's 150x150 area
        // and the green sibling's full-width/100px-tall area, which
        // start at the same (8, 8) origin since the float never
        // advances the flow cursor.
        let px = pixels[50 * win_w as usize + 50];
        assert_eq!(px & 0x00FF_FFFF, 0x00FF_0000, "the float (red) should paint on top of the later sibling's own background (green) in their overlap region");
    }

    #[test]
    fn inline_text_narrows_beside_a_left_float_and_resumes_full_width_below_it() {
        // Many repeated short words in a container only 40px wider than
        // the float, so wrapping to at least two lines is guaranteed
        // regardless of real font metrics on whatever system runs this
        // test (same "don't depend on exact glyph widths" discipline
        // `mixed_inline_wraps_when_content_does_not_fit_on_one_line`
        // already established with images) — the float itself is
        // exactly one default line-height (20px) tall, so only the
        // line(s) starting at `y == 0` overlap it; anything starting at
        // `y >= 20` is entirely below it and should see the container's
        // full, unnarrowed width again. `margin: 0` on the `<p>` so its
        // own inline content starts exactly at the container's content
        // box, not shifted down by `default.css`'s own paragraph margin.
        let word_text = "hi ".repeat(40);
        let html = format!(
            r#"<html><body><div style="width: 120px;"><div style="float: left; width: 80px; height: 20px;"></div><p style="margin: 0">{word_text}</p></div></body></html>"#
        );
        let laid = measure_fixture(&html, 400.0);
        let container = find_by_width(&laid, 120.0).expect("container not found");
        let run = find_mixed_inline(&laid)
            .expect("a plain-text paragraph sharing a BFC with an earlier float should take the floats-aware per-word path, not the plain whole-buffer one");
        let positions: Vec<(f32, f32)> = mixed_items(run).iter().map(mixed_item_xy).collect();
        let float_right_edge = container.x + 80.0;
        let float_bottom = container.y + 20.0;

        let first_line_y = positions.iter().map(|&(_, y)| y).fold(f32::MAX, f32::min);
        let (first_x, _) = *positions.iter().find(|&&(_, y)| y == first_line_y).unwrap();
        assert!(
            first_x >= float_right_edge - 0.01,
            "a line overlapping the float's own y range should start at/after its right edge ({float_right_edge}), not the container's own left edge ({}); got x={first_x}",
            container.x
        );

        let below_float = positions.iter().find(|&&(_, y)| y >= float_bottom - 0.01);
        let (x_below, _) = *below_float
            .expect("with 40 words wrapping through a 40px-narrow corridor, at least one line should end up entirely below the float");
        assert!(
            (x_below - container.x).abs() < 0.01,
            "a line entirely below the float's bottom should resume the container's own full width, starting back at its own left edge ({}); got x={x_below}",
            container.x
        );
    }

    // ---------- v55: `position: sticky` ----------

    /// Searches a laid-out `Laid` tree (not the pre-measure `LayoutBox`
    /// tree `find_box_by_position` searches) for a box whose own
    /// `position` is `pos` — used by this item's own tests, which all
    /// need the real, measured `Laid` (its `.sticky` field, its exact
    /// `x`/`y`/`h`), not just whether a box with this position exists.
    fn find_laid_by_position(laid: &Laid, pos: Position) -> Option<&Laid> {
        if laid.style.position == pos {
            return Some(laid);
        }
        if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
            for c in children {
                if let Some(found) = find_laid_by_position(c, pos) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn sticky_box_stays_in_normal_flow_and_contributes_to_its_containers_size() {
        // No inter-tag whitespace — same reason every other exact-pixel
        // flow test in this file avoids it (a whitespace-only text node
        // would otherwise become its own `Blocks` child).
        let laid = measure_fixture(
            r#"<html><body><div style="width: 100px;"><div style="height: 20px;"></div><div style="position: sticky; top: 50px; height: 21px;"></div><div style="height: 22px;"></div></div></body></html>"#,
            400.0,
        );
        let container = find_by_width(&laid, 100.0).expect("container not found");
        let LaidContent::Blocks(children) = &container.content else { panic!("expected Blocks content") };
        assert_eq!(
            children.len(),
            3,
            "a sticky box still occupies a real slot in its parent's normal flow, unlike `absolute`/`fixed` (which drop their slot entirely)"
        );
        let (first, sticky_box, third) = (&children[0], &children[1], &children[2]);
        assert_eq!(sticky_box.style.position, Position::Sticky);
        assert_eq!(
            sticky_box.y,
            first.y + first.h,
            "a sticky box's own layout-time position is exactly its ordinary static position — unlike `relative`, its `top`/`bottom` offset is never baked into `Laid::y` at all, only applied at paint time"
        );
        assert_eq!(
            third.y,
            sticky_box.y + sticky_box.h,
            "the sibling after a sticky box must land exactly where it would if the sticky box had no offset at all — `position: sticky` never affects normal flow, same as `relative`"
        );
        assert_eq!(
            container.h,
            first.h + sticky_box.h + third.h,
            "the sticky box's own height is fully counted toward its container's auto height, unlike an out-of-flow `absolute`/`fixed` box"
        );

        let info = sticky_box.sticky.expect("position: sticky should capture a StickyInfo at layout time");
        assert_eq!(info.static_top, sticky_box.y);
        assert_eq!(info.top, Some(50.0), "top: 50px should resolve against the containing block's own height basis, same timing `relative`'s own offsets use");
        assert_eq!(info.bottom, None);
    }

    /// Shared fixture for the scroll-dependent tests below: a 100px-wide
    /// column with a 200px spacer, the sticky box itself (30px tall,
    /// `top: 10px`, painted red so a pixel-level check can confirm it),
    /// and a 1000px spacer after it — enough room for the box to clear
    /// its sticking threshold long before its containing block's own far
    /// edge would catch up to it, so each test below can pick a scroll
    /// offset that isolates exactly one of the three states (`static`,
    /// `stuck`, `stopped sticking`) without the others interfering.
    fn sticky_scroll_fixture() -> Laid {
        let html = r#"<html><body><div style="width: 100px;"><div style="height: 200px;"></div><div style="position: sticky; top: 10px; height: 30px; background-color: #ff0000;"></div><div style="height: 1000px;"></div></div></body></html>"#;
        let doc = parse_html(html);
        let tree = build_layout_tree(&doc);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        measure(
            &tree, &mut font_system, 0.0, 0.0, 400.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
            &mut Vec::new(), &Floats::default(), None,
        )
    }

    #[test]
    fn sticky_box_paints_at_its_static_position_before_scroll_reaches_the_threshold() {
        let laid = sticky_scroll_fixture();
        let sticky_box = find_laid_by_position(&laid, Position::Sticky).expect("sticky box not found");
        let (static_top, box_h) = (sticky_box.y, sticky_box.h);
        let info = sticky_box.sticky.expect("sticky info not captured");

        assert_eq!(
            sticky_shift(&info, box_h, 0.0, 600.0),
            0.0,
            "with no scroll at all, the box hasn't reached its sticking threshold yet, so it should paint with zero offset from its own static position"
        );

        let mut font_system = new_font_system();
        let mut swash_cache = SwashCache::new();
        let mut cache = ShapeCache::new();
        let (win_w, win_h) = (400u32, (static_top + box_h + 50.0) as u32);
        let mut pixels = vec![0u32; (win_w * win_h) as usize];
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &mut pixels, win_w, win_h, 0.0, None);
        let probe_y = (static_top + box_h / 2.0) as usize;
        let px = pixels[probe_y * win_w as usize + 10];
        assert_eq!(
            px & 0x00FF_FFFF,
            0x00FF_0000,
            "with no scroll yet, the sticky box should paint at its own static document-space position, exactly like an ordinary (or `relative`) box would"
        );
    }

    #[test]
    fn sticky_box_clamps_to_its_top_offset_once_scroll_passes_the_threshold() {
        let laid = sticky_scroll_fixture();
        let sticky_box = find_laid_by_position(&laid, Position::Sticky).expect("sticky box not found");
        let (static_top, box_h) = (sticky_box.y, sticky_box.h);
        let info = sticky_box.sticky.expect("sticky info not captured");

        // Scrolled well past the point `top: 10px` should have kicked
        // in, but nowhere near the container's own far edge (1000px of
        // slack below) — squarely in the "stuck" range.
        let scroll_offset = static_top + 200.0;
        let dy = sticky_shift(&info, box_h, scroll_offset, 600.0);
        assert_eq!(
            dy,
            scroll_offset + 10.0 - static_top,
            "once stuck, the box should track the live scroll offset exactly: its document-space position is always `scroll_offset + top`"
        );

        let mut font_system = new_font_system();
        let mut swash_cache = SwashCache::new();
        let mut cache = ShapeCache::new();
        let (win_w, win_h) = (400u32, 100u32);
        let mut pixels = vec![0u32; (win_w * win_h) as usize];
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &mut pixels, win_w, win_h, scroll_offset, None);
        // Screen-space row = document y (`scroll_offset + 10`) minus
        // `scroll_offset` = exactly `10` — the box should now sit flush
        // `top`'s own 10px from the window's own top edge, nowhere near
        // the (off-screen, were it not stuck) row its static position
        // alone would have put it at.
        let probe_y = (10.0 + box_h / 2.0) as usize;
        let px = pixels[probe_y * win_w as usize + 10];
        assert_eq!(
            px & 0x00FF_FFFF,
            0x00FF_0000,
            "once stuck, the box should paint `top`'s own 10px from the window's top edge, not wherever its static position would otherwise have scrolled to"
        );
    }

    #[test]
    fn sticky_box_stops_sticking_once_its_containing_blocks_far_edge_would_scroll_past() {
        let laid = sticky_scroll_fixture();
        let sticky_box = find_laid_by_position(&laid, Position::Sticky).expect("sticky box not found");
        let (static_top, box_h) = (sticky_box.y, sticky_box.h);
        let info = sticky_box.sticky.expect("sticky info not captured");
        let max_top = info.cb_bottom - box_h;
        assert!(
            max_top > static_top,
            "the fixture's own 1000px trailing spacer should leave real slack between the box's static position and its containing block's far edge"
        );

        // Exactly at the boundary (`scroll_offset + top == max_top`) and
        // far past it — both should land the box at the *same* final
        // offset, never escaping further past its own containing
        // block's bottom edge no matter how much further the page
        // scrolls.
        let dy_at_boundary = sticky_shift(&info, box_h, max_top - 10.0, 600.0);
        let dy_way_past = sticky_shift(&info, box_h, max_top + 5000.0, 600.0);
        let expected = max_top - static_top;
        assert_eq!(dy_at_boundary, expected, "right at the containing block's far edge, the box should already be clamped there");
        assert_eq!(
            dy_way_past, expected,
            "scrolling far past that point must not push the box any further — it stops sticking and scrolls away with the rest of its container instead of escaping past its own parent"
        );
    }

    #[test]
    fn scrolling_only_changes_the_painted_position_never_the_laid_tree() {
        // `paint` takes `&Laid`, never `&mut Laid` — so calling it twice
        // against the very same tree with two very different scroll
        // offsets already *cannot* mutate layout, a compile-time
        // guarantee, not just a runtime one (this is the whole
        // architectural point of resolving `position: sticky` the way
        // this file does — see NOTES.md's v4/v5 writeup on why
        // scrolling is deliberately cheap). This test pins that down
        // concretely: snapshot the sticky box's own layout-time fields,
        // paint twice at very different scroll offsets, and confirm (a)
        // those fields are still exactly what they were (no relayout
        // happened — the same `Laid` tree, not a rebuilt one) while (b)
        // the two painted results genuinely differ (the box visibly
        // moved) — scroll changes the picture, never the layout.
        let laid = sticky_scroll_fixture();
        let sticky_box = find_laid_by_position(&laid, Position::Sticky).expect("sticky box not found");
        let (static_top, static_x, box_h, box_w) = (sticky_box.y, sticky_box.x, sticky_box.h, sticky_box.w);
        let info_before = sticky_box.sticky.expect("sticky info not captured");

        let mut font_system = new_font_system();
        let mut swash_cache = SwashCache::new();
        let mut cache = ShapeCache::new();
        let (win_w, win_h) = (400u32, 300u32);
        let mut pixels_a = vec![0u32; (win_w * win_h) as usize];
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &mut pixels_a, win_w, win_h, 0.0, None);
        let mut pixels_b = vec![0u32; (win_w * win_h) as usize];
        paint(
            &laid, &mut font_system, &mut swash_cache, &mut cache, &mut pixels_b, win_w, win_h, static_top + 200.0,
            None,
        );

        let sticky_box_after = find_laid_by_position(&laid, Position::Sticky).expect("sticky box not found after painting");
        assert_eq!(
            (sticky_box_after.x, sticky_box_after.y, sticky_box_after.w, sticky_box_after.h),
            (static_x, static_top, box_w, box_h),
            "the sticky box's own layout-time geometry must be byte-identical after painting at two different scroll offsets"
        );
        let info_after = sticky_box_after.sticky.expect("sticky info missing after painting");
        assert_eq!(
            (info_after.static_top, info_after.top, info_after.bottom, info_after.cb_top, info_after.cb_bottom),
            (info_before.static_top, info_before.top, info_before.bottom, info_before.cb_top, info_before.cb_bottom),
            "StickyInfo itself must also be untouched by painting — nothing about scroll ever re-resolves it"
        );

        assert_ne!(
            pixels_a, pixels_b,
            "the two different scroll offsets should visibly move the sticky box on screen, even though the underlying Laid tree never changed at all"
        );
    }

    // ---------- v56: real CSS2.1 Appendix E stacking-context semantics ----------

    /// Shared plumbing for this section's tests: parse, build, measure at
    /// a 200x200 viewport (`measure_document`, so `position: absolute`/
    /// `fixed` resolve the same way they would in the real app), paint
    /// into a real pixel buffer, and hand back just the buffer plus the
    /// window size needed to index it. `<body style="margin:0">` in every
    /// fixture below sidesteps `default.css`'s own `body { margin: 8px }`
    /// — these tests sample hand-computed pixel coordinates against
    /// in-flow boxes (unlike the pre-existing `z_index_controls_paint_
    /// order_for_overlapping_absolute_boxes` test above, whose `position:
    /// absolute; top/left: 0` boxes anchor to the *viewport*, immune to
    /// body's own margin either way).
    fn paint_fixture(html: &str) -> (Vec<u32>, u32, u32) {
        let doc = parse_html(html);
        let tree = build_layout_tree(&doc);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid =
            measure_document(&tree, &mut font_system, 200.0, 200.0, &mut next_id, &mut cache, deadline, &mut incomplete);
        let mut swash_cache = SwashCache::new();
        let (win_w, win_h) = (200u32, 200u32);
        let mut pixels = vec![0u32; (win_w * win_h) as usize];
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &mut pixels, win_w, win_h, 0.0, None);
        (pixels, win_w, win_h)
    }

    fn px_at(pixels: &[u32], win_w: u32, x: u32, y: u32) -> u32 {
        pixels[(y * win_w + x) as usize] & 0x00FF_FFFF
    }

    #[test]
    fn negative_z_index_paints_above_its_containing_boxs_background_but_below_normal_content() {
        // CSS2.1 Appendix E's own bottommost-first order
        // (https://www.w3.org/TR/CSS21/zindex.html) is: 1 (this box's own
        // background/border), 2 (negative-`z-index` descendant stacking
        // contexts), 3-5 (normal in-flow content) — i.e. negative
        // `z-index` paints *above* this box's own background (it is only
        // ever hidden behind *normal content*, layers 3+, never behind
        // the background fill of the box that establishes its stacking
        // context). Confirmed against the spec directly — the intuitive-
        // sounding "negative z-index hides behind its own box's
        // background" is a common but *inaccurate* paraphrase of the
        // real rule (it only looks that way when the relevant box has no
        // background of its own, so there's nothing *else* for the
        // negative descendant to appear in front of). Two assertions:
        // red (the `z-index: -1` child) must beat green (the container's
        // own background) where only those two overlap, and lose to blue
        // (an ordinary static sibling, layer 3) wherever blue also
        // covers the same pixel.
        //
        // No inter-tag whitespace — a literal newline/indentation between
        // these block-level tags would become its own whitespace-only
        // anonymous inline box, consuming real flow height and silently
        // shifting every box below it (the exact v51/v52-documented
        // pitfall; caught here by a debug dump during development, not
        // assumed away).
        let html = r#"<html><body style="margin:0"><div style="position: relative; width: 100px; height: 100px; background-color: #00ff00;"><div style="position: absolute; top: 0px; left: 0px; width: 100px; height: 100px; background-color: #ff0000; z-index: -1;"></div><div style="width: 100px; height: 50px; background-color: #0000ff;"></div></div></body></html>"#;
        let (pixels, win_w, _) = paint_fixture(html);
        assert_eq!(
            px_at(&pixels, win_w, 50, 80),
            0x00FF_0000,
            "the z-index:-1 child should paint above the container's own green background"
        );
        assert_eq!(
            px_at(&pixels, win_w, 50, 20),
            0x0000_00FF,
            "the z-index:-1 child should still paint below an ordinary (layer 3) static sibling"
        );
    }

    #[test]
    fn seven_layer_stacking_order_within_one_containing_block() {
        // One `position: relative` container with three children spanning
        // four of CSS2.1 Appendix E's seven layers: its own background
        // (layer 1, green, full 100x100), a `z-index: -1` absolute child
        // (layer 2, red, full 100x100 — above the background, below
        // everything else), an ordinary static in-flow child (layer 3,
        // blue, top 100x50 half — above the negative-z-index child, same
        // as any other non-positioned in-flow content), and a `z-index: 5`
        // absolute child (layer 7, yellow, top-left 30x30 — above
        // everything, including the static child). Document order is
        // deliberately *not* the paint order here (red is declared first,
        // yellow last, but neither is topmost solely from parsing order) —
        // z-index/layer membership alone must decide it.
        // No inter-tag whitespace — see the test above's own note.
        let html = r#"<html><body style="margin:0"><div style="position: relative; width: 100px; height: 100px; background-color: #00ff00;"><div style="position: absolute; top: 0px; left: 0px; width: 100px; height: 100px; background-color: #ff0000; z-index: -1;"></div><div style="width: 100px; height: 50px; background-color: #0000ff;"></div><div style="position: absolute; top: 0px; left: 0px; width: 30px; height: 30px; background-color: #ffff00; z-index: 5;"></div></div></body></html>"#;
        let (pixels, win_w, _) = paint_fixture(html);
        // Outside the static child's 0-50 range and the small yellow
        // child's 0-30 range: only green (layer 1) and red (layer 2)
        // cover this pixel — red must win (layer 2 above layer 1).
        assert_eq!(px_at(&pixels, win_w, 80, 80), 0x00FF_0000, "layer 2 (negative z-index) should paint above layer 1 (this box's own background)");
        // Inside both the static child's range and the small yellow
        // child's range: yellow (layer 7) must win over everything,
        // including the in-flow static child (layer 3).
        assert_eq!(px_at(&pixels, win_w, 10, 10), 0x00FF_FF00, "layer 7 (positive z-index) should paint above layer 3 (ordinary in-flow content)");
        // Inside the static child's range but outside the small yellow
        // child's range: blue (layer 3) must win over red (layer 2).
        assert_eq!(px_at(&pixels, win_w, 10, 40), 0x0000_00FF, "layer 3 (ordinary in-flow content) should paint above layer 2 (negative z-index)");
    }

    #[test]
    fn relative_box_with_explicit_z_index_establishes_its_own_stacking_context() {
        // A `position: relative; z-index: 1` box stays in normal flow
        // (contributes to its container's size, never displaces a
        // sibling — unchanged from `relative`'s own pre-existing
        // behavior) but, with an *explicit* z-index, now establishes its
        // own stacking context: real CSS2 §9.9.1. Shifted down by `top:
        // 30px` to genuinely overlap a later, ordinary static sibling
        // (declared *after* it, and which would win a plain document-
        // order paint) — the explicit z-index must still win.
        // No inter-tag whitespace — see the first test above's own note.
        let html = r#"<html><body style="margin:0"><div style="position: relative; top: 30px; left: 0px; width: 50px; height: 50px; background-color: #ff0000; z-index: 1;"></div><div style="width: 50px; height: 50px; background-color: #0000ff;"></div></body></html>"#;
        let (pixels, win_w, _) = paint_fixture(html);
        // Overlap region: the shifted red box covers screen y 30-80; the
        // ordinary blue sibling (document order: right after red's own
        // *unshifted* 0-50 flow slot) covers y 50-100. Sampled at y=60,
        // inside both.
        assert_eq!(
            px_at(&pixels, win_w, 10, 60),
            0x00FF_0000,
            "the relative box's own explicit z-index should establish a stacking context above its later static sibling"
        );
    }

    #[test]
    fn relative_box_with_auto_z_index_does_not_establish_a_stacking_context() {
        // Exact same fixture as the test above, minus the explicit
        // `z-index` on the shifted relative box — real CSS: `z-index:
        // auto` does *not* establish a new stacking context, so this
        // box keeps today's plain paint-position-only treatment and the
        // later static sibling (document order) still wins the overlap,
        // same as before this session's feature existed at all. This is
        // the regression pin for that "same as before" claim.
        // No inter-tag whitespace — see the first test in this section.
        let html = r#"<html><body style="margin:0"><div style="position: relative; top: 30px; left: 0px; width: 50px; height: 50px; background-color: #ff0000;"></div><div style="width: 50px; height: 50px; background-color: #0000ff;"></div></body></html>"#;
        let (pixels, win_w, _) = paint_fixture(html);
        assert_eq!(
            px_at(&pixels, win_w, 10, 60),
            0x0000_00FF,
            "z-index:auto should not establish a stacking context — the later sibling keeps winning the overlap by plain document/paint order"
        );
    }

    // ---------- v57: flexbox (`display: flex`) ----------

    /// The flex container's own direct `LaidContent::Blocks` children,
    /// in document order — exactly the flex items `measure_flex`
    /// positioned, found by locating the container itself (via its own
    /// distinctive `width`, same `find_by_width` helper every other
    /// section already uses) rather than a bespoke lookup.
    fn flex_items(laid: &Laid, container_w: f32) -> Vec<&Laid> {
        let container = find_by_width(laid, container_w).expect("flex container not found");
        let LaidContent::Blocks(children) = &container.content else {
            panic!("expected the flex container's content to be Blocks (its own flex items)");
        };
        children.iter().collect()
    }

    #[test]
    fn row_direction_lays_items_out_left_to_right() {
        let laid = measure_fixture(
            r#"<html><body><div style="display: flex; width: 300px;"><div style="width: 50px;">a</div><div style="width: 80px;">b</div></div></body></html>"#,
            500.0,
        );
        let items = flex_items(&laid, 300.0);
        assert_eq!(items.len(), 2, "both children should become flex items");
        assert!(items[0].x < items[1].x, "row direction: items should lay out left to right");
        assert_eq!(items[1].x, items[0].x + items[0].w, "second item should start exactly where the first ends (no gap/grow)");
        assert_eq!(items[0].y, items[1].y, "both items should start at the same cross-axis (y) position");
    }

    #[test]
    fn column_direction_lays_items_out_top_to_bottom() {
        let laid = measure_fixture(
            r#"<html><body><div style="display: flex; flex-direction: column; width: 300px;"><div style="height: 40px;">a</div><div style="height: 60px;">b</div></div></body></html>"#,
            500.0,
        );
        let items = flex_items(&laid, 300.0);
        assert_eq!(items.len(), 2);
        assert!(items[0].y < items[1].y, "column direction: items should lay out top to bottom");
        assert_eq!(items[1].y, items[0].y + items[0].h, "second item should start exactly where the first ends");
        assert_eq!(items[0].x, items[1].x, "both items should share the same cross-axis (x) position");
    }

    #[test]
    fn justify_content_space_between_pushes_items_to_opposite_edges() {
        let laid = measure_fixture(
            r#"<html><body style="margin: 0"><div style="display: flex; justify-content: space-between; width: 300px;"><div style="width: 50px;">a</div><div style="width: 50px;">b</div></div></body></html>"#,
            500.0,
        );
        let items = flex_items(&laid, 300.0);
        assert_eq!(items[0].x, 0.0, "first item should sit at the container's own left edge");
        assert_eq!(items[1].x, 250.0, "second item's right edge (250+50) should sit exactly at the container's right edge (300)");
    }

    #[test]
    fn align_items_stretch_grows_the_shorter_sibling_to_match_the_tallest() {
        let laid = measure_fixture(
            r#"<html><body><div style="display: flex; width: 300px;"><div style="width: 50px; height: 100px;">a</div><div style="width: 50px;">b</div></div></body></html>"#,
            500.0,
        );
        let items = flex_items(&laid, 300.0);
        assert_eq!(items[0].h, 100.0, "the explicitly-tall item keeps its own height");
        assert_eq!(items[1].h, 100.0, "align-items: stretch (the default) should grow the shorter, auto-height sibling to match the line's cross size");
    }

    #[test]
    fn align_items_center_positions_without_resizing() {
        let laid = measure_fixture(
            r#"<html><body><div style="display: flex; align-items: center; width: 300px;"><div style="width: 50px; height: 100px;">a</div><div style="width: 50px; height: 20px;">b</div></div></body></html>"#,
            500.0,
        );
        let items = flex_items(&laid, 300.0);
        assert_eq!(items[1].h, 20.0, "align-items: center should not stretch the shorter item's own height");
        // Compared *relative* to the taller sibling (which, at the
        // line's own full 100px cross size, centers to no offset at
        // all) rather than against an absolute `y`, so this doesn't
        // depend on `body`'s own default margin shifting everything's
        // absolute document-space position uniformly.
        assert_eq!(
            items[1].y - items[0].y,
            (100.0 - 20.0) / 2.0,
            "the shorter item should be vertically centered within the line's cross size (100px)"
        );
    }

    #[test]
    fn flex_grow_distributes_free_space_proportionally() {
        let laid = measure_fixture(
            r#"<html><body><div style="display: flex; width: 300px;"><div style="flex: 1 0 0;">a</div><div style="flex: 2 0 0;">b</div></div></body></html>"#,
            500.0,
        );
        let items = flex_items(&laid, 300.0);
        assert_eq!(items[0].w, 100.0, "flex-grow 1 of (1+2) should claim 1/3 of the 300px free space");
        assert_eq!(items[1].w, 200.0, "flex-grow 2 of (1+2) should claim 2/3 of the 300px free space — not just 'fit content'");
    }

    #[test]
    fn flex_shrink_distributes_deficit_proportionally_to_scaled_factor() {
        // Uses `flex-basis`, not `width`, for each item's own 100px basis
        // — an item's own explicit CSS `width` wins over a flex-shrunk
        // size in this engine (a documented, accepted simplification,
        // same category `measure_table`'s own column-width-vs-cell-width
        // gap already has — see `measure_flex_row`'s own docs), so using
        // `width` here would make every item keep its own literal width
        // regardless of `flex-shrink`, defeating the point of this test.
        let laid = measure_fixture(
            r#"<html><body><div style="display: flex; width: 100px;"><div style="flex-basis: 100px; flex-shrink: 1;">a</div><div style="flex-basis: 100px; flex-shrink: 3;">b</div></div></body></html>"#,
            500.0,
        );
        let items = flex_items(&laid, 100.0);
        // Total natural width 200px into a 100px container: a 100px
        // deficit, split by *scaled* shrink factor (`flex_shrink * basis`,
        // CSS Flexbox §9.7) — both items share the same 100px basis, so
        // the split is exactly proportional to their shrink factors alone
        // (1:3): item a loses 25px (100 * 1/4), item b loses 75px (100 * 3/4).
        assert_eq!(items[0].w, 75.0, "flex-shrink 1 (of a 1:3 scaled ratio) should lose proportionally less width");
        assert_eq!(items[1].w, 25.0, "flex-shrink 3 (of a 1:3 scaled ratio) should lose proportionally more width — not shrink equally");
    }

    #[test]
    fn flex_wrap_starts_a_second_line_once_items_no_longer_fit() {
        let laid = measure_fixture(
            r#"<html><body><div style="display: flex; flex-wrap: wrap; width: 100px;"><div style="width: 60px; height: 20px;">a</div><div style="width: 60px; height: 20px;">b</div></div></body></html>"#,
            500.0,
        );
        let items = flex_items(&laid, 100.0);
        assert_eq!(items[0].x, items[1].x, "both items should be left-aligned on their own line (each is alone on its own line)");
        assert!(items[1].y > items[0].y, "the second item (60+60 > 100px container) should wrap onto a new, lower line");
        assert_eq!(items[1].y, items[0].y + items[0].h, "the second line should start exactly where the first line's cross size ends (no row-gap declared)");
    }

    #[test]
    fn gap_adds_fixed_spacing_between_items_on_top_of_justify_content() {
        let laid = measure_fixture(
            r#"<html><body><div style="display: flex; gap: 20px; width: 300px;"><div style="width: 50px;">a</div><div style="width: 50px;">b</div></div></body></html>"#,
            500.0,
        );
        let items = flex_items(&laid, 300.0);
        assert_eq!(items[1].x, items[0].x + items[0].w + 20.0, "column-gap (from the `gap` shorthand) should add exactly 20px between items, beyond their own widths");
    }

    #[test]
    fn position_absolute_child_of_a_flex_container_is_not_treated_as_a_flex_item() {
        let laid = measure_fixture(
            // `position: relative` on the container itself so it's a
            // real "positioned ancestor" and actually catches its own
            // `absolute` child locally (`resolve_or_bubble_positioned`)
            // — otherwise (a plain `static` flex container, like every
            // other test in this section) it would correctly keep
            // bubbling past it, same as any other `Blocks` box; this
            // test is about the flex *item-list* split, not about
            // `position`'s own containing-block search.
            r#"<html><body style="margin: 0"><div style="display: flex; position: relative; width: 300px;"><div style="position: absolute; top: 5px; left: 5px; width: 20px; height: 20px;">p</div><div style="width: 50px;">a</div></div></body></html>"#,
            500.0,
        );
        let container = find_by_width(&laid, 300.0).expect("flex container not found");
        let LaidContent::Blocks(children) = &container.content else {
            panic!("expected Blocks content");
        };
        assert_eq!(children.len(), 1, "the absolutely-positioned child should be pulled out of the flex item list entirely");
        assert_eq!(children[0].x, 0.0, "the one real flex item should start at the container's left edge, not pushed aside by the out-of-flow child");
        assert_eq!(container.positioned.len(), 1, "the absolutely-positioned child should instead land in the container's own `positioned` list");
    }
}
