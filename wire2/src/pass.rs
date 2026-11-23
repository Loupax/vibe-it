//! The reference filter `pass` (SPEC "Reference filter: pass"), binary `wire2-pass`: an HTTP/1.1 server
//! on a Unix socket that forwards every request to the proxy's hub unchanged and copies bytes both ways
//! (bodies, tunnels and upgrades alike) with splice(2). It never buffers a body: it reads one request
//! head, writes it to the hub as it came (with whatever bytes followed it) and from then on only pipes.
//!
//! Handshake: a request is the handshake if and only if it carries `Proxy-Id`. `pass` answers
//! `{"name":"pass","version":1,"layer":"tunnel","hosts":["*"],"fail":"open"}` and remembers that
//! proxy's `Proxy-Hub`. Traffic (any other request, `Proxy-Flow: <proxy id>/<flow>`) goes to the hub of
//! the proxy that issued it. Traffic from a proxy whose handshake this process has not seen (`pass` was
//! restarted under a running proxy) is closed without an answer: the proxy then marks the filter down
//! and repeats the handshake (SPEC "Failure handling"), after which traffic flows again.
//!
//! [`serve`] is public so the integration tests can run the test-only pass-decrypt filter: the same code
//! with `layer: http` ([`Hello`]). The binary has no flag that changes the layer.

use std::os::fd::FromRawFd;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};

use crate::conn::Conn;
use crate::http::{self, HeadError};

/// A filter's handshake answer.
#[derive(Debug, Clone)]
pub struct Hello {
    pub name: String,
    /// `tunnel` or `http`.
    pub layer: String,
    pub hosts: Vec<String>,
    /// `open` or `closed`.
    pub fail: String,
}

impl Hello {
    /// What `wire2-pass` answers, always.
    pub fn pass() -> Hello {
        Hello {
            name: "pass".into(),
            layer: "tunnel".into(),
            hosts: vec!["*".into()],
            fail: "open".into(),
        }
    }

    pub fn json(&self) -> String {
        serde_json::json!({
            "name": self.name,
            "version": 1,
            "layer": self.layer,
            "hosts": self.hosts,
            "fail": self.fail,
        })
        .to_string()
    }
}

/// How long a proxy may take to send a request head.
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Proxies (by `Proxy-Id`) whose hub we know; a few, most recent last.
const MAX_PROXIES: usize = 16;

type Hubs = Arc<Mutex<Vec<(String, PathBuf)>>>;

/// Serve filter requests on `l` until the listener fails. `verbose` logs every request.
pub async fn serve(l: UnixListener, hello: Hello, verbose: bool) -> std::io::Result<()> {
    let hubs: Hubs = Arc::default();
    let json: Arc<str> = hello.json().into();
    loop {
        let (s, _) = match l.accept().await {
            Ok(x) => x,
            Err(e) if is_transient(&e) => {
                crate::warn!("{}: accept: {e}", hello.name);
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            Err(e) => return Err(e),
        };
        tokio::spawn(conn(s, json.clone(), hubs.clone(), verbose));
    }
}

fn is_transient(e: &std::io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(libc::EMFILE | libc::ENFILE | libc::ENOBUFS | libc::ENOMEM | libc::ECONNABORTED)
    )
}

fn header_str(head: &http::RequestHead, name: &str) -> Option<String> {
    http::header(&head.headers, name).map(|v| String::from_utf8_lossy(v).trim().to_string())
}

