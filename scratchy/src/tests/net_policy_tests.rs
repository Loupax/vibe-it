#[allow(unused_imports)]
use super::*;

// ---------- QF5: net policy + wire codec ----------

fn qf5_url(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[test]
fn qf5_is_public_ip_classes() {
    let no = [
        "127.0.0.1", "127.9.9.9", "10.0.0.1", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254", "169.254.0.1",
        "100.64.0.1", "100.127.255.255", "0.0.0.0", "0.1.2.3", "224.0.0.1", "255.255.255.255", "192.0.2.1", "198.51.100.1",
        "203.0.113.1", "198.18.0.1", "240.0.0.1", "::1", "::", "fe80::1", "febf::1", "fc00::1", "fd12::1", "ff02::1",
        "2001:db8::1", "::ffff:127.0.0.1", "::ffff:10.1.2.3", "::ffff:169.254.169.254", "64:ff9b::7f00:1",
    ];
    for s in no {
        assert!(!is_public_ip(s.parse().unwrap()), "{s} must not be public");
    }
    let yes = ["8.8.8.8", "1.1.1.1", "172.15.0.1", "172.32.0.1", "100.63.255.255", "100.128.0.1", "2606:4700::1111", "::ffff:8.8.8.8"];
    for s in yes {
        assert!(is_public_ip(s.parse().unwrap()), "{s} must be public");
    }
}

fn qf5_block_policy() -> RequestPolicy {
    let mut p = RequestPolicy::default();
    p.private_targets = PrivatePolicy::Block;
    p.file_targets = PrivatePolicy::Block;
    p
}

#[test]
fn qf5_warn_and_allow_modes_and_redirect_hops() {
    let p = RequestPolicy::default();
    assert_eq!((p.private_targets, p.file_targets), (PrivatePolicy::Warn, PrivatePolicy::Warn));
    let page = qf5_url("https://example.com/a");
    let cases = [
        ("https://127.0.0.1/", "loopback"), ("https://localhost/", "loopback"), ("https://[::1]/", "loopback"),
        ("https://10.0.0.5/", "private"), ("https://192.168.0.1/", "private"), ("https://172.20.1.1/", "private"),
        ("https://169.254.169.254/", "link-local"), ("https://[fe80::1]/", "link-local"),
        ("https://100.64.0.9/", "cgnat"), ("https://[fd00::1]/", "ula"), ("https://[::ffff:10.0.0.1]/", "private"),
        ("https://nas.local/", "local-name"), ("https://db.internal/", "local-name"), ("file:///etc/os-release", "file"),
    ];
    for (u, class) in cases {
        let out = p.check_request(&page, &qf5_url(u), "GET", false).unwrap_or_else(|e| panic!("{u}: {e}"));
        assert_eq!(out.warning, Some(class), "{u}");
    }
    assert_eq!(p.check_request(&page, &qf5_url("https://example.org/"), "GET", false).unwrap().warning, None);
    // same origin never warns
    let dev = qf5_url("http://localhost:3000/");
    assert_eq!(p.check_request(&dev, &qf5_url("http://localhost:3000/x"), "GET", false).unwrap().warning, None);
    // still enforced in Warn mode: schemes, ports, mixed content (except loopback), methods
    assert!(p.check_request(&page, &qf5_url("ftp://x.test/"), "GET", false).is_err());
    assert_eq!(p.check_request(&page, &qf5_url("http://127.0.0.1:22/"), "GET", false), Err(PolicyError::BadPort(22)));
    assert_eq!(p.check_request(&page, &qf5_url("http://10.0.0.1/"), "GET", false), Err(PolicyError::MixedContent));
    assert_eq!(p.check_request(&page, &qf5_url("http://localhost:8080/"), "GET", false).unwrap().warning, Some("loopback"));
    assert!(p.check_request(&page, &qf5_url("https://10.0.0.1/"), "TRACE", false).is_err());
    // Allow: no warning
    let mut a = RequestPolicy::default();
    a.private_targets = PrivatePolicy::Allow;
    a.file_targets = PrivatePolicy::Allow;
    assert_eq!(a.check_request(&page, &qf5_url("https://10.0.0.1/"), "GET", false).unwrap().warning, None);
    assert_eq!(a.check_request(&page, &qf5_url("file:///x"), "GET", false).unwrap().warning, None);
    // the warning line
    assert_eq!(
        format_net_warning(&page, &qf5_url("https://10.0.0.1/x"), "private"),
        "scratchy: net warning: https://example.com/a -> https://10.0.0.1/x (private) allowed by policy"
    );
    // redirect hops re-run the policy
    let b = qf5_block_policy();
    let (m, body, _) = b.check_redirect_hop(&page, &qf5_url("https://cdn.example.org/y"), 302, "POST", true, 1).unwrap();
    assert_eq!((m.as_str(), body), ("GET", false));
    let (m, body, _) = b.check_redirect_hop(&page, &qf5_url("https://cdn.example.org/y"), 307, "POST", true, 1).unwrap();
    assert_eq!((m.as_str(), body), ("POST", true));
    assert!(matches!(b.check_redirect_hop(&page, &qf5_url("http://169.254.169.254/"), 302, "GET", false, 1), Err(_)));
    assert!(matches!(b.check_redirect_hop(&page, &qf5_url("https://10.0.0.1/"), 302, "GET", false, 2), Err(PolicyError::PrivateHost(_))));
    assert_eq!(p.check_redirect_hop(&page, &qf5_url("https://10.0.0.1/"), 302, "GET", false, 2).unwrap().2.warning, Some("private"));
    assert_eq!(p.check_redirect_hop(&page, &qf5_url("https://x.example/"), 302, "GET", false, 6).unwrap_err(), PolicyError::Redirects(5));
}

#[test]
fn qf5_check_request_rules() {
    let p = qf5_block_policy();
    let page = qf5_url("https://example.com/a");
    let ok = |u: &str, m: &str, body: bool| p.check_request(&page, &qf5_url(u), m, body);
    assert!(ok("https://api.example.org/x", "GET", false).is_ok());
    // address classes by IP literal and name
    for u in [
        "https://127.0.0.1/", "https://10.0.0.5/", "https://[::1]/", "https://169.254.169.254/latest", "https://[::ffff:7f00:1]/",
        "https://localhost/", "https://a.localhost/", "https://printer.local/", "https://db.internal/", "https://LOCALHOST./",
        "https://0x7f.1/",
    ] {
        assert!(matches!(ok(u, "GET", false), Err(PolicyError::PrivateHost(_))), "{u}");
    }
    // schemes
    assert_eq!(ok("file:///etc/passwd", "GET", false), Err(PolicyError::FileNotAllowed));
    assert!(matches!(ok("ftp://example.com/", "GET", false), Err(PolicyError::Scheme(_))));
    assert!(matches!(ok("data:text/plain,hi", "GET", false), Err(PolicyError::Scheme(_))));
    // ports
    for port in [1, 22, 25, 101, 104, 513, 6000, 6667, 10080] {
        assert_eq!(ok(&format!("https://example.org:{port}/"), "GET", false), Err(PolicyError::BadPort(port)));
    }
    assert!(ok("https://example.org:8443/", "GET", false).is_ok());
    // mixed content
    assert_eq!(ok("http://example.org/", "GET", false), Err(PolicyError::MixedContent));
    assert!(p.check_request(&qf5_url("http://example.com/"), &qf5_url("http://example.org/"), "GET", false).is_ok());
    // methods and bodies
    for (m, body) in [("POST", true), ("PUT", true), ("PATCH", true), ("DELETE", true), ("OPTIONS", false), ("HEAD", false), ("post", true)] {
        assert!(ok("https://example.org/", m, body).is_ok(), "{m}");
    }
    for m in ["CONNECT", "TRACE", "track", "TRACK"] {
        assert!(matches!(ok("https://example.org/", m, false), Err(PolicyError::MethodForbidden(_))), "{m}");
    }
    assert!(matches!(ok("https://example.org/", "GET", true), Err(PolicyError::BodyNotAllowed(_))));
    assert!(matches!(ok("https://example.org/", "HEAD", true), Err(PolicyError::BodyNotAllowed(_))));
    assert!(matches!(ok("https://example.org/", "bad method", false), Err(PolicyError::MethodForbidden(_))));
    // url length
    let long = format!("https://example.org/{}", "a".repeat(p.max_url_len));
    assert!(matches!(ok(&long, "GET", false), Err(PolicyError::UrlTooLong(_))));
    // configurable
    let mut q = qf5_block_policy();
    q.private_targets = PrivatePolicy::Allow;
    assert_eq!(q.check_request(&page, &qf5_url("https://localhost/"), "GET", false), Ok(PolicyOutcome::default()));
}

#[test]
fn qf5_same_origin_exception_and_file_flag() {
    let p = qf5_block_policy();
    let dev = qf5_url("http://localhost:3000/app");
    assert!(p.check_request(&dev, &qf5_url("http://localhost:3000/api"), "POST", true).is_ok());
    assert!(p.check_request(&dev, &qf5_url("http://localhost:3001/api"), "GET", false).is_err());
    assert!(p.check_request(&dev, &qf5_url("http://127.0.0.1:3000/api"), "GET", false).is_err());
    // the bad-port check still applies to a same-origin private target
    let bad = qf5_url("http://localhost:6000/");
    assert_eq!(p.check_request(&bad, &bad, "GET", false), Err(PolicyError::BadPort(6000)));
    // file:
    let file_page = qf5_url("file:///home/u/a.html");
    assert_eq!(p.check_request(&file_page, &qf5_url("file:///etc/passwd"), "GET", false), Err(PolicyError::FileNotAllowed));
    let mut q = qf5_block_policy();
    q.file_targets = PrivatePolicy::Warn;
    assert_eq!(q.check_request(&file_page, &qf5_url("file:///home/u/b.json"), "GET", false).unwrap().warning, Some("file"));
    // a file page is never same-origin, so private hosts stay blocked in Block mode
    assert!(q.check_request(&file_page, &qf5_url("http://localhost/"), "GET", false).is_err());
}

fn qf5_h(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

#[test]
fn qf5_cors_classification_and_preflight() {
    let page = qf5_url("https://a.example/p");
    let same = qf5_url("https://a.example/x");
    let other = qf5_url("https://b.example/x");
    assert_eq!(classify_fetch(RequestMode::SameOrigin, &page, &same, "GET", &[]), Ok(FetchKind::Basic));
    assert!(classify_fetch(RequestMode::SameOrigin, &page, &other, "GET", &[]).is_err());
    assert_eq!(classify_fetch(RequestMode::NoCors, &page, &other, "GET", &[]), Ok(FetchKind::Opaque));
    assert!(classify_fetch(RequestMode::NoCors, &page, &other, "PUT", &[]).is_err());
    assert_eq!(classify_fetch(RequestMode::Cors, &page, &other, "GET", &[]), Ok(FetchKind::Cors { preflight: false }));
    assert_eq!(
        classify_fetch(RequestMode::Cors, &page, &other, "POST", &qf5_h(&[("Content-Type", "text/plain;charset=UTF-8"), ("Accept", "*/*")])),
        Ok(FetchKind::Cors { preflight: false })
    );
    assert_eq!(
        classify_fetch(RequestMode::Cors, &page, &other, "POST", &qf5_h(&[("Content-Type", "application/json")])),
        Ok(FetchKind::Cors { preflight: true })
    );
    assert!(needs_preflight("PUT", &[]) && needs_preflight("DELETE", &[]) && needs_preflight("PATCH", &[]));
    assert!(needs_preflight("GET", &qf5_h(&[("X-Token", "1")])));
    assert!(needs_preflight("GET", &qf5_h(&[("Authorization", "x")])));
    assert!(!needs_preflight("get", &qf5_h(&[("accept-language", "en")])));
    assert!(needs_preflight("GET", &qf5_h(&[("Accept", "a\"b")])));
    assert_eq!(cors_unsafe_request_headers(&qf5_h(&[("X-B", "1"), ("x-a", "1"), ("X-B", "2"), ("Accept", "*/*")])), ["x-a", "x-b"]);
}

#[test]
fn qf5_cors_response_decisions() {
    let o = "https://a.example";
    assert!(check_cors_response(o, false, &qf5_h(&[("Access-Control-Allow-Origin", "*")])).is_ok());
    // `*` with credentials is refused
    assert!(check_cors_response(o, true, &qf5_h(&[("Access-Control-Allow-Origin", "*")])).is_err());
    assert!(check_cors_response(o, false, &qf5_h(&[("access-control-allow-origin", o)])).is_ok());
    assert!(check_cors_response(o, false, &qf5_h(&[("Access-Control-Allow-Origin", "https://b.example")])).is_err());
    assert!(check_cors_response(o, false, &[]).is_err());
    assert!(check_cors_response(o, false, &qf5_h(&[("Access-Control-Allow-Origin", o), ("Access-Control-Allow-Origin", o)])).is_err());
    // credentials need an exact origin plus Allow-Credentials: true
    assert!(check_cors_response(o, true, &qf5_h(&[("Access-Control-Allow-Origin", o)])).is_err());
    assert!(check_cors_response(o, true, &qf5_h(&[("Access-Control-Allow-Origin", o), ("Access-Control-Allow-Credentials", "true")])).is_ok());
    // file pages send Origin: null
    assert_eq!(origin_header(&qf5_url("file:///a.html")), "null");
    assert_eq!(origin_header(&qf5_url("https://a.example:8443/x")), "https://a.example:8443");

    let p = RequestPolicy::default();
    let uh = vec!["x-token".to_string(), "content-type".to_string()];
    let good = qf5_h(&[
        ("Access-Control-Allow-Origin", o),
        ("Access-Control-Allow-Methods", "GET, PUT"),
        ("Access-Control-Allow-Headers", "X-Token, Content-Type"),
        ("Access-Control-Max-Age", "99999"),
    ]);
    assert_eq!(p.check_preflight_response(o, false, "PUT", &uh, 204, &good).unwrap().max_age_secs, p.preflight_max_age_cap_secs);
    assert!(p.check_preflight_response(o, false, "DELETE", &uh, 204, &good).is_err());
    assert!(p.check_preflight_response(o, false, "PUT", &["x-other".to_string()], 204, &good).is_err());
    assert!(p.check_preflight_response(o, false, "PUT", &uh, 404, &good).is_err());
    let star = qf5_h(&[("Access-Control-Allow-Origin", "*"), ("Access-Control-Allow-Methods", "*"), ("Access-Control-Allow-Headers", "*")]);
    let r = p.check_preflight_response(o, false, "PATCH", &uh, 200, &star).unwrap();
    assert_eq!(r.max_age_secs, 5);
    assert!(p.check_preflight_response(o, false, "PATCH", &["authorization".to_string()], 200, &star).is_err());
    assert!(p.check_preflight_response(o, true, "PATCH", &uh, 200, &star).is_err());
}

#[test]
fn qf5_forbidden_header_lists_match_js() {
    let js = include_str!("../fetch_prelude.js");
    let start = js.find("var FORBIDDEN_REQ = [").unwrap() + "var FORBIDDEN_REQ = [".len();
    let end = start + js[start..].find(']').unwrap();
    let from_js: Vec<String> = js[start..end].split(',').map(|s| s.trim().trim_matches('\'').to_string()).collect();
    let from_rust: Vec<String> = FORBIDDEN_REQUEST_HEADERS.iter().map(|s| s.to_string()).collect();
    assert_eq!(from_js, from_rust);
    assert!(is_forbidden_request_header("Proxy-Authorization") && is_forbidden_request_header("Sec-Fetch-Mode"));
    assert!(is_forbidden_request_header("COOKIE") && !is_forbidden_request_header("X-Token"));
    assert!(is_forbidden_response_header("Set-Cookie") && is_forbidden_response_header("set-cookie2") && !is_forbidden_response_header("etag"));
    assert!(is_wire_reserved_header("X-Wire-Policy") && !is_wire_reserved_header("X-Token"));
}

#[test]
fn qf5_beacon_classification_and_log() {
    assert_eq!(classify_beacon(NetApi::SendBeacon, false, None), Some(BeaconKind::SendBeacon));
    assert_eq!(classify_beacon(NetApi::Fetch, true, Some(FetchKind::Basic)), Some(BeaconKind::Keepalive));
    assert_eq!(classify_beacon(NetApi::Fetch, false, Some(FetchKind::Opaque)), Some(BeaconKind::NoCorsOpaque));
    assert_eq!(classify_beacon(NetApi::Fetch, false, Some(FetchKind::Cors { preflight: false })), None);
    assert_eq!(classify_beacon(NetApi::Xhr, false, None), None);
    let p = RequestPolicy::default();
    assert!(!p.beacons_send && p.suppress_beacon(Some(BeaconKind::Keepalive)) && !p.suppress_beacon(None));
    let body = "x".repeat(300);
    let line = format_beacon_log(
        BeaconKind::SendBeacon,
        "POST",
        &qf5_url("https://ophan.example/img?a=1"),
        Some("text/plain"),
        body.as_bytes(),
        &qf5_url("https://page.example/"),
        false,
    );
    assert!(line.contains("navigator.sendBeacon POST https://ophan.example/img?a=1 content-type=text/plain body=300 bytes"), "{line}");
    assert!(line.contains(&format!("\"{}\"", "x".repeat(200))) && !line.contains(&"x".repeat(201)), "{line}");
    assert!(line.ends_with("page=https://page.example/: suppressed (not sent)"), "{line}");
    let bin = format_beacon_log(BeaconKind::Keepalive, "POST", &qf5_url("https://a.example/"), Some("image/png"), &[1, 2, 3], &qf5_url("https://p.example/"), false);
    assert!(bin.contains("body=3 bytes page=") && !bin.contains("body[.."), "{bin}");
}

#[test]
fn qf5_policy_defaults_are_the_user_numbers() {
    let p = RequestPolicy::default();
    assert_eq!(p.max_response_bytes, 16 << 20);
    assert_eq!(p.max_in_flight_bytes, 64 << 20);
    assert_eq!((p.max_concurrent_requests, p.max_requests_per_page), (6, 1000));
    assert_eq!(p.read_timeout, std::time::Duration::from_secs(30));
    assert_eq!(p.sync_xhr_timeout_ms, 5000);
    assert_eq!(p.redirect_limit, 5);
    let h = p.wire_cap_headers(Some(5000));
    assert!(h.contains(&("X-Wire-Max-Bytes".to_string(), (16usize << 20).to_string())));
    assert!(h.contains(&("X-Wire-Timeout-Ms".to_string(), "5000".to_string())));
    assert!(h.contains(&("X-Wire-Policy".to_string(), "public-only".to_string())));
    assert!(p.wire_cap_headers(None).contains(&("X-Wire-Timeout-Ms".to_string(), "30000".to_string())));
}

#[test]
fn qf5_codec_build_and_parse() {
    let url = qf5_url("https://example.org:8443/p?q=1");
    let req = build_wire_request(
        "POST",
        &url,
        &qf5_h(&[("Content-Type", "application/json"), ("Host", "evil"), ("Content-Length", "99"), ("Connection", "keep-alive"), ("X-A", "1")]),
        b"{\"a\":1}",
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(req).unwrap(),
        "POST https://example.org:8443/p?q=1 HTTP/1.1\r\nHost: example.org:8443\r\nContent-Type: application/json\r\nX-A: 1\r\nContent-Length: 7\r\n\r\n{\"a\":1}"
    );
    let get = String::from_utf8(build_wire_request("GET", &url, &[], b"").unwrap()).unwrap();
    assert!(!get.contains("Content-Length"));
    let put = String::from_utf8(build_wire_request("PUT", &url, &[], b"").unwrap()).unwrap();
    assert!(put.contains("Content-Length: 0\r\n"));
    // CR/LF injection and bad names are refused
    assert!(build_wire_request("GET", &url, &qf5_h(&[("X-A", "1\r\nX-B: 2")]), b"").is_err());
    assert!(build_wire_request("GET", &url, &qf5_h(&[("X-A", "1\nX")]), b"").is_err());
    assert!(build_wire_request("GET", &url, &qf5_h(&[("X A", "1")]), b"").is_err());
    assert!(build_wire_request("GE T", &url, &[], b"").is_err());

    let raw = b"HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nX-Wire-Final-Url: https://x.test/final\r\n\r\nhello\r\n\r\nworld";
    let r = parse_wire_response_full(raw).unwrap();
    assert_eq!((r.status, r.status_text.as_str()), (201, "Created"));
    assert_eq!(r.body, b"hello\r\n\r\nworld");
    assert_eq!(r.header("Content-Type"), Some("text/plain"));
    let cookies: Vec<&str> = r.headers.iter().filter(|(n, _)| n == "set-cookie").map(|(_, v)| v.as_str()).collect();
    assert_eq!(cookies, ["a=1", "b=2"]);
    assert_eq!(r.final_url.as_deref(), Some("https://x.test/final"));
    assert_eq!(r.error, None);
    // today's synthesized 502 and the QF6 marker
    let e = parse_wire_response_full(b"HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\n\r\nwire: upstream request failed: refused").unwrap();
    assert_eq!(e.error.as_deref(), Some("wire: upstream request failed: refused"));
    let e2 = parse_wire_response_full(b"HTTP/1.1 502 Bad Gateway\r\nX-Wire-Error: 1\r\n\r\nblocked: x").unwrap();
    assert_eq!(e2.error.as_deref(), Some("blocked: x"));
    let real502 = parse_wire_response_full(b"HTTP/1.1 502 Bad Gateway\r\n\r\nnginx says no").unwrap();
    assert_eq!(real502.error, None);
    assert!(parse_wire_response_full(b"HTTP/1.1 200 OK\r\nno terminator").is_err());
    assert!(parse_wire_response_full(b"garbage\r\n\r\n").is_err());
    assert!(parse_wire_response_full(b"HTTP/1.1 999 X\r\n\r\n").is_err());
    assert_eq!(parse_wire_response_full(b"HTTP/1.1 204\r\n\r\n").unwrap().status_text, "");
}

/// In-memory fake wire: a thread on a temp unix socket. `respond` gets the
/// full request bytes and writes the answer; the received request is sent back.
fn qf5_fake_wire(
    tag: &str,
    respond: impl FnOnce(&[u8], &mut UnixStream) + Send + 'static,
) -> (PathBuf, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<()>) {
    let dir = std::env::temp_dir().join(format!("scratchy-qf5-{}-{tag}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("wire.sock");
    let _ = fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let (tx, rx) = mpsc::channel();
    let h = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = s.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..p]).to_ascii_lowercase();
                let cl = head.split("content-length:").nth(1).and_then(|v| v.lines().next()).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                if buf.len() >= p + 4 + cl {
                    break;
                }
            }
        }
        let _ = tx.send(buf.clone());
        respond(&buf, &mut s);
    });
    (path, rx, h)
}

