#[allow(unused_imports)]
use super::*;

// ---------- QB5: focus()/blur(), autofocus, window.scroll* ----------

fn qb5_engine(html: &str, code: &str) -> JsEngine {
    let doc = parse_html(html);
    start_scripts(&doc, &[src(code)], Rc::new(|_, _| {})).unwrap()
}

#[test]
fn qb5_focus_and_blur_are_functions_and_record_requests() {
    let engine = qb5_engine(
        "<html><body><input id=i><div id=d></div></body></html>",
        "var i=document.getElementById('i'), d=document.getElementById('d');
         if (typeof i.focus !== 'function' || typeof i.blur !== 'function') throw new Error('missing');
         d.focus(); // not focusable: ignored
         i.focus();",
    );
    let r = engine.take_requests();
    let Some(crate::js_dom::FocusReq::Focus(h)) = r.focus else { panic!("no focus request: {r:?}") };
    assert_eq!(element_attr(&h, "id").as_deref(), Some("i"));
    assert!(engine.take_requests().focus.is_none(), "drained");
    engine.eval("t2", "document.getElementById('i').blur()");
    assert!(matches!(engine.take_requests().focus, Some(crate::js_dom::FocusReq::Blur(_))));
}

#[test]
fn qb5_autofocus_and_boolean_attributes_reflect() {
    let engine = qb5_engine(
        "<html><body><input id=i autofocus><input id=j></body></html>",
        "var i=document.getElementById('i'), j=document.getElementById('j');
         var r=[i.autofocus, j.autofocus, j.disabled]; j.autofocus=true; j.disabled=true; i.autofocus=false;
         r.push(j.hasAttribute('autofocus'), j.disabled, i.hasAttribute('autofocus'), j.required, j.checked);
         // the portal's idiom: void 0===t.autofocus?t.focus():window.scroll(0,0)
         void 0===j.autofocus?j.focus():window.scroll(0,0);
         globalThis.out = r.join();",
    );
    assert_eq!(engine.eval_value::<String>("t", "out").unwrap(), "true,false,false,true,true,false,false,false");
}

#[test]
fn qb5_window_scroll_family_records_a_request() {
    let engine = qb5_engine("<html><body></body></html>", "window.scrollTo(0, 100);");
    let s = engine.take_requests().scroll.expect("scroll request");
    assert_eq!((s.x, s.y, s.relative), (Some(0.0), Some(100.0), false));
    engine.eval("t", "window.scroll({top: 40}); window.scrollBy(0, 5);");
    let s = engine.take_requests().scroll.unwrap();
    assert_eq!((s.y, s.relative), (Some(5.0), true), "last request wins");
    engine.eval("t", "window.scroll({top: 40})");
    let s = engine.take_requests().scroll.unwrap();
    assert_eq!((s.x, s.y), (None, Some(40.0)), "missing left keeps the axis");
    assert_eq!(engine.eval_value::<f64>("t", "window.scrollY").unwrap(), 40.0);
}

/// QB3: the pointer-idle gate for hover restyles is a pure function of
/// the last build time and the time since the pointer last moved.
#[test]
fn qb3_hover_idle_gate() {
    use std::time::Duration as D;
    let ms = D::from_millis;
    // Fast builds never wait, however recently the pointer moved.
    assert_eq!(hover_idle_wait(Some(ms(20)), Some(ms(0))), None);
    assert_eq!(hover_idle_wait(Some(HOVER_SLOW_BUILD), Some(ms(0))), None);
    // No build measured yet: nothing to protect against.
    assert_eq!(hover_idle_wait(None, Some(ms(0))), None);
    // Slow build, pointer just moved: wait out the idle window.
    assert_eq!(hover_idle_wait(Some(ms(400)), Some(ms(0))), Some(HOVER_POINTER_IDLE));
    assert_eq!(hover_idle_wait(Some(ms(400)), Some(ms(100))), Some(HOVER_POINTER_IDLE - ms(100)));
    // Slow build, pointer still long enough (or never moved): go ahead.
    assert_eq!(hover_idle_wait(Some(ms(400)), Some(HOVER_POINTER_IDLE)), None);
    assert_eq!(hover_idle_wait(Some(ms(400)), Some(ms(5000))), None);
    assert_eq!(hover_idle_wait(Some(ms(400)), None), None);
}

/// QB3: `hover_restyle_delay` uses the gate: a slow page with a moving
/// pointer defers the restyle, a still pointer or a fast page does not.
#[test]
fn qb3_app_defers_hover_while_pointer_moves_on_slow_pages() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
    app.layout_dirty = false;
    app.hold_old_frame = false;
    app.last_build_time = Some(std::time::Duration::from_millis(400));
    app.last_pointer_move = Some(std::time::Instant::now());
    assert!(app.hover_restyle_delay().is_some(), "moving pointer on a slow page defers hover");
    app.last_pointer_move = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    assert!(app.hover_restyle_delay().is_none(), "still pointer applies hover");
    app.last_pointer_move = Some(std::time::Instant::now());
    app.last_build_time = Some(std::time::Duration::from_millis(5));
    assert!(app.hover_restyle_delay().is_none(), "fast page applies hover immediately");
}

/// QB3b: a burst of pointer-driven hover changes on a slow page, then a
/// final move onto a link, must end with the link (the last target)
/// hovered: the pending flush re-arms while the pointer moves and applies
/// the final `hover_node`, never an earlier one. (The "link stays
/// un-highlighted" symptom came from the dev injector dropping its last
/// pointer motion, not from this path; this pins the app side.)
#[test]
fn qb3b_final_hover_target_applied_after_burst() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = match EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let doc = parse_html("<html><head><style>a:hover{color:red}</style></head><body><p>x</p><div>y</div><a href=\"/x\">z</a></body></html>");
    let a = node_id(&find_by_tag(&doc, "a").unwrap());
    let p = node_id(&find_by_tag(&doc, "p").unwrap());
    let div = node_id(&find_by_tag(&doc, "div").unwrap());
    let mut app = App::new_static(doc, event_loop.create_proxy(), None);
    app.layout_dirty = false;
    app.hold_old_frame = false;
    app.state_rules = STATE_HOVER;
    app.last_build_time = Some(std::time::Duration::from_millis(400));
    // The burst: the pointer keeps moving, so every flush is deferred.
    for i in 0..100 {
        app.last_pointer_move = Some(std::time::Instant::now());
        app.update_hover(if i % 2 == 0 { p } else { div });
        assert!(app.hover_restyle_delay().is_some());
    }
    // The last move lands on the link immediately after the burst.
    app.last_pointer_move = Some(std::time::Instant::now());
    app.update_hover(a);
    assert_eq!(app.hover_node, a);
    assert!(app.interaction.hover.is_empty(), "still deferred: pointer not idle");
    // Pointer goes still; the flush that was already pending fires.
    app.last_pointer_move = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
    assert!(app.hover_restyle_delay().is_none());
    app.apply_hover();
    assert!(app.interaction.hover.contains(&a), "the final target is hovered");
    assert!(app.layout_dirty, "and a restyle was requested");
}

