//! QY53n2: the paint-only hover patch (REGRESSIONS.md R47, design point 4).
//!
//! A hover change whose matched-rule changes (`hover_match_diff`) only add or
//! remove PAINT-class rules (`Effect::Paint`: colours and text decoration)
//! moves no box and changes no glyph advance. Instead of a tree build (and a
//! shaping pass on the worker), the installed `Laid` tree and the affected
//! paragraphs of the shape cache are patched in place and the frame is
//! repainted. Anything this cannot do exactly returns `Err` before touching
//! anything, and the caller builds as before.
//!
//! QY53n3: OFFSET-class rules (`transform: translate`, `top`/`right`/`bottom`/
//! `left`) are patched too: the affected boxes' laid subtrees are shifted by the
//! change of their offset (`shift_laid_tree`, what `apply_relative_offset` and
//! `resolve_positioned` do at build time) and the ink extents are recomputed.
//! Only offsets that are exact without a containing block are handled (see
//! `offset_delta`); everything else returns `Err` and builds.
//!
//! The values are recomputed with the build's own rules: the style of every
//! affected element (the changed ones and their element descendants) is
//! resolved down its ancestor chain, with the build's kept match memo, once
//! under the old interaction state and once under the new one. The old-state
//! values must equal what the tree and the paragraphs hold (a check that this
//! derivation agrees with the build for exactly these elements); the
//! new-state values are written.
use super::*;

/// The inherited and ambient values a text span of an element's own text
/// is painted with (see `collect_real_nodes`).
#[derive(Clone, Copy, PartialEq)]
struct SpanPaint {
    color: TextColor,
    bg: Option<TextColor>,
    deco: TextDecoration,
    deco_style: DecorationStyle,
    hidden: bool,
    cursor: CursorStyle,
}

/// The paint-only fields of a box's `ComputedStyle`.
#[derive(Clone, Copy, PartialEq)]
struct BoxPaint {
    color: TextColor,
    background_color: Option<TextColor>,
    border_color: [Option<TextColor>; 4],
    outline_color: Option<TextColor>,
    text_decoration: Option<DecorationValue>,
    text_decoration_line: Option<DecorationLine>,
    text_decoration_style: Option<DecorationStyle>,
    text_decoration_color: Option<TextColor>,
    /// QY53n2c: painted (rounded background/border/clip), moves nothing.
    border_radius: BoxLength,
}

impl BoxPaint {
    fn of(s: &ComputedStyle) -> Self {
        BoxPaint {
            color: s.color,
            background_color: s.background_color,
            border_color: s.border_color,
            outline_color: s.outline_color,
            text_decoration: s.text_decoration,
            text_decoration_line: s.text_decoration_line,
            text_decoration_style: s.text_decoration_style,
            text_decoration_color: s.text_decoration_color,
            border_radius: s.border_radius,
        }
    }

    fn write(&self, s: &mut ComputedStyle) {
        s.color = self.color;
        s.background_color = self.background_color;
        s.border_color = self.border_color;
        s.outline_color = self.outline_color;
        s.text_decoration = self.text_decoration;
        s.text_decoration_line = self.text_decoration_line;
        s.text_decoration_style = self.text_decoration_style;
        s.text_decoration_color = self.text_decoration_color;
        s.border_radius = self.border_radius;
    }
}

/// One element's resolved style plus the ambient values the build carries
/// down to it.
#[derive(Clone, Copy)]
struct ChainNode {
    style: ComputedStyle,
    deco: DecorationValue,
    /// The background its own text spans get (ambient through inline elements).
    bg: Option<TextColor>,
    /// Flattened into its parent's inline run (no box of its own).
    flat: bool,
    /// Its children go through `collect_children` (block or inline flow).
    collects: bool,
    /// A flattened inline element with horizontal margin/padding/border: its
    /// background is also painted by `InlineItem::Spacer`s, which are not patched.
    spacer: bool,
}

/// Resolves elements down their ancestor chains the way the build does, each
/// element once.
struct Chain<'a> {
    rules: &'a [StyleRule],
    vw: f32,
    root: InheritedStyle,
    memo: HashMap<usize, Option<ChainNode>>,
}

