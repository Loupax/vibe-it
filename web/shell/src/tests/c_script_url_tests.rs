//! C-SCRIPT-URL (SCRIPTING.md "Modules", "web:url"; DECISIONS.md 2026-11-26): the built-in modules
//! `web:api` (the `web` object) and `web:url` (a read-only WHATWG `URL` on the `url` crate).

use crate::keys::*;
use crate::protocol::Command;
use crate::script::stack_location;

use super::c_script_modules_tests::{run, Config};
use super::c_script_tests::{logs, press, sent, shell, type_line};

/// A shell (no config directory) with `URL` imported and `:E` echoing `expr` as JSON; the parsed echo.
fn url_eval(expr: &str) -> serde_json::Value {
    let mut k = shell(&format!(
        "import {{ URL }} from \"web:url\";\nweb.command(\"E\", () => web.cmd(\"echo \" + JSON.stringify({expr})));\n"
    ));
    assert_eq!(k.message, "", "{:?}", logs(&k));
    type_line(&mut k, "E");
    serde_json::from_str(&k.message).unwrap_or_else(|e| panic!("{e}: {}", k.message))
}

/// Every property of `new URL(args)` as an object.
fn parts(args: &str) -> serde_json::Value {
    url_eval(&format!(
        "(() => {{ const u = new URL({args}); return Object.fromEntries([\"href\", \"protocol\", \"username\", \"password\", \"host\", \"hostname\", \"port\", \"origin\", \"pathname\", \"search\", \"hash\"].map(k => [k, u[k]])); }})()"
    ))
}

fn opened(k: &mut Keys) -> Vec<String> {
    sent(k)
        .into_iter()
        .filter_map(|c| match c {
            Command::Open { url } => Some(url),
            _ => None,
        })
        .collect()
}

// ---- resolution ----

#[test]
fn url_builtins_import_with_no_config_directory() {
    let mut k = shell(
        r#"import { URL } from "web:url";
import w from "web:api";
web.command("T", () => web.cmd("echo " + [typeof URL, w === web, new URL("https://a.test/x").pathname].join()));
"#,
    );
    assert_eq!(k.message, "", "{:?}", logs(&k));
    type_line(&mut k, "T");
    assert_eq!(k.message, "function,true,/x");
}

#[test]
fn url_builtins_are_the_same_objects_in_init_and_a_lib_module() {
    let c = Config::new("url-same");
    c.write(
        "lib/m.js",
        "import { URL } from \"web:url\";\nimport web from \"web:api\";\nimport { cmd } from \"web:api\";\nexport { URL, web, cmd };\nexport const u = new URL(\"https://a.test/\");\n",
    );
    c.write(
        "profiles/p/init.js",
        r#"import { URL } from "web:url";
import w, { cmd } from "web:api";
import * as m from "../../lib/m.js";
web.command("S", () => web.cmd("echo " + [m.URL === URL, m.web === w, m.cmd === cmd, m.u instanceof URL, new URL("https://b.test/") instanceof m.URL].join()));
"#,
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "", "{:?}", logs(&k));
    assert_eq!(run(&mut k, "S"), "true,true,true,true,true");
}

#[test]
fn url_builtins_dynamic_import() {
    let mut k = shell(
        r#"web.command("D", async () => {
  const u = await import("web:url");
  const a = await import("web:api");
  web.cmd("echo " + [typeof u.URL, "default" in u, a.default === web, a.cmd === web.cmd, Object.keys(u).join("+")].join());
});
"#,
    );
    type_line(&mut k, "D");
    assert_eq!(k.message, "function,false,true,true,URL");
}

