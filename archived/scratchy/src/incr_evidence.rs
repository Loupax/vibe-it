//! QINC1n: engine trouble made visible in daily use (the evidence the user reads before QINC1m deletes
//! the old pipeline).
//!
//! Per session (the one engine runs on the main thread, so the counts are thread-local), for both
//! engine paths, App frames (`incr_frame::frame`) and script forced layouts (`incr_engine::update`):
//! - **updates**: the engine served the frame / forced layout from its kept trees;
//! - **restarts**: a from-scratch build started the kept trees over by design (first layout, resize,
//!   a new rule list, a global change, the relayout's drift guard). No log line;
//! - **fallbacks**: the engine produced no tree, or hit an error it recovered from, and the caller
//!   laid out from scratch. ONE diag line per page: `scratchy: engine: fallback <reason> on <url>`;
//! - **checks** / **mismatches**: sampled check mode. One engine update in
//!   [`ENGINE_CHECK_EVERY`] (0 = off) is also laid out from scratch and compared
//!   (`incr_cmp::cmp_laid`, the `SCRATCHY_INCR_CHECK` compare). A mismatch appends ONE line to
//!   `<state-dir>/engine-check.log` (mode 0600: unix ms, URL, first differing box path and field; no
//!   page text, no values) and that frame uses the from-scratch result. Without a state directory
//!   only the diag line is printed.
//!
//! `:engine` shows the counts in the status bar ([`status_text`]).
use super::*;
#[cfg(not(test))]
use crate::config::ENGINE_CHECK_EVERY;
use std::cell::{Cell, RefCell};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::Mutex;

/// The session's counts (`:engine`).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) updates: u64,
    pub(crate) restarts: u64,
    pub(crate) fallbacks: u64,
    pub(crate) checks: u64,
    pub(crate) mismatches: u64,
}

/// Longest `engine-check.log` line, newline included (one `write` on an `O_APPEND` file).
const MAX_LINE: usize = 4096;

static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

thread_local! {
    static COUNTS: Cell<Counts> = const { Cell::new(Counts { updates: 0, restarts: 0, fallbacks: 0, checks: 0, mismatches: 0 }) };
    /// Pages that printed their fallback line (weak: a dropped page frees its slot).
    static LOGGED: RefCell<Vec<WeakHandle>> = const { RefCell::new(Vec::new()) };
    /// The App's document and its URL (`set_page`).
    static PAGE: RefCell<(Option<WeakHandle>, String)> = const { RefCell::new((None, String::new())) };
    /// Engine updates since the last sampled check.
    static TICK: Cell<u64> = const { Cell::new(0) };
    #[cfg(test)]
    static SEAM: RefCell<Seam> = RefCell::new(Seam::default());
}

/// Test seams: the check interval, forced fallbacks / mismatches, a state directory, printed lines.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Seam {
    pub(crate) every: Option<u64>,
    pub(crate) fallback: bool,
    pub(crate) mismatch: bool,
    pub(crate) dir: Option<PathBuf>,
    pub(crate) lines: Vec<String>,
}

/// Test seam: runs `f` with this thread's evidence seam (counts start from zero).
#[cfg(test)]
pub(crate) fn with_seam<R>(f: impl FnOnce(&RefCell<Seam>) -> R) -> R {
    SEAM.with(f)
}

/// Test seam: resets this thread's counts, logged pages and sample tick.
#[cfg(test)]
pub(crate) fn reset() {
    COUNTS.with(|c| c.set(Counts::default()));
    LOGGED.with(|l| l.borrow_mut().clear());
    TICK.with(|t| t.set(0));
    SEAM.with(|s| *s.borrow_mut() = Seam::default());
}

/// Test hook: the next engine update falls back (`test_hook`).
#[cfg(test)]
pub(crate) fn forced_fallback() -> bool {
    SEAM.with(|s| s.borrow().fallback)
}

/// Test hook: a sampled check reports a mismatch even when the trees agree.
#[cfg(test)]
pub(crate) fn forced_mismatch() -> bool {
    SEAM.with(|s| s.borrow().mismatch)
}

/// Process start: the state directory `engine-check.log` goes to (`None`: diag line only).
pub(crate) fn init(dir: Option<PathBuf>) {
    *DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir;
}

