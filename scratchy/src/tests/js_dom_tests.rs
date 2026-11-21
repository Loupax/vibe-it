#[allow(unused_imports)]
use super::*;

#[test]
fn q33c_class_attribute_and_classlist_writes_reach_the_tree() {
    let (log, body) = mutate(
        "<html class=no-js><body><div id=a class=x>t</div></body></html>",
        "var a=document.getElementById('a'); a.setAttribute('data-k','v'); a.id='b';\
         a.classList.add('y','z'); a.classList.remove('x'); console.log(a.classList.toggle('q'), a.classList.toggle('q'), a.className);\
         a.removeAttribute('data-k'); a.title='T';",
    );
    assert_eq!(log, ["log:true false y z"], "{log:?}");
    assert_eq!(body, "<div id=\"b\" class=\"y z\" title=\"T\">t</div>");
    let doc = parse_html("<html class=no-js><body></body></html>");
    let _ = start_scripts(&doc, &[src("document.documentElement.className = 'js-enabled'")], Rc::new(|_, _| {}));
    let html = doc.children.borrow().iter().find(|c| element_tag(c) == "html").cloned().unwrap();
    assert_eq!(element_attr(&html, "class").as_deref(), Some("js-enabled"));
}

#[test]
fn q33c_text_create_append_insert_remove() {
    let (log, body) = mutate(
        "<html><body><ul id=l><li>1</li><li id=two>2</li></ul><p id=p>old <b>x</b></p></body></html>",
        "var l=document.getElementById('l');\
         var n=document.createElement('LI'); n.textContent='new'; l.appendChild(n);\
         var f=document.createElement('li'); f.appendChild(document.createTextNode('first')); l.insertBefore(f, l.firstChild);\
         l.removeChild(document.getElementById('two'));\
         document.getElementById('p').textContent='plain';\
         var c=n.cloneNode(true); c.textContent='clone'; l.append(c);\
         try { n.appendChild(l); } catch (e) { console.log(e.name); }\
         var fr=document.createDocumentFragment(); fr.appendChild(document.createElement('hr')); fr.appendChild(document.createElement('br'));\
         document.body.appendChild(fr); console.log(fr.childNodes.length, l.children.length, n.parentNode===l);",
    );
    assert_eq!(log, ["log:HierarchyRequestError", "log:0 4 true"], "{log:?}");
    assert_eq!(body, "<ul id=\"l\"><li>first</li><li>1</li><li>new</li><li>clone</li></ul><p id=\"p\">plain</p><hr><br>");
}

// Acid3 (real page): `span.firstChild.data = score` must write the text node.
#[test]
fn acid3_text_node_data_setter_writes_the_tree() {
    let (log, body) = mutate(
        "<html><body><p id=p><span id=s>JS</span></p></body></html>",
        "var t=document.getElementById('s').firstChild; t.data='42'; console.log(t.data, t.nodeValue, t.length);",
    );
    assert_eq!(log, ["log:42 42 2"], "{log:?}");
    assert_eq!(body, "<p id=\"p\"><span id=\"s\">42</span></p>");
}

#[test]
fn q33c_inner_html_and_remove() {
    let (log, body) = mutate(
        "<html><body><div id=d>gone</div><span id=s>x</span></body></html>",
        "var d=document.getElementById('d'); d.innerHTML='<p class=k>a<b>b</b></p><i>c</i>';\
         console.log(d.innerHTML, d.firstChild.className, d.outerHTML.slice(0,15));\
         document.getElementById('s').remove();",
    );
    assert_eq!(log, ["log:<p class=\"k\">a<b>b</b></p><i>c</i> k <div id=\"d\"><p "], "{log:?}");
    assert_eq!(body, "<div id=\"d\"><p class=\"k\">a<b>b</b></p><i>c</i></div>");
}

#[test]
fn q33c_style_property_writes_the_style_attribute() {
    let (log, body) = mutate(
        "<html><body><div id=a style='color: red'>t</div></body></html>",
        "var a=document.getElementById('a'); a.style.backgroundColor='blue'; a.style.color='';\
         a.style.setProperty('margin-top','3px'); console.log(a.style.backgroundColor, a.style.marginTop, JSON.stringify(a.style.color));\
         a.style.display='none';",
    );
    assert_eq!(log, ["log:blue 3px \"\""], "{log:?}");
    assert_eq!(body, "<div id=\"a\" style=\"background-color: blue; margin-top: 3px; display: none;\">t</div>");
}

#[test]
fn q33c_many_writes_mark_dirty_once_and_reads_stay_clean() {
    let doc = parse_html("<html><body><div id=a></div></body></html>");
    let engine = start_scripts(
        &doc,
        &[src("var a=document.getElementById('a'); for (var i=0;i<50;i++) a.className='c'+i;")],
        Rc::new(|_, _| {}),
    )
    .unwrap();
    assert!(engine.take_dirty());
    assert!(!engine.take_dirty(), "the flag clears once drained");
    let clean = start_scripts(
        &parse_html("<html><body><div id=a></div></body></html>"),
        &[src("document.getElementById('a').className")],
        Rc::new(|_, _| {}),
    )
    .unwrap();
    assert!(!clean.take_dirty(), "reads never dirty the page");
}

#[test]
fn q33c2_closed_dialog_is_not_rendered_open_dialog_is() {
    // QB4: an open dialog is now out of flow (UA `position: absolute`), so
    // it shows up in `positioned` and with its own height rather than
    // growing the document.
    let p = |d: &str| measure_document_fixture(&format!("<html><body><p>a</p>{d}</body></html>"), 500.0, 400.0).positioned.len();
    assert_eq!(p("<dialog><div style=\"height:100px\">x</div></dialog>"), 0, "closed dialog renders nothing");
    let laid = measure_document_fixture("<html><body><p>a</p><dialog open><div style=\"height:100px\">x</div></dialog></body></html>", 500.0, 400.0);
    let d = laid.positioned.first().expect("open dialog still renders");
    assert!(d.h >= 100.0, "open dialog renders its content, h {}", d.h);
}

#[test]
fn q33c3_select_color_transparent_paints_no_option_text() {
    // Wikipedia portal: `.styled-select.js-enabled select` overlays a
    // `color:transparent` select on a visible label; the select's option
    // text must not paint.
    let (pixels, _, _) = paint_fixture(
        r#"<html><head><style>.s select{background:transparent;border:0;margin:0;padding:0 24px;font-size:40px;color:transparent;color:hsla(0,0%,100%,0)}</style></head><body style="margin:0;background:#fff"><div class="s"><select><option selected>Afrikaans</option><option>English</option></select></div></body></html>"#,
    );
    // Unpainted pixels are 0, the white body fill is ffffff; any anti-aliased
    // glyph pixel is something else.
    let stray = pixels.iter().filter(|&&p| p & 0x00FF_FFFF != 0 && p & 0x00FF_FFFF != 0x00FF_FFFF).count();
    assert_eq!(stray, 0, "transparent select text must not paint");
    assert_eq!(
        to_text_color(&parse_declarations("color: hsl(120, 100%, 50%)").color.unwrap()),
        Some(TextColor::rgb(0, 255, 0))
    );
    assert_eq!(
        to_text_color(&parse_declarations("color: hsla(0,0%,100%,0)").color.unwrap()),
        Some(TextColor::rgba(255, 255, 255, 0))
    );
    // Partial alpha text is scaled, not painted opaque.
    let (half, _, _) = paint_fixture(
        r#"<html><body style="margin:0;background:#fff"><p style="margin:0;font-size:40px;color:rgba(0,0,0,0.2)">HHHH</p></body></html>"#,
    );
    let darkest = half.iter().filter(|&&p| p != 0).map(|&p| p & 0xFF).min().unwrap();
    assert!(darkest > 0xA0, "20% black text must stay light, darkest channel {darkest:#x}");
}

#[test]
fn q33c3_option_in_select_uses_the_selects_colour() {
    // The portal's `select option{color:grey}` must not paint the closed
    // select's text when the select itself is transparent.
    let (pixels, _, _) = paint_fixture(
        r#"<html><head><style>select{color:transparent;font-size:40px;border:0}select option{color:#777}</style></head><body style="margin:0;background:#fff"><select><option selected>Afrikaans</option></select></body></html>"#,
    );
    let stray = pixels.iter().filter(|&&p| p & 0x00FF_FFFF != 0 && p & 0x00FF_FFFF != 0x00FF_FFFF).count();
    assert_eq!(stray, 0, "option colour must not override the select's transparent text");
}

#[test]
fn q33d_capture_target_bubble_order_and_stop_propagation() {
    let (_e, _d, log) = events_engine(
        "<html><body><div id=o><p id=i>x</p></div></body></html>",
        "var o=document.getElementById('o'), i=document.getElementById('i');\
         window.addEventListener('click', function(e){ console.log('w-cap', e.eventPhase) }, true);\
         o.addEventListener('click', function(e){ console.log('o-cap', e.eventPhase) }, true);\
         i.addEventListener('click', function(e){ console.log('i-bub', e.eventPhase, e.target===i, e.currentTarget===i) });\
         i.addEventListener('click', function(e){ console.log('i-cap') }, {capture:true});\
         o.addEventListener('click', function(e){ console.log('o-bub', e.eventPhase) });\
         document.addEventListener('click', function(e){ console.log('doc'); e.stopPropagation(); });\
         window.addEventListener('click', function(e){ console.log('w-bub') });\
         i.dispatchEvent(new Event('click', {bubbles:true}));",
    );
    assert_eq!(
        *log.borrow(),
        ["log:w-cap 1", "log:o-cap 1", "log:i-cap", "log:i-bub 2 true true", "log:o-bub 3", "log:doc"]
    );
}

#[test]
fn q33d_prevent_default_once_remove_passive_onprop_and_throwing_listener() {
    let (_e, _d, log) = events_engine(
        "<html><body><div id=o><p id=i>x</p></div></body></html>",
        "var o=document.getElementById('o'), i=document.getElementById('i');\
         o.addEventListener('ping', function(){ console.log('o') });\
         var f=function(){ console.log('f') }; i.addEventListener('ping', f); i.removeEventListener('ping', f);\
         i.addEventListener('ping', function(e){ console.log('once'); e.preventDefault(); }, {once:true});\
         console.log(i.dispatchEvent(new Event('ping')));\
         console.log(i.dispatchEvent(new Event('ping', {cancelable:true})));\
         i.addEventListener('pong', function(e){ e.preventDefault(); console.log(e.defaultPrevented) }, {passive:true});\
         i.dispatchEvent(new Event('pong', {cancelable:true}));\
         i.onclick = function(){ console.log('onclick'); return false; };\
         console.log(i.dispatchEvent(new Event('click', {cancelable:true})));\
         i.addEventListener('boom', function(){ throw new Error('x') });\
         i.addEventListener('boom', function(){ console.log('after boom') });\
         i.dispatchEvent(new Event('boom'));",
    );
    let log = log.borrow();
    assert_eq!(log[..5], ["log:once", "log:true", "log:true", "log:false", "log:onclick"]);
    assert_eq!(log[5], "log:false");
    assert!(log[6].starts_with("error:Uncaught Error: x"), "{log:?}");
    assert_eq!(log[7], "log:after boom");
}

#[test]
fn q33d_native_click_reaches_page_handler_and_prevent_default_is_reported() {
    let (engine, doc, log) = events_engine(
        "<html><body><a id=a href='/x'><b id=b>t</b></a></body></html>",
        "document.getElementById('a').addEventListener('click', function(e){\
           console.log('click', e.target.id, e.isTrusted, e.clientX, e.button, e.ctrlKey); e.preventDefault(); });",
    );
    let b = by_id(&doc, "b");
    let init = [
        ("bubbles", EventField::Bool(true)),
        ("cancelable", EventField::Bool(true)),
        ("clientX", EventField::Num(12.0)),
        ("button", EventField::Num(0.0)),
    ];
    assert!(!engine.dispatch_event(Some(&b), "click", &init), "handler cancelled the click");
    assert_eq!(*log.borrow(), ["log:click b true 12 0 false"]);
    // A page with no handler lets the default action proceed.
    let (engine2, doc2, _) = events_engine("<html><body><a id=a href='/x'>t</a></body></html>", "1");
    assert!(engine2.dispatch_event(Some(&by_id(&doc2, "a")), "click", &init));
}

#[test]
fn q33d_native_key_and_focus_events_carry_their_fields() {
    let (engine, doc, log) = events_engine(
        "<html><body><a id=a href='/x'>t</a></body></html>",
        "var a=document.getElementById('a');\
         document.addEventListener('keydown', function(e){ console.log('kd', e.key, e.repeat, e.shiftKey, e.target.id); });\
         a.addEventListener('focus', function(e){ console.log('focus', e.type, e.bubbles); });\
         document.addEventListener('focusin', function(e){ console.log('focusin', e.target.id); });\
         a.addEventListener('blur', function(e){ console.log('blur'); });\
         document.body.addEventListener('keyup', function(e){ console.log('ku', e.key, e.target.tagName); e.preventDefault(); });",
    );
    let a = by_id(&doc, "a");
    let key = |k: &str, shift: bool| {
        [
            ("bubbles", EventField::Bool(true)),
            ("cancelable", EventField::Bool(true)),
            ("key", EventField::Str(k.to_string())),
            ("shiftKey", EventField::Bool(shift)),
        ]
    };
    assert!(engine.dispatch_event(Some(&a), "keydown", &key("\u{e9}", true)));
    assert!(!engine.dispatch_event(None, "keyup", &key("Enter", false)), "None targets <body>");
    engine.dispatch_event(Some(&a), "focus", &[("bubbles", EventField::Bool(false))]);
    engine.dispatch_event(Some(&a), "focusin", &[("bubbles", EventField::Bool(true))]);
    engine.dispatch_event(Some(&a), "blur", &[]);
    assert_eq!(
        *log.borrow(),
        ["log:kd \u{e9} false true a", "log:ku Enter BODY", "log:focus focus false", "log:focusin a", "log:blur"]
    );
}

#[test]
fn q33d_load_events_and_event_target_subclass() {
    let log = run_collecting(&[src(
        "document.addEventListener('DOMContentLoaded', function(e){ console.log('dcl', e.target===document, e.bubbles); });\
         window.addEventListener('DOMContentLoaded', function(e){ console.log('dcl-window'); });\
         window.addEventListener('load', function(e){ console.log('load', e.target===window); });\
         var t = new EventTarget(); t.addEventListener('x', function(e){ console.log('x', e instanceof CustomEvent, e.detail); });\
         t.dispatchEvent(new CustomEvent('x', {detail: 7}));\
         addEventListener('y', function(){ console.log('global y'); }); dispatchEvent(new Event('y'));",
    )]);
    assert_eq!(
        log,
        ["log:x true 7", "log:global y", "log:dcl true true", "log:dcl-window", "log:load true"]
    );
}

#[test]
fn qb1_event_loop_exit_message() {
    use winit::error::EventLoopError;
    assert_eq!(event_loop_exit_message(&Ok(())), None);
    let m = event_loop_exit_message(&Err(EventLoopError::ExitFailure(1))).unwrap();
    assert!(m.contains("Wayland compositor"), "{m}");
    assert!(!m.contains(char::from(10)));
}

// Q49a: a float's shrink-to-fit width is min(max(min-content, available),
// max-content), so a float in a container narrower than its longest word is
// as wide as the word, not squeezed to the container.
#[test]
fn q49a_float_in_narrow_container_is_at_least_min_content() {
    let laid = measure_fixture(
        r#"<html><body style="margin:0"><div style="width:5px"><div style="float:left;height:0">WWWWWWWWWW</div></div></body></html>"#,
        500.0,
    );
    fn find_float(l: &Laid) -> Option<&Laid> {
        if l.style.float != Float::None {
            return Some(l);
        }
        match &l.content {
            LaidContent::Blocks(cs) => cs.iter().find_map(find_float),
            _ => None,
        }
    }
    let f = find_float(&laid).expect("float");
    assert!(f.w > 20.0, "float w {} should be >= min-content, not the 5px container", f.w);
    assert!(shrink_to_fit(30.0, 80.0, 5.0) == 30.0 && shrink_to_fit(30.0, 80.0, 50.0) == 50.0 && shrink_to_fit(30.0, 80.0, 500.0) == 80.0);
}

