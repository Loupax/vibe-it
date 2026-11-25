#[allow(unused_imports)]
use super::*;

// ---------- QWPT7a: focus()/blur() fire the focus events synchronously ----------

fn engine(html: &str, code: &str) -> JsEngine {
    let doc = parse_html(html);
    start_scripts(&doc, &[src(code)], Rc::new(|_, _| {})).unwrap()
}

const LOG: &str = "var log=[]; var a=document.getElementById('a'), b=document.getElementById('b');
  ['blur','focus','focusin','focusout'].forEach(function(t){ document.addEventListener(t, function(e){
    log.push(t+':'+e.target.id+'>'+(e.relatedTarget?e.relatedTarget.id:'-')); }, true); });";
const PAGE: &str = "<html><body><input id=a><input id=b><div id=d></div></body></html>";

fn run(code: &str) -> String {
    let e = engine(PAGE, &format!("{LOG} {code} globalThis.out = log.join(' ');"));
    e.eval_value::<String>("t", "out").unwrap()
}

#[test]
fn focus_fires_events_before_it_returns() {
    assert_eq!(run("a.focus(); log.push('ret');"), "focus:a>- focusin:a>- ret");
}

#[test]
fn moving_focus_fires_blur_focusout_then_focus_focusin_with_related_target() {
    assert_eq!(run("a.focus(); log.length=0; b.focus();"), "blur:a>b focusout:a>b focus:b>a focusin:b>a");
}

#[test]
fn focusing_the_focused_element_and_blurring_an_unfocused_one_fire_nothing() {
    assert_eq!(run("a.focus(); log.length=0; a.focus(); b.blur(); document.activeElement.focus();"), "");
}

#[test]
fn blur_fires_blur_and_focusout_without_related_target() {
    assert_eq!(run("a.focus(); log.length=0; a.blur();"), "blur:a>- focusout:a>-");
}

#[test]
fn unfocusable_element_fires_nothing() {
    assert_eq!(run("document.getElementById('d').focus();"), "");
}

#[test]
fn focus_matches_the_focus_pseudo_class_at_once() {
    let e = engine(
        PAGE,
        "var a=document.getElementById('a'); a.focus(); var r=[a.matches(':focus')]; a.blur(); r.push(a.matches(':focus')); globalThis.out=r.join();",
    );
    assert_eq!(e.eval_value::<String>("t", "out").unwrap(), "true,false");
}
