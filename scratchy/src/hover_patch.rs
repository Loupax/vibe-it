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
            spacer: n.spacer,
            list_item: n.style.list_item || element_tag(h) == "li",
            pseudo,
        })
    }
}

/// Whether some `animation` of `s` names existing, non-empty keyframes (many
/// pages declare `animation-*` longhands everywhere without running anything).
fn animated(s: &ComputedStyle, vw: f32) -> bool {
    let Some(a) = s.animations else { return false };
    let Some(t) = a.table() else { return true };
    t.resolve().iter().any(|a| a.name.as_deref().is_some_and(|n| lookup_keyframes(n, vw).is_some_and(|k| !k.is_empty())))
}

/// One affected element's paint values.
#[derive(Clone, Copy)]
struct ElPaint {
    /// Its own text spans and its own box.
    own: (SpanPaint, BoxPaint),
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
pub(crate) fn paint_patch(
    document: &Handle,
    laid: &mut Laid,
    cache: &mut ShapeCache,
    font_system: &mut FontSystem,
    changes: &[MatchChange],
    old: &InteractionState,
    new: &InteractionState,
) -> Result<(), &'static str> {
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
) -> Result<(), &'static str> {
    if changes_effect(rules, changes) != Effect::Paint {
        return Err("a changed rule is not paint-only");
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
    // Their text spans carry no element (`node` 0): a list marker, a form control's text.
    if affected.iter().any(|h| matches!(element_tag(h), "li" | "input" | "textarea" | "select" | "button" | "option")) {
        return Err("a list item or form control (text not tied to its element)");
    }
    let mut paints: Paints = HashMap::new();
    set_interaction(old);
    let mut chain = Chain::new(document, rules, vw);
    for h in &affected {
        paints.insert(node_id(h), (chain.paints(h), None));
    }
    for c in changes {
        set_kept_match(&c.handle, c.slot, rules, vw, c.new.clone());
    }
    set_interaction(new);
    let mut chain = Chain::new(document, rules, vw);
    if paints.values().any(|p| p.0.is_some_and(|p| p.list_item)) {
        return Err("a list item (its marker is not tied to its element)");
    }
    for h in &affected {
        let p = chain.paints(h);
        if let Some(e) = paints.get_mut(&node_id(h)) {
            e.1 = p;
        }
    }
    // Boxes: the old values must be what the tree holds (checked for all
    // before anything is written).
    check_boxes(laid, &paints)?;
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
    // Commit.
    patch_boxes(laid, &paints);
    for (id, sp) in staged {
        cache.insert(id, Arc::new(Mutex::new(sp)));
    }
    Ok(())
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
