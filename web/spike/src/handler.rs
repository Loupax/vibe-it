//! W3: every http(s) request is answered from wire. CEF asks the client's
//! `RequestHandler` for a `ResourceRequestHandler`, which returns one
//! `ResourceHandler` per request; that handler sends the request to wire's Unix
//! socket and streams the answer back.
//!
//! Threads: `open`, `response_headers`, `read` and `cancel` run on CEF's IO thread
//! and never block. One thread per request connects to wire, sends the request,
//! reads the head (then `Callback::cont`), and afterwards serves each async `read`
//! by reading straight from the socket (then `ResourceReadCallback::cont`).

use crate::wire::{self, WireHead, WireRequest};
use cef::*;
use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// wire's socket (`WEB_SPIKE_WIRE`). None: no interception (W2 behaviour).
pub static WIRE_SOCKET: OnceLock<Option<String>> = OnceLock::new();

/// Counters for the "CEF made N requests, wire served M" comparison.
pub static SEEN_HTTP: AtomicU64 = AtomicU64::new(0); // http(s) requests CEF asked us about
pub static SEEN_OTHER: AtomicU64 = AtomicU64::new(0); // other schemes, left to CEF
pub static OPENED: AtomicU64 = AtomicU64::new(0); // ResourceHandlers opened (request ids)
pub static SENT: AtomicU64 = AtomicU64::new(0); // requests written to wire's socket
pub static ANSWERED: AtomicU64 = AtomicU64::new(0); // wire answered with a head
pub static FAILED: AtomicU64 = AtomicU64::new(0); // wire unreachable / wire-made error / cut body
pub static BYTES: AtomicU64 = AtomicU64::new(0); // body bytes passed to CEF

pub fn counters() -> String {
    let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
    format!(
        "requests: http(s) seen {} (other schemes {}), handlers {}, sent to wire {}, answered {}, failed {}, body bytes {}",
        g(&SEEN_HTTP),
        g(&SEEN_OTHER),
        g(&OPENED),
        g(&SENT),
        g(&ANSWERED),
        g(&FAILED),
        g(&BYTES)
    )
}

fn is_http(url: &str) -> bool {
    let l = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

/// CEF ref-counted objects are thread safe; the Rust wrappers just are not marked so.
struct SendBox<T>(T);
unsafe impl<T> Send for SendBox<T> {}

/// One async `read` CEF is waiting for. `data` stays valid until `cb.cont` runs
/// (CEF's contract), unless the request was cancelled first.
struct ReadReq {
    data: SendBox<*mut u8>,
    len: usize,
    cb: SendBox<ResourceReadCallback>,
}

#[derive(Default)]
struct Inner {
    /// Ok: wire's head. Err: the network error to report instead.
    head: Option<Result<WireHead, Errorcode>>,
    /// The body length for CEF (`X-Wire-Upstream-Length`), None = until EOF.
    length: Option<u64>,
    /// Feeds the body thread; dropped with the handler, which ends that thread.
    reads: Option<mpsc::Sender<ReadReq>>,
}

#[derive(Default)]
struct Conn {
    /// A clone of the socket, so `cancel` can unblock a pending read.
    stream: Option<UnixStream>,
    cancelled: bool,
}

#[derive(Default)]
struct State {
    inner: Mutex<Inner>,
    /// Separate lock: the body thread holds it while it writes into CEF's buffer,
    /// so `cancel` cannot return in the middle of that.
    conn: Arc<Mutex<Conn>>,
    id: AtomicU64,
}

/// Copies what CEF gives us into a plain request: method, URL, all headers, body.
fn take_request(request: &mut Request) -> Result<WireRequest, String> {
    let mut map = CefStringMultimap::new();
    request.header_map(Some(&mut map));
    let mut headers: Vec<(String, String)> = map.into_iter().flat_map(|(n, vs)| vs.into_iter().map(move |v| (n.clone(), v))).collect();
    // CEF keeps the referrer out of the header map (it is already reduced by the
    // page's referrer policy); Sec-Fetch-* are never in it (the network service adds
    // them after interception, so they are not sent at all).
    let referrer = CefString::from(&request.referrer_url()).to_string();
    if !referrer.is_empty() && !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("referer")) {
        headers.push(("Referer".to_string(), referrer));
    }
    let mut body = Vec::new();
    let mut missing = 0;
    if let Some(pd) = request.post_data() {
        let mut elems = vec![None; pd.element_count()];
        pd.elements(Some(&mut elems));
        for e in elems.into_iter().flatten() {
            match sys::cef_postdataelement_type_t::from(e.get_type()) {
                sys::cef_postdataelement_type_t::PDE_TYPE_BYTES => {
                    let n = e.bytes_count();
                    let start = body.len();
                    body.resize(start + n, 0);
                    let got = e.bytes(n, body[start..].as_mut_ptr());
                    body.truncate(start + got);
                }
                sys::cef_postdataelement_type_t::PDE_TYPE_FILE => {
                    let path = CefString::from(&e.file()).to_string();
                    match std::fs::read(&path) {
                        Ok(b) => body.extend_from_slice(&b),
                        Err(err) => return Err(format!("cannot read upload file {path}: {err}")),
                    }
                }
                // A Blob/File/ReadableStream part: Chromium holds it as a data pipe,
                // which CefPostData cannot express; CEF hands us an EMPTY element
                // (and has_excluded_elements() still says 0).
                _ => missing += 1,
            }
        }
        if pd.has_excluded_elements() != 0 {
            missing += 1;
        }
    }
    if missing > 0 {
        // Sending the rest would upload a silently corrupted body (a file field
        // without its file). Fail the request instead.
        return Err(format!("{missing} upload part(s) are blobs/streams CEF does not expose"));
    }
    Ok(WireRequest {
        method: CefString::from(&request.method()).to_string(),
        url: CefString::from(&request.url()).to_string(),
        headers,
        body,
    })
}