#[test]
fn qf5_fake_wire_round_trip() {
    let (path, rx, h) = qf5_fake_wire("rt", |_req, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: k=v\r\n\r\n{\"ok\":true}");
    });
    let url = qf5_url("https://example.org/api");
    let mut headers = qf5_h(&[("Content-Type", "application/json")]);
    headers.extend(RequestPolicy::default().wire_cap_headers(None));
    let req = build_wire_request("POST", &url, &headers, b"{\"a\":1}").unwrap();
    let resp = fetch_full_via_wire(&path, &req, &WireLimits::default()).unwrap();
    h.join().unwrap();
    let got = rx.recv().unwrap();
    assert_eq!(got, req);
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"{\"ok\":true}");
    assert_eq!(resp.header("set-cookie"), Some("k=v"));
}

#[test]
fn qf5_fake_wire_size_cap() {
    let (path, _rx, h) = qf5_fake_wire("cap", |_req, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
        let big = vec![b'a'; 1 << 20];
        for _ in 0..4 {
            if s.write_all(&big).is_err() {
                break;
            }
        }
    });
    let req = build_wire_request("GET", &qf5_url("https://example.org/"), &[], b"").unwrap();
    let mut limits = WireLimits::default();
    limits.max_body_bytes = 1 << 20;
    assert_eq!(fetch_full_via_wire(&path, &req, &limits), Err(NetError::TooLarge));
    h.join().unwrap();
    // exactly at the cap is fine
    let (path, _rx, h) = qf5_fake_wire("cap2", |_req, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
        let _ = s.write_all(&vec![b'a'; 1000]);
    });
    limits.max_body_bytes = 1000;
    assert_eq!(fetch_full_via_wire(&path, &req, &limits).unwrap().body.len(), 1000);
    h.join().unwrap();
}

