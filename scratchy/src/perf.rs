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
}

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static BUDGET_MS: Cell<u64> = const { Cell::new(DEFAULT_BUDGET_MS) };
    static REC: RefCell<Rec> = RefCell::new(Rec::default());
}

#[inline]
pub(crate) fn enabled() -> bool {
    ON.with(Cell::get)
}

/// Turns the recorder on (fresh state) and starts the "time to ..." clocks.
pub(crate) fn start() {
    REC.with(|r| {
        *r.borrow_mut() = Rec { start: Some((Instant::now(), thread_cpu_nanos())), ..Default::default() };
    });
    ON.with(|o| o.set(true));
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
        r.stack.push(Open { kind, cpu0: thread_cpu_nanos(), bud0: script_clock_nanos(), child_cpu: 0, child_bud: 0 });
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
        let idx = KINDS.iter().position(|(k, _)| *k == o.kind).unwrap_or(0);
        let pk = &mut r.kinds[idx];
        pk.samples.push((bud.saturating_sub(o.child_bud), cpu.saturating_sub(o.child_cpu)));
        if killed {
            pk.kills += 1;
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
        REC.with(|r| r.borrow_mut().forced.push(cpu_ns));
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
        let ftotal: u64 = r.forced.iter().sum();
        let fmax = r.forced.iter().copied().max().unwrap_or(0);
        let mark = |name: &str| r.marks.iter().find(|m| m.0 == name).map(|m| (m.1, m.2)).unwrap_or((0.0, 0));
        let wall = |x: f64| (x * 1000.0).round() / 1000.0;
        let (fw, fc) = mark("scripts_ready");
        let (dw, dc) = mark("dom_content_loaded");
        let (lw, lc) = mark("load");
        let (_, es) = mark("engine_start");
        let (_, ee) = mark("engine_ready");
        format!(
            "\"callbacks\":{{{}}},\"budget_kills\":{},\"forced_layout\":{{\"count\":{},\"total_ms\":{},\"max_ms\":{}}},\
             \"compile_cpu_ms\":{},\"engine_startup_cpu_ms\":{},\
             \"load\":{{\"fetch_wall_ms\":{},\"fetch_cpu_ms\":{},\"dom_content_loaded_wall_ms\":{},\"dom_content_loaded_cpu_ms\":{},\"load_wall_ms\":{},\"load_cpu_ms\":{}}}",
            kinds.join(","),
            kills,
            r.forced.len(),
            ms(ftotal),
            ms(fmax),
            ms(r.compile_ns),
            ms(ee.saturating_sub(es)),
            wall(fw),
            ms(fc),
            wall(dw),
            ms(dc),
            wall(lw),
            ms(lc)
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
        "\"schema\":1,\"engine\":{},\"engine_version\":{},\"commit\":{},\"target\":{},\"budget_ms\":{},\"seconds\":{},\
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
    stop();
    format!(
        "{{{},{},\"js_heap_bytes\":{},\"rss_bytes\":{},\"js_errors\":{},\"console_errors\":{}}}",
        envelope(opts, outcome, challenge, html.len(), &page_title(&html)),
        body_json(),
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
    let mut target = None;
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
            _ => target = Some(a.clone()),
        }
    }
    let Some(target) = target else {
        diag!("usage: scratchy --perf-report [--socket <path>] [--budget-ms <ms>] [--seconds <s>] <url|file>");
        return 2;
    };
    set_js_enabled(true);
    js_engine::ensure_snapshot(); // QV8-4: the startup snapshot is built BEFORE the page, on its own thread
    let json = perf_report(&ReportOpts { socket, seconds, target });
    write_raw_stdout(format_args!("{json}\n"));
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
