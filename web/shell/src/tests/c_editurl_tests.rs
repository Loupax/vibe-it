//! C-EDITURL (KEYS.md T129-T133): `:go ` and the other URL commands fill in the page's URL, selected.

use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::keys::*;
use crate::protocol::Command;

const PAGE: &str = "https://a.test/one";

fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

fn type_str(k: &mut Keys, s: &str) {
    for c in s.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

fn sym(k: &mut Keys, s: Keysym) {
    press(k, KeyInput::sym(s));
}

/// A shell on `PAGE` with the `:` line open.
fn line() -> Keys {
    let mut k = Keys::new(None, "file:///help.html".into(), None);
    k.url = PAGE.into();
    press(&mut k, KeyInput::ch(':'));
    k
}

fn text(k: &Keys) -> String {
    k.line.text.clone()
}

/// `:go ` typed, the URL selected.
fn filled() -> Keys {
    let mut k = line();
    type_str(&mut k, "go ");
    assert_eq!(k.line.sel, Some((3, 3 + PAGE.len())));
    k
}

fn opened(k: &mut Keys) -> Option<String> {
    std::mem::take(&mut k.effects).into_iter().find_map(|e| match e {
        Effect::Send(Command::Open { url }) => Some(url),
        _ => None,
    })
}

// ---------- T129: triggers ----------

#[test]
fn t129_space_after_each_url_command_fills_the_page_url_selected() {
    for name in ["go", "new", "bufferNew", "openExternal"] {
        let mut k = line();
        type_str(&mut k, &format!("{name} "));
        assert_eq!(text(&k), format!("{name} {PAGE}"), "{name}");
        assert_eq!(k.line.sel, Some((name.len() + 1, name.len() + 1 + PAGE.len())));
        assert_eq!(k.line.cursor, k.line.text.len(), "cursor at the end");
        assert!(k.ghost.is_none(), "no ghost while selected");
        let v = k.bar_view();
        assert_eq!(v.line.unwrap().3, k.line.sel, "BarView.line carries the range");
    }
}

#[test]
fn t129_tab_that_completes_the_name_fills_too() {
    let mut k = line();
    type_str(&mut k, "openE");
    sym(&mut k, Keysym::Tab);
    assert_eq!(text(&k), format!("openExternal {PAGE}"));
    assert!(k.line.sel.is_some());
}

#[test]
fn t129_other_commands_do_not_fill() {
    let mut k = line();
    type_str(&mut k, "refresh ");
    assert_eq!(text(&k), "refresh ");
    assert!(k.line.sel.is_none());
}

#[test]
fn t129_not_exactly_the_command_and_a_space_does_not_fill() {
    // `x go ` is not exactly `<command> `.
    let mut k = line();
    type_str(&mut k, "x go ");
    assert_eq!(text(&k), "x go ");
    assert!(k.line.sel.is_none());
    // Cursor not at the end: Space typed inside `gox`.
    let mut k = line();
    type_str(&mut k, "gox");
    sym(&mut k, Keysym::Left);
    type_str(&mut k, " ");
    assert_eq!(text(&k), "go x");
    assert!(k.line.sel.is_none());
}

#[test]
fn t129_clearing_the_selection_or_backspacing_to_the_command_does_not_refill() {
    for clear in [
        KeyInput::sym(Keysym::BackSpace),
        KeyInput::ch('w').ctrl(),
        KeyInput::sym(Keysym::Delete),
    ] {
        let mut k = filled();
        press(&mut k, clear);
        assert_eq!(text(&k), "go ", "the selection is gone, `go ` stays");
        assert!(k.line.sel.is_none());
        assert_eq!(k.line.cursor, 3);
    }
    // Typed `go x`, Backspace x: `go ` again, no refill.
    let mut k = filled();
    type_str(&mut k, "x");
    sym(&mut k, Keysym::BackSpace);
    assert_eq!(text(&k), "go ");
    assert!(k.line.sel.is_none());
    // Ctrl-w on `go x` leaves `go `, no refill.
    let mut k = filled();
    type_str(&mut k, "x");
    press(&mut k, KeyInput::ch('w').ctrl());
    assert_eq!(text(&k), "go ");
    assert!(k.line.sel.is_none());
}

#[test]
fn t129_a_history_recall_does_not_fill() {
    let mut k = line();
    k.cmd_history.record("go");
    type_str(&mut k, "g");
    sym(&mut k, Keysym::Up);
    assert_eq!(text(&k), "go");
    assert!(k.line.sel.is_none());
}

#[test]
fn t129_no_page_no_fill() {
    for url in ["", "about:blank", "file:///help.html"] {
        let mut k = line();
        k.url = url.into();
        type_str(&mut k, "go ");
        assert_eq!(text(&k), "go ", "{url:?}");
        assert!(k.line.sel.is_none());
    }
}

#[test]
fn t129_open_line_with_fills_for_a_mapping_without_cr() {
    let mut k = Keys::new(None, "file:///help.html".into(), None);
    k.url = PAGE.into();
    k.open_line_with("go ");
    assert_eq!(text(&k), format!("go {PAGE}"));
    assert_eq!(k.line.sel, Some((3, 3 + PAGE.len())));
    // Text that is not exactly a URL command and a space is as given.
    k.open_line_with("go x");
    assert_eq!(text(&k), "go x");
    assert!(k.line.sel.is_none());
}

#[test]
fn t129_the_search_line_has_no_fill() {
    let mut k = Keys::new(None, String::new(), None);
    k.url = PAGE.into();
    press(&mut k, KeyInput::ch('/'));
    type_str(&mut k, "go ");
    assert_eq!(text(&k), "go ");
    assert!(k.line.sel.is_none());
}

#[test]
fn t104_new_without_argument_fills_the_url_of_the_buffer_it_left() {
    let mut k = Keys::new(None, String::new(), None);
    k.buffers.tab_created(1, None);
    k.url = PAGE.into();
    press(&mut k, KeyInput::ch(':'));
    type_str(&mut k, "new");
    sym(&mut k, Keysym::Delete); // hides the ghost
    sym(&mut k, Keysym::Return);
    assert_eq!(k.vim_mode, VimMode::Command, "the line opens again");
    assert_eq!(text(&k), format!("go {PAGE}"));
    assert_eq!(k.line.sel, Some((3, 3 + PAGE.len())));
    // Enter at once opens a copy in the new buffer.
    k.effects.clear();
    sym(&mut k, Keysym::Return);
    assert_eq!(opened(&mut k).as_deref(), Some(PAGE));
}

#[test]
fn t104_new_without_argument_with_no_page_fills_nothing() {
    let mut k = Keys::new(None, String::new(), None);
    k.buffers.tab_created(1, None);
    press(&mut k, KeyInput::ch(':'));
    type_str(&mut k, "new");
    sym(&mut k, Keysym::Delete);
    sym(&mut k, Keysym::Return);
    assert_eq!(text(&k), "go ");
    assert!(k.line.sel.is_none());
}

// ---------- T130-T133: keys on the selection ----------

#[test]
fn t130_a_character_replaces_the_selection() {
    let mut k = filled();
    type_str(&mut k, "x");
    assert_eq!(text(&k), "go x");
    assert!(k.line.sel.is_none());
    assert_eq!(k.line.cursor, 4);
    // A space too; it does not refill.
    let mut k = filled();
    type_str(&mut k, " ");
    assert_eq!(text(&k), "go  ");
    assert!(k.line.sel.is_none());
}

#[test]
fn t130_backspace_delete_ctrl_w_remove_the_selection_only() {
    for key in [
        KeyInput::sym(Keysym::BackSpace),
        KeyInput::sym(Keysym::Delete),
        KeyInput::sym(Keysym::KP_Delete),
        KeyInput::ch('w').ctrl(),
    ] {
        let mut k = filled();
        press(&mut k, key);
        assert_eq!(text(&k), "go ");
        assert_eq!(k.line.cursor, 3);
        assert!(k.line.sel.is_none());
    }
}

#[test]
fn t130_ctrl_u_removes_the_selection_and_what_is_before_it() {
    let mut k = filled();
    press(&mut k, KeyInput::ch('u').ctrl());
    assert_eq!(text(&k), "");
    assert_eq!(k.line.cursor, 0);
    assert!(k.line.sel.is_none());
}

#[test]
fn t131_right_and_end_drop_the_selection_cursor_at_the_end() {
    for key in [Keysym::Right, Keysym::End, Keysym::KP_Right, Keysym::KP_End] {
        let mut k = filled();
        sym(&mut k, key);
        assert_eq!(text(&k), format!("go {PAGE}"));
        assert_eq!(k.line.cursor, k.line.text.len());
        assert!(k.line.sel.is_none());
    }
}

#[test]
fn t131_left_and_home_drop_it_cursor_at_the_start_of_the_url() {
    for key in [Keysym::Left, Keysym::Home, Keysym::KP_Left, Keysym::KP_Home] {
        let mut k = filled();
        sym(&mut k, key);
        assert_eq!(text(&k), format!("go {PAGE}"));
        assert_eq!(k.line.cursor, 3);
        assert!(k.line.sel.is_none());
    }
}

#[test]
fn t132_enter_runs_the_line_as_it_is_and_esc_cancels() {
    let mut k = filled();
    k.effects.clear();
    sym(&mut k, Keysym::Return);
    assert_eq!(opened(&mut k).as_deref(), Some(PAGE));
    assert_eq!(k.vim_mode, VimMode::Normal);
    let mut k = filled();
    sym(&mut k, Keysym::Escape);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.line.sel.is_none() && k.line.text.is_empty());
}