#[test]
fn qf5_fake_wire_timeouts_and_connect_errors() {
    let (path, _rx, h) = qf5_fake_wire("to", |_req, s| {
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\npartial");
        thread::sleep(std::time::Duration::from_millis(1500));
    });
    let req = build_wire_request("GET", &qf5_url("https://example.org/"), &[], b"").unwrap();
    let mut limits = WireLimits::default();
    limits.read_timeout = std::time::Duration::from_millis(300);
    let t = std::time::Instant::now();
    assert_eq!(fetch_full_via_wire(&path, &req, &limits), Err(NetError::Timeout));
    assert!(t.elapsed() < std::time::Duration::from_millis(1400));
    h.join().unwrap();
    // the legacy caller now has a read timeout too
    let (path, _rx, h) = qf5_fake_wire("to2", |_req, _s| thread::sleep(std::time::Duration::from_millis(1200)));
    let t = std::time::Instant::now();
    let r = fetch_raw_via_wire_with_timeout(&path, "GET http://x.test/ HTTP/1.1\r\nHost: x.test\r\n\r\n", std::time::Duration::from_millis(250));
    assert!(matches!(r, WireFetchRaw::Unreachable));
    assert!(t.elapsed() < std::time::Duration::from_millis(1100));
    h.join().unwrap();
    // nothing listening
    let missing = std::env::temp_dir().join(format!("scratchy-qf5-{}-none.sock", std::process::id()));
    assert!(matches!(fetch_full_via_wire(&missing, &req, &WireLimits::default()), Err(NetError::Connect(_))));
}

#[test]
fn qf14b_site_of_is_scheme_plus_registrable_domain() {
    let site = |s: &str| site_of(&Url::parse(s).unwrap());
    assert_eq!(site("https://www.example.com/a?b").as_deref(), Some("https://example.com"));
    assert_eq!(site("https://news.bbc.co.uk/").as_deref(), Some("https://bbc.co.uk"));
    assert_eq!(site("http://127.0.0.1:8080/x").as_deref(), Some("http://127.0.0.1"));
    assert_eq!(site("http://localhost:1/").as_deref(), Some("http://localhost"));
    assert_eq!(site("https://a.b.github.io/").as_deref(), Some("https://b.github.io"));
    assert_eq!(site("http://example.com/").as_deref(), Some("http://example.com"), "scheme is part of the site");
    assert_eq!(site("file:///tmp/x.html"), None, "file: has no cookie origin");
    assert_eq!(site("data:text/plain,hi"), None);
}

#[test]
fn qf14b_cookie_headers_follow_the_fetch_spec_and_the_navigation_kind() {
    let sup = CookieSupport::new(true);
    let hs = |ctx: &CookieContext| wire_cookie_headers_with(&sup, ctx);
    let has = |h: &[(String, String)], n: &str, v: &str| h.iter().any(|(a, b)| a == n && b == v);
    let top = Url::parse("https://www.top.example/p").unwrap();
    // Sub-resource: jar + the page's site, even for a third party.
    let h = hs(&CookieContext::subresource(&top));
    assert!(has(&h, "X-Wire-Jar", "scratchy") && has(&h, "X-Wire-Top-Site", "https://top.example"), "{h:?}");
    assert!(!h.iter().any(|(n, _)| n == "X-Wire-Nav"));
    // Navigation: Nav + (page-initiated) initiator; top site = destination.
    let dest = Url::parse("https://shop.other.test/x").unwrap();
    let h = hs(&CookieContext::navigation(&dest, Some(&top)));
    assert!(has(&h, "X-Wire-Top-Site", "https://other.test") && has(&h, "X-Wire-Nav", "1"), "{h:?}");
    assert!(has(&h, "X-Wire-Initiator-Site", "https://top.example"), "{h:?}");
    let h = hs(&CookieContext::navigation(&dest, None));
    assert!(has(&h, "X-Wire-Nav", "1") && !h.iter().any(|(n, _)| n == "X-Wire-Initiator-Site"), "browser-initiated: no initiator {h:?}");
    // Fetch credentials modes.
    let f = |c: CredentialsMode, same: bool| hs(&CookieContext::fetch(&top, c, same));
    let off = vec![("X-Wire-Cookies".to_string(), "off".to_string())];
    assert_eq!(f(CredentialsMode::Omit, true), off);
    assert_eq!(f(CredentialsMode::SameOrigin, false), off, "cross-origin same-origin mode: no credentials");
    assert!(has(&f(CredentialsMode::SameOrigin, true), "X-Wire-Jar", "scratchy"));
    assert!(has(&f(CredentialsMode::Include, false), "X-Wire-Jar", "scratchy"));
    assert_eq!(parse_credentials("omit"), CredentialsMode::Omit);
    assert_eq!(parse_credentials(""), CredentialsMode::SameOrigin);
    // No cookie origin (file: page): nothing at all.
    let file = Url::parse("file:///tmp/p.html").unwrap();
    assert!(hs(&CookieContext::subresource(&file)).is_empty());
    assert!(hs(&CookieContext::fetch(&file, CredentialsMode::Include, true)).is_empty());
}

