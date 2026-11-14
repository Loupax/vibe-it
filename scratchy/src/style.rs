use super::*;

// ---------- cascade ----------

/// The subset of computed style that inherits from parent to child
/// (`color`, and — from v6 — `font-size`/`font-weight`/`font-style`),
/// threaded through tree-building instead of re-deriving from scratch.
#[derive(Clone, Copy)]
pub(crate) struct InheritedStyle {
    pub(crate) color: TextColor,
    pub(crate) font_size: f32,
    /// For `rem` lengths (`Length::Rem`) — resolved once from the real
    /// `<html>` element's own cascaded `font-size` (`build_layout_tree`),
    /// then copied unchanged through every other element's `inherited()`/
    /// `anonymous_style` rather than re-derived per element the way real
    /// inheritance works for every other field here, since `rem` is
    /// document-root-relative, not parent-relative.
    pub(crate) root_font_size: f32,
    pub(crate) font_weight: FontWeight,
    pub(crate) font_style: FontStyle,
    pub(crate) font_family: FontFamily,
    pub(crate) white_space: WhiteSpace,
    pub(crate) word_break: WordBreak,
    pub(crate) text_transform: TextTransform,
    pub(crate) list_style_type: ListStyleType,
    pub(crate) text_align: TextAlign,
    pub(crate) line_height: LineHeight,
    pub(crate) visibility: Visibility,
    pub(crate) cursor: CursorStyle,
    /// Real CSS inheritance (CSS2.1 §16.4 lists `word-spacing` as
    /// inherited, same as `letter-spacing`) — already resolved to a
    /// pixel value (like `border_width`/`outline_width`, not kept
    /// symbolic the way `line_height`'s bare-`<number>` form is), since
    /// `word-spacing`'s own grammar has no per-descendant-reinterpreted
    /// form the way `line-height: <number>` does.
    pub(crate) word_spacing: f32,
    /// The parent's computed values of three NON-inherited properties,
    /// carried only so the CSS-wide keyword `inherit` can read them
    /// (`apply_wide`); nothing else consults these.
    pub(crate) background_color: Option<TextColor>,
    pub(crate) display: Display,
    pub(crate) vertical_align: VerticalAlign,
    /// The parent's whole computed style, so `inherit` (and `unset` on
    /// inherited properties) can read any property (`apply_rest_wide`).
    /// `None` at the root.
    pub(crate) parent: Option<ComputedStyle>,
    /// The parent's resolved custom properties (Q13), inherited.
    pub(crate) custom_props: Option<&'static CustomProps>,
}

pub(crate) const ROOT_STYLE: InheritedStyle = InheritedStyle {
    color: TextColor::rgb(0, 0, 0),
    font_size: 16.0,
    root_font_size: 16.0,
    font_weight: FontWeight::NORMAL,
    font_style: FontStyle::Normal,
    font_family: FontFamily(&[FontFamilyItem::SansSerif]),
    white_space: WhiteSpace::Normal,
    word_break: WordBreak::Normal,
    text_transform: TextTransform::None,
    list_style_type: ListStyleType::Disc,
    text_align: TextAlign::Left,
    line_height: LineHeight::Normal,
    visibility: Visibility::Visible,
    cursor: CursorStyle::Auto,
    word_spacing: 0.0,
    background_color: None,
    display: Display::Inline,
    vertical_align: VerticalAlign::Baseline,
    parent: None,
    custom_props: None,
};

/// Per-side box-model values. Physical (top/right/bottom/left), not logical
/// — scratchy only ever lays out horizontal-tb/LTR, so block-start/end and
/// inline-start/end collapse onto top/bottom and left/right respectively;
/// logical property names are expanded to these physical sides at parse
/// time (`DeclParser::parse_value`), same as `margin`/`padding` shorthands
/// are. `border_width` is a separate per-side `[f32; 4]` (top, right, bottom,
/// left) on `ComputedStyle`, parsed by `parse_border_property` (Q4).
///
/// Fields are `BoxLength`, not a plain `f32` — real CSS resolves a `%`
/// margin/padding (on *every* side, even top/bottom — see
/// `finalize_box_lengths`) against the containing block's width, which
/// `margin`/`padding` on `ComputedStyle` can't know at cascade time. See
/// `BoxLength`'s own docs for exactly when/how that final resolution
/// happens.
#[derive(Clone, Copy, Default)]
pub(crate) struct Sides {
    pub(crate) top: BoxLength,
    pub(crate) right: BoxLength,
    pub(crate) bottom: BoxLength,
    pub(crate) left: BoxLength,
}

/// The properties a `::first-line` rule actually changed relative to the
/// element's own style (each `None` means "leave the span alone").
#[derive(Clone, Copy, Default)]
pub(crate) struct FirstLineStyle {
    pub(crate) color: Option<TextColor>,
    pub(crate) background_color: Option<TextColor>,
    pub(crate) font_size: Option<f32>,
    pub(crate) font_weight: Option<FontWeight>,
    pub(crate) font_style: Option<FontStyle>,
    pub(crate) line_height: Option<f32>,
    pub(crate) word_spacing: Option<f32>,
}

