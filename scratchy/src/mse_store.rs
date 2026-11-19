//! QM1k1a: the pure-Rust half of Media Source Extensions (no JS, no mpv).
//!
//! * A byte-stream reader behind the [`ByteStreamFormat`] trait (fMP4 today in
//!   [`Fmp4Reader`]; a WebM reader can be added later). It splits appended bytes
//!   into an init segment and media segments and reads ONLY container timing
//!   headers (`mdhd` timescale, `tkhd` track id, `trex`/`tfhd` defaults, `tfdt`,
//!   `trun` sample durations), never codec data or samples (user-approved, Q7).
//! * A per-SourceBuffer segment store ([`SourceBufferStore`]) sorted by start
//!   time, with segment-granular overlap replacement and `remove`, merged
//!   `buffered` ranges, and the generation model (one mpv load = an init plus the
//!   contiguous run of segments from a start time, addressed by virtual byte
//!   offsets whose base survives eviction).
//! * [`MseStore`]: one per MediaSource; byte accounting, the eviction policy and
//!   the `MSE_MAX_BYTES` cap (env `SCRATCHY_MSE_MAX_BYTES`) across its buffers.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Default cap on stored media per MediaSource (all its SourceBuffers).
pub const MSE_MAX_BYTES: usize = 64 * 1024 * 1024;
/// Gap tolerated when merging buffered ranges and when continuing a run.
pub const MSE_GAP_TOLERANCE: f64 = 0.1;
/// Segments ending more than this before currentTime may be evicted (if read).
pub const MSE_KEEP_BEHIND: f64 = 10.0;
/// Segments starting more than this ahead of currentTime may be evicted (if unreached).
pub const MSE_KEEP_AHEAD: f64 = 60.0;
const EPS: f64 = 1e-6;
/// A single top-level box larger than this is refused as a parse error.
const MAX_BOX: u64 = 1 << 30;

/// `SCRATCHY_MSE_MAX_BYTES` if set to a positive integer, else [`MSE_MAX_BYTES`].
pub fn mse_max_bytes() -> usize {
    parse_max_bytes(std::env::var("SCRATCHY_MSE_MAX_BYTES").ok().as_deref())
}

/// The env-value rule of [`mse_max_bytes`] (separate so tests need no `set_var`).
pub fn parse_max_bytes(v: Option<&str>) -> usize {
    v.and_then(|v| v.trim().parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(MSE_MAX_BYTES)
}

#[derive(Debug, Clone, PartialEq)]
pub enum MseError {
    /// The byte stream is malformed (-> the MSE append error / MEDIA_ERR_DECODE).
    Parse(String),
    /// The cap cannot be met even after eviction (-> `QuotaExceededError`).
    QuotaExceeded,
}

/// What a byte-stream reader hands out.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Init(Vec<u8>),
    Media { bytes: Vec<u8>, start: f64, end: f64 },
    Error(String),
}

/// A container byte-stream reader (the "byte stream format" of the MSE spec).
pub trait ByteStreamFormat: Send {
    /// Feed appended bytes; boxes split across calls are buffered. After an
    /// `Error` every further call answers `Error` until [`reset`](Self::reset).
    fn feed(&mut self, data: &[u8]) -> Vec<Parsed>;
    /// `SourceBuffer.abort()`: drop partial input and the error state; keeps the
    /// track table of the last init segment.
    fn reset(&mut self);
}

// ---------- fMP4 reader ----------

#[derive(Debug, Clone, Copy)]
struct TrackInfo {
    timescale: u32,
    default_dur: u32,
}

type BoxList<'a> = Vec<([u8; 4], &'a [u8])>;

/// Split a payload into its child boxes (size 0 = to the end).
fn child_boxes(data: &[u8]) -> Result<BoxList<'_>, String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let rest = &data[pos..];
        if rest.len() < 8 {
            return Err("truncated box header".into());
        }
        let size32 = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let ty = [rest[4], rest[5], rest[6], rest[7]];
        let (hdr, size) = match size32 {
            0 => (8usize, rest.len() as u64),
            1 => {
                if rest.len() < 16 {
                    return Err("truncated large box header".into());
                }
                (16usize, rd64(rest, 8)?)
            }
            n => (8usize, u64::from(n)),
        };
        if size < hdr as u64 || size > rest.len() as u64 {
            return Err(format!("box {} overruns its parent", fourcc(&ty)));
        }
        out.push((ty, &rest[hdr..size as usize]));
        pos += size as usize;
    }
    Ok(out)
}

