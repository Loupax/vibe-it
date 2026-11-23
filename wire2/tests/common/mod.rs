//! Shared helpers for the wire2 integration tests: start the proxy binary, read its log, small test
//! servers (echo, TLS with a test root, WebSocket), and raw HTTP helpers.
#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls;

pub mod filters;
#[allow(unused_imports)]
pub use filters::*;

pub const BIN: &str = env!("CARGO_BIN_EXE_wire2");

/// A running wire2 with its own temp dir (used as XDG_DATA_HOME, so a CA would land there).
pub struct Proxy {
    child: Child,
    pub addr: Option<SocketAddr>,
    pub lines: Arc<Mutex<Vec<String>>>,
    pub dir: tempfile::TempDir,
}

impl Proxy {
    pub fn start(args: &[&str]) -> Proxy {
        Proxy::start_env(args, &[])
    }

    /// Like [`Proxy::start`] with extra environment variables (e.g. `SSL_CERT_FILE` for the upstream
    /// roots of decrypted tests).
    pub fn start_env(args: &[&str], envs: &[(&str, &Path)]) -> Proxy {
        let dir = tempfile::tempdir().unwrap();
        // XDG_RUNTIME_DIR: private, like the real one (the hub socket goes there).
        std::fs::create_dir(dir.path().join("run")).unwrap();
        let mut full: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        if !args.contains(&"--listen") {
            full.extend(["--listen".into(), "127.0.0.1:0".into()]);
        }
        // WIRE2_TEST_BIN: measure another build (a "before" binary) with the same tests.
        let bin = std::env::var("WIRE2_TEST_BIN").unwrap_or_else(|_| BIN.to_string());
        let mut child = Command::new(bin)
            .args(&full)
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("XDG_RUNTIME_DIR", dir.path().join("run"))
            .envs(envs.iter().map(|(k, v)| (*k, *v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn wire2");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let err = child.stderr.take().unwrap();
        let l2 = lines.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines() {
                let Ok(line) = line else { break };
                if std::env::var_os("W2_TEST_ECHO_LOG").is_some() {
                    eprintln!("[wire2] {line}");
                }
                l2.lock().unwrap().push(line);
            }
        });
        let mut p = Proxy {
            child,
            addr: None,
            lines,
            dir,
        };
        let line = p
            .wait_log(|l| l.contains("listening on "), Duration::from_secs(10))
            .unwrap_or_else(|| {
                panic!("wire2 did not start: {:?}", p.lines.lock().unwrap());
            });
        let after = line.split("listening on ").nth(1).unwrap();
        let addr = after.split_whitespace().next().unwrap();
        p.addr = addr.parse().ok();
        p
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr.expect("tcp listener")
    }

    pub fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    pub fn wait_log(&self, pred: impl Fn(&str) -> bool, timeout: Duration) -> Option<String> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(l) = self.lines.lock().unwrap().iter().find(|l| pred(l)) {
                return Some(l.clone());
            }
            if Instant::now() > end {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn log(&self) -> String {
        self.lines.lock().unwrap().join("\n")
    }

    /// Resident set size in KiB.
    pub fn rss_kb(&self) -> u64 {
        rss_kb(self.pid())
    }

    pub fn signal(&self, sig: i32) {
        // SAFETY: signalling our own child by PID.
        unsafe { libc::kill(self.pid(), sig) };
    }

    pub fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn rss_kb(pid: i32) -> u64 {
    let s = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// All files below `dir` (recursively).
pub fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------
// Raw HTTP helpers

/// Read until the end of an HTTP head; returns (head text, bytes after it).
pub async fn read_head<R: AsyncRead + Unpin>(r: &mut R) -> (String, Vec<u8>) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(i + 4);
            return (String::from_utf8_lossy(&buf).into_owned(), rest);
        }
        let n = r.read(&mut tmp).await.expect("read head");
        assert!(
            n > 0,
            "EOF before end of head; got {:?}",
            String::from_utf8_lossy(&buf)
        );
        buf.extend_from_slice(&tmp[..n]);
    }
}

pub fn status_of(head: &str) -> u16 {
    head.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Value of the first header `name` in a head (case-insensitive).
pub fn header_of(head: &str, name: &str) -> Option<String> {
    head.lines().skip(1).find_map(|l| {
        let (n, v) = l.split_once(':')?;
        n.trim()
            .eq_ignore_ascii_case(name)
            .then(|| v.trim().to_string())
    })
}

pub fn has_header(head: &str, name: &str) -> bool {
    header_of(head, name).is_some()
}

pub async fn read_to_end<R: AsyncRead + Unpin>(r: &mut R, mut pre: Vec<u8>) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(20), r.read_to_end(&mut pre))
        .await
        .expect("read_to_end timed out")
        .unwrap();
    pre
}

/// Open a tunnel through the proxy; returns the stream, the proxy's answer head and any bytes after it.
pub async fn connect_via(proxy: SocketAddr, target: &str) -> (TcpStream, String, Vec<u8>) {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let (head, rest) = read_head(&mut s).await;
    (s, head, rest)
}

