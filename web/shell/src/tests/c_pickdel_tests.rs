//! C-PICKDEL (KEYS.md "Deleting from a picker", T158-T166). Each test names its row.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use smithay::input::keyboard::Keysym;

use crate::buffers::TabCreated;
use crate::config::SCRIPT_BUDGET_MS;
use crate::history;
use crate::keys::*;
use crate::protocol::Command;

const BUDGET: Duration = Duration::from_millis(SCRIPT_BUDGET_MS);

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("web-c-pickdel-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn file(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.0.join(name)).unwrap_or_default()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Three pages, `b` visited twice (so the ranking is b, c, a: pages by visits, then newest first).
const HISTORY: &str = "1000\thttps://a.test/\tSite A\n\
2000\thttps://b.test/\tSite B\n\
2500\thttps://c.test/\tSite C\n\
3000\thttps://b.test/\tSite B\n";

fn keys_in(d: &Dir) -> Keys {
    let mut k = Keys::new(Some(d.0.clone()), "file:///help.html".into(), None);
    k.url = "https://a.test/".into();
    k
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

fn run(k: &mut Keys, line: &str) {
    press(k, KeyInput::ch(':'));
    type_text(k, line);
    press(k, KeyInput::sym(Keysym::Delete));
    press(k, KeyInput::sym(Keysym::Return));
}

fn ctrl_d(k: &mut Keys) {
    press(k, KeyInput::ch('d').ctrl());
}

fn history_keys(tag: &str) -> (Dir, Keys) {
    let d = Dir::new(tag);
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    run(&mut k, "historySearch");
    assert_eq!(k.vim_mode, VimMode::Pick);
    (d, k)
}

fn with_script(k: &mut Keys, src: &str) {
    assert!(k.load_script_source("init.js", src, BUDGET), "{}", k.message);
}

// ---------- T164: :historyDelete ----------

#[test]
fn t164_history_delete_removes_every_line_of_the_url_and_keeps_the_others_byte_for_byte() {
    let d = Dir::new("t164");
    d.file("history", HISTORY);
    assert_eq!(history::delete(&d.0, "https://b.test/").unwrap(), 2);
    assert_eq!(d.read("history"), "1000\thttps://a.test/\tSite A\n2500\thttps://c.test/\tSite C\n");
    assert_eq!(history::delete(&d.0, "https://b.test/").unwrap(), 0);
    // No file: nothing removed.
    let e = Dir::new("t164-empty");
    assert_eq!(history::delete(&e.0, "https://x.test/").unwrap(), 0);
    assert!(!e.0.join("history").exists());
}

#[test]
fn t164_a_site_root_and_its_pages_are_separate_urls() {
    let d = Dir::new("t164-root");
    d.file(
        "history",
        "1\thttps://a.test/\tRoot\n2\thttps://a.test/page\tPage\n3\thttps://a.test/\tRoot\n",
    );
    assert_eq!(history::delete(&d.0, "https://a.test/").unwrap(), 2);
    assert_eq!(d.read("history"), "2\thttps://a.test/page\tPage\n");
}

#[test]
fn t164_history_delete_command_messages() {
    let d = Dir::new("t164-cmd");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    run(&mut k, "historyDelete https://b.test/");
    assert_eq!(k.message, "history entry removed: https://b.test/ (2 visits)");
    assert_eq!(k.vim_mode, VimMode::Normal);
    run(&mut k, "historyDelete https://b.test/");
    assert_eq!(k.message, "web: not in the history: https://b.test/");
    // No URL: the page's.
    k.url = "https://c.test/".into();
    run(&mut k, "historyDelete");
    assert_eq!(k.message, "history entry removed: https://c.test/ (1 visits)");
    assert_eq!(d.read("history"), "1000\thttps://a.test/\tSite A\n");
    run(&mut k, "historyDelete a b");
    assert_eq!(k.message, "web: usage: :historyDelete [url]");
    // No state directory.
    let mut n = Keys::new(None, String::new(), None);
    run(&mut n, "historyDelete https://a.test/");
    assert_eq!(n.message, "web: no state directory: history is off");
    assert!(COMMAND_NAMES.contains(&"historyDelete"));
    let mut sorted = COMMAND_NAMES.to_vec();
    sorted.sort();
    assert_eq!(sorted, COMMAND_NAMES);
}

// ---------- T158, T160, T161: asking ----------

#[test]
fn t158_ctrl_d_asks() {
    let (d, mut k) = history_keys("t158");
    assert!(k.pick_log().contains("[1/3 of 3] https://b.test/"), "{}", k.pick_log());
    ctrl_d(&mut k);
    assert_eq!(k.vim_mode, VimMode::Pick);
    let v = k.bar_view();
    assert!(v.line.is_none(), "no > prompt while asking");
    assert_eq!(v.message, "delete history entry https://b.test/? (y/n)");
    assert!(k.pick_log().ends_with(" confirm"), "{}", k.pick_log());
    assert_eq!(d.read("history"), HISTORY, "nothing deleted yet");
    assert!(k.pick_log().contains("[1/3 of 3]"));
}

#[test]
fn t158_the_question_for_bookmarks_and_buffers() {
    let d = Dir::new("t158-b");
    d.file("bookmarks", "https://a.test/\tSite A\n");
    let mut k = keys_in(&d);
    run(&mut k, "bookmarkSearch");
    ctrl_d(&mut k);
    assert_eq!(k.bar_view().message, "delete bookmark https://a.test/? (y/n)");
    press(&mut k, KeyInput::ch('n'));
    assert_eq!(k.message, "not deleted");

    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = "https://a.test/".into();
    k.title = "Page one".into();
    run(&mut k, "ls");
    ctrl_d(&mut k);
    assert_eq!(k.bar_view().message, "kill buffer 1 Page one? (y/n)");
}

#[test]
fn t160_y_deletes_and_so_does_a_capital_y() {
    let (d, mut k) = history_keys("t160");
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('y'));
    assert_eq!(k.message, "history entry removed: https://b.test/ (2 visits)");
    assert_eq!(d.read("history"), "1000\thttps://a.test/\tSite A\n2500\thttps://c.test/\tSite C\n");
    assert!(!k.pick_log().contains("confirm"));
    assert_eq!(k.vim_mode, VimMode::Pick);
    // A capital Y: the Shift press alone keeps the question (T161), then Y answers.
    ctrl_d(&mut k);
    press(&mut k, KeyInput::sym(Keysym::Shift_L));
    assert!(k.pick_log().ends_with(" confirm"), "a modifier alone keeps the question");
    press(&mut k, KeyInput::ch('Y'));
    assert!(k.message.starts_with("history entry removed:"), "{}", k.message);
    assert_eq!(d.read("history").lines().count(), 1);
}

#[test]
fn t161_any_other_key_cancels_and_is_not_typed() {
    for (name, key) in [
        ("n", KeyInput::ch('n')),
        ("esc", KeyInput::sym(Keysym::Escape)),
        ("letter", KeyInput::ch('x')),
        ("enter", KeyInput::sym(Keysym::Return)),
        ("ctrl-n", KeyInput::ch('n').ctrl()),
        ("ctrl-y", KeyInput::ch('y').ctrl()),
    ] {
        let (d, mut k) = history_keys(&format!("t161-{name}"));
        k.effects.clear();
        ctrl_d(&mut k);
        press(&mut k, key);
        assert_eq!(k.message, "not deleted", "{name}");
        assert_eq!(k.vim_mode, VimMode::Pick, "{name}: the picker stays open");
        assert_eq!(k.line.text, "", "{name}: not typed");
        assert!(!k.pick_log().contains("confirm"), "{name}");
        assert_eq!(d.read("history"), HISTORY, "{name}");
        assert!(
            !k.effects.iter().any(|e| matches!(e, Effect::Send(Command::Open { .. }))),
            "{name}: Enter did not open"
        );
        assert!(k.pick_log().contains("[1/3 of 3]"), "{name}: {}", k.pick_log());
    }
}

// ---------- T162 ----------

#[test]
fn t162_ctrl_d_in_another_source_or_with_no_match_does_nothing() {
    let mut k = Keys::new(None, String::new(), None);
    k.open_picker(
        "popups",
        vec![crate::picker::Row {
            label: "x".into(),
            detail: String::new(),
            fields: vec!["x".into()],
            action: crate::picker::PickAction::Popup(1),
        }],
        "",
        "",
    );
    ctrl_d(&mut k);
    assert_eq!(k.message, "web: nothing to delete in popups");
    assert!(k.picker.as_ref().unwrap().confirm.is_none());
    // No match: nothing.
    let (d, mut k) = history_keys("t162");
    type_text(&mut k, "zzzz");
    k.message.clear();
    ctrl_d(&mut k);
    assert_eq!(k.message, "");
    assert!(k.picker.as_ref().unwrap().confirm.is_none());
    assert_eq!(d.read("history"), HISTORY);
}

// ---------- T163: refresh ----------

#[test]
fn t163_the_selection_stays_at_its_position_and_clamps_on_the_last_row() {
    let (_d, mut k) = history_keys("t163");
    // Ranking: b (2 visits), then c (newest), then a.
    press(&mut k, KeyInput::ch('n').ctrl());
    assert!(k.pick_log().contains("[2/3 of 3] https://c.test/"), "{}", k.pick_log());
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('y'));
    // The row below moved up into the same position.
    assert!(k.pick_log().contains("[2/2 of 2] https://a.test/"), "{}", k.pick_log());
    // The last row: the one above is selected.
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('y'));
    assert!(k.pick_log().contains("[1/1 of 1] https://b.test/"), "{}", k.pick_log());
}

