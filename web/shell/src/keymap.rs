//! C-KEYMAP: the host desktop's keyboard layout.
//!
//! winit turns the desktop's keymap into key events but does not expose it, so the shell's seat
//! (and with it the page and the shell's own keysym bindings) would run on xkbcommon's default
//! layout (XKB_DEFAULT_*, else us). This module reads the layout from the desktop itself, on a
//! worker thread, and sends the keymap text to the main loop, which gives it to the seat:
//!
//! - Wayland: a second `wl_keyboard` on the shell window's own connection (the same way
//!   smithay-clipboard reaches the desktop). The compositor sends `wl_keyboard.keymap` (fd +
//!   format) when the keyboard is created and again on every layout change.
//! - X11: the XKB extension on a connection of our own: the keymap of the core keyboard device
//!   comes from libxkbcommon-x11 (dlopen'd), and `NewKeyboardNotify` / `MapNotify` mean it changed.
//!
//! Everything that is not a socket is pure and unit-tested: reading the fd, trimming the text,
//! forgetting repeats (`Latest`).

use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;

use std::time::Instant;

use crate::clipboard::DesktopKind;

/// A keymap's text as the desktop sent it: Wayland's is NUL-terminated and may be padded; cut it at
/// the first NUL. None for an empty or non-UTF-8 text.
pub fn keymap_text(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let s = std::str::from_utf8(&bytes[..end]).ok()?;
    if s.trim().is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Reads `size` bytes at offset 0 of a `wl_keyboard.keymap` fd (pread: the fd's offset is shared
/// with the compositor's and may be at the end).
pub fn read_keymap_fd(fd: OwnedFd, size: u32) -> Result<String, String> {
    let file = std::fs::File::from(fd);
    let mut buf = vec![0u8; size as usize];
    let mut done = 0;
    while done < buf.len() {
        match file.read_at(&mut buf[done..], done as u64) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("reading the keymap: {e}")),
        }
    }
    keymap_text(&buf[..done]).ok_or_else(|| "the keymap is empty".to_string())
}

/// Remembers the last keymap given to the seat: a desktop sends the same keymap again on focus
/// changes (Wayland) or several notifications for one change (X11), and a repeat must not reset
/// the seat's modifier state.
#[derive(Default)]
pub struct Latest {
    last: Option<String>,
}

impl Latest {
    /// True when `text` differs from the last one (and remembers it).
    pub fn is_new(&mut self, text: &str) -> bool {
        if self.last.as_deref() == Some(text) {
            return false;
        }
        self.last = Some(text.to_string());
        true
    }

    /// Forgets `text` again (the seat refused it), so a later identical one is tried again.
    pub fn forget(&mut self) {
        self.last = None;
    }
}

/// What the desktop reports: a keymap (the first, and on every change of the layout list) or the
/// active layout group (Alt+Shift and the like switch between the layouts of one keymap).
#[derive(Debug, PartialEq)]
pub enum Update {
    Keymap(String),
    Group(u32),
    /// C3-SHIFTSYNC: the desktop's modifier masks (depressed, latched, locked), in the keymap's bits.
    Mods(u32, u32, u32),
    /// C-BARREPEAT: the desktop's key repeat: keys per second (0 = none) and the delay in ms.
    Repeat(u32, u32),
}

/// Starts the watcher thread; each keymap (the first, then every change) arrives on `tx`.
/// Failure is not fatal: the seat keeps xkbcommon's default keymap.
pub fn start(kind: DesktopKind, tx: calloop::channel::Sender<Update>) {
    let spawned = std::thread::Builder::new().name("web-keymap".into()).spawn(move || {
        crate::clipboard::block_quit_signals();
        let r = match kind {
            DesktopKind::Wayland(display) => wayland::watch(display, &tx),
            DesktopKind::X11 => x11::watch(&tx),
            DesktopKind::None => Ok(()),
        };
        if let Err(e) = r {
            eprintln!("web: the desktop keymap is not followed: {e} (the keyboard uses xkbcommon's default layout)");
        }
    });
    if let Err(e) = spawned {
        eprintln!("web: keymap thread: {e}");
    }
}

