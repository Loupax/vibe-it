//! C6: the engine host's side of the clipboard bridge (the desktop's side is `clipboard.rs`, and
//! on Wayland `desktop_selection.rs`).
//!
//! Wayland (C-CLIPTYPES): every type, both ways, without the shell reading the data. The page's
//! selections follow the desktop's: each `selection` event of the shell window's own data device
//! becomes a compositor-owned selection on the shell's seat with the desktop's types, and a page
//! that reads it gets the desktop's offer with its fd. A copy in the page (`new_selection`) is set
//! on the desktop as the shell's own source with the page's types (plus a marker type, so it is
//! not carried back), and a desktop client that reads it gets the page's source with its fd. Its
//! text is still read, for the shell's own reads of `+` (`Desktop::own`).
//!
//! X11 (and Wayland without `wl_data_device_manager`): text only. The page copies by setting the
//! selection on the shell's seat; the shell reads the text (`new_selection`) and puts it on the
//! desktop's clipboard. The other way is lazy: the desktop's text is offered to the page (a
//! compositor-owned selection served from memory) when the shell window gains the desktop's
//! focus, before a paste key reaches the page (Ctrl-V, Ctrl-Shift-V, Shift-Insert) and before a
//! middle click on the page (primary). Each time, only when the desktop holds something else
//! than what was last carried across, so a copy made in the page keeps its own selection (and
//! its rich types, text/html) for a paste in the page.

use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};

use smithay::input::Seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::selection::data_device::{
    clear_data_device_selection, request_data_device_client_selection, set_data_device_focus, set_data_device_selection, DataDeviceHandler,
    DataDeviceState,
};
use smithay::wayland::selection::primary_selection::{
    clear_primary_selection, request_primary_client_selection, set_primary_focus, set_primary_selection, PrimarySelectionHandler,
    PrimarySelectionState,
};
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
use smithay::{delegate_data_device, delegate_primary_selection};

use crate::clipboard::{pick_text_mime, Desktop, SystemClipboard, Which, TEXT_MIMES};
use crate::desktop_selection::{self as ds, DesktopOffer, FromDesktop, SourceProxy};
use crate::state::State;

/// The most of a page's copy carried to the desktop (the text bridge).
const MAX_COPY: u64 = 16 << 20;

/// A page's copy, read on a thread: which selection and its text.
pub type Copied = (Which, Result<String, String>);

/// A compositor-owned selection on the shell's seat: what a page that reads it gets.
#[derive(Clone, Debug)]
pub enum Offered {
    /// The desktop's selection (Wayland): the read goes to the desktop's offer.
    Desktop(DesktopOffer),
    /// Text served from memory (X11, and Wayland without `wl_data_device_manager`).
    Text(String),
}

pub struct Bridge {
    pub desktop: Desktop,
    /// Where the reader threads send a page's copy (the compositor's loop receives it).
    pub copies: calloop::channel::Sender<Copied>,
    /// The text last carried across, per selection: both sides hold it (the text bridge).
    synced_clipboard: Option<String>,
    synced_primary: Option<String>,
    /// Copies the page made in this dispatch, with their types: read after it, once smithay has
    /// stored the selection (`new_selection` is called before it does).
    pending: Vec<(Which, Vec<String>)>,
    /// C-CLIPTYPES: the desktop's selections with every type (None: X11, not ready yet, or the
    /// desktop lacks wl_data_device_manager: the text bridge).
    outer: Option<ds::Outer>,
    /// The desktop's current offer per selection (destroyed when the next one replaces it).
    offers: [Option<DesktopOffer>; 2],
    /// The shell's current source on the desktop per selection (it serves the page's selection).
    ours: [Option<SourceProxy>; 2],
}

impl Bridge {
    pub fn new(desktop: Desktop, copies: calloop::channel::Sender<Copied>) -> Bridge {
        Bridge {
            desktop,
            copies,
            synced_clipboard: None,
            synced_primary: None,
            pending: Vec::new(),
            outer: None,
            offers: [None, None],
            ours: [None, None],
        }
    }

    /// Whether every type is carried (Wayland); else the text bridge.
    pub fn carries_types(&self) -> bool {
        self.outer.is_some()
    }

    fn synced(&mut self, which: Which) -> &mut Option<String> {
        match which {
            Which::Clipboard => &mut self.synced_clipboard,
            Which::Primary => &mut self.synced_primary,
        }
    }
}

fn which_of(ty: SelectionTarget) -> Which {
    match ty {
        SelectionTarget::Clipboard => Which::Clipboard,
        SelectionTarget::Primary => Which::Primary,
    }
}

fn index(which: Which) -> usize {
    match which {
        Which::Clipboard => 0,
        Which::Primary => 1,
    }
}

fn name(which: Which) -> &'static str {
    match which {
        Which::Clipboard => "clipboard",
        Which::Primary => "primary selection",
    }
}

impl State {
    /// The keyboard focus moved: the selections are offered to the focused client.
    pub fn selection_focus(&mut self, focused: Option<&WlSurface>) {
        let client = focused.and_then(|s| self.display_handle.get_client(s.id()).ok());
        set_data_device_focus(&self.display_handle, &self.seat, client.clone());
        set_primary_focus(&self.display_handle, &self.seat, client);
    }

