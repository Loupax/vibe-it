//! C-EXTERNAL (KEYS.md "Opening a page in another browser", T99-T102). Each test names its row.

use crate::external::{argv, check_url, exists, resolve, spawn, Launch};
use crate::keys::*;

fn keys_at(url: &str) -> Keys {
    let mut k = Keys::new(None, "file:///help.html".into(), None);
    k.url = url.into();
    k.effects.clear();
    k
}

fn opened(k: &mut Keys) -> Vec<String> {
    k.effects
        .drain(..)
        .filter_map(|e| match e {
            Effect::OpenExternal(u) => Some(u),
            _ => None,
        })
        .collect()
}

/// A directory with executable files `names` (empty scripts), for `exists`/`resolve`.
fn bin_dir(tag: &str, names: &[&str]) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let d = std::env::temp_dir().join(format!("web-c-external-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for n in names {
        let p = d.join(n);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    d
}

#[test]
fn keys_t99_no_argument_hands_over_the_current_url() {
    let mut k = keys_at("https://a.test/page?q=1");
    k.dispatch_command("openExternal", &[]);
    assert_eq!(opened(&mut k), ["https://a.test/page?q=1"]);
}

#[test]
fn keys_t99_argument_is_normalised_like_go() {
    let mut k = keys_at("https://a.test/dir/page");
    k.dispatch_command("openExternal", &["example.com"]);
    assert_eq!(opened(&mut k), ["https://example.com/"]);
    k.dispatch_command("openExternal", &["http://b.test/x"]);
    assert_eq!(opened(&mut k), ["http://b.test/x"]);
    k.dispatch_command("openExternal", &["../up"]);
    assert_eq!(opened(&mut k), ["https://a.test/up"]);
}

#[test]
fn keys_t100_other_schemes_are_refused_with_a_message() {
    for bad in ["javascript:alert(1)", "data:text/html,hi", "about:blank", "mailto:a@b.test"] {
        let mut k = keys_at("https://a.test/");
        k.dispatch_command("openExternal", &[bad]);
        assert!(opened(&mut k).is_empty(), "{bad}");
        assert!(k.message.contains("refused"), "{bad}: {}", k.message);
    }
    let mut k = keys_at("about:blank");
    k.dispatch_command("openExternal", &[]);
    assert!(opened(&mut k).is_empty());
    assert!(k.message.contains("refused"));
    let mut k = keys_at("");
    k.dispatch_command("openExternal", &[]);
    assert!(opened(&mut k).is_empty());
    assert!(k.message.contains("no page"));
    assert_eq!(check_url("file:///tmp/a.html").unwrap(), "file:///tmp/a.html");
}

#[test]
fn keys_t100_two_arguments_print_the_usage() {
    let mut k = keys_at("https://a.test/");
    k.dispatch_command("openExternal", &["a", "b"]);
    assert!(opened(&mut k).is_empty());
    assert_eq!(k.message, "web: usage: :openExternal [url]");
}

#[test]
fn keys_t101_argv_has_the_url_as_one_element() {
    let url = "https://a.test/a%20b?x=1&y=;rm -rf";
    let t = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(argv(&t(&["firefox"]), url), t(&["firefox", url]));
    assert_eq!(
        argv(&t(&["firefox", "--new-window", "%s"]), url),
        t(&["firefox", "--new-window", url])
    );
    assert_eq!(argv(&t(&["b", "--app=%s"]), url), t(&["b", &format!("--app={url}")]));
}

#[test]
fn keys_t101_browser_variable_takes_the_first_existing_entry() {
    let d = bin_dir("browser", &["second", "xdg-open"]);
    let path = d.to_str().unwrap();
    let url = "https://a.test/";
    // The first entry does not exist; the second does and has its own argument and %s.
    let l = resolve(&[], Some("missing:second --tab %s"), path, url).unwrap();
    assert_eq!(l.argv, ["second", "--tab", url]);
    // A path entry.
    let full = format!("{path}/second");
    let l = resolve(&[], Some(&full), "", url).unwrap();
    assert_eq!(l.argv, [full.as_str(), url]);
    // Nothing usable in $BROWSER, or unset: xdg-open.
    assert_eq!(resolve(&[], Some("missing:gone"), path, url).unwrap().argv, ["xdg-open", url]);
    assert_eq!(resolve(&[], None, path, url).unwrap().argv, ["xdg-open", url]);
    assert_eq!(resolve(&[], Some(""), path, url).unwrap().argv, ["xdg-open", url]);
    // The configured argv wins over $BROWSER.
    let l = resolve(&["second", "-x"], Some("xdg-open"), path, url).unwrap();
    assert_eq!(l.argv, ["second", "-x", url]);
    // No program: an error naming it.
    let e = resolve(&[], None, "", url).unwrap_err();
    assert!(e.contains("xdg-open") && e.contains("not found"), "{e}");
    assert!(!exists("second", ""));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn keys_t102_spawn_runs_detached_with_the_single_url_argument() {
    use std::os::unix::fs::PermissionsExt;
    let d = bin_dir("spawn", &[]);
    let out = d.join("argv.txt");
    let script = d.join("rec");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> {}; done\n", out.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let url = "https://a.test/x y;touch pwned";
    let launch = Launch {
        argv: vec![script.to_str().unwrap().into(), url.into()],
    };
    spawn(&launch).unwrap();
    for _ in 0..100 {
        if out.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), format!("{url}\n"));
    assert!(!d.join("pwned").exists());
    let bad = Launch {
        argv: vec!["/nonexistent/prog".into(), url.into()],
    };
    assert!(spawn(&bad).unwrap_err().contains("/nonexistent/prog"));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn keys_t99_openexternal_is_a_command_and_completes() {
    assert!(COMMAND_NAMES.contains(&"openExternal"));
    assert!(crate::config::COMPLETERS.iter().any(|(n, _)| *n == "openExternal"));
}
