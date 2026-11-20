//! QV8-0: the permanent performance harness (`--perf-report`, `--perf-corpus`)
//! and the permanent `--budget-ms` debug flag.
//!
//! This is the AGENTS.md exception to "no timing code committed": the same
//! measurement must be repeatable on QuickJS and on V8 (QV8-m). The recorder
//! sits on the call sites that already arm the CPU budget (page scripts, timers,
//! promise jobs, events, observer delivery, network callbacks) and costs one
//! thread-local `Cell<bool>` read when `--perf-report` is off. It is
//! thread-local, not a process atomic, on purpose: scratchy runs all JS on one
//! thread, a thread-local const read is cheaper than an atomic load, and the
//! test suite (many threads, each with its own fake clock) cannot see each
//! other's recorders or budgets.
//!
//! Clocks: "raw" is this thread's CPU time; "budget" is raw minus the forced
//! layouts (`pause_budget_since`), the clock the budget really counts (R37).
//! Spans nest (an observer delivery inside a timer, a script inserted by a
//! network callback); each span records its EXCLUSIVE time, so totals add up
//! with no double counting.
use super::*;
use std::cell::{Cell, RefCell};
use crate::js_engine::{self, engine_info, Ctx, Engine, EngineOpts};
use std::time::{Duration, Instant};

/// The default per-callback CPU budget (the old `CALLBACK_BUDGET`).
pub(crate) const DEFAULT_BUDGET_MS: u64 = 500;

/// What a recorded span was.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    ClassicScript,
    ModuleScript,
    Timer,
    PromiseJob,
    Event,
    Observer,
    NetCallback,
}

/// JSON key of every kind, in report order.
const KINDS: [(Kind, &str); 7] = [
    (Kind::ClassicScript, "classic_script"),
    (Kind::ModuleScript, "module_script"),
    (Kind::Timer, "timer"),
    (Kind::PromiseJob, "promise_job"),
    (Kind::Event, "event"),
    (Kind::Observer, "observer_delivery"),
    (Kind::NetCallback, "net_callback"),
];

struct Open {
    kind: Kind,
    cpu0: u64,
    bud0: u64,
    child_cpu: u64,
    child_bud: u64,
    /// QOBS4: forced layouts that ran directly in this span (count, thread CPU ns).
    forced_n: u32,
    forced_ns: u64,
    /// QOBS4: the callback's function (`name url:line:col`) or script/event label.
    source: Option<String>,
    /// QOBS4b: the slowest event listener that ran directly in this span (ms, `event:<type> <fn>`).
    listener: Option<(f64, String)>,
}

/// QOBS4: how many callbacks `longest` keeps.
pub(crate) const LONGEST_N: usize = 20;

/// QOBS4: one of the longest callbacks of the run.
struct Longest {
    raw_ns: u64,
    bud_ns: u64,
    kind: Kind,
    forced_n: u32,
    forced_ns: u64,
    at_ms: f64,
    source: String,
    /// Tie-break so the order is total (older first).
    seq: u64,
}

impl PartialEq for Longest {
    fn eq(&self, o: &Self) -> bool {
        (self.raw_ns, o.seq) == (o.raw_ns, self.seq)
    }
}
impl Eq for Longest {}
impl PartialOrd for Longest {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Longest {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.raw_ns, o.seq).cmp(&(o.raw_ns, self.seq))
    }
}

#[derive(Default)]
struct PerKind {
    /// `(budget_ns, raw_ns)` of every finished span.
    samples: Vec<(u64, u64)>,
    kills: u64,
}

#[derive(Default)]
struct Rec {
    stack: Vec<Open>,
    kinds: [PerKind; 7],
    forced: Vec<u64>,
    compile_ns: u64,
    /// `Kind` that promise jobs run under right now (a module entry's drain).
    job_kind: Option<Kind>,
    start: Option<(Instant, u64)>,
    marks: Vec<(&'static str, f64, u64)>,
    /// QOBS4: min-heap (smallest on top) of the `LONGEST_N` longest callbacks; one allocation, never grows.
    longest: std::collections::BinaryHeap<std::cmp::Reverse<Longest>>,
    seq: u64,
    /// QOBS3: what the forced layouts were made of.
    fl: ForcedAgg,
    /// QOBS5 window mode: no per-span samples, only these running totals.
    window: bool,
    win_js_ns: u64,
    win_forced_n: u64,
    win_forced_ns: u64,
    /// QOBS6: delay (ns) between a page network result being complete and `deliver_net` running it.
    net: Vec<u64>,
    /// QOBS5c window mode: the longest callback of the current second (self CPU ns, kind, source).
    win_longest: Option<(u64, Kind, String)>,
}

/// QOBS3: the phases of a forced layout, in report order.
#[derive(Clone, Copy)]
pub(crate) enum Phase {
    CssCollect,
    RuleIndex,
    PrecomputeGenerated,
    BuildChildren,
    MeasureDocument,
    GlyphWalk,
    RectWalk,
}

const PHASE_NAMES: [&str; 7] = ["css_collect", "rule_index", "precompute_generated", "build_children", "measure_document", "glyph_walk", "rect_walk"];

/// QOBS3: totals over every forced layout of the run.
#[derive(Default)]
struct ForcedAgg {
    phase_ns: [u64; 7],
    style_resolves: u64,
    elements: u64,
    no_change: u64,
    /// `[io, script]` (count, thread CPU ns).
    reason: [(u64, u64); 2],
    /// `"function url:line:col"` -> (count, thread CPU ns).
    callers: std::collections::HashMap<String, (u64, u64)>,
}

/// QOBS3: how many callers `by_caller` lists.
const CALLERS_N: usize = 10;

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static BUDGET_MS: Cell<u64> = const { Cell::new(DEFAULT_BUDGET_MS) };
    static REC: RefCell<Rec> = RefCell::new(Rec::default());
    /// QOBS3: a forced layout is running under the recorder (gates the phase timers and counters).
    static FL_ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// QOBS3: scratchy's IntersectionObserver evaluation is running (a geometry read now is `io`).
    static IO_FLAG: Cell<bool> = const { Cell::new(false) };
    /// QOBS3: the page caller of the layout about to run (set by the native, taken by `refresh`).
    static CALLER: RefCell<Option<String>> = const { RefCell::new(None) };
    /// QOBS7: memory probes registered by per-engine caches (`name -> () -> (entries, approx bytes)`).
    static PROBES: RefCell<Vec<(&'static str, Probe)>> = const { RefCell::new(Vec::new()) };
}

type Probe = Box<dyn Fn() -> (usize, usize)>;

/// QOBS7: registers a cache that lives in a per-engine `Rc` (not a thread-local) so the report
/// can size it at the end of the run. A no-op unless the recorder is on, so a normal run pays nothing.
pub(crate) fn register_probe(name: &'static str, f: impl Fn() -> (usize, usize) + 'static) {
    if enabled() {
        PROBES.with(|p| p.borrow_mut().push((name, Box::new(f))));
    }
}

/// Peak resident set size in bytes: `VmHWM` of `/proc/self/status` (kB; the process high-water
/// mark, whole process), never below the current `VmRSS`. Falls back to `getrusage` ru_maxrss
/// (kilobytes on Linux) when /proc is unreadable; 0 on failure.
pub(crate) fn peak_rss_bytes() -> u64 {
    let kb = |key: &str| -> Option<u64> {
        let s = std::fs::read_to_string("/proc/self/status").ok()?;
        s.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())).map(|k| k * 1024)
    };
    let hwm = kb("VmHWM:").unwrap_or_else(|| {
        // SAFETY: getrusage fills the zeroed struct we pass.
        unsafe {
            let mut u: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut u) != 0 {
                return 0;
            }
            u64::try_from(u.ru_maxrss).unwrap_or(0) * 1024
        }
    });
    hwm.max(rss_bytes())
}

