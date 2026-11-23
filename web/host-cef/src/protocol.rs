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
}

const HOST_OPTS: [&str; 5] = ["control", "proxy", "profile", "url", "downloads"];

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
        let Some((name, value)) = a.strip_prefix("--").and_then(|s| s.split_once('=')) else {
            continue;
        };
        let slot = match name {
            "control" => &mut o.control,
            "proxy" => &mut o.proxy,
            "profile" => &mut o.profile,
            "url" => &mut o.url,
            "downloads" => &mut o.downloads,
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
}
