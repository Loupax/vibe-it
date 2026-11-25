// QWPT7: hit testing on the real layout (document.elementFromPoint), glyph rects of inline elements,
// and the headless trusted-input driver behind tools/wpt/testdriver-vendor.js (wpt_input.rs).
#[allow(unused_imports)]
use super::*;
use crate::wpt_input::{Driver, InputState};

const PAGE: &str = "<html><head><style>body{margin:0} div{height:50px}</style></head><body>\
    <div id=a style='width:100px'></div>\
    <div id=b style='width:200px'><div id=c style='width:20px;height:20px;margin-left:30px'></div></div>\
    <div id=f style='position:fixed;top:0;left:500px;width:50px'></div>\
    <label id=l for=cb>label</label><input id=cb type=checkbox>\
    <input id=t value=ab><button id=btn>go</button>\
    <div style='height:2000px'></div>\
    </body></html>";

fn engine(html: &str, code: &str) -> (JsEngine, Handle, Rc<RefCell<Vec<String>>>) {
    let doc = parse_html(html);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let eng = crate::js::start_scripts_clocked(&doc, &[src(code)], Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))), Some(clock))
        .unwrap();
    (eng, doc, out)
}

fn find_by_id(h: &Handle, id: &str) -> Option<Handle> {
    if element_attr(h, "id").is_some_and(|v| *v == *id) {
        return Some(h.clone());
    }
    h.children.borrow().iter().find_map(|c| find_by_id(c, id))
}

fn logs(out: &Rc<RefCell<Vec<String>>>) -> Vec<String> {
    out.borrow().iter().filter_map(|l| l.strip_prefix("log:").map(str::to_string)).collect()
}

fn ids_at(eng: &JsEngine, x: f64, y: f64) -> String {
    eng.eval_value::<String>("t", &format!("(function () {{ var e = document.elementFromPoint({x}, {y}); return e ? (e.id || e.localName) : 'null'; }})()")).unwrap()
}

#[test]
fn element_from_point_hits_the_real_layout() {
    let (eng, _doc, _) = engine(PAGE, "");
    assert_eq!(ids_at(&eng, 10.0, 10.0), "a");
    assert_eq!(ids_at(&eng, 150.0, 10.0), "body", "right of #a's box: the body");
    assert_eq!(ids_at(&eng, 35.0, 55.0), "c", "the deepest box");
    assert_eq!(ids_at(&eng, 10.0, 55.0), "b");
    assert_eq!(ids_at(&eng, 510.0, 10.0), "f", "a fixed box");
    assert_eq!(ids_at(&eng, -1.0, 10.0), "null", "outside the viewport");
    assert_eq!(ids_at(&eng, 10.0, 100_000.0), "null");
    // Scrolled: viewport point (10, 10) is document y 60 (inside #b); the fixed box stays.
    eng.eval("t", "scrollTo(0, 50);");
    assert_eq!(ids_at(&eng, 10.0, 10.0), "b");
    assert_eq!(ids_at(&eng, 510.0, 10.0), "f");
    let list = eng
        .eval_value::<String>("t", "scrollTo(0, 0); document.elementsFromPoint(35, 55).map(e => e.id || e.localName).join(',')")
        .unwrap();
    assert_eq!(list, "c,b,body,html");
    // WebIDL: two finite doubles.
    let errs = eng
        .eval_value::<String>(
            "t",
            "[() => document.elementFromPoint(), () => document.elementFromPoint(0), () => document.elementFromPoint(0, NaN), () => document.elementsFromPoint(Infinity, 0)]\
             .map(f => { try { f(); return 'ok'; } catch (e) { return e.constructor.name; } }).join(',')",
        )
        .unwrap();
    assert_eq!(errs, "TypeError,TypeError,TypeError,TypeError");
}

#[test]
fn shadow_hits_are_retargeted_to_the_host() {
    let (eng, _doc, _) = engine(
        "<html><body style='margin:0'><div id=h style='height:40px'></div></body></html>",
        "var r = document.getElementById('h').attachShadow({mode: 'open'}); r.innerHTML = \"<div id=in style='height:40px'></div>\";",
    );
    let got = eng
        .eval_value::<String>(
            "t",
            "var d = document.elementFromPoint(5, 5), s = document.getElementById('h').shadowRoot.elementFromPoint(5, 5);\
             (d && d.id) + ' ' + (s && s.id)",
        )
        .unwrap();
    // The document never sees into the shadow tree; the shadow root sees its own element (when the
    // shadow content is laid out) or the host.
    assert!(got == "h in" || got == "h h", "{got}");
}

