//! The wire protocol from the browser's side (W3), without CEF types except the
//! error codes: encode one request, parse wire's response head, map wire's own
//! failures (`X-Wire-Error`) to Chromium network errors. See wire/README.md.

use cef::sys::cef_errorcode_t as E;
use std::io::{self, BufRead};

/// `X-Wire-Timeout-Ms`: wire's default (20 s) covers the whole request including
/// the body, which would cut long downloads, media and long polls. One hour.
pub const TIMEOUT_MS: u64 = 3_600_000;

/// One request as CEF handed it to us.
#[derive(Debug, Clone, Default)]
pub struct WireRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Request headers never forwarded to wire:
/// - `host`, `content-length`, `connection`: wire/ureq set them for the upstream leg
///   (we send our own `Content-Length` for the body on the wire leg);
/// - `accept-encoding`: wire always strips `Content-Encoding` from the answer and
///   ureq only undoes gzip, so Chromium's `br, zstd` would come back as undecodable
///   bytes. Without it ureq asks for gzip itself and decodes it;
/// - `x-wire-*`: the control channel is ours, never a page's.
fn skip_request_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(n.as_str(), "host" | "content-length" | "connection" | "accept-encoding") || n.starts_with("x-wire-")
}

/// The bytes sent to wire's socket: absolute-form request line, headers, body.
pub fn encode_request(req: &WireRequest) -> Vec<u8> {
    let url = req.url.split('#').next().unwrap_or("");
    let mut out = format!("{} {} HTTP/1.1\r\n", req.method, url);
    for (name, value) in &req.headers {
        let bad = |s: &str| s.bytes().any(|b| b == b'\r' || b == b'\n');
        if skip_request_header(name) || bad(name) || bad(value) || name.contains(':') {
            continue;
        }
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    // Redirects are answered to Chromium as the 3xx (it follows them itself: right
    // URL, origin, cookies and CORS checks per hop). See web/spike/README.md W3.
    out.push_str("X-Wire-Max-Redirects: 0\r\n");
    out.push_str(&format!("X-Wire-Timeout-Ms: {TIMEOUT_MS}\r\n"));
    if !req.body.is_empty() || matches!(req.method.as_str(), "POST" | "PUT" | "PATCH") {
        out.push_str(&format!("Content-Length: {}\r\n", req.body.len()));
    }
    out.push_str("\r\n");
    let mut out = out.into_bytes();
    out.extend_from_slice(&req.body);
    out
}

/// wire's response head.
#[derive(Debug, Clone, PartialEq)]
pub struct WireHead {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
}

impl WireHead {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// `X-Wire-Error` (wire made this answer itself: the upstream failed or a cap hit).
    pub fn wire_error(&self) -> Option<&str> {
        self.header("x-wire-error")
    }

    /// The headers the page sees: everything but wire's control channel.
    pub fn page_headers(&self) -> impl Iterator<Item = &(String, String)> {
        self.headers.iter().filter(|(n, _)| !n.to_ascii_lowercase().starts_with("x-wire-"))
    }

    /// `Content-Type` split into mime type and charset (CEF wants both separately).
    pub fn mime_and_charset(&self) -> (String, String) {
        let Some(ct) = self.header("content-type") else { return (String::new(), String::new()) };
        let mut parts = ct.split(';');
        let mime = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let charset = parts
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| k.trim().eq_ignore_ascii_case("charset"))
            .map(|(_, v)| v.trim().trim_matches('"').to_string())
            .unwrap_or_default();
        (mime, charset)
    }

    /// The body length wire will send, when it is known: `X-Wire-Upstream-Length`
    /// (W-MEDIA2). None means "read to EOF" (CEF: -1). No body for HEAD, 1xx, 204, 304.
    pub fn body_length(&self, method: &str) -> Option<u64> {
        if method.eq_ignore_ascii_case("HEAD") || self.status < 200 || matches!(self.status, 204 | 304) {
            return Some(0);
        }
        self.header("x-wire-upstream-length").and_then(|v| v.trim().parse().ok())
    }
}

/// Reads wire's status line and headers (up to the blank line).
pub fn read_head(r: &mut impl BufRead) -> io::Result<WireHead> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "wire closed without a response"));
    }
    let line = line.trim_end_matches(['\r', '\n']);
    let mut parts = line.splitn(3, ' ');
    let (Some(version), Some(code)) = (parts.next(), parts.next()) else { return Err(bad("bad status line")) };
    if !version.starts_with("HTTP/") {
        return Err(bad("bad status line"));
    }
    let status: u16 = code.parse().map_err(|_| bad("bad status code"))?;
    let reason = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 {
            return Err(bad("eof in headers"));
        }
        let h = h.trim_end_matches(['\r', '\n']);
        if h.is_empty() {
            break;
        }
        let (n, v) = h.split_once(':').ok_or_else(|| bad("bad header line"))?;
        headers.push((n.trim().to_string(), v.trim().to_string()));
    }
    Ok(WireHead { status, reason, headers })
}

