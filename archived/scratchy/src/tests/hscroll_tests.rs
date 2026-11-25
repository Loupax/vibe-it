// QXSCR1: horizontal scrolling (separate overflow-x/-y, document scroll width, window `scroll_x`
// and per-region x offsets, x scroll input, real scrollLeft/scrollX).
#[allow(unused_imports)]
use super::*;
use crate::layout::xk;
use std::time::Instant;

fn regions(laid: &Laid, out: &mut Vec<ScrollInfo>) {
    if let Some(i) = laid.scroll {
        out.push(i);
    }
    let kids: &[Laid] = match &laid.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c,
        _ => &[],
    };
    kids.iter().chain(laid.positioned.iter()).for_each(|c| regions(c, out));
}

fn only_region(html: &str) -> ScrollInfo {
    let laid = measure_document_fixture(html, 800.0, 600.0);
    let mut r = Vec::new();
    regions(&laid, &mut r);
    assert_eq!(r.len(), 1, "one scroll region expected");
    r[0]
}

#[test]
fn qxscr1_overflow_axes_compute_per_css_overflow_3() {
    use crate::style::overflow_axes;
    use Overflow::*;
    assert_eq!(overflow_axes(Auto, Visible), (Auto, Auto), "visible next to a scrolling value is auto");
    assert_eq!(overflow_axes(Visible, Hidden), (Auto, Hidden));
    assert_eq!(overflow_axes(Clip, Scroll), (Hidden, Scroll), "clip next to a scrolling value is hidden");
    assert_eq!(overflow_axes(Clip, Visible), (Clip, Visible), "clip + visible stay as given");
    assert_eq!(overflow_axes(Visible, Visible), (Visible, Visible));
    // getComputedStyle reports each axis.
    let log = run_in_html(
        "<div id=a style='overflow-x:auto'></div><div id=b style='overflow:hidden clip'></div>\
         <div id=c style='overflow-x:clip'></div>",
        "for (const id of ['a','b','c']) { const s = getComputedStyle(document.getElementById(id));\
         console.log(id, s.overflowX, s.overflowY, s.overflow); }",
    );
    assert_eq!(log, ["log:a auto auto auto", "log:b hidden hidden hidden", "log:c clip visible clip visible"], "{log:?}");
}

#[test]
fn qxscr1_overflow_x_auto_scrolls_x_only_and_y_is_auto_too() {
    // A 1000 px wide child in a 200 px box: x overflows, y does not.
    let info = only_region(
        "<html><body style='margin:0'><div style='overflow-x:auto;overflow-y:visible;width:200px'>\
         <div style='width:1000px;height:20px'></div></div></body></html>",
    );
    assert!(info.user && info.user_x, "both axes user-scrollable (visible computed to auto)");
    assert!((info.client_w - 200.0).abs() < 0.5, "client_w {}", info.client_w);
    assert!((info.content_w - 1000.0).abs() < 0.5, "content_w {}", info.content_w);
    assert!((info.max_scroll_x() - 800.0).abs() < 0.5);
    assert_eq!(info.max_scroll(), 0.0, "nothing to scroll vertically");
    // overflow-y:auto with a wide child: the y axis is the one that does not take x scroll... both
    // axes are scrollable, but x has nothing to do when the content fits.
    let tall = only_region(
        "<html><body style='margin:0'><div style='overflow-y:auto;height:100px;width:200px'>\
         <div style='height:500px'></div></div></body></html>",
    );
    assert_eq!(tall.max_scroll_x(), 0.0);
    assert!(tall.max_scroll() > 350.0);
    // overflow-x:hidden: script-only on x, the wheel never moves it.
    let hid = only_region(
        "<html><body style='margin:0'><div style='overflow-x:hidden;width:200px'>\
         <div style='width:1000px;height:20px'></div></div></body></html>",
    );
    assert!(!hid.user_x && hid.user, "hidden x, auto y");
    assert!(!region_takes_wheel_x(&hid, &HashMap::new(), -40.0));
}

#[test]
fn qxscr1_overflow_clip_on_one_axis_clips_only_that_axis() {
    let html = "<html><body style='margin:0'><div style='overflow-x:clip;width:200px;height:50px'>\
        <div style='width:1000px;height:300px'></div></div><div style='height:10px'></div></body></html>";
    let laid = measure_document_fixture(html, 800.0, 600.0);
    let mut r = Vec::new();
    regions(&laid, &mut r);
    assert!(r.is_empty(), "clip is no scroll container");
    // The child spills downward (y stays visible) but not sideways.
    assert!(laid.ink.b >= 300.0, "ink bottom {}", laid.ink.b);
    assert!(crate::layout::doc_scroll_width(&laid) <= 800.5, "no horizontal overflow past the clip");
}

