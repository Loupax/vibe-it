//! QY54a2 (REGRESSIONS.md R57 section 4): the `wl-drain` thread.
//!
//! When the main thread is busy (a long script callback, a forced layout), nobody reads the Wayland
//! socket, the compositor's send buffer for scratchy fills with pointer-motion events, and the
//! compositor disconnects scratchy (R56: lost at ~190-225 unread events). This thread keeps
//! reading the socket while the main thread is busy. It only READS: libwayland puts every event
//! on the queue of the object it belongs to (winit's queue), and only the main thread dispatches
//! that queue, so input order and hit-testing are unchanged.
//!
//! It works through libwayland's multi-reader protocol (`wl_display_prepare_read_queue` / `poll` /
//! `wl_display_read_events`, or `wl_display_cancel_read`) on a second `Connection` made from
//! winit's `wl_display*` with its own private, always-empty event queue. The thread never
//! dispatches and never creates objects. It stops at the first error or POLLHUP (the main thread
//! sees the same error in its own read and QB1 prints its one line), when woken through a pipe by
//! `stop` (`App::exiting`, `Drop`), and when the thread that started it panics (a chained panic
//! hook, so an unwind through `run_app` never drops the display under a polling thread).
//!
//! QY54a4 (R57 section 7): while the main thread is idle in calloop it reads the socket itself, and a
//! second poller on the same fd only costs CPU (thread polls/reads plus extra main wake-ups). So the
//! main thread publishes a "busy since" stamp (`WlDrain::busy` in `new_events`, `WlDrain::idle` as the
//! last thing in `about_to_wait`: the stamp is non-zero for the whole iteration, callbacks included);
//! the thread only touches the Wayland fd once the stamp is older than `BUSY_AFTER`, and otherwise
//! sleeps on its wake socket for at most `IDLE_POLL`, then re-checks.
//! `SCRATCHY_WL_DRAIN=0` disables it; non-Wayland displays get no thread.

use std::ffi::c_void;
use std::io::{ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Once, OnceLock};
use std::thread::{JoinHandle, ThreadId};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use wayland_backend::client::{Backend, WaylandError};
use wayland_client::Connection;
use winit::event_loop::ActiveEventLoop;
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

/// The main thread must have been busy this long before the thread reads the socket (the
/// compositor only gives up after ~190 unread motion events, i.e. ~400 ms at 500 Hz).
const BUSY_AFTER: Duration = Duration::from_millis(50);
/// How long the thread sleeps while main is idle before it re-checks the stamp.
const IDLE_POLL: Duration = Duration::from_millis(50);
/// Poll timeout while reading: how soon the thread notices that main went idle again.
const READ_POLL: Duration = Duration::from_millis(20);

/// Monotonic nanoseconds since the first call, plus one (0 is reserved for "idle").
fn stamp_now() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64 + 1
}

/// State shared by the owner (`WlDrain`), the thread and the panic hook.
struct Shared {
    stop: AtomicBool,
    /// `stamp_now()` of the start of the main thread's current loop iteration, 0 while it is idle.
    busy_since: AtomicU64,
    /// Set by the thread as its last action (after the foreign connection is dropped).
    exited: AtomicBool,
    /// Set when the panic hook stopped the thread (the owner may restart it if the panic was caught).
    by_panic: AtomicBool,
    /// Write end of the wake pipe; the thread polls the read end next to the Wayland fd.
    wake: UnixStream,
    /// The thread that started the drain (the main thread); only its panics stop the drain.
    owner: ThreadId,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Shared {
    fn signal(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // Non-blocking: one byte is enough, a full pipe already means "wake up".
        let _ = (&self.wake).write(&[1]);
    }

    fn join(&self, handle: Option<JoinHandle<()>>) {
        if let Some(h) = handle {
            let _ = h.join();
        }
    }
}

/// Every running drain, for the panic hook (one in the app; tests may run several in parallel).
static REGISTRY: Mutex<Vec<Arc<Shared>>> = Mutex::new(Vec::new());
static HOOK: Once = Once::new();

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where the `wl_display*` comes from, and what keeps it alive while the thread runs.
enum Display {
    /// winit's display: winit's `Connection` owns it until `run_app` returns, and the thread is
    /// joined before that (`App::exiting`, or the panic hook before an unwind).
    Winit(NonNull<c_void>),
    /// Tests: the clone is moved into the thread and dropped after the foreign connection.
    #[cfg(test)]
    Conn(Connection),
}

/// The running `wl-drain` thread. Dropping it stops and joins the thread.
pub(crate) struct WlDrain {
    shared: Arc<Shared>,
}

/// The `wl_display*` to drain, or `None`: not a Wayland display, or `SCRATCHY_WL_DRAIN=0`.
pub(crate) fn wanted(raw: RawDisplayHandle, env: Option<&str>) -> Option<NonNull<c_void>> {
    if env.map(str::trim) == Some("0") {
        return None;
    }
    match raw {
        RawDisplayHandle::Wayland(h) => Some(h.display),
        _ => None,
    }
}

impl WlDrain {
    /// Starts the thread for the event loop's Wayland display (call once the window exists).
    pub(crate) fn start(event_loop: &ActiveEventLoop) -> Option<WlDrain> {
        let raw = event_loop.display_handle().ok()?.as_raw();
        let display = wanted(raw, std::env::var("SCRATCHY_WL_DRAIN").ok().as_deref())?;
        Self::spawn(Display::Winit(display))
    }