fn state_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        SEAM.with(|s| s.borrow().dir.clone())
    }
    #[cfg(not(test))]
    {
        DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn bump(f: impl FnOnce(&mut Counts)) {
    COUNTS.with(|c| {
        let mut x = c.get();
        f(&mut x);
        c.set(x);
    });
}

pub(crate) fn counts() -> Counts {
    COUNTS.with(Cell::get)
}

/// The engine served a frame or a forced layout from its kept trees.
pub(crate) fn update() {
    bump(|c| c.updates += 1);
}

/// A from-scratch build restarted the kept trees by design (no log line).
pub(crate) fn restart() {
    bump(|c| c.restarts += 1);
}

/// The App's document and URL (called per App frame; `url` runs only for a new document).
pub(crate) fn set_page(doc: &Handle, url: impl FnOnce() -> String) {
    PAGE.with(|p| {
        let mut p = p.borrow_mut();
        if !p.0.as_ref().and_then(|w| w.upgrade()).is_some_and(|d| Rc::ptr_eq(&d, doc)) {
            *p = (Some(Rc::downgrade(doc)), url());
        }
    });
}

/// The URL of `doc` for a log line: the App's, else the page whose scripts run, else `(no url)`.
fn url_of(doc: &Handle) -> String {
    let app = PAGE.with(|p| {
        let p = p.borrow();
        p.0.as_ref().and_then(|w| w.upgrade()).is_some_and(|d| Rc::ptr_eq(&d, doc)).then(|| p.1.clone())
    });
    let u = app.filter(|u| !u.is_empty()).or_else(|| crate::js::page_url().map(String::from)).unwrap_or_else(|| "(no url)".into());
    u.chars().filter(|c| !c.is_control()).collect()
}

fn print(line: String) {
    #[cfg(test)]
    SEAM.with(|s| s.borrow_mut().lines.push(line.clone()));
    diag!("{line}");
}

/// A real fallback on `doc`: counted; the first one of a page prints its line.
pub(crate) fn fallback(doc: &Handle, reason: &str) {
    bump(|c| c.fallbacks += 1);
    let first = LOGGED.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|w| w.strong_count() > 0);
        if l.iter().any(|w| w.upgrade().is_some_and(|d| Rc::ptr_eq(&d, doc))) {
            return false;
        }
        l.push(Rc::downgrade(doc));
        true
    });
    if first {
        print(format!("scratchy: engine: fallback {reason} on {}", url_of(doc)));
    }
}

/// The check interval (`config::ENGINE_CHECK_EVERY`; tests: the seam, default off).
fn every() -> u64 {
    #[cfg(test)]
    {
        SEAM.with(|s| s.borrow().every).unwrap_or(0)
    }
    #[cfg(not(test))]
    {
        ENGINE_CHECK_EVERY
    }
}

/// One engine update that a check can compare: true when this one is sampled (counted as a check).
pub(crate) fn sample() -> bool {
    let n = every();
    if n == 0 {
        return false;
    }
    let due = TICK.with(|t| {
        let x = t.get() + 1;
        t.set(if x >= n { 0 } else { x });
        x >= n
    });
    if due {
        bump(|c| c.checks += 1);
    }
    due
}

/// Box path and field of a mismatch, as written to the log (control characters become spaces).
fn clean(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// The `engine-check.log` line: `<unix ms>\t<url>\t<box path> <field>\n`, at most [`MAX_LINE`] bytes.
pub(crate) fn check_line(ms: u64, url: &str, path: &str, field: &str) -> String {
    let mut line = format!("{ms}\t{}\t{} {}", clean(url), clean(path), clean(field));
    if line.len() + 1 > MAX_LINE {
        let mut end = MAX_LINE - 1;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
    }
    line.push('\n');
    line
}

/// A sampled check found a difference at `path` (`field`): counted, one diag line, one log line.
pub(crate) fn mismatch(doc: &Handle, path: &str, field: &str) {
    bump(|c| c.mismatches += 1);
    let url = url_of(doc);
    print(format!("scratchy: engine: check mismatch on {url} at {} ({})", clean(path), clean(field)));
    let Some(dir) = state_dir() else { return };
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let line = check_line(ms, &url, path, field);
    let r = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .and_then(|()| std::fs::OpenOptions::new().append(true).create(true).mode(0o600).open(dir.join("engine-check.log")))
        .and_then(|mut f| f.write_all(line.as_bytes()));
    if let Err(e) = r {
        diag!("scratchy: engine: writing engine-check.log failed: {e}");
    }
}

/// Compares a from-scratch layout with the engine's kept laid tree: the first difference as (box
/// path, field), `None` when they agree (`("root", "no kept laid tree")` without one).
pub(crate) fn compare(doc: &Handle, fresh: &Laid, fresh_cache: &ShapeCache) -> Option<(String, String)> {
    use crate::incr_cmp::{cmp_laid, Names};
    let bad = crate::incr_layout::with_laid(|l, c| cmp_laid(&Names::default(), "root", fresh, fresh_cache, l, c).is_err()).unwrap_or(true);
    if !bad {
        return None;
    }
    let names = Names::from_dom(doc);
    Some(
        crate::incr_layout::with_laid(|l, c| cmp_laid(&names, "root", fresh, fresh_cache, l, c).err())
            .flatten()
            .map_or_else(|| ("root".into(), "no kept laid tree".into()), |m| (m.path, m.field)),
    )
}

/// `:engine`: this session's counts, for the status bar. The message gets a third of the strip, so
/// the counts that matter come first: fallbacks, then mismatches out of checks, then updates and
/// restarts (`upd`, `rst`; help.html spells them out).
pub(crate) fn status_text() -> String {
    let c = counts();
    let check = match every() {
        0 => "checks off".to_string(),
        _ => format!("{}/{} checks bad", c.mismatches, c.checks),
    };
    format!("engine: {} fallbacks, {check}, {} upd, {} rst", c.fallbacks, c.updates, c.restarts)
}

/// `--perf-report`'s `engine_counts` object (QINC1n): the session's counts.
pub(crate) fn counts_json() -> String {
    let c = counts();
    format!(
        "{{\"updates\":{},\"restarts\":{},\"fallbacks\":{},\"checks\":{},\"mismatches\":{},\"check_every\":{}}}",
        c.updates,
        c.restarts,
        c.fallbacks,
        c.checks,
        c.mismatches,
        every()
    )
}
