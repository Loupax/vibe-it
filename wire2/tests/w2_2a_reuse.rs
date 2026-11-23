//! W2-2a: upstream connections are reused inside decrypted tunnels. The origin counts its TCP
//! connections; the browser side is one decrypted connection sending several requests.
//! Names map to 127.0.0.1 with the TEST-ONLY `--test-resolve`.

mod common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls;

const HOST: &str = "www.example.test";
const RESOLVE: &[&str] = &["--test-resolve", "*.test=127.0.0.1"];

#[derive(Clone, Default)]
struct Counts {
    /// Accepted TCP connections.
    conns: Arc<AtomicUsize>,
    /// (connection index, request line, body length) of every request the origin read.
    reqs: Arc<Mutex<Vec<(usize, String, usize)>>>,
}

impl Counts {
    fn conns(&self) -> usize {
        self.conns.load(Ordering::SeqCst)
    }
    fn seen(&self, prefix: &str) -> usize {
        self.reqs
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.1.starts_with(prefix))
            .count()
    }
}

/// A keep-alive TLS origin. By path:
/// `/close` answers with `Connection: close` and closes; `/silent` answers, then closes without a
/// header; `/flaky` is answered on a connection's first request, but on a later one the origin reads
/// it and closes without answering (a reused connection that died); `/chunked` answers chunked;
/// anything else `ok <path>`. Request bodies (Content-Length or chunked) are read; a POST/PUT is
/// answered with `got <n>`.
async fn origin(cfg: Arc<rustls::ServerConfig>) -> (SocketAddr, Counts) {
    let c = Counts::default();
    let c2 = c.clone();
    let acceptor = tokio_rustls::TlsAcceptor::from(cfg);
    let addr = serve("127.0.0.1:0", move |s| {
        let (c, acceptor) = (c2.clone(), acceptor.clone());
        async move {
            let idx = c.conns.fetch_add(1, Ordering::SeqCst);
            let Ok(mut t) = acceptor.accept(s).await else {
                return;
            };
            let mut pre: Vec<u8> = Vec::new();
            let mut n = 0;
            loop {
                let Some((head, rest)) = read_head_from(&mut t, std::mem::take(&mut pre)).await
                else {
                    return;
                };
                n += 1;
                let Some((body, rest)) = read_body(&mut t, &head, rest).await else {
                    return;
                };
                pre = rest;
                let first = head.lines().next().unwrap().to_string();
                let path = first.split_whitespace().nth(1).unwrap().to_string();
                c.reqs.lock().unwrap().push((idx, first, body.len()));
                let out: Vec<u8> = match path.as_str() {
                    "/flaky" if n > 1 => return,
                    "/close" => b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 5\r\n\r\nclose".to_vec(),
                    "/silent" => {
                        let _ = t
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsilent")
                            .await;
                        let _ = t.shutdown().await;
                        return;
                    }
                    "/chunked" => b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nhello \r\n5;e=1\r\nworld\r\n0\r\n\r\n".to_vec(),
                    _ => {
                        let b = if head.starts_with("HEAD") {
                            let _ = t.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n").await;
                            continue;
                        } else if head.starts_with("POST") || head.starts_with("PUT") {
                            format!("got {}", body.len())
                        } else {
                            format!("ok {path}")
                        };
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{b}", b.len())
                            .into_bytes()
                    }
                };
                if t.write_all(&out).await.is_err() || t.flush().await.is_err() {
                    return;
                }
            }
        }
    })
    .await;
    (addr, c)
}

async fn read_head_from<R: AsyncRead + Unpin>(
    r: &mut R,
    mut buf: Vec<u8>,
) -> Option<(String, Vec<u8>)> {
    loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(i + 4);
            return Some((String::from_utf8_lossy(&buf).into_owned(), rest));
        }
        let mut tmp = [0u8; 4096];
        let n = r.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

