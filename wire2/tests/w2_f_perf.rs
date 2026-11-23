//! W2-F performance measurements (SPEC "Performance requirements"), ignored by default:
//!   cargo test --release --test w2_f_perf -- --ignored --nocapture --test-threads 1
//! On the user's machine run it under `flock /tmp/scratchy-perf.lock` and note `uptime`.
//! The decrypted target (pass-decrypt vs direct, >= 50 %) is tests/w2_2_perf.rs.
//! `W2_PERF_BYTES` sets the bytes per throughput run (default 4 GiB).

mod common;

use std::time::{Duration, Instant};

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MIB: u64 = 1024 * 1024;

fn raise_nofile() {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: plain rlimit calls on our own process.
    unsafe {
        libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim);
        lim.rlim_cur = lim.rlim_max;
        libc::setrlimit(libc::RLIMIT_NOFILE, &lim);
    }
}

async fn source(n: u64) -> std::net::SocketAddr {
    serve("127.0.0.1:0", move |mut s| async move {
        let buf = vec![0x5au8; MIB as usize];
        let mut left = n;
        while left > 0 {
            let k = left.min(MIB) as usize;
            if s.write_all(&buf[..k]).await.is_err() {
                return;
            }
            left -= k as u64;
        }
        let _ = s.shutdown().await;
    })
    .await
}

async fn drain(mut s: TcpStream, pre: usize) -> u64 {
    let mut buf = vec![0u8; MIB as usize];
    let mut got = pre as u64;
    loop {
        let n = s.read(&mut buf).await.unwrap();
        if n == 0 {
            return got;
        }
        got += n as u64;
    }
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// Undecrypted tunnel through one pass stage vs direct, over loopback. SPEC target: >= 90 %.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn perf_tunnel_through_pass_vs_direct() {
    let n = std::env::var("W2_PERF_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096 * MIB);
    let src = source(n).await;
    let d = sock_dir();
    let pass = PassBin::start(&d.path().join("pass.sock"));
    let proxy = proxy_with(&[&pass.path], &[], &[]);
    wait_up(&proxy, "pass");
    let (mut direct, mut tunnel) = (Vec::new(), Vec::new());
    for _ in 0..9 {
        let t = Instant::now();
        assert_eq!(drain(TcpStream::connect(src).await.unwrap(), 0).await, n);
        direct.push(n as f64 / MIB as f64 / t.elapsed().as_secs_f64());
        let t = Instant::now();
        let (s, head, rest) = connect_via(proxy.addr(), &src.to_string()).await;
        assert_eq!(status_of(&head), 200);
        assert_eq!(drain(s, rest.len()).await, n);
        tunnel.push(n as f64 / MIB as f64 / t.elapsed().as_secs_f64());
    }
    let (dm, tm) = (median(&mut direct), median(&mut tunnel));
    eprintln!(
        "PERF tunnel through pass: {} MiB per run, 9 runs, median direct {dm:.0} MiB/s, through pass {tm:.0} MiB/s, ratio {:.1} % (runs direct {direct:.0?}, pass {tunnel:.0?}); RSS proxy {} KiB, pass {} KiB",
        n / MIB,
        100.0 * tm / dm,
        proxy.rss_kb(),
        pass.rss_kb()
    );
}

/// Time to response headers for small plain-http requests, without filters and with one pass stage
/// (each request: browser -> proxy -> pass -> hub -> origin). SPEC: added p99 < 2 ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn perf_added_header_latency_one_pass_stage() {
    let origin = serve("127.0.0.1:0", |mut s| async move {
        let _ = read_head(&mut s).await;
        let _ = s
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await;
        let _ = s.shutdown().await;
    })
    .await;
    let plain = Proxy::start(&[]);
    let d = sock_dir();
    let pass = PassBin::start(&d.path().join("pass.sock"));
    let filtered = proxy_with(&[&pass.path], &[], &[]);
    wait_up(&filtered, "pass");
    async fn sample(
        proxy: std::net::SocketAddr,
        origin: std::net::SocketAddr,
        n: usize,
    ) -> Vec<f64> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let t = Instant::now();
            let mut s = TcpStream::connect(proxy).await.unwrap();
            let _ = s.set_nodelay(true);
            s.write_all(
                format!("GET http://{origin}/ HTTP/1.1\r\nHost: {origin}\r\n\r\n").as_bytes(),
            )
            .await
            .unwrap();
            let (h, _) = read_head(&mut s).await;
            out.push(t.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(status_of(&h), 200);
        }
        out.sort_by(|a, b| a.partial_cmp(b).unwrap());
        out
    }
    let pct = |v: &[f64], p: f64| v[((v.len() as f64 * p) as usize).min(v.len() - 1)];
    // Warm up both, then interleave rounds so both see the same machine load.
    sample(plain.addr(), origin, 200).await;
    sample(filtered.addr(), origin, 200).await;
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for _ in 0..5 {
        a.extend(sample(plain.addr(), origin, 1000).await);
        b.extend(sample(filtered.addr(), origin, 1000).await);
    }
    a.sort_by(|x, y| x.partial_cmp(y).unwrap());
    b.sort_by(|x, y| x.partial_cmp(y).unwrap());
    eprintln!(
        "PERF header latency over {} requests each: no filter p50 {:.3} ms p99 {:.3} ms; one pass stage p50 {:.3} ms p99 {:.3} ms; added p50 {:.3} ms, added p99 {:.3} ms",
        a.len(),
        pct(&a, 0.5),
        pct(&a, 0.99),
        pct(&b, 0.5),
        pct(&b, 0.99),
        pct(&b, 0.5) - pct(&a, 0.5),
        pct(&b, 0.99) - pct(&a, 0.99),
    );
}