impl FirstLineStyle {
    pub(crate) fn apply(&self, span: &mut SpanStyle) {
        if let Some(v) = self.color { span.color = v; }
        if let Some(v) = self.background_color { span.background_color = Some(v); }
        if let Some(v) = self.font_size { span.font_size = v; }
        if let Some(v) = self.font_weight { span.font_weight = v; }
        if let Some(v) = self.font_style { span.font_style = v; }
        if let Some(v) = self.line_height { span.line_height = v; }
        if let Some(v) = self.word_spacing { span.word_spacing = v; }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ComputedStyle {
    /// Q31: `node_id` of the element this style was resolved for (0 for
    /// anonymous boxes). Lets a hit test name the element under the mouse.
    pub(crate) node: usize,
    /// This element's resolved custom properties (Q13): inherited from
    /// the parent, overlaid with its own `--name` declarations, every
    /// value already `var()`-substituted. Interned, see `intern_custom_props`.
    pub(crate) custom_props: Option<&'static CustomProps>,
    pub(crate) color: TextColor,
    pub(crate) background_color: Option<TextColor>,
    pub(crate) display: Display,
    /// Not inherited (see `Position`'s own docs). Forces `display` to
    /// `Block` when `Absolute`/`Fixed` — real CSS's own "computed
    /// `display`" rule (spec §9.7) — applied once, in `resolve_style`,
    /// right after both are cascaded; everywhere else in this file
    /// (`collect_children`'s block-vs-inline split, `measure`) can keep
    /// branching on `display` alone with no separate `position` check,
    /// since an absolutely/fixed positioned element is *always*
    /// block-level by the time either of them sees it.
    pub(crate) position: Position,
    /// `top`/`right`/`bottom`/`left` — only meaningful (resolved against
    /// a real containing block) once `position` isn't `Static`; see
    /// `Insets`'s own docs. Not inherited.
    pub(crate) inset: Insets,
    /// `None` means `auto` (`z-index`'s real initial value) — paints no
    /// differently from an explicit `0` (see `catch_or_bubble_
    /// positioned`'s sort key), the one simplification this file's
    /// stacking-order support makes vs. the real spec (where `auto`
    /// additionally means "don't start a new stacking context" — a
    /// distinction this file doesn't model at all; see TODO.txt). Not
    /// inherited, and only ever consulted for a `position`-ed box (an
    /// in-flow box's own `z_index` is simply never read).
    pub(crate) z_index: Option<i32>,
    /// `opacity` (Q17), already multiplied by every ancestor's, so it is
    /// the effective group alpha for this box and everything painted in
    /// it. Not inherited as such, but accumulated down the tree in
    /// `resolve_style` via `InheritedStyle::parent`. Paint scales every
    /// alpha by it (`paint` publishes it to `blend`). Known limitation:
    /// overlapping semi-transparent children of one faded parent
    /// double-blend, unlike real group compositing.
    pub(crate) opacity: f32,
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
    pub(crate) float: Float,
    /// Not inherited. See `Clear`'s own docs.
    pub(crate) clear: Clear,
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
    pub(crate) width: Option<BoxLength>,
    /// `width: min-content|max-content|fit-content` (Q20f); `width` is
    /// `None` then and `measure` sizes the box from its content.
    pub(crate) width_kw: Option<IntrinsicSize>,
    pub(crate) min_width_kw: Option<IntrinsicSize>,
    pub(crate) height: Option<BoxLength>,
    pub(crate) min_width: Option<BoxLength>,
    pub(crate) max_width: Option<BoxLength>,
    pub(crate) min_height: Option<BoxLength>,
    pub(crate) max_height: Option<BoxLength>,
    /// Not inherited (same as `overflow`/`border_width`). See
    /// `BoxSizing`'s own docs.
    pub(crate) box_sizing: BoxSizing,
    /// `display: list-item` was cascaded (not inherited); generates a marker.
    pub(crate) list_item: bool,
    /// `display: flow-root` was cascaded (not inherited) — Q34b.
    pub(crate) flow_root: bool,
    /// `display: table-caption` was cascaded (not inherited) — Q34c.
    pub(crate) table_caption: bool,
    /// `caption-side: bottom` (not inherited) — Q34c.
    pub(crate) caption_bottom: bool,
    /// `colspan`/`rowspan` of a table cell (Q36), set by
    /// `build_table_row_cells` from the attributes; 1 everywhere else.
    pub(crate) col_span: usize,
    pub(crate) row_span: usize,
    pub(crate) margin: Sides,
    pub(crate) padding: Sides,
    /// Per-side (top, right, bottom, left). Width is already 0 for a side
    /// whose style is `none`/`hidden`.
    pub(crate) border_width: [f32; 4],
    pub(crate) border_color: [Option<TextColor>; 4],
    pub(crate) border_style: [BorderStyle; 4],
    pub(crate) font_size: f32,
    /// Passed straight through from `inherited.root_font_size`, not
    /// derived from this element's own `font_size` — see
    /// `InheritedStyle::root_font_size`'s own docs.
    pub(crate) root_font_size: f32,
    pub(crate) font_weight: FontWeight,
    pub(crate) font_style: FontStyle,
    pub(crate) font_family: FontFamily,
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
    pub(crate) text_decoration: Option<DecorationValue>,
    /// `text-decoration-line`/`-style`/`-color` (v60), each cascaded
    /// completely independently of `text_decoration` (and of each other)
    /// — `None` means not declared on *this* element, same convention as
    /// every other cascaded-but-not-inherited field here. Combined with
    /// `text_decoration` and the ambient propagated value together only
    /// at used-value time, by `resolve_own_decoration` — never folded
    /// into `text_decoration` itself during cascade, so a rule that
    /// declares only `text-decoration-color` genuinely doesn't clobber
    /// whatever line/style an ancestor or the shorthand already set.
    pub(crate) text_decoration_line: Option<DecorationLine>,
    pub(crate) text_decoration_style: Option<DecorationStyle>,
    pub(crate) text_decoration_color: Option<TextColor>,
    pub(crate) white_space: WhiteSpace,
    pub(crate) word_break: WordBreak,
    pub(crate) text_transform: TextTransform,
    pub(crate) list_style_type: ListStyleType,
    pub(crate) text_align: TextAlign,
    /// Not inherited (see `VerticalAlign`'s own docs) — propagates the
    /// same ambient way `text_decoration` does, via `collect_children`'s
    /// `valign` parameter, not through `inherited()`/`InheritedStyle`.
    pub(crate) vertical_align: VerticalAlign,
    /// Still symbolic (see `LineHeight`'s own docs) — only resolved to a
    /// pixel value in `SpanStyle::from`, where a concrete number is
    /// finally needed.
    pub(crate) line_height: LineHeight,
    pub(crate) visibility: Visibility,
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
    pub(crate) word_spacing: f32,
    /// Not inherited (same as `border_width`/`border_color`) and doesn't
    /// affect layout at all (unlike `border`, which is part of the box
    /// model) — drawn one `outline_width` ring *outside* the border box,
    /// in `paint_box_decoration`, with no `outline-offset` gap (nothing
    /// needs one yet).
    pub(crate) outline_width: f32,
    pub(crate) outline_color: Option<TextColor>,
    /// `None` means no `box-shadow` declared (or an unsupported form —
    /// see `BoxShadow`'s own docs). Not inherited.
    pub(crate) box_shadow: [Option<ResolvedBoxShadow>; MAX_SHADOWS],
    /// `background-position`/`-size`/`-repeat` (Q16), applied by
    /// `paint_background_image`. Not inherited.
    pub(crate) bg_layout: BgLayout,
    /// `mask-position`/`-size`/`-repeat` (Q29), same shape as `bg_layout`.
    pub(crate) mask_layout: BgLayout,
    /// `object-fit` and `object-position` (Q28), applied by `blit_image`.
    /// Position is `(fraction of free space, px)` per axis, like
    /// `bg_layout`. Not inherited.
    pub(crate) object_fit: ObjectFit,
    pub(crate) object_pos: ((f32, f32), (f32, f32)),
    /// Rounds both the background fill and (for `BorderStyle::Solid`
    /// only — see `fill_rounded_ring`'s own docs) the border itself, via
    /// `paint_box_decoration`. Not inherited. `BoxLength`, not a plain
    /// `f32`: real CSS resolves a `%` border-radius against the box's
    /// *own* border-box width (unlike `width`/`margin`/`padding`, whose
    /// basis is the *containing block's* width) — `paint_box_decoration`
    /// resolves it directly against its own `w` parameter, since that's
    /// already the exact value needed and it's never used anywhere
    /// `measure` would otherwise have to finalize it early.
    pub(crate) border_radius: BoxLength,
    /// Not inherited. See `Overflow`'s own docs for the clip's scope.
    pub(crate) overflow: Overflow,
    /// `text-overflow` (Q21). Not inherited; `build_box_for_element` copies
    /// an `Ellipsis` onto the anonymous inline runs of a clipping box.
    pub(crate) text_overflow: TextOverflow,
    /// Only ever `Some` on the synthetic document-root box: the window
    /// height, i.e. the initial containing block's definite height. It
    /// feeds `child_containing_height` (so `html{height:100%}` resolves)
    /// without pinning the root's own height, which `redraw` reads as
    /// the scrollable content height.
    pub(crate) icb_height: Option<f32>,
    /// `::first-line` overrides (Q19e2), set only on the anonymous inline
    /// run box that `apply_first_line` picks. Applied at layout time by
    /// `measure`'s `Inline` arm once the first line's extent is known.
    pub(crate) first_line: Option<FirstLineStyle>,
    pub(crate) cursor: CursorStyle,
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
    pub(crate) flex_direction: FlexDirection,
    pub(crate) flex_wrap: FlexWrap,
    pub(crate) justify_content: JustifyContent,
    pub(crate) align_content: AlignContent,
    pub(crate) align_items: AlignItems,
    /// `None` means `auto` (the real initial value) — resolved against
    /// the *containing* flex container's own `align_items` only at
    /// layout time (`measure_flex`), not here, since `resolve_style`
    /// doesn't know whether this element's parent is even a flex
    /// container at all.
    pub(crate) align_self: Option<AlignItems>,
    pub(crate) flex_grow: f32,
    pub(crate) flex_shrink: f32,
    pub(crate) flex_basis: FlexBasis,
    /// `order`; see `Declarations::order`.
    pub(crate) order: i32,
    /// Resolved eagerly (like `border_width`/`outline_width`) rather
    /// than deferred as a `BoxLength` — real CSS allows a `%` `row-gap`/
    /// `column-gap` (resolving against the flex container's own cross/
    /// main size respectively), but that's deliberately not supported
    /// here (see `TODO.txt`): only `px`/`em`/`rem` parse at all
    /// (`parse_length`, not `parse_length_percent`), so there's no
    /// deferred-percent case to carry through to `measure_flex` the way
    /// `width`/`margin` do.
    pub(crate) row_gap: f32,
    pub(crate) column_gap: f32,
    /// Not inherited, same category as the flexbox-only fields above —
    /// a `display: grid` container's own explicit column/row tracks
    /// never reach past it to a grandchild. `None` means no `grid-
    /// template-columns`/`-rows` declared at all — `measure_grid` falls
    /// back to a single full-width implicit column for `grid_template_
    /// columns` and purely content-sized implicit rows for `grid_
    /// template_rows` either way (see its own docs), so `None` and
    /// "every row/column is auto" are already the right default.
    pub(crate) grid_template_columns: Option<&'static [GridTrackSpec]>,
    pub(crate) grid_template_rows: Option<&'static [GridTrackSpec]>,
    /// `None` means no `grid-template-areas` declared — `measure_grid`
    /// falls back to plain auto-placement, same as today, when this is
    /// unset. See `parse_grid_template_areas`'s own docs.
    pub(crate) grid_template_areas: Option<&'static [&'static str]>,
    /// This box's own `grid-area` — read by its *parent*, in
    /// `measure_grid`, when laying this box out as one of its own grid
    /// items (same "an item property read off the item itself, not
    /// inherited, not meaningful on a container" shape `flex_grow`/
    /// `align_self` already have).
    pub(crate) grid_area: Option<&'static str>,
    /// Numeric placement longhands (`grid-row-start` etc.), see `GridLine`.
    pub(crate) grid_row_start: Option<GridLine>,
    pub(crate) grid_row_end: Option<GridLine>,
    pub(crate) grid_column_start: Option<GridLine>,
    pub(crate) grid_column_end: Option<GridLine>,
}

/// `BoxShadow` with its lengths resolved to pixels (against the
/// element's own font-size, same timing as `border-width`/`padding`'s
/// own `em` resolution) and its color converted to a paintable
/// `TextColor` — the form `paint_box_decoration` actually draws.
#[derive(Clone, Copy)]
pub(crate) struct ResolvedBoxShadow {
    pub(crate) offset_x: f32,
    pub(crate) offset_y: f32,
    pub(crate) blur: f32,
    pub(crate) spread: f32,
    pub(crate) color: TextColor,
    pub(crate) inset: bool,
}

impl ComputedStyle {
    /// Whether this box starts its own block formatting context (Q34b):
    /// float, `overflow` other than visible, `display: flow-root`, or a
    /// block-level table/flex/grid container.
    pub(crate) fn establishes_bfc(&self) -> bool {
        self.float != Float::None
            || self.overflow != Overflow::Visible
            || self.flow_root
            || matches!(self.display, Display::Table | Display::Flex | Display::Grid)
    }

