//! The `:` line's ghost suggestion (KEYS.md "Completion", T45-T53), ported from scratchy's
//! `src/completion.rs` (its QCOMP1-QCOMP4).
//!
//! Everything here is a pure function over the line text and the cursor: `compute_ghost` finds the
//! word under the cursor, asks its source (the first word: the command names; later words: the
//! command's row in `config::COMPLETERS`) and returns the matches; `Ghost` holds them and the
//! index Tab / Shift-Tab move. `Keys` only stores the `Ghost`, recomputes it after each edit and
//! draws `Ghost::rest`.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::{Source, COMPLETERS, SOURCE_MAX_OUTPUT, SOURCE_TIMEOUT_MS, SPACE_AFTER_ARG};

/// QCOMP2: what an external source's thread reports back (through the event loop's channel).
/// `line_id` names the `:` line it was started for; a result for another line is dropped.
/// `outcome` is the stdout lines (empty after a non-zero exit or no output) or a status-bar message.
pub struct ExtResult {
    pub line_id: u64,
    pub cmd: String,
    pub arg: usize,
    pub outcome: Result<Vec<String>, String>,
}

/// Where an external source's thread delivers its [`ExtResult`] (the app posts an event).
pub type ExtSink = Arc<dyn Fn(ExtResult) + Send + Sync>;

static NEXT_LINE_ID: AtomicU64 = AtomicU64::new(1);

/// Runs `argv` with no shell and the WEB_* environment; at most `timeout` (then it is
/// killed) and `SOURCE_MAX_OUTPUT` bytes. Non-zero exit gives no lines; a program that cannot be
/// started gives the status-bar message.
pub fn run_source(argv: &[&str], env: &[(&str, String)], timeout: Duration) -> Result<Vec<String>, String> {
    let Some((prog, args)) = argv.split_first() else {
        return Ok(Vec::new());
    };
    let mut cmd = std::process::Command::new(prog);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("completion: {prog}: program not found")
        } else {
            format!("completion: {prog}: {e}")
        }
    })?;
    let mut out = child.stdout.take().expect("piped stdout");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut out).take(SOURCE_MAX_OUTPUT as u64).read_to_end(&mut buf);
        // Keep draining so the program is not blocked on a full pipe (it is killed at the timeout anyway).
        let _ = std::io::copy(&mut out, &mut std::io::sink());
        let _ = tx.send(buf);
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(Vec::new());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => return Ok(Vec::new()),
        }
    };
    if !status.success() {
        return Ok(Vec::new());
    }
    let buf = rx.recv_timeout(Duration::from_millis(500)).unwrap_or_default();
    Ok(String::from_utf8_lossy(&buf)
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// What a source is asked: the whole line, the word being completed and its argument index
/// (0 = the command name, 1 = the first argument).
pub struct CompleteCtx<'a> {
    pub line: &'a str,
    pub word: &'a str,
    pub arg: usize,
    /// What sources read once per command line (the ranked history).
    pub cache: &'a CompleteCache,
}

/// One ranked history entry: the URL as visited and where its comparison text starts (after
/// `https://`/`http://` and `www.`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub url: String,
    pub skip: usize,
}

/// Per-command-line state of the sources (QCOMP4): the ranked history is read from the file the
/// first time `:go` asks for it and kept until the line is closed (the app makes a new cache each
/// time the `:` line opens), so typing never re-reads the file.
pub struct CompleteCache {
    dir: Option<PathBuf>,
    history: OnceCell<Vec<HistoryEntry>>,
    /// C-BOOKMARKS: the bookmarks, in file order, read once per line like the history.
    bookmarks: OnceCell<Vec<HistoryEntry>>,
    /// How many times the file was read (tests check "once per line").
    pub loads: Cell<usize>,
    /// QCOMP2: names this command line, so a late result for an earlier line is dropped.
    pub id: u64,
    /// Where external sources deliver (None: they are never started).
    sink: Option<ExtSink>,
    /// WEB_URL for external sources.
    pub url: String,
    /// External sources' output per (command, argument position), once it arrived.
    ext: RefCell<HashMap<(String, usize), Vec<String>>>,
    /// (command, argument position) pairs already started: one run per line and position.
    ext_started: RefCell<HashSet<(String, usize)>>,
    /// C-SCRIPT: the user commands (`web.command`), sorted; offered after the built-in names.
    pub user_commands: Vec<String>,
}

impl CompleteCache {
    pub fn new(dir: Option<PathBuf>) -> Self {
        CompleteCache {
            dir,
            history: OnceCell::new(),
            bookmarks: OnceCell::new(),
            loads: Cell::new(0),
            id: NEXT_LINE_ID.fetch_add(1, Ordering::Relaxed),
            sink: None,
            url: String::new(),
            ext: RefCell::new(HashMap::new()),
            ext_started: RefCell::new(HashSet::new()),
            user_commands: Vec::new(),
        }
    }