/// 500 (and 2,000) open tunnels through pass: proxy and pass RSS. SPEC: proxy < 100 MB at 500 flows,
/// >= 2,000 concurrent flows without degradation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn perf_many_flows_through_pass() {
    raise_nofile();
    let echo = echo_server("127.0.0.1:0").await;
    let d = sock_dir();
    let pass = PassBin::start(&d.path().join("pass.sock"));
    let proxy = proxy_with(&[&pass.path], &[], &[]);
    wait_up(&proxy, "pass");
    let idle = (proxy.rss_kb(), pass.rss_kb());
    let mut flows = Vec::new();
    let mut at_500 = (0, 0);
    let t = Instant::now();
    for i in 0..2000 {
        let (mut s, head, _) = connect_via(proxy.addr(), &echo.to_string()).await;
        assert_eq!(status_of(&head), 200, "flow {i}");
        s.write_all(&[7u8; 1024]).await.unwrap();
        let mut b = [0u8; 1024];
        s.read_exact(&mut b).await.unwrap();
        flows.push(s);
        if i + 1 == 500 {
            at_500 = (proxy.rss_kb(), pass.rss_kb());
        }
    }
    let open_secs = t.elapsed().as_secs_f64();
    let at_2000 = (proxy.rss_kb(), pass.rss_kb());
    let t = Instant::now();
    let tasks: Vec<_> = flows
        .into_iter()
        .map(|mut s| {
            tokio::spawn(async move {
                let data = vec![3u8; 64 * 1024];
                let (mut r, mut w) = s.split();
                let send = async { w.write_all(&data).await.unwrap() };
                let recv = async {
                    let mut b = vec![0u8; 64 * 1024];
                    r.read_exact(&mut b).await.unwrap();
                };
                tokio::join!(send, recv);
                s
            })
        })
        .collect();
    let mut flows = Vec::new();
    for t in tasks {
        flows.push(
            tokio::time::timeout(Duration::from_secs(60), t)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let burst_secs = t.elapsed().as_secs_f64();
    eprintln!(
        "PERF flows through pass (proxy KiB, pass KiB): idle {idle:?}, 500 open {at_500:?}, 2000 open {at_2000:?} (opened in {open_secs:.2}s), after 2000 x 64 KiB concurrent echo {:?} in {burst_secs:.2}s",
        (proxy.rss_kb(), pass.rss_kb())
    );
    assert!(
        at_500.0 < 100 * 1024,
        "proxy RSS at 500 flows {} KiB",
        at_500.0
    );
    drop(flows);
}
