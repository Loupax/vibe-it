//! QWPT7: the default actions of trusted input, shared by the App (winit) and the headless
//! testdriver of `--wpt-th-one` (`wpt_input.rs`), so a WPT test that clicks or types runs the
//! same code a user's click or key press runs: checkbox/radio toggling with legacy
//! pre-activation, label forwarding, reset and submit buttons, implicit submission, text editing
//! (`forms_text::text_control_key`), and the event init fields of key and pointer events.
//!
//! The host supplies what differs: how an event is dispatched (the App also syncs scroll and
//! restyles after it), focus (the App's interaction state, the select popup), repaint requests,
//! the select dropdown and form navigation (the headless runner has neither).
use super::*;

/// What a source of trusted input provides to the shared default actions.
pub(crate) trait InputHost {
    /// Fires a trusted event (`JsEngine::dispatch_event`); true when it was not cancelled.
    fn ih_dispatch(&mut self, target: Option<&Handle>, kind: &str, init: &[(&str, EventField)]) -> bool;
    /// The held modifier keys.
    fn ih_mods(&self) -> ModifiersState;
    /// Moves focus to `h` (`None` = blur), firing the focus events.
    fn ih_set_focus(&mut self, h: Option<&Handle>, visible: bool);
    /// A checkbox/radio's live state changed (repaint or `:checked` restyle).
    fn ih_check_state_changed(&mut self) {}
    /// A text control's value or caret changed (repaint).
    fn ih_text_redraw(&mut self) {}
    /// Q30g: a click on a `<select>` (opens its dropdown in the App).
    fn ih_select_click(&mut self, _control: &Handle, _pos: (f32, f32)) {}
    /// Q30e: submit event + navigation for `form`.
    fn ih_request_submit(&mut self, form: &Handle, submitter: Option<&Handle>);
    /// The `click` event at `target` (`None` = body) at window position `pos`.
    fn ih_click(&mut self, target: Option<&Handle>, pos: (f32, f32)) -> bool;
    /// A `keypress` (or other key event) for `key` at the focused element.
    fn ih_key(&mut self, kind: &str, key: &Key, repeat: bool) -> bool;
}

/// Q30c: a click on `target` (None = body). A checkbox/radio toggles BEFORE the click event
/// (legacy pre-activation) and is restored when a handler cancels it; `input` then `change` bubble
/// from it when the state really changed. A reset button resets its form's live state, a select
/// opens, a submit button submits. Returns whether the click was not cancelled.
pub(crate) fn click_activate<H: InputHost + ?Sized>(host: &mut H, target: Option<&Handle>, pos: (f32, f32)) -> bool {
    let Some(control) = target.and_then(enclosing_control) else { return host.ih_click(target, pos) };
    let Some(radio) = checkable_kind(&control) else {
        let ok = host.ih_click(target, pos);
        let is_reset = matches!(element_tag(&control), "button" | "input")
            && element_attr(&control, "type").is_some_and(|t| t.trim().eq_ignore_ascii_case("reset"));
        if ok && is_reset {
            reset_owner_form(host, &control);
        }
        // Q30g: a click on a select opens its dropdown / picks a listbox row.
        if ok && element_tag(&control) == "select" {
            host.ih_select_click(&control, pos);
        }
        // Q30e: a submit button submits its form owner.
        if ok && is_submit_button(&control) && !is_disabled_control(&control) && let Some(form) = form_owner(&control) {
            host.ih_request_submit(&form, Some(&control));
        }
        return ok;
    };
    let (was, was_ind) = (live_checked(&control), live_indeterminate(&control));
    if radio && was {
        return host.ih_click(target, pos);
    }
    let peer = if radio { checked_radio_peer(&control) } else { None };
    set_indeterminate(&control, false);
    set_checked(&control, if radio { true } else { !was });
    host.ih_check_state_changed();
    if !host.ih_click(target, pos) {
        set_checked(&control, was);
        set_indeterminate(&control, was_ind);
        if let Some(p) = peer {
            set_checked(&p, true);
        }
        host.ih_check_state_changed();
        return false;
    }
    let ev = [("bubbles", EventField::Bool(true))];
    host.ih_dispatch(Some(&control), "input", &ev);
    host.ih_dispatch(Some(&control), "change", &ev);
    true
}

/// Q30c: the `reset` event (cancelable) at the form owning `control`, then its checkbox/radio state
/// goes back to the defaults.
pub(crate) fn reset_owner_form<H: InputHost + ?Sized>(host: &mut H, control: &Handle) {
    let Some(form) = form_owner(control) else { return };
    let ev = [("bubbles", EventField::Bool(true)), ("cancelable", EventField::Bool(true))];
    if host.ih_dispatch(Some(&form), "reset", &ev) && reset_form_state(&form) {
        host.ih_check_state_changed();
    }
}

/// Q30b: a click on `<label>` is forwarded to its control: the control takes focus and gets its own
/// click (and activation). Returns whether a control was found.
pub(crate) fn forward_label_click<H: InputHost + ?Sized>(host: &mut H, document: &Handle, label: &Handle, pos: (f32, f32)) -> bool {
    let Some(control) = label_control(document, label) else { return false };
    if is_disabled_control(&control) {
        return true;
    }
    host.ih_set_focus(Some(&control), click_focus_is_visible(&control));
    click_activate(host, Some(&control), pos);
    true
}

