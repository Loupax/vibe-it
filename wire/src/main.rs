mod cookies;
mod mount;
mod ws;

use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use url::Url;

// `wire` — the real-networking daemon `scratchy/TODO.txt` calls for
// (2026-11-08 planning entry, "TOP PRIORITY" item): a tiny long-lived
// process, separate from both `scratchy` (no networking of its own, by
// design) and `snag` (now narrowed to exactly one job — bundling a page
// to a single file — and no longer scratchy's live-navigation
// transport). `wire` owns the one thing neither of those does: actually
// talking to a remote HTTP server. It speaks a deliberately tiny
// protocol over a Unix domain socket — read one real HTTP/1.1 request
// (proxy absolute-form request line, headers, optional body), execute
// it for real (TLS, redirects, decompression — via `ureq`, same crate
// `snag` already trusts for this), and stream back one normalized
// HTTP/1.1 response as it arrives: real headers, redirects already
// followed, body already decompressed, never `Transfer-Encoding:
// chunked` and never `Content-Length` either (see `stream_response`'s
// own docs for why — the body is never fully in this daemon's own
// memory at once, however large it is, so there's no length to state
// up front; the connection closing is the end-of-body signal instead,
// same as the protocol's request/response framing already was). No
// bundling/inlining logic lives here — that's `snag`'s exclusive job,
// which is why this file is so much smaller than `snag/src/main.rs`.
//
// Unlike `snag --serve`'s one-connection-at-a-time loop, this daemon is
// expected to field several *separate* connections concurrently (a page
// with several forms/links worth of future per-resource fetches, each
// its own one-shot connection, no multiplexing on any single one) — so
// each accepted connection gets its own thread rather than being served
// serially.

fn print_usage() {
    eprintln!("usage: wire [--socket <path>] [--print-socket] [--no-cookies] [--cookie-file <path>]");
    eprintln!("            [--mount ORIGIN=DIR]... [--mount-override PATH=FILE]...");
    eprintln!("  wire --mount http://HOST[:PORT]=DIR  serve that origin from the directory DIR, no socket");
    eprintln!("                       opened (repeatable). www./www1./www2. subdomains and PORT+1 are");
    eprintln!("                       aliases. Content-Type by extension, <file>.headers and");
    eprintln!("                       __dir__.headers, x.any.html / x.window.html generated from");
    eprintln!("                       x.any.js / x.window.js (// META: script/title/timeout/global),");
    eprintln!("                       /resources/WebIDLParser.js -> /resources/webidl2/lib/webidl2.js,");
    eprintln!("                       .sub. files substitute {{{{host}}}} {{{{domains[www]}}}} {{{{ports[http][0|1]}}}},");
    eprintln!("                       .py -> 501. For web-platform-tests, e.g.");
    eprintln!("                       --mount http://web-platform.test:8000=/path/to/wpt");
    eprintln!("  wire --mount-override PATH=FILE  answer requests for exactly PATH (on any mount) with");
    eprintln!("                       FILE, e.g. /resources/testharnessreport.js=./my-report.js");
    eprintln!("  wire                 listen on $XDG_RUNTIME_DIR/wire.sock (falling back to");
    eprintln!("                       /tmp/wire-$UID.sock), serving real HTTP requests read");
    eprintln!("                       off that socket in the absolute-form proxy shape");
    eprintln!("                       (`GET https://example.com/path HTTP/1.1`), one request");
    eprintln!("                       per connection, concurrently. Must be started manually.");
    eprintln!("  wire --cookie-file <path> keep the cookie jar in <path> (mode 0600): loaded at start,");
    eprintln!("                       saved at most once a second while it changes and on SIGTERM.");
    eprintln!("                       Session cookies (no expiry date) are not saved.");
    eprintln!("  wire --socket <path> listen on <path> instead of the default location.");
    eprintln!("  wire --no-cookies    kill switch: disable the in-memory cookie jar entirely (the");
    eprintln!("                       X-Wire-Jar header is ignored, responses carry X-Wire-Cookies: off).");
    eprintln!("  wire --print-socket  print the socket path wire would bind (honoring --socket,");
    eprintln!("                       if also given) to stdout and exit immediately — doesn't");
    eprintln!("                       bind or listen at all. Meant for scripting, so a caller");
    eprintln!("                       never has to duplicate wire's own default-path logic,");
    eprintln!("                       e.g.: scratchy --socket \"$(wire --print-socket)\" page.html");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut socket_path = None;
    let mut print_socket = false;
    let mut cookies_enabled = true;
    let mut cookie_file: Option<PathBuf> = None;
    let mut mounts = mount::Mounts::default();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_usage();
                return;
            }
            "--socket" => {
                let Some(path) = args.get(i + 1) else {
                    eprintln!("wire: --socket needs a path argument");
                    std::process::exit(1);
                };
                socket_path = Some(PathBuf::from(path));
                i += 2;
            }
            "--cookie-file" => {
                let Some(path) = args.get(i + 1) else {
                    eprintln!("wire: --cookie-file needs a path argument");
                    std::process::exit(1);
                };
                cookie_file = Some(PathBuf::from(path));
                i += 2;
            }
            "--mount" | "--mount-override" => {
                let Some(spec) = args.get(i + 1) else {
                    eprintln!("wire: {} needs an argument", args[i]);
                    std::process::exit(1);
                };
                let r = if args[i] == "--mount" { mounts.add_mount(spec) } else { mounts.add_override(spec) };
                if let Err(e) = r {
                    eprintln!("wire: {e}");
                    std::process::exit(1);
                }
                i += 2;
            }
            "--no-cookies" => {
                cookies_enabled = false;
                i += 1;
            }
            "--print-socket" => {
                print_socket = true;
                i += 1;
            }
            other => {
                eprintln!("wire: unrecognized argument {other:?}");
                print_usage();
                std::process::exit(1);
            }
        }
    }

    if cookie_file.is_some() && !cookies_enabled {
        eprintln!("wire: --cookie-file and --no-cookies contradict each other");
        std::process::exit(1);
    }
    let path = socket_path.unwrap_or_else(default_socket_path);

    // Deliberately reuses the exact same `path` value the real serving
    // path below would bind — never a second, parallel computation of
    // "what socket would wire use," which could silently drift from the
    // real answer. Exits before `run_server`/`build_agent` do any real
    // work: this is a pure path-resolution query, not a lighter-weight
    // way to start the daemon.
    if print_socket {
        println!("{}", path.display());
        return;
    }

    let agent = build_agent();
    let mut shared = Shared::new(cookies_enabled);
    shared.mounts = mounts;
    if let Some(file) = cookie_file {
        shared = shared.with_cookie_file(file);
    }
    let shared = Arc::new(shared);
    if let Err(e) = shared.write_token_file(&path) {
        eprintln!("wire: cannot write the control token {}: {e}; cookie control will not work", token_file_path(&path).display());
    }
    if shared.cookie_file.is_some() {
        start_jar_saver(&shared);
    }
    run_server(&path, agent, shared);
}

fn build_agent() -> ureq::Agent {
    agent_builder().build()
}

/// ureq's own default redirect limit; a request asking for exactly this
/// (or sending no `X-Wire-Max-Redirects`) uses the shared agent.
const DEFAULT_REDIRECTS: u32 = 5;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

fn agent_builder() -> ureq::AgentBuilder {
    ureq::AgentBuilder::new()
        .timeout(DEFAULT_TIMEOUT)
        .resolver(TrackingResolver)
        .user_agent(DEFAULT_UA)
}

/// wire's honest default User-Agent (QY39a: overridable per request).
const DEFAULT_UA: &str = "wire/0.1 (+https://github.com/Loupax/vibe-it) scratchy's real-networking daemon";

// ---------- target classes and the resolver ----------

/// Bumped when the scratchy<->wire control protocol (`X-Wire-*`) grows;
/// sent as `X-Wire-Version` on EVERY response so scratchy can tell an old
/// wire (no such header) from a new one before relying on the caps.
const WIRE_VERSION: &str = "3";

/// Comma-separated optional features, sent as `X-Wire-Features` on every
/// response (additive; absent on an older wire). `ua`: per-request User-Agent
/// override (`X-Wire-User-Agent` / the request's own `User-Agent`). `ws`: the
/// WebSocket pipe (`X-Wire-Upgrade: websocket`, QF19w1, see `ws.rs`).
/// `jarfile`: `--cookie-file` persistence exists and the control op
/// `X-Wire-Cookie-Op: clear-site` (+ `X-Wire-Top-Site`) is understood (W-PERSIST1).
/// `token`: every cookie control request must carry `X-Wire-Control-Token`
/// (read from `<socket>.token`).
const WIRE_FEATURES: &str = "ua,ws,jarfile,token";

/// The one jar name scratchy may select with `X-Wire-Jar` (QF-D2a: ONE shared
/// jar; a later `--profile NAME` changes this single value).
const JAR_NAME: &str = "scratchy";

/// Daemon-wide state shared by every connection thread.
struct Shared {
    /// `false` under `--no-cookies`.
    enabled: bool,
    jar: Mutex<cookies::Jar>,
    /// Random per start; every cookie control request must send it as
    /// `X-Wire-Control-Token`. Written to `<socket>.token` (mode 0600).
    control_token: String,
    /// `--cookie-file PATH` (W-PERSIST1): where the jar's persistent cookies live.
    cookie_file: Option<PathBuf>,
    /// Agent with redirects switched off: the jar path follows redirects
    /// itself so it can store and re-send cookies on every hop.
    manual_agent: ureq::Agent,
    /// Own agents (own connection pools) for `X-Wire-Policy: public-only`
    /// requests (W-POOL1). ureq only calls the resolver when it opens a NEW
    /// connection, so a pooled connection skips the public-only check. These
    /// pools only ever hold connections that went through the checking
    /// resolver under public-only, so reuse inside them is safe, and no plain
    /// request can leave a non-public connection in them.
    public_agent: ureq::Agent,
    public_manual_agent: ureq::Agent,
    /// `--mount` / `--mount-override` (QWPT3): origins served from directories.
    mounts: mount::Mounts,
}

impl Shared {
    fn new(enabled: bool) -> Shared {
        Shared {
            enabled,
            jar: Mutex::new(cookies::Jar::default()),
            control_token: random_token(),
            cookie_file: None,
            manual_agent: agent_builder().redirects(0).build(),
            public_agent: agent_builder().build(),
            public_manual_agent: agent_builder().redirects(0).build(),
            mounts: mount::Mounts::default(),
        }
    }

    /// Writes the control token next to the socket, replacing any old one.
    fn write_token_file(&self, socket: &std::path::Path) -> std::io::Result<()> {
        cookies::write_file_atomic(&token_file_path(socket), &format!("{}\n", self.control_token))
    }