/// QMEM3c: glibc allocator totals `(in_use, free_kept)` in bytes (`mallinfo2`: arena + mmapped
/// chunks in use, and free bytes the allocator holds but has not returned); `None` off glibc.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn malloc_stats() -> Option<(u64, u64)> {
    // SAFETY: mallinfo2 only reads allocator state and returns a plain struct.
    let m = unsafe { libc::mallinfo2() };
    Some(((m.uordblks + m.hblkhd) as u64, m.fordblks as u64))
}
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn malloc_stats() -> Option<(u64, u64)> {
    None
}

/// QMEM3b: `(nodes, approximate bytes)` of a parsed DOM: per node the rcdom `Node` plus its
/// children Vec slot, per element the attribute Vec with each name/value string, per text its
/// capacity. Counts template contents; ignores allocator rounding and the interned atoms.
pub(crate) fn dom_mem(root: &markup5ever_rcdom::Handle) -> (usize, usize) {
    use markup5ever_rcdom::NodeData;
    let (mut nodes, mut bytes) = (0usize, 0usize);
    let mut stack = vec![root.clone()];
    while let Some(n) = stack.pop() {
        nodes += 1;
        bytes += std::mem::size_of::<markup5ever_rcdom::Node>() + std::mem::size_of::<markup5ever_rcdom::Handle>();
        match &n.data {
            NodeData::Element { attrs, template_contents, .. } => {
                for a in attrs.borrow().iter() {
                    bytes += std::mem::size_of::<html5ever::Attribute>() + a.value.len();
                }
                if let Some(t) = template_contents.borrow().as_ref() {
                    stack.push(t.clone());
                }
            }
            NodeData::Text { contents } => bytes += contents.borrow().len(),
            NodeData::Comment { contents } => bytes += contents.len(),
            _ => {}
        }
        stack.extend(n.children.borrow().iter().cloned());
    }
    (nodes, bytes)
}

/// QOBS7: the `memory` member of the report, taken once at the end of the run. Byte figures of
/// caches are ESTIMATES (string and Vec capacities plus fixed per-entry overheads), not allocator
/// truth; `engine` is the page's engine (None when the page had no scripts).
pub(crate) fn memory_json(engine: Option<&crate::js::JsEngine>) -> String {
    let t0 = std::time::Instant::now();
    let mut caches: Vec<(&'static str, (usize, usize))> = Vec::new();
    let mut add = |name: &'static str, v: (usize, usize)| match caches.iter_mut().find(|c| c.0 == name) {
        Some(c) => {
            c.1.0 += v.0;
            c.1.1 += v.1;
        }
        None => caches.push((name, v)),
    };
    let (interner, memo) = crate::css::custom_props_mem();
    add("custom_props_interner", interner);
    add("custom_props_memo", memo);
    add("shape_memo", crate::layout::shape_memo_mem());
    let (rules, index) = crate::style::rules_cache_mem();
    add("rules_cache", rules);
    add("rule_index_hover_keys", index);
    add("svg_sized_cache", crate::images::svg_sized_cache_mem());
    // QMEM3b: the page's CSS text kept for getComputedStyle (EXTRA_CSS), by capacity.
    add("js_extra_css", crate::js_style::extra_css_mem());
    // The perf run has no App, so there is no image cache to size.
    add("image_cache", (0, 0));
    for n in ["style_cache", "mse_store", "geom_rects", "geom_glyphs", "media_elements", "dom", "report_html", "report_scripts", "report_css"] {
        add(n, (0, 0));
    }
    PROBES.with(|p| {
        for (name, f) in p.borrow().iter() {
            add(name, f());
        }
    });
    let players = caches.iter().find(|c| c.0 == "media_elements").map_or(0, |c| c.1.0);
    caches.retain(|c| c.0 != "media_elements");
    let total: usize = caches.iter().map(|c| c.1.1).sum();
    let body: Vec<String> = caches.iter().map(|(n, (e, b))| format!("\"{n}\":{{\"entries\":{e},\"bytes\":{b}}}")).collect();
    let (heap, js) = match engine {
        Some(e) => {
            let (u, t, x) = e.eng.heap_stats();
            (u.max(0), format!("{{\"used\":{},\"total\":{},\"external\":{}}}", u.max(0), t.max(0), x.map_or("null".to_string(), |x| x.max(0).to_string())))
        }
        None => (0, "{\"used\":null,\"total\":null,\"external\":null}".to_string()),
    };
    let (seg, seg_bytes) = crate::stack::spare_segments();
    let malloc = malloc_stats();
    // QFLK1: one rss sample; the peak is never below it (other threads may free between two /proc reads).
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |key: &str| status.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())).map_or(0, |k| k * 1024);
    // RssAnon: anonymous resident bytes (heaps, stacks, V8 pages, no mapped files); `rss_anon_bytes - malloc.free_kept` is the live anonymous memory `explained_bytes` is compared with.
    let (rss, rss_anon) = (field("VmRSS:"), field("RssAnon:"));
    let peak = peak_rss_bytes().max(rss);
    format!(
        "\"memory\":{{\"rss_bytes\":{},\"rss_anon_bytes\":{},\"peak_rss_bytes\":{},\"js_heap\":{},\"caches\":{{{}}},\"caches_bytes\":{},\"explained_bytes\":{},\"walk_us\":{},\"malloc\":{},\"stack_segments\":{{\"kept\":{},\"mapped_bytes\":{}}},\"media_players\":{},\"shape_memo_lookups\":{{\"hits\":{},\"misses\":{}}}}}",
        rss,
        rss_anon,
        peak,
        js,
        body.join(","),
        total,
        total as i64 + heap,
        t0.elapsed().as_micros(),
        malloc.map_or("null".to_string(), |(u, f)| format!("{{\"in_use\":{u},\"free_kept\":{f}}}")),
        seg,
        seg_bytes,
        players,
        crate::layout::shape_memo_lookups().0,
        crate::layout::shape_memo_lookups().1
    )
}

