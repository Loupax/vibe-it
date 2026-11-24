//! C-CMDHIST (KEYS.md T125-T128, STASH.md "Command history"): Up/Down in the `:` line and `DIR/cmdhistory`.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::cmdhist::CmdHistory;
use crate::keys::*;

fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

fn type_str(k: &mut Keys, s: &str) {
    for c in s.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

fn sym(s: Keysym) -> KeyInput {
    KeyInput::sym(s)
}

/// Opens `:`, types `text`, Enter.
fn run(k: &mut Keys, text: &str) {
    press(k, KeyInput::ch(':'));
    type_str(k, text);
    press(k, sym(Keysym::Return));
}

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("web-c-cmdhist-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        Dir(d)
    }
    fn file(&self) -> PathBuf {
        self.0.join("cmdhistory")
    }
    fn text(&self) -> String {
        std::fs::read_to_string(self.file()).unwrap_or_default()
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Lines `aa1`, `bb1`, `aa2`, `cc1` run (unknown commands: only a message), then `:` opened.
fn four() -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    for l in ["aa1", "bb1", "aa2", "cc1"] {
        run(&mut k, l);
    }
    press(&mut k, KeyInput::ch(':'));
    k
}

#[test]
fn t125_up_walks_older_and_down_newer_and_past_the_newest_restores_the_typed_text() {
    let mut k = four();
    press(&mut k, sym(Keysym::Up));
    assert_eq!(k.line.text, "cc1");
    assert_eq!(k.line.cursor, 3);
    press(&mut k, sym(Keysym::Up));
    assert_eq!(k.line.text, "aa2");
    press(&mut k, sym(Keysym::KP_Up));
    press(&mut k, sym(Keysym::Up));
    assert_eq!(k.line.text, "aa1");
    press(&mut k, sym(Keysym::Up));
    assert_eq!(k.line.text, "aa1", "the oldest: no change");
    press(&mut k, sym(Keysym::Down));
    assert_eq!(k.line.text, "bb1");
    press(&mut k, sym(Keysym::KP_Down));
    press(&mut k, sym(Keysym::Down));
    assert_eq!(k.line.text, "cc1");
    press(&mut k, sym(Keysym::Down));
    assert_eq!(k.line.text, "", "past the newest: what was typed (nothing)");
    press(&mut k, sym(Keysym::Down));
    assert_eq!(k.line.text, "");
}

#[test]
fn t126_what_was_typed_filters_and_comes_back_past_the_newest() {
    let mut k = four();
    type_str(&mut k, "aa");
    press(&mut k, sym(Keysym::Up));
    assert_eq!(k.line.text, "aa2");
    press(&mut k, sym(Keysym::Up));
    assert_eq!(k.line.text, "aa1");
    press(&mut k, sym(Keysym::Up));
    assert_eq!(k.line.text, "aa1");
    press(&mut k, sym(Keysym::Down));
    assert_eq!(k.line.text, "aa2");
    press(&mut k, sym(Keysym::Down));
    assert_eq!(k.line.text, "aa", "the typed text comes back");
}

#[test]
fn t126_no_match_changes_nothing() {
    let mut k = four();
    type_str(&mut k, "zz");
    press(&mut k, sym(Keysym::Up));
    assert_eq!(k.line.text, "zz");
    press(&mut k, sym(Keysym::Down));
    assert_eq!(k.line.text, "zz");
}

#[test]
fn t127_another_key_ends_browsing_and_keeps_the_text() {
    let mut k = four();
    type_str(&mut k, "aa");
    press(&mut k, sym(Keysym::Up)); // aa2
    type_str(&mut k, "x"); // aa2x, browsing over
    press(&mut k, sym(Keysym::Down));
    assert_eq!(k.line.text, "aa2x", "Down with no browsing does nothing");
    press(&mut k, sym(Keysym::BackSpace));
    press(&mut k, sym(Keysym::Up)); // new browse with prefix "aa2"
    assert_eq!(k.line.text, "aa2");
}

#[test]
fn t127_a_recalled_line_has_no_ghost_and_enter_runs_exactly_it() {
    let mut k = Keys::new(None, String::new(), None);
    k.cmd_history.record("se"); // a ghost-less record of a line that would get a ghost (`set`)
    press(&mut k, KeyInput::ch(':'));
    type_str(&mut k, "s");
    assert!(k.ghost.is_some(), "typing `s` shows a command-name ghost");
    press(&mut k, sym(Keysym::Up));
    assert_eq!(k.line.text, "se");
    assert!(k.ghost.is_none());
    press(&mut k, sym(Keysym::Return));
    assert_eq!(k.last_command_line(), Some("se"), "not `se` plus a ghost");
}

#[test]
fn t128_only_what_enter_ran_is_kept_and_a_repeat_moves_to_the_end() {
    let mut k = Keys::new(None, String::new(), None);
    run(&mut k, "aa");
    run(&mut k, "bb");
    run(&mut k, "aa");
    run(&mut k, "");
    k.run_command_line("never"); // a binding or mapping, not a submitted line
    assert_eq!(k.cmd_history.entries(), ["bb", "aa"]);
    assert_eq!(k.last_command_line(), Some("aa"));
}

#[test]
fn t128_a_line_opened_with_text_uses_it_as_the_prefix() {
    let mut k = Keys::new(None, String::new(), None);
    run(&mut k, "bb1");
    run(&mut k, "aa1");
    k.open_line_with("bb");
    press(&mut k, sym(Keysym::Up));
    assert_eq!(k.line.text, "bb1");
}

#[test]
fn a_line_is_saved_and_a_restart_reads_it_back() {
    let d = Dir::new("reload");
    let mut k = Keys::new(Some(d.0.clone()), String::new(), None);
    run(&mut k, "aa1");
    run(&mut k, "bb1");
    assert_eq!(d.text(), "aa1\nbb1\n");
    let mut k2 = Keys::new(Some(d.0.clone()), String::new(), None);
    assert_eq!(k2.last_command_line(), Some("bb1"));
    press(&mut k2, KeyInput::ch(':'));
    press(&mut k2, sym(Keysym::Up));
    press(&mut k2, sym(Keysym::Up));
    assert_eq!(k2.line.text, "aa1");
}

#[test]
fn the_file_is_0600_in_a_0700_directory() {
    let d = Dir::new("mode");
    let sub = d.0.join("state");
    let mut h = CmdHistory::open(Some(sub.clone()));
    h.record("aa");
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&sub.join("cmdhistory")), 0o600);
    assert_eq!(mode(&sub), 0o700);
}