    /// Loads the jar from `path` and keeps writing it there. A missing file
    /// is a fresh jar; an unreadable or corrupt one is ONE log line and an
    /// empty jar (the next save replaces it).
    fn with_cookie_file(mut self, path: PathBuf) -> Shared {
        match std::fs::read(&path) {
            Ok(bytes) => match String::from_utf8(bytes).map_err(|_| "not UTF-8".to_string()).and_then(|t| cookies::Jar::from_file_text(&t, now_unix())) {
                Ok(jar) => *self.jar.get_mut().unwrap_or_else(|e| e.into_inner()) = jar,
                Err(why) => eprintln!("wire: cookie file {} is corrupt ({why}); starting with an empty jar", path.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("wire: cannot read cookie file {} ({e}); starting with an empty jar", path.display()),
        }
        self.cookie_file = Some(path);
        self
    }

    /// Writes the jar to the cookie file when it changed (`force`: even when
    /// it did not). A failed write is logged once and retried at the next call.
    fn save_jar(&self, force: bool) {
        let Some(path) = &self.cookie_file else { return };
        let text = {
            let mut jar = self.jar();
            if !jar.take_dirty() && !force {
                return;
            }
            jar.to_file_text(now_unix())
        };
        if let Err(e) = cookies::write_file_atomic(path, &text) {
            self.jar().mark_dirty();
            if !JAR_SAVE_FAILED.swap(true, Ordering::Relaxed) {
                eprintln!("wire: cannot save cookie file {}: {e}", path.display());
            }
        } else {
            JAR_SAVE_FAILED.store(false, Ordering::Relaxed);
        }
    }

    fn jar(&self) -> std::sync::MutexGuard<'_, cookies::Jar> {
        self.jar.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 32 random bytes from the kernel, as 64 hex digits.
fn random_token() -> String {
    let mut buf = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).expect("wire: cannot read /dev/urandom for the control token");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// `<socket path>.token`
fn token_file_path(socket: &std::path::Path) -> PathBuf {
    let mut p = socket.as_os_str().to_os_string();
    p.push(".token");
    p.into()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

static JAR_SAVE_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SIGTERM_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

extern "C" fn on_sigterm(_: i32) {
    SIGTERM_SEEN.store(true, Ordering::Relaxed); // async-signal-safe: one atomic store
}

/// W-PERSIST1: a thread that saves the jar at most once a second while it has
/// changes, and on SIGTERM saves once more and exits. (The handler only sets a
/// flag; this thread does the work, polling every 100 ms.)
fn start_jar_saver(shared: &Arc<Shared>) {
    // SAFETY: installs an async-signal-safe handler (a single atomic store).
    unsafe {
        signal(15, on_sigterm as *const () as usize);
    }
    let shared = Arc::clone(shared);
    std::thread::spawn(move || {
        let mut last = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(100));
            if SIGTERM_SEEN.load(Ordering::Relaxed) {
                shared.save_jar(false);
                std::process::exit(0);
            }
            if last.elapsed() >= Duration::from_secs(1) {
                shared.save_jar(false);
                last = Instant::now();
            }
        }
    });
}

thread_local! {
    /// Value of the `X-Wire-Cookies` response header for the request this
    /// thread is serving ("" = no header). Set at the start of each request.
    static COOKIES_HDR: Cell<&'static str> = const { Cell::new("") };
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// Mirror of scratchy's `net_policy::is_public_ip`: keep both identical.
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(a: Ipv4Addr) -> bool {
    let o = a.octets();
    !(a.is_loopback()
        || a.is_private()
        || a.is_link_local()
        || a.is_unspecified()
        || a.is_broadcast()
        || a.is_multicast()
        || a.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // CGNAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (o[1] & 0xfe) == 18)
        || o[0] >= 240)
}

fn is_public_v6(a: Ipv6Addr) -> bool {
    let s = a.segments();
    if let Some(v4) = a.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    if s[..6].iter().all(|x| *x == 0) {
        return false;
    }
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6].iter().all(|x| *x == 0) {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_public_v4(v4);
    }
    !(a.is_multicast() || (s[0] & 0xffc0) == 0xfe80 || (s[0] & 0xfe00) == 0xfc00 || (s[0] == 0x2001 && s[1] == 0x0db8))
}

/// Class name of an address: `public` or the same names as scratchy's
/// `net_policy::ip_class` (`loopback`, `private`, `link-local`, `cgnat`,
/// `ula`, `multicast`, `unspecified`, `reserved`).
fn ip_class(ip: IpAddr) -> &'static str {
    if is_public_ip(ip) {
        return "public";
    }
    let v4 = match ip {
        IpAddr::V4(a) => Some(a),
        IpAddr::V6(a) => a.to_ipv4_mapped(),
    };
    if let Some(a) = v4 {
        let o = a.octets();
        return if a.is_loopback() {
            "loopback"
        } else if a.is_private() {
            "private"
        } else if a.is_link_local() {
            "link-local"
        } else if o[0] == 100 && (o[1] & 0xc0) == 64 {
            "cgnat"
        } else if a.is_multicast() {
            "multicast"
        } else if a.is_unspecified() || o[0] == 0 {
            "unspecified"
        } else {
            "reserved"
        };
    }
    let IpAddr::V6(a) = ip else { return "reserved" };
    let s = a.segments();
    if a.is_loopback() {
        "loopback"
    } else if (s[0] & 0xffc0) == 0xfe80 {
        "link-local"
    } else if (s[0] & 0xfe00) == 0xfc00 {
        "ula"
    } else if a.is_multicast() {
        "multicast"
    } else if a.is_unspecified() {
        "unspecified"
    } else {
        "reserved"
    }
}

/// How sensitive a class is, for picking the worst one of a set.
fn class_rank(class: &str) -> u8 {
    match class {
        "public" => 0,
        "cgnat" => 1,
        "multicast" => 2,
        "unspecified" | "reserved" => 3,
        "ula" => 4,
        "link-local" => 5,
        "private" => 6,
        "loopback" => 7,
        _ => 8,
    }
}

/// Per-request resolution record. Each request runs on its own thread and
/// ureq resolves synchronously on the calling thread (also for every
/// redirect hop), so a thread-local is enough to hand the shared agent's
/// resolver per-request state without one agent per request.
#[derive(Default)]
struct Tracker {
    public_only: bool,
    /// (netloc key, worst class among its addresses) for each hop, in order.
    hops: Vec<(String, &'static str)>,
    /// Set when `public_only` refused a host.
    blocked: Option<String>,
}

thread_local! {
    static TRACKER: RefCell<Tracker> = RefCell::new(Tracker::default());
}

/// The resolver every agent uses. Plain `ToSocketAddrs` resolution, plus:
/// it records the class of what it resolved (ureq tries these very
/// addresses, so the class describes what wire connects to: when a name
/// resolves to a mix, the worst class is reported), and under
/// `X-Wire-Policy: public-only` it drops non-public addresses BEFORE any
/// connect and fails when none are left. ureq connects to the returned
/// addresses only, so a DNS rebind between check and connect is impossible.
struct TrackingResolver;

/// Class of every (host, port) the resolver has resolved, so a request that
/// reuses a pooled connection (the resolver is not called) still reports
/// `X-Wire-Target-Class` / `X-Wire-Hop-Classes` (W-POOL1). Bounded and
/// expiring; looking an entry up refreshes it, so it lives as long as the
/// host keeps being used. A miss only means the class headers are omitted.
/// The key is `host:port` (ureq's netloc; the scheme is implied by the port
/// for the default ports).
const CLASS_MAP_MAX: usize = 1024;
const CLASS_MAP_TTL: Duration = Duration::from_secs(600);

type ClassEntry = (String, &'static str, Instant);
static CLASS_MAP: Mutex<Vec<ClassEntry>> = Mutex::new(Vec::new());

fn class_map() -> std::sync::MutexGuard<'static, Vec<ClassEntry>> {
    CLASS_MAP.lock().unwrap_or_else(|e| e.into_inner())
}

fn class_map_put(key: &str, class: &'static str) {
    let now = Instant::now();
    let mut m = class_map();
    m.retain(|e| now.duration_since(e.2) < CLASS_MAP_TTL);
    if let Some(e) = m.iter_mut().find(|e| e.0 == key) {
        e.1 = class;
        e.2 = now;
        return;
    }
    if m.len() >= CLASS_MAP_MAX
        && let Some(i) = m.iter().enumerate().min_by_key(|(_, e)| e.2).map(|(i, _)| i)
    {
        m.remove(i);
    }
    m.push((key.to_string(), class, now));
}

fn class_map_get(key: &str) -> Option<&'static str> {
    let now = Instant::now();
    let mut m = class_map();
    let e = m.iter_mut().find(|e| e.0 == key && now.duration_since(e.2) < CLASS_MAP_TTL)?;
    e.2 = now;
    Some(e.1)
}

/// The key ureq passes to the resolver for `url`: `host:port`.
fn netloc_key(url: &Url) -> String {
    format!("{}:{}", url.host_str().unwrap_or(""), url.port_or_known_default().unwrap_or(0)).to_ascii_lowercase()
}

/// Make sure the first and last hops of this request are represented in the
/// tracker, filling from the class map the hops that reused a pooled
/// connection (no resolution happened). An intermediate redirect hop on a
/// pooled connection is not visible to wire when ureq follows redirects
/// itself (see NOTES.md); the jar path follows them itself and is exact.
fn reconcile_hops(first: &Url, final_url: &Url) {
    TRACKER.with(|t| {
        let mut t = t.borrow_mut();
        let (a, b) = (netloc_key(first), netloc_key(final_url));
        if t.hops.first().map(|h| &h.0) != Some(&a)
            && let Some(c) = class_map_get(&a)
        {
            t.hops.insert(0, (a, c));
        }
        if t.hops.last().map(|h| &h.0) != Some(&b)
            && let Some(c) = class_map_get(&b)
        {
            t.hops.push((b, c));
        }
    });
}

impl ureq::Resolver for TrackingResolver {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        let addrs: Vec<SocketAddr> = netloc.to_socket_addrs()?.collect();
        TRACKER.with(|t| {
            let mut t = t.borrow_mut();
            let worst = addrs.iter().map(|a| ip_class(a.ip())).max_by_key(|c| class_rank(c)).unwrap_or("public");
            let key = netloc.to_ascii_lowercase();
            class_map_put(&key, worst);
            t.hops.push((key, worst));
            if !t.public_only {
                return Ok(addrs);
            }
            let kept: Vec<SocketAddr> = addrs.into_iter().filter(|a| is_public_ip(a.ip())).collect();
            if kept.is_empty() {
                let host = netloc.rsplit_once(':').map_or(netloc, |(h, _)| h);
                let msg = format!("blocked: {host} resolves to a non-public address");
                t.blocked = Some(msg.clone());
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, msg));
            }
            Ok(kept)
        })
    }
}

/// The control headers scratchy sends (all optional; an old scratchy sends
/// none and gets exactly the old behaviour).
#[derive(Default, Debug)]
struct Caps {
    public_only: bool,
    max_bytes: Option<usize>,
    timeout: Option<Duration>,
    max_redirects: Option<u32>,
}

fn parse_caps(headers: &[(String, String)]) -> Caps {
    let mut caps = Caps::default();
    for (name, value) in headers {
        let v = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "x-wire-policy" => caps.public_only = v.eq_ignore_ascii_case("public-only"),
            "x-wire-max-bytes" => caps.max_bytes = v.parse().ok(),
            "x-wire-timeout-ms" => caps.timeout = v.parse().ok().map(Duration::from_millis),
            "x-wire-max-redirects" => caps.max_redirects = v.parse().ok(),
            _ => {}
        }
    }
    caps
}

/// Same `$XDG_RUNTIME_DIR`-first, per-user-tmp-fallback convention
/// `snag --serve`'s own `socket_path` already established (see its own
/// docs) — just this tool's own filename, so the two daemons never
/// collide on one socket.
fn default_socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("wire.sock");
    }
    PathBuf::from(format!("/tmp/wire-{}.sock", current_uid()))
}

fn current_uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
}

fn run_server(path: &PathBuf, agent: ureq::Agent, shared: Arc<Shared>) {
    if path.exists() {
        // Stale socket file left behind by a crashed/`kill -9`'d previous
        // run — same reasoning as `snag --serve`'s own cleanup: nothing
        // is listening behind it, so it's safe to remove rather than
        // fail `bind` forever.
        if let Err(e) = std::fs::remove_file(path) {
            eprintln!("wire: failed to remove stale socket {}: {e}", path.display());
            std::process::exit(1);
        }
    }
    let listener = match UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("wire: failed to bind {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    eprintln!("wire: listening on {}", path.display());
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let agent = agent.clone(); // cheap — `ureq::Agent` is an `Arc` handle
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || handle_connection(stream, &agent, &shared));
            }
            Err(e) => eprintln!("wire: accept error: {e}"),
        }
    }
}

/// One client connection's full lifecycle: read exactly one request,
/// execute it for real, write back exactly one normalized response, then
/// return (dropping `stream`, closing the connection — the protocol's
/// "done" signal, same shape `snag --serve` already uses). A malformed
/// request or a write failure isn't fatal to the daemon — logged to
/// stderr, this one connection just closes early, same per-connection
/// leniency `snag --serve`'s own `handle_connection` already established
/// for a different failure (there, a bad top-level fetch; here, a
/// malformed client request).
fn handle_connection(stream: UnixStream, agent: &ureq::Agent, shared: &Shared) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("wire: failed to clone connection: {e}");
            return;
        }
    });
    let mut writer = stream;
    let request = match parse_request(&mut reader) {
        Ok(Some(r)) => r,
        Ok(None) => return, // client closed without sending anything
        Err(e) => {
            eprintln!("wire: failed to read request: {e}");
            return;
        }
    };
    // QF19w1: an explicit `X-Wire-Upgrade` opts into the WebSocket pipe (never
    // for the cookie control channel); without it an Upgrade request is an
    // ordinary request, exactly as before.
    if header(&request.headers, "x-wire-upgrade").is_some() && header(&request.headers, "x-wire-cookie-op").is_none() {
        COOKIES_HDR.with(|c| c.set(if shared.enabled { "" } else { "off" }));
        if let Err(e) = ws::handle(shared, &request, reader, &mut writer)
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            eprintln!("wire: websocket: {e}");
        }
        return;
    }
    if let Err(e) = execute_with(agent, shared, &request, &mut writer)
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        eprintln!("wire: failed to write response: {e}");
    }
}

// ---------- request parsing ----------

/// One parsed client request — `method`/`url` come straight off the
/// request line's own absolute-form target (RFC 7230 §5.3.2,
/// `GET https://example.com/path HTTP/1.1`), the same framing a real
/// HTTP proxy reads, chosen specifically because it's the one standard
/// way to tell a daemon sitting behind a bare Unix socket (no DNS/TLS-
/// SNI of its own) exactly where to actually connect.
struct ParsedRequest {
    method: String,
    url: Url,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Reads one request off `reader`: the request line, headers up to the
/// blank line, then exactly `Content-Length` bytes of body (no chunked
/// *request* support — scratchy, the only real client, always knows its
/// own body's length up front; a request with no `Content-Length` at all
/// is read with an empty body, correct for `GET`). `Ok(None)` means the
/// client closed the connection before sending anything, distinct from a
/// real parse error.
fn parse_request(reader: &mut impl BufRead) -> std::io::Result<Option<ParsedRequest>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let line = line.trim_end_matches(['\r', '\n']);
    let mut parts = line.splitn(3, ' ');
    let (Some(method), Some(target), Some(_version)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(malformed(&format!("bad request line {line:?}")));
    };
    let url = Url::parse(target).map_err(|e| malformed(&format!("bad absolute-form target {target:?}: {e}")))?;

    let mut headers = Vec::new();
    loop {
        let mut header_line = String::new();
        if reader.read_line(&mut header_line)? == 0 {
            return Err(malformed("eof before end of headers"));
        }
        let header_line = header_line.trim_end_matches(['\r', '\n']);
        if header_line.is_empty() {
            break; // blank line — end of headers
        }
        let Some((name, value)) = header_line.split_once(':') else {
            return Err(malformed(&format!("bad header line {header_line:?}")));
        };
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }

    let content_length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    Ok(Some(ParsedRequest { method: method.to_string(), url, headers, body }))
}

fn malformed(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.to_string())
}

// ---------- executing the request for real ----------

/// `WIRE_TIMING=1 wire` prints one line per request — method, URL,
/// status, and two separate timings — plus keeps a running total in the
/// atomics below. Same measure-don't-guess habit, and the same env-var-
/// gated/zero-cost-when-off shape, `snag`'s own `SNAG_TIMING` already
/// established (see its own docs) — split here into the two phases that
/// actually matter for "why is this slow": `headers` is everything
/// before a response's headers are even in hand (DNS, TCP connect, the
/// TLS handshake, sending the request) — `ureq`'s own `.call()`/`.send_
/// bytes()` already returns at exactly that point, with the body still
/// a lazy, unread stream via `.into_reader()` — and `body` is how long
/// it then took to stream that body through to the client. No end-of-
/// run summary the way `snag`'s own one-shot CLI prints one at exit:
/// this is a long-lived daemon serving many connections concurrently,
/// with no one point "the run" ends. The atomics exist so a future
/// stats surface (TODO.txt) has real numbers to read from, not for
/// anything printed today.
fn timing_enabled() -> bool {
    std::env::var_os("WIRE_TIMING").is_some()
}

static REQUEST_COUNT: AtomicU32 = AtomicU32::new(0);
static HEADERS_NANOS: AtomicU64 = AtomicU64::new(0);
static BODY_NANOS: AtomicU64 = AtomicU64::new(0);

