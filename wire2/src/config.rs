//! Command-line flags (SPEC "CLI and configuration"). No config file. Unknown flags are errors; flags
//! of later steps are recognised and refused with a clear "not yet".

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use crate::log::Level;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listen {
    Tcp(SocketAddr),
    Unix(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: Listen,
    pub allow_remote: bool,
    pub dial_timeout: Duration,
    pub log_level: Level,
    /// `--no-decrypt` host globs, normalised (SPEC: never decrypted, whatever a filter claims).
    pub no_decrypt: Vec<String>,
    /// `--ca-dir`; `None` means the spec default `$XDG_DATA_HOME/proxy`, resolved when the CA is needed.
    pub ca_dir: Option<PathBuf>,
    /// TEST-ONLY `--test-resolve GLOB=IP` (W2-1a): names the upstream connector maps to an address
    /// instead of asking DNS, so tests can use `www.example.test`. Not in `--help`.
    pub test_resolve: Vec<(String, IpAddr)>,
    /// TEST-ONLY `--test-decrypt GLOB`: a stand-in for an http-layer filter claiming these hosts, so the
    /// decrypted path can be exercised before filters exist (W2-F brings the spec's pass-decrypt). Not
    /// in `--help`.
    pub test_decrypt: Vec<String>,
}

pub enum Parsed {
    Run(Config),
    Help,
    Version,
}

pub const USAGE: &str = "\
usage: wire2 [--listen 127.0.0.1:8080 | --listen unix:<path>] [--allow-remote]
             [--dial-timeout 10s] [--log-level error|warn|info|debug]
             [--no-decrypt GLOB]... [--ca-dir DIR]

Local forward proxy (wire2/SPEC.md), steps W2-1 and W2-2: CONNECT tunnels and plain http:// requests
are forwarded. TLS is decrypted only for hosts an http-layer filter claims; filters arrive with W2-F,
so this build decrypts nothing and creates no CA.

  --listen ADDR        host:port for the browser's HTTP proxy setting (default 127.0.0.1:8080; port 0
                       picks a free one), or unix:<path> for a forwarder inside a network namespace.
  --allow-remote       allow a non-loopback --listen address (otherwise refused: an open proxy).
  --dial-timeout DUR   upstream TCP connect timeout, e.g. 10s, 500ms, 2m (default 10s).
  --log-level LEVEL    error, warn, info (default) or debug.
  --no-decrypt GLOB    host never decrypted, even when an http filter claims it (repeatable). `*` matches
                       any run of characters; *.example.com does not match example.com.
  --ca-dir DIR         where the CA is created on first need (default $XDG_DATA_HOME/proxy). ca.pem is
                       what the browser imports; ca-key.pem (mode 0600) never leaves this directory.
  --help, --version

Not yet in this build: --filter, --hub, --connect-timeout, --stage-timeout (W2-F).
";

/// Parse `10s`, `500ms`, `2m`, `1h` or a bare number of seconds.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let split = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().map_err(|_| format!("bad duration {s:?}"))?;
    let secs = match unit {
        "" | "s" => n,
        "ms" => n / 1000.0,
        "m" => n * 60.0,
        "h" => n * 3600.0,
        _ => return Err(format!("bad duration unit in {s:?} (use ms, s, m or h)")),
    };
    if !secs.is_finite() || secs <= 0.0 || secs > 86_400.0 {
        return Err(format!("duration {s:?} out of range"));
    }
    Ok(Duration::from_secs_f64(secs))
}

pub fn parse_listen(s: &str) -> Result<Listen, String> {
    if let Some(path) = s.strip_prefix("unix:") {
        if path.is_empty() {
            return Err("--listen unix: needs a path".into());
        }
        return Ok(Listen::Unix(PathBuf::from(path)));
    }
    if let Ok(addr) = s.parse::<SocketAddr>() {
        return Ok(Listen::Tcp(addr));
    }
    if let Some(port) = s.strip_prefix("localhost:") {
        let port: u16 = port
            .parse()
            .map_err(|_| format!("bad port in --listen {s:?}"))?;
        return Ok(Listen::Tcp(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
        )));
    }
    Err(format!(
        "bad --listen {s:?}: expected IP:port, [IPv6]:port, localhost:port or unix:<path>"
    ))
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.to_canonical().is_loopback()
}

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut listen = Listen::Tcp(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080));
    let mut allow_remote = false;
    let mut dial_timeout = Duration::from_secs(10);
    let mut log_level = Level::Info;
    let mut no_decrypt = Vec::new();
    let mut ca_dir = None;
    let mut test_resolve = Vec::new();
    let mut test_decrypt = Vec::new();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let value = |it: &mut I::IntoIter| -> Result<String, String> {
            match inline.clone() {
                Some(v) => Ok(v),
                None => it.next().ok_or_else(|| format!("{flag} needs a value")),
            }
        };
        match flag.as_str() {
            "--help" | "-h" => return Ok(Parsed::Help),
            "--version" | "-V" => return Ok(Parsed::Version),
            "--listen" => listen = parse_listen(&value(&mut it)?)?,
            "--allow-remote" => {
                if inline.is_some() {
                    return Err("--allow-remote takes no value".into());
                }
                allow_remote = true
            }
            "--dial-timeout" => dial_timeout = parse_duration(&value(&mut it)?)?,
            "--log-level" => {
                let v = value(&mut it)?;
                log_level = Level::parse(&v).ok_or_else(|| format!("bad --log-level {v:?}"))?;
            }
            "--filter" | "--hub" | "--connect-timeout" | "--stage-timeout" => {
                return Err(format!(
                    "{flag} is not supported yet (filters arrive with step W2-F)"
                ));
            }
            "--no-decrypt" => no_decrypt.push(parse_glob(&value(&mut it)?)?),
            "--ca-dir" => {
                let v = value(&mut it)?;
                if v.is_empty() {
                    return Err("--ca-dir needs a directory".into());
                }
                ca_dir = Some(PathBuf::from(v));
            }
            // TEST-ONLY flags, deliberately absent from USAGE (NOTES.md "Test-only flags"); using one
            // logs a warning at startup.
            "--test-resolve" => {
                let v = value(&mut it)?;
                let (name, ip) = v
                    .split_once('=')
                    .ok_or_else(|| format!("--test-resolve {v:?}: expected GLOB=IP"))?;
                let ip: IpAddr = ip
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse()
                    .map_err(|_| format!("--test-resolve {v:?}: bad IP address"))?;
                test_resolve.push((parse_glob(name)?, ip));
            }
            "--test-decrypt" => test_decrypt.push(parse_glob(&value(&mut it)?)?),
            _ => return Err(format!("unknown argument {arg:?}")),
        }
    }
    if let Listen::Tcp(addr) = &listen
        && !is_loopback(addr.ip())
        && !allow_remote
    {
        return Err(format!(
            "--listen {addr} is not a loopback address; that would be an open proxy. Add --allow-remote if you mean it"
        ));
    }
    Ok(Parsed::Run(Config {
        listen,
        allow_remote,
        dial_timeout,
        log_level,
        no_decrypt,
        ca_dir,
        test_resolve,
        test_decrypt,
    }))
}