/// QB3: timer/event callbacks get ~500 ms, not the 5 s page-load budget.
#[test]
fn qb3_callback_budget_is_much_shorter_than_load_budget() {
    let log = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let l = log.clone();
    let engine = start_scripts(
        &parse_html(""),
        &[src("setTimeout(()=>{ for(;;){} },0);")],
        Rc::new(move |lvl, t| l.borrow_mut().push(format!("{lvl}:{t}"))),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let t = std::time::Instant::now();
    engine.run_due();
    let took = t.elapsed();
    assert!(took < std::time::Duration::from_millis(2500), "callback ran {took:?}");
    assert!(took >= std::time::Duration::from_millis(400), "callback ran {took:?}");
    assert!(log.borrow().iter().any(|m| m.contains("budget")), "{:?}", log.borrow());
}

/// QB9: a writer whose every call fails with EPIPE, which is what
/// `scratchy … 2>&1 | head -1` gives once `head` has exited.
struct ClosedPipe;

impl Write for ClosedPipe {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

/// QB9: diagnostic output on a closed pipe returns normally. The old
/// `eprintln!`/`println!` call sites panicked here (exit 101).
#[test]
fn qb9_write_line_to_ignores_closed_pipe() {
    let mut pipe = ClosedPipe;
    write_line_to(&mut pipe, format_args!("scratchy: could not {}: {}", "draw", 1));
    write_line_to(&mut pipe, format_args!("plain"));
}

/// QB9: the helper writes exactly what `eprintln!` wrote (text plus `\n`).
#[test]
fn qb9_write_line_to_writes_text_and_newline() {
    let mut buf: Vec<u8> = Vec::new();
    write_line_to(&mut buf, format_args!("usage: scratchy {}", "[file|url]"));
    assert_eq!(buf, b"usage: scratchy [file|url]\n");
}

// ---------- QY3: HTMLCanvasElement ----------

#[test]
fn qy3_canvas_element_interface_without_drawing() {
    let (log, _body) = mutate(
        "<html><body><canvas id=c width=40></canvas></body></html>",
        "var c=document.createElement('canvas'); var d=document.createElementNS('http://www.w3.org/1999/xhtml','canvas');\
         console.log(c.getContext('2d'), d.getContext('webgl'), c.width, c.height, c instanceof HTMLCanvasElement, c instanceof HTMLElement);\
         c.width=20; console.log(c.width, c.getAttribute('width'), c.toDataURL());\
         var p=document.getElementById('c'); console.log(p.width, p.height, p.getContext('2d'), typeof p.toBlob);",
    );
    assert_eq!(log, ["log:null null 300 150 true true", "log:20 20 data:,", "log:40 150 null function"], "{log:?}");
}

#[test]
fn qy4_create_event_and_constructor_globals() {
    let log = run_collecting(&[src(
        "var e=document.createEvent('UIEvents');e.initUIEvent('t',true,false,null,6);\
         var d=-1;document.addEventListener('t',function(x){d=x.detail+':'+x.bubbles;});document.dispatchEvent(e);console.log(d);\
         var m=document.createEvent('MouseEvents');m.initMouseEvent('c',true,true,window,1,0,0,7,8,false,false,false,false,0,null);console.log(m.clientX+','+m.clientY+','+(m instanceof MouseEvent));\
         var c=document.createEvent('CustomEvent');c.initCustomEvent('k',false,false,{a:1});console.log(c.detail.a);\
         var v=document.createEvent('Event');v.initEvent('z',true,true);console.log(v.type+v.cancelable);\
         try{document.createEvent('Bogus')}catch(x){console.log(x.name)}\
         console.log(['DocumentFragment','CharacterData','Text','Comment','Attr','NodeList','HTMLCollection','ErrorEvent'].every(function(n){return typeof window[n]==='function'}));\
         var er=new ErrorEvent('error',{message:'m',filename:'f',lineno:3,colno:4,error:5});console.log(er.message+er.filename+er.lineno+er.colno+er.error);\
         console.log((document.body.ownerDocument===document)+','+(document.ownerDocument===null)+','+(document.defaultView===window));\
         console.log((document.createTextNode('x') instanceof Text)+','+(document.createComment('x') instanceof Comment)+','+(document.createDocumentFragment() instanceof DocumentFragment)+','+(document.createTextNode('x') instanceof CharacterData));\
         console.log(NodeFilter.SHOW_ELEMENT+','+NodeFilter.FILTER_ACCEPT+','+NodeFilter.SHOW_ALL);",
    )]);
    assert_eq!(log, ["log:6:true", "log:7,8,true", "log:1", "log:ztrue", "log:NotSupportedError",
        "log:true", "log:mf345", "log:true,true,true", "log:true,true,true,true", "log:1,1,4294967295"]);
}

/// QA17 fixture: root div#R > a(span) > ["x", b], c(i), "t", d(p) > e(em).
const QA17_TREE: &str = "var R=document.createElement('div');R.id='R';\
    function mk(t,id){var e=document.createElement(t);e.id=id;return e}\
    var a=mk('span','a'),b=mk('b','b'),c=mk('i','c'),d=mk('p','d'),e=mk('em','e');\
    a.appendChild(document.createTextNode('x'));a.appendChild(b);R.appendChild(a);R.appendChild(c);\
    R.appendChild(document.createTextNode('t'));d.appendChild(e);R.appendChild(d);\
    function nm(n){return n?(n.id||('#'+n.nodeValue)):'null'}";

#[test]
fn qa17_tree_walker_show_element_walks_forward_and_back() {
    let log = run_collecting(&[src(&format!(
        "{QA17_TREE} var w=document.createTreeWalker(R,NodeFilter.SHOW_ELEMENT,null);\
         var f=[],n;while((n=w.nextNode()))f.push(n.id);console.log(f.join(','));\
         var r=[];while((n=w.previousNode()))r.push(n.id);console.log(r.join(','));\
         console.log(w.root===R, w.whatToShow, w.filter===null, w.currentNode===R);"
    ))]);
    assert_eq!(log, ["log:a,b,c,d,e", "log:d,c,b,a,R", "log:true 1 true true"], "{log:?}");
}

#[test]
fn qa17_tree_walker_filter_skip_vs_reject() {
    let log = run_collecting(&[src(&format!(
        "{QA17_TREE} function walk(code){{var w=document.createTreeWalker(R,NodeFilter.SHOW_ELEMENT,\
           function(n){{return n.id==='a'?code:NodeFilter.FILTER_ACCEPT}});var o=[],n;while((n=w.nextNode()))o.push(n.id);return o.join(',')}}\
         console.log(walk(NodeFilter.FILTER_SKIP));console.log(walk(NodeFilter.FILTER_REJECT));\
         var w=document.createTreeWalker(R,NodeFilter.SHOW_ALL,{{acceptNode:function(n){{return n.nodeType===3?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT}}}});\
         var o=[],n;while((n=w.nextNode()))o.push(nm(n));console.log(o.join(','));"
    ))]);
    assert_eq!(log, ["log:b,c,d,e", "log:c,d,e", "log:a,b,c,d,e"], "{log:?}");
}

#[test]
fn qa17_tree_walker_current_node_and_relatives() {
    let log = run_collecting(&[src(&format!(
        "{QA17_TREE} var w=document.createTreeWalker(R,NodeFilter.SHOW_ELEMENT);\
         var o=[];o.push(nm(w.firstChild()));o.push(nm(w.nextSibling()));o.push(nm(w.nextSibling()));\
         o.push(nm(w.nextSibling()));o.push(nm(w.previousSibling()));o.push(nm(w.lastChild()));o.push(nm(w.parentNode()));\
         o.push(w.currentNode===d);w.currentNode=b;o.push(nm(w.nextNode()));o.push(nm(w.parentNode()));\
         w.currentNode=R;o.push(nm(w.nextSibling()));o.push(nm(w.parentNode()));\
         w.currentNode=d;o.push(nm(w.nextSibling()));w.currentNode=e;o.push(nm(w.nextNode()));\
         console.log(o.join(','));"
    ))]);
    assert_eq!(log, ["log:a,c,d,null,c,null,R,false,c,R,null,null,null,null"], "{log:?}");
}

#[test]
fn qa17_node_iterator_next_and_previous() {
    let log = run_collecting(&[src(&format!(
        "{QA17_TREE} var it=document.createNodeIterator(R,NodeFilter.SHOW_ELEMENT|NodeFilter.SHOW_TEXT);\
         console.log(it.root===R, it.whatToShow, it.referenceNode===R, it.pointerBeforeReferenceNode);\
         var f=[],n;while((n=it.nextNode()))f.push(nm(n));console.log(f.join(','));\
         console.log(it.pointerBeforeReferenceNode, nm(it.referenceNode));\
         var r=[];while((n=it.previousNode()))r.push(nm(n));console.log(r.join(','));\
         console.log(typeof it.detach, it.detach());\
         var o=document.createNodeIterator(R,NodeFilter.SHOW_ELEMENT,function(n){{return n.id==='b'?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT}});\
         var g=[];while((n=o.nextNode()))g.push(n.id);console.log(g.join(','));"
    ))]);
    assert_eq!(log, ["log:true 5 true true", "log:R,a,#x,b,c,#t,d,e", "log:false e",
        "log:e,d,#t,c,b,#x,a,R", "log:function undefined", "log:R,a,c,d,e"], "{log:?}");
}

#[test]
fn qa17_node_iterator_survives_removal_and_filter_reentry_throws() {
    let log = run_collecting(&[src(&format!(
        "{QA17_TREE} var it=document.createNodeIterator(R,NodeFilter.SHOW_ELEMENT);\
         it.nextNode();it.nextNode();it.nextNode();console.log(nm(it.referenceNode));\
         R.removeChild(a);console.log(nm(it.referenceNode),it.pointerBeforeReferenceNode,nm(it.nextNode()));\
         var w;w=document.createTreeWalker(R,NodeFilter.SHOW_ALL,function(){{try{{w.nextNode()}}catch(x){{console.log(x.name)}}return 1}});w.nextNode();\
         try{{document.createTreeWalker()}}catch(x){{console.log(x.name)}}"
    ))]);
    assert_eq!(log, ["log:b", "log:R false c", "log:InvalidStateError", "log:TypeError"], "{log:?}");
}

/// QY5: a runaway page-load script is stopped with one clear line naming
/// the script and the budget, not a bare `InternalError: interrupted`.
#[test]
fn qy5_interrupted_page_script_logs_a_clear_message() {
    let log = run_collecting(&[ScriptSource { name: "big.js".into(), code: "for(;;){}".into(), index: None, ..Default::default() }, src("console.log('after');")]);
    let errs: Vec<_> = log.iter().filter(|l| l.starts_with("error:")).collect();
    assert_eq!(errs.len(), 1, "{log:?}");
    assert!(errs[0].contains("big.js stopped: exceeded the 5 s"), "{log:?}");
    assert!(errs[0].contains("budget") && errs[0].contains("KB of source"), "{log:?}");
    assert!(!errs[0].contains("InternalError"), "{log:?}");
    assert!(log.iter().any(|l| l == "log:after"), "{log:?}");
}

/// QY5: same for a timer callback, with the callback budget.
#[test]
fn qy5_interrupted_callback_logs_a_clear_message() {
    let log = run_with_timers("setTimeout(()=>{ for(;;){} },0);");
    assert!(log.iter().any(|l| l.starts_with("error:js: timer stopped: exceeded the 500 ms")), "{log:?}");
    assert!(!log.iter().any(|l| l.contains("InternalError")), "{log:?}");
}

/// QY5: the budget clock is thread CPU time: it advances when the thread
/// works and not while it sleeps (a slow-to-compile or merely waiting
/// script on a loaded machine is not killed for wall-clock time).
#[test]
fn qy5_budget_clock_is_thread_cpu_time() {
    let a = crate::js::thread_cpu_nanos();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let slept = crate::js::thread_cpu_nanos() - a;
    assert!(slept < 100_000_000, "sleeping used {slept} ns of CPU");
    let t = std::time::Instant::now();
    let mut x = 0u64;
    while t.elapsed() < std::time::Duration::from_millis(100) {
        x = x.wrapping_add(1);
        std::hint::black_box(x);
    }
    let spun = crate::js::thread_cpu_nanos() - a - slept;
    assert!(spun > 20_000_000, "spinning used {spun} ns of CPU");
}

#[test]
fn absolutize_css_urls_resolves_relative_against_sheet_q35a() {
    let sheet = Url::parse("https://en.wikipedia.org/w/load.php?x=1").unwrap();
    let css = ".a{background:url(img/a.png)} .b{mask-image:url( '../m.svg' )} .c{background-image:url(\"data:image/png;base64,AAAA\")} .d{background:url(https://other/z.png)} .e{filter:url(#f)} .f{background:URL(/abs.png) no-repeat} .g{x:myurl(rel)}";
    let out = absolutize_css_urls(css, &sheet);
    assert!(out.contains("url(\"https://en.wikipedia.org/w/img/a.png\")"), "{out}");
    assert!(out.contains("url(\"https://en.wikipedia.org/m.svg\")"), "{out}");
    assert!(out.contains("url(\"data:image/png;base64,AAAA\")"), "{out}");
    assert!(out.contains("url(https://other/z.png)"), "{out}");
    assert!(out.contains("url(#f)"), "{out}");
    assert!(out.contains("url(\"https://en.wikipedia.org/abs.png\") no-repeat"), "{out}");
    assert!(out.contains("myurl(rel)"), "{out}");
    // a data: sheet has no base: left alone
    let data = Url::parse("data:text/css,").unwrap();
    assert_eq!(absolutize_css_urls(css, &data), css);
    // imported sheets resolve against their own url
    let mut stack = vec![sheet.clone()];
    let expanded = expand_css_imports("@import url(sub/x.css);\n.k{background:url(k.png)}", &sheet, &mut stack, &mut |u| {
        assert_eq!(u.as_str(), "https://en.wikipedia.org/w/sub/x.css");
        Some(".i{background:url(i.png)}".to_string())
    });
    assert!(expanded.contains("https://en.wikipedia.org/w/sub/i.png"), "{expanded}");
    assert!(expanded.contains("https://en.wikipedia.org/w/k.png"), "{expanded}");
}

#[test]
fn external_background_and_mask_urls_use_image_cache_q35a() {
    let _ = take_wanted_css_images();
    let doc = parse_html(r#"<html><body><span class="s">x</span><i class="m">y</i></body></html>"#);
    let rules = parse_stylesheet(
        r#".s{background-image:url("https://h.test/sprite.svg")} .m{background-color:red;mask-image:url("https://h.test/icon.png")}"#,
    );
    let s = find_element_by_tag(&doc, "span").unwrap();
    let m = find_element_by_tag(&doc, "i").unwrap();
    let mut cache: HashMap<String, Arc<DecodedImage>> = HashMap::new();
    // Not fetched yet: nothing resolves, but both urls are reported as wanted.
    assert!(resolve_background_image(&s, &rules, 800.0, None, &cache).is_none());
    assert!(resolve_background_image(&m, &rules, 800.0, None, &cache).is_none());
    let mut wanted = take_wanted_css_images();
    wanted.sort();
    assert_eq!(wanted, ["https://h.test/icon.png", "https://h.test/sprite.svg"]);
    // Fetched: the cached images resolve, as a background and as a mask.
    let img = decode_image_bytes(&decode_data_uri(&tiny_png_data_uri(3, 2)).unwrap()).unwrap();
    cache.insert("https://h.test/sprite.svg".into(), Arc::new(img.clone()));
    cache.insert("https://h.test/icon.png".into(), Arc::new(img));
    let bg = resolve_background_image(&s, &rules, 800.0, None, &cache).expect("cached background resolves");
    assert_eq!((bg.width, bg.height), (3, 2));
    assert!(bg.mask.is_none());
    let masked = resolve_background_image(&m, &rules, 800.0, None, &cache).expect("cached mask resolves");
    assert!(masked.mask.is_some(), "mask-image url must come from the cache");
    assert!(take_wanted_css_images().is_empty());
}

#[test]
fn layout_build_reports_wanted_css_images_and_uses_cache_q35a() {
    let _ = take_wanted_css_images();
    let doc = parse_html(r#"<html><body><div style="width:20px;height:20px;background-image:url(http://h.test/a.png)">x</div></body></html>"#);
    let mut cache: HashMap<String, Arc<DecodedImage>> = HashMap::new();
    let _ = build_layout_tree_with_resources(&doc, 400.0, Some(300.0), "", &cache);
    assert_eq!(take_wanted_css_images(), ["http://h.test/a.png"]);
    let img = decode_image_bytes(&decode_data_uri(&tiny_png_data_uri(2, 2)).unwrap()).unwrap();
    cache.insert("http://h.test/a.png".into(), Arc::new(img));
    let tree = build_layout_tree_with_resources(&doc, 400.0, Some(300.0), "", &cache);
    assert!(take_wanted_css_images().is_empty());
    fn any_bg(b: &LayoutBox) -> bool {
        b.background_image.is_some()
            || match &b.content {
                BoxContent::Blocks(c) => c.iter().any(any_bg),
                _ => false,
            }
    }
    assert!(any_bg(&tree), "the cached image must be attached to the div's box");
}

// ---------- QY10: window.matchMedia ----------

#[test]
fn qy10_match_media_features_and_combinations() {
    let _m = MOTION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = run_collecting(&[src(
        "var q = function (s) { return window.matchMedia(s).matches ? 1 : 0; };\
         console.log([q('(min-width: 1px)'), q('(max-width: 1px)'), q('(min-width: 1024px)'), q('(max-width: 1023px)'),\
           q('(min-width: 600px) and (max-width: 2000px)'), q('(min-width: 600px) and (max-width: 800px)'),\
           q('(prefers-color-scheme: light)'), q('(prefers-color-scheme: dark)'), q('print'), q('screen'),\
           q('not all'), q('not print'), q('(min-height: 700px)'), q('(orientation: landscape)'), q('(orientation: portrait)'),\
           q('(hover: hover)'), q('(pointer: coarse)'), q('(prefers-reduced-motion: reduce)'), q('(min-resolution: 2dppx)'),\
           q('(min-resolution: 1dppx)'), q('print, (min-width: 1px)'), q('(width >= 600px)'), q('(width < 600px)'),\
           q('screen and (min-width: 1px)'), q('(forced-colors: active)'), q('')].join(','));",
    )]);
    assert_eq!(out, vec!["log:1,0,1,0,1,0,1,0,0,1,0,1,1,1,0,1,0,1,0,1,1,1,0,1,0,1"]);
}

#[test]
fn qy10_match_media_invalid_is_not_all_and_media_is_reported() {
    let out = run_collecting(&[src(
        "var a = matchMedia('(((garbage'), b = matchMedia('(MIN-WIDTH: 10px)'), c = matchMedia('screen and');\
         console.log(a.matches, a.media, b.matches, b.media, c.matches, c.media, matchMedia('').media === '');",
    )]);
    assert_eq!(out, vec!["log:false not all true (min-width: 10px) false not all true"]);
}

#[test]
fn qy10_match_media_listeners_and_class() {
    let out = run_collecting(&[src(
        "var m = matchMedia('(min-width: 1px)'), n = 0, f = function () { n++; };\
         var k = 0, g2 = function () { k++; }; m.addListener(g2);\
         m.addListener(f); m.addEventListener('change', f); m.removeListener(f);\
         m.dispatchEvent(new Event('change')); if (k !== 1) n += 100; m.removeListener(g2);\
         m.dispatchEvent(new Event('change')); if (k !== 1) n += 1000;\
         m.removeEventListener('change', f);\
         m.dispatchEvent(new Event('change'));\
         m.onchange = function () { n += 10; }; m.dispatchEvent(new Event('change'));\
         console.log(m instanceof MediaQueryList, m instanceof EventTarget, typeof m.addListener, m.onchange !== undefined, n);",
    )]);
    assert_eq!(out, vec!["log:true true function true 10"]);
}

// ---------- QJ4: geometry reads (getBoundingClientRect, offset*, client*, scroll*) ----------

#[test]
fn qj4_geometry_reads_layout_backed_rects() {
    let log = run_in_html(
        "<html><head><style>body{margin:0} article{display:block;height:100px;margin:0;border:2px solid red;padding:5px;width:300px}\
         #p{padding:0;border:0;height:0;position:relative;top:0}</style></head>\
         <body><article id=a1><p class=meta id=m1>one</p></article><article id=a2>two</article><article id=a3>three</article>\
         <div id=n style='display:none'><b id=nb>x</b></div><div id=p><i id=ch style='display:block;height:10px;margin-top:20px'></i></div></body></html>",
        r#"var a = ['a1','a2','a3'].map(function (i) { return document.getElementById(i).getBoundingClientRect(); });
         console.log('tops', a[0].top < a[1].top && a[1].top < a[2].top, a[0].height, a[0].width, a[0].top, a[0].left);
         console.log('sh', document.documentElement.scrollHeight >= a[0].height + a[1].height + a[2].height, document.documentElement.clientHeight === innerHeight, document.documentElement.clientWidth === innerWidth);
         var e1 = document.getElementById('a1');
         console.log('off', e1.offsetWidth, e1.offsetHeight, e1.clientWidth, e1.clientHeight, e1.clientLeft, e1.clientTop, e1.scrollHeight);
         var n = document.getElementById('n'), z = n.getBoundingClientRect();
         console.log('none', z.width, z.height, z.top, document.getElementById('nb').getBoundingClientRect().height, n.offsetHeight, n.getClientRects().length);
         var d = document.createElement('div'); console.log('det', d.getBoundingClientRect().height, d.offsetWidth, d.getClientRects().length, d.offsetParent);
         console.log('offp', document.getElementById('ch').offsetParent.id, document.getElementById('ch').offsetTop);
         document.documentElement.scrollTop = 0; e1.scrollTop = 7; console.log('st', e1.scrollTop, document.documentElement.scrollTop, e1.scrollLeft);
         console.log('rect', e1.getBoundingClientRect() instanceof DOMRect, e1.getClientRects().item(0).height);
         e1.style.height = '300px';
         console.log('after', e1.getBoundingClientRect().height);
         scrollTo(0, 50); console.log('scrolled', document.getElementById('a1').getBoundingClientRect().top);"#,
    );
    let has = |s: &str| log.iter().any(|l| l == s);
    assert!(has("log:tops true 114 314 0 0"), "{log:?}");
    assert!(has("log:sh true true true"), "{log:?}");
    assert!(has("log:off 314 114 310 110 2 2 110"), "{log:?}");
    assert!(has("log:none 0 0 0 0 0 0"), "{log:?}");
    assert!(has("log:det 0 0 0 null"), "{log:?}");
    assert!(has("log:offp p 20"), "{log:?}");
    // QJ6: a non-scrolling (overflow: visible) box keeps scrollTop 0, as in a browser.
    assert!(has("log:st 0 0 0"), "{log:?}");
    assert!(has("log:rect true 114"), "{log:?}");
    assert!(has("log:after 314"), "{log:?}");
    assert!(has("log:scrolled -50"), "{log:?}");
}

#[test]
fn qj4_forced_layout_time_is_not_charged_to_the_script_budget() {
    let before = crate::js::script_clock_nanos();
    let start = crate::js::thread_cpu_nanos();
    let mut x = 0u64;
    while crate::js::thread_cpu_nanos() - start < 20_000_000 {
        x = x.wrapping_add(1);
    }
    std::hint::black_box(x);
    crate::js::pause_budget_since(start);
    let charged = crate::js::script_clock_nanos() - before;
    assert!(charged < 10_000_000, "paused work still charged {charged} ns");
}

#[test]
fn qj4_scroll_height_sees_overflowing_content() {
    let log = run_in_html(
        "<html><body style='margin:0'><div id=b style='height:50px;overflow:hidden;width:200px'><div style='height:400px'></div></div></body></html>",
        "var b = document.getElementById('b'); console.log('s', b.clientHeight, b.scrollHeight);",
    );
    assert!(log.contains(&"log:s 50 400".to_string()), "{log:?}");
}

// ---------- QJ5: Element.scrollIntoView ----------

#[test]
fn qj5_scroll_into_view_requests_window_scroll_from_layout_rect() {
    let doc = "<html><head><style>body{margin:0} div.s{height:2000px} #t{height:100px;background:red} #f{position:fixed;top:0;height:10px}</style></head>\
               <body><div class=s></div><div id=t></div><div class=s></div><div id=f></div></body></html>";
    let num = |log: &[String], key: &str| -> f64 { log.iter().find_map(|l| l.strip_prefix(key)).unwrap_or("NaN").parse().unwrap() };
    let vh = num(&run_in_html(doc, "console.log('vh', innerHeight);"), "log:vh ");
    let y = |js: &str| num(&run_in_html(doc, &format!("var t = document.getElementById('t'); {js}; console.log('y', scrollY);")), "log:y ");
    let top = 2000.0;
    assert_eq!(y("t.scrollIntoView()"), top);
    assert_eq!(y("t.scrollIntoView(true)"), top);
    assert_eq!(y("t.scrollIntoView({block:'start'})"), top);
    assert_eq!(y("t.scrollIntoView(false)"), top + 100.0 - vh);
    assert_eq!(y("t.scrollIntoView({block:'end'})"), top + 100.0 - vh);
    assert_eq!(y("t.scrollIntoView({block:'center'})"), top - (vh - 100.0) / 2.0);
    // nearest: below the viewport aligns the end; already visible does not move.
    assert_eq!(y("t.scrollIntoView({block:'nearest'})"), top + 100.0 - vh);
    assert_eq!(y("scrollTo(0, 2000); t.scrollIntoView({block:'nearest'})"), top);
    assert_eq!(y("scrollTo(0, 1950); t.scrollIntoView({block:'nearest'})"), 1950.0, "fully visible: no move");
    assert_eq!(y("scrollTo(0, 2050); t.scrollIntoView({block:'nearest'})"), top, "cut at the top: align start");
    // fixed and detached elements do nothing
    assert_eq!(y("document.getElementById('f').scrollIntoView()"), 0.0);
    assert_eq!(y("document.createElement('p').scrollIntoView()"), 0.0);
}

// ---------- QJ7: DOM Range (boundary points + client rects) ----------

#[test]
fn qj7_range_boundary_points_follow_the_spec() {
    let log = run_in_html(
        "<html><body><div id=d>ab<b id=b>cd</b>ef</div><p id=p></p></body></html>",
        r#"var d = document.getElementById('d'), b = document.getElementById('b'), t0 = d.firstChild, t1 = b.firstChild, t2 = d.lastChild;
         var r = document.createRange(), s = new Range();
         console.log('init', r.startContainer === document, r.startOffset, r.collapsed, s.endOffset, String(r));
         r.setStart(t0, 1); r.setEnd(t2, 1);
         console.log('set', r.startOffset, r.endOffset, r.collapsed, r.commonAncestorContainer === d, r.toString());
         r.selectNodeContents(b); console.log('snc', r.startContainer === b, r.endOffset, r.toString());
         r.selectNode(b); console.log('sn', r.startContainer === d, r.startOffset, r.endOffset, r.toString());
         r.collapse(true); console.log('col', r.collapsed, r.endOffset);
         r.setStart(t2, 2); console.log('after-end', r.startContainer === t2, r.endContainer === t2, r.endOffset, r.collapsed);
         var q = r.cloneRange(); q.setEnd(d, 3); console.log('clone', r.endContainer === t2, q.endContainer === d);
         var e = []; [function () { r.setStart(t0, 1); }, function () { r.setEnd(t0, 99); }, function () { r.setStart(document.implementation.createDocumentType('html', '', ''), 0); }, function () { r.selectNode(document); }].forEach(function (f) { try { f(); e.push('ok'); } catch (x) { e.push(x.name); } });
         console.log('err', e.join());
         var a = document.createRange(), c = document.createRange(); a.setStart(t0, 0); a.setEnd(t1, 1); c.setStart(t1, 0); c.setEnd(t2, 2);
         console.log('cmp', a.compareBoundaryPoints(Range.START_TO_START, c), a.compareBoundaryPoints(Range.END_TO_START, c), a.compareBoundaryPoints(Range.END_TO_END, c), c.compareBoundaryPoints(Range.START_TO_END, a));
         console.log('pt', a.isPointInRange(t0, 1), a.isPointInRange(t2, 0), a.comparePoint(t2, 0), a.comparePoint(t0, 0), a.intersectsNode(b), a.intersectsNode(document.getElementById('p')));
         var det = document.createElement('i'); det.appendChild(document.createTextNode('xy'));
         a.setStart(t0, 0); a.setEnd(det.firstChild, 1); console.log('root', a.startContainer === det.firstChild, a.collapsed);
         var w = document.createRange(); w.setStartBefore(b); w.setEndAfter(b); console.log('ba', w.startOffset, w.endOffset, w.toString());"#,
    );
    let has = |s: &str| log.iter().any(|l| l == s);
    assert!(has("log:init true 0 true 0 "), "{log:?}");
    assert!(has("log:set 1 1 false true bcde"), "{log:?}");
    assert!(has("log:snc true 1 cd"), "{log:?}");
    assert!(has("log:sn true 1 2 cd"), "{log:?}");
    assert!(has("log:col true 1"), "{log:?}");
    assert!(has("log:after-end true true 2 true"), "{log:?}");
    assert!(has("log:clone true true"), "{log:?}");
    assert!(has("log:err ok,IndexSizeError,InvalidNodeTypeError,InvalidNodeTypeError"), "{log:?}");
    assert!(has("log:cmp -1 -1 -1 1"), "{log:?}");
    assert!(has("log:pt true false 1 0 true false"), "{log:?}");
    assert!(has("log:root true true"), "{log:?}");
    assert!(has("log:ba 1 2 cd"), "{log:?}");
}

#[test]
fn qj7_range_client_rects_come_from_layout_per_character_and_line() {
    let log = run_in_html(
        "<html><head><style>body{margin:0} #t{font:20px monospace;width:2000px;margin:0;padding:0} #w{font:20px monospace;width:60px;margin:0}\
         #c{font:20px monospace;margin:0;position:absolute;left:100px;top:300px}</style></head>\
         <body><p id=t>abc</p><p id=w>aaa bbb ccc</p><div id=c>x<b>yz</b></div><div id=n style='display:none'>hid</div></body></html>",
        r#"var t = document.getElementById('t').firstChild, r = document.createRange(), rc = [];
         for (var i = 0; i < 3; i++) { r.setStart(t, i); r.setEnd(t, i + 1); rc.push(r.getBoundingClientRect()); }
         var same = rc[0].top === rc[1].top && rc[1].top === rc[2].top && rc[0].height > 0;
         var order = rc[0].left < rc[1].left && rc[1].left < rc[2].left && rc[0].width > 5 && Math.abs(rc[0].width - rc[1].width) < 0.5;
         console.log('chars', same, order);
         r.selectNodeContents(t); var all = r.getClientRects(), bb = r.getBoundingClientRect();
         console.log('all', all.length, Math.abs(bb.width - (rc[0].width + rc[1].width + rc[2].width)) < 0.5, bb.left === rc[0].left, all.item(0).width === bb.width, all.item(5));
         var w = document.getElementById('w').firstChild; r.selectNodeContents(w);
         var lines = r.getClientRects(); console.log('wrap', lines.length >= 2, lines[0].top < lines[1].top);
         r.setStart(w, 4); r.setEnd(w, 7); console.log('part', r.getClientRects().length >= 1, r.getBoundingClientRect().width > 0);
         r.collapse(true); console.log('collapsed', r.getClientRects().length, r.getBoundingClientRect().width);
         var c = document.getElementById('c'); r.selectNodeContents(c); var cb = r.getBoundingClientRect();
         console.log('abs', Math.round(cb.left), Math.round(cb.top), cb.width > 25);
         r.selectNodeContents(document.getElementById('n')); console.log('hidden', r.getClientRects().length, r.getBoundingClientRect().height);
         var d = document.createTextNode('zz'), q = document.createRange(); q.selectNodeContents(d); console.log('detached', q.getClientRects().length);
         t.data = 'abcdef'; r.setStart(t, 0); r.setEnd(t, 6); console.log('mutated', r.getBoundingClientRect().width > rc[0].width * 5.5);"#,
    );
    let has = |s: &str| log.iter().any(|l| l == s);
    assert!(has("log:chars true true"), "{log:?}");
    assert!(has("log:all 1 true true true null"), "{log:?}");
    assert!(has("log:wrap true true"), "{log:?}");
    assert!(has("log:part true true"), "{log:?}");
    assert!(has("log:collapsed 0 0"), "{log:?}");
    assert!(has("log:abs 100 300 true"), "{log:?}");
    assert!(has("log:hidden 0 0"), "{log:?}");
    assert!(has("log:detached 0"), "{log:?}");
    assert!(has("log:mutated true"), "{log:?}");
}

// ---------- QY34: script-inserted <script> elements ----------

fn qy34_status(status: u16, body: &str) -> Result<WireResponse, NetError> {
    Ok(WireResponse { status, status_text: String::new(), headers: vec![], body: body.as_bytes().to_vec(), error: None, final_url: None })
}

fn qy34_run(page: &str, answers: Vec<(&'static str, Result<WireResponse, NetError>)>, code: &str) -> (Vec<String>, Arc<QfFake>) {
    let fake = QfFake::new(answers);
    let (engine, log) = qf7_engine(page, &fake);
    engine.eval("t", code);
    qf7_drive(&engine, &fake);
    engine.run_due();
    qf7_drive(&engine, &fake);
    engine.run_due();
    let l = log.borrow().clone();
    (l, fake)
}

#[test]
fn qy34_inline_script_runs_synchronously_with_current_script_in_insertion_order() {
    let (l, fake) = qy34_run(
        "https://page.test/",
        vec![],
        "var a = document.createElement('script'); a.id = 'a'; a.textContent = \"console.log('run a', document.currentScript && document.currentScript.id);\";\
         var b = document.createElement('script'); b.id = 'b'; b.text = \"console.log('run b', document.currentScript.id);\";\
         document.head.appendChild(a); console.log('between'); document.body.appendChild(b); console.log('after', document.currentScript);",
    );
    assert_eq!(l, vec!["log:run a a", "log:between", "log:run b b", "log:after null"], "{l:?}");
    assert!(fake.urls().is_empty());
}

#[test]
fn qy34_src_script_goes_through_the_queue_runs_async_and_fires_load() {
    let (l, fake) = qy34_run(
        "https://page.test/dir/",
        vec![("/x.js", qy34_status(200, "console.log('x ran', document.currentScript.id);"))],
        "var s = document.createElement('script'); s.id = 'sx'; s.src = 'x.js';\
         s.onload = function () { console.log('load', this.id); }; s.addEventListener('error', function () { console.log('error'); });\
         document.head.appendChild(s); console.log('appended');",
    );
    assert_eq!(l, vec!["log:appended", "log:x ran sx", "log:load sx"], "{l:?}");
    assert_eq!(fake.urls(), vec!["https://page.test/dir/x.js"]);
}

#[test]
fn qy34_http_error_and_policy_refusal_fire_error_and_log() {
    let (l, fake) = qy34_run(
        "https://page.test/",
        vec![("/missing.js", qy34_status(404, "nope"))],
        "['/missing.js', 'ftp://page.test/p.js'].forEach(function (u) {\
           var s = document.createElement('script'); s.src = u;\
           s.onload = function () { console.log('load', u); }; s.onerror = function () { console.log('error', u); };\
           document.head.appendChild(s); });",
    );
    assert!(l.contains(&"log:error /missing.js".to_string()), "{l:?}");
    assert!(l.contains(&"log:error ftp://page.test/p.js".to_string()), "{l:?}");
    assert!(!l.iter().any(|x| x.starts_with("log:load")), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("error:https://page.test/missing.js: failed to load script (HTTP 404)")), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("net:blocked GET ftp://page.test/p.js")), "{l:?}");
    assert_eq!(fake.urls(), vec!["https://page.test/missing.js"]);
}

#[test]
fn qy34_a_moved_script_is_not_run_again_and_innerhtml_scripts_never_run() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![],
        "var s = document.createElement('script'); s.textContent = \"console.log('once');\";\
         document.head.appendChild(s); document.body.appendChild(s); document.head.insertBefore(s, null);\
         var c = s.cloneNode(true); document.body.appendChild(c);\
         var d = document.createElement('div'); d.innerHTML = '<script>console.log(\"inner\")<\\/script>'; document.body.appendChild(d);\
         var late = document.createElement('script'); document.body.appendChild(late); late.textContent = \"console.log('late');\";\
         var det = document.createElement('div'); var ds = document.createElement('script'); ds.text = \"console.log('detached');\"; det.appendChild(ds);\
         console.log('before attach'); document.body.appendChild(det);",
    );
    assert_eq!(l, vec!["log:once", "log:late", "log:before attach", "log:detached"], "{l:?}");
}

#[test]
fn qy34_type_nomodule_and_language_checks() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![],
        "function add(attrs) { var s = document.createElement('script'); for (var k in attrs) s.setAttribute(k, attrs[k]); s.textContent = \"console.log('ran', '\" + JSON.stringify(attrs) + \"');\"; document.head.appendChild(s); }\
         add({type: 'text/plain'}); add({type: 'application/json'}); add({nomodule: ''}); add({type: ''}); add({type: ' Text/JavaScript '});\
         add({language: 'javascript'}); add({language: 'vbscript'}); add({type: '', language: 'vbscript'});",
    );
    let ran: Vec<&str> = l.iter().map(String::as_str).collect();
    assert_eq!(
        ran,
        vec![
            "log:ran {\"type\":\"\"}",
            "log:ran {\"type\":\" Text/JavaScript \"}",
            "log:ran {\"language\":\"javascript\"}",
            "log:ran {\"type\":\"\",\"language\":\"vbscript\"}",
        ],
        "{l:?}"
    );
}