#[test]
fn inline_elements_get_their_glyph_box_and_empty_ones_their_pen_position() {
    let (eng, _doc, _) = engine(
        "<html><body style='margin:0'><div id=d style='width:400px'><span id=x>xx</span><span id=e></span><span id=y>yyyy</span></div></body></html>",
        "",
    );
    let r = eng
        .eval_value::<Vec<f64>>(
            "t",
            "['x','e','y','d'].map(i => document.getElementById(i).getBoundingClientRect()).flatMap(r => [r.left, r.width])",
        )
        .unwrap();
    let (x_l, x_w, e_l, e_w, y_l, y_w, d_w) = (r[0], r[1], r[2], r[3], r[4], r[5], r[7]);
    assert!(x_w > 0.0 && x_w < d_w, "a span is not its line's block: {r:?}");
    assert!(y_w > x_w, "four glyphs are wider than two: {r:?}");
    assert!((y_l - (x_l + x_w)).abs() < 0.5, "spans sit side by side: {r:?}");
    assert_eq!(e_w, 0.0, "{r:?}");
    assert!((e_l - y_l).abs() < 0.5, "the empty span sits where the next text starts: {r:?}");
    // The hit test finds the inline element under a glyph.
    assert_eq!(ids_at(&eng, x_l + 1.0, 5.0), "x");
    assert_eq!(ids_at(&eng, y_l + 1.0, 5.0), "y");
}

const LOG_EVENTS: &str = "var L = []; globalThis.L = L;\
    ['pointerover','pointerenter','mouseover','mouseenter','pointermove','mousemove','pointerdown','mousedown','pointerup','mouseup','click','dblclick','auxclick','contextmenu',\
     'focus','blur','focusin','focusout','keydown','keypress','keyup','beforeinput','input','change','wheel']\
    .forEach(t => document.addEventListener(t, e => L.push(t + '@' + (e.target.id || e.target.localName) + (e.isTrusted ? '' : '!untrusted')), true));";

fn take(eng: &JsEngine) -> String {
    eng.eval_value::<String>("t", "var s = L.join(' '); L.length = 0; s").unwrap()
}

#[test]
fn a_click_runs_pointer_mouse_focus_and_activation_in_order() {
    let (eng, doc, _) = engine(PAGE, LOG_EVENTS);
    let mut st = InputState::default();
    let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: 0.0 };
    d.pointer_move((35.0, 55.0));
    assert_eq!(take(&eng), "pointerover@c pointerenter@html pointerenter@body pointerenter@b pointerenter@c mouseover@c mouseenter@html mouseenter@body mouseenter@b mouseenter@c pointermove@c mousemove@c");
    d.pointer_down(0);
    d.pointer_up(0);
    // Nothing focusable under the pointer: no focus events; the click goes to the common ancestor.
    assert_eq!(take(&eng), "pointerdown@c mousedown@c pointerup@c mouseup@c click@c");
    // A second click soon after: detail 2 and a dblclick.
    d.now = 100.0;
    d.pointer_down(0);
    d.pointer_up(0);
    assert_eq!(take(&eng), "pointerdown@c mousedown@c pointerup@c mouseup@c click@c dblclick@c");
    // Press on #c, release on #b: the click targets #b (the nearest common ancestor).
    d.now = 2000.0;
    d.pointer_down(0);
    d.pointer_move((5.0, 55.0));
    d.pointer_up(0);
    let s = take(&eng);
    assert!(s.ends_with("pointerup@b mouseup@b click@b"), "{s}");
    // Right button: contextmenu after mousedown, auxclick instead of click.
    d.pointer_down(2);
    d.pointer_up(2);
    assert_eq!(take(&eng), "pointerdown@b mousedown@b contextmenu@b pointerup@b mouseup@b auxclick@b");
}

