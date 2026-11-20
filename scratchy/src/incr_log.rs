//! QINC1e (R60 section 5): the write log of the incremental style + layout engine.
//!
//! Every DOM-writing native of `js_dom.rs` that changed a value of a node connected to the page
//! (a no-op write, QY53m, and a write to a detached subtree log nothing) calls [`note`] with the
//! node and a [`Change`]. The log is bounded: past [`CAP`] entries the rest are dropped and the
//! log reports [`Drained::global`] (= everything dirty, today's behaviour). Nothing reads the log
//! yet except the per-layout counters below (`--perf-report` member `incr_log`); QINC1f consumes it.
//! A node is identified by its `Rc` address (`key`), which is stable for the node's lifetime.
use markup5ever_rcdom::Handle;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Entries kept between two layouts; one more makes the log Global.
pub(crate) const CAP: usize = 4096;
/// Attribute values longer than this are not copied ([`Val::Big`]).
const VAL_MAX: usize = 512;

/// An attribute value as the log keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Val {
    Absent,
    Set(Box<str>),
    /// Present but too long to copy; a consumer treats it as "some other value".
    Big,
}

impl Val {
    pub(crate) fn of(v: Option<&str>) -> Val {
        match v {
            None => Val::Absent,
            Some(s) if s.len() > VAL_MAX => Val::Big,
            Some(s) => Val::Set(s.into()),
        }
    }
}

/// Which form state a native changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormKind {
    Checked,
    Indeterminate,
    Reset,
    SelectedIndex,
    OptionSelected,
    Value,
    Selection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    /// Attribute `name` of the node: old and new value.
    Attr { name: Box<str>, old: Val, new: Val },
    /// `child` was inserted under the node (the parent); `prev`/`next` are its siblings afterwards.
    ChildInsert { child: usize, prev: Option<usize>, next: Option<usize> },
    /// `child` left the node (the parent); `prev`/`next` are its siblings before.
    ChildRemove { child: usize, prev: Option<usize>, next: Option<usize> },
    /// The data of a text node (the node).
    Text,
    /// `textContent` of an element (the node) replaced its children.
    TextContent,
    /// `innerHTML` of an element (the node) replaced its children.
    InnerHtml,
    /// `document.write` markup was inserted after the node.
    Write,
    /// Form state of a control.
    Form(FormKind),
    /// `<style>` sheet text override of the node (CSSOM rule edits).
    SheetText,
    /// Adopted stylesheets changed (no node).
    Adopted,
    /// The external / adopted CSS version moved (no node).
    CssVer,
    /// A write the log cannot describe.
    Global,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) node: usize,
    pub(crate) change: Change,
}

#[derive(Default)]
struct Log {
    entries: Vec<Entry>,
    overflow: bool,
}

/// What [`drain`] returns.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Drained {
    pub(crate) entries: Vec<Entry>,
    /// The log overflowed or held a `Global` entry: treat everything as dirty.
    pub(crate) global: bool,
}

thread_local! {
    static LOG: RefCell<Log> = RefCell::new(Log::default());
}

static LAYOUTS: AtomicU64 = AtomicU64::new(0);
static ENTRIES: AtomicU64 = AtomicU64::new(0);
static MAX_PER_LAYOUT: AtomicU64 = AtomicU64::new(0);
static GLOBALS: AtomicU64 = AtomicU64::new(0);
static OVERFLOWS: AtomicU64 = AtomicU64::new(0);

/// The identity of a node in the log.
pub(crate) fn key(h: &Handle) -> usize {
    Rc::as_ptr(h) as usize
}

/// Logs one change of `node` (use `key`).
pub(crate) fn note(node: usize, change: Change) {
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        if l.overflow {
            return;
        }
        if l.entries.len() >= CAP {
            l.overflow = true;
            OVERFLOWS.fetch_add(1, Relaxed);
            return;
        }
        l.entries.push(Entry { node, change });
    });
}

/// The siblings of `h` under its parent, as keys.
pub(crate) fn siblings(h: &Handle) -> (Option<usize>, Option<usize>) {
    let Some(p) = crate::dom::parent_handle(h) else { return (None, None) };
    let kids = p.children.borrow();
    // Appends are the common case: look at the end first.
    let at = if kids.last().is_some_and(|k| Rc::ptr_eq(k, h)) { Some(kids.len() - 1) } else { kids.iter().position(|k| Rc::ptr_eq(k, h)) };
    match at {
        Some(i) => (i.checked_sub(1).map(|j| key(&kids[j])), kids.get(i + 1).map(key)),
        None => (None, None),
    }
}

/// Takes the log (and resets it).
pub(crate) fn drain() -> Drained {
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        let entries = std::mem::take(&mut l.entries);
        let global = std::mem::take(&mut l.overflow) || entries.iter().any(|e| e.change == Change::Global);
        Drained { entries, global }
    })
}

/// A layout (forced, on the JS thread) is starting from the page as the log left it: count what
/// accumulated since the last one, then reset. Nothing consumes the entries yet.
pub(crate) fn layout_boundary() {
    let d = drain();
    let n = d.entries.len() as u64;
    LAYOUTS.fetch_add(1, Relaxed);
    ENTRIES.fetch_add(n, Relaxed);
    MAX_PER_LAYOUT.fetch_max(n, Relaxed);
    if d.global {
        GLOBALS.fetch_add(1, Relaxed);
    }
}

/// The `incr.log` object of the `--perf-report` body (a JSON object text).
pub(crate) fn stats_json() -> String {
    format!(
        "{{\"layouts\":{},\"entries\":{},\"max_per_layout\":{},\"global\":{},\"overflows\":{}}}",
        LAYOUTS.load(Relaxed),
        ENTRIES.load(Relaxed),
        MAX_PER_LAYOUT.load(Relaxed),
        GLOBALS.load(Relaxed),
        OVERFLOWS.load(Relaxed)
    )
}
