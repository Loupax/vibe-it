//! The running instance's command socket: `web --forget-site <site>` (P-FORGET-SHELL), `web URL`
//! handing the URL to the running instance (C-SINGLE), and the run directory's housekeeping
//! (C-SOCKDIR). The socket keeps its file name `forget.sock` (a shell started before C-SINGLE
//! must still be found); it takes one request line per connection, `forget-site <site>` or
//! `open <url>`, and answers with one JSON line.
//!
//! How `web --forget-site` finds a running web for the same state directory: the running
//! shell writes its pid to `<state dir>/web.pid` and listens on
//! `$XDG_RUNTIME_DIR/web-<pid>/forget.sock` (next to the control socket, so the socket path
//! stays short however deep the state directory is). The command reads the pid file, checks
//! that the pid is a live `web` (`/proc/<pid>/comm`), connects, sends `forget-site <site>` and
//! reads one line back: the host's `forgot` event as JSON (or an `error` event). A stale pid
//! file (a killed shell), a pid that is not a web, or a socket that refuses the connection all
//! mean "no instance": the command then runs `web-host-cef --no-window --forget-site` itself.
//! The exit code is 0 for `ok`, 1 for `partial`, 2 for a refusal, as the host's.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::protocol::Event;

pub const PID_FILE: &str = "web.pid";
pub const FORGET_SOCK: &str = "forget.sock";
/// The shell's `/proc/<pid>/comm` (the binary's name).
pub const SHELL_COMM: &str = "web";

/// Every file the shell puts into its run directory (removed by explicit name, never a glob).
const RUN_DIR_FILES: &[&str] = &["control.sock", FORGET_SOCK, crate::control::HELP_FILE];

pub fn run_dir(runtime: &Path, pid: u32) -> PathBuf {
    runtime.join(format!("web-{pid}"))
}

/// Is `pid` a running web shell? (`/proc/<pid>/comm` is the binary's name, "web"; the engine
/// host's is "web-host-cef".)
pub fn is_web_shell(proc_root: &Path, pid: u32) -> bool {
    std::fs::read_to_string(proc_root.join(pid.to_string()).join("comm"))
        .map(|c| c.trim_end() == SHELL_COMM)
        .unwrap_or(false)
}

/// Removes a run directory: the files the shell makes, by name, then the directory itself
/// (which stays if anything else is in it).
pub fn remove_run_dir(dir: &Path) {
    for f in RUN_DIR_FILES {
        let _ = std::fs::remove_file(dir.join(f));
    }
    let _ = std::fs::remove_dir(dir);
}

/// `web-<pid>` directories under `runtime` whose pid is not a running web shell (and not
/// `own_pid`). Only real directories named `web-` + digits count.
pub fn stale_run_dirs(runtime: &Path, own_pid: u32, proc_root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(runtime) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix("web-"))
            .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|d| d.parse::<u32>().ok())
        else {
            continue;
        };
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && pid != own_pid && !is_web_shell(proc_root, pid) {
            out.push(e.path());
        }
    }
    out.sort();
    out
}

/// C-SOCKDIR: at start, removes the run directories a hard-killed shell left behind.
pub fn clean_stale_run_dirs(runtime: &Path) -> Vec<PathBuf> {
    let stale = stale_run_dirs(runtime, std::process::id(), Path::new("/proc"));
    for d in &stale {
        remove_run_dir(d);
    }
    stale
}

// ---- the pid file

pub fn write_pid_file(state_dir: &Path) -> std::io::Result<()> {
    std::fs::write(state_dir.join(PID_FILE), format!("{}\n", std::process::id()))
}

