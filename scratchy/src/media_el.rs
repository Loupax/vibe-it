//! QM1f: the HTMLMediaElement state machine, in pure Rust, over a `MediaBackend`.
//!
//! Layer 1 of QM1f (TODO.txt): `MediaCore` models the HTML media element
//! algorithms (resource selection from `src`/`<source>`, networkState and
//! readyState transitions, play()/pause(), seeking, ended, volume, rate, the
//! event order, MediaError codes, the D5 autoplay rule). It knows nothing of JS or
//! mpv: it is driven by calls from the element wrapper (`js_media`) and by
//! `BackendEvent`s polled from a `MediaBackend`. Everything it wants the page to
//! see comes out as `Out` items (events to fire, play() promises to settle).
//! Tests drive it with `FakeBackend` (deterministic, no clock of its own).
//!
//! The real backend is `media_mpv` (cargo feature `media`, libmpv by dlopen).
//! Without it `UnavailableBackend` keeps the inert placeholder behaviour.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use crate::media_net::MediaMode;
use crate::net_policy::RequestPolicy;

pub(crate) const NETWORK_EMPTY: u8 = 0;
pub(crate) const NETWORK_IDLE: u8 = 1;
pub(crate) const NETWORK_LOADING: u8 = 2;
pub(crate) const NETWORK_NO_SOURCE: u8 = 3;
pub(crate) const HAVE_NOTHING: u8 = 0;
pub(crate) const HAVE_METADATA: u8 = 1;
pub(crate) const HAVE_CURRENT_DATA: u8 = 2;
pub(crate) const HAVE_FUTURE_DATA: u8 = 3;
pub(crate) const HAVE_ENOUGH_DATA: u8 = 4;

pub(crate) const MEDIA_ERR_ABORTED: u16 = 1;
pub(crate) const MEDIA_ERR_NETWORK: u16 = 2;
pub(crate) const MEDIA_ERR_DECODE: u16 = 3;
pub(crate) const MEDIA_ERR_SRC_NOT_SUPPORTED: u16 = 4;

/// `timeupdate` is fired at most this often while playing (~4 Hz).
const TIMEUPDATE_MS: f64 = 250.0;
/// D7: at most this many players play at once.
pub(crate) const MAX_PLAYERS: usize = 2;

// ------------------------------------------------------------ canPlayType

/// `canPlayType`: `maybe`/`probably` only for what ffmpeg here decodes
/// (mp4/h264/aac, webm/vp8/vp9/opus/vorbis, ogg, mp3, flac, wav); `` otherwise.
/// The caller returns `` for everything when no backend is available.
pub(crate) fn can_play_type(t: &str) -> &'static str {
    let t = t.trim();
    let (mime, params) = match t.split_once(';') {
        Some((m, p)) => (m.trim().to_ascii_lowercase(), p.trim()),
        None => (t.to_ascii_lowercase(), ""),
    };
    // codecs known per container family
    let allowed: &[&str] = match mime.as_str() {
        "video/mp4" | "audio/mp4" | "audio/x-m4a" | "audio/aac" | "video/x-m4v" => &["avc1", "avc3", "mp4a.40", "mp4a.69", "mp4a.6b", "mp3", "flac", "opus", "vp09", "vp9"],
        "video/webm" | "audio/webm" => &["vp8", "vp8.0", "vp9", "vp09", "opus", "vorbis"],
        "video/ogg" | "audio/ogg" | "application/ogg" => &["theora", "vorbis", "opus", "flac"],
        "audio/mpeg" | "audio/mp3" => &["mp3"],
        "audio/flac" | "audio/x-flac" => &["flac"],
        "audio/wav" | "audio/wave" | "audio/x-wav" => &["1"],
        _ => return "",
    };
    let Some(codecs) = codecs_param(params) else { return "maybe" };
    if codecs.is_empty() {
        return "maybe";
    }
    for c in codecs {
        let c = c.trim().to_ascii_lowercase();
        if c.is_empty() || !allowed.iter().any(|a| c == *a || c.starts_with(&format!("{a}."))) {
            return "";
        }
    }
    "probably"
}

