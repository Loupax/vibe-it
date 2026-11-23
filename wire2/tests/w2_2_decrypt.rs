//! W2-2: TLS interception. Decryption is forced with the TEST-ONLY `--test-decrypt GLOB` (a stand-in
//! for an http-layer filter until W2-F), names are mapped with the TEST-ONLY `--test-resolve` (W2-1a),
//! and the upstream roots are the test root through `SSL_CERT_FILE`.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_rustls::rustls;

const HOST: &str = "www.example.test";

/// Decode a chunked body from `r` (after `pre`); returns (body, bytes after it).
async fn read_chunked<R: AsyncRead + Unpin>(r: &mut R, mut buf: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
    let mut body = Vec::new();
    let mut tmp = vec![0u8; 65536];
    async fn need<R: AsyncRead + Unpin>(r: &mut R, buf: &mut Vec<u8>, tmp: &mut [u8]) {
        let n = r.read(tmp).await.unwrap();
        assert!(n > 0, "EOF inside a chunked body");
        buf.extend_from_slice(&tmp[..n]);
    }
    loop {
        let line_end = loop {
            if let Some(i) = buf.windows(2).position(|w| w == b"\r\n") {
                break i;
            }
            need(r, &mut buf, &mut tmp).await;
        };
        let line = String::from_utf8_lossy(&buf[..line_end]).to_string();
        let size = usize::from_str_radix(line.split(';').next().unwrap().trim(), 16).unwrap();
        buf.drain(..line_end + 2);
        if size == 0 {
            // No trailers in these tests: the final CRLF.
            while buf.len() < 2 {
                need(r, &mut buf, &mut tmp).await;
            }
            assert_eq!(&buf[..2], b"\r\n");
            buf.drain(..2);
            return (body, buf);
        }
        while buf.len() < size + 2 {
            need(r, &mut buf, &mut tmp).await;
        }
        body.extend_from_slice(&buf[..size]);
        buf.drain(..size + 2);
    }
}

/// Read one response (Content-Length or chunked) from a kept-alive connection.
async fn read_response<R: AsyncRead + Unpin>(r: &mut R) -> (String, Vec<u8>) {
    let (head, mut rest) = read_head(r).await;
    if header_of(&head, "transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        let (body, after) = read_chunked(r, rest).await;
        assert!(after.is_empty());
        return (head, body);
    }
    let n: usize = header_of(&head, "content-length").unwrap().parse().unwrap();
    while rest.len() < n {
        let mut tmp = vec![0u8; 65536];
        let k = r.read(&mut tmp).await.unwrap();
        assert!(k > 0);
        rest.extend_from_slice(&tmp[..k]);
    }
    assert_eq!(rest.len(), n);
    (head, rest)
}

/// HTTPS origin (one request per connection, as the proxy always sends `Connection: close`):
///   /head       -> 200, body = the request head it received
///   /echo       -> 200 Content-Length, body = the request body (Content-Length or chunked)
///   /chunked    -> 200 chunked, three chunks
///   anything    -> 404
async fn origin(cfg: Arc<rustls::ServerConfig>) -> std::net::SocketAddr {
    let acceptor = tokio_rustls::TlsAcceptor::from(cfg);
    serve("127.0.0.1:0", move |s| {
        let acceptor = acceptor.clone();
        async move {
            let Ok(mut tls) = acceptor.accept(s).await else {
                return;
            };
            let (head, rest) = read_head(&mut tls).await;
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            let reply = |body: &[u8]| {
                let mut v = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
                    .into_bytes();
                v.extend_from_slice(body);
                v
            };
            let out = match path.as_str() {
                "/head" => reply(head.as_bytes()),
                "/echo" => {
                    let body = if header_of(&head, "transfer-encoding").is_some() {
                        read_chunked(&mut tls, rest).await.0
                    } else {
                        let n: usize = header_of(&head, "content-length")
                            .map(|v| v.parse().unwrap())
                            .unwrap_or(0);
                        let mut b = rest;
                        let mut tmp = vec![0u8; 65536];
                        while b.len() < n {
                            let k = tls.read(&mut tmp).await.unwrap();
                            assert!(k > 0);
                            b.extend_from_slice(&tmp[..k]);
                        }
                        b
                    };
                    reply(&body)
                }
                "/chunked" => b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n1;ext=1\r\n \r\n5\r\nworld\r\n0\r\n\r\n".to_vec(),
                _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
            };
            let _ = tls.write_all(&out).await;
            let _ = tls.shutdown().await;
        }
    })
    .await
}