impl<'a> Chain<'a> {
    fn new(document: &Handle, rules: &'a [StyleRule], vw: f32) -> Self {
        // As `build_layout_tree_with_resources`: `rem` against `<html>`'s own font size.
        let root_font_size = find_html_element(document)
            .map(|html| resolve_style(&html, rules, ROOT_STYLE, vw).font_size)
            .unwrap_or(ROOT_STYLE.font_size);
        Chain { rules, vw, root: InheritedStyle { root_font_size, ..ROOT_STYLE }, memo: HashMap::new() }
    }

    fn node(&mut self, h: &Handle) -> Option<ChainNode> {
        let key = node_id(h);
        if let Some(n) = self.memo.get(&key) {
            return *n;
        }
        let n = self.compute(h);
        self.memo.insert(key, n);
        n
    }

    fn compute(&mut self, h: &Handle) -> Option<ChainNode> {
        let NodeData::Element { .. } = &h.data else { return None };
        let parent = parent_handle(h)?;
        let (inherited, parent_deco, parent_bg, parent_collects) = match &parent.data {
            NodeData::Document => (self.root, DecorationValue::default(), None, true),
            NodeData::Element { .. } => {
                let p = self.node(&parent)?;
                (p.style.inherited(), p.deco, if p.flat { p.bg } else { None }, p.collects)
            }
            _ => return None,
        };
        // The base cascade: `evaluate` (running animations) records into the
        // animation report of the next build; an animated affected element is
        // refused in `paints`, and an animated ancestor's base values are
        // checked against the tree like everything else.
        let style = resolve_style_core(h, self.rules, inherited, self.vw, None, &[]);
        if style.display == Display::None || style.first_line.is_some() {
            return None;
        }
        let flat = parent_collects
            && !matches!(
                style.display,
                Display::Block
                    | Display::Table
                    | Display::Flex
                    | Display::Grid
                    | Display::InlineBlock
                    | Display::InlineFlex
                    | Display::TableRow
                    | Display::TableCell
                    | Display::TableGroup
            )
            && !matches!(element_tag(h), "br" | "img" | "svg");
        let collects = !matches!(
            style.display,
            Display::Flex | Display::InlineFlex | Display::Grid | Display::Table | Display::TableRow | Display::TableGroup
        );
        let spacer = flat
            && (style.margin.left.px() + style.padding.left.px() + style.border_left() > 0.0
                || style.margin.right.px() + style.padding.right.px() + style.border_width[1] > 0.0);
        Some(ChainNode {
            style,
            deco: resolve_own_decoration(&style, parent_deco),
            bg: if flat { style.background_color.or(parent_bg) } else { None },
            flat,
            collects,
            spacer,
        })
    }

    /// Paint values of `h` and its `::before`/`::after`; `None` when `h` is not
    /// rendered or is animated (its values follow the clock; an animated
    /// ancestor is fine: the old values are checked against the tree).
    fn paints(&mut self, h: &Handle) -> Option<ElPaint> {
        let n = self.node(h)?;
        if animated(&n.style, self.vw) {
            return None;
        }
        let span_of = |s: &ComputedStyle, bg: Option<TextColor>, deco: DecorationValue| {
            let i: SpanStyle = s.inherited().into();
            SpanPaint {
                color: i.color,
                bg,
                deco: deco.deco,
                deco_style: deco.style,
                hidden: i.visibility == Visibility::Hidden,
                cursor: i.cursor,
            }
        };
        // As `push_generated`: the pseudo inherits from `h`; its text gets its
        // own background over the run's, and `h`'s decoration as ambient.
        let run_bg = if n.flat { n.bg } else { None };
        let pseudo = ["before", "after"].map(|p| {
            // Only a pseudo-element that generates something (as `generated_content`).
            let m = matching_declarations_for(h, self.rules, self.vw, Some(p));
            let content = cascade_css(&m, None, "content", |d| d.content.clone());
            if m.is_empty() || !matches!(content, Cascaded::Value(ContentDecl::Text(_) | ContentDecl::Parts(_) | ContentDecl::Image(_))) {
                return None;
            }
            let ps = resolve_style_core(h, self.rules, n.style.inherited(), self.vw, Some(p), &[]);
            (ps.display != Display::None && !animated(&ps, self.vw)).then(|| {
                (span_of(&ps, ps.background_color.or(run_bg), resolve_own_decoration(&ps, n.deco)), BoxPaint::of(&ps))
            })
        });
        Some(ElPaint {
            own: (span_of(&n.style, n.bg, n.deco), BoxPaint::of(&n.style)),
            style: n.style,
            spacer: n.spacer,
            list_item: n.style.list_item || element_tag(h) == "li",
            pseudo,
        })
    }
}

