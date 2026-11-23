//! W2-1: CONNECT tunnels with no filters (SPEC acceptance criteria that apply to this step).

mod common;

use std::time::{Duration, Instant};

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio_rustls::rustls::pki_types::ServerName;

/// SPEC: "With no filters, `curl -x 127.0.0.1:8080 https://www.example.test` succeeds and no CA file exists."
/// (Here with a Rust TLS client and `localhost`; the proxy resolves names itself and a test cannot
/// make www.example.test resolve without changing /etc/hosts.)
#[tokio::test(flavor = "multi_thread")]
async fn tls_through_tunnel_and_no_ca() {
    let pki = pki();
    let server = tls_hello_server(pki.server.clone()).await;
    let proxy = Proxy::start(&[]);
    let target = format!("localhost:{}", server.port());
    let (s, head, rest) = connect_via(proxy.addr(), &target).await;
    assert_eq!(status_of(&head), 200, "{head}");
    assert!(rest.is_empty());
    let connector = tokio_rustls::TlsConnector::from(pki.client.clone());
    let mut tls = connector
        .connect(ServerName::try_from("localhost").unwrap(), s)
        .await
        .unwrap();
    tls.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let (h, rest) = read_head(&mut tls).await;
    assert_eq!(status_of(&h), 200);
    let body = read_to_end(&mut tls, rest).await;
    assert_eq!(body, b"hello over tls");
    drop(tls);
    let line = proxy
        .wait_log(
            |l| l.contains("access flow=") && l.contains(&format!("port={}", server.port())),
            Duration::from_secs(5),
        )
        .expect("access log line");
    assert!(line.contains("CONNECT host=localhost"), "{line}");
    assert!(
        line.contains("decrypted=no chain=- status=tunnel"),
        "{line}"
    );
    assert!(
        files_under(proxy.dir.path()).is_empty(),
        "files appeared: {:?}",
        files_under(proxy.dir.path())
    );
}

/// The same with the real curl binary, if installed.
#[tokio::test(flavor = "multi_thread")]
async fn curl_through_tunnel() {
    if std::process::Command::new("curl")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("SKIP: no curl");
        return;
    }
    let pki = pki();
    let server = tls_hello_server(pki.server.clone()).await;
    let proxy = Proxy::start(&[]);
    let ca = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(ca.path(), &pki.ca_pem).unwrap();
    let out = tokio::process::Command::new("curl")
        .args(["-sS", "--fail", "--max-time", "10", "--noproxy", "", "-x"])
        .arg(format!("http://{}", proxy.addr()))
        .arg("--cacert")
        .arg(ca.path())
        .arg(format!("https://localhost:{}/", server.port()))
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "curl failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"hello over tls");
    assert!(
        proxy
            .wait_log(|l| l.contains("status=tunnel"), Duration::from_secs(5))
            .is_some()
    );
    assert!(files_under(&proxy.data_dir()).is_empty());
}

/// Bytes flow both ways, including bytes sent right behind the CONNECT head; closing propagates.
#[tokio::test(flavor = "multi_thread")]
async fn tunnel_copies_both_ways_and_early_bytes() {
    let echo = echo_server("127.0.0.1:0").await;
    let proxy = Proxy::start(&[]);
    let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
    s.write_all(format!("CONNECT 127.0.0.1:{} HTTP/1.1\r\n\r\nearly", echo.port()).as_bytes())
        .await
        .unwrap();
    let (head, mut rest) = read_head(&mut s).await;
    assert_eq!(status_of(&head), 200);
    s.write_all(b" and late").await.unwrap();
    s.shutdown().await.unwrap();
    let got = read_to_end(&mut s, std::mem::take(&mut rest)).await;
    assert_eq!(got, b"early and late");
    let line = proxy
        .wait_log(|l| l.contains("access flow="), Duration::from_secs(5))
        .unwrap();
    assert!(line.contains("status=tunnel up=14 down=53"), "{line}"); // 39-byte 200 head + 14 echoed
}

