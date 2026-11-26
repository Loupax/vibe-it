//! C-FILEPICK: the shell's own file chooser (KEYS.md "File chooser", T171-T182), shown when a page
//! asks for a file (`<input type=file>`, a save): the engine host sends `file-dialog`, the shell
//! answers `file-dialog-result` (PROTOCOL.md "File choosers"). No portal, GTK or desktop package.
//!
//! It is the C-PICK picker over a directory's entries (`picker.rs`: the list, the fuzzy filter, the
//! keys that move the selection, the drawing), in its own state (`FileDlg`): Enter on a directory
//! descends, on a file chooses it; Tab marks files (open-multiple); Backspace on an empty filter
//! goes up. Save modes the line as the file name. Esc, a click, or the page closing cancels.
//! Whatever ends the picker answers the host exactly once (`Keys::close_line` sends the cancel).
//!
//! Pure parts (`accepts`, `build_rows`, `resolve`, `human_size`) are tested in
//! `tests/c_filepick_tests.rs`.

use std::path::{Path, PathBuf};

use smithay::input::keyboard::Keysym as K;

use crate::keys::{Effect, KeyInput, Keys, VimMode};
use crate::picker::{PickAction, Picker, Row};
use crate::protocol::{Accept, Command, DialogMode};

/// One directory entry, as `read_entries` found it (symlinks followed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

/// The open chooser (`Keys::filedlg`).
#[derive(Debug, Clone)]
pub struct FileDlg {
    /// The host's dialog number (`file-dialog-result`'s `id`).
    pub id: u64,
    /// The page that asked.
    pub tab: u64,
    pub mode: DialogMode,
    /// What the header says (the page's title for the dialog, else the mode).
    pub label: String,
    /// The directory listed.
    pub dir: PathBuf,
    entries: Vec<Entry>,
    pub accept: Vec<Accept>,
    /// `Alt-a`: show every file, whatever the page's accept list says.
    pub all_files: bool,
    /// Dotfiles are listed while the filter starts with `.`.
    hidden: bool,
    /// open-multiple: the files marked with Tab, full paths, in marking order.
    pub marked: Vec<String>,
    /// save: the file that exists and was asked to be saved over once; Enter again confirms.
    overwrite: Option<PathBuf>,
}

// ---------- pure parts ----------

/// The extensions a MIME filter stands for when the host did not expand it (`image/*`). None: unknown.
fn builtin_exts(filter: &str) -> Option<&'static [&'static str]> {
    Some(match filter {
        "image/*" => &[
            ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".avif", ".ico", ".tif", ".tiff",
        ],
        "audio/*" => &[".mp3", ".wav", ".ogg", ".oga", ".flac", ".m4a", ".aac", ".opus"],
        "video/*" => &[".mp4", ".webm", ".mkv", ".mov", ".avi", ".ogv", ".m4v"],
        "text/*" => &[".txt", ".md", ".csv", ".html", ".htm", ".css", ".js", ".json", ".xml", ".log"],
        "application/pdf" => &[".pdf"],
        "application/json" => &[".json"],
        "application/zip" => &[".zip"],
        _ => return None,
    })
}

fn accept_matches(a: &Accept, lower: &str) -> bool {
    let f = a.filter.trim().to_lowercase();
    if f.starts_with('.') {
        return lower.ends_with(&f);
    }
    let exts: Vec<String> = a
        .extensions
        .split(';')
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    if !exts.is_empty() {
        return exts.iter().any(|e| lower.ends_with(e.as_str()));
    }
    match builtin_exts(&f) {
        Some(list) => list.iter().any(|e| lower.ends_with(e)),
        // A type nobody here knows the extensions of: cannot tell, so do not hide anything.
        None => true,
    }
}

/// Whether a file named `name` passes the page's accept list (an empty list passes everything).
pub fn accepts(accept: &[Accept], name: &str) -> bool {
    if accept.is_empty() {
        return true;
    }
    let lower = name.to_lowercase();
    accept.iter().any(|a| accept_matches(a, &lower))
}