// ---------------------------------------------------------------------------------------------------
// Servers

/// Accept forever; each connection is handled by `f` on its own task.
pub async fn serve<F, Fut>(addr: &str, f: F) -> SocketAddr
where
    F: Fn(TcpStream) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let l = TcpListener::bind(addr).await.unwrap();
    let a = l.local_addr().unwrap();
    let f = Arc::new(f);
    tokio::spawn(async move {
        while let Ok((s, _)) = l.accept().await {
            tokio::spawn(f(s));
        }
    });
    a
}

pub async fn echo_server(addr: &str) -> SocketAddr {
    serve(addr, |mut s| async move {
        let (mut r, mut w) = s.split();
        let _ = tokio::io::copy(&mut r, &mut w).await;
        let _ = w.shutdown().await;
    })
    .await
}

/// A port that refuses connections (bound, then closed).
pub fn refused_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

/// A listener whose accept queue is full, so new connects hang (SYNs dropped). Keep the returned value
/// alive while testing. None if the kernel kept accepting.
pub struct Blackhole {
    pub addr: SocketAddr,
    _fd: std::os::fd::OwnedFd,
    _held: Vec<std::net::TcpStream>,
}

pub fn blackhole() -> Option<Blackhole> {
    use std::os::fd::FromRawFd;
    // SAFETY: plain socket calls on a fresh fd we own.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    assert!(fd >= 0);
    let owned = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sa.sin_family = libc::AF_INET as libc::sa_family_t;
    sa.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
    let len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    unsafe {
        assert_eq!(
            libc::bind(fd, (&sa as *const libc::sockaddr_in).cast(), len),
            0
        );
        assert_eq!(libc::listen(fd, 0), 0);
        let mut l = len;
        assert_eq!(
            libc::getsockname(fd, (&mut sa as *mut libc::sockaddr_in).cast(), &mut l),
            0
        );
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], u16::from_be(sa.sin_port)));
    let mut held = Vec::new();
    for _ in 0..16 {
        match std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(300)) {
            Ok(s) => held.push(s),
            Err(_) => {
                return Some(Blackhole {
                    addr,
                    _fd: owned,
                    _held: held,
                });
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------------
// TLS test PKI

pub struct Pki {
    pub ca_pem: String,
    pub server: Arc<rustls::ServerConfig>,
    pub client: Arc<rustls::ClientConfig>,
}

pub fn pki() -> Pki {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
    let ca_key = KeyPair::generate().unwrap();
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.distinguished_name
        .push(DnType::CommonName, "wire2 test root");
    ca.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec![
        "localhost".to_string(),
        "www.example.test".to_string(),
    ])
    .unwrap()
    .signed_by(&leaf_key, &ca_cert, &ca_key)
    .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into());
    let server = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![leaf.der().clone()], key)
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca_cert.der().clone()).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Pki {
        ca_pem: ca_cert.pem(),
        server: Arc::new(server),
        client: Arc::new(client),
    }
}

/// HTTPS server answering every request with `hello over tls` and closing.
pub async fn tls_hello_server(cfg: Arc<rustls::ServerConfig>) -> SocketAddr {
    let acceptor = tokio_rustls::TlsAcceptor::from(cfg);
    serve("127.0.0.1:0", move |s| {
        let acceptor = acceptor.clone();
        async move {
            let Ok(mut tls) = acceptor.accept(s).await else {
                return;
            };
            let _ = read_head(&mut tls).await;
            let body = b"hello over tls";
            let _ = tls
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
            let _ = tls.write_all(body).await;
            let _ = tls.shutdown().await;
        }
    })
    .await
}

// ---------------------------------------------------------------------------------------------------
// WebSocket (RFC 6455), just enough for an echo test

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn ws_accept(key: &str) -> String {
    base64(&sha1(
        format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
    ))
}