    /// Lets `Source::Cmd` rows run: results go to `sink`, `url` is the current page (WEB_URL).
    pub fn with_external(mut self, sink: ExtSink, url: String) -> Self {
        self.sink = Some(sink);
        self.url = url;
        self
    }

    /// Takes an external result in; false (dropped) when it belongs to another command line.
    pub fn accept(&self, r: &ExtResult) -> bool {
        if r.line_id != self.id {
            return false;
        }
        self.ext
            .borrow_mut()
            .insert((r.cmd.clone(), r.arg), r.outcome.clone().unwrap_or_default());
        true
    }

    /// The lines of an external source: what it printed if that arrived; otherwise (first ask for
    /// this command and position) start it on a thread and answer nothing yet.
    fn external(&self, cmd: &str, argv: &'static [&'static str], ctx: &CompleteCtx) -> Vec<String> {
        let key = (cmd.to_string(), ctx.arg);
        if let Some(v) = self.ext.borrow().get(&key) {
            return v.clone();
        }
        let Some(sink) = &self.sink else { return Vec::new() };
        if self.ext_started.borrow_mut().insert(key) {
            let env = vec![
                ("WEB_LINE", ctx.line.to_string()),
                ("WEB_WORD", ctx.word.to_string()),
                ("WEB_ARG", ctx.arg.to_string()),
                ("WEB_URL", self.url.clone()),
                (
                    "WEB_STATE_DIR",
                    self.dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default(),
                ),
            ];
            let (sink, line_id, cmd, arg) = (sink.clone(), self.id, cmd.to_string(), ctx.arg);
            std::thread::spawn(move || {
                let outcome = run_source(argv, &env, Duration::from_millis(SOURCE_TIMEOUT_MS));
                sink(ExtResult {
                    line_id,
                    cmd,
                    arg,
                    outcome,
                });
            });
        }
        Vec::new()
    }

    /// The history in ghost order (see [`rank_history`]); empty without a state directory or file.
    pub fn history(&self) -> &[HistoryEntry] {
        self.history.get_or_init(|| {
            self.loads.set(self.loads.get() + 1);
            let data = self
                .dir
                .as_ref()
                .and_then(|d| std::fs::read(d.join("history")).ok())
                .unwrap_or_default();
            rank_history(&String::from_utf8_lossy(&data))
        })
    }
}

impl CompleteCache {
    /// The bookmarks (`DIR/bookmarks`) in the order they were added, as ghost entries; they come
    /// before the history in `:go`'s ghost and are `:bookmarkDelete`'s candidates.
    pub fn bookmarks(&self) -> &[HistoryEntry] {
        self.bookmarks.get_or_init(|| {
            crate::bookmarks::load(self.dir.as_deref())
                .into_iter()
                .map(|(url, _)| {
                    let skip = url.len() - strip_scheme_www(&url).len();
                    HistoryEntry { url, skip }
                })
                .collect()
        })
    }
}

/// `url` without `https://`/`http://` and then `www.`: what typed words are compared with.
pub fn strip_scheme_www(url: &str) -> &str {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
    rest.strip_prefix("www.").unwrap_or(rest)
}

/// STASH.md "Ranking": the history file's text (`ms\turl\ttitle` lines) as the ghost order: one
/// site root per origin ranked by visits to any page of it, then one entry per other URL ranked by
/// its own visits; ties go to the newest visit. Lines that do not parse (no tab, bad time, a URL
/// that is not http(s) with a host) are skipped.
pub fn rank_history(text: &str) -> Vec<HistoryEntry> {
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
pub fn complete_history_urls(ctx: &CompleteCtx) -> Vec<String> {
    // C-BOOKMARKS: bookmarks rank before history.
    match_urls(ctx.word, ctx.cache.bookmarks().iter().chain(ctx.cache.history()))
}

/// `:bookmarkDelete`'s source: the bookmarked URLs that continue the word (same matching as `:go`).
pub fn complete_bookmark_urls(ctx: &CompleteCtx) -> Vec<String> {
    match_urls(ctx.word, ctx.cache.bookmarks().iter())
}

fn match_urls<'a>(word: &str, entries: impl Iterator<Item = &'a HistoryEntry>) -> Vec<String> {
    let typed = strip_scheme_www(word);
    let mut out: Vec<String> = Vec::new();
    for e in entries {
        let cand = if e.url.starts_with(word) {
            e.url.clone()
        } else if let Some(rest) = e.url[e.skip..].strip_prefix(typed) {
            format!("{word}{rest}")
        } else {
            continue;
        };
        if !out.contains(&cand) {
            out.push(cand);
        }
    }
    out
}