    pub(crate) fn border_top(&self) -> f32 {
        self.border_width[0]
    }
    pub(crate) fn border_left(&self) -> f32 {
        self.border_width[3]
    }
    /// Left + right border width.
    pub(crate) fn border_h(&self) -> f32 {
        self.border_width[1] + self.border_width[3]
    }
    /// Top + bottom border width.
    pub(crate) fn border_v(&self) -> f32 {
        self.border_width[0] + self.border_width[2]
    }

    pub(crate) fn inherited(&self) -> InheritedStyle {
        InheritedStyle {
            color: self.color,
            font_size: self.font_size,
            root_font_size: self.root_font_size,
            font_weight: self.font_weight,
            font_style: self.font_style,
            font_family: self.font_family,
            white_space: self.white_space,
            word_break: self.word_break,
            text_transform: self.text_transform,
            list_style_type: self.list_style_type,
            text_align: self.text_align,
            line_height: self.line_height,
            visibility: self.visibility,
            cursor: self.cursor,
            word_spacing: self.word_spacing,
            background_color: self.background_color,
            display: self.display,
            vertical_align: self.vertical_align,
            parent: Some(*self),
            custom_props: self.custom_props,
        }
    }
}


pub(crate) fn cascade_property<T: Clone>(
    matches: &[(u32, &Declarations)],
    inline: Option<T>,
    get: impl Fn(&Declarations) -> Option<T>,
) -> Option<T> {
    let mut best: Option<(u32, T)> = None;
    for &(specificity, decls) in matches {
        if let Some(v) = get(decls) {
            if best.as_ref().map_or(true, |(s, _)| specificity >= *s) {
                best = Some((specificity, v));
            }
        }
    }
    match best {
        // An `!important` declaration (rule or inline) beats an inline
        // normal one.
        Some((s, v)) if s >= IMPORTANT_TIER => Some(v),
        Some((_, v)) => inline.or(Some(v)),
        None => inline,
    }
}

/// A CSS-wide keyword (`inherit`, `initial`, `unset`, `revert`,
/// `revert-layer`). `revert`/`revert-layer` are stored as `Revert` and
/// behave as `unset` (no cascade layers or UA-origin split here).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum CssWide {
    Inherit,
    Initial,
    Unset,
    Revert,
}

impl CssWide {
    pub(crate) fn from_ident(ident: &str) -> Option<CssWide> {
        let l = ident.to_ascii_lowercase();
        match l.as_str() {
            "inherit" => Some(CssWide::Inherit),
            "initial" => Some(CssWide::Initial),
            "unset" => Some(CssWide::Unset),
            "revert" | "revert-layer" => Some(CssWide::Revert),
            _ => None,
        }
    }

    /// Whether this keyword takes the parent's computed value for a
    /// property that is (`inherited_prop`) or isn't inherited by default.
    pub(crate) fn takes_parent(self, inherited_prop: bool) -> bool {
        match self {
            CssWide::Inherit => true,
            CssWide::Initial => false,
            CssWide::Unset | CssWide::Revert => inherited_prop,
        }
    }
}

pub(crate) static BORDER_WIDE_KEYS: [&str; 12] = [
    "border-top-width", "border-right-width", "border-bottom-width", "border-left-width",
    "border-top-style", "border-right-style", "border-bottom-style", "border-left-style",
    "border-top-color", "border-right-color", "border-bottom-color", "border-left-color",
];

/// `border-<side>-<width|style|color>` canonical key to (side index,
/// 'w'/'s'/'c').
pub(crate) fn border_key(key: &str) -> Option<(usize, char)> {
    let rest = key.strip_prefix("border-")?;
    let (side, prop) = rest.split_once('-')?;
    let i = SIDE_NAMES.iter().position(|n| *n == side)?;
    match prop {
        "width" => Some((i, 'w')),
        "style" => Some((i, 's')),
        "color" => Some((i, 'c')),
        _ => None,
    }
}

/// Canonical keys of the text-decoration shorthand and its longhands.
pub(crate) static TEXT_DECO_KEYS: [&str; 4] = [
    "text-decoration", "text-decoration-line", "text-decoration-style", "text-decoration-color",
];

/// Canonical `Declarations::wide` keys a property name (longhand or
/// shorthand) expands to. `None` for a property that doesn't support
/// CSS-wide keywords (`background` maps to colour + image).
pub(crate) fn wide_keys(name: &str) -> Option<&'static [&'static str]> {
    const SINGLE: &[&str] = &[
        "color", "background-color", "background-image", "background-position",
        "background-size", "background-repeat", "display", "font-size",
        "line-height", "vertical-align", "font-family", "font-weight", "text-align",
        "white-space", "cursor", "visibility", "position", "float", "clear", "top",
        "right", "bottom", "left", "z-index", "opacity", "width", "height", "min-width",
        "max-width", "min-height", "max-height", "box-sizing", "margin-top",
        "margin-right", "margin-bottom", "margin-left", "padding-top",
        "padding-right", "padding-bottom", "padding-left", "outline-width",
        "outline-color", "box-shadow", "border-radius", "overflow",
        "overflow-y", "text-overflow", "flex-direction", "flex-wrap", "justify-content", "align-items",
        "align-self", "order", "flex-grow", "flex-shrink", "flex-basis", "row-gap",
        "column-gap", "grid-template-columns", "grid-template-rows",
        "grid-template-areas", "grid-area", "font-style", "list-style-type",
        "word-spacing",
    ];
    let l = name.to_ascii_lowercase();
    let l = l.as_str();
    if let Some(k) = SINGLE.iter().copied().find(|k| *k == l) {
        // Slice of the `SINGLE` entry itself, so it is `'static`.
        let idx = SINGLE.iter().position(|x| *x == k)?;
        return Some(std::slice::from_ref(&SINGLE[idx]));
    }
    if let Some(k) = BORDER_WIDE_KEYS.iter().position(|k| *k == l) {
        return Some(std::slice::from_ref(&BORDER_WIDE_KEYS[k]));
    }
    Some(match l {
        "background" => &[
            "background-color", "background-image", "background-position",
            "background-size", "background-repeat",
        ],
        "font" => &["font-size", "line-height", "font-family", "font-weight", "font-style"],
        "margin" => &["margin-top", "margin-right", "margin-bottom", "margin-left"],
        "margin-block-start" => &["margin-top"],
        "margin-block-end" => &["margin-bottom"],
        "margin-inline-start" => &["margin-left"],
        "margin-inline-end" => &["margin-right"],
        "margin-block" => &["margin-top", "margin-bottom"],
        "margin-inline" => &["margin-left", "margin-right"],
        "padding" => &["padding-top", "padding-right", "padding-bottom", "padding-left"],
        "padding-block-start" => &["padding-top"],
        "padding-block-end" => &["padding-bottom"],
        "padding-inline-start" => &["padding-left"],
        "padding-inline-end" => &["padding-right"],
        "padding-block" => &["padding-top", "padding-bottom"],
        "padding-inline" => &["padding-left", "padding-right"],
        "border" => &BORDER_WIDE_KEYS,
        "border-width" => &BORDER_WIDE_KEYS[0..4],
        "border-style" => &BORDER_WIDE_KEYS[4..8],
        "border-color" => &BORDER_WIDE_KEYS[8..12],
        "border-top" => &["border-top-width", "border-top-style", "border-top-color"],
        "border-right" => &["border-right-width", "border-right-style", "border-right-color"],
        "border-bottom" => &["border-bottom-width", "border-bottom-style", "border-bottom-color"],
        "border-left" => &["border-left-width", "border-left-style", "border-left-color"],
        "outline" => &["outline-width", "outline-color"],
        "flex-flow" => &["flex-direction", "flex-wrap"],
        "flex" => &["flex-grow", "flex-shrink", "flex-basis"],
        "gap" => &["row-gap", "column-gap"],
        "list-style" => &["list-style-type"],
        "text-decoration" => &TEXT_DECO_KEYS,
        "text-decoration-line" => &TEXT_DECO_KEYS[1..2],
        "text-decoration-style" => &TEXT_DECO_KEYS[2..3],
        "text-decoration-color" => &TEXT_DECO_KEYS[3..4],
        "inset" => &["top", "right", "bottom", "left"],
        "inset-block" => &["top", "bottom"],
        "inset-inline" => &["left", "right"],
        "inset-block-start" => &["top"],
        "inset-block-end" => &["bottom"],
        "inset-inline-start" => &["left"],
        "inset-inline-end" => &["right"],
        "border-block" => &[
            "border-top-width", "border-bottom-width", "border-top-style",
            "border-bottom-style", "border-top-color", "border-bottom-color",
        ],
        "border-inline" => &[
            "border-left-width", "border-right-width", "border-left-style",
            "border-right-style", "border-left-color", "border-right-color",
        ],
        "border-block-width" => &["border-top-width", "border-bottom-width"],
        "border-block-style" => &["border-top-style", "border-bottom-style"],
        "border-block-color" => &["border-top-color", "border-bottom-color"],
        "border-inline-width" => &["border-left-width", "border-right-width"],
        "border-inline-style" => &["border-left-style", "border-right-style"],
        "border-inline-color" => &["border-left-color", "border-right-color"],
        "border-block-start" => &["border-top-width", "border-top-style", "border-top-color"],
        "border-block-end" => &["border-bottom-width", "border-bottom-style", "border-bottom-color"],
        "border-inline-start" => &["border-left-width", "border-left-style", "border-left-color"],
        "border-inline-end" => &["border-right-width", "border-right-style", "border-right-color"],
        "border-block-start-width" => &["border-top-width"],
        "border-block-start-style" => &["border-top-style"],
        "border-block-start-color" => &["border-top-color"],
        "border-block-end-width" => &["border-bottom-width"],
        "border-block-end-style" => &["border-bottom-style"],
        "border-block-end-color" => &["border-bottom-color"],
        "border-inline-start-width" => &["border-left-width"],
        "border-inline-start-style" => &["border-left-style"],
        "border-inline-start-color" => &["border-left-color"],
        "border-inline-end-width" => &["border-right-width"],
        "border-inline-end-style" => &["border-right-style"],
        "border-inline-end-color" => &["border-right-color"],
        _ => return None,
    })
}

