//! The compositor: smithay state, the Wayland protocol handlers, and the engine host's
//! events. The page windows (C4: one per buffer, each paired with its `tab-created`; only the
//! current buffer's is shown), any other toplevel shown as a dialog over it, xdg popups placed
//! inside the shell window.

use std::os::unix::net::UnixStream;
use std::process::Child;
use std::time::Instant;

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
use smithay::utils::{Logical, Physical, Point, Serial, Size};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler, CompositorState};
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::selection::data_device::{ClientDndGrabHandler, DataDeviceState, ServerDndGrabHandler};
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::tablet_manager::TabletSeatHandler;
use smithay::wayland::viewporter::ViewporterState;
use smithay::{
    delegate_compositor, delegate_cursor_shape, delegate_output, delegate_seat, delegate_shm, delegate_viewporter, delegate_xdg_decoration,
    delegate_xdg_shell,
};

use crate::layout::Layout;
use crate::pairing::Pairing;
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
    /// C6d: zwp_linux_dmabuf_v1, when the shell's renderer can import (`dmabuf.rs`).
    pub dmabuf: crate::dmabuf::Dmabuf,
    /// C-FLICKER: linux-drm-syncobj-v1 (`syncobj.rs`), when offered.
    pub syncobj: Option<smithay::wayland::drm_syncobj::DrmSyncobjState>,
    /// C-FLICKER: buffers held until the GPU finished the renders that read them (`sync.rs`),
    /// and whether a timer is already polling for that.
    pub held: crate::sync::Held,
    pub held_timer: bool,
    /// C-FLICKER: the acquire and release guards are on (not `WEB_NO_BUFFER_SYNC`).
    pub buffer_sync: bool,
    pub _output_manager_state: OutputManagerState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    /// The clipboard bridge between the page and the desktop (C6, `selection.rs`).
    pub bridge: crate::selection::Bridge,
    pub _viewporter_state: ViewporterState,
    /// C6b: wp_fractional_scale_v1 (with wp_viewporter, how a client draws at 1.25, 1.5, ...).
    pub _fractional_scale_state: smithay::wayland::fractional_scale::FractionalScaleManagerState,
    pub _cursor_shape_state: CursorShapeManagerState,
    /// C6e: zwp_text_input_manager_v3 for the page (`ime.rs`).
    pub _text_input_global: smithay::reexports::wayland_server::backend::GlobalId,
    /// C6h: wp_presentation, and the clock its feedback times are on.
    pub _presentation_state: smithay::wayland::presentation::PresentationState,
    pub presentation_clock: smithay::utils::Clock<smithay::utils::Monotonic>,

    pub layout: Layout,
    pub pairing: Pairing<WlSurface>,
    /// Toplevels created but not committed yet (role decided at the first commit, when
    /// the client has set its parent).
    pub pending_toplevels: Vec<ToplevelSurface>,
    /// The shown page window: the current buffer's (C4; `sync_page`).
    pub page: Option<Window>,
    /// Every page window (C4: one per buffer) and its tab. Only `page` is mapped in the Space;
    /// the others are sized to the page area too but not drawn (the host hides them).
    pub pages: Vec<(u64, Window)>,
    /// The buffer and tab last shown (logged when it changes).
    pub shown: (u32, Option<u64>),
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
    /// C6d: a dmabuf from the page was imported (logged once).
    pub dmabuf_seen: bool,
    /// C6c: frame pacing (`pacing.rs`): the outer monitor's refresh, the last render, and
    /// whether a timer is already waiting to release a render that came too soon.
    pub pacer: crate::pacing::Pacer,
    pub pace_timer: bool,
    /// C6c: renders counted since the last `frames:` line (only with WEB_FRAME_STATS=1).
    pub frame_stats: Option<(Instant, u32)>,
    /// C-FLICKER: `WEB_PROBE` rectangles read back after every render (`probe.rs`), and the frame count.
    pub probe: Option<(Vec<crate::probe::Rect>, u64)>,

    pub control: Option<UnixStream>,
    /// `web --forget-site` clients waiting for their `forgot` (site as sent, the client's socket).
    pub forget_clients: Vec<(String, UnixStream)>,
    pub host: Option<Child>,
    /// C-CLOSE-POLITE: a close was asked (`close_politely` or `terminate`). It stays set after a
    /// cancelled polite close; it only changes `tab-closed` handling and the exit code.
    pub closing: bool,
    pub exit_code: i32,
    /// The status bar's pixels (C3).
    pub bar: crate::statusbar::Bar,
    /// The picker's pixels (C-PICK).
    pub pick_overlay: crate::picker::Overlay,
    /// T66: a mouse button whose press closed the picker over its list: neither it nor its
    /// release reaches the page.
    pub swallowed_button: Option<u32>,
    /// C-NORMALMODS: buttons pressed in NORMAL with modifiers sent to the page, not yet released.
    pub click_mods_out: u32,
    /// The key model: modes, the `:` line, the status bar's text (C3).
    pub keys: crate::keys::Keys,
    pub history: crate::history::History,
    /// Visits per page (C4: pages load side by side).
    pub visits: std::collections::HashMap<u64, crate::history::Visits>,
    /// The last mouse press on a window (T29: a focus change soon after it is the user's).
    pub last_click: Option<Instant>,
    /// The status bar as last logged.
    pub bar_log: String,
    pub damage_tracker: Option<smithay::backend::renderer::damage::OutputDamageTracker>,
    /// C-ENTER: key effects waiting behind an `Effect::Pause` (kept in order) and whether its
    /// timer is running (`input.rs` `apply_key_effects`).
    pub deferred: std::collections::VecDeque<crate::keys::Effect>,
    pub paused: bool,
    pub loop_handle: smithay::reexports::calloop::LoopHandle<'static, State>,
    /// C6e: the page's text inputs and the desktop's input method (`ime.rs`).
    pub ime: crate::ime::Ime,
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
        // C6e: the key mode may have changed: the desktop's input method follows INPUT.
        self.ime_sync();
        // The status bar's state, logged when it changes (what the live check reads).
        let v = self.keys.bar_view();
        let desc = match &v.line {
            Some((p, text, _, sel)) => {
                let sel = sel.map(|(a, b)| format!(" [selected {a}..{b}]")).unwrap_or_default();
                // C-PICKMSG: the message drawn beside the PICK filter line.
                let msg = if *p == '>' && !v.message.is_empty() {
                    format!(" msg: {}", v.message)
                } else {
                    String::new()
                };
                format!("{p}{text}|{}{}{sel}{msg}", v.ghost, self.keys.pick_log())
            }
            None => format!("{} | {}{}", v.mode, v.message, self.keys.pick_log()),
        };
        // C4: the URL part (with the buffer number) on a line of its own, when it changes.
        let full = format!("{desc}\n{}", v.url);
        if full != self.bar_log {
            let (old_desc, old_url) = self.bar_log.split_once('\n').unwrap_or((self.bar_log.as_str(), ""));
            let (desc_changed, url_changed) = (desc != old_desc, v.url != old_url);
            if desc_changed {
                self.log(&format!("bar: {desc}"));
            }
            if url_changed {
                self.log(&format!("bar url: {}", v.url));
            }
            self.bar_log = full;
        }
    }

    fn window_of(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .chain(self.pages.iter().map(|(_, w)| w))
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .cloned()
    }

    /// C4: shows the current buffer's page window (once it is paired) and nothing else of the
    /// pages; logs a change of buffer. Called after key effects and host events.
    pub fn sync_page(&mut self) {
        let num = self.keys.buffers.current;
        let tab = self.keys.buffers.current_tab();
        let want = tab.and_then(|t| self.pages.iter().find(|(x, _)| *x == t).map(|(_, w)| w.clone()));
        let changed = want != self.page;
        if (num, tab) != self.shown {
            self.shown = (num, tab);
            self.log(&format!(
                "buffer {num} of {} is current (tab {})",
                self.keys.buffers.list.len(),
                tab.map(|t| t.to_string()).unwrap_or_else(|| "not created yet".into())
            ));
            self.url = self.keys.url.clone();
            self.title = self.keys.title.clone();
            self.update_window_title();
        }
        if !changed {
            return;
        }
        if self.layout.fullscreen {
            self.set_fullscreen(false);
        }
        if let Some(w) = &self.page {
            self.space.unmap_elem(w);
        }
        self.page = want;
        self.page_size = (0, 0).into();
        if let Some(t) = tab.filter(|_| self.page.is_some()) {
            self.log(&format!("buffer {num} shown: page window of tab {t}"));
        }
        self.relayout();
        self.refocus();
    }

    /// C4: `tab-created`: the page joins its buffer (shown if that buffer is current) and its window,
    /// if it came first. C-POPUPS: a popup (`popup`: its opener and cause) gets a new buffer, shown.
    fn tab_created(&mut self, tab: u64, req: Option<u64>, popup: Option<(u64, String)>) {
        if let Some((opener, cause)) = popup {
            let num = self.keys.popup_created(tab, opener, &cause);
            self.log(&format!("tab {tab} is a popup ({cause}) of tab {opener}: buffer {num}"));
            self.apply_key_effects(0);
        } else {
            self.page_created(tab, req);
        }
        if let Some(s) = self.pairing.tab_created(tab) {
            if let Some(i) = self.dialogs.iter().position(|w| w.toplevel().is_some_and(|t| t.wl_surface() == &s)) {
                let w = self.dialogs.remove(i);
                self.space.unmap_elem(&w);
                self.pages.push((tab, w));
            }
            self.log(&format!("page window paired with tab {tab}"));
        }
        self.sync_page();
        // C-SCRIPT: the first tab means the host is connected and the first buffer exists: `ready`
        // (once per start; `script_ready` does nothing after the first time).
        if self.keys.scripts.is_some() {
            self.keys.script_ready();
            self.apply_key_effects(0);
        }
    }

    /// A page that is not a popup: the first one, `new-tab`'s answer, or one nobody asked for.
    fn page_created(&mut self, tab: u64, req: Option<u64>) {
        use crate::buffers::TabCreated;
        match self.keys.buffers.tab_created(tab, req) {
            TabCreated::Buffer(n) => {
                self.log(&format!("tab {tab} is buffer {n}'s page"));
                if n == self.keys.buffers.current && req.is_some() {
                    self.send(Command::ShowTab { tab });
                }
            }
            TabCreated::Adopted(n) => self.log(&format!("tab {tab} was not asked for: buffer {n}")),
            TabCreated::Doomed => {
                self.log(&format!("tab {tab}: its buffer was killed meanwhile; closing it"));
                self.send(Command::CloseTab { tab });
            }
        }
    }

    /// C4: a page event's effect on its buffer (`Keys::page_event`). True: not the current buffer's,
    /// nothing else is done with it.
    fn buffer_event(&mut self, tab: u64, ev: &Event) -> bool {
        self.keys.page_event(tab, ev)
    }

    /// Sends a page command to page `tab` (not necessarily the shown one).
    pub fn send_to(&mut self, tab: u64, cmd: Command) {
        use std::io::Write;
        let line = cmd.line_for(tab);
        self.log(&format!("command: {}", line.trim_end()));
        if let Some(c) = &mut self.control {
            if let Err(e) = c.write_all(line.as_bytes()) {
                self.log(&format!("control socket: {e}"));
                self.control = None;
            }
        }
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
        let window_focused = self.window_focused;
        // C4: every page window gets the page area (switching does not re-layout the page); only
        // the shown one is mapped and activated.
        for w in self.pages.iter().map(|(_, w)| w.clone()).collect::<Vec<_>>() {
            let shown = self.page.as_ref() == Some(&w);
            let activated = window_focused && shown;
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
            if shown {
                self.space.map_element(w, page_rect.loc, false);
            } else {
                self.space.unmap_elem(&w);
            }
        }
        let activated = window_focused;
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

    /// The shell window was resized or its scale changed (winit: physical pixels and the
    /// desktop's scale factor for the window, e.g. after moving to another monitor).
    pub fn resized(&mut self, size: Size<i32, Physical>, scale_factor: f64) {
        let old_scale = self.layout.scale;
        self.layout.set_physical(size, scale_factor);
        self.apply_output_state();
        // Left fullscreen from the desktop side (e.g. the window manager's key): tell the page.
        if self.layout.fullscreen && self.backend.window().fullscreen().is_none() {
            self.log("fullscreen off (by the desktop)");
            self.layout.fullscreen = false;
        }
        let l = self.layout.window;
        self.log(&format!(
            "window {}x{}, page {:?}, scale {} ({}x{} px)",
            l.w,
            l.h,
            self.layout.page(),
            self.layout.scale,
            size.w,
            size.h
        ));
        if self.layout.scale != old_scale {
            self.log(&format!("scale {} -> {}", old_scale, self.layout.scale));
            self.send_scale_to_all();
        }
        self.relayout();
    }

    /// The inner output mirrors the shell window: its mode is the framebuffer (physical
    /// pixels), its scale the window's. smithay sends wl_output.scale (the scale rounded
    /// up) and xdg_output's logical size; fractional-scale-v1 carries the exact one.
    pub fn apply_output_state(&mut self) {
        use smithay::output::{Mode, Scale};
        // C6c: the page is told the outer monitor's refresh rate (60 Hz when the desktop says
        // none), so its animations and requestAnimationFrame run at the outer frame rate.
        let refresh = crate::pacing::sane_refresh(self.backend.window().current_monitor().and_then(|m| m.refresh_rate_millihertz()));
        if refresh != self.pacer.refresh_mhz() {
            self.log(&format!("output refresh {refresh} mHz"));
            self.pacer.set_refresh(refresh);
        }
        let mode = Mode {
            size: self.layout.physical,
            refresh: refresh as i32,
        };
        self.output
            .change_current_state(Some(mode), None, Some(Scale::Fractional(self.layout.scale)), None);
        self.output.set_preferred(mode);
    }

    /// Tells a surface tree the scale to draw at: fractional-scale-v1's preferred scale and
    /// wl_surface v6's preferred_buffer_scale (the integer one). Both are sent only when
    /// they change, so this is cheap on every commit.
    pub fn send_scale(&self, surface: &WlSurface) {
        use smithay::wayland::compositor::{send_surface_state, with_surface_tree_downward, TraversalAction};
        use smithay::wayland::fractional_scale::with_fractional_scale;
        let scale = self.layout.scale;
        let int = scale.ceil() as i32;
        with_surface_tree_downward(
            surface,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |s, states, _| {
                with_fractional_scale(states, |fs| fs.set_preferred_scale(scale));
                // The output's Flipped180 is winit's GL quirk, not something clients should draw.
                send_surface_state(s, states, int, smithay::utils::Transform::Normal);
            },
            |_, _, _| true,
        );
    }

    /// After a scale change: every window, popup and the cursor surface.
    pub fn send_scale_to_all(&self) {
        for w in self.space.elements() {
            if let Some(t) = w.toplevel() {
                self.send_scale(t.wl_surface());
                for (p, _) in PopupManager::popups_for_surface(t.wl_surface()) {
                    self.send_scale(p.wl_surface());
                }
            }
        }
        if let CursorImageStatus::Surface(s) = &self.cursor {
            self.send_scale(s);
        }
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
        // One write per line: the host's log shares the pipe (C4 saw two lines interleaved).
        use std::io::Write as _;
        let line = format!("[web +{}ms] {msg}\n", self.log_ms());
        let _ = std::io::stderr().write_all(line.as_bytes());
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
        // C-BELL: a notification of any buffer, shown or not.
        if let Event::Notify {
            tab,
            title,
            body,
            tag,
            origin,
        } = ev
        {
            self.notified(tab, crate::notify::Note { title, body, tag, origin });
            return;
        }
        // C-AUTOFOCUS (KEYS.md T29, T32; user, 2026-11-24): a field focus the user caused switches to
        // INPUT; the page's own (or a background buffer's) leaves the mode and parks the field. A blur
        // (`focused: false`) never changes the mode.
        if let Event::EditableFocused { tab, focused: true } = ev {
            let now = Instant::now();
            let after_click = crate::keys::click_recent(self.last_click, now);
            let outcome = match self.keys.editable_focused(tab, after_click, now) {
                crate::keys::FocusOutcome::Input => "INPUT",
                crate::keys::FocusOutcome::AlreadyInput => "already INPUT",
                crate::keys::FocusOutcome::Parked => "the page's own: parked (blur-field), mode stays",
                crate::keys::FocusOutcome::Kept => "the page's own again: kept (refocus guard), mode stays",
                crate::keys::FocusOutcome::Ignored => "no buffer has the page: ignored",
            };
            self.log(&format!("editable focused: {outcome} (tab {tab})"));
            self.apply_key_effects(0);
            return;
        }
        if let Event::FieldResult { tab, ref op, ok } = ev {
            self.keys.field_result(tab, op, ok);
            self.apply_key_effects(0);
            return;
        }
        // C4: page events of every page keep their buffer up to date; only the current buffer's
        // reach the status bar, the search and the mode below.
        if let Some(tab) = ev.page_tab() {
            if let Some((url, title)) = self.visits.entry(tab).or_default().event(&ev) {
                self.history.record(&url, &title);
            }
            if self.buffer_event(tab, &ev) {
                self.redraw();
                return;
            }
        }
        match ev {
            Event::Hello { protocol, engine } => {
                if protocol != 1 {
                    self.log(&format!("host speaks protocol {protocol} ({engine}); this shell knows 1"));
                }
            }
            Event::TabCreated {
                tab,
                req,
                opener,
                ref cause,
            } => {
                let popup = opener.map(|o| (o, cause.clone().unwrap_or_else(|| "popup".into())));
                self.tab_created(tab, req, popup)
            }
            Event::PopupBlocked {
                tab,
                popup,
                ref url,
                ref name,
                ref frame,
                ..
            } => {
                let b = crate::popups::BlockedPopup {
                    popup,
                    url: url.clone(),
                    name: name.clone(),
                    in_frame: frame == "sub",
                };
                if let Some(m) = self.keys.popup_blocked(tab, b) {
                    self.keys.message = m;
                }
            }
            // C-MIDCLICK (KEYS.md T105): a middle-click on a link of the page on screen is `:new <url>`.
            // C-FOCUSEXISTING (T170): a plain click on a target=_blank link is too, unless
            // `web.buffers.focusExisting` finds a buffer already showing the URL (`Keys::link_click`).
            Event::OpenLink { tab, ref url } | Event::BlankLink { tab, ref url } => {
                let blank = matches!(ev, Event::BlankLink { .. });
                if self.keys.link_click(tab, url, blank) {
                    self.apply_key_effects(0);
                    self.sync_page();
                } else {
                    let name = if blank { "blank-link" } else { "open-link" };
                    self.log(&format!("{name} from tab {tab}, not the page on screen: ignored"));
                }
            }
            // C-FILEPICK (KEYS.md T171): a page asks for a file chooser; the picker answers it.
            Event::FileDialog {
                tab,
                id,
                mode,
                ref title,
                ref default,
                ref accept,
            } => {
                self.keys.file_dialog(tab, id, mode, title, default, accept.clone());
                self.apply_key_effects(0);
                self.sync_page();
            }
            Event::TabClosed { tab } => {
                self.pairing.tab_closed(tab);
                self.visits.remove(&tab);
                // Not killed by the shell: the page closed itself, or a polite close closed it.
                if self.keys.tab_closed(tab, self.closing) {
                    self.apply_key_effects(0);
                }
                self.sync_page();
            }
            Event::Url { url, original, .. } => {
                self.url = url.clone();
                self.keys.url = url;
                self.keys.original_url = original;
                self.update_window_title();
            }
            Event::Title { title, .. } => {
                self.keys.title = title.clone();
                self.title = title;
                self.update_window_title();
            }
            // T31: a navigation ends the find session (the host dropped it); the query stays.
            // C-AUTOFOCUS: a field the new document focuses is not the click's (nor a popup it opens).
            Event::Load { ref phase, ref kind, .. } if phase == "start" && kind != "same-document" => {
                self.last_click = None;
                self.keys.navigated()
            }
            // C5-FRAG: a same-document navigation (#fragment, pushState) gets no cross-document
            // load, so the host's session is still there: end it (find-clear) like T7 does.
            Event::Load { ref phase, ref kind, .. } if phase == "start" && kind == "same-document" => {
                self.keys.navigated_same_document();
                // The find-clear it queued is not a key's effect: send it now.
                self.apply_key_effects(0);
            }
            Event::FindResult {
                id, index, count, wrapped, ..
            } => self.keys.find_result_wrapped(id, index, count, wrapped),
            // A page that loaded clears the last message (a failed load keeps its error), except a script's
            // error or an `:echo` (C-SCRIPT-ERRBAR: they stay until the user's next key).
            Event::Load {
                ref phase,
                ref kind,
                ref error,
                ..
            } if phase == "end" && kind == "new" && error.is_none() => self.keys.page_loaded(),
            Event::LoadError { code, ref name, .. } => {
                // ERR_ABORTED is a stopped load (or a download), not a failure to report.
                if code != -3 {
                    self.keys.message = format!("web: load failed: {name} ({code})");
                }
            }
            Event::Crashed { ref reason, .. } => {
                self.keys.message = format!("web: the page crashed ({reason}); :refresh loads it again");
            }
            Event::Error { ref message } => {
                self.keys.message = format!("web: host: {message}");
                // A refused `forget-site` has no `forgot`: the oldest waiting client gets the error.
                if !self.forget_clients.is_empty() {
                    let (_, c) = self.forget_clients.remove(0);
                    reply_to(c, line);
                }
            }
            Event::Forgot {
                ref site,
                ref domain,
                ref result,
                ref failed,
                cookies,
            } => {
                self.keys.message = if result == "ok" {
                    format!("forgot {site} ok ({cookies} cookies)")
                } else {
                    format!("forgot {site} {result} {}", failed.join(","))
                };
                if let Some(i) = self.forget_clients.iter().position(|(s, _)| s == site) {
                    let (_, c) = self.forget_clients.remove(i);
                    reply_to(c, line);
                }
                // C4: every buffer that shows the forgotten site (or a host under it) loads again.
                for (num, tab, h) in self.keys.buffers_on_site(domain) {
                    self.log(&format!("reloading {h} (buffer {num}): its site {domain} was forgotten"));
                    self.send_to(tab, Command::Reload { hard: false });
                }
            }
            _ => {}
        }
        self.redraw();
    }

    /// C-BELL: page `tab` showed a notification: unread on its buffer unless shown, and the visual
    /// bell (rate-limited); a timer puts the bar back after `BELL_FLASH_MS`.
    fn notified(&mut self, tab: u64, note: crate::notify::Note) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        if !self.keys.notified(tab, note) {
            self.log(&format!("notify from tab {tab}, which has no buffer: ignored"));
            return;
        }
        if let Some(d) = self.keys.bell.ring(Instant::now()) {
            self.log("bell: flash");
            let armed = self.loop_handle.insert_source(Timer::from_duration(d), |_, _, state| {
                state.keys.bell.done();
                state.log("bell: off");
                state.redraw();
                TimeoutAction::Drop
            });
            if armed.is_err() {
                self.keys.bell.done();
            }
        }
        self.redraw();
    }

    /// A `web --forget-site` client asked (forget.rs): pass it to the host, answer when it has.
    pub fn forget_requested(&mut self, site: String, client: UnixStream) {
        self.log(&format!("forget-site {site} (from web --forget-site)"));
        if self.control.is_none() {
            reply_to(client, r#"{"event":"error","message":"the engine host is not connected"}"#);
            return;
        }
        self.forget_clients.push((site.clone(), client));
        self.send(Command::ForgetSite { site });
    }

    /// A second `web URL` asked (C-SINGLE): open the URL in a new buffer (C4) and show it, say so in
    /// the status bar, answer it.
    pub fn open_requested(&mut self, word: &str, client: UnixStream) {
        match crate::keys::go_url(word, &self.url) {
            Ok(url) if self.control.is_some() => {
                self.log(&format!("open {url} (from a second web)"));
                reply_to(client, &serde_json::json!({"event": "opened", "url": url}).to_string());
                if self.keys.vim_mode != crate::keys::VimMode::Normal {
                    self.keys.close_line();
                }
                let num = self.keys.open_buffer(&url);
                self.keys.message = format!("opened {url} (from another web) in buffer {num}");
                self.apply_key_effects(0);
            }
            Ok(_) => reply_to(client, r#"{"event":"error","message":"the engine host is not connected"}"#),
            Err(m) => reply_to(client, &serde_json::json!({"event": "error", "message": m}).to_string()),
        }
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

    /// C-CLOSE-POLITE: the window's X, `:q`, SIGINT. Asks the host to close every page politely
    /// (`close-all`: each page may confirm with `beforeunload`); the shell exits when the host has.
    /// Sent again on every call, so pages whose user chose to stay are asked again.
    pub fn close_politely(&mut self, why: &str) {
        self.log(&format!("closing politely: {why}"));
        self.closing = true;
        self.send(Command::CloseAll);
    }

    /// C-CLOSE-POLITE: SIGTERM. Forwards SIGTERM to the host (a forced quit: it closes every page
    /// without waiting for `beforeunload` answers); the shell exits when the host has. Never SIGKILL.
    pub fn terminate(&mut self, why: &str) {
        self.log(&format!("terminating: {why}"));
        self.closing = true;
        match &self.host {
            Some(child) => {
                // SAFETY: kill(2) on our own child's pid (not reaped yet: `host` is still Some).
                unsafe {
                    libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
                }
            }
            None => self.loop_signal.stop(),
        }
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
                if !self.closing && status.code() == Some(3) {
                    // C-SINGLE: the host's initialize failed (its message is on stderr above).
                    self.log("the engine host could not start: its profile is in use or unusable (message above)");
                }
                self.exit_code = exit_code_for(self.closing, status.code());
                self.host = None;
                self.loop_signal.stop();
            }
            Ok(None) => {}
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
        let paired = self.pairing.toplevel(surface.clone(), t.parent().is_some());
        let window = Window::new_wayland_window(t.clone());
        let app_id = smithay::wayland::compositor::with_states(t.wl_surface(), |s| {
            s.data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().unwrap().app_id.clone())
        });
        match paired {
            Some(tab) => {
                self.log(&format!(
                    "page window (app_id {:?}), paired with tab {tab}",
                    app_id.unwrap_or_default()
                ));
                self.pages.push((tab, window));
            }
            None => {
                let waiting = if self.pairing.is_candidate(&surface) {
                    " (no parent: a page if a tab-created comes for it)"
                } else {
                    ""
                };
                self.log(&format!("dialog window (app_id {:?}){waiting}", app_id.unwrap_or_default()));
                t.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
                self.dialogs.push(window);
            }
        }
        self.sync_page();
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

    fn new_surface(&mut self, surface: &WlSurface) {
        // C-FLICKER: a commit with a dmabuf waits until the buffer is drawn.
        crate::sync::guard_surface(surface);
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        // C6b: new subsurfaces (and the cursor surface) learn the scale too.
        self.send_scale(surface);
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

impl smithay::wayland::dmabuf::DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState {
        &mut self.dmabuf.state
    }

    fn dmabuf_imported(
        &mut self,
        global: &smithay::wayland::dmabuf::DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: smithay::wayland::dmabuf::ImportNotifier,
    ) {
        use smithay::backend::allocator::Buffer;
        use smithay::backend::renderer::ImportDma;
        match self.backend.renderer().import_dmabuf(&dmabuf, None) {
            Ok(_) => {
                if !self.dmabuf_seen {
                    self.dmabuf_seen = true;
                    let size = dmabuf.size();
                    self.log(&format!(
                        "dmabuf: the page's first dmabuf imported ({}x{}, {:?})",
                        size.w,
                        size.h,
                        dmabuf.format()
                    ));
                }
                let _ = notifier.successful::<State>();
            }
            Err(e) => {
                // The client gets `failed` (or, for create_immed, a protocol error). Take the
                // global away so engines started from now on stay on wl_shm.
                self.log(&format!(
                    "dmabuf: import failed ({:?}): {e:?}; no dmabuf for new clients",
                    dmabuf.format()
                ));
                self.dmabuf.state.disable_global::<State>(&self.display_handle, global);
                notifier.failed();
            }
        }
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // C6b: the scale before its first buffer, so the first frame is already sharp.
        self.send_scale(surface.wl_surface());
        self.pending_toplevels.push(surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        // T29: a popup right after a click is a <select>'s menu (or a context menu): its
        // keys must reach the page, so INPUT.
        let after_click = crate::keys::click_recent(self.last_click, Instant::now());
        let was = self.keys.key_mode;
        self.keys.popup_opened(after_click);
        if self.keys.key_mode != was {
            self.log("popup after a click: INPUT");
        }
        self.send_scale(surface.wl_surface());
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
        if self.is_shown_page(surface.wl_surface()) {
            self.set_fullscreen(true);
        } else if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if self.is_shown_page(surface.wl_surface()) {
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
        if let Some(tab) = self.pairing.toplevel_destroyed(s) {
            self.log(&format!("page window of tab {tab} closed"));
            self.pages.retain(|(t, _)| *t != tab);
            if self.is_shown_page(s) {
                self.page = None;
                if self.layout.fullscreen {
                    self.set_fullscreen(false);
                }
            }
        }
        self.dialogs.retain(|w| w.toplevel().is_some_and(|t| t.wl_surface() != s));
        self.refocus();
        self.redraw();
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        self.log("popup closed");
        let was = self.keys.key_mode;
        self.keys.popup_closed();
        if self.keys.key_mode != was {
            self.log("popup closed: back to the mode before the click");
        }
        self.redraw();
    }
}

impl State {
    /// Whether `s` is the shown page window (C4: the current buffer's).
    fn is_shown_page(&self, s: &WlSurface) -> bool {
        self.page
            .as_ref()
            .is_some_and(|w| w.toplevel().is_some_and(|t| t.wl_surface() == s))
    }

    /// Keeps a popup inside the shell window (xdg_positioner constraint adjustment).
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Some(window) = find_popup_root_surface(&kind).ok().and_then(|r| self.window_of(&r)) else {
            return;
        };
        let root_loc = self.space.element_location(&window).unwrap_or_default() - window.geometry().loc;
        let target = self.layout.popup_target(root_loc, get_popup_toplevel_coords(&kind));
        popup.with_pending_state(|s| s.geometry = self.layout.constrain_popup(s.positioner, target));
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

    fn focus_changed(&mut self, _seat: &Seat<Self>, focused: Option<&WlSurface>) {
        self.selection_focus(focused);
        self.ime_focus(focused);
    }
}

impl TabletSeatHandler for State {}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}

impl OutputHandler for State {}

impl smithay::wayland::fractional_scale::FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        self.send_scale(&surface);
    }
}

delegate_compositor!(State);
delegate_shm!(State);
smithay::delegate_dmabuf!(State);
delegate_xdg_shell!(State);
delegate_xdg_decoration!(State);
delegate_seat!(State);
delegate_output!(State);
delegate_viewporter!(State);
smithay::delegate_fractional_scale!(State);
delegate_cursor_shape!(State);
smithay::delegate_presentation!(State);

/// Writes one line to a `web --forget-site` client and closes the connection.
fn reply_to(mut client: UnixStream, line: &str) {
    use std::io::Write;
    let _ = client.write_all(format!("{}\n", line.trim_end()).as_bytes());
}

/// The shell's exit status once the host has exited with `code` (None: killed by a signal).
/// After a close the shell asked for (C-CLOSE-POLITE) it is the host's own status (0 stays 0); a
/// host that exits unasked is an error, so never 0 then.
pub fn exit_code_for(closing: bool, code: Option<i32>) -> i32 {
    if closing {
        code.unwrap_or(1)
    } else {
        code.unwrap_or(1).max(1)
    }
}
