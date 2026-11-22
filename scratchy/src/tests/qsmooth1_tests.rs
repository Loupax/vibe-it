// QSMOOTH1: eased viewport scroll (target + exponential step), instant inputs and the tau 0 path.
#[allow(unused_imports)]
use super::*;

fn app() -> Option<App> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    let mut app = App::new_static(parse_html("<p>x</p>"), event_loop.create_proxy(), None);
    app.smooth_scroll_tau_ms = crate::config::SMOOTH_SCROLL_TAU_MS;
    Some(app)
}

const MAX: f32 = 5000.0;

fn run(app: &mut App) -> usize {
    let mut n = 0;
    while let Some(t) = app.scroll_target {
        app.scroll_anim_step_dt(MAX, 16.0, t);
        n += 1;
        assert!(n < 500, "never settles");
    }
    n
}

#[test]
fn qsmooth1_notch_sets_target_and_steps_converge_and_snap() {
    let Some(mut app) = app() else { return };
    app.scroll_by(40.0);
    assert_eq!(app.scroll_target, Some(40.0));
    assert_eq!(app.scroll_offset, 0.0, "painted offset unchanged until a step");
    app.scroll_anim_step_dt(MAX, 16.0, 40.0);
    assert!(app.scroll_offset > 0.0 && app.scroll_offset < 40.0);
    run(&mut app);
    assert_eq!((app.scroll_offset, app.scroll_target), (40.0, None));
}

#[test]
fn qsmooth1_step_is_frame_rate_independent() {
    let (a, _) = scroll_ease(0.0, 1000.0, 10.0, 40.0);
    let (b, _) = scroll_ease(a, 1000.0, 10.0, 40.0);
    let (c, _) = scroll_ease(0.0, 1000.0, 20.0, 40.0);
    assert!((b - c).abs() < 0.01, "{b} vs {c}");
    assert!(scroll_ease(0.0, 1000.0, 5000.0, 40.0).0 < 1000.0 * 0.75, "a stalled frame is capped");
}

#[test]
fn qsmooth1_two_notches_accumulate_into_one_target() {
    let Some(mut app) = app() else { return };
    app.scroll_by(40.0);
    app.scroll_anim_step_dt(MAX, 16.0, 40.0);
    app.scroll_by(40.0);
    assert_eq!(app.scroll_target, Some(80.0));
    app.scroll_by(-200.0);
    assert_eq!(app.scroll_target, Some(0.0), "the target never goes below 0");
}

#[test]
fn qsmooth1_pixel_delta_and_programmatic_writes_are_instant_and_cancel() {
    let Some(mut app) = app() else { return };
    app.scroll_by(400.0);
    app.scroll_anim_step_dt(MAX, 16.0, 400.0);
    app.set_scroll_now(123.0);
    assert_eq!((app.scroll_offset, app.scroll_target), (123.0, None));
    run(&mut app);
    assert_eq!(app.scroll_offset, 123.0, "a stale target must not override it");
}

#[test]
fn qsmooth1_g_ends_at_max_scroll_and_gg_at_zero() {
    let Some(mut app) = app() else { return };
    app.perform_action(Action::ScrollBottom);
    assert_eq!(app.scroll_target, Some(f32::MAX));
    run(&mut app);
    assert_eq!(app.scroll_offset, MAX);
    app.perform_action(Action::ScrollTop);
    run(&mut app);
    assert_eq!(app.scroll_offset, 0.0);
}

#[test]
fn qsmooth1_tau_zero_is_instant() {
    let Some(mut app) = app() else { return };
    app.smooth_scroll_tau_ms = 0.0;
    app.scroll_by(40.0);
    assert_eq!((app.scroll_offset, app.scroll_target), (40.0, None));
    app.perform_action(Action::ScrollBottom);
    assert_eq!(app.scroll_offset, f32::MAX);
}
