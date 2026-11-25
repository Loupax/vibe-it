//! Q30d: live text of `<input>` (text-like types) and `<textarea>`.
//!
//! DESIGN NOTE (the user will redesign the look and behaviour later; every
//! piece is a small function): same pattern as the Q30c checkbox state. The
//! layout tree does NOT hold the text of an editable control: the box is just
//! sized (one blank line, or `rows` x `cols`) and carries
//! `ComputedStyle::text_widget`; the painter (`paint::paint_text_control`)
//! shapes the LIVE value (or the placeholder) itself, so a keystroke only
//! needs a repaint, never a layout rebuild. The live value, caret and
//! selection of every control sit in one thread-local map keyed by `node_id`
//! (selector matching, JS natives and paint have no `App`). An entry is
//! "dirty" once the user or a script set the value; until then the `value`
//! attribute (textarea: its text) rules and is re-read on every access.
//!
//! Keyboard ownership (the answer to "vim keys vs a focused control"): while
//! an enabled text control is focused every key goes to it (after the page's
//! own `keydown` handlers, which may `preventDefault`), `Tab` still moves
//! focus, and `Escape` blurs the control, handing the keys back to scratchy
//! (vim keys, `:` command line). Keys the control does not use (PageUp, F5,
//! Ctrl+<other letter>) fall through to scratchy.

use super::*;

pub(crate) const TEXT_SINGLE: u8 = 1;
pub(crate) const TEXT_MULTI: u8 = 2;
pub(crate) const TEXT_PASSWORD: u8 = 4;

/// The live state of one control.
pub(crate) struct TextState {
    pub(crate) buf: TextEditBuffer,
    pub(crate) dirty: bool,
    pub(crate) placeholder: String,
    pub(crate) ph_color: Option<TextColor>,
    pub(crate) scroll_x: f32,
    pub(crate) scroll_y: f32,
    /// The value when the control got focus, for the `change` event.
    pub(crate) focus_value: Option<String>,
}

/// (node, time, position) of the previous press.
type LastClick = (usize, std::time::Instant, (f32, f32));

thread_local! {
    static TEXT_LIVE: RefCell<HashMap<usize, TextState>> = RefCell::new(HashMap::new());
    /// Internal clipboard, used when no system clipboard is reachable.
    static INTERNAL_CLIP: RefCell<String> = const { RefCell::new(String::new()) };
    /// Kept alive after `set_text` (X11/Wayland owners must stay alive to serve pastes).
    static SYS_CLIP: RefCell<Option<Clipboard>> = const { RefCell::new(None) };
    /// The node a mouse drag-selection started in (0 = none).
    static DRAG_NODE: Cell<usize> = const { Cell::new(0) };
    /// Last press for double-click detection: (node, time, position).
    static LAST_CLICK: Cell<Option<LastClick>> = const { Cell::new(None) };
}

pub(crate) fn clear_text_state() {
    TEXT_LIVE.with(|m| m.borrow_mut().clear());
    DRAG_NODE.with(|d| d.set(0));
    LAST_CLICK.with(|d| d.set(None));
}

/// `TEXT_*` bits for an editable text control, `None` for everything else
/// (date/time/number-like pickers stay static, as do buttons and checkboxes).
pub(crate) fn text_control_kind(h: &Handle) -> Option<u8> {
    match element_tag(h) {
        "textarea" => Some(TEXT_MULTI),
        "input" => {
            let ty = input_type(h);
            const OTHER: &[&str] = &[
                "hidden", "date", "month", "week", "time", "datetime-local", "range", "color", "checkbox", "radio",
                "file", "submit", "image", "reset", "button",
            ];
            if OTHER.contains(&ty.as_str()) {
                None
            } else if ty == "password" {
                Some(TEXT_SINGLE | TEXT_PASSWORD)
            } else {
                Some(TEXT_SINGLE)
            }
        }
        _ => None,
    }
}

fn sanitize(v: &str, multi: bool) -> String {
    if multi {
        v.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        v.chars().filter(|c| *c != '\n' && *c != '\r').collect()
    }
}

