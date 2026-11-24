//! C6e: input methods (IME) through text-input-v3, in INPUT mode only.
//!
//! The shell sits between two text-input-v3 connections:
//!
//! - **Downward**, it is the text-input-v3 *server* for the page: it offers
//!   `zwp_text_input_manager_v3` on its own display, sends `enter`/`leave` with the keyboard focus,
//!   and keeps each text input's double-buffered state (`PageInput`). Chromium uses it with
//!   `--enable-wayland-ime --wayland-text-input-version=3` (host.rs).
//! - **Upward**, it is a text-input-v3 *client* of the desktop: a `zwp_text_input_v3` of its own on
//!   the shell window's Wayland connection (as the keymap watcher and smithay-clipboard reach the
//!   desktop; winit's own text input stays disabled: the shell never calls `set_ime_allowed`).
//!   The desktop's input method (fcitx5, IBus, ...) sees the shell window as the text field.
//!
//! `Relay` is the pure part between them: it forwards enable/disable, surrounding text, change cause,
//! content type and the cursor rectangle up (the rectangle moved from page-surface to shell-window
//! coordinates; both are logical, so it is right at any scale), and preedit, commit and
//! delete-surrounding down. It is enabled upward only while the page has an enabled text input, the
//! desktop has entered the shell window, and the key mode is INPUT: in NORMAL (and its `:`/`/` lines
//! and the picker) the shell's own key handling owns the keyboard, so the input method is disabled
//! and a preedit shown in the page is cleared.
//!
//! X11: no input method yet (winit's XIM is not wired); the page then gets plain keys as before.

use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::{
    zwp_text_input_manager_v3::{self, ZwpTextInputManagerV3},
    zwp_text_input_v3::{self, ZwpTextInputV3},
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};

/// A text input's state, as the protocol double-buffers it (byte offsets, as the protocol has them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TiState {
    /// Text, cursor, anchor.
    pub surrounding: Option<(String, i32, i32)>,
    pub cause: u32,
    /// Hint, purpose.
    pub content: Option<(u32, u32)>,
    /// x, y, width, height (surface-local logical px).
    pub cursor: Option<(i32, i32, i32, i32)>,
}

/// What a commit of a page text input did.
#[derive(Debug, PartialEq, Eq)]
pub enum Commit {
    Enabled,
    Disabled,
    /// An enabled text input sent new state.
    Changed,
    /// A commit while disabled.
    Nothing,
}

/// One page text input's protocol state: requests go to `pending`; `commit` applies them.
#[derive(Debug, Default)]
pub struct PageState {
    pending: TiState,
    pending_enable: Option<bool>,
    pub current: TiState,
    pub enabled: bool,
    /// Commits so far: the serial `done` carries back.
    pub serial: u32,
}

impl PageState {
    pub fn enable(&mut self) {
        // `enable` resets the state to the initial one; what follows it in the same commit counts.
        self.pending = TiState::default();
        self.pending_enable = Some(true);
    }
    pub fn disable(&mut self) {
        self.pending_enable = Some(false);
    }
    pub fn surrounding(&mut self, text: String, cursor: i32, anchor: i32) {
        self.pending.surrounding = Some((text, cursor, anchor));
    }
    pub fn cause(&mut self, cause: u32) {
        self.pending.cause = cause;
    }
    pub fn content(&mut self, hint: u32, purpose: u32) {
        self.pending.content = Some((hint, purpose));
    }
    pub fn cursor(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.pending.cursor = Some((x, y, w, h));
    }

