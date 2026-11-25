//! QSTASH3: `scratchy --forget-site <site or URL>` (STASH.md "Forget site"). Headless, no window.
//! 1. stash: exclusive lock, `rm -r storage/<site>` (that one directory), generation bump.
//! 2. wire: `X-Wire-Cookie-Op: clear-site` with the control token. A wire that cannot be reached
//!    leaves step 1 done and the log line says the cookies were not cleared.

use crate::js_net::{NetExecutor, NetJob, WireExecutor};
use crate::net::{WireLimits, build_wire_request};
use crate::net_policy::{control_token, cookie_control_headers};
use crate::stash::{Stash, site_dir_name};
use std::io;
use std::path::{Path, PathBuf};
use url::Url;

/// The page URL standing for a site argument: a URL as given, a bare host as `https://host/`.
fn site_page(arg: &str) -> Option<Url> {
    let arg = arg.trim();
    let url = if arg.contains("://") { Url::parse(arg).ok()? } else { Url::parse(&format!("https://{arg}/")).ok()? };
    crate::net_policy::site_of(&url)?;
    // The site becomes a directory name: no empty labels ("..", "a..b", leading or trailing dot).
    let host = url.host_str()?;
    if host.split('.').any(str::is_empty) {
        return None;
    }
    Some(url)
}

/// Sends `clear-site` for `page`'s site. `Err` = why the cookies were not cleared.
fn clear_cookies(exec: &dyn NetExecutor, socket: &Path, page: &Url) -> Result<(), String> {
    let mut token = control_token(socket, false)?;
    let mut refreshed = false;
    loop {
        let headers = cookie_control_headers("clear-site", page, Some(&token)).ok_or("no site")?;
        let request = build_wire_request("GET", page, &headers, b"").map_err(|e| format!("{e:?}"))?;
        let job = NetJob { id: 0, request, limits: WireLimits::default(), reserve: 0 };
        let resp = exec.execute(&job).map_err(|e| format!("wire is unreachable: {e:?}"))?;
        match resp.status {
            204 => return Ok(()),
            // 403 = wire restarted and wrote a new token: re-read it once, retry once.
            403 if !refreshed => {
                refreshed = true;
                let fresh = control_token(socket, true)?;
                if fresh == token {
                    return Err("wire refused the control token (403)".into());
                }
                token = fresh;
            }
            s => return Err(format!("wire answered {s} to clear-site")),
        }
    }
}

/// The whole operation; returns the log lines and the exit code (0 unless the stash part failed).
pub(crate) fn forget_site(state_dir: &Path, arg: &str, wire: Option<(&Path, &dyn NetExecutor)>) -> (Vec<String>, i32) {
    let Some(page) = site_page(arg) else {
        return (vec![format!("scratchy: --forget-site: {arg:?} is not a site or an http(s) URL")], 2);
    };
    let Some(site) = site_dir_name(&page) else { return (vec![format!("scratchy: --forget-site: {arg:?} has no site")], 2) };
    let mut lines = Vec::new();
    let stash = match Stash::open(state_dir) {
        Ok(s) => s,
        Err(e) => return (vec![format!("scratchy: forget-site: {}: {e}", state_dir.display())], 1),
    };
    match stash.forget(&site) {
        Ok(g) => lines.push(format!("scratchy: forget-site: stash of {site} removed (generation {g})")),
        Err(e) => return (vec![format!("scratchy: forget-site: stash of {site}: {e}")], 1),
    }
    match wire {
        None => lines.push("scratchy: forget-site: no --socket, cookies were not cleared".into()),
        Some((path, exec)) => match clear_cookies(exec, path, &page) {
            Ok(()) => lines.push(format!("scratchy: forget-site: cookies of {site} cleared")),
            Err(why) => lines.push(format!("scratchy: forget-site: {why}; cookies were not cleared")),
        },
    }
    (lines, 0)
}

/// `scratchy ... --forget-site <site> [--socket P] [--state-dir D]`.
pub(crate) fn main(args: &[String]) -> i32 {
    let (mut site, mut socket, mut state_dir) = (None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--forget-site" => site = it.next().cloned(),
            "--socket" => socket = it.next().map(PathBuf::from),
            "--state-dir" => state_dir = it.next().map(PathBuf::from),
            "--user-agent" | "--budget-ms" => {
                it.next();
            }
            _ => {}
        }
    }
    let Some(site) = site else {
        diag!("usage: scratchy --forget-site <site or URL> [--socket <path>] [--state-dir <dir>]");
        return 1;
    };
    let Some(dir) = crate::js_storage::resolve_state_dir(state_dir, |k| std::env::var(k).ok()) else {
        diag!("scratchy: forget-site: no state directory (--state-dir, SCRATCHY_STATE_DIR, XDG_STATE_HOME or HOME)");
        return 1;
    };
    let exec = socket.clone().map(WireExecutor);
    let wire = socket.as_deref().zip(exec.as_ref().map(|e| e as &dyn NetExecutor));
    let (lines, code) = forget_site(&dir, &site, wire);
    for l in lines {
        diag!("{l}");
    }
    code
}
