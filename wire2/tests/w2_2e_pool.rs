//! W2-2e: parked origin connections expire on their own, and the pool is shared by all of the browser's
//! decrypted connections to one origin, keyed by host, port and the tunnel-filter chain.
//! Names map to 127.0.0.1 with the TEST-ONLY `--test-resolve`.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::*;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls;

const HOST: &str = "www.example.test";
const RESOLVE: &[&str] = &["--test-resolve", "*.test=127.0.0.1"];

#[derive(Clone, Default)]
struct Counts {
    /// Accepted TCP connections.
    accepted: Arc<AtomicUsize>,
    /// Connections open right now (accepted and not yet closed by either side).
    live: Arc<AtomicUsize>,
    /// Connections the proxy closed with a TLS close_notify (a clean EOF on our read).
    clean: Arc<AtomicUsize>,
}

impl Counts {
    fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
    fn live(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }
    fn clean(&self) -> usize {
        self.clean.load(Ordering::SeqCst)
    }
    /// Wait until `live()` is `n` (the proxy's closes arrive asynchronously), at most 5 s; returns it.
    async fn settle(&self, n: usize) -> usize {
        for _ in 0..100 {
            if self.live() == n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.live()
    }
}

struct Live(Arc<AtomicUsize>);
impl Drop for Live {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A keep-alive TLS origin; `/slow` answers after 300 ms (so parallel requests hold parallel
/// connections), anything else at once with `ok <path>`.
async fn origin(cfg: Arc<rustls::ServerConfig>) -> (SocketAddr, Counts) {
    let c = Counts::default();
    let c2 = c.clone();
    let acceptor = tokio_rustls::TlsAcceptor::from(cfg);
    let addr = serve("127.0.0.1:0", move |s| {
        let (c, acceptor) = (c2.clone(), acceptor.clone());
        async move {
            c.accepted.fetch_add(1, Ordering::SeqCst);
            c.live.fetch_add(1, Ordering::SeqCst);
            let _live = Live(c.live.clone());
            let Ok(mut t) = acceptor.accept(s).await else {
                return;
            };
            let mut pre: Vec<u8> = Vec::new();
            loop {
                let head = match read_head_from(&mut t, std::mem::take(&mut pre)).await {
                    Ok(Some((h, rest))) => {
                        pre = rest;
                        h
                    }
                    Ok(None) => {
                        c.clean.fetch_add(1, Ordering::SeqCst);
                        return;
                    }
                    Err(()) => return,
                };
                let path = head
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                if path == "/slow" {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                let b = format!("ok {path}");
                let out = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{b}", b.len());
                if t.write_all(out.as_bytes()).await.is_err() || t.flush().await.is_err() {
                    return;
                }
            }
        }
    })
    .await;
    (addr, c)
}

/// One request head (no bodies here): Ok(Some) a head, Ok(None) a clean EOF (close_notify), Err anything
/// else.
async fn read_head_from<R: AsyncRead + Unpin>(
    r: &mut R,
    mut buf: Vec<u8>,
) -> Result<Option<(String, Vec<u8>)>, ()> {
    loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(i + 4);
            return Ok(Some((String::from_utf8_lossy(&buf).into_owned(), rest)));
        }
        let mut tmp = [0u8; 4096];
        match r.read(&mut tmp).await {
            Ok(0) if buf.is_empty() => return Ok(None),
            Ok(0) | Err(_) => return Err(()),
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
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

    /// GET `path`: (status, body).
    async fn get(&mut self, path: &str) -> (u16, String) {
        let head = format!("GET {path} HTTP/1.1\r\nHost: {}\r\n\r\n", self.target);
        self.t.write_all(head.as_bytes()).await.unwrap();
        self.t.flush().await.unwrap();
        let Ok(Some((head, mut rest))) =
            read_head_from(&mut self.t, std::mem::take(&mut self.buf)).await
        else {
            panic!("no response head");
        };
        let n: usize = header_of(&head, "content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        while rest.len() < n {
            let mut tmp = [0u8; 4096];
            let k = self.t.read(&mut tmp).await.unwrap();
            assert!(k > 0, "body cut short");
            rest.extend_from_slice(&tmp[..k]);
        }
        self.buf = rest.split_off(n);
        (status_of(&head), String::from_utf8(rest).unwrap())
    }
}

fn proxy(roots: &std::path::Path, args: &[&str]) -> Decrypting {
    let mut a: Vec<&str> = RESOLVE.to_vec();
    a.extend_from_slice(args);
    decrypting(&["*.example.test"], &a, &[("SSL_CERT_FILE", roots)])
}

/// Two browser connections, one after the other (and with the first still open): one origin connection.
#[tokio::test(flavor = "multi_thread")]
async fn browser_connections_share_one_parked_origin_connection() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let p = proxy(roots.path(), &[]);
    let mut a = Browser::open(&p, addr.port()).await;
    assert_eq!(a.get("/a1").await, (200, "ok /a1".into()));
    // A second connection, the first one still open and idle: it takes the parked connection.
    let mut b = Browser::open(&p, addr.port()).await;
    assert_eq!(b.get("/b1").await, (200, "ok /b1".into()));
    assert_eq!(a.get("/a2").await, (200, "ok /a2".into()));
    drop((a, b));
    // Both browser connections gone: a third one still finds it.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut d = Browser::open(&p, addr.port()).await;
    assert_eq!(d.get("/d1").await, (200, "ok /d1".into()));
    assert_eq!(
        c.accepted(),
        1,
        "one origin connection for three browser connections"
    );
    assert_eq!(c.live(), 1);
}

/// A parked connection expires on its own (`--pool-idle`), is closed cleanly (TLS close_notify) even
/// though its browser connection is still open, and the next request dials fresh.
#[tokio::test(flavor = "multi_thread")]
async fn idle_expiry_closes_the_parked_connection_and_the_next_request_dials_fresh() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let p = proxy(roots.path(), &["--pool-idle", "400ms"]);
    let mut a = Browser::open(&p, addr.port()).await;
    assert_eq!(a.get("/one").await.0, 200);
    assert_eq!(c.live(), 1);
    assert_eq!(c.settle(0).await, 0, "the idle connection was closed");
    assert_eq!(c.clean(), 1, "with a close_notify, not a reset");
    // The browser connection is untouched; the next request opens a new origin connection.
    assert_eq!(a.get("/two").await, (200, "ok /two".into()));
    assert_eq!(c.accepted(), 2);
    // Used again before the expiry it is kept: reuse resets nothing but the clock.
    assert_eq!(a.get("/three").await.0, 200);
    assert_eq!(c.accepted(), 2);
}

/// `--pool-idle 0` turns reuse off.
#[tokio::test(flavor = "multi_thread")]
async fn pool_idle_zero_disables_reuse() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let p = proxy(roots.path(), &["--pool-idle", "0"]);
    let mut a = Browser::open(&p, addr.port()).await;
    for i in 0..3 {
        assert_eq!(a.get(&format!("/n{i}")).await.0, 200);
    }
    assert_eq!(c.accepted(), 3);
    assert_eq!(c.settle(0).await, 0);
}

/// 20 browser connections at once: 20 origin connections are needed, but only 6 (the per-host cap)
/// stay parked; the others are closed.
#[tokio::test(flavor = "multi_thread")]
async fn per_host_cap_under_20_parallel_browser_connections() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let p = proxy(roots.path(), &[]);
    let mut tasks = Vec::new();
    let mut open = Vec::new();
    for _ in 0..20 {
        open.push(Browser::open(&p, addr.port()).await);
    }
    for mut b in open {
        tasks.push(tokio::spawn(async move {
            assert_eq!(b.get("/slow").await.0, 200);
            b
        }));
    }
    let mut keep = Vec::new();
    for t in tasks {
        keep.push(t.await.unwrap());
    }
    assert_eq!(c.accepted(), 20);
    assert_eq!(c.settle(6).await, 6, "the per-host cap is 6");
    // And the 6 are in use again, not leaked: 6 quick requests on distinct connections add no dial.
    for b in keep.iter_mut().take(6) {
        assert_eq!(b.get("/fast").await.0, 200);
    }
    assert_eq!(c.accepted(), 20, "the parked six served them");
    assert_eq!(c.clean(), 14, "the other 14 were closed cleanly");
}