fn fourcc(t: &[u8; 4]) -> String {
    t.iter().map(|&c| if c.is_ascii_graphic() { c as char } else { '?' }).collect()
}

fn rd32(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "box too short".to_string())
}

fn rd64(b: &[u8], o: usize) -> Result<u64, String> {
    let hi = u64::from(rd32(b, o)?);
    let lo = u64::from(rd32(b, o + 4)?);
    Ok(hi << 32 | lo)
}

fn find<'a>(boxes: &BoxList<'a>, ty: &[u8; 4]) -> Option<&'a [u8]> {
    boxes.iter().find(|(t, _)| t == ty).map(|(_, b)| *b)
}

/// (track_id, timescale) from `trak`.
fn parse_trak(body: &[u8]) -> Result<(u32, u32), String> {
    let kids = child_boxes(body)?;
    let tkhd = find(&kids, b"tkhd").ok_or("trak without tkhd")?;
    let id = if tkhd.first() == Some(&1) { rd32(tkhd, 20)? } else { rd32(tkhd, 12)? };
    let mdia = find(&kids, b"mdia").ok_or("trak without mdia")?;
    let mkids = child_boxes(mdia)?;
    let mdhd = find(&mkids, b"mdhd").ok_or("mdia without mdhd")?;
    let ts = if mdhd.first() == Some(&1) { rd32(mdhd, 20)? } else { rd32(mdhd, 12)? };
    if ts == 0 {
        return Err("mdhd timescale is 0".into());
    }
    Ok((id, ts))
}

/// The fragmented-MP4 reader. Top-level boxes: `ftyp`+`moov` = init segment;
/// `styp`/`sidx`/`emsg`/`prft` + `moof` + `mdat` = one media segment.
pub struct Fmp4Reader {
    pending: Vec<u8>,
    init_acc: Vec<u8>,
    prefix: Vec<u8>,
    seg: Vec<u8>,
    cur: Option<(f64, f64)>,
    tracks: HashMap<u32, TrackInfo>,
    last_end: HashMap<u32, u64>,
    failed: Option<String>,
}

impl Default for Fmp4Reader {
    fn default() -> Self {
        Self::new()
    }
}

impl Fmp4Reader {
    pub fn new() -> Self {
        Fmp4Reader {
            pending: Vec::new(),
            init_acc: Vec::new(),
            prefix: Vec::new(),
            seg: Vec::new(),
            cur: None,
            tracks: HashMap::new(),
            last_end: HashMap::new(),
            failed: None,
        }
    }

    fn parse_moov(&mut self, payload: &[u8]) -> Result<(), String> {
        let mut tracks = HashMap::new();
        let mut trex: HashMap<u32, u32> = HashMap::new();
        for (ty, body) in child_boxes(payload)? {
            match &ty {
                b"trak" => {
                    let (id, timescale) = parse_trak(body)?;
                    tracks.insert(id, TrackInfo { timescale, default_dur: 0 });
                }
                b"mvex" => {
                    for (t2, b2) in child_boxes(body)? {
                        if &t2 == b"trex" {
                            // ver/flags, track_id, sample_description_index, default_sample_duration
                            trex.insert(rd32(b2, 4)?, rd32(b2, 12)?);
                        }
                    }
                }
                _ => {}
            }
        }
        if tracks.is_empty() {
            return Err("moov has no track".into());
        }
        for (id, d) in trex {
            if let Some(t) = tracks.get_mut(&id) {
                t.default_dur = d;
            }
        }
        self.tracks = tracks;
        self.last_end.clear();
        Ok(())
    }

