//! Keyboard and pointer from the shell window (winit) to the page.
//!
//! C2 forwards every key to the focused window (the page, or a dialog, or a popup that
//! holds a grab). C3 hooks its modes into the keyboard filter below.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent,
};
use smithay::desktop::WindowSurfaceType;
use smithay::input::keyboard::FilterResult;
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use crate::layout::Region;
use crate::state::State;

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
                let keyboard = self.seat.get_keyboard().unwrap();
                // C3: modes (KEYS.md) decide here what the page gets.
                keyboard.input::<(), _>(self, event.key_code(), event.state(), serial, time, |_, _, _| {
                    FilterResult::Forward
                });
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let pos = event.position_transformed(self.layout.window);
                self.pointer_motion(pos, Event::time_msec(&event));
            }
            InputEvent::PointerButton { event } => {
                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let state = event.state();
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
                    &ButtonEvent { button: event.button_code(), state, serial, time: Event::time_msec(&event) },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let amount = |axis| {
                    event.amount(axis).unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0)
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
        pointer.motion(self, under, &MotionEvent { location: pos, serial, time });
        pointer.frame(self);
        // A surface cursor follows the pointer.
        if matches!(self.cursor, smithay::input::pointer::CursorImageStatus::Surface(_)) {
            self.redraw();
        }
    }
}
