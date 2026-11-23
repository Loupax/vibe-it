//! Control protocol v0 (web/PROTOCOL.md): one JSON object per line, shell -> host
//! commands tagged by "cmd", host -> shell events tagged by "event". No CEF types
//! here, so all of it is unit tested.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 0;

/// The page's id in events: the CEF browser identifier. There is one page per host
/// today; the id stays in the events so a later multi-page design needs no change.
pub type TabId = i32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindDir {
    Next,
    Prev,
}

/// shell -> host. The host shows one page, so no command names a tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Command {
    Open {
        url: String,
    },
    Back,
    Forward,
    Reload {
        /// Bypass the HTTP cache (`:refresh hard`).
        #[serde(default, skip_serializing_if = "is_false")]
        hard: bool,
    },
    Stop,
    Find {
        query: String,
        dir: FindDir,
    },
    FindClear,
    /// Page zoom as a factor: 1.0 = 100 %.
    Zoom {
        factor: f64,
    },
    /// Forget a site (STASH.md "Forget site"): `site` is a site, host or URL; its
    /// registrable domain and every host under it are forgotten. Answered by `forgot`.
    ForgetSite {
        site: String,
    },
    Quit,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoadPhase {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoadKind {
    New,
    Reload,
    Traverse,
    SameDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ForgetResult {
    Ok,
    Partial,
}

/// host -> shell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// First line on every connection.
    Hello {
        protocol: u32,
        engine: String,
    },
    /// Sent once, before the page's window is mapped.
    TabCreated {
        tab: TabId,
    },
    /// Sent once, when the page's browser has closed (the host exits next).
    TabClosed {
        tab: TabId,
    },
    Url {
        tab: TabId,
        url: String,
    },
    Title {
        tab: TabId,
        title: String,
    },
    /// Main-frame loads. `status`: HTTP status on `end` (0 on `start`, for
    /// same-document loads and when no response arrived); `error`: the engine's
    /// error name when the load failed or was stopped.
    Load {
        tab: TabId,
        phase: LoadPhase,
        status: i32,
        kind: LoadKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// A main-frame load failed at the network level (or was stopped). `code`: the
    /// engine's error number (-105), `name`: its name (ERR_NAME_NOT_RESOLVED),
    /// `text`: the engine's description. Sent before the `load end` carrying `name`.
    LoadError {
        tab: TabId,
        code: i32,
        name: String,
        text: String,
    },
    EditableFocused {
        tab: TabId,
        focused: bool,
    },
    /// `index`: 1-based active match (0 when there is none), `count`: matches.
    FindResult {
        tab: TabId,
        index: i32,
        count: i32,
    },
    Crashed {
        tab: TabId,
        reason: String,
    },
    /// The answer to `forget-site`. `site`: the argument as the shell sent it;
    /// `domain`: the registrable domain that was forgotten (with every host under it);
    /// `result`: `ok`, or `partial` with `failed` naming what could not be cleared;
    /// `cookies`: cookies deleted; `origins`: origins whose storage was cleared.
    Forgot {
        site: String,
        domain: String,
        result: ForgetResult,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        failed: Vec<String>,
        cookies: u32,
        origins: Vec<String>,
    },
    /// A command line the host could not use (not JSON, unknown command, bad value).
    Error {
        message: String,
    },
}

pub fn parse_command(line: &str) -> Result<Command, String> {
    serde_json::from_str(line.trim()).map_err(|e| format!("bad command {:?}: {e}", line.trim()))
}

pub fn encode_event(ev: &Event) -> String {
    serde_json::to_string(ev).expect("events always serialize")
}

pub fn kind_name(k: LoadKind) -> &'static str {
    match k {
        LoadKind::New => "new",
        LoadKind::Reload => "reload",
        LoadKind::Traverse => "traverse",
        LoadKind::SameDocument => "same-document",
    }
}

/// The short text form used in logs and PROTOCOL.md (`load 3 end 200 new`).
pub fn event_text(ev: &Event) -> String {
    match ev {
        Event::Hello { protocol, engine } => format!("hello {protocol} {engine}"),
        Event::TabCreated { tab } => format!("tab-created {tab}"),
        Event::TabClosed { tab } => format!("tab-closed {tab}"),
        Event::Url { tab, url } => format!("url {tab} {url}"),
        Event::Title { tab, title } => format!("title {tab} {title}"),
        Event::Load {
            tab,
            phase,
            status,
            kind,
            error,
        } => {
            let p = if *phase == LoadPhase::Start { "start" } else { "end" };
            let e = error.as_ref().map(|e| format!(" ({e})")).unwrap_or_default();
            format!("load {tab} {p} {status} {}{e}", kind_name(*kind))
        }
        Event::LoadError { tab, code, name, text } => format!("load-error {tab} {code} {name} {text}"),
        Event::EditableFocused { tab, focused } => format!("editable-focused {tab} {focused}"),
        Event::FindResult { tab, index, count } => format!("find-result {tab} {index} {count}"),
        Event::Crashed { tab, reason } => format!("crashed {tab} ({reason})"),
        Event::Forgot { site, result, failed, .. } => match result {
            ForgetResult::Ok => format!("forgot {site} ok"),
            ForgetResult::Partial => format!("forgot {site} partial {}", failed.join(",")),
        },
        Event::Error { message } => format!("error {message}"),
    }
}

// cef_transition_type_t values (include/internal/cef_types.h).
const TT_SOURCE_MASK: u32 = 0xff;
const TT_RELOAD: u32 = 8;
const TT_FORWARD_BACK_FLAG: u32 = 0x0100_0000;

/// The load kind of a cross-document main-frame load from CEF's transition type.
pub fn kind_from_transition(t: u32) -> LoadKind {
    if t & TT_FORWARD_BACK_FLAG != 0 {
        LoadKind::Traverse
    } else if t & TT_SOURCE_MASK == TT_RELOAD {
        LoadKind::Reload
    } else {
        LoadKind::New
    }
}

pub const ZOOM_MIN: f64 = 0.25;
pub const ZOOM_MAX: f64 = 5.0;

/// CEF's zoom level is logarithmic: factor = 1.2 ^ level.
pub fn zoom_level(factor: f64) -> Result<f64, String> {
    if !factor.is_finite() || !(ZOOM_MIN..=ZOOM_MAX).contains(&factor) {
        return Err(format!("zoom factor {factor} is outside {ZOOM_MIN}..{ZOOM_MAX}"));
    }
    Ok(factor.ln() / 1.2f64.ln())
}

/// What the user typed -> a URL CEF can load: no scheme means https.
pub fn normalize_url(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "about:blank".to_string();
    }
    if s.contains("://") || s.starts_with("about:") || s.starts_with("data:") || s.starts_with("chrome:") {
        s.to_string()
    } else {
        format!("https://{s}")
    }
}

