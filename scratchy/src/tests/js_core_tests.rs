#[allow(unused_imports)]
use super::*;

#[test]
fn q33a_scripts_run_in_order_then_dom_content_loaded_then_load() {
    let log = run_collecting(&[
        src("console.log('a'); document.addEventListener('DOMContentLoaded', function(){ console.log('dcl:' + document.readyState); });"),
        src("window.addEventListener('load', function(){ console.log('load:' + document.readyState); }); window.onload = function(){ console.log('onload'); }; console.log('b');"),
    ]);
    assert_eq!(log, ["log:a", "log:b", "log:dcl:interactive", "log:load:complete", "log:onload"]);
}

#[test]
fn q33a_script_errors_are_logged_and_not_fatal() {
    let log = run_collecting(&[
        src("throw new Error('boom');"),
        src("syntax error here ("),
        src("Promise.resolve().then(function(){ console.log('job'); }); console.warn('still', 1, {x: 2});"),
    ]);
    assert!(log[0].starts_with("error:t: Error: boom"), "{log:?}");
    assert!(log[1].starts_with("error:t: "), "{log:?}");
    assert_eq!(&log[2..], ["warn:still 1 {\"x\":2}", "log:job"]);
}

#[test]
fn qj1_classic_scripts_run_in_sloppy_mode() {
    let log = run_collecting(&[
        src("x = 1; console.log(typeof x)"),
        src("console.log((function(){ return this === globalThis })())"),
        src("\"use strict\"; y = 1"),
    ]);
    assert_eq!(&log[..2], ["log:number", "log:true"], "{log:?}");
    assert_eq!(log.len(), 3, "{log:?}");
    assert!(log[2].starts_with("error:t: "), "{log:?}");
}

#[test]
fn qj2_error_lines_carry_name_message_and_script_number() {
    let log = run_collecting(&[
        src("window.addEventListener('load', function(){ throw new TypeError('boom'); });"),
        ScriptSource { name: "inline script #2".into(), code: "throw new RangeError('r');".into(), index: None, ..Default::default() },
        src("throw 'plain'"),
        src("throw {a: 1}"),
    ]);
    assert!(log.iter().any(|l| l.contains("Uncaught TypeError: boom")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("inline script #2: RangeError: r") && l.contains("inline script #2:1")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("t: Uncaught plain")), "{log:?}");
    assert!(log.iter().all(|l| !l.contains("Value(")), "{log:?}");
}

#[test]
fn qj2_inline_scripts_are_numbered_by_document_position() {
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let doc = parse_html("<script src=a.js></script><script>1</script><script>2</script>");
    let names: Vec<_> = inline_scripts(&doc).into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["inline script #2", "inline script #3"]);
}

#[test]
fn q33a_runaway_script_is_interrupted() {
    let start = std::time::Instant::now();
    let log = run_collecting(&[src("while(true){}"), src("console.log('after');")]);
    assert!(start.elapsed() < std::time::Duration::from_secs(30));
    assert!(log[0].starts_with("error:js: t stopped"), "{log:?}");
}

#[test]
fn q33a_classic_script_types_and_collection_order() {
    // JS_ENABLED is process-wide; tests that read or flip it hold this lock (main CI flake 4608cd4).
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let doc = parse_html(
        "<html><head><script>var a=1;</script><script src=\"x.js\"></script>\
         <script type=\"module\">import 'z';</script><script type=\"application/json\">{}</script>\
         <script type=\"text/javascript\">var b=2;</script><script> </script></head></html>",
    );
    let mut res = ExternalResources::default();
    find_external_resources(&doc, &mut res);
    assert_eq!(
        res.scripts,
        [
            ScriptRef::Inline("var a=1;".into()),
            ScriptRef::External("x.js".into()),
            // QY15: module scripts are collected now.
            ScriptRef::ModuleInline("import 'z';".into()),
            ScriptRef::Inline("var b=2;".into())
        ]
    );
    assert_eq!(inline_scripts(&doc).len(), 3);
}

#[test]
fn q33a_no_js_flag_and_set_command_toggle() {
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert!(SET_ARGS.contains(&"js") && SET_ARGS.contains(&"nojs"));
    set_js_enabled(false);
    assert!(!js_enabled());
    let base = Url::parse("http://example.test/").unwrap();
    assert!(fetch_scripts(std::path::Path::new("/nonexistent"), &base, &base, vec![ScriptRef::Inline("1".into())]).is_empty());
    set_js_enabled(true);
    assert!(js_enabled());
    assert_eq!(fetch_scripts(std::path::Path::new("/nonexistent"), &base, &base, vec![ScriptRef::Inline("1".into())]).len(), 1);
}