/// QY53n2c: whether two resolved styles differ only in what the patch writes
/// or validates: paint fields, the offset (checked by `plan_moves`) and the
/// creation counter.
fn value_same(o: &ComputedStyle, n: &ComputedStyle) -> bool {
    let mut m = *n;
    BoxPaint::of(o).write(&mut m);
    m.translate = o.translate;
    m.inset = o.inset;
    m.tree_order = o.tree_order;
    m == *o
}

/// Whether some `animation` of `s` names existing, non-empty keyframes (many
/// pages declare `animation-*` longhands everywhere without running anything).
fn animated(s: &ComputedStyle, vw: f32) -> bool {
    // QANIMHLb: an element that declares transitions (or runs one) needs the real restyle, which
    // starts them; the patch would jump to the end value.
    if s.transitions.is_some() || crate::transition::has_running(s.node, s.pseudo) {
        return true;
    }
    let Some(a) = s.animations else { return false };
    let Some(t) = a.table() else { return true };
    t.resolve().iter().any(|a| a.name.as_deref().is_some_and(|n| lookup_keyframes(n, vw).is_some_and(|k| !k.is_empty())))
}

/// One affected element's paint values.
#[derive(Clone, Copy)]
struct ElPaint {
    /// Its own text spans and its own box.
    own: (SpanPaint, BoxPaint),
    /// QY53n2c: its whole resolved style (compared between the two states).
    style: ComputedStyle,
    spacer: bool,
    /// A list item: its marker is generated text too.
    list_item: bool,
    /// `::before`, `::after` (`None`: animated).
    pseudo: [Option<(SpanPaint, BoxPaint)>; 2],
}

/// Old and new paint values of each affected element (`None` = not rendered).
type Paints = HashMap<usize, (Option<ElPaint>, Option<ElPaint>)>;

fn add_subtree(h: &Handle, out: &mut Vec<Handle>, seen: &mut HashSet<usize>) {
    if matches!(h.data, NodeData::Element { .. }) && seen.insert(node_id(h)) {
        out.push(h.clone());
    }
    for c in h.children.borrow().iter() {
        add_subtree(c, out, seen);
    }
}

/// Picks the new value for something that now holds `held`, among
/// `(old, new)` candidates: exactly the candidates whose old value equals
/// `held` must agree on the new one.
fn pick<T: PartialEq + Copy>(held: impl Fn(&T) -> bool, candidates: impl Iterator<Item = (T, T)>) -> Result<T, &'static str> {
    let mut out: Option<T> = None;
    for (o, n) in candidates {
        if held(&o) {
            match out {
                Some(x) if x != n => return Err("an element's own and generated paint values cannot be told apart"),
                _ => out = Some(n),
            }
        }
    }
    out.ok_or("old paint values differ from the tree's")
}

/// Patches `laid` and `cache` for a hover change from `old` to `new` whose
/// matched-rule changes are `changes` (from `hover_match_diff`). On `Ok`
/// the tree and cache paint exactly as a full rebuild would, and the kept
/// match memo holds the new sets; on `Err` (the reason) nothing changed.
/// Expects `new` to be installed (`set_interaction`) and leaves it so.
/// `Ok(true)`: some box moved (the caller refreshes what depends on geometry).
pub(crate) fn paint_patch(
    document: &Handle,
    laid: &mut Laid,
    cache: &mut ShapeCache,
    font_system: &mut FontSystem,
    changes: &[MatchChange],
    old: &InteractionState,
    new: &InteractionState,
) -> Result<bool, &'static str> {
    if changes.is_empty() {
        return Err("nothing changed");
    }
    // `::before`/`::after` (slots 1, 2) are patched; `::first-line`,
    // `::first-letter` and `::marker` are not.
    if changes.iter().any(|c| c.slot > 2) {
        return Err("a ::first-line/::first-letter/::marker rule changed");
    }
    let out = with_kept_build(|rules, vw| {
        let r = patch_with_rules(document, laid, cache, font_system, changes, old, new, rules, vw);
        if r.is_err() {
            for c in changes {
                set_kept_match(&c.handle, c.slot, rules, vw, c.old.clone());
            }
        }
        r
    });
    set_interaction(new);
    out.unwrap_or(Err("no kept build"))
}

