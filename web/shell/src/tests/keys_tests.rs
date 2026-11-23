//! KEYS.md T1-T34, T54-T56 and the commands (C3). Each test names its row(s).

use std::time::{Duration, Instant};

use smithay::input::keyboard::Keysym;

use crate::config::{evdev, ScrollInput, BINDINGS, ESC_ESC_MS, KEY_SEQUENCES, SCROLL_INPUTS};
use crate::keys::*;
use crate::protocol::Command;
use crate::registers::{let_message, parse_let, CLIPBOARD_NOT_YET, LET_USAGE};

const HELP: &str = "file:///run/user/1000/web-1/help.html";

fn keys() -> Keys {
    Keys::new(None, HELP.into(), None)
}

fn esc() -> KeyInput {
    KeyInput::sym(Keysym::Escape)
}

fn ms(base: Instant, n: u64) -> Instant {
    base + Duration::from_millis(n)
}

/// A key's code in these tests: its keysym (press and release only have to agree).
fn code(k: &KeyInput) -> u32 {
    k.sym.raw()
}

/// Press: whether the page got it.
fn press(k: &mut Keys, key: KeyInput, t: Instant) -> bool {
    k.key(code(&key), true, &key, t)
}

/// Release: whether the page got it.
fn release(k: &mut Keys, key: KeyInput, t: Instant) -> bool {
    k.key(code(&key), false, &key, t)
}

/// Press and release: (page got the press, page got the release).
fn tap(k: &mut Keys, key: KeyInput, t: Instant) -> (bool, bool) {
    (press(k, key, t), release(k, key, t))
}

fn type_str(k: &mut Keys, s: &str, t: Instant) {
    for c in s.chars() {
        let key = if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) };
        tap(k, key, t);
    }
}

fn effects(k: &mut Keys) -> Vec<Effect> {
    std::mem::take(&mut k.effects)
}

fn input_mode() -> Keys {
    let mut k = keys();
    k.set_key_mode(KeyMode::Input);
    k
}

// ---------- T1-T5: INPUT and ESC1 ----------

#[test]
fn keys_t01_t02_esc_esc_within_300ms_enters() {
    let t = Instant::now();
    let (s, enter) = resolve_esc_esc(EscState::Idle, &esc(), false, t);
    assert_eq!((s, enter), (EscState::Esc1(t), false));
    let (s, enter) = resolve_esc_esc(s, &esc(), false, ms(t, ESC_ESC_MS));
    assert_eq!((s, enter), (EscState::Idle, true));
}

#[test]
fn keys_t03_slow_second_esc_is_a_new_first_esc() {
    let t = Instant::now();
    let (s, _) = resolve_esc_esc(EscState::Idle, &esc(), false, t);
    let late = ms(t, ESC_ESC_MS + 1);
    let (s, enter) = resolve_esc_esc(s, &esc(), false, late);
    assert_eq!((s, enter), (EscState::Esc1(late), false));
    let (_, enter) = resolve_esc_esc(s, &esc(), false, ms(late, 10));
    assert!(enter);
}

#[test]
fn keys_t04_other_key_or_repeat_or_modified_esc_resets() {
    let t = Instant::now();
    let armed = EscState::Esc1(t);
    assert_eq!(
        resolve_esc_esc(armed, &KeyInput::ch('a'), false, ms(t, 10)),
        (EscState::Idle, false)
    );
    assert_eq!(resolve_esc_esc(armed, &esc(), true, ms(t, 10)), (EscState::Idle, false));
    assert_eq!(resolve_esc_esc(armed, &esc().ctrl(), false, ms(t, 10)), (EscState::Idle, false));
    assert_eq!(resolve_esc_esc(armed, &esc().alt(), false, ms(t, 10)), (EscState::Idle, false));
    // Shift is allowed (only Ctrl, Alt and Super disqualify).
    assert_eq!(resolve_esc_esc(armed, &esc().shift(), false, ms(t, 10)), (EscState::Idle, true));
    // In the key model: Esc, `a`, Esc is no Esc Esc.
    let mut k = input_mode();
    assert!(press(&mut k, esc(), t));
    release(&mut k, esc(), t);
    tap(&mut k, KeyInput::ch('a'), ms(t, 50));
    assert!(press(&mut k, esc(), ms(t, 100)), "a new first Esc reaches the page");
    assert_eq!(k.key_mode, KeyMode::Input);
}