// ---------- QB2: JS DOM writes use the restyle path ----------

/// An `App` with a complete cached frame and a JS engine whose script has
/// already written the DOM (engine dirty). `None` without a display.
fn qb2_app(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) -> App {
    let doc = parse_html("<html><body><p>hello</p></body></html>");
    let mut app = App::new_static(doc.clone(), proxy.clone(), None);
    app.js = run_page_scripts(&doc, &[src("document.body.setAttribute('data-tick', '1')")], None);
    app.cached = Some(Rc::new(measure_fixture("<p>hello</p>", 400.0)));
    app.layout_dirty = false;
    app.restyle_requested = false;
    app.keep_scroll_on_rebuild = false;
    app
}

/// One process can build only one event loop, so both scenarios share it.
#[test]
fn qb2_js_dom_writes_use_restyle_path() {
    let event_loop = match test_event_loop() {
        Ok(l) => l,
        Err(_) => return, // no display available (headless CI)
    };
    let proxy = event_loop.create_proxy();
    qb2_restyle_and_keep_old_frame(qb2_app(&proxy));
    qb2_coalesce_into_one_rebuild(qb2_app(&proxy));
qy53m_noop_write_no_restyle(&proxy);
    qb2_first_layout_not_made_stale(qb2_app(&proxy));
    qb5_app_applies_requests(&proxy);
    qb6_help_suspends_js(&proxy);
    qf7_app_net(&proxy);
    qf14b_app_initiator(&proxy);
    qnav9_app_file_links(&proxy);
}

// ---------- QB6: `:help` suspends the page's JS ----------

fn qb6_counter(app: &App) -> f64 {
    app.js.as_ref().unwrap().eval_value::<f64>("t", "window.__n").unwrap()
}