#[test]
fn qf14b_old_wire_is_detected_once_and_stops_the_jar_headers() {
    let top = Url::parse("https://top.example/").unwrap();
    let ctx = CookieContext::subresource(&top);
    // A pre-cookie wire: no X-Wire-Version.
    let sup = CookieSupport::new(true);
    assert!(!wire_cookie_headers_with(&sup, &ctx).is_empty(), "optimistic until proven otherwise");
    assert_eq!(sup.observe(None, None).as_deref(), Some("scratchy: cookies unavailable: wire is too old (restart it)"));
    assert_eq!(sup.observe(None, None), None, "logged ONCE");
    assert_eq!(sup.observe(Some("2"), None), None);
    assert!(wire_cookie_headers_with(&sup, &ctx).is_empty(), "no jar header to a wire that cannot use it");
    // --no-cookies.
    let sup = CookieSupport::new(true);
    assert_eq!(sup.observe(Some("3"), Some("off")).as_deref(), Some("scratchy: cookies unavailable: wire has cookies disabled"));
    assert!(!sup.usable());
    // A good wire.
    let sup = CookieSupport::new(true);
    assert_eq!(sup.observe(Some("3"), Some("on")), None);
    assert!(sup.usable());
    // Over a real socket: an old fake wire (no version) switches it off.
    let path = unique_test_socket_path("qf14b-old");
    let _ = std::fs::remove_file(&path);
    spawn_mock_wire_once(&path, b"HTTP/1.1 200 OK\r\nContent-Type: text/css\r\n\r\np{}");
    let sup = CookieSupport::new(true);
    let req = build_get_request_ctx(&Url::parse("https://cdn.test/a.css").unwrap(), &CookieContext::subresource(&top));
    assert!(req.contains("X-Wire-Jar: scratchy"));
    let _ = fetch_raw_via_wire_with_support(&path, &req, std::time::Duration::from_secs(5), &sup);
    assert!(!sup.usable(), "learned from the first response");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn qf14b_cookie_headers_are_built_in_one_place() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if name == "net_policy.rs" || name == "tests.rs" || !name.ends_with(".rs") {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap().to_ascii_lowercase();
        for lit in ["x-wire-jar", "x-wire-top-site", "x-wire-nav", "x-wire-initiator-site", "x-wire-cookie-op", "x-wire-cookie-url"] {
            // Only comments may name them (those lines start with `//`).
            let code_use = text.lines().any(|l| l.contains(lit) && !l.trim_start().starts_with("//") && !l.contains("`"));
            assert!(!code_use, "{name} builds a cookie header ({lit}) outside net_policy.rs");
        }
        checked += 1;
    }
    assert!(checked > 5);
}

