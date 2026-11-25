//! QM1b spike: libmpv software render into a rect of a winit+softbuffer page.
//! Usage: mpvspike <local-file | http://127.0.0.1:PORT/name.mp4>
//! An http:// source is NOT given to mpv: mpv only sees scratchy-media://1 and
//! its bytes come from 1 MiB ranged GETs made by our own reader (stream_cb).
//! Keys: space pause, l/h seek +-5 s, s seek to 12.5 (test hook), j/k scroll
//! the rect up/down, left/right move it, wheel scrolls. MPVSPIKE_AO overrides
//! ao (default null). libmpv is dlopen()ed: without it the page stays white.
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const VW: usize = 640;
const VH: usize = 360;
const CHUNK: u64 = 1 << 20;

#[derive(Debug)]
enum Ev {
    Mpv,
    Frame,
}

// ---------------------------------------------------------------- libmpv FFI
#[repr(C)]
struct MpvEvent {
    id: c_int,
    error: c_int,
    userdata: u64,
    data: *mut c_void,
}
#[repr(C)]
struct MpvEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}
#[repr(C)]
struct MpvEventEndFile {
    reason: c_int,
    error: c_int,
}
#[repr(C)]
struct RenderParam {
    ty: c_int,
    data: *mut c_void,
}
#[repr(C)]
struct StreamCbInfo {
    cookie: *mut c_void,
    read_fn: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, u64) -> i64>,
    seek_fn: Option<unsafe extern "C" fn(*mut c_void, i64) -> i64>,
    size_fn: Option<unsafe extern "C" fn(*mut c_void) -> i64>,
    close_fn: Option<unsafe extern "C" fn(*mut c_void)>,
    cancel_fn: Option<unsafe extern "C" fn(*mut c_void)>,
}
type OpenFn = unsafe extern "C" fn(*mut c_void, *mut c_char, *mut StreamCbInfo) -> c_int;

const EV_END_FILE: c_int = 7;
const EV_FILE_LOADED: c_int = 8;
const EV_PLAYBACK_RESTART: c_int = 21;
const EV_PROPERTY: c_int = 22;
const FMT_FLAG: c_int = 3;
const FMT_DOUBLE: c_int = 5;

struct Mpv {
    _lib: libloading::Library,
    h: *mut c_void,
    rc: *mut c_void,
    wait_event: unsafe extern "C" fn(*mut c_void, f64) -> *mut MpvEvent,
    command: unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int,
    get_prop: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int,
    rc_update: unsafe extern "C" fn(*mut c_void) -> u64,
    rc_render: unsafe extern "C" fn(*mut c_void, *mut RenderParam) -> c_int,
}

macro_rules! sym {
    ($lib:expr, $name:literal) => {
        *$lib.get::<unsafe extern "C" fn()>(concat!($name, "\0").as_bytes()).ok()?
    };
}

unsafe extern "C" fn wakeup(d: *mut c_void) {
    let p = &*(d as *const EventLoopProxy<Ev>);
    let _ = p.send_event(Ev::Mpv);
}
unsafe extern "C" fn frame_cb(d: *mut c_void) {
    let p = &*(d as *const EventLoopProxy<Ev>);
    let _ = p.send_event(Ev::Frame);
}