#[test]
fn q33a_noscript_content_hidden_only_when_js_is_on() {
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    fn has_size(l: &Laid, w: f32, h: f32) -> bool {
        (l.w == w && l.h == h) || matches!(&l.content, LaidContent::Blocks(c) if c.iter().any(|k| has_size(k, w, h)))
    }
    let visible = |js: bool| -> bool {
        set_js_enabled(js);
        let doc = parse_html("<html><body style=\"margin:0\"><noscript><div style=\"width:123px;height:45px\"></div></noscript></body></html>");
        let tree = build_layout_tree(&doc, 1000.0);
        let mut fs = new_font_system_with(&[]);
        let mut cache = ShapeCache::new();
        let (mut next_id, mut incomplete) = (0usize, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let laid = measure(&tree, &mut fs, 0.0, 0.0, 1000.0, None, &mut next_id, &mut cache, deadline, &mut incomplete, &mut Vec::new(), &Floats::default(), None);
        has_size(&laid, 123.0, 45.0)
    };
    let (on, off) = (visible(true), visible(false));
    set_js_enabled(true);
    assert!(!on, "noscript content must be hidden with JS on");
    assert!(off, "noscript content must show with JS off");
}

#[test]
fn qj3_js_report_counts_scripts_and_errors() {
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let html = "<html><body><script>console.log('ok');</script><script>throw new Error('boom');</script></body></html>";
    let report = js::js_report_for_html(html, None, "none.html");
    assert!(report.contains("scripts run: 2\n"), "{report}");
    assert!(report.contains("errors total: 1\n"), "{report}");
    assert!(report.contains("boom"), "{report}");
}

#[test]
fn q33b_get_element_by_id_and_attributes() {
    let log = run_in_html(
        "<html><body><div id=a class='x y' data-k=v>hi <b>there</b></div></body></html>",
        "var e=document.getElementById('a');\
         console.log(e.tagName, e.id, e.className, e.getAttribute('data-k'), e.hasAttribute('nope'), e.getAttribute('nope'), e.textContent);\
         console.log(document.getElementById('zz'), e===document.getElementById('a'), e.classList.contains('y'), e.classList.contains('q'));",
    );
    assert_eq!(log, ["log:DIV a x y v false null hi there", "log:null true true false"], "{log:?}");
}

#[test]
fn q33b_query_selector_and_tree_walk() {
    let log = run_in_html(
        "<html><head></head><body><ul id=l><li class=i>1</li><li>2</li><li class=i>3</li></ul></body></html>",
        "var l=document.querySelector('#l');\
         console.log(document.querySelectorAll('li.i').length, l.querySelector('li').textContent, l.children.length);\
         console.log(l.parentNode===document.body, document.documentElement.tagName, document.head.tagName, l.firstElementChild.nextSibling.textContent);\
         console.log(l.querySelector('li:nth-child(2)').matches('li'), l.closest('body')===document.body, document.querySelector('p'));\
         try { document.querySelector('!!'); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        ["log:2 1 3", "log:true HTML HEAD 2", "log:true true null", "log:SyntaxError"],
        "{log:?}"
    );
}
// ---------- Q33b1: startup globals ----------

#[test]
fn q33b1_globals_feature_test_and_polyfill() {
    set_page_url(Url::parse("https://example.org:8080/a/b?x=1#frag").ok().as_ref());
    let out = run_collecting(&[src(
        "var r = [];\
         r.push(typeof Node, typeof Element, typeof HTMLElement, typeof Document, typeof Event, typeof CustomEvent, typeof Image);\
         Element.prototype.polyfilled = Element.prototype.polyfilled || function () { return true; };\
         r.push(Element.prototype.polyfilled() === true);\
         r.push(new Image() instanceof Element, new Image(3,4).width, document instanceof Document);\
         var e = new CustomEvent('x', { detail: 5, cancelable: true }); e.preventDefault();\
         r.push(e.type, e.detail, e.defaultPrevented, e instanceof Event);\
         r.push(navigator.userAgent.indexOf('scratchy') > 0, navigator.language);\
         r.push(innerWidth > 0, window.innerHeight > 0);\
         r.push(document.readyState, document.URL, document.cookie); document.cookie = 'a=b'; r.push(document.cookie === '');\
         r.push(location.href, location.protocol, location.host, location.hostname, location.port, location.pathname, location.search, location.hash, location.origin, String(location));\
         console.log(r.join('|'));",
    )]);
    assert_eq!(
        out,
        vec![
            "log:function|function|function|function|function|function|function|true|true|3|true|x|5|true|true|true|en-US|true|true|loading|https://example.org:8080/a/b?x=1#frag||true|https://example.org:8080/a/b?x=1#frag|https:|example.org:8080|example.org|8080|/a/b|?x=1|#frag|https://example.org:8080|https://example.org:8080/a/b?x=1#frag"
        ]
    );
}

#[test]
fn q33b1_no_url_gives_about_blank() {
    set_page_url(None);
    let out = run_collecting(&[src("console.log(location.href, location.protocol, typeof document.title)")]);
    assert_eq!(out, vec!["log:about:blank about: string"]);
}

#[test]
fn q33b1_wrappers_are_instances_of_the_constructors() {
    let out = run_collecting(&[src(
        "var b = document.body;\
         Element.prototype.myPoly = function () { return this.tagName; };\
         console.log([b instanceof Element, b instanceof HTMLElement, b instanceof Node, document.documentElement instanceof Element,\
           document.head.myPoly(), document instanceof Node, typeof document.getElementById].join('|'));",
    )]);
    assert_eq!(out, vec!["log:true|true|true|true|HEAD|true|function"]);
}

#[test]
fn q33t_set_timeout_runs_after_the_turn() {
    let log = run_with_timers("setTimeout(()=>console.log('a'),0); console.log('b');");
    assert_eq!(log, vec!["log:b", "log:a"]);
}

#[test]
fn q33t_clear_timeout_cancels() {
    let log = run_with_timers(
        "var t = setTimeout(()=>console.log('no'),0); clearTimeout(t); window.setTimeout(()=>console.log('yes'),1);",
    );
    assert_eq!(log, vec!["log:yes"]);
}

#[test]
fn q33t_interval_runs_n_times_then_clears_itself() {
    let log = run_with_timers(
        "var n=0; var id=setInterval(function(){ n++; console.log('i'+n); if(n==3) clearInterval(id); }, 1);",
    );
    assert_eq!(log, vec!["log:i1", "log:i2", "log:i3"]);
}

#[test]
fn q33t_timer_args_string_handler_raf_and_order() {
    let log = run_with_timers(
        "setTimeout(function(a,b){console.log('args '+a+b)},30,'x','y');\
         setTimeout(\"console.log('str')\",1);\
         requestAnimationFrame(function(ts){console.log(typeof ts)});\
         var c=requestAnimationFrame(function(){console.log('cancelled')}); cancelAnimationFrame(c);",
    );
    assert_eq!(log, vec!["log:str", "log:number", "log:args xy"]);
}

#[test]
fn q33t_timer_error_is_logged_and_does_not_stop_others() {
    let log = run_with_timers("setTimeout(()=>{throw new Error('boom')},0); setTimeout(()=>console.log('ok'),1);");
    assert!(log[0].starts_with("error:Uncaught Error: boom"), "{log:?}");
    assert_eq!(log[1], "log:ok");
}

#[test]
fn q33t_no_timers_means_no_deadline() {
    let engine = start_scripts(&parse_html(""), &[src("1+1")], Rc::new(|_, _| {})).unwrap();
    assert!(engine.next_deadline().is_none());
}

#[test]
fn q33t_runaway_timer_callback_is_interrupted_with_its_own_budget() {
    let log = run_with_timers("setTimeout(()=>{ for(;;){} },0); setTimeout(()=>console.log('after'),1);");
    assert!(log.iter().any(|l| l.starts_with("error:")), "{log:?}");
    assert!(log.iter().any(|l| l == "log:after"), "{log:?}");
}

/// R7: an image fetched for the current navigation can arrive before
/// that navigation's `NavResult` (scripts are fetched synchronously
/// before `NavResult` is sent); the document swap must not drop it,
/// while a stale-generation image must still be dropped.
#[test]
fn r7_image_fetched_before_nav_result_survives_document_swap() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    let img = || Some(DecodedImage { width: 1, height: 1, pixels: vec![TextColor::rgb(1, 2, 3)], svg: None, mask: None });
    app.image_cache.insert("stale-from-old-page".into(), Arc::new(img().unwrap()));
    app.fetch_url_inner(Url::parse("http://127.0.0.1:9/").unwrap());
    assert!(app.image_cache.is_empty(), "starting a navigation clears the old page images");
    let generation = app.nav_generation;
    app.handle_image_fetched(generation - 1, "old".into(), img());
    app.handle_image_fetched(generation, "globe.png".into(), img());
    let page = FetchedPage {
        html: b"<html><body><img src=\"globe.png\"></body></html>".to_vec(),
        extra_css: String::new(),
        sheets: Vec::new(),
        scripts: Vec::new(),
        url: Url::parse("http://127.0.0.1:9/").unwrap(),
    };
    app.handle_nav_result(generation, Ok(page));
    assert!(app.image_cache.contains_key("globe.png"), "image that beat NavResult must survive");
    assert!(!app.image_cache.contains_key("old"), "stale-generation image must be dropped");
}

/// QOBS5b: with a far `WaitUntil` (a pending JS timer) the loop must sleep. `pump_net` used to clone
/// a winit `EventLoopProxy` on every call and drop it again; the drop pings the loop, so
/// about_to_wait -> js_sync -> pump_net woke the loop at ~10 kHz (100 % CPU on an idle page).
#[test]
fn qobs5b_idle_pump_net_does_not_wake_the_loop() {
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::ControlFlow;
    use winit::platform::pump_events::EventLoopExtPumpEvents;
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    use winit::window::WindowId;
    struct Probe {
        app: App,
        waits: usize,
    }
    impl ApplicationHandler<AppEvent> for Probe {
        fn resumed(&mut self, _: &ActiveEventLoop) {}
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        fn about_to_wait(&mut self, el: &ActiveEventLoop) {
            self.waits += 1;
            self.app.pump_net();
            el.set_control_flow(ControlFlow::WaitUntil(std::time::Instant::now() + std::time::Duration::from_secs(30)));
        }
    }
    let mut event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    let mut probe = Probe { app, waits: 0 };
    // Let the init wakeups settle, then count the loop turns over 400 ms of wall time
    // (each call runs one turn at most, so keep calling).
    let spin = |event_loop: &mut EventLoop<AppEvent>, probe: &mut Probe, ms: u64| {
        let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while std::time::Instant::now() < end {
            event_loop.pump_app_events(Some(std::time::Duration::from_millis(20)), probe);
        }
    };
    spin(&mut event_loop, &mut probe, 100);
    probe.waits = 0;
    spin(&mut event_loop, &mut probe, 400);
    assert!(probe.waits < 60, "an idle WaitUntil loop woke {} times in 400 ms", probe.waits);
}

#[test]
fn qb8_current_script_is_the_executing_element_then_null() {
    let (_engine, out) = qb8_start(
        "<script src=x.js></script><script id=a>console.log(document.currentScript.id, document.currentScript.tagName);
         document.addEventListener('DOMContentLoaded', function(){ console.log('dcl', document.currentScript); });</script>
         <script id=b>console.log(document.currentScript.id)</script>",
    );
    assert_eq!(*out.borrow(), ["log:a SCRIPT", "log:b", "log:dcl null"]);
}

#[test]
fn qb8_timer_callback_sees_null_current_script() {
    let (engine, out) = qb8_start("<script id=a>setTimeout(function(){ console.log('t:' + document.currentScript); }, 0)</script>");
    std::thread::sleep(std::time::Duration::from_millis(20));
    engine.run_due();
    assert_eq!(*out.borrow(), ["log:t:null"]);
}