#[test]
fn qf14b_every_loader_sends_the_jar_headers_with_the_pages_top_site() {
    set_js_enabled(true);
    let path = unique_test_socket_path("qf14b-loaders");
    let seen = qf14b_fake_wire(&path);
    let top = Url::parse("https://www.top.example/page").unwrap();
    let third = |p: &str| Url::parse(&format!("https://cdn.third.test/{p}")).unwrap();
    assert_eq!(fetch_stylesheet_via_wire(&path, &third("a.css"), &top).as_deref(), Some("p{margin:0}"));
    assert_eq!(fetch_script_via_wire(&path, &third("b.js"), &top).as_deref(), Some("var a = 1;"));
    let _ = fetch_image_via_wire(&path, &third("c.png"), &top);
    assert_eq!(fetch_stylesheets(&path, &top, &top, &["https://cdn.third.test/d.css".to_string()]), "p{margin:0}\n");
    let scripts = fetch_scripts(&path, &top, &top, vec![ScriptRef::ModuleExternal("https://cdn.third.test/e.js".into())]);
    assert_eq!(scripts.len(), 1);
    let doc = parse_html("<html><body><iframe src='https://www.top.example/f.html'></iframe></body></html>");
    prefetch_iframes(&path, &top, &top, &doc);
    let reqs = qf14b_wait(&seen, 6);
    assert_eq!(reqs.len(), 6, "{reqs:?}");
    for r in &reqs {
        assert!(qf14b_has(r, "X-Wire-Jar: scratchy"), "no jar header: {r}");
        assert!(qf14b_has(r, "X-Wire-Top-Site: https://top.example"), "wrong partition key: {r}");
        assert!(!r.to_ascii_lowercase().contains("x-wire-nav"), "sub-resources are not navigations: {r}");
    }
    // The same third party under ANOTHER top site gets another partition key.
    let top2 = Url::parse("https://other.example/").unwrap();
    let _ = fetch_stylesheet_via_wire(&path, &third("a.css"), &top2);
    let reqs = qf14b_wait(&seen, 7);
    assert!(qf14b_has(reqs.last().unwrap(), "X-Wire-Top-Site: https://other.example"));
    // A file: page has no cookie origin: no jar headers.
    let file = Url::parse("file:///tmp/p.html").unwrap();
    let _ = fetch_stylesheet_via_wire(&path, &third("a.css"), &file);
    let reqs = qf14b_wait(&seen, 8);
    assert!(!reqs.last().unwrap().to_ascii_lowercase().contains("x-wire-"), "{:?}", reqs.last());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn qf14b_link_navigation_sends_nav_and_initiator_but_typed_go_does_not() {
    let dest = Url::parse("https://www.shop.example/cart").unwrap();
    let from = Url::parse("https://news.example/").unwrap();
    let click = build_get_request_ctx(&dest, &CookieContext::navigation(&dest, Some(&from)));
    assert!(click.starts_with("GET https://www.shop.example/cart HTTP/1.1\r\nHost: www.shop.example\r\n"), "{click}");
    for h in ["X-Wire-Jar: scratchy", "X-Wire-Top-Site: https://shop.example", "X-Wire-Nav: 1", "X-Wire-Initiator-Site: https://news.example"] {
        assert!(qf14b_has(&click, h), "{h} missing: {click}");
    }
    assert!(click.ends_with("\r\n\r\n"));
    let typed = build_get_request_ctx(&dest, &CookieContext::navigation(&dest, None));
    assert!(qf14b_has(&typed, "X-Wire-Nav: 1") && !typed.contains("Initiator"), "{typed}");
}

/// A canned wire for the control channel: keeps a tiny jar (one HttpOnly
/// cookie the page must never see) and records requests.
struct JarFake {
    cookies: Mutex<Vec<(String, String, bool)>>,
    raw: Mutex<Vec<String>>,
    timeouts: Mutex<Vec<std::time::Duration>>,
    version: Option<&'static str>,
}

impl JarFake {
    fn new(version: Option<&'static str>) -> Arc<Self> {
        Arc::new(JarFake {
            cookies: Mutex::new(vec![("sid".into(), "SECRET".into(), true), ("seen".into(), "1".into(), false)]),
            raw: Mutex::new(Vec::new()),
            timeouts: Mutex::new(Vec::new()),
            version,
        })
    }
}

impl NetExecutor for JarFake {
    fn execute(&self, job: &crate::js_net::NetJob) -> Result<WireResponse, NetError> {
        let text = String::from_utf8_lossy(&job.request).to_string();
        self.raw.lock().unwrap().push(text.clone());
        self.timeouts.lock().unwrap().push(job.limits.read_timeout);
        let mut headers = vec![];
        if let Some(v) = self.version {
            headers.push(("x-wire-version".to_string(), v.to_string()));
            headers.push(("x-wire-cookies".to_string(), "on".to_string()));
        }
        let mk = |status: u16, body: String| Ok(WireResponse { status, headers: headers.clone(), body: body.into_bytes(), ..Default::default() });
        if text.contains("X-Wire-Cookie-Op: get") {
            let v: Vec<String> = self.cookies.lock().unwrap().iter().filter(|c| !c.2).map(|c| format!("{}={}", c.0, c.1)).collect();
            return mk(200, v.join("; "));
        }
        if text.contains("X-Wire-Cookie-Op: set") {
            let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
            if !body.to_ascii_lowercase().contains("httponly") {
                if let Some((n, v)) = body.split(';').next().unwrap_or("").split_once('=') {
                    self.cookies.lock().unwrap().push((n.trim().to_string(), v.trim().to_string(), false));
                }
            }
            return mk(204, String::new());
        }
        mk(200, "ok".into())
    }
}

fn qf14b_engine(page: &str, fake: &Arc<JarFake>) -> (JsEngine, Rc<RefCell<Vec<String>>>) {
    set_page_url(Some(&Url::parse(page).unwrap()));
    crate::js_net::set_net_executor(fake.clone());
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = crate::js::start_scripts(&parse_html(""), &[src("")], Rc::new(move |l, t| o.borrow_mut().push(format!("{l}:{t}")))).unwrap();
    (engine, out)
}

#[test]
fn qf14b_document_cookie_round_trips_through_the_control_channel_and_hides_httponly() {
    let fake = JarFake::new(Some("3"));
    let (engine, log) = qf14b_engine("https://www.page.test/dir/x", &fake);
    engine.eval(
        "t",
        "var d = Object.getOwnPropertyDescriptor(Document.prototype, 'cookie');\
         console.log('proto', typeof d.get, typeof d.set, document.hasOwnProperty('cookie'));\
         console.log('before', document.cookie);\
         document.cookie = 'a=1; path=/';\
         console.log('after', document.cookie);\
         var second = document.implementation.createHTMLDocument('x');\
         second.cookie = 'zzz=9';\
         console.log('second', JSON.stringify(second.cookie));",
    );
    let l = log.borrow().clone();
    assert!(l.contains(&"log:proto function function false".to_string()), "{l:?}");
    assert!(l.contains(&"log:before seen=1".to_string()), "HttpOnly sid never reaches the page: {l:?}");
    assert!(l.contains(&"log:after seen=1; a=1".to_string()), "{l:?}");
    assert!(l.contains(&"log:second \"\"".to_string()), "second documents are inert: {l:?}");
    let raw = fake.raw.lock().unwrap().clone();
    assert_eq!(raw.len(), 3, "get, set, get; the second document made no request: {raw:?}");
    let set = &raw[1];
    for h in ["X-Wire-Jar: scratchy", "X-Wire-Top-Site: https://page.test", "X-Wire-Cookie-Op: set", "X-Wire-Cookie-Url: https://www.page.test/dir/x"] {
        assert!(qf14b_has(set, h), "{h}: {set}");
    }
    assert!(set.ends_with("\r\n\r\na=1; path=/"), "the assigned string is the body: {set}");
    assert!(fake.timeouts.lock().unwrap().iter().all(|t| *t == std::time::Duration::from_millis(2000)), "cookie_control_timeout_ms");
    assert!(!l.iter().any(|x| x.starts_with("error:")), "{l:?}");
}

#[test]
fn qf14b_document_cookie_is_inert_on_file_pages() {
    let fake = JarFake::new(Some("3"));
    let (engine, log) = qf14b_engine("file:///tmp/p.html", &fake);
    engine.eval("t", "document.cookie = 'a=1'; console.log(JSON.stringify(document.cookie));");
    assert!(log.borrow().contains(&"log:\"\"".to_string()), "{:?}", log.borrow());
    assert!(fake.raw.lock().unwrap().is_empty(), "no control request without a cookie origin");
}

#[test]
fn qf14b_document_cookie_failure_logs_once_and_reads_empty() {
    let fake = QfFake::new(vec![]); // every request fails
    let (engine, log) = qf7_engine("https://page.test/", &fake);
    engine.eval("t", "document.cookie = 'a=1'; console.log(JSON.stringify(document.cookie)); console.log(JSON.stringify(document.cookie));");
    let l = log.borrow().clone();
    assert_eq!(l.iter().filter(|x| x.starts_with("net:document.cookie:")).count(), 1, "{l:?}");
    assert_eq!(l.iter().filter(|x| *x == "log:\"\"").count(), 2, "{l:?}");
    assert_eq!(fake.urls().len(), 1, "after a failure the engine thread never blocks on wire again");
}

#[test]
fn qf14b_old_wire_for_document_cookie_logs_once_and_sends_no_more_jar_headers() {
    let fake = JarFake::new(None); // no X-Wire-Version: pre-cookie wire
    let (engine, log) = qf14b_engine("https://page.test/", &fake);
    engine.set_cookie_support(Box::leak(Box::new(CookieSupport::new(true))));
    engine.eval(
        "t",
        "document.cookie; document.cookie = 'a=1'; document.cookie;\
         __net_request({method:'GET', url:'https://page.test/x', headers:[], body:null});",
    );
    qf14b_drive(&engine, &fake);
    let l = log.borrow().clone();
    let warn = "net-warning:scratchy: cookies unavailable: wire is too old (restart it)";
    assert_eq!(l.iter().filter(|x| *x == warn).count(), 1, "{l:?}");
    let raw = fake.raw.lock().unwrap().clone();
    assert_eq!(raw.len(), 2, "one control probe, then the plain request: {raw:?}");
    assert!(!raw[1].to_ascii_lowercase().contains("x-wire-jar"), "{}", raw[1]);
}

fn qf14b_drive(engine: &JsEngine, fake: &Arc<JarFake>) {
    let mut sched = NetScheduler::new(&engine.net_policy());
    crate::js_net::drive_report_net(engine, &mut sched, &**fake);
}

#[test]
fn qf14b_fetch_and_xhr_credentials_decide_whether_the_jar_is_used() {
    let ok = || qf7_ok("x", &[("access-control-allow-origin", "https://page.test"), ("access-control-allow-credentials", "true")]);
    let fake = QfFake::new(vec![("/same", ok()), ("/cross", ok())]);
    let (engine, _log) = qf7_engine("https://page.test/", &fake);
    let go = |url: &str, cred: &str| qf7_go(url, &format!(",credentials:'{cred}'"));
    engine.eval(
        "t",
        &[
            go("https://page.test/same?default", "same-origin"),
            go("https://page.test/same?omit", "omit"),
            go("https://page.test/same?include", "include"),
            go("https://other.test/cross?default", "same-origin"),
            go("https://other.test/cross?omit", "omit"),
            go("https://other.test/cross?include", "include"),
        ]
        .concat(),
    );
    qf7_drive(&engine, &fake);
    let raw: Vec<String> = fake.raw.lock().unwrap().iter().map(|r| String::from_utf8_lossy(r).to_string()).collect();
    let find = |needle: &str| raw.iter().find(|r| r.contains(needle)).unwrap_or_else(|| panic!("{needle} not sent: {raw:?}")).clone();
    let jar = |r: &str| qf14b_has(r, "X-Wire-Jar: scratchy") && qf14b_has(r, "X-Wire-Top-Site: https://page.test");
    let off = |r: &str| qf14b_has(r, "X-Wire-Cookies: off") && !r.to_ascii_lowercase().contains("x-wire-jar");
    assert!(jar(&find("same?default")));
    assert!(off(&find("same?omit")), "omit sends no jar and says off");
    assert!(jar(&find("same?include")));
    assert!(off(&find("cross?default")), "cross-origin with the default same-origin mode: no credentials");
    assert!(off(&find("cross?omit")));
    assert!(jar(&find("cross?include")));
    let e = engine.net_entries();
    assert_eq!(e.iter().map(|x| x.jar).collect::<Vec<_>>(), vec![true, false, true, false, false, true]);
    assert!(raw.iter().all(|r| !r.to_ascii_lowercase().contains("\r\ncookie:")), "page JS never sets Cookie itself");
    let report = crate::js_net::format_net_report(&e);
    assert!(report.contains("jar: scratchy") && report.contains("jar: off"), "{report}");
}

#[test]
fn qf14b_xhr_with_credentials_maps_to_include() {
    let fake = QfFake::new(vec![("/x", qf7_ok("x", &[("access-control-allow-origin", "https://page.test"), ("access-control-allow-credentials", "true")]))]);
    let (engine, _log) = qf7_engine("https://page.test/", &fake);
    engine.eval(
        "t",
        "var a = new XMLHttpRequest(); a.open('GET', 'https://other.test/x?plain'); a.send();\
         var b = new XMLHttpRequest(); b.open('GET', 'https://other.test/x?creds'); b.withCredentials = true; b.send();",
    );
    qf7_drive(&engine, &fake);
    let raw: Vec<String> = fake.raw.lock().unwrap().iter().map(|r| String::from_utf8_lossy(r).to_string()).collect();
    let plain = raw.iter().find(|r| r.contains("?plain")).unwrap();
    let creds = raw.iter().find(|r| r.contains("?creds")).unwrap();
    assert!(!plain.to_ascii_lowercase().contains("x-wire-jar"), "cross-origin XHR without withCredentials: no jar");
    assert!(qf14b_has(creds, "X-Wire-Jar: scratchy"));
}

#[test]
fn qf14b_js_report_has_a_cookie_smoke_line() {
    let fake = QfFake::new(vec![("page.test/data", qf7_ok("12345", &[]))]);
    crate::js_net::set_net_executor(fake.clone());
    let html = "<html><head><base href='https://page.test/'></head><body><script>\
        __net_request({method:'GET', url:'https://page.test/data', headers:[], body:null});\
        </script></body></html>";
    set_js_enabled(true);
    let report = js::js_report_for_html(html, None, "/tmp/qf14b-none.html");
    assert!(report.contains("GET https://page.test/data -> 200 OK 5 bytes"), "{report}");
    assert!(report.contains("jar: scratchy"), "{report}");
    assert!(report.contains("--- cookies ---\ndocument.cookie: "), "{report}");
    assert!(report.contains("errors total: 0"), "{report}");
}


// ---------- QF13: all methods with bodies, CORS preflight + cache ----------

type PfHandler = Box<dyn Fn(&str, &str, &str) -> Result<WireResponse, NetError> + Send + Sync>;

/// Method-aware fake wire: records every raw request (in order) with its
/// read timeout; `handler(method, url, raw)` answers.
struct PfFake {
    seen: Mutex<Vec<(String, String, String, std::time::Duration)>>,
    handler: PfHandler,
}

impl PfFake {
    fn new(handler: impl Fn(&str, &str, &str) -> Result<WireResponse, NetError> + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(PfFake { seen: Mutex::new(Vec::new()), handler: Box::new(handler) })
    }
    /// `METHOD url` per request, in order.
    fn order(&self) -> Vec<String> {
        self.seen.lock().unwrap().iter().map(|(m, u, _, _)| format!("{m} {u}")).collect()
    }
    fn raw(&self, i: usize) -> String {
        self.seen.lock().unwrap()[i].2.clone()
    }
}

impl NetExecutor for PfFake {
    fn execute(&self, job: &crate::js_net::NetJob) -> Result<WireResponse, NetError> {
        let raw = String::from_utf8_lossy(&job.request).to_string();
        let mut it = raw.lines().next().unwrap_or("").split(' ');
        let (m, u) = (it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string());
        self.seen.lock().unwrap().push((m.clone(), u.clone(), raw.clone(), job.limits.read_timeout));
        (self.handler)(&m, &u, &raw)
    }
}

fn pf_resp(status: u16, headers: &[(&str, &str)], body: &str) -> Result<WireResponse, NetError> {
    Ok(WireResponse {
        status,
        status_text: if status == 204 { "No Content".into() } else { "OK".into() },
        headers: headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        body: body.as_bytes().to_vec(),
        error: None,
        final_url: None,
    })
}

/// A cooperative CORS server for `https://page.test`: the preflight carries
/// `pre` (extra/override headers), everything else answers 200 with ACAO.
fn pf_server(pre: &'static [(&'static str, &'static str)]) -> impl Fn(&str, &str, &str) -> Result<WireResponse, NetError> + Send + Sync + 'static {
    move |m, _u, _raw| {
        if m == "OPTIONS" {
            pf_resp(204, pre, "")
        } else {
            pf_resp(200, &[("access-control-allow-origin", "https://page.test"), ("content-type", "text/plain")], "done")
        }
    }
}

const PF_ALLOW_ALL: &[(&str, &str)] = &[
    ("access-control-allow-origin", "https://page.test"),
    ("access-control-allow-methods", "POST, PUT, PATCH, DELETE"),
    ("access-control-allow-headers", "content-type, x-foo"),
];

fn pf_engine(page: &str, fake: &Arc<PfFake>, clock: Option<Rc<std::cell::Cell<f64>>>) -> (JsEngine, Rc<RefCell<Vec<String>>>) {
    set_page_url(Some(&Url::parse(page).unwrap()));
    crate::js_net::set_net_executor(fake.clone());
    let out = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let engine = crate::js::start_scripts_clocked(&parse_html(""), &[src("")], Rc::new(move |l, t| o.borrow_mut().push(format!("{l}:{t}"))), clock).unwrap();
    (engine, out)
}

fn pf_drive(engine: &JsEngine, fake: &Arc<PfFake>) {
    let mut sched = NetScheduler::new(&engine.net_policy());
    crate::js_net::drive_report_net(engine, &mut sched, &**fake);
}

const PF_JSON_POST: &str = "fetch('https://api.test/x', {method:'POST', headers:{'Content-Type':'application/json','X-Foo':'1'}, body:'{\"k\":1}'})\
    .then(function(r){ return r.text(); }).then(function(t){ console.log('ok', t); }, function(e){ console.log('err', e.name, e.message); });";

#[test]
fn qf13_preflight_precedes_a_cross_origin_json_post_without_cookies() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", &PF_JSON_POST.replace("headers:", "credentials:'omit', headers:"));
    pf_drive(&engine, &fake);
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/x", "POST https://api.test/x"]);
    let pre = fake.raw(0);
    assert!(pre.contains("Origin: https://page.test\r\n"), "{pre}");
    assert!(pre.contains("Access-Control-Request-Method: POST\r\n"), "{pre}");
    assert!(pre.contains("Access-Control-Request-Headers: content-type,x-foo\r\n"), "lowercased and sorted: {pre}");
    assert!(pre.contains("X-Wire-Cookies: off"), "{pre}");
    assert!(!pre.to_ascii_lowercase().contains("x-wire-jar"), "{pre}");
    assert!(!pre.to_ascii_lowercase().contains("content-length") && pre.ends_with("\r\n\r\n"), "no body: {pre}");
    let act = fake.raw(1);
    assert!(act.contains("{\"k\":1}") && act.contains("Origin: https://page.test"), "{act}");
    let l = log.borrow().clone();
    assert!(l.contains(&"log:ok done".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.starts_with("net:preflight OPTIONS https://api.test/x for POST (Access-Control-Request-Headers: content-type,x-foo)")), "{l:?}");
    let e = engine.net_entries();
    assert!(e.iter().any(|x| x.method == "OPTIONS (preflight)" && x.outcome.starts_with("204")), "{e:?}");
    let report = crate::js_net::format_net_report(&e);
    assert!(report.contains("OPTIONS (preflight) https://api.test/x -> 204"), "{report}");
}


#[test]
fn qf13_preflight_never_sends_the_jar_but_the_credentialed_request_does() {
    let fake = PfFake::new(pf_server(&[
        ("access-control-allow-origin", "https://page.test"),
        ("access-control-allow-credentials", "true"),
        ("access-control-allow-methods", "PUT"),
        ("access-control-allow-headers", "x-foo"),
    ]));
    let (engine, _log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "fetch('https://api.test/x', {method:'PUT', credentials:'include', headers:{'X-Foo':'1'}, body:'b'}).catch(function(e){ console.log('err', e.message); });",
    );
    pf_drive(&engine, &fake);
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/x", "PUT https://api.test/x"], "{:?}", fake.order());
    assert!(!fake.raw(0).to_ascii_lowercase().contains("x-wire-jar"), "{}", fake.raw(0));
    assert!(fake.raw(0).contains("X-Wire-Cookies: off"));
    assert!(fake.raw(1).contains("X-Wire-Jar: scratchy"), "{}", fake.raw(1));
}

fn qf13_denied(pre: &'static [(&'static str, &'static str)], status: u16, code: &str) -> (Vec<String>, Vec<String>) {
    let fake = PfFake::new(move |m, _u, _r| {
        if m == "OPTIONS" {
            pf_resp(status, pre, "")
        } else {
            pf_resp(200, &[("access-control-allow-origin", "https://page.test")], "SECRET")
        }
    });
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", code);
    pf_drive(&engine, &fake);
    let l = log.borrow().clone();
    (fake.order(), l)
}

#[test]
fn qf13_each_missing_piece_denies_the_preflight_and_sends_nothing_else() {
    let cases: Vec<(&'static [(&'static str, &'static str)], u16, &str, &str)> = vec![
        // header missing
        (&[("access-control-allow-origin", "https://page.test"), ("access-control-allow-methods", "POST"), ("access-control-allow-headers", "content-type")], 204, PF_JSON_POST, "Access-Control-Allow-Headers missing 'x-foo'"),
        // method missing
        (&[("access-control-allow-origin", "https://page.test"), ("access-control-allow-methods", "GET"), ("access-control-allow-headers", "content-type, x-foo")], 204, "fetch('https://api.test/x', {method:'PUT', headers:{'X-Foo':'1'}, body:'b'}).catch(function(e){ console.log('err', e.name, e.message); });", "Access-Control-Allow-Methods missing 'PUT'"),
        // origin missing / wrong
        (&[("access-control-allow-methods", "POST"), ("access-control-allow-headers", "content-type, x-foo")], 204, PF_JSON_POST, "no Access-Control-Allow-Origin header"),
        (&[("access-control-allow-origin", "https://evil.test"), ("access-control-allow-methods", "POST"), ("access-control-allow-headers", "content-type, x-foo")], 204, PF_JSON_POST, "does not match the page origin"),
        // bad status
        (&[("access-control-allow-origin", "https://page.test"), ("access-control-allow-methods", "POST"), ("access-control-allow-headers", "content-type, x-foo")], 403, PF_JSON_POST, "preflight response has status 403"),
        // wildcard never covers Authorization
        (&[("access-control-allow-origin", "*"), ("access-control-allow-methods", "*"), ("access-control-allow-headers", "*")], 204, "fetch('https://api.test/x', {headers:{'Authorization':'t'}}).catch(function(e){ console.log('err', e.name, e.message); });", "Access-Control-Allow-Headers missing 'authorization'"),
        // credentials: ACAO * with credentials, and ACAC missing
        (&[("access-control-allow-origin", "*"), ("access-control-allow-methods", "POST"), ("access-control-allow-headers", "x-foo,content-type")], 204, "fetch('https://api.test/x', {method:'POST', credentials:'include', headers:{'X-Foo':'1'}, body:'b'}).catch(function(e){ console.log('err', e.name, e.message); });", "Access-Control-Allow-Origin is * but the request includes credentials"),
        (&[("access-control-allow-origin", "https://page.test"), ("access-control-allow-methods", "POST"), ("access-control-allow-headers", "x-foo")], 204, "fetch('https://api.test/x', {method:'POST', credentials:'include', headers:{'X-Foo':'1'}, body:'b'}).catch(function(e){ console.log('err', e.name, e.message); });", "Access-Control-Allow-Credentials is not true"),
    ];
    for (pre, status, code, what) in cases {
        let (order, l) = qf13_denied(pre, status, code);
        assert_eq!(order, vec!["OPTIONS https://api.test/x"], "only the preflight is sent ({what}): {l:?}");
        assert!(l.iter().any(|x| x.starts_with("net:CORS preflight failed: https://api.test/x: ") && x.contains(what)), "{what}: {l:?}");
        assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{what}: {l:?}");
        assert!(!l.iter().any(|x| x.contains("SECRET")), "{l:?}");
    }
}

#[test]
fn qf13_a_preflight_transport_error_fails_the_request_too() {
    let fake = PfFake::new(|_m, _u, _r| Err(NetError::Io("boom".into())));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/x"]);
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.starts_with("net:CORS preflight failed: https://api.test/x: ") && x.contains("boom")), "{l:?}");
    assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{l:?}");
}