fn decrypting_proxy(extra: &[&str], roots: &Path) -> Proxy {
    let mut args = vec![
        "--test-resolve",
        "*.example.test=127.0.0.1",
        "--test-decrypt",
        "*.example.test",
    ];
    args.extend_from_slice(extra);
    Proxy::start_env(&args, &[("SSL_CERT_FILE", roots)])
}

async fn send<W: AsyncWrite + Unpin>(w: &mut W, data: &[u8]) {
    w.write_all(data).await.unwrap();
    w.flush().await.unwrap();
}

/// GET, POST (Content-Length), POST (chunked) and a chunked response, all on one decrypted
/// keep-alive connection; the browser sees a minted leaf chaining to the CA, never the origin's.
#[tokio::test(flavor = "multi_thread")]
async fn decrypted_requests_and_cert_chain() {
    let pki = pki();
    let roots = root_file(&pki);
    let server = origin(pki.server.clone()).await;
    let proxy = decrypting_proxy(&[], roots.path());
    // The claim (--test-decrypt) created the CA at startup, loudly, with a 0600 key.
    assert!(proxy.log().contains("CREATED A NEW CA"), "{}", proxy.log());
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(ca_dir(&proxy).join("ca-key.pem"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let target = format!("{HOST}:{}", server.port());

    // A client trusting only the origin's test root must reject the proxy's minted leaf.
    let e = tls_via(proxy.addr(), &target, HOST, pki.client.clone())
        .await
        .unwrap_err();
    assert!(e.to_string().contains("UnknownIssuer"), "{e}");

    let mut tls = tls_via(
        proxy.addr(),
        &target,
        HOST,
        client_trusting(&ca_pem(&proxy)),
    )
    .await
    .expect("minted leaf chains to the proxy CA");
    {
        let (_, c) = tls.get_ref();
        assert_eq!(c.alpn_protocol(), Some(&b"http/1.1"[..]));
        assert_eq!(c.peer_certificates().unwrap().len(), 1, "leaf only");
    }

    // 1. GET: the origin sees origin-form, Host from the tunnel, no Proxy-* or hop-by-hop headers.
    send(
        &mut tls,
        format!("GET /head HTTP/1.1\r\nHost: {target}\r\nProxy-Flow: forged/1\r\nKeep-Alive: 5\r\nX-Test: yes\r\n\r\n").as_bytes(),
    )
    .await;
    let (h, body) = read_response(&mut tls).await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert!(!has_header(&h, "connection"), "keep-alive response: {h}");
    let seen = String::from_utf8(body).unwrap();
    assert!(seen.starts_with("GET /head HTTP/1.1\r\n"), "{seen}");
    assert_eq!(header_of(&seen, "host").unwrap(), target);
    assert_eq!(header_of(&seen, "x-test").unwrap(), "yes");
    assert!(!has_header(&seen, "proxy-flow"), "{seen}");
    assert!(!has_header(&seen, "keep-alive"), "{seen}");

    // 2. POST with Content-Length, streamed.
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    send(
        &mut tls,
        format!(
            "POST /echo HTTP/1.1\r\nHost: {target}\r\nContent-Length: {}\r\n\r\n",
            big.len()
        )
        .as_bytes(),
    )
    .await;
    send(&mut tls, &big).await;
    let (h, body) = read_response(&mut tls).await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert!(body == big, "POST body differs");

    // 3. POST chunked.
    send(
        &mut tls,
        format!("POST /echo HTTP/1.1\r\nHost: {target}\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n4\r\ndefg\r\n0\r\n\r\n").as_bytes(),
    )
    .await;
    let (h, body) = read_response(&mut tls).await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert_eq!(body, b"abcdefg");

    // 4. A chunked response passes through with its framing; Connection: close ends the connection.
    send(
        &mut tls,
        format!("GET /chunked HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await;
    let (h, rest) = read_head(&mut tls).await;
    assert_eq!(status_of(&h), 200, "{h}");
    assert_eq!(header_of(&h, "connection").as_deref(), Some("close"));
    assert_eq!(
        header_of(&h, "transfer-encoding").as_deref(),
        Some("chunked")
    );
    let raw = read_to_end(&mut tls, rest).await;
    assert_eq!(
        raw, b"5\r\nhello\r\n1;ext=1\r\n \r\n5\r\nworld\r\n0\r\n\r\n",
        "chunks unchanged"
    );

    // Log: one line per request with the full https URL, the flow line says decrypted, the first
    // decrypt of the host logged once.
    let url = format!("https://{target}");
    for (n, req) in [
        (1, "GET /head"),
        (2, "POST /echo"),
        (3, "POST /echo"),
        (4, "GET /chunked"),
    ] {
        let (m, p) = req.split_once(' ').unwrap();
        let want = format!("req={n} {m} {url}{p} status=200");
        proxy
            .wait_log(|l| l.contains(&want), Duration::from_secs(5))
            .unwrap_or_else(|| panic!("no {want:?} in\n{}", proxy.log()));
    }
    let flow_line = proxy
        .wait_log(
            |l| l.contains("access flow=") && l.contains("CONNECT host=www.example.test"),
            Duration::from_secs(5),
        )
        .unwrap();
    assert!(flow_line.contains("decrypted=yes"), "{flow_line}");
    let log = proxy.log();
    assert_eq!(
        log.matches("decrypting www.example.test for the first time")
            .count(),
        1,
        "{log}"
    );
}

/// Hosts nobody claims stay opaque tunnels (the origin's own certificate reaches the client), and
/// `--no-decrypt` wins over a claim; `*.example.test` does not cover `example.test`.
#[tokio::test(flavor = "multi_thread")]
async fn unclaimed_and_no_decrypt_hosts_stay_tunnels() {
    let pki = pki();
    let roots = root_file(&pki);
    let server = tls_hello_server(pki.server.clone()).await;
    let proxy = decrypting_proxy(
        &[
            "--no-decrypt",
            "WWW.Example.Test.",
            "--test-resolve",
            "localhost=127.0.0.1",
        ],
        roots.path(),
    );
    for (target, sni) in [
        (format!("{HOST}:{}", server.port()), HOST),
        (format!("localhost:{}", server.port()), "localhost"),
    ] {
        // Trusting only the origin's root works: the bytes are the origin's TLS, not the proxy's.
        let mut tls = tls_via(proxy.addr(), &target, sni, pki.client.clone())
            .await
            .unwrap_or_else(|e| panic!("{target}: {e}"));
        send(
            &mut tls,
            format!("GET / HTTP/1.1\r\nHost: {sni}\r\n\r\n").as_bytes(),
        )
        .await;
        let (h, rest) = read_head(&mut tls).await;
        assert_eq!(status_of(&h), 200);
        assert_eq!(read_to_end(&mut tls, rest).await, b"hello over tls");
    }
    let line = proxy
        .wait_log(
            |l| l.contains("access flow=") && l.contains("host=www.example.test"),
            Duration::from_secs(5),
        )
        .unwrap();
    assert!(
        line.contains("decrypted=no") && line.contains("status=tunnel"),
        "{line}"
    );
    let log = proxy.log();
    assert!(
        log.contains("never decrypting www.example.test: on --no-decrypt www.example.test"),
        "{log}"
    );
    assert!(
        !log.contains("decrypting www.example.test for the first time"),
        "{log}"
    );
}

/// Upstream certificate problems answer 502 inside the decrypted tunnel and are logged.
#[tokio::test(flavor = "multi_thread")]
async fn bad_upstream_certificate_is_502() {
    let pki = pki();
    let roots = root_file(&pki);
    // An origin whose certificate comes from a root the proxy does not trust.
    let other = common::pki();
    let untrusted = origin(other.server.clone()).await;
    // An origin with a trusted certificate that does not name the host asked for.
    let wrong_name = origin(pki.server.clone()).await;
    let proxy = decrypting_proxy(&[], roots.path());
    let browser = client_trusting(&ca_pem(&proxy));
    for (host, port, why) in [
        (HOST, untrusted.port(), "invalid peer certificate"),
        (
            "other.example.test",
            wrong_name.port(),
            "not valid for name",
        ),
    ] {
        let target = format!("{host}:{port}");
        let mut tls = tls_via(proxy.addr(), &target, host, browser.clone())
            .await
            .unwrap();
        send(
            &mut tls,
            format!("GET /head HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes(),
        )
        .await;
        let (h, rest) = read_head(&mut tls).await;
        assert_eq!(status_of(&h), 502, "{h}");
        let body = String::from_utf8(read_to_end(&mut tls, rest).await).unwrap();
        assert!(body.contains("upstream tls"), "{body}");
        let line = proxy
            .wait_log(
                |l| {
                    l.contains("upstream error, answering 502")
                        && l.contains(&format!("host={host}"))
                },
                Duration::from_secs(5),
            )
            .unwrap_or_else(|| panic!("{}", proxy.log()));
        assert!(line.contains(why), "{line}");
    }
}

/// Without a claim nothing is decrypted and no CA is created, also for www.example.test mapped with
/// --test-resolve (W2-1a) and with --no-decrypt / --ca-dir given.
#[tokio::test(flavor = "multi_thread")]
async fn no_claim_no_ca() {
    let pki = pki();
    let roots = root_file(&pki);
    let server = tls_hello_server(pki.server.clone()).await;
    let proxy = Proxy::start_env(
        &[
            "--test-resolve",
            "www.example.test=127.0.0.1",
            "--no-decrypt",
            "*.bank.test",
        ],
        &[("SSL_CERT_FILE", roots.path())],
    );
    let target = format!("{HOST}:{}", server.port());
    let mut tls = tls_via(proxy.addr(), &target, HOST, pki.client.clone())
        .await
        .unwrap();
    send(
        &mut tls,
        format!("GET / HTTP/1.1\r\nHost: {HOST}\r\n\r\n").as_bytes(),
    )
    .await;
    let (h, rest) = read_head(&mut tls).await;
    assert_eq!(status_of(&h), 200);
    assert_eq!(read_to_end(&mut tls, rest).await, b"hello over tls");
    drop(tls);
    let line = proxy
        .wait_log(|l| l.contains("access flow="), Duration::from_secs(5))
        .unwrap();
    assert!(
        line.contains("host=www.example.test") && line.contains("decrypted=no"),
        "{line}"
    );
    assert!(
        files_under(proxy.dir.path()).is_empty(),
        "files appeared: {:?}",
        files_under(proxy.dir.path())
    );
}

/// A second start with the same --ca-dir reuses the CA (no new one, leaves still chain to ca.pem).
#[tokio::test(flavor = "multi_thread")]
async fn ca_reused_across_restarts() {
    let pki = pki();
    let roots = root_file(&pki);
    let server = origin(pki.server.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("ca");
    let ca_s = ca.to_str().unwrap();
    let first = decrypting_proxy(&["--ca-dir", ca_s], roots.path());
    assert!(first.log().contains("CREATED A NEW CA"));
    let pem = std::fs::read_to_string(ca.join("ca.pem")).unwrap();
    drop(first);
    let second = decrypting_proxy(&["--ca-dir", ca_s], roots.path());
    let log = second.log();
    assert!(
        !log.contains("CREATED A NEW CA") && log.contains("using the CA"),
        "{log}"
    );
    assert_eq!(std::fs::read_to_string(ca.join("ca.pem")).unwrap(), pem);
    let target = format!("{HOST}:{}", server.port());
    let mut tls = tls_via(second.addr(), &target, HOST, client_trusting(&pem))
        .await
        .expect("reloaded CA signs leaves that chain to the original ca.pem");
    send(
        &mut tls,
        format!("GET /head HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await;
    let (h, _) = read_head(&mut tls).await;
    assert_eq!(status_of(&h), 200, "{h}");
}
