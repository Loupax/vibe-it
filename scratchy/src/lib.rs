use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::{mpsc, Arc, Mutex};
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
use signal_hook::consts::SIGUSR1;
use signal_hook::iterator::Signals;
use softbuffer::{Context, Surface};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

// ---------- CLI input ----------

enum Input {
    File(String),
    Stdin,
}

/// `--socket <path>`, alongside the pre-existing `[file]` argument — an
/// explicit opt-in flag, not an auto-discovered default (see `wire/
/// TODO.txt`'s own "TOP PRIORITY" item for why): scratchy never guesses
/// where a `wire` daemon might be listening, and never starts one
/// itself. With no `--socket` at all, link navigation always falls
/// straight to `start_navigation`'s own print-the-request-to-stdout
/// fallback — see its own docs.
struct Args {
    input: Input,
    socket: Option<PathBuf>,
}

fn parse_args() -> Args {
    let raw: Vec<String> = env::args().skip(1).collect();
    let mut socket = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == "--socket" {
            let Some(path) = raw.get(i + 1) else {
                eprintln!("scratchy: --socket needs a path argument");
                std::process::exit(1);
            };
            socket = Some(PathBuf::from(path));
            i += 2;
        } else {
            rest.push(raw[i].clone());
            i += 1;
        }
    }
    let input = match rest.len() {
        0 => Input::Stdin,
        1 => Input::File(rest[0].clone()),
        _ => {
            eprintln!("usage: scratchy [--socket <path>] [file]");
            std::process::exit(1);
        }
    };
    Args { input, socket }
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

/// Leaks an owned copy of `s` into a `&'static str` — the same tradeoff
/// `font-family`'s own named fallback lists already made (see
/// `FontFamily`'s own docs/`TODO.txt`): bounded by how many distinct
/// values are walked per layout rebuild (here, how many distinct
/// `a`/`area href`s a document has), not by click/redraw count, kept
/// only so the value can live inside a `#[derive(Copy)]` type
/// (`SpanStyle::href`) the same cheap way every other per-span field
/// does.
fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
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
    /// `display: grid` — a block-level container (in the outer flow,
    /// same as `Table`/`Flex`) whose direct children each become grid
    /// items, placed into a 2D grid (`measure_grid`) per CSS Grid
    /// Layout (https://www.w3.org/TR/css-grid-1/), scoped to a real but
    /// deliberately narrower subset than flexbox's own — explicit
    /// `grid-template-columns`/`-rows` track lists (fixed lengths, `%`,
    /// `fr`, `repeat()`) plus implicit row-by-row auto-placement, no
    /// explicit per-item placement — see `measure_grid`'s own docs and
    /// `TODO.txt` for exactly what's cut. `inline-grid` is not
    /// implemented either, same as `inline-flex`.
    Grid,
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

/// One grid track's declared size — `grid-template-columns`/`-rows`'
/// own `<track-size>` grammar, scoped to the two forms this item
/// implements (see `parse_grid_track_size`'s own docs for what's cut:
/// no `minmax()`, no content-based keywords). `Len`, not yet resolved
/// to a pixel/`BoxLength` value — unlike every other length-ish
/// property in this file, a track's real basis (`resolve_grid_tracks`'s
/// own `available` parameter: the grid container's own content *width*
/// for a column track, its content *height* — not always definite, see
/// `measure_grid`'s own docs — for a row track) isn't known at cascade
/// time, so resolution happens in `measure_grid` instead, mirroring
/// `FlexBasis`'s/`LineHeight`'s own "stay symbolic until a real basis
/// exists" pattern rather than `BoxLength`'s (which only ever needs
/// *one* of two possible bases per property, always the same one for
/// that property — a track's basis genuinely differs by axis, which
/// `BoxLength` has no slot for). `Fr` is the raw flex factor (`2.0` for
/// `2fr`) — grid's own flexible-length primitive, directly analogous to
/// flexbox's `flex-grow`.
#[derive(Clone, Copy)]
enum GridTrackSpec {
    Len(Length),
    Fr(f32),
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
    /// `text-decoration-line`/`-style`/`-color`, declared independently of
    /// the `text-decoration` shorthand (v60) — see `DecorationLine`'s own
    /// docs and `resolve_own_decoration` for how these three combine with
    /// each other and with whatever the shorthand (or an ambient
    /// propagated value) already set, at used-value time.
    text_decoration_line: Option<DecorationLine>,
    text_decoration_style: Option<DecorationStyle>,
    text_decoration_color: Option<CssColor>,
    white_space: Option<WhiteSpace>,
    list_style_type: Option<ListStyleType>,
    text_align: Option<TextAlign>,
    vertical_align: Option<VerticalAlign>,
    line_height: Option<LineHeight>,
    /// `normal` (parsed as `Length::Px(0.0)`, same "no extra spacing"
    /// representation as an explicit `0`) or a plain `<length>` — no
    /// `<percentage>` (same grammar `parse_length`, not `parse_length_
    /// percent`, already gives `border-width`/`outline-width`; real CSS
    /// doesn't allow one here either, at least not before Level 3, which
    /// nothing here targets). See `ComputedStyle::word_spacing`'s own
    /// docs for where this is actually applied — deliberately *not* fed
    /// into cosmic-text's own `Attrs`, unlike what `letter-spacing`
    /// (still reverted, see TODO.txt) tried.
    word_spacing: Option<Length>,
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
    /// Leaked (`Box::leak`, once per distinct parsed declaration) into
    /// a `&'static` slice — see `grid-template-columns`'s own
    /// `DeclParser` branch and `GridTrackSpec`'s own docs.
    grid_template_columns: Option<&'static [GridTrackSpec]>,
    grid_template_rows: Option<&'static [GridTrackSpec]>,
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

/// `visible`/`hidden`/`scroll`/`auto`. Not inherited. `Hidden`/`Scroll`/
/// `Auto` all clip a descendant's painting the same way — text (`paint`'s
/// `clip` parameter, read by `paint_buffer`/`paint_span_backgrounds`)
/// *and* box decoration (background/border/outline/shadow, via
/// `fill_rect` itself applying `clip`, so every caller gets it for free)
/// — but never a box's *own* decoration against its *own* `overflow`,
/// only a descendant's (see `paint`'s own docs on `child_clip` vs. the
/// ambient `clip` it passes to `paint_box_decoration`), same as real
/// CSS: an element's `overflow` clips its content, not itself.
///
/// `Scroll`/`Auto` (v62) additionally make this box its own
/// independently-scrollable region — see `ScrollInfo`'s own docs for the
/// layout-time/paint-time split this needs, same shape `position:
/// sticky` (v55) already established for a different property. The real
/// distinction between the two is *only* scrollbar visibility, not
/// clipping/scrollability — both clip and scroll identically whenever
/// content actually overflows (`ScrollInfo::max_scroll() > 0.0`);
/// `Scroll` always paints a scrollbar indicator even when nothing
/// overflows yet, `Auto` only when it does (see `paint_scrollbar`).
#[derive(Clone, Copy, PartialEq, Default)]
enum Overflow {
    #[default]
    Visible,
    Hidden,
    Scroll,
    Auto,
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

/// Originally just `auto`/`pointer` — the only two keywords
/// `default.css` itself needs (`a { cursor: pointer }`). Extended
/// (TODO.txt, autonomous pass) to the rest of the real CSS `cursor`
/// keyword set that winit's own `CursorIcon` can actually represent —
/// every one of them maps cleanly (confirmed against `cursor_icon`
/// 1.2.0's vendored source, the crate winit's own `CursorIcon` re-
/// exports), so nothing here is an invented/approximate mapping; `none`
/// is real CSS too (hide the cursor entirely), which winit supports via
/// `Window::set_cursor_visible` — not a `CursorIcon` at all, so
/// `to_winit()` returns `Option<CursorIcon>` with `None` meaning "hide,
/// don't set an icon" (see its own docs). Real CSS inheritance (`Auto`
/// is the real spec initial value too), unlike `vertical-align` — a
/// `<span>` nested inside an `<a>` should show the pointer cursor too
/// without re-declaring it, which real inheritance gives for free.
/// Threaded per-span (`SpanStyle`/`shape_spans`'s `Vec<CursorStyle>`,
/// same pattern as `vertical_align`/`visibility`), since the common
/// real use (`<a>`) is inline content inside a block, not a block-level
/// box of its own — hit-testing needs to know which *glyph* the mouse
/// is over, not just which block. `hit_test_cursor` itself needed no
/// changes at all for this extension — it's already keyed on "which
/// `CursorStyle` does this element/glyph resolve to," never on
/// `Pointer` specifically.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum CursorStyle {
    #[default]
    Auto,
    Default,
    None,
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    EResize,
    NResize,
    NeResize,
    NwResize,
    SResize,
    SeResize,
    SwResize,
    WResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
}

impl CursorStyle {
    /// Maps to winit's own `CursorIcon`, or `None` for the one real CSS
    /// `cursor` keyword that isn't an icon at all: `none` itself, which
    /// means "hide the OS cursor," not "show some particular icon" —
    /// the caller is expected to call `Window::set_cursor_visible(false)`
    /// instead of `Window::set_cursor` whenever this returns `None`.
    /// Every other keyword maps onto a real `CursorIcon` variant with no
    /// approximation needed (checked against `cursor_icon` 1.2.0's own
    /// enum directly).
    fn to_winit(self) -> Option<CursorIcon> {
        Some(match self {
            CursorStyle::Auto | CursorStyle::Default => CursorIcon::Default,
            CursorStyle::None => return None,
            CursorStyle::ContextMenu => CursorIcon::ContextMenu,
            CursorStyle::Help => CursorIcon::Help,
            CursorStyle::Pointer => CursorIcon::Pointer,
            CursorStyle::Progress => CursorIcon::Progress,
            CursorStyle::Wait => CursorIcon::Wait,
            CursorStyle::Cell => CursorIcon::Cell,
            CursorStyle::Crosshair => CursorIcon::Crosshair,
            CursorStyle::Text => CursorIcon::Text,
            CursorStyle::VerticalText => CursorIcon::VerticalText,
            CursorStyle::Alias => CursorIcon::Alias,
            CursorStyle::Copy => CursorIcon::Copy,
            CursorStyle::Move => CursorIcon::Move,
            CursorStyle::NoDrop => CursorIcon::NoDrop,
            CursorStyle::NotAllowed => CursorIcon::NotAllowed,
            CursorStyle::Grab => CursorIcon::Grab,
            CursorStyle::Grabbing => CursorIcon::Grabbing,
            CursorStyle::EResize => CursorIcon::EResize,
            CursorStyle::NResize => CursorIcon::NResize,
            CursorStyle::NeResize => CursorIcon::NeResize,
            CursorStyle::NwResize => CursorIcon::NwResize,
            CursorStyle::SResize => CursorIcon::SResize,
            CursorStyle::SeResize => CursorIcon::SeResize,
            CursorStyle::SwResize => CursorIcon::SwResize,
            CursorStyle::WResize => CursorIcon::WResize,
            CursorStyle::EwResize => CursorIcon::EwResize,
            CursorStyle::NsResize => CursorIcon::NsResize,
            CursorStyle::NeswResize => CursorIcon::NeswResize,
            CursorStyle::NwseResize => CursorIcon::NwseResize,
            CursorStyle::ColResize => CursorIcon::ColResize,
            CursorStyle::RowResize => CursorIcon::RowResize,
            CursorStyle::AllScroll => CursorIcon::AllScroll,
            CursorStyle::ZoomIn => CursorIcon::ZoomIn,
            CursorStyle::ZoomOut => CursorIcon::ZoomOut,
        })
    }
}

/// Parses a `cursor` keyword into `CursorStyle` — every real CSS
/// `cursor` keyword winit's `CursorIcon` can represent (see
/// `CursorStyle`'s own docs for why none needed to be skipped), same
/// `eq_ignore_ascii_case` ladder shape `decoration_style_from_keyword`
/// already uses for a similar keyword-to-enum property.
fn cursor_style_from_keyword(kw: &str) -> Option<CursorStyle> {
    let kw_match = |target: &str| kw.eq_ignore_ascii_case(target);
    Some(if kw_match("auto") {
        CursorStyle::Auto
    } else if kw_match("default") {
        CursorStyle::Default
    } else if kw_match("none") {
        CursorStyle::None
    } else if kw_match("context-menu") {
        CursorStyle::ContextMenu
    } else if kw_match("help") {
        CursorStyle::Help
    } else if kw_match("pointer") {
        CursorStyle::Pointer
    } else if kw_match("progress") {
        CursorStyle::Progress
    } else if kw_match("wait") {
        CursorStyle::Wait
    } else if kw_match("cell") {
        CursorStyle::Cell
    } else if kw_match("crosshair") {
        CursorStyle::Crosshair
    } else if kw_match("text") {
        CursorStyle::Text
    } else if kw_match("vertical-text") {
        CursorStyle::VerticalText
    } else if kw_match("alias") {
        CursorStyle::Alias
    } else if kw_match("copy") {
        CursorStyle::Copy
    } else if kw_match("move") {
        CursorStyle::Move
    } else if kw_match("no-drop") {
        CursorStyle::NoDrop
    } else if kw_match("not-allowed") {
        CursorStyle::NotAllowed
    } else if kw_match("grab") {
        CursorStyle::Grab
    } else if kw_match("grabbing") {
        CursorStyle::Grabbing
    } else if kw_match("e-resize") {
        CursorStyle::EResize
    } else if kw_match("n-resize") {
        CursorStyle::NResize
    } else if kw_match("ne-resize") {
        CursorStyle::NeResize
    } else if kw_match("nw-resize") {
        CursorStyle::NwResize
    } else if kw_match("s-resize") {
        CursorStyle::SResize
    } else if kw_match("se-resize") {
        CursorStyle::SeResize
    } else if kw_match("sw-resize") {
        CursorStyle::SwResize
    } else if kw_match("w-resize") {
        CursorStyle::WResize
    } else if kw_match("ew-resize") {
        CursorStyle::EwResize
    } else if kw_match("ns-resize") {
        CursorStyle::NsResize
    } else if kw_match("nesw-resize") {
        CursorStyle::NeswResize
    } else if kw_match("nwse-resize") {
        CursorStyle::NwseResize
    } else if kw_match("col-resize") {
        CursorStyle::ColResize
    } else if kw_match("row-resize") {
        CursorStyle::RowResize
    } else if kw_match("all-scroll") {
        CursorStyle::AllScroll
    } else if kw_match("zoom-in") {
        CursorStyle::ZoomIn
    } else if kw_match("zoom-out") {
        CursorStyle::ZoomOut
    } else {
        return None;
    })
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
    /// Zero or more comma-separated `@media` conditions this rule is
    /// wrapped in (comma = OR between entries; `and` is folded into one
    /// entry's own fields, real Media Queries list grammar). An empty
    /// `Vec` means "not inside any `@media` at all" — always applies,
    /// independent of viewport width, exactly as before this existed.
    /// See `media_list_matches`.
    media: Vec<MediaQuery>,
}

/// A real media type keyword this engine can evaluate. `screen` and
/// `all` both match (scratchy only ever renders to a screen); `print`
/// never matches — scratchy has no print/paged-media concept at all,
/// so `@media print { ... }` rules are intentionally always inert, the
/// correct behavior for a screen-only renderer, not a gap to close.
/// Any other/unrecognized media type keyword (`speech`, a future CSS
/// keyword, a typo) is represented by simply marking the owning
/// `MediaQuery::never` instead of adding a third "unknown" variant
/// here — real Media Queries syntax already treats an unrecognized
/// media type as making that one query "not all" (never matches), the
/// same place this engine's own general "unsupported feature → never
/// matches" fallback (see `MediaQuery::never`) already lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaType {
    All,
    Screen,
    Print,
}

/// scratchy has no OS dark/light theme detection at all — there's no
/// real answer to "what does `prefers-color-scheme` resolve to" the
/// way there would be in a real browser reading an actual OS setting.
/// Fixed, documented default: always `light` (`dark` never matches).
/// An arbitrary but harmless choice — light is the conventional default
/// `prefers-color-scheme` itself defines when no preference is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrefersColorScheme {
    Light,
    Dark,
}

/// Same story as `PrefersColorScheme`: scratchy has no OS reduced-
/// motion setting to read, and (per this item's own scope) no
/// animations to reduce in the first place, making this largely moot
/// either way. Fixed, documented default: always `no-preference`
/// (`reduce` never matches).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrefersReducedMotion {
    NoPreference,
    Reduce,
}

/// One item of a comma-separated `@media` query list — a media type
/// plus zero or more `and`-joined features, matching real CSS Media
/// Queries Level 4 grammar for exactly the subset this engine
/// evaluates (`min-width`/`max-width`, `prefers-color-scheme`,
/// `prefers-reduced-motion`). `never`, once set, makes this item
/// permanently non-matching regardless of anything else in it — set
/// whenever parsing hits something this engine can't evaluate at all
/// (an unrecognized media type/feature, a `not` negation — not
/// implemented, see `parse_media_query_list`'s own docs — or a feature
/// value this engine can't compute, most notably `calc()`, which this
/// project has no arithmetic-expression evaluator for anywhere). This
/// mirrors real Media Queries syntax directly, not just an engineering
/// convenience: an individual query in a comma-separated list that
/// fails to parse is defined by the spec itself to evaluate as
/// "not all" (never matches) rather than invalidating the whole list
/// or the whole `@media` rule — so "safely never matches" here is
/// spec-accurate behavior, not a lossy fallback.
#[derive(Clone, Debug, PartialEq)]
struct MediaQuery {
    media_type: MediaType,
    min_width: Option<f32>,
    max_width: Option<f32>,
    prefers_color_scheme: Option<PrefersColorScheme>,
    prefers_reduced_motion: Option<PrefersReducedMotion>,
    never: bool,
}

impl Default for MediaQuery {
    /// An unqualified condition with no explicit media type at all
    /// (e.g. `@media (min-width:640px)`) implicitly means `all`, per
    /// real CSS Media Queries grammar — `all` is this type's own
    /// default, not a separate special case elsewhere.
    fn default() -> Self {
        MediaQuery {
            media_type: MediaType::All,
            min_width: None,
            max_width: None,
            prefers_color_scheme: None,
            prefers_reduced_motion: None,
            never: false,
        }
    }
}

impl MediaQuery {
    /// Whether this one query currently holds, given the live viewport
    /// width (in CSS px) — re-evaluated fresh every time this is
    /// called, never cached, since the whole point is that the answer
    /// can change across a resize (see `build_layout_tree`'s own docs).
    fn matches(&self, viewport_width: f32) -> bool {
        if self.never {
            return false;
        }
        match self.media_type {
            MediaType::All | MediaType::Screen => {}
            MediaType::Print => return false,
        }
        if let Some(min) = self.min_width
            && viewport_width < min
        {
            return false;
        }
        if let Some(max) = self.max_width
            && viewport_width > max
        {
            return false;
        }
        if let Some(pcs) = self.prefers_color_scheme
            && pcs != PrefersColorScheme::Light
        {
            return false;
        }
        if let Some(prm) = self.prefers_reduced_motion
            && prm != PrefersReducedMotion::NoPreference
        {
            return false;
        }
        true
    }
}

/// Whether a rule's own `media` list currently applies: an empty list
/// (not wrapped in any `@media`) always applies, independent of
/// `viewport_width`; a non-empty list applies if *any* one entry
/// matches (comma-separated = OR, real Media Queries list grammar).
fn media_list_matches(queries: &[MediaQuery], viewport_width: f32) -> bool {
    queries.is_empty() || queries.iter().any(|q| q.matches(viewport_width))
}

/// Parses one `(feature: value)` media feature, already positioned
/// right after the opening `(` has been consumed (i.e. called from
/// inside `input.parse_nested_block`, so `input` is scoped to exactly
/// this one parenthesized feature). Only `min-width`/`max-width`/
/// `prefers-color-scheme`/`prefers-reduced-motion` are recognized —
/// anything else (including a feature name this engine doesn't know,
/// or a value it can't parse, most notably `calc()` — `parse_length`
/// has no arithmetic-expression support at all, confirmed by grep
/// before writing this, so a `calc(...)` value simply fails here, the
/// same way any other unsupported value fails elsewhere in this file)
/// returns `Err(())`, which the caller turns into `MediaQuery::never`
/// — this one query item safely never matches rather than crashing or
/// guessing. Real CSS also allows a boolean (valueless) or range
/// (`<`/`>`/`<=`/`>=`) feature syntax — neither is attempted here,
/// same `Err(())` fallback.
fn parse_media_feature(input: &mut CssParser, q: &mut MediaQuery) -> Result<(), ()> {
    let name = input.expect_ident().map_err(|_| ())?.clone();
    input.expect_colon().map_err(|_| ())?;
    if name.eq_ignore_ascii_case("min-width") {
        q.min_width = Some(media_feature_length_px(input)?);
    } else if name.eq_ignore_ascii_case("max-width") {
        q.max_width = Some(media_feature_length_px(input)?);
    } else if name.eq_ignore_ascii_case("prefers-color-scheme") {
        let v = input.expect_ident().map_err(|_| ())?;
        q.prefers_color_scheme = Some(if v.eq_ignore_ascii_case("dark") {
            PrefersColorScheme::Dark
        } else if v.eq_ignore_ascii_case("light") {
            PrefersColorScheme::Light
        } else {
            return Err(());
        });
    } else if name.eq_ignore_ascii_case("prefers-reduced-motion") {
        let v = input.expect_ident().map_err(|_| ())?;
        q.prefers_reduced_motion = Some(if v.eq_ignore_ascii_case("reduce") {
            PrefersReducedMotion::Reduce
        } else if v.eq_ignore_ascii_case("no-preference") {
            PrefersReducedMotion::NoPreference
        } else {
            return Err(());
        });
    } else {
        return Err(());
    }
    Ok(())
}

/// A media feature's own length value, resolved all the way to a plain
/// px number at *parse* time (unlike `BoxLength`'s deferred-resolution
/// pattern elsewhere in this file) — a media feature has no element/
/// cascade context at all to resolve an `em`/`rem` against later, so
/// there's nothing to defer to. Reuses `parse_length` exactly as-is
/// (checked first: it already accepts `px`/`em`/`rem`, the task's own
/// "check what unit forms your length parser already accepts" guidance)
/// rather than writing a second length grammar — `calc()` fails here
/// for the same reason it fails in `parse_length` itself (no
/// `Function` arm matches any of its token patterns). `em`/`rem`
/// resolve against the same `16.0` UA-default root font-size
/// `ROOT_STYLE` uses, since a media feature is evaluated with no
/// cascaded font-size of any element in scope at all — the real spec
/// basis for either unit used in a media query is the initial
/// (UA-default) font-size, not some element's computed one.
fn media_feature_length_px(input: &mut CssParser) -> Result<f32, ()> {
    match parse_length(input)? {
        Length::Px(v) => Ok(v),
        Length::Em(v) | Length::Rem(v) => Ok(v * 16.0),
        // `parse_length` itself never actually produces any of these
        // (confirmed by its own match arms) — only reachable if that
        // changes later, in which case failing safely here is right
        // regardless of which new variant showed up.
        Length::Percent(_) | Length::Larger | Length::Smaller => Err(()),
    }
}

/// Parses a full `@media <media-query-list>` prelude (the part between
/// `@media` and the `{`) into one `MediaQuery` per comma-separated
/// item. Deliberately permissive/never hard-fails: an unrecognized
/// token at the top level (not `and`/`not`/`only`/a known media type/a
/// `(feature)` block) is just skipped rather than aborting the whole
/// list, and a `(feature)` block that fails to parse marks only its
/// own query item `never` (see `parse_media_feature`'s own docs) — so
/// a real, mostly-valid media query list is never thrown away wholesale
/// just because one clause in it uses something this engine doesn't
/// understand. `not` is recognized (so it doesn't fall through to the
/// "unrecognized media type" branch) but not actually evaluated as a
/// real negation — real CSS `not screen` / `not (min-width: ...)`
/// would need this engine to invert everything that follows it, which
/// isn't implemented; treated instead as "this query uses something
/// unsupported" and marked `never`, the same safe-fallback shape as any
/// other unsupported feature here, not a silent wrong-match. `only` is
/// recognized and skipped with no effect (legacy compatibility syntax
/// with no real bearing on evaluation either way).
fn parse_media_query_list(input: &mut CssParser) -> Vec<MediaQuery> {
    let mut queries = Vec::new();
    loop {
        let mut q = MediaQuery::default();
        loop {
            match input.next() {
                Ok(Token::Comma) => break,
                Ok(Token::Ident(id)) => {
                    if id.eq_ignore_ascii_case("and") || id.eq_ignore_ascii_case("only") {
                        // Combinator / legacy no-op keyword — nothing to
                        // record, keep scanning this same query item.
                    } else if id.eq_ignore_ascii_case("not") {
                        q.never = true;
                    } else if id.eq_ignore_ascii_case("screen") {
                        q.media_type = MediaType::Screen;
                    } else if id.eq_ignore_ascii_case("all") {
                        q.media_type = MediaType::All;
                    } else if id.eq_ignore_ascii_case("print") {
                        q.media_type = MediaType::Print;
                    } else {
                        // Unrecognized media type keyword — real spec:
                        // an unknown media type makes the whole query
                        // "not all", i.e. never matches.
                        q.never = true;
                    }
                }
                Ok(Token::ParenthesisBlock) => {
                    let result = input.parse_nested_block(|input| {
                        parse_media_feature(input, &mut q)
                            .map_err(|_| input.new_error_for_next_token::<()>())
                    });
                    if result.is_err() {
                        q.never = true;
                    }
                }
                Ok(_) => {
                    // Anything else this grammar doesn't expect — skip
                    // just this one token and keep scanning rather than
                    // abandoning the rest of the list.
                }
                Err(_) => {
                    // Ran out of input — end of the (already delimited)
                    // prelude.
                    queries.push(q);
                    return queries;
                }
            }
        }
        queries.push(q);
    }
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

/// `text-decoration-style`'s value. `double` and `wavy` (v60) are now real
/// variants, hand-painted the same way `Dashed`/`Dotted` already were
/// (v33) — see `paint_decoration_line`'s own docs for exactly how each
/// paints. Before v60, `double` wasn't a variant here at all: cosmic-text
/// natively supports a double *underline* (`UnderlineStyle::Double`), so
/// the old `parse_text_decoration` folded a `double` keyword straight into
/// `deco.underline` instead of going through this enum — but that only
/// ever reached underline (strikethrough/overline have no double variant
/// in cosmic-text 0.19's own `TextDecoration`), so `text-decoration:
/// line-through double` silently painted one plain line. `Double` is now
/// handled entirely by this file's own `paint_decoration_line` (doubling
/// applies uniformly to whichever line(s) are active — underline,
/// strikethrough, *and* overline — since `paint_decorations` never relied
/// on cosmic's native renderer at runtime to begin with, only mirrored its
/// formulas; see `paint_decorations`'s own docs). `pub` — appears in the
/// public `ShapedParagraph`/`ShapeCache` type aliases, same reason
/// `VerticalAlign`/`CursorStyle` are `pub` too.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum DecorationStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    /// Two parallel solid strokes, offset by one stroke's own thickness
    /// plus an equal gap — the exact formula cosmic-text's own
    /// `render_decoration` already uses for native double-underline,
    /// generalized here to strikethrough/overline too (real CSS applies
    /// `text-decoration-style: double` uniformly to every active line,
    /// not underline only).
    Double,
    /// A real sine-wave stroke, approximated as a sequence of short
    /// `Renderer::rectangle` segments tracing `sin` along the line's own
    /// extent — see `paint_decoration_line`'s own docs for the exact
    /// construction. Before v60 this kept falling back to `Solid` (no
    /// curve-rasterization primitive existed in this file at all); the
    /// `Renderer` trait still only offers axis-aligned rectangles, so this
    /// is a piecewise-linear approximation, not a true analytic curve —
    /// visually indistinguishable from one at the segment width used here.
    Wavy,
}

/// Just `text-decoration-line`'s value in isolation — only the
/// independent longhand (v60) needs this; the `text-decoration` shorthand
/// still parses straight into cosmic-text's own `TextDecoration` (`deco`)
/// since it always sets line+style+color together. `double`-underline
/// isn't representable here — that's a `-style` keyword, not a line
/// keyword — only which of the three lines are switched on. `none` (or no
/// keyword at all) is simply every field `false`, the same representable
/// "explicitly off" state `TextDecoration::new()` already gives the
/// shorthand.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
struct DecorationLine {
    underline: bool,
    strikethrough: bool,
    overline: bool,
}

/// What the `text-decoration` shorthand resolves to for one declaration —
/// `deco` (cosmic-text's own struct: which lines, and per-line color,
/// resolved directly inside `parse_text_decoration` since resolving a
/// color token needs no cascade context) plus `style` (solid/dashed/
/// dotted/double/wavy, painted by hand in `paint_decorations` instead of
/// cosmic-text's own renderer). Bundled as one struct because both always
/// come from the same shorthand *when the shorthand is what set them* —
/// but since v60, `text-decoration-line`/`-style`/`-color` can also each
/// be declared independently and combined with whatever this struct
/// already holds (ambient-propagated or shorthand-set) at used-value
/// time — see `resolve_own_decoration`, which every call site that used
/// to write `style.text_decoration.unwrap_or(decoration)` now goes through
/// instead.
#[derive(Clone, Copy, Default)]
struct DecorationValue {
    deco: TextDecoration,
    style: DecorationStyle,
}

