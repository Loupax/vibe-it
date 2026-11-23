//! The compositor: smithay state, the Wayland protocol handlers, and the engine host's
//! events. One page window (the host's toplevel paired with `tab-created`), any other
//! toplevel shown as a dialog over it, xdg popups placed inside the shell window.

use std::os::unix::net::UnixStream;
use std::process::Child;
use std::time::{Duration, Instant};

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::desktop::{
    find_popup_root_surface, get_popup_toplevel_coords, PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, PopupUngrabStrategy,
    Space, Window,
};
use smithay::input::pointer::{CursorIcon, CursorImageStatus, Focus};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::calloop::LoopSignal;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as ToplevelState;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::{wl_buffer, wl_output, wl_seat, wl_surface::WlSurface};
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::reexports::winit::window::Fullscreen;
use smithay::utils::{Logical, Point, Serial, Size};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler, CompositorState};
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::selection::data_device::{ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::tablet_manager::TabletSeatHandler;
use smithay::wayland::viewporter::ViewporterState;
use smithay::{
    delegate_compositor, delegate_cursor_shape, delegate_data_device, delegate_output, delegate_seat, delegate_shm, delegate_viewporter,
    delegate_xdg_decoration, delegate_xdg_shell,
};

use crate::layout::Layout;
use crate::pairing::{Pairing, Role};
use crate::protocol::{Command, Event};

pub struct State {
    pub start: Instant,
    pub display_handle: DisplayHandle,
    pub loop_signal: LoopSignal,
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    pub output: Output,
    pub space: Space<Window>,
    pub popups: PopupManager,
    pub seat: Seat<State>,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub _xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub _output_manager_state: OutputManagerState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,
    pub _viewporter_state: ViewporterState,
    pub _cursor_shape_state: CursorShapeManagerState,

    pub layout: Layout,
    pub pairing: Pairing<WlSurface>,
    /// Toplevels created but not committed yet (role decided at the first commit, when
    /// the client has set its parent).
    pub pending_toplevels: Vec<ToplevelSurface>,
    pub page: Option<Window>,
    /// The page window's size as last committed (logged when it changes).
    pub page_size: Size<i32, Logical>,
    pub dialogs: Vec<Window>,
    /// The shell window has the host desktop's keyboard focus.
    pub window_focused: bool,
    pub pointer_loc: Point<f64, Logical>,
    pub cursor: CursorImageStatus,
    pub cursor_log: String,
    pub over_bar: bool,
    pub url: String,
    pub title: String,
    pub dirty: bool,

    pub control: Option<UnixStream>,
    pub host: Option<Child>,
    pub quit_deadline: Option<Instant>,
    pub exit_code: i32,
    /// The status bar's pixels (C3).
    pub bar: crate::statusbar::Bar,
    /// The key model: modes, the `:` line, the status bar's text (C3).
    pub keys: crate::keys::Keys,
    pub history: crate::history::History,
    pub visits: crate::history::Visits,
    /// The last mouse press on a window (T29: a focus change soon after it is the user's).
    pub last_click: Option<Instant>,
    /// The status bar as last logged.
    pub bar_log: String,
    pub damage_tracker: Option<smithay::backend::renderer::damage::OutputDamageTracker>,
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

impl State {
    pub fn log_ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    pub fn redraw(&mut self) {
        self.dirty = true;
        // The status bar's state, logged when it changes (what the live check reads).
        let v = self.keys.bar_view();
        let desc = match &v.line {
            Some((p, text, _)) => format!("{p}{text}|{}", v.ghost),
            None => format!("{} | {}", v.mode, v.message),
        };
        if desc != self.bar_log {
            self.log(&format!("bar: {desc}"));
            self.bar_log = desc;
        }
    }

    fn window_of(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .cloned()
    }

    /// The window whose surface tree (popups excluded) holds `surface`.
    fn root_window(&self, surface: &WlSurface) -> Option<Window> {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        self.window_of(&root)
    }

    /// The window that should get the keyboard: the newest dialog, else the page.
    pub fn focus_target(&self) -> Option<WlSurface> {
        self.dialogs
            .iter()
            .rev()
            .chain(self.page.iter())
            .find(|w| has_buffer(w))
            .and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()))
    }

    /// Gives the keyboard to `focus_target` unless a popup grab holds it.
    pub fn refocus(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        if keyboard.is_grabbed() {
            return;
        }
        let want = if self.window_focused { self.focus_target() } else { None };
        if keyboard.current_focus() != want {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            keyboard.set_focus(self, want, serial);
        }
    }

    /// Places and sizes every window after a change of the shell window or fullscreen.
    pub fn relayout(&mut self) {
        let page_rect = self.layout.page();
        let fullscreen = self.layout.fullscreen;
        let activated = self.window_focused;
        if let Some(w) = self.page.clone() {
            let t = w.toplevel().unwrap();
            let wl_output = t.wl_surface().client().and_then(|c| self.output.client_outputs(&c).next());
            t.with_pending_state(|s| {
                s.size = Some(page_rect.size);
                s.decoration_mode = Some(DecorationMode::ServerSide);
                set_state(s, ToplevelState::Activated, activated);
                set_state(s, ToplevelState::Fullscreen, fullscreen);
                s.fullscreen_output = if fullscreen { wl_output.clone() } else { None };
                // Maximized: the client must take the configured size (a plain toplevel
                // may treat it as a hint; Chromium then keeps its preferred height).
                // Tiled on every side: no shadow, no rounded corners, no resize border.
                for e in [
                    ToplevelState::Maximized,
                    ToplevelState::TiledLeft,
                    ToplevelState::TiledRight,
                    ToplevelState::TiledTop,
                    ToplevelState::TiledBottom,
                ] {
                    set_state(s, e, !fullscreen);
                }
            });
            if t.is_initial_configure_sent() {
                t.send_pending_configure();
            }
            self.space.map_element(w, page_rect.loc, false);
        }
        for w in self.dialogs.clone() {
            let t = w.toplevel().unwrap();
            t.with_pending_state(|s| set_state(s, ToplevelState::Activated, activated));
            if t.is_initial_configure_sent() {
                t.send_pending_configure();
            }
            let loc = self.layout.dialog_loc(w.geometry().size);
            self.space.map_element(w, loc, false);
        }
        self.redraw();
    }

    /// The shell window was resized (physical pixels; scale 1 until C6).
    pub fn resized(&mut self, size: Size<i32, Logical>) {
        use smithay::output::Mode;
        let mode = Mode {
            size: (size.w, size.h).into(),
            refresh: 60_000,
        };
        self.output.change_current_state(Some(mode), None, None, None);
        self.output.set_preferred(mode);
        self.layout.window = size;
        // Left fullscreen from the desktop side (e.g. the window manager's key): tell the page.
        if self.layout.fullscreen && self.backend.window().fullscreen().is_none() {
            self.log("fullscreen off (by the desktop)");
            self.layout.fullscreen = false;
        }
        self.log(&format!("window {}x{}, page {:?}", size.w, size.h, self.layout.page()));
        self.relayout();
    }

    pub fn set_fullscreen(&mut self, on: bool) {
        if self.layout.fullscreen == on {
            return;
        }
        self.log(if on {
            "fullscreen on (page asked)"
        } else {
            "fullscreen off (page asked)"
        });
        self.layout.fullscreen = on;
        self.backend.window().set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
        self.relayout();
    }

    pub fn log(&self, msg: &str) {
        eprintln!("[web +{}ms] {msg}", self.log_ms());
    }

    fn update_window_title(&self) {
        let t = if !self.title.is_empty() { &self.title } else { &self.url };
        let t = if t.is_empty() { "web".to_string() } else { format!("{t} - web") };
        self.backend.window().set_title(&t);
    }

    // ---- engine host ----

    pub fn host_event(&mut self, line: &str) {
        let ev = match Event::parse(line) {
            Ok(ev) => ev,
            Err(e) => {
                self.log(&format!("host: {e}"));
                return;
            }
        };
        self.log(&format!("host event: {line}"));
        if let Some((url, title)) = self.visits.event(&ev) {
            self.history.record(&url, &title);
        }
        match ev {
            Event::Hello { protocol, engine } => {
                if protocol != 0 {
                    self.log(&format!("host speaks protocol {protocol} ({engine}); this shell knows 0"));
                }
            }
            Event::TabCreated { tab } => match self.pairing.tab_created(tab) {
                Ok(true) => self.log(&format!("page window paired with tab {tab}")),
                Ok(false) => {}
                Err(e) => self.log(&e),
            },
            Event::TabClosed { tab } => {
                self.pairing.tab_closed(tab);
            }
            Event::Url { url, .. } => {
                self.url = url.clone();
                self.keys.url = url;
                self.update_window_title();
            }
            Event::Title { title, .. } => {
                self.title = title;
                self.update_window_title();
            }
            // T31: a navigation ends the find session (the host dropped it); the query stays.
            Event::Load { ref phase, ref kind, .. } if phase == "start" && kind != "same-document" => self.keys.navigated(),
            Event::FindResult { index, count, .. } => self.keys.find_result(index, count),
            // A page that loaded clears the last message (a failed load keeps its error).
            Event::Load {
                ref phase,
                ref kind,
                ref error,
                ..
            } if phase == "end" && kind == "new" && error.is_none() => self.keys.message.clear(),
            Event::LoadError { code, ref name, .. } => {
                // ERR_ABORTED is a stopped load (or a download), not a failure to report.
                if code != -3 {
                    self.keys.message = format!("web: load failed: {name} ({code})");
                }
            }
            Event::EditableFocused { focused: true, .. } => {
                // T29 (a click into a text field) switches to INPUT; T32 (the page's own
                // focus(), autofocus) does not.
                if crate::keys::click_recent(self.last_click, Instant::now()) {
                    self.log("editable focused after a click: INPUT");
                    self.keys.user_focused_control();
                }
            }
            Event::Crashed { ref reason, .. } => {
                self.keys.message = format!("web: the page crashed ({reason}); :refresh loads it again");
            }
            Event::Error { ref message } => self.keys.message = format!("web: host: {message}"),
            _ => {}
        }
        self.redraw();
    }

    pub fn send(&mut self, cmd: Command) {
        use std::io::Write;
        if cmd != Command::Quit {
            self.log(&format!("command: {}", cmd.line().trim_end()));
        }
        if let Some(c) = &mut self.control {
            if let Err(e) = c.write_all(cmd.line().as_bytes()) {
                self.log(&format!("control socket: {e}"));
                self.control = None;
            }
        }
    }

    /// Asks the host to quit; the shell exits when it has (or kills it after 5 s).
    pub fn quit(&mut self, why: &str) {
        if self.quit_deadline.is_some() {
            return;
        }
        self.log(&format!("quitting: {why}"));
        self.send(Command::Quit);
        self.quit_deadline = Some(Instant::now() + Duration::from_secs(5));
    }

    /// Called from a timer: stops the loop once the host has exited.
    pub fn watch_host(&mut self) {
        let Some(child) = &mut self.host else {
            self.loop_signal.stop();
            return;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                self.log(&format!("engine host exited: {status}"));
                self.exit_code = if self.quit_deadline.is_some() {
                    0
                } else {
                    status.code().unwrap_or(1).max(1)
                };
                self.host = None;
                self.loop_signal.stop();
            }
            Ok(None) => {
                if self.quit_deadline.is_some_and(|d| Instant::now() > d) {
                    let _ = child.kill();
                    let pid = child.id();
                    self.log(&format!("engine host {pid} did not quit; killed it"));
                }
            }
            Err(e) => {
                self.log(&format!("engine host: {e}"));
                self.host = None;
                self.loop_signal.stop();
            }
        }
    }

    // ---- cursor ----

    pub fn apply_cursor(&mut self) {
        let status = if self.over_bar {
            CursorImageStatus::Named(CursorIcon::Default)
        } else {
            self.cursor.clone()
        };
        let w = self.backend.window();
        let desc = match &status {
            CursorImageStatus::Named(icon) => {
                w.set_cursor(*icon);
                w.set_cursor_visible(true);
                format!("named {}", icon.name())
            }
            CursorImageStatus::Hidden => {
                w.set_cursor_visible(false);
                "hidden".to_string()
            }
            CursorImageStatus::Surface(_) => {
                w.set_cursor_visible(false);
                "surface (drawn by the shell)".to_string()
            }
        };
        if desc != self.cursor_log {
            self.log(&format!("cursor: {desc}"));
            self.cursor_log = desc;
        }
        self.redraw();
    }

    // ---- toplevels ----

    /// First commit of a toplevel: decide whether it is the page or a dialog.
    fn classify(&mut self, t: ToplevelSurface) {
        let surface = t.wl_surface().clone();
        let role = self.pairing.toplevel(surface, t.parent().is_some());
        let window = Window::new_wayland_window(t.clone());
        let app_id = smithay::wayland::compositor::with_states(t.wl_surface(), |s| {
            s.data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().unwrap().app_id.clone())
        });
        match role {
            Role::Page => {
                self.log(&format!(
                    "page window (app_id {:?}){}",
                    app_id.unwrap_or_default(),
                    match self.pairing.paired_tab() {
                        Some(tab) => format!(", paired with tab {tab}"),
                        None => ", waiting for tab-created".into(),
                    }
                ));
                self.page = Some(window);
            }
            Role::Dialog => {
                self.log(&format!("dialog window (app_id {:?})", app_id.unwrap_or_default()));
                t.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
                self.dialogs.push(window);
            }
        }
        self.relayout();
        t.send_configure();
    }
}