#[test]
fn qy34_async_false_scripts_run_in_insertion_order() {
    let fake = QfFake::new(vec![("/slow.js", qy34_status(200, "console.log('slow');")), ("/fast.js", qy34_status(200, "console.log('fast');"))]);
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "['slow', 'fast'].forEach(function (n) { var s = document.createElement('script'); s.async = false; s.src = '/' + n + '.js';\
         s.onload = function () { console.log('load', n); }; document.head.appendChild(s); });",
    );
    qf7_drive(&engine, &fake);
    assert_eq!(*log.borrow(), vec!["log:slow", "log:load slow", "log:fast", "log:load fast"]);
}

#[test]
fn qy34_module_script_inline_and_src_with_import() {
    let (l, fake) = qy34_run(
        "https://page.test/",
        vec![
            ("/m.js", qy34_status(200, "import { v } from './dep.js'; console.log('m', v, document.currentScript);")),
            ("/dep.js", qy34_status(200, "export const v = 42;")),
        ],
        "var s = document.createElement('script'); s.type = 'module'; s.src = '/m.js'; s.onload = function () { console.log('load m'); };\
         document.head.appendChild(s);\
         var i = document.createElement('script'); i.type = 'module'; i.textContent = \"console.log('inline module', typeof import.meta.url);\"; document.head.appendChild(i);",
    );
    assert!(l.contains(&"log:m 42 null".to_string()), "{l:?}");
    assert!(l.contains(&"log:load m".to_string()), "{l:?}");
    assert!(l.contains(&"log:inline module string".to_string()), "{l:?}");
    assert_eq!(fake.urls(), vec!["https://page.test/m.js", "https://page.test/dep.js"]);
}

#[test]
fn qy34_script_errors_are_logged_and_do_not_break_the_inserter() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![("/bad.js", qy34_status(200, "throw new Error('boom');"))],
        "var s = document.createElement('script'); s.textContent = 'null.x'; document.head.appendChild(s); console.log('still here');\
         var t = document.createElement('script'); t.src = '/bad.js'; t.onload = function () { console.log('load bad'); }; document.head.appendChild(t);",
    );
    assert!(l.iter().any(|x| x.starts_with("error:inline script (inserted): TypeError")), "{l:?}");
    assert!(l.contains(&"log:still here".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("error:https://page.test/bad.js: Error: boom")), "{l:?}");
    assert!(l.contains(&"log:load bad".to_string()), "{l:?}");
}

#[test]
fn qy34_cross_origin_script_runs_without_cors_and_is_not_a_suppressed_beacon() {
    let (l, fake) = qy34_run(
        "https://page.test/",
        vec![("cdn.test/a.js", qy34_status(200, "console.log('cdn ran');")), ("cdn.test/c.js", qy34_status(200, "console.log('cors ran');"))],
        "var s = document.createElement('script'); s.src = 'https://cdn.test/a.js'; s.onload = function () { console.log('load a'); }; document.head.appendChild(s);\
         var c = document.createElement('script'); c.src = 'https://cdn.test/c.js'; c.crossOrigin = 'anonymous'; c.onerror = function () { console.log('cors error'); }; document.head.appendChild(c);",
    );
    assert!(l.contains(&"log:cdn ran".to_string()), "{l:?}");
    assert!(l.contains(&"log:load a".to_string()), "{l:?}");
    // crossorigin=anonymous is a CORS request: no Access-Control-Allow-Origin, so it is blocked.
    assert!(l.contains(&"log:cors error".to_string()), "{l:?}");
    assert!(!l.contains(&"log:cors ran".to_string()), "{l:?}");
    assert!(!l.iter().any(|x| x.contains("beacon")), "{l:?}");
    assert_eq!(fake.urls().len(), 2);
}