/// The overall cap: two hosts (ports), 10 parallel connections each, per-host 6, total 8.
#[tokio::test(flavor = "multi_thread")]
async fn overall_cap() {
    let pki = pki();
    let roots = root_file(&pki);
    let (a1, c1) = origin(pki.server.clone()).await;
    let (a2, c2) = origin(pki.server.clone()).await;
    let p = proxy(roots.path(), &["--pool-total", "8"]);
    let mut tasks = Vec::new();
    for port in [a1.port(), a2.port()] {
        for _ in 0..10 {
            let mut b = Browser::open(&p, port).await;
            tasks.push(tokio::spawn(async move {
                assert_eq!(b.get("/slow").await.0, 200);
                b
            }));
        }
    }
    let mut keep = Vec::new();
    for t in tasks {
        keep.push(t.await.unwrap());
    }
    assert_eq!(c1.accepted() + c2.accepted(), 20);
    for _ in 0..100 {
        if c1.live() + c2.live() <= 8 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(c1.live() + c2.live(), 8, "{} + {}", c1.live(), c2.live());
    assert!(c1.live() <= 6 && c2.live() <= 6);
}

fn connects(f: &Fake) -> usize {
    f.seen()
        .iter()
        .filter(|h| h.starts_with("CONNECT "))
        .count()
}

/// With a tunnel filter in the chain each upstream connection is still one CONNECT, a shared connection
/// does not cost another, and a filter that closes one connection ends exactly that one (the others keep
/// serving; the next requests redial through the filter).
#[tokio::test(flavor = "multi_thread")]
async fn tunnel_filter_close_ends_exactly_the_connection_it_carried() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let d = sock_dir();
    let pd = pass_decrypt(&d.path().join("pd.sock"), &["*.example.test"]);
    let f = Fake::start(
        &d.path().join("gate.sock"),
        hello_json("gate", "tunnel", &["*.example.test"], "closed"),
        Action::CloseFirstAfter(Duration::from_millis(1200)),
    );
    let p = proxy_with(
        &[&pd.path, &f.inproc.path],
        RESOLVE,
        &[("SSL_CERT_FILE", roots.path())],
    );
    wait_up(&p, "pass-decrypt");
    wait_up(&p, "gate");
    // Two parallel connections need two upstream connections: two CONNECTs.
    let mut a = Browser::open(&p, addr.port()).await;
    let mut b = Browser::open(&p, addr.port()).await;
    let (ra, rb) = tokio::join!(a.get("/slow"), b.get("/slow"));
    assert_eq!((ra.0, rb.0), (200, 200));
    assert_eq!(connects(&f), 2, "{:?}", f.seen());
    assert_eq!(c.accepted(), 2);
    // Sequential use shares: still two CONNECTs.
    assert_eq!(a.get("/x").await.0, 200);
    assert_eq!(b.get("/y").await.0, 200);
    assert_eq!(connects(&f), 2);
    // The filter closes the first connection it carried and only that one.
    assert_eq!(
        c.settle(1).await,
        1,
        "exactly one of two connections was ended"
    );
    assert_eq!(f.open.load(Ordering::SeqCst), 1);
    // Both browser connections carry on: one takes the surviving connection, the other redials through
    // the gate (a third CONNECT); a dead one is never used.
    let (ra, rb) = tokio::join!(a.get("/slow"), b.get("/slow"));
    assert_eq!((ra.0, rb.0), (200, 200), "{:?} {:?}", ra, rb);
    assert_eq!(connects(&f), 3, "{:?}", f.seen());
    assert_eq!(c.accepted(), 3);
}