/// Removes the pid file if it is ours.
pub fn remove_pid_file(state_dir: &Path) {
    let path = state_dir.join(PID_FILE);
    if read_pid(&path) == Some(std::process::id()) {
        let _ = std::fs::remove_file(path);
    }
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The pid and command socket of the running web that owns `state_dir`, if there is one.
pub fn running_instance(state_dir: &Path, runtime: &Path, proc_root: &Path) -> Option<(u32, PathBuf)> {
    let pid = read_pid(&state_dir.join(PID_FILE))?;
    if !is_web_shell(proc_root, pid) {
        return None;
    }
    let sock = run_dir(runtime, pid).join(FORGET_SOCK);
    sock.exists().then_some((pid, sock))
}

// ---- the running shell's end

/// One request line from a client.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    /// `forget-site <site>`
    ForgetSite(String),
    /// `open <url>` (C-SINGLE): load the URL in the page.
    Open(String),
}

/// A client's request line, or None for anything else.
pub fn parse_request(line: &str) -> Option<Request> {
    let line = line.trim();
    let arg = |prefix: &str| {
        Some(line.strip_prefix(prefix)?.trim())
            .filter(|a| !a.is_empty())
            .map(str::to_string)
    };
    if let Some(site) = arg("forget-site ") {
        return Some(Request::ForgetSite(site));
    }
    arg("open ").map(Request::Open)
}

// ---- the client's end

/// The exit code a `forgot` / `error` line stands for: 0 ok, 1 partial, 2 anything else.
pub fn exit_code_of(line: &str) -> i32 {
    match Event::parse(line.trim()) {
        Ok(Event::Forgot { result, .. }) if result == "ok" => 0,
        Ok(Event::Forgot { .. }) => 1,
        _ => 2,
    }
}

/// Sends one request line to the running web at `sock`; returns the line it answers with.
pub fn ask_running(sock: &Path, request: &str, timeout: Duration) -> std::io::Result<String> {
    let mut s = UnixStream::connect(sock)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(Duration::from_secs(5)))?;
    s.write_all(format!("{request}\n").as_bytes())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    if line.trim().is_empty() {
        return Err(std::io::Error::other("the running web closed the connection without an answer"));
    }
    Ok(line)
}

/// What `web [URL]` does when a web already runs on the state directory (C-SINGLE): with a URL,
/// asks it to load it; without, only says so (raising its window is not done: a Wayland client
/// cannot ask for focus without a token, and there is no portable X11 route from here). Returns
/// the exit code, or None when no instance runs (the caller then starts a new one, after the
/// profile-lock check).
pub fn run_second(url: Option<&str>, state_dir: &Path) -> Option<i32> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)?;
    let (pid, sock) = running_instance(state_dir, &runtime, Path::new("/proc"))?;
    let Some(url) = url else {
        println!(
            "web: already running (pid {pid}, state directory {}); nothing started. Give a URL to open it there.",
            state_dir.display()
        );
        return Some(0);
    };
    let url = match crate::keys::go_url(url, "") {
        Ok(u) => u,
        Err(m) => {
            eprintln!("{m}");
            return Some(2);
        }
    };
    match ask_running(&sock, &format!("open {url}"), Duration::from_secs(10)) {
        Ok(line) => match open_answer(&line) {
            Ok(opened) => {
                println!("web: opened {opened} in the running instance (pid {pid})");
                Some(0)
            }
            Err(m) => {
                eprintln!("web: the running instance (pid {pid}) refused: {m}");
                Some(2)
            }
        },
        // A shell that is going away, or one older than C-SINGLE that only knows forget-site.
        Err(e) => {
            eprintln!("web: the running instance (pid {pid}) did not answer ({e})");
            Some(2)
        }
    }
}

/// The shell's answer to `open`: the URL it loaded, or its error message.
pub fn open_answer(line: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).map_err(|_| format!("unexpected answer {line:?}"))?;
    match v["event"].as_str() {
        Some("opened") => Ok(v["url"].as_str().unwrap_or_default().to_string()),
        Some("error") => Err(v["message"].as_str().unwrap_or("error").to_string()),
        _ => Err(format!("unexpected answer {line:?}")),
    }
}