/// `1.5 KiB`-style size for the detail column.
pub fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// The directory's entries, directories first, each group by lower-cased name. Entries whose
/// metadata cannot be read are left out.
pub fn read_entries(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let Ok(e) = e else { continue };
        let Ok(md) = std::fs::metadata(e.path()) else { continue };
        out.push(Entry {
            name: e.file_name().to_string_lossy().into_owned(),
            dir: md.is_dir(),
            size: md.len(),
        });
    }
    out.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

/// What a typed save name means: `~/x` under the home directory, an absolute path as it is, else
/// under `dir`.
pub fn resolve(dir: &Path, typed: &str, home: Option<&Path>) -> PathBuf {
    if let Some(rest) = typed.strip_prefix("~/") {
        if let Some(h) = home {
            return h.join(rest);
        }
    }
    if typed == "~" {
        if let Some(h) = home {
            return h.to_path_buf();
        }
    }
    let p = Path::new(typed);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        dir.join(p)
    }
}

fn full(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

/// The rows of a directory listing, in order: for open-multiple with files marked a "finish" row;
/// for open-folder "choose this folder"; `../`; the directories; then (not for open-folder) the
/// files that pass the accept list (all of them with `all_files`), marked ones prefixed `* `.
/// Dotfiles only with `hidden`.
#[allow(clippy::too_many_arguments)]
pub fn build_rows(
    dir: &Path,
    entries: &[Entry],
    mode: DialogMode,
    accept: &[Accept],
    all_files: bool,
    hidden: bool,
    marked: &[String],
) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut add = |label: String, detail: String, field: &str, action: PickAction| {
        rows.push(Row {
            label,
            detail,
            fields: vec![field.to_string()],
            action,
        })
    };
    if mode == DialogMode::OpenMultiple && !marked.is_empty() {
        add(
            format!("[open {} selected]", marked.len()),
            String::new(),
            "open selected",
            PickAction::FileHere(String::new()),
        );
    }
    if mode == DialogMode::OpenFolder {
        add(
            "[choose this folder]".into(),
            dir.to_string_lossy().into_owned(),
            "choose this folder",
            PickAction::FileHere(dir.to_string_lossy().into_owned()),
        );
    }
    if let Some(parent) = dir.parent() {
        add(
            "../".into(),
            String::new(),
            "..",
            PickAction::FileEntry {
                path: parent.to_string_lossy().into_owned(),
                dir: true,
            },
        );
    }
    for e in entries.iter().filter(|e| e.dir && (hidden || !e.name.starts_with('.'))) {
        add(
            format!("{}/", e.name),
            String::new(),
            &e.name,
            PickAction::FileEntry {
                path: full(dir, &e.name),
                dir: true,
            },
        );
    }
    if mode != DialogMode::OpenFolder {
        for e in entries.iter().filter(|e| !e.dir && (hidden || !e.name.starts_with('.'))) {
            if mode != DialogMode::Save && !all_files && !accepts(accept, &e.name) {
                continue;
            }
            let path = full(dir, &e.name);
            let mark = if marked.contains(&path) { "* " } else { "" };
            add(
                format!("{mark}{}", e.name),
                human_size(e.size),
                &e.name,
                PickAction::FileEntry { path, dir: false },
            );
        }
    }
    rows
}

fn label_of(mode: DialogMode, title: &str) -> String {
    if !title.trim().is_empty() {
        return title.trim().to_string();
    }
    match mode {
        DialogMode::Open => "open file",
        DialogMode::OpenMultiple => "open files",
        DialogMode::OpenFolder => "open folder",
        DialogMode::Save => "save as",
    }
    .to_string()
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_dir())
}

/// `/home/u/src` as `~/src` for the header.
fn tilde(dir: &Path) -> String {
    match home().and_then(|h| dir.strip_prefix(&h).ok().map(|r| r.to_path_buf())) {
        Some(r) if r.as_os_str().is_empty() => "~".into(),
        Some(r) => format!("~/{}", r.display()),
        None => dir.display().to_string(),
    }
}

