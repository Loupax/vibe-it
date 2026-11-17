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
const ACCENT: (u8, u8, u8) = (0, 117, 255);

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

fn attr_string(handle: &Handle, name: &str) -> Option<String> {
    element_attr(handle, name).map(|a| a.to_string())
}

fn attr_count(handle: &Handle, name: &str) -> Option<u32> {
    attr_string(handle, name)?.trim().parse::<u32>().ok().filter(|n| *n > 0)
}

fn attr_f32(handle: &Handle, name: &str) -> Option<f32> {
    attr_string(handle, name)?.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

fn avg_char_w(style: &ComputedStyle, mono: bool) -> f32 {
    style.font_size * if mono { AVG_CHAR_EM_MONO } else { AVG_CHAR_EM }
}

fn line_h(style: &ComputedStyle) -> f32 {
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

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn input_type(handle: &Handle) -> String {
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
    style.overflow = Overflow::Hidden;
    let value = attr_string(handle, "value").unwrap_or_default();
    let item = if !value.is_empty() {
        // `value` has newlines stripped (sanitization algorithm).
        let value: String = value.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        let shown = if ty == "password" { "\u{2022}".repeat(value.chars().count()) } else { value };
        text_item(&style, shown, WhiteSpace::Pre)
    } else if let Some(ph) = attr_string(handle, "placeholder").filter(|p| !p.is_empty()) {
        placeholder_item(handle, ctx, &style, ph.replace(['\n', '\r'], " "), WhiteSpace::Pre)
    } else if let Some(fmt) = date_placeholder(&ty) {
        placeholder_item(handle, ctx, &style, fmt.to_string(), WhiteSpace::Pre)
    } else {
        text_item(&style, BLANK_LINE.to_string(), WhiteSpace::Pre)
    };
    (style, BoxContent::Inline(vec![item]))
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

/// checkbox / radio: a 13x13 box (UA CSS) with a check mark or a dot when the
/// `checked` attribute is present. `appearance:none` paints nothing extra.
fn build_checkable(handle: &Handle, mut style: ComputedStyle, radio: bool) -> (ComputedStyle, BoxContent) {
    let checked = element_attr(handle, "checked").is_some();
    if !checked || style.appearance_none {
        return (style, BoxContent::Inline(Vec::new()));
    }
    let accent = rgb(ACCENT);
    style.border_color = [Some(accent); 4];
    if radio {
        // A white disc with an accent ring and dot, like the native radio.
        let mut dot = anonymous_style(style.inherited());
        dot.display = Display::Block;
        dot.width = Some(BoxLength::Px(5.0));
        dot.height = Some(BoxLength::Px(5.0));
        let m = BoxLength::Px(3.0);
        dot.margin = Sides { top: m, right: m, bottom: m, left: m };
        dot.background_color = Some(accent);
        dot.border_radius = BoxLength::Percent(0.5);
        return (style, BoxContent::Blocks(vec![empty_block(dot)]));
    }
    style.background_color = Some(accent);
    style.text_align = TextAlign::Center;
    style.overflow = Overflow::Hidden;
    let mut span = text_span(&style, WhiteSpace::Pre);
    span.color = TextColor::rgb(255, 255, 255);
    span.font_size = 11.0;
    span.font_weight = FontWeight::BOLD;
    span.line_height = 11.0;
    (style, BoxContent::Inline(vec![InlineItem::Text("\u{2713}".to_string(), span)]))
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
    style.overflow = Overflow::Hidden;
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
    track.border_radius = BoxLength::Px(2.0);
    let mut fill = anonymous_style(style.inherited());
    fill.display = Display::Block;
    fill.width = Some(BoxLength::Percent(frac));
    fill.height = Some(BoxLength::Px(4.0));
    fill.background_color = Some(rgb(ACCENT));
    fill.border_radius = BoxLength::Px(2.0);
    let track = LayoutBox { style: track, background_image: None, content: BoxContent::Blocks(vec![empty_block(fill)]) };
    (style, BoxContent::Blocks(vec![track]))
}

/// `<textarea>`: `rows` x `cols` box, its text content (or the placeholder),
/// wrapped (`pre-wrap`) and clipped.
fn build_textarea(handle: &Handle, mut style: ComputedStyle, ctx: &FormCtx) -> (ComputedStyle, BoxContent) {
    style.white_space = WhiteSpace::PreWrap;
    style.overflow = Overflow::Hidden;
    let cols = attr_count(handle, "cols").unwrap_or(20) as f32;
    let rows = attr_count(handle, "rows").unwrap_or(2) as f32;
    if style.width.is_none() && style.width_kw.is_none() {
        style.width = Some(sized(&style, cols * avg_char_w(&style, true), true));
    }
    if style.height.is_none() {
        style.height = Some(sized(&style, rows * line_h(&style), false));
    }
    let mut text = String::new();
    collect_text_only(handle, &mut text);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let item = if !text.is_empty() {
        Some(text_item(&style, text, WhiteSpace::PreWrap))
    } else {
        attr_string(handle, "placeholder")
            .filter(|p| !p.is_empty())
            .map(|ph| placeholder_item(handle, ctx, &style, ph, WhiteSpace::PreWrap))
    };
    (style, BoxContent::Inline(item.into_iter().collect()))
}

/// The `option` elements of a `<select>` (also inside `<optgroup>`), as
/// (label, selected, disabled).
fn select_options(select: &Handle) -> Vec<(String, bool, bool)> {
    fn walk(h: &Handle, out: &mut Vec<(String, bool, bool)>) {
        for c in h.children.borrow().iter() {
            if !matches!(c.data, NodeData::Element { .. }) {
                continue;
            }
            match element_tag(c) {
                "option" => {
                    let label = match attr_string(c, "label").filter(|l| !l.is_empty()) {
                        Some(l) => l,
                        None => {
                            let mut t = String::new();
                            collect_text_only(c, &mut t);
                            collapse_ws(&t)
                        }
                    };
                    out.push((label, element_attr(c, "selected").is_some(), element_attr(c, "disabled").is_some()));
                }
                "optgroup" => walk(c, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(select, &mut out);
    out
}

/// `<select>`: closed state shows the selected (else first) option's text and
/// a drop arrow; `multiple`/`size>1` render a listbox of `size` rows (4 by
/// default). The options themselves are never laid out in flow.
fn build_select(handle: &Handle, mut style: ComputedStyle) -> (ComputedStyle, BoxContent) {
    let options = select_options(handle);
    let size = attr_count(handle, "size");
    let listbox = element_attr(handle, "multiple").is_some() || size.is_some_and(|s| s > 1);
    style.white_space = WhiteSpace::Pre;
    let widest = options.iter().map(|(l, _, _)| l.chars().count()).max().unwrap_or(0) as f32;
    if listbox {
        let rows = size.unwrap_or(4).max(1) as f32;
        style.overflow = Overflow::Hidden;
        if style.height.is_none() {
            style.height = Some(sized(&style, rows * line_h(&style), false));
        }
        if style.width.is_none() && style.width_kw.is_none() {
            style.width = Some(sized(&style, widest.max(4.0) * avg_char_w(&style, false) + 16.0, true));
        }
        let mut row_boxes = Vec::new();
        for (label, selected, _) in &options {
            let mut row = anonymous_style(style.inherited());
            row.display = Display::Block;
            if *selected {
                row.background_color = Some(TextColor::rgb(206, 206, 206));
            }
            let text = if label.is_empty() { BLANK_LINE.to_string() } else { label.clone() };
            row_boxes.push(LayoutBox {
                style: row,
                background_image: None,
                content: BoxContent::Inline(vec![text_item(&style, text, WhiteSpace::Pre)]),
            });
        }
        return (style, BoxContent::Blocks(row_boxes));
    }
    let shown = options
        .iter()
        .rev()
        .find(|(_, sel, _)| *sel)
        .or_else(|| options.iter().find(|(_, _, dis)| !*dis))
        .or_else(|| options.first())
        .map(|(l, _, _)| l.clone())
        .unwrap_or_default();
    if style.width.is_none() && style.width_kw.is_none() && style.min_width.is_none() {
        // Chrome sizes a closed select to its widest option.
        style.min_width = Some(sized(&style, widest * avg_char_w(&style, false), true));
    }
    let shown = if shown.is_empty() { BLANK_LINE.to_string() } else { shown };
    let mut items = vec![text_item(&style, shown, WhiteSpace::Pre)];
    if !style.appearance_none {
        items.push(text_item(&style, "\u{a0}\u{2304}".to_string(), WhiteSpace::Pre));
    }
    (style, BoxContent::Inline(items))
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
    fill.border_radius = BoxLength::Px(7.0);
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
