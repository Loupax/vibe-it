//! Keyboard and pointer from the shell window (winit) to the page.
//!
//! A key reaches the focused window (the page, or a dialog, or a popup that holds a grab)
//! only when the key model (C3, `keys.rs`, KEYS.md) says so; pointer events always go to
//! the surface under the pointer (T30).

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyboardKeyEvent, PointerAxisEvent,
    PointerButtonEvent,
};
use std::time::{Duration, Instant};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use smithay::backend::input::KeyState;
use smithay::desktop::WindowSurfaceType;
use smithay::input::keyboard::{FilterResult, Keycode};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use crate::keys::{Effect, KeyInput};
use crate::layout::Region;
use crate::state::State;

/// evdev's middle button.
const BTN_MIDDLE: u32 = 0x112;

/// C-NORMALMODS (KEYS.md T92): what to do with the page's modifier state around a pointer button.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ClickMods {
    /// Nothing: in INPUT the page already has the desktop's modifiers; in NORMAL none are held.
    Nothing,
    /// NORMAL press with a modifier held: tell the page the held modifiers just before the press.
    Send,
    /// The release of a button whose press sent them: put the page's modifiers back
    /// (unmodified in NORMAL, the desktop's in INPUT).
    Restore,
}

/// Pure decision for a button event. `input`: the key mode is INPUT; `held`: the seat has a modifier held;
/// `outstanding`: buttons pressed with `Send` and not yet released.
pub fn click_mods(input: bool, pressed: bool, held: bool, outstanding: u32) -> ClickMods {
    if pressed {
        if !input && held {
            ClickMods::Send
        } else {
            ClickMods::Nothing
        }
    } else if outstanding > 0 {
        ClickMods::Restore
    } else {
        ClickMods::Nothing
    }
}

