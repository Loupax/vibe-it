// QEL7: `scroll` / `scrollend` from the rendering update's scroll steps, `wheel` events, MediaQueryList
// `change` events.
#[allow(unused_imports)]
use super::*;

const HTML: &str = "<html><body style='margin:0'>\
    <div id=s style='overflow:auto;width:100px;height:50px'><div style='height:500px'></div></div>\
    <div id=t style='overflow:auto;width:100px;height:50px'><div style='height:500px'></div></div>\
    </body></html>";

fn run(code: &str) -> Vec<String> {
    run_html_with_timers(HTML, code)
}

fn logs(log: &[String]) -> Vec<&str> {
    log.iter().filter_map(|l| l.strip_prefix("log:")).collect()
}

#[test]
fn window_scroll_fires_once_at_the_document_in_the_rendering_step() {
    let log = run(
        "window.addEventListener('scroll', e => console.log('scroll', e.target === document, e.bubbles, e.isTrusted));\
         document.addEventListener('scroll', () => console.log('doc'));\
         requestAnimationFrame(() => console.log('raf'));\
         scrollTo(0, 10); scrollTo(0, 20); scrollBy(0, 5);\
         console.log('sync', scrollY);",
    );
    // One event for three scrolls (coalesced), after the script, before the frame callbacks.
    assert_eq!(logs(&log), ["sync 25", "doc", "scroll true true true", "raf"], "{log:?}");
}

#[test]
fn unchanged_scroll_position_fires_nothing() {
    let log = run(
        "window.addEventListener('scroll', () => console.log('scroll'));\
         window.addEventListener('scrollend', () => console.log('scrollend'));\
         scrollTo(0, 0); scrollBy(0, 0);",
    );
    assert!(logs(&log).is_empty(), "{log:?}");
}

#[test]
fn scrollend_follows_the_last_scroll_event_and_does_not_repeat() {
    let log = run(
        "var n = 0;\
         window.addEventListener('scroll', () => console.log('scroll'));\
         window.addEventListener('scrollend', e => console.log('scrollend', e.target === document, e.bubbles));\
         document.onscrollend = () => console.log('onscrollend');\
         scrollTo(0, 30);\
         setTimeout(() => scrollTo(0, 60), 40);",
    );
    // The second scroll (a few frames later) is its own sequence: scroll, then scrollend.
    assert_eq!(
        logs(&log),
        ["scroll", "onscrollend", "scrollend true true", "scroll", "onscrollend", "scrollend true true"],
        "{log:?}"
    );
}

#[test]
fn element_scroll_fires_at_the_element_only_and_coalesces() {
    let log = run(
        "var s = document.getElementById('s'), t = document.getElementById('t');\
         s.addEventListener('scroll', e => console.log('s', e.target.id, e.bubbles, s.scrollTop));\
         t.addEventListener('scroll', () => console.log('t'));\
         document.addEventListener('scroll', () => console.log('doc'), true);\
         window.addEventListener('scroll', () => console.log('win'));\
         s.addEventListener('scrollend', () => console.log('s-end'));\
         s.scrollTop = 10; s.scrollTop = 40; s.scrollBy(0, 2);",
    );
    // The capture listener on the document sees it (capture still runs); the bubble listener on window does not.
    assert_eq!(logs(&log), ["doc", "s s false 42", "s-end"], "{log:?}");
}

#[test]
fn element_scroll_to_the_same_offset_fires_nothing() {
    let log = run(
        "var s = document.getElementById('s');\
         s.addEventListener('scroll', () => console.log('s'));\
         s.scrollTop = 0; s.scrollTop = 10;",
    );
    assert_eq!(logs(&log), ["s"], "{log:?}");
}

#[test]
fn scroll_events_run_before_animation_frame_callbacks_in_the_same_frame() {
    let log = run(
        "var s = document.getElementById('s');\
         s.addEventListener('scroll', () => console.log('scroll'));\
         requestAnimationFrame(() => { console.log('raf1'); s.scrollTop = 5; });\
         requestAnimationFrame(() => console.log('raf2'));",
    );
    // The scroll made inside a frame callback is reported by the NEXT update.
    assert_eq!(logs(&log), ["raf1", "raf2", "scroll"], "{log:?}");
}

