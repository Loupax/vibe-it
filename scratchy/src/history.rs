//! QCOMP3: the history file writer (STASH.md "History", QCOMP-D C5/C7).
//!
//! `DIR/history`, mode 0600, one line per visit: `<unix ms>\t<url>\t<title>\n`. A line is at most
//! [`MAX_LINE`] bytes and goes out in one `write` on an `O_APPEND` file, so several scratchy
//! processes can append without a lock. The startup trim keeps the newest `HISTORY_MAX_LINES`
//! lines under an exclusive flock on `DIR/.history.lock`.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use markup5ever_rcdom::Handle;
use url::Url;

use crate::config::{HISTORY, HISTORY_MAX_LINES};

/// Longest line, newline included (a `write` this small to an `O_APPEND` file is not interleaved).
pub(crate) const MAX_LINE: usize = 4096;

static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Process start: remembers the state directory and trims an over-long file. No-op when `HISTORY` is off.
pub(crate) fn init(dir: Option<PathBuf>) {
    if !HISTORY {
        return;
    }
    if let Some(d) = &dir
        && let Err(e) = trim(d, HISTORY_MAX_LINES)
    {
        eprintln!("scratchy: history trim failed: {e}");
    }
    *DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir;
}

/// The state directory the history lives in (`None`: no history, or `HISTORY` is off).
pub(crate) fn dir() -> Option<PathBuf> {
    DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The URL to record for a finished top-level load: the final URL without its fragment, only for
/// web origins (`file:`, `data:`, `about:` and other opaque origins are not visits).
pub(crate) fn visit_url(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let mut u = url.clone();
    u.set_fragment(None);
    Some(u.into())
}

/// Records one visit now (errors are logged, never shown to the user).
pub(crate) fn record_visit(url: &Url, title: &str) {
    let Some(dir) = DIR.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
    let Some(url) = visit_url(url) else { return };
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    if let Err(e) = append(&dir, ms, &url, title) {
        eprintln!("scratchy: history append failed: {e}");
    }
}

/// Tabs, newlines and other control characters become a space.
fn escape_title(title: &str) -> String {
    title.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string()
}

/// The line for a visit, at most [`MAX_LINE`] bytes (the title is cut on a char boundary).
/// `None` when even the URL does not fit.
pub(crate) fn format_line(ms: u64, url: &str, title: &str) -> Option<String> {
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
pub(crate) fn append(dir: &Path, ms: u64, url: &str, title: &str) -> io::Result<()> {
    let Some(line) = format_line(ms, url, title) else { return Ok(()) };
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let mut f = OpenOptions::new().append(true).create(true).mode(0o600).open(dir.join("history"))?;
    f.write_all(line.as_bytes())
}

/// Keeps the newest `max` lines: temp file + rename under an exclusive flock on `.history.lock`.
pub(crate) fn trim(dir: &Path, max: usize) -> io::Result<()> {
    let path = dir.join("history");
    if !path.exists() {
        return Ok(());
    }
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(dir.join(".history.lock"))?;
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

/// The text of the first `<title>` in `document` (when no script engine can answer `document.title`).
pub(crate) fn dom_title(document: &Handle) -> String {
    fn find(h: &Handle) -> Option<Handle> {
        if crate::dom::element_tag(h) == "title" {
            return Some(h.clone());
        }
        h.children.borrow().iter().find_map(find)
    }
    let Some(t) = find(document) else { return String::new() };
    let mut s = String::new();
    crate::images::collect_text_only(&t, &mut s);
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
