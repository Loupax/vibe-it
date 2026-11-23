//! The control socket (web/PROTOCOL.md "Transport"): the shell listens on a Unix socket in
//! a private directory under $XDG_RUNTIME_DIR, the engine host connects once.

use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};

use crate::protocol::LineBuffer;
use crate::state::State;

pub struct Control {
    pub path: PathBuf,
    listener: UnixListener,
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
        Ok(Control { path, listener })
    }

    /// Accepts the host's one connection, then reads its events into the state.
    pub fn register(self, handle: &LoopHandle<'static, State>) -> std::io::Result<()> {
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

/// Removes the socket and its directory.
pub fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}
