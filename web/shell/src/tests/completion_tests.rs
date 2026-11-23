//! KEYS.md T45-T53 (the ghost completion) and its sources, ported from scratchy's
//! completion tests (QCOMP1, QCOMP2, QCOMP4).

use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use smithay::input::keyboard::Keysym;

use crate::completion::*;
use crate::config::Source;
use crate::keys::*;
use crate::protocol::Command;

fn ghost(text: &str) -> Option<Ghost> {
    compute_ghost(text, text.len(), &CompleteCache::new(None))
}

fn line_with(dir: Option<std::path::PathBuf>) -> Keys {
    let mut k = Keys::new(dir, "file:///help.html".into(), None);
    tap(&mut k, KeyInput::ch(':'));
    k
}

fn tap(k: &mut Keys, key: KeyInput) {
    let code = key.sym.raw();
    k.key(code, true, &key, Instant::now());
    k.key(code, false, &key, Instant::now());
}

fn named(k: &mut Keys, sym: Keysym) {
    tap(k, KeyInput::sym(sym));
}

fn type_line(k: &mut Keys, s: &str) {
    for c in s.chars() {
        tap(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

fn ghost_rest(k: &Keys) -> Option<String> {
    k.ghost.as_ref().map(|g| g.rest(&k.line.text))
}

fn opened(k: &mut Keys) -> Option<String> {
    std::mem::take(&mut k.effects).into_iter().find_map(|e| match e {
        Effect::Send(Command::Open { url }) => Some(url),
        _ => None,
    })
}

#[test]
fn keys_t45_an_edit_shows_the_first_match_as_the_ghost() {
    let g = ghost("hel").expect("ghost");
    assert_eq!(g.rest("hel"), "p");
    assert_eq!(g.index, 0);
    let mut k = line_with(None);
    type_line(&mut k, "re");
    assert_eq!(ghost_rest(&k).as_deref(), Some("fresh"));
    type_line(&mut k, "fresh h");
    assert_eq!(ghost_rest(&k).as_deref(), Some("ard"), ":refresh's argument completes");
    named(&mut k, Keysym::BackSpace);
    assert_eq!(ghost_rest(&k), None, "empty word: no ghost");
    type_line(&mut k, "h");
    assert_eq!(ghost_rest(&k).as_deref(), Some("ard"), "Backspace then a key: back");
}

#[test]
fn keys_t46_no_match_empty_word_or_cursor_not_at_the_end_means_no_ghost() {
    assert!(ghost("").is_none());
    assert!(ghost("refresh ").is_none(), "empty word");
    assert!(ghost("o").is_none(), "prefix, not substring");
    assert!(ghost("refresh x").is_none());
    assert!(ghost("help x").is_none(), "a command without a COMPLETERS row");
    assert!(ghost("go ht").is_none(), "`:go` with no history file");
    assert!(compute_ghost("hel", 1, &CompleteCache::new(None)).is_none());
    let mut k = line_with(None);
    type_line(&mut k, "hel");
    assert!(k.ghost.is_some());
    named(&mut k, Keysym::Left);
    assert!(k.ghost.is_none(), "a cursor move away from the end");
}

#[test]
fn keys_t47_t48_tab_cycles_and_wraps_shift_tab_goes_back() {
    // `h` matches help, historyBack, historyFront, historySearch (C-PICK).
    let mut g = ghost("h").expect("ghost");
    assert_eq!(g.candidates, vec!["help", "historyBack", "historyFront", "historySearch"]);
    assert_eq!(g.tab("h"), None);
    assert_eq!(g.rest("h"), "istoryBack");
    g.tab("h");
    g.tab("h");
    g.tab("h");
    assert_eq!(g.rest("h"), "elp", "wrapped to the first");
    g.prev();
    assert_eq!(g.rest("h"), "istorySearch", "Shift-Tab wraps backwards");
    // In the key model: the typed text does not change; Shift-Tab arrives as ISO_Left_Tab.
    let mut k = line_with(None);
    type_line(&mut k, "h");
    named(&mut k, Keysym::Tab);
    assert_eq!(k.line.text, "h");
    assert_eq!(ghost_rest(&k).as_deref(), Some("istoryBack"));
    tap(&mut k, KeyInput::sym(Keysym::ISO_Left_Tab).shift());
    assert_eq!(ghost_rest(&k).as_deref(), Some("elp"));
    tap(&mut k, KeyInput::sym(Keysym::Tab).shift());
    assert_eq!(ghost_rest(&k).as_deref(), Some("istorySearch"));
}

#[test]
fn keys_t53_tab_with_one_match_selects_it_a_command_name_gets_a_space_a_url_none() {
    let mut k = line_with(None);
    type_line(&mut k, "ref");
    named(&mut k, Keysym::Tab);
    assert_eq!(k.line.text, "refresh ");
    assert_eq!(k.line.cursor, 8);
    type_line(&mut k, "h");
    named(&mut k, Keysym::Tab);
    assert_eq!(k.line.text, "refresh hard", "an argument not in SPACE_AFTER_ARG: no space");
    let mut g = Ghost {
        word_start: 3,
        candidates: vec!["https://a.test/".into()],
        index: 0,
        space_after: false,
    };
    assert_eq!(g.tab("go https://a").as_deref(), Some(".test/"));
}

#[test]
fn keys_t49_tab_without_a_ghost_does_nothing() {
    let mut k = line_with(None);
    type_line(&mut k, "zz");
    named(&mut k, Keysym::Tab);
    tap(&mut k, KeyInput::sym(Keysym::ISO_Left_Tab).shift());
    assert_eq!(k.line.text, "zz");
    assert!(k.ghost.is_none());
}

#[test]
fn keys_t50_enter_runs_typed_plus_ghost() {
    let mut k = line_with(None);
    type_line(&mut k, "hel");
    assert_eq!(ghost_rest(&k).as_deref(), Some("p"));
    named(&mut k, Keysym::Return);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.ghost.is_none());
    assert_eq!(opened(&mut k).as_deref(), Some("file:///help.html"), "ran :help, not :hel");
}

#[test]
fn keys_t51_right_and_end_accept_the_ghost() {
    let mut k = line_with(None);
    type_line(&mut k, "hel");
    named(&mut k, Keysym::Right);
    assert_eq!(k.line.text, "help");
    assert_eq!(k.line.cursor, 4);
    named(&mut k, Keysym::Escape);
    tap(&mut k, KeyInput::ch(':'));
    type_line(&mut k, "ref");
    named(&mut k, Keysym::End);
    assert_eq!(k.line.text, "refresh");
}

#[test]
fn keys_t52_delete_hides_the_ghost_so_enter_runs_the_typed_text() {
    let mut k = line_with(None);
    type_line(&mut k, "hel");
    named(&mut k, Keysym::Delete);
    assert!(k.ghost.is_none());
    named(&mut k, Keysym::Tab);
    assert_eq!(k.line.text, "hel", "Tab does nothing without a ghost");
    named(&mut k, Keysym::Return);
    assert!(k.message.contains(":hel"), "ran the typed `hel`: {}", k.message);
    // The next edit brings the ghost back.
    tap(&mut k, KeyInput::ch(':'));
    type_line(&mut k, "he");
    named(&mut k, Keysym::Delete);
    type_line(&mut k, "l");
    assert_eq!(ghost_rest(&k).as_deref(), Some("p"));
}

#[test]
fn keys_t45_token_start_before_finds_the_last_whitespace_boundary() {
    assert_eq!(token_start_before("go", 2), 0);
    assert_eq!(token_start_before("go ex", 5), 3);
    assert_eq!(token_start_before("go ", 3), 3);
    assert_eq!(token_start_before("", 0), 0);
}

// ---------- the `:go` history source (STASH.md "Ranking") ----------

fn history_dir(tag: &str, lines: &[(u64, &str)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("web-c3-rank-{}-{tag}", std::process::id()));
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
    compute_ghost(&format!("go {word}"), 3 + word.len(), &cache)
        .map(|g| g.candidates)
        .unwrap_or_default()
}

#[test]
fn ranking_a_site_with_many_single_visit_pages_outranks_a_page_visited_twice() {
    let mut lines = vec![(1, "https://b.test/x"), (2, "https://b.test/x")];
    let pages = [
        "https://a.test/1",
        "https://a.test/2",
        "https://a.test/3",
        "https://a.test/4",
        "https://a.test/5",
    ];
    for (i, p) in pages.iter().enumerate() {
        lines.push((10 + i as u64, p));
    }
    let dir = history_dir("sites", &lines);
    assert_eq!(
        go_candidates(&dir, "https://"),
        vec![
            "https://a.test/",
            "https://b.test/",
            "https://b.test/x",
            "https://a.test/5",
            "https://a.test/4",
            "https://a.test/3",
            "https://a.test/2",
            "https://a.test/1"
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ranking_a_root_url_appears_once_and_ties_go_to_the_newest() {
    let dir = history_dir(
        "ties",
        &[
            (1, "https://a.test/"),
            (2, "https://a.test/"),
            (3, "https://b.test/"),
            (4, "https://b.test/"),
            (5, "https://c.test/p"),
            (6, "https://d.test/p"),
        ],
    );
    let c = go_candidates(&dir, "https://");
    assert_eq!(c.iter().filter(|u| *u == "https://a.test/").count(), 1);
    assert_eq!(c[..2], ["https://b.test/", "https://a.test/"]);
    let pages: Vec<_> = c.iter().filter(|u| u.ends_with("/p")).collect();
    assert_eq!(pages, ["https://d.test/p", "https://c.test/p"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ranking_bad_lines_are_skipped() {
    let dir = history_dir("bad", &[]);
    std::fs::write(
        dir.join("history"),
        "garbage\nx\thttps://bad-time.test/\t\n5\tnot a url\tt\n6\tfile:///etc/passwd\tt\n7\tftp://f.test/\tt\n8\thttps://ok.test/\tt\n\n",
    )
    .unwrap();
    assert_eq!(go_candidates(&dir, "https://"), vec!["https://ok.test/"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keys_t45_go_you_finds_youtube_and_https_google_ghosts_dot_com() {
    let dir = history_dir(
        "match",
        &[
            (1, "https://www.youtube.com/watch?v=1"),
            (2, "https://www.youtube.com/watch?v=2"),
            (3, "https://www.youtube.com/"),
            (4, "https://www.google.com/"),
        ],
    );
    let cache = CompleteCache::new(Some(dir.clone()));
    let g = compute_ghost("go you", 6, &cache).expect("ghost");
    assert_eq!(g.candidates[0], "youtube.com/", "the site root first, www dropped like the word");
    assert_eq!(g.rest("go you"), "tube.com/");
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
fn keys_t50_t52_go_enter_runs_the_ghost_delete_then_enter_runs_what_was_typed() {
    let dir = history_dir("enter", &[(1, "https://www.google.com/")]);
    let mut k = line_with(Some(dir.clone()));
    type_line(&mut k, "go goo");
    assert_eq!(ghost_rest(&k).as_deref(), Some("gle.com/"));
    named(&mut k, Keysym::Return);
    assert_eq!(
        opened(&mut k).as_deref(),
        Some("https://google.com/"),
        "the scheme is added when :go runs"
    );
    tap(&mut k, KeyInput::ch(':'));
    type_line(&mut k, "go https://google");
    named(&mut k, Keysym::Delete);
    named(&mut k, Keysym::Return);
    assert_eq!(opened(&mut k).as_deref(), Some("https://google"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ranking_history_is_read_once_per_line_not_per_key() {
    let dir = history_dir("once", &[(1, "https://www.google.com/"), (2, "https://www.example.com/")]);
    let mut k = line_with(Some(dir.clone()));
    type_line(&mut k, "go https://goo");
    named(&mut k, Keysym::Tab);
    type_line(&mut k, "g");
    assert_eq!(k.cache.loads.get(), 1, "one read for 14+ keys");
    assert_eq!(k.cache.history().len(), 2);
    // A new line reads it again (a visit since the last line counts).
    named(&mut k, Keysym::Escape);
    tap(&mut k, KeyInput::ch(':'));
    type_line(&mut k, "go g");
    assert_eq!(k.cache.loads.get(), 1, "the new line's own cache");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- external sources (Source::Cmd) ----------

fn ext_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("web-c3-ext-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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
fn external_source_lines_become_candidates_with_the_web_environment() {
    let dir = ext_dir("order");
    let s = script(&dir, "s.sh", "echo alpha\necho alpine\necho beta\n");
    let t = ext_table(&s);
    let (cache, rx) = ext_cache();
    assert!(ext_ghost(t, "x al", &cache).is_none(), "nothing until the source answered");
    let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(cache.accept(&r));
    assert_eq!(ext_ghost(t, "x al", &cache).expect("ghost").candidates, vec!["alpha", "alpine"]);
    assert_eq!(ext_ghost(t, "x alpi", &cache).unwrap().candidates, vec!["alpine"]);
    let (cache2, rx2) = ext_cache();
    let s2 = script(&dir, "e.sh", "echo \"$WEB_ARG|$WEB_WORD|$WEB_URL|$WEB_LINE\"\n");
    let t2 = ext_table(&s2);
    let _ = ext_ghost(t2, "x 1", &cache2);
    let r = rx2.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(r.outcome.unwrap(), vec!["1|1|https://page.test/|x 1"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn external_source_failures_give_no_ghost_and_a_missing_program_a_message() {
    let dir = ext_dir("fail");
    for (name, body) in [("bad.sh", "echo alpha\nexit 3\n"), ("empty.sh", "true\n")] {
        let s = script(&dir, name, body);
        let t = ext_table(&s);
        let (cache, rx) = ext_cache();
        let _ = ext_ghost(t, "x a", &cache);
        let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(r.outcome, Ok(Vec::new()), "{name}");
    }
    let argv: &'static [&'static str] = &["/nonexistent/web-c3-source"];
    let t: &'static [(&'static str, Source)] = Box::leak(Box::new([("x", Source::Cmd(argv))]));
    let (cache, rx) = ext_cache();
    let _ = ext_ghost(t, "x a", &cache);
    let msg = rx.recv_timeout(Duration::from_secs(10)).unwrap().outcome.unwrap_err();
    assert!(msg.contains("not found"), "{msg}");
    // The key model shows it, for its own line only.
    let mut k = line_with(None);
    let id = k.cache.id;
    k.complete_result(ExtResult {
        line_id: id.wrapping_sub(1),
        cmd: "go".into(),
        arg: 1,
        outcome: Err("late".into()),
    });
    assert!(k.message.is_empty(), "a stale result is dropped");
    k.complete_result(ExtResult {
        line_id: id,
        cmd: "x".into(),
        arg: 1,
        outcome: Err(msg.clone()),
    });
    assert_eq!(k.message, msg);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn external_source_shell_syntax_in_the_line_is_plain_text() {
    let dir = ext_dir("plain");
    let pwned = dir.join("PWNED");
    let s = script(&dir, "s.sh", "printf '%sz\\n' \"$WEB_WORD\"\n");
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
fn external_source_runs_once_per_line_and_position_and_is_killed_when_too_slow() {
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
    let _ = ext_ghost(t, "x abc a", &cache);
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap().arg, 2);
    let t0 = Instant::now();
    assert_eq!(run_source(&["sleep", "30"], &[], Duration::from_millis(150)), Ok(Vec::new()));
    assert!(t0.elapsed() < Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- C3-GOHTTP: `:go` runs a history URL with its own scheme ----------

fn go_run(lines: &[(u64, &str)], tag: &str, typed: &str, tab: bool) -> Option<String> {
    let dir = history_dir(tag, lines);
    let mut k = line_with(Some(dir.clone()));
    type_line(&mut k, typed);
    if tab {
        named(&mut k, Keysym::Tab);
    }
    named(&mut k, Keysym::Return);
    let r = opened(&mut k);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

#[test]
fn keys_t50_t53_go_http_history_entry_completed_and_run_keeps_http() {
    let h = [(1, "http://127.0.0.1:8000/")];
    assert_eq!(
        go_run(&h, "gh1", "go 127", true).as_deref(),
        Some("http://127.0.0.1:8000/"),
        "Tab, Enter"
    );
    assert_eq!(
        go_run(&h, "gh2", "go 127", false).as_deref(),
        Some("http://127.0.0.1:8000/"),
        "Enter with the ghost"
    );
    assert_eq!(
        go_run(&[(1, "http://www.plain.test/")], "gh3", "go plain", false).as_deref(),
        Some("http://plain.test/")
    );
}

#[test]
fn keys_t50_go_https_history_entry_keeps_https() {
    let h = [(1, "https://www.google.com/")];
    assert_eq!(go_run(&h, "gh4", "go goo", true).as_deref(), Some("https://google.com/"));
}

#[test]
fn keys_t50_go_a_typed_scheme_wins() {
    let h = [(1, "http://127.0.0.1:8000/"), (2, "https://127.0.0.1:8000/")];
    assert_eq!(go_run(&h, "gh5", "go http://127", false).as_deref(), Some("http://127.0.0.1:8000/"));
    assert_eq!(
        go_run(&h, "gh6", "go https://127", false).as_deref(),
        Some("https://127.0.0.1:8000/")
    );
    // Ghost deleted: what was typed runs, scheme-less words included.
    let dir = history_dir("gh7", &h);
    let mut k = line_with(Some(dir.clone()));
    type_line(&mut k, "go http://x");
    named(&mut k, Keysym::Delete);
    named(&mut k, Keysym::Return);
    assert_eq!(opened(&mut k).as_deref(), Some("http://x"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keys_t50_go_without_a_history_match_assumes_https() {
    assert_eq!(go_run(&[], "gh8", "go example.com", false).as_deref(), Some("https://example.com"));
    let h = [(1, "http://127.0.0.1:8000/")];
    // typed word is not the whole stripped URL (ghost deleted): no history match, https
    let dir = history_dir("gh9", &h);
    let mut k = line_with(Some(dir.clone()));
    type_line(&mut k, "go 127");
    named(&mut k, Keysym::Delete);
    named(&mut k, Keysym::Return);
    assert_eq!(opened(&mut k).as_deref(), Some("https://127"));
    let _ = std::fs::remove_dir_all(&dir);
}