fn qb6_help_suspends_js(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
    let doc = parse_html("<html><body><p>page</p></body></html>");
    let mut app = App::new_static(doc.clone(), proxy.clone(), None);
    app.js = run_page_scripts(
        &doc,
        &[src("window.__n = 0; setInterval(function(){ window.__n++; document.body.setAttribute('data-n', window.__n); }, 1);")],
        None,
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    app.js.as_ref().unwrap().run_due();
    let before = qb6_counter(&app);
    assert!(before >= 1.0, "interval should have fired");

    app.show_help();
    assert!(app.js.is_none(), "help page must have no JS engine");
    assert!(app.pre_help_page.as_ref().unwrap().js.is_some(), "engine parked in SavedPage");
    // Key dispatch and sync reach nothing: the old page's listeners are gone.
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(app.js_dispatch(None, "keydown", &[]), "no engine: dispatch returns true");
    app.js_sync();
    // Re-running :help keeps the parked engine.
    app.show_help();
    assert!(app.pre_help_page.as_ref().unwrap().js.is_some());

    app.restore_saved_page();
    assert!(app.js.is_some(), "engine restored");
    assert_eq!(qb6_counter(&app), before, "timers did not run while help was shown");
    app.js.as_ref().unwrap().run_due();
    assert!(qb6_counter(&app) > before, "overdue timer fires after return");

    // show_message also drops the engine.
    app.show_message("oops");
    assert!(app.js.is_none());
}

/// QY53m: a write batch that changed no value leaves `layout_dirty` false; a
/// value-changing write (attribute or stylesheet text) still sets it.
fn qy53m_noop_write_no_restyle(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
    let doc = parse_html("<html><head><style id=s>p{color:red}</style></head><body id=b class=x><p>hello</p></body></html>");
    let mut app = App::new_static(doc.clone(), proxy.clone(), None);
    app.js = run_page_scripts(&doc, &[src("window.__noop = function(){ var b = document.body; b.setAttribute('class','x'); b.removeAttribute('nope'); document.getElementById('s').textContent = 'p{color:red}'; }; window.__chg = function(){ document.body.setAttribute('class','y'); }; window.__css = function(){ document.getElementById('s').textContent = 'p{color:blue}'; };")], None);
    app.cached = Some(Rc::new(measure_fixture("<p>hello</p>", 400.0)));
    let reset = |app: &mut App| {
        app.js.as_ref().unwrap().take_dirty();
        app.layout_dirty = false;
        app.restyle_requested = false;
    };
    reset(&mut app);
    app.js.as_ref().unwrap().eval_value::<bool>("t", "(window.__noop(), true)").unwrap();
    app.js_sync();
    assert!(!app.layout_dirty && !app.restyle_requested, "same-value writes queue no restyle");
    app.js.as_ref().unwrap().eval_value::<bool>("t", "(window.__chg(), true)").unwrap();
    app.js_sync();
    assert!(app.layout_dirty, "a changed attribute restyles");
    reset(&mut app);
    app.js.as_ref().unwrap().eval_value::<bool>("t", "(window.__css(), true)").unwrap();
    app.js_sync();
    assert!(app.layout_dirty, "a stylesheet change restyles");
}

fn qb2_restyle_and_keep_old_frame(mut app: App) {
    let generation = app.layout_generation;
    app.js_sync();
    assert!(app.layout_dirty && app.restyle_requested && app.keep_scroll_on_rebuild);
    assert_eq!(app.layout_generation, generation, "redraw bumps the generation, not js_sync");
    // Simulate what redraw does when it queues the rebuild.
    app.restyle_requested = false;
    app.hold_old_frame = true;
    app.layout_in_flight = true;
    app.layout_dirty = false;
    app.layout_generation += 1;
    let h_before = app.cached.as_ref().unwrap().h;
    let partial = measure_fixture("<p>hello</p><p>world</p><p>more</p><p>text</p>", 400.0);
    app.handle_shape_result(app.layout_generation, Box::new(partial), ShapeCache::new(), true);
    assert_eq!(app.cached.as_ref().unwrap().h, h_before, "incomplete pass must not replace the old frame");
    assert!(app.hold_old_frame);
}

fn qb2_coalesce_into_one_rebuild(mut app: App) {
    app.hold_old_frame = true;
    app.layout_in_flight = true;
    let generation = app.layout_generation;
    app.js_sync();
    assert!(!app.layout_dirty && !app.restyle_requested, "no rebuild queued while a restyle is in flight");
    assert!(app.js_restyle_pending);
    assert_eq!(app.layout_generation, generation, "in-flight result must not go stale");
    // A second write while still holding changes nothing more.
    app.js_sync();
    assert!(!app.layout_dirty && app.js_restyle_pending);
    // The complete pass lands: exactly one follow-up rebuild is requested.
    app.handle_shape_result(generation, Box::new(measure_fixture("<p>hello</p>", 400.0)), ShapeCache::new(), false);
    assert!(!app.hold_old_frame);
    assert!(!app.js_restyle_pending);
    assert!(app.layout_dirty && app.restyle_requested && app.keep_scroll_on_rebuild);
    assert!(!app.layout_in_flight);
}

/// A write while the very first layout is still being shaped (no complete
/// frame yet) must not make that pass stale either.
fn qb2_first_layout_not_made_stale(mut app: App) {
    app.cached = None;
    app.layout_in_flight = true;
    let generation = app.layout_generation;
    app.js_sync();
    assert!(app.js_restyle_pending && !app.layout_dirty);
    assert_eq!(app.layout_generation, generation);
    app.handle_shape_result(generation, Box::new(measure_fixture("<p>hello</p>", 400.0)), ShapeCache::new(), false);
    assert!(app.cached.is_some());
    assert!(app.layout_dirty && app.restyle_requested);
}

// ---------- QB4: HTMLDialogElement ----------

#[test]
fn qb4_dialog_show_modal_close_return_value_and_event() {
    let (log, body) = mutate(
        "<html><body><dialog id=d><button id=b>x</button></dialog></body></html>",
        "var d=document.getElementById('d'); var b=document.getElementById('b');\
         console.log(typeof d.showModal, typeof d.close, d.open, JSON.stringify(d.returnValue));\
         d.addEventListener('close', function(e){ console.log('closed', d.open, d.returnValue, e.type); });\
         b.addEventListener('click', function(){ d.close('x'); });\
         d.showModal(); console.log(d.open, d.hasAttribute('open'));\
         b.dispatchEvent(new Event('click',{bubbles:true}));\
         console.log(d.open, d.returnValue, d instanceof HTMLDialogElement);\
         d.show(); console.log(d.open); d.open = false; console.log(d.open);",
    );
    assert_eq!(
        log,
        ["log:function function false \"\"", "log:true true", "log:closed false x close", "log:false x true", "log:true", "log:false"],
        "{log:?}"
    );
    assert!(!body.contains("open") && !body.contains("modal"), "{body}");
}

#[test]
fn qb4_modal_dialog_is_centred_in_the_viewport() {
    let w = |attrs: &str| {
        measure_document_fixture(
            &format!("<html><body style=\"margin:0\"><dialog {attrs} style=\"width:100px;height:50px;padding:0;border:0\">x</dialog></body></html>"),
            400.0,
            300.0,
        )
    };
    let closed = w("");
    assert!(closed.positioned.is_empty(), "closed dialog renders nothing");
    let laid = w("open data-scratchy-modal");
    let d = laid.positioned.first().expect("modal dialog is out of flow");
    assert_eq!((d.x, d.y), (150.0, 125.0), "centred in 400x300");
}

#[test]
fn qb4_clicks_inside_a_modal_dialog_hit_its_content() {
    let html = "<html><body style=\"margin:0\"><p>outer</p><dialog open data-scratchy-modal style=\"width:100px;height:50px;padding:0;border:0\"><div style=\"height:50px\">in</div></dialog></body></html>";
    let doc = parse_html(html);
    let tree = build_layout_tree(&doc, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut incomplete) = (0usize, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut font_system, 400.0, 300.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let so = HashMap::new();
    let node = hit_test_element(&laid, 200.0, 150.0, &cache, &so, 0.0, 300.0);
    let h = crate::dom::find_node_by_id(&doc, node).unwrap_or_else(|| panic!("hit node {node}"));
    assert_eq!(element_tag(&h), "div", "click at the dialog's centre lands on its content");
}

/// App side: `js_sync` applies a scroll request and a focus request.
fn qb5_app_applies_requests(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
    let doc = parse_html("<html><body><input id=i><p>hello</p></body></html>");
    let mut app = App::new_static(doc.clone(), proxy.clone(), None);
    app.js = run_page_scripts(
        &doc,
        &[src("window.scrollTo(0, 120); document.getElementById('i').focus();")],
        None,
    );
    app.js_sync();
    assert_eq!(app.scroll_offset, 120.0);
    let i = crate::dom::find_node_by_id(&doc, app.interaction.focus.expect("focused")).unwrap();
    assert_eq!(element_attr(&i, "id").as_deref(), Some("i"));
    app.js.as_ref().unwrap().eval("t", "window.scrollBy(0, -500); document.getElementById('i').blur();");
    app.js_sync();
    assert_eq!(app.scroll_offset, 0.0, "clamped at the top");
    assert!(app.interaction.focus.is_none(), "blur() of the focused element clears focus");
}

#[test]
fn qa13_write_inserts_after_the_running_script() {
    let out = qa13_run(
        "<html><body><p id=a>x<script>document.write('<b id=w>hi</b>')</script><i id=z></i></p>\
         <script>var s=document.getElementById('a').getElementsByTagName('script')[0];\
         console.log(s.nextSibling.id, s.nextSibling.textContent, s.nextSibling.nextSibling.id);</script></body></html>",
    );
    assert_eq!(out, ["log:w hi z"]);
}

#[test]
fn qa13_two_scripts_each_write_and_consecutive_writes_keep_order() {
    let out = qa13_run(
        "<html><body><script>document.write('<u id=a1></u>'); document.write('<u id=a2></u>', '<u id=a3></u>')</script>\
         <script>document.write('<u id=b1></u>')</script>\
         <script>console.log(Array.prototype.map.call(document.body.children, function(e){return e.id||e.tagName}).join())</script></body></html>",
    );
    assert_eq!(out, ["log:SCRIPT,a1,a2,a3,SCRIPT,b1,SCRIPT"]);
}

#[test]
fn qa13_write_inside_head_and_writeln() {
    let out = qa13_run(
        "<html><head><script>document.writeln('<meta id=m name=k>'); document.write('<meta id=n>')</script></head>\
         <body><script>var m=document.getElementById('m'); console.log(document.head.children.length, m.nextSibling.nodeType, m.nextSibling.data === '\\n');</script></body></html>",
    );
    assert_eq!(out, ["log:3 3 true"]);
}

#[test]
fn qa13_write_after_load_is_ignored_with_a_warning() {
    let (engine, out) = qb8_start(
        "<body><script>setTimeout(function(){ document.write('<b id=late></b>'); console.log(String(document.getElementById('late'))); }, 0)</script></body>",
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    engine.run_due();
    let out = out.borrow();
    assert!(out.iter().any(|l| l.contains("ignored")), "{out:?}");
    assert!(out.iter().any(|l| l == "log:null"), "{out:?}");
}

// ---------- QA16: getComputedStyle ----------

#[test]
fn qa16_get_computed_style_cascade_inheritance_and_unknowns() {
    let (_e, _d, log) = events_engine(
        "<html><head><style>#a{white-space:pre-wrap;position:relative;z-index:3;display:flex;clear:both}\
         .c{color:#102030;font-size:2em;opacity:.5;margin-left:7px;border:2px solid red}\
         #k{white-space:pre-line;white-space:x-bogus}</style></head>\
         <body style='font-size:10px'><div id=a>x</div><p id=b class=c style='background-color:rgb(1,2,3)'>y<span id=s>z</span></p>\
         <div id=h style='display:none'></div><i id=k></i></body></html>",
        "var a=document.getElementById('a'), b=document.getElementById('b'), cs=getComputedStyle(a);\
         console.log(cs.whiteSpace, cs.zIndex, cs.display, cs.cssFloat, cs['float'], cs.position, cs.getPropertyValue('white-space'));\
         var cb=getComputedStyle(b);\
         console.log(cb.color, cb.fontSize, cb.opacity, cb.marginLeft, cb.borderTopWidth, cb.getPropertyValue('border-top-style'), cb['background-color'], cb.zIndex, cb.display);\
         console.log(getComputedStyle(document.getElementById('s')).color, getComputedStyle(document.getElementById('s')).fontSize);\
         console.log(getComputedStyle(document.getElementById('h')).display, JSON.stringify(cs.bogus), JSON.stringify(cs.getPropertyValue('bogus')), cs.length > 10, typeof cs.item(0));\
         b.style.display='none'; console.log(getComputedStyle(b).display);\
         var d=document.createElement('div'); console.log(JSON.stringify(getComputedStyle(d).display), window.document.defaultView === window);\
         console.log(getComputedStyle(document.getElementById('k')).whiteSpace); try { getComputedStyle(null) } catch (e) { console.log('te', e instanceof TypeError) }",
    );
    let l = log.borrow();
    assert_eq!(l[0], "log:pre-wrap 3 flex none none relative pre-wrap", "{l:?}");
    assert_eq!(l[1], "log:rgb(16, 32, 48) 20px 0.5 7px 2px solid rgb(1, 2, 3) auto block", "{l:?}");
    assert_eq!(l[2], "log:rgb(16, 32, 48) 20px", "{l:?}");
    assert_eq!(l[3], "log:none \"\" \"\" true string", "{l:?}");
    assert_eq!(l[4], "log:none", "write invalidates the cache: {l:?}");
    assert_eq!(l[5], "log:\"\" true", "{l:?}");
    assert_eq!(l[6], "log:pre-line", "invalid later keyword must not override: {l:?}");
    assert_eq!(l[7], "log:te true", "{l:?}");
}

// QY53f1e: several <style> pieces with @layer take the concatenated-parse path
// (parse_stylesheet_into straight into the rule list); results unchanged.
#[test]
fn qy53f1e_get_computed_style_layered_multi_style() {
    let (_e, _d, log) = events_engine(
        "<html><head><style>@layer a, b; @layer a { #x { color: red; margin-left: 1px } }</style>\
         <style>@layer b { #x { color: blue } } .y { margin-left: 9px }</style>\
         <style>#x { padding-top: 4px }</style></head><body><p id=x class=y>t</p></body></html>",
        "var cs=getComputedStyle(document.getElementById('x'));\
         console.log(cs.color, cs.marginLeft, cs.paddingTop);",
    );
    let l = log.borrow();
    assert_eq!(l[0], "log:rgb(0, 0, 255) 9px 4px", "{l:?}");
}

// QW3: wikipedia.org language-button divider. A later z-auto relative 1px
// line pulled up by a negative margin must be hidden by the earlier
// z-index:1 inline button's white background / outline, not drawn over it.
#[test]
fn qw3_divider_line_is_hidden_behind_z_indexed_inline_button() {
    let html = r#"<html><body style="margin:0;background:#fff"><div style="text-align:center;font-size:14px"><button style="display:inline;position:relative;z-index:1;margin:0 auto;padding:10px 20px;outline:20px solid #fff;border:2px solid #0000ff;font-family:inherit;background-color:#f8f9fa;font-size:14px;line-height:1"><i style="display:inline-block;vertical-align:middle;width:10px;height:10px"></i><span style="padding:0 6px;vertical-align:middle">Read</span><i style="display:inline-block;vertical-align:middle;width:10px;height:10px"></i></button></div><div style="background-color:#ff0000;display:block;position:relative;width:80%;margin:-20px auto 20px;height:1px"></div></body></html>"#;
    let (pixels, win_w, _) = paint_fixture(html);
    let mut red_in_button = 0;
    for x in 50..150u32 {
        for y in 0..40u32 {
            if px_at(&pixels, win_w, x, y) == 0x00FF_0000 {
                red_in_button += 1;
            }
        }
    }
    assert_eq!(red_in_button, 0, "divider must not paint over the button");
    // A `<button>` with `display:inline` computes to inline-block, so its
    // border paints (a plain inline box paints none).
    let mut blue = 0;
    for x in 0..200u32 {
        for y in 0..60u32 {
            if px_at(&pixels, win_w, x, y) == 0x0000_00FF {
                blue += 1;
            }
        }
    }
    assert!(blue > 20, "button border must paint, got {blue} blue pixels");
}

// ---------- QA20: table and form DOM interfaces ----------

#[test]
fn qa20_table_rows_cells_and_sections() {
    let log = run_in_html(
        "<html><body><table id=t><caption>c</caption><thead><tr><th>h</th></tr></thead>\
         <tbody><tr id=r1><td>a</td><td colspan=3 rowspan=2>b</td></tr><tr id=r2><td>c</td></tr></tbody>\
         <tfoot><tr><td>f</td></tr></tfoot></table></body></html>",
        "var t=document.getElementById('t'), r2=document.getElementById('r2'), r1=document.getElementById('r1');\
         console.log(t.rows.length, t.tBodies.length, t.tHead.tagName, t.tFoot.tagName, t.caption.tagName);\
         console.log(r2.rowIndex, r2.sectionRowIndex, r1.cells.length, r1.cells[1].cellIndex, r1.cells[1].colSpan, r1.cells[1].rowSpan, r1.cells[0].colSpan);\
         console.log(t.rows[0].parentNode.tagName, t.rows[3].parentNode.tagName, t.rows.item(1)===r1, t instanceof HTMLTableElement, r1 instanceof HTMLTableRowElement, r1.cells[0] instanceof HTMLTableCellElement);",
    );
    assert_eq!(log, ["log:4 1 THEAD TFOOT CAPTION", "log:2 1 2 1 3 2 1", "log:THEAD TFOOT true true true true"], "{log:?}");
}

#[test]
fn qa20_table_insert_delete_and_create() {
    let log = run_in_html(
        "<html><body><table id=t></table></body></html>",
        "var t=document.getElementById('t');\
         var r=t.insertRow(); var c=r.insertCell(); c.textContent='x'; r.insertCell(0).textContent='y';\
         console.log(t.rows.length, t.tBodies.length, r.cells.length, r.cells[0].textContent, r.cells[1].textContent);\
         t.insertRow(0); console.log(t.rows.length, t.rows[1]===r);\
         t.deleteRow(0); console.log(t.rows.length, t.rows[0]===r); r.deleteCell(-1); console.log(r.cells.length);\
         var h=t.createTHead(); console.log(h===t.createTHead(), t.tHead===h, t.firstChild===h);\
         var cap=t.createCaption(); console.log(t.firstChild===cap, t.caption===cap);\
         var f=t.createTFoot(); console.log(t.lastChild===f); t.deleteTHead(); t.deleteCaption(); t.deleteTFoot();\
         console.log(t.tHead, t.caption, t.tFoot);\
         try { t.insertRow(9); } catch (e) { console.log(e.name); }\
         try { t.deleteRow(9); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(
        log,
        ["log:1 1 2 y x", "log:2 true", "log:1 true", "log:1", "log:true true true", "log:true true", "log:true", "log:null null null", "log:IndexSizeError", "log:IndexSizeError"],
        "{log:?}"
    );
}

#[test]
fn qa20_forms_collection_and_elements_named_access() {
    let log = run_in_html(
        "<html><body><form id=f1 name=first action=/go method=post><input name=a value=1><select name=s><option>x<option selected value=v>y</select>\
         <textarea name=ta>hi</textarea><button name=b>go</button><input type=image name=img></form>\
         <form id=f2><input id=z></form></body></html>",
        "var f=document.forms[0];\
         console.log(document.forms.length, f.id, document.forms.f2.id, document.forms.namedItem('first')===f, f.length, f.elements.length);\
         console.log(f.elements.a.value, f.elements.namedItem('s').tagName, f.elements[2].name, f.elements.img, f.action, f.method);\
         console.log(document.forms.f2.elements.z.form===document.forms.f2, typeof f.submit, typeof f.reset);",
    );
    assert_eq!(log, ["log:2 f1 f2 true 4 4", "log:1 SELECT ta undefined /go post", "log:true function function"], "{log:?}");
}

#[test]
fn qa20_form_control_reflections() {
    let log = run_in_html(
        "<html><body><form id=f><input id=i name=n value=v><input id=c type=checkbox checked><input id=bad type=bogus>\
         <button id=b></button><button id=b2 type=button></button>\
         <select id=s><option id=o1>one</option><option id=o2 value=2 selected>two</option></select>\
         <label id=l for=i>L</label><textarea id=ta>t</textarea><input id=d disabled></form></body></html>",
        "var $=function(i){return document.getElementById(i)};\
         console.log($('i').type, $('i').name, $('i').value, $('c').type, $('c').checked, $('bad').type, $('d').disabled, $('i').form===$('f'));\
         console.log($('b').type, $('b2').type, $('s').type, $('s').options.length, $('s').selectedIndex, $('s').value, $('o1').value, $('o2').value, $('o2').selected, $('o2').defaultSelected, $('o2').index);\
         $('s').selectedIndex=0; console.log($('s').value, $('o2').selected);\
         $('i').type='password'; $('i').name='m'; $('c').checked=false; console.log($('i').getAttribute('type'), $('i').name, $('c').checked);\
         console.log($('l').htmlFor, $('l').control===$('i'), $('ta').value, $('ta').type);\
         var o=document.createElement('option'); o.text='three'; $('s').add(o); console.log($('s').length, $('s').options[2].value);",
    );
    assert_eq!(
        log,
        [
            "log:text n v checkbox true text true true",
            "log:submit button select-one 2 1 2 one 2 true true 1",
            "log:one false",
            "log:password m false",
            "log:i true t textarea",
            "log:3 three"
        ],
        "{log:?}"
    );
}

#[test]
fn qa20_map_areas_and_form_global_constructors() {
    let log = run_in_html(
        "<html><body><map id=m name=mm><area id=a1 shape=rect coords='0,0,1,1' href=x><area id=a2></map><img usemap='#mm'></body></html>",
        "var m=document.getElementById('m');\
         console.log(m.areas.length, m.areas[0].shape, m.areas[0].coords, m.name, m.images.length, m instanceof HTMLMapElement, m instanceof HTMLElement, typeof HTMLFormElement);",
    );
    assert_eq!(log, ["log:2 rect 0,0,1,1 mm 1 true true function"], "{log:?}");
}

#[test]
fn qa20_input_value_property_is_separate_from_the_attribute_and_meta_http_equiv() {
    let log = run_in_html(
        "<html><body></body></html>",
        "var i=document.createElement('input'); i.value='test';\
         console.log(i.hasAttribute('value'), i.value, i.defaultValue);\
         var m=document.createElement('meta'); m.setAttribute('http-equiv','x'); console.log(m.httpEquiv, m.hasAttribute('httpEquiv'));",
    );
    assert_eq!(log, ["log:false test ", "log:x false"], "{log:?}");
}

// ---------- QA18b: iframe.contentDocument / contentWindow ----------

#[test]
fn qa18b_iframe_content_document_and_window() {
    let log = run_with_timers(
        "var f=document.createElement('iframe');\
         console.log(f.contentDocument, f.contentWindow);\
         f.onload=function(){ console.log('onload', this===f); };\
         f.addEventListener('load', function(){ console.log('listener'); });\
         document.body.appendChild(f);\
         var d=f.contentDocument; var w=f.contentWindow;\
         console.log(d!==document, d.nodeType, d.documentElement.tagName, d.body.tagName, d.head.tagName,\
           w.document===d, d.defaultView===w, w.parent===window, w.top===window, w.frameElement===f, f.contentDocument===d);\
         var e=d.createElement('p'); d.body.appendChild(e); console.log(d.getElementsByTagName('p').length, document.getElementsByTagName('p').length, e.ownerDocument===d);\
         var n=document.createElement('iframe'); n.src='http://example.org/x.html'; document.body.appendChild(n);\
         console.log(n.contentDocument, n.contentWindow);\
         var u=document.createElement('iframe'); u.setAttribute('src','about:blank'); document.body.appendChild(u);\
         var dd=document.createElement('iframe'); dd.src='data:text/html,hi'; document.body.appendChild(dd);\
         console.log(u.contentDocument.nodeType, dd.contentDocument, f instanceof HTMLIFrameElement);\
         var r=document.createElement('iframe'); document.body.appendChild(r); document.body.removeChild(r); console.log(r.contentDocument);\
         console.log('sync-end');",
    );
    assert_eq!(
        log,
        [
            "log:null null",
            "log:true 9 HTML BODY HEAD true true true true true true",
            "log:1 0 true",
            "log:null null",
            "log:9 null true",
            "log:null",
            "log:sync-end",
            "log:listener",
            "log:onload true",
        ],
        "{log:?}"
    );
}

// ---------- QA18b1 / QA18c: iframe src documents, styles in second documents ----------

#[test]
fn qa18b1_iframe_src_document_is_loaded_and_styled() {
    let mut pages = HashMap::new();
    pages.insert(
        "http://qa18b1.test/dir/empty.html".to_string(),
        "<!doctype html><html><head><style>p{white-space:pre;color:#102030}</style><title>T</title></head><body><p id=a>hi</p></body></html>".to_string(),
    );
    crate::net::set_iframe_pages(pages);
    crate::js::set_page_url(Url::parse("http://qa18b1.test/dir/page.html").ok().as_ref());
    let (_e, _d, log) = events_engine(
        "<html><body><iframe id=f src='empty.html'></iframe><iframe id=g src='missing.html'></iframe>\
         <iframe id=h src='http://other.test/empty.html'></iframe></body></html>",
        "var f=document.getElementById('f'), d=f.contentDocument, w=f.contentWindow;\
         console.log(d!==null, d.title, d.getElementById('a').textContent, d.URL, w.location.href, d.defaultView===w, w.parent===window);\
         var cs=getComputedStyle(d.getElementById('a'));\
         console.log(cs.whiteSpace, cs.color, getComputedStyle(document.body).whiteSpace);\
         console.log(document.getElementById('g').contentDocument, document.getElementById('h').contentDocument);\
         console.log(document.getElementById('a'), document.getElementsByTagName('p').length);\
         var e=d.createElement('div'); d.body.appendChild(e); e.style.whiteSpace='pre-wrap';\
         console.log(getComputedStyle(e).whiteSpace);",
    );
    assert_eq!(
        *log.borrow(),
        [
            "log:true T hi http://qa18b1.test/dir/empty.html http://qa18b1.test/dir/empty.html true true",
            "log:pre rgb(16, 32, 48) normal",
            "log:null null",
            "log:null 0",
            "log:pre-wrap",
        ],
        "{:?}",
        log.borrow()
    );
}

// ---------- QPT1a: a same-origin iframe gets its own V8 realm (R53) ----------

#[test]
fn qpt1a_iframe_window_is_its_own_realm() {
    crate::js::set_page_url(Url::parse("https://qpt1a.test/dir/page.html").ok().as_ref());
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f);\
         var w=f.contentWindow, d=f.contentDocument;\
         console.log(typeof w.String, w.String!==String, w.String('x')==='x', [] instanceof w.Array, w.Array.isArray([]),\
           w.eval('[]') instanceof w.Array, w.Object.getOwnPropertyDescriptor(w, 'String').writable);\
         console.log(typeof w.HTMLElement, typeof w.Navigator, typeof w.SubtleCrypto, typeof w.Object, typeof w.navigator.userAgent,\
           w.HTMLElement!==HTMLElement, String(w), Object.prototype.toString.call(w));\
         console.log(w.document===d, d.defaultView===w, w.document.body instanceof w.HTMLElement, w.document.body instanceof HTMLElement,\
           f.contentWindow===w, f.contentDocument===d, w.window===w, w.self===w, w.frames===w);\
         console.log(w.parent===window, w.top===window, w.frameElement===f, w.opener, w.location.href, w.location.origin,\
           w.document.URL, w.document.readyState, w.document.baseURI, w.isSecureContext);\
         var p=w.document.createElement('p'); p.id='rp'; w.document.body.appendChild(p);\
         console.log(w.document.getElementById('rp')===p, document.getElementById('rp'), document.getElementsByTagName('p').length,\
           w.document.getElementsByTagName('p').length, p.ownerDocument===w.document, p instanceof w.HTMLElement);\
         w.eval(\"document.body.appendChild(document.createElement('section'))\");\
         console.log(document.getElementsByTagName('section').length, d.getElementsByTagName('section').length, w.document.documentElement.outerHTML);\
         console.log(w.eval('typeof __qx'), w.__scratchy_run_global('var __qx = 5;', 'realm-script', false), w.__qx, typeof __qx);\
         var cs=w.getComputedStyle(p); console.log(typeof cs.getPropertyValue('display'), w.document.all.length > 0, typeof w.document.all);\
         var thisOk; w.setTimeout(function(){ thisOk = this === w; console.log('realm timeout', thisOk, w.document === document); }, 5);\
         w.setTimeout('console.log(\"string timer\", document.body.tagName, window.parent === parent)', 1);\
         var c=w.setTimeout(function(){ console.log('cleared ran'); }, 2); w.clearTimeout(c);\
         w.requestIdleCallback(function(){ console.log('realm idle'); });\
         w.requestAnimationFrame(function(t){ console.log('realm raf', typeof t); });\
         w.addEventListener('unhandledrejection', function(e){ console.log('realm unhandledrejection', e.reason.message); e.preventDefault(); });\
         w.eval(\"Promise.reject(new Error('in realm'))\");\
         w.eval(\"Promise.reject(new Error('logged'))\");\
         console.log('sync-end');",
    );
    let rest: Vec<&String> = log.iter().filter(|l| !l.starts_with("log:")).collect();
    let logs: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("log:")).collect();
    assert_eq!(logs[0], "function true true false true true true", "{log:?}");
    assert_eq!(logs[1], "function function function function string true [object Object] [object Object]", "{log:?}");
    assert_eq!(logs[2], "true true true false true true true true true", "{log:?}");
    assert_eq!(
        logs[3],
        "true true true null about:blank https://qpt1a.test about:blank complete https://qpt1a.test/dir/page.html true",
        "{log:?}"
    );
    assert_eq!(logs[4], "true null 0 1 true true", "{log:?}");
    assert_eq!(logs[5], "0 1 <html><head></head><body><p id=\"rp\"></p><section></section></body></html>", "{log:?}");
    assert_eq!(logs[6], "undefined true 5 undefined", "{log:?}");
    assert_eq!(logs[7], "string true undefined", "{log:?}");
    assert_eq!(logs[8], "sync-end", "{log:?}");
    let after: Vec<&str> = logs[9..].to_vec();
    for want in ["realm timeout true false", "string timer BODY true", "realm idle", "realm raf number", "realm unhandledrejection in realm", "realm unhandledrejection logged"] {
        assert!(after.contains(&want), "missing {want:?} in {log:?}");
    }
    assert!(!after.contains(&"cleared ran"), "{log:?}");
    assert!(rest.is_empty(), "{log:?}");
}

#[test]
fn qpt1a_realm_unhandled_rejection_is_logged_and_nested_realms_work() {
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
         w.eval(\"Promise.reject(new TypeError('realm boom'))\");\
         var g=w.document.createElement('iframe'); w.document.body.appendChild(g); var w2=g.contentWindow;\
         console.log(w2!==null, w2!==w, w2.parent===w, w2.top===window, w2.String!==w.String, w2.frameElement===g);\
         w2.setTimeout(function(){ console.log('nested timeout', this===w2); }, 1);\
         var sf=document.createElement('iframe'); document.body.appendChild(sf); /* QPT1b: a src/srcdoc change drops that frame's realm */ var dd=document.createElement('iframe'); dd.src='data:text/html,hi'; document.body.appendChild(dd);\
         console.log(typeof dd.contentWindow.postMessage, dd.contentDocument, sf.srcdoc, (sf.srcdoc='<b>x</b>', sf.getAttribute('srcdoc')));",
    );
    assert!(log.iter().any(|l| l.starts_with("error:Uncaught (in promise) TypeError: realm boom")), "{log:?}");
    assert!(log.contains(&"log:true true true true true true".to_string()), "{log:?}");
    assert!(log.contains(&"log:function null  <b>x</b>".to_string()), "data: iframes are opaque (QPT1-D1): {log:?}");
    assert!(log.contains(&"log:nested timeout true".to_string()), "{log:?}");
}

/// QPT1e: realm tests run a second time on a page engine without the startup snapshot (`Context::new`,
/// natives re-bound by name, preludes replayed): the realm must behave the same.
macro_rules! qpt1e_nosnap_twins {
    ($($name:ident => $twin:ident),* $(,)?) => {
        $(
            #[test]
            fn $twin() {
                crate::js_engine::without_snapshot($name);
            }
        )*
    };
}

qpt1e_nosnap_twins! {
    qpt1a_iframe_window_is_its_own_realm => qpt1e_nosnap_iframe_window_is_its_own_realm,
    qpt1a_realm_unhandled_rejection_is_logged_and_nested_realms_work => qpt1e_nosnap_unhandled_rejection_and_nested_realms,
    qpt1a_realm_fetch_and_xhr_go_through_the_page_queue => qpt1e_nosnap_fetch_and_xhr_through_the_page_queue,
    qpt1b_removal_drops_the_realm_and_cancels_its_timers => qpt1e_nosnap_removal_drops_the_realm,
    qpt1b_src_change_makes_a_new_realm => qpt1e_nosnap_src_change_makes_a_new_realm,
    qpt1b_cap_of_64_realms_logs_once_and_falls_back_to_the_stand_in => qpt1e_nosnap_cap_of_64_realms,
    qpt1b_fifty_create_remove_cycles_return_to_baseline => qpt1e_nosnap_create_remove_cycles,
    qpt1c_one_wrapper_per_node_made_by_the_owner_documents_realm => qpt1e_nosnap_one_wrapper_per_node,
    qpt1c_mutation_observers_see_the_same_wrapper_in_both_realms => qpt1e_nosnap_mutation_observers,
    qpt1d_post_message_between_page_and_realm => qpt1e_nosnap_post_message,
    qpt1f_srcdoc_scripts_run_in_the_realm_in_order_then_dcl_load_and_iframe_load => qpt1e_nosnap_srcdoc_scripts,
    qpt1f_document_open_write_close_runs_the_written_scripts_in_the_realm => qpt1e_nosnap_document_write,
    qpt1f_js_inserted_scripts_run_in_the_realm_of_their_document => qpt1e_nosnap_inserted_scripts,
    qpt1f_custom_elements_upgrade_per_realm => qpt1e_nosnap_custom_elements,
}

#[test]
fn qpt1e_nosnap_realm_has_its_own_builtins_and_no_stand_in_warning() {
    let log = crate::js_engine::without_snapshot(|| {
        run_with_timers(
            "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
             console.log(w.String!==String, w.Object!==Object, w.Array!==Array, w.Promise!==Promise, w.document===f.contentDocument, w.parent===window);",
        )
    });
    assert_eq!(log, ["log:true true true true true true"], "{log:?}");
}

#[test]
fn qpt1a_realm_fetch_and_xhr_go_through_the_page_queue() {
    let fake = QfFake::new(vec![("/a", qf7_ok("A", &[("content-type", "text/plain")])), ("/b", qf7_ok("B", &[]))]);
    let log = qf8_run(
        "https://page.test/dir/page.html",
        &fake,
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
         w.fetch('/a').then(function(r){ console.log('fetch', r instanceof w.Response, r instanceof Response, r.status); return r.text(); })\
           .then(function(t){ console.log('body', t); });\
         var x=new w.XMLHttpRequest(); x.open('GET', 'b'); x.onload=function(){ console.log('xhr', x.status, x.responseText, this===x); }; x.send();",
    );
    assert_eq!(fake.urls(), vec!["https://page.test/a", "https://page.test/dir/b"], "{log:?}");
    for want in ["log:fetch true false 200", "log:body A", "log:xhr 200 B true"] {
        assert!(log.contains(&want.to_string()), "missing {want:?} in {log:?}");
    }
}

#[test]
fn qpt1d3_closed_window_timers_never_fire_and_requests_are_not_sent() {
    let fake = QfFake::new(vec![("/a", qf7_ok("A", &[]))]);
    let log = qf8_run(
        "https://page.test/dir/page.html",
        &fake,
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow; f.remove();\
         console.log('closed', w.closed, typeof w.setTimeout(function(){ console.log('FIRED timeout'); }, 1), typeof w.setInterval(function(){ console.log('FIRED interval'); }, 1));\
         w.requestAnimationFrame(function(){ console.log('FIRED raf'); }); w.requestIdleCallback(function(){ console.log('FIRED idle'); });\
         w.fetch('/a').then(function(){ console.log('FIRED fetch'); }, function(){ console.log('FIRED fetch'); });\
         var x=new w.XMLHttpRequest(); x.open('GET', '/a'); x.onload=x.onerror=function(){ console.log('FIRED xhr'); }; x.send();\
         setTimeout(function(){ console.log('page ok'); }, 50);",
    );
    assert!(fake.urls().is_empty(), "{:?} {log:?}", fake.urls());
    assert!(!log.iter().any(|l| l.contains("FIRED")), "{log:?}");
    assert!(log.contains(&"log:closed true number number".to_string()), "{log:?}");
}

// ---------- QY8: O(1) sibling access ----------

#[test]
fn qy8_walk_counts_and_stays_correct_after_mutations() {
    let log = run_in_html(
        "<div id=p></div>",
        "var p=document.getElementById('p');
         for(var i=0;i<50;i++){var s=document.createElement('span');s.id='s'+i;p.appendChild(s);}
         var w=function(){var o=[];for(var n=p.firstChild;n;n=n.nextSibling)o.push(n.id||'#');return o.join(',');};
         var b=function(){var o=[];for(var n=p.lastChild;n;n=n.previousSibling)o.push(n.id||'#');return o.join(',');};
         var s5=document.getElementById('s5');
         console.log(s5.nextSibling.id+' '+s5.previousSibling.id);
         p.insertBefore(document.createElement('i'),s5);
         console.log(s5.previousSibling.tagName+' '+s5.nextSibling.id);
         p.removeChild(document.getElementById('s6'));
         console.log(s5.nextSibling.id);
         s5.before(document.createTextNode('t'));
         console.log(s5.previousSibling.nodeType+' '+s5.previousElementSibling.tagName+' '+s5.nextElementSibling.id);
         s5.replaceWith(document.createElement('b'));
         console.log(s5.nextSibling+' '+s5.parentNode);
         var c=0;for(var n=p.firstChild;n;n=n.nextSibling)c++;
         console.log(c+' '+(w().split(',').length)+' '+(b().split(',').reverse().join(',')===w()));
         p.innerHTML='<a></a><b></b>';
         console.log(p.firstChild.tagName+p.lastChild.tagName+p.firstChild.nextSibling.tagName+p.lastElementChild.previousElementSibling.tagName);
         var x=document.createElement('div');console.log(x.nextSibling+' '+x.firstChild+' '+x.nextElementSibling);",
    );
    assert_eq!(
        log,
        vec![
            "log:s6 s4",
            "log:I s6",
            "log:s7",
            "log:3 I s7",
            "log:null null",
            "log:51 51 true",
            "log:ABBA",
            "log:null null null",
        ]
    );
}

#[test]
fn qy8_sibling_hint_is_o1_when_walking() {
    use markup5ever_rcdom::Node;
    let kids: Vec<_> = (0..2000).map(|_| Node::new(markup5ever_rcdom::NodeData::Comment { contents: "x".into() })).collect();
    let mut steps = 0;
    let mut hint = 0;
    for (i, k) in kids.iter().enumerate() {
        let got = crate::js_dom::child_index(&kids, k, hint, &mut steps);
        assert_eq!(got, Some(i));
        hint = i + 1; // the native stores the sibling's index when it returns it
    }
    // One probe per step when the hint is right (a scan would be ~2M).
    assert!(steps <= 2 * 2000, "steps = {steps}");
    // A stale hint still gives the right answer (falls back to a scan).
    assert_eq!(crate::js_dom::child_index(&kids, &kids[10], 1999, &mut steps), Some(10));
    assert_eq!(crate::js_dom::child_index(&kids, &kids[10], 99999, &mut steps), Some(10));
}

// ---------- QY11: reflected URL attributes, dataset, attributes, classList ----------

#[test]
fn qy11_url_reflection_is_absolute_and_honours_base() {
    set_page_url(Url::parse("https://page.test/dir/index.html?q=1").ok().as_ref());
    let log = run_in_html(
        "<html><head><base href=https://x.org/d/></head><body><script src=a.js id=s></script><img src=../p.png id=i>\
         <a href='sub/x?y#z' id=a></a><form action=go id=f></form><link rel=stylesheet href=/c.css id=l>\
         <iframe src=about:blank id=fr></iframe><a id=n></a></body></html>",
        "var g=function(i){return document.getElementById(i)};\
         var r=[g('s').src,g('i').src,g('i').currentSrc,g('a').href,g('f').action,g('l').href,g('fr').src,g('n').href,document.baseURI];\
         g('i').src='/z.png'; r.push(g('i').getAttribute('src'), g('i').src);\
         var a=g('a'); r.push(a.origin,a.protocol,a.host,a.hostname,a.port,a.pathname,a.search,a.hash);\
         a.pathname='/new'; r.push(a.href);\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:https://x.org/d/a.js|https://x.org/p.png|https://x.org/p.png|https://x.org/d/sub/x?y#z|https://x.org/d/go|https://x.org/c.css|about:blank||https://x.org/d/|/z.png|https://x.org/z.png|https://x.org|https:|x.org|x.org||/d/sub/x|?y|#z|https://x.org/new?y#z"],
        "{log:?}"
    );
    set_page_url(None);
}

