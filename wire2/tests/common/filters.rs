//! W2-F test filters: the `wire2-pass` binary, the test-only pass-decrypt filter (the library's pass
//! code with `layer: http`, run in-process on its own runtime) and a scripted fake filter that records
//! what it receives and can forward, answer, rewrite, delay, hang, close or send malformed HTTP.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use super::Proxy;

pub const PASS_BIN: &str = env!("CARGO_BIN_EXE_wire2-pass");

/// A private (0700) directory for filter sockets.
pub fn sock_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// Start the proxy with these filters (in chain order) plus `args`.
pub fn proxy_with(filters: &[&Path], args: &[&str], envs: &[(&str, &Path)]) -> Proxy {
    let mut full: Vec<String> = Vec::new();
    for f in filters {
        full.push("--filter".into());
        full.push(format!("unix:{}", f.display()));
    }
    // Quiet by default so tests that count handshakes stay exact; a test may pass its own.
    full.push("--filter-check".into());
    full.push("1h".into());
    full.extend(args.iter().map(|s| s.to_string()));
    let refs: Vec<&str> = full.iter().map(|s| s.as_str()).collect();
    Proxy::start_env(&refs, envs)
}

/// Wait until the proxy logs `filter <name> up`.
pub fn wait_up(p: &Proxy, name: &str) {
    let pat = format!("filter {name} up");
    p.wait_log(|l| l.contains(&pat), Duration::from_secs(10))
        .unwrap_or_else(|| panic!("filter {name} never came up: {}", p.log()));
}

// ---------------------------------------------------------------------------------------------------
// wire2-pass, the shipped filter, as a child process

pub struct PassBin {
    child: Child,
    pub path: PathBuf,
    pub lines: Arc<Mutex<Vec<String>>>,
}

impl PassBin {
    pub fn start(path: &Path) -> PassBin {
        let mut child = Command::new(PASS_BIN)
            .arg("--socket")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn wire2-pass");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let err = child.stderr.take().unwrap();
        let l2 = lines.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines() {
                let Ok(line) = line else { break };
                if std::env::var_os("W2_TEST_ECHO_LOG").is_some() {
                    eprintln!("[pass] {line}");
                }
                l2.lock().unwrap().push(line);
            }
        });
        let end = Instant::now() + Duration::from_secs(10);
        while !lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("listening on"))
        {
            assert!(Instant::now() < end, "wire2-pass did not start");
            std::thread::sleep(Duration::from_millis(5));
        }
        PassBin {
            child,
            path: path.to_path_buf(),
            lines,
        }
    }

    pub fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    pub fn rss_kb(&self) -> u64 {
        super::rss_kb(self.pid())
    }

    pub fn log(&self) -> String {
        self.lines.lock().unwrap().join("\n")
    }

    /// SIGTERM and wait; returns the exit status.
    pub fn term(mut self) -> std::process::ExitStatus {
        // SAFETY: signalling our own child by PID.
        unsafe { libc::kill(self.pid(), libc::SIGTERM) };
        self.child.wait().unwrap()
    }
}

impl Drop for PassBin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------------------------------
// In-process filters on their own thread and runtime

/// Runs `serve` on a dedicated multi-thread runtime until dropped.
pub struct InProc {
    pub path: PathBuf,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl InProc {
    fn spawn<F, Fut>(path: &Path, serve: F) -> InProc
    where
        F: FnOnce(UnixListener) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()>,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let p = path.to_path_buf();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            let p2 = p.clone();
            rt.block_on(async move {
                let l = UnixListener::bind(&p2).unwrap();
                ready_tx.send(()).unwrap();
                tokio::select! {
                    _ = serve(l) => {}
                    _ = rx => {}
                }
            });
            rt.shutdown_background();
            let _ = std::fs::remove_file(&p);
        });
        ready_rx.recv().unwrap();
        InProc {
            path: path.to_path_buf(),
            stop: Some(tx),
            thread: Some(thread),
        }
    }
}

