//! QM1f: the real media backend, libmpv by `dlopen("libmpv.so.2")` (cargo feature
//! `media`, off by default). Layer 2 of QM1f; the element algorithms live in
//! `media_el`. FFI shapes and the option list come from the QM1b spike
//! (tools/mpvspike); bytes only ever come through `media_net` (D4): mpv is
//! given `scratchy-media://1` and our `stream_cb` reader, never a real URL.
//!
//! QM1g: mpv runs with `vo=libmpv` and a SOFTWARE render context (`sw`, format
//! `bgr0` = the page buffer's 0RGB). The render-update callback fires on an mpv
//! thread and only flags + posts a user event (`media_video::wake`); the frame is
//! rendered on the main thread by `take_frame`, at the visible box's size. With no
//! visible box the video track is switched off (`vid=no`) once metadata is known
//! (audio keeps playing) and any frame mpv still queues is consumed with
//! SKIP_RENDERING. Audio: `ao=null` unless `SCRATCHY_MEDIA_AO` names an output
//! (e.g. `pulse`); an element only opens it while it plays. Seeks use
//! `absolute+exact` (QM1b: plain seeks land on keyframes).

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::media_el::{BackendCtx, BackendEvent, LoadError, LoadReq, MediaBackend, MEDIA_ERR_DECODE, MEDIA_ERR_NETWORK, MEDIA_ERR_SRC_NOT_SUPPORTED};
use crate::media_net::{CancelHandle, MediaNetError, MediaSource, MediaStream, MPV_OPTIONS};
use crate::net_policy::RequestPolicy;
use std::sync::Arc;

use crate::css::ObjectFit;
use crate::media_video::{VideoFrame, VideoTarget};

#[repr(C)]
struct RenderParam {
    ty: c_int,
    data: *mut c_void,
}
const RP_API_TYPE: c_int = 1;
const RP_SKIP_RENDERING: c_int = 13;
const RP_SW_SIZE: c_int = 17;
const RP_SW_FORMAT: c_int = 18;
const RP_SW_STRIDE: c_int = 19;
const RP_SW_POINTER: c_int = 20;
const RENDER_UPDATE_FRAME: u64 = 1;

/// Set by mpv's render-update callback (any thread).
struct Shared {
    ready: AtomicBool,
}
unsafe extern "C" fn rc_update_cb(d: *mut c_void) {
    // SAFETY: `d` is the `Shared` leaked by `ensure`, freed after the render context.
    unsafe { (*(d as *const Shared)).ready.store(true, Ordering::SeqCst) };
    crate::media_video::wake();
}

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
struct StreamCbInfo {
    cookie: *mut c_void,
    read_fn: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, u64) -> i64>,
    seek_fn: Option<unsafe extern "C" fn(*mut c_void, i64) -> i64>,
    size_fn: Option<unsafe extern "C" fn(*mut c_void) -> i64>,
    close_fn: Option<unsafe extern "C" fn(*mut c_void)>,
    cancel_fn: Option<unsafe extern "C" fn(*mut c_void)>,
}
type OpenFn = unsafe extern "C" fn(*mut c_void, *mut c_char, *mut StreamCbInfo) -> c_int;

const EV_NONE: c_int = 0;
const EV_END_FILE: c_int = 7;
const EV_FILE_LOADED: c_int = 8;
const EV_PLAYBACK_RESTART: c_int = 21;
const EV_PROPERTY: c_int = 22;
const FMT_FLAG: c_int = 3;
const FMT_INT64: c_int = 4;
const FMT_DOUBLE: c_int = 5;
const END_EOF: c_int = 0;
const END_ERROR: c_int = 4;
const MPV_LOADING_FAILED: c_int = -13;
const MPV_UNSUPPORTED: c_int = -18;

const P_TIME: u64 = 1;
const P_DURATION: u64 = 2;
const P_CACHE: u64 = 3;
const P_EOF: u64 = 4;
const P_SEEKABLE: u64 = 5;
const P_BUFFERING: u64 = 6;