#[test]
fn t163_the_filter_is_kept_and_the_last_row_closes_the_picker() {
    let (d, mut k) = history_keys("t163-last");
    type_text(&mut k, "site");
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('y'));
    assert!(k.pick_log().contains("[1/2 of 2]"), "{}", k.pick_log());
    assert_eq!(k.line.text, "site");
    // The file is down to one entry (by hand); the next delete empties the source.
    d.file("history", "1000\thttps://a.test/\tSite A\n");
    k.refresh_picker();
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('y'));
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.picker.is_none());
    assert_eq!(k.message, "history entry removed: https://a.test/ (1 visits)");
    assert_eq!(d.read("history"), "");
}

#[test]
fn t163_bookmarks_refresh_too() {
    let d = Dir::new("t163-bm");
    d.file("bookmarks", "https://a.test/\tA\nhttps://b.test/\tB\n");
    let mut k = keys_in(&d);
    run(&mut k, "bookmarkSearch");
    assert!(k.pick_log().contains("https://b.test/"), "newest first");
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('y'));
    assert_eq!(k.message, "bookmark removed: https://b.test/");
    assert_eq!(d.read("bookmarks"), "https://a.test/\tA\n");
    assert!(k.pick_log().contains("[1/1 of 1] https://a.test/"), "{}", k.pick_log());
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('y'));
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(k.message, "bookmark removed: https://a.test/");
}

