//! W2-F: SPEC "A 2 GB download through pass and through pass-decrypt keeps proxy and filter memory
//! flat, and its first bytes arrive before the upstream finishes sending." This file: through the
//! `wire2-pass` binary, as an opaque tunnel and as plain http (pass-decrypt: tests/w2_2_large.rs).
//! Generated on the fly, never stored. `W2_LARGE_BYTES` overrides the size.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;

const MIB: u64 = 1024 * 1024;
/// Allowed RSS growth of the proxy and of pass during the transfer.
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
                s.write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n").as_bytes(),
                )
                .await
                .unwrap();
            }
            let chunk: Vec<u8> = (0..MIB as usize).map(|i| (i % 253) as u8).collect();
            let first = MIB.min(total);
            s.write_all(&chunk[..first as usize]).await.unwrap();
            let mut sent = first;
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

/// Read everything while sampling the RSS of `pids`; returns (bytes, base and max RSS per pid, MiB/s).
async fn drain(
    s: &mut TcpStream,
    pre: Vec<u8>,
    pids: &[i32],
    go: &Notify,
    finished: &AtomicBool,
) -> (u64, Vec<(u64, u64)>, f64) {
    let mut got = pre.len() as u64;
    let mut buf = vec![0u8; MIB as usize];
    while got == 0 {
        got += s.read(&mut buf).await.unwrap() as u64;
    }
    assert!(
        !finished.load(Ordering::SeqCst),
        "origin finished before the first bytes arrived"
    );
    let base: Vec<u64> = pids.iter().map(|&p| rss_kb(p)).collect();
    go.notify_one();
    let maxes: Arc<Vec<AtomicU64>> = Arc::new(base.iter().map(|&b| AtomicU64::new(b)).collect());
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (maxes, stop, pids) = (maxes.clone(), stop.clone(), pids.to_vec());
        tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                for (i, &p) in pids.iter().enumerate() {
                    maxes[i].fetch_max(rss_kb(p), Ordering::Relaxed);
                }
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
    let mem = base
        .iter()
        .zip(maxes.iter())
        .map(|(&b, m)| (b, m.load(Ordering::Relaxed)))
        .collect();
    (got, mem, got as f64 / MIB as f64 / secs)
}

fn check(what: &str, total: u64, got: u64, mem: &[(u64, u64)], mbs: f64) {
    assert_eq!(got, total);
    eprintln!(
        "{what}: {} MiB, proxy RSS {} -> max {} KiB, pass RSS {} -> max {} KiB, {mbs:.0} MiB/s",
        total / MIB,
        mem[0].0,
        mem[0].1,
        mem[1].0,
        mem[1].1
    );
    for (who, (b, m)) in ["proxy", "pass"].iter().zip(mem) {
        assert!(
            m.saturating_sub(*b) < MAX_GROWTH_KB,
            "{what}: {who} RSS grew {b} -> {m} KiB"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn large_download_through_pass_tunnel() {
    let total = size();
    let (go, finished) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(false)));
    let origin = big_origin(total, false, go.clone(), finished.clone()).await;
    let d = sock_dir();
    let pass = PassBin::start(&d.path().join("pass.sock"));
    let proxy = proxy_with(&[&pass.path], &[], &[]);
    wait_up(&proxy, "pass");
    let (mut s, head, rest) = connect_via(proxy.addr(), &origin.to_string()).await;
    assert_eq!(status_of(&head), 200);
    let (got, mem, mbs) = drain(&mut s, rest, &[proxy.pid(), pass.pid()], &go, &finished).await;
    check("tunnel through pass", total, got, &mem, mbs);
    drop(s);
    let l = proxy
        .wait_log(|l| l.contains("access flow=1 "), Duration::from_secs(10))
        .unwrap();
    assert!(
        l.contains("chain=pass") && l.contains(&format!("down={}", total + 39)),
        "{l}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn large_download_through_pass_plain_http() {
    let total = size();
    let (go, finished) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(false)));
    let origin = big_origin(total, true, go.clone(), finished.clone()).await;
    let d = sock_dir();
    let pass = PassBin::start(&d.path().join("pass.sock"));
    let proxy = proxy_with(&[&pass.path], &[], &[]);
    wait_up(&proxy, "pass");
    let mut s = TcpStream::connect(proxy.addr()).await.unwrap();
    s.write_all(format!("GET http://{origin}/big HTTP/1.1\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let (head, rest) = read_head(&mut s).await;
    assert_eq!(status_of(&head), 200);
    assert_eq!(header_of(&head, "content-length"), Some(total.to_string()));
    let (got, mem, mbs) = drain(&mut s, rest, &[proxy.pid(), pass.pid()], &go, &finished).await;
    check("plain http through pass", total, got, &mem, mbs);
}