struct Lib {
    _lib: libloading::Library,
    create: unsafe extern "C" fn() -> *mut c_void,
    set_opt: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    init: unsafe extern "C" fn(*mut c_void) -> c_int,
    wait_event: unsafe extern "C" fn(*mut c_void, f64) -> *mut MpvEvent,
    command: unsafe extern "C" fn(*mut c_void, *const *const c_char) -> c_int,
    get_prop: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int,
    set_prop_str: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    observe: unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int,
    stream_add: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void, OpenFn) -> c_int,
    destroy: unsafe extern "C" fn(*mut c_void),
    rc_create: unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut RenderParam) -> c_int,
    rc_cb: unsafe extern "C" fn(*mut c_void, unsafe extern "C" fn(*mut c_void), *mut c_void),
    rc_update: unsafe extern "C" fn(*mut c_void) -> u64,
    rc_render: unsafe extern "C" fn(*mut c_void, *mut RenderParam) -> c_int,
    rc_free: unsafe extern "C" fn(*mut c_void),
}

/// Reinterpret a resolved symbol as the typed fn pointer the target field expects (same as
/// `transmute`, which clippy wants fully annotated; here `T` is inferred from the field).
///
/// # Safety
/// `T` must be the `unsafe extern "C" fn` type matching the symbol's real C signature.
unsafe fn cast_sym<T: Copy>(f: unsafe extern "C" fn()) -> T {
    unsafe { std::mem::transmute_copy(&f) }
}

fn load_lib() -> Option<Lib> {
    unsafe {
        let lib = libloading::Library::new("libmpv.so.2").ok()?;
        macro_rules! sym {
            ($n:literal) => {
                cast_sym(*lib.get::<unsafe extern "C" fn()>(concat!($n, "\0").as_bytes()).ok()?)
            };
        }
        Some(Lib {
            create: sym!("mpv_create"),
            set_opt: sym!("mpv_set_option_string"),
            init: sym!("mpv_initialize"),
            wait_event: sym!("mpv_wait_event"),
            command: sym!("mpv_command"),
            get_prop: sym!("mpv_get_property"),
            set_prop_str: sym!("mpv_set_property_string"),
            observe: sym!("mpv_observe_property"),
            stream_add: sym!("mpv_stream_cb_add_ro"),
            destroy: sym!("mpv_terminate_destroy"),
            rc_create: sym!("mpv_render_context_create"),
            rc_cb: sym!("mpv_render_context_set_update_callback"),
            rc_update: sym!("mpv_render_context_update"),
            rc_render: sym!("mpv_render_context_render"),
            rc_free: sym!("mpv_render_context_free"),
            _lib: lib,
        })
    }
}

fn lib() -> Option<&'static Lib> {
    static LIB: OnceLock<Option<Lib>> = OnceLock::new();
    LIB.get_or_init(load_lib).as_ref()
}

/// True when libmpv could be loaded (tests skip when it cannot).
pub(crate) fn libmpv_available() -> bool {
    lib().is_some()
}

/// What `open_fn` (mpv's opener thread) needs; freed after mpv is destroyed.
struct Reg {
    socket: Option<std::path::PathBuf>,
    policy: RequestPolicy,
    src: Mutex<Option<MediaSource>>,
    err: Mutex<Option<MediaNetError>>,
    cancel: Mutex<Option<CancelHandle>>,
    opened: AtomicBool,
}

struct Cookie {
    stream: MediaStream,
    cancel: CancelHandle,
    reg: *const Reg,
}

