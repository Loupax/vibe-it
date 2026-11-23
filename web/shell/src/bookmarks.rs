//! The bookmarks file (STASH.md "Bookmarks", C-BOOKMARKS).
//!
//! `DIR/bookmarks`, mode 0600, plain UTF-8, one bookmark per line, `url<TAB>title`, in the order they
//! were added; meant to be editable by hand (a line without a tab is a URL with no title, blank lines
//! are skipped). Every change reads the file, edits the list and writes it back as a temp file
//! (`DIR/.bookmarks.tmp-<pid>`) renamed over `bookmarks`, under an exclusive flock on `DIR/.history.lock`
//! (the history's lock), so a crash never leaves a half-written file.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use crate::picker::{PickAction, Row};

/// What `add` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Added {
    New,
    /// The URL was there: its title was replaced.
    Updated,
}

/// Tabs, newlines and other control characters become a space.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// The file's text as (url, title) pairs in file order. A URL seen twice keeps its first place and
/// its last title.
pub fn parse(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let (url, title) = line.split_once('\t').unwrap_or((line, ""));
        let (url, title) = (url.trim(), title.trim());
        if url.is_empty() {
            continue;
        }
        match out.iter_mut().find(|(u, _)| u == url) {
            Some(e) => e.1 = title.to_string(),
            None => out.push((url.to_string(), title.to_string())),
        }
    }
    out
}

/// The bookmarks in `DIR/bookmarks` (none without a state directory or file).
pub fn load(dir: Option<&Path>) -> Vec<(String, String)> {
    let data = dir.and_then(|d| fs::read(d.join("bookmarks")).ok()).unwrap_or_default();
    parse(&String::from_utf8_lossy(&data))
}

fn render(list: &[(String, String)]) -> String {
    let mut s = String::new();
    for (u, t) in list {
        s.push_str(u);
        if !t.is_empty() {
            s.push('\t');
            s.push_str(t);
        }
        s.push('\n');
    }
    s
}

/// Reads, edits and rewrites the file safely; `edit` returns what to report.
fn change<T>(dir: &Path, edit: impl FnOnce(&mut Vec<(String, String)>) -> T) -> io::Result<T> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join(".history.lock"))?;
    lock.lock()?;
    let mut list = load(Some(dir));
    let before = list.clone();
    let r = edit(&mut list);
    if list != before {
        let tmp = dir.join(format!(".bookmarks.tmp-{}", std::process::id()));
        {
            let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            f.write_all(render(&list).as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, dir.join("bookmarks"))?;
    }
    Ok(r)
}

/// Adds `url` with `title`, or replaces the title of the one already there.
pub fn add(dir: &Path, url: &str, title: &str) -> io::Result<Added> {
    let url: String = url.chars().filter(|c| !c.is_control()).collect();
    let title = clean(title);
    change(dir, |list| match list.iter_mut().find(|(u, _)| *u == url) {
        Some(e) => {
            e.1 = title;
            Added::Updated
        }
        None => {
            list.push((url, title));
            Added::New
        }
    })
}

/// Removes `url`; false when it was not bookmarked.
pub fn delete(dir: &Path, url: &str) -> io::Result<bool> {
    change(dir, |list| {
        let n = list.len();
        list.retain(|(u, _)| u != url);
        list.len() != n
    })
}

/// `:bookmarkSearch`'s rows: newest first. Label: the title (a bookmark with none shows its URL, no
/// detail); detail: the URL. Matched on the URL without `https://`/`http://`/`www.`, and the title.
pub fn rows(list: &[(String, String)]) -> Vec<Row> {
    list.iter()
        .rev()
        .map(|(url, title)| {
            let (label, detail) = if title.is_empty() {
                (url.clone(), String::new())
            } else {
                (title.clone(), url.clone())
            };
            Row {
                label,
                detail,
                fields: vec![crate::completion::strip_scheme_www(url).to_string(), title.clone()],
                action: PickAction::Open(url.clone()),
            }
        })
        .collect()
}
