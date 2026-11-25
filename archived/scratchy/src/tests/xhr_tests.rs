#[allow(unused_imports)]
use super::*;

#[test]
fn qf9_get_event_order_and_response_fields() {
    let fake = QfFake::new(vec![("/a", qf7_ok("hello", &[("content-type", "text/plain"), ("content-length", "5")]))]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); console.log(typeof XMLHttpRequest, XMLHttpRequest.DONE, x.UNSENT, x.readyState, x instanceof EventTarget);\
         track(x); x.open('get', '/a'); x.send();\
         x.addEventListener('loadend', function () { console.log('final', x.status, x.statusText, x.responseURL, x.responseText, x.response, x.getResponseHeader('Content-Type')); });",
    );
    let want = [
        "log:function 4 0 0 true",
        "log:rs1",
        "log:loadstart 0 0 false",
        "log:rs2",
        "log:rs3",
        "log:progress 5 5 true",
        "log:rs4",
        "log:load 5 5 true",
        "log:loadend 5 5 true",
        "log:final 200 OK https://page.test/a hello hello text/plain",
    ];
    assert_eq!(l, want, "{l:?}");
}

#[test]
fn qf9_post_body_and_headers_reach_the_executor_and_forbidden_is_ignored() {
    let fake = QfFake::new(vec![("/p", qf7_ok("{}", &[]))]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); x.open('POST', '/p');\
         x.setRequestHeader('X-Tok', 'a'); x.setRequestHeader('x-tok', 'b'); x.setRequestHeader('Cookie', 'evil=1'); x.setRequestHeader('Host', 'evil');\
         try { x.send('payload'); x.setRequestHeader('X-Late', '1'); } catch (e) { console.log('late', e.name); }\
         try { new XMLHttpRequest().setRequestHeader('A', 'b'); } catch (e) { console.log('unopened', e.name); }",
    );
    assert!(l.contains(&"log:late InvalidStateError".to_string()), "{l:?}");
    assert!(l.contains(&"log:unopened InvalidStateError".to_string()), "{l:?}");
    let raw = String::from_utf8_lossy(&fake.raw.lock().unwrap()[0]).to_string();
    let lower = raw.to_lowercase();
    assert!(raw.starts_with("POST https://page.test/p"), "{raw}");
    assert!(lower.contains("x-tok: a, b"), "{raw}");
    assert!(lower.contains("content-type: text/plain;charset=utf-8"), "{raw}");
    assert!(raw.ends_with("payload"), "{raw}");
    assert!(!lower.contains("evil"), "{raw}");
}

#[test]
fn qf9_get_ignores_the_body_and_open_validates_the_method() {
    let fake = QfFake::new(vec![("/g", qf7_ok("x", &[]))]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); x.open('GET', '/g'); x.send('ignored');\
         ['TRACE','connect','Track'].forEach(function (m) { try { new XMLHttpRequest().open(m, '/'); } catch (e) { console.log(m, e.name); } });\
         try { new XMLHttpRequest().open('bad method', '/'); } catch (e) { console.log('bad', e.name); }",
    );
    assert!(l.contains(&"log:TRACE SecurityError".to_string()) && l.contains(&"log:connect SecurityError".to_string()), "{l:?}");
    assert!(l.contains(&"log:bad SyntaxError".to_string()), "{l:?}");
    let raw = String::from_utf8_lossy(&fake.raw.lock().unwrap()[0]).to_string();
    assert!(raw.starts_with("GET ") && !raw.contains("ignored"), "{raw}");
}

#[test]
fn qf9_response_types() {
    let fake = QfFake::new(vec![("/j", qf7_ok("{\"a\":[1,2]}", &[("content-type", "application/json")]))]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "function go(t, f) { var x = new XMLHttpRequest(); x.open('GET', '/j'); x.responseType = t;\
           x.onload = function () { f(x); }; x.send(); }\
         go('json', function (x) { console.log('json', JSON.stringify(x.response), x.responseType); });\
         go('arraybuffer', function (x) { console.log('ab', x.response instanceof ArrayBuffer, x.response.byteLength); });\
         go('blob', function (x) { console.log('blob', x.response instanceof Blob, x.response.size, x.response.type); });\
         go('text', function (x) { console.log('text', x.response, x.responseText.length); });\
         go('json', function (x) { try { x.responseText; } catch (e) { console.log('rt', e.name); } });",
    );
    assert!(l.contains(&"log:json {\"a\":[1,2]} json".to_string()), "{l:?}");
    assert!(l.contains(&"log:ab true 11".to_string()), "{l:?}");
    assert!(l.contains(&"log:blob true 11 application/json".to_string()), "{l:?}");
    assert!(l.contains(&"log:text {\"a\":[1,2]} 11".to_string()), "{l:?}");
    assert!(l.contains(&"log:rt InvalidStateError".to_string()), "{l:?}");
}

