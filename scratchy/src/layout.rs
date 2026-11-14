use super::*;

// ---------- layout tree ----------

/// Per-span style for one piece of inline text — everything a text run
/// needs from its nearest ancestor's computed style to be shaped
/// correctly, carried independently of the anonymous inline box's own
/// style since a single run can mix spans from different elements
/// (e.g. plain text next to a `<b>`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpanStyle {
    pub(crate) color: TextColor,
    pub(crate) background_color: Option<TextColor>,
    pub(crate) font_size: f32,
    pub(crate) font_weight: FontWeight,
    pub(crate) font_style: FontStyle,
    pub(crate) font_family: FontFamily,
    pub(crate) text_decoration: TextDecoration,
    /// Ambient, same as `text_decoration` (see `DecorationValue`'s docs)
    /// — always set together with it, never independently.
    pub(crate) decoration_style: DecorationStyle,
    pub(crate) vertical_align: VerticalAlign,
    pub(crate) line_height: f32,
    /// Real CSS inheritance (see `Visibility`'s own docs), so this is
    /// read straight off `s.visibility` — already the correctly-cascaded
    /// value for wherever this span's text node actually sits, descendant
    /// overrides included — not a separate ambient-propagation parameter.
    pub(crate) visibility: Visibility,
    pub(crate) cursor: CursorStyle,
    /// Real CSS inheritance too (see `ComputedStyle::word_spacing`'s own
    /// docs) — carried per-span, same as `font_size`/`font_family` above,
    /// so a nested element overriding `word-spacing` only affects its own
    /// words, not its siblings'. Only ever read by `measure_mixed_
    /// inline`, which applies it directly to a word's own measured
    /// advance (see its own docs) — never by `shape_spans`/cosmic-text.
    pub(crate) word_spacing: f32,
    /// `word-break: break-all` (Q20d) switches the run to glyph wrapping.
    pub(crate) word_break: WordBreak,
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
    pub(crate) href: Option<&'static str>,
    /// Q31: `node_id` of the element whose text this is (0 = unknown), for
    /// hover/active hit testing of inline text.
    pub(crate) node: usize,
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
            word_break: s.word_break,
            href: None,
            node: 0,
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
/// `<br>`'s text payload: a lone newline, which cosmic-text breaks on and
/// `measure_mixed_inline` turns into a forced break (Q41).
pub(crate) const BR_TEXT: &str = "\n";

pub(crate) enum InlineItem {
    Text(String, SpanStyle),
    Image {
        image: Arc<DecodedImage>,
        style: ComputedStyle,
        background_image: Option<Arc<DecodedImage>>,
    },
    /// `display: inline-block`/`inline-flex` — an *unmeasured* element
    /// that keeps its own real box model and children (unlike `Image`,
    /// which is a void/replaced element with no descendants at all),
    /// flowing inline instead of flushing to the block-level list the
    /// way an ordinary `display: block` child would — see
    /// `collect_children`'s own docs for the dispatch. Measured for
    /// real inside `measure_mixed_inline`, which is also where its
    /// `width: auto` shrink-to-fit sizing happens (reusing `cell_
    /// natural_width`, the same machinery `float`'s own `width: auto`
    /// already uses). Boxed for the same reason `MixedPositioned::
    /// Embedded`'s own `Laid` is: keeps this enum's own size small
    /// regardless of how large `LayoutBox` gets.
    Embedded(Box<LayoutBox>),
}

pub(crate) enum BoxContent {
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
pub(crate) struct DecodedImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Vec<TextColor>,
    /// The original SVG document when this image came from one, so
    /// `resolve_img` can re-rasterize it at the CSS box size (Q12a2).
    pub(crate) svg: Option<Arc<Vec<u8>>>,
    /// `mask-image` (Q29) of the element this image bundle belongs to
    /// (white pixels carrying the mask's alpha). When an element has a
    /// mask but no `background-image`, the bundle is a 0x0 placeholder.
    pub(crate) mask: Option<Arc<DecodedImage>>,
}

pub struct LayoutBox {
    pub(crate) style: ComputedStyle,
    /// `None` for an anonymous box (not a real element — nothing to
    /// resolve `background-image` against) or a real element that
    /// didn't declare one. See `resolve_background_image`'s own docs
    /// for why this lives here instead of on `style` itself.
    pub(crate) background_image: Option<Arc<DecodedImage>>,
    pub(crate) content: BoxContent,
}


// ---------- measure + paint ----------

/// A laid-out box: resolved border-box rect (absolute document coordinates,
/// not yet scroll-adjusted) plus either its laid-out children or a stable id
/// identifying its shaped text in the shape cache (see `measure`).
pub struct Laid {
    pub(crate) style: ComputedStyle,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
    /// Carried straight over from the `LayoutBox` this was measured
    /// from (an `Arc` clone — see its own docs for why it isn't on
    /// `style`), for `paint` to tile behind this box's own content.
    pub(crate) background_image: Option<Arc<DecodedImage>>,
    pub(crate) content: LaidContent,
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
    pub(crate) positioned: Vec<Laid>,
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
    pub(crate) fixed: bool,
    /// `Some` only for a `position: sticky` box — everything `sticky_
    /// shift` needs to compute this frame's painted position from the
    /// live scroll offset, resolved once here at layout time (never
    /// touched again by a scroll-offset change alone — see `Position::
    /// Sticky`'s own docs). `None` for every other box, including every
    /// other `position` value (`relative`'s own offset is instead baked
    /// permanently into `x`/`y` by `apply_relative_offset`, since it
    /// doesn't need to react to scroll at all).
    pub(crate) sticky: Option<StickyInfo>,
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
    pub(crate) scroll: Option<ScrollInfo>,
    /// Document-space extent of everything this box can paint or be hit
    /// at: its own border box unioned with every descendant's extent
    /// (visible overflow, negative margins, fixed-height wrappers and
    /// out-of-flow descendants all push it past the border box). Computed
    /// once by `compute_ink` at the end of `measure_document`; `paint`
    /// and hit testing prune on it instead of the border box.
    /// `Ink::UNKNOWN` (never prunes) until then.
    pub(crate) ink: Ink,
}

/// See `Laid::ink`.
#[derive(Clone, Copy)]
pub(crate) struct Ink {
    pub(crate) l: f32,
    pub(crate) t: f32,
    pub(crate) r: f32,
    pub(crate) b: f32,
}

impl Ink {
    pub(crate) const UNKNOWN: Ink = Ink { l: f32::NEG_INFINITY, t: f32::NEG_INFINITY, r: f32::INFINITY, b: f32::INFINITY };

    pub(crate) fn union(&mut self, o: Ink) {
        self.l = self.l.min(o.l);
        self.t = self.t.min(o.t);
        self.r = self.r.max(o.r);
        self.b = self.b.max(o.b);
    }

    pub(crate) fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }
}