    pub fn commit(&mut self) -> Commit {
        self.serial = self.serial.wrapping_add(1);
        let p = std::mem::take(&mut self.pending);
        let was = self.enabled;
        match self.pending_enable.take() {
            Some(true) => {
                self.enabled = true;
                self.current = p;
                return Commit::Enabled;
            }
            Some(false) => {
                self.enabled = false;
                self.current = TiState::default();
                return if was { Commit::Disabled } else { Commit::Nothing };
            }
            None => {}
        }
        if !self.enabled {
            return Commit::Nothing;
        }
        // Fields not sent in this commit keep their value (the cause goes back to "other").
        let cur = &mut self.current;
        if p.surrounding.is_some() {
            cur.surrounding = p.surrounding;
        }
        if p.content.is_some() {
            cur.content = p.content;
        }
        if p.cursor.is_some() {
            cur.cursor = p.cursor;
        }
        cur.cause = p.cause;
        Commit::Changed
    }
}

/// A request to the desktop's text input.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Up {
    Enable,
    Disable,
    Surrounding(String, i32, i32),
    Cause(u32),
    ContentType(u32, u32),
    Cursor(i32, i32, i32, i32),
    Commit,
}

/// An event to the page's text input (`Done` takes the page's serial in the glue).
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Down {
    Delete(u32, u32),
    Preedit(Option<String>, i32, i32),
    Commit(String),
    Done,
}

/// What the desktop's input method sent between two `done`s.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Composed {
    pub preedit: Option<(String, i32, i32)>,
    pub commit: Option<String>,
    pub delete: Option<(u32, u32)>,
}

/// The pure relay between the page's active text input and the desktop's.
#[derive(Debug, Default)]
pub struct Relay {
    /// The page's active text input's state, while it is enabled.
    page: Option<TiState>,
    page_changed: bool,
    /// The desktop sent `enter` (the shell window has its keyboard focus) and no `leave` since.
    desktop_entered: bool,
    /// Enabled upward.
    up: bool,
    /// The page surface's origin in the shell window when the cursor rectangle was last sent.
    sent_offset: (i32, i32),
    /// The page shows a preedit the input method gave it.
    preedit_shown: bool,
}

impl Relay {
    /// The page's active text input was enabled (or another one became the active one).
    pub fn page_enabled(&mut self, st: TiState) {
        self.page = Some(st);
        self.page_changed = true;
        self.preedit_shown = false;
    }
    /// The page's active text input committed new state.
    pub fn page_changed(&mut self, st: TiState) {
        if self.page.is_some() {
            self.page = Some(st);
            self.page_changed = true;
        }
    }
    /// The page disabled its text input, left it (focus), or destroyed it.
    pub fn page_disabled(&mut self) {
        self.page = None;
        self.preedit_shown = false;
    }
    pub fn desktop_enter(&mut self) {
        self.desktop_entered = true;
    }
    pub fn desktop_leave(&mut self) {
        self.desktop_entered = false;
    }
    pub fn up_enabled(&self) -> bool {
        self.up
    }

    fn state_up(st: &TiState, offset: (i32, i32), out: &mut Vec<Up>) {
        if let Some((t, c, a)) = &st.surrounding {
            out.push(Up::Surrounding(t.clone(), *c, *a));
        }
        out.push(Up::Cause(st.cause));
        if let Some((h, p)) = st.content {
            out.push(Up::ContentType(h, p));
        }
        if let Some(r) = map_cursor(st.cursor, offset) {
            out.push(Up::Cursor(r.0, r.1, r.2, r.3));
        }
    }

    /// Brings the desktop's text input in line: `input` is the key mode, `offset` the page
    /// surface's origin in the shell window (logical). Returns the requests for the desktop and
    /// what the page must be told (a preedit to clear when the input method goes away).
    pub fn sync(&mut self, input: bool, offset: (i32, i32)) -> (Vec<Up>, Vec<Down>) {
        let want = input && self.desktop_entered && self.page.is_some();
        let (mut up, mut down) = (Vec::new(), Vec::new());
        match (self.up, want) {
            (false, true) => {
                up.push(Up::Enable);
                Self::state_up(self.page.as_ref().unwrap(), offset, &mut up);
                up.push(Up::Commit);
                self.up = true;
            }
            (true, true) if self.page_changed || offset != self.sent_offset => {
                Self::state_up(self.page.as_ref().unwrap(), offset, &mut up);
                up.push(Up::Commit);
            }
            (true, false) => {
                up.push(Up::Disable);
                up.push(Up::Commit);
                self.up = false;
                if self.preedit_shown && self.page.is_some() {
                    down.push(Down::Preedit(None, 0, 0));
                    down.push(Down::Done);
                }
                self.preedit_shown = false;
            }
            _ => {}
        }
        if self.up {
            self.page_changed = false;
            self.sent_offset = offset;
        }
        (up, down)
    }