impl Mpv {
    fn new(proxy: &EventLoopProxy<Ev>, src: &str) -> Option<Mpv> {
        unsafe {
            let lib = libloading::Library::new("libmpv.so.2").ok()?;
            let create: unsafe extern "C" fn() -> *mut c_void = std::mem::transmute(sym!(lib, "mpv_create"));
            let set_opt: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int =
                std::mem::transmute(sym!(lib, "mpv_set_option_string"));
            let init: unsafe extern "C" fn(*mut c_void) -> c_int = std::mem::transmute(sym!(lib, "mpv_initialize"));
            let wait_event = std::mem::transmute(sym!(lib, "mpv_wait_event"));
            let command = std::mem::transmute(sym!(lib, "mpv_command"));
            let get_prop = std::mem::transmute(sym!(lib, "mpv_get_property"));
            let observe: unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int =
                std::mem::transmute(sym!(lib, "mpv_observe_property"));
            let set_wakeup: unsafe extern "C" fn(*mut c_void, unsafe extern "C" fn(*mut c_void), *mut c_void) =
                std::mem::transmute(sym!(lib, "mpv_set_wakeup_callback"));
            let rc_create: unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut RenderParam) -> c_int =
                std::mem::transmute(sym!(lib, "mpv_render_context_create"));
            let rc_cb: unsafe extern "C" fn(*mut c_void, unsafe extern "C" fn(*mut c_void), *mut c_void) =
                std::mem::transmute(sym!(lib, "mpv_render_context_set_update_callback"));
            let rc_update = std::mem::transmute(sym!(lib, "mpv_render_context_update"));
            let rc_render = std::mem::transmute(sym!(lib, "mpv_render_context_render"));
            let stream_add: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void, OpenFn) -> c_int =
                std::mem::transmute(sym!(lib, "mpv_stream_cb_add_ro"));

            let h = create();
            if h.is_null() {
                return None;
            }
            let ao = std::env::var("MPVSPIKE_AO").unwrap_or_else(|_| "null".into());
            // Disable everything of mpv's that could touch the outside world.
            let opts: Vec<(&str, String)> = vec![
                ("config", "no".into()),
                ("ytdl", "no".into()),
                ("load-scripts", "no".into()),
                ("osc", "no".into()),
                ("input-default-bindings", "no".into()),
                ("input-vo-keyboard", "no".into()),
                ("input-terminal", "no".into()),
                ("terminal", "no".into()),
                ("load-unsafe-playlists", "no".into()),
                ("load-auto-profiles", "no".into()),
                ("load-stats-overlay", "no".into()),
                ("load-console", "no".into()),
                ("load-select", "no".into()),
                ("audio-display", "no".into()),
                ("cover-art-auto", "no".into()),
                ("sub-auto", "no".into()),
                ("audio-file-auto", "no".into()),
                ("save-position-on-quit", "no".into()),
                ("watch-later-directory", "/nonexistent".into()),
                ("vo", "libmpv".into()),
                ("ao", ao),
                ("keep-open", "yes".into()),
                ("loop-file", "inf".into()),
                ("demuxer-max-bytes", "32MiB".into()),
                ("demuxer-max-back-bytes", "16MiB".into()),
            ];
            for (k, v) in &opts {
                let (k, v) = (CString::new(*k).unwrap(), CString::new(v.as_str()).unwrap());
                let r = set_opt(h, k.as_ptr(), v.as_ptr());
                if r < 0 {
                    eprintln!("mpv: option {:?}={:?} -> {}", k, v, r);
                }
            }
            if init(h) < 0 {
                eprintln!("mpv_initialize failed");
                return None;
            }
            let pp: *mut EventLoopProxy<Ev> = Box::into_raw(Box::new(proxy.clone()));
            set_wakeup(h, wakeup, pp as *mut c_void);
            let name = CString::new("time-pos").unwrap();
            observe(h, 1, name.as_ptr(), FMT_DOUBLE);
            let name = CString::new("pause").unwrap();
            observe(h, 2, name.as_ptr(), FMT_FLAG);

            let mut rc: *mut c_void = std::ptr::null_mut();
            let sw = CString::new("sw").unwrap();
            let mut params = [
                RenderParam { ty: 1, data: sw.as_ptr() as *mut c_void },
                RenderParam { ty: 0, data: std::ptr::null_mut() },
            ];
            let r = rc_create(&mut rc, h, params.as_mut_ptr());
            if r < 0 {
                eprintln!("mpv_render_context_create failed {}", r);
                return None;
            }
            rc_cb(rc, frame_cb, pp as *mut c_void);

