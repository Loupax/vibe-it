//! W2-2: SPEC "A 2 GB download through ... pass-decrypt keeps proxy ... memory flat, and its first bytes
//! arrive before the upstream finishes sending." Decryption forced with the TEST-ONLY `--test-decrypt`
//! (pass-decrypt arrives with W2-F). Generated on the fly, never stored. `W2_LARGE_BYTES` overrides
//! the size.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

const MIB: u64 = 1024 * 1024;
/// Allowed RSS growth of the proxy during the transfer.
const MAX_GROWTH_KB: u64 = 24 * 1024;
const HOST: &str = "www.example.test";

fn size() -> u64 {
    std::env::var("W2_LARGE_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2048 * MIB)
}

#[tokio::test(flavor = "multi_thread")]
async fn large_download_decrypted_flat_memory() {
    let total = size();
    let pki = pki();
    let roots = root_file(&pki);
    let (go, finished) = (Arc::new(Notify::new()), Arc::new(AtomicBool::new(false)));
    // HTTPS origin: 1 MiB, then waits for `go` (so the client provably got bytes before the origin
    // finished), then the rest.
    let acceptor = tokio_rustls::TlsAcceptor::from(pki.server.clone());
    let origin = {
        let (go, finished) = (go.clone(), finished.clone());
        serve("127.0.0.1:0", move |s| {
            let (acceptor, go, finished) = (acceptor.clone(), go.clone(), finished.clone());
            async move {
                let Ok(mut s) = acceptor.accept(s).await else {
                    return;
                };
                let _ = read_head(&mut s).await;
                s.write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n").as_bytes(),
                )
                .await
                .unwrap();
                let chunk: Vec<u8> = (0..MIB as usize).map(|i| (i % 253) as u8).collect();
                let first = MIB.min(total);
                s.write_all(&chunk[..first as usize]).await.unwrap();
                s.flush().await.unwrap();
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
    };
    let proxy = Proxy::start_env(
        &[
            "--test-resolve",
            "*.example.test=127.0.0.1",
            "--test-decrypt",
            HOST,
        ],
        &[("SSL_CERT_FILE", roots.path())],
    );
    let target = format!("{HOST}:{}", origin.port());
    let mut tls = tls_via(
        proxy.addr(),
        &target,
        HOST,
        client_trusting(&ca_pem(&proxy)),
    )
    .await
    .unwrap();
    tls.write_all(
        format!("GET /big HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    tls.flush().await.unwrap();
    let (head, rest) = read_head(&mut tls).await;
    assert_eq!(status_of(&head), 200, "{head}");
    assert_eq!(header_of(&head, "content-length"), Some(total.to_string()));

    let mut got = rest.len() as u64;
    let mut buf = vec![0u8; MIB as usize];
    while got == 0 {
        got += tls.read(&mut buf).await.unwrap() as u64;
    }
    assert!(
        !finished.load(Ordering::SeqCst),
        "origin finished before the first bytes arrived"
    );
    let base = proxy.rss_kb();
    go.notify_one();
    let max_rss = Arc::new(AtomicU64::new(base));
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
        let n = tokio::time::timeout(Duration::from_secs(30), tls.read(&mut buf))
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
    let max = max_rss.load(Ordering::Relaxed);
    assert_eq!(got, total);
    eprintln!(
        "decrypted: {} MiB, proxy RSS {base} KiB -> max {max} KiB, {:.0} MiB/s",
        total / MIB,
        got as f64 / MIB as f64 / secs
    );
    assert!(
        max.saturating_sub(base) < MAX_GROWTH_KB,
        "proxy RSS grew {base} -> {max} KiB"
    );
    let l = proxy
        .wait_log(|l| l.contains("req=1 GET https://"), Duration::from_secs(5))
        .unwrap();
    assert!(l.contains("status=200"), "{l}");
}
