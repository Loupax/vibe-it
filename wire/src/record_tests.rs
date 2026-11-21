// Tests for record.rs (QREC1). Test recordings live only in a temp dir and are
// removed again; they may never be committed (scratchy DECISIONS Q5).

use crate::record::*;
use crate::{ParsedRequest, Shared, build_agent, execute_with, handle_connection};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use url::Url;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wire-record-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A one-thread origin on 127.0.0.1 that answers `n` requests: /big is 100
/// digits, /n counts "call 0", "call 1", ..., everything else echoes its path.
fn origin(n: usize) -> (u16, std::thread::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        for i in 0..n {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = Vec::new();
            let mut b = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let k = s.read(&mut b).unwrap();
                buf.extend_from_slice(&b[..k]);
            }
            let text = String::from_utf8_lossy(&buf).to_string();
            let path = text.split_whitespace().nth(1).unwrap().to_string();
            let body = if path == "/big" {
                (0..100).map(|i| char::from(b'0' + (i % 10) as u8)).collect::<String>()
            } else if path == "/n" {
                format!("call {i}")
            } else {
                format!("path {path}")
            };
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            s.write_all(resp.as_bytes()).unwrap();
        }
    });
    (port, h)
}

fn get(shared: &Shared, url: &str, headers: &[(&str, &str)], body: &[u8]) -> (String, Vec<u8>) {
    let req = ParsedRequest {
        method: if body.is_empty() { "GET" } else { "POST" }.into(),
        url: Url::parse(url).unwrap(),
        headers: headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        body: body.to_vec(),
    };
    let mut raw = Vec::new();
    execute_with(&build_agent(), shared, &req, &mut raw).unwrap();
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (String::from_utf8(raw[..end].to_vec()).unwrap(), raw[end + 4..].to_vec())
}

