//! QCOMP1: the `:` line's ghost suggestion (KEYS.md "Completion", T45-T53; QCOMP-D C1, C8).
//!
//! Everything here is a pure function over the line text and the cursor: `compute_ghost` finds the
//! word under the cursor, asks its source (the first word: the command names; later words: the
//! command's row in `config::COMPLETERS`) and returns the matches; `Ghost` holds them and the
//! index Tab / Shift-Tab move. `App` only stores the `Ghost`, recomputes it after each edit and
//! draws `Ghost::rest`.

use std::cell::{Cell, OnceCell};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::{Source, COMPLETERS, SPACE_AFTER_ARG};

/// What a source is asked: the whole line, the word being completed and its argument index
/// (0 = the command name, 1 = the first argument).
pub(crate) struct CompleteCtx<'a> {
    #[allow(dead_code)] // for sources that look at the whole line (QCOMP2 passes it as SCRATCHY_LINE)
    pub line: &'a str,
    pub word: &'a str,
    #[allow(dead_code)] // for sources with a different list per argument position
    pub arg: usize,
    /// What sources read once per command line (the ranked history).
    pub cache: &'a CompleteCache,
}

/// One ranked history entry: the URL as visited and where its comparison text starts (after
/// `https://`/`http://` and `www.`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryEntry {
    pub url: String,
    pub skip: usize,
}

/// Per-command-line state of the sources (QCOMP4): the ranked history is read from the file the
/// first time `:go` asks for it and kept until the line is closed (the app makes a new cache each
/// time the `:` line opens), so typing never re-reads the file.
pub(crate) struct CompleteCache {
    dir: Option<PathBuf>,
    history: OnceCell<Vec<HistoryEntry>>,
    /// How many times the file was read (tests check "once per line").
    pub loads: Cell<usize>,
}

impl CompleteCache {
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        CompleteCache { dir, history: OnceCell::new(), loads: Cell::new(0) }
    }

    /// The history in ghost order (see [`rank_history`]); empty without a state directory or file.
    pub(crate) fn history(&self) -> &[HistoryEntry] {
        self.history.get_or_init(|| {
            self.loads.set(self.loads.get() + 1);
            let data = self.dir.as_ref().and_then(|d| std::fs::read(d.join("history")).ok()).unwrap_or_default();
            rank_history(&String::from_utf8_lossy(&data))
        })
    }
}

/// `url` without `https://`/`http://` and then `www.`: what typed words are compared with.
fn strip_scheme_www(url: &str) -> &str {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
    rest.strip_prefix("www.").unwrap_or(rest)
}

/// STASH.md "Ranking": the history file's text (`ms\turl\ttitle` lines) as the ghost order: one
/// site root per origin ranked by visits to any page of it, then one entry per other URL ranked by
/// its own visits; ties go to the newest visit. Lines that do not parse (no tab, bad time, a URL
/// that is not http(s) with a host) are skipped.
pub(crate) fn rank_history(text: &str) -> Vec<HistoryEntry> {
    // (visits, newest ms) per origin root and per page.
    let mut sites: HashMap<String, (u32, u64)> = HashMap::new();
    let mut pages: HashMap<String, (u32, u64)> = HashMap::new();
    for line in text.lines() {
        let mut f = line.split('\t');
        let (Some(ms), Some(url)) = (f.next(), f.next()) else { continue };
        let Ok(ms) = ms.trim().parse::<u64>() else { continue };
        let Ok(parsed) = url::Url::parse(url) else { continue };
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            continue;
        }
        let root = format!("{}/", parsed.origin().ascii_serialization());
        let bump = |m: &mut HashMap<String, (u32, u64)>, k: String| {
            let e = m.entry(k).or_insert((0, 0));
            e.0 += 1;
            e.1 = e.1.max(ms);
        };
        if url != root {
            bump(&mut pages, url.to_string());
        }
        bump(&mut sites, root);
    }
    let sorted = |m: HashMap<String, (u32, u64)>| {
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    };
    sorted(sites)
        .into_iter()
        .chain(sorted(pages))
        .map(|(url, _)| {
            let skip = url.len() - strip_scheme_www(&url).len();
            HistoryEntry { url, skip }
        })
        .collect()
}

