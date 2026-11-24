//! C-SCRIPT-ERRBAR, C-SOURCE, C-ECHO (SCRIPTING.md ":source", ":echo", "Errors"; KEYS.md T148-T157).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::buffers::TabCreated;
use crate::config::SCRIPT_BUDGET_MS;
use crate::keys::*;
use crate::protocol::Command;
use crate::script::{init_files, ScriptFile};

use super::c_script_tests::{logs, press, shell, type_line};

const BUDGET: Duration = Duration::from_millis(SCRIPT_BUDGET_MS);

/// A config directory of its own (`XDG_CONFIG_HOME` for `init_files`), removed by name at the end.
struct Config {
    dir: PathBuf,
}

impl Config {
    fn new(name: &str) -> Config {
        let dir = std::env::temp_dir().join(format!("web-c-source-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web/profiles/p")).unwrap();
        Config { dir }
    }

    fn global(&self, src: &str) {
        std::fs::write(self.dir.join("web/init.js"), src).unwrap();
    }

    fn profile(&self, src: &str) {
        std::fs::write(self.dir.join("web/profiles/p/init.js"), src).unwrap();
    }

    fn files(&self, profile: Option<&str>) -> Vec<ScriptFile> {
        let home = self.dir.display().to_string();
        init_files(&move |k| (k == "XDG_CONFIG_HOME").then(|| home.clone()), profile)
    }