/// Different chains never share: the same host and port reached through a different tunnel-filter
/// chain gets its own connections (here the gate stops claiming the host for NEW flows).
#[tokio::test(flavor = "multi_thread")]
async fn different_chains_never_share() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let d = sock_dir();
    let pd = pass_decrypt(&d.path().join("pd.sock"), &["*.example.test"]);
    let f = Fake::start(
        &d.path().join("gate.sock"),
        hello_json("gate", "tunnel", &["*.example.test"], "closed"),
        Action::Forward,
    );
    let p = proxy_with(
        &[&pd.path, &f.inproc.path],
        &[
            "--test-resolve",
            "*.test=127.0.0.1",
            "--filter-check",
            "100ms",
        ],
        &[("SSL_CERT_FILE", roots.path())],
    );
    wait_up(&p, "pass-decrypt");
    wait_up(&p, "gate");
    let mut a = Browser::open(&p, addr.port()).await; // chain [gate]
    assert_eq!(a.get("/a1").await.0, 200);
    assert_eq!((connects(&f), c.accepted()), (1, 1));
    // Two more browser connections on the same chain share it.
    let mut a2 = Browser::open(&p, addr.port()).await;
    assert_eq!(a2.get("/a2").await.0, 200);
    assert_eq!((connects(&f), c.accepted()), (1, 1));
    *f.hello.lock().unwrap() = (
        200,
        hello_json("gate", "tunnel", &["nothing.test"], "closed"),
    );
    p.wait_log(
        |l| l.contains("filter gate CHANGED"),
        Duration::from_secs(10),
    )
    .unwrap_or_else(|| panic!("{}", p.log()));
    // A new flow has the empty chain: it must not take the gate's connection.
    let mut b = Browser::open(&p, addr.port()).await;
    assert_eq!(b.get("/b1").await.0, 200);
    assert_eq!(
        c.accepted(),
        2,
        "the empty chain dialled its own connection"
    );
    assert_eq!(connects(&f), 1, "the gate was not asked again");
    // Each chain keeps using its own, and only its own.
    for _ in 0..3 {
        assert_eq!(a.get("/a").await.0, 200);
        assert_eq!(b.get("/b").await.0, 200);
    }
    assert_eq!(c.accepted(), 2);
    assert_eq!(connects(&f), 1);
}

