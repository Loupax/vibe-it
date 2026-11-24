//! C6e: `inject ime`: a scripted test input method (zwp_input_method_v2) for the nested compositor.
//!
//! The smallest IME there is: no keyboard grab, no popup. It prints what the compositor tells it,
//! one line each on stdout (flushed): `bound`, `activate`, `deactivate`, `surrounding <cursor>
//! <anchor> <text>`, `content_type <hint> <purpose>`, `cause <n>`, `done <serial>`, `unavailable`.
//! It reads commands from stdin, one per line, and sends them at once:
//!   preedit <cursor_begin> <cursor_end> <text>   set_preedit_string + commit
//!   commit <text>                                commit_string + commit (preedit cleared)
//!   delete <before> <after>                      delete_surrounding_text + commit
//!   compose <before> <after> <preedit>|<commit>  all three in one commit (an empty side is left out)
//!   nudge                                        an empty commit (the client answers with its state)
//!   quit
//! Like fcitx5, it answers an `activate` with an empty commit once the `done` has come: the client
//! then commits its pending state (Chromium sends its cursor rectangle that way).
//! Lengths and cursors are bytes (UTF-8), as the protocol says. The commit serial is the number
//! of `done` events seen so far.
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::{self, ZwpInputMethodV2},
};

struct Ime {
    dones: Arc<AtomicU32>,
    /// An `activate` came; answer at its `done`.
    activated: bool,
}

fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Ime {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Ime {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpInputMethodManagerV2, ()> for Ime {
    fn event(
        _: &mut Self,
        _: &ZwpInputMethodManagerV2,
        _: <ZwpInputMethodManagerV2 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for Ime {
    fn event(
        s: &mut Self,
        im: &ZwpInputMethodV2,
        e: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_input_method_v2::Event as E;
        match e {
            E::Activate => {
                s.activated = true;
                say("activate");
            }
            E::Deactivate => say("deactivate"),
            E::SurroundingText {
                text,
                cursor,
                anchor,
            } => say(&format!("surrounding {cursor} {anchor} {text}")),
            E::TextChangeCause { cause } => say(&format!("cause {}", u32::from(cause))),
            E::ContentType { hint, purpose } => say(&format!(
                "content_type {} {}",
                u32::from(hint),
                u32::from(purpose)
            )),
            E::Done => {
                let n = s.dones.fetch_add(1, Ordering::SeqCst) + 1;
                say(&format!("done {n}"));
                if std::mem::take(&mut s.activated) {
                    im.commit(n);
                    say(&format!("answered activate serial {n}"));
                }
            }
            E::Unavailable => {
                say("unavailable");
                std::process::exit(2);
            }
            _ => {}
        }
    }
}

/// One stdin line to requests on `im`; false for `quit`.
fn run_command(im: &ZwpInputMethodV2, line: &str, serial: u32) -> bool {
    let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
    match cmd {
        "preedit" => {
            let mut p = rest.splitn(3, ' ');
            let b: i32 = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let e: i32 = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            im.set_preedit_string(p.next().unwrap_or("").to_string(), b, e);
        }
        "commit" => im.commit_string(rest.to_string()),
        "delete" => {
            let mut p = rest.split(' ');
            let b: u32 = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let a: u32 = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            im.delete_surrounding_text(b, a);
        }
        "compose" => {
            let mut p = rest.splitn(3, ' ');
            let b: u32 = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let a: u32 = p.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let (pre, com) = p.next().unwrap_or("").split_once('|').unwrap_or(("", ""));
            if b > 0 || a > 0 {
                im.delete_surrounding_text(b, a);
            }
            if !pre.is_empty() {
                im.set_preedit_string(pre.to_string(), pre.len() as i32, pre.len() as i32);
            }
            if !com.is_empty() {
                im.commit_string(com.to_string());
            }
        }
        "nudge" => {}
        "quit" => return false,
        "" => return true,
        other => {
            say(&format!("error unknown command {other}"));
            return true;
        }
    }
    im.commit(serial);
    say(&format!("sent {cmd} serial {serial}"));
    true
}

pub fn run() {
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY");
    let (globals, mut queue) = registry_queue_init::<Ime>(&conn).expect("registry");
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("wl_seat");
    let mgr: ZwpInputMethodManagerV2 = globals
        .bind(&qh, 1..=1, ())
        .expect("the compositor has no zwp_input_method_manager_v2");
    let im = mgr.get_input_method(&seat, &qh, ());
    let dones = Arc::new(AtomicU32::new(0));
    let mut st = Ime {
        dones: dones.clone(),
        activated: false,
    };
    queue.roundtrip(&mut st).expect("roundtrip");
    say("bound");
    let conn2 = conn.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if !run_command(&im, line.trim_end(), dones.load(Ordering::SeqCst)) {
                break;
            }
            let _ = conn2.flush();
        }
        std::process::exit(0);
    });
    loop {
        if queue.blocking_dispatch(&mut st).is_err() {
            say("error dispatch");
            std::process::exit(1);
        }
    }
}
