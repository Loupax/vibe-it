// QEL4: "report the exception" for timer, rAF and observer callbacks (ErrorEvent at window / onerror,
// then the log line; the other callbacks still run).
#[allow(unused_imports)]
use super::*;

const HTML: &str = "<html><body style='margin:0'><div id=a style='width:100px;height:50px'></div></body></html>";

fn run(code: &str) -> Vec<String> {
    run_html_with_timers(HTML, code)
}

#[test]
fn qel4_throwing_timeout_fires_error_event_and_onerror() {
    let log = run(
        "window.addEventListener('error', e => console.log('ev', e.message, e.error && e.error.message));\
         window.onerror = function (m, f, l, c, e) { console.log('onerror', typeof m, arguments.length, e.message); };\
         setTimeout(function () { throw new Error('boom'); }, 0);\
         setTimeout(function () { console.log('next'); }, 0);",
    );
    assert!(log.iter().any(|l| l == "log:ev Uncaught Error: boom boom"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:onerror string 5 boom"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:next"), "{log:?}");
}

#[test]
fn qel4_interval_keeps_running_after_a_throw() {
    let log = run(
        "var n = 0, id = setInterval(function () { n++; if (n == 3) { clearInterval(id); console.log('done'); } throw new Error('x' + n); }, 0);\
         window.onerror = function (m) { console.log('err', m); return true; };",
    );
    assert!(log.iter().any(|l| l == "log:err Uncaught Error: x1"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:err Uncaught Error: x3"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:done"), "{log:?}");
}

#[test]
fn qel4_onerror_true_suppresses_the_log_line() {
    let quiet = run("window.onerror = () => true; setTimeout(() => { throw new Error('quiet'); }, 0);");
    assert!(!quiet.iter().any(|l| l.contains("quiet")), "{quiet:?}");
    let loud = run("window.onerror = () => false; setTimeout(() => { throw new Error('loud'); }, 0);");
    assert!(loud.iter().any(|l| l.contains("Uncaught Error: loud")), "{loud:?}");
}

#[test]
fn qel4_throwing_raf_does_not_stop_the_next_callback() {
    let log = run(
        "window.onerror = function (m) { console.log('err', m); return true; };\
         requestAnimationFrame(() => { throw new Error('r1'); });\
         requestAnimationFrame(() => console.log('second'));",
    );
    assert!(log.iter().any(|l| l == "log:err Uncaught Error: r1"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:second"), "{log:?}");
}

#[test]
fn qel4_throwing_mutation_observer_is_reported() {
    let log = run(
        "window.onerror = function (m) { console.log('err', m); return true; };\
         var a = document.getElementById('a');\
         new MutationObserver(() => { throw new Error('mo'); }).observe(a, { attributes: true });\
         new MutationObserver(() => console.log('mo2')).observe(a, { attributes: true });\
         a.setAttribute('x', '1');",
    );
    assert!(log.iter().any(|l| l == "log:err Uncaught Error: mo"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:mo2"), "{log:?}");
}

#[test]
fn qel4_throwing_resize_and_intersection_observers_are_reported() {
    let log = run(
        "window.onerror = function (m) { console.log('err', m); return true; };\
         var a = document.getElementById('a');\
         new ResizeObserver(() => { throw new Error('ro'); }).observe(a);\
         new IntersectionObserver(() => { throw new Error('io'); }).observe(a);",
    );
    assert!(log.iter().any(|l| l == "log:err Uncaught Error: ro"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:err Uncaught Error: io"), "{log:?}");
}