impl crate::state::State {
    /// Gives the desktop's keymap to the seat (smithay sends it on to the page). False when the seat
    /// could not compile it, so the same text is tried again if the desktop repeats it.
    pub fn set_desktop_keymap(&mut self, text: String) -> bool {
        let Some(keyboard) = self.seat.get_keyboard() else { return false };
        let name = keymap_name(&text);
        match keyboard.set_keymap_from_string(self, text) {
            Ok(()) => {
                self.log(&format!("desktop keymap: {name}"));
                true
            }
            Err(e) => {
                self.log(&format!("desktop keymap refused: {e:?}"));
                false
            }
        }
    }
}

impl crate::state::State {
    /// C-BARREPEAT: the desktop's repeat rate and delay for the shell's own lines.
    pub fn set_desktop_repeat(&mut self, rate: u32, delay_ms: u32) {
        self.keys.repeat_rate = rate;
        self.keys.repeat_delay_ms = delay_ms;
        self.log(&format!("desktop key repeat: {rate}/s after {delay_ms} ms"));
    }

    /// C-BARREPEAT: after a key event, starts the timer that repeats a held key of a shell line
    /// (winit gives no OS repeats). One timer at a time; it ends itself when nothing repeats.
    pub fn arm_key_repeat(&mut self) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        if self.keys.repeat_armed {
            return;
        }
        let Some(wait) = self.keys.repeat_wait(Instant::now()) else {
            return;
        };
        let armed = self.loop_handle.insert_source(Timer::from_duration(wait), |_, _, state| {
            match state.keys.repeat_tick(Instant::now()) {
                Some(wait) => {
                    let time = state.log_ms() as u32;
                    state.apply_key_effects(time);
                    TimeoutAction::ToDuration(wait)
                }
                None => {
                    state.keys.repeat_armed = false;
                    state.redraw();
                    TimeoutAction::Drop
                }
            }
        });
        self.keys.repeat_armed = armed.is_ok();
    }

    /// C-SCRIPT-MAP: after a key event, starts the timer that ends the wait for the rest of a user
    /// mapping (`MAP_TIMEOUT_MS`); the pending keys are then handled as without it. One timer at a
    /// time; it follows a deadline moved by later keys and ends itself when nothing waits.
    pub fn arm_map_timer(&mut self) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        if self.keys.map_armed {
            return;
        }
        let Some(wait) = self.keys.map_wait(Instant::now()) else {
            return;
        };
        let armed = self.loop_handle.insert_source(Timer::from_duration(wait), |_, _, state| {
            let left = state.keys.map_tick(Instant::now());
            let time = state.log_ms() as u32;
            state.apply_key_effects(time);
            match left {
                Some(wait) => TimeoutAction::ToDuration(wait),
                None => {
                    state.keys.map_armed = false;
                    TimeoutAction::Drop
                }
            }
        });
        self.keys.map_armed = armed.is_ok();
    }

    /// The desktop's active layout group (index into the keymap's layouts) goes to the seat.
    pub fn set_desktop_group(&mut self, group: u32) {
        use smithay::input::keyboard::Layout;
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        let changed = keyboard.with_xkb_state(self, |mut ctx| {
            let n = ctx.xkb().lock().unwrap().layouts().count() as u32;
            if group >= n || ctx.xkb().lock().unwrap().active_layout().0 == group {
                return false;
            }
            ctx.set_layout(Layout(group));
            true
        });
        if changed {
            self.log(&format!("desktop layout group: {group}"));
        }
    }
}

