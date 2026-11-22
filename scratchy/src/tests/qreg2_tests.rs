// QREG2: `:let @+ = <JS expression>`.
#[allow(unused_imports)]
use super::*;

fn app_with(title: &str) -> Option<App> {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let event_loop = EventLoop::<AppEvent>::with_user_event().with_any_thread(true).build().ok()?;
    let doc = parse_html(&format!("<html><head><title>{title}</title></head><body><p>x</p></body></html>"));
    let mut app = App::new_static(doc.clone(), event_loop.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/page").unwrap());
    app.js = run_page_scripts(&doc, &[], app.base_url.as_ref());
    crate::forms_text::clipboard_set("untouched");
    Some(app)
}

#[test]
fn qreg2_let_evaluates_title_and_copies_it() {
    let Some(mut app) = app_with("Hello  World") else { return };
    app.run_command_line("let @+ = document.title");
    assert_eq!(app.status_message, "Copied to clipboard");
    assert_eq!(crate::forms_text::clipboard_get().as_deref(), Some("Hello World"));
    // the raw line: spacing inside string literals survives, non-strings go through String()
    app.run_command_line("let @+ = 'a   b' + 1");
    assert_eq!(crate::forms_text::clipboard_get().as_deref(), Some("a   b1"));
    app.run_command_line("let @+ = [1,2].length");
    assert_eq!(crate::forms_text::clipboard_get().as_deref(), Some("2"));
    // long results are copied whole
    app.run_command_line("let @+ = 'x'.repeat(5000)");
    assert_eq!(crate::forms_text::clipboard_get().map(|s| s.len()), Some(5000));
    assert_eq!(app.status_message, "Copied to clipboard");
}

#[test]
fn qreg2_throwing_expression_shows_message_and_copies_nothing() {
    let Some(mut app) = app_with("t") else { return };
    app.run_command_line("let @+ = nope.x");
    assert!(app.status_message.starts_with("scratchy: ") && app.status_message.contains("nope"), "{}", app.status_message);
    app.run_command_line("let @+ = (");
    assert!(app.status_message.starts_with("scratchy: "), "{}", app.status_message);
    assert_eq!(crate::forms_text::clipboard_get().as_deref(), Some("untouched"));
}

#[test]
fn qreg2_let_expression_needs_js() {
    let Some(mut app) = app_with("t") else { return };
    app.js = None;
    app.run_command_line("let @+ = document.title");
    assert_eq!(app.status_message, "scratchy: :let needs JS (:set js)");
    assert_eq!(crate::forms_text::clipboard_get().as_deref(), Some("untouched"));
}