#[test]
fn qy34_runaway_fetched_script_hits_the_cpu_budget_and_later_scripts_still_run() {
    let (l, _) = qy34_run(
        "https://page.test/",
        vec![("/loop.js", qy34_status(200, "for(;;){}")), ("/ok.js", qy34_status(200, "console.log('ok after');"))],
        "['loop', 'ok'].forEach(function (n) { var s = document.createElement('script'); s.src = '/' + n + '.js'; document.head.appendChild(s); });",
    );
    assert!(l.iter().any(|x| x.starts_with("error:js: https://page.test/loop.js stopped: exceeded the 5 s CPU-time page-load budget")), "{l:?}");
    assert!(l.contains(&"log:ok after".to_string()), "{l:?}");
}

/// QY42: a non-empty descendant's definite px `width` is its max-content
/// contribution (YouTube's 40 px kebab button around a 20 px icon).
#[test]
fn qy42_definite_width_descendant_non_empty() {
    let mk = |inner: &str| {
        format!(
            r#"<html><body style="margin:0"><div style="display:flex;width:500px"><div style="flex:none;display:flex">{inner}</div><div style="width:10px;height:5px"></div></div></body></html>"#
        )
    };
    let marker_x = |html: &str| {
        let laid = measure_fixture(html, 600.0);
        find_by_width(&laid, 10.0).expect("marker item").x
    };
    let base = r#"<span style="display:inline-block;width:40px"><span style="display:inline-block;width:20px;height:3px"></span></span>"#;
    assert!((marker_x(&mk(base)) - 40.0).abs() < 1.0);
    let bb = r#"<span style="display:inline-block;width:40px;box-sizing:border-box;padding:0 5px"><span style="display:inline-block;width:20px;height:3px"></span></span>"#;
    assert!((marker_x(&mk(bb)) - 40.0).abs() < 1.0);
    let mx = r#"<span style="display:inline-block;width:40px;max-width:30px"><span style="display:inline-block;width:20px;height:3px"></span></span>"#;
    assert!((marker_x(&mk(mx)) - 30.0).abs() < 1.0);
    let mn = r#"<span style="display:inline-block;width:40px;min-width:50px"><span style="display:inline-block;width:20px;height:3px"></span></span>"#;
    assert!((marker_x(&mk(mn)) - 50.0).abs() < 1.0);
    // YouTube chain: flex wrapper > flex > inline-flex > flex a > inline-block 40px > inline-block 100% > inline-block 24px
    let yt = r#"<div style="display:flex"><span style="display:inline-flex"><a style="display:flex"><span style="display:inline-block;width:40px;height:40px"><span style="display:inline-block;width:100%"><span style="display:inline-block;width:24px;height:24px"></span></span></span></a></span><span style="display:inline-block;width:95px">x</span></div>"#;
    assert!((marker_x(&mk(yt)) - 135.0).abs() < 1.0, "yt chain {}", marker_x(&mk(yt)));
    // max-width: fit-content must keep measuring the content (WPT).
    let fc = r#"<span style="display:inline-block;width:40px;max-width:fit-content"><span style="display:inline-block;width:20px;height:3px"></span></span>"#;
    assert!((marker_x(&mk(fc)) - 20.0).abs() < 1.0);
}

// ---------- QY44b: window.screen / visualViewport ----------

#[test]
fn qy44b_window_screen_and_visual_viewport() {
    let out = run_collecting(&[src(
        "var r = [];\
         r.push(typeof screen, screen instanceof Screen, screen === window.screen, screen.width >= innerWidth, screen.height >= innerHeight);\
         r.push(screen.availWidth > 0, screen.availHeight > 0, screen.availLeft, screen.availTop, screen.colorDepth, screen.pixelDepth);\
         r.push(typeof screen.orientation.type, screen.orientation.angle, typeof screen.orientation.addEventListener);\
         screen.orientation.addEventListener('change', function () {}); screen.orientation.removeEventListener('change', function () {});\
         var Jp = { height: window?.screen.height, width: window?.screen.width }; r.push(Jp.height === innerHeight);\
         r.push(screenX, screenY, screenLeft, screenTop);\
         r.push(visualViewport.width === innerWidth, visualViewport.height === innerHeight, visualViewport.scale, visualViewport.offsetLeft, visualViewport.offsetTop, visualViewport.pageLeft, visualViewport.pageTop);\
         visualViewport.addEventListener('resize', function () {}); visualViewport.removeEventListener('resize', function () {});\
         console.log(r.join('|'));",
    )]);
    assert_eq!(
        out,
        vec!["log:object|true|true|true|true|true|true|0|0|24|24|string|0|function|true|0|0|0|0|true|true|1|0|0|0|0"]
    );
}

/// QY43: a preserved newline inside a paragraph on the per-word path
/// (inline-block / padded inline present) is a forced break; a blank line
/// ("\n\n") is an empty line; no two word boxes on a line overlap.
type Qy43Words = Vec<(f32, f32, f32)>;
fn qy43_items(html: &str) -> (Qy43Words, Option<(f32, f32)>) {
    let laid = measure_fixture(html, 800.0);
    let run = find_mixed_inline(&laid).expect("mixed-inline path");
    let items = mixed_items(run);
    let words: Vec<(f32, f32, f32)> = items
        .iter()
        .filter_map(|i| match i {
            MixedPositioned::Word { x, y, w, .. } => Some((*x, *y, *w)),
            _ => None,
        })
        .collect();
    let emb = items.iter().find_map(|i| match i {
        MixedPositioned::Embedded(l) => Some((l.x, l.y)),
        _ => None,
    });
    (words, emb)
}

fn qy43_assert_no_overlap(words: &[(f32, f32, f32)]) {
    for (i, a) in words.iter().enumerate() {
        for b in &words[i + 1..] {
            if (a.1 - b.1).abs() < 0.5 {
                assert!(a.0 + a.2 <= b.0 + 0.5 || b.0 + b.2 <= a.0 + 0.5, "overlap {a:?} {b:?}");
            }
        }
    }
}

#[test]
fn qy43_pre_wrap_newlines_with_inline_block_do_not_overprint() {
    let (words, emb) = qy43_items(
        "<html><body style=\"margin:0\"><div style=\"white-space:pre-wrap;width:400px\">Aaa bbb.\n\nWenn du mehr <span style=\"display:inline-block\">Info</span> ccc</div></body></html>",
    );
    qy43_assert_no_overlap(&words);
    let first_y = words[0].1;
    let last_y = words.last().unwrap().1;
    // Line 1, an empty line, line 3: the last word sits two line heights down.
    let step = (last_y - first_y) / 2.0;
    assert!(step > 5.0, "blank line missing: {first_y} {last_y}");
    let (_, ey) = emb.expect("inline-block");
    assert!(ey > first_y + step, "inline-block on the third block: {ey}");
}

#[test]
fn qy43_pre_wrap_pre_line_pre_with_padded_inline_break_at_newline() {
    for ws in ["pre-wrap", "pre-line", "pre"] {
        let (words, _) = qy43_items(&format!(
            "<html><body style=\"margin:0\"><div style=\"white-space:{ws}\">one two\nthree <span style=\"padding:0 3px\">four</span></div></body></html>"
        ));
        qy43_assert_no_overlap(&words);
        let ys: Vec<f32> = words.iter().map(|w| w.1).collect();
        // A chunk that stays on one line is one merged word box.
        assert!(words.len() >= 2, "{ws}: {ys:?}");
        assert!(ys[1] > ys[0] + 5.0, "{ws}: second line lower {ys:?}");
        assert!((ys[1] - ys[2]).abs() < 0.5 || ys.len() == 2, "{ws}: three/four share a line {ys:?}");
    }
}

#[test]
fn qy43_inline_pre_wrap_span_in_normal_block_breaks_at_newline() {
    // YouTube: `yt-formatted-string{white-space:pre-wrap}` is an inline
    // element in a block whose own white-space is `normal`.
    let (words, emb) = qy43_items(
        "<html><body style=\"margin:0\"><div style=\"width:300px\"><span style=\"white-space:pre-wrap\">Aaa bbb.\n\nWenn du die Einstellung aendern moechtest, schalte den YouTube-Verlauf ein und bestaetige alles, </span><a style=\"display:inline-block\">Info</a></div></body></html>",
    );
    qy43_assert_no_overlap(&words);
    let first_y = words[0].1;
    assert!(words.iter().skip(1).all(|w| w.1 >= first_y + 2.0 * 10.0), "text after the blank line stays below: {words:?}");
    assert!(emb.unwrap().1 > words.last().unwrap().1 - 1.0);
}

#[test]
fn qy43_plain_pre_wrap_blank_line_and_normal_collapse_unchanged() {
    let two = measure_fixture("<html><body style=\"margin:0\"><div style=\"white-space:pre-wrap\">a\n\nb</div></body></html>", 800.0);
    let one = measure_fixture("<html><body style=\"margin:0\"><div style=\"white-space:pre-wrap\">a\nb</div></body></html>", 800.0);
    assert!(two.h > one.h + 5.0, "blank line adds height: {} vs {}", two.h, one.h);
    let (words, _) = qy43_items(
        "<html><body style=\"margin:0\"><div>one\ntwo <span style=\"display:inline-block\">x</span></div></body></html>",
    );
    assert!(words.iter().all(|w| (w.1 - words[0].1).abs() < 0.5), "{words:?}");
}

// ---------- QJ8: document.activeElement / hasFocus ----------

#[test]
fn qj8_active_element_follows_focus_blur_and_removal() {
    crate::js_dom::set_active_element(None);
    let log = run_in_html(
        "<html><body><input id=i><button id=b>x</button><div id=d></div></body></html>",
        "var i=document.getElementById('i'), b=document.getElementById('b');\
         console.log(document.activeElement===document.body, document.hasFocus());\
         i.focus(); console.log(document.activeElement===i);\
         b.focus(); console.log(document.activeElement===b);\
         i.blur(); console.log(document.activeElement===b);\
         b.blur(); console.log(document.activeElement===document.body);\
         i.focus(); i.remove(); console.log(document.activeElement===document.body);\
         var d2=document.implementation.createHTMLDocument('x'); var e=d2.createElement('input'); d2.body.appendChild(e);\
         b.focus(); console.log(d2.activeElement===d2.body, document.activeElement===b);",
    );
    assert_eq!(
        log,
        ["log:true true", "log:true", "log:true", "log:true", "log:true", "log:true", "log:true true"],
        "{log:?}"
    );
}

// ---------- QY44c: document.styleSheets, style/link .sheet, insertRule reaches the cascade ----------

const QY44C_HTML: &str = "<html><head><style id=s>p{color:#0000ff}</style><link rel=stylesheet href=x.css id=l></head><body><p id=p>x</p></body></html>";

#[test]
fn qy44c_insert_rule_into_empty_style_changes_colour() {
    let html = "<html><head><style id=s></style></head><body><p id=p>x</p></body></html>";
    let (c, l, _) = qsd4_color(html, "var sh=document.getElementById('s').sheet; sh.insertRule('p{color:#ff0000}',0); console.log(sh.cssRules.length, getComputedStyle(document.getElementById('p')).color);");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)), "{l:?}");
    assert_eq!(l[0], "log:1 rgb(255, 0, 0)");
}

#[test]
fn qy44c_delete_rule_and_text_unchanged() {
    let (c, l, _) = qsd4_color(QY44C_HTML, "var e=document.getElementById('s'),sh=e.sheet; sh.insertRule('p{color:#ff0000}',1); console.log(sh.cssRules.length, e.textContent); sh.deleteRule(1);");
    assert_eq!(c, Some(TextColor::rgb(0, 0, 255)), "{l:?}");
    assert_eq!(l[0], "log:2 p{color:#0000ff}");
    let (c, _, _) = qsd4_color(QY44C_HTML, "var sh=document.getElementById('s').sheet; sh.deleteRule(0);");
    assert_ne!(c, Some(TextColor::rgb(0, 0, 255)));
    let (c, l, _) = qsd4_color(QY44C_HTML, "var sh=document.getElementById('s').sheet; sh.insertRule('p{color:#00ff00}'); sh.removeRule(1); sh.addRule('p','color:#ff0000'); console.log(sh.cssRules.length);");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)), "{l:?}");
}

#[test]
fn qy44c_text_content_change_replaces_sheet() {
    let (c, l, _) = qsd4_color(
        QY44C_HTML,
        "var e=document.getElementById('s'),a=e.sheet; a.insertRule('p{color:#ff0000}',1); e.textContent='p{color:#00ff00}'; var b=e.sheet; console.log(a===b, b.cssRules.length, b.cssRules[0].selectorText);",
    );
    assert_eq!(c, Some(TextColor::rgb(0, 255, 0)), "{l:?}");
    assert_eq!(l[0], "log:false 1 p");
}

#[test]
fn qy44c_style_sheets_list_and_owner() {
    let (_e, _d, l) = events_engine(
        QY44C_HTML,
        "var ss=document.styleSheets; var s=document.getElementById('s'),k=document.getElementById('l');\
         console.log(ss.length, ss[0].ownerNode===s, ss.item(1).ownerNode===k, ss[1].cssRules.length, s.sheet===ss[0], ss[0] instanceof CSSStyleSheet, ss.item(5));\
         var d=document.createElement('style'); console.log(d.sheet); document.head.appendChild(d); console.log(document.styleSheets.length, d.sheet.cssRules.length);\
         var c=new CSSStyleSheet(); document.adoptedStyleSheets=[c]; console.log(document.styleSheets.length, ss[0].disabled, ss[0].type);",
    );
    let l = l.borrow();
    assert_eq!(l[0], "log:2 true true 0 true true null", "{l:?}");
    assert_eq!(l[1], "log:null");
    assert_eq!(l[2], "log:3 0");
    assert_eq!(l[3], "log:3 false text/css");
}

#[test]
fn qy44c_disabled_sheet_and_rule_style_mutation() {
    let (c, _, _) = qsd4_color(QY44C_HTML, "var sh=document.getElementById('s').sheet; sh.disabled=true;");
    assert_ne!(c, Some(TextColor::rgb(0, 0, 255)));
    let (c, l, _) = qsd4_color(QY44C_HTML, "var sh=document.getElementById('s').sheet; sh.cssRules[0].style.color='#ff0000'; console.log(sh.cssRules[0].cssText);");
    assert_eq!(c, Some(TextColor::rgb(255, 0, 0)), "{l:?}");
}

// ---------- QJ9: window.open / close / alert / confirm / prompt ----------

#[test]
fn qj9_window_open_close_dialogs_are_inert_and_log_once() {
    let log = run_in_html(
        "<html><body></body></html>",
        "console.log(typeof window.open, typeof window.close, typeof alert, typeof confirm, typeof prompt);\
         console.log(window.open('/x', '_blank', 'noopener'));\
         console.log(window.close());\
         console.log(alert('hi'), confirm('sure?'), prompt('name?'));\
         console.log('still running');",
    );
    assert_eq!(log[0], "log:function function function function function", "{log:?}");
    assert_eq!(log[1], "error:scratchy: js: not implemented: window.open \"/x target=_blank features=noopener\"", "{log:?}");
    assert_eq!(log[2], "log:null", "{log:?}");
    assert_eq!(log[3], "error:scratchy: js: not implemented: window.close", "{log:?}");
    assert_eq!(log[4], "log:undefined", "{log:?}");
    assert!(log[5].contains("window.alert \"hi\""), "{log:?}");
    assert!(log[6].contains("window.confirm"), "{log:?}");
    assert!(log[7].contains("window.prompt"), "{log:?}");
    assert_eq!(log[8], "log:undefined false null", "{log:?}");
    assert_eq!(log[9], "log:still running", "{log:?}");
    assert_eq!(log.len(), 10, "{log:?}");
}

