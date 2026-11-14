use super::*;

// ---------- CLI input ----------

pub(crate) enum Input {
    File(String),
    Stdin,
    /// The lone positional argument was an absolute `http`/`https` URL
    /// (`classify_input`), not a local path — `run()` fetches it live
    /// through `wire` the same way a clicked link or `:go` would,
    /// instead of requiring `snag url | scratchy` first. `snag` is
    /// still the only thing that can turn an arbitrary live page into a
    /// single self-contained bundle up front (inlined images, a synced
    /// stylesheet fetch before the very first paint — see its own
    /// NOTES.md) — this doesn't replace that, it just means a bare
    /// `scratchy https://example.com` (with `--socket` pointed at a
    /// running `wire`) no longer needs `snag` in the loop at all for
    /// the common case of just wanting to look at a page.
    Url(Url),
}

/// Disambiguates `parse_args`'s lone positional argument. `Url::parse`
/// alone isn't a strict enough test — it also happily parses things
/// like `C:\path` lookalikes and schemes scratchy has no fetch path for
/// at all (`file://`, `data:`, ...) — so the scheme is checked
/// explicitly rather than trusting "parsed successfully" to mean "is a
/// live URL scratchy can act on." Anything else, including a bare
/// relative path and an absolute one, stays `Input::File`, parsed by
/// `fs::read_to_string` exactly as before this existed.
pub(crate) fn classify_input(arg: &str) -> Input {
    match Url::parse(arg) {
        Ok(url) if url.scheme() == "http" || url.scheme() == "https" => Input::Url(url),
        _ => Input::File(arg.to_string()),
    }
}

/// `--socket <path>`, alongside the pre-existing `[file]` argument — an
/// explicit opt-in flag, not an auto-discovered default (see `wire/
/// TODO.txt`'s own "TOP PRIORITY" item for why): scratchy never guesses
/// where a `wire` daemon might be listening, and never starts one
/// itself. With no `--socket` at all, link navigation always falls
/// straight to `start_navigation`'s own print-the-request-to-stdout
/// fallback — see its own docs.
pub(crate) struct Args {
    pub(crate) input: Input,
    pub(crate) socket: Option<PathBuf>,
}

pub(crate) fn parse_args() -> Args {
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
        1 => classify_input(&rest[0]),
        _ => {
            eprintln!("usage: scratchy [--socket <path>] [file|url]");
            std::process::exit(1);
        }
    };
    Args { input, socket }
}

/// Charset of a `Content-Type` header value's `charset=` parameter, if
/// it names an encoding `encoding_rs` knows. WHATWG labels apply, so
/// `iso-8859-1`/`latin1` come back as windows-1252.
pub(crate) fn content_type_charset(content_type: &str) -> Option<&'static encoding_rs::Encoding> {
    let lower = content_type.to_ascii_lowercase();
    let idx = lower.find("charset")?;
    let rest = lower[idx + "charset".len()..].trim_start().strip_prefix('=')?;
    let label: String = rest
        .trim_start()
        .trim_start_matches(['"', '\''])
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
        .collect();
    encoding_rs::Encoding::for_label(label.as_bytes())
}

/// `<meta charset=...>` / `<meta http-equiv=content-type content="...;
/// charset=...">` within the first 1024 bytes. A UTF-16 label found
/// this way means UTF-8 (the bytes are demonstrably ASCII-compatible).
pub(crate) fn meta_charset(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let head = &bytes[..bytes.len().min(1024)];
    let lower: Vec<u8> = head.to_ascii_lowercase();
    let text = String::from_utf8_lossy(&lower);
    let mut from = 0;
    while let Some(i) = text[from..].find("<meta") {
        let start = from + i;
        let end = text[start..].find('>').map(|e| start + e).unwrap_or(text.len());
        let tag = &text[start..end];
        if let Some(enc) = content_type_charset(tag) {
            return Some(if enc == encoding_rs::UTF_16LE || enc == encoding_rs::UTF_16BE { encoding_rs::UTF_8 } else { enc });
        }
        from = end;
    }
    None
}

