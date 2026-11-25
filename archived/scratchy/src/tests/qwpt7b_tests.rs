// QWPT7b: the App fires the pointer, click-count and key events the headless driver fires, through the
// shared `input_host` functions (pointer_press / pointer_release / auxclick / dblclick / keypress).
#[allow(unused_imports)]
use super::*;
use crate::input_host;
use crate::wpt_input::{Driver, InputState};

const PAGE: &str = "<html><body style='margin:0'><div id=a style='width:100px;height:50px'></div></body></html>";

/// Logs type@id[:pointerId+type][:detail][:button/buttons] for the pointer, mouse and key events.
const LOG: &str = "var L = []; globalThis.L = L; window.L = L;\
    ['pointerdown','mousedown','pointerup','mouseup','click','dblclick','auxclick','contextmenu','keydown','keypress','keyup']\
    .forEach(t => document.addEventListener(t, e => { var k = t.startsWith('key'); L.push(t + '@' + (e.target.id || e.target.localName)\
      + (e.pointerType !== undefined ? ':' + e.pointerId + e.pointerType : '')\
      + (!k && !t.startsWith('pointer') ? ':d' + e.detail : '')\
      + (!k ? ':b' + e.button + '/' + e.buttons : '')); }, true));";

fn engine(html: &str, code: &str) -> (JsEngine, Handle) {
    let doc = parse_html(html);
    let eng = crate::js::start_scripts_clocked(&doc, &[src(code)], Rc::new(|_, _| {}), Some(Rc::new(std::cell::Cell::new(0.0)))).unwrap();
    (eng, doc)
}

fn take(eng: &JsEngine) -> String {
    eng.eval_value::<String>("t", "var s = L.join(' '); L.length = 0; s").unwrap()
}

fn id_of(doc: &Handle, id: &str) -> Handle {
    fn walk(h: &Handle, id: &str) -> Option<Handle> {
        if element_attr(h, "id").is_some_and(|v| *v == *id) {
            return Some(h.clone());
        }
        h.children.borrow().iter().find_map(|c| walk(c, id))
    }
    walk(doc, id).unwrap()
}

#[test]
fn pointer_events_carry_pointer_id_type_and_button_masks() {
    let (eng, doc) = engine(PAGE, LOG);
    let mut st = InputState::default();
    let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: 0.0 };
    d.pointer_move((10.0, 10.0));
    d.pointer_down(0);
    d.pointer_up(0);
    assert_eq!(take(&eng), "pointerdown@a:1mouse:b0/1 mousedown@a:d1:b0/1 pointerup@a:1mouse:b0/0 mouseup@a:d1:b0/0 click@a:d1:b0/0");
}

#[test]
fn double_and_triple_clicks_count_and_only_the_second_fires_dblclick() {
    let (eng, doc) = engine(PAGE, LOG);
    let mut st = InputState::default();
    let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: 0.0 };
    d.pointer_move((10.0, 10.0));
    for (i, t) in [0.0, 100.0, 200.0].into_iter().enumerate() {
        d.now = t;
        d.pointer_down(0);
        d.pointer_up(0);
        let s = take(&eng);
        let n = i + 1;
        assert!(s.contains(&format!("mousedown@a:d{n}")) && s.contains(&format!("click@a:d{n}")), "{s}");
        assert_eq!(s.contains("dblclick"), n == 2, "{s}");
    }
    // Too late: the count starts over.
    d.now = 2000.0;
    d.pointer_down(0);
    d.pointer_up(0);
    assert!(take(&eng).contains("click@a:d1"));
}

#[test]
fn middle_button_fires_auxclick_and_right_button_contextmenu_without_click() {
    let (eng, doc) = engine(PAGE, LOG);
    let mut st = InputState::default();
    let mut d = Driver { js: &eng, doc: doc.clone(), st: &mut st, now: 0.0 };
    d.pointer_move((10.0, 10.0));
    d.pointer_down(1);
    d.pointer_up(1);
    assert_eq!(take(&eng), "pointerdown@a:1mouse:b1/4 mousedown@a:d1:b1/4 pointerup@a:1mouse:b1/0 mouseup@a:d1:b1/0 auxclick@a:d1:b1/0");
    d.pointer_down(2);
    d.pointer_up(2);
    let s = take(&eng);
    assert!(s.starts_with("pointerdown@a:1mouse:b2/2 mousedown@a:d") && s.contains("contextmenu@a:d0:b2/2") && s.ends_with("auxclick@a:d1:b2/0"), "{s}");
    assert!(!s.contains(" click@a"), "{s}");
}