/// The default value: the `value` attribute, or a textarea's text.
fn default_text(h: &Handle) -> String {
    if element_tag(h) == "textarea" {
        let mut t = String::new();
        collect_text_only(h, &mut t);
        sanitize(&t, true)
    } else {
        sanitize(&attr_string(h, "value").unwrap_or_default(), false)
    }
}

/// Every use but layout's `register_text_control` may change what paints (text, caret, selection):
/// QTYPE1: the control's box joins the next paint's damage.
fn with_state<R>(h: &Handle, f: impl FnOnce(&mut TextState) -> R) -> R {
    crate::forms::mark_ctl_dirty(node_id(h));
    with_state_quiet(h, f)
}

fn with_state_quiet<R>(h: &Handle, f: impl FnOnce(&mut TextState) -> R) -> R {
    let def = default_text(h);
    TEXT_LIVE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.entry(node_id(h)).or_insert_with(|| TextState {
            buf: TextEditBuffer { text: def.clone(), cursor: 0, anchor: None },
            dirty: false,
            placeholder: String::new(),
            ph_color: None,
            scroll_x: 0.0,
            scroll_y: 0.0,
            focus_value: None,
        });
        if !st.dirty && st.buf.text != def {
            st.buf.text = def;
            st.buf.cursor = 0;
            st.buf.anchor = None;
        }
        f(st)
    })
}

/// Called whenever the layout box of a control is (re)built: remembers the
/// placeholder and its colour (resolved with the `::placeholder` rules).
pub(crate) fn register_text_control(h: &Handle, placeholder: String, ph_color: TextColor) {
    with_state_quiet(h, |st| {
        st.placeholder = placeholder;
        st.ph_color = Some(ph_color);
    });
}

pub(crate) fn text_value(h: &Handle) -> String {
    with_state_quiet(h, |st| st.buf.text.clone())
}

/// `el.value = v`: replaces the live value (dirty), caret to the end when it
/// changed. Returns whether the text changed.
pub(crate) fn set_text_value(h: &Handle, v: &str) -> bool {
    let v = sanitize(v, element_tag(h) == "textarea");
    with_state(h, |st| {
        let changed = st.buf.text != v;
        st.dirty = true;
        if changed {
            st.buf.text = v;
            st.buf.cursor = st.buf.text.len();
            st.buf.anchor = None;
        }
        changed
    })
}

fn byte_to_utf16(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].encode_utf16().count()
}

fn utf16_to_byte(s: &str, units: usize) -> usize {
    let mut n = 0;
    for (i, c) in s.char_indices() {
        if n >= units {
            return i;
        }
        n += c.len_utf16();
    }
    s.len()
}

/// `selectionStart` (`end` false) / `selectionEnd` in UTF-16 units.
pub(crate) fn text_selection(h: &Handle, end: bool) -> usize {
    with_state_quiet(h, |st| {
        let (a, b) = st.buf.selection().unwrap_or((st.buf.cursor, st.buf.cursor));
        byte_to_utf16(&st.buf.text, if end { b } else { a })
    })
}

pub(crate) fn set_text_selection(h: &Handle, start: usize, end: usize) {
    with_state(h, |st| {
        let a = utf16_to_byte(&st.buf.text, start.min(end));
        let b = utf16_to_byte(&st.buf.text, end);
        st.buf.anchor = (a != b).then_some(a);
        st.buf.cursor = b;
    });
}

pub(crate) fn text_select_all(h: &Handle) {
    with_state(h, |st| st.buf.select_all());
}

/// `form.reset()`: text controls go back to their defaults. True if any
/// control owned by `form` had live (dirty) text.
pub(crate) fn reset_text_state(form: &Handle, controls: &[Handle]) -> bool {
    let mut any = false;
    for c in controls {
        if text_control_kind(c).is_some() && form_owner(c).is_some_and(|f| Rc::ptr_eq(&f, form)) {
            any |= with_state(c, |st| {
                let was = st.dirty;
                st.dirty = false;
                st.buf.text = String::new();
                st.buf.cursor = 0;
                st.buf.anchor = None;
                st.focus_value = None;
                was
            });
            // `with_state` re-reads the default on the next access.
            with_state(c, |_| ());
        }
    }
    any
}

