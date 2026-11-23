// Test-only input injector for a NESTED headless compositor (copied from scratchy/tools/inject; adds hold:/release:).
// Usage: inject W H cmd...
//   move:X,Y  click  rclick  mclick  hold:c|s|cs  release  down  up  scroll:DY  hscroll:DX  sscroll:DY (Shift held)  wait:MS
//   key:NAME (j k g G h l esc ret tab space bs colon slash ins ...) with optional c- / s- prefix
//   ev:CODE evs:CODE (raw evdev code, evs with Shift)  group:N (layout group)
//   mods:BITS (modifier state alone)  kdown:CODE kup:CODE (a key held across commands)
//   type:TEXT (letters, digits, space, . : / - _ = ?)
// Or: inject output W H SCALE   (C6b: the output becomes W x H pixels at SCALE; src/output.rs)
mod output;

use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};
use wayland_client::{
    protocol::{wl_keyboard, wl_pointer, wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

#[derive(Default)]
struct St {
    seat: Option<wl_seat::WlSeat>,
    vpm: Option<ZwlrVirtualPointerManagerV1>,
    vkm: Option<ZwpVirtualKeyboardManagerV1>,
    /// C-KEYMAP: the desktop's own keymap text (from a wl_keyboard of ours), so the virtual
    /// keyboard has the same keymap and the seat's keymap does not flip to us while injecting.
    desktop_keymap: Option<Vec<u8>>,
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for St {
    fn event(
        s: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        e: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Keymap { fd, size, .. } = e {
            use std::os::unix::fs::FileExt;
            let f = std::fs::File::from(fd);
            let mut buf = vec![0u8; size as usize];
            if f.read_exact_at(&mut buf, 0).is_ok() {
                if let Some(end) = buf.iter().position(|b| *b == 0) {
                    buf.truncate(end);
                }
                s.desktop_keymap = Some(buf);
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for St {
    fn event(
        s: &mut Self,
        r: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = e
        {
            match interface.as_str() {
                "wl_seat" => s.seat = Some(r.bind(name, version.min(7), qh, ())),
                "zwlr_virtual_pointer_manager_v1" => {
                    s.vpm = Some(r.bind(name, version.min(2), qh, ()))
                }
                "zwp_virtual_keyboard_manager_v1" => s.vkm = Some(r.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}
macro_rules! noop { ($($t:ty),*) => { $(impl Dispatch<$t, ()> for St { fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {} })* } }
noop!(
    wl_seat::WlSeat,
    ZwlrVirtualPointerManagerV1,
    ZwlrVirtualPointerV1,
    ZwpVirtualKeyboardManagerV1,
    ZwpVirtualKeyboardV1
);

fn code(name: &str) -> Option<(u32, bool)> {
    // evdev codes; bool = needs shift
    let letters = "qwertyuiop";
    let l2 = "asdfghjkl";
    let l3 = "zxcvbnm";
    if name.len() == 1 {
        let c = name.chars().next().unwrap();
        let lc = c.to_ascii_lowercase();
        let shift = c.is_ascii_uppercase();
        if let Some(i) = letters.find(lc) {
            return Some((16 + i as u32, shift));
        }
        if let Some(i) = l2.find(lc) {
            return Some((30 + i as u32, shift));
        }
        if let Some(i) = l3.find(lc) {
            return Some((44 + i as u32, shift));
        }
        if let Some(d) = c.to_digit(10) {
            return Some((if d == 0 { 11 } else { 1 + d }, false));
        }
        return match c {
            ' ' => Some((57, false)),
            '.' => Some((52, false)),
            ':' => Some((39, true)),
            ';' => Some((39, false)),
            '/' => Some((53, false)),
            '-' => Some((12, false)),
            '_' => Some((12, true)),
            '=' => Some((13, false)),
            '?' => Some((53, true)),
            '&' => Some((8, true)),
            '#' => Some((4, true)),
            '%' => Some((6, true)),
            _ => None,
        };
    }
    Some(match name {
        "esc" => (1, false),
        "ret" => (28, false),
        "tab" => (15, false),
        "space" => (57, false),
        "bs" => (14, false),
        "colon" => (39, true),
        "slash" => (53, false),
        "up" => (103, false),
        "down" => (108, false),
        "left" => (105, false),
        "right" => (106, false),
        "pgdn" => (109, false),
        "pgup" => (104, false),
        "home" => (102, false),
        "end" => (107, false),
        "del" => (111, false),
        "ins" => (110, false),
        _ => return None,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("output") {
        return output::set(
            args[2].parse().unwrap(),
            args[3].parse().unwrap(),
            args[4].parse().unwrap(),
        );
    }
    let w: u32 = args[1].parse().unwrap();
    let h: u32 = args[2].parse().unwrap();
    let conn = Connection::connect_to_env().unwrap();
    let mut q = conn.new_event_queue();
    let qh = q.handle();
    conn.display().get_registry(&qh, ());
    let mut st = St::default();
    q.roundtrip(&mut st).unwrap();
    q.roundtrip(&mut st).unwrap();
    let seat = st.seat.clone();
    let vp = st
        .vpm
        .as_ref()
        .expect("no virtual pointer")
        .create_virtual_pointer(seat.as_ref(), &qh, ());
    let vk = st
        .vkm
        .as_ref()
        .expect("no virtual keyboard")
        .create_virtual_keyboard(seat.as_ref().unwrap(), &qh, ());
    let _kb = seat.as_ref().map(|sh| sh.get_keyboard(&qh, ()));
    q.roundtrip(&mut st).unwrap();
    let builtin: &[u8] = include_bytes!("../keymap.xkb");
    let km: &[u8] = st.desktop_keymap.as_deref().unwrap_or(builtin);
    let path = std::env::temp_dir().join(format!("inject-km-{}", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(km).unwrap();
    f.write_all(&[0]).unwrap();
    drop(f);
    let f = std::fs::File::open(&path).unwrap();
    vk.keymap(1, f.as_fd(), km.len() as u32 + 1);
    let _ = std::fs::remove_file(&path);
    q.roundtrip(&mut st).unwrap();
    let t0 = Instant::now();
    let ms = || t0.elapsed().as_millis() as u32;
    let mut q_rt = |st: &mut St| {
        conn.flush().unwrap();
        q.roundtrip(st).unwrap();
    };
    let group = std::cell::Cell::new(0u32);
    let key = |code: u32, shift: bool, ctrl: bool| {
        let mut mods = 0;
        if shift {
            mods |= 1;
        }
        if ctrl {
            mods |= 4;
        }
        vk.modifiers(mods, 0, 0, group.get());
        vk.key(ms(), code, 1);
        vk.key(ms(), code, 0);
        vk.modifiers(0, 0, 0, group.get());
    };
    // NEST-HEADLESS: motions carry time 0. dwl's motionabsolute() warps the cursor only
    // when the event time is 0 ("this is 0 with virtual pointers"); with any other time
    // motionnotify() picks the surface and position under the cursor BEFORE moving it, so
    // the client got the previous position and a `move` then `click` clicked where the
    // pointer had been (C-NOTAB's ctrl+click landed on the page, not the link).
    let mut last_move: Option<(u32, u32)> = None;
    let mut held = String::new();
    // Nested dwl delivers a pointer motion to the client only when the next
    // motion arrives (the last `move` of a run, or before a `wait`, was
    // otherwise lost: QB3b). Repeating the position once pushes it out.
    let flush_move = |m: &mut Option<(u32, u32)>| {
        if let Some((x, y)) = m.take() {
            vp.motion_absolute(0, x, y, w, h);
            vp.frame();
        }
    };
    for cmd in &args[3..] {
        let (op, arg) = cmd.split_once(':').unwrap_or((cmd.as_str(), ""));
        match op {
            "move" => {
                let (x, y) = arg.split_once(',').unwrap();
                vp.motion_absolute(0, x.parse().unwrap(), y.parse().unwrap(), w, h);
                vp.frame();
                last_move = Some((x.parse::<u32>().unwrap(), y.parse::<u32>().unwrap()));
            }
            "click" | "rclick" | "mclick" => {
                let b = match op {
                    "click" => 0x110,
                    "rclick" => 0x111,
                    _ => 0x112,
                };
                vp.button(ms(), b, wl_pointer::ButtonState::Pressed);
                vp.frame();
                q_rt(&mut st);
                std::thread::sleep(Duration::from_millis(40));
                vp.button(ms(), b, wl_pointer::ButtonState::Released);
                vp.frame();
            }
            // QSB1: press and release the left button separately, for drags.
            "down" | "up" => {
                flush_move(&mut last_move);
                let state = if op == "down" {
                    wl_pointer::ButtonState::Pressed
                } else {
                    wl_pointer::ButtonState::Released
                };
                vp.button(ms(), 0x110, state);
                vp.frame();
                q_rt(&mut st);
            }
            "scroll" => {
                let dy: f64 = arg.parse().unwrap();
                vp.axis_source(wl_pointer::AxisSource::Wheel);
                vp.axis_discrete(ms(), wl_pointer::Axis::VerticalScroll, dy * 15.0, dy as i32);
                vp.frame();
            }
            // QXSCR1: a horizontal wheel (positive = right), and a vertical wheel with Shift held.
            "hscroll" => {
                let dx: f64 = arg.parse().unwrap();
                vp.axis_source(wl_pointer::AxisSource::Wheel);
                vp.axis_discrete(
                    ms(),
                    wl_pointer::Axis::HorizontalScroll,
                    dx * 15.0,
                    dx as i32,
                );
                vp.frame();
            }
            "sscroll" => {
                let dy: f64 = arg.parse().unwrap();
                vk.modifiers(1, 0, 0, 0);
                q_rt(&mut st);
                vp.axis_source(wl_pointer::AxisSource::Wheel);
                vp.axis_discrete(ms(), wl_pointer::Axis::VerticalScroll, dy * 15.0, dy as i32);
                vp.frame();
                q_rt(&mut st);
                vk.modifiers(0, 0, 0, 0);
            }
            // C-NOTAB: hold:c, hold:s or hold:cs presses Ctrl and/or Shift (real key events,
            // so the page and the host see them) until `release`; click, mclick etc. in
            // between are modified clicks.
            "hold" => {
                let mut mods = 0;
                if arg.contains('c') {
                    vk.key(ms(), 29, 1);
                    mods |= 4;
                }
                if arg.contains('s') {
                    vk.key(ms(), 42, 1);
                    mods |= 1;
                }
                vk.modifiers(mods, 0, 0, 0);
                held = arg.to_string();
                q_rt(&mut st);
            }
            "release" => {
                vk.modifiers(0, 0, 0, 0);
                if held.contains('c') {
                    vk.key(ms(), 29, 0);
                }
                if held.contains('s') {
                    vk.key(ms(), 42, 0);
                }
                held.clear();
                q_rt(&mut st);
            }
            "wait" => {
                flush_move(&mut last_move);
                q_rt(&mut st);
                std::thread::sleep(Duration::from_millis(arg.parse().unwrap()));
            }
            "key" => {
                let (mut ctrl, mut sh, mut name) = (false, false, arg);
                if let Some(r) = name.strip_prefix("c-") {
                    ctrl = true;
                    name = r;
                }
                // QWPT7: s- holds Shift (Shift+Tab, Shift+arrows); may follow c-.
                if let Some(r) = name.strip_prefix("s-") {
                    sh = true;
                    name = r;
                }
                let (c, shift) = code(name).unwrap_or_else(|| panic!("unknown key {name}"));
                key(c, shift || sh, ctrl);
            }
            // C-KEYMAP: a raw evdev key code (the physical key, whatever the layout makes of it):
            // ev:21 presses it, evs:52 with Shift. group:N sets the virtual keyboard's active layout group.
            "ev" | "evs" => {
                key(arg.parse().unwrap(), op == "evs", false);
            }
            // C-KEYMAP: chord:56,42 presses the keys in order and releases them in reverse, as bare
            // key events (no modifiers request): Alt+Shift, which toggles a keymap's layout group.
            "chord" => {
                let codes: Vec<u32> = arg.split(',').map(|c| c.parse().unwrap()).collect();
                for c in &codes {
                    vk.key(ms(), *c, 1);
                    q_rt(&mut st);
                    std::thread::sleep(Duration::from_millis(30));
                }
                for c in codes.iter().rev() {
                    vk.key(ms(), *c, 0);
                    q_rt(&mut st);
                    std::thread::sleep(Duration::from_millis(30));
                }
            }
            // C-FOCUSKEYS / C3-SHIFTSYNC: mods:BITS sets the modifier state alone (1 Shift, 4 Ctrl,
            // 8 Alt; no key event, as a desktop does when a modifier was already held); kdown:CODE
            // and kup:CODE are bare key events (the key stays down until kup, also across `wait:`).
            "mods" => {
                vk.modifiers(arg.parse().unwrap(), 0, 0, group.get());
            }
            "kdown" | "kup" => {
                vk.key(
                    ms(),
                    arg.parse().unwrap(),
                    if op == "kdown" { 1 } else { 0 },
                );
            }
            "group" => {
                group.set(arg.parse().unwrap());
                vk.modifiers(0, 0, 0, group.get());
            }
            "type" => {
                for ch in arg.chars() {
                    let s = ch.to_string();
                    let (c, shift) = code(&s).unwrap_or_else(|| panic!("untypeable {ch}"));
                    key(c, shift, false);
                    q_rt(&mut st);
                    std::thread::sleep(Duration::from_millis(15));
                }
            }
            _ => panic!("bad cmd {cmd}"),
        }
        q_rt(&mut st);
    }
    flush_move(&mut last_move);
    q_rt(&mut st);
    std::thread::sleep(Duration::from_millis(100));
}