    /// Offers the desktop's text to the page, if it changed since it was last carried across.
    /// The text bridge only: with every type carried (Wayland) the page's selections follow the
    /// desktop's `selection` events, which the desktop sends again on every focus gain.
    pub fn sync_from_desktop(&mut self, which: Which) {
        if self.bridge.carries_types() {
            return;
        }
        let text = match self.bridge.desktop.read(which) {
            Ok(t) => t,
            Err(e) => {
                self.log(&format!("clipboard: cannot read the desktop's {}: {e}", name(which)));
                return;
            }
        };
        if text.is_empty() || self.bridge.synced(which).as_deref() == Some(text.as_str()) {
            return;
        }
        self.log(&format!(
            "clipboard: desktop {} ({} bytes) offered to the page",
            name(which),
            text.len()
        ));
        let mimes: Vec<String> = TEXT_MIMES.iter().map(|m| m.to_string()).collect();
        let offered = Offered::Text(text.clone());
        match which {
            Which::Clipboard => set_data_device_selection(&self.display_handle, &self.seat, mimes, offered),
            Which::Primary => set_primary_selection(&self.display_handle, &self.seat, mimes, offered),
        }
        *self.bridge.synced(which) = Some(text);
    }

    /// C-CLIPTYPES: an event of the desktop's selections (Wayland).
    pub fn desktop_selection(&mut self, msg: FromDesktop) {
        match msg {
            FromDesktop::Ready(outer) => {
                self.log(&format!(
                    "clipboard: every type is carried between the desktop and the page (primary selection: {})",
                    if outer.has_primary() { "yes" } else { "no, the desktop lacks it" }
                ));
                self.bridge.outer = Some(outer);
            }
            FromDesktop::Unavailable(e) => {
                self.log(&format!("clipboard: text only: {e}"));
                self.bridge.outer = None;
            }
            FromDesktop::Selection { which, offer } => self.desktop_offered(which, offer),
            FromDesktop::Send { which, mime, fd } => {
                if mime == ds::MARKER {
                    return; // the fd closes
                }
                self.log(&format!(
                    "clipboard: the desktop reads the {} as {mime} (from the page)",
                    name(which)
                ));
                let asked = match which {
                    Which::Clipboard => request_data_device_client_selection(&self.seat, mime, fd).map_err(|e| e.to_string()),
                    Which::Primary => request_primary_client_selection(&self.seat, mime, fd).map_err(|e| e.to_string()),
                };
                match asked {
                    Ok(()) => {
                        let _ = self.display_handle.flush_clients();
                    }
                    Err(e) => self.log(&format!("clipboard: cannot ask the page for its {}: {e}", name(which))),
                }
            }
            FromDesktop::Cancelled { which, source } => {
                let i = index(which);
                if self.bridge.ours[i].as_ref() == Some(&source) {
                    self.bridge.ours[i] = None;
                    self.bridge.desktop.set_own(which, None);
                }
            }
        }
    }

    /// The desktop's `which` changed: the page's follows it (unless it is the page's own).
    fn desktop_offered(&mut self, which: Which, offer: Option<DesktopOffer>) {
        let i = index(which);
        let old = std::mem::replace(&mut self.bridge.offers[i], offer.clone());
        match offer {
            None => {
                match which {
                    Which::Clipboard => clear_data_device_selection(&self.display_handle, &self.seat),
                    Which::Primary => clear_primary_selection(&self.display_handle, &self.seat),
                }
                self.log(&format!("clipboard: the desktop's {} is empty, and so is the page's", name(which)));
            }
            Some(o) if ds::is_own(&o.mimes) => {}
            Some(o) => {
                let mimes = ds::mimes_for_page(&o.mimes);
                self.log(&format!(
                    "clipboard: desktop {} offered to the page ({})",
                    name(which),
                    ds::summary(&mimes)
                ));
                match which {
                    Which::Clipboard => set_data_device_selection(&self.display_handle, &self.seat, mimes, Offered::Desktop(o)),
                    Which::Primary => set_primary_selection(&self.display_handle, &self.seat, mimes, Offered::Desktop(o)),
                }
            }
        }
        if let Some(old) = old {
            if self.bridge.offers[i].as_ref().map(|o| &o.proxy) != Some(&old.proxy) {
                old.destroy();
            }
        }
        if let Some(outer) = &self.bridge.outer {
            outer.flush();
        }
    }

    /// C-CLIPTYPES: a copy in the page is set on the desktop with all its types (Wayland).
    fn carry_page_selection(&mut self, which: Which, offered: &[String]) {
        let Some(outer) = &self.bridge.outer else { return };
        if !outer.focused() {
            self.log(&format!(
                "clipboard: the shell window has no focus; the page's {} is not carried",
                name(which)
            ));
            return;
        }
        // None: the desktop has no primary selection.
        let mimes = ds::mimes_for_desktop(offered);
        let Some(source) = outer.set(which, &mimes) else {
            return;
        };
        // The source it replaces gets `cancelled`, ignored then: it is no longer `ours`.
        self.bridge.ours[index(which)] = Some(source);
        self.bridge.desktop.set_own(which, Some(String::new()));
        self.log(&format!(
            "clipboard: page {} offered to the desktop ({})",
            name(which),
            ds::summary(&ds::mimes_for_page(&mimes))
        ));
    }