unsafe extern "C" fn st_open(ud: *mut c_void, uri: *mut c_char, info: *mut StreamCbInfo) -> c_int {
    unsafe {
        let reg = &*(ud as *const Reg);
        let uri = CStr::from_ptr(uri).to_string_lossy();
        if !uri.starts_with("scratchy-media://") {
            return MPV_LOADING_FAILED; // D4: mpv never opens anything else through us
        }
        let src = reg.src.lock().unwrap().clone();
        let (Some(src), Some(socket)) = (src, reg.socket.clone()) else { return MPV_LOADING_FAILED };
        match MediaStream::open(socket, src, reg.policy.clone()) {
            Ok(s) => {
                let cancel = s.cancel_handle();
                *reg.cancel.lock().unwrap() = Some(cancel.clone());
                reg.opened.store(true, Ordering::SeqCst);
                let ck = Box::new(Cookie { stream: s, cancel, reg: reg as *const Reg });
                *info = StreamCbInfo {
                    cookie: Box::into_raw(ck) as *mut c_void,
                    read_fn: Some(st_read),
                    seek_fn: Some(st_seek),
                    size_fn: Some(st_size),
                    close_fn: Some(st_close),
                    cancel_fn: Some(st_cancel),
                };
                0
            }
            Err(e) => {
                *reg.err.lock().unwrap() = Some(e);
                MPV_LOADING_FAILED
            }
        }
    }
}
unsafe extern "C" fn st_read(c: *mut c_void, buf: *mut c_char, n: u64) -> i64 {
    unsafe {
        let ck = &mut *(c as *mut Cookie);
        let out = std::slice::from_raw_parts_mut(buf as *mut u8, n as usize);
        match ck.stream.read(out) {
            Ok(k) => k as i64,
            Err(e) => {
                if e != MediaNetError::Cancelled {
                    *(*ck.reg).err.lock().unwrap() = Some(e);
                }
                -1
            }
        }
    }
}
unsafe extern "C" fn st_seek(c: *mut c_void, off: i64) -> i64 {
    unsafe {
        let ck = &mut *(c as *mut Cookie);
        if off < 0 {
            return MPV_UNSUPPORTED as i64;
        }
        match ck.stream.seek(off as u64) {
            Ok(p) => p as i64,
            Err(_) => -1,
        }
    }
}
unsafe extern "C" fn st_size(c: *mut c_void) -> i64 {
    unsafe {
        match (*(c as *mut Cookie)).stream.size() {
            Some(s) => s as i64,
            None => MPV_UNSUPPORTED as i64,
        }
    }
}
unsafe extern "C" fn st_close(c: *mut c_void) {
    unsafe {
        drop(Box::from_raw(c as *mut Cookie));
    }
}
unsafe extern "C" fn st_cancel(c: *mut c_void) {
    unsafe {
        // Only the `cancel` field: the reader thread holds the `stream` field.
        (*std::ptr::addr_of!((*(c as *mut Cookie)).cancel)).cancel();
    }
}

pub(crate) struct MpvBackend {
    ctx: BackendCtx,
    h: *mut c_void,
    reg: *mut Reg,
    loaded_file: bool,
    meta_sent: bool,
    cache_end: f64,
    seekable: bool,
    duration: f64,
    rc: *mut c_void,
    shared: *const Shared,
    target: Option<VideoTarget>,
    vid_off: bool,
}

impl MpvBackend {
    pub(crate) fn new(ctx: BackendCtx) -> Self {
        MpvBackend { ctx, h: std::ptr::null_mut(), reg: std::ptr::null_mut(), loaded_file: false, meta_sent: false, cache_end: 0.0, seekable: false, duration: f64::NAN, rc: std::ptr::null_mut(), shared: std::ptr::null(), target: None, vid_off: false }
    }

    fn cs(s: &str) -> CString {
        CString::new(s.replace('\0', "")).unwrap_or_default()
    }

