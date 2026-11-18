#[allow(unused_imports)]
use super::*;

// ---- QV8-0: perf harness (--perf-report, --perf-corpus, --budget-ms)

/// A minimal JSON value, enough to check the shape of the harness output.
#[derive(Debug, Clone, PartialEq)]
enum PJ {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Arr(Vec<PJ>),
    Obj(Vec<(String, PJ)>),
}

impl PJ {
    fn get(&self, key: &str) -> &PJ {
        match self {
            PJ::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap_or_else(|| panic!("missing `{key}` in {self:?}")),
            _ => panic!("not an object: {self:?}"),
        }
    }
    fn num(&self) -> f64 {
        match self {
            PJ::Num(n) => *n,
            _ => panic!("not a number: {self:?}"),
        }
    }
}

fn pj_parse(s: &str) -> PJ {
    fn ws(b: &[u8], i: &mut usize) {
        while *i < b.len() && b[*i].is_ascii_whitespace() {
            *i += 1;
        }
    }
    fn val(b: &[u8], i: &mut usize) -> PJ {
        ws(b, i);
        match b[*i] {
            b'{' => {
                *i += 1;
                let mut m = Vec::new();
                loop {
                    ws(b, i);
                    if b[*i] == b'}' {
                        *i += 1;
                        return PJ::Obj(m);
                    }
                    if b[*i] == b',' {
                        *i += 1;
                        continue;
                    }
                    let PJ::Str(k) = val(b, i) else { panic!("object key") };
                    ws(b, i);
                    assert_eq!(b[*i], b':');
                    *i += 1;
                    m.push((k, val(b, i)));
                }
            }
            b'[' => {
                *i += 1;
                let mut v = Vec::new();
                loop {
                    ws(b, i);
                    if b[*i] == b']' {
                        *i += 1;
                        return PJ::Arr(v);
                    }
                    if b[*i] == b',' {
                        *i += 1;
                        continue;
                    }
                    v.push(val(b, i));
                }
            }
            b'"' => {
                *i += 1;
                let mut out = Vec::new();
                while b[*i] != b'"' {
                    if b[*i] == b'\\' {
                        *i += 1;
                        match b[*i] {
                            b'n' => out.push(b'\n'),
                            b'r' => out.push(b'\r'),
                            b't' => out.push(b'\t'),
                            b'u' => {
                                let c = u32::from_str_radix(std::str::from_utf8(&b[*i + 1..*i + 5]).unwrap(), 16).unwrap();
                                out.extend(char::from_u32(c).unwrap_or('?').to_string().bytes());
                                *i += 4;
                            }
                            c => out.push(c),
                        }
                    } else {
                        out.push(b[*i]);
                    }
                    *i += 1;
                }
                *i += 1;
                PJ::Str(String::from_utf8(out).unwrap())
            }
            b't' => {
                *i += 4;
                PJ::Bool(true)
            }
            b'f' => {
                *i += 5;
                PJ::Bool(false)
            }
            b'n' => {
                *i += 4;
                PJ::Null
            }
            _ => {
                let st = *i;
                while *i < b.len() && (b[*i].is_ascii_digit() || matches!(b[*i], b'-' | b'+' | b'.' | b'e' | b'E')) {
                    *i += 1;
                }
                PJ::Num(std::str::from_utf8(&b[st..*i]).unwrap().parse().unwrap_or_else(|_| panic!("bad number at {st}")))
            }
        }
    }
    let b = s.trim().as_bytes();
    let mut i = 0;
    let v = val(b, &mut i);
    ws(b, &mut i);
    assert_eq!(i, b.len(), "trailing data after the JSON value");
    v
}

fn pj_all_numbers_non_negative(v: &PJ, path: &str) {
    match v {
        PJ::Num(n) => assert!(*n >= 0.0 && n.is_finite(), "{path} = {n}"),
        PJ::Arr(a) => a.iter().enumerate().for_each(|(i, x)| pj_all_numbers_non_negative(x, &format!("{path}[{i}]"))),
        PJ::Obj(m) => m.iter().for_each(|(k, x)| pj_all_numbers_non_negative(x, &format!("{path}.{k}"))),
        _ => {}
    }
}

/// Resets the per-thread perf state when a test ends (even on a panic).
struct PerfGuard;
impl Drop for PerfGuard {
    fn drop(&mut self) {
        crate::perf::stop();
        crate::perf::set_budget_ms(crate::perf::DEFAULT_BUDGET_MS);
        crate::js::FAKE_CLOCK.with(|f| f.set(None));
    }
}

fn qv8_report(name: &str, body: &str) -> PJ {
    let path = std::env::temp_dir().join(format!("scratchy-qv8-0-{}-{name}.html", std::process::id()));
    std::fs::write(&path, format!("<html><head><title>qv8 fixture</title></head><body>{body}</body></html>")).unwrap();
    let target = path.to_string_lossy().into_owned();
    // Fake CPU clock that never advances by itself: `__test_burn(ms)` is the only CPU.
    crate::js::FAKE_CLOCK.with(|f| f.set(Some((1_000_000_000, 0))));
    let json = crate::perf::perf_report(&crate::perf::ReportOpts { socket: None, seconds: 0.5, target });
    let _ = std::fs::remove_file(path);
    pj_parse(&json)
}