#[test]
fn qxscr1_nowrap_text_overflow_counts_as_scroll_width() {
    let words = "word ".repeat(200);
    let info = only_region(&format!(
        "<html><body style='margin:0'><div style='overflow-x:auto;width:200px;white-space:nowrap'>{words}</div></body></html>"
    ));
    assert!(info.content_w > info.client_w * 3.0, "nowrap text overflows: {} vs {}", info.content_w, info.client_w);
    // Wrapping text fits its box.
    let wrap = only_region(&format!("<html><body style='margin:0'><div style='overflow-x:auto;width:200px'>{words}</div></body></html>"));
    assert_eq!(wrap.max_scroll_x(), 0.0);
}

#[test]
fn qxscr1_document_scroll_width_follows_wide_content() {
    let laid = measure_document_fixture(
        "<html><body style='margin:0'><div style='width:2000px;height:10px'></div></body></html>",
        800.0,
        600.0,
    );
    assert!((crate::layout::doc_scroll_width(&laid) - 2000.0).abs() < 0.5);
    let narrow = measure_document_fixture("<html><body style='margin:0'><p>x</p></body></html>", 800.0, 600.0);
    assert!((crate::layout::doc_scroll_width(&narrow) - 800.0).abs() < 0.5, "never less than the viewport");
    // A fixed box adds nothing.
    let fixed = measure_document_fixture(
        "<html><body style='margin:0'><div style='position:fixed;left:0;top:0;width:3000px;height:10px'></div></body></html>",
        800.0,
        600.0,
    );
    assert!((crate::layout::doc_scroll_width(&fixed) - 800.0).abs() < 0.5);
    // overflow-x:hidden on body goes to the viewport (user x scroll blocked), y untouched.
    let hid = measure_document_fixture(
        "<html><body style='margin:0;overflow-x:hidden'><div style='width:2000px;height:10px'></div></body></html>",
        800.0,
        600.0,
    );
    assert!(hid.style.viewport_hidden_x && !hid.style.viewport_hidden);
}

#[test]
fn qxscr1_shift_wheel_turns_vertical_ticks_horizontal() {
    assert_eq!(wheel_axes(0.0, -40.0, true), (-40.0, 0.0));
    assert_eq!(wheel_axes(0.0, -40.0, false), (0.0, -40.0));
    // A real horizontal wheel/touchpad delta stays as it is, Shift or not.
    assert_eq!(wheel_axes(-12.0, -3.0, true), (-12.0, -3.0));
    assert_eq!(wheel_axes(-12.0, 0.0, false), (-12.0, 0.0));
}

fn app_with(html: &str) -> Option<App> {
    app_with_script(html, "window.L=[]; addEventListener('keydown', e => L.push(e.key));")
}

fn app_with_script(html: &str, script: &str) -> Option<App> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    let doc = parse_html(html);
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/page").unwrap());
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    Some(app)
}

/// `App::cached` from a synchronous measure at 800x600.
fn install(app: &mut App, _html: &str) {
    let tree = build_layout_tree(&app.document, 1920.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut incomplete) = (0usize, false);
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut font_system, 800.0, 600.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    app.cached = Some(Rc::new(laid));
}

fn tap(app: &mut App, key: &Key) {
    let t = Instant::now();
    app.route_key(key, None, true, false, t);
    app.route_key(key, None, false, false, t);
}

const WIDE: &str = "<html><body style='margin:0'><div style='width:2000px;height:10px'></div></body></html>";

