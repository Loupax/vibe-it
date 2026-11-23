//! W2-1: SPEC "A 2 GB download ... keeps proxy memory flat, and its first bytes arrive before the
//! upstream finishes sending." Generated on the fly, never stored. `W2_LARGE_BYTES` overrides the size.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;

const MIB: u64 = 1024 * 1024;
/// Allowed RSS growth of the proxy during the transfer.
const MAX_GROWTH_KB: u64 = 24 * 1024;

fn size() -> u64 {
    std::env::var("W2_LARGE_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2048 * MIB)
}

/// Origin that sends 1 MiB, waits until the test says go (so the client provably got bytes before the
/// upstream finished), then the rest. `http` prefixes a response head with Content-Length.
async fn big_origin(
    total: u64,
    http: bool,
    go: Arc<Notify>,
    finished: Arc<AtomicBool>,
) -> std::net::SocketAddr {
    serve("127.0.0.1:0", move |mut s| {
        let go = go.clone();
        let finished = finished.clone();
        async move {
            if http {
                let _ = read_head(&mut s).await;
                s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes())
                    .await
                    .unwrap();
            }
            let chunk: Vec<u8> = (0..MIB as usize).map(|i| (i % 253) as u8).collect();
            let mut sent = 0u64;
            let first = MIB.min(total);
            s.write_all(&chunk[..first as usize]).await.unwrap();
            sent += first;
            go.notified().await;
            while sent < total {
                let n = (total - sent).min(MIB) as usize;
                if s.write_all(&chunk[..n]).await.is_err() {
                    return;
                }
                sent += n as u64;
            }
            finished.store(true, Ordering::SeqCst);
            let _ = s.shutdown().await;
        }
    })
    .await
}

/// Read everything, sampling the proxy's RSS; returns (bytes, max RSS KiB, MB/s).
async fn drain_and_sample(
    s: &mut TcpStream,
    pre: Vec<u8>,
    proxy: &Proxy,
    go: &Notify,
    finished: &AtomicBool,
) -> (u64, u64, f64) {
    let mut got = pre.len() as u64;
    let mut buf = vec![0u8; MIB as usize];
    // First bytes, before the origin has finished (it is waiting for `go`).
    while got == 0 {
        got += s.read(&mut buf).await.unwrap() as u64;
    }
    assert!(
        !finished.load(Ordering::SeqCst),
        "origin finished before the first bytes arrived"
    );
    go.notify_one();
    let max_rss = Arc::new(AtomicU64::new(proxy.rss_kb()));
    let stop = Arc::new(AtomicBool::new(false));
    let pid = proxy.pid();
    let sampler = {
        let (max_rss, stop) = (max_rss.clone(), stop.clone());
        tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                max_rss.fetch_max(rss_kb(pid), Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    };
    let t = Instant::now();
    loop {
        let n = tokio::time::timeout(Duration::from_secs(30), s.read(&mut buf))
            .await
            .expect("stalled")
            .unwrap();
        if n == 0 {
            break;
        }
        got += n as u64;
    }
    let secs = t.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    sampler.await.unwrap();
    (
        got,
        max_rss.load(Ordering::Relaxed),
        got as f64 / MIB as f64 / secs,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn large_download_through_tunnel_flat_memory() {
    let total = size();
    let (go, finished) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(false)));
    let origin = big_origin(total, false, go.clone(), finished.clone()).await;
    let proxy = Proxy::start(&[]);
    let (mut s, head, rest) = connect_via(proxy.addr(), &origin.to_string()).await;
    assert_eq!(status_of(&head), 200);
    let base = proxy.rss_kb();
    let (got, max, mbs) = drain_and_sample(&mut s, rest, &proxy, &go, &finished).await;
    assert_eq!(got, total);
    eprintln!(
        "tunnel: {} MiB, proxy RSS {base} KiB -> max {max} KiB, {mbs:.0} MiB/s",
        total / MIB
    );
    assert!(
        max.saturating_sub(base) < MAX_GROWTH_KB,
        "proxy RSS grew {base} -> {max} KiB"
    );
    drop(s); // the browser closes too: the flow ends at once (else after the half-close grace)
    let l = proxy
        .wait_log(|l| l.contains("access flow=1 "), Duration::from_secs(5))
        .unwrap();
    assert!(l.contains(&format!("down={}", total + 39)), "{l}");
}

#[tokio::test(flavor = "multi_thread")]
async fn large_download_plain_http_flat_memory() {
    let total = size();
    let (go, finished) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(false)));
    let origin = big_origin(total, true, go.clone(), finished.clone()).await;
    let proxy = Proxy::start(&[]);
    let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
    s.write_all(format!("GET http://{origin}/big HTTP/1.1\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let (head, rest) = read_head(&mut s).await;
    assert_eq!(status_of(&head), 200);
    assert_eq!(header_of(&head, "content-length"), Some(total.to_string()));
    let base = proxy.rss_kb();
    let (got, max, mbs) = drain_and_sample(&mut s, rest, &proxy, &go, &finished).await;
    assert_eq!(got, total);
    eprintln!(
        "plain http: {} MiB, proxy RSS {base} KiB -> max {max} KiB, {mbs:.0} MiB/s",
        total / MIB
    );
    assert!(
        max.saturating_sub(base) < MAX_GROWTH_KB,
        "proxy RSS grew {base} -> {max} KiB"
    );
}