    fn ensure(&mut self) -> Result<(), LoadError> {
        if !self.h.is_null() {
            return Ok(());
        }
        let l = lib().ok_or(LoadError::Unavailable)?;
        unsafe {
            let h = (l.create)();
            if h.is_null() {
                return Err(LoadError::Unavailable);
            }
            let ao = std::env::var("SCRATCHY_MEDIA_AO").unwrap_or_else(|_| if cfg!(test) { "null" } else { "pulse,null" }.into());
            for (k, v) in MPV_OPTIONS.iter().filter(|(k, _)| *k != "vo" && *k != "ao") {
                (l.set_opt)(h, Self::cs(k).as_ptr(), Self::cs(v).as_ptr());
            }
            for (k, v) in [("vo", "libmpv"), ("ao", ao.as_str()), ("pause", "yes"), ("idle", "yes")] {
                (l.set_opt)(h, Self::cs(k).as_ptr(), Self::cs(v).as_ptr());
            }
            if (l.init)(h) < 0 {
                (l.destroy)(h);
                return Err(LoadError::Unavailable);
            }
            let reg = Box::into_raw(Box::new(Reg {
                socket: self.ctx.socket.clone(),
                policy: self.ctx.policy.clone(),
                src: Mutex::new(None),
                err: Mutex::new(None),
                cancel: Mutex::new(None),
                opened: AtomicBool::new(false),
            }));
            (l.stream_add)(h, Self::cs("scratchy-media").as_ptr(), reg as *mut c_void, st_open);
            // QM1g: the software render context (frames are rendered by `take_frame`).
            let mut rc: *mut c_void = std::ptr::null_mut();
            let sw = Self::cs("sw");
            let mut p = [RenderParam { ty: RP_API_TYPE, data: sw.as_ptr() as *mut c_void }, RenderParam { ty: 0, data: std::ptr::null_mut() }];
            if (l.rc_create)(&mut rc, h, p.as_mut_ptr()) < 0 {
                drop(Box::from_raw(reg));
                (l.destroy)(h);
                return Err(LoadError::Unavailable);
            }
            let shared = Arc::into_raw(Arc::new(Shared { ready: AtomicBool::new(false) }));
            (l.rc_cb)(rc, rc_update_cb, shared as *mut c_void);
            self.rc = rc;
            self.shared = shared;
            for (id, name, fmt) in [
                (P_TIME, "time-pos", FMT_DOUBLE),
                (P_DURATION, "duration", FMT_DOUBLE),
                (P_CACHE, "demuxer-cache-time", FMT_DOUBLE),
                (P_EOF, "eof-reached", FMT_FLAG),
                (P_SEEKABLE, "seekable", FMT_FLAG),
                (P_BUFFERING, "paused-for-cache", FMT_FLAG),
            ] {
                (l.observe)(h, id, Self::cs(name).as_ptr(), fmt);
            }
            self.h = h;
            self.reg = reg;
        }
        Ok(())
    }

    fn cmd(&self, args: &[&str]) {
        let Some(l) = lib() else { return };
        let cs: Vec<CString> = args.iter().map(|a| Self::cs(a)).collect();
        let mut p: Vec<*const c_char> = cs.iter().map(|c| c.as_ptr()).collect();
        p.push(std::ptr::null());
        unsafe { (l.command)(self.h, p.as_ptr()) };
    }

    fn set(&self, name: &str, val: &str) {
        if let (Some(l), false) = (lib(), self.h.is_null()) {
            unsafe { (l.set_prop_str)(self.h, Self::cs(name).as_ptr(), Self::cs(val).as_ptr()) };
        }
    }