fn codecs_param(params: &str) -> Option<Vec<String>> {
    for p in params.split(';') {
        let p = p.trim();
        let (k, v) = p.split_once('=')?;
        if k.trim().eq_ignore_ascii_case("codecs") {
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            return Some(v.split(',').map(|s| s.to_string()).collect());
        }
    }
    None
}

// ------------------------------------------------------------ backend

#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub url: String,
    /// The `type` attribute of a `<source>` ('' = none).
    pub mime: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadReq {
    pub url: String,
    pub page: String,
    pub mode: MediaMode,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LoadError {
    /// No backend at all (libmpv missing, or built without `media`).
    Unavailable,
    Other(String),
}

/// What a backend tells the element, in the order it happened.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BackendEvent {
    /// Metadata known (mpv file-loaded): duration (NaN = unknown), video size.
    Loaded { duration: f64, width: u32, height: u32 },
    DurationChanged(f64),
    /// First frame/sample decoded, or a seek finished (mpv playback-restart).
    Restart,
    /// Playback position in seconds (mpv time-pos).
    Time(f64),
    /// The end of the media was reached (mpv end-file, reason eof).
    Eof,
    /// Playback stalled (true) or resumed (false) for lack of data.
    Buffering(bool),
    /// A MediaError code and a message.
    Error(u16, String),
}

pub(crate) trait MediaBackend {
    /// "mpv", "fake", "none".
    fn name(&self) -> &'static str;
    /// False when playing is impossible here (the placeholder path).
    fn available(&self) -> bool;
    /// Starts loading; always paused. Events follow through `poll`.
    fn load(&mut self, req: &LoadReq) -> Result<(), LoadError>;
    fn unload(&mut self);
    fn set_paused(&mut self, paused: bool);
    fn seek(&mut self, secs: f64);
    fn set_volume(&mut self, v: f64);
    fn set_muted(&mut self, m: bool);
    fn set_rate(&mut self, r: f64);
    /// Pending events; may wait up to `wait` for the first one.
    fn poll(&mut self, wait: Duration) -> Vec<BackendEvent>;
    fn buffered(&self) -> Vec<(f64, f64)>;
    fn seekable(&self) -> Vec<(f64, f64)>;
    /// True while the backend has something loaded and the host should poll it.
    fn active(&self) -> bool;
}

/// Context a backend factory gets (the wire socket and the page's policy).
#[derive(Clone)]
pub(crate) struct BackendCtx {
    pub socket: Option<PathBuf>,
    pub policy: RequestPolicy,
}

/// The placeholder: nothing plays, no network.
pub(crate) struct UnavailableBackend;

impl MediaBackend for UnavailableBackend {
    fn name(&self) -> &'static str {
        "none"
    }
    fn available(&self) -> bool {
        false
    }
    fn load(&mut self, _: &LoadReq) -> Result<(), LoadError> {
        Err(LoadError::Unavailable)
    }
    fn unload(&mut self) {}
    fn set_paused(&mut self, _: bool) {}
    fn seek(&mut self, _: f64) {}
    fn set_volume(&mut self, _: f64) {}
    fn set_muted(&mut self, _: bool) {}
    fn set_rate(&mut self, _: f64) {}
    fn poll(&mut self, _: Duration) -> Vec<BackendEvent> {
        Vec::new()
    }
    fn buffered(&self) -> Vec<(f64, f64)> {
        Vec::new()
    }
    fn seekable(&self) -> Vec<(f64, f64)> {
        Vec::new()
    }
    fn active(&self) -> bool {
        false
    }
}