/// C3-SHIFTSYNC: the shell's modifier state for the desktop's masks (pure: tests use it).
pub fn mods_for_masks(
    keymap: &smithay::input::keyboard::xkb::Keymap,
    depressed: u32,
    latched: u32,
    locked: u32,
) -> smithay::input::keyboard::ModifiersState {
    let mut st = smithay::input::keyboard::xkb::State::new(keymap);
    st.update_mask(depressed, latched, locked, 0, 0, 0);
    let mut m = smithay::input::keyboard::ModifiersState::default();
    m.update_with(&st);
    m
}

/// The modifiers the bindings and the page look at.
pub fn same_mods(a: &smithay::input::keyboard::ModifiersState, b: &smithay::input::keyboard::ModifiersState) -> bool {
    (a.ctrl, a.alt, a.shift, a.caps_lock, a.logo, a.num_lock, a.iso_level3_shift)
        == (b.ctrl, b.alt, b.shift, b.caps_lock, b.logo, b.num_lock, b.iso_level3_shift)
}

impl crate::state::State {
    /// C3-SHIFTSYNC: the desktop's modifier state is the truth for the bindings (`KeyInput` reads the
    /// seat's modifiers): a modifier held when the window gained focus, or set by `wl_keyboard.modifiers`
    /// alone, never came as a key event. Applied only while no key is down: with keys down the key events
    /// (which this message may overtake or trail by a moment) are the better source. The page is told too.
    pub fn set_desktop_mods(&mut self, depressed: u32, latched: u32, locked: u32) {
        if !self.keys.idle() {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        let want = keyboard.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            // Safety: a read-only look at the seat's current keymap.
            mods_for_masks(unsafe { xkb.keymap() }, depressed, latched, locked)
        });
        if same_mods(&keyboard.modifier_state(), &want) {
            return;
        }
        keyboard.set_modifier_state(want);
        // NORMAL keeps the page's idea of the modifiers as it was (a scroll key given to it is unmodified, KEYS.md).
        let focus = if self.keys.key_mode == crate::keys::KeyMode::Input {
            keyboard.current_focus()
        } else {
            None
        };
        if let Some(focus) = focus {
            use smithay::input::keyboard::KeyboardTarget;
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            let seat = self.seat.clone();
            focus.modifiers(&seat, self, keyboard.modifier_state(), serial);
        }
        self.log(&format!(
            "desktop modifiers: shift={} ctrl={} alt={} logo={}",
            want.shift, want.ctrl, want.alt, want.logo
        ));
    }

    /// C-FOCUSKEYS: releases, in the page, every key it still has pressed (see `Keys::release_all`).
    pub fn release_pressed_keys(&mut self) {
        use smithay::backend::input::KeyState;
        use smithay::input::keyboard::{FilterResult, Keycode};
        let (codes, ours) = self.keys.release_all();
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        let time = self.log_ms() as u32;
        for code in &codes {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            keyboard.input::<(), _>(self, Keycode::new(*code), KeyState::Released, serial, time, |_, _, _| {
                FilterResult::Forward
            });
        }
        for code in &ours {
            keyboard.input_intercept(self, Keycode::new(*code), KeyState::Released, |_, _, _| ());
        }
        if !codes.is_empty() {
            self.log(&format!("focus lost: released {} key(s) in the page", codes.len()));
        }
        self.apply_key_effects(time);
    }
}

/// The `xkb_symbols "..."` name of a keymap, for the log (what the live check reads).
pub fn keymap_name(text: &str) -> String {
    text.lines()
        .find(|l| l.trim_start().starts_with("xkb_symbols"))
        .and_then(|l| l.split('"').nth(1))
        .unwrap_or("?")
        .to_string()
}