/// Post-pass: fills `Laid::ink` for the whole finished tree (children
/// first). A clipping box (`overflow` other than `visible`) contributes
/// only its own rect for in-flow content, since descendants are clipped
/// to it; its out-of-flow descendants are still unioned (conservatively,
/// widened upward by the box's max scroll, since those paint shifted by
/// the live scroll offset). `fixed` boxes live in viewport space and are
/// not unioned into their parent.
pub(crate) fn compute_ink(l: &mut Laid) -> Ink {
    let mut ink = Ink { l: l.x, t: l.y, r: l.x + l.w, b: l.y + l.h };
    let clips = l.style.overflow != Overflow::Visible;
    match &mut l.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            for c in children.iter_mut() {
                let ci = compute_ink(c);
                if clips {
                    continue;
                }
                ink.union(ci);
                if let Some(info) = &c.sticky {
                    ink.t = ink.t.min(info.cb_top);
                    ink.b = ink.b.max(info.cb_bottom);
                }
            }
        }
        LaidContent::MixedInline(items) => {
            for item in items.iter_mut() {
                match item {
                    MixedPositioned::Embedded(e) => {
                        let ei = compute_ink(e);
                        if !clips {
                            ink.union(ei);
                        }
                    }
                    MixedPositioned::Word { x, y, w, h, .. } => {
                        if !clips {
                            ink.union(Ink { l: *x, t: *y, r: *x + *w, b: *y + *h });
                        }
                    }
                }
            }
        }
        LaidContent::Text(_) | LaidContent::Image(_) => {}
    }
    let extra = l.scroll.map_or(0.0, |i| i.max_scroll());
    for p in l.positioned.iter_mut() {
        let pi = compute_ink(p);
        if p.fixed {
            continue;
        }
        let mut pi = pi;
        pi.t -= extra;
        ink.union(pi);
    }
    l.ink = ink;
    ink
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
pub(crate) struct StickyInfo {
    /// This box's own would-be-static in-flow position, document-space
    /// — identical to `relative`'s own basis, and (unlike every other
    /// field here) already exactly `laid.y` by the time `capture_
    /// sticky_info` runs, since sticky never leaves flow and nothing
    /// has shifted it yet.
    pub(crate) static_top: f32,
    /// Resolved against this box's own containing block's height
    /// (`Insets`'s usual basis), same timing/basis `apply_relative_
    /// offset` already uses for `relative`'s own `top`/`bottom`. `None`
    /// means not declared (real CSS `auto`) — this box only sticks if
    /// at least one of `top`/`bottom` is set.
    pub(crate) top: Option<f32>,
    pub(crate) bottom: Option<f32>,
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
    pub(crate) cb_top: f32,
    pub(crate) cb_bottom: f32,
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
pub(crate) fn sticky_shift(info: &StickyInfo, box_h: f32, scroll_offset: f32, viewport_h: f32) -> f32 {
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
pub(crate) fn capture_sticky_info(laid: &mut Laid, s: &ComputedStyle, basis_h: Option<f32>) {
    if s.position != Position::Sticky {
        return;
    }
    let resolve_v = |bl: BoxLength| bl.resolve_opt(basis_h).unwrap_or(0.0);
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
pub(crate) fn patch_sticky_containing_block(children: &mut [Laid], cb_top: f32, cb_bottom: f32) {
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
pub(crate) struct ScrollInfo {
    /// Assigned from the same shared `next_id` counter `measure`'s own
    /// paragraph ids already use (`ShapeCache`'s key namespace) — a
    /// different, unrelated-by-value id space, since `App::
    /// scroll_offsets` is its own separate `HashMap`, so there's no
    /// collision risk despite sharing a counter. Reusing it avoids
    /// threading yet another `&mut usize` through every recursive
    /// `measure` call just for this.
    pub(crate) id: usize,
    /// This box's own full in-flow content height — before any
    /// `overflow` clipping, exactly the same quantity `measure`'s
    /// `BoxContent::Blocks` arm already computes as `content_h` for its
    /// own auto-height calculation.
    pub(crate) content_h: f32,
    /// This box's own final, clamped content-box height (`border_h`
    /// minus the border/padding `extra_h` `measure` already subtracts)
    /// — the visible window into `content_h`. Can be *smaller* than an
    /// `auto`-height box's own natural content height whenever an
    /// explicit `height`/`max-height` clamps it, which is exactly the
    /// case that makes this box scrollable at all (real CSS `overflow:
    /// visible` would let the extra content spill out instead — see
    /// v42's own height-overflow note).
    pub(crate) client_h: f32,
}

/// `Some(ScrollInfo)` (with a fresh id) for an `overflow: scroll`/`auto`
/// box, `None` otherwise. Shared by the ordinary `Blocks` arm of `measure`
/// and the table/flex/grid container tails (Q24).
pub(crate) fn make_scroll_info(overflow: Overflow, next_id: &mut usize, content_h: f32, border_h: f32, extra_h: f32) -> Option<ScrollInfo> {
    matches!(overflow, Overflow::Scroll | Overflow::Auto).then(|| {
        let id = *next_id;
        *next_id += 1;
        ScrollInfo { id, content_h: content_h.max(0.0), client_h: (border_h - extra_h).max(0.0) }
    })
}

impl ScrollInfo {
    /// How far this box's own live scroll offset can go before it runs
    /// out of content to reveal — `0.0` whenever `content_h <= client_h`
    /// (nothing actually overflows, the `Auto`-vs-`Scroll` distinction
    /// this whole property hinges on).
    pub(crate) fn max_scroll(&self) -> f32 {
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
pub(crate) struct FloatEntry {
    pub(crate) top: f32,
    pub(crate) bottom: f32,
    pub(crate) edge: f32,
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
    // Interior mutability (Q34a): a float belongs to its BFC, not to the
    // `Blocks` box whose loop placed it, so a `Blocks` arm pushes into the
    // *shared* set it was handed and every later box in the same BFC (even
    // outside that arm's own parent) sees it. Boxes that start a new BFC
    // hand their content a fresh `Floats::default()`, so nothing leaks.
    pub(crate) left: std::cell::RefCell<Vec<FloatEntry>>,
    pub(crate) right: std::cell::RefCell<Vec<FloatEntry>>,
}

impl Floats {
    pub(crate) fn is_empty(&self) -> bool {
        self.left.borrow().is_empty() && self.right.borrow().is_empty()
    }

    /// The lowest bottom edge among every float placed so far (on either
    /// side) — used as a cheap, conservative check (`measure`'s own
    /// `BoxContent::Inline` dispatch) for "has every float in this BFC
    /// already ended by the time this content starts," so a paragraph
    /// safely below every float can keep taking the fast, whole-buffer
    /// shaping path instead of the slower per-word one `avail_bounds`
    /// needs. `f32::MIN` (never `>=` any real `content_y`) when there
    /// are no floats at all.
    pub(crate) fn max_bottom(&self) -> f32 {
        let (l, r) = (self.left.borrow(), self.right.borrow());
        l.iter().chain(r.iter()).map(|f| f.bottom).fold(f32::MIN, f32::max)
    }

    /// The horizontal range available for content at document-space `y`
    /// (a single probe at this exact `y`, not a check across a whole
    /// line's/box's real height range — the same real-but-approximate
    /// simplification a line's own height isn't known until every item
    /// on it is chosen; see `measure_mixed_inline`'s own docs), clamped
    /// within `[outer_left, outer_right]`. Only a float whose own
    /// `top..bottom` range actually contains `y` can narrow it — an
    /// already-finished (or not-yet-started) float has no effect.
    pub(crate) fn avail_bounds(&self, y: f32, outer_left: f32, outer_right: f32) -> (f32, f32) {
        let left_bound = self
            .left
            .borrow()
            .iter()
            .filter(|f| f.top <= y && y < f.bottom)
            .map(|f| f.edge)
            .fold(outer_left, f32::max);
        let right_bound = self
            .right
            .borrow()
            .iter()
            .filter(|f| f.top <= y && y < f.bottom)
            .map(|f| f.edge)
            .fold(outer_right, f32::min);
        (left_bound, right_bound.max(left_bound))
    }

    /// The smallest float bottom edge strictly below `y`, if any — where
    /// a box that can't fit beside the current floats retries (Q34b).
    pub(crate) fn next_bottom(&self, y: f32) -> Option<f32> {
        let (l, r) = (self.left.borrow(), self.right.borrow());
        l.iter().chain(r.iter()).map(|f| f.bottom).filter(|&b| b > y).fold(None, |m, b| Some(m.map_or(b, |m: f32| m.min(b))))
    }

    /// The flow position a `clear`-ing box must not start above: the
    /// lowest bottom edge among the floats `clear` actually names
    /// (`Left`/`Right`/`Both`), or `f32::MIN` if `clear` is `None` or
    /// there's nothing to clear — safe to `cursor_y.max(...)` against
    /// unconditionally, since `f32::MIN` never wins that `max`.
    pub(crate) fn clear_y(&self, clear: Clear) -> f32 {
        let mut y = f32::MIN;
        if matches!(clear, Clear::Left | Clear::Both) {
            y = self.left.borrow().iter().map(|f| f.bottom).fold(y, f32::max);
        }
        if matches!(clear, Clear::Right | Clear::Both) {
            y = self.right.borrow().iter().map(|f| f.bottom).fold(y, f32::max);
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
    pub(crate) b: &'a LayoutBox,
    /// `true` for `position: fixed`. Unlike `absolute`, a `fixed` box's
    /// containing block is always the viewport (not whatever positioned
    /// ancestor happens to be nearest) — real CSS spec behavior pre-
    /// transforms, the only case this engine has any notion of at all —
    /// so a `fixed` entry is never caught by `catch_or_bubble_
    /// positioned` at an intermediate box regardless of that box's own
    /// `position`; it always keeps bubbling until `measure_document`'s
    /// own root-level resolution.
    pub(crate) fixed: bool,
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
    pub(crate) static_x: f32,
    pub(crate) static_y: f32,
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
pub(crate) fn shift_laid_tree(laid: &mut Laid, dx: f32, dy: f32) {
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
                    MixedPositioned::Embedded(laid) => shift_laid_tree(laid, dx, dy),
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
pub(crate) fn apply_relative_offset(laid: &mut Laid, s: &ComputedStyle, basis_w: f32, basis_h: Option<f32>) {
    if s.position != Position::Relative {
        return;
    }
    let resolve_v = |bl: BoxLength| bl.resolve_opt(basis_h).unwrap_or(0.0);
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
pub(crate) fn resolve_or_bubble_positioned<'a>(
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
pub(crate) fn resolve_positioned(
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

pub(crate) enum LaidContent {
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
/// inline`'s own docs for why). `Embedded` is a complete, ordinary
/// `Laid` (`x`/`y` already absolute) — holding either an inline `<img>`
/// (`LaidContent::Image`) or a real `display: inline-block`/`inline-
/// flex` element's own fully measured subtree (`LaidContent::Blocks`,
/// built by `measure_mixed_inline`'s own `InlineItem::Embedded`
/// handling). `paint`/`hit_test_cursor` just recurse into it exactly
/// like any other child `Laid`, regardless of which of those two it
/// actually holds, getting full box-decoration/clipping fidelity for
/// free instead of a second, parallel painting path — this variant was
/// named `Image` before `inline-block`/`inline-flex` existed, back when
/// an inline `<img>` was the only thing it could ever hold.
pub(crate) enum MixedPositioned {
    Word { id: usize, x: f32, y: f32, w: f32, h: f32 },
    Embedded(Box<Laid>),
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
/// text-decoration style, per-span `(link href, node_id)`)`.
pub type ShapedParagraph = (
    Buffer,
    f32,
    Vec<Option<TextColor>>,
    Vec<VerticalAlign>,
    Vec<bool>,
    Vec<CursorStyle>,
    Vec<DecorationStyle>,
    Vec<(Option<&'static str>, usize)>,
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
pub(crate) const PLACEHOLDER_LINE_HEIGHT: f32 = 20.0;

/// Minimum spacing between two hover-driven restyles (R2.3).
pub(crate) const HOVER_RESTYLE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);
/// Hover restyles wait this long after the last wheel tick (R2.3).
pub(crate) const HOVER_WHEEL_QUIET: std::time::Duration = std::time::Duration::from_millis(200);

thread_local! {
    /// Content-keyed shaping memo (R2.2), enabled only on the shaping
    /// worker thread. Paragraph ids are positional and restart at 0 for
    /// every tree, so a restyle can't reuse the id-keyed `ShapeCache`;
    /// instead every shaped result is also remembered under a hash of
    /// everything that determined it (text, span styles, width, flags),
    /// and an identical request later is served from here.
    pub(crate) static SHAPE_MEMO: RefCell<HashMap<u64, Arc<Mutex<ShapedParagraph>>>> = RefCell::new(HashMap::new());
    pub(crate) static SHAPE_MEMO_ENABLED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn memo_key(kind: u8, items: &[InlineItem], width: f32, pre: bool, align: Option<CosmicAlign>) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    if !SHAPE_MEMO_ENABLED.with(|m| m.get()) {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut h);
    width.to_bits().hash(&mut h);
    pre.hash(&mut h);
    format!("{align:?}").hash(&mut h);
    for item in items {
        match item {
            InlineItem::Text(t, span) => {
                t.hash(&mut h);
                format!("{span:?}").hash(&mut h);
            }
            _ => return None,
        }
    }
    Some(h.finish())
}

pub(crate) fn memo_get(key: Option<u64>) -> Option<Arc<Mutex<ShapedParagraph>>> {
    SHAPE_MEMO.with(|m| m.borrow().get(&key?).cloned())
}

pub(crate) fn memo_put(key: Option<u64>, entry: &Arc<Mutex<ShapedParagraph>>) {
    if let Some(k) = key {
        SHAPE_MEMO.with(|m| m.borrow_mut().insert(k, entry.clone()));
    }
}

/// Asks the system's real fontconfig (via `fc-match`, the same resolution
/// path Firefox and other fontconfig-aware apps use) which actually-
/// installed family a generic name like "sans-serif" currently resolves
/// to. Returns `None` on any failure (command missing, non-zero exit,
/// empty output) rather than guessing.
pub(crate) fn fc_match_family(generic: &str) -> Option<String> {
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
    new_font_system_with(&[])
}

/// `new_font_system` plus extra font files loaded into the database (the
/// WPT runner loads `Ahem.ttf` this way).
pub(crate) fn new_font_system_with(extra_fonts: &[PathBuf]) -> FontSystem {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    for f in extra_fonts {
        if let Err(e) = db.load_font_file(f) {
            eprintln!("scratchy: cannot load font {}: {e}", f.display());
        }
    }
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
pub(crate) fn shape_spans(
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
            // Never actually reached — the fast-path guard in `measure`
            // keeps any run containing one of these out of this
            // function entirely, routing it to `measure_mixed_inline`
            // instead (same reasoning the `Image` arm already has).
            InlineItem::Image { .. } | InlineItem::Embedded(_) => None,
        })
        .collect();
    let metrics = Metrics::new(16.0, 20.0);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(Some(width.max(0.0)), None);
    if pre {
        // `white-space: pre`/`nowrap` both disable wrapping — content
        // overflows its box rather than breaking mid-line; literal `\n`s
        // in the text still start new lines regardless of wrap mode
        // (cosmic-text splits buffer lines on them either way).
        buffer.set_wrap(Wrap::None);
    } else if spans.iter().any(|(_, span)| span.word_break == WordBreak::BreakAll) {
        buffer.set_wrap(Wrap::Glyph);
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
            // cosmic-text's `shape_until_scroll` hangs forever (confirmed: a genuine
            // infinite loop, not slowness) when a span's line height is exactly 0.0 —
            // which real CSS legitimately produces via `line-height: 0` (e.g.
            // Wikipedia's thumbnail-figure styling) and which the cascade then carries
            // down into otherwise-ordinary text spans. Floor it just above zero; at
            // this height the line is already visually collapsed, so the clamp isn't
            // perceptible.
            let line_height = if span.line_height <= 0.0 { 0.01 } else { span.line_height };
            attrs = attrs.metrics(Metrics::new(span.font_size, line_height));
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
    let hrefs = spans.iter().map(|(_, span)| (span.href, span.node)).collect();
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
pub(crate) fn clamp_box_dim(
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
pub(crate) fn finalize_box_lengths(s: &mut ComputedStyle, width: f32, containing_height: Option<f32>) {
    let resolve_h = |v: Option<BoxLength>| -> Option<f32> {
        v.and_then(|bl| bl.resolve_opt(containing_height))
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
pub(crate) struct BoxModelEntry {
    pub(crate) border_x: f32,
    pub(crate) border_y: f32,
    pub(crate) border_w: f32,
    pub(crate) content_x: f32,
    pub(crate) content_y: f32,
    pub(crate) content_w: f32,
    pub(crate) extra_h: f32,
    pub(crate) child_containing_height: Option<f32>,
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
pub(crate) fn box_model_entry(
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
    let extra_w = s.border_h() + padding_left + padding_right;
    let auto_border_w = match auto_margin_box_override {
        Some(mb) => (mb - margin_left - margin_right).max(0.0),
        None => (width - margin_left - margin_right).max(0.0),
    };
    let border_w = clamp_box_dim(s.width, s.min_width, s.max_width, auto_border_w, s.box_sizing, extra_w);
    let content_x = border_x + s.border_left() + padding_left;
    let content_y = border_y + s.border_top() + padding_top;
    let content_w = (border_w - extra_w).max(0.0);
    let extra_h = s.border_v() + padding_top + padding_bottom;
    let child_containing_height = s
        .height
        .map(|h| match s.box_sizing {
            BoxSizing::ContentBox => h.px().max(0.0),
            BoxSizing::BorderBox => (h.px() - extra_h).max(0.0),
        })
        .or(s.icb_height);
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
            auto_margin_box_override,
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
    if matches!(b.style.display, Display::Flex | Display::InlineFlex)
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
    let mut auto_margin_box_override = auto_margin_box_override;
    // `width`/`min-width: min-content|max-content|fit-content` (Q20f):
    // sized from the box's own min/max-content widths (margin-box terms,
    // the same `cell_natural_width` a `width: auto` float uses). Not
    // applied to table/flex/grid containers, which return above.
    if s.width_kw.is_some() || s.min_width_kw.is_some() {
        let (min_mb, max_mb) = cell_natural_width(b, font_system, next_id, cache, deadline, incomplete);
        let pick = |k: IntrinsicSize| match k {
            IntrinsicSize::MinContent => min_mb,
            IntrinsicSize::MaxContent => max_mb,
            IntrinsicSize::FitContent => width.min(max_mb).max(min_mb.min(width)),
        };
        if let Some(k) = s.width_kw {
            auto_margin_box_override = Some(pick(k));
        }
        if let Some(k) = s.min_width_kw {
            let mut t = s;
            finalize_box_lengths(&mut t, 0.0, None);
            let margins = t.margin.left.px() + t.margin.right.px();
            let extra = t.border_h() + t.padding.left.px() + t.padding.right.px();
            let border_box = (pick(k) - margins).max(0.0);
            let px = match s.box_sizing {
                BoxSizing::ContentBox => (border_box - extra).max(0.0),
                BoxSizing::BorderBox => border_box,
            };
            s.min_width = Some(BoxLength::Px(px));
        }
    }
    // A box that starts its own BFC (Q34b) sees no outer floats and keeps
    // its own inside it (its auto height grows to contain them, below).
    let bfc_root = b.style.establishes_bfc();
    let fresh_floats = Floats::default();
    let floats = if bfc_root { &fresh_floats } else { floats };
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
            if items.iter().any(|i| matches!(i, InlineItem::Image { .. } | InlineItem::Embedded(_)))
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
                incomplete, floats, pending, false,
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
                ink: Ink::UNKNOWN,
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
            } else if let Some(hit) = {
                let pre = matches!(s.white_space, WhiteSpace::Pre | WhiteSpace::Nowrap);
                let plain = s.first_line.is_none() && !(pre && s.text_overflow == TextOverflow::Ellipsis);
                if plain { memo_get(memo_key(0, items, content_w, pre, s.text_align.to_cosmic())) } else { None }
            } {
                let h = hit.lock().unwrap().1;
                cache.insert(id, hit);
                h
            } else if std::time::Instant::now() < deadline {
                let pre = matches!(s.white_space, WhiteSpace::Pre | WhiteSpace::Nowrap);
                let mut shaped = shape_spans(font_system, items, content_w, pre, s.text_align.to_cosmic());
                let ellipsized;
                let items: &[InlineItem] = if pre && s.text_overflow == TextOverflow::Ellipsis {
                    match ellipsize_items(font_system, items, content_w, s.text_align.to_cosmic(), &mut shaped) {
                        Some(v) => {
                            ellipsized = v;
                            &ellipsized
                        }
                        None => items,
                    }
                } else {
                    items
                };
                // `::first-line` (Q19e2): find where the first laid-out
                // line ends, restyle that prefix, and shape again. The
                // restyle can move the break; that is not re-iterated.
                if let Some(fl) = s.first_line {
                    let first_end = |b: &Buffer| b.layout_runs().next().and_then(|run| run.glyphs.iter().map(|g| g.end).max());
                    let mut end = first_end(&shaped.0);
                    // The restyle (bigger/bolder text) can move the break,
                    // so re-split until the styled prefix is exactly the
                    // first line (bounded; no convergence guarantee).
                    for _ in 0..4 {
                        let Some(e) = end else { break };
                        let restyled = apply_first_line_split(items, e, &fl);
                        shaped = shape_spans(font_system, &restyled, content_w, pre, s.text_align.to_cosmic());
                        let new_end = first_end(&shaped.0);
                        if new_end == Some(e) {
                            break;
                        }
                        end = new_end;
                    }
                }
                let (buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs) = shaped;
                let entry = Arc::new(Mutex::new((buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs)));
                if s.first_line.is_none() && !(pre && s.text_overflow == TextOverflow::Ellipsis) {
                    memo_put(memo_key(0, items, content_w, pre, s.text_align.to_cosmic()), &entry);
                }
                cache.insert(id, entry);
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
                ink: Ink::UNKNOWN,
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
            // Q34a: not a clone any more — floats placed here stay in the
            // shared BFC set for the caller's later siblings.
            let local_floats = floats;
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
                            local_floats.left.borrow_mut().push(FloatEntry { top, bottom, edge });
                        }
                        Float::Right => {
                            let edge = lc.x - margin_left;
                            local_floats.right.borrow_mut().push(FloatEntry { top, bottom, edge });
                        }
                        Float::None => unreachable!(),
                    }
                    float_children.push(lc);
                    continue;
                }
                // Q34b (CSS2.1 §9.5): a BFC root's border box must not
                // overlap a float — place it in the corridor the floats
                // leave, dropping below the next float if its min-content
                // width doesn't fit.
                let (mut child_x, mut child_w) = (content_x, content_w);
                if child.style.establishes_bfc() && child.style.display != Display::None && !local_floats.is_empty() {
                    loop {
                        let (l, r) = local_floats.avail_bounds(cursor_y, content_x, content_x + content_w);
                        if l <= content_x && r >= content_x + content_w {
                            break;
                        }
                        (child_x, child_w) = (l, r - l);
                        let (min_mb, _) = cell_natural_width(child, font_system, next_id, cache, deadline, incomplete);
                        if min_mb <= child_w + 0.01 {
                            break;
                        }
                        match local_floats.next_bottom(cursor_y) {
                            Some(y) => {
                                (child_x, child_w) = (content_x, content_w);
                                cursor_y = y;
                            }
                            None => break,
                        }
                    }
                }
                let lc = measure(
                    child, font_system, child_x, cursor_y, child_w, child_containing_height, next_id, cache,
                    deadline, incomplete, &mut own_pending, local_floats, None,
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
            let mut content_h = cursor_y - content_y;
            if bfc_root {
                content_h = content_h.max(local_floats.max_bottom() - content_y);
            }
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
            let scroll = make_scroll_info(s.overflow, next_id, content_h, border_h, extra_h);
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
                border_x + s.border_left(),
                border_y + s.border_top(),
                (border_w - s.border_h()).max(0.0),
                (border_h - s.border_v()).max(0.0),
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
                ink: Ink::UNKNOWN,
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
            let placement: Vec<(usize, usize)> = (0..cells.len()).map(|i| (i, 1)).collect();
            let (row_h, laid_cells) = lay_out_row_cells(
                cells, &col_widths, &placement, font_system, content_x, content_y, child_containing_height, next_id, cache,
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
                ink: Ink::UNKNOWN,
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
                ink: Ink::UNKNOWN,
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
pub(crate) fn image_box_size(image: &DecodedImage, s: &ComputedStyle) -> (f32, f32) {
    let intrinsic_w = (image.width.max(1)) as f32;
    let intrinsic_h = (image.height.max(1)) as f32;
    let aspect = intrinsic_h / intrinsic_w;
    let extra_w = s.border_h() + s.padding.left.px() + s.padding.right.px();
    let extra_h = s.border_v() + s.padding.top.px() + s.padding.bottom.px();
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
pub(crate) fn image_natural_width(image: &DecodedImage, s: &ComputedStyle) -> f32 {
    match s.width {
        Some(BoxLength::Px(v)) => v.max(1.0),
        _ => image.width.max(1) as f32,
    }
}

/// Splits `text` into words the same way `measure_mixed_inline`'s own
/// line-breaker always has (`split_inclusive(' ')`, each word keeping
/// its own trailing space), but returns byte ranges instead of the
/// substrings themselves — what `word_widths_from_buffer` needs to
/// match words back up against a shaped `Buffer`'s own glyph clusters.
pub(crate) fn word_byte_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut offset = 0usize;
    for word in text.split_inclusive(' ') {
        if !word.is_empty() {
            ranges.push(offset..offset + word.len());
        }
        offset += word.len();
    }
    ranges
}

/// Reads per-word pixel widths back out of a `Buffer` already shaped
/// for its *whole* containing text in one `shape_spans` call — the
/// measurement half of `measure_mixed_inline`'s own whole-chunk
/// shaping (see its own docs for why shaping one `Buffer` per word was
/// a measured 135x slower than one `Buffer` for many words at once).
/// `ranges` must be sorted, non-overlapping, and cover (contiguously)
/// the same text this buffer was shaped from — exactly what `word_byte_
/// ranges` produces on that same text.
///
/// Matches each range against the glyphs whose own `start`/`end` (byte
/// offsets into the line, per cosmic-text's own `LayoutGlyph` docs)
/// overlap it, width = the rightmost glyph edge minus the leftmost —
/// correct for left-to-right runs (both `ranges` and `run.glyphs` are
/// walked forward together, one pass, no backtracking). Not attempted
/// for mixed bidi within one chunk: no worse than before this existed
/// (each word was shaped in total isolation, so cross-word bidi could
/// never be correct either way — see `measure_mixed_inline`'s own
/// docs), and often better now (one real shaping pass over multiple
/// words can get bidi *within* a chunk right, where two separate
/// single-word passes architecturally couldn't).
pub(crate) fn word_widths_from_buffer(buffer: &Buffer, ranges: &[std::ops::Range<usize>]) -> Vec<f32> {
    let mut widths = vec![0.0f32; ranges.len()];
    let Some(run) = buffer.layout_runs().next() else { return widths };
    let mut glyphs = run.glyphs.iter().peekable();
    for (i, range) in ranges.iter().enumerate() {
        let mut min_x: Option<f32> = None;
        let mut max_edge = 0.0f32;
        while let Some(g) = glyphs.peek() {
            if g.end <= range.start {
                glyphs.next();
                continue;
            }
            if g.start >= range.end {
                break;
            }
            min_x = Some(min_x.map_or(g.x, |m| m.min(g.x)));
            max_edge = max_edge.max(g.x + g.w);
            glyphs.next();
        }
        widths[i] = min_x.map(|m| max_edge - m).unwrap_or(0.0);
    }
    widths
}

/// Shapes (or resumes, from a previous tick's cache entry) exactly one
/// word on its own, inserting/reading it under `id` in `cache` —
/// `measure_mixed_inline`'s own per-word fallback path, reached only
/// for `word-spacing != 0` (can't reuse a shared multi-word buffer: see
/// its own docs) or a whole-chunk shape that ends up split across more
/// than one wrapped line (rare). Exactly the logic every word in a
/// mixed-inline run used before this function existed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shape_or_resume_word(
    font_system: &mut FontSystem,
    cache: &mut ShapeCache,
    id: usize,
    word: &str,
    span: SpanStyle,
    width: f32,
    deadline: std::time::Instant,
    incomplete: &mut bool,
) -> (f32, f32) {
    if let Some(entry) = cache.get(&id) {
        let guard = entry.lock().unwrap();
        (guard.0.layout_runs().next().map(|r| r.line_w).unwrap_or(0.0), guard.1)
    } else if std::time::Instant::now() < deadline {
        let synth = [InlineItem::Text(word.to_string(), span)];
        let mkey = memo_key(1, &synth, width, true, None);
        if let Some(hit) = memo_get(mkey) {
            let (w, h) = {
                let g = hit.lock().unwrap();
                (g.0.layout_runs().next().map(|r| r.line_w).unwrap_or(0.0), g.1)
            };
            cache.insert(id, hit);
            return (w, h);
        }
        let (buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs) =
            shape_spans(font_system, &synth, width, true, None);
        let w = buffer.layout_runs().next().map(|r| r.line_w).unwrap_or(0.0);
        let entry = Arc::new(Mutex::new((buffer, h, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs)));
        memo_put(mkey, &entry);
        cache.insert(id, entry);
        (w, h)
    } else {
        *incomplete = true;
        (0.0, PLACEHOLDER_LINE_HEIGHT)
    }
}

/// Real inline-level flow for a `BoxContent::Inline` run that mixes text
/// with at least one inline `<img>` — text genuinely shares a line with
/// the image and wraps around it, unlike a plain-text run (which hands
/// the whole paragraph to cosmic-text's own `Buffer`/`Wrap` in one call
/// and lets it decide line breaks). cosmic-text has no concept of an
/// embedded non-text box mid-paragraph (confirmed against how Firefox's
/// `nsLineLayout` actually handles this — see NOTES.md), so this hand-
/// rolls a simple greedy line-breaker instead, needing each *word*'s own
/// width to decide where lines break around an image.
///
/// **Shapes each `InlineItem::Text` as one whole-item `Buffer`, not one
/// per word** — the second version of this, after real-world testing (a
/// real Wikipedia page, through a merged PR) found the first version's
/// one-`shape_spans`-call-per-word measuring/painting cost a *measured*
/// 135x more than shaping the same words in one call (isolated directly:
/// `shape_spans_per_call_overhead_diagnostic`), the dominant cost behind
/// an even bigger end-to-end slowdown (`mixed_inline_overhead_
/// diagnostic`) — cosmic-text's own per-call setup (a new `Buffer`, its
/// shaping-engine init) dominates completely once called thousands of
/// times for only a word or two of text each. Per-word *widths* are
/// still needed for the wrap decision, so each chunk is shaped once
/// (`word_byte_ranges`/`word_widths_from_buffer` read them back out of
/// that one `Buffer`'s own glyphs) — then, in pass 2, every chunk whose
/// words all land on the *same* wrapped line (the common case for a
/// sparse inline icon in flowing prose — confirmed by this function's
/// own `WordSource::Chunk`/`chunk_fits_one_line` bookkeeping) reuses
/// that same `Buffer` directly for painting too, one `MixedPositioned::
/// Word` for the whole chunk instead of one per word. A chunk that
/// itself ends up split across more than one wrapped line (rare: the
/// chunk alone is wide enough to wrap even before an image's own width
/// is counted) falls back to shaping just its own words individually —
/// same as `word-spacing != 0` already must (see below), since neither
/// can reuse one shared, already-positioned buffer. A pleasant side
/// effect: shaping multiple words together, where it applies, now gets
/// real cross-word kerning/ligatures and bidi *within* one chunk, which
/// shaping one `Buffer` per word architecturally never could.
///
/// Also where `word-spacing` (v59) is actually applied, now that any run
/// with a nonzero value is routed here (see `measure`'s own routing
/// check): baked directly into a word's own measured `width` whenever
/// its token carries the trailing space that marks a real word boundary
/// (`word.ends_with(' ')`) — not a separate field threaded through every
/// downstream use, since every consumer of a word's width here (the
/// wrap-check/cursor-advance just below, the final line-width/`text-
/// align` math, and the hit-test bounding box in `MixedPositioned::
/// Word`) already wants the bigger number for free. Shaped word-by-word
/// (the per-word fallback path, not the whole-chunk one above) precisely
/// because the extra space it inserts after *each* word has to move
/// every word after it independently — only separately-positioned
/// buffers allow that, a shared chunk buffer's own internal layout
/// can't. cosmic-text's own shaping of a word is completely untouched
/// either way — its `Buffer` is shaped at the word's own natural width,
/// with no spacing concept fed into `Attrs` at all; only the *position*
/// of the next word on the line moves. This is the deliberate reason
/// this item routes through the per-word path at all instead of
/// cosmic-text's own `Attrs`/`Buffer` (like the reverted `letter-
/// spacing` attempt did, see TODO.txt): cosmic-text's own internal
/// wrap-width computation never sees word-spacing-adjusted widths, so it
/// has nothing new to get wrong.
///
/// **Known simplifications, not attempted here** (see TODO.txt): no
/// cross-word kerning/ligatures/bidi *across* a chunk boundary (an
/// image, or a `word-spacing` run) — only ever a sub-problem of the
/// same gap any real UA has at a genuinely replaced inline element;
/// `white-space: pre` isn't honored in a mixed run (no special handling
/// of embedded `\n`/multiple spaces — words are always split on plain
/// spaces); an inline `<img>`'s vertical margin is ignored (horizontal
/// margin reserves line space correctly, matching real inline-level box
/// behavior, but doesn't affect line height); an image is bottom-
/// aligned within its line (an approximation of real CSS's baseline
/// alignment — the actual font-metrics baseline offset isn't threaded
/// out here); only `text-align: left/right/center` are applied per line
/// (`justify` falls back to `left` — no inter-word space stretching); a
/// line's trailing space (every word's measured width includes its own
/// trailing space) counts toward line width for alignment purposes,
/// unlike a real UA which hangs trailing whitespace past the edge.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn measure_mixed_inline<'a>(
    items: &'a [InlineItem],
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
    // Where a `position: absolute`/`fixed` descendant *inside* an
    // `InlineItem::Embedded` box bubbles out to, exactly the same
    // `own_pending`/`resolve_or_bubble_positioned` convention every
    // other box shape already uses — this function never needed it
    // before since its only prior non-text item, `Image`, is a void
    // element with no descendants to bubble anything out of. Unused
    // (and safe to pass a throwaway scratch `Vec` into) when `trial` is
    // `true`, since a trial call never descends into a real `measure`.
    pending: &mut Vec<PendingPositioned<'a>>,
    // `true` only for `cell_natural_width`'s own two sizing-only probes
    // (see its own docs) — an `Embedded` item resolves its own natural
    // width there but is never actually measured for real (no `Laid`,
    // no paint, no bubbled positioned descendants), keeping those two
    // probes exactly as cheap as every other `cell_natural_width` arm
    // already is. Every real call (from `measure` itself) passes `false`.
    trial: bool,
) -> (f32, Vec<MixedPositioned>) {
    // Effectively unconstrained — combined with `pre: true` below (which
    // also forces `Wrap::None`), a word is never wrapped/truncated
    // internally by cosmic-text regardless of this value; only used as
    // the buffer's nominal width.
    const WORD_MEASURE_WIDTH: f32 = 1_000_000.0;

    /// Where a word's measured width/height came from — see this
    /// function's own docs for why most words share one buffer per
    /// text item (`Chunk`) instead of getting their own.
    enum WordSource {
        /// `cache[chunk_id]`'s own shared buffer, shaped once for this
        /// whole `InlineItem::Text` — `range` is this word's own byte
        /// range within that item's text, used only if pass 2 ends up
        /// falling back to a per-word re-shape (this chunk split across
        /// more than one wrapped line — rare).
        ///
        /// `word_id` (Q34d) is a cache id reserved for the per-word
        /// fallback up front, in pass 1, so the number of ids a run
        /// consumes never depends on where lines happen to break: with
        /// progressive layout the widths differ between passes, and ids
        /// allocated only on the (width-dependent) fallback shifted every
        /// later id, so later items read another paragraph's cache entry
        /// (blank or wrong words in the live window).
        Chunk { chunk_id: usize, range: std::ops::Range<usize>, word_id: usize },
        /// Already fully shaped and cached under this id on its own —
        /// `word-spacing != 0` (can't reuse a shared multi-word buffer:
        /// the extra space it inserts after *this* word has to move
        /// every word after it independently, which only each having
        /// its own separately-positioned buffer allows).
        Cached(usize),
    }

    #[allow(clippy::large_enum_variant)]
    enum Measured<'a> {
        Word { source: WordSource, width: f32, height: f32 },
        /// A forced line break (`<br>`): ends the current line.
        Break { height: f32 },
        Image { laid_style: ComputedStyle, background_image: Option<Arc<DecodedImage>>, image: Arc<DecodedImage>, advance_w: f32, img_w: f32, img_h: f32 },
        /// `laid`/`own_pending` are `None`/empty in `trial` mode (see
        /// this function's own `trial` parameter docs) — no real
        /// `measure()` call happens there at all, only the width
        /// resolution needed for `cell_natural_width`'s own min/max
        /// probing. `h` is `0.0` in that case too (unused: `cell_
        /// natural_width`'s own callers only ever read `mixed_inline_
        /// extent`, which never inspects height). Deliberately *not* a
        /// margin-box height — `h` is this item's own border-box height
        /// alone (`laid.h`), the exact same "vertical margin ignored for
        /// line-height purposes" simplification `Image`'s own `img_h`
        /// already has (only horizontal margin reserves real line
        /// space — see `advance_w`), not a new one introduced here.
        Embedded { laid: Option<Laid>, own_pending: Vec<PendingPositioned<'a>>, advance_w: f32, h: f32, style: ComputedStyle },
    }

    let mut measured: Vec<Measured<'a>> = Vec::new();
    // Keyed by `chunk_id`, needed only if pass 2 falls back to shaping
    // one of a chunk's words on its own (see `WordSource::Chunk`'s own
    // docs) — every other word's chunk buffer is read straight out of
    // `cache` by id, same as any other `ShapeCache` entry.
    let mut chunk_text: HashMap<usize, (&str, SpanStyle)> = HashMap::new();
    for item in items {
        match item {
            InlineItem::Text(text, span) if text == BR_TEXT => {
                // `<br>` (Q41): a zero-width forced break whose height is
                // the line height of the surrounding text. One id always.
                let id = *next_id;
                *next_id += 1;
                let (_, height) = shape_or_resume_word(font_system, cache, id, " ", *span, WORD_MEASURE_WIDTH, deadline, incomplete);
                measured.push(Measured::Break { height });
            }
            InlineItem::Text(text, span) => {
                if span.word_spacing != 0.0 {
                    for word in text.split_inclusive(' ') {
                        if word.is_empty() {
                            continue;
                        }
                        let id = *next_id;
                        *next_id += 1;
                        let (width, height) = shape_or_resume_word(font_system, cache, id, word, *span, WORD_MEASURE_WIDTH, deadline, incomplete);
                        // Real CSS adds extra space after each word-
                        // separator space character, so only a token
                        // that actually carries its own trailing space
                        // (a real boundary — not the very last word in
                        // the whole run, which never got one) gets any.
                        // Baked straight into `width` rather than a new
                        // field — every consumer of a word's width here
                        // already wants the bigger number for free.
                        let extra = if word.ends_with(' ') { span.word_spacing } else { 0.0 };
                        measured.push(Measured::Word { source: WordSource::Cached(id), width: width + extra, height });
                    }
                    continue;
                }
                // The common case: shape this whole text item as one
                // `Buffer` — see this function's own docs for why that's
                // ~135x cheaper in practice than one `Buffer` per word —
                // then read each word's own width back out of it
                // (`word_widths_from_buffer`). `word-spacing` is zero
                // here (checked above), so nothing needs repositioning
                // relative to this buffer's own natural spacing.
                let ranges = word_byte_ranges(text);
                if ranges.is_empty() {
                    continue;
                }
                let chunk_id = *next_id;
                *next_id += 1;
                let (widths, height) = if let Some(entry) = cache.get(&chunk_id) {
                    let guard = entry.lock().unwrap();
                    (word_widths_from_buffer(&guard.0, &ranges), guard.1)
                } else if std::time::Instant::now() < deadline {
                    let synth = [InlineItem::Text(text.clone(), *span)];
                    let mkey = memo_key(2, &synth, WORD_MEASURE_WIDTH, true, None);
                    if let Some(hit) = memo_get(mkey) {
                        let r = {
                            let g = hit.lock().unwrap();
                            (word_widths_from_buffer(&g.0, &ranges), g.1)
                        };
                        cache.insert(chunk_id, hit);
                        r
                    } else {
                        let (buffer, height, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs) =
                            shape_spans(font_system, &synth, WORD_MEASURE_WIDTH, true, None);
                        let widths = word_widths_from_buffer(&buffer, &ranges);
                        let entry = Arc::new(Mutex::new((buffer, height, backgrounds, valigns, hidden, cursors, decoration_styles, hrefs)));
                        memo_put(mkey, &entry);
                        cache.insert(chunk_id, entry);
                        (widths, height)
                    }
                } else {
                    *incomplete = true;
                    (vec![0.0; ranges.len()], PLACEHOLDER_LINE_HEIGHT)
                };
                chunk_text.insert(chunk_id, (text.as_str(), *span));
                for (range, width) in ranges.into_iter().zip(widths) {
                    let word_id = *next_id;
                    *next_id += 1;
                    measured.push(Measured::Word { source: WordSource::Chunk { chunk_id, range, word_id }, width, height });
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
            InlineItem::Embedded(embedded_box) => {
                // `width: auto` shrink-to-fit — the exact same clamp
                // `float`'s own `width: auto` sizing already uses
                // (`content_w.min(max).max(min.min(content_w))`), reused
                // verbatim in real mode. `box_model_entry`'s own docs
                // confirm this is always safe to compute and pass
                // through even when it turns out not to matter: an
                // *explicit* `width` ignores `auto_margin_box_override`
                // entirely, so skipping the (otherwise pointless) `cell_
                // natural_width` call when `width` is already set is a
                // pure optimization, not a correctness branch.
                let auto_override = if embedded_box.style.width.is_none() {
                    let (min_mb, max_mb) =
                        cell_natural_width(embedded_box, font_system, next_id, cache, deadline, incomplete);
                    Some(if trial {
                        // The float formula above degenerates incorrectly
                        // at exactly the `content_w == 0.0` extreme `cell_
                        // natural_width`'s own min-content trial calls
                        // with (verified by hand: `0.0.min(max).max(min.
                        // min(0.0))` is always `0.0`, never `min`), which
                        // would silently under-report this run's own
                        // min-content width whenever it contains an auto-
                        // width embedded box. The textbook CSS2.1 §10.3.5
                        // shrink-to-fit form is correct at *both*
                        // extremes `cell_natural_width` actually probes
                        // with (`0.0` -> `min_mb`; unbounded -> `max_mb`),
                        // so trial mode uses that instead — a deliberate,
                        // narrow deviation from "mirror float exactly,"
                        // scoped only to this sizing-only path.
                        content_w.max(min_mb).min(max_mb)
                    } else {
                        content_w.min(max_mb).max(min_mb.min(content_w))
                    })
                } else {
                    None
                };
                if trial {
                    // `cell_natural_width`'s own two sizing-only probes
                    // (see this function's own `trial` parameter docs) —
                    // cheap width resolution only, no real recursive
                    // `measure()` call, keeping this exactly as cheap as
                    // every other `cell_natural_width` arm already is.
                    let mut trial_style = embedded_box.style;
                    let entry = box_model_entry(&mut trial_style, 0.0, 0.0, content_w, containing_height, auto_override);
                    let advance_w = entry.border_w + trial_style.margin.left.px() + trial_style.margin.right.px();
                    measured.push(Measured::Embedded { laid: None, own_pending: Vec::new(), advance_w, h: 0.0, style: trial_style });
                } else {
                    // The one real, recursive layout call this item
                    // needs — placed at this run's own placeholder
                    // `(content_x, content_y)` for now (position never
                    // affects width/height resolution, only `box_model_
                    // entry`'s own `width`/`containing_height`/`auto_
                    // margin_box_override` params do), repositioned by
                    // `shift_laid_tree` once pass 2 knows this item's
                    // real final line position — the same provisional-
                    // then-shift trick `float`'s own positioning already
                    // uses. A fresh, empty `Floats` — inline-block/
                    // inline-flex establishes its own BFC, same category
                    // this file already gives a float's own interior, a
                    // table cell's own interior, and absolute/fixed
                    // content (see `Floats`'s own docs). Participates in
                    // the existing incremental-shaping `deadline`/`cache`/
                    // `next_id` budget automatically, with zero new
                    // machinery — one more (possibly expensive) recursive
                    // call, not a new mechanism.
                    let mut own_pending: Vec<PendingPositioned<'a>> = Vec::new();
                    let embedded_floats = Floats::default();
                    let laid = measure(
                        embedded_box, font_system, content_x, content_y, content_w, containing_height, next_id, cache,
                        deadline, incomplete, &mut own_pending, &embedded_floats, auto_override,
                    );
                    let h = laid.h;
                    let advance_w = laid.style.margin.left.px() + laid.w + laid.style.margin.right.px();
                    let style = laid.style;
                    measured.push(Measured::Embedded { laid: Some(laid), own_pending, advance_w, h, style });
                }
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
            Measured::Break { .. } => 0.0,
            Measured::Image { advance_w, .. } | Measured::Embedded { advance_w, .. } => *advance_w,
        };
        if !current.is_empty() && s.white_space != WhiteSpace::Nowrap && cursor_x + w > avail_w {
            let line_height = current
                .iter()
                .map(|&(i, _)| match &measured[i] {
                    Measured::Word { height, .. } | Measured::Break { height } => *height,
                    Measured::Image { img_h, .. } => *img_h,
                    Measured::Embedded { h, .. } => *h,
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
        if let Measured::Break { height } = m {
            // Forced break: close this line now (its height includes the
            // break's own so an empty line still has height).
            let _ = height;
            let line_height = current
                .iter()
                .map(|&(i, _)| match &measured[i] {
                    Measured::Word { height, .. } | Measured::Break { height } => *height,
                    Measured::Image { img_h, .. } => *img_h,
                    Measured::Embedded { h, .. } => *h,
                })
                .fold(0.0f32, f32::max);
            lines.push((left_bound, avail_w, std::mem::take(&mut current)));
            cursor_y += line_height;
            cursor_x = 0.0;
            let bounds = line_bounds(cursor_y);
            left_bound = bounds.0;
            avail_w = bounds.1;
        }
    }
    if !current.is_empty() {
        lines.push((left_bound, avail_w, current));
    }

    // Which line (index into `lines`) each `measured` entry landed on —
    // needed next to decide, per chunk, whether every one of its words
    // stayed together on one line (cheap: reuse that chunk's own shared
    // buffer directly) or got split across more than one by the wrap
    // above (rare: fall back to shaping just that chunk's words on
    // their own, same as `word-spacing != 0` already does).
    let mut line_of = vec![0usize; measured.len()];
    for (line_idx, (_, _, line_items)) in lines.iter().enumerate() {
        for &(i, _) in line_items {
            line_of[i] = line_idx;
        }
    }
    let mut chunk_bounds: HashMap<usize, (usize, usize)> = HashMap::new();
    for (i, m) in measured.iter().enumerate() {
        if let Measured::Word { source: WordSource::Chunk { chunk_id, .. }, .. } = m {
            chunk_bounds.entry(*chunk_id).and_modify(|(_, last)| *last = i).or_insert((i, i));
        }
    }
    let chunk_fits_one_line: HashMap<usize, bool> =
        chunk_bounds.iter().map(|(&id, &(first, last))| (id, line_of[first] == line_of[last])).collect();

    // Pass 2: resolve each line's height/`text-align` offset, emit
    // final absolute (document-space) positions.
    let mut positioned: Vec<MixedPositioned> = Vec::with_capacity(measured.len());
    let mut cursor_y = 0.0f32;
    for (left_bound, avail_w, line) in &lines {
        let line_height = line
            .iter()
            .map(|&(i, _)| match &measured[i] {
                Measured::Word { height, .. } | Measured::Break { height } => *height,
                Measured::Image { img_h, .. } => *img_h,
                Measured::Embedded { h, .. } => *h,
            })
            .fold(0.0f32, f32::max);
        let line_w = line
            .last()
            .map(|&(i, x)| {
                x + match &measured[i] {
                    Measured::Word { width, .. } => *width,
                    Measured::Break { .. } => 0.0,
                    Measured::Image { advance_w, .. } | Measured::Embedded { advance_w, .. } => *advance_w,
                }
            })
            .unwrap_or(0.0);
        let align_offset = match s.text_align {
            TextAlign::Right => (avail_w - line_w).max(0.0),
            TextAlign::Center => (avail_w - line_w).max(0.0) / 2.0,
            TextAlign::Left | TextAlign::Justify => 0.0,
        };
        let mut idx = 0;
        while idx < line.len() {
            let (i, x) = line[idx];
            let final_x = left_bound + x + align_offset;
            if matches!(&measured[i], Measured::Embedded { .. }) {
                // Needs owned access to `laid`/`own_pending` (to move the
                // `Laid` out for `shift_laid_tree` and drain the bubbled
                // positioned descendants) — pulled out of the shared
                // `match &measured[i]` below for exactly that reason;
                // every other arm only ever needs to read its own fields.
                // Safe to take ownership here: each `measured` index is
                // visited exactly once across every line (pass 1 assigns
                // every item to exactly one line, `chunk_fits_one_line`'s
                // own multi-word merge aside, which never applies to this
                // variant).
                let Measured::Embedded { laid, own_pending, advance_w, style, .. } = &mut measured[i] else { unreachable!() };
                match laid.take() {
                    Some(mut laid) => {
                        let own_pending = std::mem::take(own_pending);
                        let target_x = final_x + laid.style.margin.left.px();
                        let target_y = content_y + cursor_y + laid.style.vertical_align.inline_box_dy(line_height, laid.h);
                        let dx = target_x - laid.x;
                        let dy = target_y - laid.y;
                        shift_laid_tree(&mut laid, dx, dy);
                        for mut p in own_pending {
                            // `shift_laid_tree` just moved everything
                            // `laid` already *caught* (its own `Laid::
                            // positioned` list) by exactly `(dx, dy)` —
                            // these entries are still bubbling *past*
                            // this box (`position: static`, no positioned
                            // ancestor found inside it yet), so they need
                            // the identical correction applied by hand,
                            // one `f32` pair at a time, to their own
                            // static-position fallback.
                            p.static_x += dx;
                            p.static_y += dy;
                            pending.push(p);
                        }
                        positioned.push(MixedPositioned::Embedded(Box::new(laid)));
                    }
                    None => {
                        // Trial mode (`cell_natural_width`'s own two
                        // sizing-only probes) — no real `Laid` was ever
                        // produced (see this function's own `trial`
                        // parameter docs), so pass 2 still runs (this
                        // function always runs both passes, trial or
                        // not) but has nothing real to position. Emit a
                        // minimal placeholder purely to keep `Mixed
                        // Positioned`'s own shape — never painted,
                        // `cell_natural_width`'s own callers only ever
                        // read `mixed_inline_extent`, which only touches
                        // `.x`/`.w`.
                        positioned.push(MixedPositioned::Embedded(Box::new(Laid {
                            style: *style,
                            x: final_x,
                            y: content_y + cursor_y,
                            w: *advance_w,
                            h: 0.0,
                            background_image: None,
                            content: LaidContent::Blocks(Vec::new()),
                            positioned: Vec::new(),
                            fixed: false,
                            ink: Ink::UNKNOWN,
                            sticky: None,
                            scroll: None,
                        })));
                    }
                }
                idx += 1;
                continue;
            }
            match &measured[i] {
                Measured::Word { source: WordSource::Chunk { chunk_id, .. }, width, height } if chunk_fits_one_line[chunk_id] => {
                    // This word's whole chunk stayed on one line —
                    // merge it with every immediately-following word on
                    // this same line from the same chunk (contiguous:
                    // a chunk's own word order is preserved end to end)
                    // into one `MixedPositioned::Word`, reusing that
                    // chunk's shared buffer directly instead of
                    // shaping any of them again. See this function's
                    // own docs.
                    let mut end = idx;
                    let mut total_w = 0.0f32;
                    let mut max_h = *height;
                    while end < line.len() {
                        let (j, _) = line[end];
                        match &measured[j] {
                            Measured::Word { source: WordSource::Chunk { chunk_id: other, .. }, width: w2, height: h2 } if other == chunk_id => {
                                total_w += w2;
                                max_h = max_h.max(*h2);
                                end += 1;
                            }
                            _ => break,
                        }
                    }
                    positioned.push(MixedPositioned::Word { id: *chunk_id, x: final_x, y: content_y + cursor_y, w: total_w, h: max_h });
                    idx = end;
                    continue;
                }
                Measured::Word { source: WordSource::Chunk { chunk_id, range, word_id }, .. } => {
                    // Rare fallback: this chunk ended up split across
                    // more than one wrapped line, so its shared buffer
                    // (shaped as one unwrapped line) can't be reused
                    // for painting here — shape just this one word on
                    // its own instead, same as `word-spacing != 0`
                    // already does.
                    let &(text, span) = chunk_text.get(chunk_id).expect("chunk_text missing an entry its own chunk_id was inserted under");
                    let word = &text[range.clone()];
                    let id = *word_id;
                    let (w, h) = shape_or_resume_word(font_system, cache, id, word, span, WORD_MEASURE_WIDTH, deadline, incomplete);
                    positioned.push(MixedPositioned::Word { id, x: final_x, y: content_y + cursor_y, w, h });
                    idx += 1;
                }
                Measured::Break { .. } => {
                    idx += 1;
                }
                Measured::Word { source: WordSource::Cached(id), width, height } => {
                    positioned.push(MixedPositioned::Word { id: *id, x: final_x, y: content_y + cursor_y, w: *width, h: *height });
                    idx += 1;
                }
                Measured::Image { laid_style, background_image, image, img_w, img_h, .. } => {
                    positioned.push(MixedPositioned::Embedded(Box::new(Laid {
                        style: *laid_style,
                        x: final_x + laid_style.margin.left.px(),
                        y: content_y + cursor_y + laid_style.vertical_align.inline_box_dy(line_height, *img_h),
                        w: *img_w,
                        h: *img_h,
                        background_image: background_image.clone(),
                        content: LaidContent::Image(Arc::clone(image)),
                        positioned: Vec::new(),
                        fixed: false,
                        ink: Ink::UNKNOWN,
                        sticky: None,
                        scroll: None,
                    })));
                    idx += 1;
                }
                Measured::Embedded { .. } => {
                    // Needs owned access to `laid`/`own_pending` (to move
                    // the `Laid` out for `shift_laid_tree` and drain the
                    // bubbled positioned descendants) — not reachable
                    // through the shared `&measured[i]` the rest of this
                    // match borrows, so this one arm is pulled out into
                    // its own mutable-borrow step just below instead of
                    // living inline here (see directly below the `match`
                    // this arm's own guard skips past).
                    unreachable!("handled before this match — see the `if let Measured::Embedded` just above it")
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
pub(crate) fn mixed_inline_extent(positioned: &[MixedPositioned]) -> f32 {
    positioned
        .iter()
        .map(|p| match p {
            MixedPositioned::Word { x, w, .. } => x + w,
            MixedPositioned::Embedded(laid) => laid.x + laid.w,
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
pub(crate) fn cell_natural_width(
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
        s.margin.left.px() + s.margin.right.px() + s.border_h() + s.padding.left.px() + s.padding.right.px();
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
            // Both trials' own `pending` is a throwaway scratch `Vec` —
            // `trial: true` guarantees nothing is ever pushed into it
            // (an `Embedded` item resolves only its own width here,
            // never a real nested `measure()` call that could produce a
            // positioned descendant to bubble out — see `measure_mixed_
            // inline`'s own `trial` parameter docs), same spirit as the
            // `no_floats` scratch value just above for the same "no
            // real containing block/BFC exists yet" reason.
            let (_, min_items) = measure_mixed_inline(
                items, &trial_style, font_system, 0.0, 0.0, 0.0, None, next_id, cache, deadline, incomplete,
                &no_floats, &mut Vec::new(), true,
            );
            let (_, max_items) = measure_mixed_inline(
                items, &trial_style, font_system, 0.0, 0.0, UNBOUNDED, None, next_id, cache, deadline, incomplete,
                &no_floats, &mut Vec::new(), true,
            );
            (mixed_inline_extent(&min_items), mixed_inline_extent(&max_items))
        }
        // A nested block stacks its children vertically — its own
        // natural width is the *widest* child, not their sum.
        BoxContent::Blocks(children) => children.iter().fold((0.0f32, 0.0f32), |(mn, mx), child| {
            let (cmn, cmx) = cell_natural_width(child, font_system, next_id, cache, deadline, incomplete);
            // Q34c: a table's caption contributes only its *min*-content
            // width to the table's max-content width (CSS Tables 3).
            let cmx = if child.style.table_caption && cell.style.display == Display::Table { cmn } else { cmx };
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

/// Grid placement of every cell of every `TableRow` among `children`
/// (Q36): one `(start_column, colspan)` per cell, one `Vec` per row, in
/// row order. A cell goes in the first column at or after the previous
/// cell's end that no `rowspan` cell from an earlier row still occupies.
pub(crate) fn table_grid_placement(children: &[LayoutBox]) -> Vec<Vec<(usize, usize)>> {
    // `occupied[c]` = how many more rows (including the current one)
    // column `c` is taken by a rowspan cell from above.
    let mut occupied: Vec<usize> = Vec::new();
    let mut result = Vec::new();
    for child in children {
        let BoxContent::TableRow(cells) = &child.content else { continue };
        let mut col = 0;
        let mut row = Vec::with_capacity(cells.len());
        for cell in cells {
            while occupied.get(col).copied().unwrap_or(0) > 0 {
                col += 1;
            }
            let span = cell.style.col_span.max(1);
            if occupied.len() < col + span {
                occupied.resize(col + span, 0);
            }
            // The row's own tick below removes one, so a rowspan of `k`
            // blocks the next `k - 1` rows.
            for o in &mut occupied[col..col + span] {
                *o = cell.style.row_span.max(1);
            }
            row.push((col, span));
            col += span;
        }
        for o in &mut occupied {
            *o = o.saturating_sub(1);
        }
        result.push(row);
    }
    result
}

/// The real per-column widths `measure_table` distributes a table's own
/// `content_w` into — CSS2.1 §17.5.2.2's automatic table layout, scoped
/// to what matters (a cell's column comes from `table_grid_placement`,
/// which accounts for `colspan`/`rowspan`; a spanning cell only widens
/// the columns it spans if single-column cells left them too narrow). Each column's own `min`/`max` is
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
pub(crate) fn table_column_widths(
    children: &[LayoutBox],
    content_w: f32,
    font_system: &mut FontSystem,
    next_id: &mut usize,
    cache: &mut ShapeCache,
    deadline: std::time::Instant,
    incomplete: &mut bool,
) -> Vec<f32> {
    let placement = table_grid_placement(children);
    let num_cols = placement.iter().flatten().map(|&(start, span)| start + span).max().unwrap_or(0);
    if num_cols == 0 {
        return Vec::new();
    }
    let mut col_min = vec![0.0f32; num_cols];
    let mut col_max = vec![0.0f32; num_cols];
    // Cells spanning several columns are applied after every single-
    // column cell, so they only widen columns that the single-column
    // cells left too narrow for them (CSS Tables 3, spanning cells).
    let mut spanning: Vec<(usize, usize, f32, f32)> = Vec::new();
    let mut row_idx = 0;
    for child in children {
        if let BoxContent::TableRow(cells) = &child.content {
            for (cell, &(start, span)) in cells.iter().zip(&placement[row_idx]) {
                let (min_w, max_w) = cell_natural_width(cell, font_system, next_id, cache, deadline, incomplete);
                if span > 1 {
                    spanning.push((start, span, min_w, max_w));
                    continue;
                }
                col_min[start] = col_min[start].max(min_w);
                // A column's `max` is never smaller than its own `min`
                // (can happen per-cell if a trial measurement landed on
                // an incomplete/placeholder shape under the deadline
                // budget — see `cell_natural_width`'s own docs) so the
                // distribution math below never has to handle `max <
                // min` for a single column.
                col_max[start] = col_max[start].max(max_w).max(col_min[start]);
            }
            row_idx += 1;
        }
    }
    for (start, span, min_w, max_w) in spanning {
        let cols = start..start + span;
        for (want, col) in [(min_w, &mut col_min), (max_w, &mut col_max)] {
            let have: f32 = col[cols.clone()].iter().sum();
            if want > have {
                // Spread the shortfall in proportion to each spanned
                // column's current width (equally if they're all 0).
                for c in cols.clone() {
                    let share = if have > 0.0 { col[c] / have } else { 1.0 / span as f32 };
                    col[c] += (want - have) * share;
                }
            }
        }
        for c in cols {
            col_max[c] = col_max[c].max(col_min[c]);
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
pub(crate) fn lay_out_row_cells<'a>(
    cells: &'a [LayoutBox],
    col_widths: &[f32],
    placement: &[(usize, usize)],
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
    let mut row_h = 0.0f32;
    let mut laid_cells = Vec::with_capacity(cells.len());
    // A table cell's own interior is its own fresh BFC (same category
    // as a float's own content/positioned content — see `Floats`'s own
    // docs): floats don't escape a cell's own box, in either direction.
    let no_floats = Floats::default();
    for (i, cell) in cells.iter().enumerate() {
        // Q36: a cell sits at its grid column and spans `colspan` columns.
        let (start, span) = placement.get(i).copied().unwrap_or((i, 1));
        let width_of = |c: usize| col_widths.get(c).copied().unwrap_or(fallback_w);
        let cell_w: f32 = (start..start + span).map(width_of).sum();
        let cursor_x = content_x + (0..start).map(width_of).sum::<f32>();
        let lc = measure(
            cell, font_system, cursor_x, content_y, cell_w, child_containing_height, next_id, cache, deadline,
            incomplete, pending, &no_floats, None,
        );
        // A `rowspan` cell's height is settled by `measure_table`, after
        // it knows every spanned row's height.
        if cell.style.row_span <= 1 {
            row_h = row_h.max(lc.style.margin.top.px() + lc.style.margin.bottom.px() + lc.h);
        }
        laid_cells.push(lc);
    }
    // `vertical-align: middle`/`bottom` on a cell: stretch it to the row
    // height and move its content down by half/all of the slack. `top`
    // (and `baseline`, approximated as top) leave the cell untouched.
    for lc in laid_cells.iter_mut().filter(|lc| lc.style.row_span <= 1) {
        let extra = row_h - (lc.style.margin.top.px() + lc.style.margin.bottom.px() + lc.h);
        let dy = match lc.style.vertical_align {
            VerticalAlign::Middle => extra / 2.0,
            VerticalAlign::Bottom | VerticalAlign::TextBottom => extra,
            _ => 0.0,
        };
        if extra > 0.0 && dy > 0.0 {
            let own_y = lc.y;
            shift_laid_tree(lc, 0.0, dy);
            lc.y = own_y;
            lc.h += extra;
        }
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
pub(crate) fn measure_row_with_widths<'a>(
    row: &'a LayoutBox,
    col_widths: &[f32],
    placement: &[(usize, usize)],
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
        cells, col_widths, placement, font_system, entry.content_x, entry.content_y, entry.child_containing_height, next_id,
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
        ink: Ink::UNKNOWN,
        sticky: None,
        scroll: None,
    }
}

/// Q36: gives every `rowspan` cell the summed height of the rows it
/// spans. Rows are already laid out with their rowspan cells ignored;
/// when a rowspan cell is taller than its rows together, the last
/// spanned row grows, and every later child of the table shifts down.
/// Returns the total height added.
pub(crate) fn settle_rowspans(laid_children: &mut [Laid]) -> f32 {
    let row_idx: Vec<usize> =
        (0..laid_children.len()).filter(|&i| matches!(laid_children[i].content, LaidContent::TableRow(_))).collect();
    let mut row_h: Vec<f32> = row_idx.iter().map(|&i| laid_children[i].h).collect();
    let mut extra = vec![0.0f32; row_idx.len()];
    // (row ordinal, cell index, last spanned ordinal)
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    for (r, &i) in row_idx.iter().enumerate() {
        let LaidContent::TableRow(cells) = &laid_children[i].content else { continue };
        for (ci, c) in cells.iter().enumerate().filter(|(_, c)| c.style.row_span > 1) {
            let end = (r + c.style.row_span - 1).min(row_idx.len() - 1);
            spans.push((r, ci, end));
            let need = c.style.margin.top.px() + c.style.margin.bottom.px() + c.h;
            let have: f32 = row_h[r..=end].iter().sum();
            if need > have {
                row_h[end] += need - have;
                extra[end] += need - have;
            }
        }
    }
    let mut shift = 0.0f32;
    let mut ord = 0;
    for (i, child) in laid_children.iter_mut().enumerate() {
        shift_laid_tree(child, 0.0, shift);
        if ord < row_idx.len() && row_idx[ord] == i {
            child.h += extra[ord];
            shift += extra[ord];
            ord += 1;
        }
    }
    for (r, ci, end) in spans {
        let total: f32 = row_h[r..=end].iter().sum();
        if let LaidContent::TableRow(cells) = &mut laid_children[row_idx[r]].content {
            let c = &mut cells[ci];
            c.h = (total - c.style.margin.top.px() - c.style.margin.bottom.px()).max(c.h);
        }
    }
    shift
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
/// `colspan`/`rowspan` are supported (Q36: `table_grid_placement`,
/// `settle_rowspans`). No border-collapse/cell borders — still out of
/// scope, see `TODO.txt`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn measure_table<'a>(
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
    auto_margin_box_override: Option<f32>,
) -> Laid {
    let mut s = b.style;
    let entry = box_model_entry(&mut s, x, y, width, containing_height, auto_margin_box_override);
    let col_widths = table_column_widths(children, entry.content_w, font_system, next_id, cache, deadline, incomplete);

    let placement = table_grid_placement(children);
    let mut row_ordinal = 0;
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
    // Q34c: `caption-side: bottom` captions are laid out after the grid.
    let ordered = children
        .iter()
        .filter(|c| !(c.style.table_caption && c.style.caption_bottom))
        .chain(children.iter().filter(|c| c.style.table_caption && c.style.caption_bottom));
    for child in ordered {
        let lc = if matches!(child.content, BoxContent::TableRow(_)) {
            row_ordinal += 1;
            measure_row_with_widths(
                child, &col_widths, &placement[row_ordinal - 1], font_system, entry.content_x, cursor_y, entry.content_w,
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
    cursor_y += settle_rowspans(&mut laid_children);
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
        entry.border_x + s.border_left(),
        entry.border_y + s.border_top(),
        (entry.border_w - s.border_h()).max(0.0),
        (border_h - s.border_v()).max(0.0),
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
        ink: Ink::UNKNOWN,
        sticky: None,
        scroll: make_scroll_info(s.overflow, next_id, content_h, border_h, entry.extra_h),
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
pub(crate) struct FlexBasisInfo {
    pub(crate) basis: f32,
    pub(crate) grow: f32,
    pub(crate) shrink: f32,
    pub(crate) min: f32,
    pub(crate) max: f32,
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
pub(crate) fn resolve_flex_main_sizes(main_size: f32, gap: f32, items: &[FlexBasisInfo]) -> Vec<f32> {
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    let gaps = gap * (n - 1) as f32;
    let clamp = |i: &FlexBasisInfo, v: f32| v.clamp(i.min, i.max.max(i.min));
    let sum_hyp: f32 = items.iter().map(|i| clamp(i, i.basis)).sum::<f32>() + gaps;
    let growing = sum_hyp < main_size;
    let mut sizes: Vec<f32> = items.iter().map(|i| i.basis).collect();
    let mut frozen = vec![false; n];
    for (k, i) in items.iter().enumerate() {
        let hyp = clamp(i, i.basis);
        let factor = if growing { i.grow } else { i.shrink };
        if factor == 0.0 || (growing && i.basis > hyp) || (!growing && i.basis < hyp) {
            frozen[k] = true;
            sizes[k] = hyp;
        }
    }
    let free_now = |sizes: &[f32], frozen: &[bool]| -> f32 {
        main_size - gaps - sizes.iter().zip(frozen).zip(items).map(|((&sz, &f), i)| if f { sz } else { i.basis }).sum::<f32>()
    };
    let initial_free = free_now(&sizes, &frozen);
    // Each pass freezes at least one item, so n passes always suffice.
    for _ in 0..=n {
        if frozen.iter().all(|&f| f) {
            break;
        }
        let factor = |i: &FlexBasisInfo| if growing { i.grow } else { i.shrink };
        let sum_factor: f32 = items.iter().zip(&frozen).filter(|(_, f)| !**f).map(|(i, _)| factor(i)).sum();
        let mut remaining = free_now(&sizes, &frozen);
        if sum_factor < 1.0 && (initial_free * sum_factor).abs() < remaining.abs() {
            remaining = initial_free * sum_factor;
        }
        let sum_scaled: f32 = items.iter().zip(&frozen).filter(|(_, f)| !**f).map(|(i, _)| i.shrink * i.basis).sum();
        let mut unclamped = sizes.clone();
        for (k, i) in items.iter().enumerate() {
            if frozen[k] {
                continue;
            }
            unclamped[k] = if growing {
                i.basis + remaining * (i.grow / sum_factor)
            } else if sum_scaled > 0.0 {
                i.basis - remaining.abs() * (i.shrink * i.basis / sum_scaled)
            } else {
                i.basis
            };
        }
        let mut total_violation = 0.0f32;
        let mut clamped = unclamped.clone();
        for (k, i) in items.iter().enumerate() {
            if !frozen[k] {
                clamped[k] = clamp(i, unclamped[k]).max(0.0);
                total_violation += clamped[k] - unclamped[k];
            }
        }
        for k in 0..n {
            if frozen[k] {
                continue;
            }
            let freeze = if total_violation == 0.0 {
                true
            } else if total_violation > 0.0 {
                clamped[k] > unclamped[k]
            } else {
                clamped[k] < unclamped[k]
            };
            if freeze {
                frozen[k] = true;
            }
        }
        for k in 0..n {
            if frozen[k] {
                sizes[k] = clamped[k];
            }
        }
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
pub(crate) fn distribute_justify_content(leftover: f32, n: usize, justify: JustifyContent) -> (f32, f32) {
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
/// cut (no `align-content`, real baseline alignment
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
pub(crate) fn measure_flex<'a>(
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
    // `order`: stable sort, ties keep source order.
    items.sort_by_key(|c| c.style.order);
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
            reverse, main_gap, cross_gap,
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
        entry.border_x + s.border_left(),
        entry.border_y + s.border_top(),
        (entry.border_w - s.border_h()).max(0.0),
        (border_h - s.border_v()).max(0.0),
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
        ink: Ink::UNKNOWN,
        sticky: None,
        scroll: make_scroll_info(s.overflow, next_id, content_h, border_h, entry.extra_h),
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
pub(crate) fn measure_flex_row<'a>(
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
            let extra_w = item_style.border_h() + item_style.padding.left.px() + item_style.padding.right.px();
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
            // `cell_natural_width` already computes (min, max) together in
            // one call (it shares the same two `measure_mixed_inline`
            // trials either figure would need) — called at most once per
            // item, even though both `basis`'s own `width: auto` fallback
            // below and `min`'s own automatic-minimum-size fallback
            // (CSS Flexbox §4.5, see its own comment below) can each need
            // it; an item with an explicit `flex-basis`/`width` *and* an
            // explicit `min-width` never needs it at all.
            let natural_mb = (basis_from_decl.is_none() && item_style.width.is_none()) || item_style.min_width.is_none();
            let natural_mb =
                natural_mb.then(|| cell_natural_width(item, font_system, next_id, cache, deadline, incomplete));
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
                    let (_, max_mb) = natural_mb.expect("width.is_none() (and basis_from_decl.is_none(), this branch's own guard) guarantees natural_mb is Some");
                    (max_mb - margin_left - margin_right).max(0.0)
                }
            });
            let min = match item_style.min_width {
                Some(v) => to_box(v.px()),
                // CSS Flexbox's own "automatic minimum size" (§4.5): a
                // flex item's real shrink floor, when `min-width` isn't
                // declared, is its own content-based minimum — not `0`.
                // Getting this wrong lets the shrink phase below squeeze
                // an item's text narrower than its own unbreakable
                // content, overflowing it into (and visually overlapping)
                // its neighbor instead of this item just not fully
                // fitting the row — found live on a real Wikipedia page's
                // nav tabs (`Article`/`Talk`, `Read`/`Edit`/`View
                // history`), each an auto-width, `white-space: nowrap`
                // flex item competing for space with its siblings.
                // Clamped to this item's own `basis` so an explicit
                // `width`/`flex-basis` narrower than its min-content
                // isn't forced wider by this fallback.
                None => {
                    let (min_mb, _) = natural_mb.expect("min_width.is_none() (this branch's own guard) guarantees natural_mb is Some");
                    (min_mb - margin_left - margin_right).max(0.0).min(basis)
                }
            };
            Prep {
                basis,
                margin_left,
                margin_right,
                min,
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
    let mut line_crosses: Vec<f32> = Vec::with_capacity(lines.len());
    // `align-items: baseline`: each such item's baseline offset from its
    // own margin-box top, and each line's shared baseline (the max offset).
    let mut base_off: Vec<Option<f32>> = vec![None; items.len()];
    let mut line_bases: Vec<f32> = Vec::with_capacity(lines.len());
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
            if item.style.align_self.unwrap_or(s.align_items) == AlignItems::Baseline {
                let base = first_baseline(&lc, cache).unwrap_or(lc.y + lc.h);
                base_off[i] = Some(lc.style.margin.top.px() + (base - lc.y));
            }
            cursor_x += preps[i].margin_left + main_w + preps[i].margin_right + main_gap + extra_between;
            laid[i] = Some(lc);
        }
        // Baseline-aligned items need room above and below the shared
        // baseline, which can exceed the tallest margin box.
        let line_base = line.iter().filter_map(|&i| base_off[i]).fold(0.0f32, f32::max);
        let below = line
            .iter()
            .filter_map(|&i| {
                let lc = laid[i].as_ref()?;
                Some(lc.style.margin.top.px() + lc.h + lc.style.margin.bottom.px() - base_off[i]?)
            })
            .fold(0.0f32, f32::max);
        line_cross = line_cross.max(line_base + below);
        line_bases.push(line_base);
        line_crosses.push(line_cross);
        line_top += line_cross + cross_gap;
    }

    // `align-content`: only a multi-line container with a definite
    // height has free cross space to distribute (a single line, i.e.
    // `nowrap`, ignores it).
    let (mut ac_start, mut ac_between) = (0.0f32, 0.0f32);
    if wrap != FlexWrap::NoWrap {
        if let Some(inner_h) = s.height.map(|h| match s.box_sizing {
            BoxSizing::ContentBox => h.px().max(0.0),
            BoxSizing::BorderBox => (h.px() - entry.extra_h).max(0.0),
        }) {
            let n = lines.len();
            let used: f32 = line_crosses.iter().sum::<f32>() + cross_gap * (n.saturating_sub(1)) as f32;
            let (st, bt, each) = distribute_align_content(inner_h - used, n, s.align_content);
            ac_start = st;
            ac_between = bt;
            for c in line_crosses.iter_mut() {
                *c += each;
            }
        }
    }

    let mut line_top = entry.content_y + ac_start;
    for (j, line) in lines.iter().enumerate() {
        let line_cross = line_crosses[j];
        for &i in line {
            let lc = laid[i].as_mut().expect("just measured above");
            let align = items[i].style.align_self.unwrap_or(s.align_items);
            let margin_top = lc.style.margin.top.px();
            let margin_bottom = lc.style.margin.bottom.px();
            if align == AlignItems::Stretch && items[i].style.height.is_none() {
                lc.h = (line_cross - margin_top - margin_bottom).max(0.0);
            }
            let target_y = match align {
                AlignItems::Stretch | AlignItems::FlexStart => line_top + margin_top,
                AlignItems::Baseline => line_top + (line_bases[j] - base_off[i].unwrap_or(margin_top)) + margin_top,
                AlignItems::FlexEnd => line_top + (line_cross - lc.h - margin_bottom),
                AlignItems::Center => line_top + (line_cross - lc.h - margin_top - margin_bottom) / 2.0 + margin_top,
            };
            shift_laid_tree(lc, 0.0, target_y - lc.y);
        }
        line_top += line_cross + cross_gap + ac_between;
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
pub(crate) fn measure_flex_column<'a>(
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
    cross_gap: f32,
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
    // Wrapping needs a definite container height to break columns at.
    let wrapping = s.flex_wrap != FlexWrap::NoWrap && entry.child_containing_height.is_some();
    let mut prepped: Vec<ColItem> = items
        .iter()
        .map(|item| {
            let align = item.style.align_self.unwrap_or(s.align_items);
            // Stretch (the default) or an explicit `width` both measure
            // at the full cross width — `box_model_entry`'s own
            // explicit-width-wins-else-auto-fills logic already does
            // the right thing for both for free. Only a non-stretch,
            // `width: auto` item needs an actual shrink-to-fit width
            // computed up front. A wrapping container measures every
            // auto-width item at its fit-content width, because the line's
            // cross size is the widest item and stretch is applied after.
            let width_param = if (wrapping || align != AlignItems::Stretch) && item.style.width.is_none() {
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
            // Automatic minimum size (CSS Flexbox 4.5), column axis: with no
            // `min-height` and `overflow: visible`, the floor is the item's
            // content height (its measured height when `height` is auto),
            // clamped to the basis. An explicit `height` makes the content
            // height unknown without a second measure, so that case keeps a
            // floor of 0 (documented approximation).
            let min = match lc.style.min_height {
                Some(v) => v.px(),
                None if item.style.height.is_none() && lc.style.overflow == Overflow::Visible => lc.h.min(basis),
                None => 0.0,
            };
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

    // Break into lines (columns): one, or greedy by hypothetical main size.
    let mut lines: Vec<Vec<usize>> = Vec::new();
    if !wrapping {
        lines.push((0..n).collect());
    } else {
        let mut current: Vec<usize> = Vec::new();
        let mut extent = 0.0f32;
        for (i, p) in prepped.iter().enumerate() {
            let item_extent = p.basis + p.margin_top + p.margin_bottom;
            let needed = if current.is_empty() { item_extent } else { extent + main_gap + item_extent };
            if !current.is_empty() && needed > main_size {
                lines.push(std::mem::take(&mut current));
                extent = item_extent;
            } else {
                extent = needed;
            }
            current.push(i);
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }

    // Cross size of each line: the full width when not wrapping, else the
    // widest margin box, then `align-content` distributes the free width.
    let mut line_crosses: Vec<f32> = lines
        .iter()
        .map(|line| {
            if !wrapping {
                entry.content_w
            } else {
                line.iter()
                    .map(|&i| prepped[i].lc.style.margin.left.px() + prepped[i].lc.w + prepped[i].lc.style.margin.right.px())
                    .fold(0.0, f32::max)
            }
        })
        .collect();
    let (mut ac_start, mut ac_between) = (0.0f32, 0.0f32);
    if wrapping {
        let used: f32 = line_crosses.iter().sum::<f32>() + cross_gap * (lines.len().saturating_sub(1)) as f32;
        let (st, bt, each) = distribute_align_content(entry.content_w - used, lines.len(), s.align_content);
        ac_start = st;
        ac_between = bt;
        for c in line_crosses.iter_mut() {
            *c += each;
        }
    }
    // `wrap-reverse` stacks the columns from the right edge.
    let mut order: Vec<usize> = (0..lines.len()).collect();
    if s.flex_wrap == FlexWrap::WrapReverse && wrapping {
        order.reverse();
    }
    let mut line_x = vec![0.0f32; lines.len()];
    let mut cursor = entry.content_x + ac_start;
    for &j in &order {
        line_x[j] = cursor;
        cursor += line_crosses[j] + cross_gap + ac_between;
    }

    for (j, line) in lines.iter().enumerate() {
        let infos: Vec<FlexBasisInfo> = line
            .iter()
            .map(|&i| {
                let p = &prepped[i];
                FlexBasisInfo { basis: p.basis, grow: p.grow, shrink: p.shrink, min: p.min, max: p.max }
            })
            .collect();
        let final_sizes = resolve_flex_main_sizes(main_size, main_gap, &infos);
        let used: f32 = line.iter().zip(&final_sizes).map(|(&i, &f)| f + prepped[i].margin_top + prepped[i].margin_bottom).sum::<f32>()
            + main_gap * (line.len().saturating_sub(1)) as f32;
        let leftover = (main_size - used).max(0.0);
        let (start_offset, extra_between) = distribute_justify_content(leftover, line.len(), s.justify_content);
        let mut cursor_y = entry.content_y + start_offset;
        for (&i, &final_h) in line.iter().zip(&final_sizes) {
            let item = items[i];
            let p = &mut prepped[i];
            p.lc.h = final_h;
            let align = item.style.align_self.unwrap_or(s.align_items);
            let margin_l = p.lc.style.margin.left.px();
            let margin_r = p.lc.style.margin.right.px();
            if wrapping && align == AlignItems::Stretch && item.style.width.is_none() {
                p.lc.w = (line_crosses[j] - margin_l - margin_r).max(0.0);
            }
            let target_x = match align {
                AlignItems::Stretch | AlignItems::FlexStart | AlignItems::Baseline => line_x[j] + margin_l,
                AlignItems::FlexEnd => line_x[j] + line_crosses[j] - p.lc.w - margin_r,
                AlignItems::Center => line_x[j] + (line_crosses[j] - p.lc.w - margin_l - margin_r) / 2.0 + margin_l,
            };
            let target_y = cursor_y + p.margin_top;
            let dx = target_x - p.lc.x;
            let dy = target_y - p.lc.y;
            shift_laid_tree(&mut p.lc, dx, dy);
            cursor_y += p.margin_top + final_h + p.margin_bottom + main_gap + extra_between;
        }
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

/// Absolute y of a laid box's first baseline, when it has text: a text
/// box's first shaped line, else the first child (or table cell) that has
/// one. `None` for boxes with no text (images, empty boxes) and for text
/// not shaped yet; callers fall back to the border-box bottom edge.
pub(crate) fn first_baseline(laid: &Laid, cache: &ShapeCache) -> Option<f32> {
    match &laid.content {
        LaidContent::Text(id) => {
            let entry = cache.get(id)?;
            let guard = entry.lock().unwrap();
            let run = guard.0.layout_runs().next()?;
            Some(laid.y + laid.style.border_top() + laid.style.padding.top.px() + run.line_y)
        }
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            children.iter().find_map(|c| first_baseline(c, cache))
        }
        LaidContent::Image(_) | LaidContent::MixedInline(_) => None,
    }
}

/// `align-content`'s distribution of `free` cross space among `n` lines:
/// returns `(start_offset, extra_between_lines, extra_added_to_each_line)`.
/// Shared by the row and column flex layouts.
pub(crate) fn distribute_align_content(free: f32, n: usize, ac: AlignContent) -> (f32, f32, f32) {
    if free <= 0.0 || n == 0 {
        return (0.0, 0.0, 0.0);
    }
    match ac {
        AlignContent::Stretch => (0.0, 0.0, free / n as f32),
        AlignContent::FlexStart => (0.0, 0.0, 0.0),
        AlignContent::FlexEnd => (free, 0.0, 0.0),
        AlignContent::Center => (free / 2.0, 0.0, 0.0),
        AlignContent::SpaceBetween if n > 1 => (0.0, free / (n - 1) as f32, 0.0),
        AlignContent::SpaceBetween => (0.0, 0.0, 0.0),
        AlignContent::SpaceAround => (free / n as f32 / 2.0, free / n as f32, 0.0),
        AlignContent::SpaceEvenly => (free / (n + 1) as f32, free / (n + 1) as f32, 0.0),
    }
}

/// One track's classification for `resolve_grid_tracks`'s own
/// distribution pass — every `GridTrackSpec` variant (plain `Len`/`Fr`,
/// or a `MinMax` of either) collapses to exactly one of these before
/// any arithmetic happens, so the actual distribution loop below never
/// has to re-examine which `GridTrackSpec` variant it came from.
/// `floor` is `0.0` for every non-`MinMax` track — "no extra
/// constraint" is the correct behavior there, not a special case.
pub(crate) struct TrackCalc {
    /// `Some(px)` for a track that takes a fixed size regardless of free
    /// space (a plain `Len`, a `min-content`/`max-content` track, or a
    /// `MinMax` whose own max side is a `Len`). `None` for a flexible
    /// (`fr`) or growable (`intr`) track.
    pub(crate) fixed: Option<f32>,
    /// A flexible track's own floor (`minmax`'s first argument,
    /// resolved) — `0.0` for a plain `Fr` track or a fixed one.
    pub(crate) floor: f32,
    /// A flexible track's own `fr` factor — `0.0` for a fixed track.
    pub(crate) fr: f32,
    /// A content-sized growable track: `(base, growth limit, stretch)`.
    /// Starts at `base`, grows toward `limit` with free space, and when
    /// `stretch` (an `auto` max) also absorbs leftover space equally if
    /// no `fr` track claims it.
    pub(crate) intr: Option<(f32, f32, bool)>,
}

/// CSS Grid's own track sizing for one axis's track list, sized against
/// `available` — a real, independent-of-content, definite extent (the
/// grid container's own content *width*, for `grid-template-columns`,
/// always definite; see `measure_grid`'s own docs for why the same
/// isn't attempted for row tracks). `content[i]` is the `(min, max)`
/// content width of track `i`'s items (only consulted by `auto`/
/// `min-content`/`max-content` tracks). Order: every fixed/`%` track
/// takes its own size; content-sized tracks start at their base and
/// are maximised toward their growth limit with free space; flexible
/// tracks (floor reserved first) share what's left by `fr` factor
/// (CSS Grid §11.7); with no `fr` track, leftover space stretches the
/// `auto` tracks equally.
///
/// Deliberately single-pass, same simplification `resolve_flex_main_
/// sizes` already carries for flexbox: the real spec's own iterative
/// algorithm re-checks whether *this* pass's result actually respects
/// every track's floor/ceiling and, if not, freezes the violating track
/// and redistributes. This matches the real spec exactly whenever every
/// flexible track's floor is `0` and is a reasonable approximation
/// otherwise. Items spanning several columns don't contribute to
/// content sizing (see `measure_grid`).
pub(crate) fn resolve_grid_tracks(
    tracks: &[GridTrackSpec],
    content: &[(f32, f32)],
    available: f32,
    gap: f32,
    font_size: f32,
    root_font_size: f32,
) -> Vec<f32> {
    let n = tracks.len();
    if n == 0 {
        return Vec::new();
    }
    let breadth_px = |b: TrackBreadth, c: (f32, f32)| -> f32 {
        match b {
            TrackBreadth::Len(Length::Percent(p)) => available * p,
            TrackBreadth::Len(l) => l.resolve(font_size, root_font_size),
            TrackBreadth::Auto | TrackBreadth::MinContent => c.0,
            TrackBreadth::MaxContent => c.1,
            // A flexible minimum isn't real CSS; treated as "no floor".
            TrackBreadth::Fr(_) => 0.0,
        }
    };
    let calcs: Vec<TrackCalc> = tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let c = content.get(i).copied().unwrap_or((0.0, 0.0));
            let fixed = |px: f32| TrackCalc { fixed: Some(px), floor: 0.0, fr: 0.0, intr: None };
            match *t {
                GridTrackSpec::AutoRepeat { .. } => fixed(0.0), // expanded before this point
                GridTrackSpec::Fr(f) => TrackCalc { fixed: None, floor: 0.0, fr: f, intr: None },
                GridTrackSpec::Len(Length::Percent(p)) => fixed(available * p),
                GridTrackSpec::Len(l) => fixed(l.resolve(font_size, root_font_size)),
                GridTrackSpec::Auto => TrackCalc { fixed: None, floor: 0.0, fr: 0.0, intr: Some((c.0, c.1.max(c.0), true)) },
                GridTrackSpec::MinContent => fixed(c.0),
                GridTrackSpec::MaxContent => fixed(c.1),
                GridTrackSpec::MinMax(min, max) => {
                    let floor = breadth_px(min, c);
                    match max {
                        TrackBreadth::Fr(f) => TrackCalc { fixed: None, floor, fr: f, intr: None },
                        TrackBreadth::Len(_) => fixed(breadth_px(max, c).max(floor)),
                        TrackBreadth::Auto => {
                            TrackCalc { fixed: None, floor: 0.0, fr: 0.0, intr: Some((floor, c.1.max(floor), true)) }
                        }
                        TrackBreadth::MaxContent | TrackBreadth::MinContent => {
                            let lim = breadth_px(max, c).max(floor);
                            TrackCalc { fixed: None, floor: 0.0, fr: 0.0, intr: Some((floor, lim, false)) }
                        }
                    }
                }
            }
        })
        .collect();
    let flexible = |c: &&TrackCalc| c.fixed.is_none() && c.intr.is_none();
    let sum_fixed: f32 = calcs.iter().filter_map(|c| c.fixed).sum();
    let sum_floor: f32 = calcs.iter().filter(flexible).map(|c| c.floor).sum();
    let sum_fr: f32 = calcs.iter().filter(flexible).map(|c| c.fr).sum();
    let sum_base: f32 = calcs.iter().filter_map(|c| c.intr).map(|i| i.0).sum();
    let gaps = gap * (n.saturating_sub(1)) as f32;
    let mut free = (available - sum_fixed - sum_floor - sum_base - gaps).max(0.0);
    let want: f32 = calcs.iter().filter_map(|c| c.intr).map(|i| i.1 - i.0).sum();
    let grow_frac = if want > 0.0 { (free / want).min(1.0) } else { 0.0 };
    free = (free - want * grow_frac).max(0.0);
    let num_stretch = calcs.iter().filter(|c| matches!(c.intr, Some((_, _, true)))).count();
    let stretch_each = if sum_fr == 0.0 && num_stretch > 0 { free / num_stretch as f32 } else { 0.0 };
    calcs
        .iter()
        .map(|c| match (c.fixed, c.intr) {
            (Some(px), _) => px,
            (None, Some((base, limit, stretch))) => {
                base + (limit - base) * grow_frac + if stretch { stretch_each } else { 0.0 }
            }
            (None, None) if sum_fr > 0.0 => c.floor + free * (c.fr / sum_fr),
            (None, None) => c.floor,
        })
        .collect()
}

/// One grid item's own resolved placement — a single cell for an
/// auto-placed item, or a real multi-column span for one placed by a
/// `grid-area` name that `grid-template-areas` gives more than one
/// column to (e.g. the real Wikipedia CSS this feature was built
/// against: `grid-template-areas:'siteNotice siteNotice'` spans two
/// columns in one row). `col_end` is exclusive, the same convention a
/// Rust range already uses. No `row_end` field: a multi-*row* span's
/// own occupied cells are still marked (see this function's own call
/// site), but nothing past that is tracked per placement, since its own
/// height is attributed entirely to `row_start` — see `measure_grid`'s
/// own docs for why that's a real, narrow, documented cut rather than
/// an oversight.
pub(crate) struct GridPlacement<'a> {
    pub(crate) item: &'a LayoutBox,
    pub(crate) row_start: usize,
    pub(crate) col_start: usize,
    pub(crate) col_end: usize,
}

/// One `grid-template-areas` row string's own per-column name tokens —
/// plain whitespace splitting, the real spec's own grammar for a single
/// template-area string.
pub(crate) fn grid_area_row_tokens(row: &str) -> impl Iterator<Item = &str> {
    row.split_whitespace()
}

/// The number of columns a `grid-template-areas` value implies — the
/// first row's own token count. Real CSS requires every row to agree;
/// a ragged template isn't validated here (see `parse_grid_template_
/// areas`'s own docs for why a malformed value is allowed to fall
/// through to a best-effort placement rather than being rejected
/// outright the way most other unsupported values in this file are).
pub(crate) fn grid_areas_num_cols(areas: &[&str]) -> usize {
    areas.first().map(|row| grid_area_row_tokens(row).count()).unwrap_or(0)
}

/// One named area's own bounding rectangle (`row_start..row_end`,
/// `col_start..col_end`, both exclusive) — the min/max bounds of every
/// cell naming `name` across the whole template. Real CSS requires a
/// name's every occurrence to already form a single rectangle; this
/// doesn't re-validate that, it just takes the bounding box, which is
/// the right rectangle whenever the template *is* well-formed and a
/// harmless (if slightly too generous) guess on a malformed one. The
/// literal `.` token (a real CSS "no name here" placeholder) never
/// matches — no real item would ever declare `grid-area: .`.
pub(crate) fn grid_area_rect(areas: &[&str], name: &str) -> Option<(usize, usize, usize, usize)> {
    let mut rect: Option<(usize, usize, usize, usize)> = None;
    for (r, row) in areas.iter().enumerate() {
        for (c, tok) in grid_area_row_tokens(row).enumerate() {
            if tok != name || tok == "." {
                continue;
            }
            rect = Some(match rect {
                Some((rs, re, cs, ce)) => (rs.min(r), re.max(r + 1), cs.min(c), ce.max(c + 1)),
                None => (r, r + 1, c, c + 1),
            });
        }
    }
    rect
}

/// A `display: grid` element's own box model (`box_model_entry`, same
/// as any other box) plus its content: a real but deliberately
/// narrower subset of CSS Grid Layout
/// (https://www.w3.org/TR/css-grid-1/) than the full spec — explicit
/// `grid-template-columns`/`-rows` track lists (fixed lengths, `%`, the
/// real `fr`-unit flexible-length algorithm, and `minmax()`, all in
/// `resolve_grid_tracks`), `repeat(<integer>, ...)`, `gap`/`row-gap`/
/// `column-gap` (the same `ComputedStyle` fields `measure_flex` already
/// reads — never flexbox-specific), `grid-template-areas` + `grid-area`
/// named placement (`grid_area_rect`), and implicit auto-placement in
/// document order, row by row, for everything a name doesn't place
/// (`grid-auto-flow: row`, the real spec's own default and the only
/// flow this implements).
///
/// **Deliberately cut, each documented precisely, not silently folded
/// into "done" (see `TODO.txt` for the authoritative list)**: no
/// numeric explicit item placement (`grid-column`/`grid-row`/their
/// `-start`/`-end` longhands, or `grid-area`'s own fuller `<grid-line>
/// / <grid-line> / <grid-line> / <grid-line>` numeric form) — only a
/// bare `grid-area: <name>` naming one of `grid-template-areas`'s own
/// areas is recognized; everything else still falls through to plain
/// auto-placement. No `repeat(auto-fill/auto-fit, ...)`. No `justify-
/// items`/`align-items`/`-self` within a cell — an item always measures
/// at its own cell's full width (the same "stretch" an ordinary table
/// cell already gets) and sits at its row's own top edge. No content-
/// based track-sizing keywords (`auto`/`min-content`/`max-content`) —
/// every column/row track is a plain fixed/`%`/`fr`/`minmax()` length
/// (`GridTrackSpec`'s own grammar) or, for an *implicit* row (one
/// beyond `grid-template-rows`'s own list, or when no `grid-template-
/// rows` is declared at all), sized from the max natural height of
/// whatever items landed in it — the same "row height follows its own
/// content" rule a table's own rows already use (`measure_table`).
///
/// **A multi-row-spanning named area's own height is attributed
/// entirely to its *first* row** (`by_row` below buckets a placement
/// under its own `row_start` only) rather than distributed across every
/// row it spans the way real CSS Grid would — the same kind of
/// single-row-attribution simplification a table's own rowspan would
/// need and doesn't have at all (`TODO.txt`'s own table-layout entry).
/// Not reachable by the real CSS this feature was built against (every
/// named area there spans multiple *columns*, never multiple rows,
/// confirmed by reading the actual fetched Wikipedia stylesheet) — a
/// real, narrower gap than silently pretending row-spanning works in
/// general, not a blocker for the columns-spanning case that's actually
/// used.
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
/// **No `grid-template-columns` declared at all** falls back to one
/// equal-`fr` implicit column per `grid-template-areas` column, when
/// one's declared (real CSS has no such fallback at all — named areas
/// without a matching explicit track list is formally an author error —
/// but it's the one honest single-pass answer when the *real* explicit
/// column sizes are themselves unresolvable, e.g. a `minmax()`/`fr`
/// mixed in with a content-based keyword this file still doesn't parse,
/// which is exactly the shape real Wikipedia CSS hits — see `TODO.txt`'s
/// own `display: grid` entry), or, with neither declared, a single
/// full-width implicit column (every item on its own row) — not real
/// CSS's own implicit-column auto-creation for an empty explicit grid
/// (each item would instead get its own *column* in a single row) — a
/// deliberately simplified, documented substitute for an edge case
/// outside this item's own explicit scope.
#[allow(clippy::too_many_arguments)]
pub(crate) fn measure_grid<'a>(
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

    let area_template = s.grid_template_areas;
    let areas_num_cols = area_template.map(grid_areas_num_cols).unwrap_or(0);

    // No `grid-template-columns` at all → one equal-`fr` implicit column
    // per `grid-template-areas` column when one's declared, else a
    // single full-width implicit column (see this function's own docs
    // for why, rather than real CSS's own implicit-column-per-item
    // fallback).
    let (mut col_tracks, fit_range): (Vec<GridTrackSpec>, Option<(usize, usize)>) = match s.grid_template_columns {
        Some(cols) => expand_auto_repeat(cols, entry.content_w, s.column_gap, s.font_size, s.root_font_size),
        None => (vec![GridTrackSpec::Fr(1.0); areas_num_cols.max(1)], None),
    };
    let num_cols = col_tracks.len().max(1);

    // Placement: an item whose own `grid-area` names one of `grid-
    // template-areas`'s own areas goes exactly there (`occupied` marks
    // every cell it covers so auto-placement, just below, never reuses
    // one); everything else — no `grid-area` at all, or one that names
    // nothing in the template — falls through to the same auto-
    // placement cursor this function always had (`grid-auto-flow: row`,
    // document order, skipping any cell a named placement already
    // claimed).
    let template_rows = area_template.map(|a| a.len()).unwrap_or(0);
    let explicit_rows = template_rows.max(s.grid_template_rows.map(|t| t.len()).unwrap_or(0));
    let mut occupied: Vec<Vec<bool>> = vec![vec![false; num_cols]; template_rows];
    let mut placements: Vec<GridPlacement<'a>> = Vec::with_capacity(items.len());
    // Marks a rectangle occupied (growing the implicit rows) and records it.
    let place = |occupied: &mut Vec<Vec<bool>>,
                     placements: &mut Vec<GridPlacement<'a>>,
                     item: &'a LayoutBox,
                     r0: usize,
                     r1: usize,
                     c0: usize,
                     c1: usize| {
        while occupied.len() < r1 {
            occupied.push(vec![false; num_cols]);
        }
        for row in occupied.iter_mut().take(r1).skip(r0) {
            for cell in row.iter_mut().take(c1).skip(c0) {
                *cell = true;
            }
        }
        placements.push(GridPlacement { item, row_start: r0, col_start: c0, col_end: c1 });
    };
    let free = |occupied: &Vec<Vec<bool>>, r0: usize, r1: usize, c0: usize, c1: usize| {
        (r0..r1).all(|r| occupied.get(r).is_none_or(|row| row[c0..c1].iter().all(|o| !o)))
    };
    // Pass 1: named areas and fully-fixed numeric placements. Pass 2: one
    // fixed axis. Pass 3: auto-placement with the cursor.
    let mut one_axis: Vec<(&'a LayoutBox, AxisPlace, AxisPlace)> = Vec::new();
    let mut unplaced: Vec<(&'a LayoutBox, usize, usize)> = Vec::new();
    for &item in &items {
        let rect = item.style.grid_area.zip(area_template).and_then(|(name, areas)| grid_area_rect(areas, name));
        if let Some((row_start, row_end, col_start, col_end)) = rect {
            let col_end = col_end.min(num_cols).max(col_start + 1);
            place(&mut occupied, &mut placements, item, row_start, row_end, col_start, col_end);
            continue;
        }
        let st = &item.style;
        let cols = resolve_grid_axis(st.grid_column_start, st.grid_column_end, num_cols);
        let rows = resolve_grid_axis(st.grid_row_start, st.grid_row_end, explicit_rows);
        match (rows, cols) {
            (AxisPlace::Fixed(r0, r1), AxisPlace::Fixed(c0, c1)) => {
                let c0 = c0.min(num_cols - 1);
                let c1 = c1.min(num_cols).max(c0 + 1);
                place(&mut occupied, &mut placements, item, r0, r1, c0, c1);
            }
            (AxisPlace::Auto(rs), AxisPlace::Auto(cs)) => unplaced.push((item, rs, cs.min(num_cols))),
            _ => one_axis.push((item, rows, cols)),
        }
    }
    for (item, rows, cols) in one_axis {
        match (rows, cols) {
            (AxisPlace::Fixed(r0, r1), AxisPlace::Auto(cs)) => {
                let cs = cs.min(num_cols);
                let c0 = (0..=num_cols - cs).find(|&c| free(&occupied, r0, r1, c, c + cs)).unwrap_or(0);
                place(&mut occupied, &mut placements, item, r0, r1, c0, c0 + cs);
            }
            (AxisPlace::Auto(rs), AxisPlace::Fixed(c0, c1)) => {
                let c0 = c0.min(num_cols - 1);
                let c1 = c1.min(num_cols).max(c0 + 1);
                let mut r0 = 0;
                while !free(&occupied, r0, r0 + rs, c0, c1) {
                    r0 += 1;
                }
                place(&mut occupied, &mut placements, item, r0, r0 + rs, c0, c1);
            }
            _ => unreachable!(),
        }
    }
    let mut cursor = (0usize, 0usize);
    for (item, rs, cs) in unplaced {
        loop {
            let (r, c) = cursor;
            if c + cs <= num_cols && free(&occupied, r, r + rs, c, c + cs) {
                place(&mut occupied, &mut placements, item, r, r + rs, c, c + cs);
                cursor = if c + cs < num_cols { (r, c + cs) } else { (r + 1, 0) };
                break;
            }
            cursor = if c + 1 < num_cols { (r, c + 1) } else { (r + 1, 0) };
        }
    }
    let num_rows = occupied.len();

    // Content-sized (`auto`/`min-content`/`max-content`) column tracks
    // get their `(min, max)` from the single-column items placed in
    // them (`cell_natural_width`); multi-column spans don't contribute.
    let content_sized = |t: &GridTrackSpec| {
        matches!(
            t,
            GridTrackSpec::Auto
                | GridTrackSpec::MinContent
                | GridTrackSpec::MaxContent
                | GridTrackSpec::MinMax(_, TrackBreadth::Auto | TrackBreadth::MinContent | TrackBreadth::MaxContent)
                | GridTrackSpec::MinMax(TrackBreadth::Auto | TrackBreadth::MinContent | TrackBreadth::MaxContent, _)
        )
    };
    // `auto-fit`: repetitions with no item collapse to zero width (their
    // gaps are not collapsed — simplification).
    if let Some((a, b)) = fit_range {
        for c in a..b.min(num_cols) {
            if !placements.iter().any(|p| p.col_start < c + 1 && c < p.col_end) {
                col_tracks[c] = GridTrackSpec::Len(Length::Px(0.0));
            }
        }
    }
    let col_tracks = &col_tracks[..];
    let mut col_content = vec![(0.0f32, 0.0f32); num_cols];
    for p in &placements {
        if p.col_end - p.col_start == 1 && col_tracks.get(p.col_start).is_some_and(content_sized) {
            let (mn, mx) = cell_natural_width(p.item, font_system, next_id, cache, deadline, incomplete);
            let c = &mut col_content[p.col_start];
            c.0 = c.0.max(mn);
            c.1 = c.1.max(mx);
        }
    }
    let col_widths =
        resolve_grid_tracks(col_tracks, &col_content, entry.content_w, s.column_gap, s.font_size, s.root_font_size);
    let mut col_x = Vec::with_capacity(num_cols);
    let mut cx = entry.content_x;
    for w in &col_widths {
        col_x.push(cx);
        cx += w + s.column_gap;
    }

    // Bucketed by `row_start` (see `GridPlacement`'s own docs for why a
    // multi-row span's own height only ever counts toward this one row)
    // and sorted by `col_start` within a row — document order alone
    // doesn't already give left-to-right order once named placement can
    // reorder items relative to their source position, unlike the
    // simpler pre-`grid-template-areas` auto-placement this generalizes.
    let mut by_row: Vec<Vec<&GridPlacement<'a>>> = vec![Vec::new(); num_rows];
    for p in &placements {
        by_row[p.row_start].push(p);
    }
    for row in &mut by_row {
        row.sort_by_key(|p| p.col_start);
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
    for (r, row_placements) in by_row.iter().enumerate() {
        // An explicit row track's own fixed size always applies;
        // `%`/`fr` only when there's a real basis to resolve against —
        // see this function's own docs for why that's the deliberate
        // cut, not a guess. A `minmax()` row track's own `Fr` max gets
        // the same no-basis treatment as a plain `Fr`; its `Len` max
        // (`minmax`'s floor is never used for a row — see `resolve_
        // grid_tracks`'s own docs for why the *column* axis is where
        // that matters) resolves exactly like a plain fixed/`%` track.
        let explicit_h = s.grid_template_rows.and_then(|tracks| tracks.get(r)).and_then(|t| match *t {
            GridTrackSpec::Len(Length::Percent(p)) => containing_height.map(|h| h * p),
            GridTrackSpec::Len(l) => Some(l.resolve(s.font_size, s.root_font_size)),
            GridTrackSpec::Fr(_)
            | GridTrackSpec::AutoRepeat { .. }
            | GridTrackSpec::Auto
            | GridTrackSpec::MinContent
            | GridTrackSpec::MaxContent
            | GridTrackSpec::MinMax(
                _,
                TrackBreadth::Fr(_) | TrackBreadth::Auto | TrackBreadth::MinContent | TrackBreadth::MaxContent,
            ) => None,
            GridTrackSpec::MinMax(_, TrackBreadth::Len(Length::Percent(p))) => containing_height.map(|h| h * p),
            GridTrackSpec::MinMax(_, TrackBreadth::Len(l)) => Some(l.resolve(s.font_size, s.root_font_size)),
        });
        let mut row_h = explicit_h.unwrap_or(0.0);
        let mut row_laid = Vec::with_capacity(row_placements.len());
        for p in row_placements {
            let cw: f32 =
                col_widths[p.col_start..p.col_end].iter().sum::<f32>() + s.column_gap * (p.col_end - p.col_start - 1) as f32;
            let lc = measure(
                p.item, font_system, col_x[p.col_start], cursor_y, cw, entry.child_containing_height, next_id, cache,
                deadline, incomplete, &mut own_pending, &no_floats, None,
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
        entry.border_x + s.border_left(),
        entry.border_y + s.border_top(),
        (entry.border_w - s.border_h()).max(0.0),
        (border_h - s.border_v()).max(0.0),
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
        ink: Ink::UNKNOWN,
        sticky: None,
        scroll: make_scroll_info(s.overflow, next_id, content_h, border_h, entry.extra_h),
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
    // The root's `h` is what `redraw` clamps scrolling to. With a definite
    // initial-containing-block height an `html{height:100%}` box ends at
    // the window bottom while its content overflows, so grow `h` to the
    // lowest in-flow descendant edge.
    let extent = in_flow_bottom(&laid);
    if extent > laid.y + laid.h {
        laid.h = extent - laid.y;
    }
    compute_ink(&mut laid);
    laid
}

/// Lowest bottom edge (document space) of `l` and its in-flow
/// descendants. A box that clips its overflow contributes only its own
/// edge; out-of-flow (`positioned`) boxes are ignored.
pub(crate) fn in_flow_bottom(l: &Laid) -> f32 {
    let mut bottom = l.y + l.h;
    if l.style.overflow != Overflow::Visible {
        return bottom;
    }
    match &l.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            for c in children {
                bottom = bottom.max(in_flow_bottom(c));
            }
        }
        LaidContent::MixedInline(items) => {
            for item in items {
                if let MixedPositioned::Embedded(e) = item {
                    bottom = bottom.max(in_flow_bottom(e));
                }
            }
        }
        LaidContent::Text(_) | LaidContent::Image(_) => {}
    }
    bottom
}
