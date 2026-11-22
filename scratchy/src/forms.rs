//! Q30a: form controls as real boxes (render only; no focus/editing/state yet).
//!
//! DESIGN NOTE (the look and behaviour will be redesigned later): every UA
//! *style* of a control (border, padding, background, font, disabled colours)
//! lives in the "forms" section of `default.css`, so authors override it with
//! the normal cascade. This file only builds the *content* a control paints
//! (value/placeholder text, bullets, check marks, select arrow, bars) and the
//! intrinsic size from `size`/`cols`/`rows`. Each widget is its own small
//! function (`build_text_like`, `build_checkable`, `build_select`, ...), so a
//! redesign replaces one function. `appearance: none` (`ComputedStyle::
//! appearance_none`) drops the widget decorations (check mark, select arrow,
//! progress fill, ...) but keeps the text, so an author-styled control never
//! gets a double border or a second mark.

use super::*;

/// Average glyph advance as a fraction of the font size; stands in for the
/// font's real average char width (no metrics at box-building time).
const AVG_CHAR_EM: f32 = 0.55;
const AVG_CHAR_EM_MONO: f32 = 0.6;
/// Line height used for `rows` (a `<textarea>`'s height) and listboxes.
const LINE_EM: f32 = 1.15;

/// Fill of a checked checkbox/radio, and of a progress/meter bar.
pub(crate) const ACCENT: (u8, u8, u8) = (0, 117, 255);

fn rgb((r, g, b): (u8, u8, u8)) -> TextColor {
    TextColor::rgb(r, g, b)
}

/// The tags `build_form_control_box` handles itself (everything else, e.g.
/// `<button>`, goes through the normal children walk with the UA styles).
pub(crate) fn is_form_control_tag(tag: &str) -> bool {
    matches!(tag, "input" | "textarea" | "select" | "progress" | "meter")
}

fn px_or_zero(l: BoxLength) -> f32 {
    match l {
        BoxLength::Px(v) => v,
        _ => 0.0,
    }
}

/// A content-box size `content` expressed in the box's own `box-sizing`.
fn sized(style: &ComputedStyle, content: f32, horizontal: bool) -> BoxLength {
    let extra = if style.box_sizing == BoxSizing::BorderBox {
        if horizontal {
            px_or_zero(style.padding.left) + px_or_zero(style.padding.right) + style.border_h()
        } else {
            px_or_zero(style.padding.top) + px_or_zero(style.padding.bottom) + style.border_v()
        }
    } else {
        0.0
    };
    BoxLength::Px(content + extra)
}

pub(crate) fn attr_string(handle: &Handle, name: &str) -> Option<String> {
    element_attr(handle, name).map(|a| a.to_string())
}

fn attr_count(handle: &Handle, name: &str) -> Option<u32> {
    attr_string(handle, name)?.trim().parse::<u32>().ok().filter(|n| *n > 0)
}

fn attr_f32(handle: &Handle, name: &str) -> Option<f32> {
    attr_string(handle, name)?.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

pub(crate) fn avg_char_w(style: &ComputedStyle, mono: bool) -> f32 {
    style.font_size * if mono { AVG_CHAR_EM_MONO } else { AVG_CHAR_EM }
}

pub(crate) fn line_h(style: &ComputedStyle) -> f32 {
    (style.font_size * LINE_EM).round()
}

/// The control's own text span: its inherited font/colour.
fn text_span(style: &ComputedStyle, ws: WhiteSpace) -> SpanStyle {
    let mut span: SpanStyle = style.inherited().into();
    span.white_space = ws;
    span
}

fn text_item(style: &ComputedStyle, text: String, ws: WhiteSpace) -> InlineItem {
    InlineItem::Text(text, text_span(style, ws))
}

/// What `::placeholder` rules (the UA one in default.css, then the author's)
/// need to be resolved at box-building time.
pub(crate) struct FormCtx<'a> {
    pub(crate) rules: &'a [StyleRule],
    pub(crate) viewport_width: f32,
}