mod wayland {
    use super::*;
    use wayland_backend::client::Backend;
    use wayland_client::globals::{registry_queue_init, GlobalListContents};
    use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
    use wayland_client::protocol::wl_registry::WlRegistry;
    use wayland_client::protocol::wl_seat::{self, WlSeat};
    use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};

    struct Watch {
        tx: calloop::channel::Sender<Update>,
        keyboard: Option<WlKeyboard>,
    }

    impl Dispatch<WlRegistry, GlobalListContents> for Watch {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: <WlRegistry as wayland_client::Proxy>::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<WlSeat, ()> for Watch {
        fn event(w: &mut Self, seat: &WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
            if let wl_seat::Event::Capabilities {
                capabilities: WEnum::Value(caps),
            } = event
            {
                if caps.contains(wl_seat::Capability::Keyboard) && w.keyboard.is_none() {
                    w.keyboard = Some(seat.get_keyboard(qh, ()));
                }
            }
        }
    }

    impl Dispatch<WlKeyboard, ()> for Watch {
        fn event(w: &mut Self, _: &WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            if let wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } = event
            {
                let _ = w.tx.send(Update::Mods(mods_depressed, mods_latched, mods_locked));
                let _ = w.tx.send(Update::Group(group));
                return;
            }
            if let wl_keyboard::Event::RepeatInfo { rate, delay } = event {
                let _ = w.tx.send(Update::Repeat(rate.max(0) as u32, delay.max(0) as u32));
                return;
            }
            if let wl_keyboard::Event::Keymap { format, fd, size } = event {
                if format != WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    return;
                }
                match read_keymap_fd(fd, size) {
                    Ok(text) => {
                        let _ = w.tx.send(Update::Keymap(text));
                    }
                    Err(e) => eprintln!("web: desktop keymap: {e}"),
                }
            }
        }
    }

    pub fn watch(display: usize, tx: &calloop::channel::Sender<Update>) -> Result<(), String> {
        // Safety: the pointer is winit's live wl_display, which outlives the shell.
        let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
        let conn = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<Watch>(&conn).map_err(|e| format!("wayland registry: {e}"))?;
        let qh = queue.handle();
        let mut w = Watch {
            tx: tx.clone(),
            keyboard: None,
        };
        let _seat: WlSeat = globals
            .bind(&qh, 1..=7, ())
            .map_err(|e| format!("the desktop has no wl_seat: {e}"))?;
        loop {
            queue.blocking_dispatch(&mut w).map_err(|e| format!("wayland dispatch: {e}"))?;
        }
    }
}

mod x11 {
    use super::Update;
    use std::time::Duration;
    use x11rb::connection::Connection;
    use x11rb::protocol::xkb::{self, ConnectionExt as _};
    use x11rb::protocol::Event;
    use x11rb::xcb_ffi::XCBConnection;
    use xkbcommon_dl::x11::{xkb_x11_setup_xkb_extension_flags, xkbcommon_x11_option};
    use xkbcommon_dl::{xkb_context_flags, xkb_keymap_compile_flags, xkb_keymap_format, xkbcommon_handle};

    /// The core keyboard's keymap as text, via libxkbcommon-x11.
    fn fetch(conn: &XCBConnection) -> Result<String, String> {
        let x = xkbcommon_x11_option().ok_or("libxkbcommon-x11 is not installed")?;
        let c = xkbcommon_handle();
        let raw = conn.get_raw_xcb_connection();
        // Safety: libxkbcommon calls on a live xcb connection; every object made is released.
        unsafe {
            let mut sink = (0u16, 0u16, 0u8, 0u8);
            let ok = (x.xkb_x11_setup_xkb_extension)(
                raw,
                1,
                0,
                xkb_x11_setup_xkb_extension_flags::XKB_X11_SETUP_XKB_EXTENSION_NO_FLAGS,
                &mut sink.0,
                &mut sink.1,
                &mut sink.2,
                &mut sink.3,
            );
            if ok == 0 {
                return Err("the X server has no XKB extension".into());
            }
            let device = (x.xkb_x11_get_core_keyboard_device_id)(raw);
            if device < 0 {
                return Err("no core keyboard device".into());
            }
            let ctx = (c.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS);
            if ctx.is_null() {
                return Err("xkb_context_new failed".into());
            }
            let map = (x.xkb_x11_keymap_new_from_device)(ctx, raw, device, xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS);
            let out = if map.is_null() {
                Err("the X server's keymap did not compile".to_string())
            } else {
                let s = (c.xkb_keymap_get_as_string)(map, xkb_keymap_format::XKB_KEYMAP_FORMAT_TEXT_V1);
                let r = if s.is_null() {
                    Err("xkb_keymap_get_as_string failed".to_string())
                } else {
                    let text = std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned();
                    libc::free(s as *mut libc::c_void);
                    Ok(text)
                };
                (c.xkb_keymap_unref)(map);
                r
            };
            (c.xkb_context_unref)(ctx);
            out
        }
    }