#[allow(clippy::too_many_arguments)]
fn patch_with_rules(
    document: &Handle,
    laid: &mut Laid,
    cache: &mut ShapeCache,
    font_system: &mut FontSystem,
    changes: &[MatchChange],
    old: &InteractionState,
    new: &InteractionState,
    rules: &Rc<Vec<StyleRule>>,
    vw: f32,
) -> Result<bool, &'static str> {
    let effect = changes_effect(rules, changes);
    if effect == Effect::Layout {
        return Err("a changed rule is not paint/offset-only");
    }
    // `Value`: the declared names say nothing; the resolved styles are compared below.
    let value = effect == Effect::Value;
    let offset = effect >= Effect::Offset;
    // An offset rule on a `::before`/`::after` box is not handled (its box style is not resolved here).
    if offset && changes.iter().any(|c| c.slot != 0 && changes_effect(rules, std::slice::from_ref(c)) != Effect::Paint) {
        return Err("an offset rule on a pseudo-element");
    }
    if rules_have_pseudo(rules, "first-line") || rules_have_pseudo(rules, "first-letter") {
        return Err("::first-line/::first-letter rules");
    }
    // The changed elements and every element below them (colour inherits,
    // decorations propagate).
    let mut affected = Vec::new();
    let mut seen = HashSet::new();
    for c in changes {
        add_subtree(&c.handle, &mut affected, &mut seen);
    }
    if affected.iter().any(|h| matches!(element_tag(h), "html" | "body")) {
        return Err("<html>/<body> (canvas background)");
    }
    let mut paints: Paints = HashMap::new();
    let mut bg_keys: HashMap<usize, [Option<String>; 2]> = HashMap::new();
    set_interaction(old);
    let mut chain = Chain::new(document, rules, vw);
    for h in &affected {
        paints.insert(node_id(h), (chain.paints(h), None));
        if value && let Some(n) = chain.node(h) {
            bg_keys.insert(node_id(h), background_image_key(h, rules, vw, n.style.custom_props));
        }
    }
    let mut old_styles: HashMap<usize, ComputedStyle> = HashMap::new();
    if offset {
        for c in changes.iter().filter(|c| c.slot == 0) {
            if let Some(n) = chain.node(&c.handle) {
                old_styles.insert(node_id(&c.handle), n.style);
            }
        }
    }
    for c in changes {
        set_kept_match(&c.handle, c.slot, rules, vw, c.new.clone());
    }
    set_interaction(new);
    let mut chain = Chain::new(document, rules, vw);
    for h in &affected {
        let p = chain.paints(h);
        if let Some(e) = paints.get_mut(&node_id(h)) {
            e.1 = p;
        }
        if value {
            let new_key = chain.node(h).map(|n| background_image_key(h, rules, vw, n.style.custom_props));
            if bg_keys.get(&node_id(h)) != new_key.as_ref() {
                return Err("a background/mask image changed");
            }
        }
    }
    // A list item's marker and a form control's text carry no element (`node` 0), so they
    // cannot be patched: allowed only when what they inherit (colour, background,
    // decoration, cursor, visibility) is the same in both states.
    for h in &affected {
        let Some((o, n)) = paints.get(&node_id(h)) else { continue };
        let tied = matches!(element_tag(h), "li" | "input" | "textarea" | "select" | "button" | "option")
            || o.is_some_and(|p| p.list_item)
            || n.is_some_and(|p| p.list_item);
        if tied {
            match (o, n) {
                (None, None) => {}
                (Some(o), Some(n)) if o.own.0 == n.own.0 => {}
                _ => return Err("a list item or form control whose text paint changes (text not tied to its element)"),
            }
        }
    }
    // An inline SVG is rasterised with its `color` (`currentColor`) at build time.
    for h in &affected {
        if element_tag(h) == "svg"
            && let Some((Some(o), Some(n))) = paints.get(&node_id(h))
            && o.own.1.color != n.own.1.color
        {
            return Err("an inline svg changed colour");
        }
    }
    if value {
        for p in paints.values() {
            match p {
                (None, None) => {}
                (Some(o), Some(n)) if value_same(&o.style, &n.style) => {}
                _ => return Err("a changed rule changes more than paint (resolved value)"),
            }
        }
    }
    // Boxes: the old values must be what the tree holds (checked for all
    // before anything is written).
    check_boxes(laid, &paints)?;
    let moves = if offset { plan_moves(laid, changes, &old_styles, &mut chain, &paints)? } else { Vec::new() };
    // Paragraphs: validate and re-lay copies first; commit nothing until all pass.
    let mut staged: Vec<(usize, ShapedParagraph)> = Vec::new();
    for (&id, entry) in cache.iter() {
        let sp = entry.lock().unwrap_or_else(|e| e.into_inner());
        if !sp.7.iter().any(|s| paints.contains_key(&s.1)) {
            continue;
        }
        // What the buffer holds per span: (colour, decoration).
        let mut held: HashMap<usize, (Option<TextColor>, TextDecoration)> = HashMap::new();
        for line in &sp.0.lines {
            for (_, a) in line.attrs_list().spans_iter() {
                held.entry(a.metadata).or_insert((a.color_opt, a.text_decoration));
            }
        }
        // QMEM3j: the first span's attrs are the line default (no span entry).
        for line in &sp.0.lines {
            let d = line.attrs_list().defaults();
            held.entry(d.metadata).or_insert((d.color_opt, d.text_decoration));
        }
        let mut spans: HashMap<usize, SpanPaint> = HashMap::new();
        for (i, &(_, node, tnode)) in sp.7.iter().enumerate() {
            let Some(entry) = paints.get(&node) else { continue };
            let (Some(o), Some(n)) = entry else {
                return Err("a span of an element that is not rendered or animated");
            };
            let is_held = |p: &SpanPaint| {
                sp.2.get(i) == Some(&p.bg)
                    && sp.4.get(i) == Some(&p.hidden)
                    && sp.5.get(i) == Some(&p.cursor)
                    && sp.6.get(i) == Some(&p.deco_style)
                    && held.get(&i).is_none_or(|&(c, d)| c == Some(p.color) && d == p.deco)
            };
            let next = if tnode != 0 {
                // The element's own DOM text.
                if (o.spacer || n.spacer) && o.own.0.bg != n.own.0.bg {
                    return Err("an inline element with padding/margin/border changed background");
                }
                pick(is_held, std::iter::once((o.own.0, n.own.0))).map_err(|_| "span: old values differ")?
            } else {
                // Generated text: `::before`/`::after` (a marker, `<br>`, alt
                // text or element `content` is not told apart: refused).
                if matches!(element_tag_of(&affected, node), Some("br" | "img")) {
                    return Err("generated text other than ::before/::after");
                }
                let cands = (0..2).filter_map(|k| Some((o.pseudo[k]?.0, n.pseudo[k]?.0)));
                pick(is_held, cands)?
            };
            if sp.4.get(i) != Some(&next.hidden) || sp.5.get(i) != Some(&next.cursor) {
                return Err("visibility or cursor changed");
            }
            spans.insert(i, next);
        }
        // Only spans whose values change are rewritten.
        spans.retain(|&i, p| {
            sp.2.get(i) != Some(&p.bg)
                || sp.6.get(i) != Some(&p.deco_style)
                || held.get(&i).is_some_and(|&(c, d)| c != Some(p.color) || d != p.deco)
        });
        if spans.is_empty() {
            continue;
        }
        let mut sp2: ShapedParagraph = sp.clone();
        drop(sp);
        for (&i, p) in &spans {
            sp2.2[i] = p.bg;
            sp2.6[i] = p.deco_style;
        }
        for line in sp2.0.lines.iter_mut() {
            let patch = |at: &mut Attrs| {
                if let Some(p) = spans.get(&at.metadata) {
                    at.color_opt = Some(p.color);
                    at.text_decoration = p.deco;
                }
            };
            let list = {
                let old_list = line.attrs_list();
                if !spans.contains_key(&old_list.defaults().metadata)
                    && !old_list.spans_iter().any(|(_, a)| spans.contains_key(&a.metadata))
                {
                    continue;
                }
                let mut d = old_list.defaults();
                patch(&mut d);
                let mut l = AttrsList::new(&d);
                for (r, a) in old_list.spans_iter() {
                    let mut at = a.as_attrs();
                    patch(&mut at);
                    l.add_span(r.clone(), &at);
                }
                l
            };
            line.set_attrs_list(list);
        }
        // Colour and decoration change no advance: same lines, same height.
        sp2.0.shape_until_scroll(font_system, false);
        let mut height = 0.0f32;
        for run in sp2.0.layout_runs() {
            height = height.max(run.line_top + run.line_height);
        }
        if height != sp2.1 {
            return Err("the re-laid paragraph changed height");
        }
        staged.push((id, sp2));
    }
    // Commit. Moves first: they can still be undone (a changed document
    // height is not handled).
    if !moves.is_empty() {
        let before = in_flow_bottom(laid);
        for m in &moves {
            if let Some(b) = find_box_mut(laid, m.node) {
                shift_laid_tree(b, m.dx, m.dy);
            }
        }
        let after = in_flow_bottom(laid);
        let limit = laid.y + laid.h;
        // The root's height was grown to the lowest in-flow edge at build time.
        if after != before && !(before < limit && after <= limit) {
            for m in moves.iter().rev() {
                if let Some(b) = find_box_mut(laid, m.node) {
                    shift_laid_tree(b, -m.dx, -m.dy);
                }
            }
            compute_ink(laid, cache);
            return Err("the document height depends on a moved box");
        }
        for m in &moves {
            if let Some(b) = find_box_mut(laid, m.node) {
                b.style.translate = m.translate;
                b.style.inset = m.inset;
            }
        }
        compute_ink(laid, cache);
    }
    patch_boxes(laid, &paints);
    for (id, sp) in staged {
        cache.insert(id, Arc::new(Mutex::new(sp)));
    }
    Ok(!moves.is_empty())
}