#[test]
fn url_unknown_builtin_is_refused() {
    for spec in ["web:nope", "web:", "web:URL", "web:url.js", "web:api/x"] {
        let k = shell(&format!("import {{ URL }} from {spec:?};\nweb.command(\"Ran\", () => {{}});\n"));
        let want = format!("init.js: import {spec:?}: no such built-in module");
        assert!(k.message.starts_with(&want), "{spec}: {:?}", k.message);
        assert!(k.user_command_names().is_empty(), "{spec}: nothing of the file ran");
    }
    // Dynamic: the promise rejects with the same message.
    let mut k = shell(r#"web.command("Dyn", () => import("web:nope").catch(e => web.cmd("echo " + e.message)));"#);
    type_line(&mut k, "Dyn");
    assert_eq!(k.message, "import \"web:nope\": no such built-in module");
}

#[test]
fn url_bare_url_is_still_a_file_import() {
    let c = Config::new("url-bare");
    c.write(
        "profiles/p/init.js",
        "import { URL } from \"url\";\nweb.command(\"Ran\", () => {});\n",
    );
    let k = c.shell("p");
    let want = format!("init.js: import \"url\": not a .js file: {}", c.web("profiles/p/url").display());
    assert!(k.message.starts_with(&want), "{:?} vs {want:?}", k.message);
    assert!(k.user_command_names().is_empty());
}

#[test]
fn url_a_file_named_like_a_builtin_is_a_file() {
    let c = Config::new("url-file");
    c.write("profiles/p/web:url.js", "export const URL = \"the file\";\n");
    let abs = c.web("profiles/p/web:url.js").display().to_string();
    c.write(
        "profiles/p/init.js",
        &format!(
            "import {{ URL }} from \"./web:url.js\";\nimport {{ URL as Abs }} from {abs:?};\nweb.command(\"F\", () => web.cmd(\"echo \" + URL + \"|\" + Abs));\n"
        ),
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "", "{:?}", logs(&k));
    assert_eq!(run(&mut k, "F"), "the file|the file");
}

#[test]
fn url_has_no_default_export() {
    let k = shell("import URL from \"web:url\";\nweb.command(\"Ran\", () => {});\n");
    assert!(k.message.starts_with("init.js"), "{}", k.message);
    assert!(k.message.contains("default"), "{}", k.message);
    assert!(k.user_command_names().is_empty());
}

#[test]
fn url_adds_no_globals() {
    let names = "web.command(\"G\", () => web.cmd(\"echo \" + JSON.stringify(Object.getOwnPropertyNames(globalThis).sort())));\n";
    let mut plain = shell(&format!("if (typeof URL !== \"undefined\") throw new Error(\"URL\");\n{names}"));
    assert_eq!(plain.message, "", "{:?}", logs(&plain));
    type_line(&mut plain, "G");
    let mut imported = shell(&format!(
        "import {{ URL as U }} from \"web:url\";\nimport * as api from \"web:api\";\nnew U(\"https://a.test/\");\nif (typeof URL !== \"undefined\") throw new Error(\"URL\");\n{names}"
    ));
    assert_eq!(imported.message, "", "{:?}", logs(&imported));
    type_line(&mut imported, "G");
    assert_eq!(plain.message, imported.message, "the same globals with and without the imports");
    assert!(!plain.message.contains("\"URL\""));
}

#[test]
fn url_builtins_after_source() {
    let c = Config::new("url-source");
    c.write(
        "profiles/p/init.js",
        "import { URL } from \"web:url\";\nimport w from \"web:api\";\nweb.command(\"V\", () => w.cmd(\"echo \" + new URL(\"https://h.test/p\").hostname));\n",
    );
    let mut k = c.shell("p");
    assert_eq!(run(&mut k, "V"), "h.test");
    assert_eq!(run(&mut k, "source"), "sourced 1 file(s)");
    assert_eq!(run(&mut k, "V"), "h.test");
}

// ---- web:api ----

#[test]
fn api_default_and_named_exports_follow_web() {
    let mut k = shell(
        r#"import * as ns from "web:api";
web.command("K", () => {
  const keys = Object.keys(ns).filter(k => k !== "default").sort();
  web.cmd("echo " + [ns.default === globalThis.web, keys.join("+") === Object.keys(web).sort().join("+"), keys.every(k => ns[k] === web[k]), keys.join("+")].join());
});
"#,
    );
    assert_eq!(k.message, "", "{:?}", logs(&k));
    type_line(&mut k, "K");
    assert_eq!(k.message, "true,true,true,bookmarks+buffers+cmd+command+history+map+on");
}

#[test]
fn api_bare_functions_and_shared_settings() {
    let mut k = shell(
        r#"import { cmd, map, command, buffers } from "web:api";
buffers.focusExisting = true;
map("n", "gx", () => cmd("go /mapped"));
command("G", () => cmd("go /x"));
command("F", () => cmd("echo " + web.buffers.focusExisting));
"#,
    );
    assert_eq!(k.message, "", "{:?}", logs(&k));
    k.script_ready();
    k.effects.clear();
    type_line(&mut k, "G");
    assert_eq!(opened(&mut k), vec!["https://a.test/x".to_string()]);
    press(&mut k, KeyInput::ch('g'));
    press(&mut k, KeyInput::ch('x'));
    assert_eq!(opened(&mut k), vec!["https://a.test/mapped".to_string()]);
    type_line(&mut k, "F");
    assert_eq!(k.message, "true");
}

#[test]
fn api_gives_web_back_after_the_global_is_overwritten() {
    let c = Config::new("api-back");
    c.write(
        "lib/back.js",
        r#"import web from "web:api";
web.command("Back", () => web.cmd("go /back"));
web.map("n", "gb", () => web.cmd("go /mapped"));
"#,
    );
    c.write(
        "profiles/p/init.js",
        "globalThis.web = null;\nawait import(\"../../lib/back.js\");\nif (globalThis.web !== null) throw new Error(\"global not overwritten\");\n",
    );
    let mut k = c.shell("p");
    assert_eq!(k.message, "", "{:?}", logs(&k));
    k.script_ready();
    k.effects.clear();
    type_line(&mut k, "Back");
    assert_eq!(opened(&mut k), vec!["https://a.test/back".to_string()]);
    press(&mut k, KeyInput::ch('g'));
    press(&mut k, KeyInput::ch('b'));
    assert_eq!(opened(&mut k), vec!["https://a.test/mapped".to_string()]);
    // Deleted instead of overwritten, imported dynamically from init.js itself.
    let mut k = shell(
        r#"delete globalThis.web;
if (typeof web !== "undefined") throw new Error("still there");
const { default: w } = await import("web:api");
w.command("Back", () => w.cmd("go /back"));
"#,
    );
    assert_eq!(k.message, "", "{:?}", logs(&k));
    k.script_ready();
    k.effects.clear();
    type_line(&mut k, "Back");
    assert_eq!(opened(&mut k), vec!["https://a.test/back".to_string()]);
}

#[test]
fn api_the_global_is_unchanged() {
    let mut k = shell(
        r#"web.command("D", () => {
  const d = Object.getOwnPropertyDescriptor(globalThis, "web");
  web.cmd("echo " + [d.writable, d.configurable, Object.isFrozen(web)].join());
});
"#,
    );
    type_line(&mut k, "D");
    assert_eq!(k.message, "true,true,true");
}