#[test]
fn keys_t01_t02_t34_first_esc_reaches_the_page_second_and_its_release_do_not() {
    let t = Instant::now();
    let mut k = input_mode();
    assert_eq!(tap(&mut k, esc(), t), (true, true), "T1: the page gets the first Esc");
    assert_eq!(k.key_mode, KeyMode::Input, "ESC1 shows as INPUT");
    assert_eq!(k.bar_view().mode, "INPUT");
    assert_eq!(
        tap(&mut k, esc(), ms(t, 120)),
        (false, false),
        "T2 + T34: neither press nor release"
    );
    assert_eq!(k.key_mode, KeyMode::Normal);
}

#[test]
fn keys_t05_input_mode_forwards_everything_and_runs_no_binding() {
    let t = Instant::now();
    let mut k = input_mode();
    for key in [
        KeyInput::ch('j'),
        KeyInput::ch('l'),
        KeyInput::ch(':'),
        KeyInput::ch('d').ctrl(),
        KeyInput::ch('g'),
        KeyInput::ch('g'),
        KeyInput::sym(Keysym::Tab),
        KeyInput::sym(Keysym::Return),
    ] {
        assert_eq!(tap(&mut k, key, t), (true, true), "{key:?}");
    }
    assert!(effects(&mut k).is_empty());
    assert_eq!(k.key_mode, KeyMode::Input);
}

#[test]
fn keys_start_mode_is_normal() {
    // scratchy QKEYS5 (user 2026-11-21): a fresh window starts in NORMAL.
    let k = keys();
    assert_eq!(k.key_mode, KeyMode::Normal);
    assert_eq!(k.bar_view().mode, "NORMAL");
}

// ---------- T6-T21: NORMAL and G ----------

#[test]
fn keys_t06_t34_i_enters_input_and_its_release_is_not_delivered() {
    let t = Instant::now();
    let mut k = keys();
    assert!(!press(&mut k, KeyInput::ch('i'), t));
    assert_eq!(k.key_mode, KeyMode::Input);
    assert!(!release(&mut k, KeyInput::ch('i'), t), "T34: no keyup without its keydown");
    assert_eq!(tap(&mut k, KeyInput::ch('i'), t), (true, true), "the next `i` is the page's");
}

#[test]
fn keys_t07_esc_in_normal_stays_normal() {
    let t = Instant::now();
    let mut k = keys();
    assert_eq!(tap(&mut k, esc(), t), (false, false));
    assert_eq!(tap(&mut k, esc(), ms(t, 50)), (false, false));
    assert_eq!(k.key_mode, KeyMode::Normal);
    assert!(effects(&mut k).is_empty());
}

#[test]
fn keys_t09_colon_opens_the_command_line() {
    let mut k = keys();
    tap(&mut k, KeyInput::ch(':'), Instant::now());
    assert_eq!(k.vim_mode, VimMode::Command);
    let v = k.bar_view();
    assert_eq!(v.line, Some((':', String::new(), 0)), "the line takes the bar");
}

#[test]
fn keys_t10_t11_gg_scrolls_to_the_top() {
    let t = Instant::now();
    let mut k = keys();
    tap(&mut k, KeyInput::ch('g'), t);
    assert!(effects(&mut k).is_empty(), "T10: one g does nothing");
    assert_eq!(k.bar_view().mode, "NORMAL", "G shows as NORMAL");
    let g = KeyInput::ch('g');
    press(&mut k, g, t);
    assert_eq!(effects(&mut k), vec![Effect::KeyDown(evdev::KEY_HOME)]);
    release(&mut k, g, t);
    assert_eq!(effects(&mut k), vec![Effect::KeyUp(evdev::KEY_HOME)]);
}