/// QOBS3: a phase timer; adds its thread CPU to the phase when dropped. `None` (free) unless a forced layout is recorded.
pub(crate) struct PhaseTimer(Option<(Phase, u64)>);

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        if let Some((p, c0)) = self.0 {
            let d = thread_cpu_nanos().saturating_sub(c0);
            REC.with(|r| r.borrow_mut().fl.phase_ns[p as usize] += d);
        }
    }
}

#[inline]
pub(crate) fn phase(p: Phase) -> PhaseTimer {
    PhaseTimer(if FL_ACTIVE.with(Cell::get) { Some((p, thread_cpu_nanos())) } else { None })
}

/// QOBS3: one `resolve_style_for` call (`element`: not a pseudo-element).
#[inline]
pub(crate) fn count_style_resolve(element: bool) {
    if FL_ACTIVE.with(Cell::get) {
        REC.with(|r| {
            let mut r = r.borrow_mut();
            r.fl.style_resolves += 1;
            r.fl.elements += element as u64;
        });
    }
}

/// QOBS3: brackets one forced layout. `begin_forced` is true when the recorder wants the breakdown.
pub(crate) fn begin_forced() -> bool {
    let on = enabled();
    FL_ACTIVE.with(|a| a.set(on));
    on
}

pub(crate) fn end_forced() {
    FL_ACTIVE.with(|a| a.set(false));
}

/// QOBS3: the IO prelude's evaluation brackets its geometry reads with this.
pub(crate) fn set_io(on: bool) {
    IO_FLAG.with(|f| f.set(on));
}

pub(crate) fn in_io() -> bool {
    IO_FLAG.with(Cell::get)
}

pub(crate) fn set_caller(c: Option<String>) {
    CALLER.with(|x| *x.borrow_mut() = c);
}

/// QOBS3: one finished forced layout: reason, caller and whether its rects equalled the previous layout's.
pub(crate) fn forced_detail(cpu_ns: u64, no_change: bool) {
    let caller = CALLER.with(|x| x.borrow_mut().take());
    let io = IO_FLAG.with(Cell::get);
    REC.with(|r| {
        let mut r = r.borrow_mut();
        let f = &mut r.fl;
        f.no_change += no_change as u64;
        let e = &mut f.reason[if io { 0 } else { 1 }];
        e.0 += 1;
        e.1 += cpu_ns;
        if let Some(c) = caller {
            let e = f.callers.entry(c).or_default();
            e.0 += 1;
            e.1 += cpu_ns;
        }
    });
}

#[inline]
pub(crate) fn enabled() -> bool {
    ON.with(Cell::get)
}

/// Turns the recorder on (fresh state) and starts the "time to ..." clocks.
pub(crate) fn start() {
    incr_reset();
    REC.with(|r| {
        *r.borrow_mut() = Rec { start: Some((Instant::now(), thread_cpu_nanos())), ..Default::default() };
    });
    PROBES.with(|p| p.borrow_mut().clear());
    ON.with(|o| o.set(true));
}

/// QOBS5: the live window's frame stats turn the recorder on in window mode:
/// spans keep only running totals (page JS, forced layouts), read by `take_window`.
pub(crate) fn start_window() {
    REC.with(|r| *r.borrow_mut() = Rec { window: true, ..Default::default() });
    ON.with(|o| o.set(true));
}

/// `(page js ns, forced layouts, forced ns)` since the last call (window mode).
pub(crate) fn take_window() -> (u64, u64, u64) {
    REC.with(|r| {
        let mut r = r.borrow_mut();
        let out = (r.win_js_ns, r.win_forced_n, r.win_forced_ns);
        r.win_js_ns = 0;
        r.win_forced_n = 0;
        r.win_forced_ns = 0;
        out
    })
}

/// QOBS5c: `(caller, count, ns)`.
pub(crate) type Caller = (String, u64, u64);
/// QOBS5c: `(self CPU ns, kind name, source)`.
pub(crate) type LongestCb = (u64, &'static str, String);

/// QOBS5c (window mode): the top `n` forced-layout callers of the second as `(caller, count, ns)`
/// (slowest first) and the longest callback `(ns, kind name, source)`; both reset.
pub(crate) fn take_window_callers(n: usize) -> (Vec<Caller>, Option<LongestCb>) {
    REC.with(|r| {
        let mut r = r.borrow_mut();
        let mut v: Vec<Caller> = std::mem::take(&mut r.fl.callers).into_iter().map(|(k, (c, ns))| (k, c, ns)).collect();
        v.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)).then(a.0.cmp(&b.0)));
        v.truncate(n);
        let longest = r.win_longest.take().map(|(ns, k, s)| (ns, KINDS.iter().find(|(x, _)| *x == k).map_or("", |x| x.1), s));
        (v, longest)
    })
}

/// QOBS6: one page network result was delivered `delay_ns` after it was complete (the
/// `AppEvent::JsNet` was sent). Window mode keeps the samples of the current second only.
pub(crate) fn net_delivery(delay_ns: u64) {
    if enabled() {
        REC.with(|r| {
            let mut r = r.borrow_mut();
            if r.net.len() < NET_CAP {
                r.net.push(delay_ns);
            }
        });
    }
}

/// Cap on stored delivery samples (a window second, or a whole report run).
const NET_CAP: usize = 100_000;

/// QOBS6 (window mode): `(count, p50 ns, p95 ns, max ns)` of the deliveries since the last call.
pub(crate) fn take_window_net() -> (u64, u64, u64, u64) {
    REC.with(|r| {
        let mut v = std::mem::take(&mut r.borrow_mut().net);
        v.sort_unstable();
        let out = (v.len() as u64, pct(&v, 0.5), pct(&v, 0.95), v.last().copied().unwrap_or(0));
        v.clear();
        r.borrow_mut().net = v;
        out
    })
}

/// Turns the recorder off (tests, and the end of a report).
pub(crate) fn stop() {
    ON.with(|o| o.set(false));
}

/// The per-callback CPU budget (`--budget-ms`, default 500 ms).
pub(crate) fn callback_budget() -> Duration {
    Duration::from_millis(BUDGET_MS.with(Cell::get))
}

/// The page-load budget for one script: the old 5 s, or the callback budget
/// when `--budget-ms` asks for more (so "true cost" runs are not cut at 5 s).
pub(crate) fn script_budget() -> Duration {
    SCRIPT_BUDGET.max(callback_budget())
}

/// One promise drain stops starting jobs after 4 callback budgets (2 s by default).
pub(crate) fn drain_cap() -> Duration {
    callback_budget() * 4
}

pub(crate) fn set_budget_ms(ms: u64) {
    BUDGET_MS.with(|b| b.set(ms));
}

/// Largest accepted `--budget-ms` (one hour).
const MAX_BUDGET_MS: u64 = 3_600_000;