/// The host's own options. Everything else on the command line is passed to Chromium.
#[derive(Debug, Default, PartialEq)]
pub struct HostOptions {
    /// Unix socket the shell listens on; the host connects to it. None: standalone.
    pub control: Option<String>,
    /// HTTP forward proxy `host:port`; None: direct connections.
    pub proxy: Option<String>,
    pub profile: Option<String>,
    pub url: Option<String>,
    pub downloads: Option<String>,
    /// `--forget-site <site>`: forget it at start (with `--no-window`: and exit).
    pub forget_site: Option<String>,
    /// `--no-window`: show no page; only with `--forget-site`.
    pub no_window: bool,
}

const HOST_OPTS: [&str; 6] = ["control", "proxy", "profile", "url", "downloads", "forget-site"];

/// Rewrites `--opt value` to `--opt=value` for the host's own options, so Chromium's
/// command-line parser (which only knows `--opt=value`) does not see `value` as a
/// positional argument (a URL to open, for Chrome).
pub fn normalize_argv(argv: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::with_capacity(argv.len());
    let mut it = argv.iter();
    if let Some(prog) = it.next() {
        out.push(prog.clone());
    }
    while let Some(a) = it.next() {
        let name = a.strip_prefix("--").unwrap_or("");
        if HOST_OPTS.contains(&name) {
            match it.next() {
                Some(v) if !v.starts_with("--") => out.push(format!("--{name}={v}")),
                _ => return Err(format!("--{name} needs a value")),
            }
        } else if a.starts_with('-') {
            out.push(a.clone());
        } else {
            return Err(format!("unexpected argument {a:?} (use --url=<url>)"));
        }
    }
    Ok(out)
}

