//! W2-3: WebSockets inside decrypted tunnels. The upgrade request goes through the single
//! `proxy::forward` path; after `101` the connection is a plain byte copy (no frame parsing).
//! Decryption is forced with the TEST-ONLY `--test-decrypt`, names mapped with `--test-resolve`.
//! (ws:// through plain http and wss:// through an opaque tunnel live in w2_1_http / w2_1_tunnel.)

mod common;

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls;

const HOST: &str = "www.example.test";
const MIB: usize = 1024 * 1024;

#[derive(Clone, Default)]
struct Seen {
    /// Request heads the origin received, one per connection.
    heads: Arc<Mutex<Vec<String>>>,
    conns: Arc<AtomicU64>,
}

/// wss origin. Paths:
///   /chat    -> 101 (with Sec-WebSocket-Protocol), echo text/binary, answer a close frame and end
///   /bye     -> 101, then closes itself (close frame, then the socket) after the first message
///   /deny    -> 401 with a body, `Connection: close`
///   /plain   -> 200 with a body
async fn origin(cfg: Arc<rustls::ServerConfig>, seen: Seen) -> std::net::SocketAddr {
    let acceptor = tokio_rustls::TlsAcceptor::from(cfg);
    serve("127.0.0.1:0", move |s| {
        let (acceptor, seen) = (acceptor.clone(), seen.clone());
        async move {
            let Ok(mut s) = acceptor.accept(s).await else {
                return;
            };
            seen.conns.fetch_add(1, Ordering::SeqCst);
            let (head, _) = read_head(&mut s).await;
            seen.heads.lock().unwrap().push(head.clone());
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            match path.as_str() {
                "/deny" => {
                    let _ = s
                        .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 6\r\nConnection: close\r\n\r\nnope!\n")
                        .await;
                    let _ = s.shutdown().await;
                }
                "/plain" => {
                    let _ = s
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nhey")
                        .await;
                    let _ = s.shutdown().await;
                }
                _ => {
                    let key = header_of(&head, "sec-websocket-key").unwrap_or_default();
                    let resp = format!(
                        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: chat\r\nX-Origin: yes\r\n\r\n",
                        ws_accept(&key)
                    );
                    s.write_all(resp.as_bytes()).await.unwrap();
                    while let Some((op, p)) = ws_read(&mut s).await {
                        if op == 8 {
                            ws_write(&mut s, 8, &p, None).await;
                            let _ = s.shutdown().await;
                            return;
                        }
                        ws_write(&mut s, op, &p, None).await;
                        if path == "/bye" {
                            ws_write(&mut s, 8, &[0x03, 0xe8], None).await;
                            let _ = s.shutdown().await;
                            return;
                        }
                    }
                }
            }
        }
    })
    .await
}

fn proxy(roots: &Path) -> Proxy {
    Proxy::start_env(
        &[
            "--test-resolve",
            "*.example.test=127.0.0.1",
            "--test-decrypt",
            "*.example.test",
        ],
        &[("SSL_CERT_FILE", roots)],
    )
}

type Tls = tokio_rustls::client::TlsStream<tokio::net::TcpStream>;

async fn client(p: &Proxy, port: u16) -> (Tls, String) {
    let target = format!("{HOST}:{port}");
    let tls = tls_via(p.addr(), &target, HOST, client_trusting(&ca_pem(p)))
        .await
        .expect("minted leaf chains to the proxy CA");
    (tls, target)
}

const KEY: &str = "d2lyZTItdGVzdC1rZXkhIQ==";

fn upgrade_req(path: &str, target: &str) -> String {
    format!(
        "GET {path} HTTP/1.1\r\nHost: {target}\r\nUpgrade: websocket\r\nConnection: keep-alive, Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: chat, superchat\r\nSec-WebSocket-Extensions: permessage-deflate\r\nOrigin: https://{HOST}\r\nProxy-Flow: forged/1\r\n\r\n"
    )
}

async fn handshake(tls: &mut Tls, path: &str, target: &str) -> String {
    tls.write_all(upgrade_req(path, target).as_bytes())
        .await
        .unwrap();
    let (head, rest) = read_head(tls).await;
    assert_eq!(status_of(&head), 101, "{head}");
    assert!(rest.is_empty());
    head
}

fn field(line: &str, key: &str) -> u64 {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key))
        .unwrap()
        .parse()
        .unwrap()
}