    /// The desktop's `done`: what goes to the page (nothing unless enabled upward).
    pub fn desktop_done(&mut self, c: Composed) -> Vec<Down> {
        if !self.up || self.page.is_none() {
            return Vec::new();
        }
        let mut down = Vec::new();
        if let Some((b, a)) = c.delete.filter(|d| *d != (0, 0)) {
            down.push(Down::Delete(b, a));
        }
        // text-input-v3: a `done` without a preedit_string clears the preedit; say so every time.
        match c.preedit {
            Some((t, b, e)) if !t.is_empty() => {
                down.push(Down::Preedit(Some(t), b, e));
                self.preedit_shown = true;
            }
            _ => {
                down.push(Down::Preedit(None, 0, 0));
                self.preedit_shown = false;
            }
        }
        if let Some(t) = c.commit {
            down.push(Down::Commit(t));
        }
        down.push(Down::Done);
        down
    }
}

/// The page's cursor rectangle (page-surface logical px) in the shell window's surface (logical px).
/// text-input-v3 rectangles are surface-local and logical on both sides, so no scale enters.
pub fn map_cursor(cursor: Option<(i32, i32, i32, i32)>, offset: (i32, i32)) -> Option<(i32, i32, i32, i32)> {
    cursor.map(|(x, y, w, h)| (x + offset.0, y + offset.1, w, h))
}

/// The page's text inputs, and the bridge to the desktop.
#[derive(Default)]
pub struct Ime {
    pub relay: Relay,
    pub inputs: Vec<(ZwpTextInputV3, PageState)>,
    /// The text input the relay follows (enabled last, on the focused surface).
    pub active: Option<ZwpTextInputV3>,
    /// The surface the page's text inputs were last sent `enter` for.
    pub entered: Option<WlSurface>,
    pub desktop: Option<desktop::Outer>,
    /// The cursor rectangle last sent up (logged when it changes).
    pub logged_cursor: Option<(i32, i32, i32, i32)>,
}

/// From the desktop's text input (the watcher thread) to the main loop.
pub enum FromDesktop {
    Ready(desktop::Outer),
    Enter,
    Leave,
    Done(Composed),
    /// The desktop has no text-input-v3.
    Unavailable(String),
}

/// Starts the desktop side (Wayland only; `display` is winit's wl_display).
pub fn start(display: usize, tx: calloop::channel::Sender<FromDesktop>) {
    let spawned = std::thread::Builder::new().name("web-ime".into()).spawn(move || {
        crate::clipboard::block_quit_signals();
        if let Err(e) = desktop::watch(display, &tx) {
            let _ = tx.send(FromDesktop::Unavailable(e));
        }
    });
    if let Err(e) = spawned {
        eprintln!("web: ime thread: {e}");
    }
}

impl crate::state::State {
    /// Offers zwp_text_input_manager_v3 to the page.
    pub fn ime_global(dh: &DisplayHandle) -> smithay::reexports::wayland_server::backend::GlobalId {
        dh.create_global::<crate::state::State, ZwpTextInputManagerV3, _>(1, ())
    }

    /// The page surface's origin in the shell window: where the window holding `surface` is drawn.
    fn ime_offset(&self, surface: Option<&WlSurface>) -> (i32, i32) {
        let Some(surface) = surface else { return (0, 0) };
        for w in self.space.elements() {
            if w.toplevel().is_some_and(|t| t.wl_surface() == surface) {
                let loc = self.space.element_location(w).unwrap_or_default() - w.geometry().loc;
                return (loc.x, loc.y);
            }
        }
        (0, 0)
    }