const QV8_BUSY_50: &str = "<script>setTimeout(function(){for(var i=1;i<=1000000;i++){if(i%20000===0)__test_burn(1);}},0);</script>";

#[test]
fn qv8_0_perf_report_json_shape() {
    let _g = PerfGuard;
    let j = qv8_report("shape", "<p>hi</p><script>console.log('x');</script><script>throw new Error('boom');</script>");
    for k in ["schema", "engine", "engine_version", "commit", "target", "budget_ms", "seconds", "outcome", "challenge", "page_bytes", "title"] {
        j.get(k);
    }
    // QV8-3a: the report names whichever engine this build uses.
    assert_eq!(j.get("engine"), &PJ::Str(crate::js_engine::engine_info().0.into()));
    assert_eq!(j.get("outcome"), &PJ::Str("ok".into()));
    assert_eq!(j.get("budget_ms").num(), 500.0);
    assert_eq!(j.get("title"), &PJ::Str("qv8 fixture".into()));
    let cb = j.get("callbacks");
    for kind in ["classic_script", "module_script", "timer", "promise_job", "event", "observer_delivery", "net_callback"] {
        let k = cb.get(kind);
        k.get("count");
        k.get("kills");
        for clock in ["budget_clock", "raw_cpu"] {
            for f in ["total_ms", "max_ms", "p50_ms", "p95_ms"] {
                k.get(clock).get(f);
            }
        }
    }
    assert_eq!(cb.get("classic_script").get("count").num(), 2.0);
    // DOMContentLoaded and load are two events.
    assert_eq!(cb.get("event").get("count").num(), 2.0);
    for f in ["count", "total_ms", "max_ms"] {
        j.get("forced_layout").get(f);
    }
    for f in ["fetch_wall_ms", "fetch_cpu_ms", "dom_content_loaded_wall_ms", "dom_content_loaded_cpu_ms", "load_wall_ms", "load_cpu_ms"] {
        j.get("load").get(f);
    }
    for f in ["budget_kills", "compile_cpu_ms", "engine_startup_cpu_ms", "js_heap_bytes", "rss_bytes", "js_errors", "console_errors"] {
        j.get(f);
    }
    assert_eq!(j.get("js_errors").num(), 1.0);
    assert!(j.get("js_heap_bytes").num() > 0.0);
    assert!(j.get("rss_bytes").num() > 0.0);
    pj_all_numbers_non_negative(&j, "$");
}

#[test]
fn qv8_0_budget_flag_kills_the_50ms_loop_only_when_lower() {
    let _g = PerfGuard;
    // Default budget (500 ms): the 50 ms loop completes, one timer entry >= 50 ms.
    let j = qv8_report("busy-default", QV8_BUSY_50);
    let timer = j.get("callbacks").get("timer");
    assert_eq!(timer.get("count").num(), 1.0, "{timer:?}");
    assert!(timer.get("raw_cpu").get("max_ms").num() >= 50.0, "{timer:?}");
    assert_eq!(timer.get("kills").num(), 0.0);
    assert_eq!(j.get("budget_kills").num(), 0.0);
    // --budget-ms 20: the same loop is killed once, and counted.
    crate::perf::set_budget_ms(20);
    let j = qv8_report("busy-20", QV8_BUSY_50);
    assert_eq!(j.get("budget_ms").num(), 20.0);
    let timer = j.get("callbacks").get("timer");
    assert_eq!(timer.get("count").num(), 1.0, "{timer:?}");
    assert_eq!(timer.get("kills").num(), 1.0, "{timer:?}");
    assert_eq!(j.get("budget_kills").num(), 1.0);
    assert!(timer.get("raw_cpu").get("max_ms").num() < 50.0, "{timer:?}");
}

#[test]
fn qv8_0_budget_clock_excludes_forced_layout_time() {
    let _g = PerfGuard;
    crate::js::FAKE_CLOCK.with(|f| f.set(Some((1_000_000_000, 0))));
    crate::perf::start();
    let span = crate::perf::begin(Some(crate::perf::Kind::Timer));
    let cpu0 = crate::js::thread_cpu_nanos();
    crate::js::FAKE_CLOCK.with(|f| f.set(f.get().map(|(n, s)| (n + 30_000_000, s))));
    crate::js::pause_budget_since(cpu0); // 30 ms of forced layout
    crate::js::FAKE_CLOCK.with(|f| f.set(f.get().map(|(n, s)| (n + 10_000_000, s))));
    crate::perf::end(span, false);
    let j = pj_parse(&format!("{{{}}}", crate::perf::body_json()));
    let t = j.get("callbacks").get("timer");
    assert_eq!(t.get("raw_cpu").get("total_ms").num(), 40.0);
    assert_eq!(t.get("budget_clock").get("total_ms").num(), 10.0);
    assert_eq!(j.get("forced_layout").get("count").num(), 1.0);
    assert_eq!(j.get("forced_layout").get("total_ms").num(), 30.0);
}