fn log_timing(method: &str, url: &Url, status: u16, headers_elapsed: Duration, body_elapsed: Duration) {
    REQUEST_COUNT.fetch_add(1, Ordering::Relaxed);
    HEADERS_NANOS.fetch_add(headers_elapsed.as_nanos() as u64, Ordering::Relaxed);
    BODY_NANOS.fetch_add(body_elapsed.as_nanos() as u64, Ordering::Relaxed);
    if timing_enabled() {
        eprintln!(
            "wire: TIMING headers={:>7.1}ms body={:>7.1}ms status={status} {method} {url}",
            headers_elapsed.as_secs_f64() * 1000.0,
            body_elapsed.as_secs_f64() * 1000.0,
        );
    }
}

/// Request header names this file sets itself from `url`/`body`, or that
/// `ureq` manages internally — forwarding the client's own copy would
/// either conflict with what `ureq` is about to compute (`Host`,
/// `Content-Length`) or make no sense across the proxy hop (`Connection`,
/// which only ever described the *scratchy-to-wire* leg, not the
/// upstream one).
fn is_request_header_to_skip(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    // `X-Wire-*` is the scratchy<->wire control channel (caps, policy):
    // never forwarded upstream.
    // `User-Agent` is set separately from `resolve_user_agent` (QY39a).
    matches!(n.as_str(), "host" | "content-length" | "connection" | "user-agent") || n.starts_with("x-wire-")
}

/// Longest accepted per-request User-Agent (QY39a).
const MAX_UA_LEN: usize = 512;

/// The upstream User-Agent for a request (QY39a): `X-Wire-User-Agent` wins, then
/// the request's own `User-Agent`; `Ok(None)` keeps wire's honest default.
/// Printable ASCII (space allowed), at most `MAX_UA_LEN` bytes; anything else
/// (CR/LF, controls, non-ASCII, oversize) is `Err` and is never sent upstream.
fn resolve_user_agent(headers: &[(String, String)]) -> Result<Option<String>, ()> {
    let find = |n: &str| headers.iter().find(|(h, _)| h.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim());
    let Some(ua) = find("x-wire-user-agent").or_else(|| find("user-agent")).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if ua.len() > MAX_UA_LEN || !ua.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(());
    }
    Ok(Some(ua.to_string()))
}

/// 400 + `X-Wire-Error: bad-user-agent`.
fn write_bad_ua_response(w: &mut impl Write) -> std::io::Result<u16> {
    write_status_and_headers(
        w,
        400,
        "Bad Request",
        &[("Content-Type".to_string(), "text/plain".to_string()), ("X-Wire-Error".to_string(), "bad-user-agent".to_string())],
    )?;
    w.write_all(b"wire: invalid User-Agent (printable ASCII, max 512 bytes)")?;
    w.flush()?;
    Ok(400)
}

/// Response header names this file strips before forwarding — each one
/// describes the *wire-to-real-server* transport leg (`transfer-
/// encoding`/`content-encoding`: the body has already been de-chunked/
/// decompressed by the time any of it reaches the client) or a length
/// this daemon can no longer state up front now that the body streams
/// through as it arrives rather than being fully in hand first
/// (`content-length` — see `stream_response`'s own docs for why there's
/// nothing to put there instead, not even a recomputed one).
fn is_response_header_to_strip(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "content-length" | "transfer-encoding" | "content-encoding")
}

/// Sends `request` for real via `agent`, then streams a normalized
/// response — status/reason/headers first, then the body, as it arrives
/// — straight to `w`. Always produces *some* response (even a connection/
/// TLS/DNS failure becomes a synthesized `502`, never a hang or a
/// dropped connection with nothing explained): the one thing that can
/// now fail is writing to `w` itself (the client side), propagated to
/// the caller exactly as `write_response` used to.
#[cfg(test)]
fn execute_and_stream(agent: &ureq::Agent, request: &ParsedRequest, w: &mut impl Write) -> std::io::Result<()> {
    execute_with(agent, &Shared::new(true), request, w)
}

/// The access-token check on the cookie control channel (`X-Wire-Cookie-Op`):
/// anything that can write to wire's socket could otherwise read non-HttpOnly
/// cookies or clear the jar. The token is random per wire start and lives in
/// `<socket>.token` (mode 0600), so only the user's own processes can read it.
fn control_authorized(shared: &Shared, headers: &[(String, String)]) -> bool {
    header(headers, "x-wire-control-token").is_some_and(|t| constant_time_eq(t.trim().as_bytes(), shared.control_token.as_bytes()))
}

/// The `document.cookie` control channel: `X-Wire-Cookie-Op: get|set|clear-site` +
/// `X-Wire-Cookie-Url` (the page URL) + `X-Wire-Top-Site` + `X-Wire-Jar`.
/// Never makes an upstream request. `get` answers the non-HttpOnly cookie
/// string as text/plain; `set` takes the `document.cookie = ...` string as the
/// body and answers 204.
fn cookie_control(shared: &Shared, request: &ParsedRequest, w: &mut impl Write) -> std::io::Result<()> {
    let text_plain = || vec![("Content-Type".to_string(), "text/plain".to_string())];
    let h = &request.headers;
    if !control_authorized(shared, h) {
        eprintln!("wire: cookie control request refused: missing or wrong X-Wire-Control-Token");
        write_status_and_headers(w, 403, "Forbidden", &[("Content-Type".to_string(), "text/plain".to_string()), ("X-Wire-Error".to_string(), "cookie-control".to_string())])?;
        w.write_all(b"wire: cookie control needs X-Wire-Control-Token")?;
        return w.flush();
    }
    if !shared.enabled {
        write_status_and_headers(w, 404, "Not Found", &text_plain())?;
        w.write_all(b"wire: cookies are disabled (--no-cookies)")?;
        return w.flush();
    }
    COOKIES_HDR.with(|c| c.set("on"));
    let mut bad = |msg: &str| -> std::io::Result<()> {
        let mut hs = text_plain();
        hs.push(("X-Wire-Error".to_string(), "cookie-control".to_string()));
        write_status_and_headers(w, 400, "Bad Request", &hs)?;
        w.write_all(format!("wire: {msg}").as_bytes())?;
        w.flush()
    };
    if header(h, "x-wire-jar") != Some(JAR_NAME) {
        return bad("missing or unknown X-Wire-Jar");
    }
    if header(h, "x-wire-cookie-op") == Some("clear-site") {
        // Forget-site (STASH.md): drop one whole partition, persist at once.
        let Some(site) = header(h, "x-wire-top-site").and_then(cookies::normalize_site) else {
            return bad("clear-site needs a valid X-Wire-Top-Site");
        };
        let n = shared.jar().clear_site(&site);
        shared.save_jar(false);
        if timing_enabled() {
            eprintln!("wire: cookies clear-site removed={n}");
        }
        write_status_and_headers(w, 204, "No Content", &[])?;
        return w.flush();
    }
    let Some(page) = header(h, "x-wire-cookie-url").and_then(|u| Url::parse(u).ok()).filter(|u| u.host_str().is_some()) else {
        return bad("missing or invalid X-Wire-Cookie-Url");
    };
    let top = header(h, "x-wire-top-site");
    let ctx = cookies::RequestCtx::new(&page, "GET", top, false, None, true);
    match header(h, "x-wire-cookie-op").unwrap_or("") {
        "get" => {
            let value = shared.jar().document_cookie(&ctx, now_unix());
            write_status_and_headers(w, 200, "OK", &text_plain())?;
            w.write_all(value.as_bytes())?;
            w.flush()
        }
        "set" => {
            let line = String::from_utf8_lossy(&request.body).to_string();
            let stored = shared.jar().store(&ctx, &line, now_unix());
            if timing_enabled() {
                eprintln!("{}", cookies::log_line(&ctx.host, 0, usize::from(stored)));
            }
            write_status_and_headers(w, 204, "No Content", &[])?;
            w.flush()
        }
        _ => bad("X-Wire-Cookie-Op must be get, set or clear-site"),
    }
}

fn execute_with(agent: &ureq::Agent, shared: &Shared, request: &ParsedRequest, w: &mut impl Write) -> std::io::Result<()> {
    COOKIES_HDR.with(|c| c.set(if shared.enabled { "" } else { "off" }));
    let caps = parse_caps(&request.headers);
    if header(&request.headers, "x-wire-cookie-op").is_some() {
        return cookie_control(shared, request, w);
    }
    if request.url.scheme() == "file" {
        return serve_file(request, &caps, w);
    }
    // QWPT3: a mounted origin is answered from its directory, never from the
    // network (the operator opted in with --mount, so `public-only` does not apply).
    if let Some(m) = shared.mounts.find(&request.url) {
        let r = shared.mounts.serve(m, &request.method, &request.url);
        let mut headers = r.headers;
        headers.push(("X-Wire-Target-Class".into(), "file".into()));
        headers.push(("X-Wire-Final-Url".into(), request.url.to_string()));
        write_status_and_headers(w, r.status, r.reason, &headers)?;
        w.write_all(&r.body)?;
        w.flush()?;
        log_timing(&request.method, &request.url, r.status, Duration::ZERO, Duration::ZERO);
        return Ok(());
    }
    let Ok(ua) = resolve_user_agent(&request.headers) else {
        write_bad_ua_response(w)?;
        return Ok(());
    };
    let use_jar = shared.enabled
        && header(&request.headers, "x-wire-jar") == Some(JAR_NAME)
        && !header(&request.headers, "x-wire-cookies").is_some_and(|v| v.eq_ignore_ascii_case("off"));
    if use_jar {
        COOKIES_HDR.with(|c| c.set("on"));
        return execute_with_jar(shared, request, &caps, ua.as_deref(), w);
    }
    TRACKER.with(|t| *t.borrow_mut() = Tracker { public_only: caps.public_only, ..Tracker::default() });
    // Redirect limits are agent-level in ureq: only a request asking for a
    // non-default limit pays for its own agent (and loses connection reuse).
    let own_agent;
    let agent = match caps.max_redirects {
        Some(n) if n != DEFAULT_REDIRECTS => {
            own_agent = agent_builder().redirects(n).build();
            &own_agent
        }
        // public-only never shares a pool with plain requests (W-POOL1).
        _ if caps.public_only => &shared.public_agent,
        _ => agent,
    };
    let mut req = agent.request(&request.method, request.url.as_str());
    if let Some(t) = caps.timeout {
        req = req.timeout(t);
    }
    if let Some(ua) = &ua {
        req = req.set("User-Agent", ua);
    }
    for (name, value) in &request.headers {
        if !is_request_header_to_skip(name) {
            req = req.set(name, value);
        }
    }
    let headers_start = Instant::now();
    let result = if request.body.is_empty() { req.call() } else { req.send_bytes(&request.body) };
    let headers_elapsed = headers_start.elapsed();

    let body_start = Instant::now();
    let status = match result {
        // `ureq` treats any non-2xx status as `Err(Error::Status(..))`,
        // but the wrapped `Response` is a perfectly real upstream
        // response (a 404, a 500, ...) — not a transport failure — so it
        // streams through exactly like `Ok` does, just reached from the
        // other match arm.
        Ok(resp) | Err(ureq::Error::Status(_, resp)) => {
            let status = resp.status();
            if let Ok(final_url) = Url::parse(resp.get_url()) {
                reconcile_hops(&request.url, &final_url);
            }
            stream_response(resp, &caps, w, None, false)?;
            status
        }
        Err(ureq::Error::Transport(e)) => {
            let blocked = TRACKER.with(|t| t.borrow().blocked.clone());
            match blocked {
                Some(msg) => write_policy_response(w, &msg)?,
                None => write_synthetic_error_response(w, &e.to_string())?,
            }
        }
    };
    log_timing(&request.method, &request.url, status, headers_elapsed, body_start.elapsed());
    Ok(())
}

/// The request path when the jar is used (`X-Wire-Jar: scratchy`). Redirects
/// are followed HERE (the shared `manual_agent` never follows them) so that
/// every hop gets its own `Cookie:` header computed from the jar and every
/// hop's `Set-Cookie` lines are stored before the next hop is made.
///
/// The jar is authoritative: a client-supplied `Cookie` header is dropped.
/// `Set-Cookie` lines are stripped from the answer (the jar is wire's;
/// scratchy does not need them). Timeouts apply per hop in this path.
fn execute_with_jar(shared: &Shared, request: &ParsedRequest, caps: &Caps, ua: Option<&str>, w: &mut impl Write) -> std::io::Result<()> {
    TRACKER.with(|t| *t.borrow_mut() = Tracker { public_only: caps.public_only, ..Tracker::default() });
    let h = &request.headers;
    let top = header(h, "x-wire-top-site");
    let nav = header(h, "x-wire-nav") == Some("1");
    let initiator = header(h, "x-wire-initiator-site");
    let limit = caps.max_redirects.unwrap_or(DEFAULT_REDIRECTS);
    let (mut url, mut method, mut body) = (request.url.clone(), request.method.clone(), request.body.clone());
    let (mut sent, mut stored) = (0usize, 0usize);
    let mut hop = 0u32;
    let headers_start = Instant::now();
    let finish = |url: &Url, status: u16, sent: usize, stored: usize, start: Instant| {
        if timing_enabled() {
            eprintln!("{}", cookies::log_line(url.host_str().unwrap_or(""), sent, stored));
        }
        log_timing(&request.method, &request.url, status, start.elapsed(), Duration::ZERO);
    };
    loop {
        let ctx = cookies::RequestCtx::new(&url, &method, top, nav, initiator, false);
        let cookie = shared.jar().cookie_header(&ctx, now_unix());
        if let Some(c) = &cookie {
            sent += c.split("; ").count();
        }
        let hops_before = TRACKER.with(|t| t.borrow().hops.len());
        let manual = if caps.public_only { &shared.public_manual_agent } else { &shared.manual_agent };
        let mut req = manual.request(&method, url.as_str());
        if let Some(t) = caps.timeout {
            req = req.timeout(t);
        }
        if let Some(ua) = ua {
            req = req.set("User-Agent", ua);
        }
        let drop_body_headers = body.is_empty() && request.method != method;
        for (name, value) in h {
            if is_request_header_to_skip(name)
                || name.eq_ignore_ascii_case("cookie")
                || (drop_body_headers && name.eq_ignore_ascii_case("content-type"))
            {
                continue;
            }
            req = req.set(name, value);
        }
        if let Some(c) = &cookie {
            req = req.set("Cookie", c);
        }
        let result = if body.is_empty() { req.call() } else { req.send_bytes(&body) };
        let resp = match result {
            Ok(r) | Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(e)) => {
                let blocked = TRACKER.with(|t| t.borrow().blocked.clone());
                let status = match blocked {
                    Some(msg) => write_policy_response(w, &msg)?,
                    None => write_synthetic_error_response(w, &e.to_string())?,
                };
                finish(&url, status, sent, stored, headers_start);
                return Ok(());
            }
        };
        // A pooled connection skips the resolver: fill this hop's class from the map.
        if TRACKER.with(|t| t.borrow().hops.len()) == hops_before {
            let key = netloc_key(&url);
            if let Some(c) = class_map_get(&key) {
                TRACKER.with(|t| t.borrow_mut().hops.push((key, c)));
            }
        }
        {
            let mut jar = shared.jar();
            for line in resp.all("set-cookie") {
                if jar.store(&ctx, line, now_unix()) {
                    stored += 1;
                }
            }
        }
        let status = resp.status();
        let next = if matches!(status, 301 | 302 | 303 | 307 | 308) {
            resp.header("location").and_then(|l| url.join(l).ok()).filter(|u| matches!(u.scheme(), "http" | "https"))
        } else {
            None
        };
        let Some(next) = next else {
            stream_response(resp, caps, w, Some(url.as_str()), true)?;
            finish(&url, status, sent, stored, headers_start);
            return Ok(());
        };
        if hop >= limit {
            let code = write_synthetic_error_response(w, &format!("too many redirects (limit {limit})"))?;
            finish(&url, code, sent, stored, headers_start);
            return Ok(());
        }
        hop += 1;
        // 303 always becomes GET (HEAD stays); 301/302 turn POST into GET;
        // 307/308 keep the method and body.
        if (status == 303 && method != "HEAD") || (matches!(status, 301 | 302) && method == "POST") {
            method = "GET".to_string();
            body.clear();
        }
        url = next;
    }
}