/// One box's new offset: the shift of its laid subtree and the style fields to store.
struct Move {
    node: usize,
    dx: f32,
    dy: f32,
    translate: Option<(TranslateAxis, TranslateAxis)>,
    inset: Insets,
}

/// What `plan_moves` reads of a laid box.
struct BoxInfo {
    w: f32,
    h: f32,
    translate: Option<(TranslateAxis, TranslateAxis)>,
    inset: Insets,
    position: Position,
    /// An inline-level box in a line (the build's inline layout places it itself).
    inline: bool,
}

fn collect_box_infos(l: &Laid, inline: bool, nodes: &HashSet<usize>, out: &mut HashMap<usize, Vec<BoxInfo>>) {
    if l.style.pseudo == 0 && nodes.contains(&l.style.node) {
        out.entry(l.style.node).or_default().push(BoxInfo {
            w: l.w,
            h: l.h,
            translate: l.style.translate,
            inset: l.style.inset,
            position: l.style.position,
            inline,
        });
    }
    match &l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().for_each(|c| collect_box_infos(c, false, nodes, out)),
        LaidContent::MixedInline(items) => {
            for it in items {
                if let MixedPositioned::Embedded(b) = it {
                    collect_box_infos(b, true, nodes, out);
                }
            }
        }
        LaidContent::Text(_) | LaidContent::Image(_) => {}
    }
    l.positioned.iter().for_each(|c| collect_box_infos(c, false, nodes, out));
}

