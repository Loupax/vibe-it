//! QANIMHLb: CSS transitions: parsing, the engine (start on a computed-value change, timing,
//! reversing, cancel) and the events in the rendering step, App-style builds and headless.
use super::*;
use crate::transition::{TransProp, parse_transition_value};

#[test]
fn qanimhlb_parses_the_shorthand_and_longhands() {
    let d = parse_transition_value("transition", "opacity 1s ease-in 200ms, color 2s").unwrap();
    assert_eq!(d.property, Some(vec![TransProp::Named("opacity".into()), TransProp::Named("color".into())]));
    assert_eq!(d.duration, Some(vec![1000.0, 2000.0]));
    assert_eq!(d.delay, Some(vec![200.0, 0.0]));
    assert_eq!(d.timing, Some(vec![TimingFunction::CubicBezier(0.42, 0.0, 1.0, 1.0), TimingFunction::EASE]));
    let d = parse_transition_value("transition", "1ms").unwrap();
    assert_eq!(d.property, Some(vec![TransProp::All]));
    let d = parse_transition_value("transition", "display 1s allow-discrete").unwrap();
    assert_eq!(d.behavior, Some(vec![true]));
    assert!(parse_transition_value("transition", "none, color 1s").is_err());
    assert!(parse_transition_value("transition-duration", "-1s").is_err());
    assert_eq!(parse_transition_value("transition-delay", "-1s, 2ms").unwrap().delay, Some(vec![-1000.0, 2.0]));
    assert_eq!(parse_transition_value("transition-property", "none").unwrap().property, Some(vec![TransProp::None]));
    // `-webkit-transition` is the same property.
    assert_eq!(crate::css::unprefixed_property("-webkit-transition"), "transition");
}

/// One App-style restyle of `div` at app clock `ms`.
fn restyle(div: &Handle, rules: &[StyleRule], ms: f64) -> ComputedStyle {
    set_app_clock_override(Some(ms));
    set_app_clock(9);
    let s = resolve_style(div, rules, ROOT_STYLE, 800.0);
    finish_animation_pass();
    s
}

#[test]
fn qanimhlb_class_change_starts_a_transition_that_runs_on_the_clock() {
    let doc = parse_html("<html><body><div class=a>x</div></body></html>");
    let rules = parse_stylesheet(".a { opacity: 1; transition: opacity 100ms linear } .a.b { opacity: 0 }");
    let div = find_element_by_tag(&doc, "div").unwrap();
    crate::transition::reset();
    // The first style: no transition.
    assert_eq!(restyle(&div, &rules, 0.0).opacity, 1.0);
    assert!(crate::transition::running_of(node_id(&div), 0).is_empty());
    crate::js_dom::set_attr(&div, "class", "a b");
    assert_eq!(restyle(&div, &rules, 10.0).opacity, 1.0, "starts at the before-change value");
    assert_eq!(crate::transition::running_of(node_id(&div), 0), vec![("opacity", 10.0, 0.0, 100.0)]);
    let mid = restyle(&div, &rules, 60.0).opacity;
    assert!((mid - 0.5).abs() < 0.01, "{mid}");
    // A second resolve of the same state changes nothing (idempotent).
    let again = restyle(&div, &rules, 60.0).opacity;
    assert_eq!(mid, again);
    assert_eq!(crate::transition::running_of(node_id(&div), 0).len(), 1);
    // Reversing halfway: back to 1 in half the duration (§3.1 shortening).
    crate::js_dom::set_attr(&div, "class", "a");
    assert!((restyle(&div, &rules, 60.0).opacity - 0.5).abs() < 0.01);
    let r = crate::transition::running_of(node_id(&div), 0);
    assert_eq!(r.len(), 1);
    assert!((r[0].3 - 50.0).abs() < 0.5, "{r:?}");
    assert_eq!(restyle(&div, &rules, 200.0).opacity, 1.0, "after its end the base value");
    crate::transition::reset();
    set_app_clock_override(None);
}