/// Whether a canonical key names an inherited property.
pub(crate) fn is_inherited_key(key: &str) -> bool {
    matches!(
        key,
        "color" | "font-size" | "line-height" | "font-family" | "font-weight" | "text-align"
            | "white-space" | "cursor" | "visibility" | "font-style" | "list-style-type"
            | "word-spacing"
    )
}

/// Copies the computed value of the canonical key `key` from `src` into
/// `dst`. Keys with no `ComputedStyle` field (colour/font properties
/// handled in `resolve_style` itself, `background-image`) are no-ops.
pub(crate) fn copy_prop(dst: &mut ComputedStyle, src: &ComputedStyle, key: &str) {
    if let Some((i, kind)) = border_key(key) {
        match kind {
            'w' => dst.border_width[i] = src.border_width[i],
            's' => dst.border_style[i] = src.border_style[i],
            _ => dst.border_color[i] = src.border_color[i],
        }
        return;
    }
    match key {
        "position" => dst.position = src.position,
        "float" => dst.float = src.float,
        "clear" => dst.clear = src.clear,
        "top" => dst.inset.top = src.inset.top,
        "right" => dst.inset.right = src.inset.right,
        "bottom" => dst.inset.bottom = src.inset.bottom,
        "left" => dst.inset.left = src.inset.left,
        "z-index" => dst.z_index = src.z_index,
        "opacity" => dst.opacity = src.opacity,
        "width" => dst.width = src.width,
        "height" => dst.height = src.height,
        "min-width" => dst.min_width = src.min_width,
        "max-width" => dst.max_width = src.max_width,
        "min-height" => dst.min_height = src.min_height,
        "max-height" => dst.max_height = src.max_height,
        "box-sizing" => dst.box_sizing = src.box_sizing,
        "margin-top" => dst.margin.top = src.margin.top,
        "margin-right" => dst.margin.right = src.margin.right,
        "margin-bottom" => dst.margin.bottom = src.margin.bottom,
        "margin-left" => dst.margin.left = src.margin.left,
        "padding-top" => dst.padding.top = src.padding.top,
        "padding-right" => dst.padding.right = src.padding.right,
        "padding-bottom" => dst.padding.bottom = src.padding.bottom,
        "padding-left" => dst.padding.left = src.padding.left,
        "outline-width" => dst.outline_width = src.outline_width,
        "outline-color" => dst.outline_color = src.outline_color,
        "box-shadow" => dst.box_shadow = src.box_shadow,
        "border-radius" => dst.border_radius = src.border_radius,
        "overflow" => dst.overflow = src.overflow,
        "text-overflow" => dst.text_overflow = src.text_overflow,
        "flex-direction" => dst.flex_direction = src.flex_direction,
        "flex-wrap" => dst.flex_wrap = src.flex_wrap,
        "justify-content" => dst.justify_content = src.justify_content,
        "align-content" => dst.align_content = src.align_content,
        "align-items" => dst.align_items = src.align_items,
        "align-self" => dst.align_self = src.align_self,
        "order" => dst.order = src.order,
        "flex-grow" => dst.flex_grow = src.flex_grow,
        "flex-shrink" => dst.flex_shrink = src.flex_shrink,
        "flex-basis" => dst.flex_basis = src.flex_basis,
        "row-gap" => dst.row_gap = src.row_gap,
        "column-gap" => dst.column_gap = src.column_gap,
        "grid-template-columns" => dst.grid_template_columns = src.grid_template_columns,
        "grid-template-rows" => dst.grid_template_rows = src.grid_template_rows,
        "grid-template-areas" => dst.grid_template_areas = src.grid_template_areas,
        "grid-area" => dst.grid_area = src.grid_area,
        "font-style" => dst.font_style = src.font_style,
        "list-style-type" => dst.list_style_type = src.list_style_type,
        "word-spacing" => dst.word_spacing = src.word_spacing,
        _ => {}
    }
}

/// The post-pass of `resolve_style` for CSS-wide keywords on every
/// property outside the core set `cascade_css` already handles inline:
/// for each such key whose cascade winner is a keyword, overwrite the
/// computed field with the parent's (`inherit`, or `unset` on an
/// inherited property) or the initial value.
pub(crate) fn apply_rest_wide(
    cs: &mut ComputedStyle,
    matches: &[(u32, &Declarations)],
    inline: Option<&Declarations>,
    inherited: &InheritedStyle,
) {
    let mut keys: Vec<&'static str> = Vec::new();
    for d in matches.iter().map(|(_, d)| *d).chain(inline) {
        for (k, _) in &d.wide {
            if !keys.contains(k) {
                keys.push(k);
            }
        }
    }
    if keys.is_empty() {
        return;
    }
    let initial = anonymous_style(ROOT_STYLE);
    for key in keys {
        if let Cascaded::Wide(kw) = cascade_css(matches, inline, key, |d| d.has_typed(key).then_some(())) {
            let src = if kw.takes_parent(is_inherited_key(key)) {
                inherited.parent.as_ref().unwrap_or(&initial)
            } else {
                &initial
            };
            copy_prop(cs, src, key);
        }
    }
}

/// The winner of a cascade that can also see CSS-wide keywords.
pub(crate) enum Cascaded<T> {
    Value(T),
    Wide(CssWide),
    Unset,
}

/// Like `cascade_property`, but a CSS-wide keyword declared for `key`
/// (`Declarations::wide`) competes at its own specificity, source order
/// and `!important` tier exactly like a normal value.
pub(crate) fn cascade_css<T: Clone>(
    matches: &[(u32, &Declarations)],
    inline: Option<&Declarations>,
    key: &str,
    get: impl Fn(&Declarations) -> Option<T>,
) -> Cascaded<T> {
    let cand = |d: &Declarations| -> Option<Cascaded<T>> {
        if let Some((_, kw)) = d.wide.iter().find(|(k, _)| *k == key) {
            Some(Cascaded::Wide(*kw))
        } else {
            get(d).map(Cascaded::Value)
        }
    };
    let mut best: Option<(u32, Cascaded<T>)> = None;
    for &(specificity, decls) in matches {
        if let Some(c) = cand(decls)
            && best.as_ref().is_none_or(|(s, _)| specificity >= *s)
        {
            best = Some((specificity, c));
        }
    }
    let inline = inline.and_then(cand);
    match best {
        Some((s, c)) if s >= IMPORTANT_TIER => c,
        Some((_, c)) => inline.unwrap_or(c),
        None => inline.unwrap_or(Cascaded::Unset),
    }
}

