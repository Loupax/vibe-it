// QCOMP1: the ghost suggestion in the `:` line (KEYS.md T45-T53).
#[allow(unused_imports)]
use super::*;

fn ghost(text: &str) -> Option<Ghost> {
    compute_ghost(text, text.len())
}

fn app_in_command_line() -> Option<App> {
    let event_loop = test_event_loop().ok()?;
    let doc = parse_html("<html><body><p>x</p></body></html>");
    let mut app = App::new_static(doc, event_loop.create_proxy(), None);
    app.key_mode = KeyMode::Normal;
    app.perform_action(Action::EnterCommandMode);
    Some(app)
}

fn type_line(app: &mut App, s: &str) {
    for c in s.chars() {
        let key = if c == ' ' { Key::Named(NamedKey::Space) } else { Key::Character(c.to_string().as_str().into()) };
        app.command_line_key(&key);
    }
}

fn named(app: &mut App, k: NamedKey) {
    app.command_line_key(&Key::Named(k));
}

fn ghost_rest(app: &App) -> Option<String> {
    app.ghost.as_ref().map(|g| g.rest(&app.command_line.text))
}

#[test]
fn qcomp1_se_ghosts_t_and_tab_gives_set_space() {
    let g = ghost("se").expect("ghost");
    assert_eq!(g.rest("se"), "t");
    let Some(mut app) = app_in_command_line() else { return };
    type_line(&mut app, "se");
    assert_eq!(ghost_rest(&app).as_deref(), Some("t"));
    named(&mut app, NamedKey::Tab);
    assert_eq!(app.command_line.text, "set ");
    assert_eq!(app.command_line.cursor, 4);
}

#[test]
fn qcomp1_single_match_without_space_after_is_selected_with_no_space() {
    // A source whose selection takes no trailing space (a URL, QCOMP4): `space_after` false.
    let mut g = Ghost { word_start: 3, candidates: vec!["https://a.test/".into()], index: 0, space_after: false };
    assert_eq!(g.rest("go https://a"), ".test/");
    assert_eq!(g.tab("go https://a").as_deref(), Some(".test/"));
    // A `:set` argument gets a space.
    let mut s = ghost("set mo").expect("ghost");
    assert_eq!(s.tab("set mo").as_deref(), Some("tion "));
}

#[test]
fn qcomp1_tab_cycles_and_wraps_with_two_matches_shift_tab_goes_back() {
    // `h` matches help, historyBack, historyFront.
    let mut g = ghost("h").expect("ghost");
    assert_eq!(g.candidates, vec!["help", "historyBack", "historyFront"]);
    assert_eq!(g.tab("h"), None);
    assert_eq!(g.rest("h"), "istoryBack");
    g.tab("h");
    g.tab("h");
    assert_eq!(g.rest("h"), "elp", "wrapped to the first");
    g.prev();
    assert_eq!(g.rest("h"), "istoryFront", "Shift-Tab wraps backwards");
    g.prev();
    assert_eq!(g.rest("h"), "istoryBack");
}

#[test]
fn qcomp1_tab_and_shift_tab_in_the_app_keep_the_typed_text() {
    let Some(mut app) = app_in_command_line() else { return };
    type_line(&mut app, "h");
    named(&mut app, NamedKey::Tab);
    assert_eq!(app.command_line.text, "h");
    assert_eq!(ghost_rest(&app).as_deref(), Some("istoryBack"));
    app.modifiers = ModifiersState::SHIFT;
    named(&mut app, NamedKey::Tab);
    assert_eq!(ghost_rest(&app).as_deref(), Some("elp"));
}

#[test]
fn qcomp1_enter_runs_typed_plus_ghost() {
    let Some(mut app) = app_in_command_line() else { return };
    type_line(&mut app, "hel");
    assert_eq!(ghost_rest(&app).as_deref(), Some("p"));
    named(&mut app, NamedKey::Enter);
    assert_eq!(app.vim_mode, VimMode::Normal);
    assert!(app.ghost.is_none());
    assert!(!app.status_message.contains("unknown command"), "ran :help, not :hel: {}", app.status_message);
}

#[test]
fn qcomp1_delete_hides_the_ghost_so_enter_runs_the_typed_text() {
    let Some(mut app) = app_in_command_line() else { return };
    type_line(&mut app, "hel");
    named(&mut app, NamedKey::Delete);
    assert!(app.ghost.is_none());
    named(&mut app, NamedKey::Tab);
    assert_eq!(app.command_line.text, "hel", "Tab does nothing without a ghost");
    named(&mut app, NamedKey::Enter);
    assert!(app.status_message.contains("hel"), "ran the typed `hel`: {}", app.status_message);
    // The next edit brings the ghost back.
    app.perform_action(Action::EnterCommandMode);
    type_line(&mut app, "hel");
    named(&mut app, NamedKey::Delete);
    type_line(&mut app, "p");
    assert_eq!(ghost_rest(&app).as_deref(), Some(""));
}

#[test]
fn qcomp1_right_and_end_accept_the_ghost() {
    let Some(mut app) = app_in_command_line() else { return };
    type_line(&mut app, "hel");
    named(&mut app, NamedKey::ArrowRight);
    assert_eq!(app.command_line.text, "help");
    assert_eq!(app.command_line.cursor, 4);
    app.perform_action(Action::EnterCommandMode);
    type_line(&mut app, "ref");
    named(&mut app, NamedKey::End);
    assert_eq!(app.command_line.text, "refresh");
}

#[test]
fn qcomp1_cursor_not_at_the_end_means_no_ghost() {
    assert!(compute_ghost("se", 1).is_none());
    let Some(mut app) = app_in_command_line() else { return };
    type_line(&mut app, "se");
    assert!(app.ghost.is_some());
    named(&mut app, NamedKey::ArrowLeft);
    assert!(app.ghost.is_none());
}

#[test]
fn qcomp1_a_command_without_a_row_gets_no_ghost_after_its_name() {
    assert!(ghost("go ht").is_none(), "`:go` has no source until QCOMP4");
    assert!(ghost("refresh x").is_none());
    assert!(ghost("set m").is_some());
    assert!(ghost("").is_none());
    assert!(ghost("set ").is_none(), "empty word: no ghost");
    assert!(ghost("o").is_none(), "prefix, not substring");
}

#[test]
fn qcomp1_search_line_has_no_ghost() {
    let Some(mut app) = app_in_command_line() else { return };
    app.perform_action(Action::EnterSearchMode);
    app.search_line_key(&Key::Character("s".into()));
    app.search_line_key(&Key::Character("e".into()));
    assert!(app.ghost.is_none());
}

#[test]
fn qcomp1_token_start_before_finds_the_last_whitespace_boundary() {
    assert_eq!(token_start_before("go", 2), 0);
    assert_eq!(token_start_before("set mo", 6), 4);
    assert_eq!(token_start_before("set ", 4), 4);
    assert_eq!(token_start_before("", 0), 0);
}