// ------------------------------------------------------------ core

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Out {
    /// Fire this event at the media element.
    Fire(&'static str),
    /// Fire `error` at the `<source>` candidate with this index.
    SourceError(usize),
    /// Resolve / reject the play() promise with this token (DOMException name).
    Resolve(u32),
    Reject(u32, &'static str),
    /// Host-only: the placeholder path was hit (log once).
    Unavailable,
    /// Host-only: the backend accepted a load (log `media: mpv` once).
    BackendStarted,
}

pub(crate) struct MediaCore {
    backend: Box<dyn MediaBackend>,
    outs: Vec<Out>,
    pub network_state: u8,
    pub ready_state: u8,
    pub paused: bool,
    pub seeking: bool,
    seek_target: f64,
    pub current_time: f64,
    pub duration: f64,
    pub volume: f64,
    pub muted: bool,
    pub rate: f64,
    pub default_rate: f64,
    pub loop_: bool,
    pub autoplay: bool,
    pub video_width: u32,
    pub video_height: u32,
    pub error: Option<(u16, String)>,
    cands: Vec<Candidate>,
    idx: usize,
    from_attr: bool,
    mode: MediaMode,
    page: String,
    pub current_src: String,
    pending: Vec<u32>,
    playing_fired: bool,
    last_tu: f64,
    default_start: Option<f64>,
    loaded: bool,
    announced: bool,
    /// D5: set by the host from the page's user activation before each call.
    pub activated: bool,
}

impl MediaCore {
    pub(crate) fn new(backend: Box<dyn MediaBackend>) -> Self {
        MediaCore {
            backend,
            outs: Vec::new(),
            network_state: NETWORK_EMPTY,
            ready_state: HAVE_NOTHING,
            paused: true,
            seeking: false,
            seek_target: 0.0,
            current_time: 0.0,
            duration: f64::NAN,
            volume: 1.0,
            muted: false,
            rate: 1.0,
            default_rate: 1.0,
            loop_: false,
            autoplay: false,
            video_width: 0,
            video_height: 0,
            error: None,
            cands: Vec::new(),
            idx: 0,
            from_attr: true,
            mode: MediaMode::NoCors,
            page: String::new(),
            current_src: String::new(),
            pending: Vec::new(),
            playing_fired: false,
            last_tu: f64::NEG_INFINITY,
            default_start: None,
            loaded: false,
            announced: false,
            activated: false,
        }
    }

    pub(crate) fn take_outs(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.outs)
    }
    pub(crate) fn available(&self) -> bool {
        self.backend.available()
    }
    pub(crate) fn backend_name(&self) -> &'static str {
        self.backend.name()
    }
    pub(crate) fn active(&self) -> bool {
        self.backend.active()
    }
    pub(crate) fn buffered(&self) -> Vec<(f64, f64)> {
        self.backend.buffered()
    }
    pub(crate) fn seekable(&self) -> Vec<(f64, f64)> {
        self.backend.seekable()
    }
    fn fire(&mut self, n: &'static str) {
        self.outs.push(Out::Fire(n));
    }
    fn reject_pending(&mut self, name: &'static str) {
        for t in std::mem::take(&mut self.pending) {
            self.outs.push(Out::Reject(t, name));
        }
    }

    pub(crate) fn ended(&self) -> bool {
        self.ready_state >= HAVE_METADATA && self.duration.is_finite() && self.current_time >= self.duration && !self.loop_
    }

    // ---- the load algorithm and resource selection ----

    /// "media element load algorithm" + resource selection. `from_attr`: the
    /// single candidate is the `src` attribute (failure = MediaError on the
    /// element); otherwise `<source>` children (failure = `error` on the
    /// source, next candidate, no MediaError).
    pub(crate) fn load(&mut self, cands: Vec<Candidate>, from_attr: bool, mode: MediaMode, page: String) {
        if self.network_state != NETWORK_EMPTY {
            self.backend.unload();
            if self.network_state == NETWORK_LOADING || self.network_state == NETWORK_IDLE {
                self.fire("abort");
            }
            self.fire("emptied");
            self.reject_pending("AbortError");
            self.ready_state = HAVE_NOTHING;
            self.paused = true;
            self.seeking = false;
            self.current_time = 0.0;
            self.duration = f64::NAN;
            self.video_width = 0;
            self.video_height = 0;
        }
        self.error = None;
        self.playing_fired = false;
        self.default_start = None;
        self.loaded = false;
        self.announced = false;
        self.network_state = NETWORK_EMPTY;
        self.current_src.clear();
        self.cands = cands;
        self.from_attr = from_attr;
        self.mode = mode;
        self.page = page;
        self.idx = 0;
        if self.cands.is_empty() {
            return;
        }
        self.network_state = NETWORK_NO_SOURCE;
        self.network_state = NETWORK_LOADING;
        self.fire("loadstart");
        self.start_candidate();
    }

    fn start_candidate(&mut self) {
        while self.idx < self.cands.len() {
            let c = self.cands[self.idx].clone();
            if !self.from_attr && !c.mime.is_empty() && can_play_type(&c.mime).is_empty() {
                self.outs.push(Out::SourceError(self.idx));
                self.idx += 1;
                continue;
            }
            let req = LoadReq { url: c.url.clone(), page: self.page.clone(), mode: self.mode };
            match self.backend.load(&req) {
                Ok(()) => {
                    self.current_src = c.url;
                    self.backend.set_volume(self.volume);
                    self.backend.set_muted(self.muted);
                    self.backend.set_rate(self.rate);
                    if !self.announced {
                        self.announced = true;
                        self.outs.push(Out::BackendStarted);
                    }
                    return;
                }
                Err(LoadError::Unavailable) => {
                    self.outs.push(Out::Unavailable);
                    self.candidate_failed(MEDIA_ERR_SRC_NOT_SUPPORTED, "libmpv not available");
                    return;
                }
                Err(LoadError::Other(m)) => {
                    self.candidate_failed(MEDIA_ERR_SRC_NOT_SUPPORTED, &m);
                    return;
                }
            }
        }
        self.network_state = NETWORK_NO_SOURCE;
    }

    fn candidate_failed(&mut self, code: u16, msg: &str) {
        if self.from_attr {
            self.fatal_error(code, msg);
        } else {
            self.outs.push(Out::SourceError(self.idx));
            self.idx += 1;
            self.backend.unload();
            self.start_candidate();
        }
    }

    fn fatal_error(&mut self, code: u16, msg: &str) {
        self.error = Some((code, msg.to_string()));
        self.network_state = if self.ready_state == HAVE_NOTHING { NETWORK_NO_SOURCE } else { NETWORK_IDLE };
        self.fire("error");
        self.reject_pending(if code == MEDIA_ERR_SRC_NOT_SUPPORTED { "NotSupportedError" } else { "AbortError" });
    }

    // ---- play / pause / seek ----

    pub(crate) fn play(&mut self, token: u32) {
        if !self.backend.available() {
            self.outs.push(Out::Unavailable);
            self.outs.push(Out::Reject(token, "NotSupportedError"));
            return;
        }
        if self.network_state == NETWORK_EMPTY || matches!(self.error, Some((MEDIA_ERR_SRC_NOT_SUPPORTED, _))) || self.network_state == NETWORK_NO_SOURCE {
            self.outs.push(Out::Reject(token, "NotSupportedError"));
            return;
        }
        if !self.can_autoplay() {
            self.outs.push(Out::Reject(token, "NotAllowedError"));
            return;
        }
        if self.ended() {
            self.set_current_time(0.0);
        }
        if self.paused {
            self.start_playing();
            self.pending.push(token);
            self.maybe_playing();
        } else if self.ready_state >= HAVE_FUTURE_DATA && self.playing_fired {
            self.outs.push(Out::Resolve(token));
        } else {
            self.pending.push(token);
        }
    }

    /// D5: no autoplay unless muted or after a user gesture.
    fn can_autoplay(&self) -> bool {
        self.muted || self.activated
    }

    fn start_playing(&mut self) {
        self.paused = false;
        self.backend.set_paused(false);
        self.fire("play");
        if self.ready_state <= HAVE_CURRENT_DATA {
            self.fire("waiting");
        }
    }

    fn maybe_playing(&mut self) {
        if !self.paused && self.ready_state >= HAVE_FUTURE_DATA && !self.playing_fired {
            self.playing_fired = true;
            self.fire("playing");
            for t in std::mem::take(&mut self.pending) {
                self.outs.push(Out::Resolve(t));
            }
        }
    }

    pub(crate) fn pause(&mut self) {
        if self.paused {
            return;
        }
        self.paused = true;
        self.playing_fired = false;
        self.backend.set_paused(true);
        self.fire("timeupdate");
        self.fire("pause");
        self.reject_pending("AbortError");
    }

    pub(crate) fn set_current_time(&mut self, t: f64) {
        if t.is_nan() {
            return;
        }
        if self.ready_state == HAVE_NOTHING {
            self.default_start = Some(t.max(0.0));
            self.current_time = t.max(0.0);
            return;
        }
        let mut t = t.max(0.0);
        if self.duration.is_finite() {
            t = t.min(self.duration);
        }
        self.seeking = true;
        self.seek_target = t;
        self.current_time = t;
        self.fire("seeking");
        self.backend.seek(t);
    }

    pub(crate) fn set_volume(&mut self, v: f64) -> Result<(), &'static str> {
        if !(0.0..=1.0).contains(&v) {
            return Err("IndexSizeError");
        }
        if v != self.volume {
            self.volume = v;
            self.backend.set_volume(v);
            self.fire("volumechange");
        }
        Ok(())
    }

    pub(crate) fn set_muted(&mut self, m: bool) {
        if m != self.muted {
            self.muted = m;
            self.backend.set_muted(m);
            self.fire("volumechange");
        }
    }

    pub(crate) fn set_rate(&mut self, r: f64) -> Result<(), &'static str> {
        if !(r >= 0.0625 && r <= 16.0) {
            return Err("NotSupportedError");
        }
        if r != self.rate {
            self.rate = r;
            self.backend.set_rate(r);
            self.fire("ratechange");
        }
        Ok(())
    }

    pub(crate) fn set_default_rate(&mut self, r: f64) {
        if r != self.default_rate {
            self.default_rate = r;
            self.fire("ratechange");
        }
    }

    // ---- backend events ----

    /// Polls the backend (waiting up to `wait` for the first event) and
    /// handles what came. `now` is the page clock in ms.
    pub(crate) fn pump(&mut self, now: f64, wait: Duration) {
        for e in self.backend.poll(wait) {
            self.handle(e, now);
        }
    }

    pub(crate) fn handle(&mut self, ev: BackendEvent, now: f64) {
        if self.network_state == NETWORK_EMPTY {
            return; // a late event of an aborted load
        }
        match ev {
            BackendEvent::Loaded { duration, width, height } => {
                self.loaded = true;
                self.duration = duration;
                self.video_width = width;
                self.video_height = height;
                self.fire("durationchange");
                self.ready_state = HAVE_METADATA;
                self.fire("loadedmetadata");
                if let Some(t) = self.default_start.take() {
                    self.set_current_time(t);
                }
            }
            BackendEvent::DurationChanged(d) => {
                if !(d == self.duration || (d.is_nan() && self.duration.is_nan())) {
                    self.duration = d;
                    self.fire("durationchange");
                }
            }
            BackendEvent::Restart => {
                if self.seeking {
                    self.seeking = false;
                    self.current_time = self.seek_target;
                    self.fire("timeupdate");
                    self.fire("seeked");
                    if !self.paused && self.ready_state >= HAVE_FUTURE_DATA {
                        self.playing_fired = false;
                        self.maybe_playing();
                    }
                }
                if self.ready_state == HAVE_METADATA {
                    self.ready_state = HAVE_CURRENT_DATA;
                    self.fire("loadeddata");
                    self.ready_state = HAVE_ENOUGH_DATA;
                    self.fire("canplay");
                    if self.paused && self.autoplay && self.can_autoplay() {
                        self.start_playing();
                    }
                    self.maybe_playing();
                    self.fire("canplaythrough");
                    self.network_state = NETWORK_IDLE;
                }
            }
            BackendEvent::Time(p) => {
                if !self.seeking {
                    self.current_time = p;
                    if !self.paused && now - self.last_tu >= TIMEUPDATE_MS {
                        self.last_tu = now;
                        self.fire("timeupdate");
                    }
                }
            }
            BackendEvent::Eof => {
                if self.duration.is_finite() {
                    self.current_time = self.duration;
                }
                if self.loop_ {
                    self.set_current_time(0.0);
                } else {
                    self.fire("timeupdate");
                    if !self.paused {
                        self.paused = true;
                        self.playing_fired = false;
                        self.fire("pause");
                    }
                    self.fire("ended");
                }
            }
            BackendEvent::Buffering(true) => {
                if !self.paused && self.ready_state >= HAVE_FUTURE_DATA {
                    self.ready_state = HAVE_CURRENT_DATA;
                    self.playing_fired = false;
                    self.fire("waiting");
                }
            }
            BackendEvent::Buffering(false) => {
                if self.ready_state == HAVE_CURRENT_DATA && self.loaded {
                    self.ready_state = HAVE_ENOUGH_DATA;
                    self.fire("canplay");
                    self.maybe_playing();
                }
            }
            BackendEvent::Error(code, msg) => {
                if self.loaded {
                    self.fatal_error(code, &msg);
                } else {
                    self.candidate_failed(code, &msg);
                }
            }
        }
    }
}

