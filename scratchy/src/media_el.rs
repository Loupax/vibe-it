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
use std::sync::Arc;
use std::time::Duration;

use crate::media_video::{VideoFrame, VideoTarget};

use crate::media_net::MediaMode;
use crate::mse_store::MseFeed;
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

/// A parsed MIME type: lowercase `type/subtype` and its `name=value` parameters
/// (names lowercased, quotes stripped; `;` inside quotes does not split).
struct ParsedMime {
    mime: String,
    params: Vec<(String, String)>,
}

fn parse_mime(t: &str) -> ParsedMime {
    let mut parts: Vec<String> = vec![String::new()];
    let mut quote: Option<char> = None;
    for ch in t.trim().chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
                parts.last_mut().unwrap().push(ch);
            }
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                parts.last_mut().unwrap().push(ch);
            }
            None if ch == ';' => parts.push(String::new()),
            None => parts.last_mut().unwrap().push(ch),
        }
    }
    let mime = parts[0].trim().to_ascii_lowercase();
    let params = parts[1..]
        .iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            Some((k.trim().to_ascii_lowercase(), v.to_string()))
        })
        .collect();
    ParsedMime { mime, params }
}

/// Chrome-like validation of the probing parameters (YouTube sends each one valid and
/// invalid). Unknown parameters are ignored; `codecs` is checked by the callers.
fn params_valid(params: &[(String, String)]) -> bool {
    params.iter().all(|(k, v)| {
        let in_range = |lo: f64, hi: f64| v.parse::<f64>().is_ok_and(|n| n.is_finite() && n >= lo && n <= hi);
        let v = v.to_ascii_lowercase();
        match k.as_str() {
            "width" | "height" => in_range(1.0, 8192.0),
            "framerate" => in_range(1.0, 480.0),
            "bitrate" => in_range(1.0, 200_000_000.0),
            // `2`, `6`, or `5.1` style: the leading count must be 1..=8
            "channels" => v.parse::<f64>().is_ok() && v.split('.').next().is_some_and(|c| c.parse::<f64>().is_ok_and(|n| (1.0..=8.0).contains(&n))),
            "eotf" => matches!(v.as_str(), "bt709" | "smpte2084" | "arib-std-b67"),
            "cryptoblockformat" => matches!(v.as_str(), "subsample" | "sample"),
            "decode-to-texture" | "tunnelmode" => matches!(v.as_str(), "true" | "false"),
            "experimental" => v == "allowed",
            _ => true,
        }
    })
}

fn codecs_of(params: &[(String, String)]) -> Option<Vec<String>> {
    let (_, v) = params.iter().find(|(k, _)| k == "codecs")?;
    Some(v.split(',').map(|s| s.trim().to_ascii_lowercase()).collect())
}

fn codec_matches(c: &str, prefix: &str) -> bool {
    c == prefix || (c.starts_with(prefix) && c.as_bytes()[prefix.len()] == b'.')
}

/// `canPlayType`: `maybe`/`probably` only for what ffmpeg here decodes
/// (mp4/h264/aac, webm/vp8/vp9/opus/vorbis, ogg, mp3, flac, wav); `` otherwise.
/// Invalid probing parameters (width=99999, eotf=catavision, ...) answer ``.
/// The caller returns `` for everything when no backend is available.
pub(crate) fn can_play_type(t: &str) -> &'static str {
    let p = parse_mime(t);
    // codecs known per container family
    let allowed: &[&str] = match p.mime.as_str() {
        "video/mp4" | "audio/mp4" | "audio/x-m4a" | "audio/aac" | "video/x-m4v" => &["avc1", "avc3", "mp4a.40", "mp4a.69", "mp4a.6b", "mp3", "flac", "opus", "vp09", "vp9"],
        "video/webm" | "audio/webm" => &["vp8", "vp8.0", "vp9", "vp09", "opus", "vorbis"],
        "video/ogg" | "audio/ogg" | "application/ogg" => &["theora", "vorbis", "opus", "flac"],
        "audio/mpeg" | "audio/mp3" => &["mp3"],
        "audio/flac" | "audio/x-flac" => &["flac"],
        "audio/wav" | "audio/wave" | "audio/x-wav" => &["1"],
        _ => return "",
    };
    if !params_valid(&p.params) {
        return "";
    }
    let Some(codecs) = codecs_of(&p.params) else { return "maybe" };
    if codecs.iter().all(|c| c.is_empty()) {
        return "maybe";
    }
    if codecs.iter().all(|c| !c.is_empty() && allowed.iter().any(|a| codec_matches(c, a))) {
        "probably"
    } else {
        ""
    }
}