/// Q30e: Enter in a single-line text field: click the default button, else submit when the form
/// has a single text field (spec "implicit submission"). A disabled default button blocks it.
pub(crate) fn implicit_submit<H: InputHost + ?Sized>(host: &mut H, field: &Handle) {
    let Some(form) = form_owner(field) else { return };
    match default_button(&form) {
        Some(b) => {
            if !is_disabled_control(&b) {
                click_activate(host, Some(&b), (0.0, 0.0));
            }
        }
        None => {
            if implicit_submission_ok(&form) {
                host.ih_request_submit(&form, None);
            }
        }
    }
}

/// Q33d1: `mousedown`/`mouseup`/`click` init fields at window position `pos` (`button` 0 = main).
pub(crate) fn mouse_init(pos: (f32, f32), m: ModifiersState) -> [(&'static str, EventField); 10] {
    [
        ("bubbles", EventField::Bool(true)),
        ("composed", EventField::Bool(true)),
        ("cancelable", EventField::Bool(true)),
        ("clientX", EventField::Num(pos.0 as f64)),
        ("clientY", EventField::Num(pos.1 as f64)),
        ("button", EventField::Num(0.0)),
        ("ctrlKey", EventField::Bool(m.control_key())),
        ("shiftKey", EventField::Bool(m.shift_key())),
        ("altKey", EventField::Bool(m.alt_key())),
        ("metaKey", EventField::Bool(m.super_key())),
    ]
}

/// Q33d1/QCUR1: pointer/mouse motion and boundary event init fields. `page` is the window's scroll
/// offset, `buttons` the held-button mask.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pointer_init(
    pos: (f32, f32), page: (f32, f32), buttons: f64, m: ModifiersState, related: &Option<Handle>, bubbles: bool, pointer: bool,
) -> Vec<(&'static str, EventField)> {
    let mut v = vec![
        ("bubbles", EventField::Bool(bubbles)),
        ("composed", EventField::Bool(bubbles)),
        ("cancelable", EventField::Bool(bubbles)),
        ("clientX", EventField::Num(f64::from(pos.0))),
        ("clientY", EventField::Num(f64::from(pos.1))),
        ("screenX", EventField::Num(f64::from(pos.0))),
        ("screenY", EventField::Num(f64::from(pos.1))),
        // QWPT7c: a pointing-device event: offsetX/Y are relative to its target.
        // A keyboard-generated click (accesskey, Enter) is sent at (0, 0): no position, offsetX/Y stay 0.
        ("positioned", EventField::Bool(pos != (0.0, 0.0))),
        ("pageX", EventField::Num(f64::from(pos.0 + page.0))),
        ("pageY", EventField::Num(f64::from(pos.1 + page.1))),
        ("buttons", EventField::Num(buttons)),
        ("ctrlKey", EventField::Bool(m.control_key())),
        ("shiftKey", EventField::Bool(m.shift_key())),
        ("altKey", EventField::Bool(m.alt_key())),
        ("metaKey", EventField::Bool(m.super_key())),
    ];
    if pointer {
        v.push(("pointerId", EventField::Num(1.0)));
        v.push(("pointerType", EventField::Str("mouse".into())));
        v.push(("isPrimary", EventField::Bool(true)));
        v.push(("pressure", EventField::Num(if buttons != 0.0 { 0.5 } else { 0.0 })));
    }
    if let Some(r) = related {
        v.push(("relatedTargetId", EventField::Node(r.clone())));
    }
    v
}

/// Q33d: the DOM `key` value of a winit key (named keys by their winit name, as the App always did;
/// `Super` is the DOM's `Meta`).
pub(crate) fn dom_key_value(logical_key: &Key) -> String {
    match logical_key {
        Key::Character(s) => s.to_string(),
        Key::Named(NamedKey::Space) => " ".to_string(),
        Key::Named(NamedKey::Super) => "Meta".to_string(),
        Key::Named(n) => format!("{n:?}"),
        _ => "Unidentified".to_string(),
    }
}