#[test]
fn keys_t12_g_then_another_key_is_handled_as_in_normal() {
    let t = Instant::now();
    let mut k = keys();
    tap(&mut k, KeyInput::ch('g'), t);
    tap(&mut k, KeyInput::ch('i'), t);
    assert_eq!(k.key_mode, KeyMode::Input);
    let mut k = keys();
    tap(&mut k, KeyInput::ch('g'), t);
    tap(&mut k, KeyInput::ch(':'), t);
    assert_eq!(k.vim_mode, VimMode::Command);
    let mut k = keys();
    tap(&mut k, KeyInput::ch('g'), t);
    tap(&mut k, KeyInput::ch('j'), t);
    assert_eq!(
        effects(&mut k),
        vec![Effect::KeyDown(evdev::KEY_DOWN), Effect::KeyUp(evdev::KEY_DOWN)]
    );
    // `g` `x` `g` is not gg.
    let mut k = keys();
    for c in ['g', 'x', 'g'] {
        tap(&mut k, KeyInput::ch(c), t);
    }
    assert!(effects(&mut k).is_empty());
}

#[test]
fn keys_t13_scroll_keys_hold_a_real_key_until_released() {
    let t = Instant::now();
    let cases: [(KeyInput, u32); 12] = [
        (KeyInput::ch('j'), evdev::KEY_DOWN),
        (KeyInput::ch('k'), evdev::KEY_UP),
        (KeyInput::ch('d').ctrl(), evdev::KEY_PAGEDOWN),
        (KeyInput::ch('u').ctrl(), evdev::KEY_PAGEUP),
        (KeyInput::ch('G'), evdev::KEY_END),
        (KeyInput::sym(Keysym::Home), evdev::KEY_HOME),
        (KeyInput::sym(Keysym::End), evdev::KEY_END),
        (KeyInput::sym(Keysym::Down), evdev::KEY_DOWN),
        (KeyInput::sym(Keysym::space), evdev::KEY_PAGEDOWN),
        (KeyInput::sym(Keysym::space).shift(), evdev::KEY_PAGEUP),
        (KeyInput::sym(Keysym::Page_Up), evdev::KEY_PAGEUP),
        (KeyInput::sym(Keysym::Left), evdev::KEY_LEFT),
    ];
    for (key, want) in cases {
        let mut k = keys();
        assert!(!press(&mut k, key, t), "{key:?}: the bound key itself never reaches the page");
        assert_eq!(effects(&mut k), vec![Effect::KeyDown(want)], "{key:?}");
        assert_eq!(k.held(), &[(code(&key), want)]);
        // Held: an OS repeat of the bound key presses nothing more (the engine repeats).
        press(&mut k, key, ms(t, 500));
        assert!(effects(&mut k).is_empty(), "{key:?}");
        assert!(!release(&mut k, key, ms(t, 600)));
        assert_eq!(effects(&mut k), vec![Effect::KeyUp(want)], "{key:?}");
        assert!(k.held().is_empty());
    }
}

#[test]
fn keys_t13_the_scroll_table_has_a_row_for_every_binding_and_held_keys_survive_a_mode_change() {
    for b in BINDINGS {
        if let Action::Scroll(i) = b.action {
            assert!(i < SCROLL_INPUTS.len(), "{b:?}");
        }
    }
    assert!(SCROLL_INPUTS.iter().all(|s| matches!(s, ScrollInput::Key(_))));
    // `j` held, then `:` opens the line: releasing `j` still releases the page's Down.
    let t = Instant::now();
    let mut k = keys();
    press(&mut k, KeyInput::ch('j'), t);
    tap(&mut k, KeyInput::ch(':'), t);
    effects(&mut k);
    release(&mut k, KeyInput::ch('j'), t);
    assert_eq!(effects(&mut k), vec![Effect::KeyUp(evdev::KEY_DOWN)]);
}