#[test]
fn qf13_cache_hit_skips_the_second_preflight_and_max_age_expires_on_the_clock() {
    let fake = PfFake::new(pf_server(&[
        ("access-control-allow-origin", "https://page.test"),
        ("access-control-allow-methods", "POST"),
        ("access-control-allow-headers", "content-type, x-foo"),
        ("access-control-max-age", "10"),
    ]));
    let clock = Rc::new(std::cell::Cell::new(0.0f64));
    let (engine, log) = pf_engine("https://page.test/", &fake, Some(clock.clone()));
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    clock.set(9_000.0);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 1, "second request used the cache: {:?}", fake.order());
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("POST")).count(), 2);
    assert!(log.borrow().iter().any(|x| x.starts_with("net:preflight cache hit: OPTIONS https://api.test/x for POST")), "{:?}", log.borrow());
    let report = crate::js_net::format_net_report(&engine.net_entries());
    assert!(report.contains("OPTIONS (preflight) https://api.test/x -> cache hit"), "{report}");
    clock.set(10_500.0);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 2, "expired after Max-Age: {:?}", fake.order());
}

#[test]
fn qf13_default_max_age_is_five_seconds_and_the_cap_comes_from_the_policy() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let clock = Rc::new(std::cell::Cell::new(0.0f64));
    let (engine, _log) = pf_engine("https://page.test/", &fake, Some(clock.clone()));
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    clock.set(4_900.0);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 1);
    clock.set(5_100.0);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 2, "default 5 s");
    // a huge Max-Age is capped by the policy
    let fake = PfFake::new(pf_server(&[
        ("access-control-allow-origin", "https://page.test"),
        ("access-control-allow-methods", "POST"),
        ("access-control-allow-headers", "content-type, x-foo"),
        ("access-control-max-age", "999999"),
    ]));
    let clock = Rc::new(std::cell::Cell::new(0.0f64));
    let (engine, _log) = pf_engine("https://page.test/", &fake, Some(clock.clone()));
    let mut p = RequestPolicy::default();
    p.preflight_max_age_cap_secs = 20;
    engine.set_net_policy(p);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    clock.set(21_000.0);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 2, "cap of 20 s applied");
}