// ---- web:url, the class ----

#[test]
fn url_properties_of_a_slack_url() {
    let p = parts("\"https://app.slack.com/client/E02996YDSAK/C123?x=1#m\"");
    assert_eq!(p["href"], "https://app.slack.com/client/E02996YDSAK/C123?x=1#m");
    assert_eq!(p["protocol"], "https:");
    assert_eq!(p["username"], "");
    assert_eq!(p["password"], "");
    assert_eq!(p["hostname"], "app.slack.com");
    assert_eq!(p["host"], "app.slack.com");
    assert_eq!(p["port"], "");
    assert_eq!(p["origin"], "https://app.slack.com");
    assert_eq!(p["pathname"], "/client/E02996YDSAK/C123");
    assert_eq!(p["search"], "?x=1");
    assert_eq!(p["hash"], "#m");
}

#[test]
fn url_ports() {
    let p = parts("\"http://a.test:8080/\"");
    assert_eq!((p["port"].as_str(), p["host"].as_str()), (Some("8080"), Some("a.test:8080")));
    assert_eq!(p["origin"], "http://a.test:8080");
    let p = parts("\"https://a.test:443/\"");
    assert_eq!((p["port"].as_str(), p["host"].as_str()), (Some(""), Some("a.test")));
    assert_eq!(p["href"], "https://a.test/");
}

