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
