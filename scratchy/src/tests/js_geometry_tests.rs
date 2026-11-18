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
    let (c0, p0) = (crate::js_style::cascade_count(), crate::js_style::parse_count());
    // Attribute and text writes elsewhere: every read is answered from the cache.
    assert_eq!(ev("var r; for (var i=0;i<5;i++){ u.setAttribute('data-x',''+i); u.textContent='n'+i; r=rd(); } r"), "rgb(9, 9, 9)");
    assert_eq!(crate::js_style::cascade_count(), c0, "unrelated writes must not re-cascade");
    assert_eq!(crate::js_style::parse_count(), p0, "unrelated writes must not re-parse");
    // A write on the element itself re-cascades just that element.
    assert_eq!(ev("t.className=''; rd()"), "rgb(0, 0, 0)");
    let c1 = crate::js_style::cascade_count();
    assert!(c1 > c0 && c1 - c0 <= 2, "{c0} -> {c1}");
    // A new stylesheet invalidates everything and is re-parsed (only the new piece).
    assert_eq!(ev("var st=document.createElement('style'); st.textContent='.t{color:rgb(1,1,1)}'; document.head.appendChild(st); t.className='t'; rd()"), "rgb(1, 1, 1)");
    assert_eq!(crate::js_style::parse_count() - p0, 1, "only the new <style> is parsed");
    assert_eq!(ev("st.textContent='.t{color:rgb(2,2,2)}'; rd()"), "rgb(2, 2, 2)");
    assert_eq!(crate::js_style::parse_count() - p0, 2);
}
