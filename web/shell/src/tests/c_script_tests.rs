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

pub(super) fn press(k: &mut Keys, key: KeyInput) {
    k.key(key.sym.raw(), true, &key, Instant::now());
    k.key(key.sym.raw(), false, &key, Instant::now());
}

/// Types `:line` (Delete hides a ghost) and Enter.
pub(super) fn type_line(k: &mut Keys, line: &str) {
    press(k, KeyInput::ch(':'));
    for c in line.chars() {
        press(k, if c == ' ' { KeyInput::sym(Keysym::space) } else { KeyInput::ch(c) });
    }
    press(k, KeyInput::sym(Keysym::Delete));
    press(k, KeyInput::sym(Keysym::Return));
}

pub(super) fn sent(k: &mut Keys) -> Vec<Command> {
    std::mem::take(&mut k.effects)
        .into_iter()
        .filter_map(|e| match e {
            Effect::Send(c) => Some(c),
            _ => None,
        })
        .collect()
}

pub(super) fn logs(k: &Keys) -> Vec<String> {
    k.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Log(l) => Some(l.clone()),
            _ => None,
        })
        .collect()
}

/// A shell whose first tab exists (what `ready` waits for), with `src` loaded as init.js.
pub(super) fn shell(src: &str) -> Keys {
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
        if (keys !== "bookmarks,buffers,cmd,command,history,map,on") throw new Error(keys);
        const bk = Object.keys(web.buffers).sort().join(",");
        if (bk !== "deleteConfirmation,focusExisting,list") throw new Error(bk);
        web.extra = 1;
        "#,
    );
    // `web` is frozen: the assignment throws in strict mode, after the checks passed.
    assert!(k.message.starts_with("init.js:8: "), "{}", k.message);
    let k = shell(r#"web.on("load", () => {});"#);
    assert!(k.message.contains("web.on: unknown event"), "{}", k.message);
    // C-SCRIPT-URL: importing a built-in adds no global (`URL` is an import only).
    let k = shell(
        r#"import { URL as U } from "web:url";
        import * as api from "web:api";
        if (typeof U !== "function" || typeof URL !== "undefined" || "URL" in globalThis) throw new Error("URL global");"#,
    );
    assert_eq!(k.message, "");
    // C-SCRIPT-MODULES: the file is a module, so `import` parses; with no file behind the text there is
    // nothing to import from, so a static import is the file's error and a dynamic one rejects (promise
    // jobs run under the budget right after the call). Imports that work: c_script_modules_tests.rs.
    let k = shell("import x from \"./x.js\";\n");
    assert!(k.message.starts_with("init.js: import \"./x.js\": "), "{}", k.message);
    let mut k = shell(r#"import("./x.js").then(() => web.cmd("go /loaded"), () => web.cmd("go /rejected"));"#);
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
        root: "/nonexistent/c-script".into(),
    }]);
    assert!(k.scripts.is_none(), "no runtime without a file");
    assert_eq!(k.message, "");
    k.script_ready();
    let env =
        |pairs: &'static [(&'static str, &'static str)]| move |key: &str| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string());
    // C-SCRIPT-MODULES: one file, the profile's own; `default` (no --profile) is a profile like any other.
    let f = init_files(&env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")]), "default");
    assert_eq!(
        f,
        vec![ScriptFile {
            path: "/x/web/profiles/default/init.js".into(),
            label: "init.js".into(),
            root: "/x/web".into(),
        }]
    );
    let f = init_files(&env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")]), "default");
    assert_eq!(f[0].path, std::path::PathBuf::from("/h/.config/web/profiles/default/init.js"));
    let f = init_files(&env(&[("XDG_CONFIG_HOME", "rel"), ("HOME", "/h")]), "default");
    assert_eq!(
        f[0].path,
        std::path::PathBuf::from("/h/.config/web/profiles/default/init.js"),
        "a relative XDG_CONFIG_HOME is ignored"
    );
    assert!(init_files(&env(&[]), "default").is_empty());
}

#[test]
fn c_script_modules_init_files_is_the_profile_alone() {
    let env =
        |pairs: &'static [(&'static str, &'static str)]| move |key: &str| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string());
    let f = init_files(&env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")]), "work");
    assert_eq!(f.len(), 1, "no global init.js before the profile's: {f:?}");
    assert_eq!(f[0].path, std::path::PathBuf::from("/x/web/profiles/work/init.js"));
    assert_eq!(f[0].label, "init.js", "the bar names it init.js, as every profile's");
    assert_eq!(f[0].root, std::path::PathBuf::from("/x/web"));
    let f = init_files(&env(&[("HOME", "/h")]), "p");
    assert_eq!(f[0].path, std::path::PathBuf::from("/h/.config/web/profiles/p/init.js"));
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

// ---- C-SCRIPT-BUFLIST: web.buffers.list() ----

/// A shell with three buffers (3 current, 2 alternate) and `:L` echoing `expr` as JSON.
fn buflist_shell(expr: &str) -> Keys {
    let mut k = shell(&format!(r#"web.command("L", () => web.cmd("echo " + JSON.stringify({expr})));"#));
    k.script_ready();
    k.title = "One".into();
    type_line(&mut k, "new https://two.test/");
    k.buffers.tab_created(2, Some(1));
    type_line(&mut k, "new https://three.test/");
    k.buffers.tab_created(3, Some(2));
    k.buffers.get_mut(2).unwrap().description = "mid".into();
    k.buffers.get_mut(1).unwrap().loading = true;
    k.buffers.get_mut(2).unwrap().crashed = true;
    k
}

#[test]
fn c_script_buflist_fields_and_order() {
    let mut k = buflist_shell("web.buffers.list()");
    assert_eq!(k.buffers.current, 3);
    type_line(&mut k, "L");
    let v: serde_json::Value = serde_json::from_str(&k.message).unwrap_or_else(|e| panic!("{e}: {}", k.message));
    let a = v.as_array().unwrap();
    let nums: Vec<u64> = a.iter().map(|b| b["number"].as_u64().unwrap()).collect();
    assert_eq!(nums, vec![1, 2, 3]);
    assert_eq!(a[0]["url"], "https://a.test/");
    assert_eq!(a[0]["title"], "One", "a buffer left is saved before the snapshot");
    assert_eq!(a[0]["alternate"], false);
    assert_eq!(a[1]["alternate"], true, "2 was current before 3");
    assert_eq!(a[1]["description"], "mid");
    assert_eq!(a[1]["crashed"], true);
    assert_eq!(a[0]["loading"], true);
    assert_eq!(a[2]["current"], true);
    assert_eq!(a[2]["alternate"], false);
    assert_eq!(a[0]["current"], false);
    assert_eq!(a[2]["unread"], 0);
    assert_eq!(a[2]["url"], "https://three.test/");
    // The same order and numbers as `:ls`.
    let picker: Vec<u32> = crate::buffers::rows(&k.buffers)
        .iter()
        .map(|r| match r.action {
            crate::picker::PickAction::Buffer(n) => n,
            _ => 0,
        })
        .collect();
    assert_eq!(picker, vec![1, 2, 3]);
}

#[test]
fn c_script_buflist_empty_before_ready_and_current_url_is_live() {
    let mut k = Keys::new(None, String::new(), None);
    k.load_script_source(
        "init.js",
        r#"web.on("ready", () => web.cmd("echo r" + web.buffers.list().length));
           web.command("U", () => web.cmd("echo " + web.buffers.list()[0].url));
           if (web.buffers.list().length !== 0) throw new Error("not empty before ready");"#,
        BUDGET,
    );
    assert_eq!(k.message, "");
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = "https://a.test/".into();
    k.script_ready();
    assert_eq!(k.message, "r1", "ready handlers see the buffers");
    k.url = "https://a.test/moved".into();
    type_line(&mut k, "U");
    assert_eq!(k.message, "https://a.test/moved", "the current buffer's url is read at call time");
}

#[test]
fn c_script_buflist_cmd_result_seen_in_later_call_only() {
    let mut k = shell(
        r#"web.command("N", () => { web.cmd("new https://n.test/"); web.cmd("echo in" + web.buffers.list().length); });
           web.command("C", () => web.cmd("echo later" + web.buffers.list().length));"#,
    );
    k.script_ready();
    // The lines run after the call returned: it saw one buffer; the list is read when called.
    type_line(&mut k, "N");
    assert_eq!(k.message, "in1");
    assert_eq!(k.buffers.list.len(), 2);
    type_line(&mut k, "C");
    assert_eq!(k.message, "later2");
}

#[test]
fn c_script_buflist_objects_frozen_and_fresh() {
    let mut k = shell(
        r#"web.command("F", () => {
            const a = web.buffers.list(), b = web.buffers.list();
            let thrown = false;
            try { a[0].url = "x"; } catch (e) { thrown = true; }
            web.cmd("echo " + [thrown, Object.isFrozen(a[0]), a !== b, a[0] !== b[0], a[0].url === b[0].url].join());
        });"#,
    );
    k.script_ready();
    type_line(&mut k, "F");
    // Module code is strict: assigning to a frozen object throws.
    assert_eq!(k.message, "true,true,true,true,true");
}

#[test]
fn c_script_buflist_find_and_switch_example() {
    let mut k = shell(
        r#"web.map("n", "gs", () => {
            const b = web.buffers.list().find(b => b.url.startsWith("https://app.slack.com/"));
            web.cmd(b ? "b " + b.number : "new https://app.slack.com");
        });"#,
    );
    k.script_ready();
    type_line(&mut k, "new https://app.slack.com/client");
    k.buffers.tab_created(2, Some(1));
    type_line(&mut k, "new https://other.test/");
    k.buffers.tab_created(3, Some(2));
    assert_eq!(k.buffers.current, 3);
    press(&mut k, KeyInput::ch('g'));
    press(&mut k, KeyInput::ch('s'));
    assert_eq!(k.buffers.current, 2, "switched to the slack buffer");
    assert_eq!(k.buffers.list.len(), 3);
}

// ---- C-CMDRESULT: web.cmd returns a promise settled with { ok, message } ----

/// A shell with the extra commands `defs` (JS), `:Run` as an async function with `body`, and `:Show`
/// echoing the array `out` as JSON.
fn result_shell(defs: &str, body: &str) -> Keys {
    let mut k = shell(&format!(
        r#"globalThis.out = [];
           {defs}
           web.command("Run", async () => {{ {body} }});
           web.command("Show", () => web.cmd("echo " + JSON.stringify(out)));"#
    ));
    k.script_ready();
    k
}

fn shown(k: &mut Keys) -> serde_json::Value {
    type_line(k, "Show");
    serde_json::from_str(&k.message).unwrap_or_else(|e| panic!("{e}: {}", k.message))
}

#[test]
fn c_cmdresult_ok_and_message_for_good_and_failing_lines() {
    let mut k = result_shell(
        "",
        r#"out.push(await web.cmd("go /x"));
           out.push(await web.cmd(":echo hello  there"));
           out.push(await web.cmd("nope"));
           out.push(await web.cmd("b nomatch"));
           out.push(await web.cmd("b 9"));
           out.push(await web.cmd("go"));
           out.push(await web.cmd("b 1"));"#,
    );
    k.effects.clear();
    type_line(&mut k, "Run");
    let v = shown(&mut k);
    assert_eq!(v[0], serde_json::json!({"ok": true, "message": ""}));
    assert_eq!(v[1], serde_json::json!({"ok": true, "message": "hello  there"}));
    assert_eq!(v[2], serde_json::json!({"ok": false, "message": "web: unknown command: :nope"}));
    assert_eq!(v[3], serde_json::json!({"ok": false, "message": "web: no buffer matches nomatch"}));
    assert_eq!(v[4], serde_json::json!({"ok": false, "message": "web: no buffer 9"}));
    assert_eq!(v[5], serde_json::json!({"ok": false, "message": "web: usage: :go <url>"}));
    assert_eq!(v[6]["ok"], true, "{v}");
}

#[test]
fn c_cmdresult_user_command_that_throws_is_not_ok() {
    let mut k = result_shell(
        r#"web.command("Boom", () => { throw new Error("bad"); });
           web.command("Fine", () => {});
           web.command("Inner", () => web.cmd("nope"));"#,
        r#"out.push(await web.cmd("Boom"));
           out.push(await web.cmd("Fine"));
           out.push(await web.cmd("Inner"));"#,
    );
    type_line(&mut k, "Run");
    let v = shown(&mut k);
    assert_eq!(v[0]["ok"], false);
    let m = v[0]["message"].as_str().unwrap();
    assert!(m.starts_with("init.js:") && m.contains("bad"), "{v}");
    assert_eq!(v[1], serde_json::json!({"ok": true, "message": ""}));
    // A line the command itself ran failed; the command did not.
    assert_eq!(v[2]["ok"], true, "{v}");
}

#[test]
fn c_cmdresult_awaited_lines_run_in_order_and_the_slack_example_falls_back() {
    let mut k = shell(
        r#"web.command("Run", async () => {
             web.cmd("go /first");
             await web.cmd("go /second");
             web.cmd("go /fourth");
             const r = await web.cmd("b app.slack.com");
             if (!r.ok) web.cmd("go /fallback");
           });"#,
    );
    k.script_ready();
    k.effects.clear();
    type_line(&mut k, "Run");
    let urls: Vec<String> = sent(&mut k)
        .into_iter()
        .map(|c| match c {
            Command::Open { url } => url,
            o => panic!("{o:?}"),
        })
        .collect();
    assert_eq!(
        urls,
        [
            "https://a.test/first",
            "https://a.test/second",
            "https://a.test/fourth",
            "https://a.test/fallback"
        ]
    );
}