/// Resolves a `cascade_css` result: a real value goes through `f`
/// (falling back as if undeclared when `f` rejects it), a keyword takes
/// the parent's or the initial value, and "nothing declared" is the
/// property's natural default (inherit for inherited properties).
pub(crate) fn apply_wide<T, V>(
    c: Cascaded<V>,
    inherited_prop: bool,
    parent: T,
    initial: T,
    f: impl FnOnce(V) -> Option<T>,
) -> T {
    let pick = |parent: T, initial: T, take_parent: bool| if take_parent { parent } else { initial };
    match c {
        Cascaded::Value(v) => match f(v) {
            Some(t) => t,
            None => pick(parent, initial, inherited_prop),
        },
        Cascaded::Wide(kw) => pick(parent, initial, kw.takes_parent(inherited_prop)),
        Cascaded::Unset => pick(parent, initial, inherited_prop),
    }
}

/// Specificities are 30 bits; this bit marks the `!important` tier.
pub(crate) const IMPORTANT_TIER: u32 = 1 << 31;

/// Every rule in `rules` whose selector matches `handle`, paired with its
/// specificity — the real selector-matching work `resolve_style` itself
/// needs, factored out so `resolve_background_image` can reuse it
/// (instead of re-running `matches_selector_list` over every rule a
/// second time) rather than folding background-image into
/// `ComputedStyle` itself — see `resolve_background_image`'s own docs
/// for why it's resolved separately at all.
pub(crate) fn matching_declarations<'a>(handle: &Handle, rules: &'a [StyleRule], viewport_width: f32) -> Vec<(u32, &'a Declarations)> {
    matching_declarations_for(handle, rules, viewport_width, None)
}

/// `matching_declarations`, optionally for a generated pseudo-element of
/// `handle` (`Some("before")`/`Some("after")`, Q19): then only selectors
/// carrying exactly that pseudo-element count, matched against the
/// originating element in `ForStatelessPseudoElement` mode.
pub(crate) fn matching_declarations_for<'a>(
    handle: &Handle,
    rules: &'a [StyleRule],
    viewport_width: f32,
    pseudo: Option<&str>,
) -> Vec<(u32, &'a Declarations)> {
    let element = El(handle.clone());
    let mut caches = SelectorCaches::default();
    let mut matches: Vec<(u32, &Declarations)> = Vec::new();
    // R3.3: when an index was registered for exactly this rule slice,
    // only test the rules whose rightmost compound could match this
    // element (ascending rule order preserved, so cascade ties are
    // unchanged); otherwise test every rule.
    let candidates = RULE_INDEX.with(|ri| {
        let ri = ri.borrow();
        let (ptr, len, idx) = ri.as_ref()?;
        (*ptr == rules.as_ptr() as usize && *len == rules.len()).then(|| idx.candidates(handle, pseudo.is_some()))
    });
    let all: Vec<usize>;
    let order: &mut dyn Iterator<Item = &'a StyleRule> = &mut match &candidates {
        Some(c) => {
            all = c.clone();
            Box::new(all.into_iter().map(|i| &rules[i])) as Box<dyn Iterator<Item = &'a StyleRule>>
        }
        None => Box::new(rules.iter()),
    };
    for rule in order {
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
        if let Some(name) = pseudo {
            let mut best: Option<u32> = None;
            for sel in rule.selectors.slice() {
                if !matches!(sel.pseudo_element(), Some(OurPseudoElement::Named(n)) if *n == name) {
                    continue;
                }
                let mut ctx = MatchingContext::new(
                    MatchingMode::ForStatelessPseudoElement,
                    None,
                    &mut caches,
                    QuirksMode::NoQuirks,
                    NeedsSelectorFlags::No,
                    MatchingForInvalidation::No,
                );
                if matches_selector(sel, 0, None, &element, &mut ctx) {
                    best = best.max(Some(sel.specificity()));
                }
            }
            if let Some(specificity) = best {
                matches.push((specificity, &rule.decls));
                matches.push((specificity | IMPORTANT_TIER, &rule.important));
            }
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
                .filter(|s| !s.has_pseudo_element())
                .map(|s| s.specificity())
                .max()
                .unwrap_or(0);
            matches.push((specificity, &rule.decls));
            matches.push((specificity | IMPORTANT_TIER, &rule.important));
        }
    }
    matches
}

