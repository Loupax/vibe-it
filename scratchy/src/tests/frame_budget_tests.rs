// QM1k1n6 (REGRESSIONS.md R46): the two main-thread stalls fixed with the frame-budget
// design. The live effect (video painted per second, idle turns per second) is measured
// in a nested window with SCRATCHY_FRAME_STATS; these tests pin the code facts it rests on.
#[allow(unused_imports)]
use super::*;

/// `App::pump_net` runs on every event-loop turn and may only touch the event-loop proxy
/// (and so drop a clone, which wakes the loop) when a job actually starts. That rests on
/// `NetScheduler::pump` never calling its starter for an empty queue.
#[test]
fn qm1k1n6_pump_with_nothing_queued_starts_nothing() {
    let mut sched = NetScheduler::new(&RequestPolicy::default());
    let mut calls = 0;
    for _ in 0..1000 {
        sched.pump(&mut |_| calls += 1);
    }
    assert_eq!(calls, 0);
}

/// mpv's software render must not sleep until the frame's target time on the main thread.
#[cfg(feature = "media")]
#[test]
fn qm1k1n6_sw_render_does_not_block_for_target_time() {
    let block = crate::media_mpv::sw_render_param_ints();
    assert_eq!(block, vec![(crate::media_mpv::RP_BLOCK_FOR_TARGET_TIME, 0)]);
    assert_eq!(crate::media_mpv::RP_BLOCK_FOR_TARGET_TIME, 12, "MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME in mpv/render.h");
}

// QM1k1n6a: pointer-motion coalescing.

/// Needs an event loop for the proxy; `None` without a display (headless CI).
fn qm1k1n6a_app(html: &str) -> Option<App> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    let mut app = App::new_static(parse_html(html), event_loop.create_proxy(), None);
    app.cached = Some(measure_fixture(html, 400.0));
    app.layout_dirty = false;
    // Leak the loop: only one can be built per process and the proxy must stay valid.
    std::mem::forget(event_loop);
    Some(app)
}

#[test]
fn qm1k1n6a_queued_moves_cost_one_hit_test() {
    let Some(mut app) = qm1k1n6a_app("<p>hello</p>") else { return };
    for i in 0..200 {
        app.queue_pointer_move((i as f32, 5.0));
    }
    assert_eq!(app.pointer_moves_processed, 0, "queueing must not hit-test");
    assert_eq!(app.last_mouse_pos, None);
    app.flush_pointer_move();
    assert_eq!(app.pointer_moves_processed, 1);
    assert_eq!(app.last_mouse_pos, Some((199.0, 5.0)), "the newest position wins");
    app.flush_pointer_move();
    assert_eq!(app.pointer_moves_processed, 1, "nothing pending: no second hit test");
}

/// A button, wheel or key event flushes first, so the click or wheel lands at the
/// latest position (`MouseInput`/`MouseWheel` read `last_mouse_pos`); moves do not.
#[test]
fn qm1k1n6a_buttons_wheel_keys_flush_pending_move_first() {
    let dev = winit::event::DeviceId::dummy();
    let wheel = WindowEvent::MouseWheel {
        device_id: dev,
        delta: MouseScrollDelta::LineDelta(0.0, 1.0),
        phase: winit::event::TouchPhase::Moved,
    };
    let click = WindowEvent::MouseInput { device_id: dev, state: ElementState::Pressed, button: MouseButton::Left };
    let moved = WindowEvent::CursorMoved { device_id: dev, position: winit::dpi::PhysicalPosition::new(1.0, 2.0) };
    assert!(event_flushes_pointer(&wheel));
    assert!(event_flushes_pointer(&click));
    assert!(event_flushes_pointer(&WindowEvent::CursorLeft { device_id: dev }));
    assert!(!event_flushes_pointer(&moved));
    assert!(!event_flushes_pointer(&WindowEvent::RedrawRequested));
}