// ---- QY45: namespaced attributes (setAttributeNS family), Attr nodes, prefix/localName.
#[test]
fn qy45_xlink_href_round_trip_and_use() {
    let log = run_in_html(
        "<html><body><svg id=s xmlns='http://www.w3.org/2000/svg' viewBox='0 0 4 4'><use id=u xlink:href='#a'/></svg></body></html>",
        "var X='http://www.w3.org/1999/xlink', S='http://www.w3.org/2000/svg', r=[];\
         var u=document.getElementById('u');\
         r.push(u.getAttribute('xlink:href'), u.getAttributeNS(X,'href'), u.hasAttributeNS(X,'href'), u.getAttribute('href'));\
         var n=document.createElementNS(S,'use'); n.setAttributeNS(X,'xlink:href','#b');\
         r.push(n.getAttribute('xlink:href'), n.getAttributeNS(X,'href'), n.getAttribute('href'), n.hasAttribute('xlink:href'));\
         r.push(n.attributes[0].name, n.attributes[0].localName, n.attributes[0].prefix, n.attributes[0].namespaceURI);\
         n.setAttributeNS(X,'xlink:href','#c'); r.push(n.attributes.length, n.getAttribute('xlink:href'));\
         var sv=document.getElementById('s'); r.push(sv.getAttribute('viewBox'), sv.getAttribute('viewbox'));\
         sv.setAttribute('viewBox','0 0 8 8'); r.push(sv.getAttribute('viewBox'), sv.attributes.length);\
         r.push(document.createElement('div').appendChild(n).outerHTML);\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:#a|#a|true||#b|#b||true|xlink:href|href|xlink|http://www.w3.org/1999/xlink|1|#c|0 0 4 4||0 0 8 8|3|<use xlink:href=\"#c\"></use>"],
        "{log:?}"
    );
}

#[test]
fn qy45_remove_and_order_and_case() {
    let log = run_in_html(
        "<html><body><div id=d a=1></div></body></html>",
        "var X='http://www.w3.org/1999/xlink', r=[], d=document.getElementById('d');\
         d.setAttributeNS(X,'xlink:b','2'); d.setAttributeNS(null,'c','3'); d.setAttributeNS('', 'e', '4');\
         r.push(d.getAttributeNames().join(','), d.getAttributeNS(null,'c'), d.getAttributeNS(X,'c'), d.getAttributeNS(X,'nope'));\
         d.removeAttributeNS(X,'b'); r.push(d.getAttributeNames().join(','), d.hasAttributeNS(X,'b'));\
         d.removeAttributeNS(null,'c'); r.push(d.getAttributeNames().join(','));\
         d.setAttribute('MixedCase','v'); r.push(d.getAttribute('mixedcase'), d.getAttribute('MIXEDCASE'), d.getAttributeNames().join(','));\
         d.setAttributeNS(null,'CamelNS','w'); r.push(d.getAttributeNS(null,'CamelNS'), d.getAttribute('camelns'), d.getAttribute('CamelNS'));\
         var s=document.createElementNS('http://www.w3.org/2000/svg','svg'); s.setAttribute('viewBox','1 2 3 4'); s.setAttribute('ViewBox','x');\
         r.push(s.getAttribute('viewBox'), s.getAttribute('ViewBox'), s.attributes.length);\
         d.setAttribute('x','1'); d.setAttribute('a','one'); r.push(d.getAttributeNames().join(','), d.getAttribute('a'));\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:id,a,xlink:b,c,e|3|||id,a,c,e|false|id,a,e|v|v|id,a,e,mixedcase|w|||1 2 3 4|x|2|id,a,e,mixedcase,CamelNS,x|one"],
        "{log:?}"
    );
}

#[test]
fn qy45_validation_errors() {
    let log = run_in_html(
        "<html><body><div id=d></div></body></html>",
        "var d=document.getElementById('d'), r=[];\
         var t=function(f){ try { f(); r.push('ok'); } catch(e){ r.push(e.name); } };\
         var X='http://www.w3.org/XML/1998/namespace', N='http://www.w3.org/2000/xmlns/';\
         t(function(){ d.setAttributeNS(null,'a:b','v'); });\
         t(function(){ d.setAttributeNS('urn:x','a b','v'); });\
         t(function(){ d.setAttributeNS('urn:x','a:b:c','v'); });\
         t(function(){ d.setAttributeNS('urn:x',':b','v'); });\
         t(function(){ d.setAttributeNS('urn:x','a:','v'); });\
         t(function(){ d.setAttributeNS('urn:x','xml:lang','v'); });\
         t(function(){ d.setAttributeNS(X,'xml:lang','en'); });\
         t(function(){ d.setAttributeNS('urn:x','xmlns','v'); });\
         t(function(){ d.setAttributeNS('urn:x','xmlns:a','v'); });\
         t(function(){ d.setAttributeNS(N,'xmlns:a','urn:a'); });\
         t(function(){ d.setAttributeNS(N,'xmlns','urn:d'); });\
         t(function(){ d.setAttributeNS(N,'foo:bar','v'); });\
         t(function(){ d.setAttributeNS('urn:x','p:q','v'); });\
         t(function(){ document.createAttributeNS('urn:x','a b'); });\
         t(function(){ document.createAttributeNS(null,'p:q'); });\
         t(function(){ document.createAttribute('1a b'); });\
         t(function(){ d.setAttribute('a b','v'); });\
         r.push(d.getAttributeNS(X,'lang'), d.getAttribute('xml:lang'), d.getAttribute('xmlns:a'), d.getAttribute('xmlns'), d.getAttributeNames().join(','));\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:NamespaceError|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError|InvalidCharacterError|NamespaceError|ok|NamespaceError|NamespaceError|ok|ok|NamespaceError|ok|InvalidCharacterError|NamespaceError|InvalidCharacterError|InvalidCharacterError|en|en|urn:a|urn:d|id,xml:lang,xmlns:a,xmlns,p:q"],
        "{log:?}"
    );
}

#[test]
fn qy45_attr_nodes() {
    let log = run_in_html(
        "<html><body><div id=d a=1></div><p id=p></p></body></html>",
        "var X='http://www.w3.org/1999/xlink', r=[], d=document.getElementById('d'), p=document.getElementById('p');\
         var a=document.createAttributeNS(X,'xlink:href'); a.value='#z';\
         r.push(a.name, a.localName, a.prefix, a.namespaceURI, a.ownerElement, a.value);\
         r.push(d.setAttributeNodeNS(a), a.ownerElement===d, d.getAttribute('xlink:href'));\
         r.push(d.getAttributeNodeNS(X,'href').value, d.getAttributeNodeNS(X,'zip'), d.getAttributeNode('xlink:href').namespaceURI);\
         var b=document.createAttributeNS(X,'xlink:href'); b.value='#y';\
         var old=d.setAttributeNode(b); r.push(old.value, old.ownerElement, d.getAttribute('xlink:href'));\
         var inUse; try { p.setAttributeNode(b); } catch(e){ inUse=e.name; } r.push(inUse);\
         var rem=d.removeAttributeNode(b); r.push(rem.value, rem.ownerElement, d.hasAttribute('xlink:href'), rem===b);\
         var nf; try { d.removeAttributeNode(b); } catch(e){ nf=e.name; } r.push(nf);\
         var c=document.createAttribute('Title'); c.value='T'; d.setAttributeNode(c); r.push(d.getAttribute('title'), c.name);\
         r.push(d.attributes.getNamedItemNS(null,'title').value, d.attributes.removeNamedItem('title').value, d.hasAttribute('title'));\
         var a2=document.createAttributeNS(X,'xlink:href'); a2.value='#w'; d.attributes.setNamedItemNS(a2); r.push(d.attributes.length, d.attributes.getNamedItemNS(X,'href').value);\
         d.toggleAttribute('hidden'); r.push(d.hasAttribute('hidden')); d.toggleAttribute('hidden'); r.push(d.hasAttribute('hidden'));\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:xlink:href|href|xlink|http://www.w3.org/1999/xlink||#z||true|#z|#z||http://www.w3.org/1999/xlink|#z||#y|InUseAttributeError|#y||false|true|NotFoundError|T|title|T|T|false|3|#w|true|false"],
        "{log:?}"
    );
}

#[test]
fn qy45_element_prefix_local_name_and_xml_doc() {
    let log = run_in_html(
        "<html><body><div id=d></div></body></html>",
        "var S='http://www.w3.org/2000/svg', r=[];\
         var e=document.createElementNS(S,'svg:linearGradient');\
         r.push(e.tagName, e.localName, e.prefix, e.namespaceURI);\
         var f=document.createElementNS(S,'rect'); r.push(f.tagName, f.localName, f.prefix);\
         var h=document.createElement('DIV'); r.push(h.tagName, h.localName, h.prefix, h.namespaceURI);\
         var x=new DOMParser().parseFromString('<r xmlns=\"urn:a\" xmlns:q=\"urn:q\"><c q:k=\"1\" z=\"2\"/></r>','application/xml');\
         var c=x.documentElement.firstElementChild;\
         r.push(c.getAttribute('q:k'), c.getAttributeNS('urn:q','k'), c.getAttributeNS(null,'z'), c.attributes[0].prefix, c.attributes[0].localName, c.attributes[0].namespaceURI);\
         r.push(x.documentElement.getAttributeNames().join(','), x.documentElement.getAttributeNS('http://www.w3.org/2000/xmlns/','q'));\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:svg:linearGradient|linearGradient|svg|http://www.w3.org/2000/svg|rect|rect||DIV|div||http://www.w3.org/1999/xhtml|1|1|2|q|k|urn:q|xmlns,xmlns:q|urn:q"],
        "{log:?}"
    );
}

// ---------- QY52: getComputedStyle reuses styles across unrelated writes ----------

#[test]
fn qy52_get_computed_style_reflects_every_kind_of_write() {
    let (_e, _d, log) = events_engine(
        "<html><head><style>.big{font-size:30px} .red{color:rgb(255,0,0)} .a + .b{margin-left:9px} \
         .box::before{content:'x';color:rgb(0,0,255)} p:empty{padding-left:4px} .anc .kid{display:block}</style></head>\
         <body><div id=anc><span id=kid class=kid>k</span></div><div id=s1 class=a></div><div id=s2 class=b></div>\
         <div class=box id=box></div><p id=e>t</p><div id=par><i id=ch>c</i></div></body></html>",
        "var ch=document.getElementById('ch'); var g=function(id,p){return getComputedStyle(document.getElementById(id)).getPropertyValue(p)};\
         var o=[];\
         o.push(g('kid','display'));\
         document.getElementById('anc').className='anc'; o.push(g('kid','display'));\
         document.getElementById('anc').className=''; o.push(g('kid','display'));\
         document.getElementById('kid').style.color='rgb(1,2,3)'; o.push(g('kid','color'));\
         document.getElementById('par').className='red'; o.push(g('ch','color'));\
         document.getElementById('par').className='big'; o.push(g('ch','font-size'));\
         o.push(g('s2','margin-left'));\
         document.getElementById('s1').className='x'; o.push(g('s2','margin-left'));\
         document.getElementById('s1').className='a'; o.push(g('s2','margin-left'));\
         document.getElementById('s2').parentNode.insertBefore(document.createElement('div'), document.getElementById('s2')); o.push(g('s2','margin-left'));\
         o.push(getComputedStyle(document.getElementById('box'),'::before').color);\
         var st=document.createElement('style'); st.textContent='.box::before{color:rgb(0,128,0)} .big{font-size:11px}'; document.head.appendChild(st);\
         o.push(getComputedStyle(document.getElementById('box'),'::before').color); o.push(g('ch','font-size'));\
         st.sheet.insertRule('#ch{font-size:22px}', st.sheet.cssRules.length); o.push(g('ch','font-size'));\
         o.push(g('e','padding-left')); document.getElementById('e').textContent=''; o.push(g('e','padding-left'));\
         document.body.removeChild(document.getElementById('par')); o.push(JSON.stringify(getComputedStyle(ch).getPropertyValue('font-size')));\
         console.log(o.join('|'));",
    );
    let got = log.borrow().join("\n");
    assert!(
        got.contains("inline|block|inline|rgb(1, 2, 3)|rgb(255, 0, 0)|30px|9px|0px|9px|0px|rgb(0, 0, 255)|rgb(0, 128, 0)|11px|22px|0px|4px|\"\""),
        "{got}"
    );
}

#[test]
fn qy52_unrelated_writes_keep_cached_styles_and_stylesheets_invalidate() {
    let (e, _d, _log) = events_engine(
        "<html><head><style>.t{color:rgb(9,9,9)}</style></head><body><p id=u>x</p><div><div><div class=t id=t>y</div></div></div></body></html>",
        "var t=document.getElementById('t'), u=document.getElementById('u'); window.rd=function(){return getComputedStyle(t).color};",
    );
    let ev = |code: &str| -> String { e.eval_value("q", code).expect("eval") };
    assert_eq!(ev("rd()"), "rgb(9, 9, 9)");
    let (c0, p0) = (crate::js_style::cascade_count(), crate::rules_cache::PIECES_PARSED.with(|c| c.get() as u64));
    // Attribute and text writes elsewhere: every read is answered from the cache.
    assert_eq!(ev("var r; for (var i=0;i<5;i++){ u.setAttribute('data-x',''+i); u.textContent='n'+i; r=rd(); } r"), "rgb(9, 9, 9)");
    assert_eq!(crate::js_style::cascade_count(), c0, "unrelated writes must not re-cascade");
    assert_eq!(crate::rules_cache::PIECES_PARSED.with(|c| c.get() as u64), p0, "unrelated writes must not re-parse");
    // A write on the element itself re-cascades just that element.
    assert_eq!(ev("t.className=''; rd()"), "rgb(0, 0, 0)");
    let c1 = crate::js_style::cascade_count();
    assert!(c1 > c0 && c1 - c0 <= 2, "{c0} -> {c1}");
    // A new stylesheet invalidates everything and is re-parsed (only the new piece).
    assert_eq!(ev("var st=document.createElement('style'); st.textContent='.t{color:rgb(1,1,1)}'; document.head.appendChild(st); t.className='t'; rd()"), "rgb(1, 1, 1)");
    assert_eq!(crate::rules_cache::PIECES_PARSED.with(|c| c.get() as u64) - p0, 1, "only the new <style> is parsed");
    assert_eq!(ev("st.textContent='.t{color:rgb(2,2,2)}'; rd()"), "rgb(2, 2, 2)");
    assert_eq!(crate::rules_cache::PIECES_PARSED.with(|c| c.get() as u64) - p0, 2);
    // QMEM3e: the page's rules are the layout cache's, StyleCache parses nothing of its own.
    assert_eq!(crate::js_style::parse_count(), 0, "StyleCache must not parse the page's CSS itself");
}

// ---------- QY53c: only connected, value-changing writes invalidate geometry ----------

/// Runs `code` and returns (the log, how many forced layouts it caused).
fn qy53c_run(html: &str, code: &str) -> (Vec<String>, usize) {
    crate::js_geom::GEOM_LAYOUTS.with(|c| c.set(0));
    let log = run_in_html(html, code);
    (log, crate::js_geom::GEOM_LAYOUTS.with(|c| c.get()))
}

const QY53C_PAGE: &str = "<html><body style='margin:0'><div id=a style='width:100px;height:20px'>hello</div></body></html>";

#[test]
fn qy53c_same_value_attribute_write_does_not_force_a_new_layout() {
    let (log, n) = qy53c_run(
        QY53C_PAGE,
        "var a = document.getElementById('a'); a.offsetWidth;
         a.setAttribute('lang', '1'); a.offsetWidth;
         for (var i = 0; i < 5; i++) { a.setAttribute('lang', '1'); a.offsetWidth; }
         a.setAttribute('style', 'width:100px;height:20px'); a.offsetWidth;
         console.log('w', a.offsetWidth);",
    );
    assert!(log.contains(&"log:w 100".to_string()), "{log:?}");
    // 1: first read; 2: after the new attribute. Nothing after the no-op writes.
    assert_eq!(n, 2, "{log:?}");
}

