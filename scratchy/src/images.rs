use super::*;

// ---------- images ----------

/// Decodes a `data:` URI's base64 payload into raw bytes — the only
/// `<img src>` form this engine understands (see TODO.txt: no
/// networking, and no local file reads either, until that's revisited
/// — `snag` already rewrites real pages' `<img src>`s into exactly this
/// form). Returns `None` for anything else: a non-`data:` scheme, a
/// non-base64 payload (the alternate, URL-encoded `data:` text form —
/// not what `snag` emits), or a malformed one.
pub(crate) fn decode_data_uri(src: &str) -> Option<Vec<u8>> {
    let rest = src.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    if !meta.split(';').any(|p| p.eq_ignore_ascii_case("base64")) {
        // Non-base64 payloads are percent-encoded text (the usual form for
        // `data:image/svg+xml,...`).
        let raw = payload.as_bytes();
        let mut out = Vec::with_capacity(raw.len());
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == b'%' && i + 2 < raw.len() && let Ok(h) = u8::from_str_radix(&payload[i + 1..i + 3], 16) {
                out.push(h);
                i += 3;
            } else {
                out.push(raw[i]);
                i += 1;
            }
        }
        return Some(out);
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(payload.trim()).ok()
}

/// True when `bytes` look like an SVG document (optional BOM/whitespace,
/// then `<svg`, `<?xml` or `<!DOCTYPE svg`).
pub(crate) fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(512)];
    let head = head.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(head);
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start().to_ascii_lowercase();
    t.starts_with("<svg") || t.starts_with("<?xml") || t.starts_with("<!doctype svg")
}

/// Intrinsic size of an SVG: usvg's resolved `width`/`height` (else
/// `viewBox`); the CSS default replaced-element size, 300x150, when the
/// document declares neither (usvg then fails to size it).
pub(crate) fn svg_intrinsic_size(tree: &resvg::usvg::Tree) -> (u32, u32) {
    let size = tree.size().to_int_size();
    (size.width().max(1), size.height().max(1))
}

pub(crate) fn parse_svg(bytes: &[u8]) -> Option<resvg::usvg::Tree> {
    let opt = resvg::usvg::Options::default();
    // usvg falls back to 100x100 when the root has no size at all; CSS
    // says 300x150, so give such a root those dimensions up front.
    if let Ok(text) = std::str::from_utf8(bytes) {
        let lower = text.to_ascii_lowercase();
        if let Some(at) = lower.find("<svg") {
            let tag_end = lower[at..].find('>').map_or(lower.len(), |e| at + e);
            let tag = &lower[at..tag_end];
            if !tag.contains("width") && !tag.contains("height") && !tag.contains("viewbox") {
                let patched = format!("{} width=\"300\" height=\"150\"{}", &text[..at + 4], &text[at + 4..]);
                return resvg::usvg::Tree::from_data(patched.as_bytes(), &opt).ok();
            }
        }
    }
    resvg::usvg::Tree::from_data(bytes, &opt).ok()
}

/// Rasterizes an SVG document to `target` pixels (`None` = its intrinsic
/// size), un-premultiplying to RGBA. A different target scales the
/// vector drawing (not a bitmap), so it stays sharp.
pub(crate) fn rasterize_svg(bytes: &[u8], target: Option<(u32, u32)>) -> Option<DecodedImage> {
    let tree = parse_svg(bytes)?;
    let (iw, ih) = svg_intrinsic_size(&tree);
    let (width, height) = target.unwrap_or((iw, ih));
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return None;
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    let ts = resvg::tiny_skia::Transform::from_scale(width as f32 / iw as f32, height as f32 / ih as f32);
    resvg::render(&tree, ts, &mut pixmap.as_mut());
    let pixels = pixmap
        .pixels()
        .iter()
        .map(|p| {
            let c = p.demultiply();
            TextColor::rgba(c.red(), c.green(), c.blue(), c.alpha())
        })
        .collect();
    Some(DecodedImage { width, height, pixels, svg: Some(Arc::new(bytes.to_vec())), mask: None })
}

pub(crate) fn decode_svg_bytes(bytes: &[u8]) -> Option<DecodedImage> {
    rasterize_svg(bytes, None)
}

pub(crate) type SvgSizedKey = (u64, u32, u32);

thread_local! {
    /// Sized SVG rasterizations, keyed by (hash of the document, w, h).
    pub(crate) static SVG_SIZED_CACHE: std::cell::RefCell<HashMap<SvgSizedKey, Option<Arc<DecodedImage>>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Re-rasterizes an SVG-backed `image` at the CSS box size when the
/// style gives a pixel width and/or height (the missing one follows the
/// intrinsic aspect ratio). Anything else is returned unchanged.
pub(crate) fn svg_at_css_size(image: Arc<DecodedImage>, style: &ComputedStyle) -> Arc<DecodedImage> {
    let Some(svg) = image.svg.clone() else { return image };
    let px = |l: Option<BoxLength>| match l {
        Some(BoxLength::Px(v)) if v >= 1.0 => Some(v),
        _ => None,
    };
    let (iw, ih) = (image.width as f32, image.height as f32);
    let (w, h) = match (px(style.width), px(style.height)) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * ih / iw),
        (None, Some(h)) => (h * iw / ih, h),
        (None, None) => return image,
    };
    let (w, h) = (w.round().max(1.0) as u32, h.round().max(1.0) as u32);
    if (w, h) == (image.width, image.height) {
        return image;
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    svg.hash(&mut hasher);
    let key = (hasher.finish(), w, h);
    SVG_SIZED_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 256 {
            c.clear();
        }
        c.entry(key).or_insert_with(|| rasterize_svg(&svg, Some((w, h))).map(Arc::new)).clone()
    })
    .unwrap_or(image)
}

/// Decodes raw image bytes (already extracted from a `data:` URI) via
/// the `image` crate, converting straight to this file's own packed-
/// `u32` pixel form so nothing downstream needs to know `image`'s types
/// exist at all. `None` for anything `image` can't decode — an
/// unsupported format (only PNG/JPEG/GIF/WebP are compiled in, see
/// `Cargo.toml`), or truncated/corrupt data.
pub(crate) fn decode_image_bytes(bytes: &[u8]) -> Option<DecodedImage> {
    if looks_like_svg(bytes) {
        return decode_svg_bytes(bytes);
    }
    let decoded = image::load_from_memory(bytes).ok()?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    let pixels = rgba.pixels().map(|p| TextColor::rgba(p[0], p[1], p[2], p[3])).collect();
    Some(DecodedImage { width, height, pixels, svg: None, mask: None })
}