/// Picks the host's options out of a normalized argv.
pub fn host_options(argv: &[String]) -> Result<HostOptions, String> {
    let mut o = HostOptions::default();
    for a in argv.iter().skip(1) {
        if a == "--no-window" {
            o.no_window = true;
            continue;
        }
        let Some((name, value)) = a.strip_prefix("--").and_then(|s| s.split_once('=')) else {
            continue;
        };
        let slot = match name {
            "control" => &mut o.control,
            "proxy" => &mut o.proxy,
            "profile" => &mut o.profile,
            "url" => &mut o.url,
            "downloads" => &mut o.downloads,
            "forget-site" => &mut o.forget_site,
            _ => continue,
        };
        if value.is_empty() {
            return Err(format!("--{name} needs a value"));
        }
        *slot = Some(value.to_string());
    }
    if let Some(p) = &o.proxy {
        o.proxy = Some(check_proxy(p)?);
    }
    if let Some(site) = &o.forget_site {
        forget_domain(site).map_err(|e| format!("--forget-site: {e}"))?;
    }
    if o.no_window && o.forget_site.is_none() {
        return Err("--no-window needs --forget-site (there is nothing else to do without a window)".into());
    }
    Ok(o)
}

/// `host:port` or `http://host:port` -> `host:port` (Chromium's `--proxy-server` takes
/// that for an HTTP proxy). Other schemes are refused: web's proxy contract is an HTTP
/// forward proxy (plain requests and CONNECT).
pub fn check_proxy(p: &str) -> Result<String, String> {
    let hp = p.strip_prefix("http://").unwrap_or(p).trim_end_matches('/');
    let bad = || format!("--proxy {p:?}: expected host:port (an HTTP forward proxy)");
    if hp.contains("://") || hp.contains([';', ',', ' ', '=', '/']) {
        return Err(bad());
    }
    let (host, port) = hp.rsplit_once(':').ok_or_else(bad)?;
    if host.is_empty() || port.parse::<u16>().map(|p| p == 0).unwrap_or(true) {
        return Err(bad());
    }
    Ok(hp.to_string())
}

// ---- forget-site: which hosts and origins a site covers (STASH.md "Forget site").

/// Common multi-label public suffixes. COPY of scratchy's `MULTI_SUFFIXES`
/// (scratchy/src/net_policy.rs, itself a copy of wire's). Not the full Public Suffix
/// List: a missing suffix makes `forget-site` coarser (every sibling site under that
/// suffix is forgotten too), never finer.
#[rustfmt::skip]
const MULTI_SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "ltd.uk", "plc.uk", "net.uk", "sch.uk", "com.au", "net.au", "org.au",
    "edu.au", "gov.au", "asn.au", "id.au", "co.nz", "org.nz", "net.nz", "govt.nz", "ac.nz", "co.jp", "ne.jp", "or.jp",
    "ac.jp", "go.jp", "com.br", "net.br", "org.br", "gov.br", "com.cn", "net.cn", "org.cn", "gov.cn", "edu.cn", "co.in",
    "net.in", "org.in", "ac.in", "gov.in", "co.za", "org.za", "com.mx", "com.ar", "com.tr", "com.tw", "com.hk", "com.sg",
    "co.kr", "or.kr", "co.il", "com.ua", "com.pl", "co.id", "com.my", "com.ph", "com.vn", "com.co", "com.pe", "com.ng",
    "co.th", "com.eg", "com.sa", "github.io", "gitlab.io", "herokuapp.com", "blogspot.com", "appspot.com", "netlify.app",
    "vercel.app", "pages.dev", "workers.dev", "cloudfront.net", "s3.amazonaws.com", "azurewebsites.net", "web.app",
    "firebaseapp.com", "myshopify.com", "wordpress.com", "fly.dev", "onrender.com",
];

