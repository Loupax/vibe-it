//! W2-1: plain http:// requests in absolute form (one request per browser connection).

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// An origin that records the request head it got, reads the body as framed by Content-Length or chunked
/// (raw), and answers with `response`. It does NOT close after answering, so the proxy must find the end
/// of the response from its framing.
async fn recording_origin(
    response: &'static [u8],
) -> (std::net::SocketAddr, Arc<Mutex<Vec<(String, Vec<u8>)>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let addr = serve("127.0.0.1:0", move |mut s| {
        let seen = s2.clone();
        async move {
            let (head, mut body) = read_head(&mut s).await;
            if let Some(n) =
                header_of(&head, "content-length").and_then(|v| v.parse::<usize>().ok())
            {
                while body.len() < n {
                    let mut b = vec![0u8; n - body.len()];
                    let k = s.read(&mut b).await.unwrap();
                    assert!(k > 0);
                    body.extend_from_slice(&b[..k]);
                }
            } else if header_of(&head, "transfer-encoding").is_some() {
                while !body.ends_with(b"0\r\n\r\n") {
                    let mut b = [0u8; 4096];
                    let k = s.read(&mut b).await.unwrap();
                    assert!(k > 0);
                    body.extend_from_slice(&b[..k]);
                }
            }
            seen.lock().unwrap().push((head, body));
            s.write_all(response).await.unwrap();
            // Keep the connection open for a while: the proxy must not wait for our close.
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    })
    .await;
    (addr, seen)
}

/// SPEC: "A plain `http://` request to a non-80 port reaches that port without hop-by-hop headers and is
/// answered with `Connection: close`." Plus: every Proxy-* header from the browser is stripped, the
/// response is streamed back unchanged (status 3xx, Content-Encoding, chunked framing).
#[tokio::test(flavor = "multi_thread")]
async fn plain_http_non80_hop_by_hop_and_proxy_headers() {
    const RESP: &[u8] = b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Encoding: gzip\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive, X-Origin-Hop\r\nX-Origin-Hop: 1\r\nKeep-Alive: timeout=5\r\nProxy-Authenticate: Basic\r\nX-Origin-Keep: 1\r\n\r\n4\r\n\x1f\x8b\x08\x00\r\n3;ext=1\r\nabc\r\n0\r\nX-Trailer: t\r\n\r\n";
    let (origin, seen) = recording_origin(RESP).await;
    let proxy = Proxy::start(&[]);
    let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
    let req = format!(
        "GET http://127.0.0.1:{}/path?q=1#frag HTTP/1.1\r\nHost: wrong.example\r\nConnection: keep-alive, X-Hop\r\nX-Hop: drop me\r\nKeep-Alive: 300\r\nTE: trailers\r\nTrailer: X-T\r\nUpgrade: h2c\r\nProxy-Connection: keep-alive\r\nProxy-Authorization: Basic Zm9vOmJhcg==\r\nProxy-Id: forged\r\nProxy-Flow: forged/1\r\nProxy-Custom: x\r\nAccept-Encoding: gzip\r\nX-Keep: yes\r\n\r\n",
        origin.port()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let (head, rest) = read_head(&mut s).await;
    let body = read_to_end(&mut s, rest).await; // proxy closes after the terminal chunk
    assert_eq!(status_of(&head), 302, "{head}");
    assert_eq!(header_of(&head, "location").as_deref(), Some("/elsewhere"));
    assert_eq!(
        header_of(&head, "content-encoding").as_deref(),
        Some("gzip")
    );
    assert_eq!(
        header_of(&head, "transfer-encoding").as_deref(),
        Some("chunked")
    );
    assert_eq!(header_of(&head, "connection").as_deref(), Some("close"));
    assert_eq!(header_of(&head, "x-origin-keep").as_deref(), Some("1"));
    for gone in ["x-origin-hop", "keep-alive", "proxy-authenticate"] {
        assert!(
            !has_header(&head, gone),
            "{gone} leaked to the browser: {head}"
        );
    }
    assert_eq!(
        body,
        &b"4\r\n\x1f\x8b\x08\x00\r\n3;ext=1\r\nabc\r\n0\r\nX-Trailer: t\r\n\r\n"[..]
    );

    let (up_head, _) = seen.lock().unwrap()[0].clone();
    assert!(
        up_head.starts_with("GET /path?q=1 HTTP/1.1\r\n"),
        "{up_head}"
    );
    assert_eq!(
        header_of(&up_head, "host"),
        Some(format!("127.0.0.1:{}", origin.port()))
    );
    assert_eq!(header_of(&up_head, "connection").as_deref(), Some("close"));
    assert_eq!(header_of(&up_head, "x-keep").as_deref(), Some("yes"));
    assert_eq!(
        header_of(&up_head, "accept-encoding").as_deref(),
        Some("gzip")
    );
    for gone in ["x-hop", "keep-alive", "te", "trailer", "upgrade"] {
        assert!(
            !has_header(&up_head, gone),
            "{gone} reached the origin: {up_head}"
        );
    }
    assert!(
        !up_head
            .lines()
            .any(|l| l.to_ascii_lowercase().starts_with("proxy-")),
        "a Proxy-* header reached the origin: {up_head}"
    );
    let l = proxy
        .wait_log(|l| l.contains("access flow=1 "), Duration::from_secs(5))
        .unwrap();
    assert!(
        l.contains(&format!(
            "access flow=1 GET host=127.0.0.1 port={} decrypted=no chain=- status=302",
            origin.port()
        )),
        "{l}"
    );
}

/// Request bodies stream to the origin with their framing (Content-Length and chunked).
#[tokio::test(flavor = "multi_thread")]
async fn plain_http_request_bodies() {
    const RESP: &[u8] = b"HTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\nok";
    let (origin, seen) = recording_origin(RESP).await;
    let proxy = Proxy::start(&[]);
    for (framing, body) in [
        ("Content-Length: 11", &b"hello world"[..]),
        (
            "Transfer-Encoding: chunked",
            &b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"[..],
        ),
    ] {
        let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
        let req = format!(
            "POST http://localhost:{}/up HTTP/1.1\r\nHost: localhost\r\n{framing}\r\n\r\n",
            origin.port()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        // Send the body in two parts with a pause, to make sure it is streamed and not awaited as a whole.
        s.write_all(&body[..3]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        s.write_all(&body[3..]).await.unwrap();
        let (head, rest) = read_head(&mut s).await;
        assert_eq!(status_of(&head), 201, "{head}");
        assert_eq!(read_to_end(&mut s, rest).await, b"ok");
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].1, b"hello world");
    assert!(seen[0].0.contains("Content-Length: 11"));
    assert_eq!(seen[1].1, b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n");
}

/// A response without length runs until the origin closes; HEAD and 304 have no body; 100-continue is
/// relayed.
#[tokio::test(flavor = "multi_thread")]
async fn plain_http_response_framings() {
    let origin = serve("127.0.0.1:0", |mut s| async move {
        let (head, _) = read_head(&mut s).await;
        let path = head.split_whitespace().nth(1).unwrap().to_string();
        match path.as_str() {
            "/close" => {
                s.write_all(b"HTTP/1.0 200 OK\r\n\r\nuntil close")
                    .await
                    .unwrap();
                s.shutdown().await.unwrap();
            }
            "/head" => {
                s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n")
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            "/304" => {
                s.write_all(b"HTTP/1.1 304 Not Modified\r\nETag: \"x\"\r\n\r\n")
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            "/continue" => {
                s.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await.unwrap();
                let mut b = [0u8; 4];
                s.read_exact(&mut b).await.unwrap();
                s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n")
                    .await
                    .unwrap();
                s.write_all(&b).await.unwrap();
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            _ => unreachable!(),
        }
    })
    .await;
    let proxy = Proxy::start(&[]);
    let p = origin.port();
    let cases: [(&str, String, &[u8]); 4] = [
        (
            "/close",
            format!("GET http://127.0.0.1:{p}/close HTTP/1.1\r\n\r\n"),
            b"until close",
        ),
        (
            "/head",
            format!("HEAD http://127.0.0.1:{p}/head HTTP/1.1\r\n\r\n"),
            b"",
        ),
        (
            "/304",
            format!("GET http://127.0.0.1:{p}/304 HTTP/1.1\r\n\r\n"),
            b"",
        ),
        (
            "/continue",
            format!(
                "PUT http://127.0.0.1:{p}/continue HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: 4\r\n\r\n"
            ),
            b"",
        ),
    ];
    for (path, req, want) in cases {
        let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        let (head, rest) = read_head(&mut s).await;
        if path == "/continue" {
            assert_eq!(status_of(&head), 100, "{head}");
            // The origin sends its final head only after it got the body, so nothing follows yet.
            assert!(rest.is_empty());
            s.write_all(b"data").await.unwrap();
            let (head, rest) = read_head(&mut s).await;
            assert_eq!(status_of(&head), 200, "{head}");
            assert_eq!(read_to_end(&mut s, rest).await, b"data");
            continue;
        }
        assert_eq!(
            header_of(&head, "connection").as_deref(),
            Some("close"),
            "{path}: {head}"
        );
        assert_eq!(read_to_end(&mut s, rest).await, want, "{path}");
    }
}

/// Requests the proxy refuses itself.
#[tokio::test(flavor = "multi_thread")]
async fn plain_http_refusals() {
    let proxy = Proxy::start(&[]);
    for (req, code) in [
        ("GET / HTTP/1.1\r\nHost: x\r\n\r\n", 400),
        ("GET https://example.test/ HTTP/1.1\r\n\r\n", 400),
        ("GET http://user:pw@example.test/ HTTP/1.1\r\n\r\n", 400),
        ("GET http://exa mple/ HTTP/1.1\r\n\r\n", 400),
        (
            "POST http://127.0.0.1:9/ HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\n",
            400,
        ),
        ("garbage\r\n\r\n", 400),
    ] {
        let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        let (head, _) = read_head(&mut s).await;
        assert_eq!(status_of(&head), code, "{req:?}: {head}");
    }
}

/// SPEC upstream errors for plain http: refused -> 502.
#[tokio::test(flavor = "multi_thread")]
async fn plain_http_refused_is_502() {
    let proxy = Proxy::start(&[]);
    let port = refused_port();
    let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
    s.write_all(format!("GET http://127.0.0.1:{port}/ HTTP/1.1\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let (head, _) = read_head(&mut s).await;
    assert_eq!(status_of(&head), 502, "{head}");
    let l = proxy
        .wait_log(|l| l.contains("upstream error"), Duration::from_secs(5))
        .unwrap();
    assert!(
        l.contains(&format!(
            "flow=1 host=127.0.0.1 port={port} upstream error, answering 502"
        )),
        "{l}"
    );
}

/// An origin that sends garbage instead of a response head -> 502.
#[tokio::test(flavor = "multi_thread")]
async fn plain_http_bad_origin_is_502() {
    let origin = serve("127.0.0.1:0", |mut s| async move {
        let _ = read_head(&mut s).await;
        s.write_all(b"NOT HTTP AT ALL\r\n\r\n").await.unwrap();
    })
    .await;
    let proxy = Proxy::start(&[]);
    let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
    s.write_all(format!("GET http://127.0.0.1:{}/ HTTP/1.1\r\n\r\n", origin.port()).as_bytes())
        .await
        .unwrap();
    let (head, _) = read_head(&mut s).await;
    assert_eq!(status_of(&head), 502, "{head}");
}

/// An absolute-form ws:// style upgrade (GET http://... with Upgrade: websocket) becomes a byte pipe
/// after 101.
#[tokio::test(flavor = "multi_thread")]
async fn plain_http_websocket_upgrade() {
    let ws = ws_echo_server().await;
    let proxy = Proxy::start(&[]);
    let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
    let target = format!("http://127.0.0.1:{}/chat", ws.port());
    ws_echo_session(&mut s, &target, &format!("127.0.0.1:{}", ws.port())).await;
    drop(s);
    let l = proxy
        .wait_log(|l| l.contains("access flow=1 "), Duration::from_secs(5))
        .unwrap();
    assert!(l.contains("status=101"), "{l}");
}
