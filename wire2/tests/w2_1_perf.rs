//! W2-1 performance measurements (SPEC "Performance requirements"), ignored by default:
//!   cargo test --release --test w2_1_perf -- --ignored --nocapture --test-threads 1
//! On the user's machine run it under `flock /tmp/scratchy-perf.lock` and note `uptime`.
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

/// Origin sending `n` bytes per connection, 1 MiB writes.
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

/// Undecrypted tunnel with no filters vs direct, over loopback. SPEC target: >= 95 %.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn perf_tunnel_throughput_vs_direct() {
    let n = std::env::var("W2_PERF_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096 * MIB);
    let src = source(n).await;
    let proxy = Proxy::start(&[]);
    let (mut direct, mut tunnel) = (Vec::new(), Vec::new());
    for _ in 0..9 {
        let t = Instant::now();
        let got = drain(TcpStream::connect(src).await.unwrap(), 0).await;
        assert_eq!(got, n);
        direct.push(n as f64 / MIB as f64 / t.elapsed().as_secs_f64());

        let t = Instant::now();
        let (s, head, rest) = connect_via(proxy.addr(), &src.to_string()).await;
        assert_eq!(status_of(&head), 200);
        let got = drain(s, rest.len()).await;
        assert_eq!(got, n);
        tunnel.push(n as f64 / MIB as f64 / t.elapsed().as_secs_f64());
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let (d, t) = (med(&mut direct), med(&mut tunnel));
    eprintln!(
        "PERF tunnel throughput: {} MiB per run, 9 runs, median direct {d:.0} MiB/s, tunnel {t:.0} MiB/s, ratio {:.1} % (runs direct {direct:.0?}, tunnel {tunnel:.0?}), proxy RSS {} KiB",
        n / MIB,
        100.0 * t / d,
        proxy.rss_kb()
    );
}

/// Many concurrent open tunnels: RSS at 500 flows (SPEC < 100 MB) and 2,000 flows working at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn perf_many_flows_memory() {
    raise_nofile();
    let echo = echo_server("127.0.0.1:0").await;
    let proxy = Proxy::start(&[]);
    let idle = proxy.rss_kb();
    let mut flows = Vec::new();
    let mut at_500 = 0;
    let t = Instant::now();
    for i in 0..2000 {
        let (mut s, head, _) = connect_via(proxy.addr(), &echo.to_string()).await;
        assert_eq!(status_of(&head), 200, "flow {i}");
        s.write_all(&[7u8; 1024]).await.unwrap();
        let mut b = [0u8; 1024];
        s.read_exact(&mut b).await.unwrap();
        flows.push(s);
        if i + 1 == 500 {
            at_500 = proxy.rss_kb();
        }
    }
    let open_secs = t.elapsed().as_secs_f64();
    let at_2000 = proxy.rss_kb();
    // All 2,000 at once: every flow echoes 64 KiB concurrently.
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
    let peak = proxy.rss_kb();
    proxy.signal(libc::SIGUSR1);
    let dump = proxy
        .wait_log(|l| l.contains("registry: "), Duration::from_secs(5))
        .unwrap();
    eprintln!(
        "PERF flows: idle RSS {idle} KiB, 500 open {at_500} KiB, 2000 open {at_2000} KiB (opened in {open_secs:.2}s), after 2000 x 64 KiB concurrent echo {peak} KiB in {burst_secs:.2}s; {}",
        dump.split("INFO").nth(1).unwrap_or("").trim()
    );
    assert!(at_500 < 100 * 1024, "RSS at 500 flows {at_500} KiB");
    drop(flows);
}
