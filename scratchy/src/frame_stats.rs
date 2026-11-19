//! QOBS5: window frame stats. `SCRATCHY_FRAME_STATS=<path>` (or `=1` for stderr)
//! makes the live App append one JSON line per second of wall time (redraws,
//! builds, shaping, paint/present, page JS, input latency, video frames).
//!
//! Off (the variable unset, empty or `0`) the App holds `None`: no file is
//! created and every hook is one `Option` test. On, the aggregator is plain
//! counters plus two fixed-capacity buffers that are reused every second, so
//! there is no allocation per event (one `String` per second for the line).
//! All methods take the time as an argument so the tests can feed timestamps.
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Shape-worker wall time since the last roll-up (the worker is another thread).
static WORKER_NS: AtomicU64 = AtomicU64::new(0);
static WORKER_ON: AtomicBool = AtomicBool::new(false);

/// The shaping worker asks this before timing a pass.
pub(crate) fn worker_timing() -> bool {
    WORKER_ON.load(Ordering::Relaxed)
}

pub(crate) fn add_worker_ns(ns: u64) {
    WORKER_NS.fetch_add(ns, Ordering::Relaxed);
}

/// Cap on input events waiting for a present and on latency samples per second.
const MAX_INPUT: usize = 512;
/// Distinct `<video>` nodes tracked per second.
const MAX_VIDEOS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InputKind {
    Move,
    Button,
    Key,
    Wheel,
}

/// What other threads and the perf recorder counted in the second (the caller
/// gathers it, so `FrameStats::roll` stays a pure function of its inputs).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct External {
    pub(crate) js_ns: u64,
    pub(crate) forced_n: u64,
    pub(crate) forced_ns: u64,
    pub(crate) worker_ns: u64,
    /// QOBS6: page network deliveries of the second: count, p50, p95, max (ns of delay).
    pub(crate) net: (u64, u64, u64, u64),
}

#[derive(Clone, Copy, Default)]
struct VideoCount {
    node: usize,
    rendered: u32,
    painted: u32,
}

enum Sink {
    Stderr,
    File(std::fs::File),
}

pub(crate) struct FrameStats {
    sink: Sink,
    t0: Instant,
    window_start: Instant,
    last_redraw: Option<Instant>,
    redraws: u32,
    damage_presents: u32,
    gap_max: Duration,
    builds: u32,
    build_ns: u64,
    shape_complete: u32,
    shape_incomplete: u32,
    paint_ns: u64,
    present_ns: u64,
    hover_restyles: u32,
    inputs: [u32; 4],
    /// Input events not yet followed by a present (their arrival times).
    pending: [Option<Instant>; MAX_INPUT],
    npending: usize,
    /// Latency samples (microseconds) of the current second.
    lat: Vec<u32>,
    video_wakes: u32,
    videos: [VideoCount; MAX_VIDEOS],
    nvideos: usize,
}

impl FrameStats {
    /// `None` when `spec` is unset, empty or `0` (nothing is created). `1` is
    /// stderr; anything else is a file path, opened for append.
    pub(crate) fn open(spec: Option<&str>, now: Instant) -> Option<FrameStats> {
        let spec = spec?.trim();
        let sink = match spec {
            "" | "0" => return None,
            "1" => Sink::Stderr,
            path => match std::fs::OpenOptions::new().create(true).append(true).open(path) {
                Ok(f) => Sink::File(f),
                Err(e) => {
                    eprintln!("scratchy: SCRATCHY_FRAME_STATS: cannot open {path}: {e}");
                    return None;
                }
            },
        };
        Some(Self::with_sink(sink, now))
    }

    fn with_sink(sink: Sink, now: Instant) -> FrameStats {
        FrameStats {
            sink,
            t0: now,
            window_start: now,
            last_redraw: None,
            redraws: 0,
            damage_presents: 0,
            gap_max: Duration::ZERO,
            builds: 0,
            build_ns: 0,
            shape_complete: 0,
            shape_incomplete: 0,
            paint_ns: 0,
            present_ns: 0,
            hover_restyles: 0,
            inputs: [0; 4],
            pending: [None; MAX_INPUT],
            npending: 0,
            lat: Vec::with_capacity(MAX_INPUT),
            video_wakes: 0,
            videos: [VideoCount::default(); MAX_VIDEOS],
            nvideos: 0,
        }
    }