#[test]
fn t132_tab_and_unbound_keys_change_nothing() {
    for key in [
        KeyInput::sym(Keysym::Tab),
        KeyInput::sym(Keysym::ISO_Left_Tab).shift(),
        KeyInput::ch('a').ctrl(),
        KeyInput::ch('x').alt(),
    ] {
        let mut k = filled();
        press(&mut k, key);
        assert_eq!(text(&k), format!("go {PAGE}"));
        assert_eq!(k.line.sel, Some((3, 3 + PAGE.len())));
    }
}

#[test]
fn t132_paste_without_a_clipboard_keeps_the_selection() {
    let mut k = filled();
    press(&mut k, KeyInput::ch('v').ctrl());
    assert_eq!(text(&k), format!("go {PAGE}"));
    assert_eq!(k.line.sel, Some((3, 3 + PAGE.len())));
}

#[test]
fn t129_the_bar_paints_the_selected_range_in_the_selection_colours() {
    use crate::config::SELECTION_BG;
    use crate::statusbar::Painter;
    let mut p = Painter::new();
    if !p.has_fonts() {
        eprintln!("no fonts on this machine: skipped");
        return;
    }
    let v = filled().bar_view();
    let mut plain = v.clone();
    plain.line.as_mut().unwrap().3 = None;
    let plain = p.paint(&plain, 600, 24).unwrap();
    let sel = p.paint(&v, 600, 24).unwrap();
    let count = |pm: &tiny_skia::Pixmap| {
        pm.pixels()
            .iter()
            .filter(|c| (c.red(), c.green(), c.blue()) == SELECTION_BG)
            .count()
    };
    assert_eq!(count(&plain), 0);
    assert!(count(&sel) > 200, "the highlight is drawn");
}

#[test]
fn t133_up_down_browse_history_by_what_is_before_the_selection() {
    let mut k = line();
    k.cmd_history.record("refresh");
    k.cmd_history.record("go https://old.test/");
    type_str(&mut k, "go ");
    assert!(k.line.sel.is_some());
    sym(&mut k, Keysym::Up);
    assert_eq!(text(&k), "go https://old.test/", "filtered by `go `, not by the URL");
    assert!(k.line.sel.is_none(), "a recall is plain text, no new fill");
    sym(&mut k, Keysym::Down);
    assert_eq!(text(&k), "go ", "past the newest the typed prefix comes back");
    assert!(k.line.sel.is_none(), "and no refill");
    // No match: the selection stays.
    let mut k = filled();
    sym(&mut k, Keysym::Up);
    assert_eq!(text(&k), format!("go {PAGE}"));
    assert!(k.line.sel.is_some());
}