    /// Brings the desktop's text input in line with the page and the key mode (called on every
    /// redraw: cheap when nothing changed).
    pub fn ime_sync(&mut self) {
        let input = self.keys.key_mode == crate::keys::KeyMode::Input;
        let offset = self.ime_offset(self.ime.entered.as_ref());
        let was = self.ime.relay.up_enabled();
        let (up, down) = self.ime.relay.sync(input, offset);
        if up.is_empty() && down.is_empty() {
            return;
        }
        if was != self.ime.relay.up_enabled() {
            let cur = self
                .ime
                .active
                .as_ref()
                .and_then(|a| self.ime.inputs.iter().find(|(t, _)| t == a))
                .and_then(|(_, s)| map_cursor(s.current.cursor, offset));
            self.log(&format!(
                "ime: desktop input method {} (cursor {cur:?})",
                if self.ime.relay.up_enabled() { "enabled" } else { "disabled" }
            ));
            self.ime.logged_cursor = None;
        }
        for u in &up {
            if let Up::Cursor(x, y, w, h) = u {
                let r = (*x, *y, *w, *h);
                if self.ime.logged_cursor != Some(r) {
                    self.ime.logged_cursor = Some(r);
                    self.log(&format!("ime: cursor rectangle {r:?} to the desktop"));
                }
            }
        }
        if let Some(d) = &self.ime.desktop {
            d.send(&up);
        }
        self.ime_down(down);
    }

    /// Sends events to the page's active text input.
    fn ime_down(&mut self, down: Vec<Down>) {
        let Some(ti) = self.ime.active.clone() else { return };
        let serial = self.ime.inputs.iter().find(|(t, _)| *t == ti).map(|(_, s)| s.serial).unwrap_or(0);
        for d in down {
            match d {
                Down::Delete(b, a) => {
                    self.log(&format!("ime: delete {b},{a} to the page"));
                    ti.delete_surrounding_text(b, a);
                }
                Down::Preedit(t, b, e) => {
                    if let Some(t) = &t {
                        self.log(&format!("ime: preedit {t:?} to the page"));
                    }
                    ti.preedit_string(t, b, e);
                }
                Down::Commit(t) => {
                    self.log(&format!("ime: commit {t:?} to the page"));
                    ti.commit_string(Some(t));
                }
                Down::Done => ti.done(serial),
            }
        }
    }

    /// A message from the desktop's text input.
    pub fn ime_from_desktop(&mut self, m: FromDesktop) {
        match m {
            FromDesktop::Ready(o) => {
                self.log("ime: the desktop has text-input-v3");
                self.ime.desktop = Some(o);
            }
            FromDesktop::Unavailable(e) => self.log(&format!("ime: no desktop input method: {e}")),
            FromDesktop::Enter => {
                self.ime.relay.desktop_enter();
                self.ime_sync();
            }
            FromDesktop::Leave => {
                self.ime.relay.desktop_leave();
                self.ime_sync();
            }
            FromDesktop::Done(c) => {
                if !self.ime.relay.up_enabled() {
                    self.log("ime: input method text dropped (not enabled)");
                }
                let down = self.ime.relay.desktop_done(c);
                self.ime_down(down);
            }
        }
    }

    /// The keyboard focus moved: `leave` and `enter` for the page's text inputs.
    pub fn ime_focus(&mut self, focused: Option<&WlSurface>) {
        if self.ime.entered.as_ref() == focused {
            return;
        }
        if let Some(old) = self.ime.entered.take() {
            for (t, _) in &self.ime.inputs {
                if t.id().same_client_as(&old.id()) {
                    t.leave(&old);
                }
            }
            if self.ime.active.take().is_some() {
                self.ime.relay.page_disabled();
            }
        }
        if let Some(s) = focused {
            for (t, _) in &self.ime.inputs {
                if t.id().same_client_as(&s.id()) {
                    t.enter(s);
                }
            }
            self.ime.entered = Some(s.clone());
        }
        self.ime_sync();
    }