    fn get_f64(&self, name: &str) -> Option<f64> {
        let l = lib()?;
        let mut v = 0f64;
        let r = unsafe { (l.get_prop)(self.h, Self::cs(name).as_ptr(), FMT_DOUBLE, &mut v as *mut f64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    fn get_i64(&self, name: &str) -> Option<i64> {
        let l = lib()?;
        let mut v = 0i64;
        let r = unsafe { (l.get_prop)(self.h, Self::cs(name).as_ptr(), FMT_INT64, &mut v as *mut i64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    /// Applies the target (video on/off and how the picture fits the box).
    fn apply_target(&mut self) {
        if self.h.is_null() {
            return;
        }
        match self.target {
            Some(t) => {
                let (keep, pan, unscaled) = match t.fit {
                    ObjectFit::Fill => ("no", "0", "no"),
                    ObjectFit::Cover => ("yes", "1", "no"),
                    ObjectFit::None => ("yes", "0", "yes"),
                    ObjectFit::ScaleDown => ("yes", "0", "downscale-big"),
                    ObjectFit::Contain => ("yes", "0", "no"),
                };
                self.set("keepaspect", keep);
                self.set("panscan", pan);
                self.set("video-unscaled", unscaled);
                if self.vid_off {
                    self.set("vid", "auto");
                    self.vid_off = false;
                }
            }
            None => {
                // Only once the file is open (the size is read from the video) and only
                // when there is audio to keep playing: a video-only file with `vid=no`
                // would end at once.
                if self.meta_sent && !self.vid_off && self.get_i64("current-tracks/audio/id").is_some() {
                    self.set("vid", "no");
                    self.vid_off = true;
                }
            }
        }
    }

    fn reg(&self) -> Option<&Reg> {
        if self.reg.is_null() { None } else { Some(unsafe { &*self.reg }) }
    }

    fn cancel_streams(&self) {
        if let Some(c) = self.reg().and_then(|r| r.cancel.lock().unwrap().clone()) {
            c.cancel();
        }
    }

    fn error_code(&self, mpv_err: c_int) -> (u16, String) {
        let net = self.reg().and_then(|r| r.err.lock().unwrap().take());
        match net {
            Some(MediaNetError::Network(m)) => (MEDIA_ERR_NETWORK, m),
            Some(e) if self.meta_sent => (MEDIA_ERR_NETWORK, e.to_string()),
            Some(e) => (MEDIA_ERR_SRC_NOT_SUPPORTED, e.to_string()),
            None if self.meta_sent => (MEDIA_ERR_DECODE, format!("decode error ({mpv_err})")),
            None => (MEDIA_ERR_SRC_NOT_SUPPORTED, format!("could not open the media ({mpv_err})")),
        }
    }
}

impl MediaBackend for MpvBackend {
    fn name(&self) -> &'static str {
        "mpv"
    }
    fn available(&self) -> bool {
        libmpv_available()
    }
    fn load(&mut self, req: &LoadReq) -> Result<(), LoadError> {
        self.ensure()?;
        let Ok(url) = url::Url::parse(&req.url) else { return Err(LoadError::Other("invalid media URL".into())) };
        let Ok(page) = url::Url::parse(&req.page) else { return Err(LoadError::Other("invalid page URL".into())) };
        self.cancel_streams();
        if let Some(r) = self.reg() {
            *r.src.lock().unwrap() = Some(MediaSource { url, page, mode: req.mode });
            *r.err.lock().unwrap() = None;
        }
        self.loaded_file = true;
        self.meta_sent = false;
        self.cache_end = 0.0;
        self.seekable = false;
        self.duration = f64::NAN;
        self.set("pause", "yes");
        if self.vid_off {
            self.set("vid", "auto");
            self.vid_off = false;
        }
        self.cmd(&["loadfile", "scratchy-media://1", "replace"]);
        Ok(())
    }
    fn unload(&mut self) {
        if self.h.is_null() {
            return;
        }
        self.cancel_streams();
        self.cmd(&["stop"]);
        self.loaded_file = false;
        self.meta_sent = false;
    }
    fn set_paused(&mut self, p: bool) {
        self.set("pause", if p { "yes" } else { "no" });
    }
    fn seek(&mut self, secs: f64) {
        self.cmd(&["seek", &format!("{secs}"), "absolute+exact"]);
    }
    fn set_volume(&mut self, v: f64) {
        self.set("volume", &format!("{}", v * 100.0));
    }
    fn set_muted(&mut self, m: bool) {
        self.set("mute", if m { "yes" } else { "no" });
    }
    fn set_rate(&mut self, r: f64) {
        self.set("speed", &format!("{r}"));
    }
    fn poll(&mut self, wait: Duration) -> Vec<BackendEvent> {
        let mut out = Vec::new();
        let Some(l) = lib() else { return out };
        if self.h.is_null() {
            return out;
        }
        // Nobody wants the picture: still consume mpv's frames (vo=libmpv stalls
        // playback, audio included, when they pile up).
        if self.target.is_none() {
            self.take_frame();
        }
        let mut w = wait.as_secs_f64();
        loop {
            let ev = unsafe { &*(l.wait_event)(self.h, w) };
            w = 0.0;
            match ev.id {
                EV_NONE => break,
                EV_FILE_LOADED => {
                    let duration = self.get_f64("duration").unwrap_or(f64::NAN);
                    let width = self.get_i64("width").unwrap_or(0).max(0) as u32;
                    let height = self.get_i64("height").unwrap_or(0).max(0) as u32;
                    self.duration = duration;
                    self.meta_sent = true;
                    self.vid_off = false;
                    self.apply_target();
                    out.push(BackendEvent::Loaded { duration, width, height });
                }
                EV_PLAYBACK_RESTART => out.push(BackendEvent::Restart),
                EV_END_FILE => {
                    let e = unsafe { &*(ev.data as *const MpvEventEndFile) };
                    if e.reason == END_ERROR {
                        let (c, m) = self.error_code(e.error);
                        out.push(BackendEvent::Error(c, m));
                    } else if e.reason == END_EOF {
                        out.push(BackendEvent::Eof);
                    }
                }
                EV_PROPERTY => {
                    let p = unsafe { &*(ev.data as *const MpvEventProperty) };
                    if p.data.is_null() {
                        continue;
                    }
                    let (f, flag) = unsafe {
                        if p.format == FMT_DOUBLE { (*(p.data as *const f64), false) } else { (0.0, *(p.data as *const c_int) != 0) }
                    };
                    match ev.userdata {
                        P_TIME if p.format == FMT_DOUBLE && self.meta_sent => out.push(BackendEvent::Time(f)),
                        P_DURATION if p.format == FMT_DOUBLE && self.meta_sent => {
                            self.duration = f;
                            out.push(BackendEvent::DurationChanged(f));
                        }
                        P_CACHE if p.format == FMT_DOUBLE => self.cache_end = f,
                        P_SEEKABLE if p.format == FMT_FLAG => self.seekable = flag,
                        P_EOF if p.format == FMT_FLAG && flag && self.meta_sent => out.push(BackendEvent::Eof),
                        P_BUFFERING if p.format == FMT_FLAG && self.meta_sent => out.push(BackendEvent::Buffering(flag)),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        out
    }
    fn set_video_target(&mut self, t: Option<VideoTarget>) {
        self.target = t;
        self.apply_target();
    }
    fn frame_pending(&self) -> bool {
        !self.shared.is_null() && unsafe { (*self.shared).ready.load(Ordering::SeqCst) }
    }
    fn take_frame(&mut self) -> Option<Arc<VideoFrame>> {
        let l = lib()?;
        if self.rc.is_null() || self.shared.is_null() || !unsafe { (*self.shared).ready.swap(false, Ordering::SeqCst) } {
            return None;
        }
        unsafe {
            if (l.rc_update)(self.rc) & RENDER_UPDATE_FRAME == 0 {
                return None;
            }
            let Some(t) = self.target.filter(|t| t.w > 0 && t.h > 0 && t.w <= 8192 && t.h <= 8192) else {
                // Nobody looks at it: consume the frame without rendering.
                let mut one: c_int = 1;
                let mut p = [RenderParam { ty: RP_SKIP_RENDERING, data: &mut one as *mut c_int as *mut c_void }, RenderParam { ty: 0, data: std::ptr::null_mut() }];
                (l.rc_render)(self.rc, p.as_mut_ptr());
                return None;
            };
            let mut px = vec![0u32; t.w as usize * t.h as usize];
            let mut size = [t.w as c_int, t.h as c_int];
            let fmt = Self::cs("bgr0");
            let mut stride: usize = t.w as usize * 4;
            let mut p = [
                RenderParam { ty: RP_SW_SIZE, data: size.as_mut_ptr() as *mut c_void },
                RenderParam { ty: RP_SW_FORMAT, data: fmt.as_ptr() as *mut c_void },
                RenderParam { ty: RP_SW_STRIDE, data: &mut stride as *mut usize as *mut c_void },
                RenderParam { ty: RP_SW_POINTER, data: px.as_mut_ptr() as *mut c_void },
                RenderParam { ty: 0, data: std::ptr::null_mut() },
            ];
            if (l.rc_render)(self.rc, p.as_mut_ptr()) < 0 {
                return None;
            }
            // `bgr0` leaves the pad byte 0; keep the top byte clear (the page sentinel is 0xFE).
            for v in px.iter_mut() {
                *v &= 0x00FF_FFFF;
            }
            Some(Arc::new(VideoFrame { w: t.w, h: t.h, px }))
        }
    }
    fn buffered(&self) -> Vec<(f64, f64)> {
        if self.cache_end > 0.0 { vec![(0.0, self.cache_end)] } else { Vec::new() }
    }
    fn seekable(&self) -> Vec<(f64, f64)> {
        if self.seekable && self.duration.is_finite() { vec![(0.0, self.duration)] } else { Vec::new() }
    }
    fn active(&self) -> bool {
        self.loaded_file
    }
}

impl Drop for MpvBackend {
    fn drop(&mut self) {
        if self.h.is_null() {
            return;
        }
        self.cancel_streams();
        if let Some(l) = lib() {
            unsafe {
                // The render context must go before the core.
                if !self.rc.is_null() {
                    (l.rc_free)(self.rc);
                }
                (l.destroy)(self.h);
            }
        }
        if !self.shared.is_null() {
            unsafe { drop(Arc::from_raw(self.shared)) };
        }
        if !self.reg.is_null() {
            unsafe { drop(Box::from_raw(self.reg)) };
        }
    }
}
