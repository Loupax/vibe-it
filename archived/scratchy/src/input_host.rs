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
    /// QDET1: the activation behaviour of a `<summary>`: toggle its `<details>`.
    fn ih_summary_activate(&mut self, summary: &Handle);
    /// QWPT7b: the pointer's button/click-count state.
    fn ih_ptr(&mut self) -> &mut PointerState;
    /// QWPT7b: the window's scroll offset (page coordinates of pointer events).
    fn ih_page_offset(&self) -> (f32, f32);
}

/// QWPT7b: how close in time (ms) two clicks on the same element must be to count as one multi-click.
pub(crate) const MULTI_CLICK_MS: f64 = 500.0;

/// QWPT7b: the mouse's button state across events, for `buttons`, click counts and the
/// pointerdown/mousedown suppression rule. One per input source (the App, the headless driver).
#[derive(Default)]
pub(crate) struct PointerState {
    /// DOM `buttons` mask.
    pub(crate) buttons: u16,
    /// The pointerdown of the current press was cancelled: no compatibility mouse events until all
    /// buttons are up.
    pub(crate) suppress_mouse: bool,
    /// Where each button went down (DOM button number -> target).
    pub(crate) down: Vec<(i16, Option<Handle>)>,
    /// (target, time in ms, count) of the last click.
    pub(crate) last_click: Option<(Handle, f64, u32)>,
}

impl PointerState {
    /// The click count a press or click on `target` at `now` would have.
    pub(crate) fn count_for(&self, target: Option<&Handle>, now: f64) -> u32 {
        match &self.last_click {
            Some((h, t, n)) if target.is_some_and(|x| Rc::ptr_eq(x, h)) && now - t <= MULTI_CLICK_MS => n + 1,
            _ => 1,
        }
    }
    /// Records a click on `target` at `now`; returns its count.
    pub(crate) fn note_click(&mut self, target: &Handle, now: f64) -> u32 {
        let n = self.count_for(Some(target), now);
        self.last_click = Some((target.clone(), now, n));
        n
    }
    /// Forgets a press whose release will never reach the page (a UI widget took it).
    pub(crate) fn forget(&mut self, b: i16) {
        self.buttons &= !button_bit(b);
        self.down.retain(|(x, _)| *x != b);
        if self.buttons == 0 {
            self.suppress_mouse = false;
        }
    }
    pub(crate) fn is_down(&self, b: i16) -> bool {
        self.down.iter().any(|(x, _)| *x == b)
    }
}

/// WebDriver / DOM `button` -> the DOM `buttons` bit.
pub(crate) fn button_bit(b: i16) -> u16 {
    match b {
        0 => 1,
        1 => 4,
        2 => 2,
        n if (0..16).contains(&n) => 1 << n,
        _ => 0,
    }
}

/// QWPT7b: init fields of a mouse or pointer event at `pos`: the position, modifiers, the held
/// `buttons`, `button`, `detail` (mouse events only) and, for pointer events, the PointerEvent fields
/// (`pointerId` 1, `pointerType` "mouse").
pub(crate) fn mouse_fields<H: InputHost + ?Sized>(
    host: &mut H, pos: (f32, f32), button: i16, detail: u32, bubbles: bool, pointer: bool, related: &Option<Handle>,
) -> Vec<(&'static str, EventField)> {
    let page = host.ih_page_offset();
    let buttons = f64::from(host.ih_ptr().buttons);
    let mut v = pointer_init(pos, page, buttons, host.ih_mods(), related, bubbles, pointer);
    v.push(("button", EventField::Num(f64::from(button))));
    if !pointer {
        v.push(("detail", EventField::Num(f64::from(detail))));
    }
    v
}

/// QWPT7b: the `click` event (main button) at `target`. A pointer click carries the click count
/// as `detail`; one sent by the keyboard or an accesskey (position (0, 0)) has `detail` 0.
pub(crate) fn click_event<H: InputHost + ?Sized>(host: &mut H, target: Option<&Handle>, pos: (f32, f32)) -> bool {
    let detail = if pos == (0.0, 0.0) { 0 } else { host.ih_ptr().last_click.as_ref().map_or(1, |c| c.2) };
    let init = mouse_fields(host, pos, 0, detail, true, false, &None);
    host.ih_dispatch(target, "click", &init)
}

/// QWPT7b: button `b` goes down on `target`: `pointerdown` (the first button held only; a cancelled
/// one suppresses the compatibility mouse events until all buttons are up), `mousedown` (`detail` =
/// the click count so far), `contextmenu` for the right button when `mousedown` was not cancelled.
/// Returns whether `mousedown` was not cancelled (and fired). Scratchy shows no menu: what the
/// page does with `contextmenu` is all there is.
pub(crate) fn pointer_press<H: InputHost + ?Sized>(host: &mut H, target: Option<&Handle>, pos: (f32, f32), b: i16, now: f64) -> bool {
    host.ih_ptr().forget(b); // a stale press of this button (its release never arrived)
    let first = host.ih_ptr().buttons == 0;
    let detail = host.ih_ptr().count_for(target, now);
    {
        let st = host.ih_ptr();
        st.buttons |= button_bit(b);
        st.down.push((b, target.cloned()));
    }
    if first {
        let f = mouse_fields(host, pos, b, 0, true, true, &None);
        let ok = host.ih_dispatch(target, "pointerdown", &f);
        host.ih_ptr().suppress_mouse = !ok;
    }
    let mut ok = true;
    if !host.ih_ptr().suppress_mouse {
        let f = mouse_fields(host, pos, b, detail, true, false, &None);
        ok = host.ih_dispatch(target, "mousedown", &f);
    }
    if b == 2 && ok {
        let f = mouse_fields(host, pos, b, 0, true, false, &None);
        host.ih_dispatch(target, "contextmenu", &f);
    }
    ok
}

