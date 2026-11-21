//! QREC2: `--deterministic` (report modes): two runs of a page that prints Math.random(),
//! crypto.getRandomValues, Date.now() and the completion order of two racing fetches print the
//! same lines; without the flag they differ. The "network" is a tiny wire stand-in on a unix
//! socket that answers the first request late (a race the order must not depend on).

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::process::Command;

const PAGE: &str = r#"<html><head><base href="http://det.test/"></head><body><script>
var a = new Uint32Array(2); crypto.getRandomValues(a);
console.error('RND', Math.random(), Array.from(a).join(','), crypto.randomUUID(), Date.now(), +new Date());
var order = [];
function done(n) { order.push(n); if (order.length === 2) console.error('ORDER', order.join('')); }
fetch('/slow').then(function (r) { return r.text(); }).then(function () { done('slow'); });
fetch('/fast').then(function (r) { return r.text(); }).then(function () { done('fast'); });
</script></body></html>"#;

fn serve(listener: UnixListener) {
    for stream in listener.incoming() {
        let Ok(mut s) = stream else { return };
        std::thread::spawn(move || {
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let line = String::from_utf8_lossy(&req).lines().next().unwrap_or("").to_string();
            let url = line.split(' ').nth(1).unwrap_or("/").to_string();
            if url.contains("slow") {
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            let body = "ok";
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nX-Wire-Version: 2\r\nX-Wire-Final-Url: http://det.test{url}\r\n\r\n{body}",
                body.len()
            );
        });
    }
}

fn run(dir: &std::path::Path, sock: &std::path::Path, flags: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_scratchy"))
        .arg("--js-report")
        .args(flags)
        .arg("--socket")
        .arg(sock)
        .arg(dir.join("page.html"))
        .output()
        .expect("run scratchy");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let mut lines: Vec<String> = text.lines().filter(|l| l.contains("RND") || l.contains("ORDER")).map(|l| l.trim().to_string()).collect();
    lines.sort();
    lines.dedup(); // the report prints each console line twice (live and in the summary)
    assert!(lines.iter().any(|l| l.contains("RND")), "no RND line in:\n{text}");
    lines.join("\n")
}

#[test]
fn deterministic_runs_are_identical_and_plain_runs_differ() {
    let dir = std::env::temp_dir().join(format!("scratchy-det-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("page.html"), PAGE).unwrap();
    let sock = dir.join("w.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    std::thread::spawn(move || serve(listener));

    let a = run(&dir, &sock, &["--deterministic=1700000000000"]);
    let b = run(&dir, &sock, &["--deterministic=1700000000000"]);
    assert_eq!(a, b, "two deterministic runs differ");
    assert!(a.contains("1700000000000"), "Date does not start at the epoch: {a}");
    assert!(a.contains("ORDER slowfast"), "network results not in request order: {a}");

    let c = run(&dir, &sock, &[]);
    let d = run(&dir, &sock, &[]);
    assert_ne!(c, d, "two plain runs are identical");

    // Window mode never runs deterministic.
    let out = Command::new(env!("CARGO_BIN_EXE_scratchy")).arg("--deterministic").arg("x.html").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&dir);
}
