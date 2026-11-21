//! QWPT4a (R64 2.2, 2.3): the testharness.js test child.
//!
//! `scratchy --wpt-th-one --socket S URL` loads one WPT testharness page headless through wire
//! (normally a private `wire --mount http://web-platform.test:8000=CHECKOUT --mount-override
//! /resources/testharnessreport.js=tools/wpt/testharnessreport.js`), runs it on a virtual clock and
//! prints ONE JSON line `{"harness", "message", "tests"}` on stdout, or nothing when the page never
//! reported (the parent, QWPT4b, then records CRASH). Format: NOTES.md "WPT testharness child".
//!
//! Virtual clock: time starts at 0 and only moves when nothing can run now (no due timer, no
//! queued microtask, no network result to deliver) AND no load is in flight; it then jumps to the
//! next timer. So testharness's own 10 s / 60 s timeout costs milliseconds of real time, while a
//! slow fetch is waited for in real time and never turns into a false TIMEOUT.
//! Viewport 800x600 (WPT's default); Ahem is fetched from the page's origin (`/fonts/Ahem.ttf`)
//! and added to the forced-layout fonts. No window, no App (the rendering steps are QEL1).
use super::*;
use crate::js_net::{NetExecutor, NetJob};
use std::time::{Duration, Instant};

/// WPT's default viewport (R64 2.3).
pub(crate) const VIEWPORT: (u32, u32) = (800, 600);
/// Stop pumping after this many timer callbacks (a page polling forever with its harness timeout
/// disabled); no result is printed then.
const MAX_CALLBACKS: usize = 200_000;
/// Stop when the virtual clock passes this (ms): far beyond `timeout=long` (60 s) times any sane
/// `timeout_multiplier`.
const MAX_VIRTUAL_MS: f64 = 3_600_000.0;
/// A network result not arriving within this much real time ends the run (the request itself
/// times out sooner, by the policy's read timeout).
const MAX_NET_WAIT: Duration = Duration::from_secs(120);

pub(crate) fn main(args: &[String]) -> i32 {
    let mut socket = None;
    let mut url = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--socket" => socket = it.next().map(PathBuf::from),
            "--user-agent" | "--budget-ms" => {
                it.next(); // installed by `run` (QY39b, QV8-0)
            }
            a if url.is_none() && !a.starts_with("--") => url = Some(a.to_string()),
            a => {
                diag!("scratchy --wpt-th-one: unexpected argument {a}");
                return 2;
            }
        }
    }
    let (Some(socket), Some(url)) = (socket, url) else {
        diag!("usage: scratchy --wpt-th-one --socket <path> <url>");
        return 2;
    };
    let Ok(url) = Url::parse(&url) else {
        diag!("scratchy --wpt-th-one: bad URL {url}");
        return 2;
    };
    js_engine::ensure_snapshot(); // QV8-4: one-shot mode: build the startup snapshot first
    let exec: Arc<dyn NetExecutor> = Arc::new(WireExecutor(socket.clone()));
    match run_one(&url, &socket, exec) {
        Some(line) => {
            write_raw_stdout(format_args!("{line}\n"));
            0
        }
        None => 1,
    }
}

/// Fetches `url` through wire at `socket`, runs it and returns the vendor hook's JSON (`None`: the
/// page never reported). Page network requests go through `exec` on worker threads.
pub(crate) fn run_one(url: &Url, socket: &std::path::Path, exec: Arc<dyn NetExecutor>) -> Option<String> {
    set_js_enabled(true);
    crate::js::set_window_viewport(VIEWPORT.0, VIEWPORT.1);
    js_net::set_net_executor(exec.clone());
    let fonts = get(socket, &url.join("/fonts/Ahem.ttf").ok()?).map(|(_, b)| vec![b]).unwrap_or_else(|e| {
        diag!("scratchy --wpt-th-one: no Ahem font: {e}");
        Vec::new()
    });
    crate::js_geom::install_extra_fonts(fonts);
    let (ct, body) = match get(socket, url) {
        Ok(r) => r,
        Err(e) => {
            diag!("scratchy --wpt-th-one: {url}: {e}");
            return None;
        }
    };
    let html = decode_html_bytes(&body, ct.as_deref().and_then(content_type_charset));
    let dom = new_parser().read_from(&mut html.as_bytes()).ok()?;
    let document = dom.document;
    set_page_url(Some(url));
    let base = find_base_url(&document).unwrap_or_else(|| url.clone());
    let mut res = ExternalResources::default();
    find_external_resources(&document, &mut res);
    let (css, _) = fetch_stylesheet_parts(socket, &base, url, &res.stylesheets);
    js_style::set_js_extra_css(&css);
    prefetch_iframes(socket, &base, url, &document);
    let scripts = fetch_scripts(socket, &base, url, res.scripts);

    let clock = Rc::new(Cell::new(0.0f64));
    let done: Rc<RefCell<Option<String>>> = Default::default();
    let log: LogFn = Rc::new(|level, text| {
        if level == "error" || level == "console.error" {
            diag!("scratchy: {level}: {text}");
        }
    });
    let sink = done.clone();
    let engine = start_scripts_hooked(&document, &scripts, log, Some(clock.clone()), &move |e| e.install_wpt_done(sink.clone()))?;
    pump(&engine, &clock, &done, exec);
    let out = done.borrow_mut().take();
    drop(engine);
    out
}

