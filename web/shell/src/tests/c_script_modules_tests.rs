//! C-SCRIPT-MODULES (SCRIPTING.md "Files and profiles", "Modules"; DECISIONS.md 2026-11-25): one init.js
//! per profile, evaluated as an ES module; imports confined to `$XDG_CONFIG_HOME/web/`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::buffers::TabCreated;
use crate::config::{SCRIPT_BUDGET_MS, SCRIPT_MODULE_MAX};
use crate::keys::*;
use crate::script::init_files;

use super::c_script_tests::{logs, type_line};

const BUDGET: Duration = Duration::from_millis(SCRIPT_BUDGET_MS);

/// A config directory of its own (`XDG_CONFIG_HOME`), removed by name at the end. Paths are the
/// real ones (symlinks resolved), as error messages name modules by their real path.
struct Config {
    dir: PathBuf,
}

impl Config {
    fn new(name: &str) -> Config {
        let dir = std::env::temp_dir().join(format!("web-c-modules-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        Config {
            dir: dir.canonicalize().unwrap(),
        }
    }

    /// `$XDG_CONFIG_HOME/web/<rel>`.
    fn web(&self, rel: &str) -> PathBuf {
        self.dir.join("web").join(rel)
    }

    /// Writes `web/<rel>` (making its folders).
    fn write(&self, rel: &str, src: &str) {
        let p = self.web(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }

    /// A shell started with `--profile profile`, its first tab existing.
    fn shell(&self, profile: &str) -> Keys {
        let home = self.dir.display().to_string();
        let files = init_files(&move |k| (k == "XDG_CONFIG_HOME").then(|| home.clone()), profile);
        let mut k = Keys::new(None, String::new(), None);
        k.load_scripts_with(&files, BUDGET);
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

/// Runs `:name` and gives the bar's message.
fn run(k: &mut Keys, name: &str) -> String {
    type_line(k, name);
    k.message.clone()
}

fn script_errors(k: &Keys) -> Vec<String> {
    logs(k).into_iter().filter(|l| l.starts_with("script error: ")).collect()
}

#[test]
fn modules_static_and_dynamic_import_relative_to_the_init_file() {
    let c = Config::new("static");
    c.write("profiles/p/util.js", "export const word = \"static\";\n");
    c.write("profiles/p/later.js", "export default function () { return \"dynamic\"; }\n");
    c.write(
        "profiles/p/init.js",
        r#"import { word } from "./util.js";
web.command("S", () => web.cmd("echo " + word));
web.command("D", async () => {
  const m = await import("./later.js");
  web.cmd("echo " + m.default());
});
"#,
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "", "{:?}", logs(&k));
    assert_eq!(run(&mut k, "S"), "static");
    assert_eq!(run(&mut k, "D"), "dynamic");
}

#[test]
fn modules_parent_specifier_from_a_nested_module() {
    let c = Config::new("nested");
    c.write(
        "profiles/p/sub/deeper/a.js",
        "import { b } from \"../b.js\";\nexport const a = \"a+\" + b;\n",
    );
    c.write("profiles/p/sub/b.js", "export const b = \"b\";\n");
    c.write(
        "profiles/p/init.js",
        "import { a } from \"./sub/deeper/a.js\";\nweb.command(\"A\", () => web.cmd(\"echo \" + a));\n",
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "");
    assert_eq!(run(&mut k, "A"), "a+b");
}

#[test]
fn modules_two_profiles_share_lib_and_the_default_profile_is_not_inherited() {
    let c = Config::new("shared");
    // A lib module imports its neighbour with `./` (a bare name would be the importing profile's).
    c.write(
        "lib/common.js",
        "import { tag } from \"./tag.js\";\nexport function hello(who) { return tag + who; }\n",
    );
    c.write("lib/tag.js", "export const tag = \"hi \";\n");
    c.write(
        "profiles/default/init.js",
        "import { hello } from \"../../lib/common.js\";\nweb.command(\"Hello\", () => web.cmd(\"echo \" + hello(\"default\")));\nweb.command(\"OnlyDefault\", () => {});\n",
    );
    let abs = c.web("lib/common.js");
    c.write(
        "profiles/work/init.js",
        &format!(
            "import {{ hello }} from {:?};\nweb.command(\"Hello\", () => web.cmd(\"echo \" + hello(\"work\")));\n",
            abs.display().to_string()
        ),
    );
    let mut d = c.shell("default");
    assert_eq!(d.message, "");
    assert_eq!(run(&mut d, "Hello"), "hi default");
    let mut w = c.shell("work");
    assert_eq!(w.message, "");
    assert_eq!(run(&mut w, "Hello"), "hi work");
    assert_eq!(
        w.user_command_names(),
        vec!["Hello".to_string()],
        "nothing of the default profile's"
    );
}

#[test]
fn modules_a_module_imported_twice_runs_once() {
    let c = Config::new("once");
    c.write(
        "profiles/p/count.js",
        "globalThis.runs = (globalThis.runs ?? 0) + 1;\nexport const n = 1;\n",
    );
    c.write("profiles/p/a.js", "import \"./count.js\";\n");
    c.write("profiles/p/b.js", "import \"../p/count.js\";\n");
    let abs = c.web("profiles/p/count.js");
    c.write(
        "profiles/p/init.js",
        &format!(
            "import \"./a.js\";\nimport \"./b.js\";\nimport \"count.js\";\nimport {:?};\nweb.command(\"Runs\", async () => {{ await import(\"./count.js\"); web.cmd(\"echo \" + globalThis.runs); }});\n",
            abs.display().to_string()
        ),
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "");
    assert_eq!(run(&mut k, "Runs"), "1");
}

#[test]
fn modules_bare_specifier_is_the_profile_folder() {
    let c = Config::new("bare");
    c.write("profiles/p/helpers.js", "export const h = \"from p\";\n");
    // In a lib module, a bare name is the importing profile's folder too.
    c.write(
        "lib/uses_bare.js",
        "import { h } from \"helpers.js\";\nexport const via = \"lib \" + h;\n",
    );
    c.write(
        "profiles/p/init.js",
        "import { h } from \"helpers.js\";\nimport { via } from \"../../lib/uses_bare.js\";\nweb.command(\"H\", () => web.cmd(\"echo \" + h + \", \" + via));\n",
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "");
    assert_eq!(run(&mut k, "H"), "from p, lib from p");
    // A bare name that is not in the profile's folder: an error naming where it looked.
    c.write("profiles/q/init.js", "import \"nothere.js\";\nweb.command(\"Q\", () => {});\n");
    let k = c.shell("q");
    let want = format!(
        "init.js: import \"nothere.js\": no such file: {}",
        c.web("profiles/q/nothere.js").display()
    );
    assert_eq!(k.message, want);
    assert!(k.user_command_names().is_empty(), "a failed import runs nothing of the file");
}

#[test]
fn modules_missing_module_names_its_path() {
    let c = Config::new("missing");
    c.write("profiles/p/a.js", "import \"./gone.js\";\n");
    c.write("profiles/p/init.js", "import \"./a.js\";\n");
    let k = c.shell("p");
    let want = format!(
        "init.js: import \"./gone.js\": no such file: {}",
        c.web("profiles/p/gone.js").display()
    );
    assert_eq!(k.message, want);
    assert_eq!(script_errors(&k).len(), 1, "{:?}", logs(&k));
}

#[test]
fn modules_a_syntax_error_in_an_imported_module_is_the_init_files_error() {
    let c = Config::new("syntax");
    c.write("profiles/p/bad.js", "export const ok = 1;\nlet x = ;\n");
    c.write("profiles/p/init.js", "import \"./bad.js\";\n");
    let k = c.shell("p");
    let want = format!("init.js: {}:2: ", c.web("profiles/p/bad.js").display());
    assert!(k.message.starts_with(&want), "{:?} vs {want:?}", k.message);
}

#[test]
fn modules_a_throw_inside_an_imported_module_names_its_path_and_line() {
    let c = Config::new("throw");
    c.write(
        "profiles/p/lib.js",
        "export function boom() {\n  throw new Error(\"in lib\");\n}\nexport const top = 1;\n",
    );
    c.write(
        "profiles/p/init.js",
        "import { boom } from \"./lib.js\";\nweb.command(\"Boom\", () => boom());\n",
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "");
    assert_eq!(run(&mut k, "Boom"), format!("{}:2: in lib", c.web("profiles/p/lib.js").display()));
    // A throw at a module's top level, at start.
    c.write("profiles/r/top.js", "\n\nthrow new Error(\"top of module\");\n");
    c.write("profiles/r/init.js", "web.command(\"Before\", () => {});\nimport \"./top.js\";\n");
    let k = c.shell("r");
    assert_eq!(k.message, format!("{}:3: top of module", c.web("profiles/r/top.js").display()));
}

#[test]
fn modules_a_top_level_throw_is_reported_once() {
    let c = Config::new("once-err");
    c.write(
        "profiles/p/init.js",
        "web.command(\"Kept\", () => {});\n\nthrow new Error(\"top\");\n",
    );
    let k = c.shell("p");
    assert_eq!(k.message, "init.js:3: top");
    let errors = script_errors(&k);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(!errors[0].contains("unhandled rejection"), "{errors:?}");
    assert_eq!(k.user_command_names(), vec!["Kept".to_string()], "what ran before the error stays");
}

#[test]
fn modules_top_level_await_of_a_resolved_promise_finishes_at_start() {
    let c = Config::new("tla");
    c.write("profiles/p/v.js", "export const v = await Promise.resolve(\"awaited\");\n");
    c.write(
        "profiles/p/init.js",
        "import { v } from \"./v.js\";\nconst w = await Promise.resolve(v + \"!\");\nweb.command(\"W\", () => web.cmd(\"echo \" + w));\n",
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "");
    assert_eq!(run(&mut k, "W"), "awaited!");
}

#[test]
fn modules_top_level_await_on_ready_continues_in_the_ready_call() {
    let c = Config::new("tla-ready");
    c.write(
        "profiles/p/init.js",
        "web.command(\"Early\", () => {});\nawait new Promise(r => web.on(\"ready\", r));\nweb.command(\"Late\", () => web.cmd(\"echo late\"));\n\nthrow new Error(\"after ready\");\n",
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "", "a pending top-level await is not an error");
    assert_eq!(k.user_command_names(), vec!["Early".to_string()]);
    k.effects.clear();
    k.script_ready();
    assert_eq!(k.user_command_names(), vec!["Early".to_string(), "Late".to_string()]);
    assert_eq!(k.message, "init.js:5: after ready", "its error, once, as the file's");
    let errors = script_errors(&k);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(!errors[0].contains("unhandled rejection"), "{errors:?}");
    assert_eq!(run(&mut k, "Late"), "late");
}

#[test]
fn modules_source_reads_an_edited_module_again() {
    let c = Config::new("source");
    c.write("lib/m.js", "export const v = \"one\";\n");
    c.write(
        "profiles/p/init.js",
        "import { v } from \"../../lib/m.js\";\nweb.command(\"V\", () => web.cmd(\"echo \" + v));\n",
    );
    let mut k = c.shell("p");
    assert_eq!(run(&mut k, "V"), "one");
    c.write("lib/m.js", "export const v = \"two\";\n");
    assert_eq!(run(&mut k, "source"), "sourced 1 file(s)");
    assert_eq!(run(&mut k, "V"), "two");
}

#[test]
fn modules_budget_overrun_inside_an_imported_module() {
    let c = Config::new("budget");
    c.write("profiles/p/spin.js", "export const x = 1;\nwhile (true) {}\n");
    c.write("profiles/p/init.js", "web.command(\"Before\", () => {});\nimport \"./spin.js\";\n");
    let start = Instant::now();
    let k = c.shell("p");
    assert!(start.elapsed() < BUDGET * 20, "{:?}", start.elapsed());
    let spin = c.web("profiles/p/spin.js").display().to_string();
    assert!(k.message.starts_with(&spin), "{}", k.message);
    assert!(k.message.contains("interrupted"), "{}", k.message);
}

#[test]
fn modules_have_module_scope() {
    let c = Config::new("scope");
    c.write(
        "profiles/p/init.js",
        "var topVar = 1;\nfunction topFn() {}\nweb.command(\"S\", () => web.cmd(\"echo \" + [typeof globalThis.topVar, typeof globalThis.topFn, String(this)].join(\" \")));\n",
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "");
    assert_eq!(run(&mut k, "S"), "undefined undefined undefined");
}

/// Every refusal: the message names the rule and the path; nothing of the module runs.
fn refused(c: &Config, specifier: &str, rule: &str) {
    c.write(
        "profiles/p/init.js",
        &format!("import {specifier:?};\nweb.command(\"Ran\", () => {{}});\n"),
    );
    let start = Instant::now();
    let k = c.shell("p");
    assert!(start.elapsed() < Duration::from_secs(5), "{specifier}: {:?}", start.elapsed());
    let want = format!("init.js: import {specifier:?}: {rule}");
    assert!(k.message.starts_with(&want), "{specifier}: {:?} vs {want:?}", k.message);
    assert!(k.user_command_names().is_empty(), "{specifier}");
    assert!(
        !logs(&k).iter().any(|l| l.contains("read me")),
        "{specifier}: nothing of the file reached the script"
    );
}

#[test]
fn modules_imports_are_confined_to_the_config_directory() {
    let c = Config::new("confine");
    let web = c.web("");
    let web = web.to_str().unwrap().trim_end_matches('/');
    // A file next to the config directory, reachable by `../` and by its absolute path.
    std::fs::write(c.dir.join("outside.js"), "web.cmd(\"echo read me\");\n").unwrap();
    let outside = c.dir.join("outside.js").display().to_string();
    refused(&c, "../../../outside.js", &format!("outside {web}/: {outside}"));
    refused(&c, &outside, &format!("outside {web}/: {outside}"));
    refused(&c, "/etc/passwd.js", &format!("outside {web}/: /etc/passwd.js"));
    // A symlink inside pointing out (as a cloned repo could have).
    std::os::unix::fs::symlink(c.dir.join("outside.js"), c.web("profiles/p/link.js")).unwrap();
    refused(
        &c,
        "./link.js",
        &format!("outside {web}/: {}", c.web("profiles/p/link.js").display()),
    );
    std::os::unix::fs::symlink("/dev/zero", c.web("profiles/p/zero.js")).unwrap();
    refused(&c, "zero.js", &format!("outside {web}/: {}", c.web("profiles/p/zero.js").display()));
    // Not a regular file: a FIFO (opening it would block) and a folder.
    let fifo = c.web("profiles/p/fifo.js");
    let cpath = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: a plain mkfifo(3) on a NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    refused(&c, "./fifo.js", &format!("not a regular file: {}", fifo.display()));
    std::fs::create_dir_all(c.web("profiles/p/dir.js")).unwrap();
    refused(
        &c,
        "./dir.js",
        &format!("not a regular file: {}", c.web("profiles/p/dir.js").display()),
    );
    // Too large.
    let big = format!("// read me\n{}", " ".repeat(SCRIPT_MODULE_MAX as usize));
    c.write("profiles/p/big.js", &big);
    refused(
        &c,
        "./big.js",
        &format!(
            "larger than SCRIPT_MODULE_MAX ({SCRIPT_MODULE_MAX} bytes): {}",
            c.web("profiles/p/big.js").display()
        ),
    );
    // Not `.js`, even inside, and no extension guessing.
    c.write("profiles/p/data.json", "{}");
    refused(
        &c,
        "./data.json",
        &format!("not a .js file: {}", c.web("profiles/p/data.json").display()),
    );
    c.write("profiles/p/noext.js", "export const x = 1;\n");
    refused(&c, "./noext", &format!("not a .js file: {}", c.web("profiles/p/noext").display()));
    // A `.js` symlink inside to a non-`.js` file inside.
    std::os::unix::fs::symlink(c.web("profiles/p/data.json"), c.web("profiles/p/data.js")).unwrap();
    refused(
        &c,
        "./data.js",
        &format!("not a .js file: {}", c.web("profiles/p/data.js").display()),
    );
    // A dynamic import is refused the same way (its promise rejects with the rule).
    c.write(
        "profiles/p/init.js",
        &format!("web.command(\"Dyn\", () => import({outside:?}).catch(e => web.cmd(\"echo \" + e.message)));\n"),
    );
    let mut k = c.shell("p");
    assert_eq!(run(&mut k, "Dyn"), format!("import {outside:?}: outside {web}/: {outside}"));
}

#[test]
fn modules_an_init_file_that_is_not_a_regular_file_is_refused() {
    let c = Config::new("init-dir");
    std::fs::create_dir_all(c.web("profiles/p/init.js")).unwrap();
    let k = c.shell("p");
    assert!(k.message.starts_with("init.js: not a regular file"), "{}", k.message);
}