/// The body that follows `head` (Content-Length or chunked; none for HEAD requests' responses is the
/// caller's business), and what came after it.
async fn read_body<R: AsyncRead + Unpin>(
    r: &mut R,
    head: &str,
    mut buf: Vec<u8>,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let chunked = head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");
    let len = header_of(head, "content-length").and_then(|v| v.parse::<usize>().ok());
    loop {
        if chunked {
            if let Some((body, used)) = dechunk(&buf) {
                return Some((body, buf.split_off(used)));
            }
        } else {
            let n = len.unwrap_or(0);
            if buf.len() >= n {
                let rest = buf.split_off(n);
                return Some((buf, rest));
            }
        }
        let mut tmp = [0u8; 8192];
        let n = r.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

/// (decoded body, bytes consumed) if `b` holds a whole chunked body.
fn dechunk(b: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    loop {
        let eol = b[pos..].windows(2).position(|w| w == b"\r\n")? + pos;
        let size = std::str::from_utf8(&b[pos..eol]).ok()?;
        let n = usize::from_str_radix(size.split(';').next()?.trim(), 16).ok()?;
        pos = eol + 2;
        if n == 0 {
            return (b.len() >= pos + 2).then_some((out, pos + 2));
        }
        if b.len() < pos + n + 2 {
            return None;
        }
        out.extend_from_slice(&b[pos..pos + n]);
        pos += n + 2;
    }
}

type Tls = tokio_rustls::client::TlsStream<tokio::net::TcpStream>;

/// A decrypted browser connection (the client trusts the proxy's CA).
struct Browser {
    t: Tls,
    buf: Vec<u8>,
    target: String,
}

impl Browser {
    async fn open(p: &Proxy, port: u16) -> Browser {
        let target = format!("{HOST}:{port}");
        let t = tls_via(p.addr(), &target, HOST, client_trusting(&ca_pem(p)))
            .await
            .unwrap();
        Browser {
            t,
            buf: Vec::new(),
            target,
        }
    }

    /// Send one request and read its whole response: (status, head, body).
    async fn req(
        &mut self,
        method: &str,
        path: &str,
        extra: &str,
        body: &[u8],
    ) -> (u16, String, Vec<u8>) {
        self.send(method, path, extra, body).await;
        self.recv().await.expect("a response")
    }

    async fn send(&mut self, method: &str, path: &str, extra: &str, body: &[u8]) {
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\n{extra}\r\n",
            self.target
        );
        self.t.write_all(head.as_bytes()).await.unwrap();
        self.t.write_all(body).await.unwrap();
        self.t.flush().await.unwrap();
    }

    async fn recv(&mut self) -> Option<(u16, String, Vec<u8>)> {
        let (head, rest) = read_head_from(&mut self.t, std::mem::take(&mut self.buf)).await?;
        let (body, rest) = read_body(&mut self.t, &head, rest).await?;
        self.buf = rest;
        Some((status_of(&head), head, body))
    }
}

fn proxy(roots: &std::path::Path) -> Decrypting {
    decrypting(&["*.example.test"], RESOLVE, &[("SSL_CERT_FILE", roots)])
}

#[tokio::test(flavor = "multi_thread")]
async fn requests_on_one_connection_reuse_one_upstream_connection() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let p = proxy(roots.path());
    let mut b = Browser::open(&p, addr.port()).await;
    for i in 0..5 {
        let (st, h, body) = b.req("GET", &format!("/n{i}"), "", b"").await;
        assert_eq!(st, 200, "{h}");
        assert_eq!(body, format!("ok /n{i}").as_bytes());
        assert!(
            !has_header(&h, "connection"),
            "browser side stays alive: {h}"
        );
    }
    let (_, h, body) = b.req("GET", "/chunked", "", b"").await;
    assert!(
        h.to_ascii_lowercase()
            .contains("transfer-encoding: chunked"),
        "{h}"
    );
    assert_eq!(body, b"hello world");
    let (_, _, body) = b.req("GET", "/last", "", b"").await;
    assert_eq!(body, b"ok /last");
    drop(b);
    assert_eq!(c.conns(), 1, "one upstream connection for all requests");
    assert_eq!(c.reqs.lock().unwrap().len(), 7);
    // The origin never saw `Connection: close` from the proxy.
    p.wait_log(|l| l.contains("decrypted=yes"), Duration::from_secs(5))
        .unwrap_or_else(|| panic!("{}", p.log()));
    let log = p.log();
    for n in 1..=7 {
        assert!(
            log.contains(&format!("req={n} ")),
            "one access line per request ({n}): {log}"
        );
    }
}