// ------------------------------------------------------------ the fake backend

/// Shared state of one `FakeBackend`, kept by the test to script it.
#[derive(Default)]
pub(crate) struct FakeState {
    pub calls: Vec<String>,
    pub queue: Vec<BackendEvent>,
    pub loaded: Option<LoadReq>,
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
    pub rate: f64,
    pub fail_load: Option<LoadError>,
    pub buffered: Vec<(f64, f64)>,
    pub seekable: Vec<(f64, f64)>,
    /// A seek answers with `Restart` right away (default true via `fake_new`).
    pub auto_restart: bool,
}

pub(crate) type FakeHandle = Rc<RefCell<FakeState>>;

pub(crate) fn fake_new() -> FakeHandle {
    Rc::new(RefCell::new(FakeState { auto_restart: true, rate: 1.0, duration: f64::NAN, ..Default::default() }))
}

/// Test script helpers on the shared state.
pub(crate) trait FakeScript {
    /// Metadata arrived (mpv file-loaded).
    fn meta(&self, duration: f64, w: u32, h: u32);
    /// First frame decoded (mpv playback-restart).
    fn first_frame(&self);
    /// Playback advances by `secs` (a Time event; Eof at the end).
    fn advance(&self, secs: f64);
    fn push(&self, e: BackendEvent);
}