/// Read until a complete request head is in `buf` (which keeps every byte read, head included, so it
/// can be passed on unchanged).
async fn read_raw_head(c: &mut Conn, buf: &mut Vec<u8>) -> Result<http::RequestHead, HeadError> {
    let mut tmp = [0u8; 8192];
    loop {
        if !buf.is_empty() {
            match http::parse_request(buf) {
                Ok(Some((h, _))) => return Ok(h),
                Ok(None) => {}
                Err(m) => return Err(HeadError::Malformed(m)),
            }
        }
        if buf.len() >= http::MAX_HEAD {
            return Err(HeadError::TooLarge);
        }
        let n = c.read(&mut tmp).await.map_err(HeadError::Io)?;
        if n == 0 {
            return Err(if buf.is_empty() {
                HeadError::Eof
            } else {
                HeadError::Truncated
            });
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

async fn conn(s: UnixStream, json: Arc<str>, hubs: Hubs, verbose: bool) {
    let mut c = Conn::Unix(s);
    let mut buf = Vec::new();
    let head = match tokio::time::timeout(HEAD_TIMEOUT, read_raw_head(&mut c, &mut buf)).await {
        Ok(Ok(h)) => h,
        Ok(Err(HeadError::Eof)) => return,
        Ok(Err(e)) => {
            crate::warn!("bad request from the proxy: {e}");
            let _ = c
                .write_all(&http::error_response(400, "pass: malformed request"))
                .await;
            return;
        }
        Err(_) => return,
    };
    // The handshake is exactly the request that carries Proxy-Id; anything else is traffic, whatever
    // its method or path (a page's own GET /hello included).
    if let Some(id) = header_str(&head, "proxy-id") {
        let hub = header_str(&head, "proxy-hub");
        let Some(hub) = hub.as_deref().and_then(|h| h.strip_prefix("unix:")) else {
            crate::warn!("handshake from proxy {id} without a unix: Proxy-Hub; refusing");
            let _ = c
                .write_all(&http::error_response(
                    400,
                    "pass: Proxy-Hub must be unix:<path>",
                ))
                .await;
            return;
        };
        {
            let mut h = hubs.lock().unwrap_or_else(|e| e.into_inner());
            h.retain(|(i, _)| *i != id);
            if h.len() >= MAX_PROXIES {
                h.remove(0);
            }
            h.push((id.clone(), PathBuf::from(hub)));
        }
        crate::info!("handshake from proxy {id} (hub unix:{hub})");
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
            json.len()
        );
        let _ = c.write_all(resp.as_bytes()).await;
        let _ = c.shutdown().await;
        return;
    }
    let flow = header_str(&head, "proxy-flow").unwrap_or_default();
    let id = flow.split('/').next().unwrap_or("");
    let hub = {
        let h = hubs.lock().unwrap_or_else(|e| e.into_inner());
        h.iter().find(|(i, _)| i == id).map(|(_, p)| p.clone())
    };
    let Some(hub) = hub else {
        crate::info!(
            "{} {}: Proxy-Flow {flow:?} is from a proxy this filter has not shaken hands with; \
             closing so it repeats the handshake",
            head.method,
            head.target
        );
        return;
    };
    if verbose {
        crate::info!("flow {flow}: {} {}", head.method, head.target);
    }
    let up = match UnixStream::connect(&hub).await {
        Ok(u) => Conn::Unix(u),
        Err(e) => {
            // The proxy is gone (or restarted elsewhere): close; a live proxy re-handshakes.
            crate::warn!("hub unix:{}: {e}", hub.display());
            return;
        }
    };
    // The head exactly as received, plus whatever followed it; then only bytes.
    let mut up = up;
    if up.write_all(&buf).await.is_err() {
        return;
    }
    drop(buf);
    drop(head);
    let (a, b) = (AtomicU64::new(0), AtomicU64::new(0));
    if let Err(e) = crate::splice::splice_bidi(&c, &up, &a, &b).await {
        crate::debug!("flow {flow}: pipe ended: {e}");
    }
}

/// A listening socket from systemd socket activation (`LISTEN_PID` = us, `LISTEN_FDS` >= 1: fd 3).
fn systemd_listener() -> Option<std::os::unix::net::UnixListener> {
    let pid: u32 = std::env::var("LISTEN_PID").ok()?.parse().ok()?;
    let fds: u32 = std::env::var("LISTEN_FDS").ok()?.parse().ok()?;
    if pid != std::process::id() || fds < 1 {
        return None;
    }
    // SAFETY: systemd passed fd 3 to us (LISTEN_PID matches); we take ownership of it once. Clearing
    // the variables keeps them from being misread later.
    unsafe {
        std::env::remove_var("LISTEN_PID");
        std::env::remove_var("LISTEN_FDS");
        std::env::remove_var("LISTEN_FDNAMES");
        Some(std::os::unix::net::UnixListener::from_raw_fd(3))
    }
}

const USAGE: &str = "\
usage: wire2-pass [--socket PATH] [--verbose]

The reference filter `pass` for wire2 (SPEC.md \"Reference filter: pass\"): tunnel layer, all hosts,
fail open. Forwards every request to the proxy's hub unchanged and copies bytes both ways; it never
asks for plaintext, so it never makes the proxy decrypt anything.

  --socket PATH   listen on this Unix socket (created with mode 0600). Not needed under systemd
                  socket activation (LISTEN_FDS, fd 3), which is the recommended setup.
  --verbose       log every request.
  --help, --version

Logs go to stderr. SIGTERM exits.
";

pub fn main() -> ExitCode {
    let mut socket: Option<PathBuf> = None;
    let mut verbose = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        match flag.as_str() {
            "--help" | "-h" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("wire2-pass {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--verbose" | "-v" => verbose = true,
            "--socket" => match inline.or_else(|| args.next()) {
                Some(p) if !p.is_empty() => socket = Some(PathBuf::from(p)),
                _ => {
                    eprintln!("wire2-pass: --socket needs a path\n\n{USAGE}");
                    return ExitCode::from(2);
                }
            },
            _ => {
                eprintln!("wire2-pass: unknown argument {a:?}\n\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let activated = systemd_listener();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            crate::error!("cannot start the runtime: {e}");
            return ExitCode::from(1);
        }
    };
    rt.block_on(async move {
        let (l, shown) = match (activated, &socket) {
            (Some(l), _) => {
                let l = l
                    .set_nonblocking(true)
                    .and_then(|_| UnixListener::from_std(l));
                match l {
                    Ok(l) => (l, "the systemd socket (fd 3)".to_string()),
                    Err(e) => {
                        crate::error!("systemd socket: {e}");
                        return ExitCode::from(1);
                    }
                }
            }
            (None, Some(p)) => match crate::conn::bind_unix("--socket", p) {
                Ok(l) => (l, format!("unix:{}", p.display())),
                Err(e) => {
                    crate::error!("{e}");
                    return ExitCode::from(1);
                }
            },
            (None, None) => {
                eprintln!("wire2-pass: no socket: run under systemd socket activation or pass --socket\n\n{USAGE}");
                return ExitCode::from(2);
            }
        };
        crate::info!(
            "pass {} listening on {shown} (layer tunnel, all hosts, fail open)",
            env!("CARGO_PKG_VERSION")
        );
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                crate::error!("SIGTERM handler: {e}");
                return ExitCode::from(1);
            }
        };
        let mut int = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(e) => {
                crate::error!("SIGINT handler: {e}");
                return ExitCode::from(1);
            }
        };
        let code = tokio::select! {
            r = serve(l, Hello::pass(), verbose) => match r {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => { crate::error!("accept: {e}"); ExitCode::from(1) }
            },
            _ = term.recv() => { crate::info!("SIGTERM: exiting"); ExitCode::SUCCESS }
            _ = int.recv() => ExitCode::SUCCESS,
        };
        if let Some(p) = &socket {
            let _ = std::fs::remove_file(p);
        }
        code
    })
}
