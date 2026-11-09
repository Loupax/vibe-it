use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

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
// `snag` already trusts for this), and write back one normalized
// HTTP/1.1 response: real headers, redirects already followed, body
// already decompressed, framed with a correct `Content-Length` and
// never `Transfer-Encoding: chunked`. No bundling/inlining logic lives
// here — that's `snag`'s exclusive job, which is why this file is so
// much smaller than `snag/src/main.rs`.
//
// Unlike `snag --serve`'s one-connection-at-a-time loop, this daemon is
// expected to field several *separate* connections concurrently (a page
// with several forms/links worth of future per-resource fetches, each
// its own one-shot connection, no multiplexing on any single one) — so
// each accepted connection gets its own thread rather than being served
// serially.

fn print_usage() {
    eprintln!("usage: wire [--socket <path>] [--print-socket]");
    eprintln!("  wire                 listen on $XDG_RUNTIME_DIR/wire.sock (falling back to");
    eprintln!("                       /tmp/wire-$UID.sock), serving real HTTP requests read");
    eprintln!("                       off that socket in the absolute-form proxy shape");
    eprintln!("                       (`GET https://example.com/path HTTP/1.1`), one request");
    eprintln!("                       per connection, concurrently. Must be started manually.");
    eprintln!("  wire --socket <path> listen on <path> instead of the default location.");
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
    run_server(&path, agent);
}

fn build_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent("wire/0.1 (+https://github.com/Loupax/vibe-it) scratchy's real-networking daemon")
        .build()
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