/// QM1k4: the ONE table behind `MediaSource.isTypeSupported`: (container MIME, codec
/// prefix, enabled). A codec is switched on by flipping its flag (QM1k1j) and adding its
/// test; listed-but-off entries document what is planned. An empty prefix = the
/// container itself (what a type without `codecs` asks about).
pub(crate) const MSE_CODECS: &[(&str, &str, bool)] = &[
    ("video/mp4", "", true),
    ("audio/mp4", "", true),
    ("video/mp4", "avc1", true),
    ("video/mp4", "avc3", true),
    ("video/mp4", "mp4a.40.2", true),
    ("video/mp4", "mp4a.40.5", true),
    ("video/mp4", "mp4a.40.29", true),
    ("audio/mp4", "mp4a.40.2", true),
    ("audio/mp4", "mp4a.40.5", true),
    ("audio/mp4", "mp4a.40.29", true),
    ("video/mp4", "vp09", false),
    ("video/mp4", "av01", false),
    ("video/mp4", "opus", false),
    ("video/mp4", "flac", false),
    ("audio/mp4", "opus", false),
    ("audio/mp4", "flac", false),
    ("video/webm", "", false),
    ("audio/webm", "", false),
    ("video/webm", "vp8", false),
    ("video/webm", "vp9", false),
    ("video/webm", "vp09", false),
    ("video/webm", "av01", false),
    ("video/webm", "opus", false),
    ("video/webm", "vorbis", false),
    ("audio/webm", "opus", false),
    ("audio/webm", "vorbis", false),
];

/// `MediaSource.isTypeSupported(t)`: pure, no backend. True only when the container and
/// every listed codec are enabled in `MSE_CODECS` and the probing parameters are valid.
pub(crate) fn mse_type_supported(t: &str) -> bool {
    let p = parse_mime(t);
    if !params_valid(&p.params) {
        return false;
    }
    let enabled = |prefix: &str| MSE_CODECS.iter().any(|(m, c, on)| *on && *m == p.mime && *c == prefix);
    let enabled_codec = |codec: &str| MSE_CODECS.iter().any(|(m, c, on)| *on && *m == p.mime && !c.is_empty() && codec_matches(codec, c));
    match codecs_of(&p.params) {
        None => enabled(""),
        Some(codecs) => enabled("") && codecs.iter().all(|c| !c.is_empty() && enabled_codec(c)),
    }
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
    /// QM1k1b: loads a MediaSource feed (generation starting at `start`, media time). The
    /// bytes come from memory (`scratchy-mse://`), never from the wire. Default: refused.
    #[allow(dead_code)] // the element core calls it from QM1k1c
    fn load_mse(&mut self, _feed: Arc<MseFeed>, _start: f64) -> Result<(), LoadError> {
        Err(LoadError::Other("MSE not supported".into()))
    }
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
    /// QM1g: what to render (the visible video box's size), or None when the
    /// video is not on screen (stop rendering, and decoding where possible).
    fn set_video_target(&mut self, _t: Option<VideoTarget>) {}
    /// QM1g: true when a new frame is waiting (set from another thread).
    fn frame_pending(&self) -> bool {
        false
    }
    /// QM1g: renders the newest frame into a fresh buffer of the target size
    /// (None: nothing new, or no target; a pending frame is still consumed).
    fn take_frame(&mut self) -> Option<Arc<VideoFrame>> {
        None
    }
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
    /// QM1k1c: fire `sourceopen` / `sourceended` / `sourceclose` at the MediaSource
    /// whose feed has this id (the JS side routes it, QM1k1d).
    MseOpen(u64),
    MseEnded(u64),
    MseClose(u64),
}

/// QM1k1c: the MSE attachment of one element (the QM1k-PLAN "Element" paragraph).
struct MseAttach {
    feed: Arc<MseFeed>,
    /// `loadedmetadata` was fired (every SourceBuffer has its init segment).
    meta: bool,
    /// A `load_mse` is live (a generation exists in the backend).
    issued: bool,
    /// Where the next (re)load starts, until both tracks cover it.
    want: Option<f64>,
    /// `endOfStream()` was called: only then may `ended` happen.
    ended: bool,
    last_progress: f64,
}

