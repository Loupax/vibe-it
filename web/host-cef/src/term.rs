//! SIGTERM is a forced quit (C-CLOSE-POLITE): the shell forwards its own SIGTERM here, and the
//! host closes every page without waiting for `beforeunload` answers (`page::quit`), then exits
//! normally through CEF's shutdown. The handler only writes one byte to a pipe (async-signal-safe);
//! a thread reads it and posts a task to the UI thread.

use cef::*;
use std::sync::atomic::{AtomicI32, Ordering};

/// The pipe's write end, for the handler (-1 until `install`).
static WRITE_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_sigterm(_sig: libc::c_int) {
    let fd = WRITE_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let b = 1u8;
        // SAFETY: write(2) is async-signal-safe; `b` lives for the call.
        unsafe {
            libc::write(fd, &b as *const u8 as *const libc::c_void, 1);
        }
    }
}

wrap_task! {
    struct TermTask {}

    impl Task {
        fn execute(&self) {
            log!("SIGTERM: forced quit");
            crate::page::quit();
        }
    }
}

/// Installs the SIGTERM handler. Call after `cef::initialize` (which may install Chromium's own
/// handler: ours replaces it) and before the message loop.
pub fn install() {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` has room for the two descriptors pipe(2) writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        log!("SIGTERM handler: pipe failed: {}", std::io::Error::last_os_error());
        return;
    }
    let [read_fd, write_fd] = fds;
    WRITE_FD.store(write_fd, Ordering::Relaxed);
    // SAFETY: a zeroed sigaction is valid; the handler only does async-signal-safe work.
    let ok = unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_sigterm as extern "C" fn(libc::c_int) as libc::sighandler_t;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut()) == 0
    };
    if !ok {
        log!("SIGTERM handler: sigaction failed: {}", std::io::Error::last_os_error());
        return;
    }
    std::thread::spawn(move || loop {
        let mut b = 0u8;
        // SAFETY: reads one byte into `b`.
        let n = unsafe { libc::read(read_fd, &mut b as *mut u8 as *mut libc::c_void, 1) };
        match n {
            1 => {
                post_task(ThreadId::UI, Some(&mut TermTask::new()));
            }
            -1 if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted => {}
            _ => break,
        }
    });
}
