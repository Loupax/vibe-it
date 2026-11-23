//! STASH.md "History" (C3): the file writer (ported from scratchy's QCOMP3 tests) and what
//! counts as a visit, from the host's events.

use crate::history::*;
use crate::protocol::Event;

struct TestDir(std::path::PathBuf);

impl TestDir {
    fn new(tag: &str) -> TestDir {
        let d = std::env::temp_dir().join(format!("web-c3-history-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        TestDir(d)
    }
    fn lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.0.join("history"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn history_one_line_per_visit_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let d = TestDir::new("line");
    append(&d.0, 1700000000000, "https://example.com/", "Example Domain").unwrap();
    assert_eq!(d.lines(), vec!["1700000000000\thttps://example.com/\tExample Domain"]);
    let mode = std::fs::metadata(d.0.join("history")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let dmode = std::fs::metadata(&d.0).unwrap().permissions().mode() & 0o777;
    assert_eq!(dmode, 0o700);
}

#[test]
fn history_visit_url_strips_the_fragment_and_skips_opaque_origins() {
    assert_eq!(visit_url("https://a.test/p?q#frag").as_deref(), Some("https://a.test/p?q"));
    assert_eq!(visit_url("http://a.test/").as_deref(), Some("http://a.test/"));
    for u in [
        "file:///tmp/help.html",
        "data:text/html,x",
        "about:blank",
        "chrome://version",
        "not a url",
    ] {
        assert_eq!(visit_url(u), None, "{u}");
    }
}

#[test]
fn history_title_with_tab_and_newline_stays_one_line_and_is_cut_at_4096() {
    let line = format_line(1, "https://a.test/", "a\tb\nc").unwrap();
    assert_eq!(line, "1\thttps://a.test/\ta b c\n");
    let long = "é".repeat(5000);
    let line = format_line(1, "https://a.test/", &long).unwrap();
    assert!(line.len() <= MAX_LINE && line.ends_with('\n'));
    assert!(format_line(1, &"x".repeat(5000), "t").is_none());
}

#[test]
fn history_trim_keeps_the_newest_lines() {
    let d = TestDir::new("trim");
    for i in 0..10 {
        append(&d.0, i, &format!("https://a.test/{i}"), "").unwrap();
    }
    trim(&d.0, 4).unwrap();
    let l = d.lines();
    assert_eq!(l.len(), 4);
    assert!(l[0].starts_with("6\t") && l[3].starts_with("9\t"), "{l:?}");
}

fn ev(json: &str) -> Event {
    Event::parse(json).unwrap()
}

fn url(u: &str) -> Event {
    ev(&format!(r#"{{"event":"url","tab":1,"url":"{u}"}}"#))
}

fn title(t: &str) -> Event {
    ev(&format!(r#"{{"event":"title","tab":1,"title":"{t}"}}"#))
}

fn load(phase: &str, status: u32, kind: &str, error: Option<&str>) -> Event {
    let err = error.map(|e| format!(r#","error":"{e}""#)).unwrap_or_default();
    ev(&format!(
        r#"{{"event":"load","tab":1,"phase":"{phase}","status":{status},"kind":"{kind}"{err}}}"#
    ))
}

fn feed(v: &mut Visits, evs: &[Event]) -> Vec<(String, String)> {
    evs.iter().filter_map(|e| v.event(e)).collect()
}

#[test]
fn history_a_new_load_that_ended_with_a_response_is_one_visit_of_its_final_url() {
    let mut v = Visits::default();
    let got = feed(
        &mut v,
        &[
            load("start", 0, "new", None),
            url("https://example.com/after-redirect#top"),
            title("Example"),
            load("end", 200, "new", None),
        ],
    );
    assert_eq!(got, vec![("https://example.com/after-redirect".to_string(), "Example".to_string())]);
    // A 404 is a page that loaded: a visit too.
    let got = feed(
        &mut v,
        &[
            load("start", 0, "new", None),
            url("https://example.com/missing"),
            load("end", 404, "new", None),
        ],
    );
    assert_eq!(
        got,
        vec![("https://example.com/missing".to_string(), String::new())],
        "the old title does not stick"
    );
}

#[test]
fn history_reload_traverse_same_document_failed_and_opaque_loads_are_not_visits() {
    let mut v = Visits::default();
    feed(&mut v, &[url("https://example.com/"), title("E")]);
    let none = feed(
        &mut v,
        &[
            load("start", 0, "reload", None),
            load("end", 200, "reload", None),
            load("start", 0, "traverse", None),
            load("end", 200, "traverse", None),
            url("https://example.com/#b"),
            load("start", 0, "same-document", None),
            load("end", 0, "same-document", None),
            load("start", 0, "new", None),
            load("end", 0, "new", Some("ERR_NAME_NOT_RESOLVED")),
            url("file:///run/web-1/help.html"),
            load("start", 0, "new", None),
            load("end", 200, "new", None),
        ],
    );
    assert!(none.is_empty(), "{none:?}");
}

#[test]
fn history_a_title_that_is_only_the_url_is_recorded_empty() {
    let mut v = Visits::default();
    let got = feed(
        &mut v,
        &[
            load("start", 0, "new", None),
            url("https://example.com/plain.txt"),
            title("example.com/plain.txt"),
            load("end", 200, "new", None),
        ],
    );
    assert_eq!(got[0].1, "");
}

#[test]
fn history_writer_records_into_the_state_dir() {
    let d = TestDir::new("writer");
    let h = History::open(Some(d.0.clone()));
    h.record("https://a.test/", "A");
    let l = d.lines();
    assert_eq!(l.len(), 1);
    assert!(l[0].ends_with("\thttps://a.test/\tA"), "{l:?}");
    History::open(None).record("https://a.test/", "A"); // no state dir: nothing, no panic
}
