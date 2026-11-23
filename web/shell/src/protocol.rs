//! The shell's side of the engine-host control protocol v0 (web/PROTOCOL.md): JSON lines,
//! events tagged by "event", commands by "cmd". Unknown fields and unknown events are
//! ignored (both sides ignore what they do not know).

use serde::{Deserialize, Serialize};

/// An event from the engine host. Fields the shell does not use yet are still parsed, so a
/// malformed event is caught here and C3 can use them without touching the parser.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
#[allow(dead_code)] // C3 (status bar, history, INPUT mode) reads the fields
pub enum Event {
    Hello {
        protocol: u32,
        #[serde(default)]
        engine: String,
    },
    TabCreated {
        tab: u64,
    },
    TabClosed {
        tab: u64,
    },
    Url {
        tab: u64,
        url: String,
    },
    Title {
        tab: u64,
        title: String,
    },
    Load {
        tab: u64,
        phase: String,
        #[serde(default)]
        status: u32,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        error: Option<String>,
    },
    LoadError {
        tab: u64,
        code: i64,
        #[serde(default)]
        name: String,
        #[serde(default)]
        text: String,
    },
    EditableFocused {
        tab: u64,
        focused: bool,
    },
    FindResult {
        tab: u64,
        /// The `find` request this answers (0: a host that does not echo ids).
        #[serde(default)]
        id: u64,
        index: u32,
        count: u32,
        /// C5b: the find asked for `screen_top` and no match is at or below it: the match is
        /// the first one, after a wrap.
        #[serde(default)]
        wrapped: bool,
    },
    Crashed {
        tab: u64,
        #[serde(default)]
        reason: String,
    },
    Error {
        message: String,
    },
    /// The answer to `forget-site` (P-FORGET-SHELL).
    Forgot {
        site: String,
        #[serde(default)]
        domain: String,
        result: String,
        #[serde(default)]
        failed: Vec<String>,
        #[serde(default)]
        cookies: u32,
    },
    /// An event this shell does not know (a newer host).
    #[serde(other)]
    Unknown,
}

impl Event {
    pub fn parse(line: &str) -> Result<Event, String> {
        serde_json::from_str(line).map_err(|e| format!("bad event {line:?}: {e}"))
    }
}

/// A command to the engine host (the ones the shell sends so far; `zoom` and
/// `forget-site` come with their items).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Command {
    Open {
        url: String,
    },
    Back,
    Forward,
    Reload {
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        hard: bool,
    },
    /// Find in the page (`dir` as in PROTOCOL.md; the same query again steps).
    Find {
        /// Request id (C5-STALE): the host echoes it in the `find-result` that answers.
        id: u64,
        query: String,
        dir: FindDir,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        match_case: bool,
        /// C5b (T23): the engine starts at the first text at or below the top of the screen
        /// (`/` Enter); without it, where the engine picks.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        screen_top: bool,
    },
    FindClear,
    /// Caret browsing on/off (`:set caret` / `:set nocaret`, C-CARETCMD).
    Caret {
        on: bool,
    },
    /// Forget a site's cookies and storage (P-FORGET-SHELL; `:forget-site`, `web --forget-site`).
    ForgetSite {
        site: String,
    },
    Quit,
}

/// Which way a `find` steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindDir {
    Next,
    Prev,
}

impl Command {
    /// One JSON line, `\n`-terminated.
    pub fn line(&self) -> String {
        let mut s = serde_json::to_string(self).expect("a command always serializes");
        s.push('\n');
        s
    }
}

/// Splits a byte stream into lines. Bytes after the last `\n` wait for more input.
#[derive(Debug, Default)]
pub struct LineBuffer {
    buf: Vec<u8>,
}

