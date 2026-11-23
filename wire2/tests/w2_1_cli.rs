//! W2-1: command line and security refusals.

mod common;

use std::process::{Command, Stdio};
use std::time::Duration;

use common::*;

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(BIN)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn unknown_and_not_yet_flags_are_errors() {
    let (code, err) = run(&["--bogus"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("unknown argument \"--bogus\""), "{err}");
    let (code, err) = run(&["--filter", "unix:/tmp/pass.sock"]);
    assert_eq!(code, 2);
    assert!(err.contains("--filter is not supported yet"), "{err}");
    let (code, err) = run(&["--ca-dir", "/tmp/x"]);
    assert_eq!(code, 2);
    assert!(err.contains("--ca-dir is not supported yet"), "{err}");
    let (code, _) = run(&["--help"]);
    assert_eq!(code, 0);
}

/// SPEC: refuses a non-loopback --listen without --allow-remote.
#[test]
fn non_loopback_needs_allow_remote() {
    for addr in ["0.0.0.0:0", "[::]:0", "192.0.2.1:8080"] {
        let (code, err) = run(&["--listen", addr]);
        assert_eq!(code, 2, "{addr}: {err}");
        assert!(
            err.contains("not a loopback address") && err.contains("--allow-remote"),
            "{err}"
        );
    }
    let p = Proxy::start(&["--listen", "0.0.0.0:0", "--allow-remote"]);
    assert!(
        p.wait_log(
            |l| l.contains("--allow-remote: anyone who can reach"),
            Duration::from_secs(5)
        )
        .is_some()
    );
}

/// SPEC: refuses to run as root. Uses a user namespace (`unshare -r`) to be uid 0 without privileges.
#[test]
fn refuses_root() {
    let probe = Command::new("unshare").args(["-r", "id", "-u"]).output();
    match probe {
        Ok(o) if o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "0" => {}
        _ => {
            eprintln!("SKIP: unshare -r not available (user namespaces off?)");
            return;
        }
    }
    let out = Command::new("unshare")
        .args(["-r", BIN, "--listen", "127.0.0.1:0"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("refusing to run as root"), "{err}");
    assert!(!err.contains("listening on"), "{err}");
}

#[test]
fn listen_port_in_use_is_an_error() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let (code, err) = run(&["--listen", &addr]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains(&addr), "{err}");
}
