// QSCR404: a script that fails to load (non-2xx status, network error) is not run and its element gets `error`.
#[allow(unused_imports)]
use super::*;
use super::js_core_tests::run_module_page;
use super::js_geometry_tests::{qy34_run, qy34_status};

// ---- QSCR404: a script that fails to load (404/500/network error = the fetcher answers None) is not run
// and its element gets `error`, not `load`; the scripts after it still run.

#[test]
fn qscr404_parser_inserted_classic_script_that_fails_fires_error_not_load() {
    let log = run_module_page(
        "<script src=\"/gone.js\" onerror=\"console.log('onerror attr')\" onload=\"console.log('onload attr')\"></script>\
         <script async src=\"/gone2.js\" onerror=\"console.log('async error')\"></script>\
         <script defer src=\"/gone3.js\" onerror=\"console.log('defer error')\"></script>\
         <script src=\"/ok.js\"></script>\
         <script>console.log('inline ran');</script>",
        &[("https://example.test/ok.js", "console.log('ok ran');")],
    );
    let has = |s: &str| log.iter().any(|l| l == s);
    assert!(has("log:onerror attr") && has("log:async error") && has("log:defer error"), "{log:?}");
    assert!(!has("log:onload attr"), "{log:?}");
    assert!(has("log:ok ran") && has("log:inline ran"), "{log:?}");
    assert!(log.iter().all(|l| !l.contains("SyntaxError")), "{log:?}");
    assert!(log.iter().any(|l| l.starts_with("error:https://example.test/gone.js: failed to load script")), "{log:?}");
}

#[test]
fn qscr404_failed_module_script_and_failed_static_import_fire_error_and_do_not_run() {
    let log = run_module_page(
        "<script type=module src=\"/gone.js\" onerror=\"console.log('error gone')\"></script>\
         <script type=module src=\"/m.js\" onerror=\"console.log('error m')\" onload=\"console.log('load m')\"></script>\
         <script type=module onerror=\"console.log('error inline')\">import './dep404.js'; console.log('inline ran');</script>\
         <script type=module src=\"/good.js\" onerror=\"console.log('error good')\"></script>",
        &[
            ("https://example.test/m.js", "import { x } from './dep.js'; console.log('m ran', x);"),
            ("https://example.test/dep.js", "import './deep404.js'; export const x = 1;"),
            ("https://example.test/good.js", "console.log('good ran');"),
        ],
    );
    let has = |s: &str| log.iter().any(|l| l == s);
    assert!(has("log:error gone") && has("log:error m") && has("log:error inline"), "{log:?}");
    assert!(has("log:good ran") && !has("log:error good"), "{log:?}");
    assert!(!has("log:inline ran") && !has("log:m ran 1") && !has("log:load m"), "{log:?}");
}

#[test]
fn qscr404_dynamic_import_of_a_missing_module_rejects_and_does_not_error_the_script() {
    let log = run_module_page(
        "<script type=module onerror=\"console.log('script error')\">import('./gone.js').then(function () { console.log('imported'); }, function () { console.log('rejected'); });</script>",
        &[],
    );
    assert!(log.contains(&"log:rejected".to_string()) && !log.contains(&"log:script error".to_string()), "{log:?}");
}

// ---------- QSCR404: dynamic module scripts and imports that fail to load ----------

#[test]
fn qscr404_dynamic_module_404_and_failed_static_import_fire_error_not_load() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![
            ("/m404.js", qy34_status(404, "Not found")),
            ("/m.js", qy34_status(200, "import './gone.js'; console.log('m ran');")),
            ("/gone.js", qy34_status(500, "oops")),
        ],
        "['/m404.js', '/m.js'].forEach(function (u) {\
           var s = document.createElement('script'); s.type = 'module'; s.src = u; s.async = true;\
           s.onload = function () { console.log('load', u); }; s.onerror = function () { console.log('error', u); };\
           document.head.appendChild(s); });",
    );
    assert!(l.contains(&"log:error /m404.js".to_string()), "{l:?}");
    assert!(l.contains(&"log:error /m.js".to_string()), "{l:?}");
    assert!(!l.iter().any(|x| x.starts_with("log:load") || x == "log:m ran"), "{l:?}");
}

#[test]
fn qscr404_dynamic_import_of_a_404_rejects() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![("/gone.js", qy34_status(404, "Not found"))],
        "var s = document.createElement('script'); s.type = 'module';\
         s.textContent = \"import('/gone.js').then(function () { console.log('imported'); }, function () { console.log('rejected'); });\";\
         document.head.appendChild(s);",
    );
    assert!(l.contains(&"log:rejected".to_string()), "{l:?}");
    assert!(!l.contains(&"log:imported".to_string()), "{l:?}");
}

#[test]
fn qscr404_dynamic_async_and_defer_classic_404_fire_error() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![("/a.js", qy34_status(404, "Not found")), ("/d.js", qy34_status(500, "Not found"))],
        "[['/a.js', 'async'], ['/d.js', 'defer']].forEach(function (p) {\
           var s = document.createElement('script'); s.src = p[0]; s[p[1]] = true;\
           s.onload = function () { console.log('load', p[0]); }; s.onerror = function () { console.log('error', p[0]); };\
           document.head.appendChild(s); });",
    );
    assert!(l.contains(&"log:error /a.js".to_string()) && l.contains(&"log:error /d.js".to_string()), "{l:?}");
    assert!(!l.iter().any(|x| x.starts_with("log:load")), "{l:?}");
}
#[test]
fn qscr404_fetch_script_via_wire_refuses_a_non_2xx_status() {
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_js_enabled(true);
    let top = Url::parse("https://www.top.example/page").unwrap();
    let u = Url::parse("https://cdn.third.test/a.js").unwrap();
    for (name, resp) in [
        ("404", &b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 9\r\n\r\nNot found"[..]),
        ("500", &b"HTTP/1.1 500 Oops\r\nContent-Type: application/javascript\r\nContent-Length: 8\r\n\r\nvar a=1;"[..]),
        ("200", &b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: 8\r\n\r\nvar a=1;"[..]),
    ] {
        let path = unique_test_socket_path(&format!("qscr404-{name}"));
        spawn_mock_wire_once(&path, resp);
        let got = fetch_script_via_wire(&path, &u, &top);
        if name == "200" {
            assert_eq!(got.as_deref(), Some("var a=1;"));
        } else {
            assert_eq!(got, None, "status {name} must not yield a script");
        }
    }
}