#[test]
fn qxscr1_wide_page_scrolls_with_shift_wheel_and_arrows() {
    let Some(mut app) = app_with(WIDE) else { return };
    install(&mut app, WIDE);
    // Shift+wheel down (winit dy < 0 = content up) moves the view right.
    let (dx, _) = wheel_axes(0.0, -crate::config::WHEEL_LINE_PX, true);
    app.wheel_x(&[], dx);
    assert_eq!(app.scroll_x, crate::config::WHEEL_LINE_PX);
    app.wheel_x(&[], -dx);
    assert_eq!(app.scroll_x, 0.0);
    // Input mode: ArrowRight/Left are UA scroll keys after the page saw them.
    app.key_mode = KeyMode::Input; // the start mode is a knob (QKEYS5)
    tap(&mut app, &Key::Named(NamedKey::ArrowRight));
    assert_eq!(app.scroll_x, crate::config::SCROLL_LINE_PX);
    let seen = app.js.as_ref().unwrap().eval_value::<String>("t", "L.join()").unwrap_or_default();
    assert_eq!(seen, "ArrowRight", "the page sees the key first");
    tap(&mut app, &Key::Named(NamedKey::ArrowLeft));
    assert_eq!(app.scroll_x, 0.0);
    // Normal mode: the bindings.
    let n = VimMode::Normal;
    let m = ModifiersState::empty();
    assert_eq!(resolve_binding(&Key::Named(NamedKey::ArrowRight), m, n), Some(Action::ScrollRight));
    assert_eq!(resolve_binding(&Key::Named(NamedKey::ArrowLeft), m, n), Some(Action::ScrollLeft));
    // No letter keys: h/l stay history.
    assert_eq!(resolve_binding(&Key::Character("h".into()), m, n), Some(Action::HistoryBack));
    assert_eq!(resolve_binding(&Key::Character("l".into()), m, n), Some(Action::HistoryFront));
    app.perform_action(Action::ScrollRight);
    assert_eq!(app.scroll_x, crate::config::SCROLL_LINE_PX);
}

#[test]
fn qxscr1_viewport_overflow_x_hidden_blocks_user_x_scroll() {
    let html = "<html><body style='margin:0;overflow-x:hidden'><div style='width:2000px;height:10px'></div></body></html>";
    let Some(mut app) = app_with(html) else { return };
    install(&mut app, html);
    app.wheel_x(&[], -40.0);
    app.perform_action(Action::ScrollRight);
    assert_eq!(app.scroll_x, 0.0);
    app.perform_action(Action::ScrollDown);
    assert_eq!(app.scroll_offset, crate::config::SCROLL_LINE_PX, "y is still free");
}

#[test]
fn qxscr1_wheel_x_takes_the_innermost_x_region_then_chains() {
    let html = "<html><body style='margin:0'><div style='overflow-x:auto;width:200px'>\
        <div style='width:1000px;height:20px'></div></div><div style='width:2000px;height:10px'></div></body></html>";
    let Some(mut app) = app_with(html) else { return };
    install(&mut app, html);
    let laid = app.cached.clone().unwrap();
    let chain = crate::paint::hit_test_scroll_chain(&laid, 10.0, 10.0, &app.scroll_offsets, 0.0, 600.0);
    assert_eq!(chain.len(), 1);
    let id = chain[0].id;
    app.wheel_x(&chain, -40.0);
    assert_eq!(app.scroll_offsets.get(&xk(id)).copied(), Some(40.0));
    assert_eq!(app.scroll_x, 0.0, "the region took it");
    assert!(!app.scroll_offsets.contains_key(&id), "the y offset is untouched");
    // At its end the tick chains to the window.
    app.scroll_offsets.insert(xk(id), 800.0);
    app.wheel_x(&chain, -40.0);
    assert_eq!(app.scroll_x, 40.0);
}

#[test]
fn qxscr1_hit_testing_after_an_x_scroll() {
    let html = "<html><body style='margin:0'><div style='width:2000px;height:10px'></div>\
        <div id=far style='position:absolute;left:1000px;top:0;width:100px;height:100px'></div></body></html>";
    let Some(mut app) = app_with(html) else { return };
    install(&mut app, html);
    let far = node_id(&find_node_by_id_attr(&app.document, "far"));
    assert_ne!(app.element_at((60.0, 50.0)), far, "not under the pointer before scrolling");
    app.scroll_x = 950.0;
    assert_eq!(app.element_at((60.0, 50.0)), far, "window x scroll moves the hit");
    // A region's x offset moves its content's hits too.
    let html = "<html><body style='margin:0'><div style='overflow-x:auto;width:200px;height:50px'>\
        <div style='width:1000px;height:50px;display:flex'><div style='flex:none;width:500px;height:50px'></div>\
        <div style='flex:none;width:50px;height:50px'></div></div></div></body></html>";
    let laid = measure_document_fixture(html, 800.0, 600.0);
    let mut r = Vec::new();
    regions(&laid, &mut r);
    let id = r[0].id;
    let mut offs = HashMap::new();
    let before = hit_test_element(&laid, 20.0, 20.0, &ShapeCache::new(), &offs, 0.0, 600.0);
    offs.insert(xk(id), 490.0);
    let after = hit_test_element(&laid, 20.0, 20.0, &ShapeCache::new(), &offs, 0.0, 600.0);
    assert_ne!(before, after, "the second flex item is under x 20 after scrolling 490");
}

