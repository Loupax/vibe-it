//! Starting the engine host (web/PROTOCOL.md "Host command line") and the paths it gets.

use crate::args::{Args, Engine};
use std::path::{Path, PathBuf};

/// The state directory (STASH.md "State directory"): `--state-dir` as given, else the base
/// (`$WEB_STATE_DIR`, `$XDG_STATE_HOME/web`, `~/.local/state/web`); with `--profile NAME` the
/// base gets `/profiles/NAME` (C-PROFILE: `$WEB_STATE_DIR` is a base, so profiles stay apart
/// under it; only `--state-dir` names the exact directory).
pub fn state_dir(args: &Args, env: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    if let Some(d) = &args.state_dir {
        return Ok(d.clone());
    }
    let set = |k: &str| env(k).filter(|v| !v.is_empty());
    let base = if let Some(d) = set("WEB_STATE_DIR") {
        PathBuf::from(d)
    } else if let Some(d) = set("XDG_STATE_HOME") {
        Path::new(&d).join("web")
    } else if let Some(h) = set("HOME") {
        Path::new(&h).join(".local/state/web")
    } else {
        return Err("no state directory: set --state-dir, WEB_STATE_DIR, XDG_STATE_HOME or HOME".into());
    };
    Ok(match &args.profile {
        Some(name) => base.join("profiles").join(name),
        None => base,
    })
}

/// The engine's profile inside the state directory: `DIR/engines/<engine>/`.
pub fn profile_dir(state: &Path, engine: Engine) -> PathBuf {
    state.join("engines").join(engine.name())
}

/// The engine host binary: `--host-bin`, `$WEB_HOST_CEF`, `web-host-cef` next to this
/// binary if it is there, else `web-host-cef` from PATH.
pub fn host_bin(args: &Args, env: &dyn Fn(&str) -> Option<String>, exe: Option<&Path>) -> PathBuf {
    if let Some(b) = &args.host_bin {
        return b.clone();
    }
    if let Some(b) = env("WEB_HOST_CEF").filter(|v| !v.is_empty()) {
        return PathBuf::from(b);
    }
    if let Some(sibling) = exe.and_then(|e| e.parent()).map(|d| d.join("web-host-cef")) {
        if sibling.is_file() {
            return sibling;
        }
    }
    PathBuf::from("web-host-cef")
}

/// What to run: argv (without the program) and the environment changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
}

pub fn host_command(args: &Args, program: PathBuf, profile: &Path, control: &Path, wayland_display: &str) -> HostCommand {
    let mut a = vec![
        format!("--profile={}", profile.display()),
        format!("--control={}", control.display()),
        // The host draws into this shell, never into the desktop the shell runs on.
        "--ozone-platform=wayland".to_string(),
        // C6e: Chromium's Wayland IME (off by default) on text-input-v3, which the shell offers
        // (`ime.rs`); without the version Chromium may pick text-input-v1, which it does not.
        "--enable-wayland-ime".to_string(),
        "--wayland-text-input-version=3".to_string(),
    ];
    if let Some(p) = &args.proxy {
        a.push(format!("--proxy={p}"));
    }
    if let Some(u) = &args.url {
        a.push(format!("--url={u}"));
    }
    if let Some(s) = notify_sites_arg(crate::config::NOTIFY_SITES) {
        a.push(s);
    }
    a.extend(args.host_args.iter().cloned());
    HostCommand {
        program,
        args: a,
        env: vec![
            ("WAYLAND_DISPLAY".into(), wayland_display.into()),
            // Keeps Chromium from moving itself out of the caller's (memory-capped) systemd
            // scope through the xdg portal (web/spike/README.md W1, host-cef/run.sh).
            ("FLATPAK_SANDBOX_DIR".into(), profile.display().to_string()),
        ],
        // On an X11 desktop the host must still talk Wayland to the shell, not X11 to the
        // desktop; WAYLAND_SOCKET would bypass WAYLAND_DISPLAY.
        env_remove: vec!["DISPLAY".into(), "WAYLAND_SOCKET".into()],
    }
}

/// C-BELL: config.rs `NOTIFY_SITES` as the host's option (`All`: none, the host's default).
pub fn notify_sites_arg(sites: crate::config::NotifySites) -> Option<String> {
    match sites {
        crate::config::NotifySites::All => None,
        crate::config::NotifySites::Only(list) => Some(format!("--notify-sites={}", list.join(","))),
    }
}