    /// Reads `SCRATCHY_FRAME_STATS` and turns the worker timing on with it.
    pub(crate) fn from_env() -> Option<FrameStats> {
        let s = Self::open(std::env::var("SCRATCHY_FRAME_STATS").ok().as_deref(), Instant::now())?;
        WORKER_ON.store(true, Ordering::Relaxed);
        Some(s)
    }

    /// The next wall-clock moment a line is due.
    pub(crate) fn next_due(&self) -> Instant {
        self.window_start + Duration::from_secs(1)
    }

    pub(crate) fn redraw_start(&mut self, now: Instant) {
        if let Some(prev) = self.last_redraw {
            self.gap_max = self.gap_max.max(now.saturating_duration_since(prev));
        }
        self.last_redraw = Some(now);
        self.redraws += 1;
    }

    pub(crate) fn build(&mut self, d: Duration) {
        self.builds += 1;
        self.build_ns += d.as_nanos() as u64;
    }

    pub(crate) fn shape_result(&mut self, incomplete: bool) {
        if incomplete {
            self.shape_incomplete += 1;
        } else {
            self.shape_complete += 1;
        }
    }

    pub(crate) fn paint(&mut self, d: Duration) {
        self.paint_ns += d.as_nanos() as u64;
    }

    pub(crate) fn present(&mut self, d: Duration) {
        self.present_ns += d.as_nanos() as u64;
    }

    pub(crate) fn hover_restyle(&mut self) {
        self.hover_restyles += 1;
    }

    pub(crate) fn input(&mut self, kind: InputKind, now: Instant) {
        self.inputs[kind as usize] += 1;
        if self.npending < MAX_INPUT {
            self.pending[self.npending] = Some(now);
            self.npending += 1;
        }
    }

    /// A frame reached the screen at `now`: every waiting input gets its latency.
    pub(crate) fn presented(&mut self, now: Instant) {
        for slot in &mut self.pending[..self.npending] {
            if let Some(t) = slot.take() {
                let us = now.saturating_duration_since(t).as_micros().min(u32::MAX as u128) as u32;
                if self.lat.len() < MAX_INPUT {
                    self.lat.push(us);
                }
            }
        }
        self.npending = 0;
    }

    /// A damage-only present (video rect) happened.
    pub(crate) fn damage_present(&mut self) {
        self.damage_presents += 1;
    }

    pub(crate) fn video_wake(&mut self) {
        self.video_wakes += 1;
    }

    fn video_slot(&mut self, node: usize) -> Option<&mut VideoCount> {
        let n = self.nvideos;
        if let Some(i) = self.videos[..n].iter().position(|v| v.node == node) {
            return Some(&mut self.videos[i]);
        }
        if n < MAX_VIDEOS {
            self.videos[n] = VideoCount { node, ..Default::default() };
            self.nvideos += 1;
            return Some(&mut self.videos[n]);
        }
        None
    }

    /// A new mpv frame was rendered for `node`.
    pub(crate) fn video_rendered(&mut self, node: usize) {
        if let Some(v) = self.video_slot(node) {
            v.rendered += 1;
        }
    }

    /// `node`'s video was painted into a presented frame.
    pub(crate) fn video_painted(&mut self, node: usize) {
        if let Some(v) = self.video_slot(node) {
            v.painted += 1;
        }
    }