#[test]
fn url_relative_with_base() {
    assert_eq!(parts("\"../x?y\", \"https://a.test/b/c/d\"")["href"], "https://a.test/b/x?y");
    assert_eq!(
        parts("\"/z\", new URL(\"https://a.test/b/c\")")["href"],
        "https://a.test/z",
        "a URL works as the base"
    );
    assert_eq!(
        parts("new URL(\"https://a.test/q\")")["href"],
        "https://a.test/q",
        "and as the input"
    );
    assert_eq!(parts("\"https://a.test/\", undefined")["href"], "https://a.test/");
}

#[test]
fn url_invalid_input_throws_type_error() {
    let v = url_eval(
        r#"[["not a url"], [""], ["x", "also not a url"], ["https://a.test/", null]].map(a => {
  try { new URL(...a); return "no throw"; } catch (e) { return [e instanceof TypeError, e.message]; }
})"#,
    );
    assert_eq!(
        v,
        serde_json::json!([
            [true, "Invalid URL: not a url"],
            [true, "Invalid URL: "],
            [true, "Invalid URL: x"],
            [true, "Invalid URL: https://a.test/"]
        ])
    );
    let v = url_eval(r#"(() => { try { URL("https://a.test"); return "no throw"; } catch (e) { return e instanceof TypeError; } })()"#);
    assert_eq!(v, true, "calling URL without new");
}

#[test]
fn url_uncaught_invalid_url_is_shown_at_the_callers_line() {
    let k = shell("import { URL } from \"web:url\";\n\nnew URL(\"nope\");\n");
    assert_eq!(k.message, "init.js:3: Invalid URL: nope");
    assert_eq!(
        stack_location("    at URL (web:url:7:13)\n    at <eval> (init.js:3:1)\n").as_deref(),
        Some("init.js:3"),
        "a built-in's frame is skipped"
    );
}

#[test]
fn url_opaque_origins_idn_userinfo_empty_parts() {
    let p = parts("\"about:blank\"");
    assert_eq!((p["protocol"].as_str(), p["hostname"].as_str()), (Some("about:"), Some("")));
    assert_eq!(p["origin"], "null");
    let p = parts("\"file:///tmp/x\"");
    assert_eq!(p["pathname"], "/tmp/x");
    assert_eq!(p["origin"], "null");
    assert_eq!(parts("\"https://bücher.example/\"")["hostname"], "xn--bcher-kva.example");
    let p = parts("\"https://u:p@a.test/\"");
    assert_eq!((p["username"].as_str(), p["password"].as_str()), (Some("u"), Some("p")));
    let p = parts("\"https://a.test/?#\"");
    assert_eq!((p["search"].as_str(), p["hash"].as_str()), (Some(""), Some("")));
    assert_eq!(p["href"], "https://a.test/?#");
    let p = parts("\"http://[::1]:8000/\"");
    assert_eq!((p["hostname"].as_str(), p["host"].as_str()), (Some("[::1]"), Some("[::1]:8000")));
}

#[test]
fn url_to_string_and_json() {
    let v = url_eval(
        r#"(() => { const u = new URL("https://a.test/p?q#h"); return [String(u), `${u}`, u.toString(), u.toJSON(), JSON.stringify({ u })]; })()"#,
    );
    let h = "https://a.test/p?q#h";
    assert_eq!(v, serde_json::json!([h, h, h, h, format!("{{\"u\":\"{h}\"}}")]));
}