#[test]
fn qanimhlb_untransitioned_property_jumps_and_display_none_cancels() {
    let doc = parse_html("<html><body><div class=a>x</div></body></html>");
    let rules = parse_stylesheet(".a { opacity: 1; width: 10px; transition: opacity 1s } .b { opacity: 0; width: 20px } .c { display: none }");
    let div = find_element_by_tag(&doc, "div").unwrap();
    crate::transition::reset();
    restyle(&div, &rules, 0.0);
    crate::js_dom::set_attr(&div, "class", "a b");
    let s = restyle(&div, &rules, 0.0);
    assert_eq!(s.width, Some(BoxLength::Px(20.0)), "width is not transitioned");
    assert_eq!(crate::transition::running_of(node_id(&div), 0).len(), 1);
    crate::js_dom::set_attr(&div, "class", "a b c");
    restyle(&div, &rules, 100.0);
    assert!(crate::transition::running_of(node_id(&div), 0).is_empty());
    crate::transition::reset();
    set_app_clock_override(None);
}

#[test]
fn qanimhlb_tick_phases_and_frame_cap() {
    let doc = parse_html("<html><body><div class=a>x</div></body></html>");
    let rules = parse_stylesheet(".a { color: black; transition: color 100ms 50ms } .b { color: red }");
    let div = find_element_by_tag(&doc, "div").unwrap();
    crate::transition::reset();
    restyle(&div, &rules, 0.0);
    crate::js_dom::set_attr(&div, "class", "a b");
    restyle(&div, &rules, 0.0);
    let kinds = |t: &crate::transition::TransTick| t.events.iter().map(|e| format!("{}:{}:{}", e.kind, e.property, e.elapsed_s)).collect::<Vec<_>>();
    let t = crate::transition::tick(0.0, 33.0);
    assert_eq!(kinds(&t), ["transitionrun:color:0"]);
    assert_eq!(t.next, Some(50.0), "next change: the end of the delay");
    assert!(!t.restyle, "in the delay the value holds");
    let t = crate::transition::tick(50.0, 33.0);
    assert_eq!(kinds(&t), ["transitionstart:color:0"]);
    assert!(t.restyle, "an active transition asks for a frame");
    assert_eq!(t.next, Some(83.0), "the next frame after the cap");
    let t = crate::transition::tick(60.0, 33.0);
    assert!(!t.restyle, "not before the frame cap");
    let t = crate::transition::tick(150.0, 33.0);
    assert_eq!(kinds(&t), ["transitionend:color:0.1"]);
    assert!(t.restyle);
    restyle(&div, &rules, 150.0);
    assert!(!crate::transition::has_running(node_id(&div), 0));
    let t = crate::transition::tick(160.0, 33.0);
    assert!(t.events.is_empty() && !t.restyle && t.next.is_none());
    assert!(!crate::transition::live(), "nothing runs: no clock needed");
    crate::transition::reset();
    set_app_clock_override(None);
}

#[test]
fn qanimhlb_child_follows_transitioning_parent_without_its_own_transition() {
    let doc = parse_html("<html><body><div class=a><p>x</p></div></body></html>");
    let rules = parse_stylesheet("* { transition: color 100ms linear } .a { color: black } .b { color: red }");
    let div = find_element_by_tag(&doc, "div").unwrap();
    let p = find_element_by_tag(&doc, "p").unwrap();
    crate::transition::reset();
    let style = |ms: f64| {
        set_app_clock_override(Some(ms));
        set_app_clock(9);
        let d = resolve_style(&div, &rules, ROOT_STYLE, 800.0);
        let c = resolve_style(&p, &rules, d.inherited(), 800.0);
        finish_animation_pass();
        (d, c)
    };
    style(0.0);
    crate::js_dom::set_attr(&div, "class", "b");
    style(0.0);
    let (d, c) = style(50.0);
    assert_eq!(d.color, c.color, "the child inherits the parent's transitioning value");
    assert!(crate::transition::running_of(node_id(&p), 0).is_empty(), "no transition of its own");
    crate::transition::reset();
    set_app_clock_override(None);
}

