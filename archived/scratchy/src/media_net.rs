//! QM1e: the media BYTE CHANNEL. Plain Rust, no libmpv: a seekable byte
//! source over wire, for the later libmpv `stream_cb` glue (QM1f/QM1g) and the
//! `<video>`/`<audio>` element layer to sit on.
//!
//! Design (TODO.txt QM1e, REGRESSIONS.md R30): repeated ranged GETs, one wire
//! connection each, NO wire protocol change. Every request carries `Range`
//! and `X-Wire-Max-Bytes`. Reads are served from a small chunk cache (mpv asks
//! for 64-128 KiB at a time); a miss fetches 1 MiB, 4 MiB after two
//! contiguous fetches, back to 1 MiB after a seek. Nothing here touches the
//! page-JS fetch queues (`js_net`): the policy checks, cap headers and cookie
//! partition headers are the same building blocks, called directly.
//!
//! Threading: `MediaStream` is `Send`; `read`/`seek` block on wire and are
//! meant to run on mpv's demux thread. `CancelHandle::cancel` may be called
//! from any thread: it shuts down the in-flight socket, so a blocked `read`
//! returns `MediaNetError::Cancelled` at once instead of after 30 s.
//!
//! NOT here (QM1f/QM1g): the libmpv dlopen/stream_cb glue, MediaError mapping,
//! the `file:` Range case (needs wire W-MEDIA1; `file:` media falls back to
//! the forward-only mode today because wire ignores Range there).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use url::Url;

use crate::net::{build_wire_request, note_cookie_response, parse_wire_response_full, timing_enabled, WireResponse};
use crate::net_policy::{
    cookie_support, is_same_origin, wire_cookie_headers_with, CookieContext, CookieSupport, CredentialsMode, PolicyError,
    PrivatePolicy, RequestPolicy, WIRE_HEADER_MAX_BYTES, WIRE_HEADER_MAX_REDIRECTS, WIRE_HEADER_POLICY, WIRE_HEADER_TIMEOUT_MS,
};

pub(crate) const CHUNK_SMALL: u64 = 1 << 20;
pub(crate) const CHUNK_LARGE: u64 = 4 << 20;
/// Chunks kept per stream (mpv seeks to the moov at the end and back at load).
pub(crate) const CACHE_CHUNKS: usize = 4;
/// Per-request wire timeout (it includes the body).
pub(crate) const RANGE_TIMEOUT_MS: u64 = 30_000;
/// Timeout of the single streaming GET of the forward-only fallback (1 h).
pub(crate) const STREAM_TIMEOUT_MS: u64 = 3_600_000;
const MAX_HEAD: usize = 64 * 1024;

/// The mpv options QM1b tested (no option rejected); the glue sets them all
/// before `mpv_initialize`. Bounded demuxer cache included.
pub(crate) const MPV_OPTIONS: &[(&str, &str)] = &[
    ("config", "no"),
    ("ytdl", "no"),
    ("load-scripts", "no"),
    ("osc", "no"),
    ("input-default-bindings", "no"),
    ("input-vo-keyboard", "no"),
    ("input-terminal", "no"),
    ("terminal", "no"),
    ("load-unsafe-playlists", "no"),
    ("load-auto-profiles", "no"),
    ("load-stats-overlay", "no"),
    ("load-console", "no"),
    ("load-select", "no"),
    ("audio-display", "no"),
    ("cover-art-auto", "no"),
    ("sub-auto", "no"),
    ("audio-file-auto", "no"),
    ("save-position-on-quit", "no"),
    ("watch-later-directory", "/nonexistent"),
    ("vo", "libmpv"),
    ("ao", "null"),
    ("keep-open", "yes"),
    // QNET1: defence in depth. ffmpeg (HLS/DASH playlists, mov external refs) may
    // not open any nested URL itself: only `crypto`/`data` are whitelisted, so no
    // http/tcp/udp/file/... protocol exists for it (the seccomp network lock in
    // netlock.rs is the hard stop). `%11%` = length-prefixed value (it holds a comma).
    ("demuxer-lavf-o", "protocol_whitelist=%11%crypto,data"),
    ("stream-lavf-o", "protocol_whitelist=%11%crypto,data"),
    ("demuxer-max-bytes", "32MiB"),
    ("demuxer-max-back-bytes", "16MiB"),
];