/// `background-image`'s own declared value, as stored in `Declarations`
/// — either a real `url(...)` or the literal `none` keyword, which real
/// CSS needs to win the cascade at its own specificity (overriding a
/// real image a *less*-specific rule declared) exactly like any other
/// explicit value would. Plain `Option<String>` can't represent that:
/// both "this rule said `none`" and "this rule never mentioned
/// `background-image` at all" would collapse to the same `None`, so
/// `cascade_property` (which skips a rule entirely when its own
/// accessor returns `None`) could never let an explicit `none` override
/// a less-specific real image — it would just fall through to that
/// image as if `none` had never been declared. Same shape `box-shadow`'s
/// own `ShadowDecl` already established for the identical problem.
#[derive(Clone, PartialEq)]
pub(crate) enum BackgroundImageDecl {
    None,
    Url(String),
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
pub(crate) fn resolve_background_image(
    handle: &Handle,
    rules: &[StyleRule],
    viewport_width: f32,
    custom_props: Option<&'static CustomProps>,
) -> Option<Arc<DecodedImage>> {
    let base_matches = matching_declarations(handle, rules, viewport_width);
    let var_lookup = |n: &str| custom_props.and_then(|p| p.get(n).cloned());
    let synth = expand_var_decls(&base_matches, &var_lookup);
    let mut matches = merge_var_matches(&base_matches, &synth);
    let inline_src = element_attr(handle, "style").map(|s| substitute_css_vars(&s, &var_lookup).unwrap_or_default());
    let inline_split = inline_src.map(|s| parse_declarations_split(&s));
    let (inline_decls, inline_important) = match inline_split {
        Some((n, i)) => (Some(n), Some(i)),
        None => (None, None),
    };
    if let Some(i) = inline_important.as_ref() {
        matches.push((u32::MAX, i));
    }
    let bg = match cascade_css(&matches, inline_decls.as_ref(), "background-image", |d| d.background_image.clone()) {
        Cascaded::Value(BackgroundImageDecl::Url(url)) => decode_data_uri(&url).and_then(|b| decode_image_bytes(&b)),
        // `inherit` takes the parent element's resolved image;
        // `background-image` isn't inherited otherwise.
        Cascaded::Wide(CssWide::Inherit) => parent_handle(handle)
            .filter(|p| matches!(p.data, NodeData::Element { .. }))
            .and_then(|parent| resolve_background_image(&parent, rules, viewport_width, custom_props))
            .and_then(|a| Arc::try_unwrap(a).ok())
            .map(|mut i| {
                i.mask = None;
                i
            }),
        // An explicit `none`, another CSS-wide keyword, or undeclared.
        _ => None,
    };
    // `mask-image` (Q29), same data-URI policy; not inherited.
    let mask = match cascade_css(&matches, inline_decls.as_ref(), "mask-image", |d| d.mask_image.clone()) {
        Cascaded::Value(BackgroundImageDecl::Url(url)) => {
            decode_data_uri(&url).and_then(|b| decode_image_bytes(&b)).map(whiten_mask).map(Arc::new)
        }
        _ => None,
    };
    match (bg, mask) {
        (None, None) => None,
        (Some(img), None) => Some(Arc::new(img)),
        (img, mask) => {
            let mut img = img.unwrap_or(DecodedImage { width: 0, height: 0, pixels: Vec::new(), svg: None, mask: None });
            img.mask = mask;
            Some(Arc::new(img))
        }
    }
}

/// Turns a decoded mask image into white pixels carrying its alpha, so
/// painting it onto a black scratch buffer yields the coverage (Q29).
pub(crate) fn whiten_mask(mut img: DecodedImage) -> DecodedImage {
    for p in img.pixels.iter_mut() {
        *p = TextColor::rgba(255, 255, 255, p.a());
    }
    img
}

/// Re-parses each matched rule's `var()`-bearing declarations with
/// `lookup`'s custom properties substituted (Q13); one slot per match.
pub(crate) fn expand_var_decls(
    base: &[(u32, &Declarations)],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Vec<Option<(Declarations, Declarations)>> {
    base.iter()
        .map(|(_, d)| {
            if d.var_decls.is_empty() {
                return None;
            }
            let mut src = String::new();
            for (name, raw) in &d.var_decls {
                src.push_str(name);
                src.push(':');
                src.push_str(&substitute_css_vars(raw, lookup).unwrap_or_default());
                src.push(';');
            }
            Some(parse_declarations_split(&src))
        })
        .collect()
}

/// Slots each `expand_var_decls` result right after its rule's own
/// entries, so source order still breaks specificity ties.
pub(crate) fn merge_var_matches<'a>(
    base: &[(u32, &'a Declarations)],
    synth: &'a [Option<(Declarations, Declarations)>],
) -> Vec<(u32, &'a Declarations)> {
    let mut matches = Vec::with_capacity(base.len() + 2);
    for (idx, &(spec, d)) in base.iter().enumerate() {
        matches.push((spec, d));
        if let Some(Some((n, i))) = synth.get(idx) {
            matches.push((spec, n));
            matches.push((spec | IMPORTANT_TIER, i));
        }
    }
    matches
}

/// Rule index (R3.3): rules bucketed by the id / class / tag of each
/// selector's rightmost compound, like real engines. Only a candidate
/// filter — every candidate still goes through the real selector match.
/// Two bucket sets: selectors without a pseudo-element (the normal
/// match) and selectors with one (`::before`/`::after` lookups), since
/// each kind of match can only ever succeed for its own.
#[derive(Default)]
pub(crate) struct RuleBuckets {
    pub(crate) by_id: HashMap<String, Vec<usize>>,
    pub(crate) by_class: HashMap<String, Vec<usize>>,
    pub(crate) by_tag: HashMap<String, Vec<usize>>,
    pub(crate) universal: Vec<usize>,
}

#[derive(Default)]
pub(crate) struct RuleIndex {
    pub(crate) normal: RuleBuckets,
    pub(crate) pseudo: RuleBuckets,
}

impl RuleIndex {
    pub(crate) fn build(rules: &[StyleRule]) -> Self {
        let mut ix = RuleIndex::default();
        for (i, rule) in rules.iter().enumerate() {
            for (with_pseudo, buckets) in [(false, &mut ix.normal), (true, &mut ix.pseudo)] {
                let mut placed: Vec<(u8, String)> = Vec::new();
                let mut universal = false;
                let mut any = false;
                for sel in rule.selectors.slice() {
                    if sel.has_pseudo_element() != with_pseudo {
                        continue;
                    }
                    any = true;
                    let (mut id, mut class, mut tag) = (None, None, None);
                    for c in sel.iter() {
                        match c {
                            Component::ID(a) => id = Some(a.0.clone()),
                            Component::Class(a) => class = class.or_else(|| Some(a.0.clone())),
                            Component::LocalName(n) => tag = Some(n.lower_name.0.clone()),
                            _ => {}
                        }
                    }
                    let key = match (id, class, tag) {
                        (Some(id), _, _) => Some((0u8, id)),
                        (_, Some(c), _) => Some((1, c)),
                        (_, _, Some(t)) => Some((2, t)),
                        _ => None,
                    };
                    match key {
                        Some(k) => {
                            if !placed.contains(&k) {
                                placed.push(k);
                            }
                        }
                        None => universal = true,
                    }
                }
                if !any {
                    continue;
                }
                if universal {
                    buckets.universal.push(i);
                    continue;
                }
                for (kind, name) in placed {
                    let map = match kind {
                        0 => &mut buckets.by_id,
                        1 => &mut buckets.by_class,
                        _ => &mut buckets.by_tag,
                    };
                    map.entry(name).or_default().push(i);
                }
            }
        }
        ix
    }

    /// Sorted, deduplicated indices of rules that could match `handle`
    /// (for a pseudo-element lookup if `pseudo`).
    pub(crate) fn candidates(&self, handle: &Handle, pseudo: bool) -> Vec<usize> {
        let b = if pseudo { &self.pseudo } else { &self.normal };
        let mut out: Vec<usize> = b.universal.clone();
        if let NodeData::Element { name, .. } = &handle.data
            && let Some(v) = b.by_tag.get(&name.local.as_ref().to_ascii_lowercase())
        {
            out.extend_from_slice(v);
        }
        if let Some(id) = element_attr(handle, "id")
            && let Some(v) = b.by_id.get(&*id)
        {
            out.extend_from_slice(v);
        }
        if let Some(classes) = element_attr(handle, "class") {
            for c in classes.split_ascii_whitespace() {
                if let Some(v) = b.by_class.get(c) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// `(css source, parsed rules, STATE_* mask seen while parsing)`.
pub(crate) type ParsedRules = (String, Rc<Vec<StyleRule>>, u8);

/// Which elements a `:hover` rule can possibly restyle (R2.4): for every
/// selector compound containing `:hover`, the id / class / tag it
/// requires (or `any` when it requires none, or `:hover` is nested
/// somewhere this scan can't see). A hover change between elements none
/// of whose chain members could match such a compound can't change any
/// style, so it needs no restyle. Over-approximate by construction.
#[derive(Default, Clone)]
pub(crate) struct HoverKeys {
    pub(crate) any: bool,
    pub(crate) ids: std::collections::HashSet<String>,
    pub(crate) classes: std::collections::HashSet<String>,
    pub(crate) tags: std::collections::HashSet<String>,
}

impl HoverKeys {
    pub(crate) fn build(rules: &[StyleRule]) -> Self {
        let mut k = HoverKeys::default();
        for rule in rules {
            for sel in rule.selectors.slice() {
                // Debug/css text of the whole selector: counts every `:hover`,
                // including ones nested in `:not()`/`:is()` this scan can't see
                // (a class merely named "hover" only makes us more cautious).
                let total = format!("{sel:?}").to_ascii_lowercase().matches("hover").count();
                let mut found = 0;
                let mut it = sel.iter();
                loop {
                    let comps: Vec<_> = it.by_ref().collect();
                    let hover = comps.iter().filter(|c| matches!(c, Component::NonTSPseudoClass(OurPseudoClass::Hover))).count();
                    if hover > 0 {
                        found += hover;
                        let (mut id, mut class, mut tag) = (None, None, None);
                        for c in &comps {
                            match c {
                                Component::ID(a) => id = Some(a.0.clone()),
                                Component::Class(a) => class = class.or_else(|| Some(a.0.clone())),
                                Component::LocalName(n) => tag = Some(n.lower_name.0.clone()),
                                _ => {}
                            }
                        }
                        match (id, class, tag) {
                            (Some(i), _, _) => {
                                k.ids.insert(i);
                            }
                            (_, Some(c), _) => {
                                k.classes.insert(c);
                            }
                            (_, _, Some(t)) => {
                                k.tags.insert(t);
                            }
                            _ => k.any = true,
                        }
                    }
                    if it.next_sequence().is_none() {
                        break;
                    }
                }
                if found < total {
                    k.any = true;
                }
            }
        }
        k
    }

    pub(crate) fn may_match(&self, handle: &Handle) -> bool {
        if self.any {
            return true;
        }
        let NodeData::Element { name, .. } = &handle.data else { return false };
        if self.tags.contains(&name.local.as_ref().to_ascii_lowercase()) {
            return true;
        }
        if let Some(id) = element_attr(handle, "id")
            && self.ids.contains(&*id)
        {
            return true;
        }
        element_attr(handle, "class").is_some_and(|c| c.split_ascii_whitespace().any(|c| self.classes.contains(c)))
    }
}

thread_local! {
    /// `HoverKeys` of the most recent `build_layout_tree_with_resources`.
    pub(crate) static HOVER_KEYS: RefCell<HoverKeys> = RefCell::new(HoverKeys::default());
    /// Last parsed stylesheet: `(css source, rules, STATE_* mask seen while parsing)`.
    pub(crate) static RULES_CACHE: RefCell<Option<ParsedRules>> = const { RefCell::new(None) };
    /// `(rules.as_ptr(), rules.len(), index)` for the rule slice of the
    /// `build_layout_tree_with_resources` call in progress. Matched by
    /// slice identity, so any other caller falls back to a full scan.
    pub(crate) static RULE_INDEX: RefCell<Option<(usize, usize, RuleIndex)>> = const { RefCell::new(None) };
}

pub(crate) fn resolve_style(handle: &Handle, rules: &[StyleRule], inherited: InheritedStyle, viewport_width: f32) -> ComputedStyle {
    resolve_style_for(handle, rules, inherited, viewport_width, None)
}

/// `resolve_style`, optionally for the `::before`/`::after` pseudo-element
/// of `handle` (Q19): matches only that pseudo's rules and ignores the
/// element's inline `style`.
pub(crate) fn resolve_style_for(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    viewport_width: f32,
    pseudo: Option<&str>,
) -> ComputedStyle {
    let base_matches = matching_declarations_for(handle, rules, viewport_width, pseudo);
    // Q13: re-parse each matched rule's `var()`-bearing declarations with
    // this element's custom properties substituted, slotting the result
    // in right after the rule's own entries (so source order still breaks ties).
    let inline_src = if pseudo.is_some() { None } else { element_attr(handle, "style") };
    let mut inline_split = inline_src.as_ref().map(|s| parse_declarations_split(s));
    let rule_has_vars = base_matches.iter().any(|(_, d)| !d.var_decls.is_empty());
    let inline_has_vars = inline_split.as_ref().is_some_and(|(n, i)| !n.var_decls.is_empty() || !i.var_decls.is_empty());
    let has_custom = base_matches.iter().any(|(_, d)| !d.custom_props.is_empty())
        || inline_split.as_ref().is_some_and(|(n, _)| !n.custom_props.is_empty());
    let custom_props = if has_custom {
        compute_custom_props(&base_matches, inline_split.as_ref().map(|(n, _)| n), inherited.custom_props)
    } else {
        inherited.custom_props
    };
    let var_lookup = |n: &str| custom_props.and_then(|p| p.get(n).cloned());
    let synth = if rule_has_vars { expand_var_decls(&base_matches, &var_lookup) } else { Vec::new() };
    let mut matches = merge_var_matches(&base_matches, &synth);
    if inline_has_vars {
        let src = inline_src.as_ref().map(|s| s.to_string()).unwrap_or_default();
        inline_split = Some(parse_declarations_split(&substitute_css_vars(&src, &var_lookup).unwrap_or_default()));
    }
    let (inline_decls, inline_important) = match inline_split {
        Some((n, i)) => (Some(n), Some(i)),
        None => (None, None),
    };
    if let Some(i) = inline_important.as_ref() {
        matches.push((u32::MAX, i));
    }

    let inl = inline_decls.as_ref();
    let color = apply_wide(
        cascade_css(&matches, inl, "color", |d| d.color),
        true,
        inherited.color,
        ROOT_STYLE.color,
        |c| to_text_color(&c),
    );
    let background_color = apply_wide(
        cascade_css(&matches, inl, "background-color", |d| d.background_color),
        false,
        inherited.background_color,
        None,
        |c| Some(to_text_color(&c)),
    );
    let display = apply_wide(
        cascade_css(&matches, inl, "display", |d| d.display),
        false,
        inherited.display,
        Display::Inline,
        Some,
    );
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
    let display_declared = display;
    let list_item = matches!(
        cascade_css(&matches, inl, "display", |d| d.list_item),
        Cascaded::Value(true)
    ) && display == Display::Block;
    let flow_root = matches!(
        cascade_css(&matches, inl, "display", |d| d.flow_root),
        Cascaded::Value(true)
    ) && display == Display::Block;
    let table_caption = matches!(
        cascade_css(&matches, inl, "display", |d| d.table_caption),
        Cascaded::Value(true)
    ) && display == Display::Block;
    let caption_bottom = matches!(
        cascade_css(&matches, inl, "caption-side", |d| d.caption_bottom),
        Cascaded::Value(true)
    );
    let force_block = |display: Display, position: Position, float: Float| {
        // CSS2.1 §9.7: `table` stays `table` when floated/absolute (Q34c).
        if (matches!(position, Position::Absolute | Position::Fixed) || float != Float::None)
            && !matches!(display, Display::None | Display::Table)
        {
            Display::Block
        } else {
            display
        }
    };
    let display = force_block(display, position, float);
    // Resolved before margin/padding/border-width: an `em` on any of those
    // is relative to *this* element's own font-size (not its parent's),
    // so font-size has to be cascaded and resolved first. An `em` on
    // font-size itself is relative to the parent's (`inherited.font_size`)
    // — there's no self-reference, since font-size can only ever be `em`
    // relative to the value it's inheriting from, never its own result.
    let font_size = apply_wide(
        cascade_css(&matches, inl, "font-size", |d| d.font_size),
        true,
        inherited.font_size,
        ROOT_STYLE.font_size,
        |l| Some(l.resolve(inherited.font_size, inherited.root_font_size)),
    );
    let raw_width = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.width), |d| d.width);
    let width_kw = match raw_width {
        Some(Length::Intrinsic(k)) => Some(k),
        _ => None,
    };
    let width = raw_width
        .filter(|_| width_kw.is_none())
        .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size));
    let height = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.height), |d| d.height)
        .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size));
    let raw_min_width = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.min_width), |d| d.min_width);
    let min_width_kw = match raw_min_width {
        Some(Length::Intrinsic(k)) => Some(k),
        _ => None,
    };
    let min_width = raw_min_width
        .filter(|_| min_width_kw.is_none())
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
    // Q17: not inherited, but ACCUMULATED — own value times the parent's
    // accumulated one, so every paint can just scale its alpha by this.
    let opacity = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.opacity), |d| d.opacity)
        .unwrap_or(1.0)
        * inherited.parent.map_or(1.0, |p| p.opacity);
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
    let mut border_width = [0.0f32; 4];
    let mut border_color = [None; 4];
    let mut border_style = [BorderStyle::default(); 4];
    for i in 0..4 {
        border_style[i] = cascade_property(
            &matches,
            inline_decls.as_ref().and_then(|d| d.border_style[i]),
            |d| d.border_style[i],
        )
        .unwrap_or_default();
        border_width[i] = if border_style[i] == BorderStyle::None {
            0.0
        } else {
            cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.border_width[i]), |d| d.border_width[i])
                .map(|l| l.resolve(font_size, inherited.root_font_size))
                .unwrap_or(0.0)
        };
        border_color[i] = cascade_property(
            &matches,
            inline_decls.as_ref().and_then(|d| d.border_color[i]),
            |d| d.border_color[i],
        )
        .and_then(|c| to_text_color(&c));
    }
    let font_weight = apply_wide(
        cascade_css(&matches, inl, "font-weight", |d| d.font_weight),
        true,
        inherited.font_weight,
        ROOT_STYLE.font_weight,
        |v| Some(v.resolve(inherited.font_weight)),
    );
    let font_style = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.font_style),
        |d| d.font_style,
    )
    .unwrap_or(inherited.font_style);
    let font_family = apply_wide(
        cascade_css(&matches, inl, "font-family", |d| d.font_family),
        true,
        inherited.font_family,
        ROOT_STYLE.font_family,
        Some,
    );
    // Text-decoration properties aren't inherited, so `inherit` copies
    // the parent's own (cascaded) value, while `initial`/`unset`/
    // `revert` mean "not declared here" (the ambient decoration an
    // ancestor already draws is unaffected, as in real CSS).
    let deco_parent = inherited.parent.as_ref();
    let text_decoration = match cascade_css(&matches, inl, "text-decoration", |d| d.text_decoration) {
        Cascaded::Value(v) => Some(v),
        Cascaded::Wide(CssWide::Inherit) => deco_parent.and_then(|p| p.text_decoration),
        _ => None,
    };
    let text_decoration_line = match cascade_css(&matches, inl, "text-decoration-line", |d| d.text_decoration_line) {
        Cascaded::Value(v) => Some(v),
        Cascaded::Wide(CssWide::Inherit) => deco_parent.and_then(|p| p.text_decoration_line),
        _ => None,
    };
    let text_decoration_style = match cascade_css(&matches, inl, "text-decoration-style", |d| d.text_decoration_style) {
        Cascaded::Value(v) => Some(v),
        Cascaded::Wide(CssWide::Inherit) => deco_parent.and_then(|p| p.text_decoration_style),
        _ => None,
    };
    let text_decoration_color = match cascade_css(&matches, inl, "text-decoration-color", |d| d.text_decoration_color) {
        Cascaded::Value(c) => to_text_color(&c),
        Cascaded::Wide(CssWide::Inherit) => deco_parent.and_then(|p| p.text_decoration_color),
        _ => None,
    };
    let white_space = apply_wide(
        cascade_css(&matches, inl, "white-space", |d| d.white_space),
        true,
        inherited.white_space,
        ROOT_STYLE.white_space,
        Some,
    );
    let list_style_type = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.list_style_type),
        |d| d.list_style_type,
    )
    .unwrap_or(inherited.list_style_type);
    let text_align = apply_wide(
        cascade_css(&matches, inl, "text-align", |d| d.text_align),
        true,
        inherited.text_align,
        ROOT_STYLE.text_align,
        Some,
    );
    // Not inherited — no `inherited.vertical_align` to fall back to;
    // the initial value (`Baseline`, also `VerticalAlign::default()`)
    // applies to any element that doesn't declare it itself.
    let vertical_align = apply_wide(
        cascade_css(&matches, inl, "vertical-align", |d| d.vertical_align),
        false,
        inherited.vertical_align,
        VerticalAlign::Baseline,
        Some,
    );
    let line_height = apply_wide(
        cascade_css(&matches, inl, "line-height", |d| d.line_height),
        true,
        inherited.line_height,
        ROOT_STYLE.line_height,
        Some,
    );
    let visibility = apply_wide(
        cascade_css(&matches, inl, "visibility", |d| d.visibility),
        true,
        inherited.visibility,
        ROOT_STYLE.visibility,
        Some,
    );
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
    .map(|decl| match decl {
        // A real `none` at the winning specificity: no shadow regardless
        // of less-specific rules (see `ShadowDecl`).
        ShadowDecl::None => [None; MAX_SHADOWS],
        ShadowDecl::Shadow(list) => {
            let rl = |l: Length| l.resolve(font_size, inherited.root_font_size);
            let mut out = [None; MAX_SHADOWS];
            for (o, bs) in out.iter_mut().zip(list.iter()) {
                *o = bs.and_then(|bs| {
                    let c = match bs.color {
                        None | Some(CssColor::CurrentColor) => Some(color),
                        Some(c) => to_text_color(&c),
                    }?;
                    Some(ResolvedBoxShadow {
                        offset_x: rl(bs.offset_x),
                        offset_y: rl(bs.offset_y),
                        blur: rl(bs.blur).max(0.0),
                        spread: rl(bs.spread),
                        color: c,
                        inset: bs.inset,
                    })
                });
            }
            out
        }
    })
    .unwrap_or([None; MAX_SHADOWS]);
    let layout_for = |mask: bool| {
        let rl = |l: Length| l.resolve(font_size, inherited.root_font_size);
        let pos = apply_wide(
            cascade_css(&matches, inl, if mask { "mask-position" } else { "background-position" }, |d| if mask { d.mask_position } else { d.background_position }),
            false,
            BgPosDecl::default(),
            BgPosDecl::default(),
            Some,
        );
        let size = apply_wide(
            cascade_css(&matches, inl, if mask { "mask-size" } else { "background-size" }, |d| if mask { d.mask_size } else { d.background_size }),
            false,
            BgSizeDecl::default(),
            BgSizeDecl::default(),
            Some,
        );
        let repeat = apply_wide(
            cascade_css(&matches, inl, if mask { "mask-repeat" } else { "background-repeat" }, |d| if mask { d.mask_repeat } else { d.background_repeat }),
            false,
            BgRepeat::default(),
            BgRepeat::default(),
            Some,
        );
        let dim = |d: BgDimDecl| match d {
            BgDimDecl::Auto => BgDim::Auto,
            BgDimDecl::Len(Length::Percent(p)) => BgDim::Pct(p),
            BgDimDecl::Len(l) => BgDim::Px(rl(l)),
        };
        BgLayout {
            pos_x: (pos.x.pct, rl(pos.x.off)),
            pos_y: (pos.y.pct, rl(pos.y.off)),
            size: match size {
                BgSizeDecl::Auto => BgSize::Explicit(BgDim::Auto, BgDim::Auto),
                BgSizeDecl::Cover => BgSize::Cover,
                BgSizeDecl::Contain => BgSize::Contain,
                BgSizeDecl::Explicit(w, h) => BgSize::Explicit(dim(w), dim(h)),
            },
            repeat,
        }
    };
    let bg_layout = layout_for(false);
    let mask_layout = layout_for(true);
    let object_fit = apply_wide(
        cascade_css(&matches, inl, "object-fit", |d| d.object_fit),
        false,
        ObjectFit::Fill,
        ObjectFit::Fill,
        Some,
    );
    let object_pos = {
        let half = BgAxisDecl { pct: 0.5, off: Length::Px(0.0) };
        let def = BgPosDecl { x: half, y: half };
        let p = apply_wide(cascade_css(&matches, inl, "object-position", |d| d.object_position), false, def, def, Some);
        (
            (p.x.pct, p.x.off.resolve(font_size, inherited.root_font_size)),
            (p.y.pct, p.y.off.resolve(font_size, inherited.root_font_size)),
        )
    };
    let border_radius = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_radius),
        |d| d.border_radius,
    )
    .map(|l| BoxLength::from_length(l, font_size, inherited.root_font_size))
    .unwrap_or_default();
    let overflow_x = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.overflow), |d| d.overflow)
        .unwrap_or_default();
    let overflow_y = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.overflow_y), |d| d.overflow_y)
        .unwrap_or_default();
    // The style model has one clip/scroll mode per box, not one per axis,
    // so fold the two: a `visible` axis yields to the other one (real CSS
    // computes `visible` to `auto` there; the single-mode model treats
    // `hidden` as clipping both axes, the common `overflow-x:hidden` use).
    let overflow = if overflow_x == Overflow::Visible { overflow_y } else { overflow_x };
    let text_overflow = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.text_overflow), |d| d.text_overflow)
        .unwrap_or_default();
    let cursor = apply_wide(
        cascade_css(&matches, inl, "cursor", |d| d.cursor),
        true,
        inherited.cursor,
        ROOT_STYLE.cursor,
        Some,
    );
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
    let align_content = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.align_content),
        |d| d.align_content,
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
    let order = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.order), |d| d.order).unwrap_or(0);
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
    let grid_template_areas = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.grid_template_areas),
        |d| d.grid_template_areas,
    );
    let grid_area = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.grid_area), |d| d.grid_area);
    let grid_row_start = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.grid_row_start), |d| d.grid_row_start);
    let grid_row_end = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.grid_row_end), |d| d.grid_row_end);
    let grid_column_start =
        cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.grid_column_start), |d| d.grid_column_start);
    let grid_column_end =
        cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.grid_column_end), |d| d.grid_column_end);

    let mut cs = ComputedStyle {
        node: node_id(handle),
        custom_props,
        color,
        background_color,
        display,
        position,
        inset,
        z_index,
        opacity,
        float,
        clear,
        width,
        height,
        min_width,
        width_kw,
        min_width_kw,
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
        word_break: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.word_break), |d| d.word_break)
            .unwrap_or(inherited.word_break),
        text_transform: cascade_property(
            &matches,
            inline_decls.as_ref().and_then(|d| d.text_transform),
            |d| d.text_transform,
        )
        .unwrap_or(inherited.text_transform),
        list_style_type,
        list_item,
        flow_root,
        table_caption,
        caption_bottom,
        col_span: 1,
        row_span: 1,
        text_align,
        vertical_align,
        line_height,
        visibility,
        word_spacing,
        outline_width,
        outline_color,
        box_shadow,
        bg_layout,
        mask_layout,
        object_fit,
        object_pos,
        border_radius,
        overflow,
        text_overflow,
        icb_height: None,
        first_line: None,
        cursor,
        flex_direction,
        flex_wrap,
        justify_content,
        align_content,
        align_items,
        align_self,
        flex_grow,
        order,
        flex_shrink,
        flex_basis,
        row_gap,
        column_gap,
        grid_template_columns,
        grid_template_rows,
        grid_template_areas,
        grid_area,
        grid_row_start,
        grid_row_end,
        grid_column_start,
        grid_column_end,
    };
    // CSS-wide keywords on the remaining properties (`Q7b`).
    if matches.iter().any(|(_, d)| !d.wide.is_empty()) || inl.is_some_and(|d| !d.wide.is_empty()) {
        apply_rest_wide(&mut cs, &matches, inl, &inherited);
        cs.display = force_block(display_declared, cs.position, cs.float);
    }
    cs
}

