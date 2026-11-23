//! W2-F: the filter chain (SPEC "Filter protocol", "Routing and decryption", "Failure handling",
//! "Security requirements", "Reference filter: pass", "Acceptance criteria"). Filters: the shipped
//! `wire2-pass` binary, the test-only pass-decrypt (in-process) and the scripted fake filter from
//! tests/common/filters.rs. Names map to 127.0.0.1 with the TEST-ONLY `--test-resolve`.

mod common;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpStream, UnixStream};
use tokio_rustls::rustls;

const HOST: &str = "www.example.test";
const RESOLVE: &[&str] = &["--test-resolve", "*.test=127.0.0.1"];

type Seen = Arc<Mutex<Vec<String>>>;

async fn respond<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S, seen: &Seen) {
    let (head, _) = read_head(s).await;
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
    seen.lock().unwrap().push(head);
    let out: Vec<u8> = match path.as_str() {
        "/chunked" => b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nX-Origin: yes\r\n\r\n6\r\nhello \r\n3;e=1\r\nwor\r\n2\r\nld\r\n0\r\n\r\n".to_vec(),
        "/slow" => {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nslow".to_vec()
        }
        p => {
            let body = format!("ok {p}");
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
        }
    };
    let _ = s.write_all(&out).await;
    let _ = s.flush().await;
    let _ = s.shutdown().await;
}

/// An origin (TLS with the test PKI when `tls` is given) that records request heads and answers by
/// path: `/chunked`, `/slow` (headers after 1.5 s), anything else `ok <path>`.
async fn origin(tls: Option<Arc<rustls::ServerConfig>>) -> (SocketAddr, Seen) {
    let seen: Seen = Arc::default();
    let s2 = seen.clone();
    let acceptor = tls.map(tokio_rustls::TlsAcceptor::from);
    let addr = serve("127.0.0.1:0", move |s| {
        let seen = s2.clone();
        let acceptor = acceptor.clone();
        async move {
            match acceptor {
                Some(a) => {
                    if let Ok(mut t) = a.accept(s).await {
                        respond(&mut t, &seen).await;
                    }
                }
                None => {
                    let mut s = s;
                    respond(&mut s, &seen).await;
                }
            }
        }
    })
    .await;
    (addr, seen)
}

/// TLS echo server (stays open for many round trips).
async fn tls_echo(cfg: Arc<rustls::ServerConfig>) -> SocketAddr {
    let a = tokio_rustls::TlsAcceptor::from(cfg);
    serve("127.0.0.1:0", move |s| {
        let a = a.clone();
        async move {
            let Ok(t) = a.accept(s).await else { return };
            let (mut r, mut w) = tokio::io::split(t);
            let mut buf = [0u8; 4096];
            while let Ok(n) = r.read(&mut buf).await {
                if n == 0 || w.write_all(&buf[..n]).await.is_err() || w.flush().await.is_err() {
                    break;
                }
            }
        }
    })
    .await
}

/// Plain `http://` request through the proxy; returns (head, raw body until close).
async fn get(proxy: SocketAddr, url: &str, extra: &str) -> (String, Vec<u8>) {
    let auth = url.split('/').nth(2).unwrap();
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(format!("GET {url} HTTP/1.1\r\nHost: {auth}\r\n{extra}\r\n").as_bytes())
        .await
        .unwrap();
    let (head, rest) = read_head(&mut s).await;
    let body = read_to_end(&mut s, rest).await;
    (head, body)
}

/// GET over a decrypted tunnel to HOST:port (client trusts the proxy CA), Connection: close.
async fn get_tls(p: &Proxy, port: u16, path: &str, extra: &str) -> (String, Vec<u8>) {
    let target = format!("{HOST}:{port}");
    let mut t = tls_via(p.addr(), &target, HOST, client_trusting(&ca_pem(p)))
        .await
        .unwrap();
    t.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: {target}\r\n{extra}Connection: close\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    t.flush().await.unwrap();
    let (head, rest) = read_head(&mut t).await;
    let body = read_to_end(&mut t, rest).await;
    (head, body)
}