#[test]
fn keys_t14_h_and_l_go_back_and_forward_not_on_repeat() {
    let t = Instant::now();
    let mut k = keys();
    tap(&mut k, KeyInput::ch('h'), t);
    tap(&mut k, KeyInput::ch('l'), t);
    assert_eq!(effects(&mut k), vec![Effect::Send(Command::Back), Effect::Send(Command::Forward)]);
    // Held `h`: a second press without a release is an OS repeat and does nothing.
    press(&mut k, KeyInput::ch('h'), t);
    press(&mut k, KeyInput::ch('h'), ms(t, 400));
    press(&mut k, KeyInput::ch('h'), ms(t, 430));
    assert_eq!(effects(&mut k), vec![Effect::Send(Command::Back)]);
    assert!(action_ignores_repeat(Action::HistoryBack) && action_ignores_repeat(Action::HistoryForward));
}

#[test]
fn keys_t20_enter_in_normal_does_nothing() {
    let mut k = keys();
    assert_eq!(tap(&mut k, KeyInput::sym(Keysym::Return), Instant::now()), (false, false));
    assert!(effects(&mut k).is_empty());
    assert!(k.message.is_empty());
}

#[test]
fn keys_t21_t33_normal_mode_the_page_sees_no_key_nor_release() {
    let t = Instant::now();
    let mut k = keys();
    for key in [
        KeyInput::ch('x'),
        KeyInput::sym(Keysym::Tab),
        KeyInput::sym(Keysym::Shift_L),
        KeyInput::sym(Keysym::Control_L),
        KeyInput::ch('a').ctrl(),
        KeyInput::sym(Keysym::F5),
    ] {
        assert_eq!(tap(&mut k, key, t), (false, false), "{key:?}");
    }
    assert!(effects(&mut k).is_empty());
    assert_eq!(k.key_mode, KeyMode::Normal);
}

#[test]
fn keys_t33_a_release_follows_its_press_across_a_mode_change() {
    // A key pressed in INPUT (the page saw it go down) and released after Esc Esc: the page
    // gets the release, so nothing stays pressed in the page.
    let t = Instant::now();
    let mut k = input_mode();
    let a = KeyInput::ch('a');
    assert!(press(&mut k, a, t));
    tap(&mut k, esc(), ms(t, 10));
    tap(&mut k, esc(), ms(t, 20));
    assert_eq!(k.key_mode, KeyMode::Normal);
    assert!(release(&mut k, a, ms(t, 30)));
    // And the other way: pressed in NORMAL, released in INPUT: not delivered (T34).
    let mut k = keys();
    let shift = KeyInput::sym(Keysym::Shift_L);
    assert!(!press(&mut k, shift, t));
    k.user_focused_control();
    assert!(!release(&mut k, shift, t));
}

// ---------- T26-T28: COMMAND ----------

fn in_line() -> Keys {
    let mut k = keys();
    tap(&mut k, KeyInput::ch(':'), Instant::now());
    k
}