#[test]
fn qy11_url_reflection_without_base_uses_page_url() {
    set_page_url(Url::parse("https://page.test/dir/index.html").ok().as_ref());
    let log = run_in_html(
        "<html><body><script src=a.js id=s></script></body></html>",
        "console.log(document.getElementById('s').src, document.baseURI)",
    );
    assert_eq!(log, ["log:https://page.test/dir/a.js https://page.test/dir/index.html"], "{log:?}");
    set_page_url(None);
}

#[test]
fn qy11_dataset_roundtrip() {
    let log = run_in_html(
        "<html><body><div id=a data-foo-bar=1 data-x=2></div></body></html>",
        "var e=document.getElementById('a'), d=e.dataset, r=[];\
         r.push(d.fooBar, d.x, d.nope, 'fooBar' in d, 'zip' in d);\
         d.newThing=5; r.push(e.getAttribute('data-new-thing'));\
         e.setAttribute('data-late','L'); r.push(d.late);\
         delete d.x; r.push(e.hasAttribute('data-x'), 'x' in d);\
         r.push(Object.keys(d).join(','));\
         var k=[]; for (var p in d) k.push(p); r.push(k.join(','));\
         console.log(r.join('|'));",
    );
    assert_eq!(log, ["log:1|2||true|false|5|L|false|false|fooBar,newThing,late|fooBar,newThing,late"], "{log:?}");
}

#[test]
fn qy11_attributes_and_attribute_names() {
    let log = run_in_html(
        "<html><body><p id=a class='c d' title=t></p><p id=e></p></body></html>",
        "var e=document.getElementById('a'), m=e.attributes, r=[];\
         r.push(m.length, m[0].name, m.item(1).value, m.item(9), m.getNamedItem('title').value, m.title.name, m.id.specified, m.id.ownerElement===e, m[2].nodeName);\
         r.push(e.getAttributeNames().join(','), e.hasAttributes(), document.createElement('i').hasAttributes());\
         var n=[]; for (var x of m) n.push(x.name+'='+x.value); r.push(n.join(';'));\
         m.title.value='T2'; r.push(e.getAttribute('title'), e.getAttributeNode('id').value, e.getAttributeNode('zz'));\
         e.setAttribute('lang','en'); r.push(m.length, e.getAttributeNames().pop());\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:3|id|c d||t|title|true|true|title|id,class,title|true|false|id=a;class=c d;title=t|T2|a||4|lang"],
        "{log:?}"
    );
}

#[test]
fn qy11_classlist_iterable_and_consistent() {
    let log = run_in_html(
        "<html><body><div id=a class='x y'></div></body></html>",
        "var e=document.getElementById('a'), l=e.classList, r=[];\
         r.push([...l].join(','), l.length, l.item(1), l[0], l.value, l.supports('x'), e.classList===l);\
         var f=[]; l.forEach(function(v,i){f.push(i+v)}); r.push(f.join(','));\
         r.push([...l.keys()].join(','), [...l.values()].join(','), [...l.entries()].join(';'));\
         for (var c of l) r.push('c'+c);\
         l.add('z'); r.push(e.className); e.className='q'; r.push(l.value, l.length);\
         l.value='m n'; r.push(e.getAttribute('class'), l.toggle('n'), e.className);\
         console.log(r.join('|'));",
    );
    assert_eq!(
        log,
        ["log:x,y|2|y|x|x y|true|true|0x,1y|0,1|x,y|0,x;1,y|cx|cy|x y z|q|1|m n|false|m"],
        "{log:?}"
    );
}

#[test]
fn qy11_style_string_reflection() {
    let log = run_in_html(
        "<html><body><div id=a></div></body></html>",
        "var e=document.getElementById('a'); e.style.color='red'; e.style.marginTop='1px';\
         console.log(e.getAttribute('style'), '|', e.style.cssText, '|', e.style.color);",
    );
    assert_eq!(log, ["log:color: red; margin-top: 1px; | color: red; margin-top: 1px; | red"], "{log:?}");
}

