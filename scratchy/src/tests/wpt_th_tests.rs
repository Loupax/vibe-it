#[allow(unused_imports)]
use super::*;

// ---------- QWPT4a: the testharness.js test child (`--wpt-th-one`) ----------
//
// A mini checkout in a temp dir, served by a fake wire on a unix socket the way `wire --mount`
// does (path -> file, `/resources/testharnessreport.js` -> scratchy's own tools/wpt file). CI has
// no WPT checkout when the unit tests run, so `resources/testharness.js` is a small stand-in with
// testharness.js's API and status numbers; with `SCRATCHY_WPT_DIR=<checkout>` the same fixtures
// also run against the real testharness.js.

const REPORT_JS: &str = include_str!("../../tools/wpt/testharnessreport.js");

/// testharness.js's surface the fixtures use (test, async_test, promise_test, setup,
/// add_completion_callback, the 10 s harness timeout), with its numeric statuses.
const STAND_IN_HARNESS: &str = r#"
(function () {
  var tests = [], cbs = [], harness = { status: 0, message: null }, finished = false, loaded = false;
  function Test(name) { this.name = name; this.status = null; this.message = null; tests.push(this); }
  Test.prototype.step = function (f, self) {
    if (this.status !== null) return;
    try { return f.apply(self || this, Array.prototype.slice.call(arguments, 2)); }
    catch (e) { this.status = 1; this.message = String(e && e.message || e); check(); }
  };
  Test.prototype.step_func = function (f) { var t = this; return function () { return t.step.apply(t, [f, this].concat([].slice.call(arguments))); }; };
  Test.prototype.step_func_done = function (f) { var t = this; return function () { if (f) t.step.apply(t, [f, this].concat([].slice.call(arguments))); t.done(); }; };
  Test.prototype.done = function () { if (this.status === null) { this.status = 0; } check(); };
  window.test = function (f, name) { var t = new Test(name); t.step(f, t, t); t.done(); };
  window.async_test = function (name) { return new Test(name); };
  window.promise_test = function (f, name) {
    var t = new Test(name);
    Promise.resolve().then(function () { return f(t); }).then(function () { t.done(); },
      function (e) { if (t.status === null) { t.status = 1; t.message = String(e && e.message || e); } check(); });
  };
  window.assert_true = function (v, m) { if (v !== true) throw new Error('assert_true: ' + (m || '') + ' expected true got ' + v); };
  window.assert_equals = function (a, b, m) { if (a !== b) throw new Error('assert_equals: ' + (m || '') + ' expected ' + b + ' but got ' + a); };
  window.setup = function (o) {
    if (typeof o !== 'function') return;
    try { o(); } catch (e) { harness.status = 1; harness.message = String(e && e.message || e); finish(); }
  };
  window.add_completion_callback = function (f) { cbs.push(f); };
  var timer = setTimeout(function () {
    harness.status = 2;
    tests.forEach(function (t) { if (t.status === null) { t.status = 2; t.message = 'Test timed out'; } });
    finish();
  }, 10000);
  window.addEventListener('load', function () { loaded = true; check(); });
  function check() { if (loaded && !finished && tests.every(function (t) { return t.status !== null; })) finish(); }
  function finish() { if (finished) return; finished = true; clearTimeout(timer); cbs.forEach(function (f) { f(tests, harness); }); }
})();
"#;

fn qwpt4a_page(body: &str) -> String {
    format!("<!doctype html><meta charset=utf-8><script src=/resources/testharness.js></script><script src=/resources/testharnessreport.js></script><div id=log></div><script>{body}</script>")
}

/// Fixture name -> page body.
const QWPT4A_FIXTURES: &[(&str, &str)] = &[
    ("pass.html", "test(function () { assert_true(true); }, 'ok');"),
    ("fail.html", "test(function () { assert_equals(1, 2); }, 'bad'); test(function () {}, 'good');"),
    ("error.html", "setup(function () { throw new Error('boom'); }); test(function () {}, 'after');"),
    (
        "timeout.html",
        "async_test('never').step(function () {}); var t = async_test('at 5 s'); setTimeout(t.step_func_done(), 5000);",
    ),
    (
        "fetch.html",
        "var ticked = false; setTimeout(function () { ticked = true; }, 0);\
         promise_test(function () { return fetch('slow.txt').then(function (r) { return r.text(); })\
           .then(function (t) { assert_equals(t, 'slow body'); assert_true(ticked, 'a timer ran while the fetch was pending'); }); }, 'slow fetch');",
    ),
    ("viewport.html", "test(function () { assert_equals(innerWidth, 800); assert_equals(innerHeight, 600); assert_true(matchMedia('(max-width: 800px)').matches); }, 'vp');"),
];