/// Whether `find_box_mut` finds `node` below (or at) `l`.
fn has_box(l: &Laid, node: usize) -> bool {
    if l.style.pseudo == 0 && l.style.node == node {
        return true;
    }
    let in_content = match &l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().any(|c| has_box(c, node)),
        LaidContent::MixedInline(items) => items.iter().any(|it| matches!(it, MixedPositioned::Embedded(b) if has_box(b, node))),
        LaidContent::Text(_) | LaidContent::Image(_) => false,
    };
    in_content || l.positioned.iter().any(|c| has_box(c, node))
}

fn find_box_mut(l: &mut Laid, node: usize) -> Option<&mut Laid> {
    if l.style.pseudo == 0 && l.style.node == node {
        return Some(l);
    }
    match &mut l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => {
            if let Some(i) = c.iter().position(|c| has_box(c, node)) {
                return find_box_mut(&mut c[i], node);
            }
        }
        LaidContent::MixedInline(items) => {
            let at = items.iter().position(|it| matches!(it, MixedPositioned::Embedded(b) if has_box(b, node)));
            if let Some(i) = at
                && let MixedPositioned::Embedded(b) = &mut items[i]
            {
                return find_box_mut(b, node);
            }
        }
        LaidContent::Text(_) | LaidContent::Image(_) => {}
    }
    let i = l.positioned.iter().position(|c| has_box(c, node))?;
    find_box_mut(&mut l.positioned[i], node)
}

fn same_len(a: Option<BoxLength>, b: Option<BoxLength>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(BoxLength::Px(a)), Some(BoxLength::Px(b))) | (Some(BoxLength::Percent(a)), Some(BoxLength::Percent(b))) => a == b,
        _ => false,
    }
}

fn same_insets(a: &Insets, b: &Insets) -> bool {
    same_len(a.top, b.top) && same_len(a.right, b.right) && same_len(a.bottom, b.bottom) && same_len(a.left, b.left)
}