/// How the element asks for the resource (Fetch request mode + credentials).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaMode {
    /// No `crossorigin` attribute: no-cors with credentials (jar on, opaque
    /// to page JS but playable).
    NoCors,
    /// `crossorigin=anonymous`: CORS, no credentials (`X-Wire-Cookies: off`).
    Anonymous,
    /// `crossorigin=use-credentials`: CORS with the jar; ACAO + ACAC checked.
    UseCredentials,
}

/// What the QM1f element layer maps to a `MediaError` (it picks the code).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaNetError {
    /// The request policy refused the URL (scheme, bad port, mixed content, ...).
    Blocked(String),
    /// wire unreachable, I/O error, timeout, or wire's own transport failure.
    Network(String),
    /// A status the channel does not accept (404, 403, 5xx, a 3xx, ...).
    Status(u16),
    /// CORS check failed on the first response.
    Cors(String),
    /// A later response came from a different origin than the first.
    MixedOrigin { first: String, later: String },
    /// Missing/wrong `Content-Range`, a body longer than asked, or the resource
    /// changed under us.
    BadRange(String),
    /// A short body that was still short after one retry.
    Truncated,
    /// `CancelHandle::cancel` was called.
    Cancelled,
}

impl std::fmt::Display for MediaNetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaNetError::Blocked(e) => write!(f, "media request blocked: {e}"),
            MediaNetError::Network(e) => write!(f, "media network error: {e}"),
            MediaNetError::Status(s) => write!(f, "media request failed with status {s}"),
            MediaNetError::Cors(e) => write!(f, "media CORS check failed: {e}"),
            MediaNetError::MixedOrigin { first, later } => {
                write!(f, "media redirected across origins ({first} then {later})")
            }
            MediaNetError::BadRange(e) => write!(f, "media range response invalid: {e}"),
            MediaNetError::Truncated => write!(f, "media response truncated"),
            MediaNetError::Cancelled => write!(f, "media request cancelled"),
        }
    }
}

impl std::error::Error for MediaNetError {}

/// Cancels the in-flight request of one stream from any thread.
#[derive(Clone)]
pub(crate) struct CancelHandle(Arc<CancelState>);

struct CancelState {
    cancelled: AtomicBool,
    conn: Mutex<Option<UnixStream>>,
}

impl CancelHandle {
    pub(crate) fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        if let Some(c) = self.0.conn.lock().unwrap().take() {
            let _ = c.shutdown(std::net::Shutdown::Both);
        }
    }
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }
}

/// One media resource: what the element holds, mapped from the `<id>` of
/// `scratchy-media://<id>`.
#[derive(Debug, Clone)]
pub(crate) struct MediaSource {
    pub url: Url,
    pub page: Url,
    pub mode: MediaMode,
}

struct Chunk {
    start: u64,
    data: Vec<u8>,
}

/// Forward-only fallback: ONE streaming GET (the server ignores Range).
struct Forward {
    conn: UnixStream,
    /// Absolute position of the next byte `conn` yields.
    pos: u64,
    /// Body bytes read together with the head.
    pending: Vec<u8>,
}

enum Mode {
    /// Range-capable: size known (None only for `bytes a-b/*`).
    Ranged,
    /// The whole body was small enough to arrive in the first answer.
    Whole,
    Forward(Forward),
}

pub(crate) struct MediaStream {
    socket: PathBuf,
    src: MediaSource,
    policy: RequestPolicy,
    support: &'static CookieSupport,
    cancel: CancelHandle,
    /// First response's final URL (or the source URL): later requests go here.
    pinned: Url,
    size: Option<u64>,
    pos: u64,
    mode: Mode,
    cache: Vec<Chunk>, // most recently used last
    /// Contiguous fetches in a row (drives the 1 MiB -> 4 MiB growth).
    seq: u32,
    next_expected: u64,
    /// Number of wire requests made (tests, timing).
    pub(crate) requests: u32,
    /// Size of the next fetch is computed from `seq`; exposed for tests.
    pub(crate) last_chunk_len: u64,
    warning: Option<&'static str>,
}

fn origin_str(u: &Url) -> String {
    u.origin().ascii_serialization()
}

