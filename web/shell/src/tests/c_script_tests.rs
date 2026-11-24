//! C-SCRIPT (SCRIPTING.md, KEYS.md "User commands"): init.js, `web.cmd`, `web.command`, `web.on("ready")`.

use std::time::{Duration, Instant};

use smithay::input::keyboard::Keysym;

use crate::buffers::TabCreated;
use crate::completion::{compute_ghost, CompleteCache};
use crate::config::{SCRIPT_BUDGET_MS, SCRIPT_DEPTH_MAX};
use crate::keys::*;
use crate::protocol::Command;
use crate::script::{init_files, stack_location, valid_command_name, ScriptFile};

const BUDGET: Duration = Duration::from_millis(SCRIPT_BUDGET_MS);

fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

/// Types `:line` (Delete hides a ghost) and Enter.
fn type_line(k: &mut Keys, line: &str) {
    press(k, KeyInput::ch(':'));
    for c in line.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
    press(k, KeyInput::sym(Keysym::Delete));
    press(k, KeyInput::sym(Keysym::Return));
}

fn sent(k: &mut Keys) -> Vec<Command> {
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter_map(|e| match e {
            Effect::Send(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn logs(k: &Keys) -> Vec<String> {
    k.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Log(l) => Some(l.clone()),
            _ => None,
        })
        .collect()
}

/// A shell whose first tab exists (what `ready` waits for), with `src` loaded as init.js.
fn shell(src: &str) -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    k.load_script_source("init.js", src, BUDGET);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = "https://a.test/".into();
    k
}

fn new_tab(url: &str, req: u64) -> Command {
    Command::NewTab {
        req,
        url: Some(url.into()),
    }
}

#[test]
fn c_script_ready_opens_buffers_with_web_cmd() {
    let mut k = shell(
        r#"
        web.on("ready", () => {
            web.cmd("new http://127.0.0.1:1/one");
            web.cmd(":new http://127.0.0.1:1/two");
        });
        "#,
    );
    assert_eq!(k.message, "");
    k.effects.clear();
    assert_eq!(k.buffers.list.len(), 1, "nothing runs before ready");
    k.script_ready();
    assert_eq!(
        sent(&mut k),
        vec![new_tab("http://127.0.0.1:1/one", 1), new_tab("http://127.0.0.1:1/two", 2)]
    );
    assert_eq!(k.buffers.list.len(), 3);
    assert_eq!(k.buffers.current, 3, "as :new: the last one is current");
    // Once per start.
    k.script_ready();
    assert!(sent(&mut k).is_empty());
    assert_eq!(k.buffers.list.len(), 3);
}

#[test]
fn c_script_top_level_cmd_waits_for_ready() {
    let mut k = shell(r#"web.cmd("new http://127.0.0.1:1/x");"#);
    assert!(sent(&mut k).is_empty(), "no host yet: held");
    k.script_ready();
    assert_eq!(sent(&mut k), vec![new_tab("http://127.0.0.1:1/x", 1)]);
}

#[test]
fn c_script_throwing_script_keeps_what_it_set_up_and_reports_line() {
    let mut k = shell("web.command(\"Early\", () => web.cmd(\"go /early\"));\n\nnull.boom;\nweb.command(\"Late\", () => {});\n");
    assert!(k.message.starts_with("init.js:3: "), "{}", k.message);
    assert!(
        logs(&k).iter().any(|l| l.starts_with("script error: init.js:3: ")),
        "{:?}",
        logs(&k)
    );
    assert_eq!(k.user_command_names(), vec!["Early".to_string()], "what ran before the error stays");
    k.effects.clear();
    type_line(&mut k, "Early");
    assert_eq!(
        sent(&mut k),
        vec![Command::Open {
            url: "https://a.test/early".into()
        }]
    );
    type_line(&mut k, "Late");
    assert!(k.message.starts_with("web: unknown command: :Late"), "{}", k.message);
}

#[test]
fn c_script_syntax_error_reports_line() {
    let k = shell("web.command(\"A\", () => {});\nlet x = ;\n");
    assert!(k.message.starts_with("init.js:2: "), "{}", k.message);
    // A syntax error stops the whole file before it runs (the file is one script).
    assert!(k.user_command_names().is_empty());
}

#[test]
fn c_script_strict_mode() {
    let k = shell("undeclared = 1;\n");
    assert!(k.message.starts_with("init.js:1: "), "sloppy mode would allow this: {}", k.message);
}

#[test]
fn c_script_error_in_user_command_and_ready_shows_line() {
    let mut k = shell("web.command(\"Bad\", () => {\n  throw new Error(\"nope\");\n});\nweb.on(\"ready\", () => { throw \"plain\"; });\n");
    assert_eq!(k.message, "");
    type_line(&mut k, "Bad");
    assert_eq!(k.message, "init.js:2: nope");
    k.script_ready();
    assert_eq!(k.message, "init.js: plain");
}

#[test]
fn c_script_runaway_loop_is_interrupted() {
    let start = Instant::now();
    let mut k = shell("web.command(\"Spin\", () => { for (;;) {} });\nwhile (true) {}\n");
    assert!(start.elapsed() < BUDGET * 20, "{:?}", start.elapsed());
    assert!(k.message.contains("interrupted"), "{}", k.message);
    // QuickJS gives an interrupted call the line its function (here the file) starts on, not the loop's.
    assert!(k.message.starts_with("init.js:1: "), "{}", k.message);
    k.message.clear();
    let start = Instant::now();
    type_line(&mut k, "Spin");
    assert!(start.elapsed() < BUDGET * 20);
    assert!(k.message.starts_with("init.js:1: interrupted"), "{}", k.message);
    // The runtime still works afterwards.
    k.load_script_source("init.js", "web.command(\"Ok\", () => web.cmd(\"go /ok\"));", BUDGET);
    k.effects.clear();
    type_line(&mut k, "Ok");
    assert_eq!(
        sent(&mut k),
        vec![Command::Open {
            url: "https://a.test/ok".into()
        }]
    );
}

#[test]
fn c_script_depth_limit() {
    let mut k = shell(r#"web.command("Loop", () => { web.cmd("Loop"); web.cmd("go /after"); });"#);
    k.effects.clear();
    let start = Instant::now();
    type_line(&mut k, "Loop");
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(k.message.contains(&format!("deeper than {SCRIPT_DEPTH_MAX}")), "{}", k.message);
    assert!(sent(&mut k).is_empty(), "the whole chain stopped");
    // The next command works again.
    let mut k2 = shell(r#"web.command("A", () => web.cmd("B")); web.command("B", (a) => web.cmd("go /" + a.length));"#);
    k2.effects.clear();
    type_line(&mut k2, "A");
    assert_eq!(
        sent(&mut k2),
        vec![Command::Open {
            url: "https://a.test/0".into()
        }]
    );
    type_line(&mut k, "Loop");
    assert!(k.message.contains("deeper than"), "still an error, not stuck");
}

#[test]
fn c_script_order_is_as_if_web_cmd_ran_at_once() {
    let mut k = shell(
        r#"web.command("C", () => web.cmd("go /c"));
           web.command("A", () => { web.cmd("go /a"); web.cmd("C"); web.cmd("go /b"); });"#,
    );
    k.effects.clear();
    type_line(&mut k, "A");
    let urls: Vec<String> = sent(&mut k)
        .into_iter()
        .map(|c| match c {
            Command::Open { url } => url,
            o => panic!("{o:?}"),
        })
        .collect();
    assert_eq!(urls, ["https://a.test/a", "https://a.test/c", "https://a.test/b"]);
}

#[test]
fn c_script_user_command_args_split_like_builtins() {
    let mut k = shell(r#"web.command("Echo", (args) => web.cmd("go /" + args.length + "-" + args.join("+")));"#);
    k.effects.clear();
    type_line(&mut k, "Echo a  b");
    assert_eq!(
        sent(&mut k),
        vec![Command::Open {
            url: "https://a.test/2-a+b".into()
        }]
    );
    k.run_command_line("Echo");
    assert_eq!(
        sent(&mut k),
        vec![Command::Open {
            url: "https://a.test/0-".into()
        }]
    );
}

#[test]
fn c_script_uppercase_name_rule_and_redefinition() {
    assert!(valid_command_name("Foo"));
    assert!(valid_command_name("Ü"));
    assert!(!valid_command_name("foo"));
    assert!(!valid_command_name(""));
    assert!(!valid_command_name("_Foo"));
    assert!(!valid_command_name("Foo bar"));
    let mut k = shell("web.command(\"go\", () => {});\n");
    assert!(
        k.message
            .starts_with("init.js:1: web.command: a command name starts with an uppercase"),
        "{}",
        k.message
    );
    assert!(k.user_command_names().is_empty());
    // A redefinition replaces.
    k.load_script_source(
        "init.js",
        r#"web.command("X", () => web.cmd("go /one")); web.command("X", () => web.cmd("go /two"));"#,
        BUDGET,
    );
    k.effects.clear();
    type_line(&mut k, "X");
    assert_eq!(
        sent(&mut k),
        vec![Command::Open {
            url: "https://a.test/two".into()
        }]
    );
    // Built-ins still win for their own names (lowercase), and errors from web.cmd'd lines are the command's.
    k.load_script_source("init.js", r#"web.command("Y", () => web.cmd("go"));"#, BUDGET);
    type_line(&mut k, "Y");
    assert_eq!(k.message, "web: usage: :go <url>");
}

#[test]
fn c_script_only_web_is_exposed() {
    let k = shell(
        r#"
        if (typeof std !== "undefined" || typeof os !== "undefined" || typeof setTimeout !== "undefined"
            || typeof require !== "undefined" || typeof print !== "undefined") throw new Error("leak");
        const keys = Object.keys(web).sort().join(",");
        if (keys !== "cmd,command,on") throw new Error(keys);
        web.extra = 1;
        "#,
    );
    // `web` is frozen: the assignment throws in strict mode, after the checks passed.
    assert!(k.message.starts_with("init.js:6: "), "{}", k.message);
    let k = shell(r#"web.on("load", () => {});"#);
    assert!(k.message.contains("web.on: unknown event"), "{}", k.message);
    // No module loader: a static import is a syntax error in a classic script, a dynamic one rejects
    // (promise jobs run under the budget right after the call).
    let k = shell("import x from \"x\";\n");
    assert!(k.message.starts_with("init.js:1: "), "{}", k.message);
    let mut k = shell(r#"import("x").then(() => web.cmd("go /loaded"), () => web.cmd("go /rejected"));"#);
    k.effects.clear();
    k.script_ready();
    assert_eq!(
        sent(&mut k),
        vec![Command::Open {
            url: "https://a.test/rejected".into()
        }]
    );
}

#[test]
fn c_script_completion_lists_user_commands() {
    let mut k = shell(r#"web.command("Hello", () => {}); web.command("Home", () => {});"#);
    let mut cache = CompleteCache::new(None);
    cache.user_commands = k.user_command_names();
    let g = compute_ghost("H", 1, &cache).expect("a ghost");
    assert_eq!(g.candidates, ["Hello", "Home"]);
    // With the built-ins: `h` offers the built-ins only (names are case-sensitive).
    let g = compute_ghost("h", 1, &cache).expect("a ghost");
    assert!(g.candidates.iter().all(|m| m.starts_with('h')), "{:?}", g.candidates);
    // The `:` line's cache gets them.
    press(&mut k, KeyInput::ch(':'));
    press(&mut k, KeyInput::ch('H'));
    assert_eq!(k.ghost.as_ref().map(|g| g.rest("H")), Some("ello".to_string()));
    press(&mut k, KeyInput::sym(Keysym::Escape));
}

#[test]
fn c_script_missing_file_is_not_an_error_and_paths() {
    let mut k = Keys::new(None, String::new(), None);
    k.load_scripts(&[ScriptFile {
        path: "/nonexistent/c-script/init.js".into(),
        label: "init.js".into(),
    }]);
    assert!(k.scripts.is_none(), "no runtime without a file");
    assert_eq!(k.message, "");
    k.script_ready();
    let env =
        |pairs: &'static [(&'static str, &'static str)]| move |key: &str| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string());
    let f = init_files(&env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")]));
    assert_eq!(
        f,
        vec![ScriptFile {
            path: "/x/web/init.js".into(),
            label: "init.js".into()
        }]
    );
    let f = init_files(&env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")]));
    assert_eq!(f[0].path, std::path::PathBuf::from("/h/.config/web/init.js"));
    let f = init_files(&env(&[("XDG_CONFIG_HOME", "rel"), ("HOME", "/h")]));
    assert_eq!(
        f[0].path,
        std::path::PathBuf::from("/h/.config/web/init.js"),
        "a relative XDG_CONFIG_HOME is ignored"
    );
    assert!(init_files(&env(&[])).is_empty());
}

#[test]
fn c_script_stack_location() {
    assert_eq!(
        stack_location("    at f (init.js:3:5)\n    at <eval> (init.js:9:1)\n").as_deref(),
        Some("init.js:3")
    );
    assert_eq!(
        stack_location("    at cmd (native)\n    at <eval> (init.js:7:2)\n").as_deref(),
        Some("init.js:7")
    );
    assert_eq!(stack_location("    at init.js:2:9\n").as_deref(), Some("init.js:2"));
    assert_eq!(
        stack_location("    at /home/u/.config/web/profiles/p/init.js:4:1\n").as_deref(),
        Some("/home/u/.config/web/profiles/p/init.js:4")
    );
    assert_eq!(stack_location(""), None);
}