#[test]
fn qb8_event_and_timer_errors_name_their_frame() {
    let (engine, out) = qb8_start(
        "<body><script>document.body.addEventListener('click', function(){ throw new Error('boom'); }); setTimeout(function(){ throw new Error('tick'); }, 0);</script></body>",
    );
    engine.dispatch_event(None, "click", &[]);
    std::thread::sleep(std::time::Duration::from_millis(20));
    engine.run_due();
    let out = out.borrow();
    assert!(out.iter().any(|l| l.contains("boom") && l.contains("event:click")), "{out:?}");
    assert!(out.iter().any(|l| l.contains("tick") && l.contains("timer")), "{out:?}");
    assert!(out.iter().all(|l| !l.contains("(timers:1:1)")), "{out:?}");
}

#[test]
fn qa1_border_style_initial_value_is_none() {
    let div_of = |style: &str, width: f32| {
        let html = format!(r#"<html><body><div style="width: {width}px; {style}">x</div></body></html>"#);
        let laid = measure_fixture(&html, 500.0);
        find_by_width(&laid, width).expect("div not found").style
    };
    let s = div_of("border: 4px blue;", 311.0);
    assert_eq!(s.border_width, [0.0; 4]);
    assert_eq!(s.border_style, [BorderStyle::None; 4]);
    let s = div_of("border-width: 4px;", 312.0);
    assert_eq!(s.border_width, [0.0; 4]);
    let s = div_of("border: 4px solid blue;", 313.0);
    assert_eq!(s.border_width, [4.0; 4]);
    let s = div_of("border-top: 4px solid blue;", 314.0);
    assert_eq!(s.border_width, [4.0, 0.0, 0.0, 0.0]);
    // The UA sheet's own borders still name a style.
    let laid = measure_fixture(r#"<html><body><hr style="width: 321px"></body></html>"#, 500.0);
    let hr = find_by_width(&laid, 321.0).expect("hr not found").style;
    assert_eq!(hr.border_width, [1.0; 4]);
}

// ---------- QY9: URL and URLSearchParams ----------

fn qy9_run(code: &str) -> Vec<String> {
    run_in_html("<html><body></body></html>", code)
}

#[test]
fn qy9_url_parse_and_relative() {
    let log = qy9_run(
        "var u=new URL('https://u:p@Example.org:8080/a/b?x=1#frag');\
         console.log(u.href,u.protocol,u.host,u.hostname,u.port,u.pathname,u.search,u.hash,u.origin,u.username,u.password);\
         console.log(new URL('../a?b=1#c','https://x.org/p/q').href, new URL('/z','https://x.org/p/q').href, String(new URL('a','https://x.org/')), JSON.stringify({u:new URL('https://x.org')}));\
         console.log(URL.canParse('nope'), URL.canParse('a','https://x.org'), URL.parse('nope'));",
    );
    assert_eq!(
        log,
        [
            "log:https://u:p@example.org:8080/a/b?x=1#frag https: example.org:8080 example.org 8080 /a/b ?x=1 #frag https://example.org:8080 u p",
            "log:https://x.org/a?b=1#c https://x.org/z https://x.org/a {\"u\":\"https://x.org/\"}",
            "log:false true null",
        ],
        "{log:?}"
    );
}

#[test]
fn qy9_url_invalid_throws_type_error() {
    let log = qy9_run("try{new URL('not a url');console.log('no')}catch(e){console.log(e instanceof TypeError, e.name)}");
    assert_eq!(log, ["log:true TypeError"], "{log:?}");
}

#[test]
fn qy9_search_params_live_link_both_ways() {
    let log = qy9_run(
        "var u=new URL('https://x.org/p?a=1&a=2');\
         u.searchParams.set('z','1 2');\
         console.log(u.search, u.href);\
         u.search='?q=%C3%A9&w=a+b';\
         console.log(u.searchParams.get('q'), u.searchParams.get('w'), u.searchParams.size);\
         u.searchParams.delete('q'); u.searchParams.delete('w');\
         console.log(JSON.stringify(u.search), u.href);\
         u.pathname='/new path'; u.hash='h'; u.port='81';\
         console.log(u.href);",
    );
    assert_eq!(
        log,
        [
            "log:?a=1&a=2&z=1+2 https://x.org/p?a=1&a=2&z=1+2",
            "log:é a b 2",
            "log:\"\" https://x.org/p",
            "log:https://x.org:81/new%20path#h",
        ],
        "{log:?}"
    );
}

#[test]
fn qy9_search_params_encoding_and_iteration() {
    let log = qy9_run(
        "var p=new URLSearchParams('a=1&a=2&b=%20&c');\
         console.log(p.getAll('a').length, JSON.stringify(p.get('b')), p.has('c'), p.get('nope'), p.size);\
         var q=new URLSearchParams({k:'a b', u:'h\\u00e9llo&='});\
         console.log(q.toString());\
         var r=new URLSearchParams([['x','1'],['y','2']]); r.append('x','3'); r.sort();\
         var out=[]; for (var e of r) out.push(e.join(':'));\
         console.log(out.join(','), Array.from(r.keys()).join(''), Array.from(r.values()).join(''));\
         var n=[]; r.forEach(function(v,k){n.push(k+v)}); console.log(n.join(','));\
         console.log(new URLSearchParams('?a=1').toString());",
    );
    assert_eq!(
        log,
        [
            "log:2 \" \" true null 4",
            "log:k=a+b&u=h%C3%A9llo%26%3D",
            "log:x:1,x:3,y:2 xxy 132",
            "log:x1,x3,y2",
            "log:a=1",
        ],
        "{log:?}"
    );
}

// ---------- QY13: history, AbortController, MessageChannel, storage, ... ----------

#[test]
fn qy13_history_push_replace() {
    set_page_url(Url::parse("https://page.test/dir/index.html").ok().as_ref());
    let log = run_with_timers(
        "var n0=history.length; history.pushState({a:1},'','/p?q=1#h');\
         console.log(n0, history.length, JSON.stringify(history.state), location.pathname, location.search, location.hash, document.URL.indexOf('/p?q=1#h')>0);\
         history.replaceState({b:2},'','/r'); console.log(history.length, history.state.b, location.pathname, history.scrollRestoration);\
         window.addEventListener('popstate', function(e){ console.log('pop', e.state, location.pathname); });\
         history.back();",
    );
    assert_eq!(log, ["log:1 2 {\"a\":1} /p ?q=1 #h true", "log:2 2 /r auto", "log:pop null /dir/index.html"], "{log:?}");
    set_page_url(None);
}

#[test]
fn qy13_abort_controller() {
    let log = run_with_timers(
        "var c=new AbortController(), n=0; c.signal.addEventListener('abort',()=>n++); c.signal.onabort=()=>n+=10;\
         console.log(c.signal.aborted); c.abort('why'); c.abort('again');\
         console.log(c.signal.aborted, c.signal.reason, n);\
         try { c.signal.throwIfAborted(); } catch (e) { console.log('threw', e); }\
         console.log(AbortSignal.abort().reason.name);\
         var t=AbortSignal.timeout(5); t.onabort=()=>console.log('timeout', t.reason.name);\
         var a=new AbortController(); var any=AbortSignal.any([a.signal]); a.abort(); console.log(any.aborted);",
    );
    assert_eq!(log, ["log:false", "log:true why 11", "log:threw why", "log:AbortError", "log:true", "log:timeout TimeoutError"], "{log:?}");
}

#[test]
fn qy13_message_channel_and_post_message() {
    let log = run_with_timers(
        "var ch=new MessageChannel(); ch.port1.onmessage=function(e){console.log('port', e.data.x)};\
         ch.port2.postMessage({x:5}); console.log('sync');\
         window.addEventListener('message', function(e){ console.log('win', e.data, e.source===window); });\
         window.postMessage('hi','*'); window.postMessage('no','http://other.example');\
         setTimeout(()=>console.log('t10'),10);",
    );
    assert_eq!(log, ["log:sync", "log:port 5", "log:win hi true", "log:t10"], "{log:?}");
}

#[test]
fn qy13_storage_and_misc() {
    let log = run_with_timers(
        "localStorage.setItem('a',1); localStorage.b='two'; sessionStorage.setItem('c','3');\
         console.log(localStorage.getItem('a'), localStorage.a, localStorage.length, localStorage.key(1), localStorage.getItem('zz'), sessionStorage.length);\
         delete localStorage.a; localStorage.removeItem('b'); console.log(localStorage.length); localStorage.clear();\
         var o=structuredClone({d:new Date(5),m:new Map([[1,2]]),s:new Set([1]),arr:[1,{z:2}]}); console.log(o.d.getTime(), o.m.get(1), o.s.has(1), o.arr[1].z);\
         try { structuredClone(function(){}) } catch(e) { console.log(e.name) }\
         queueMicrotask(()=>console.log('micro')); console.log('sync');\
         requestIdleCallback(function(d){ console.log('idle', d.didTimeout, typeof d.timeRemaining()); });\
         var id=requestIdleCallback(function(){console.log('never')}); cancelIdleCallback(id);",
    );
    assert_eq!(log, ["log:1 1 2 b null 1","log:0", "log:5 2 true 2", "log:DataCloneError", "log:sync", "log:micro", "log:idle false number"], "{log:?}");
}

// ---------- QY14: JS diagnostics ----------

#[test]
fn qy14_rejected_promise_job_and_unhandled_rejection_log_message() {
    // A throw inside a `.then` job rejects the derived promise: one line.
    let log = run_in_html("", "Promise.resolve().then(function () { throw new TypeError('job-boom'); });");
    let hits: Vec<_> = log.iter().filter(|l| l.contains("job-boom")).collect();
    assert_eq!(hits.len(), 1, "{log:?}");
    assert!(hits[0].starts_with("error:Uncaught (in promise) TypeError: job-boom"), "{log:?}");
    // Async function throwing, and a plain reject of a non-Error value.
    let log = run_in_html("", "(async function () { throw new RangeError('async-boom'); })(); Promise.reject('plain-reason');");
    assert!(log.iter().any(|l| l.starts_with("error:Uncaught (in promise) RangeError: async-boom")), "{log:?}");
    assert!(log.iter().any(|l| l == "error:Uncaught (in promise) plain-reason"), "{log:?}");
}

#[test]
fn qy14_handled_rejections_stay_silent_and_events_fire() {
    let log = run_in_html("", "Promise.reject(new Error('quiet')).catch(function () {}); (async function () { try { await Promise.reject(1); } catch (e) {} })();");
    assert!(!log.iter().any(|l| l.starts_with("error:")), "{log:?}");
    let log = run_in_html(
        "",
        "var late = Promise.reject(new Error('later'));\
         addEventListener('unhandledrejection', function (e) { console.log('unhandled', e.reason.message, e instanceof PromiseRejectionEvent); });\
         addEventListener('rejectionhandled', function (e) { console.log('handled', e.reason.message); });\
         setTimeout(function () { late.catch(function () {}); }, 0);\
         Promise.reject(new Error('stopped'));",
    );
    assert!(log.iter().any(|l| l == "log:unhandled stopped true"), "{log:?}");
    assert_eq!(log.iter().filter(|l| l.contains("Uncaught (in promise)")).count(), 2, "{log:?}");
}

#[test]
fn qy14_prevent_default_suppresses_unhandled_rejection_log() {
    let log = run_in_html("", "addEventListener('unhandledrejection', function (e) { e.preventDefault(); }); Promise.reject(new Error('muted'));");
    assert!(!log.iter().any(|l| l.contains("muted")), "{log:?}");
}

#[test]
fn qy14_js_report_run_timers_finds_timer_and_channel_errors() {
    let html = "<html><body><script>\
        setTimeout(function () { throw new Error('late-timer'); }, 1000);\
        var ch = new MessageChannel(); ch.port1.onmessage = function () { throw new Error('chan-boom'); }; ch.port2.postMessage(1);\
        requestIdleCallback(function () { throw new Error('idle-boom'); });\
        setTimeout(function () { Promise.reject(new Error('timer-rejection')); }, 50);\
        console.error('page-console-error');\
        </script></body></html>";
    // Default: unchanged format, nothing from the timer phase.
    let plain = js::js_report_for_html(html, None, "none.html");
    assert!(!plain.contains("late-timer") && !plain.contains("timers ("), "{plain}");
    assert!(plain.contains("errors total: 0\n") && plain.contains("console.error calls: 1\n"), "{plain}");
    let report = js::js_report_for_html_timers(html, None, "none.html", Some(2000));
    for needle in ["late-timer", "chan-boom", "idle-boom", "timer-rejection"] {
        assert!(report.contains(needle), "{needle}: {report}");
    }
    assert!(report.contains("js error (timers): "), "{report}");
    assert!(report.contains("errors in timer phase: 4\n"), "{report}");
    assert!(report.contains("errors total: 0\n"), "{report}");
    let short = js::js_report_for_html_timers(html, None, "none.html", Some(500));
    assert!(!short.contains("late-timer") && short.contains("chan-boom"), "{short}");
}

#[test]
fn qy31_js_report_keeps_pumping_after_a_tripped_callback() {
    let html = "<html><body><script>\
        setTimeout(function () { for (;;) {} }, 10);\
        setTimeout(function () { console.error('after-trip-1000'); }, 1000);\
        setTimeout(function () { throw new Error('after-trip-3000'); }, 3000);\
        </script></body></html>";
    let report = js::js_report_for_html_timers(html, None, "none.html", Some(5000));
    assert!(report.contains("CPU-time callback budget"), "{report}");
    assert!(report.contains("after-trip-1000") && report.contains("after-trip-3000"), "{report}");
    assert!(report.contains("--- timers (simulated 3000 ms, 3 callbacks) ---\n"), "{report}");
}

#[test]
fn qy14_js_report_run_timers_is_bounded() {
    let html = "<html><body><script>setInterval(function () { throw new Error('tick'); }, 1);</script></body></html>";
    let report = js::js_report_for_html_timers(html, None, "none.html", Some(100000));
    assert!(report.contains("errors in timer phase: 5000\n"), "{}", &report[report.len().saturating_sub(300)..]);
}

#[test]
fn qy14_console_methods_exist() {
    let log = run_in_html(
        "",
        "console.assert(true, 'no'); console.assert(false, 'yes'); console.group('g'); console.table([1]); console.trace('t'); console.groupEnd(); console.dir({a:1}); console.count(); console.time('x'); console.timeEnd('x'); console.error('e'); console.info('i'); console.debug('d'); console.warn('w');",
    );
    assert!(log.iter().any(|l| l == "console.error:Assertion failed: yes"), "{log:?}");
    assert!(log.iter().any(|l| l == "console.error:e"), "{log:?}");
    assert!(log.iter().any(|l| l.starts_with("log:Trace: t")), "{log:?}");
    assert!(log.iter().any(|l| l == "log:default: 1") && log.iter().any(|l| l == "warn:w") && log.iter().any(|l| l == "info:i"), "{log:?}");
    assert!(!log.iter().any(|l| l.contains(":no")), "{log:?}");
}

// ---------- QY16: minimal en-US Intl ----------

#[test]
fn qy16_number_format() {
    let log = run_with_timers(
        "var nf=new Intl.NumberFormat('en-US');\
         console.log(nf.format(1234.5), nf.format(1234567.891), nf.format(-0.5), new Intl.NumberFormat('en',{maximumFractionDigits:2}).format(1.005), nf.format(NaN));\
         console.log(new Intl.NumberFormat('en',{minimumFractionDigits:2}).format(5), new Intl.NumberFormat('en',{maximumFractionDigits:0}).format(2.5), new Intl.NumberFormat('en',{useGrouping:false}).format(12345));\
         console.log(new Intl.NumberFormat('en',{style:'percent'}).format(0.256), new Intl.NumberFormat('en',{style:'percent',maximumFractionDigits:1}).format(0.2567));\
         var c=function(cur,v,o){return new Intl.NumberFormat('en-US',Object.assign({style:'currency',currency:cur},o)).format(v)};\
         console.log(c('USD',1234.5), c('EUR',-3), c('GBP',0.5), c('JPY',1234.5), c('USD',-5,{currencySign:'accounting'}));\
         var k=function(v,o){return new Intl.NumberFormat('en',Object.assign({notation:'compact'},o)).format(v)};\
         console.log(k(999), k(1234), k(12345), k(1500000), k(2.5e9), k(999999), k(1234,{compactDisplay:'long'}));\
         console.log(new Intl.NumberFormat('en',{maximumSignificantDigits:3}).format(123456), new Intl.NumberFormat('en',{minimumIntegerDigits:3}).format(5));\
         console.log(JSON.stringify(nf.formatToParts(-1234.5).map(function(p){return p.type+'='+p.value})));\
         var r=new Intl.NumberFormat('de-DE').resolvedOptions(); console.log(r.locale, r.minimumFractionDigits, r.maximumFractionDigits, r.style);\
         var f=nf.format; console.log(f(7), typeof Intl, Object.prototype.toString.call(Intl));",
    );
    assert_eq!(
        log,
        [
            "log:1,234.5 1,234,567.891 -0.5 1.01 NaN",
            "log:5.00 3 12345",
            "log:26% 25.7%",
            "log:$1,234.50 -€3.00 £0.50 ¥1,235 ($5.00)",
            "log:999 1.2K 12K 1.5M 2.5B 1M 1.2 thousand",
            "log:123,000 005",
            "log:[\"minusSign=-\",\"integer=1\",\"group=,\",\"integer=234\",\"decimal=.\",\"fraction=5\"]",
            "log:en-US 0 3 decimal",
            "log:7 object [object Intl]",
        ],
        "{log:?}"
    );
}

#[test]
fn qy16_date_time_format() {
    let log = run_with_timers(
        "var t=Date.UTC(2024,0,2,15,4,5,67);\
         var f=function(o,l){return new Intl.DateTimeFormat(l||'en-US',Object.assign({timeZone:'UTC'},o)).format(t)};\
         console.log(f({}), f({dateStyle:'short'}), f({dateStyle:'medium'}), f({dateStyle:'long'}), f({dateStyle:'full'}));\
         console.log(f({timeStyle:'short'}), f({timeStyle:'medium'}), f({timeStyle:'long'}), f({timeStyle:'full'}));\
         console.log(f({dateStyle:'medium',timeStyle:'short'}), '|', f({dateStyle:'long',timeStyle:'short'}));\
         console.log(f({year:'numeric',month:'long',day:'numeric'}), f({month:'short',day:'numeric'}), f({year:'numeric',month:'2-digit',day:'2-digit'}), f({weekday:'long'}), f({month:'long',year:'numeric'}));\
         console.log(f({hour:'numeric',minute:'2-digit',hour12:false}), f({hour:'numeric',hour12:true}), f({hour:'2-digit',minute:'2-digit',second:'2-digit',hour12:false}));\
         console.log(JSON.stringify(new Intl.DateTimeFormat('en-US',{timeZone:'UTC',month:'short',day:'numeric'}).formatToParts(t)));\
         var r=new Intl.DateTimeFormat('fr-FR').resolvedOptions(); console.log(r.locale, r.timeZone, r.calendar);\
         var d=new Date(t); console.log(d.toLocaleDateString('en-US',{timeZone:'UTC'}), '|', d.toLocaleTimeString('en-US',{timeZone:'UTC'}), '|', d.toLocaleString('en-US',{timeZone:'UTC'}), '|', d.toLocaleDateString(undefined,{month:'long',day:'numeric'}));\
         console.log(new Date(NaN).toLocaleString(), (function(){try{new Intl.DateTimeFormat().format(NaN)}catch(e){return e.name}})());",
    );
    assert_eq!(
        log,
        [
            "log:1/2/2024 1/2/24 Jan 2, 2024 January 2, 2024 Tuesday, January 2, 2024",
            "log:3:04 PM 3:04:05 PM 3:04:05 PM UTC 3:04:05 PM Coordinated Universal Time",
            "log:Jan 2, 2024, 3:04 PM | January 2, 2024 at 3:04 PM",
            "log:January 2, 2024 Jan 2 01/02/2024 Tuesday January 2024",
            "log:15:04 3 PM 15:04:05",
            "log:[{\"type\":\"month\",\"value\":\"Jan\"},{\"type\":\"literal\",\"value\":\" \"},{\"type\":\"day\",\"value\":\"2\"}]",
            "log:en-US UTC gregory",
            "log:1/2/2024 | 3:04:05 PM | 1/2/2024, 3:04:05 PM | January 2",
            "log:Invalid Date RangeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy16_plural_relative_list_collator() {
    let log = run_with_timers(
        "var pr=new Intl.PluralRules('en'), po=new Intl.PluralRules('en-US',{type:'ordinal'});\
         console.log(pr.select(0), pr.select(1), pr.select(2), pr.select(1.5), [1,2,3,4,11,12,13,21,22,23,101].map(function(n){return po.select(n)}).join());\
         var rt=new Intl.RelativeTimeFormat('en'), ra=new Intl.RelativeTimeFormat('en',{numeric:'auto'});\
         console.log(rt.format(3,'day'), rt.format(-3,'days'), rt.format(1,'hour'), rt.format(-1,'month'), rt.format(2,'year'));\
         console.log(ra.format(-1,'day'), ra.format(1,'day'), ra.format(0,'day'), ra.format(1,'year'), ra.format(-2,'day'), ra.format(0,'second'));\
         var lf=function(a,o){return new Intl.ListFormat('en',o).format(a)};\
         console.log(lf(['a','b','c']), '|', lf(['a','b']), '|', lf(['a']), '|', lf(['a','b','c'],{type:'disjunction'}), '|', lf(['a','b','c'],{style:'short'}), '|', lf(['a','b','c'],{type:'unit',style:'narrow'}));\
         var co=new Intl.Collator();\
         console.log(['b','a','C','B','c','A'].sort(co.compare).join(''), co.compare('a','b'), co.compare('b','a'), co.compare('a','a'), 'a'.localeCompare('B'), 'résumé'.localeCompare('resume'));\
         console.log(['a10','a2','a1'].sort(new Intl.Collator(undefined,{numeric:true}).compare).join(), ['a10','a2','a1'].sort(new Intl.Collator().compare).join(), new Intl.Collator('en',{sensitivity:'base'}).compare('a','Á'));",
    );
    assert_eq!(
        log,
        [
            "log:other one other other one,two,few,other,other,other,other,one,two,few,one",
            "log:in 3 days 3 days ago in 1 hour 1 month ago in 2 years",
            "log:yesterday tomorrow today next year 2 days ago now",
            "log:a, b, and c | a and b | a | a, b, or c | a, b, & c | a b c",
            "log:aAbBcC -1 1 0 -1 1",
            "log:a1,a2,a10 a1,a10,a2 0",
        ],
        "{log:?}"
    );
}

#[test]
fn qy16_locale_fallback_and_misc_apis() {
    let log = run_with_timers(
        "var mk=[Intl.NumberFormat,Intl.DateTimeFormat,Intl.PluralRules,Intl.RelativeTimeFormat,Intl.ListFormat,Intl.Collator,Intl.Segmenter];\
         console.log(mk.map(function(F){return new F('de-DE-u-nu-latn').resolvedOptions().locale}).join());\
         console.log(mk.map(function(F){return new F(['xx','ja'],{bogusOption:1}).resolvedOptions().locale}).join());\
         try { new Intl.NumberFormat('en_US') } catch (e) { console.log(e.name) }\
         try { new Intl.NumberFormat('') } catch (e) { console.log(e.name) }\
         console.log(Intl.getCanonicalLocales('EN-us').join(), JSON.stringify(Intl.supportedValuesOf('currency')), Intl.NumberFormat.supportedLocalesOf(['en-GB','de']).join());\
         var l=new Intl.Locale('en-Latn-US'); console.log(l.language, l.script, l.region, l.baseName, String(l), new Intl.Locale('fr').maximize().toString(), new Intl.Locale('en-Latn-US').minimize().toString(), new Intl.Locale('en-US-u-hc-h23').hourCycle);\
         console.log(new Intl.DisplayNames('en',{type:'region'}).of('US'), Array.from(new Intl.Segmenter('en').segment('a\\u0301b')).length, Array.from(new Intl.Segmenter('en',{granularity:'word'}).segment('Hi there!')).filter(function(s){return s.isWordLike}).map(function(s){return s.segment}).join());\
         console.log((1234.5).toLocaleString(), (1234.5).toLocaleString('en-US',{style:'currency',currency:'USD'}), (0.5).toLocaleString(undefined,{style:'percent'}), [1234,5].toLocaleString(), (12345678901234567890n).toLocaleString());",
    );
    assert_eq!(
        log,
        [
            "log:en-US,en-US,en-US,en-US,en-US,en-US,en-US",
            "log:en-US,en-US,en-US,en-US,en-US,en-US,en-US",
            "log:RangeError",
            "log:RangeError",
            "log:en-US [] en-GB",
            "log:en Latn US en-Latn-US en-Latn-US fr-Latn-FR en h23",
            "log:US 2 Hi,there",
            "log:1,234.5 $1,234.50 50% 1,234,5 12,345,678,901,234,567,890",
        ],
        "{log:?}"
    );
}

// ---- QY15: ES modules

/// Runs `html` as a page whose module fetches are served from `files`
/// (url -> source); returns the log lines.
fn run_module_page(html: &str, files: &[(&str, &str)]) -> Vec<String> {
    let _g = JS_FLAG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_js_enabled(true);
    let dom = parse_html(html);
    let mut res = ExternalResources::default();
    find_external_resources(&dom, &mut res);
    let base = Url::parse("https://example.test/dir/page.html").unwrap();
    let files: Vec<(String, String)> = files.iter().map(|(u, s)| (u.to_string(), s.to_string())).collect();
    let fetch = move |u: &Url| files.iter().find(|(k, _)| k == u.as_str()).map(|(_, s)| s.clone());
    let scripts = fetch_scripts_with(&base, res.scripts, &fetch);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    run_scripts_in(&dom, &scripts, Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))));
    let v = out.borrow().clone();
    v
}

#[test]
fn qy15_inline_module_imports_prefetched_module_default_and_named() {
    let log = run_module_page(
        "<script type=module>import d, { n } from './b.js'; console.log(d + n);</script>",
        &[("https://example.test/dir/b.js", "export default 'D'; export const n = 'N';")],
    );
    assert_eq!(log, ["log:DN"], "{log:?}");
}

#[test]
fn qy15_external_module_and_transitive_imports_and_import_meta_url() {
    let log = run_module_page(
        "<script type=module src=\"/m/a.js\"></script>",
        &[
            ("https://example.test/m/a.js", "import { b } from './b.js'; console.log(b, import.meta.url);"),
            ("https://example.test/m/b.js", "export { c as b } from '../c.js';"),
            ("https://example.test/c.js", "export const c = 'C';"),
        ],
    );
    assert_eq!(log, ["log:C https://example.test/m/a.js"], "{log:?}");
}

#[test]
fn qy15_inline_import_meta_url_is_document_url() {
    let log = run_module_page("<script type=module>console.log(import.meta.url);</script>", &[]);
    assert_eq!(log, ["log:https://example.test/dir/page.html"], "{log:?}");
}

#[test]
fn qy15_nomodule_scripts_are_skipped_and_classic_runs_first() {
    let log = run_module_page(
        "<script type=module>console.log('module');</script>\
         <script nomodule>console.log('nomodule');</script>\
         <script>console.log('classic');</script>\
         <script>document.addEventListener('DOMContentLoaded', function(){ console.log('dcl'); });</script>",
        &[],
    );
    assert_eq!(log, ["log:classic", "log:module", "log:dcl"], "{log:?}");
}

#[test]
fn qy15_module_scripts_run_in_document_order() {
    let log = run_module_page(
        "<script type=module src=a.js></script><script type=module>console.log('inline');</script><script type=module src=b.js></script>",
        &[("https://example.test/dir/a.js", "console.log('a');"), ("https://example.test/dir/b.js", "console.log('b');")],
    );
    assert_eq!(log, ["log:a", "log:inline", "log:b"], "{log:?}");
}

#[test]
fn qy15_importmap_resolves_bare_specifiers_and_prefixes() {
    let log = run_module_page(
        "<script type=importmap>{\"imports\":{\"lib\":\"/vendor/lib.js\",\"u/\":\"/utils/\"}}</script>\
         <script type=module>import { l } from 'lib'; import { x } from 'u/x.js'; console.log(l + x);</script>",
        &[("https://example.test/vendor/lib.js", "export const l = 'L';"), ("https://example.test/utils/x.js", "export const x = 'X';")],
    );
    assert_eq!(log, ["log:LX"], "{log:?}");
}

#[test]
fn qy15_bare_specifier_without_importmap_is_an_error() {
    let log = run_module_page("<script type=module>import 'nope';</script>", &[]);
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(log[0].starts_with("error:") && log[0].contains("nope"), "{log:?}");
}

#[test]
fn qy15_failed_fetch_logs_once_and_other_scripts_still_run() {
    let log = run_module_page(
        "<script type=module src=gone.js></script><script type=module>import './gone2.js';</script><script>console.log('classic');</script><script type=module>console.log('ok');</script>",
        &[],
    );
    assert_eq!(log.iter().filter(|l| l.starts_with("error:") && l.contains("gone.js")).count(), 1, "{log:?}");
    assert_eq!(log.iter().filter(|l| l.starts_with("error:") && l.contains("gone2.js")).count(), 1, "{log:?}");
    assert!(log.contains(&"log:classic".to_string()) && log.contains(&"log:ok".to_string()), "{log:?}");
}

#[test]
fn qy15_module_is_strict_scoped_and_this_is_undefined() {
    let log = run_module_page(
        "<script type=module>var leaked = 1; console.log(this === undefined, typeof globalThis.leaked, (function(){ return this; })() === undefined);</script>\
         <script type=module>try { undeclared = 1; } catch (e) { console.log(e.name); }</script>",
        &[],
    );
    assert_eq!(log, ["log:true undefined true", "log:ReferenceError"], "{log:?}");
}

#[test]
fn qy15_dynamic_import_of_prefetched_and_data_modules() {
    let log = run_module_page(
        "<script>import('./d.js').then(function(m){ console.log('classic ' + m.v); });</script>\
         <script type=module>const m = await import('data:text/javascript,export default 7'); console.log('data ' + m.default);</script>",
        &[("https://example.test/dir/d.js", "export const v = 'V';")],
    );
    assert!(log.contains(&"log:data 7".to_string()), "{log:?}");
    assert!(log.iter().any(|l| l == "log:classic V"), "{log:?}");
}

#[test]
fn qy15_dynamic_import_of_unfetched_url_rejects_with_clear_error() {
    let log = run_module_page(
        "<script type=module>try { await import('https://other.test/' + 'x.js'); } catch (e) { console.log('rejected ' + /not pre-fetched/.test(String(e.message))); }</script>",
        &[],
    );
    assert_eq!(log, ["log:rejected true"], "{log:?}");
}

#[test]
fn qy15_module_syntax_error_and_throw_are_logged_with_url() {
    let log = run_module_page(
        "<script type=module src=bad.js></script><script type=module src=thrower.js></script>",
        &[("https://example.test/dir/bad.js", "export const = ;"), ("https://example.test/dir/thrower.js", "throw new Error('boom');")],
    );
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].starts_with("error:https://example.test/dir/bad.js: "), "{log:?}");
    assert!(log[1].starts_with("error:https://example.test/dir/thrower.js: ") && log[1].contains("boom"), "{log:?}");
}