/// What the painter needs of one control.
pub(crate) struct PaintText {
    /// What is shown: the live value (bullets for a password) or the placeholder.
    pub(crate) display: String,
    pub(crate) placeholder: bool,
    pub(crate) ph_color: Option<TextColor>,
    /// Byte offsets into `display`.
    pub(crate) caret: usize,
    pub(crate) selection: Option<(usize, usize)>,
    pub(crate) scroll_x: f32,
    pub(crate) scroll_y: f32,
}

const BULLET: &str = "\u{2022}";

/// Value byte offset -> display byte offset (a bullet is 3 bytes).
fn disp_offset(text: &str, pos: usize, password: bool) -> usize {
    if password {
        text[..pos.min(text.len())].chars().count() * BULLET.len()
    } else {
        pos
    }
}

/// Display byte offset -> value byte offset.
fn val_offset(text: &str, disp: usize, password: bool) -> usize {
    if password {
        text.char_indices().nth(disp / BULLET.len()).map_or(text.len(), |(i, _)| i)
    } else {
        disp.min(text.len())
    }
}

pub(crate) fn text_paint_info(node: usize, password: bool) -> Option<PaintText> {
    TEXT_LIVE.with(|m| {
        let m = m.borrow();
        let st = m.get(&node)?;
        let t = &st.buf.text;
        let empty = t.is_empty();
        Some(PaintText {
            display: if empty {
                st.placeholder.clone()
            } else if password {
                BULLET.repeat(t.chars().count())
            } else {
                t.clone()
            },
            placeholder: empty,
            ph_color: st.ph_color,
            caret: if empty { 0 } else { disp_offset(t, st.buf.cursor, password) },
            selection: st.buf.selection().map(|(a, b)| (disp_offset(t, a, password), disp_offset(t, b, password))),
            scroll_x: st.scroll_x,
            scroll_y: st.scroll_y,
        })
    })
}

pub(crate) fn set_text_scroll(node: usize, x: f32, y: f32) {
    TEXT_LIVE.with(|m| {
        if let Some(st) = m.borrow_mut().get_mut(&node) {
            st.scroll_x = x;
            st.scroll_y = y;
        }
    });
}