#[test]
fn start_drops_duplicates_keeping_the_newest_and_trims_to_the_newest_max() {
    let d = Dir::new("trim");
    std::fs::create_dir_all(&d.0).unwrap();
    std::fs::write(d.file(), "a\nb\na\nc\nd\ne\n").unwrap();
    let h = CmdHistory::open_max(Some(d.0.clone()), 100);
    assert_eq!(h.entries(), ["b", "a", "c", "d", "e"]);
    assert_eq!(d.text(), "b\na\nc\nd\ne\n", "the file was rewritten");
    let h = CmdHistory::open_max(Some(d.0.clone()), 3);
    assert_eq!(h.entries(), ["c", "d", "e"]);
    assert_eq!(d.text(), "c\nd\ne\n");
    assert!(!d
        .0
        .read_dir()
        .unwrap()
        .any(|e| e.unwrap().file_name().to_string_lossy().contains("tmp")));
}

#[test]
fn control_characters_are_stripped_and_blank_lines_skipped() {
    let d = Dir::new("ctl");
    std::fs::create_dir_all(&d.0).unwrap();
    std::fs::write(d.file(), "go \u{1b}[31mx\ttab\n\n   \nok\r\n").unwrap();
    let mut h = CmdHistory::open(Some(d.0.clone()));
    assert_eq!(h.entries(), ["go [31mxtab", "ok"]);
    h.record("a\u{7}b\nc");
    assert_eq!(h.last(), Some("abc"));
    assert!(!d.text().contains(|c: char| c.is_control() && c != '\n'));
    assert_eq!(d.text().lines().count(), 3);
}

#[test]
fn without_a_state_dir_it_stays_in_memory() {
    let mut h = CmdHistory::open(None);
    h.record("aa");
    assert_eq!(h.last(), Some("aa"));
}

#[test]
fn the_memory_list_is_capped_too() {
    let mut h = CmdHistory::open_max(None, 2);
    for l in ["a", "b", "c"] {
        h.record(l);
    }
    assert_eq!(h.entries(), ["b", "c"]);
}