/// QV8-3c: a shared dependency runs once, an import cycle links, a dependency with top-level
/// await finishes before its importer runs, and a failed instantiation rejects `import()`.
#[test]
fn qv8_3c_shared_cyclic_and_tla_module_graphs() {
    let log = run_module_page(
        "<script type=module src=a.js></script><script type=module src=b.js></script>\
         <script type=module>try { await import('./nope.js'); } catch (e) { console.log('rejected'); }</script>",
        &[
            ("https://example.test/dir/a.js", "import './shared.js'; import { c } from './cyc.js'; import { t } from './tla.js'; console.log('a ' + c() + t);"),
            ("https://example.test/dir/b.js", "import './shared.js'; console.log('b');"),
            ("https://example.test/dir/shared.js", "console.log('shared');"),
            ("https://example.test/dir/cyc.js", "import { d } from './cyc2.js'; export function c() { return 'c' + d(); }"),
            ("https://example.test/dir/cyc2.js", "import { c } from './cyc.js'; export function d() { return typeof c; }"),
            ("https://example.test/dir/tla.js", "await Promise.resolve(); export const t = 'T';"),
        ],
    );
    assert_eq!(log[0], "log:shared", "{log:?}");
    assert!(log.contains(&"log:a cfunctionT".to_string()), "{log:?}");
    assert_eq!(log.iter().filter(|l| *l == "log:shared").count(), 1, "{log:?}");
    assert!(log.contains(&"log:b".to_string()) && log.contains(&"log:rejected".to_string()), "{log:?}");
}