/// `Content-Type` guessed from a path's extension, for `file://` URLs
/// (there is no upstream to tell us). Anything unknown is
/// `application/octet-stream`.
fn content_type_for_path(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("html" | "htm") => "text/html",
        Some("css") => "text/css",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Serves a `file://` URL by reading the file: `200` with a guessed
/// `Content-Type`, `404` if it can't be read, `405` for anything but
/// `GET`/`HEAD`-less use. Local files never touch `ureq`.
fn serve_file(request: &ParsedRequest, caps: &Caps, w: &mut impl Write) -> std::io::Result<()> {
    let text_plain = || vec![("Content-Type".to_string(), "text/plain".to_string())];
    let status = if caps.public_only {
        write_policy_response(w, "blocked: file: targets are not public")?
    } else if request.method != "GET" {
        write_status_and_headers(w, 405, "Method Not Allowed", &text_plain())?;
        w.write_all(b"wire: file:// URLs only support GET")?;
        405
    } else {
        let read = request.url.to_file_path().map_err(|_| "not a local file path".to_string()).and_then(|p| {
            let data = std::fs::read(&p).map_err(|e| e.to_string())?;
            Ok((p, data))
        });
        match read {
            Ok((path, data)) => {
                write_status_and_headers(
                    w,
                    200,
                    "OK",
                    &[
                        ("Content-Type".to_string(), content_type_for_path(&path).to_string()),
                        ("X-Wire-Target-Class".to_string(), "file".to_string()),
                        ("X-Wire-Final-Url".to_string(), request.url.to_string()),
                    ],
                )?;
                w.write_all(&data)?;
                200
            }
            Err(e) => {
                write_status_and_headers(w, 404, "Not Found", &text_plain())?;
                w.write_all(format!("wire: cannot read file: {e}").as_bytes())?;
                404
            }
        }
    };
    w.flush()?;
    log_timing(&request.method, &request.url, status, Duration::ZERO, Duration::ZERO);
    Ok(())
}

/// Writes a real upstream response's status/headers, then streams its
/// body straight through to `w` as it arrives — never buffered in this
/// daemon's own memory first, regardless of how large it is. `ureq`'s
/// own `gzip` feature already decompressed it transparently on the way
/// in if `Content-Encoding: gzip` was sent (the dominant real-world
/// encoding; see this crate's own `Cargo.toml` for why `brotli` isn't
/// enabled too — nothing has needed it yet), so what streams through
/// here is the real, final bytes.
///
/// **No `Content-Length`, ever — on purpose, not an oversight.** The
/// whole point of streaming is to never need the body fully in hand
/// before sending anything, and the real length simply isn't known
/// until the last byte arrives; computing one would mean going back to
/// buffering first, exactly what this exists to stop doing (a real
/// video/large-file download sitting entirely in this daemon's memory
/// before a single byte reached scratchy, for no reason). Instead this
/// relies on the one framing signal the protocol already had either
/// way: `handle_connection` closes the connection the instant this
/// function returns, and scratchy's own receiving side (`fetch_raw_via_
/// wire`) already reads until *that* EOF, never a length header — see
/// its own docs. A real HTTP/1.1 response with neither `Content-Length`
/// nor `Transfer-Encoding`, read until the connection closes, is exactly
/// this same rule, not an invented shortcut.
fn stream_response(resp: ureq::Response, caps: &Caps, w: &mut impl Write, final_url: Option<&str>, strip_set_cookie: bool) -> std::io::Result<()> {
    let status = resp.status();
    let reason = resp.status_text().to_string();
    let mut headers = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for name in resp.headers_names() {
        if is_response_header_to_strip(&name) || (strip_set_cookie && name.eq_ignore_ascii_case("set-cookie")) || seen.contains(&name.to_ascii_lowercase()) {
            continue;
        }
        seen.push(name.to_ascii_lowercase());
        // `all`, not `header`: a repeated header (two `Set-Cookie`s) must
        // reach the client as separate lines.
        for value in resp.all(&name) {
            headers.push((name.clone(), value.to_string()));
        }
    }
    // The control channel's own names must never be spoofed by upstream.
    headers.retain(|(n, _)| !n.to_ascii_lowercase().starts_with("x-wire-"));
    headers.push(("X-Wire-Final-Url".to_string(), final_url.map_or_else(|| resp.get_url().to_string(), str::to_string)));
    let hops: Vec<&'static str> = TRACKER.with(|t| t.borrow().hops.iter().map(|h| h.1).collect());
    if let Some(worst) = hops.iter().copied().max_by_key(|c| class_rank(c)) {
        headers.push(("X-Wire-Target-Class".to_string(), worst.to_string()));
        headers.push(("X-Wire-Hop-Classes".to_string(), hops.join(",")));
    }
    let mut reader = resp.into_reader();
    let Some(max) = caps.max_bytes else {
        write_status_and_headers(w, status, &reason, &headers)?;
        return copy_body(reader, w);
    };
    // `X-Wire-Max-Bytes`: headers are already out once streaming starts, so
    // a cap needs the body buffered (at most max+1 bytes: the cap itself
    // bounds memory). Over the cap -> 502 + `X-Wire-Error: too-large`.
    let mut buf = Vec::new();
    let _ = (&mut reader).take(max as u64 + 1).read_to_end(&mut buf);
    if buf.len() > max {
        let body = format!("wire: response too large (limit {max} bytes)");
        write_status_and_headers(
            w,
            502,
            "Bad Gateway",
            &[("Content-Type".to_string(), "text/plain".to_string()), ("X-Wire-Error".to_string(), "too-large".to_string())],
        )?;
        w.write_all(body.as_bytes())?;
        return w.flush();
    }
    write_status_and_headers(w, status, &reason, &headers)?;
    w.write_all(&buf)
}

/// `X-Wire-Policy: public-only` refusal: 403 + `X-Wire-Error: policy`.
fn write_policy_response(w: &mut impl Write, message: &str) -> std::io::Result<u16> {
    write_status_and_headers(
        w,
        403,
        "Forbidden",
        &[("Content-Type".to_string(), "text/plain".to_string()), ("X-Wire-Error".to_string(), "policy".to_string())],
    )?;
    w.write_all(format!("wire: {message}").as_bytes())?;
    w.flush()?;
    Ok(403)
}

/// What gets sent back when the upstream request never got far enough
/// to produce a real HTTP response at all (DNS failure, connection
/// refused, TLS handshake failure, timeout) — a synthesized `502` so
/// scratchy's own dispatch (which only ever reads `Content-Type`) still
/// gets one consistent, well-formed response shape to work with, never
/// a raw error string with no framing. Small and already fully in
/// memory as a plain `String`, so there's no streaming concern here —
/// just reuses `write_status_and_headers`/a plain `write_all`, same as
/// `stream_response`'s own shape minus the streaming. Returns the
/// status written, purely so `execute_and_stream` has something to log.
fn write_synthetic_error_response(w: &mut impl Write, message: &str) -> std::io::Result<u16> {
    let body = format!("wire: upstream request failed: {message}");
    write_status_and_headers(
        w,
        502,
        "Bad Gateway",
        &[("Content-Type".to_string(), "text/plain".to_string()), ("X-Wire-Error".to_string(), "1".to_string())],
    )?;
    w.write_all(body.as_bytes())?;
    w.flush()?;
    Ok(502)
}

fn write_status_and_headers(w: &mut impl Write, status: u16, reason: &str, headers: &[(String, String)]) -> std::io::Result<()> {
    write!(w, "HTTP/1.1 {status} {reason}\r\n")?;
    write!(w, "X-Wire-Version: {WIRE_VERSION}\r\n")?;
    write!(w, "X-Wire-Features: {WIRE_FEATURES}\r\n")?;
    let cookies_hdr = COOKIES_HDR.with(Cell::get);
    if !cookies_hdr.is_empty() {
        write!(w, "X-Wire-Cookies: {cookies_hdr}\r\n")?;
    }
    for (name, value) in headers {
        write!(w, "{name}: {value}\r\n")?;
    }
    write!(w, "\r\n")
}

/// The actual streaming copy, in fixed-size chunks — never reading the
/// whole of `reader` into memory first (`std::io::copy` would do exactly
/// that same chunked copy, but can't distinguish *which* side an error
/// came from, and the two sides need different handling here: see
/// below). 8 KiB: the same chunk size `std::io::copy`'s own default
/// buffer already uses, not a figure picked specifically for this.
fn copy_body(mut reader: impl Read, w: &mut impl Write) -> std::io::Result<()> {
    let mut buf = [0u8; 8192];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            // `Interrupted` means try the exact same read again, not
            // "the stream is done" — a signal landed mid-syscall, the
            // data this read was waiting for is still coming. `std::io::
            // Read::read_to_end`'s own default implementation (what
            // this file used before streaming existed) already retries
            // on exactly this, transparently; a hand-rolled loop doesn't
            // get that for free and has to do it explicitly.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // Any other transport error reading the body (connection
            // dropped mid-stream) isn't fatal here either — whatever
            // bytes were already streamed through stay sent, same
            // lenient spirit `execute_and_stream`'s own transport-error
            // handling already has for a failure before any bytes
            // arrived at all. Only a failure *writing to the client*
            // (`w`, below) propagates — `handle_connection`'s own
            // `BrokenPipe` handling already expects exactly that.
            Err(_) => return Ok(()),
        };
        w.write_all(&buf[..n])?;
    }
}