impl FakeScript for FakeHandle {
    fn meta(&self, duration: f64, w: u32, h: u32) {
        let mut s = self.borrow_mut();
        s.duration = duration;
        s.queue.push(BackendEvent::Loaded { duration, width: w, height: h });
    }
    fn first_frame(&self) {
        self.borrow_mut().queue.push(BackendEvent::Restart);
    }
    fn advance(&self, secs: f64) {
        let mut s = self.borrow_mut();
        if !s.playing {
            return;
        }
        s.position = (s.position + secs * s.rate).min(s.duration);
        let p = s.position;
        s.queue.push(BackendEvent::Time(p));
        if p >= s.duration {
            s.playing = false;
            s.queue.push(BackendEvent::Eof);
        }
    }
    fn push(&self, e: BackendEvent) {
        self.borrow_mut().queue.push(e);
    }
}

pub(crate) struct FakeBackend(pub FakeHandle);

impl MediaBackend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn available(&self) -> bool {
        true
    }
    fn load(&mut self, req: &LoadReq) -> Result<(), LoadError> {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("load {}", req.url));
        if let Some(e) = s.fail_load.clone() {
            return Err(e);
        }
        s.loaded = Some(req.clone());
        s.position = 0.0;
        s.playing = false;
        Ok(())
    }
    fn unload(&mut self) {
        let mut s = self.0.borrow_mut();
        s.calls.push("unload".into());
        s.loaded = None;
        s.playing = false;
        s.queue.clear();
    }
    fn set_paused(&mut self, p: bool) {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("paused {p}"));
        s.playing = !p;
    }
    fn seek(&mut self, t: f64) {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("seek {t}"));
        s.position = t;
        if s.auto_restart {
            s.queue.push(BackendEvent::Restart);
        }
    }
    fn set_volume(&mut self, v: f64) {
        self.0.borrow_mut().calls.push(format!("volume {v}"));
    }
    fn set_muted(&mut self, m: bool) {
        self.0.borrow_mut().calls.push(format!("muted {m}"));
    }
    fn set_rate(&mut self, r: f64) {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("rate {r}"));
        s.rate = r;
    }
    fn poll(&mut self, _: Duration) -> Vec<BackendEvent> {
        std::mem::take(&mut self.0.borrow_mut().queue)
    }
    fn buffered(&self) -> Vec<(f64, f64)> {
        self.0.borrow().buffered.clone()
    }
    fn seekable(&self) -> Vec<(f64, f64)> {
        self.0.borrow().seekable.clone()
    }
    fn active(&self) -> bool {
        self.0.borrow().loaded.is_some()
    }
}