#[test]
fn a_cancelled_pointerdown_suppresses_the_compatibility_mouse_events_but_not_click() {
    let (eng, doc, _) = engine(PAGE, &format!("{LOG_EVENTS} document.getElementById('c').addEventListener('pointerdown', e => e.preventDefault());"));
    let mut st = InputState::default();
    let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: 0.0 };
    d.pointer_move((35.0, 55.0));
    take(&eng);
    d.pointer_down(0);
    d.pointer_up(0);
    assert_eq!(take(&eng), "pointerdown@c pointerup@c click@c");
}

fn rect_center(eng: &JsEngine, id: &str) -> (f32, f32) {
    let r = eng
        .eval_value::<Vec<f64>>("t", &format!("var r = document.getElementById('{id}').getBoundingClientRect(); [r.left + r.width / 2, r.top + r.height / 2]"))
        .unwrap();
    (r[0] as f32, r[1] as f32)
}

fn click(d: &mut Driver<'_>, at: (f32, f32)) {
    d.pointer_move(at);
    d.pointer_down(0);
    d.pointer_up(0);
}

#[test]
fn checkbox_and_label_clicks_use_the_apps_activation_behaviour() {
    let (eng, doc, _) = engine(PAGE, LOG_EVENTS);
    let cb = find_by_id(&doc, "cb").unwrap();
    let (cbc, lc) = (rect_center(&eng, "cb"), rect_center(&eng, "l"));
    let mut st = InputState::default();
    let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: 0.0 };
    click(&mut d, cbc);
    assert!(live_checked(&cb));
    let s = take(&eng);
    assert!(s.contains("mousedown@cb focus@cb focusin@cb pointerup@cb mouseup@cb click@cb input@cb change@cb"), "{s}");
    assert_eq!(eng.eval_value::<String>("t", "document.activeElement.id").unwrap(), "cb");
    // The label forwards its click to the control: toggled back.
    d.now = 5000.0;
    click(&mut d, lc);
    assert!(!live_checked(&cb));
    let s = take(&eng);
    assert!(s.contains("click@l focus@cb focusin@cb click@cb input@cb change@cb"), "{s}");
    // A cancelled click restores the state.
    eng.eval("t", "document.getElementById('cb').addEventListener('click', e => e.preventDefault());");
    d.now = 10000.0;
    click(&mut d, cbc);
    assert!(!live_checked(&cb));
    assert!(!take(&eng).contains("change@cb"));
}

#[test]
fn keys_edit_a_focused_text_control_and_tab_moves_focus() {
    let (eng, doc, _) = engine(PAGE, LOG_EVENTS);
    let t = find_by_id(&doc, "t").unwrap();
    let mut st = InputState::default();
    let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: 0.0 };
    d.focus_element(&t);
    crate::forms_text::text_caret_to_end(&t);
    assert_eq!(take(&eng), "focus@t focusin@t");
    d.key_down(&['c' as u32]);
    d.key_up(&['c' as u32]);
    assert_eq!(take(&eng), "keydown@t keypress@t beforeinput@t input@t keyup@t");
    assert_eq!(eng.eval_value::<String>("t", "document.getElementById('t').value").unwrap(), "abc");
    // Shift + a: an upper-case letter, shiftKey set.
    d.key_down(&[0xE008]);
    d.key_down(&['A' as u32]);
    d.key_up(&['A' as u32]);
    d.key_up(&[0xE008]);
    assert_eq!(eng.eval_value::<String>("t", "document.getElementById('t').value").unwrap(), "abcA");
    take(&eng);
    // Backspace deletes, Tab moves focus to the button (change fires before blur).
    d.key_down(&[0xE003]);
    d.key_down(&[0xE004]);
    assert_eq!(eng.eval_value::<String>("t", "document.getElementById('t').value").unwrap(), "abc");
    let s = take(&eng);
    assert!(s.contains("change@t blur@t focusout@t focus@btn focusin@btn"), "{s}");
    // Enter on the focused button clicks it.
    d.key_down(&[0xE007]);
    assert!(take(&eng).contains("keydown@btn click@btn"));
}

