//! The shell's side of the engine-host control protocol v0 (web/PROTOCOL.md): JSON lines,
//! events tagged by "event", commands by "cmd". Unknown fields and unknown events are
//! ignored (both sides ignore what they do not know).

use serde::Deserialize;

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
        index: u32,
        count: u32,
    },
    Crashed {
        tab: u64,
        #[serde(default)]
        reason: String,
    },
    Error {
        message: String,
    },
    /// An event this shell does not know (a newer host, or P-FORGET's `forgot`).
    #[serde(other)]
    Unknown,
}

impl Event {
    pub fn parse(line: &str) -> Result<Event, String> {
        serde_json::from_str(line).map_err(|e| format!("bad event {line:?}: {e}"))
    }
}

/// A command to the engine host. C2 only needs `quit`; C3 adds the rest of v0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Quit,
}

impl Command {
    /// One JSON line, `\n`-terminated.
    pub fn line(self) -> String {
        match self {
            Command::Quit => "{\"cmd\":\"quit\"}\n".to_string(),
        }
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
                r#"{"event":"find-result","tab":1,"index":2,"count":5}"#,
                Event::FindResult {
                    tab: 1,
                    index: 2,
                    count: 5,
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
        assert_eq!(Event::parse(r#"{"event":"forgot","site":"example.com"}"#).unwrap(), Event::Unknown);
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
    fn line_buffer_splits_and_keeps_partial_lines() {
        let mut b = LineBuffer::default();
        assert!(b.push(b"{\"a\":1").is_empty());
        assert_eq!(b.push(b"}\n{\"b\"").as_slice(), ["{\"a\":1}"]);
        assert_eq!(b.push(b":2}\n\n{\"c\":3}\n").as_slice(), ["{\"b\":2}", "{\"c\":3}"]);
        assert!(b.push(b"").is_empty());
    }
}