    fn ime_commit(&mut self, ti: &ZwpTextInputV3) {
        let Some((_, st)) = self.ime.inputs.iter_mut().find(|(t, _)| t == ti) else {
            return;
        };
        let what = st.commit();
        let cur = st.current.clone();
        let focused = self.ime.entered.as_ref().is_some_and(|s| s.id().same_client_as(&ti.id()));
        match what {
            Commit::Enabled if focused => {
                self.log(&format!("ime: page text input enabled (content {:?})", cur.content));
                self.ime.active = Some(ti.clone());
                self.ime.relay.page_enabled(cur);
            }
            Commit::Changed if self.ime.active.as_ref() == Some(ti) => self.ime.relay.page_changed(cur),
            Commit::Disabled if self.ime.active.as_ref() == Some(ti) => {
                self.log("ime: page text input disabled");
                self.ime.active = None;
                self.ime.relay.page_disabled();
            }
            _ => {}
        }
        self.ime_sync();
    }
}

impl GlobalDispatch<ZwpTextInputManagerV3, ()> for crate::state::State {
    fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<ZwpTextInputManagerV3>, _: &(), data_init: &mut DataInit<'_, Self>) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwpTextInputManagerV3, ()> for crate::state::State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwpTextInputManagerV3,
        request: zwp_text_input_manager_v3::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let zwp_text_input_manager_v3::Request::GetTextInput { id, .. } = request {
            let ti = data_init.init(id, ());
            if state.ime.inputs.is_empty() {
                state.log("ime: the page has a text input (text-input-v3)");
            }
            if let Some(s) = state.ime.entered.as_ref().filter(|s| s.id().same_client_as(&ti.id())) {
                ti.enter(s);
            }
            state.ime.inputs.push((ti, PageState::default()));
        }
    }
}

impl Dispatch<ZwpTextInputV3, ()> for crate::state::State {
    fn request(
        state: &mut Self,
        _: &Client,
        ti: &ZwpTextInputV3,
        request: zwp_text_input_v3::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use zwp_text_input_v3::Request as R;
        if let R::Commit = request {
            state.ime_commit(ti);
            return;
        }
        let Some((_, st)) = state.ime.inputs.iter_mut().find(|(t, _)| t == ti) else {
            return;
        };
        match request {
            R::Enable => st.enable(),
            R::Disable => st.disable(),
            R::SetSurroundingText { text, cursor, anchor } => st.surrounding(text, cursor, anchor),
            R::SetTextChangeCause { cause } => st.cause(cause.into()),
            R::SetContentType { hint, purpose } => st.content(hint.into(), purpose.into()),
            R::SetCursorRectangle { x, y, width, height } => st.cursor(x, y, width, height),
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _: smithay::reexports::wayland_server::backend::ClientId, ti: &ZwpTextInputV3, _: &()) {
        state.ime.inputs.retain(|(t, _)| t != ti);
        if state.ime.active.as_ref() == Some(ti) {
            state.ime.active = None;
            state.ime.relay.page_disabled();
            state.ime_sync();
        }
    }
}

pub mod desktop {
    //! The shell window's own text input on the desktop (a worker thread dispatches its events).
    use super::{Composed, FromDesktop, Up};
    use wayland_backend::client::Backend;
    use wayland_client::globals::{registry_queue_init, GlobalListContents};
    use wayland_client::protocol::wl_registry::WlRegistry;
    use wayland_client::protocol::wl_seat::WlSeat;
    use wayland_client::{Connection, Dispatch, QueueHandle};
    use wayland_protocols::wp::text_input::zv3::client::{
        zwp_text_input_manager_v3::ZwpTextInputManagerV3,
        zwp_text_input_v3::{self, ContentHint, ContentPurpose, ZwpTextInputV3},
    };