/// The element with `id` attribute `id`.
fn find_node_by_id_attr(doc: &Handle, id: &str) -> Handle {
    by_id(doc, id)
}

#[test]
fn qxscr1_fixed_box_ignores_x_scroll_and_content_moves() {
    let html = "<html><body style='margin:0'><div style='width:2000px;height:200px;background-color:#0000ff'></div>\
        <div style='position:fixed;left:0;top:0;width:50px;height:50px;background-color:#ff0000'></div>\
        <div style='position:absolute;left:300px;top:100px;width:50px;height:50px;background-color:#00ff00'></div></body></html>";
    let doc = parse_html(html);
    let tree = build_layout_tree_sized(&doc, 1920.0, 200.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut incomplete) = (0usize, false);
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut font_system, 200.0, 200.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let mut swash_cache = SwashCache::new();
    let mut pixels = vec![0xFFFF_FFFFu32; 200 * 200];
    crate::paint::with_scroll_x(300.0, || {
        paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &HashMap::new(), &mut pixels, 200, 200, 0.0, None)
    });
    assert_eq!(px_at(&pixels, 200, 10, 10), 0xFF0000, "the fixed box stays at the viewport's left");
    assert_eq!(px_at(&pixels, 200, 10, 110), 0x00FF00, "the box at x 300 is now at x 0");
    assert_eq!(px_at(&pixels, 200, 100, 110), 0x0000FF);
    assert_eq!(crate::paint::scroll_x(), 0.0, "restored after the frame");
}

#[test]
fn qxscr1_region_x_offset_moves_painted_content_inside_the_clip() {
    let html = "<html><body style='margin:0'><div style='overflow-x:auto;width:100px;height:50px'>\
        <div style='width:1000px;height:50px;display:flex'>\
        <div style='flex:none;width:200px;height:50px;background-color:#ff0000'></div>\
        <div style='flex:none;width:200px;height:50px;background-color:#00ff00'></div></div></div></body></html>";
    let doc = parse_html(html);
    let tree = build_layout_tree_sized(&doc, 1920.0, 200.0);
    let mut font_system = new_font_system();
    let mut cache = ShapeCache::new();
    let (mut next_id, mut incomplete) = (0usize, false);
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    let laid = measure_document(&tree, &mut font_system, 200.0, 200.0, &mut next_id, &mut cache, deadline, &mut incomplete);
    let mut r = Vec::new();
    regions(&laid, &mut r);
    let mut offs = HashMap::new();
    offs.insert(xk(r[0].id), 250.0);
    let mut swash_cache = SwashCache::new();
    let mut pixels = vec![0xFFFF_FFFFu32; 200 * 200];
    paint(&laid, &mut font_system, &mut swash_cache, &mut cache, &offs, &mut pixels, 200, 200, 0.0, None);
    assert_eq!(px_at(&pixels, 200, 10, 10), 0x00FF00, "scrolled 250: the second box is under x 10");
    assert_eq!(px_at(&pixels, 200, 150, 10), 0xFFFFFF, "clipped to the 100 px box");
}

const QX_BOX: &str = "<html><head><style>body{margin:0} #box{width:200px;height:50px;overflow-x:auto}\
    #wide{width:600px;height:20px;position:relative} #r{position:absolute;left:300px;top:0;width:10px;height:10px}\
    #page{width:3000px;height:10px}</style></head><body><div id=box><div id=wide><div id=r></div></div></div><div id=page></div></body></html>";

#[test]
fn qxscr1_scroll_left_round_trips_and_moves_child_rects() {
    let log = run_in_html(
        QX_BOX,
        "var box=document.getElementById('box'), r=document.getElementById('r');\
         console.log(box.scrollWidth, box.clientWidth, box.scrollLeft);\
         var x0=r.getBoundingClientRect().left;\
         box.scrollLeft = 50; console.log(box.scrollLeft, x0 - r.getBoundingClientRect().left);\
         box.scrollLeft = 9999; console.log(box.scrollLeft);\
         box.scrollLeft = -5; console.log(box.scrollLeft);\
         box.scrollTo({left: 30}); console.log(box.scrollLeft, box.scrollTop);\
         box.scrollBy(20, 0); console.log(box.scrollLeft);",
    );
    assert_eq!(log, ["log:600 200 0", "log:50 50", "log:400", "log:0", "log:30 0", "log:50"], "{log:?}");
}