/// Writes the mini checkout under a fresh temp dir; `harness` is resources/testharness.js.
fn qwpt4a_checkout(tag: &str, harness: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("scratchy-qwpt4a-{tag}-{}-{:?}", std::process::id(), thread::current().id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("resources")).unwrap();
    fs::create_dir_all(d.join("t")).unwrap();
    fs::write(d.join("resources/testharness.js"), harness).unwrap();
    fs::write(d.join("resources/testharnessreport.js"), "throw new Error('the override must be served, not this');").unwrap();
    fs::write(d.join("t/slow.txt"), "slow body").unwrap();
    for (name, body) in QWPT4A_FIXTURES {
        fs::write(d.join("t").join(name), qwpt4a_page(body)).unwrap();
    }
    d
}

/// A fake `wire --mount http://web-platform.test:8000=DIR --mount-override
/// /resources/testharnessreport.js=tools/wpt/testharnessreport.js`; `slow.txt` answers after 400 ms.
fn qwpt4a_fake_wire(sock: &std::path::Path, root: PathBuf) {
    use std::os::unix::net::UnixListener;
    let _ = fs::remove_file(sock);
    let listener = UnixListener::bind(sock).expect("bind fake wire");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let root = root.clone();
            thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match Read::read(&mut stream, &mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let target = text.lines().next().unwrap_or("").split(' ').nth(1).unwrap_or("").to_string();
                let path = Url::parse(&target).map(|u| u.path().to_string()).unwrap_or_default();
                let body = if path == "/resources/testharnessreport.js" {
                    Some(REPORT_JS.as_bytes().to_vec())
                } else {
                    if path.ends_with("slow.txt") {
                        thread::sleep(std::time::Duration::from_millis(400));
                    }
                    fs::read(root.join(path.trim_start_matches('/'))).ok()
                };
                let ct = if path.ends_with(".js") { "text/javascript" } else if path.ends_with(".html") { "text/html" } else { "text/plain" };
                let resp = match body {
                    Some(b) => [format!("HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nX-Wire-Version: 3\r\n\r\n").into_bytes(), b].concat(),
                    None => b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nX-Wire-Version: 3\r\n\r\nnot found".to_vec(),
                };
                let _ = stream.write_all(&resp);
            });
        }
    });
}

/// Runs every fixture through `wpt_th::run_one`; (name, JSON line or None, real time).
fn qwpt4a_run_all(tag: &str, harness: &str) -> Vec<(&'static str, Option<String>, std::time::Duration)> {
    let root = qwpt4a_checkout(tag, harness);
    let sock = unique_test_socket_path(&format!("qwpt4a-{tag}"));
    qwpt4a_fake_wire(&sock, root.clone());
    let exec: Arc<dyn NetExecutor> = Arc::new(crate::js_net::WireExecutor(sock.clone()));
    let mut out = Vec::new();
    for (name, _) in QWPT4A_FIXTURES {
        let url = Url::parse(&format!("http://web-platform.test:8000/t/{name}")).unwrap();
        let t0 = std::time::Instant::now();
        let r = crate::wpt_th::run_one(&url, &sock, exec.clone());
        out.push((*name, r, t0.elapsed()));
    }
    let _ = fs::remove_file(&sock);
    let _ = fs::remove_dir_all(&root);
    out
}

