// QWU1: one "wire not reachable" stderr line per page load.
#[allow(unused_imports)]
use super::*;

/// The tracker is process-global: these tests must not interleave.
static QWU1_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn fixture_fetches(path: &std::path::Path) {
    let page = Url::parse("https://example.test/").unwrap();
    let css = Url::parse("https://example.test/a.css").unwrap();
    assert!(fetch_stylesheet_via_wire(path, &css, &page).is_none());
    for i in 0..3 {
        let img = Url::parse(&format!("https://example.test/{i}.png")).unwrap();
        assert!(fetch_image_via_wire(path, &img, &page).is_none());
    }
}

#[test]
fn qwu1_missing_socket_prints_exactly_one_line_with_the_count() {
    let _g = QWU1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    wire_report_test_setup(30);
    let path = unique_test_socket_path("qwu1-missing");
    let _ = std::fs::remove_file(&path);
    begin_wire_load(1);
    fixture_fetches(&path);
    // Let the watcher settle by itself (quiet period), then check nothing more arrives.
    thread::sleep(std::time::Duration::from_millis(300));
    flush_wire_report();
    let lines = wire_report_test_take();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(
        lines[0],
        format!("scratchy: wire not reachable at {} (No such file or directory, os error 2): 4 sub-resources skipped", path.display())
    );
}

#[test]
fn qwu1_stale_socket_file_shows_connection_refused() {
    let _g = QWU1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    wire_report_test_setup(30);
    let path = unique_test_socket_path("qwu1-stale");
    let _ = std::fs::remove_file(&path);
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap()); // file stays, no listener
    begin_wire_load(1);
    fixture_fetches(&path);
    flush_wire_report();
    let lines = wire_report_test_take();
    let _ = std::fs::remove_file(&path);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("Connection refused, os error 111"), "{lines:?}");
    assert!(lines[0].ends_with(": 4 sub-resources skipped"));
}

#[test]
fn qwu1_working_socket_prints_no_line() {
    let _g = QWU1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    wire_report_test_setup(30);
    let path = unique_test_socket_path("qwu1-ok");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\n\r\np{margin:0}");
    begin_wire_load(1);
    let url = Url::parse("file:///tmp/a.css").unwrap();
    assert!(fetch_stylesheet_via_wire(&path, &url, &url).is_some());
    thread::sleep(std::time::Duration::from_millis(100));
    flush_wire_report();
    assert!(wire_report_test_take().is_empty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn qwu1_file_page_without_socket_prints_no_line() {
    let _g = QWU1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    wire_report_test_setup(30);
    // No load was begun (a file:// page without --socket never fetches):
    // even a stray failure must not print.
    wire_report_test_disarm();
    note_wire_skip(None, "no --socket given");
    thread::sleep(std::time::Duration::from_millis(100));
    flush_wire_report();
    assert!(wire_report_test_take().is_empty());
}

#[test]
fn qwu1_each_navigation_prints_its_own_line() {
    let _g = QWU1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    wire_report_test_setup(30);
    let path = unique_test_socket_path("qwu1-gens");
    let _ = std::fs::remove_file(&path);
    begin_wire_load(1);
    fixture_fetches(&path);
    flush_wire_report();
    begin_wire_load(2);
    let page = Url::parse("https://example.test/").unwrap();
    assert!(fetch_image_via_wire(&path, &Url::parse("https://example.test/x.png").unwrap(), &page).is_none());
    thread::sleep(std::time::Duration::from_millis(300));
    let lines = wire_report_test_take();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].ends_with(": 4 sub-resources skipped"));
    assert!(lines[1].ends_with(": 1 sub-resources skipped"));
}

#[test]
fn qwu1_no_socket_wording_and_page_js_failures_are_counted() {
    let _g = QWU1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    wire_report_test_setup(30);
    begin_wire_load(1);
    note_wire_skip(None, "unused");
    flush_wire_report();
    assert_eq!(wire_report_test_take(), vec!["scratchy: wire not reachable, no --socket given: 1 sub-resources skipped".to_string()]);
    // A page-JS request counts toward the same load (its own error is separate).
    let path = unique_test_socket_path("qwu1-js");
    let _ = std::fs::remove_file(&path);
    begin_wire_load(2);
    let limits = WireLimits::default();
    assert!(fetch_full_via_wire(&path, b"GET http://a.test/ HTTP/1.1\r\n\r\n", &limits).is_err());
    flush_wire_report();
    let lines = wire_report_test_take();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].ends_with(": 1 sub-resources skipped"));
}
