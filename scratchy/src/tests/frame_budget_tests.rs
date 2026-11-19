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