// ---------- tests ----------
//
// Same "no real network, no real sockets outside this process" testing
// convention `snag`'s own `#[cfg(test)]` module established: every test
// that needs a real HTTP exchange spins up its own tiny loopback
// `TcpListener` fixture (standing in for "the real remote server"), and
// the one test exercising the actual Unix-socket protocol opens its own
// throwaway socket in the OS temp dir.
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::thread;

    fn parse(input: &str) -> ParsedRequest {
        let mut reader = BufReader::new(input.as_bytes());
        parse_request(&mut reader).expect("parse_request").expect("Some(request)")
    }

    #[test]
    fn parses_a_simple_get_in_absolute_form() {
        let req = parse("GET https://example.com/path?q=1 HTTP/1.1\r\nHost: example.com\r\n\r\n");
        assert_eq!(req.method, "GET");
        assert_eq!(req.url.as_str(), "https://example.com/path?q=1");
        assert!(req.body.is_empty());
    }

    #[test]
    fn parses_headers_and_a_content_length_body() {
        let req = parse(
            "POST https://example.com/submit HTTP/1.1\r\n\
             Host: example.com\r\n\
             Content-Type: application/x-www-form-urlencoded\r\n\
             Content-Length: 7\r\n\
             \r\n\
             a=1&b=2",
        );
        assert_eq!(req.method, "POST");
        assert_eq!(req.body, b"a=1&b=2");
        assert!(req.headers.iter().any(|(n, v)| n == "Content-Type" && v == "application/x-www-form-urlencoded"));
    }

    #[test]
    fn empty_input_is_none_not_an_error() {
        let mut reader = BufReader::new(&b""[..]);
        assert!(parse_request(&mut reader).expect("ok").is_none());
    }

    #[test]
    fn malformed_request_line_is_an_error_not_a_panic() {
        let mut reader = BufReader::new(&b"not a request line at all\r\n\r\n"[..]);
        assert!(parse_request(&mut reader).is_err());
    }

    fn unique_temp_socket_path() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wire-test-{}-{n}.sock", std::process::id()))
    }

    /// A minimal loopback fixture that always answers with `response`
    /// verbatim (raw bytes — callers build whatever framing, including a
    /// deliberately chunked or gzip-compressed body, the specific thing
    /// each test below wants `wire` to normalize away).
    fn spawn_fixture(response: &'static [u8]) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture listener");
        let addr = listener.local_addr().expect("local_addr");
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut discard = [0u8; 1024];
                let _ = Read::read(&mut stream, &mut discard);
                let _ = stream.write_all(response);
            }
        });
        addr
    }

    fn get_request(addr: std::net::SocketAddr, path: &str) -> ParsedRequest {
        let url = Url::parse(&format!("http://{addr}{path}")).expect("fixture URL");
        ParsedRequest { method: "GET".to_string(), url, headers: Vec::new(), body: Vec::new() }
    }

    /// A parsed response, for tests only — `execute_and_stream` itself
    /// only ever writes bytes, it has no return value to assert on
    /// (that's the whole point: nothing it produces is ever fully
    /// buffered as one value first). Runs it against an in-memory
    /// `Vec<u8>` and parses the raw bytes back out, same as the real
    /// Unix-socket round trip (`full_unix_socket_round_trip_matches_
    /// execute_and_stream_directly`, below) already has to.
    struct TestResponse {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    fn execute_for_test(agent: &ureq::Agent, request: &ParsedRequest) -> TestResponse {
        let mut raw = Vec::new();
        execute_and_stream(agent, request, &mut raw).expect("execute_and_stream into a Vec<u8> can't fail");
        let header_end = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("header terminator");
        let header_text = std::str::from_utf8(&raw[..header_end]).expect("utf8 headers");
        let mut lines = header_text.split("\r\n");
        let status_line = lines.next().expect("status line");
        let status = status_line.split_whitespace().nth(1).expect("status code").parse().expect("status code parses");
        let headers = lines
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_string(), value.trim().to_string()))
            })
            .collect();
        let body = raw[header_end + 4..].to_vec();
        TestResponse { status, headers, body }
    }

    #[test]
    fn file_urls_are_served_from_disk_with_a_guessed_content_type() {
        let dir = std::env::temp_dir().join(format!("wire-file-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let css = dir.join("a.css");
        std::fs::write(&css, "p{color:red}").unwrap();
        let bin = dir.join("x.weird");
        std::fs::write(&bin, [1u8, 2, 3]).unwrap();
        let agent = build_agent();
        let get = |p: &std::path::Path| {
            let url = Url::from_file_path(p).unwrap();
            execute_for_test(&agent, &ParsedRequest { method: "GET".into(), url, headers: vec![], body: vec![] })
        };
        let r = get(&css);
        assert_eq!(r.status, 200);
        assert!(r.headers.iter().any(|(n, v)| n == "Content-Type" && v == "text/css"));
        assert_eq!(r.body, b"p{color:red}");
        let r = get(&bin);
        assert!(r.headers.iter().any(|(n, v)| n == "Content-Type" && v == "application/octet-stream"));
        assert_eq!(get(&dir.join("missing.html")).status, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ordinary_response_streams_through_with_no_content_length() {
        let body = "hello from the fixture";
        let addr = spawn_fixture(
            format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                .into_bytes()
                .leak(),
        );
        let agent = build_agent();
        let resp = execute_for_test(&agent, &get_request(addr, "/"));
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, body.as_bytes());
        // No Content-Length — the body streamed through as it arrived,
        // so there was never a complete size to state up front. See
        // `stream_response`'s own docs for why that's deliberate.
        assert!(
            !resp.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-length")),
            "a streamed response must never carry a Content-Length, got: {:?}",
            resp.headers
        );
    }

    #[test]
    fn chunked_upstream_response_normalizes_to_a_plain_streamed_body() {
        // Two chunks ("hello" + " world"), terminated by the 0-length
        // chunk — real `Transfer-Encoding: chunked` framing, exactly the
        // shape this daemon exists to never make scratchy decode itself.
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let addr = spawn_fixture(raw);
        let agent = build_agent();
        let resp = execute_for_test(&agent, &get_request(addr, "/"));
        assert_eq!(resp.body, b"hello world");
        assert!(
            !resp.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("transfer-encoding")),
            "Transfer-Encoding must not leak through a normalized response, got: {:?}",
            resp.headers
        );
    }

    #[test]
    fn gzip_upstream_response_is_decompressed_before_forwarding() {
        // A real gzip member wrapping the ASCII bytes "hello gzip" —
        // built once, by hand, rather than depending on a compression
        // crate from the test itself, so this test proves `wire`'s own
        // `ureq` `gzip` feature does the decoding, not some circular
        // test-side shortcut.
        let gz: &[u8] = &[
            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0x48,
            0xaf, 0xca, 0x2c, 0x00, 0x00, 0x19, 0x6a, 0xd2, 0xdf, 0x0a, 0x00, 0x00, 0x00,
        ];
        let mut raw = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            gz.len()
        )
        .into_bytes();
        raw.extend_from_slice(gz);
        let addr = spawn_fixture(raw.leak());
        let agent = build_agent();
        let resp = execute_for_test(&agent, &get_request(addr, "/"));
        assert_eq!(resp.body, b"hello gzip");
        assert!(
            !resp.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-encoding")),
            "Content-Encoding must not leak through once the body is already decompressed, got: {:?}",
            resp.headers
        );
    }

    #[test]
    fn a_real_4xx_from_upstream_forwards_as_itself_not_a_transport_error() {
        let addr = spawn_fixture(b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found");
        let agent = build_agent();
        let resp = execute_for_test(&agent, &get_request(addr, "/missing"));
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body, b"not found");
    }

    #[test]
    fn a_connection_failure_synthesizes_a_502_not_a_hang_or_a_crash() {
        // Nothing is listening on this address — ureq's own connect
        // attempt fails fast with a transport error.
        let agent = build_agent();
        let req = get_request("127.0.0.1:1".parse().expect("addr"), "/");
        let resp = execute_for_test(&agent, &req);
        assert_eq!(resp.status, 502);
        assert!(!resp.body.is_empty());
    }

    #[test]
    fn full_unix_socket_round_trip_matches_execute_and_stream_directly() {
        let body = "hello from the fixture";
        let addr = spawn_fixture(
            format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                .into_bytes()
                .leak(),
        );

        let socket_path = unique_temp_socket_path();
        let listener = UnixListener::bind(&socket_path).expect("bind unix socket");
        let server_agent = build_agent();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            handle_connection(stream, &server_agent, &Shared::new(true));
        });

        let mut client = UnixStream::connect(&socket_path).expect("connect unix socket");
        write!(client, "GET http://{addr}/ HTTP/1.1\r\nHost: {addr}\r\n\r\n").expect("write request");
        let mut response = Vec::new();
        client.read_to_end(&mut response).expect("read response");
        server.join().expect("server thread");
        let _ = std::fs::remove_file(&socket_path);

        let response = String::from_utf8(response).expect("utf8 response");
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "got: {response}");
        assert!(response.ends_with(body), "got: {response}");
    }

    // ---------- QF6: caps, class, policy, final URL, duplicate headers ----------

    /// A fixture whose answer is built from the request text (so a
    /// redirect can name its own address); every request text is also
    /// sent down the returned channel.
    fn spawn_dynamic(
        make: impl Fn(&str, std::net::SocketAddr) -> Vec<u8> + Send + 'static,
    ) -> (std::net::SocketAddr, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let n = Read::read(&mut stream, &mut buf).unwrap_or(0);
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = stream.write_all(&make(&text, addr));
                let _ = tx.send(text);
            }
        });
        (addr, rx)
    }

    fn ok_text(body: &str) -> Vec<u8> {
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes()
    }

    fn with_headers(mut req: ParsedRequest, h: &[(&str, &str)]) -> ParsedRequest {
        req.headers.extend(h.iter().map(|(n, v)| (n.to_string(), v.to_string())));
        req
    }

    fn hdr<'a>(r: &'a TestResponse, name: &str) -> Option<&'a str> {
        r.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    #[test]
    fn every_response_carries_the_wire_version() {
        let addr = spawn_fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        let agent = build_agent();
        assert_eq!(hdr(&execute_for_test(&agent, &get_request(addr, "/")), "X-Wire-Version"), Some("3"));
        let down = get_request("127.0.0.1:1".parse().unwrap(), "/");
        assert_eq!(hdr(&execute_for_test(&agent, &down), "X-Wire-Version"), Some("3"));
    }

    #[test]
    fn loopback_target_is_classified_and_final_url_reported() {
        let addr = spawn_fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        let agent = build_agent();
        let r = execute_for_test(&agent, &get_request(addr, "/x"));
        assert_eq!(r.status, 200);
        assert_eq!(hdr(&r, "X-Wire-Target-Class"), Some("loopback"));
        assert_eq!(hdr(&r, "X-Wire-Hop-Classes"), Some("loopback"));
        assert_eq!(hdr(&r, "X-Wire-Final-Url"), Some(format!("http://{addr}/x").as_str()));
    }

    #[test]
    fn ip_class_names_match_scratchy() {
        let c = |s: &str| ip_class(s.parse().unwrap());
        assert_eq!(c("127.0.0.1"), "loopback");
        assert_eq!(c("::1"), "loopback");
        assert_eq!(c("10.1.2.3"), "private");
        assert_eq!(c("192.168.0.1"), "private");
        assert_eq!(c("169.254.1.1"), "link-local");
        assert_eq!(c("100.64.0.1"), "cgnat");
        assert_eq!(c("fd00::1"), "ula");
        assert_eq!(c("fe80::1"), "link-local");
        assert_eq!(c("224.0.0.1"), "multicast");
        assert_eq!(c("8.8.8.8"), "public");
        assert_eq!(c("::ffff:127.0.0.1"), "loopback");
    }

    #[test]
    fn redirect_reports_final_url_and_each_hop_class() {
        let (addr, _rx) = spawn_dynamic(|req, addr| {
            if req.starts_with("GET /start") {
                format!("HTTP/1.1 302 Found\r\nLocation: http://{addr}/end\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
            } else {
                ok_text("done")
            }
        });
        let agent = build_agent();
        let r = execute_for_test(&agent, &get_request(addr, "/start"));
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"done");
        assert_eq!(hdr(&r, "X-Wire-Final-Url"), Some(format!("http://{addr}/end").as_str()));
        assert_eq!(hdr(&r, "X-Wire-Hop-Classes"), Some("loopback,loopback"));
    }

    #[test]
    fn max_redirects_header_limits_the_chain_and_is_not_forwarded() {
        let (addr, rx) = spawn_dynamic(|req, addr| {
            if req.starts_with("GET /a") {
                format!("HTTP/1.1 302 Found\r\nLocation: http://{addr}/b\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
            } else if req.starts_with("GET /b") {
                format!("HTTP/1.1 302 Found\r\nLocation: http://{addr}/c\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
            } else {
                ok_text("c")
            }
        });
        let agent = build_agent();
        // Limit 1: /a -> /b is followed, /b -> /c is not: ureq fails the
        // request, which wire reports as the usual marked 502.
        let r = execute_for_test(&agent, &with_headers(get_request(addr, "/a"), &[("X-Wire-Max-Redirects", "1")]));
        assert_eq!(r.status, 502);
        assert_eq!(hdr(&r, "X-Wire-Error"), Some("1"));
        assert!(String::from_utf8_lossy(&r.body).to_lowercase().contains("redirect"), "{:?}", r.body);
        // Limit 5 follows all the way.
        let r = execute_for_test(&agent, &with_headers(get_request(addr, "/a"), &[("X-Wire-Max-Redirects", "5")]));
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"c");
        while let Ok(text) = rx.try_recv() {
            assert!(!text.to_ascii_lowercase().contains("x-wire-"), "control header leaked upstream: {text}");
        }
    }

    #[test]
    fn control_headers_are_never_forwarded_upstream() {
        let (addr, rx) = spawn_dynamic(|_, _| ok_text("hi"));
        let agent = build_agent();
        let req = with_headers(
            get_request(addr, "/"),
            &[
                ("X-Wire-Max-Bytes", "100"),
                ("X-Wire-Timeout-Ms", "5000"),
                ("X-Wire-Max-Redirects", "3"),
                ("X-Wire-Policy", "none"),
                ("X-Custom", "kept"),
            ],
        );
        assert_eq!(execute_for_test(&agent, &req).status, 200);
        let text = rx.recv().unwrap().to_ascii_lowercase();
        assert!(!text.contains("x-wire-"), "leaked: {text}");
        assert!(text.contains("x-custom: kept"), "ordinary headers still forwarded: {text}");
    }

    #[test]
    fn max_bytes_over_the_cap_is_502_too_large_and_under_is_unchanged() {
        let body = "0123456789".repeat(10); // 100 bytes
        let b2 = body.clone();
        let (addr, _rx) = spawn_dynamic(move |_, _| ok_text(&b2));
        let agent = build_agent();
        let r = execute_for_test(&agent, &with_headers(get_request(addr, "/"), &[("X-Wire-Max-Bytes", "99")]));
        assert_eq!(r.status, 502);
        assert_eq!(hdr(&r, "X-Wire-Error"), Some("too-large"));
        assert!(String::from_utf8_lossy(&r.body).contains("too large"));
        // Exactly at the cap is fine.
        let r = execute_for_test(&agent, &with_headers(get_request(addr, "/"), &[("X-Wire-Max-Bytes", "100")]));
        assert_eq!(r.status, 200);
        assert_eq!(r.body, body.as_bytes());
        assert_eq!(hdr(&r, "X-Wire-Error"), None);
        // No header: no cap at all.
        assert_eq!(execute_for_test(&agent, &get_request(addr, "/")).body, body.as_bytes());
    }

    #[test]
    fn timeout_header_overrides_the_default_timeout() {
        // Accepts, never answers.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let mut held = Vec::new();
            for s in listener.incoming().flatten() {
                held.push(s);
            }
        });
        let agent = build_agent();
        let start = Instant::now();
        let r = execute_for_test(&agent, &with_headers(get_request(addr, "/"), &[("X-Wire-Timeout-Ms", "300")]));
        assert!(start.elapsed() < Duration::from_secs(5), "took {:?}", start.elapsed());
        assert_eq!(r.status, 502);
        assert_eq!(hdr(&r, "X-Wire-Error"), Some("1"));
    }

    #[test]
    fn duplicate_response_headers_are_written_as_separate_lines() {
        let addr = spawn_fixture(b"HTTP/1.1 200 OK\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2; Path=/\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        let agent = build_agent();
        let r = execute_for_test(&agent, &get_request(addr, "/"));
        let cookies: Vec<&str> = r.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.as_str()).collect();
        assert_eq!(cookies, vec!["a=1", "b=2; Path=/"]);
    }

    #[test]
    fn public_only_refuses_a_loopback_target_with_403_policy() {
        let (addr, rx) = spawn_dynamic(|_, _| ok_text("secret"));
        let agent = build_agent();
        let r = execute_for_test(&agent, &with_headers(get_request(addr, "/"), &[("X-Wire-Policy", "public-only")]));
        assert_eq!(r.status, 403);
        assert_eq!(hdr(&r, "X-Wire-Error"), Some("policy"));
        assert!(String::from_utf8_lossy(&r.body).contains("blocked"));
        assert!(rx.try_recv().is_err(), "the upstream must never have been contacted");
        // The same request without the header still works.
        let r = execute_for_test(&agent, &get_request(addr, "/"));
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"secret");
        // A later request on the same thread is not affected by the earlier policy.
        let r = execute_for_test(&agent, &with_headers(get_request(addr, "/"), &[("X-Wire-Policy", "something-else")]));
        assert_eq!(r.status, 200);
    }

    #[test]
    fn public_only_resolver_refuses_every_non_public_hop() {
        // Public-only can never reach a loopback fixture, so the per-hop
        // check (ureq calls the resolver again for every redirect hop) is
        // exercised through the resolver directly.
        TRACKER.with(|t| *t.borrow_mut() = Tracker { public_only: true, ..Tracker::default() });
        assert!(ureq::Resolver::resolve(&TrackingResolver, "127.0.0.1:80").is_err());
        assert!(ureq::Resolver::resolve(&TrackingResolver, "10.0.0.5:443").is_err());
        let ok = ureq::Resolver::resolve(&TrackingResolver, "8.8.8.8:443").expect("public passes");
        assert_eq!(ok.len(), 1);
        TRACKER.with(|t| {
            let t = t.borrow();
            assert_eq!(t.hops.iter().map(|h| h.1).collect::<Vec<_>>(), vec!["loopback", "private", "public"]);
            assert!(t.blocked.as_deref().unwrap().contains("blocked"));
        });
        TRACKER.with(|t| *t.borrow_mut() = Tracker::default());
    }

    #[test]
    fn file_urls_with_public_only_answer_403() {
        let dir = std::env::temp_dir().join(format!("wire-file-policy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.css");
        std::fs::write(&f, "x").unwrap();
        let agent = build_agent();
        let mk = |h: &[(&str, &str)]| {
            with_headers(ParsedRequest { method: "GET".into(), url: Url::from_file_path(&f).unwrap(), headers: vec![], body: vec![] }, h)
        };
        let r = execute_for_test(&agent, &mk(&[("X-Wire-Policy", "public-only")]));
        assert_eq!(r.status, 403);
        assert_eq!(hdr(&r, "X-Wire-Error"), Some("policy"));
        let r = execute_for_test(&agent, &mk(&[]));
        assert_eq!(r.status, 200);
        assert_eq!(hdr(&r, "X-Wire-Target-Class"), Some("file"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transport_failure_keeps_the_old_text_and_adds_the_error_marker() {
        let agent = build_agent();
        let r = execute_for_test(&agent, &get_request("127.0.0.1:1".parse().unwrap(), "/"));
        assert_eq!(r.status, 502);
        assert_eq!(hdr(&r, "X-Wire-Error"), Some("1"));
        assert!(String::from_utf8_lossy(&r.body).starts_with("wire: upstream request failed:"));
    }

    #[test]
    fn upstream_cannot_spoof_control_headers() {
        let addr = spawn_fixture(b"HTTP/1.1 200 OK\r\nX-Wire-Target-Class: public\r\nX-Wire-Error: policy\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        let agent = build_agent();
        let r = execute_for_test(&agent, &get_request(addr, "/"));
        assert_eq!(hdr(&r, "X-Wire-Target-Class"), Some("loopback"));
        assert_eq!(hdr(&r, "X-Wire-Error"), None);
    }

    // ---------- QF14: the cookie jar through the request path ----------

    fn cookie_header_of(req_text: &str) -> Option<String> {
        req_text.lines().find_map(|l| l.strip_prefix("Cookie: ").or_else(|| l.strip_prefix("cookie: "))).map(|v| v.trim().to_string())
    }

    /// `/set` answers two Set-Cookie lines (one HttpOnly); `/r1` -> `/r2` ->
    /// `/r3` is a redirect chain that sets a cookie on each of the first two
    /// hops; anything else echoes the request's Cookie header as the body.
    fn cookie_server() -> (std::net::SocketAddr, std::sync::mpsc::Receiver<String>) {
        spawn_dynamic(|req, addr| {
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            let redirect = |to: &str, cookie: &str| {
                format!("HTTP/1.1 302 Found\r\nLocation: http://{addr}{to}\r\nSet-Cookie: {cookie}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .into_bytes()
            };
            match path.as_str() {
                "/set" => b"HTTP/1.1 200 OK\r\nSet-Cookie: sid=abc123; HttpOnly\r\nSet-Cookie: pub=visible\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                    .to_vec(),
                "/r1" => redirect("/r2", "a=1"),
                "/r2" => redirect("/r3", "b=2"),
                "/bad" => b"HTTP/1.1 200 OK\r\nSet-Cookie: novalue\r\nSet-Cookie: =x\r\nSet-Cookie: ok=1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                _ => ok_text(&cookie_header_of(req).unwrap_or_else(|| "<none>".to_string())),
            }
        })
    }

    const JAR: (&str, &str) = ("X-Wire-Jar", "scratchy");

    fn run(shared: &Shared, req: ParsedRequest) -> TestResponse {
        let mut raw = Vec::new();
        execute_with(&build_agent(), shared, &req, &mut raw).expect("write into a Vec cannot fail");
        let header_end = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("header terminator");
        let text = std::str::from_utf8(&raw[..header_end]).unwrap();
        let mut lines = text.split("\r\n");
        let status = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
        let headers = lines
            .filter_map(|l| l.split_once(':').map(|(n, v)| (n.trim().to_string(), v.trim().to_string())))
            .collect();
        TestResponse { status, headers, body: raw[header_end + 4..].to_vec() }
    }

    fn control(shared: &Shared, op: &str, page: &str, top: Option<&str>, body: &str) -> TestResponse {
        let mut h = vec![("X-Wire-Cookie-Op", op), ("X-Wire-Cookie-Url", page), JAR, ("X-Wire-Control-Token", shared.control_token.as_str())];
        if let Some(t) = top {
            h.push(("X-Wire-Top-Site", t));
        }
        let mut req = with_headers(
            ParsedRequest { method: "POST".into(), url: Url::parse("http://wire.invalid/cookies").unwrap(), headers: vec![], body: body.as_bytes().to_vec() },
            &h,
        );
        req.method = if op == "get" { "GET".into() } else { "POST".into() };
        run(shared, req)
    }

    // ---------- control token ----------

    fn control_with(shared: &Shared, op: &str, token: Option<&str>) -> TestResponse {
        let mut h = vec![("X-Wire-Cookie-Op", op), ("X-Wire-Cookie-Url", "https://a.example/"), JAR, ("X-Wire-Top-Site", "https://a.example")];
        if let Some(t) = token {
            h.push(("X-Wire-Control-Token", t));
        }
        let method = if op == "get" { "GET" } else { "POST" };
        run(shared, with_headers(ParsedRequest { method: method.into(), url: Url::parse("http://wire.invalid/c").unwrap(), headers: vec![], body: b"k=v; Max-Age=60".to_vec() }, &h))
    }

    #[test]
    fn every_cookie_control_op_needs_the_token() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        assert_eq!(shared.control_token.len(), 64);
        assert!(shared.control_token.bytes().all(|b| b.is_ascii_hexdigit()));
        let good = shared.control_token.clone();
        let mut wrong = good.clone();
        wrong.replace_range(0..1, if good.starts_with('0') { "1" } else { "0" });
        for op in ["get", "set", "clear-site", "frob"] {
            for token in [None, Some(""), Some(wrong.as_str()), Some(&good[..63])] {
                let r = control_with(&shared, op, token);
                assert_eq!(r.status, 403, "{op} with {token:?}");
                assert_eq!(hdr(&r, "X-Wire-Error"), Some("cookie-control"));
            }
        }
        assert_eq!(shared.jar().len(), 0, "a refused set stored nothing");
        assert_eq!(control_with(&shared, "set", Some(&good)).status, 204);
        assert_eq!(control_with(&shared, "get", Some(&good)).body, b"k=v");
        assert_eq!(control_with(&shared, "clear-site", Some(&good)).status, 204);
        assert_eq!(shared.jar().len(), 0);
        // A control request aimed at a real upstream is refused, not forwarded.
        let req = with_headers(get_request(addr, "/echo"), &[JAR, ("X-Wire-Cookie-Op", "get"), ("X-Wire-Cookie-Url", "https://a.example/")]);
        assert_eq!(run(&shared, req).status, 403);
        assert!(rx.try_recv().is_err(), "nothing was forwarded upstream");
        // The feature is advertised.
        assert!(hdr(&control_with(&shared, "get", Some(&good)), "X-Wire-Features").unwrap().split(',').any(|f| f == "token"));
    }

    #[test]
    fn page_requests_through_the_jar_do_not_need_the_token() {
        let (addr, _rx) = cookie_server();
        let shared = Shared::new(true);
        assert_eq!(run(&shared, with_headers(get_request(addr, "/set"), &[JAR])).status, 200);
        let r = run(&shared, with_headers(get_request(addr, "/echo"), &[JAR]));
        assert_eq!(r.body, b"sid=abc123; pub=visible");
    }

    #[test]
    fn token_file_is_mode_0600_next_to_the_socket_and_replaced_on_every_start() {
        use std::os::unix::fs::PermissionsExt;
        let dir = jar_dir("token");
        let sock = dir.join("w.sock");
        let file = token_file_path(&sock);
        assert_eq!(file, dir.join("w.sock.token"));
        let first = Shared::new(true);
        first.write_token_file(&sock).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), first.control_token);
        let second = Shared::new(true);
        assert_ne!(first.control_token, second.control_token);
        second.write_token_file(&sock).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), second.control_token);
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(control_with(&first, "get", Some(&second.control_token)).status, 403, "the old wire's token does not work on the new one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- W-PERSIST1: --cookie-file and clear-site ----------

    fn jar_dir(name: &str) -> PathBuf {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("wire-jarfile-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn get_cookies(shared: &Shared, page: &str, top: &str) -> String {
        String::from_utf8(control(shared, "get", page, Some(top), "").body).unwrap()
    }

    #[test]
    fn cookie_file_round_trips_persistent_cookies_only_with_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = jar_dir("roundtrip");
        let file = dir.join("cookies");
        let shared = Shared::new(true).with_cookie_file(file.clone());
        assert!(!file.exists(), "a fresh jar writes nothing until it changes");
        let page = "https://www.example.com/a/";
        assert_eq!(control(&shared, "set", page, None, "keep=1%20two\ttab; Max-Age=3600; Path=/").status, 204);
        assert_eq!(control(&shared, "set", page, None, "sess=1; Path=/").status, 204);
        assert_eq!(get_cookies(&shared, page, "https://example.com"), "keep=1%20two\ttab; sess=1");
        shared.save_jar(false);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("# wire cookies v1\n"), "{text}");
        assert_eq!(text.lines().count(), 2, "header + the one persistent cookie: {text}");
        assert!(!text.contains("sess"), "session cookies are never saved");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| e.unwrap().file_name() == "cookies"), "no temp file left behind");
        // A second wire reads it back; the session cookie is gone.
        let again = Shared::new(true).with_cookie_file(file.clone());
        assert_eq!(get_cookies(&again, page, "https://example.com"), "keep=1%20two\ttab");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cookie_file_text_escapes_separators_and_drops_expired_on_load() {
        let now = now_unix();
        let mut jar = cookies::Jar::default();
        let ctx = cookies::RequestCtx::new(&Url::parse("https://example.com/").unwrap(), "GET", None, false, None, false);
        assert!(jar.store(&ctx, "weird=a%b; Max-Age=100; Path=/", now));
        let text = jar.to_file_text(now);
        assert_eq!(cookies::Jar::from_file_text(&text, now).unwrap().to_file_text(now), text);
        // Loaded after its expiry: dropped.
        let late = cookies::Jar::from_file_text(&text, now + 101).unwrap();
        assert_eq!(late.to_file_text(now + 101).lines().count(), 1);
        let hand = format!(
            "# wire cookies v1\nhttps://example.com\texample.com\t1\t/\told\tx\t0\t0\t0\t{}\t1\nhttps://example.com\texample.com\t1\t/\tnew\ty\t0\t0\t0\t{}\t2\n",
            now - 5,
            now + 500
        );
        let j = cookies::Jar::from_file_text(&hand, now).unwrap();
        let out = j.to_file_text(now);
        assert!(out.contains("\tnew\t") && !out.contains("\told\t"), "{out}");
    }

    #[test]
    fn corrupt_cookie_file_gives_an_empty_jar_and_is_replaced_on_the_next_save() {
        let dir = jar_dir("corrupt");
        let file = dir.join("cookies");
        for bad in ["not a cookie file\n", "# wire cookies v1\nonly\ttwo\n", "# wire cookies v1\na\tb\t9\t/\tn\tv\t0\t0\t0\t5\t1\n", ""] {
            assert!(cookies::Jar::from_file_text(bad, 0).is_err(), "{bad:?}");
            std::fs::write(&file, bad).unwrap();
            let shared = Shared::new(true).with_cookie_file(file.clone());
            assert_eq!(shared.jar().to_file_text(now_unix()).lines().count(), 1, "empty jar after {bad:?}");
        }
        std::fs::write(&file, [0xff, 0xfe, b'\n']).unwrap();
        let shared = Shared::new(true).with_cookie_file(file.clone());
        assert_eq!(control(&shared, "set", "https://example.com/", None, "a=1; Max-Age=60").status, 204);
        shared.save_jar(false);
        assert!(std::fs::read_to_string(&file).unwrap().starts_with("# wire cookies v1\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_site_removes_first_party_and_partitioned_cookies_and_persists() {
        let dir = jar_dir("clear");
        let file = dir.join("cookies");
        let shared = Shared::new(true).with_cookie_file(file.clone());
        let (a, b) = ("https://a.example", "https://b.example");
        // a.example first party, a third party embedded under a.example, and the
        // same third party under b.example plus b.example itself.
        for (page, top) in [("https://www.a.example/", a), ("https://tracker.test/", a), ("https://tracker.test/", b), ("https://b.example/", b)] {
            assert_eq!(control(&shared, "set", page, Some(top), "k=v; Max-Age=3600; Path=/").status, 204);
        }
        shared.save_jar(false);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 5);
        let r = control(&shared, "clear-site", "", Some("https://www.a.example/page"), "");
        assert_eq!(r.status, 204);
        assert_eq!(get_cookies(&shared, "https://www.a.example/", a), "");
        assert_eq!(get_cookies(&shared, "https://tracker.test/", a), "");
        assert_eq!(get_cookies(&shared, "https://tracker.test/", b), "k=v");
        assert_eq!(get_cookies(&shared, "https://b.example/", b), "k=v");
        // persisted immediately, without waiting for the saver thread
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().count(), 3, "{text}");
        assert!(!text.contains("a.example"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_site_is_honoured_only_on_the_jar_control_path() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        let a = "https://a.example";
        assert_eq!(control(&shared, "set", "https://a.example/", Some(a), "k=v; Max-Age=3600").status, 204);
        // No X-Wire-Jar: refused, nothing cleared.
        let mut req = ParsedRequest { method: "POST".into(), url: Url::parse("http://wire.invalid/").unwrap(), headers: vec![], body: vec![] };
        req = with_headers(req, &[("X-Wire-Cookie-Op", "clear-site"), ("X-Wire-Top-Site", a), ("X-Wire-Control-Token", shared.control_token.as_str())]);
        assert_eq!(run(&shared, req).status, 400);
        // No usable top site: refused.
        assert_eq!(control(&shared, "clear-site", "", None, "").status, 400);
        assert_eq!(control(&shared, "clear-site", "", Some("not a url"), "").status, 400);
        assert_eq!(get_cookies(&shared, "https://a.example/", a), "k=v");
        // The op is answered by wire and never reaches the upstream, even when
        // the request URL points at a real server.
        let req = with_headers(get_request(addr, "/echo"), &[JAR, ("X-Wire-Cookie-Op", "clear-site"), ("X-Wire-Top-Site", "https://other.example"), ("X-Wire-Control-Token", shared.control_token.as_str())]);
        assert_eq!(run(&shared, req).status, 204);
        assert!(rx.try_recv().is_err(), "nothing was forwarded upstream");
        assert_eq!(get_cookies(&shared, "https://a.example/", a), "k=v", "only the named partition is cleared");
        // Cookies disabled: 404, the op does nothing.
        assert_eq!(control(&Shared::new(false), "clear-site", "", Some(a), "").status, 404);
        // The advertised feature.
        let r = control(&shared, "clear-site", "", Some(a), "");
        assert!(hdr(&r, "X-Wire-Features").unwrap().split(',').any(|f| f == "jarfile"));
        assert_eq!(get_cookies(&shared, "https://a.example/", a), "");
    }

    #[test]
    fn jar_stores_on_one_request_and_sends_on_the_next() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        let r1 = run(&shared, with_headers(get_request(addr, "/set"), &[JAR]));
        assert_eq!(r1.status, 200);
        assert_eq!(hdr(&r1, "X-Wire-Cookies"), Some("on"));
        assert_eq!(hdr(&r1, "X-Wire-Version"), Some("3"));
        assert_eq!(hdr(&r1, "Set-Cookie"), None, "Set-Cookie is stripped when the jar was used");
        assert_eq!(hdr(&r1, "X-Wire-Jar"), None);
        assert_eq!(cookie_header_of(&rx.recv().unwrap()), None);
        let r2 = run(&shared, with_headers(get_request(addr, "/echo"), &[JAR]));
        assert_eq!(r2.body, b"sid=abc123; pub=visible");
        assert_eq!(cookie_header_of(&rx.recv().unwrap()).as_deref(), Some("sid=abc123; pub=visible"));
        // the jar header itself was never forwarded
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn no_jar_header_or_cookies_off_sends_and_stores_nothing() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        // no jar header: Set-Cookie passes through untouched, nothing stored
        let r = run(&shared, get_request(addr, "/set"));
        assert_eq!(hdr(&r, "X-Wire-Cookies"), None);
        assert_eq!(r.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("set-cookie")).count(), 2);
        let _ = rx.recv().unwrap();
        // a request that does use the jar sees an empty jar
        let r = run(&shared, with_headers(get_request(addr, "/echo"), &[JAR]));
        assert_eq!(r.body, b"<none>");
        let _ = rx.recv().unwrap();
        // store something, then opt out with X-Wire-Cookies: off
        run(&shared, with_headers(get_request(addr, "/set"), &[JAR]));
        let _ = rx.recv().unwrap();
        let r = run(&shared, with_headers(get_request(addr, "/echo"), &[JAR, ("X-Wire-Cookies", "off")]));
        assert_eq!(r.body, b"<none>");
        assert_eq!(hdr(&r, "X-Wire-Cookies"), None);
        let _ = rx.recv().unwrap();
        // and a client Cookie header is forwarded as before when the jar is not used
        let r = run(&shared, with_headers(get_request(addr, "/echo"), &[("Cookie", "mine=1")]));
        assert_eq!(r.body, b"mine=1");
    }

    #[test]
    fn a_client_cookie_header_is_dropped_when_the_jar_is_used() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        let r = run(&shared, with_headers(get_request(addr, "/echo"), &[JAR, ("Cookie", "evil=1")]));
        assert_eq!(r.body, b"<none>");
        assert!(!rx.recv().unwrap().to_ascii_lowercase().contains("evil"));
        run(&shared, with_headers(get_request(addr, "/set"), &[JAR]));
        let _ = rx.recv().unwrap();
        let r = run(&shared, with_headers(get_request(addr, "/echo"), &[JAR, ("cookie", "evil=1")]));
        assert_eq!(r.body, b"sid=abc123; pub=visible");
    }

    #[test]
    fn no_cookies_flag_disables_the_jar_and_the_control_channel() {
        let (addr, _rx) = cookie_server();
        let shared = Shared::new(false);
        let r = run(&shared, with_headers(get_request(addr, "/set"), &[JAR]));
        assert_eq!(hdr(&r, "X-Wire-Cookies"), Some("off"));
        assert_eq!(hdr(&r, "X-Wire-Version"), Some("3"));
        assert_eq!(r.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("set-cookie")).count(), 2, "Set-Cookie passes through untouched");
        let r = run(&shared, with_headers(get_request(addr, "/echo"), &[JAR]));
        assert_eq!(r.body, b"<none>");
        let r = control(&shared, "get", &format!("http://{addr}/"), None, "");
        assert_eq!(r.status, 404);
        assert_eq!(hdr(&r, "X-Wire-Cookies"), Some("off"));
        // errors and file: answers carry the marker too
        let r = run(&shared, with_headers(get_request("127.0.0.1:1".parse().unwrap(), "/"), &[JAR]));
        assert_eq!(hdr(&r, "X-Wire-Cookies"), Some("off"));
    }

    #[test]
    fn httponly_is_hidden_from_the_control_channel_but_sent_in_requests() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        run(&shared, with_headers(get_request(addr, "/set"), &[JAR]));
        let _ = rx.recv().unwrap();
        let page = format!("http://{addr}/page");
        let r = control(&shared, "get", &page, None, "");
        assert_eq!(r.status, 200);
        assert_eq!(hdr(&r, "Content-Type"), Some("text/plain"));
        assert_eq!(hdr(&r, "X-Wire-Cookies"), Some("on"));
        assert_eq!(r.body, b"pub=visible");
        let r = run(&shared, with_headers(get_request(addr, "/echo"), &[JAR]));
        assert_eq!(r.body, b"sid=abc123; pub=visible");
        // the control channel made no upstream request
        let _ = rx.recv().unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn control_channel_set_then_get_and_requests_see_it() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        let page = format!("http://{addr}/page");
        let r = control(&shared, "set", &page, None, "js=1; Path=/");
        assert_eq!(r.status, 204);
        assert_eq!(hdr(&r, "X-Wire-Cookies"), Some("on"));
        // a script cannot create an HttpOnly cookie
        assert_eq!(control(&shared, "set", &page, None, "h=1; HttpOnly").status, 204);
        assert_eq!(control(&shared, "get", &page, None, "").body, b"js=1");
        let r = run(&shared, with_headers(get_request(addr, "/echo"), &[JAR]));
        assert_eq!(r.body, b"js=1");
        let _ = rx.recv().unwrap();
        // bad controls
        assert_eq!(control(&shared, "frob", &page, None, "").status, 400);
        let mut req = get_request(addr, "/");
        req.headers.push(("X-Wire-Cookie-Op".into(), "get".into()));
        req.headers.push(("X-Wire-Control-Token".into(), shared.control_token.clone()));
        let r = run(&shared, req);
        assert_eq!(r.status, 400, "the control channel needs the jar header");
        assert_eq!(control(&shared, "get", "not a url", None, "").status, 400);
    }

    #[test]
    fn partitioning_separates_the_same_host_under_two_top_sites() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        run(&shared, with_headers(get_request(addr, "/set"), &[JAR, ("X-Wire-Top-Site", "https://a.example")]));
        let _ = rx.recv().unwrap();
        let echo = |top: Option<&str>| {
            let mut h = vec![JAR];
            if let Some(t) = top {
                h.push(("X-Wire-Top-Site", t));
            }
            let r = run(&shared, with_headers(get_request(addr, "/echo"), &h));
            let _ = rx.recv().unwrap();
            String::from_utf8(r.body).unwrap()
        };
        assert_eq!(echo(Some("https://a.example")), "sid=abc123; pub=visible");
        assert_eq!(echo(Some("https://www.a.example/page")), "sid=abc123; pub=visible", "same site = same partition");
        assert_eq!(echo(Some("https://b.example")), "<none>");
        assert_eq!(echo(Some("http://a.example")), "<none>", "the scheme is part of the site");
        assert_eq!(echo(None), "<none>", "first-party partition is separate");
        assert_eq!(control(&shared, "get", &format!("http://{addr}/"), Some("https://b.example"), "").body, b"");
        assert_eq!(control(&shared, "get", &format!("http://{addr}/"), Some("https://a.example"), "").body, b"pub=visible");
    }

    #[test]
    fn samesite_is_enforced_with_top_sites_and_navigations() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        let top = ("X-Wire-Top-Site", "https://a.example");
        let page = format!("http://{addr}/page");
        for c in ["st=1; SameSite=Strict", "lx=1; SameSite=Lax", "nn=1; SameSite=None", "un=1"] {
            control(&shared, "set", &page, Some("https://a.example"), c);
        }
        let echo = |h: &[(&str, &str)]| {
            let mut all = vec![JAR, top];
            all.extend_from_slice(h);
            let r = run(&shared, with_headers(get_request(addr, "/echo"), &all));
            let _ = rx.recv().unwrap();
            String::from_utf8(r.body).unwrap()
        };
        // embedded under a.example: the request is cross-site with its top site
        assert_eq!(echo(&[]), "nn=1; un=1");
        // top-level navigation from another site: Lax is sent, Strict is not
        assert_eq!(echo(&[("X-Wire-Nav", "1"), ("X-Wire-Initiator-Site", "https://evil.example")]), "lx=1; nn=1; un=1");
        // a navigation that is same-site with its initiator gets everything
        let me = format!("http://{addr}/");
        assert_eq!(echo(&[("X-Wire-Nav", "1"), ("X-Wire-Initiator-Site", &me)]), "st=1; lx=1; nn=1; un=1");
    }

    #[test]
    fn redirects_store_and_resend_cookies_on_every_hop() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        let r = run(&shared, with_headers(get_request(addr, "/r1"), &[JAR]));
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"a=1; b=2", "the third hop carries both cookies");
        assert_eq!(hdr(&r, "X-Wire-Final-Url"), Some(format!("http://{addr}/r3").as_str()));
        assert_eq!(hdr(&r, "X-Wire-Hop-Classes"), Some("loopback,loopback,loopback"));
        assert_eq!(hdr(&r, "Set-Cookie"), None);
        let (h1, h2, h3) = (rx.recv().unwrap(), rx.recv().unwrap(), rx.recv().unwrap());
        assert_eq!(cookie_header_of(&h1), None);
        assert_eq!(cookie_header_of(&h2).as_deref(), Some("a=1"));
        assert_eq!(cookie_header_of(&h3).as_deref(), Some("a=1; b=2"));
        // the redirect limit still applies
        let r = run(&shared, with_headers(get_request(addr, "/r1"), &[JAR, ("X-Wire-Max-Redirects", "1")]));
        assert_eq!(r.status, 502);
        assert_eq!(hdr(&r, "X-Wire-Error"), Some("1"));
    }

    #[test]
    fn malformed_set_cookie_lines_are_ignored_through_the_wire() {
        let (addr, rx) = cookie_server();
        let shared = Shared::new(true);
        run(&shared, with_headers(get_request(addr, "/bad"), &[JAR]));
        let _ = rx.recv().unwrap();
        let r = run(&shared, with_headers(get_request(addr, "/echo"), &[JAR]));
        assert_eq!(r.body, b"ok=1");
    }

    #[test]
    fn cookie_values_never_appear_in_wires_log_lines() {
        // The only cookie log line wire can print carries a host and counts.
        let line = cookies::log_line("127.0.0.1", 2, 1);
        assert!(!line.contains("sid") && !line.contains("abc123") && !line.contains("visible"));
        assert!(line.contains("sent=2") && line.contains("stored=1"));
    }

    #[test]
    fn full_unix_socket_round_trip_with_the_jar() {
        let (addr, _rx) = cookie_server();
        let socket_path = unique_temp_socket_path();
        let listener = UnixListener::bind(&socket_path).unwrap();
        let shared = Shared::new(true);
        let agent = build_agent();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                handle_connection(stream, &agent, &shared);
            }
        });
        let ask = |path: &str| {
            let mut c = UnixStream::connect(&socket_path).unwrap();
            write!(c, "GET http://{addr}{path} HTTP/1.1\r\nX-Wire-Jar: scratchy\r\n\r\n").unwrap();
            let mut out = String::new();
            c.read_to_string(&mut out).unwrap();
            out
        };
        let first = ask("/set");
        assert!(first.contains("X-Wire-Cookies: on\r\n") && !first.contains("Set-Cookie"), "{first}");
        let second = ask("/echo");
        assert!(second.ends_with("sid=abc123; pub=visible"), "{second}");
        server.join().unwrap();
        let _ = std::fs::remove_file(&socket_path);
    }

    // ---------- W-POOL1: policy and class headers on pooled connections ----------

    /// Keep-alive fixture: serves many requests per connection (so ureq pools
    /// the socket). `/redir` answers 302 to `/end`; the counter counts accepted
    /// TCP connections.
    fn spawn_keepalive() -> (std::net::SocketAddr, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let conns = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c2 = conns.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = Read::read(&mut stream, &mut buf).unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        let text = String::from_utf8_lossy(&buf[..n]).to_string();
                        let resp = if text.starts_with("GET /redir") {
                            format!("HTTP/1.1 302 Found\r\nLocation: http://{addr}/end\r\nContent-Length: 0\r\n\r\n")
                        } else {
                            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_string()
                        };
                        if stream.write_all(resp.as_bytes()).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (addr, conns)
    }

    fn run_on(agent: &ureq::Agent, shared: &Shared, req: ParsedRequest) -> TestResponse {
        let mut raw = Vec::new();
        execute_with(agent, shared, &req, &mut raw).expect("write into a Vec cannot fail");
        let header_end = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("header terminator");
        let text = std::str::from_utf8(&raw[..header_end]).unwrap();
        let mut lines = text.split("\r\n");
        let status = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
        let headers = lines.filter_map(|l| l.split_once(':').map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))).collect();
        TestResponse { status, headers, body: raw[header_end + 4..].to_vec() }
    }

    const PO: &[(&str, &str)] = &[("X-Wire-Policy", "public-only")];

    #[test]
    fn public_only_is_refused_after_a_plain_request_pooled_the_connection() {
        let (addr, conns) = spawn_keepalive();
        let (agent, shared) = (build_agent(), Shared::new(true));
        let r = run_on(&agent, &shared, with_headers(get_request(addr, "/"), PO));
        assert_eq!((r.status, hdr(&r, "X-Wire-Error")), (403, Some("policy")), "cold pool");
        let r = run_on(&agent, &shared, get_request(addr, "/"));
        assert_eq!(r.status, 200);
        let r = run_on(&agent, &shared, with_headers(get_request(addr, "/"), PO));
        assert_eq!((r.status, hdr(&r, "X-Wire-Error")), (403, Some("policy")), "warm plain pool must not bypass the check");
        assert_eq!(conns.load(std::sync::atomic::Ordering::SeqCst), 1, "only the plain request ever connected");
    }

    #[test]
    fn pooled_plain_requests_still_report_the_target_class() {
        let (addr, conns) = spawn_keepalive();
        let (agent, shared) = (build_agent(), Shared::new(true));
        for _ in 0..3 {
            let r = run_on(&agent, &shared, get_request(addr, "/"));
            assert_eq!(hdr(&r, "X-Wire-Target-Class"), Some("loopback"));
            assert_eq!(hdr(&r, "X-Wire-Hop-Classes"), Some("loopback"));
        }
        assert_eq!(conns.load(std::sync::atomic::Ordering::SeqCst), 1, "the connection really was reused");
    }

    #[test]
    fn pooled_redirect_reports_a_class_per_hop() {
        let (addr, conns) = spawn_keepalive();
        let (agent, shared) = (build_agent(), Shared::new(true));
        let _ = run_on(&agent, &shared, get_request(addr, "/"));
        let r = run_on(&agent, &shared, get_request(addr, "/redir"));
        assert_eq!(r.status, 200);
        assert_eq!(hdr(&r, "X-Wire-Final-Url"), Some(format!("http://{addr}/end").as_str()));
        assert_eq!(hdr(&r, "X-Wire-Target-Class"), Some("loopback"));
        assert_eq!(conns.load(std::sync::atomic::Ordering::SeqCst), 1);
        // public-only follows the same rule through a redirect: refused at the first hop.
        let r = run_on(&agent, &shared, with_headers(get_request(addr, "/redir"), PO));
        assert_eq!((r.status, hdr(&r, "X-Wire-Error")), (403, Some("policy")));
    }

    #[test]
    fn jar_path_checks_policy_and_reports_classes_on_pooled_connections() {
        let (addr, conns) = spawn_keepalive();
        let (agent, shared) = (build_agent(), Shared::new(true));
        let jar = [("X-Wire-Jar", "scratchy")];
        for _ in 0..2 {
            let r = run_on(&agent, &shared, with_headers(get_request(addr, "/"), &jar));
            assert_eq!(r.status, 200);
            assert_eq!(hdr(&r, "X-Wire-Target-Class"), Some("loopback"));
        }
        let r = run_on(&agent, &shared, with_headers(get_request(addr, "/redir"), &jar));
        assert_eq!(hdr(&r, "X-Wire-Hop-Classes"), Some("loopback,loopback"), "exact per hop on the jar path");
        assert_eq!(conns.load(std::sync::atomic::Ordering::SeqCst), 1, "the manual agent pooled");
        let both = [("X-Wire-Jar", "scratchy"), ("X-Wire-Policy", "public-only")];
        let r = run_on(&agent, &shared, with_headers(get_request(addr, "/"), &both));
        assert_eq!((r.status, hdr(&r, "X-Wire-Error")), (403, Some("policy")));
    }

    #[test]
    fn class_map_is_bounded_and_expires_by_lookup() {
        for i in 0..(CLASS_MAP_MAX + 50) {
            class_map_put(&format!("bound-test-{i}.invalid:1"), "public");
        }
        assert!(class_map().len() <= CLASS_MAP_MAX);
        class_map_put("lookup-test.invalid:1", "private");
        assert_eq!(class_map_get("lookup-test.invalid:1"), Some("private"));
        assert_eq!(class_map_get("never-seen.invalid:1"), None);
    }

    // ---- QY39a: per-request User-Agent override ----

    const DEFAULT_UA: &str = "wire/0.1 (+https://github.com/Loupax/vibe-it) scratchy's real-networking daemon";

    /// The User-Agent line a fixture saw (`None` when it got none).
    fn ua_of(req_text: &str) -> Option<String> {
        req_text.lines().find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.eq_ignore_ascii_case("user-agent").then(|| v.trim().to_string())
        })
    }

    /// Fixture that redirects /r -> /end and answers everything else 200.
    fn ua_server() -> (std::net::SocketAddr, std::sync::mpsc::Receiver<String>) {
        spawn_dynamic(|text, addr| {
            if text.starts_with("GET /r ") {
                format!("HTTP/1.1 302 Found\r\nLocation: http://{addr}/end\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
            } else {
                ok_text("ok")
            }
        })
    }

    #[test]
    fn user_agent_default_when_nothing_is_sent() {
        let (addr, rx) = ua_server();
        let r = execute_for_test(&build_agent(), &get_request(addr, "/"));
        assert_eq!(r.status, 200);
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some(DEFAULT_UA));
    }

    #[test]
    fn requests_own_user_agent_is_forwarded() {
        let (addr, rx) = ua_server();
        execute_for_test(&build_agent(), &with_headers(get_request(addr, "/"), &[("User-Agent", "Mozilla/5.0 scratchy")]));
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some("Mozilla/5.0 scratchy"));
    }

    #[test]
    fn x_wire_user_agent_wins_and_no_x_wire_header_reaches_upstream() {
        let (addr, rx) = ua_server();
        let r = execute_for_test(
            &build_agent(),
            &with_headers(get_request(addr, "/"), &[("User-Agent", "plain"), ("X-Wire-User-Agent", "Override/9 (X11)")]),
        );
        assert_eq!(r.status, 200);
        let text = rx.recv().unwrap();
        assert_eq!(ua_of(&text).as_deref(), Some("Override/9 (X11)"));
        assert_eq!(text.to_ascii_lowercase().matches("user-agent").count(), 1, "exactly one UA line: {text}");
        assert!(!text.to_ascii_lowercase().contains("x-wire-"), "leaked: {text}");
        // alone, without a plain User-Agent
        execute_for_test(&build_agent(), &with_headers(get_request(addr, "/"), &[("x-wire-user-agent", "Only/1")]));
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some("Only/1"));
    }

    #[test]
    fn invalid_user_agents_are_400_and_never_sent() {
        let (addr, rx) = ua_server();
        let long = "a".repeat(MAX_UA_LEN + 1);
        let ok_max = "a".repeat(MAX_UA_LEN);
        for bad in ["evil\r\nX-Injected: 1", "evil\nx", "tab\there", "caf\u{e9}", long.as_str()] {
            for name in ["X-Wire-User-Agent", "User-Agent"] {
                let r = execute_for_test(&build_agent(), &with_headers(get_request(addr, "/"), &[(name, bad)]));
                assert_eq!((r.status, hdr(&r, "X-Wire-Error")), (400, Some("bad-user-agent")), "{name}: {bad:?}");
                assert_eq!(hdr(&r, "X-Wire-Version"), Some("3"));
            }
        }
        assert!(rx.try_recv().is_err(), "an invalid UA must never reach upstream");
        let r = execute_for_test(&build_agent(), &with_headers(get_request(addr, "/"), &[("X-Wire-User-Agent", &ok_max)]));
        assert_eq!(r.status, 200);
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some(ok_max.as_str()));
    }

    #[test]
    fn user_agent_is_kept_on_every_redirect_hop() {
        let (addr, rx) = ua_server();
        let r = execute_for_test(&build_agent(), &with_headers(get_request(addr, "/r"), &[("X-Wire-User-Agent", "Hop/1")]));
        assert_eq!(hdr(&r, "X-Wire-Final-Url"), Some(format!("http://{addr}/end").as_str()));
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some("Hop/1"));
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some("Hop/1"));
        // a non-default redirect limit uses its own agent: still honoured
        execute_for_test(&build_agent(), &with_headers(get_request(addr, "/r"), &[("User-Agent", "Own/2"), ("X-Wire-Max-Redirects", "2")]));
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some("Own/2"));
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some("Own/2"));
    }

    #[test]
    fn jar_path_honours_the_user_agent_on_every_hop() {
        let (addr, rx) = ua_server();
        let shared = Shared::new(true);
        run(&shared, with_headers(get_request(addr, "/r"), &[JAR, ("X-Wire-User-Agent", "Jar/1")]));
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some("Jar/1"));
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some("Jar/1"));
        run(&shared, with_headers(get_request(addr, "/"), &[JAR, ("User-Agent", "Jar/2")]));
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some("Jar/2"));
        run(&shared, with_headers(get_request(addr, "/"), &[JAR]));
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some(DEFAULT_UA));
        let r = run(&shared, with_headers(get_request(addr, "/"), &[JAR, ("User-Agent", "bad\r\nx")]));
        assert_eq!((r.status, hdr(&r, "X-Wire-Error")), (400, Some("bad-user-agent")));
    }

    #[test]
    fn public_only_pool_path_would_honour_the_user_agent() {
        // public-only refuses the loopback fixture, so check the request
        // builder indirectly: the refusal must not depend on the UA, and the
        // plain pool with the same UA still works afterwards.
        let (addr, rx) = ua_server();
        let shared = Shared::new(true);
        let mut h = PO.to_vec();
        h.push(("X-Wire-User-Agent", "Pub/1"));
        let r = run_on(&build_agent(), &shared, with_headers(get_request(addr, "/"), &h));
        assert_eq!((r.status, hdr(&r, "X-Wire-Error")), (403, Some("policy")));
        assert!(rx.try_recv().is_err());
        let r = run_on(&build_agent(), &shared, with_headers(get_request(addr, "/"), &[("X-Wire-User-Agent", "Pub/1")]));
        assert_eq!(r.status, 200);
        assert_eq!(ua_of(&rx.recv().unwrap()).as_deref(), Some("Pub/1"));
    }

    #[test]
    fn every_response_advertises_the_ua_feature() {
        let (addr, _rx) = ua_server();
        assert_eq!(hdr(&execute_for_test(&build_agent(), &get_request(addr, "/")), "X-Wire-Features"), Some("ua,ws,jarfile,token"));
        let down = get_request("127.0.0.1:1".parse().unwrap(), "/");
        assert_eq!(hdr(&execute_for_test(&build_agent(), &down), "X-Wire-Features"), Some("ua,ws,jarfile,token"));
        let bad = with_headers(get_request(addr, "/"), &[("X-Wire-User-Agent", "a\r\nb")]);
        assert_eq!(hdr(&execute_for_test(&build_agent(), &bad), "X-Wire-Features"), Some("ua,ws,jarfile,token"));
    }
}