/// SPEC: `CONNECT [::1]:443` parses as host ::1, port 443.
#[tokio::test(flavor = "multi_thread")]
async fn connect_ipv6_literal() {
    if TcpListener::bind("[::1]:0").await.is_err() {
        eprintln!("SKIP: no IPv6 loopback");
        return;
    }
    let echo = echo_server("[::1]:0").await;
    let proxy = Proxy::start(&[]);
    let (mut s, head, _) = connect_via(proxy.addr(), &format!("[::1]:{}", echo.port())).await;
    assert_eq!(status_of(&head), 200, "{head}");
    s.write_all(b"v6").await.unwrap();
    let mut b = [0u8; 2];
    s.read_exact(&mut b).await.unwrap();
    assert_eq!(&b, b"v6");
    drop(s);
    let line = proxy
        .wait_log(|l| l.contains("access flow="), Duration::from_secs(5))
        .unwrap();
    assert!(
        line.contains(&format!("host=::1 port={}", echo.port())),
        "{line}"
    );
}

/// SPEC: `WWW.Example.Test`, `www.example.test.` and a Unicode name are normalised (lowercase, no
/// trailing dot, punycode) before logging and (later) matching.
#[tokio::test(flavor = "multi_thread")]
async fn connect_host_normalisation() {
    let echo = echo_server("127.0.0.1:0").await;
    let proxy = Proxy::start(&[]);
    let (s, head, _) = connect_via(proxy.addr(), &format!("LocalHost.:{}", echo.port())).await;
    assert_eq!(status_of(&head), 200, "{head}");
    drop(s);
    let line = proxy
        .wait_log(|l| l.contains("access flow=1 "), Duration::from_secs(5))
        .unwrap();
    assert!(
        line.contains(&format!("host=localhost port={}", echo.port())),
        "{line}"
    );

    let (_s, head, _) = connect_via(proxy.addr(), "BÜCHER.invalid.:443").await;
    assert_eq!(status_of(&head), 502, "{head}");
    let line = proxy
        .wait_log(|l| l.contains("access flow=2 "), Duration::from_secs(20))
        .unwrap();
    assert!(
        line.contains("host=xn--bcher-kva.invalid port=443"),
        "{line}"
    );

    for bad in ["no-port.test", "[::1", "a b:443", "host:0", "::1:443"] {
        let (_s, head, _) = connect_via(proxy.addr(), bad).await;
        assert_eq!(status_of(&head), 400, "{bad}: {head}");
    }
}

/// SPEC: DNS failure or refused connection -> 502, no connection within --dial-timeout -> 504, each
/// logged with flow id, host and cause.
#[tokio::test(flavor = "multi_thread")]
async fn upstream_errors() {
    let proxy = Proxy::start(&["--dial-timeout", "1s"]);
    let port = refused_port();
    let (mut s, head, rest) = connect_via(proxy.addr(), &format!("127.0.0.1:{port}")).await;
    assert_eq!(status_of(&head), 502, "{head}");
    assert_eq!(header_of(&head, "connection").as_deref(), Some("close"));
    let body = read_to_end(&mut s, rest).await;
    assert!(
        String::from_utf8_lossy(&body).contains("refused"),
        "{}",
        String::from_utf8_lossy(&body)
    );
    let l = proxy
        .wait_log(|l| l.contains("upstream error"), Duration::from_secs(5))
        .unwrap();
    assert!(
        l.contains(&format!(
            "flow=1 host=127.0.0.1 port={port} upstream error, answering 502: connect:"
        )),
        "{l}"
    );
    assert!(l.to_lowercase().contains("refused"), "{l}");

    let (_s, head, _) = connect_via(proxy.addr(), "does-not-exist.invalid:443").await;
    assert_eq!(status_of(&head), 502, "{head}");
    let l = proxy
        .wait_log(
            |l| l.contains("flow=2 ") && l.contains("upstream error"),
            Duration::from_secs(20),
        )
        .unwrap();
    assert!(
        l.contains("host=does-not-exist.invalid port=443 upstream error, answering 502: dns:"),
        "{l}"
    );

    match blackhole() {
        None => eprintln!("SKIP 504 part: could not fill an accept queue"),
        Some(bh) => {
            let t = Instant::now();
            let (_s, head, _) = connect_via(proxy.addr(), &bh.addr.to_string()).await;
            let took = t.elapsed();
            assert_eq!(status_of(&head), 504, "{head}");
            assert!(
                took >= Duration::from_millis(900) && took < Duration::from_secs(5),
                "{took:?}"
            );
            let l = proxy
                .wait_log(
                    |l| l.contains("flow=3 ") && l.contains("upstream error"),
                    Duration::from_secs(5),
                )
                .unwrap();
            assert!(
                l.contains("answering 504: dial timeout after 1000ms"),
                "{l}"
            );
        }
    }
    let l = proxy
        .wait_log(|l| l.contains("access flow=1 "), Duration::from_secs(5))
        .unwrap();
    assert!(l.contains("status=502"), "{l}");
}