#[test]
fn qy23_style_assignment_puts_forward_to_css_text() {
    let log = run_in_html(
        "<html><body><div id=a></div></body></html>",
        "'use strict'; var d=document.getElementById('a'); d.style='color: red';\
         console.log(d.getAttribute('style'), '|', d.style.color);\
         d.style=null; console.log(d.getAttribute('style'), '|', d.style.cssText === '');",
    );
    assert_eq!(log, ["log:color: red | red", "log:null | true"], "{log:?}");
}

#[test]
fn qy24_class_list_reads_the_class_attribute_not_class_name() {
    let log = run_in_html(
        "<html><body><x-q id=a class='p q'></x-q></body></html>",
        "'use strict'; class XQ extends HTMLElement { get className() { return undefined; } set className(v) {} }\
         customElements.define('x-q', XQ); var d=document.getElementById('a');\
         d.classList.add('r'); console.log(d.classList.contains('p'), d.classList.toggle('p'), d.getAttribute('class'), d.classList.length, String(d.classList));",
    );
    assert_eq!(log, ["log:true false q r 2 q r"], "{log:?}");
}

#[test]
fn qy12_window_interface_has_its_own_prototype() {
    let out = run_collecting(&[src(
        "console.log(window instanceof Window, window instanceof EventTarget, Window.prototype !== Object.prototype,\
         Object.getPrototypeOf(Object.getPrototypeOf(Window.prototype)) === EventTarget.prototype, ({}) instanceof Window,\
         window.top === window, window.parent === window, window.frames === window, window.opener === null);\
         try { new Window(); } catch (e) { console.log(e.name); }",
    )]);
    assert_eq!(out, vec!["log:true true true true false true true true true", "log:TypeError"]);
}

#[test]
fn qy12_html_element_interfaces_and_instanceof() {
    let out = run_in_html(
        "<html><body><div id=d></div><span></span><a href=x></a><ul><li></li></ul><h3></h3><foo-bar></foo-bar><blink></blink><template></template><video></video></body></html>",
        "var t = function (s) { return document.querySelector(s); };\
         console.log(t('div') instanceof HTMLDivElement, t('span') instanceof HTMLSpanElement, t('a') instanceof HTMLAnchorElement,\
           t('li') instanceof HTMLLIElement, t('ul') instanceof HTMLUListElement, t('h3') instanceof HTMLHeadingElement,\
           document.body instanceof HTMLBodyElement, document.documentElement instanceof HTMLHtmlElement,\
           document.createElement('img') instanceof HTMLImageElement, t('template') instanceof HTMLTemplateElement,\
           t('div') instanceof HTMLElement, t('div') instanceof HTMLSpanElement);\
         console.log(t('blink') instanceof HTMLUnknownElement, t('foo-bar') instanceof HTMLUnknownElement, t('foo-bar') instanceof HTMLElement,\
           t('div') instanceof HTMLUnknownElement, t('div').constructor === HTMLDivElement);\
         var c = [HTMLDivElement, HTMLScriptElement, HTMLStyleElement, HTMLLinkElement, HTMLMetaElement, HTMLTitleElement, HTMLHeadElement, HTMLSlotElement, HTMLParagraphElement, HTMLOListElement, HTMLButtonElement, HTMLInputElement, HTMLFormElement, HTMLLabelElement];\
         var bad = 0; c.forEach(function (C) { try { C(); bad++; } catch (e) { if (!(e instanceof TypeError)) bad++; } try { new C(); bad++; } catch (e) {} });\
         console.log(bad, Object.getPrototypeOf(HTMLDivElement.prototype) === HTMLElement.prototype);",
    );
    assert_eq!(out, vec!["log:true true true true true true true true true true true false", "log:true false true false true", "log:0 true"], "{out:?}");
}

#[test]
fn qy12_svg_interfaces_by_namespace() {
    let out = run_in_html(
        "<html><body><svg id=s><path d='M0 0'></path><g></g></svg></body></html>",
        "var svg = 'http://www.w3.org/2000/svg';\
         var p = document.createElementNS(svg, 'path'), r = document.createElementNS(svg, 'rect'), x = document.createElementNS(svg, 'foo');\
         var s = document.getElementById('s');\
         console.log(s instanceof SVGSVGElement, s instanceof SVGGraphicsElement, s instanceof SVGElement, s instanceof Element, s instanceof HTMLElement,\
           s.firstChild instanceof SVGPathElement, p instanceof SVGGeometryElement, r instanceof SVGRectElement, x instanceof SVGElement, x instanceof SVGGraphicsElement,\
           document.createElement('div') instanceof SVGElement, typeof SVGUseElement, typeof SVGGElement);",
    );
    assert_eq!(out, vec!["log:true true true true false true true true true false false function function"], "{out:?}");
}

#[test]
fn qy12_cdata_pi_range_and_fragment_children() {
    let out = run_in_html(
        "<html><body></body></html>",
        "var f = document.createDocumentFragment(); var a = document.createElement('p'); f.appendChild(document.createTextNode('t')); f.appendChild(a); f.appendChild(document.createElement('i'));\
         console.log(f.children.length, f.childElementCount, f.firstElementChild === a, f.lastElementChild.tagName, Array.from(f.children).length);\
         f.prepend(document.createElement('b')); f.append(document.createElement('u')); console.log(f.children.length, f.firstElementChild.tagName);\
         console.log(typeof CDATASection, typeof ProcessingInstruction, CDATASection.prototype instanceof Text, ProcessingInstruction.prototype instanceof CharacterData, typeof Range);\
         try { new Range(); } catch (e) { console.log(e.name); }\
         try { Selection(); } catch (e) { console.log(e.name); }",
    );
    assert_eq!(out, vec!["log:2 2 true I 2", "log:4 B", "log:function function true true function", "log:TypeError"], "{out:?}");
}

#[test]
fn qy12_event_subclasses_construct_with_init_dicts() {
    let out = run_collecting(&[src(
        "var p = new PointerEvent('pointerdown', { pointerId: 3, pointerType: 'mouse', clientX: 5, bubbles: true });\
         var w = new WheelEvent('wheel', { deltaY: 100 }), h = new HashChangeEvent('hashchange', { oldURL: 'a', newURL: 'b' });\
         var m = new MessageEvent('message', { data: { x: 1 }, origin: 'o' }), pr = new ProgressEvent('progress', { loaded: 2, total: 4, lengthComputable: true });\
         var t = new TouchEvent('touchstart'), s = new StorageEvent('storage', { key: 'k' });\
         console.log(p instanceof MouseEvent, p instanceof Event, p.pointerId, p.pointerType, p.clientX, p.bubbles, w.deltaY, w.deltaX, h.newURL, m.data.x, m.origin, pr.loaded, pr.total, pr.lengthComputable, t.touches.length, s.key, s.oldValue);\
         var n = 0; ['TransitionEvent', 'AnimationEvent', 'PromiseRejectionEvent', 'PopStateEvent', 'ClipboardEvent', 'DragEvent', 'CompositionEvent'].forEach(function (k) { if (new window[k]('x') instanceof Event) n++; });\
         console.log(n, new PopStateEvent('popstate', { state: 7 }).state, new TransitionEvent('transitionend', { propertyName: 'width' }).propertyName);",
    )]);
    assert_eq!(out[0], "log:true true 3 mouse 5 true 100 0 b 1 o 2 4 true 0 k null", "{out:?}");
    assert_eq!(out[1], "log:7 7 width", "{out:?}");
}

#[test]
fn qy12_media_interfaces_are_inert_placeholders() {
    // QM1f: with no backend (libmpv missing / built without `media`) the element API
    // is still the harmless placeholder: one `not implemented` line, play() rejects.
    crate::media_el::set_backend_factory(Some(Rc::new(|_| Box::new(crate::media_el::UnavailableBackend))));
    let out = run_in_html(
        "<html><body><video id=v></video><audio></audio></body></html>",
        "var v = document.getElementById('v');\
         console.log(v instanceof HTMLVideoElement, v instanceof HTMLMediaElement, document.querySelector('audio') instanceof HTMLAudioElement,\
           typeof HTMLSourceElement, typeof HTMLTrackElement, v.canPlayType('video/mp4') === '');\
         v.pause(); v.load();\
         var pr = v.play(); console.log(pr instanceof Promise);\
         pr.catch(function (e) { console.log('rejected', e.name); });",
    );
    crate::media_el::set_backend_factory(None);
    assert!(out.iter().any(|l| l == "log:true true true function function true"), "{out:?}");
    assert_eq!(out.iter().filter(|l| l.starts_with("error:") && l.contains("not implemented: media (libmpv not available)")).count(), 1, "{out:?}");
    // (the rejection itself is delivered from a timer now: see qm1f::js_placeholder_path)
    assert!(out.iter().any(|l| l == "log:true"), "{out:?}");
}

// ---------- QY16b: Node.prototype.isEqualNode / isSameNode ----------

#[test]
fn qy16b_is_equal_node_compares_subtrees() {
    let log = run_in_html(
        "<html><body></body></html>",
        "function mk(attrs, kids) {\
           var e = document.createElement('div');\
           attrs.forEach(function (p) { e.setAttribute(p[0], p[1]); });\
           kids.forEach(function (k) { e.appendChild(k); });\
           return e;\
         }\
         function p(word) {\
           var q = document.createElement('p');\
           q.appendChild(document.createTextNode('hi '));\
           var b = document.createElement(word);\
           b.appendChild(document.createTextNode('there'));\
           q.appendChild(b);\
           return q;\
         }\
         function tree(comment) {\
           return mk([['class', 'x'], ['data-k', '1']], [p('b'), document.createComment(comment || 'c')]);\
         }\
         var a = tree();\
         console.log(a.isEqualNode(tree()), a.isEqualNode(a), a.isEqualNode(a.firstChild));\
         console.log(a.isEqualNode(mk([['class', 'x'], ['data-k', '2']], [p('b'), document.createComment('c')])));\
         console.log(a.isEqualNode(tree('d')), a.isEqualNode(mk([['class', 'x'], ['data-k', '1']], [p('i'), document.createComment('c')])));\
         console.log(a.isEqualNode(mk([['class', 'x'], ['data-k', '1']], [p('b'), document.createComment('c'), document.createElement('span')])));\
         console.log(a.isEqualNode(mk([['class', 'x'], ['data-k', '1']], [p('b')])));\
         console.log(document.createTextNode('x').isEqualNode(document.createTextNode('x')), document.createTextNode('x').isEqualNode(document.createTextNode('y')));\
         console.log(document.createComment('x').isEqualNode(document.createTextNode('x')), document.createElement('a').isEqualNode(document.createElement('b')));\
         console.log(a.isEqualNode(null), a.isEqualNode(undefined), document.createDocumentFragment().isEqualNode(document.createDocumentFragment()));\
         var t1 = document.implementation.createDocumentType('html', '-//A', 's.dtd'), t2 = document.implementation.createDocumentType('html', '-//A', 't.dtd'), t3 = document.implementation.createDocumentType('html', '-//A', 's.dtd');\
         console.log(t1.isEqualNode(t2), t1.isEqualNode(t3));\
         console.log(a.isSameNode(a), a.isSameNode(tree()), a.isSameNode(null));",
    );
    assert_eq!(
        log,
        [
            "log:true true false",
            "log:false",
            "log:false false",
            "log:false",
            "log:false",
            "log:true false",
            "log:false false",
            "log:false false true",
            "log:false true",
            "log:true false false",
        ],
        "{log:?}"
    );
}

#[test]
fn qy16b_is_equal_node_attributes_are_a_set() {
    let log = run_in_html(
        "<html><body></body></html>",
        "function mk(attrs) {\
           var e = document.createElement('a');\
           attrs.forEach(function (p) { e.setAttribute(p[0], p[1]); });\
           return e;\
         }\
         var a = mk([['href', 'x'], ['title', 't']]);\
         console.log(a.isEqualNode(mk([['title', 't'], ['href', 'x']])), a.isEqualNode(mk([['href', 'y'], ['title', 't']])), a.isEqualNode(mk([['href', 'x']])));\
         var e = mk([['href', 'x'], ['title', 't'], ['data-z', '1']]);\
         console.log(e.isEqualNode(mk([['data-z', '1'], ['href', 'x'], ['title', 't']])), a.isEqualNode(e), a.isEqualNode(mk([['href', 'x'], ['alt', 't']])));\
         console.log(document.createElementNS('http://www.w3.org/2000/svg', 'a').isEqualNode(document.createElement('a')));",
    );
    assert_eq!(
        log,
        ["log:true false false", "log:true false false", "log:false"],
        "{log:?}"
    );
}