/// Where a dialog starts: the page's default path (a directory, or a file in one), else the
/// directory last used, else the home directory, else `/`. The second value is the default file
/// name (save).
pub fn start_dir(default: &str, last: Option<&Path>) -> (PathBuf, String) {
    let d = default.trim();
    if !d.is_empty() {
        let p = Path::new(d);
        if p.is_dir() {
            return (p.to_path_buf(), String::new());
        }
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(parent) = p.parent().filter(|q| q.is_dir()) {
            return (parent.to_path_buf(), name);
        }
        return (last.map(Path::to_path_buf).or_else(home).unwrap_or_else(|| "/".into()), name);
    }
    (
        last.map(Path::to_path_buf)
            .filter(|l| l.is_dir())
            .or_else(home)
            .unwrap_or_else(|| "/".into()),
        String::new(),
    )
}

// ---------- the chooser on the keys ----------

impl Keys {
    /// The host's `file-dialog` for page `tab` (PROTOCOL.md "File choosers"): opens the chooser, or
    /// answers "cancelled" at once when it cannot (the page is not the one on screen, a chooser is
    /// already open, the directory cannot be listed).
    pub fn file_dialog(&mut self, tab: u64, id: u64, mode: DialogMode, title: &str, default: &str, accept: Vec<Accept>) {
        let cancel = |k: &mut Keys, why: &str| {
            k.effects.push(Effect::Log(format!("file-dialog {id}: cancelled, {why}")));
            k.effects.push(Effect::Send(Command::FileDialogResult { id, paths: vec![] }));
        };
        if self.buffers.current().tab != Some(tab) {
            return cancel(self, "not the page on screen");
        }
        if self.filedlg.is_some() {
            return cancel(self, "a chooser is already open");
        }
        let (mut dir, name) = start_dir(default, self.file_dir.as_deref());
        let mut entries = read_entries(&dir);
        if entries.is_err() {
            // The page's default is unreadable: the home directory, then `/`.
            for d in [home(), Some(PathBuf::from("/"))].into_iter().flatten() {
                if let Ok(e) = read_entries(&d) {
                    dir = d;
                    entries = Ok(e);
                    break;
                }
            }
        }
        let Ok(entries) = entries else {
            return cancel(self, "no directory can be listed");
        };
        self.set_key_mode(crate::keys::KeyMode::Normal);
        self.close_line();
        let dlg = FileDlg {
            id,
            tab,
            mode,
            label: label_of(mode, title),
            dir,
            entries,
            accept,
            all_files: false,
            hidden: false,
            marked: Vec::new(),
            overwrite: None,
        };
        self.filedlg = Some(dlg);
        self.file_show(false);
        self.line.clear();
        if mode == DialogMode::Save {
            self.line.insert_str(&name);
        }
        self.vim_mode = VimMode::Pick;
        self.pick_msg.clear();
        self.effects.push(Effect::Log(format!("file-dialog {id}: open, tab {tab}")));
    }

    /// A new picker over the dialog's directory (selection at the top, or on the entry named `keep`).
    fn file_show(&mut self, keep_selection: bool) {
        let Some(d) = &self.filedlg else { return };
        let rows = build_rows(&d.dir, &d.entries, d.mode, &d.accept, d.all_files, d.hidden, &d.marked);
        let name = format!("{} {}", d.label, tilde(&d.dir));
        if keep_selection {
            let filter = self.line.text.clone();
            if let Some(p) = &mut self.picker {
                p.name = name;
                p.replace_rows(rows, if d.mode == DialogMode::Save { "" } else { &filter });
                return;
            }
        }
        self.ghost = None;
        self.picker = Some(Picker::new(&name, rows));
    }