/// The placeholder text: coloured by the control's `::placeholder` style.
fn placeholder_item(handle: &Handle, ctx: &FormCtx, style: &ComputedStyle, text: String, ws: WhiteSpace) -> InlineItem {
    let ph = resolve_style_for(handle, ctx.rules, style.inherited(), ctx.viewport_width, Some("placeholder"));
    let mut span = text_span(style, ws);
    span.color = ph.color;
    InlineItem::Text(text, span)
}

/// An empty single-line box still needs one line of height: a no-break space
/// gives it the font's natural line box without painting anything.
const BLANK_LINE: &str = "\u{a0}";

pub(crate) fn input_type(handle: &Handle) -> String {
    attr_string(handle, "type").map(|t| t.trim().to_ascii_lowercase()).unwrap_or_default()
}

fn empty_block(style: ComputedStyle) -> LayoutBox {
    LayoutBox { style, background_image: None, content: BoxContent::Inline(Vec::new()) }
}

/// Builds the box of a `<input>`/`<textarea>`/`<select>`/`<progress>`/
/// `<meter>` element, or `None` when it has no special box (hidden inputs
/// never get here: the UA stylesheet makes them `display:none`).
pub(crate) fn build_form_control_box(
    handle: &Handle,
    style: ComputedStyle,
    background_image: Option<Arc<DecodedImage>>,
    ctx: &FormCtx,
) -> Option<LayoutBox> {
    let (style, content) = match element_tag(handle) {
        "input" => match input_type(handle).as_str() {
            "checkbox" => build_checkable(handle, style, false),
            "radio" => build_checkable(handle, style, true),
            "submit" | "reset" | "button" | "image" => build_button_like(handle, style),
            "color" => build_color(handle, style),
            "file" => build_file(style),
            "range" => build_range(handle, style),
            _ => build_text_like(handle, style, ctx),
        },
        "textarea" => build_textarea(handle, style, ctx),
        "select" => build_select(handle, style),
        "progress" => build_bar(handle, style, false),
        "meter" => build_bar(handle, style, true),
        _ => return None,
    };
    Some(LayoutBox { style, background_image, content })
}

/// text/search/email/url/tel/password/number/date-ish: one line of the
/// `value` (bullets for `password`) or the dimmer `placeholder`.
fn build_text_like(handle: &Handle, mut style: ComputedStyle, ctx: &FormCtx) -> (ComputedStyle, BoxContent) {
    let ty = input_type(handle);
    style.white_space = WhiteSpace::Pre;
    if style.width.is_none() && style.width_kw.is_none() {
        let chars = attr_count(handle, "size").unwrap_or(20) as f32;
        style.width = Some(sized(&style, chars * avg_char_w(&style, false), true));
    }
    style.set_overflow(Overflow::Hidden);
    if let Some(kind) = text_control_kind(handle) {
        return build_editable(handle, style, kind, ctx);
    }
    let value = attr_string(handle, "value").unwrap_or_default();
    let item = if !value.is_empty() {
        let value: String = value.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        text_item(&style, value, WhiteSpace::Pre)
    } else if let Some(fmt) = date_placeholder(&ty) {
        placeholder_item(handle, ctx, &style, fmt.to_string(), WhiteSpace::Pre)
    } else {
        text_item(&style, BLANK_LINE.to_string(), WhiteSpace::Pre)
    };
    (style, BoxContent::Inline(vec![item]))
}

/// Q30d: an editable text control. The box holds only one blank line (its
/// height) or nothing (a textarea has its `rows` height); the value, the
/// placeholder, the caret and the selection are painted from the live state
/// (`forms_text.rs`, `paint::paint_text_control`), so editing never rebuilds
/// the layout.
fn build_editable(handle: &Handle, mut style: ComputedStyle, kind: u8, ctx: &FormCtx) -> (ComputedStyle, BoxContent) {
    let ph = resolve_style_for(handle, ctx.rules, style.inherited(), ctx.viewport_width, Some("placeholder")).color;
    let placeholder = attr_string(handle, "placeholder").unwrap_or_default().replace(['\n', '\r'], " ");
    register_text_control(handle, placeholder, ph);
    style.text_widget = kind;
    let content = if kind & TEXT_MULTI != 0 {
        BoxContent::Inline(Vec::new())
    } else {
        BoxContent::Inline(vec![text_item(&style, BLANK_LINE.to_string(), WhiteSpace::Pre)])
    };
    (style, content)
}