/// App side: queued requests are started by `js_sync`, results of a
/// superseded page are dropped, current ones reach the page.
fn qf7_app_net(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
    let fake = QfFake::new(vec![("/a", qf7_ok("AAA", &[]))]);
    let doc = parse_html("<html><body></body></html>");
    let mut app = App::new_static(doc.clone(), proxy.clone(), None);
    crate::js_net::set_net_executor(fake.clone());
    let page = Url::parse("https://page.test/").unwrap();
    // Not `qf7_go`: its own `.then` already consumes the body with `r.text()`, so a second
    // `r.text()` here rejects with "body already used" and `__done` is never set (QFLK3).
    let code = "var h = __net_request({method:'GET', url:'https://page.test/a', headers:[], body:null}); \
                h.promise.then(function(r){ return r.text(); }).then(function(t){ window.__done = t; });";
    app.js = run_page_scripts(&doc, &[src(code)], Some(&page));
    app.js_sync();
    for _ in 0..200 {
        if !fake.urls().is_empty() {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(fake.urls(), vec!["https://page.test/a"], "js_sync started the request on a thread");
    let done = |app: &App| app.js.as_ref().unwrap().eval_value::<String>("t", "String(window.__done)");
    let id = app.js.as_ref().unwrap().eval_value::<f64>("t", "h.id").unwrap() as u64;
    // A stale generation is dropped: the promise stays pending.
    app.handle_js_net(app.nav_generation + 1, id, qf7_ok("STALE", &[]), std::time::Instant::now());
    assert_eq!(done(&app).as_deref(), Some("undefined"));
    crate::perf::start();
    let stamped = std::time::Instant::now() - std::time::Duration::from_millis(250);
    app.handle_js_net(app.nav_generation, id, qf7_ok("AAA", &[]), stamped);
    let (n, _, _, max) = crate::perf::take_window_net();
    crate::perf::stop();
    assert_eq!(n, 1, "QOBS6: one delivery recorded (the stale one is not)");
    assert!(max >= 250_000_000, "QOBS6: a result stamped 250 ms ago waited >= 250 ms, got {max} ns");
    assert_eq!(done(&app).as_deref(), Some("AAA"));
}

/// App side (needs a display like the other App tests): `start_navigation`
/// (a link) sends the current page as the initiator, `:go` does not.
fn qf14b_app_initiator(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
    let path = unique_test_socket_path("qf14b-app");
    let seen = qf14b_fake_wire(&path);
    let doc = parse_html("<html><body></body></html>");
    let mut app = App::new_static(doc, proxy.clone(), Some(path.clone()));
    app.base_url = Some(Url::parse("https://news.example/a").unwrap());
    app.start_navigation("https://shop.example/x");
    let reqs = qf14b_wait(&seen, 1);
    assert!(qf14b_has(&reqs[0], "X-Wire-Initiator-Site: https://news.example"), "{reqs:?}");
    app.dispatch_command("go", &["https://typed.example/y"]);
    let reqs = qf14b_wait(&seen, 2);
    let typed = reqs.iter().find(|r| r.contains("typed.example")).expect("typed request");
    assert!(qf14b_has(typed, "X-Wire-Nav: 1") && !typed.contains("Initiator"), "{typed}");
    let _ = std::fs::remove_file(&path);
}

/// QNAV9, App side: every kind of `href` on a page opened from a local file
/// (base `file:///...`) is resolved against the file base and dispatched to wire
/// (a `#frag` is same-document: no request at all).
fn qnav9_app_file_links(proxy: &winit::event_loop::EventLoopProxy<AppEvent>) {
    let path = unique_test_socket_path("qnav9-app");
    let seen = qf14b_fake_wire(&path);
    let mut app = App::new_static(parse_html("<html><body></body></html>"), proxy.clone(), Some(path.clone()));
    for (n, (href, want)) in [
        ("other.html", "GET file:///home/u/site/pages/other.html "),
        ("../x.html", "GET file:///home/u/site/x.html "),
        ("file:///tmp/abs.html", "GET file:///tmp/abs.html "),
        ("https://example.org/p", "GET https://example.org/p "),
    ]
    .into_iter()
    .enumerate()
    {
        // Each navigation resolves against the original page.
        app.base_url = Some(Url::parse("file:///home/u/site/pages/index.html").unwrap());
        app.start_navigation(href);
        let reqs = qf14b_wait(&seen, n + 1);
        assert!(reqs.iter().any(|r| r.starts_with(want)), "{href}: wanted {want:?}, got {reqs:?}");
    }
    app.base_url = Some(Url::parse("file:///home/u/site/pages/index.html").unwrap());
    let before = seen.lock().unwrap().len();
    app.start_navigation("#frag");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(seen.lock().unwrap().len(), before, "a fragment link is not a fetch");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn qan1h3_js_class_writes_are_seen_by_matches() {
    let (log, _body) = mutate(
        "<html><body><p id=p class=x>t</p></body></html>",
        "var e=document.getElementById('p'),r=[e.matches('.on')];\
         e.classList.toggle('on');r.push(e.matches('.on'));\
         e.classList.replace('on','off');r.push(e.matches('.on'),e.matches('.off'));\
         e.className='on';r.push(e.matches('.on'));\
         e.setAttribute('class','  x  ');r.push(e.matches('.on'),e.matches('.x'));\
         e.removeAttribute('class');r.push(e.matches('.x'));console.log(r.join());",
    );
    assert_eq!(log, ["log:false,true,false,true,true,false,true,false"], "{log:?}");
}

// ---------- QPT1b: realm lifetime (R53 D6) ----------

#[test]
fn qpt1b_removal_drops_the_realm_and_cancels_its_timers() {
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow, S=w.String;\
         w.setTimeout(function(){ console.log('FIRED timeout'); }, 10);\
         w.setInterval(function(){ console.log('FIRED interval'); }, 10);\
         w.requestAnimationFrame(function(){ console.log('FIRED raf'); });\
         w.requestIdleCallback(function(){ console.log('FIRED idle'); });\
         setTimeout(function(){ console.log('page timer ok'); }, 10);\
         console.log('live', __scratchy_realm_count(), f.contentWindow===w);\
         f.remove();\
         console.log('removed', __scratchy_realm_count(), f.contentWindow, f.contentDocument);\
         document.body.appendChild(f); var w2=f.contentWindow;\
         console.log('again', __scratchy_realm_count(), w2!==null, w2!==w, w2.String!==S, w2.String!==String, w2.parent===window);\
         var box=document.createElement('div'); var g=document.createElement('iframe'); box.appendChild(g); document.body.appendChild(box);\
         var w3=g.contentWindow; w3.setTimeout(function(){ console.log('FIRED nested-in-div'); }, 10);\
         var w4=w3.document.createElement('iframe'); w3.document.body.appendChild(w4); w4.contentWindow.setTimeout(function(){ console.log('FIRED deep'); }, 10);\
         console.log('before', __scratchy_realm_count());\
         box.innerHTML='';\
         console.log('after', __scratchy_realm_count(), g.contentWindow);",
    );
    assert!(!log.iter().any(|l| l.contains("FIRED")), "{log:?}");
    for want in ["log:live 1 true", "log:removed 0 null null", "log:again 1 true true true true true", "log:before 3", "log:after 1 null", "log:page timer ok"] {
        assert!(log.contains(&want.to_string()), "missing {want:?} in {log:?}");
    }
}

#[test]
fn qpt1b_src_change_makes_a_new_realm() {
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow, S=w.String;\
         w.setTimeout(function(){ console.log('FIRED old'); }, 5);\
         f.setAttribute('src', 'about:blank');\
         var n=f.contentWindow;\
         console.log(n!==w, n.String!==S, __scratchy_realm_count());\
         f.src='about:blank#x'; var m=f.contentWindow; console.log(m!==n, __scratchy_realm_count());\
         f.removeAttribute('src'); console.log(f.contentWindow!==m, __scratchy_realm_count());",
    );
    assert!(!log.iter().any(|l| l.contains("FIRED")), "{log:?}");
    for want in ["log:true true 1", "log:true 1", "log:true 1"] {
        assert!(log.contains(&want.to_string()), "missing {want:?} in {log:?}");
    }
}

#[test]
fn qpt1b_cap_of_64_realms_logs_once_and_falls_back_to_the_stand_in() {
    let log = run_with_timers(
        "var ws=[];\
         for (var i=0;i<70;i++){ var f=document.createElement('iframe'); document.body.appendChild(f); ws.push(f.contentWindow); }\
         console.log('count', __scratchy_realm_count(), typeof ws[0].String, typeof ws[63].String, typeof ws[64].String, ws[69].document!==null);\
         var fs=document.getElementsByTagName('iframe'); fs[0].remove();\
         var h=document.createElement('iframe'); document.body.appendChild(h); var hs=typeof h.contentWindow.String; console.log('after remove', __scratchy_realm_count(), hs);",
    );
    assert!(log.contains(&"log:count 64 function function undefined true".to_string()), "{log:?}");
    assert!(log.contains(&"log:after remove 64 function".to_string()), "{log:?}");
    assert_eq!(log.iter().filter(|l| l.contains("too many iframe windows")).count(), 1, "{log:?}");
}

#[test]
fn qpt1b_fifty_create_remove_cycles_return_to_baseline() {
    let log = run_with_timers(
        "function cycle(){ var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
           w.setTimeout(function(){}, 1000); w.eval('var a=[]; for(var i=0;i<1000;i++) a.push({i:i});');\
           var g=w.document.createElement('iframe'); w.document.body.appendChild(g); g.contentWindow.setInterval(function(){}, 50);\
           f.remove(); }\
         for (var i=0;i<10;i++) cycle();\
         __scratchy_heap_gc(); var h0=__scratchy_heap_gc(), r0=__scratchy_realm_count();\
         for (var i=0;i<50;i++) cycle();\
         var h1=__scratchy_heap_gc(), r1=__scratchy_realm_count();\
         console.log('realms', r0, r1, 'heapdelta', h1-h0);",
    );
    let l = log.iter().find(|l| l.starts_with("log:realms")).unwrap_or_else(|| panic!("{log:?}"));
    let p: Vec<&str> = l.split(' ').collect();
    assert_eq!((p[1], p[2]), ("0", "0"), "{log:?}");
    let delta: f64 = p[4].parse().unwrap();
    // QFLK5: CI measured 2.4 MB of residue (allocator/GC noise) against the old 1 MB limit. A leaked
    // realm costs ~0.65 MB, so 50 leaks are ~32 MB: 8 MB still catches a leak of even a fifth of them.
    assert!(delta < 8.0 * 1_048_576.0, "heap grew by {delta} bytes over 50 cycles (0.65 MB per leaked realm): {log:?}");
}

#[test]
fn qpt1b_removal_cancels_pending_network_callbacks() {
    let fake = QfFake::new(vec![("/a", qf7_ok("A", &[]))]);
    let log = qf8_run(
        "https://page.test/dir/page.html",
        &fake,
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
         w.fetch('/a').then(function(){ console.log('FIRED fetch'); }, function(){ console.log('FIRED fetch-err'); });\
         f.remove(); console.log('removed', __scratchy_realm_count());",
    );
    assert!(!log.iter().any(|l| l.contains("FIRED")), "{log:?}");
    assert!(log.contains(&"log:removed 0".to_string()), "{log:?}");
}

#[test]
fn qpt1b_navigation_drops_all_realms() {
    // A page navigation drops its engine; the isolate (and so every realm in it) goes with it.
    let r = run_with_timers(
        "for (var i=0;i<5;i++){ var f=document.createElement('iframe'); document.body.appendChild(f); f.contentWindow; }\
         console.log('live', __scratchy_realm_count());",
    );
    assert!(r.contains(&"log:live 5".to_string()), "{r:?}");
    let r2 = run_with_timers("console.log('next page', __scratchy_realm_count());");
    assert!(r2.contains(&"log:next page 0".to_string()), "{r2:?}");
}

// ---------- QPT1c: one wrapper per node across realms (R53 D7) ----------

#[test]
fn qpt1c_one_wrapper_per_node_made_by_the_owner_documents_realm() {
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
         var kids=Object.getOwnPropertyDescriptor(Node.prototype,'childNodes').get, first=Object.getOwnPropertyDescriptor(Node.prototype,'firstChild').get;\
         var b=f.contentDocument.body;\
         console.log('a', b===w.document.body, b instanceof w.HTMLElement, b instanceof HTMLElement, first.call(w.document.documentElement)===w.document.head);\
         w.document.body.innerHTML='<div id=a><span>x</span><i><em>y</em></i></div>'; /* em: never wrapped in the realm */\
         var span=kids.call(kids.call(w.document.body)[0])[0];\
         console.log('b', span===w.document.querySelector('span'), span instanceof w.HTMLSpanElement, span instanceof HTMLElement, span.ownerDocument===w.document);\
         var d=w.document.getElementById('a'); document.body.appendChild(d);\
         var em=document.querySelector('em');\
         console.log('c', document.body.lastChild===d, document.getElementById('a')===d, d instanceof w.HTMLDivElement, d instanceof HTMLElement,\
           d.ownerDocument===document, d.firstChild===span, em instanceof HTMLElement, em instanceof w.HTMLElement, w.document.getElementById('a'));\
         d.remove(); console.log('d', d.ownerDocument===document, span.ownerDocument===document);\
         var q=w.document.createElement('q'); console.log('e', q.ownerDocument===w.document, first.call(q));\
         document.adoptNode(q); console.log('f', q.ownerDocument===document, q instanceof w.HTMLElement);\
         document.body.appendChild(q); console.log('g', document.body.lastChild===q, q.parentNode===document.body);\
         var pn=document.createElement('b'); w.document.adoptNode(pn); console.log('h', pn.ownerDocument===w.document, pn instanceof HTMLElement);\
         w.document.body.appendChild(pn); console.log('i', w.document.body.lastChild===pn, pn.ownerDocument===w.document, pn instanceof w.HTMLElement);\
         var t=w.document.createElement('p'); t.innerHTML='<a>1</a>'; console.log('j', first.call(t) instanceof w.HTMLAnchorElement, first.call(t)===t.firstChild);",
    );
    let logs: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("log:")).collect();
    assert_eq!(
        logs,
        [
            "a true true false true",
            "b true true false true",
            "c true true true false true true true false null",
            "d true true",
            "e true null",
            "f true true",
            "g true true",
            "h true true",
            "i true true false",
            "j true true",
        ],
        "{log:?}"
    );
}

#[test]
fn qpt1c_mutation_observers_see_the_same_wrapper_in_both_realms() {
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
         new MutationObserver(function(r){ console.log('page mo', r[0].addedNodes[0]===w.__n, r[0].addedNodes[0] instanceof w.HTMLElement); }).observe(document.body,{childList:true});\
         w.eval(\"var n=document.createElement('i'); window.__n=n; parent.document.body.appendChild(n);\");\
         w.eval(\"new MutationObserver(function(r){ parent.__rec=r[0].addedNodes[0]; }).observe(document.body,{childList:true})\");\
         var pe=document.createElement('u'); f.contentDocument.body.appendChild(pe);\
         w.setTimeout(function(){ console.log('realm mo', __rec===pe, __rec instanceof HTMLElement, __rec.ownerDocument===w.document); }, 1);",
    );
    assert!(log.contains(&"log:page mo true true".to_string()), "{log:?}");
    assert!(log.contains(&"log:realm mo true true true".to_string()), "{log:?}");
}

#[test]
fn qpt1c2_page_observer_sees_mutations_made_by_realm_code_on_the_iframe_document() {
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow, d=f.contentDocument;\
         var b=d.body, t=d.createTextNode('a'); b.appendChild(t);\
         var got=[]; new MutationObserver(function(r){ r.forEach(function(x){ got.push(x.type+':'+(x instanceof MutationRecord)+':'+(x instanceof w.MutationRecord)+':'+(x.target===b||x.target===t)); }); }).observe(b,{childList:true,attributes:true,characterData:true,subtree:true});\
         w.eval(\"var b=document.body; b.appendChild(document.createElement('i')); b.setAttribute('x','1'); b.firstChild.data='zz';\");\
         setTimeout(function(){ console.log('page', got.join(',')); }, 5);",
    );
    let l = log.iter().find(|l| l.starts_with("log:page")).unwrap_or_else(|| panic!("{log:?}"));
    for want in ["childList:true:false:true", "attributes:true:false:true", "characterData:true:false:true"] {
        assert!(l.contains(want), "missing {want} in {l} {log:?}");
    }
}

#[test]
fn qpt1c2_realm_observer_sees_page_mutations_with_its_own_record_prototype() {
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
         var t=document.createTextNode('a'); document.body.appendChild(t);\
         w.__got=[]; w.eval(\"var pb=parent.document.body; new MutationObserver(function(r){ r.forEach(function(x){ __got.push(x.type+':'+(x instanceof MutationRecord)+':'+(x instanceof parent.MutationRecord)+':'+(x.target instanceof parent.Node)); }); }).observe(pb,{childList:true,attributes:true,characterData:true,subtree:true});\");\
         document.body.appendChild(document.createElement('u')); document.body.setAttribute('y','2'); t.data='bb';\
         w.setTimeout(function(){ console.log('realm', w.__got.join(',')); }, 5);",
    );
    let l = log.iter().find(|l| l.starts_with("log:realm")).unwrap_or_else(|| panic!("{log:?}"));
    for want in ["childList:true:false:true", "attributes:true:false:true", "characterData:true:false:true"] {
        assert!(l.contains(want), "missing {want} in {l} {log:?}");
    }
}

#[test]
fn qpt1c2_dropped_realm_observers_stop() {
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
         w.eval(\"new MutationObserver(function(){ parent.console.log('FIRED realm mo'); }).observe(parent.document.body,{childList:true,attributes:true});\");\
         var n=0; new MutationObserver(function(r){ n+=r.length; }).observe(document.body,{childList:true});\
         f.remove();\
         document.body.appendChild(document.createElement('p'));\
         setTimeout(function(){ console.log('page', n, __scratchy_realm_count()); }, 5);",
    );
    assert!(!log.iter().any(|l| l.contains("FIRED")), "{log:?}");
    assert!(log.contains(&"log:page 2 0".to_string()), "{log:?}");
}

#[test]
fn qpt1c_dropped_realm_leaves_no_wrappers_behind() {
    let log = run_with_timers(
        "var kids=Object.getOwnPropertyDescriptor(Node.prototype,'childNodes').get, kept=[];\
         function cycle(keep){ var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
           w.document.body.innerHTML=new Array(200).join('<div><p>x</p></div>');\
           kids.call(w.document.body).forEach(function(n){ kids.call(n); });\
           var lone=w.document.createElement('section'); lone.innerHTML='<b>1</b>'; kids.call(lone);\
           if (keep) { var m=w.document.body.firstChild; document.body.appendChild(m); kept.push(m); }\
           f.remove(); }\
         for (var i=0;i<3;i++) cycle(false);\
         var h0=__scratchy_heap_gc();\
         for (var i=0;i<30;i++) cycle(false);\
         var h1=__scratchy_heap_gc();\
         cycle(true); var m=kept[0];\
         console.log('heapdelta', h1-h0, 'realms', __scratchy_realm_count());\
         console.log('kept', document.body.lastChild===m, m.firstChild===m.firstChild, m.ownerDocument===document, m instanceof HTMLElement);",
    );
    let l = log.iter().find(|l| l.starts_with("log:heapdelta")).unwrap_or_else(|| panic!("{log:?}"));
    let p: Vec<&str> = l.split(' ').collect();
    assert_eq!(p[3], "0", "{log:?}");
    let delta: f64 = p[1].parse().unwrap();
    assert!(delta < 1_048_576.0, "heap grew by {delta} bytes over 30 realms with page-made wrappers: {log:?}");
    assert!(log.contains(&"log:kept true true true false".to_string()), "a moved node keeps its wrapper: {log:?}");
}