    /// When a second has passed, returns the JSON line (without a newline) and
    /// resets the counters. `ext` is what other threads counted meanwhile.
    pub(crate) fn roll(&mut self, now: Instant, ext: External) -> Option<String> {
        let dt = now.saturating_duration_since(self.window_start);
        if dt < Duration::from_secs(1) {
            return None;
        }
        let ms = |ns: u64| ns as f64 / 1e6;
        self.lat.sort_unstable();
        let (lmax, p95) = match self.lat.len() {
            0 => (0, 0),
            n => (self.lat[n - 1], self.lat[((n as f64 * 0.95).ceil() as usize).clamp(1, n) - 1]),
        };
        let oldest = self.pending[..self.npending].iter().flatten().map(|t| now.saturating_duration_since(*t)).max().unwrap_or(Duration::ZERO);
        let mut v = String::new();
        for (i, c) in self.videos[..self.nvideos].iter().enumerate() {
            if i > 0 {
                v.push(',');
            }
            v.push_str(&format!("{{\"node\":{},\"rendered\":{},\"painted\":{}}}", c.node, c.rendered, c.painted));
        }
        let line = format!(
            "{{\"t\":{:.1},\"dt_ms\":{:.0},\"redraws\":{},\"damage_presents\":{},\"gap_max_ms\":{:.1},\
\"builds\":{{\"n\":{},\"ms\":{:.2}}},\"shape\":{{\"complete\":{},\"incomplete\":{},\"worker_ms\":{:.2}}},\
\"paint_ms\":{:.2},\"present_ms\":{:.2},\"js_ms\":{:.2},\"forced\":{{\"n\":{},\"ms\":{:.2}}},\"hover_restyles\":{},\
\"input\":{{\"move\":{},\"button\":{},\"key\":{},\"wheel\":{},\"lat_max_ms\":{:.2},\"lat_p95_ms\":{:.2},\"pending\":{},\"oldest_pending_ms\":{:.1}}},\
\"net\":{{\"n\":{},\"p50_ms\":{:.2},\"p95_ms\":{:.2},\"max_ms\":{:.2}}},\"video\":{{\"wakes\":{},\"v\":[{}]}}}}",
            now.saturating_duration_since(self.t0).as_secs_f64(),
            dt.as_secs_f64() * 1000.0,
            self.redraws,
            self.damage_presents,
            self.gap_max.as_secs_f64() * 1000.0,
            self.builds,
            ms(self.build_ns),
            self.shape_complete,
            self.shape_incomplete,
            ms(ext.worker_ns),
            ms(self.paint_ns),
            ms(self.present_ns),
            ms(ext.js_ns),
            ext.forced_n,
            ms(ext.forced_ns),
            self.hover_restyles,
            self.inputs[0],
            self.inputs[1],
            self.inputs[2],
            self.inputs[3],
            lmax as f64 / 1000.0,
            p95 as f64 / 1000.0,
            self.npending,
            oldest.as_secs_f64() * 1000.0,
            ext.net.0,
            ms(ext.net.1),
            ms(ext.net.2),
            ms(ext.net.3),
            self.video_wakes,
            v,
        );
        self.window_start = now;
        self.redraws = 0;
        self.damage_presents = 0;
        self.gap_max = Duration::ZERO;
        self.builds = 0;
        self.build_ns = 0;
        self.shape_complete = 0;
        self.shape_incomplete = 0;
        self.paint_ns = 0;
        self.present_ns = 0;
        self.hover_restyles = 0;
        self.inputs = [0; 4];
        self.lat.clear();
        self.video_wakes = 0;
        self.nvideos = 0;
        Some(line)
    }

    /// Writes one line to the sink (errors are ignored: stats must never hurt the page).
    pub(crate) fn emit(&mut self, line: &str) {
        match &mut self.sink {
            Sink::Stderr => eprintln!("{line}"),
            Sink::File(f) => {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    /// For the App: gathers `External` from the perf recorder and the worker.
    pub(crate) fn take_external() -> External {
        let (js_ns, forced_n, forced_ns) = crate::perf::take_window();
        External { js_ns, forced_n, forced_ns, worker_ns: WORKER_NS.swap(0, Ordering::Relaxed), net: crate::perf::take_window_net() }
    }

    /// Test sink: stats that write nowhere.
    #[cfg(test)]
    pub(crate) fn for_test(now: Instant) -> FrameStats {
        let f = std::fs::File::options().write(true).open("/dev/null").expect("/dev/null");
        Self::with_sink(Sink::File(f), now)
    }
}