/// The request thread: talk to wire, report the head, then serve body reads.
fn run_request(state: Arc<State>, socket: String, req: WireRequest, ready: SendBox<Callback>) {
    let id = state.id.load(Ordering::Relaxed);
    let t0 = Instant::now();
    let fail = |state: &State, code: Errorcode, why: String| {
        FAILED.fetch_add(1, Ordering::Relaxed);
        log!("wire #{id} {} {} -> {:?} ({why})", req.method, req.url, sys::cef_errorcode_t::from(code));
        if state.conn.lock().unwrap().cancelled {
            return; // CEF gave up on this request; no callback after cancel()
        }
        state.inner.lock().unwrap().head = Some(Err(code));
        ready.0.cont();
    };
    let mut stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(e) => return fail(&state, Errorcode::PROXY_CONNECTION_FAILED, format!("connect {socket}: {e}")),
    };
    {
        let mut conn = state.conn.lock().unwrap();
        if conn.cancelled {
            return;
        }
        conn.stream = stream.try_clone().ok();
    }
    if let Err(e) = stream.write_all(&wire::encode_request(&req)) {
        return fail(&state, Errorcode::PROXY_CONNECTION_FAILED, format!("send: {e}"));
    }
    SENT.fetch_add(1, Ordering::Relaxed);
    let mut reader = BufReader::with_capacity(64 * 1024, stream);
    let head = match wire::read_head(&mut reader) {
        Ok(h) => h,
        Err(e) => return fail(&state, Errorcode::EMPTY_RESPONSE, format!("head: {e}")),
    };
    if let Some(kind) = head.wire_error() {
        let kind = kind.to_string();
        let mut body = String::new();
        let _ = (&mut reader).take(64 * 1024).read_to_string(&mut body);
        let code = Errorcode::from(wire::error_for(&kind, &body));
        return fail(&state, code, format!("HTTP {} X-Wire-Error: {kind}: {}", head.status, body.trim()));
    }
    ANSWERED.fetch_add(1, Ordering::Relaxed);
    let expected = head.body_length(&req.method);
    log!(
        "wire #{id} {} {} -> {} {}{} ({} ms to head{})",
        req.method,
        req.url,
        head.status,
        head.reason,
        head.header("location").map(|l| format!(" -> {l}")).unwrap_or_default(),
        t0.elapsed().as_millis(),
        head.header("x-wire-cache").map(|c| format!(", cache {c}")).unwrap_or_default()
    );
    let (tx, rx) = mpsc::channel::<ReadReq>();
    {
        let mut inner = state.inner.lock().unwrap();
        inner.head = Some(Ok(head));
        inner.length = expected;
        inner.reads = Some(tx);
    }
    let conn = Arc::clone(&state.conn);
    drop(state); // the handler owns State; dropping it drops `tx` and ends the loop below
    ready.0.cont();

    let mut total: u64 = 0;
    let mut buf = Vec::new();
    while let Ok(r) = rx.recv() {
        buf.resize(r.len.min(256 * 1024), 0);
        let res = reader.read(&mut buf);
        let conn = conn.lock().unwrap();
        if conn.cancelled {
            return;
        }
        let n: i32 = match res {
            Ok(0) => match expected {
                Some(want) if total < want => {
                    FAILED.fetch_add(1, Ordering::Relaxed);
                    log!("wire #{id} {} body cut: {total} of {want} bytes", req.url);
                    sys::cef_errorcode_t::ERR_CONTENT_LENGTH_MISMATCH as i32
                }
                _ => 0,
            },
            Ok(n) => {
                // SAFETY: CEF keeps `data` (len bytes) valid until cont() and we hold
                // the conn lock, so cancel() has not run.
                unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), r.data.0, n) };
                total += n as u64;
                BYTES.fetch_add(n as u64, Ordering::Relaxed);
                n as i32
            }
            Err(e) => {
                FAILED.fetch_add(1, Ordering::Relaxed);
                log!("wire #{id} {} body read error after {total} bytes: {e}", req.url);
                sys::cef_errorcode_t::ERR_CONNECTION_RESET as i32
            }
        };
        r.cb.0.cont(n);
        if n <= 0 {
            return;
        }
    }
}