    /// (start, end) seconds of a `moof` from its container headers only.
    fn parse_moof(&mut self, payload: &[u8]) -> Result<(f64, f64), String> {
        let mut t0 = f64::INFINITY;
        let mut t1 = f64::NEG_INFINITY;
        for (ty, traf) in child_boxes(payload)? {
            if &ty != b"traf" {
                continue;
            }
            let kids = child_boxes(traf)?;
            let tfhd = find(&kids, b"tfhd").ok_or("traf without tfhd")?;
            let flags = rd32(tfhd, 0)? & 0x00ff_ffff;
            let id = rd32(tfhd, 4)?;
            let info = *self.tracks.get(&id).ok_or_else(|| format!("traf for unknown track {id}"))?;
            let mut o = 8usize;
            if flags & 0x1 != 0 {
                o += 8;
            }
            if flags & 0x2 != 0 {
                o += 4;
            }
            let mut default_dur = info.default_dur;
            if flags & 0x8 != 0 {
                default_dur = rd32(tfhd, o)?;
            }
            let start_ticks = match find(&kids, b"tfdt") {
                Some(d) => {
                    if d.first() == Some(&1) {
                        rd64(d, 4)?
                    } else {
                        u64::from(rd32(d, 4)?)
                    }
                }
                None => self.last_end.get(&id).copied().unwrap_or(0),
            };
            let mut dur: u64 = 0;
            for (t, run) in &kids {
                if t != b"trun" {
                    continue;
                }
                let rf = rd32(run, 0)? & 0x00ff_ffff;
                let count = rd32(run, 4)? as usize;
                let mut p = 8usize;
                if rf & 0x1 != 0 {
                    p += 4;
                }
                if rf & 0x4 != 0 {
                    p += 4;
                }
                let has_dur = rf & 0x100 != 0;
                let fields = usize::from(has_dur) + usize::from(rf & 0x200 != 0) + usize::from(rf & 0x400 != 0) + usize::from(rf & 0x800 != 0);
                let stride = 4 * fields;
                if run.len() < p.saturating_add(stride.saturating_mul(count)) {
                    return Err("trun shorter than its sample count".into());
                }
                if has_dur {
                    for i in 0..count {
                        dur += u64::from(rd32(run, p + i * stride)?);
                    }
                } else if count > 0 {
                    if default_dur == 0 && flags & 0x10000 == 0 {
                        return Err("no sample duration in trun, tfhd or trex".into());
                    }
                    dur += u64::from(default_dur) * count as u64;
                }
            }
            let end_ticks = start_ticks + dur;
            self.last_end.insert(id, end_ticks);
            t0 = t0.min(start_ticks as f64 / f64::from(info.timescale));
            t1 = t1.max(end_ticks as f64 / f64::from(info.timescale));
        }
        if !t0.is_finite() {
            return Err("moof without traf".into());
        }
        Ok((t0, t1))
    }

    fn handle_box(&mut self, ty: [u8; 4], whole: &[u8], out: &mut Vec<Parsed>) -> Result<(), String> {
        let hdr = if rd32(whole, 0)? == 1 { 16 } else { 8 };
        match &ty {
            b"ftyp" => {
                self.init_acc.clear();
                self.init_acc.extend_from_slice(whole);
            }
            b"moov" => {
                self.parse_moov(&whole[hdr..])?;
                self.init_acc.extend_from_slice(whole);
                self.prefix.clear();
                self.seg.clear();
                self.cur = None;
                out.push(Parsed::Init(std::mem::take(&mut self.init_acc)));
            }
            b"styp" | b"sidx" | b"emsg" | b"prft" => {
                if self.tracks.is_empty() {
                    return Err("media segment before the init segment".into());
                }
                self.prefix.extend_from_slice(whole);
            }
            b"moof" => {
                if self.tracks.is_empty() {
                    return Err("moof before the init segment".into());
                }
                let t = self.parse_moof(&whole[hdr..])?;
                self.seg = std::mem::take(&mut self.prefix);
                self.seg.extend_from_slice(whole);
                self.cur = Some(t);
            }
            b"mdat" => {
                let (start, end) = self.cur.take().ok_or("mdat without a moof")?;
                self.seg.extend_from_slice(whole);
                out.push(Parsed::Media { bytes: std::mem::take(&mut self.seg), start, end });
            }
            _ => {} // free, skip, mfra, meta, ...: ignored
        }
        Ok(())
    }