#[test]
fn qf13_cache_is_keyed_by_url_method_headers_and_credentials_and_bounded() {
    let fake = PfFake::new(pf_server(&[
        ("access-control-allow-origin", "https://page.test"),
        ("access-control-allow-credentials", "true"),
        ("access-control-allow-methods", "POST, PUT"),
        ("access-control-allow-headers", "content-type, x-foo"),
        ("access-control-max-age", "100"),
    ]));
    let (engine, _log) = pf_engine("https://page.test/", &fake, Some(Rc::new(std::cell::Cell::new(0.0))));
    let go = |url: &str, method: &str, cred: &str| {
        format!("fetch('{url}', {{method:'{method}', credentials:'{cred}', headers:{{'X-Foo':'1'}}, body:'b'}}).catch(function(){{}});")
    };
    engine.eval(
        "t",
        &[
            go("https://api.test/a", "POST", "omit"),
            go("https://api.test/a", "POST", "omit"),
            go("https://api.test/a", "PUT", "omit"),
            go("https://api.test/a", "POST", "include"),
            go("https://api.test/b", "POST", "omit"),
        ]
        .concat(),
    );
    pf_drive(&engine, &fake);
    // The first two POSTs are issued before the first preflight returns, so both
    // preflight; then PUT, include and /b are different keys.
    let pre = fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count();
    assert_eq!(pre, 5, "{:?}", fake.order());
    engine.eval("t", &go("https://api.test/a", "POST", "omit"));
    pf_drive(&engine, &fake);
    assert_eq!(fake.order().iter().filter(|x| x.starts_with("OPTIONS")).count(), 5, "now cached");
    let mut c = crate::js_net::PreflightCache::default();
    let u = Url::parse("https://a.test/").unwrap();
    for i in 0..10 {
        let k = crate::js_net::PreflightCache::key("o", &u, &format!("M{i}"), &[], false);
        c.insert(k, 0.0, 10 + i, 4);
    }
    assert_eq!(c.len(), 4, "bounded by the policy's max entries");
    c.insert("zero".into(), 0.0, 0, 4);
    assert_eq!(c.len(), 4, "Max-Age 0 is not cached");
}

#[test]
fn qf13_put_patch_delete_with_bodies_reach_the_executor_after_their_preflights() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "fetch('https://api.test/r', {method:'PUT', body:'put-body'}).catch(function(e){ console.log('e1', e.message); });\
         fetch('https://api.test/r', {method:'PATCH', body:'patch-body'}).catch(function(e){ console.log('e2', e.message); });\
         fetch('https://api.test/r', {method:'DELETE', body:'del-body'}).catch(function(e){ console.log('e3', e.message); });\
         var x = new XMLHttpRequest(); x.open('PUT', 'https://api.test/x'); x.send('xhr-body');",
    );
    pf_drive(&engine, &fake);
    let seen = fake.seen.lock().unwrap().clone();
    for (m, body) in [("PUT", "put-body"), ("PATCH", "patch-body"), ("DELETE", "del-body")] {
        let (_, _, raw, _) = seen.iter().find(|(mm, u, _, _)| mm == m && u.ends_with("/r")).unwrap_or_else(|| panic!("{m} not sent: {seen:?}"));
        assert!(raw.ends_with(body), "{raw}");
    }
    assert!(seen.iter().any(|(m, u, r, _)| m == "PUT" && u.ends_with("/x") && r.ends_with("xhr-body")), "{seen:?}");
    // every actual request came after an OPTIONS for the same URL+method
    let opts = seen.iter().filter(|s| s.0 == "OPTIONS").count();
    assert!(opts >= 4, "{:?}", fake.order());
    assert!(!log.borrow().iter().any(|x| x.contains("blocked")), "{:?}", log.borrow());
}

#[test]
fn qf13_forbidden_methods_are_still_refused_before_any_preflight() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", "__net_request({method:'TRACE', url:'https://api.test/x', headers:[], body:null}).promise.catch(function(e){ console.log('err', e.name); });");
    pf_drive(&engine, &fake);
    assert!(fake.order().is_empty(), "{:?}", fake.order());
    assert!(log.borrow().iter().any(|x| x.starts_with("net:blocked TRACE")), "{:?}", log.borrow());
}

#[test]
fn qf13_same_origin_and_simple_requests_never_preflight() {
    let fake = PfFake::new(|_m, _u, _r| pf_resp(200, &[("access-control-allow-origin", "https://page.test")], "x"));
    let (engine, _log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "fetch('/same', {method:'PUT', headers:{'Content-Type':'application/json','X-Foo':'1'}, body:'{}'});\
         fetch('https://api.test/simple', {method:'POST', headers:{'Content-Type':'text/plain'}, body:'a'});\
         fetch('https://api.test/get');\
         var x = new XMLHttpRequest(); x.open('POST', 'https://api.test/form'); x.setRequestHeader('Content-Type', 'application/x-www-form-urlencoded'); x.send('a=b');",
    );
    pf_drive(&engine, &fake);
    let o = fake.order();
    assert_eq!(o.len(), 4, "{o:?}");
    assert!(o.iter().all(|x| !x.starts_with("OPTIONS")), "{o:?}");
}

#[test]
fn qf13_the_preflight_counts_toward_the_page_cap_and_the_in_flight_limits() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    let mut p = RequestPolicy::default();
    p.max_requests_per_page = 3;
    engine.set_net_policy(p);
    engine.eval(
        "t",
        "for (var i = 0; i < 3; i++) fetch('https://api.test/c' + i, {method:'PUT', body:'b'}).then(function(){ console.log('ok'); }, function(e){ console.log('err', e.message); });",
    );
    pf_drive(&engine, &fake);
    let o = fake.order();
    assert_eq!(o.len(), 3, "cap of 3 requests counts OPTIONS too: {o:?}");
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.contains("too many requests from this page (limit 3)")), "{l:?}");
    // in flight: a preflight is an ordinary job for the scheduler
    let mut sched = NetScheduler::new(&engine.net_policy());
    let (engine2, _l2) = pf_engine("https://page.test/", &fake, None);
    engine2.eval("t", "fetch('https://api.test/z', {method:'PUT', body:'b'});");
    let reqs = engine2.take_requests();
    assert_eq!(reqs.net.len(), 1, "only the preflight is queued first");
    for j in reqs.net {
        assert_eq!(j.reserve, 0, "a preflight reserves no body bytes");
        sched.enqueue(j);
    }
    assert_eq!(sched.counts().0, 1);
}

#[test]
fn qf13_aborting_during_the_preflight_sends_nothing() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", "var c = new AbortController(); fetch('https://api.test/a', {method:'PUT', body:'b', signal:c.signal}).catch(function(e){ console.log('err', e.name); }); c.abort();");
    pf_drive(&engine, &fake);
    assert!(fake.order().is_empty(), "{:?}", fake.order());
    assert!(log.borrow().contains(&"log:err AbortError".to_string()), "{:?}", log.borrow());
}

#[test]
fn qf13_xhr_fires_error_when_the_preflight_is_denied() {
    let fake = PfFake::new(pf_server(&[("access-control-allow-origin", "https://page.test"), ("access-control-allow-methods", "GET")]));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "var x = new XMLHttpRequest(); x.open('PUT', 'https://api.test/x');\
         ['load','error','loadend'].forEach(function(t){ x.addEventListener(t, function(){ console.log('ev', t, x.status, x.readyState); }); });\
         x.send('b');",
    );
    pf_drive(&engine, &fake);
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/x"]);
    let l = log.borrow().clone();
    assert!(l.contains(&"log:ev error 0 4".to_string()) && !l.contains(&"log:ev load 0 4".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.contains("CORS preflight failed: https://api.test/x: Access-Control-Allow-Methods missing 'PUT'")), "{l:?}");
}