/// A host glob as written on the command line, normalised like hosts are (lowercase, no trailing dot,
/// punycode, canonical IP), with `*` kept as the wildcard.
pub fn parse_glob(s: &str) -> Result<String, String> {
    crate::host::normalise_glob(s).map_err(|e| format!("bad host glob {s:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Config, String> {
        match parse_args(args.iter().map(|s| s.to_string()))? {
            Parsed::Run(c) => Ok(c),
            _ => Err("not run".into()),
        }
    }

    #[test]
    fn defaults() {
        let c = run(&[]).unwrap();
        assert_eq!(c.listen, Listen::Tcp("127.0.0.1:8080".parse().unwrap()));
        assert_eq!(c.dial_timeout, Duration::from_secs(10));
        assert_eq!(c.log_level, Level::Info);
    }

    #[test]
    fn flags() {
        let c = run(&[
            "--listen",
            "[::1]:3128",
            "--dial-timeout=250ms",
            "--log-level",
            "debug",
        ])
        .unwrap();
        assert_eq!(c.listen, Listen::Tcp("[::1]:3128".parse().unwrap()));
        assert_eq!(c.dial_timeout, Duration::from_millis(250));
        assert_eq!(c.log_level, Level::Debug);
        assert_eq!(
            run(&["--listen=unix:/run/x.sock"]).unwrap().listen,
            Listen::Unix("/run/x.sock".into())
        );
        assert_eq!(
            run(&["--listen", "localhost:9"]).unwrap().listen,
            Listen::Tcp("127.0.0.1:9".parse().unwrap())
        );
    }

    #[test]
    fn refusals() {
        assert!(run(&["--bogus"]).unwrap_err().contains("unknown"));
        assert!(run(&["positional"]).unwrap_err().contains("unknown"));
        assert!(
            run(&["--filter", "unix:/x"])
                .unwrap_err()
                .contains("not supported yet")
        );
        assert!(run(&["--no-decrypt", "a..b"]).is_err());
        assert!(
            run(&["--no-decrypt"])
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(run(&["--test-resolve", "a.test"]).is_err());
        assert!(run(&["--test-resolve", "a.test=nope"]).is_err());
        assert!(run(&["--listen"]).unwrap_err().contains("needs a value"));
        assert!(
            run(&["--listen", "0.0.0.0:8080"])
                .unwrap_err()
                .contains("--allow-remote")
        );
        assert!(
            run(&["--listen", "[::]:8080"])
                .unwrap_err()
                .contains("--allow-remote")
        );
        assert!(run(&["--listen", "0.0.0.0:8080", "--allow-remote"]).is_ok());
        assert!(run(&["--listen", "example.com:80"]).is_err());
        assert!(run(&["--dial-timeout", "0"]).is_err());
        assert!(run(&["--dial-timeout", "5parsecs"]).is_err());
    }

    #[test]
    fn decrypt_flags() {
        let c = run(&[
            "--no-decrypt",
            "*.Bank.Example.",
            "--no-decrypt=Bücher.de",
            "--ca-dir",
            "/x/ca",
            "--test-resolve",
            "WWW.example.test=127.0.0.1",
            "--test-resolve",
            "*.v6.test=[::1]",
            "--test-decrypt",
            "*.example.test",
        ])
        .unwrap();
        assert_eq!(c.no_decrypt, vec!["*.bank.example", "xn--bcher-kva.de"]);
        assert_eq!(c.ca_dir, Some(PathBuf::from("/x/ca")));
        assert_eq!(
            c.test_resolve,
            vec![
                ("www.example.test".to_string(), "127.0.0.1".parse().unwrap()),
                ("*.v6.test".to_string(), "::1".parse().unwrap())
            ]
        );
        assert_eq!(c.test_decrypt, vec!["*.example.test"]);
        let d = run(&[]).unwrap();
        assert!(d.no_decrypt.is_empty() && d.ca_dir.is_none() && d.test_decrypt.is_empty());
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("10").unwrap(), Duration::from_secs(10));
        assert_eq!(parse_duration("1.5s").unwrap(), Duration::from_millis(1500));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
    }
}