// ---------- T159, T166: the switches ----------

#[test]
fn t159_a_switch_set_false_deletes_without_asking() {
    let d = Dir::new("t159");
    d.file("history", HISTORY);
    d.file("bookmarks", "https://a.test/\tA\nhttps://b.test/\tB\n");
    let mut k = keys_in(&d);
    with_script(
        &mut k,
        "web.history.deleteConfirmation = false; web.bookmarks.deleteConfirmation = false;",
    );
    run(&mut k, "historySearch");
    ctrl_d(&mut k);
    assert_eq!(k.message, "history entry removed: https://b.test/ (2 visits)");
    assert!(k.picker.as_ref().unwrap().confirm.is_none());
    press(&mut k, KeyInput::sym(Keysym::Escape));
    run(&mut k, "bookmarkSearch");
    ctrl_d(&mut k);
    assert_eq!(k.message, "bookmark removed: https://b.test/");
}

#[test]
fn t159_the_buffers_switch_set_false_kills_without_asking() {
    let mut k = two_buffers();
    with_script(&mut k, "web.buffers.deleteConfirmation = false;");
    run(&mut k, "ls");
    press(&mut k, KeyInput::ch('p').ctrl());
    ctrl_d(&mut k);
    assert_eq!(k.message, "buffer 1 killed");
}

