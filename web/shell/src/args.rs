//! The `web` command line.
//!
//!   web [--engine cef] [--proxy HOST:PORT] [--profile NAME] [--state-dir DIR] [--host-bin PATH] [URL] [-- HOST-ARGS...]
//!
//! `--proxy` is optional and handed to the engine host unchanged (DECISIONS.md "The proxy is
//! optional"). Everything after `--` goes to the engine host as is (e.g. Chromium switches).

use std::path::PathBuf;

/// The engine host to start. Only CEF exists until E1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Cef,
}

impl Engine {
    pub fn name(self) -> &'static str {
        match self {
            Engine::Cef => "cef",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub engine: Engine,
    pub proxy: Option<String>,
    pub state_dir: Option<PathBuf>,
    /// `--profile NAME` (C-PROFILE): `[A-Za-z0-9_-]+`; its own init.js and state directory.
    pub profile: Option<String>,
    pub host_bin: Option<PathBuf>,
    pub url: Option<String>,
    /// `--forget-site <site>` (P-FORGET-SHELL): forget the site and exit, no window.
    pub forget_site: Option<String>,
    pub host_args: Vec<String>,
}

pub const USAGE: &str =
    "usage: web [--engine cef] [--proxy HOST:PORT] [--profile NAME] [--state-dir DIR] [--host-bin PATH] [URL] [-- HOST-ARGS...]
       web [--profile NAME] [--state-dir DIR] [--host-bin PATH] --forget-site SITE
  --engine cef       the engine host (cef is the only one for now)
  --proxy HOST:PORT  send every request through this HTTP forward proxy (default: direct)
  --profile NAME     run $XDG_CONFIG_HOME/web/profiles/NAME/init.js after the global init.js, with its
                     own state directory (NAME: letters, digits, _ and -; see --state-dir)
  --state-dir DIR    state directory (default $WEB_STATE_DIR, else $XDG_STATE_HOME/web; with
                     --profile NAME: that plus /profiles/NAME; DIR itself is always used as given)
  --host-bin PATH    the engine host binary (default $WEB_HOST_CEF, else web-host-cef next to
                     this binary, else on PATH)
  --forget-site SITE forget SITE's cookies and storage (through the running web on the same state
                     directory, else with no window) and exit: 0 ok, 1 partial, 2 refused
  A web already running on the same state directory gets the URL (it opens it and shows a message)
  and this command exits 0; with no URL it only says that one runs.
  -- HOST-ARGS       passed to the engine host unchanged (e.g. Chromium switches)";

