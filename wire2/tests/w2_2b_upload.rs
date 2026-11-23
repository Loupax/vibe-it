//! W2-2b: request bodies (uploads) inside decrypted tunnels are pipelined (decrypt and encrypt on two
//! tasks), like W2-2 did for responses. A 2 GiB upload through the test-only pass-decrypt keeps the
//! proxy's memory flat and the origin sees the right bytes. Generated on the fly, never stored.
//! `W2_LARGE_BYTES` overrides the size. The ignored `perf_upload_throughput_vs_direct` measures:
//!   cargo test --release --test w2_2b_upload -- --ignored --nocapture
//! (on the user's machine under `flock /tmp/scratchy-perf.lock`, noting `uptime`).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::*;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_rustls::rustls::pki_types::ServerName;

const MIB: u64 = 1024 * 1024;
const MAX_GROWTH_KB: u64 = 24 * 1024;
const HOST: &str = "www.example.test";

fn size() -> u64 {
    std::env::var("W2_LARGE_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2048 * MIB)
}

/// The byte at offset `i` of the generated upload.
fn pat(i: u64) -> u8 {
    (i % 251) as u8
}

fn chunk_at(off: u64, n: usize) -> Vec<u8> {
    (0..n as u64).map(|i| pat(off + i)).collect()
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

fn check(b: &[u8], off: &mut u64, ok: &mut bool) {
    for (i, x) in b.iter().enumerate() {
        if *x != pat(*off + i as u64) {
            *ok = false;
        }
    }
    *off += b.len() as u64;
}

/// Reads a request body (Content-Length or chunked), checks every byte against `pat`; returns
/// (bytes, all correct, bytes read past the body).
async fn check_body<S: AsyncRead + Unpin>(
    s: &mut S,
    head: &str,
    pre: Vec<u8>,
) -> Option<(u64, bool, Vec<u8>)> {
    let chunked = head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");
    let (mut off, mut ok) = (0u64, true);
    let mut buf = pre;
    if chunked {
        loop {
            let line_end = loop {
                if let Some(p) = buf.windows(2).position(|w| w == b"\r\n") {
                    break p;
                }
                let mut tmp = vec![0u8; 65536];
                let n = s.read(&mut tmp).await.ok()?;
                if n == 0 {
                    return None;
                }
                buf.extend_from_slice(&tmp[..n]);
            };
            let n = usize::from_str_radix(std::str::from_utf8(&buf[..line_end]).ok()?, 16).ok()?;
            buf.drain(..line_end + 2);
            while buf.len() < n + 2 {
                let mut tmp = vec![0u8; 65536];
                let k = s.read(&mut tmp).await.ok()?;
                if k == 0 {
                    return None;
                }
                buf.extend_from_slice(&tmp[..k]);
            }
            check(&buf[..n], &mut off, &mut ok);
            buf.drain(..n + 2);
            if n == 0 {
                return Some((off, ok, buf));
            }
        }
    }
    let len: u64 = header_of(head, "content-length")?.parse().ok()?;
    let take = (buf.len() as u64).min(len) as usize;
    check(&buf[..take], &mut off, &mut ok);
    let rest = buf.split_off(take);
    let mut buf = vec![0u8; 256 * 1024];
    while off < len {
        let want = ((len - off) as usize).min(buf.len());
        let n = s.read(&mut buf[..want]).await.ok()?;
        if n == 0 {
            return None;
        }
        check(&buf[..n], &mut off, &mut ok);
    }
    Some((off, ok, rest))
}

async fn upload_origin(acceptor: tokio_rustls::TlsAcceptor) -> std::net::SocketAddr {
    serve("127.0.0.1:0", move |s| {
        let acceptor = acceptor.clone();
        async move {
            let Ok(mut s) = acceptor.accept(s).await else {
                return;
            };
            let mut pre = Vec::new();
            loop {
                let Some((head, rest)) = read_head_from(&mut s, std::mem::take(&mut pre)).await
                else {
                    return;
                };
                let Some((n, ok, rest)) = check_body(&mut s, &head, rest).await else {
                    return;
                };
                pre = rest;
                let body = format!("got {n} {}", if ok { "ok" } else { "bad" });
                let out = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                if s.write_all(out.as_bytes()).await.is_err() {
                    return;
                }
            }
        }
    })
    .await
}

async fn send_len<S: AsyncWrite + Unpin>(s: &mut S, total: u64) {
    // The pattern repeats every 251 bytes: build a buffer of whole periods and slice it per offset.
    let period = 251usize;
    let base = chunk_at(0, period * 4200 + period);
    let mut sent = 0u64;
    while sent < total {
        let start = (sent % period as u64) as usize;
        let n = ((total - sent) as usize).min(MIB as usize);
        s.write_all(&base[start..start + n]).await.unwrap();
        sent += n as u64;
    }
    s.flush().await.unwrap();
}

async fn read_reply<S: AsyncRead + Unpin>(s: &mut S) -> String {
    let (h, rest) = read_head(s).await;
    assert_eq!(status_of(&h), 200, "{h}");
    let n: usize = header_of(&h, "content-length").unwrap().parse().unwrap();
    let mut body = rest;
    while body.len() < n {
        let mut tmp = [0u8; 256];
        let k = s.read(&mut tmp).await.unwrap();
        assert!(k > 0);
        body.extend_from_slice(&tmp[..k]);
    }
    String::from_utf8(body).unwrap()
}

/// Uploads through a decrypted tunnel arrive intact: Content-Length (above and below the pipelining
/// threshold) and chunked, several on one browser connection (the origin connection is reused too).
#[tokio::test(flavor = "multi_thread")]
async fn uploads_arrive_intact_length_and_chunked() {
    let pki = pki();
    let roots = root_file(&pki);
    let origin = upload_origin(tokio_rustls::TlsAcceptor::from(pki.server.clone())).await;
    let proxy = decrypting(
        &[HOST],
        &["--test-resolve", "*.example.test=127.0.0.1"],
        &[("SSL_CERT_FILE", roots.path())],
    );
    let target = format!("{HOST}:{}", origin.port());
    let mut t = tls_via(
        proxy.addr(),
        &target,
        HOST,
        client_trusting(&ca_pem(&proxy)),
    )
    .await
    .unwrap();
    for n in [10u64, 100_000, 5 * MIB + 7, 300_000] {
        t.write_all(
            format!("POST /up HTTP/1.1\r\nHost: {target}\r\nContent-Length: {n}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
        send_len(&mut t, n).await;
        assert_eq!(read_reply(&mut t).await, format!("got {n} ok"));
    }
    t.write_all(
        format!("POST /up HTTP/1.1\r\nHost: {target}\r\nTransfer-Encoding: chunked\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let total = 3 * MIB;
    let mut off = 0u64;
    while off < total {
        let n = (total - off).min(1000) as usize;
        let mut c = format!("{n:x}\r\n").into_bytes();
        c.extend(chunk_at(off, n));
        c.extend_from_slice(b"\r\n");
        t.write_all(&c).await.unwrap();
        off += n as u64;
    }
    t.write_all(b"0\r\n\r\n").await.unwrap();
    assert_eq!(read_reply(&mut t).await, format!("got {total} ok"));
    // The browser connection survived all of it.
    t.write_all(
        format!("POST /up HTTP/1.1\r\nHost: {target}\r\nContent-Length: 3\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    send_len(&mut t, 3).await;
    assert_eq!(read_reply(&mut t).await, "got 3 ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn large_upload_decrypted_flat_memory() {
    let total = size();
    let pki = pki();
    let roots = root_file(&pki);
    let origin = upload_origin(tokio_rustls::TlsAcceptor::from(pki.server.clone())).await;
    let proxy = decrypting(
        &[HOST],
        &["--test-resolve", "*.example.test=127.0.0.1"],
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
        format!("POST /up HTTP/1.1\r\nHost: {target}\r\nContent-Length: {total}\r\n\r\n")
            .as_bytes(),
    )
    .await
    .unwrap();
    let base = proxy.rss_kb();
    let me = std::process::id() as i32;
    let base_self = rss_kb(me);
    let max_rss = Arc::new(AtomicU64::new(base));
    let max_self = Arc::new(AtomicU64::new(base_self));
    let stop = Arc::new(AtomicBool::new(false));
    let pid = proxy.pid();
    let sampler = {
        let (max_rss, max_self, stop) = (max_rss.clone(), max_self.clone(), stop.clone());
        tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                max_rss.fetch_max(rss_kb(pid), Ordering::Relaxed);
                max_self.fetch_max(rss_kb(me), Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    };
    let t = Instant::now();
    send_len(&mut tls, total).await;
    let reply = tokio::time::timeout(Duration::from_secs(60), read_reply(&mut tls))
        .await
        .expect("no reply");
    let secs = t.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    sampler.await.unwrap();
    assert_eq!(reply, format!("got {total} ok"));
    let (max, max_s) = (
        max_rss.load(Ordering::Relaxed),
        max_self.load(Ordering::Relaxed),
    );
    eprintln!(
        "upload through pass-decrypt: {} MiB, proxy RSS {base} KiB -> max {max} KiB, test process \
         (pass-decrypt, origin, client) {base_self} KiB -> max {max_s} KiB, {:.0} MiB/s",
        total / MIB,
        total as f64 / MIB as f64 / secs
    );
    assert!(
        max.saturating_sub(base) < MAX_GROWTH_KB,
        "proxy RSS grew {base} -> {max} KiB"
    );
    assert!(
        max_s.saturating_sub(base_self) < MAX_GROWTH_KB,
        "filter/test RSS grew {base_self} -> {max_s} KiB"
    );
}

/// Upload throughput, decrypted vs direct TLS to the origin (the origin checks the bytes).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn perf_upload_throughput_vs_direct() {
    let n: u64 = std::env::var("W2_PERF_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2048 * MIB);
    let pki = pki();
    let roots = root_file(&pki);
    let origin = upload_origin(tokio_rustls::TlsAcceptor::from(pki.server.clone())).await;
    let proxy = decrypting(
        &[HOST],
        &["--test-resolve", "*.example.test=127.0.0.1"],
        &[("SSL_CERT_FILE", roots.path())],
    );
    let browser = client_trusting(&ca_pem(&proxy));
    let target = format!("{HOST}:{}", origin.port());
    let req = format!("POST /up HTTP/1.1\r\nHost: {target}\r\nContent-Length: {n}\r\n\r\n");
    let (mut direct, mut dec) = (Vec::new(), Vec::new());
    for _ in 0..5 {
        let t = Instant::now();
        let tcp = tokio::net::TcpStream::connect(origin).await.unwrap();
        let mut s = tokio_rustls::TlsConnector::from(pki.client.clone())
            .connect(ServerName::try_from(HOST).unwrap(), tcp)
            .await
            .unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        send_len(&mut s, n).await;
        assert_eq!(read_reply(&mut s).await, format!("got {n} ok"));
        direct.push(n as f64 / MIB as f64 / t.elapsed().as_secs_f64());

        let t = Instant::now();
        let mut s = tls_via(proxy.addr(), &target, HOST, browser.clone())
            .await
            .unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        send_len(&mut s, n).await;
        assert_eq!(read_reply(&mut s).await, format!("got {n} ok"));
        dec.push(n as f64 / MIB as f64 / t.elapsed().as_secs_f64());
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let (d, x) = (med(&mut direct), med(&mut dec));
    eprintln!(
        "PERF upload: {} MiB per run, 5 runs, median direct {d:.0} MiB/s, decrypted {x:.0} MiB/s, ratio {:.1} % (direct {direct:.0?}, decrypted {dec:.0?})",
        n / MIB,
        100.0 * x / d
    );
}
