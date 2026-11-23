//! The control socket (web/PROTOCOL.md "Transport"): the shell listens on a Unix socket in
//! a private directory under $XDG_RUNTIME_DIR, the engine host connects once.

use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};

use crate::protocol::LineBuffer;
use crate::state::State;

/// `:help`'s page, written next to the socket at start (C3).
pub const HELP_FILE: &str = "help.html";

/// Writes `:help`'s page (built into the binary) into the shell's private run directory;
/// returns its `file://` URL for the host to load.
pub fn write_help(control_path: &Path) -> std::io::Result<String> {
    let dir = control_path.parent().ok_or_else(|| std::io::Error::other("no run directory"))?;
    let path = dir.join(HELP_FILE);
    std::fs::write(&path, include_str!("help.html"))?;
    url::Url::from_file_path(&path)
        .map(String::from)
        .map_err(|_| std::io::Error::other(format!("{}: not an absolute path", path.display())))
}

pub struct Control {
    pub path: PathBuf,
    listener: UnixListener,
    /// P-FORGET-SHELL: `web --forget-site` clients (forget.rs).
    forget_listener: UnixListener,
}

impl Control {
    pub fn listen() -> std::io::Result<Control> {
        use std::os::unix::fs::DirBuilderExt;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| std::io::Error::other("XDG_RUNTIME_DIR is not set"))?;
        let dir = Path::new(&runtime).join(format!("web-{}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let path = dir.join("control.sock");
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        let forget_listener = UnixListener::bind(dir.join(crate::forget::FORGET_SOCK))?;
        forget_listener.set_nonblocking(true)?;
        Ok(Control {
            path,
            listener,
            forget_listener,
        })
    }

    /// Accepts the host's one connection, then reads its events into the state.
    pub fn register(self, handle: &LoopHandle<'static, State>) -> std::io::Result<()> {
        handle
            .insert_source(
                Generic::new(self.forget_listener, Interest::READ, Mode::Level),
                |_, listener, state| loop {
                    match listener.accept() {
                        Ok((s, _)) => forget_client(s, state),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(PostAction::Continue),
                        Err(e) => return Err(e),
                    }
                },
            )
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let h = handle.clone();
        handle
            .insert_source(
                Generic::new(self.listener, Interest::READ, Mode::Level),
                move |_, listener, state| {
                    let stream = match listener.accept() {
                        Ok((s, _)) => s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(PostAction::Continue),
                        Err(e) => return Err(e),
                    };
                    state.log("engine host connected to the control socket");
                    stream.set_nonblocking(true)?;
                    state.control = Some(stream.try_clone()?);
                    let mut lines = LineBuffer::default();
                    h.insert_source(Generic::new(stream, Interest::READ, Mode::Level), move |_, stream, state| {
                        read_events(stream, &mut lines, state)
                    })
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                    // One host, one connection.
                    Ok(PostAction::Remove)
                },
            )
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(())
    }
}

/// A `web --forget-site` or `web URL` client: reads its one request line (it sends it at once; a short
/// timeout keeps a silent client from stalling the loop), then the state sends the command
/// and answers the client when the host's `forgot` arrives.
fn forget_client(stream: UnixStream, state: &mut State) {
    use std::io::BufRead;
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    let mut line = String::new();
    let read = std::io::BufReader::new((&stream).take(4096)).read_line(&mut line);
    match (read, crate::forget::parse_request(&line)) {
        (Ok(_), Some(crate::forget::Request::ForgetSite(site))) => state.forget_requested(site, stream),
        (Ok(_), Some(crate::forget::Request::Open(url))) => state.open_requested(&url, stream),
        _ => state.log(&format!("forget socket: bad request {line:?}")),
    }
}

fn read_events(stream: &UnixStream, lines: &mut LineBuffer, state: &mut State) -> std::io::Result<PostAction> {
    let mut buf = [0u8; 16384];
    let mut stream = stream;
    loop {
        match stream.read(&mut buf) {
            Ok(0) => {
                state.log("control socket closed by the engine host");
                state.control = None;
                return Ok(PostAction::Remove);
            }
            Ok(n) => {
                for line in lines.push(&buf[..n]) {
                    state.host_event(&line);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(PostAction::Continue),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                state.log(&format!("control socket: {e}"));
                state.control = None;
                return Ok(PostAction::Remove);
            }
        }
    }
}

/// Removes the sockets, `:help`'s page and their directory.
pub fn cleanup(path: &Path) {
    if let Some(dir) = path.parent() {
        crate::forget::remove_run_dir(dir);
    }
}