// ------------------------------------------------------------ the host

pub(crate) type BackendFactory = Rc<dyn Fn(&BackendCtx) -> Box<dyn MediaBackend>>;

thread_local! {
    static FACTORY_OVERRIDE: RefCell<Option<BackendFactory>> = const { RefCell::new(None) };
}

/// Test seam: the factory new engines on this thread use (None = the default).
pub(crate) fn set_backend_factory(f: Option<BackendFactory>) {
    FACTORY_OVERRIDE.with(|c| *c.borrow_mut() = f);
}

pub(crate) fn default_factory() -> BackendFactory {
    if let Some(f) = FACTORY_OVERRIDE.with(|c| c.borrow().clone()) {
        return f;
    }
    #[cfg(feature = "media")]
    {
        Rc::new(|ctx: &BackendCtx| -> Box<dyn MediaBackend> { Box::new(crate::media_mpv::MpvBackend::new(ctx.clone())) })
    }
    #[cfg(not(feature = "media"))]
    {
        Rc::new(|_: &BackendCtx| -> Box<dyn MediaBackend> { Box::new(UnavailableBackend) })
    }
}

/// One `(element id, thing for the page)` pair.
pub(crate) type HostOut = (i32, Out);

/// All media elements of one page: owns their cores, applies D7 (at most 2
/// players) and logs the one-time lines.
pub(crate) struct MediaHost {
    els: HashMap<i32, (MediaCore, u64)>,
    factory: BackendFactory,
    ctx: BackendCtx,
    pub(crate) activated: bool,
    seq: u64,
    logged_unavailable: bool,
    logged_backend: bool,
    log: Rc<dyn Fn(&str, &str)>,
    avail: Option<bool>,
}