/// Handshake headers, text and binary echoes (small, medium, 200 kB), client-initiated close ends the
/// flow, the log has 101 with both byte counts, and the connection is not reused afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn decrypted_websocket_echo_and_close() {
    let pki = pki();
    let roots = root_file(&pki);
    let seen = Seen::default();
    let server = origin(pki.server.clone(), seen.clone()).await;
    let p = proxy(roots.path());
    let (mut tls, target) = client(&p, server.port()).await;

    // A normal request first, on the same keep-alive connection: the upgrade may follow it.
    tls.write_all(format!("GET /plain HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut b = vec![0u8; 256];
    let mut got = Vec::new();
    while !got.ends_with(b"hey") {
        let n = tls.read(&mut b).await.unwrap();
        assert!(n > 0);
        got.extend_from_slice(&b[..n]);
    }

    let head = handshake(&mut tls, "/chat", &target).await;
    assert_eq!(
        header_of(&head, "sec-websocket-accept").as_deref(),
        Some(ws_accept(KEY).as_str())
    );
    assert_eq!(
        header_of(&head, "sec-websocket-protocol").as_deref(),
        Some("chat")
    );
    assert_eq!(header_of(&head, "upgrade").as_deref(), Some("websocket"));
    assert!(
        header_of(&head, "connection")
            .unwrap()
            .eq_ignore_ascii_case("upgrade"),
        "{head}"
    );
    assert_eq!(header_of(&head, "x-origin").as_deref(), Some("yes"));
    {
        // What the origin saw: the upgrade kept, Sec-WebSocket-* intact, Proxy-* gone, Host from tunnel.
        let heads = seen.heads.lock().unwrap();
        let h = &heads[1];
        assert!(h.starts_with("GET /chat HTTP/1.1\r\n"), "{h}");
        assert_eq!(header_of(h, "upgrade").as_deref(), Some("websocket"));
        assert!(
            header_of(h, "connection")
                .unwrap()
                .eq_ignore_ascii_case("upgrade"),
            "{h}"
        );
        assert_eq!(header_of(h, "sec-websocket-key").as_deref(), Some(KEY));
        assert_eq!(header_of(h, "sec-websocket-version").as_deref(), Some("13"));
        assert_eq!(
            header_of(h, "sec-websocket-protocol").as_deref(),
            Some("chat, superchat")
        );
        assert_eq!(
            header_of(h, "sec-websocket-extensions").as_deref(),
            Some("permessage-deflate")
        );
        assert_eq!(header_of(h, "host").unwrap(), target);
        assert!(!has_header(h, "proxy-flow"), "{h}");
    }

    let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    for (op, msg) in [
        (1u8, b"hello wss".to_vec()),
        (2, vec![0u8, 255, 7, 13, 10, 13, 10]),
        (2, big),
    ] {
        ws_write(&mut tls, op, &msg, Some([1, 2, 3, 4])).await;
        let (rop, echoed) = tokio::time::timeout(Duration::from_secs(10), ws_read(&mut tls))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rop, op);
        assert!(echoed == msg, "echo differs");
    }
    ws_write(&mut tls, 8, &[0x03, 0xe8], Some([9, 9, 9, 9])).await;
    let (op, _) = ws_read(&mut tls).await.unwrap();
    assert_eq!(op, 8);
    // The origin closed after its close frame: the proxy ends the flow, the browser sees EOF.
    let mut tail = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), tls.read_to_end(&mut tail))
        .await
        .expect("flow did not end after close");
    assert!(tail.is_empty());

    // Not reused: another request on this connection gets no HTTP answer, and the origin never saw a
    // third connection.
    let _ = tls
        .write_all(format!("GET /plain HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await;
    let mut tail = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(2), tls.read_to_end(&mut tail)).await;
    assert!(tail.is_empty(), "answered on an upgraded connection");
    assert_eq!(seen.conns.load(Ordering::SeqCst), 2);
    // Like a browser, close our side once the peer is gone (otherwise the proxy waits its 5 s
    // half-close grace before ending the flow).
    drop(tls);

    let l = p
        .wait_log(|l| l.contains("req=2 GET"), Duration::from_secs(10))
        .unwrap();
    assert!(
        l.contains(&format!("https://{target}/chat status=101")),
        "{l}"
    );
    let (up, down) = (field(&l, "up="), field(&l, "down="));
    // 200 kB each way is the bulk (plus heads and the small frames).
    assert!(up > 200_000 && down > 200_000, "{l}");
    let f = p
        .wait_log(
            |l| l.contains("access flow=1 CONNECT"),
            Duration::from_secs(5),
        )
        .unwrap();
    assert!(f.contains("decrypted=yes"), "{f}");
    assert_eq!(p.log().matches("access flow=1 CONNECT").count(), 1);
}

/// The origin closes first: the browser gets the echo and the close frame, then the flow is over.
#[tokio::test(flavor = "multi_thread")]
async fn server_initiated_close_ends_flow() {
    let pki = pki();
    let roots = root_file(&pki);
    let server = origin(pki.server.clone(), Seen::default()).await;
    let p = proxy(roots.path());
    let (mut tls, target) = client(&p, server.port()).await;
    handshake(&mut tls, "/bye", &target).await;
    ws_write(&mut tls, 1, b"ping", Some([5, 6, 7, 8])).await;
    assert_eq!(ws_read(&mut tls).await.unwrap(), (1, b"ping".to_vec()));
    assert_eq!(ws_read(&mut tls).await.unwrap().0, 8);
    // The browser does not answer; the flow still ends because the origin hung up.
    let mut tail = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), tls.read_to_end(&mut tail))
        .await
        .expect("flow did not end after the origin closed");
    drop(tls); // like a browser: close our side, else the proxy waits its 5 s half-close grace
    let l = p
        .wait_log(|l| l.contains("req=1 GET"), Duration::from_secs(5))
        .unwrap();
    assert!(l.contains("status=101"), "{l}");
    assert!(field(&l, "down=") > 0 && field(&l, "up=") > 0, "{l}");
    p.wait_log(
        |l| l.contains("access flow=1 CONNECT"),
        Duration::from_secs(5),
    )
    .expect("flow line");
}

