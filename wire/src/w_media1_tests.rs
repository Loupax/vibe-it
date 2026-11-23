// W-MEDIA1 tests: `file:` URLs honour Range, send Accept-Ranges and name media types.

use crate::{Caps, ParsedRequest, serve_file};
use std::io::Write;
use url::Url;

fn get(path: &std::path::Path, range: Option<&str>) -> (String, Vec<u8>) {
    let headers = range.map(|r| vec![("Range".to_string(), r.to_string())]).unwrap_or_default();
    let req = ParsedRequest { method: "GET".into(), url: Url::from_file_path(path).unwrap(), headers, body: Vec::new() };
    let mut out = Vec::new();
    serve_file(&req, &Caps::default(), &mut out).unwrap();
    let p = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (String::from_utf8_lossy(&out[..p + 2]).into_owned(), out[p + 4..].to_vec())
}

fn temp(name: &str, data: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wire-w-media1-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::File::create(&p).unwrap().write_all(data).unwrap();
    p
}

#[test]
fn ranges_and_plain_200() {
    let p = temp("digits.bin", b"0123456789ABCDEFGHIJ");
    let (h, b) = get(&p, None);
    assert!(h.starts_with("HTTP/1.1 200"), "{h}");
    assert!(h.contains("Accept-Ranges: bytes\r\n"), "{h}");
    assert_eq!(b, b"0123456789ABCDEFGHIJ");

    let (h, b) = get(&p, Some("bytes=0-9"));
    assert!(h.starts_with("HTTP/1.1 206"), "{h}");
    assert!(h.contains("Content-Range: bytes 0-9/20\r\n") && h.contains("Accept-Ranges: bytes\r\n"), "{h}");
    assert_eq!(b, b"0123456789");

    let (h, b) = get(&p, Some("bytes=5-"));
    assert!(h.contains("Content-Range: bytes 5-19/20\r\n"), "{h}");
    assert_eq!(b, b"56789ABCDEFGHIJ");

    let (h, b) = get(&p, Some("bytes=-3"));
    assert!(h.contains("Content-Range: bytes 17-19/20\r\n"), "{h}");
    assert_eq!(b, b"HIJ");

    let (h, b) = get(&p, Some("bytes=15-100"));
    assert!(h.contains("Content-Range: bytes 15-19/20\r\n"), "{h}");
    assert_eq!(b, b"FGHIJ");
}

#[test]
fn out_of_range_is_416() {
    let p = temp("short.bin", b"0123456789");
    for r in ["bytes=10-", "bytes=50-60", "bytes=-0"] {
        let (h, b) = get(&p, Some(r));
        assert!(h.starts_with("HTTP/1.1 416"), "{r}: {h}");
        assert!(h.contains("Content-Range: bytes */10\r\n"), "{r}: {h}");
        assert!(b.is_empty());
    }
}

#[test]
fn multi_range_and_junk_answer_the_whole_file() {
    let p = temp("whole.bin", b"0123456789");
    for r in ["bytes=0-1,4-5", "items=0-1", "bytes=abc", "bytes=5-2"] {
        let (h, b) = get(&p, Some(r));
        assert!(h.starts_with("HTTP/1.1 200"), "{r}: {h}");
        assert_eq!(b, b"0123456789", "{r}");
    }
}

#[test]
fn media_types() {
    for (ext, ty) in [("mp4", "video/mp4"), ("webm", "video/webm"), ("ogg", "audio/ogg"), ("mp3", "audio/mpeg"), ("m4a", "audio/mp4"), ("opus", "audio/opus"), ("wav", "audio/wav")] {
        let p = temp(&format!("clip.{ext}"), b"x");
        let (h, _) = get(&p, None);
        assert!(h.contains(&format!("Content-Type: {ty}\r\n")), "{ext}: {h}");
    }
}

#[test]
fn big_file_slice_reads_only_the_slice() {
    let p = temp("big.mp4", b"");
    let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
    f.set_len(64 * 1024 * 1024).unwrap();
    let (h, b) = get(&p, Some("bytes=0-1023"));
    assert!(h.contains("Content-Range: bytes 0-1023/67108864\r\n"), "{h}");
    assert_eq!(b.len(), 1024);
}
