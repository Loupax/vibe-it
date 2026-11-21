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

// ---- QCOMP2: external sources (Source::Cmd) ----

use crate::config::Source;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

fn ext_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("scratchy-qcomp2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A cache whose external results arrive on the returned receiver.
fn ext_cache() -> (CompleteCache, mpsc::Receiver<ExtResult>) {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let sink: ExtSink = std::sync::Arc::new(move |r| {
        let _ = tx.lock().unwrap().send(r);
    });
    (CompleteCache::new(None).with_external(sink, "https://page.test/".into()), rx)
}

/// A table with one `x` row running `sh <script>` (leaked: rows are 'static).
fn ext_table(script: &std::path::Path) -> &'static [(&'static str, Source)] {
    let path: &'static str = Box::leak(script.display().to_string().into_boxed_str());
    let argv: &'static [&'static str] = Box::leak(Box::new(["sh", path]));
    Box::leak(Box::new([("x", Source::Cmd(argv))]))
}

fn script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

fn ext_ghost(t: &[(&str, Source)], text: &str, cache: &CompleteCache) -> Option<Ghost> {
    compute_ghost_in(t, text, text.len(), cache)
}

#[test]
fn qcomp2_lines_become_candidates_in_order_with_the_context_in_the_environment() {
    let dir = ext_dir("order");
    let s = script(&dir, "s.sh", "echo alpha\necho alpine\necho beta\n");
    let t = ext_table(&s);
    let (cache, rx) = ext_cache();
    assert!(ext_ghost(t, "x al", &cache).is_none(), "nothing until the source answered");
    let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(cache.accept(&r));
    assert_eq!(ext_ghost(t, "x al", &cache).expect("ghost").candidates, vec!["alpha", "alpine"]);
    // Filtered per keystroke from the same output.
    assert_eq!(ext_ghost(t, "x alp", &cache).unwrap().candidates, vec!["alpha", "alpine"]);
    assert_eq!(ext_ghost(t, "x alpi", &cache).unwrap().candidates, vec!["alpine"]);
    // The context.
    let (cache2, rx2) = ext_cache();
    let s2 = script(&dir, "e.sh", "echo \"$SCRATCHY_ARG|$SCRATCHY_WORD|$SCRATCHY_URL|$SCRATCHY_LINE\"\n");
    let t2 = ext_table(&s2);
    let _ = ext_ghost(t2, "x 1", &cache2);
    let r = rx2.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(r.outcome.unwrap(), vec!["1|1|https://page.test/|x 1"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn qcomp2_non_zero_exit_and_empty_output_give_no_ghost() {
    let dir = ext_dir("fail");
    for (name, body) in [("bad.sh", "echo alpha\nexit 3\n"), ("empty.sh", "true\n")] {
        let s = script(&dir, name, body);
        let t = ext_table(&s);
        let (cache, rx) = ext_cache();
        let _ = ext_ghost(t, "x a", &cache);
        let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(r.outcome, Ok(Vec::new()), "{name}");
        assert!(cache.accept(&r));
        assert!(ext_ghost(t, "x a", &cache).is_none(), "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn qcomp2_a_missing_program_gives_a_status_bar_message() {
    let argv: &'static [&'static str] = &["/nonexistent/scratchy-qcomp2-source"];
    let t: &'static [(&'static str, Source)] = Box::leak(Box::new([("x", Source::Cmd(argv))]));
    let (cache, rx) = ext_cache();
    let _ = ext_ghost(t, "x a", &cache);
    let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let msg = r.outcome.clone().unwrap_err();
    assert!(msg.contains("not found"), "{msg}");
    // The app shows it and keeps no ghost.
    let Some(mut app) = app_in_command_line() else { return };
    let r = ExtResult { line_id: app.complete_cache.id, cmd: "x".into(), arg: 1, outcome: Err(msg.clone()) };
    app.handle_complete_result(r);
    assert_eq!(app.status_message, msg);
    assert!(app.ghost.is_none());
}

#[test]
fn qcomp2_shell_syntax_in_the_line_is_plain_text() {
    let dir = ext_dir("plain");
    let pwned = dir.join("PWNED");
    let s = script(&dir, "s.sh", "printf '%sz\\n' \"$SCRATCHY_WORD\"\n");
    let t = ext_table(&s);
    let (cache, rx) = ext_cache();
    let word = format!("a;$(touch${{IFS}}{})`id`", pwned.display());
    let _ = ext_ghost(t, &format!("x {word}"), &cache);
    let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(cache.accept(&r));
    let g = ext_ghost(t, &format!("x {word}"), &cache).expect("ghost");
    assert_eq!(g.candidates, vec![format!("{word}z")]);
    assert!(!pwned.exists(), "nothing was run through a shell");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn qcomp2_the_source_runs_once_per_line_and_position_not_per_key() {
    let dir = ext_dir("once");
    let log = dir.join("runs");
    let s = script(&dir, "s.sh", &format!("echo run >> {}\necho abc\necho abd\n", log.display()));
    let t = ext_table(&s);
    let (cache, rx) = ext_cache();
    for text in ["x a", "x a", "x a"] {
        let _ = ext_ghost(t, text, &cache);
    }
    let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(cache.accept(&r));
    for text in ["x a", "x ab", "x abc", "x abd"] {
        assert!(ext_ghost(t, text, &cache).is_some(), "{text}");
    }
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "no second run");
    assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 1);
    // Another argument position is its own run.
    let _ = ext_ghost(t, "x abc a", &cache);
    let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(r.arg, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn qcomp2_a_stale_result_is_ignored() {
    let dir = ext_dir("stale");
    let s = script(&dir, "s.sh", "echo abc\n");
    let t = ext_table(&s);
    let (old, rx) = ext_cache();
    let _ = ext_ghost(t, "x a", &old);
    let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    // The line was closed and a new one opened before it arrived.
    let (new, _rx2) = ext_cache();
    assert!(!new.accept(&r));
    assert!(ext_ghost(t, "x a", &new).is_none(), "the dropped result gave nothing");
    // In the app, a result for an earlier line changes nothing.
    let Some(mut app) = app_in_command_line() else { return };
    let id = app.complete_cache.id;
    app.handle_complete_result(ExtResult { line_id: id.wrapping_sub(1), cmd: "go".into(), arg: 1, outcome: Err("late".into()) });
    assert!(app.status_message.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn qcomp2_a_source_that_runs_too_long_is_killed() {
    let t0 = std::time::Instant::now();
    let r = run_source(&["sleep", "30"], &[], Duration::from_millis(150));
    assert_eq!(r, Ok(Vec::new()));
    assert!(t0.elapsed() < Duration::from_secs(5));
}
