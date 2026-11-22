use super::*;

// ---------- cascade ----------

/// QCSS-A: the SVG paint properties, in `Declarations::svg` order. All inherit.
pub(crate) const SVG_KEYS: [&str; 7] =
    ["fill", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin", "fill-opacity", "stroke-opacity"];

/// QCSS-A: the computed SVG paint properties of an element: per key the canonical text (see
/// `css::parse_svg_prop`; `None` = the initial value), and `decl`: bit i set when `SVG_KEYS[i]` was
/// declared by CSS on THIS element (presentation attributes lose to it; not inherited). Interned
/// (one leaked copy per distinct value, so `ComputedStyle` stays `Copy` and equal values are cheap).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct SvgProps {
    pub(crate) vals: [Option<&'static str>; 7],
    pub(crate) decl: u8,
}
pub(crate) type SvgRef = &'static SvgProps;

type SvgIntern = (HashMap<SvgProps, SvgRef>, HashMap<String, &'static str>);
thread_local! {
    static SVG_INTERN: std::cell::RefCell<SvgIntern> = std::cell::RefCell::new((HashMap::new(), HashMap::new()));
}

pub(crate) fn intern_svg(p: SvgProps) -> SvgRef {
    SVG_INTERN.with(|t| {
        let mut t = t.borrow_mut();
        if let Some(r) = t.0.get(&p) {
            return *r;
        }
        let r: SvgRef = Box::leak(Box::new(p));
        t.0.insert(p, r);
        r
    })
}

fn intern_svg_str(s: &str) -> &'static str {
    SVG_INTERN.with(|t| {
        let mut t = t.borrow_mut();
        if let Some(r) = t.1.get(s) {
            return *r;
        }
        let r: &'static str = Box::leak(s.to_string().into_boxed_str());
        t.1.insert(s.to_string(), r);
        r
    })
}

/// What a child inherits of `s`: the values, no `decl` bits.
pub(crate) fn svg_inherited(s: Option<SvgRef>) -> Option<SvgRef> {
    s.map(|p| if p.decl == 0 { p } else { intern_svg(SvgProps { decl: 0, ..*p }) })
}

fn decls_have_svg(d: &Declarations) -> bool {
    d.svg.iter().any(Option::is_some) || d.wide.iter().any(|(k, _)| SVG_KEYS.contains(k))
}

/// QCSS-A: cascades the SVG paint properties (all inherited). Nothing declared (the overwhelmingly
/// common case): the parent's values.
fn cascade_svg(matches: &[(u64, &Declarations)], inl: Option<&Declarations>, parent: Option<SvgRef>) -> Option<SvgRef> {
    if !matches.iter().any(|(_, d)| decls_have_svg(d)) && !inl.is_some_and(decls_have_svg) {
        return parent;
    }
    let mut out = parent.copied_or_default();
    out.decl = 0;
    for (i, key) in SVG_KEYS.iter().enumerate() {
        match cascade_css(matches, inl, key, |d| d.svg[i].clone()) {
            Cascaded::Value(v) => {
                out.vals[i] = Some(intern_svg_str(&v));
                out.decl |= 1 << i;
            }
            Cascaded::Wide(kw) => {
                if !kw.takes_parent(true) {
                    out.vals[i] = None;
                }
                out.decl |= 1 << i;
            }
            Cascaded::Unset => {}
        }
    }
    if out.vals.iter().all(Option::is_none) && out.decl == 0 {
        return None;
    }
    Some(intern_svg(out))
}

trait SvgOrDefault {
    fn copied_or_default(self) -> SvgProps;
}
impl SvgOrDefault for Option<SvgRef> {
    fn copied_or_default(self) -> SvgProps {
        self.copied().unwrap_or(SvgProps { vals: [None; 7], decl: 0 })
    }
}

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
    /// QCSS-E: `letter-spacing` (inherited), px. Applied as `Attrs::letter_spacing` (cosmic-text
    /// takes it in em, so the span's own font size divides it out; see `shape_spans`).
    pub(crate) letter_spacing: f32,
    /// QCSS-E: `direction` (inherited): the inline base direction.
    pub(crate) direction: Direction,
    /// QCSS-E: `border-spacing` (inherited): horizontal, vertical px between table cells.
    pub(crate) border_spacing: (f32, f32),
    /// `border-collapse: collapse` (inherited). Only used to decide that a
    /// `tr`'s own border is ignored in the separated model (Q52c1); there is
    /// no real collapsed-border layout.
    pub(crate) border_collapse: bool,
    /// `text-indent` (QW2), inherited, eagerly resolved to px. Only a
    /// negative value is honoured (painted as a first-line x shift, see
    /// `paint_buffer`); positive indents don't narrow the first line yet.
    pub(crate) text_indent: f32,
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
    pub(crate) custom_props: Option<CustomPropsRef>,
    /// QCSS-A: the parent's SVG paint properties (inherited; `decl` bits cleared).
    pub(crate) svg: Option<SvgRef>,
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
    text_align: TextAlign::Start,
    line_height: LineHeight::Normal,
    visibility: Visibility::Visible,
    cursor: CursorStyle::Auto,
    word_spacing: 0.0,
    letter_spacing: 0.0,
    direction: Direction::Ltr,
    border_spacing: (0.0, 0.0),
    border_collapse: false,
    text_indent: 0.0,
    background_color: None,
    display: Display::Inline,
    vertical_align: VerticalAlign::Baseline,
    parent: None,
    custom_props: None,
    svg: None,
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
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Sides {
    pub(crate) top: BoxLength,
    pub(crate) right: BoxLength,
    pub(crate) bottom: BoxLength,
    pub(crate) left: BoxLength,
}

/// The properties a `::first-line` rule actually changed relative to the
/// element's own style (each `None` means "leave the span alone").
#[derive(Debug, Clone, Copy, Default, PartialEq)]
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

/// QCSS-E: the initial `transform-origin`, `50% 50%`.
pub(crate) const INITIAL_TRANSFORM_ORIGIN: (TranslateAxis, TranslateAxis) =
    (TranslateAxis { px: 0.0, frac: 0.5 }, TranslateAxis { px: 0.0, frac: 0.5 });

/// One axis of a QY18 translate: `px` plus a fraction of the box's own size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TranslateAxis {
    pub(crate) px: f32,
    pub(crate) frac: f32,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct ComputedStyle {
    /// Q31: `node_id` of the element this style was resolved for (0 for
    /// anonymous boxes). Lets a hit test name the element under the mouse.
    pub(crate) node: usize,
    /// QY53n2: which pseudo-element of `node` this is the style of (the
    /// `pseudo_slot`: 0 = the element itself, 1 `::before`, 2 `::after`, ...).
    pub(crate) pseudo: u8,
    /// QY1: creation sequence number, ascending in the order styles are
    /// computed (parent before children, siblings in document order), so
    /// it is a tree-order key. Used as the tie-break when positioned boxes
    /// in one stacking context share a `z-index`. 0 for anonymous boxes.
    pub(crate) tree_order: usize,
    /// This element's resolved custom properties (Q13): inherited from
    /// the parent, overlaid with its own `--name` declarations, every
    /// value already `var()`-substituted. Interned, see `intern_custom_props`.
    pub(crate) custom_props: Option<CustomPropsRef>,
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
    /// QY18: summed translate-only `transform`, applied as a paint/hit-test
    /// offset (`apply_transform_offset`); no layout effect.
    pub(crate) translate: Option<(TranslateAxis, TranslateAxis)>,
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
    /// QY42: `max-width` was an intrinsic keyword (`fit-content` etc.).
    pub(crate) max_width_intrinsic: bool,
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
    /// (left, right) margin is `auto` (Q59p); `margin` holds 0 for it until
    /// layout resolves the free space.
    pub(crate) margin_auto: (bool, bool),
    /// (top, bottom) margin is `auto` (QB4): only used to centre an out-of-flow box between top/bottom insets.
    pub(crate) margin_auto_v: (bool, bool),
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
    /// QCSS-A: inherited SVG paint properties (`None`: all initial, nothing declared anywhere above).
    pub(crate) svg: Option<SvgRef>,
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
    /// QCSS-E: inherited, see `InheritedStyle`.
    pub(crate) letter_spacing: f32,
    pub(crate) direction: Direction,
    pub(crate) border_spacing: (f32, f32),
    /// QCSS-E: `outline-offset` px (not inherited; negative moves the outline inside the border
    /// box). The outline's paint reach is `outline_width + max(outline_offset, 0)`.
    pub(crate) outline_offset: f32,
    /// QCSS-E: `transform-origin` x, y as px plus a fraction of the border box. Initial `50% 50%`.
    /// The only transform this engine applies is a translate, which does not depend on the
    /// origin; `transform_origin_px` is what a scale or rotate would read.
    pub(crate) transform_origin: (TranslateAxis, TranslateAxis),
    /// Inherited `border-collapse: collapse` (see `InheritedStyle`).
    pub(crate) border_collapse: bool,
    /// Inherited `text-indent` in px (see `InheritedStyle::text_indent`).
    pub(crate) text_indent: f32,
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
    /// QCSS-D: `filter` / `backdrop-filter` (`None` = none), `clip-path`, `mix-blend-mode`: paint
    /// only, not inherited. `text-shadow` is inherited. Lists are interned (`fx.rs`).
    pub(crate) filter: Option<&'static [FilterCompFn]>,
    pub(crate) backdrop_filter: Option<&'static [FilterCompFn]>,
    pub(crate) text_shadow: Option<&'static [ShadowCompFn]>,
    pub(crate) clip_path: Option<&'static Shape<LP>>,
    pub(crate) mix_blend_mode: BlendMode,
    /// `background-position`/`-size`/`-repeat` (Q16), applied by
    /// `paint_background_image`. Not inherited.
    pub(crate) bg_layout: BgLayout,
    /// QW6: 1 for the `<html>` element's box, 2 for `<body>`'s, else 0.
    /// Lets `build_layout_tree_with_resources` propagate the root/body
    /// background to the canvas (CSS Backgrounds 3 2.11.2). Not inherited.
    pub(crate) canvas_src: u8,
    /// QYSCR1b: on the document root box only: the overflow propagated to the viewport is `hidden`
    /// (user scrolling blocked, script scrolling still works). Not inherited.
    pub(crate) viewport_hidden: bool,
    /// QXSCR1: the same for the x axis (the propagated `overflow-x` is `hidden`).
    pub(crate) viewport_hidden_x: bool,
    /// `mask-position`/`-size`/`-repeat` (Q29), same shape as `bg_layout`.
    pub(crate) mask_layout: BgLayout,
    /// `object-fit` and `object-position` (Q28), applied by `blit_image`.
    /// Position is `(fraction of free space, px)` per axis, like
    /// `bg_layout`. Not inherited.
    pub(crate) object_fit: ObjectFit,
    pub(crate) object_pos: ((f32, f32), (f32, f32)),
    /// QCSS-B: `aspect-ratio` (not inherited).
    pub(crate) aspect_ratio: AspectRatio,
    /// Rounds both the background fill and (for `BorderStyle::Solid`
    /// only — see `fill_rounded_ring`'s own docs) the border itself, via
    /// `paint_box_decoration`. Not inherited. `BoxLength`, not a plain
    /// `f32`: real CSS resolves a `%` border-radius against the box's
    /// *own* border-box width (unlike `width`/`margin`/`padding`, whose
    /// basis is the *containing block's* width) — `paint_box_decoration`
    /// resolves it directly against its own `w` parameter, since that's
    /// already the exact value needed and it's never used anywhere
    /// `measure` would otherwise have to finalize it early.
    pub(crate) border_radius: CornerRadii,
    /// Not inherited. See `Overflow`'s own docs for the clip's scope. QXSCR1: one value per axis,
    /// already fixed up per CSS Overflow 3 (`overflow_axes`): when either axis is a scrolling value
    /// (`hidden`/`scroll`/`auto`), a `visible` other axis is `auto` and a `clip` one is `hidden`.
    pub(crate) overflow_x: Overflow,
    pub(crate) overflow_y: Overflow,
    /// `text-overflow` (Q21). Not inherited; `build_box_for_element` copies
    /// an `Ellipsis` onto the anonymous inline runs of a clipping box.
    pub(crate) text_overflow: TextOverflow,
    /// QAN1a: the interned animation lists (parse only, not evaluated yet).
    pub(crate) animations: Option<AnimRef>,
    /// QANIMHLb: the interned transition lists.
    pub(crate) transitions: Option<TransRef>,
    /// `appearance: none` (Q30a): form controls keep their text but get no
    /// widget decorations (check mark, select arrow, bar fill). Not inherited.
    pub(crate) appearance_none: bool,
    /// QYSCR1: `contain` includes `layout` or `paint` (also via `strict`/`content`).
    /// Only read on `<html>`/`<body>`: such containment stops `overflow`
    /// propagation to the viewport (CSS Overflow 3). Not inherited.
    pub(crate) contain_lp: bool,
    /// Q30c: `CHECK_*` bits of a checkbox/radio box (the painter draws its mark
    /// from the live state), 0 otherwise. Not inherited.
    pub(crate) check_widget: u8,
    /// Q30d: `TEXT_*` bits of an editable `<input>`/`<textarea>` box; the painter
    /// draws its live value, caret and selection (forms_text.rs). Not inherited.
    pub(crate) text_widget: u8,
    /// Q30g: `SEL_*` bits of a `<select>` box; the painter draws the live selection
    /// (forms_select.rs). Not inherited.
    pub(crate) select_widget: u8,
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
    /// QCSS-C: grid alignment of items on the inline axis; `justify_self` `None` is `auto`.
    pub(crate) justify_items: AlignItems,
    pub(crate) justify_self: Option<AlignItems>,
    pub(crate) grid_auto_flow: GridAutoFlow,
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
    pub(crate) grid_auto_columns: Option<&'static [GridTrackSpec]>,
    pub(crate) grid_auto_rows: Option<&'static [GridTrackSpec]>,
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
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// QXSCR1: some axis clips (anything but `visible`).
    pub(crate) fn clips_any(&self) -> bool {
        self.overflow_x != Overflow::Visible || self.overflow_y != Overflow::Visible
    }

    /// QXSCR1: the box is a scroll container (`hidden`/`scroll`/`auto` on an axis; after
    /// `overflow_axes` the other axis then is one of those too).
    pub(crate) fn scroll_container(&self) -> bool {
        matches!(self.overflow_x, Overflow::Hidden | Overflow::Scroll | Overflow::Auto)
            || matches!(self.overflow_y, Overflow::Hidden | Overflow::Scroll | Overflow::Auto)
    }

    /// QXSCR1: sets both axes (UA boxes that clip, e.g. form controls).
    pub(crate) fn set_overflow(&mut self, o: Overflow) {
        self.overflow_x = o;
        self.overflow_y = o;
    }

    pub(crate) fn establishes_bfc(&self) -> bool {
        self.float != Float::None
            || matches!(self.position, Position::Absolute | Position::Fixed)
            || self.clips_any()
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

    /// QCSS-E: how far (px) the outline paints past the border box: `outline-offset` plus
    /// `outline-width`, 0 without a visible outline or when a negative offset pulls it inside.
    /// Anything that bounds what a box paints (the damage-rect repaint's `paint_reach`) must
    /// include this.
    pub(crate) fn outline_reach(&self) -> f32 {
        if self.outline_width > 0.0 && self.outline_color.is_some() {
            (self.outline_width + self.outline_offset).max(0.0)
        } else {
            0.0
        }
    }

    /// QCSS-E: `transform-origin` in px for a `w` x `h` border box (what a scale or rotate would
    /// pivot on; the translate-only transform this engine paints does not read it).
    pub(crate) fn transform_origin_px(&self, w: f32, h: f32) -> (f32, f32) {
        let (x, y) = self.transform_origin;
        (x.px + x.frac * w, y.px + y.frac * h)
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
            letter_spacing: self.letter_spacing,
            direction: self.direction,
            border_spacing: self.border_spacing,
            border_collapse: self.border_collapse,
            text_indent: self.text_indent,
            background_color: self.background_color,
            display: self.display,
            vertical_align: self.vertical_align,
            parent: Some(*self),
            custom_props: self.custom_props,
            svg: svg_inherited(self.svg),
        }
    }
}


pub(crate) fn cascade_property<T: Clone>(
    matches: &[(u64, &Declarations)],
    inline: Option<T>,
    get: impl Fn(&Declarations) -> Option<T>,
) -> Option<T> {
    let mut best: Option<(u64, T)> = None;
    for &(specificity, decls) in matches {
        if let Some(v) = get(decls) {
            if best.as_ref().is_none_or(|(s, _)| specificity >= *s) {
                best = Some((specificity, v));
            }
        }
    }
    match best {
        // An `!important` declaration (rule or inline) beats an inline
        // normal one.
        Some((s, v)) if s >= IMPORTANT_TIER || s == ANIM_KEY => Some(v),
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

/// QCSS-E: the canonical keys of the four corner radii (top-left, top-right, bottom-right,
/// bottom-left), the order of `Declarations::border_radius`.
pub(crate) static CORNER_KEYS: [&str; 4] =
    ["border-top-left-radius", "border-top-right-radius", "border-bottom-right-radius", "border-bottom-left-radius"];

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
        "outline-color", "box-shadow", "filter", "backdrop-filter", "text-shadow", "mix-blend-mode", "clip-path",
        "outline-offset", "transform-origin", "border-spacing",
        "letter-spacing", "direction",
        "overflow-y", "text-overflow", "flex-direction", "flex-wrap", "justify-content", "align-items",
        "align-content", "justify-items", "justify-self", "grid-auto-flow", "align-self", "order", "flex-grow", "flex-shrink", "flex-basis", "row-gap",
        "column-gap", "grid-template-columns", "grid-template-rows", "grid-auto-columns", "grid-auto-rows",
        "grid-template-areas", "grid-area", "font-style", "list-style-type",
        "fill", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin", "fill-opacity", "stroke-opacity",
        "word-spacing", "animation-name", "animation-duration", "animation-timing-function",
        "animation-delay", "animation-iteration-count", "animation-direction",
        "animation-fill-mode", "animation-play-state", "aspect-ratio",
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
    if let Some(k) = CORNER_KEYS.iter().position(|k| *k == l) {
        return Some(std::slice::from_ref(&CORNER_KEYS[k]));
    }
    Some(match l {
        // QXSCR1: the canonical key `overflow` is the x axis.
        "overflow" => &["overflow", "overflow-y"],
        "overflow-x" => &["overflow"],
        "animation" => &ANIMATION_KEYS,
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
        "border-radius" => &CORNER_KEYS,
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
        "gap" | "grid-gap" => &["row-gap", "column-gap"],
        "grid-row-gap" => &["row-gap"],
        "grid-column-gap" => &["column-gap"],
        "place-content" => &["align-content", "justify-content"],
        "place-items" => &["align-items", "justify-items"],
        "place-self" => &["align-self", "justify-self"],
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
            | "word-spacing" | "text-shadow" | "letter-spacing" | "direction" | "border-spacing"
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
            // An unset colour is `currentColor`; `inherit`/`initial` of it
            // resolve against this element's own colour (border-color-011).
            _ => dst.border_color[i] = src.border_color[i].or(Some(dst.color)),
        }
        return;
    }
    if let Some(i) = CORNER_KEYS.iter().position(|k| *k == key) {
        dst.border_radius.0[i] = src.border_radius.0[i];
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
        "filter" => dst.filter = src.filter,
        "backdrop-filter" => dst.backdrop_filter = src.backdrop_filter,
        "text-shadow" => dst.text_shadow = src.text_shadow,
        "clip-path" => dst.clip_path = src.clip_path,
        "mix-blend-mode" => dst.mix_blend_mode = src.mix_blend_mode,
        "outline-offset" => dst.outline_offset = src.outline_offset,
        "transform-origin" => dst.transform_origin = src.transform_origin,
        "border-spacing" => dst.border_spacing = src.border_spacing,
        "letter-spacing" => dst.letter_spacing = src.letter_spacing,
        "direction" => dst.direction = src.direction,
        "overflow" => dst.overflow_x = src.overflow_x,
        "overflow-y" => dst.overflow_y = src.overflow_y,
        "text-overflow" => dst.text_overflow = src.text_overflow,
        "flex-direction" => dst.flex_direction = src.flex_direction,
        "flex-wrap" => dst.flex_wrap = src.flex_wrap,
        "justify-content" => dst.justify_content = src.justify_content,
        "align-content" => dst.align_content = src.align_content,
        "align-items" => dst.align_items = src.align_items,
        "align-self" => dst.align_self = src.align_self,
        "justify-items" => dst.justify_items = src.justify_items,
        "justify-self" => dst.justify_self = src.justify_self,
        "grid-auto-flow" => dst.grid_auto_flow = src.grid_auto_flow,
        "order" => dst.order = src.order,
        "flex-grow" => dst.flex_grow = src.flex_grow,
        "flex-shrink" => dst.flex_shrink = src.flex_shrink,
        "flex-basis" => dst.flex_basis = src.flex_basis,
        "row-gap" => dst.row_gap = src.row_gap,
        "column-gap" => dst.column_gap = src.column_gap,
        "grid-template-columns" => dst.grid_template_columns = src.grid_template_columns,
        "grid-template-rows" => dst.grid_template_rows = src.grid_template_rows,
        "grid-auto-columns" => dst.grid_auto_columns = src.grid_auto_columns,
        "grid-auto-rows" => dst.grid_auto_rows = src.grid_auto_rows,
        "grid-template-areas" => dst.grid_template_areas = src.grid_template_areas,
        "grid-area" => dst.grid_area = src.grid_area,
        "font-style" => dst.font_style = src.font_style,
        "list-style-type" => dst.list_style_type = src.list_style_type,
        "word-spacing" => dst.word_spacing = src.word_spacing,
        "text-indent" => dst.text_indent = src.text_indent,
        k if k.starts_with("animation-") => copy_animation_longhand(&mut dst.animations, src.animations, k),
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
    matches: &[(u64, &Declarations)],
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
    matches: &[(u64, &Declarations)],
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
    let mut best: Option<(u64, Cascaded<T>)> = None;
    for &(specificity, decls) in matches {
        if let Some(c) = cand(decls)
            && best.as_ref().is_none_or(|(s, _)| specificity >= *s)
        {
            best = Some((specificity, c));
        }
    }
    let inline = inline.and_then(cand);
    match best {
        Some((s, c)) if s >= IMPORTANT_TIER || s == ANIM_KEY => c,
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

/// Cascade sort key (QY17): bit 62 marks the `!important` tier, bits
/// 32..61 hold the layer rank, bits 0..31 the specificity (30 bits).
/// Compared as one integer, later source order wins ties (`>=`).
pub(crate) const IMPORTANT_TIER: u64 = 1 << 62;
/// QAN1b: the cascade key of animation (keyframe) declarations: above every
/// normal declaration (inline included), below every `!important` one.
pub(crate) const ANIM_KEY: u64 = IMPORTANT_TIER - 1;
/// Rank of unlayered author rules for normal declarations (the highest).
const LAYER_MAX: u64 = (1 << 29) - 1;

/// Key for a normal declaration of a rule in `layer` (`StyleRule::layer`:
/// 0 unlayered, 1 user-agent, n >= 2 author layer): unlayered author rules
/// beat every layer, later layers beat earlier ones, the UA is lowest.
pub(crate) fn normal_key(spec: u32, layer: u32) -> u64 {
    let rank = if layer == 0 { LAYER_MAX } else { layer as u64 };
    (rank << 32) | spec as u64
}

/// Key for an `!important` declaration: the layer order is reversed
/// (UA first, then earlier layers, unlayered last; CSS Cascade 5 §6.4).
pub(crate) fn important_key(spec: u32, layer: u32) -> u64 {
    to_important(normal_key(spec, layer))
}

/// The `!important` key of a rule whose normal key is `k`.
pub(crate) fn to_important(k: u64) -> u64 {
    if k & IMPORTANT_TIER != 0 {
        return k;
    }
    IMPORTANT_TIER | ((LAYER_MAX - (k >> 32)) << 32) | (k & 0xFFFF_FFFF)
}

/// Every rule in `rules` whose selector matches `handle`, paired with its
/// specificity — the real selector-matching work `resolve_style` itself
/// needs, factored out so `resolve_background_image` can reuse it
/// (instead of re-running `matches_selector_list` over every rule a
/// second time) rather than folding background-image into
/// `ComputedStyle` itself — see `resolve_background_image`'s own docs
/// for why it's resolved separately at all.
pub(crate) fn matching_declarations<'a>(handle: &Handle, rules: &'a [StyleRule], viewport_width: f32) -> Vec<(u64, &'a Declarations)> {
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
) -> Vec<(u64, &'a Declarations)> {
    // QY53d: inside one layout build every (element, pseudo) is matched once;
    // the later passes (counters, background image, the cascade itself) reuse
    // the result. The memo holds rule indices, so it never outlives the rules.
    let memo_key = pseudo_slot(pseudo).map(|s| (Rc::as_ptr(handle) as usize, s, rules.as_ptr() as usize, rules.len(), viewport_width.to_bits()));
    let resolve = |found: &[(u64, u32, bool)]| -> Vec<(u64, &'a Declarations)> {
        found
            .iter()
            .map(|&(key, i, imp)| (key, if imp { rules[i as usize].important() } else { rules[i as usize].decls() }))
            .collect()
    };
    if let Some(k) = memo_key
        && let Some(hit) = MATCH_MEMO.with(|m| m.borrow().as_ref().and_then(|m| m.get(&k).cloned()))
    {
        return resolve(&hit);
    }
    // QINC1g: the incremental box rebuild reads the element table's stored match.
    if let Some(k) = memo_key
        && let Some(found) = crate::incr_box::stored_match(handle, k.1)
    {
        MATCH_MEMO.with(|m| {
            if let Some(m) = m.borrow_mut().as_mut() {
                m.insert(k, found.clone());
            }
        });
        return resolve(&found);
    }
    #[cfg(test)]
    MATCH_RUNS.with(|c| c.set(c.get() + 1));
    let found = matching_indices_for(handle, rules, viewport_width, pseudo);
    let out = resolve(&found);
    if let Some(k) = memo_key {
        MATCH_MEMO.with(|m| {
            if let Some(m) = m.borrow_mut().as_mut() {
                m.insert(k, Rc::new(found));
            }
        });
    }
    out
}

/// Memo slot of the pseudo-elements the build matches; `None` = never memoized.
pub(crate) fn pseudo_slot(pseudo: Option<&str>) -> Option<u8> {
    match pseudo {
        None => Some(0),
        Some("before") => Some(1),
        Some("after") => Some(2),
        Some("first-line") => Some(3),
        Some("first-letter") => Some(4),
        Some("marker") => Some(5),
        Some(_) => None,
    }
}

/// One (element, pseudo) match: `(cascade key, rule index, is the important half)`.
pub(crate) type MatchList = Rc<Vec<(u64, u32, bool)>>;

pub(crate) type MatchMemo = HashMap<(usize, u8, usize, usize, u32), MatchList>;

/// QINC1f: the pseudo-element name of memo slot `slot` (inverse of `pseudo_slot`).
pub(crate) fn slot_pseudo(slot: u8) -> Option<&'static str> {
    match slot {
        1 => Some("before"),
        2 => Some("after"),
        3 => Some("first-line"),
        4 => Some("first-letter"),
        5 => Some("marker"),
        _ => None,
    }
}

/// QINC1f: the match of `(handle, slot)` through the build memo (which must be active): a memo hit, or
/// a real match that is then memoized. The incremental restyle stores the result per element.
pub(crate) fn match_slot(handle: &Handle, rules: &[StyleRule], viewport_width: f32, slot: u8) -> MatchList {
    let key = (Rc::as_ptr(handle) as usize, slot, rules.as_ptr() as usize, rules.len(), viewport_width.to_bits());
    if let Some(hit) = MATCH_MEMO.with(|m| m.borrow().as_ref().and_then(|m| m.get(&key).cloned())) {
        return hit;
    }
    if let Some(found) = crate::incr_box::stored_match(handle, slot) {
        return found;
    }
    #[cfg(test)]
    MATCH_RUNS.with(|c| c.set(c.get() + 1));
    let found = Rc::new(matching_indices_for(handle, rules, viewport_width, slot_pseudo(slot)));
    MATCH_MEMO.with(|m| {
        if let Some(m) = m.borrow_mut().as_mut() {
            m.insert(key, found.clone());
        }
    });
    found
}

/// QINC1f: puts a stored match into the build memo, so the cascade of `(handle, slot)` reuses it
/// instead of matching (a re-cascade from the element table).
pub(crate) fn seed_match(handle: &Handle, rules: &[StyleRule], viewport_width: f32, slot: u8, found: MatchList) {
    let key = (Rc::as_ptr(handle) as usize, slot, rules.as_ptr() as usize, rules.len(), viewport_width.to_bits());
    MATCH_MEMO.with(|m| {
        if let Some(m) = m.borrow_mut().as_mut() {
            m.insert(key, found);
        }
    });
}

thread_local! {
    /// QY53d: `Some` only while `build_layout_tree_with_resources` runs.
    pub(crate) static MATCH_MEMO: RefCell<Option<MatchMemo>> = const { RefCell::new(None) };
    /// QAPI6b: `Some` only while a build (or the engine's update) runs, with `CLASS_CACHE`: the DOM does not
    /// change meanwhile, so the selector caches of one match are valid for the next.
    pub(crate) static SEL_CACHES: RefCell<Option<SelectorCaches>> = const { RefCell::new(None) };
    /// QY53g: `Some` only while `build_layout_tree_with_resources` runs.
    pub(crate) static ANCESTOR_BLOOM: RefCell<Option<AncestorBloom>> = const { RefCell::new(None) };
    /// Test-only: how many real selector-matching passes ran.
    #[cfg(test)]
    pub(crate) static MATCH_RUNS: Cell<usize> = const { Cell::new(0) };
}

/// The real matching work: `(key, rule index, is the important half)`.
fn matching_indices_for(
    handle: &Handle,
    rules: &[StyleRule],
    viewport_width: f32,
    pseudo: Option<&str>,
) -> Vec<(u64, u32, bool)> {
    // QAPI6b: inside a build (the DOM does not change) every match shares one `SelectorCaches`: `:has()`
    // results per (anchor, relative selector) and `:nth-*` indices are computed once per build, not per element.
    let shared = SEL_CACHES.with(|c| c.try_borrow_mut().ok().and_then(|mut c| c.take()));
    match shared {
        Some(mut caches) => {
            let r = matching_indices_with(handle, rules, viewport_width, pseudo, &mut caches);
            SEL_CACHES.with(|c| {
                if let Ok(mut c) = c.try_borrow_mut() {
                    *c = Some(caches);
                }
            });
            r
        }
        None => matching_indices_with(handle, rules, viewport_width, pseudo, &mut SelectorCaches::default()),
    }
}

fn matching_indices_with(
    handle: &Handle,
    rules: &[StyleRule],
    viewport_width: f32,
    pseudo: Option<&str>,
    caches: &mut SelectorCaches,
) -> Vec<(u64, u32, bool)> {
    let element = El(handle.clone());
    let mut matches: Vec<(u64, u32, bool)> = Vec::new();
    // R3.3: when an index was registered for exactly this rule slice,
    // only test the rules whose rightmost compound could match this
    // element (ascending rule order preserved, so cascade ties are
    // unchanged); otherwise test every rule.
    let candidates = RULE_INDEX.with(|ri| {
        let ri = ri.borrow();
        let (ptr, len, idx) = ri.as_ref()?;
        (*ptr == rules.as_ptr() as usize && *len == rules.len())
            .then(|| (idx.candidates(handle, pseudo.is_some()), idx.ancestor_hashes.clone()))
    });
    let (order, hashes): (Vec<usize>, _) = match candidates {
        Some((c, h)) => (c, Some(h)),
        None => ((0..rules.len()).collect(), None),
    };
    // QY53g: the ancestor bloom filter (only inside a layout build, where
    // the index is registered too): a selector whose ancestor hashes are not
    // all in the filter cannot match, so it is skipped without walking the
    // ancestor chain. A shared borrow for the whole loop; a nested match
    // (none today) would find it busy and match unfiltered.
    if hashes.is_some() {
        ANCESTOR_BLOOM.with(|b| {
            if let Ok(mut b) = b.try_borrow_mut()
                && let Some(b) = b.as_mut()
            {
                b.sync_to(handle);
            }
        });
    }
    ANCESTOR_BLOOM.with(|cell| {
        let guard = if hashes.is_some() { cell.try_borrow().ok() } else { None };
        let filter: Option<&BloomFilter> = guard.as_deref().and_then(Option::as_ref).map(|b| &*b.filter);
        let may_match = |ri: usize, k: usize| -> bool {
            match (filter, &hashes) {
                (Some(f), Some(h)) => h[ri].get(k).is_none_or(|ah| selectors::matching::selector_may_match(ah, f)),
                _ => true,
            }
        };
        for ri in order {
            let rule = &rules[ri];
            // The one choke point every cascade/matching call in this file
            // goes through — a media-conditioned rule is only ever a
            // cascade candidate when its own condition currently holds for
            // `viewport_width`. See `MediaQuery`'s own docs for exactly
            // what's evaluated. Checked before the (more expensive) real
            // selector match, same short-circuit order `collect_root_
            // custom_props` also uses.
            // QAPI6c: a height media read here is not the resolving element's own (`match_media`).
            if !rule.media.is_empty() && !crate::incr_style::match_media(|| media_list_matches(&rule.media, viewport_width)) {
                continue;
            }
            if let Some(name) = pseudo {
                let mut best: Option<u32> = None;
                for (k, sel) in rule.selectors.slice().iter().enumerate() {
                    if !matches!(sel.pseudo_element(), Some(OurPseudoElement::Named(n)) if *n == name) {
                        continue;
                    }
                    if !may_match(ri, k) {
                        continue;
                    }
                    let mut ctx = MatchingContext::new(
                        MatchingMode::ForStatelessPseudoElement,
                        None,
                        caches,
                        QuirksMode::NoQuirks,
                        NeedsSelectorFlags::No,
                        MatchingForInvalidation::No,
                    );
                    if matches_selector(sel, 0, None, &element, &mut ctx) {
                        best = best.max(Some(sel.specificity()));
                    }
                }
                if let Some(specificity) = best {
                    matches.push((normal_key(specificity, rule.layer), ri as u32, false));
                    matches.push((important_key(specificity, rule.layer), ri as u32, true));
                }
                continue;
            }
            if !(0..rule.selectors.slice().len()).any(|k| may_match(ri, k)) {
                continue;
            }
            let mut ctx = MatchingContext::new(
                MatchingMode::Normal,
                None,
                caches,
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
                matches.push((normal_key(specificity, rule.layer), ri as u32, false));
                matches.push((important_key(specificity, rule.layer), ri as u32, true));
            }
        }
    });
    matches
}

/// QY53g step 3: a counting bloom filter of the tag, namespace, id and
/// classes of the ancestor elements of the element being matched, kept in
/// step with the layout build's (mostly depth-first) matching order: the
/// filter holds the chain `stack` (root element first, each entry the
/// parent of the next), and moving to another element only pops and pushes
/// the part of the chain that differs. Lives only while a build runs
/// (`images.rs` sets and clears it with `MATCH_MEMO`; the DOM does not
/// change during a build), so node pointers in `stack` never outlive their
/// nodes.
pub(crate) struct AncestorBloom {
    filter: Box<BloomFilter>,
    stack: Vec<(usize, Vec<u32>)>,
}

impl Default for AncestorBloom {
    fn default() -> Self {
        AncestorBloom { filter: Box::new(BloomFilter::new()), stack: Vec::new() }
    }
}

impl AncestorBloom {
    /// The hashes a selector's `AncestorHashes` can ask about for `h`
    /// (same hash as `Atom::precomputed_hash`).
    fn element_hashes(h: &Handle) -> Vec<u32> {
        let mut out = Vec::new();
        if let NodeData::Element { name, .. } = &h.data {
            out.push(crate::dom::atom_hash(name.local.as_ref()));
            out.push(crate::dom::atom_hash(name.ns.as_ref()));
        }
        if let Some(id) = element_attr(h, "id") {
            out.push(crate::dom::atom_hash(&id));
        }
        if let Some(classes) = element_attr(h, "class") {
            for c in classes.split_ascii_whitespace() {
                out.push(crate::dom::atom_hash(c));
            }
        }
        out
    }

    /// Make the filter hold exactly the ancestor elements of `handle`.
    pub(crate) fn sync_to(&mut self, handle: &Handle) {
        let mut missing: Vec<Handle> = Vec::new();
        let mut cur = El(handle.clone()).parent_element();
        let keep = loop {
            let Some(p) = cur else { break 0 };
            let ptr = Rc::as_ptr(&p.0) as usize;
            if let Some(pos) = self.stack.iter().rposition(|(q, _)| *q == ptr) {
                break pos + 1;
            }
            cur = p.parent_element();
            missing.push(p.0);
        };
        while self.stack.len() > keep {
            let (_, hs) = self.stack.pop().expect("len > keep");
            for h in hs {
                self.filter.remove_hash(h);
            }
        }
        for p in missing.into_iter().rev() {
            let hs = Self::element_hashes(&p);
            for &h in &hs {
                self.filter.insert_hash(h);
            }
            self.stack.push((Rc::as_ptr(&p) as usize, hs));
        }
    }

    /// Test-only: the filter, synced to the ancestors of `handle`.
    #[cfg(test)]
    pub(crate) fn filter_for(&mut self, handle: &Handle) -> &BloomFilter {
        self.sync_to(handle);
        &self.filter
    }
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
    custom_props: Option<CustomPropsRef>,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> Option<Arc<DecodedImage>> {
    let base_matches = matching_declarations(handle, rules, viewport_width);
    let var_lookup = |n: &str| custom_props.and_then(|p| p.get(n));
    let synth = expand_var_decls(&base_matches, custom_props);
    let mut matches = merge_var_matches(&base_matches, &synth);
    let inline_src = element_attr(handle, "style").map(|s| substitute_css_vars(&s, &var_lookup).unwrap_or_default());
    let inline_split = inline_src.map(|s| parse_declarations_split(&s));
    let (inline_decls, inline_important) = match inline_split {
        Some((n, i)) => (Some(n), Some(i)),
        None => (None, None),
    };
    if let Some(i) = inline_important.as_ref() {
        matches.push((u64::MAX, i));
    }
    let bg = match cascade_css(&matches, inline_decls.as_ref(), "background-image", |d| d.background_image.clone()) {
        Cascaded::Value(BackgroundImageDecl::Url(url)) => lookup_css_image(&url, image_cache),
        // `inherit` takes the parent element's resolved image;
        // `background-image` isn't inherited otherwise.
        Cascaded::Wide(CssWide::Inherit) => parent_handle(handle)
            .filter(|p| matches!(p.data, NodeData::Element { .. }))
            .and_then(|parent| resolve_background_image(&parent, rules, viewport_width, custom_props, image_cache))
            .map(|a| {
                if a.mask.is_none() {
                    return a;
                }
                let mut i = DecodedImage::clone(&a);
                i.mask = None;
                Arc::new(i)
            }),
        // An explicit `none`, another CSS-wide keyword, or undeclared.
        _ => None,
    };
    // `mask-image` (Q29), same data-URI policy; not inherited.
    let mask = match cascade_css(&matches, inline_decls.as_ref(), "mask-image", |d| d.mask_image.clone()) {
        Cascaded::Value(BackgroundImageDecl::Url(url)) => {
            lookup_css_image(&url, image_cache).map(|a| Arc::new(whiten_mask(DecodedImage::clone(&a))))
        }
        _ => None,
    };
    match (bg, mask) {
        (None, None) => None,
        (Some(img), None) => Some(img),
        (img, mask) => {
            let mut img = match img {
                Some(a) => DecodedImage::clone(&a),
                None => DecodedImage { width: 0, height: 0, pixels: Vec::new(), svg: None, mask: None },
            };
            img.mask = mask;
            Some(Arc::new(img))
        }
    }
}

thread_local! {
    /// Q35a: external `url()` references of `background-image`/`mask-image`
    /// that `resolve_background_image` met during the current layout build
    /// and found in no cache. The app drains this after each build
    /// (`take_wanted_css_images`) and fetches them like `<img src>`s.
    static WANTED_CSS_IMAGES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Drains the external CSS image URLs (raw, as written / as made absolute
/// by `absolutize_css_urls`) the last layout build asked for and missed.
pub(crate) fn take_wanted_css_images() -> Vec<String> {
    WANTED_CSS_IMAGES.with(|w| std::mem::take(&mut *w.borrow_mut()))
}

/// A CSS `url()` image: a `data:` URI is decoded on the spot; anything
/// else is looked up in `image_cache` (keyed by the raw url text, like
/// `<img src>`), and on a miss recorded in `WANTED_CSS_IMAGES` so the app
/// can fetch it (Q35a).
fn lookup_css_image(url: &str, image_cache: &HashMap<String, Arc<DecodedImage>>) -> Option<Arc<DecodedImage>> {
    if is_data_url(url) {
        return decode_data_uri(url).and_then(|b| decode_image_bytes(&b)).map(Arc::new);
    }
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    crate::incr_box::note_src(url);
    if let Some(img) = image_cache.get(url) {
        return Some(img.clone());
    }
    if crate::incr_box::quiet_wants() {
        return None;
    }
    WANTED_CSS_IMAGES.with(|w| {
        let mut w = w.borrow_mut();
        if !w.iter().any(|u| u == url) {
            w.push(url.to_string());
        }
    });
    None
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
    base: &[(u64, &Declarations)],
    custom_props: Option<CustomPropsRef>,
) -> Vec<Option<VarGroups>> {
    let lookup = |n: &str| custom_props.and_then(|p| p.get(n));
    base.iter()
        .map(|(_, d)| {
            if d.var_decls.is_empty() {
                return None;
            }
            // QAN1h1: identical declaration blocks under the same custom-property
            // table expand identically; memoised for the duration of one build.
            let key = (std::ptr::from_ref::<Declarations>(d) as usize, custom_props);
            let hit = VAR_EXPAND_MEMO.with(|m| m.borrow().as_ref().and_then(|m| m.get(&key).cloned()));
            if let Some(h) = hit {
                #[cfg(test)]
                VAR_EXPAND_HITS.with(|c| c.set(c.get() + 1));
                return Some(h);
            }
            #[cfg(test)]
            VAR_EXPANSIONS.with(|c| c.set(c.get() + 1));
            let lookup: &dyn Fn(&str) -> Option<String> = &lookup;
            // One group per source-order segment (Q52b): the `var()`
            // declarations between two runs of plain ones.
            let groups = d.var_seg.iter().copied().max().map_or(1, |m| m + 1);
            let out: VarGroups = Rc::new(
                (0..groups)
                    .map(|k| {
                        let mut src = String::new();
                        for (idx, (name, raw)) in d.var_decls.iter().enumerate() {
                            if d.var_seg.get(idx).copied().unwrap_or(0) != k {
                                continue;
                            }
                            // Q57b: a value that is invalid at computed-value time makes
                            // the property `unset` instead of dropping the declaration.
                            let value = substitute_css_vars(raw, lookup).unwrap_or_default();
                            let one = format!("{name}:{value};");
                            let (n, i) = parse_declarations_split(&one);
                            if n == Declarations::default() && i == Declarations::default() {
                                src.push_str(name);
                                src.push_str(if strip_important(raw).is_some() { ":unset !important;" } else { ":unset;" });
                            } else {
                                src.push_str(&one);
                            }
                        }
                        parse_declarations_split(&src)
                    })
                    .collect(),
            );
            VAR_EXPAND_MEMO.with(|m| {
                if let Some(m) = m.borrow_mut().as_mut() {
                    m.insert(key, out.clone());
                }
            });
            Some(out)
        })
        .collect()
}

/// One rule block's `var()` groups, expanded (see `expand_var_decls`).
pub(crate) type VarGroups = Rc<Vec<(Declarations, Declarations)>>;
type VarMemo = HashMap<(usize, Option<CustomPropsRef>), VarGroups>;

thread_local! {
    /// QAN1h1: `(declaration block address, custom-property table)` -> its expanded
    /// groups. `Some` only while `build_layout_tree_with_resources` runs (the rules
    /// slice, so the addresses, is fixed then); `None` everywhere else, so a
    /// `getComputedStyle` between builds never sees a stale entry.
    pub(crate) static VAR_EXPAND_MEMO: RefCell<Option<VarMemo>> = const { RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// Test-only counters: real expansions / memo answers on this thread.
    pub(crate) static VAR_EXPANSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static VAR_EXPAND_HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Slots each `expand_var_decls` result right after its rule's own
/// entries, so source order still breaks specificity ties; the plain
/// declarations written after a `var()` group follow it (`late`).
pub(crate) fn merge_var_matches<'a>(
    base: &[(u64, &'a Declarations)],
    synth: &'a [Option<VarGroups>],
) -> Vec<(u64, &'a Declarations)> {
    let mut matches = Vec::with_capacity(base.len() + 2);
    for (idx, &(spec, d)) in base.iter().enumerate() {
        matches.push((spec, d));
        if let Some(Some(groups)) = synth.get(idx) {
            for (k, (n, i)) in groups.iter().enumerate() {
                matches.push((spec, n));
                matches.push((to_important(spec), i));
                if let Some(l) = d.late.get(k) {
                    matches.push((spec, l));
                }
            }
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
    /// QAN1h2: keyed by the lowercased name of an attribute the rightmost compound requires.
    pub(crate) by_attr: HashMap<String, Vec<usize>>,
    /// QAN1h2: rules whose rightmost compound requires `:root`.
    pub(crate) root: Vec<usize>,
    pub(crate) universal: Vec<usize>,
}

const KEY_ID: u8 = 0;
const KEY_CLASS: u8 = 1;
const KEY_TAG: u8 = 2;
const KEY_ATTR: u8 = 3;
const KEY_ROOT: u8 = 4;

/// QAN1h2: the index keys of one rightmost compound (its components, with the
/// pseudo-element compound merged in): every element this compound matches has
/// at least one of the returned keys, so a rule is a candidate only for an
/// element that has one. `None` when nothing here constrains the element
/// (the rule must then stay universal). Preference: id, class, the union of the
/// branches of an `:is()`/`:where()` (only when every branch has keys), `:root`,
/// attribute name, tag.
fn compound_keys(comps: &[&Component<OurSelectorImpl>]) -> Option<Vec<(u8, String)>> {
    let (mut id, mut class, mut tag, mut attr, mut root) = (None, None, None, None, false);
    let mut is_keys: Option<Vec<(u8, String)>> = None;
    for c in comps {
        match c {
            Component::ID(a) => id = Some(a.0.clone()),
            Component::Class(a) => class = class.or_else(|| Some(a.0.clone())),
            Component::LocalName(n) => tag = Some(n.lower_name.0.clone()),
            Component::AttributeInNoNamespaceExists { local_name_lower, .. } => {
                attr = attr.or_else(|| Some(local_name_lower.0.to_ascii_lowercase()));
            }
            Component::AttributeInNoNamespace { local_name, .. } => {
                attr = attr.or_else(|| Some(local_name.0.to_ascii_lowercase()));
            }
            // Mixed-case or namespaced names are parsed into this boxed form;
            // the lowercase of either spelling is what the lookup uses.
            Component::AttributeOther(a) => {
                attr = attr.or_else(|| Some(a.local_name_lower.0.to_ascii_lowercase()));
            }
            Component::Root | Component::NonTSPseudoClass(OurPseudoClass::Root) => root = true,
            Component::Is(list) | Component::Where(list) if is_keys.is_none() => {
                // Every branch's own rightmost compound must be keyed.
                let mut all = Vec::new();
                let mut ok = !list.slice().is_empty();
                for sel in list.slice() {
                    let inner: Vec<&Component<OurSelectorImpl>> = sel.iter().collect();
                    match compound_keys(&inner) {
                        Some(ks) => all.extend(ks),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    all.sort();
                    all.dedup();
                    is_keys = Some(all);
                }
            }
            _ => {}
        }
    }
    if let Some(i) = id {
        Some(vec![(KEY_ID, i)])
    } else if let Some(c) = class {
        Some(vec![(KEY_CLASS, c)])
    } else if let Some(k) = is_keys {
        Some(k)
    } else if root {
        Some(vec![(KEY_ROOT, String::new())])
    } else if let Some(a) = attr {
        Some(vec![(KEY_ATTR, a)])
    } else {
        tag.map(|t| vec![(KEY_TAG, t)])
    }
}

#[derive(Default)]
pub(crate) struct RuleIndex {
    pub(crate) normal: RuleBuckets,
    pub(crate) pseudo: RuleBuckets,
    /// QY53g: per rule, per selector (in `rule.selectors.slice()` order),
    /// the tag/id/class hashes its ANCESTOR compounds require, for the
    /// ancestor bloom filter (`AncestorBloom`) to reject a rule without
    /// walking the ancestor chain.
    pub(crate) ancestor_hashes: Rc<Vec<Box<[selectors::parser::AncestorHashes]>>>,
}

impl RuleIndex {
    pub(crate) fn build(rules: &[StyleRule]) -> Self {
        #[cfg(test)]
        INDEX_BUILDS.with(|c| c.set(c.get() + 1));
        let mut ix = RuleIndex {
            ancestor_hashes: Rc::new(
                rules
                    .iter()
                    .map(|r| r.selectors.slice().iter().map(|s| selectors::parser::AncestorHashes::new(s, QuirksMode::NoQuirks)).collect())
                    .collect(),
            ),
            ..Default::default()
        };
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
                    // QY53g: a pseudo-element selector (`.a::before`) stores
                    // `::before` as its own rightmost compound behind a
                    // pseudo-element combinator; the keys live in the
                    // originating element's compound left of it. Without
                    // this every pseudo rule landed in `universal` (673 on
                    // YouTube, tested for every element and pseudo slot).
                    let mut comps: Vec<&Component<OurSelectorImpl>> = Vec::new();
                    let mut it = sel.iter();
                    loop {
                        comps.extend(it.by_ref());
                        if it.next_sequence() != Some(selectors::parser::Combinator::PseudoElement) {
                            break;
                        }
                    }
                    let key = compound_keys(&comps);
                    match key {
                        Some(ks) => {
                            for k in ks {
                                if !placed.contains(&k) {
                                    placed.push(k);
                                }
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
                    if kind == KEY_ROOT {
                        buckets.root.push(i);
                        continue;
                    }
                    let map = match kind {
                        KEY_ID => &mut buckets.by_id,
                        KEY_CLASS => &mut buckets.by_class,
                        KEY_ATTR => &mut buckets.by_attr,
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
        if !b.by_attr.is_empty()
            && let NodeData::Element { attrs, .. } = &handle.data
        {
            for a in attrs.borrow().iter() {
                let n = a.name.local.as_ref();
                let v = if n.bytes().any(|c| c.is_ascii_uppercase()) { b.by_attr.get(&n.to_ascii_lowercase()) } else { b.by_attr.get(n) };
                if let Some(v) = v {
                    out.extend_from_slice(v);
                }
            }
        }
        if !b.root.is_empty() && El(handle.clone()).parent_element().is_none() {
            out.extend_from_slice(&b.root);
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}


/// Which elements a `:hover` rule can possibly restyle (R2.4): for every
/// selector compound containing `:hover`, the id / class / tag it
/// requires (or `any` when it requires none, or `:hover` is nested
/// somewhere this scan can't see). A hover change between elements none
/// of whose chain members could match such a compound can't change any
/// style, so it needs no restyle. Over-approximate by construction.
#[derive(Default, Clone)]
pub(crate) struct HoverKeys {
    /// Some `:hover` compound has no id/class/tag to key on, or `:hover` sits
    /// somewhere the scan cannot see (`:not()`, `:has()`...).
    pub(crate) any: bool,
    /// QY53n1: `:hover` sits where the scan cannot bound what it affects;
    /// the exact affected-set path then always builds.
    pub(crate) unbounded: bool,
    pub(crate) ids: HashMap<String, HoverFollow>,
    pub(crate) classes: HashMap<String, HoverFollow>,
    pub(crate) tags: HashMap<String, HoverFollow>,
    /// Follow of the key-less compounds.
    pub(crate) any_follow: HoverFollow,
}

/// QY53n1: what the selector asks of the elements after a `:hover` compound.
/// `desc`: a descendant/child combinator follows (the hovered element's
/// subtree is affected); `sib`: a sibling combinator follows (its later
/// siblings and their subtrees are too).
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub(crate) struct HoverFollow {
    pub(crate) desc: bool,
    pub(crate) sib: bool,
}

impl HoverFollow {
    fn union(self, o: HoverFollow) -> HoverFollow {
        HoverFollow { desc: self.desc || o.desc, sib: self.sib || o.sib }
    }
}

type OurSel = selectors::parser::Selector<OurSelectorImpl>;

fn sel_has_hover(sel: &OurSel) -> bool {
    sel.iter_raw_match_order().any(|c| matches!(c, Component::NonTSPseudoClass(OurPseudoClass::Hover)) || nested_hover(c) != NestedHover::No)
}

#[derive(PartialEq, Clone, Copy)]
enum NestedHover {
    No,
    /// `:not(:hover)`, `:is(:hover)`: the same element's state, so the enclosing compound keys it.
    Compound,
    /// Reaches other elements (`:has(:hover)`, `:not(.a :hover)`...).
    Unbounded,
}

/// QY53n1: how a `:hover` inside the functional pseudo-class `c` (`:not()`, `:is()`,
/// `:where()`, `:has()`, `:nth-child(of)`) affects matching.
fn nested_hover(c: &Component<OurSelectorImpl>) -> NestedHover {
    let list = |l: &[OurSel]| {
        let mut out = NestedHover::No;
        for s in l.iter().filter(|s| sel_has_hover(s)) {
            let reaches = s.iter_raw_match_order().any(|c| matches!(c, Component::Combinator(_)));
            // A hover nested one more level down is not keyed either.
            let deeper = s.iter_raw_match_order().any(|c| nested_hover(c) != NestedHover::No);
            if reaches || deeper {
                return NestedHover::Unbounded;
            }
            out = NestedHover::Compound;
        }
        out
    };
    match c {
        Component::Negation(l) | Component::Is(l) | Component::Where(l) => list(l.slice()),
        Component::Has(l) => {
            if l.iter().any(|h| sel_has_hover(&h.selector)) {
                NestedHover::Unbounded
            } else {
                NestedHover::No
            }
        }
        Component::NthOf(d) => {
            if d.selectors().iter().any(sel_has_hover) {
                NestedHover::Unbounded
            } else {
                NestedHover::No
            }
        }
        _ => NestedHover::No,
    }
}

fn note_follow(map: &mut HashMap<String, HoverFollow>, key: String, f: HoverFollow) {
    let e = map.entry(key).or_default();
    *e = e.union(f);
}

impl HoverKeys {
    pub(crate) fn build(rules: &[StyleRule]) -> Self {
        use selectors::parser::Combinator;
        let mut k = HoverKeys::default();
        for rule in rules {
            for sel in rule.selectors.slice() {
                let mut follow = HoverFollow::default();
                let mut it = sel.iter();
                loop {
                    let comps: Vec<_> = it.by_ref().collect();
                    let top = comps.iter().any(|c| matches!(c, Component::NonTSPseudoClass(OurPseudoClass::Hover)));
                    let nest: Vec<NestedHover> = comps.iter().map(|c| nested_hover(c)).collect();
                    if nest.contains(&NestedHover::Unbounded) {
                        k.any = true;
                        k.unbounded = true;
                    }
                    if top || nest.contains(&NestedHover::Compound) {
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
                            (Some(i), _, _) => note_follow(&mut k.ids, i, follow),
                            (_, Some(c), _) => note_follow(&mut k.classes, c, follow),
                            (_, _, Some(t)) => note_follow(&mut k.tags, t, follow),
                            _ => {
                                k.any = true;
                                k.any_follow = k.any_follow.union(follow);
                            }
                        }
                    }
                    match it.next_sequence() {
                        None => break,
                        Some(Combinator::Descendant | Combinator::Child) => follow.desc = true,
                        Some(Combinator::NextSibling | Combinator::LaterSibling) => follow.sib = true,
                        // A pseudo-element hangs off the same element.
                        Some(Combinator::PseudoElement) => {}
                        Some(_) => {
                            follow.desc = true;
                            follow.sib = true;
                        }
                    }
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
        if self.tags.contains_key(&name.local.as_ref().to_ascii_lowercase()) {
            return true;
        }
        if let Some(id) = element_attr(handle, "id")
            && self.ids.contains_key(&*id)
        {
            return true;
        }
        element_attr(handle, "class").is_some_and(|c| c.split_ascii_whitespace().any(|c| self.classes.contains_key(c)))
    }
}

/// QY53a: whether some rule targets `::first-line` / `::first-letter`, worked
/// out once per parsed rule list (the `Rc` from `RULES_CACHE`) instead of once
/// per element.
pub(crate) struct RuleDerived {
    /// Held so the pointer used as the key cannot be reused by another list.
    pub(crate) rules: Rc<Vec<StyleRule>>,
    pub(crate) first_line: bool,
    pub(crate) first_letter: bool,
    /// QY53b: the selector index; moved into `RULE_INDEX` for the duration of a
    /// build and back afterwards (`None` while a build is running).
    pub(crate) index: Option<Rc<RuleIndex>>,
}

fn scan_pseudo(rules: &[StyleRule], name: &str) -> bool {
    #[cfg(test)]
    PSEUDO_SCANS.with(|c| c.set(c.get() + 1));
    rules
        .iter()
        .any(|r| r.selectors.slice().iter().any(|s| matches!(s.pseudo_element(), Some(OurPseudoElement::Named(n)) if *n == name)))
}

impl RuleDerived {
    pub(crate) fn new(rules: &Rc<Vec<StyleRule>>) -> Self {
        #[cfg(test)]
        DERIVED_BUILDS.with(|c| c.set(c.get() + 1));
        RuleDerived {
            rules: rules.clone(),
            first_line: scan_pseudo(rules, "first-line"),
            first_letter: scan_pseudo(rules, "first-letter"),
            index: Some(crate::rules_cache::shared_index(rules)),
        }
    }
}

thread_local! {
    /// Derived data of the rule list used by the latest build.
    pub(crate) static RULE_DERIVED: RefCell<Option<RuleDerived>> = const { RefCell::new(None) };
    /// `(rules.as_ptr(), len, first-line rule exists, first-letter rule exists)`
    /// for the build in progress; callers with another slice scan the rules.
    pub(crate) static PSEUDO_FLAGS: std::cell::Cell<Option<(usize, usize, bool, bool)>> = const { std::cell::Cell::new(None) };
    /// Test-only: how many times the derived data (index, hover keys, flags) was built.
    #[cfg(test)]
    pub(crate) static DERIVED_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test-only: how many times `RuleIndex::build` ran (QY53f1d2).
    #[cfg(test)]
    pub(crate) static INDEX_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test-only: how many full rule scans for a pseudo-element rule ran.
    #[cfg(test)]
    pub(crate) static PSEUDO_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether some rule targets `::<name>` (`first-line` or `first-letter`):
/// read from the build's precomputed flags, else scanned.
pub(crate) fn rules_have_pseudo(rules: &[StyleRule], name: &str) -> bool {
    if let Some((ptr, len, fl, flt)) = PSEUDO_FLAGS.with(|f| f.get())
        && ptr == rules.as_ptr() as usize
        && len == rules.len()
    {
        return if name == "first-line" { fl } else { flt };
    }
    scan_pseudo(rules, name)
}

thread_local! {
    /// `HoverKeys` of the most recent `build_layout_tree_with_resources`.
    pub(crate) static HOVER_KEYS: RefCell<HoverKeys> = RefCell::new(HoverKeys::default());
    /// Last parsed stylesheet: `(css source, rules, STATE_* mask seen while parsing)`.
    pub(crate) static RULES_CACHE: RefCell<Option<crate::rules_cache::RulesCache>> = const { RefCell::new(None) };
    /// `(rules.as_ptr(), rules.len(), index)` for the rule slice of the
    /// `build_layout_tree_with_resources` call in progress. Matched by
    /// slice identity, so any other caller falls back to a full scan.
    pub(crate) static RULE_INDEX: RefCell<Option<(usize, usize, Rc<RuleIndex>)>> = const { RefCell::new(None) };
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
    // QINC1g: the incremental box rebuild reads the element table's stored style.
    if let Some(s) = crate::incr_box::stored_style(handle, pseudo) {
        return s;
    }
    crate::perf::count_style_resolve(pseudo.is_none());
    // QINC1i4: a recording build notes whether THIS resolve read the viewport height (off: one flag test).
    let rec = crate::incr_style::record_begin();
    let base = resolve_style_core(handle, rules, inherited, viewport_width, pseudo, &[]);
    // QAN1b: no `animation` declared (the overwhelmingly common case) costs
    // exactly this one test.
    let out = match base.animations {
        None => base,
        Some(a) => match a.table() {
            Some(lists) => evaluate(base, &lists, handle, rules, inherited, viewport_width, pseudo),
            None => base, // QY53i3: a handle of a freed arena
        },
    };
    // QANIMHLb: transitions (one flag test when the element declares none and none run anywhere).
    let out = crate::transition::apply(handle, pseudo, &base, out);
    // QINC1f: the shadow check records what the from-scratch build resolved (off: one flag test).
    crate::incr_style::record(handle, pseudo, &out, rec);
    out
}

/// The cascade proper. `over` are keyframe declarations of running
/// animations, entered at `ANIM_KEY` (QAN1b); empty for the base style.
pub(crate) fn resolve_style_core<'a>(
    handle: &Handle,
    rules: &'a [StyleRule],
    inherited: InheritedStyle,
    viewport_width: f32,
    pseudo: Option<&str>,
    over: &[&'a Declarations],
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
    let var_lookup = |n: &str| custom_props.and_then(|p| p.get(n));
    let synth = if rule_has_vars { expand_var_decls(&base_matches, custom_props) } else { Vec::new() };
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
        matches.push((u64::MAX, i));
    }
    for d in over {
        matches.push((ANIM_KEY, d));
    }

    let inl = inline_decls.as_ref();
    let color = apply_wide(
        cascade_css(&matches, inl, "color", |d| d.color),
        true,
        inherited.color,
        ROOT_STYLE.color,
        |c| to_text_color(&c),
    );
    // A closed `<select>` paints its selected option's text in the
    // select's own colour (as browsers do); an `option`'s own `color` only
    // matters in the popup list, which scratchy doesn't draw. Q33c3: the
    // portal sets `select{color:transparent}` and `select option{color:grey}`.
    let color = if pseudo.is_none()
        && element_tag(handle) == "option"
        && parent_handle(handle).is_some_and(|p| matches!(element_tag(&p), "select" | "optgroup" | "datalist"))
    {
        inherited.color
    } else {
        color
    };
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
    // QW3: a `<button>` is never a plain inline box (browsers compute
    // `display:inline` to `inline-block` for it); scratchy only paints
    // border/outline/vertical padding on real boxes, so without this the
    // wikipedia.org language button lost its white outline and the divider
    // line showed through its text.
    let button_inline_block =
        pseudo.is_none() && matches!(element_tag(handle), "button" | "input" | "select" | "textarea" | "progress" | "meter");
    let display = if button_inline_block && display == Display::Inline { Display::InlineBlock } else { display };
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
    let max_width_raw = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.max_width), |d| d.max_width);
    let max_width_intrinsic = max_width_raw == Some(MaxLen::Intrinsic);
    let max_width = max_width_raw
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
    // QY18: sum the translate list as (px, fraction) per axis; BoxLength::
    // Calc is not needed since `%` + `px` mixes are kept as a Calc-free pair
    // by resolving lengths here and carrying percentages separately.
    let translate = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.transform.clone()), |d| d.transform.clone())
        .filter(|l| !l.is_empty())
        .map(|list| {
            let (mut px, mut py, mut fx, mut fy) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for (x, y) in list {
                for (l, p, f) in [(x, &mut px, &mut fx), (y, &mut py, &mut fy)] {
                    match l {
                        Length::Percent(v) => *f += v,
                        other => *p += other.resolve(font_size, inherited.root_font_size),
                    }
                }
            }
            (TranslateAxis { px, frac: fx }, TranslateAxis { px: py, frac: fy })
        });
    let margin_auto = (
        matches!(cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_left), |d| d.margin_left), Some(Length::Auto)),
        matches!(cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_right), |d| d.margin_right), Some(Length::Auto)),
    );
    let margin_auto_v = (
        matches!(cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_top), |d| d.margin_top), Some(Length::Auto)),
        matches!(cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.margin_bottom), |d| d.margin_bottom), Some(Length::Auto)),
    );
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
                .unwrap_or(3.0) // initial `medium`
        };
        border_color[i] = cascade_property(
            &matches,
            inline_decls.as_ref().and_then(|d| d.border_color[i]),
            |d| d.border_color[i],
        )
        .and_then(|c| match c {
            // An explicit `currentColor` is the element's own colour.
            CssColor::CurrentColor => Some(color),
            c => to_text_color(&c),
        });
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
    let border_collapse = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_collapse),
        |d| d.border_collapse,
    )
    .unwrap_or(inherited.border_collapse);
    let word_spacing = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.word_spacing),
        |d| d.word_spacing,
    )
    .map(|l| l.resolve(font_size, inherited.root_font_size))
    .unwrap_or(inherited.word_spacing);
    let letter_spacing = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.letter_spacing),
        |d| d.letter_spacing,
    )
    .map(|l| l.resolve(font_size, inherited.root_font_size))
    .unwrap_or(inherited.letter_spacing);
    let direction = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.direction), |d| d.direction)
        .unwrap_or(inherited.direction);
    let border_spacing = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.border_spacing),
        |d| d.border_spacing,
    )
    .map(|(h, v)| (h.resolve(font_size, inherited.root_font_size).max(0.0), v.resolve(font_size, inherited.root_font_size).max(0.0)))
    .unwrap_or(inherited.border_spacing);
    let outline_offset = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.outline_offset),
        |d| d.outline_offset,
    )
    .map(|l| l.resolve(font_size, inherited.root_font_size))
    .unwrap_or(0.0);
    let transform_origin = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.transform_origin),
        |d| d.transform_origin,
    )
    .map(|(x, y)| {
        let axis = |l: Length| match l {
            Length::Percent(v) => TranslateAxis { px: 0.0, frac: v },
            other => TranslateAxis { px: other.resolve(font_size, inherited.root_font_size), frac: 0.0 },
        };
        (axis(x), axis(y))
    })
    .unwrap_or(INITIAL_TRANSFORM_ORIGIN);
    let text_indent = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.text_indent),
        |d| d.text_indent,
    )
    .map(|l| l.resolve(font_size, inherited.root_font_size))
    .unwrap_or(inherited.text_indent);
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
    // QCSS-D: filter, backdrop-filter, text-shadow (inherited), clip-path, mix-blend-mode.
    let (filter, backdrop_filter, text_shadow, clip_path, mix_blend_mode) = {
        let rfs = inherited.root_font_size;
        let par = inherited.parent;
        let filter = apply_wide(
            cascade_css(&matches, inl, "filter", |d| d.filter),
            false,
            par.and_then(|p| p.filter),
            None,
            |v| Some(crate::fx::resolve_filter(v, font_size, rfs, color)),
        );
        let backdrop_filter = apply_wide(
            cascade_css(&matches, inl, "backdrop-filter", |d| d.backdrop_filter),
            false,
            par.and_then(|p| p.backdrop_filter),
            None,
            |v| Some(crate::fx::resolve_filter(v, font_size, rfs, color)),
        );
        let text_shadow = apply_wide(
            cascade_css(&matches, inl, "text-shadow", |d| d.text_shadow),
            true,
            par.and_then(|p| p.text_shadow),
            None,
            |v| Some(crate::fx::resolve_text_shadow(v, font_size, rfs, color)),
        );
        let clip_path = apply_wide(
            cascade_css(&matches, inl, "clip-path", |d| d.clip_path),
            false,
            par.and_then(|p| p.clip_path),
            None,
            |v| Some(crate::fx::resolve_clip_path(v, font_size, rfs)),
        );
        let mix_blend_mode = apply_wide(
            cascade_css(&matches, inl, "mix-blend-mode", |d| d.mix_blend_mode),
            false,
            par.map_or(BlendMode::Normal, |p| p.mix_blend_mode),
            BlendMode::Normal,
            Some,
        );
        (filter, backdrop_filter, text_shadow, clip_path, mix_blend_mode)
    };
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
        let fixed = !mask
            && apply_wide(
                cascade_css(&matches, inl, "background-attachment", |d| d.background_attachment),
                false,
                false,
                false,
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
            fixed,
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
    let aspect_ratio = apply_wide(
        cascade_css(&matches, inl, "aspect-ratio", |d| d.aspect_ratio),
        false,
        AspectRatio::AUTO,
        AspectRatio::AUTO,
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
    let mut border_radius = CornerRadii::default();
    for (i, c) in border_radius.0.iter_mut().enumerate() {
        if let Some((h, v)) = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.border_radius[i]), |d| d.border_radius[i]) {
            *c = (
                BoxLength::from_length(h, font_size, inherited.root_font_size),
                BoxLength::from_length(v, font_size, inherited.root_font_size),
            );
        }
    }
    let overflow_x = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.overflow), |d| d.overflow)
        .unwrap_or_default();
    let overflow_y = cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.overflow_y), |d| d.overflow_y)
        .unwrap_or_default();
    let (overflow_x, overflow_y) = overflow_axes(overflow_x, overflow_y);
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
    // `align-self: auto` is `None`.
    let align_self = align_self.filter(|a| *a != AlignItems::Auto);
    let justify_items = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.justify_items),
        |d| d.justify_items,
    )
    .unwrap_or_default();
    let justify_self = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.justify_self),
        |d| d.justify_self,
    )
    .filter(|a| *a != AlignItems::Auto);
    let grid_auto_flow = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.grid_auto_flow),
        |d| d.grid_auto_flow,
    )
    .unwrap_or_default();
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
    let grid_auto_columns = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.grid_auto_columns),
        |d| d.grid_auto_columns,
    );
    let grid_auto_rows = cascade_property(
        &matches,
        inline_decls.as_ref().and_then(|d| d.grid_auto_rows),
        |d| d.grid_auto_rows,
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
        pseudo: pseudo_slot(pseudo).unwrap_or(u8::MAX),
        tree_order: next_style_order(),
        custom_props,
        color,
        background_color,
        display,
        position,
        inset,
        z_index,
        opacity,
        translate,
        float,
        clear,
        width,
        height,
        min_width,
        width_kw,
        min_width_kw,
        max_width,
        max_width_intrinsic,
        min_height,
        max_height,
        box_sizing,
        margin,
        margin_auto,
        margin_auto_v,
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
        svg: cascade_svg(&matches, inl, inherited.svg),
        word_spacing,
        letter_spacing,
        direction,
        border_spacing,
        outline_offset,
        transform_origin,
        border_collapse,
        text_indent,
        outline_width,
        outline_color,
        box_shadow,
        filter,
        backdrop_filter,
        text_shadow,
        clip_path,
        mix_blend_mode,
        bg_layout,
        canvas_src: 0,
        viewport_hidden: false,
        viewport_hidden_x: false,
        mask_layout,
        object_fit,
        aspect_ratio,
        object_pos,
        border_radius,
        overflow_x,
        overflow_y,
        text_overflow,
        animations: cascade_animations(&matches, inl),
        transitions: cascade_transitions(&matches, inl),
        contain_lp: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.contain_lp), |d| d.contain_lp).unwrap_or(false),
        appearance_none: cascade_property(&matches, inline_decls.as_ref().and_then(|d| d.appearance_none), |d| d.appearance_none)
            .unwrap_or(false),
        check_widget: 0,
        text_widget: 0,
        select_widget: 0,
        icb_height: None,
        first_line: None,
        cursor,
        flex_direction,
        flex_wrap,
        justify_content,
        align_content,
        align_items,
        align_self,
        justify_items,
        justify_self,
        grid_auto_flow,
        flex_grow,
        order,
        flex_shrink,
        flex_basis,
        row_gap,
        column_gap,
        grid_template_columns,
        grid_template_rows,
        grid_auto_columns,
        grid_auto_rows,
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
        if button_inline_block && cs.display == Display::Inline {
            cs.display = Display::InlineBlock;
        }
    }
    cs
}