// ---------- QPT1d: WindowProxy surface of same-origin iframe realms (R53 D8) ----------

#[test]
fn qpt1d_frames_length_indices_and_names() {
    crate::js::set_page_url(Url::parse("https://qpt1d.test/p.html").ok().as_ref());
    let log = run_with_timers(
        "console.log('n0', window.length, window.frames===window, window[0], window.name==='', window.closed);\
         var a=document.createElement('iframe'); a.name='fa'; document.body.appendChild(a);\
         var b=document.createElement('iframe'); b.name='fb'; document.body.appendChild(b);\
         var c=document.createElement('iframe'); document.body.appendChild(c);\
         console.log('n3', length, window[0]===a.contentWindow, window[1]===b.contentWindow, window[2]===c.contentWindow, window[3],\
           window.fa===a.contentWindow, window.fb===b.contentWindow, frames[1]===b.contentWindow, a.contentWindow.name, window.frames.length);\
         var f0=document.createElement('iframe'); document.body.insertBefore(f0, a);\
         console.log('order', window.length, window[0]===f0.contentWindow, window[1]===a.contentWindow, window[3]===c.contentWindow);\
         f0.remove(); a.remove();\
         console.log('removed', window.length, window[0]===b.contentWindow, window[1]===c.contentWindow, window[2], window.fa, window.fb===b.contentWindow);\
         b.setAttribute('name','renamed'); console.log('rename', window.fb, window.renamed===b.contentWindow);\
         b.contentWindow.name='inner'; console.log('setname', window.renamed, window.inner===b.contentWindow, b.contentWindow.name);\
         var n=document.createElement('iframe'); n.src='https://other.test/x'; document.body.appendChild(n);\
         console.log('net', window.length);\
         var cw=c.contentWindow; c.remove(); b.remove();\
         console.log('none', window.length, window[0], window.inner, cw.length);\
         var d=document.createElement('div'); d.innerHTML='<iframe name=\"hh\"></iframe>'; document.body.appendChild(d);\
         console.log('html', window.length, window[0]===d.firstChild.contentWindow, window.hh===d.firstChild.contentWindow);\
         var nest=d.firstChild.contentWindow; nest.document.body.appendChild(nest.document.createElement('iframe'));\
         console.log('nested', nest.length, window.length, nest[0]===nest.document.querySelector('iframe').contentWindow);",
    );
    let logs: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("log:")).collect();
    assert_eq!(logs[0], "n0 0 true undefined true false", "{log:?}");
    assert_eq!(logs[1], "n3 3 true true true undefined true true true fa 3", "{log:?}");
    assert_eq!(logs[2], "order 4 true true true", "{log:?}");
    assert_eq!(logs[3], "removed 2 true true undefined undefined true", "{log:?}");
    assert_eq!(logs[4], "rename undefined true", "{log:?}");
    assert_eq!(logs[5], "setname undefined true inner", "{log:?}");
    assert_eq!(logs[6], "net 2", "{log:?}");
    assert_eq!(logs[7], "none 0 undefined undefined 0", "{log:?}");
    assert_eq!(logs[8], "html 1 true true", "{log:?}");
    assert_eq!(logs[9], "nested 1 1 true", "{log:?}");
}

#[test]
fn qpt1d_post_message_between_page_and_realm() {
    crate::js::set_page_url(Url::parse("https://qpt1d.test/p.html").ok().as_ref());
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
         window.addEventListener('message', function(e){ console.log('page got', JSON.stringify(e.data), e.origin, e.source===w, e.data instanceof Object, e.data instanceof w.Object); });\
         w.addEventListener('message', function(e){ console.log('realm got', JSON.stringify(e.data), e.origin, e.source===window, e.data instanceof w.Object, e.data instanceof Object, e instanceof w.MessageEvent); });\
         w.postMessage({a:1}, '*'); w.postMessage({b:2}, '/'); w.postMessage({c:3}, 'https://qpt1d.test'); w.postMessage({d:4}, 'https://evil.test');\
         __scratchy_enter(w, function(){ w.eval(\"parent.postMessage({up:1}, '*'); parent.postMessage({no:1}, 'https://evil.test'); parent.postMessage({up:2}, location.origin)\"); });\
         console.log('sync-end');",
    );
    let logs: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("log:")).collect();
    assert_eq!(logs[0], "sync-end", "{log:?}");
    let o = "https://qpt1d.test";
    let rest: Vec<&str> = logs[1..].to_vec();
    assert_eq!(rest.len(), 5, "{log:?}");
    for want in [
        format!("realm got {{\"a\":1}} {o} true true false true"),
        format!("realm got {{\"b\":2}} {o} true true false true"),
        format!("realm got {{\"c\":3}} {o} true true false true"),
        format!("page got {{\"up\":1}} {o} true true false"),
        format!("page got {{\"up\":2}} {o} true true false"),
    ] {
        assert!(rest.contains(&want.as_str()), "missing {want:?} in {log:?}");
    }
}

#[test]
fn qpt1d_closed_after_removal() {
    crate::js::set_page_url(Url::parse("https://qpt1d.test/p.html").ok().as_ref());
    let log = run_with_timers(
        "var f=document.createElement('iframe'); document.body.appendChild(f); var w=f.contentWindow;\
         var got=0; w.addEventListener('message', function(){ got++; });\
         console.log('open', w.closed, window.closed);\
         f.remove();\
         w.postMessage('late','*');\
         setTimeout(function(){ console.log('after', w.closed, got, f.contentWindow); }, 20);",
    );
    let logs: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("log:")).collect();
    assert_eq!(logs[0], "open false false", "{log:?}");
    assert_eq!(logs[1], "after true 0 null", "{log:?}");
}

// ---------- QPT1f: scripts in same-origin iframe realms (R53 D9) ----------

#[test]
fn qpt1f_srcdoc_scripts_run_in_the_realm_in_order_then_dcl_load_and_iframe_load() {
    crate::js::set_page_url(Url::parse("https://qpt1f.test/dir/p.html").ok().as_ref());
    let log = run_with_timers(
        "var order=[], srcs=[];\
         addEventListener('message', function(e){ srcs.push(e.source===f.contentWindow, e.data); });\
         var f=document.createElement('iframe');\
         f.srcdoc=\"<p id=a>A</p><script>parent.order.push(['s1', window!==parent, String!==parent.String, document.getElementById('a').textContent, parent.x,\
           location.href, frameElement===parent.f, document.readyState, document.currentScript.tagName, document.baseURI].join());\
           document.addEventListener('DOMContentLoaded', function(){ parent.order.push('dcl ' + document.readyState); });\
           addEventListener('load', function(){ parent.order.push('load ' + document.readyState); });\
           parent.postMessage('hi', '*');</script>\
           <script>parent.order.push('s2 ' + typeof S1); var S1 = 1;</script><script>parent.order.push('s3 ' + S1);</script>\";\
         f.addEventListener('load', function(){ order.push('iframe load ' + f.contentDocument.readyState); });\
         document.body.appendChild(f); window.x=7; order.push('sync');\
         setTimeout(function(){ console.log(order.join(' | ')); console.log(srcs.join()); }, 50);",
    );
    let logs: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("log:")).collect();
    assert_eq!(
        logs,
        [
            "sync | s1,true,true,A,7,about:srcdoc,true,loading,SCRIPT,https://qpt1f.test/dir/p.html | s2 undefined | s3 1 | dcl interactive | load complete | iframe load complete",
            "true,hi"
        ],
        "{log:?}"
    );
}

/// QPT1-D1: a `data:` iframe has an opaque origin (Chrome): the parent gets no document and only a
/// cross-origin WindowProxy; the frame's own scripts run in its realm and cannot reach the parent's DOM.
#[test]
fn qpt1_d1_data_iframe_is_opaque_to_the_parent_and_its_scripts_run() {
    crate::js::set_page_url(Url::parse("https://qpt1d1.test/p.html").ok().as_ref());
    let log = run_with_timers(
        "var seen=[];\
         addEventListener('message', function(e){ seen.push(e.data + (e.source===f.contentWindow ? '@frame' : '@other') + ' ' + e.origin); });\
         var f=document.createElement('iframe');\
         f.src='data:text/html,<p id=a>A</p><script>parent.postMessage(\"run \"+location.origin+\" \"+(frameElement===null)+\" \"+document.getElementById(\"a\").textContent+\" \"+(window.parent===window.top),\"*\");\
try{parent.document;parent.postMessage(\"leak\",\"*\")}catch(e){parent.postMessage(\"blocked \"+e.name,\"*\")}\
try{top.document.body;parent.postMessage(\"leak top\",\"*\")}catch(e){parent.postMessage(\"blocked top \"+e.name,\"*\")}</script>';\
         document.body.appendChild(f);\
         var w=f.contentWindow, r=[];\
         r.push(f.contentDocument, w!==null, typeof w.postMessage, w.window===w, w.self===w, w.parent===window, w.top===window, w.length, w.closed, w.opener);\
         ['document','location.href','String','a','name'].forEach(function(k){ try { r.push(k+' '+k.split('.').reduce(function(o,p){ return o[p]; }, w)); } catch(e) { r.push(k+' '+e.name); } });\
         try { w.foo=1; r.push('set ok'); } catch(e) { r.push('set '+e.name); }\
         try { r.push(String(w.location)); } catch(e) { r.push('loc '+e.name); }\
         try { r.push(Object.keys(w).length > 0, 'x' in w, 'postMessage' in w); } catch(e) { r.push('keys '+e.name); }\
         console.log(r.join('|'));\
         setTimeout(function(){ console.log(seen.join(' ; ')); }, 50);",
    );
    let logs: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("log:")).collect();
    assert_eq!(logs.len(), 2, "{log:?}");
    assert_eq!(
        logs[0],
        "|true|function|true|true|true|true|0|false||document SecurityError|location.href SecurityError|String SecurityError|a SecurityError|name SecurityError|set SecurityError|loc SecurityError|false|false|true",
        "{log:?}"
    );
    assert_eq!(
        logs[1],
        "run null true A true@frame null ; blocked SecurityError@frame null ; blocked top SecurityError@frame null",
        "{log:?}"
    );
}

/// QPT1-D1: the same-origin kinds keep a readable document and the page's origin.
#[test]
fn qpt1_d1_about_blank_and_srcdoc_iframes_stay_same_origin() {
    crate::js::set_page_url(Url::parse("https://qpt1d1.test/p.html").ok().as_ref());
    let log = run_with_timers(
        "var a=document.createElement('iframe'), s=document.createElement('iframe'); s.srcdoc='<p id=q>Q</p>';\
         document.body.appendChild(a); document.body.appendChild(s);\
         setTimeout(function(){ console.log(a.contentDocument.nodeType, a.contentWindow.location.origin, s.contentDocument.getElementById('q').textContent, s.contentWindow.location.origin, typeof a.contentWindow.String, a.contentWindow.parent===window); }, 20);",
    );
    assert!(log.contains(&"log:9 https://qpt1d1.test Q https://qpt1d1.test function true".to_string()), "{log:?}");
}

#[test]
fn qpt1f_page_markup_srcdoc_loads_before_the_page_load_event() {
    let log = run_html_with_timers(
        "<body><iframe id=f srcdoc=\"<script>parent.ev.push('f script ' + (window !== parent)); \
         document.addEventListener('DOMContentLoaded', function(){ parent.ev.push('f dcl'); }); \
         addEventListener('load', function(){ parent.ev.push('f load'); });</script>\"></iframe></body>",
        "var ev=[]; document.addEventListener('DOMContentLoaded', function(){ ev.push('page dcl'); });\
         addEventListener('load', function(){ ev.push('page load'); });\
         document.getElementById('f').addEventListener('load', function(){ ev.push('iframe load'); });\
         setTimeout(function(){ console.log(ev.join(' | ')); }, 50);",
    );
    assert_eq!(log, ["log:page dcl | f script true | f dcl | f load | iframe load | page load"], "{log:?}");
}

#[test]
fn qpt1f_document_open_write_close_runs_the_written_scripts_in_the_realm() {
    let log = run_with_timers(
        "var g=document.createElement('iframe'); document.body.appendChild(g); var gw=g.contentWindow, d=g.contentDocument, loads=[];\
         setTimeout(function(){\
           g.addEventListener('load', function(){ loads.push('iframe load'); });\
           gw.addEventListener('load', function(){ loads.push('window load'); });\
           console.log(d.open()===d, d.readyState, d.body);\
           d.write('<body><p>x</p><script>parent.wr=[window===parent.gw, document===parent.d, document.body.tagName, document.readyState].join(); document.write(\"<i id=w>w</i>\");</scr' + 'ipt></body>');\
           console.log(typeof wr);\
           d.close();\
           console.log(wr, d.getElementById('w').textContent, d.body.lastElementChild.tagName, d.readyState, loads.join());\
           gw.eval(\"document.open(); document.write('<script>parent.r2 = location.href + \\\" \\\" + (window === parent.gw)</scr' + 'ipt>'); document.close();\");\
           console.log(r2, d.getElementsByTagName('p').length);\
           d.write('<script>parent.r3 = 1</scr' + 'ipt>'); d.close(); console.log(r3);\
         }, 5);",
    );
    let logs: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("log:")).collect();
    assert_eq!(
        logs,
        ["true loading null", "undefined", "true,true,BODY,loading w I complete window load,iframe load", "about:blank true 0", "1"],
        "{log:?}"
    );
}

#[test]
fn qpt1f_js_inserted_scripts_run_in_the_realm_of_their_document() {
    let log = run_with_timers(
        "var h=document.createElement('iframe'); document.body.appendChild(h); var hw=h.contentWindow;\
         var s1=document.createElement('script'); s1.textContent='parent.ins1=[window===parent.hw, String===parent.hw.String, document.currentScript===parent.s1].join()';\
         hw.document.body.appendChild(s1);\
         var s2=hw.document.createElement('script'); s2.text='parent.ins2=(window===parent.hw)'; hw.document.head.appendChild(s2);\
         var s3=hw.document.createElement('script'); s3.text='var ins3=(String===top.String)'; document.body.appendChild(s3);\
         console.log(ins1, ins2, ins3);\
         var r=document.createElement('iframe'); document.body.appendChild(r); var rw=r.contentWindow, rs=rw.document.createElement('script');\
         r.remove(); rs.text='parent.RAN1=1'; rw.document.body.appendChild(rs);\
         var q=document.createElement('iframe'); q.srcdoc='<script>parent.RAN2=1</scr'+'ipt>'; document.body.appendChild(q); q.remove();\
         var k=document.createElement('iframe'); k.srcdoc='<script>parent.RAN3a=1; frameElement.remove();</scr'+'ipt><script>parent.RAN3=1</scr'+'ipt>';\
         k.onload=function(){ console.log('FIRED load'); }; document.body.appendChild(k);\
         setTimeout(function(){ console.log(typeof RAN1, typeof RAN2, typeof RAN3a, typeof RAN3); }, 50);",
    );
    let logs: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("log:")).collect();
    assert_eq!(logs, ["true,true,true true true", "undefined undefined number undefined"], "{log:?}");
}