/// The empty-state text of the native date/time pickers.
fn date_placeholder(ty: &str) -> Option<&'static str> {
    Some(match ty {
        "date" => "dd/mm/yyyy",
        "time" => "--:--",
        "datetime-local" => "dd/mm/yyyy, --:--",
        "month" => "---------- ----",
        "week" => "Week --, ----",
        _ => return None,
    })
}

/// checkbox / radio: a 13x13 box (UA CSS). The check mark / dot is NOT part of
/// the layout tree: the box only carries `style.check_widget` and the painter
/// draws the mark from the LIVE state (`CHECK_LIVE`, Q30c), so a toggle repaints
/// without rebuilding the layout. `appearance:none` marks nothing.
fn build_checkable(handle: &Handle, mut style: ComputedStyle, radio: bool) -> (ComputedStyle, BoxContent) {
    if !style.appearance_none {
        style.check_widget = if radio { CHECK_RADIO } else { CHECK_BOX };
        if element_attr(handle, "checked").is_some() {
            style.check_widget |= CHECK_ATTR;
        }
    }
    (style, BoxContent::Inline(Vec::new()))
}

// ---- Q30c: live checkbox/radio state -------------------------------------
//
// DESIGN NOTE (easy to change): the state lives in one thread-local map keyed
// by `node_id` (like `INTERACTION`), NOT on `App`, because the three readers
// (selector matching in `dom.rs`, the JS natives in `js_dom.rs`, the painter)
// all run on the main thread without access to the `App`. An entry exists only
// once the user or a script touched the element ("dirty"); until then the
// `checked` attribute controls it. The `App` clears the map on navigation.

pub(crate) const CHECK_BOX: u8 = 1;
pub(crate) const CHECK_RADIO: u8 = 2;
/// The `checked` attribute was present when the box was built.
pub(crate) const CHECK_ATTR: u8 = 4;

/// What the user/script set. `checked: None` = not dirty (attribute rules).
#[derive(Clone, Copy, Default)]
pub(crate) struct CheckLive {
    pub(crate) checked: Option<bool>,
    pub(crate) indeterminate: bool,
}

thread_local! {
    static CHECK_LIVE: RefCell<HashMap<usize, CheckLive>> = RefCell::new(HashMap::new());
}

pub(crate) fn clear_check_state() {
    CHECK_LIVE.with(|m| m.borrow_mut().clear());
}

/// `Some(is_radio)` for an `<input type=checkbox|radio>`.
pub(crate) fn checkable_kind(h: &Handle) -> Option<bool> {
    if element_tag(h) != "input" {
        return None;
    }
    match input_type(h).as_str() {
        "checkbox" => Some(false),
        "radio" => Some(true),
        _ => None,
    }
}

/// The live `checked` of a checkable (the attribute while not dirty).
pub(crate) fn live_checked(h: &Handle) -> bool {
    let dirty = CHECK_LIVE.with(|m| m.borrow().get(&node_id(h)).and_then(|l| l.checked));
    dirty.unwrap_or_else(|| element_attr(h, "checked").is_some())
}

/// Painter's view: `attr` is the `CHECK_ATTR` bit of the built box.
pub(crate) fn live_checked_node(node: usize, attr: bool) -> bool {
    CHECK_LIVE.with(|m| m.borrow().get(&node).and_then(|l| l.checked)).unwrap_or(attr)
}

pub(crate) fn live_indeterminate(h: &Handle) -> bool {
    live_indeterminate_node(node_id(h))
}

pub(crate) fn live_indeterminate_node(node: usize) -> bool {
    CHECK_LIVE.with(|m| m.borrow().get(&node).is_some_and(|l| l.indeterminate))
}

fn tree_root(h: &Handle) -> Handle {
    let mut top = h.clone();
    while let Some(p) = parent_handle(&top) {
        top = p;
    }
    top
}

