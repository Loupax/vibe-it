//! C6: the host desktop's clipboard and primary selection (the "outer" side of the bridge;
//! the engine host's side, wl_data_device and primary selection on the shell's own seat, is
//! in `selection.rs`).
//!
//! The shell is a window on the user's desktop (winit: Wayland or X11), so the desktop's
//! clipboard is reached as that window: on Wayland through smithay-clipboard on the window's
//! own connection (core wl_data_device and zwp_primary_selection, which every Wayland desktop
//! has; data-control, which arboard / wl-clipboard-rs use, is missing on GNOME), on X11
//! through x11-clipboard (CLIPBOARD and PRIMARY). Both come through `copypasta`, as in
//! alacritty. On X11 this is the whole bridge, text only. On Wayland (C-CLIPTYPES) the page's
//! selections follow the desktop's with every type (`desktop_selection.rs`), and this is used
//! only for the shell's own text reads and writes (`yy`, `@+`, paste into the `:` and `/` lines).
//!
//! The clipboard libraries block, so they run on a worker thread; the compositor waits for an
//! answer at most `WAIT`, so a desktop clipboard owner that never answers cannot freeze it.

use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// Which selection: the clipboard (Ctrl-C / Ctrl-V, register `+`) or the primary selection
/// (select / middle-click).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Which {
    Clipboard,
    Primary,
}

/// The desktop's clipboard as `Keys` and the bridge see it. `read` gives `Ok("")` for an empty
/// clipboard and `Err` when it cannot be reached.
pub trait SystemClipboard {
    fn read(&mut self, which: Which) -> Result<String, String>;
    fn write(&mut self, which: Which, text: &str) -> Result<(), String>;
}

/// The longest the compositor waits for the desktop's clipboard.
pub const WAIT: Duration = Duration::from_millis(1000);

/// The text types a page's copy is read in, best first, and the ones the shell offers the
/// page (the usual Wayland and X11 names for UTF-8 text).
pub const TEXT_MIMES: &[&str] = &["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING", "TEXT"];

/// The text type to read a copy offered in `offered` as; None when it holds no text.
pub fn pick_text_mime(offered: &[String]) -> Option<String> {
    TEXT_MIMES.iter().find(|m| offered.iter().any(|o| o == *m)).map(|m| m.to_string())
}

/// KEYS.md "Paste": control characters (a newline, a tab) become a space, a CR before a newline
/// goes, trailing whitespace is dropped.
pub fn clean_paste_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' && chars.peek() == Some(&'\n') {
            continue;
        }
        out.push(if c.is_control() { ' ' } else { c });
    }
    out.truncate(out.trim_end().len());
    out
}

/// What the shell window is on the desktop.
#[derive(Clone, Copy, Debug)]
pub enum DesktopKind {
    /// A Wayland window: the `wl_display *` of winit's connection.
    Wayland(usize),
    X11,
    /// Neither (no clipboard).
    None,
}

enum Req {
    Read(Which, mpsc::Sender<Result<String, String>>),
    Write(Which, String, mpsc::Sender<Result<(), String>>),
}

/// C-CLIPTYPES: per selection (clipboard, primary), `Some(text)` while the desktop's selection is
/// the page's own, carried there by the shell (`desktop_selection.rs`); `text` is the page's copy as
/// text ("" when it has none, or before it has been read).
pub type Own = Arc<Mutex<[Option<String>; 2]>>;

fn index(which: Which) -> usize {
    match which {
        Which::Clipboard => 0,
        Which::Primary => 1,
    }
}

/// The desktop's clipboard, served by a worker thread. Cheap to clone.
#[derive(Clone)]
pub struct Desktop {
    tx: mpsc::Sender<Req>,
    own: Own,
}

type Provider = Box<dyn copypasta::ClipboardProvider>;

fn providers(kind: DesktopKind) -> Result<(Provider, Provider), String> {
    match kind {
        DesktopKind::Wayland(display) => {
            // Safety: `display` is winit's wl_display, alive as long as the shell window (the
            // whole run of the shell).
            let (primary, clipboard) =
                unsafe { copypasta::wayland_clipboard::create_clipboards_from_external(display as *mut std::ffi::c_void) };
            Ok((Box::new(clipboard), Box::new(primary)))
        }
        DesktopKind::X11 => {
            use copypasta::x11_clipboard::{Clipboard, Primary, X11ClipboardContext};
            let c = X11ClipboardContext::<Clipboard>::new().map_err(|e| e.to_string())?;
            let p = X11ClipboardContext::<Primary>::new().map_err(|e| e.to_string())?;
            Ok((Box::new(c), Box::new(p)))
        }
        DesktopKind::None => Err("the shell window is neither Wayland nor X11".into()),
    }
}

