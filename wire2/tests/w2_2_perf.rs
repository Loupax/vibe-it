//! W2-2 performance measurement (SPEC "Decrypted transfer through pass-decrypt vs. direct: >= 50 %"),
//! ignored by default:
//!   cargo test --release --test w2_2_perf -- --ignored --nocapture --test-threads 1
//! On the user's machine run it under `flock /tmp/scratchy-perf.lock` and note `uptime`.
//! "Direct" is a TLS client talking to the HTTPS origin itself; "decrypted" is the same client through
//! the proxy with decryption forced by the TEST-ONLY `--test-decrypt` (pass-decrypt comes with W2-F,
//! and will add one filter stage on top). `W2_PERF_BYTES` sets the bytes per run (default 2 GiB).

mod common;

use std::time::Instant;

use common::*;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls::pki_types::ServerName;

const MIB: u64 = 1024 * 1024;
const HOST: &str = "www.example.test";

async fn drain<R: AsyncRead + Unpin>(r: &mut R, pre: usize) -> u64 {
    let mut buf = vec![0u8; MIB as usize];
    let mut got = pre as u64;
    loop {
        let n = r.read(&mut buf).await.unwrap();
        if n == 0 {
            return got;
        }
        got += n as u64;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn perf_decrypted_throughput_vs_direct() {
    let n: u64 = std::env::var("W2_PERF_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2048 * MIB);
    let pki = pki();
    let roots = root_file(&pki);
    let acceptor = tokio_rustls::TlsAcceptor::from(pki.server.clone());
    let origin = serve("127.0.0.1:0", move |s| {
        let acceptor = acceptor.clone();
        async move {
            let Ok(mut s) = acceptor.accept(s).await else {
                return;
            };
            let _ = read_head(&mut s).await;
            s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {n}\r\n\r\n").as_bytes())
                .await
                .unwrap();
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
        }
    })
    .await;
    let proxy = Proxy::start_env(
        &[
            "--test-resolve",
            "*.example.test=127.0.0.1",
            "--test-decrypt",
            HOST,
        ],
        &[("SSL_CERT_FILE", roots.path())],
    );
    let browser = client_trusting(&ca_pem(&proxy));
    let target = format!("{HOST}:{}", origin.port());
    let req = format!("GET /big HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n");
    let (mut direct, mut dec) = (Vec::new(), Vec::new());
    for _ in 0..7 {
        let t = Instant::now();
        let tcp = tokio::net::TcpStream::connect(origin).await.unwrap();
        let mut s = tokio_rustls::TlsConnector::from(pki.client.clone())
            .connect(ServerName::try_from(HOST).unwrap(), tcp)
            .await
            .unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        let (_, rest) = read_head(&mut s).await;
        assert_eq!(drain(&mut s, rest.len()).await, n);
        direct.push(n as f64 / MIB as f64 / t.elapsed().as_secs_f64());

        let t = Instant::now();
        let mut s = tls_via(proxy.addr(), &target, HOST, browser.clone())
            .await
            .unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        let (h, rest) = read_head(&mut s).await;
        assert_eq!(status_of(&h), 200, "{h}");
        assert_eq!(drain(&mut s, rest.len()).await, n);
        dec.push(n as f64 / MIB as f64 / t.elapsed().as_secs_f64());
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let (d, x) = (med(&mut direct), med(&mut dec));
    eprintln!(
        "PERF decrypted throughput: {} MiB per run, 7 runs, median direct TLS {d:.0} MiB/s, decrypted {x:.0} MiB/s, ratio {:.1} % (runs direct {direct:.0?}, decrypted {dec:.0?}), proxy RSS {} KiB",
        n / MIB,
        100.0 * x / d,
        proxy.rss_kb()
    );
}