/// GET through wire: (Content-Type, body) of a 2xx answer.
fn get(socket: &std::path::Path, url: &Url) -> Result<(Option<String>, Vec<u8>), String> {
    let req = build_wire_request("GET", url, &[], &[]).map_err(|e| e.to_string())?;
    let r = fetch_full_via_wire(socket, &req, &WireLimits::default()).map_err(|e| e.to_string())?;
    if let Some(e) = &r.error {
        return Err(e.clone());
    }
    if !(200..300).contains(&r.status) {
        return Err(format!("HTTP {} {}", r.status, r.status_text));
    }
    Ok((r.header("content-type").map(str::to_string), r.body))
}

/// The event loop of the child (R64 2.3): network first, then due timers; the clock advances only
/// when neither has work and no request is in flight. Returns when the page reported, or when it
/// can make no more progress.
fn pump(engine: &JsEngine, clock: &Rc<Cell<f64>>, done: &Rc<RefCell<Option<String>>>, exec: Arc<dyn NetExecutor>) {
    let mut sched = NetScheduler::new(&RequestPolicy::default());
    let (tx, rx) = mpsc::channel::<(u64, Result<WireResponse, NetError>)>();
    let mut in_flight: HashSet<u64> = HashSet::new();
    let mut callbacks = 0usize;
    while done.borrow().is_none() {
        // 1. Network: queue new requests, drop aborted ones, deliver what is ready, start more.
        let mut progressed = false;
        let reqs = engine.take_requests();
        for id in reqs.net_abort {
            sched.abort(id);
            in_flight.remove(&id); // its late result is ignored by `deliver_net`
        }
        for job in reqs.net {
            sched.enqueue(job);
        }
        for (id, r) in reqs.net_done {
            engine.deliver_net(id, r);
            progressed = true;
        }
        while let Ok((id, r)) = rx.try_recv() {
            sched.done(id);
            if in_flight.remove(&id) {
                engine.deliver_net(id, r);
                progressed = true;
            }
        }
        let mut started: Vec<NetJob> = Vec::new();
        sched.pump(&mut |j| started.push(j));
        for job in started {
            in_flight.insert(job.id);
            let (tx, exec) = (tx.clone(), exec.clone());
            thread::spawn(move || {
                let r = exec.execute(&job);
                let _ = tx.send((job.id, r));
            });
        }
        while engine.pump_ws() {
            progressed = true;
        }
        if done.borrow().is_some() {
            break;
        }
        if progressed {
            continue;
        }
        // 2. Timers (and deferred promise jobs) due at the current virtual time, then (QEL1) the
        //    rendering update when its virtual 16.7 ms frame is due (rAF batch, observer hooks).
        let timer_due = engine.next_due_ms();
        let mut n = 0;
        if timer_due.is_some_and(|d| d <= clock.get()) {
            n = engine.run_due_counted();
        }
        if n == 0 {
            n = crate::event_loop::headless_frame(engine, clock.get());
        }
        callbacks += n;
        if callbacks > MAX_CALLBACKS {
            diag!("scratchy --wpt-th-one: stopped after {MAX_CALLBACKS} timer callbacks");
            break;
        }
        if n > 0 {
            continue;
        }
        let due = match (timer_due, crate::event_loop::headless_frame_due(engine)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        // 3. A load is pending: wait for it in real time; the clock stays where it is.
        if !in_flight.is_empty() || engine.ws_busy() {
            let wait = if in_flight.is_empty() { Duration::from_millis(10) } else { MAX_NET_WAIT };
            let t0 = Instant::now();
            match rx.recv_timeout(wait) {
                Ok((id, r)) => {
                    sched.done(id);
                    if in_flight.remove(&id) {
                        engine.deliver_net(id, r);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) if !in_flight.is_empty() => {
                    diag!("scratchy --wpt-th-one: no network result in {} s", t0.elapsed().as_secs());
                    break;
                }
                Err(_) => {}
            }
            continue;
        }
        // 4. Idle: jump to the next timer or frame, or stop when there is none.
        match due {
            Some(d) if d > clock.get() && d <= MAX_VIRTUAL_MS => clock.set(d),
            Some(d) if d > MAX_VIRTUAL_MS => {
                diag!("scratchy --wpt-th-one: virtual clock passed {} s without a result", MAX_VIRTUAL_MS / 1000.0);
                break;
            }
            // No timer at all, or one that was due but did not run (stale due time): stuck.
            _ => break,
        }
    }
}