/// C3-GOHTTP: the history URL (first in ghost order) that `word` equals once both lose their
/// scheme and `www.`: that entry's scheme (`http://` or `https://`) for `:go` to put before the
/// word, instead of assuming `https://`. The host is the word's own (typed `google`, visited
/// `https://www.google.com/`: `https://google.com/`, as before).
pub fn history_scheme_for(cache: &CompleteCache, word: &str) -> Option<&'static str> {
    let typed = strip_scheme_www(word);
    cache
        .bookmarks()
        .iter()
        .chain(cache.history())
        .find(|e| strip_scheme_www(&e.url) == typed)
        .map(|e| if e.url.starts_with("http://") { "http://" } else { "https://" })
}

/// A live ghost: the candidates for the word that starts at `word_start` (each one a full word
/// that starts with what was typed) and which one is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ghost {
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
    pub fn rest(&self, text: &str) -> String {
        let typed = text.get(self.word_start..).unwrap_or("");
        self.candidates[self.index].strip_prefix(typed).unwrap_or("").to_string()
    }

    /// Tab (T47): the next match, wrapping.
    pub fn next(&mut self) {
        self.index = (self.index + 1) % self.candidates.len();
    }

    /// Shift-Tab (T48): the previous match, wrapping.
    pub fn prev(&mut self) {
        self.index = (self.index + self.candidates.len() - 1) % self.candidates.len();
    }

    /// Tab with exactly one match (T53) selects it: returns the text to append to the line (the
    /// ghost, plus a space when `space_after`). With 2+ matches it cycles (the typed text does not
    /// change) and returns `None`.
    pub fn tab(&mut self, text: &str) -> Option<String> {
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
    pub fn accepted(&self, text: &str) -> String {
        format!("{text}{}", self.rest(text))
    }
}

/// The built-in source of the first word: the command names.
/// C4: an alias typed in full (`b`, `ls`, ...) comes first with nothing to add, so Enter runs it
/// as typed; the long names follow for Tab. Aliases are never offered otherwise.
pub fn complete_command_names(ctx: &CompleteCtx) -> Vec<String> {
    let alias = crate::keys::COMMAND_ALIASES.iter().find(|a| **a == ctx.word);
    alias
        .into_iter()
        .chain(crate::keys::COMMAND_NAMES.iter().filter(|n| n.starts_with(ctx.word)))
        .map(|n| n.to_string())
        .chain(ctx.cache.user_commands.iter().filter(|n| n.starts_with(ctx.word)).cloned())
        .collect()
}

/// The built-in source of `:refresh`'s argument (`config::REFRESH_ARGS`).
pub fn complete_refresh_args(ctx: &CompleteCtx) -> Vec<String> {
    crate::config::REFRESH_ARGS
        .iter()
        .filter(|a| a.starts_with(ctx.word))
        .map(|a| a.to_string())
        .collect()
}

/// The built-in source of `:set`'s argument (`config::SET_ARGS`).
pub fn complete_set_args(ctx: &CompleteCtx) -> Vec<String> {
    crate::config::SET_ARGS
        .iter()
        .filter(|a| a.starts_with(ctx.word))
        .map(|a| a.to_string())
        .collect()
}

/// T45/T46: the ghost for `text` with the cursor at byte offset `cursor`, or `None` (cursor not
/// at the end, empty word, a command without a `COMPLETERS` row, no match).
pub fn compute_ghost(text: &str, cursor: usize, cache: &CompleteCache) -> Option<Ghost> {
    compute_ghost_in(COMPLETERS, text, cursor, cache)
}

/// `compute_ghost` over an explicit completion table (the tests use their own rows).
pub fn compute_ghost_in(table: &[(&str, Source)], text: &str, cursor: usize, cache: &CompleteCache) -> Option<Ghost> {
    if cursor != text.len() {
        return None;
    }
    let word_start = crate::keys::token_start_before(text, cursor);
    let word = &text[word_start..];
    if word.is_empty() {
        return None;
    }
    let mut earlier = text[..word_start].split_whitespace();
    let command = earlier.next();
    let arg = earlier.count() + usize::from(command.is_some());
    let ctx = CompleteCtx {
        line: text,
        word,
        arg,
        cache,
    };
    let (candidates, space_after) = match command {
        None => (complete_command_names(&ctx), true),
        Some(cmd) => {
            let (_, source) = table.iter().find(|(name, _)| *name == cmd)?;
            let found = match source {
                Source::Builtin(f) => f(&ctx),
                Source::Cmd(argv) => cache.external(cmd, argv, &ctx),
            };
            (found, SPACE_AFTER_ARG.contains(&cmd))
        }
    };
    let candidates: Vec<String> = candidates.into_iter().filter(|c| c.starts_with(word)).collect();
    if candidates.is_empty() {
        return None;
    }
    Some(Ghost {
        word_start,
        candidates,
        index: 0,
        space_after,
    })
}
