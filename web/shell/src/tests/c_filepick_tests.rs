//! C-FILEPICK (KEYS.md "File chooser", T171-T182). Each test names its row.

use std::path::{Path, PathBuf};
use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::buffers::TabCreated;
use crate::filepick::*;
use crate::keys::*;
use crate::picker::PickAction;
use crate::protocol::{Accept, Command, DialogMode, Event};

/// A directory tree in a fresh place (removed on drop):
/// `a.png`, `b.txt`, `c.PDF`, `.hidden`, `sub/inner.txt`, `sub/deep/`, `zdir/`.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        let d = std::env::temp_dir().join(format!("web-c-filepick-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub/deep")).unwrap();
        std::fs::create_dir_all(d.join("zdir")).unwrap();
        for f in ["a.png", "b.txt", "c.PDF", ".hidden", "sub/inner.txt"] {
            std::fs::write(d.join(f), "12345").unwrap();
        }
        Tree(d)
    }
    fn p(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().into_owned()
    }
    fn s(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

fn type_text(k: &mut Keys, text: &str) {
    for c in text.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

fn results(k: &mut Keys) -> Vec<(u64, Vec<String>)> {
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter_map(|e| match e {
            Effect::Send(Command::FileDialogResult { id, paths }) => Some((id, paths)),
            _ => None,
        })
        .collect()
}

/// Buffer 1 on tab 1.
fn keys() -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.effects.clear();
    k
}

fn ask(k: &mut Keys, mode: DialogMode, dir: &str, accept: Vec<Accept>) {
    k.file_dialog(1, 7, mode, "", dir, accept);
}

fn labels(k: &Keys) -> Vec<String> {
    k.picker.as_ref().expect("a picker").matches().map(|r| r.label.clone()).collect()
}

fn selected(k: &Keys) -> String {
    k.picker.as_ref().unwrap().selected_row().unwrap().label.clone()
}

fn select(k: &mut Keys, label: &str) {
    for _ in 0..40 {
        press(k, KeyInput::sym(Keysym::Up));
    }
    for _ in 0..40 {
        if selected(k) == label {
            return;
        }
        press(k, KeyInput::sym(Keysym::Down));
    }
    panic!("{label} not reachable, rows {:?}", labels(k));
}

fn acc(f: &str, ext: &str) -> Accept {
    Accept {
        filter: f.into(),
        extensions: ext.into(),
        description: String::new(),
    }
}

// ---------- pure parts ----------

#[test]
fn accept_lists_match_extensions_mime_types_and_expansions() {
    assert!(accepts(&[], "x.bin"));
    assert!(accepts(&[acc(".png", "")], "A.PNG"));
    assert!(!accepts(&[acc(".png", "")], "a.jpg"));
    assert!(accepts(&[acc("image/*", "")], "a.jpeg"), "a built-in MIME table");
    assert!(!accepts(&[acc("image/*", "")], "a.txt"));
    assert!(accepts(&[acc("image/*", ".png;.xyz")], "a.xyz"), "the host's expansion wins");
    assert!(!accepts(&[acc("image/*", ".png;.xyz")], "a.jpg"));
    assert!(
        accepts(&[acc("application/x-unknown", "")], "a.whatever"),
        "an unknown type hides nothing"
    );
    assert!(accepts(&[acc(".png", ""), acc(".pdf", "")], "c.pdf"));
}

#[test]
fn sizes_and_save_names() {
    assert_eq!(human_size(5), "5 B");
    assert_eq!(human_size(1536), "1.5 KiB");
    let d = Path::new("/d");
    assert_eq!(resolve(d, "x.txt", None), PathBuf::from("/d/x.txt"));
    assert_eq!(resolve(d, "/abs/y", None), PathBuf::from("/abs/y"));
    assert_eq!(resolve(d, "~/z", Some(Path::new("/home/u"))), PathBuf::from("/home/u/z"));
}

#[test]
fn listing_is_directories_first_by_name_and_marks_show() {
    let t = Tree::new("list");
    let e = read_entries(&t.0).unwrap();
    let rows = build_rows(&t.0, &e, DialogMode::OpenMultiple, &[], false, false, &[t.p("b.txt")]);
    let l: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(
        l,
        ["[open 1 selected]", "../", "sub/", "zdir/", "a.png", "* b.txt", "c.PDF"],
        "no dotfile"
    );
    let hidden = build_rows(&t.0, &e, DialogMode::Open, &[], false, true, &[]);
    assert!(hidden.iter().any(|r| r.label == ".hidden"));
    let folders = build_rows(&t.0, &e, DialogMode::OpenFolder, &[], false, false, &[]);
    let l: Vec<&str> = folders.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(l, ["[choose this folder]", "../", "sub/", "zdir/"], "no files in open-folder");
}

// ---------- the keys ----------

#[test]
fn t171_a_chooser_opens_at_the_default_directory() {
    let t = Tree::new("t171");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert_eq!(labels(&k), ["../", "sub/", "zdir/", "a.png", "b.txt", "c.PDF"]);
    assert!(results(&mut k).is_empty(), "nothing answered yet");
    // A default that is a file: its directory; for save the name is typed.
    let mut k = keys();
    ask(&mut k, DialogMode::Save, &t.p("b.txt"), vec![]);
    assert_eq!(k.line.text, "b.txt");
    assert!(k.picker.as_ref().unwrap().name.contains("save as"));
}

#[test]
fn t172_not_the_page_on_screen_or_already_open_cancels_at_once() {
    let t = Tree::new("t172");
    let mut k = keys();
    k.file_dialog(9, 1, DialogMode::Open, "", &t.s(), vec![]);
    assert_eq!(results(&mut k), vec![(1, vec![])]);
    assert_eq!(k.vim_mode, VimMode::Normal);
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    k.file_dialog(1, 8, DialogMode::Open, "", &t.s(), vec![]);
    assert_eq!(results(&mut k), vec![(8, vec![])], "the second one is cancelled, the first stays");
    assert!(k.filedlg.is_some());
}

#[test]
fn t173_typing_filters_and_hidden_files_need_a_dot() {
    let t = Tree::new("t173");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    type_text(&mut k, "b.t");
    assert_eq!(selected(&k), "b.txt");
    for _ in 0..3 {
        press(&mut k, KeyInput::sym(Keysym::BackSpace));
    }
    type_text(&mut k, ".h");
    assert!(labels(&k).contains(&".hidden".to_string()));
}

#[test]
fn t174_enter_on_a_directory_descends_and_up_selects_where_we_were() {
    let t = Tree::new("t174");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    select(&mut k, "sub/");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(labels(&k), ["../", "deep/", "inner.txt"]);
    assert!(results(&mut k).is_empty());
    press(&mut k, KeyInput::sym(Keysym::BackSpace)); // T177
    assert_eq!(selected(&k), "sub/");
    assert!(k.filedlg.is_some());
}

#[test]
fn t175_enter_on_a_file_answers_and_closes() {
    let t = Tree::new("t175");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    select(&mut k, "b.txt");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(results(&mut k), vec![(7, vec![t.p("b.txt")])]);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.filedlg.is_none());
    // The next chooser starts where this one ended.
    ask(&mut k, DialogMode::Open, "", vec![]);
    assert_eq!(k.filedlg.as_ref().unwrap().dir, t.0);
}

