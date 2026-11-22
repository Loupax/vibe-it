//! QEL3: IntersectionObserver is evaluated once per rendering update (step 7, after ResizeObserver),
//! its callbacks are queued as a task; no timer exists for it.
use super::*;

const PRE: &str = "const $ = id => document.getElementById(id);\n";

#[test]
fn qel3_observe_creates_no_timer_and_waits_for_the_frame() {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html("<div id=a style='height:50px'></div>"),
        &[src(&format!(
            "{PRE}new IntersectionObserver(es => console.log('io', es.length)).observe($('a')); console.log('sync');"
        ))],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    assert_eq!(*out.borrow(), ["log:sync"]);
    // No timer; only the rendering frame is owed.
    assert!(engine.next_due_ms().is_none());
    let due = crate::event_loop::headless_next_due(&engine).expect("a rendering update is owed");
    // A timer-style pump never delivers it.
    assert_eq!(engine.run_due_counted(), 0);
    assert_eq!(*out.borrow(), ["log:sync"]);
    clock.set(due);
    // The update computes the entries and queues the callback as a task: not run inside the update.
    crate::event_loop::headless_frame(&engine, due);
    assert_eq!(*out.borrow(), ["log:sync"]);
    assert!(engine.next_due_ms().is_some());
    engine.run_due_counted();
    assert_eq!(*out.borrow(), ["log:sync", "log:io 1"]);
    // Nothing is owed afterwards.
    assert!(crate::event_loop::headless_next_due(&engine).is_none());
}

#[test]
fn qel3_ro_notifications_come_before_io_in_one_update() {
    let log = run_html_with_timers(
        "<div id=a style='height:50px'></div>",
        &format!("{PRE}new IntersectionObserver(() => console.log('io')).observe($('a')); new ResizeObserver(() => console.log('ro')).observe($('a'));"),
    );
    assert_eq!(log, ["log:ro", "log:io"], "{log:?}");
}

#[test]
fn qel3_entries_of_one_update_share_the_frame_time() {
    let log = run_html_with_timers(
        "<div id=a style='height:50px'></div><div id=b></div>",
        &format!(
            "{PRE}new IntersectionObserver(es => console.log('x', es[0].time)).observe($('a')); \
             new IntersectionObserver(es => console.log('y', es[0].time)).observe($('b'));"
        ),
    );
    assert_eq!(log.len(), 2, "{log:?}");
    let t = |s: &str| s.rsplit(' ').next().unwrap().to_string();
    assert_eq!(t(&log[0]), t(&log[1]), "{log:?}");
    assert!(t(&log[0]).parse::<f64>().unwrap() > 0.0, "{log:?}");
}
