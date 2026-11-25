//! QSTASH2: `localStorage` over stash (design in STASH.md, which wins over these comments).
//!
//! - The process (window mode, or `--js-report --state-dir DIR`) opens a [`Backend`]: the state
//!   directory plus its writer thread. Without one (tests, `--wpt`, `--perf-report`, a state dir that
//!   cannot be used) every page's localStorage stays in memory, as before.
//! - At page start (`startup_engine`, before the first script) the prelude calls `__scratchy_ls_load()`:
//!   for a page with an http(s) origin this reads the origin's pairs (shared lock) into the prelude's
//!   Storage `Map` and opens a [`Session`]. Opaque origins (file:, data:, about:) get `undefined` and
//!   keep the in-memory Storage.
//! - `setItem` / `removeItem` / `clear()` call `__scratchy_ls_op` (after the quota check in JS, so the
//!   page sees `QuotaExceededError` synchronously); the op is queued here and the engine hands the queue
//!   to the writer at the end of each task ([`LocalStorage::end_task`]), so one task's ops are one commit.
//! - Keys and values cross from V8 as UTF-16 code units ([`crate::js_engine::Utf16`]); stash encodes WTF-8.

use crate::js_engine::{native, Ctx, Utf16};
use crate::stash::{self, CommitOutcome, Op, Stash, Writer};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{mpsc, Arc};

/// The environment variable for the state directory (the `--state-dir` flag wins).
pub(crate) const STATE_DIR_ENV: &str = "SCRATCHY_STATE_DIR";

/// An open state directory and its writer thread.
pub(crate) struct Backend {
    stash: Arc<Stash>,
    writer: Writer,
}

impl Backend {
    /// Opens `dir` (creating it, mode 0700). `None` after the one fallback log line when it cannot be used.
    pub(crate) fn open(dir: &Path) -> Option<Rc<Backend>> {
        let stash = Arc::new(Stash::open_or_log(dir)?);
        match Writer::spawn(stash.clone()) {
            Ok(writer) => Some(Rc::new(Backend { stash, writer })),
            Err(e) => {
                diag!("{}", stash::fallback_line(dir, &e));
                None
            }
        }
    }

    /// Blocks until every queued commit is on disk.
    pub(crate) fn flush(&self) {
        self.writer.flush();
    }
}

thread_local! {
    /// The page thread's backend (`None`: localStorage is in memory).
    static BACKEND: RefCell<Option<Rc<Backend>>> = const { RefCell::new(None) };
}

/// Installs the backend new pages on this thread use (`None`: in memory). The old one finishes its queued
/// commits before this returns, unless a live page still holds it.
pub(crate) fn set_backend(b: Option<Rc<Backend>>) {
    let old = BACKEND.with(|c| std::mem::replace(&mut *c.borrow_mut(), b));
    if let Some(old) = old {
        old.flush();
    }
}

pub(crate) fn backend() -> Option<Rc<Backend>> {
    BACKEND.with(|c| c.borrow().clone())
}

/// Process exit: every queued commit is written before this returns.
pub(crate) fn shutdown() {
    set_backend(None);
}

/// The window's state directory: `--state-dir DIR`, else `$SCRATCHY_STATE_DIR`, else
/// `$XDG_STATE_HOME/scratchy`, else `$HOME/.local/state/scratchy`. Empty values count as unset.
/// `None` only when none of them is known (no HOME).
pub(crate) fn resolve_state_dir(flag: Option<PathBuf>, env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |k: &str| env(k).filter(|v| !v.is_empty());
    flag.filter(|p| !p.as_os_str().is_empty())
        .or_else(|| var(STATE_DIR_ENV).map(PathBuf::from))
        .or_else(|| var("XDG_STATE_HOME").map(|x| Path::new(&x).join("scratchy")))
        .or_else(|| var("HOME").map(|h| Path::new(&h).join(".local/state/scratchy")))
}

/// One page's persistent localStorage: where it lives, the generation it loaded, the ops of the current task.
struct Session {
    backend: Rc<Backend>,
    site: String,
    origin: String,
    generation: u64,
    ops: Vec<Op>,
    /// Commits handed to the writer whose outcome was not looked at yet.
    pending: Vec<mpsc::Receiver<io::Result<CommitOutcome>>>,
    /// Test seam: commits handed to the writer.
    #[cfg(test)]
    sent: usize,
}

impl Session {
    fn send(&mut self) {
        if self.ops.is_empty() {
            return;
        }
        let ops = std::mem::take(&mut self.ops);
        let rx = self.backend.writer.commit(&self.site, &self.origin, self.generation, ops);
        self.pending.push(rx);
        #[cfg(test)]
        {
            self.sent += 1;
        }
    }
}

/// The engine's localStorage state (a field of `JsEngine`). Dropping it (the page goes away) hands the
/// last ops to the writer.
#[derive(Default)]
pub(crate) struct LocalStorage(Rc<RefCell<Option<Session>>>);