#[test]
fn t175_the_accept_list_hides_files_and_alt_a_shows_them() {
    let t = Tree::new("t175a");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![acc("image/*", ""), acc(".pdf", "")]);
    assert_eq!(labels(&k), ["../", "sub/", "zdir/", "a.png", "c.PDF"]);
    let mut alt = KeyInput::ch('a');
    alt.alt = true;
    press(&mut k, alt);
    assert!(labels(&k).contains(&"b.txt".to_string()));
    press(&mut k, alt);
    assert!(!labels(&k).contains(&"b.txt".to_string()));
}

#[test]
fn t175_open_folder_answers_the_listed_directory() {
    let t = Tree::new("t175f");
    let mut k = keys();
    ask(&mut k, DialogMode::OpenFolder, &t.s(), vec![]);
    select(&mut k, "sub/");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(selected(&k), "[choose this folder]");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(results(&mut k), vec![(7, vec![t.p("sub")])]);
}

#[test]
fn t176_tab_marks_files_and_enter_answers_the_marks_plus_the_current() {
    let t = Tree::new("t176");
    let mut k = keys();
    ask(&mut k, DialogMode::OpenMultiple, &t.s(), vec![]);
    select(&mut k, "a.png");
    press(&mut k, KeyInput::sym(Keysym::Tab));
    assert_eq!(selected(&k), "b.txt", "moves down");
    assert!(labels(&k).contains(&"* a.png".to_string()));
    press(&mut k, KeyInput::sym(Keysym::Tab));
    press(&mut k, KeyInput::sym(Keysym::Return)); // on c.PDF: marks + it
    assert_eq!(results(&mut k), vec![(7, vec![t.p("a.png"), t.p("b.txt"), t.p("c.PDF")])]);
}

#[test]
fn t176_the_open_selected_row_answers_the_marks_from_a_directory_row() {
    let t = Tree::new("t176b");
    let mut k = keys();
    ask(&mut k, DialogMode::OpenMultiple, &t.s(), vec![]);
    select(&mut k, "a.png");
    press(&mut k, KeyInput::sym(Keysym::Tab));
    select(&mut k, "[open 1 selected]");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(results(&mut k), vec![(7, vec![t.p("a.png")])]);
}

#[test]
fn t176_a_plain_open_takes_one_file_whatever_tab_did() {
    let t = Tree::new("t176c");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    select(&mut k, "a.png");
    press(&mut k, KeyInput::sym(Keysym::Tab));
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(results(&mut k), vec![(7, vec![t.p("a.png")])]);
}