impl Drop for InProc {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The test-only pass-decrypt filter: pass's forwarding code with `layer: http` for `hosts`.
pub fn pass_decrypt(path: &Path, hosts: &[&str]) -> InProc {
    let hello = wire2::pass::Hello {
        name: "pass-decrypt".into(),
        layer: "http".into(),
        hosts: hosts.iter().map(|s| s.to_string()).collect(),
        fail: "open".into(),
    };
    InProc::spawn(path, move |l| async move {
        let _ = wire2::pass::serve(l, hello, false).await;
    })
}

/// A proxy with pass-decrypt (in-process, socket in its own dir) claiming `hosts`, up before return.
pub struct Decrypting {
    pub proxy: Proxy,
    pub filter: InProc,
    _dir: tempfile::TempDir,
}

impl std::ops::Deref for Decrypting {
    type Target = Proxy;
    fn deref(&self) -> &Proxy {
        &self.proxy
    }
}

pub fn decrypting(hosts: &[&str], args: &[&str], envs: &[(&str, &Path)]) -> Decrypting {
    let dir = sock_dir();
    let path = dir.path().join("pass-decrypt.sock");
    let filter = pass_decrypt(&path, hosts);
    let proxy = proxy_with(&[&path], args, envs);
    wait_up(&proxy, "pass-decrypt");
    Decrypting {
        proxy,
        filter,
        _dir: dir,
    }
}

// ---------------------------------------------------------------------------------------------------
// The scripted fake filter

#[derive(Clone, Debug)]
pub enum Action {
    /// Forward unchanged to the hub and pipe both ways.
    Forward,
    /// Forward with these raw header lines (each ending in \r\n) added.
    ForwardAdding(String),
    /// Forward with the Proxy-Flow value replaced.
    ForwardWithFlow(String),
    /// Answer itself: status, content type, body (Content-Length framing).
    Answer(u16, String, Vec<u8>),
    /// Forward, then send the response body uppercased, re-chunked in 3-byte chunks.
    Uppercase,
    /// Wait, then forward.
    Delay(Duration),
    /// Never answer (keep the connection open).
    Hang,
    /// Close without answering.
    Close,
    /// Answer with bytes that are not HTTP.
    Malformed,
    /// Forward, and close everything this long after the request arrived.
    CloseAfter(Duration),
    /// Forward; the FIRST traffic request's connection is closed this long after it arrived, every other
    /// one keeps being piped (W2-2e: a filter ending exactly the connection it carried).
    CloseFirstAfter(Duration),
}

#[derive(Clone)]
pub struct FakeState {
    /// Raw heads of traffic requests, in arrival order.
    pub seen: Arc<Mutex<Vec<String>>>,
    /// Raw heads of handshakes.
    pub hellos: Arc<Mutex<Vec<String>>>,
    pub action: Arc<Mutex<Action>>,
    /// Handshake answer: (status, body).
    pub hello: Arc<Mutex<(u16, String)>>,
    pub hub: Arc<Mutex<Option<PathBuf>>>,
    pub open: Arc<AtomicUsize>,
}

pub struct Fake {
    pub st: FakeState,
    pub inproc: InProc,
}

impl std::ops::Deref for Fake {
    type Target = FakeState;
    fn deref(&self) -> &FakeState {
        &self.st
    }
}

pub fn hello_json(name: &str, layer: &str, hosts: &[&str], fail: &str) -> String {
    let hosts: Vec<String> = hosts.iter().map(|h| format!("{h:?}")).collect();
    format!(
        r#"{{"name":"{name}","version":1,"layer":"{layer}","hosts":[{}],"fail":"{fail}"}}"#,
        hosts.join(",")
    )
}

impl Fake {
    pub fn start(path: &Path, hello: String, action: Action) -> Fake {
        let st = FakeState {
            seen: Arc::default(),
            hellos: Arc::default(),
            action: Arc::new(Mutex::new(action)),
            hello: Arc::new(Mutex::new((200, hello))),
            hub: Arc::default(),
            open: Arc::default(),
        };
        let st2 = st.clone();
        let inproc = InProc::spawn(path, move |l| async move {
            loop {
                let Ok((s, _)) = l.accept().await else { break };
                tokio::spawn(fake_conn(s, st2.clone()));
            }
        });
        Fake { st, inproc }
    }

    pub fn set(&self, a: Action) {
        *self.action.lock().unwrap() = a;
    }

    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    pub fn hello_count(&self) -> usize {
        self.hellos.lock().unwrap().len()
    }

    pub fn hub(&self) -> PathBuf {
        self.hub.lock().unwrap().clone().expect("no handshake yet")
    }
}

pub fn head_header(head: &str, name: &str) -> Vec<String> {
    head.lines()
        .skip(1)
        .filter_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
        .collect()
}

async fn read_raw_head<R: AsyncRead + Unpin>(r: &mut R) -> Option<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(i + 4);
            return Some((String::from_utf8_lossy(&buf).into_owned(), rest));
        }
        let n = r.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