thread_local! {
    pub(crate) static STYLE_ORDER: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Next value of the monotonically increasing tree-order counter (QY1).
pub(crate) fn next_style_order() -> usize {
    STYLE_ORDER.with(|c| {
        let v = c.get() + 1;
        c.set(v);
        v
    })
}

pub(crate) fn anonymous_style(inherited: InheritedStyle) -> ComputedStyle {
    ComputedStyle {
        node: 0,
        pseudo: 0,
        tree_order: 0,
        custom_props: inherited.custom_props,
        color: inherited.color,
        background_color: None,
        display: Display::Block,
        position: Position::Static,
        inset: Insets::default(),
        z_index: None,
        opacity: 1.0,
        translate: None,
        float: Float::default(),
        clear: Clear::default(),
        width: None,
        height: None,
        min_width: None,
        width_kw: None,
        min_width_kw: None,
        max_width: None,
        max_width_intrinsic: false,
        min_height: None,
        max_height: None,
        box_sizing: BoxSizing::default(),
        margin: Sides::default(),
        margin_auto: (false, false),
        margin_auto_v: (false, false),
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
        svg: inherited.svg,
        word_spacing: inherited.word_spacing,
        letter_spacing: inherited.letter_spacing,
        direction: inherited.direction,
        border_spacing: inherited.border_spacing,
        outline_offset: 0.0,
        transform_origin: INITIAL_TRANSFORM_ORIGIN,
        border_collapse: inherited.border_collapse,
        text_indent: inherited.text_indent,
        outline_width: 0.0,
        outline_color: None,
        box_shadow: [None; MAX_SHADOWS],
        filter: None,
        backdrop_filter: None,
        text_shadow: inherited.parent.and_then(|p| p.text_shadow),
        clip_path: None,
        mix_blend_mode: BlendMode::Normal,
        bg_layout: BgLayout::default(),
        canvas_src: 0,
        viewport_hidden: false,
        viewport_hidden_x: false,
        mask_layout: BgLayout::default(),
        object_fit: ObjectFit::Fill,
        aspect_ratio: AspectRatio::AUTO,
        object_pos: ((0.5, 0.0), (0.5, 0.0)),
        border_radius: CornerRadii::default(),
        overflow_x: Overflow::default(),
        overflow_y: Overflow::default(),
        text_overflow: TextOverflow::default(),
        animations: None,
        transitions: None,
        contain_lp: false,
        appearance_none: false,
        check_widget: 0,
        text_widget: 0,
        select_widget: 0,
        icb_height: None,
        first_line: None,
        cursor: inherited.cursor,
        flex_direction: FlexDirection::default(),
        flex_wrap: FlexWrap::default(),
        justify_content: JustifyContent::default(),
        align_content: AlignContent::default(),
        align_items: AlignItems::default(),
        align_self: None,
        justify_items: AlignItems::default(),
        justify_self: None,
        grid_auto_flow: GridAutoFlow::default(),
        flex_grow: 0.0,
        order: 0,
        flex_shrink: 1.0,
        flex_basis: FlexBasis::default(),
        row_gap: 0.0,
        column_gap: 0.0,
        grid_template_columns: None,
        grid_template_rows: None,
        grid_auto_columns: None,
        grid_auto_rows: None,
        grid_template_areas: None,
        grid_area: None,
        grid_row_start: None,
        grid_row_end: None,
        grid_column_start: None,
        grid_column_end: None,
    }
}

pub(crate) fn normalize_whitespace(s: &str) -> String {
    s.split(is_css_ws).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// QOBS7/QMEM3c: `(entries, approx bytes)` of `RULES_CACHE` (source text + parsed rules incl.
/// selector components) and of the rule index (`RULE_INDEX` while a build holds it, else
/// `RULE_DERIVED.index`) + `HOVER_KEYS`. The rule list shared by `RULE_DERIVED` is the cache's.
pub(crate) fn rules_cache_mem() -> ((usize, usize), (usize, usize)) {
    let rules = RULES_CACHE.with(|c| match &*c.borrow() {
        Some(rc) => rc.mem(),
        None => (0, 0),
    });
    let buckets = |b: &RuleBuckets| -> (usize, usize) {
        let m = |h: &HashMap<String, Vec<usize>>| -> (usize, usize) {
            (h.len(), h.iter().map(|(k, v)| k.capacity() + 24 + v.capacity() * 8 + 24).sum::<usize>())
        };
        let (a, b1) = m(&b.by_id);
        let (c, b2) = m(&b.by_class);
        let (d, b3) = m(&b.by_tag);
        let (e, b4) = m(&b.by_attr);
        (a + c + d + e, b1 + b2 + b3 + b4 + (b.universal.capacity() + b.root.capacity()) * 8)
    };
    let size_index = |ix: &RuleIndex| -> (usize, usize) {
        let (n1, b1) = buckets(&ix.normal);
        let (n2, b2) = buckets(&ix.pseudo);
        let anc: usize = ix.ancestor_hashes.iter().map(|s| 16 + s.len() * std::mem::size_of::<selectors::parser::AncestorHashes>()).sum();
        (n1 + n2 + ix.ancestor_hashes.len(), b1 + b2 + anc)
    };
    let held = RULE_INDEX.with(|c| c.borrow().as_ref().map(|(_, _, ix)| size_index(ix)));
    let index = held.unwrap_or_else(|| RULE_DERIVED.with(|d| d.borrow().as_ref().and_then(|d| d.index.as_deref()).map_or((0, 0), size_index)));
    let hover = HOVER_KEYS.with(|h| {
        let h = h.borrow();
        let set = |s: &HashMap<String, HoverFollow>| s.keys().map(|k| k.capacity() + 24 + 2).sum::<usize>();
        (h.ids.len() + h.classes.len() + h.tags.len(), set(&h.ids) + set(&h.classes) + set(&h.tags))
    });
    (rules, (index.0 + hover.0, index.1 + hover.1))
}

/// QXSCR1: CSS Overflow 3 computed values: when one axis is a scrolling value (`hidden`, `scroll`,
/// `auto`), a `visible` other axis computes to `auto` and a `clip` one to `hidden`. `visible` with
/// `clip` (either way round) stays as given.
pub(crate) fn overflow_axes(x: Overflow, y: Overflow) -> (Overflow, Overflow) {
    let scrolls = |o: Overflow| matches!(o, Overflow::Hidden | Overflow::Scroll | Overflow::Auto);
    let fix = |o: Overflow| match o {
        Overflow::Visible => Overflow::Auto,
        Overflow::Clip => Overflow::Hidden,
        o => o,
    };
    if scrolls(x) || scrolls(y) { (fix(x), fix(y)) } else { (x, y) }
}