            let url = if src.starts_with("http://") {
                let proto = CString::new("scratchy-media").unwrap();
                let r = stream_add(h, proto.as_ptr(), std::ptr::null_mut(), stream_open);
                eprintln!("stream_cb_add_ro -> {}", r);
                SRC.set(src.to_string()).ok();
                "scratchy-media://1".to_string()
            } else {
                src.to_string()
            };
            let m = Mpv { _lib: lib, h, rc, wait_event, command, get_prop, rc_update, rc_render };
            m.cmd(&["loadfile", &url]);
            Some(m)
        }
    }

    fn cmd(&self, args: &[&str]) {
        let cs: Vec<CString> = args.iter().map(|a| CString::new(*a).unwrap()).collect();
        let mut p: Vec<*const c_char> = cs.iter().map(|c| c.as_ptr()).collect();
        p.push(std::ptr::null());
        let r = unsafe { (self.command)(self.h, p.as_ptr()) };
        if r < 0 {
            eprintln!("mpv command {:?} -> {}", args, r);
        }
    }

    fn time_pos(&self) -> Option<f64> {
        let mut v = 0f64;
        let n = CString::new("time-pos").unwrap();
        let r = unsafe { (self.get_prop)(self.h, n.as_ptr(), FMT_DOUBLE, &mut v as *mut f64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    /// Render the pending frame (if any) into `buf` (VW x VH, bgr0). True if drawn.
    fn render(&self, buf: &mut [u32]) -> bool {
        unsafe {
            if (self.rc_update)(self.rc) & 1 == 0 {
                return false;
            }
            let mut size = [VW as c_int, VH as c_int];
            let fmt = CString::new("bgr0").unwrap();
            let mut stride: usize = VW * 4;
            let mut p = [
                RenderParam { ty: 17, data: size.as_mut_ptr() as *mut c_void },
                RenderParam { ty: 18, data: fmt.as_ptr() as *mut c_void },
                RenderParam { ty: 19, data: &mut stride as *mut usize as *mut c_void },
                RenderParam { ty: 20, data: buf.as_mut_ptr() as *mut c_void },
                RenderParam { ty: 0, data: std::ptr::null_mut() },
            ];
            (self.rc_render)(self.rc, p.as_mut_ptr()) >= 0
        }
    }
}

// ------------------------------------------------- stream_cb ranged-GET reader
static SRC: OnceLock<String> = OnceLock::new();

struct Stream {
    addr: String,
    path: String,
    pos: u64,
    total: u64,
    chunks: Vec<(u64, Vec<u8>)>, // most recent last, max 4
    requests: u32,
}

fn range_get(addr: &str, path: &str, lo: u64, hi: u64) -> Option<(u64, Vec<u8>)> {
    let t = Instant::now();
    let mut s = TcpStream::connect(addr).ok()?;
    write!(s, "GET {path} HTTP/1.1\r\nHost: {addr}\r\nRange: bytes={lo}-{hi}\r\nConnection: close\r\n\r\n").ok()?;
    let mut all = Vec::new();
    s.read_to_end(&mut all).ok()?;
    let split = all.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = String::from_utf8_lossy(&all[..split]).to_string();
    if !head.starts_with("HTTP/1.1 206") && !head.starts_with("HTTP/1.0 206") {
        eprintln!("reader: not 206: {}", head.lines().next().unwrap_or(""));
        return None;
    }
    let total: u64 = head
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-range:"))?
        .rsplit('/')
        .next()?
        .trim()
        .parse()
        .ok()?;
    let body = all[split..].to_vec();
    eprintln!("reader: GET bytes={lo}-{hi} -> 206 {} bytes in {:?}", body.len(), t.elapsed());
    Some((total, body))
}

unsafe extern "C" fn stream_open(_ud: *mut c_void, uri: *mut c_char, info: *mut StreamCbInfo) -> c_int {
    let uri = CStr::from_ptr(uri).to_string_lossy().to_string();
    eprintln!("stream_open {uri}");
    let src = SRC.get().unwrap(); // http://host:port/path
    let rest = &src["http://".len()..];
    let (addr, path) = rest.split_once('/').unwrap();
    let path = format!("/{path}");
    let Some((total, body)) = range_get(addr, &path, 0, CHUNK - 1) else { return -13 };
    let st = Box::new(Stream { addr: addr.into(), path, pos: 0, total, chunks: vec![(0, body)], requests: 1 });
    *info = StreamCbInfo {
        cookie: Box::into_raw(st) as *mut c_void,
        read_fn: Some(stream_read),
        seek_fn: Some(stream_seek),
        size_fn: Some(stream_size),
        close_fn: Some(stream_close),
        cancel_fn: None,
    };
    0
}
unsafe extern "C" fn stream_read(c: *mut c_void, buf: *mut c_char, n: u64) -> i64 {
    let st = &mut *(c as *mut Stream);
    if st.pos >= st.total {
        return 0;
    }
    let hit = st.chunks.iter().position(|(s, b)| st.pos >= *s && st.pos < *s + b.len() as u64);
    let i = match hit {
        Some(i) => i,
        None => {
            let hi = (st.pos + CHUNK - 1).min(st.total - 1);
            let Some((_, body)) = range_get(&st.addr, &st.path, st.pos, hi) else { return -1 };
            st.requests += 1;
            if st.chunks.len() >= 4 {
                st.chunks.remove(0);
            }
            st.chunks.push((st.pos, body));
            st.chunks.len() - 1
        }
    };
    let (s, b) = &st.chunks[i];
    let off = (st.pos - s) as usize;
    let k = (b.len() - off).min(n as usize);
    std::ptr::copy_nonoverlapping(b[off..].as_ptr(), buf as *mut u8, k);
    st.pos += k as u64;
    k as i64
}
unsafe extern "C" fn stream_seek(c: *mut c_void, off: i64) -> i64 {
    let st = &mut *(c as *mut Stream);
    if off < 0 || off as u64 > st.total {
        return -18; // MPV_ERROR_UNSUPPORTED
    }
    st.pos = off as u64;
    off
}
unsafe extern "C" fn stream_size(c: *mut c_void) -> i64 {
    (*(c as *mut Stream)).total as i64
}
unsafe extern "C" fn stream_close(c: *mut c_void) {
    let st = Box::from_raw(c as *mut Stream);
    eprintln!("stream_close: {} ranged GETs total", st.requests);
}

// ------------------------------------------------------------------- the app
struct App {
    mpv: Option<Mpv>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    vid: Vec<u32>,
    have_frame: bool,
    sx: i32,
    sy: i32,
    last_pos_log: Instant,
    frames: u32,
    last_fps: Instant,
}

impl App {
    fn redraw(&mut self) {
        let (Some(w), Some(s)) = (&self.window, &mut self.surface) else { return };
        let sz = w.inner_size();
        let (Some(nw), Some(nh)) = (NonZeroU32::new(sz.width), NonZeroU32::new(sz.height)) else { return };
        s.resize(nw, nh).unwrap();
        let mut b = s.buffer_mut().unwrap();
        b.fill(0x00ff_ffff);
        let (ww, wh) = (sz.width as i32, sz.height as i32);
        let (rx, ry) = (100 - self.sx, 100 - self.sy);
        if self.have_frame {
            let (x0, x1) = (rx.max(0), (rx + VW as i32).min(ww));
            let (y0, y1) = (ry.max(0), (ry + VH as i32).min(wh));
            if x0 < x1 && y0 < y1 {
                for y in y0..y1 {
                    let src = ((y - ry) as usize) * VW + (x0 - rx) as usize;
                    let dst = (y as usize) * ww as usize + x0 as usize;
                    let n = (x1 - x0) as usize;
                    b[dst..dst + n].copy_from_slice(&self.vid[src..src + n]);
                }
            }
        }
        b.present().unwrap();
        self.frames += 1;
        if self.last_fps.elapsed() >= Duration::from_secs(1) {
            eprintln!("fps {} rect=({rx},{ry}) win={ww}x{wh}", self.frames);
            self.frames = 0;
            self.last_fps = Instant::now();
        }
    }

    fn drain_mpv(&mut self) {
        let Some(m) = &self.mpv else { return };
        loop {
            let ev = unsafe { &*(m.wait_event)(m.h, 0.0) };
            match ev.id {
                0 => break,
                EV_FILE_LOADED => eprintln!("event: file-loaded"),
                EV_PLAYBACK_RESTART => eprintln!("event: playback-restart time-pos={:?}", m.time_pos()),
                EV_END_FILE => {
                    let e = unsafe { &*(ev.data as *const MpvEventEndFile) };
                    eprintln!("event: end-file reason={} error={}", e.reason, e.error);
                }
                EV_PROPERTY => {
                    let p = unsafe { &*(ev.data as *const MpvEventProperty) };
                    let name = unsafe { CStr::from_ptr(p.name) }.to_string_lossy();
                    if name == "time-pos" && p.format == FMT_DOUBLE {
                        if self.last_pos_log.elapsed() >= Duration::from_secs(1) {
                            self.last_pos_log = Instant::now();
                            eprintln!("time-pos {:.2}", unsafe { *(p.data as *const f64) });
                        }
                    } else if name == "pause" && p.format == FMT_FLAG {
                        eprintln!("pause {}", unsafe { *(p.data as *const c_int) });
                    }
                }
                _ => {}
            }
        }
    }
}

impl ApplicationHandler<Ev> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let w = Rc::new(el.create_window(Window::default_attributes().with_title("mpvspike").with_inner_size(LogicalSize::new(1000, 800))).unwrap());
        let ctx = softbuffer::Context::new(w.clone()).unwrap();
        self.surface = Some(softbuffer::Surface::new(&ctx, w.clone()).unwrap());
        self.window = Some(w);
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, ev: Ev) {
        match ev {
            Ev::Mpv => self.drain_mpv(),
            Ev::Frame => {
                if let Some(m) = &self.mpv {
                    if m.render(&mut self.vid) {
                        self.have_frame = true;
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, e: WindowEvent) {
        match e {
            WindowEvent::CloseRequested => {
                std::process::exit(0); // skip mpv teardown: spike
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 40.0,
                    MouseScrollDelta::PixelDelta(p) => -p.y as f32,
                };
                self.sy = (self.sy + dy as i32).clamp(-2000, 2000);
                eprintln!("scroll_y={} rect_y={}", self.sy, 100 - self.sy);
                self.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
                let mut redraw = true;
                match &event.logical_key {
                    Key::Named(NamedKey::Space) => {
                        if let Some(m) = &self.mpv { m.cmd(&["cycle", "pause"]) }
                    }
                    Key::Named(NamedKey::ArrowLeft) => self.sx -= 40,
                    Key::Named(NamedKey::ArrowRight) => self.sx += 40,
                    Key::Character(c) => match c.as_str() {
                        "l" => if let Some(m) = &self.mpv { m.cmd(&["seek", "5", "relative+exact"]) },
                        "h" => if let Some(m) = &self.mpv { m.cmd(&["seek", "-5", "relative+exact"]) },
                        "s" => if let Some(m) = &self.mpv { m.cmd(&["seek", "12.5", "absolute+exact"]) },
                        "j" => self.sy += 40,
                        "k" => self.sy -= 40,
                        "t" => if let Some(m) = &self.mpv { eprintln!("TIME-POS {:?}", m.time_pos()) },
                        "q" => el.exit(),
                        _ => redraw = false,
                    },
                    _ => redraw = false,
                }
                if redraw {
                    self.window.as_ref().unwrap().request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let src = std::env::args().nth(1).unwrap_or_else(|| "t720.mp4".into());
    let el = EventLoop::<Ev>::with_user_event().build().unwrap();
    let proxy = el.create_proxy();
    let mpv = Mpv::new(&proxy, &src);
    if mpv.is_none() {
        eprintln!("libmpv not available or failed to start: page stays white (optional dependency)");
    }
    let mut app = App {
        mpv,
        window: None,
        surface: None,
        vid: vec![0x00ff_ffff; VW * VH],
        have_frame: false,
        sx: 0,
        sy: 0,
        last_pos_log: Instant::now(),
        frames: 0,
        last_fps: Instant::now(),
    };
    el.run_app(&mut app).unwrap();
}
