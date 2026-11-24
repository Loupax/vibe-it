//! C-CMDHIST: the history of the `:` lines run (STASH.md "Command history"), browsed with Up/Down.
//!
//! `DIR/cmdhistory`, mode 0600 (dir 0700), one `:` line per run, no timestamps. Each line goes out
//! in one `write` on an `O_APPEND` file (a crash loses nothing). At start the file is read, control
//! characters are stripped, duplicates dropped (the newest stays), and the newest
//! `CMD_HISTORY_MAX` kept; if that changed anything the file is rewritten as a temp file + rename.
//! C-SINGLE runs one web per state directory, so there is one writer. No state directory: memory only.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::config::CMD_HISTORY_MAX;

/// Longest line kept, newline included.
pub const MAX_LINE: usize = 4096;

const FILE: &str = "cmdhistory";

/// Control characters stripped, surrounding blanks trimmed, cut at [`MAX_LINE`] - 1 bytes.
pub fn clean(line: &str) -> String {
    let s: String = line.chars().filter(|c| !c.is_control()).collect();
    let s = s.trim();
    let mut end = s.len().min(MAX_LINE - 1);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Newest last; a repeated line keeps only its newest place; at most `max` lines.
fn normalize(raw: impl Iterator<Item = String>, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in raw {
        let l = clean(&l);
        if l.is_empty() {
            continue;
        }
        out.retain(|e| *e != l);
        out.push(l);
    }
    let over = out.len().saturating_sub(max);
    out.drain(..over);
    out
}

/// Where Up/Down is while browsing: `prefix` is what was typed when it began.
#[derive(Debug, Clone)]
struct Browse {
    prefix: String,
    /// Index into the entries.
    pos: usize,
}

#[derive(Debug)]
pub struct CmdHistory {
    dir: Option<PathBuf>,
    entries: Vec<String>,
    max: usize,
    browse: Option<Browse>,
}

impl CmdHistory {
    /// Reads, dedups and trims the file in `dir` (None: memory only).
    pub fn open(dir: Option<PathBuf>) -> CmdHistory {
        Self::open_max(dir, CMD_HISTORY_MAX)
    }

    pub fn open_max(dir: Option<PathBuf>, max: usize) -> CmdHistory {
        let mut h = CmdHistory {
            dir,
            entries: Vec::new(),
            max,
            browse: None,
        };
        if let Some(d) = h.dir.clone() {
            if let Ok(bytes) = fs::read(d.join(FILE)) {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                h.entries = normalize(text.lines().map(String::from), max);
                let clean_file: String = h.entries.iter().map(|e| format!("{e}\n")).collect();
                if clean_file.as_bytes() != bytes.as_slice() {
                    if let Err(e) = rewrite(&d, &clean_file) {
                        eprintln!("web: command history trim failed: {e}");
                    }
                }
            }
        }
        h
    }

    /// The line run last (C-REPEAT's `@:`).
    #[allow(dead_code)] // read by C-REPEAT
    pub fn last(&self) -> Option<&str> {
        self.entries.last().map(String::as_str)
    }

    #[allow(dead_code)] // tests
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// A line Enter ran in the `:` line: appended to the file and made the newest.
    pub fn record(&mut self, line: &str) {
        self.browse = None;
        let l = clean(line);
        if l.is_empty() {
            return;
        }
        self.entries.retain(|e| *e != l);
        self.entries.push(l.clone());
        let over = self.entries.len().saturating_sub(self.max);
        self.entries.drain(..over);
        if let Some(d) = &self.dir {
            if let Err(e) = append(d, &l) {
                eprintln!("web: command history append failed: {e}");
            }
        }
    }

    pub fn end_browse(&mut self) {
        self.browse = None;
    }

    /// Up: the newest older entry starting with what was typed when browsing began. `typed` is the
    /// line now. None: no match, nothing changes.
    pub fn up(&mut self, typed: &str) -> Option<String> {
        let (prefix, pos) = match &self.browse {
            Some(b) => (b.prefix.clone(), b.pos),
            None => (typed.to_string(), self.entries.len()),
        };
        let i = (0..pos).rev().find(|&i| self.entries[i].starts_with(&prefix))?;
        let text = self.entries[i].clone();
        self.browse = Some(Browse { prefix, pos: i });
        Some(text)
    }

    /// Down: the next newer match; past the newest the typed text comes back. None: not browsing.
    pub fn down(&mut self) -> Option<String> {
        let b = self.browse.as_ref()?;
        let (prefix, pos) = (b.prefix.clone(), b.pos);
        match (pos + 1..self.entries.len()).find(|&i| self.entries[i].starts_with(&prefix)) {
            Some(i) => {
                self.browse = Some(Browse { prefix, pos: i });
                Some(self.entries[i].clone())
            }
            None => {
                self.browse = None;
                Some(prefix)
            }
        }
    }
}

fn append(dir: &Path, line: &str) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let mut f = OpenOptions::new().append(true).create(true).mode(0o600).open(dir.join(FILE))?;
    f.write_all(format!("{line}\n").as_bytes())
}

fn rewrite(dir: &Path, content: &str) -> io::Result<()> {
    let tmp = dir.join(format!(".cmdhistory.tmp-{}", std::process::id()));
    {
        let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, dir.join(FILE))
}