#[test]
fn qxscr1_window_scroll_x_is_clamped_to_the_scroll_width() {
    let log = run_in_html(
        QX_BOX,
        "console.log(document.documentElement.scrollWidth > innerWidth);\
         scrollTo(100, 0); console.log(scrollX, pageXOffset);\
         scrollTo(999999, 0); console.log(scrollX === document.documentElement.scrollWidth - innerWidth);\
         scrollBy(-50, 0); console.log(scrollX === document.documentElement.scrollWidth - innerWidth - 50);\
         document.documentElement.scrollLeft = 10; console.log(scrollX);",
    );
    assert_eq!(log, ["log:true", "log:100 100", "log:true", "log:true", "log:10"], "{log:?}");
}

#[test]
fn qxscr1_scroll_left_fires_a_scroll_event() {
    let doc = parse_html(QX_BOX);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &doc,
        &[src("var box=document.getElementById('box'); box.addEventListener('scroll', function(){ console.log('scroll', box.scrollLeft); });\
               box.scrollLeft = 40; box.scrollLeft = 40;")],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    for _ in 0..100 {
        let Some(due) = crate::event_loop::headless_next_due(&engine) else { break };
        clock.set(clock.get().max(due));
        crate::event_loop::headless_turn(&engine, clock.get());
    }
    assert_eq!(*out.borrow(), ["log:scroll 40"]);
}

#[test]
fn qxscr1_app_applies_script_x_scroll_and_mirrors_it_back() {
    let script = "document.getElementById('box').scrollLeft = 70; scrollTo(120, 0);";
    let Some(mut app) = app_with_script(QX_BOX, script) else { return };
    install(&mut app, QX_BOX);
    let doc = app.document.clone();
    app.js_sync();
    assert_eq!(app.scroll_x, 120.0);
    let node = node_id(&by_id(&doc, "box"));
    assert_eq!(app.node_scroll.get(&xk(node)).copied(), Some(70.0));
    assert!(!app.node_scroll.contains_key(&node), "y untouched");
    let laid = app.cached.clone().unwrap();
    let info = crate::paint::find_scroll_node(&laid, node).unwrap();
    assert_eq!(app.scroll_offsets.get(&xk(info.id)).copied(), Some(70.0));
    // The App's offsets go back to script (e.g. after a wheel scroll).
    app.node_scroll.insert(xk(node), 90.0);
    app.node_scroll_dirty = true;
    app.scroll_x = 33.0;
    app.js_dispatch(None, "custom", &[]);
    let js = app.js.as_ref().unwrap();
    let got = js.eval_value::<String>("t", "document.getElementById('box').scrollLeft + ' ' + scrollX").unwrap_or_default();
    assert_eq!(got, "90 33");
}

#[test]
fn qxscr1_search_reveal_x_scrolls_the_page_to_an_offscreen_match() {
    let lines = vec![crate::search::SearchLine::from_text("needle", 1500.0, 10.0, 8.0, 16.0)];
    let m = crate::search::find_matches(&lines, "needle").remove(0);
    let (changes, page) = m.reveal_x(&HashMap::new(), 0.0, 800.0, 3000.0);
    assert!(changes.is_empty());
    let p = page.expect("off screen to the right: the page scrolls");
    assert!(p > 700.0 && p < 1500.0, "centred: {p}");
    let (_, page) = m.reveal_x(&HashMap::new(), 1400.0, 800.0, 3000.0);
    assert_eq!(page, None, "already visible: nothing moves");
    // Highlights follow the region's x offset.
    let mut m = m;
    m.regions = vec![crate::search::SearchRegion { id: 3, node: 0, rect: (1400.0, 0.0, 400.0, 100.0), max_scroll: 0.0, max_scroll_x: 500.0 }];
    let mut offs = HashMap::new();
    offs.insert(xk(3), 100.0);
    let r = m.visible_rects(&offs);
    assert!((r[0].0 - 1400.0).abs() < 0.5, "shifted left by 100: {:?}", r);
    assert!((m.doc_left(&offs) - 1400.0).abs() < 0.5);
}