/// Q33d: `keydown`/`keypress`/`keyup` init fields.
pub(crate) fn key_init(kind: &str, logical_key: &Key, physical: Option<PhysicalKey>, repeat: bool, m: ModifiersState) -> [(&'static str, EventField); 13] {
    let kc = crate::app::key_codes(logical_key, physical, kind == "keypress");
    [
        ("bubbles", EventField::Bool(true)),
        ("composed", EventField::Bool(true)),
        ("cancelable", EventField::Bool(true)),
        ("key", EventField::Str(dom_key_value(logical_key))),
        ("code", EventField::Str(kc.code)),
        ("keyCode", EventField::Num(f64::from(kc.key_code))),
        ("charCode", EventField::Num(f64::from(kc.char_code))),
        ("location", EventField::Num(f64::from(kc.location))),
        ("repeat", EventField::Bool(repeat)),
        ("ctrlKey", EventField::Bool(m.control_key())),
        ("shiftKey", EventField::Bool(m.shift_key())),
        ("altKey", EventField::Bool(m.alt_key())),
        ("metaKey", EventField::Bool(m.super_key())),
    ]
}

/// Q33d: the focus events of a focus move from `old` to `new` (blur/focusout on the old element,
/// focus/focusin on the new one; focus and blur do not bubble). `relatedTarget` is the other one.
pub(crate) fn focus_events<H: InputHost + ?Sized>(host: &mut H, old: Option<&Handle>, new: Option<&Handle>) {
    let init = |bubbles: bool, related: Option<&Handle>| {
        let mut v = vec![("bubbles", EventField::Bool(bubbles)), ("composed", EventField::Bool(true))];
        if let Some(r) = related {
            v.push(("relatedTargetId", EventField::Node(r.clone())));
        }
        v
    };
    if let Some(h) = old {
        host.ih_dispatch(Some(h), "blur", &init(false, new));
        host.ih_dispatch(Some(h), "focusout", &init(true, new));
    }
    if let Some(h) = new {
        host.ih_dispatch(Some(h), "focus", &init(false, old));
        host.ih_dispatch(Some(h), "focusin", &init(true, old));
    }
}

/// QWPT7c: the first element (document order) whose `accesskey` lists `key` (one character,
/// compared ignoring case) and that can be activated: not a disabled control, not `hidden`.
pub(crate) fn accesskey_target(document: &Handle, key: &str) -> Option<Handle> {
    let mut chars = key.chars();
    let (Some(want), None) = (chars.next(), chars.next()) else { return None };
    let want = want.to_lowercase().to_string();
    let mut stack = vec![document.clone()];
    while let Some(n) = stack.pop() {
        if matches!(n.data, NodeData::Element { .. })
            && element_attr(&n, "accesskey").is_some_and(|v| v.split_whitespace().any(|t| t.to_lowercase() == want))
            && !is_disabled_control(&n)
            && element_attr(&n, "hidden").is_none()
        {
            return Some(n);
        }
        stack.extend(n.children.borrow().iter().rev().cloned());
    }
    None
}

/// QWPT7c: accesskey activation (the key combination is the caller's: Alt+Shift+key, see KEYS.md).
/// Per the HTML spec: an element with activation behaviour (button, link, checkbox, radio, label,
/// summary, ...) gets a synthetic click (no pointer or mouse events; client coordinates 0), after
/// taking focus when focusable; a text field, textarea or select only takes focus; any other
/// element is focused when it can be. Returns whether an element was found.
pub(crate) fn accesskey_activate<H: InputHost + ?Sized>(host: &mut H, document: &Handle, key: &str) -> bool {
    let Some(el) = accesskey_target(document, key) else { return false };
    let tag = element_tag(&el);
    let ty = element_attr(&el, "type").map(|t| t.trim().to_ascii_lowercase()).unwrap_or_default();
    let text_like = tag == "textarea" || tag == "select" || (tag == "input" && !matches!(ty.as_str(), "button" | "submit" | "reset" | "image" | "checkbox" | "radio" | "file" | "color" | "range"));
    let activatable = !text_like
        && (matches!(tag, "button" | "label" | "summary" | "input") || (matches!(tag, "a" | "area") && element_attr(&el, "href").is_some()));
    if tag == "label" {
        // The label's click, then the forwarded click on its control.
        host.ih_click(Some(&el), (0.0, 0.0));
        if let Some(c) = label_control(document, &el)
            && !is_disabled_control(&c)
        {
            host.ih_set_focus(Some(&c), click_focus_is_visible(&c));
            click_activate(host, Some(&c), (0.0, 0.0));
        }
        return true;
    }
    if is_focusable(&el) {
        host.ih_set_focus(Some(&el), true);
    }
    if activatable {
        click_activate(host, Some(&el), (0.0, 0.0));
    }
    true
}

/// QWPT7c: the accesskey character of a key press when the chord is held: Alt+Shift+key (Ctrl and
/// Super not held). This is the Firefox convention on Linux and Windows (Chrome uses Alt alone);
/// plain Alt is left to the window manager and menu mnemonics, and Alt+Shift keeps scratchy's vim
/// keys (which never use that chord) working. The key comes from the physical code when it is a
/// letter or digit (Shift changes the logical character), else from the logical character.
pub(crate) fn accesskey_chord(key: &Key, physical: Option<PhysicalKey>, m: ModifiersState) -> Option<String> {
    if !(m.alt_key() && m.shift_key()) || m.control_key() || m.super_key() {
        return None;
    }
    if let Some(PhysicalKey::Code(c)) = physical {
        let s = format!("{c:?}");
        if let Some(l) = s.strip_prefix("Key").filter(|l| l.len() == 1) {
            return Some(l.to_lowercase());
        }
        if let Some(d) = s.strip_prefix("Digit").filter(|d| d.len() == 1) {
            return Some(d.to_string());
        }
    }
    match key {
        Key::Character(s) if s.chars().count() == 1 => Some(s.to_lowercase()),
        _ => None,
    }
}