/// smithay-clipboard and x11-clipboard report an empty selection as an error.
fn is_empty_error(e: &str) -> bool {
    e.contains("selection is empty") || e.contains("supported mime-type is not found")
}

/// SIGTERM and SIGINT reach the shell through calloop's signalfd, which works only while every
/// thread blocks them: this thread (and smithay-clipboard's, started from it) is created before
/// the main thread blocks them, and would otherwise take them and kill the process.
pub fn block_quit_signals() {
    // Safety: plain libc calls on a local, zeroed signal set.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

impl Desktop {
    /// C-CLIPTYPES: the page's text while the desktop's `which` is the page's own selection.
    pub fn own(&self, which: Which) -> Option<String> {
        self.own.lock().unwrap_or_else(|e| e.into_inner())[index(which)].clone()
    }

    /// C-CLIPTYPES: records whether the desktop's `which` is the page's own selection (and its text).
    pub fn set_own(&self, which: Which, text: Option<String>) {
        self.own.lock().unwrap_or_else(|e| e.into_inner())[index(which)] = text;
    }

    /// A Desktop whose worker is gone (tests: every request to the desktop fails).
    #[cfg(test)]
    pub fn unconnected() -> Desktop {
        let (tx, _) = mpsc::channel::<Req>();
        Desktop { tx, own: Own::default() }
    }

    pub fn start(kind: DesktopKind) -> Desktop {
        let (tx, rx) = mpsc::channel::<Req>();
        let spawned = std::thread::Builder::new().name("web-clipboard".into()).spawn(move || {
            block_quit_signals();
            let mut p = providers(kind);
            if let Err(e) = &p {
                eprintln!("web: the desktop clipboard is not available: {e}");
            }
            for req in rx {
                match req {
                    Req::Read(w, reply) => {
                        let r = match &mut p {
                            Ok((c, pr)) => {
                                let prov = if w == Which::Clipboard { c } else { pr };
                                match prov.get_contents() {
                                    Ok(t) => Ok(t),
                                    Err(e) if is_empty_error(&e.to_string()) => Ok(String::new()),
                                    Err(e) => Err(e.to_string()),
                                }
                            }
                            Err(e) => Err(e.clone()),
                        };
                        let _ = reply.send(r);
                    }
                    Req::Write(w, text, reply) => {
                        let r = match &mut p {
                            Ok((c, pr)) => {
                                let prov = if w == Which::Clipboard { c } else { pr };
                                prov.set_contents(text).map_err(|e| e.to_string())
                            }
                            Err(e) => Err(e.clone()),
                        };
                        let _ = reply.send(r);
                    }
                }
            }
        });
        if let Err(e) = spawned {
            eprintln!("web: clipboard thread: {e}");
        }
        Desktop { tx, own: Own::default() }
    }
}

impl SystemClipboard for Desktop {
    fn read(&mut self, which: Which) -> Result<String, String> {
        // C-CLIPTYPES: the desktop would ask the page's selection through the shell's own source,
        // whose `send` is served on the main thread, the one waiting here: answer from memory.
        if let Some(text) = self.own(which) {
            return Ok(text);
        }
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Req::Read(which, reply))
            .map_err(|_| "the clipboard thread is gone".to_string())?;
        rx.recv_timeout(WAIT)
            .map_err(|_| "the desktop clipboard did not answer".to_string())?
    }

    fn write(&mut self, which: Which, text: &str) -> Result<(), String> {
        // smithay-clipboard's source replaces the page's on the desktop (its `cancelled` comes later).
        self.set_own(which, None);
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Req::Write(which, text.to_string(), reply))
            .map_err(|_| "the clipboard thread is gone".to_string())?;
        rx.recv_timeout(WAIT)
            .map_err(|_| "the desktop clipboard did not answer".to_string())?
    }
}
