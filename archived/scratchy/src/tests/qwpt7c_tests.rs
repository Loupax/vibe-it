// QWPT7c: hit testing gaps: `pointer-events`, SVG children, element scroll offsets, offsetX/Y and
// accesskey. The App's pointer path and elementFromPoint share `paint::hit_test_element` and
// `js_geom::refine_svg_hit`; both are exercised here.
#[allow(unused_imports)]
use super::*;
use crate::wpt_input::{Driver, InputState};

fn engine(html: &str, code: &str) -> (JsEngine, Handle, Rc<RefCell<Vec<String>>>) {
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let eng = crate::js::start_scripts_clocked(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))), Some(clock))
        .unwrap();
    (eng, doc, out)
}

fn ids_at(eng: &JsEngine, x: f64, y: f64) -> String {
    eng.eval_value::<String>("t", &format!("(function () {{ var e = document.elementFromPoint({x}, {y}); return e ? (e.id || e.localName) : 'null'; }})()")).unwrap()
}

const OVERLAY: &str = "<html><head><style>body{margin:0}\
    #under{position:absolute;left:0;top:0;width:100px;height:100px}\
    #ov{position:fixed;left:0;top:0;width:300px;height:300px;pointer-events:none}\
    #ov2{pointer-events:auto;position:absolute;left:200px;top:0;width:50px;height:50px}\
    #kid{width:20px;height:20px}\
    </style></head><body><button id=under>b</button><div id=ov><div id=kid></div><div id=ov2></div></div></body></html>";

#[test]
fn pointer_events_none_is_skipped_by_element_from_point() {
    let (eng, _doc, _) = engine(OVERLAY, "");
    // The transparent full-viewport overlay and its (inherited none) child are not targets.
    assert_eq!(ids_at(&eng, 10.0, 10.0), "under");
    assert_eq!(ids_at(&eng, 150.0, 150.0), "html");
    // `pointer-events: auto` on a descendant makes it a target again.
    assert_eq!(ids_at(&eng, 210.0, 10.0), "ov2");
    let cs = eng.eval_value::<String>("t", "getComputedStyle(document.getElementById('kid')).pointerEvents + ',' + getComputedStyle(document.getElementById('ov2')).pointerEvents").unwrap();
    assert_eq!(cs, "none,auto");
}

#[test]
fn pointer_events_none_lets_a_driver_click_through() {
    let (eng, doc, _) = engine(OVERLAY, "var n = 0; document.getElementById('under').addEventListener('click', () => n++);");
    let mut st = InputState::default();
    let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: 0.0 };
    d.pointer_move((10.0, 10.0));
    d.pointer_down(0);
    d.pointer_up(0);
    assert_eq!(eng.eval_value::<f64>("t", "n").unwrap(), 1.0);
}

#[test]
fn pointer_events_none_in_the_apps_hit_test() {
    let (laid, cache) = measure_fixture_with_cache(
        "<html><body style='margin:0'><div id=a style='width:100px;height:100px'><div id=b style='pointer-events:none;width:50px;height:50px'>text</div></div></body></html>",
        400.0,
    );
    let hit = |x: f32, y: f32| crate::paint::hit_test_element(&laid, x, y, &cache, &HashMap::new(), 0.0, 600.0);
    let a = hit(80.0, 80.0);
    assert_ne!(a, 0);
    // Inside #b (and over its text): the hit is #a, never #b.
    assert_eq!(hit(5.0, 5.0), a);
    assert_eq!(hit(40.0, 40.0), a);
}

const SVG: &str = "<html><head><style>body{margin:0}</style></head><body>\
    <svg id=s width=100 height=100 viewBox='0 0 200 200'>\
      <rect id=r x=100 y=100 width=60 height=60 fill=green></rect>\
      <g id=g transform='translate(10 10)'><circle id=c cx=30 cy=30 r=20></circle></g>\
      <rect id=none x=0 y=150 width=40 height=40 pointer-events=none></rect>\
      <text id=t x=20 y=190><tspan id=ts>abcd</tspan></text>\
    </svg><div style='margin-top:200px'><button id=btn>b</button></div></body></html>";

#[test]
fn svg_children_are_hit_and_have_their_own_rects() {
    let (eng, _doc, _) = engine(SVG, "");
    // viewBox scale 0.5: #r is at (50, 50) .. (80, 80) in the page.
    assert_eq!(ids_at(&eng, 65.0, 65.0), "r");
    assert_eq!(ids_at(&eng, 10.0, 10.0), "s", "empty svg area: the svg itself");
    // The circle sits inside the translated group: centre (40, 40) user -> (20, 20) page.
    assert_eq!(ids_at(&eng, 20.0, 20.0), "c");
    assert_eq!(ids_at(&eng, 1.0, 1.0), "s", "outside the circle");
    assert_eq!(ids_at(&eng, 10.0, 85.0), "s", "pointer-events=none on the child");
    let r = eng.eval_value::<Vec<f64>>("t", "var b = document.getElementById('r').getBoundingClientRect(); [b.x, b.y, b.width, b.height]").unwrap();
    assert_eq!(r, vec![50.0, 50.0, 30.0, 30.0]);
    let c = eng.eval_value::<Vec<f64>>("t", "var b = document.getElementById('c').getBoundingClientRect(); [b.x, b.y, b.width, b.height]").unwrap();
    assert_eq!(c, vec![10.0, 10.0, 20.0, 20.0]);
    // A tspan has a box of its own, and hitting its centre hits it (not the text or the svg).
    let t = eng.eval_value::<Vec<f64>>("t", "var b = document.getElementById('ts').getBoundingClientRect(); [b.x + b.width / 2, b.y + b.height / 2]").unwrap();
    assert_eq!(ids_at(&eng, t[0], t[1]), "ts");
}

