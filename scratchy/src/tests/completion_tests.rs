// QCOMP1: the ghost suggestion in the `:` line (KEYS.md T45-T53).
#[allow(unused_imports)]
use super::*;

fn ghost(text: &str) -> Option<Ghost> {
    compute_ghost(text, text.len(), &CompleteCache::new(None))
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
    assert!(compute_ghost("se", 1, &CompleteCache::new(None)).is_none());
    let Some(mut app) = app_in_command_line() else { return };
    type_line(&mut app, "se");
    assert!(app.ghost.is_some());
    named(&mut app, NamedKey::ArrowLeft);
    assert!(app.ghost.is_none());
}

#[test]
fn qcomp1_a_command_without_a_row_gets_no_ghost_after_its_name() {
    assert!(ghost("go ht").is_none(), "`:go` with no history file has no ghost");
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

// ---------- QCOMP4: the `:go` history source (STASH.md "Ranking") ----------

fn history_dir(tag: &str, lines: &[(u64, &str)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("scratchy-qcomp4-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut text = String::new();
    for (ms, url) in lines {
        text.push_str(&format!("{ms}\t{url}\ttitle\n"));
    }
    std::fs::write(dir.join("history"), text).unwrap();
    dir
}

fn go_candidates(dir: &std::path::Path, word: &str) -> Vec<String> {
    let cache = CompleteCache::new(Some(dir.to_path_buf()));
    compute_ghost(&format!("go {word}"), 3 + word.len(), &cache).map(|g| g.candidates).unwrap_or_default()
}

#[test]
fn qcomp4_a_site_with_many_single_visit_pages_outranks_a_page_visited_twice() {
    let mut lines = vec![(1, "https://b.test/x"), (2, "https://b.test/x")];
    for i in 0..5 {
        lines.push((10 + i, ["https://a.test/1", "https://a.test/2", "https://a.test/3", "https://a.test/4", "https://a.test/5"][i as usize]));
    }
    let dir = history_dir("sites", &lines);
    // Roots first by site visits (a: 5, b: 2), then pages by own visits.
    assert_eq!(go_candidates(&dir, "https://"), vec![
        "https://a.test/", "https://b.test/", "https://b.test/x", "https://a.test/5", "https://a.test/4", "https://a.test/3", "https://a.test/2", "https://a.test/1"
    ]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn qcomp4_a_root_url_appears_once_and_ties_go_to_the_newest() {
    let dir = history_dir("ties", &[(1, "https://a.test/"), (2, "https://a.test/"), (3, "https://b.test/"), (4, "https://b.test/"), (5, "https://c.test/p"), (6, "https://d.test/p")]);
    let c = go_candidates(&dir, "https://");
    assert_eq!(c.iter().filter(|u| *u == "https://a.test/").count(), 1);
    // a and b both have 2 visits: b is newer. c and d tie at 1: d newer. Sites before pages.
    assert_eq!(c[..2], ["https://b.test/", "https://a.test/"]);
    let pages: Vec<_> = c.iter().filter(|u| u.ends_with("/p")).collect();
    assert_eq!(pages, ["https://d.test/p", "https://c.test/p"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn qcomp4_bad_lines_are_skipped() {
    let dir = std::env::temp_dir().join(format!("scratchy-qcomp4-{}-bad", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("history"), "garbage\nx\thttps://bad-time.test/\t\n5\tnot a url\tt\n6\tfile:///etc/passwd\tt\n7\tftp://f.test/\tt\n8\thttps://ok.test/\tt\n\n").unwrap();
    assert_eq!(go_candidates(&dir, "https://"), vec!["https://ok.test/"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn qcomp4_you_finds_youtube_and_https_google_ghosts_dot_com() {
    let dir = history_dir("match", &[(1, "https://www.youtube.com/watch?v=1"), (2, "https://www.youtube.com/watch?v=2"), (3, "https://www.youtube.com/"), (4, "https://www.google.com/")]);
    let cache = CompleteCache::new(Some(dir.clone()));
    let g = compute_ghost("go you", 6, &cache).expect("ghost");
    assert_eq!(g.candidates[0], "youtube.com/", "the site root first, www dropped like the word");
    assert_eq!(g.rest("go you"), "tube.com/");
    let g = compute_ghost("go www.you", 10, &cache).expect("ghost");
    assert_eq!(g.candidates[0], "www.youtube.com/");
    let g = compute_ghost("go https://www.you", 18, &cache).expect("ghost");
    assert_eq!(g.candidates[0], "https://www.youtube.com/", "a raw prefix keeps the URL as visited");
    let g = compute_ghost("go https://google", 17, &cache).expect("ghost");
    assert_eq!(g.rest("go https://google"), ".com/");
    assert_eq!(g.accepted("go https://google"), "go https://google.com/");
    assert!(!g.space_after, "a URL gets no trailing space");
    assert!(compute_ghost("go https://nomatch", 18, &cache).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn qcomp4_enter_runs_the_ghost_and_delete_then_enter_runs_what_was_typed() {
    let dir = history_dir("enter", &[(1, "https://www.google.com/")]);
    let Some(mut app) = app_in_command_line() else { return };
    app.complete_cache = CompleteCache::new(Some(dir.clone()));
    type_line(&mut app, "go https://google");
    assert_eq!(ghost_rest(&app).as_deref(), Some(".com/"));
    named(&mut app, NamedKey::Enter);
    assert_eq!(app.history.last().map(|u| u.as_str()), Some("https://google.com/"));
    app.perform_action(Action::EnterCommandMode);
    app.complete_cache = CompleteCache::new(Some(dir.clone()));
    type_line(&mut app, "go https://google");
    named(&mut app, NamedKey::Delete);
    named(&mut app, NamedKey::Enter);
    assert_eq!(app.history.last().map(|u| u.as_str()), Some("https://google/"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn qcomp4_history_is_read_once_per_line_not_per_key() {
    let dir = history_dir("once", &[(1, "https://www.google.com/"), (2, "https://www.example.com/")]);
    let Some(mut app) = app_in_command_line() else { return };
    app.complete_cache = CompleteCache::new(Some(dir.clone()));
    type_line(&mut app, "go https://goo");
    named(&mut app, NamedKey::Tab);
    type_line(&mut app, "g");
    assert_eq!(app.complete_cache.loads.get(), 1, "one read for 14+ keys");
    assert_eq!(app.complete_cache.history().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}
