//! C-SCRIPT-MEM: the script heap limit and unhandled promise rejections.

use std::time::{Duration, Instant};

use crate::config::SCRIPT_BUDGET_MS;
use crate::keys::*;

use super::c_script_tests::{logs, shell, type_line};

const BUDGET: Duration = Duration::from_millis(SCRIPT_BUDGET_MS);
// Grows an array of 800 KB arrays: reaches the limit fast.
const ALLOC: &str = "const a = []; for (;;) { a.push(new Array(100000).fill(1)); }";

fn count(k: &Keys, needle: &str) -> usize {
    logs(k).iter().filter(|l| l.contains(needle)).count()
}

fn sent_any(k: &Keys) -> bool {
    k.effects.iter().any(|e| matches!(e, Effect::Send(_)))
}

#[test]
fn oom_at_top_level_is_an_error_and_runtime_survives() {
    let mut k = shell(&format!("web.command(\"Ok\", () => web.cmd(\"go /ok\"));\n{ALLOC}\n"));
    assert!(k.message.starts_with("init.js:2: "), "{}", k.message);
    assert!(k.message.contains("out of memory"), "{}", k.message);
    assert_eq!(count(&k, "script error: init.js:2"), 1);
    // Still usable: a command defined before the failure runs, and a new file loads.
    k.effects.clear();
    type_line(&mut k, "Ok");
    assert!(sent_any(&k));
    k.message.clear();
    k.load_script_source("b.js", "web.command(\"Two\", () => web.cmd(\"go /two\"));", BUDGET);
    assert_eq!(k.message, "");
    k.effects.clear();
    type_line(&mut k, "Two");
    assert!(sent_any(&k));
}

#[test]
fn oom_in_a_command_then_it_works_again() {
    let mut k = shell(&format!(
        "web.command(\"Eat\", () => {{ {ALLOC} }});\nweb.command(\"Ok\", () => web.cmd(\"go /ok\"));"
    ));
    assert_eq!(k.message, "");
    let start = Instant::now();
    type_line(&mut k, "Eat");
    assert!(start.elapsed() < BUDGET * 20, "{:?}", start.elapsed());
    assert!(k.message.contains("out of memory"), "{}", k.message);
    // The memory is back: the same command fails the same way again, and a small one works.
    k.message.clear();
    type_line(&mut k, "Eat");
    assert!(k.message.contains("out of memory"), "{}", k.message);
    k.effects.clear();
    type_line(&mut k, "Ok");
    assert!(sent_any(&k));
}

#[test]
fn rejected_promise_at_top_level_is_reported_once() {
    let k = shell("\nPromise.reject(new Error(\"boom\"));\n");
    assert_eq!(k.message, "init.js:2: unhandled rejection: boom");
    assert_eq!(count(&k, "script error: "), 1);
}

#[test]
fn rejected_promise_in_async_command_and_ready() {
    let mut k = shell(
        "web.command(\"Bad\", async () => { throw new Error(\"later\"); });\n\
         web.on(\"ready\", async () => { await null; throw new Error(\"in ready\"); });",
    );
    assert_eq!(k.message, "");
    type_line(&mut k, "Bad");
    assert_eq!(k.message, "init.js:1: unhandled rejection: later");
    assert_eq!(count(&k, "unhandled rejection: later"), 1);
    k.script_ready();
    assert_eq!(k.message, "init.js:2: unhandled rejection: in ready");
    assert_eq!(count(&k, "unhandled rejection: in ready"), 1);
}

#[test]
fn handled_rejections_are_not_reported() {
    let k = shell(
        "const p = Promise.reject(new Error(\"x\")); p.catch(() => {});\n\
         (async () => { try { await Promise.reject(1); } catch (e) {} })();",
    );
    assert_eq!(k.message, "");
    assert_eq!(count(&k, "script error"), 0);
}