#[test]
fn c_cmdresult_ignoring_the_promise_is_silent_and_failures_never_reject() {
    let mut k = shell(r#"web.command("Run", () => { web.cmd("nope"); web.cmd("b nomatch"); web.cmd("go /ok"); });"#);
    k.script_ready();
    k.effects.clear();
    type_line(&mut k, "Run");
    assert_eq!(
        k.message, "web: no buffer matches nomatch",
        "the last error message, no rejection report"
    );
    assert!(!logs(&k).iter().any(|l| l.contains("unhandled")), "{:?}", logs(&k));
    assert_eq!(sent(&mut k).len(), 1);
    // With a handler the failure is a value, not an exception.
    let mut k = shell(r#"web.command("Run", () => web.cmd("nope").then(r => web.cmd("echo ok=" + r.ok)));"#);
    k.script_ready();
    type_line(&mut k, "Run");
    assert_eq!(k.message, "ok=false");
}

#[test]
fn c_cmdresult_the_bar_keeps_its_message_when_the_line_gave_none() {
    let mut k = shell(r#"web.command("Run", () => web.cmd("go /x"));"#);
    k.script_ready();
    k.message = "earlier".into();
    k.run_command_line("Run");
    assert_eq!(k.message, "earlier");
}

#[test]
fn c_cmdresult_error_in_the_continuation_is_reported_like_any_call() {
    let mut k = shell(r#"web.command("Run", async () => { await web.cmd("go /x"); throw new Error("after"); });"#);
    k.script_ready();
    type_line(&mut k, "Run");
    assert!(k.message.contains("unhandled rejection: after"), "{}", k.message);
}

#[test]
fn c_cmdresult_top_level_await_resumes_at_ready() {
    let mut k = Keys::new(None, String::new(), None);
    k.load_script_source(
        "init.js",
        r#"const r = await web.cmd("nope"); web.cmd("echo top " + r.ok);"#,
        BUDGET,
    );
    assert_eq!(k.message, "");
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.script_ready();
    assert_eq!(k.message, "top false");
}

#[test]
fn c_cmdresult_a_chain_that_never_ends_is_stopped() {
    let mut k = shell(r#"web.command("Run", async () => { while (true) await web.cmd("go /x"); });"#);
    k.script_ready();
    k.effects.clear();
    let start = Instant::now();
    type_line(&mut k, "Run");
    assert!(start.elapsed() < Duration::from_secs(20));
    assert!(k.message.contains("lines in one chain"), "{}", k.message);
    // Works again afterwards.
    type_line(&mut k, "echo fine");
    assert_eq!(k.message, "fine");
}