    /// A page's copy has been read as text: it goes to the desktop (the text bridge), or is kept
    /// for the shell's own reads of `+` while the desktop holds the page's selection (Wayland).
    pub fn page_copied(&mut self, (which, text): Copied) {
        let text = match text {
            Ok(t) => t,
            Err(e) => {
                self.log(&format!("clipboard: cannot read the page's {}: {e}", name(which)));
                return;
            }
        };
        if self.bridge.carries_types() {
            if self.bridge.desktop.own(which).is_some() {
                self.bridge.desktop.set_own(which, Some(text));
            }
            return;
        }
        if text.is_empty() {
            // Chromium sets an empty primary selection when a selection goes away.
            return;
        }
        // Written even when it is the text last carried: the desktop may hold another since.
        match self.bridge.desktop.write(which, &text) {
            Ok(()) => self.log(&format!("clipboard: page {} ({} bytes) to the desktop", name(which), text.len())),
            Err(e) => self.log(&format!("clipboard: cannot write the desktop's {}: {e}", name(which))),
        }
        *self.bridge.synced(which) = Some(text);
    }

    /// Carries and reads the copies the page made since the last call (from the event loop,
    /// after dispatch).
    pub fn read_page_copies(&mut self) {
        for (which, offered) in std::mem::take(&mut self.bridge.pending) {
            if self.bridge.carries_types() {
                self.carry_page_selection(which, &offered);
            }
            self.read_page_selection(which, offered);
        }
    }

    /// Asks the client that owns the selection for its text; a thread reads it.
    fn read_page_selection(&mut self, which: Which, offered: Vec<String>) {
        let Some(mime) = pick_text_mime(&offered) else {
            if !self.bridge.carries_types() {
                self.log(&format!(
                    "clipboard: the page's {} holds no text ({offered:?}), not carried",
                    name(which)
                ));
            }
            return;
        };
        let (mut reader, writer) = match std::io::pipe() {
            Ok(p) => p,
            Err(e) => {
                self.log(&format!("clipboard: pipe: {e}"));
                return;
            }
        };
        let fd = OwnedFd::from(writer);
        let asked = match which {
            Which::Clipboard => request_data_device_client_selection(&self.seat, mime, fd).map_err(|e| e.to_string()),
            Which::Primary => request_primary_client_selection(&self.seat, mime, fd).map_err(|e| e.to_string()),
        };
        if let Err(e) = asked {
            self.log(&format!("clipboard: cannot ask the page for its {}: {e}", name(which)));
            return;
        }
        // The client writes once it has the request: flush it now, read off the loop.
        let _ = self.display_handle.flush_clients();
        let tx = self.bridge.copies.clone();
        let _ = std::thread::Builder::new().name("web-copy".into()).spawn(move || {
            let mut buf = Vec::new();
            let r = (&mut reader)
                .take(MAX_COPY)
                .read_to_end(&mut buf)
                .map_err(|e| e.to_string())
                .map(|_| String::from_utf8_lossy(&buf).into_owned());
            let _ = tx.send((which, r));
        });
    }
}

impl SelectionHandler for State {
    /// What a compositor-owned selection serves.
    type SelectionUserData = Offered;

    /// C-CLIPTYPES step 7: smithay 0.7 calls this only for a client's `set_selection`
    /// (`data_device/device.rs`, `primary_selection/device.rs`), never for the shell's own
    /// `set_data_device_selection` / `set_primary_selection` / `clear_*` (they store the
    /// selection directly, `seat_data.rs`): a desktop selection carried to the page does not
    /// bounce back to the desktop.
    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, _seat: Seat<Self>) {
        if let Some(source) = source {
            let which = which_of(ty);
            self.bridge.pending.retain(|(w, _)| *w != which);
            self.bridge.pending.push((which, source.mime_types()));
        }
    }

    fn send_selection(&mut self, ty: SelectionTarget, mime: String, fd: OwnedFd, _seat: Seat<Self>, offered: &Offered) {
        let text = match offered {
            Offered::Desktop(offer) => {
                // The desktop's owner writes into the page's fd; ours closes once it is sent.
                self.log(&format!(
                    "clipboard: the page reads the {} as {mime} (from the desktop)",
                    name(which_of(ty))
                ));
                offer.receive(mime, fd.as_fd());
                if let Some(outer) = &self.bridge.outer {
                    outer.flush();
                }
                return;
            }
            Offered::Text(t) => t.clone(),
        };
        self.log(&format!("clipboard: the page reads the {} as {mime}", name(which_of(ty))));
        // A thread: the page may read slowly, and a full pipe must not block the compositor.
        let _ = std::thread::Builder::new().name("web-paste".into()).spawn(move || {
            let _ = std::fs::File::from(fd).write_all(text.as_bytes());
        });
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

delegate_data_device!(State);
delegate_primary_selection!(State);