impl LocalStorage {
    /// `__scratchy_ls_load()` (once, at page start) and `__scratchy_ls_op(kind, key, value)`.
    pub(crate) fn install(&self, ctx: &Ctx<'_>) {
        let s = self.0.clone();
        native!(ctx, "__scratchy_ls_load", move || -> Option<Vec<Utf16>> {
            let backend = backend()?;
            let top = crate::js::page_url()?;
            let (site, origin) = (stash::site_dir_name(&top)?, stash::origin_dir_name(&top)?);
            // The previous page's last commits (same origin, maybe) are read back by this load.
            backend.flush();
            let loaded = match backend.stash.load(&site, &origin) {
                Ok(l) => l,
                Err(e) => {
                    diag!("scratchy: stash: {}: {site}/{origin}: load failed: {e}, localStorage is in-memory for this page", backend.stash.dir().display());
                    return None;
                }
            };
            let mut flat = Vec::with_capacity(loaded.items.len() * 2);
            for (k, v) in loaded.items {
                flat.push(Utf16(k));
                flat.push(Utf16(v));
            }
            *s.borrow_mut() = Some(Session { backend, site, origin, generation: loaded.generation, ops: Vec::new(), pending: Vec::new(), #[cfg(test)] sent: 0 });
            Some(flat)
        });
        let s = self.0.clone();
        native!(ctx, "__scratchy_ls_op", move |kind: i32, key: Option<Utf16>, value: Option<Utf16>| {
            let mut s = s.borrow_mut();
            let Some(s) = s.as_mut() else { return };
            let op = match (kind, key, value) {
                (0, Some(k), Some(v)) => Op::Set(k.0, v.0),
                (1, Some(k), _) => Op::Remove(k.0),
                (2, _, _) => Op::Clear,
                _ => return,
            };
            s.ops.push(op);
        });
    }

    /// End of a task: the task's ops go to the writer as one commit. Returns true when an earlier commit
    /// found the site forgotten (`--forget-site`): the caller empties the page's copy
    /// (`__scratchy_ls_forgotten()`); the ops of this task are dropped and the session continues on the
    /// new generation.
    pub(crate) fn end_task(&self) -> bool {
        let mut g = self.0.borrow_mut();
        let Some(s) = g.as_mut() else { return false };
        let mut forgotten = false;
        s.pending.retain(|rx| match rx.try_recv() {
            Ok(Ok(CommitOutcome::Forgotten)) => {
                forgotten = true;
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            _ => false,
        });
        if forgotten {
            s.ops.clear();
            match s.backend.stash.load(&s.site, &s.origin) {
                Ok(l) => s.generation = l.generation,
                Err(_) => *g = None,
            }
            return true;
        }
        s.send();
        false
    }

    /// Test seam: `(persistent, commits handed to the writer, ops queued for the current task)`.
    #[cfg(test)]
    pub(crate) fn stats(&self) -> (bool, usize, usize) {
        self.0.borrow().as_ref().map_or((false, 0, 0), |s| (true, s.sent, s.ops.len()))
    }
}

impl Drop for LocalStorage {
    fn drop(&mut self) {
        if let Some(s) = self.0.borrow_mut().as_mut() {
            s.send();
        }
    }
}

// ---------- QSTASH4: sessionStorage ----------
//
// Kept in memory per origin for the life of the process (the window), never on disk. A page's prelude
// calls `__scratchy_ss_load()` at start (same slot as the localStorage load) to copy its origin's pairs in,
// and every setItem/removeItem/clear calls `__scratchy_ss_op`, which applies to the process copy at once
// (no writer thread, nothing to batch). Opaque origins get `undefined` and keep an in-memory area.

/// One origin's pairs: key -> (insertion sequence, value). Loaded in sequence order so `key(i)` is stable.
#[derive(Default)]
struct SessionArea {
    next: u64,
    pairs: HashMap<Vec<u16>, (u64, Vec<u16>)>,
}

thread_local! {
    /// Origin (stash::origin_dir_name) -> its sessionStorage, for this process's page thread.
    static SESSION: RefCell<HashMap<String, SessionArea>> = RefCell::new(HashMap::new());
}

/// The origin of the current page when it has one (http/https), else `None`.
fn session_origin() -> Option<String> {
    stash::origin_dir_name(&crate::js::page_url()?)
}

/// Installs `__scratchy_ss_load()` and `__scratchy_ss_op(kind, key, value)` (kinds as `__scratchy_ls_op`).
pub(crate) fn install_session(ctx: &Ctx<'_>) {
    // The origin is fixed per page: looked up once at load, reused by the ops.
    let origin: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let o = origin.clone();
    native!(ctx, "__scratchy_ss_load", move || -> Option<Vec<Utf16>> {
        let key = session_origin()?;
        let flat = SESSION.with(|m| {
            let mut m = m.borrow_mut();
            let area = m.entry(key.clone()).or_default();
            let mut v: Vec<(u64, &Vec<u16>, &Vec<u16>)> = area.pairs.iter().map(|(k, (n, val))| (*n, k, val)).collect();
            v.sort_by_key(|t| t.0);
            v.into_iter().flat_map(|(_, k, val)| [Utf16(k.clone()), Utf16(val.clone())]).collect::<Vec<_>>()
        });
        *o.borrow_mut() = Some(key);
        Some(flat)
    });
    native!(ctx, "__scratchy_ss_op", move |kind: i32, key: Option<Utf16>, value: Option<Utf16>| {
        let origin = origin.borrow();
        let Some(origin) = origin.as_ref() else { return };
        SESSION.with(|m| {
            let mut m = m.borrow_mut();
            let area = m.entry(origin.clone()).or_default();
            match (kind, key, value) {
                (0, Some(k), Some(v)) => {
                    let n = area.next;
                    match area.pairs.get_mut(&k.0) {
                        Some(e) => e.1 = v.0,
                        None => {
                            area.pairs.insert(k.0, (n, v.0));
                            area.next += 1;
                        }
                    }
                }
                (1, Some(k), _) => {
                    area.pairs.remove(&k.0);
                }
                (2, _, _) => area.pairs.clear(),
                _ => {}
            }
        });
    });
}
