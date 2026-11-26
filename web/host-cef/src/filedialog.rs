//! C-FILEPICK: file choosers (`<input type=file>`, saves) are answered by the shell's own picker.
//!
//! Without a `DialogHandler` Chromium opens its stock Linux dialog (GTK, or a portal), which
//! needs desktop packages and on some systems never returns the file to the page. This handler
//! takes every chooser (`on_file_dialog` returns true), numbers it, tells the shell (`file-dialog`)
//! and keeps CEF's callback until the shell answers (`file-dialog-result`; `answer`). The callback
//! is always completed: the shell's answer, a closed page (`page_closed`) and the shell going away
//! (`cancel_all`) all end it; a dialog with no shell connected is cancelled at once.

use cef::*;
use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::control::{connected, emit};
use crate::protocol::{accept_list, DialogMode, Event, TabId};

/// The open dialogs, by id, and the next id. Generic over the callback so it is tested without CEF.
pub struct Registry<C> {
    next: u64,
    open: BTreeMap<u64, (TabId, C)>,
}

impl<C> Default for Registry<C> {
    fn default() -> Self {
        Registry {
            next: 1,
            open: BTreeMap::new(),
        }
    }
}

impl<C> Registry<C> {
    /// Registers a dialog of page `tab`; its id.
    pub fn add(&mut self, tab: TabId, cb: C) -> u64 {
        let id = self.next;
        self.next += 1;
        self.open.insert(id, (tab, cb));
        id
    }

    /// The dialog `id`'s callback, once: None when it is unknown or already ended.
    pub fn take(&mut self, id: u64) -> Option<C> {
        self.open.remove(&id).map(|(_, c)| c)
    }

    /// Every dialog of page `tab`, ended.
    pub fn take_tab(&mut self, tab: TabId) -> Vec<(u64, C)> {
        let ids: Vec<u64> = self.open.iter().filter(|(_, (t, _))| *t == tab).map(|(i, _)| *i).collect();
        ids.into_iter().filter_map(|i| self.open.remove(&i).map(|(_, c)| (i, c))).collect()
    }

    /// Every dialog, ended.
    pub fn take_all(&mut self) -> Vec<(u64, C)> {
        std::mem::take(&mut self.open).into_iter().map(|(i, (_, c))| (i, c)).collect()
    }

    pub fn len(&self) -> usize {
        self.open.len()
    }
}

thread_local! {
    static OPEN: RefCell<Registry<FileDialogCallback>> = RefCell::new(Registry::default());
}

fn strings(l: Option<&mut CefStringList>) -> Vec<String> {
    l.map(|l| l.clone().into_iter().collect()).unwrap_or_default()
}

fn finish(cb: FileDialogCallback, paths: &[String]) {
    if paths.is_empty() {
        cb.cancel();
    } else {
        let mut l = CefStringList::new();
        for p in paths {
            l.append(p);
        }
        cb.cont(Some(&mut l));
    }
}

/// The shell's `file-dialog-result`.
pub fn answer(id: u64, paths: &[String]) {
    match OPEN.with(|o| o.borrow_mut().take(id)) {
        Some(cb) => {
            log!("file dialog {id}: {} path(s) chosen", paths.len());
            finish(cb, paths);
        }
        None => log!("file dialog {id}: answer ignored (unknown or already ended)"),
    }
}

/// Page `tab` closed: its dialogs end as cancelled (the shell learns it from `tab-closed`).
pub fn page_closed(tab: TabId) {
    for (id, cb) in OPEN.with(|o| o.borrow_mut().take_tab(tab)) {
        log!("file dialog {id}: page {tab} closed, cancelled");
        cb.cancel();
    }
}

/// The shell is gone (or the host quits): every dialog ends as cancelled.
pub fn cancel_all() {
    for (id, cb) in OPEN.with(|o| o.borrow_mut().take_all()) {
        log!("file dialog {id}: shell gone, cancelled");
        cb.cancel();
    }
}

fn mode_of(m: FileDialogMode) -> DialogMode {
    match sys::cef_file_dialog_mode_t::from(m) {
        sys::cef_file_dialog_mode_t::FILE_DIALOG_OPEN_MULTIPLE => DialogMode::OpenMultiple,
        sys::cef_file_dialog_mode_t::FILE_DIALOG_OPEN_FOLDER => DialogMode::OpenFolder,
        sys::cef_file_dialog_mode_t::FILE_DIALOG_SAVE => DialogMode::Save,
        _ => DialogMode::Open,
    }
}

wrap_dialog_handler! {
    pub struct HostDialogHandler {}

    impl DialogHandler {
        fn on_file_dialog(
            &self,
            browser: Option<&mut Browser>,
            mode: FileDialogMode,
            title: Option<&CefString>,
            default_file_path: Option<&CefString>,
            accept_filters: Option<&mut CefStringList>,
            accept_extensions: Option<&mut CefStringList>,
            accept_descriptions: Option<&mut CefStringList>,
            callback: Option<&mut FileDialogCallback>,
        ) -> i32 {
            let (Some(browser), Some(callback)) = (browser, callback) else { return 0 };
            let tab = browser.identifier();
            let mode = mode_of(mode);
            log!("on_file_dialog called: tab {tab} mode {}", crate::protocol::dialog_mode_name(mode));
            if !connected() {
                // Standalone (no shell): nobody can choose.
                callback.cancel();
                return 1;
            }
            let accept = accept_list(&strings(accept_filters), &strings(accept_extensions), &strings(accept_descriptions));
            let id = OPEN.with(|o| o.borrow_mut().add(tab, callback.clone()));
            emit(Event::FileDialog {
                tab,
                id,
                mode,
                title: title.map(|t| t.to_string()).unwrap_or_default(),
                default: default_file_path.map(|t| t.to_string()).unwrap_or_default(),
                accept,
            });
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_count_up_and_an_answer_ends_a_dialog_once() {
        let mut r: Registry<&str> = Registry::default();
        let a = r.add(1, "a");
        let b = r.add(2, "b");
        assert_eq!((a, b), (1, 2));
        assert_eq!(r.take(a), Some("a"));
        assert_eq!(r.take(a), None, "a second answer finds nothing");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn a_closed_page_ends_only_its_dialogs() {
        let mut r: Registry<&str> = Registry::default();
        r.add(1, "a");
        r.add(2, "b");
        r.add(1, "c");
        let gone = r.take_tab(1);
        assert_eq!(gone, vec![(1, "a"), (3, "c")]);
        assert_eq!(r.len(), 1);
        assert_eq!(r.take(2), Some("b"));
    }

    #[test]
    fn a_gone_shell_ends_every_dialog() {
        let mut r: Registry<&str> = Registry::default();
        r.add(1, "a");
        r.add(2, "b");
        assert_eq!(r.take_all().len(), 2);
        assert_eq!(r.len(), 0);
        assert!(r.take_all().is_empty());
    }
}