/// The host command that forgets without a window (PROTOCOL.md "--no-window").
pub fn standalone_command(host_bin: PathBuf, profile: &Path, site: &str, host_args: &[String]) -> std::process::Command {
    let mut c = std::process::Command::new(host_bin);
    c.arg(format!("--profile={}", profile.display()))
        .arg("--no-window")
        .arg(format!("--forget-site={site}"))
        .args(host_args)
        // Keeps Chromium from leaving the caller's systemd scope (as host::host_command).
        .env("FLATPAK_SANDBOX_DIR", profile)
        .stdin(std::process::Stdio::null());
    c
}

/// `web --forget-site`: returns the process exit code. Prints the `forgot` line on stdout.
pub fn run_cli(site: &str, state_dir: &Path, profile: &Path, host_bin: PathBuf, host_args: &[String]) -> i32 {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some((_, sock)) = runtime.and_then(|r| running_instance(state_dir, &r, Path::new("/proc"))) {
        match ask_running(&sock, &format!("forget-site {site}"), Duration::from_secs(120)) {
            Ok(line) => {
                println!("{}", line.trim_end());
                return exit_code_of(&line);
            }
            // The instance is going away or is no web: fall through to the standalone host.
            Err(e) => eprintln!("web: the running instance did not answer ({e}); forgetting without it"),
        }
    }
    match standalone_command(host_bin.clone(), profile, site, host_args).status() {
        // The host prints the `forgot` line itself (inherited stdout).
        Ok(s) => s.code().unwrap_or(2),
        Err(e) => {
            eprintln!("web: cannot start the engine host {}: {e}", host_bin.display());
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under the system temp dir (removed by the test, by explicit paths).
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let p = std::env::temp_dir().join(format!("web-forget-test-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn mkdir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
        fn fake_proc(&self, pid: u32, comm: &str) {
            let d = self.mkdir(&format!("proc/{pid}"));
            std::fs::write(d.join("comm"), format!("{comm}\n")).unwrap();
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn web_shell_is_told_by_comm() {
        let t = Tmp::new("comm");
        t.fake_proc(10, "web");
        t.fake_proc(11, "web-host-cef");
        t.fake_proc(12, "bash");
        let proc = t.0.join("proc");
        assert!(is_web_shell(&proc, 10));
        assert!(!is_web_shell(&proc, 11));
        assert!(!is_web_shell(&proc, 12));
        assert!(!is_web_shell(&proc, 13), "no such process");
    }

    #[test]
    fn stale_dirs_are_those_of_dead_pids_only() {
        let t = Tmp::new("stale");
        t.fake_proc(100, "web"); // alive
        t.fake_proc(300, "bash"); // pid reused by something else
        let run = t.mkdir("run");
        for n in [
            "web-100", "web-200", "web-300", "web-400", "web", "web-", "web-x1", "web-12x", "webkit-5", "other-7",
        ] {
            std::fs::create_dir(run.join(n)).unwrap();
        }
        std::fs::write(run.join("web-500"), "a file, not a directory").unwrap();
        let stale = stale_run_dirs(&run, 400, &t.0.join("proc"));
        let names: Vec<_> = stale.iter().map(|p| p.file_name().unwrap().to_str().unwrap().to_string()).collect();
        assert_eq!(names, ["web-200", "web-300"], "live web-100, own web-400, odd names and files stay");
    }

    #[test]
    fn removing_a_run_dir_takes_our_files_and_keeps_strangers() {
        let t = Tmp::new("rm");
        let run = t.mkdir("run");
        let a = t.mkdir("run/web-1");
        for f in ["control.sock", "forget.sock", "help.html"] {
            std::fs::write(a.join(f), "").unwrap();
        }
        remove_run_dir(&a);
        assert!(!a.exists());
        let b = t.mkdir("run/web-2");
        std::fs::write(b.join("control.sock"), "").unwrap();
        std::fs::write(b.join("precious"), "").unwrap();
        remove_run_dir(&b);
        assert!(b.join("precious").exists() && !b.join("control.sock").exists());
        assert!(run.exists());
    }

    #[test]
    fn running_instance_needs_a_live_web_and_its_socket() {
        let t = Tmp::new("run");
        let state = t.mkdir("state");
        let runtime = t.mkdir("run");
        let proc = t.0.join("proc");
        assert_eq!(running_instance(&state, &runtime, &proc), None, "no pid file");
        std::fs::write(state.join(PID_FILE), "55\n").unwrap();
        t.fake_proc(55, "web");
        assert_eq!(running_instance(&state, &runtime, &proc), None, "no socket yet");
        let d = t.mkdir("run/web-55");
        std::fs::write(d.join(FORGET_SOCK), "").unwrap();
        assert_eq!(running_instance(&state, &runtime, &proc), Some((55, d.join(FORGET_SOCK))));
        t.fake_proc(55, "firefox");
        assert_eq!(running_instance(&state, &runtime, &proc), None, "pid reused by another program");
        std::fs::write(state.join(PID_FILE), "garbage").unwrap();
        assert_eq!(running_instance(&state, &runtime, &proc), None);
    }

    #[test]
    fn requests_and_exit_codes() {
        let fs = |s: &str| Some(Request::ForgetSite(s.to_string()));
        assert_eq!(parse_request("forget-site example.com\n"), fs("example.com"));
        assert_eq!(parse_request("forget-site   https://a.b/x  "), fs("https://a.b/x"));
        assert_eq!(
            parse_request("open https://a.test/p?q=1\n"),
            Some(Request::Open("https://a.test/p?q=1".into()))
        );
        assert_eq!(parse_request("open"), None);
        assert_eq!(parse_request("open   \n"), None);
        assert_eq!(parse_request("opened x"), None);
        assert_eq!(open_answer(r#"{"event":"opened","url":"https://a/"}"#), Ok("https://a/".into()));
        assert_eq!(open_answer(r#"{"event":"error","message":"no host"}"#), Err("no host".into()));
        assert!(open_answer("junk").is_err());
        assert_eq!(parse_request("forget-site"), None);
        assert_eq!(parse_request("forget-site  \n"), None);
        assert_eq!(parse_request("quit"), None);
        let forgot = |r: &str| format!(r#"{{"event":"forgot","site":"a","domain":"a","result":"{r}","cookies":1,"origins":[]}}"#);
        assert_eq!(exit_code_of(&forgot("ok")), 0);
        assert_eq!(exit_code_of(&forgot("partial")), 1);
        assert_eq!(exit_code_of(r#"{"event":"error","message":"nope"}"#), 2);
        assert_eq!(exit_code_of("not json"), 2);
    }

    #[test]
    fn standalone_command_is_the_documented_host_invocation() {
        let c = standalone_command("h".into(), Path::new("/s/engines/cef"), "https://a.test/x", &["--foo".to_string()]);
        let args: Vec<_> = c.get_args().map(|a| a.to_str().unwrap().to_string()).collect();
        assert_eq!(
            args,
            ["--profile=/s/engines/cef", "--no-window", "--forget-site=https://a.test/x", "--foo"]
        );
    }

    #[test]
    fn a_client_talks_to_a_socket_and_gets_one_line_back() {
        let t = Tmp::new("sock");
        let sock = t.0.join("f.sock");
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let h = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&s).read_line(&mut line).unwrap();
            assert_eq!(parse_request(&line), Some(Request::ForgetSite("a.test".into())));
            (&s).write_all(b"{\"event\":\"forgot\",\"site\":\"a.test\",\"result\":\"ok\"}\n")
                .unwrap();
        });
        let ans = ask_running(&sock, "forget-site a.test", Duration::from_secs(5)).unwrap();
        assert_eq!(exit_code_of(&ans), 0);
        h.join().unwrap();
        assert!(ask_running(&t.0.join("none.sock"), "forget-site a.test", Duration::from_secs(1)).is_err());
    }
}
