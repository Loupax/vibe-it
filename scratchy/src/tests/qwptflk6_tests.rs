//! QWPTFLK6: `performance.timeOrigin + performance.now()` must track `Date.now()` even when `now()` is a
//! manual clock (the WPT runner's virtual clock) and real time passes before the page script runs.
#[allow(unused_imports)]
use super::*;

fn run(js: &str) -> String {
    let doc = parse_html("<html><body></body></html>");
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let scripts = vec![src(js)];
    let clock = Rc::new(std::cell::Cell::new(0.0));
    start_scripts_clocked(&doc, &scripts, Rc::new(move |_l, t| o.borrow_mut().push(t.to_string())), Some(clock)).unwrap();
    let lines = out.borrow();
    lines.join("\n")
}

#[test]
fn time_origin_tracks_date_now_under_a_manual_clock_after_a_slow_start() {
    let r = run("var s = Date.now(); while (Date.now() - s < 120) {}\
        var o = performance.timeOrigin;\
        console.log(JSON.stringify({ ok: Date.now() <= o + performance.now() + 30 && o <= Date.now() + 30,\
          stable: performance.timeOrigin === o, json: performance.toJSON().timeOrigin === o }));");
    assert_eq!(r, r#"{"ok":true,"stable":true,"json":true}"#);
}