/// An `<img width>`/`<img height>` HTML attribute, as a plain unitless
/// pixel integer (the only form the HTML spec gives these attributes —
/// no `em`/`%`/other CSS length syntax). Only consulted as a fallback
/// when the CSS cascade left the matching property `auto` (see
/// `build_img_box`) — a real UA attaches these as the lowest-priority
/// presentational hint, exactly as if `img { width: Npx }` were in the
/// UA stylesheet, so CSS always wins once any is declared.
pub(crate) fn img_size_attr(handle: &Handle, name: &str) -> Option<f32> {
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
pub(crate) fn resolve_img(handle: &Handle, mut style: ComputedStyle, image_cache: &HashMap<String, Arc<DecodedImage>>) -> (ComputedStyle, Option<Arc<DecodedImage>>) {
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
    let decoded = decoded.map(|d| svg_at_css_size(d, &style));
    (style, decoded)
}

/// Serializes an inline `<svg>` subtree back to standalone XML (html5ever
/// has already case-fixed `viewBox`, `linearGradient` etc.), making sure
/// the root carries the SVG (and xlink) namespace declarations a
/// standalone document needs, and substituting every `currentColor` with
/// the element's computed `color` (resvg has no inherited CSS `color` to
/// resolve it against).
pub(crate) fn serialize_inline_svg(handle: &Handle, color: TextColor) -> String {
    fn esc(out: &mut String, s: &str, attr: bool) {
        for c in s.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' if attr => out.push_str("&quot;"),
                _ => out.push(c),
            }
        }
    }
    fn walk(node: &Handle, out: &mut String, root: bool) {
        match &node.data {
            NodeData::Text { contents } => esc(out, &contents.borrow(), false),
            NodeData::Element { name, attrs, .. } => {
                let tag = name.local.as_ref();
                out.push('<');
                out.push_str(tag);
                let attrs = attrs.borrow();
                let (mut has_ns, mut has_xlink) = (false, false);
                for a in attrs.iter() {
                    let local = a.name.local.as_ref();
                    match a.name.prefix.as_ref().map(|p| p.as_ref()) {
                        Some(p) => {
                            has_xlink |= p == "xmlns" && local == "xlink";
                            out.push_str(&format!(" {p}:{local}=\""));
                        }
                        None => {
                            has_ns |= local == "xmlns";
                            out.push_str(&format!(" {local}=\""));
                        }
                    }
                    esc(out, a.value.as_ref(), true);
                    out.push('"');
                }
                if root {
                    if !has_ns {
                        out.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
                    }
                    if !has_xlink {
                        out.push_str(" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
                    }
                }
                out.push('>');
                for child in node.children.borrow().iter() {
                    walk(child, out, false);
                }
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
            _ => {}
        }
    }
    let mut xml = String::new();
    walk(handle, &mut xml, true);
    let replacement = if color.a() == 255 {
        format!("rgb({},{},{})", color.r(), color.g(), color.b())
    } else {
        format!("rgba({},{},{},{})", color.r(), color.g(), color.b(), color.a() as f32 / 255.0)
    };
    // ASCII lowercasing keeps byte offsets identical to `xml`.
    let lower = xml.to_ascii_lowercase();
    let needle = "currentcolor";
    let mut out = String::with_capacity(xml.len());
    let mut last = 0;
    while let Some(i) = lower[last..].find(needle) {
        out.push_str(&xml[last..last + i]);
        out.push_str(&replacement);
        last += i + needle.len();
    }
    out.push_str(&xml[last..]);
    out
}

/// Inline `<svg>` as a replaced element: the same style-size fallbacks as
/// `resolve_img` (CSS `width`/`height`, else the unitless HTML
/// attributes), then the serialized subtree is rasterized at the
/// resulting size. `None` when the markup doesn't parse as SVG.
pub(crate) fn resolve_inline_svg(handle: &Handle, mut style: ComputedStyle) -> (ComputedStyle, Option<Arc<DecodedImage>>) {
    if style.width.is_none() {
        style.width = img_size_attr(handle, "width").map(BoxLength::Px);
    }
    if style.height.is_none() {
        style.height = img_size_attr(handle, "height").map(BoxLength::Px);
    }
    let xml = serialize_inline_svg(handle, style.color);
    let decoded = decode_svg_bytes(xml.as_bytes()).map(Arc::new).map(|d| svg_at_css_size(d, &style));
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
pub(crate) fn build_img_box(
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
pub(crate) fn build_box_for_element(
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
    let background_image = resolve_background_image(handle, rules, viewport_width, style.custom_props);
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
    if element_tag(handle) == "svg" {
        let (style, decoded) = resolve_inline_svg(handle, style);
        let content = match decoded {
            Some(image) => BoxContent::Image(image),
            None => BoxContent::Inline(Vec::new()),
        };
        return LayoutBox { style, background_image, content };
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
    // `display: flex`/`inline-flex` lay their *content* out identically
    // — each direct child becomes a flex item (`build_flex_children`),
    // measured per `measure_flex` instead of normal block/inline flow —
    // differing only in their own *outer* placement (block-level vs.
    // inline-level, decided entirely by `collect_children`, not here).
    if matches!(style.display, Display::Flex | Display::InlineFlex) {
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
    let prefix = if element_tag(handle) == "li" || style.list_item {
        marker_prefix(handle, rules, &style, viewport_width)
    } else {
        None
    };
    let mut children =
        build_children_with_prefix(handle, rules, style.inherited(), prefix, own_decoration, href, viewport_width, image_cache);
    if style.text_overflow == TextOverflow::Ellipsis && style.overflow != Overflow::Visible {
        for c in children.iter_mut().filter(|c| matches!(c.content, BoxContent::Inline(_))) {
            c.style.text_overflow = TextOverflow::Ellipsis;
        }
    }
    LayoutBox { style, background_image, content: BoxContent::Blocks(children) }
}

/// Walks a `display: table` element's children for its rows (and any
/// directly-placed block content, e.g. a `caption`) — flattening through
/// `thead`/`tbody`/`tfoot`/`colgroup`/`col` (`Display::TableGroup`), which
/// carry no box of their own in this simplified table layout. A nested
/// `<table>` is not flattened into: it's not `TableRow`/`TableGroup`
/// display, so it falls to the `Block` arm and gets its own box via the
/// normal `build_box_for_element` call, which re-enters this same
/// table-building path independently for its own rows.
pub(crate) fn build_table_children(
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
                let background_image = resolve_background_image(child, rules, viewport_width, style.custom_props);
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
pub(crate) fn build_table_row_cells(
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
            let mut cell = build_box_for_element(child, rules, inherited, decoration, href, viewport_width, image_cache);
            let span = |name: &str, max: usize| {
                element_attr(child, name).and_then(|v| v.trim().parse::<usize>().ok()).map_or(1, |n| n.clamp(1, max))
            };
            cell.style.col_span = span("colspan", 1000);
            cell.style.row_span = span("rowspan", 65534);
            cells.push(cell);
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
/// A run of consecutive text nodes directly inside the container becomes
/// one anonymous block flex/grid item (CSS Flexbox §4); whitespace-only
/// runs are dropped. `position: absolute`/`fixed`
/// children are *not* filtered out here (unlike `build_table_
/// children`'s own `Display::None` skip) — they're still real children
/// of this returned list, but `measure_flex`/`measure_grid` themselves
/// (not this function) are what actually take them out of their own
/// respective algorithm, the same split `measure`'s own `BoxContent::
/// Blocks` arm already uses for an ordinary block's absolutely/fixed
/// positioned children.
pub(crate) fn build_flex_children(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    decoration: DecorationValue,
    href: Option<&'static str>,
    viewport_width: f32,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> Vec<LayoutBox> {
    let mut result = Vec::new();
    // Consecutive text nodes accumulate here and become one anonymous
    // flex item (CSS Flexbox §4); whitespace-only runs are dropped.
    let mut pending: Vec<InlineItem> = Vec::new();
    let flush = |pending: &mut Vec<InlineItem>, result: &mut Vec<LayoutBox>| {
        let has_text = pending.iter().any(|i| matches!(i, InlineItem::Text(t, _) if !t.trim().is_empty()));
        if has_text {
            result.push(LayoutBox {
                style: anonymous_style(inherited),
                background_image: None,
                content: BoxContent::Inline(std::mem::take(pending)),
            });
        } else {
            pending.clear();
        }
    };
    // `::before` / `::after` are real (blockified) flex items (Q19b).
    let push_pseudo = |which: &str, result: &mut Vec<LayoutBox>| {
        if let Some((mut st, text)) = generated_content(handle, rules, inherited, which, viewport_width, image_cache) {
            st.display = Display::Block;
            let mut none = Vec::new();
            push_generated((st, text), inherited, None, decoration, href, VerticalAlign::default(), &mut none, result);
        }
    };
    push_pseudo("before", &mut result);
    for child in handle.children.borrow().iter() {
        if let NodeData::Text { contents } = &child.data {
            let mut span: SpanStyle = inherited.into();
            span.text_decoration = decoration.deco;
            span.decoration_style = decoration.style;
            span.href = href;
            span.node = node_id(handle);
            pending.push(InlineItem::Text(
                inline_text(&contents.borrow(), inherited.white_space, inherited.text_transform),
                span,
            ));
            continue;
        }
        let NodeData::Element { .. } = &child.data else { continue };
        let tag = element_tag(child);
        if tag == "script" || tag == "style" {
            continue;
        }
        let style = resolve_style(child, rules, inherited, viewport_width);
        if style.display == Display::None {
            continue;
        }
        flush(&mut pending, &mut result);
        result.push(build_box_for_element(child, rules, inherited, decoration, href, viewport_width, image_cache));
    }
    flush(&mut pending, &mut result);
    push_pseudo("after", &mut result);
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
pub(crate) fn build_children(
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
pub(crate) fn build_children_with_prefix(
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
    let has_prefix = prefix.is_some();
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
    discard_if_collapsible_whitespace_only(&mut pending);
    if !pending.is_empty() {
        result.push(LayoutBox {
            style: anonymous_style(inherited),
            background_image: None,
            content: BoxContent::Inline(pending),
        });
    }
    apply_first_letter(handle, rules, inherited, viewport_width, has_prefix, &mut result);
    apply_first_line(handle, rules, inherited, viewport_width, &mut result);
    result
}

/// `::first-line` (Q19e2): records, on the first anonymous inline run, the
/// properties the pseudo's rules change relative to the element's own
/// style. The line's extent is only known at layout, so `measure`'s
/// `Inline` arm does the actual split (`apply_first_line_split`).
pub(crate) fn apply_first_line(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    viewport_width: f32,
    result: &mut [LayoutBox],
) {
    if !matches!(handle.data, NodeData::Element { .. })
        || !rules.iter().any(|r| {
            r.selectors.slice().iter().any(|s| matches!(s.pseudo_element(), Some(OurPseudoElement::Named("first-line"))))
        })
    {
        return;
    }
    let Some(first) = result.first_mut() else { return };
    if !matches!(first.content, BoxContent::Inline(_)) {
        return;
    }
    if matching_declarations_for(handle, rules, viewport_width, Some("first-line")).is_empty() {
        return;
    }
    let base: SpanStyle = inherited.into();
    let pseudo_style = resolve_style_for(handle, rules, inherited, viewport_width, Some("first-line"));
    let p: SpanStyle = pseudo_style.inherited().into();
    let diff = |changed: bool| changed;
    let fl = FirstLineStyle {
        color: diff(p.color != base.color).then_some(p.color),
        background_color: pseudo_style.background_color,
        font_size: diff(p.font_size != base.font_size).then_some(p.font_size),
        font_weight: diff(p.font_weight != base.font_weight).then_some(p.font_weight),
        font_style: diff(p.font_style != base.font_style).then_some(p.font_style),
        line_height: diff(p.line_height != base.line_height).then_some(p.line_height),
        word_spacing: diff(p.word_spacing != base.word_spacing).then_some(p.word_spacing),
    };
    first.style.first_line = Some(fl);
}

/// Splits `items` at byte offset `end` (into the concatenated text) and
/// restyles everything before it with `fl`.
pub(crate) fn apply_first_line_split(items: &[InlineItem], end: usize, fl: &FirstLineStyle) -> Vec<InlineItem> {
    let mut out = Vec::with_capacity(items.len() + 1);
    let mut pos = 0usize;
    for item in items {
        match item {
            InlineItem::Text(text, span) => {
                let len = text.len();
                if pos + len <= end {
                    let mut s = *span;
                    fl.apply(&mut s);
                    out.push(InlineItem::Text(text.clone(), s));
                } else if pos >= end {
                    out.push(InlineItem::Text(text.clone(), *span));
                } else {
                    let mut cut = end - pos;
                    while !text.is_char_boundary(cut) {
                        cut += 1;
                    }
                    let mut s = *span;
                    fl.apply(&mut s);
                    out.push(InlineItem::Text(text[..cut].to_string(), s));
                    out.push(InlineItem::Text(text[cut..].to_string(), *span));
                }
                pos += len;
            }
            InlineItem::Image { image, style, background_image } => {
                out.push(InlineItem::Image { image: image.clone(), style: *style, background_image: background_image.clone() });
            }
            InlineItem::Embedded(_) => return items_clone_fallback(items),
        }
    }
    out
}

/// `text-overflow: ellipsis` (Q21) for a single-line text-only run: when
/// the shaped line is wider than `width`, cuts the text and appends "…" so
/// the result fits, re-shaping into `shaped`. Returns the new items, or
/// `None` when nothing overflows or the run is not plain single-line text.
pub(crate) fn ellipsize_items(
    font_system: &mut FontSystem,
    items: &[InlineItem],
    width: f32,
    align: Option<CosmicAlign>,
    shaped: &mut ShapedParagraph,
) -> Option<Vec<InlineItem>> {
    let line_w = |b: &Buffer| b.layout_runs().map(|r| r.line_w).fold(0.0f32, f32::max);
    if line_w(&shaped.0) <= width + 0.5
        || !items.iter().all(|i| matches!(i, InlineItem::Text(t, _) if !t.contains('\n')))
    {
        return None;
    }
    let total: usize = items.iter().map(|i| if let InlineItem::Text(t, _) = i { t.len() } else { 0 }).sum();
    // Largest end byte such that the glyphs before it plus "…" fit.
    let ew = items
        .iter()
        .rev()
        .find_map(|i| if let InlineItem::Text(_, s) = i { Some(s.font_size * 0.9) } else { None })
        .unwrap_or(14.0);
    let limit = (width - ew).max(0.0);
    let mut end = shaped
        .0
        .layout_runs()
        .next()
        .map(|r| r.glyphs.iter().filter(|g| g.x + g.w <= limit).map(|g| g.end).max().unwrap_or(0))
        .unwrap_or(0)
        .min(total);
    for _ in 0..16 {
        let mut out: Vec<InlineItem> = Vec::new();
        let mut pos = 0usize;
        let mut last_style = None;
        for item in items {
            let InlineItem::Text(text, span) = item else { continue };
            if pos >= end {
                break;
            }
            let mut cut = (end - pos).min(text.len());
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            out.push(InlineItem::Text(text[..cut].trim_end_matches(char::is_whitespace).to_string(), *span));
            last_style = Some(*span);
            pos += text.len();
        }
        let style = last_style.or_else(|| items.iter().find_map(|i| if let InlineItem::Text(_, s) = i { Some(*s) } else { None }))?;
        out.push(InlineItem::Text("\u{2026}".to_string(), style));
        let reshaped = shape_spans(font_system, &out, width, true, align);
        let fits = line_w(&reshaped.0) <= width + 0.5;
        *shaped = reshaped;
        if fits || end == 0 {
            return Some(out);
        }
        end = (end.saturating_sub(1)).min(total);
        // step back to the previous char boundary of the concatenation
        // is handled by the `is_char_boundary` loop above
    }
    None
}

pub(crate) fn parse_overflow_keyword(input: &mut CssParser) -> Option<Overflow> {
    let ident = input.expect_ident().ok()?;
    if ident.eq_ignore_ascii_case("visible") {
        Some(Overflow::Visible)
    } else if ident.eq_ignore_ascii_case("hidden") || ident.eq_ignore_ascii_case("clip") {
        Some(Overflow::Hidden)
    } else if ident.eq_ignore_ascii_case("scroll") {
        Some(Overflow::Scroll)
    } else if ident.eq_ignore_ascii_case("auto") {
        Some(Overflow::Auto)
    } else {
        None
    }
}

pub(crate) fn items_clone_fallback(items: &[InlineItem]) -> Vec<InlineItem> {
    items.iter().filter_map(|i| match i {
        InlineItem::Text(t, s) => Some(InlineItem::Text(t.clone(), *s)),
        _ => None,
    }).collect()
}

/// `::first-letter` (Q19e): styles the first letter (with any leading
/// punctuation) of the element's first inline run by splitting it off
/// into its own text item carrying the pseudo's style. Applies only when
/// the first child box is an anonymous inline run starting with text
/// (after the `<li>` marker prefix, if any); `float` (drop caps) is not
/// honored.
pub(crate) fn apply_first_letter(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    viewport_width: f32,
    has_prefix: bool,
    result: &mut [LayoutBox],
) {
    if !matches!(handle.data, NodeData::Element { .. })
        || !rules.iter().any(|r| {
            r.selectors.slice().iter().any(|s| matches!(s.pseudo_element(), Some(OurPseudoElement::Named("first-letter"))))
        })
    {
        return;
    }
    let Some(LayoutBox { content: BoxContent::Inline(items), .. }) = result.first_mut() else { return };
    let idx = usize::from(has_prefix);
    let Some(InlineItem::Text(text, span)) = items.get(idx) else { return };
    let mut split = None;
    let mut seen_punct = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() && !seen_punct {
            continue;
        }
        if c.is_alphanumeric() {
            split = Some(i + c.len_utf8());
            break;
        }
        if c.is_whitespace() {
            return;
        }
        seen_punct = true;
    }
    let Some(end) = split else { return };
    let start = text.len() - text.trim_start().len();
    let matches = matching_declarations_for(handle, rules, viewport_width, Some("first-letter"));
    if matches.is_empty() {
        return;
    }
    let style = resolve_style_for(handle, rules, inherited, viewport_width, Some("first-letter"));
    let mut letter_span: SpanStyle = style.inherited().into();
    letter_span.background_color = style.background_color.or(span.background_color);
    letter_span.text_decoration = span.text_decoration;
    letter_span.decoration_style = span.decoration_style;
    letter_span.vertical_align = span.vertical_align;
    letter_span.href = span.href;
    let (lead, rest) = (text[..start].to_string(), text[end..].to_string());
    let letter = text[start..end].to_string();
    let orig = *span;
    let mut new_items = Vec::new();
    if !lead.is_empty() {
        new_items.push(InlineItem::Text(lead, orig));
    }
    new_items.push(InlineItem::Text(letter, letter_span));
    if !rest.is_empty() {
        new_items.push(InlineItem::Text(rest, orig));
    }
    items.splice(idx..=idx, new_items);
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
/// Empties `pending` when it holds nothing but collapsed inter-tag
/// whitespace, so the caller's "flush `pending` into an anonymous inline
/// box" step emits nothing. CSS 2.1 §9.2.2.1: "White space content that
/// would subsequently be collapsed away according to the 'white-space'
/// property does not generate any anonymous inline boxes." Without this,
/// every newline/indent between two block-level siblings became its own
/// full-height line. On a real Wikipedia article that was ~30% of the
/// page's total height, and the whole blank band above the title.
///
/// "Collapsible" is recognized by shape: the non-`pre` text path in
/// `collect_children` turns a whitespace-only text node into exactly
/// `" "` (`normalize_whitespace` gives `""`, then the trailing space is
/// appended). `white-space: pre` text is pushed verbatim (`"\n"`,
/// `"  "`, ...), so it is never equal to `" "` and keeps its line, as it
/// does in real CSS. The one miss is a `pre` text node that is exactly
/// one space and nothing else, which renders as a blank line in a real
/// browser and is dropped here. Any `Image`/`Embedded` item, or a list
/// marker prefix (real text), keeps the whole run.
pub(crate) fn discard_if_collapsible_whitespace_only(pending: &mut Vec<InlineItem>) {
    if pending.iter().all(|item| matches!(item, InlineItem::Text(t, _) if t == " ")) {
        pending.clear();
    }
}

thread_local! {
    /// Evaluated dynamic `content` (`attr()`, counters, quotes) per
    /// (element pointer, is-`::after`), filled by `precompute_generated`.
    pub(crate) static GENERATED_TEXT: std::cell::RefCell<HashMap<(usize, bool), String>> =
        std::cell::RefCell::new(HashMap::new());
}

thread_local! {
    /// `content` declared on a real (non-pseudo) element (Q19f): it
    /// replaces the element's children. Filled by `precompute_generated`.
    pub(crate) static ELEMENT_CONTENT: std::cell::RefCell<HashMap<usize, String>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Formats a counter value in a `counter()` style.
pub(crate) fn format_counter(n: i32, style: &str) -> String {
    fn alpha(n: i32, base: u8) -> String {
        if n <= 0 {
            return n.to_string();
        }
        let mut n = n as u32;
        let mut out = Vec::new();
        while n > 0 {
            n -= 1;
            out.push(base + (n % 26) as u8);
            n /= 26;
        }
        out.reverse();
        String::from_utf8(out).unwrap_or_default()
    }
    fn roman(n: i32) -> String {
        if !(1..4000).contains(&n) {
            return n.to_string();
        }
        let mut n = n;
        let mut out = String::new();
        for (v, r) in [
            (1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"),
            (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i"),
        ] {
            while n >= v {
                out.push_str(r);
                n -= v;
            }
        }
        out
    }
    match style {
        "none" => String::new(),
        "decimal-leading-zero" => format!("{n:02}"),
        "lower-alpha" | "lower-latin" => alpha(n, b'a'),
        "upper-alpha" | "upper-latin" => alpha(n, b'A'),
        "lower-roman" => roman(n),
        "upper-roman" => roman(n).to_uppercase(),
        "lower-greek" if (1..=24).contains(&n) => {
            char::from_u32('α' as u32 + (n as u32 - 1) + u32::from(n >= 18)).map(String::from).unwrap_or_default()
        }
        "disc" => "\u{2022}".into(),
        "circle" => "\u{25E6}".into(),
        "square" => "\u{25AA}".into(),
        _ => n.to_string(),
    }
}

/// Document-order pass (Q19b) that evaluates every `::before`/`::after`
/// `content` containing `attr()`, `counter()`/`counters()` or quote
/// keywords, into `GENERATED_TEXT`. Does nothing (no per-element cost)
/// unless some rule declares such a `content` or a counter property.
pub(crate) fn precompute_generated(document: &Handle, rules: &[StyleRule], viewport_width: f32) {
    GENERATED_TEXT.with(|g| g.borrow_mut().clear());
    ELEMENT_CONTENT.with(|g| g.borrow_mut().clear());
    let wanted = rules.iter().any(|r| {
        [&r.decls, &r.important].iter().any(|d| {
            matches!(d.content, Some(ContentDecl::Parts(_)))
                || d.counter_reset.is_some()
                || d.counter_increment.is_some()
                || d.counter_set.is_some()
                || d.quotes.is_some()
        })
    });
    // A `content` on a selector with no pseudo-element targets the
    // element itself (Q19f).
    let wants_element_content = rules.iter().any(|r| {
        (r.decls.content.is_some() || r.important.content.is_some())
            && r.selectors.slice().iter().any(|s| s.pseudo_element().is_none())
    });
    if !wanted && !wants_element_content {
        return;
    }
    struct State {
        counters: Vec<(String, i32, usize)>,
        quote_depth: usize,
        quotes: Vec<(String, String)>,
        element_content: bool,
    }
    fn quote(st: &State, open: bool, depth: usize) -> String {
        match st.quotes.get(depth.min(st.quotes.len().saturating_sub(1))) {
            Some((o, c)) => (if open { o } else { c }).clone(),
            None => String::new(),
        }
    }
    fn eval(parts: &[ContentPart], handle: &Handle, st: &mut State) -> String {
        let mut out = String::new();
        for p in parts {
            match p {
                ContentPart::Str(s) => out.push_str(s),
                ContentPart::Attr(n) => {
                    if let Some(v) = element_attr(handle, n) {
                        out.push_str(&v);
                    }
                }
                ContentPart::Counter { name, style } => {
                    let v = st.counters.iter().rev().find(|c| c.0 == *name).map_or(0, |c| c.1);
                    out.push_str(&format_counter(v, style));
                }
                ContentPart::Counters { name, sep, style } => {
                    let vals: Vec<String> =
                        st.counters.iter().filter(|c| c.0 == *name).map(|c| format_counter(c.1, style)).collect();
                    out.push_str(&if vals.is_empty() { format_counter(0, style) } else { vals.join(sep) });
                }
                ContentPart::OpenQuote => {
                    out.push_str(&quote(st, true, st.quote_depth));
                    st.quote_depth += 1;
                }
                ContentPart::CloseQuote => {
                    st.quote_depth = st.quote_depth.saturating_sub(1);
                    out.push_str(&quote(st, false, st.quote_depth));
                }
                ContentPart::NoOpenQuote => st.quote_depth += 1,
                ContentPart::NoCloseQuote => st.quote_depth = st.quote_depth.saturating_sub(1),
            }
        }
        out
    }
    fn walk(handle: &Handle, rules: &[StyleRule], vw: f32, depth: usize, st: &mut State) {
        let is_el = matches!(handle.data, NodeData::Element { .. });
        let key = Rc::as_ptr(handle) as usize;
        let saved_quotes = st.quotes.clone();
        if is_el {
            let own = matching_declarations_for(handle, rules, vw, None);
            // `display: none` elements generate no boxes: no counters, no
            // pseudos, nothing in the subtree.
            if matches!(cascade_css(&own, None, "display", |d| d.display), Cascaded::Value(Display::None)) {
                return;
            }
            if let Cascaded::Value(q) = cascade_css(&own, None, "quotes", |d| d.quotes.clone()) {
                st.quotes = q;
            }
            if st.element_content {
                let c = match cascade_css(&own, None, "content", |d| d.content.clone()) {
                    Cascaded::Value(ContentDecl::Text(t)) => Some(t),
                    Cascaded::Value(ContentDecl::Parts(p)) => Some(eval(&p, handle, st)),
                    _ => None,
                };
                if let Some(t) = c {
                    ELEMENT_CONTENT.with(|g| g.borrow_mut().insert(key, t));
                }
            }
            let tag = match &handle.data {
                NodeData::Element { name, .. } => name.local.to_string(),
                _ => String::new(),
            };
            let reset = match cascade_css(&own, None, "counter-reset", |d| d.counter_reset.clone()) {
                Cascaded::Value(ops) => ops,
                _ => Vec::new(),
            };
            let set = match cascade_css(&own, None, "counter-set", |d| d.counter_set.clone()) {
                Cascaded::Value(ops) => ops,
                _ => Vec::new(),
            };
            let mut inc = match cascade_css(&own, None, "counter-increment", |d| d.counter_increment.clone()) {
                Cascaded::Value(ops) => ops,
                _ => Vec::new(),
            };
            // Implicit `list-item` counter: lists start one, `li` counts.
            let mentions = |n: &str| reset.iter().chain(&set).chain(&inc).any(|c| c.0 == n);
            let mut reset = reset.clone();
            if matches!(tag.as_str(), "ol" | "ul") && !mentions("list-item") {
                reset.push(("list-item".into(), 0));
            }
            if tag == "li" && !mentions("list-item") {
                inc.push(("list-item".into(), 1));
            }
            for (name, v) in reset {
                st.counters.retain(|c| !(c.0 == name && c.2 == depth));
                st.counters.push((name, v, depth));
            }
            for (name, v) in set {
                match st.counters.iter_mut().rev().find(|c| c.0 == name) {
                    Some(c) => c.1 = v,
                    None => st.counters.push((name, v, depth)),
                }
            }
            for (name, v) in inc {
                match st.counters.iter_mut().rev().find(|c| c.0 == name) {
                    Some(c) => c.1 += v,
                    None => st.counters.push((name, v, depth)),
                }
            }
            let m = matching_declarations_for(handle, rules, vw, Some("before"));
            if let Cascaded::Value(ContentDecl::Parts(parts)) = cascade_css(&m, None, "content", |d| d.content.clone()) {
                let t = eval(&parts, handle, st);
                GENERATED_TEXT.with(|g| g.borrow_mut().insert((key, false), t));
            }
        }
        for child in handle.children.borrow().iter() {
            walk(child, rules, vw, depth + 1, st);
        }
        if is_el {
            let m = matching_declarations_for(handle, rules, vw, Some("after"));
            if let Cascaded::Value(ContentDecl::Parts(parts)) = cascade_css(&m, None, "content", |d| d.content.clone()) {
                let t = eval(&parts, handle, st);
                GENERATED_TEXT.with(|g| g.borrow_mut().insert((key, true), t));
            }
            st.counters.retain(|c| c.2 <= depth);
        }
        st.quotes = saved_quotes;
    }
    let mut st = State { counters: Vec::new(), quote_depth: 0, quotes: default_quotes(), element_content: wants_element_content };
    walk(document, rules, viewport_width, 0, &mut st);
}

pub(crate) fn collect_children(
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
    // `content` on the element itself replaces its children (Q19f).
    let own_content = ELEMENT_CONTENT.with(|g| g.borrow().get(&(Rc::as_ptr(handle) as usize)).cloned());
    if let Some(text) = own_content {
        if !text.is_empty() {
            let mut span: SpanStyle = inherited.into();
            span.background_color = bg;
            span.text_decoration = decoration.deco;
            span.decoration_style = decoration.style;
            span.vertical_align = valign;
            span.href = href;
            span.node = node_id(handle);
            pending.push(InlineItem::Text(text, span));
        }
        return;
    }
    let gen_before = generated_content(handle, rules, inherited, "before", viewport_width, image_cache);
    if let Some(g) = gen_before {
        push_generated(g, inherited, bg, decoration, href, valign, pending, result);
    }
    collect_real_children(
        handle, rules, inherited, bg, decoration, href, valign, pending, result, viewport_width, image_cache,
    );
    let gen_after = generated_content(handle, rules, inherited, "after", viewport_width, image_cache);
    if let Some(g) = gen_after {
        push_generated(g, inherited, bg, decoration, href, valign, pending, result);
    }
}

/// The `::before`/`::after` box of `handle` (Q19): its resolved style and
/// its string content, or `None` when no rule gives it a string `content`
/// (none/normal/unsupported), or it computes to `display: none`.
pub(crate) fn generated_content(
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    which: &str,
    viewport_width: f32,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> Option<(ComputedStyle, GenContent)> {
    if !matches!(handle.data, NodeData::Element { .. }) {
        return None;
    }
    let matches = matching_declarations_for(handle, rules, viewport_width, Some(which));
    if matches.is_empty() {
        return None;
    }
    let content = match cascade_css(&matches, None, "content", |d| d.content.clone()) {
        Cascaded::Value(ContentDecl::Text(t)) => GenContent::Text(t),
        Cascaded::Value(ContentDecl::Parts(_)) => GenContent::Text(GENERATED_TEXT
            .with(|g| g.borrow().get(&(Rc::as_ptr(handle) as usize, which == "after")).cloned())?),
        Cascaded::Value(ContentDecl::Image(url)) => {
            let img = match decode_data_uri(&url) {
                Some(bytes) => decode_image_bytes(&bytes).map(Arc::new),
                None => image_cache.get(&*url).cloned(),
            }?;
            GenContent::Image(img)
        }
        _ => return None,
    };
    let style = resolve_style_for(handle, rules, inherited, viewport_width, Some(which));
    if style.display == Display::None {
        return None;
    }
    let content = match content {
        GenContent::Image(img) => GenContent::Image(svg_at_css_size(img, &style)),
        t => t,
    };
    Some((style, content))
}

/// What a generated box holds (Q19d).
pub(crate) enum GenContent {
    Text(String),
    Image(Arc<DecodedImage>),
}

/// Inserts a generated box into the in-progress inline run (inline
/// display) or as its own block / inline-block box.
pub(crate) fn push_generated(
    (mut style, content): (ComputedStyle, GenContent),
    inherited: InheritedStyle,
    bg: Option<TextColor>,
    decoration: DecorationValue,
    href: Option<&'static str>,
    valign: VerticalAlign,
    pending: &mut Vec<InlineItem>,
    result: &mut Vec<LayoutBox>,
) {
    let mut span: SpanStyle = style.inherited().into();
    span.background_color = style.background_color.or(bg);
    let deco = resolve_own_decoration(&style, decoration);
    span.text_decoration = deco.deco;
    span.decoration_style = deco.style;
    span.vertical_align = if style.vertical_align == VerticalAlign::Baseline { valign } else { style.vertical_align };
    span.href = href;
    span.node = style.node;
    let text = match content {
        GenContent::Text(t) => t,
        GenContent::Image(image) => return push_generated_image(image, style, pending, result, inherited),
    };
    let items = if text.is_empty() { Vec::new() } else { vec![InlineItem::Text(text.clone(), span)] };
    match style.display {
        Display::Block | Display::Table | Display::Flex | Display::Grid => {
            style.display = Display::Block;
            discard_if_collapsible_whitespace_only(pending);
            if !pending.is_empty() {
                result.push(LayoutBox {
                    style: anonymous_style(inherited),
                    background_image: None,
                    content: BoxContent::Inline(std::mem::take(pending)),
                });
            }
            result.push(LayoutBox { style, background_image: None, content: BoxContent::Inline(items) });
        }
        Display::InlineBlock | Display::InlineFlex => {
            style.display = Display::InlineBlock;
            pending.push(InlineItem::Embedded(Box::new(LayoutBox {
                style,
                background_image: None,
                content: BoxContent::Inline(items),
            })));
        }
        _ => pending.extend(items),
    }
}

/// `push_generated` for `content: url(...)`: an inline image item, or a
/// block replaced box, depending on the pseudo's display.
pub(crate) fn push_generated_image(
    image: Arc<DecodedImage>,
    mut style: ComputedStyle,
    pending: &mut Vec<InlineItem>,
    result: &mut Vec<LayoutBox>,
    inherited: InheritedStyle,
) {
    match style.display {
        Display::Block | Display::Table | Display::Flex | Display::Grid => {
            style.display = Display::Block;
            discard_if_collapsible_whitespace_only(pending);
            if !pending.is_empty() {
                result.push(LayoutBox {
                    style: anonymous_style(inherited),
                    background_image: None,
                    content: BoxContent::Inline(std::mem::take(pending)),
                });
            }
            result.push(LayoutBox { style, background_image: None, content: BoxContent::Image(image) });
        }
        _ => pending.push(InlineItem::Image { image, style, background_image: None }),
    }
}

pub(crate) fn collect_real_children(
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
                span.node = node_id(handle);
                // `white-space: pre` preserves the text byte-for-byte
                // (literal newlines, runs of spaces, no fabricated
                // trailing space) instead of collapsing it.
                let mut text = inline_text(&contents.borrow(), inherited.white_space, inherited.text_transform);
                // Collapsible spaces at the start of a line (right after a
                // `<br>`) are removed, so they never form a line of their
                // own (Q41).
                if inherited.white_space != WhiteSpace::Pre && matches!(pending.last(), Some(InlineItem::Text(t, _)) if t == BR_TEXT) {
                    text = text.trim_start_matches(' ').to_string();
                    if text.is_empty() {
                        continue;
                    }
                }
                pending.push(InlineItem::Text(text, span));
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
                if tag == "br" {
                    let mut span: SpanStyle = inherited.into();
                    span.background_color = bg;
                    span.vertical_align = valign;
                    span.href = child_href;
                    span.node = node_id(child);
                    pending.push(InlineItem::Text(BR_TEXT.to_string(), span));
                } else if tag == "img" && !matches!(style.display, Display::Block | Display::Table | Display::Flex | Display::Grid) {
                    let background_image = resolve_background_image(child, rules, viewport_width, style.custom_props);
                    let (img_style, decoded) = resolve_img(child, style, image_cache);
                    match decoded {
                        Some(image) => pending.push(InlineItem::Image { image, style: img_style, background_image }),
                        None => {
                            let alt = element_attr(child, "alt").map(|a| a.to_string()).unwrap_or_default();
                            let mut span: SpanStyle = inherited.into();
                            span.background_color = bg;
                            span.vertical_align = valign;
                            span.href = child_href;
                            span.node = node_id(child);
                            pending.push(InlineItem::Text(alt, span));
                        }
                    }
                } else if tag == "svg" && !matches!(style.display, Display::Block | Display::Table | Display::Flex | Display::Grid) {
                    // Inline `<svg>`: a replaced inline item like `<img>`.
                    let background_image = resolve_background_image(child, rules, viewport_width, style.custom_props);
                    let (svg_style, decoded) = resolve_inline_svg(child, style);
                    if let Some(image) = decoded {
                        pending.push(InlineItem::Image { image, style: svg_style, background_image });
                    }
                } else if matches!(style.display, Display::Block | Display::Table | Display::Flex | Display::Grid) {
                    discard_if_collapsible_whitespace_only(pending);
                    if !pending.is_empty() {
                        result.push(LayoutBox {
                            style: anonymous_style(inherited),
                            background_image: None,
                            content: BoxContent::Inline(std::mem::take(pending)),
                        });
                    }
                    result.push(build_box_for_element(child, rules, inherited, decoration, child_href, viewport_width, image_cache));
                } else if matches!(style.display, Display::InlineBlock | Display::InlineFlex) {
                    // Inline-level, but — unlike the plain-`Inline`
                    // flatten-and-recurse arm just below — keeps its own
                    // real box (background/border/padding/margin/width/
                    // height, children laid out normally or via flex,
                    // see `Display::InlineBlock`/`InlineFlex`'s own
                    // docs), pushed straight into the surrounding run
                    // with *no* flush of `pending` — unlike the block-
                    // level arm just above, this item has to keep
                    // sharing a line with whatever text/images/other
                    // inline-blocks already came before it. Measured for
                    // real inside `measure_mixed_inline`, not here.
                    let embedded =
                        build_box_for_element(child, rules, inherited, decoration, child_href, viewport_width, image_cache);
                    pending.push(InlineItem::Embedded(Box::new(embedded)));
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
pub(crate) const DEFAULT_CSS: &str = include_str!("default.css");

/// `:help`'s own page (see `src/help.html`, `App::show_help`) — embedded
/// the same way `DEFAULT_CSS` is, for the same reason: a single-binary
/// tool with no install-path/XDG-discovery problem for an asset that
/// ships with it.
pub(crate) const HELP_PAGE_HTML: &str = include_str!("help.html");

/// Finds the document's `<html>` element, if present — needed up front so
/// `rem` lengths can resolve against its real cascaded `font-size` (an
/// `html { font-size: ... }` override) instead of a fixed constant. A
/// shallow search would be enough for any well-formed document (html5ever's
/// `Document` node's only element child is `<html>`), but this walks the
/// whole tree so a malformed/fragment document still finds it if it's
/// nested unexpectedly, rather than silently falling back to the default.
pub(crate) fn find_html_element(handle: &Handle) -> Option<Handle> {
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
    build_layout_tree_with_resources(document, viewport_width, None, "", &HashMap::new())
}

/// Like `build_layout_tree`, but knows the window height too: `vh`/`vmin`/
/// `vmax` resolve against it and the document root gets a definite
/// height (the initial containing block), so `html{height:100%}` works.
pub fn build_layout_tree_sized(document: &Handle, viewport_width: f32, viewport_height: f32) -> LayoutBox {
    build_layout_tree_with_resources(document, viewport_width, Some(viewport_height), "", &HashMap::new())
}

/// Same as `build_layout_tree`, but also takes CSS fetched (via `wire`)
/// from this page's own `<link rel=stylesheet href>` references
/// (`extra_css` — concatenated into the same stylesheet source
/// `collect_style_css`'s own inline `<style>` text already feeds into,
/// so it cascades identically; see `App::extra_css`'s own docs for how
/// it gets populated) and a cache of already-decoded images fetched for
/// non-`data:` `<img src>` references (`image_cache`, consulted by
/// `resolve_img` — see `App::image_cache`'s own docs).
pub(crate) fn build_layout_tree_with_resources(
    document: &Handle,
    viewport_width: f32,
    viewport_height: Option<f32>,
    extra_css: &str,
    image_cache: &HashMap<String, Arc<DecodedImage>>,
) -> LayoutBox {
    set_viewport(viewport_width, viewport_height.unwrap_or(1080.0));
    let mut css_source = String::new();
    collect_style_css(document, &mut css_source);
    css_source.push('\n');
    css_source.push_str(extra_css);
    let html_handle = find_html_element(document);
    STATE_RULES.with(|m| m.set(0));
    VIEWPORT_H_READ.with(|m| m.set(false));
    // R3.1: a restyle of an unchanged document re-uses the parsed rules.
    // Parsing also records which state pseudo-classes appear
    // (`note_state_rule`), so that mask is cached with the rules.
    let rules: Rc<Vec<StyleRule>> = RULES_CACHE.with(|c| {
        if let Some((src, rules, mask)) = c.borrow().as_ref()
            && *src == css_source
        {
            note_state_rule(*mask);
            return rules.clone();
        }
        let mut rules = parse_stylesheet(DEFAULT_CSS);
        rules.extend(parse_stylesheet(&css_source));
        let rules = Rc::new(rules);
        *c.borrow_mut() = Some((css_source.clone(), rules.clone(), state_rules_mask()));
        rules
    });
    HOVER_KEYS.with(|k| *k.borrow_mut() = HoverKeys::build(&rules));
    RULE_INDEX.with(|ri| *ri.borrow_mut() = Some((rules.as_ptr() as usize, rules.len(), RuleIndex::build(&rules))));
    precompute_generated(document, &rules, viewport_width);
    // `var()` is resolved per element in `resolve_style` (Q13).
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
    let mut root_style = anonymous_style(root_inherited);
    root_style.icb_height = viewport_height;
    let tree = LayoutBox {
        style: root_style,
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
    };
    RULE_INDEX.with(|ri| *ri.borrow_mut() = None);
    if tree_reads_viewport_height(&tree, false) {
        note_viewport_height_read();
    }
    tree
}

pub(crate) fn collect_text_only(handle: &Handle, out: &mut String) {
    if let NodeData::Text { contents } = &handle.data {
        out.push_str(&contents.borrow());
    }
    for child in handle.children.borrow().iter() {
        collect_text_only(child, out);
    }
}

pub(crate) fn collect_style_css(handle: &Handle, out: &mut String) {
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