pub async fn ws_write<W: AsyncWrite + Unpin>(
    w: &mut W,
    opcode: u8,
    payload: &[u8],
    mask: Option<[u8; 4]>,
) {
    let mut f = vec![0x80 | opcode];
    let m = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => f.push(m | n as u8),
        n if n < 65536 => {
            f.push(m | 126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            f.push(m | 127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    match mask {
        Some(k) => {
            f.extend_from_slice(&k);
            f.extend(payload.iter().enumerate().map(|(i, b)| b ^ k[i % 4]));
        }
        None => f.extend_from_slice(payload),
    }
    w.write_all(&f).await.unwrap();
    // TLS writers keep an unsent tail until flushed.
    w.flush().await.unwrap();
}

/// Read one frame -> (opcode, unmasked payload).
pub async fn ws_read<R: AsyncRead + Unpin>(r: &mut R) -> Option<(u8, Vec<u8>)> {
    let mut h = [0u8; 2];
    r.read_exact(&mut h).await.ok()?;
    let masked = h[1] & 0x80 != 0;
    let len = match h[1] & 0x7f {
        126 => {
            let mut b = [0u8; 2];
            r.read_exact(&mut b).await.ok()?;
            u16::from_be_bytes(b) as usize
        }
        127 => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b).await.ok()?;
            u64::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    let mut key = [0u8; 4];
    if masked {
        r.read_exact(&mut key).await.ok()?;
    }
    let mut p = vec![0u8; len];
    r.read_exact(&mut p).await.ok()?;
    if masked {
        for (i, b) in p.iter_mut().enumerate() {
            *b ^= key[i % 4];
        }
    }
    Some((h[0] & 0x0f, p))
}

/// WebSocket echo server: completes the handshake, then echoes every text/binary frame until close.
pub async fn ws_echo_server() -> SocketAddr {
    serve("127.0.0.1:0", |mut s| async move {
        let (head, _) = read_head(&mut s).await;
        let key = header_of(&head, "sec-websocket-key").unwrap_or_default();
        let resp = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            ws_accept(&key)
        );
        s.write_all(resp.as_bytes()).await.unwrap();
        while let Some((op, p)) = ws_read(&mut s).await {
            if op == 8 {
                ws_write(&mut s, 8, &p, None).await;
                break;
            }
            ws_write(&mut s, op, &p, None).await;
        }
    })
    .await
}

/// Client side of a WebSocket echo session over an already-open stream to the server.
pub async fn ws_echo_session<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    request_target: &str,
    host: &str,
) {
    let key = base64(b"wire2-test-key!!");
    let req = format!(
        "GET {request_target} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let (head, rest) = read_head(s).await;
    assert_eq!(status_of(&head), 101, "{head}");
    assert_eq!(
        header_of(&head, "sec-websocket-accept").as_deref(),
        Some(ws_accept(&key).as_str())
    );
    assert!(rest.is_empty());
    let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    for (op, msg) in [(1u8, b"hello".to_vec()), (2, vec![7u8; 300]), (2, big)] {
        ws_write(s, op, &msg, Some([1, 2, 3, 4])).await;
        let (rop, got) = tokio::time::timeout(Duration::from_secs(10), ws_read(s))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((rop, got.len()), (op, msg.len()));
        assert!(got == msg, "echo differs");
    }
    ws_write(s, 8, &[0x03, 0xe8], Some([9, 9, 9, 9])).await;
    let (op, _) = ws_read(s).await.unwrap();
    assert_eq!(op, 8);
}

#[test]
fn sha1_and_base64_known_answers() {
    // RFC 6455 section 1.3 example.
    assert_eq!(
        ws_accept("dGhlIHNhbXBsZSBub25jZQ=="),
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );
    assert_eq!(base64(b"ab"), "YWI=");
}

// ---------------------------------------------------------------------------------------------------
// Browser-side TLS for decrypted tunnels (W2-2)

use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};

/// The test root as a file, for `SSL_CERT_FILE`.
pub fn root_file(pki: &Pki) -> tempfile::NamedTempFile {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), &pki.ca_pem).unwrap();
    f
}

/// A browser-side TLS client that trusts exactly the PEM certificates in `pem`.
pub fn client_trusting(pem: &str) -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    for c in pem_certs(pem) {
        roots.add(c).unwrap();
    }
    let mut cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    // Like a browser: offer h2 too; the proxy must pick http/1.1.
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Arc::new(cfg)
}

pub fn pem_certs(pem: &str) -> Vec<CertificateDer<'static>> {
    let mut out = Vec::new();
    for block in pem.split("-----BEGIN CERTIFICATE-----").skip(1) {
        let b64: String = block
            .split("-----END CERTIFICATE-----")
            .next()
            .unwrap()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        out.push(CertificateDer::from(unbase64(&b64)));
    }
    out
}

pub fn unbase64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("bad base64"),
        }
    };
    let bytes: Vec<u8> = s.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::new();
    for ch in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in ch.iter().enumerate() {
            n |= val(c) << (18 - 6 * i);
        }
        let take = ch.len() * 6 / 8;
        out.extend_from_slice(&n.to_be_bytes()[1..1 + take]);
    }
    out
}

pub fn ca_dir(p: &Proxy) -> PathBuf {
    p.data_dir().join("proxy")
}

pub fn ca_pem(p: &Proxy) -> String {
    std::fs::read_to_string(ca_dir(p).join("ca.pem")).expect("ca.pem")
}

/// CONNECT through the proxy, then TLS with `cfg` for `sni`.
pub async fn tls_via(
    proxy: std::net::SocketAddr,
    target: &str,
    sni: &str,
    cfg: Arc<rustls::ClientConfig>,
) -> Result<TlsStream<TcpStream>, std::io::Error> {
    let (s, head, rest) = connect_via(proxy, target).await;
    assert_eq!(status_of(&head), 200, "{head}");
    assert!(rest.is_empty());
    tokio_rustls::TlsConnector::from(cfg)
        .connect(ServerName::try_from(sni.to_string()).unwrap(), s)
        .await
}
