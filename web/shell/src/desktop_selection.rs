//! C-CLIPTYPES: the desktop's clipboard and primary selection with every type, on Wayland.
//!
//! The shell window's own `wl_data_device` (and `zwp_primary_selection_device_v1` when the
//! desktop has it) on winit's connection, dispatched by a worker thread (`web-selection`), as
//! `ime.rs` `mod desktop` does for text input. The shell never reads the data: a page that
//! reads the desktop's selection gets the desktop's offer with the page's fd
//! (`wl_data_offer.receive`), and a desktop client that reads the page's selection gets the
//! page's source with that client's fd (`request_data_device_client_selection`).
//!
//! Every source the shell sets on the desktop also offers `MARKER`: an offer that carries it is
//! the page's own selection coming back, and is not carried to the page again.
//!
//! Requests are sent from the main thread on the proxies (`Outer`), then flushed; events reach
//! the main loop through a calloop channel (`FromDesktop`, `State::desktop_selection` in
//! `selection.rs`).

use std::os::fd::{BorrowedFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use wayland_backend::client::Backend;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::WlDataDeviceManager;
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
    zwp_primary_selection_device_v1::{self, ZwpPrimarySelectionDeviceV1},
    zwp_primary_selection_offer_v1::{self, ZwpPrimarySelectionOfferV1},
    zwp_primary_selection_source_v1::{self, ZwpPrimarySelectionSourceV1},
};

use crate::clipboard::Which;

/// The private type every source the shell sets on the desktop offers.
pub const MARKER: &str = "application/x-web-shell-own";

/// Whether an offer with these types is the shell's own selection coming back.
pub fn is_own(mimes: &[String]) -> bool {
    mimes.iter().any(|m| m == MARKER)
}

/// The types the desktop is offered for a page selection with `page_mimes`: the page's, in its
/// order, without duplicates, then the marker (once).
pub fn mimes_for_desktop(page_mimes: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(page_mimes.len() + 1);
    for m in page_mimes.iter().map(String::as_str).chain([MARKER]) {
        if !out.iter().any(|o| o == m) {
            out.push(m.to_string());
        }
    }
    out
}

/// The types the page is offered for a desktop selection with `desktop_mimes`: the desktop's, in
/// its order, without duplicates and without the marker.
pub fn mimes_for_page(desktop_mimes: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(desktop_mimes.len());
    for m in desktop_mimes {
        if m != MARKER && !out.contains(m) {
            out.push(m.clone());
        }
    }
    out
}

/// "<N> types: a, b, c, ..." for the log (the first few types).
pub fn summary(mimes: &[String]) -> String {
    const FEW: usize = 4;
    let mut s = format!("{} types", mimes.len());
    if !mimes.is_empty() {
        s.push_str(": ");
        s.push_str(&mimes.iter().take(FEW).cloned().collect::<Vec<_>>().join(", "));
        if mimes.len() > FEW {
            s.push_str(", ...");
        }
    }
    s
}

/// A desktop offer's proxy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfferProxy {
    Clipboard(WlDataOffer),
    Primary(ZwpPrimarySelectionOfferV1),
}

/// The desktop's selection, as offered to the page.
#[derive(Clone, Debug)]
pub struct DesktopOffer {
    pub proxy: OfferProxy,
    pub mimes: Vec<String>,
}

impl DesktopOffer {
    /// Asks the desktop's owner to write the selection as `mime` into `fd` (flush after).
    pub fn receive(&self, mime: String, fd: BorrowedFd) {
        match &self.proxy {
            OfferProxy::Clipboard(o) => o.receive(mime, fd),
            OfferProxy::Primary(o) => o.receive(mime, fd),
        }
    }

    pub fn destroy(&self) {
        match &self.proxy {
            OfferProxy::Clipboard(o) => o.destroy(),
            OfferProxy::Primary(o) => o.destroy(),
        }
    }
}

/// A source the shell set on the desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceProxy {
    Clipboard(WlDataSource),
    Primary(ZwpPrimarySelectionSourceV1),
}

/// What the worker thread tells the main loop.
pub enum FromDesktop {
    Ready(Outer),
    /// The desktop lacks `wl_data_device_manager` (or the connection failed): text only.
    Unavailable(String),
    /// The desktop's selection changed (None: it is empty).
    Selection {
        which: Which,
        offer: Option<DesktopOffer>,
    },
    /// A desktop client reads the shell's source as `mime`, into `fd`.
    Send {
        which: Which,
        mime: String,
        fd: OwnedFd,
    },
    /// The shell's source was replaced on the desktop (it is destroyed).
    Cancelled {
        which: Which,
        source: SourceProxy,
    },
}