/// Combines an element's own cascaded `text-decoration` shorthand and
/// `text-decoration-line`/`-style`/`-color` longhands (each independently
/// `Option`-cascaded — `None` means "not declared on this element") with
/// `ambient`, the effective `DecorationValue` propagated down from
/// whatever ancestor last set one (see `collect_children`'s own docs for
/// why this is ambient propagation, not real CSS inheritance). Mirrors
/// real CSS's own "combine at used-value time" semantics for these
/// properties: the shorthand (if declared here) replaces the ambient
/// value outright, exactly as before v60; each longhand then overrides
/// only its own one component on top of whatever's left (ambient, or the
/// shorthand's own result) — so `text-decoration-line: underline;
/// text-decoration-style: dashed; text-decoration-color: red;` as three
/// separate declarations combines into the same `DecorationValue` a
/// single `text-decoration: underline dashed red` shorthand would. A
/// `-color` override applies to whichever line(s) are active *after* the
/// `-line` override is applied (so `text-decoration-color` alone, with no
/// line declared anywhere, correctly has no visible effect — same "don't
/// leave a stray color on an inactive line" rule `parse_text_decoration`
/// already enforces for the shorthand). The *combined result* (not the
/// three raw longhands separately) is what gets threaded on as the new
/// ambient value for this element's own children — same propagation
/// shape the shorthand's own override already had, so a descendant that
/// declares none of these four properties still sees its nearest
/// ancestor's fully-combined effective decoration, longhands included.
fn resolve_own_decoration(style: &ComputedStyle, ambient: DecorationValue) -> DecorationValue {
    let mut result = style.text_decoration.unwrap_or(ambient);
    if let Some(line) = style.text_decoration_line {
        result.deco = TextDecoration::new();
        if line.underline {
            result.deco.underline = UnderlineStyle::Single;
        }
        result.deco.strikethrough = line.strikethrough;
        result.deco.overline = line.overline;
    }
    if let Some(s) = style.text_decoration_style {
        result.style = s;
    }
    if let Some(color) = style.text_decoration_color {
        if result.deco.underline != UnderlineStyle::None {
            result.deco.underline_color_opt = Some(color);
        }
        if result.deco.strikethrough {
            result.deco.strikethrough_color_opt = Some(color);
        }
        if result.deco.overline {
            result.deco.overline_color_opt = Some(color);
        }
    }
    result
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

/// The 5 `text-decoration-style`/shorthand style keywords this file
/// recognizes, shared between `parse_text_decoration` (the shorthand) and
/// `parse_text_decoration_style` (the v60 longhand) so the two can't drift.
fn decoration_style_from_keyword(kw: &str) -> Option<DecorationStyle> {
    if kw.eq_ignore_ascii_case("solid") {
        Some(DecorationStyle::Solid)
    } else if kw.eq_ignore_ascii_case("dashed") {
        Some(DecorationStyle::Dashed)
    } else if kw.eq_ignore_ascii_case("dotted") {
        Some(DecorationStyle::Dotted)
    } else if kw.eq_ignore_ascii_case("double") {
        Some(DecorationStyle::Double)
    } else if kw.eq_ignore_ascii_case("wavy") {
        Some(DecorationStyle::Wavy)
    } else {
        None
    }
}

fn parse_text_decoration(input: &mut CssParser) -> Result<(TextDecoration, DecorationStyle), ()> {
    let mut deco = TextDecoration::new();
    let mut style = DecorationStyle::Solid;
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
            } else if let Some(s) = decoration_style_from_keyword(ident) {
                style = s;
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

/// `text-decoration-style`'s own standalone longhand (v60) — exactly one
/// of the 5 keywords `decoration_style_from_keyword` recognizes; anything
/// else (including a trailing second token) fails the whole declaration,
/// same as any other single-keyword longhand in this file (e.g.
/// `font-style`).
fn parse_text_decoration_style(input: &mut CssParser) -> Result<DecorationStyle, ()> {
    let ident = input.expect_ident().map_err(|_| ())?;
    decoration_style_from_keyword(ident).ok_or(())
}

/// `text-decoration-line`'s own standalone longhand (v60): `none`, or any
/// of `underline`/`overline`/`line-through` in any combination/order (real
/// CSS grammar: `none | [ underline || overline || line-through ]`). Same
/// lenient "collect order-independently, decide at the end" shape
/// `parse_text_decoration` already uses for the shorthand — just without
/// a style/color half, since those are this file's two *other* longhands
/// now. Fails only if no recognized keyword was found at all.
fn parse_text_decoration_line(input: &mut CssParser) -> Result<DecorationLine, ()> {
    let mut line = DecorationLine::default();
    let mut found = false;
    loop {
        let state = input.state();
        match input.next() {
            Ok(Token::Ident(ident)) if ident.eq_ignore_ascii_case("underline") => {
                line.underline = true;
                found = true;
            }
            Ok(Token::Ident(ident)) if ident.eq_ignore_ascii_case("line-through") => {
                line.strikethrough = true;
                found = true;
            }
            Ok(Token::Ident(ident)) if ident.eq_ignore_ascii_case("overline") => {
                line.overline = true;
                found = true;
            }
            Ok(Token::Ident(ident)) if ident.eq_ignore_ascii_case("none") => {
                found = true;
            }
            _ => {
                input.reset(&state);
                break;
            }
        }
    }
    if found {
        Ok(line)
    } else {
        Err(())
    }
}

/// The 8 keywords that paint visibly differently from a plain solid
/// stroke. `Groove`/`Ridge`/`Inset`/`Outset` (PROMPTS.md v63) are real
/// two-tone bevel styles — CSS2.1 §8.5.3 — painted from lighter/darker
/// computed variants of the declared `border-color` (see `lighten`/
/// `darken`, `bevel_colors`): `Inset`/`Outset` paint each whole edge a
/// single light-or-dark variant (top/left vs. bottom/right); `Groove`/
/// `Ridge` additionally split each edge's own stroke width into two
/// sub-stripes (an "outer" half, farther from the box's own content,
/// and an "inner" half, closer to it) so a single edge shows a visible
/// crease, not just a flat shade — see `paint_border` for exactly how.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
enum BorderStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    Double,
    /// Carved-into-the-page bevel: top/left edges' outer half dark,
    /// inner half light; bottom/right edges the reverse.
    Groove,
    /// The opposite of `Groove`: top/left edges' outer half light,
    /// inner half dark; bottom/right edges the reverse.
    Ridge,
    /// Pressed-into-the-page bevel: top/left edges uniformly dark,
    /// bottom/right edges uniformly light — same light-direction logic
    /// as `Groove` but applied once per whole edge, no inner split.
    Inset,
    /// The opposite of `Inset`: top/left edges uniformly light,
    /// bottom/right edges uniformly dark.
    Outset,
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
    } else if kw.eq_ignore_ascii_case("groove") {
        BorderStyle::Groove
    } else if kw.eq_ignore_ascii_case("ridge") {
        BorderStyle::Ridge
    } else if kw.eq_ignore_ascii_case("inset") {
        BorderStyle::Inset
    } else if kw.eq_ignore_ascii_case("outset") {
        BorderStyle::Outset
    } else {
        // solid, and anything unrecognized.
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

/// One `<track-size>`: a `<length-percentage>` (reusing `parse_length_
/// percent`, the same px/%/em/rem grammar `width`/`margin`/etc. already
/// accept) or `<flex>` — the `fr` unit, which tokenizes as an ordinary
/// `<dimension-token>` with unit `fr` (same as `px`/`em`/`rem` already
/// do — no special-case lexing needed, same `state`/`reset` trial
/// pattern `parse_flex_basis`/`set_max_length` already use to try one
/// grammar before falling back to another). Content-based sizing
/// keywords (`auto`/`min-content`/`max-content`) and `minmax()` aren't
/// recognized — deferred, see `TODO.txt`.
fn parse_grid_track_size(input: &mut CssParser) -> Result<GridTrackSpec, ()> {
    let state = input.state();
    let fr = match input.next() {
        Ok(Token::Dimension { value, unit, .. }) if unit.eq_ignore_ascii_case("fr") => Some(*value),
        _ => None,
    };
    if let Some(v) = fr {
        return Ok(GridTrackSpec::Fr(v));
    }
    input.reset(&state);
    parse_length_percent(input).map(GridTrackSpec::Len)
}

/// `repeat(<integer>, <track-list>)` — the `N > 0` fixed-count form
/// only; `repeat(auto-fill, ...)`/`repeat(auto-fit, ...)` need real
/// available-space-driven track-count resolution (the number of tracks
/// depends on how many fit, not a literal count) — a materially
/// different algorithm, deferred (see `TODO.txt`). An `auto-fill`/
/// `auto-fit` first argument simply fails `expect_integer` here, same
/// "drop the whole declaration" outcome as any other unsupported form.
fn parse_grid_repeat(input: &mut CssParser) -> Result<Vec<GridTrackSpec>, ()> {
    let (count, one): (i32, Vec<GridTrackSpec>) = input
        .parse_nested_block(|input| {
            let count = input.expect_integer()?;
            input.expect_comma()?;
            let mut one = Vec::new();
            while !input.is_exhausted() {
                one.push(parse_grid_track_size(input).map_err(|_| input.new_error_for_next_token::<()>())?);
            }
            Ok((count, one))
        })
        .map_err(|_: ParseError<()>| ())?;
    if count <= 0 || one.is_empty() {
        return Err(());
    }
    let mut tracks = Vec::with_capacity(one.len() * count as usize);
    for _ in 0..count {
        tracks.extend_from_slice(&one);
    }
    Ok(tracks)
}

/// `grid-template-columns`/`grid-template-rows`'s own grammar, scoped
/// to what this item actually implements: a flat, space-separated list
/// of `<track-size>` entries (`parse_grid_track_size`) and `repeat(
/// <integer>, <track-list>)` (`parse_grid_repeat`) expanded eagerly
/// into that same flat list — no `minmax()`, no named lines/areas. The
/// real `none` keyword isn't matched explicitly — same "unparseable
/// leaves this property unset" convention as every other not-yet-
/// supported value in this file; an absent declaration and an explicit
/// `none` already mean the same thing to `measure_grid` (no explicit
/// tracks on this axis).
fn parse_grid_template(input: &mut CssParser) -> Result<Vec<GridTrackSpec>, ()> {
    let mut tracks = Vec::new();
    while !input.is_exhausted() {
        let state = input.state();
        if input.expect_function_matching("repeat").is_ok() {
            tracks.extend(parse_grid_repeat(input)?);
            continue;
        }
        input.reset(&state);
        tracks.push(parse_grid_track_size(input)?);
    }
    if tracks.is_empty() {
        Err(())
    } else {
        Ok(tracks)
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
                } else if ident.eq_ignore_ascii_case("grid") {
                    Some(Display::Grid)
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
                } else if ident.eq_ignore_ascii_case("scroll") {
                    Some(Overflow::Scroll)
                } else if ident.eq_ignore_ascii_case("auto") {
                    Some(Overflow::Auto)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("cursor") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.cursor = cursor_style_from_keyword(ident);
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
        } else if name.eq_ignore_ascii_case("grid-template-columns") {
            // Leaked once per distinct parsed declaration, same `Box::
            // leak` pattern `font-family`'s own fallback list already
            // uses (v51) to carry a variable-length value through
            // `ComputedStyle`/`Declarations` without losing their
            // `#[derive(Copy)]`.
            if let Ok(tracks) = parse_grid_template(input) {
                self.decls.grid_template_columns = Some(Box::leak(tracks.into_boxed_slice()));
            }
        } else if name.eq_ignore_ascii_case("grid-template-rows") {
            if let Ok(tracks) = parse_grid_template(input) {
                self.decls.grid_template_rows = Some(Box::leak(tracks.into_boxed_slice()));
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
        } else if name.eq_ignore_ascii_case("text-decoration-line") {
            // Independent longhand (v60) — see `resolve_own_decoration`
            // for how this combines with the shorthand/ambient value.
            if let Ok(line) = parse_text_decoration_line(input) {
                self.decls.text_decoration_line = Some(line);
            }
        } else if name.eq_ignore_ascii_case("text-decoration-style") {
            if let Ok(s) = parse_text_decoration_style(input) {
                self.decls.text_decoration_style = Some(s);
            }
        } else if name.eq_ignore_ascii_case("text-decoration-color") {
            if let Ok(c) = CssColor::parse(input) {
                self.decls.text_decoration_color = Some(c);
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
        } else if name.eq_ignore_ascii_case("word-spacing") {
            // `normal | <length>` — same `larger`/`smaller`-style
            // "try a keyword first, fall back to a length" shape
            // `font-size` already uses above, not `set_length`'s
            // leave-untouched-on-failure convention, since `normal` is a
            // real, explicitly-representable value here (zero extra
            // spacing), not just "unparsed."
            let state = input.state();
            let is_normal = input
                .next()
                .ok()
                .map(|t| matches!(t, Token::Ident(ident) if ident.eq_ignore_ascii_case("normal")))
                .unwrap_or(false);
            if is_normal {
                self.decls.word_spacing = Some(Length::Px(0.0));
            } else {
                input.reset(&state);
                if let Ok(v) = parse_length(input) {
                    self.decls.word_spacing = Some(v);
                }
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
    // Only `@media`'s own query-list grammar is parsed here — every
    // other at-rule name falls through to `parse_prelude`'s `Err` arm
    // below, which is exactly "skip @import/@font-face/@keyframes/etc."
    // for this stage, same as the old blanket no-op (see TODO.txt's own
    // narrower item for exactly what's still deferred and why).
    type Prelude = Vec<MediaQuery>;
    // One at-rule can expand into several real rules (every rule inside
    // an `@media` block), so this can't stay a single `StyleRule` the
    // way a plain qualified rule's `QualifiedRule` output can — see
    // `QualifiedRuleParser`'s own matching `Vec<StyleRule>` just below
    // (`StyleSheetParser` requires `AtRule` and `QualifiedRule` to be
    // the same type) and `parse_stylesheet`'s `rules.extend(..)`.
    type AtRule = Vec<StyleRule>;
    type Error = CssError;

    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut CssParser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        if name.eq_ignore_ascii_case("media") {
            Ok(parse_media_query_list(input))
        } else {
            Err(input.new_error_for_next_token())
        }
    }

    fn parse_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut CssParser<'i>,
    ) -> Result<Self::AtRule, ParseError<Self::Error>> {
        // The block's own content is an ordinary rule list (selectors +
        // declarations, same grammar as the top level) — recursing with
        // a fresh `TopLevelParser` over this already block-delimited
        // `input` reuses the exact same rule-list parsing rather than a
        // second, parallel implementation.
        let mut nested = TopLevelParser;
        let mut rules = Vec::new();
        let iter = StyleSheetParser::new(input, &mut nested);
        for inner_rules in iter.flatten() {
            for mut rule in inner_rules {
                // A rule nested directly inside this `@media` block
                // inherits this block's own condition. A rule that's
                // itself inside a *further*-nested `@media` (legal,
                // if rare, real CSS) already has its own, deeper
                // `media` list set by that recursive call — real CSS
                // would AND the two conditions together; this
                // deliberately doesn't (see this fn's own module
                // docs / TODO.txt), so the outer condition here is
                // simply left off whenever an inner one already won.
                if rule.media.is_empty() {
                    rule.media = prelude.clone();
                }
                rules.push(rule);
            }
        }
        Ok(rules)
    }
}

impl<'i> QualifiedRuleParser<'i> for TopLevelParser {
    type Prelude = SelectorList<OurSelectorImpl>;
    // Wrapped in a one-element `Vec` so this matches `AtRuleParser::
    // AtRule`'s own type exactly (`StyleSheetParser` requires both to
    // agree) — see that impl's own docs for why an at-rule needs more
    // than one slot.
    type QualifiedRule = Vec<StyleRule>;
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
        Ok(vec![StyleRule {
            selectors: prelude,
            decls,
            media: Vec::new(),
        }])
    }
}

fn parse_stylesheet(css: &str) -> Vec<StyleRule> {
    let mut input = CssParser::new(css);
    let mut top = TopLevelParser;
    let mut rules = Vec::new();
    let mut stylesheet = StyleSheetParser::new(&mut input, &mut top);
    while let Some(result) = stylesheet.next() {
        if let Ok(rule_list) = result {
            rules.extend(rule_list);
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
fn collect_root_custom_props(rules: &[StyleRule], html: &Handle, viewport_width: f32) -> HashMap<String, String> {
    let element = El(html.clone());
    let mut caches = SelectorCaches::default();
    let mut props = HashMap::new();
    for rule in rules {
        if !media_list_matches(&rule.media, viewport_width) {
            continue;
        }
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
    /// Real CSS inheritance (CSS2.1 §16.4 lists `word-spacing` as
    /// inherited, same as `letter-spacing`) — already resolved to a
    /// pixel value (like `border_width`/`outline_width`, not kept
    /// symbolic the way `line_height`'s bare-`<number>` form is), since
    /// `word-spacing`'s own grammar has no per-descendant-reinterpreted
    /// form the way `line-height: <number>` does.
    word_spacing: f32,
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
    word_spacing: 0.0,
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
    /// `text-decoration-line`/`-style`/`-color` (v60), each cascaded
    /// completely independently of `text_decoration` (and of each other)
    /// — `None` means not declared on *this* element, same convention as
    /// every other cascaded-but-not-inherited field here. Combined with
    /// `text_decoration` and the ambient propagated value together only
    /// at used-value time, by `resolve_own_decoration` — never folded
    /// into `text_decoration` itself during cascade, so a rule that
    /// declares only `text-decoration-color` genuinely doesn't clobber
    /// whatever line/style an ancestor or the shorthand already set.
    text_decoration_line: Option<DecorationLine>,
    text_decoration_style: Option<DecorationStyle>,
    text_decoration_color: Option<TextColor>,
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
    /// Real CSS inheritance (see `InheritedStyle::word_spacing`'s own
    /// docs), already resolved to pixels. **Deliberately never read by
    /// `shape_spans`/cosmic-text's own `Attrs`** — unlike the reverted
    /// `letter-spacing` attempt (see TODO.txt), which wired extra
    /// spacing straight into cosmic-text's own per-glyph shaping and hit
    /// a real cosmic-text 0.19 bug where its *internal* wrap-width math
    /// started misbehaving once that spacing crossed a small threshold.
    /// cosmic-text 0.19's `Attrs` has no `word_spacing` knob to hook into
    /// at all anyway (confirmed by grepping its vendored source —
    /// `letter_spacing_opt` is the only tracking-style field it exposes),
    /// so this never had that option in the first place. Instead, only
    /// `measure`'s routing check (just below `BoxContent::Inline`'s
    /// match arm) and `measure_mixed_inline` itself ever read this field
    /// — a nonzero value routes the run through that hand-rolled,
    /// word-by-word greedy line-breaker (built for inline `<img>` in v48,
    /// already extended to float-affected plain text in v54) instead of
    /// cosmic-text's own whole-paragraph `Buffer`/`Wrap` path, and the
    /// extra spacing is added directly to a word's own measured advance
    /// there — a pure position/advance change in scratchy's own code,
    /// never something cosmic-text's own internal wrap computation has
    /// to account for.
    word_spacing: f32,
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
    /// `display` is). The two `*_gap` fields are genuinely shared, not
    /// flexbox-specific — `measure_grid` reads them too (real CSS `gap`
    /// is a generic Box Alignment property, not a flexbox one; this
    /// file's implementation happened to land on a flex container
    /// first, but nothing about the fields themselves is flex-only).
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
    /// Not inherited, same category as the flexbox-only fields above —
    /// a `display: grid` container's own explicit column/row tracks
    /// never reach past it to a grandchild. `None` means no `grid-
    /// template-columns`/`-rows` declared at all — `measure_grid` falls
    /// back to a single full-width implicit column for `grid_template_
    /// columns` and purely content-sized implicit rows for `grid_
    /// template_rows` either way (see its own docs), so `None` and
    /// "every row/column is auto" are already the right default.
    grid_template_columns: Option<&'static [GridTrackSpec]>,
    grid_template_rows: Option<&'static [GridTrackSpec]>,
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
            word_spacing: self.word_spacing,
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
fn matching_declarations<'a>(handle: &Handle, rules: &'a [StyleRule], viewport_width: f32) -> Vec<(u32, &'a Declarations)> {
    let element = El(handle.clone());
    let mut caches = SelectorCaches::default();
    let mut matches: Vec<(u32, &Declarations)> = Vec::new();
    for rule in rules {
        // The one choke point every cascade/matching call in this file
        // goes through — a media-conditioned rule is only ever a
        // cascade candidate when its own condition currently holds for
        // `viewport_width`. See `MediaQuery`'s own docs for exactly
        // what's evaluated. Checked before the (more expensive) real
        // selector match, same short-circuit order `collect_root_
        // custom_props` also uses.
        if !media_list_matches(&rule.media, viewport_width) {
            continue;
        }
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
/// `Arc` — same reasoning, and the same `Arc<DecodedImage>` type, as
/// `BoxContent::Image` (see its own docs). Only a `data:` URI `url(...)`
/// decodes — same `<img src>` policy, see `decode_data_uri`'s own docs.
fn resolve_background_image(handle: &Handle, rules: &[StyleRule], viewport_width: f32) -> Option<Arc<DecodedImage>> {
    let matches = matching_declarations(handle, rules, viewport_width);
    let inline_decls = element_attr(handle, "style").map(|s| parse_declarations(&s));
    let url = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.background_image.clone()),
        |d| d.background_image.clone(),
    )?;
    decode_data_uri(&url).and_then(|b| decode_image_bytes(&b)).map(Arc::new)
}

fn resolve_style(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle, viewport_width: f32) -> ComputedStyle {
    let matches = matching_declarations(handle, rules, viewport_width);
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
    let text_decoration_line = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.text_decoration_line),
        |d| d.text_decoration_line,
    );
    let text_decoration_style = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.text_decoration_style),
        |d| d.text_decoration_style,
    );
    let text_decoration_color = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.text_decoration_color),
        |d| d.text_decoration_color,
    )
    .and_then(|c| to_text_color(&c));
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
    // Eagerly resolved to pixels, like `border_width`/`outline_width`
    // just below (real CSS grammar is a plain `<length>`, no `%`, so
    // there's no layout-time basis to defer this against the way
    // `width`/`margin`/etc. need `BoxLength` for).
    let word_spacing = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.word_spacing),
        |d| d.word_spacing,
    )
    .map(|l| l.resolve(font_size, inherited.root_font_size))
    .unwrap_or(inherited.word_spacing);
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
    let grid_template_columns = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.grid_template_columns),
        |d| d.grid_template_columns,
    );
    let grid_template_rows = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.grid_template_rows),
        |d| d.grid_template_rows,
    );

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
        text_decoration_line,
        text_decoration_style,
        text_decoration_color,
        white_space,
        list_style_type,
        text_align,
        vertical_align,
        line_height,
        visibility,
        word_spacing,
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
        grid_template_columns,
        grid_template_rows,
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
        text_decoration_line: None,
        text_decoration_style: None,
        text_decoration_color: None,
        white_space: inherited.white_space,
        list_style_type: inherited.list_style_type,
        text_align: inherited.text_align,
        vertical_align: VerticalAlign::default(),
        line_height: inherited.line_height,
        visibility: inherited.visibility,
        word_spacing: inherited.word_spacing,
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
        grid_template_columns: None,
        grid_template_rows: None,
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
    /// Real CSS inheritance too (see `ComputedStyle::word_spacing`'s own
    /// docs) — carried per-span, same as `font_size`/`font_family` above,
    /// so a nested element overriding `word-spacing` only affects its own
    /// words, not its siblings'. Only ever read by `measure_mixed_
    /// inline`, which applies it directly to a word's own measured
    /// advance (see its own docs) — never by `shape_spans`/cosmic-text.
    word_spacing: f32,
    /// The absolute-or-relative `href` text of the nearest enclosing
    /// `a`/`area` with an `href` attribute, if any — not a CSS property
    /// at all, so it's carried the same *ambient* way `background_color`/
    /// `text_decoration` are (ancestor's value flows down to any
    /// descendant text that doesn't come from its own, more specific,
    /// `a href`), computed once per element in `collect_children` rather
    /// than resolved through the cascade. Leaked (`Box::leak`, same
    /// tradeoff `FontFamily`'s own named lists already made, see
    /// TODO.txt) into a `&'static str` so `SpanStyle` can stay
    /// `#[derive(Copy)]` — bounded by how many distinct `a href` values
    /// are walked per layout rebuild, not by click/redraw count.
    /// `hit_test_cursor` reads this back out per-glyph (via `glyph.
    /// metadata`, the same mechanism it already uses for `cursor`) to
    /// answer "is this exact point a link, and to what" for
    /// `WindowEvent::MouseInput`'s click handler.
    href: Option<&'static str>,
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
            word_spacing: s.word_spacing,
            href: None,
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
        image: Arc<DecodedImage>,
        style: ComputedStyle,
        background_image: Option<Arc<DecodedImage>>,
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
    /// A decoded `<img>` — see `build_img_box`. Wrapped in `Arc` so
    /// `measure` can hand `Laid` its own cheap reference (`LaidContent::
    /// Image`) without re-decoding or copying the pixel buffer on every
    /// layout pass.
    Image(Arc<DecodedImage>),
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
    background_image: Option<Arc<DecodedImage>>,
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
/// Image` instead, see its own docs). A `data:` URI `src` decodes
/// immediately, same as always; anything else — a real `http(s)://` URL
/// — is looked up in `image_cache` instead (keyed by the exact,
/// unresolved `src` attribute text — safe only because `App::replace_
/// document` clears this cache on every navigation, so the same raw
/// relative string can never carry over from a *different* page's own
/// base URL; see `App::image_cache`'s own docs). Not yet fetched, or a
/// fetch that failed/decoded to nothing — missing, unsupported format, a
/// local file path (not supported, see TODO.txt), or corrupt data —
/// both fall out of a plain cache miss, `None` either way.
fn resolve_img(handle: &Handle, mut style: ComputedStyle, image_cache: &HashMap<String, Arc<DecodedImage>>) -> (ComputedStyle, Option<Arc<DecodedImage>>) {
    if style.width.is_none() {
        style.width = img_size_attr(handle, "width").map(BoxLength::Px);
    }
    if style.height.is_none() {
        style.height = img_size_attr(handle, "height").map(BoxLength::Px);
    }
    let decoded = element_attr(handle, "src").and_then(|src| match decode_data_uri(&src) {
        Some(bytes) => decode_image_bytes(&bytes).map(Arc::new),
        None => image_cache.get(&*src).cloned(),
    });
    (style, decoded)
}

