//! C-EXTERNAL: `:openExternal [url]` hands a page to another browser (KEYS.md "Opening a page in
//! another browser"). Pure argv building here (`check_url`, `resolve`, `argv`), plus `spawn`, which
//! starts the program detached, never through a shell, and never waits for it.

use std::path::Path;

/// The schemes `:openExternal` passes on; anything else (`javascript:`, `data:`, `about:`, `mailto:`)
/// is refused, because the other program would act on it with powers web never gave the page.
pub const ALLOWED_SCHEMES: &[&str] = &["http", "https", "file"];

/// The program and arguments to run, the URL included as ONE element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub argv: Vec<String>,
}

impl Launch {
    /// The program's name for the status bar.
    pub fn program(&self) -> &str {
        &self.argv[0]
    }
}

/// The URL to hand over: parsed, and only an `ALLOWED_SCHEMES` one. The result is the parser's
/// serialization, so it has no whitespace and starts with its scheme (never with `-`).
pub fn check_url(url: &str) -> Result<String, String> {
    if url.is_empty() {
        return Err("web: :openExternal: no page to open".to_string());
    }
    let parsed = url::Url::parse(url).map_err(|e| format!("web: :openExternal: {url}: {e}"))?;
    if !ALLOWED_SCHEMES.contains(&parsed.scheme()) {
        return Err(format!(
            "web: :openExternal: refused {}: URLs (only {} are handed over)",
            parsed.scheme(),
            ALLOWED_SCHEMES.join(", ")
        ));
    }
    Ok(parsed.to_string())
}

/// One command template (`firefox -new-tab %s` split on whitespace) with `url` put in: each `%s`
/// inside an element is replaced by the URL; without any `%s` the URL is appended as a last
/// element. Never a shell: the template is only split, never interpreted.
pub fn argv(template: &[String], url: &str) -> Vec<String> {
    let mut out: Vec<String> = template.iter().map(|t| t.replace("%s", url)).collect();
    if !template.iter().any(|t| t.contains("%s")) {
        out.push(url.to_string());
    }
    out
}

/// Whether `program` is a runnable file: a path with a slash, or a name found in `path_var`.
pub fn exists(program: &str, path_var: &str) -> bool {
    let runnable = |p: &Path| {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    };
    if program.contains('/') {
        return runnable(Path::new(program));
    }
    path_var
        .split(':')
        .filter(|d| !d.is_empty())
        .any(|d| runnable(&Path::new(d).join(program)))
}

/// The program to run for `url`: `configured` (config.rs `EXTERNAL_BROWSER`, a non-empty argv) if
/// set; else the first entry of `browser_var` (`$BROWSER`, entries separated by `:`) whose program
/// exists; else `xdg-open`. `path_var` is `$PATH`.
pub fn resolve(configured: &[&str], browser_var: Option<&str>, path_var: &str, url: &str) -> Result<Launch, String> {
    let template: Vec<String> = if !configured.is_empty() {
        configured.iter().map(|s| s.to_string()).collect()
    } else {
        let from_env = browser_var.unwrap_or("").split(':').find_map(|entry| {
            let words: Vec<String> = entry.split_whitespace().map(str::to_string).collect();
            (!words.is_empty() && exists(&words[0], path_var)).then_some(words)
        });
        from_env.unwrap_or_else(|| vec!["xdg-open".to_string()])
    };
    if !exists(&template[0], path_var) {
        return Err(format!(
            "web: :openExternal: {}: not found (set $BROWSER or config::EXTERNAL_BROWSER)",
            template[0]
        ));
    }
    Ok(Launch {
        argv: argv(&template, url),
    })
}

/// Starts `launch` detached: a new session (it outlives web and gets no terminal), stdio on
/// /dev/null, never a shell. A thread reaps the child so no zombie stays; web never waits for it.
pub fn spawn(launch: &Launch) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(&launch.argv[0]);
    cmd.args(&launch.argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and runs between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| format!("web: :openExternal: {}: {e}", launch.argv[0]))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Resolves from the real environment and the configured argv, then spawns. Returns the status bar text.
pub fn open(url: &str, configured: &[&str]) -> String {
    let browser = std::env::var("BROWSER").ok();
    let path = std::env::var("PATH").unwrap_or_default();
    match resolve(configured, browser.as_deref(), &path, url).and_then(|l| spawn(&l).map(|_| l)) {
        Ok(l) => format!("opened externally: {url} ({})", l.program()),
        Err(m) => m,
    }
}