/// A HEAD response carries Content-Length but no body: the next response must still line up.
#[tokio::test(flavor = "multi_thread")]
async fn head_request_does_not_desync_the_kept_connection() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let p = proxy(roots.path());
    let mut b = Browser::open(&p, addr.port()).await;
    b.send("HEAD", "/h", "", b"").await;
    let (head, rest) = read_head_from(&mut b.t, Vec::new()).await.unwrap();
    assert_eq!(status_of(&head), 200);
    assert!(rest.is_empty());
    let (_, _, body) = b.req("GET", "/after", "", b"").await;
    assert_eq!(body, b"ok /after");
    assert_eq!(c.conns(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn bodies_content_length_and_chunked() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let p = proxy(roots.path());
    let mut b = Browser::open(&p, addr.port()).await;
    let big = vec![b'x'; 300_000];
    let cl = format!("Content-Length: {}\r\n", big.len());
    let (_, _, body) = b.req("POST", "/up", &cl, &big).await;
    assert_eq!(body, b"got 300000");
    let chunked = b"5\r\nhello\r\n6;x=1\r\n world\r\n0\r\n\r\n";
    let (_, _, body) = b
        .req("POST", "/up", "Transfer-Encoding: chunked\r\n", chunked)
        .await;
    assert_eq!(body, b"got 11");
    let (_, _, body) = b.req("PUT", "/up", "Content-Length: 3\r\n", b"abc").await;
    assert_eq!(body, b"got 3");
    let (_, _, body) = b.req("GET", "/chunked", "", b"").await;
    assert_eq!(body, b"hello world");
    let (_, _, body) = b.req("GET", "/z", "", b"").await;
    assert_eq!(body, b"ok /z");
    assert_eq!(c.conns(), 1);
}

/// The origin says `Connection: close` or closes the idle connection: the next request, GET or POST,
/// simply uses a new connection (nothing was sent on the dead one).
#[tokio::test(flavor = "multi_thread")]
async fn origin_close_between_requests_uses_a_new_connection() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let p = proxy(roots.path());
    let mut b = Browser::open(&p, addr.port()).await;
    let (_, h, body) = b.req("GET", "/close", "", b"").await;
    assert_eq!(body, b"close");
    assert!(
        !has_header(&h, "connection"),
        "the browser connection is independent: {h}"
    );
    let (_, _, body) = b.req("POST", "/p", "Content-Length: 2\r\n", b"hi").await;
    assert_eq!(body, b"got 2");
    assert_eq!(c.conns(), 2);
    // Silent close while idle (the proxy notices before sending anything).
    let (_, _, body) = b.req("GET", "/silent", "", b"").await;
    assert_eq!(body, b"silent");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, _, body) = b.req("POST", "/p", "Content-Length: 2\r\n", b"hi").await;
    assert_eq!(body, b"got 2");
    assert_eq!(c.conns(), 3, "{:?}", c.reqs.lock().unwrap());
}