#[test]
fn qv8_0_recorder_is_a_no_op_when_off() {
    let _g = PerfGuard;
    assert!(!crate::perf::enabled());
    let span = crate::perf::begin(Some(crate::perf::Kind::Timer));
    crate::perf::end(span, true);
    crate::perf::forced_layout(5_000_000);
    crate::perf::add_compile(5_000_000);
    crate::perf::mark("load");
    crate::perf::set_job_kind(Some(crate::perf::Kind::ModuleScript));
    assert_eq!(crate::perf::job_kind(), crate::perf::Kind::PromiseJob);
    // A normal engine run with the recorder off records nothing.
    let log: crate::js::LogFn = Rc::new(|_, _| {});
    drop(crate::js::start_scripts(&parse_html(""), &[src("setTimeout(function(){},0);")], log));
    crate::perf::start();
    let j = pj_parse(&format!("{{{}}}", crate::perf::body_json()));
    assert_eq!(j.get("callbacks").get("classic_script").get("count").num(), 0.0);
    assert_eq!(j.get("callbacks").get("timer").get("count").num(), 0.0);
    assert_eq!(j.get("budget_kills").num(), 0.0);
    assert_eq!(j.get("forced_layout").get("count").num(), 0.0);
    assert_eq!(j.get("compile_cpu_ms").num(), 0.0);
}

#[test]
fn qv8_0_budget_ms_parsing() {
    let a = |v: &[&str]| crate::perf::parse_budget_ms(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(a(&["--js-report", "x.html"]), Ok(None));
    assert_eq!(a(&["--budget-ms", "20000", "x.html"]), Ok(Some(20000)));
    assert_eq!(a(&["x.html", "--budget-ms", "1"]), Ok(Some(1)));
    for bad in [&["--budget-ms"][..], &["--budget-ms", "abc"], &["--budget-ms", "-5"], &["--budget-ms", "0"], &["--budget-ms", "1.5"], &["--budget-ms", "99999999999"], &["--budget-ms", ""]] {
        assert!(a(bad).is_err(), "{bad:?} must be rejected");
    }
    // Defaults: the callback budget is 500 ms and a page-load script keeps its 5 s.
    assert_eq!(crate::perf::callback_budget().as_millis(), 500);
    assert_eq!(crate::perf::script_budget().as_secs(), 5);
    assert_eq!(crate::perf::drain_cap().as_millis(), 2000);
    let _g = PerfGuard;
    crate::perf::set_budget_ms(20_000);
    assert_eq!(crate::perf::callback_budget().as_secs(), 20);
    assert_eq!(crate::perf::script_budget().as_secs(), 20);
}

#[test]
fn qv8_0_perf_corpus_json_shape_on_a_tiny_script() {
    let _g = PerfGuard;
    let dir = std::env::temp_dir().join(format!("scratchy-qv8-0-corpus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tiny-1.0.min.js"), "var tiny = {add: function(a, b) { return a + b; }}; if (tiny.add(1, 2) !== 3) throw new Error('x');").unwrap();
    std::fs::write(dir.join("jquery-0.0.min.js"), "window.__dom = document.createElement('div').tagName;").unwrap();
    std::fs::write(dir.join("broken.js"), "this is not javascript (").unwrap();
    std::fs::write(dir.join("readme.txt"), "ignored").unwrap();
    let json = crate::perf::perf_corpus(dir.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let j = pj_parse(&json);
    // QV8-3a: the report names whichever engine this build uses.
    assert_eq!(j.get("engine"), &PJ::Str(crate::js_engine::engine_info().0.into()));
    let PJ::Arr(scripts) = j.get("scripts") else { panic!() };
    let names: Vec<_> = scripts.iter().map(|s| s.get("name").clone()).collect();
    assert_eq!(names, [PJ::Str("broken.js".into()), PJ::Str("jquery-0.0.min.js".into()), PJ::Str("tiny-1.0.min.js".into())]);
    for s in scripts {
        for f in ["mode", "bytes", "compile_cpu_ms", "first_run_cpu_ms", "error"] {
            s.get(f);
        }
    }
    assert_eq!(scripts[0].get("mode"), &PJ::Str("bare".into()));
    assert_ne!(scripts[0].get("error"), &PJ::Null, "a syntax error is reported, not fatal");
    assert_eq!(scripts[1].get("mode"), &PJ::Str("dom_prelude".into()));
    assert_eq!(scripts[1].get("error"), &PJ::Null, "{:?}", scripts[1]);
    assert_eq!(scripts[2].get("error"), &PJ::Null, "{:?}", scripts[2]);
    let PJ::Arr(loops) = j.get("cpu_loops") else { panic!() };
    let lnames: Vec<_> = loops.iter().map(|s| s.get("name").clone()).collect();
    assert_eq!(lnames, [PJ::Str("fib25".into()), PJ::Str("sort_200k_objects".into()), PJ::Str("json_round_trip".into())]);
    for l in loops {
        assert_eq!(l.get("error"), &PJ::Null, "{l:?}");
        assert!(l.get("first_run_cpu_ms").num() > 0.0, "{l:?}");
    }
    pj_all_numbers_non_negative(&j, "$");
}
