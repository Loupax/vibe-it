// QEL1 (R64 section 4, E1-E3): the rendering-steps driver (src/event_loop.rs) and the rAF list.
#[allow(unused_imports)]
use super::*;
use crate::event_loop::{FRAME_MS, RenderingHost, headless_frame, headless_frame_due, update_the_rendering};

type Clocked = (crate::js::JsEngine, Rc<std::cell::Cell<f64>>, Rc<RefCell<Vec<String>>>);

/// An engine on a manual clock whose console lines go to the returned log.
fn clocked(code: &str) -> Clocked {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html(""),
        &[src(code)],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
    )
    .unwrap();
    (engine, clock, out)
}

/// E2: rAF ids are their own space (1, 2, 3 whatever timers did), and the two cancel functions do
/// not cross (WPT no-interference-timers).
#[test]
fn qel1_raf_ids_are_their_own_space() {
    let log = run_with_timers(
        "setTimeout(function(){}, 0); var a = requestAnimationFrame(function(){});\
         setTimeout(function(){}, 0); var b = requestAnimationFrame(function(){});\
         var iv = setInterval(function(){}, 0); clearInterval(iv); var c = requestAnimationFrame(function(){});\
         console.log('ids', a, b, c);\
         clearTimeout(requestAnimationFrame(function(){ console.log('raf survives clearTimeout'); }));\
         clearInterval(requestAnimationFrame(function(){ console.log('raf survives clearInterval'); }));\
         for (var k = 0; k < 10; k++) setTimeout(function(){}, 0);  /* timer ids now past every rAF id */\
         cancelAnimationFrame(setTimeout(function(){ console.log('timeout survives cancelAnimationFrame'); }, 0));",
    );
    assert_eq!(log[0], "log:ids 1 2 3", "{log:?}");
    for want in ["log:raf survives clearTimeout", "log:raf survives clearInterval", "log:timeout survives cancelAnimationFrame"] {
        assert!(log.iter().any(|l| l == want), "missing {want:?} in {log:?}");
    }
}

/// E1/E2: one batch per frame: every callback gets the same timestamp, a microtask checkpoint runs
/// after each callback, a callback cancelled by an earlier one in the batch does not run, and a rAF
/// requested inside the batch waits for the next frame. More than the timer pump's 100-per-turn cap
/// still runs in ONE batch (never split).
#[test]
fn qel1_raf_batch_one_timestamp_microtasks_cancel_and_next_frame() {
    let (engine, clock, log) = clocked(
        "var ts = [];\
         requestAnimationFrame(function(t){ ts.push(t); console.log('raf1'); Promise.resolve().then(function(){ console.log('micro1'); }); cancelAnimationFrame(c3);\
           requestAnimationFrame(function(t2){ console.log('next frame ' + (t2 > t)); }); });\
         requestAnimationFrame(function(t){ ts.push(t); console.log('raf2'); });\
         var c3 = requestAnimationFrame(function(){ console.log('cancelled ran'); });\
         for (var i = 0; i < 150; i++) requestAnimationFrame(function(t){ ts.push(t); });\
         setTimeout(function(){ console.log('timeout0'); }, 0);",
    );
    assert_eq!(engine.next_due_ms(), Some(0.0), "the timeout is a timer; rAFs are not");
    let due = headless_frame_due(&engine).expect("a frame is due");
    assert!((due - FRAME_MS).abs() < 1e-9, "first frame at the 60 Hz boundary after 0: {due}");
    engine.run_due_counted();
    clock.set(due);
    let ran = headless_frame(&engine, due);
    assert_eq!(ran, 152, "the whole batch in one update (150 + raf1 + raf2; c3 cancelled)");
    let same: bool = engine.eval_value("t", "ts.length === 152 && ts.every(function(x){ return x === ts[0]; })").unwrap();
    assert!(same, "one timestamp for the batch");
    let l: Vec<String> = log.borrow().iter().filter(|l| l.starts_with("log:")).cloned().collect();
    assert_eq!(l, ["log:timeout0", "log:raf1", "log:micro1", "log:raf2"], "{l:?}");
    let next = headless_frame_due(&engine).expect("the nested rAF waits");
    assert!(next > due, "nested rAF in a later frame: {next} vs {due}");
    clock.set(next);
    headless_frame(&engine, next);
    assert!(log.borrow().iter().any(|l| l == "log:next frame true"), "{:?}", log.borrow());
    assert!(!log.borrow().iter().any(|l| l.contains("cancelled ran")), "{:?}", log.borrow());
}

