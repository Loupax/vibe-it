//! C6: KEYS.md "Paste", "Registers" (`+`), T22/T26 paste and T54-T56 `yy` with a desktop
//! clipboard (a fake one here; `shell/test/c6_live.py` uses the nested desktop's real one).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use smithay::input::keyboard::Keysym;

use crate::clipboard::{clean_paste_text, pick_text_mime, SystemClipboard, Which};
use crate::keys::*;
use crate::registers::{let_message, CLIPBOARD_EMPTY, CLIPBOARD_UNREACHABLE, COPIED};

/// The desktop's clipboard as the test sees it: `None` = unreachable.
#[derive(Default)]
struct Fake {
    clipboard: Option<String>,
    primary: Option<String>,
    writes: usize,
}

#[derive(Clone, Default)]
struct FakeClipboard(Rc<RefCell<Fake>>);

impl SystemClipboard for FakeClipboard {
    fn read(&mut self, which: Which) -> Result<String, String> {
        let f = self.0.borrow();
        let v = if which == Which::Clipboard { &f.clipboard } else { &f.primary };
        v.clone().ok_or_else(|| "unreachable".to_string())
    }
    fn write(&mut self, which: Which, text: &str) -> Result<(), String> {
        let mut f = self.0.borrow_mut();
        let v = if which == Which::Clipboard {
            &mut f.clipboard
        } else {
            &mut f.primary
        };
        if v.is_none() {
            return Err("unreachable".into());
        }
        *v = Some(text.to_string());
        f.writes += 1;
        Ok(())
    }
}

fn fake(text: &str) -> FakeClipboard {
    FakeClipboard(Rc::new(RefCell::new(Fake {
        clipboard: Some(text.to_string()),
        primary: Some(String::new()),
        writes: 0,
    })))
}

fn unreachable_clipboard() -> FakeClipboard {
    FakeClipboard::default()
}

const URL: &str = "https://example.com/a?b#c";

fn keys_with(cb: &FakeClipboard) -> Keys {
    let mut k = Keys::new(None, "file:///help.html".into(), None);
    k.clipboard = Some(Box::new(cb.clone()));
    k.url = URL.into();
    k
}

fn tap(k: &mut Keys, key: KeyInput) -> bool {
    let t = Instant::now();
    let c = key.sym.raw();
    let got = k.key(c, true, &key, t);
    k.key(c, false, &key, t);
    got
}