/// The reused connection dies after the origin read the request (the race the idle check cannot
/// see): a GET is retried once on a fresh connection, a POST is answered 502 and not sent again.
#[tokio::test(flavor = "multi_thread")]
async fn stale_reused_connection_retries_get_never_post() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let p = proxy(roots.path());
    let mut b = Browser::open(&p, addr.port()).await;
    b.req("GET", "/first", "", b"").await;
    let (st, h, body) = b.req("GET", "/flaky", "", b"").await;
    assert_eq!(st, 200, "{h} {}", String::from_utf8_lossy(&body));
    assert_eq!(body, b"ok /flaky");
    assert_eq!(c.conns(), 2, "retried on a fresh connection");
    assert_eq!(c.seen("GET /flaky"), 2, "one read per attempt");
    // The fresh connection answered its first request and is parked; the next /flaky on it is its
    // second request there and gets dropped: a POST must not be resent.
    let (st, h, _) = b
        .req("POST", "/flaky", "Content-Length: 4\r\n", b"data")
        .await;
    assert_eq!(st, 502, "{h}");
    assert_eq!(c.seen("POST /flaky"), 1, "sent once, never again");
    assert_eq!(c.conns(), 2);
    // The dead connection was dropped; the browser connection (the 502 came through pass-decrypt as an
    // ordinary response) carries on with a new upstream connection.
    let (st, _, body) = b.req("GET", "/next", "", b"").await;
    assert_eq!((st, body), (200, b"ok /next".to_vec()));
    assert_eq!(c.conns(), 3);
}

fn connects(f: &Fake) -> usize {
    f.seen()
        .iter()
        .filter(|h| h.starts_with("CONNECT "))
        .count()
}

/// A tunnel filter sees one CONNECT per upstream connection, and closing that connection ends the
/// reused connection cleanly: the next request dials again, through the filter, which may refuse.
#[tokio::test(flavor = "multi_thread")]
async fn tunnel_filter_one_connect_and_can_revoke_the_reused_connection() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let d = sock_dir();
    let pd = pass_decrypt(&d.path().join("pd.sock"), &["*.example.test"]);
    let f = Fake::start(
        &d.path().join("gate.sock"),
        hello_json("gate", "tunnel", &["*.example.test"], "closed"),
        Action::CloseAfter(Duration::from_millis(1500)),
    );
    let p = proxy_with(
        &[&pd.path, &f.inproc.path],
        RESOLVE,
        &[("SSL_CERT_FILE", roots.path())],
    );
    wait_up(&p, "pass-decrypt");
    wait_up(&p, "gate");
    let mut b = Browser::open(&p, addr.port()).await;
    for i in 0..4 {
        let (st, h, _) = b.req("GET", &format!("/q{i}"), "", b"").await;
        assert_eq!(st, 200, "{h}");
    }
    assert_eq!(
        connects(&f),
        1,
        "one CONNECT for the reused connection: {:?}",
        f.seen()
    );
    assert_eq!(c.conns(), 1);
    // The filter closes the tunnel (a quota running out), then refuses the next CONNECT.
    tokio::time::sleep(Duration::from_millis(2200)).await;
    f.set(Action::Answer(
        403,
        "text/plain".into(),
        b"quota used".to_vec(),
    ));
    let (st, h, _) = b.req("GET", "/after", "", b"").await;
    assert_eq!(st, 403, "revoked, the new CONNECT is refused: {h}");
    assert_eq!(connects(&f), 2);
    assert_eq!(c.conns(), 1, "nothing reached the origin again");
}

/// The tunnel filter closes mid-session and lets the next CONNECT through: a transparent redial.
#[tokio::test(flavor = "multi_thread")]
async fn tunnel_filter_close_then_redial() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let d = sock_dir();
    let pd = pass_decrypt(&d.path().join("pd.sock"), &["*.example.test"]);
    let f = Fake::start(
        &d.path().join("gate.sock"),
        hello_json("gate", "tunnel", &["*.example.test"], "closed"),
        Action::CloseAfter(Duration::from_millis(700)),
    );
    let p = proxy_with(
        &[&pd.path, &f.inproc.path],
        RESOLVE,
        &[("SSL_CERT_FILE", roots.path())],
    );
    wait_up(&p, "pass-decrypt");
    wait_up(&p, "gate");
    let mut b = Browser::open(&p, addr.port()).await;
    b.req("GET", "/a", "", b"").await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let (st, h, body) = b.req("GET", "/b", "", b"").await;
    assert_eq!(st, 200, "{h}");
    assert_eq!(body, b"ok /b");
    assert_eq!(c.conns(), 2);
    assert_eq!(connects(&f), 2);
}