#[test]
fn qy15_module_runaway_loop_hits_the_cpu_budget() {
    let log = run_module_page("<script type=module>for(;;){}</script><script type=module>console.log('after');</script>", &[]);
    assert!(log[0].starts_with("error:js: ") && log[0].contains("stopped"), "{log:?}");
    assert_eq!(log[1], "log:after", "{log:?}");
}

#[test]
fn qy15_import_scanner_finds_specifiers_and_ignores_strings_and_comments() {
    let src = "import a from 'a'; import {b as c, d} from \"b\"; import * as ns from './ns.js';\n\
               export * from 'e'; export { x } from 'f'; import 'side';\n\
               // import 'commented'\n/* import 'block' */ var s = \"import 'instr'\"; var t = `import 'tpl'`;\n\
               var r = /import 're'/; import('dyn'); const u = import.meta.url; export const k = 1;";
    assert_eq!(crate::js_modules::scan_import_specifiers(src), ["a", "b", "./ns.js", "e", "f", "side", "dyn"]);
}

#[test]
fn qy15_graph_prefetch_terminates_on_import_cycles() {
    let log = run_module_page(
        "<script type=module>import './a.js'; console.log('done');</script>",
        &[
            ("https://example.test/dir/a.js", "import './b.js'; export const a = 1;"),
            ("https://example.test/dir/b.js", "import './a.js'; export const b = 2;"),
        ],
    );
    assert_eq!(log, ["log:done"], "{log:?}");
}