/// The App's shared host path gives the same sequences as the driver (the App's own element lookup
/// needs a layout; the events are what is compared).
#[test]
fn the_app_fires_the_same_pointer_sequences_as_the_driver() {
    let Ok(el) = test_event_loop() else { return };
    let doc = parse_html(PAGE);
    let mut app = App::new_static(doc.clone(), el.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/page").unwrap());
    app.js = run_page_scripts(&doc, &[src(LOG)], app.base_url.as_ref());
    let a = id_of(&doc, "a");
    let take_app = |app: &App| app.js.as_ref().unwrap().eval_value::<String>("t", "var s = L.join(' '); L.length = 0; s").unwrap();
    let pos = (10.0, 10.0);
    input_host::pointer_press(&mut app, Some(&a), pos, 0, 0.0);
    input_host::pointer_release(&mut app, Some(&a), pos, 0, 0.0);
    app.ptr.note_click(&a, 0.0);
    input_host::click_event(&mut app, Some(&a), pos);
    assert_eq!(take_app(&app), "pointerdown@a:1mouse:b0/1 mousedown@a:d1:b0/1 pointerup@a:1mouse:b0/0 mouseup@a:d1:b0/0 click@a:d1:b0/0");
    input_host::pointer_press(&mut app, Some(&a), pos, 0, 100.0);
    input_host::pointer_release(&mut app, Some(&a), pos, 0, 100.0);
    let n = app.ptr.note_click(&a, 100.0);
    input_host::click_event(&mut app, Some(&a), pos);
    input_host::dblclick(&mut app, &a, pos);
    assert_eq!(n, 2);
    assert_eq!(
        take_app(&app),
        "pointerdown@a:1mouse:b0/1 mousedown@a:d2:b0/1 pointerup@a:1mouse:b0/0 mouseup@a:d2:b0/0 click@a:d2:b0/0 dblclick@a:d2:b0/0"
    );
    input_host::pointer_press(&mut app, Some(&a), pos, 2, 5000.0);
    input_host::pointer_release(&mut app, Some(&a), pos, 2, 5000.0);
    input_host::auxclick(&mut app, &a, pos, 2, 1);
    let s = take_app(&app);
    assert!(s.contains("mousedown@a:d1:b2/2 contextmenu@a:d0:b2/2 pointerup") && s.ends_with("auxclick@a:d1:b2/0"), "{s}");
    assert_eq!(app.ptr.buttons, 0);
    // A keyboard-sent click has detail 0.
    input_host::click_event(&mut app, Some(&a), (0.0, 0.0));
    assert_eq!(take_app(&app), "click@a:d0:b0/0");
}

#[test]
fn a_stale_press_does_not_swallow_the_next_pointerdown() {
    let Ok(el) = test_event_loop() else { return };
    let doc = parse_html(PAGE);
    let mut app = App::new_static(doc.clone(), el.create_proxy(), None);
    app.js = run_page_scripts(&doc, &[src(LOG)], None);
    let a = id_of(&doc, "a");
    input_host::pointer_press(&mut app, Some(&a), (1.0, 1.0), 0, 0.0);
    // The release never reached the page (a UI widget took it): the next press still starts afresh.
    input_host::pointer_press(&mut app, Some(&a), (1.0, 1.0), 0, 1000.0);
    let l = app.js.as_ref().unwrap().eval_value::<String>("t", "L.join(' ')").unwrap();
    assert_eq!(l.matches("pointerdown").count(), 2, "{l}");
}

fn keys(html: &str, script: &str, mode: KeyMode) -> Option<App> {
    let el = test_event_loop().ok()?;
    let doc = parse_html(&format!("<html><body>{html}</body></html>"));
    let mut app = App::new_static(doc.clone(), el.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/page").unwrap());
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    app.key_mode = mode;
    Some(app)
}

fn tap(app: &mut App, key: &Key) {
    let t = std::time::Instant::now();
    app.route_key(key, None, true, false, t);
    app.route_key(key, None, false, false, t);
}

fn l(app: &App) -> String {
    app.js.as_ref().unwrap().eval_value::<String>("t", "var s = L.join(' '); L.length = 0; s").unwrap_or_default()
}

#[test]
fn keypress_fires_for_printable_keys_and_enter_outside_text_controls() {
    let Some(mut app) = keys("<div id=a></div>", LOG, KeyMode::Input) else { return };
    tap(&mut app, &Key::Character("x".into()));
    assert_eq!(l(&app), "keydown@body keypress@body keyup@body");
    tap(&mut app, &Key::Named(NamedKey::Enter));
    assert_eq!(l(&app), "keydown@body keypress@body keyup@body");
    tap(&mut app, &Key::Named(NamedKey::ArrowDown));
    assert_eq!(l(&app), "keydown@body keyup@body", "no keypress for non-character keys");
    app.modifiers = ModifiersState::CONTROL;
    tap(&mut app, &Key::Character("c".into()));
    assert_eq!(l(&app), "keydown@body keyup@body", "no keypress with Ctrl held");
}

#[test]
fn a_cancelled_keydown_and_normal_mode_keep_their_behaviour() {
    let cancel = format!("{LOG} document.addEventListener('keydown', e => {{ if (e.key == 'q') e.preventDefault(); }});");
    let Some(mut app) = keys("<div id=a></div>", &cancel, KeyMode::Input) else { return };
    tap(&mut app, &Key::Character("q".into()));
    assert_eq!(l(&app), "keydown@body keyup@body", "preventDefault on keydown: no keypress");
    // Normal mode: the vim keys work and the page sees no key events at all.
    let Some(mut app) = keys("<div id=a style='height:3000px'></div>", LOG, KeyMode::Normal) else { return };
    tap(&mut app, &Key::Character("x".into()));
    assert_eq!(l(&app), "");
}