/// A profile name: `[A-Za-z0-9_-]+` (it becomes a directory name, so no dots or slashes).
pub fn valid_profile_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Parses the arguments after the program name. `Ok(None)` means `--help`.
pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> Result<Option<Args>, String> {
    let mut a = Args {
        engine: Engine::Cef,
        proxy: None,
        state_dir: None,
        profile: None,
        host_bin: None,
        url: None,
        forget_site: None,
        host_args: Vec::new(),
    };
    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        if arg == "--" {
            a.host_args.extend(it.by_ref());
            break;
        }
        if arg == "-h" || arg == "--help" {
            return Ok(None);
        }
        if let Some(opt) = arg.strip_prefix("--") {
            let (name, inline) = match opt.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (opt, None),
            };
            let mut value = || -> Result<String, String> {
                match inline.clone() {
                    Some(v) => Ok(v),
                    None => it.next().ok_or_else(|| format!("--{name} needs a value")),
                }
            };
            match name {
                "engine" => {
                    a.engine = match value()?.as_str() {
                        "cef" => Engine::Cef,
                        "webkit" => return Err("--engine webkit: not available yet (web/TODO.txt E1)".into()),
                        other => return Err(format!("--engine {other}: unknown engine (cef)")),
                    }
                }
                "proxy" => {
                    let v = value()?;
                    if v.is_empty() {
                        return Err("--proxy needs HOST:PORT".into());
                    }
                    a.proxy = Some(v)
                }
                "state-dir" => a.state_dir = Some(PathBuf::from(value()?)),
                "profile" => {
                    let v = value()?;
                    if !valid_profile_name(&v) {
                        return Err(format!("--profile {v:?}: a name is letters, digits, _ and - (at least one)"));
                    }
                    a.profile = Some(v)
                }
                "host-bin" => a.host_bin = Some(PathBuf::from(value()?)),
                "forget-site" => {
                    let v = value()?;
                    if v.trim().is_empty() {
                        return Err("--forget-site needs a site".into());
                    }
                    a.forget_site = Some(v)
                }
                _ => return Err(format!("unknown option --{name}")),
            }
        } else if a.url.is_none() {
            a.url = Some(arg);
        } else {
            return Err(format!("unexpected argument {arg:?} (one URL only)"));
        }
    }
    if a.forget_site.is_some() && a.url.is_some() {
        return Err("--forget-site opens no page: give a site or a URL, not both".into());
    }
    Ok(Some(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Option<Args>, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults() {
        let a = p(&[]).unwrap().unwrap();
        assert_eq!(a.engine, Engine::Cef);
        assert_eq!(a.proxy, None);
        assert_eq!(a.url, None);
        assert!(a.host_args.is_empty());
    }

    #[test]
    fn all_options_both_spellings() {
        let a = p(&[
            "--engine",
            "cef",
            "--proxy=127.0.0.1:3128",
            "--state-dir",
            "/s",
            "--host-bin=/h",
            "example.com",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(a.proxy.as_deref(), Some("127.0.0.1:3128"));
        assert_eq!(a.state_dir, Some(PathBuf::from("/s")));
        assert_eq!(a.host_bin, Some(PathBuf::from("/h")));
        assert_eq!(a.url.as_deref(), Some("example.com"));
    }

    #[test]
    fn host_args_after_double_dash() {
        let a = p(&["x.org", "--", "--proxy-bypass-list=<-loopback>", "--foo"]).unwrap().unwrap();
        assert_eq!(a.url.as_deref(), Some("x.org"));
        assert_eq!(a.host_args, vec!["--proxy-bypass-list=<-loopback>", "--foo"]);
        assert_eq!(a.proxy, None);
    }

    #[test]
    fn forget_site_both_spellings_and_conflicts() {
        let a = p(&["--forget-site", "example.com", "--state-dir=/s"]).unwrap().unwrap();
        assert_eq!(a.forget_site.as_deref(), Some("example.com"));
        assert_eq!(a.state_dir, Some(PathBuf::from("/s")));
        let a = p(&["--forget-site=https://a.b/x"]).unwrap().unwrap();
        assert_eq!(a.forget_site.as_deref(), Some("https://a.b/x"));
        assert_eq!(p(&[]).unwrap().unwrap().forget_site, None);
        assert!(p(&["--forget-site"]).is_err());
        assert!(p(&["--forget-site="]).is_err());
        assert!(p(&["--forget-site", "a.test", "b.test"]).is_err());
    }

    #[test]
    fn profile_names_and_spellings() {
        assert_eq!(p(&[]).unwrap().unwrap().profile, None);
        assert_eq!(p(&["--profile", "work"]).unwrap().unwrap().profile.as_deref(), Some("work"));
        assert_eq!(p(&["--profile=a_B-9"]).unwrap().unwrap().profile.as_deref(), Some("a_B-9"));
        let a = p(&["--profile=w", "--state-dir", "/s", "x.org"]).unwrap().unwrap();
        assert_eq!(
            (a.profile.as_deref(), a.state_dir, a.url.as_deref()),
            (Some("w"), Some(PathBuf::from("/s")), Some("x.org"))
        );
        let a = p(&["--forget-site", "a.test", "--profile=w"]).unwrap().unwrap();
        assert_eq!((a.profile.as_deref(), a.forget_site.as_deref()), (Some("w"), Some("a.test")));
        for bad in ["", "a b", "a/b", "..", "a.b", "../x", "é"] {
            assert!(p(&["--profile", bad]).is_err(), "{bad:?}");
            assert!(p(&[&format!("--profile={bad}")]).is_err(), "{bad:?}");
        }
        assert!(p(&["--profile"]).is_err());
    }

    #[test]
    fn errors() {
        assert!(p(&["--engine", "webkit"]).unwrap_err().contains("E1"));
        assert!(p(&["--engine", "gecko"]).is_err());
        assert!(p(&["--proxy"]).is_err());
        assert!(p(&["--proxy="]).is_err());
        assert!(p(&["--bogus"]).is_err());
        assert!(p(&["a", "b"]).is_err());
        assert_eq!(p(&["--help"]).unwrap(), None);
    }
}