#[test]
fn qy15_literal_dynamic_import_is_prefetched_only_from_entries() {
    // The entry's own `import('./lazy1.js')` is pre-fetched; the one inside a
    // statically imported module (a lazy chunk of a big app) is not.
    let log = run_module_page(
        "<script type=module>import './a.js'; const m = await import('./lazy1.js'); console.log(m.v); try { await import('./lazy' + '2.js'); } catch (e) { console.log('lazy2 rejected'); }</script>",
        &[
            ("https://example.test/dir/a.js", "export const a = 1; export const f = () => import('./lazy2.js');"),
            ("https://example.test/dir/lazy1.js", "export const v = 'L1';"),
            ("https://example.test/dir/lazy2.js", "export const v = 'L2';"),
        ],
    );
    assert_eq!(log, ["log:L1", "log:lazy2 rejected"], "{log:?}");
}

// ---------- QY15b: TextEncoder / TextDecoder / atob / btoa ----------

#[test]
fn qy15b_text_encoder_utf8_bytes() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"var e = new TextEncoder();
         function b(s) { return Array.from(e.encode(s)).join(','); }
         console.log(e.encoding, b('abc'), e.encode() instanceof Uint8Array, e.encode().length);
         console.log(b('é€'));
         console.log(b('😀'));
         console.log(b('a\uD800b'), b('\uDC00'));"#,
    );
    assert_eq!(
        log,
        [
            "log:utf-8 97,98,99 true 0",
            "log:195,169,226,130,172",
            "log:240,159,152,128",
            "log:97,239,191,189,98 239,191,189",
        ],
        "{log:?}"
    );
}