#[test]
fn qf9_get_all_response_headers_format() {
    let fake = QfFake::new(vec![(
        "/h",
        qf7_ok("", &[("X-B", "2"), ("Content-Type", "text/plain"), ("Set-Cookie", "s=1"), ("x-a", "1"), ("X-B", "3")]),
    )]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); console.log(JSON.stringify(x.getAllResponseHeaders()), x.getResponseHeader('x-a'));\
         x.open('GET', '/h'); x.onload = function () { console.log(JSON.stringify(x.getAllResponseHeaders()), x.getResponseHeader('X-B'), x.getResponseHeader('set-cookie'), x.getResponseHeader('nope')); }; x.send();",
    );
    assert_eq!(l[0], "log:\"\" null");
    assert_eq!(l[1], "log:\"content-type: text/plain\\r\\nx-a: 1\\r\\nx-b: 2, 3\\r\\n\" 2, 3 null null", "{l:?}");
}

#[test]
fn qf9_abort_fires_abort_and_loadend_then_unsent() {
    let fake = QfFake::new(vec![("/a", qf7_ok("late", &[]))]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); track(x); x.open('GET', '/a'); x.send(); x.abort(); console.log('after', x.readyState, x.status);\
         var y = new XMLHttpRequest(); y.abort(); console.log('idle', y.readyState);",
    );
    let want = ["log:rs1", "log:loadstart 0 0 false", "log:rs4", "log:abort 0 0 false", "log:loadend 0 0 false", "log:after 0 0", "log:idle 0"];
    assert_eq!(l, want, "{l:?}");
}

#[test]
fn qf9_timeout_fires_on_the_manual_clock() {
    let fake = QfFake::new(vec![]);
    set_page_url(Some(&Url::parse("https://page.test/").unwrap()));
    crate::js_net::set_net_executor(fake.clone());
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html(""),
        &[src(&format!(
            "{QF9_TRACK}var x = new XMLHttpRequest(); x.open('GET', '/slow'); x.timeout = 250; track(x); x.send();\
             console.log('t', x.timeout);"
        ))],
        Rc::new(move |l, t| o.borrow_mut().push(format!("{l}:{t}"))),
        Some(clock.clone()),
    )
    .unwrap();
    // The request is queued but never delivered: only the timer can end it.
    let due = engine.next_due_ms().expect("a timeout timer");
    assert!((249.0..=251.0).contains(&due), "{due}");
    clock.set(due);
    engine.run_due();
    let l = out.borrow().clone();
    assert_eq!(l, ["log:loadstart 0 0 false", "log:t 250", "log:rs4", "log:timeout 0 0 false", "log:loadend 0 0 false"], "{l:?}");
}