/// What the shell's connection last learnt from its own input objects.
#[derive(Default)]
struct Input {
    /// The serial of the latest input event (keyboard enter / key, pointer button).
    serial: AtomicU32,
    /// Whether the shell window has the desktop's keyboard focus.
    focused: AtomicBool,
}

/// The main loop's handle on the desktop's selections: requests are sent from there.
pub struct Outer {
    conn: Connection,
    qh: QueueHandle<Watch>,
    manager: WlDataDeviceManager,
    device: WlDataDevice,
    primary: Option<(ZwpPrimarySelectionDeviceManagerV1, ZwpPrimarySelectionDeviceV1)>,
    input: Arc<Input>,
}

impl Outer {
    /// Whether the shell window has the desktop's keyboard focus (needed to set a selection).
    pub fn focused(&self) -> bool {
        self.input.focused.load(Ordering::SeqCst)
    }

    pub fn has_primary(&self) -> bool {
        self.primary.is_some()
    }

    /// Sets a source offering `mimes` as the desktop's `which` (None: the desktop has no primary
    /// selection). The caller checks `focused` first.
    pub fn set(&self, which: Which, mimes: &[String]) -> Option<SourceProxy> {
        let serial = self.input.serial.load(Ordering::SeqCst);
        let source = match which {
            Which::Clipboard => {
                let src = self.manager.create_data_source(&self.qh, ());
                for m in mimes {
                    src.offer(m.clone());
                }
                self.device.set_selection(Some(&src), serial);
                SourceProxy::Clipboard(src)
            }
            Which::Primary => {
                let (manager, device) = self.primary.as_ref()?;
                let src = manager.create_source(&self.qh, ());
                for m in mimes {
                    src.offer(m.clone());
                }
                device.set_selection(Some(&src), serial);
                SourceProxy::Primary(src)
            }
        };
        self.flush();
        Some(source)
    }

    pub fn flush(&self) {
        let _ = self.conn.flush();
    }
}

/// The offer's types, as the desktop announces them.
type Mimes = Mutex<Vec<String>>;

struct Watch {
    tx: calloop::channel::Sender<FromDesktop>,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,
    input: Arc<Input>,
}

impl Watch {
    fn serial(&self, serial: u32) {
        self.input.serial.store(serial, Ordering::SeqCst);
    }
}

fn mimes_of<I: Proxy>(offer: &I) -> Vec<String> {
    offer
        .data::<Mimes>()
        .map(|m| m.lock().unwrap_or_else(|e| e.into_inner()).clone())
        .unwrap_or_default()
}