#[test]
fn qy53c_real_change_on_a_connected_node_still_relayouts() {
    let (log, n) = qy53c_run(
        QY53C_PAGE,
        "var a = document.getElementById('a'); console.log('w1', a.offsetWidth);
         a.setAttribute('style', 'width:150px;height:20px'); console.log('w2', a.offsetWidth);
         a.removeAttribute('style'); console.log('w3', a.offsetWidth > 150);
         a.style.width = '60px'; console.log('w4', a.offsetWidth);",
    );
    for l in ["log:w1 100", "log:w2 150", "log:w3 true", "log:w4 60"] {
        assert!(log.contains(&l.to_string()), "{l} in {log:?}");
    }
    assert_eq!(n, 4, "{log:?}");
}

#[test]
fn qy53c_writes_to_a_detached_subtree_do_not_relayout_until_it_is_attached() {
    let (log, n) = qy53c_run(
        QY53C_PAGE,
        "var a = document.getElementById('a'); a.offsetWidth;
         var d = document.createElement('div'); d.setAttribute('style', 'width:40px;height:30px');
         var s = document.createElement('span'); s.textContent = 'x'; d.appendChild(s); d.innerHTML = '<b>y</b>';
         d.className = 'k'; d.removeAttribute('class');
         a.offsetWidth;
         document.body.appendChild(d);
         console.log('h', d.offsetHeight, 'w', d.offsetWidth);",
    );
    assert!(log.contains(&"log:h 30 w 40".to_string()), "attached box has its size: {log:?}");
    // 1: first read, 2: after the append. The detached writes caused none.
    assert_eq!(n, 2, "{log:?}");
}

#[test]
fn qy53c_removing_a_connected_node_relayouts_and_detached_removal_does_not() {
    let (log, n) = qy53c_run(
        "<html><body style='margin:0'><div id=a style='height:20px'>a</div><div id=b style='height:30px'>b</div></body></html>",
        "var a = document.getElementById('a'), b = document.getElementById('b');
         console.log('y1', b.getBoundingClientRect().top);
         var x = document.createElement('i'); var y = document.createElement('i'); x.appendChild(y); x.removeChild(y);
         console.log('y2', b.getBoundingClientRect().top);
         a.parentNode.removeChild(a);
         console.log('y3', b.getBoundingClientRect().top);",
    );
    for l in ["log:y1 20", "log:y2 20", "log:y3 0"] {
        assert!(log.contains(&l.to_string()), "{l} in {log:?}");
    }
    assert_eq!(n, 2, "{log:?}");
}

#[test]
fn qy53c_text_node_write_of_the_same_text_does_not_relayout() {
    let (log, n) = qy53c_run(
        "<html><body style='margin:0'><span id=s style='font-size:20px;display:inline-block'>hello</span></body></html>",
        "var s = document.getElementById('s'), t = s.firstChild; var w = s.offsetWidth;
         t.data = 'hello'; s.offsetWidth; s.textContent = 'hello'; s.offsetWidth;
         console.log('same', s.offsetWidth === w);
         s.firstChild.data = 'hello hello'; console.log('wider', s.offsetWidth > w);",
    );
    assert!(log.contains(&"log:same true".to_string()) && log.contains(&"log:wider true".to_string()), "{log:?}");
    assert_eq!(n, 2, "{log:?}");
}

#[test]
fn qy53c_stylesheet_change_and_class_change_relayout() {
    let (log, n) = qy53c_run(
        "<html><head><style>.w { width: 50px } .v { width: 70px }</style></head><body style='margin:0'><div id=a class=w style='height:10px'></div></body></html>",
        "var a = document.getElementById('a'); console.log('w1', a.offsetWidth);
         a.className = 'v'; console.log('w2', a.offsetWidth);
         var st = document.createElement('style'); st.textContent = '.v { width: 90px }'; document.head.appendChild(st);
         console.log('w3', a.offsetWidth);
         st.textContent = '.v { width: 95px }'; console.log('w4', a.offsetWidth);",
    );
    for l in ["log:w1 50", "log:w2 70", "log:w3 90", "log:w4 95"] {
        assert!(log.contains(&l.to_string()), "{l} in {log:?}");
    }
    assert_eq!(n, 4, "{log:?}");
}

// ---------- QY53h: GeomCache keyed on the CSS version, no page_css() on a hit ----------

#[test]
fn qy53h_geometry_hit_does_not_build_page_css_and_stylesheet_changes_invalidate() {
    let builds = || crate::js_style::PAGE_CSS_BUILDS.with(|c| c.get());
    let layouts = || crate::js_geom::GEOM_LAYOUTS.with(|c| c.get());
    crate::js_style::set_js_extra_css("");
    crate::js_style::set_js_adopted_css("");
    let e = events_engine(
        "<html><head><style>#a{width:10px}</style></head><body style='margin:0'><div id=a style='height:5px'>x</div></body></html>",
        "var a=document.getElementById('a'); window.w=function(){return a.offsetWidth};",
    )
    .0;
    let ev = |code: &str| -> String { e.eval_value("q", code).expect("eval") };
    assert_eq!(ev("''+w()"), "10");
    let (b0, l0) = (builds(), layouts());
    assert_eq!(ev("var r=0; for (var i=0;i<20;i++) r+=w(); ''+r"), "200");
    assert_eq!(builds(), b0, "a cache hit must not build page_css");
    assert_eq!(layouts(), l0);
    crate::js_style::set_js_extra_css("#a{width:20px}");
    assert_eq!(ev("''+w()"), "20");
    assert!(layouts() > l0);
    crate::js_style::set_js_adopted_css("#a{width:30px}");
    assert_eq!(ev("''+w()"), "30");
    assert_eq!(ev("var s=document.createElement('style'); s.textContent='#a{width:40px !important}'; document.head.appendChild(s); ''+w()"), "40");
    assert_eq!(ev("s.sheet.insertRule('#a{width:50px !important}', 1); ''+w()"), "50");
    crate::js_style::set_js_extra_css("");
    crate::js_style::set_js_adopted_css("");
}

// ---------- QY53l: style.<prop> = <same value> is a no-op (CSSOM updates in place) ----------

#[test]
fn qy53l_style_property_rewrite_keeps_declaration_order() {
    // YouTube's progress bars set `left` and `transform` in turn every tick: the old
    // remove-and-append setter rotated the declarations on every write.
    let log = run_in_html(
        QY53C_PAGE,
        "var a = document.getElementById('a');
         a.setAttribute('style', 'left: 0px; transform: scaleX(0);');
         a.style.left = '0px'; console.log('1', a.getAttribute('style'));
         a.style.transform = 'scaleX(0)'; console.log('2', a.getAttribute('style'));
         a.style.left = '5px'; console.log('3', a.getAttribute('style'));
         a.style.setProperty('--k', '1'); console.log('4', a.getAttribute('style'));
         a.style.left = ''; console.log('5', a.getAttribute('style'));
         a.style.removeProperty('transform'); a.style.removeProperty('--k'); console.log('6', a.getAttribute('style'));",
    );
    for l in [
        "log:1 left: 0px; transform: scaleX(0);",
        "log:2 left: 0px; transform: scaleX(0);",
        "log:3 left: 5px; transform: scaleX(0);",
        "log:4 left: 5px; transform: scaleX(0); --k: 1;",
        "log:5 transform: scaleX(0); --k: 1;",
        "log:6 null",
    ] {
        assert!(log.contains(&l.to_string()), "{l} in {log:?}");
    }
}

#[test]
fn qy53l_same_value_style_property_write_does_not_force_a_layout() {
    let (log, n) = qy53c_run(
        QY53C_PAGE,
        "var a = document.getElementById('a'); a.style.left = '0px'; a.style.transform = 'scaleX(0)';
         a.offsetWidth;
         for (var i = 0; i < 5; i++) { a.style.left = '0px'; a.offsetWidth; a.style.transform = 'scaleX(0)'; a.offsetWidth;
           a.style.setProperty('width', '100px'); a.clientWidth; }
         console.log('w', a.offsetWidth);",
    );
    assert!(log.contains(&"log:w 100".to_string()), "{log:?}");
    // Only the first read lays out: every later write sets a value already there.
    assert_eq!(n, 1, "{log:?}");
}

#[test]
fn qy53l_style_property_write_that_changes_a_value_still_relayouts() {
    let (log, n) = qy53c_run(
        QY53C_PAGE,
        "var a = document.getElementById('a'); a.style.left = '0px'; console.log('w1', a.offsetWidth);
         a.style.width = '70px'; console.log('w2', a.offsetWidth);
         a.style.width = '70px'; console.log('w3', a.offsetWidth);
         a.style.width = '80px'; console.log('w4', a.offsetWidth);",
    );
    for l in ["log:w1 100", "log:w2 70", "log:w3 70", "log:w4 80"] {
        assert!(log.contains(&l.to_string()), "{l} in {log:?}");
    }
    // w1, w2, w4: the same-value write before w3 forces none.
    assert_eq!(n, 3, "{log:?}");
}

// ---------- QY53l2: writes inside a `display: none` subtree force no layout ----------

const QY53L2_PAGE: &str = "<html><head><style>.off{display:none} .e:empty{display:none}</style></head>\
<body style='margin:0'><div id=a style='width:100px;height:20px'>a</div>\
<div id=wrap><div id=h class=off><div id=in><span id=s>x</span></div></div></div>\
<div id=e class=e></div><div id=b style='height:30px'>b</div></body></html>";

#[test]
fn qy53l2_writes_inside_a_hidden_subtree_force_no_layout() {
    let (log, n) = qy53c_run(
        QY53L2_PAGE,
        "var a = document.getElementById('a'), h = document.getElementById('h'), inn = document.getElementById('in'),
             s = document.getElementById('s'), b = document.getElementById('b');
         var y0 = b.getBoundingClientRect().top;
         inn.setAttribute('class', 'q'); a.offsetWidth;
         s.style.width = '50px'; s.className = 'k'; s.id = 's2'; a.offsetWidth;
         s.textContent = 'longer text'; s.firstChild.data = 'more'; a.offsetWidth;
         var p = document.createElement('p'); p.textContent = 'new'; inn.appendChild(p); a.offsetWidth;
         inn.insertBefore(document.createElement('svg'), p); inn.removeChild(p); a.offsetWidth;
         inn.innerHTML = '<i>z</i><b>w</b>'; a.offsetWidth;
         var q = document.createElement('div'); h.appendChild(q); h.removeChild(q); a.offsetWidth;
         var j = document.createElement('script'); j.type = 'application/json'; document.head.appendChild(j); a.offsetWidth;
         console.log('a', a.offsetWidth, 'same', b.getBoundingClientRect().top === y0);
         console.log('hidden', s.offsetWidth, inn.offsetHeight, inn.offsetParent, inn.getBoundingClientRect().width, h.clientWidth);",
    );
    assert!(log.contains(&"log:a 100 same true".to_string()), "{log:?}");
    assert!(log.contains(&"log:hidden 0 0 null 0 0".to_string()), "{log:?}");
    assert_eq!(n, 1, "only the first read lays out: {log:?}");
}

#[test]
fn qy53l2_unhiding_the_root_by_class_or_style_relayouts() {
    for unhide in ["h.className = ''", "h.style.display = 'block'", "h.removeAttribute('class')"] {
        let (log, n) = qy53c_run(
            QY53L2_PAGE,
            &format!(
                "var h = document.getElementById('h'), inn = document.getElementById('in'), b = document.getElementById('b');
                 var y0 = b.getBoundingClientRect().top;
                 inn.firstChild.textContent = 'shown'; b.offsetWidth;
                 {unhide};
                 var y1 = b.getBoundingClientRect().top;
                 console.log('moved', y1 > y0, 'w', inn.offsetWidth > 0, inn.firstChild.offsetWidth > 0, inn.getBoundingClientRect().top === y0);"
            ),
        );
        assert!(log.contains(&"log:moved true w true true true".to_string()), "{unhide}: {log:?}");
        assert_eq!(n, 2, "{unhide}: {log:?}");
    }
}

#[test]
fn qy53l2_moving_a_node_out_of_the_hidden_subtree_relayouts() {
    let (log, n) = qy53c_run(
        QY53L2_PAGE,
        "var s = document.getElementById('s'), b = document.getElementById('b');
         var y0 = b.getBoundingClientRect().top;
         var d = document.createElement('div'); d.style.height = '15px'; document.getElementById('in').appendChild(d);
         console.log('still', b.getBoundingClientRect().top === y0);
         document.body.insertBefore(d, b);
         console.log('dy', b.getBoundingClientRect().top - y0, 'dh', d.offsetHeight);
         document.body.appendChild(s);
         console.log('sw', s.offsetWidth > 0);",
    );
    for l in ["log:still true", "log:dy 15 dh 15", "log:sw true"] {
        assert!(log.contains(&l.to_string()), "{l} in {log:?}");
    }
    assert_eq!(n, 3, "{log:?}");
}

#[test]
fn qy53l2_write_on_an_ancestor_of_the_hidden_root_relayouts() {
    let (log, n) = qy53c_run(
        "<html><head><style>.off{display:none} .shown .off{display:block}</style></head><body style='margin:0'>\
         <div id=wrap><div id=h class=off><div id=in style='height:12px'>x</div></div></div><div id=b>b</div></body></html>",
        "var w = document.getElementById('wrap'), b = document.getElementById('b'), inn = document.getElementById('in');
         var y0 = b.getBoundingClientRect().top;
         w.setAttribute('lang', '1'); b.offsetWidth;
         w.className = 'shown';
         console.log('dy', b.getBoundingClientRect().top - y0, 'h', inn.offsetHeight);",
    );
    assert!(log.contains(&"log:dy 12 h 12".to_string()), "{log:?}");
    assert_eq!(n, 3, "{log:?}");
}

#[test]
fn qy53l2_empty_root_style_and_select_options_stay_conservative() {
    // `.e:empty{display:none}`: a child for the hidden root itself un-hides it.
    let (log, n) = qy53c_run(
        QY53L2_PAGE,
        "var e = document.getElementById('e'), b = document.getElementById('b'), a = document.getElementById('a');
         var y0 = b.getBoundingClientRect().top;
         var c = document.createElement('div'); c.style.height = '9px'; e.appendChild(c);
         console.log('dy', b.getBoundingClientRect().top - y0);
         var st = document.createElement('style'); st.textContent = '#a{width:77px !important}'; document.getElementById('in').appendChild(st);
         console.log('a', a.offsetWidth);",
    );
    assert!(log.contains(&"log:dy 9".to_string()) && log.contains(&"log:a 77".to_string()), "{log:?}");
    assert_eq!(n, 3, "{log:?}");
    // A hidden `<option>`'s text is still read by its `<select>` (not through the box
    // build): writes there keep forcing a layout. (`:has()` rules would disable the skip
    // entirely, but scratchy's selector parser does not accept `:has()` yet.)
    let (log, n) = qy53c_run(
        "<html><body style='margin:0'><select id=sel><option>a</option><option id=o hidden><b>b</b></option></select></body></html>",
        "var sel = document.getElementById('sel'), o = document.getElementById('o');
         sel.offsetWidth; o.textContent = 'a much longer option label'; sel.offsetWidth;
         o.setAttribute('label', 'x'); console.log('w', sel.offsetWidth > 0);",
    );
    assert!(log.contains(&"log:w true".to_string()), "{log:?}");
    assert_eq!(n, 3, "{log:?}");
}

// ---------- QY53o3a: the JS-thread shape memo is swept per layout, not cleared at a cap ----------

#[test]
fn qy53o3a_big_layout_keeps_its_memo_across_forced_layouts() {
    let mut html = String::from("<html><body style='margin:0'><a id=t href='#'>link</a>");
    for i in 0..4200 {
        html.push_str(&format!("<p style='margin:0'>run number {i}</p>"));
    }
    html.push_str("</body></html>");
    // QFLK1: the layout is time-boxed (500 ms passes, 10 s cap); on a loaded machine it stopped half way.
    crate::js_geom::TEST_NO_LAYOUT_LIMITS.with(|c| c.set(true));
    let doc = parse_html(&html);
    let t = by_id(&doc, "t");
    crate::layout::SHAPE_MEMO.with(|m| m.borrow_mut().clear());
    let puts = || crate::layout::SHAPE_MEMO_PUTS.with(|c| c.get());
    let len = || crate::layout::SHAPE_MEMO.with(|m| m.borrow().len());
    let mut g = crate::js_geom::GeomCache::default();
    g.rect_of(&doc, 1, &t, 800.0, 600.0);
    assert!(len() > 4096, "one layout stores more than the old cap: {}", len());
    let p0 = puts();
    g.rect_of(&doc, 2, &t, 800.0, 600.0);
    g.rect_of(&doc, 3, &t, 800.0, 600.0);
    assert_eq!(puts(), p0, "refreshes after a write shape nothing");
    assert!(len() > 4096, "{}", len());
    // A viewport change clears the memo (old widths) and the layout refills it.
    g.rect_of(&doc, 4, &t, 500.0, 600.0);
    assert!(puts() > p0, "a resize reshapes");
}

