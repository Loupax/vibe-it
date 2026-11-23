//! QWPT7: the headless trusted-input driver of `--wpt-th-one` (the testdriver vendor's back end).
//!
//! `tools/wpt/testdriver-vendor.js` turns `test_driver.click` / `send_keys` / `action_sequence`
//! into ticks of primitive ops (format in that file); `wpt_th::pump` takes one tick when the page
//! is idle and runs it here. Every op goes through the same steps a user's input takes in the App:
//! the element under the pointer comes from hit testing the real layout (`GeomCache::hit`, the
//! App's `hit_test_element` walk), events are trusted (`JsEngine::dispatch_event`), and the default
//! actions are the App's own (`input_host.rs`: checkbox/radio pre-activation, labels, reset and
//! submit buttons, implicit submission; `forms_text::text_control_key`: text editing).
//!
//! Headless differences (no window): no select dropdown, no navigation (a link click or a form
//! submission only fires its events), element scroll offsets are script-side only, and focus has no
//! ring. What the App does not do yet and this driver does (spec order, UI Events / Pointer Events):
//! `pointerdown`/`pointerup` around `mousedown`/`mouseup` (a cancelled `pointerdown` suppresses the
//! compatibility mouse events), `detail` click counts, `dblclick`, `auxclick`, `contextmenu`, and
//! `keypress` for printable keys outside text controls.
use super::*;
use crate::input_host::{self, ancestors_incl, common_ancestor, InputHost, PointerState};

/// Input state that lives across ticks (the "input source" state of WebDriver).
#[derive(Default)]
pub(crate) struct InputState {
    pos: (f32, f32),
    hover: Option<Handle>,
    ptr: PointerState,
    mods: ModifiersState,
    focus: Option<Handle>,
    interaction: InteractionState,
}

/// One tick's worth of access to the page: the engine, the document and the persistent state.
pub(crate) struct Driver<'a> {
    pub(crate) js: &'a JsEngine,
    pub(crate) doc: Handle,
    pub(crate) st: &'a mut InputState,
    /// The virtual clock (ms), for click counting.
    pub(crate) now: f64,
}

impl InputHost for Driver<'_> {
    fn ih_dispatch(&mut self, target: Option<&Handle>, kind: &str, init: &[(&str, EventField)]) -> bool {
        self.js.dispatch_event(target, kind, init)
    }
    fn ih_mods(&self) -> ModifiersState {
        self.st.mods
    }
    fn ih_set_focus(&mut self, h: Option<&Handle>, visible: bool) {
        self.set_focus_ev(h, visible, true)
    }
    fn ih_request_submit(&mut self, form: &Handle, submitter: Option<&Handle>) {
        // No navigation headless: the submit event only (as `form.requestSubmit()` fires it).
        let mut init = vec![("bubbles", EventField::Bool(true)), ("cancelable", EventField::Bool(true))];
        if let Some(s) = submitter {
            init.push(("submitterId", EventField::Node(s.clone())));
        }
        self.js.dispatch_event(Some(form), "submit", &init);
    }
    fn ih_click(&mut self, target: Option<&Handle>, pos: (f32, f32)) -> bool {
        input_host::click_event(self, target, pos)
    }
    fn ih_ptr(&mut self) -> &mut PointerState {
        &mut self.st.ptr
    }
    fn ih_page_offset(&self) -> (f32, f32) {
        let scroll = self.js.eval_value::<Vec<f64>>("testdriver", "[scrollX, scrollY]").unwrap_or_default();
        (scroll.first().copied().unwrap_or(0.0) as f32, scroll.get(1).copied().unwrap_or(0.0) as f32)
    }
    fn ih_key(&mut self, kind: &str, key: &Key, repeat: bool) -> bool {
        let init = input_host::key_init(kind, key, None, repeat, self.st.mods);
        let target = self.st.focus.clone();
        self.js.dispatch_event(target.as_ref(), kind, &init)
    }
}

