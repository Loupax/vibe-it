// QKEYS2: input mode / normal mode, Esc Esc, `i` (KEYS.md T1-T21, T29-T34).
#[allow(unused_imports)]
use super::*;
use std::time::{Duration, Instant};

fn esc() -> Key {
    Key::Named(NamedKey::Escape)
}

fn chr(s: &str) -> Key {
    Key::Character(s.into())
}

fn ms(base: Instant, n: u64) -> Instant {
    base + Duration::from_millis(n)
}

#[test]
fn keys_t01_t02_esc_esc_within_300ms_enters() {
    let t0 = Instant::now();
    let none = ModifiersState::empty();
    let (s, enter) = resolve_esc_esc(EscState::Idle, &esc(), none, false, t0);
    assert!(!enter && s == EscState::Esc1(t0));
    let (s, enter) = resolve_esc_esc(s, &esc(), none, false, ms(t0, 300));
    assert!(enter && s == EscState::Idle, "exactly ESC_ESC_MS still counts");
}

#[test]
fn keys_t03_slow_second_esc_is_a_new_first_esc() {
    let t0 = Instant::now();
    let none = ModifiersState::empty();
    let (s, _) = resolve_esc_esc(EscState::Idle, &esc(), none, false, t0);
    let (s, enter) = resolve_esc_esc(s, &esc(), none, false, ms(t0, 301));
    assert!(!enter && s == EscState::Esc1(ms(t0, 301)));
    let (_, enter) = resolve_esc_esc(s, &esc(), none, false, ms(t0, 400));
    assert!(enter, "and that new first Esc pairs with the next one");
}

#[test]
fn keys_t04_other_key_or_repeat_or_modified_esc_resets() {
    let t0 = Instant::now();
    let none = ModifiersState::empty();
    let (first, _) = resolve_esc_esc(EscState::Idle, &esc(), none, false, t0);
    let (s, enter) = resolve_esc_esc(first, &chr("a"), none, false, ms(t0, 10));
    assert!(!enter && s == EscState::Idle);
    let (_, enter) = resolve_esc_esc(s, &esc(), none, false, ms(t0, 20));
    assert!(!enter, "a key between: that Esc is only a first one");
    let (s, enter) = resolve_esc_esc(first, &esc(), none, true, ms(t0, 10));
    assert!(!enter && s == EscState::Idle, "a repeat never counts");
    let (s, enter) = resolve_esc_esc(first, &esc(), ModifiersState::CONTROL, false, ms(t0, 10));
    assert!(!enter && s == EscState::Idle, "Ctrl-Esc is not Esc");
    let (s, enter) = resolve_esc_esc(EscState::Idle, &esc(), ModifiersState::ALT, false, t0);
    assert!(!enter && s == EscState::Idle);
}

fn keys_app(html: &str, script: &str) -> Option<App> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    let doc = parse_html(&format!("<html><body>{html}</body></html>"));
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/page").unwrap());
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    // These tests are about input mode (QKEYS5: a fresh App starts in NORMAL).
    app.key_mode = KeyMode::Input;
    Some(app)
}

fn log(app: &App) -> String {
    app.js.as_ref().unwrap().eval_value::<String>("t", "window.L.join('|')").unwrap_or_default()
}

/// Press then release `key` at `t`.
fn tap(app: &mut App, key: &Key, t: Instant) {
    app.route_key(key, None, true, false, t);
    app.route_key(key, None, false, false, t);
}

const LOG_SCRIPT: &str = "window.L=[]; for (const k of ['keydown','keyup']) document.addEventListener(k, function(e){ L.push(k[3]+':'+e.key); });";

#[test]
fn qkeys5_fresh_app_starts_in_normal_and_j_scrolls() {
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    // keys_app forces INPUT for the older tests; build the real default here.
    app.key_mode = crate::config::START_MODE;
    assert_eq!(crate::config::START_MODE, KeyMode::Normal, "QKEYS5: the default start mode");
    let t0 = Instant::now();
    tap(&mut app, &chr("j"), t0);
    assert_eq!(app.scroll_offset, crate::config::SCROLL_LINE_PX);
    assert_eq!(log(&app), "", "the page sees nothing in NORMAL");
}