impl MediaStream {
    /// Checks the policy, makes the first ranged request and the CORS check.
    pub(crate) fn open(socket: PathBuf, src: MediaSource, policy: RequestPolicy) -> Result<Self, MediaNetError> {
        Self::open_with(socket, src, policy, cookie_support())
    }

    pub(crate) fn open_with(
        socket: PathBuf,
        src: MediaSource,
        policy: RequestPolicy,
        support: &'static CookieSupport,
    ) -> Result<Self, MediaNetError> {
        let outcome = policy.check_request(&src.page, &src.url, "GET", false).map_err(|e: PolicyError| MediaNetError::Blocked(e.to_string()))?;
        let cancel = CancelHandle(Arc::new(CancelState { cancelled: AtomicBool::new(false), conn: Mutex::new(None) }));
        let pinned = src.url.clone();
        let mut s = MediaStream {
            socket,
            src,
            policy,
            support,
            cancel,
            pinned,
            size: None,
            pos: 0,
            mode: Mode::Ranged,
            cache: Vec::new(),
            seq: 0,
            next_expected: 0,
            requests: 0,
            last_chunk_len: 0,
            warning: outcome.warning,
        };
        s.first_request()?;
        Ok(s)
    }

    pub(crate) fn cancel_handle(&self) -> CancelHandle {
        self.cancel.clone()
    }