/// The form owner of a control: the `form` attribute's id, else the nearest
/// `<form>` ancestor.
pub(crate) fn form_owner(h: &Handle) -> Option<Handle> {
    if let Some(id) = element_attr(h, "form").map(|v| v.to_string()) {
        let root = tree_root(h);
        return find_by_id_attr(&root, &id).filter(|f| element_tag(f) == "form");
    }
    let mut cur = parent_handle(h);
    while let Some(n) = cur {
        if element_tag(&n) == "form" {
            return Some(n);
        }
        cur = parent_handle(&n);
    }
    None
}

fn find_by_id_attr(h: &Handle, id: &str) -> Option<Handle> {
    if element_attr(h, "id").is_some_and(|v| *v == *id) {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_by_id_attr(c, id))
}

fn all_inputs(h: &Handle, out: &mut Vec<Handle>) {
    if matches!(element_tag(h), "input" | "textarea") {
        out.push(h.clone());
    }
    for c in h.children.borrow().iter() {
        all_inputs(c, out);
    }
}

/// The other radios in `h`'s group: same tree, same form owner, same
/// non-empty `name`.
fn radio_peers(h: &Handle) -> Vec<Handle> {
    let Some(name) = element_attr(h, "name").map(|v| v.to_string()).filter(|n| !n.is_empty()) else {
        return Vec::new();
    };
    let owner = form_owner(h).map(|f| node_id(&f));
    let mut all = Vec::new();
    all_inputs(&tree_root(h), &mut all);
    all.into_iter()
        .filter(|o| !Rc::ptr_eq(o, h) && checkable_kind(o) == Some(true))
        .filter(|o| element_attr(o, "name").is_some_and(|n| *n == *name))
        .filter(|o| form_owner(o).map(|f| node_id(&f)) == owner)
        .collect()
}

/// The checked radio of `h`'s group other than `h`, if any.
pub(crate) fn checked_radio_peer(h: &Handle) -> Option<Handle> {
    radio_peers(h).into_iter().find(live_checked)
}

/// Sets the live `checked` (making it dirty). Checking a radio unchecks its
/// group. Returns whether anything changed.
pub(crate) fn set_checked(h: &Handle, v: bool) -> bool {
    if element_tag(h) != "input" {
        return false;
    }
    let radio = checkable_kind(h) == Some(true);
    let mut changed = live_checked(h) != v;
    CHECK_LIVE.with(|m| m.borrow_mut().entry(node_id(h)).or_default().checked = Some(v));
    if radio && v {
        for o in radio_peers(h) {
            if live_checked(&o) {
                CHECK_LIVE.with(|m| m.borrow_mut().entry(node_id(&o)).or_default().checked = Some(false));
                changed = true;
            }
        }
    }
    changed
}

pub(crate) fn set_indeterminate(h: &Handle, v: bool) {
    if element_tag(h) == "input" {
        CHECK_LIVE.with(|m| m.borrow_mut().entry(node_id(h)).or_default().indeterminate = v);
    }
}

/// `form.reset()`: every control owned by `form` goes back to its defaults
/// (the attribute controls again, not indeterminate). Returns whether any
/// live state was dropped.
pub(crate) fn reset_form_state(form: &Handle) -> bool {
    let mut all = Vec::new();
    all_inputs(&tree_root(form), &mut all);
    let mut any = reset_text_state(form, &all);
    any |= reset_select_state(form);
    for c in all {
        if element_tag(&c) == "input" && form_owner(&c).is_some_and(|f| Rc::ptr_eq(&f, form)) {
            any |= CHECK_LIVE.with(|m| m.borrow_mut().remove(&node_id(&c))).is_some();
        }
    }
    any
}

/// submit / reset / button / image inputs: a button face (UA CSS) with the
/// `value` (or the type's default label; `alt` for images) as its text.
fn build_button_like(handle: &Handle, mut style: ComputedStyle) -> (ComputedStyle, BoxContent) {
    let ty = input_type(handle);
    let label = match attr_string(handle, "value").filter(|v| !v.is_empty()) {
        Some(v) => v,
        None => match ty.as_str() {
            "submit" => "Submit".to_string(),
            "reset" => "Reset".to_string(),
            "image" => attr_string(handle, "alt").unwrap_or_else(|| "Submit".to_string()),
            _ => String::new(),
        },
    };
    if ty == "image" {
        // `width`/`height` attributes size an image button.
        style.width = style.width.or_else(|| img_size_length(handle, "width"));
        style.height = style.height.or_else(|| img_size_length(handle, "height"));
    }
    style.white_space = WhiteSpace::Pre;
    let label = if label.is_empty() { BLANK_LINE.to_string() } else { label.replace(['\n', '\r'], " ") };
    let item = text_item(&style, label, WhiteSpace::Pre);
    (style, BoxContent::Inline(vec![item]))
}

