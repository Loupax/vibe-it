// QF19w2: page `WebSocket` over wire's upgrade pipe. Framing unit tests, and
// end-to-end tests through a REAL wire built from ../wire (own socket) against
// a local echo server with a hand-written handshake. Tests that need the real
// wire skip (with a message) when it cannot be built.
#[allow(unused_imports)]
use super::*;
use crate::ws_frames::*;
use std::net::{TcpListener, TcpStream as TestTcp};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

// ------------------------------------------------------------ framing

#[test]
fn ws_accept_key_matches_the_rfc_example() {
    assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    assert_eq!(sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect::<String>(), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(sha1(b"").iter().map(|b| format!("{b:02x}")).collect::<String>(), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_ne!(new_key(), new_key());
}

fn unmask(f: &[u8]) -> (u8, Vec<u8>, usize) {
    // Decodes one masked client frame: (first byte, payload, header length).
    let (len, head) = match f[1] & 0x7F {
        126 => (u16::from_be_bytes([f[2], f[3]]) as usize, 4),
        127 => (u64::from_be_bytes(f[2..10].try_into().unwrap()) as usize, 10),
        n => (n as usize, 2),
    };
    assert!(f[1] & 0x80 != 0, "client frames are masked");
    let m = [f[head], f[head + 1], f[head + 2], f[head + 3]];
    let p: Vec<u8> = f[head + 4..head + 4 + len].iter().enumerate().map(|(i, b)| b ^ m[i & 3]).collect();
    (f[0], p, head)
}

#[test]
fn ws_encode_masks_and_uses_the_right_length_form() {
    for (n, head) in [(0usize, 2usize), (1, 2), (125, 2), (126, 4), (127, 4), (65535, 4), (65536, 10), (100_000, 10)] {
        let payload: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
        let f = encode_frame(true, OP_BINARY, &payload, [1, 2, 3, 4]);
        assert_eq!(f.len(), head + 4 + n, "n={n}");
        let (b0, p, h) = unmask(&f);
        assert_eq!((b0, h), (0x82, head));
        assert_eq!(p, payload);
        if n > 3 {
            assert_ne!(&f[head + 4..head + 8], &payload[..4], "the payload is masked");
        }
    }
    assert_eq!(encode_frame(false, OP_TEXT, b"a", [0; 4])[0], 0x01, "fin clear");
    assert_eq!(close_payload(Some(1000), "bye"), [0x03, 0xE8, b'b', b'y', b'e']);
    assert!(close_payload(None, "ignored").is_empty());
}

fn server_frame(fin: bool, op: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![if fin { 0x80 } else { 0 } | op];
    if payload.len() <= 125 {
        v.push(payload.len() as u8);
    } else if payload.len() <= 65535 {
        v.push(126);
        v.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        v.push(127);
        v.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    v.extend_from_slice(payload);
    v
}

fn decode_all(bytes: &[u8]) -> Result<Vec<Incoming>, Violation> {
    let (mut d, mut a, mut out) = (FrameDecoder::default(), Assembler::default(), vec![]);
    d.push(bytes);
    while let Some(f) = d.next_frame()? {
        if let Some(i) = a.accept(f)? {
            out.push(i);
        }
    }
    Ok(out)
}

#[test]
fn ws_decoder_handles_length_boundaries_and_split_input() {
    for n in [0usize, 1, 125, 126, 127, 65535, 65536, 70000] {
        let p = vec![b'x'; n];
        let got = decode_all(&server_frame(true, OP_TEXT, &p)).unwrap();
        assert_eq!(got, vec![Incoming::Text("x".repeat(n))], "n={n}");
    }
    // one byte at a time
    let bytes = [server_frame(true, OP_TEXT, b"hello"), server_frame(true, OP_BINARY, &[1, 2, 3])].concat();
    let (mut d, mut a, mut out) = (FrameDecoder::default(), Assembler::default(), vec![]);
    for b in &bytes {
        d.push(&[*b]);
        while let Some(f) = d.next_frame().unwrap() {
            out.extend(a.accept(f).unwrap());
        }
    }
    assert_eq!(out, vec![Incoming::Text("hello".into()), Incoming::Binary(vec![1, 2, 3])]);
}

#[test]
fn ws_fragmentation_with_interleaved_control_frames() {
    let bytes = [
        server_frame(false, OP_TEXT, b"hel"),
        server_frame(true, OP_PING, b"p"),
        server_frame(false, OP_CONT, b"lo, "),
        server_frame(true, OP_CONT, "wörld".as_bytes()),
        server_frame(true, OP_PONG, b""),
    ]
    .concat();
    assert_eq!(
        decode_all(&bytes).unwrap(),
        vec![Incoming::Ping(b"p".to_vec()), Incoming::Text("hello, wörld".into()), Incoming::Pong(vec![])]
    );
    // a multi-byte character split across fragments is fine
    let w = "é".as_bytes();
    let split = [server_frame(false, OP_TEXT, &w[..1]), server_frame(true, OP_CONT, &w[1..])].concat();
    assert_eq!(decode_all(&split).unwrap(), vec![Incoming::Text("é".into())]);
}

#[test]
fn ws_decoder_rejects_violations_with_the_right_codes() {
    let code = |b: Vec<u8>| decode_all(&b).unwrap_err().code;
    assert_eq!(code(server_frame(true, OP_TEXT, &[0xFF, 0xFE])), 1007, "invalid UTF-8");
    assert_eq!(code(server_frame(true, OP_CLOSE, &[0x03, 0xE8, 0xFF])), 1007, "close reason not UTF-8");
    assert_eq!(code(vec![0xC1, 0]), 1002, "rsv1");
    assert_eq!(code(vec![0x83, 0]), 1002, "unknown opcode");
    assert_eq!(code(vec![0x81, 0x80, 0, 0, 0, 0]), 1002, "masked server frame");
    assert_eq!(code(server_frame(false, OP_PING, b"")), 1002, "fragmented control");
    assert_eq!(code(server_frame(true, OP_PING, &[0; 126])), 1002, "long control");
    assert_eq!(code(server_frame(true, OP_CONT, b"x")), 1002, "orphan continuation");
    assert_eq!(code([server_frame(false, OP_TEXT, b"a"), server_frame(true, OP_TEXT, b"b")].concat()), 1002, "new message in a fragmented one");
    assert_eq!(code(server_frame(true, OP_CLOSE, &[1])), 1002, "1-byte close payload");
    assert_eq!(code(server_frame(true, OP_CLOSE, &1005u16.to_be_bytes())), 1002, "reserved close code");
    // oversize: a header announcing more than the cap is refused before the payload arrives
    let mut huge = vec![0x82, 127];
    huge.extend_from_slice(&((MAX_MESSAGE as u64) + 1).to_be_bytes());
    assert_eq!(code(huge), 1009);
    // oversize by fragments
    let chunk = vec![0u8; MAX_MESSAGE / 2 + 1];
    assert_eq!(code([server_frame(false, OP_BINARY, &chunk), server_frame(true, OP_CONT, &chunk)].concat()), 1009);
    assert_eq!(decode_all(&server_frame(true, OP_CLOSE, &[])).unwrap(), vec![Incoming::Close(None, String::new())]);
    assert_eq!(decode_all(&server_frame(true, OP_CLOSE, &[0x0F, 0xA0, b'x'])).unwrap(), vec![Incoming::Close(Some(4000), "x".into())]);
}

// ------------------------------------------------------------ real wire + echo server

fn wire_bin() -> Option<&'static std::path::PathBuf> {
    static BIN: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Ok(p) = std::env::var("SCRATCHY_TEST_WIRE") {
            return Some(p.into());
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let wire = manifest.join("../wire/Cargo.toml");
        let target = manifest.join("target/wire-for-tests");
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        for offline in [true, false] {
            let mut c = std::process::Command::new(&cargo);
            c.args(["build", "--release", "--locked", "--manifest-path"]).arg(&wire).arg("--target-dir").arg(&target);
            if offline {
                c.arg("--offline");
            }
            if c.output().is_ok_and(|o| o.status.success()) {
                return Some(target.join("release/wire"));
            }
        }
        None
    })
    .as_ref()
}

struct RealWire {
    child: std::process::Child,
    path: std::path::PathBuf,
}

impl Drop for RealWire {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.path);
    }
}