#[test]
fn t159_each_switch_is_its_own() {
    let d = Dir::new("t159-own");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    with_script(
        &mut k,
        "web.bookmarks.deleteConfirmation = false; web.buffers.deleteConfirmation = false;",
    );
    run(&mut k, "historySearch");
    ctrl_d(&mut k);
    assert!(k.picker.as_ref().unwrap().confirm.is_some(), "history still asks");
    let s = k.scripts.as_ref().unwrap();
    assert!(s.delete_confirmation("history"));
    assert!(!s.delete_confirmation("buffers"));
    assert!(s.delete_confirmation("popups"));
}

#[test]
fn t166_the_switch_reads_back_and_a_non_boolean_is_a_script_error() {
    let mut k = Keys::new(None, String::new(), None);
    with_script(
        &mut k,
        "if (web.history.deleteConfirmation !== true) throw new Error('default');\n\
         web.history.deleteConfirmation = false;\n\
         if (web.history.deleteConfirmation !== false) throw new Error('set');",
    );
    let mut k = Keys::new(None, String::new(), None);
    assert!(!k.load_script_source("init.js", "web.bookmarks.deleteConfirmation = 1;", BUDGET));
    assert_eq!(k.message, "init.js:1: web.bookmarks.deleteConfirmation must be true or false");
    assert!(k.scripts.as_ref().unwrap().delete_confirmation("bookmarks"), "keeps its value");
    // The switch objects are frozen: nothing else can be added.
    let mut k = Keys::new(None, String::new(), None);
    assert!(!k.load_script_source("init.js", "web.history.other = 1;", BUDGET));
}

#[test]
fn t166_source_puts_the_switches_back_to_true() {
    let d = Dir::new("t166");
    let mut k = keys_in(&d);
    with_script(
        &mut k,
        "web.history.deleteConfirmation = false; web.buffers.deleteConfirmation = false;",
    );
    assert!(!k.scripts.as_ref().unwrap().delete_confirmation("history"));
    k.source();
    // No file behind it now: the runtime is gone or new, every switch is true again.
    assert!(k.scripts.as_ref().is_none_or(|s| s.delete_confirmation("history")));
    assert!(k.scripts.as_ref().is_none_or(|s| s.delete_confirmation("buffers")));
}

// ---------- T165: :ls ----------

fn two_buffers() -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = "https://a.test/one".into();
    k.title = "Page one".into();
    k.open_buffer("https://blue.example/two");
    assert_eq!(k.buffers.tab_created(2, Some(1)), TabCreated::Buffer(2));
    k.title = "Blue page".into();
    k.effects.clear();
    k.message.clear();
    k
}

#[test]
fn t165_ctrl_d_y_on_another_buffer_kills_it_and_keeps_the_list() {
    let mut k = two_buffers();
    run(&mut k, "ls");
    press(&mut k, KeyInput::ch('p').ctrl());
    assert!(k.pick_log().contains("buffer 1"), "{}", k.pick_log());
    ctrl_d(&mut k);
    assert_eq!(k.bar_view().message, "kill buffer 1 Page one? (y/n)");
    press(&mut k, KeyInput::ch('y'));
    assert_eq!(k.message, "buffer 1 killed");
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert!(k.pick_log().contains("[1/1 of 1] buffer 2"), "{}", k.pick_log());
    assert_eq!(k.buffers.current, 2);
}

#[test]
fn t165_ctrl_d_y_on_the_current_buffer_switches_and_keeps_pick() {
    let mut k = two_buffers();
    run(&mut k, "ls");
    press(&mut k, KeyInput::ch('n').ctrl());
    assert!(k.pick_log().contains("buffer 2"), "{}", k.pick_log());
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('y'));
    assert_eq!(k.vim_mode, VimMode::Pick, "the list stays open");
    assert_eq!(k.buffers.current, 1, "the page behind switched to the alternate");
    assert!(k.pick_log().contains("[1/1 of 1] buffer 1"), "{}", k.pick_log());
    assert!(k.effects.iter().any(|e| matches!(e, Effect::Send(Command::CloseTab { tab: 2 }))));
}