#[test]
fn qkeys5_input_start_mode_still_works() {
    // START_MODE = Input is the old behaviour: keys reach the page, Esc Esc enters NORMAL.
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    app.key_mode = KeyMode::Input;
    let t0 = Instant::now();
    tap(&mut app, &chr("j"), t0);
    assert_eq!(log(&app), "d:j|u:j");
    tap(&mut app, &esc(), ms(t0, 10));
    tap(&mut app, &esc(), ms(t0, 60));
    assert_eq!(app.key_mode, KeyMode::Normal);
}

#[test]
fn keys_modes_esc_esc_enters_page_sees_first_esc_only() {
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    let t0 = Instant::now();
    assert_eq!(app.key_mode, KeyMode::Input, "set by keys_app");
    tap(&mut app, &esc(), t0);
    assert_eq!(app.key_mode, KeyMode::Input);
    tap(&mut app, &esc(), ms(t0, 100));
    assert_eq!(app.key_mode, KeyMode::Normal);
    // The first Esc reached the page (down and up), the second neither (T2, T34).
    assert_eq!(log(&app), "d:Escape|u:Escape");
}

#[test]
fn keys_t05_slow_or_interrupted_esc_esc_does_not_enter() {
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    let t0 = Instant::now();
    tap(&mut app, &esc(), t0);
    tap(&mut app, &esc(), ms(t0, 500));
    assert_eq!(app.key_mode, KeyMode::Input);
    tap(&mut app, &chr("a"), ms(t0, 510));
    tap(&mut app, &esc(), ms(t0, 520));
    assert_eq!(app.key_mode, KeyMode::Input, "another key between");
    app.route_key(&esc(), None, true, true, ms(t0, 530));
    assert_eq!(app.key_mode, KeyMode::Input, "a repeat");
}

#[test]
fn keys_p3_page_cancelling_esc_still_lets_esc_esc_through() {
    let script = "window.L=[]; document.addEventListener('keydown', function(e){ L.push(e.key); e.preventDefault(); });";
    let Some(mut app) = keys_app("<p>x</p>", script) else { return };
    let t0 = Instant::now();
    tap(&mut app, &esc(), t0);
    tap(&mut app, &esc(), ms(t0, 50));
    assert_eq!(app.key_mode, KeyMode::Normal);
    assert_eq!(log(&app), "Escape");
}

#[test]
fn keys_t21_t13_normal_mode_page_sees_nothing_and_j_scrolls() {
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    app.key_mode = KeyMode::Normal;
    let t0 = Instant::now();
    tap(&mut app, &chr("j"), t0);
    tap(&mut app, &Key::Named(NamedKey::Tab), t0);
    tap(&mut app, &chr("z"), t0);
    assert_eq!(app.scroll_offset, crate::config::SCROLL_LINE_PX);
    assert_eq!(log(&app), "", "no keydown/keyup in normal mode (P2)");
}

#[test]
fn keys_t05_input_mode_page_gets_j_and_l_and_scratchy_does_nothing() {
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    let t0 = Instant::now();
    tap(&mut app, &chr("j"), t0);
    tap(&mut app, &chr("l"), t0);
    tap(&mut app, &chr("g"), t0);
    tap(&mut app, &chr(":"), t0);
    assert_eq!(app.scroll_offset, 0.0);
    assert_eq!(app.history_index, 0);
    assert_eq!(app.vim_mode, VimMode::Normal, "`:` did not open the command line");
    assert!(!app.pending_g);
    assert_eq!(log(&app), "d:j|u:j|d:l|u:l|d:g|u:g|d::|u::");
}