wrap_resource_handler! {
    pub struct WireResourceHandler {
        state: Arc<State>,
    }

    impl ResourceHandler {
        fn open(
            &self,
            request: Option<&mut Request>,
            handle_request: Option<&mut i32>,
            callback: Option<&mut Callback>,
        ) -> i32 {
            let (Some(request), Some(handle_request), Some(callback)) = (request, handle_request, callback) else {
                return 0;
            };
            let Some(Some(socket)) = WIRE_SOCKET.get() else { return 0 };
            let id = OPENED.fetch_add(1, Ordering::Relaxed) + 1;
            self.state.id.store(id, Ordering::Relaxed);
            let req = match take_request(request) {
                Ok(r) => r,
                Err(why) => {
                    FAILED.fetch_add(1, Ordering::Relaxed);
                    let url = CefString::from(&request.url()).to_string();
                    log!("wire #{id} {} {url} -> ERR_NOT_IMPLEMENTED (not sent: {why})", CefString::from(&request.method()));
                    self.state.inner.lock().unwrap().head = Some(Err(Errorcode::NOT_IMPLEMENTED));
                    *handle_request = 1;
                    return 1;
                }
            };
            if std::env::var_os("WEB_SPIKE_LOG_HEADERS").is_some() {
                let names: Vec<&str> = req.headers.iter().map(|(n, _)| n.as_str()).collect();
                log!("headers for {} {}: {names:?}, body {} bytes", req.method, req.url, req.body.len());
            }
            *handle_request = 0; // decide later: callback.cont() once wire's head is in
            let (state, socket, cb) = (Arc::clone(&self.state), socket.clone(), SendBox(callback.clone()));
            std::thread::Builder::new()
                .name(format!("wire-{id}"))
                .spawn(move || run_request(state, socket, req, cb))
                .is_ok() as i32
        }

        fn response_headers(
            &self,
            response: Option<&mut Response>,
            response_length: Option<&mut i64>,
            _redirect_url: Option<&mut CefString>,
        ) {
            let (Some(response), Some(len)) = (response, response_length) else { return };
            let inner = self.state.inner.lock().unwrap();
            match &inner.head {
                Some(Ok(head)) => {
                    response.set_status(head.status as i32);
                    response.set_status_text(Some(&CefString::from(head.reason.as_str())));
                    let (mime, charset) = head.mime_and_charset();
                    if !mime.is_empty() {
                        response.set_mime_type(Some(&CefString::from(mime.as_str())));
                    }
                    if !charset.is_empty() {
                        response.set_charset(Some(&CefString::from(charset.as_str())));
                    }
                    let mut map = CefStringMultimap::new();
                    for (n, v) in head.page_headers() {
                        map.append(n, v);
                    }
                    // A 3xx with Location: CEF follows it itself (a new request,
                    // through this handler again).
                    response.set_header_map(Some(&mut map));
                    *len = inner.length.map_or(-1, |n| n as i64);
                }
                Some(Err(code)) => {
                    response.set_error(*code);
                    *len = 0;
                }
                None => {
                    response.set_error(Errorcode::FAILED);
                    *len = 0;
                }
            }
        }

        fn read(
            &self,
            data_out: *mut u8,
            bytes_to_read: i32,
            bytes_read: Option<&mut i32>,
            callback: Option<&mut ResourceReadCallback>,
        ) -> i32 {
            let (Some(bytes_read), Some(callback)) = (bytes_read, callback) else { return 0 };
            *bytes_read = 0;
            let inner = self.state.inner.lock().unwrap();
            let Some(tx) = &inner.reads else { return 0 }; // no body (error answer): complete
            let req = ReadReq { data: SendBox(data_out), len: bytes_to_read.max(0) as usize, cb: SendBox(callback.clone()) };
            // Async: the request thread fills data_out and calls callback.cont(n).
            tx.send(req).is_ok() as i32
        }

        fn cancel(&self) {
            let mut conn = self.state.conn.lock().unwrap();
            conn.cancelled = true;
            if let Some(s) = conn.stream.take() {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            drop(conn);
            self.state.inner.lock().unwrap().reads = None;
        }
    }
}

wrap_resource_request_handler! {
    pub struct WireRequestHandler {}

    impl ResourceRequestHandler {
        // The cef crate's default is `ReturnValue::default()` = RV_CANCEL (CEF's own
        // C++ default is RV_CONTINUE): without this every request ends ERR_ABORTED.
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            ReturnValue::CONTINUE
        }

        fn resource_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
        ) -> Option<ResourceHandler> {
            Some(WireResourceHandler::new(Arc::new(State::default())))
        }

        fn on_resource_redirect(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _response: Option<&mut Response>,
            new_url: Option<&mut CefString>,
        ) {
            log!(
                "redirect (CEF follows): {} -> {}",
                request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default(),
                new_url.map(|u| u.to_string()).unwrap_or_default()
            );
        }
    }
}