    /// A shell that loaded this config as at start, its first tab existing.
    fn shell(&self, profile: Option<&str>) -> Keys {
        let mut k = Keys::new(None, String::new(), None);
        k.load_scripts_with(&self.files(profile), BUDGET);
        assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
        k.url = "https://a.test/".into();
        k
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
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

fn new_tab(url: &str, req: u64) -> Command {
    Command::NewTab {
        req,
        url: Some(url.into()),
    }
}

// ---------- C-SCRIPT-ERRBAR ----------

#[test]
fn errbar_a_script_error_survives_page_loads_until_the_next_key() {
    let mut k = shell("\nthrow new Error(\"startup boom\");\n");
    assert_eq!(k.message, "init.js:2: startup boom");
    // The first successful load (state.rs calls `page_loaded`) leaves it, and so do more.
    k.page_loaded();
    k.page_loaded();
    assert_eq!(k.message, "init.js:2: startup boom");
    // The user's next key ends the protection; the next load clears it as any message.
    press(&mut k, KeyInput::ch('j'));
    assert_eq!(k.message, "init.js:2: startup boom", "the key itself does not clear it");
    k.page_loaded();
    assert_eq!(k.message, "");
}

#[test]
fn errbar_other_messages_are_still_cleared_by_a_load() {
    let mut k = shell("");
    type_line(&mut k, "nosuch");
    assert!(k.message.starts_with("web: unknown command"), "{:?}", k.message);
    k.page_loaded();
    assert_eq!(k.message, "");
}

#[test]
fn errbar_a_later_message_replaces_the_protected_one() {
    let mut k = shell("throw new Error(\"x\");");
    assert_eq!(k.message, "init.js:1: x");
    // A message that is not the script's (set with no key between: a host event) is not protected.
    k.message = "web: host: something".into();
    k.page_loaded();
    assert_eq!(k.message, "");
}

#[test]
fn errbar_errors_of_commands_mappings_and_ready_are_protected_too() {
    let mut k = shell(
        r#"
        web.command("Boom", () => { throw new Error("in command"); });
        web.on("ready", () => { throw new Error("in ready"); });
        "#,
    );
    k.script_ready();
    assert_eq!(k.message, "init.js:3: in ready");
    k.page_loaded();
    assert_eq!(k.message, "init.js:3: in ready");
    type_line(&mut k, "Boom");
    assert_eq!(k.message, "init.js:2: in command");
    k.page_loaded();
    assert_eq!(k.message, "init.js:2: in command", "the Enter was before the error");
}

// ---------- C-ECHO ----------

#[test]
fn echo_shows_the_rest_of_the_line_verbatim_and_logs_it() {
    let mut k = shell("");
    k.effects.clear();
    type_line(&mut k, "echo hello   big  world ");
    assert_eq!(k.message, "hello   big  world ");
    assert!(logs(&k).contains(&"echo: hello   big  world ".to_string()), "{:?}", logs(&k));
    type_line(&mut k, "echo");
    assert_eq!(k.message, "", "an empty echo clears the bar");
    // Everything after the name, quotes and `:` included.
    k.run_command_line("echo   :x \"y\" 'z'");
    assert_eq!(k.message, ":x \"y\" 'z'");
}

#[test]
fn echo_from_a_script_at_start_survives_the_page_load() {
    let mut k = shell(r#"web.on("ready", () => web.cmd("echo from init " + (1 + 2)));"#);
    k.script_ready();
    assert_eq!(k.message, "from init 3");
    k.page_loaded();
    assert_eq!(k.message, "from init 3");
    press(&mut k, KeyInput::ch('j'));
    k.page_loaded();
    assert_eq!(k.message, "");
}

#[test]
fn echo_is_a_command_the_completion_knows() {
    assert!(COMMAND_NAMES.contains(&"echo"));
    assert!(COMMAND_NAMES.contains(&"source"));
    let mut sorted = COMMAND_NAMES.to_vec();
    sorted.sort_unstable();
    assert_eq!(sorted, COMMAND_NAMES, "COMMAND_NAMES stays sorted");
}

#[test]
fn echo_there_is_no_console_object() {
    let k = shell("console.log(1);");
    assert!(k.message.contains("console"), "{:?}", k.message);
}

// ---------- C-SOURCE ----------

#[test]
fn source_drops_old_commands_and_runs_the_changed_file() {
    let c = Config::new("cmds");
    c.global(r#"web.command("Old", () => web.cmd("echo old")); web.command("Keep", () => web.cmd("echo keep1"));"#);
    let mut k = c.shell(None);
    type_line(&mut k, "Old");
    assert_eq!(k.message, "old");
    c.global(r#"web.command("New", () => web.cmd("echo new")); web.command("Keep", () => web.cmd("echo keep2"));"#);
    type_line(&mut k, "source");
    assert_eq!(k.message, "sourced 1 file(s)");
    assert!(logs(&k).iter().any(|l| l.starts_with("script: sourced 1 file(s)")));
    type_line(&mut k, "New");
    assert_eq!(k.message, "new");
    type_line(&mut k, "Keep");
    assert_eq!(k.message, "keep2", "a command defined again is the new function");
    type_line(&mut k, "Old");
    assert!(k.message.starts_with("web: unknown command: :Old"), "{:?}", k.message);
    assert_eq!(k.user_command_names(), vec!["Keep".to_string(), "New".to_string()]);
}

#[test]
fn source_replaces_mappings_and_forgets_removed_ones() {
    let c = Config::new("maps");
    c.global(r#"web.map("n", "gx", ":echo one<CR>"); web.map("n", "gy", ":echo gy<CR>");"#);
    let mut k = c.shell(None);
    let t = Instant::now();
    let tap = |k: &mut Keys, s: &str| {
        for ch in s.chars() {
            let key = KeyInput::ch(ch);
            k.key(key.sym.raw(), true, &key, t);
            k.key(key.sym.raw(), false, &key, t);
        }
    };
    tap(&mut k, "gx");
    assert_eq!(k.message, "one");
    c.global(r#"web.map("n", "gx", ":echo two<CR>");"#);
    type_line(&mut k, "source");
    k.message.clear();
    tap(&mut k, "gx");
    assert_eq!(k.message, "two", "the mapping is the new one");
    k.message.clear();
    tap(&mut k, "gy");
    assert_eq!(k.message, "", "a mapping the file no longer has is gone");
    // No mapping left at all: the keys go straight to the bindings again.
    c.global("");
    type_line(&mut k, "source");
    assert!(!k.scripts.as_ref().is_some_and(|s| s.has_maps()));
}

#[test]
fn source_does_not_fire_ready_again() {
    let c = Config::new("ready");
    c.global(r#"web.on("ready", () => web.cmd("new http://127.0.0.1:1/start"));"#);
    let mut k = c.shell(None);
    k.script_ready();
    assert_eq!(sent(&mut k), vec![new_tab("http://127.0.0.1:1/start", 1)]);
    assert_eq!(k.buffers.list.len(), 2);
    type_line(&mut k, "source");
    assert_eq!(k.message, "sourced 1 file(s)");
    k.script_ready();
    assert!(sent(&mut k).is_empty(), "the handler is registered again but not fired");
    assert_eq!(k.buffers.list.len(), 2, "no duplicate buffer");
    // A later tab (the compositor calls script_ready for each) does not fire it either.
    k.script_ready();
    assert!(sent(&mut k).is_empty());
}

#[test]
fn source_before_ready_leaves_ready_for_its_time() {
    let c = Config::new("early");
    c.global(r#"web.on("ready", () => web.cmd("new http://127.0.0.1:1/a"));"#);
    let mut k = c.shell(None);
    c.global(r#"web.on("ready", () => web.cmd("new http://127.0.0.1:1/b"));"#);
    type_line(&mut k, "source");
    k.script_ready();
    assert_eq!(sent(&mut k), vec![new_tab("http://127.0.0.1:1/b", 1)]);
}

#[test]
fn source_error_is_reported_as_at_start_and_keeps_what_ran_before_it() {
    let c = Config::new("err");
    c.global(r#"web.command("A", () => web.cmd("echo a"));"#);
    let mut k = c.shell(None);
    c.global("web.command(\"B\", () => web.cmd(\"echo b\"));\nthrow new Error(\"second boom\");\nweb.command(\"C\", () => {});");
    type_line(&mut k, "source");
    assert_eq!(k.message, "init.js:2: second boom");
    assert!(logs(&k).iter().any(|l| l.starts_with("script error: init.js:2: second boom")));
    assert_eq!(
        k.user_command_names(),
        vec!["B".to_string()],
        "A is gone, B ran before the error, C never ran"
    );
    // The error is protected like one at start.
    k.page_loaded();
    assert_eq!(k.message, "init.js:2: second boom");
    // A syntax error: the old setup is gone (clean reload), the bar says where.
    c.global("web.command(\"D\", ()");
    type_line(&mut k, "source");
    assert!(k.message.starts_with("init.js:"), "{:?}", k.message);
    assert!(k.user_command_names().is_empty());
}

#[test]
fn source_reruns_global_then_profile_with_the_same_profile() {
    let c = Config::new("profile");
    c.global(r#"web.command("G", () => web.cmd("echo g1"));"#);
    c.profile(r#"web.command("P", () => web.cmd("echo " + typeof G + "1"));"#);
    let mut k = c.shell(Some("p"));
    c.global(r#"web.command("G", () => web.cmd("echo g2"));"#);
    c.profile(r#"web.command("P", () => web.cmd("echo p2"));"#);
    type_line(&mut k, "source");
    assert_eq!(k.message, "sourced 2 file(s)");
    type_line(&mut k, "G");
    assert_eq!(k.message, "g2");
    type_line(&mut k, "P");
    assert_eq!(k.message, "p2");
    // A file that disappeared is not an error; the count says how many ran.
    std::fs::remove_file(c.dir.join("web/profiles/p/init.js")).unwrap();
    type_line(&mut k, "source");
    assert_eq!(k.message, "sourced 1 file(s)");
    type_line(&mut k, "P");
    assert!(k.message.starts_with("web: unknown command: :P"), "{:?}", k.message);
}

#[test]
fn source_with_no_file_left_gives_a_clean_runtime() {
    let c = Config::new("none");
    c.global(r#"web.command("A", () => {});"#);
    let mut k = c.shell(None);
    std::fs::remove_file(c.dir.join("web/init.js")).unwrap();
    type_line(&mut k, "source");
    assert_eq!(k.message, "sourced 0 file(s)");
    assert!(k.scripts.is_none());
    assert!(k.user_command_names().is_empty());
    // And a file that appears later loads, without a `ready`.
    c.global(r#"web.on("ready", () => web.cmd("new http://127.0.0.1:1/x")); web.command("N", () => web.cmd("echo n"));"#);
    type_line(&mut k, "source");
    assert_eq!(k.message, "sourced 1 file(s)");
    k.script_ready();
    assert!(sent(&mut k).is_empty());
    type_line(&mut k, "N");
    assert_eq!(k.message, "n");
}

#[test]
fn source_with_no_scripts_at_all_works() {
    let mut k = Keys::new(None, String::new(), None);
    type_line(&mut k, "source");
    assert_eq!(k.message, "sourced 0 file(s)");
}

#[test]
fn source_from_web_cmd_is_an_error_and_changes_nothing() {
    let c = Config::new("cmd");
    c.global(
        r#"
        web.command("A", () => web.cmd("echo a"));
        web.command("Reload", () => web.cmd("source"));
        web.map("n", "gr", () => web.cmd(":source"));
        web.on("ready", () => web.cmd("source"));
        "#,
    );
    let mut k = c.shell(None);
    type_line(&mut k, "Reload");
    assert!(
        k.message.contains(":source") && k.message.contains("cannot reload"),
        "{:?}",
        k.message
    );
    assert!(logs(&k).iter().any(|l| l.starts_with("script error: web.cmd: :source: refused")));
    assert!(k.user_command_names().contains(&"A".to_string()), "nothing was reloaded");
    // A map function too.
    k.message.clear();
    let t = Instant::now();
    for ch in "gr".chars() {
        let key = KeyInput::ch(ch);
        k.key(key.sym.raw(), true, &key, t);
        k.key(key.sym.raw(), false, &key, t);
    }
    assert!(k.message.contains("cannot reload"), "{:?}", k.message);
    // And `ready`.
    k.message.clear();
    k.script_ready();
    assert!(k.message.contains("cannot reload"), "{:?}", k.message);
    // The refusal does not leave the guard on: the `:` line still reloads.
    type_line(&mut k, "source");
    assert_eq!(k.message, "sourced 1 file(s)");
}

#[test]
fn source_arguments_are_an_error_for_now() {
    let c = Config::new("args");
    c.global(r#"web.command("A", () => {});"#);
    let mut k = c.shell(None);
    type_line(&mut k, "source other.js");
    assert_eq!(k.message, "web: usage: :source (no arguments yet)");
    assert_eq!(k.user_command_names(), vec!["A".to_string()], "nothing was reloaded");
}

#[test]
fn source_runs_from_a_user_mapping_key_outside_a_js_call() {
    let c = Config::new("mapkey");
    c.global(r#"web.map("n", "ss", ":source<CR>"); web.command("V", () => web.cmd("echo v1"));"#);
    let mut k = c.shell(None);
    c.global(r#"web.map("n", "ss", ":source<CR>"); web.command("V", () => web.cmd("echo v2"));"#);
    let t = Instant::now();
    for ch in "ss".chars() {
        let key = KeyInput::ch(ch);
        k.key(key.sym.raw(), true, &key, t);
        k.key(key.sym.raw(), false, &key, t);
    }
    assert_eq!(k.message, "sourced 1 file(s)");
    assert_eq!(k.map_wait(t), None, "no mapping timer is left running");
    type_line(&mut k, "V");
    assert_eq!(k.message, "v2");
    // The mapping is in the new runtime too: it works again.
    k.message.clear();
    for ch in "ss".chars() {
        let key = KeyInput::ch(ch);
        k.key(key.sym.raw(), true, &key, t);
        k.key(key.sym.raw(), false, &key, t);
    }
    assert_eq!(k.message, "sourced 1 file(s)");
}

#[test]
fn source_matches_a_half_typed_mapping_again_against_the_new_mappings() {
    let c = Config::new("pending");
    c.global(r#"web.map("n", "xa", ":echo old<CR>");"#);
    let mut k = c.shell(None);
    let t = Instant::now();
    let tap = |k: &mut Keys, ch: char, t: Instant| {
        let key = KeyInput::ch(ch);
        k.key(key.sym.raw(), true, &key, t);
        k.key(key.sym.raw(), false, &key, t);
    };
    tap(&mut k, 'x', t);
    assert!(k.map_wait(t).is_some(), "x waits for xa");
    c.global(r#"web.map("n", "xb", ":echo new<CR>");"#);
    k.source();
    assert!(k.map_wait(t).is_some(), "the key is still waiting, for the new mappings now");
    k.message.clear();
    tap(&mut k, 'a', t);
    assert_eq!(k.message, "", "xa is gone: nothing fires");
    assert_eq!(k.map_wait(t), None);
    tap(&mut k, 'x', t);
    tap(&mut k, 'b', t);
    assert_eq!(k.message, "new");
}

#[test]
fn source_runs_top_level_web_cmd_lines_at_once() {
    let c = Config::new("top");
    c.global(r#"web.cmd("echo top level");"#);
    let mut k = c.shell(None);
    assert_eq!(k.message, "", "at start they wait for ready");
    k.script_ready();
    assert_eq!(k.message, "top level");
    k.message.clear();
    type_line(&mut k, "source");
    assert_eq!(k.message, "top level", "the host is connected: they run now");
}

#[test]
fn source_resets_the_runtime_state_the_old_chain_left() {
    // A user command chain that hit the depth limit leaves `overflow` set only while it unwinds;
    // the new runtime starts with none, and a runaway file at :source ends under the budget.
    let c = Config::new("budget");
    c.global(r#"web.command("A", () => {});"#);
    let mut k = c.shell(None);
    c.global("while (true) {}");
    let t0 = Instant::now();
    type_line(&mut k, "source");
    assert!(t0.elapsed() < Duration::from_secs(5));
    assert!(k.message.contains("interrupted"), "{:?}", k.message);
    c.global(r#"web.command("A", () => web.cmd("echo again"));"#);
    type_line(&mut k, "source");
    type_line(&mut k, "A");
    assert_eq!(k.message, "again");
}