#[test]
fn url_can_parse_and_parse() {
    let v = url_eval(
        r#"[URL.canParse("https://a.test/"), URL.canParse("nope"), URL.canParse("x", "https://a.test/"), URL.canParse(Symbol()),
            URL.parse("https://a.test/p") instanceof URL, URL.parse("https://a.test/p").pathname, URL.parse("nope"), URL.parse("../y", "https://a.test/a/b").href]"#,
    );
    assert_eq!(
        v,
        serde_json::json!([true, false, true, false, true, "/p", null, "https://a.test/y"])
    );
}

#[test]
fn url_is_read_only_and_frozen() {
    let v = url_eval(
        r#"(() => {
  const u = new URL("https://a.test/");
  const r = [];
  try { u.hostname = "x"; r.push("assigned"); } catch (e) { r.push(e instanceof TypeError); }
  try { Object.defineProperty(URL.prototype, "hostname", { get() { return "x"; } }); r.push("redefined"); } catch (e) { r.push(e instanceof TypeError); }
  try { URL.parse = () => null; r.push("replaced"); } catch (e) { r.push(e instanceof TypeError); }
  r.push(u.hostname, Object.isFrozen(URL), Object.isFrozen(URL.prototype));
  r.push(Object.keys(URL.prototype).sort().join("+"));
  r.push(Object.getOwnPropertyDescriptor(URL.prototype, "href").set === undefined);
  return r;
})()"#,
    );
    assert_eq!(
        v,
        serde_json::json!([
            true,
            true,
            true,
            "a.test",
            true,
            true,
            "hash+host+hostname+href+origin+password+pathname+port+protocol+search+username",
            true
        ])
    );
}

// ---- the user's case ----

/// Buffers 1..=6, 3 and 5 on Slack (when `slack`), 6 current; the `gs` mapping from SCRIPTING.md.
fn slack_shell(slack: bool) -> Keys {
    let mut k = shell(
        r#"import { URL } from "web:url";

// gs: go to the first Slack buffer, or open Slack.
web.map("n", "gs", () => {
  const b = web.buffers.list().find(b => URL.parse(b.url)?.hostname === "app.slack.com");
  web.cmd(b ? "b " + b.number : "new https://app.slack.com");
});
"#,
    );
    assert_eq!(k.message, "", "{:?}", logs(&k));
    k.script_ready();
    let host = if slack { "app.slack.com" } else { "other.test" };
    let urls = [
        "https://two.test/".to_string(),
        format!("https://{host}/client/E1/C1"),
        "https://four.test/".to_string(),
        format!("https://{host}/client/E1/C2"),
        "https://six.test/".to_string(),
    ];
    for (i, u) in urls.iter().enumerate() {
        type_line(&mut k, &format!("new {u}"));
        k.buffers.tab_created(i as u64 + 2, Some(i as u64 + 1));
    }
    assert_eq!(k.buffers.list.len(), 6);
    assert_eq!(k.buffers.current, 6);
    k.effects.clear();
    k
}

#[test]
fn url_slack_mapping_goes_to_the_first_slack_buffer() {
    let mut k = slack_shell(true);
    press(&mut k, KeyInput::ch('g'));
    press(&mut k, KeyInput::ch('s'));
    assert_eq!(k.buffers.current, 3, "switched to the first slack buffer");
    assert_eq!(k.buffers.list.len(), 6);
}

#[test]
fn url_slack_mapping_opens_slack_when_there_is_none() {
    let mut k = slack_shell(false);
    press(&mut k, KeyInput::ch('g'));
    press(&mut k, KeyInput::ch('s'));
    let new: Vec<_> = sent(&mut k)
        .into_iter()
        .filter_map(|c| match c {
            Command::NewTab { url, .. } => url,
            _ => None,
        })
        .collect();
    assert_eq!(new.len(), 1, "{new:?}");
    assert!(new[0].starts_with("https://app.slack.com"), "{new:?}");
    assert_eq!(k.buffers.list.len(), 7);
}