/// `--budget-ms N`: `Ok(None)` when absent, `Err` for a missing or invalid
/// value (a positive whole number of milliseconds, at most one hour).
pub(crate) fn parse_budget_ms(args: &[String]) -> Result<Option<u64>, String> {
    let mut out = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--budget-ms" {
            let v = args.get(i + 1).ok_or("--budget-ms needs a number of milliseconds")?;
            let n: u64 = v.parse().map_err(|_| format!("--budget-ms: `{v}` is not a whole number of milliseconds"))?;
            if n == 0 || n > MAX_BUDGET_MS {
                return Err(format!("--budget-ms: {n} is out of range (1..={MAX_BUDGET_MS})"));
            }
            out = Some(n);
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(out)
}

/// Called once from `run`: installs `--budget-ms` for every mode.
pub(crate) fn init_from_process(args: &[String]) {
    match parse_budget_ms(args) {
        Ok(Some(n)) => set_budget_ms(n),
        Ok(None) => {}
        Err(e) => {
            diag!("scratchy: {e}");
            std::process::exit(2);
        }
    }
}

/// An open span; `Span(None)` (recorder off or nothing to record) is inert.
pub(crate) struct Span(Option<usize>);

pub(crate) fn begin(kind: Option<Kind>) -> Span {
    let Some(kind) = kind else { return Span(None) };
    if !enabled() {
        return Span(None);
    }
    REC.with(|r| {
        let mut r = r.borrow_mut();
        r.stack.push(Open { kind, cpu0: thread_cpu_nanos(), bud0: script_clock_nanos(), child_cpu: 0, child_bud: 0, forced_n: 0, forced_ns: 0, source: None, listener: None });
        Span(Some(r.stack.len() - 1))
    })
}

/// Closes `span` (and drops any child an uncatchable budget kill left open).
pub(crate) fn end(span: Span, killed: bool) {
    let Some(depth) = span.0 else { return };
    REC.with(|r| {
        let mut r = r.borrow_mut();
        if r.stack.len() <= depth {
            return;
        }
        r.stack.truncate(depth + 1);
        let Some(o) = r.stack.pop() else { return };
        let cpu = thread_cpu_nanos().saturating_sub(o.cpu0);
        let bud = script_clock_nanos().saturating_sub(o.bud0);
        if r.window {
            let c = cpu.saturating_sub(o.child_cpu);
            r.win_js_ns += c;
            if r.win_longest.as_ref().is_none_or(|l| c > l.0) {
                let source = match (o.source, o.listener) {
                    (s, Some((_, l))) if s.as_deref().is_none_or(is_prelude_source) => l,
                    (s, _) => s.unwrap_or_default(),
                };
                r.win_longest = Some((c, o.kind, source));
            }
            if let Some(p) = r.stack.last_mut() {
                p.child_cpu += cpu;
                p.child_bud += bud;
            }
            return;
        }
        let idx = KINDS.iter().position(|(k, _)| *k == o.kind).unwrap_or(0);
        let (b, c) = (bud.saturating_sub(o.child_bud), cpu.saturating_sub(o.child_cpu));
        r.kinds[idx].samples.push((b, c));
        if killed {
            r.kinds[idx].kills += 1;
        }
        r.seq += 1;
        let seq = r.seq;
        if r.longest.len() < LONGEST_N || r.longest.peek().is_some_and(|m| c > m.0.raw_ns) {
            if r.longest.capacity() == 0 {
                r.longest.reserve(LONGEST_N + 1);
            }
            if r.longest.len() >= LONGEST_N {
                r.longest.pop();
            }
            let at_ms = r.start.map_or(0.0, |(t0, _)| t0.elapsed().as_secs_f64() * 1000.0);
            r.longest.push(std::cmp::Reverse(Longest {
                raw_ns: c,
                bud_ns: b,
                kind: o.kind,
                forced_n: o.forced_n,
                forced_ns: o.forced_ns,
                at_ms,
                source: match (o.source, o.listener) {
                    // QOBS4b: a scratchy wrapper (media pump, rIC shim, ...) or nothing: name the slowest listener.
                    (s, Some((_, l))) if s.as_deref().is_none_or(is_prelude_source) => {
                        let _ = s;
                        l
                    }
                    (s, _) => s.unwrap_or_default(),
                },
                seq,
            }));
        }
        if let Some(p) = r.stack.last_mut() {
            p.child_cpu += cpu;
            p.child_bud += bud;
        }
    });
}

/// Drops `span` without recording it (an idle poll).
pub(crate) fn discard(span: Span) {
    let Some(depth) = span.0 else { return };
    REC.with(|r| r.borrow_mut().stack.truncate(depth));
}

/// Observer deliveries run inside JS (`__perf_b`/`__perf_e`, installed only
/// while the recorder is on): open and close an `Observer` span.
pub(crate) fn observer_begin() {
    let _ = begin(Some(Kind::Observer));
}

pub(crate) fn observer_end() {
    let depth = REC.with(|r| r.borrow().stack.iter().rposition(|o| o.kind == Kind::Observer));
    if let Some(d) = depth {
        end(Span(Some(d)), false);
    }
}

/// QOBS4: names the innermost open span of `kind` (`name url:line:col`, a script name, an event).
/// Only the first name of a span sticks (a timer's function, not what it later evals).
pub(crate) fn set_source(kind: Kind, source: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    REC.with(|r| {
        let mut r = r.borrow_mut();
        if let Some(o) = r.stack.iter_mut().rev().find(|o| o.kind == kind)
            && o.source.is_none()
        {
            o.source = Some(source());
        }
    });
}

/// QOBS4b: true when `source` (`name url:line:col`) points into one of scratchy's own prelude
/// scripts, i.e. it names a wrapper and not the page's function.
fn is_prelude_source(source: &str) -> bool {
    let Some(loc) = source.rsplit(' ').next() else { return false };
    let mut it = loc.rsplitn(3, ':');
    let (_c, _l, url) = (it.next(), it.next(), it.next());
    url.is_some_and(|u| crate::js::PRELUDE_SCRIPTS.contains(&u))
}

/// QOBS4b: this thread's CPU clock in ms (the recorder's own clock, fake-clock aware).
pub(crate) fn thread_cpu_ms() -> f64 {
    thread_cpu_nanos() as f64 / 1e6
}

/// QOBS4b: an event listener took `ms`; remembered on the innermost open callback span when it is the
/// slowest so far (`source` is only built then). `__perf_lis` is installed only while the recorder is on.
pub(crate) fn note_listener(event_type: &str, ms: f64, source: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    REC.with(|r| {
        let mut r = r.borrow_mut();
        if let Some(o) = r.stack.iter_mut().rev().find(|o| o.kind != Kind::Observer)
            && o.listener.as_ref().is_none_or(|(m, _)| ms > *m)
        {
            o.listener = Some((ms, format!("event:{event_type} {}", source())));
        }
    });
}

/// Promise jobs run as `kind` (a module entry's drain) until reset with `None`.
pub(crate) fn set_job_kind(kind: Option<Kind>) {
    if enabled() {
        REC.with(|r| r.borrow_mut().job_kind = kind);
    }
}