/// QWPT7b: button `b` goes up on `target`: `pointerup` (when the last button is released), `mouseup`
/// (unless the press was cancelled). Returns where the button went down (`None` outer = it never
/// went down here).
pub(crate) fn pointer_release<H: InputHost + ?Sized>(host: &mut H, target: Option<&Handle>, pos: (f32, f32), b: i16, now: f64) -> Option<Option<Handle>> {
    let (down, suppressed) = {
        let st = host.ih_ptr();
        st.buttons &= !button_bit(b);
        (st.down.iter().position(|(x, _)| *x == b).map(|i| st.down.remove(i).1), st.suppress_mouse)
    };
    if host.ih_ptr().buttons == 0 {
        let f = mouse_fields(host, pos, b, 0, true, true, &None);
        host.ih_dispatch(target, "pointerup", &f);
        host.ih_ptr().suppress_mouse = false;
    }
    if !suppressed {
        // `detail` of mouseup = the click count the release completes (the press's count).
        let detail = match &down {
            Some(Some(d)) => {
                let c = target.and_then(|t| common_ancestor(d, t));
                host.ih_ptr().count_for(c.as_ref(), now)
            }
            _ => 1,
        };
        let f = mouse_fields(host, pos, b, detail, true, false, &None);
        host.ih_dispatch(target, "mouseup", &f);
    }
    down
}

/// QWPT7b: `auxclick` for a non-main button at the common ancestor of press and release.
pub(crate) fn auxclick<H: InputHost + ?Sized>(host: &mut H, target: &Handle, pos: (f32, f32), b: i16, count: u32) {
    let f = mouse_fields(host, pos, b, count, true, false, &None);
    host.ih_dispatch(Some(target), "auxclick", &f);
}

/// QWPT7b: `dblclick` after the second main-button click of a series.
pub(crate) fn dblclick<H: InputHost + ?Sized>(host: &mut H, target: &Handle, pos: (f32, f32)) {
    let f = mouse_fields(host, pos, 0, 2, true, false, &None);
    host.ih_dispatch(Some(target), "dblclick", &f);
}

/// QWPT7b: after `keydown` was not cancelled and nothing consumed the key: `keypress` for Enter and
/// printable keys (no Ctrl/Alt/Meta). Returns false when the page cancelled it.
pub(crate) fn keypress<H: InputHost + ?Sized>(host: &mut H, key: &Key, repeat: bool) -> bool {
    let m = host.ih_mods();
    if m.control_key() || m.alt_key() || m.super_key() {
        return true;
    }
    if matches!(key, Key::Named(NamedKey::Enter)) || key_insertion_text(key).is_some_and(|t| !t.chars().all(char::is_control)) {
        return host.ih_key("keypress", key, repeat);
    }
    true
}

/// Q30c: a click on `target` (None = body). A checkbox/radio toggles BEFORE the click event
/// (legacy pre-activation) and is restored when a handler cancels it; `input` then `change` bubble
/// from it when the state really changed. A reset button resets its form's live state, a select
/// opens, a submit button submits. Returns whether the click was not cancelled.
pub(crate) fn click_activate<H: InputHost + ?Sized>(host: &mut H, target: Option<&Handle>, pos: (f32, f32)) -> bool {
    let Some(control) = target.and_then(enclosing_control) else {
        let ok = host.ih_click(target, pos);
        // QDET1: a click inside the first <summary> of a <details> (not on a link in it) toggles it.
        if ok
            && let Some(t) = target
            && enclosing_link(t).is_none()
            && let Some(s) = ancestors_incl(t).into_iter().find(|e| element_tag(e) == "summary")
            && is_details_summary(&s)
        {
            host.ih_summary_activate(&s);
        }
        return ok;
    };
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

/// `h` and its element ancestors, innermost first.
pub(crate) fn ancestors_incl(h: &Handle) -> Vec<Handle> {
    let mut out = Vec::new();
    let mut cur = Some(h.clone());
    while let Some(n) = cur {
        if matches!(n.data, NodeData::Element { .. }) {
            out.push(n.clone());
        }
        cur = parent_handle(&n);
    }
    out
}

/// The nearest inclusive ancestor element of both `a` and `b`.
pub(crate) fn common_ancestor(a: &Handle, b: &Handle) -> Option<Handle> {
    let bs = ancestors_incl(b);
    ancestors_incl(a).into_iter().find(|x| bs.iter().any(|y| Rc::ptr_eq(x, y)))
}