    /// Lists `dir` (the chooser stays where it was when it cannot be read). `from`: the directory
    /// left by going up, which is selected afterwards.
    fn file_cd(&mut self, dir: PathBuf) {
        let entries = match read_entries(&dir) {
            Ok(e) => e,
            Err(e) => {
                self.message = format!("web: cannot read {}: {e}", dir.display());
                return;
            }
        };
        let from = self.filedlg.as_ref().map(|d| d.dir.clone());
        let Some(d) = &mut self.filedlg else { return };
        d.dir = dir.clone();
        d.entries = entries;
        d.hidden = false;
        d.overwrite = None;
        if d.mode != DialogMode::Save {
            self.line.clear();
        }
        self.file_show(false);
        if let (Some(from), Some(p)) = (from, &mut self.picker) {
            if from.parent() == Some(dir.as_path()) {
                let at = p.rows_position(|r| matches!(&r.action, PickAction::FileEntry { path, .. } if Path::new(path) == from));
                if let Some(at) = at {
                    p.move_by(at as isize);
                }
            }
        }
    }

    /// The answer: `paths` chosen. The chooser closes; the host gets the result once.
    fn file_finish(&mut self, paths: Vec<String>) {
        if let Some(d) = self.filedlg.take() {
            self.file_dir = Some(d.dir.clone());
            self.effects
                .push(Effect::Log(format!("file-dialog {}: chose {} path(s)", d.id, paths.len())));
            self.effects.push(Effect::Send(Command::FileDialogResult { id: d.id, paths }));
        }
        self.close_line();
    }

    /// The page `tab` closed: its chooser goes with it (the host has already cancelled the dialog).
    pub fn file_dialog_tab_closed(&mut self, tab: u64) {
        if self.filedlg.as_ref().is_some_and(|d| d.tab == tab) {
            self.filedlg = None;
            self.close_line();
        }
    }

    /// `close_line` ended the chooser without an answer (Esc, a click, ...): cancel it at the host.
    pub(crate) fn file_dialog_cancelled(&mut self) {
        if let Some(d) = self.filedlg.take() {
            self.effects.push(Effect::Log(format!("file-dialog {}: cancelled", d.id)));
            self.effects
                .push(Effect::Send(Command::FileDialogResult { id: d.id, paths: vec![] }));
        }
    }

    /// A key in PICK while a chooser is open (T171-T182). Returns whether it was taken here; the
    /// rest (moving the selection, Esc, the clicks) is the picker's own.
    pub(crate) fn file_key(&mut self, key: &KeyInput) -> bool {
        let Some(d) = self.filedlg.as_mut() else { return false };
        let overwrite = d.overwrite.take();
        let mode = d.mode;
        let s = key.sym;
        let plain = !key.ctrl && !key.alt && !key.logo;
        let nav = matches!(
            s,
            K::Escape
                | K::Down
                | K::KP_Down
                | K::Up
                | K::KP_Up
                | K::Page_Down
                | K::KP_Page_Down
                | K::Page_Up
                | K::KP_Page_Up
                | K::Right
                | K::End
                | K::KP_Right
                | K::KP_End
                | K::ISO_Left_Tab
                | K::Delete
                | K::KP_Delete
        ) || (key.ctrl && matches!(key.ch, Some('n') | Some('p') | Some('d')));
        if nav {
            return false;
        }
        self.pick_msg.clear();
        if s == K::Return || s == K::KP_Enter {
            self.file_enter(overwrite);
        } else if s == K::Tab && plain {
            self.file_tab();
        } else if key.alt && !key.ctrl && key.ch == Some('a') {
            // T181: the page's accept list on/off.
            if let Some(d) = &mut self.filedlg {
                d.all_files = !d.all_files;
                self.message = if d.all_files {
                    "showing all files".into()
                } else {
                    "showing the accepted files".into()
                };
            }
            self.file_show(true);
        } else if s == K::BackSpace && plain && self.line.text.is_empty() && mode != DialogMode::Save {
            // T177: up one directory.
            if let Some(parent) = self.filedlg.as_ref().and_then(|d| d.dir.parent().map(Path::to_path_buf)) {
                self.file_cd(parent);
            }
        } else {
            let before = self.line.text.clone();
            self.line_edit_key(key);
            if self.line.text != before && mode != DialogMode::Save {
                let filter = self.line.text.clone();
                let hidden = filter.starts_with('.');
                if let Some(d) = &mut self.filedlg {
                    if d.hidden != hidden {
                        d.hidden = hidden;
                        let rows = build_rows(&d.dir, &d.entries, d.mode, &d.accept, d.all_files, d.hidden, &d.marked);
                        if let Some(p) = &mut self.picker {
                            p.replace_rows(rows, &filter);
                        }
                    }
                }
                if let Some(p) = &mut self.picker {
                    p.set_filter(&filter);
                }
            }
        }
        true
    }