// ---------- QY53o1: writes of attributes nothing reads force no layout ----------

fn qy53o1_page(css: &str) -> String {
    format!(
        "<html><head><style>{css}</style></head><body style='margin:0'>\
<a id=a href='#x'>link</a><abbr id=ab>ab</abbr><div id=d style='width:100px;height:20px'>d</div></body></html>"
    )
}

/// Reads geometry, runs `writes`, reads again; returns the number of layouts.
fn qy53o1_layouts(css: &str, writes: &str) -> usize {
    let (log, n) = qy53c_run(
        &qy53o1_page(css),
        &format!(
            "var a = document.getElementById('a'), ab = document.getElementById('ab'), d = document.getElementById('d');
             d.offsetWidth; {writes}; console.log('w', d.offsetWidth);"
        ),
    );
    assert!(log.contains(&"log:w 100".to_string()), "{log:?}");
    n
}

#[test]
fn qy53o1_write_to_an_unread_attribute_forces_no_layout() {
    // `abbr[title]` only reads title on <abbr>, so a <a title> write is inert.
    let css = "abbr[title]{padding:5px} a:hover{color:red}";
    assert_eq!(qy53o1_layouts(css, "a.setAttribute('title', 'x'); a.title = ''; a.removeAttribute('title'); a.setAttribute('data-k', '1')"), 1);
}

#[test]
fn qy53o1_attribute_selectors_make_the_write_relevant() {
    let w = "a.setAttribute('title', 'x')";
    assert_eq!(qy53o1_layouts("a[title]{padding:5px}", w), 2, "[attr] on the same tag");
    assert_eq!(qy53o1_layouts("[title]{padding:5px}", w), 2, "bare [attr]");
    assert_eq!(qy53o1_layouts("a:is([title]){padding:5px}", w), 2, "[attr] in :is");
    assert_eq!(qy53o1_layouts("a:not([title]){padding:5px}", w), 2, "[attr] in :not");
    assert_eq!(qy53o1_layouts("div [TITLE=x] span{padding:5px}", w), 2, "attr in an ancestor compound");
    assert_eq!(qy53o1_layouts("abbr[title]{padding:5px}", "ab.setAttribute('title', 'x')"), 2, "on abbr itself");
}

#[test]
fn qy53o1_attr_function_makes_the_write_relevant() {
    let css = "a::before{content: attr(title)}";
    assert_eq!(qy53o1_layouts(css, "a.setAttribute('title', 'x')"), 2);
    assert_eq!(qy53o1_layouts(css, "a.setAttribute('data-q', 'x')"), 1);
    // `content` with var(): an attr() inside it is unpinned (everything relevant); a plain one is not.
    let css = "a::before{--s:'-'; content: attr(href) var(--s) attr(title)}";
    assert_eq!(qy53o1_layouts(css, "a.setAttribute('title', 'x')"), 2);
    let css = "a::before{--s:'-'; content: counter(c) var(--s)}";
    assert_eq!(qy53o1_layouts(css, "a.setAttribute('title', 'x')"), 1);
}

#[test]
fn qy53o1_layout_relevant_attributes_still_relayout() {
    assert_eq!(qy53o1_layouts("p{color:red}", "d.setAttribute('width', '50')"), 2);
    // QYTC1d: a class no rule names is inert now, so the class must be one a rule names.
    assert_eq!(qy53o1_layouts("p{color:red} .k{color:blue}", "d.className = 'k'"), 2);
}

#[test]
fn qy53o1_a_new_rule_list_invalidates_the_inert_verdict() {
    let (log, n) = qy53c_run(
        &qy53o1_page("abbr[title]{padding:5px}"),
        "var a = document.getElementById('a'), d = document.getElementById('d');
         d.offsetWidth;
         var s = document.createElement('style'); s.textContent = 'a[title]{display:block;height:40px}'; document.head.appendChild(s);
         a.setAttribute('title', 'x');
         console.log('h', a.offsetHeight);",
    );
    assert!(log.contains(&"log:h 40".to_string()), "{log:?}");
    assert_eq!(n, 2, "{log:?}");
}

// ---------- QMEM3e: getComputedStyle shares the layout build's parsed rules ----------

fn qmem3e_parsed() -> usize {
    crate::rules_cache::PIECES_PARSED.with(|c| c.get())
}

#[test]
fn qmem3e_pieces_are_parsed_once_across_computed_style_and_forced_layout() {
    // 3 <style> pieces + the (empty) extra piece = 4, parsed once although both
    // getComputedStyle and the forced layout read the rules.
    let n0 = qmem3e_parsed();
    let log = run_in_html(
        "<html><head><style>.a{color:rgb(1,2,3)}</style><style>.b{font-size:20px}</style><style>#c{margin-left:7px}</style></head>\
         <body><p id=c class='a b'>x</p></body></html>",
        "var c=document.getElementById('c'); var cs=getComputedStyle(c); var o=[cs.color, cs.fontSize, cs.marginLeft]; \
         o.push(c.offsetWidth>=0); o.push(getComputedStyle(c).color); console.log(o.join('|'));",
    );
    assert_eq!(log.join("\n"), "log:rgb(1, 2, 3)|20px|7px|true|rgb(1, 2, 3)", "{log:?}");
    assert_eq!(qmem3e_parsed() - n0, 4, "each piece parsed once across both caches");
}

/// QY53f1d2: one `RuleIndex::build` per rules_cache generation across getComputedStyle, a forced
/// layout and a plain layout build; a spliced `<style>` insert builds exactly one more.
#[test]
fn qy53f1d2_rule_index_is_built_once_per_generation() {
    let builds = || crate::style::INDEX_BUILDS.with(|c| c.get());
    let n0 = builds();
    let log = run_in_html(
        "<html><head><style>.qf2a{color:rgb(4,5,6)}</style></head><body><p id=c class=qf2a>x</p></body></html>",
        "var c=document.getElementById('c'); var o=[getComputedStyle(c).color]; o.push(c.offsetWidth>=0); \
         o.push(getComputedStyle(c).color); console.log(o.join('|'));",
    );
    assert_eq!(log.join("\n"), "log:rgb(4, 5, 6)|true|rgb(4, 5, 6)", "{log:?}");
    assert_eq!(builds() - n0, 1, "getComputedStyle and the layout build share one index");
    let doc = parse_html("<html><head><style>.qf2b{color:rgb(7,8,9)}</style></head><body><p class=qf2b>x</p></body></html>");
    let n1 = builds();
    let _ = build_layout_tree(&doc, 800.0);
    let _ = build_layout_tree(&doc, 800.0);
    let (rules, ..) = crate::rules_cache::current().unwrap();
    let _ = crate::rules_cache::shared_index(&rules);
    assert_eq!(builds() - n1, 1, "layout builds and a later shared_index reuse the cache's index");
    drop(rules);
    let head = find_by_tag(&doc, "head").unwrap();
    let frag = parse_html("<style>.qf2c{color:red}</style>");
    let mut v = Vec::new();
    crate::images::collect_style_nodes(&frag, &mut v);
    head.children.borrow_mut().push(v.remove(0));
    crate::rules_cache::FULL_PARSES.with(|c| c.set(0));
    let _ = build_layout_tree(&doc, 800.0);
    assert_eq!(crate::rules_cache::FULL_PARSES.with(|c| c.get()), 0, "still spliced");
    assert_eq!(builds() - n1, 2, "one rebuild for the new generation");
}

#[test]
fn qmem3e_layer_page_and_sibling_rule_page_unchanged() {
    let log = run_in_html(
        "<html><head><style>@layer a, b; @layer b{.x{color:rgb(0,0,255)}} @layer a{.x{color:rgb(255,0,0)}}</style>\
         <style>.y{color:rgb(9,9,9)} p + p{margin-left:11px}</style></head>\
         <body><p id=p class=x><span></span></p><p id=q class=y></p></body></html>",
        "var p=document.getElementById('p'), q=document.getElementById('q'); var o=[getComputedStyle(p).color, getComputedStyle(q).marginLeft, getComputedStyle(q).color]; \
         q.parentNode.removeChild(p); o.push(getComputedStyle(q).marginLeft); console.log(o.join('|'));",
    );
    assert_eq!(log.join("\n"), "log:rgb(0, 0, 255)|11px|rgb(9, 9, 9)|0px", "{log:?}");
}

// ---------- QYTC1a: contained inline-style writes move the cached boxes in place ----------

/// A YouTube-progress-bar-like fixture: absolutely positioned bars inside a relative track.
fn qytc1a_page(css: &str, extra: &str) -> String {
    format!(
        "<html><head><style>{css}</style></head><body style='margin:0'>\
<div id=wrap style='padding:5px'><div id=bar style='position:relative;width:400px;height:10px;margin:7px 3px'>\
<div id=play style='position:absolute;left:0px;width:100%;height:3px;transform:scalex(0.5);background-size:10px 10px;opacity:0.8;background-color:blue'></div>\
<div id=hover style='position:absolute;left:42.19px;top:2px;width:30px;height:4px'><span>x y</span><div style='width:5px;height:5px'></div></div>\
<div id=scrub style='position:absolute;width:13px;height:13px;transform:translateX(42.19px)'>\
<div id=knob style='position:absolute;left:3px;top:3px;width:7px;height:7px'></div><p style='margin:0'>t</p>{extra}</div>\
<div id=auto style='position:absolute;left:10px;top:0px'>auto width</div>\
<div id=rel style='position:relative;left:10px;width:20px;height:2px'></div>\
</div></div><div id=after style='height:50px'>after</div></body></html>"
    )
}

/// Lays `page` out, writes `style=new` on `#id`, and returns whether the write was applied in
/// place. When it was, every rect (and ink extent) must equal a fresh full layout's.
fn qytc1a_write(page: &str, id: &str, new: &str) -> bool {
    let doc = parse_html(page);
    let t = by_id(&doc, id);
    let mut g = crate::js_geom::GeomCache::default();
    g.rect_of(&doc, 1, &t, 800.0, 600.0);
    let old = element_attr(&t, "style").map(|s| s.to_string());
    crate::js_dom::set_attr(&t, "style", new);
    if !g.style_write_in_place(&t, old.as_deref(), Some(new), 1) {
        return false;
    }
    let mut fresh = crate::js_geom::GeomCache::default();
    fresh.rect_of(&doc, 2, &t, 800.0, 600.0);
    let (a, b) = (g.all_rects(), fresh.all_rects());
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!((x.0, x.2), (y.0, y.2));
        for k in 0..6 {
            assert!((x.1[k] - y.1[k]).abs() < 0.01, "node {} field {k}: in place {} vs fresh {} ({id} -> {new})", x.0, x.1[k], y.1[k]);
        }
    }
    true
}

#[test]
fn qytc1a_contained_writes_match_a_fresh_layout() {
    let p = qytc1a_page("", "");
    let h = "position:absolute;top:2px;width:30px;height:4px";
    assert!(qytc1a_write(&p, "hover", &format!("{h};left:42.45px")), "abs left, fractional");
    // Past the track's right edge: the ancestors' ink extents grow.
    assert!(qytc1a_write(&p, "hover", &format!("{h};left:600px")), "abs left beyond the parent");
    assert!(qytc1a_write(&p, "hover", "position:absolute;left:42.19px;top:30px;width:30px;height:4px"), "abs top");
    let s = "position:absolute;width:13px;height:13px";
    assert!(qytc1a_write(&p, "scrub", &format!("{s};transform:translateX(42.45px)")), "translate");
    assert!(qytc1a_write(&p, "scrub", &format!("{s};transform:translate(50%, 700px)")), "translate % and down");
    assert!(
        qytc1a_write(&p, "play", "position:absolute;left:0px;width:100%;height:3px;transform:scalex(0.7);background-size:12px 10px;opacity:0.5;background-color:red"),
        "paint only"
    );
    // Added (not changed) opacity / background colour: their cascaded old value is unknown.
    assert!(!qytc1a_write(&p, "hover", &format!("{h};left:42.19px;opacity:0.5;background-color:red")), "paint fields added");
    // Positioned descendants that layout hoists into the stacking-context owner (`#knob`, a
    // z-indexed one, a relative one, nested) move with the box and their owner's extent follows.
    let deep = "<div style='position:absolute;z-index:2;left:20px;top:30px;width:3px;height:3px'>\
<div style='position:relative;left:900px;width:4px;height:4px'></div></div>";
    let p = qytc1a_page("", deep);
    assert!(qytc1a_write(&p, "scrub", &format!("{s};transform:translateX(160px)")), "hoisted descendants");
    assert!(qytc1a_write(&p, "scrub", &format!("{s};transform:translateX(-30px)")), "hoisted descendants, back");
    let p = qytc1a_page("#bar{opacity:0.9}", deep);
    assert!(qytc1a_write(&p, "scrub", &format!("{s};transform:translate(10px, 9px)")), "owner is the track");
}

#[test]
fn qytc1a_uncontained_writes_fall_back_to_a_full_layout() {
    let h = "position:absolute;top:2px;width:30px;height:4px";
    let s = "position:absolute;width:13px;height:13px";
    let w = |css: &str, extra: &str, id: &str, new: &str| qytc1a_write(&qytc1a_page(css, extra), id, new);
    assert!(!w("[style*=left]{color:red}", "", "hover", &format!("{h};left:60px")), "a selector reads style");
    assert!(!w("div::before{content: attr(style)}", "", "hover", &format!("{h};left:60px")), "attr(style)");
    assert!(!w("#bar > #hover{left:1px !important}", "", "hover", &format!("{h};left:60px")), "an !important left rule");
    assert!(!w("div{transform:none !important}", "", "scrub", &format!("{s};transform:translateX(60px)")), "!important transform");
    assert!(!w("div{left:var(--l) !important}", "", "hover", &format!("{h};left:60px")), "!important var()");
    // An `!important` rule that does not match the written element does not matter.
    assert!(w("#knob{left:1px !important}", "", "hover", &format!("{h};left:60px")), "!important elsewhere");
    assert!(!w("", "", "hover", &format!("{h};left:60px !important")), "inline !important");
    assert!(!w("", "", "auto", "position:absolute;left:30px;top:0px"), "width: auto");
    assert!(!w("", "", "hover", &format!("{h};left:42.19px;display:none")), "display change");
    assert!(!w("", "", "hover", &format!("{h};left:42.19px;width:31px")), "width change");
    assert!(!w("", "", "rel", "position:relative;left:30px;width:20px;height:2px"), "relative offset");
    let fixed = "<div style='position:fixed;width:3px;height:3px'></div>";
    assert!(!w("", fixed, "scrub", &format!("{s};transform:translateX(60px)")), "fixed descendant");
    assert!(!w("", "", "scrub", &format!("{s};transform:translateX(2em)")), "em translate");
    assert!(!w("", "", "hover", &format!("{h};left:42.19px;opacity:1")), "opacity added at 1");
    assert!(!w("", "", "hover", &format!("{h};left:var(--x)")), "var()");
    assert!(!w("", "", "hover", h), "left removed");
}