#[test]
fn qy15b_text_encoder_encode_into_read_and_written() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"var e = new TextEncoder();
         var b4 = new Uint8Array(4), r = e.encodeInto('aéb', b4);
         console.log(r.read, r.written, Array.from(b4).join(','));
         var b3 = new Uint8Array(3);
         r = e.encodeInto('aéb', b3);
         console.log(r.read, r.written, Array.from(b3).join(','));
         r = e.encodeInto('€', new Uint8Array(2));
         console.log(r.read, r.written);
         r = e.encodeInto('😀', new Uint8Array(3));
         console.log(r.read, r.written);
         r = e.encodeInto('x😀', new Uint8Array(5));
         console.log(r.read, r.written);
         try { e.encodeInto('a', 'nope'); console.log('no'); } catch (err) { console.log(err.name); }"#,
    );
    assert_eq!(
        log,
        [
            "log:3 4 97,195,169,98",
            "log:2 3 97,195,169",
            "log:0 0",
            "log:0 0",
            "log:3 5",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy15b_text_decoder_utf8_bom_invalid_and_fatal() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"function cs(s) { return Array.prototype.map.call(s, function (c) { return c.charCodeAt(0); }).join(','); }
         var d = new TextDecoder();
         console.log(d.encoding, d.fatal, d.ignoreBOM);
         console.log(d.decode(new Uint8Array([0xEF, 0xBB, 0xBF, 0x68, 0x69])));
         console.log(new TextDecoder('utf-8', { ignoreBOM: true }).decode(new Uint8Array([0xEF, 0xBB, 0xBF, 0x68])).length);
         console.log(cs(d.decode(new Uint8Array([0x61, 0xFF, 0x62]))));
         console.log(cs(d.decode(new Uint8Array([0xE2, 0x82, 0x61]))));
         console.log(cs(d.decode(new Uint8Array([0xED, 0xA0, 0x80]))));
         console.log(cs(d.decode(new Uint8Array([0xC0, 0x80]))));
         console.log(cs(d.decode(new Uint8Array([0xF0, 0x9F, 0x98, 0x80]))));
         console.log(cs(d.decode(new Uint8Array([0xE2, 0x82]))));
         var f = new TextDecoder('utf-8', { fatal: true });
         console.log(f.fatal, f.decode(new Uint8Array([0xF0, 0x9F, 0x98, 0x80])).length);
         try { f.decode(new Uint8Array([0x61, 0xFF])); console.log('no'); } catch (err) { console.log(err.name); }
         var buf = new Uint8Array([0x78, 0x79, 0x7a, 0x7a]).buffer;
         console.log(d.decode(buf), d.decode(new DataView(buf, 1, 2)), d.decode().length);"#,
    );
    assert_eq!(
        log,
        [
            "log:utf-8 false false",
            "log:hi",
            "log:2",
            "log:97,65533,98",
            "log:65533,97",
            "log:65533,65533,65533",
            "log:65533,65533",
            "log:55357,56832",
            "log:65533",
            "log:true 2",
            "log:TypeError",
            "log:xyzz yz 0",
        ],
        "{log:?}"
    );
}

#[test]
fn qy15b_text_decoder_streaming_splits_characters() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"function cs(s) { return Array.prototype.map.call(s, function (c) { return c.charCodeAt(0); }).join(','); }
         var s = new TextDecoder();
         var a1 = s.decode(new Uint8Array([0x61, 0xE2, 0x82]), { stream: true });
         var a2 = s.decode(new Uint8Array([0xAC, 0x62]));
         console.log(cs(a1), cs(a2));
         var t = new TextDecoder(), out = '';
         [0xF0, 0x9F, 0x98, 0x80].forEach(function (x) { out += t.decode(new Uint8Array([x]), { stream: true }); });
         out += t.decode();
         console.log(cs(out));
         var u = new TextDecoder();
         var p = u.decode(new Uint8Array([0xEF]), { stream: true });
         console.log(p.length, u.decode(new Uint8Array([0xBB, 0xBF, 0x78])));
         var bad = new TextDecoder('utf-8', { fatal: true });
         try { bad.decode(new Uint8Array([0xE2]), { stream: true }); bad.decode(new Uint8Array([0x61])); console.log('no'); } catch (err) { console.log(err.name); }
         console.log(bad.decode(new Uint8Array([0x62])));"#,
    );
    assert_eq!(
        log,
        [
            "log:97 8364,98",
            "log:55357,56832",
            "log:0 x",
            "log:TypeError",
            "log:b",
        ],
        "{log:?}"
    );
}