/// Shapes `display` with the control's font, for painting and hit testing.
/// Returns the buffer and the line height.
pub(crate) fn shape_control(
    font_system: &mut FontSystem,
    s: &ComputedStyle,
    display: &str,
    color: TextColor,
    multiline: bool,
    content_w: f32,
) -> (Buffer, f32) {
    let span: SpanStyle = s.inherited().into();
    let line_h = if span.line_height <= 0.0 { 0.01 } else { span.line_height };
    let mut buffer = Buffer::new(font_system, Metrics::new(span.font_size, line_h));
    if multiline {
        buffer.set_size(Some(content_w.max(1.0)), None);
    } else {
        buffer.set_size(None, None);
        buffer.set_wrap(Wrap::None);
    }
    let mut attrs = Attrs::new();
    attrs.color_opt = Some(color);
    attrs.weight = span.font_weight;
    attrs.style = span.font_style;
    attrs.family = span.font_family.resolve(font_system.db(), attrs.weight, attrs.style);
    attrs = attrs.metrics(Metrics::new(span.font_size, line_h));
    buffer.set_text(display, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    (buffer, line_h)
}

/// Byte offset where each `\n`-separated line of `text` starts.
fn line_starts(text: &str) -> Vec<usize> {
    let mut v = vec![0];
    v.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    v
}

/// The caret rectangle `(x, y, height)` (buffer space) at display byte `pos`.
pub(crate) fn caret_geometry(buffer: &Buffer, display: &str, pos: usize) -> (f32, f32, f32) {
    let starts = line_starts(display);
    let line = starts.partition_point(|s| *s <= pos) - 1;
    let b = pos - starts[line];
    let mut last = None;
    for run in buffer.layout_runs().filter(|r| r.line_i == line) {
        for g in run.glyphs {
            if g.start <= b && b < g.end {
                return (g.x, run.line_top, run.line_height);
            }
        }
        last = Some((run.glyphs.last().map_or(0.0, |g| g.x + g.w), run.line_top, run.line_height));
    }
    last.unwrap_or((0.0, 0.0, buffer.metrics().line_height))
}

/// The selection highlight rectangles `(x, y, w, h)` in buffer space.
pub(crate) fn selection_rects(buffer: &Buffer, display: &str, sel: (usize, usize)) -> Vec<(f32, f32, f32, f32)> {
    let starts = line_starts(display);
    let mut out = Vec::new();
    for run in buffer.layout_runs() {
        let ls = starts.get(run.line_i).copied().unwrap_or(0);
        let a = sel.0.saturating_sub(ls);
        let b = sel.1.saturating_sub(ls);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for g in run.glyphs {
            if g.start < b && g.end > a && sel.1 > ls {
                lo = lo.min(g.x);
                hi = hi.max(g.x + g.w);
            }
        }
        if lo < hi {
            out.push((lo, run.line_top, hi - lo, run.line_height));
        }
    }
    out
}

/// The display byte offset nearest to buffer-space `(x, y)`.
pub(crate) fn hit_offset(buffer: &Buffer, display: &str, x: f32, y: f32) -> usize {
    let starts = line_starts(display);
    match buffer.hit(x, y) {
        Some(c) => (starts.get(c.line).copied().unwrap_or(0) + c.index).min(display.len()),
        None => display.len(),
    }
}

/// Total text height and width of a shaped buffer.
pub(crate) fn text_extent(buffer: &Buffer) -> (f32, f32) {
    let (mut w, mut h) = (0.0f32, 0.0f32);
    for run in buffer.layout_runs() {
        w = w.max(run.line_w);
        h = h.max(run.line_top + run.line_height);
    }
    (w, h)
}

/// Puts `text` on the clipboard; true when the system clipboard took it (always true in unit
/// tests, which use the internal one only).
pub(crate) fn clipboard_set(text: &str) -> bool {
    INTERNAL_CLIP.with(|c| *c.borrow_mut() = text.to_string());
    // QFLK4: the real clipboard is shared by every process on the desktop (other test runs, the user's
    // own copies), so a copy->paste pair can read someone else's text. Unit tests use the internal one only.
    if cfg!(test) {
        return true;
    }
    if let Ok(mut cb) = Clipboard::new() {
        let ok = cb.set_text(text.to_string()).is_ok();
        SYS_CLIP.with(|s| *s.borrow_mut() = Some(cb));
        return ok;
    }
    false
}

pub(crate) fn clipboard_get() -> Option<String> {
    if !cfg!(test)
        && let Ok(mut cb) = Clipboard::new()
        && let Ok(t) = cb.get_text()
        && !t.is_empty()
    {
        return Some(t);
    }
    Some(INTERNAL_CLIP.with(|c| c.borrow().clone())).filter(|t| !t.is_empty())
}

fn maxlength(h: &Handle) -> Option<usize> {
    attr_string(h, "maxlength")?.trim().parse::<usize>().ok()
}

fn is_readonly(h: &Handle) -> bool {
    element_attr(h, "readonly").is_some() || is_disabled_control(h)
}

impl App {
    /// The focused element when it is an enabled text control.
    pub(crate) fn focused_text_control(&self) -> Option<Handle> {
        let h = find_node_by_id(&self.document, self.interaction.focus?)?;
        (text_control_kind(&h).is_some() && !is_disabled_control(&h)).then_some(h)
    }

    fn text_redraw(&mut self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Tab put focus on `h`: browsers select the whole text.
    pub(crate) fn text_focus_by_tab(&mut self, h: &Handle) {
        if text_control_kind(h).is_some() {
            text_select_all(h);
        }
    }

    /// Focus moved to `h`: remember the value for the `change` event.
    pub(crate) fn text_focus_gained(&mut self, h: &Handle) {
        text_focus_gained(h)
    }

    /// Focus left `h`: `change` if the value changed since focus, selection dropped.
    pub(crate) fn text_focus_lost(&mut self, h: &Handle) {
        text_focus_lost(self, h)
    }

    /// A key pressed (and not cancelled by the page's `keydown`) while text
    /// control `h` has focus. Returns whether it was consumed; when false the
    /// key falls through to scratchy's own bindings.
    pub(crate) fn text_control_key(&mut self, h: &Handle, key: &Key, repeat: bool) -> bool {
        text_control_key(self, h, key, repeat)
    }

    /// The value byte offset under window position `pos` in text control `h`.
    fn text_hit(&mut self, h: &Handle, pos: (f32, f32)) -> Option<usize> {
        let node = node_id(h);
        let laid = self.cached.as_ref()?;
        let l = crate::paint::find_laid_node(laid, node)?;
        let s = l.style;
        let kind = s.text_widget;
        if kind == 0 {
            return None;
        }
        let password = kind & TEXT_PASSWORD != 0;
        let multi = kind & TEXT_MULTI != 0;
        let info = text_paint_info(node, password)?;
        if info.placeholder {
            return Some(0);
        }
        let (cx, cy, cw, ch) = crate::paint::control_content_box(&s, l.x, l.y, l.w, l.h);
        let (buffer, line_h) = shape_control(&mut self.font_system, &s, &info.display, TextColor::rgb(0, 0, 0), multi, cw);
        let top = if multi { cy } else { cy + ((ch - line_h) / 2.0).max(0.0) };
        let (x, y) = (pos.0 + self.scroll_x - cx + info.scroll_x, pos.1 + self.scroll_offset - top + info.scroll_y);
        let d = hit_offset(&buffer, &info.display, x, y);
        let text = text_value(h);
        Some(val_offset(&text, d, password))
    }

    /// Left press at `pos`: places the caret (Shift extends, a quick second
    /// press selects the word) in the focused text control under the pointer.
    pub(crate) fn text_mouse_press(&mut self, pos: (f32, f32)) {
        DRAG_NODE.with(|d| d.set(0));
        let Some(h) = self.focused_text_control() else { return };
        let node = node_id(&h);
        let under = self.element_at(pos);
        if !self.chain_for(under).contains(&node) {
            return;
        }
        let Some(idx) = self.text_hit(&h, pos) else { return };
        let now = std::time::Instant::now();
        let double = LAST_CLICK.with(|c| c.get()).is_some_and(|(n, t, p)| {
            n == node && now.duration_since(t).as_millis() < 500 && (p.0 - pos.0).abs() < 4.0 && (p.1 - pos.1).abs() < 4.0
        });
        LAST_CLICK.with(|c| c.set(if double { None } else { Some((node, now, pos)) }));
        let shift = self.modifiers.shift_key();
        with_state(&h, |st| {
            if double {
                let (a, b) = st.buf.word_at(idx);
                st.buf.anchor = Some(a);
                st.buf.cursor = b;
            } else {
                st.buf.move_to(idx, shift);
            }
        });
        DRAG_NODE.with(|d| d.set(node));
        self.text_redraw();
    }

    /// Pointer moved with the left button down: extends the selection.
    pub(crate) fn text_mouse_drag(&mut self, pos: (f32, f32)) {
        let node = DRAG_NODE.with(|d| d.get());
        if node == 0 {
            return;
        }
        let Some(h) = find_node_by_id(&self.document, node) else { return };
        if let Some(idx) = self.text_hit(&h, pos) {
            with_state(&h, |st| st.buf.move_to(idx, true));
            self.text_redraw();
        }
    }

    pub(crate) fn text_mouse_release(&mut self) {
        DRAG_NODE.with(|d| d.set(0));
    }
}

// QWPT7: the text-control key path, generic over the input source (App or headless testdriver).
use crate::input_host::InputHost;

/// Focus moved to `h`: remember the value for the `change` event.
pub(crate) fn text_focus_gained(h: &Handle) {
    if text_control_kind(h).is_some() {
        with_state(h, |st| st.focus_value = Some(st.buf.text.clone()));
    }
}

/// Focus left `h`: `change` if the value changed since focus, selection dropped.
pub(crate) fn text_focus_lost<H: InputHost + ?Sized>(host: &mut H, h: &Handle) {
    if text_control_kind(h).is_some() {
        fire_change(host, h);
        with_state(h, |st| {
            st.focus_value = None;
            st.buf.anchor = None;
        });
    }
}

/// QWPT7: WebDriver Element Send Keys puts the caret at the end of a text control's value.
pub(crate) fn text_caret_to_end(h: &Handle) {
    if text_control_kind(h).is_some() {
        with_state(h, |st| {
            st.buf.cursor = st.buf.text.len();
            st.buf.anchor = None;
        });
    }
}

/// `change` when the value differs from the one at focus (or last change).
pub(crate) fn fire_change<H: InputHost + ?Sized>(host: &mut H, h: &Handle) {
    let changed = with_state(h, |st| match st.focus_value.clone() {
        Some(fv) if fv != st.buf.text => {
            st.focus_value = Some(st.buf.text.clone());
            true
        }
        _ => false,
    });
    if changed {
        host.ih_dispatch(Some(h), "change", &[("bubbles", EventField::Bool(true))]);
    }
}

/// One edit: `beforeinput` (cancelable), the change itself, then `input`.
fn edit_text<H: InputHost + ?Sized>(host: &mut H, h: &Handle, input_type: &str, data: Option<&str>, f: impl FnOnce(&mut TextEditBuffer)) {
    let mut init = vec![
        ("bubbles", EventField::Bool(true)),
        ("composed", EventField::Bool(true)),
        ("cancelable", EventField::Bool(true)),
        ("inputType", EventField::Str(input_type.to_string())),
    ];
    if let Some(d) = data {
        init.push(("data", EventField::Str(d.to_string())));
    }
    if !host.ih_dispatch(Some(h), "beforeinput", &init) {
        return;
    }
    let changed = with_state(h, |st| {
        let old = st.buf.text.clone();
        f(&mut st.buf);
        let changed = st.buf.text != old;
        st.dirty |= changed;
        changed
    });
    if changed {
        init.retain(|(k, _)| *k != "cancelable");
        init.push(("cancelable", EventField::Bool(false)));
        host.ih_dispatch(Some(h), "input", &init);
    }
    host.ih_text_redraw();
}

fn insert_text<H: InputHost + ?Sized>(host: &mut H, h: &Handle, text: &str, input_type: &str) {
    let multi = element_tag(h) == "textarea";
    let max = maxlength(h);
    let t = text.to_string();
    edit_text(host, h, input_type, Some(text), move |b| {
        let t: String = match max {
            Some(m) => {
                let used = b.text.chars().count() - b.selected_text().map_or(0, |s| s.chars().count());
                t.chars().take(m.saturating_sub(used)).collect()
            }
            None => t,
        };
        if multi {
            b.insert_multiline(&t);
        } else {
            b.insert_str(&t);
        }
    });
}

fn nav_text<H: InputHost + ?Sized>(host: &mut H, h: &Handle, f: impl FnOnce(&mut TextEditBuffer)) {
    with_state(h, |st| f(&mut st.buf));
    host.ih_text_redraw();
}

/// A key pressed (and not cancelled by the page's `keydown`) while text
/// control `h` has focus. Returns whether it was consumed; when false the
/// key falls through to scratchy's own bindings.
pub(crate) fn text_control_key<H: InputHost + ?Sized>(host: &mut H, h: &Handle, key: &Key, repeat: bool) -> bool {
    let multi = element_tag(h) == "textarea";
    let m = host.ih_mods();
    let (ctrl, shift, alt) = (m.control_key(), m.shift_key(), m.alt_key());
    let editable = !is_readonly(h);
    match key {
        Key::Named(NamedKey::Escape) => {
            host.ih_set_focus(None, false);
            true
        }
        Key::Named(NamedKey::Enter) => {
            if host.ih_key("keypress", key, repeat) {
                if multi && editable {
                    insert_text(host, h, "\n", "insertLineBreak");
                } else if !multi {
                    // Enter commits the value, then implicit submission (Q30e).
                    fire_change(host, h);
                    crate::input_host::implicit_submit(host, h);
                }
            }
            true
        }
        Key::Named(NamedKey::Backspace) => {
            if editable {
                let it = if ctrl { "deleteWordBackward" } else { "deleteContentBackward" };
                edit_text(host, h, it, None, move |b| {
                    if ctrl && b.selection().is_none() {
                        let p = b.word_left_pos();
                        b.move_to(p, true);
                    }
                    b.backspace();
                });
            }
            true
        }
        Key::Named(NamedKey::Delete) => {
            if editable {
                let it = if ctrl { "deleteWordForward" } else { "deleteContentForward" };
                edit_text(host, h, it, None, move |b| {
                    if ctrl && b.selection().is_none() {
                        let p = b.word_right_pos();
                        b.move_to(p, true);
                    }
                    b.delete_forward();
                });
            }
            true
        }
        Key::Named(n @ (NamedKey::ArrowLeft | NamedKey::ArrowRight)) => {
            let right = *n == NamedKey::ArrowRight;
            nav_text(host, h, move |b| {
                if ctrl {
                    let p = if right { b.word_right_pos() } else { b.word_left_pos() };
                    b.move_to(p, shift);
                } else {
                    b.nav_horizontal(right, shift);
                }
            });
            true
        }
        Key::Named(n @ (NamedKey::ArrowUp | NamedKey::ArrowDown)) => {
            let down = *n == NamedKey::ArrowDown;
            nav_text(host, h, move |b| {
                let p = if multi { b.vertical_pos(down) } else { None };
                let p = p.unwrap_or(if down { b.text.len() } else { 0 });
                b.move_to(p, shift);
            });
            true
        }
        Key::Named(n @ (NamedKey::Home | NamedKey::End)) => {
            let end = *n == NamedKey::End;
            nav_text(host, h, move |b| {
                let p = match (end, multi && !ctrl) {
                    (false, false) => 0,
                    (true, false) => b.text.len(),
                    (false, true) => b.line_start_pos(),
                    (true, true) => b.line_end_pos(),
                };
                b.move_to(p, shift);
            });
            true
        }
        Key::Named(NamedKey::Insert) if shift && !ctrl && !alt => {
            if editable && let Some(t) = clipboard_get() {
                insert_text(host, h, &t, "insertFromPaste");
            }
            true
        }
        Key::Character(s) if ctrl && !alt => match s.to_lowercase().as_str() {
            "a" => {
                nav_text(host, h, |b| b.select_all());
                true
            }
            "c" | "x" => {
                let password = text_control_kind(h).is_some_and(|k| k & TEXT_PASSWORD != 0);
                let sel = with_state_quiet(h, |st| st.buf.selected_text().map(str::to_string));
                if let (Some(t), false) = (sel, password) {
                    let _ = clipboard_set(&t);
                    if s.to_lowercase() == "x" && editable {
                        edit_text(host, h, "deleteByCut", None, |b| {
                            b.delete_selection();
                        });
                    }
                }
                true
            }
            "v" => {
                if editable && let Some(t) = clipboard_get() {
                    insert_text(host, h, &t, "insertFromPaste");
                }
                true
            }
            _ => false,
        },
        k if !ctrl && !alt && !m.super_key() => {
            let Some(text) = key_insertion_text(k) else { return false };
            if text.chars().all(char::is_control) {
                return false;
            }
            if host.ih_key("keypress", key, repeat) && editable {
                insert_text(host, h, text, "insertText");
            }
            true
        }
        _ => false,
    }
}