wrap_request_handler! {
    pub struct SpikeRequestHandler {}

    impl RequestHandler {
        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            is_navigation: i32,
            is_download: i32,
            _request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut i32>,
        ) -> Option<ResourceRequestHandler> {
            let request = request?;
            let url = CefString::from(&request.url()).to_string();
            if !is_http(&url) {
                SEEN_OTHER.fetch_add(1, Ordering::Relaxed);
                log!("not http(s), left to CEF: {}", url.get(..80).unwrap_or(&url));
                return None;
            }
            SEEN_HTTP.fetch_add(1, Ordering::Relaxed);
            if is_download != 0 || std::env::var_os("WEB_SPIKE_LOG_REQUESTS").is_some() {
                log!(
                    "request {} {url} (navigation {is_navigation}, download {is_download}, type {:?})",
                    CefString::from(&request.method()),
                    sys::cef_resource_type_t::from(request.resource_type())
                );
            }
            WIRE_SOCKET.get().cloned().flatten()?;
            Some(WireRequestHandler::new())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn http_schemes_only() {
        assert!(super::is_http("https://a.test/"));
        assert!(super::is_http("HTTP://a.test/"));
        for u in ["data:text/html,x", "blob:https://a.test/1", "chrome://version", "devtools://x", "about:blank", "ws://a"] {
            assert!(!super::is_http(u), "{u}");
        }
    }
}