#[test]
fn record_then_replay_without_network() {
    let dir = tmp("rt");
    let (port, server) = origin(5);
    let base = format!("http://127.0.0.1:{port}");
    let mut rec = Shared::new(false);
    rec.recorder = Some(Recorder::open(&dir).unwrap());
    let (h, b) = get(&rec, &format!("{base}/a?cb=1"), &[], b"");
    assert!(h.starts_with("HTTP/1.1 200") && b == b"path /a?cb=1", "recording does not change the answer: {h}");
    get(&rec, &format!("{base}/big"), &[], b"");
    get(&rec, &format!("{base}/n"), &[], b"");
    get(&rec, &format!("{base}/n"), &[], b"");
    get(&rec, &format!("{base}/same"), &[], b"");
    server.join().unwrap();
    drop(rec);

    let index = std::fs::read_to_string(dir.join("index.jsonl")).unwrap();
    assert_eq!(index.lines().count(), 5);
    assert_eq!(std::fs::read_dir(dir.join("blobs")).unwrap().count(), 5, "five different bodies, one blob each");
    let big: Vec<u8> = (0..100).map(|i| b'0' + (i % 10) as u8).collect();
    let blob = std::fs::read(dir.join("blobs").join(sha256_hex(&big))).unwrap();
    assert_eq!(&blob[..4], &[0x28, 0xb5, 0x2f, 0xfd], "blobs are zstd frames");

    // The origin is gone: everything below comes from the recording.
    let mut rep = Shared::new(false);
    rep.replayer = Some(Replayer::open(&dir, Rules::parse("127.0.0.1 cb"), Duration::ZERO).unwrap());
    let (h, b) = get(&rep, &format!("{base}/a?cb=1"), &[], b"");
    assert!(h.starts_with("HTTP/1.1 200 OK") && h.contains("X-Wire-Replay: hit") && b == b"path /a?cb=1", "{h}");
    let (_, b) = get(&rep, &format!("{base}/a?cb=999"), &[], b"");
    assert_eq!(b, b"path /a?cb=1", "ignore list drops cb");
    let b1 = get(&rep, &format!("{base}/n"), &[], b"").1;
    let b2 = get(&rep, &format!("{base}/n"), &[], b"").1;
    let b3 = get(&rep, &format!("{base}/n"), &[], b"").1;
    assert_eq!((b1, b2, b3), (b"call 2".to_vec(), b"call 3".to_vec(), b"call 3".to_vec()), "n-th same-path, then the last");
    let (_, b) = get(&rep, &format!("{base}/same?other=1"), &[], b"");
    assert_eq!(b, b"path /same", "falls back to the same path");
    assert_eq!(rep.replayer.as_ref().unwrap().misses(), 0);

    // Range inside a recorded full body.
    let (h, b) = get(&rep, &format!("{base}/big"), &[("Range", "bytes=10-19")], b"");
    assert!(h.starts_with("HTTP/1.1 206") && h.contains("Content-Range: bytes 10-19/100") && h.contains("Content-Length: 10"), "{h}");
    assert_eq!(b, b"0123456789");

    // A miss is 404 with the marker and is counted.
    let (h, _) = get(&rep, "http://never-recorded.invalid/x", &[], b"");
    assert!(h.starts_with("HTTP/1.1 404") && h.contains("X-Wire-Replay: miss"), "{h}");
    assert_eq!(rep.replayer.as_ref().unwrap().misses(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn post_body_hash_and_recorded_range_matching() {
    let dir = tmp("post");
    let rec = Recorder::open(&dir).unwrap();
    let u = Url::parse("http://h.test/p").unwrap();
    let ok = |body: &str| format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n{body}").into_bytes();
    rec.record("POST", &u, None, b"one", &ok("answer one"));
    rec.record("POST", &u, None, b"two", &ok("answer two"));
    let body: String = (0..100).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    let part = format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 100-199/1000\r\n\r\n{body}").into_bytes();
    rec.record("GET", &Url::parse("http://h.test/v").unwrap(), Some("bytes=100-199"), b"", &part);
    rec.record("GET", &Url::parse("http://h.test/e").unwrap(), None, b"", b"HTTP/1.1 200 OK\r\nX-Wire-Error: dns\r\n\r\nboom");
    drop(rec);
    assert_eq!(std::fs::read_to_string(dir.join("index.jsonl")).unwrap().lines().count(), 3, "X-Wire-Error answers are not recorded");
    let mut rep = Shared::new(false);
    rep.replayer = Some(Replayer::open(&dir, Rules::default(), Duration::ZERO).unwrap());
    assert_eq!(get(&rep, "http://h.test/p", &[], b"two").1, b"answer two");
    assert_eq!(get(&rep, "http://h.test/p", &[], b"one").1, b"answer one");
    let (h, _) = get(&rep, "http://h.test/v", &[("Range", "bytes=100-199")], b"");
    assert!(h.starts_with("HTTP/1.1 206") && h.contains("Content-Range: bytes 100-199/1000"), "{h}");
    let (h, b) = get(&rep, "http://h.test/v", &[("Range", "bytes=110-119")], b"");
    assert!(h.starts_with("HTTP/1.1 206") && h.contains("Content-Range: bytes 110-119/1000"), "inside the recorded range: {h}");
    assert_eq!(b, body.as_bytes()[10..20]);
    let (h, _) = get(&rep, "http://h.test/v", &[("Range", "bytes=0-9")], b"");
    assert!(h.starts_with("HTTP/1.1 404") && h.contains("X-Wire-Replay: miss"), "outside the recorded range is a miss: {h}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn replay_delay_and_rules_parse() {
    assert_eq!(parse_delay("5ms").unwrap(), Duration::from_millis(5));
    assert_eq!(parse_delay("2s").unwrap(), Duration::from_secs(2));
    assert!(parse_delay("fast").is_err());
    let dir = tmp("delay");
    Recorder::open(&dir).unwrap();
    let mut rep = Shared::new(false);
    rep.replayer = Some(Replayer::open(&dir, Rules::parse("# comment\n* cb, _\nyoutube.com rn,cpn\n"), Duration::from_millis(40)).unwrap());
    let t = Instant::now();
    get(&rep, "http://h.test/x", &[], b"");
    assert!(t.elapsed() >= Duration::from_millis(40));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn websocket_is_refused_in_replay_and_mounts_are_not_recorded() {
    let dir = tmp("ws");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.jsonl"), "").unwrap();
    let mut rep = Shared::new(false);
    rep.replayer = Some(Replayer::open(&dir, Rules::default(), Duration::ZERO).unwrap());
    let (mut a, b) = UnixStream::pair().unwrap();
    a.write_all(b"GET http://h.test/ws HTTP/1.1\r\nX-Wire-Upgrade: websocket\r\n\r\n").unwrap();
    handle_connection(b, &build_agent(), &rep);
    let mut out = String::new();
    a.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 403") && out.contains("X-Wire-Replay: websocket-refused"), "{out}");

    let site = tmp("site");
    std::fs::create_dir_all(&site).unwrap();
    std::fs::write(site.join("a.txt"), "mounted").unwrap();
    let rdir = tmp("mountrec");
    let mut rec = Shared::new(false);
    rec.mounts.add_mount(&format!("http://m.test:8000={}", site.display())).unwrap();
    rec.recorder = Some(Recorder::open(&rdir).unwrap());
    assert_eq!(get(&rec, "http://m.test:8000/a.txt", &[], b"").1, b"mounted");
    assert_eq!(std::fs::read_to_string(rdir.join("index.jsonl")).unwrap(), "", "a mounted origin is never recorded");
    for d in [dir, site, rdir] {
        let _ = std::fs::remove_dir_all(&d);
    }
}