pub(crate) fn job_kind() -> Kind {
    REC.with(|r| r.borrow().job_kind).unwrap_or(Kind::PromiseJob)
}

/// One forced layout of `cpu_ns` thread CPU (called from `pause_budget_since`).
pub(crate) fn forced_layout(cpu_ns: u64) {
    if enabled() {
        REC.with(|r| {
            let mut r = r.borrow_mut();
            if r.window {
                r.win_forced_n += 1;
                r.win_forced_ns += cpu_ns;
            } else {
                r.forced.push(cpu_ns);
            }
            if let Some(o) = r.stack.last_mut() {
                o.forced_n += 1;
                o.forced_ns += cpu_ns;
            }
        });
    }
}

/// CPU spent compiling one page script.
pub(crate) fn add_compile(cpu_ns: u64) {
    if enabled() {
        REC.with(|r| r.borrow_mut().compile_ns += cpu_ns);
    }
}

/// A named point on the page-load timeline: wall ms and thread CPU ns since `start`.
pub(crate) fn mark(name: &'static str) {
    if !enabled() {
        return;
    }
    REC.with(|r| {
        let mut r = r.borrow_mut();
        let Some((t0, c0)) = r.start else { return };
        let (w, c) = (t0.elapsed().as_secs_f64() * 1000.0, thread_cpu_nanos().saturating_sub(c0));
        r.marks.push((name, w, c));
    });
}

/// QINC1a (R60 section 10): the incremental engine's counters, reported as the `incr` object of
/// `--perf-report`. Every one is zero until the engine lands (QINC1f onward); each stage's gate reads
/// them. Plain thread-local cells: bumping one costs a few ns whether or not the recorder is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub(crate) enum Incr {
    /// Incremental updates run (restyle + box update + relayout, once per layout).
    Updates,
    /// Elements re-matched (RESTYLE) / re-cascaded from a stored match (RECASCADE).
    RestyledMatched,
    RestyledCascaded,
    /// Boxes rebuilt by the builder / re-measured by layout.
    BoxesRebuilt,
    BoxesRemeasured,
    /// Fragment cache: hits, and misses by reason.
    CacheHits,
    MissDirty,
    MissKey,
    MissFloats,
    MissNotCacheable,
    MissNoOld,
    /// `SCRATCHY_INCR_CHECK=1` differences between the incremental and the from-scratch result.
    CheckMismatch,
}

const INCR_NAMES: [&str; 12] = [
    "updates",
    "restyled_matched",
    "restyled_cascaded",
    "boxes_rebuilt",
    "boxes_remeasured",
    "cache_hits",
    "miss_dirty",
    "miss_key",
    "miss_floats",
    "miss_not_cacheable",
    "miss_no_old",
    "check_mismatch",
];

/// Bottom-up relayout stops are counted per tree level (the last bucket takes every deeper level).
pub(crate) const INCR_LEVELS: usize = 8;