#[cfg(test)]
mod mount_wire_tests {
    use super::*;

    /// A mounted origin is answered through the normal request path with no
    /// network involved (nothing listens on web-platform.test:8000).
    #[test]
    fn mounted_origin_is_served_without_a_socket_even_under_public_only() {
        let dir = std::env::temp_dir().join(format!("wire-mount-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("resources")).unwrap();
        std::fs::write(dir.join("resources/a.txt"), "hello").unwrap();
        let mut shared = Shared::new(true);
        shared.mounts.add_mount(&format!("http://web-platform.test:8000={}", dir.display())).unwrap();
        let req = |m: &str, u: &str, h: &[(&str, &str)]| ParsedRequest {
            method: m.into(),
            url: Url::parse(u).unwrap(),
            headers: h.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            body: vec![],
        };
        let mut raw = Vec::new();
        execute_with(&build_agent(), &shared, &req("GET", "http://www1.web-platform.test:8001/resources/a.txt", &[("X-Wire-Policy", "public-only")]), &mut raw).unwrap();
        let text = String::from_utf8(raw).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(text.contains("Content-Type: text/plain\r\n") && text.contains("X-Wire-Version: 3\r\n"), "{text}");
        assert!(text.ends_with("\r\n\r\nhello"), "{text}");
        let mut raw = Vec::new();
        execute_with(&build_agent(), &shared, &req("GET", "http://web-platform.test:8000/resources/../../etc/passwd", &[]), &mut raw).unwrap();
        assert!(String::from_utf8(raw).unwrap().starts_with("HTTP/1.1 404"), "traversal collapsed by URL parsing stays inside the mount");
    }
}