#[test]
fn t165_the_only_buffer_is_replaced_by_about_blank_and_one_row_stays() {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = "https://a.test/".into();
    run(&mut k, "ls");
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('y'));
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert_eq!(k.buffers.list.len(), 1);
    assert_eq!(k.url, "about:blank");
    assert!(k.pick_log().contains("of 1]"), "{}", k.pick_log());
}

// ---------- C-PICKMSG: the message shows beside the filter line ----------

fn pick_bar(k: &Keys) -> (String, bool) {
    let v = k.bar_view();
    (v.message, v.line.is_some())
}

#[test]
fn pickmsg_not_deleted_shows_beside_the_filter_and_goes_on_the_next_filter_key() {
    let (_d, mut k) = history_keys("pickmsg-n");
    k.message = "old bar message".into();
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('n'));
    assert_eq!(pick_bar(&k), ("not deleted".to_string(), true));
    // A key that changes neither the filter nor the selection keeps it.
    press(&mut k, KeyInput::sym(Keysym::Right));
    assert_eq!(k.bar_view().message, "not deleted");
    // The next filter key takes it away.
    type_text(&mut k, "b");
    assert_eq!(pick_bar(&k), (String::new(), true));
}

#[test]
fn pickmsg_a_delete_message_shows_and_a_move_clears_it() {
    let (_d, mut k) = history_keys("pickmsg-y");
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('y'));
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert_eq!(
        pick_bar(&k),
        ("history entry removed: https://b.test/ (2 visits)".to_string(), true)
    );
    press(&mut k, KeyInput::ch('n').ctrl());
    assert_eq!(k.bar_view().message, "");
}

#[test]
fn pickmsg_nothing_to_delete_shows_and_the_question_replaces_it() {
    let (_d, mut k) = history_keys("pickmsg-other");
    k.picker.as_mut().unwrap().name = "popups".into();
    ctrl_d(&mut k);
    assert_eq!(pick_bar(&k), ("web: nothing to delete in popups".to_string(), true));
    k.picker.as_mut().unwrap().name = "history".into();
    ctrl_d(&mut k);
    let v = k.bar_view();
    assert!(v.line.is_none());
    assert_eq!(v.message, "delete history entry https://b.test/? (y/n)");
}

#[test]
fn pickmsg_does_not_leak_into_the_next_picker() {
    let (_d, mut k) = history_keys("pickmsg-leak");
    ctrl_d(&mut k);
    press(&mut k, KeyInput::ch('n'));
    press(&mut k, KeyInput::sym(Keysym::Escape));
    assert_eq!(k.vim_mode, VimMode::Normal);
    run(&mut k, "historySearch");
    assert_eq!(pick_bar(&k), (String::new(), true));
}

#[test]
fn pickmsg_the_bar_paints_it_right_of_the_filter() {
    let mut p = crate::statusbar::Painter::new();
    if !p.has_fonts() {
        return;
    }
    let mut v = BarView {
        mode: "PICK".into(),
        url: String::new(),
        message: String::new(),
        line: Some(('>', "ab".into(), 2, None)),
        ghost: String::new(),
        flash: false,
    };
    let plain = p.paint(&v, 600, 24).unwrap();
    v.message = "not deleted".into();
    let with = p.paint(&v, 600, 24).unwrap();
    assert_ne!(plain.data(), with.data());
    // The filter's own left part is the same.
    assert_eq!(plain.data()[..60 * 4], with.data()[..60 * 4]);
}

// ---------- C-PICKDEL-MAP: web.map("p", ...) (T167-T168) ----------

fn ctrl(k: &mut Keys, c: char) {
    press(k, KeyInput::ch(c).ctrl());
}

#[test]
fn t167_a_mapped_key_deletes_the_selected_history_row_without_asking_and_the_list_refreshes() {
    let d = Dir::new("t167-hist");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    with_script(
        &mut k,
        r#"web.map("p", "<C-x>", row => { if (row.source === "history") web.cmd("historyDelete " + row.url); });"#,
    );
    run(&mut k, "historySearch");
    ctrl(&mut k, 'x');
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert!(k.picker.as_ref().unwrap().confirm.is_none());
    assert!(!d.read("history").contains("https://b.test/"), "{}", d.read("history"));
    assert_eq!(k.picker.as_ref().unwrap().total(), 2);
    // T169: the command's message shows beside the filter line.
    assert_eq!(
        pick_bar(&k),
        ("history entry removed: https://b.test/ (2 visits)".to_string(), true)
    );
}

