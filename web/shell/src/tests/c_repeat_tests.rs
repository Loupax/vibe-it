//! C-REPEAT (KEYS.md T140-T146): vim's `@:`, `@@` and `:@:`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use smithay::input::keyboard::Keysym;

use crate::config::SCRIPT_BUDGET_MS;
use crate::keys::*;
use crate::protocol::Command;

const RELOAD: Effect = Effect::Send(Command::Reload { hard: false });

fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

fn type_str(k: &mut Keys, s: &str) {
    for c in s.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
}

/// Opens `:`, types `text`, Enter.
fn run(k: &mut Keys, text: &str) {
    press(k, KeyInput::ch(':'));
    type_str(k, text);
    press(k, KeyInput::sym(Keysym::Return));
}

fn effects(k: &mut Keys) -> Vec<Effect> {
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter(|e| !matches!(e, Effect::Log(_)))
        .collect()
}

fn open(url: &str) -> Effect {
    Effect::Send(Command::Open { url: url.into() })
}

fn keys() -> Keys {
    Keys::new(None, String::new(), None)
}

fn shell(src: &str) -> Keys {
    let mut k = keys();
    k.load_script_source("init.js", src, Duration::from_millis(SCRIPT_BUDGET_MS));
    k.url = "https://a.test/".into();
    k.effects.clear();
    k
}

#[test]
fn t140_at_colon_runs_the_last_line_again() {
    let mut k = keys();
    run(&mut k, "refresh");
    assert_eq!(effects(&mut k), vec![RELOAD]);
    type_str(&mut k, "@:");
    assert_eq!(effects(&mut k), vec![RELOAD]);
    type_str(&mut k, "@:");
    assert_eq!(effects(&mut k), vec![RELOAD]);
    assert_eq!(k.vim_mode, VimMode::Normal);
}

#[test]
fn t141_no_previous_line_says_so() {
    let mut k = keys();
    type_str(&mut k, "@:");
    assert_eq!(k.message, "web: no previous command line");
    assert!(effects(&mut k).is_empty());
}

#[test]
fn t142_at_at_repeats_the_last_at_colon_and_needs_one() {
    let mut k = keys();
    run(&mut k, "refresh");
    effects(&mut k);
    type_str(&mut k, "@@");
    assert_eq!(k.message, "web: no previously used register");
    assert!(effects(&mut k).is_empty());
    type_str(&mut k, "@:");
    effects(&mut k);
    type_str(&mut k, "@@");
    assert_eq!(effects(&mut k), vec![RELOAD]);
    type_str(&mut k, "@@");
    assert_eq!(effects(&mut k), vec![RELOAD]);
}

#[test]
fn t143_the_repeat_is_not_the_last_line() {
    let mut k = keys();
    run(&mut k, "refresh");
    run(&mut k, "@:"); // `:@:` as a line
    assert_eq!(effects(&mut k), vec![RELOAD, RELOAD]);
    run(&mut k, "@@");
    assert_eq!(effects(&mut k), vec![RELOAD]);
    assert_eq!(k.last_command_line(), Some("refresh"));
    type_str(&mut k, "@:");
    assert_eq!(effects(&mut k), vec![RELOAD]);
    // a new line replaces the last one
    run(&mut k, "go http://127.0.0.1:1/");
    effects(&mut k);
    type_str(&mut k, "@:");
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/")]);
}

#[test]
fn t144_at_then_another_key_is_handled_as_normal() {
    // `@:` consumed the colon: no `:` line opens, and `@` then `x` leaves NORMAL as it was
    let mut k = keys();
    type_str(&mut k, "@:");
    assert_eq!(k.vim_mode, VimMode::Normal);
    let mut k = keys();
    type_str(&mut k, "@x");
    assert_eq!(k.vim_mode, VimMode::Normal);
    // `@` then `/` opens the search line as `/` does alone
    let mut k = keys();
    type_str(&mut k, "@/");
    assert_eq!(k.vim_mode, VimMode::Search);
}

#[test]
fn t147_a_shift_press_between_the_two_keys_keeps_the_sequence() {
    // On a real keyboard `@` and `:` each hold Shift: its own press arrives between them.
    let mut k = keys();
    run(&mut k, "refresh");
    effects(&mut k);
    let shift = KeyInput::sym(Keysym::Shift_L);
    for key in [shift, KeyInput::ch('@'), shift, KeyInput::ch(':')] {
        k.key(key.sym.raw(), true, &key, Instant::now());
        k.key(key.sym.raw(), false, &key, Instant::now());
    }
    assert_eq!(effects(&mut k), vec![RELOAD]);
    assert_eq!(k.vim_mode, VimMode::Normal);
}

#[test]
fn t145_survives_a_restart() {
    let d = std::env::temp_dir().join(format!("web-c-repeat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    {
        let mut k = Keys::new(Some(PathBuf::from(&d)), String::new(), None);
        run(&mut k, "refresh");
    }
    let mut k = Keys::new(Some(PathBuf::from(&d)), String::new(), None);
    effects(&mut k);
    type_str(&mut k, "@:");
    assert_eq!(effects(&mut k), vec![RELOAD]);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn t146_a_repeated_user_command_runs_and_mappings_work() {
    let mut k = shell(
        r#"
        web.command("Hi", (a) => web.cmd("go http://127.0.0.1:1/" + a.join("-")));
        web.map("n", ".", ":@:<CR>");
        web.map("n", "@x", ":refresh<CR>");
        "#,
    );
    assert_eq!(k.message, "");
    run(&mut k, "Hi a b");
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/a-b")]);
    type_str(&mut k, "@:");
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/a-b")]);
    type_str(&mut k, ".");
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/a-b")]);
    type_str(&mut k, ".");
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/a-b")]);
    // a user mapping on `@x` runs; `@:` and `@@` still work beside it
    type_str(&mut k, "@x");
    assert_eq!(effects(&mut k), vec![RELOAD]);
    type_str(&mut k, "@:");
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/a-b")]);
    type_str(&mut k, "@@");
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/a-b")]);
}