    pub fn watch(tx: &calloop::channel::Sender<Update>) -> Result<(), String> {
        let (conn, _) = XCBConnection::connect(None).map_err(|e| format!("X11 connection: {e}"))?;
        conn.xkb_use_extension(1, 0)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| format!("XKB extension: {e}"))?;
        let parts = xkb::MapPart::KEY_TYPES
            | xkb::MapPart::KEY_SYMS
            | xkb::MapPart::MODIFIER_MAP
            | xkb::MapPart::EXPLICIT_COMPONENTS
            | xkb::MapPart::KEY_ACTIONS
            | xkb::MapPart::VIRTUAL_MODS
            | xkb::MapPart::VIRTUAL_MOD_MAP;
        // select_all: the events that need no details; the state notifications (group changes
        // only) take theirs from the aux.
        let full = xkb::EventType::NEW_KEYBOARD_NOTIFY | xkb::EventType::MAP_NOTIFY;
        conn.xkb_select_events(
            xkb::ID::USE_CORE_KBD.into(),
            xkb::EventType::from(0u16),
            full,
            parts,
            parts,
            &xkb::SelectEventsAux::new().state_notify(xkb::SelectEventsAuxStateNotify {
                affect_state: xkb::StatePart::GROUP_STATE | xkb::StatePart::MODIFIER_STATE,
                state_details: xkb::StatePart::GROUP_STATE | xkb::StatePart::MODIFIER_STATE,
            }),
        )
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("XKB select events: {e}"))?;
        let send = |conn: &XCBConnection| match fetch(conn) {
            Ok(t) => {
                let _ = tx.send(Update::Keymap(t));
            }
            Err(e) => eprintln!("web: desktop keymap: {e}"),
        };
        send(&conn);
        if let Ok(reply) = conn
            .xkb_get_controls(xkb::ID::USE_CORE_KBD.into())
            .map_err(|e| e.to_string())
            .and_then(|c| c.reply().map_err(|e| e.to_string()))
        {
            let _ = tx.send(Update::Repeat(
                1000 / u32::from(reply.repeat_interval).max(1),
                u32::from(reply.repeat_delay),
            ));
        }
        loop {
            let ev = conn.wait_for_event().map_err(|e| format!("X11: {e}"))?;
            if let Event::XkbStateNotify(s) = &ev {
                if s.changed.intersects(xkb::StatePart::MODIFIER_STATE) {
                    let _ = tx.send(Update::Mods(
                        u16::from(s.base_mods) as u32,
                        u16::from(s.latched_mods) as u32,
                        u16::from(s.locked_mods) as u32,
                    ));
                }
                if s.changed.contains(xkb::StatePart::GROUP_STATE) {
                    let _ = tx.send(Update::Group(u8::from(s.group) as u32));
                }
                continue;
            }
            if !matches!(ev, Event::XkbNewKeyboardNotify(_) | Event::XkbMapNotify(_)) {
                continue;
            }
            // One layout change is several notifications; let them all arrive, then read once.
            std::thread::sleep(Duration::from_millis(60));
            while let Some(_more) = conn.poll_for_event().map_err(|e| format!("X11: {e}"))? {}
            send(&conn);
        }
    }
}
