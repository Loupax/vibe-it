// QTYPE1: typing into a second text field (Tab or click from the first) must insert every character.
#[allow(unused_imports)]
use super::*;
use std::time::Instant;

fn app_with(html: &str, script: &str) -> Option<App> {
    let el = test_event_loop().ok()?;
    let doc = parse_html(&format!("<html><body>{html}</body></html>"));
    let mut app = App::new_static(doc.clone(), el.create_proxy(), None);
    app.base_url = Some(Url::parse("https://e.org/page").unwrap());
    app.js = run_page_scripts(&doc, &[src(script)], app.base_url.as_ref());
    app.key_mode = KeyMode::Input;
    Some(app)
}

fn tap(app: &mut App, key: &Key) {
    let t = Instant::now();
    app.route_key(key, None, true, false, t);
    app.route_key(key, None, false, false, t);
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        tap(app, &Key::Character(c.to_string().into()));
    }
}

fn val(app: &App, id: &str) -> String {
    let h = by_id(&app.document, id);
    text_value(&h)
}

#[test]
fn qtype1_tab_to_second_field_types_every_char() {
    let Some(mut app) = app_with("<input id=a><input id=b>", "") else { return };
    app.tab_focus(false);
    type_str(&mut app, "abc");
    assert_eq!(val(&app, "a"), "abc");
    tap(&mut app, &Key::Named(NamedKey::Tab));
    type_str(&mut app, "hello");
    assert_eq!(val(&app, "b"), "hello");
    assert_eq!(val(&app, "a"), "abc");
}

/// The paint's damage comes from the layout engine, which sees nothing when only a control's live state
/// changes: every edit must queue the control's box (the App turns the queue into damage rects).
#[test]
fn qtype1_every_edit_queues_the_controls_box_for_repaint() {
    let Some(mut app) = app_with("<input id=a><input id=b><textarea id=c></textarea><input id=k type=checkbox>", "") else { return };
    let (a, b, c, k) = (by_id(&app.document, "a"), by_id(&app.document, "b"), by_id(&app.document, "c"), by_id(&app.document, "k"));
    crate::forms::take_ctl_dirty();
    app.tab_focus(false);
    tap(&mut app, &Key::Character("x".into()));
    assert!(crate::forms::take_ctl_dirty().contains(&node_id(&a)), "first field");
    // Tab to the second field, then each character of the word, one repaint request per key.
    tap(&mut app, &Key::Named(NamedKey::Tab));
    for ch in "hello".chars() {
        crate::forms::take_ctl_dirty();
        tap(&mut app, &Key::Character(ch.to_string().into()));
        assert!(crate::forms::take_ctl_dirty().contains(&node_id(&b)), "second field, char {ch}");
    }
    // Caret keys and the textarea too.
    crate::forms::take_ctl_dirty();
    tap(&mut app, &Key::Named(NamedKey::ArrowLeft));
    assert!(crate::forms::take_ctl_dirty().contains(&node_id(&b)), "caret move");
    tap(&mut app, &Key::Named(NamedKey::Tab));
    crate::forms::take_ctl_dirty();
    tap(&mut app, &Key::Character("q".into()));
    assert!(crate::forms::take_ctl_dirty().contains(&node_id(&c)), "textarea");
    // Script writes and checkbox toggles.
    crate::forms::take_ctl_dirty();
    set_text_value(&a, "script");
    assert!(crate::forms::take_ctl_dirty().contains(&node_id(&a)), "el.value = ...");
    crate::forms::set_checked(&k, true);
    assert!(crate::forms::take_ctl_dirty().contains(&node_id(&k)), "checkbox");
    // Pure reads do not.
    let _ = text_value(&a);
    assert!(crate::forms::take_ctl_dirty().is_empty(), "reading the value is not a change");
}