    fn pump(&mut self, out: &mut Vec<Parsed>) -> Result<(), String> {
        let mut pos = 0usize;
        let res = loop {
            let rest = &self.pending[pos..];
            if rest.len() < 8 {
                break Ok(());
            }
            let size32 = u64::from(rd32(rest, 0)?);
            let ty = [rest[4], rest[5], rest[6], rest[7]];
            let size = match size32 {
                0 => break Err("a top-level box of size 0 is not supported".to_string()),
                1 => {
                    if rest.len() < 16 {
                        break Ok(());
                    }
                    rd64(rest, 8)?
                }
                n => n,
            };
            let hdr = if size32 == 1 { 16 } else { 8 };
            if size < hdr || size > MAX_BOX {
                break Err(format!("bad size {size} for box {}", fourcc(&ty)));
            }
            if (rest.len() as u64) < size {
                break Ok(());
            }
            let whole = rest[..size as usize].to_vec();
            pos += size as usize;
            if let Err(e) = self.handle_box(ty, &whole, out) {
                break Err(e);
            }
        };
        self.pending.drain(..pos);
        res
    }
}

impl ByteStreamFormat for Fmp4Reader {
    fn feed(&mut self, data: &[u8]) -> Vec<Parsed> {
        if let Some(e) = &self.failed {
            return vec![Parsed::Error(e.clone())];
        }
        self.pending.extend_from_slice(data);
        let mut out = Vec::new();
        if let Err(e) = self.pump(&mut out) {
            self.failed = Some(e.clone());
            self.pending.clear();
            out.push(Parsed::Error(e));
        }
        out
    }

    fn reset(&mut self) {
        self.pending.clear();
        self.init_acc.clear();
        self.prefix.clear();
        self.seg.clear();
        self.cur = None;
        self.failed = None;
    }
}

// ---------- segment store ----------

#[derive(Debug, Clone)]
pub struct Segment {
    pub id: u64,
    pub start: f64,
    pub end: f64,
    /// Which init segment this media goes with (see `SourceBufferStore::init_id`).
    pub init_id: u64,
    pub data: Arc<Vec<u8>>,
}

#[derive(Debug, Clone)]
struct RunSeg {
    id: u64,
    off: u64,
    len: u64,
    end: f64,
}

/// One mpv load: an init segment plus the contiguous run of segments from `start`.
/// Virtual bytes: `[0, init.len())` = init, then the run's segments back to back.
/// `base` = the first still-held virtual offset of the run; it only grows when the
/// run's front is evicted, so offsets of the remaining bytes never change.
#[derive(Debug, Clone)]
pub struct Generation {
    pub id: u64,
    pub start: f64,
    init: Arc<Vec<u8>>,
    init_id: u64,
    run: VecDeque<RunSeg>,
    base: u64,
    next_off: u64,
    read_off: u64,
}

impl Generation {
    /// Length of the init segment (virtual offsets below it are the init).
    pub fn init_len(&self) -> u64 {
        self.init.len() as u64
    }
}

/// Result of [`SourceBufferStore::read_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadAt {
    /// This many bytes were copied (> 0).
    Bytes(usize),
    /// No byte at that offset yet: block until more is appended (or EOF after endOfStream).
    Wait,
    /// The offset was evicted from the front of the run.
    Evicted,
    /// The generation is not the live one any more.
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendOutcome {
    /// An init segment arrived after another one: the caller must start a new generation.
    pub new_generation: bool,
    /// Media segments stored by this append.
    pub segments: usize,
}

pub struct SourceBufferStore {
    fmt: Box<dyn ByteStreamFormat>,
    init: Option<Arc<Vec<u8>>>,
    init_id: u64,
    segments: Vec<Segment>, // sorted by start
    live: Option<Generation>,
    next_seg_id: u64,
}

impl SourceBufferStore {
    pub fn new(fmt: Box<dyn ByteStreamFormat>) -> Self {
        SourceBufferStore { fmt, init: None, init_id: 0, segments: Vec::new(), live: None, next_seg_id: 1 }
    }

    pub fn fmp4() -> Self {
        Self::new(Box::new(Fmp4Reader::new()))
    }

    pub fn has_init(&self) -> bool {
        self.init.is_some()
    }