#[test]
fn keys_t26_editing_keys_edit_the_line() {
    let t = Instant::now();
    let mut k = in_line();
    type_str(&mut k, "go exa mple", t);
    assert_eq!(k.line.text, "go exa mple");
    tap(&mut k, KeyInput::sym(Keysym::BackSpace), t);
    assert_eq!(k.line.text, "go exa mpl");
    tap(&mut k, KeyInput::sym(Keysym::Left), t);
    tap(&mut k, KeyInput::sym(Keysym::Left), t);
    type_str(&mut k, "X", t);
    assert_eq!(k.line.text, "go exa mXpl");
    tap(&mut k, KeyInput::sym(Keysym::End), t);
    tap(&mut k, KeyInput::ch('w').ctrl(), t);
    assert_eq!(k.line.text, "go exa ", "Ctrl-w deletes the word before the cursor");
    tap(&mut k, KeyInput::ch('w').ctrl(), t);
    assert_eq!(k.line.text, "go ");
    tap(&mut k, KeyInput::sym(Keysym::Home), t);
    assert_eq!(k.line.cursor, 0);
    tap(&mut k, KeyInput::sym(Keysym::End), t);
    tap(&mut k, KeyInput::ch('u').ctrl(), t);
    assert_eq!(k.line.text, "", "Ctrl-u deletes to the start");
    // Non-ASCII text and the caret stay on char boundaries.
    type_str(&mut k, "ñé", t);
    tap(&mut k, KeyInput::sym(Keysym::BackSpace), t);
    assert_eq!(k.line.text, "ñ");
    // Ctrl/Alt chords type nothing; paste says it needs C6.
    tap(&mut k, KeyInput::ch('k').ctrl(), t);
    tap(&mut k, KeyInput::ch('k').alt(), t);
    assert_eq!(k.line.text, "ñ");
    tap(&mut k, KeyInput::ch('v').ctrl(), t);
    assert_eq!(k.message, NOT_YET_PASTE);
    assert_eq!(k.line.text, "ñ");
    tap(&mut k, KeyInput::sym(Keysym::Insert).shift(), t);
    assert_eq!(k.line.text, "ñ");
    assert_eq!(k.vim_mode, VimMode::Command);
}

#[test]
fn keys_t27_enter_runs_the_command() {
    let t = Instant::now();
    let mut k = in_line();
    type_str(&mut k, "go example.com/a", t);
    // Delete hides any ghost so exactly the typed text runs.
    tap(&mut k, KeyInput::sym(Keysym::Delete), t);
    tap(&mut k, KeyInput::sym(Keysym::Return), t);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert_eq!(k.key_mode, KeyMode::Normal);
    assert_eq!(
        effects(&mut k),
        vec![Effect::Send(Command::Open {
            url: "https://example.com/a".into()
        })]
    );
    // An empty line runs nothing.
    let mut k = in_line();
    tap(&mut k, KeyInput::sym(Keysym::Return), t);
    assert!(effects(&mut k).is_empty() && k.message.is_empty());
}

#[test]
fn keys_t28_esc_cancels_the_line() {
    let t = Instant::now();
    let mut k = in_line();
    type_str(&mut k, "q", t);
    assert_eq!(tap(&mut k, esc(), t), (false, false));
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.line.text.is_empty());
    assert!(effects(&mut k).is_empty(), "nothing ran");
    assert_eq!(k.bar_view().line, None);
}

// ---------- T29-T32: events that are not keys ----------

#[test]
fn keys_t29_a_click_into_a_field_enters_input_and_cancels_the_line() {
    let t = Instant::now();
    let mut k = in_line();
    type_str(&mut k, "go x", t);
    k.user_focused_control();
    assert_eq!(k.key_mode, KeyMode::Input);
    assert_eq!(k.vim_mode, VimMode::Normal);
    assert!(k.line.text.is_empty());
    assert!(effects(&mut k).is_empty(), "the cancelled line did not run");
    // The decision: a focus change within CLICK_FOCUS_MS of a mouse press is the user's.
    assert!(click_recent(Some(t), ms(t, 200)));
    assert!(click_recent(Some(t), ms(t, crate::config::CLICK_FOCUS_MS)));
}

#[test]
fn keys_t31_navigation_keeps_the_mode() {
    for mode in [KeyMode::Input, KeyMode::Normal] {
        let mut k = keys();
        k.set_key_mode(mode);
        k.url = "https://a.test/".into();
        k.url = "https://b.test/".into();
        assert_eq!(k.key_mode, mode);
    }
    // An open line stays open across a navigation.
    let mut k = in_line();
    type_str(&mut k, "go", Instant::now());
    k.url = "https://c.test/".into();
    assert_eq!(k.vim_mode, VimMode::Command);
    assert_eq!(k.line.text, "go");
}