    /// The main loop's handle on the desktop's text input: requests are sent from there.
    pub struct Outer {
        ti: ZwpTextInputV3,
        conn: Connection,
    }

    impl Outer {
        pub fn send(&self, ups: &[Up]) {
            for u in ups {
                match u {
                    Up::Enable => self.ti.enable(),
                    Up::Disable => self.ti.disable(),
                    Up::Surrounding(t, c, a) => self.ti.set_surrounding_text(t.clone(), *c, *a),
                    Up::Cause(c) => {
                        let cause = zwp_text_input_v3::ChangeCause::try_from(*c).unwrap_or(zwp_text_input_v3::ChangeCause::Other);
                        self.ti.set_text_change_cause(cause);
                    }
                    Up::ContentType(h, p) => self.ti.set_content_type(
                        ContentHint::from_bits_truncate(*h),
                        ContentPurpose::try_from(*p).unwrap_or(ContentPurpose::Normal),
                    ),
                    Up::Cursor(x, y, w, h) => self.ti.set_cursor_rectangle(*x, *y, *w, *h),
                    Up::Commit => self.ti.commit(),
                }
            }
            let _ = self.conn.flush();
        }
    }

    struct Watch {
        tx: calloop::channel::Sender<FromDesktop>,
        pending: Composed,
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
        fn event(_: &mut Self, _: &WlSeat, _: <WlSeat as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }
    impl Dispatch<ZwpTextInputManagerV3, ()> for Watch {
        fn event(
            _: &mut Self,
            _: &ZwpTextInputManagerV3,
            _: <ZwpTextInputManagerV3 as wayland_client::Proxy>::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    impl Dispatch<ZwpTextInputV3, ()> for Watch {
        fn event(w: &mut Self, _: &ZwpTextInputV3, e: zwp_text_input_v3::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            use zwp_text_input_v3::Event as E;
            match e {
                // The shell has one window: whatever surface the desktop names is it.
                E::Enter { .. } => {
                    let _ = w.tx.send(FromDesktop::Enter);
                }
                E::Leave { .. } => {
                    let _ = w.tx.send(FromDesktop::Leave);
                }
                E::PreeditString {
                    text,
                    cursor_begin,
                    cursor_end,
                } => {
                    w.pending.preedit = text.map(|t| (t, cursor_begin, cursor_end));
                }
                E::CommitString { text } => w.pending.commit = text,
                E::DeleteSurroundingText {
                    before_length,
                    after_length,
                } => w.pending.delete = Some((before_length, after_length)),
                E::Done { .. } => {
                    let _ = w.tx.send(FromDesktop::Done(std::mem::take(&mut w.pending)));
                }
                _ => {}
            }
        }
    }

    pub fn watch(display: usize, tx: &calloop::channel::Sender<FromDesktop>) -> Result<(), String> {
        // Safety: the pointer is winit's live wl_display, which outlives the shell.
        let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
        let conn = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<Watch>(&conn).map_err(|e| format!("wayland registry: {e}"))?;
        let qh = queue.handle();
        let seat: WlSeat = globals
            .bind(&qh, 1..=7, ())
            .map_err(|e| format!("the desktop has no wl_seat: {e}"))?;
        let mgr: ZwpTextInputManagerV3 = globals
            .bind(&qh, 1..=1, ())
            .map_err(|e| format!("the desktop has no zwp_text_input_manager_v3: {e}"))?;
        let ti = mgr.get_text_input(&seat, &qh, ());
        let _ = conn.flush();
        let _ = tx.send(FromDesktop::Ready(Outer { ti, conn: conn.clone() }));
        let mut w = Watch {
            tx: tx.clone(),
            pending: Composed::default(),
        };
        loop {
            queue.blocking_dispatch(&mut w).map_err(|e| format!("wayland dispatch: {e}"))?;
        }
    }
}