/// The built-in source of `:go`'s argument (C7): the history in ranked order. A URL that starts
/// with the word is a candidate as it is; otherwise the word is compared with `https://`/`http://`
/// and `www.` removed from both sides and the candidate is the word plus the rest of the URL, so it
/// always continues exactly what was typed (typed `https://google`, visited `https://www.google.com/`:
/// candidate `https://google.com/`).
pub(crate) fn complete_history_urls(ctx: &CompleteCtx) -> Vec<String> {
    let typed = strip_scheme_www(ctx.word);
    let mut out: Vec<String> = Vec::new();
    for e in ctx.cache.history() {
        let cand = if e.url.starts_with(ctx.word) {
            e.url.clone()
        } else if let Some(rest) = e.url[e.skip..].strip_prefix(typed) {
            format!("{}{rest}", ctx.word)
        } else {
            continue;
        };
        if !out.contains(&cand) {
            out.push(cand);
        }
    }
    out
}

/// A live ghost: the candidates for the word that starts at `word_start` (each one a full word
/// that starts with what was typed) and which one is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ghost {
    pub word_start: usize,
    pub candidates: Vec<String>,
    pub index: usize,
    /// Whether selecting the only match is followed by a space (T53: a command name or a `:set`
    /// argument yes, a URL no).
    pub space_after: bool,
}

impl Ghost {
    /// The dimmed text to draw after the cursor and to run on Enter: the shown candidate minus
    /// what is already typed (`text[word_start..]`).
    pub(crate) fn rest(&self, text: &str) -> String {
        let typed = text.get(self.word_start..).unwrap_or("");
        self.candidates[self.index].strip_prefix(typed).unwrap_or("").to_string()
    }

    /// Tab (T47): the next match, wrapping.
    pub(crate) fn next(&mut self) {
        self.index = (self.index + 1) % self.candidates.len();
    }

    /// Shift-Tab (T48): the previous match, wrapping.
    pub(crate) fn prev(&mut self) {
        self.index = (self.index + self.candidates.len() - 1) % self.candidates.len();
    }

    /// Tab with exactly one match (T53) selects it: returns the text to append to the line (the
    /// ghost, plus a space when `space_after`). With 2+ matches it cycles (the typed text does not
    /// change) and returns `None`.
    pub(crate) fn tab(&mut self, text: &str) -> Option<String> {
        if self.candidates.len() == 1 {
            let mut add = self.rest(text);
            if self.space_after {
                add.push(' ');
            }
            Some(add)
        } else {
            self.next();
            None
        }
    }

    /// The line text with the ghost accepted (Enter, T50; Right/End, T51).
    pub(crate) fn accepted(&self, text: &str) -> String {
        format!("{text}{}", self.rest(text))
    }
}

/// The built-in source of the first word: the command names.
pub(crate) fn complete_command_names(ctx: &CompleteCtx) -> Vec<String> {
    crate::COMMAND_NAMES.iter().filter(|n| n.starts_with(ctx.word)).map(|n| n.to_string()).collect()
}

/// The built-in source of `:set`'s arguments (`config::SET_ARGS`).
pub(crate) fn complete_set_args(ctx: &CompleteCtx) -> Vec<String> {
    crate::config::SET_ARGS.iter().filter(|a| a.starts_with(ctx.word)).map(|a| a.to_string()).collect()
}

/// T45/T46: the ghost for `text` with the cursor at byte offset `cursor`, or `None` (cursor not
/// at the end, empty word, a command without a `COMPLETERS` row, no match).
pub(crate) fn compute_ghost(text: &str, cursor: usize, cache: &CompleteCache) -> Option<Ghost> {
    if cursor != text.len() {
        return None;
    }
    let word_start = crate::token_start_before(text, cursor);
    let word = &text[word_start..];
    if word.is_empty() {
        return None;
    }
    let mut earlier = text[..word_start].split_whitespace();
    let command = earlier.next();
    let arg = earlier.count() + usize::from(command.is_some());
    let ctx = CompleteCtx { line: text, word, arg, cache };
    let (candidates, space_after) = match command {
        None => (complete_command_names(&ctx), true),
        Some(cmd) => {
            let (_, Source::Builtin(f)) = COMPLETERS.iter().find(|(name, _)| *name == cmd)?;
            (f(&ctx), SPACE_AFTER_ARG.contains(&cmd))
        }
    };
    let candidates: Vec<String> = candidates.into_iter().filter(|c| c.starts_with(word)).collect();
    if candidates.is_empty() {
        return None;
    }
    Some(Ghost { word_start, candidates, index: 0, space_after })
}