    /// Tests: a drain on `conn`'s display (the thread keeps a clone of `conn` alive).
    #[cfg(test)]
    pub(crate) fn start_for_test(conn: &Connection) -> Option<WlDrain> {
        Self::spawn(Display::Conn(conn.clone()))
    }

    fn spawn(display: Display) -> Option<WlDrain> {
        let (wake_rx, wake_tx) = UnixStream::pair().ok()?;
        wake_tx.set_nonblocking(true).ok()?;
        #[allow(unused_mut)]
        let (ptr, mut keep_alive): (*mut c_void, Option<Connection>) = match display {
            Display::Winit(p) => (p.as_ptr(), None),
            #[cfg(test)]
            Display::Conn(c) => (c.backend().display_ptr().cast(), Some(c)),
        };
        // SAFETY: `from_foreign_display` needs the `wl_display*` to stay valid for as long as the
        // returned backend (and its clones) exists. The backend never disconnects a foreign
        // display; it only creates (and on drop destroys) its own private `wl_event_queue`. The
        // only clone is the `Connection` moved into the thread below, which drops it before it
        // exits. For `Display::Winit` the display is owned by winit's own `Connection`, alive
        // until `run_app` returns; the thread is joined before that: in `App::exiting` (winit
        // calls it on a clean exit and on the `ExitFailure` path, before it drops the event loop)
        // and, if the main thread panics, in the chained panic hook, which runs before the unwind
        // reaches winit. For `Display::Conn` (tests) the owning `Connection` is moved into the
        // thread and dropped after the foreign one.
        let conn = Connection::from_backend(unsafe { Backend::from_foreign_display(ptr.cast()) });
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            busy_since: AtomicU64::new(0),
            exited: AtomicBool::new(false),
            by_panic: AtomicBool::new(false),
            wake: wake_tx,
            owner: std::thread::current().id(),
            thread: Mutex::new(None),
        });
        let s = shared.clone();
        let handle = std::thread::Builder::new()
            .name("wl-drain".into())
            .spawn(move || {
                drain_loop(&conn, &wake_rx, &s.stop, &s.busy_since);
                drop(conn);
                drop(keep_alive.take());
                s.exited.store(true, Ordering::SeqCst);
            })
            .ok()?;
        *lock(&shared.thread) = Some(handle);
        HOOK.call_once(install_panic_hook);
        lock(&REGISTRY).push(shared.clone());
        Some(WlDrain { shared })
    }

    /// The main thread starts a loop iteration (`new_events`): the thread starts timing the stall.
    pub(crate) fn busy(&self) {
        self.shared.busy_since.store(stamp_now(), Ordering::Release);
    }

    /// The main thread is about to sleep in calloop (end of `about_to_wait`): it reads the socket itself.
    pub(crate) fn idle(&self) {
        self.shared.busy_since.store(0, Ordering::Release);
    }

    /// Tests: main has been busy for `ago` already.
    #[cfg(test)]
    pub(crate) fn busy_for_test(&self, ago: Duration) {
        let now = stamp_now();
        self.shared.busy_since.store(now.saturating_sub(ago.as_nanos() as u64).max(1), Ordering::Release);
    }

    /// Stops the thread (flag + one byte on the wake pipe) and joins it.
    pub(crate) fn stop(&mut self) {
        self.shared.signal();
        let handle = lock(&self.shared.thread).take();
        self.shared.join(handle);
        lock(&REGISTRY).retain(|s| !Arc::ptr_eq(s, &self.shared));
    }

    /// The panic hook stopped this drain. If the app is still running the panic was caught (e.g.
    /// js_geom's layout guard), so the owner may start a new one.
    pub(crate) fn stopped_by_panic(&self) -> bool {
        self.shared.by_panic.load(Ordering::SeqCst)
    }

    /// The thread has exited (stopped, or by itself on an error / POLLHUP).
    #[cfg(test)]
    pub(crate) fn is_finished(&self) -> bool {
        self.shared.exited.load(Ordering::SeqCst)
    }
}

