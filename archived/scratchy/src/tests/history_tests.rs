// QCOMP3: the history file writer (src/history.rs, STASH.md "History"). Real disk under target/, never /tmp.
use crate::history;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// `history::init` sets a process-wide directory: App tests take turns.
static DIR_LOCK: Mutex<()> = Mutex::new(());

struct TestDir(PathBuf);

impl TestDir {
    fn new(tag: &str) -> TestDir {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("history-tests").join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TestDir(p)
    }
    fn lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.0.join("history")).unwrap_or_default().lines().map(String::from).collect()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fields(line: &str) -> Vec<&str> {
    line.split('\t').collect()
}

#[test]
fn qcomp3_one_line_per_visit_with_title() {
    let d = TestDir::new("basic");
    history::append(&d.0, 1700000000123, "https://a.example/x", "Hello").unwrap();
    history::append(&d.0, 1700000000456, "https://a.example/y", "").unwrap();
    let l = d.lines();
    assert_eq!(l, vec!["1700000000123\thttps://a.example/x\tHello", "1700000000456\thttps://a.example/y\t"]);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(d.0.join("history")).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn qcomp3_visit_url_strips_fragment_and_skips_opaque_origins() {
    let u = |s: &str| history::visit_url(&url::Url::parse(s).unwrap());
    assert_eq!(u("https://a.example/p?q=1#a").as_deref(), Some("https://a.example/p?q=1"));
    assert_eq!(u("https://a.example/p#a"), u("https://a.example/p#b"));
    assert_eq!(u("file:///home/x/index.html"), None);
    assert_eq!(u("data:text/html,hi"), None);
    assert_eq!(u("about:blank"), None);
}

#[test]
fn qcomp3_title_with_tab_and_newline_stays_one_line() {
    let d = TestDir::new("escape");
    history::append(&d.0, 1, "https://a.example/", "a\tb\nc\r\nd\u{7}e").unwrap();
    let l = d.lines();
    assert_eq!(l.len(), 1);
    assert_eq!(fields(&l[0]), vec!["1", "https://a.example/", "a b c  d e"]);
}

#[test]
fn qcomp3_long_title_is_cut_and_line_fits_4096() {
    let d = TestDir::new("long");
    let title = "é".repeat(5000); // 2 bytes each: the cut must land on a char boundary
    history::append(&d.0, 1700000000000, "https://a.example/", &title).unwrap();
    let raw = std::fs::read(d.0.join("history")).unwrap();
    assert!(raw.len() <= 4096 && raw.len() > 4000, "{}", raw.len());
    assert!(String::from_utf8(raw.clone()).is_ok());
    assert_eq!(*raw.last().unwrap(), b'\n');
    // A URL too long for any line is not recorded at all.
    let huge = format!("https://a.example/{}", "x".repeat(5000));
    history::append(&d.0, 2, &huge, "t").unwrap();
    assert_eq!(d.lines().len(), 1);
}

#[test]
fn qcomp3_trim_keeps_the_newest_lines() {
    let d = TestDir::new("trim");
    for i in 0..10 {
        history::append(&d.0, i, &format!("https://a.example/{i}"), "t").unwrap();
    }
    history::trim(&d.0, 4).unwrap();
    let l = d.lines();
    assert_eq!(l.len(), 4);
    assert!(l[0].starts_with("6\t") && l[3].starts_with("9\t"), "{l:?}");
    history::trim(&d.0, 4).unwrap(); // at the limit: untouched
    assert_eq!(d.lines(), l);
    history::trim(&d.0, 100).unwrap();
    assert_eq!(d.lines(), l);
    history::trim(&d.0.join("nope"), 4).unwrap(); // no file: fine
}

#[test]
fn qcomp3_concurrent_writers_produce_whole_lines() {
    let d = TestDir::new("conc");
    let dir = d.0.clone();
    let handles: Vec<_> = (0..4u64)
        .map(|t| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                // Each append opens its own O_APPEND handle, like another process would.
                let title = format!("T{t}-{}", "w".repeat(3000));
                for i in 0..200u64 {
                    history::append(&dir, t * 1000 + i, &format!("https://a.example/{t}/{i}"), &title).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let l = d.lines();
    assert_eq!(l.len(), 800);
    for line in &l {
        let f = fields(line);
        assert_eq!(f.len(), 3, "{line}");
        assert!(f[2].starts_with('T') && f[2].ends_with('w'), "torn line");
    }
}

// ---------- App path: which navigations are visits ----------

mod app_visits {
    use super::*;
    use crate::tests::*;

    fn page(url: &str, title: &str) -> FetchedPage {
        FetchedPage {
            html: format!("<html><head><title>{title}</title></head><body>x</body></html>").into_bytes(),
            extra_css: String::new(),
            sheets: Vec::new(),
            scripts: Vec::new(),
            url: Url::parse(url).unwrap(),
        }
    }

    /// Navigates (link/`:go`/script) to `href`; the fetch answers with `final_url`.
    fn visit(app: &mut App, href: &str, final_url: &str, title: &str) {
        app.navigate_to(href, true, false);
        let g = app.nav_generation;
        app.handle_nav_result(g, Ok(page(final_url, title)));
    }

    #[test]
    fn qcomp3_app_records_visits_not_reloads_or_traversal() {
        let Ok(event_loop) = test_event_loop() else {
            return; // no display available (headless CI)
        };
        let _g = DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = TestDir::new("app");
        history::init(Some(d.0.clone()));
        let mut app = App::new_static(parse_html("<html></html>"), event_loop.create_proxy(), None);
        app.base_url = Some(Url::parse("https://a.example/").unwrap());

        visit(&mut app, "https://a.example/one", "https://a.example/one", "One");
        assert_eq!(d.lines().len(), 1);
        assert_eq!(fields(&d.lines()[0])[1..], ["https://a.example/one", "One"]);

        // A redirect records the final URL; `#a` is stripped.
        visit(&mut app, "https://a.example/old#a", "https://b.example/new#a", "New");
        let l = d.lines();
        assert_eq!(l.len(), 2);
        assert_eq!(fields(&l[1])[1], "https://b.example/new");

        // Reload and back/forward add nothing.
        app.refresh();
        let g = app.nav_generation;
        app.handle_nav_result(g, Ok(page("https://b.example/new", "New")));
        app.navigate_history(-1);
        let g = app.nav_generation;
        app.handle_nav_result(g, Ok(page("https://a.example/one", "One")));
        app.navigate_history(1);
        let g = app.nav_generation;
        app.handle_nav_result(g, Ok(page("https://b.example/new", "New")));
        assert_eq!(d.lines().len(), 2, "{:?}", d.lines());

        // A failed load adds nothing; a file: page is not recorded.
        app.navigate_to("https://c.example/", true, false);
        let g = app.nav_generation;
        app.handle_nav_result(g, Err("no response".into()));
        visit(&mut app, "file:///tmp/x.html", "file:///tmp/x.html", "Local");
        assert_eq!(d.lines().len(), 2);

        // Same-document (fragment only) navigation adds nothing.
        visit(&mut app, "https://a.example/two", "https://a.example/two", "Two");
        assert_eq!(d.lines().len(), 3);
        app.navigate_to("#frag", true, false);
        assert_eq!(d.lines().len(), 3);
        history::init(None);
    }
}