impl Dispatch<WlRegistry, GlobalListContents> for Watch {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {
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
            if caps.contains(wl_seat::Capability::Pointer) && w.pointer.is_none() {
                w.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

/// Only for the serials `set_selection` needs, and the keyboard focus.
impl Dispatch<WlKeyboard, ()> for Watch {
    fn event(w: &mut Self, _: &WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_keyboard::Event::Enter { serial, .. } => {
                w.serial(serial);
                w.input.focused.store(true, Ordering::SeqCst);
            }
            wl_keyboard::Event::Leave { .. } => w.input.focused.store(false, Ordering::SeqCst),
            wl_keyboard::Event::Key { serial, .. } => w.serial(serial),
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, ()> for Watch {
    fn event(w: &mut Self, _: &WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_pointer::Event::Button { serial, .. } = event {
            w.serial(serial);
        }
    }
}

impl Dispatch<WlDataDeviceManager, ()> for Watch {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDevice, ()> for Watch {
    fn event(w: &mut Self, _: &WlDataDevice, event: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // Drag and drop (enter / leave / motion / drop) is not carried.
        if let wl_data_device::Event::Selection { id } = event {
            let offer = id.map(|o| DesktopOffer {
                mimes: mimes_of(&o),
                proxy: OfferProxy::Clipboard(o),
            });
            let _ = w.tx.send(FromDesktop::Selection {
                which: Which::Clipboard,
                offer,
            });
        }
    }

    event_created_child!(Watch, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, Mimes::default()),
    ]);
}

impl Dispatch<WlDataOffer, Mimes> for Watch {
    fn event(_: &mut Self, _: &WlDataOffer, event: wl_data_offer::Event, mimes: &Mimes, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            mimes.lock().unwrap_or_else(|e| e.into_inner()).push(mime_type);
        }
    }
}

impl Dispatch<WlDataSource, ()> for Watch {
    fn event(w: &mut Self, src: &WlDataSource, event: wl_data_source::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                let _ = w.tx.send(FromDesktop::Send {
                    which: Which::Clipboard,
                    mime: mime_type,
                    fd,
                });
            }
            wl_data_source::Event::Cancelled => {
                let _ = w.tx.send(FromDesktop::Cancelled {
                    which: Which::Clipboard,
                    source: SourceProxy::Clipboard(src.clone()),
                });
                src.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpPrimarySelectionDeviceManagerV1, ()> for Watch {
    fn event(
        _: &mut Self,
        _: &ZwpPrimarySelectionDeviceManagerV1,
        _: <ZwpPrimarySelectionDeviceManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpPrimarySelectionDeviceV1, ()> for Watch {
    fn event(
        w: &mut Self,
        _: &ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_device_v1::Event::Selection { id } = event {
            let offer = id.map(|o| DesktopOffer {
                mimes: mimes_of(&o),
                proxy: OfferProxy::Primary(o),
            });
            let _ = w.tx.send(FromDesktop::Selection {
                which: Which::Primary,
                offer,
            });
        }
    }

    event_created_child!(Watch, ZwpPrimarySelectionDeviceV1, [
        zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE => (ZwpPrimarySelectionOfferV1, Mimes::default()),
    ]);
}

impl Dispatch<ZwpPrimarySelectionOfferV1, Mimes> for Watch {
    fn event(
        _: &mut Self,
        _: &ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        mimes: &Mimes,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event {
            mimes.lock().unwrap_or_else(|e| e.into_inner()).push(mime_type);
        }
    }
}

impl Dispatch<ZwpPrimarySelectionSourceV1, ()> for Watch {
    fn event(
        w: &mut Self,
        src: &ZwpPrimarySelectionSourceV1,
        event: zwp_primary_selection_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_primary_selection_source_v1::Event::Send { mime_type, fd } => {
                let _ = w.tx.send(FromDesktop::Send {
                    which: Which::Primary,
                    mime: mime_type,
                    fd,
                });
            }
            zwp_primary_selection_source_v1::Event::Cancelled => {
                let _ = w.tx.send(FromDesktop::Cancelled {
                    which: Which::Primary,
                    source: SourceProxy::Primary(src.clone()),
                });
                src.destroy();
            }
            _ => {}
        }
    }
}

fn watch(display: usize, tx: &calloop::channel::Sender<FromDesktop>) -> Result<(), String> {
    // Safety: the pointer is winit's live wl_display, which outlives the shell.
    let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
    let conn = Connection::from_backend(backend);
    let (globals, mut queue) = registry_queue_init::<Watch>(&conn).map_err(|e| format!("wayland registry: {e}"))?;
    let qh = queue.handle();
    let seat: WlSeat = globals
        .bind(&qh, 1..=7, ())
        .map_err(|e| format!("the desktop has no wl_seat: {e}"))?;
    let manager: WlDataDeviceManager = globals
        .bind(&qh, 1..=3, ())
        .map_err(|e| format!("the desktop has no wl_data_device_manager: {e}"))?;
    let device = manager.get_data_device(&seat, &qh, ());
    let primary = globals
        .bind::<ZwpPrimarySelectionDeviceManagerV1, _, _>(&qh, 1..=1, ())
        .ok()
        .map(|m| {
            let d = m.get_device(&seat, &qh, ());
            (m, d)
        });
    let _ = conn.flush();
    let input = Arc::new(Input::default());
    let _ = tx.send(FromDesktop::Ready(Outer {
        conn: conn.clone(),
        qh: qh.clone(),
        manager,
        device,
        primary,
        input: input.clone(),
    }));
    let mut w = Watch {
        tx: tx.clone(),
        keyboard: None,
        pointer: None,
        input,
    };
    loop {
        queue.blocking_dispatch(&mut w).map_err(|e| format!("wayland dispatch: {e}"))?;
    }
}

/// Starts the worker thread on the shell window's `wl_display` (winit's).
pub fn start(display: usize, tx: calloop::channel::Sender<FromDesktop>) {
    let spawned = std::thread::Builder::new().name("web-selection".into()).spawn(move || {
        crate::clipboard::block_quit_signals();
        if let Err(e) = watch(display, &tx) {
            let _ = tx.send(FromDesktop::Unavailable(e));
        }
    });
    if let Err(e) = spawned {
        eprintln!("web: selection thread: {e}");
    }
}