#[derive(Default)]
struct IncrCounters {
    flat: [u64; 12],
    stops_by_level: [u64; INCR_LEVELS],
    /// Shaping-worker fallbacks by reason (name -> count), in first-seen order.
    worker_fallbacks: Vec<(&'static str, u64)>,
}

thread_local! {
    static INCR: RefCell<IncrCounters> = RefCell::new(IncrCounters::default());
}

/// Adds `n` to a QINC1 counter.
#[allow(dead_code)]
pub(crate) fn incr_add(c: Incr, n: u64) {
    INCR.with(|i| i.borrow_mut().flat[c as usize] += n);
}

/// Current value of a QINC1 counter (tests, gates).
#[allow(dead_code)]
pub(crate) fn incr_get(c: Incr) -> u64 {
    INCR.with(|i| i.borrow().flat[c as usize])
}

/// A bottom-up relayout stopped `level` levels below the root.
#[allow(dead_code)]
pub(crate) fn incr_stop_at(level: usize) {
    INCR.with(|i| i.borrow_mut().stops_by_level[level.min(INCR_LEVELS - 1)] += 1);
}

/// The shaping worker could not serve a request incrementally, for `reason`.
#[allow(dead_code)]
pub(crate) fn incr_worker_fallback(reason: &'static str) {
    INCR.with(|i| {
        let mut i = i.borrow_mut();
        match i.worker_fallbacks.iter_mut().find(|r| r.0 == reason) {
            Some(r) => r.1 += 1,
            None => i.worker_fallbacks.push((reason, 1)),
        }
    });
}

fn incr_reset() {
    INCR.with(|i| *i.borrow_mut() = IncrCounters::default());
}

/// The `incr` object (a JSON object text).
pub(crate) fn incr_json() -> String {
    INCR.with(|i| {
        let i = i.borrow();
        let mut parts: Vec<String> = INCR_NAMES.iter().zip(i.flat).map(|(n, v)| format!("\"{n}\":{v}")).collect();
        let levels: Vec<String> = i.stops_by_level.iter().map(u64::to_string).collect();
        parts.push(format!("\"bottom_up_stops_by_level\":[{}]", levels.join(",")));
        let wf: Vec<String> = i.worker_fallbacks.iter().map(|(r, n)| format!("{}:{n}", json_str(r))).collect();
        parts.push(format!("\"worker_fallbacks\":{{{}}}", wf.join(",")));
        format!("{{{}}}", parts.join(","))
    })
}

fn ms(ns: u64) -> f64 {
    (ns as f64 / 1e6 * 1000.0).round() / 1000.0
}

fn pct(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

fn stats_json(mut v: Vec<u64>) -> String {
    v.sort_unstable();
    let total: u64 = v.iter().sum();
    format!(
        "{{\"total_ms\":{},\"max_ms\":{},\"p50_ms\":{},\"p95_ms\":{}}}",
        ms(total),
        ms(v.last().copied().unwrap_or(0)),
        ms(pct(&v, 0.5)),
        ms(pct(&v, 0.95))
    )
}

/// JSON string literal.
pub(crate) fn json_str(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The `callbacks`, kill, forced-layout, compile and timeline members of the report.
pub(crate) fn body_json() -> String {
    REC.with(|r| {
        let r = r.borrow();
        let mut kinds = Vec::new();
        let mut kills = 0;
        for (i, (_, name)) in KINDS.iter().enumerate() {
            let pk = &r.kinds[i];
            kills += pk.kills;
            kinds.push(format!(
                "{}:{{\"count\":{},\"kills\":{},\"budget_clock\":{},\"raw_cpu\":{}}}",
                json_str(name),
                pk.samples.len(),
                pk.kills,
                stats_json(pk.samples.iter().map(|s| s.0).collect()),
                stats_json(pk.samples.iter().map(|s| s.1).collect())
            ));
        }
        let mut lg: Vec<&Longest> = r.longest.iter().map(|x| &x.0).collect();
        lg.sort_by(|a, b| b.raw_ns.cmp(&a.raw_ns).then(a.seq.cmp(&b.seq)));
        let longest: Vec<String> = lg
            .iter()
            .map(|l| {
                let name = KINDS.iter().find(|(k, _)| *k == l.kind).map_or("", |k| k.1);
                format!(
                    "{{\"kind\":{},\"raw_ms\":{},\"budget_ms\":{},\"forced_layouts\":{},\"forced_ms\":{},\"at_ms\":{},\"source\":{}}}",
                    json_str(name),
                    ms(l.raw_ns),
                    ms(l.bud_ns),
                    l.forced_n,
                    ms(l.forced_ns),
                    (l.at_ms * 1000.0).round() / 1000.0,
                    json_str(&l.source)
                )
            })
            .collect();
        let fl = &r.fl;
        let phases: Vec<String> = PHASE_NAMES.iter().zip(fl.phase_ns).map(|(n, ns)| format!("\"{n}\":{{\"total_ms\":{}}}", ms(ns))).collect();
        let reason = |i: usize| format!("{{\"count\":{},\"total_ms\":{}}}", fl.reason[i].0, ms(fl.reason[i].1));
        let mut callers: Vec<(&String, &(u64, u64))> = fl.callers.iter().collect();
        callers.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(b.1.0.cmp(&a.1.0)).then(a.0.cmp(b.0)));
        let by_caller: Vec<String> = callers
            .iter()
            .take(CALLERS_N)
            .map(|(k, v)| format!("{{\"caller\":{},\"count\":{},\"total_ms\":{}}}", json_str(k), v.0, ms(v.1)))
            .collect();
        let forced_extra = format!(
            "\"phases\":{{{}}},\"style_resolves\":{},\"elements\":{},\"no_change\":{},\"by_reason\":{{\"io\":{},\"script\":{}}},\"by_caller\":[{}]",
            phases.join(","),
            fl.style_resolves,
            fl.elements,
            fl.no_change,
            reason(0),
            reason(1),
            by_caller.join(",")
        );
        let ftotal: u64 = r.forced.iter().sum();
        let fmax = r.forced.iter().copied().max().unwrap_or(0);
        let mark = |name: &str| r.marks.iter().find(|m| m.0 == name).map(|m| (m.1, m.2)).unwrap_or((0.0, 0));
        let wall = |x: f64| (x * 1000.0).round() / 1000.0;
        let (fw, fc) = mark("scripts_ready");
        let (dw, dc) = mark("dom_content_loaded");
        let (lw, lc) = mark("load");
        let (_, es) = mark("engine_start");
        let (_, ee) = mark("engine_ready");
        let net = {
            let mut v = r.net.clone();
            v.sort_unstable();
            format!("{{\"count\":{},\"p50_ms\":{},\"p95_ms\":{},\"max_ms\":{}}}", v.len(), ms(pct(&v, 0.5)), ms(pct(&v, 0.95)), ms(v.last().copied().unwrap_or(0)))
        };
        format!(
            "\"callbacks\":{{{}}},\"longest\":[{}],\"budget_kills\":{},\"forced_layout\":{{\"count\":{},\"total_ms\":{},\"max_ms\":{},{}}},\
             \"net_delivery\":{},\"compile_cpu_ms\":{},\"engine_startup_cpu_ms\":{},\
             \"load\":{{\"fetch_wall_ms\":{},\"fetch_cpu_ms\":{},\"dom_content_loaded_wall_ms\":{},\"dom_content_loaded_cpu_ms\":{},\"load_wall_ms\":{},\"load_cpu_ms\":{}}},\
             \"incr\":{}",
            kinds.join(","),
            longest.join(","),
            kills,
            r.forced.len(),
            ms(ftotal),
            ms(fmax),
            forced_extra,
            net,
            ms(r.compile_ns),
            ms(ee.saturating_sub(es)),
            wall(fw),
            ms(fc),
            wall(dw),
            ms(dc),
            wall(lw),
            ms(lc),
            incr_json()
        )
    })
}

/// Resident set size of this process in bytes (0 when unreadable).
pub(crate) fn rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("VmRSS:").map(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().unwrap_or(0) * 1024)))
        .unwrap_or(0)
}

/// Commit the binary was built from (build.rs), "unknown" outside a git checkout.
pub(crate) fn commit() -> &'static str {
    option_env!("SCRATCHY_GIT_COMMIT").filter(|c| !c.is_empty()).unwrap_or("unknown")
}

/// Markers of a bot or JS challenge page. The harness never tries to pass one;
/// it only reports that the page it measured was one.
fn looks_like_challenge(html: &str) -> bool {
    // A challenge page is a small stub; a megabyte article that merely mentions CAPTCHAs is not one.
    if html.len() > 150_000 {
        return false;
    }
    let h = html.to_ascii_lowercase();
    [
        "just a moment...",
        "cf-challenge",
        "challenge-platform",
        "captcha",
        "bots use duckduckgo",
        "unusual traffic",
        "verify you are human",
        "are you a robot",
        "attention required",
        "px-captcha",
    ]
    .iter()
    .any(|m| h.contains(m))
}

/// Options of one `--perf-report` run.
pub(crate) struct ReportOpts {
    pub(crate) socket: Option<std::path::PathBuf>,
    pub(crate) seconds: f64,
    pub(crate) target: String,
}

/// Default `--seconds`: how long the timers and network are pumped after `load`.
pub(crate) const DEFAULT_SECONDS: f64 = 5.0;

/// The fixed envelope members of the report (everything but the recorder's).
fn envelope(opts: &ReportOpts, outcome: &str, challenge: bool, page_bytes: usize, title: &str) -> String {
    let (name, version) = engine_info();
    format!(
        "\"schema\":2,\"engine\":{},\"engine_version\":{},\"commit\":{},\"target\":{},\"budget_ms\":{},\"seconds\":{},\
         \"outcome\":{},\"challenge\":{},\"page_bytes\":{},\"title\":{}",
        json_str(name),
        json_str(&version),
        json_str(commit()),
        json_str(&opts.target),
        BUDGET_MS.with(Cell::get),
        opts.seconds,
        json_str(outcome),
        challenge,
        page_bytes,
        json_str(title)
    )
}

fn page_title(html: &str) -> String {
    let l = html.to_ascii_lowercase();
    let Some(a) = l.find("<title") else { return String::new() };
    let Some(b) = l[a..].find('>') else { return String::new() };
    let rest = &html[a + b + 1..];
    let end = l[a + b + 1..].find("</title").unwrap_or(rest.len().min(200));
    rest[..end.min(200)].trim().to_string()
}

/// A report for a page that could not be loaded (still valid JSON, all numbers zero).
fn failed_report(opts: &ReportOpts, outcome: &str, why: &str) -> String {
    stop();
    let body = body_json();
    format!("{{{},\"error\":{},{},\"js_heap_bytes\":0,\"rss_bytes\":{},\"js_errors\":0,\"console_errors\":0}}", envelope(opts, outcome, false, 0, ""), json_str(why), body, rss_bytes())
}

