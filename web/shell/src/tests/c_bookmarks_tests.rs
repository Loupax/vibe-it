//! C-BOOKMARKS (KEYS.md "Bookmarks", T69-T76). Each test names its row.

use std::path::PathBuf;
use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::bookmarks;
use crate::completion::{compute_ghost, CompleteCache};
use crate::keys::*;
use crate::picker::PickAction;
use crate::protocol::Command;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("web-c-bookmarks-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn file(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
    fn bookmarks(&self) -> String {
        std::fs::read_to_string(self.0.join("bookmarks")).unwrap_or_default()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn keys_on(dir: &Dir, url: &str, title: &str) -> Keys {
    let mut k = Keys::new(Some(dir.0.clone()), "file:///help.html".into(), None);
    k.url = url.into();
    k.title = title.into();
    k
}

fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

fn run(k: &mut Keys, line: &str) {
    press(k, KeyInput::ch(':'));
    for c in line.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
    // A ghost could add to what was typed; Delete hides it (T52).
    press(k, KeyInput::sym(Keysym::Delete));
    press(k, KeyInput::sym(Keysym::Return));
}

fn lines(d: &Dir) -> Vec<String> {
    d.bookmarks().lines().map(String::from).collect()
}

#[test]
fn keys_t69_bookmark_add_writes_the_page_and_its_title() {
    use std::os::unix::fs::PermissionsExt;
    let d = Dir::new("t69");
    let mut k = keys_on(&d, "https://a.test/page", "A page");
    run(&mut k, "bookmarkAdd");
    assert_eq!(lines(&d), ["https://a.test/page\tA page"]);
    assert_eq!(k.message, "bookmarked: https://a.test/page");
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(
        std::fs::metadata(d.0.join("bookmarks")).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // Added in order; a title that is only the URL (a page without one) is stored as no title.
    k.url = "https://b.test/".into();
    k.title = "b.test/".into();
    run(&mut k, "bookmarkAdd");
    assert_eq!(lines(&d), ["https://a.test/page\tA page", "https://b.test/"]);
    // Nothing to bookmark.
    k.url = "about:blank".into();
    run(&mut k, "bookmarkAdd");
    assert_eq!(k.message, "web: no page to bookmark");
    assert_eq!(lines(&d).len(), 2);
}

#[test]
fn keys_t70_bookmark_add_twice_updates_the_title() {
    let d = Dir::new("t70");
    let mut k = keys_on(&d, "https://a.test/", "Old");
    run(&mut k, "bookmarkAdd");
    k.url = "https://b.test/".into();
    k.title = "B".into();
    run(&mut k, "bookmarkAdd");
    k.url = "https://a.test/".into();
    k.title = "New\tTitle".into();
    run(&mut k, "bookmarkAdd");
    // Same place in the file, new title (a tab in it becomes a space), still one line per URL.
    assert_eq!(lines(&d), ["https://a.test/\tNew Title", "https://b.test/\tB"]);
    assert_eq!(k.message, "bookmark updated: https://a.test/");
    // No temp file is left behind.
    let left: Vec<_> = std::fs::read_dir(&d.0)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with(".bookmarks.tmp"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn keys_t71_bookmark_delete_removes_the_current_page_or_the_given_url() {
    let d = Dir::new("t71");
    d.file("bookmarks", "https://a.test/\tA\nhttps://b.test/\tB\nhttps://c.test/\tC\n");
    let mut k = keys_on(&d, "https://a.test/", "A");
    run(&mut k, "bookmarkDelete");
    assert_eq!(lines(&d), ["https://b.test/\tB", "https://c.test/\tC"]);
    assert_eq!(k.message, "bookmark removed: https://a.test/");
    run(&mut k, "bookmarkDelete https://c.test/");
    assert_eq!(lines(&d), ["https://b.test/\tB"]);
    assert_eq!(k.message, "bookmark removed: https://c.test/");
}

#[test]
fn keys_t72_bookmark_delete_of_an_unknown_url_changes_nothing() {
    let d = Dir::new("t72");
    d.file("bookmarks", "https://a.test/\tA\n");
    let mut k = keys_on(&d, "https://zzz.test/", "Z");
    run(&mut k, "bookmarkDelete");
    assert_eq!(k.message, "web: not bookmarked: https://zzz.test/");
    assert_eq!(lines(&d), ["https://a.test/\tA"]);
}

#[test]
fn keys_t73_bookmark_search_opens_the_picker_and_enter_opens_the_url() {
    let d = Dir::new("t73");
    // A hand-edited file: a line with no tab, a blank line, a duplicate.
    d.file(
        "bookmarks",
        "https://a.test/\tAlpha site\n\nhttps://b.test/docs\tBeta docs\nhttps://c.test/\nhttps://a.test/\tAlpha\n",
    );
    let mut k = keys_on(&d, "https://x.test/", "X");
    run(&mut k, "bookmarkSearch");
    assert_eq!(k.vim_mode, VimMode::Pick);
    let p = k.picker.as_ref().unwrap();
    assert_eq!(p.name, "bookmarks");
    // Newest first; a bookmark with no title shows its URL; the duplicate kept its first place, last title.
    let labels: Vec<&str> = p.matches().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["https://c.test/", "Beta docs", "Alpha"]);
    // Filter over the URL and over the title.
    for c in "beta".chars() {
        press(&mut k, KeyInput::ch(c));
    }
    assert_eq!(k.picker.as_ref().unwrap().match_count(), 1);
    k.effects.clear();
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(k.vim_mode, VimMode::Normal);
    let opened: Vec<_> = k
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(Command::Open { url }) => Some(url.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(opened, ["https://b.test/docs"]);
    // By URL: `c.test` finds the row that has no title.
    run(&mut k, "bookmarkSearch c.test");
    let p = k.picker.as_ref().unwrap();
    assert_eq!(p.match_count(), 1);
    assert!(matches!(&p.selected_row().unwrap().action, PickAction::Open(u) if u == "https://c.test/"));
}

#[test]
fn keys_t74_bookmark_search_with_no_bookmarks_says_so() {
    let d = Dir::new("t74");
    let mut k = keys_on(&d, "https://x.test/", "X");
    run(&mut k, "bookmarkSearch");
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(k.message, "web: no bookmarks (:bookmarkAdd adds the page)");
}

#[test]
fn keys_t75_go_ghost_ranks_bookmarks_before_history() {
    let d = Dir::new("t75");
    // The history has more visits to example.com than the bookmark's site has, so on history alone
    // `exa` gives the history's URL; the bookmark wins when there is one.
    d.file(
        "history",
        "1\thttps://example.com/\t\n2\thttps://example.com/\t\n3\thttps://example.com/\t\n4\thttps://example.net/\t\n",
    );
    let cache = CompleteCache::new(Some(d.0.clone()));
    assert_eq!(compute_ghost("go exa", 6, &cache).unwrap().candidates[0], "example.com/");
    d.file("bookmarks", "https://www.example.net/page\tNet\n");
    let cache = CompleteCache::new(Some(d.0.clone()));
    let g = compute_ghost("go exa", 6, &cache).unwrap();
    assert_eq!(
        g.candidates,
        ["example.net/page", "example.com/", "example.net/"],
        "bookmark first, then the history's order"
    );
}

#[test]
fn keys_t76_bookmark_delete_argument_completes_from_the_bookmarks() {
    let d = Dir::new("t76");
    d.file(
        "bookmarks",
        "https://a.test/one\tOne\nhttps://a.test/two\tTwo\nhttps://b.test/\tB\n",
    );
    d.file("history", "1\thttps://a.test/hist\t\n");
    let cache = CompleteCache::new(Some(d.0.clone()));
    let g = compute_ghost("bookmarkDelete https://a", 24, &cache).unwrap();
    assert_eq!(
        g.candidates,
        ["https://a.test/one", "https://a.test/two"],
        "bookmarks only, not the history"
    );
    // The command names complete too.
    let cache = CompleteCache::new(Some(d.0.clone()));
    let g = compute_ghost("bookm", 5, &cache).unwrap();
    assert_eq!(g.candidates, ["bookmarkAdd", "bookmarkDelete", "bookmarkSearch"]);
}

#[test]
fn bookmarks_file_parse_and_rewrite_helpers() {
    let l = bookmarks::parse("https://a/\tA\n\nhttps://b/\nhttps://a/\tA2\n");
    assert_eq!(
        l,
        [
            ("https://a/".to_string(), "A2".to_string()),
            ("https://b/".to_string(), String::new())
        ]
    );
    let d = Dir::new("helpers");
    assert_eq!(bookmarks::add(&d.0, "https://a/", "A").unwrap(), bookmarks::Added::New);
    assert!(bookmarks::delete(&d.0, "https://a/").unwrap());
    assert!(!bookmarks::delete(&d.0, "https://a/").unwrap());
    assert_eq!(d.bookmarks(), "");
}