#[test]
fn qpt1f_custom_elements_upgrade_per_realm() {
    let log = run_with_timers(
        "var c=document.createElement('iframe'); document.body.appendChild(c); var cw=c.contentWindow, cd=c.contentDocument, ceLog=[];\
         cd.body.innerHTML='<x-a></x-a>'; document.body.appendChild(document.createElement('x-a'));\
         cw.eval(\"customElements.define('x-a', class extends HTMLElement { connectedCallback(){ parent.ceLog.push('realm ' + (this.ownerDocument === document)); } })\");\
         var X=cw.customElements.get('x-a');\
         console.log(ceLog.join(), cd.body.firstChild instanceof X, document.querySelector('x-a') instanceof X, customElements.get('x-a'));\
         cd.body.appendChild(cd.createElement('x-a'));\
         customElements.define('x-a', class extends HTMLElement { connectedCallback(){ ceLog.push('page'); } });\
         var p=document.createElement('x-a'); document.body.appendChild(p);\
         console.log(ceLog.join(), p instanceof X, p instanceof customElements.get('x-a'), cd.body.firstChild instanceof customElements.get('x-a'));",
    );
    let logs: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("log:")).collect();
    assert_eq!(logs, ["realm true true false undefined", "realm true,realm true,page,page false true false"], "{log:?}");
}

#[test]
fn qpt1f4_page_methods_drive_the_realm_registry_for_iframe_elements() {
    let log = run_with_timers(
        "var c=document.createElement('iframe'); document.body.appendChild(c); var cw=c.contentWindow, cd=c.contentDocument; window.L=[];\
         cw.eval(\"customElements.define('x-a', class extends HTMLElement { static get observedAttributes(){ return ['k']; } \
           connectedCallback(){ parent.L.push('conn:' + (window !== parent)); } disconnectedCallback(){ parent.L.push('disc'); } \
           attributeChangedCallback(n,o,v){ parent.L.push('attr:'+n+':'+o+':'+v+':'+(window !== parent)); } \
           adoptedCallback(a,b){ parent.L.push('adopt:'+(a===document)+':'+(b===parent.document)); } })\");\
         var X=cw.customElements.get('x-a');\
         var e=document.createElement('x-a'); Node.prototype.appendChild.call(cd.body, e);\
         L.push('up:'+(e instanceof X));\
         Element.prototype.setAttribute.call(e,'k','1'); Element.prototype.removeAttribute.call(e,'k');\
         var h=document.createElement('div'); h.innerHTML='<x-a></x-a>'; Node.prototype.appendChild.call(cd.body, h);\
         L.push('html:'+(h.firstChild instanceof X));\
         Node.prototype.appendChild.call(document.body, e);\
         L.push('moved:'+(e instanceof X));\
         document.body.removeChild(e);\
         console.log(L.join());",
    );
    assert_eq!(
        log,
        ["log:conn:true,up:true,attr:k:null:1:true,attr:k:1:null:true,conn:true,html:true,disc,adopt:true:true,conn:true,moved:true,disc"],
        "{log:?}"
    );
}

#[test]
fn qpt1f4_moved_to_page_uses_page_registry_when_not_upgraded_and_dropped_realm_is_inert() {
    let log = run_with_timers(
        "var c=document.createElement('iframe'); document.body.appendChild(c); var cw=c.contentWindow, cd=c.contentDocument; window.L=[];\
         var u=cd.createElement('x-b'); cd.body.appendChild(u);\
         customElements.define('x-b', class extends HTMLElement { connectedCallback(){ L.push('page-conn'); } });\
         document.body.appendChild(u); L.push('page:'+(u instanceof customElements.get('x-b')));\
         cw.eval(\"customElements.define('x-c', class extends HTMLElement { connectedCallback(){ parent.L.push('dead-conn'); } disconnectedCallback(){ parent.L.push('dead-disc'); } static get observedAttributes(){ return ['q']; } attributeChangedCallback(){ parent.L.push('dead-attr'); } })\");\
         var k=cd.createElement('x-c'); cd.body.appendChild(k);\
         c.remove(); k.setAttribute('q','1'); k.remove(); document.body.appendChild(k);\
         console.log(L.join());",
    );
    assert_eq!(log, ["log:page-conn,page:true,dead-conn"], "{log:?}");
}

#[test]
fn qpt1f_srcdoc_external_script_goes_through_the_page_queue() {
    let fake = QfFake::new(vec![("/dir/e.js", qf7_ok("var E = (window !== parent) + ' ' + document.currentScript.src;", &[("content-type", "text/javascript")]))]);
    let log = qf8_run(
        "https://page.test/dir/page.html",
        &fake,
        "var f=document.createElement('iframe');\
         f.srcdoc='<script src=\"e.js\"></scr'+'ipt><script>console.log(\"after\", E)</scr'+'ipt>';\
         f.onload=function(){ console.log('iframe load'); }; document.body.appendChild(f);\
         /* this harness pumps the network only, not timers: run the srcdoc load task's body directly */\
         f.contentWindow.__scratchy_frame_load(f.srcdoc, true);",
    );
    assert_eq!(fake.urls(), vec!["https://page.test/dir/e.js"], "{log:?}");
    assert_eq!(log, ["log:after true https://page.test/dir/e.js", "log:iframe load"], "{log:?}");
}

// QERR1: an uncaught exception in a top-level script fires an ErrorEvent at window (R64 E7).
#[test]
fn qerr1_uncaught_script_exception_fires_error_event() {
    let log = run_collecting(&[
        src("window.addEventListener('error', function (e) { console.log('L', e.message, e.filename, e.lineno, e.colno, e.error instanceof Error, e.cancelable); });\
             window.onerror = function () { console.log('O', arguments.length, typeof arguments[0], arguments[1], arguments[2], arguments[3], arguments[4] instanceof RangeError); };"),
        src("\n  throw new RangeError('boom');"),
    ]);
    assert!(log[0].starts_with("log:L Uncaught RangeError: boom t 2 "), "{log:?}");
    assert!(log[0].ends_with(" true true"), "{log:?}");
    assert!(log[1].starts_with("log:O 5 string t 2 "), "{log:?}");
    assert!(log[1].ends_with(" true"), "{log:?}");
    assert!(log[2].starts_with("error:t: RangeError: boom"), "{log:?}");
    assert_eq!(log.len(), 3, "{log:?}");
}

#[test]
fn qerr1_onerror_true_suppresses_log_and_throwing_handler_does_not_recurse() {
    let log = run_collecting(&[
        src("window.onerror = function () { return true; };"),
        src("throw 'x';"),
        src("window.onerror = function () { throw new Error('again'); };"),
        src("throw new Error('first');"),
        src("window.onerror = null; window.addEventListener('error', function (e) { e.preventDefault(); });"),
        src("null.x;"),
        src("var = ;"),
    ]);
    assert!(!log.iter().any(|l| l.contains("Uncaught x") || l.contains("TypeError") || l.contains("SyntaxError")), "{log:?}");
    assert_eq!(log.iter().filter(|l| l.contains("first")).count(), 1, "{log:?}");
    assert_eq!(log.iter().filter(|l| l.contains("again")).count(), 1, "{log:?}");
}


// QDOM4: DOM event flags, listener exceptions, handleEvent lookup, isTrusted, passive defaults (R65).
fn qdom4_run(code: &str) -> Vec<String> {
    run_collecting(&[src(code)])
}

#[test]
fn qdom4_stop_flags_cancel_bubble_and_return_value() {
    let log = qdom4_run(
        "var d=document.createElement('div'), p=document.createElement('p'); d.appendChild(p); document.body.appendChild(d);\
         var e=new Event('x',{bubbles:true,cancelable:true});\
         p.addEventListener('x',function(){console.log('p1', e.cancelBubble); e.cancelBubble=false; console.log('p1b', e.cancelBubble); e.stopPropagation(); console.log('p1c', e.cancelBubble)});\
         p.addEventListener('x',function(){console.log('p2')});\
         d.addEventListener('x',function(){console.log('d')});\
         p.dispatchEvent(e); console.log('after', e.cancelBubble, e.composedPath().length);\
         var f=new Event('y',{bubbles:true}); f.cancelBubble=true; d.addEventListener('y',function(){console.log('bad')}); p.dispatchEvent(f); console.log('pre-stopped', f.cancelBubble);\
         var g=new Event('z',{cancelable:true}); console.log(g.returnValue, g.defaultPrevented); g.returnValue=true; console.log(g.returnValue); g.returnValue=false; console.log(g.returnValue, g.defaultPrevented);\
         var h=new Event('z',{cancelable:true}); d.addEventListener('z',function(ev){ev.stopImmediatePropagation()}); d.addEventListener('z',function(){console.log('imm-bad')}); d.dispatchEvent(h); console.log('imm', h.cancelBubble);",
    );
    assert_eq!(
        log,
        ["log:p1 false", "log:p1b false", "log:p1c true", "log:p2", "log:after false 0", "log:pre-stopped false", "log:true false", "log:true",
         "log:false true", "log:imm false"],
        "{log:?}"
    );
}

#[test]
fn qdom4_init_event_during_dispatch_and_src_element() {
    let log = qdom4_run(
        "var d=document.createElement('div'); var e=new Event('a',{cancelable:true});\
         console.log(e.srcElement, e.target);\
         d.addEventListener('a',function(){ e.initEvent('b', true, false); console.log(e.type, e.bubbles, e.srcElement===d); });\
         d.dispatchEvent(e); e.initEvent('c', true, false); console.log(e.type, e.bubbles, e.target);\
         var ev=document.createEvent('Event'); try { d.dispatchEvent(ev); } catch (x) { console.log(x.name); } ev.initEvent('q'); console.log(d.dispatchEvent(ev));\
         d.addEventListener('r', function (x) { try { d.dispatchEvent(x); } catch (y) { console.log(y.name); } }); d.dispatchEvent(new Event('r'));",
    );
    assert_eq!(
        log,
        ["log:null null", "log:a false true", "log:c true null", "log:InvalidStateError", "log:true", "log:InvalidStateError"],
        "{log:?}"
    );
}

#[test]
fn qdom4_listener_exceptions_are_reported_and_dispatch_continues() {
    let log = qdom4_run(
        "var seen=[]; window.addEventListener('error', function (e) { seen.push(e.message); e.preventDefault(); });\
         var d=document.createElement('div');\
         d.addEventListener('t',function(){ throw new Error('one') }); d.addEventListener('t',function(){ console.log('second') });\
         d.addEventListener('t',{ get handleEvent() { throw new RangeError('getter') } }); d.addEventListener('t',{ handleEvent: 5 });\
         console.log(d.dispatchEvent(new Event('t')), seen.join('|'));",
    );
    assert_eq!(
        log,
        ["log:second", "log:true Uncaught Error: one|Uncaught RangeError: getter|Uncaught TypeError: handleEvent is not a function"],
        "{log:?}"
    );
}

#[test]
fn qdom4_handle_event_looked_up_every_invoke() {
    let log = qdom4_run(
        "var o={handleEvent:function(){console.log('A', this===o)}}; var d=document.createElement('div'); d.addEventListener('t',o);\
         d.dispatchEvent(new Event('t')); o.handleEvent=function(){console.log('B')}; d.dispatchEvent(new Event('t'));",
    );
    assert_eq!(log, ["log:A true", "log:B"], "{log:?}");
}

#[test]
fn qdom4_is_trusted_is_an_own_unforgeable_getter() {
    let log = qdom4_run(
        "var e=new Event('x'); var dsc=Object.getOwnPropertyDescriptor(e,'isTrusted');\
         console.log(typeof dsc.get, dsc.set, dsc.configurable, dsc.enumerable, e.isTrusted);\
         try { Object.defineProperty(e,'isTrusted',{value:true}); } catch (x) { console.log(x.name); }\
         e.isTrusted=true; console.log(e.isTrusted, document.createEvent('Event').isTrusted);",
    );
    assert_eq!(log, ["log:function undefined false true false", "log:TypeError", "log:false false"], "{log:?}");
}

#[test]
fn qdom4_passive_by_default_on_window_document_html_body() {
    let log = qdom4_run(
        "var els=[window, document, document.documentElement, document.body, document.createElement('div')];\
         ['touchstart','wheel','click'].forEach(function (t) { var r=[]; els.forEach(function (el) {\
           var f=function (e) { e.preventDefault(); r.push(e.defaultPrevented ? 'c' : 'p'); };\
           el.addEventListener(t, f); var ev=new Event(t,{cancelable:true}); el.dispatchEvent(ev); });\
           console.log(t, r.join('')); });\
         var ev2=new Event('wheel',{cancelable:true}); var f2=function (e) { e.preventDefault(); }; document.addEventListener('wheel', f2, {passive:false}); document.dispatchEvent(ev2); console.log(ev2.defaultPrevented);",
    );
    assert_eq!(log, ["log:touchstart ppppc", "log:wheel ppppc", "log:click ccccc", "log:true"], "{log:?}");
}

// QDOM7: Comment data is mutable (data setter, nodeValue, textContent, CharacterData editing), seen by serialization, clone
// and characterData records.
#[test]
fn qdom7_comment_data_is_mutable() {
    let (log, body) = mutate(
        "<html><body><div id=d><!--abc--></div></body></html>",
        "var d=document.getElementById('d'), c=d.firstChild;\
         var mo=new MutationObserver(function(){}); mo.observe(d,{characterData:true,subtree:true,characterDataOldValue:true});\
         c.data='xyz'; c.appendData('!'); c.insertData(1,'--'); c.deleteData(0,1); c.replaceData(0,1,'Q');\
         console.log(c.data, c.nodeValue, c.textContent, c.length, c.substringData(1,2), c.cloneNode().data);\
         console.log(mo.takeRecords().map(function(x){ return x.type+':'+x.oldValue; }).join(',')); c.nodeValue='nv'; c.textContent='tc'; console.log(c.data, d.innerHTML, d.textContent);",
    );
    let _ = body; // the helper serializes with rcdom's own serializer, which does not see the override
    assert_eq!(
        log,
        ["log:Q-yz! Q-yz! Q-yz! 5 -y Q-yz!", "log:characterData:abc,characterData:xyz,characterData:xyz!,characterData:x--yz!,characterData:--yz!", "log:tc <!--tc--> "],
        "{log:?}"
    );
}

// ---------- QDOM1: window named access (WindowProperties) ----------

#[test]
fn qdom1_window_named_access() {
    crate::js::set_page_url(Url::parse("https://qdom1.test/p.html").ok().as_ref());
    let log = run_in_html(
        "<html><body><div id=box></div><form name=fm></form><img name=pic><object name=ob></object><embed name=em>\
         <span name=nope></span><p id=shadowed></p><iframe name=ifr></iframe></body></html>",
        "var box2 = typeof box;\
         console.log(box2, box === document.getElementById('box'), typeof fm, fm.tagName, pic.tagName, ob.tagName, em.tagName, typeof nope);\
         console.log(window.box === box, 'box' in window, Object.keys(window).indexOf('box'), Object.getOwnPropertyNames(window).indexOf('box'));\
         var shadowed = 5; console.log(shadowed, window.shadowed);\
         document.getElementById('box').remove(); console.log(typeof box, 'box' in window);\
         var d = document.createElement('div'); d.id = 'late'; document.body.appendChild(d); console.log(late === d);\
         d.id = 'later'; console.log(typeof late, later === d);\
         console.log(typeof frames[0].document, window.length);\
         try { notDefinedAnywhere; console.log('no throw'); } catch (e) { console.log(e.name); }\
         undeclared1 = 7; console.log(undeclared1, Object.getOwnPropertyNames(window).indexOf('undeclared1') >= 0);\
         console.log(typeof addEventListener, Object.getPrototypeOf(Window.prototype) !== EventTarget.prototype, window instanceof EventTarget);",
    );
    assert_eq!(
        log,
        [
            "log:object true object FORM IMG OBJECT EMBED undefined",
            "log:true true -1 -1",
            "log:5 5",
            "log:undefined false",
            "log:true",
            "log:undefined true",
            "log:object 1",
            "log:ReferenceError",
            "log:7 true",
            "log:function true true",
        ],
        "{log:?}"
    );
}

/// QEL1: iframes from the parser have their `window[i]` before the first script reads it (no
/// `length` read needed first; WPT animation-frames/callback-cross-realm-report-exception).
#[test]
fn qel1_parser_iframes_have_window_index_at_first_script() {
    let log = run_in_html("<iframe></iframe><iframe name=two></iframe>", "console.log(typeof frames[0], typeof window[1], typeof two, frames.length);");
    assert_eq!(log, ["log:object object object 2"], "{log:?}");
}