fn qwpt4a_check(results: &[(&str, Option<String>, std::time::Duration)]) {
    let get = |n: &str| results.iter().find(|r| r.0 == n).and_then(|r| r.1.clone()).unwrap_or_else(|| panic!("{n}: no result"));
    // One line of JSON with the three keys.
    for (n, r, _) in results {
        let line = r.as_deref().unwrap_or_else(|| panic!("{n}: no result"));
        assert!(!line.contains('\n') && line.starts_with("{\"harness\":") && line.contains(",\"message\":") && line.contains(",\"tests\":["), "{n}: {line}");
    }
    assert_eq!(get("pass.html"), r#"{"harness":"OK","message":null,"tests":[["ok","PASS",null]]}"#);
    let fail = get("fail.html");
    assert!(fail.starts_with(r#"{"harness":"OK","message":null,"tests":[["bad","FAIL",""#) && fail.ends_with(r#"["good","PASS",null]]}"#), "{fail}");
    assert!(fail.contains("assert_equals"), "{fail}");
    let err = get("error.html");
    assert!(err.starts_with(r#"{"harness":"ERROR","message":""#) && err.contains("boom"), "{err}");
    let to = get("timeout.html");
    assert!(to.starts_with(r#"{"harness":"TIMEOUT","#), "{to}");
    assert!(to.contains(r#"["never","TIMEOUT","#) && to.contains(r#"["at 5 s","PASS",null]"#), "{to}");
    let to_time = results.iter().find(|r| r.0 == "timeout.html").unwrap().2;
    assert!(to_time < std::time::Duration::from_secs(1), "a 10 s harness timeout took {to_time:?} of real time");
    assert_eq!(get("fetch.html"), r#"{"harness":"OK","message":null,"tests":[["slow fetch","PASS",null]]}"#, "a pending load must stop the clock");
    assert_eq!(get("viewport.html"), r#"{"harness":"OK","message":null,"tests":[["vp","PASS",null]]}"#);
}

#[test]
fn qwpt4a_child_reports_pass_fail_error_timeout_and_waits_for_loads() {
    let results = qwpt4a_run_all("standin", STAND_IN_HARNESS);
    qwpt4a_check(&results);
}

/// The same fixtures against WPT's real resources/testharness.js (local runs:
/// `SCRATCHY_WPT_DIR=/path/to/wpt cargo test qwpt4a`); skipped without it.
#[test]
fn qwpt4a_child_with_real_testharness() {
    let Some(dir) = std::env::var_os("SCRATCHY_WPT_DIR") else { return };
    let Ok(harness) = fs::read_to_string(PathBuf::from(dir).join("resources/testharness.js")) else { return };
    let results = qwpt4a_run_all("real", &harness);
    qwpt4a_check(&results);
}

#[test]
fn qwpt4a_no_result_without_the_hook() {
    // The page never loads the vendor hook (a 404 for it): nothing is reported, the parent says CRASH.
    let root = qwpt4a_checkout("nohook", STAND_IN_HARNESS);
    fs::write(root.join("t/nohook.html"), "<script src=/resources/testharness.js></script><script>test(function () {}, 'x');</script>").unwrap();
    let sock = unique_test_socket_path("qwpt4a-nohook");
    qwpt4a_fake_wire(&sock, root.clone());
    let exec: Arc<dyn NetExecutor> = Arc::new(crate::js_net::WireExecutor(sock.clone()));
    let url = Url::parse("http://web-platform.test:8000/t/nohook.html").unwrap();
    assert_eq!(crate::wpt_th::run_one(&url, &sock, exec.clone()), None);
    // A missing page: no result either.
    let url = Url::parse("http://web-platform.test:8000/t/missing.html").unwrap();
    assert_eq!(crate::wpt_th::run_one(&url, &sock, exec), None);
    let _ = fs::remove_file(&sock);
    let _ = fs::remove_dir_all(&root);
}

// ---------- QWPT4b: the runner parent (`--wpt-th`, src/wpt_th_run.rs) ----------

mod qwpt4b {
    use crate::wpt_th_run::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    fn fr(status: &str, passed: usize, total: usize, np: &[(&str, &str)]) -> FileResult {
        FileResult { status: status.into(), passed, total, nonpass: np.iter().map(|(s, n)| (s.to_string(), n.to_string())).collect() }
    }

    #[test]
    fn child_json_parses_with_escapes_and_duplicate_names() {
        let r = parse_child_json(r#"{"harness":"OK","message":null,"tests":[["a\tb\n","PASS",null],["x","FAIL","m \"q\""],["x","FAIL","m"],["x","PASS",null],["é","TIMEOUT",null]]}"#).unwrap();
        assert_eq!((r.status.as_str(), r.passed, r.total), ("OK", 2, 5));
        assert_eq!(r.nonpass, vec![("FAIL".into(), "x".into()), ("FAIL".into(), "x #2".into()), ("TIMEOUT".into(), "\u{e9}".into())]);
        assert!(parse_child_json("not json").is_none());
        assert!(parse_child_json(r#"{"harness":"OK"}"#).is_none());
    }

    #[test]
    fn expected_format_round_trips() {
        let mut m = BTreeMap::new();
        m.insert("dom/a.html".to_string(), fr("OK", 2, 4, &[("FAIL", "tab\there"), ("PRECONDITION_FAILED", "  lead\\ing\nnl")]));
        m.insert("dom/b.html".to_string(), fr("TIMEOUT", 0, 0, &[]));
        m.insert("dom/w.worker.html".to_string(), fr("NOT-RUN", 0, 0, &[]));
        let text = format_expected("dom", &m);
        assert!(text.contains("OK           2/4  dom/a.html\n  FAIL      tab\\there\n"), "{text}");
        assert!(text.contains("NOT-RUN           dom/w.worker.html\n"), "{text}");
        assert_eq!(parse_expected(&text), m);
    }

    #[test]
    fn verdicts() {
        let e = fr("OK", 3, 4, &[("FAIL", "a")]);
        assert_eq!(compare(&e, &e.clone()), Verdict::Same);
        // a passing subtest now fails
        assert!(matches!(compare(&e, &fr("OK", 2, 4, &[("FAIL", "a"), ("FAIL", "b")])), Verdict::Regression(_)));
        // file got worse
        assert!(matches!(compare(&e, &fr("ERROR", 0, 0, &[])), Verdict::Regression(_)));
        assert!(matches!(compare(&e, &fr("CRASH", 0, 0, &[])), Verdict::Regression(_)));
        // subtests appear
        assert!(matches!(compare(&e, &fr("OK", 4, 5, &[("FAIL", "a")])), Verdict::Changed(_)));
        // newly passing
        assert!(matches!(compare(&e, &fr("OK", 4, 4, &[])), Verdict::NewlyPassing(_)));
        assert!(matches!(compare(&fr("ERROR", 1, 2, &[("FAIL", "a")]), &fr("OK", 2, 2, &[])), Verdict::NewlyPassing(_)));
        // FAIL -> TIMEOUT is a change, not a regression of a passing subtest
        assert!(matches!(compare(&e, &fr("OK", 3, 4, &[("TIMEOUT", "a")])), Verdict::Changed(_)));
    }

    #[test]
    fn subset_flaky_worker_and_areas() {
        let s = parse_subset("# x\n# area: dom/nodes\na.html\n\nb.any.worker.html\n# area: url\nc.html\na.html\n");
        assert_eq!(s.iter().map(|e| (e.path.as_str(), e.area.as_str())).collect::<Vec<_>>(), vec![("a.html", "dom/nodes"), ("b.any.worker.html", "dom/nodes"), ("c.html", "url")]);
        assert_eq!(parse_flaky("# c\nx/y.html  # why\n\nz.html"), ["x/y.html", "z.html"].iter().map(|s| s.to_string()).collect());
        assert!(is_worker_variant("fetch/x.any.worker.html") && is_worker_variant("a/b.serviceworker.html?x=1") && !is_worker_variant("a/worker/b.html"));
        let a = fr("OK", 1, 2, &[("FAIL", "n")]);
        let b = fr("OK", 2, 2, &[]);
        let c = fr("CRASH", 0, 0, &[]);
        let row = area_row("dom", 4, &[&a, &b, &c]);
        assert_eq!((row.files, row.ran, row.all_pass, row.subtests, row.pass), (4, 2, 1, 4, 3));
        assert!((row.score - 75.0).abs() < 1e-9, "{}", row.score);
        let text = format_areas(std::slice::from_ref(&row));
        assert!(text.contains("TOTAL"), "{text}");
        let back = parse_areas(&text);
        assert_eq!(back.len(), 1);
        assert_eq!((back[0].files, back[0].pass), (4, 3));
    }

    /// A fake child: a shell script keyed on the URL it is given.
    fn fake(dir: &std::path::Path, script: &str) -> ChildCmd {
        let p = dir.join("child.sh");
        fs::write(&p, script).unwrap();
        ChildCmd { program: "sh".into(), args: vec![p.display().to_string()], verbose: false, origin: "http://web-platform.test:8000".into() }
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("scratchy-qwpt4b-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    const OKJ: &str = r#"{"harness":"OK","message":null,"tests":[["one","PASS",null],["two","FAIL","x"]]}"#;

    #[test]
    fn runner_end_to_end_with_a_fake_child() {
        let d = tmp("e2e");
        let wd = d.join("wpt-th");
        fs::create_dir_all(&wd).unwrap();
        fs::write(wd.join("subset.txt"), "# area: dom\ndom/pass.html\ndom/hang.html\ndom/none.html\ndom/w.any.worker.html\ndom/flaky.html\n# area: url\nurl/p.html\n").unwrap();
        fs::write(wd.join("flaky.txt"), "dom/flaky.html # unstable\n").unwrap();
        let script = format!(
            "case \"$1\" in\n *hang.html) sleep 30;;\n *none.html) ;;\n *flaky.html) echo '{{\"harness\":\"OK\",\"message\":null,\"tests\":[[\"r\",\"PASS\",null]]}}';;\n *) echo '{OKJ}';;\nesac\n"
        );
        let cmd = fake(&d, &script);
        let cfg = |update: bool, area: Option<&str>| Config { dir: wd.clone(), update, area: area.map(String::from), except: None, jobs: 3, timeout: Duration::from_secs(1), verbose: false, wptserve: false };
        // Nothing recorded yet: UNRECORDED blocks.
        assert_eq!(execute(&cfg(false, None), &cmd), 1);
        assert_eq!(execute(&cfg(true, None), &cmd), 0);
        let dom = fs::read_to_string(wd.join("expected/dom.txt")).unwrap();
        assert!(dom.contains("OK           1/2  dom/pass.html\n  FAIL      two\n"), "{dom}");
        assert!(dom.contains("TIMEOUT      0/0  dom/hang.html\n") && dom.contains("CRASH        0/0  dom/none.html\n"), "{dom}");
        assert!(dom.contains("NOT-RUN           dom/w.any.worker.html\n") && !dom.contains("flaky"), "{dom}");
        let areas = fs::read_to_string(wd.join("areas.txt")).unwrap();
        assert!(areas.lines().any(|l| l.starts_with("url ") && l.ends_with("50.0")) && areas.contains("TOTAL"), "{areas}");
        assert_eq!(execute(&cfg(false, None), &cmd), 0);
        // --area only touches that area's row and file.
        assert_eq!(execute(&cfg(false, Some("url")), &cmd), 0);
        assert!(fs::read_to_string(wd.join("areas.txt")).unwrap().lines().any(|l| l.starts_with("dom ")));
        // A passing subtest starts failing: REGRESSION.
        let worse = fake(&d, &script.replace(OKJ, r#"{"harness":"OK","message":null,"tests":[["one","FAIL","x"],["two","FAIL","x"]]}"#));
        assert_eq!(execute(&cfg(false, Some("url")), &worse), 1);
        // A new pass does not block.
        let better = fake(&d, &script.replace(OKJ, r#"{"harness":"OK","message":null,"tests":[["one","PASS",null],["two","PASS",null]]}"#));
        assert_eq!(execute(&cfg(false, Some("url")), &better), 0);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_hung_child_is_killed_and_recorded_timeout() {
        let d = tmp("kill");
        let cmd = fake(&d, "sleep 30\n");
        let e = [Entry { path: "x.html".into(), area: "a".into() }];
        let t0 = std::time::Instant::now();
        let r = run_all(&cmd, &e, 1, Duration::from_millis(300));
        assert_eq!(r[0].status, "TIMEOUT");
        assert!(t0.elapsed() < Duration::from_secs(5));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn areas_only_on_a_fixture_matches_a_full_run_rows() {
        let d = tmp("areas");
        fs::create_dir_all(d.join("expected")).unwrap();
        fs::write(d.join("subset.txt"), "# area: a/b\na1.html\na2.html\na3.html\na4.html\n# area: c\nc1.html\n").unwrap();
        fs::write(d.join("flaky.txt"), "a3.html unstable\n").unwrap();
        let mut ab = BTreeMap::new();
        ab.insert("a1.html".to_string(), fr("OK", 2, 2, &[]));
        ab.insert("a2.html".to_string(), fr("OK", 1, 4, &[("FAIL", "x")]));
        fs::write(d.join("expected").join("a-b.txt"), format_expected("a/b", &ab)).unwrap();
        let mut c = BTreeMap::new();
        c.insert("c1.html".to_string(), fr("TIMEOUT", 0, 0, &[]));
        fs::write(d.join("expected").join("c.txt"), format_expected("c", &c)).unwrap();
        let got = areas_from_dir(&d).unwrap();
        let r1 = ab.values().collect::<Vec<_>>();
        let want = format_areas(&[area_row("a/b", 4, &r1), area_row("c", 1, &c.values().collect::<Vec<_>>())]);
        assert_eq!(got, want);
        let rows = parse_areas(&got);
        assert_eq!((rows[0].files, rows[0].ran, rows[0].all_pass, rows[0].subtests, rows[0].pass), (4, 2, 1, 6, 3));
        let _ = fs::remove_dir_all(&d);
    }

    /// The committed wpt-th/ reproduces its committed areas.txt byte for byte.
    #[test]
    fn areas_only_reproduces_the_committed_areas_txt() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("wpt-th");
        let committed = fs::read_to_string(dir.join("areas.txt")).unwrap();
        assert_eq!(areas_from_dir(&dir).unwrap(), committed);
    }
}