pub(crate) fn anonymous_style(inherited: InheritedStyle) -> ComputedStyle {
    ComputedStyle {
        node: 0,
        custom_props: inherited.custom_props,
        color: inherited.color,
        background_color: None,
        display: Display::Block,
        position: Position::Static,
        inset: Insets::default(),
        z_index: None,
        opacity: 1.0,
        float: Float::default(),
        clear: Clear::default(),
        width: None,
        height: None,
        min_width: None,
        width_kw: None,
        min_width_kw: None,
        max_width: None,
        min_height: None,
        max_height: None,
        box_sizing: BoxSizing::default(),
        margin: Sides::default(),
        padding: Sides::default(),
        border_width: [0.0; 4],
        border_color: [None; 4],
        border_style: [BorderStyle::default(); 4],
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
        word_break: inherited.word_break,
        text_transform: inherited.text_transform,
        list_style_type: inherited.list_style_type,
        list_item: false,
        flow_root: false,
        table_caption: false,
        caption_bottom: false,
        col_span: 1,
        row_span: 1,
        text_align: inherited.text_align,
        vertical_align: VerticalAlign::default(),
        line_height: inherited.line_height,
        visibility: inherited.visibility,
        word_spacing: inherited.word_spacing,
        outline_width: 0.0,
        outline_color: None,
        box_shadow: [None; MAX_SHADOWS],
        bg_layout: BgLayout::default(),
        mask_layout: BgLayout::default(),
        object_fit: ObjectFit::Fill,
        object_pos: ((0.5, 0.0), (0.5, 0.0)),
        border_radius: BoxLength::default(),
        overflow: Overflow::default(),
        text_overflow: TextOverflow::default(),
        icb_height: None,
        first_line: None,
        cursor: inherited.cursor,
        flex_direction: FlexDirection::default(),
        flex_wrap: FlexWrap::default(),
        justify_content: JustifyContent::default(),
        align_content: AlignContent::default(),
        align_items: AlignItems::default(),
        align_self: None,
        flex_grow: 0.0,
        order: 0,
        flex_shrink: 1.0,
        flex_basis: FlexBasis::default(),
        row_gap: 0.0,
        column_gap: 0.0,
        grid_template_columns: None,
        grid_template_rows: None,
        grid_template_areas: None,
        grid_area: None,
        grid_row_start: None,
        grid_row_end: None,
        grid_column_start: None,
        grid_column_end: None,
    }
}

pub(crate) fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