/// After moves, a click's hit test (`element_at(last_mouse_pos)`) sees the latest position.
#[test]
fn qm1k1n6a_click_after_moves_hits_latest_position() {
    let Some(mut app) = qm1k1n6a_app("<p>hello</p>") else { return };
    app.queue_pointer_move((1000.0, 1000.0));
    app.queue_pointer_move((5.0, 5.0));
    app.flush_pointer_move();
    let pos = app.last_mouse_pos.expect("flushed");
    assert_eq!(pos, (5.0, 5.0));
    assert_eq!(app.element_at(pos), app.element_at((5.0, 5.0)));
}

// QM1k1n6b: `run_due` time slice.

fn qm1k1n6b_log(out: &Rc<RefCell<Vec<String>>>) -> Vec<String> {
    out.borrow().iter().filter_map(|l| l.strip_prefix("log:").map(str::to_string)).collect()
}

/// A slice ends the turn once its wall time is used; the rest runs on the next turns, in order.
#[test]
fn qm1k1n6b_order_preserved_across_slices() {
    let (engine, out) = qb8_start(
        "<body><script>window.__spin = function(ms){ var e = Date.now() + ms; while (Date.now() < e); };
        for (var i = 0; i < 6; i++) (function(i){ setTimeout(function(){ __spin(8); console.log('t' + i); }, 0); })(i);</script></body>",
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    let slice = Some(std::time::Duration::from_millis(16));
    let mut turns = 0;
    while engine.run_due_slice(slice) {
        turns += 1;
        assert!(turns < 20);
    }
    assert!(turns >= 2, "6 x 8 ms must not fit one 16 ms slice (turns {turns})");
    assert_eq!(qm1k1n6b_log(&out), ["t0", "t1", "t2", "t3", "t4", "t5"]);
}

/// After a sliced turn work is still due, so the loop's next deadline is already in the past
/// (it wakes again at once); with nothing due there is no wake-up at all.
#[test]
fn qm1k1n6b_leftover_work_wakes_the_loop_and_idle_does_not() {
    let (engine, _out) = qb8_start(
        "<body><script>for (var i = 0; i < 4; i++) setTimeout(function(){ var e = Date.now() + 10; while (Date.now() < e); }, 0);</script></body>",
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(engine.run_due_slice(Some(std::time::Duration::from_millis(5))));
    assert!(engine.next_deadline().is_some_and(|t| t <= std::time::Instant::now()), "work remains: wake now");
    while engine.run_due_slice(Some(std::time::Duration::from_millis(5))) {}
    assert_eq!(engine.next_deadline(), None, "nothing pending: the loop sleeps");
    assert!(!engine.run_due_slice(Some(std::time::Duration::from_millis(5))));
}

/// One callback longer than the slice cannot be split: it runs whole and ends the slice.
#[test]
fn qm1k1n6b_long_single_callback_still_runs() {
    let (engine, out) = qb8_start(
        "<body><script>setTimeout(function(){ var e = Date.now() + 40; while (Date.now() < e); console.log('long'); }, 0);
        setTimeout(function(){ console.log('next'); }, 0);</script></body>",
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(engine.run_due_slice(Some(std::time::Duration::from_millis(16))), "slice used up, one timer left");
    assert_eq!(qm1k1n6b_log(&out), ["long"]);
    assert!(!engine.run_due_slice(Some(std::time::Duration::from_millis(16))));
    assert_eq!(qm1k1n6b_log(&out), ["long", "next"]);
}

/// 16 ms while a video is on screen or input is fresh, 50 ms otherwise.
#[test]
fn qm1k1n6b_slice_length_follows_video_and_input() {
    let ms = |i, v| timer_slice(i, v).as_millis();
    assert_eq!((ms(false, false), ms(true, false), ms(false, true), ms(true, true)), (50, 16, 16, 16));
}

/// A queued pointer move shortens the slice, so the pending input gets its turn sooner.
#[test]
fn qm1k1n6b_pending_input_counts_as_recent() {
    let Some(mut app) = qm1k1n6a_app("<p>hello</p>") else { return };
    assert!(!app.input_recent());
    app.queue_pointer_move((1.0, 1.0));
    assert!(app.input_recent());
}

// QCUR1: mousemove/pointermove (and enter/leave/pointerover/out) reach the page.

fn qcur1_app(html: &str, script: &str) -> Option<App> {
    let mut app = qm1k1n6a_app(html)?;
    let doc = app.document.clone();
    // Lay out the app's OWN document (node ids are addresses), not the fixture's copy.
    let tree = build_layout_tree(&doc, 1920.0);
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    app.cached = Some(measure(
        &tree, &mut new_font_system(), 0.0, 0.0, 400.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
        &mut Vec::new(), &Floats::default(), None,
    ));
    app.js =run_page_scripts(&doc, &[src(script)], None);
    Some(app)
}

fn qcur1_log(app: &App) -> String {
    app.js.as_ref().unwrap().eval_value::<String>("t", "window.L.join('|')").unwrap_or_default()
}

const QCUR1_HTML: &str = "<html><body style='margin:0'><p id=p style='margin:0;height:40px'>hello</p></body></html>";

#[test]
fn qcur1_mousemove_once_per_turn_with_latest_coordinates() {
    let script = "window.L=[]; document.addEventListener('mousemove', function(e){ L.push('m'+e.clientX+','+e.clientY+','+e.movementX+','+e.target.id+','+e.buttons+','+e.isTrusted); });\
                  document.addEventListener('pointermove', function(e){ L.push('p'+e.pointerId+e.pointerType+e.isPrimary+e.clientX); });";
    let Some(mut app) = qcur1_app(QCUR1_HTML, script) else { return };
    for i in 0..50 {
        app.queue_pointer_move((i as f32, 5.0));
    }
    app.flush_pointer_move();
    assert_eq!(qcur1_log(&app), "p1mousetrue49|m49,5,0,p,0,true");
    app.queue_pointer_move((51.0, 6.0));
    app.flush_pointer_move();
    assert_eq!(qcur1_log(&app).matches("|m").count(), 1, "a second turn inside the throttle interval dispatches nothing");
    std::thread::sleep(MOVE_DISPATCH_INTERVAL * 2);
    app.queue_pointer_move((52.0, 6.0));
    app.flush_pointer_move();
    app.flush_pointer_move();
    let l = qcur1_log(&app);
    assert!(l.ends_with("|p1mousetrue52|m52,6,3,p,0,true"), "{l}");
    assert_eq!(l.matches("|m").count(), 2, "one more mousemove (two in all), not one per queued move: {l}");
}

#[test]
fn qcur1_no_listener_no_dispatch_and_mouseover_unchanged() {
    let script = "window.L=[]; document.addEventListener('mouseover', function(e){ L.push('over'); });";
    let Some(mut app) = qcur1_app(QCUR1_HTML, script) else { return };
    app.queue_pointer_move((5.0, 5.0));
    app.flush_pointer_move();
    assert_eq!(qcur1_log(&app), "over");
    assert_eq!(app.last_move_dispatched, None, "no move listener: nothing built or dispatched");
}

#[test]
fn qcur1_removed_listener_stops_dispatch() {
    let script = "window.L=[]; window.f=function(){ L.push('m'); }; document.addEventListener('mousemove', f);";
    let Some(mut app) = qcur1_app(QCUR1_HTML, script) else { return };
    app.queue_pointer_move((5.0, 5.0));
    app.flush_pointer_move();
    app.js.as_ref().unwrap().eval("t", "document.removeEventListener('mousemove', f)");
    std::thread::sleep(MOVE_DISPATCH_INTERVAL * 2);
    app.queue_pointer_move((6.0, 5.0));
    app.flush_pointer_move();
    assert_eq!(qcur1_log(&app), "m");
}

#[test]
fn qcur1_entering_an_element_fires_over_enter_move_in_order() {
    let html = "<html><body style='margin:0'><div id=o style='height:40px'><p id=p style='margin:0;height:40px'>hello</p></div></body></html>";
    let script = "window.L=[]; var o=document.getElementById('o');\
                  ['pointerover','pointerenter','mouseover','mouseenter'].forEach(function(t){ o.addEventListener(t, function(e){ L.push(t+':'+e.target.id); }); });\
                  o.addEventListener('pointermove', function(e){ L.push('pm:'+e.target.id); });\
                  document.addEventListener('mousemove', function(e){ L.push('mm'); });";
    let Some(mut app) = qcur1_app(html, script) else { return };
    app.queue_pointer_move((5.0, 5.0));
    app.flush_pointer_move();
    let l = qcur1_log(&app);
    let pos = |s: &str| l.find(s).unwrap_or_else(|| panic!("{s} in {l}"));
    assert!(pos("pointerover:p") < pos("pointerenter:o") && pos("pointerenter:o") < pos("mouseover:p"), "{l}");
    assert!(pos("mouseover:p") < pos("mouseenter:o") && pos("mouseenter:o") < pos("pm:p"), "{l}");
    assert!(pos("pm:p") < pos("mm"), "{l}");
}

const QCUR1_MM: &str = "window.L=[]; document.addEventListener('mousemove', function(e){ L.push(String(e.clientX)); });";

fn qcur1_move(app: &mut App, x: f32) {
    app.queue_pointer_move((x, 5.0));
    app.flush_pointer_move();
}

#[test]
fn qcur1_playing_video_spaces_dispatches_150ms_and_first_move_after_idle_is_immediate() {
    let Some(mut app) = qcur1_app(QCUR1_HTML, QCUR1_MM) else { return };
    app.last_video_frame = Some(std::time::Instant::now());
    qcur1_move(&mut app, 1.0);
    std::thread::sleep(std::time::Duration::from_millis(80));
    app.last_video_frame = Some(std::time::Instant::now());
    qcur1_move(&mut app, 2.0);
    assert_eq!(qcur1_log(&app), "1", "80 ms after the last dispatch: still inside the 150 ms gap");
    std::thread::sleep(std::time::Duration::from_millis(100));
    app.last_video_frame = Some(std::time::Instant::now());
    qcur1_move(&mut app, 3.0);
    assert_eq!(qcur1_log(&app), "1|3");
    // A move after >= 1 s without one is dispatched at once, however recent the last dispatch.
    app.last_move_seen = Some(std::time::Instant::now() - MOVE_IDLE * 2);
    qcur1_move(&mut app, 4.0);
    assert_eq!(qcur1_log(&app), "1|3|4");
}

#[test]
fn qcur1_throttled_move_gets_a_trailing_dispatch_of_the_last_position() {
    let Some(mut app) = qcur1_app(QCUR1_HTML, QCUR1_MM) else { return };
    qcur1_move(&mut app, 1.0);
    qcur1_move(&mut app, 2.0);
    qcur1_move(&mut app, 3.0);
    assert_eq!(qcur1_log(&app), "1");
    let due = app.flush_move_trailing().expect("a trailing dispatch is pending");
    assert_eq!(qcur1_log(&app), "1", "not before the gap has passed");
    std::thread::sleep(due.saturating_duration_since(std::time::Instant::now()) + std::time::Duration::from_millis(5));
    assert_eq!(app.flush_move_trailing(), None);
    assert_eq!(qcur1_log(&app), "1|3", "the final position, not 2");
    assert_eq!(app.flush_move_trailing(), None, "nothing left");
}

#[test]
fn qcur1_drag_dispatches_every_turn() {
    let Some(mut app) = qcur1_app(QCUR1_HTML, QCUR1_MM) else { return };
    app.last_video_frame = Some(std::time::Instant::now());
    app.mouse_down_pos = Some((0.0, 0.0));
    for x in [1.0, 2.0, 3.0] {
        qcur1_move(&mut app, x);
    }
    assert_eq!(qcur1_log(&app), "1|2|3");
}