impl LineBuffer {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1]).trim().to_string();
            if !text.is_empty() {
                out.push(text);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_v0_event_from_protocol_md() {
        let cases = [
            (
                r#"{"event":"hello","protocol":0,"engine":"cef"}"#,
                Event::Hello {
                    protocol: 0,
                    engine: "cef".into(),
                },
            ),
            (r#"{"event":"tab-created","tab":1}"#, Event::TabCreated { tab: 1 }),
            (r#"{"event":"tab-closed","tab":1}"#, Event::TabClosed { tab: 1 }),
            (
                r#"{"event":"url","tab":1,"url":"https://example.com/"}"#,
                Event::Url {
                    tab: 1,
                    url: "https://example.com/".into(),
                },
            ),
            (
                r#"{"event":"title","tab":1,"title":"Example Domain"}"#,
                Event::Title {
                    tab: 1,
                    title: "Example Domain".into(),
                },
            ),
            (
                r#"{"event":"load","tab":1,"phase":"end","status":200,"kind":"new"}"#,
                Event::Load {
                    tab: 1,
                    phase: "end".into(),
                    status: 200,
                    kind: "new".into(),
                    error: None,
                },
            ),
            (
                r#"{"event":"load","tab":1,"phase":"end","status":0,"kind":"new","error":"ERR_ABORTED"}"#,
                Event::Load {
                    tab: 1,
                    phase: "end".into(),
                    status: 0,
                    kind: "new".into(),
                    error: Some("ERR_ABORTED".into()),
                },
            ),
            (
                r#"{"event":"load-error","tab":1,"code":-105,"name":"ERR_NAME_NOT_RESOLVED","text":"ERR_NAME_NOT_RESOLVED"}"#,
                Event::LoadError {
                    tab: 1,
                    code: -105,
                    name: "ERR_NAME_NOT_RESOLVED".into(),
                    text: "ERR_NAME_NOT_RESOLVED".into(),
                },
            ),
            (
                r#"{"event":"editable-focused","tab":1,"focused":true}"#,
                Event::EditableFocused { tab: 1, focused: true },
            ),
            (
                r#"{"event":"find-result","tab":1,"id":7,"index":2,"count":5}"#,
                Event::FindResult {
                    tab: 1,
                    id: 7,
                    index: 2,
                    count: 5,
                    wrapped: false,
                },
            ),
            (
                r#"{"event":"find-result","tab":1,"id":8,"index":1,"count":5,"wrapped":true}"#,
                Event::FindResult {
                    tab: 1,
                    id: 8,
                    index: 1,
                    count: 5,
                    wrapped: true,
                },
            ),
            (
                r#"{"event":"crashed","tab":1,"reason":"TS_PROCESS_CRASHED (exit code 139)"}"#,
                Event::Crashed {
                    tab: 1,
                    reason: "TS_PROCESS_CRASHED (exit code 139)".into(),
                },
            ),
            (
                r#"{"event":"error","message":"unknown cmd"}"#,
                Event::Error {
                    message: "unknown cmd".into(),
                },
            ),
        ];
        for (line, want) in cases {
            assert_eq!(Event::parse(line).unwrap(), want, "{line}");
        }
    }

    #[test]
    fn unknown_fields_and_events_are_ignored() {
        assert_eq!(
            Event::parse(r#"{"event":"tab-created","tab":3,"later":"field"}"#).unwrap(),
            Event::TabCreated { tab: 3 }
        );
        assert_eq!(Event::parse(r#"{"event":"future-thing","x":1}"#).unwrap(), Event::Unknown);
        assert_eq!(
            Event::parse(
                r#"{"event":"forgot","site":"a.test","domain":"a.test","result":"partial","failed":["cookies"],"cookies":2,"origins":[]}"#
            )
            .unwrap(),
            Event::Forgot {
                site: "a.test".into(),
                domain: "a.test".into(),
                result: "partial".into(),
                failed: vec!["cookies".into()],
                cookies: 2
            }
        );
    }

    #[test]
    fn malformed_events_are_errors() {
        assert!(Event::parse("not json").is_err());
        assert!(Event::parse(r#"{"event":"url","tab":1}"#).is_err());
        assert!(Event::parse(r#"{"tab":1}"#).is_err());
    }

    #[test]
    fn quit_is_one_json_line() {
        let l = Command::Quit.line();
        assert!(l.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(l.trim()).unwrap();
        assert_eq!(v["cmd"], "quit");
    }

    #[test]
    fn c3_commands_match_protocol_md() {
        let cases = [
            (
                Command::Open {
                    url: "https://example.com/".into(),
                },
                r#"{"cmd":"open","url":"https://example.com/"}"#,
            ),
            (Command::Back, r#"{"cmd":"back"}"#),
            (Command::Forward, r#"{"cmd":"forward"}"#),
            (Command::Reload { hard: false }, r#"{"cmd":"reload"}"#),
            (Command::Reload { hard: true }, r#"{"cmd":"reload","hard":true}"#),
            (
                Command::Find {
                    id: 1,
                    query: "needle".into(),
                    dir: FindDir::Next,
                    match_case: false,
                    screen_top: false,
                },
                r#"{"cmd":"find","id":1,"query":"needle","dir":"next"}"#,
            ),
            (
                Command::Find {
                    id: 2,
                    query: "Needle".into(),
                    dir: FindDir::Prev,
                    match_case: true,
                    screen_top: false,
                },
                r#"{"cmd":"find","id":2,"query":"Needle","dir":"prev","match_case":true}"#,
            ),
            (
                Command::Find {
                    id: 3,
                    query: "x".into(),
                    dir: FindDir::Next,
                    match_case: false,
                    screen_top: true,
                },
                r#"{"cmd":"find","id":3,"query":"x","dir":"next","screen_top":true}"#,
            ),
            (Command::FindClear, r#"{"cmd":"find-clear"}"#),
            (Command::Caret { on: true }, r#"{"cmd":"caret","on":true}"#),
            (Command::Caret { on: false }, r#"{"cmd":"caret","on":false}"#),
            (
                Command::ForgetSite {
                    site: "https://a.example/".into(),
                },
                r#"{"cmd":"forget-site","site":"https://a.example/"}"#,
            ),
            (Command::Quit, r#"{"cmd":"quit"}"#),
        ];
        for (cmd, want) in cases {
            assert_eq!(cmd.line(), format!("{want}\n"));
        }
    }

    #[test]
    fn line_buffer_splits_and_keeps_partial_lines() {
        let mut b = LineBuffer::default();
        assert!(b.push(b"{\"a\":1").is_empty());
        assert_eq!(b.push(b"}\n{\"b\"").as_slice(), ["{\"a\":1}"]);
        assert_eq!(b.push(b":2}\n\n{\"c\":3}\n").as_slice(), ["{\"b\":2}", "{\"c\":3}"]);
        assert!(b.push(b"").is_empty());
    }
}