/// Plain http:// stays one request per browser connection, `Connection: close` both ways.
#[tokio::test(flavor = "multi_thread")]
async fn plain_http_still_closes() {
    let p = Proxy::start(RESOLVE);
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let s2 = seen.clone();
    let o = serve("127.0.0.1:0", move |mut s| {
        let seen = s2.clone();
        async move {
            let (h, _) = read_head(&mut s).await;
            seen.lock().unwrap().push(h);
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi")
                .await;
        }
    })
    .await;
    let mut s = tokio::net::TcpStream::connect(p.addr()).await.unwrap();
    let req = format!(
        "GET http://{HOST}:{}/ HTTP/1.1\r\nHost: x\r\n\r\n",
        o.port()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let (h, rest) = read_head(&mut s).await;
    assert!(h.to_ascii_lowercase().contains("connection: close"), "{h}");
    let _ = read_to_end(&mut s, rest).await;
    assert!(
        seen.lock().unwrap()[0]
            .to_ascii_lowercase()
            .contains("connection: close")
    );
}

/// Manual browser check: a proxy with pass-decrypt for `*.example.test` (upstream roots from
/// `W2_ROOT`), its address and CA written to `W2_BROWSER_DIR/{addr.txt,ca.pem}`, running until
/// `W2_BROWSER_DIR/stop` exists (at most 3 minutes); the proxy log goes to `proxy.log` there.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn serve_for_browser() {
    let dir = std::path::PathBuf::from(std::env::var("W2_BROWSER_DIR").unwrap());
    let root = std::path::PathBuf::from(std::env::var("W2_ROOT").unwrap());
    let p = proxy(&root);
    std::fs::write(dir.join("ca.pem"), ca_pem(&p)).unwrap();
    std::fs::write(dir.join("addr.txt"), p.addr().to_string()).unwrap();
    for _ in 0..360 {
        if dir.join("stop").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    std::fs::write(dir.join("proxy.log"), p.log()).unwrap();
}

/// Page-like load: `W2_REQS` (default 50) small GETs on each of `W2_CONNS` (default 1) decrypted
/// browser connections, repeated `W2_RUNS` (default 15) times; prints the median wall time.
///   cargo test --release --test w2_2a_reuse -- --ignored --nocapture perf
/// Run under `flock /tmp/scratchy-perf.lock`; `WIRE2_TEST_BIN` points the harness at another wire2
/// binary (the "before" build).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn perf_small_requests_on_decrypted_connections() {
    let env = |k: &str, d: usize| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let (reqs, conns, runs) = (env("W2_REQS", 50), env("W2_CONNS", 1), env("W2_RUNS", 15));
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let p = proxy(roots.path());
    let mut times = Vec::new();
    for _ in 0..runs {
        let t0 = std::time::Instant::now();
        let mut tasks = Vec::new();
        for _ in 0..conns {
            let mut b = Browser::open(&p, addr.port()).await;
            tasks.push(tokio::spawn(async move {
                for i in 0..reqs {
                    let (st, _, body) = b.req("GET", &format!("/r{i}"), "", b"").await;
                    assert_eq!(st, 200);
                    assert_eq!(body, format!("ok /r{i}").as_bytes());
                }
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        times.push(t0.elapsed());
    }
    times.sort();
    let med = times[times.len() / 2];
    println!(
        "perf: {reqs} requests x {conns} connection(s): median {:.1} ms per run ({:.2} ms/request), min {:.1} ms; origin accepted {} TCP connections over {runs} runs",
        med.as_secs_f64() * 1e3,
        med.as_secs_f64() * 1e3 / (reqs * conns) as f64,
        times[0].as_secs_f64() * 1e3,
        c.conns()
    );
}