#[test]
fn webdriver_special_keys_map_to_dom_keys_and_codes() {
    let key_of = |cp: u32| {
        let (k, p) = crate::wpt_input::webdriver_key(&[cp]);
        let init = crate::input_host::key_init("keydown", &k, p, false, ModifiersState::empty());
        let get = |name: &str| {
            init.iter().find(|(n, _)| *n == name).map(|(_, v)| match v {
                EventField::Str(s) => s.clone(),
                EventField::Num(n) => n.to_string(),
                _ => String::new(),
            })
        };
        (get("key").unwrap(), get("code").unwrap(), get("location").unwrap())
    };
    assert_eq!(key_of(0xE004), ("Tab".into(), "Tab".into(), "0".into()));
    assert_eq!(key_of(0xE008), ("Shift".into(), "ShiftLeft".into(), "1".into()));
    assert_eq!(key_of(0xE050), ("Shift".into(), "ShiftRight".into(), "2".into()));
    assert_eq!(key_of(0xE03D), ("Meta".into(), "SuperLeft".into(), "1".into()));
    assert_eq!(key_of(0xE00D), (" ".into(), "Space".into(), "0".into()));
    assert_eq!(key_of(0xE01A + 7), ("7".into(), "Numpad7".into(), "3".into()));
    assert_eq!(key_of('q' as u32), ("q".into(), "KeyQ".into(), "0".into()));
}

/// The vendor file, as served by the runner, against a stand-in testdriver.js: its ticks run
/// through the driver like `wpt_th::pump` runs them, and the job's promise settles after them.
#[test]
fn the_vendor_turns_actions_into_ticks_the_driver_runs() {
    let stub = "window.test_driver_internal = {};";
    let vendor = include_str!("../../tools/wpt/testdriver-vendor.js");
    let doc = parse_html(PAGE);
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let wake = Rc::new(std::cell::Cell::new(false));
    let w = wake.clone();
    let clock = Rc::new(std::cell::Cell::new(0.0));
    let eng = crate::js::start_scripts_hooked(
        &doc,
        &[src(stub), src(vendor), src(LOG_EVENTS)],
        Rc::new(move |level, text| o.borrow_mut().push(format!("{level}:{text}"))),
        Some(clock.clone()),
        &move |e| e.install_testdriver(w.clone()),
    )
    .unwrap();
    eng.eval(
        "t",
        "var c = document.getElementById('c');\
         test_driver_internal.action_sequence([\
           {type: 'pointer', id: 'p', parameters: {pointerType: 'mouse'}, actions: [{type: 'pointerMove', x: 0, y: 0, origin: c}, {type: 'pointerDown', button: 0}, {type: 'pause', duration: 40}, {type: 'pointerUp', button: 0}]},\
           {type: 'key', id: 'k', actions: [{type: 'pause'}, {type: 'keyDown', value: '\\uE008'}, {type: 'pause'}, {type: 'keyUp', value: '\\uE008'}]}\
         ]).then(() => console.log('done', L.join(' ')), e => console.log('err', e));\
         test_driver_internal.send_keys(document.getElementById('t'), 'x').then(() => console.log('keys', document.getElementById('t').value));",
    );
    assert!(wake.get(), "queueing an action wakes the runner");
    let mut st = InputState::default();
    let mut ticks = Vec::new();
    let mut at = 0.0;
    for _ in 0..20 {
        if !wake.get() {
            break;
        }
        wake.set(false);
        let Some(tick) = eng.td_next() else { continue };
        ticks.push(tick.clone());
        let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: at };
        at += d.run_tick(&tick);
        wake.set(true);
    }
    assert_eq!(ticks[0], "m 40 60\nt 0", "element origin = its in-view centre");
    assert_eq!(ticks[1], "d 0\nkd 57352\nt 0");
    assert_eq!(ticks[2], "t 40", "a tick lasts as long as its longest pause");
    assert_eq!(ticks[3], "u 0\nku 57352\nt 0");
    assert_eq!(at, 40.0);
    let l = logs(&out);
    assert!(l.iter().any(|s| s.starts_with("done pointerover@c") && s.contains("pointerdown@c mousedown@c keydown@body") && s.ends_with("click@c keyup@body")), "{l:?}");
    assert!(l.iter().any(|s| s == "keys abx"), "{l:?}");
}