fn proxy_id(p: &Proxy) -> String {
    let l = p
        .wait_log(|l| l.contains("proxy-id="), Duration::from_secs(5))
        .unwrap();
    l.split("proxy-id=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn no_proxy_headers(head: &str) -> bool {
    !head
        .lines()
        .skip(1)
        .any(|l| l.to_ascii_lowercase().starts_with("proxy-"))
}

/// Poll `f` until it returns true (at most `secs`).
async fn eventually<F, Fut>(secs: u64, mut f: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

// ---------------------------------------------------------------------------------------------------

/// SPEC: "With only pass (tunnel layer), browsing works, no CA file exists, and the access log has one
/// line per tunnel." Plus pass itself: socket mode 0600, plain http through it, SIGTERM exits.
#[tokio::test(flavor = "multi_thread")]
async fn pass_tunnels_and_plain_http_no_ca() {
    use std::os::unix::fs::PermissionsExt;
    let pki = pki();
    let tls = tls_hello_server(pki.server.clone()).await;
    let (plain, seen) = origin(None).await;
    let d = sock_dir();
    let pass = PassBin::start(&d.path().join("pass.sock"));
    assert_eq!(
        std::fs::metadata(&pass.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let proxy = proxy_with(&[&pass.path], RESOLVE, &[]);
    wait_up(&proxy, "pass");
    for i in 0..3 {
        let mut t = tls_via(
            proxy.addr(),
            &format!("{HOST}:{}", tls.port()),
            HOST,
            pki.client.clone(),
        )
        .await
        .expect("the origin's own certificate through pass");
        t.write_all(format!("GET / HTTP/1.1\r\nHost: {HOST}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let (h, rest) = read_head(&mut t).await;
        assert_eq!(status_of(&h), 200, "{i}: {h}");
        assert_eq!(read_to_end(&mut t, rest).await, b"hello over tls");
    }
    let (h, body) = get(
        proxy.addr(),
        &format!("http://{HOST}:{}/x", plain.port()),
        "",
    )
    .await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert_eq!(body, b"ok /x");
    assert!(no_proxy_headers(&seen.lock().unwrap()[0]));
    for id in 1..=3 {
        let l = proxy
            .wait_log(
                |l| l.contains(&format!("access flow={id} ")),
                Duration::from_secs(5),
            )
            .unwrap_or_else(|| panic!("{}", proxy.log()));
        assert!(
            l.contains("CONNECT host=www.example.test")
                && l.contains("decrypted=no chain=pass status=tunnel"),
            "{l}"
        );
    }
    let l = proxy
        .wait_log(|l| l.contains("access flow=4 "), Duration::from_secs(5))
        .unwrap();
    assert!(
        l.contains("GET host=www.example.test") && l.contains("chain=pass status=200"),
        "{l}"
    );
    let log = proxy.log();
    assert_eq!(log.matches("access flow=").count(), 4, "{log}");
    assert!(
        files_under(&proxy.data_dir()).is_empty(),
        "a CA appeared: {:?}",
        files_under(&proxy.data_dir())
    );
    assert!(!log.contains("CREATED A NEW CA"), "{log}");
    let st = pass.term();
    assert!(st.success(), "pass exit on SIGTERM: {st:?}");
}

/// SPEC: "Connecting pass-decrypt with hosts *.example.test creates the CA. New tunnels to
/// www.example.test are decrypted (the fake filter sees GET https://...), other hosts stay tunnels,
/// and tunnels opened earlier are untouched." The fake filter first shakes hands as a tunnel filter,
/// then (after it drops a request, so the proxy re-handshakes) as an http filter.
#[tokio::test(flavor = "multi_thread")]
async fn http_claim_creates_ca_new_tunnels_only() {
    let pki = pki();
    let roots = root_file(&pki);
    let echo = tls_echo(pki.server.clone()).await;
    let (tls_origin, seen) = origin(Some(pki.server.clone())).await;
    let plain_echo = echo_server("127.0.0.1:0").await;
    let d = sock_dir();
    let f = Fake::start(
        &d.path().join("f.sock"),
        hello_json("f", "tunnel", &["*"], "open"),
        Action::Forward,
    );
    let proxy = proxy_with(
        &[&f.inproc.path],
        RESOLVE,
        &[("SSL_CERT_FILE", roots.path())],
    );
    wait_up(&proxy, "f");
    // A tunnel opened before any http filter: opaque (the origin's own certificate), through f.
    let mut early = tls_via(
        proxy.addr(),
        &format!("{HOST}:{}", echo.port()),
        HOST,
        pki.client.clone(),
    )
    .await
    .unwrap();
    early.write_all(b"ping1").await.unwrap();
    early.flush().await.unwrap();
    let mut b = [0u8; 5];
    early.read_exact(&mut b).await.unwrap();
    assert_eq!(&b, b"ping1");
    assert!(
        !ca_dir(&proxy).join("ca.pem").exists(),
        "CA before any http filter"
    );

    // f restarts as an http filter: it drops the next request, the proxy re-handshakes.
    *f.hello.lock().unwrap() = (200, hello_json("f", "http", &["*.example.test"], "open"));
    f.set(Action::Close);
    let (_s, head, _) = connect_via(proxy.addr(), "trigger.test:9").await;
    assert_eq!(status_of(&head), 502, "{head}");
    f.set(Action::Forward);
    proxy
        .wait_log(
            |l| l.contains("http-layer claim by f"),
            Duration::from_secs(10),
        )
        .unwrap_or_else(|| panic!("{}", proxy.log()));
    assert!(proxy.log().contains("CREATED A NEW CA"));
    assert!(ca_dir(&proxy).join("ca.pem").exists());

    // New tunnels to a claimed host are decrypted; f sees the request in absolute form.
    let (h, body) = get_tls(&proxy, tls_origin.port(), "/new", "").await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert_eq!(body, b"ok /new");
    let req = f.seen().last().unwrap().clone();
    assert!(
        req.starts_with(&format!(
            "GET https://{HOST}:{}/new HTTP/1.1",
            tls_origin.port()
        )),
        "{req}"
    );
    assert_eq!(head_header(&req, "proxy-flow").len(), 1, "{req}");
    assert!(seen.lock().unwrap()[0].starts_with("GET /new HTTP/1.1"));
    // Other hosts stay opaque tunnels, unseen by f.
    let n = f.seen().len();
    let (mut s, head, _) =
        connect_via(proxy.addr(), &format!("other.test:{}", plain_echo.port())).await;
    assert_eq!(status_of(&head), 200);
    s.write_all(b"opaque").await.unwrap();
    let mut b = [0u8; 6];
    s.read_exact(&mut b).await.unwrap();
    assert_eq!(&b, b"opaque");
    assert_eq!(f.seen().len(), n, "f saw an unclaimed host");
    // The early tunnel is untouched.
    early.write_all(b"ping2").await.unwrap();
    early.flush().await.unwrap();
    let mut b = [0u8; 5];
    early.read_exact(&mut b).await.unwrap();
    assert_eq!(&b, b"ping2");
    drop((early, s));
    let l = proxy
        .wait_log(|l| l.contains("access flow=1 "), Duration::from_secs(10))
        .unwrap();
    assert!(l.contains("decrypted=no chain=f status=tunnel"), "{l}");
}

/// SPEC: "A host on --no-decrypt is never decrypted, even when an http filter claims it."
#[tokio::test(flavor = "multi_thread")]
async fn no_decrypt_wins_over_pass_decrypt() {
    let pki = pki();
    let roots = root_file(&pki);
    let tls = tls_hello_server(pki.server.clone()).await;
    let p = decrypting(
        &["*.example.test"],
        &["--test-resolve", "*.test=127.0.0.1", "--no-decrypt", HOST],
        &[("SSL_CERT_FILE", roots.path())],
    );
    let mut t = tls_via(
        p.addr(),
        &format!("{HOST}:{}", tls.port()),
        HOST,
        pki.client.clone(),
    )
    .await
    .expect("the origin's own certificate: not decrypted");
    t.write_all(format!("GET / HTTP/1.1\r\nHost: {HOST}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let (h, _) = read_head(&mut t).await;
    assert_eq!(status_of(&h), 200);
    assert!(
        p.log().contains("never decrypting www.example.test"),
        "{}",
        p.log()
    );
    drop(t);
    let l = p
        .wait_log(|l| l.contains("access flow=1 "), Duration::from_secs(5))
        .unwrap();
    assert!(l.contains("decrypted=no chain=- status=tunnel"), "{l}");
}

/// SPEC: "An http filter that answers a request itself (e.g. 451 with an HTML body) delivers exactly
/// that response, for https and for plain http://." The origin is never contacted.
#[tokio::test(flavor = "multi_thread")]
async fn http_filter_answers_itself() {
    let pki = pki();
    let roots = root_file(&pki);
    let (tls_origin, tls_seen) = origin(Some(pki.server.clone())).await;
    let (plain, plain_seen) = origin(None).await;
    let d = sock_dir();
    let page = b"<!doctype html><h1>Unavailable For Legal Reasons</h1>".to_vec();
    let f = Fake::start(
        &d.path().join("block.sock"),
        hello_json("block", "http", &["*.example.test"], "closed"),
        Action::Answer(451, "text/html".into(), page.clone()),
    );
    let proxy = proxy_with(
        &[&f.inproc.path],
        RESOLVE,
        &[("SSL_CERT_FILE", roots.path())],
    );
    wait_up(&proxy, "block");
    let (h, body) = get_tls(&proxy, tls_origin.port(), "/x", "").await;
    assert_eq!(status_of(&h), 451, "{h}");
    assert_eq!(header_of(&h, "content-type").as_deref(), Some("text/html"));
    assert_eq!(body, page);
    let (h, body) = get(
        proxy.addr(),
        &format!("http://{HOST}:{}/y", plain.port()),
        "",
    )
    .await;
    assert_eq!(status_of(&h), 451, "{h}");
    assert_eq!(body, page);
    assert!(tls_seen.lock().unwrap().is_empty() && plain_seen.lock().unwrap().is_empty());
    assert_eq!(f.seen().len(), 2);
}

/// SPEC: "An http filter that rewrites a chunked response body delivers the changed body with valid
/// framing." Plain http and a decrypted keep-alive connection (a second request after it works).
#[tokio::test(flavor = "multi_thread")]
async fn http_filter_rewrites_chunked_body() {
    let pki = pki();
    let roots = root_file(&pki);
    let (tls_origin, _) = origin(Some(pki.server.clone())).await;
    let (plain, _) = origin(None).await;
    let d = sock_dir();
    let f = Fake::start(
        &d.path().join("up.sock"),
        hello_json("upper", "http", &["*.example.test"], "closed"),
        Action::Uppercase,
    );
    let proxy = proxy_with(
        &[&f.inproc.path],
        RESOLVE,
        &[("SSL_CERT_FILE", roots.path())],
    );
    wait_up(&proxy, "upper");
    let (h, body) = get(
        proxy.addr(),
        &format!("http://{HOST}:{}/chunked", plain.port()),
        "",
    )
    .await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert_eq!(
        header_of(&h, "transfer-encoding").as_deref(),
        Some("chunked")
    );
    assert!(!has_header(&h, "content-length"));
    assert_eq!(header_of(&h, "x-origin").as_deref(), Some("yes"));
    assert_eq!(
        dechunk_complete(&body).expect("valid chunked framing"),
        b"HELLO WORLD"
    );

    let target = format!("{HOST}:{}", tls_origin.port());
    let mut t = tls_via(
        proxy.addr(),
        &target,
        HOST,
        client_trusting(&ca_pem(&proxy)),
    )
    .await
    .unwrap();
    for (i, (path, want)) in [("/chunked", &b"HELLO WORLD"[..]), ("/two", b"OK /TWO")]
        .iter()
        .enumerate()
    {
        t.write_all(format!("GET {path} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        t.flush().await.unwrap();
        let (h, mut rest) = read_head(&mut t).await;
        assert_eq!(status_of(&h), 200, "{i}: {h}");
        assert!(!has_header(&h, "connection"), "kept alive: {h}");
        let body = loop {
            if let Some(b) = dechunk_complete(&rest) {
                break b;
            }
            let mut tmp = [0u8; 4096];
            let n = t.read(&mut tmp).await.unwrap();
            assert!(n > 0, "EOF inside the rewritten body");
            rest.extend_from_slice(&tmp[..n]);
        };
        assert_eq!(&body, want);
    }
}

/// SPEC: "A tunnel filter that answers 403 to a CONNECT refuses the tunnel; one that closes an open
/// tunnel ends it within 1 s."
#[tokio::test(flavor = "multi_thread")]
async fn tunnel_filter_refuses_and_revokes() {
    let echo = echo_server("127.0.0.1:0").await;
    let d = sock_dir();
    let f = Fake::start(
        &d.path().join("t.sock"),
        hello_json("gate", "tunnel", &["*"], "closed"),
        Action::Answer(403, "text/plain".into(), b"no".to_vec()),
    );
    let proxy = proxy_with(&[&f.inproc.path], RESOLVE, &[]);
    wait_up(&proxy, "gate");
    let (_s, head, _) = connect_via(proxy.addr(), &format!("{HOST}:{}", echo.port())).await;
    assert_eq!(status_of(&head), 403, "{head}");
    assert!(f.seen()[0].starts_with(&format!("CONNECT {HOST}:{} HTTP/1.1", echo.port())));

    f.set(Action::CloseAfter(Duration::from_millis(300)));
    let t0 = Instant::now();
    let (mut s, head, _) = connect_via(proxy.addr(), &format!("{HOST}:{}", echo.port())).await;
    assert_eq!(status_of(&head), 200, "{head}");
    s.write_all(b"hi").await.unwrap();
    let mut b = [0u8; 2];
    s.read_exact(&mut b).await.unwrap();
    let mut rest = Vec::new();
    let n = tokio::time::timeout(Duration::from_secs(3), s.read_to_end(&mut rest))
        .await
        .expect("tunnel still open after the filter closed it");
    let _ = n;
    let took = t0.elapsed();
    assert!(took < Duration::from_millis(1300), "ended after {took:?}");
}

/// SPEC: "A filter whose hosts don't match a flow never gets a connection for it."
#[tokio::test(flavor = "multi_thread")]
async fn unmatched_filter_never_sees_the_flow() {
    let echo = echo_server("127.0.0.1:0").await;
    let (plain, _) = origin(None).await;
    let d = sock_dir();
    let f = Fake::start(
        &d.path().join("o.sock"),
        hello_json("only", "tunnel", &["only.test"], "closed"),
        Action::Forward,
    );
    let g = Fake::start(
        &d.path().join("h.sock"),
        hello_json("honly", "http", &["only.test"], "closed"),
        Action::Forward,
    );
    let proxy = proxy_with(&[&f.inproc.path, &g.inproc.path], RESOLVE, &[]);
    wait_up(&proxy, "only");
    wait_up(&proxy, "honly");
    let (mut s, head, _) = connect_via(proxy.addr(), &format!("{HOST}:{}", echo.port())).await;
    assert_eq!(status_of(&head), 200);
    s.write_all(b"x").await.unwrap();
    s.read_exact(&mut [0u8; 1]).await.unwrap();
    let (h, _) = get(
        proxy.addr(),
        &format!("http://{HOST}:{}/", plain.port()),
        "",
    )
    .await;
    assert_eq!(status_of(&h), 200);
    assert!(
        f.seen().is_empty() && g.seen().is_empty(),
        "{:?} {:?}",
        f.seen(),
        g.seen()
    );
    let (h, _) = get(
        proxy.addr(),
        &format!("http://only.test:{}/", plain.port()),
        "",
    )
    .await;
    assert_eq!(status_of(&h), 200);
    assert_eq!((f.seen().len(), g.seen().len()), (1, 1));
    assert!(f.seen()[0].starts_with("CONNECT only.test:"));
    assert!(g.seen()[0].starts_with("GET http://only.test:"));
    let l = proxy
        .wait_log(|l| l.contains("access flow=3 "), Duration::from_secs(5))
        .unwrap();
    assert!(l.contains("chain=honly,only"), "{l}");
}

/// SPEC: "Killing a fail-closed filter gives its hosts 503; restarting it restores service without
/// restarting the proxy. A fail-open filter that is down is skipped." Also the startup case: before
/// any handshake a filter blocks every host, loudly logged.
#[tokio::test(flavor = "multi_thread")]
async fn fail_closed_503_recovery_and_fail_open_skip() {
    let (plain, _) = origin(None).await;
    let url = format!("http://{HOST}:{}/", plain.port());
    let d = sock_dir();
    let closed_path = d.path().join("closed.sock");
    let open_path = d.path().join("open.sock");
    let hello_closed = hello_json("guard", "tunnel", &["*.example.test"], "closed");
    let f = Fake::start(&closed_path, hello_closed.clone(), Action::Forward);
    let g = Fake::start(
        &open_path,
        hello_json("optional", "tunnel", &["*"], "open"),
        Action::Forward,
    );
    let proxy = proxy_with(&[&closed_path, &open_path], RESOLVE, &[]);
    wait_up(&proxy, "guard");
    wait_up(&proxy, "optional");
    let (h, _) = get(proxy.addr(), &url, "").await;
    assert_eq!(status_of(&h), 200);
    // The fail-open filter goes away: skipped.
    drop(g);
    let (h, _) = get(proxy.addr(), &url, "").await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert!(
        proxy.log().contains("filter optional is DOWN"),
        "{}",
        proxy.log()
    );
    // The fail-closed one goes away: 503 naming it, for its hosts only.
    drop(f);
    for _ in 0..2 {
        let (h, body) = get(proxy.addr(), &url, "").await;
        assert_eq!(status_of(&h), 503, "{h}");
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains("filter guard is down"), "{body}");
    }
    let (h, _) = get(
        proxy.addr(),
        &format!("http://other.test:{}/", plain.port()),
        "",
    )
    .await;
    assert_eq!(status_of(&h), 200, "a host guard does not cover");
    let (_s, head, _) = connect_via(proxy.addr(), &format!("{HOST}:443")).await;
    assert_eq!(status_of(&head), 503, "a refused CONNECT: {head}");
    // Restart it: service comes back without restarting the proxy.
    let f2 = Fake::start(&closed_path, hello_closed, Action::Forward);
    let addr = proxy.addr();
    let back = eventually(10, || {
        let url = url.clone();
        async move { status_of(&get(addr, &url, "").await.0) == 200 }
    })
    .await;
    assert!(back, "no recovery: {}", proxy.log());
    assert!(f2.hello_count() >= 1 && !f2.seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn before_any_handshake_everything_is_refused() {
    let (plain, _) = origin(None).await;
    let url = format!("http://{HOST}:{}/", plain.port());
    let d = sock_dir();
    let path = d.path().join("late.sock");
    let proxy = proxy_with(
        &[&path],
        &[
            "--test-resolve",
            "*.test=127.0.0.1",
            "--connect-timeout",
            "300ms",
        ],
        &[],
    );
    let t = Instant::now();
    let (h, body) = get(proxy.addr(), &url, "").await;
    assert_eq!(status_of(&h), 503, "{h}");
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains(&format!("unix:{}", path.display()))
    );
    assert!(t.elapsed() < Duration::from_secs(3));
    let (_s, head, _) = connect_via(proxy.addr(), "anything.test:443").await;
    assert_eq!(status_of(&head), 503);
    assert!(
        proxy.log().contains("did not complete its handshake")
            && proxy.log().contains("EVERY host"),
        "{}",
        proxy.log()
    );
    // The filter shows up later: the proxy finds it (systemd socket activation makes this instant).
    let _f = Fake::start(
        &path,
        hello_json("late", "tunnel", &["*"], "closed"),
        Action::Forward,
    );
    let addr = proxy.addr();
    assert!(
        eventually(10, || {
            let url = url.clone();
            async move { status_of(&get(addr, &url, "").await.0) == 200 }
        })
        .await,
        "{}",
        proxy.log()
    );
}

/// SPEC: "A stage that never sends response headers gives 504 after --stage-timeout." The filter
/// stays connected; time the request spends further down the chain (a slow origin) does not count.
#[tokio::test(flavor = "multi_thread")]
async fn stage_timeout_504() {
    let (plain, _) = origin(None).await;
    let d = sock_dir();
    let f = Fake::start(
        &d.path().join("slow.sock"),
        hello_json("slowpoke", "http", &["*.example.test"], "closed"),
        Action::Hang,
    );
    let t = Fake::start(
        &d.path().join("tslow.sock"),
        hello_json("tslow", "tunnel", &["tunnel.test"], "closed"),
        Action::Hang,
    );
    let proxy = proxy_with(
        &[&f.inproc.path, &t.inproc.path],
        &[
            "--test-resolve",
            "*.test=127.0.0.1",
            "--stage-timeout",
            "500ms",
        ],
        &[],
    );
    wait_up(&proxy, "slowpoke");
    let t0 = Instant::now();
    let (h, _) = get(
        proxy.addr(),
        &format!("http://{HOST}:{}/", plain.port()),
        "",
    )
    .await;
    let took = t0.elapsed();
    assert_eq!(status_of(&h), 504, "{h}");
    assert!(
        took >= Duration::from_millis(450) && took < Duration::from_secs(3),
        "{took:?}"
    );
    assert!(
        proxy
            .log()
            .contains("filter slowpoke sent no response headers")
    );
    let t0 = Instant::now();
    let (_s, head, _) = connect_via(proxy.addr(), "tunnel.test:443").await;
    assert_eq!(status_of(&head), 504, "{head}");
    assert!(t0.elapsed() < Duration::from_secs(3));
    // Still connected: no new handshake. The origin takes 1.5 s, three stage timeouts, to answer;
    // the stage forwarded promptly, so that is fine.
    f.set(Action::Forward);
    let (h, body) = get(
        proxy.addr(),
        &format!("http://{HOST}:{}/slow", plain.port()),
        "",
    )
    .await;
    assert_eq!(status_of(&h), 200, "{h} {}", proxy.log());
    assert_eq!(body, b"slow");
    assert_eq!(f.hello_count(), 1);
    assert_eq!(t.hello_count(), 1);
}

/// SPEC: "Malformed HTTP from a filter: 502, the connection is closed, and the error is logged";
/// "Filter connection fails or closes before response headers: 502; marked down, redialled, the
/// handshake repeats."
#[tokio::test(flavor = "multi_thread")]
async fn malformed_and_closed_filter_502() {
    let (plain, _) = origin(None).await;
    let url = format!("http://{HOST}:{}/", plain.port());
    let d = sock_dir();
    let f = Fake::start(
        &d.path().join("bad.sock"),
        hello_json("bad", "http", &["*"], "closed"),
        Action::Malformed,
    );
    let proxy = proxy_with(&[&f.inproc.path], RESOLVE, &[]);
    wait_up(&proxy, "bad");
    let (h, _) = get(proxy.addr(), &url, "").await;
    assert_eq!(status_of(&h), 502, "{h}");
    assert!(
        proxy.log().contains("malformed HTTP from filter bad"),
        "{}",
        proxy.log()
    );
    assert_eq!(f.hello_count(), 1, "malformed: still up");
    f.set(Action::Close);
    let (h, _) = get(proxy.addr(), &url, "").await;
    assert_eq!(status_of(&h), 502, "{h}");
    assert!(
        proxy.log().contains("filter bad is DOWN"),
        "{}",
        proxy.log()
    );
    assert!(
        eventually(5, || {
            let n = f.hello_count();
            async move { n >= 2 }
        })
        .await,
        "no re-handshake: {}",
        proxy.log()
    );
    f.set(Action::Forward);
    let (h, _) = get(proxy.addr(), &url, "").await;
    assert_eq!(status_of(&h), 200, "{h}");
}

/// SPEC: "Proxy-* headers sent by the browser or a filter that the proxy did not set never reach the
/// next stage or the server"; "Proxy-Id or Proxy-Flow sent by a browser is stripped and never reaches
/// a filter".
#[tokio::test(flavor = "multi_thread")]
async fn proxy_headers_are_stripped() {
    let pki = pki();
    let roots = root_file(&pki);
    let (plain, seen) = origin(None).await;
    let (tls_origin, tls_seen) = origin(Some(pki.server.clone())).await;
    let d = sock_dir();
    let h = Fake::start(
        &d.path().join("h.sock"),
        hello_json("hfilter", "http", &["*.example.test"], "closed"),
        Action::ForwardAdding("Proxy-Id: forged-by-filter\r\nProxy-Foo: filter\r\n".into()),
    );
    let t = Fake::start(
        &d.path().join("t.sock"),
        hello_json("tfilter", "tunnel", &["*"], "closed"),
        Action::Forward,
    );
    let proxy = proxy_with(
        &[&h.inproc.path, &t.inproc.path],
        RESOLVE,
        &[("SSL_CERT_FILE", roots.path())],
    );
    wait_up(&proxy, "hfilter");
    wait_up(&proxy, "tfilter");
    let id = proxy_id(&proxy);
    let forged = "Proxy-Id: forged\r\nProxy-Flow: forged/1\r\nProxy-Foo: browser\r\nProxy-Authorization: Basic eA==\r\n";
    let (r, body) = get(
        proxy.addr(),
        &format!("http://{HOST}:{}/p", plain.port()),
        forged,
    )
    .await;
    assert_eq!(status_of(&r), 200, "{r}");
    assert_eq!(body, b"ok /p");
    let (r, body) = get_tls(&proxy, tls_origin.port(), "/s", forged).await;
    assert_eq!(status_of(&r), 200, "{r}");
    assert_eq!(body, b"ok /s");
    let hs = h.seen();
    assert_eq!(hs.len(), 2);
    for req in &hs {
        let flows = head_header(req, "proxy-flow");
        assert_eq!(flows.len(), 1, "{req}");
        assert!(flows[0].starts_with(&format!("{id}/")), "{req}");
        assert!(head_header(req, "proxy-id").is_empty(), "{req}");
        assert!(head_header(req, "proxy-foo").is_empty(), "{req}");
        assert!(head_header(req, "proxy-authorization").is_empty(), "{req}");
    }
    // The tunnel filter (upstream connector) got CONNECTs with only the proxy's Proxy-Flow.
    let ts = t.seen();
    assert_eq!(ts.len(), 2, "{ts:?}");
    for req in &ts {
        assert!(req.starts_with("CONNECT "), "{req}");
        let proxy_lines: Vec<&str> = req
            .lines()
            .filter(|l| l.to_ascii_lowercase().starts_with("proxy-"))
            .collect();
        assert_eq!(proxy_lines.len(), 1, "{req}");
        assert!(
            proxy_lines[0].starts_with(&format!("Proxy-Flow: {id}/")),
            "{req}"
        );
    }
    // A browser CONNECT with forged headers: the tunnel filter sees only Proxy-Flow.
    let echo = echo_server("127.0.0.1:0").await;
    let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
    s.write_all(
        format!("CONNECT other.test:{} HTTP/1.1\r\nHost: x\r\nProxy-Id: forged\r\nProxy-Flow: forged/2\r\n\r\n", echo.port())
            .as_bytes(),
    )
    .await
    .unwrap();
    let (head, _) = read_head(&mut s).await;
    assert_eq!(status_of(&head), 200, "{head}");
    let last = t.seen().last().unwrap().clone();
    assert!(head_header(&last, "proxy-id").is_empty(), "{last}");
    assert_eq!(head_header(&last, "proxy-flow").len(), 1, "{last}");
    // Nothing Proxy-* reached the servers, the filter's forgeries included.
    for req in seen
        .lock()
        .unwrap()
        .iter()
        .chain(tls_seen.lock().unwrap().iter())
    {
        assert!(no_proxy_headers(req), "{req}");
    }
}

/// SPEC: "A page's GET https://www.example.test/hello reaches filters as traffic and is forwarded to
/// the server; a filter never treats it as a handshake." pass-decrypt is the real pass code, which
/// answers a handshake if it sees Proxy-Id: the page's own (forged) one must not reach it.
#[tokio::test(flavor = "multi_thread")]
async fn page_hello_is_traffic() {
    let pki = pki();
    let roots = root_file(&pki);
    let (tls_origin, tls_seen) = origin(Some(pki.server.clone())).await;
    let (plain, plain_seen) = origin(None).await;
    let d = sock_dir();
    let rec = Fake::start(
        &d.path().join("rec.sock"),
        hello_json("rec", "http", &["*.example.test"], "closed"),
        Action::Forward,
    );
    let pd_path = d.path().join("pd.sock");
    let _pd = pass_decrypt(&pd_path, &["*.example.test"]);
    let proxy = proxy_with(
        &[&rec.inproc.path, &pd_path],
        RESOLVE,
        &[("SSL_CERT_FILE", roots.path())],
    );
    wait_up(&proxy, "rec");
    wait_up(&proxy, "pass-decrypt");
    let hellos = rec.hello_count();
    let (h, body) = get_tls(
        &proxy,
        tls_origin.port(),
        "/hello",
        "Proxy-Id: page\r\nProxy-Hub: unix:/x\r\n",
    )
    .await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert_eq!(body, b"ok /hello");
    let (h, body) = get(
        proxy.addr(),
        &format!("http://{HOST}:{}/hello", plain.port()),
        "Proxy-Id: page\r\n",
    )
    .await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert_eq!(body, b"ok /hello");
    let seen = rec.seen();
    assert!(
        seen[0].starts_with(&format!(
            "GET https://{HOST}:{}/hello HTTP/1.1",
            tls_origin.port()
        )),
        "{}",
        seen[0]
    );
    assert!(
        seen[1].starts_with(&format!(
            "GET http://{HOST}:{}/hello HTTP/1.1",
            plain.port()
        )),
        "{}",
        seen[1]
    );
    assert_eq!(
        rec.hello_count(),
        hellos,
        "a page request was taken for a handshake"
    );
    assert!(tls_seen.lock().unwrap()[0].starts_with("GET /hello HTTP/1.1"));
    assert!(plain_seen.lock().unwrap()[0].starts_with("GET /hello HTTP/1.1"));
}

/// SPEC: "a request to the hub with a Proxy-Flow the proxy did not issue is refused".
#[tokio::test(flavor = "multi_thread")]
async fn hub_refuses_unknown_flows() {
    let (plain, plain_seen) = origin(None).await;
    let d = sock_dir();
    let f = Fake::start(
        &d.path().join("f.sock"),
        hello_json("forger", "http", &["*"], "closed"),
        Action::ForwardWithFlow("feedc0ffee00/1".into()),
    );
    let proxy = proxy_with(&[&f.inproc.path], RESOLVE, &[]);
    wait_up(&proxy, "forger");
    let id = proxy_id(&proxy);
    // A filter forwarding with a Proxy-Flow from another proxy: the hub says 403, the browser sees it.
    let (h, body) = get(
        proxy.addr(),
        &format!("http://{HOST}:{}/", plain.port()),
        "",
    )
    .await;
    assert_eq!(status_of(&h), 403, "{h}");
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("not issued by this proxy")
    );
    assert!(plain_seen.lock().unwrap().is_empty());
    let hub = f.hub();
    async fn ask(hub: &Path, req: String) -> String {
        let mut s = UnixStream::connect(hub).await.unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await;
        String::from_utf8_lossy(&out).into_owned()
    }
    let url = format!("http://{HOST}:{}/", plain.port());
    for (flow, why) in [
        (None, "no Proxy-Flow header"),
        (Some("bogus".to_string()), "not issued by this proxy"),
        (Some(format!("{id}/999999")), "no such open flow"),
        (Some(format!("{id}/x")), "not issued by this proxy"),
    ] {
        let hdr = flow
            .map(|v| format!("Proxy-Flow: {v}\r\n"))
            .unwrap_or_default();
        let r = ask(&hub, format!("GET {url} HTTP/1.1\r\nHost: x\r\n{hdr}\r\n")).await;
        assert_eq!(status_of(&r), 403, "{why}: {r}");
        assert!(r.contains(why), "{r}");
    }
    // An open flow that is not waiting for a request at that layer.
    f.set(Action::Hang);
    let addr = proxy.addr();
    let u2 = url.clone();
    let pending = tokio::spawn(async move { get(addr, &u2, "").await });
    let flow_id = {
        let rx = f.seen.clone();
        assert!(
            eventually(5, || {
                let n = rx.lock().unwrap().len();
                async move { n >= 2 }
            })
            .await
        );
        let last = f.seen().last().unwrap().clone();
        head_header(&last, "proxy-flow")[0].clone()
    };
    let r = ask(
        &hub,
        format!(
            "CONNECT {HOST}:{} HTTP/1.1\r\nHost: x\r\nProxy-Flow: {flow_id}\r\n\r\n",
            plain.port()
        ),
    )
    .await;
    assert_eq!(status_of(&r), 403, "{r}");
    assert!(r.contains("not waiting"), "{r}");
    pending.abort();
    assert!(plain_seen.lock().unwrap().is_empty());
}

/// SPEC "Socket trust": the proxy refuses a filter socket or hub in a directory that is not its own
/// (root's /run) or is world-writable (/tmp); other filter schemes are a startup error.
#[test]
fn untrusted_socket_dirs_are_refused() {
    fn run(args: &[&str]) -> (i32, String) {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("run")).unwrap();
        let out = std::process::Command::new(BIN)
            .args(args)
            .args(["--listen", "127.0.0.1:0"])
            .env("XDG_RUNTIME_DIR", d.path().join("run"))
            .env("XDG_DATA_HOME", d.path().join("data"))
            .output()
            .unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
    let (code, err) = run(&["--filter", "unix:/tmp/wire2-w2f-test.sock"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("world-writable"), "{err}");
    let (code, err) = run(&["--filter", "unix:/run/wire2-w2f-test.sock"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("owned by uid 0"), "{err}");
    let ok = sock_dir();
    let f = format!("unix:{}", ok.path().join("f.sock").display());
    let (code, err) = run(&["--filter", &f, "--hub", "/tmp/wire2-w2f-hub.sock"]);
    assert_eq!(code, 1, "{err}");
    assert!(
        err.contains("world-writable") && err.contains("hub"),
        "{err}"
    );
    let (code, err) = run(&["--filter", "tcp:127.0.0.1:1"]);
    assert_eq!(code, 2, "{err}");
}

/// SPEC: "A WebSocket echo works through pass" (through pass-decrypt: tests/w2_3_websocket.rs), as an
/// opaque tunnel and as a plain ws:// upgrade (which pass sees as a CONNECT from the upstream
/// connector).
#[tokio::test(flavor = "multi_thread")]
async fn websocket_through_pass() {
    let ws = ws_echo_server().await;
    let d = sock_dir();
    let pass = PassBin::start(&d.path().join("pass.sock"));
    let proxy = proxy_with(&[&pass.path], RESOLVE, &[]);
    wait_up(&proxy, "pass");
    let target = format!("{HOST}:{}", ws.port());
    let (mut s, head, rest) = connect_via(proxy.addr(), &target).await;
    assert_eq!(status_of(&head), 200);
    assert!(rest.is_empty());
    ws_echo_session(&mut s, "/chat", &target).await;
    drop(s);
    let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
    ws_echo_session(&mut s, &format!("http://{target}/chat"), &target).await;
    drop(s);
    let l = proxy
        .wait_log(|l| l.contains("access flow=2 "), Duration::from_secs(10))
        .unwrap();
    assert!(l.contains("status=101") && l.contains("chain=pass"), "{l}");
}

/// pass under systemd socket activation (LISTEN_FDS, fd 3), via systemd-socket-activate when present.
#[tokio::test(flavor = "multi_thread")]
async fn pass_socket_activation() {
    if std::process::Command::new("systemd-socket-activate")
        .arg("--help")
        .output()
        .is_err()
    {
        eprintln!("SKIP: systemd-socket-activate not installed");
        return;
    }
    let (plain, _) = origin(None).await;
    let d = sock_dir();
    let sock = d.path().join("act.sock");
    let mut act = std::process::Command::new("systemd-socket-activate")
        .arg("-l")
        .arg(&sock)
        .arg(PASS_BIN)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < end, "no socket");
        std::thread::sleep(Duration::from_millis(10));
    }
    let proxy = proxy_with(&[&sock], RESOLVE, &[]);
    wait_up(&proxy, "pass");
    let (h, body) = get(
        proxy.addr(),
        &format!("http://{HOST}:{}/a", plain.port()),
        "",
    )
    .await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert_eq!(body, b"ok /a");
    // SAFETY: our own child.
    unsafe { libc::kill(act.id() as i32, libc::SIGTERM) };
    let _ = act.wait();
}
