//! The history file (STASH.md "History"), ported from scratchy's `src/history.rs` (QCOMP3).
//!
//! `DIR/history`, mode 0600, one line per visit: `<unix ms>\t<url>\t<title>\n`. A line is at
//! most [`MAX_LINE`] bytes and goes out in one `write` on an `O_APPEND` file, so several web
//! processes can append without a lock. The startup trim keeps the newest
//! `HISTORY_MAX_LINES` lines under an exclusive flock on `DIR/.history.lock`.
//!
//! web: the engine host reports the loads (`url`, `title`, `load` events, PROTOCOL.md);
//! [`Visits`] decides from them what is a visit.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use url::Url;

use crate::config::{HISTORY, HISTORY_MAX_LINES};
use crate::protocol::Event;

/// Longest line, newline included (a `write` this small to an `O_APPEND` file is not interleaved).
pub const MAX_LINE: usize = 4096;

/// The URL to record for a finished top-level load: the final URL without its fragment,
/// only for web origins (`file:`, `data:`, `about:` and other opaque origins are not visits).
pub fn visit_url(url: &str) -> Option<String> {
    let mut u = Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        return None;
    }
    u.set_fragment(None);
    Some(u.into())
}

/// Tabs, newlines and other control characters become a space.
fn escape_title(title: &str) -> String {
    title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// The line for a visit, at most [`MAX_LINE`] bytes (the title is cut on a char boundary).
/// `None` when even the URL does not fit.
pub fn format_line(ms: u64, url: &str, title: &str) -> Option<String> {
    let url: String = url.chars().filter(|c| !c.is_control()).collect();
    let mut line = format!("{ms}\t{url}\t");
    if line.len() + 1 > MAX_LINE {
        return None;
    }
    let room = MAX_LINE - line.len() - 1;
    let title = escape_title(title);
    let mut end = title.len().min(room);
    while !title.is_char_boundary(end) {
        end -= 1;
    }
    line.push_str(&title[..end]);
    line.push('\n');
    Some(line)
}

/// Appends one visit line (one `write`, `O_APPEND`, mode 0600); creates `dir` (0700) if missing.
pub fn append(dir: &Path, ms: u64, url: &str, title: &str) -> io::Result<()> {
    let Some(line) = format_line(ms, url, title) else { return Ok(()) };
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let mut f = OpenOptions::new().append(true).create(true).mode(0o600).open(dir.join("history"))?;
    f.write_all(line.as_bytes())
}

/// Keeps the newest `max` lines: temp file + rename under an exclusive flock on `.history.lock`.
pub fn trim(dir: &Path, max: usize) -> io::Result<()> {
    let path = dir.join("history");
    if !path.exists() {
        return Ok(());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join(".history.lock"))?;
    lock.lock()?;
    let mut data = Vec::new();
    File::open(&path)?.read_to_end(&mut data)?;
    let lines = data.iter().filter(|&&b| b == b'\n').count();
    if lines > max {
        // Keep everything after the (lines - max)th newline.
        let skip = lines - max;
        let mut seen = 0;
        let start = data
            .iter()
            .position(|&b| {
                if b == b'\n' {
                    seen += 1;
                }
                seen == skip
            })
            .map_or(0, |i| i + 1);
        let tmp = dir.join(format!(".history.tmp-{}", std::process::id()));
        {
            let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            f.write_all(&data[start..])?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
    }
    Ok(()) // dropping `lock` releases the flock
}

/// The history writer for one state directory (`None`: no history, or `HISTORY` is off).
pub struct History {
    dir: Option<PathBuf>,
}

impl History {
    /// Process start: remembers the state directory and trims an over-long file.
    pub fn open(dir: Option<PathBuf>) -> History {
        let dir = dir.filter(|_| HISTORY);
        if let Some(d) = &dir {
            if let Err(e) = trim(d, HISTORY_MAX_LINES) {
                eprintln!("web: history trim failed: {e}");
            }
        }
        History { dir }
    }

    /// Records one visit now (errors are logged, never shown to the user).
    pub fn record(&self, url: &str, title: &str) {
        let Some(dir) = &self.dir else { return };
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if let Err(e) = append(dir, ms, url, title) {
            eprintln!("web: history append failed: {e}");
        }
    }
}

/// Turns the host's events into visits (STASH.md "What is a visit"): a main-frame load of
/// kind `new` that ended with a response (`status` > 0) and no error is one visit of its
/// final URL (the `url` event comes before its `load end`, PROTOCOL.md), with the title at
/// that moment. Reloads, traversals, same-document navigations, failed loads and opaque
/// origins are not visits.
#[derive(Debug, Default)]
pub struct Visits {
    url: String,
    title: String,
}

impl Visits {
    /// Feeds one event; returns `(url, title)` when it completed a visit.
    pub fn event(&mut self, ev: &Event) -> Option<(String, String)> {
        match ev {
            Event::Url { url, .. } => self.url = url.clone(),
            Event::Title { title, .. } => self.title = title.clone(),
            // A new document starts with no title; the previous page's must not stick to it.
            Event::Load { phase, kind, .. } if phase == "start" && kind != "same-document" => self.title.clear(),
            Event::Load {
                phase,
                status,
                kind,
                error,
                ..
            } if phase == "end" && kind == "new" && *status > 0 && error.is_none() => {
                let url = visit_url(&self.url)?;
                // Chromium reports the URL as the title of a page without one: record none.
                let bare = |u: &str| -> String { u.split_once("://").map_or(u, |(_, r)| r).to_string() };
                let title = if self.title == self.url || self.title == bare(&self.url) || self.title == bare(&url) {
                    String::new()
                } else {
                    self.title.clone()
                };
                return Some((url, title));
            }
            _ => {}
        }
        None
    }
}