#[test]
fn qytc1a_style_writes_between_reads_force_no_layout() {
    let (log, n) = qy53c_run(
        &qytc1a_page(".relayout{color:red}", ""), // QYTC1d: a class some rule names
        "var hv = document.getElementById('hover'), sc = document.getElementById('scrub'), pl = document.getElementById('play');
         var wr = document.getElementById('wrap');
         var x0 = hv.getBoundingClientRect().left, s0 = sc.getBoundingClientRect().left;
         for (var i = 1; i <= 5; i++) {
           hv.style.left = (42.19 + i) + 'px'; console.log('hv', Math.round(hv.getBoundingClientRect().left - x0));
           sc.style.transform = 'translateX(' + (42.19 + 2 * i) + 'px)'; console.log('sc', Math.round(sc.getBoundingClientRect().left - s0));
           pl.style.transform = 'scalex(0.' + i + ')'; pl.offsetWidth;
         }
         hv.style.left = '600px'; var sw = wr.scrollWidth;
         wr.className = 'relayout'; console.log('sw', sw === wr.scrollWidth, sw > 600);",
    );
    assert!(log.contains(&"log:hv 5".to_string()) && log.contains(&"log:sc 10".to_string()), "{log:?}");
    assert!(log.contains(&"log:sw true true".to_string()), "{log:?}");
    assert_eq!(n, 2, "only the first read and the class change lay out: {log:?}");
    // The same write with a selector reading `style` lays out on the next read.
    let (_, n) = qy53c_run(
        &qytc1a_page("[style*=zz]{color:red}", ""),
        "var hv = document.getElementById('hover'); hv.offsetLeft; hv.style.left = '50px'; hv.offsetLeft;",
    );
    assert_eq!(n, 2);
}

/// QY53f1d: QYTC1a's `!important` geometry check used to keep a clone of the rule list's `Rc`,
/// so every `<style>` insert after a forced layout re-parsed the whole page CSS (YouTube: 3 extra
/// 4.4 MB parses per minute) instead of splicing in the new piece. The check must still see a
/// rule that arrives in the inserted piece.
#[test]
fn qy53f1d_important_geo_check_keeps_style_inserts_incremental() {
    let page = qytc1a_page("#knob{left:1px !important}", "");
    let doc = parse_html(&page);
    let hover = by_id(&doc, "hover");
    let mut g = crate::js_geom::GeomCache::default();
    g.rect_of(&doc, 1, &hover, 800.0, 600.0);
    let h = "position:absolute;top:2px;width:30px;height:4px";
    let old = element_attr(&hover, "style").map(|s| s.to_string());
    let new = format!("{h};left:60px");
    crate::js_dom::set_attr(&hover, "style", &new);
    assert!(g.style_write_in_place(&hover, old.as_deref(), Some(&new), 1), "an !important rule elsewhere does not matter");
    // Insert a second `<style>` whose `!important` rule targets `#hover`, then lay out again.
    let head = find_by_tag(&doc, "head").unwrap();
    let frag = parse_html("<style>#bar > #hover{left:1px !important}</style>");
    let mut v = Vec::new();
    crate::images::collect_style_nodes(&frag, &mut v);
    head.children.borrow_mut().push(v.remove(0));
    crate::rules_cache::FULL_PARSES.with(|c| c.set(0));
    crate::rules_cache::PIECES_PARSED.with(|c| c.set(0));
    g.rect_of(&doc, 2, &hover, 800.0, 600.0);
    assert_eq!(crate::rules_cache::FULL_PARSES.with(|c| c.get()), 0, "the insert is spliced, not a full re-parse");
    assert_eq!(crate::rules_cache::PIECES_PARSED.with(|c| c.get()), 1, "only the new piece is parsed");
    let old = element_attr(&hover, "style").map(|s| s.to_string());
    let new = format!("{h};left:70px");
    crate::js_dom::set_attr(&hover, "style", &new);
    assert!(!g.style_write_in_place(&hover, old.as_deref(), Some(&new), 2), "the spliced-in !important left rule is seen");
}

// ---------- QYTC1d: `class` writes that change no element's matched rules keep the layout ----------

/// `class` of `x` becomes `new` (`None`: removed) the way `__d_setattr`/`__d_rmattr` do it;
/// returns whether the write was found inert (no geometry bump).
fn qytc1d_class(x: &Handle, new: Option<&str>) -> bool {
    let old = element_attr(x, "class").map(|s| s.to_string());
    let probe = crate::js_geom::class_write_pre(x, old.as_deref(), new);
    match new {
        Some(v) => crate::js_dom::set_attr(x, "class", v),
        None => crate::js_dom::remove_attr(x, "class"),
    }
    probe.is_some_and(crate::js_geom::class_write_post)
}

const QYTC1D_CSS: &str = ".autohide .no-such{display:none} .q.r{height:9px} .on #bar{padding-top:10px} .tint #knob{color:red} \
.sib ~ #auto{width:5px} #hover.big{width:60px}";

#[test]
fn qytc1d_mixed_inert_writes_match_a_fresh_layout() {
    let doc = parse_html(&qytc1a_page(QYTC1D_CSS, ""));
    let (wrap, bar, hover, scrub) = (by_id(&doc, "wrap"), by_id(&doc, "bar"), by_id(&doc, "hover"), by_id(&doc, "scrub"));
    let g0 = crate::js_dom::geom_gen();
    let mut g = crate::js_geom::GeomCache::default();
    g.rect_of(&doc, g0, &wrap, 800.0, 600.0);
    assert!(qytc1d_class(&wrap, Some("js-state")), "a class no rule names");
    assert!(qytc1d_class(&bar, Some("q")), ".q.r needs both classes");
    assert!(qytc1d_class(&wrap, Some("js-state autohide")), "a descendant rule matching nothing below");
    let old = element_attr(&hover, "style").map(|s| s.to_string());
    let new = "position:absolute;left:50px;top:2px;width:30px;height:4px";
    crate::js_dom::set_attr(&hover, "style", new);
    assert!(g.style_write_in_place(&hover, old.as_deref(), Some(new), g0), "an in-place shift after inert class writes");
    assert!(crate::js_geom::attr_write_unread(&bar, "aria-valuenow"), "aria after inert class writes");
    crate::js_dom::set_attr(&bar, "aria-valuenow", "5");
    assert!(qytc1d_class(&scrub, Some("x y")), "classes on a box that moved");
    assert!(qytc1d_class(&bar, None), "class removed");
    assert!(qytc1d_class(&wrap, Some("autohide js-state")), "token order");
    let mut fresh = crate::js_geom::GeomCache::default();
    fresh.rect_of(&doc, g0 + 1, &wrap, 800.0, 600.0);
    let (a, b) = (g.all_rects(), fresh.all_rects());
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!((x.0, x.2), (y.0, y.2));
        for k in 0..6 {
            assert!((x.1[k] - y.1[k]).abs() < 0.01, "node {} field {k}: kept {} vs fresh {}", x.0, x.1[k], y.1[k]);
        }
    }
}

#[test]
fn qytc1d_class_writes_that_change_matched_rules_are_not_inert() {
    let fresh = |css: &str| {
        let doc = parse_html(&qytc1a_page(css, ""));
        let mut g = crate::js_geom::GeomCache::default();
        g.rect_of(&doc, crate::js_dom::geom_gen(), &by_id(&doc, "wrap"), 800.0, 600.0);
        (doc, g)
    };
    let (doc, _g) = fresh(QYTC1D_CSS);
    assert!(!qytc1d_class(&by_id(&doc, "wrap"), Some("on")), "a descendant's rule starts to match");
    let (doc, _g) = fresh(QYTC1D_CSS);
    // Exactness first: a paint-only rule that starts to match still counts.
    assert!(!qytc1d_class(&by_id(&doc, "scrub"), Some("tint")), "a colour rule on a descendant");
    let (doc, _g) = fresh(QYTC1D_CSS);
    assert!(!qytc1d_class(&by_id(&doc, "scrub"), Some("sib")), "a later sibling's rule");
    let (doc, _g) = fresh(QYTC1D_CSS);
    assert!(!qytc1d_class(&by_id(&doc, "hover"), Some("big")), "the element's own rule");
    let (doc, _g) = fresh(QYTC1D_CSS);
    assert!(!qytc1d_class(&by_id(&doc, "bar"), Some("q r")), "both classes of a compound");
    // (`:has()` / `:nth-child(of)` are not parsed today, so their rules never reach the probe; it
    // treats a class inside them as unbounded should that change.)
    // Selectors reading `class` as an attribute are probed with every `class` write.
    let (doc, _g) = fresh("[class*=z]{padding:1px}");
    assert!(!qytc1d_class(&by_id(&doc, "knob"), Some("qq z1")), "[class*=] starts to match");
    let (doc, _g) = fresh("[class*=z]{padding:1px}");
    assert!(qytc1d_class(&by_id(&doc, "knob"), Some("qq")), "[class*=] still does not match");
    let (doc, _g) = fresh("[class='b a']{padding:1px}");
    assert!(!qytc1d_class(&by_id(&doc, "knob"), Some("b a")), "[class=] on the whole string");
    let (doc, _g) = fresh("p::before{content: attr(class)}");
    assert!(!qytc1d_class(&by_id(&doc, "knob"), Some("qq")), "attr(class)");
    // A stale cache (some other write since the layout) gives no verdict either.
    let (doc, _g) = fresh(QYTC1D_CSS);
    let k = by_id(&doc, "knob");
    crate::js_dom::geom_bump_for_tests();
    assert!(!qytc1d_class(&k, Some("js-state")), "stale cache");
}

/// Like QY53f1d's `!important` cache, the class index must not pin the rule list (a `<style>`
/// insert after it must still splice), and must see a class rule the insert brings.
#[test]
fn qytc1d_class_index_keeps_style_inserts_incremental() {
    let doc = parse_html(&qytc1a_page(QYTC1D_CSS, ""));
    let knob = by_id(&doc, "knob");
    let mut g = crate::js_geom::GeomCache::default();
    g.rect_of(&doc, crate::js_dom::geom_gen(), &knob, 800.0, 600.0);
    assert!(qytc1d_class(&knob, Some("later")), "no rule names it yet");
    let head = find_by_tag(&doc, "head").unwrap();
    let frag = parse_html("<style>.later2{margin-left:3px}</style>");
    let mut v = Vec::new();
    crate::images::collect_style_nodes(&frag, &mut v);
    head.children.borrow_mut().push(v.remove(0));
    crate::rules_cache::FULL_PARSES.with(|c| c.set(0));
    crate::js_dom::geom_bump_for_tests();
    g.rect_of(&doc, crate::js_dom::geom_gen(), &knob, 800.0, 600.0);
    assert_eq!(crate::rules_cache::FULL_PARSES.with(|c| c.get()), 0, "the insert is spliced, not a full re-parse");
    assert!(!qytc1d_class(&knob, Some("later later2")), "the spliced-in class rule is seen");
}

/// The probe only runs for the first few class writes after a geometry read: a script churning
/// classes without reading pays one layout on its next read instead of a probe per write.
#[test]
fn qytc1d_class_churn_without_reads_stops_probing() {
    let code = |read_every: u32| {
        format!(
            "var wr = document.getElementById('wrap'); wr.offsetWidth;
             for (var i = 1; i <= 100; i++) {{ wr.classList.toggle('js-state'); if (i % {read_every} == 0) wr.offsetWidth; }}
             wr.offsetWidth;"
        )
    };
    let (_, n) = qy53c_run(&qytc1a_page(QYTC1D_CSS, ""), &code(1000));
    assert_eq!(n, 2, "100 writes without a read: the budget runs out, one more layout");
    let (_, n) = qy53c_run(&qytc1a_page(QYTC1D_CSS, ""), &code(10));
    assert_eq!(n, 1, "a read every 10 writes refills the budget");
}

#[test]
fn qytc1d_inert_writes_between_reads_force_no_layout() {
    let (log, n) = qy53c_run(
        &qytc1a_page(QYTC1D_CSS, ""),
        "var wr = document.getElementById('wrap'), hv = document.getElementById('hover'), bar = document.getElementById('bar');
         var x0 = hv.getBoundingClientRect().left;
         for (var i = 1; i <= 5; i++) {
           wr.classList.toggle('autohide'); wr.classList.toggle('js-active');
           bar.setAttribute('aria-valuenow', String(i)); bar.setAttribute('aria-valuetext', i + ' s');
           hv.style.left = (42.19 + i) + 'px';
           console.log('hv', Math.round(hv.getBoundingClientRect().left - x0), bar.offsetHeight);
         }
         wr.classList.add('on'); console.log('bar', bar.offsetHeight);",
    );
    assert!(log.contains(&"log:hv 5 10".to_string()), "{log:?}");
    assert!(log.contains(&"log:bar 20".to_string()), "{log:?}");
    assert_eq!(n, 2, "only the first read and the rule-changing class lay out: {log:?}");
}

// ---------- Q33b2: the real window size ----------

fn q33b2_engine(html: &str, code: &str) -> (JsEngine, Rc<RefCell<Vec<String>>>) {
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let eng = start_scripts(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}")))).unwrap();
    (eng, out)
}

#[test]
fn q33b2_viewport_follows_window_size_and_forced_layout_matches() {
    // Headless: no window size published -> the default.
    crate::js::set_window_viewport(0, 0);
    let page = "<html><head><style>body{margin:0}#p{width:50%;height:10px}</style></head><body><div id=p></div></body></html>";
    let code = "var n = 0; addEventListener('resize', function () { n++; });\
                globalThis.snap = function () { var r = document.getElementById('p').getBoundingClientRect();\
                  console.log('s', innerWidth, innerHeight, document.documentElement.clientWidth, document.documentElement.clientHeight, visualViewport.width, r.width, n); };\
                snap();";
    let (eng, out) = q33b2_engine(page, code);
    assert!(out.borrow().iter().any(|l| l == "log:s 1024 768 1024 768 1024 512 0"), "{:?}", out.borrow());
    // A resize: the values follow, the %-width box is laid out at the new width, one event.
    eng.set_viewport(500, 400);
    eng.set_viewport(500, 400);
    eng.eval("t", "snap();");
    assert!(out.borrow().iter().any(|l| l == "log:s 500 400 500 400 500 250 1"), "{:?}", out.borrow());
    eng.set_viewport(800, 400);
    eng.eval("t", "snap();");
    assert!(out.borrow().iter().any(|l| l == "log:s 800 400 800 400 800 400 2"), "{:?}", out.borrow());
    // A new engine starts at the published window size, with no event.
    crate::js::set_window_viewport(640, 480);
    let (_e2, out2) = q33b2_engine(page, code);
    assert!(out2.borrow().iter().any(|l| l == "log:s 640 480 640 480 640 320 0"), "{:?}", out2.borrow());
    crate::js::set_window_viewport(0, 0);
}

#[test]
fn q33b2w_engine_created_after_publish_sees_size_in_first_script_and_no_resize() {
    crate::js::set_window_viewport(956, 312);
    let code = "var n = 0; addEventListener('resize', function () { n++; }); console.log('first', innerWidth, innerHeight);";
    let (eng, out) = q33b2_engine("<html><body></body></html>", code);
    // The App's first about_to_wait pushes the same size: no event.
    eng.set_viewport(956, 312);
    eng.eval("t", "console.log('after', n);");
    let o = out.borrow();
    assert!(o.iter().any(|l| l == "log:first 956 312"), "{o:?}");
    assert!(o.iter().any(|l| l == "log:after 0"), "{o:?}");
    crate::js::set_window_viewport(0, 0);
}

/// QYSCR1a: `offsetParent` is null for a `position:fixed` box itself but NOT for its descendants (they get the
/// fixed box). YouTube's post-load focus helper skips elements whose `offsetParent` is null, so the fixed
/// masthead subtree was skipped and the first filter chip took the focus instead (Space then pressed it).
#[test]
fn qyscr1a_offset_parent_of_fixed_descendants() {
    let log = run_in_html(
        "<html><head><style>body{margin:0} #f{position:fixed;top:0;left:0;width:100px;height:40px} #g{position:absolute;top:5px;left:5px}</style></head>\
         <body><div id=f><button id=b>x</button><div id=g><i id=i>y</i></div></div><p id=p>text</p></body></html>",
        r#"var f = document.getElementById('f'), b = document.getElementById('b'), i = document.getElementById('i');
         console.log('op', f.offsetParent, b.offsetParent && b.offsetParent.id, i.offsetParent && i.offsetParent.id,
                     document.getElementById('p').offsetParent.tagName, document.body.offsetParent);"#,
    );
    assert!(log.iter().any(|l| l == "log:op null f g BODY null"), "{log:?}");
}