/// The browser drops the connection mid-session: the flow line appears.
#[tokio::test(flavor = "multi_thread")]
async fn browser_hangup_ends_flow() {
    let pki = pki();
    let roots = root_file(&pki);
    let server = origin(pki.server.clone(), Seen::default()).await;
    let p = proxy(roots.path());
    let (mut tls, target) = client(&p, server.port()).await;
    handshake(&mut tls, "/chat", &target).await;
    ws_write(&mut tls, 1, b"x", Some([1, 1, 1, 1])).await;
    assert_eq!(ws_read(&mut tls).await.unwrap().0, 1);
    drop(tls);
    let f = p
        .wait_log(
            |l| l.contains("access flow=1 CONNECT"),
            Duration::from_secs(5),
        )
        .expect("flow line after the browser left");
    assert!(f.contains("decrypted=yes"), "{f}");
}

/// A non-101 answer to an upgrade request is a normal response: status, body and framing pass, the
/// browser gets `Connection: close`, and nothing turns into a byte pipe.
#[tokio::test(flavor = "multi_thread")]
async fn non_101_answer_is_a_normal_response() {
    let pki = pki();
    let roots = root_file(&pki);
    let server = origin(pki.server.clone(), Seen::default()).await;
    let p = proxy(roots.path());
    let (mut tls, target) = client(&p, server.port()).await;
    tls.write_all(upgrade_req("/deny", &target).as_bytes())
        .await
        .unwrap();
    let (head, mut rest) = read_head(&mut tls).await;
    assert_eq!(status_of(&head), 401, "{head}");
    assert!(!has_header(&head, "upgrade"), "{head}");
    while rest.len() < 6 {
        let mut b = [0u8; 64];
        let n = tls.read(&mut b).await.unwrap();
        assert!(n > 0);
        rest.extend_from_slice(&b[..n]);
    }
    assert_eq!(rest, b"nope!\n");
    let mut tail = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), tls.read_to_end(&mut tail)).await;
    assert!(tail.is_empty());
    let l = p
        .wait_log(|l| l.contains("req=1 GET"), Duration::from_secs(5))
        .unwrap();
    assert!(l.contains("status=401"), "{l}");
}

/// Several MiB in one message each way, echoed through the decrypted pipe, with flat proxy memory.
#[tokio::test(flavor = "multi_thread")]
async fn large_message_flat_memory() {
    let pki = pki();
    let roots = root_file(&pki);
    let server = origin(pki.server.clone(), Seen::default()).await;
    let p = proxy(roots.path());
    let (mut tls, target) = client(&p, server.port()).await;
    handshake(&mut tls, "/chat", &target).await;
    // Warm up so buffers and pipes exist before the baseline.
    ws_write(&mut tls, 2, &[1u8; 1000], Some([1, 2, 3, 4])).await;
    ws_read(&mut tls).await.unwrap();
    let base = p.rss_kb();
    let msg: Vec<u8> = (0..32 * MIB).map(|i| (i % 253) as u8).collect();
    let mut max = base;
    let pid = p.pid();
    for _ in 0..2 {
        let (mut r, mut w) = tokio::io::split(&mut tls);
        let writer = async {
            ws_write(&mut w, 2, &msg, Some([4, 3, 2, 1])).await;
            w.flush().await.unwrap();
        };
        let reader = async {
            let mut m = base;
            let mut read = Box::pin(ws_read(&mut r));
            let got = loop {
                // Sample while the message is in flight.
                m = m.max(rss_kb(pid));
                tokio::select! {
                    g = &mut read => break g,
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                }
            };
            (got, m)
        };
        let (_, (got, m)) = tokio::time::timeout(Duration::from_secs(60), async {
            tokio::join!(writer, reader)
        })
        .await
        .unwrap_or_else(|_| {
            p.signal(libc::SIGUSR1);
            std::thread::sleep(Duration::from_millis(200));
            panic!("large message stalled:\n{}", p.log())
        });
        let (op, echoed) = got.unwrap();
        assert_eq!(op, 2);
        assert!(echoed == msg, "large echo differs");
        max = max.max(m);
    }
    eprintln!("ws large: 32 MiB x2 echoed, proxy RSS {base} -> max {max} KiB");
    assert!(
        max.saturating_sub(base) < 16 * 1024,
        "proxy RSS grew {base} -> {max} KiB"
    );
}