#[test]
fn keys_t32_a_page_focus_enters_input_and_a_blur_changes_nothing() {
    // T32 (user, 2026-11-23): `editable-focused true` is `user_focused_control` whatever caused it, so
    // the decision does not depend on a click. A blur has no key-model call: the mode stays.
    let mut k = keys();
    k.set_key_mode(KeyMode::Normal);
    k.user_focused_control();
    assert_eq!(k.key_mode, KeyMode::Input);
    k.set_key_mode(KeyMode::Normal); // the user left the field with Esc Esc; a later blur leaves NORMAL alone
    assert_eq!(k.key_mode, KeyMode::Normal);
    // The popup half still depends on a click: an old one is the page's own popup.
    let t = Instant::now();
    assert!(!click_recent(None, t));
    assert!(!click_recent(Some(t), ms(t, crate::config::CLICK_FOCUS_MS + 1)), "an old click");
}

// ---------- T54-T56: yy (C6 stub) ----------

#[test]
fn keys_t54_t55_yy_says_it_needs_the_clipboard_and_the_page_sees_nothing() {
    let t = Instant::now();
    let mut k = keys();
    k.url = "https://example.com/".into();
    assert_eq!(tap(&mut k, KeyInput::ch('y'), t), (false, false));
    assert!(k.message.is_empty(), "T54: one y does nothing");
    assert_eq!(tap(&mut k, KeyInput::ch('y'), t), (false, false));
    assert_eq!(k.message, CLIPBOARD_NOT_YET);
    assert!(KEY_SEQUENCES.iter().any(|s| s.first == 'g' && s.second == 'g'));
    assert!(KEY_SEQUENCES
        .iter()
        .any(|s| s.first == 'y' && s.second == 'y' && s.action == Action::YankUrl));
    // In INPUT `yy` is the page's.
    let mut k = input_mode();
    assert_eq!(tap(&mut k, KeyInput::ch('y'), t), (true, true));
    assert_eq!(tap(&mut k, KeyInput::ch('y'), t), (true, true));
    assert!(k.message.is_empty());
}

#[test]
fn keys_t55_yy_ignores_key_repeat() {
    let t = Instant::now();
    let mut k = keys();
    k.url = "https://example.com/".into();
    press(&mut k, KeyInput::ch('y'), t);
    press(&mut k, KeyInput::ch('y'), ms(t, 400)); // an OS repeat of the held y
    assert!(k.message.is_empty());
}

#[test]
fn keys_t56_g_y_and_y_g_rearm() {
    let t = Instant::now();
    let mut k = keys();
    k.url = "https://example.com/".into();
    for c in ['g', 'y', 'y'] {
        tap(&mut k, KeyInput::ch(c), t);
    }
    assert_eq!(k.message, CLIPBOARD_NOT_YET, "g y leaves y pending");
    let mut k = keys();
    for c in ['y', 'g', 'g'] {
        tap(&mut k, KeyInput::ch(c), t);
    }
    assert_eq!(
        effects(&mut k),
        vec![Effect::KeyDown(evdev::KEY_HOME), Effect::KeyUp(evdev::KEY_HOME)]
    );
    assert!(k.message.is_empty());
}

// ---------- commands ----------

fn run(k: &mut Keys, line: &str) -> Vec<Effect> {
    k.run_command_line(line);
    effects(k)
}

#[test]
fn keys_go_assumes_https_like_scratchy_qcomp6() {
    let cur = "https://cur.test/dir/page.html?x#f";
    for (word, want) in [
        ("google.com/", "https://google.com/"),
        ("localhost:8080", "https://localhost:8080"),
        ("foo.html", "https://foo.html"),
        ("http://plain.test/", "http://plain.test/"),
        ("https://x.test/a b", "https://x.test/a b"),
        ("about:blank", "about:blank"),
        ("file:///tmp/x.html", "file:///tmp/x.html"),
        ("data:text/html,hi", "data:text/html,hi"),
        ("/root", "https://cur.test/root"),
        ("./same", "https://cur.test/dir/same"),
        ("../up", "https://cur.test/up"),
        ("#frag", "https://cur.test/dir/page.html?x#frag"),
        ("?q=1", "https://cur.test/dir/page.html?q=1"),
    ] {
        assert_eq!(go_url(word, cur).as_deref(), Ok(want), "{word}");
    }
    assert!(go_url("/x", "").is_err(), "relative with no page");
    let mut k = keys();
    k.url = cur.into();
    assert_eq!(
        run(&mut k, "go  example.com "),
        vec![Effect::Send(Command::Open {
            url: "https://example.com".into()
        })]
    );
    assert!(run(&mut k, "go").is_empty());
    assert!(k.message.contains("usage"), "{}", k.message);
    assert!(run(&mut k, "go a b").is_empty());
}