/// Builds `<img>`'s own box directly, bypassing the normal children walk
/// (it's a void element — nothing to recurse into) — only reached for a
/// `display: block`/`table` `<img>` (see `collect_children`'s own docs);
/// the default `display: inline` case never calls this. A `src` that
/// fails to decode falls back to rendering `alt` as plain text in the
/// image's place, same as a real browser's "fallback content" for a
/// replaced element that failed to load, rather than collapsing to a
/// blank 0×0 box.
fn build_img_box(
    handle: &Handle,
    style: ComputedStyle,
    background_image: Option<Arc<DecodedImage>>,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> LayoutBox {
    let (style, decoded) = resolve_img(handle, style, image_cache);
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
fn build_box_for_element(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    decoration: DecorationValue,
    href: Option<&'static str>,
    viewport_width: f32,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> LayoutBox {
    let style = resolve_style(handle, rules, inherited, viewport_width);
    // Resolved once here (cheap to carry further — just an `Arc` clone —
    // vs. re-running `resolve_background_image`'s own selector-matching
    // pass again in every branch below).
    let background_image = resolve_background_image(handle, rules, viewport_width);
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
        return build_img_box(handle, style, background_image, image_cache);
    }
    let own_decoration = resolve_own_decoration(&style, decoration);
    if style.display == Display::Table {
        return LayoutBox {
            style,
            background_image,
            content: BoxContent::Blocks(build_table_children(
                handle,
                rules,
                style.inherited(),
                own_decoration,
                href,
                viewport_width,
                image_cache,
            )),
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
            content: BoxContent::Blocks(build_flex_children(
                handle,
                rules,
                style.inherited(),
                own_decoration,
                href,
                viewport_width,
                image_cache,
            )),
        };
    }
    // `display: grid` is block-level in the outer flow (same as `Table`/
    // `Flex`, just above) but lays its content out as a 2D grid instead —
    // each direct child becomes a grid item, placed per `measure_grid`.
    // Reuses `build_flex_children` outright rather than a near-duplicate
    // function: both container types "blockify" every direct child
    // identically (see that function's own docs) — there's no grid-
    // specific difference at the box-*building* stage, only at the
    // measuring stage (`measure_grid`, not this function).
    if style.display == Display::Grid {
        return LayoutBox {
            style,
            background_image,
            content: BoxContent::Blocks(build_flex_children(
                handle,
                rules,
                style.inherited(),
                own_decoration,
                href,
                viewport_width,
                image_cache,
            )),
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
        content: BoxContent::Blocks(build_children_with_prefix(
            handle,
            rules,
            style.inherited(),
            prefix,
            own_decoration,
            href,
            viewport_width,
            image_cache,
        )),
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
fn build_table_children(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    decoration: DecorationValue,
    href: Option<&'static str>,
    viewport_width: f32,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    for child in handle.children.borrow().iter() {
        let NodeData::Element { .. } = &child.data else { continue };
        let tag = element_tag(child);
        if tag == "script" || tag == "style" {
            continue;
        }
        let style = resolve_style(child, rules, inherited, viewport_width);
        // Resolved once here (not left to `build_box_for_element`, since
        // the `TableRow`/`TableGroup` arms build their `LayoutBox`
        // directly rather than delegating to it) — the `Block` arm below
        // passes the un-resolved `decoration` straight through instead,
        // since `build_box_for_element` already redoes this same
        // combine itself once it has the child's own `style`.
        let child_decoration = resolve_own_decoration(&style, decoration);
        match style.display {
            Display::None => {}
            Display::TableRow => {
                let cells =
                    build_table_row_cells(child, rules, style.inherited(), child_decoration, href, viewport_width, image_cache);
                let background_image = resolve_background_image(child, rules, viewport_width);
                result.push(LayoutBox { style, background_image, content: BoxContent::TableRow(cells) });
            }
            Display::TableGroup => {
                result.extend(build_table_children(
                    child,
                    rules,
                    style.inherited(),
                    child_decoration,
                    href,
                    viewport_width,
                    image_cache,
                ));
            }
            Display::Block | Display::Flex | Display::Grid => {
                // e.g. `caption`, or any other block-level content placed
                // directly under `<table>` — a stray `display: flex`/
                // `grid` element here is the same kind of edge case,
                // handled the same way (its own box via the normal
                // dispatch, which re-enters `build_flex_children`/
                // `measure_flex`/`measure_grid` for it).
                result.push(build_box_for_element(child, rules, inherited, decoration, href, viewport_width, image_cache));
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
fn build_table_row_cells(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    decoration: DecorationValue,
    href: Option<&'static str>,
    viewport_width: f32,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> Vec<LayoutBox> {
    let mut cells = Vec::new();
    for child in handle.children.borrow().iter() {
        let NodeData::Element { .. } = &child.data else { continue };
        let style = resolve_style(child, rules, inherited, viewport_width);
        if style.display == Display::TableCell {
            cells.push(build_box_for_element(child, rules, inherited, decoration, href, viewport_width, image_cache));
        }
    }
    cells
}

/// A `display: flex` **or `display: grid`** element's direct children,
/// each becoming its own flex/grid item — shared by both container
/// types (see `build_box_for_element`'s own `Grid` branch), since real
/// CSS "blockifies" every flex/grid item identically (spec §4/§3: a
/// flex or grid item's outer display type is always block-level,
/// regardless of its own `display`), which falls out *for free* here:
/// `build_box_for_element` already builds a full `Blocks`-content box
/// for whatever it's given regardless of that element's own `style.
/// display` (it only ever branches on `display` to special-case
/// `img`/`Table`/`Flex`/`Grid` themselves, never to decide "does this
/// get its own box at all" — only `collect_children`'s inline-run
/// grouping makes that decision, which this function bypasses entirely
/// by calling `build_box_for_element` on every child directly). So a
/// `<span>` (default `display: inline`) placed directly inside a
/// flex/grid container still gets its own real box here, exactly like
/// a `<div>` would — nothing extra needed to force block-level
/// behavior.
///
/// **Deliberately not attempted** (see `TODO.txt`): a bare text node
/// directly inside the container (no wrapping element) doesn't become
/// its own anonymous flex/grid item the way real CSS would — only
/// element children are collected here, matching the task's own
/// explicitly-permitted scope-narrowing latitude; non-whitespace text
/// placed directly in a flex/grid container (uncommon in real page
/// chrome, which almost always wraps each item in an element) is
/// silently dropped rather than rendered. `position: absolute`/`fixed`
/// children are *not* filtered out here (unlike `build_table_
/// children`'s own `Display::None` skip) — they're still real children
/// of this returned list, but `measure_flex`/`measure_grid` themselves
/// (not this function) are what actually take them out of their own
/// respective algorithm, the same split `measure`'s own `BoxContent::
/// Blocks` arm already uses for an ordinary block's absolutely/fixed
/// positioned children.
fn build_flex_children(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    decoration: DecorationValue,
    href: Option<&'static str>,
    viewport_width: f32,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    for child in handle.children.borrow().iter() {
        let NodeData::Element { .. } = &child.data else { continue };
        let tag = element_tag(child);
        if tag == "script" || tag == "style" {
            continue;
        }
        let style = resolve_style(child, rules, inherited, viewport_width);
        if style.display == Display::None {
            continue;
        }
        result.push(build_box_for_element(child, rules, inherited, decoration, href, viewport_width, image_cache));
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
fn build_children(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    decoration: DecorationValue,
    href: Option<&'static str>,
    viewport_width: f32,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> Vec<LayoutBox> {
    build_children_with_prefix(handle, rules, inherited, None, decoration, href, viewport_width, image_cache)
}

/// Same as `build_children`, but seeds the first inline run with `prefix`
/// (used for a `<li>`'s marker) before any of `handle`'s own children.
fn build_children_with_prefix(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    prefix: Option<(String, SpanStyle)>,
    decoration: DecorationValue,
    href: Option<&'static str>,
    viewport_width: f32,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
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
        href,
        VerticalAlign::default(),
        &mut pending,
        &mut result,
        viewport_width,
        image_cache,
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
    href: Option<&'static str>,
    valign: VerticalAlign,
    pending: &mut Vec<InlineItem>,
    result: &mut Vec<LayoutBox>,
    viewport_width: f32,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) {
    for child in handle.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => {
                let mut span: SpanStyle = inherited.into();
                span.background_color = bg;
                span.text_decoration = decoration.deco;
                span.decoration_style = decoration.style;
                span.vertical_align = valign;
                span.href = href;
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
                let style = resolve_style(child, rules, inherited, viewport_width);
                if style.display == Display::None {
                    continue;
                }
                // Not a CSS property at all — a plain HTML-attribute
                // lookup, same ambient-propagation shape `bg`/`decoration`
                // above already use (see `SpanStyle::href`'s own docs).
                // An `a`/`area` with its own `href` always wins for its
                // own subtree; anything else (including an `a`/`area`
                // with *no* `href`, a plain fragment target rather than a
                // real link — see `OurPseudoClass::Link`'s own docs for
                // why that distinction matters) just keeps carrying
                // whatever ancestor link, if any, it's already nested
                // inside.
                let child_href = if matches!(tag, "a" | "area") {
                    element_attr(child, "href").map(|h| leak_str(&h)).or(href)
                } else {
                    href
                };
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
                if tag == "img" && !matches!(style.display, Display::Block | Display::Table | Display::Flex | Display::Grid) {
                    let background_image = resolve_background_image(child, rules, viewport_width);
                    let (img_style, decoded) = resolve_img(child, style, image_cache);
                    match decoded {
                        Some(image) => pending.push(InlineItem::Image { image, style: img_style, background_image }),
                        None => {
                            let alt = element_attr(child, "alt").map(|a| a.to_string()).unwrap_or_default();
                            let mut span: SpanStyle = inherited.into();
                            span.background_color = bg;
                            span.vertical_align = valign;
                            span.href = child_href;
                            pending.push(InlineItem::Text(alt, span));
                        }
                    }
                } else if matches!(style.display, Display::Block | Display::Table | Display::Flex | Display::Grid) {
                    if !pending.is_empty() {
                        result.push(LayoutBox {
                            style: anonymous_style(inherited),
                            background_image: None,
                            content: BoxContent::Inline(std::mem::take(pending)),
                        });
                    }
                    result.push(build_box_for_element(child, rules, inherited, decoration, child_href, viewport_width, image_cache));
                } else {
                    let child_bg = style.background_color.or(bg);
                    let child_decoration = resolve_own_decoration(&style, decoration);
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
                        child_href,
                        child_valign,
                        pending,
                        result,
                        viewport_width,
                        image_cache,
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

/// `viewport_width` (CSS px) is the live window width at the moment this
/// is called — the one new input a `@media` `min-width`/`max-width`
/// rule's own applicability depends on (see `MediaQuery`/`matching_
/// declarations`). No new resize-reaction mechanism was added for this:
/// `App::redraw` already calls `build_layout_tree` fresh every time the
/// window's width changes (`self.cached_width != width`, to begin with
/// for `%`/`em`/`rem`/table/flex/grid measurement's own sake) — passing
/// that same live width through here just makes the rules a rebuild
/// picks up media-aware too, with zero new plumbing on the resize side.
///
/// A thin wrapper over `build_layout_tree_with_resources` supplying no
/// live-fetched CSS and an empty image cache — every test/benchmark call
/// site (there are dozens) keeps using this simpler signature unchanged;
/// only `App::redraw`'s one real call site needs the richer one.
pub fn build_layout_tree(document: &Handle, viewport_width: f32) -> LayoutBox {
    build_layout_tree_with_resources(document, viewport_width, "", &HashMap::new())
}

/// Same as `build_layout_tree`, but also takes CSS fetched (via `wire`)
/// from this page's own `<link rel=stylesheet href>` references
/// (`extra_css` — concatenated into the same stylesheet source
/// `collect_style_css`'s own inline `<style>` text already feeds into,
/// so it cascades identically; see `App::extra_css`'s own docs for how
/// it gets populated) and a cache of already-decoded images fetched for
/// non-`data:` `<img src>` references (`image_cache`, consulted by
/// `resolve_img` — see `App::image_cache`'s own docs).
fn build_layout_tree_with_resources(
    document: &Handle,
    viewport_width: f32,
    extra_css: &str,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> LayoutBox {
    let mut css_source = String::new();
    collect_style_css(document, &mut css_source);
    css_source.push('\n');
    css_source.push_str(extra_css);
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
            .map(|html| collect_root_custom_props(&rules, html, viewport_width))
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
        .map(|html| resolve_style(html, &rules, ROOT_STYLE, viewport_width).font_size)
        .unwrap_or(ROOT_STYLE.font_size);
    let root_inherited = InheritedStyle {
        root_font_size,
        ..ROOT_STYLE
    };
    LayoutBox {
        style: anonymous_style(root_inherited),
        background_image: None,
        content: BoxContent::Blocks(build_children(
            document,
            &rules,
            root_inherited,
            DecorationValue::default(),
            None,
            viewport_width,
            image_cache,
        )),
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
    /// from (an `Arc` clone — see its own docs for why it isn't on
    /// `style`), for `paint` to tile behind this box's own content.
    background_image: Option<Arc<DecodedImage>>,
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
    /// `Some` only for an `overflow: scroll`/`auto` box (v62) — the
    /// bounds a *live* per-element scroll offset (kept outside `Laid`
    /// entirely — see `ScrollInfo`'s own docs) gets clamped against,
    /// resolved once here at layout time same as `sticky` just above.
    /// `None` for every other box, including a `scroll`/`auto` box whose
    /// content doesn't actually overflow — it's still tracked as a
    /// `ScrollInfo` (`max_scroll()` just comes out `0.0`), not treated
    /// as if it weren't scrollable at all, so `Auto`'s own "only show a
    /// scrollbar when it actually overflows" check has a real number to
    /// compare against.
    scroll: Option<ScrollInfo>,
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

/// `overflow: scroll`/`auto`'s own layout-time-resolved state (v62) —
/// same split `StickyInfo` (v55) already established for a different
/// property: resolve the *bounds* once, at layout time, from values
/// `measure` already has lying around; never store the *live* scroll
/// amount here at all (that would let a mouse-wheel tick invalidate
/// `Laid`, exactly the "scrolling never touches layout" invariant this
/// whole split exists to protect — see NOTES.md's v4/v5 writeup). The
/// live amount instead lives in `App::scroll_offsets`, a
/// `HashMap<usize, f32>` keyed by `id`, read fresh by `paint`/
/// `hit_test_cursor`/`hit_test_scroll_region` every call — a box's own
/// current scroll is just that map's value for its `id` (defaulting to
/// `0.0` and re-clamped against `max_scroll()` at every read site,
/// rather than trusted to already be in range — cheap, and self-healing
/// if a relayout ever shrinks `content_h` out from under a stale,
/// larger stored offset).
#[derive(Clone, Copy)]
struct ScrollInfo {
    /// Assigned from the same shared `next_id` counter `measure`'s own
    /// paragraph ids already use (`ShapeCache`'s key namespace) — a
    /// different, unrelated-by-value id space, since `App::
    /// scroll_offsets` is its own separate `HashMap`, so there's no
    /// collision risk despite sharing a counter. Reusing it avoids
    /// threading yet another `&mut usize` through every recursive
    /// `measure` call just for this.
    id: usize,
    /// This box's own full in-flow content height — before any
    /// `overflow` clipping, exactly the same quantity `measure`'s
    /// `BoxContent::Blocks` arm already computes as `content_h` for its
    /// own auto-height calculation.
    content_h: f32,
    /// This box's own final, clamped content-box height (`border_h`
    /// minus the border/padding `extra_h` `measure` already subtracts)
    /// — the visible window into `content_h`. Can be *smaller* than an
    /// `auto`-height box's own natural content height whenever an
    /// explicit `height`/`max-height` clamps it, which is exactly the
    /// case that makes this box scrollable at all (real CSS `overflow:
    /// visible` would let the extra content spill out instead — see
    /// v42's own height-overflow note).
    client_h: f32,
}

impl ScrollInfo {
    /// How far this box's own live scroll offset can go before it runs
    /// out of content to reveal — `0.0` whenever `content_h <= client_h`
    /// (nothing actually overflows, the `Auto`-vs-`Scroll` distinction
    /// this whole property hinges on).
    fn max_scroll(&self) -> f32 {
        (self.content_h - self.client_h).max(0.0)
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
    Image(Arc<DecodedImage>),
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
/// text-decoration style, per-span link href)`.
pub type ShapedParagraph = (
    Buffer,
    f32,
    Vec<Option<TextColor>>,
    Vec<VerticalAlign>,
    Vec<bool>,
    Vec<CursorStyle>,
    Vec<DecorationStyle>,
    Vec<Option<&'static str>>,
);

/// Each entry wrapped in `Arc<Mutex<_>>` — not for real concurrent
/// mutation (only `paint`/`paint_buffer`, main-thread-only, ever takes
/// `&mut` through the lock; `App::spawn_shaping_worker`'s own thread
/// only ever reads an existing entry via `.get`, never writes one a
/// second time) but so the *whole map* is cheaply `Clone` — plain
/// refcount bumps, no `cosmic_text::Buffer` ever actually copied —
/// letting a background shaping pass hand the main thread a snapshot
/// of everything shaped *so far* every tick while it keeps shaping
/// into its own, same, continuing map. See `spawn_shaping_worker`'s own
/// docs for why that's what makes a page fill in progressively again
/// now that shaping runs off the main thread.
pub type ShapeCache = std::collections::HashMap<usize, Arc<Mutex<ShapedParagraph>>>;

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
    let hrefs = spans.iter().map(|(_, span)| span.href).collect();
    (buffer, height, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs)
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
    // `display: grid` — same shape of special-case as `Table`/`Flex`
    // just above, and for the same reason: a grid's own column (and,
    // where explicit, row) track sizes are a container-wide decision
    // made before any one item can be measured. See `measure_grid`'s
    // own docs for the real algorithm. A grid container establishes its
    // own fresh BFC (same category as a table/flex container), so
    // `floats`/`auto_margin_box_override` have nothing to do with it
    // either way; `measure_grid` doesn't take either.
    if b.style.display == Display::Grid
        && let BoxContent::Blocks(children) = &b.content
    {
        return measure_grid(
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
                || (!floats.is_empty() && content_y < floats.max_bottom())
                || items.iter().any(|i| matches!(i, InlineItem::Text(_, span) if span.word_spacing != 0.0)) =>
        {
            // Real inline-level flow for a run that mixes text with at
            // least one inline `<img>` — a materially different (and
            // much more expensive) path than the plain-text case below,
            // so only taken when actually needed; see `measure_mixed_
            // inline`'s own docs. Also taken (the `float`/`clear` item,
            // v54) for a *plain-text* run that starts before every float
            // in this BFC has already ended, since only this hand-rolled
            // per-word breaker can narrow/shift individual lines around
            // a float — cosmic-text's own whole-buffer `Buffer`/`Wrap`
            // path below has one fixed width for the entire paragraph.
            // Known cost, not attempted away: once *any* float exists
            // anywhere earlier in this BFC, every paragraph that starts
            // before the *lowest* float's bottom edge pays this slower
            // per-word path, even one that turns out not to actually
            // overlap any specific float once its own line positions are
            // known — see TODO.txt.
            //
            // Also taken (new — `word-spacing`) whenever any span in
            // this run actually declares a nonzero `word-spacing`, with
            // no image or float involved at all — the same "route
            // through the hand-rolled per-word path instead of cosmic-
            // text's own whole-buffer one" move v54 already made for
            // floats, now for the same underlying reason: this is the
            // only path with per-word advances scratchy can adjust
            // itself, without asking cosmic-text's own shaping/wrap-
            // width math to account for extra spacing it never sees
            // (exactly the risk that broke the reverted `letter-spacing`
            // attempt — see TODO.txt and `ComputedStyle::word_spacing`'s
            // own docs). A plain-text run with `word-spacing: normal`
            // (the overwhelming common case) is completely unaffected —
            // still the fast single-`Buffer` path below, zero regression
            // risk/cost.
            //
            // Not gated on `s.word_spacing` (this box's own anonymous-
            // inline-run style) because a run can mix spans from
            // different elements with different `word-spacing` values
            // (e.g. plain text next to a `<span>` that overrides it) —
            // checking every span individually catches that case too,
            // not just a value declared on the run's own synthetic box.
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
                scroll: None,
            }
        }
        BoxContent::Inline(items) => {
            let id = *next_id;
            *next_id += 1;

            let content_h = if let Some(entry) = cache.get(&id) {
                entry.lock().unwrap().1
            } else if items.is_empty() {
                cache.insert(
                    id,
                    Arc::new(Mutex::new((
                        Buffer::new_empty(Metrics::new(16.0, 20.0)),
                        0.0,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    ))),
                );
                0.0
            } else if std::time::Instant::now() < deadline {
                let (buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs) = shape_spans(
                    font_system,
                    items,
                    content_w,
                    s.white_space == WhiteSpace::Pre,
                    s.text_align.to_cosmic(),
                );
                cache.insert(id, Arc::new(Mutex::new((buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs))));
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
                scroll: None,
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
            // `overflow: scroll`/`auto` (v62): this box becomes its own
            // independently-scrollable region — see `ScrollInfo`'s own
            // docs. Deliberately scoped to this ordinary `Blocks` arm
            // only, not `measure_table`/`measure_flex`/`measure_grid`'s
            // own, separate tails below (see TODO.txt) — the single
            // most common real case (a plain block container whose
            // content overflows its own box), same "narrow choke point
            // over the general case" latitude `overflow: hidden`'s own
            // v31/v40 scoping already used. `client_h` is this box's own
            // just-computed `border_h` minus the same `extra_h` (border
            // + padding) already subtracted everywhere else in this
            // function — the content-box height `content_h` is actually
            // visible through, before any scrolling moves it.
            let scroll = matches!(s.overflow, Overflow::Scroll | Overflow::Auto).then(|| {
                let id = *next_id;
                *next_id += 1;
                ScrollInfo { id, content_h: content_h.max(0.0), client_h: (border_h - extra_h).max(0.0) }
            });
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
                scroll,
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
                scroll: None,
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
                content: LaidContent::Image(Arc::clone(image)),
                positioned: Vec::new(),
                fixed: false,
                sticky: None,
                scroll: None,
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
/// Also where `word-spacing` (v59) is actually applied, now that any run
/// with a nonzero value is routed here (see `measure`'s own routing
/// check): baked directly into a word's own measured `width` whenever
/// its token carries the trailing space that marks a real word boundary
/// (`word.ends_with(' ')`) — not a separate field threaded through every
/// downstream use, since every consumer of a word's width here (the
/// wrap-check/cursor-advance just below, the final line-width/`text-
/// align` math, and the hit-test bounding box in `MixedPositioned::
/// Word`) already wants the bigger number for free. cosmic-text's own
/// shaping of that one word is completely untouched — its `Buffer` is
/// shaped at the word's own natural width, with no spacing concept fed
/// into `Attrs` at all; only the *position* of the next word on the line
/// moves. This is the deliberate reason this item routes through here at
/// all instead of cosmic-text's own `Attrs`/`Buffer` (like the reverted
/// `letter-spacing` attempt did, see TODO.txt): cosmic-text's own
/// internal wrap-width computation never sees word-spacing-adjusted
/// widths, so it has nothing new to get wrong.
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
        Image { laid_style: ComputedStyle, background_image: Option<Arc<DecodedImage>>, image: Arc<DecodedImage>, advance_w: f32, img_w: f32, img_h: f32 },
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
                    let (width, height) = if let Some(entry) = cache.get(&id) {
                        let guard = entry.lock().unwrap();
                        (guard.0.layout_runs().next().map(|r| r.line_w).unwrap_or(0.0), guard.1)
                    } else if std::time::Instant::now() < deadline {
                        let synth = [InlineItem::Text(word.to_string(), *span)];
                        let (buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs) =
                            shape_spans(font_system, &synth, WORD_MEASURE_WIDTH, true, None);
                        let w = buffer.layout_runs().next().map(|r| r.line_w).unwrap_or(0.0);
                        cache.insert(id, Arc::new(Mutex::new((buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs))));
                        (w, h)
                    } else {
                        *incomplete = true;
                        (0.0, PLACEHOLDER_LINE_HEIGHT)
                    };
                    // `word-spacing`: real CSS adds extra space after
                    // each word-separator space character, so only a
                    // token that actually carries its own trailing space
                    // (a real boundary — not the very last word in the
                    // whole run, which never got one) gets any. Baked
                    // straight into `width` rather than a new field — see
                    // this function's own docs for why that's enough.
                    let extra = if word.ends_with(' ') { span.word_spacing } else { 0.0 };
                    measured.push(Measured::Word { id, width: width + extra, height });
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
                    image: Arc::clone(image),
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
                        content: LaidContent::Image(Arc::clone(image)),
                        positioned: Vec::new(),
                        fixed: false,
                        sticky: None,
                        scroll: None,
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
        scroll: None,
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
        scroll: None,
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
        scroll: None,
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

/// CSS Grid's own flexible-length distribution for one axis's track
/// list, sized against `available` — a real, independent-of-content,
/// definite extent (the grid container's own content *width*, for
/// `grid-template-columns`, always definite; see `measure_grid`'s own
/// docs for why the same isn't attempted for row tracks). Every
/// non-`fr` track takes its own fixed/`%` size first (`%` against
/// `available`, the same containing-block-width basis `width`/`margin`
/// already resolve a box's own `%` against); whatever's left over
/// (never negative — an overcommitted explicit grid simply doesn't
/// leave `fr` tracks any extra space, same "no free space to grow
/// into" outcome `resolve_flex_main_sizes` already gives an
/// overcommitted flex line) is divided among the `fr` tracks in
/// proportion to their own factor (CSS Grid §11.7's "flexible sizing"
/// step, scoped to a single pass — no real content-based minimum per
/// track, since every track here is already a plain `<length-
/// percentage>` or `fr`, never `auto`/`min-content`/`minmax()`; see
/// `GridTrackSpec`'s own docs).
fn resolve_grid_tracks(tracks: &[GridTrackSpec], available: f32, gap: f32, font_size: f32, root_font_size: f32) -> Vec<f32> {
    let n = tracks.len();
    if n == 0 {
        return Vec::new();
    }
    let fixed: Vec<Option<f32>> = tracks
        .iter()
        .map(|t| match *t {
            GridTrackSpec::Fr(_) => None,
            GridTrackSpec::Len(Length::Percent(p)) => Some(available * p),
            GridTrackSpec::Len(l) => Some(l.resolve(font_size, root_font_size)),
        })
        .collect();
    let sum_fixed: f32 = fixed.iter().filter_map(|f| *f).sum();
    let sum_fr: f32 = tracks
        .iter()
        .filter_map(|t| match *t {
            GridTrackSpec::Fr(f) => Some(f),
            _ => None,
        })
        .sum();
    let free = (available - sum_fixed - gap * (n.saturating_sub(1)) as f32).max(0.0);
    tracks
        .iter()
        .zip(&fixed)
        .map(|(t, f)| match (*t, f) {
            (GridTrackSpec::Fr(factor), _) if sum_fr > 0.0 => free * (factor / sum_fr),
            (GridTrackSpec::Fr(_), _) => 0.0,
            (_, Some(px)) => *px,
            _ => 0.0,
        })
        .collect()
}

/// A `display: grid` element's own box model (`box_model_entry`, same
/// as any other box) plus its content: a real but deliberately
/// narrower subset of CSS Grid Layout
/// (https://www.w3.org/TR/css-grid-1/) than the full spec — explicit
/// `grid-template-columns`/`-rows` track lists (fixed lengths, `%`,
/// and the real `fr`-unit flexible-length algorithm,
/// `resolve_grid_tracks`), `repeat(<integer>, ...)`, `gap`/`row-gap`/
/// `column-gap` (the same `ComputedStyle` fields `measure_flex` already
/// reads — never flexbox-specific), and implicit auto-placement in
/// document order, row by row (`grid-auto-flow: row`, the real spec's
/// own default and the only flow this implements).
///
/// **Deliberately cut, each documented precisely, not silently folded
/// into "done" (see `TODO.txt` for the authoritative list)**: no
/// explicit item placement (`grid-column`/`grid-row`/their `-start`/
/// `-end` longhands) — every item occupies exactly one cell, in
/// document order; this function's own auto-placement cursor decides
/// entirely. No `repeat(auto-fill/auto-fit, ...)`. No `grid-template-
/// areas`/named lines. No `justify-items`/`align-items`/`-self` within
/// a cell — an item always measures at its column's own full width
/// (the same "stretch" an ordinary table cell already gets) and sits
/// at its row's own top edge. No content-based track-sizing keywords
/// (`auto`/`min-content`/`max-content`/`minmax()`) — every column/row
/// track is a plain fixed/`%`/`fr` length (`GridTrackSpec`'s own
/// grammar) or, for an *implicit* row (one beyond `grid-template-
/// rows`'s own list, or when no `grid-template-rows` is declared at
/// all), sized from the max natural height of whatever items landed in
/// it — the same "row height follows its own content" rule a table's
/// own rows already use (`measure_table`).
///
/// **`fr` row tracks always fall back to the same auto/content sizing
/// an implicit row already gets, even when an *explicit* `grid-
/// template-rows` names one** (so does a `%` row track, when this
/// grid's own content height is indefinite — the overwhelmingly common
/// case for a `height: auto` grid container, i.e. almost every real
/// one). Real CSS Grid distributes a flexible row track's own share of
/// the grid container's *own* definite height — but that height is
/// itself almost never known up front here (same "no free space to
/// distribute" story flexbox's own `main_size` already tells for an
/// auto-height column container — see `measure_flex_column`'s own
/// docs), and *guessing* at a basis for the rare case it is known would
/// be inventing semantics rather than narrowing scope — so this
/// deliberately never attempts it, the same "percentage against an
/// indefinite basis computes as if it weren't declared" policy v49
/// already established for `height`'s own `%`, extended here to `fr`
/// too. A **fixed** (`px`/`em`/`rem`) `grid-template-rows` entry is
/// unaffected by any of this — it needs no basis at all and is sized
/// exactly as declared, content overflowing past it if too short (the
/// same `overflow: visible`-by-default consequence an explicit CSS
/// `height` already has on any other box since v42).
///
/// Column tracks have no such gap: a grid container's own content
/// *width* is always definite (`entry.content_w`, same value `measure_
/// flex_row`'s own main axis already relies on being known up front),
/// so `fr` is fully real there — deliberately prioritized over row
/// `fr` for exactly that reason: `grid-template-columns: 1fr 2fr 1fr`
/// is this feature's single most load-bearing primitive, and it works
/// correctly.
///
/// **No `grid-template-columns` declared at all** falls back to a
/// single full-width implicit column (every item on its own row) —
/// not real CSS's own implicit-column auto-creation for an empty
/// explicit grid (each item would instead get its own *column* in a
/// single row) — a deliberately simplified, documented substitute for
/// an edge case outside this item's own explicit scope (`grid-
/// template-columns`/`-rows` is the thing being implemented; "what
/// happens with neither declared at all" is a corner of the spec this
/// narrowing doesn't need to chase).
#[allow(clippy::too_many_arguments)]
fn measure_grid<'a>(
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

    // No `grid-template-columns` at all → a single full-width implicit
    // column (see this function's own docs for why, rather than real
    // CSS's own implicit-column-per-item fallback).
    const SINGLE_COLUMN: [GridTrackSpec; 1] = [GridTrackSpec::Fr(1.0)];
    let col_tracks = s.grid_template_columns.unwrap_or(&SINGLE_COLUMN);
    let num_cols = col_tracks.len().max(1);
    let col_widths = resolve_grid_tracks(col_tracks, entry.content_w, s.column_gap, s.font_size, s.root_font_size);
    let mut col_x = Vec::with_capacity(num_cols);
    let mut cx = entry.content_x;
    for w in &col_widths {
        col_x.push(cx);
        cx += w + s.column_gap;
    }

    // Auto-placement, document order, row by row (`grid-auto-flow:
    // row`, the only flow this implements — see this function's own
    // docs).
    let mut rows: Vec<Vec<&'a LayoutBox>> = Vec::new();
    for &item in &items {
        let need_new_row = match rows.last() {
            Some(r) => r.len() >= num_cols,
            None => true,
        };
        if need_new_row {
            rows.push(Vec::new());
        }
        rows.last_mut().expect("just ensured a row exists above").push(item);
    }

    let mut laid_children: Vec<Laid> = Vec::new();
    let mut cursor_y = entry.content_y;
    // The real bottom edge of the *last* row placed — tracked
    // separately from `cursor_y` (which always includes a trailing
    // `row_gap` after whichever row was placed most recently) so the
    // container's own final content height never counts a gap past its
    // last row. Deliberately *not* derived from the laid items' own
    // `y + h` extents the way `measure_flex` derives its own content
    // height (which works there because a line's cross size is always
    // exactly its tallest item) — an *explicit* `grid-template-rows`
    // entry can set a row taller than every item placed in it, and that
    // extra space has to come from `row_h` itself, not from any one
    // item's own measured position.
    let mut content_bottom = entry.content_y;
    for (r, row_items) in rows.iter().enumerate() {
        // An explicit row track's own fixed size always applies;
        // `%`/`fr` only when there's a real basis to resolve against —
        // see this function's own docs for why that's the deliberate
        // cut, not a guess.
        let explicit_h = s.grid_template_rows.and_then(|tracks| tracks.get(r)).and_then(|t| match *t {
            GridTrackSpec::Len(Length::Percent(p)) => containing_height.map(|h| h * p),
            GridTrackSpec::Len(l) => Some(l.resolve(s.font_size, s.root_font_size)),
            GridTrackSpec::Fr(_) => None,
        });
        let mut row_h = explicit_h.unwrap_or(0.0);
        let mut row_laid = Vec::with_capacity(row_items.len());
        for (c, item) in row_items.iter().enumerate() {
            let cw = col_widths.get(c).copied().unwrap_or(0.0);
            let lc = measure(
                item, font_system, col_x[c], cursor_y, cw, entry.child_containing_height, next_id, cache, deadline,
                incomplete, &mut own_pending, &no_floats, None,
            );
            if explicit_h.is_none() {
                row_h = row_h.max(lc.style.margin.top.px() + lc.style.margin.bottom.px() + lc.h);
            }
            row_laid.push(lc);
        }
        laid_children.extend(row_laid);
        content_bottom = cursor_y + row_h;
        cursor_y = content_bottom + s.row_gap;
    }

    patch_sticky_containing_block(&mut laid_children, entry.content_y, content_bottom);
    let content_h = content_bottom - entry.content_y;
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
        scroll: None,
    };
    apply_relative_offset(&mut laid, &s, width, containing_height);
    capture_sticky_info(&mut laid, &s, containing_height);
    laid
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

/// How far `lighten`/`darken` blend toward white/black — CSS2.1 §8.5.3
/// doesn't mandate one exact ratio for a bevel border's two-tone
/// shading, so this picks a single reasonable, clearly-visible value
/// and applies it consistently both directions (`Groove`/`Ridge`/
/// `Inset`/`Outset` — see `bevel_colors`).
const BEVEL_RATIO: f32 = 0.5;

/// Blends `c` `ratio` of the way toward white, per channel, keeping its
/// own alpha unchanged. No existing color-lightening helper was found
/// anywhere in this file or in cosmic-text's own `Color`/`Attrs` API
/// (checked before writing this) — `blend` above is a src-over alpha
/// compositor, a different operation entirely.
fn lighten(c: TextColor, ratio: f32) -> TextColor {
    let mix = |ch: u8| -> u8 { (ch as f32 + (255.0 - ch as f32) * ratio).round().clamp(0.0, 255.0) as u8 };
    TextColor::rgba(mix(c.r()), mix(c.g()), mix(c.b()), c.a())
}

/// Blends `c` `ratio` of the way toward black, per channel, keeping its
/// own alpha unchanged — the darkening counterpart to `lighten`.
fn darken(c: TextColor, ratio: f32) -> TextColor {
    let mix = |ch: u8| -> u8 { (ch as f32 * (1.0 - ratio)).round().clamp(0.0, 255.0) as u8 };
    TextColor::rgba(mix(c.r()), mix(c.g()), mix(c.b()), c.a())
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
        BorderStyle::Groove | BorderStyle::Ridge | BorderStyle::Inset | BorderStyle::Outset => {
            // Never actually reached: `paint_box_decoration` always
            // routes these 4 bevel styles through `paint_border`
            // instead, which calls this function once per already-
            // light/dark-shaded sub-stripe/edge with `BorderStyle::Solid`
            // — see its own docs. Kept as a defensive plain-solid
            // fallback (not folded into a `_ =>` catch-all) so adding a
            // future real `BorderStyle` variant still gets a compile
            // error here, not a silently-wrong paint.
            fill_rect(pixels, win_w, win_h, x, y, w, h, color, clip)
        }
    }
}

/// Lighter/darker variants of a bevel border's own declared color — see
/// `lighten`/`darken` and `BEVEL_RATIO`. `None` for every non-bevel
/// `BorderStyle` (nothing to compute; `paint_border` falls back to a
/// plain flat-colored edge for those).
fn bevel_colors(style: BorderStyle, base: TextColor) -> Option<(TextColor, TextColor)> {
    match style {
        BorderStyle::Groove | BorderStyle::Ridge | BorderStyle::Inset | BorderStyle::Outset => {
            Some((lighten(base, BEVEL_RATIO), darken(base, BEVEL_RATIO)))
        }
        _ => None,
    }
}

/// Paints all 4 edges of a border rect `(x, y, w, h)` with thickness
/// `bw`. Every style but the 4 bevel ones (`Groove`/`Ridge`/`Inset`/
/// `Outset`) just delegates to `paint_border_edge` 4 times with one flat
/// `color`, unchanged from before this function existed.
///
/// The bevel styles need per-edge (and, for `Groove`/`Ridge`, per-half-
/// edge) color, computed from `bevel_colors`: for every edge, "outer"
/// names the half/whole farther from this box's own content (the top of
/// the top edge, left of the left edge, bottom of the bottom edge, right
/// of the right edge) and "inner" the half closer to it.
/// - `Inset`/`Outset` paint each *whole* edge one color: `Inset`'s
///   top/left are uniformly dark and bottom/right uniformly light (the
///   box reads as pressed in, lit from the top-left); `Outset` is the
///   reverse.
/// - `Groove`/`Ridge` additionally split *each* edge's own thickness
///   into an outer and inner half (`half`, clamped to at least `0.5px`
///   so a 1px bevel border still shows two distinct rows/columns), so a
///   single edge itself shows a crease: `Groove`'s top/left edges are
///   outer-dark/inner-light and bottom/right outer-light/inner-dark
///   (the same "carved in" light direction as `Inset`, just applied
///   within each edge rather than across the whole edge); `Ridge` is
///   the reverse of `Groove`, edge-for-edge.
#[allow(clippy::too_many_arguments)]
fn paint_border(
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    bw: f32,
    style: BorderStyle,
    color: TextColor,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let Some((light, dark)) = bevel_colors(style, color) else {
        paint_border_edge(pixels, win_w, win_h, x, y, w, bw, style, color, clip);
        paint_border_edge(pixels, win_w, win_h, x, y + h - bw, w, bw, style, color, clip);
        paint_border_edge(pixels, win_w, win_h, x, y, bw, h, style, color, clip);
        paint_border_edge(pixels, win_w, win_h, x + w - bw, y, bw, h, style, color, clip);
        return;
    };
    match style {
        BorderStyle::Inset | BorderStyle::Outset => {
            let (top, left, bottom, right) = if style == BorderStyle::Inset {
                (dark, dark, light, light)
            } else {
                (light, light, dark, dark)
            };
            paint_border_edge(pixels, win_w, win_h, x, y, w, bw, BorderStyle::Solid, top, clip);
            paint_border_edge(pixels, win_w, win_h, x, y + h - bw, w, bw, BorderStyle::Solid, bottom, clip);
            paint_border_edge(pixels, win_w, win_h, x, y, bw, h, BorderStyle::Solid, left, clip);
            paint_border_edge(pixels, win_w, win_h, x + w - bw, y, bw, h, BorderStyle::Solid, right, clip);
        }
        BorderStyle::Groove | BorderStyle::Ridge => {
            // top/left outer/inner, bottom/right outer/inner.
            let (outer_tl, inner_tl, outer_br, inner_br) = if style == BorderStyle::Groove {
                (dark, light, light, dark)
            } else {
                (light, dark, dark, light)
            };
            let half = (bw / 2.0).max(0.5);
            // Top edge: outer = upper stripe, inner = lower stripe.
            paint_border_edge(pixels, win_w, win_h, x, y, w, half, BorderStyle::Solid, outer_tl, clip);
            paint_border_edge(pixels, win_w, win_h, x, y + half, w, bw - half, BorderStyle::Solid, inner_tl, clip);
            // Left edge: outer = left stripe, inner = right stripe.
            paint_border_edge(pixels, win_w, win_h, x, y, half, h, BorderStyle::Solid, outer_tl, clip);
            paint_border_edge(pixels, win_w, win_h, x + half, y, bw - half, h, BorderStyle::Solid, inner_tl, clip);
            // Bottom edge: inner = upper stripe, outer = lower stripe.
            paint_border_edge(pixels, win_w, win_h, x, y + h - bw, w, bw - half, BorderStyle::Solid, inner_br, clip);
            paint_border_edge(pixels, win_w, win_h, x, y + h - half, w, half, BorderStyle::Solid, outer_br, clip);
            // Right edge: inner = left stripe, outer = right stripe.
            paint_border_edge(pixels, win_w, win_h, x + w - bw, y, bw - half, h, BorderStyle::Solid, inner_br, clip);
            paint_border_edge(pixels, win_w, win_h, x + w - half, y, half, h, BorderStyle::Solid, outer_br, clip);
        }
        _ => unreachable!("bevel_colors only returns Some for the 4 bevel styles"),
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
    // (or, since v63, bevel — `Groove`/`Ridge`/`Inset`/`Outset`) ring
    // would need the same curved-path tracing a solid ring does, just
    // repeated per dash/dot/stripe/shaded-half — a real `TODO.txt` gap,
    // not attempted here — so any of those combined with `border-radius`
    // fall back to the existing square-corner rendering rather than a
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
                paint_border(pixels, win_w, win_h, x, y, w, h, bw, s.border_style, bc, clip);
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
/// `Solid` can paint a dashed/dotted/double/wavy line instead of a plain
/// solid stroke across the same extent — a hook the public `render_
/// decoration` doesn't offer. Mirrors its exact offset/thickness/color
/// formulas, so `DecorationStyle::Solid` still looks pixel-identical to
/// calling `render_decoration` directly; `Dashed`/`Dotted`/`Double`/`Wavy`
/// diverge (see `paint_decoration_line`'s own docs for each). Each span's
/// style is looked up by `decoration_styles[glyph.metadata]` off its first
/// glyph — the same per-span index every other per-span property
/// (backgrounds, valigns, ...) already uses, and applies uniformly to
/// whichever of underline/strikethrough/overline this span has active —
/// real CSS `text-decoration-style` is one value shared by every line a
/// `text-decoration-line` turns on, not set per-line.
///
/// Doubling (v60) no longer depends on cosmic-text's own native
/// `UnderlineStyle::Double` at all — this function never called cosmic's
/// own runtime renderer to begin with (only mirrored its formulas), so
/// there was never a real reason to special-case *only* underline the way
/// pre-v60 `parse_text_decoration` did. `UnderlineStyle::Double` is still
/// matched defensively (treated exactly like `Single` plus a forced
/// `Double` style) in case some other code path ever sets it independently
/// of this file's own `DecorationStyle`, but neither `parse_text_
/// decoration` nor the v60 longhands produce it anymore — `double` always
/// goes through `DecorationStyle::Double` now, which this function applies
/// the same way to strikethrough/overline too.
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
            UnderlineStyle::Single | UnderlineStyle::Double => {
                let color = td.underline_color_opt.or(span.color_opt).unwrap_or(default_color);
                let thickness = (deco.underline_metrics.thickness * font_size).max(1.0).ceil();
                let y = run.line_y - deco.underline_metrics.offset * font_size;
                // See this function's own docs — native `Double` forces
                // doubling even if `style` itself isn't `Double`.
                let style = if td.underline == UnderlineStyle::Double {
                    DecorationStyle::Double
                } else {
                    style
                };
                paint_decoration_line(renderer, x_start, y, width, thickness, color, style);
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

/// Paints one decoration line across `(x_start, y)..(x_start + width, y +
/// thickness)` — the whole span width in one `Renderer::rectangle` call
/// for `Solid`, several short ones spaced across the same extent for
/// `Dashed`/`Dotted`, two parallel `Solid` strokes (offset by `thickness`
/// plus an equal gap, same formula cosmic-text's own native double-
/// underline already used) for `Double`, and a piecewise sine-wave trace
/// for `Wavy` (v60) — see each arm's own comment. `x_start`/`y` are in the
/// same float coordinate space `render_decoration` itself uses (rounded to
/// pixels by `Renderer` implementors, same as any other rectangle it
/// draws).
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
        DecorationStyle::Double => {
            let gap = thickness;
            paint_decoration_line(renderer, x_start, y, width, thickness, color, DecorationStyle::Solid);
            paint_decoration_line(renderer, x_start, y + thickness + gap, width, thickness, color, DecorationStyle::Solid);
        }
        DecorationStyle::Wavy => {
            // `Renderer` offers only axis-aligned rectangles (confirmed by
            // reading cosmic-text's own `Renderer` trait — just
            // `rectangle`/`glyph`, no path/curve primitive at all), so a
            // real sine curve is approximated as a sequence of narrow
            // rectangles, each centered on the curve's own `sin` value at
            // that x — the same "break one long line into several small
            // `rectangle` calls" trick `Dashed`/`Dotted` already use, just
            // with a continuously-varying y instead of a gap pattern.
            // Amplitude/wavelength are both derived from `thickness` (the
            // stroke's own line weight) so the wave scales with font size
            // the same way dash/dot spacing already does, rather than
            // using fixed pixel constants that would look disproportionate
            // at very small or very large text.
            let amplitude = (thickness * 1.5).max(2.0);
            let wavelength = (thickness * 8.0).max(8.0);
            let step = (thickness * 0.5).max(1.0);
            let seg_h = thickness.max(1.0);
            let mut x = 0.0f32;
            while x < width {
                let seg_w = step.min(width - x);
                let center = x + seg_w / 2.0;
                let phase = (center / wavelength) * std::f32::consts::TAU;
                let y_off = amplitude * phase.sin();
                let seg_w_px = seg_w.max(1.0) as u32;
                if seg_w_px > 0 {
                    renderer.rectangle((x_start + x) as i32, (y + y_off) as i32, seg_w_px, seg_h as u32, color);
                }
                x += step;
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

/// `overflow: scroll`/`auto`'s own minimal scrollbar indicator (v62) —
/// a flat, simple painting primitive in the same spirit as this file's
/// other decorations (box-shadow/border): a translucent track plus a
/// translucent thumb, both hard-edged `fill_rect`s, no rounded ends, no
/// hover states. Deliberately an *overlay*, not a reserved-space
/// classic scrollbar — painted inside this box's own border box without
/// narrowing `content_w` for it at layout time at all (same "scrolling
/// never touches layout" reasoning `ScrollInfo`'s own docs already
/// give, extended to this box's own width too — an overlay-scrollbar OS
/// convention this matches, not just a shortcut). `Scroll` always
/// paints the track+thumb, even with nothing to scroll yet (thumb fills
/// the whole track); `Auto` only paints anything once `max_scroll() >
/// 0.0` — the one real, documented difference between the two keywords
/// (see `Overflow`'s own docs). Clipped by the *ambient* `clip`, not
/// `child_clip` — this is this box's own chrome, not descendant
/// content, same treatment `paint_box_decoration` already gets.
/// **Interactive drag-to-scroll on the thumb itself is explicitly out
/// of scope** (see TODO.txt) — wheel-scroll is this feature's own
/// interactive core; dragging the indicator is a documented cut.
#[allow(clippy::too_many_arguments)]
fn paint_scrollbar(
    info: &ScrollInfo,
    own_scroll: f32,
    show_when_not_overflowing: bool,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    box_x: f32,
    draw_y: f32,
    box_w: f32,
    box_h: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let max_scroll = info.max_scroll();
    if max_scroll <= 0.0 && !show_when_not_overflowing {
        return;
    }
    const TRACK_W: f32 = 8.0;
    const MIN_THUMB_H: f32 = 16.0;
    if box_w < TRACK_W || box_h <= 0.0 {
        return;
    }
    let track_x = box_x + box_w - TRACK_W;
    let track_y = draw_y;
    let track_h = box_h;
    let ratio = if info.content_h > 0.0 { (info.client_h / info.content_h).clamp(0.0, 1.0) } else { 1.0 };
    let thumb_h = (track_h * ratio).clamp(MIN_THUMB_H.min(track_h), track_h);
    let thumb_y = if max_scroll > 0.0 { track_y + (own_scroll / max_scroll) * (track_h - thumb_h) } else { track_y };
    fill_rect(pixels, win_w, win_h, track_x, track_y, TRACK_W, track_h, TextColor::rgba(0, 0, 0, 28), clip);
    fill_rect(pixels, win_w, win_h, track_x, thumb_y, TRACK_W, thumb_h, TextColor::rgba(0, 0, 0, 120), clip);
}

/// Paints vim mode's `:`-command bar — chrome drawn on top of the
/// already-laid-out page, outside the DOM/layout tree entirely, same
/// role `paint_scrollbar` already plays for the scrollbar overlay
/// (that's the existing precedent TODO.txt's own vim-mode plan points
/// at for "where chrome gets painted"). Bottom of the window, not top —
/// matches real vim's own command line, and Vimium/qutebrowser's bars,
/// rather than reading as a browser address bar (a placement TODO.txt
/// explicitly left open; settled here as bottom).
fn paint_command_bar(
    command_line: &TextEditBuffer,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
) {
    const BAR_H: f32 = 24.0;
    const PAD_X: f32 = 4.0;
    let bar_y = (win_h as f32 - BAR_H).max(0.0);
    fill_rect(pixels, win_w, win_h, 0.0, bar_y, win_w as f32, BAR_H, TextColor::rgba(24, 24, 24, 255), None);

    // The leading `:` is purely this bar's own display convention
    // (vim's own prompt character) — `command_line.text` itself never
    // stores it, so every real cursor byte offset into `text` needs a
    // `+1` shift to land on the matching glyph in `display`.
    let display = format!(":{}", command_line.text);
    let metrics = Metrics::new(14.0, BAR_H);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(Some((win_w as f32 - PAD_X).max(0.0)), Some(BAR_H));
    buffer.set_wrap(Wrap::None);
    let mut attrs = Attrs::new();
    attrs.color_opt = Some(TextColor::rgb(225, 225, 225));
    buffer.set_text(&display, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    let target = command_line.cursor + 1;
    let caret_x = buffer
        .layout_runs()
        .next()
        .map(|run| {
            run.glyphs
                .iter()
                .find(|g| g.start >= target)
                .map(|g| g.x)
                .unwrap_or_else(|| run.glyphs.last().map(|g| g.x + g.w).unwrap_or(0.0))
        })
        .unwrap_or(0.0);

    // One span (`Attrs::new()`'s own default `metadata: 0`), so every
    // per-span array `paint_buffer` expects just needs that one entry.
    let backgrounds = [None];
    let valigns = [VerticalAlign::default()];
    let hidden = [false];
    let decoration_styles = [DecorationStyle::default()];
    paint_buffer(
        &mut buffer, font_system, swash_cache, &backgrounds, &valigns, &hidden, &decoration_styles, pixels, win_w,
        win_h, PAD_X, bar_y + (BAR_H - 20.0) / 2.0, None,
    );

    fill_rect(pixels, win_w, win_h, PAD_X + caret_x, bar_y + 3.0, 1.5, BAR_H - 6.0, TextColor::rgb(225, 225, 225), None);
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
    // `v62`: passed straight through to every recursive `paint` call
    // below unmodified — a positioned/explicit-stacking entry is NOT
    // itself shifted by this box's own `overflow: scroll`/`auto` (see
    // TODO.txt: a documented, narrower cut from the general case, same
    // shape as several other out-of-flow-content limitations already
    // accepted elsewhere in this file) — but each entry may still
    // independently be (or contain) its own scrollable region, which is
    // why this still needs forwarding rather than dropping entirely.
    scroll_offsets: &HashMap<usize, f32>,
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
            paint(item, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, child_scroll, clip);
        } else {
            // An out-of-flow `positioned` entry — same `fixed` handling
            // `paint`'s own old positioned-list loop always used.
            paint(
                item, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h,
                if item.fixed { 0.0 } else { scroll_offset }, clip,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint(
    laid: &Laid,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    cache: &mut ShapeCache,
    // `overflow: scroll`/`auto`'s own *live* per-element scroll amounts
    // (v62), keyed by `ScrollInfo::id` — see that type's own docs for
    // why this lives outside `Laid` entirely. Read fresh here (via
    // `own_scroll`, just below), never written — `App`'s own event
    // handling is the only thing that ever updates it.
    scroll_offsets: &HashMap<usize, f32>,
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
    let child_clip = if matches!(s.overflow, Overflow::Hidden | Overflow::Scroll | Overflow::Auto) {
        let (cx, cy, cw, ch) = intersect_clip(laid.x, draw_y, laid.w, laid.h, clip);
        Some((cx, cy, cw, ch))
    } else {
        clip
    };

    // `overflow: scroll`/`auto`'s own live scroll amount (v62), looked
    // up fresh every call and re-clamped against `max_scroll()` at the
    // read site (self-healing if a relayout ever shrinks `content_h`
    // out from under a stale, larger stored offset, rather than
    // trusting the stored value is already in range). `0.0` for every
    // other box, including a `scroll`/`auto` box with nothing stored
    // for its id yet (the common "never scrolled" starting state).
    let own_scroll = laid
        .scroll
        .map_or(0.0, |info| scroll_offsets.get(&info.id).copied().unwrap_or(0.0).clamp(0.0, info.max_scroll()));
    // Only this box's own *descendants* see the extra shift — this
    // box's own painted position (`draw_y` above) deliberately still
    // uses the un-adjusted `scroll_offset`, since scrolling a box moves
    // its *content*, never the box itself (unlike `sticky`/`fixed`,
    // which shift the whole subtree including the box). Composes
    // naturally with nested scrolling and with a nested `position:
    // sticky`'s own `sticky_shift` basis (both read whatever ambient
    // `scroll_offset` is lexically in scope at their own recursion
    // depth) the same way `fixed` forcing it to `0.0` already composes
    // — no special-casing needed for either. **Deliberately not
    // threaded into the `paint_z_layer` calls below** (layers 2/6+7 —
    // this box's own out-of-flow/explicitly-stacked descendants): see
    // TODO.txt, a documented, narrower cut.
    let content_scroll_offset = scroll_offset + own_scroll;

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
        neg_positioned, &neg_stacking_children, font_system, swash_cache, cache, scroll_offsets, pixels, win_w,
        win_h, scroll_offset, child_clip,
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
                    Some(info) => {
                        content_scroll_offset - sticky_shift(info, child.h, content_scroll_offset, win_h as f32)
                    }
                    None => content_scroll_offset,
                };
                paint(
                    child, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, child_scroll,
                    child_clip,
                );
            }
        }
        LaidContent::Text(id) => {
            // Not yet shaped (still mid-way through a budgeted measure pass)
            // just means nothing to paint here yet — it'll appear once a
            // later frame finishes shaping it.
            if let Some(entry) = cache.get(id) {
                let mut guard = entry.lock().unwrap();
                let (buffer, _, backgrounds, valigns, hidden, _, decoration_styles, _) = &mut *guard;
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
                        paint(
                            img_laid, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h,
                            scroll_offset, child_clip,
                        );
                    }
                    MixedPositioned::Word { id, x, y, .. } => {
                        if let Some(entry) = cache.get(id) {
                            let mut guard = entry.lock().unwrap();
                            let (buffer, _, backgrounds, valigns, hidden, _, decoration_styles, _) = &mut *guard;
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

    // `overflow: scroll`/`auto`'s own minimal scrollbar indicator
    // (v62) — this box's own chrome, painted in screen space (`draw_y`,
    // never shifted by `own_scroll` — the indicator itself doesn't
    // scroll, only the content behind it does) and clipped by the
    // *ambient* `clip`, not `child_clip`, same treatment the background/
    // border decoration above gets. `Scroll` always shows it; `Auto`
    // only once content actually overflows (see `paint_scrollbar`'s own
    // docs for the real difference between the two keywords).
    if let Some(info) = &laid.scroll {
        paint_scrollbar(
            info, own_scroll, s.overflow == Overflow::Scroll, pixels, win_w, win_h, laid.x, draw_y, laid.w, laid.h,
            clip,
        );
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
        nonneg_positioned, &nonneg_stacking_children, font_system, swash_cache, cache, scroll_offsets, pixels, win_w,
        win_h, scroll_offset, child_clip,
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
/// `bottom`-offset math. `scroll_offsets` (v62) is the live per-element
/// `overflow: scroll`/`auto` state — see `ScrollInfo`'s own docs —
/// consulted only in the ordinary `Blocks`/`TableRow` loop below (the
/// "above" stacking/positioned-descendant loop deliberately isn't, same
/// documented cut `paint`'s own `paint_z_layer` calls just took).
///
/// Returns both the `CursorStyle` to show (what this function originally
/// returned, pre-link-navigation) and the `href` of whatever link, if
/// any, is under this exact point — link navigation (v65) reuses this
/// exact glyph-precision walk rather than a second, parallel one, since
/// it's answering the same underlying question ("which glyph is under
/// this point, and what span does it belong to") for a different event
/// (`WindowEvent::MouseInput`'s click handler instead of `CursorMoved`'s
/// hover). `href` has no box-level fallback the way `cursor` does
/// (`laid.style.cursor` always has *some* real cascaded value; there's
/// no equivalent "ambient box href" — only a per-glyph `SpanStyle::href`
/// exists at all, see its own docs), so every arm that falls back to
/// `laid.style.cursor` for the cursor half pairs it with a plain `None`
/// for the href half.
#[allow(clippy::too_many_arguments)]
fn hit_test_cursor(
    laid: &Laid,
    x: f32,
    y: f32,
    cache: &ShapeCache,
    scroll_offsets: &HashMap<usize, f32>,
    scroll_offset: f32,
    viewport_h: f32,
) -> (CursorStyle, Option<&'static str>) {
    if x < laid.x || x >= laid.x + laid.w || y < laid.y || y >= laid.y + laid.h {
        return (CursorStyle::Auto, None);
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
                return hit_test_cursor(item, x, y - dy, cache, scroll_offsets, scroll_offset, viewport_h);
            }
        } else {
            let (ix, iy, inner_scroll) = if item.fixed { (x, y - scroll_offset, 0.0) } else { (x, y, scroll_offset) };
            if ix >= item.x && ix < item.x + item.w && iy >= item.y && iy < item.y + item.h {
                return hit_test_cursor(item, ix, iy, cache, scroll_offsets, inner_scroll, viewport_h);
            }
        }
    }
    match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            // `overflow: scroll`/`auto` (v62): this box's own live
            // scroll amount shifts every *ordinary* descendant's own
            // effective position the same way `paint`'s own
            // `content_scroll_offset` does — see that computation's own
            // docs for the full derivation. Composes with a nested
            // sticky/scrollable descendant for free, same mechanism.
            let own_scroll = laid.scroll.map_or(0.0, |info| {
                scroll_offsets.get(&info.id).copied().unwrap_or(0.0).clamp(0.0, info.max_scroll())
            });
            let y = y + own_scroll;
            let scroll_offset = scroll_offset + own_scroll;
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
                        return hit_test_cursor(child, x, y - dy, cache, scroll_offsets, scroll_offset, viewport_h);
                    }
                    continue;
                }
                if child.x <= x && x < child.x + child.w && child.y <= y && y < child.y + child.h {
                    return hit_test_cursor(child, x, y, cache, scroll_offsets, scroll_offset, viewport_h);
                }
            }
            (laid.style.cursor, None)
        }
        LaidContent::Text(id) => {
            if let Some(entry) = cache.get(id) {
                let guard = entry.lock().unwrap();
                let (buffer, _, _, _, _, cursors, _, hrefs) = &*guard;
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
                            return (
                                cursors.get(glyph.metadata).copied().unwrap_or(laid.style.cursor),
                                hrefs.get(glyph.metadata).copied().flatten(),
                            );
                        }
                    }
                }
            }
            (laid.style.cursor, None)
        }
        // No finer-grained hit test within an image — just this box's
        // own (already-cascaded) cursor, same fallback the other arms
        // use once nothing more specific matches underneath. (An
        // `<img>` directly inside an `<a>` isn't itself clickable this
        // way — `href` only ever reaches a `SpanStyle`, never an image's
        // own box — see `SpanStyle::href`'s own docs; a deliberate,
        // narrow scope cut, same spirit as other "images don't carry
        // the per-span machinery text does" gaps already in TODO.txt.)
        LaidContent::Image(_) => (laid.style.cursor, None),
        LaidContent::MixedInline(positioned) => {
            for item in positioned {
                match item {
                    MixedPositioned::Image(img_laid) => {
                        if img_laid.x <= x && x < img_laid.x + img_laid.w && img_laid.y <= y && y < img_laid.y + img_laid.h {
                            return hit_test_cursor(img_laid, x, y, cache, scroll_offsets, scroll_offset, viewport_h);
                        }
                    }
                    MixedPositioned::Word { id, x: wx, y: wy, w, h } => {
                        if x < *wx || x >= *wx + *w || y < *wy || y >= *wy + *h {
                            continue;
                        }
                        if let Some(entry) = cache.get(id) {
                            let guard = entry.lock().unwrap();
                            let (buffer, _, _, _, _, cursors, _, hrefs) = &*guard;
                            let local_x = x - wx;
                            let local_y = y - wy;
                            for run in buffer.layout_runs() {
                                if local_y < run.line_top || local_y >= run.line_top + run.line_height {
                                    continue;
                                }
                                for glyph in run.glyphs {
                                    if local_x >= glyph.x && local_x < glyph.x + glyph.w {
                                        return (
                                            cursors.get(glyph.metadata).copied().unwrap_or(laid.style.cursor),
                                            hrefs.get(glyph.metadata).copied().flatten(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            (laid.style.cursor, None)
        }
    }
}

/// Finds the deepest `overflow: scroll`/`auto` box whose own border box
/// contains document-space point `(x, y)` — what `WindowEvent::
/// MouseWheel` (v62) needs to route a wheel tick to the right scroll
/// region instead of always the page's own top-level scroll. Same
/// coordinate conventions as `hit_test_cursor` (`y` already has the
/// live ambient scroll added in, same as the `CursorMoved` handler
/// already does for that function).
///
/// Deliberately narrower than `hit_test_cursor`'s own walk: only
/// ordinary `Blocks`/`TableRow` children are descended into (including
/// the same `position: sticky` translation `hit_test_cursor` needs,
/// since a sticky child with no explicit `z-index` still appears in
/// that ordinary list) — a `position: relative`/`sticky` child that
/// *does* establish its own explicit stacking context, or any
/// `position: absolute`/`fixed` descendant, is not walked into at all
/// (see TODO.txt: such content inside a scroll region doesn't actually
/// scroll with it either, the same documented cut `paint`'s own
/// `paint_z_layer` calls took, so there's nothing additional to find by
/// walking into it here).
///
/// Bubbles the innermost match up via `Option::or` — if a deeper call
/// finds no scrollable box at all, falls back to whichever scrollable
/// *ancestor* (if any) contains the point, exactly real "wheel bubbles
/// to the nearest scrollable ancestor" behavior; `None` all the way up
/// means "nothing scrollable under the cursor at all," the caller's own
/// cue to fall back to the page's own top-level scroll.
fn hit_test_scroll_region(
    laid: &Laid,
    x: f32,
    y: f32,
    scroll_offsets: &HashMap<usize, f32>,
    scroll_offset: f32,
    viewport_h: f32,
) -> Option<ScrollInfo> {
    if x < laid.x || x >= laid.x + laid.w || y < laid.y || y >= laid.y + laid.h {
        return None;
    }
    let self_info = laid.scroll;
    let (LaidContent::Blocks(children) | LaidContent::TableRow(children)) = &laid.content else {
        return self_info;
    };
    // Same `own_scroll` shift `hit_test_cursor`'s own `Blocks`/
    // `TableRow` arm applies, for the same reason — see that
    // computation's own docs.
    let own_scroll =
        self_info.map_or(0.0, |info| scroll_offsets.get(&info.id).copied().unwrap_or(0.0).clamp(0.0, info.max_scroll()));
    let child_y = y + own_scroll;
    let child_scroll_offset = scroll_offset + own_scroll;
    for child in children.iter().filter(|c| !is_explicit_stacking_child(c)) {
        if matches!(child.style.position, Position::Absolute | Position::Fixed) {
            // Never actually appears here (see `measure`'s own `Blocks`
            // arm — diverted to `own_pending`/`positioned` instead),
            // defensive only.
            continue;
        }
        if let Some(info) = &child.sticky {
            let dy = sticky_shift(info, child.h, child_scroll_offset, viewport_h);
            let (cx, cy) = (child.x, child.y + dy);
            if cx <= x && x < cx + child.w && cy <= child_y && child_y < cy + child.h {
                let sub =
                    hit_test_scroll_region(child, x, child_y - dy, scroll_offsets, child_scroll_offset, viewport_h);
                return sub.or(self_info);
            }
            continue;
        }
        if child.x <= x && x < child.x + child.w && child.y <= child_y && child_y < child.y + child.h {
            let sub = hit_test_scroll_region(child, x, child_y, scroll_offsets, child_scroll_offset, viewport_h);
            return sub.or(self_info);
        }
    }
    self_info
}

// ---------- link navigation (v65, re-platformed onto `wire` in v66) ----------
//
// scratchy still performs zero networking itself (see NOTES.md's
// "rendering, not networking" priority, in force since v1) — clicking a
// link builds a real HTTP/1.1 request (absolute-form request line, see
// `build_get_request_text`) and hands it to a `wire` daemon over a Unix
// socket (see `wire/NOTES.md` for that daemon's own design — a separate
// crate in this same repo, not a dependency of this one), then swaps
// whatever normalized response comes back into the live `App`, same as
// `run()`'s own `Input::File` case builds the *initial* document. v65
// originally talked to `snag --serve` directly (a GET-only, bundle-
// shaped line protocol); that's gone now, replaced outright per
// `scratchy/TODO.txt`'s own "TOP PRIORITY" planning entry, which also
// covers *why*: `snag`'s own job is bundling a page to one file, not
// being scratchy's live HTTP client, and the two were already showing
// seams (no way for the old protocol to express a form's method/body).
// Everything in this section is plain, `App`-free logic so it can be
// unit tested without a real window/event loop, matching this file's
// existing "headless function-level testing" convention (see NOTES.md's
// "Working notes for next session" — no other feature here constructs
// a real winit `EventLoop`/`Window` inside `#[cfg(test)]` either).

/// A real click (mouse-down then mouse-up close to the same point) vs. a
/// drag (mouse-down, move, mouse-up elsewhere) — scratchy has no text
/// selection yet, so there's nothing a drag would otherwise *do*, but
/// binding navigation to "mouse released over a link" without this
/// check would still misfire the moment a drag just happens to start
/// and end over one (and the instant text selection exists, it would
/// also fire navigation mid-selection-drag, which is squarely wrong).
/// `threshold` is in document pixels — `down`/`up` are both whatever
/// consistent coordinate space the caller uses for both (document space
/// in `App`'s own use, see `WindowEvent::MouseInput`'s handler).
fn is_click(down: (f32, f32), up: (f32, f32), threshold: f32) -> bool {
    let dx = up.0 - down.0;
    let dy = up.1 - down.1;
    dx * dx + dy * dy <= threshold * threshold
}

/// Half a cosmic-text glyph's worth of slack — small enough that an
/// actual drag (even a short one) is never mistaken for a click, large
/// enough that ordinary human hand tremor between a real mouse-down and
/// mouse-up over the same link doesn't miss.
const CLICK_DRAG_THRESHOLD_PX: f32 = 5.0;

/// How long each tick of `App::spawn_shaping_worker`'s own background
/// loop lets `measure_document` run before yielding and reporting
/// whatever it has so far — originally bounded `App::redraw`'s own
/// progressive-layout branch directly, back when shaping ran inline on
/// the single-threaded main loop (winit) and every input event sat
/// unhandled for the entire budget if it arrived mid-tick; measured
/// then (on a 151-paragraph page, 80ms budget -> ~6-8 paragraphs/tick,
/// 58 ticks, 7.1s total; 500ms -> ~30-34 paragraphs/tick, ~12 ticks,
/// 6.2s total; on a much larger, RTL page, 16ms -> 306 ticks/17.0s,
/// 80ms -> 90 ticks/13.0s, 150ms -> 53 ticks/12.6s — diminishing
/// returns past ~80ms there already, since individual complex-script
/// paragraphs can themselves take hundreds of ms, capping how much a
/// larger budget can batch together regardless of this value). Once
/// `measure_document` moved to its own background thread, the reason
/// to keep this *small* (bounding input latency) went away — but
/// ticking at all turned out to still matter, just for a different
/// reason: it's what lets a big page fill in progressively again
/// (`App::shape_cache`'s own docs) instead of showing nothing until
/// the entire page is shaped, which running to completion in one call
/// (tried first, see TODO.txt and this constant's own git history)
/// made painfully visible on a real, large page (confirmed: real
/// Wikipedia went from progressively filling in to a blank page for
/// most of a minute). Still `80ms` — nothing about *this* value's own
/// tradeoff changed, only who's bound by it (a background thread's own
/// progress-reporting cadence now, not main-thread input latency).
const PROGRESSIVE_LAYOUT_TICK_BUDGET: std::time::Duration = std::time::Duration::from_millis(80);

/// Resolves a clicked `<a href>` into an absolute URL against the
/// page's own `<base href>` — ordinary URL-join logic via the `url`
/// crate (already a dependency of `snag`, not previously one of
/// scratchy's own — see `Cargo.toml`), not hand-rolled relative-path
/// math. `Url::join`'s real behavior already does the right thing for
/// an `href` that's already absolute (resolves to itself, `base`
/// ignored, same as a real browser) and for one that isn't (resolves
/// against `base`) — so the only genuinely different case this
/// function itself has to handle is `base` being `None` at all (no
/// `<base href>` anywhere in the current page — see `find_base_url`'s
/// own docs for when that happens), where a relative `href` has nothing
/// to resolve against and reports a clear error rather than guessing.
fn resolve_href(base: Option<&Url>, href: &str) -> Result<Url, String> {
    match base {
        Some(base) => base.join(href).map_err(|e| format!("cannot resolve link {href:?}: {e}")),
        None => Url::parse(href)
            .map_err(|_| format!("cannot resolve link {href:?}: no <base href> on this page to resolve it against")),
    }
}

/// Finds the document's own `<base href="...">`, if any, and parses it
/// — the resolution basis `resolve_href` needs. `snag` (v9) always
/// emits a real one reflecting the page's actual URL, in both its
/// one-shot CLI and `--serve` modes (see `snag/NOTES.md`'s own v9
/// entry); a hand-written local test fixture with no `<base>` at all
/// just means a *relative* link on it can't be resolved — reported as
/// an inline error by `resolve_href`, not guessed at — while an
/// already-absolute one still resolves fine regardless.
fn find_base_url(document: &Handle) -> Option<Url> {
    find_base_href(document).and_then(|h| Url::parse(&h).ok())
}

/// The base URL `AppEvent::NavResult`'s handler resolves a freshly-
/// fetched page against (v68): the page's own `<base href>`, if it
/// declares one, else `fetched_url` — the real address `wire` fetched
/// it from. Mirrors a real browser's own "document base URL" rule
/// (defaults to the document's own address when there's no `<base>`
/// element at all) and the equivalent `snag` already gives a bundled
/// page via a synthesized `<base href>` (v9) — `wire`-navigated pages
/// never had that until now. Before this fix, a fetched page with no
/// `<base>` of its own left `base_url` at `None`, which broke *every*
/// relative link on that page, including a same-page fragment link
/// (found live: MediaWiki's own `#bodyContent` "jump to content" skip-
/// link, on an otherwise ordinary page with no explicit `<base>` tag —
/// the overwhelmingly common real-world case, not an edge case). Pulled
/// out as its own pure function specifically so this exact regression
/// has a permanent test, not just a one-line inline fix nobody can
/// re-verify later.
fn resolve_page_base_url(document: &Handle, fetched_url: Url) -> Url {
    find_base_url(document).unwrap_or(fetched_url)
}

/// Same recursive shape as `find_html_element` — walks the whole tree
/// (not just `<head>`) so a malformed/fragment document still finds a
/// `<base>` wherever it actually ended up, rather than silently falling
/// back to "none found" the moment it's outside where it belongs.
fn find_base_href(handle: &Handle) -> Option<String> {
    if let NodeData::Element { name, .. } = &handle.data {
        if name.local.as_ref() == "base" {
            if let Some(href) = element_attr(handle, "href") {
                return Some(href.to_string());
            }
        }
    }
    for child in handle.children.borrow().iter() {
        if let Some(found) = find_base_href(child) {
            return Some(found);
        }
    }
    None
}

/// Minimal HTML-escaping for `App::show_message`'s synthetic `<p>...
/// </p>` wrapper — `text` can carry a clicked `href` or an OS/`io::
/// Error` message, neither of which scratchy controls the contents of,
/// so a stray `<`/`&`/`>` needs to display literally rather than
/// corrupt the synthetic markup it's being spliced into.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Builds a real HTTP/1.1 request, in the standard proxy absolute-form
/// shape (RFC 7230 §5.3.2) `wire` itself parses (see `wire/src/main.rs`'s
/// own `parse_request`) — the request line carries the *whole* URL, not
/// just a path, since `wire` sits behind a bare Unix socket with no
/// DNS/TLS-SNI of its own to infer a destination from. GET-only for now
/// (no body, no method parameter) — form submission needs a real
/// `<button>`/focus/keyboard subsystem scratchy doesn't have yet (see
/// `scratchy/TODO.txt`'s own "Form elements — sequencing plan" item);
/// this is purely what a clicked `<a href>` needs.
fn build_get_request_text(url: &Url) -> String {
    let host = url.host_str().unwrap_or("");
    let host_header = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    format!("GET {url} HTTP/1.1\r\nHost: {host_header}\r\n\r\n")
}

/// Splits a raw response buffer into its `Content-Type` (if any) and
/// body — `wire` always frames its responses with a real `Content-
/// Length` and never `Transfer-Encoding: chunked` (see `wire/NOTES.md`),
/// so there's no chunked-response decoding to do here; this just finds
/// the blank-line header terminator and reads one header out of the
/// block before it, per the contract `scratchy/TODO.txt`'s own "TOP
/// PRIORITY" item settled: scratchy reads `Content-Type` and nothing
/// else — every other header (`Cache-Control`, `Server`, cookies, ...)
/// is deliberately left unparsed.
fn parse_wire_response(bytes: &[u8]) -> Result<(Option<String>, Vec<u8>), String> {
    let header_end = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "malformed response from wire: no header terminator found".to_string())?;
    let header_text = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "malformed response from wire: headers are not valid UTF-8".to_string())?;
    let content_type = header_text.split("\r\n").skip(1 /* status line */).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("content-type").then(|| value.trim().to_string())
    });
    let body = bytes[header_end + 4..].to_vec();
    Ok((content_type, body))
}

/// Wraps already-decoded image bytes (`wire` never forwards a
/// `Content-Encoding`, so these are the real final bytes) in a minimal
/// synthetic HTML document, reusing the `<img src="data:...">` decoding
/// path `decode_data_uri`/`decode_image_bytes` already implement for
/// `<img>` tags — rather than inventing a second, parallel "the whole
/// document is just an image" rendering mode. `mime` is expected to
/// already be one `dispatch_wire_response` has recognized (`image/png`/
/// `jpeg`/`gif`/`webp`).
fn wrap_image_as_html(mime: &str, bytes: &[u8]) -> Vec<u8> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(r#"<html><body><img src="data:{mime};base64,{encoded}"></body></html>"#).into_bytes()
}

/// The response-framing contract `scratchy/TODO.txt`'s own "TOP
/// PRIORITY" item settled on: `text/html` goes straight into the
/// existing HTML-parse path untouched; a real decoded image gets
/// wrapped (`wrap_image_as_html`) so it still reaches scratchy through
/// that same one path, never a second rendering mode; anything else is a
/// clear, named "can't render this" error — never a guess at forcing
/// unknown bytes through either path. The `;charset=...`/`; boundary=...`
/// parameter half of a real `Content-Type`, if any, is dropped before
/// matching — only the MIME type itself decides.
fn dispatch_wire_response(content_type: Option<String>, body: Vec<u8>) -> Result<Vec<u8>, String> {
    let content_type = content_type.unwrap_or_default();
    let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match mime.as_str() {
        "text/html" => Ok(body),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" => Ok(wrap_image_as_html(&mime, &body)),
        _ => Err(format!("can't render this content type: {mime:?}")),
    }
}

/// What connecting to `wire` and reading one full response back came out
/// to — the lowest-level outcome, shared by the top-level page fetch
/// (`start_navigation`) and every sub-resource fetch (v67's `fetch_
/// stylesheet_via_wire`/`fetch_image_via_wire`): either a real response
/// genuinely came back (status is deliberately not kept at all — see
/// `parse_wire_response`'s own docs on why only `Content-Type` ever
/// mattered), or nothing did. Kept distinct from a plain `Result` for
/// the same reason `start_navigation` itself needs it for the top-level
/// page: "never reached the daemon at all" is the one case with
/// something useful to do about it (the stdout-fallback print, for the
/// top-level page only — a sub-resource fetch has nothing worth printing
/// since the user never directly asked for *it*).
enum WireFetchRaw {
    Responded { content_type: Option<String>, body: Vec<u8> },
    Unreachable,
}

/// Connects to `path`, sends `request_text` verbatim, reads the whole
/// response until `wire` closes the connection (its own protocol's
/// "done" signal — no length prefix, no explicit end marker, same shape
/// `snag --serve`'s older line protocol already used), then parses just
/// far enough to hand the caller `Content-Type` and the raw body —
/// dispatching *what that means* (HTML? an image to wrap? CSS? an
/// unsupported type?) is each caller's own job, since the top-level page
/// and a sub-resource fetch each need a different answer to that
/// question (see `dispatch_wire_response`/`fetch_stylesheet_via_wire`/
/// `fetch_image_via_wire`). Any failure before a complete response was
/// read back (connect refused, write failed, connection reset mid-read)
/// collapses to `Unreachable` — from scratchy's point of view those are
/// all exactly the same outcome: no usable response came back, for
/// whatever reason.
fn fetch_raw_via_wire(path: &std::path::Path, request_text: &str) -> WireFetchRaw {
    let Ok(mut stream) = UnixStream::connect(path) else { return WireFetchRaw::Unreachable };
    if stream.write_all(request_text.as_bytes()).is_err() {
        return WireFetchRaw::Unreachable;
    }
    let mut buf = Vec::new();
    if stream.read_to_end(&mut buf).is_err() {
        return WireFetchRaw::Unreachable;
    }
    match parse_wire_response(&buf) {
        Ok((content_type, body)) => WireFetchRaw::Responded { content_type, body },
        Err(_) => WireFetchRaw::Unreachable,
    }
}

/// `start_navigation`'s own stdout fallback — per `scratchy/TODO.txt`'s
/// own design, printed verbatim as the literal HTTP/1.1 wire text
/// (request line, `Host`, blank line) specifically so it's directly
/// replayable by `nc`/a raw socket/a manually-run `wire` instance, with
/// zero translation, whenever no `--socket` was given at all or the
/// configured one couldn't be reached.
fn print_fallback_request(request_text: &str) {
    print!("{request_text}");
    let _ = io::stdout().flush();
}

// ---------- sub-resources: external stylesheets and images (v67) ----------
//
// The same `wire` daemon the top-level page fetch already talks to is
// reused for a page's own `<link rel=stylesheet href>`/non-`data:`
// `<img src>` references — scratchy still performs zero networking of
// its own beyond that one daemon. Nothing here is `snag`'s job to
// duplicate: `snag`'s own inlining only ever applies to a page bundled
// *ahead of time* via its CLI; a page reached by clicking a live link
// never goes through `snag` at all (see `scratchy/TODO.txt`'s own "TOP
// PRIORITY" item), so scratchy has to do this inlining-equivalent work
// itself for exactly that path.

/// Mime-checked low-level fetch shared by `fetch_stylesheet_via_wire`/
/// `fetch_image_via_wire`: runs `fetch_raw_via_wire`, then accepts the
/// body only if `Content-Type` (ignoring any `;charset=...`/`;
/// boundary=...` parameter, same as `dispatch_wire_response`) is one of
/// `accept`. Any other outcome — unreachable, no/mismatched `Content-
/// Type` — is a silent `None`, not an error: a single sub-resource
/// failing to fetch is never allowed to sink the whole page, the same
/// lenient "one bad image/stylesheet doesn't stop the rest" precedent
/// `snag`'s own sub-resource fetching already established (see `snag/
/// TODO.txt`'s "no retry/backoff" item for the same spirit applied to a
/// different failure mode).
fn fetch_sub_resource_via_wire(path: &std::path::Path, url: &Url, accept: &[&str]) -> Option<Vec<u8>> {
    let request_text = build_get_request_text(url);
    let WireFetchRaw::Responded { content_type, body } = fetch_raw_via_wire(path, &request_text) else { return None };
    let content_type = content_type.unwrap_or_default();
    let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if !accept.contains(&mime.as_str()) {
        return None;
    }
    Some(body)
}

/// Fetches one external stylesheet — `None` on any failure (unreachable,
/// not real CSS, or not valid UTF-8 — a real stylesheet declaring some
/// other encoding is a narrower gap than this function alone, not
/// attempted here; nothing has needed it yet).
fn fetch_stylesheet_via_wire(path: &std::path::Path, url: &Url) -> Option<String> {
    let body = fetch_sub_resource_via_wire(path, url, &["text/css"])?;
    String::from_utf8(body).ok()
}

/// Fetches and decodes one external image — `None` on any failure
/// (unreachable, not a supported image content type, or bytes `image`
/// itself can't decode). Reuses `decode_image_bytes` verbatim — the
/// exact same decoder `resolve_img` already uses for a `data:` URI
/// `src`, so a live-fetched image and an inlined one decode identically.
fn fetch_image_via_wire(path: &std::path::Path, url: &Url) -> Option<DecodedImage> {
    let body = fetch_sub_resource_via_wire(path, url, &["image/png", "image/jpeg", "image/gif", "image/webp"])?;
    decode_image_bytes(&body)
}

/// Every external stylesheet/image reference a page's own markup names,
/// exactly as written (unresolved — resolving against the page's base
/// URL is each caller's own job, done once both lists are complete, not
/// per-element here).
#[derive(Default)]
struct ExternalResources {
    /// Each `<link rel=stylesheet href="...">`'s own `href`, in document
    /// order.
    stylesheets: Vec<String>,
    /// Each non-`data:` `<img src="...">`'s own `src` — a `data:` URI is
    /// already fully inlined, so it's deliberately excluded here (it
    /// needs no fetch, and `resolve_img` never consults `image_cache`
    /// for one anyway).
    images: Vec<String>,
}

/// Walks the whole tree (not just `<head>`, same "a malformed document
/// might put it anywhere" latitude `find_base_href` already takes)
/// collecting every stylesheet `<link>`/non-`data:` `<img>` reference.
/// `rel` is matched the same space-separated, case-insensitive way a
/// real `rel` attribute's token list always is (a `<link>` can legally
/// carry more than one keyword, e.g. `rel="stylesheet alternate"`).
fn find_external_resources(handle: &Handle, out: &mut ExternalResources) {
    if let NodeData::Element { name, .. } = &handle.data {
        match name.local.as_ref() {
            "link" => {
                let is_stylesheet = element_attr(handle, "rel")
                    .map(|r| r.split_ascii_whitespace().any(|t| t.eq_ignore_ascii_case("stylesheet")))
                    .unwrap_or(false);
                if is_stylesheet
                    && let Some(href) = element_attr(handle, "href")
                {
                    out.stylesheets.push(href.to_string());
                }
            }
            "img" => {
                if let Some(src) = element_attr(handle, "src")
                    && !src.starts_with("data:")
                {
                    out.images.push(src.to_string());
                }
            }
            _ => {}
        }
    }
    for child in handle.children.borrow().iter() {
        find_external_resources(child, out);
    }
}

/// The one function that turns a freshly-fetched page's raw HTML bytes
/// into both halves of v67's sub-resource support: every external
/// stylesheet, fetched synchronously (right here, on `start_navigation`'s
/// own background thread) and concatenated into the returned `String`
/// (`App::extra_css` — CSS has to be fully in hand *before* the page's
/// very first layout, so this can't be deferred the way images can);
/// and every non-`data:` image, each fired off as its own independent
/// background thread (`thread::spawn`, mirroring `wire`'s own explicit
/// "concurrent *separate* connections are expected" design) reporting
/// back through `AppEvent::ImageFetched` whenever it finishes — this
/// function itself returns immediately after *starting* those, long
/// before any of them resolve.
///
/// Parses `html` into its own throwaway `Handle` purely to walk it with
/// `find_external_resources` and `find_base_url` — that DOM is never
/// touched again once this function returns and, critically, never
/// crosses a thread boundary itself (a `Handle` is `Rc`-based, so it
/// can't: `Rc<T>` is `!Send`). Only plain owned `String`s (a stylesheet's
/// fetched text, an image's own raw `src` key) and a plain `DecodedImage`
/// struct (no `Rc` of its own — see `AppEvent::ImageFetched`'s own docs)
/// ever actually travel back across the channel.
fn process_external_resources(
    proxy: &EventLoopProxy<AppEvent>,
    path: &std::path::Path,
    page_url: &Url,
    html: &[u8],
    generation: u64,
) -> String {
    let Ok(dom) = new_parser().read_from(&mut &html[..]) else { return String::new() };
    // The fetched page's own `<base href>`, if it declares one, wins —
    // same priority `resolve_href`'s every other caller already gives
    // an explicit `<base>` over the document's own URL.
    let base = find_base_url(&dom.document).unwrap_or_else(|| page_url.clone());
    let mut resources = ExternalResources::default();
    find_external_resources(&dom.document, &mut resources);

    let mut extra_css = String::new();
    for href in resources.stylesheets {
        let Ok(url) = resolve_href(Some(&base), &href) else { continue };
        if let Some(css) = fetch_stylesheet_via_wire(path, &url) {
            extra_css.push_str(&css);
            extra_css.push('\n');
        }
    }

    // Deduplicated — a real page commonly repeats the exact same icon/
    // logo URL many times; one fetch (and one `image_cache` entry, keyed
    // by this shared raw `src` string) already covers every occurrence.
    let mut seen = HashSet::new();
    for src in resources.images {
        if !seen.insert(src.clone()) {
            continue;
        }
        let Ok(url) = resolve_href(Some(&base), &src) else { continue };
        let path = path.to_path_buf();
        let proxy = proxy.clone();
        thread::spawn(move || {
            let image = fetch_image_via_wire(&path, &url);
            let _ = proxy.send_event(AppEvent::ImageFetched(generation, src, image));
        });
    }

    extra_css
}

// ---------- windowing ----------

/// A fully-fetched, ready-to-display page — the `Ok` payload of
/// `AppEvent::NavResult` (v67). `html` is ready-to-parse bytes (same
/// bytes `Input::File`'s own `fs::read_to_string` would have read from a
/// saved page — already dispatched through `dispatch_wire_response`, so
/// a fetched image has already been wrapped into a synthetic HTML
/// document by this point too); `extra_css` is every external
/// stylesheet `process_external_resources` found and fetched for this
/// page, concatenated — see `App::extra_css`'s own docs. `url` is the
/// real address this page was fetched from — needed as `NavResult`'s
/// own fallback base URL (v68) for a fetched page that declares no
/// `<base href>` of its own, the overwhelmingly common real-world case
/// (a real browser's own "document base URL" already defaults to
/// exactly this, same as `snag`'s own bundled pages already get via a
/// synthesized `<base href>` — see `find_base_url`'s own docs). All
/// three are plain owned data (`Vec<u8>`/`String`/`Url`, all `Send`),
/// deliberately never a parsed `Handle` (an `Rc`-based type — `!Send` —
/// see `process_external_resources`'s own docs for why its own DOM walk
/// never crosses this same thread boundary).
struct FetchedPage {
    html: Vec<u8>,
    extra_css: String,
    url: Url,
}

enum AppEvent {
    Chunk(Vec<u8>),
    Eof,
    /// The result of a background navigation fetch (`App::start_
    /// navigation`'s spawned thread) — `Err` is a human-readable message
    /// to show in place of the page (no `--socket` configured, the
    /// daemon unreachable, or a response `wire` sent back in a content
    /// type scratchy can't render — see `fetch_raw_via_wire`'s own docs
    /// for exactly which). The `u64` is `App::nav_generation` at the
    /// moment this navigation started — compared against the *current*
    /// generation when this event arrives so a stale result from a
    /// since-superseded navigation (the user clicked a second link
    /// before the first one's fetch finished) is silently dropped
    /// instead of clobbering newer, already-displayed content. No
    /// back/forward history is built on top of this (see TODO.txt's own
    /// link-navigation item) — this is just the one hook a future
    /// history stack would need to land next to.
    NavResult(u64, Result<FetchedPage, String>),
    /// One non-`data:` `<img src>` reference's own fetch finishing (v67)
    /// — `String` is the exact, unresolved `src` attribute text
    /// `process_external_resources` found it under (see `App::image_
    /// cache`'s own docs for why that's the right cache key), `None` for
    /// any failure (unreachable daemon, non-image content type, corrupt/
    /// unsupported image data — `fetch_image_via_wire`'s own docs). Each
    /// arrival inserts (or, on failure, simply doesn't) one cache entry
    /// and marks layout dirty — images materialize one at a time as
    /// their own fetches land, the page never blocks on all of them the
    /// way `extra_css` necessarily does. Tagged with the same `nav_
    /// generation` convention `NavResult` already uses, for the same
    /// reason: a since-superseded page's late-arriving image must never
    /// populate the *new* page's cache.
    ImageFetched(u64, String, Option<DecodedImage>),
    /// One tick of the persistent background shaping worker's own
    /// progress (`App::spawn_shaping_worker`) — the "move page shaping
    /// off the main thread" plan in TODO.txt, landed, fires *once per
    /// tick*, not once per page: a big page arrives as a short burst of
    /// these, each one a little further along, which is what lets it
    /// fill in progressively again on the main thread despite shaping
    /// happening on a different thread entirely (see `spawn_shaping_
    /// worker`'s own docs for why that took a second attempt). Tagged
    /// with `App::layout_generation` at the moment the request that
    /// produced this was queued, same `NavResult`/`ImageFetched`
    /// convention: compared against the *current* generation on
    /// arrival, a mismatch (a resize, a new chunk, a newly-arrived
    /// image, a navigation — anything that bumped `layout_generation`
    /// again before this landed) means the reshape this was computing
    /// is since-superseded, dropped silently instead of clobbering
    /// newer content. The `ShapeCache` crosses back too, growing a
    /// little with each successive delivery for the same generation —
    /// `paint` needs the *matching* cache for whatever ids this exact
    /// `Laid` references, so `App::shape_cache` is replaced wholesale
    /// on arrival, never merged with whatever was there before (always
    /// safe here: each delivery's cache is a strict superset of the
    /// last one's, same tree, same worker, nothing removed between
    /// ticks). No `incomplete` flag, unlike `measure`'s own parameter
    /// of that name — nothing downstream needs to tell a mid-page
    /// delivery apart from the final one; both just replace `cached`/
    /// `shape_cache` the same way. `Laid` itself is boxed — otherwise
    /// this variant alone (`Laid` plus a whole `ShapeCache`) would
    /// dwarf every other `AppEvent` variant's size and every one of
    /// them pays for that on the stack (`clippy::large_enum_variant`);
    /// `self.cached: Option<Laid>` on the receiving end stays unboxed,
    /// same as before this plan.
    ShapeResult(u64, Box<Laid>, ShapeCache),
    /// `SIGUSR1`'s own background thread (`run()`) waking the event
    /// loop — the out-of-band bootstrap the vim-mode plan in TODO.txt
    /// settled on for turning vim mode back *on* once it's off: with
    /// `vim_enabled == false`, scratchy intercepts zero keys, so
    /// nothing in-band (no keystroke, however it's bound) could ever
    /// type `:set vim` again. A signal sent to the process sidesteps
    /// that entirely, immune to both a WM claiming a reserved chord and
    /// unusual keyboard-firmware Unicode-input quirks (the two reasons
    /// a dedicated keyboard chord was considered and dropped — see
    /// TODO.txt). Toggles rather than only turning vim mode on, so the
    /// same external signal also works as an off switch.
    ToggleVimMode,
}

/// `Normal` resolves bare keys into an `Action` via `resolve_binding`;
/// `Command` is the `:`-entered command line itself, editing `App::
/// command_line` directly rather than going through any binding at
/// all. Meaningless while `App::vim_enabled` is `false` — vim mode off
/// always means `Normal` with every key ignored by `resolve_binding`
/// (`mode` is checked there too, not just by the caller), not a third
/// state of its own (see `App::handle_key_event`'s own docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VimMode {
    Normal,
    Command,
}

/// The effect a resolved Normal-mode binding actually has — kept as its
/// own small enum, not performed inline inside `resolve_binding` itself,
/// so that function can stay pure (no `&mut App`) and unit-testable on
/// its own, same reasoning `resolve_href`'s own split from `start_
/// navigation` already follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    ScrollDown,
    ScrollUp,
    HalfPageDown,
    HalfPageUp,
    EnterCommandMode,
}

/// The one chokepoint a real key event gets resolved into an `Action`
/// through — hardcoded `match` arms today (dwm `config.h` style,
/// explicitly fine per the user's own call in TODO.txt), but the exact
/// spot a future JS-scriptable `bind()` config layer would call into or
/// override instead of a redesign. `mode` gates this to `VimMode::
/// Normal` even though every caller today only ever calls this from
/// that branch already — kept as a real parameter (not dropped) so a
/// future third mode (link hints — see TODO.txt's own "explicitly out
/// of scope for now" note) extends this one `match`'s arm list instead
/// of requiring a new chokepoint. `mods` is checked, not `logical_key`
/// alone, only for the one binding that needs it (`Ctrl`+`d`/`u`) —
/// `winit`'s own `logical_key` is already shift-aware (`Key::
/// Character(":")` for `Shift`+`;`), so bare `j`/`k`/`:` need no
/// modifier check at all.
fn resolve_binding(key: &Key, mods: ModifiersState, mode: VimMode) -> Option<Action> {
    if mode != VimMode::Normal {
        return None;
    }
    match key {
        Key::Character(s) if s.as_str() == "j" => Some(Action::ScrollDown),
        Key::Character(s) if s.as_str() == "k" => Some(Action::ScrollUp),
        Key::Character(s) if s.as_str() == "d" && mods.control_key() => Some(Action::HalfPageDown),
        Key::Character(s) if s.as_str() == "u" && mods.control_key() => Some(Action::HalfPageUp),
        Key::Character(s) if s.as_str() == ":" => Some(Action::EnterCommandMode),
        _ => None,
    }
}

/// What a key event should insert into a text-editing buffer, if
/// anything at all — the one chokepoint `Key::Named` vs. `Key::
/// Character` gets resolved into actual insertable text through,
/// instead of leaving every text-editing call site (today: the command
/// line; later: a focused `<input>`, per `TextEditBuffer`'s own docs)
/// to independently remember the one surprising case: `winit` reports
/// the space bar as `Key::Named(NamedKey::Space)`, *not* `Key::
/// Character(" ")` — confirmed the hard way, post-merge (`:visit
/// https://...` landed as `:visithttp...`, the space silently dropped
/// by a bare `_ => {}` catch-all that only matched `Key::Character`).
/// Kept pure and separate from `resolve_binding` (same reasoning that
/// one's own docs give for staying a plain `key -> Option<Action>` map)
/// so this exact class of bug — a `Key::Named` variant quietly falling
/// through a wildcard arm — has a real unit test instead of only
/// surfacing by hand-testing every key on a real keyboard.
fn key_insertion_text(key: &Key) -> Option<&str> {
    match key {
        Key::Named(NamedKey::Space) => Some(" "),
        Key::Character(s) => Some(s.as_str()),
        _ => None,
    }
}

/// A single-line text-editing buffer — insert, backspace, cursor move,
/// nothing else — written standalone (knows nothing about `App`,
/// chrome, or the page it might eventually sit over) on purpose: the
/// Form elements plan's own step 4 (TODO.txt) needs the *exact* same
/// primitive for a focused `<input type=text>`'s own editing later, and
/// the command line below is meant to be its first caller, not its
/// only one. `cursor` is always a byte offset on a `char` boundary
/// within `text` — every method below moves it by exactly one `char`'s
/// `len_utf8()`, never a raw byte, so it can't land mid-codepoint.
#[derive(Default)]
struct TextEditBuffer {
    text: String,
    cursor: usize,
}

impl TextEditBuffer {
    fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn backspace(&mut self) {
        let Some(prev) = self.text[..self.cursor].chars().next_back() else { return };
        self.cursor -= prev.len_utf8();
        self.text.remove(self.cursor);
    }

    fn move_left(&mut self) {
        if let Some(prev) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= prev.len_utf8();
        }
    }

    fn move_right(&mut self) {
        if let Some(next) = self.text[self.cursor..].chars().next() {
            self.cursor += next.len_utf8();
        }
    }
}

/// One rebuild's worth of work for `App::spawn_shaping_worker`'s own
/// persistent background thread — everything `measure_document` needs
/// that `App` itself held inline before this plan started. `tree` is
/// `LayoutBox`, built synchronously on the main thread (the DOM walk +
/// cascade match can't leave it — see `App::redraw`'s own comment)
/// just before this is queued.
struct ShapingRequest {
    tree: LayoutBox,
    width: f32,
    height: f32,
    generation: u64,
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
    /// Shaped paragraphs matching whatever `Laid` tree is currently in
    /// `cached`, keyed by the stable id `measure` assigned them when it
    /// was built — `paint` looks a `LaidContent::Text(id)` up in here.
    /// Replaced wholesale, never merged, every time an `AppEvent::
    /// ShapeResult` lands from the background shaping worker (see that
    /// type's own docs) — not touched in between arrivals, so `cached`/
    /// `shape_cache` stay a consistent pair for `paint` to read at any
    /// point. Since the worker reports progress one tick at a time
    /// (`App::spawn_shaping_worker`), this normally updates every
    /// `PROGRESSIVE_LAYOUT_TICK_BUDGET` or so while a page is still
    /// shaping, each delivery a little more complete than the last —
    /// what makes a big page fill in progressively again despite
    /// shaping happening off the main thread entirely.
    shape_cache: ShapeCache,
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
    /// The OS cursor state last actually set, so `CursorMoved` only calls
    /// `window.set_cursor`/`set_cursor_visible` when the hit-tested style
    /// actually changes, not on every mouse-move tick. `Some(icon)` means
    /// visible showing that icon; `None` means hidden (`cursor: none`,
    /// the one `CursorStyle` with no `CursorIcon` of its own — see
    /// `CursorStyle::to_winit`'s own docs).
    last_cursor: Option<CursorIcon>,
    /// `overflow: scroll`/`auto`'s own live per-element scroll amounts
    /// (v62), keyed by `ScrollInfo::id` — the one piece of this
    /// feature's state that genuinely persists across frames and is
    /// deliberately *not* part of `Laid` (see `ScrollInfo`'s own docs).
    /// Cleared alongside `shape_cache` whenever the layout tree is
    /// rebuilt (`redraw`'s own `need_rebuild` branch) — stale ids from a
    /// since-replaced tree are harmless either way (just unused map
    /// entries), but clearing keeps this from growing unboundedly across
    /// many stdin chunks in streaming mode.
    scroll_offsets: HashMap<usize, f32>,
    /// The last `CursorMoved` position, window/screen space — `winit`'s
    /// own `MouseWheel` event carries no position at all, so routing a
    /// wheel tick to the right scroll region (`hit_test_scroll_region`)
    /// needs this remembered from whatever `CursorMoved` last reported.
    /// `None` until the first `CursorMoved` ever arrives (e.g. a wheel
    /// tick before the mouse has moved over the window at all) — falls
    /// back to the page's own top-level scroll in that case, same as
    /// "cursor not over any scroll region."
    last_mouse_pos: Option<(f32, f32)>,
    /// Document-space position at the most recent `MouseInput`
    /// `Pressed(Left)`, if any — compared against `last_mouse_pos` at
    /// the matching `Released(Left)` to tell a real click (navigates)
    /// apart from a drag (scratchy has no text selection yet, but a
    /// careless same-event binding would still misfire once it exists —
    /// see `is_click`'s own docs). Cleared back to `None` on release
    /// regardless of outcome.
    mouse_down_pos: Option<(f32, f32)>,
    /// Lets a background navigation thread (`start_navigation`) get its
    /// result back onto this (the event-loop) thread — the exact same
    /// `thread::spawn` + `event_loop.create_proxy().send_event(..)`
    /// pattern `run()`'s own `Input::Stdin` case already uses for
    /// reading stdin off the main thread, reused here for the socket
    /// I/O a real page's fetch (through `wire`, formerly `snag --serve`)
    /// can take 10-30+ seconds to finish (confirmed directly against
    /// real Wikipedia in an earlier session). Created once in `run()`
    /// (an `ActiveEventLoop`, unlike
    /// the top-level `EventLoop`, has no `create_proxy` of its own —
    /// confirmed by reading winit's vendored source — so this has to be
    /// threaded in at construction time instead of fetched lazily from
    /// `resumed`'s own `&ActiveEventLoop`).
    proxy: EventLoopProxy<AppEvent>,
    /// The persistent background shaping worker's inbox — see `spawn_
    /// shaping_worker`'s own docs. `redraw`'s rebuild branch sends a
    /// fresh `ShapingRequest` here (`queue_shaping_request`) instead of
    /// spawning a new thread (and paying for a new `FontSystem`) every
    /// time a page needs reshaping.
    shaping_requests: mpsc::Sender<ShapingRequest>,
    /// The current page's own `<base href>` (recomputed from the real
    /// document every time it's replaced — initial load included, not
    /// just navigation — see `replace_document`), used to resolve a
    /// clicked `<a href>` into an absolute URL (`resolve_href`). `None`
    /// means a relative `href` can't be resolved at all (no base to
    /// join against) — reported as an inline error rather than guessed
    /// at; an already-absolute `href` still resolves fine either way,
    /// same as a real browser.
    base_url: Option<Url>,
    /// Incremented every time `start_navigation` fires — tags the
    /// `AppEvent::NavResult` its spawned thread will eventually send
    /// back, so a result from a navigation the user has since moved on
    /// from (clicked a second link before the first one's fetch
    /// finished) is recognized as stale and dropped instead of
    /// clobbering newer content. See `AppEvent::NavResult`'s own docs.
    nav_generation: u64,
    /// Bumped every time this *page*'s own layout would need reshaping
    /// from scratch — every site that sets `layout_dirty = true` today
    /// (a new/changed `document`: `replace_document`, `AppEvent::Chunk`/
    /// `Eof`, `AppEvent::ImageFetched`), plus the one place `cached_
    /// width` itself changes (`redraw`'s rebuild commit, the purely-
    /// resize-driven case that never touches `layout_dirty` at all).
    /// Unlike `nav_generation` — which answers "which navigated-to
    /// *page*" — this answers "which *reshape* of the current page";
    /// see TODO.txt's "Move page shaping off the main thread" plan for
    /// why that's a different question, not yet reusable as-is. Not
    /// read anywhere yet — this field only exists so a future
    /// background-shaping thread has something correct to tag its
    /// spawn with and check on arrival; landed on its own, ahead of
    /// that thread, so the bookkeeping itself is proven not to regress
    /// anything before anything depends on it.
    layout_generation: u64,
    /// The `--socket <path>` CLI flag, if given — where a `wire` daemon
    /// is expected to be listening. `None` means the user never opted
    /// in at all; `start_navigation` treats that identically to "gave a
    /// path but nothing's listening there" (see its own docs), just
    /// without even attempting a connection first.
    socket_path: Option<PathBuf>,
    /// CSS fetched via `wire` from the current page's own `<link
    /// rel=stylesheet href>` references (v67) — concatenated once, by
    /// `start_navigation`'s own background thread (`process_external_
    /// resources`), *before* `AppEvent::NavResult` is even sent, so it's
    /// fully present for the very first `build_layout_tree_with_
    /// resources` call rather than arriving after an initial unstyled
    /// flash. Set by `replace_document`, same lifetime as `document`
    /// itself — cleared (to empty) on every navigation/`show_message`,
    /// never merged across pages.
    extra_css: String,
    /// Already-decoded images fetched via `wire` for the current page's
    /// own non-`data:` `<img src>` references (v67), keyed by the exact,
    /// unresolved `src` attribute text — see `resolve_img`'s own docs
    /// for why that raw-string key is safe here. Unlike `extra_css`,
    /// populated gradually, *after* the page is already showing: each
    /// `AppEvent::ImageFetched` that arrives inserts one entry and marks
    /// layout dirty, so images materialize one at a time as their own
    /// fetches finish rather than blocking the page on all of them.
    /// Cleared on every navigation/`show_message`, same as `extra_css`.
    image_cache: HashMap<String, Arc<DecodedImage>>,
    /// Whether vim-mode key interception is on at all — `:set vim`/
    /// `:set novim`'s own stored state (plus `AppEvent::ToggleVimMode`'s
    /// own flip). `false` means `resolve_binding` is never even reached
    /// from `handle_key_event` for an ordinary keystroke: the explicit,
    /// stated use case (TODO.txt) is turning this fully off to play a
    /// browser-rendered game that wants raw keys, so "off" has to mean
    /// zero claimed keys, not "off except for the keys scratchy still
    /// wants." Starts `true` — real bare-key Normal-mode bindings work
    /// immediately, no setup step required, same as real vim's own
    /// default.
    vim_enabled: bool,
    /// `Normal` vs. the `:`-entered command line itself — see `VimMode`'s
    /// own docs. Only ever meaningfully `Command` while `vim_enabled` is
    /// `true` (there is no key left to reach `Action::EnterCommandMode`
    /// through otherwise), but not reset to `Normal` on `vim_enabled`
    /// flipping false — `dispatch_command`'s own `:set novim` handler
    /// already runs from inside a `Command`-mode keystroke and resets
    /// this itself right after, same as every other command-line
    /// submission does.
    vim_mode: VimMode,
    /// The `:`-command line's own live text, editable only while `vim_
    /// mode` is `Command` — see `TextEditBuffer`'s own docs for why this
    /// is a standalone type rather than a plain `String` field.
    command_line: TextEditBuffer,
    /// Live modifier-key state, kept current by `WindowEvent::
    /// ModifiersChanged` — `winit`'s own `KeyEvent` carries no
    /// modifiers of its own (`logical_key` is already shift-adjusted,
    /// but `Ctrl` specifically is excluded from that — see `KeyEvent::
    /// logical_key`'s own docs), so this is the only way `resolve_
    /// binding` can tell a bare `d`/`u` apart from `Ctrl`+`d`/`u`.
    modifiers: ModifiersState,
}

impl App {
    fn new_static(document: Handle, proxy: EventLoopProxy<AppEvent>, socket_path: Option<PathBuf>) -> Self {
        let base_url = find_base_url(&document);
        let shaping_requests = Self::spawn_shaping_worker(proxy.clone());
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
            shape_cache: ShapeCache::new(),
            surface_size: (0, 0),
            pending_width: None,
            last_cursor: Some(CursorIcon::Default),
            scroll_offsets: HashMap::new(),
            last_mouse_pos: None,
            mouse_down_pos: None,
            proxy,
            shaping_requests,
            base_url,
            nav_generation: 0,
            layout_generation: 0,
            socket_path,
            extra_css: String::new(),
            image_cache: HashMap::new(),
            vim_enabled: true,
            vim_mode: VimMode::Normal,
            command_line: TextEditBuffer::default(),
            modifiers: ModifiersState::empty(),
        }
    }

    fn new_streaming(proxy: EventLoopProxy<AppEvent>, socket_path: Option<PathBuf>) -> Self {
        let decoder = new_parser();
        let document = decoder.inner_sink.tokenizer.sink.sink.document.clone();
        let shaping_requests = Self::spawn_shaping_worker(proxy.clone());
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
            shape_cache: ShapeCache::new(),
            surface_size: (0, 0),
            pending_width: None,
            last_cursor: Some(CursorIcon::Default),
            scroll_offsets: HashMap::new(),
            last_mouse_pos: None,
            mouse_down_pos: None,
            proxy,
            shaping_requests,
            base_url: None,
            nav_generation: 0,
            layout_generation: 0,
            socket_path,
            extra_css: String::new(),
            image_cache: HashMap::new(),
            vim_enabled: true,
            vim_mode: VimMode::Normal,
            command_line: TextEditBuffer::default(),
            modifiers: ModifiersState::empty(),
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
            // The DOM walk + cascade match itself stays synchronous,
            // right here, on the main thread — `Handle` (`self.document`)
            // is `Rc`-based, so it can never cross a thread boundary (see
            // `spawn_shaping_worker`'s own docs). Only the `LayoutBox`
            // tree this produces — plain owned data, no `Handle` left
            // inside it — gets handed off below.
            let tree = build_layout_tree_with_resources(&self.document, width as f32, &self.extra_css, &self.image_cache);
            // Ids the worker is about to assign restart at `0` for this
            // new tree (see `spawn_shaping_worker`'s own docs) — stale
            // entries keyed by an old tree's ids would just be harmless
            // dead weight otherwise, but clearing keeps this from
            // growing unboundedly across many stdin chunks in
            // streaming mode.
            self.scroll_offsets.clear();
            self.cached_width = width;
            self.layout_dirty = false;
            self.layout_generation += 1;
            Self::queue_shaping_request(&self.shaping_requests, tree, width as f32, height as f32, self.layout_generation);
        } else if need_rebuild {
            window.request_redraw();
        }

        let mut pixels = surface.buffer_mut().unwrap();
        for pixel in pixels.iter_mut() {
            *pixel = 0x00FFFFFF;
        }

        // `None` on the very first tick or two, while we're still
        // waiting to see whether the width is about to change again
        // (see `pending_width`); also `None` until the background
        // shaping worker's own *first* tick for this page lands
        // (`AppEvent::ShapeResult`) — normally one `PROGRESSIVE_LAYOUT_
        // TICK_BUDGET` or so, same order of magnitude as the old inline
        // call's own first partial result used to take. Either way,
        // paint a blank frame rather than stall on it.
        if let Some(laid) = self.cached.as_ref() {
            let max_scroll = (laid.h - height as f32).max(0.0);
            self.scroll_offset = self.scroll_offset.clamp(0.0, max_scroll);

            paint(
                laid,
                &mut self.font_system,
                &mut self.swash_cache,
                &mut self.shape_cache,
                &self.scroll_offsets,
                &mut pixels,
                width,
                height,
                self.scroll_offset,
                None,
            );
        }

        if self.vim_mode == VimMode::Command {
            paint_command_bar(&self.command_line, &mut self.font_system, &mut self.swash_cache, &mut pixels, width, height);
        }

        window.pre_present_notify();
        pixels.present().unwrap();
    }

    /// Queues a fresh `LayoutBox` tree for the persistent background
    /// shaping worker (`spawn_shaping_worker`, started once in `new_
    /// static`/`new_streaming`) instead of spawning a brand-new thread
    /// per rebuild — step 3 of the "move page shaping off the main
    /// thread" plan in TODO.txt, landed, then revised once real-world
    /// testing (a real Wikipedia page, through a merged PR) surfaced
    /// two problems the one-shot-thread-per-rebuild version had: (1)
    /// paying `new_font_system()`'s own system-font-database scan again
    /// on *every* navigation/resize, not once ever the way `self.font_
    /// system` already was; (2) running `measure_document` to
    /// completion in one call, with no progressive fill-in at all,
    /// which turned "laggy while loading" into "blank for most of a
    /// minute" on a big real page — worse than the problem this whole
    /// plan set out to fix, even though input itself stayed responsive
    /// throughout. A free function, not an `&self` method: `redraw`'s
    /// own caller already holds `self.window`/`self.surface` borrowed
    /// apart from the rest of `self` (see its own destructuring `let`),
    /// which an `&self` method call can't see past.
    fn queue_shaping_request(requests: &mpsc::Sender<ShapingRequest>, tree: LayoutBox, width: f32, height: f32, generation: u64) {
        let _ = requests.send(ShapingRequest { tree, width, height, generation });
    }

    /// Starts the persistent background shaping worker — called once,
    /// from `new_static`/`new_streaming`, not per rebuild. Returns the
    /// `Sender` half; `redraw`'s own rebuild branch queues a `Shaping
    /// Request` through it (`queue_shaping_request`) instead of
    /// spawning a new thread each time.
    ///
    /// **Started eagerly at construction, not lazily on first use** —
    /// so its own one-time cost (building a `FontSystem`, scanning the
    /// system's font database — the same call `self.font_system` itself
    /// already pays once at startup) overlaps with whatever `App`
    /// construction/the very first document parse is doing anyway,
    /// instead of sitting serially in front of the very first page's
    /// own first shape, which is exactly when a user is already
    /// waiting on it.
    ///
    /// **A second, independently-owned `FontSystem`/`SwashCache` pair,
    /// not `self`'s own, but paid for only once, here, not per
    /// request.** Confirmed by reading cosmic-text's own vendored
    /// source: `FontSystem`/`Buffer`/`SwashCache` hold no `Rc`/`RefCell`
    /// internally, so sharing one *instance* with the main thread would
    /// at least compile (behind a `Mutex`, say) — but would just
    /// reintroduce the exact stall this plan exists to remove: the
    /// main thread blocking on the lock while this thread holds it
    /// mid-shape. So this thread keeps its own independent copy for its
    /// entire lifetime instead. `self.font_system`/`self.swash_cache`
    /// stay exactly what `paint`/`paint_command_bar` already use to
    /// paint a finished `Laid` tree on the main thread; nothing about
    /// painting itself changes.
    ///
    /// **Ticks internally, same `PROGRESSIVE_LAYOUT_TICK_BUDGET` this
    /// plan's own first version removed** — but keeps its own
    /// `ShapeCache` growing across ticks *locally*, the same continuity
    /// `App::redraw`'s old inline loop used to get for free by staying
    /// on one thread the whole time, now reproduced across a real
    /// thread boundary: every tick, the whole map — cheap to clone now
    /// that each entry is `Arc<Mutex<_>>` (see `ShapeCache`'s own docs)
    /// — goes out as one `AppEvent::ShapeResult`, so the page fills in
    /// progressively on the main thread exactly as it did before this
    /// plan started, while this thread keeps shaping the rest into the
    /// *same* cache underneath, never losing already-shaped work the
    /// way sending the cache away by value would.
    ///
    /// **Checks for a newer, superseding request between ticks, not
    /// just at the very start** — a real improvement over the no-
    /// cancellation tradeoff the single-spawn-per-rebuild version
    /// (and `start_navigation`'s own fetch thread) still accept:
    /// since this thread is already idle between ticks anyway, a
    /// cheap non-blocking check (`try_recv`) can switch straight to a
    /// resize/navigation/chunk that arrived mid-shape instead of
    /// finishing the now-stale request first, bounding the wasted work
    /// to at most one tick instead of an entire shape pass.
    fn spawn_shaping_worker(proxy: EventLoopProxy<AppEvent>) -> mpsc::Sender<ShapingRequest> {
        let (tx, rx) = mpsc::channel::<ShapingRequest>();
        thread::spawn(move || {
            let mut font_system = new_font_system();
            // `rx.recv()` blocks here whenever there's no page waiting
            // to be shaped — this thread is otherwise entirely idle,
            // not spinning. `next` carries a request straight into the
            // next iteration when the inner loop below was interrupted
            // by one arriving mid-shape (see its own comment), instead
            // of going back through `rx.recv()` and losing it.
            let mut next = rx.recv();
            while let Ok(mut request) = next {
                // Jump straight to the newest already-queued request,
                // if more than one piled up while this thread was busy
                // with something else (e.g. several resizes in a row) —
                // see this function's own docs.
                while let Ok(newer) = rx.try_recv() {
                    request = newer;
                }
                let ShapingRequest { tree, width, height, generation } = request;
                let mut shape_cache = ShapeCache::new();
                let mut superseded_by = None;
                loop {
                    let deadline = std::time::Instant::now() + PROGRESSIVE_LAYOUT_TICK_BUDGET;
                    let mut incomplete = false;
                    let mut next_id = 0usize;
                    let laid = measure_document(
                        &tree,
                        &mut font_system,
                        width,
                        // Same real, definite window dimension `redraw`'s
                        // own synchronous call used to pass here before
                        // this plan started — see `measure_document`'s
                        // own docs for why `viewport_h` specifically
                        // (only `position: fixed`/an orphaned `position:
                        // absolute`'s own containing block) is the one
                        // place that's true.
                        height,
                        &mut next_id,
                        &mut shape_cache,
                        deadline,
                        &mut incomplete,
                    );
                    if proxy.send_event(AppEvent::ShapeResult(generation, Box::new(laid), shape_cache.clone())).is_err() {
                        return; // the event loop (and App) is gone
                    }
                    if !incomplete {
                        break;
                    }
                    // Mid-shape staleness check — see this function's
                    // own docs. A fresh request replaces this one
                    // outright; its own first tick starts from a brand
                    // new, empty `shape_cache`, same as any other new
                    // generation's first tick.
                    if let Ok(newer) = rx.try_recv() {
                        superseded_by = Some(newer);
                        break;
                    }
                }
                next = match superseded_by {
                    Some(newer) => Ok(newer),
                    None => rx.recv(),
                };
            }
        });
        tx
    }

    /// Swaps in a brand-new document — the one place both link
    /// navigation and the "Loading…"/error placeholders it shows while
    /// waiting (`show_message`) go through. Reuses `run()`'s own
    /// initial `App::new_static(dom.document)` path's *shape* (build a
    /// fresh DOM, hand it to the engine) but mid-session instead of only
    /// at startup — everything downstream (`redraw`'s own layout-dirty
    /// rebuild) already works unmodified, since from its point of view
    /// this looks exactly like the document simply having changed, the
    /// same thing `AppEvent::Chunk`/`Eof` already do for streaming mode.
    /// Does *not* touch `base_url` — callers that are loading a real new
    /// page (not just a transient status message) update that
    /// separately, since a synthetic "Loading…" page has no `<base>` of
    /// its own and clobbering `base_url` from it would make a page's
    /// own links briefly unresolvable while its replacement is loading.
    ///
    /// `extra_css` (v67) replaces `self.extra_css` wholesale — a page's
    /// own externally-fetched stylesheets never carry over to whatever
    /// replaces it, same "nothing leaks across a document swap" rule
    /// `scroll_offsets`/`shape_cache` already follow just below.
    /// `image_cache` is cleared unconditionally for the same reason: a
    /// new document starts owing nothing to the previous one's fetched
    /// images (any background fetch still in flight for the *old* page
    /// is harmless — `AppEvent::ImageFetched`'s own `nav_generation`
    /// check drops it before it could repopulate this cache anyway).
    fn replace_document(&mut self, document: Handle, extra_css: String) {
        self.document = document;
        self.extra_css = extra_css;
        self.image_cache.clear();
        self.layout_dirty = true;
        self.layout_generation += 1;
        self.cached = None;
        self.shape_cache.clear();
        self.scroll_offsets.clear();
        self.scroll_offset = 0.0;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Replaces the page with a plain one-paragraph status message —
    /// used for both the "Loading…" indicator `start_navigation` shows
    /// immediately (a real page can take 10-30+ seconds to fetch through
    /// a real `wire` daemon, confirmed directly against Wikipedia in an
    /// earlier session, back when this fetched through `snag --serve`
    /// instead — freezing with no feedback would look broken) and a
    /// failed navigation's inline error (no `--socket` configured, the
    /// daemon unreachable, or a response in a content type scratchy
    /// can't render — see `fetch_raw_via_wire`'s own docs for which).
    /// `escape_html` is needed since `text` can echo back arbitrary
    /// attacker^Wpage-controlled content (a clicked `href`, or an error
    /// string) — without it, a stray `<`/`&` in either would corrupt the
    /// synthetic markup instead of just displaying literally. Never has
    /// any externally-fetched CSS of its own — a synthetic status page
    /// is never real content, so `replace_document` always clears
    /// `extra_css` for it.
    fn show_message(&mut self, text: &str) {
        let html = format!("<html><body><p>{}</p></body></html>", escape_html(text));
        self.replace_document(parse_html(&html), String::new());
    }

    /// Entry point for `WindowEvent::MouseInput`'s click handler:
    /// resolves `href` against the current page's own `<base href>`
    /// (`resolve_href` — ordinary URL-join logic via the `url` crate,
    /// not hand-rolled), then, if that succeeds, shows "Loading…" and
    /// fetches the resolved page on a background thread, exactly the
    /// `thread::spawn` + proxy pattern `run()`'s own `Input::Stdin` case
    /// already established — see `AppEvent::NavResult`'s own docs for
    /// how the result gets back onto this thread and `nav_generation`'s
    /// own docs for why it's tagged.
    ///
    /// Per `scratchy/TODO.txt`'s own settled design, `--socket` is a
    /// strict opt-in: with none configured, or the configured one
    /// unreachable, this never silently does nothing — it prints the
    /// literal HTTP/1.1 request text to stdout (`print_fallback_
    /// request`, directly replayable by `nc`/a raw socket/a manually-run
    /// `wire`) and shows a clear inline error, rather than either
    /// guessing at a default daemon location or leaving the user
    /// wondering whether the click even registered.
    fn start_navigation(&mut self, href: &str) {
        let url = match resolve_href(self.base_url.as_ref(), href) {
            Ok(url) => url,
            Err(message) => {
                self.show_message(&message);
                return;
            }
        };
        self.nav_generation += 1;
        let generation = self.nav_generation;
        self.show_message("Loading…");
        let proxy = self.proxy.clone();
        let socket_path = self.socket_path.clone();
        thread::spawn(move || {
            let request_text = build_get_request_text(&url);
            let outcome: Result<FetchedPage, String> = match &socket_path {
                Some(path) => match fetch_raw_via_wire(path, &request_text) {
                    WireFetchRaw::Responded { content_type, body } => match dispatch_wire_response(content_type, body) {
                        Ok(html) => {
                            // Sub-resources (v67): fetched from *this*
                            // background thread, not the event-loop one —
                            // stylesheets synchronously, since CSS must be
                            // fully in hand before the very first layout
                            // (see `process_external_resources`'s own
                            // docs); images only kicked off here, each
                            // resolving independently and later, via its
                            // own `AppEvent::ImageFetched`.
                            let extra_css = process_external_resources(&proxy, path, &url, &html, generation);
                            Ok(FetchedPage { html, extra_css, url })
                        }
                        Err(e) => Err(e),
                    },
                    WireFetchRaw::Unreachable => {
                        print_fallback_request(&request_text);
                        Err(format!(
                            "wire daemon not reachable at {} — printed the request to stdout; \
                             start it there (e.g. `wire --socket {}`) and click again, or replay \
                             the printed request manually",
                            path.display(),
                            path.display()
                        ))
                    }
                },
                None => {
                    print_fallback_request(&request_text);
                    Err("no --socket given — scratchy never fetches a page on its own without \
                         one; printed the request to stdout for manual replay"
                        .to_string())
                }
            };
            let _ = proxy.send_event(AppEvent::NavResult(generation, outcome));
        });
    }

    /// The one real entry point a pressed key reaches `App` through —
    /// see its own call site (`WindowEvent::KeyboardInput`, filtered to
    /// `ElementState::Pressed` there, release events never reach here
    /// at all). `vim_mode == Command` always goes to the command-line
    /// editor regardless of `vim_enabled`: once `:` has been pressed
    /// (only possible while vim mode was already on), backing out with
    /// `Escape` — or finishing a `:set novim` typed from *inside* the
    /// bar — must still work even the instant after `vim_enabled` would
    /// otherwise gate everything else out. Otherwise, vim mode off is
    /// real pass-through: this function does nothing at all, the "zero
    /// claimed keys" guarantee the whole toggle exists for (TODO.txt).
    fn handle_key_event(&mut self, event: &KeyEvent) {
        if self.vim_mode == VimMode::Command {
            self.handle_command_line_key(event);
            return;
        }
        if !self.vim_enabled {
            return;
        }
        if let Some(action) = resolve_binding(&event.logical_key, self.modifiers, self.vim_mode) {
            self.perform_action(action);
        }
    }

    /// Carries out an already-resolved `Action` — kept separate from
    /// `resolve_binding` itself so that function can stay a pure,
    /// unit-testable `key -> Option<Action>` mapping with no `&mut App`
    /// at all.
    fn perform_action(&mut self, action: Action) {
        const SCROLL_LINE_PX: f32 = 40.0; // matches `MouseWheel`'s own `LineDelta` step
        match action {
            Action::ScrollDown => self.scroll_by(SCROLL_LINE_PX),
            Action::ScrollUp => self.scroll_by(-SCROLL_LINE_PX),
            Action::HalfPageDown => self.scroll_by(self.surface_size.1 as f32 / 2.0),
            Action::HalfPageUp => self.scroll_by(-(self.surface_size.1 as f32 / 2.0)),
            Action::EnterCommandMode => {
                self.vim_mode = VimMode::Command;
                self.command_line.clear();
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Only clamps the lower bound (`0.0`) — same split `WindowEvent::
    /// MouseWheel`'s own top-level-scroll branch already relies on:
    /// `redraw`'s own per-frame `self.scroll_offset.clamp(0.0, max_
    /// scroll)` is what enforces the upper bound, since `max_scroll`
    /// depends on `self.cached.h`, which this method has no reason to
    /// duplicate.
    fn scroll_by(&mut self, dy: f32) {
        self.scroll_offset = (self.scroll_offset + dy).max(0.0);
    }

    /// `vim_mode == Command`'s own key handling — direct text-buffer
    /// editing (`TextEditBuffer`'s methods), not a `resolve_binding`
    /// lookup: there's no sensible "binding" for "the character the
    /// user just typed," and command-mode text input needs to see the
    /// actual OS-composed logical key output (dead keys, non-Latin
    /// layouts via `logical_key`'s own IME-adjacent handling), which is
    /// exactly what the ordinary page a browser would otherwise send
    /// key events to gets too.
    fn handle_command_line_key(&mut self, event: &KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.vim_mode = VimMode::Normal;
                self.command_line.clear();
            }
            Key::Named(NamedKey::Enter) => {
                let input = std::mem::take(&mut self.command_line.text);
                self.command_line.cursor = 0;
                self.vim_mode = VimMode::Normal;
                self.run_command_line(&input);
            }
            Key::Named(NamedKey::Backspace) => self.command_line.backspace(),
            Key::Named(NamedKey::ArrowLeft) => self.command_line.move_left(),
            Key::Named(NamedKey::ArrowRight) => self.command_line.move_right(),
            key => {
                if let Some(text) = key_insertion_text(key) {
                    for c in text.chars() {
                        self.command_line.insert(c);
                    }
                }
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Splits a submitted command line into its name and whitespace-
    /// separated args and hands it to `dispatch_command` — kept as its
    /// own step so `dispatch_command` itself only ever deals with
    /// already-tokenized input, same shape a future `:`-command with
    /// quoted args could still slot into later without reworking the
    /// split-and-dispatch boundary.
    fn run_command_line(&mut self, input: &str) {
        let mut parts = input.split_whitespace();
        let Some(name) = parts.next() else { return };
        let args: Vec<&str> = parts.collect();
        self.dispatch_command(name, &args);
    }

    /// The one chokepoint a submitted `:`-command's name+args get
    /// resolved into its actual effect through — hardcoded `match` arms
    /// today, same "named override point for a future JS-scriptable
    /// config layer" shape `resolve_binding` already follows for
    /// keybindings (TODO.txt). `:visit` reuses `start_navigation`
    /// outright — exactly the same href-resolve-then-fetch path a real
    /// `<a>` click already goes through, so an already-absolute URL
    /// (the common case typed here) or one relative to the current
    /// page's own `<base href>` both just work.
    fn dispatch_command(&mut self, name: &str, args: &[&str]) {
        match (name, args) {
            ("visit", [url]) => self.start_navigation(url),
            ("set", ["vim"]) => self.vim_enabled = true,
            ("set", ["novim"]) => self.vim_enabled = false,
            _ => {
                let rest = args.join(" ");
                self.show_message(&format!("scratchy: unknown command: :{name}{}{rest}", if rest.is_empty() { "" } else { " " }));
            }
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
                    self.layout_generation += 1;
                }
            }
            AppEvent::Eof => {
                if let Some(decoder) = self.decoder.take() {
                    let dom = decoder.finish();
                    self.document = dom.document;
                    self.layout_dirty = true;
                    self.layout_generation += 1;
                    // Streaming mode (`snag url | scratchy`) never had a
                    // `<base href>` to read before now — the whole
                    // document, `<head>` included, only exists once EOF
                    // has been seen. Picking it up here (rather than
                    // leaving `base_url` permanently `None` for a piped
                    // page) is what makes clicking a link actually work
                    // after `snag url | scratchy`, not just after
                    // `scratchy saved-bundle.html`.
                    self.base_url = find_base_url(&self.document);
                }
            }
            AppEvent::NavResult(generation, result) => {
                // A result from a navigation the user has since clicked
                // past (a second link, before the first one's fetch
                // finished) — drop it silently rather than clobbering
                // whatever's already newer on screen. See `AppEvent::
                // NavResult`'s own docs.
                if generation != self.nav_generation {
                    return;
                }
                match result {
                    Ok(FetchedPage { html, extra_css, url }) => {
                        let dom = new_parser().read_from(&mut html.as_slice()).unwrap();
                        self.base_url = Some(resolve_page_base_url(&dom.document, url));
                        self.replace_document(dom.document, extra_css);
                    }
                    Err(message) => self.show_message(&message),
                }
            }
            AppEvent::ImageFetched(generation, src, image) => {
                // Same staleness guard `NavResult` already uses — a slow
                // image fetch that only finishes after the user has since
                // navigated elsewhere must never populate the *new*
                // page's `image_cache` just because it happens to share
                // the same raw `src` string. See `AppEvent::ImageFetched`'s
                // own docs.
                if generation != self.nav_generation {
                    return;
                }
                let Some(image) = image else { return };
                self.image_cache.insert(src, Arc::new(image));
                // A newly-arrived image can change this element's own
                // intrinsic size (no explicit CSS/HTML width/height —
                // see `resolve_img`), so the whole tree needs a real
                // rebuild, not just a repaint — the same `layout_dirty`
                // flag `redraw` already checks before deciding whether to
                // call `build_layout_tree_with_resources` again.
                self.layout_dirty = true;
                self.layout_generation += 1;
            }
            AppEvent::ShapeResult(generation, laid, shape_cache) => {
                // Same staleness guard `NavResult`/`ImageFetched` already
                // use, tagged with `layout_generation` instead of `nav_
                // generation` — a background shaping pass that only
                // finishes after a resize/chunk/navigation/image already
                // started a newer one must never clobber content that's
                // already further along. See `AppEvent::ShapeResult`'s
                // own docs.
                if generation != self.layout_generation {
                    return;
                }
                // Replaced wholesale on every tick this generation's
                // worker reports, not merged — see `App::shape_cache`'s
                // own docs for why that's always safe (every entry here
                // is a strict superset of the last delivery's, same
                // generation, same underlying tree).
                self.shape_cache = shape_cache;
                self.cached = Some(*laid);
            }
            AppEvent::ToggleVimMode => {
                self.vim_enabled = !self.vim_enabled;
                self.vim_mode = VimMode::Normal;
                self.command_line.clear();
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
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Release events never reach `handle_key_event` at all —
                // every binding today is a one-shot action (scroll a
                // fixed step, enter command mode, edit the command
                // line), nothing holds a key down for continuous effect
                // yet, so there's nothing a release would need to do.
                if event.state == ElementState::Pressed {
                    self.handle_key_event(&event);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // Remembered in window/screen space, same form this event
                // itself reports — `MouseWheel` (just below) needs it to
                // pick the right scroll region, since that event carries
                // no position of its own at all.
                self.last_mouse_pos = Some((position.x as f32, position.y as f32));
                // `position` is window/screen space; `Laid.x`/`.y` are
                // document space (scroll isn't applied until paint time),
                // so the same `+ scroll_offset` adjustment `paint` itself
                // makes (in reverse) is needed here too.
                if let (Some(laid), Some(window)) = (&self.cached, &self.window) {
                    let doc_x = position.x as f32;
                    let doc_y = position.y as f32 + self.scroll_offset;
                    let (style, _href) = hit_test_cursor(
                        laid, doc_x, doc_y, &self.shape_cache, &self.scroll_offsets, self.scroll_offset,
                        self.surface_size.1 as f32,
                    );
                    let icon = style.to_winit();
                    if icon != self.last_cursor {
                        match icon {
                            // Restore visibility too, in case the cursor
                            // was hidden (`cursor: none`) immediately
                            // beforehand — only when actually transitioning
                            // out of that state, not on every tick.
                            Some(ic) => {
                                window.set_cursor(ic);
                                if self.last_cursor.is_none() {
                                    window.set_cursor_visible(true);
                                }
                            }
                            None => window.set_cursor_visible(false),
                        }
                        self.last_cursor = icon;
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                // Scratchy has no text selection yet (see `is_click`'s
                // own docs), but binding navigation to mouse-down alone
                // would still be wrong on its own terms — a drag that
                // happens to start and end over a link shouldn't
                // navigate either. `winit`'s own `MouseInput` carries no
                // position of its own (same gap `MouseWheel` already
                // has) — `last_mouse_pos`, kept live by `CursorMoved`
                // just above, stands in for both the press and release
                // point.
                if button != MouseButton::Left {
                    return;
                }
                match state {
                    ElementState::Pressed => self.mouse_down_pos = self.last_mouse_pos,
                    ElementState::Released => {
                        // Computed into a plain, owned `Option<&'static
                        // str>` (not borrowed from `self`) *before*
                        // calling `start_navigation` below, which needs
                        // `&mut self` — `laid`/`shape_cache`/
                        // `scroll_offsets` are only borrowed for this
                        // one `match`, not held across that call.
                        let href = match (self.mouse_down_pos.take(), self.last_mouse_pos, &self.cached) {
                            (Some(down), Some(up), Some(laid)) if is_click(down, up, CLICK_DRAG_THRESHOLD_PX) => {
                                let doc_x = up.0;
                                let doc_y = up.1 + self.scroll_offset;
                                hit_test_cursor(
                                    laid, doc_x, doc_y, &self.shape_cache, &self.scroll_offsets,
                                    self.scroll_offset, self.surface_size.1 as f32,
                                )
                                .1
                            }
                            _ => None,
                        };
                        self.mouse_down_pos = None;
                        if let Some(href) = href {
                            self.start_navigation(href);
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 40.0,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32,
                };
                // Routes to the deepest `overflow: scroll`/`auto` region
                // under the last known mouse position, if any — falls
                // through to the page's own top-level scroll (the
                // pre-v62 behavior, unchanged) whenever the cursor isn't
                // over one, or hasn't moved over the window at all yet.
                let region = self.last_mouse_pos.zip(self.cached.as_ref()).and_then(|((mx, my), laid)| {
                    let doc_x = mx;
                    let doc_y = my + self.scroll_offset;
                    hit_test_scroll_region(
                        laid, doc_x, doc_y, &self.scroll_offsets, self.scroll_offset, self.surface_size.1 as f32,
                    )
                });
                match region {
                    Some(info) => {
                        let current = self.scroll_offsets.get(&info.id).copied().unwrap_or(0.0);
                        let next = (current - dy).clamp(0.0, info.max_scroll());
                        self.scroll_offsets.insert(info.id, next);
                    }
                    None => {
                        self.scroll_offset -= dy;
                        if self.scroll_offset < 0.0 {
                            self.scroll_offset = 0.0;
                        }
                    }
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
    let Args { input, socket } = parse_args();

    let event_loop = EventLoop::<AppEvent>::with_user_event().build().unwrap();
    // Needed unconditionally now, not just for `Input::Stdin`'s own
    // background thread — `App::start_navigation`'s link-navigation
    // fetch (v65) needs a clone too, and `ActiveEventLoop` (unlike the
    // top-level `EventLoop` itself) has no `create_proxy` of its own
    // (confirmed by reading winit's vendored source), so this has to be
    // created here and threaded into `App` at construction time.
    let proxy = event_loop.create_proxy();

    // `SIGUSR1` bootstrap (TODO.txt's own vim-mode plan): the only way
    // to turn vim mode back *on* once `:set novim` has turned it off,
    // since nothing in-band could reach `:set vim` again at that point
    // — see `AppEvent::ToggleVimMode`'s own docs. Registered
    // unconditionally, same as the proxy above, not gated behind vim
    // mode ever having been used — a signal handler can't safely touch
    // `App` state directly (signal-safety: a raw handler interrupts the
    // main thread at an arbitrary point), so this follows the exact
    // `thread::spawn` + `proxy.send_event` shape `Input::Stdin`'s own
    // reader thread and `App::start_navigation`'s fetch thread already
    // use below — not new architecture, one more thread of the same
    // shape, blocking on `Signals::forever()` instead of a socket/stdin
    // read.
    let mut signals = Signals::new([SIGUSR1]).expect("failed to register SIGUSR1 handler");
    {
        let proxy = proxy.clone();
        thread::spawn(move || {
            for _ in signals.forever() {
                if proxy.send_event(AppEvent::ToggleVimMode).is_err() {
                    break;
                }
            }
        });
    }

    let mut app = match input {
        Input::File(path) => {
            let html = fs::read_to_string(&path).unwrap_or_else(|e| {
                eprintln!("scratchy: {e}");
                std::process::exit(1);
            });
            let dom = new_parser().read_from(&mut html.as_bytes()).unwrap();
            App::new_static(dom.document, proxy, socket)
        }
        Input::Stdin => {
            let stdin_proxy = proxy.clone();
            thread::spawn(move || {
                let proxy = stdin_proxy;
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
            App::new_streaming(proxy, socket)
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
        let tree = build_layout_tree(&doc, 1920.0);
        let div = find_deepest_nonzero_margin_top(&tree).expect("div not found");
        assert_eq!(div.style.margin.top.px(), 80.0, "2rem should resolve against html's 40px, giving 80px");
    }

    #[test]
    fn rem_defaults_to_16_without_html_override() {
        let doc = parse_html(r#"<html><body><div style="margin-top: 2rem">x</div></body></html>"#);
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(color, TextColor::rgb(0xff, 0, 0), ":link should match an <a href> and apply red");
    }

    #[test]
    fn link_pseudo_does_not_match_anchor_without_href() {
        let doc = parse_html(r#"<html><body><style>a:link { color: #ff0000; }</style><a>hi</a></body></html>"#);
        let tree = build_layout_tree(&doc, 1920.0);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_ne!(color, TextColor::rgb(0xff, 0, 0), ":link should not match an <a> without href");
    }

    #[test]
    fn visited_never_matches() {
        let doc = parse_html(
            r#"<html><body><style>a:visited { color: #00ff00; }</style><a href="x">hi</a></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
        let mut decos = Vec::new();
        collect_decorations(&tree, &mut decos);
        assert!(decos.iter().any(|d| *d != TextDecoration::new()), "p's own direct text should be underlined: {:?}", decos);
    }

    #[test]
    fn decoration_propagates_through_nested_block_descendant() {
        let doc = parse_html(
            r#"<html><body><div style="text-decoration: underline"><div>nested</div></div></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
        let mut decos = Vec::new();
        collect_decorations(&tree, &mut decos);
        assert!(decos.iter().any(|d| *d != TextDecoration::new()), "nested block descendant should inherit ambient decoration: {:?}", decos);
    }

    #[test]
    fn nested_block_can_opt_out_of_decoration_with_none() {
        let doc = parse_html(
            r#"<html><body><div style="text-decoration: underline"><div style="text-decoration: none">nested</div></div></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
        let mut decos = Vec::new();
        collect_decorations(&tree, &mut decos);
        assert!(decos.iter().all(|d| *d == TextDecoration::new()), "explicit none should opt out: {:?}", decos);
    }

    #[test]
    fn sibling_block_does_not_leak_decoration() {
        let doc = parse_html(
            r#"<html><body><div style="text-decoration: underline">a</div><div>b</div></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);

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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(color, TextColor::rgb(0xff, 0, 0), "var(--c) should resolve to the :root-declared value");
    }

    #[test]
    fn html_selector_also_scopes_a_custom_prop() {
        let doc = parse_html(
            r#"<html><head><style>html { --c: #0000ff; } p { color: var(--c); }</style></head><body><p>hi</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(color, TextColor::rgb(0, 0, 0xff), "var(--c) should resolve via a plain `html` selector too");
    }

    #[test]
    fn var_fallback_used_when_prop_undeclared() {
        let doc = parse_html(
            r#"<html><head><style>p { color: var(--undefined, #00ff00); }</style></head><body><p>hi</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(color, TextColor::rgb(0, 0xff, 0), "an undeclared custom property should fall back to the fallback value");
    }

    #[test]
    fn nested_var_fallback_resolves() {
        let doc = parse_html(
            r#"<html><head><style>p { color: var(--a, var(--b, #112233)); }</style></head><body><p>hi</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
        let color = find_first_span_color(&tree).expect("no span found");
        assert_eq!(color, TextColor::rgb(0x11, 0x22, 0x33), "a nested var() inside a fallback should itself resolve");
    }

    #[test]
    fn non_root_custom_prop_is_not_picked_up() {
        let doc = parse_html(
            r#"<html><head><style>.scope { --c: #ff0000; } p { color: var(--c, #00ff00); }</style></head><body><p class="scope">hi</p></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
        let text = find_inline_text(&tree).expect("no text found");
        assert!(text.contains("a fallback"), "a missing src should fall back to rendering alt text: {text:?}");
    }

    #[test]
    fn img_with_undecodable_src_falls_back_to_alt_text() {
        let doc = parse_html(r#"<html><body><img src="data:image/png;base64,not-valid-base64!!" alt="broken img"></body></html>"#);
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);

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

    // ---------- border-style bevel variants: groove/ridge/inset/outset (v63) ----------

    /// Reads back the plain `0x00RRGGBB` pixel at `(x, y)` out of a
    /// `paint_border`-painted buffer — same masking `px_at` uses, kept
    /// local to this section since these tests paint directly into a
    /// hand-rolled buffer rather than through `paint_fixture`.
    fn bevel_px(pixels: &[u32], win_w: u32, x: f32, y: f32) -> u32 {
        pixels[(y as u32 * win_w + x as u32) as usize] & 0x00FF_FFFF
    }

    #[test]
    fn inset_border_paints_top_left_edges_darker_than_bottom_right() {
        // Mid-gray so `lighten`/`darken` produce two values that are
        // both clearly different from the base and from each other.
        let base = TextColor::rgb(128, 128, 128);
        let (win_w, win_h) = (100u32, 100u32);
        let mut pixels = vec![0x00FF_FFFFu32; (win_w * win_h) as usize];
        let (x, y, w, h, bw) = (10.0, 10.0, 80.0, 80.0, 10.0);
        paint_border(&mut pixels, win_w, win_h, x, y, w, h, bw, BorderStyle::Inset, base, None);

        let top = bevel_px(&pixels, win_w, x + w / 2.0, y + bw / 2.0);
        let left = bevel_px(&pixels, win_w, x + bw / 2.0, y + h / 2.0);
        let bottom = bevel_px(&pixels, win_w, x + w / 2.0, y + h - bw / 2.0);
        let right = bevel_px(&pixels, win_w, x + w - bw / 2.0, y + h / 2.0);

        let dark = darken(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
        let light = lighten(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
        assert_eq!(top, dark, "inset's top edge should be the darkened variant");
        assert_eq!(left, dark, "inset's left edge should be the darkened variant");
        assert_eq!(bottom, light, "inset's bottom edge should be the lightened variant");
        assert_eq!(right, light, "inset's right edge should be the lightened variant");
        assert_ne!(dark, light, "the two computed variants must actually differ for this test to mean anything");
    }

    #[test]
    fn outset_border_is_the_reverse_of_inset() {
        let base = TextColor::rgb(128, 128, 128);
        let (win_w, win_h) = (100u32, 100u32);
        let mut pixels = vec![0x00FF_FFFFu32; (win_w * win_h) as usize];
        let (x, y, w, h, bw) = (10.0, 10.0, 80.0, 80.0, 10.0);
        paint_border(&mut pixels, win_w, win_h, x, y, w, h, bw, BorderStyle::Outset, base, None);

        let top = bevel_px(&pixels, win_w, x + w / 2.0, y + bw / 2.0);
        let left = bevel_px(&pixels, win_w, x + bw / 2.0, y + h / 2.0);
        let bottom = bevel_px(&pixels, win_w, x + w / 2.0, y + h - bw / 2.0);
        let right = bevel_px(&pixels, win_w, x + w - bw / 2.0, y + h / 2.0);

        let dark = darken(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
        let light = lighten(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
        assert_eq!(top, light, "outset's top edge should be the lightened variant (opposite of inset)");
        assert_eq!(left, light, "outset's left edge should be the lightened variant");
        assert_eq!(bottom, dark, "outset's bottom edge should be the darkened variant");
        assert_eq!(right, dark, "outset's right edge should be the darkened variant");
    }

    #[test]
    fn groove_border_splits_each_edge_into_an_outer_dark_and_inner_light_half() {
        // `Groove`'s own "carved in" crease: unlike `Inset`, each single
        // edge shows *two* shades, not one — the half farther from this
        // box's own content ("outer") dark, the half closer to it
        // ("inner") light, for the top/left edges; reversed for
        // bottom/right. `bw = 10` → `half = 5`, so the outer/inner
        // sample points below (2px/8px into a 10px-thick edge) land
        // solidly inside their own half with no rounding ambiguity.
        let base = TextColor::rgb(128, 128, 128);
        let (win_w, win_h) = (100u32, 100u32);
        let mut pixels = vec![0x00FF_FFFFu32; (win_w * win_h) as usize];
        let (x, y, w, h, bw) = (10.0, 10.0, 80.0, 80.0, 10.0);
        paint_border(&mut pixels, win_w, win_h, x, y, w, h, bw, BorderStyle::Groove, base, None);

        let dark = darken(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
        let light = lighten(base, BEVEL_RATIO).0 & 0x00FF_FFFF;

        // Top edge: outer (upper, y=12) dark, inner (lower, y=18) light.
        assert_eq!(bevel_px(&pixels, win_w, x + w / 2.0, y + 2.0), dark, "groove top edge's outer half should be dark");
        assert_eq!(bevel_px(&pixels, win_w, x + w / 2.0, y + 8.0), light, "groove top edge's inner half should be light");
        // Left edge: outer (left, x=12) dark, inner (right, x=18) light.
        assert_eq!(bevel_px(&pixels, win_w, x + 2.0, y + h / 2.0), dark, "groove left edge's outer half should be dark");
        assert_eq!(bevel_px(&pixels, win_w, x + 8.0, y + h / 2.0), light, "groove left edge's inner half should be light");
        // Bottom edge: inner (upper, closer to content) dark, outer (lower) light.
        assert_eq!(
            bevel_px(&pixels, win_w, x + w / 2.0, y + h - 8.0),
            dark,
            "groove bottom edge's inner half should be dark"
        );
        assert_eq!(
            bevel_px(&pixels, win_w, x + w / 2.0, y + h - 2.0),
            light,
            "groove bottom edge's outer half should be light"
        );
        // Right edge: inner (left, closer to content) dark, outer (right) light.
        assert_eq!(
            bevel_px(&pixels, win_w, x + w - 8.0, y + h / 2.0),
            dark,
            "groove right edge's inner half should be dark"
        );
        assert_eq!(
            bevel_px(&pixels, win_w, x + w - 2.0, y + h / 2.0),
            light,
            "groove right edge's outer half should be light"
        );
    }

    #[test]
    fn ridge_border_is_the_reverse_of_groove_at_every_sampled_point() {
        let base = TextColor::rgb(128, 128, 128);
        let (win_w, win_h) = (100u32, 100u32);
        let mut groove_pixels = vec![0x00FF_FFFFu32; (win_w * win_h) as usize];
        let mut ridge_pixels = vec![0x00FF_FFFFu32; (win_w * win_h) as usize];
        let (x, y, w, h, bw) = (10.0, 10.0, 80.0, 80.0, 10.0);
        paint_border(&mut groove_pixels, win_w, win_h, x, y, w, h, bw, BorderStyle::Groove, base, None);
        paint_border(&mut ridge_pixels, win_w, win_h, x, y, w, h, bw, BorderStyle::Ridge, base, None);

        let dark = darken(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
        let light = lighten(base, BEVEL_RATIO).0 & 0x00FF_FFFF;
        let sample_points = [
            (x + w / 2.0, y + 2.0),       // top, outer
            (x + w / 2.0, y + 8.0),       // top, inner
            (x + 2.0, y + h / 2.0),       // left, outer
            (x + 8.0, y + h / 2.0),       // left, inner
            (x + w / 2.0, y + h - 8.0),   // bottom, inner
            (x + w / 2.0, y + h - 2.0),   // bottom, outer
            (x + w - 8.0, y + h / 2.0),   // right, inner
            (x + w - 2.0, y + h / 2.0),   // right, outer
        ];
        for (sx, sy) in sample_points {
            let g = bevel_px(&groove_pixels, win_w, sx, sy);
            let r = bevel_px(&ridge_pixels, win_w, sx, sy);
            assert_ne!(g, r, "groove and ridge should disagree at every sampled point ({sx}, {sy})");
            assert!(g == dark || g == light, "groove's own sample should be one of the two bevel shades");
            assert!(r == dark || r == light, "ridge's own sample should be one of the two bevel shades");
            assert_eq!(g, if r == dark { light } else { dark }, "ridge should be exactly groove's opposite shade at this point");
        }
    }

    #[test]
    fn border_style_longhand_parses_all_four_bevel_keywords() {
        for (keyword, width, expected) in [
            ("groove", 201.0, BorderStyle::Groove),
            ("ridge", 202.0, BorderStyle::Ridge),
            ("inset", 203.0, BorderStyle::Inset),
            ("outset", 204.0, BorderStyle::Outset),
        ] {
            let html = format!(r#"<html><body><div style="width: {width}px; border-style: {keyword};">x</div></body></html>"#);
            let laid = measure_fixture(&html, 500.0);
            let div = find_by_width(&laid, width).unwrap_or_else(|| panic!("div with border-style: {keyword} not found"));
            assert_eq!(
                div.style.border_style, expected,
                "border-style: {keyword} should resolve to BorderStyle::{expected:?}"
            );
        }
    }

    #[test]
    fn border_style_inset_paints_visibly_different_shades_through_the_real_pipeline() {
        // End-to-end through `paint_box_decoration`'s own real
        // `paint_border` call (not the primitive tested directly above)
        // — confirms the new bevel branch is actually reached from a
        // real laid-out, painted document, not just callable in
        // isolation.
        let (pixels, win_w, _win_h) = paint_fixture(
            r#"<html><body style="margin:0"><div style="box-sizing: border-box; width: 100px; height: 100px; border: 10px solid gray; border-style: inset;"></div></body></html>"#,
        );
        let top = px_at(&pixels, win_w, 50, 5);
        let bottom = px_at(&pixels, win_w, 50, 95);
        assert_ne!(top, bottom, "an inset border's top and bottom edges should paint visibly different shades");
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
            let entry = cache.get(&id).expect("missing shaped paragraph");
            let guard = entry.lock().unwrap();
            guard
                .0
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
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
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, scroll_offset, None);
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
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels_a, win_w, win_h, 0.0, None);
        let mut pixels_b = vec![0u32; (win_w * win_h) as usize];
        paint(
            &laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels_b, win_w, win_h, static_top + 200.0,
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
        let tree = build_layout_tree(&doc, 1920.0);
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
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h, 0.0, None);
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

    // ---------- v58: `display: grid` ----------

    /// The grid container's own direct `LaidContent::Blocks` children,
    /// in document order — exactly the grid items `measure_grid`
    /// placed, found by locating the container itself (via its own
    /// distinctive `width`, same `find_by_width` helper every other
    /// section already uses), mirroring `flex_items` above.
    fn grid_items(laid: &Laid, container_w: f32) -> Vec<&Laid> {
        let container = find_by_width(laid, container_w).expect("grid container not found");
        let LaidContent::Blocks(children) = &container.content else {
            panic!("expected the grid container's content to be Blocks (its own grid items)");
        };
        children.iter().collect()
    }

    #[test]
    fn grid_fixed_track_widths_place_items_at_the_right_x_offsets() {
        let laid = measure_fixture(
            r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 50px 100px 150px; width: 300px;"><div>a</div><div>b</div><div>c</div></div></body></html>"#,
            500.0,
        );
        let items = grid_items(&laid, 300.0);
        assert_eq!(items.len(), 3, "all three children should become grid items");
        assert_eq!(items[0].x, 0.0, "the first column should start at the grid's own content edge");
        assert_eq!(items[0].w, 50.0, "the first item should fill its own 50px column, not shrink to its own content");
        assert_eq!(items[1].x, 50.0, "the second column should start exactly where the first ends");
        assert_eq!(items[1].w, 100.0);
        assert_eq!(items[2].x, 150.0, "the third column should start exactly where the second ends");
        assert_eq!(items[2].w, 150.0);
        assert_eq!(items[0].y, items[1].y, "fixed-width columns should all share one row");
        assert_eq!(items[1].y, items[2].y);
    }

    #[test]
    fn grid_fr_tracks_distribute_proportionally_with_asymmetric_ratios() {
        let laid = measure_fixture(
            r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 1fr 3fr; width: 400px;"><div>a</div><div>b</div></div></body></html>"#,
            500.0,
        );
        let items = grid_items(&laid, 400.0);
        assert_eq!(items[0].w, 100.0, "1fr of (1+3) should claim 1/4 of the 400px content width");
        assert_eq!(items[1].w, 300.0, "3fr of (1+3) should claim 3/4 — not an equal 50/50 split");
        assert_eq!(items[1].x, items[0].x + items[0].w, "the second column should start exactly where the first, proportionally-sized one ends");
    }

    #[test]
    fn grid_fr_and_fixed_tracks_mix_correctly() {
        // A fixed 100px track plus a 1fr/3fr pair sharing whatever's
        // left (400 - 100 = 300, split 1:3 → 75/225) — pins that `fr`
        // distribution happens against the *leftover* space, not the
        // container's full width, once a fixed track is also present.
        let laid = measure_fixture(
            r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 100px 1fr 3fr; width: 400px;"><div>a</div><div>b</div><div>c</div></div></body></html>"#,
            500.0,
        );
        let items = grid_items(&laid, 400.0);
        assert_eq!(items[0].w, 100.0, "the fixed track keeps its own declared width regardless of its fr siblings");
        assert_eq!(items[1].w, 75.0, "1fr of the remaining 300px (1+3 share) is 75px");
        assert_eq!(items[2].w, 225.0, "3fr of the remaining 300px (1+3 share) is 225px");
    }

    #[test]
    fn grid_auto_placement_wraps_to_a_new_row_once_a_row_fills() {
        let laid = measure_fixture(
            r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 100px 100px; width: 200px;"><div style="height: 30px;">a</div><div style="height: 30px;">b</div><div style="height: 40px;">c</div></div></body></html>"#,
            500.0,
        );
        let items = grid_items(&laid, 200.0);
        assert_eq!(items.len(), 3, "all three children should become grid items despite the wrap");
        assert_eq!(items[0].y, items[1].y, "the first two items (filling the 2-column row) should share the first row");
        assert_eq!(items[0].x, 0.0, "the first item should sit in the first column");
        assert_eq!(items[1].x, 100.0, "the second item should sit in the second column, not stacked on the first");
        assert_eq!(items[2].x, 0.0, "a third item, once the 2-column row is full, should wrap back to the first column of a new row");
        assert_eq!(
            items[2].y,
            items[0].y + 30.0,
            "the new row should start exactly where the first row's own height (the taller of its two 30px items) ends"
        );
    }

    #[test]
    fn grid_gap_adds_spacing_between_both_columns_and_rows() {
        let laid = measure_fixture(
            r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 50px 50px; gap: 10px; width: 110px;"><div style="height: 20px;">a</div><div style="height: 20px;">b</div><div style="height: 20px;">c</div><div style="height: 20px;">d</div></div></body></html>"#,
            500.0,
        );
        let items = grid_items(&laid, 110.0);
        assert_eq!(items.len(), 4, "a 2x2 grid of 4 items, wrapping once the 2-column row fills");
        assert_eq!(items[1].x, items[0].x + items[0].w + 10.0, "column-gap (from the `gap` shorthand) should add exactly 10px between the two columns");
        assert_eq!(items[2].y, items[0].y + items[0].h + 10.0, "row-gap (from the same `gap` shorthand) should add exactly 10px between the two rows");
    }

    #[test]
    fn grid_explicit_row_track_sizes_the_row_regardless_of_content() {
        let laid = measure_fixture(
            r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: 100px; grid-template-rows: 80px; width: 100px;"><div style="height: 10px;">a</div></div></body></html>"#,
            500.0,
        );
        let items = grid_items(&laid, 100.0);
        assert_eq!(items.len(), 1);
        let container = find_by_width(&laid, 100.0).expect("grid container not found");
        assert_eq!(container.h, 80.0, "an explicit grid-template-rows size should set the row's (and so the container's) height regardless of its shorter content");
        assert_eq!(items[0].h, 10.0, "the item itself still reports its own natural content height — only the row's track, not the item, is forced to 80px");
    }

    #[test]
    fn grid_repeat_expands_to_the_same_track_list_as_writing_it_out() {
        let laid = measure_fixture(
            r#"<html><body style="margin: 0"><div style="display: grid; grid-template-columns: repeat(3, 50px); width: 150px;"><div>a</div><div>b</div><div>c</div></div></body></html>"#,
            500.0,
        );
        let items = grid_items(&laid, 150.0);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].w, 50.0);
        assert_eq!(items[1].x, 50.0);
        assert_eq!(items[2].x, 100.0);
    }

    #[test]
    fn grid_position_absolute_child_is_not_treated_as_a_grid_item() {
        let laid = measure_fixture(
            r#"<html><body style="margin: 0"><div style="display: grid; position: relative; grid-template-columns: 100px; width: 100px;"><div style="position: absolute; top: 5px; left: 5px; width: 20px; height: 20px;">p</div><div>a</div></div></body></html>"#,
            500.0,
        );
        let container = find_by_width(&laid, 100.0).expect("grid container not found");
        let LaidContent::Blocks(children) = &container.content else {
            panic!("expected Blocks content");
        };
        assert_eq!(children.len(), 1, "the absolutely-positioned child should be pulled out of the grid item list entirely");
        assert_eq!(children[0].x, container.x, "the one real grid item should occupy the grid's first cell (its own left content edge), not pushed aside by the out-of-flow child");
        assert_eq!(container.positioned.len(), 1, "the absolutely-positioned child should instead land in the container's own `positioned` list");
    }

    // ---------- v59: `word-spacing` ----------

    /// Every distinct `y` among a `MixedInline` run's own positioned
    /// items, rounded to avoid float-equality flakiness — the number of
    /// *lines* a `measure_mixed_inline` call actually produced, used by
    /// this section's wrap-regression tests the same way `line_height`-
    /// based line counting would, but without depending on real font
    /// metrics for line height itself.
    fn mixed_inline_line_count(laid: &Laid) -> usize {
        let items = mixed_items(laid);
        let ys: std::collections::BTreeSet<i64> =
            items.iter().map(|it| (mixed_item_xy(it).1 * 100.0).round() as i64).collect();
        ys.len()
    }

    #[test]
    fn word_spacing_normal_keeps_the_fast_cosmic_text_path() {
        // The overwhelming common case (`word-spacing` never declared,
        // or explicitly `normal`) must stay on the existing single-
        // `Buffer`-per-paragraph path — zero regression risk/cost for
        // every page that doesn't use this property, same guarantee
        // v48/v54 already give for a run with no image and no active
        // float.
        let laid = measure_fixture(r#"<html><body><p style="word-spacing: normal">several plain words here</p></body></html>"#, 500.0);
        assert!(
            find_mixed_inline(&laid).is_none(),
            "`word-spacing: normal` should not route a plain-text run through the slower per-word path"
        );
        let mut ids = Vec::new();
        collect_text_shape_ids(&laid, &mut ids);
        assert!(!ids.is_empty(), "sanity: the paragraph should still have shaped via the ordinary whole-buffer `LaidContent::Text` path");
    }

    #[test]
    fn nonzero_word_spacing_routes_through_the_per_word_path_with_no_image_or_float() {
        // The core routing change this item adds: unlike v48 (image
        // present) and v54 (an active float), a plain-text run with a
        // declared `word-spacing` has neither — it still has to take
        // `measure_mixed_inline`'s hand-rolled path, since that's the
        // only place this property is actually applied (see
        // `ComputedStyle::word_spacing`'s own docs).
        let laid = measure_fixture(r#"<html><body><p style="word-spacing: 10px">foo bar</p></body></html>"#, 500.0);
        let run = find_mixed_inline(&laid)
            .expect("a run with nonzero word-spacing and no image/float should still route through measure_mixed_inline");
        assert_eq!(mixed_items(run).len(), 2, "two words, each its own MixedPositioned::Word");
    }

    #[test]
    fn word_spacing_increases_the_gap_between_two_words() {
        // The real geometry proof, not just "doesn't crash": the second
        // word's own starting `x` should shift further right as the
        // declared `word-spacing` grows, by exactly the difference
        // between the two declared values — proving this is a real,
        // precisely-sized per-word position change, not a no-op that
        // merely avoids a crash/mis-wrap.
        //
        // Deliberately *not* computed as "word 1's right edge to word
        // 2's left edge": the extra spacing is baked directly into word
        // 1's own stored `w` (see `measure_mixed_inline`'s own docs) —
        // the same "a trailing space counts toward this word's own
        // width" treatment this file already gives the word's ordinary
        // trailing space, just extended uniformly to cover the extra
        // word-spacing too — so that naive gap would always measure ~0
        // regardless of the declared value. The second word's absolute
        // `x` is where the extra space actually becomes observable.
        let second_word_x = |spacing_px: f32| -> f32 {
            let html = format!(r#"<html><body><p style="margin: 0; word-spacing: {spacing_px}px">foo bar</p></body></html>"#);
            let laid = measure_fixture(&html, 500.0);
            let run = find_mixed_inline(&laid).expect("nonzero word-spacing should route through the per-word path");
            let items = mixed_items(run);
            assert_eq!(items.len(), 2);
            match &items[1] {
                MixedPositioned::Word { x, .. } => *x,
                MixedPositioned::Image(_) => panic!("expected a word"),
            }
        };
        let x1_10 = second_word_x(10.0);
        let x1_30 = second_word_x(30.0);
        assert!(
            (x1_30 - x1_10 - 20.0).abs() < 0.5,
            "raising word-spacing from 10px to 30px should shift the second word exactly 20px further right (only the extra spacing differs between these two calls — the words themselves shape identically); got {x1_10} then {x1_30} (delta {})",
            x1_30 - x1_10
        );
    }

    #[test]
    fn word_spacing_does_not_shatter_a_short_paragraph_the_way_letter_spacing_did() {
        // Direct regression test against the exact failure shape that
        // broke the reverted `letter-spacing` attempt (see TODO.txt):
        // a short paragraph that trivially fits on one line at a wide
        // (900px) container shattered into one line per word (worse at
        // larger values) once that attempt's spacing crossed roughly
        // 2-3px, because it wired spacing into cosmic-text's own
        // `Attrs`/shaping and cosmic-text's *internal* wrap-width
        // computation got the word width wrong. `word-spacing` never
        // touches `Attrs` at all — only this hand-rolled path's own
        // position math changes — so the exact same scenario, at the
        // exact value (8px) that broke letter-spacing into 4 lines,
        // should still fit on one line here.
        for spacing_px in [2.0, 8.0] {
            let html = format!(
                r#"<html><body><p style="margin: 0; word-spacing: {spacing_px}px">Wide word spacing</p></body></html>"#
            );
            let laid = measure_fixture(&html, 900.0);
            let run = find_mixed_inline(&laid).expect("nonzero word-spacing should route through the per-word path");
            let lines = mixed_inline_line_count(run);
            assert_eq!(
                lines, 1,
                "a short 3-word paragraph at a 900px container should still fit on one line with {spacing_px}px word-spacing, not shatter into {lines} lines the way letter-spacing's own attempt did at comparable values"
            );
        }
    }

    #[test]
    fn word_spacing_does_not_break_realistic_multi_word_wrapping() {
        // The other half of the same reproduction: a longer, realistic
        // paragraph that genuinely needs to wrap across several lines at
        // a normal article width should still group multiple words per
        // line with a typical `word-spacing` value — not degrade toward
        // one word (or worse) per line the way letter-spacing's own bug
        // did at a real paragraph width.
        let paragraph = "The quick brown fox jumps over the lazy dog while the sun sets \
             behind the distant mountains slowly and quietly each evening";
        let word_count = paragraph.split_whitespace().count();
        let html = format!(r#"<html><body><p style="margin: 0; word-spacing: 3px">{paragraph}</p></body></html>"#);
        let laid = measure_fixture(&html, 600.0);
        let run = find_mixed_inline(&laid).expect("nonzero word-spacing should route through the per-word path");
        let lines = mixed_inline_line_count(run);
        assert!(lines >= 1, "sanity: should produce at least one line");
        assert!(
            lines * 3 < word_count,
            "a {word_count}-word paragraph wrapping into {lines} lines at a realistic 600px width should still average well over 3 words per line, not shatter down toward one word per line the way letter-spacing's own bug did"
        );
    }

    // ---------- v60: `text-decoration-line`/`-style`/`-color` longhands, `wavy`, `double` for line-through/overline ----------

    /// Same shape as v39's own `collect_decorations`, but also captures
    /// `decoration_style` alongside `text_decoration` — needed by this
    /// section's tests, which care about the *style* (solid/dashed/
    /// double/wavy) a span resolved to, not just which lines are on.
    fn collect_decoration_values(b: &LayoutBox, out: &mut Vec<(TextDecoration, DecorationStyle)>) {
        if let BoxContent::Inline(items) = &b.content {
            for item in items {
                if let InlineItem::Text(_, s) = item {
                    out.push((s.text_decoration, s.decoration_style));
                }
            }
        }
        if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                collect_decoration_values(c, out);
            }
        }
    }

    #[test]
    fn text_decoration_longhands_combine_like_the_shorthand() {
        // The core claim of this item's gap #1: `text-decoration-line`/
        // `-style`/`-color`, declared as three *separate* properties with
        // no `text-decoration` shorthand anywhere, should combine into
        // exactly the same effective decoration a single `text-decoration:
        // underline dashed #ff0000` shorthand declaration would produce.
        let longhand_doc = parse_html(
            r#"<html><body><span style="text-decoration-line: underline; text-decoration-style: dashed; text-decoration-color: #ff0000">x</span></body></html>"#,
        );
        let shorthand_doc = parse_html(
            r#"<html><body><span style="text-decoration: underline dashed #ff0000">x</span></body></html>"#,
        );
        let mut longhand_values = Vec::new();
        collect_decoration_values(&build_layout_tree(&longhand_doc, 1920.0), &mut longhand_values);
        let mut shorthand_values = Vec::new();
        collect_decoration_values(&build_layout_tree(&shorthand_doc, 1920.0), &mut shorthand_values);
        assert_eq!(longhand_values.len(), 1, "expected exactly one span: {:?}", longhand_values);
        assert_eq!(shorthand_values.len(), 1, "expected exactly one span: {:?}", shorthand_values);
        let (ld, ls) = longhand_values[0];
        let (sd, ss) = shorthand_values[0];
        assert_eq!(ls, ss, "the three longhands together should resolve to the same DecorationStyle as the shorthand");
        assert_eq!(ld.underline, sd.underline, "both should turn underline on");
        assert_eq!(ld.underline_color_opt, sd.underline_color_opt, "both should attach the same color to it");
        assert_eq!(ld.underline_color_opt, Some(TextColor::rgb(0xff, 0, 0)), "sanity: the color should actually be red, not just equal-to-each-other-and-wrong");
    }

    #[test]
    fn text_decoration_style_longhand_overrides_shorthands_style_on_the_same_element() {
        // A longhand declared *alongside* the shorthand on the very same
        // element should override just its own one component — real
        // CSS's "combine at used-value time" semantics, not an
        // all-or-nothing replace the way the shorthand's own cascade
        // already works against an *ambient* value.
        let doc = parse_html(
            r#"<html><body><span style="text-decoration: underline red; text-decoration-style: dashed">x</span></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
        let mut values = Vec::new();
        collect_decoration_values(&tree, &mut values);
        assert_eq!(values.len(), 1);
        let (deco, style) = values[0];
        assert_eq!(deco.underline, UnderlineStyle::Single, "the shorthand's own line should still apply");
        assert_eq!(deco.underline_color_opt, Some(TextColor::rgb(0xff, 0, 0)), "the shorthand's own color should still apply");
        assert_eq!(style, DecorationStyle::Dashed, "the longhand text-decoration-style should override the shorthand's own implicit `solid`");
    }

    #[test]
    fn text_decoration_color_longhand_alone_has_no_effect_without_any_line() {
        // `text-decoration-color` (or `-style`) declared with no line
        // active anywhere (no shorthand, no `-line` longhand, no ambient
        // decoration from an ancestor) must stay a no-op — same "don't
        // leave a stray color/style on an inactive line" rule
        // `parse_text_decoration` already enforces for the shorthand
        // itself, now also true across the three longhands combined.
        let doc = parse_html(r#"<html><body><span style="text-decoration-color: red; text-decoration-style: dashed">x</span></body></html>"#);
        let tree = build_layout_tree(&doc, 1920.0);
        let mut values = Vec::new();
        collect_decoration_values(&tree, &mut values);
        assert_eq!(values.len(), 1);
        let (deco, _style) = values[0];
        assert_eq!(deco, TextDecoration::new(), "no line was ever turned on, so color/style alone should paint nothing: {:?}", deco);
    }

    #[test]
    fn text_decoration_line_longhand_propagates_ambiently_like_the_shorthand() {
        // The combined *result* (not the raw longhand) is what threads on
        // as the new ambient value for children — a descendant that
        // declares none of these four properties should still see its
        // ancestor's longhand-only decoration, same ambient-propagation
        // shape the shorthand already had since v17/v33.
        let doc = parse_html(
            r#"<html><body><div style="text-decoration-line: underline"><div>nested</div></div></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
        let mut values = Vec::new();
        collect_decoration_values(&tree, &mut values);
        assert!(
            values.iter().any(|(d, _)| d.underline == UnderlineStyle::Single),
            "a nested block descendant should inherit the ambient `text-decoration-line: underline`: {:?}",
            values
        );
    }

    /// A minimal `Renderer` test double that just records every
    /// `rectangle` call verbatim — lets `paint_decoration_line` be
    /// exercised directly, with no real `cosmic_text::Buffer`/font
    /// shaping needed, the same way v33's own (since-deleted, per the
    /// since-reverted pre-v41 policy) test double worked.
    #[derive(Default)]
    struct CollectingRenderer {
        rects: Vec<(i32, i32, u32, u32, TextColor)>,
    }

    impl Renderer for CollectingRenderer {
        fn rectangle(&mut self, x: i32, y: i32, w: u32, h: u32, color: TextColor) {
            self.rects.push((x, y, w, h, color));
        }
        fn glyph(&mut self, _physical_glyph: cosmic_text::PhysicalGlyph, _color: TextColor) {}
    }

    #[test]
    fn wavy_decoration_produces_a_real_sine_wave_not_a_straight_line() {
        // The core claim of this item's gap #2: `Wavy` must paint a
        // genuine sine-wave pattern — not just "doesn't crash" and not
        // just "paints something" — verified by checking the painted
        // segments' own `y` actually rises *and* falls across the
        // stroke's width (a monotonic ramp would also vary, but isn't a
        // wave), unlike `Solid`, which paints one single flat rectangle.
        let mut renderer = CollectingRenderer::default();
        paint_decoration_line(&mut renderer, 0.0, 100.0, 60.0, 2.0, TextColor::rgb(0, 0, 0), DecorationStyle::Wavy);
        assert!(
            renderer.rects.len() > 4,
            "a wavy stroke across 60px should paint several small segments tracing the curve, not one rectangle: {:?}",
            renderer.rects
        );
        let ys: Vec<i32> = renderer.rects.iter().map(|r| r.1).collect();
        let min_y = *ys.iter().min().unwrap();
        let max_y = *ys.iter().max().unwrap();
        assert!(
            max_y - min_y >= 2,
            "a real sine-wave stroke should vary its y position across its width, not stay flat the way a Solid line's single rectangle does: observed y range {min_y}..{max_y}"
        );
        let mut rose = false;
        let mut fell = false;
        for i in 1..ys.len() {
            if ys[i] > ys[i - 1] {
                rose = true;
            }
            if ys[i] < ys[i - 1] {
                fell = true;
            }
        }
        assert!(rose && fell, "a sine wave should both rise and fall across its own extent, not move monotonically like a diagonal line: {:?}", ys);
    }

    #[test]
    fn solid_decoration_stays_perfectly_flat_unlike_wavy() {
        // The control case for the test just above — confirms the flat-
        // line assertion wavy is contrasted against is actually true of
        // `Solid`, not an assumption.
        let mut renderer = CollectingRenderer::default();
        paint_decoration_line(&mut renderer, 0.0, 100.0, 60.0, 2.0, TextColor::rgb(0, 0, 0), DecorationStyle::Solid);
        assert_eq!(renderer.rects.len(), 1, "Solid should paint exactly one rectangle spanning the whole width");
        assert_eq!(renderer.rects[0].1, 100, "Solid's one rectangle should sit at exactly the requested y, with no wave offset");
    }

    #[test]
    fn double_style_paints_two_parallel_solid_strokes_via_the_shared_primitive() {
        // `paint_decoration_line` is the one shared primitive `paint_
        // decorations` calls identically for underline/strikethrough/
        // overline (see its own docs) — proving `Double` paints two
        // separate `Solid` rectangles here, once, covers all three line
        // types at once; the two pixel tests below additionally confirm
        // it end-to-end through real text shaping for line-through/
        // overline specifically (this item's own gap #3).
        let mut renderer = CollectingRenderer::default();
        paint_decoration_line(&mut renderer, 0.0, 100.0, 60.0, 2.0, TextColor::rgb(255, 0, 0), DecorationStyle::Double);
        assert_eq!(renderer.rects.len(), 2, "Double should paint exactly two rectangles: {:?}", renderer.rects);
        assert_eq!(renderer.rects[0].1, 100, "first stroke at the requested y");
        assert_eq!(renderer.rects[1].1, 104, "second stroke offset by thickness (2) + an equal gap (2) = 4px below the first");
        for r in &renderer.rects {
            assert_eq!(r.4, TextColor::rgb(255, 0, 0));
            assert_eq!(r.2, 60, "each stroke should span the full requested width");
        }
    }

    /// Finds every contiguous run ("band") of exactly `color` pixels down
    /// column `x` of a painted `(pixels, win_w, win_h)` fixture — used by
    /// this section's end-to-end double-line-through/overline tests to
    /// confirm *two* separate bands paint, not one, proving `Double`
    /// reaches real text rendering (not just the primitive tested above
    /// in isolation).
    fn color_bands_in_column(pixels: &[u32], win_w: u32, win_h: u32, x: u32, color: u32) -> usize {
        let mut bands = 0;
        let mut in_band = false;
        for y in 0..win_h {
            let matches = pixels[(y * win_w + x) as usize] & 0x00FF_FFFF == color;
            if matches && !in_band {
                bands += 1;
            }
            in_band = matches;
        }
        bands
    }

    #[test]
    fn double_line_through_paints_two_distinct_strokes_end_to_end() {
        // This item's own gap #3: cosmic-text 0.19 has no double variant
        // for strikethrough at all (only underline) — before this item,
        // `text-decoration: line-through double` silently painted one
        // plain line. Verified with real text shaping/painting (`paint_
        // fixture`), not just the primitive-level test above, so this
        // actually proves the full pipeline (span style → `paint_
        // decorations` → `paint_decoration_line`) wires `Double` through
        // for strikethrough, not only underline.
        let (pixels, win_w, win_h) = paint_fixture(
            r#"<html><body style="margin:0"><p style="margin:0; font-size: 48px; text-decoration: line-through double red">HELLO</p></body></html>"#,
        );
        let bands = color_bands_in_column(&pixels, win_w, win_h, 10, 0x00FF_0000);
        assert_eq!(bands, 2, "a double line-through should paint two distinct red strokes down this column, not one");
    }

    #[test]
    fn double_overline_paints_two_distinct_strokes_end_to_end() {
        // Same gap, the other previously-unreachable line: cosmic-text
        // 0.19 has no double variant for overline either.
        let (pixels, win_w, win_h) = paint_fixture(
            r#"<html><body style="margin:0"><p style="margin:0; font-size: 48px; text-decoration: overline double red">HELLO</p></body></html>"#,
        );
        let bands = color_bands_in_column(&pixels, win_w, win_h, 10, 0x00FF_0000);
        assert_eq!(bands, 2, "a double overline should paint two distinct red strokes down this column, not one");
    }

    #[test]
    fn double_underline_still_paints_two_distinct_strokes_after_the_double_refactor() {
        // Regression guard for this item's refactor: doubling used to be
        // driven by cosmic-text's own native `UnderlineStyle::Double`
        // (set directly by the old `parse_text_decoration`), now it's
        // driven uniformly by `DecorationStyle::Double` for all three
        // lines instead (see `paint_decorations`'s own docs on why that's
        // safe) — confirms underline's own pre-existing v33 behavior
        // wasn't lost in that switch.
        let (pixels, win_w, win_h) = paint_fixture(
            r#"<html><body style="margin:0"><p style="margin:0; font-size: 48px; text-decoration: underline double red">HELLO</p></body></html>"#,
        );
        let bands = color_bands_in_column(&pixels, win_w, win_h, 10, 0x00FF_0000);
        assert_eq!(bands, 2, "a double underline should still paint two distinct red strokes, same as before this item's refactor");
    }

    // ---------- v61: extended `cursor` keyword set (TODO.txt autonomous pass) ----------

    fn find_first_span_cursor(b: &LayoutBox) -> Option<CursorStyle> {
        if let BoxContent::Inline(items) = &b.content {
            if let Some(InlineItem::Text(_, s)) = items.first() {
                return Some(s.cursor);
            }
        } else if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                if let Some(found) = find_first_span_cursor(c) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn every_cursor_keyword_parses_and_cascades_to_the_right_style() {
        // Covers the original two keywords (v32, re-added here since v32
        // predates v41's "keep tests permanently" policy and its own were
        // written-then-deleted) plus every keyword this item added — one
        // table-driven test through the real `style="cursor: ..."`
        // parsing path (`DeclParser`/`cursor_style_from_keyword`), not
        // just the keyword-mapping helper in isolation.
        let cases: &[(&str, CursorStyle)] = &[
            ("auto", CursorStyle::Auto),
            ("pointer", CursorStyle::Pointer),
            ("default", CursorStyle::Default),
            ("none", CursorStyle::None),
            ("context-menu", CursorStyle::ContextMenu),
            ("help", CursorStyle::Help),
            ("progress", CursorStyle::Progress),
            ("wait", CursorStyle::Wait),
            ("cell", CursorStyle::Cell),
            ("crosshair", CursorStyle::Crosshair),
            ("text", CursorStyle::Text),
            ("vertical-text", CursorStyle::VerticalText),
            ("alias", CursorStyle::Alias),
            ("copy", CursorStyle::Copy),
            ("move", CursorStyle::Move),
            ("no-drop", CursorStyle::NoDrop),
            ("not-allowed", CursorStyle::NotAllowed),
            ("grab", CursorStyle::Grab),
            ("grabbing", CursorStyle::Grabbing),
            ("e-resize", CursorStyle::EResize),
            ("n-resize", CursorStyle::NResize),
            ("ne-resize", CursorStyle::NeResize),
            ("nw-resize", CursorStyle::NwResize),
            ("s-resize", CursorStyle::SResize),
            ("se-resize", CursorStyle::SeResize),
            ("sw-resize", CursorStyle::SwResize),
            ("w-resize", CursorStyle::WResize),
            ("ew-resize", CursorStyle::EwResize),
            ("ns-resize", CursorStyle::NsResize),
            ("nesw-resize", CursorStyle::NeswResize),
            ("nwse-resize", CursorStyle::NwseResize),
            ("col-resize", CursorStyle::ColResize),
            ("row-resize", CursorStyle::RowResize),
            ("all-scroll", CursorStyle::AllScroll),
            ("zoom-in", CursorStyle::ZoomIn),
            ("zoom-out", CursorStyle::ZoomOut),
        ];
        for (keyword, expected) in cases {
            let html = format!(r#"<html><body><span style="cursor: {keyword}">x</span></body></html>"#);
            let doc = parse_html(&html);
            let tree = build_layout_tree(&doc, 1920.0);
            let cursor = find_first_span_cursor(&tree).unwrap_or_else(|| panic!("no span found for cursor: {keyword}"));
            assert_eq!(cursor, *expected, "cursor: {keyword} should cascade to {expected:?}, got {cursor:?}");
        }
    }

    #[test]
    fn unrecognized_cursor_keyword_is_dropped_not_guessed() {
        assert!(cursor_style_from_keyword("bogus-keyword-xyz").is_none());
    }

    #[test]
    fn cursor_inherits_through_a_nested_span_for_a_new_keyword() {
        // Same real-CSS-inheritance shape v32's own doc comment described
        // for `pointer`/`<a>` — exercised here with one of this item's new
        // keywords to confirm inheritance wasn't special-cased to just the
        // two original variants.
        let doc = parse_html(r#"<html><body><div style="cursor: wait"><span>inherited</span></div></body></html>"#);
        let tree = build_layout_tree(&doc, 1920.0);
        let cursor = find_first_span_cursor(&tree).expect("no span found");
        assert_eq!(cursor, CursorStyle::Wait, "a nested span with no cursor of its own should inherit the ancestor's wait");
    }

    #[test]
    fn to_winit_maps_new_keywords_onto_matching_cursor_icons() {
        // Direct `CursorStyle` -> `CursorIcon` mapping tests for a
        // representative spread of the new variants (every arm is a
        // literal 1:1 rename, so this isn't exhaustive over all 30+, just
        // enough to prove the match arms aren't transposed/miswired).
        assert_eq!(CursorStyle::Default.to_winit(), Some(CursorIcon::Default));
        assert_eq!(CursorStyle::Text.to_winit(), Some(CursorIcon::Text));
        assert_eq!(CursorStyle::Wait.to_winit(), Some(CursorIcon::Wait));
        assert_eq!(CursorStyle::Progress.to_winit(), Some(CursorIcon::Progress));
        assert_eq!(CursorStyle::Crosshair.to_winit(), Some(CursorIcon::Crosshair));
        assert_eq!(CursorStyle::Move.to_winit(), Some(CursorIcon::Move));
        assert_eq!(CursorStyle::Grab.to_winit(), Some(CursorIcon::Grab));
        assert_eq!(CursorStyle::Grabbing.to_winit(), Some(CursorIcon::Grabbing));
        assert_eq!(CursorStyle::NotAllowed.to_winit(), Some(CursorIcon::NotAllowed));
        assert_eq!(CursorStyle::Help.to_winit(), Some(CursorIcon::Help));
        assert_eq!(CursorStyle::ZoomIn.to_winit(), Some(CursorIcon::ZoomIn));
        assert_eq!(CursorStyle::ZoomOut.to_winit(), Some(CursorIcon::ZoomOut));
        assert_eq!(CursorStyle::NResize.to_winit(), Some(CursorIcon::NResize));
        assert_eq!(CursorStyle::EwResize.to_winit(), Some(CursorIcon::EwResize));
        assert_eq!(CursorStyle::ColResize.to_winit(), Some(CursorIcon::ColResize));
        assert_eq!(CursorStyle::RowResize.to_winit(), Some(CursorIcon::RowResize));
        assert_eq!(CursorStyle::AllScroll.to_winit(), Some(CursorIcon::AllScroll));
    }

    #[test]
    fn cursor_none_maps_to_hide_not_to_any_icon() {
        // The one real keyword with no `CursorIcon` equivalent at all —
        // `to_winit()` signals "hide the cursor" via `None`, which the
        // live `CursorMoved` handler turns into `window.set_cursor_
        // visible(false)` instead of `window.set_cursor(..)`.
        assert_eq!(CursorStyle::None.to_winit(), None, "cursor: none should signal \"hide,\" not any particular icon");
    }

    fn measure_fixture_with_cache(html: &str, available_width: f32) -> (Laid, ShapeCache) {
        let doc = parse_html(html);
        let tree = build_layout_tree(&doc, 1920.0);
        let mut font_system = new_font_system();
        let mut cache = ShapeCache::new();
        let mut next_id = 0usize;
        let mut incomplete = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure(
            &tree, &mut font_system, 0.0, 0.0, available_width, None, &mut next_id, &mut cache, deadline,
            &mut incomplete, &mut Vec::new(), &Floats::default(), None,
        );
        (laid, cache)
    }

    /// Finds the pixel-space center of the first glyph whose own per-span
    /// cursor (via `glyph.metadata`, the same lookup `hit_test_cursor`
    /// itself does) matches `want` — lets a test drive `hit_test_cursor`
    /// at a real, specific coordinate instead of guessing one.
    fn find_glyph_center(laid: &Laid, cache: &ShapeCache, want: CursorStyle) -> Option<(f32, f32)> {
        match &laid.content {
            LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
                children.iter().find_map(|c| find_glyph_center(c, cache, want))
            }
            LaidContent::Text(id) => {
                let entry = cache.get(id)?;
                let guard = entry.lock().unwrap();
                let (buffer, _, _, _, _, cursors, _, _) = &*guard;
                let content_x = laid.x + laid.style.border_width + laid.style.padding.left.px();
                let content_y = laid.y + laid.style.border_width + laid.style.padding.top.px();
                for run in buffer.layout_runs() {
                    for glyph in run.glyphs {
                        if cursors.get(glyph.metadata).copied() == Some(want) {
                            return Some((content_x + glyph.x + glyph.w / 2.0, content_y + run.line_top + run.line_height / 2.0));
                        }
                    }
                }
                None
            }
            LaidContent::Image(_) | LaidContent::MixedInline(_) => None,
        }
    }

    #[test]
    fn hit_test_cursor_generalizes_to_a_new_keyword_at_glyph_precision() {
        // The requirement this item's task explicitly called out: v32's
        // `hit_test_cursor` is keyed on "which `CursorStyle` does this
        // glyph's span resolve to," not hardcoded to `Pointer` — so a
        // brand-new keyword (`grab`, picked arbitrarily, not special) needs
        // zero changes to `hit_test_cursor` itself to hit-test correctly at
        // real glyph precision, same diligence v32's own pipeline test used
        // for `Pointer` vs. `Auto`.
        let (laid, cache) = measure_fixture_with_cache(
            r#"<html><body><p><span style="cursor: grab">drag me</span> plain text</p></body></html>"#,
            400.0,
        );
        let (gx, gy) = find_glyph_center(&laid, &cache, CursorStyle::Grab).expect("no grab-styled glyph found");
        let (ax, ay) = find_glyph_center(&laid, &cache, CursorStyle::Auto).expect("no plain-text glyph found");
        let scroll_offsets = HashMap::new();
        assert_eq!(
            hit_test_cursor(&laid, gx, gy, &cache, &scroll_offsets, 0.0, 600.0).0,
            CursorStyle::Grab,
            "a glyph inside the styled span should report Grab"
        );
        assert_eq!(
            hit_test_cursor(&laid, ax, ay, &cache, &scroll_offsets, 0.0, 600.0).0,
            CursorStyle::Auto,
            "an adjacent plain-text glyph should still report Auto"
        );
    }

    // ---------- v62: `overflow: scroll`/`auto` ----------

    /// Depth-first search for the first box with its own `ScrollInfo`
    /// (an `overflow: scroll`/`auto` box) — the scroll analogue of
    /// `find_laid_by_position`, just above.
    fn find_laid_with_scroll(laid: &Laid) -> Option<&Laid> {
        if laid.scroll.is_some() {
            return Some(laid);
        }
        if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
            for c in children {
                if let Some(found) = find_laid_with_scroll(c) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Finds a box by its own exact laid-out height — used instead of
    /// `find_laid_by_margin_top` wherever a fixture's boxes all happen to
    /// share the real CSS initial margin (`0`, e.g. every plain `<div>`
    /// here), which would make that finder match the very first box in
    /// the whole tree (even the synthetic document-root wrapper) instead
    /// of the one a test actually means.
    fn find_laid_by_height(laid: &Laid, target: f32) -> Option<&Laid> {
        if laid.h == target {
            return Some(laid);
        }
        if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
            for c in children {
                if let Some(found) = find_laid_by_height(c, target) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Same as `find_laid_with_scroll`, but collects every match instead
    /// of stopping at the first — needed by tests with more than one
    /// scrollable region on the same page (e.g. the wheel-routing test),
    /// to tell them apart by their own distinct `x`/`y`/`id`.
    fn find_all_laid_with_scroll<'a>(laid: &'a Laid, out: &mut Vec<&'a Laid>) {
        if laid.scroll.is_some() {
            out.push(laid);
        }
        if let LaidContent::Blocks(children) | LaidContent::TableRow(children) = &laid.content {
            for c in children {
                find_all_laid_with_scroll(c, out);
            }
        }
    }

    #[test]
    fn overflow_scroll_assigns_a_scroll_id_and_computes_content_vs_client_height() {
        let laid = measure_fixture(
            r#"<html><body><div style="overflow: scroll; height: 50px;">
                <p style="margin: 0;">one</p>
                <p style="margin: 0;">two</p>
                <p style="margin: 0;">three</p>
                <p style="margin: 0;">four</p>
            </div></body></html>"#,
            400.0,
        );
        let scroll_box = find_laid_with_scroll(&laid).expect("overflow: scroll box should get its own ScrollInfo");
        let info = scroll_box.scroll.expect("already found via find_laid_with_scroll");
        assert_eq!(info.client_h, 50.0, "client_h should be this box's own clamped content-box height");
        assert!(
            info.content_h > info.client_h,
            "four 20px-tall paragraphs (80px) should overflow a 50px client height: content_h={}, client_h={}",
            info.content_h,
            info.client_h
        );
        assert_eq!(
            info.max_scroll(),
            info.content_h - info.client_h,
            "max_scroll should be exactly the overflowing amount"
        );
    }

    #[test]
    fn overflow_auto_only_scrollable_when_content_actually_overflows() {
        let fits = measure_fixture(
            r#"<html><body><div style="overflow: auto; height: 200px;"><p style="margin: 0;">one short line</p></div></body></html>"#,
            400.0,
        );
        let fits_box = find_laid_with_scroll(&fits).expect("overflow: auto box should still get a ScrollInfo");
        assert_eq!(
            fits_box.scroll.unwrap().max_scroll(),
            0.0,
            "content shorter than the box's own height shouldn't be scrollable at all"
        );

        let overflows = measure_fixture(
            r#"<html><body><div style="overflow: auto; height: 20px;">
                <p style="margin: 0;">one</p>
                <p style="margin: 0;">two</p>
                <p style="margin: 0;">three</p>
            </div></body></html>"#,
            400.0,
        );
        let overflow_box = find_laid_with_scroll(&overflows).expect("overflow: auto box should get a ScrollInfo");
        assert!(
            overflow_box.scroll.unwrap().max_scroll() > 0.0,
            "three 20px paragraphs inside a 20px-tall box should actually overflow and be scrollable"
        );
    }

    #[test]
    fn scroll_offset_clips_and_translates_painted_content() {
        // No inter-tag whitespace — same reason every other exact-pixel
        // test in this file avoids it (a stray text node between tags
        // would become its own anonymous inline box and shift things).
        let laid = measure_fixture(
            concat!(
                r#"<html><body><div style="overflow: scroll; height: 50px; width: 100px; background-color: blue;">"#,
                r#"<div style="background-color: red; height: 20px; margin-top: 100px;"></div>"#,
                "</div></body></html>",
            ),
            400.0,
        );
        let scroll_box = find_laid_with_scroll(&laid).expect("overflow: scroll box not found");
        let info = scroll_box.scroll.expect("already confirmed Some above");
        let max_scroll = info.max_scroll();
        assert!(max_scroll > 0.0, "the red child (100px margin-top + 20px height) should overflow the 50px box");

        let mut font_system = new_font_system();
        let mut swash_cache = SwashCache::new();
        let mut cache = ShapeCache::new();
        let (win_w, win_h) = (200u32, 200u32);

        // At the default (no entry at all, i.e. `0.0`) scroll offset, the
        // red child sits far below the 50px-tall visible window — fully
        // clipped. The outer box's own blue background is unaffected by
        // its *own* overflow (an element's `overflow` clips its content,
        // never itself — same rule v31/v40 already established).
        let mut pixels_unscrolled = vec![0u32; (win_w * win_h) as usize];
        paint(
            &laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels_unscrolled, win_w,
            win_h, 0.0, None,
        );
        assert!(!pixels_unscrolled.contains(&0xFF0000), "the red child should be clipped away before any scrolling");
        assert!(pixels_unscrolled.contains(&0x0000FF), "the blue outer box should still paint regardless of its own scroll");

        // Scrolled all the way to `max_scroll`, the red child's bottom
        // edge lines up with the box's own bottom edge — now visible.
        let mut scroll_offsets = HashMap::new();
        scroll_offsets.insert(info.id, max_scroll);
        let mut pixels_scrolled = vec![0u32; (win_w * win_h) as usize];
        paint(
            &laid, &mut font_system, &mut swash_cache, &mut cache, &scroll_offsets, &mut pixels_scrolled, win_w,
            win_h, 0.0, None,
        );
        assert!(
            pixels_scrolled.contains(&0xFF0000),
            "scrolling to max_scroll should bring the red child into the clipped view"
        );
        assert!(pixels_scrolled.contains(&0x0000FF), "the blue outer box should still paint after scrolling too");
    }

    #[test]
    fn wheel_routing_picks_the_scroll_region_under_the_cursor_not_the_page() {
        // Two independent (sibling, not nested) `overflow` regions plus
        // one plain, non-scrollable paragraph — enough to confirm
        // `hit_test_scroll_region` tells the two scrollable regions
        // apart by position and correctly falls through to `None`
        // (→ the page's own top-level scroll) outside both.
        let laid = measure_fixture(
            concat!(
                r#"<html><body>"#,
                r#"<div style="overflow: scroll; height: 50px; width: 100px;">first region text</div>"#,
                r#"<div style="overflow: auto; height: 50px; width: 100px;">second region needs more text than fits so it overflows its own fifty pixel height for real this time yes</div>"#,
                r#"<p style="height: 30px; margin: 0;">not scrollable at all</p>"#,
                "</body></html>",
            ),
            400.0,
        );
        let mut regions = Vec::new();
        find_all_laid_with_scroll(&laid, &mut regions);
        assert_eq!(regions.len(), 2, "both overflow regions should each get their own ScrollInfo");
        let (first, second) = (regions[0], regions[1]);
        assert_ne!(first.scroll.unwrap().id, second.scroll.unwrap().id, "each region should get a distinct id");

        let scroll_offsets = HashMap::new();
        let viewport_h = 600.0;

        // A point inside the first region routes to it, not the second.
        let (fx, fy) = (first.x + 5.0, first.y + 5.0);
        let hit_first = hit_test_scroll_region(&laid, fx, fy, &scroll_offsets, 0.0, viewport_h);
        assert_eq!(
            hit_first.map(|i| i.id),
            Some(first.scroll.unwrap().id),
            "a point inside the first scrollable region should route to it"
        );

        // A point inside the second region routes to *that* one instead.
        let (sx, sy) = (second.x + 5.0, second.y + 5.0);
        let hit_second = hit_test_scroll_region(&laid, sx, sy, &scroll_offsets, 0.0, viewport_h);
        assert_eq!(
            hit_second.map(|i| i.id),
            Some(second.scroll.unwrap().id),
            "a point inside the second scrollable region should route to it, not the first"
        );

        // A point over the plain, non-scrollable `<p>` falls through to
        // `None` — the caller's own cue to fall back to the page's own
        // top-level scroll instead. Found by its own distinctive 30px
        // height (chosen to not collide with either 50px scroll region,
        // or with the root/html/body wrapper boxes' own, much taller,
        // whole-document heights).
        let p = find_laid_by_height(&laid, 30.0).expect("the plain, non-scrollable <p> should be findable by its own height");
        let (px, py) = (p.x + 5.0, p.y + 5.0);
        assert!(
            px != fx || py != fy,
            "sanity: the <p>'s own point shouldn't accidentally coincide with the first region's"
        );
        let hit_none = hit_test_scroll_region(&laid, px, py, &scroll_offsets, 0.0, viewport_h);
        assert!(
            hit_none.is_none(),
            "a point over plain, non-scrollable content should fall through to the page's own scroll"
        );
    }

    #[test]
    fn scrollbar_paints_for_scroll_even_when_not_overflowing_but_auto_only_paints_when_it_does() {
        let mut font_system = new_font_system();
        let mut swash_cache = SwashCache::new();
        let mut cache = ShapeCache::new();
        // Taller than any one fixture's own box (body's 8px margin +
        // up to 200px box height) so the scan below never runs past the
        // buffer's own bounds.
        let (win_w, win_h) = (200u32, 260u32);
        // A translucent black-on-white pixel near the box's own right
        // edge (within `TRACK_W` of it) is the scrollbar's own track or
        // thumb — nothing else in this fixture paints anything but white
        // background and (deliberately) no text color near that edge,
        // so any non-white pixel there is unambiguously the indicator.
        let has_scrollbar_pixel = |pixels: &[u32], box_right: f32, box_top: f32, box_bottom: f32, win_w: u32| {
            let x = (box_right - 4.0) as u32;
            (box_top as u32..box_bottom as u32).any(|y| pixels[(y * win_w + x) as usize] != 0x00FFFFFF)
        };

        // `overflow: scroll` with content that fits entirely — real CSS
        // `scroll` always shows a scrollbar regardless.
        let scroll_fits = measure_fixture(
            r#"<html><body><div style="overflow: scroll; height: 200px; width: 100px;">short</div></body></html>"#,
            400.0,
        );
        let scroll_box = find_laid_with_scroll(&scroll_fits).expect("overflow: scroll box not found");
        let mut pixels = vec![0x00FFFFFFu32; (win_w * win_h) as usize];
        paint(
            &scroll_fits, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w,
            win_h, 0.0, None,
        );
        assert!(
            has_scrollbar_pixel(&pixels, scroll_box.x + scroll_box.w, scroll_box.y, scroll_box.y + scroll_box.h, win_w),
            "overflow: scroll should always paint a scrollbar indicator, even when nothing overflows"
        );

        // `overflow: auto` with content that fits — no scrollbar at all.
        let auto_fits = measure_fixture(
            r#"<html><body><div style="overflow: auto; height: 200px; width: 100px;">short</div></body></html>"#,
            400.0,
        );
        let auto_fits_box = find_laid_with_scroll(&auto_fits).expect("overflow: auto box not found");
        let mut pixels = vec![0x00FFFFFFu32; (win_w * win_h) as usize];
        paint(
            &auto_fits, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w, win_h,
            0.0, None,
        );
        assert!(
            !has_scrollbar_pixel(&pixels, auto_fits_box.x + auto_fits_box.w, auto_fits_box.y, auto_fits_box.y + auto_fits_box.h, win_w),
            "overflow: auto shouldn't paint a scrollbar at all when nothing overflows"
        );

        // `overflow: auto` with content that genuinely overflows — now a
        // scrollbar should appear, same as `scroll`.
        let auto_overflows = measure_fixture(
            concat!(
                r#"<html><body><div style="overflow: auto; height: 20px; width: 100px;">"#,
                r#"<p style="margin: 0;">one</p><p style="margin: 0;">two</p><p style="margin: 0;">three</p>"#,
                "</div></body></html>",
            ),
            400.0,
        );
        let auto_overflow_box = find_laid_with_scroll(&auto_overflows).expect("overflow: auto box not found");
        let mut pixels = vec![0x00FFFFFFu32; (win_w * win_h) as usize];
        paint(
            &auto_overflows, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, win_w,
            win_h, 0.0, None,
        );
        assert!(
            has_scrollbar_pixel(
                &pixels, auto_overflow_box.x + auto_overflow_box.w, auto_overflow_box.y,
                auto_overflow_box.y + auto_overflow_box.h, win_w
            ),
            "overflow: auto should paint a scrollbar once its content actually overflows"
        );
    }

    // ---------- v64: `@media` (min-width/max-width, and/comma lists, print, viewport resize) ----------

    /// Walks the tree for the first box whose own resolved style declares
    /// a `background-color` — the simplest single signal to check whether
    /// a `@media`-wrapped rule actually applied or not, reused by every
    /// test below instead of a bespoke finder per test.
    fn find_box_with_background(b: &LayoutBox) -> Option<TextColor> {
        if b.style.background_color.is_some() {
            return b.style.background_color;
        }
        if let BoxContent::Blocks(children) = &b.content {
            for c in children {
                if let Some(found) = find_box_with_background(c) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn media_min_width_applies_only_at_or_above_the_breakpoint() {
        let doc = parse_html(
            r#"<html><body><style>@media (min-width: 600px) { .box { background-color: #ff0000; } }</style><div class="box">x</div></body></html>"#,
        );
        let wide = build_layout_tree(&doc, 800.0);
        assert_eq!(
            find_box_with_background(&wide),
            Some(TextColor::rgb(0xff, 0, 0)),
            "a min-width: 600px rule should apply at an 800px viewport"
        );
        let narrow = build_layout_tree(&doc, 400.0);
        assert_eq!(
            find_box_with_background(&narrow),
            None,
            "a min-width: 600px rule should NOT apply at a 400px viewport"
        );
        // Exactly at the breakpoint counts as matching (min-width is
        // inclusive, real CSS `>=` semantics).
        let exact = build_layout_tree(&doc, 600.0);
        assert_eq!(
            find_box_with_background(&exact),
            Some(TextColor::rgb(0xff, 0, 0)),
            "min-width: 600px should match exactly at 600px (inclusive)"
        );
    }

    #[test]
    fn media_max_width_applies_only_at_or_below_the_breakpoint() {
        let doc = parse_html(
            r#"<html><body><style>@media (max-width: 600px) { .box { background-color: #00ff00; } }</style><div class="box">x</div></body></html>"#,
        );
        let narrow = build_layout_tree(&doc, 400.0);
        assert_eq!(
            find_box_with_background(&narrow),
            Some(TextColor::rgb(0, 0xff, 0)),
            "a max-width: 600px rule should apply at a 400px viewport"
        );
        let wide = build_layout_tree(&doc, 800.0);
        assert_eq!(
            find_box_with_background(&wide),
            None,
            "a max-width: 600px rule should NOT apply at an 800px viewport — this is the exact inverse of min-width, confirming the comparison direction isn't flipped"
        );
    }

    #[test]
    fn media_and_requires_every_feature_to_hold() {
        // `screen and (min-width: 600px)` — both the type and the width
        // feature must hold; screen always matches here, so this is
        // really exercising that `and` doesn't silently drop the width
        // condition (a bug that would make this behave like a bare
        // `screen` query with no width gating at all).
        let doc = parse_html(
            r#"<html><body><style>@media screen and (min-width: 600px) { .box { background-color: #0000ff; } }</style><div class="box">x</div></body></html>"#,
        );
        let wide = build_layout_tree(&doc, 800.0);
        assert_eq!(find_box_with_background(&wide), Some(TextColor::rgb(0, 0, 0xff)));
        let narrow = build_layout_tree(&doc, 400.0);
        assert_eq!(
            find_box_with_background(&narrow),
            None,
            "`screen and (min-width: 600px)` must fail at 400px even though `screen` alone would match"
        );
    }

    #[test]
    fn media_comma_list_is_or_either_branch_matching_is_enough() {
        // `(min-width: 900px), (max-width: 300px)` — two mutually
        // exclusive branches at any viewport between them; a width of
        // 500px should match NEITHER, confirming comma is OR (not
        // silently treated as AND, which would make this never match at
        // any width, or silently treated as the same branch twice).
        let doc = parse_html(
            r#"<html><body><style>@media (min-width: 900px), (max-width: 300px) { .box { background-color: #ff00ff; } }</style><div class="box">x</div></body></html>"#,
        );
        let neither = build_layout_tree(&doc, 500.0);
        assert_eq!(find_box_with_background(&neither), None, "500px matches neither comma branch");
        let first_branch = build_layout_tree(&doc, 1000.0);
        assert_eq!(
            find_box_with_background(&first_branch),
            Some(TextColor::rgb(0xff, 0, 0xff)),
            "1000px should match the first (min-width: 900px) branch"
        );
        let second_branch = build_layout_tree(&doc, 200.0);
        assert_eq!(
            find_box_with_background(&second_branch),
            Some(TextColor::rgb(0xff, 0, 0xff)),
            "200px should match the second (max-width: 300px) branch"
        );
    }

    #[test]
    fn media_print_never_matches_regardless_of_width() {
        // scratchy has no print/paged-media concept at all — `@media
        // print` is intentionally always inert, a documented design
        // choice (see TODO.txt), not a gap. Checked at a width that
        // would satisfy the rule's own min-width if the type weren't
        // print, to make sure `print` itself is what's rejecting it,
        // not the width condition.
        let doc = parse_html(
            r#"<html><body><style>@media print and (min-width: 0px) { .box { background-color: #123456; } }</style><div class="box">x</div></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
        assert_eq!(find_box_with_background(&tree), None, "@media print should never match — scratchy has no print concept");
    }

    #[test]
    fn media_unsupported_calc_feature_never_applies_without_crashing() {
        // This project has no `calc()` arithmetic evaluator anywhere
        // (confirmed by grep before implementing this) — a media
        // feature using it must fail to parse into a real constraint and
        // safely never match, the same "unsupported value drops" policy
        // used throughout this file, rather than crashing the parser or
        // (the dangerous alternative) silently matching unconditionally.
        let doc = parse_html(
            r#"<html><body><style>@media (max-width: calc(1120px - 1px)) { .box { background-color: #abcdef; } }</style><div class="box">x</div></body></html>"#,
        );
        let narrow = build_layout_tree(&doc, 50.0);
        assert_eq!(
            find_box_with_background(&narrow),
            None,
            "an unparseable calc() feature must never match, even at a width a correct calc() evaluation would have satisfied"
        );
        let wide = build_layout_tree(&doc, 5000.0);
        assert_eq!(find_box_with_background(&wide), None, "and must still never match at a width nothing would satisfy either — not a crash");
    }

    #[test]
    fn media_rule_safely_coexists_with_an_unconditioned_rule() {
        // A plain, non-`@media` rule (empty `media` list) must keep
        // applying regardless of viewport width, unaffected by this
        // item's own changes — the lowest-risk possible regression
        // check for "did wiring viewport width into matching_
        // declarations somehow make ordinary rules width-sensitive too."
        let doc = parse_html(
            r#"<html><body><style>.always { background-color: #111111; }</style><div class="always">x</div></body></html>"#,
        );
        let narrow = build_layout_tree(&doc, 10.0);
        assert_eq!(find_box_with_background(&narrow), Some(TextColor::rgb(0x11, 0x11, 0x11)));
        let wide = build_layout_tree(&doc, 10000.0);
        assert_eq!(find_box_with_background(&wide), Some(TextColor::rgb(0x11, 0x11, 0x11)));
    }

    #[test]
    fn media_rematches_on_simulated_resize_with_no_new_mechanism() {
        // `build_layout_tree` is the exact function `App::redraw` already
        // reruns whenever the live window width changes — calling it
        // twice with two different widths from the same parsed document
        // is a faithful simulation of that real resize path, not just a
        // unit-level check of `MediaQuery::matches` in isolation.
        let doc = parse_html(
            r#"<html><body><style>@media (min-width: 500px) { .box { background-color: #fedcba; } }</style><div class="box">x</div></body></html>"#,
        );
        let before_resize = build_layout_tree(&doc, 300.0);
        assert_eq!(find_box_with_background(&before_resize), None);
        let after_resize = build_layout_tree(&doc, 700.0);
        assert_eq!(
            find_box_with_background(&after_resize),
            Some(TextColor::rgb(0xfe, 0xdc, 0xba)),
            "the same document, rebuilt at a wider width, must pick up a min-width rule it didn't before"
        );
    }

    #[test]
    fn prefers_color_scheme_defaults_to_light_never_dark() {
        // scratchy has no OS theme detection — documented fixed default:
        // `light` always matches, `dark` never does.
        let doc = parse_html(
            r#"<html><body><style>
                @media (prefers-color-scheme: light) { .box { background-color: #00ffff; } }
                @media (prefers-color-scheme: dark) { .other { background-color: #ff0000; } }
            </style><div class="box">x</div><div class="other">y</div></body></html>"#,
        );
        let tree = build_layout_tree(&doc, 1920.0);
        assert_eq!(
            find_box_with_background(&tree),
            Some(TextColor::rgb(0, 0xff, 0xff)),
            "prefers-color-scheme: light should match (the only color found should be .box's cyan, not .other's red)"
        );
    }

    // ---------- v65: link navigation ----------

    // -- href resolution against the page's own <base href> --

    #[test]
    fn resolve_href_relative_joins_against_base() {
        let base = Url::parse("https://example.com/articles/foo").unwrap();
        let resolved = resolve_href(Some(&base), "bar").unwrap();
        assert_eq!(resolved.as_str(), "https://example.com/articles/bar");
    }

    #[test]
    fn resolve_href_absolute_ignores_base_entirely() {
        // Real `Url::join` behavior, same as a real browser: an already-
        // absolute href resolves to itself regardless of what base the
        // page declares.
        let base = Url::parse("https://example.com/articles/foo").unwrap();
        let resolved = resolve_href(Some(&base), "https://other.example/bar").unwrap();
        assert_eq!(resolved.as_str(), "https://other.example/bar");
    }

    #[test]
    fn resolve_href_relative_with_no_base_is_a_clear_error() {
        let err = resolve_href(None, "relative/path").unwrap_err();
        assert!(
            err.contains("no <base href>"),
            "error should explain there's nothing to resolve a relative link against: {err}"
        );
    }

    #[test]
    fn resolve_href_absolute_with_no_base_still_resolves() {
        // No `<base>` on the page at all doesn't block an *already*-
        // absolute href — only a relative one has nothing to join
        // against.
        let resolved = resolve_href(None, "https://example.com/standalone").unwrap();
        assert_eq!(resolved.as_str(), "https://example.com/standalone");
    }

    #[test]
    fn find_base_href_finds_a_real_base_tag() {
        let doc = parse_html(
            r#"<html><head><base href="https://example.com/articles/"></head><body>hi</body></html>"#,
        );
        assert_eq!(find_base_href(&doc), Some("https://example.com/articles/".to_string()));
    }

    #[test]
    fn find_base_href_is_none_when_the_page_has_no_base_tag() {
        let doc = parse_html("<html><head></head><body>hi</body></html>");
        assert_eq!(find_base_href(&doc), None);
    }

    // -- v68: a fetched page with no <base> of its own falls back to the
    // real URL it was fetched from, instead of leaving base_url at None
    // and breaking every relative link on that page (found live — see
    // PROMPTS.md's own v68 entry) --

    #[test]
    fn resolve_page_base_url_prefers_the_pages_own_base_tag_when_present() {
        let doc = parse_html(r#"<html><head><base href="https://example.com/real/"></head><body>hi</body></html>"#);
        let fetched_url = Url::parse("https://example.com/different-url").unwrap();
        assert_eq!(resolve_page_base_url(&doc, fetched_url).as_str(), "https://example.com/real/");
    }

    #[test]
    fn resolve_page_base_url_falls_back_to_the_fetched_url_when_the_page_has_no_base_tag() {
        // The exact real-world shape that broke live: a page with no
        // <base> at all (the common case) must still let a relative —
        // including a same-page fragment, e.g. MediaWiki's own
        // "#bodyContent" skip-link — link resolve, by falling back to
        // wherever the page was actually fetched from.
        let doc = parse_html("<html><head></head><body><a href=\"#bodyContent\">skip</a></body></html>");
        let fetched_url = Url::parse("https://en.wikipedia.org/wiki/Example").unwrap();
        let base = resolve_page_base_url(&doc, fetched_url.clone());
        assert_eq!(base, fetched_url);
        // And confirms the actual downstream effect: a relative href on
        // this page now resolves at all, instead of erroring with "no
        // <base href> on this page to resolve it against".
        assert_eq!(
            resolve_href(Some(&base), "#bodyContent").unwrap().as_str(),
            "https://en.wikipedia.org/wiki/Example#bodyContent"
        );
    }

    // -- click-vs-drag distinction --

    #[test]
    fn is_click_accepts_a_tiny_move_between_down_and_up() {
        // Ordinary hand tremor between a real mouse-down and mouse-up
        // over the same link shouldn't be mistaken for a drag.
        assert!(is_click((100.0, 100.0), (102.0, 101.0), CLICK_DRAG_THRESHOLD_PX));
    }

    #[test]
    fn is_click_rejects_a_real_drag() {
        assert!(!is_click((100.0, 100.0), (260.0, 100.0), CLICK_DRAG_THRESHOLD_PX));
    }

    #[test]
    fn is_click_boundary_is_inclusive() {
        // Exactly at the threshold distance still counts as a click, not
        // a drag — consistent with this file's other boundary-inclusive
        // conventions (see e.g. `@media`'s own min-width tests).
        assert!(is_click((0.0, 0.0), (CLICK_DRAG_THRESHOLD_PX, 0.0), CLICK_DRAG_THRESHOLD_PX));
    }

    // -- href recovered at glyph precision (reuses `cursor`'s own v32/
    // v61 hit-testing machinery — same `hit_test_cursor` function, now
    // returning the href half of its result too) --

    #[test]
    fn hit_test_cursor_recovers_the_real_href_of_a_clicked_link() {
        // `default.css` already declares `a { cursor: pointer }` (v32),
        // so `find_glyph_center`'s existing `CursorStyle::Pointer` lookup
        // doubles as "find the link's own glyph" here with no new test
        // helper needed.
        let (laid, cache) = measure_fixture_with_cache(
            r#"<html><body><p><a href="/page">link text</a> plain text</p></body></html>"#,
            400.0,
        );
        let (lx, ly) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("no link glyph found");
        let (px, py) = find_glyph_center(&laid, &cache, CursorStyle::Auto).expect("no plain-text glyph found");
        let scroll_offsets = HashMap::new();
        assert_eq!(
            hit_test_cursor(&laid, lx, ly, &cache, &scroll_offsets, 0.0, 600.0).1,
            Some("/page"),
            "a glyph inside the link should report its real href"
        );
        assert_eq!(
            hit_test_cursor(&laid, px, py, &cache, &scroll_offsets, 0.0, 600.0).1,
            None,
            "an adjacent plain-text glyph should report no href at all"
        );
    }

    #[test]
    fn href_ambiently_propagates_into_a_nested_element_with_no_href_of_its_own() {
        // Not a CSS property (see `SpanStyle::href`'s own docs), so this
        // isn't real CSS inheritance — but a `<b>` nested inside a real
        // `<a href>`, with no `href` of its own, still needs to report
        // the ancestor link's href, the same ambient-propagation shape
        // `background-color`/`text-decoration` already use.
        let (laid, cache) = measure_fixture_with_cache(
            r#"<html><body><p><a href="/x"><b>bold link</b></a></p></body></html>"#,
            400.0,
        );
        let (bx, by) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("no link glyph found");
        let scroll_offsets = HashMap::new();
        assert_eq!(hit_test_cursor(&laid, bx, by, &cache, &scroll_offsets, 0.0, 600.0).1, Some("/x"));
    }

    #[test]
    fn an_anchor_with_no_href_is_not_a_link() {
        // A fragment-target `<a name="...">`/`<a>` with no `href` at all
        // is real HTML, not a link — same distinction `OurPseudoClass::
        // Link`'s own `:link` matching already makes (v37). Note this
        // doesn't mean no cursor: `default.css`'s `a { cursor: pointer
        // }` rule is a plain tag selector, not `:link`-scoped, so it
        // still applies here — `find_glyph_center` has to look for
        // `Pointer`, same as a real link, even though `href` itself must
        // come back `None`. This is exactly why `href` needs its own
        // tracking rather than piggybacking on `cursor`.
        let (laid, cache) = measure_fixture_with_cache(
            r#"<html><body><p><a>not a link</a></p></body></html>"#,
            400.0,
        );
        let (x, y) = find_glyph_center(&laid, &cache, CursorStyle::Pointer).expect("no glyph found");
        let scroll_offsets = HashMap::new();
        assert_eq!(hit_test_cursor(&laid, x, y, &cache, &scroll_offsets, 0.0, 600.0).1, None);
    }

    // -- building the absolute-form request `wire` itself parses --

    #[test]
    fn build_get_request_text_uses_absolute_form_and_a_bare_host_header() {
        let url = Url::parse("https://example.com/path?q=1").unwrap();
        assert_eq!(
            build_get_request_text(&url),
            "GET https://example.com/path?q=1 HTTP/1.1\r\nHost: example.com\r\n\r\n"
        );
    }

    #[test]
    fn build_get_request_text_includes_an_explicit_port_in_the_host_header() {
        let url = Url::parse("http://example.com:8080/x").unwrap();
        assert_eq!(build_get_request_text(&url), "GET http://example.com:8080/x HTTP/1.1\r\nHost: example.com:8080\r\n\r\n");
    }

    // -- parsing wire's own normalized response shape --

    #[test]
    fn parse_wire_response_extracts_content_type_and_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhello";
        let (content_type, body) = parse_wire_response(raw).expect("parse");
        assert_eq!(content_type, Some("text/html".to_string()));
        assert_eq!(body, b"hello");
    }

    #[test]
    fn parse_wire_response_content_type_lookup_is_case_insensitive_and_ignores_other_headers() {
        let raw = b"HTTP/1.1 200 OK\r\ncontent-type: image/png\r\nServer: wire\r\n\r\n\x89PNG";
        let (content_type, body) = parse_wire_response(raw).expect("parse");
        assert_eq!(content_type, Some("image/png".to_string()));
        assert_eq!(body, b"\x89PNG");
    }

    #[test]
    fn parse_wire_response_errors_on_a_missing_header_terminator() {
        assert!(parse_wire_response(b"not a real response at all").is_err());
    }

    // -- dispatching by Content-Type, per `scratchy/TODO.txt`'s own
    // settled response-framing contract --

    #[test]
    fn dispatch_wire_response_passes_html_through_untouched() {
        let body = b"<html><body>hi</body></html>".to_vec();
        assert_eq!(dispatch_wire_response(Some("text/html; charset=utf-8".to_string()), body.clone()), Ok(body));
    }

    #[test]
    fn dispatch_wire_response_wraps_a_real_image_in_a_synthetic_html_document() {
        // A real tiny PNG, built the same way `tiny_png_data_uri` builds
        // one for the pre-existing `<img>`-rendering tests just above —
        // decoded back out of its own data: URI purely to get the raw
        // bytes a real `wire` response body would carry. Round-tripped
        // through the *real* `decode_data_uri` this produces an
        // `<img src>` for — not just a string-shape check — to confirm
        // the wrapped document is actually renderable, not merely
        // well-formed-looking HTML.
        let png_bytes = decode_data_uri(&tiny_png_data_uri(1, 1)).expect("decode the fixture's own data: URI");
        let html = dispatch_wire_response(Some("image/png".to_string()), png_bytes.clone()).expect("dispatch");
        let html = String::from_utf8(html).expect("utf8 html");
        assert!(html.starts_with("<html><body><img src=\"data:image/png;base64,"), "got: {html}");
        let data_uri_start = html.find("src=\"").unwrap() + "src=\"".len();
        let data_uri_end = html[data_uri_start..].find('"').unwrap() + data_uri_start;
        let decoded = decode_data_uri(&html[data_uri_start..data_uri_end]).expect("decode_data_uri");
        assert_eq!(decoded, png_bytes, "the wrapped data: URI must decode back to the exact original bytes");
    }

    #[test]
    fn dispatch_wire_response_errors_on_an_unsupported_content_type() {
        let err = dispatch_wire_response(Some("application/pdf".to_string()), vec![1, 2, 3]).unwrap_err();
        assert!(err.contains("application/pdf"), "error should name the unsupported content type: {err}");
    }

    #[test]
    fn dispatch_wire_response_errors_clearly_when_there_is_no_content_type_at_all() {
        assert!(dispatch_wire_response(None, vec![1, 2, 3]).is_err());
    }

    // -- the real wire-socket protocol end to end, against a tiny
    // in-process mock listener (same spirit as `wire`'s own test module,
    // which does the mirror image of this: a real client against a real
    // listener, rather than touching the real internet) --

    fn unique_test_socket_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "scratchy-test-{name}-{}-{:?}.sock",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn fetch_raw_via_wire_round_trips_a_real_response_over_a_real_unix_socket() {
        use std::os::unix::net::UnixListener;
        let path = unique_test_socket_path("roundtrip");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind mock wire socket");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut received = Vec::new();
            // The client always closes its write side (or just drops
            // the connection after reading) once it has what it wants —
            // reading to EOF here mirrors exactly what the real `wire`
            // daemon's own `handle_connection` does on its side of this
            // same protocol.
            let mut buf = [0u8; 4096];
            loop {
                match Read::read(&mut stream, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        received.extend_from_slice(&buf[..n]);
                        if received.ends_with(b"\r\n\r\n") {
                            break; // a GET request has no body to wait for
                        }
                    }
                }
            }
            assert_eq!(
                received,
                build_get_request_text(&Url::parse("https://example.com/page").unwrap()).into_bytes(),
                "scratchy must send the real absolute-form request wire itself parses"
            );
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 29\r\n\r\n<html><body>hi</body></html>")
                .expect("write response");
            // Dropping `stream` here closes the connection — the real
            // `wire` protocol's own "done" signal (see `wire/src/
            // main.rs`'s own `handle_connection`).
        });
        let url = Url::parse("https://example.com/page").unwrap();
        let request_text = build_get_request_text(&url);
        let result = fetch_raw_via_wire(&path, &request_text);
        server.join().expect("mock server thread panicked");
        let _ = std::fs::remove_file(&path);
        match result {
            WireFetchRaw::Responded { content_type, body } => {
                assert_eq!(content_type, Some("text/html".to_string()));
                assert_eq!(body, b"<html><body>hi</body></html>");
            }
            WireFetchRaw::Unreachable => panic!("expected a successful response, got Unreachable"),
        }
    }

    #[test]
    fn fetch_raw_via_wire_reports_unreachable_when_nothing_is_listening() {
        let path = unique_test_socket_path("missing");
        let _ = std::fs::remove_file(&path);
        let request_text = build_get_request_text(&Url::parse("https://example.com/").unwrap());
        assert!(matches!(fetch_raw_via_wire(&path, &request_text), WireFetchRaw::Unreachable));
    }

    // -- sub-resource fetching (v67): external stylesheets and images --

    #[test]
    fn find_external_resources_collects_stylesheet_links_and_non_data_img_srcs() {
        let doc = parse_html(
            r#"<html><head>
                <link rel="stylesheet" href="a.css">
                <link rel="icon" href="favicon.ico">
                <link rel="alternate stylesheet" href="b.css">
            </head><body>
                <img src="photo.jpg">
                <img src="data:image/png;base64,iVBORw0KGgo=">
            </body></html>"#,
        );
        let mut resources = ExternalResources::default();
        find_external_resources(&doc, &mut resources);
        assert_eq!(resources.stylesheets, vec!["a.css".to_string(), "b.css".to_string()]);
        assert_eq!(resources.images, vec!["photo.jpg".to_string()]);
    }

    /// A tiny single-shot mock `wire` listener: binds `path`, accepts
    /// exactly one connection, writes `response` verbatim, then returns
    /// (dropping the stream, closing the connection — the real `wire`
    /// protocol's own "done" signal, same shape every other mock server
    /// in this file already uses). Doesn't bother reading/validating the
    /// request at all — unlike `fetch_raw_via_wire`'s own round-trip
    /// test, these tests are about what `fetch_stylesheet_via_wire`/
    /// `fetch_image_via_wire` do with a *response*, not about re-proving
    /// the request framing that's already covered elsewhere.
    fn spawn_mock_wire_once(path: &std::path::Path, response: &'static [u8]) {
        use std::os::unix::net::UnixListener;
        let listener = UnixListener::bind(path).expect("bind mock wire socket");
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut discard = [0u8; 4096];
            let _ = Read::read(&mut stream, &mut discard);
            let _ = stream.write_all(response);
        });
    }

    #[test]
    fn fetch_stylesheet_via_wire_only_accepts_real_css() {
        let path = unique_test_socket_path("css-ok");
        let _ = std::fs::remove_file(&path);
        spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: 15\r\n\r\nbody{color:red}");
        let url = Url::parse("https://example.com/style.css").unwrap();
        // A real connection, over a real socket — not a direct call into
        // `fetch_sub_resource_via_wire`'s own internals — the same
        // "real I/O, not just the pure logic" bar `fetch_raw_via_wire`'s
        // own test already holds to.
        assert_eq!(fetch_stylesheet_via_wire(&path, &url), Some("body{color:red}".to_string()));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fetch_stylesheet_via_wire_rejects_a_non_css_content_type() {
        let path = unique_test_socket_path("css-wrong-type");
        let _ = std::fs::remove_file(&path);
        spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 4\r\n\r\n<p/>");
        let url = Url::parse("https://example.com/not-css").unwrap();
        assert_eq!(fetch_stylesheet_via_wire(&path, &url), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fetch_image_via_wire_decodes_a_real_fetched_png() {
        use base64::Engine;
        let data_uri = tiny_png_data_uri(2, 2);
        let payload = data_uri.strip_prefix("data:image/png;base64,").unwrap();
        let png_bytes = base64::engine::general_purpose::STANDARD.decode(payload).unwrap();
        let mut raw = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", png_bytes.len()).into_bytes();
        raw.extend_from_slice(&png_bytes);
        let path = unique_test_socket_path("image-ok");
        let _ = std::fs::remove_file(&path);
        spawn_mock_wire_once(&path, raw.leak());
        let url = Url::parse("https://example.com/pixel.png").unwrap();
        let decoded = fetch_image_via_wire(&path, &url).expect("decode");
        assert_eq!((decoded.width, decoded.height), (2, 2));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fetch_image_via_wire_rejects_a_non_image_content_type() {
        let path = unique_test_socket_path("image-wrong-type");
        let _ = std::fs::remove_file(&path);
        spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello");
        let url = Url::parse("https://example.com/not-an-image").unwrap();
        assert!(fetch_image_via_wire(&path, &url).is_none());
        let _ = std::fs::remove_file(&path);
    }

    // -- the rendering side: does a cached live-fetched resource
    // actually reach the page, not just the fetch functions above --

    fn find_element_by_tag(handle: &Handle, tag: &str) -> Option<Handle> {
        if element_tag(handle) == tag {
            return Some(handle.clone());
        }
        handle.children.borrow().iter().find_map(|c| find_element_by_tag(c, tag))
    }

    #[test]
    fn resolve_img_renders_a_cached_live_fetched_image_for_a_non_data_uri_src() {
        let doc = parse_html(r#"<html><body><img src="https://example.com/photo.jpg" style="display:block"></body></html>"#);
        let img_handle = find_element_by_tag(&doc, "img").expect("no <img> found in fixture");
        let decoded = DecodedImage { width: 4, height: 4, pixels: vec![TextColor::rgb(1, 2, 3); 16] };
        let mut image_cache = HashMap::new();
        image_cache.insert("https://example.com/photo.jpg".to_string(), Arc::new(decoded));
        let (_, found) = resolve_img(&img_handle, resolve_style(&img_handle, &[], ROOT_STYLE, 1920.0), &image_cache);
        assert!(found.is_some(), "a src present in image_cache must be used, not fall back to alt text");
        let found = found.unwrap();
        assert_eq!((found.width, found.height), (4, 4));
    }

    #[test]
    fn resolve_img_is_a_cache_miss_for_a_non_data_uri_src_not_yet_fetched() {
        let doc = parse_html(r#"<html><body><img src="https://example.com/not-fetched-yet.jpg"></body></html>"#);
        let img_handle = find_element_by_tag(&doc, "img").expect("no <img> found in fixture");
        let (_, found) = resolve_img(&img_handle, resolve_style(&img_handle, &[], ROOT_STYLE, 1920.0), &HashMap::new());
        assert!(found.is_none(), "an empty image_cache must be a clean miss, not a panic or a guess");
    }

    #[test]
    fn build_layout_tree_with_resources_applies_extra_css_to_the_real_cascade() {
        let doc = parse_html(r#"<html><body><div class="box">x</div></body></html>"#);
        let extra_css = ".box { background-color: #00ff00; }";
        let tree = build_layout_tree_with_resources(&doc, 1920.0, extra_css, &HashMap::new());
        assert_eq!(
            find_box_with_background(&tree),
            Some(TextColor::rgb(0, 0xff, 0)),
            "CSS fetched from an external stylesheet must cascade exactly like an inline <style> block's own rules"
        );
    }

    #[test]
    fn build_layout_tree_with_resources_renders_a_cached_image_as_a_real_image_box() {
        let doc = parse_html(r#"<html><body><img src="https://example.com/photo.jpg" style="display:block"></body></html>"#);
        let mut image_cache = HashMap::new();
        image_cache.insert(
            "https://example.com/photo.jpg".to_string(),
            Arc::new(DecodedImage { width: 2, height: 2, pixels: vec![TextColor::rgb(9, 9, 9); 4] }),
        );
        let tree = build_layout_tree_with_resources(&doc, 1920.0, "", &image_cache);
        fn find_image_box(b: &LayoutBox) -> Option<&LayoutBox> {
            if matches!(b.content, BoxContent::Image(_)) {
                return Some(b);
            }
            if let BoxContent::Blocks(children) = &b.content {
                for c in children {
                    if let Some(found) = find_image_box(c) {
                        return Some(found);
                    }
                }
            }
            None
        }
        assert!(
            find_image_box(&tree).is_some(),
            "a non-data: <img> with its src already in image_cache must render as a real image box, not fall back to alt text"
        );
    }

    // `process_external_resources` itself isn't unit-tested directly —
    // it takes a real `EventLoopProxy<AppEvent>`, and this file's own
    // established precedent (see NOTES.md's v65/v62/v55/v32 entries) is
    // to never construct a real winit `EventLoop` inside `#[cfg(test)]`
    // just to get one; its two real pieces (`find_external_resources`,
    // `fetch_stylesheet_via_wire`, `fetch_image_via_wire`, all above)
    // are each independently covered, and the thin orchestration between
    // them (resolve each href, fetch/spawn) rests on code review plus
    // the live interactive verification `PROMPTS.md`'s own v67 entry
    // describes, same category as `start_navigation`'s own click-side
    // wiring already is.

    // -- `App::show_message`'s HTML-escaping, since it echoes back
    // arbitrary page/OS-controlled text (a clicked href, an io::Error)
    // into synthetic markup --

    #[test]
    fn escape_html_escapes_the_characters_that_would_corrupt_the_synthetic_markup() {
        assert_eq!(escape_html("a < b & c > d"), "a &lt; b &amp; c &gt; d");
    }

    // ---------- vim mode: `resolve_binding`/`TextEditBuffer` ----------
    // (TODO.txt's own plan — the mode enum, real keyboard wiring, the
    // `:` command bar, `:set vim`/`:set novim`, `:visit`, and the
    // `SIGUSR1` bootstrap for turning vim mode back on once off.)

    #[test]
    fn resolve_binding_maps_bare_jk_to_scroll_in_normal_mode() {
        let key = Key::Character("j".into());
        assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::ScrollDown));
        let key = Key::Character("k".into());
        assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::ScrollUp));
    }

    #[test]
    fn resolve_binding_requires_control_for_half_page_scroll() {
        let key = Key::Character("d".into());
        assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), None);
        assert_eq!(
            resolve_binding(&key, ModifiersState::CONTROL, VimMode::Normal),
            Some(Action::HalfPageDown)
        );
    }

    #[test]
    fn resolve_binding_colon_enters_command_mode() {
        let key = Key::Character(":".into());
        assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), Some(Action::EnterCommandMode));
    }

    #[test]
    fn resolve_binding_never_fires_outside_normal_mode() {
        let key = Key::Character("j".into());
        assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Command), None);
    }

    #[test]
    fn resolve_binding_ignores_unbound_keys() {
        let key = Key::Character("x".into());
        assert_eq!(resolve_binding(&key, ModifiersState::empty(), VimMode::Normal), None);
    }

    #[test]
    fn key_insertion_text_handles_the_named_space_key_not_just_character_keys() {
        // The exact regression reported post-merge: `winit` reports
        // space as `Key::Named(NamedKey::Space)`, so a match arm that
        // only handles `Key::Character` silently drops it.
        assert_eq!(key_insertion_text(&Key::Named(NamedKey::Space)), Some(" "));
        assert_eq!(key_insertion_text(&Key::Character("a".into())), Some("a"));
        assert_eq!(key_insertion_text(&Key::Named(NamedKey::Enter)), None);
        assert_eq!(key_insertion_text(&Key::Named(NamedKey::Escape)), None);
    }

    #[test]
    fn text_edit_buffer_insert_backspace_and_move_track_a_real_cursor() {
        let mut buf = TextEditBuffer::default();
        buf.insert('v');
        buf.insert('i');
        buf.insert('m');
        assert_eq!(buf.text, "vim");
        assert_eq!(buf.cursor, 3);

        buf.move_left();
        buf.move_left();
        assert_eq!(buf.cursor, 1);

        buf.insert('!');
        assert_eq!(buf.text, "v!im");
        assert_eq!(buf.cursor, 2);

        buf.backspace();
        assert_eq!(buf.text, "vim");
        assert_eq!(buf.cursor, 1);
    }

    #[test]
    fn text_edit_buffer_backspace_and_move_left_at_start_are_no_ops() {
        let mut buf = TextEditBuffer::default();
        buf.backspace();
        buf.move_left();
        assert_eq!(buf.text, "");
        assert_eq!(buf.cursor, 0);
    }

    #[test]
    fn text_edit_buffer_move_right_at_end_is_a_no_op() {
        let mut buf = TextEditBuffer::default();
        buf.insert('x');
        buf.move_right();
        assert_eq!(buf.cursor, 1);
    }

    #[test]
    fn text_edit_buffer_moves_by_whole_codepoints_not_raw_bytes() {
        let mut buf = TextEditBuffer::default();
        buf.insert('é'); // 2 UTF-8 bytes, one char
        assert_eq!(buf.cursor, 2);
        buf.move_left();
        assert_eq!(buf.cursor, 0);
        buf.backspace(); // no-op at start, must not panic on a mid-codepoint byte index
        assert_eq!(buf.text, "é");
    }
}