#[test]
fn t167_the_row_has_source_label_detail_and_the_one_field_that_applies() {
    let d = Dir::new("t167-row");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    with_script(&mut k, r#"web.map("p", "<C-x>", row => web.cmd("echo " + JSON.stringify(row)));"#);
    run(&mut k, "historySearch");
    ctrl(&mut k, 'x');
    assert!(
        k.message.starts_with(r#"echo"#) || k.message.starts_with(r#"{"source":"history","label":"Site B","detail":"#),
        "{}",
        k.message
    );
    assert!(k.message.contains(r#""source":"history""#), "{}", k.message);
    assert!(k.message.contains(r#""label":"Site B""#), "{}", k.message);
    assert!(k.message.contains(r#""url":"https://b.test/""#), "{}", k.message);
    assert!(!k.message.contains("buffer"));
}

#[test]
fn t167_bookmarks_and_buffers_rows() {
    let d = Dir::new("t167-bm");
    d.file("bookmarks", "https://a.test/\tA\nhttps://b.test/\tB\n");
    let mut k = keys_in(&d);
    with_script(
        &mut k,
        r#"web.map("p", "<C-x>", row => { if (row.source === "bookmarks") web.cmd("bookmarkDelete " + row.url); });"#,
    );
    run(&mut k, "bookmarkSearch");
    ctrl(&mut k, 'x');
    assert_eq!(pick_bar(&k).0, "bookmark removed: https://b.test/");
    assert_eq!(k.picker.as_ref().unwrap().total(), 1);

    let mut k = two_buffers();
    with_script(
        &mut k,
        r#"web.map("p", "<C-x>", row => { if (row.source === "buffers") web.cmd("bd " + row.buffer); });"#,
    );
    run(&mut k, "ls");
    assert_eq!(k.vim_mode, VimMode::Pick);
    let total = k.picker.as_ref().unwrap().total();
    ctrl(&mut k, 'x');
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert_eq!(k.picker.as_ref().unwrap().total(), total - 1);
}

#[test]
fn t167_a_mapped_ctrl_d_replaces_the_built_in_one() {
    let d = Dir::new("t167-cd");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    with_script(&mut k, r#"web.map("p", "<C-d>", row => web.cmd("echo mapped " + row.label));"#);
    run(&mut k, "historySearch");
    ctrl_d(&mut k);
    assert!(k.picker.as_ref().unwrap().confirm.is_none());
    assert_eq!(pick_bar(&k).0, "mapped Site B");
    assert_eq!(d.read("history"), HISTORY);
}

#[test]
fn t167_a_mapped_ctrl_n_replaces_the_move() {
    let d = Dir::new("t167-cn");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    with_script(&mut k, r#"web.map("p", "<C-n>", row => web.cmd("echo at " + row.label));"#);
    run(&mut k, "historySearch");
    ctrl(&mut k, 'n');
    assert_eq!(pick_bar(&k).0, "at Site B");
    assert_eq!(k.picker.as_ref().unwrap().selected, 0);
}

#[test]
fn t167_no_row_selected_does_nothing() {
    let d = Dir::new("t167-norow");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    with_script(&mut k, r#"web.map("p", "<C-x>", row => web.cmd("echo called"));"#);
    run(&mut k, "historySearch");
    type_text(&mut k, "zzzzqqq");
    assert_eq!(k.picker.as_ref().unwrap().match_count(), 0);
    ctrl(&mut k, 'x');
    assert_ne!(k.message, "called");
    assert_eq!(k.vim_mode, VimMode::Pick);
}

#[test]
fn t167_modes_are_separate() {
    let d = Dir::new("t167-modes");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    with_script(
        &mut k,
        r#"web.map("p", "<C-x>", row => web.cmd("echo pick")); web.map("n", "<C-y>", () => web.cmd("echo normal"));"#,
    );
    ctrl(&mut k, 'x');
    assert_ne!(k.message, "pick");
    run(&mut k, "historySearch");
    ctrl(&mut k, 'y');
    assert_ne!(k.message, "normal");
}

#[test]
fn t167_the_same_key_replaces_in_place_and_other_keys_still_type() {
    let d = Dir::new("t167-repl");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    with_script(
        &mut k,
        r#"web.map("p", "<C-x>", () => web.cmd("echo one")); web.map("p", "<C-x>", () => web.cmd("echo two"));"#,
    );
    run(&mut k, "historySearch");
    type_text(&mut k, "b");
    assert_eq!(k.line.text, "b");
    ctrl(&mut k, 'x');
    assert_eq!(pick_bar(&k).0, "two");
}

#[test]
fn t167_bad_p_mappings_are_refused() {
    let one_key = r#"a "p" mapping is one <C-x> key (letters type the filter)"#;
    let cases = [
        (r#"web.map("p", "x", () => {})"#, one_key),
        (r#"web.map("p", "<C-a><C-b>", () => {})"#, one_key),
        (r#"web.map("p", "<C-d>", ":bd<CR>")"#, r#"a "p" mapping's rhs is a function"#),
        (r#"web.map("q", "<C-d>", () => {})"#, r#"(known: "n", "p")"#),
    ];
    for (i, (src, want)) in cases.iter().enumerate() {
        let d = Dir::new(&format!("t167-bad{i}"));
        let mut k = keys_in(&d);
        k.load_script_source("init.js", src, BUDGET);
        assert!(k.message.contains(want), "{src}: {}", k.message);
        for c in ['x', 'a', 'd'] {
            assert!(
                k.scripts
                    .as_ref()
                    .is_none_or(|s| s.pick_map(&MapKey { ch: c, ctrl: true }).is_none()),
                "{src}"
            );
        }
    }
}

#[test]
fn t167_source_drops_p_mappings() {
    let d = Dir::new("t167-source");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    with_script(&mut k, r#"web.map("p", "<C-x>", () => web.cmd("echo gone"));"#);
    k.source();
    run(&mut k, "historySearch");
    ctrl(&mut k, 'x');
    assert_ne!(k.message, "gone");
}

#[test]
fn t168_a_script_error_shows_beside_the_filter_and_pick_stays() {
    let d = Dir::new("t168-err");
    d.file("history", HISTORY);
    let mut k = keys_in(&d);
    with_script(&mut k, r#"web.map("p", "<C-x>", row => { throw new Error("boom"); });"#);
    run(&mut k, "historySearch");
    ctrl(&mut k, 'x');
    assert_eq!(k.vim_mode, VimMode::Pick);
    assert!(pick_bar(&k).0.contains("boom"), "{}", k.message);
}

#[test]
fn t168_deleting_the_last_row_closes_the_picker() {
    let d = Dir::new("t168-last");
    d.file("history", "1000\thttps://a.test/\tSite A\n");
    let mut k = keys_in(&d);
    with_script(&mut k, r#"web.map("p", "<C-x>", row => web.cmd("historyDelete " + row.url));"#);
    run(&mut k, "historySearch");
    ctrl(&mut k, 'x');
    assert_eq!(k.vim_mode, VimMode::Normal);
}

// ---------- C-HISTREAL: every row is a real history line ----------

#[test]
fn c_histreal_ctrl_d_y_on_every_row_deletes_something() {
    let d = Dir::new("histreal");
    // Pages of two hosts, no root line for either: no made-up root rows to fail on.
    d.file(
        "history",
        "1\thttps://h.test/a\tA\n2\thttps://h.test/b\tB\n3\thttps://h.test/b\tB\n4\thttps://i.test/c\tC\n",
    );
    let mut k = keys_in(&d);
    run(&mut k, "historySearch");
    assert!(k.pick_log().contains("[1/3 of 3]"), "{}", k.pick_log());
    for left in (1..=3).rev() {
        ctrl_d(&mut k);
        press(&mut k, KeyInput::ch('y'));
        assert!(k.message.starts_with("history entry removed:"), "{left}: {}", k.message);
    }
    assert_eq!(d.read("history"), "");
    assert_eq!(k.vim_mode, VimMode::Normal);
}
