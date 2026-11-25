//! QV8-0: records the commit the binary was built from (for `--perf-report`).
use std::process::Command;

fn vcs(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

fn main() {
    if let Some(c) = vcs(&["rev-parse", "--short=10", "HEAD"]) {
        let dirty = vcs(&["status", "--porcelain", "--untracked-files=no"]).is_some();
        println!("cargo:rustc-env=SCRATCHY_GIT_COMMIT={c}{}", if dirty { "-dirty" } else { "" });
    }
    for what in ["HEAD", "refs"] {
        if let Some(p) = vcs(&["rev-parse", "--git-path", what]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