impl HostCommand {
    pub fn spawn(&self) -> std::io::Result<std::process::Child> {
        let mut c = std::process::Command::new(&self.program);
        c.args(&self.args);
        for k in &self.env_remove {
            c.env_remove(k);
        }
        for (k, v) in &self.env {
            c.env(k, v);
        }
        c.stdin(std::process::Stdio::null());
        c.spawn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::parse;

    fn args(a: &[&str]) -> Args {
        parse(a.iter().map(|s| s.to_string())).unwrap().unwrap()
    }

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn state_dir_order() {
        let e = env_of(&[("WEB_STATE_DIR", "/w"), ("XDG_STATE_HOME", "/x"), ("HOME", "/h")]);
        assert_eq!(state_dir(&args(&["--state-dir", "/s"]), &e).unwrap(), PathBuf::from("/s"));
        assert_eq!(state_dir(&args(&[]), &e).unwrap(), PathBuf::from("/w"));
        let e = env_of(&[("WEB_STATE_DIR", ""), ("XDG_STATE_HOME", "/x"), ("HOME", "/h")]);
        assert_eq!(state_dir(&args(&[]), &e).unwrap(), PathBuf::from("/x/web"));
        let e = env_of(&[("HOME", "/h")]);
        assert_eq!(state_dir(&args(&[]), &e).unwrap(), PathBuf::from("/h/.local/state/web"));
        assert!(state_dir(&args(&[]), &env_of(&[])).is_err());
        // C-PROFILE: the base gets profiles/NAME; --state-dir is exact.
        let e = env_of(&[("XDG_STATE_HOME", "/x"), ("HOME", "/h")]);
        assert_eq!(
            state_dir(&args(&["--profile", "p"]), &e).unwrap(),
            PathBuf::from("/x/web/profiles/p")
        );
        assert_eq!(
            state_dir(&args(&["--profile=p", "--state-dir=/s"]), &e).unwrap(),
            PathBuf::from("/s")
        );
        let e = env_of(&[("WEB_STATE_DIR", "/w"), ("XDG_STATE_HOME", "/x")]);
        assert_eq!(state_dir(&args(&["--profile=p"]), &e).unwrap(), PathBuf::from("/w/profiles/p"));
        let e = env_of(&[("HOME", "/h")]);
        assert_eq!(
            state_dir(&args(&["--profile=p"]), &e).unwrap(),
            PathBuf::from("/h/.local/state/web/profiles/p")
        );
        assert!(state_dir(&args(&["--profile=p"]), &env_of(&[])).is_err());
        assert_eq!(profile_dir(Path::new("/s"), Engine::Cef), PathBuf::from("/s/engines/cef"));
    }

    #[test]
    fn host_bin_order() {
        let e = env_of(&[("WEB_HOST_CEF", "/env/host")]);
        assert_eq!(host_bin(&args(&["--host-bin", "/x"]), &e, None), PathBuf::from("/x"));
        assert_eq!(host_bin(&args(&[]), &e, None), PathBuf::from("/env/host"));
        assert_eq!(
            host_bin(&args(&[]), &env_of(&[]), Some(Path::new("/nonexistent/web"))),
            PathBuf::from("web-host-cef")
        );
    }

    #[test]
    fn command_without_proxy() {
        let c = host_command(
            &args(&[]),
            "h".into(),
            Path::new("/s/engines/cef"),
            Path::new("/r/control.sock"),
            "wayland-3",
        );
        assert_eq!(
            c.args,
            vec![
                "--profile=/s/engines/cef",
                "--control=/r/control.sock",
                "--ozone-platform=wayland",
                "--enable-wayland-ime",
                "--wayland-text-input-version=3"
            ]
        );
        assert!(!c.args.iter().any(|a| a.contains("proxy")), "no --proxy: direct, never an error");
        assert!(c.env.contains(&("WAYLAND_DISPLAY".into(), "wayland-3".into())));
        assert!(c.env_remove.contains(&"DISPLAY".to_string()));
    }

    #[test]
    fn command_passes_proxy_url_and_host_args() {
        let a = args(&["--proxy", "127.0.0.1:3128", "example.com", "--", "--proxy-bypass-list=<-loopback>"]);
        let c = host_command(&a, "h".into(), Path::new("/p"), Path::new("/c"), "wayland-1");
        assert_eq!(
            &c.args[5..],
            ["--proxy=127.0.0.1:3128", "--url=example.com", "--proxy-bypass-list=<-loopback>"]
        );
    }
}