/// `<input type=color>`: a swatch of the `value` (default black).
fn build_color(handle: &Handle, mut style: ComputedStyle) -> (ComputedStyle, BoxContent) {
    let value = attr_string(handle, "value").unwrap_or_else(|| "#000000".to_string());
    let hex = value.trim().trim_start_matches('#');
    let channel = |i: usize| hex.get(i * 2..i * 2 + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    let color = match (hex.len(), channel(0), channel(1), channel(2)) {
        (6, Some(r), Some(g), Some(b)) => TextColor::rgb(r, g, b),
        _ => TextColor::rgb(0, 0, 0),
    };
    let mut swatch = anonymous_style(style.inherited());
    swatch.display = Display::Block;
    swatch.background_color = Some(color);
    swatch.height = Some(BoxLength::Px(16.0));
    swatch.border_width = [1.0; 4];
    swatch.border_color = [Some(TextColor::rgb(119, 119, 119)); 4];
    swatch.border_style = [BorderStyle::Solid; 4];
    style.set_overflow(Overflow::Hidden);
    if style.width.is_none() {
        style.width = Some(sized(&style, 36.0, true));
    }
    if style.height.is_none() {
        style.height = Some(sized(&style, 20.0, false));
    }
    (style, BoxContent::Blocks(vec![empty_block(swatch)]))
}

/// `<input type=file>`: the "Choose File" button face and "No file chosen".
fn build_file(mut style: ComputedStyle) -> (ComputedStyle, BoxContent) {
    style.white_space = WhiteSpace::Pre;
    let item = text_item(&style, "Choose File  No file chosen".to_string(), WhiteSpace::Pre);
    (style, BoxContent::Inline(vec![item]))
}

/// `<input type=range>`: a thin track filled up to the value's position.
fn build_range(handle: &Handle, mut style: ComputedStyle) -> (ComputedStyle, BoxContent) {
    let min = attr_f32(handle, "min").unwrap_or(0.0);
    let max = attr_f32(handle, "max").unwrap_or(100.0);
    let value = attr_f32(handle, "value").unwrap_or((min + max) / 2.0);
    let frac = if max > min { ((value - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
    if style.width.is_none() {
        style.width = Some(sized(&style, 129.0, true));
    }
    if style.height.is_none() {
        style.height = Some(sized(&style, 16.0, false));
    }
    let mut track = anonymous_style(style.inherited());
    track.display = Display::Block;
    track.height = Some(BoxLength::Px(4.0));
    track.margin.top = BoxLength::Px(6.0);
    track.background_color = Some(TextColor::rgb(187, 187, 187));
    track.border_radius = CornerRadii::uniform(BoxLength::Px(2.0));
    let mut fill = anonymous_style(style.inherited());
    fill.display = Display::Block;
    fill.width = Some(BoxLength::Percent(frac));
    fill.height = Some(BoxLength::Px(4.0));
    fill.background_color = Some(rgb(ACCENT));
    fill.border_radius = CornerRadii::uniform(BoxLength::Px(2.0));
    let track = LayoutBox { style: track, background_image: None, content: BoxContent::Blocks(vec![empty_block(fill)]) };
    (style, BoxContent::Blocks(vec![track]))
}

/// `<textarea>`: `rows` x `cols` box, its text content (or the placeholder),
/// wrapped (`pre-wrap`) and clipped.
fn build_textarea(handle: &Handle, mut style: ComputedStyle, ctx: &FormCtx) -> (ComputedStyle, BoxContent) {
    style.white_space = WhiteSpace::PreWrap;
    style.set_overflow(Overflow::Hidden);
    let cols = attr_count(handle, "cols").unwrap_or(20) as f32;
    let rows = attr_count(handle, "rows").unwrap_or(2) as f32;
    if style.width.is_none() && style.width_kw.is_none() {
        style.width = Some(sized(&style, cols * avg_char_w(&style, true), true));
    }
    if style.height.is_none() {
        style.height = Some(sized(&style, rows * line_h(&style), false));
    }
    build_editable(handle, style, TEXT_MULTI, ctx)
}

/// `<select>`: the box is only SIZED here (one blank line, or `size` rows for a
/// listbox); the selected text, the arrow and the listbox rows are painted from
/// the live selection (forms_select.rs, `paint_select_control`), so choosing an
/// option never rebuilds the layout. `register_select` snapshots the options for
/// the painter.
fn build_select(handle: &Handle, mut style: ComputedStyle) -> (ComputedStyle, BoxContent) {
    let snap = register_select(handle);
    let size = attr_count(handle, "size");
    style.white_space = WhiteSpace::Pre;
    let widest = snap.widest_label_chars() as f32;
    if snap.listbox {
        let rows = size.unwrap_or(4).max(1) as f32;
        style.select_widget = SEL_LIST;
        style.set_overflow(Overflow::Hidden);
        if style.height.is_none() {
            style.height = Some(sized(&style, rows * line_h(&style), false));
        }
        if style.width.is_none() && style.width_kw.is_none() {
            style.width = Some(sized(&style, widest.max(4.0) * avg_char_w(&style, false) + 16.0, true));
        }
        return (style, BoxContent::Inline(Vec::new()));
    }
    style.select_widget = SEL_DROP;
    if style.width.is_none() && style.width_kw.is_none() && style.min_width.is_none() {
        // Chrome sizes a closed select to its widest option (+ the arrow).
        let arrow = if style.appearance_none { 0.0 } else { SELECT_ARROW_W };
        style.min_width = Some(sized(&style, widest * avg_char_w(&style, false) + arrow, true));
    }
    (style, BoxContent::Inline(vec![text_item(&style, BLANK_LINE.to_string(), WhiteSpace::Pre)]))
}

/// `<progress>` and `<meter>`: a track (UA CSS) with a filled bar child.
fn build_bar(handle: &Handle, style: ComputedStyle, meter: bool) -> (ComputedStyle, BoxContent) {
    if style.appearance_none {
        return (style, BoxContent::Inline(Vec::new()));
    }
    let (min, max) = if meter {
        (attr_f32(handle, "min").unwrap_or(0.0), attr_f32(handle, "max").unwrap_or(1.0))
    } else {
        (0.0, attr_f32(handle, "max").filter(|m| *m > 0.0).unwrap_or(1.0))
    };
    let value = attr_f32(handle, "value");
    let frac = match value {
        Some(v) if max > min => ((v - min) / (max - min)).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let mut fill = anonymous_style(style.inherited());
    fill.display = Display::Block;
    fill.width = Some(BoxLength::Percent(frac));
    fill.height = Some(BoxLength::Percent(1.0));
    fill.background_color = Some(if meter { meter_color(handle, value, min, max) } else { rgb(ACCENT) });
    fill.border_radius = CornerRadii::uniform(BoxLength::Px(7.0));
    (style, BoxContent::Blocks(vec![empty_block(fill)]))
}

/// Green in the optimum region, yellow/red outside it (a simplification of
/// the meter value states: only `low`/`high`/`optimum` are consulted).
fn meter_color(handle: &Handle, value: Option<f32>, min: f32, max: f32) -> TextColor {
    let green = TextColor::rgb(26, 163, 74);
    let yellow = TextColor::rgb(255, 214, 48);
    let red = TextColor::rgb(222, 56, 56);
    let Some(v) = value else { return green };
    let low = attr_f32(handle, "low").unwrap_or(min);
    let high = attr_f32(handle, "high").unwrap_or(max);
    let optimum = attr_f32(handle, "optimum").unwrap_or((min + max) / 2.0);
    let region = |x: f32| if x < low { 0i32 } else if x > high { 2 } else { 1 };
    let (rv, ro) = (region(v), region(optimum));
    if rv == ro {
        green
    } else if (rv - ro).abs() == 1 {
        yellow
    } else {
        red
    }
}