impl Driver<'_> {
    fn set_interaction(&mut self, st: InteractionState) {
        if st != self.st.interaction {
            self.st.interaction = st;
            set_interaction(&self.st.interaction);
            crate::js_dom::interaction_changed();
        }
    }

    /// Mouse/pointer event fields at `pos` (window space) with the window's current scroll.
    fn mouse_fields(&mut self, pos: (f32, f32), button: i16, detail: u32, bubbles: bool, pointer: bool, related: &Option<Handle>) -> Vec<(&'static str, EventField)> {
        input_host::mouse_fields(self, pos, button, detail, bubbles, pointer, related)
    }

    fn hit(&self, pos: (f32, f32)) -> Option<Handle> {
        self.js.element_at(pos.0, pos.1)
    }

    /// The pointer moves to `pos`: boundary events when the element under it changed (App order:
    /// out/leave on the old chain, over/enter on the new one), then `pointermove` and `mousemove`.
    pub(crate) fn pointer_move(&mut self, pos: (f32, f32)) {
        let prev = self.st.pos;
        self.st.pos = pos;
        let target = self.hit(pos);
        let old = self.st.hover.clone();
        if old.as_ref().map(node_id) != target.as_ref().map(node_id) {
            self.st.hover = target.clone();
            let chain = |h: &Option<Handle>| h.as_ref().map(ancestors_incl).unwrap_or_default();
            let (old_chain, new_chain) = (chain(&old), chain(&target));
            let has = |c: &[Handle], h: &Handle| c.iter().any(|x| Rc::ptr_eq(x, h));
            let left: Vec<Handle> = old_chain.iter().filter(|h| !has(&new_chain, h)).cloned().collect();
            let entered: Vec<Handle> = new_chain.iter().rev().filter(|h| !has(&old_chain, h)).cloned().collect();
            if let Some(o) = &old {
                let f = self.mouse_fields(pos, 0, 0, true, true, &target);
                self.js.dispatch_event(Some(o), "pointerout", &f);
                let f = self.mouse_fields(pos, 0, 0, false, true, &target);
                for l in &left {
                    self.js.dispatch_event(Some(l), "pointerleave", &f);
                }
                if !self.st.ptr.suppress_mouse {
                    let f = self.mouse_fields(pos, 0, 0, true, false, &target);
                    self.js.dispatch_event(Some(o), "mouseout", &f);
                    let f = self.mouse_fields(pos, 0, 0, false, false, &target);
                    for l in &left {
                        self.js.dispatch_event(Some(l), "mouseleave", &f);
                    }
                }
            }
            if let Some(n) = &target {
                let f = self.mouse_fields(pos, 0, 0, true, true, &old);
                self.js.dispatch_event(Some(n), "pointerover", &f);
                let f = self.mouse_fields(pos, 0, 0, false, true, &old);
                for e in &entered {
                    self.js.dispatch_event(Some(e), "pointerenter", &f);
                }
                if !self.st.ptr.suppress_mouse {
                    let f = self.mouse_fields(pos, 0, 0, true, false, &old);
                    self.js.dispatch_event(Some(n), "mouseover", &f);
                    let f = self.mouse_fields(pos, 0, 0, false, false, &old);
                    for e in &entered {
                        self.js.dispatch_event(Some(e), "mouseenter", &f);
                    }
                }
            }
            let mut st = self.st.interaction.clone();
            st.hover = target.as_ref().map(|h| ancestor_ids(h).into_iter().collect()).unwrap_or_default();
            self.set_interaction(st);
        }
        if prev == pos && old.as_ref().map(node_id) == target.as_ref().map(node_id) {
            return; // WebDriver moves to the same point fire no motion events
        }
        let mv = |mut f: Vec<(&'static str, EventField)>| {
            f.push(("movementX", EventField::Num(f64::from(pos.0 - prev.0))));
            f.push(("movementY", EventField::Num(f64::from(pos.1 - prev.1))));
            f
        };
        let f = mv(self.mouse_fields(pos, -1, 0, true, true, &None));
        self.js.dispatch_event(target.as_ref(), "pointermove", &f);
        if !self.st.ptr.suppress_mouse {
            let f = mv(self.mouse_fields(pos, 0, 0, true, false, &None));
            self.js.dispatch_event(target.as_ref(), "mousemove", &f);
        }
    }

    /// Button `b` goes down at the pointer: the shared `pointer_press` (`pointerdown`, `mousedown`,
    /// `contextmenu`), then the App's press default action (focus the nearest click-focusable
    /// ancestor, `:active`).
    pub(crate) fn pointer_down(&mut self, b: i16) {
        let pos = self.st.pos;
        let target = self.hit(pos);
        let ok = input_host::pointer_press(self, target.as_ref(), pos, b, self.now);
        if b == 0 {
            let mut st = self.st.interaction.clone();
            st.active = target.as_ref().map(|h| ancestor_ids(h).into_iter().collect()).unwrap_or_default();
            self.set_interaction(st);
            if ok {
                self.press_focus(target.as_ref());
            }
        }
    }

    /// App `press_state`: the nearest click-focusable ancestor-or-self takes focus (nothing inside
    /// a disabled control; a press on nothing focusable blurs).
    fn press_focus(&mut self, target: Option<&Handle>) {
        let mut focus = None;
        let mut cur = target.cloned();
        while let Some(n) = cur {
            if is_click_focusable(&n) {
                focus = Some(n);
                break;
            }
            if is_disabled_control(&n) {
                return;
            }
            cur = parent_handle(&n);
        }
        let visible = focus.as_ref().is_some_and(click_focus_is_visible);
        self.ih_set_focus(focus.as_ref(), visible);
    }

    /// Button `b` goes up: the shared `pointer_release` (`pointerup`, `mouseup`), then the click on the
    /// nearest common ancestor of the press and release targets with the App's activation behaviour
    /// (main button), `dblclick` on the second click, `auxclick` for the other buttons.
    pub(crate) fn pointer_up(&mut self, b: i16) {
        let pos = self.st.pos;
        let target = self.hit(pos);
        let down = input_host::pointer_release(self, target.as_ref(), pos, b, self.now);
        if b == 0 {
            let mut st = self.st.interaction.clone();
            st.active.clear();
            self.set_interaction(st);
        }
        let Some(Some(down)) = down else { return };
        let Some(up) = target else { return };
        let Some(common) = common_ancestor(&down, &up) else { return };
        if b != 0 {
            let count = self.st.ptr.count_for(Some(&common), self.now);
            input_host::auxclick(self, &common, pos, b, count);
            return;
        }
        let count = self.st.ptr.note_click(&common, self.now);
        // App MouseInput: a click inside a disabled control does nothing; else activation, then a
        // label forwards to its control (only when the click was not inside a control).
        let control = enclosing_control(&common);
        if control.as_ref().is_some_and(is_disabled_control) {
            return;
        }
        let ok = input_host::click_activate(self, Some(&common), pos);
        if ok && control.is_none() && let Some(label) = enclosing_label(&common) {
            let doc = self.doc.clone();
            input_host::forward_label_click(self, &doc, &label, pos);
        }
        if count == 2 {
            input_host::dblclick(self, &common, pos);
        }
    }

    /// A wheel tick at `pos`: a cancelable `wheel` at the element there, then (not cancelled) the
    /// UA scroll of the nearest scroll container (`__scratchy_td_wheel` in the vendor file).
    pub(crate) fn wheel(&mut self, pos: (f32, f32), dx: f64, dy: f64) {
        self.pointer_move(pos);
        let Some(target) = self.hit(pos) else { return };
        let mut f = self.mouse_fields(pos, 0, 0, true, false, &None);
        f.push(("deltaX", EventField::Num(dx)));
        f.push(("deltaY", EventField::Num(dy)));
        f.push(("deltaZ", EventField::Num(0.0)));
        f.push(("deltaMode", EventField::Num(0.0)));
        if self.js.dispatch_event(Some(&target), "wheel", &f) {
            let id = self.js_id(&target);
            self.js.eval("testdriver", &format!("__scratchy_td_wheel(__scratchy_node({id}),{dx},{dy})"));
        }
    }

    fn js_id(&self, h: &Handle) -> i32 {
        self.js.intern_node(h)
    }

    /// Key `key` (WebDriver code points) goes down: modifiers update, `keydown` at the focused
    /// element (else the body), then the App's key default actions: a focused text control edits
    /// (`keypress`, `beforeinput`, `input`), Tab moves focus, Space/Enter activate a focused
    /// checkbox, radio, button or link; other printable keys get a `keypress`.
    pub(crate) fn key_down(&mut self, cps: &[u32]) {
        let (key, physical) = webdriver_key(cps);
        if let Some(m) = modifier_of(&key) {
            self.st.mods |= m;
        }
        let init = input_host::key_init("keydown", &key, physical, false, self.st.mods);
        let focus = self.st.focus.clone();
        if !self.js.dispatch_event(focus.as_ref(), "keydown", &init) {
            return;
        }
        // QWPT7c: the accesskey chord.
        if let Some(k) = input_host::accesskey_chord(&key, physical, self.st.mods) {
            let doc = self.doc.clone();
            if input_host::accesskey_activate(self, &doc, &k) {
                return;
            }
        }
        let tab = matches!(key, Key::Named(NamedKey::Tab));
        let text = focus.as_ref().filter(|h| !tab && crate::forms_text::text_control_kind(h).is_some() && !is_disabled_control(h)).cloned();
        if let Some(h) = text
            && crate::forms_text::text_control_key(self, &h, &key, false)
        {
            return;
        }
        let m = self.st.mods;
        match &key {
            Key::Named(NamedKey::Tab) => self.tab_focus(m.shift_key()),
            Key::Named(NamedKey::Space)
                if !m.control_key() && !m.alt_key() && focus.as_ref().is_some_and(|h| checkable_kind(h).is_some() && !is_disabled_control(h)) =>
            {
                input_host::click_activate(self, focus.as_ref(), (0.0, 0.0));
            }
            Key::Named(NamedKey::Enter | NamedKey::Space)
                if focus.as_ref().is_some_and(|h| {
                    (element_tag(h) == "button" || (element_tag(h) == "input" && matches!(input_type(h).as_str(), "submit" | "image" | "reset" | "button")))
                        && !is_disabled_control(h)
                }) =>
            {
                input_host::click_activate(self, focus.as_ref(), (0.0, 0.0));
            }
            Key::Named(NamedKey::Enter) if focus.as_ref().is_some_and(|h| matches!(element_tag(h), "a" | "area")) => {
                self.ih_click(focus.as_ref(), (0.0, 0.0));
            }
            k => {
                input_host::keypress(self, k, false);
            }
        }
    }

    pub(crate) fn key_up(&mut self, cps: &[u32]) {
        let (key, physical) = webdriver_key(cps);
        if let Some(m) = modifier_of(&key) {
            self.st.mods &= !m;
        }
        let init = input_host::key_init("keyup", &key, physical, false, self.st.mods);
        let focus = self.st.focus.clone();
        self.js.dispatch_event(focus.as_ref(), "keyup", &init);
    }

    /// App `tab_focus`: the next (Shift: previous) element in tab order, `:focus-visible`, a text
    /// control's whole value selected.
    fn tab_focus(&mut self, backwards: bool) {
        let order = focusable_in_tab_order(&self.doc);
        if order.is_empty() {
            return;
        }
        let cur = self.st.focus.as_ref().and_then(|f| order.iter().position(|h| Rc::ptr_eq(h, f)));
        let next = match (cur, backwards) {
            (None, false) => 0,
            (None, true) => order.len() - 1,
            (Some(i), false) => (i + 1) % order.len(),
            (Some(i), true) => (i + order.len() - 1) % order.len(),
        };
        let h = order[next].clone();
        self.ih_set_focus(Some(&h), true);
        if crate::forms_text::text_control_kind(&h).is_some() {
            text_select_all(&h);
        }
    }

    /// WebDriver Element Send Keys focus step: focus `h` unless it has focus (click-focusable only).
    pub(crate) fn focus_element(&mut self, h: &Handle) {
        if self.st.focus.as_ref().is_some_and(|f| Rc::ptr_eq(f, h)) || !is_click_focusable(h) {
            return;
        }
        self.ih_set_focus(Some(h), true);
    }

    /// Runs one tick from the vendor (ops one per line). Returns the tick's duration (ms).
    pub(crate) fn run_tick(&mut self, tick: &str) -> f64 {
        let mut dur = 0.0;
        for line in tick.lines() {
            let mut it = line.split(' ');
            let op = it.next().unwrap_or("");
            let nums: Vec<f64> = it.clone().filter_map(|s| s.parse().ok()).collect();
            let num = |i: usize| nums.get(i).copied().unwrap_or(0.0);
            match op {
                "m" => self.pointer_move((num(0) as f32, num(1) as f32)),
                "d" => self.pointer_down(num(0) as i16),
                "u" => self.pointer_up(num(0) as i16),
                "kd" | "ku" => {
                    let cps: Vec<u32> = it.next().unwrap_or("").split(',').filter_map(|s| s.parse().ok()).collect();
                    if op == "kd" { self.key_down(&cps) } else { self.key_up(&cps) }
                }
                "w" => self.wheel((num(0) as f32, num(1) as f32), num(2), num(3)),
                "f" | "e" => {
                    if let Some(h) = self.js.node_by_js_id(num(0) as i32) {
                        if op == "f" {
                            self.focus_element(&h);
                        } else {
                            crate::forms_text::text_caret_to_end(&h);
                        }
                    }
                }
                "t" => dur = num(0).max(0.0),
                _ => diag!("scratchy --wpt-th-one: testdriver: unknown op {line:?}"),
            }
        }
        dur
    }

    /// Focus state change; `events` false when script's `focus()`/`blur()` fired them itself (QWPT7a).
    pub(crate) fn set_focus_ev(&mut self, h: Option<&Handle>, visible: bool, events: bool) {
        let old = self.st.focus.clone();
        if old.as_ref().map(node_id) == h.map(node_id) {
            return;
        }
        if events {
            crate::js_dom::set_active_element(h);
        }
        self.st.focus = h.cloned();
        let mut st = self.st.interaction.clone();
        match h {
            Some(h) => {
                let ids = ancestor_ids(h);
                st.focus = ids.first().copied();
                st.focus_within = ids.into_iter().collect();
                st.focus_visible = visible;
            }
            None => {
                st.focus = None;
                st.focus_within.clear();
                st.focus_visible = false;
            }
        }
        self.set_interaction(st);
        if let Some(o) = &old {
            crate::forms_text::text_focus_lost(self, o);
        }
        if let Some(n) = h {
            crate::forms_text::text_focus_gained(n);
        }
        if events {
            input_host::focus_events(self, old.as_ref(), h);
        }
    }

    /// A focus request page script made (`el.focus()` / `el.blur()`, QB5): applied as the App's
    /// `js_sync` does, so the focus events fire.
    pub(crate) fn apply_focus_request(&mut self, req: crate::js_dom::FocusReq) {
        match req {
            crate::js_dom::FocusReq::Focus(h) => self.set_focus_ev(Some(&h), false, false),
            crate::js_dom::FocusReq::Blur(h) => {
                if self.st.focus.as_ref().is_some_and(|f| Rc::ptr_eq(f, &h)) || crate::js_dom::js_active_element().is_none() {
                    self.set_focus_ev(None, false, false);
                }
            }
        }
    }
}

