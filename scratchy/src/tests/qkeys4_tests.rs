// QKEYS4: the UA default scroll keys in input mode (P6).
#[allow(unused_imports)]
use super::*;
use std::time::Instant;

const LOG_SCRIPT: &str = "window.L=[]; for (const k of ['keydown','keyup']) document.addEventListener(k, function(e){ L.push(k[3]+':'+e.key); });";

fn named(n: NamedKey) -> Key {
    Key::Named(n)
}

fn chr(s: &str) -> Key {
    Key::Character(s.into())
}

fn app_with(html: &str, script: &str) -> Option<App> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    let doc = parse_html(&format!("<html><body>{html}</body></html>"));
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/page").unwrap());
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    Some(app)
}

fn log(app: &App) -> String {
    app.js.as_ref().unwrap().eval_value::<String>("t", "window.L.join('|')").unwrap_or_default()
}

fn tap(app: &mut App, key: &Key) {
    let t = Instant::now();
    app.route_key(key, None, true, false, t);
    app.route_key(key, None, false, false, t);
}

#[test]
fn qkeys4_each_scroll_key_scrolls_in_input_mode() {
    let Some(mut app) = app_with("<p>x</p>", LOG_SCRIPT) else { return };
    let line = crate::config::SCROLL_LINE_PX;
    tap(&mut app, &named(NamedKey::ArrowDown));
    assert_eq!(app.scroll_offset, line);
    tap(&mut app, &named(NamedKey::ArrowUp));
    assert_eq!(app.scroll_offset, 0.0);
    let page = page_step(app.page_size().1 as f32);
    tap(&mut app, &named(NamedKey::PageDown));
    assert_eq!(app.scroll_offset, page);
    tap(&mut app, &named(NamedKey::PageUp));
    assert_eq!(app.scroll_offset, 0.0);
    tap(&mut app, &named(NamedKey::Space));
    assert_eq!(app.scroll_offset, page);
    app.modifiers = ModifiersState::SHIFT;
    tap(&mut app, &named(NamedKey::Space));
    app.modifiers = ModifiersState::empty();
    assert_eq!(app.scroll_offset, 0.0, "Shift-Space pages up");
    tap(&mut app, &named(NamedKey::End));
    assert_eq!(app.scroll_offset, f32::MAX);
    tap(&mut app, &named(NamedKey::Home));
    assert_eq!(app.scroll_offset, 0.0);
    assert_eq!(app.key_mode, KeyMode::Input);
    // `j`/`k`/`G` stay the page's.
    tap(&mut app, &chr("j"));
    tap(&mut app, &chr("G"));
    assert_eq!(app.scroll_offset, 0.0);
}

#[test]
fn qkeys4_page_preventdefault_stops_the_scroll() {
    let script = "window.L=[]; document.addEventListener('keydown', function(e){ L.push(e.key); e.preventDefault(); });";
    let Some(mut app) = app_with("<p>x</p>", script) else { return };
    tap(&mut app, &named(NamedKey::ArrowDown));
    tap(&mut app, &named(NamedKey::Space));
    tap(&mut app, &named(NamedKey::PageDown));
    assert_eq!(app.scroll_offset, 0.0);
    assert_eq!(log(&app), "ArrowDown| |PageDown");
}

#[test]
fn qkeys4_focused_text_control_keeps_space_and_arrows() {
    let Some(mut app) = app_with("<input id=i value=ab>", LOG_SCRIPT) else { return };
    let i = q30c_by(&app.document.clone(), "i");
    app.set_focus_element(Some(&i), false);
    tap(&mut app, &named(NamedKey::Space));
    tap(&mut app, &named(NamedKey::ArrowDown));
    tap(&mut app, &named(NamedKey::ArrowUp));
    tap(&mut app, &named(NamedKey::Home));
    tap(&mut app, &named(NamedKey::End));
    assert_eq!(app.scroll_offset, 0.0, "the control owns the keys");
}

#[test]
fn qkeys4_focused_button_keeps_space_and_normal_mode_is_unchanged() {
    let Some(mut app) = app_with("<button id=b>go</button>", LOG_SCRIPT) else { return };
    let b = q30c_by(&app.document.clone(), "b");
    app.set_focus_element(Some(&b), false);
    tap(&mut app, &named(NamedKey::Space));
    assert_eq!(app.scroll_offset, 0.0, "Space presses the focused button");
    app.key_mode = KeyMode::Normal;
    app.set_focus_element(None, false);
    let before = log(&app);
    tap(&mut app, &named(NamedKey::Space));
    let page = page_step(app.page_size().1 as f32);
    assert_eq!(app.scroll_offset, page);
    tap(&mut app, &named(NamedKey::ArrowDown));
    assert_eq!(app.scroll_offset, page + crate::config::SCROLL_LINE_PX);
    assert_eq!(log(&app), before, "the page saw no more keys in normal mode");
}
