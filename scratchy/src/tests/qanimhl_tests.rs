//! QANIMHL: CSS animations in the headless rendering steps (`--wpt-th-one`, `--js-report --run-timers`,
//! the test pump): the style step reports the animations the page styles, the next update's step 4
//! sends their events at the page clock, before the animation frame callbacks.
use super::*;

/// The dom/events/webkit-animation-*-event setup: an empty `@keyframes` from `insertRule`, a 1 ms
/// animation set by script.
const EMPTY_KF: &str = "const st = document.createElement('style'); document.head.appendChild(st); \
     st.sheet.insertRule('@keyframes anim {}'); \
     const d = document.createElement('div'); document.body.appendChild(d); getComputedStyle(d).width;\n";

#[test]
fn qanimhl_empty_keyframes_fire_start_and_end() {
    let log = run_with_timers(&format!(
        "{EMPTY_KF}\
         for (const k of ['animationstart', 'animationend']) d.addEventListener(k, e => console.log(k, e.animationName, e.elapsedTime, e.isTrusted));\n\
         d.style.animation = 'anim 1ms';"
    ));
    assert_eq!(log, ["log:animationstart anim 0 true", "log:animationend anim 0.001 true"], "{log:?}");
}

#[test]
fn qanimhl_events_bubble_to_window_and_raf_from_handler_runs_in_same_update() {
    // waitForEventThenAnimationFrame: a window listener asks for a frame; it runs in the same update,
    // with that update's timestamp.
    let log = run_with_timers(&format!(
        "{EMPTY_KF}\
         window.addEventListener('animationend', () => {{ const t = performance.now(); \
           requestAnimationFrame(ts => console.log('raf same update', ts === t)); }});\n\
         d.style.animation = 'anim 1ms';"
    ));
    assert_eq!(log, ["log:raf same update true"], "{log:?}");
}

#[test]
fn qanimhl_animation_events_come_before_animation_frame_callbacks() {
    // QEL1 step 4 (animation events) before step 5 (rAF): a rAF pending for the same frame runs after.
    let log = run_with_timers(&format!(
        "{EMPTY_KF}\
         d.addEventListener('animationstart', () => console.log('start'));\n\
         d.style.animation = 'anim 1ms';\n\
         requestAnimationFrame(() => {{ console.log('raf1'); requestAnimationFrame(() => console.log('raf2')); }});"
    ));
    // Frame 1 styles the animation (and runs raf1); frame 2 sends its start event, then raf2.
    assert_eq!(log, ["log:raf1", "log:start", "log:raf2"], "{log:?}");
}

#[test]
fn qanimhl_style_text_keyframes_run_on_the_virtual_clock() {
    let log = run_html_with_timers(
        "<style>@keyframes grow { from { width: 0px } to { width: 100px } } .a { animation: grow 100ms linear }</style><div id=t></div>",
        "const t = document.getElementById('t');\n\
         t.addEventListener('animationstart', () => console.log('start', Math.round(performance.now())));\n\
         t.addEventListener('animationend', e => console.log('end', Math.round(performance.now()), e.elapsedTime, getComputedStyle(t).width));\n\
         setTimeout(() => t.className = 'a', 10);\n\
         setTimeout(() => console.log('mid', getComputedStyle(t).width !== '0px' && getComputedStyle(t).width !== '100px'), 70);",
    );
    assert_eq!(log.len(), 3, "{log:?}");
    // Styled at the first frame after the write (16.7 ms), started there, ended ~100 ms later.
    assert_eq!(log[0], "log:start 33", "{log:?}");
    assert_eq!(log[1], "log:mid true", "{log:?}");
    let end: Vec<&str> = log[2].split(' ').collect();
    let at: f64 = end[1].parse().unwrap();
    assert!((116.0..=135.0).contains(&at), "{log:?}");
    assert_eq!(end[2], "0.1", "{log:?}");
}

#[test]
fn qanimhl_iteration_event_with_negative_delay() {
    // webkit-animation-iteration-event.html: `100s -99.9s 2` crosses its first iteration 100 ms in.
    let log = run_with_timers(&format!(
        "{EMPTY_KF}\
         d.addEventListener('animationiteration', e => {{ console.log('iter', e.elapsedTime); d.style.animation = ''; }});\n\
         d.style.animation = 'anim 100s -99.9s 2';"
    ));
    assert_eq!(log, ["log:iter 100"], "{log:?}");
}

#[test]
fn qanimhl_removed_element_gets_animationcancel() {
    let log = run_with_timers(&format!(
        "{EMPTY_KF}\
         d.addEventListener('animationstart', () => {{ console.log('start'); d.remove(); }});\n\
         d.addEventListener('animationcancel', e => console.log('cancel', e.animationName));\n\
         d.addEventListener('animationend', () => console.log('end'));\n\
         d.style.animation = 'anim 10s';"
    ));
    assert_eq!(log, ["log:start", "log:cancel anim"], "{log:?}");
}

#[test]
fn qanimhl_page_without_keyframes_owes_no_frame() {
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html("<div id=a></div>"),
        &[src("document.getElementById('a').style.width = '10px'; document.body.appendChild(document.createElement('p'));")],
        Rc::new(|_, _| {}),
        Some(clock),
    )
    .unwrap();
    assert!(crate::event_loop::headless_next_due(&engine).is_none());
}

#[test]
fn qanimhl_finished_animation_owes_no_more_frames() {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html(""),
        &[src(&format!("{EMPTY_KF} d.addEventListener('animationend', () => console.log('end')); d.style.animation = 'anim 1ms';"))],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    let mut frames = 0;
    while let Some(due) = crate::event_loop::headless_next_due(&engine) {
        frames += 1;
        assert!(frames < 10, "frames never stop: {:?}", out.borrow());
        clock.set(clock.get().max(due));
        crate::event_loop::headless_turn(&engine, clock.get());
    }
    assert_eq!(*out.borrow(), ["log:end"]);
    // One update styles it, the next sends start + end; then the page is idle.
    assert!(frames <= 3, "{frames}");
}