    /// Tab: open-multiple marks (or unmarks) the selected file and moves down; save descends into
    /// the selected directory.
    fn file_tab(&mut self) {
        let Some(d) = &self.filedlg else { return };
        let mode = d.mode;
        let action = self.picker.as_ref().and_then(|p| p.selected_row()).map(|r| r.action.clone());
        match (mode, action) {
            (DialogMode::OpenMultiple, Some(PickAction::FileEntry { path, dir: false })) => {
                let marked_path = path.clone();
                if let Some(d) = &mut self.filedlg {
                    match d.marked.iter().position(|m| *m == path) {
                        Some(i) => {
                            d.marked.remove(i);
                        }
                        None => d.marked.push(path),
                    }
                }
                let filter = self.line.text.clone();
                if let Some(d) = &self.filedlg {
                    let rows = build_rows(&d.dir, &d.entries, d.mode, &d.accept, d.all_files, d.hidden, &d.marked);
                    if let Some(p) = &mut self.picker {
                        p.replace_rows(rows, &filter);
                        // The rows above may have grown by the "open N selected" row: start from the
                        // marked file's own row.
                        if let Some(at) =
                            p.rows_position(|r| matches!(&r.action, PickAction::FileEntry { path: q, .. } if *q == marked_path))
                        {
                            p.selected = at;
                        }
                        p.move_by(1);
                    }
                }
            }
            (DialogMode::Save, Some(PickAction::FileEntry { path, dir: true })) => self.file_cd(PathBuf::from(path)),
            _ => {}
        }
    }

    /// Enter (T172-T176).
    fn file_enter(&mut self, overwrite: Option<PathBuf>) {
        let Some(d) = &self.filedlg else { return };
        let (mode, dir) = (d.mode, d.dir.clone());
        let marked = d.marked.clone();
        let action = self.picker.as_ref().and_then(|p| p.selected_row()).map(|r| r.action.clone());
        if mode == DialogMode::Save {
            let typed = self.line.text.trim().to_string();
            let target = if !typed.is_empty() {
                resolve(&dir, &typed, home().as_deref())
            } else {
                match action {
                    Some(PickAction::FileEntry { path, .. }) => PathBuf::from(path),
                    _ => {
                        self.message = "web: type a file name".into();
                        return;
                    }
                }
            };
            if target.is_dir() {
                return self.file_cd(target);
            }
            if target.parent().is_some_and(|p| !p.as_os_str().is_empty() && !p.is_dir()) {
                self.message = format!(
                    "web: no such directory: {}",
                    target.parent().map(|p| p.display().to_string()).unwrap_or_default()
                );
                return;
            }
            if target.exists() && overwrite.as_deref() != Some(target.as_path()) {
                self.message = format!("{} exists: Enter again to overwrite", target.display());
                if let Some(d) = &mut self.filedlg {
                    d.overwrite = Some(target);
                }
                return;
            }
            return self.file_finish(vec![target.to_string_lossy().into_owned()]);
        }
        match action {
            None => self.message = "web: nothing matches".into(),
            Some(PickAction::FileHere(p)) if p.is_empty() => self.file_finish(marked),
            Some(PickAction::FileHere(p)) => self.file_finish(vec![p]),
            Some(PickAction::FileEntry { path, dir: true }) => self.file_cd(PathBuf::from(path)),
            Some(PickAction::FileEntry { path, dir: false }) => {
                let mut paths = marked;
                if mode != DialogMode::OpenMultiple {
                    paths.clear();
                }
                if !paths.contains(&path) {
                    paths.push(path);
                }
                self.file_finish(paths);
            }
            Some(_) => {}
        }
    }
}