    pub fn init_bytes(&self) -> Option<&[u8]> {
        self.init.as_deref().map(|v| v.as_slice())
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Stored bytes (init + all segments).
    pub fn bytes(&self) -> usize {
        self.init.as_ref().map_or(0, |i| i.len()) + self.segments.iter().map(|s| s.data.len()).sum::<usize>()
    }

    /// `SourceBuffer.abort()` parser reset.
    pub fn abort(&mut self) {
        self.fmt.reset();
    }

    /// Merged ranges `(start, end)`; neighbours within [`MSE_GAP_TOLERANCE`] merge.
    pub fn buffered(&self) -> Vec<(f64, f64)> {
        let mut out: Vec<(f64, f64)> = Vec::new();
        for s in &self.segments {
            match out.last_mut() {
                Some(last) if s.start <= last.1 + MSE_GAP_TOLERANCE + EPS => last.1 = last.1.max(s.end),
                _ => out.push((s.start, s.end)),
            }
        }
        out
    }

    /// `remove(a, b)`: drop segments lying fully inside `[a, b]`; returns how many.
    pub fn remove(&mut self, a: f64, b: f64) -> usize {
        let before = self.segments.len();
        self.segments.retain(|s| !(s.start >= a - EPS && s.end <= b + EPS));
        self.fix_run();
        before - self.segments.len()
    }

    fn remove_id(&mut self, id: u64) {
        self.segments.retain(|s| s.id != id);
        self.fix_run();
    }

    fn insert(&mut self, start: f64, end: f64, bytes: Vec<u8>) {
        // Overlap replacement (whole segments).
        self.segments.retain(|s| !(s.start < end - EPS && s.end > start + EPS));
        let seg = Segment { id: self.next_seg_id, start, end, init_id: self.init_id, data: Arc::new(bytes) };
        self.next_seg_id += 1;
        let at = self.segments.partition_point(|s| s.start < start);
        self.segments.insert(at, seg);
        self.fix_run();
        self.extend_run();
    }

    /// Feed bytes. Parsed segments are stored before an error is reported.
    pub fn append(&mut self, data: &[u8]) -> Result<AppendOutcome, MseError> {
        let mut outcome = AppendOutcome { new_generation: false, segments: 0 };
        for p in self.fmt.feed(data) {
            match p {
                Parsed::Init(bytes) => {
                    if self.init.is_some() {
                        outcome.new_generation = true;
                    }
                    self.init = Some(Arc::new(bytes));
                    self.init_id += 1;
                    self.live = None; // the caller starts a new generation
                }
                Parsed::Media { bytes, start, end } => {
                    self.insert(start, end, bytes);
                    outcome.segments += 1;
                }
                Parsed::Error(e) => return Err(MseError::Parse(e)),
            }
        }
        Ok(outcome)
    }

    // ----- generation -----

    pub fn generation(&self) -> Option<&Generation> {
        self.live.as_ref()
    }

    /// Begin generation `id` at time `t`: the run starts at the stored segment
    /// covering `t` and continues through contiguous segments. Needs an init segment.
    pub fn start_generation(&mut self, id: u64, t: f64) -> bool {
        let Some(init) = self.init.clone() else {
            self.live = None;
            return false;
        };
        let len = init.len() as u64;
        self.live = Some(Generation {
            id,
            start: t,
            init,
            init_id: self.init_id,
            run: VecDeque::new(),
            base: len,
            next_off: len,
            read_off: 0,
        });
        self.extend_run();
        true
    }

    /// Drop the live generation (readers of it get `ReadAt::Stale`).
    pub fn stop_generation(&mut self) {
        self.live = None;
    }

    /// End time of the live run (None when it is empty or there is no generation).
    pub fn run_end(&self) -> Option<f64> {
        self.live.as_ref().and_then(|g| g.run.back().map(|r| r.end))
    }

    /// Virtual size so far: init + all run bytes ever put in the run (the "file" length).
    pub fn generation_len(&self) -> u64 {
        self.live.as_ref().map_or(0, |g| g.next_off)
    }

    /// First still-held virtual offset of the run.
    pub fn generation_base(&self) -> u64 {
        self.live.as_ref().map_or(0, |g| g.base)
    }

    /// Copy virtual bytes `[off, ..)` of generation `gen_id` into `out`.
    pub fn read_at(&mut self, gen_id: u64, off: u64, out: &mut [u8]) -> ReadAt {
        let Some(g) = self.live.as_mut() else { return ReadAt::Stale };
        if g.id != gen_id {
            return ReadAt::Stale;
        }
        if out.is_empty() {
            return ReadAt::Wait;
        }
        let init_len = g.init.len() as u64;
        let n = if off < init_len {
            let n = out.len().min((init_len - off) as usize);
            out[..n].copy_from_slice(&g.init[off as usize..off as usize + n]);
            n
        } else {
            if off < g.base {
                return ReadAt::Evicted;
            }
            let Some(r) = g.run.iter().find(|r| off >= r.off && off < r.off + r.len) else {
                return ReadAt::Wait;
            };
            let Some(seg) = self.segments.iter().find(|s| s.id == r.id) else { return ReadAt::Evicted };
            let within = (off - r.off) as usize;
            let n = out.len().min(seg.data.len() - within);
            out[..n].copy_from_slice(&seg.data[within..within + n]);
            n
        };
        g.read_off = g.read_off.max(off + n as u64);
        ReadAt::Bytes(n)
    }

    /// All held bytes of the live generation: init + the run (test/diagnostic helper;
    /// `None` after the front of the run was evicted).
    pub fn generation_bytes(&mut self) -> Option<Vec<u8>> {
        let (id, len, base, init_len) = {
            let g = self.live.as_ref()?;
            (g.id, g.next_off, g.base, g.init.len() as u64)
        };
        if base > init_len {
            return None;
        }
        let mut out = Vec::new();
        let mut buf = vec![0u8; 65536];
        while (out.len() as u64) < len {
            match self.read_at(id, out.len() as u64, &mut buf) {
                ReadAt::Bytes(n) => out.extend_from_slice(&buf[..n]),
                _ => return None,
            }
        }
        Some(out)
    }

    /// Bring the run back in line with the segment list after removals: a removed
    /// prefix advances `base`; a hole in the middle truncates the run there.
    fn fix_run(&mut self) {
        let Some(g) = self.live.as_mut() else { return };
        let segs = &self.segments;
        let alive = |id: u64| segs.iter().any(|s| s.id == id);
        while g.run.front().is_some_and(|r| !alive(r.id)) {
            g.run.pop_front();
        }
        if let Some(i) = g.run.iter().position(|r| !alive(r.id)) {
            g.run.truncate(i);
        }
        if let Some(r) = g.run.back() {
            g.next_off = r.off + r.len;
        }
        g.base = g.run.front().map_or(g.next_off, |r| r.off);
    }

    /// Append every stored segment that continues the run (or covers the start time).
    fn extend_run(&mut self) {
        loop {
            let Some(g) = self.live.as_mut() else { return };
            let next = match g.run.back() {
                Some(last) => {
                    let t = last.end;
                    self.segments
                        .iter()
                        .filter(|s| s.init_id == g.init_id && s.id != last.id)
                        .filter(|s| s.start >= t - MSE_GAP_TOLERANCE - EPS && s.start <= t + MSE_GAP_TOLERANCE + EPS)
                        .filter(|s| !g.run.iter().any(|r| r.id == s.id))
                        .min_by(|a, b| a.start.total_cmp(&b.start))
                }
                None => {
                    let t = g.start;
                    self.segments
                        .iter()
                        .filter(|s| s.init_id == g.init_id && s.start <= t + EPS && s.end > t)
                        .min_by(|a, b| a.start.total_cmp(&b.start))
                }
            };
            let Some(s) = next else { return };
            let len = s.data.len() as u64;
            g.run.push_back(RunSeg { id: s.id, off: g.next_off, len, end: s.end });
            g.next_off += len;
        }
    }

    fn in_run(&self, id: u64) -> bool {
        self.live.as_ref().is_some_and(|g| g.run.iter().any(|r| r.id == id))
    }

    /// In the live run and fully served to the reader (mpv has read past it).
    fn run_read_past(&self, id: u64) -> bool {
        self.live.as_ref().is_some_and(|g| g.run.iter().any(|r| r.id == id && r.off + r.len <= g.read_off))
    }
}

/// One per MediaSource: its SourceBuffers, the shared cap and the generation counter.
pub struct MseStore {
    pub buffers: Vec<SourceBufferStore>,
    max_bytes: usize,
    next_gen: u64,
}

impl MseStore {
    pub fn new(max_bytes: usize) -> Self {
        MseStore { buffers: Vec::new(), max_bytes, next_gen: 1 }
    }