/// Encoding order: BOM, header charset (wire only), meta within the
/// first 1024 bytes, UTF-8.
pub(crate) fn sniff_encoding(bytes: &[u8], header: Option<&'static encoding_rs::Encoding>) -> &'static encoding_rs::Encoding {
    if let Some((enc, _)) = encoding_rs::Encoding::for_bom(bytes) {
        return enc;
    }
    header.or_else(|| meta_charset(bytes)).unwrap_or(encoding_rs::UTF_8)
}

/// Decodes a whole document to a `String` (BOM stripped, malformed
/// sequences replaced).
pub(crate) fn decode_html_bytes(bytes: &[u8], header: Option<&'static encoding_rs::Encoding>) -> String {
    let enc = sniff_encoding(bytes, header);
    enc.decode_with_bom_removal(bytes).0.into_owned()
}

/// Incremental bytes-to-UTF-8 transcoder for streaming stdin: buffers
/// until 1024 bytes (or EOF) to sniff, then decodes chunk by chunk.
pub(crate) struct StreamTranscoder {
    pub(crate) pending: Vec<u8>,
    pub(crate) decoder: Option<encoding_rs::Decoder>,
}

impl StreamTranscoder {
    pub(crate) fn new() -> Self {
        Self { pending: Vec::new(), decoder: None }
    }

    pub(crate) fn decode(&mut self, bytes: &[u8], last: bool) -> Vec<u8> {
        if self.decoder.is_none() {
            self.pending.extend_from_slice(bytes);
            if self.pending.len() < 1024 && !last {
                return Vec::new();
            }
            let enc = sniff_encoding(&self.pending, None);
            self.decoder = Some(enc.new_decoder_with_bom_removal());
            let buf = std::mem::take(&mut self.pending);
            return self.run(&buf, last);
        }
        self.run(bytes, last)
    }

    pub(crate) fn run(&mut self, bytes: &[u8], last: bool) -> Vec<u8> {
        let decoder = self.decoder.as_mut().expect("decoder set");
        let mut out = String::with_capacity(decoder.max_utf8_buffer_length(bytes.len()).unwrap_or(bytes.len() * 3 + 16));
        let _ = decoder.decode_to_string(bytes, &mut out, last);
        out.into_bytes()
    }
}

pub(crate) fn new_parser() -> Utf8LossyDecoder<Parser<RcDom>> {
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
pub(crate) struct Atom(pub(crate) String);

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
pub(crate) enum OurPseudoClass {
    Link,
    Visited,
    Root,
    /// `:target` (Q31b): the element a `#fragment` navigation names
    /// (`InteractionState::target`).
    Target,
    /// Q31: live pointer/keyboard state, read from `INTERACTION` (see
    /// `InteractionState`). `:hover` also matches the hovered element's
    /// ancestors, `:active` the pressed element's ancestors, `:focus` just
    /// the focused element, `:focus-visible` only when focus arrived by
    /// keyboard, `:focus-within` the focused element and its ancestors.
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    Checked,
    Disabled,
    Enabled,
    /// `:dir(ltr)` (true) / `:dir(rtl)` (false). Engine is LTR-only.
    Dir(bool),
    /// `:lang(x)`, stored as given.
    Lang(String),
}

impl cssparser::ToCss for OurPseudoClass {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        dest.write_str(match self {
            OurPseudoClass::Link => ":link",
            OurPseudoClass::Visited => ":visited",
            OurPseudoClass::Root => ":root",
            OurPseudoClass::Target => ":target",
            OurPseudoClass::Hover => ":hover",
            OurPseudoClass::Active => ":active",
            OurPseudoClass::Focus => ":focus",
            OurPseudoClass::FocusVisible => ":focus-visible",
            OurPseudoClass::FocusWithin => ":focus-within",
            OurPseudoClass::Checked => ":checked",
            OurPseudoClass::Disabled => ":disabled",
            OurPseudoClass::Enabled => ":enabled",
            OurPseudoClass::Dir(ltr) => {
                return write!(dest, ":dir({})", if *ltr { "ltr" } else { "rtl" })
            }
            OurPseudoClass::Lang(l) => return write!(dest, ":lang({l})"),
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

/// Q31: live pointer/keyboard state the `:hover`/`:active`/`:focus*`
/// pseudo-classes match against. Element identity is `node_id` (the DOM
/// node's address), because the `LayoutBox`/`Laid` tree holds no `Handle`
/// (`ComputedStyle::node` and `SpanStyle::node` carry the id, which is how
/// a hit test finds the element under the mouse). The set of ids is already
/// expanded to ancestors (`hover`, `active`, `focus_within`), so matching is
/// one hash lookup. Lives in a thread-local (`INTERACTION`) because style
/// matching runs synchronously on the main thread inside
/// `build_layout_tree_with_resources` and has no other channel to the `App`.
/// Reusable for forms (Q30): a text input / button just needs to be made
/// focusable in `is_focusable` and to call `App::set_focus_element`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InteractionState {
    pub(crate) hover: std::collections::HashSet<usize>,
    pub(crate) active: std::collections::HashSet<usize>,
    pub(crate) focus: Option<usize>,
    pub(crate) focus_within: std::collections::HashSet<usize>,
    /// Focus arrived by keyboard (Tab), so `:focus-visible` matches.
    pub(crate) focus_visible: bool,
    /// `:target` (Q31b): the element named by the current URL fragment.
    pub(crate) target: Option<usize>,
}

pub(crate) const STATE_HOVER: u8 = 1;
pub(crate) const STATE_ACTIVE: u8 = 2;
pub(crate) const STATE_FOCUS: u8 = 4;
pub(crate) const STATE_TARGET: u8 = 8;

thread_local! {
    pub(crate) static INTERACTION: RefCell<InteractionState> = RefCell::new(InteractionState::default());
    /// Which state pseudo-classes the stylesheets of the most recent
    /// `build_layout_tree_with_resources` mentioned (bitmask of `STATE_*`).
    /// A state change only needs a restyle when the relevant bit is set.
    pub(crate) static STATE_RULES: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    /// Whether anything in the most recent `build_layout_tree_with_resources`
    /// depended on the viewport *height* (R4): `vh`/`vmin`/`vmax` units,
    /// percentage heights (they can chain up to the initial containing
    /// block), or `position: fixed`/orphaned `absolute` boxes (resolved
    /// against the viewport in `measure_document`). When false, a
    /// height-only window change needs no rebuild at all.
    pub(crate) static VIEWPORT_H_READ: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn note_viewport_height_read() {
    VIEWPORT_H_READ.with(|m| m.set(true));
}

pub(crate) fn viewport_height_read() -> bool {
    VIEWPORT_H_READ.with(|m| m.get())
}

/// Conservative static check of a built tree for boxes whose layout
/// depends on the viewport height (see `VIEWPORT_H_READ`).
pub(crate) fn tree_reads_viewport_height(b: &LayoutBox, positioned_ancestor: bool) -> bool {
    let s = &b.style;
    let pct = |l: &Option<BoxLength>| matches!(l, Some(BoxLength::Percent(_) | BoxLength::Calc(..)));
    if pct(&s.height) || pct(&s.min_height) || pct(&s.max_height) {
        return true;
    }
    match s.position {
        Position::Fixed => return true,
        Position::Absolute if !positioned_ancestor => return true,
        _ => {}
    }
    let anc = positioned_ancestor || s.position != Position::Static;
    match &b.content {
        BoxContent::Blocks(children) | BoxContent::TableRow(children) => {
            children.iter().any(|c| tree_reads_viewport_height(c, anc))
        }
        BoxContent::Inline(items) => items.iter().any(|i| match i {
            InlineItem::Embedded(e) => tree_reads_viewport_height(e, anc),
            _ => false,
        }),
        BoxContent::Image(_) => false,
    }
}

pub(crate) fn note_state_rule(bit: u8) {
    STATE_RULES.with(|m| m.set(m.get() | bit));
}

pub(crate) fn state_rules_mask() -> u8 {
    STATE_RULES.with(|m| m.get())
}

pub(crate) fn interaction_has(f: impl FnOnce(&InteractionState) -> bool) -> bool {
    INTERACTION.with(|i| f(&i.borrow()))
}

pub(crate) fn set_interaction(state: &InteractionState) {
    INTERACTION.with(|i| *i.borrow_mut() = state.clone());
}

/// The identity of a DOM node for `InteractionState`: its address.
pub(crate) fn node_id(h: &Handle) -> usize {
    Rc::as_ptr(h) as usize
}

/// `h` and its ancestors' ids (elements only).
pub(crate) fn ancestor_ids(h: &Handle) -> Vec<usize> {
    let mut out = Vec::new();
    let mut cur = Some(h.clone());
    while let Some(n) = cur {
        if matches!(n.data, NodeData::Element { .. }) {
            out.push(node_id(&n));
        }
        cur = parent_handle(&n);
    }
    out
}

/// The element with `id` under `root`, if it still exists.
pub(crate) fn find_node_by_id(root: &Handle, id: usize) -> Option<Handle> {
    if node_id(root) == id {
        return Some(root.clone());
    }
    for child in root.children.borrow().iter() {
        if let Some(f) = find_node_by_id(child, id) {
            return Some(f);
        }
    }
    None
}

/// Whether Tab / a click can focus `h`: links and `tabindex` elements.
/// (Form controls are Q30's job: add them here.)
pub(crate) fn is_focusable(h: &Handle) -> bool {
    if !matches!(h.data, NodeData::Element { .. }) {
        return false;
    }
    if let Some(n) = element_attr(h, "tabindex").and_then(|t| t.trim().parse::<i32>().ok()) {
        return n >= 0;
    }
    matches!(element_tag(h), "a" | "area") && element_attr(h, "href").is_some()
}

/// The element a URL fragment names (Q31b): `id == frag`, else an
/// `<a name == frag>`, same as browsers.
pub(crate) fn find_fragment_target(root: &Handle, frag: &str) -> Option<Handle> {
    fn walk(h: &Handle, frag: &str, by_name: bool) -> Option<Handle> {
        if matches!(h.data, NodeData::Element { .. }) {
            let attr = if by_name { "name" } else { "id" };
            let ok = element_attr(h, attr).is_some_and(|v| *v == *frag) && (!by_name || element_tag(h) == "a");
            if ok {
                return Some(h.clone());
            }
        }
        h.children.borrow().iter().find_map(|c| walk(c, frag, by_name))
    }
    if frag.is_empty() {
        return None;
    }
    walk(root, frag, false).or_else(|| walk(root, frag, true))
}

/// Where an element ended up on screen: document-space rects (its own box,
/// or one per line for an inline element's glyph runs) and whether an
/// author `outline` already marks it. Used for scroll-into-view and the
/// default focus ring (Q31b). Ignores scroll containers and sticky shifts.
#[derive(Default)]
pub(crate) struct NodeGeom {
    pub(crate) rects: Vec<(f32, f32, f32, f32)>,
    pub(crate) outline: bool,
}

pub(crate) fn node_geometry(laid: &Laid, cache: &ShapeCache, node: usize) -> NodeGeom {
    fn glyph_rects(
        cache: &ShapeCache, id: usize, node: usize, ox: f32, oy: f32, out: &mut Vec<(f32, f32, f32, f32)>,
    ) {
        let Some(entry) = cache.get(&id) else { return };
        let guard = entry.lock().unwrap();
        let (buffer, _, _, _, _, _, _, hrefs) = &*guard;
        for run in buffer.layout_runs() {
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for g in run.glyphs {
                if hrefs.get(g.metadata).is_some_and(|h| h.1 == node) {
                    lo = lo.min(g.x);
                    hi = hi.max(g.x + g.w);
                }
            }
            if lo < hi {
                out.push((ox + lo, oy + run.line_top, hi - lo, run.line_height));
            }
        }
    }
    fn walk(laid: &Laid, cache: &ShapeCache, node: usize, out: &mut NodeGeom) {
        if laid.style.node == node {
            out.rects.push((laid.x, laid.y, laid.w, laid.h));
            out.outline |= laid.style.outline_width > 0.0;
            return;
        }
        match &laid.content {
            LaidContent::Blocks(cs) | LaidContent::TableRow(cs) => cs.iter().for_each(|c| walk(c, cache, node, out)),
            LaidContent::Text(id) => {
                let cx = laid.x + laid.style.border_left() + laid.style.padding.left.px();
                let cy = laid.y + laid.style.border_top() + laid.style.padding.top.px();
                glyph_rects(cache, *id, node, cx, cy, &mut out.rects);
            }
            LaidContent::MixedInline(items) => {
                for item in items {
                    match item {
                        MixedPositioned::Embedded(l) => walk(l, cache, node, out),
                        MixedPositioned::Word { id, x, y, .. } => glyph_rects(cache, *id, node, *x, *y, &mut out.rects),
                    }
                }
            }
            LaidContent::Image(_) => {}
        }
        laid.positioned.iter().for_each(|c| walk(c, cache, node, out));
    }
    let mut out = NodeGeom::default();
    if node != 0 {
        walk(laid, cache, node, &mut out);
    }
    out
}

/// Decodes `%XX` in a URL fragment (UTF-8, lossy).
pub(crate) fn percent_decode_fragment(frag: &str) -> String {
    let b = frag.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The default `:focus-visible` ring (Q31b): 2px, just outside `(x, y, w, h)`
/// (window space). Two colors so it shows on light and dark backgrounds.
pub(crate) fn paint_focus_ring(pixels: &mut [u32], win_w: u32, win_h: u32, x: f32, y: f32, w: f32, h: f32) {
    for (grow, color) in [(2.0, TextColor::rgb(0xff, 0xff, 0xff)), (0.0, TextColor::rgb(0x10, 0x5c, 0xd6))] {
        let (ox, oy, ow, oh) = (x - 2.0 - grow, y - 2.0 - grow, w + 4.0 + 2.0 * grow, h + 4.0 + 2.0 * grow);
        let t = 2.0;
        fill_rect(pixels, win_w, win_h, ox, oy, ow, t, color, None);
        fill_rect(pixels, win_w, win_h, ox, oy + oh - t, ow, t, color, None);
        fill_rect(pixels, win_w, win_h, ox, oy, t, oh, color, None);
        fill_rect(pixels, win_w, win_h, ox + ow - t, oy, t, oh, color, None);
    }
}

/// Focusable elements in tab order: positive `tabindex` ascending first,
/// then the rest (`tabindex=0`/links) in document order.
pub(crate) fn focusable_in_tab_order(root: &Handle) -> Vec<Handle> {
    fn walk(h: &Handle, out: &mut Vec<Handle>) {
        if is_focusable(h) {
            out.push(h.clone());
        }
        for c in h.children.borrow().iter() {
            walk(c, out);
        }
    }
    let mut all = Vec::new();
    walk(root, &mut all);
    let key = |h: &Handle| match element_attr(h, "tabindex").and_then(|t| t.trim().parse::<i32>().ok()) {
        Some(n) if n > 0 => n,
        _ => i32::MAX,
    };
    all.sort_by_key(key);
    all
}

/// A pseudo-element a selector may carry. Only parsed (so a rule like
/// `a, a::after {..}` keeps its `a` part); no `::before`/`::after`
/// boxes are generated yet (Q19), and selectors carrying one never match
/// ordinary elements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OurPseudoElement {
    /// A recognized standard name (`before`, `after`, `marker`, ...).
    Named(&'static str),
    /// Any `::-webkit-*` / `::-moz-*` vendor pseudo-element.
    Vendor(String),
}

impl cssparser::ToCss for OurPseudoElement {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        match self {
            OurPseudoElement::Named(n) => write!(dest, "::{n}"),
            OurPseudoElement::Vendor(n) => write!(dest, "::{n}"),
        }
    }
}

impl PseudoElement for OurPseudoElement {}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OurSelectorImpl;

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
    type PseudoElement = OurPseudoElement;
}

#[derive(Debug)]
pub(crate) struct CssError;

impl From<SelectorParseErrorKind> for CssError {
    fn from(_: SelectorParseErrorKind) -> Self {
        CssError
    }
}

pub(crate) struct OurSelectorParser;

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
            "hover" => {
                note_state_rule(STATE_HOVER);
                return Ok(OurPseudoClass::Hover);
            },
            "active" => {
                note_state_rule(STATE_ACTIVE);
                return Ok(OurPseudoClass::Active);
            },
            "focus" => {
                note_state_rule(STATE_FOCUS);
                return Ok(OurPseudoClass::Focus);
            },
            "focus-visible" => {
                note_state_rule(STATE_FOCUS);
                return Ok(OurPseudoClass::FocusVisible);
            },
            "focus-within" => {
                note_state_rule(STATE_FOCUS);
                return Ok(OurPseudoClass::FocusWithin);
            },
            "target" => {
                note_state_rule(STATE_TARGET);
                return Ok(OurPseudoClass::Target);
            },
            "checked" => return Ok(OurPseudoClass::Checked),
            "disabled" => return Ok(OurPseudoClass::Disabled),
            "enabled" => return Ok(OurPseudoClass::Enabled),
            _ => {}
        }
        Err(ParseError::custom(SelectorParseErrorKind::UnsupportedPseudoClassOrElement))
    }

    fn parse_pseudo_element(
        &self,
        name: CowRcStr<'i>,
    ) -> Result<OurPseudoElement, ParseError<CssError>> {
        match_ignore_ascii_case! { &name,
            "before" => return Ok(OurPseudoElement::Named("before")),
            "after" => return Ok(OurPseudoElement::Named("after")),
            "marker" => return Ok(OurPseudoElement::Named("marker")),
            "placeholder" => return Ok(OurPseudoElement::Named("placeholder")),
            "selection" => return Ok(OurPseudoElement::Named("selection")),
            "first-line" => return Ok(OurPseudoElement::Named("first-line")),
            "first-letter" => return Ok(OurPseudoElement::Named("first-letter")),
            "backdrop" => return Ok(OurPseudoElement::Named("backdrop")),
            "file-selector-button" => return Ok(OurPseudoElement::Named("file-selector-button")),
            _ => {}
        }
        if name.starts_with("-webkit-") || name.starts_with("-moz-") || name.starts_with("-ms-") {
            return Ok(OurPseudoElement::Vendor(name.to_ascii_lowercase()));
        }
        Err(ParseError::custom(SelectorParseErrorKind::UnsupportedPseudoClassOrElement))
    }

    fn parse_non_ts_functional_pseudo_class(
        &self,
        name: CowRcStr<'i>,
        parser: &mut cssparser::Parser<'i>,
        _after_part: bool,
    ) -> Result<OurPseudoClass, ParseError<CssError>> {
        match_ignore_ascii_case! { &name,
            "dir" => {
                let ident = parser.expect_ident()?.clone();
                if ident.eq_ignore_ascii_case("ltr") {
                    return Ok(OurPseudoClass::Dir(true));
                } else if ident.eq_ignore_ascii_case("rtl") {
                    return Ok(OurPseudoClass::Dir(false));
                }
            },
            "lang" => {
                let l = parser.expect_ident_or_string()?.as_ref().to_owned();
                return Ok(OurPseudoClass::Lang(l));
            },
            _ => {}
        }
        Err(ParseError::custom(SelectorParseErrorKind::UnsupportedPseudoClassOrElement))
    }
}

/// A DOM node wrapped so we can implement the foreign `selectors::Element`
/// trait on it (Rust's orphan rules forbid implementing it directly on
/// `Handle`, which is itself a foreign type alias for `Rc<Node>`).
#[derive(Clone, Debug)]
pub(crate) struct El(pub(crate) Handle);

pub(crate) fn is_form_control(h: &Handle) -> bool {
    matches!(
        element_tag(h),
        "input" | "button" | "select" | "textarea" | "option" | "fieldset"
    )
}

pub(crate) fn parent_handle(node: &Handle) -> Option<Handle> {
    let weak: Option<WeakHandle> = node.parent.take();
    let result = weak.as_ref().and_then(Weak::upgrade);
    node.parent.set(weak);
    result
}

pub(crate) fn element_tag(handle: &Handle) -> &str {
    match &handle.data {
        NodeData::Element { name, .. } => name.local.as_ref(),
        _ => "",
    }
}

/// The `<li>` marker run (Q19c): the default bullet/number text styled by
/// the element's inherited style, or, when a `li::marker` rule matches,
/// that rule's own style (color, font, ...) and `content` string (`none`
/// removes the marker). Dynamic `content` (`attr()`/counters) is ignored.
pub(crate) fn marker_prefix(
    handle: &Handle,
    rules: &[StyleRule],
    style: &ComputedStyle,
    viewport_width: f32,
) -> Option<(String, SpanStyle)> {
    let mut text = list_marker(handle, style.list_style_type);
    let mut span: SpanStyle = style.inherited().into();
    let matches = matching_declarations_for(handle, rules, viewport_width, Some("marker"));
    if !matches.is_empty() {
        let mstyle = resolve_style_for(handle, rules, style.inherited(), viewport_width, Some("marker"));
        span = mstyle.inherited().into();
        match cascade_css(&matches, None, "content", |d| d.content.clone()) {
            Cascaded::Value(ContentDecl::Text(t)) => text = t,
            Cascaded::Value(ContentDecl::None) => text.clear(),
            _ => {}
        }
    }
    if text.is_empty() { None } else { Some((text, span)) }
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
pub(crate) fn list_marker(handle: &Handle, style_type: ListStyleType) -> String {
    match style_type {
        ListStyleType::Disc => "\u{2022} ".to_string(),
        ListStyleType::Circle => "\u{25E6} ".to_string(),
        ListStyleType::Square => "\u{25AA} ".to_string(),
        ListStyleType::None => String::new(),
        numbered => {
            let style = numbered.counter_style().unwrap_or("decimal");
            let Some(parent) = parent_handle(handle) else {
                return format!("{}. ", format_counter(1, style));
            };
            let children = parent.children.borrow();
            let tag = element_tag(handle);
            let index = children
                .iter()
                .filter(|c| element_tag(c) == tag)
                .position(|c| Rc::ptr_eq(c, handle))
                .map(|i| i + 1)
                .unwrap_or(1);
            format!("{}. ", format_counter(index as i32, style))
        }
    }
}

pub(crate) fn element_attr<'a>(handle: &'a Handle, name: &str) -> Option<std::cell::Ref<'a, str>> {
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
pub(crate) fn leak_str(s: &str) -> &'static str {
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
            OurPseudoClass::Target => interaction_has(|st| st.target == Some(node_id(&self.0))),
            OurPseudoClass::Hover => interaction_has(|st| st.hover.contains(&node_id(&self.0))),
            OurPseudoClass::Active => interaction_has(|st| st.active.contains(&node_id(&self.0))),
            OurPseudoClass::Focus => interaction_has(|st| st.focus == Some(node_id(&self.0))),
            OurPseudoClass::FocusVisible => {
                interaction_has(|st| st.focus_visible && st.focus == Some(node_id(&self.0)))
            }
            OurPseudoClass::FocusWithin => interaction_has(|st| st.focus_within.contains(&node_id(&self.0))),
            OurPseudoClass::Checked => match element_tag(&self.0) {
                "input" => element_attr(&self.0, "checked").is_some(),
                "option" => element_attr(&self.0, "selected").is_some(),
                _ => false,
            },
            OurPseudoClass::Disabled => is_form_control(&self.0) && element_attr(&self.0, "disabled").is_some(),
            OurPseudoClass::Enabled => is_form_control(&self.0) && element_attr(&self.0, "disabled").is_none(),
            OurPseudoClass::Dir(ltr) => *ltr,
            OurPseudoClass::Lang(want) => {
                let mut cur = Some(self.0.clone());
                while let Some(n) = cur {
                    if let Some(v) = element_attr(&n, "lang") {
                        let v = v.to_ascii_lowercase();
                        let w = want.to_ascii_lowercase();
                        return v == w || v.starts_with(&format!("{w}-"));
                    }
                    cur = parent_handle(&n);
                }
                false
            }
        }
    }

    fn match_pseudo_element(
        &self,
        _pe: &OurPseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        false
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