/// A pixel inset (`None` = auto); a percentage or `calc()` needs the containing block.
fn px_inset(b: Option<BoxLength>) -> Result<Option<f32>, &'static str> {
    match b {
        None => Ok(None),
        Some(BoxLength::Px(v)) => Ok(Some(v)),
        Some(_) => Err("a percentage/calc() inset"),
    }
}

fn translate_of(t: Option<(TranslateAxis, TranslateAxis)>, w: f32, h: f32) -> (f32, f32) {
    t.map_or((0.0, 0.0), |(x, y)| (x.px + x.frac * w, y.px + y.frac * h))
}

/// The shift of a box of size `w` x `h` whose style changes from `o` to `n`
/// (the same numbers `apply_relative_offset` / `resolve_positioned` add), or
/// why it cannot be derived without the containing block.
fn offset_delta(o: &ComputedStyle, n: &ComputedStyle, w: f32, h: f32) -> Result<(f32, f32), &'static str> {
    if o.position != n.position {
        return Err("position changed");
    }
    let (to, tn) = (translate_of(o.translate, w, h), translate_of(n.translate, w, h));
    let (mut dx, mut dy) = (tn.0 - to.0, tn.1 - to.1);
    if same_insets(&o.inset, &n.inset) {
        return Ok((dx, dy));
    }
    match o.position {
        // The insets are ignored.
        Position::Static => {}
        Position::Relative => {
            let g = |s: &ComputedStyle| -> Result<(f32, f32), &'static str> {
                let (l, r) = (px_inset(s.inset.left)?, px_inset(s.inset.right)?);
                let (t, b) = (px_inset(s.inset.top)?, px_inset(s.inset.bottom)?);
                Ok((l.unwrap_or(0.0) - r.unwrap_or(0.0), t.unwrap_or(0.0) - b.unwrap_or(0.0)))
            };
            let (go, gn) = (g(o)?, g(n)?);
            dx += gn.0 - go.0;
            dy += gn.1 - go.1;
        }
        Position::Absolute => {
            // An explicit width: the insets do not stretch or shrink-wrap the box.
            // No `auto` margins (they centre between two insets).
            if o.width.is_none() || n.width.is_none() || o.margin_auto != (false, false) || o.margin_auto_v != (false, false) {
                return Err("an absolute box with auto width or auto margins");
            }
            // The edge the position is solved from: `left` wins over `right`, `top` over `bottom`.
            type L = Option<BoxLength>;
            let axis = |lo: L, ln: L, ro: L, rn: L| -> Result<f32, &'static str> {
                let (lo, ln) = (px_inset(lo)?, px_inset(ln)?);
                let (ro, rn) = (px_inset(ro)?, px_inset(rn)?);
                match (lo, ln) {
                    (Some(a), Some(b)) => Ok(b - a),
                    (None, None) => match (ro, rn) {
                        (Some(a), Some(b)) => Ok(-(b - a)),
                        (None, None) => Ok(0.0),
                        _ => Err("an absolute box changes the edge it is placed from"),
                    },
                    _ => Err("an absolute box changes the edge it is placed from"),
                }
            };
            dx += axis(o.inset.left, n.inset.left, o.inset.right, n.inset.right)?;
            dy += axis(o.inset.top, n.inset.top, o.inset.bottom, n.inset.bottom)?;
        }
        // Sticky and fixed boxes keep their own offset state.
        _ => return Err("an inset change on a sticky or fixed box"),
    }
    Ok((dx, dy))
}