fn type_str(k: &mut Keys, s: &str) {
    for c in s.chars() {
        tap(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

fn clip(cb: &FakeClipboard) -> Option<String> {
    cb.0.borrow().clipboard.clone()
}

// ---------- T54-T56: yy ----------

#[test]
fn c6_t55_yy_copies_the_url_to_the_clipboard() {
    let cb = fake("old");
    let mut k = keys_with(&cb);
    assert!(!tap(&mut k, KeyInput::ch('y')), "T54: the page does not get y");
    assert_eq!(clip(&cb).as_deref(), Some("old"), "T54: one y copies nothing");
    assert!(!tap(&mut k, KeyInput::ch('y')));
    assert_eq!(clip(&cb).as_deref(), Some(URL));
    assert_eq!(k.message, COPIED);
    assert_eq!(k.vim_mode, VimMode::Normal);
}

#[test]
fn c6_t55_yy_ignores_key_repeat() {
    let cb = fake("old");
    let mut k = keys_with(&cb);
    let t = Instant::now();
    let y = KeyInput::ch('y');
    k.key(y.sym.raw(), true, &y, t);
    k.key(y.sym.raw(), true, &y, t); // an OS repeat of the held y
    assert_eq!(clip(&cb).as_deref(), Some("old"));
    assert_eq!(cb.0.borrow().writes, 0);
}

#[test]
fn c6_t56_g_y_y_copies_and_y_g_g_does_not() {
    let cb = fake("old");
    let mut k = keys_with(&cb);
    type_str(&mut k, "gyy");
    assert_eq!(clip(&cb).as_deref(), Some(URL), "g y leaves y pending");
    let cb = fake("old");
    let mut k = keys_with(&cb);
    type_str(&mut k, "ygg");
    assert_eq!(clip(&cb).as_deref(), Some("old"));
}

#[test]
fn c6_t55_yy_in_input_is_the_pages() {
    let cb = fake("old");
    let mut k = keys_with(&cb);
    k.set_key_mode(KeyMode::Input);
    assert!(tap(&mut k, KeyInput::ch('y')));
    assert!(tap(&mut k, KeyInput::ch('y')));
    assert_eq!(clip(&cb).as_deref(), Some("old"));
}

#[test]
fn c6_t55_yy_with_no_url_or_no_clipboard() {
    let cb = fake("old");
    let mut k = keys_with(&cb);
    k.url.clear();
    type_str(&mut k, "yy");
    assert_eq!(k.message, "web: no URL to copy");
    assert_eq!(clip(&cb).as_deref(), Some("old"));
    let cb = unreachable_clipboard();
    let mut k = keys_with(&cb);
    type_str(&mut k, "yy");
    assert_eq!(k.message, CLIPBOARD_UNREACHABLE);
}

// ---------- Registers: :let with @+ ----------

#[test]
fn c6_registers_let_plus_eq_percent_copies_the_url() {
    let cb = fake("old");
    let mut k = keys_with(&cb);
    k.run_command_line("let @+ = @%");
    assert_eq!(k.message, COPIED);
    assert_eq!(clip(&cb).as_deref(), Some(URL));
    // Spaces optional.
    let cb = fake("old");
    let mut k = keys_with(&cb);
    k.run_command_line("let @+=@%");
    assert_eq!(clip(&cb).as_deref(), Some(URL));
}

#[test]
fn c6_registers_plus_is_readable() {
    // `:let @+ = @+` reads the clipboard (and writes it back): an empty one says so.
    let mut cb = fake("hello");
    assert_eq!(let_message("@+ = @+", URL, Some(&mut cb as &mut dyn SystemClipboard)), COPIED);
    assert_eq!(clip(&cb).as_deref(), Some("hello"));
    let mut cb = fake("");
    assert_eq!(
        let_message("@+ = @+", URL, Some(&mut cb as &mut dyn SystemClipboard)),
        CLIPBOARD_EMPTY
    );
    assert_eq!(cb.0.borrow().writes, 0);
    let mut cb = unreachable_clipboard();
    assert_eq!(
        let_message("@+ = @+", URL, Some(&mut cb as &mut dyn SystemClipboard)),
        CLIPBOARD_UNREACHABLE
    );
    assert_eq!(
        let_message("@+ = @%", URL, Some(&mut cb as &mut dyn SystemClipboard)),
        CLIPBOARD_UNREACHABLE
    );
    // `%` stays read-only, whatever the clipboard holds.
    let mut cb = fake("hello");
    assert_eq!(
        let_message("@% = @+", URL, Some(&mut cb as &mut dyn SystemClipboard)),
        "web: register @% is read-only"
    );
    assert_eq!(cb.0.borrow().writes, 0);
}

// ---------- Paste (T22, T26) ----------

fn paste_keys() -> [KeyInput; 3] {
    [
        KeyInput::ch('v').ctrl(),
        KeyInput::ch('V').ctrl().shift(),
        KeyInput::sym(Keysym::Insert).shift(),
    ]
}

#[test]
fn c6_paste_t26_ctrl_v_ctrl_shift_v_shift_insert_paste_into_the_command_line() {
    for key in paste_keys() {
        let cb = fake("example.org");
        let mut k = keys_with(&cb);
        tap(&mut k, KeyInput::ch(':'));
        type_str(&mut k, "go ");
        tap(&mut k, key);
        assert_eq!(k.line.text, "go example.org", "{key:?}");
        assert_eq!(k.line.cursor, k.line.text.len());
        assert_eq!(k.vim_mode, VimMode::Command);
        // At the cursor, not at the end.
        tap(&mut k, KeyInput::sym(Keysym::Home));
        tap(&mut k, key);
        assert_eq!(k.line.text, "example.orggo example.org");
    }
}

#[test]
fn c6_paste_t45_a_paste_is_an_edit_for_the_ghost() {
    let cb = fake("he");
    let mut k = keys_with(&cb);
    tap(&mut k, KeyInput::ch(':'));
    tap(&mut k, KeyInput::ch('v').ctrl());
    assert_eq!(k.line.text, "he");
    assert_eq!(k.bar_view().ghost, "lp", "the pasted word completes like a typed one");
}

#[test]
fn c6_paste_t22_pastes_into_the_search_line() {
    for key in paste_keys() {
        let cb = fake("needle");
        let mut k = keys_with(&cb);
        tap(&mut k, KeyInput::ch('/'));
        tap(&mut k, key);
        assert_eq!(k.line.text, "needle", "{key:?}");
        assert_eq!(k.vim_mode, VimMode::Search);
    }
}

#[test]
fn c6_paste_makes_one_line() {
    assert_eq!(clean_paste_text("https://a.test/\n"), "https://a.test/");
    assert_eq!(clean_paste_text("a\tb\r\nc\n\n  "), "a b c");
    assert_eq!(clean_paste_text("  lead"), "  lead");
    assert_eq!(clean_paste_text("ñ\u{7}x"), "ñ x");
    let cb = fake("one\ntwo\n");
    let mut k = keys_with(&cb);
    tap(&mut k, KeyInput::ch(':'));
    tap(&mut k, KeyInput::ch('v').ctrl());
    assert_eq!(k.line.text, "one two");
}

#[test]
fn c6_paste_empty_or_unreadable_changes_nothing() {
    for cb in [fake(""), fake("\n \n"), unreachable_clipboard()] {
        let mut k = keys_with(&cb);
        tap(&mut k, KeyInput::ch(':'));
        type_str(&mut k, "go");
        tap(&mut k, KeyInput::ch('v').ctrl());
        assert_eq!(k.line.text, "go");
        assert_eq!(k.vim_mode, VimMode::Command);
        assert!(k.message.is_empty(), "{}", k.message);
    }
}

#[test]
fn c6_paste_in_input_reaches_the_page() {
    // INPUT: Ctrl-v is the page's (the compositor offers it the desktop's clipboard first).
    let cb = fake("x");
    let mut k = keys_with(&cb);
    k.set_key_mode(KeyMode::Input);
    for key in paste_keys() {
        assert!(is_paste_chord(&key));
        assert!(tap(&mut k, key), "{key:?}");
    }
    assert!(!is_paste_chord(&KeyInput::ch('v')));
    assert!(!is_paste_chord(&KeyInput::sym(Keysym::Insert)));
}

// ---------- the bridge's choice of text type ----------

#[test]
fn c6_bridge_reads_a_pages_copy_as_utf8_text() {
    let m = |v: &[&str]| pick_text_mime(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    // Chromium's offer.
    assert_eq!(
        m(&["text/html", "text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]).as_deref(),
        Some("text/plain;charset=utf-8")
    );
    assert_eq!(m(&["STRING", "UTF8_STRING"]).as_deref(), Some("UTF8_STRING"));
    assert_eq!(m(&["image/png"]), None);
}