fn engine(html: &str, code: &str, log: &Rc<RefCell<Vec<String>>>) -> crate::js::JsEngine {
    let o = log.clone();
    crate::js::start_scripts_clocked(
        &parse_html(html),
        &[src(code)],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(Rc::new(std::cell::Cell::new(0.0))),
    )
    .unwrap()
}

#[test]
fn wheel_event_carries_deltas_and_preventdefault_is_reported() {
    use crate::js::EventField::{Bool, Num};
    let log = Rc::new(RefCell::new(Vec::new()));
    let js = engine(
        HTML,
        "var s = document.getElementById('s');\
         s.addEventListener('wheel', e => { console.log('wheel', e instanceof WheelEvent, e.deltaX, e.deltaY, e.deltaMode, e.cancelable, e.isTrusted); e.preventDefault(); });\
         document.addEventListener('wheel', e => console.log('doc-passive', e.cancelable));",
        &log,
    );
    // Dispatch at the document's body (None): reaches the document listener only; nothing cancels it.
    let init = [("bubbles", Bool(true)), ("cancelable", Bool(true)), ("deltaX", Num(0.0)), ("deltaY", Num(100.0)), ("deltaMode", Num(0.0))];
    let ok = js.dispatch_event(None, "wheel", &init);
    assert!(ok, "a passive document listener cannot cancel");
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x == "log:doc-passive true"), "{l:?}");
    assert!(js.pointer_listener_mask() & (1 << 8) != 0, "a wheel listener is visible to the App's cheap check");
}

#[test]
fn wheel_listener_on_an_element_can_cancel() {
    use crate::js::EventField::{Bool, Num};
    let log2 = Rc::new(RefCell::new(Vec::new()));
    let js2 = engine(
        HTML,
        "document.body.addEventListener('wheel', e => { console.log('wheel', e instanceof WheelEvent, e.deltaX, e.deltaY, e.deltaMode, e.cancelable, e.isTrusted); e.preventDefault(); }, { passive: false });",
        &log2,
    );
    let init = [("bubbles", Bool(true)), ("cancelable", Bool(true)), ("deltaX", Num(-3.0)), ("deltaY", Num(100.0)), ("deltaMode", Num(0.0))];
    let ok = js2.dispatch_event(None, "wheel", &init);
    assert!(!ok, "preventDefault stops the scroll");
    let l = log2.borrow().clone();
    assert!(l.iter().any(|x| x == "log:wheel true -3 100 0 true true"), "{l:?}");
}

#[test]
fn media_query_list_change_fires_when_the_viewport_flips_the_result() {
    let log = Rc::new(RefCell::new(Vec::new()));
    crate::js::set_window_viewport(800, 600);
    let js = engine(
        HTML,
        "var m = matchMedia('(max-width: 500px)');\
         m.addEventListener('change', e => console.log('change', e instanceof MediaQueryListEvent, e.matches, e.media, m.matches, e.isTrusted));\
         m.onchange = e => console.log('onchange', e.matches);\
         var wide = matchMedia('(min-width: 300px)'); wide.addListener(e => console.log('wide', e.matches));\
         console.log('start', m.matches);",
        &log,
    );
    let frame = |now: f64| {
        crate::event_loop::update_the_rendering(&mut crate::event_loop::Headless { js: &js, now }, None);
    };
    frame(16.0);
    assert!(!log.borrow().iter().any(|l| l.contains("change")), "{:?}", log.borrow());
    // 800 -> 400: `(max-width: 500px)` flips to true; `(min-width: 300px)` stays true.
    crate::js::set_window_viewport(400, 300);
    js.set_viewport(400, 300);
    frame(33.0);
    frame(50.0);
    let l = log.borrow().clone();
    let got: Vec<&str> = l.iter().filter_map(|x| x.strip_prefix("log:")).collect();
    assert_eq!(got, ["start false", "change true true (max-width: 500px) true true", "onchange true"], "{l:?}");
    // And back: false again.
    crate::js::set_window_viewport(800, 600);
    js.set_viewport(800, 600);
    frame(66.0);
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x == "log:onchange false"), "{l:?}");
}

#[test]
fn media_query_list_without_listeners_costs_nothing() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let js = engine(HTML, "var m = matchMedia('(max-width: 500px)'); console.log(m.matches);", &log);
    assert_eq!(js.qel7_bits(), 0, "an unobserved list registers nothing");
}