/// eTLD+1 of a lowercase host, as scratchy's `registrable_domain`. An IP address or a
/// single-label host is its own site.
pub fn registrable_domain(host: &str) -> String {
    if host.parse::<std::net::IpAddr>().is_ok() || !host.contains('.') {
        return host.to_string();
    }
    let labels: Vec<&str> = host.split('.').collect();
    for i in 0..labels.len() {
        let suffix = labels[i..].join(".");
        if !suffix.contains('.') || MULTI_SUFFIXES.contains(&suffix.as_str()) {
            return if i == 0 { host.to_string() } else { labels[i - 1..].join(".") };
        }
    }
    host.to_string()
}

/// The authority part of `scheme://authority/...` (or of a bare `host[:port]/...`),
/// lowercased, without userinfo. Err for a scheme other than http(s).
fn authority_of(input: &str) -> Result<String, String> {
    let s = input.trim();
    let rest = match s.split_once("://") {
        Some((scheme, rest)) => {
            let scheme = scheme.to_ascii_lowercase();
            if scheme != "http" && scheme != "https" {
                return Err(format!("{s:?}: only http and https sites can be forgotten"));
            }
            rest
        }
        None => s,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    Ok(authority.to_ascii_lowercase())
}

/// The host of an authority (`host`, `host:port`, `[v6]:port`), checked.
fn host_of_authority(authority: &str) -> Option<String> {
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("").to_string()
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    let host = host.trim_end_matches('.').to_string();
    let ok_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | ':');
    if host.is_empty() || !host.chars().all(ok_char) || host.starts_with('.') || host.contains("..") {
        return None;
    }
    Some(host)
}

/// The registrable domain `forget-site <arg>` forgets: `https://mail.example.co.uk/x`
/// -> `example.co.uk`; `example.com` -> `example.com`. Both schemes go (cookies do not
/// keep the scheme apart either).
pub fn forget_domain(arg: &str) -> Result<String, String> {
    let host = host_of_authority(&authority_of(arg)?).ok_or_else(|| format!("{:?} is not a site, host or http(s) URL", arg.trim()))?;
    if MULTI_SUFFIXES.contains(&host.as_str()) || (!host.contains('.') && host.parse::<std::net::IpAddr>().is_err() && host != "localhost")
    {
        return Err(format!("{:?} is a top-level domain or public suffix, not a site", arg.trim()));
    }
    Ok(registrable_domain(&host))
}

/// `host` is `domain` or a host under it. A cookie domain may start with a dot.
pub fn host_in_site(host: &str, domain: &str) -> bool {
    let h = host.trim_start_matches('.').trim_end_matches('.').to_ascii_lowercase();
    h == domain || h.strip_suffix(domain).map(|p| p.ends_with('.')).unwrap_or(false)
}

/// The origin (`scheme://host[:port]`, default port left out) and host of an http(s)
/// URL; None for other schemes.
pub fn origin_of(url: &str) -> Option<(String, String)> {
    let (scheme, _) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let authority = authority_of(url).ok()?;
    let host = host_of_authority(&authority)?;
    let default_port = if scheme == "http" { ":80" } else { ":443" };
    let authority = authority.strip_suffix(default_port).unwrap_or(&authority);
    Some((format!("{scheme}://{authority}"), host))
}