/// "Report the exception" for rAF callbacks (WPT animation-frames/callback-exception): an
/// ErrorEvent at window, and the other callbacks of the batch still run.
#[test]
fn qel1_raf_exception_is_reported_at_window() {
    let log = run_with_timers(
        "addEventListener('error', function(e){ console.log('error event ' + e.error.message); e.preventDefault(); });\
         requestAnimationFrame(function(){ throw new Error('boom'); });\
         requestAnimationFrame(function(){ console.log('second ran'); });",
    );
    assert_eq!(log, ["log:error event boom", "log:second ran"], "{log:?}");
}

/// R64 section 4 order probe (the parts QEL1 owns; RO/IO sizes and delivery are QEL2/QEL3):
/// a timer task and its microtask, then the frame: rAF 1, its microtask, rAF 2 with the same
/// timestamp, then the next frame.
#[test]
fn qel1_order_probe_task_microtask_raf_batch() {
    let log = run_with_timers(
        "var t1;\
         setTimeout(function(){ console.log('timeout0'); Promise.resolve().then(function(){ console.log('micro-after-timeout0'); }); }, 0);\
         requestAnimationFrame(function(t){ t1 = t; console.log('raf1'); Promise.resolve().then(function(){ console.log('micro-after-raf1'); });\
           requestAnimationFrame(function(){ console.log('raf-next-frame'); }); });\
         requestAnimationFrame(function(t){ console.log('raf2 same ts ' + (t === t1)); });",
    );
    assert_eq!(log, ["log:timeout0", "log:micro-after-timeout0", "log:raf1", "log:micro-after-raf1", "log:raf2 same ts true", "log:raf-next-frame"], "{log:?}");
}

/// A host that records which step ran when; the rAF callback logs into the same list.
struct Probe {
    js: crate::js::JsEngine,
    steps: Rc<RefCell<Vec<String>>>,
}

impl RenderingHost for Probe {
    fn engine(&self) -> Option<&crate::js::JsEngine> {
        Some(&self.js)
    }
    fn run_resize_steps(&mut self) {
        self.steps.borrow_mut().push("resize".into());
    }
    fn run_scroll_steps(&mut self) {
        self.steps.borrow_mut().push("scroll".into());
    }
    fn evaluate_media_queries(&mut self) {
        self.steps.borrow_mut().push("mql".into());
    }
    fn update_animations(&mut self) {
        self.steps.borrow_mut().push("animations".into());
    }
    fn update_style_layout(&mut self) {
        self.steps.borrow_mut().push("style+layout".into());
    }
    fn run_observers(&mut self) {
        self.steps.borrow_mut().push("observers".into());
    }
    fn paint(&mut self) {
        self.steps.borrow_mut().push("paint".into());
    }
}

/// E3: the driver runs the HTML order: resize -> scroll -> MQL -> animations -> rAF -> style/layout
/// -> observers -> paint.
#[test]
fn qel1_driver_runs_the_html_step_order() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let s = steps.clone();
    let js = crate::js::start_scripts_clocked(
        &parse_html(""),
        &[src("requestAnimationFrame(function(){ console.log('rAF'); });")],
        Rc::new(move |level, text| {
            if level == "log" {
                s.borrow_mut().push(text.to_string());
            }
        }),
        Some(Rc::new(std::cell::Cell::new(0.0))),
    )
    .unwrap();
    let mut host = Probe { js, steps: steps.clone() };
    assert_eq!(update_the_rendering(&mut host, Some(FRAME_MS)), 1);
    assert_eq!(*steps.borrow(), ["resize", "scroll", "mql", "animations", "rAF", "style+layout", "observers", "paint"]);
    // An update that is not an animation frame (only a resize or an animation event is due) runs
    // the other steps and leaves the rAF list alone.
    host.js.eval("t", "requestAnimationFrame(function(){ console.log('early rAF'); });");
    steps.borrow_mut().clear();
    assert_eq!(update_the_rendering(&mut host, None), 0);
    assert_eq!(*steps.borrow(), ["resize", "scroll", "mql", "animations", "style+layout", "observers", "paint"]);
    assert!(host.js.raf_due_ms().is_some(), "the rAF still waits for its frame");
}