#[test]
fn t178_save_types_a_name_and_asks_before_overwriting() {
    let t = Tree::new("t178");
    let mut k = keys();
    ask(&mut k, DialogMode::Save, &t.s(), vec![]);
    type_text(&mut k, "new.txt");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(results(&mut k), vec![(7, vec![t.p("new.txt")])]);

    ask(&mut k, DialogMode::Save, &t.s(), vec![]);
    type_text(&mut k, "b.txt");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert!(results(&mut k).is_empty(), "exists: asks");
    assert!(k.pick_msg.contains("exists"), "{}", k.pick_msg);
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(results(&mut k), vec![(7, vec![t.p("b.txt")])]);

    // Another key forgets the question.
    ask(&mut k, DialogMode::Save, &t.s(), vec![]);
    type_text(&mut k, "b.txt");
    press(&mut k, KeyInput::sym(Keysym::Return));
    type_text(&mut k, "x");
    press(&mut k, KeyInput::sym(Keysym::BackSpace));
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert!(results(&mut k).is_empty(), "asked again");
}

#[test]
fn t178_save_enters_a_directory_by_name_and_refuses_a_missing_parent() {
    let t = Tree::new("t178b");
    let mut k = keys();
    ask(&mut k, DialogMode::Save, &t.s(), vec![]);
    type_text(&mut k, "sub");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert_eq!(k.filedlg.as_ref().unwrap().dir, t.0.join("sub"));
    assert!(results(&mut k).is_empty());
    k.line.clear();
    type_text(&mut k, "nodir/x");
    press(&mut k, KeyInput::sym(Keysym::Return));
    assert!(k.pick_msg.contains("no such directory"), "{}", k.pick_msg);
    assert!(results(&mut k).is_empty());
}

#[test]
fn t179_escape_cancels_once() {
    let t = Tree::new("t179");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    press(&mut k, KeyInput::sym(Keysym::Escape));
    assert_eq!(results(&mut k), vec![(7, vec![])]);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.filedlg.is_none() && k.picker.is_none());
    press(&mut k, KeyInput::sym(Keysym::Escape));
    assert!(results(&mut k).is_empty(), "answered once");
}

#[test]
fn t179_a_click_cancels() {
    let t = Tree::new("t179c");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    k.pick_click(false);
    assert_eq!(results(&mut k), vec![(7, vec![])]);
}

#[test]
fn t180_the_page_closing_drops_the_chooser_without_an_answer() {
    let t = Tree::new("t180");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    k.tab_closed(1, false);
    assert!(results(&mut k).is_empty(), "the host cancelled its dialog already");
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.filedlg.is_none());
}

#[test]
fn t182_moving_keys_are_the_pickers() {
    let t = Tree::new("t182");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    assert_eq!(selected(&k), "../");
    press(&mut k, KeyInput::ch('n').ctrl());
    assert_eq!(selected(&k), "sub/");
    press(&mut k, KeyInput::ch('p').ctrl());
    assert_eq!(selected(&k), "../");
}

#[test]
fn the_page_gets_no_keys_while_the_chooser_is_open() {
    let t = Tree::new("keys");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    let key = KeyInput::ch('x');
    let page = k.key(key.sym.raw(), true, &key, Instant::now());
    assert!(!page);
}

// ---------- the protocol ----------

#[test]
fn the_file_dialog_event_parses_and_the_result_command_serializes() {
    let ev = Event::parse(
        r#"{"event":"file-dialog","tab":2,"id":5,"mode":"open-multiple","title":"","default":"/home/u","accept":[{"filter":"image/*","extensions":".png;.jpg","description":"Image Files"},{"filter":".pdf"}]}"#,
    )
    .unwrap();
    let mut first = acc("image/*", ".png;.jpg");
    first.description = "Image Files".into();
    assert_eq!(
        ev,
        Event::FileDialog {
            tab: 2,
            id: 5,
            mode: DialogMode::OpenMultiple,
            title: "".into(),
            default: "/home/u".into(),
            accept: vec![first, acc(".pdf", "")],
        }
    );
    // Missing optional fields.
    assert!(matches!(
        Event::parse(r#"{"event":"file-dialog","tab":1,"id":1,"mode":"save"}"#).unwrap(),
        Event::FileDialog {
            mode: DialogMode::Save,
            ..
        }
    ));
    assert_eq!(
        Command::FileDialogResult {
            id: 5,
            paths: vec!["/a b".into()]
        }
        .line(),
        "{\"cmd\":\"file-dialog-result\",\"id\":5,\"paths\":[\"/a b\"]}\n"
    );
    assert_eq!(
        Command::FileDialogResult { id: 6, paths: vec![] }.line(),
        "{\"cmd\":\"file-dialog-result\",\"id\":6,\"paths\":[]}\n"
    );
}

#[test]
fn pick_rows_of_a_chooser_carry_paths() {
    let t = Tree::new("rows");
    let mut k = keys();
    ask(&mut k, DialogMode::Open, &t.s(), vec![]);
    select(&mut k, "b.txt");
    assert!(matches!(
        &k.picker.as_ref().unwrap().selected_row().unwrap().action,
        PickAction::FileEntry { path, dir: false } if *path == t.p("b.txt")
    ));
}
