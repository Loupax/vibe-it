//! C-MAPTIMEOUT (SCRIPTING.md "web.settings", KEYS.md T118; DECISIONS.md 2026-11-27, issue #772):
//! `web.settings.mapTimeout`, the wait for the rest of a mapping, default 250 ms.

use std::time::{Duration, Instant};

use crate::config::MAP_TIMEOUT_MS;
use crate::keys::*;
use crate::protocol::Command;

use super::c_script_map_tests::{effects, ms, shell};
use super::c_script_modules_tests::{run, Config};
use super::c_script_tests::{logs, type_line};

/// Taps `c` at `t` (press and release, as the compositor delivers them).
fn tap(k: &mut Keys, c: char, t: Instant) {
    let key = KeyInput::ch(c);
    k.key(key.sym.raw(), true, &key, t);
    k.key(key.sym.raw(), false, &key, t);
}

fn open(url: &str) -> Effect {
    Effect::Send(Command::Open { url: url.into() })
}

const MAP: &str = r#"web.map("n", "gh", ":go http://127.0.0.1:1/h<CR>");"#;

fn wait_after_g(k: &mut Keys, t: Instant) -> Option<Duration> {
    tap(k, 'g', t);
    k.map_wait(t)
}

fn ms_d(n: u64) -> Option<Duration> {
    Some(Duration::from_millis(n))
}

#[test]
fn default_is_250_and_the_getter_says_so() {
    assert_eq!(MAP_TIMEOUT_MS, 250);
    let t = Instant::now();
    let mut k = shell(&format!(
        "{MAP}\nweb.command(\"V\", () => web.cmd(\"echo \" + web.settings.mapTimeout));"
    ));
    assert_eq!(wait_after_g(&mut k, t), ms_d(250));
    k.map_tick(ms(t, 250));
    effects(&mut k);
    assert_eq!(run(&mut k, "V"), "250");
}

#[test]
fn a_set_value_is_the_wait() {
    let t = Instant::now();
    let src = format!("{MAP}\nweb.settings.mapTimeout = 600;");
    let mut k = shell(&src);
    assert_eq!(wait_after_g(&mut k, t), ms_d(600));
    tap(&mut k, 'h', ms(t, 500));
    assert_eq!(effects(&mut k), vec![open("http://127.0.0.1:1/h")]);
    // At 601 ms the wait is over: the keys are handled as without the mapping (`h` is Back; no Open).
    let mut k = shell(&src);
    tap(&mut k, 'g', t);
    k.map_tick(ms(t, 601));
    assert_eq!(k.map_wait(ms(t, 601)), None);
    tap(&mut k, 'h', ms(t, 601));
    assert!(!effects(&mut k).contains(&open("http://127.0.0.1:1/h")));
    // Just before 600 ms it still waits.
    let mut k = shell(&src);
    tap(&mut k, 'g', t);
    assert_eq!(k.map_tick(ms(t, 599)), ms_d(1));
}

#[test]
fn bad_values_throw_a_type_error_and_keep_the_value() {
    for bad in [
        "\"300\"",
        "1.5",
        "0",
        "-1",
        "10001",
        "20000",
        "NaN",
        "Infinity",
        "null",
        "undefined",
    ] {
        let src = format!(
            "web.settings.mapTimeout = 400;\nlet err = null;\ntry {{ web.settings.mapTimeout = {bad}; }} catch (e) {{ err = e; }}\n\
             web.command(\"V\", () => web.cmd(\"echo \" + [err instanceof TypeError, err && err.message, web.settings.mapTimeout].join(\"|\")));"
        );
        let mut k = shell(&src);
        assert_eq!(
            run(&mut k, "V"),
            "true|web.settings.mapTimeout must be an integer from 1 to 10000 (milliseconds)|400",
            "{bad}"
        );
        // Uncaught, it is the file's error.
        let k = shell(&format!("web.settings.mapTimeout = {bad};"));
        assert!(
            k.message.contains("web.settings.mapTimeout must be an integer from 1 to 10000"),
            "{bad}: {}",
            k.message
        );
    }
    let mut k = shell(
        "web.settings.mapTimeout = 1; const a = web.settings.mapTimeout; web.settings.mapTimeout = 10000;\n\
         web.command(\"V\", () => web.cmd(\"echo \" + a + \",\" + web.settings.mapTimeout));",
    );
    assert_eq!(run(&mut k, "V"), "1,10000");
}

#[test]
fn source_puts_it_back_to_the_default() {
    let c = Config::new("maptimeout-source");
    c.write("profiles/p/init.js", &format!("{MAP}\nweb.settings.mapTimeout = 600;\n"));
    let mut k = c.shell("p");
    let t = Instant::now();
    assert_eq!(wait_after_g(&mut k, t), ms_d(600));
    k.map_tick(ms(t, 700));
    c.write("profiles/p/init.js", MAP);
    assert!(run(&mut k, "source").starts_with("sourced"), "{}", k.message);
    let t = ms(t, 5000);
    assert_eq!(wait_after_g(&mut k, t), ms_d(250));
}

#[test]
fn a_change_from_a_mapping_function_affects_the_next_wait() {
    let t = Instant::now();
    let mut k = shell(
        r#"web.map("n", "g", () => { web.settings.mapTimeout = 700; });
        web.map("n", "gg", ":go http://127.0.0.1:1/gg<CR>");"#,
    );
    assert_eq!(wait_after_g(&mut k, t), ms_d(250));
    // The wait runs out, the lone `g` runs and sets 700; the wait already over was 250.
    assert_eq!(k.map_tick(ms(t, 250)), None);
    let t2 = ms(t, 1000);
    assert_eq!(wait_after_g(&mut k, t2), ms_d(700));
}

#[test]
fn settings_is_frozen() {
    let k = shell("web.settings.foo = 1;");
    assert!(k.message.contains("not extensible"), "{}", k.message);
    let k = shell("web.settings = {};");
    assert!(!k.message.is_empty());
    let mut k = shell(
        "let r = [];\ntry { web.settings.foo = 1; } catch (e) { r.push('a'); }\n\
         try { web.settings = {}; } catch (e) { r.push('b'); }\n\
         web.command(\"V\", () => web.cmd(\"echo \" + r.join() + \"|\" + Object.keys(web.settings).join()));",
    );
    assert_eq!(run(&mut k, "V"), "a,b|mapTimeout");
}

#[test]
fn web_api_exports_settings() {
    let t = Instant::now();
    let src = format!(
        "{MAP}\nimport {{ settings }} from \"web:api\";\nsettings.mapTimeout = 500;\n\
         web.command(\"V\", () => web.cmd(\"echo \" + (settings === web.settings)));"
    );
    let mut k = shell(&src);
    assert_eq!(wait_after_g(&mut k, t), ms_d(500));
    k.map_tick(ms(t, 500));
    effects(&mut k);
    type_line(&mut k, "V");
    assert_eq!(k.message, "true", "{:?}", logs(&k));
}