impl MediaHost {
    pub(crate) fn new(ctx: BackendCtx, log: Rc<dyn Fn(&str, &str)>) -> Self {
        MediaHost {
            els: HashMap::new(),
            factory: default_factory(),
            ctx,
            activated: false,
            seq: 0,
            logged_unavailable: false,
            logged_backend: false,
            log,
            avail: None,
        }
    }

    pub(crate) fn backend_available(&mut self) -> bool {
        if self.avail.is_none() {
            self.avail = Some((self.factory)(&self.ctx).available());
        }
        self.avail.unwrap_or(false)
    }

    pub(crate) fn core(&mut self, id: i32) -> &mut MediaCore {
        let (f, ctx) = (self.factory.clone(), self.ctx.clone());
        let act = self.activated;
        let c = &mut self.els.entry(id).or_insert_with(|| (MediaCore::new(f(&ctx)), 0)).0;
        c.activated = act;
        c
    }

    pub(crate) fn peek(&self, id: i32) -> Option<&MediaCore> {
        self.els.get(&id).map(|e| &e.0)
    }

    pub(crate) fn remove(&mut self, id: i32) {
        if let Some((mut c, _)) = self.els.remove(&id) {
            c.backend.unload();
        }
    }

    /// True while some element has live backend work to poll.
    pub(crate) fn busy(&self) -> bool {
        self.els.values().any(|(c, _)| c.active() && (!c.paused || c.seeking || c.network_state == NETWORK_LOADING))
    }