    pub fn from_env() -> Self {
        Self::new(mse_max_bytes())
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub fn add_buffer(&mut self, b: SourceBufferStore) -> usize {
        self.buffers.push(b);
        self.buffers.len() - 1
    }

    pub fn total_bytes(&self) -> usize {
        self.buffers.iter().map(|b| b.bytes()).sum()
    }

    /// Every buffer has buffered data around `t` (within `slack` of a range edge).
    pub fn covers(&self, t: f64, slack: f64) -> bool {
        !self.buffers.is_empty() && self.buffers.iter().all(|b| b.buffered().iter().any(|&(s, e)| s <= t + slack && e > t))
    }

    /// `t` lies inside the live generation of every buffer (an mpv seek can reach it).
    pub fn in_generation(&self, t: f64) -> bool {
        !self.buffers.is_empty() && self.buffers.iter().all(|b| b.generation().is_some_and(|g| g.start <= t + 0.1) && b.run_end().is_some_and(|e| e > t))
    }

    /// Start a new generation at `t` on every buffer; returns its id.
    pub fn start_generation(&mut self, t: f64) -> u64 {
        let id = self.next_gen;
        self.next_gen += 1;
        for b in &mut self.buffers {
            b.start_generation(id, t);
        }
        id
    }

    /// Evict until `need` more bytes fit under the cap. Order: (1) segments that
    /// end before `now - 10 s` and are not in the live run or were fully read by
    /// mpv, oldest first; (2) segments starting after `now + 60 s` that the live
    /// run has not reached, farthest first. Returns false when still over.
    pub fn make_room(&mut self, need: usize, now: f64) -> bool {
        if self.total_bytes() + need <= self.max_bytes {
            return true;
        }
        // (buffer, segment id, sort key)
        let mut behind: Vec<(usize, u64, f64)> = Vec::new();
        let mut ahead: Vec<(usize, u64, f64)> = Vec::new();
        for (bi, b) in self.buffers.iter().enumerate() {
            for s in &b.segments {
                if s.end < now - MSE_KEEP_BEHIND && (!b.in_run(s.id) || b.run_read_past(s.id)) {
                    behind.push((bi, s.id, s.end));
                } else if s.start > now + MSE_KEEP_AHEAD && !b.in_run(s.id) {
                    ahead.push((bi, s.id, s.start));
                }
            }
        }
        behind.sort_by(|a, b| a.2.total_cmp(&b.2));
        ahead.sort_by(|a, b| b.2.total_cmp(&a.2));
        for (bi, id, _) in behind.into_iter().chain(ahead) {
            if self.total_bytes() + need <= self.max_bytes {
                break;
            }
            self.buffers[bi].remove_id(id);
        }
        self.total_bytes() + need <= self.max_bytes
    }

    /// `appendBuffer` on buffer `bi` at page time `now` (currentTime). The cap is
    /// checked before the bytes are parsed (the incoming size is an upper bound of
    /// what gets stored), so a `QuotaExceeded` leaves the parser untouched.
    pub fn append(&mut self, bi: usize, data: &[u8], now: f64) -> Result<AppendOutcome, MseError> {
        if !self.make_room(data.len(), now) {
            return Err(MseError::QuotaExceeded);
        }
        self.buffers[bi].append(data)
    }
}

// ---------- the shared feed (QM1k1b) ----------

/// What a blocking feed read answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedRead {
    /// This many bytes were copied (> 0).
    Bytes(usize),
    /// End of stream: `endOfStream()` was called and no byte exists at that offset.
    Eof,
    /// Cancelled, the generation is stale, or the bytes were evicted.
    Gone,
}

struct FeedState {
    store: MseStore,
    ended: bool,
}

/// One per MediaSource: the [`MseStore`] behind a Mutex + Condvar. The JS thread appends
/// (never waits on mpv); mpv's demux threads block in [`read`](Self::read) until bytes
/// arrive, `endOfStream` is called, or the generation goes away.
pub struct MseFeed {
    pub id: u64,
    state: Mutex<FeedState>,
    cv: Condvar,
}

static NEXT_FEED_ID: AtomicU64 = AtomicU64::new(1);

impl MseFeed {
    pub fn new(store: MseStore) -> Arc<MseFeed> {
        Arc::new(MseFeed { id: NEXT_FEED_ID.fetch_add(1, Ordering::Relaxed), state: Mutex::new(FeedState { store, ended: false }), cv: Condvar::new() })
    }