fn set_state(s: &mut smithay::wayland::shell::xdg::ToplevelState, st: ToplevelState, on: bool) {
    if on {
        s.states.set(st);
    } else {
        s.states.unset(st);
    }
}

fn has_buffer(w: &Window) -> bool {
    w.toplevel()
        .map(|t| with_renderer_surface_state(t.wl_surface(), |s| s.buffer().is_some()).unwrap_or(false))
        .unwrap_or(false)
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            if let Some(w) = self.root_window(surface) {
                w.on_commit();
            }
        }
        if let Some(i) = self.pending_toplevels.iter().position(|t| t.wl_surface() == surface) {
            let t = self.pending_toplevels.remove(i);
            self.classify(t);
        } else if self
            .page
            .as_ref()
            .is_some_and(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
        {
            let size = self.page.as_ref().unwrap().geometry().size;
            if size != self.page_size {
                self.page_size = size;
                self.log(&format!("page window is {}x{}", size.w, size.h));
            }
        } else if let Some(w) = self
            .dialogs
            .iter()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .cloned()
        {
            // Its size is known only now; keep it centred.
            let loc = self.layout.dialog_loc(w.geometry().size);
            self.space.map_element(w, loc, false);
        }
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(p)) = self.popups.find_popup(surface) {
            if !p.is_initial_configure_sent() {
                let _ = p.send_configure();
            }
        }
        self.refocus();
        self.redraw();
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.pending_toplevels.push(surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        // T29: a popup right after a click is a <select>'s menu (or a context menu): its
        // keys must reach the page, so INPUT.
        if crate::keys::click_recent(self.last_click, Instant::now()) && self.keys.key_mode != crate::keys::KeyMode::Input {
            self.log("popup after a click: INPUT");
            self.keys.user_focused_control();
        }
        self.unconstrain_popup(&surface);
        let g = surface.with_pending_state(|s| s.geometry);
        self.log(&format!(
            "popup at {},{} {}x{} (parent coordinates)",
            g.loc.x, g.loc.y, g.size.w, g.size.h
        ));
        if let Err(e) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            self.log(&format!("popup: {e:?}"));
        }
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|s| s.positioner = positioner);
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::<State>::from_resource(&seat) else { return };
        let kind = PopupKind::Xdg(surface);
        let Some(root) = find_popup_root_surface(&kind).ok().and_then(|r| self.window_of(&r)) else {
            return;
        };
        let root = root.toplevel().unwrap().wl_surface().clone();
        let Ok(mut grab) = self.popups.grab_popup(root, kind, &seat, serial) else {
            return;
        };
        self.log("popup grab (keyboard and pointer)");
        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed() && !(keyboard.has_grab(serial) || keyboard.has_grab(grab.previous_serial().unwrap_or(serial))) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial) || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<wl_output::WlOutput>) {
        if self.pairing.is_page(surface.wl_surface()) {
            self.set_fullscreen(true);
        } else if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if self.pairing.is_page(surface.wl_surface()) {
            self.set_fullscreen(false);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        // The page is always as large as it can be; answer with the current state.
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let s = surface.wl_surface();
        self.pending_toplevels.retain(|t| t.wl_surface() != s);
        if let Some(w) = self.window_of(s) {
            self.space.unmap_elem(&w);
        }
        if self.pairing.toplevel_destroyed(s) {
            self.log("page window closed");
            self.page = None;
            if self.layout.fullscreen {
                self.set_fullscreen(false);
            }
        }
        self.dialogs.retain(|w| w.toplevel().is_some_and(|t| t.wl_surface() != s));
        self.refocus();
        self.redraw();
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        self.log("popup closed");
        self.redraw();
    }
}

impl State {
    /// Keeps a popup inside the shell window (xdg_positioner constraint adjustment).
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Some(window) = find_popup_root_surface(&kind).ok().and_then(|r| self.window_of(&r)) else {
            return;
        };
        let root_loc = self.space.element_location(&window).unwrap_or_default() - window.geometry().loc;
        let target = self.layout.popup_target(root_loc, get_popup_toplevel_coords(&kind));
        popup.with_pending_state(|s| s.geometry = s.positioner.get_unconstrained_geometry(target));
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        // The shell draws no decorations and wants none drawn by the page.
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.request_mode(toplevel, DecorationMode::ServerSide);
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor = image;
        self.apply_cursor();
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}
}

impl TabletSeatHandler for State {}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}

impl OutputHandler for State {}

delegate_compositor!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
delegate_xdg_decoration!(State);
delegate_seat!(State);
delegate_data_device!(State);
delegate_output!(State);
delegate_viewporter!(State);
delegate_cursor_shape!(State);