/// Intersection of several sorted range lists (an empty list in = nothing out).
pub(crate) fn intersect_ranges(all: &[Vec<(f64, f64)>]) -> Vec<(f64, f64)> {
    let Some((first, rest)) = all.split_first() else { return Vec::new() };
    let mut cur = first.clone();
    for other in rest {
        let mut next = Vec::new();
        for &(a, b) in &cur {
            for &(c, d) in other {
                let (lo, hi) = (a.max(c), b.min(d));
                if hi > lo {
                    next.push((lo, hi));
                }
            }
        }
        cur = next;
    }
    cur
}

/// A start time this close to a range edge counts as inside (first frames rarely start at
/// exactly the requested time).
const MSE_COVER_SLACK: f64 = 0.1;
/// `progress` is fired at most this often while appending (ms).
const MSE_PROGRESS_MS: f64 = 350.0;

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
    /// QM1g: the newest rendered frame and the size the backend was asked for.
    pub last_frame: Option<Arc<VideoFrame>>,
    pub target: Option<VideoTarget>,
    mse: Option<MseAttach>,
    loadeddata_fired: bool,
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
            last_frame: None,
            target: None,
            mse: None,
            loadeddata_fired: false,
        }
    }

    /// QM1g: tells the backend the on-screen size (None = not visible).
    pub(crate) fn set_target(&mut self, t: Option<VideoTarget>) {
        if self.target != t {
            self.target = t;
            self.backend.set_video_target(t);
        }
    }
    /// QM1g: a frame is waiting in the backend.
    pub(crate) fn frame_pending(&self) -> bool {
        self.backend.frame_pending()
    }
    /// QM1g: renders the waiting frame; true when `last_frame` is new.
    pub(crate) fn render_frame(&mut self) -> bool {
        match self.backend.take_frame() {
            Some(f) => {
                self.last_frame = Some(f);
                true
            }
            None => false,
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
        match &self.mse {
            Some(m) => {
                let ended = m.ended;
                m.feed.with_store(|st| {
                    let mut per: Vec<Vec<(f64, f64)>> = st.buffers.iter().map(|b| b.buffered()).collect();
                    if ended {
                        // after endOfStream every track's last range reaches the highest end
                        let hi = per.iter().filter_map(|r| r.last().map(|x| x.1)).fold(0.0, f64::max);
                        for r in per.iter_mut() {
                            if let Some(l) = r.last_mut() {
                                l.1 = hi;
                            }
                        }
                    }
                    intersect_ranges(&per)
                })
            }
            None => self.backend.buffered(),
        }
    }
    pub(crate) fn seekable(&self) -> Vec<(f64, f64)> {
        match &self.mse {
            Some(_) if self.duration.is_finite() && self.duration > 0.0 => vec![(0.0, self.duration)],
            Some(_) => Vec::new(),
            None => self.backend.seekable(),
        }
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
        self.reset_for_load();
        self.begin(cands, from_attr, mode, page);
        if self.cands.is_empty() {
            return;
        }
        self.network_state = NETWORK_NO_SOURCE;
        self.network_state = NETWORK_LOADING;
        self.fire("loadstart");
        self.start_candidate();
    }

    /// The first half of the load algorithm: abort what is loaded (an MSE attachment is
    /// detached: its feed stops and its MediaSource gets `sourceclose`).
    fn reset_for_load(&mut self) {
        if let Some(m) = self.mse.take() {
            m.feed.stop();
            self.outs.push(Out::MseClose(m.feed.id));
        }
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
            self.last_frame = None;
        }
        self.loadeddata_fired = false;
    }

    fn begin(&mut self, cands: Vec<Candidate>, from_attr: bool, mode: MediaMode, page: String) {
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


    // ---- MSE (QM1k1c) ----

    /// Attach a MediaSource feed as this element's resource (`el.src = blobUrl`): the load
    /// algorithm with no candidate. `sourceopen` follows; `loadedmetadata` once every
    /// SourceBuffer has its init segment, the first backend load once both tracks cover the
    /// start time. `duration` is the MediaSource's current duration (NaN until set).
    pub(crate) fn attach_mse(&mut self, feed: Arc<MseFeed>, duration: f64, page: String) {
        self.reset_for_load();
        self.begin(Vec::new(), true, MediaMode::NoCors, page);
        self.network_state = NETWORK_LOADING;
        self.duration = duration;
        self.current_src = format!("blob:scratchy-mse/{}", feed.id);
        self.fire("loadstart");
        self.outs.push(Out::MseOpen(feed.id));
        self.mse = Some(MseAttach { feed, meta: false, issued: false, want: None, ended: false, last_progress: f64::NEG_INFINITY });
    }

    /// A `blob:` src that names nothing (revoked before the load): the load algorithm with one
    /// candidate that fails at once (MEDIA_ERR_SRC_NOT_SUPPORTED).
    pub(crate) fn fail_src(&mut self, page: String, msg: &str) {
        self.reset_for_load();
        self.begin(Vec::new(), true, MediaMode::NoCors, page);
        self.network_state = NETWORK_LOADING;
        self.fire("loadstart");
        self.fatal_error(MEDIA_ERR_SRC_NOT_SUPPORTED, msg);
    }

    pub(crate) fn mse_attached(&self) -> bool {
        self.mse.is_some()
    }

    /// The id of the attached MediaSource feed.
    pub(crate) fn mse_feed_id(&self) -> Option<u64> {
        self.mse.as_ref().map(|m| m.feed.id)
    }

    /// `endOfStream('network' | 'decode')`: like [`mse_end_of_stream`](Self::mse_end_of_stream)
    /// (EOF to the player, `sourceended`) and the element gets a MediaError.
    pub(crate) fn mse_end_of_stream_error(&mut self, code: u16) {
        let Some(m) = self.mse.as_mut() else { return };
        if m.ended {
            return;
        }
        m.ended = true;
        m.feed.end_of_stream();
        self.outs.push(Out::MseEnded(m.feed.id));
        self.fatal_error(code, if code == MEDIA_ERR_DECODE { "MediaSource decode error" } else { "MediaSource network error" });
    }

    /// The element goes away (removed from the page): backend unloaded, MediaSource closed.
    pub(crate) fn detach(&mut self) {
        self.backend.unload();
        if let Some(m) = self.mse.take() {
            m.feed.stop();
            self.outs.push(Out::MseClose(m.feed.id));
        }
    }

    /// `MediaSource.duration = d`: the element's duration follows (mpv's is ignored).
    pub(crate) fn mse_set_duration(&mut self, d: f64) {
        if self.mse.is_some() && !(d == self.duration || (d.is_nan() && self.duration.is_nan())) {
            self.duration = d;
            self.fire("durationchange");
        }
    }

    /// `MediaSource.endOfStream()`: the feed reports EOF to the player, the duration becomes
    /// the highest buffered end and `ended` may now happen.
    pub(crate) fn mse_end_of_stream(&mut self) {
        let Some(m) = self.mse.as_mut() else { return };
        if m.ended {
            return;
        }
        m.ended = true;
        m.feed.end_of_stream();
        let id = m.feed.id;
        let hi = m.feed.with_store(|st| st.buffers.iter().filter_map(|b| b.buffered().last().map(|r| r.1)).fold(0.0, f64::max));
        self.outs.push(Out::MseEnded(id));
        if hi > 0.0 {
            self.mse_set_duration(hi);
        }
    }

    /// The JS side appended or removed data (`new_init`: the append carried a second init
    /// segment, a quality switch / changeType). `now` = page clock in ms.
    pub(crate) fn mse_data_changed(&mut self, now: f64, new_init: bool) {
        let Some(m) = self.mse.as_mut() else { return };
        let mut fire_meta = false;
        if !m.meta && m.feed.all_have_init() {
            m.meta = true;
            fire_meta = true;
        }
        if new_init && m.issued && m.want.is_none() {
            m.want = Some(self.current_time);
        }
        let progress = now - m.last_progress >= MSE_PROGRESS_MS;
        if progress {
            m.last_progress = now;
        }
        if fire_meta {
            self.loaded = true;
            self.ready_state = HAVE_METADATA;
            self.fire("loadedmetadata");
            if let Some(t) = self.default_start.take() {
                self.seeking = true;
                self.seek_target = t;
                self.current_time = t;
                self.fire("seeking");
            }
            let t = if self.seeking { self.seek_target } else { self.current_time };
            if let Some(m) = self.mse.as_mut() {
                m.want = Some(t);
            }
        }
        if progress {
            self.fire("progress");
        }
        self.mse_resolve();
    }

    /// A seek on an MSE element: inside the live generation it is a backend seek; anywhere
    /// else it waits (`waiting`, readyState HAVE_METADATA) until every track covers `t`,
    /// then the next generation is loaded at `t`.
    fn mse_seek(&mut self, t: f64) {
        self.seeking = true;
        self.seek_target = t;
        self.current_time = t;
        self.fire("seeking");
        let Some(m) = self.mse.as_mut() else { return };
        m.want = Some(t);
        let (issued, feed) = (m.issued, m.feed.clone());
        let playable = feed.with_store(|st| st.in_generation(t) || st.covers(t, MSE_COVER_SLACK));
        if issued && !playable && self.ready_state > HAVE_METADATA {
            self.ready_state = HAVE_METADATA;
            self.playing_fired = false;
            if !self.paused {
                self.fire("waiting");
            }
        }
        self.mse_resolve();
    }

    /// Acts on `want`: a backend seek, a new generation, or keeps waiting.
    fn mse_resolve(&mut self) {
        let Some(m) = self.mse.as_mut() else { return };
        let Some(t) = m.want else { return };
        if !m.meta {
            return;
        }
        let (issued, feed) = (m.issued, m.feed.clone());
        if issued && feed.with_store(|st| st.in_generation(t)) {
            m.want = None;
            self.backend.seek(t);
            return;
        }
        if !feed.with_store(|st| st.covers(t, MSE_COVER_SLACK)) {
            return;
        }
        m.want = None;
        if issued {
            self.backend.unload();
        }
        match self.backend.load_mse(feed, t) {
            Ok(()) => {
                if let Some(m) = self.mse.as_mut() {
                    m.issued = true;
                }
                self.backend.set_volume(self.volume);
                self.backend.set_muted(self.muted);
                self.backend.set_rate(self.rate);
                self.backend.set_paused(self.paused);
                if !self.announced {
                    self.announced = true;
                    self.outs.push(Out::BackendStarted);
                }
            }
            Err(LoadError::Unavailable) => {
                self.outs.push(Out::Unavailable);
                self.fatal_error(MEDIA_ERR_SRC_NOT_SUPPORTED, "libmpv not available");
            }
            Err(LoadError::Other(m)) => self.fatal_error(MEDIA_ERR_SRC_NOT_SUPPORTED, &m),
        }
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
        if self.mse.is_some() {
            return self.mse_seek(t);
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
        if !(0.0625..=16.0).contains(&r) {
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
            BackendEvent::Loaded { width, height, .. } if self.mse.is_some() => {
                // MSE: the duration is the MediaSource's, loadedmetadata already fired
                self.video_width = width;
                self.video_height = height;
            }
            BackendEvent::DurationChanged(_) if self.mse.is_some() => {}
            BackendEvent::Eof if self.mse.as_ref().is_some_and(|m| !m.ended) => {
                // the player ran out of appended data: a stall, never `ended`
                if !self.paused && self.ready_state >= HAVE_FUTURE_DATA {
                    self.ready_state = HAVE_CURRENT_DATA;
                    self.playing_fired = false;
                    self.fire("waiting");
                }
            }
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
                    if !self.loadeddata_fired {
                        self.loadeddata_fired = true;
                        self.fire("loadeddata");
                    }
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
    /// QM1g: the last target given, a pending frame flag and the frames rendered.
    pub target: Option<VideoTarget>,
    pub pending_frame: bool,
    pub renders: u32,
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
    fn load_mse(&mut self, feed: Arc<MseFeed>, start: f64) -> Result<(), LoadError> {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("load_mse {} {start}", feed.id));
        if let Some(e) = s.fail_load.clone() {
            return Err(e);
        }
        let gid = feed.start_generation(start);
        s.loaded = Some(LoadReq { url: format!("scratchy-mse://{}/{gid}/0", feed.id), page: String::new(), mode: MediaMode::NoCors });
        s.position = start;
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
    fn set_video_target(&mut self, t: Option<VideoTarget>) {
        let mut s = self.0.borrow_mut();
        s.calls.push(format!("target {:?}", t.map(|t| (t.w, t.h))));
        s.target = t;
    }
    fn frame_pending(&self) -> bool {
        self.0.borrow().pending_frame
    }
    fn take_frame(&mut self) -> Option<Arc<VideoFrame>> {
        let mut s = self.0.borrow_mut();
        if !std::mem::take(&mut s.pending_frame) {
            return None;
        }
        let t = s.target?;
        s.renders += 1;
        let v = 0x0010_0000 + s.renders;
        Some(Arc::new(VideoFrame { w: t.w, h: t.h, px: vec![v; t.w as usize * t.h as usize] }))
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

/// QM1g: `(element id, newest frame, intrinsic video size)`.
pub(crate) type VideoStateRow = (i32, Option<Arc<VideoFrame>>, (u32, u32));

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

/// Where the host writes its log lines: (level, text).
pub(crate) type MediaLog = Rc<dyn Fn(&str, &str)>;

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
    log: MediaLog,
    avail: Option<bool>,
    sizes_seen: HashMap<i32, (u32, u32)>,
    /// QM1k1c: the MediaSource feeds of this page.
    pub(crate) mse: MseHost,
}

/// QM1k1c: per page, owns the [`MseFeed`] of every MediaSource (the JS objects hold ids).
#[derive(Default)]
pub(crate) struct MseHost {
    feeds: HashMap<u64, Arc<MseFeed>>,
}

impl MseHost {
    /// A new MediaSource: an empty feed (SourceBuffers are added with `add_buffer`).
    pub(crate) fn create(&mut self) -> Arc<MseFeed> {
        let f = MseFeed::new(crate::mse_store::MseStore::from_env());
        self.feeds.insert(f.id, f.clone());
        f
    }
    pub(crate) fn get(&self, id: u64) -> Option<Arc<MseFeed>> {
        self.feeds.get(&id).cloned()
    }
    pub(crate) fn len(&self) -> usize {
        self.feeds.len()
    }
    /// The MediaSource was collected: stop its readers and forget it.
    pub(crate) fn drop_feed(&mut self, id: u64) {
        if let Some(f) = self.feeds.remove(&id) {
            f.stop();
        }
    }
}

impl MediaHost {
    pub(crate) fn new(ctx: BackendCtx, log: MediaLog) -> Self {
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
            sizes_seen: HashMap::new(),
            mse: MseHost::default(),
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

    /// Runs `f` on the element the MediaSource feed `feed` is attached to (None: not attached).
    pub(crate) fn with_mse<R>(&mut self, feed: u64, f: impl FnOnce(&mut MediaCore) -> R) -> Option<(R, Vec<HostOut>)> {
        let id = self.els.iter().find(|(_, (c, _))| c.mse_feed_id() == Some(feed)).map(|(i, _)| *i)?;
        Some(self.with(id, f))
    }

    pub(crate) fn peek(&self, id: i32) -> Option<&MediaCore> {
        self.els.get(&id).map(|e| &e.0)
    }

    /// The element left the page; returns what it still owes the page (`sourceclose`).
    pub(crate) fn remove(&mut self, id: i32) -> Vec<HostOut> {
        match self.els.remove(&id) {
            Some((mut c, _)) => {
                c.detach();
                let outs = c.take_outs();
                self.absorb(id, outs)
            }
            None => Vec::new(),
        }
    }

    /// QM1g: sets the render target of every element (ids absent from `t`
    /// are not visible: no rendering).
    pub(crate) fn set_targets(&mut self, t: &HashMap<i32, VideoTarget>) {
        for (id, (c, _)) in self.els.iter_mut() {
            c.set_target(t.get(id).copied());
        }
    }

    /// QM1g: renders every waiting frame (also draining invisible ones);
    /// returns the ids that got a new frame.
    pub(crate) fn render_frames(&mut self) -> Vec<i32> {
        let mut ids: Vec<i32> = self.els.iter().filter(|(_, (c, _))| c.frame_pending()).map(|(i, _)| *i).collect();
        ids.sort();
        ids.retain(|id| self.els.get_mut(id).is_some_and(|(c, _)| c.render_frame()));
        ids
    }

    /// QM1g: `(id, newest frame, intrinsic size)` of every element that has one.
    pub(crate) fn video_state(&self) -> Vec<VideoStateRow> {
        self.els.iter().map(|(i, (c, _))| (*i, c.last_frame.clone(), (c.video_width, c.video_height))).collect()
    }

    /// QM1g: true once when some element's video size changed since the last
    /// call (the page layout must be rebuilt: the box takes the video size).
    pub(crate) fn take_size_changed(&mut self) -> bool {
        let now: HashMap<i32, (u32, u32)> = self.els.iter().filter(|(_, (c, _))| c.video_width > 0).map(|(i, (c, _))| (*i, (c.video_width, c.video_height))).collect();
        let changed = now != self.sizes_seen;
        self.sizes_seen = now;
        changed
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
        if let (true, Some(e)) = (was_paused && !now_paused, self.els.get_mut(&id)) {
            e.1 = seq;
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