/// Chromium's network error for a wire-made answer (`X-Wire-Error: <kind>`, the
/// small text body wire wrote). Never a fake 200: the page sees what a direct
/// load would have shown.
pub fn error_for(kind: &str, body: &str) -> E {
    match kind {
        "policy" => E::ERR_BLOCKED_BY_CLIENT,
        "truncated" => E::ERR_CONTENT_LENGTH_MISMATCH,
        "too-large" => E::ERR_FILE_TOO_BIG,
        "only-if-cached" => E::ERR_CACHE_MISS,
        "bad-user-agent" => E::ERR_INVALID_ARGUMENT,
        // "1": the upstream request failed; the body is ureq's error text.
        _ => {
            let b = body.to_ascii_lowercase();
            if b.contains("dns failed") || b.contains("failed to lookup address") {
                E::ERR_NAME_NOT_RESOLVED
            } else if b.contains("connection refused") {
                E::ERR_CONNECTION_REFUSED
            } else if b.contains("timed out") || b.contains("timeout") {
                E::ERR_TIMED_OUT
            } else if b.contains("unknownissuer") {
                E::ERR_CERT_AUTHORITY_INVALID
            } else if b.contains("notvalidforname") {
                E::ERR_CERT_COMMON_NAME_INVALID
            } else if b.contains("expired") || b.contains("notvalidyet") {
                E::ERR_CERT_DATE_INVALID
            } else if b.contains("certificate") {
                E::ERR_CERT_INVALID
            } else if b.contains("tls") || b.contains("handshake") {
                E::ERR_SSL_PROTOCOL_ERROR
            } else if b.contains("connection reset") || b.contains("broken pipe") {
                E::ERR_CONNECTION_RESET
            } else if b.contains("too many redirects") {
                E::ERR_TOO_MANY_REDIRECTS
            } else {
                E::ERR_CONNECTION_FAILED
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn req(method: &str, headers: &[(&str, &str)], body: &[u8]) -> String {
        let r = WireRequest {
            method: method.into(),
            url: "https://a.test/p?q=1#frag".into(),
            headers: headers.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect(),
            body: body.to_vec(),
        };
        String::from_utf8(encode_request(&r)).unwrap()
    }

    #[test]
    fn request_line_headers_and_body() {
        let s = req(
            "POST",
            &[
                ("Accept", "*/*"),
                ("Accept-Encoding", "gzip, br, zstd"),
                ("Host", "a.test"),
                ("X-Wire-Jar", "scratchy"),
                ("Cookie", "a=1"),
                ("Evil", "x\r\nInjected: 1"),
            ],
            b"k=v",
        );
        assert!(s.starts_with("POST https://a.test/p?q=1 HTTP/1.1\r\n"), "{s}");
        assert!(s.contains("Accept: */*\r\n") && s.contains("Cookie: a=1\r\n"));
        for gone in ["Accept-Encoding", "Host:", "X-Wire-Jar", "Injected", "Evil"] {
            assert!(!s.contains(gone), "{gone} in {s}");
        }
        assert!(s.contains("X-Wire-Max-Redirects: 0\r\n"));
        assert!(s.contains(&format!("X-Wire-Timeout-Ms: {TIMEOUT_MS}\r\n")));
        assert!(s.ends_with("Content-Length: 3\r\n\r\nk=v"), "{s}");
        // GET without a body: no Content-Length; an empty POST says 0.
        assert!(!req("GET", &[], b"").contains("Content-Length"));
        assert!(req("POST", &[], b"").ends_with("Content-Length: 0\r\n\r\n"));
    }

    #[test]
    fn head_parsing_and_page_headers() {
        let raw = "HTTP/1.1 302 Found\r\nX-Wire-Version: 4\r\nLocation: https://b.test/\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nContent-Type: text/html; Charset=\"UTF-8\"\r\nX-Wire-Upstream-Length: 12\r\n\r\nbody";
        let mut c = Cursor::new(raw.as_bytes());
        let h = read_head(&mut c).unwrap();
        assert_eq!((h.status, h.reason.as_str()), (302, "Found"));
        let page: Vec<_> = h.page_headers().map(|(n, _)| n.as_str()).collect();
        assert_eq!(page, ["Location", "Set-Cookie", "Set-Cookie", "Content-Type"]);
        assert_eq!(h.mime_and_charset(), ("text/html".to_string(), "UTF-8".to_string()));
        assert_eq!(h.body_length("GET"), Some(12));
        assert_eq!(h.body_length("HEAD"), Some(0));
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut c, &mut rest).unwrap();
        assert_eq!(rest, "body", "the body is left in the reader");
        assert!(read_head(&mut Cursor::new(&b""[..])).is_err());
        assert!(read_head(&mut Cursor::new(&b"garbage\r\n\r\n"[..])).is_err());
    }

    #[test]
    fn length_unknown_without_upstream_length() {
        let h = WireHead { status: 200, reason: "OK".into(), headers: vec![] };
        assert_eq!(h.body_length("GET"), None);
        let h = WireHead { status: 304, ..h };
        assert_eq!(h.body_length("GET"), Some(0));
    }

    #[test]
    fn wire_errors_map_to_network_errors() {
        let f = |b: &str| error_for("1", &format!("wire: upstream request failed: {b}"));
        assert_eq!(f("Dns Failed: resolve dns name 'nx.invalid:443': failed to lookup address information"), E::ERR_NAME_NOT_RESOLVED);
        assert_eq!(f("Connection Failed: Connect error: Connection refused (os error 111)"), E::ERR_CONNECTION_REFUSED);
        assert_eq!(f("Network Error: timed out reading response"), E::ERR_TIMED_OUT);
        assert_eq!(f("Connection Failed: tls connection init failed: invalid peer certificate: UnknownIssuer"), E::ERR_CERT_AUTHORITY_INVALID);
        assert_eq!(f("something else"), E::ERR_CONNECTION_FAILED);
        assert_eq!(error_for("policy", ""), E::ERR_BLOCKED_BY_CLIENT);
        assert_eq!(error_for("truncated", ""), E::ERR_CONTENT_LENGTH_MISMATCH);
    }
}