/// The moves for the offset changes of `changes` (slot 0), validated against the
/// installed tree; `Err` before anything is written.
fn plan_moves(
    laid: &Laid,
    changes: &[MatchChange],
    old_styles: &HashMap<usize, ComputedStyle>,
    chain: &mut Chain,
    paints: &Paints,
) -> Result<Vec<Move>, &'static str> {
    // (handle, old style, new style) of each changed element whose offset state differs.
    let mut todo: Vec<(Handle, ComputedStyle, ComputedStyle)> = Vec::new();
    let mut seen = HashSet::new();
    for c in changes.iter().filter(|c| c.slot == 0) {
        let id = node_id(&c.handle);
        if !seen.insert(id) {
            continue;
        }
        let (Some(o), Some(n)) = (old_styles.get(&id).copied(), chain.node(&c.handle).map(|n| n.style)) else { continue };
        if o.translate == n.translate && same_insets(&o.inset, &n.inset) {
            continue;
        }
        todo.push((c.handle.clone(), o, n));
    }
    if todo.is_empty() {
        return Ok(Vec::new());
    }
    let nodes: HashSet<usize> = todo.iter().map(|t| node_id(&t.0)).collect();
    let mut infos = HashMap::new();
    collect_box_infos(laid, false, &nodes, &mut infos);
    let mut out = Vec::new();
    for (h, o, n) in todo {
        let id = node_id(&h);
        // Animated elements follow the clock (`paints` is `None` for them).
        if !paints.get(&id).is_some_and(|p| p.0.is_some() && p.1.is_some()) {
            return Err("an offset change on an element that is not rendered or animated");
        }
        // Positioned descendants were moved to the stacking context's list at
        // build time (`hoist_negative_z`) and their static positions follow
        // the box: not shifted with it here.
        let mut below = Vec::new();
        add_subtree(&h, &mut below, &mut HashSet::new());
        for d in below.iter().skip(1) {
            if chain.node(d).is_some_and(|c| c.style.position != Position::Static) {
                return Err("a positioned element below an offset change");
            }
        }
        let [b] = infos.get(&id).map(Vec::as_slice).unwrap_or(&[]) else {
            return Err("an offset change on an element without exactly one box");
        };
        if b.inline {
            return Err("an offset change on an inline-level box (the line layout places it)");
        }
        // The tree must hold the old offset state.
        if b.translate != o.translate || b.position != o.position || !same_insets(&b.inset, &o.inset) {
            return Err("an offset change whose old values differ from the tree's");
        }
        let (dx, dy) = offset_delta(&o, &n, b.w, b.h)?;
        out.push(Move { node: id, dx, dy, translate: n.translate, inset: n.inset });
    }
    Ok(out)
}


fn element_tag_of(affected: &[Handle], node: usize) -> Option<&str> {
    affected.iter().find(|h| node_id(h) == node).map(element_tag)
}

/// The new paint values for a box of an affected element holding `held`:
/// the element's own box (`slot` 0) or its `::before`/`::after` box (1, 2).
fn box_next(p: &(Option<ElPaint>, Option<ElPaint>), held: &BoxPaint, slot: u8) -> Result<BoxPaint, &'static str> {
    let (Some(o), Some(n)) = p else { return Err("a box of an element that is not rendered or animated") };
    let (o, n) = match slot {
        0 => (Some(o.own.1), Some(n.own.1)),
        1 | 2 => (o.pseudo[slot as usize - 1].map(|p| p.1), n.pseudo[slot as usize - 1].map(|p| p.1)),
        _ => return Err("a ::marker/::first-letter box"),
    };
    match (o, n) {
        (Some(o), Some(n)) if o == *held => Ok(n),
        (Some(_), Some(_)) => Err("a box's old paint values differ from the tree's"),
        _ => Err("a generated box that is animated or not generated"),
    }
}

/// Checks every box of an affected element.
fn check_boxes(laid: &Laid, paints: &Paints) -> Result<(), &'static str> {
    if let Some(p) = paints.get(&laid.style.node) {
        box_next(p, &BoxPaint::of(&laid.style), laid.style.pseudo)?;
    }
    match &laid.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().try_for_each(|c| check_boxes(c, paints))?,
        LaidContent::MixedInline(items) => {
            for it in items {
                if let MixedPositioned::Embedded(b) = it {
                    check_boxes(b, paints)?;
                }
            }
        }
        LaidContent::Text(_) | LaidContent::Image(_) => {}
    }
    laid.positioned.iter().try_for_each(|c| check_boxes(c, paints))
}

/// Writes what `check_boxes` checked.
fn patch_boxes(laid: &mut Laid, paints: &Paints) {
    if let Some(p) = paints.get(&laid.style.node)
        && let Ok(b) = box_next(p, &BoxPaint::of(&laid.style), laid.style.pseudo)
    {
        b.write(&mut laid.style);
    }
    match &mut laid.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter_mut().for_each(|c| patch_boxes(c, paints)),
        LaidContent::MixedInline(items) => {
            for it in items {
                if let MixedPositioned::Embedded(b) = it {
                    patch_boxes(b, paints);
                }
            }
        }
        LaidContent::Text(_) | LaidContent::Image(_) => {}
    }
    laid.positioned.iter_mut().for_each(|c| patch_boxes(c, paints));
}