    /// Runs `f` on element `id`'s core and returns what it produced for the page.
    pub(crate) fn with<R>(&mut self, id: i32, f: impl FnOnce(&mut MediaCore) -> R) -> (R, Vec<HostOut>) {
        self.seq += 1;
        let seq = self.seq;
        let c = self.core(id);
        let was_paused = c.paused;
        let r = f(c);
        let now_paused = c.paused;
        let outs = c.take_outs();
        if was_paused && !now_paused {
            if let Some(e) = self.els.get_mut(&id) {
                e.1 = seq;
            }
        }
        let mut all = self.absorb(id, outs);
        all.extend(self.enforce_cap());
        (r, all)
    }

    /// Polls every live backend; the first one may block up to `wait`.
    pub(crate) fn pump(&mut self, now: f64, wait: Duration) -> Vec<HostOut> {
        let mut ids: Vec<i32> = self.els.iter().filter(|(_, (c, _))| c.active()).map(|(i, _)| *i).collect();
        ids.sort();
        let mut all = Vec::new();
        for (n, id) in ids.into_iter().enumerate() {
            let act = self.activated;
            let Some((c, _)) = self.els.get_mut(&id) else { continue };
            c.activated = act;
            c.pump(now, if n == 0 { wait } else { Duration::ZERO });
            let outs = c.take_outs();
            all.extend(self.absorb(id, outs));
        }
        all.extend(self.enforce_cap());
        all
    }

    fn absorb(&mut self, id: i32, outs: Vec<Out>) -> Vec<HostOut> {
        let mut v = Vec::new();
        for o in outs {
            match o {
                Out::Unavailable => {
                    if !self.logged_unavailable {
                        self.logged_unavailable = true;
                        (self.log)("error", "scratchy: js: not implemented: media (libmpv not available)");
                    }
                }
                Out::BackendStarted => {
                    if !self.logged_backend {
                        self.logged_backend = true;
                        let name = self.els.get(&id).map_or("", |e| e.0.backend_name());
                        if name == "mpv" {
                            (self.log)("media", "media: mpv");
                        }
                    }
                }
                o => v.push((id, o)),
            }
        }
        v
    }

    /// D7: starting a third player pauses the oldest one that is playing.
    fn enforce_cap(&mut self) -> Vec<HostOut> {
        let mut all = Vec::new();
        loop {
            let mut playing: Vec<(u64, i32)> = self.els.iter().filter(|(_, (c, _))| !c.paused && c.active()).map(|(i, (_, s))| (*s, *i)).collect();
            if playing.len() <= MAX_PLAYERS {
                break;
            }
            playing.sort();
            let id = playing[0].1;
            let Some((c, _)) = self.els.get_mut(&id) else { break };
            c.pause();
            let outs = c.take_outs();
            all.extend(self.absorb(id, outs));
        }
        all
    }
}