fn run_server(path: &PathBuf, agent: ureq::Agent) {
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
                std::thread::spawn(move || handle_connection(stream, &agent));
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
fn handle_connection(stream: UnixStream, agent: &ureq::Agent) {
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
    let response = execute(agent, &request);
    if let Err(e) = write_response(&mut writer, &response)
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

/// What `wire` actually sends back — a normalized shape, not a 1:1 byte
/// relay. `headers` never contains `Content-Length`/`Transfer-Encoding`/
/// `Content-Encoding` (see `normalize_headers`): the daemon has already
/// read the real, possibly-chunked, possibly-compressed upstream
/// response to completion, so `body`'s length is simply its own correct
/// `Content-Length`, computed fresh, and there is nothing left to
/// decompress downstream.
struct NormalizedResponse {
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Request header names this file sets itself from `url`/`body`, or that
/// `ureq` manages internally — forwarding the client's own copy would
/// either conflict with what `ureq` is about to compute (`Host`,
/// `Content-Length`) or make no sense across the proxy hop (`Connection`,
/// which only ever described the *scratchy-to-wire* leg, not the
/// upstream one).
fn is_request_header_to_skip(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "host" | "content-length" | "connection")
}

/// Response header names `normalize_headers` strips before forwarding —
/// each one describes the *wire-to-real-server* transport leg, which no
/// longer applies once the body has already been read, decompressed, and
/// re-framed by this daemon.
fn is_response_header_to_strip(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "content-length" | "transfer-encoding" | "content-encoding")
}

/// Sends `request` for real via `agent` and always returns a response to
/// forward — never an `Err`, since a connection/TLS/DNS failure is just
/// as real an outcome to report back to scratchy as any HTTP status, and
/// the socket protocol has exactly one shape (a normalized response) for
/// every case, not a separate out-of-band error channel.
fn execute(agent: &ureq::Agent, request: &ParsedRequest) -> NormalizedResponse {
    let mut req = agent.request(&request.method, request.url.as_str());
    for (name, value) in &request.headers {
        if !is_request_header_to_skip(name) {
            req = req.set(name, value);
        }
    }
    let result =
        if request.body.is_empty() { req.call() } else { req.send_bytes(&request.body) };

    match result {
        // `ureq` treats any non-2xx status as `Err(Error::Status(..))`,
        // but the wrapped `Response` is a perfectly real upstream
        // response (a 404, a 500, ...) — not a transport failure — so it
        // goes through the exact same normalization path as `Ok`, just
        // reached from the other match arm.
        Ok(resp) | Err(ureq::Error::Status(_, resp)) => normalize_response(resp),
        Err(ureq::Error::Transport(e)) => synthetic_error_response(&e.to_string()),
    }
}

/// Reads a real upstream response to completion (decompressing
/// transparently — `ureq`'s `gzip` feature wraps the reader whenever
/// `Content-Encoding: gzip` was sent, the dominant real-world encoding;
/// see this crate's own `Cargo.toml` for why `brotli` isn't enabled too —
/// nothing has needed it yet) and re-frames it with a fresh,
/// now-correct `Content-Length` and no leftover transport-layer headers.
fn normalize_response(resp: ureq::Response) -> NormalizedResponse {
    let status = resp.status();
    let reason = resp.status_text().to_string();
    let mut headers = Vec::new();
    for name in resp.headers_names() {
        if is_response_header_to_strip(&name) {
            continue;
        }
        if let Some(value) = resp.header(&name) {
            headers.push((name, value.to_string()));
        }
    }
    let mut body = Vec::new();
    // A transport error reading the body (connection dropped mid-
    // stream) isn't fatal here either — whatever bytes were read
    // successfully are still forwarded, same lenient spirit `execute`'s
    // own transport-error handling already has for a failure before any
    // bytes arrived at all.
    let _ = resp.into_reader().read_to_end(&mut body);
    headers.push(("Content-Length".to_string(), body.len().to_string()));
    NormalizedResponse { status, reason, headers, body }
}

/// What gets sent back when the upstream request never got far enough
/// to produce a real HTTP response at all (DNS failure, connection
/// refused, TLS handshake failure, timeout) — a synthesized `502` so
/// scratchy's own dispatch (which only ever reads `Content-Type`) still
/// gets one consistent, well-formed response shape to work with, never
/// a raw error string with no framing.
fn synthetic_error_response(message: &str) -> NormalizedResponse {
    let body = format!("wire: upstream request failed: {message}").into_bytes();
    NormalizedResponse {
        status: 502,
        reason: "Bad Gateway".to_string(),
        headers: vec![
            ("Content-Type".to_string(), "text/plain".to_string()),
            ("Content-Length".to_string(), body.len().to_string()),
        ],
        body,
    }
}

fn write_response(w: &mut impl Write, resp: &NormalizedResponse) -> std::io::Result<()> {
    write!(w, "HTTP/1.1 {} {}\r\n", resp.status, resp.reason)?;
    for (name, value) in &resp.headers {
        write!(w, "{name}: {value}\r\n")?;
    }
    write!(w, "\r\n")?;
    w.write_all(&resp.body)?;
    w.flush()
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

    #[test]
    fn ordinary_response_round_trips_with_a_fresh_content_length() {
        let body = "hello from the fixture";
        let addr = spawn_fixture(
            format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                .into_bytes()
                .leak(),
        );
        let agent = build_agent();
        let resp = execute(&agent, &get_request(addr, "/"));
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, body.as_bytes());
        assert!(resp.headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("content-length") && v == &body.len().to_string()));
    }

    #[test]
    fn chunked_upstream_response_normalizes_to_content_length() {
        // Two chunks ("hello" + " world"), terminated by the 0-length
        // chunk — real `Transfer-Encoding: chunked` framing, exactly the
        // shape this daemon exists to never make scratchy decode itself.
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let addr = spawn_fixture(raw);
        let agent = build_agent();
        let resp = execute(&agent, &get_request(addr, "/"));
        assert_eq!(resp.body, b"hello world");
        assert!(
            !resp.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("transfer-encoding")),
            "Transfer-Encoding must not leak through a normalized response, got: {:?}",
            resp.headers
        );
        assert!(resp.headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("content-length") && v == "11"));
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
        let resp = execute(&agent, &get_request(addr, "/"));
        assert_eq!(resp.body, b"hello gzip");
        assert!(
            !resp.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-encoding")),
            "Content-Encoding must not leak through once the body is already decompressed, got: {:?}",
            resp.headers
        );
        assert!(resp.headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("content-length") && v == "10"));
    }

    #[test]
    fn a_real_4xx_from_upstream_forwards_as_itself_not_a_transport_error() {
        let addr = spawn_fixture(b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found");
        let agent = build_agent();
        let resp = execute(&agent, &get_request(addr, "/missing"));
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body, b"not found");
    }

    #[test]
    fn a_connection_failure_synthesizes_a_502_not_a_hang_or_a_crash() {
        // Nothing is listening on this address — ureq's own connect
        // attempt fails fast with a transport error.
        let agent = build_agent();
        let req = get_request("127.0.0.1:1".parse().expect("addr"), "/");
        let resp = execute(&agent, &req);
        assert_eq!(resp.status, 502);
        assert!(!resp.body.is_empty());
    }

    #[test]
    fn full_unix_socket_round_trip_matches_execute_directly() {
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
            handle_connection(stream, &server_agent);
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
}