    /// A feed with `tracks` fMP4 SourceBuffers and the default cap.
    pub fn with_fmp4_tracks(tracks: usize) -> Arc<MseFeed> {
        let mut st = MseStore::from_env();
        for _ in 0..tracks {
            st.add_buffer(SourceBufferStore::fmp4());
        }
        Self::new(st)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FeedState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `appendBuffer` on buffer `bi` at page time `now`; wakes blocked readers.
    pub fn append(&self, bi: usize, data: &[u8], now: f64) -> Result<AppendOutcome, MseError> {
        let r = self.lock().store.append(bi, data, now);
        self.cv.notify_all();
        r
    }

    /// `endOfStream()`: readers that run out of bytes now see EOF.
    pub fn end_of_stream(&self) {
        self.lock().ended = true;
        self.cv.notify_all();
    }

    pub fn track_count(&self) -> usize {
        self.lock().store.buffers.len()
    }

    /// Every buffer has its init segment (an mpv load may be issued).
    pub fn all_have_init(&self) -> bool {
        let g = self.lock();
        !g.store.buffers.is_empty() && g.store.buffers.iter().all(|b| b.has_init())
    }

    /// Run `f` on the store under the lock (buffered ranges, remove, ...).
    pub fn with_store<R>(&self, f: impl FnOnce(&mut MseStore) -> R) -> R {
        let mut g = self.lock();
        let r = f(&mut g.store);
        drop(g);
        self.cv.notify_all();
        r
    }

    /// A new generation at `t` (the previous one goes stale and its readers return).
    pub fn start_generation(&self, t: f64) -> u64 {
        let id = self.lock().store.start_generation(t);
        self.cv.notify_all();
        id
    }

    /// Drop the live generation: every blocked reader returns [`FeedRead::Gone`].
    pub fn stop(&self) {
        for b in &mut self.lock().store.buffers {
            b.stop_generation();
        }
        self.cv.notify_all();
    }

    /// Wake blocked readers so they re-check their cancel flag (takes the lock first, so
    /// a reader between its flag check and its wait cannot miss it).
    pub fn wake(&self) {
        drop(self.lock());
        self.cv.notify_all();
    }

    /// Whether virtual offset `off` of generation `gid` on buffer `bi` can be seeked to
    /// now: inside the init, or inside the still-held part of the run, up to the bytes
    /// known so far.
    pub fn seekable(&self, bi: usize, gid: u64, off: u64) -> bool {
        let g = self.lock();
        let Some(b) = g.store.buffers.get(bi) else { return false };
        let Some(live) = b.generation() else { return false };
        live.id == gid && off <= b.generation_len() && (off < live.init_len() || off >= b.generation_base())
    }

    /// Blocking read of virtual bytes `[off, ..)` of generation `gid` of buffer `bi`.
    /// Waits on the Condvar (in 200 ms slices, to notice `cancel`) while no byte is
    /// there yet and `endOfStream` has not been called.
    pub fn read(&self, bi: usize, gid: u64, off: u64, out: &mut [u8], cancel: &AtomicBool) -> FeedRead {
        let mut g = self.lock();
        loop {
            if cancel.load(Ordering::SeqCst) {
                return FeedRead::Gone;
            }
            let Some(b) = g.store.buffers.get_mut(bi) else { return FeedRead::Gone };
            match b.read_at(gid, off, out) {
                ReadAt::Bytes(n) => return FeedRead::Bytes(n),
                ReadAt::Evicted | ReadAt::Stale => return FeedRead::Gone,
                ReadAt::Wait => {
                    if g.ended {
                        return FeedRead::Eof;
                    }
                    g = self.cv.wait_timeout(g, Duration::from_millis(200)).unwrap_or_else(|e| e.into_inner()).0;
                }
            }
        }
    }
}
