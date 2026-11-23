//! Keyboard and pointer from the shell window (winit) to the page.
//!
//! A key reaches the focused window (the page, or a dialog, or a popup that holds a grab)
//! only when the key model (C3, `keys.rs`, KEYS.md) says so; pointer events always go to
//! the surface under the pointer (T30).

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyboardKeyEvent, PointerAxisEvent,
    PointerButtonEvent,
};
use std::time::Instant;

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
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let pos = event.position_transformed(self.layout.window);
                self.pointer_motion(pos, Event::time_msec(&event));
            }
            InputEvent::PointerButton { event } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let state = event.state();
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
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let amount = |axis| {
                    event
                        .amount(axis)
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
    pub fn apply_key_effects(&mut self, time: u32) {
        for effect in std::mem::take(&mut self.keys.effects) {
            match effect {
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
            }
        }
        self.redraw();
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