/// Loads `opts.target` (an http(s) URL through wire, or a file), runs its
/// scripts under the recorder, pumps timers and the network for `opts.seconds`,
/// and returns the JSON report.
pub(crate) fn perf_report(opts: &ReportOpts) -> String {
    start();
    // QY53i2: each page is a navigation, as `App::replace_document` does.
    crate::css::reset_custom_props_arena();
    let target = opts.target.clone();
    let (html, page_url, mut scripts, mut css) = match classify_input(&target) {
        Input::Url(url) => {
            let Some(socket) = &opts.socket else { return failed_report(opts, "fetch_failed", "a URL needs --socket <wire socket>") };
            js_net::set_net_executor(Arc::new(WireExecutor(socket.clone())));
            let request = build_get_request_ctx(&url, &CookieContext::navigation(&url, None));
            begin_wire_load(0);
            let (content_type, body, final_url) = match fetch_raw_via_wire(socket, &request) {
                WireFetchRaw::Responded { content_type, body, final_url } => (content_type, body, final_url),
                WireFetchRaw::Unreachable => return failed_report(opts, "fetch_failed", "wire is not reachable"),
            };
            let html = match dispatch_wire_response(content_type, body) {
                Ok(h) => String::from_utf8_lossy(&h).into_owned(),
                Err(e) => return failed_report(opts, "fetch_failed", &e),
            };
            let mut page = url;
            if let Some(f) = final_url.and_then(|f| Url::parse(&f).ok()).filter(|f| matches!(f.scheme(), "http" | "https")) {
                page = f;
            }
            page.set_fragment(None);
            (html, Some(page), None::<Vec<ScriptSource>>, String::new())
        }
        Input::File(file) => {
            let bytes = match std::fs::read(&file) {
                Ok(b) => b,
                Err(e) => return failed_report(opts, "fetch_failed", &format!("{file}: {e}")),
            };
            if let Some(s) = &opts.socket {
                js_net::set_net_executor(Arc::new(WireExecutor(s.clone())));
            }
            (decode_html_bytes(&bytes, None), file_page_url(&file), None, String::new())
        }
        _ => return failed_report(opts, "fetch_failed", "no page given"),
    };
    let Ok(dom) = new_parser().read_from(&mut html.as_bytes()) else { return failed_report(opts, "fetch_failed", "could not parse the page") };
    let base = find_base_url(&dom.document).or(page_url.clone());
    set_page_url(base.as_ref());
    if let (Some(socket), Some(base)) = (&opts.socket, &base) {
        let top = page_url.clone().unwrap_or_else(|| base.clone());
        let mut res = ExternalResources::default();
        find_external_resources(&dom.document, &mut res);
        css = fetch_stylesheets(socket, base, &top, &res.stylesheets);
        prefetch_iframes(socket, base, &top, &dom.document);
        scripts = Some(fetch_scripts(socket, base, &top, res.scripts));
    }
    js_style::set_js_extra_css(&css);
    let scripts = scripts.unwrap_or_else(|| inline_scripts(&dom.document));
    let errors = Rc::new(Cell::new(0u64));
    let console_errors = Rc::new(Cell::new(0u64));
    let (e, c) = (errors.clone(), console_errors.clone());
    let log: LogFn = Rc::new(move |level, text| {
        if level == "error" {
            e.set(e.get() + 1);
            diag!("scratchy: js error: {text}");
        } else if level == "console.error" {
            c.set(c.get() + 1);
        }
    });
    mark("scripts_ready");
    // QMEM3b: what this harness itself keeps alive to the end of the run (a window drops the
    // fetched script and page text once the page started; the DOM lives on in both).
    {
        let doc = dom.document.clone();
        register_probe("dom", move || dom_mem(&doc));
        let n = html.capacity();
        register_probe("report_html", move || (1, n));
        let n: usize = scripts.iter().map(|s| s.code.capacity() + s.name.capacity()).sum();
        let c = scripts.len();
        register_probe("report_scripts", move || (c, n));
        let n = css.capacity();
        register_probe("report_css", move || (1, n));
    }
    let engine = start_scripts(&dom.document, &scripts, log);
    let mut heap = 0i64;
    if let Some(engine) = &engine {
        // Pump the page for `seconds` of real time: timers when due, then the network.
        let mut sched = NetScheduler::new(&RequestPolicy::default());
        let exec = js_net::net_executor();
        let end = Instant::now() + Duration::from_secs_f64(opts.seconds.max(0.0));
        js_net::drive_report_net(engine, &mut sched, &*exec);
        while Instant::now() < end {
            let Some(due) = engine.next_deadline() else { break };
            let now = Instant::now();
            if due > now {
                std::thread::sleep((due - now).min(end.saturating_duration_since(now)));
                if Instant::now() >= end {
                    break;
                }
            }
            if engine.run_due_counted() == 0 {
                break;
            }
            js_net::drive_report_net(engine, &mut sched, &*exec);
        }
        heap = engine.eng.memory_used();
    }
    let challenge = looks_like_challenge(&html);
    let outcome = if challenge { "challenge" } else { "ok" };
    #[cfg(feature = "memprof")]
    crate::memprof::dump_perf_report(); // QMEM3g
    let memory = memory_json(engine.as_ref());
    stop();
    format!(
        "{{{},{},{},\"js_heap_bytes\":{},\"rss_bytes\":{},\"js_errors\":{},\"console_errors\":{}}}",
        envelope(opts, outcome, challenge, html.len(), &page_title(&html)),
        body_json(),
        memory,
        heap.max(0),
        rss_bytes(),
        errors.get(),
        console_errors.get()
    )
}

/// `scratchy --perf-report [--socket S] [--budget-ms N] [--seconds T] <url|file>`.
pub(crate) fn perf_report_main(args: &[String]) -> i32 {
    let mut socket = None;
    let mut seconds = DEFAULT_SECONDS;
    let mut targets = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--socket" => socket = it.next().map(std::path::PathBuf::from),
            "--budget-ms" | "--user-agent" => {
                it.next(); // installed by `run`
            }
            "--seconds" => match it.next().and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0 && *v <= 3600.0) {
                Some(v) => seconds = v,
                None => {
                    diag!("scratchy: --seconds needs a number of seconds (0..3600)");
                    return 2;
                }
            },
            _ => targets.push(a.clone()),
        }
    }
    if targets.is_empty() {
        diag!("usage: scratchy --perf-report [--socket <path>] [--budget-ms <ms>] [--seconds <s>] <url|file>...");
        return 2;
    }
    set_js_enabled(true);
    js_engine::ensure_snapshot(); // QV8-4: the startup snapshot is built BEFORE the page, on its own thread
    // QY53i2: several targets = several navigations in ONE process, one JSON line each (cross-page
    // memory: the `memory` object of page N includes whatever pages 1..N-1 left behind).
    for target in targets {
        let json = perf_report(&ReportOpts { socket: socket.clone(), seconds, target });
        write_raw_stdout(format_args!("{json}\n"));
    }
    0
}

