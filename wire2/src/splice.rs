//! Zero-copy byte pipe between two sockets with splice(2) (SPEC "Performance requirements").
//!
//! Each direction moves bytes socket -> pipe -> socket inside the kernel; the byte counts splice returns
//! feed the flow's counters. Pipes come from a small shared pool and are held only while bytes are in
//! flight (a direction takes one when its source is readable and returns it once it has drained it), so
//! an idle flow holds no pipe: thousands of open tunnels cost two sockets each, not two extra pipes,
//! and stay under the kernel's per-user pipe budget (fs.pipe-user-pages-soft).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::Interest;

use crate::conn::Conn;

/// Requested pipe capacity. Larger pipes mean fewer syscalls per byte; the kernel may refuse (then the
/// default 64 KiB stays).
const PIPE_SIZE: libc::c_int = 256 * 1024;
const POOL_MAX: usize = 128;

struct Pipe {
    r: OwnedFd,
    w: OwnedFd,
    cap: usize,
}

static POOL: Mutex<Vec<Pipe>> = Mutex::new(Vec::new());

impl Pipe {
    fn new() -> io::Result<Pipe> {
        let mut fds = [0 as RawFd; 2];
        // SAFETY: pipe2 fills two fds on success; we take ownership right after.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fresh fds from pipe2, owned by nobody else.
        let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        // SAFETY: fcntl on our own pipe fd.
        let cap = unsafe {
            libc::fcntl(w.as_raw_fd(), libc::F_SETPIPE_SZ, PIPE_SIZE);
            libc::fcntl(w.as_raw_fd(), libc::F_GETPIPE_SZ)
        };
        Ok(Pipe { r, w, cap: if cap > 0 { cap as usize } else { 64 * 1024 } })
    }

    fn take() -> io::Result<Pipe> {
        let pooled = POOL.lock().unwrap_or_else(|e| e.into_inner()).pop();
        match pooled {
            Some(p) => Ok(p),
            None => Pipe::new(),
        }
    }

    /// Return an EMPTY pipe to the pool (a pipe that still holds bytes is simply dropped).
    fn give(self) {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        if pool.len() < POOL_MAX {
            pool.push(self);
        }
    }
}

fn splice(from: RawFd, to: RawFd, len: usize) -> io::Result<usize> {
    // SAFETY: splice(2) between fds we own, no offsets.
    let n = unsafe {
        libc::splice(
            from,
            std::ptr::null_mut(),
            to,
            std::ptr::null_mut(),
            len,
            libc::SPLICE_F_MOVE | libc::SPLICE_F_NONBLOCK,
        )
    };
    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
}

/// Copy `src` -> `dst` until `src` reaches EOF, then shut down `dst`'s write side (half-close).
async fn pump(src: &Conn, dst: &Conn, counter: &AtomicU64) -> io::Result<()> {
    loop {
        src.ready(Interest::READABLE).await?;
        let pipe = Pipe::take()?;
        // The pipe is empty, so EAGAIN here can only come from the socket (correctly clearing readiness).
        let n = match src.try_io(Interest::READABLE, || splice(src.as_raw_fd(), pipe.w.as_raw_fd(), pipe.cap)) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                pipe.give();
                continue;
            }
            Err(e) => return Err(e),
        };
        if n == 0 {
            pipe.give();
            dst.shutdown_write();
            return Ok(());
        }
        let mut left = n;
        while left > 0 {
            dst.ready(Interest::WRITABLE).await?;
            // The pipe holds `left` bytes, so EAGAIN here can only come from the socket.
            match dst.try_io(Interest::WRITABLE, || splice(pipe.r.as_raw_fd(), dst.as_raw_fd(), left)) {
                Ok(m) => {
                    left -= m;
                    counter.fetch_add(m as u64, Ordering::Relaxed);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
        }
        pipe.give();
    }
}

/// Pipe bytes both ways between the browser side `a` and the upstream side `b` until both directions
/// have seen EOF, or either fails. `up` counts a -> b bytes, `down` counts b -> a bytes.
pub async fn splice_bidi(a: &Conn, b: &Conn, up: &AtomicU64, down: &AtomicU64) -> io::Result<()> {
    tokio::try_join!(pump(a, b, up), pump(b, a, down)).map(|_| ())
}

/// Number of pipes currently pooled (tests / debug dumps).
pub fn pooled_pipes() -> usize {
    POOL.lock().map(|p| p.len()).unwrap_or(0)
}