/// SPEC acceptance: a WebSocket echo works (here: through the plain tunnel, as browsers send ws:// via
/// CONNECT to an HTTP proxy).
#[tokio::test(flavor = "multi_thread")]
async fn websocket_echo_through_tunnel() {
    let ws = ws_echo_server().await;
    let proxy = Proxy::start(&[]);
    let (mut s, head, rest) = connect_via(proxy.addr(), &format!("127.0.0.1:{}", ws.port())).await;
    assert_eq!(status_of(&head), 200);
    assert!(rest.is_empty());
    ws_echo_session(&mut s, "/chat", &format!("127.0.0.1:{}", ws.port())).await;
}

/// `--listen unix:<path>`: socket mode 0600, CONNECT works through it.
#[tokio::test(flavor = "multi_thread")]
async fn unix_listener() {
    use std::os::unix::fs::PermissionsExt;
    let echo = echo_server("127.0.0.1:0").await;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("wire2.sock");
    let listen = format!("unix:{}", sock.display());
    let proxy = Proxy::start(&["--listen", &listen]);
    assert_eq!(
        std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let mut s = UnixStream::connect(&sock).await.unwrap();
    s.write_all(format!("CONNECT 127.0.0.1:{} HTTP/1.1\r\n\r\n", echo.port()).as_bytes())
        .await
        .unwrap();
    let (head, _) = read_head(&mut s).await;
    assert_eq!(status_of(&head), 200);
    s.write_all(b"over unix").await.unwrap();
    let mut b = [0u8; 9];
    s.read_exact(&mut b).await.unwrap();
    assert_eq!(&b, b"over unix");
    drop(proxy);
}

/// SIGUSR1 dumps the registry (flow, host, age, bytes) to the log.
#[tokio::test(flavor = "multi_thread")]
async fn sigusr1_dumps_registry() {
    let echo = echo_server("127.0.0.1:0").await;
    let proxy = Proxy::start(&[]);
    let (mut s, head, _) = connect_via(proxy.addr(), &format!("localhost:{}", echo.port())).await;
    assert_eq!(status_of(&head), 200);
    s.write_all(b"abc").await.unwrap();
    let mut b = [0u8; 3];
    s.read_exact(&mut b).await.unwrap();
    proxy.signal(libc::SIGUSR1);
    let l = proxy
        .wait_log(|l| l.contains("registry flow="), Duration::from_secs(5))
        .expect("dump");
    assert!(
        l.contains(&format!(
            "registry flow=1 CONNECT host=localhost port={}",
            echo.port()
        )),
        "{l}"
    );
    assert!(l.contains(" age=") && l.contains(" up=3 down=42"), "{l}");
    assert!(proxy.log().contains("registry: 1 open flow(s)"));
    drop(s);
}

/// When the upstream closes and the browser stays idle, the proxy closes the rest of the flow after the
/// half-close grace (5 s), delivering everything first.
#[tokio::test(flavor = "multi_thread")]
async fn upstream_close_ends_flow() {
    let origin = serve("127.0.0.1:0", |mut s| async move {
        s.write_all(b"bye").await.unwrap();
    })
    .await;
    let proxy = Proxy::start(&[]);
    let t = Instant::now();
    let (mut s, head, rest) = connect_via(proxy.addr(), &origin.to_string()).await;
    assert_eq!(status_of(&head), 200);
    // EOF arrives right away (the FIN is passed on) while our socket stays open.
    assert_eq!(read_to_end(&mut s, rest).await, b"bye");
    assert!(t.elapsed() < Duration::from_secs(3));
    let l = proxy
        .wait_log(|l| l.contains("access flow=1 "), Duration::from_secs(10))
        .expect("flow closed");
    let took = t.elapsed();
    assert!(
        took >= Duration::from_secs(4) && took < Duration::from_secs(9),
        "{took:?}"
    );
    assert!(l.contains("status=tunnel up=0 down=42"), "{l}");
}