#[test]
fn keys_t06_i_leaves_normal_mode_and_its_release_stays_undelivered() {
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    app.key_mode = KeyMode::Normal;
    let t0 = Instant::now();
    tap(&mut app, &chr("i"), t0);
    assert_eq!(app.key_mode, KeyMode::Input);
    assert_eq!(log(&app), "", "T34: no keyup without its keydown");
    tap(&mut app, &chr("j"), t0);
    assert_eq!(log(&app), "d:j|u:j");
}

#[test]
fn keys_t07_esc_in_normal_mode_does_not_leave_it() {
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    app.key_mode = KeyMode::Normal;
    let t0 = Instant::now();
    tap(&mut app, &esc(), t0);
    tap(&mut app, &esc(), ms(t0, 10));
    assert_eq!(app.key_mode, KeyMode::Normal);
    assert_eq!(log(&app), "");
}

#[test]
fn keys_t10_t11_gg_and_t12_g_then_i() {
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    app.key_mode = KeyMode::Normal;
    app.scroll_offset = 100.0;
    let t0 = Instant::now();
    tap(&mut app, &chr("g"), t0);
    tap(&mut app, &chr("g"), t0);
    assert_eq!(app.scroll_offset, 0.0);
    tap(&mut app, &chr("g"), t0);
    tap(&mut app, &chr("i"), t0);
    assert_eq!(app.key_mode, KeyMode::Input, "T12: the key after g is handled as in normal mode");
}

#[test]
fn keys_t09_colon_opens_the_command_line_in_normal_mode() {
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    app.key_mode = KeyMode::Normal;
    tap(&mut app, &chr(":"), Instant::now());
    assert_eq!(app.vim_mode, VimMode::Command);
    assert_eq!(app.key_mode, KeyMode::Normal);
    assert_eq!(log(&app), "");
}

#[test]
fn keys_t31_navigation_keeps_the_mode() {
    let Some(mut app) = keys_app("<p>x</p>", LOG_SCRIPT) else { return };
    app.key_mode = KeyMode::Normal;
    app.start_navigation("https://e.org/other");
    assert_eq!(app.key_mode, KeyMode::Normal);
    app.key_mode = KeyMode::Input;
    app.start_navigation("https://e.org/other2");
    assert_eq!(app.key_mode, KeyMode::Input);
}

#[test]
fn keys_t29_click_into_text_control_or_select_switches_to_input() {
    let html = "<html><body style='margin:0'><input id=i style='display:block;margin:0;width:200px;height:30px'>\
                <select style='display:block;margin:0;width:200px;height:30px'><option>a</option></select>\
                <p style='margin:0;height:40px'>plain</p></body></html>";
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let Some(event_loop) = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok() else { return };
    let mut app = App::new_static(parse_html(html), event_loop.create_proxy(), None);
    let doc = app.document.clone();
    let tree = build_layout_tree(&doc, 400.0);
    let mut cache = ShapeCache::new();
    let mut next_id = 0usize;
    let mut incomplete = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    app.cached = Some(Rc::new(measure(
        &tree, &mut new_font_system(), 0.0, 0.0, 400.0, None, &mut next_id, &mut cache, deadline, &mut incomplete,
        &mut Vec::new(), &Floats::default(), None,
    )));
    app.layout_dirty = false;
    app.key_mode = KeyMode::Normal;
    app.press_switches_to_input((10.0, 70.0));
    assert_eq!(app.key_mode, KeyMode::Normal, "a click on plain text keeps normal mode");
    app.vim_mode = VimMode::Command;
    app.press_switches_to_input((10.0, 10.0));
    assert_eq!(app.key_mode, KeyMode::Input, "the text input");
    assert_eq!(app.vim_mode, VimMode::Normal, "an open `:` line is cancelled");
    app.key_mode = KeyMode::Normal;
    app.press_switches_to_input((10.0, 45.0));
    assert_eq!(app.key_mode, KeyMode::Input, "the select");
}