#[test]
fn qf9_cors_failure_is_an_error_event_with_status_0() {
    let fake = QfFake::new(vec![("/closed", qf7_ok("secret", &[]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        &format!(
            "{QF9_TRACK}var x = new XMLHttpRequest(); x.open('GET', 'https://other.test/closed'); track(x); x.send();\
             x.addEventListener('loadend', function () {{ console.log('end', x.status, JSON.stringify(x.responseText)); }});\
             var p = new XMLHttpRequest(); p.open('PUT', 'https://other.test/pf'); p.onerror = function () {{ console.log('pf error'); }}; p.send('x');"
        ),
    );
    qf7_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(l.contains(&"log:error 0 0 false".to_string()), "{l:?}");
    assert!(l.contains(&"log:end 0 \"\"".to_string()), "{l:?}");
    assert!(l.contains(&"log:pf error".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.contains("GET https://other.test/closed: CORS: no Access-Control-Allow-Origin")), "{l:?}");
    assert!(l.iter().any(|x| x.contains("CORS preflight failed: https://other.test/pf")), "{l:?}");
}

#[test]
fn qf9_404_still_fires_load() {
    let fake = QfFake::new(vec![(
        "/nf",
        Ok(WireResponse { status: 404, status_text: "Not Found".into(), headers: vec![], body: b"nope".to_vec(), error: None, final_url: None }),
    )]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); x.open('GET', '/nf'); x.onload = function () { console.log('load', x.status, x.statusText, x.responseText); };\
         x.onerror = function () { console.log('error'); }; x.send();",
    );
    assert_eq!(l, ["log:load 404 Not Found nope"], "{l:?}");
}

#[test]
fn qf9_sync_returns_synchronously_and_logs() {
    let fake = QfFake::new(vec![("/s", qf7_ok("sync-body", &[("content-type", "text/plain")]))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        &format!(
            "{QF9_TRACK}var x = new XMLHttpRequest(); x.open('GET', '/s', false); track(x); x.send(); console.log('after-send', x.readyState, x.status, x.responseText);\
             var r = new XMLHttpRequest(); r.open('GET', '/s', false);\
             try {{ r.responseType = 'json'; }} catch (e) {{ console.log('rt', e.name); }}\
             try {{ r.timeout = 5; }} catch (e) {{ console.log('to', e.name); }}\
             var q = new XMLHttpRequest(); q.responseType = 'blob';\
             try {{ q.open('GET', '/s', false); }} catch (e) {{ console.log('open', e.name); }}"
        ),
    );
    let l = log.borrow().clone();
    assert!(l.contains(&"log:after-send 4 200 sync-body".to_string()), "{l:?}");
    let pos = |s: &str| l.iter().position(|x| x == s).unwrap_or_else(|| panic!("{s} in {l:?}"));
    assert!(pos("log:rs4") < pos("log:load 9 9 true") && pos("log:load 9 9 true") < pos("log:after-send 4 200 sync-body"), "{l:?}");
    assert!(l.iter().any(|x| x.contains("sync GET https://page.test/s: blocking the page")), "{l:?}");
    assert!(l.contains(&"log:rt InvalidAccessError".to_string()) && l.contains(&"log:to InvalidAccessError".to_string()), "{l:?}");
    assert!(l.contains(&"log:open InvalidAccessError".to_string()), "{l:?}");
    assert_eq!(fake.urls().len(), 1);
}

#[test]
fn qf9_sync_failure_throws_network_error() {
    let fake = QfFake::new(vec![]);
    let l = qf9_run(
        "https://page.test/",
        &fake,
        "var x = new XMLHttpRequest(); x.open('GET', '/nothing', false);\
         try { x.send(); console.log('no throw'); } catch (e) { console.log('thrown', e.name, e instanceof DOMException, x.readyState, x.status); }",
    );
    assert!(l.contains(&"log:thrown NetworkError true 4 0".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.contains("sync GET https://page.test/nothing")), "{l:?}");
}

#[test]
fn qf9_form_data_urlsearchparams_and_arraybuffer_bodies() {
    let fake = QfFake::new(vec![("/b", qf7_ok("", &[]))]);
    qf9_run(
        "https://page.test/",
        &fake,
        "var fd = new FormData(); fd.append('k', 'v');\
         [fd, new URLSearchParams('a=1'), new Uint8Array([65, 66]).buffer].forEach(function (b) { var x = new XMLHttpRequest(); x.open('POST', '/b'); x.send(b); });",
    );
    let raws: Vec<String> = fake.raw.lock().unwrap().iter().map(|r| String::from_utf8_lossy(r).to_lowercase()).collect();
    assert!(raws[0].contains("content-type: multipart/form-data; boundary=") && raws[0].contains("name=\"k\""), "{}", raws[0]);
    assert!(raws[1].contains("application/x-www-form-urlencoded") && raws[1].ends_with("a=1"), "{}", raws[1]);
    assert!(raws[2].ends_with("ab") && !raws[2].contains("content-type"), "{}", raws[2]);
}