#[test]
fn qy15b_text_decoder_utf16le_windows1252_and_labels() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"function cs(s) { return Array.prototype.map.call(s, function (c) { return c.charCodeAt(0); }).join(','); }
         var u = new TextDecoder('utf-16le');
         console.log(u.encoding, cs(u.decode(new Uint8Array([0x61, 0x00, 0x3d, 0xd8, 0x00, 0xde]))));
         console.log(cs(u.decode(new Uint8Array([0xff, 0xfe, 0x62, 0x00]))));
         console.log(cs(u.decode(new Uint8Array([0x61]))));
         console.log(new TextDecoder('utf8').encoding, new TextDecoder('unicode-1-1-utf-8').encoding, new TextDecoder(' LATIN1 ').encoding, new TextDecoder('iso-8859-1').encoding, new TextDecoder('windows-1252').encoding);
         console.log(cs(new TextDecoder('latin1').decode(new Uint8Array([0x41, 0xe9, 0x80]))));
         try { new TextDecoder('klingon'); console.log('no'); } catch (err) { console.log(err.name); }
         try { TextDecoder('utf-8'); console.log('no'); } catch (err) { console.log(err.name); }"#,
    );
    assert_eq!(
        log,
        [
            "log:utf-16le 97,55357,56832",
            "log:98",
            "log:65533",
            "log:utf-8 utf-8 windows-1252 windows-1252 windows-1252",
            "log:65,233,8364",
            "log:RangeError",
            "log:TypeError",
        ],
        "{log:?}"
    );
}

#[test]
fn qy15b_atob_btoa_and_invalid_character() {
    let log = run_in_html(
        "<html><body></body></html>",
        r#"console.log(btoa(''), btoa('f'), btoa('fo'), btoa('foo'), btoa('foob'), btoa('fooba'), btoa('foobar'));
         console.log(btoa('ÿ'));
         console.log(atob('Zm9vYmFy'), atob('Zm9vYg'), atob(' Zm9v YmE= '), atob('Zg'));
         try { btoa('Ā'); console.log('no'); } catch (err) { console.log(err.name); }
         try { atob('a'); console.log('no'); } catch (err) { console.log(err.name); }
         try { atob('@@@@'); console.log('no'); } catch (err) { console.log(err.name); }"#,
    );
    assert_eq!(
        log,
        [
            "log: Zg== Zm8= Zm9v Zm9vYg== Zm9vYmE= Zm9vYmFy",
            "log:/w==",
            "log:foobar foob fooba f",
            "log:InvalidCharacterError",
            "log:InvalidCharacterError",
            "log:InvalidCharacterError",
        ],
        "{log:?}"
    );
}

// ---- QY46: each promise job gets its own budget; the drain is capped at 2 s and defers the rest.

/// Runs `code` with the fake CPU clock `(1 s, step)`, pumping timers up to `rounds` times.
pub(super) fn qy46_run(code: &str, step: u64, rounds: usize) -> Vec<String> {
    use crate::js::FAKE_CLOCK;
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    FAKE_CLOCK.with(|f| f.set(Some((1_000_000_000, step))));
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html(""),
        &[src(code)],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    for _ in 0..rounds {
        let Some(due) = engine.next_due_ms() else { break };
        clock.set(clock.get().max(due));
        engine.run_due();
    }
    FAKE_CLOCK.with(|f| f.set(None));
    out.take()
}

#[test]
fn qy46_three_300ms_jobs_all_complete_in_one_drain() {
    // The reporting timer is queued AFTER the jobs' timer, so if it ran before
    // all three jobs finished, n would be < 3.
    let log = qy46_run(
        "var n=0;setTimeout(()=>{for(var i=0;i<3;i++)Promise.resolve().then(()=>{__test_burn(300);n++});},0);\
         setTimeout(()=>console.log('n='+n),0);",
        0,
        10,
    );
    assert!(!log.iter().any(|l| l.starts_with("error:")), "{log:?}");
    assert!(log.iter().any(|l| l == "log:n=3"), "{log:?}");
}

#[test]
fn qy46_runaway_job_is_still_killed() {
    // QV8-3b: the fake clock advances per READ; V8's watchdog reads it once per 2 ms tick, so
    // the step is 100 ms (5 reads).
    let step = 100_000_000;
    let log = qy46_run("setTimeout(()=>{Promise.resolve().then(()=>{for(;;){}});},0);", step, 5);
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert_eq!(errs.len(), 1, "{log:?}");
    assert!(errs[0].contains("promise jobs stopped") && errs[0].contains("1 job killed"), "{log:?}");
}

#[test]
fn qy46_ten_300ms_jobs_stop_at_the_cap_and_resume_without_kills() {
    // A marker job queued behind the ten records how many ran before the
    // first drain ended (a timer can only run between drains).
    let log = qy46_run(
        "var n=0;setTimeout(()=>{for(var i=0;i<10;i++)Promise.resolve().then(()=>{__test_burn(300);n++});},0);\
         setTimeout(()=>console.log('n='+n),0);",
        0,
        20,
    );
    // 7 jobs start before 2 s of CPU is spent (the 7th starts at 1.8 s); the
    // timer that logs runs between the capped drain and its resumption.
    // QV8-3b (R39 C2): V8 cannot resume a cut-short drain; it drops the rest of the queue and
    // says so in one line. Jobs 8-10 never finish, and job 8 may or may not have run its body
    // before the termination landed, so n is 7 or 8, never 10.
    {
        let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
        assert_eq!(errs.len(), 1, "{log:?}");
        assert!(errs[0].contains("promise jobs stopped") && errs[0].contains("remaining jobs dropped"), "{log:?}");
        assert!(log.iter().any(|l| l == "log:n=7" || l == "log:n=8"), "{log:?}");
    }
}

// ---------- QY47a: JS stack limit ----------

fn qy47a_run(code: &str) -> String {
    let doc = parse_html("");
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let _e = start_scripts(&doc, &[src(code)], Rc::new(move |l, t| o.borrow_mut().push(format!("{l}:{t}"))));
    out.borrow().join("|")
}

#[test]
fn qy47a_deep_recursion_works_and_runaway_is_catchable() {
    let h = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(|| {
            let deep = qy47a_run("function f(n){return n?1+f(n-1):0} console.log(String(f(1500)))");
            let run = qy47a_run("function g(){g()} var r; try{g()}catch(e){r=e.name} console.log(r)");
            (deep, run)
        })
        .unwrap();
    let (deep, run) = h.join().unwrap();
    assert!(deep.contains("1500"), "{deep}");
    assert!(run.contains("RangeError"), "{run}");
}

#[test]
fn qy47a_runaway_recursion_catchable_on_2mb_thread() {
    let h = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(|| qy47a_run("function g(){g()} var r; try{g()}catch(e){r=e.name} console.log(r)"))
        .unwrap();
    assert!(h.join().unwrap().contains("RangeError"));
}

#[test]
fn qy47a_limit_never_exceeds_half_thread_stack() {
    // glibc may reuse a cached larger stack, so compare against the size the
    // thread really reports, not the size requested.
    for mb in [1usize, 2, 8, 16] {
        let h = std::thread::Builder::new()
            .stack_size(mb << 20)
            .spawn(|| (crate::js::js_stack_limit(), crate::js::thread_stack_size()))
            .unwrap();
        let (l, real) = h.join().unwrap();
        let real = real.expect("pthread_getattr_np works on Linux");
        assert!(real >= mb << 20, "{mb} MB thread reports {real}");
        assert!(l > 0 && l <= real / 2 && l <= 4 << 20, "{mb} MB -> {l} (real {real})");
    }
}

#[test]
fn qy50_document_scripts_embeds_plugins_anchors() {
    let log = run_in_html(
        "<html><body><script>1</script><a name=top>t</a><a href=/x>nn</a><embed id=e1><script id=s2>2</script></body></html>",
        "var s=document.scripts;\
         console.log(s.length, Array.from(s).length, s[1].id, typeof s.item, document.embeds.length, document.plugins.length, document.plugins[0]===document.embeds[0]);\
         console.log(document.anchors.length, document.anchors[0].name);\
         var n=document.createElement('script'); document.body.appendChild(n);\
         console.log(document.scripts.length, [...document.scripts].indexOf(n));\
         var m=document.createElement('script'); m.textContent='var q=document.createElement(\"script\");document.body.appendChild(q);';\
         document.body.appendChild(m);\
         console.log(document.scripts.length);",
    );
    assert_eq!(log[0], "log:2 2 s2 function 1 1 true", "{log:?}");
    assert_eq!(log[1], "log:1 top", "{log:?}");
    assert_eq!(log[2], "log:3 2", "{log:?}");
    assert_eq!(log[3], "log:5", "{log:?}");
}
