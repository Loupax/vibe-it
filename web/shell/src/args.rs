//! The `web` command line.
//!
//!   web [--engine cef] [--proxy HOST:PORT] [--state-dir DIR] [--host-bin PATH] [URL] [-- HOST-ARGS...]
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
    pub host_bin: Option<PathBuf>,
    pub url: Option<String>,
    pub host_args: Vec<String>,
}

pub const USAGE: &str = "usage: web [--engine cef] [--proxy HOST:PORT] [--state-dir DIR] [--host-bin PATH] [URL] [-- HOST-ARGS...]
  --engine cef       the engine host (cef is the only one for now)
  --proxy HOST:PORT  send every request through this HTTP forward proxy (default: direct)
  --state-dir DIR    state directory (default $WEB_STATE_DIR, else $XDG_STATE_HOME/web)
  --host-bin PATH    the engine host binary (default $WEB_HOST_CEF, else web-host-cef next to
                     this binary, else on PATH)
  -- HOST-ARGS       passed to the engine host unchanged (e.g. Chromium switches)";

/// Parses the arguments after the program name. `Ok(None)` means `--help`.
pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> Result<Option<Args>, String> {
    let mut a = Args {
        engine: Engine::Cef,
        proxy: None,
        state_dir: None,
        host_bin: None,
        url: None,
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
                "host-bin" => a.host_bin = Some(PathBuf::from(value()?)),
                _ => return Err(format!("unknown option --{name}")),
            }
        } else if a.url.is_none() {
            a.url = Some(arg);
        } else {
            return Err(format!("unexpected argument {arg:?} (one URL only)"));
        }
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