    /// The private-target class to warn about once per element (policy `Warn`).
    pub(crate) fn policy_warning(&self) -> Option<&'static str> {
        self.warning
    }

    /// Total size (stream_cb `size_fn`); None when unknown (forward-only mode).
    pub(crate) fn size(&self) -> Option<u64> {
        self.size
    }

    /// False in forward-only mode: backward seeks restart the download.
    pub(crate) fn seekable(&self) -> bool {
        !matches!(self.mode, Mode::Forward(_))
    }

    pub(crate) fn position(&self) -> u64 {
        self.pos
    }

    pub(crate) fn is_forward_only(&self) -> bool {
        matches!(self.mode, Mode::Forward(_))
    }

    // ---- stream_cb-shaped wrappers: bytes read, 0 = EOF, -1 = error ----

    pub(crate) fn read_cb(&mut self, buf: &mut [u8]) -> i64 {
        match self.read(buf) {
            Ok(n) => n as i64,
            Err(_) => -1,
        }
    }
    pub(crate) fn seek_cb(&mut self, pos: u64) -> i64 {
        match self.seek(pos) {
            Ok(p) => p as i64,
            Err(_) => -1,
        }
    }
    pub(crate) fn size_cb(&self) -> i64 {
        self.size.map_or(-1, |s| s as i64)
    }

    // ---- public byte API ----

    pub(crate) fn seek(&mut self, pos: u64) -> Result<u64, MediaNetError> {
        if self.cancel.is_cancelled() {
            return Err(MediaNetError::Cancelled);
        }
        if matches!(&self.mode, Mode::Forward(f) if pos < f.pos) {
            // Backward: restart the GET from 0 (discarding happens lazily in read).
            let fresh = self.start_forward()?;
            self.mode = Mode::Forward(fresh);
        }
        self.pos = pos;
        Ok(pos)
    }

    /// Reads up to `buf.len()` bytes at the current position; 0 = EOF.
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> Result<usize, MediaNetError> {
        if self.cancel.is_cancelled() {
            return Err(MediaNetError::Cancelled);
        }
        if buf.is_empty() {
            return Ok(0);
        }
        if matches!(self.mode, Mode::Forward(_)) {
            return self.read_forward(buf);
        }
        if self.size.is_some_and(|s| self.pos >= s) {
            return Ok(0);
        }
        if !self.cache.iter().any(|c| Self::covers(c, self.pos)) {
            self.fetch_at(self.pos)?;
            if self.size.is_some_and(|s| self.pos >= s) {
                return Ok(0);
            }
        }
        let Some(i) = self.cache.iter().position(|c| Self::covers(c, self.pos)) else {
            return Ok(0); // EOF found by an empty/short answer
        };
        let c = self.cache.remove(i);
        let off = (self.pos - c.start) as usize;
        let n = buf.len().min(c.data.len() - off);
        buf[..n].copy_from_slice(&c.data[off..off + n]);
        self.cache.push(c);
        self.pos += n as u64;
        Ok(n)
    }

    fn covers(c: &Chunk, pos: u64) -> bool {
        pos >= c.start && pos < c.start + c.data.len() as u64
    }

    // ---- requests ----

    fn base_headers(&self, mode_range: Option<(u64, u64)>, max_bytes: Option<u64>, timeout_ms: u64) -> Vec<(String, String)> {
        let mut h: Vec<(String, String)> = Vec::new();
        if let Some((a, b)) = mode_range {
            h.push(("Range".into(), format!("bytes={a}-{b}")));
        }
        if let Some(m) = max_bytes {
            h.push((WIRE_HEADER_MAX_BYTES.into(), m.to_string()));
        }
        h.push((WIRE_HEADER_TIMEOUT_MS.into(), timeout_ms.to_string()));
        h.push((WIRE_HEADER_MAX_REDIRECTS.into(), self.policy.redirect_limit.to_string()));
        if self.policy.private_targets == PrivatePolicy::Block {
            h.push((WIRE_HEADER_POLICY.into(), "public-only".into()));
        }
        let ctx = match self.src.mode {
            MediaMode::NoCors => CookieContext::subresource(&self.src.page),
            MediaMode::Anonymous => CookieContext::fetch(&self.src.page, CredentialsMode::Omit, false),
            MediaMode::UseCredentials => CookieContext::fetch(&self.src.page, CredentialsMode::Include, true),
        };
        h.extend(wire_cookie_headers_with(self.support, &ctx));
        if self.src.mode != MediaMode::NoCors && !is_same_origin(&self.src.page, &self.pinned) {
            h.push(("Origin".into(), origin_str(&self.src.page)));
        }
        h
    }

    fn connect(&self) -> Result<UnixStream, MediaNetError> {
        if self.cancel.is_cancelled() {
            return Err(MediaNetError::Cancelled);
        }
        let conn = UnixStream::connect(&self.socket).map_err(|e| {
            crate::note_wire_skip(Some(&self.socket), &crate::describe_os_error(&e));
            MediaNetError::Network(format!("cannot connect to wire: {e}"))
        })?;
        *self.cancel.0.conn.lock().unwrap() = conn.try_clone().ok();
        if self.cancel.is_cancelled() {
            let _ = conn.shutdown(std::net::Shutdown::Both);
            return Err(MediaNetError::Cancelled);
        }
        Ok(conn)
    }

    fn io_err(&self, e: std::io::Error) -> MediaNetError {
        if self.cancel.is_cancelled() {
            return MediaNetError::Cancelled;
        }
        match e.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => MediaNetError::Network("timed out".into()),
            _ => MediaNetError::Network(e.to_string()),
        }
    }

    fn send(&self, conn: &mut UnixStream, headers: &[(String, String)], idle: Duration) -> Result<Vec<u8>, MediaNetError> {
        let req = build_wire_request("GET", &self.pinned, headers, &[]).map_err(|e| MediaNetError::Network(e.to_string()))?;
        let _ = conn.set_read_timeout(Some(idle));
        let _ = conn.set_write_timeout(Some(idle));
        conn.write_all(&req).map_err(|e| self.io_err(e))?;
        Ok(req)
    }

    /// Reads the response head; returns (head incl. blank line, body bytes already read).
    fn read_head(&self, conn: &mut UnixStream) -> Result<(Vec<u8>, Vec<u8>), MediaNetError> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        loop {
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let rest = buf.split_off(p + 4);
                return Ok((buf, rest));
            }
            if buf.len() > MAX_HEAD {
                return Err(MediaNetError::Network("response head too large".into()));
            }
            let n = conn.read(&mut tmp).map_err(|e| self.io_err(e))?;
            if n == 0 {
                return Err(if self.cancel.is_cancelled() { MediaNetError::Cancelled } else { MediaNetError::Network("wire closed the connection before a response".into()) });
            }
            buf.extend_from_slice(&tmp[..n]);
        }
    }

    /// One whole ranged exchange: returns the parsed response (body = what arrived,
    /// at most `limit`+1 bytes).
    fn exchange(&mut self, start: u64, len: u64) -> Result<WireResponse, MediaNetError> {
        let t0 = Instant::now();
        let end = start + len - 1;
        let headers = self.base_headers(Some((start, end)), Some(len), RANGE_TIMEOUT_MS);
        let mut conn = self.connect()?;
        let idle = Duration::from_millis(RANGE_TIMEOUT_MS + 5_000);
        let req = self.send(&mut conn, &headers, idle)?;
        self.requests += 1;
        let (head, mut body) = self.read_head(&mut conn)?;
        let mut parsed = parse_wire_response_full(&head).map_err(|e| MediaNetError::Network(e.to_string()))?;
        // Body: at most len+1 bytes for a payload; error pages are tiny.
        let cap = (len + 1) as usize;
        if body.len() < cap {
            let mut limited = (&mut conn).take((cap - body.len()) as u64);
            limited.read_to_end(&mut body).map_err(|e| self.io_err(e))?;
        }
        if self.cancel.is_cancelled() {
            return Err(MediaNetError::Cancelled);
        }
        parsed.body = body;
        if parsed.status == 502 {
            // Re-derive the error text now that the body is here.
            let text = String::from_utf8_lossy(&parsed.body).trim().to_string();
            if parsed.error.is_some() || text.starts_with("wire: ") {
                parsed.error = Some(text);
            }
        } else {
            parsed.error = None;
        }
        note_cookie_response(self.support, &req, parsed.header("x-wire-version"), parsed.header("x-wire-cookies"));
        crate::user_agent::note_ua_response(parsed.header("x-wire-features"));
        if timing_enabled() {
            eprintln!(
                "media range {} bytes={start}-{end} status {} bytes {} {} ms",
                self.pinned,
                parsed.status,
                parsed.body.len(),
                t0.elapsed().as_millis()
            );
        }
        *self.cancel.0.conn.lock().unwrap() = None;
        Ok(parsed)
    }

    /// Redirect pinning + same-origin check of every response's final URL.
    fn note_final_url(&mut self, resp: &WireResponse, first: bool) -> Result<(), MediaNetError> {
        let Some(f) = resp.final_url.as_deref().and_then(|u| Url::parse(u).ok()) else { return Ok(()) };
        if first {
            if f != self.pinned {
                self.policy
                    .check_request(&self.src.page, &f, "GET", false)
                    .map_err(|e| MediaNetError::Blocked(e.to_string()))?;
                self.pinned = f;
            }
        } else if origin_str(&f) != origin_str(&self.pinned) {
            return Err(MediaNetError::MixedOrigin { first: origin_str(&self.pinned), later: origin_str(&f) });
        }
        Ok(())
    }

    fn cors_check(&self, resp: &WireResponse) -> Result<(), MediaNetError> {
        if self.src.mode == MediaMode::NoCors || is_same_origin(&self.src.page, &self.pinned) {
            return Ok(());
        }
        let origin = origin_str(&self.src.page);
        let acao = resp.header("access-control-allow-origin").map(str::trim);
        match (self.src.mode, acao) {
            (_, None) => Err(MediaNetError::Cors("no Access-Control-Allow-Origin header".into())),
            (MediaMode::Anonymous, Some("*")) => Ok(()),
            (MediaMode::UseCredentials, Some("*")) => Err(MediaNetError::Cors("wildcard Access-Control-Allow-Origin with credentials".into())),
            (m, Some(a)) if a == origin => {
                if m == MediaMode::UseCredentials
                    && resp.header("access-control-allow-credentials").is_none_or(|v| v.trim() != "true")
                {
                    return Err(MediaNetError::Cors("missing Access-Control-Allow-Credentials: true".into()));
                }
                Ok(())
            }
            (_, Some(a)) => Err(MediaNetError::Cors(format!("Access-Control-Allow-Origin {a:?} does not match {origin}"))),
        }
    }

    fn is_too_large(resp: &WireResponse) -> bool {
        resp.status == 502 && resp.error.as_deref().is_some_and(|e| e.to_ascii_lowercase().contains("too large") || e.contains("too-large"))
    }

    fn status_error(resp: &WireResponse) -> MediaNetError {
        if let Some(e) = &resp.error {
            return MediaNetError::Network(e.clone());
        }
        MediaNetError::Status(resp.status)
    }

    fn first_request(&mut self) -> Result<(), MediaNetError> {
        let resp = self.exchange(0, CHUNK_SMALL)?;
        self.last_chunk_len = CHUNK_SMALL;
        self.note_final_url(&resp, true)?;
        match resp.status {
            206 => {
                self.cors_check(&resp)?;
                let (s, e, total) = parse_content_range(&resp)?;
                if s != 0 {
                    return Err(MediaNetError::BadRange(format!("Content-Range starts at {s}, asked for 0")));
                }
                self.size = total;
                self.accept_body(0, e, resp.body)?;
                self.seq = 1;
                self.next_expected = e + 1;
                Ok(())
            }
            200 if resp.body.len() as u64 <= CHUNK_SMALL => {
                // Server ignored Range but the whole file fit in the cap.
                self.cors_check(&resp)?;
                self.size = Some(resp.body.len() as u64);
                self.mode = Mode::Whole;
                self.cache.push(Chunk { start: 0, data: resp.body });
                Ok(())
            }
            416 => {
                self.cors_check(&resp)?;
                self.size = Some(0);
                Ok(())
            }
            200 => self.go_forward(),
            _ if Self::is_too_large(&resp) => self.go_forward(),
            _ => Err(Self::status_error(&resp)),
        }
    }

    fn go_forward(&mut self) -> Result<(), MediaNetError> {
        let f = self.start_forward()?;
        self.size = None;
        self.cache.clear();
        self.mode = Mode::Forward(f);
        Ok(())
    }

    /// ONE streaming GET from 0, no `X-Wire-Max-Bytes`, long timeout.
    fn start_forward(&mut self) -> Result<Forward, MediaNetError> {
        let headers = self.base_headers(None, None, STREAM_TIMEOUT_MS);
        let mut conn = self.connect()?;
        let req = self.send(&mut conn, &headers, Duration::from_millis(STREAM_TIMEOUT_MS))?;
        self.requests += 1;
        let (head, pending) = self.read_head(&mut conn)?;
        let resp = parse_wire_response_full(&head).map_err(|e| MediaNetError::Network(e.to_string()))?;
        note_cookie_response(self.support, &req, resp.header("x-wire-version"), resp.header("x-wire-cookies"));
        crate::user_agent::note_ua_response(resp.header("x-wire-features"));
        if timing_enabled() {
            eprintln!("media stream {} (forward-only) status {}", self.pinned, resp.status);
        }
        if resp.status != 200 {
            return Err(MediaNetError::Status(resp.status));
        }
        self.note_final_url(&resp, false)?;
        self.cors_check(&resp)?;
        // The idle timeout of later reads: long, wire's own timeout is the cap.
        let _ = conn.set_read_timeout(Some(Duration::from_secs(120)));
        Ok(Forward { conn, pos: 0, pending })
    }

    fn read_forward(&mut self, buf: &mut [u8]) -> Result<usize, MediaNetError> {
        let target = self.pos;
        let cancel = self.cancel.clone();
        let Mode::Forward(f) = &mut self.mode else { unreachable!() };
        let ioe = |e: std::io::Error| {
            if cancel.is_cancelled() {
                MediaNetError::Cancelled
            } else {
                MediaNetError::Network(e.to_string())
            }
        };
        // Discard up to the target (forward seek).
        let mut tmp = [0u8; 16 * 1024];
        while f.pos < target {
            let want = ((target - f.pos) as usize).min(tmp.len());
            let n = Self::forward_raw(f, &mut tmp[..want]).map_err(ioe)?;
            if n == 0 {
                return Ok(0);
            }
        }
        let n = Self::forward_raw(f, buf).map_err(ioe)?;
        self.pos += n as u64;
        Ok(n)
    }

    fn forward_raw(f: &mut Forward, out: &mut [u8]) -> std::io::Result<usize> {
        let n = if !f.pending.is_empty() {
            let n = out.len().min(f.pending.len());
            out[..n].copy_from_slice(&f.pending[..n]);
            f.pending.drain(..n);
            n
        } else {
            f.conn.read(out)?
        };
        f.pos += n as u64;
        Ok(n)
    }

    /// Validates a 206 body against its Content-Range and caches it.
    fn accept_body(&mut self, start: u64, end: u64, body: Vec<u8>) -> Result<(), MediaNetError> {
        let want = end - start + 1;
        if body.len() as u64 > want {
            return Err(MediaNetError::BadRange("body longer than Content-Range".into()));
        }
        if (body.len() as u64) < want {
            return Err(MediaNetError::Truncated);
        }
        self.cache.push(Chunk { start, data: body });
        while self.cache.len() > CACHE_CHUNKS {
            self.cache.remove(0);
        }
        Ok(())
    }

    /// Fetches the chunk holding `pos` (a cache miss).
    fn fetch_at(&mut self, pos: u64) -> Result<(), MediaNetError> {
        if pos == self.next_expected && self.seq > 0 {
            self.seq += 1;
        } else {
            self.seq = 1;
        }
        // After two contiguous fetches the third (and later) are 4 MiB.
        let mut len = if self.seq > 2 { CHUNK_LARGE } else { CHUNK_SMALL };
        if let Some(total) = self.size {
            len = len.min(total.saturating_sub(pos)).max(1);
        }
        self.last_chunk_len = len;
        let mut got: Vec<u8> = Vec::new();
        let mut at = pos;
        let mut retried = false;
        loop {
            let want = len - got.len() as u64;
            let resp = self.exchange(at, want)?;
            self.note_final_url(&resp, false)?;
            match resp.status {
                206 => {}
                416 => {
                    // Past the end (or the file shrank): Content-Range `bytes */TOTAL`.
                    if let Some(t) = resp.header("content-range").and_then(|v| v.rsplit('/').next()).and_then(|t| t.trim().parse::<u64>().ok()) {
                        self.size = Some(t);
                    } else {
                        self.size = Some(at.min(self.size.unwrap_or(at)));
                    }
                    return self.finish_fetch(pos, got);
                }
                _ => return Err(Self::status_error(&resp)),
            }
            let (s, e, total) = parse_content_range(&resp)?;
            if s != at {
                return Err(MediaNetError::BadRange(format!("Content-Range starts at {s}, asked for {at}")));
            }
            if total.is_some() && self.size.is_some() && total != self.size {
                return Err(MediaNetError::BadRange(format!("resource size changed from {:?} to {total:?}", self.size)));
            }
            if total.is_some() {
                self.size = total;
            }
            if e - s + 1 > want {
                return Err(MediaNetError::BadRange("Content-Range longer than asked".into()));
            }
            let expect = e - s + 1;
            let n = resp.body.len() as u64;
            if n > expect {
                return Err(MediaNetError::BadRange("body longer than Content-Range".into()));
            }
            got.extend_from_slice(&resp.body);
            if n == expect {
                break;
            }
            // Short body (wire cannot signal truncation, W-MEDIA2): retry once from
            // the new position.
            if retried || n == 0 {
                return Err(MediaNetError::Truncated);
            }
            retried = true;
            at = pos + got.len() as u64;
        }
        self.finish_fetch(pos, got)
    }

    fn finish_fetch(&mut self, pos: u64, got: Vec<u8>) -> Result<(), MediaNetError> {
        self.next_expected = pos + got.len() as u64;
        if !got.is_empty() {
            self.cache.push(Chunk { start: pos, data: got });
            while self.cache.len() > CACHE_CHUNKS {
                self.cache.remove(0);
            }
        }
        Ok(())
    }
}

/// `Content-Range: bytes S-E/T` (T may be `*`). A missing or malformed header on a
/// 206 is an error.
fn parse_content_range(resp: &WireResponse) -> Result<(u64, u64, Option<u64>), MediaNetError> {
    let v = resp.header("content-range").ok_or_else(|| MediaNetError::BadRange("206 without Content-Range".into()))?;
    let bad = || MediaNetError::BadRange(format!("malformed Content-Range {v:?}"));
    let rest = v.trim().strip_prefix("bytes").ok_or_else(bad)?.trim();
    let (range, total) = rest.split_once('/').ok_or_else(bad)?;
    let (s, e) = range.split_once('-').ok_or_else(bad)?;
    let (s, e): (u64, u64) = (s.trim().parse().map_err(|_| bad())?, e.trim().parse().map_err(|_| bad())?);
    if e < s {
        return Err(bad());
    }
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse::<u64>().map_err(|_| bad())?),
    };
    Ok((s, e, total))
}