async fn fake_conn(mut s: UnixStream, st: FakeState) {
    let Some((head, rest)) = read_raw_head(&mut s).await else {
        return;
    };
    if !head_header(&head, "proxy-id").is_empty() {
        if let Some(h) = head_header(&head, "proxy-hub").first() {
            *st.hub.lock().unwrap() = h.strip_prefix("unix:").map(PathBuf::from);
        }
        st.hellos.lock().unwrap().push(head);
        let (code, body) = st.hello.lock().unwrap().clone();
        let resp = format!(
            "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = s.write_all(resp.as_bytes()).await;
        return;
    }
    let nth = {
        let mut seen = st.seen.lock().unwrap();
        seen.push(head.clone());
        seen.len()
    };
    let action = st.action.lock().unwrap().clone();
    st.open.fetch_add(1, Ordering::SeqCst);
    let _guard = Dec(st.open.clone());
    let hub = st.hub.lock().unwrap().clone();
    let forward = |head: String| async move {
        let mut h = UnixStream::connect(hub.expect("hub")).await.unwrap();
        h.write_all(head.as_bytes()).await.unwrap();
        h.write_all(&rest).await.unwrap();
        h
    };
    match action {
        Action::Forward | Action::Delay(_) | Action::CloseAfter(_) | Action::CloseFirstAfter(_) => {
            if let Action::Delay(d) = action {
                tokio::time::sleep(d).await;
            }
            let mut h = forward(head).await;
            let pipe = tokio::io::copy_bidirectional(&mut s, &mut h);
            if let Action::CloseAfter(d) = action {
                let _ = tokio::time::timeout(d, pipe).await;
            } else if let (Action::CloseFirstAfter(d), 1) = (&action, nth) {
                let _ = tokio::time::timeout(*d, pipe).await;
            } else {
                let _ = pipe.await;
            }
        }
        Action::ForwardAdding(extra) => {
            let head = format!("{}{extra}\r\n", &head[..head.len() - 2]);
            let mut h = forward(head).await;
            let _ = tokio::io::copy_bidirectional(&mut s, &mut h).await;
        }
        Action::ForwardWithFlow(v) => {
            let head: Vec<String> = head
                .split("\r\n")
                .map(|l| {
                    if l.to_ascii_lowercase().starts_with("proxy-flow:") {
                        format!("Proxy-Flow: {v}")
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            let mut h = forward(head.join("\r\n")).await;
            let _ = tokio::io::copy_bidirectional(&mut s, &mut h).await;
        }
        Action::Answer(code, ctype, body) => {
            let mut out = format!(
                "HTTP/1.1 {code} Filtered\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            out.extend_from_slice(&body);
            let _ = s.write_all(&out).await;
            let _ = s.shutdown().await;
        }
        Action::Uppercase => {
            let mut h = forward(head).await;
            let (rh, mut rest) = read_raw_head(&mut h).await.expect("response head");
            let chunked = head_header(&rh, "transfer-encoding")
                .iter()
                .any(|v| v.eq_ignore_ascii_case("chunked"));
            let body = if chunked {
                loop {
                    if let Some(b) = dechunk_complete(&rest) {
                        break b;
                    }
                    let mut tmp = [0u8; 8192];
                    let n = h.read(&mut tmp).await.unwrap();
                    assert!(n > 0);
                    rest.extend_from_slice(&tmp[..n]);
                }
            } else {
                let n: usize = head_header(&rh, "content-length")[0].parse().unwrap();
                while rest.len() < n {
                    let mut tmp = [0u8; 8192];
                    let k = h.read(&mut tmp).await.unwrap();
                    assert!(k > 0);
                    rest.extend_from_slice(&tmp[..k]);
                }
                rest.truncate(n);
                rest
            };
            let upper = body.to_ascii_uppercase();
            let mut out = String::new();
            for l in rh.trim_end().split("\r\n") {
                let low = l.to_ascii_lowercase();
                if low.starts_with("content-length:") || low.starts_with("transfer-encoding:") {
                    continue;
                }
                out.push_str(l);
                out.push_str("\r\n");
            }
            out.push_str("Transfer-Encoding: chunked\r\n\r\n");
            let mut bytes = out.into_bytes();
            for c in upper.chunks(3) {
                bytes.extend_from_slice(format!("{:x}\r\n", c.len()).as_bytes());
                bytes.extend_from_slice(c);
                bytes.extend_from_slice(b"\r\n");
            }
            bytes.extend_from_slice(b"0\r\n\r\n");
            let _ = s.write_all(&bytes).await;
            let _ = s.shutdown().await;
        }
        Action::Hang => {
            let mut sink = [0u8; 1024];
            while let Ok(n) = s.read(&mut sink).await {
                if n == 0 {
                    break;
                }
            }
        }
        Action::Close => drop(s),
        Action::Malformed => {
            let _ = s.write_all(b"this is not HTTP at all\r\n\r\n").await;
            let _ = s.shutdown().await;
        }
    }
}

/// Decode a chunked body if `b` holds all of it (extensions and trailers skipped); None if incomplete.
pub fn dechunk_complete(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n")?;
        let line = std::str::from_utf8(&b[..eol]).ok()?;
        let n = usize::from_str_radix(line.split(';').next()?.trim(), 16).ok()?;
        b = &b[eol + 2..];
        if n == 0 {
            // Trailers end with an empty line.
            return b.windows(2).any(|w| w == b"\r\n").then_some(out);
        }
        if b.len() < n + 2 {
            return None;
        }
        out.extend_from_slice(&b[..n]);
        b = &b[n + 2..];
    }
}

struct Dec(Arc<AtomicUsize>);

impl Drop for Dec {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