// ---------------------------------------------------------------- corpus

/// A bare QuickJS context (no DOM prelude): `(compile_cpu_ns, run_cpu_ns, error)`.
fn bare_eval(code: &str, preamble: &[&str]) -> (u64, u64, Option<String>) {
    let eng = match Engine::new(EngineOpts { memory_limit: 1 << 30, stack_limit: js_stack_limit(), page: None }) {
        Ok(e) => e,
        Err(m) => return (0, 0, Some(m.replace("could not create the JS runtime", "no runtime").replace("could not create the JS context", "no context"))),
    };
    let eval = |ctx: &Ctx<'_>, src: &str| -> (u64, u64, Option<String>) {
        let Some(source) = js_engine::Source::new(src, "corpus") else { return (0, 0, Some("NUL in source".into())) };
        let t0 = thread_cpu_nanos();
        let Ok(func) = source.compile(ctx) else {
            return (thread_cpu_nanos().saturating_sub(t0), 0, Some(js_engine::describe_exception(ctx)));
        };
        let t1 = thread_cpu_nanos();
        let res = func.run(ctx);
        let t2 = thread_cpu_nanos();
        if res.is_err() {
            return (t1.saturating_sub(t0), t2.saturating_sub(t1), Some(js_engine::describe_exception(ctx)));
        }
        (t1.saturating_sub(t0), t2.saturating_sub(t1), None)
    };
    let r = eng.with(|ctx| {
        for p in preamble {
            let r = eval(&ctx, p);
            if r.2.is_some() {
                return r;
            }
        }
        eval(&ctx, code)
    });
    // Run the queued promise jobs of the script too (counted as run time).
    let t = thread_cpu_nanos();
    eng.run_all_jobs();
    (r.0, r.1 + thread_cpu_nanos().saturating_sub(t), r.2)
}

/// The same, with the DOM prelude (a page engine over an empty document);
/// `prereq` scripts run first and are not measured.
fn dom_eval(code: &str, prereq: &[String]) -> (u64, u64, Option<String>) {
    let err = Rc::new(RefCell::new(None::<String>));
    let e = err.clone();
    let log: LogFn = Rc::new(move |level, text| {
        if level == "error" && e.borrow().is_none() {
            *e.borrow_mut() = Some(text.lines().next().unwrap_or("").to_string());
        }
    });
    set_page_url(None);
    let Some(engine) = start_scripts(&parse_html("<html><body></body></html>"), &[], log) else { return (0, 0, Some("no engine".into())) };
    for p in prereq {
        engine.eval("prereq", p);
    }
    *err.borrow_mut() = None;
    let t = engine.eval_as(None, "corpus", code);
    let e = err.borrow().clone();
    (t.0, t.1, e)
}

/// The fixed CPU loops (fib 25, sort of 200k objects, JSON round trip of a big structure).
const CPU_LOOPS: [(&str, &str); 3] = [
    ("fib25", "function fib(n){return n<2?n:fib(n-1)+fib(n-2)} if(fib(25)!==75025) throw new Error('fib');"),
    (
        "sort_200k_objects",
        "var a=[],s=12345;for(var i=0;i<200000;i++){s=(s*1103515245+12345)&0x7fffffff;a.push({k:s,i:i});} a.sort(function(x,y){return x.k-y.k}); \
         for(var i=1;i<a.length;i++) if(a[i-1].k>a[i].k) throw new Error('sort');",
    ),
    (
        "json_round_trip",
        "var o=[];for(var i=0;i<20000;i++)o.push({id:i,name:'item'+i,tags:['a','b','c'],nested:{x:i/3,y:[1,2,3],s:'str\\u00e9'+i}}); \
         var t=JSON.stringify(o);var p=JSON.parse(t);if(p.length!==20000||JSON.stringify(p)!==t) throw new Error('json');",
    ),
];

fn entry_json(name: &str, mode: &str, r: (u64, u64, Option<String>), bytes: usize) -> String {
    let err = r.2.as_deref().map(json_str).unwrap_or_else(|| "null".into());
    format!("{{\"name\":{},\"mode\":{},\"bytes\":{},\"compile_cpu_ms\":{},\"first_run_cpu_ms\":{},\"error\":{}}}", json_str(name), json_str(mode), bytes, ms(r.0), ms(r.1), err)
}

/// Scripts that need the DOM prelude (and what must run before them).
fn corpus_needs_dom(file: &str) -> bool {
    file.starts_with("jquery") || file.starts_with("react-dom")
}

/// Measures every `*.js` in `dir` (sorted by name) and the fixed CPU loops; returns the JSON.
pub(crate) fn perf_corpus(dir: &str) -> Result<String, String> {
    let mut files: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("{dir}: {e}"))?.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "js")).collect();
    files.sort();
    let mut texts = Vec::new();
    for p in &files {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
        texts.push((name, String::from_utf8_lossy(&bytes).into_owned()));
    }
    let mut entries = Vec::new();
    for (name, text) in &texts {
        let dom = corpus_needs_dom(name);
        let r = if dom {
            // react-dom needs react first (UMD globals); not measured.
            let prereq: Vec<String> = if name.starts_with("react-dom") { texts.iter().filter(|(n, _)| n.starts_with("react-1") || n.starts_with("react.")).map(|(_, t)| t.clone()).collect() } else { Vec::new() };
            dom_eval(text, &prereq)
        } else {
            // No DOM prelude: only what a UMD wrapper and a deprecation warning need.
            bare_eval(text, &["var self=globalThis;var console={log:function(){},warn:function(){},error:function(){}};"])
        };
        entries.push(entry_json(name, if dom { "dom_prelude" } else { "bare" }, r, text.len()));
    }
    let loops: Vec<String> = CPU_LOOPS.iter().map(|(n, code)| entry_json(n, "bare", bare_eval(code, &[]), code.len())).collect();
    let (name, version) = engine_info();
    Ok(format!(
        "{{\"schema\":1,\"engine\":{},\"engine_version\":{},\"commit\":{},\"scripts\":[{}],\"cpu_loops\":[{}]}}",
        json_str(name),
        json_str(&version),
        json_str(commit()),
        entries.join(","),
        loops.join(",")
    ))
}

/// `scratchy --perf-corpus <dir>`.
pub(crate) fn perf_corpus_main(args: &[String]) -> i32 {
    let Some(dir) = args.iter().find(|a| !a.starts_with("--")) else {
        diag!("usage: scratchy --perf-corpus <dir of .js files>");
        return 2;
    };
    set_js_enabled(true);
    match perf_corpus(dir) {
        Ok(j) => {
            write_raw_stdout(format_args!("{j}\n"));
            0
        }
        Err(e) => {
            diag!("scratchy: {e}");
            2
        }
    }
}
