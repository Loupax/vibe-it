//! C6: the engine host's side of the clipboard bridge (the desktop's side is `clipboard.rs`).
//!
//! The page copies by setting the selection on the shell's seat (wl_data_device for the
//! clipboard, zwp_primary_selection for the primary selection). The shell reads the text
//! (`new_selection`) and puts it on the desktop's clipboard. The other way is lazy: the
//! desktop's text is offered to the page (a compositor-owned selection served from memory)
//! when the shell window gains the desktop's focus, before a paste key reaches the page
//! (Ctrl-V, Ctrl-Shift-V, Shift-Insert) and before a middle click on the page (primary). Each
//! time, only when the desktop holds something else than what was last carried across, so a
//! copy made in the page keeps its own selection (and its rich types, text/html) for a paste
//! in the page.

use std::io::{Read, Write};
use std::os::fd::OwnedFd;

use smithay::input::Seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::selection::data_device::{
    request_data_device_client_selection, set_data_device_focus, set_data_device_selection, DataDeviceHandler, DataDeviceState,
};
use smithay::wayland::selection::primary_selection::{
    request_primary_client_selection, set_primary_focus, set_primary_selection, PrimarySelectionHandler, PrimarySelectionState,
};
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
use smithay::{delegate_data_device, delegate_primary_selection};

use crate::clipboard::{pick_text_mime, Desktop, SystemClipboard, Which, TEXT_MIMES};
use crate::state::State;

/// The most of a page's copy carried to the desktop.
const MAX_COPY: u64 = 16 << 20;

/// A page's copy, read on a thread: which selection and its text.
pub type Copied = (Which, Result<String, String>);

pub struct Bridge {
    pub desktop: Desktop,
    /// Where the reader threads send a page's copy (the compositor's loop receives it).
    pub copies: calloop::channel::Sender<Copied>,
    /// The text last carried across, per selection: both sides hold it.
    synced_clipboard: Option<String>,
    synced_primary: Option<String>,
    /// Copies the page made in this dispatch, with their types: read after it, once smithay has
    /// stored the selection (`new_selection` is called before it does).
    pending: Vec<(Which, Vec<String>)>,
}

impl Bridge {
    pub fn new(desktop: Desktop, copies: calloop::channel::Sender<Copied>) -> Bridge {
        Bridge {
            desktop,
            copies,
            synced_clipboard: None,
            synced_primary: None,
            pending: Vec::new(),
        }
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
    pub fn sync_from_desktop(&mut self, which: Which) {
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
        match which {
            Which::Clipboard => set_data_device_selection(&self.display_handle, &self.seat, mimes, text.clone()),
            Which::Primary => set_primary_selection(&self.display_handle, &self.seat, mimes, text.clone()),
        }
        *self.bridge.synced(which) = Some(text);
    }

    /// A page's copy has been read: it goes to the desktop.
    pub fn page_copied(&mut self, (which, text): Copied) {
        let text = match text {
            Ok(t) => t,
            Err(e) => {
                self.log(&format!("clipboard: cannot read the page's {}: {e}", name(which)));
                return;
            }
        };
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

    /// Reads the copies the page made since the last call (from the event loop, after dispatch).
    pub fn read_page_copies(&mut self) {
        for (which, offered) in std::mem::take(&mut self.bridge.pending) {
            self.read_page_selection(which, offered);
        }
    }

    /// Asks the client that owns the selection for its text; a thread reads it.
    fn read_page_selection(&mut self, which: Which, offered: Vec<String>) {
        let Some(mime) = pick_text_mime(&offered) else {
            self.log(&format!(
                "clipboard: the page's {} holds no text ({offered:?}), not carried",
                name(which)
            ));
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
    /// The text a compositor-owned selection holds.
    type SelectionUserData = String;

    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, _seat: Seat<Self>) {
        if let Some(source) = source {
            let which = which_of(ty);
            self.bridge.pending.retain(|(w, _)| *w != which);
            self.bridge.pending.push((which, source.mime_types()));
        }
    }

    fn send_selection(&mut self, ty: SelectionTarget, mime: String, fd: OwnedFd, _seat: Seat<Self>, text: &String) {
        self.log(&format!("clipboard: the page reads the {} as {mime}", name(which_of(ty))));
        // A thread: the page may read slowly, and a full pipe must not block the compositor.
        let text = text.clone();
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