#[test]
fn qf13_sync_xhr_preflights_inside_the_blocking_call_within_the_sync_budget() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "var x = new XMLHttpRequest(); x.open('POST', 'https://api.test/s', false); x.setRequestHeader('X-Foo', '1');\
         x.send('sync-body'); console.log('after', x.status, x.responseText);",
    );
    // no driving at all: everything happened inside send()
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/s", "POST https://api.test/s"]);
    let l = log.borrow().clone();
    assert!(l.contains(&"log:after 200 done".to_string()), "{l:?}");
    assert!(l.iter().any(|x| x.contains("blocking the page for up to 5000 ms (including its preflight)")), "{l:?}");
    let seen = fake.seen.lock().unwrap().clone();
    assert!(seen[0].3 <= std::time::Duration::from_millis(5000) && seen[1].3 <= std::time::Duration::from_millis(5000), "{seen:?}");
    assert!(seen[1].2.contains("X-Wire-Timeout-Ms: "), "{}", seen[1].2);
    // denied: throws NetworkError, only the preflight was sent
    let fake = PfFake::new(pf_server(&[("access-control-allow-origin", "https://page.test")]));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "var x = new XMLHttpRequest(); x.open('PUT', 'https://api.test/s', false);\
         try { x.send('b'); console.log('no throw'); } catch (e) { console.log('thrown', e.name); }",
    );
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/s"]);
    assert!(log.borrow().contains(&"log:thrown NetworkError".to_string()), "{:?}", log.borrow());
}

#[test]
fn qf13_a_redirect_of_a_preflighted_request_fails_closed_and_wire_is_told_not_to_follow() {
    let fake = PfFake::new(|m, _u, _r| {
        if m == "OPTIONS" {
            pf_resp(204, PF_ALLOW_ALL, "")
        } else {
            pf_resp(307, &[("location", "https://evil.test/steal"), ("access-control-allow-origin", "https://page.test")], "")
        }
    });
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert!(fake.raw(0).contains("X-Wire-Max-Redirects: 0") && fake.raw(1).contains("X-Wire-Max-Redirects: 0"));
    let l = log.borrow().clone();
    assert!(l.iter().any(|x| x.contains("of a preflighted request is not followed")), "{l:?}");
    assert!(l.contains(&"log:err TypeError Failed to fetch".to_string()), "{l:?}");
}

#[test]
fn qf13_a_preflight_that_redirects_fails() {
    let fake = PfFake::new(|_m, _u, _r| pf_resp(302, &[("location", "https://other.test/"), ("access-control-allow-origin", "*")], ""));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval("t", PF_JSON_POST);
    pf_drive(&engine, &fake);
    assert_eq!(fake.order(), vec!["OPTIONS https://api.test/x"]);
    assert!(log.borrow().iter().any(|x| x.contains("preflight response has status 302")), "{:?}", log.borrow());
}

#[test]
fn qf13_a_simple_cors_request_redirected_cross_origin_is_checked_against_origin_null() {
    // A GET that wire redirected from api.test to cdn.test: the final answer
    // only allows the page origin, which a browser (Origin: null after the
    // hop) would not accept.
    let mk = |acao: &'static str| {
        PfFake::new(move |_m, _u, _r| {
            Ok(WireResponse {
                status: 200,
                status_text: "OK".into(),
                headers: vec![("access-control-allow-origin".into(), acao.into())],
                body: b"x".to_vec(),
                error: None,
                final_url: Some("https://cdn.test/y".into()),
            })
        })
    };
    for (acao, ok) in [("https://page.test", false), ("null", true), ("*", true)] {
        let fake = mk(acao);
        let (engine, log) = pf_engine("https://page.test/", &fake, None);
        engine.eval("t", "fetch('https://api.test/x').then(function(){ console.log('ok'); }, function(e){ console.log('err', e.message); });");
        pf_drive(&engine, &fake);
        let l = log.borrow().clone();
        assert_eq!(l.contains(&"log:ok".to_string()), ok, "{acao}: {l:?}");
    }
}

#[test]
fn qf13_navigator_send_beacon_exists_logs_and_returns_true() {
    let fake = PfFake::new(pf_server(PF_ALLOW_ALL));
    let (engine, log) = pf_engine("https://page.test/", &fake, None);
    engine.eval(
        "t",
        "console.log('t', typeof navigator.sendBeacon, navigator.sendBeacon('https://track.test/b', 'hi'), (function () { try { navigator.sendBeacon('ftp://x.test/'); return 'no-throw'; } catch (e) { return e.name; } })());",
    );
    pf_drive(&engine, &fake);
    let l = log.borrow().clone();
    assert!(l.contains(&"log:t function true TypeError".to_string()), "{l:?}");
    assert!(fake.order().is_empty(), "beacons are suppressed by default");
    assert!(l.iter().any(|x| x.starts_with("net:beacon navigator.sendBeacon POST https://track.test/b")), "{l:?}");
}

// ---------- QY39b: --user-agent ----------

fn qy39b_args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

#[test]
fn qy39b_flag_parsing_and_env() {
    assert_eq!(user_agent_from_args(&qy39b_args(&["--no-js", "x"])), Ok(None));
    assert_eq!(resolve_user_agent(&qy39b_args(&["--user-agent", "x-test/1.0", "u"]), None), Ok(Some("x-test/1.0".into())));
    // The flag wins over the environment; an empty variable is unset.
    assert_eq!(resolve_user_agent(&qy39b_args(&["--user-agent", "f"]), Some("e".into())), Ok(Some("f".into())));
    assert_eq!(resolve_user_agent(&[], Some("e".into())), Ok(Some("e".into())));
    assert_eq!(resolve_user_agent(&[], Some(String::new())), Ok(None));
    assert!(resolve_user_agent(&qy39b_args(&["--user-agent"]), None).unwrap_err().contains("needs a string"));
}

#[test]
fn qy39b_validation_errors() {
    assert!(validate_user_agent("Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0").is_ok());
    assert!(validate_user_agent("").is_err());
    assert!(validate_user_agent("bad\r\nX: y").is_err());
    assert!(validate_user_agent("caf\u{e9}").is_err());
    assert!(validate_user_agent("tab\there").is_err());
    assert!(validate_user_agent(&"a".repeat(512)).is_ok());
    assert!(validate_user_agent(&"a".repeat(513)).is_err());
    assert!(resolve_user_agent(&qy39b_args(&["--user-agent", "bad\nx"]), None).is_err());
    assert!(resolve_user_agent(&[], Some("\u{7f}".into())).is_err());
}

fn qy39b_count(req: &[u8]) -> usize {
    String::from_utf8_lossy(req).lines().filter(|l| l.to_ascii_lowercase().starts_with("x-wire-user-agent:")).count()
}

#[test]
fn qy39b_header_added_once_per_request_kind_and_never_without_flag() {
    let url = Url::parse("https://page.test/a.png").unwrap();
    let on = UaOverride::new(Some("x-test/1.0".into()));
    let off = UaOverride::new(None);
    // Navigation / sub-resource builder (the ctx builder appends cookie headers to this text).
    let t = build_get_request_text_with(&url, &on);
    assert_eq!(qy39b_count(t.as_bytes()), 1, "{t}");
    assert!(t.contains("X-Wire-User-Agent: x-test/1.0\r\n") && t.ends_with("\r\n\r\n"));
    assert_eq!(qy39b_count(build_get_request_text_with(&url, &off).as_bytes()), 0);
    // fetch/XHR/beacon/media builder, even if a caller smuggled the header in.
    let smuggled = vec![("X-Wire-User-Agent".to_string(), "evil".to_string()), ("Accept".to_string(), "*/*".to_string())];
    for method in ["GET", "POST"] {
        let r = build_wire_request_with(method, &url, &smuggled, b"", &on).unwrap();
        assert_eq!(qy39b_count(&r), 1);
        let s = String::from_utf8_lossy(&r).to_string();
        assert!(s.contains("X-Wire-User-Agent: x-test/1.0\r\n") && !s.contains("evil") && s.contains("Accept: */*"), "{s}");
        assert_eq!(qy39b_count(&build_wire_request_with(method, &url, &[], b"", &off).unwrap()), 0);
    }
    // The global (no flag in tests) adds nothing anywhere.
    assert_eq!(qy39b_count(build_get_request_ctx(&url, &CookieContext::subresource(&url)).as_bytes()), 0);
    assert_eq!(qy39b_count(&build_wire_request("GET", &url, &[], b"").unwrap()), 0);
}

#[test]
fn qy39b_old_wire_warns_once_and_stops_sending() {
    let url = Url::parse("https://page.test/").unwrap();
    let ua = UaOverride::new(Some("x-test/1.0".into()));
    assert!(ua.header().is_some(), "sent until wire says otherwise");
    assert_eq!(ua.observe(None), Some(OLD_WIRE_WARNING));
    assert!(OLD_WIRE_WARNING.contains("wire does not support a User-Agent override (restart wire from a current build)"));
    assert_eq!(ua.observe(Some("")), None, "warned once");
    assert_eq!(qy39b_count(build_get_request_text_with(&url, &ua).as_bytes()), 0);
    // A current wire: nothing logged, still sent.
    let ok = UaOverride::new(Some("x".into()));
    assert_eq!(ok.observe(Some("foo, ua")), None);
    assert!(ok.header().is_some());
    // Without the flag nothing is ever learned or warned.
    assert_eq!(UaOverride::new(None).observe(None), None);
}

#[test]
fn qy39b_navigator_agrees_with_the_flag() {
    assert_eq!(navigator_user_agent_for(Some("Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0")), "Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0");
    assert_eq!(navigator_app_version_for(Some("Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0")), "5.0 (X11; Linux x86_64) Firefox/130.0");
    assert_eq!(navigator_app_version_for(Some("x-test/1.0")), "x-test/1.0");
    // Unchanged without the flag.
    assert!(navigator_user_agent_for(None).starts_with("Mozilla/5.0 (X11; Linux x86_64) scratchy/"));
    assert_eq!(navigator_app_version_for(None), "5.0 (X11)");
    // And the JS prelude reads both from the environment.
    let fake = QfFake::new(vec![]);
    let l = qf8_run("https://page.test/", &fake, "console.log(navigator.userAgent, '|', navigator.appVersion);");
    assert_eq!(l, vec![format!("log:{} | {}", navigator_user_agent(), navigator_app_version())], "{l:?}");
}