/// The dom/events/webkit-transition-end-event setup.
const WEBKIT_SETUP: &str = "const st = document.createElement('style'); document.head.appendChild(st); \
     st.sheet.insertRule('.baseStyle { width: 100px; transition: width 1ms; }'); \
     st.sheet.insertRule('.transition { width: 200px !important; }'); \
     const d = document.createElement('div'); d.classList.add('baseStyle'); document.body.appendChild(d); \
     getComputedStyle(d).transition; getComputedStyle(d).width;\n";

#[test]
fn qanimhlb_headless_transition_events_in_order() {
    let log = run_with_timers(&format!(
        "{WEBKIT_SETUP}\
         for (const k of ['transitionrun', 'transitionstart', 'transitionend', 'transitioncancel']) \
           d.addEventListener(k, e => console.log(k, e.propertyName, e.elapsedTime, e.isTrusted, e instanceof TransitionEvent));\n\
         d.classList.add('transition');"
    ));
    assert_eq!(
        log,
        ["log:transitionrun width 0 true true", "log:transitionstart width 0 true true", "log:transitionend width 0.001 true true"],
        "{log:?}"
    );
}

#[test]
fn qanimhlb_legacy_webkit_transition_end_listener() {
    let log = run_with_timers(&format!(
        "{WEBKIT_SETUP}\
         d.onwebkittransitionend = e => console.log('legacy', e.type);\n\
         window.addEventListener('transitionend', () => requestAnimationFrame(() => console.log('raf')));\n\
         d.classList.add('transition');"
    ));
    assert_eq!(log, ["log:legacy webkitTransitionEnd", "log:raf"], "{log:?}");
}

#[test]
fn qanimhlb_inline_style_transition_and_value_midway() {
    // EventListener-invoke-legacy's setupTransition, with a long duration read midway.
    let log = run_with_timers(
        "const e = document.createElement('div'); document.body.appendChild(e);\n\
         getComputedStyle(e).width;\n\
         e.style.width = '100px'; e.style.transition = 'width 100ms linear';\n\
         e.addEventListener('transitionend', ev => console.log('end', ev.propertyName, getComputedStyle(e).width));\n\
         setTimeout(() => { const w = parseFloat(getComputedStyle(e).width); console.log('mid', w > 0 && w < 100); }, 70);",
    );
    // `width: auto` -> 100px does not interpolate: no transition, no event.
    assert_eq!(log, ["log:mid false"], "{log:?}");
    let log = run_with_timers(
        "const e = document.createElement('div'); e.style.width = '0px'; document.body.appendChild(e);\n\
         getComputedStyle(e).width;\n\
         e.style.width = '100px'; e.style.transition = 'width 100ms linear';\n\
         e.addEventListener('transitionend', ev => console.log('end', ev.propertyName, getComputedStyle(e).width));\n\
         setTimeout(() => { const w = parseFloat(getComputedStyle(e).width); console.log('mid', w > 0 && w < 100); }, 70);",
    );
    assert_eq!(log, ["log:mid true", "log:end width 100px"], "{log:?}");
}

#[test]
fn qanimhlb_display_none_sends_transitioncancel() {
    let log = run_with_timers(&format!(
        "{}\
         d.addEventListener('transitionrun', () => {{ console.log('run'); d.style.display = 'none'; }});\n\
         d.addEventListener('transitioncancel', e => console.log('cancel', e.propertyName));\n\
         d.addEventListener('transitionend', () => console.log('end'));\n\
         d.classList.add('transition');",
        WEBKIT_SETUP.replace("width 1ms", "width 10s")
    ));
    assert_eq!(log, ["log:run", "log:cancel width"], "{log:?}");
}

#[test]
fn qanimhlb_finished_transition_owes_no_more_frames() {
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let engine = crate::js::start_scripts_clocked(
        &parse_html(""),
        &[src(&format!("{WEBKIT_SETUP} d.addEventListener('transitionend', () => console.log('end')); d.classList.add('transition');"))],
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
    assert!(frames <= 4, "{frames}");
}
