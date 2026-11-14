use super::*;

// ---------- CSS parsing ----------

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Display {
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
    /// normal block/inline flow. See `InlineFlex` for the inline-level
    /// outer-placement twin of this — `measure_flex`'s own internals
    /// don't care which of the two built this box.
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
    /// implemented — would need the identical treatment `InlineFlex`
    /// just below gets, applied to `Grid`'s own dispatch guards instead,
    /// a trivial follow-on not bundled into this item.
    Grid,
    /// `display: inline-block` — inline-level outer placement (shares a
    /// line with surrounding text/images, wraps like any other inline
    /// item — see `InlineItem::Embedded`) but keeps its own real box
    /// model and lays its children out exactly like an ordinary block
    /// box (normal flow, via the same generic `Blocks` path `Block`
    /// already falls into — nothing inline-block-specific exists in
    /// `measure`/`build_box_for_element` at all, since neither has any
    /// special case keyed on `Block` that `InlineBlock` would also need
    /// to match).
    InlineBlock,
    /// `display: inline-flex` — `InlineBlock`'s own inline-level outer
    /// placement, but this box's *children* are laid out via the same
    /// flex algorithm `Flex` already uses (`measure_flex`) instead of
    /// normal block flow. Needs both dispatch guards that currently
    /// check `style.display == Display::Flex` widened to also match
    /// this variant (`build_box_for_element`, `measure`) — confirmed by
    /// reading `measure_flex`/`measure_flex_row`/`measure_flex_column`
    /// directly that nothing inside them reads `Display::Flex` at all,
    /// so those two dispatch sites are the entire flex-specific
    /// footprint this variant needs.
    InlineFlex,
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
pub(crate) enum Position {
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
pub(crate) enum Float {
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
pub(crate) enum Clear {
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
pub(crate) enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

impl FlexDirection {
    pub(crate) fn is_row(self) -> bool {
        matches!(self, FlexDirection::Row | FlexDirection::RowReverse)
    }

    pub(crate) fn is_reverse(self) -> bool {
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
pub(crate) enum FlexWrap {
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
pub(crate) enum JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// `align-content`'s keywords: how the lines of a *wrapped* row flex
/// container are distributed along the cross axis when the container
/// has a definite height. `normal` is `stretch`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub(crate) enum AlignContent {
    #[default]
    Stretch,
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
pub(crate) enum AlignItems {
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
#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) enum FlexBasis {
    #[default]
    Auto,
    Len(Length),
}

/// One grid track's declared size — `grid-template-columns`/`-rows`'
/// own `<track-size>` grammar, scoped to the forms this item
/// implements (see `parse_grid_track_size`'s own docs for what's still
/// cut: content-based keywords only, `auto`/`min-content`/`max-content`
/// — `minmax()` itself landed, see `MinMax`'s own docs below). `Len`,
/// not yet resolved to a pixel/`BoxLength` value — unlike every other
/// length-ish property in this file, a track's real basis
/// (`resolve_grid_tracks`'s own `available` parameter: the grid
/// container's own content *width* for a column track, its content
/// *height* — not always definite, see `measure_grid`'s own docs — for
/// a row track) isn't known at cascade time, so resolution happens in
/// `measure_grid` instead, mirroring `FlexBasis`'s/`LineHeight`'s own
/// "stay symbolic until a real basis exists" pattern rather than
/// `BoxLength`'s (which only ever needs *one* of two possible bases per
/// property, always the same one for that property — a track's basis
/// genuinely differs by axis, which `BoxLength` has no slot for). `Fr`
/// is the raw flex factor (`2.0` for `2fr`) — grid's own flexible-length
/// primitive, directly analogous to flexbox's `flex-grow`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum GridTrackSpec {
    Len(Length),
    Fr(f32),
    /// `auto` — sized from the items in the track (`minmax(auto, auto)`):
    /// at least their min-content, growing toward max-content, then
    /// stretched equally to absorb leftover space when no `fr` track
    /// wants it. See `resolve_grid_tracks`.
    Auto,
    /// `min-content` / `max-content` — the narrowest unbreakable / the
    /// unwrapped width of the items in the track (`cell_natural_width`).
    MinContent,
    MaxContent,
    /// `minmax(<min>, <max>)` — real CSS Grid §7.2.3, scoped to the two
    /// `TrackBreadth` forms this file's own `<track-size>` grammar
    /// already allows outside of `minmax()` (a plain length/`%`/`fr`),
    /// same cut the real spec's own grammar makes a reasonable subset
    /// of (no `auto`/`min-content`/`max-content` inside `minmax()`
    /// either — that's still the same, separate, not-yet-attempted
    /// content-based-track-sizing item `TODO.txt` already names). Real
    /// usage in the wild overwhelmingly looks like `minmax(0, 1fr)` —
    /// a flexible track with a `0` floor, purely to stop it from ever
    /// shrinking *past* zero under content pressure — confirmed by
    /// pulling real Wikipedia CSS and grepping every `minmax(...)` call
    /// in it (`scratchy/TODO.txt`'s own `display: grid` entry) — which
    /// is exactly the shape `resolve_grid_tracks`'s own single-pass
    /// floor-then-distribute algorithm resolves correctly, not just
    /// approximately (a `0` floor adds no real constraint at all). A
    /// nonzero floor against a flexible max is still real, single-pass
    /// CSS Grid §11.7-shaped math (not the full iterative "freeze a
    /// track whose floor violates its own distributed share and redo
    /// the whole pass" algorithm — same documented simplification
    /// flexbox's own `resolve_flex_main_sizes` already carries) — see
    /// `resolve_grid_tracks`'s own docs for exactly what that trades
    /// away.
    MinMax(TrackBreadth, TrackBreadth),
    /// `repeat(auto-fill|auto-fit, <track-list>)` — the number of
    /// repetitions depends on the available space, so it stays
    /// symbolic until `measure_grid` calls `expand_auto_repeat`
    /// (columns only; rows reject it at parse time). `fit` collapses
    /// repetitions that end up with no items. Never reaches
    /// `resolve_grid_tracks`.
    AutoRepeat { fit: bool, tracks: &'static [GridTrackSpec] },
}

/// One `minmax()` argument — deliberately a narrower type than
/// `GridTrackSpec` itself (no nested `MinMax`, which the real grammar
/// doesn't allow either): a plain length/`%` or an `fr` factor, the
/// same two forms a bare (non-`minmax`) track already accepts. A real
/// `minmax()` disallows `fr` as its *first* (minimum) argument — not
/// enforced here; a `MinMax(Fr(_), _)` simply never arises from real
/// CSS and `resolve_grid_tracks` treats a flexible minimum as an
/// (accepted, harmless) `0` floor if it ever did.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum TrackBreadth {
    Len(Length),
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
}

/// `auto` / `min-content` / `max-content` as a track breadth keyword.
pub(crate) fn parse_track_keyword(input: &mut CssParser) -> Option<TrackBreadth> {
    let state = input.state();
    let found = match input.next() {
        Ok(Token::Ident(id)) if id.eq_ignore_ascii_case("auto") => Some(TrackBreadth::Auto),
        Ok(Token::Ident(id)) if id.eq_ignore_ascii_case("min-content") => Some(TrackBreadth::MinContent),
        Ok(Token::Ident(id)) if id.eq_ignore_ascii_case("max-content") => Some(TrackBreadth::MaxContent),
        _ => None,
    };
    if found.is_none() {
        input.reset(&state);
    }
    found
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
pub(crate) struct Insets {
    pub(crate) top: Option<BoxLength>,
    pub(crate) right: Option<BoxLength>,
    pub(crate) bottom: Option<BoxLength>,
    pub(crate) left: Option<BoxLength>,
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
/// CSS's own fixed absolute-unit conversion (1in = 96px, 1pt = 1in/72,
/// both exact per the spec, not an approximation) — `pt` tokenizes as
/// an ordinary `<dimension-token>` with unit `pt`, same as `px`/`em`/
/// `rem` already do, so every `pt` value is just converted to `Px` once,
/// at parse time, rather than adding a whole new `Length` variant and
/// teaching every consumer of this enum (`resolve`, `BoxLength`, the
/// grid/flex track machinery, ...) a fourth absolute-unit case to
/// handle identically to `Px` everywhere it matters.
pub(crate) const PX_PER_PT: f32 = 96.0 / 72.0;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Length {
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
    /// A `calc()`/`min()`/`max()`/`clamp()` expression (see `CalcNode`).
    /// The tree is leaked (`Box::leak`) at parse time so this enum, and
    /// everything that embeds it, stays `Copy`.
    Calc(&'static CalcNode),
    /// `vw`/`vh`/`vmin`/`vmax` (Q14): resolved against the viewport
    /// recorded by `set_viewport` for the current style build.
    Vp(VpUnit, f32),
    /// `min-content`/`max-content`/`fit-content` (Q20f); only `width` and
    /// `min-width` accept it, and `resolve_style` turns it into
    /// `ComputedStyle::width_kw`/`min_width_kw` (the plain length stays
    /// unset), so it never reaches `resolve`.
    Intrinsic(IntrinsicSize),
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum IntrinsicSize {
    MinContent,
    MaxContent,
    FitContent,
}

/// Which viewport-relative unit a `Length::Vp`/`CalcNode::Vp` is in.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum VpUnit {
    W,
    H,
    Min,
    Max,
}

thread_local! {
    /// The viewport (width, height) in CSS px that `vw`/`vh`/`vmin`/`vmax`
    /// resolve against. Set by `build_layout_tree_sized` at the start of
    /// every style build (style resolution runs synchronously on the
    /// calling thread, so this never needs to cross to the shaping
    /// worker: `BoxLength::Calc` carries its own copy).
    pub(crate) static VIEWPORT: std::cell::Cell<(f32, f32)> = const { std::cell::Cell::new((1920.0, 1080.0)) };
}

pub(crate) fn set_viewport(w: f32, h: f32) {
    VIEWPORT.with(|v| v.set((w, h)));
}

pub(crate) fn current_viewport() -> (f32, f32) {
    VIEWPORT.with(|v| v.get())
}

impl VpUnit {
    pub(crate) fn from_unit(unit: &str) -> Option<VpUnit> {
        match unit.to_ascii_lowercase().as_str() {
            "vw" => Some(VpUnit::W),
            "vh" => Some(VpUnit::H),
            "vmin" => Some(VpUnit::Min),
            "vmax" => Some(VpUnit::Max),
            _ => None,
        }
    }

    /// `v` units of this kind (1 unit = 1% of the viewport dimension).
    pub(crate) fn px(self, v: f32, vp: (f32, f32)) -> f32 {
        if self != VpUnit::W {
            note_viewport_height_read();
        }
        let base = match self {
            VpUnit::W => vp.0,
            VpUnit::H => vp.1,
            VpUnit::Min => vp.0.min(vp.1),
            VpUnit::Max => vp.0.max(vp.1),
        };
        v * base / 100.0
    }
}

/// A parsed `calc()`/`min()`/`max()`/`clamp()` expression. `Num` is a
/// bare number, every other leaf is a length (`Pct` is the raw fraction).
/// `parse_calc_function` type-checks the tree (`+`/`-` operands agree,
/// `*` has at most one length side, `/` has a non-zero number on the
/// right), so `eval` can't fail.
#[derive(PartialEq, Debug)]
pub(crate) enum CalcNode {
    Num(f32),
    Px(f32),
    Em(f32),
    Rem(f32),
    Pct(f32),
    Vp(VpUnit, f32),
    Add(Box<CalcNode>, Box<CalcNode>),
    Sub(Box<CalcNode>, Box<CalcNode>),
    Mul(Box<CalcNode>, Box<CalcNode>),
    Div(Box<CalcNode>, Box<CalcNode>),
    Min(Vec<CalcNode>),
    Max(Vec<CalcNode>),
    Clamp(Box<CalcNode>, Box<CalcNode>, Box<CalcNode>),
}

impl CalcNode {
    /// Evaluates to a px value (or a bare number for number-typed
    /// subtrees). `pct_basis` is what `100%` means.
    pub(crate) fn eval(&self, font_size: f32, root_font_size: f32, pct_basis: f32, vp: (f32, f32)) -> f32 {
        let e = |n: &CalcNode| n.eval(font_size, root_font_size, pct_basis, vp);
        match self {
            CalcNode::Num(v) | CalcNode::Px(v) => *v,
            CalcNode::Em(v) => v * font_size,
            CalcNode::Rem(v) => v * root_font_size,
            CalcNode::Pct(p) => p * pct_basis,
            CalcNode::Vp(u, v) => u.px(*v, vp),
            CalcNode::Add(a, b) => e(a) + e(b),
            CalcNode::Sub(a, b) => e(a) - e(b),
            CalcNode::Mul(a, b) => e(a) * e(b),
            CalcNode::Div(a, b) => {
                let d = e(b);
                if d == 0.0 { 0.0 } else { e(a) / d }
            }
            CalcNode::Min(v) => v.iter().map(e).fold(f32::INFINITY, f32::min),
            CalcNode::Max(v) => v.iter().map(e).fold(f32::NEG_INFINITY, f32::max),
            // CSS: clamp(MIN, VAL, MAX) = max(MIN, min(VAL, MAX)).
            CalcNode::Clamp(lo, v, hi) => e(lo).max(e(v).min(e(hi))),
        }
    }

    pub(crate) fn has_percent(&self) -> bool {
        match self {
            CalcNode::Pct(_) => true,
            CalcNode::Num(_) | CalcNode::Px(_) | CalcNode::Em(_) | CalcNode::Rem(_) | CalcNode::Vp(..) => false,
            CalcNode::Add(a, b) | CalcNode::Sub(a, b) | CalcNode::Mul(a, b) | CalcNode::Div(a, b) => {
                a.has_percent() || b.has_percent()
            }
            CalcNode::Min(v) | CalcNode::Max(v) => v.iter().any(CalcNode::has_percent),
            CalcNode::Clamp(a, b, c) => a.has_percent() || b.has_percent() || c.has_percent(),
        }
    }

    /// `Some(true)` for a length-typed subtree, `Some(false)` for a
    /// number, `None` when the tree is ill-typed.
    pub(crate) fn is_length(&self) -> Option<bool> {
        match self {
            CalcNode::Num(_) => Some(false),
            CalcNode::Px(_) | CalcNode::Em(_) | CalcNode::Rem(_) | CalcNode::Pct(_) | CalcNode::Vp(..) => Some(true),
            CalcNode::Add(a, b) | CalcNode::Sub(a, b) => {
                let (x, y) = (a.is_length()?, b.is_length()?);
                (x == y).then_some(x)
            }
            CalcNode::Mul(a, b) => {
                let (x, y) = (a.is_length()?, b.is_length()?);
                (!(x && y)).then_some(x || y)
            }
            CalcNode::Div(a, b) => {
                let (x, y) = (a.is_length()?, b.is_length()?);
                // Divisor must be a non-zero number (no units, so a
                // zero-basis evaluation is exact).
                (!y && b.eval(0.0, 0.0, 0.0, (0.0, 0.0)) != 0.0).then_some(x)
            }
            CalcNode::Min(v) | CalcNode::Max(v) => {
                let first = v.first()?.is_length()?;
                v.iter().all(|n| n.is_length() == Some(first)).then_some(first)
            }
            CalcNode::Clamp(a, b, c) => {
                let x = a.is_length()?;
                (b.is_length()? == x && c.is_length()? == x).then_some(x)
            }
        }
    }
}

pub(crate) fn parse_calc_sum(input: &mut CssParser) -> Result<CalcNode, ()> {
    let mut acc = parse_calc_product(input)?;
    loop {
        let state = input.state();
        match input.next() {
            Ok(Token::Delim('+')) => acc = CalcNode::Add(Box::new(acc), Box::new(parse_calc_product(input)?)),
            Ok(Token::Delim('-')) => acc = CalcNode::Sub(Box::new(acc), Box::new(parse_calc_product(input)?)),
            _ => {
                input.reset(&state);
                return Ok(acc);
            }
        }
    }
}

pub(crate) fn parse_calc_product(input: &mut CssParser) -> Result<CalcNode, ()> {
    let mut acc = parse_calc_atom(input)?;
    loop {
        let state = input.state();
        match input.next() {
            Ok(Token::Delim('*')) => acc = CalcNode::Mul(Box::new(acc), Box::new(parse_calc_atom(input)?)),
            Ok(Token::Delim('/')) => acc = CalcNode::Div(Box::new(acc), Box::new(parse_calc_atom(input)?)),
            _ => {
                input.reset(&state);
                return Ok(acc);
            }
        }
    }
}

/// Comma-separated list of sums inside a `min(`/`max(`/`clamp(` block.
pub(crate) fn parse_calc_args(input: &mut CssParser) -> Result<Vec<CalcNode>, ()> {
    let mut args = vec![parse_calc_sum(input)?];
    while input.try_parse(|i| i.expect_comma()).is_ok() {
        args.push(parse_calc_sum(input)?);
    }
    input.expect_exhausted().map_err(|_| ())?;
    Ok(args)
}

/// The body of a math function (`calc`/`min`/`max`/`clamp`, or a bare
/// parenthesis when `name` is empty) whose opening token was consumed.
pub(crate) fn parse_calc_body(input: &mut CssParser, name: &str) -> Result<CalcNode, ()> {
    let name = name.to_ascii_lowercase();
    input
        .parse_nested_block(|i| {
            let bad = |i: &mut CssParser| i.new_error_for_next_token::<()>();
            match name.as_str() {
                "" | "calc" => {
                    let n = parse_calc_sum(i).map_err(|_| bad(i))?;
                    i.expect_exhausted()?;
                    Ok(n)
                }
                "min" => Ok(CalcNode::Min(parse_calc_args(i).map_err(|_| bad(i))?)),
                "max" => Ok(CalcNode::Max(parse_calc_args(i).map_err(|_| bad(i))?)),
                "clamp" => {
                    let mut a = parse_calc_args(i).map_err(|_| bad(i))?;
                    if a.len() != 3 {
                        return Err(bad(i));
                    }
                    let hi = a.pop().unwrap();
                    let v = a.pop().unwrap();
                    let lo = a.pop().unwrap();
                    Ok(CalcNode::Clamp(Box::new(lo), Box::new(v), Box::new(hi)))
                }
                _ => Err(i.new_error_for_next_token()),
            }
        })
        .map_err(|_: cssparser::ParseError<()>| ())
}

pub(crate) fn parse_calc_atom(input: &mut CssParser) -> Result<CalcNode, ()> {
    match input.next().map_err(|_| ())?.clone() {
        Token::Number { value, .. } => Ok(CalcNode::Num(value)),
        Token::Percentage { unit_value, .. } => Ok(CalcNode::Pct(unit_value)),
        Token::Dimension { value, unit, .. } => {
            if unit.eq_ignore_ascii_case("px") {
                Ok(CalcNode::Px(value))
            } else if unit.eq_ignore_ascii_case("em") {
                Ok(CalcNode::Em(value))
            } else if unit.eq_ignore_ascii_case("rem") {
                Ok(CalcNode::Rem(value))
            } else if unit.eq_ignore_ascii_case("pt") {
                Ok(CalcNode::Px(value * PX_PER_PT))
            } else if let Some(u) = VpUnit::from_unit(&unit) {
                Ok(CalcNode::Vp(u, value))
            } else {
                Err(())
            }
        }
        Token::ParenthesisBlock => parse_calc_body(input, ""),
        Token::Function(name) => parse_calc_body(input, &name),
        _ => Err(()),
    }
}

/// Parses a `calc()`/`min()`/`max()`/`clamp()` function whose name token
/// has just been consumed (`name`), into a `Length`. Must be a length
/// (not a bare number); `allow_percent` is false for `<length>`-only
/// properties. Ill-typed or zero-dividing expressions are `Err`.
pub(crate) fn parse_calc_function(input: &mut CssParser, name: &str, allow_percent: bool) -> Result<Length, ()> {
    if !["calc", "min", "max", "clamp"].iter().any(|f| name.eq_ignore_ascii_case(f)) {
        return Err(());
    }
    let node = parse_calc_body(input, name)?;
    if node.is_length() != Some(true) || (!allow_percent && node.has_percent()) {
        return Err(());
    }
    Ok(Length::Calc(Box::leak(Box::new(node))))
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
    pub(crate) fn resolve(self, font_size: f32, root_font_size: f32) -> f32 {
        match self {
            Length::Px(v) => v,
            Length::Em(v) => v * font_size,
            Length::Rem(v) => v * root_font_size,
            Length::Percent(p) => font_size * p,
            Length::Larger => font_size * 1.2,
            Length::Smaller => font_size / 1.2,
            Length::Calc(c) => c.eval(font_size, root_font_size, font_size, current_viewport()),
            Length::Vp(u, v) => u.px(v, current_viewport()),
            Length::Intrinsic(_) => 0.0,
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
pub(crate) enum BoxLength {
    Px(f32),
    Percent(f32),
    /// A `calc()` containing a `%`: em/rem parts are resolved against the
    /// carried (font-size, root-font-size) now, the `%` against the
    /// containing-block basis later.
    Calc(&'static CalcNode, f32, f32, (f32, f32)),
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
    pub(crate) fn from_length(l: Length, font_size: f32, root_font_size: f32) -> Self {
        match l {
            Length::Percent(p) => BoxLength::Percent(p),
            Length::Calc(c) if c.has_percent() => {
                note_viewport_height_read();
                BoxLength::Calc(c, font_size, root_font_size, current_viewport())
            }
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
    pub(crate) fn resolve(self, basis: f32) -> f32 {
        match self {
            BoxLength::Px(v) => v,
            BoxLength::Percent(p) => basis * p,
            BoxLength::Calc(c, fs, rfs, vp) => c.eval(fs, rfs, basis, vp),
        }
    }

    /// Like `resolve`, but for a basis that may be indefinite (`height`
    /// against an auto-height containing block): a percentage-bearing
    /// value then has no answer.
    pub(crate) fn resolve_opt(self, basis: Option<f32>) -> Option<f32> {
        match self {
            BoxLength::Px(v) => Some(v),
            other => basis.map(|b| other.resolve(b)),
        }
    }

    /// Reads an already-finalized value with no basis needed — see this
    /// type's own docs for the invariant that makes this safe wherever
    /// it's called. Debug-asserts (rather than silently resolving
    /// against a bogus implicit basis of `0.0`) if that invariant is
    /// ever violated, which would otherwise be a confusing, silent
    /// wrong-number-not-a-crash bug.
    pub(crate) fn px(self) -> f32 {
        match self {
            BoxLength::Px(v) => v,
            BoxLength::Percent(_) | BoxLength::Calc(..) => {
                debug_assert!(false, "BoxLength::px() called on an unresolved percent");
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
#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) enum LineHeight {
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
    pub(crate) fn resolve(self, font_size: f32, root_font_size: f32) -> f32 {
        match self {
            LineHeight::Normal => font_size * 1.25,
            LineHeight::Multiplier(m) => font_size * m,
            LineHeight::Len(l) => l.resolve(font_size, root_font_size),
        }
    }
}

/// `content` value (Q19, Q19b). `url()` is not supported and makes the
/// declaration invalid.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ContentDecl {
    /// `none` / `normal`: no generated box.
    None,
    /// One or more adjacent strings, concatenated.
    Text(String),
    /// A lone `url(...)` (Q19d): an image as generated content. The raw
    /// argument; decoded in `generated_content` (a `data:` URI, or an
    /// already-fetched `image_cache` entry keyed by the raw text).
    Image(String),
    /// A list containing at least one dynamic part (`attr()`,
    /// `counter()`, `counters()`, a quote keyword). Evaluated in document
    /// order by `precompute_generated`.
    Parts(Vec<ContentPart>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ContentPart {
    Str(String),
    Attr(String),
    Counter { name: String, style: String },
    Counters { name: String, sep: String, style: String },
    OpenQuote,
    CloseQuote,
    NoOpenQuote,
    NoCloseQuote,
}

/// Parses a `content` value: `none`, `normal`, or a list of strings,
/// `attr()`, `counter()`, `counters()` and quote keywords.
pub(crate) fn parse_content(input: &mut CssParser) -> Result<ContentDecl, ()> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok()
        || input.try_parse(|i| i.expect_ident_matching("normal")).is_ok()
    {
        return Ok(ContentDecl::None);
    }
    let mut parts: Vec<ContentPart> = Vec::new();
    if let Ok(url) = input.try_parse(|i| i.expect_url().map(|u| u.to_string())) {
        return if input.is_exhausted() { Ok(ContentDecl::Image(url)) } else { Err(()) };
    }
    while let Ok(tok) = input.next() {
        match tok.clone() {
            Token::QuotedString(q) => parts.push(ContentPart::Str(q.to_string())),
            Token::Ident(id) => parts.push(match &*id.to_ascii_lowercase() {
                "open-quote" => ContentPart::OpenQuote,
                "close-quote" => ContentPart::CloseQuote,
                "no-open-quote" => ContentPart::NoOpenQuote,
                "no-close-quote" => ContentPart::NoCloseQuote,
                _ => return Err(()),
            }),
            Token::Function(f) => {
                let f = f.to_ascii_lowercase();
                let part = input
                    .parse_nested_block(|i| -> Result<ContentPart, ParseError<()>> {
                        let name = match i.next()?.clone() {
                            Token::Ident(n) => n.to_string(),
                            _ => return Err(i.new_error_for_next_token()),
                        };
                        match &*f {
                            "attr" => Ok(ContentPart::Attr(name)),
                            "counter" => {
                                let style = if i.try_parse(|i| i.expect_comma()).is_ok() {
                                    i.expect_ident()?.to_ascii_lowercase()
                                } else {
                                    "decimal".to_string()
                                };
                                Ok(ContentPart::Counter { name, style })
                            }
                            "counters" => {
                                i.expect_comma()?;
                                let sep = i.expect_string()?.to_string();
                                let style = if i.try_parse(|i| i.expect_comma()).is_ok() {
                                    i.expect_ident()?.to_ascii_lowercase()
                                } else {
                                    "decimal".to_string()
                                };
                                Ok(ContentPart::Counters { name, sep, style })
                            }
                            _ => Err(i.new_error_for_next_token()),
                        }
                    })
                    .map_err(|_| ())?;
                parts.push(part);
            }
            _ => return Err(()),
        }
    }
    if parts.is_empty() {
        Err(())
    } else if parts.iter().all(|p| matches!(p, ContentPart::Str(_))) {
        let mut text = String::new();
        for p in &parts {
            if let ContentPart::Str(s) = p {
                text.push_str(s);
            }
        }
        Ok(ContentDecl::Text(text))
    } else {
        Ok(ContentDecl::Parts(parts))
    }
}

/// `counter-reset` / `counter-increment`: `<name> [<integer>]` pairs, or
/// `none`. `default` is the integer used when none is given (0 for reset,
/// 1 for increment).
pub(crate) fn parse_counter_ops(input: &mut CssParser, default: i32) -> Result<Vec<(String, i32)>, ()> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    while let Ok(name) = input.try_parse(|i| i.expect_ident().map(|s| s.to_string())) {
        // CSS-wide keywords are not counter names (declaration ignored).
        if ["inherit", "initial", "unset", "revert", "revert-layer"].iter().any(|k| name.eq_ignore_ascii_case(k)) {
            return Err(());
        }
        let n = input.try_parse(|i| i.expect_integer()).unwrap_or(default);
        out.push((name, n));
    }
    if out.is_empty() || !input.is_exhausted() { Err(()) } else { Ok(out) }
}

/// The default `quotes: auto` pairs (English, two nesting levels).
pub(crate) fn default_quotes() -> Vec<(String, String)> {
    vec![("\u{201C}".into(), "\u{201D}".into()), ("\u{2018}".into(), "\u{2019}".into())]
}

/// `quotes: none | auto | [<string> <string>]+`. `none` is an empty list.
pub(crate) fn parse_quotes(input: &mut CssParser) -> Result<Vec<(String, String)>, ()> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return if input.is_exhausted() { Ok(Vec::new()) } else { Err(()) };
    }
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return if input.is_exhausted() { Ok(default_quotes()) } else { Err(()) };
    }
    let mut out = Vec::new();
    while let Ok(open) = input.try_parse(|i| i.expect_string().map(|s| s.to_string())) {
        let close = input.expect_string().map_err(|_| ())?.to_string();
        out.push((open, close));
    }
    if out.is_empty() || !input.is_exhausted() { Err(()) } else { Ok(out) }
}

#[derive(Clone, Default, PartialEq)]
pub(crate) struct Declarations {
    /// `--name: <raw text>` custom-property declarations on this rule,
    /// in source order, name kept exactly as written (custom property
    /// names are case-*sensitive*, unlike every other property this file
    /// matches via `eq_ignore_ascii_case`). The raw value text is kept
    /// unparsed/unresolved here — `var()` substitution happens as a
    /// per-element pass in `resolve_style` (`var_decls`), not here; see
    /// its own docs for why and for the "`:root`/`html`-scoped only, no
    /// per-element override chain" simplification this implies.
    pub(crate) custom_props: Vec<(String, String)>,
    /// Declarations whose value mentions `var(` (Q13): `(name, raw
    /// value text)`, deferred until `resolve_style` knows the element's
    /// custom properties, then re-parsed with the calls substituted.
    pub(crate) var_decls: Vec<(String, String)>,
    /// CSS-wide keywords (`inherit`/`initial`/`unset`/`revert`) declared
    /// for a property, keyed by `wide_key`. Mutually exclusive with that
    /// property's typed field on the same `Declarations`: whichever was
    /// declared last in the rule wins (`set_wide`/`clear_wide`).
    pub(crate) wide: Vec<(&'static str, CssWide)>,
    pub(crate) color: Option<CssColor>,
    pub(crate) background_color: Option<CssColor>,
    /// The raw `url(...)` argument, unresolved — decoding (a `data:`
    /// URI only, same `<img src>` policy) happens once per matching
    /// element in `resolve_background_image`, not here, since a
    /// decoded image can't live in `ComputedStyle` (it's `Copy`; see
    /// `resolve_background_image`'s own docs). `Some(BackgroundImageDecl
    /// ::None)` for an explicit `background-image: none` — see that
    /// type's own docs for why a plain `Option<String>` couldn't
    /// represent that distinctly from "not declared" (same shape
    /// `box-shadow`'s own `ShadowDecl` already established).
    pub(crate) background_image: Option<BackgroundImageDecl>,
    /// `mask-image` / `-webkit-mask-image` and their layout longhands (Q29).
    pub(crate) mask_image: Option<BackgroundImageDecl>,
    pub(crate) mask_position: Option<BgPosDecl>,
    pub(crate) mask_size: Option<BgSizeDecl>,
    pub(crate) mask_repeat: Option<BgRepeat>,
    /// `background-position`/`-size`/`-repeat` (Q16): only the first
    /// layer of a comma list is kept. Resolved in `resolve_style` into
    /// `ComputedStyle::bg_layout`.
    pub(crate) background_position: Option<BgPosDecl>,
    pub(crate) background_size: Option<BgSizeDecl>,
    pub(crate) background_repeat: Option<BgRepeat>,
    /// `object-fit` / `object-position` (Q28) for `<img>`.
    pub(crate) object_fit: Option<ObjectFit>,
    pub(crate) object_position: Option<BgPosDecl>,
    pub(crate) display: Option<Display>,
    pub(crate) position: Option<Position>,
    pub(crate) float: Option<Float>,
    pub(crate) clear: Option<Clear>,
    pub(crate) top: Option<Length>,
    pub(crate) right: Option<Length>,
    pub(crate) bottom: Option<Length>,
    pub(crate) left: Option<Length>,
    pub(crate) z_index: Option<i32>,
    pub(crate) opacity: Option<f32>,
    /// `content` (Q19): only string content (and `none`/`normal`) parses.
    /// Only read for `::before`/`::after`.
    pub(crate) content: Option<ContentDecl>,
    /// `counter-reset` / `counter-increment` (Q19b); read only by `precompute_generated`.
    pub(crate) counter_reset: Option<Vec<(String, i32)>>,
    pub(crate) counter_increment: Option<Vec<(String, i32)>>,
    /// `counter-set` (Q19f) and `quotes` (Q19f; empty = `none`, inherited).
    pub(crate) counter_set: Option<Vec<(String, i32)>>,
    pub(crate) quotes: Option<Vec<(String, String)>>,
    pub(crate) width: Option<Length>,
    pub(crate) height: Option<Length>,
    pub(crate) min_width: Option<Length>,
    pub(crate) max_width: Option<MaxLen>,
    pub(crate) min_height: Option<Length>,
    pub(crate) max_height: Option<MaxLen>,
    pub(crate) box_sizing: Option<BoxSizing>,
    pub(crate) margin_top: Option<Length>,
    pub(crate) margin_right: Option<Length>,
    pub(crate) margin_bottom: Option<Length>,
    pub(crate) margin_left: Option<Length>,
    pub(crate) padding_top: Option<Length>,
    pub(crate) padding_right: Option<Length>,
    pub(crate) padding_bottom: Option<Length>,
    pub(crate) padding_left: Option<Length>,
    /// Per-side (top, right, bottom, left), see `SIDE_NAMES`.
    pub(crate) border_width: [Option<Length>; 4],
    pub(crate) border_color: [Option<CssColor>; 4],
    pub(crate) border_style: [Option<BorderStyle>; 4],
    pub(crate) font_size: Option<Length>,
    pub(crate) font_weight: Option<FontWeightValue>,
    pub(crate) font_style: Option<FontStyle>,
    pub(crate) font_family: Option<FontFamily>,
    pub(crate) text_decoration: Option<DecorationValue>,
    /// `text-decoration-line`/`-style`/`-color`, declared independently of
    /// the `text-decoration` shorthand (v60) — see `DecorationLine`'s own
    /// docs and `resolve_own_decoration` for how these three combine with
    /// each other and with whatever the shorthand (or an ambient
    /// propagated value) already set, at used-value time.
    pub(crate) text_decoration_line: Option<DecorationLine>,
    pub(crate) text_decoration_style: Option<DecorationStyle>,
    pub(crate) text_decoration_color: Option<CssColor>,
    pub(crate) white_space: Option<WhiteSpace>,
    pub(crate) word_break: Option<WordBreak>,
    pub(crate) text_transform: Option<TextTransform>,
    pub(crate) list_style_type: Option<ListStyleType>,
    /// `display: list-item` (true) or any other `display` (false).
    pub(crate) list_item: Option<bool>,
    /// `display: flow-root` (true) or any other `display` (false) — Q34b.
    pub(crate) flow_root: Option<bool>,
    /// `display: table-caption` (true) or any other `display` (false) — Q34c.
    pub(crate) table_caption: Option<bool>,
    /// `caption-side: bottom` (true) / `top` (false) — Q34c. Not inherited.
    pub(crate) caption_bottom: Option<bool>,
    pub(crate) text_align: Option<TextAlign>,
    pub(crate) vertical_align: Option<VerticalAlign>,
    pub(crate) line_height: Option<LineHeight>,
    /// `normal` (parsed as `Length::Px(0.0)`, same "no extra spacing"
    /// representation as an explicit `0`) or a plain `<length>` — no
    /// `<percentage>` (same grammar `parse_length`, not `parse_length_
    /// percent`, already gives `border-width`/`outline-width`; real CSS
    /// doesn't allow one here either, at least not before Level 3, which
    /// nothing here targets). See `ComputedStyle::word_spacing`'s own
    /// docs for where this is actually applied — deliberately *not* fed
    /// into cosmic-text's own `Attrs`, unlike what `letter-spacing`
    /// (still reverted, see TODO.txt) tried.
    pub(crate) word_spacing: Option<Length>,
    pub(crate) visibility: Option<Visibility>,
    pub(crate) outline_width: Option<Length>,
    pub(crate) outline_color: Option<CssColor>,
    /// `Some(ShadowDecl::None)` for an explicit `box-shadow: none` — a
    /// real, win-the-cascade-at-this-specificity declaration, distinct
    /// from `None` here (property not declared by this rule at all).
    /// See `ShadowDecl`'s own docs for why a plain `Option<BoxShadow>`
    /// can't tell those two cases apart.
    pub(crate) box_shadow: Option<ShadowDecl>,
    pub(crate) border_radius: Option<Length>,
    /// `overflow` (shorthand) and `overflow-x` both land here: the x axis.
    pub(crate) overflow: Option<Overflow>,
    /// `overflow-y` (Q21); the `overflow` shorthand sets it as well.
    pub(crate) overflow_y: Option<Overflow>,
    pub(crate) text_overflow: Option<TextOverflow>,
    pub(crate) cursor: Option<CursorStyle>,
    pub(crate) flex_direction: Option<FlexDirection>,
    pub(crate) flex_wrap: Option<FlexWrap>,
    pub(crate) justify_content: Option<JustifyContent>,
    pub(crate) align_content: Option<AlignContent>,
    pub(crate) align_items: Option<AlignItems>,
    /// `None` here means "not declared" *and* the real `auto` keyword
    /// (an item's `align-self: auto` falls back to its container's own
    /// `align-items` — see `ComputedStyle::align_self`'s own docs) —
    /// same "both collapse to the same thing" simplification `z_index`/
    /// `top`/etc. already use for their own `auto` keyword.
    pub(crate) align_self: Option<AlignItems>,
    pub(crate) flex_grow: Option<f32>,
    pub(crate) flex_shrink: Option<f32>,
    pub(crate) flex_basis: Option<FlexBasis>,
    /// `order` (integer): flex items lay out in ascending order, ties
    /// keep source order. Only read by `measure_flex`.
    pub(crate) order: Option<i32>,
    pub(crate) row_gap: Option<Length>,
    pub(crate) column_gap: Option<Length>,
    /// Leaked (`Box::leak`, once per distinct parsed declaration) into
    /// a `&'static` slice — see `grid-template-columns`'s own
    /// `DeclParser` branch and `GridTrackSpec`'s own docs.
    pub(crate) grid_template_columns: Option<&'static [GridTrackSpec]>,
    pub(crate) grid_template_rows: Option<&'static [GridTrackSpec]>,
    /// One leaked `&'static str` per template row — see `parse_grid_
    /// template_areas`'s own docs for why the raw strings, not an
    /// already-split cell grid, are what gets stored and leaked.
    pub(crate) grid_template_areas: Option<&'static [&'static str]>,
    /// This *item's* own named placement — see `parse_grid_area_name`'s
    /// own docs for the scoped-down grammar. Unlike every other field in
    /// this group, which describes a `display: grid` *container*, this
    /// one is read off a grid *item* (a container's own child) in
    /// `measure_grid`, the same "a flex-item-only property, nothing to
    /// do with the container declaring it" split `align-self`/`flex-
    /// grow`/etc. already have.
    pub(crate) grid_area: Option<&'static str>,
    /// Numeric placement longhands (`grid-row-start` etc.), see `GridLine`.
    pub(crate) grid_row_start: Option<GridLine>,
    pub(crate) grid_row_end: Option<GridLine>,
    pub(crate) grid_column_start: Option<GridLine>,
    pub(crate) grid_column_end: Option<GridLine>,
}

/// `normal` (the default — current behavior, collapse+wrap), `pre`
/// (`default.css`'s only real use, on `pre`/`listing`/`xmp`/`plaintext`),
/// and `nowrap` (collapses whitespace the same as `normal` — see
/// `collect_children`'s own `white_space == Pre` check, deliberately
/// left as an exact-match against `Pre` alone so `Nowrap` falls into
/// the same collapsing branch `normal` already uses — but never wraps
/// to a new line, same as `pre`). No `pre-wrap`/`pre-line`, nothing
/// declares them. `nowrap` is only honored in the plain-text fast path
/// (`shape_spans`'s own `pre: bool` parameter, despite its name, now
/// really means "disable wrapping" — both `Pre` and `Nowrap` pass
/// `true`) — a mixed run with an inline `<img>`/inline-block doesn't
/// look at `white_space` at all yet, same pre-existing gap `white-space:
/// pre` already has there (see TODO.txt).
#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum WhiteSpace {
    #[default]
    Normal,
    Pre,
    Nowrap,
    /// `pre-wrap` (Q20f): whitespace preserved like `pre`, lines still wrap.
    PreWrap,
    /// `pre-line` (Q20f): spaces collapse, newlines are kept, lines wrap.
    PreLine,
}

/// `word-break`/`overflow-wrap` (Q20d), one inherited value. cosmic-text's
/// default `Wrap::WordOrGlyph` already breaks a word that is too long for
/// its line, i.e. what `overflow-wrap: break-word`/`anywhere` ask for, so
/// `BreakWord` is the same as `Normal` here (a deliberate deviation from
/// real CSS, where plain `normal` overflows); only `BreakAll` changes
/// shaping (`Wrap::Glyph`).
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(crate) enum WordBreak {
    #[default]
    Normal,
    BreakWord,
    BreakAll,
}

/// `text-transform` (Q20e), inherited; applied to the text when the
/// inline items are collected, before shaping.
#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

/// The text of one DOM text node as an inline item should carry it:
/// whitespace per `white-space`, then `text-transform`.
pub(crate) fn inline_text(raw: &str, ws: WhiteSpace, tt: TextTransform) -> String {
    let mut text = match ws {
        WhiteSpace::Pre | WhiteSpace::PreWrap => raw.to_string(),
        WhiteSpace::PreLine => raw
            .split('\n')
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => {
            let mut t = normalize_whitespace(raw);
            t.push(' ');
            t
        }
    };
    match tt {
        TextTransform::None => {}
        TextTransform::Uppercase => text = text.to_uppercase(),
        TextTransform::Lowercase => text = text.to_lowercase(),
        TextTransform::Capitalize => {
            let mut start = true;
            text = text
                .chars()
                .flat_map(|c| {
                    let up = start && c.is_alphanumeric();
                    start = !c.is_alphanumeric() && c != '\'';
                    if up { c.to_uppercase().collect::<Vec<_>>() } else { vec![c] }
                })
                .collect();
        }
    }
    text
}

/// The 4 keywords `default.css` actually declares (`disc`/`decimal`
/// directly on `ul`/`ol`, `circle`/`square` for nested lists by depth),
/// plus `None` (no marker at all — CSS-gap report: 40 occurrences,
/// almost always `list-style: none` on a nav/menu `<ul>`) — no the-many-
/// other-counter-styles (roman numerals, alpha, ...). `Disc` as
/// `#[default]` matches the real CSS initial value.
#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum ListStyleType {
    #[default]
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    LowerGreek,
    None,
}

impl ListStyleType {
    /// The `counter()` style name behind a numbered type (Q20c).
    pub(crate) fn counter_style(self) -> Option<&'static str> {
        Some(match self {
            ListStyleType::Decimal => "decimal",
            ListStyleType::LowerAlpha => "lower-alpha",
            ListStyleType::UpperAlpha => "upper-alpha",
            ListStyleType::LowerRoman => "lower-roman",
            ListStyleType::UpperRoman => "upper-roman",
            ListStyleType::LowerGreek => "lower-greek",
            _ => return None,
        })
    }

    /// Parses a `list-style-type` keyword.
    pub(crate) fn from_keyword(id: &str) -> Option<ListStyleType> {
        Some(match id.to_ascii_lowercase().as_str() {
            "disc" => ListStyleType::Disc,
            "circle" => ListStyleType::Circle,
            "square" => ListStyleType::Square,
            "decimal" => ListStyleType::Decimal,
            "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
            "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
            "lower-roman" => ListStyleType::LowerRoman,
            "upper-roman" => ListStyleType::UpperRoman,
            "lower-greek" => ListStyleType::LowerGreek,
            "none" => ListStyleType::None,
            _ => return None,
        })
    }
}

/// The 4 keywords `text-align` actually has (no `start`/`end` — nothing
/// declares them, and they're writing-mode-relative, which scratchy has
/// no notion of). `Left` as `#[default]` matches the real CSS initial
/// value (for left-to-right content, which is all scratchy renders).
#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum TextAlign {
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
    pub(crate) fn to_cosmic(self) -> Option<CosmicAlign> {
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
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum VerticalAlign {
    #[default]
    Baseline,
    Sub,
    Super,
    Top,
    Middle,
    Bottom,
    TextTop,
    TextBottom,
}

impl VerticalAlign {
    /// Vertical offset (from the line's top) of an inline replaced/
    /// inline-block box of height `h` on a line `line_height` tall.
    /// `Middle` centers it, `Top`/`TextTop` pin it to the line top, every
    /// other value (`Baseline`, `Sub`, `Super`, `Bottom`, `TextBottom`)
    /// keeps the original bottom alignment — a documented approximation
    /// of the real baseline/text-bottom rules.
    pub(crate) fn inline_box_dy(self, line_height: f32, h: f32) -> f32 {
        match self {
            VerticalAlign::Middle => (line_height - h) / 2.0,
            VerticalAlign::Top | VerticalAlign::TextTop => 0.0,
            _ => line_height - h,
        }
    }

    /// Pixels to shift a glyph's baseline by (positive = down), scaled to
    /// that glyph's own font size so it still looks right if `font-size`
    /// ever varies within a sub/sup span. Not real font-metric
    /// `subscriptYOffset`/`superscriptYOffset` values (most fonts don't
    /// expose those through `fontdb`/`cosmic-text` at all) — just the
    /// same rough fractions browsers fall back to when a font doesn't
    /// supply its own.
    pub(crate) fn y_shift(self, font_size: f32) -> f32 {
        match self {
            VerticalAlign::Baseline => 0.0,
            VerticalAlign::Sub => font_size * 0.2,
            VerticalAlign::Super => font_size * -0.35,
            // Documented approximation: text spans treat the box-relative
            // keywords as `baseline`.
            _ => 0.0,
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
#[derive(Clone, Copy, PartialEq, Default, Debug)]
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
/// `text-overflow` (Q21): `Ellipsis` replaces the clipped tail of a
/// single-line (`nowrap`) text run with "…".
#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}

#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum Overflow {
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
pub(crate) enum BoxSizing {
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
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum MaxLen {
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
    pub(crate) fn resolve(self, font_size: f32, root_font_size: f32) -> Option<BoxLength> {
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
    pub(crate) fn to_winit(self) -> Option<CursorIcon> {
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
pub(crate) fn cursor_style_from_keyword(kw: &str) -> Option<CursorStyle> {
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum FontFamilyItem {
    Name(&'static str),
    SansSerif,
    Serif,
    Monospace,
    Cursive,
    Fantasy,
}

impl FontFamilyItem {
    pub(crate) fn to_cosmic(self) -> CosmicFamily<'static> {
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct FontFamily(pub(crate) &'static [FontFamilyItem]);

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
    ///
    /// **Memoized, thread-local** — added once real-world testing traced
    /// a measured 17x slowdown (a synthetic diagnostic: 300 paragraphs
    /// of plain text vs. the same text with one inline image dropped
    /// into each paragraph) to this call. `fontdb::Database::query`
    /// (confirmed by reading its vendored source) does a full linear
    /// scan over *every* installed font face, allocating a `Vec` of
    /// candidates, on every call — fine when paid once or twice per
    /// paragraph (the ordinary whole-paragraph shaping path), but
    /// `measure_mixed_inline`'s own word-by-word shaping (needed for any
    /// paragraph with an inline `<img>` mixed into its text — see its
    /// own docs) re-resolves the *same* `SpanStyle`'s font on every
    /// single word instead of once per paragraph, the dominant cost
    /// behind that 17x. Thread-local rather than a cache threaded
    /// through every caller as a new parameter: each thread in this
    /// process builds exactly one `FontSystem`/`Database` for its own
    /// lifetime (`App::font_system` on the main thread, a second
    /// independent one on `spawn_shaping_worker`'s own thread — see its
    /// own docs) and never swaps it for a different one, so nothing
    /// cached here can go stale within a run. Keyed without the
    /// `Database` itself: correct as long as the set of fonts actually
    /// installed on the system doesn't change mid-process — an
    /// assumption every `new_font_system()` call already makes, reading
    /// the same system font directories regardless of which thread or
    /// instance asks.
    pub(crate) fn resolve(self, db: &fontdb::Database, weight: FontWeight, style: FontStyle) -> CosmicFamily<'static> {
        thread_local! {
            static CACHE: RefCell<HashMap<(FontFamily, FontWeight, FontStyle), CosmicFamily<'static>>> =
                RefCell::new(HashMap::new());
        }
        let key = (self, weight, style);
        if let Some(cached) = CACHE.with(|c| c.borrow().get(&key).copied()) {
            return cached;
        }
        let mut resolved = CosmicFamily::SansSerif;
        for item in self.0.iter().copied() {
            let family = item.to_cosmic();
            let query = fontdb::Query {
                families: std::slice::from_ref(&family),
                weight,
                style,
                ..Default::default()
            };
            if db.query(&query).is_some() {
                resolved = family;
                break;
            }
        }
        CACHE.with(|c| c.borrow_mut().insert(key, resolved));
        resolved
    }
}

pub(crate) struct StyleRule {
    pub(crate) selectors: SelectorList<OurSelectorImpl>,
    pub(crate) decls: Declarations,
    /// The `!important` declarations of this rule, kept apart from
    /// `decls` so the cascade can put them in their own, stronger tier
    /// (see `IMPORTANT_TIER` and `cascade_property`).
    pub(crate) important: Declarations,
    /// Zero or more comma-separated `@media` conditions this rule is
    /// wrapped in (comma = OR between entries; `and` is folded into one
    /// entry's own fields, real Media Queries list grammar). An empty
    /// `Vec` means "not inside any `@media` at all" — always applies,
    /// independent of viewport width, exactly as before this existed.
    /// See `media_list_matches`.
    pub(crate) media: Vec<MediaQuery>,
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
pub(crate) enum MediaType {
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
pub(crate) enum PrefersColorScheme {
    Light,
    Dark,
}

/// Same story as `PrefersColorScheme`: scratchy has no OS reduced-
/// motion setting to read, and (per this item's own scope) no
/// animations to reduce in the first place, making this largely moot
/// either way. Fixed, documented default: always `no-preference`
/// (`reduce` never matches).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrefersReducedMotion {
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
pub(crate) struct MediaQuery {
    pub(crate) media_type: MediaType,
    pub(crate) min_width: Option<f32>,
    pub(crate) max_width: Option<f32>,
    pub(crate) prefers_color_scheme: Option<PrefersColorScheme>,
    pub(crate) prefers_reduced_motion: Option<PrefersReducedMotion>,
    pub(crate) never: bool,
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
    pub(crate) fn matches(&self, viewport_width: f32) -> bool {
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
pub(crate) fn media_list_matches(queries: &[MediaQuery], viewport_width: f32) -> bool {
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
pub(crate) fn parse_media_feature(input: &mut CssParser, q: &mut MediaQuery) -> Result<(), ()> {
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
pub(crate) fn media_feature_length_px(input: &mut CssParser) -> Result<f32, ()> {
    match parse_length(input)? {
        Length::Px(v) => Ok(v),
        Length::Em(v) | Length::Rem(v) => Ok(v * 16.0),
        // `parse_length` itself never actually produces any of these
        // (confirmed by its own match arms) — only reachable if that
        // changes later, in which case failing safely here is right
        // regardless of which new variant showed up.
        Length::Calc(c) => Ok(c.eval(16.0, 16.0, 0.0, current_viewport())),
        Length::Vp(..) => Err(()),
        Length::Percent(_) | Length::Larger | Length::Smaller | Length::Intrinsic(_) => Err(()),
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
pub(crate) fn parse_media_query_list(input: &mut CssParser) -> Vec<MediaQuery> {
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
pub(crate) fn parse_length(input: &mut CssParser) -> Result<Length, ()> {
    match input.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => Ok(Length::Px(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("em") => Ok(Length::Em(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("rem") => Ok(Length::Rem(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("pt") => Ok(Length::Px(value * PX_PER_PT)),
        Token::Dimension { value, unit, .. } if VpUnit::from_unit(unit).is_some() => {
            Ok(Length::Vp(VpUnit::from_unit(unit).unwrap(), *value))
        }
        Token::Number { value, .. } if *value == 0.0 => Ok(Length::Px(0.0)),
        Token::Function(name) => {
            let name = name.clone();
            parse_calc_function(input, &name, false)
        }
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
pub(crate) fn parse_length_percent(input: &mut CssParser) -> Result<Length, ()> {
    match input.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("px") => Ok(Length::Px(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("em") => Ok(Length::Em(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("rem") => Ok(Length::Rem(*value)),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("pt") => Ok(Length::Px(value * PX_PER_PT)),
        Token::Dimension { value, unit, .. } if VpUnit::from_unit(unit).is_some() => {
            Ok(Length::Vp(VpUnit::from_unit(unit).unwrap(), *value))
        }
        Token::Percentage { unit_value, .. } => Ok(Length::Percent(*unit_value)),
        Token::Number { value, .. } if *value == 0.0 => Ok(Length::Px(0.0)),
        Token::Function(name) => {
            let name = name.clone();
            parse_calc_function(input, &name, true)
        }
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
/// `width`/`min-width`: a length or an intrinsic-size keyword (Q20f).
pub(crate) fn set_width_length(input: &mut CssParser, target: &mut Option<Length>) {
    let kw = input
        .try_parse(|i| {
            let id = i.expect_ident().map_err(|_| ())?;
            match id.to_ascii_lowercase().as_str() {
                "min-content" => Ok(IntrinsicSize::MinContent),
                "max-content" | "-webkit-max-content" | "-moz-max-content" => Ok(IntrinsicSize::MaxContent),
                "fit-content" | "-webkit-fit-content" | "-moz-fit-content" => Ok(IntrinsicSize::FitContent),
                _ => Err(()),
            }
        })
        .ok();
    match kw {
        Some(k) => *target = Some(Length::Intrinsic(k)),
        None => set_length(input, target),
    }
}

pub(crate) fn set_length(input: &mut CssParser, target: &mut Option<Length>) {
    if let Ok(v) = parse_length_percent(input) {
        *target = Some(v);
    }
}

/// `max-width`/`max-height`'s own grammar: `none` (the real initial
/// value) alongside the usual `px`/`em`/`rem`/`%`/unitless-zero lengths —
/// same unparseable-leaves-target-untouched philosophy as `set_length`.
pub(crate) fn set_max_length(input: &mut CssParser, target: &mut Option<MaxLen>) {
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
/// `opacity`: a `<number>` or `<percentage>`, clamped to `0..=1`.
pub(crate) fn parse_opacity(input: &mut CssParser) -> Result<f32, ()> {
    match input.next().map_err(|_| ())? {
        Token::Number { value, .. } => Ok(value.clamp(0.0, 1.0)),
        Token::Percentage { unit_value, .. } => Ok(unit_value.clamp(0.0, 1.0)),
        _ => Err(()),
    }
}

pub(crate) fn parse_z_index(input: &mut CssParser) -> Result<i32, ()> {
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
pub(crate) fn parse_line_height(input: &mut CssParser) -> Result<LineHeight, ()> {
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
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("pt") => {
            Ok(LineHeight::Len(Length::Px(value * PX_PER_PT)))
        }
        Token::Percentage { unit_value, .. } => Ok(LineHeight::Len(Length::Percent(*unit_value))),
        _ => Err(()),
    }
}

/// Vendor-prefixed aliases (Q20b) that are plain synonyms of the standard
/// property: `-webkit-box-sizing` is `box-sizing`, and so on. Legacy
/// `-webkit-box` flexbox semantics are deliberately not implemented, and
/// `-webkit-transform` stays ignored along with `transform`.
pub(crate) fn unprefixed_property(name: &str) -> &str {
    const ALIASED: &[&str] = &[
        "box-sizing", "flex", "flex-direction", "flex-wrap", "flex-flow", "flex-grow",
        "flex-shrink", "flex-basis", "box-shadow", "justify-content", "align-items",
        "align-self", "order", "border-radius", "opacity",
    ];
    let lower = name.to_ascii_lowercase();
    for prefix in ["-webkit-", "-moz-"] {
        if let Some(rest) = lower.strip_prefix(prefix)
            && let Some(p) = ALIASED.iter().find(|p| **p == rest)
        {
            return p;
        }
    }
    name
}

/// `font` shorthand (Q20a): `[style] [variant] [weight] [stretch] size
/// [/ line-height] family`. The leading keywords may come in any order;
/// style/weight/line-height reset to their initial values when omitted.
/// System-font keywords (`caption`, `menu`, ...) fail to parse, so the
/// declaration is dropped.
pub(crate) fn parse_font_shorthand(
    input: &mut CssParser,
) -> Result<(FontStyle, FontWeightValue, Length, LineHeight, FontFamily), ()> {
    let mut style = FontStyle::Normal;
    let mut weight = FontWeightValue::Absolute(FontWeight::NORMAL);
    let size = loop {
        let state = input.state();
        let tok = input.next().map_err(|_| ())?.clone();
        match tok {
            Token::Ident(ref id) => {
                let id = id.to_ascii_lowercase();
                match id.as_str() {
                    "normal" | "small-caps" | "ultra-condensed" | "extra-condensed" | "condensed"
                    | "semi-condensed" | "semi-expanded" | "expanded" | "extra-expanded"
                    | "ultra-expanded" => {}
                    "italic" => style = FontStyle::Italic,
                    "oblique" => style = FontStyle::Oblique,
                    "bold" | "bolder" | "lighter" => {
                        input.reset(&state);
                        weight = parse_font_weight(input)?;
                    }
                    _ => {
                        let px = font_size_keyword(&id).ok_or(())?;
                        break Length::Px(px);
                    }
                }
            }
            Token::Number { value, .. } => weight = FontWeightValue::Absolute(FontWeight(value.round() as u16)),
            _ => {
                input.reset(&state);
                break parse_length_percent(input)?;
            }
        }
    };
    let line_height = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        parse_line_height(input)?
    } else {
        LineHeight::Normal
    };
    let items = parse_font_family(input)?;
    Ok((style, weight, size, line_height, FontFamily(Box::leak(items.into_boxed_slice()))))
}

/// The absolute `font-size` keywords, in px.
pub(crate) fn font_size_keyword(id: &str) -> Option<f32> {
    Some(match id {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        _ => return None,
    })
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
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FontWeightValue {
    Absolute(FontWeight),
    Bolder,
    Lighter,
}

impl FontWeightValue {
    /// The CSS Fonts step table for relative weights (CSS Fonts Level 4
    /// §font-weight-prop) — not a simple ratio the way `font-size`'s
    /// `larger`/`smaller` use, since real browsers converge on these
    /// specific breakpoints rather than a multiply.
    pub(crate) fn resolve(self, inherited: FontWeight) -> FontWeight {
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

pub(crate) fn parse_font_weight(input: &mut CssParser) -> Result<FontWeightValue, ()> {
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
pub(crate) struct DecorationLine {
    pub(crate) underline: bool,
    pub(crate) strikethrough: bool,
    pub(crate) overline: bool,
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
#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) struct DecorationValue {
    pub(crate) deco: TextDecoration,
    pub(crate) style: DecorationStyle,
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
pub(crate) fn resolve_own_decoration(style: &ComputedStyle, ambient: DecorationValue) -> DecorationValue {
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
pub(crate) fn parse_font_family(input: &mut CssParser) -> Result<Vec<FontFamilyItem>, ()> {
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
pub(crate) fn decoration_style_from_keyword(kw: &str) -> Option<DecorationStyle> {
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

pub(crate) fn parse_text_decoration(input: &mut CssParser) -> Result<(TextDecoration, DecorationStyle), ()> {
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
pub(crate) fn parse_text_decoration_style(input: &mut CssParser) -> Result<DecorationStyle, ()> {
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
pub(crate) fn parse_text_decoration_line(input: &mut CssParser) -> Result<DecorationLine, ()> {
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
pub(crate) enum BorderStyle {
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

/// Side order for every per-side border array: top, right, bottom, left
/// (CSS box-syntax order).
pub(crate) const SIDE_NAMES: [&str; 4] = ["top", "right", "bottom", "left"];

/// 1-to-4-value CSS box syntax (`a`, `a b`, `a b c`, `a b c d`) expanded to
/// `[top, right, bottom, left]`. `one` parses a single value.
pub(crate) fn parse_box4<T: Copy>(input: &mut CssParser, mut one: impl FnMut(&mut CssParser) -> Option<T>) -> Option<[T; 4]> {
    let mut v: Vec<T> = vec![one(input)?];
    while v.len() < 4 {
        match one(input) {
            Some(x) => v.push(x),
            None => break,
        }
    }
    Some(match v.len() {
        1 => [v[0]; 4],
        2 => [v[0], v[1], v[0], v[1]],
        3 => [v[0], v[1], v[2], v[1]],
        _ => [v[0], v[1], v[2], v[3]],
    })
}

/// Handles `border`, `border-{width,color,style}` (box syntax),
/// `border-{top,right,bottom,left}` shorthands and the 12 side longhands.
/// Returns false for any other `border-*` name (e.g. `border-radius`).
pub(crate) fn parse_border_property(decls: &mut Declarations, name: &str, input: &mut CssParser) -> bool {
    let lname = name.to_ascii_lowercase();
    let Some(rest) = lname.strip_prefix("border") else { return false };
    let set_shorthand = |decls: &mut Declarations, sides: &[usize], parsed: BorderShorthand| {
        let (w, st, c) = parsed;
        for &i in sides {
            if let Some(w) = w {
                decls.border_width[i] = Some(w);
            }
            if let Some(st) = st {
                decls.border_style[i] = Some(st);
            }
            if let Some(c) = c {
                decls.border_color[i] = Some(c);
            }
        }
    };
    match rest {
        "" => {
            if let Ok(p) = parse_border_shorthand(input) {
                set_shorthand(decls, &[0, 1, 2, 3], p);
            }
        }
        "-width" => {
            if let Some(v) = parse_box4(input, |i| parse_length(i).ok()) {
                for (i, x) in v.into_iter().enumerate() {
                    decls.border_width[i] = Some(x);
                }
            }
        }
        "-style" => {
            if let Some(v) = parse_box4(input, |i| i.expect_ident().ok().map(|s| border_style_from_keyword(s))) {
                for (i, x) in v.into_iter().enumerate() {
                    decls.border_style[i] = Some(x);
                }
            }
        }
        "-color" => {
            let mut colors: Vec<CssColor> = Vec::new();
            while colors.len() < 4 {
                match CssColor::parse(input) {
                    Ok(c) => colors.push(c),
                    Err(_) => break,
                }
            }
            let n = colors.len();
            if n > 0 {
                let idx: [usize; 4] = match n {
                    1 => [0, 0, 0, 0],
                    2 => [0, 1, 0, 1],
                    3 => [0, 1, 2, 1],
                    _ => [0, 1, 2, 3],
                };
                for (i, k) in idx.into_iter().enumerate() {
                    decls.border_color[i] = Some(colors[k]);
                }
            }
        }
        _ => {
            let Some(rest) = rest.strip_prefix('-') else { return false };
            let (side, prop) = match rest.split_once('-') {
                Some((a, b)) => (a, Some(b)),
                None => (rest, None),
            };
            let Some(i) = SIDE_NAMES.iter().position(|n| *n == side) else { return false };
            match prop {
                None => {
                    if let Ok(p) = parse_border_shorthand(input) {
                        set_shorthand(decls, &[i], p);
                    }
                }
                Some("width") => {
                    if let Ok(v) = parse_length(input) {
                        decls.border_width[i] = Some(v);
                    }
                }
                Some("style") => {
                    if let Ok(id) = input.expect_ident() {
                        decls.border_style[i] = Some(border_style_from_keyword(id));
                    }
                }
                Some("color") => {
                    if let Ok(c) = CssColor::parse(input) {
                        decls.border_color[i] = Some(c);
                    }
                }
                _ => return false,
            }
        }
    }
    true
}

pub(crate) fn border_style_from_keyword(kw: &str) -> BorderStyle {
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
pub(crate) type BorderShorthand = (Option<Length>, Option<BorderStyle>, Option<CssColor>);

pub(crate) fn parse_border_shorthand(input: &mut CssParser) -> Result<BorderShorthand, ()> {
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
pub(crate) fn parse_outline_shorthand(input: &mut CssParser) -> Result<(Option<Length>, Option<CssColor>), ()> {
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
pub(crate) type BorderLikeShorthand<'k> = (Option<Length>, Option<&'k str>, Option<CssColor>);

pub(crate) fn parse_border_like_shorthand<'k>(
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

/// `list-style`'s own grammar: `<list-style-type> || <list-style-position>
/// || <list-style-image>`, any order, each 0-1 times. Only the type
/// component is tracked — `list-style-position`/`list-style-image` aren't
/// implemented as their own longhands either (see TODO.txt), so a
/// position keyword or a `url(...)` image is skipped rather than failing
/// the whole value, same lenient philosophy as `parse_border_like_
/// shorthand`. A bare `none` (far and away the real-world common case —
/// hiding a nav/menu `<ul>`'s marker) resolves as `ListStyleType::None`.
pub(crate) fn parse_list_style_shorthand(input: &mut CssParser) -> Result<ListStyleType, ()> {
    let mut style_type = None;
    loop {
        let state = input.state();
        if let Ok(Token::Ident(ident)) = input.next() {
            if let Some(ty) = ListStyleType::from_keyword(ident) {
                style_type = Some(ty);
                continue;
            }
        }
        input.reset(&state);
        match input.next() {
            Ok(_) => {}
            Err(_) => break,
        }
    }
    style_type.ok_or(())
}

/// One axis of a `background-position`: `pct` of the free space
/// (box size minus tile size) plus a fixed `off` offset. `left`/`top`
/// are `0%`, `center` `50%`, `right`/`bottom` `100%`; a bare percentage
/// is `pct`, any other length is `off` (with `pct` 0).
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct BgAxisDecl {
    pub(crate) pct: f32,
    pub(crate) off: Length,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct BgPosDecl {
    pub(crate) x: BgAxisDecl,
    pub(crate) y: BgAxisDecl,
}

impl Default for BgPosDecl {
    fn default() -> Self {
        let z = BgAxisDecl { pct: 0.0, off: Length::Px(0.0) };
        BgPosDecl { x: z, y: z }
    }
}

/// `object-fit` (Q28): how a replaced `<img>`'s picture sits in its box.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ObjectFit {
    #[default]
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

impl ObjectFit {
    pub(crate) fn from_keyword(k: &str) -> Option<ObjectFit> {
        Some(match k {
            "fill" => ObjectFit::Fill,
            "contain" => ObjectFit::Contain,
            "cover" => ObjectFit::Cover,
            "none" => ObjectFit::None,
            "scale-down" => ObjectFit::ScaleDown,
            _ => return Option::None,
        })
    }
}

/// One dimension of an explicit `background-size`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum BgDimDecl {
    Auto,
    Len(Length),
}

#[derive(Clone, Copy, PartialEq, Default)]
pub(crate) enum BgSizeDecl {
    #[default]
    Auto,
    Cover,
    Contain,
    Explicit(BgDimDecl, BgDimDecl),
}

/// `background-repeat` per axis. `space`/`round` are treated as
/// `repeat`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct BgRepeat {
    pub(crate) x: bool,
    pub(crate) y: bool,
}

impl Default for BgRepeat {
    fn default() -> Self {
        BgRepeat { x: true, y: true }
    }
}

/// Computed `background-size` dimension.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum BgDim {
    Auto,
    Px(f32),
    Pct(f32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum BgSize {
    Cover,
    Contain,
    Explicit(BgDim, BgDim),
}

/// Computed background layout (everything but the image itself):
/// position offsets are `(fraction of free space, px)` per axis.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct BgLayout {
    pub(crate) pos_x: (f32, f32),
    pub(crate) pos_y: (f32, f32),
    pub(crate) size: BgSize,
    pub(crate) repeat: BgRepeat,
}

impl Default for BgLayout {
    fn default() -> Self {
        BgLayout {
            pos_x: (0.0, 0.0),
            pos_y: (0.0, 0.0),
            size: BgSize::Explicit(BgDim::Auto, BgDim::Auto),
            repeat: BgRepeat::default(),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum BgPosItem {
    Kw(char), // 'l' 'r' 't' 'b' 'c'
    Len(Length),
}

pub(crate) fn bg_pos_item(input: &mut CssParser) -> Option<BgPosItem> {
    let state = input.state();
    match input.next() {
        Ok(Token::Ident(id)) => {
            let k = match id.to_ascii_lowercase().as_str() {
                "left" => 'l',
                "right" => 'r',
                "top" => 't',
                "bottom" => 'b',
                "center" => 'c',
                _ => {
                    input.reset(&state);
                    return None;
                }
            };
            Some(BgPosItem::Kw(k))
        }
        Ok(_) => {
            input.reset(&state);
            match parse_length_percent(input) {
                Ok(l) if !matches!(l, Length::Calc(c) if c.has_percent()) => Some(BgPosItem::Len(l)),
                _ => {
                    input.reset(&state);
                    None
                }
            }
        }
        Err(_) => None,
    }
}

pub(crate) fn bg_axis_from_len(l: Length) -> BgAxisDecl {
    match l {
        Length::Percent(p) => BgAxisDecl { pct: p, off: Length::Px(0.0) },
        other => BgAxisDecl { pct: 0.0, off: other },
    }
}

pub(crate) fn bg_axis_from_kw(k: char) -> BgAxisDecl {
    let pct = match k {
        'l' | 't' => 0.0,
        'r' | 'b' => 1.0,
        _ => 0.5,
    };
    BgAxisDecl { pct, off: Length::Px(0.0) }
}

pub(crate) fn negate_length(l: Length) -> Option<Length> {
    match l {
        Length::Px(v) => Some(Length::Px(-v)),
        Length::Em(v) => Some(Length::Em(-v)),
        Length::Rem(v) => Some(Length::Rem(-v)),
        Length::Vp(u, v) => Some(Length::Vp(u, -v)),
        _ => None,
    }
}

/// Parses the longest run of position components (1 to 4) from the
/// parser's current place. Handles the one/two-value forms and the
/// three/four-value edge-offset form (`right 10px bottom 20px`).
pub(crate) fn parse_bg_position_items(input: &mut CssParser) -> Result<BgPosDecl, ()> {
    let mut items = Vec::new();
    while items.len() < 4 {
        match bg_pos_item(input) {
            Some(i) => items.push(i),
            None => break,
        }
    }
    let is_x = |c: char| matches!(c, 'l' | 'r');
    let is_y = |c: char| matches!(c, 't' | 'b');
    let center = BgAxisDecl { pct: 0.5, off: Length::Px(0.0) };
    match items.as_slice() {
        [] => Err(()),
        [BgPosItem::Kw(k)] => Ok(if is_y(*k) {
            BgPosDecl { x: center, y: bg_axis_from_kw(*k) }
        } else {
            BgPosDecl { x: bg_axis_from_kw(*k), y: center }
        }),
        [BgPosItem::Len(l)] => Ok(BgPosDecl { x: bg_axis_from_len(*l), y: center }),
        [a, b] => {
            let swap = match (a, b) {
                (BgPosItem::Kw(a), BgPosItem::Kw(b)) => is_y(*a) || is_x(*b),
                (BgPosItem::Kw(a), BgPosItem::Len(_)) => is_y(*a),
                (BgPosItem::Len(_), BgPosItem::Kw(b)) => is_x(*b),
                _ => false,
            };
            let (a, b) = if swap { (b, a) } else { (a, b) };
            let ax = |i: &BgPosItem| match i {
                BgPosItem::Kw(k) => bg_axis_from_kw(*k),
                BgPosItem::Len(l) => bg_axis_from_len(*l),
            };
            // A keyword on the wrong axis (`left right`) is invalid.
            if matches!(a, BgPosItem::Kw(k) if is_y(*k)) || matches!(b, BgPosItem::Kw(k) if is_x(*k)) {
                return Err(());
            }
            Ok(BgPosDecl { x: ax(a), y: ax(b) })
        }
        _ => {
            // Edge-offset form: two groups of `<keyword> [<length>]`.
            let mut groups: Vec<(char, Option<Length>)> = Vec::new();
            let mut i = 0;
            while i < items.len() {
                let BgPosItem::Kw(k) = items[i] else { return Err(()) };
                if let Some(BgPosItem::Len(l)) = items.get(i + 1) {
                    groups.push((k, Some(*l)));
                    i += 2;
                } else {
                    groups.push((k, None));
                    i += 1;
                }
            }
            if groups.len() != 2 {
                return Err(());
            }
            let axis = |k: char, l: Option<Length>| -> Result<BgAxisDecl, ()> {
                match (k, l) {
                    ('c', None) => Ok(center),
                    ('c', Some(_)) => Err(()),
                    (_, None) => Ok(bg_axis_from_kw(k)),
                    (_, Some(Length::Percent(p))) => {
                        let base = bg_axis_from_kw(k).pct;
                        Ok(BgAxisDecl { pct: if matches!(k, 'r' | 'b') { base - p } else { base + p }, off: Length::Px(0.0) })
                    }
                    ('l' | 't', Some(l)) => Ok(BgAxisDecl { pct: 0.0, off: l }),
                    (_, Some(l)) => Ok(BgAxisDecl { pct: 1.0, off: negate_length(l).ok_or(())? }),
                }
            };
            let (g0, g1) = (groups[0], groups[1]);
            let (xg, yg) = if is_y(g0.0) || is_x(g1.0) { (g1, g0) } else { (g0, g1) };
            if is_y(xg.0) || is_x(yg.0) {
                return Err(());
            }
            Ok(BgPosDecl { x: axis(xg.0, xg.1)?, y: axis(yg.0, yg.1)? })
        }
    }
}

pub(crate) fn parse_bg_position(input: &mut CssParser) -> Result<BgPosDecl, ()> {
    parse_bg_position_items(input)
}

pub(crate) fn parse_bg_size_items(input: &mut CssParser) -> Result<BgSizeDecl, ()> {
    let state = input.state();
    if let Ok(Token::Ident(id)) = input.next() {
        let l = id.to_ascii_lowercase();
        if l == "cover" {
            return Ok(BgSizeDecl::Cover);
        }
        if l == "contain" {
            return Ok(BgSizeDecl::Contain);
        }
    }
    input.reset(&state);
    let dim = |input: &mut CssParser| -> Option<BgDimDecl> {
        let st = input.state();
        if let Ok(Token::Ident(id)) = input.next() {
            if id.eq_ignore_ascii_case("auto") {
                return Some(BgDimDecl::Auto);
            }
        }
        input.reset(&st);
        match parse_length_percent(input) {
            Ok(Length::Calc(c)) if c.has_percent() => {
                input.reset(&st);
                None
            }
            Ok(l) => Some(BgDimDecl::Len(l)),
            Err(_) => {
                input.reset(&st);
                None
            }
        }
    };
    let w = dim(input).ok_or(())?;
    let h = dim(input).unwrap_or(BgDimDecl::Auto);
    Ok(BgSizeDecl::Explicit(w, h))
}

pub(crate) fn parse_bg_size(input: &mut CssParser) -> Result<BgSizeDecl, ()> {
    parse_bg_size_items(input)
}

/// `repeat-x`/`repeat-y`/`repeat`/`no-repeat`, or one keyword per axis.
/// Returns `None` (without consuming) if the next token isn't one.
pub(crate) fn parse_bg_repeat_items(input: &mut CssParser) -> Result<BgRepeat, ()> {
    fn kw(input: &mut CssParser) -> Option<String> {
        let st = input.state();
        match input.next() {
            Ok(Token::Ident(id)) => {
                let l = id.to_ascii_lowercase();
                if matches!(l.as_str(), "repeat-x" | "repeat-y" | "repeat" | "no-repeat" | "space" | "round") {
                    return Some(l);
                }
                input.reset(&st);
                None
            }
            _ => {
                input.reset(&st);
                None
            }
        }
    }
    let a = kw(input).ok_or(())?;
    match a.as_str() {
        "repeat-x" => return Ok(BgRepeat { x: true, y: false }),
        "repeat-y" => return Ok(BgRepeat { x: false, y: true }),
        _ => {}
    }
    let on = |s: &str| s != "no-repeat";
    match kw(input) {
        Some(b) if !b.starts_with("repeat-") => Ok(BgRepeat { x: on(&a), y: on(&b) }),
        _ => Ok(BgRepeat { x: on(&a), y: on(&a) }),
    }
}

pub(crate) fn parse_bg_repeat(input: &mut CssParser) -> Result<BgRepeat, ()> {
    parse_bg_repeat_items(input)
}

/// What `parse_background_shorthand` found. `position`/`size`/`repeat`
/// are from the first layer only.
pub(crate) struct BgShorthand {
    pub(crate) color: Option<CssColor>,
    pub(crate) image: Option<String>,
    pub(crate) position: Option<BgPosDecl>,
    pub(crate) size: Option<BgSizeDecl>,
    pub(crate) repeat: Option<BgRepeat>,
}

/// `background`'s grammar: a color, an image, `<position> [/ <size>]`
/// and a repeat style, in any order. Attachment/origin/clip keywords and
/// anything else unrecognized are skipped rather than failing the whole
/// value (lenient, like `border`'s shorthand). Only the first layer of a
/// comma list contributes the image, position, size and repeat; a color
/// anywhere is taken.
pub(crate) fn parse_background_shorthand(input: &mut CssParser) -> Result<BgShorthand, ()> {
    let mut out = BgShorthand { color: None, image: None, position: None, size: None, repeat: None };
    let mut found = false;
    let mut first_layer = true;
    loop {
        let state = input.state();
        if let Ok(url) = input.expect_url() {
            if first_layer {
                out.image = Some(url.to_string());
            }
            found = true;
            continue;
        }
        input.reset(&state);
        if let Ok(c) = CssColor::parse(input) {
            out.color = Some(c);
            found = true;
            continue;
        }
        input.reset(&state);
        if let Ok(r) = parse_bg_repeat_items(input) {
            if first_layer {
                out.repeat = Some(r);
            }
            found = true;
            continue;
        }
        input.reset(&state);
        if let Ok(p) = parse_bg_position_items(input) {
            let mut size = None;
            let st = input.state();
            if matches!(input.next(), Ok(Token::Delim('/'))) {
                size = parse_bg_size_items(input).ok();
            } else {
                input.reset(&st);
            }
            if first_layer {
                out.position = Some(p);
                if size.is_some() {
                    out.size = size;
                }
            }
            found = true;
            continue;
        }
        input.reset(&state);
        match input.next() {
            Ok(Token::Comma) => first_layer = false,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    if found {
        Ok(out)
    } else {
        Err(())
    }
}

/// Most comma-separated shadows kept per box (extras are ignored).
/// `ComputedStyle` is `Copy`, so the list is a fixed array.
pub(crate) const MAX_SHADOWS: usize = 4;

/// A parsed (not-yet-resolved) `box-shadow` layer:
/// `inset? <offset-x> <offset-y> <blur>? <spread>? <color>?`. A missing
/// color means `currentColor`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct BoxShadow {
    pub(crate) offset_x: Length,
    pub(crate) offset_y: Length,
    pub(crate) blur: Length,
    pub(crate) spread: Length,
    /// `None` = no color given, which (like an explicit `currentColor`)
    /// resolves to the element's own `color`.
    pub(crate) color: Option<CssColor>,
    pub(crate) inset: bool,
}

/// `box-shadow`'s own declared value, as stored in `Declarations` —
/// either a real shadow list or the literal `none` keyword, which real
/// CSS needs to win the cascade at its own specificity (overriding a real
/// shadow a *less*-specific rule declared) exactly like any other
/// explicit value would. Plain `Option<BoxShadow>` can't represent
/// that: both "this rule said `none`" and "this rule never mentioned
/// `box-shadow` at all" would collapse to the same `None`, so `cascade_
/// property` (which skips a rule entirely when its own accessor
/// returns `None`) could never let an explicit `none` override a less-
/// specific real shadow.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ShadowDecl {
    None,
    Shadow([Option<BoxShadow>; MAX_SHADOWS]),
}

/// One shadow layer, up to the next top-level comma (consumed) or the end
/// of input. Returns the layer and whether more layers follow. A `<color>`
/// is accepted at any position, `inset` first or last; any other
/// unrecognized token drops the whole declaration.
pub(crate) fn parse_box_shadow_layer(input: &mut CssParser) -> Result<(BoxShadow, bool), ()> {
    let mut lengths: Vec<Length> = Vec::new();
    let mut color: Option<CssColor> = None;
    let mut inset = false;
    let more;
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
            Ok(Token::Comma) => {
                more = true;
                break;
            }
            Ok(Token::Ident(id)) if id.eq_ignore_ascii_case("inset") && !inset => inset = true,
            Ok(_) => return Err(()),
            Err(_) => {
                more = false;
                break;
            }
        }
    }
    if lengths.len() < 2 || lengths.len() > 4 {
        return Err(());
    }
    Ok((
        BoxShadow {
            offset_x: lengths[0],
            offset_y: lengths[1],
            blur: lengths.get(2).copied().unwrap_or(Length::Px(0.0)),
            spread: lengths.get(3).copied().unwrap_or(Length::Px(0.0)),
            color,
            inset,
        },
        more,
    ))
}

pub(crate) fn parse_box_shadow(input: &mut CssParser) -> Result<[Option<BoxShadow>; MAX_SHADOWS], ()> {
    let mut out = [None; MAX_SHADOWS];
    let mut n = 0;
    loop {
        let (layer, more) = parse_box_shadow_layer(input)?;
        if n < MAX_SHADOWS {
            out[n] = Some(layer);
            n += 1;
        }
        if !more {
            return Ok(out);
        }
    }
}

pub(crate) fn flex_direction_from_keyword(ident: &str) -> Option<FlexDirection> {
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

pub(crate) fn flex_wrap_from_keyword(ident: &str) -> Option<FlexWrap> {
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

pub(crate) fn parse_flex_wrap(input: &mut CssParser) -> Result<FlexWrap, ()> {
    let ident = input.expect_ident().map_err(|_| ())?;
    flex_wrap_from_keyword(ident).ok_or(())
}

/// `align-items`/`align-self`'s 5 keywords (minus `align-self`'s own
/// extra `auto`, handled separately — see its own `DeclParser` branch).
pub(crate) fn parse_align_keyword(input: &mut CssParser) -> Result<AlignItems, ()> {
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
pub(crate) fn parse_nonneg_number(input: &mut CssParser) -> Result<f32, ()> {
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
pub(crate) fn parse_flex_basis(input: &mut CssParser) -> Result<FlexBasis, ()> {
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
pub(crate) fn parse_flex_shorthand(input: &mut CssParser) -> Result<(f32, f32, FlexBasis), ()> {
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

/// One `minmax()` argument's own narrower grammar — see
/// `TrackBreadth`'s own docs for why it's not just `GridTrackSpec`
/// again. Same `fr`-then-length trial order `parse_grid_track_size`
/// itself uses, since this is exactly that function's grammar minus
/// `minmax()` recursion.
pub(crate) fn parse_track_breadth(input: &mut CssParser) -> Result<TrackBreadth, ()> {
    if let Some(k) = parse_track_keyword(input) {
        return Ok(k);
    }
    let state = input.state();
    let fr = match input.next() {
        Ok(Token::Dimension { value, unit, .. }) if unit.eq_ignore_ascii_case("fr") => Some(*value),
        _ => None,
    };
    if let Some(v) = fr {
        return Ok(TrackBreadth::Fr(v));
    }
    input.reset(&state);
    parse_length_percent(input).map(TrackBreadth::Len)
}

/// One `<track-size>`: a `<length-percentage>` (reusing `parse_length_
/// percent`, the same px/%/em/rem grammar `width`/`margin`/etc. already
/// accept), `<flex>` — the `fr` unit, which tokenizes as an ordinary
/// `<dimension-token>` with unit `fr` (same as `px`/`em`/`rem` already
/// do — no special-case lexing needed, same `state`/`reset` trial
/// pattern `parse_flex_basis`/`set_max_length` already use to try one
/// grammar before falling back to another) — or `minmax(<track-
/// breadth>, <track-breadth>)` (see `GridTrackSpec::MinMax`'s own
/// docs). Content-based sizing keywords (`auto`/`min-content`/`max-
/// content`), bare or inside `minmax()`, still aren't recognized —
/// still deferred, see `TODO.txt`.
pub(crate) fn parse_grid_track_size(input: &mut CssParser) -> Result<GridTrackSpec, ()> {
    let state = input.state();
    if input.expect_function_matching("minmax").is_ok() {
        let (min, max): (TrackBreadth, TrackBreadth) = input
            .parse_nested_block(|input| {
                let min = parse_track_breadth(input).map_err(|_| input.new_error_for_next_token::<()>())?;
                input.expect_comma()?;
                let max = parse_track_breadth(input).map_err(|_| input.new_error_for_next_token::<()>())?;
                Ok((min, max))
            })
            .map_err(|_: ParseError<()>| ())?;
        return Ok(GridTrackSpec::MinMax(min, max));
    }
    input.reset(&state);
    match parse_track_keyword(input) {
        Some(TrackBreadth::Auto) => return Ok(GridTrackSpec::Auto),
        Some(TrackBreadth::MinContent) => return Ok(GridTrackSpec::MinContent),
        Some(TrackBreadth::MaxContent) => return Ok(GridTrackSpec::MaxContent),
        _ => {}
    }
    let fr = match input.next() {
        Ok(Token::Dimension { value, unit, .. }) if unit.eq_ignore_ascii_case("fr") => Some(*value),
        _ => None,
    };
    if let Some(v) = fr {
        return Ok(GridTrackSpec::Fr(v));
    }
    input.reset(&state);
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
pub(crate) fn parse_nested_auto_repeat_keyword(input: &mut CssParser) -> Result<GridTrackSpec, ParseError<()>> {
    input.parse_nested_block(|input| {
        let fit = {
            let id = input.expect_ident()?;
            if id.eq_ignore_ascii_case("auto-fit") {
                true
            } else if id.eq_ignore_ascii_case("auto-fill") {
                false
            } else {
                return Err(input.new_error_for_next_token::<()>());
            }
        };
        input.expect_comma()?;
        let mut one = Vec::new();
        while !input.is_exhausted() {
            one.push(parse_grid_track_size(input).map_err(|_| input.new_error_for_next_token::<()>())?);
        }
        // Real CSS needs every track fixed-size (or minmax with a fixed
        // side) so a repetition width is knowable.
        if one.is_empty() || one.iter().any(|t| auto_repeat_track_px(t, 0.0, 0.0, 0.0).is_none()) {
            return Err(input.new_error_for_next_token::<()>());
        }
        Ok(GridTrackSpec::AutoRepeat { fit, tracks: Box::leak(one.into_boxed_slice()) })
    })
}

/// A track's definite size for counting `auto-fill`/`auto-fit`
/// repetitions: a length, or a `minmax()` with a length minimum (else
/// its length maximum). `None` when neither side is a length.
pub(crate) fn auto_repeat_track_px(t: &GridTrackSpec, available: f32, font_size: f32, root_font_size: f32) -> Option<f32> {
    let len = |l: Length| match l {
        Length::Percent(p) => available * p,
        l => l.resolve(font_size, root_font_size),
    };
    match *t {
        GridTrackSpec::Len(l) => Some(len(l)),
        GridTrackSpec::MinMax(TrackBreadth::Len(l), _) | GridTrackSpec::MinMax(_, TrackBreadth::Len(l)) => Some(len(l)),
        _ => None,
    }
}

/// Expands an `AutoRepeat` entry into as many repetitions as fit in
/// `available` (at least one). Returns the flat list and, for
/// `auto-fit`, the index range of the repeated tracks.
pub(crate) fn expand_auto_repeat(
    tracks: &[GridTrackSpec],
    available: f32,
    gap: f32,
    font_size: f32,
    root_font_size: f32,
) -> (Vec<GridTrackSpec>, Option<(usize, usize)>) {
    let Some(pos) = tracks.iter().position(|t| matches!(t, GridTrackSpec::AutoRepeat { .. })) else {
        return (tracks.to_vec(), None);
    };
    let GridTrackSpec::AutoRepeat { fit, tracks: rep } = tracks[pos] else { unreachable!() };
    let others: Vec<f32> = tracks
        .iter()
        .filter(|t| !matches!(t, GridTrackSpec::AutoRepeat { .. }))
        .map(|t| auto_repeat_track_px(t, available, font_size, root_font_size).unwrap_or(0.0))
        .collect();
    let rep_px: f32 = rep.iter().filter_map(|t| auto_repeat_track_px(t, available, font_size, root_font_size)).sum();
    let k = rep.len() as f32;
    let fixed_total: f32 = others.iter().sum::<f32>() + gap * others.len() as f32;
    // n repetitions occupy n*(rep_px + k*gap) plus the other tracks.
    let per = rep_px + k * gap;
    let n = if per > 0.0 { (((available - fixed_total) / per).floor().max(1.0)) as usize } else { 1 };
    let mut out = Vec::with_capacity(tracks.len() + rep.len() * n);
    out.extend_from_slice(&tracks[..pos]);
    for _ in 0..n {
        out.extend_from_slice(rep);
    }
    out.extend_from_slice(&tracks[pos + 1..]);
    (out, fit.then_some((pos, pos + rep.len() * n)))
}

pub(crate) fn parse_grid_repeat(input: &mut CssParser) -> Result<Vec<GridTrackSpec>, ()> {
    if let Ok(auto) = input.try_parse(parse_nested_auto_repeat_keyword) {
        return Ok(vec![auto]);
    }
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
/// of `<track-size>` entries (`parse_grid_track_size`, `minmax()`
/// included) and `repeat(<integer>, <track-list>)` (`parse_grid_
/// repeat`) expanded eagerly into that same flat list — no named
/// lines interleaved into the track list itself (`grid-template-areas`
/// is its own separate property instead, see `parse_grid_template_
/// areas`). The real `none` keyword isn't matched explicitly — same
/// "unparseable leaves this property unset" convention as every other
/// not-yet-supported value in this file; an absent declaration and an
/// explicit `none` already mean the same thing to `measure_grid` (no
/// explicit tracks on this axis).
pub(crate) fn parse_grid_template(input: &mut CssParser) -> Result<Vec<GridTrackSpec>, ()> {
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
    if tracks.is_empty() || tracks.iter().filter(|t| matches!(t, GridTrackSpec::AutoRepeat { .. })).count() > 1 {
        Err(())
    } else {
        Ok(tracks)
    }
}

/// Rows can't use `repeat(auto-fill|auto-fit, …)` (no definite height).
pub(crate) fn reject_auto_repeat(tracks: Vec<GridTrackSpec>) -> Result<Vec<GridTrackSpec>, ()> {
    if tracks.iter().any(|t| matches!(t, GridTrackSpec::AutoRepeat { .. })) {
        Err(())
    } else {
        Ok(tracks)
    }
}

/// `grid-template` shorthand: `none`, `<rows> / <columns>`, or area
/// strings each optionally followed by a row `<track-size>`
/// (`"a a" 40px "b c" / 1fr 2fr`). Returns (rows, columns, areas); an
/// empty vec means that part was not declared. Area rows with no size
/// get `auto`. Named lines (`[x]`) are not supported (fails to parse).
#[allow(clippy::type_complexity)]
pub(crate) fn parse_grid_template_shorthand(
    input: &mut CssParser,
) -> Result<(Vec<GridTrackSpec>, Vec<GridTrackSpec>, Vec<&'static str>), ()> {
    let state = input.state();
    if let Ok(id) = input.expect_ident() {
        if id.eq_ignore_ascii_case("none") && input.is_exhausted() {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }
    }
    input.reset(&state);
    let mut rows = Vec::new();
    let mut areas: Vec<&'static str> = Vec::new();
    let mut cols = Vec::new();
    while !input.is_exhausted() {
        let state = input.state();
        if input.try_parse(|i| i.expect_delim('/')).is_ok() {
            cols = parse_grid_template(input)?;
            break;
        }
        input.reset(&state);
        if let Ok(s) = input.try_parse(|i| i.expect_string().map(|s| s.as_ref().to_string())) {
            areas.push(Box::leak(s.into_boxed_str()));
            let size = input
                .try_parse(parse_grid_track_size)
                .unwrap_or(GridTrackSpec::Auto);
            rows.push(size);
            continue;
        }
        if !areas.is_empty() {
            return Err(());
        }
        let state = input.state();
        if input.expect_function_matching("repeat").is_ok() {
            rows.extend(parse_grid_repeat(input)?);
            continue;
        }
        input.reset(&state);
        rows.push(parse_grid_track_size(input)?);
    }
    if rows.is_empty() && cols.is_empty() {
        return Err(());
    }
    Ok((reject_auto_repeat(rows)?, cols, areas))
}

/// `grid-template-areas`'s own grammar: one or more `<string>` tokens in
/// a row, each one a template row (`"a a" "b c"`), split into per-cell
/// name tokens by `measure_grid`'s own `grid_area_rect` at measure time,
/// not here — storing the raw per-row strings is enough to leak once
/// (`DeclParser`'s own branch, the same `Box::leak`-per-distinct-value
/// pattern `font-family`'s fallback list and `grid-template-columns`'s
/// own track list already use) and re-split cheaply on each use, same
/// as re-splitting a `class` attribute on every selector match already
/// does elsewhere in this file. Real CSS requires every row to name the
/// same number of columns and a name's own occurrences to form a single
/// rectangle — neither is validated at parse time here; a malformed
/// template is instead caught (and the item it would have placed simply
/// falls through to ordinary auto-placement) in `grid_area_rect`, not
/// rejected outright the way most other unsupported values in this file
/// are — see its own docs for why that's the more useful failure mode
/// for this specific property. The literal `.` token (an empty/null
/// cell, real CSS's own escape hatch for "no name here") is preserved
/// as an ordinary string, not special-cased — `grid_area_rect` simply
/// never finds a real item whose `grid-area` is literally `.`.
pub(crate) fn parse_grid_template_areas(input: &mut CssParser) -> Result<Vec<&'static str>, ()> {
    let mut rows = Vec::new();
    while let Ok(s) = input.try_parse(|input| input.expect_string().map(|s| s.as_ref().to_string())) {
        rows.push(Box::leak(s.into_boxed_str()) as &'static str);
    }
    if rows.is_empty() || !input.is_exhausted() {
        Err(())
    } else {
        Ok(rows)
    }
}

/// `grid-area`'s own grammar, scoped to a single bare `<custom-ident>`
/// naming one of `grid-template-areas`'s own areas (`grid-area:
/// columnStart`) — the one form real Wikipedia CSS actually uses
/// (confirmed by grep, see `scratchy/TODO.txt`'s own `display: grid`
/// entry). The real property's fuller `<grid-line> / <grid-line> /
/// <grid-line> / <grid-line>` numeric-placement grammar (and the bare
/// `grid-row`/`grid-column`/their `-start`/`-end` longhands) still
/// aren't parsed at all — same pre-existing "no explicit grid item
/// placement" scope cut `TODO.txt` already names, narrowed but not
/// closed by this.
pub(crate) fn parse_grid_area_name(input: &mut CssParser) -> Result<&'static str, ()> {
    let ident = input.expect_ident().map_err(|_| ())?.as_ref().to_string();
    if !input.is_exhausted() || ident.eq_ignore_ascii_case("auto") || ident.eq_ignore_ascii_case("span") {
        return Err(());
    }
    Ok(Box::leak(ident.into_boxed_str()))
}

/// One `<grid-line>` of numeric placement: `auto`, a non-zero line
/// number (negative counts from the end), or `span N`. Named lines are
/// not supported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GridLine {
    Auto,
    Line(i32),
    Span(usize),
}

pub(crate) fn parse_grid_line(input: &mut CssParser) -> Result<GridLine, ()> {
    let tok = input.next().map_err(|_| ())?.clone();
    match tok {
        Token::Ident(id) if id.eq_ignore_ascii_case("auto") => Ok(GridLine::Auto),
        Token::Ident(id) if id.eq_ignore_ascii_case("span") => {
            match input.try_parse(|i| match i.next().map_err(|_| ())? {
                Token::Number { int_value: Some(n), .. } if *n > 0 => Ok(*n as usize),
                _ => Err(()),
            }) {
                Ok(n) => Ok(GridLine::Span(n)),
                Err(()) => Ok(GridLine::Span(1)),
            }
        }
        Token::Number { int_value: Some(n), .. } if n != 0 => {
            // `<integer> span` is not valid; a bare integer is a line.
            Ok(GridLine::Line(n))
        }
        _ => Err(()),
    }
}

/// `<grid-line> [ / <grid-line> ]?` (`grid-row`/`grid-column`).
pub(crate) fn parse_grid_line_pair(input: &mut CssParser) -> Result<(GridLine, GridLine), ()> {
    let a = parse_grid_line(input)?;
    let b = if input.try_parse(|i| i.expect_delim('/')).is_ok() { parse_grid_line(input)? } else { GridLine::Auto };
    if !input.is_exhausted() {
        return Err(());
    }
    Ok((a, b))
}

/// `grid-area` numeric form: up to four `<grid-line>`s separated by `/`
/// in row-start, column-start, row-end, column-end order; omitted ones
/// are `auto`. Returns `[row_start, column_start, row_end, column_end]`.
pub(crate) fn parse_grid_area_lines(input: &mut CssParser) -> Result<[GridLine; 4], ()> {
    let mut out = [GridLine::Auto; 4];
    out[0] = parse_grid_line(input)?;
    for slot in out.iter_mut().skip(1) {
        if input.try_parse(|i| i.expect_delim('/')).is_err() {
            break;
        }
        *slot = parse_grid_line(input)?;
    }
    if !input.is_exhausted() {
        return Err(());
    }
    Ok(out)
}

/// One axis' placement after resolving `GridLine`s against the number of
/// explicit tracks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AxisPlace {
    Fixed(usize, usize),
    Auto(usize),
}

pub(crate) fn resolve_grid_axis(start: Option<GridLine>, end: Option<GridLine>, explicit: usize) -> AxisPlace {
    let line = |n: i32| -> usize {
        if n > 0 { (n - 1) as usize } else { (explicit as i32 + 1 + n).max(0) as usize }
    };
    use GridLine::*;
    match (start.unwrap_or(Auto), end.unwrap_or(Auto)) {
        (Line(a), Line(b)) => {
            let (a, b) = (line(a), line(b));
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            AxisPlace::Fixed(a, if a == b { a + 1 } else { b })
        }
        (Line(a), Span(n)) => AxisPlace::Fixed(line(a), line(a) + n),
        (Span(n), Line(b)) => AxisPlace::Fixed(line(b).saturating_sub(n), line(b).max(n)),
        (Line(a), Auto) => AxisPlace::Fixed(line(a), line(a) + 1),
        (Auto, Line(b)) => AxisPlace::Fixed(line(b).saturating_sub(1), line(b).max(1)),
        (Span(n), Span(_)) | (Span(n), Auto) | (Auto, Span(n)) => AxisPlace::Auto(n),
        (Auto, Auto) => AxisPlace::Auto(1),
    }
}

/// `SCRATCHY_CSS_DEBUG=1 scratchy ...` prints every CSS declaration
/// this file's own cascade couldn't act on — same env-var-gated,
/// zero-cost-when-off shape `SCRATCHY_TIMING`/`snag`'s own `SNAG_
/// TIMING`/`wire`'s own `WIRE_TIMING` already established (see
/// `timing_enabled`'s own docs), chosen over a CLI flag for exactly
/// that reason: it's the existing convention for an opt-in diagnostic
/// in this project, not a new one invented for this feature.
///
/// The user's own request (see `TODO.txt`), motivated by this exact
/// kind of gap: diagnosing why a real Wikipedia page's page-shell
/// layout was broken (the `grid-template-areas`/`minmax()` item)
/// meant manually `curl`-ing Wikipedia's real CSS out-of-band and
/// grepping it by hand for the properties scratchy was silently
/// dropping — scratchy itself gave no signal that anything had failed
/// to parse at all. `parse_value`'s own before/after `Declarations`
/// snapshot (see its own call site) reports at the raw-declaration
/// level — `property: value`, exactly as the stylesheet wrote it —
/// not resolved to "which `TODO.txt` item this maps to," the
/// cheaper, more honest answer to that item's own open question: this
/// file's own cascade can know a declaration had no effect without
/// knowing *why* in any structured way (an unknown property name, a
/// known one whose particular value shape isn't supported, or a
/// value shape that's supported but happens to assign the exact same
/// value a declaration already had — the one rare false-negative this
/// approach accepts, same spirit as every other documented
/// simplification in this file).
///
/// Not deduplicated by property/value — a page with a thousand
/// elements all declaring the same unsupported `justify-items: center`
/// prints a thousand identical lines. Deliberately not addressed here:
/// summarizing/counting would need to buffer state across the whole
/// parse rather than report inline per declaration, real but separate
/// scope past "see that something's being dropped at all," which is
/// this feature's one actual job.
pub(crate) fn css_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SCRATCHY_CSS_DEBUG").is_some())
        || CSS_DEBUG_CAPTURE.with(|c| c.borrow().is_some())
}

thread_local! {
    /// Test hook: when `Some`, `css_debug_report` collects lines here
    /// (and `css_debug_enabled` reports true) instead of using stderr.
    pub(crate) static CSS_DEBUG_CAPTURE: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// Single sink for every `SCRATCHY_CSS_DEBUG` line.
pub(crate) fn css_debug_report(line: String) {
    let captured = CSS_DEBUG_CAPTURE.with(|c| {
        if let Some(v) = c.borrow_mut().as_mut() {
            v.push(line.clone());
            true
        } else {
            false
        }
    });
    if !captured {
        eprintln!("{line}");
    }
}

/// Reports a rule the stylesheet parser rejected: a dropped at-rule
/// (`@name`) or a qualified rule whose selector failed to parse.
pub(crate) fn css_debug_report_dropped_rule(slice: &str) {
    if !css_debug_enabled() {
        return;
    }
    let slice = slice.trim();
    if let Some(rest) = slice.strip_prefix('@') {
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        css_debug_report(format!("scratchy: css: dropped @{name}"));
    } else {
        let sel = slice.split('{').next().unwrap_or(slice).trim();
        css_debug_report(format!("scratchy: css: dropped selector `{sel}`"));
    }
}

pub(crate) struct DeclParser<'a> {
    pub(crate) decls: &'a mut Declarations,
    /// Receives declarations that end in `!important`.
    pub(crate) important: &'a mut Declarations,
}

/// Strips a trailing `!important` (any case, optional whitespace after
/// the `!`) from raw declaration text, if present.
pub(crate) fn strip_important(raw: &str) -> Option<&str> {
    let t = raw.trim_end();
    let len = t.len();
    if len < 9 || !t.is_char_boundary(len - 9) || !t[len - 9..].eq_ignore_ascii_case("important") {
        return None;
    }
    let before = t[..len - 9].trim_end();
    before.strip_suffix('!').map(|v| v.trim_end())
}

impl Declarations {
    /// Records a CSS-wide keyword for `key`, dropping the typed value.
    pub(crate) fn set_wide(&mut self, key: &'static str, kw: CssWide) {
        self.clear_wide(key);
        self.clear_typed(key);
        self.wide.push((key, kw));
    }

    /// Whether this declaration block holds a typed (non-keyword) value
    /// for the canonical CSS-wide key `key`.
    pub(crate) fn has_typed(&self, key: &str) -> bool {
        let mut probe = self.clone();
        probe.clear_typed(key);
        probe != *self
    }

    /// Drops the typed value for the canonical CSS-wide key `key`.
    pub(crate) fn clear_typed(&mut self, key: &str) {
        macro_rules! clear {
            ($($k:literal => $f:ident),* $(,)?) => {
                match key {
                    $($k => self.$f = None,)*
                    _ => {
                        if let Some((i, kind)) = border_key(key) {
                            match kind {
                                'w' => self.border_width[i] = None,
                                's' => self.border_style[i] = None,
                                _ => self.border_color[i] = None,
                            }
                        }
                    }
                }
            };
        }
        clear! {
            "color" => color, "background-color" => background_color,
            "background-image" => background_image, "display" => display,
            "background-position" => background_position,
            "background-size" => background_size,
            "background-repeat" => background_repeat,
            "object-fit" => object_fit,
            "object-position" => object_position,
            "font-size" => font_size, "line-height" => line_height,
            "vertical-align" => vertical_align, "font-family" => font_family,
            "font-weight" => font_weight, "text-align" => text_align,
            "white-space" => white_space, "cursor" => cursor,
            "visibility" => visibility, "position" => position, "float" => float,
            "clear" => clear, "top" => top, "right" => right, "bottom" => bottom,
            "left" => left, "z-index" => z_index, "opacity" => opacity, "width" => width,
            "height" => height, "min-width" => min_width, "max-width" => max_width,
            "min-height" => min_height, "max-height" => max_height,
            "box-sizing" => box_sizing, "margin-top" => margin_top,
            "margin-right" => margin_right, "margin-bottom" => margin_bottom,
            "margin-left" => margin_left, "padding-top" => padding_top,
            "padding-right" => padding_right, "padding-bottom" => padding_bottom,
            "padding-left" => padding_left, "outline-width" => outline_width,
            "outline-color" => outline_color, "box-shadow" => box_shadow,
            "border-radius" => border_radius, "overflow" => overflow,
            "overflow-y" => overflow_y, "text-overflow" => text_overflow,
            "flex-direction" => flex_direction, "flex-wrap" => flex_wrap,
            "justify-content" => justify_content, "align-items" => align_items,
            "align-content" => align_content,
            "align-self" => align_self, "flex-grow" => flex_grow,
            "flex-shrink" => flex_shrink, "flex-basis" => flex_basis, "order" => order,
            "row-gap" => row_gap, "column-gap" => column_gap,
            "grid-template-columns" => grid_template_columns,
            "grid-template-rows" => grid_template_rows,
            "grid-template-areas" => grid_template_areas, "grid-area" => grid_area,
            "font-style" => font_style, "list-style-type" => list_style_type,
            "word-spacing" => word_spacing,
            "text-decoration" => text_decoration,
            "text-decoration-line" => text_decoration_line,
            "text-decoration-style" => text_decoration_style,
            "text-decoration-color" => text_decoration_color,
        }
    }

    pub(crate) fn clear_wide(&mut self, key: &str) {
        self.wide.retain(|(k, _)| *k != key);
    }
}

impl DeclParser<'_> {
    /// The per-property dispatch, split out of `parse_value` so the
    /// debug path can re-run it against a scratch `Declarations`.
    pub(crate) fn apply<'i>(&mut self, name: &str, input: &mut CssParser<'i>) {
        let name = unprefixed_property(name);
        if !name.starts_with("--") {
            let start = input.state();
            while input.next().is_ok() {}
            let raw = input.slice_from(start.position()).trim();
            if raw.contains("var(") {
                self.decls.var_decls.push((name.to_string(), raw.to_string()));
                return;
            }
            input.reset(&start);
        }
        if !name.starts_with("--") {
            if let Some(keys) = wide_keys(name) {
                // A CSS-wide keyword (optionally followed by `!important`).
                let kw = input.try_parse(|i| {
                    let kw = {
                        let id = i.expect_ident().map_err(|_| ())?;
                        CssWide::from_ident(id).ok_or(())?
                    };
                    match i.next() {
                        Err(_) => Ok(kw),
                        Ok(Token::Delim('!')) => Ok(kw),
                        _ => Err(()),
                    }
                });
                if let Ok(kw) = kw {
                    // Swallow the remaining tokens (`important`).
                    while input.next().is_ok() {}
                    for key in keys {
                        self.decls.set_wide(key, kw);
                    }
                    return;
                }
                for key in keys {
                    self.decls.clear_wide(key);
                }
            }
        }
        if name.starts_with("--") {
            // Custom property: grammar is "anything", so there's no typed
            // value to parse — just capture the raw source text of the
            // whole value (consuming every token so `parse_entirely`,
            // which wraps this call, doesn't see it as a parse error).
            let start = input.state();
            while input.next().is_ok() {}
            let raw = input.slice_from(start.position()).trim();
            let raw = strip_important(raw).unwrap_or(raw).to_string();
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
            if input.try_parse(|input| input.expect_ident_matching("none")).is_ok() {
                self.decls.background_image = Some(BackgroundImageDecl::None);
            } else if let Ok(url) = input.expect_url() {
                self.decls.background_image = Some(BackgroundImageDecl::Url(url.to_string()));
            }
        } else if name.eq_ignore_ascii_case("mask-image") || name.eq_ignore_ascii_case("-webkit-mask-image") {
            if input.try_parse(|input| input.expect_ident_matching("none")).is_ok() {
                self.decls.mask_image = Some(BackgroundImageDecl::None);
            } else if let Ok(url) = input.expect_url() {
                self.decls.mask_image = Some(BackgroundImageDecl::Url(url.to_string()));
            }
        } else if name.eq_ignore_ascii_case("mask-position") || name.eq_ignore_ascii_case("-webkit-mask-position") {
            if let Ok(v) = parse_bg_position(input) {
                self.decls.mask_position = Some(v);
            }
        } else if name.eq_ignore_ascii_case("mask-size") || name.eq_ignore_ascii_case("-webkit-mask-size") {
            if let Ok(v) = parse_bg_size(input) {
                self.decls.mask_size = Some(v);
            }
        } else if name.eq_ignore_ascii_case("mask-repeat") || name.eq_ignore_ascii_case("-webkit-mask-repeat") {
            if let Ok(v) = parse_bg_repeat(input) {
                self.decls.mask_repeat = Some(v);
            }
        } else if name.eq_ignore_ascii_case("mask") || name.eq_ignore_ascii_case("-webkit-mask") {
            if let Ok(bg) = parse_background_shorthand(input) {
                self.decls.mask_image = Some(match bg.image {
                    Some(url) => BackgroundImageDecl::Url(url),
                    None => BackgroundImageDecl::None,
                });
                self.decls.mask_position = Some(bg.position.unwrap_or_default());
                self.decls.mask_size = Some(bg.size.unwrap_or_default());
                self.decls.mask_repeat = Some(bg.repeat.unwrap_or_default());
            }
        } else if name.eq_ignore_ascii_case("background-position") {
            if let Ok(v) = parse_bg_position(input) {
                self.decls.background_position = Some(v);
            }
        } else if name.eq_ignore_ascii_case("background-size") {
            if let Ok(v) = parse_bg_size(input) {
                self.decls.background_size = Some(v);
            }
        } else if name.eq_ignore_ascii_case("object-fit") {
            if let Ok(id) = input.expect_ident() {
                self.decls.object_fit = ObjectFit::from_keyword(&id.to_ascii_lowercase());
            }
        } else if name.eq_ignore_ascii_case("object-position") {
            if let Ok(v) = parse_bg_position(input) {
                self.decls.object_position = Some(v);
            }
        } else if name.eq_ignore_ascii_case("background-repeat") {
            if let Ok(v) = parse_bg_repeat(input) {
                self.decls.background_repeat = Some(v);
            }
        } else if name.eq_ignore_ascii_case("background") {
            if let Ok(bg) = parse_background_shorthand(input) {
                if let Some(c) = bg.color {
                    self.decls.background_color = Some(c);
                }
                if let Some(url) = bg.image {
                    self.decls.background_image = Some(BackgroundImageDecl::Url(url));
                }
                // The shorthand resets the three longhands to their
                // initial values when it omits them.
                self.decls.background_position = Some(bg.position.unwrap_or_default());
                self.decls.background_size = Some(bg.size.unwrap_or_default());
                self.decls.background_repeat = Some(bg.repeat.unwrap_or_default());
            }
        } else if parse_border_property(self.decls, name, input) {
        } else if name.eq_ignore_ascii_case("display") {
            if let Ok(ident) = input.expect_ident() {
                // `list-item` is a block that also generates a marker (Q20c).
                let is_list_item = ident.eq_ignore_ascii_case("list-item");
                let is_flow_root = ident.eq_ignore_ascii_case("flow-root");
                let is_caption = ident.eq_ignore_ascii_case("table-caption");
                self.decls.display = if ident.eq_ignore_ascii_case("block") || ident.eq_ignore_ascii_case("list-item") {
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
                    // A block box plus the `table_caption` flag (Q34c):
                    // `measure_table` places it by `caption-side`, and it
                    // adds only its min-content width to a table's
                    // shrink-to-fit size.
                    Some(Display::Block)
                } else if ident.eq_ignore_ascii_case("flex") || ident.eq_ignore_ascii_case("-webkit-flex") {
                    Some(Display::Flex)
                } else if ident.eq_ignore_ascii_case("grid") {
                    Some(Display::Grid)
                } else if ident.eq_ignore_ascii_case("flow-root") {
                    // A block box plus the `flow_root` flag (Q34b), which
                    // makes it establish its own BFC (`establishes_bfc`).
                    Some(Display::Block)
                } else if ident.eq_ignore_ascii_case("inline-block") {
                    Some(Display::InlineBlock)
                } else if ident.eq_ignore_ascii_case("inline-flex")
                    || ident.eq_ignore_ascii_case("-webkit-inline-flex")
                {
                    Some(Display::InlineFlex)
                } else {
                    None
                };
                if self.decls.display.is_some() {
                    self.decls.list_item = Some(is_list_item);
                    self.decls.flow_root = Some(is_flow_root);
                    self.decls.table_caption = Some(is_caption);
                }
            }
        } else if name.eq_ignore_ascii_case("caption-side") {
            if let Ok(ident) = input.expect_ident() {
                if ident.eq_ignore_ascii_case("bottom") {
                    self.decls.caption_bottom = Some(true);
                } else if ident.eq_ignore_ascii_case("top") {
                    self.decls.caption_bottom = Some(false);
                }
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
        } else if name.eq_ignore_ascii_case("order") {
            if let Ok(Token::Number { int_value: Some(v), .. }) = input.next().map_err(|_| ()) {
                self.decls.order = Some(*v);
            }
        } else if name.eq_ignore_ascii_case("z-index") {
            if let Ok(z) = parse_z_index(input) {
                self.decls.z_index = Some(z);
            }
        } else if name.eq_ignore_ascii_case("content") {
            if let Ok(c) = parse_content(input) {
                self.decls.content = Some(c);
            }
        } else if name.eq_ignore_ascii_case("counter-reset") {
            if let Ok(c) = parse_counter_ops(input, 0) {
                self.decls.counter_reset = Some(c);
            }
        } else if name.eq_ignore_ascii_case("counter-increment") {
            if let Ok(c) = parse_counter_ops(input, 1) {
                self.decls.counter_increment = Some(c);
            }
        } else if name.eq_ignore_ascii_case("counter-set") {
            if let Ok(c) = parse_counter_ops(input, 0) {
                self.decls.counter_set = Some(c);
            }
        } else if name.eq_ignore_ascii_case("quotes") {
            if let Ok(q) = parse_quotes(input) {
                self.decls.quotes = Some(q);
            }
        } else if name.eq_ignore_ascii_case("opacity") {
            if let Ok(o) = parse_opacity(input) {
                self.decls.opacity = Some(o);
            }
        } else if name.eq_ignore_ascii_case("width") {
            set_width_length(input, &mut self.decls.width);
        } else if name.eq_ignore_ascii_case("height") {
            set_length(input, &mut self.decls.height);
        } else if name.eq_ignore_ascii_case("min-width") {
            set_width_length(input, &mut self.decls.min_width);
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
            if input.try_parse(|input| input.expect_ident_matching("none")).is_ok() {
                self.decls.box_shadow = Some(ShadowDecl::None);
            } else if let Ok(v) = parse_box_shadow(input) {
                self.decls.box_shadow = Some(ShadowDecl::Shadow(v));
            }
        } else if name.eq_ignore_ascii_case("border-radius") {
            set_length(input, &mut self.decls.border_radius);
        } else if name.eq_ignore_ascii_case("overflow")
            || name.eq_ignore_ascii_case("overflow-x")
            || name.eq_ignore_ascii_case("overflow-y")
        {
            if name.eq_ignore_ascii_case("overflow") {
                // One value sets both axes; two values are `x y`.
                if let Some(x) = parse_overflow_keyword(input) {
                    let y = if input.is_exhausted() { Some(x) } else { parse_overflow_keyword(input) };
                    if let Some(y) = y {
                        self.decls.overflow = Some(x);
                        self.decls.overflow_y = Some(y);
                    }
                }
            } else if let Some(v) = parse_overflow_keyword(input) {
                if name.eq_ignore_ascii_case("overflow-x") {
                    self.decls.overflow = Some(v);
                } else {
                    self.decls.overflow_y = Some(v);
                }
            }
        } else if name.eq_ignore_ascii_case("text-overflow") {
            if let Ok(ident) = input.expect_ident() {
                if ident.eq_ignore_ascii_case("ellipsis") {
                    self.decls.text_overflow = Some(TextOverflow::Ellipsis);
                } else if ident.eq_ignore_ascii_case("clip") {
                    self.decls.text_overflow = Some(TextOverflow::Clip);
                }
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
        } else if name.eq_ignore_ascii_case("align-content") {
            if let Ok(ident) = input.expect_ident() {
                let v = match &*ident.to_ascii_lowercase() {
                    "stretch" | "normal" => Some(AlignContent::Stretch),
                    "flex-start" | "start" => Some(AlignContent::FlexStart),
                    "flex-end" | "end" => Some(AlignContent::FlexEnd),
                    "center" => Some(AlignContent::Center),
                    "space-between" => Some(AlignContent::SpaceBetween),
                    "space-around" => Some(AlignContent::SpaceAround),
                    "space-evenly" => Some(AlignContent::SpaceEvenly),
                    _ => None,
                };
                if v.is_some() {
                    self.decls.align_content = v;
                }
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
            if let Ok(tracks) = parse_grid_template(input).and_then(reject_auto_repeat) {
                self.decls.grid_template_rows = Some(Box::leak(tracks.into_boxed_slice()));
            }
        } else if name.eq_ignore_ascii_case("grid-template") {
            if let Ok((rows, cols, areas)) = parse_grid_template_shorthand(input) {
                self.decls.grid_template_rows = Some(Box::leak(rows.into_boxed_slice()));
                self.decls.grid_template_columns = Some(Box::leak(cols.into_boxed_slice()));
                self.decls.grid_template_areas = Some(Box::leak(areas.into_boxed_slice()));
            }
        } else if name.eq_ignore_ascii_case("grid-template-areas") {
            if let Ok(rows) = parse_grid_template_areas(input) {
                self.decls.grid_template_areas = Some(Box::leak(rows.into_boxed_slice()));
            }
        } else if name.eq_ignore_ascii_case("grid-row") || name.eq_ignore_ascii_case("grid-column") {
            if let Ok((a, b)) = parse_grid_line_pair(input) {
                if name.eq_ignore_ascii_case("grid-row") {
                    self.decls.grid_row_start = Some(a);
                    self.decls.grid_row_end = Some(b);
                } else {
                    self.decls.grid_column_start = Some(a);
                    self.decls.grid_column_end = Some(b);
                }
            }
        } else if let Some(which) = ["grid-row-start", "grid-row-end", "grid-column-start", "grid-column-end"]
            .iter()
            .position(|n| name.eq_ignore_ascii_case(n))
        {
            if let Ok(l) = parse_grid_line(input).and_then(|l| if input.is_exhausted() { Ok(l) } else { Err(()) }) {
                match which {
                    0 => self.decls.grid_row_start = Some(l),
                    1 => self.decls.grid_row_end = Some(l),
                    2 => self.decls.grid_column_start = Some(l),
                    _ => self.decls.grid_column_end = Some(l),
                }
            }
        } else if name.eq_ignore_ascii_case("grid-area") {
            if let Ok(name) = input.try_parse(parse_grid_area_name) {
                self.decls.grid_area = Some(name);
            } else if let Ok(l) = parse_grid_area_lines(input) {
                self.decls.grid_row_start = Some(l[0]);
                self.decls.grid_column_start = Some(l[1]);
                self.decls.grid_row_end = Some(l[2]);
                self.decls.grid_column_end = Some(l[3]);
            }
        } else if name.eq_ignore_ascii_case("font-size") {
            let state = input.state();
            let keyword = input.next().ok().and_then(|t| match t {
                Token::Ident(ident) if ident.eq_ignore_ascii_case("larger") => Some(Length::Larger),
                Token::Ident(ident) if ident.eq_ignore_ascii_case("smaller") => Some(Length::Smaller),
                Token::Ident(ident) => font_size_keyword(&ident.to_ascii_lowercase()).map(Length::Px),
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
        } else if name.eq_ignore_ascii_case("font") {
            if let Ok((style, weight, size, lh, family)) = parse_font_shorthand(input) {
                self.decls.font_style = Some(style);
                self.decls.font_weight = Some(weight);
                self.decls.font_size = Some(size);
                self.decls.line_height = Some(lh);
                self.decls.font_family = Some(family);
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
                } else if ident.eq_ignore_ascii_case("nowrap") {
                    Some(WhiteSpace::Nowrap)
                } else if ident.eq_ignore_ascii_case("pre-wrap") || ident.eq_ignore_ascii_case("break-spaces") {
                    Some(WhiteSpace::PreWrap)
                } else if ident.eq_ignore_ascii_case("pre-line") {
                    Some(WhiteSpace::PreLine)
                } else {
                    None
                };
            }
        } else if name.eq_ignore_ascii_case("word-break") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.word_break = match ident.to_ascii_lowercase().as_str() {
                    "normal" | "keep-all" => Some(WordBreak::Normal),
                    "break-all" => Some(WordBreak::BreakAll),
                    "break-word" => Some(WordBreak::BreakWord),
                    _ => None,
                };
            }
        } else if name.eq_ignore_ascii_case("overflow-wrap") || name.eq_ignore_ascii_case("word-wrap") {
            if let Ok(ident) = input.expect_ident() {
                match ident.to_ascii_lowercase().as_str() {
                    "normal" => self.decls.word_break = Some(WordBreak::Normal),
                    "break-word" | "anywhere" => self.decls.word_break = Some(WordBreak::BreakWord),
                    _ => {}
                }
            }
        } else if name.eq_ignore_ascii_case("text-transform") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.text_transform = match ident.to_ascii_lowercase().as_str() {
                    "none" => Some(TextTransform::None),
                    "uppercase" => Some(TextTransform::Uppercase),
                    "lowercase" => Some(TextTransform::Lowercase),
                    "capitalize" => Some(TextTransform::Capitalize),
                    _ => None,
                };
            }
        } else if name.eq_ignore_ascii_case("list-style-type") {
            if let Ok(ident) = input.expect_ident() {
                self.decls.list_style_type = ListStyleType::from_keyword(ident);
            }
        } else if name.eq_ignore_ascii_case("list-style") {
            if let Ok(ty) = parse_list_style_shorthand(input) {
                self.decls.list_style_type = Some(ty);
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
                } else if ident.eq_ignore_ascii_case("start") {
                    // Logical, writing-mode-relative keywords — this
                    // engine has no notion of a writing mode other than
                    // left-to-right (see `TextAlign`'s own docs, and
                    // `margin-inline-start`/`-end`'s own already-landed
                    // "always resolve against left/right" precedent), so
                    // `start`/`end` always resolve to `left`/`right`.
                    Some(TextAlign::Left)
                } else if ident.eq_ignore_ascii_case("end") {
                    Some(TextAlign::Right)
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
                } else if ident.eq_ignore_ascii_case("top") {
                    Some(VerticalAlign::Top)
                } else if ident.eq_ignore_ascii_case("middle") {
                    Some(VerticalAlign::Middle)
                } else if ident.eq_ignore_ascii_case("bottom") {
                    Some(VerticalAlign::Bottom)
                } else if ident.eq_ignore_ascii_case("text-top") {
                    Some(VerticalAlign::TextTop)
                } else if ident.eq_ignore_ascii_case("text-bottom") {
                    Some(VerticalAlign::TextBottom)
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
    }
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
        // `SCRATCHY_CSS_DEBUG`'s own before/after snapshot (see `css_
        // debug_enabled`'s own docs) — captured *before* any of the
        // branches below touch `input`, then rewound, so the branch that
        // actually matches `name` (if any) still sees the exact same
        // untouched `input` it always has; this adds a read, never a
        // behavior change. Skipped entirely when the flag is off, so
        // the already-acknowledged "reparse the whole stylesheet every
        // redraw" cost (see `NOTES.md`) never grows for the common case.
        let debug = css_debug_enabled();
        let (raw_value, before, value_start) = if debug {
            let value_start = input.state();
            while input.next().is_ok() {}
            let raw = input.slice_from(value_start.position()).trim().to_string();
            input.reset(&value_start);
            (raw, Some(self.decls.clone()), Some(value_start))
        } else {
            (String::new(), None, None)
        };

        // Detect a trailing `!important` (it is part of this value's
        // tokens, not delimited off by the caller) and route the
        // declaration into the important `Declarations` by swapping the
        // two for the duration of `apply`. Custom properties always stay
        // in the normal set (`var()` substitution has no tiers).
        let important = !name.starts_with("--") && {
            let start = input.state();
            while input.next().is_ok() {}
            let raw = input.slice_from(start.position());
            let found = strip_important(raw).is_some();
            input.reset(&start);
            found
        };
        if important {
            std::mem::swap(self.decls, self.important);
            self.apply(&name, input);
            std::mem::swap(self.decls, self.important);
            return Ok(());
        }
        self.apply(&name, input);
        if let (Some(before), Some(value_start)) = (before, value_start) {
            if *self.decls == before {
                // Unchanged: either no branch accepted the value, or a
                // branch accepted it and it equalled what the rule
                // already had (Wikipedia writes every colour twice).
                // Re-apply onto a blank `Declarations` to tell them
                // apart: a branch that returned `Ok` sets something
                // there, since every field defaults to unset.
                input.reset(&value_start);
                let mut scratch = Declarations::default();
                DeclParser { decls: &mut scratch, important: &mut Declarations::default() }.apply(&name, input);
                if scratch == Declarations::default() {
                    css_debug_report(format!("scratchy: css: ignored `{name}: {raw_value}`"));
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

#[cfg(test)]
pub(crate) fn parse_declarations(source: &str) -> Declarations {
    parse_declarations_split(source).0
}

/// Parses a declaration list into (normal, `!important`) sets.
pub(crate) fn parse_declarations_split(source: &str) -> (Declarations, Declarations) {
    let mut decls = Declarations::default();
    let mut important = Declarations::default();
    {
        let mut input = CssParser::new(source);
        let mut decl_parser = DeclParser { decls: &mut decls, important: &mut important };
        let mut iter = RuleBodyParser::new(&mut input, &mut decl_parser);
        while iter.next().is_some() {}
    }
    (decls, important)
}

/// Prelude of a top-level at-rule we understand.
pub(crate) enum AtPrelude {
    Media(Vec<MediaQuery>),
    /// `@supports`: the condition's truth value, evaluated up front.
    Supports(bool),
}

/// `@supports` condition: `not <in-parens>` or `<in-parens> [and|or
/// <in-parens>]*` (mixing `and`/`or` unparenthesised is invalid, so false).
pub(crate) fn parse_supports_condition<'i>(input: &mut CssParser<'i>) -> Result<bool, ParseError<CssError>> {
    let state = input.state();
    let first = input.expect_ident_matching("not").is_ok();
    if !first {
        input.reset(&state);
    }
    if first {
        return Ok(!parse_supports_in_parens(input)?);
    }
    let mut value = parse_supports_in_parens(input)?;
    let mut op: Option<bool> = None; // true = and
    while !input.is_exhausted() {
        let is_and = {
            let t = input.expect_ident()?;
            if t.eq_ignore_ascii_case("and") {
                true
            } else if t.eq_ignore_ascii_case("or") {
                false
            } else {
                return Err(input.new_error_for_next_token());
            }
        };
        if *op.get_or_insert(is_and) != is_and {
            return Ok(false);
        }
        let rhs = parse_supports_in_parens(input)?;
        value = if is_and { value && rhs } else { value || rhs };
    }
    Ok(value)
}

pub(crate) fn parse_supports_in_parens<'i>(input: &mut CssParser<'i>) -> Result<bool, ParseError<CssError>> {
    let tok = input.next()?.clone();
    match tok {
        Token::ParenthesisBlock => input.parse_nested_block(|i| {
            let state = i.state();
            // `(prop: value)` first; otherwise a nested condition.
            if let Ok(name) = i.expect_ident().map(|n| n.clone()) {
                if i.expect_colon().is_ok() {
                    let mut scratch = Declarations::default();
                    DeclParser { decls: &mut scratch, important: &mut Declarations::default() }
                        .apply(&name, i);
                    while i.next().is_ok() {}
                    return Ok(scratch != Declarations::default());
                }
            }
            i.reset(&state);
            let v = parse_supports_condition(i)?;
            while i.next().is_ok() {}
            Ok(v)
        }),
        Token::Function(ref f) if f.eq_ignore_ascii_case("selector") => input.parse_nested_block(|i| {
            let ok = SelectorList::parse(&OurSelectorParser, i, ParseRelative::No).is_ok();
            while i.next().is_ok() {}
            Ok(ok)
        }),
        Token::Function(_) => {
            input.parse_nested_block(|i| {
                while i.next().is_ok() {}
                Ok::<(), ParseError<CssError>>(())
            })?;
            Ok(false)
        }
        _ => Err(input.new_error_for_next_token()),
    }
}

pub(crate) struct TopLevelParser;

impl<'i> AtRuleParser<'i> for TopLevelParser {
    // Only `@media`'s own query-list grammar is parsed here — every
    // other at-rule name falls through to `parse_prelude`'s `Err` arm
    // below, which is exactly "skip @import/@font-face/@keyframes/etc."
    // for this stage, same as the old blanket no-op (see TODO.txt's own
    // narrower item for exactly what's still deferred and why).
    type Prelude = AtPrelude;
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
            Ok(AtPrelude::Media(parse_media_query_list(input)))
        } else if name.eq_ignore_ascii_case("supports") {
            // Unparseable conditions are false (rules dropped), never an error.
            let holds = parse_supports_condition(input).unwrap_or(false);
            while input.next().is_ok() {}
            Ok(AtPrelude::Supports(holds))
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
        let (prelude, supported) = match prelude {
            AtPrelude::Media(q) => (q, true),
            AtPrelude::Supports(b) => (Vec::new(), b),
        };
        if !supported {
            // Consume the block; its rules never apply.
            while input.next().is_ok() {}
            return Ok(Vec::new());
        }
        // The block's own content is an ordinary rule list (selectors +
        // declarations, same grammar as the top level) — recursing with
        // a fresh `TopLevelParser` over this already block-delimited
        // `input` reuses the exact same rule-list parsing rather than a
        // second, parallel implementation.
        let mut nested = TopLevelParser;
        let mut rules = Vec::new();
        let iter = StyleSheetParser::new(input, &mut nested);
        for result in iter {
            let inner_rules = match result {
                Ok(r) => r,
                Err((_, slice, _)) => {
                    css_debug_report_dropped_rule(slice);
                    continue;
                }
            };
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
        let mut important = Declarations::default();
        {
            let mut decl_parser = DeclParser { decls: &mut decls, important: &mut important };
            let mut iter = RuleBodyParser::new(input, &mut decl_parser);
            while iter.next().is_some() {}
        }
        Ok(vec![StyleRule {
            selectors: prelude,
            decls,
            important,
            media: Vec::new(),
        }])
    }
}

pub(crate) fn parse_stylesheet(css: &str) -> Vec<StyleRule> {
    let mut input = CssParser::new(css);
    let mut top = TopLevelParser;
    let mut rules = Vec::new();
    let mut stylesheet = StyleSheetParser::new(&mut input, &mut top);
    while let Some(result) = stylesheet.next() {
        match result {
            Ok(rule_list) => rules.extend(rule_list),
            Err((_, slice, _)) => css_debug_report_dropped_rule(slice),
        }
    }
    rules
}

/// A resolved custom-property table (Q13).
pub(crate) type CustomProps = std::collections::BTreeMap<String, String>;

thread_local! {
    pub(crate) static CUSTOM_PROPS_INTERNER: RefCell<HashMap<CustomProps, &'static CustomProps>> =
        RefCell::new(HashMap::new());
}

/// Leaks one copy of each distinct table so `ComputedStyle` can stay
/// `Copy` while carrying it; bounded by the number of distinct tables.
pub(crate) fn intern_custom_props(map: CustomProps) -> &'static CustomProps {
    CUSTOM_PROPS_INTERNER.with(|i| {
        let mut i = i.borrow_mut();
        if let Some(r) = i.get(&map) {
            return *r;
        }
        let leaked: &'static CustomProps = Box::leak(Box::new(map.clone()));
        i.insert(map, leaked);
        leaked
    })
}

/// Computes an element's custom properties: the parent's table overlaid
/// with this element's own `--name` declarations (highest specificity,
/// later rule on ties, inline last), each own value `var()`-substituted
/// against the combined table. A cycle makes the property invalid
/// (removed, so a `var()` of it uses its fallback).
pub(crate) fn compute_custom_props(
    matches: &[(u32, &Declarations)],
    inline: Option<&Declarations>,
    parent: Option<&'static CustomProps>,
) -> Option<&'static CustomProps> {
    let mut own: HashMap<&str, (u32, &str)> = HashMap::new();
    for &(spec, decls) in matches {
        for (name, value) in &decls.custom_props {
            if own.get(name.as_str()).is_none_or(|(s, _)| spec >= *s) {
                own.insert(name, (spec, value));
            }
        }
    }
    if let Some(inl) = inline {
        for (name, value) in &inl.custom_props {
            own.insert(name, (u32::MAX, value));
        }
    }
    if own.is_empty() {
        return parent;
    }
    // `visiting` is the current resolution stack; every name on it when a
    // cycle is hit goes into `invalid` (the whole cycle is invalid).
    let visiting: RefCell<(Vec<String>, HashSet<String>)> = RefCell::new((Vec::new(), HashSet::new()));
    fn lookup(
        name: &str,
        own: &HashMap<&str, (u32, &str)>,
        parent: Option<&'static CustomProps>,
        visiting: &RefCell<(Vec<String>, HashSet<String>)>,
    ) -> Option<String> {
        if let Some((_, raw)) = own.get(name) {
            {
                let mut v = visiting.borrow_mut();
                if v.1.contains(name) {
                    return None;
                }
                if let Some(pos) = v.0.iter().position(|n| n == name) {
                    let cyc: Vec<String> = v.0[pos..].to_vec();
                    v.1.extend(cyc);
                    return None;
                }
                v.0.push(name.to_string());
            }
            let v = substitute_css_vars(raw, &|n| lookup(n, own, parent, visiting));
            let mut vis = visiting.borrow_mut();
            vis.0.pop();
            if vis.1.contains(name) {
                return None;
            }
            return v;
        }
        parent.and_then(|p| p.get(name).cloned())
    }
    let mut map = parent.cloned().unwrap_or_default();
    for name in own.keys() {
        match lookup(name, &own, parent, &visiting) {
            Some(v) => {
                map.insert(name.to_string(), v);
            }
            None => {
                map.remove(*name);
            }
        }
    }
    if Some(&map) == parent {
        return parent;
    }
    Some(intern_custom_props(map))
}

/// Replaces every `var(--name)` / `var(--name, <fallback>)` call in
/// `css` using `lookup`; an unknown name uses the (recursively
/// substituted) fallback. Quoted strings are copied verbatim. Returns
/// `None` when a lookup hit a cycle (a cycle makes the whole value
/// invalid), and an empty string for an unresolvable call with no
/// fallback, so that one declaration just fails to parse.
pub(crate) fn substitute_css_vars(css: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(bytes.len());
            out.push_str(&css[i..end]);
            i = end;
            continue;
        }
        if css[i..].starts_with("var(") && is_var_call_boundary(bytes, i) {
            let call_start = i + "var(".len();
            if let Some(call_end) = matching_close_paren(bytes, call_start) {
                out.push_str(&resolve_var_call(&css[call_start..call_end], lookup)?);
                i = call_end + 1;
                continue;
            }
        }
        let ch_len = css[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&css[i..i + ch_len]);
        i += ch_len;
    }
    Some(out)
}

/// `var(` shouldn't match as a trailing slice of a longer identifier
/// (`-var(`, `myvar(`) — require the preceding byte, if any, to not be
/// an identifier character.
pub(crate) fn is_var_call_boundary(bytes: &[u8], i: usize) -> bool {
    i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'-' || bytes[i - 1] == b'_')
}

/// `start` is the index of the first byte *after* an already-consumed
/// `(` — returns the index of its matching `)`, accounting for any
/// nested parens in between.
pub(crate) fn matching_close_paren(bytes: &[u8], start: usize) -> Option<usize> {
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
pub(crate) fn resolve_var_call(inner: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
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
    if let Some(v) = lookup(name) {
        return Some(v);
    }
    match fallback {
        Some(f) => substitute_css_vars(f, lookup),
        None => Some(String::new()),
    }
}

pub(crate) fn to_text_color(color: &CssColor) -> Option<TextColor> {
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