#[test]
fn offset_xy_is_relative_to_the_target_and_to_the_root_svg() {
    let code = "var log = []; window.addEventListener('click', e => log.push(e.target.id + ':' + e.offsetX + ',' + e.offsetY));";
    let (eng, doc, _) = engine(&SVG.replace("margin:0", "margin:0;padding-left:0").replace("<button", "<div id=box style='margin:0 0 0 30px;width:50px;height:20px'></div><button"), code);
    let mut st = InputState::default();
    let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: 0.0 };
    let (bx, by) = {
        let r = eng.eval_value::<Vec<f64>>("t", "var r = document.getElementById('box').getBoundingClientRect(); [r.left + 10, r.top + 5]").unwrap();
        (r[0] as f32, r[1] as f32)
    };
    for at in [(60.0, 60.0), (bx, by)] {
        d.pointer_move(at);
        d.pointer_down(0);
        d.pointer_up(0);
        d.now += 1000.0;
    }
    let log = eng.eval_value::<String>("t", "log.join(' ')").unwrap();
    // Clicking the svg's rect: relative to the root svg (at the page origin), not to the rect.
    assert!(log.starts_with("r:60,60"), "{log}");
    // Clicking the box 30 px from the left edge, 10 px in.
    assert!(log.contains("box:10,5"), "{log}");
}

#[test]
fn element_scroll_offsets_apply_to_hit_testing() {
    let html = "<html><head><style>body{margin:0}#box{width:100px;height:100px;overflow:auto}\
        .row{height:50px;width:80px}</style></head><body>\
        <div id=box><div id=r1 class=row></div><div id=r2 class=row></div><div id=r3 class=row></div><div id=r4 class=row></div></div></body></html>";
    let (eng, _doc, _) = engine(html, "");
    assert_eq!(ids_at(&eng, 10.0, 10.0), "r1");
    eng.eval("t", "document.getElementById('box').scrollTop = 100;");
    assert_eq!(ids_at(&eng, 10.0, 10.0), "r3", "scrolled by 100: the third row is at the top");
    assert_eq!(ids_at(&eng, 10.0, 70.0), "r4");
}

#[test]
fn accesskey_alt_shift_clicks_the_element_without_pointer_events() {
    let html = "<html><body><button id=b accesskey='g'>go</button><input id=i accesskey='t'><label id=l accesskey='l' for=cb>l</label><input id=cb type=checkbox></body></html>";
    let code = "var log = []; ['pointerdown','mousedown','click','focus'].forEach(t => document.addEventListener(t, e => log.push(t + '@' + e.target.id + (t == 'focus' ? '' : ':' + e.offsetX + ',' + e.clientX)), true));";
    let (eng, doc, _) = engine(html, code);
    let mut st = InputState::default();
    let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: 0.0 };
    let chord = |d: &mut Driver<'_>, k: char| {
        d.key_down(&[0xE008]);
        d.key_down(&[0xE00A]);
        d.key_down(&[k as u32]);
        d.key_up(&[k as u32]);
        d.key_up(&[0xE00A]);
        d.key_up(&[0xE008]);
    };
    chord(&mut d, 'g');
    // Plain Alt+key does nothing (it is the window manager's).
    d.key_down(&[0xE00A]);
    d.key_down(&['t' as u32]);
    d.key_up(&['t' as u32]);
    d.key_up(&[0xE00A]);
    let log = eng.eval_value::<String>("t", "log.join(' ')").unwrap();
    assert_eq!(log, "focus@b click@b:0,0", "accesskey g: focus, then only a click with default coordinates");
    eng.eval("t", "log.length = 0;");
    chord(&mut d, 't');
    assert_eq!(eng.eval_value::<String>("t", "log.join(' ')").unwrap(), "focus@i", "a text field only takes focus");
    eng.eval("t", "log.length = 0;");
    chord(&mut d, 'L');
    let log = eng.eval_value::<String>("t", "log.join(' ')").unwrap();
    assert!(log.starts_with("click@l"), "{log}");
}

#[test]
fn the_accesskey_chord_is_alt_shift_and_leaves_the_vim_keys_alone() {
    use winit::keyboard::{Key, ModifiersState};
    let k = |s: &str| Key::Character(s.into());
    let alt_shift = ModifiersState::ALT | ModifiersState::SHIFT;
    assert_eq!(crate::input_host::accesskey_chord(&k("G"), None, alt_shift).as_deref(), Some("g"));
    assert_eq!(crate::input_host::accesskey_chord(&k("g"), None, ModifiersState::ALT), None);
    assert_eq!(crate::input_host::accesskey_chord(&k("j"), None, ModifiersState::empty()), None);
    assert_eq!(crate::input_host::accesskey_chord(&k("j"), None, ModifiersState::SHIFT), None);
    assert_eq!(crate::input_host::accesskey_chord(&k("g"), None, alt_shift | ModifiersState::CONTROL), None);
}