/// The flow that dialled a connection through a tunnel filter ends; its parked connection (and the
/// hub's pipe behind the filter) lives on and serves the next flow without another CONNECT.
#[tokio::test(flavor = "multi_thread")]
async fn parked_connection_outlives_the_flow_that_dialled_it_through_a_filter() {
    let pki = pki();
    let roots = root_file(&pki);
    let (addr, c) = origin(pki.server.clone()).await;
    let d = sock_dir();
    let pd = pass_decrypt(&d.path().join("pd.sock"), &["*.example.test"]);
    let f = Fake::start(
        &d.path().join("gate.sock"),
        hello_json("gate", "tunnel", &["*.example.test"], "closed"),
        Action::Forward,
    );
    let p = proxy_with(
        &[&pd.path, &f.inproc.path],
        RESOLVE,
        &[("SSL_CERT_FILE", roots.path())],
    );
    wait_up(&p, "pass-decrypt");
    wait_up(&p, "gate");
    let mut a = Browser::open(&p, addr.port()).await;
    assert_eq!(a.get("/a").await.0, 200);
    drop(a);
    p.wait_log(|l| l.contains("decrypted=yes"), Duration::from_secs(5))
        .unwrap_or_else(|| panic!("{}", p.log()));
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut b = Browser::open(&p, addr.port()).await;
    assert_eq!(b.get("/b").await, (200, "ok /b".into()));
    assert_eq!((connects(&f), c.accepted(), c.live()), (1, 1, 1));
}