fn start_wire(tag: &str) -> Option<RealWire> {
    let Some(bin) = wire_bin() else {
        eprintln!("SKIP: wire cannot be built here");
        return None;
    };
    let path = std::env::temp_dir().join(format!("qf19w2-{}-{tag}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let child = std::process::Command::new(bin)
        .arg("--socket")
        .arg(&path)
        // W-CACHE1: a test wire never writes into ~/.cache/wire.
        .arg("--no-cache")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let w = RealWire { child, path };
    let t0 = Instant::now();
    while !w.path.exists() {
        assert!(t0.elapsed() < Duration::from_secs(10), "wire did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    Some(w)
}

/// Reads one masked client frame from the server side.
fn read_client_frame(s: &mut TestTcp) -> Option<(u8, Vec<u8>)> {
    let mut h = [0u8; 2];
    s.read_exact(&mut h).ok()?;
    let len = match h[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).ok()?;
            u16::from_be_bytes(b) as usize
        }
        127 => {
            let mut b = [0u8; 8];
            s.read_exact(&mut b).ok()?;
            u64::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    let mut m = [0u8; 4];
    s.read_exact(&mut m).ok()?;
    let mut p = vec![0u8; len];
    s.read_exact(&mut p).ok()?;
    for (i, b) in p.iter_mut().enumerate() {
        *b ^= m[i & 3];
    }
    Some((h[0] & 0x0F, p))
}

#[derive(Clone, Default)]
struct Echo {
    /// Subprotocol the server selects from the offer (None: select nothing).
    pick_protocol: Option<String>,
    /// A wrong Sec-WebSocket-Accept.
    bad_accept: bool,
    /// Answer 403 instead of upgrading.
    refuse: bool,
}

struct EchoServer {
    port: u16,
    /// Requests seen (the raw handshake text), for header assertions.
    seen: Arc<Mutex<Vec<String>>>,
}

fn start_echo(cfg: Echo) -> EchoServer {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (cfg, seen) = (cfg.clone(), seen2.clone());
            thread::spawn(move || echo_conn(s, cfg, seen));
        }
    });
    EchoServer { port, seen }
}

fn echo_conn(mut s: TestTcp, cfg: Echo, seen: Arc<Mutex<Vec<String>>>) {
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if s.read_exact(&mut b).is_err() {
            return;
        }
        head.push(b[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    seen.lock().unwrap().push(text.clone());
    if cfg.refuse {
        let _ = s.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno");
        return;
    }
    let hdr = |n: &str| text.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().to_string()));
    let key = hdr("sec-websocket-key").unwrap_or_default();
    let mut accept = accept_key(&key);
    if cfg.bad_accept {
        accept = accept_key("other");
    }
    let mut resp = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n");
    if let Some(p) = &cfg.pick_protocol {
        resp.push_str(&format!("Sec-WebSocket-Protocol: {p}\r\n"));
    }
    resp.push_str("\r\n");
    if s.write_all(resp.as_bytes()).is_err() {
        return;
    }
    while let Some((op, p)) = read_client_frame(&mut s) {
        let w = |s: &mut TestTcp, b: Vec<u8>| s.write_all(&b).is_ok();
        let ok = match op {
            0x1 => {
                let t = String::from_utf8_lossy(&p).into_owned();
                if t == "frag" {
                    w(&mut s, server_frame(false, OP_TEXT, b"hel")) && w(&mut s, server_frame(true, OP_PING, b"mid")) && w(&mut s, server_frame(false, OP_CONT, b"lo ")) && w(&mut s, server_frame(true, OP_CONT, b"frag"))
                } else if t == "ping" {
                    // expect the pong for our ping before answering
                    w(&mut s, server_frame(true, OP_PING, b"are-you-there"))
                } else if let Some(rest) = t.strip_prefix("close:") {
                    let (code, reason) = rest.split_once(':').unwrap_or((rest, ""));
                    let mut pl = code.parse::<u16>().unwrap().to_be_bytes().to_vec();
                    pl.extend_from_slice(reason.as_bytes());
                    w(&mut s, server_frame(true, OP_CLOSE, &pl))
                } else if t == "badutf8" {
                    w(&mut s, server_frame(true, OP_TEXT, &[0xFF, 0xFE]))
                } else if t == "drop" {
                    return;
                } else {
                    w(&mut s, server_frame(true, OP_TEXT, &p))
                }
            }
            0x2 => w(&mut s, server_frame(true, OP_BINARY, &p)),
            0xA => w(&mut s, server_frame(true, OP_TEXT, format!("pong:{}", String::from_utf8_lossy(&p)).as_bytes())),
            0x8 => {
                // echo the close frame and finish
                let _ = s.write_all(&server_frame(true, OP_CLOSE, &p));
                return;
            }
            _ => true,
        };
        if !ok {
            return;
        }
    }
}

struct Page {
    engine: JsEngine,
    log: Rc<RefCell<Vec<String>>>,
    _wire: Option<RealWire>,
}

/// An engine on `page` whose executor is wire at `sock`; private targets are
/// allowed so wire may reach the loopback echo server.
fn ws_page(page: &str, sock: Option<std::path::PathBuf>) -> Page {
    set_page_url(Some(&Url::parse(page).unwrap()));
    match sock {
        Some(p) => crate::js_net::set_net_executor(Arc::new(crate::js_net::WireExecutor(p))),
        None => crate::js_net::set_net_executor(Arc::new(crate::js_net::NoSocketExecutor)),
    }
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    let engine = crate::js::start_scripts(&parse_html(""), &[src("")], Rc::new(move |lv, t| l.borrow_mut().push(format!("{lv}:{t}")))).unwrap();
    let mut p = engine.net_policy();
    p.private_targets = crate::net_policy::PrivatePolicy::Allow;
    engine.set_net_policy(p);
    Page { engine, log, _wire: None }
}

fn real_page(tag: &str, page: &str) -> Option<Page> {
    let w = start_wire(tag)?;
    let mut p = ws_page(page, Some(w.path.clone()));
    p._wire = Some(w);
    Some(p)
}

impl Page {
    fn eval(&self, code: &str) {
        self.engine.eval("t", code);
    }
    fn logs(&self) -> Vec<String> {
        self.log.borrow().iter().filter_map(|l| l.strip_prefix("log:").map(str::to_string)).collect()
    }
    /// Pumps socket events until a log line equals `want` (or fails after 10 s).
    fn wait_for(&self, want: &str) {
        let t0 = Instant::now();
        loop {
            while self.engine.pump_ws() {}
            if self.logs().iter().any(|l| l == want) {
                return;
            }
            assert!(t0.elapsed() < Duration::from_secs(10), "timeout waiting for {want:?}; log: {:?}", self.log.borrow());
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn settle(&self, ms: u64) {
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_millis(ms) {
            while self.engine.pump_ws() {}
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

const WS_TRACE: &str = "
var ws; var ev = function(t){ return function(e){ console.log(t + (e.data !== undefined ? ':' + (typeof e.data === 'string' ? e.data : Object.prototype.toString.call(e.data)) : '') + (e.code !== undefined ? ':' + e.code + ':' + e.reason + ':' + e.wasClean : '')); }; };
function wire(w){ w.onopen = ev('open'); w.onmessage = ev('message'); w.onerror = ev('error'); w.onclose = ev('close'); return w; }
";

#[test]
fn ws_text_echo_event_order_ready_state_and_buffered_amount() {
    let Some(p) = real_page("text", "http://page.test/app") else { return };
    let srv = start_echo(Echo::default());
    p.eval(&format!(
        "{WS_TRACE}
         ws = wire(new WebSocket('ws://127.0.0.1:{}/chat'));
         console.log('rs' + ws.readyState + ' ' + ws.url + ' [' + ws.extensions + '] ' + ws.protocol + '|' + ws.binaryType);
         try {{ ws.send('early'); }} catch (e) {{ console.log('early:' + e.name); }}
         console.log(WebSocket.CONNECTING + '' + WebSocket.OPEN + WebSocket.CLOSING + WebSocket.CLOSED + (ws.OPEN === 1));
         ws.addEventListener('open', function () {{ ws.send('héllo'); console.log('buffered:' + ws.bufferedAmount); ws.send('x'); console.log('buffered:' + ws.bufferedAmount); }});",
        srv.port
    ));
    p.wait_for("message:héllo");
    p.wait_for("message:x");
    p.settle(100);
    p.eval("console.log('rs' + ws.readyState + ' buffered=' + ws.bufferedAmount); ws.close(1000, 'done');");
    p.wait_for("close:1000:done:true");
    let l = p.logs();
    assert_eq!(l[0], format!("rs0 ws://127.0.0.1:{}/chat [] |blob", srv.port));
    assert_eq!(l[1], "early:InvalidStateError");
    assert_eq!(l[2], "0123true");
    let pos = |s: &str| l.iter().position(|x| x == s).unwrap_or_else(|| panic!("{s} missing in {l:?}"));
    assert!(pos("buffered:6") < pos("buffered:7") && pos("open") < pos("message:héllo") && pos("buffered:7") < pos("message:héllo") && pos("message:héllo") < pos("message:x"), "{l:?}");
    assert!(l.contains(&"rs1 buffered=0".to_string()), "bufferedAmount returns to 0 once written: {l:?}");
    assert_eq!(p.eval_rs(), 3);
    let hs = srv.seen.lock().unwrap()[0].clone();
    for want in ["GET /chat HTTP/1.1", "Origin: http://page.test", "Sec-WebSocket-Version: 13", "Upgrade: websocket", "Connection: Upgrade"] {
        assert!(hs.contains(want), "{want} missing in the upstream handshake:\n{hs}");
    }
    assert!(!hs.to_ascii_lowercase().contains("x-wire-"), "{hs}");
}

impl Page {
    fn eval_rs(&self) -> u32 {
        self.engine.eval_value::<u32>("t", "ws.readyState").unwrap()
    }
}

#[test]
fn ws_binary_arraybuffer_blob_and_views() {
    let Some(p) = real_page("bin", "http://page.test/") else { return };
    let srv = start_echo(Echo::default());
    p.eval(&format!(
        "{WS_TRACE}
         ws = new WebSocket('ws://127.0.0.1:{}/');
         ws.binaryType = 'arraybuffer';
         ws.onmessage = function (e) {{ console.log('ab:' + (e.data instanceof ArrayBuffer) + ':' + Array.from(new Uint8Array(e.data)).join(',')); }};
         ws.onopen = function () {{
           ws.send(new Uint8Array([1, 2, 3]).buffer);
           ws.send(new Uint8Array([9, 8, 7, 6, 5]).subarray(1, 4));
           ws.send(new Blob([new Uint8Array([200, 201]), 'A']));
           ws.send(new DataView(new Uint8Array([4, 4]).buffer));
         }};",
        srv.port
    ));
    p.wait_for("ab:true:4,4");
    let l = p.logs();
    assert_eq!(l, vec!["ab:true:1,2,3", "ab:true:8,7,6", "ab:true:200,201,65", "ab:true:4,4"]);
    // binaryType blob (the default)
    p.eval(&format!(
        "var w2 = new WebSocket('ws://127.0.0.1:{}/');
         w2.onopen = function () {{ w2.send(new Uint8Array([65, 66])); }};
         w2.onmessage = function (e) {{ console.log('blob:' + (e.data instanceof Blob) + ':' + e.data.size); w2.close(); }};",
        srv.port
    ));
    p.wait_for("blob:true:2");
}

#[test]
fn ws_fragmentation_ping_pong_and_message_event_fields() {
    let Some(p) = real_page("frag", "http://page.test/") else { return };
    let srv = start_echo(Echo::default());
    p.eval(&format!(
        "{WS_TRACE}
         ws = wire(new WebSocket('ws://127.0.0.1:{}/'));
         ws.addEventListener('message', function (e) {{ console.log('origin:' + e.origin + ':' + (e instanceof MessageEvent)); }});
         ws.onopen = function () {{ ws.send('frag'); }};",
        srv.port
    ));
    p.wait_for("message:hello frag");
    // the server pinged in the middle of the fragments: our pong came back as a text "pong:mid"
    p.eval("ws.send('ping');");
    p.wait_for("message:pong:are-you-there");
    let l = p.logs();
    assert!(l.contains(&format!("origin:ws://127.0.0.1:{}:true", srv.port)), "{l:?}");
}

#[test]
fn ws_close_codes_both_ways() {
    let Some(p) = real_page("close", "http://page.test/") else { return };
    let srv = start_echo(Echo::default());
    // server-initiated
    p.eval(&format!(
        "{WS_TRACE}
         ws = wire(new WebSocket('ws://127.0.0.1:{}/'));
         ws.onopen = function () {{ ws.send('close:4001:server says bye'); }};",
        srv.port
    ));
    p.wait_for("close:4001:server says bye:true");
    assert_eq!(p.eval_rs(), 3);
    // client-initiated with a code and a reason; the server echoes the frame
    p.eval(&format!(
        "ws = wire(new WebSocket('ws://127.0.0.1:{}/'));
         ws.onopen = function () {{ ws.close(3456, 'client bye'); console.log('state:' + ws.readyState); ws.close(); }};",
        srv.port
    ));
    p.wait_for("close:3456:client bye:true");
    assert!(p.logs().contains(&"state:2".to_string()));
    // validation
    p.eval(&format!(
        "ws = new WebSocket('ws://127.0.0.1:{}/');
         try {{ ws.close(1001); }} catch (e) {{ console.log('c1001:' + e.name); }}
         try {{ ws.close(2999); }} catch (e) {{ console.log('c2999:' + e.name); }}
         try {{ ws.close(1000, 'x'.repeat(124)); }} catch (e) {{ console.log('long:' + e.name); }}
         try {{ ws.close(4999, 'é'.repeat(62)); }} catch (e) {{ console.log('long2:' + e.name); }}
         console.log('rs' + ws.readyState);",
        srv.port
    ));
    let l = p.logs();
    for want in ["c1001:InvalidAccessError", "c2999:InvalidAccessError", "long:SyntaxError", "long2:SyntaxError", "rs0"] {
        assert!(l.contains(&want.to_string()), "{want} missing: {l:?}");
    }
    // close while CONNECTING: error then close 1006, never open
    p.eval(&format!(
        "{WS_TRACE}
         ws = wire(new WebSocket('ws://127.0.0.1:{}/')); ws.close(); console.log('rs' + ws.readyState);",
        srv.port
    ));
    p.settle(300);
    let l = p.logs();
    let n = l.len();
    assert_eq!(l[n - 3..], ["rs2", "error", "close:1006::false"], "{l:?}");
    assert!(!l.contains(&"open".to_string()));
    // abnormal: the server drops the TCP connection without a close frame
    p.eval(&format!(
        "ws = wire(new WebSocket('ws://127.0.0.1:{}/')); ws.onopen = function () {{ ws.send('drop'); }}; ws.onclose = function (e) {{ console.log('dropclose:' + e.code + ':' + e.wasClean); }};",
        srv.port
    ));
    p.wait_for("dropclose:1006:false");
}

#[test]
fn ws_protocol_negotiation_and_handshake_failures() {
    let Some(p) = real_page("proto", "http://page.test/") else { return };
    let srv = start_echo(Echo { pick_protocol: Some("chat.v2".into()), ..Default::default() });
    p.eval(&format!(
        "{WS_TRACE}
         ws = wire(new WebSocket('ws://127.0.0.1:{}/', ['chat.v1', 'chat.v2']));
         ws.onopen = function () {{ console.log('proto:' + ws.protocol); ws.close(); }};",
        srv.port
    ));
    p.wait_for("proto:chat.v2");
    let hs = srv.seen.lock().unwrap()[0].clone();
    assert!(hs.contains("Sec-WebSocket-Protocol: chat.v1, chat.v2"), "{hs}");
    // a server that picks a protocol that was not offered
    let bad = start_echo(Echo { pick_protocol: Some("nope".into()), ..Default::default() });
    p.eval(&format!("{WS_TRACE} ws = wire(new WebSocket('ws://127.0.0.1:{}/', 'chat'));", bad.port));
    p.wait_for("close:1006::false");
    assert!(p.log.borrow().iter().any(|l| l.starts_with("net:WebSocket ws://127.0.0.1:") && l.contains("not offered")), "{:?}", p.log.borrow());
    assert!(!p.logs().contains(&"open".to_string()));
    // wrong Sec-WebSocket-Accept
    let ba = start_echo(Echo { bad_accept: true, ..Default::default() });
    p.eval(&format!("{WS_TRACE} ws = wire(new WebSocket('ws://127.0.0.1:{}/'));", ba.port));
    p.settle(500);
    assert!(p.log.borrow().iter().any(|l| l.contains("Sec-WebSocket-Accept is wrong")), "{:?}", p.log.borrow());
    // a refusal (403): wire hands the buffered answer back
    let rf = start_echo(Echo { refuse: true, ..Default::default() });
    p.eval(&format!("{WS_TRACE} ws = wire(new WebSocket('ws://127.0.0.1:{}/'));", rf.port));
    p.settle(500);
    assert!(p.log.borrow().iter().any(|l| l.contains("refused: HTTP 403")), "{:?}", p.log.borrow());
    // bad server text frame: protocol failure
    let ok = start_echo(Echo::default());
    p.eval(&format!("{WS_TRACE} ws = wire(new WebSocket('ws://127.0.0.1:{}/')); ws.onopen = function () {{ ws.send('badutf8'); }};", ok.port));
    p.settle(500);
    assert!(p.log.borrow().iter().any(|l| l.contains("text message is not UTF-8")), "{:?}", p.log.borrow());
}

#[test]
fn ws_constructor_validation() {
    let p = ws_page("http://page.test/dir/page.html", None);
    p.eval(
        "function t(name, f) { try { var w = f(); console.log(name + ':ok:' + w.url); w.onerror = function(){}; } catch (e) { console.log(name + ':' + e.name); } }
         t('rel', function () { return new WebSocket('/sock'); });
         t('http', function () { return new WebSocket('http://a.test/x'); });
         t('https', function () { return new WebSocket('https://a.test/x'); });
         t('ftp', function () { return new WebSocket('ftp://a.test/x'); });
         t('frag', function () { return new WebSocket('ws://a.test/x#f'); });
         t('bad', function () { return new WebSocket('ws://'); });
         t('dup', function () { return new WebSocket('ws://a.test/', ['a', 'a']); });
         t('tok', function () { return new WebSocket('ws://a.test/', 'a b'); });
         t('noargs', function () { return new WebSocket(); });
         try { WebSocket('ws://a.test'); } catch (e) { console.log('call:' + e.name); }",
    );
    let l = p.logs();
    for want in [
        "rel:ok:ws://page.test/sock", "http:ok:ws://a.test/x", "https:ok:wss://a.test/x", "ftp:SyntaxError", "frag:SyntaxError", "bad:SyntaxError",
        "dup:SyntaxError", "tok:SyntaxError", "noargs:TypeError", "call:TypeError",
    ] {
        assert!(l.contains(&want.to_string()), "{want} missing: {l:?}");
    }
}

#[test]
fn ws_mixed_content_and_policy_block_with_error_and_close_1006() {
    let p = ws_page("https://secure.test/", Some("/nonexistent".into()));
    p.eval(&format!("{WS_TRACE} ws = wire(new WebSocket('ws://plain.test/x'));"));
    p.settle(100);
    let l = p.logs();
    assert_eq!(l, vec!["error", "close:1006::false"]);
    assert!(p.log.borrow().iter().any(|x| x.starts_with("net:blocked WebSocket ws://plain.test/x: mixed content")), "{:?}", p.log.borrow());
    // a bad port
    let p = ws_page("http://page.test/", Some("/nonexistent".into()));
    p.eval(&format!("{WS_TRACE} ws = wire(new WebSocket('ws://other.test:6667/x'));"));
    p.settle(100);
    assert!(p.log.borrow().iter().any(|x| x.contains("port 6667 is not allowed")), "{:?}", p.log.borrow());
    assert_eq!(p.logs(), vec!["error", "close:1006::false"]);
    // private target with the default policy (Block would be a user choice; Warn logs)
    let p = ws_page("http://page.test/", None);
    let mut pol = p.engine.net_policy();
    pol.private_targets = crate::net_policy::PrivatePolicy::Block;
    p.engine.set_net_policy(pol);
    p.eval(&format!("{WS_TRACE} ws = wire(new WebSocket('ws://192.168.1.5/x'));"));
    p.settle(100);
    assert!(p.log.borrow().iter().any(|x| x.contains("private, loopback or link-local")), "{:?}", p.log.borrow());
    // no wire at all
    let p = ws_page("http://page.test/", None);
    p.eval(&format!("{WS_TRACE} ws = wire(new WebSocket('ws://x.test/x'));"));
    p.settle(100);
    assert_eq!(p.logs(), vec!["error", "close:1006::false"]);
}

#[test]
fn ws_old_wire_fails_closed_with_one_log_line() {
    // A "wire" that answers every request as an ordinary response, with no `ws` feature.
    let path = std::env::temp_dir().join(format!("qf19w2-old-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let hits = Arc::new(Mutex::new(0usize));
    let h2 = hits.clone();
    thread::spawn(move || {
        for mut s in l.incoming().flatten() {
            *h2.lock().unwrap() += 1;
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nX-Wire-Version: 2\r\nX-Wire-Features: ua\r\nContent-Length: 2\r\n\r\nhi");
        }
    });
    let p = ws_page("http://page.test/", Some(path.clone()));
    p.eval(&format!("{WS_TRACE} ws = wire(new WebSocket('ws://x.test/a'));"));
    p.wait_for("close:1006::false");
    // a second socket fails at once, without another request to wire
    p.eval(&format!("{WS_TRACE} ws = wire(new WebSocket('ws://x.test/b'));"));
    p.settle(300);
    let _ = std::fs::remove_file(&path);
    let logs = p.log.borrow().clone();
    assert_eq!(logs.iter().filter(|x| x.starts_with("net:WebSocket") && x.contains("wire is too old")).count(), 1, "{logs:?}");
    assert_eq!(logs.iter().filter(|x| x.as_str() == "log:error").count(), 2);
    assert_eq!(*hits.lock().unwrap(), 1, "the second socket did not reach wire");
}

#[test]
fn ws_cap_of_sixteen_sockets_and_engine_drop_closes_with_1001() {
    let Some(p) = real_page("cap", "http://page.test/") else { return };
    let srv = start_echo(Echo::default());
    p.eval(&format!(
        "var socks = []; for (var i = 0; i < 17; i++) {{ var w = new WebSocket('ws://127.0.0.1:{}/'); w.onopen = function(){{ console.log('open'); }}; w.onclose = function(e){{ console.log('close:' + e.code); }}; socks.push(w); }}",
        srv.port
    ));
    p.wait_for("close:1006");
    let t0 = Instant::now();
    loop {
        p.settle(10);
        if p.logs().iter().filter(|l| *l == "open").count() == 16 || t0.elapsed() > Duration::from_secs(10) {
            break;
        }
    }
    assert_eq!(p.logs().iter().filter(|l| *l == "open").count(), 16);
    assert!(p.log.borrow().iter().any(|l| l.contains("too many open WebSockets")));
    assert_eq!(p.engine.ws_socket_count(), 16);
}

#[test]
fn ws_engine_drop_sends_close_1001_to_the_server() {
    let Some(w) = start_wire("drop") else { return };
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel::<Option<(u8, Vec<u8>)>>();
    thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            s.read_exact(&mut b).unwrap();
            head.push(b[0]);
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let key = text.lines().find_map(|l| l.strip_prefix("Sec-WebSocket-Key: ")).unwrap().to_string();
        s.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(&key)).as_bytes()).unwrap();
        let _ = tx.send(read_client_frame(&mut s));
    });
    let p = ws_page("http://page.test/", Some(w.path.clone()));
    p.eval(&format!("var ws = new WebSocket('ws://127.0.0.1:{port}/'); ws.onopen = function () {{ console.log('open'); }};"));
    p.wait_for("open");
    drop(p);
    let got = rx.recv_timeout(Duration::from_secs(10)).expect("server saw nothing");
    let (op, payload) = got.expect("connection closed without a close frame");
    assert_eq!(op, 0x8);
    assert_eq!(&payload[..2], &1001u16.to_be_bytes());
    drop(w);
}

#[test]
fn ws_message_flood_keeps_order() {
    let Some(p) = real_page("flood", "http://page.test/") else { return };
    let srv = start_echo(Echo::default());
    p.eval(&format!(
        "var got = []; var ws = new WebSocket('ws://127.0.0.1:{}/');
         ws.onopen = function () {{ for (var i = 0; i < 500; i++) ws.send('m' + i); }};
         ws.onmessage = function (e) {{ got.push(e.data); if (got.length === 500) console.log('all:' + got.every(function (v, i) {{ return v === 'm' + i; }})); }};",
        srv.port
    ));
    p.wait_for("all:true");
}

// QPT1b2 (R53 D6): a dropped iframe realm closes the WebSockets it created (through wire), fires no
// event into the dropped realm, and leaves the page's own sockets alone.
fn realm_ws_case(tag: &str, drop_js: &str) {
    let Some(p) = real_page(tag, "http://page.test/") else { return };
    let srv = start_echo(Echo::default());
    p.eval(&format!(
        "var f = document.createElement('iframe'); document.body.appendChild(f); var w = f.contentWindow;\
         var url = 'ws://127.0.0.1:{}/';\
         var pg = new WebSocket(url); pg.onopen = function(){{ console.log('page open'); }}; pg.onmessage = function(e){{ console.log('page msg ' + e.data); }}; pg.onclose = function(){{ console.log('page close'); }};\
         var rs = new w.WebSocket(url); console.log('wrapper', rs instanceof w.WebSocket, rs instanceof WebSocket, w.WebSocket !== WebSocket, w.WebSocket.OPEN);\
         rs.onopen = function(){{ console.log('realm open'); }}; rs.onclose = function(){{ console.log('REALM close'); }}; rs.onerror = function(){{ console.log('REALM error'); }}; rs.onmessage = function(){{ console.log('REALM msg'); }};\
         rs.addEventListener('close', function(){{ console.log('REALM close2'); }});\
         var rc = new w.WebSocket(url);",
        srv.port
    ));
    p.wait_for("page open");
    p.wait_for("realm open");
    while p.engine.ws_socket_count() < 3 {
        p.settle(5);
    }
    p.eval(drop_js);
    let t0 = Instant::now();
    while p.engine.ws_socket_count() != 1 {
        assert!(t0.elapsed() < Duration::from_secs(10), "realm sockets still open: {}", p.engine.ws_socket_count());
        p.settle(10);
    }
    p.settle(100);
    p.eval("console.log('states', pg.readyState, rs.readyState); pg.send('hi');");
    p.wait_for("page msg hi");
    let logs = p.logs();
    assert!(logs.contains(&"wrapper true true true 1".to_string()), "{logs:?}");
    assert!(logs.contains(&"states 1 3".to_string()), "{logs:?}");
    assert!(!logs.iter().any(|l| l.contains("REALM") || l == "page close"), "{logs:?}");
    assert_eq!(p.engine.ws_socket_count(), 1);
}

#[test]
fn qpt1b2_removing_the_iframe_closes_its_websockets_not_the_pages() {
    realm_ws_case("rm", "f.remove();");
}

#[test]
fn qpt1b2_changing_the_iframe_src_closes_its_websockets_not_the_pages() {
    realm_ws_case("src", "f.src = 'about:blank#x';");
}
