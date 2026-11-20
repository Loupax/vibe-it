//! QY54a2 (REGRESSIONS.md R57 section 5): the wl-drain thread, over a socketpair "compositor".

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use wayland_client::protocol::{wl_callback, wl_display::WlDisplay};
use wayland_client::{Connection, Dispatch, QueueHandle};
use winit::raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle, XlibDisplayHandle};

use crate::wl_drain::{WlDrain, wanted};

/// The callbacks' `done` events in dispatch order (the user data is the request index).
#[derive(Default)]
struct Got(Vec<u32>);

impl Dispatch<wl_callback::WlCallback, u32> for Got {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        idx: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { callback_data } = event {
            assert_eq!(callback_data, *idx, "done serial matches its request");
            state.0.push(*idx);
        }
    }
}

/// A client `Connection` (system libwayland) on one end of a socketpair; the other end is the test's
/// "compositor", which writes raw wire messages.
fn pair() -> (Connection, UnixStream) {
    let (client, server) = UnixStream::pair().unwrap();
    (Connection::from_socket(client).unwrap(), server)
}

/// Writes as much of `bytes` as the socket takes before `deadline`; returns the count written.
fn write_until(server: &UnixStream, bytes: &[u8], deadline: Instant) -> usize {
    server.set_nonblocking(true).unwrap();
    let mut off = 0;
    while off < bytes.len() {
        match (&*server).write(&bytes[off..]) {
            Ok(n) => off += n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let ts = rustix::event::Timespec::try_from(left).unwrap();
                let mut fds = [PollFd::new(server, PollFlags::OUT)];
                let _ = poll(&mut fds, Some(&ts));
            }
            Err(e) => panic!("server write: {e}"),
        }
    }
    off
}

/// Sends `n` `wl_display.sync` requests (never dispatched) and returns the matching server events:
/// `wl_callback.done(i)` + `wl_display.delete_id` for each, in request order.
fn sync_requests(conn: &Connection, server: &UnixStream, qh: &QueueHandle<Got>, n: u32) -> Vec<u8> {
    let display: WlDisplay = conn.display();
    for i in 0..n {
        let _ = display.sync(qh, i);
    }
    conn.flush().unwrap();
    // Each request: object 1, (12 << 16) | opcode 0, new callback id.
    let mut req = vec![0u8; 12 * n as usize];
    server.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    (&*server).read_exact(&mut req).unwrap();
    let mut out = Vec::with_capacity(24 * n as usize);
    for (i, r) in req.chunks(12).enumerate() {
        let word = |k: usize| u32::from_ne_bytes(r[k * 4..k * 4 + 4].try_into().unwrap());
        assert_eq!((word(0), word(1)), (1, 12 << 16), "a wl_display.sync request");
        let id = word(2);
        for w in [id, 12 << 16, i as u32, 1, (12 << 16) | 1, id] {
            out.extend_from_slice(&w.to_ne_bytes());
        }
    }
    out
}

#[test]
fn qy54a2_drain_reads_without_dispatching_and_keeps_order() {
    let (conn, server) = pair();
    // A small send buffer so a client that does not read backs the "compositor" up quickly.
    rustix::net::sockopt::set_socket_send_buffer_size(&server, 4096).unwrap();
    let mut queue = conn.new_event_queue::<Got>();
    let n = 3000; // 72 KB of events: more than the shrunk socket buffer holds
    let events = sync_requests(&conn, &server, &queue.handle(), n);

    // Without the drain nobody reads: the server's writes back up.
    let stuck = write_until(&server, &events, Instant::now() + Duration::from_millis(300));
    assert!(stuck < events.len(), "without a reader the socket must fill ({stuck} of {} bytes went out)", events.len());

    // With the drain, everything is consumed although nothing is dispatched.
    let drain = WlDrain::start_for_test(&conn).expect("drain starts");
    let rest = write_until(&server, &events[stuck..], Instant::now() + Duration::from_secs(5));
    assert_eq!(stuck + rest, events.len(), "the drain keeps the socket readable");
    let fd = conn.backend().poll_fd().try_clone_to_owned().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while rustix::io::ioctl_fionread(&fd).unwrap() > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(rustix::io::ioctl_fionread(&fd).unwrap(), 0, "all bytes read off the client socket");
    assert!(!drain.is_finished(), "the drain keeps running");

    // One dispatch on the owner's queue delivers every callback, in request order.
    let mut got = Got::default();
    queue.dispatch_pending(&mut got).unwrap();
    assert_eq!(got.0, (0..n).collect::<Vec<_>>());
    drop(drain);
}

#[test]
fn qy54a2_stop_is_prompt_and_pollhup_ends_the_thread() {
    let (conn, server) = pair();
    let mut drain = WlDrain::start_for_test(&conn).expect("drain starts");
    std::thread::sleep(Duration::from_millis(50)); // idle in poll
    let t0 = Instant::now();
    drain.stop();
    assert!(t0.elapsed() < Duration::from_millis(100), "stop took {:?}", t0.elapsed());
    assert!(drain.is_finished());

    // The compositor goes away: the thread notices POLLHUP and exits by itself, once.
    let drain = WlDrain::start_for_test(&conn).expect("drain restarts");
    drop(server);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !drain.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(drain.is_finished(), "the thread exits on POLLHUP");
    assert!(!drain.stopped_by_panic());
}

#[test]
fn qy54a2_panic_on_owner_thread_stops_drain() {
    let (conn, _server) = pair();
    std::thread::spawn(move || {
        let drain = WlDrain::start_for_test(&conn).expect("drain starts");
        let caught = std::panic::catch_unwind(|| panic!("qy54a2 test panic (expected)"));
        assert!(caught.is_err());
        assert!(drain.stopped_by_panic(), "the chained hook stopped the owner's drain");
        assert!(drain.is_finished(), "and the thread is gone before the unwind");
    })
    .join()
    .unwrap();
}

#[test]
fn qy54a2_no_thread_off_wayland_or_when_disabled() {
    let wl = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(NonNull::dangling()));
    let x11 = RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0));
    assert!(wanted(x11, None).is_none(), "X11: no thread");
    assert!(wanted(wl, Some("0")).is_none(), "SCRATCHY_WL_DRAIN=0 disables");
    assert!(wanted(wl, None).is_some(), "Wayland: on by default");
    assert!(wanted(wl, Some("1")).is_some());
}