/// The origins whose storage `forget-site` clears for `domain`: the origins the host
/// recorded (frames it loaded) that are under the site, plus http and https of the
/// domain itself, its `www.` and every host a cookie of the site names. Sorted.
pub fn forget_origins<'a>(
    domain: &str,
    recorded: impl IntoIterator<Item = &'a String>,
    cookie_hosts: impl IntoIterator<Item = &'a String>,
) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for o in recorded {
        if let Some((origin, host)) = origin_of(o) {
            if host_in_site(&host, domain) {
                out.insert(origin);
            }
        }
    }
    let mut hosts = vec![domain.to_string()];
    if domain.parse::<std::net::IpAddr>().is_err() {
        hosts.push(format!("www.{domain}"));
    }
    hosts.extend(
        cookie_hosts
            .into_iter()
            .map(|h| h.trim_start_matches('.').to_ascii_lowercase())
            .filter(|h| host_in_site(h, domain)),
    );
    for h in hosts {
        let h = if h.contains(':') { format!("[{h}]") } else { h };
        out.insert(format!("http://{h}"));
        out.insert(format!("https://{h}"));
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_every_command() {
        let cases: Vec<(&str, Command)> = vec![
            (
                r#"{"cmd":"open","url":"https://a.test/"}"#,
                Command::Open {
                    url: "https://a.test/".into(),
                },
            ),
            (r#"{"cmd":"back"}"#, Command::Back),
            (r#"{"cmd":"forward"}"#, Command::Forward),
            (r#"{"cmd":"reload"}"#, Command::Reload { hard: false }),
            (r#"{"cmd":"reload","hard":true}"#, Command::Reload { hard: true }),
            (r#"{"cmd":"stop"}"#, Command::Stop),
            (
                r#"{"cmd":"find","query":"needle","dir":"next"}"#,
                Command::Find {
                    query: "needle".into(),
                    dir: FindDir::Next,
                },
            ),
            (
                r#"{"cmd":"find","query":"n","dir":"prev"}"#,
                Command::Find {
                    query: "n".into(),
                    dir: FindDir::Prev,
                },
            ),
            (r#"{"cmd":"find-clear"}"#, Command::FindClear),
            (r#"{"cmd":"zoom","factor":1.5}"#, Command::Zoom { factor: 1.5 }),
            (r#"{"cmd":"quit"}"#, Command::Quit),
        ];
        for (line, want) in cases {
            assert_eq!(parse_command(line).unwrap(), want, "{line}");
            // Encoding and parsing again gives the same command.
            let again = serde_json::to_string(&want).unwrap();
            assert_eq!(parse_command(&again).unwrap(), want, "{again}");
        }
    }

    #[test]
    fn tolerates_whitespace_and_unknown_fields() {
        assert_eq!(parse_command("  {\"cmd\":\"quit\",\"x\":1}\n").unwrap(), Command::Quit);
    }

    #[test]
    fn rejects_bad_commands() {
        for bad in [
            "",
            "open https://a.test/",
            r#"{"cmd":"fly"}"#,
            r#"{"cmd":"open"}"#,
            r#"{"cmd":"new-tab"}"#,
            r#"{"cmd":"close-tab","tab":1}"#,
            r#"{"cmd":"find","query":"x","dir":"up"}"#,
            r#"{"cmd":"zoom","factor":"big"}"#,
            r#"{"url":"x"}"#,
        ] {
            assert!(parse_command(bad).is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn encodes_every_event() {
        let cases: Vec<(Event, &str)> = vec![
            (
                Event::Hello {
                    protocol: 0,
                    engine: "cef".into(),
                },
                r#"{"event":"hello","protocol":0,"engine":"cef"}"#,
            ),
            (Event::TabCreated { tab: 1 }, r#"{"event":"tab-created","tab":1}"#),
            (Event::TabClosed { tab: 3 }, r#"{"event":"tab-closed","tab":3}"#),
            (
                Event::Url {
                    tab: 1,
                    url: "https://a.test/\"q\"".into(),
                },
                r#"{"event":"url","tab":1,"url":"https://a.test/\"q\""}"#,
            ),
            (
                Event::Title {
                    tab: 1,
                    title: "A\nB ü".into(),
                },
                r#"{"event":"title","tab":1,"title":"A\nB ü"}"#,
            ),
            (
                Event::Load {
                    tab: 1,
                    phase: LoadPhase::Start,
                    status: 0,
                    kind: LoadKind::New,
                    error: None,
                },
                r#"{"event":"load","tab":1,"phase":"start","status":0,"kind":"new"}"#,
            ),
            (
                Event::Load {
                    tab: 2,
                    phase: LoadPhase::End,
                    status: 404,
                    kind: LoadKind::SameDocument,
                    error: Some("ERR_ABORTED".into()),
                },
                r#"{"event":"load","tab":2,"phase":"end","status":404,"kind":"same-document","error":"ERR_ABORTED"}"#,
            ),
            (
                Event::EditableFocused { tab: 1, focused: true },
                r#"{"event":"editable-focused","tab":1,"focused":true}"#,
            ),
            (
                Event::FindResult {
                    tab: 1,
                    index: 2,
                    count: 3,
                },
                r#"{"event":"find-result","tab":1,"index":2,"count":3}"#,
            ),
            (
                Event::Crashed {
                    tab: 1,
                    reason: "crashed".into(),
                },
                r#"{"event":"crashed","tab":1,"reason":"crashed"}"#,
            ),
            (
                Event::LoadError {
                    tab: 1,
                    code: -105,
                    name: "ERR_NAME_NOT_RESOLVED".into(),
                    text: "net::ERR_NAME_NOT_RESOLVED".into(),
                },
                r#"{"event":"load-error","tab":1,"code":-105,"name":"ERR_NAME_NOT_RESOLVED","text":"net::ERR_NAME_NOT_RESOLVED"}"#,
            ),
            (Event::Error { message: "bad".into() }, r#"{"event":"error","message":"bad"}"#),
        ];
        for (ev, want) in cases {
            let line = encode_event(&ev);
            assert_eq!(line, want);
            assert!(!line.contains('\n'), "one line per event");
            let back: Event = serde_json::from_str(&line).unwrap();
            assert_eq!(back, ev);
        }
    }

    #[test]
    fn event_text_form() {
        let ev = Event::Load {
            tab: 3,
            phase: LoadPhase::End,
            status: 200,
            kind: LoadKind::Traverse,
            error: None,
        };
        assert_eq!(event_text(&ev), "load 3 end 200 traverse");
        let ev = Event::Load {
            tab: 3,
            phase: LoadPhase::End,
            status: 0,
            kind: LoadKind::New,
            error: Some("ERR_ABORTED".into()),
        };
        assert_eq!(event_text(&ev), "load 3 end 0 new (ERR_ABORTED)");
        assert_eq!(event_text(&Event::TabCreated { tab: 4 }), "tab-created 4");
        assert_eq!(
            event_text(&Event::FindResult {
                tab: 1,
                index: 1,
                count: 3
            }),
            "find-result 1 1 3"
        );
    }

    #[test]
    fn transition_kinds() {
        assert_eq!(kind_from_transition(0), LoadKind::New); // TT_LINK
        assert_eq!(kind_from_transition(1 | 0x0200_0000 | 0x0800_0000), LoadKind::New); // explicit, direct load, from API
        assert_eq!(kind_from_transition(7), LoadKind::New); // form submit
        assert_eq!(kind_from_transition(8), LoadKind::Reload);
        assert_eq!(kind_from_transition(8 | 0x0800_0000), LoadKind::Reload);
        assert_eq!(kind_from_transition(0x0100_0000), LoadKind::Traverse);
        assert_eq!(kind_from_transition(1 | 0x0100_0000 | 0x0800_0000), LoadKind::Traverse);
    }

    #[test]
    fn zoom_levels() {
        assert!((zoom_level(1.0).unwrap()).abs() < 1e-12);
        assert!((zoom_level(1.2).unwrap() - 1.0).abs() < 1e-12);
        assert!((zoom_level(1.44).unwrap() - 2.0).abs() < 1e-9);
        assert!(zoom_level(0.5).unwrap() < 0.0);
        for bad in [0.0, -1.0, 0.1, 6.0, f64::NAN, f64::INFINITY] {
            assert!(zoom_level(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_url(""), "about:blank");
        assert_eq!(normalize_url(" example.com "), "https://example.com");
        assert_eq!(normalize_url("http://a.test/x"), "http://a.test/x");
        assert_eq!(normalize_url("about:blank"), "about:blank");
        assert_eq!(normalize_url("chrome://version"), "chrome://version");
    }

    #[test]
    fn argv_rewrites_host_options_only() {
        let argv = s(&[
            "web-host-cef",
            "--proxy",
            "127.0.0.1:8080",
            "--control",
            "/s/c.sock",
            "--ozone-platform=wayland",
            "--url=https://a.test/",
        ]);
        let n = normalize_argv(&argv).unwrap();
        assert_eq!(
            n,
            s(&[
                "web-host-cef",
                "--proxy=127.0.0.1:8080",
                "--control=/s/c.sock",
                "--ozone-platform=wayland",
                "--url=https://a.test/"
            ])
        );
        let o = host_options(&n).unwrap();
        assert_eq!(o.proxy.as_deref(), Some("127.0.0.1:8080"));
        assert_eq!(o.control.as_deref(), Some("/s/c.sock"));
        assert_eq!(o.url.as_deref(), Some("https://a.test/"));
        assert_eq!(o.profile, None);
    }

    #[test]
    fn no_proxy_is_fine() {
        let o = host_options(&s(&["web-host-cef", "--profile=/p"])).unwrap();
        assert_eq!(o.proxy, None);
        assert_eq!(o.control, None);
        assert_eq!(o.profile.as_deref(), Some("/p"));
    }

    #[test]
    fn argv_errors() {
        assert!(normalize_argv(&s(&["h", "--proxy"])).is_err());
        assert!(normalize_argv(&s(&["h", "--proxy", "--url=x"])).is_err());
        assert!(normalize_argv(&s(&["h", "https://a.test/"])).is_err());
        assert!(host_options(&s(&["h", "--proxy="])).is_err());
    }

    #[test]
    fn proxies() {
        assert_eq!(check_proxy("127.0.0.1:3128").unwrap(), "127.0.0.1:3128");
        assert_eq!(check_proxy("http://proxy.test:8080/").unwrap(), "proxy.test:8080");
        assert_eq!(check_proxy("[::1]:3128").unwrap(), "[::1]:3128");
        for bad in ["proxy.test", ":80", "a:0", "a:99999", "socks5://a:1080", "a:1;b:2", "a:1,b:2"] {
            assert!(check_proxy(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn forget_site_command_and_event() {
        assert_eq!(
            parse_command(r#"{"cmd":"forget-site","site":"https://mail.example.com/x"}"#).unwrap(),
            Command::ForgetSite {
                site: "https://mail.example.com/x".into()
            }
        );
        assert!(parse_command(r#"{"cmd":"forget-site"}"#).is_err());
        let ok = Event::Forgot {
            site: "example.com".into(),
            domain: "example.com".into(),
            result: ForgetResult::Ok,
            failed: vec![],
            cookies: 2,
            origins: vec!["http://example.com".into(), "https://example.com".into()],
        };
        let line = encode_event(&ok);
        assert_eq!(
            line,
            r#"{"event":"forgot","site":"example.com","domain":"example.com","result":"ok","cookies":2,"origins":["http://example.com","https://example.com"]}"#
        );
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), ok);
        assert_eq!(event_text(&ok), "forgot example.com ok");
        let partial = Event::Forgot {
            site: "https://a.test/".into(),
            domain: "a.test".into(),
            result: ForgetResult::Partial,
            failed: vec!["storage https://a.test (timeout)".into(), "http-cache".into()],
            cookies: 0,
            origins: vec![],
        };
        let line = encode_event(&partial);
        assert!(
            line.contains(r#""result":"partial","failed":["storage https://a.test (timeout)","http-cache"]"#),
            "{line}"
        );
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), partial);
        assert_eq!(
            event_text(&partial),
            "forgot https://a.test/ partial storage https://a.test (timeout),http-cache"
        );
    }

    #[test]
    fn forget_domains() {
        for (arg, want) in [
            ("example.com", "example.com"),
            ("https://mail.example.co.uk/x?y#z", "example.co.uk"),
            ("HTTP://User@Sub.Example.COM:8080/", "example.com"),
            ("sub.site-a.test", "site-a.test"),
            ("site-a.test.", "site-a.test"),
            ("me.github.io", "me.github.io"),
            ("x.me.github.io/a", "me.github.io"),
            ("127.0.0.1:8000", "127.0.0.1"),
            ("http://[::1]:80/", "::1"),
            ("localhost", "localhost"),
        ] {
            assert_eq!(forget_domain(arg).as_deref(), Ok(want), "{arg}");
        }
        for bad in [
            "",
            "com",
            "co.uk",
            "github.io",
            "ftp://example.com",
            "file:///etc",
            "exa mple.com",
            ".com",
            "a..b",
            "https:///x",
        ] {
            assert!(forget_domain(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn hosts_in_site() {
        assert!(host_in_site("example.com", "example.com"));
        assert!(host_in_site(".example.com", "example.com"));
        assert!(host_in_site("a.b.EXAMPLE.com", "example.com"));
        assert!(!host_in_site("badexample.com", "example.com"));
        assert!(!host_in_site("example.com.evil.test", "example.com"));
        assert!(!host_in_site("com", "example.com"));
    }

    #[test]
    fn origins() {
        assert_eq!(origin_of("https://a.test/x"), Some(("https://a.test".into(), "a.test".into())));
        assert_eq!(origin_of("https://a.test:443/x"), Some(("https://a.test".into(), "a.test".into())));
        assert_eq!(
            origin_of("http://A.test:8080/#f"),
            Some(("http://a.test:8080".into(), "a.test".into()))
        );
        assert_eq!(origin_of("http://[::1]:80/"), Some(("http://[::1]".into(), "::1".into())));
        assert_eq!(origin_of("about:blank"), None);
        assert_eq!(origin_of("data:text/html,x"), None);
        assert_eq!(origin_of("chrome://version"), None);

        let rec: Vec<String> = [
            "https://sub.site-a.test",
            "http://site-a.test:8080",
            "https://site-b.test",
            "https://xsite-a.test",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let cookies: Vec<String> = [".login.site-a.test", "site-b.test"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            forget_origins("site-a.test", &rec, &cookies),
            vec![
                "http://login.site-a.test",
                "http://site-a.test",
                "http://site-a.test:8080",
                "http://www.site-a.test",
                "https://login.site-a.test",
                "https://site-a.test",
                "https://sub.site-a.test",
                "https://www.site-a.test",
            ]
        );
        assert_eq!(forget_origins("127.0.0.1", &[], &[]), vec!["http://127.0.0.1", "https://127.0.0.1"]);
    }

    #[test]
    fn forget_options() {
        let n = normalize_argv(&s(&["h", "--profile", "/p", "--forget-site", "a.test", "--no-window"])).unwrap();
        assert_eq!(n, s(&["h", "--profile=/p", "--forget-site=a.test", "--no-window"]));
        let o = host_options(&n).unwrap();
        assert_eq!(o.forget_site.as_deref(), Some("a.test"));
        assert!(o.no_window);
        assert!(host_options(&s(&["h", "--no-window"])).is_err());
        assert!(host_options(&s(&["h", "--forget-site=com"])).is_err());
        assert!(normalize_argv(&s(&["h", "--forget-site"])).is_err());
        assert!(!host_options(&s(&["h", "--forget-site=a.test"])).unwrap().no_window);
    }
}