impl Drop for WlDrain {
    fn drop(&mut self) {
        self.stop();
    }
}

/// While main is idle (stamp 0) or has been busy for less than `BUSY_AFTER`: sleep on the wake
/// socket only. Otherwise prepare_read / poll / read without dispatch, with a short poll timeout so
/// the thread goes back to sleep soon after main does. Runs until stopped or the connection fails.
fn drain_loop(conn: &Connection, wake: &UnixStream, stop: &AtomicBool, busy_since: &AtomicU64) {
    let poll_timeout = |d: Duration| rustix::event::Timespec::try_from(d).ok();
    while !stop.load(Ordering::SeqCst) {
        let since = busy_since.load(Ordering::Acquire);
        let sleep = if since == 0 {
            Some(IDLE_POLL)
        } else {
            let age = Duration::from_nanos(stamp_now().saturating_sub(since));
            BUSY_AFTER.checked_sub(age).filter(|d| !d.is_zero())
        };
        if let Some(d) = sleep {
            let mut fds = [PollFd::new(wake, PollFlags::IN)];
            match poll(&mut fds, poll_timeout(d).as_ref()) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => break,
            }
            if !fds[0].revents().is_empty() {
                break; // stop() wrote a byte (or the owner is gone)
            }
            continue; // re-check the stamp
        }
        // Our private queue is never filled (we create no objects), so this only fails on a
        // broken display.
        let Some(guard) = conn.prepare_read() else { break };
        let (wl, woken) = {
            let fd = guard.connection_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN), PollFd::new(wake, PollFlags::IN)];
            match poll(&mut fds, poll_timeout(READ_POLL).as_ref()) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue, // drops the guard = cancel_read
                Err(_) => break,
            }
            (fds[0].revents(), !fds[1].revents().is_empty())
        };
        if woken || stop.load(Ordering::SeqCst) {
            break; // the guard's drop cancels the read
        }
        if wl.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL) {
            break; // the compositor is gone; the main thread reports it
        }
        if wl.contains(PollFlags::IN) {
            match guard.read_without_dispatch() {
                Ok(()) => {}
                Err(WaylandError::Io(e)) if e.kind() == ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
        }
    }
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prev(info);
        stop_for_panic();
    }));
}

/// Panic hook: stop (and join, bounded) every drain the panicking thread started, before the
/// unwind can drop the display. Other threads' panics leave the drain alone.
fn stop_for_panic() {
    let me = std::thread::current().id();
    // try_lock: the panicking thread may itself hold the registry (never wait on ourselves).
    let mine: Vec<Arc<Shared>> = match REGISTRY.try_lock() {
        Ok(reg) => reg.iter().filter(|s| s.owner == me).cloned().collect(),
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner().iter().filter(|s| s.owner == me).cloned().collect(),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    for s in mine {
        s.by_panic.store(true, Ordering::SeqCst);
        s.signal();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !s.exited.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        if s.exited.load(Ordering::SeqCst) {
            let handle = s.thread.try_lock().ok().and_then(|mut t| t.take());
            s.join(handle);
        }
    }
}