impl State {
    /// The surface under a point of the shell window, and that surface's origin.
    /// Popups and dialogs come before the page; the bar strip has no surface.
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space.element_under(pos).and_then(|(window, loc)| {
            window
                .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + loc).to_f64()))
        })
    }

    /// Tells the page (the keyboard focus) a modifier state; the seat's own state is untouched.
    fn page_modifiers(&mut self, mods: smithay::input::keyboard::ModifiersState) {
        use smithay::input::keyboard::KeyboardTarget;
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        if let Some(focus) = keyboard.current_focus() {
            let serial = SERIAL_COUNTER.next_serial();
            let seat = self.seat.clone();
            focus.modifiers(&seat, self, mods, serial);
        }
    }

    pub fn process_input<B: InputBackend>(&mut self, event: InputEvent<B>) {
        match event {
            InputEvent::Keyboard { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let code = event.key_code();
                let pressed = event.state() == KeyState::Pressed;
                let keyboard = self.seat.get_keyboard().unwrap();
                // C3: the modes (KEYS.md, `keys.rs`) decide what the page gets. Modifier
                // keys go through the same decision, so in NORMAL the page's idea of the
                // held modifiers stays as it was (a scroll key given to it is unmodified).
                keyboard.input::<(), _>(self, code, event.state(), serial, time, |state, mods, handle| {
                    let key = KeyInput::from_xkb(handle.modified_sym(), mods);
                    if state.keys.key(code.raw(), pressed, &key, Instant::now()) {
                        // C6: the page reads the desktop's clipboard on a paste key.
                        if pressed && crate::keys::is_paste_chord(&key) {
                            state.sync_from_desktop(crate::clipboard::Which::Clipboard);
                        }
                        FilterResult::Forward
                    } else {
                        FilterResult::Intercept(())
                    }
                });
                self.apply_key_effects(time);
                self.arm_key_repeat();
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let pos = event.position_transformed(self.layout.window);
                self.pointer_motion(pos, Event::time_msec(&event));
            }
            InputEvent::PointerButton { event } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let state = event.state();
                let button = event.button_code();
                // C-PICK T66/T67: a press closes the picker; over its list neither the press nor its
                // release reaches the page (the list was drawn over it).
                if state == ButtonState::Pressed && self.keys.vim_mode == crate::keys::VimMode::Pick {
                    let pos = pointer.current_location();
                    let over = crate::picker::current_rect(&self.keys, &self.layout).is_some_and(|r| r.to_f64().contains(pos));
                    let swallow = self.keys.pick_click(over);
                    self.redraw();
                    if swallow {
                        self.swallowed_button = Some(button);
                        return;
                    }
                }
                if state == ButtonState::Released && self.swallowed_button == Some(button) {
                    self.swallowed_button = None;
                    return;
                }
                if state == ButtonState::Pressed && self.surface_under(pointer.current_location()).is_some() {
                    // T29: a focus change reported soon after this is the user's click.
                    self.last_click = Some(Instant::now());
                    // C6: a middle click pastes the primary selection: offer the desktop's.
                    if event.button_code() == BTN_MIDDLE {
                        self.sync_from_desktop(crate::clipboard::Which::Primary);
                    }
                }
                if state == ButtonState::Pressed && !pointer.is_grabbed() {
                    // A click on a dialog raises it and gives it the keyboard; a click on
                    // the page focuses the page (dialogs stay above it).
                    let hit = self.space.element_under(pointer.current_location()).map(|(w, _)| w.clone());
                    if let Some(w) = hit {
                        if self.dialogs.contains(&w) {
                            self.space.raise_element(&w, false);
                            self.dialogs.retain(|d| d != &w);
                            self.dialogs.push(w.clone());
                        }
                        let keyboard = self.seat.get_keyboard().unwrap();
                        keyboard.set_focus(self, w.toplevel().map(|t| t.wl_surface().clone()), serial);
                    }
                }
                // C-NORMALMODS (T92): a click in NORMAL carries the modifiers held; NORMAL's keys stay unmodified.
                let keyboard = self.seat.get_keyboard().unwrap();
                let ms = keyboard.modifier_state();
                let held = ms.ctrl || ms.shift || ms.alt || ms.logo;
                let input = self.keys.key_mode == crate::keys::KeyMode::Input;
                let action = click_mods(input, state == ButtonState::Pressed, held, self.click_mods_out);
                match action {
                    ClickMods::Send => self.click_mods_out += 1,
                    ClickMods::Restore => self.click_mods_out -= 1,
                    ClickMods::Nothing => {}
                }
                if action == ClickMods::Send {
                    self.page_modifiers(ms);
                }
                pointer.button(
                    self,
                    &ButtonEvent {
                        button: event.button_code(),
                        state,
                        serial,
                        time: Event::time_msec(&event),
                    },
                );
                pointer.frame(self);
                if action == ClickMods::Restore {
                    let restore = if self.keys.key_mode == crate::keys::KeyMode::Input {
                        keyboard.modifier_state()
                    } else {
                        smithay::input::keyboard::ModifiersState::default()
                    };
                    self.page_modifiers(restore);
                }
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                // C6b: winit's pixel deltas are physical; the page scrolls in logical px.
                let scale = self.layout.scale;
                let amount = |axis| {
                    event
                        .amount(axis)
                        .map(|v| v / scale)
                        .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0)
                };
                let mut frame = AxisFrame::new(Event::time_msec(&event)).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let value = amount(axis);
                    if value != 0.0 {
                        frame = frame.value(axis, value);
                        if let Some(v120) = event.amount_v120(axis) {
                            frame = frame.v120(axis, v120 as i32);
                        }
                    } else if source == AxisSource::Finger {
                        frame = frame.stop(axis);
                    }
                }
                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    /// Carries out what the key model queued: commands to the host, scroll input given
    /// to the page window (KEYS.md T13), quitting.
    /// An `Effect::Pause` holds back everything queued after it, in order, until its timer fires.
    pub fn apply_key_effects(&mut self, time: u32) {
        let new = std::mem::take(&mut self.keys.effects);
        self.deferred.extend(new);
        if !self.paused {
            self.run_effects(time);
        }
        // C4: a buffer command may have changed the current buffer.
        self.sync_page();
        self.redraw();
    }

    fn run_effects(&mut self, time: u32) {
        while let Some(effect) = self.deferred.pop_front() {
            match effect {
                Effect::Pause(ms) => {
                    self.paused = true;
                    let timer = Timer::from_duration(Duration::from_millis(ms));
                    let later = time.wrapping_add(ms as u32);
                    let inserted = self.loop_handle.insert_source(timer, move |_, _, state| {
                        state.paused = false;
                        state.run_effects(later);
                        state.redraw();
                        TimeoutAction::Drop
                    });
                    if inserted.is_err() {
                        // No timer: carry on at once rather than lose the rest.
                        self.paused = false;
                        continue;
                    }
                    return;
                }
                Effect::Send(cmd) => self.send(cmd),
                Effect::KeyDown(k) | Effect::KeyUp(k) => {
                    let st = if matches!(effect, Effect::KeyDown(_)) {
                        KeyState::Pressed
                    } else {
                        KeyState::Released
                    };
                    let keyboard = self.seat.get_keyboard().unwrap();
                    let serial = SERIAL_COUNTER.next_serial();
                    // xkb key codes are evdev + 8. Not through the filter: the page gets it.
                    keyboard.input_forward(self, Keycode::new(k + 8), st, serial, time, false);
                }
                Effect::Wheel(steps) => {
                    let pointer = self.seat.get_pointer().unwrap();
                    let over_page = pointer.current_focus().is_some_and(|s| {
                        self.page
                            .as_ref()
                            .is_some_and(|p| p.toplevel().is_some_and(|t| t.wl_surface() == &s))
                    });
                    if over_page {
                        let frame = AxisFrame::new(time)
                            .source(AxisSource::Wheel)
                            .value(Axis::Vertical, steps as f64 * 15.0)
                            .v120(Axis::Vertical, steps * 120);
                        pointer.axis(self, frame);
                        pointer.frame(self);
                    } else {
                        self.keys.message = "web: wheel scrolling needs the pointer over the page".into();
                    }
                }
                Effect::Quit => self.quit(":q"),
                Effect::OpenExternal(url) => {
                    self.keys.message = crate::external::open(&url, crate::config::EXTERNAL_BROWSER);
                }
            }
        }
    }

    pub fn pointer_motion(&mut self, pos: Point<f64, Logical>, time: u32) {
        self.pointer_loc = pos;
        let under = self.surface_under(pos);
        let over_bar = under.is_none() && self.layout.region(pos) == Region::Bar;
        if over_bar != self.over_bar {
            self.over_bar = over_bar;
            self.apply_cursor();
        }
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time,
            },
        );
        pointer.frame(self);
        // A surface cursor follows the pointer.
        if matches!(self.cursor, smithay::input::pointer::CursorImageStatus::Surface(_)) {
            self.redraw();
        }
    }
}