#[test]
fn keys_refresh_history_help_quit_and_unknown_commands() {
    let mut k = keys();
    assert_eq!(run(&mut k, "refresh"), vec![Effect::Send(Command::Reload { hard: false })]);
    assert_eq!(run(&mut k, "refresh hard"), vec![Effect::Send(Command::Reload { hard: true })]);
    assert_eq!(run(&mut k, "historyBack"), vec![Effect::Send(Command::Back)]);
    assert_eq!(run(&mut k, "historyFront"), vec![Effect::Send(Command::Forward)]);
    assert_eq!(run(&mut k, "help"), vec![Effect::Send(Command::Open { url: HELP.into() })]);
    // On the help page `:q` goes back to the page before it; elsewhere it quits.
    k.url = HELP.into();
    assert_eq!(run(&mut k, "q"), vec![Effect::Send(Command::Back)]);
    k.url = "https://example.com/".into();
    assert_eq!(run(&mut k, "q"), vec![Effect::Quit]);
    assert_eq!(run(&mut k, "quit"), vec![Effect::Quit]);
    assert!(run(&mut k, "refresh soft").is_empty());
    assert_eq!(k.message, "web: unknown command: :refresh soft");
    assert!(run(&mut k, "bogus").is_empty());
    assert_eq!(k.message, "web: unknown command: :bogus");
    // Every name the first word completes from is a command.
    for name in COMMAND_NAMES {
        let mut k = keys();
        k.run_command_line(name);
        assert!(!k.message.contains("unknown command"), "{name}: {}", k.message);
    }
}

#[test]
fn keys_let_reads_the_url_register_and_reports_errors() {
    assert_eq!(parse_let("@+ = @%"), Some(('+', '%')));
    assert_eq!(parse_let(" @+=@% "), Some(('+', '%')));
    assert_eq!(parse_let("@+ = @% x"), None);
    assert_eq!(parse_let("@+ = document.title"), None);
    let url = "https://example.com/";
    assert_eq!(let_message("@+ = @%", url), CLIPBOARD_NOT_YET);
    assert_eq!(let_message("@+ = @%", ""), "web: no URL to copy");
    assert_eq!(let_message("@% = @+", url), "web: register @% is read-only");
    assert_eq!(let_message("@x = @%", url), "web: no register @x");
    assert_eq!(let_message("@+ = @q", url), "web: no register @q");
    assert_eq!(let_message("+ = %", url), LET_USAGE);
    let mut k = keys();
    k.url = url.into();
    assert!(run(&mut k, "let @+ = @%").is_empty());
    assert_eq!(k.message, CLIPBOARD_NOT_YET);
}

#[test]
fn keys_bar_view_shows_mode_url_message_or_the_line() {
    let t = Instant::now();
    let mut k = keys();
    k.url = "https://example.com/".into();
    k.message = "hello".into();
    let v = k.bar_view();
    assert_eq!(
        (v.mode.as_str(), v.url.as_str(), v.message.as_str(), v.line),
        ("NORMAL", "https://example.com/", "hello", None)
    );
    tap(&mut k, KeyInput::ch(':'), t);
    type_str(&mut k, "hel", t);
    let v = k.bar_view();
    assert_eq!(v.line, Some((':', "hel".to_string(), 3)));
    assert_eq!(v.ghost, "p");
}
