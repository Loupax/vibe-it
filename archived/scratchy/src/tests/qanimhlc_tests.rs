//! QANIMHLc: geometry reads during a running CSS animation or transition see the animated box at the
//! current document clock, not at the first frame; pages with nothing animated get no extra layouts.
use super::*;

fn reads(css: &str, props: &str) -> Vec<String> {
    run_with_timers(&format!(
        "const st = document.createElement('style'); st.textContent = `{css}`; document.head.appendChild(st);\n\
         const d = document.createElement('div'); d.id = 'd'; document.body.appendChild(d);\n\
         const read = t => {{ const r = d.getBoundingClientRect(); console.log(t, {props}); }};\n\
         read(0);\n\
         setTimeout(() => read(1), 200);\n\
         setTimeout(() => read(2), 600);"
    ))
}

fn nums(log: &[String]) -> Vec<f64> {
    log.iter().map(|l| l.rsplit(' ').next().unwrap().parse().unwrap()).collect()
}

#[test]
fn qanimhlc_animated_left_differs_between_reads() {
    let log = reads(
        "@keyframes mv { from { left: 0px } to { left: 400px } } \
         #d { position: absolute; top: 0; width: 10px; height: 10px; animation: mv 800ms linear forwards }",
        "r.left",
    );
    let v = nums(&log);
    assert_eq!(v.len(), 3, "{log:?}");
    assert!(v[0] < v[1] && v[1] < v[2], "left must advance with the clock: {log:?}");
}

#[test]
fn qanimhlc_animated_width_and_offset_width_differ_between_reads() {
    let log = reads(
        "@keyframes gr { from { width: 10px } to { width: 410px } } \
         #d { height: 10px; animation: gr 800ms linear forwards }",
        "d.offsetWidth",
    );
    let v = nums(&log);
    assert_eq!(v.len(), 3, "{log:?}");
    assert!(v[0] < v[1] && v[1] < v[2], "offsetWidth must advance with the clock: {log:?}");
}

#[test]
fn qanimhlc_animated_transform_differs_between_reads() {
    let log = reads(
        "@keyframes tr { from { transform: translateX(0px) } to { transform: translateX(400px) } } \
         #d { width: 10px; height: 10px; animation: tr 800ms linear forwards }",
        "r.left",
    );
    let v = nums(&log);
    assert_eq!(v.len(), 3, "{log:?}");
    assert!(v[0] < v[1] && v[1] < v[2], "transformed left must advance with the clock: {log:?}");
}

#[test]
fn qanimhlc_transitioning_width_differs_between_reads() {
    let log = run_with_timers(
        "const d = document.createElement('div'); d.style.cssText = 'width: 0px; height: 10px; transition: width 800ms linear';\n\
         document.body.appendChild(d); d.getBoundingClientRect();\n\
         d.style.width = '400px';\n\
         const read = t => console.log(t, d.getBoundingClientRect().width);\n\
         read(0); setTimeout(() => read(1), 200); setTimeout(() => read(2), 600);",
    );
    let v = nums(&log);
    assert_eq!(v.len(), 3, "{log:?}");
    assert!(v[0] < v[1] && v[1] < v[2], "{log:?}");
}

#[test]
fn qanimhlc_no_extra_layouts_without_animations() {
    let before = crate::js_geom::GEOM_LAYOUTS.with(|c| c.get());
    let log = run_with_timers(
        "const d = document.createElement('div'); d.style.cssText = 'width: 10px; height: 10px'; document.body.appendChild(d);\n\
         d.getBoundingClientRect();\n\
         setTimeout(() => { d.getBoundingClientRect(); setTimeout(() => { d.getBoundingClientRect(); console.log('done'); }, 100); }, 100);",
    );
    assert_eq!(log, ["log:done"], "{log:?}");
    assert_eq!(crate::js_geom::GEOM_LAYOUTS.with(|c| c.get()) - before, 1, "reads without a DOM write must not lay out again");
}