fn modifier_of(key: &Key) -> Option<ModifiersState> {
    match key {
        Key::Named(NamedKey::Shift) => Some(ModifiersState::SHIFT),
        Key::Named(NamedKey::Control) => Some(ModifiersState::CONTROL),
        Key::Named(NamedKey::Alt) => Some(ModifiersState::ALT),
        Key::Named(NamedKey::Meta | NamedKey::Super) => Some(ModifiersState::SUPER),
        _ => None,
    }
}

/// A WebDriver key value (one code point; the U+E000 block are the special keys) as a winit
/// logical key, with the physical key where the location matters (right modifiers, numpad).
pub(crate) fn webdriver_key(cps: &[u32]) -> (Key, Option<PhysicalKey>) {
    use winit::keyboard::KeyCode as C;
    let named = |n: NamedKey| (Key::Named(n), None);
    let phys = |n: NamedKey, c: C| (Key::Named(n), Some(PhysicalKey::Code(c)));
    let ch = |s: &str, c: C| (Key::Character(s.into()), Some(PhysicalKey::Code(c)));
    let Some(&cp) = cps.first() else { return (Key::Unidentified(winit::keyboard::NativeKey::Unidentified), None) };
    if cps.len() > 1 || !(0xE000..=0xF8FF).contains(&cp) {
        let s: String = cps.iter().filter_map(|&c| char::from_u32(c)).collect();
        return if s == " " { named(NamedKey::Space) } else { (Key::Character(s.as_str().into()), None) };
    }
    match cp {
        0xE001 => named(NamedKey::Cancel),
        0xE002 => named(NamedKey::Help),
        0xE003 => named(NamedKey::Backspace),
        0xE004 => named(NamedKey::Tab),
        0xE005 => named(NamedKey::Clear),
        0xE006 | 0xE007 => named(NamedKey::Enter),
        0xE008 => phys(NamedKey::Shift, C::ShiftLeft),
        0xE009 => phys(NamedKey::Control, C::ControlLeft),
        0xE00A => phys(NamedKey::Alt, C::AltLeft),
        0xE00B => named(NamedKey::Pause),
        0xE00C => named(NamedKey::Escape),
        0xE00D => named(NamedKey::Space),
        0xE00E => named(NamedKey::PageUp),
        0xE00F => named(NamedKey::PageDown),
        0xE010 => named(NamedKey::End),
        0xE011 => named(NamedKey::Home),
        0xE012 => named(NamedKey::ArrowLeft),
        0xE013 => named(NamedKey::ArrowUp),
        0xE014 => named(NamedKey::ArrowRight),
        0xE015 => named(NamedKey::ArrowDown),
        0xE016 => named(NamedKey::Insert),
        0xE017 => named(NamedKey::Delete),
        0xE018 => (Key::Character(";".into()), None),
        0xE019 => (Key::Character("=".into()), None),
        0xE01A..=0xE023 => {
            let d = cp - 0xE01A;
            let code = [C::Numpad0, C::Numpad1, C::Numpad2, C::Numpad3, C::Numpad4, C::Numpad5, C::Numpad6, C::Numpad7, C::Numpad8, C::Numpad9][d as usize];
            (Key::Character(d.to_string().as_str().into()), Some(PhysicalKey::Code(code)))
        }
        0xE024 => ch("*", C::NumpadMultiply),
        0xE025 => ch("+", C::NumpadAdd),
        0xE026 => ch(",", C::NumpadComma),
        0xE027 => ch("-", C::NumpadSubtract),
        0xE028 => ch(".", C::NumpadDecimal),
        0xE029 => ch("/", C::NumpadDivide),
        0xE031..=0xE03C => {
            let f = [
                NamedKey::F1, NamedKey::F2, NamedKey::F3, NamedKey::F4, NamedKey::F5, NamedKey::F6, NamedKey::F7, NamedKey::F8,
                NamedKey::F9, NamedKey::F10, NamedKey::F11, NamedKey::F12,
            ];
            named(f[(cp - 0xE031) as usize])
        }
        0xE03D => phys(NamedKey::Meta, C::SuperLeft),
        0xE050 => phys(NamedKey::Shift, C::ShiftRight),
        0xE051 => phys(NamedKey::Control, C::ControlRight),
        0xE052 => phys(NamedKey::Alt, C::AltRight),
        0xE053 => phys(NamedKey::Meta, C::SuperRight),
        0xE054 => phys(NamedKey::PageUp, C::Numpad9),
        0xE055 => phys(NamedKey::PageDown, C::Numpad3),
        0xE056 => phys(NamedKey::End, C::Numpad1),
        0xE057 => phys(NamedKey::Home, C::Numpad7),
        0xE058 => phys(NamedKey::ArrowLeft, C::Numpad4),
        0xE059 => phys(NamedKey::ArrowUp, C::Numpad8),
        0xE05A => phys(NamedKey::ArrowRight, C::Numpad6),
        0xE05B => phys(NamedKey::ArrowDown, C::Numpad2),
        0xE05C => phys(NamedKey::Insert, C::Numpad0),
        0xE05D => phys(NamedKey::Delete, C::NumpadDecimal),
        _ => (Key::Unidentified(winit::keyboard::NativeKey::Unidentified), None),
    }
}
