//! HTTP/1.1 message heads: parsing (httparse), hop-by-hop / `Proxy-*` stripping, body framing, and a
//! pass-through chunked-encoding scanner that finds the end of a chunked body without decoding it.
//!
//! Only heads are buffered; bodies are streamed by the caller using [`Framing`] and [`ChunkScanner`].

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt};

/// Largest request or response head the proxy accepts.
pub const MAX_HEAD: usize = 64 * 1024;
const MAX_HEADERS: usize = 128;

pub type Headers = Vec<(String, Vec<u8>)>;

#[derive(Debug, Clone)]
pub struct RequestHead {
    pub method: String,
    pub target: String,
    pub headers: Headers,
    /// Minor version: 1 for HTTP/1.1, 0 for HTTP/1.0.
    pub version: u8,
}

impl RequestHead {
    /// Does the browser want the connection closed after this exchange (RFC 9112 9.3)?
    pub fn wants_close(&self) -> bool {
        let t = tokens(&self.headers, "connection");
        t.iter().any(|t| t == "close")
            || (self.version == 0 && !t.iter().any(|t| t == "keep-alive"))
    }
}

#[derive(Debug, Clone)]
pub struct ResponseHead {
    /// Minor version: 1 for HTTP/1.1, 0 for HTTP/1.0.
    pub version: u8,
    pub code: u16,
    pub reason: String,
    pub headers: Headers,
}

#[derive(Debug)]
pub enum HeadError {
    /// The peer closed before sending anything.
    Eof,
    /// The peer closed in the middle of a head.
    Truncated,
    TooLarge,
    Malformed(String),
    Io(io::Error),
}

impl std::fmt::Display for HeadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeadError::Eof => f.write_str("connection closed before a head"),
            HeadError::Truncated => f.write_str("connection closed inside a head"),
            HeadError::TooLarge => write!(f, "head larger than {MAX_HEAD} bytes"),
            HeadError::Malformed(m) => write!(f, "malformed head: {m}"),
            HeadError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Read from `r` into `buf` until `parse` finds a complete head; returns the head and leaves only the bytes
/// after it in `buf`.
async fn read_head<R, T>(
    r: &mut R,
    buf: &mut Vec<u8>,
    parse: impl Fn(&[u8]) -> Result<Option<(T, usize)>, String>,
) -> Result<T, HeadError>
where
    R: AsyncRead + Unpin,
{
    loop {
        if !buf.is_empty() {
            match parse(buf) {
                Ok(Some((head, len))) => {
                    buf.drain(..len);
                    return Ok(head);
                }
                Ok(None) => {}
                Err(m) => return Err(HeadError::Malformed(m)),
            }
        }
        if buf.len() >= MAX_HEAD {
            return Err(HeadError::TooLarge);
        }
        let old = buf.len();
        buf.resize(MAX_HEAD.min(old + 8192), 0);
        let n = match r.read(&mut buf[old..]).await {
            Ok(n) => n,
            Err(e) => {
                buf.truncate(old);
                return Err(HeadError::Io(e));
            }
        };
        buf.truncate(old + n);
        if n == 0 {
            return Err(if old == 0 {
                HeadError::Eof
            } else {
                HeadError::Truncated
            });
        }
    }
}

fn owned_headers(hs: &[httparse::Header]) -> Result<Headers, String> {
    hs.iter()
        .map(|h| {
            if h.value.iter().any(|&b| b == b'\r' || b == b'\n' || b == 0) {
                Err(format!("bad value for header {}", h.name))
            } else {
                Ok((h.name.to_string(), h.value.to_vec()))
            }
        })
        .collect()
}

pub fn parse_request(buf: &[u8]) -> Result<Option<(RequestHead, usize)>, String> {
    let mut hs = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut req = httparse::Request::new(&mut hs);
    match req.parse(buf) {
        Ok(httparse::Status::Complete(len)) => {
            if req.version != Some(1) && req.version != Some(0) {
                return Err("unsupported HTTP version".into());
            }
            let head = RequestHead {
                method: req.method.unwrap_or_default().to_string(),
                target: req.path.unwrap_or_default().to_string(),
                headers: owned_headers(req.headers)?,
                version: req.version.unwrap_or(1),
            };
            Ok(Some((head, len)))
        }
        Ok(httparse::Status::Partial) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn parse_response(buf: &[u8]) -> Result<Option<(ResponseHead, usize)>, String> {
    let mut hs = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut res = httparse::Response::new(&mut hs);
    match res.parse(buf) {
        Ok(httparse::Status::Complete(len)) => {
            let head = ResponseHead {
                version: res.version.unwrap_or(1),
                code: res.code.unwrap_or(0),
                reason: res.reason.unwrap_or("").to_string(),
                headers: owned_headers(res.headers)?,
            };
            Ok(Some((head, len)))
        }
        Ok(httparse::Status::Partial) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub async fn read_request<R: AsyncRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
) -> Result<RequestHead, HeadError> {
    read_head(r, buf, parse_request).await
}

pub async fn read_response<R: AsyncRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
) -> Result<ResponseHead, HeadError> {
    read_head(r, buf, parse_response).await
}

pub fn header<'a>(hs: &'a Headers, name: &str) -> Option<&'a [u8]> {
    hs.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_slice())
}

fn all_values<'a>(hs: &'a Headers, name: &'a str) -> impl Iterator<Item = &'a [u8]> + 'a {
    hs.iter()
        .filter(move |(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_slice())
}

/// Comma-separated tokens of every `name` header, trimmed and lowercased.
pub fn tokens(hs: &Headers, name: &str) -> Vec<String> {
    all_values(hs, name)
        .flat_map(|v| v.split(|&b| b == b','))
        .map(|t| String::from_utf8_lossy(t).trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Is this request asking to switch protocols (`Connection: upgrade` plus an `Upgrade` header)?
pub fn is_upgrade_request(hs: &Headers) -> bool {
    header(hs, "upgrade").is_some() && tokens(hs, "connection").iter().any(|t| t == "upgrade")
}

/// Remove hop-by-hop headers (SPEC: `Proxy-Connection`, `Proxy-Authorization`, `Connection` and the headers
/// it names, `Keep-Alive`, `TE`, `Trailer`) and every `Proxy-*` header (the proxy sets none of its own on
/// this path yet). `Upgrade` is kept only when `keep_upgrade` (the request is an upgrade), else removed.
pub fn strip_hop_by_hop(hs: &Headers, keep_upgrade: bool) -> Headers {
    let named = tokens(hs, "connection");
    hs.iter()
        .filter(|(n, _)| {
            let l = n.to_ascii_lowercase();
            if l == "upgrade" {
                return keep_upgrade;
            }
            !(l == "connection"
                || l == "keep-alive"
                || l == "te"
                || l == "trailer"
                || l.starts_with("proxy-")
                || named.contains(&l))
        })
        .cloned()
        .collect()
}

/// How a message body is delimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    None,
    Length(u64),
    Chunked,
    /// Response body delimited by the server closing the connection.
    UntilClose,
}

fn content_length(hs: &Headers) -> Result<Option<u64>, String> {
    let mut found: Option<u64> = None;
    for v in all_values(hs, "content-length") {
        for part in v.split(|&b| b == b',') {
            let s = std::str::from_utf8(part)
                .map_err(|_| "bad Content-Length")?
                .trim();
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return Err("bad Content-Length".into());
            }
            let n: u64 = s.parse().map_err(|_| "bad Content-Length")?;
            if found.is_some_and(|f| f != n) {
                return Err("conflicting Content-Length values".into());
            }
            found = Some(n);
        }
    }
    Ok(found)
}

/// Framing of a request body (RFC 9112 6.3). A non-chunked Transfer-Encoding on a request is an error.
pub fn request_framing(hs: &Headers) -> Result<Framing, String> {
    let te = tokens(hs, "transfer-encoding");
    if !te.is_empty() {
        return if te.last().map(String::as_str) == Some("chunked") {
            Ok(Framing::Chunked)
        } else {
            Err("request Transfer-Encoding is not chunked".into())
        };
    }
    Ok(match content_length(hs)? {
        Some(0) | None => Framing::None,
        Some(n) => Framing::Length(n),
    })
}

/// Framing of a response body to a request with method `method`.
pub fn response_framing(method: &str, code: u16, hs: &Headers) -> Result<Framing, String> {
    if method.eq_ignore_ascii_case("HEAD")
        || (100..200).contains(&code)
        || code == 204
        || code == 304
    {
        return Ok(Framing::None);
    }
    let te = tokens(hs, "transfer-encoding");
    if !te.is_empty() {
        return Ok(if te.last().map(String::as_str) == Some("chunked") {
            Framing::Chunked
        } else {
            Framing::UntilClose
        });
    }
    Ok(match content_length(hs)? {
        Some(0) => Framing::None,
        Some(n) => Framing::Length(n),
        None => Framing::UntilClose,
    })
}

/// With Transfer-Encoding present, a Content-Length must not be forwarded (RFC 9112 6.3, smuggling).
pub fn drop_length_if_chunked(hs: &mut Headers) {
    if header(hs, "transfer-encoding").is_some() {
        hs.retain(|(n, _)| !n.eq_ignore_ascii_case("content-length"));
    }
}

pub fn write_request_head(method: &str, target: &str, hs: &Headers) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(format!("{method} {target} HTTP/1.1\r\n").as_bytes());
    write_headers(&mut out, hs);
    out
}

pub fn write_response_head(code: u16, reason: &str, hs: &Headers) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(format!("HTTP/1.1 {code} {reason}\r\n").as_bytes());
    write_headers(&mut out, hs);
    out
}

fn write_headers(out: &mut Vec<u8>, hs: &Headers) {
    for (n, v) in hs {
        out.extend_from_slice(n.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(v);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
}

pub fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        421 => "Misdirected Request",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}

/// A complete error response the proxy answers itself, with a short plain-text body.
pub fn error_response(code: u16, body: &str) -> Vec<u8> {
    let body = format!("{body}\n");
    let mut out = format!(
        "HTTP/1.1 {code} {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reason(code),
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Finds where a chunked body ends while its bytes stream past unchanged (chunk extensions and trailers
/// included). Never buffers: it only keeps a little state between calls.
#[derive(Debug, Default)]
pub struct ChunkScanner {
    st: ChunkState,
    size: u64,
    digits: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ChunkState {
    #[default]
    Size,
    Ext,
    SizeLf,
    Data(u64),
    DataCr,
    DataLf,
    TrailerStart,
    TrailerLine,
    TrailerLf,
    EndLf,
    Done,
}

impl ChunkScanner {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn done(&self) -> bool {
        self.st == ChunkState::Done
    }

    /// Feed the next bytes. Returns `Some(n)` when the body ends after `data[..n]`, `None` if more is needed.
    pub fn feed(&mut self, data: &[u8]) -> Result<Option<usize>, String> {
        use ChunkState::*;
        let mut i = 0;
        while i < data.len() {
            let b = data[i];
            match self.st {
                Size => {
                    if let Some(v) = (b as char).to_digit(16) {
                        if self.size > (u64::MAX >> 4) {
                            return Err("chunk size overflow".into());
                        }
                        self.size = self.size << 4 | v as u64;
                        self.digits = true;
                    } else if self.digits && (b == b';' || b == b' ' || b == b'\t') {
                        self.st = Ext;
                    } else if self.digits && b == b'\r' {
                        self.st = SizeLf;
                    } else {
                        return Err("bad chunk size line".into());
                    }
                }
                Ext => {
                    if b == b'\r' {
                        self.st = SizeLf;
                    } else if b == b'\n' {
                        return Err("bare LF in chunk size line".into());
                    }
                }
                SizeLf => {
                    if b != b'\n' {
                        return Err("chunk size line without LF".into());
                    }
                    self.st = if self.size == 0 {
                        TrailerStart
                    } else {
                        Data(self.size)
                    };
                    self.size = 0;
                    self.digits = false;
                }
                Data(left) => {
                    let take = left.min((data.len() - i) as u64);
                    i += take as usize;
                    self.st = if take == left {
                        DataCr
                    } else {
                        Data(left - take)
                    };
                    continue;
                }
                DataCr => {
                    if b != b'\r' {
                        return Err("chunk data not followed by CRLF".into());
                    }
                    self.st = DataLf;
                }
                DataLf => {
                    if b != b'\n' {
                        return Err("chunk data not followed by CRLF".into());
                    }
                    self.st = Size;
                }
                TrailerStart => self.st = if b == b'\r' { EndLf } else { TrailerLine },
                TrailerLine => {
                    if b == b'\r' {
                        self.st = TrailerLf;
                    }
                }
                TrailerLf => {
                    if b != b'\n' {
                        return Err("trailer line without LF".into());
                    }
                    self.st = TrailerStart;
                }
                EndLf => {
                    if b != b'\n' {
                        return Err("chunked body without final CRLF".into());
                    }
                    self.st = Done;
                    return Ok(Some(i + 1));
                }
                Done => return Ok(Some(i)),
            }
            i += 1;
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hs(pairs: &[(&str, &str)]) -> Headers {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.as_bytes().to_vec()))
            .collect()
    }

    fn names(h: &Headers) -> Vec<String> {
        h.iter().map(|(n, _)| n.to_ascii_lowercase()).collect()
    }

    #[test]
    fn strips_hop_by_hop_and_proxy_headers() {
        let h = hs(&[
            ("Host", "a"),
            ("Connection", "keep-alive, X-Hop"),
            ("connection", "Other"),
            ("X-Hop", "1"),
            ("other", "1"),
            ("Keep-Alive", "timeout=5"),
            ("TE", "trailers"),
            ("Trailer", "X"),
            ("Proxy-Connection", "keep-alive"),
            ("Proxy-Authorization", "Basic x"),
            ("Proxy-Id", "forged"),
            ("Proxy-Flow", "forged/1"),
            ("proxy-anything", "x"),
            ("Upgrade", "websocket"),
            ("Transfer-Encoding", "chunked"),
            ("Content-Encoding", "gzip"),
            ("X-Keep", "1"),
        ]);
        assert_eq!(
            names(&strip_hop_by_hop(&h, false)),
            ["host", "transfer-encoding", "content-encoding", "x-keep"]
        );
        assert!(names(&strip_hop_by_hop(&h, true)).contains(&"upgrade".to_string()));
    }

    #[test]
    fn upgrade_detection() {
        assert!(is_upgrade_request(&hs(&[
            ("Connection", "keep-alive, Upgrade"),
            ("Upgrade", "websocket")
        ])));
        assert!(!is_upgrade_request(&hs(&[("Upgrade", "websocket")])));
    }

    #[test]
    fn framing() {
        assert_eq!(request_framing(&hs(&[])).unwrap(), Framing::None);
        assert_eq!(
            request_framing(&hs(&[("Content-Length", "10")])).unwrap(),
            Framing::Length(10)
        );
        assert_eq!(
            request_framing(&hs(&[("Content-Length", "10, 10")])).unwrap(),
            Framing::Length(10)
        );
        assert!(
            request_framing(&hs(&[("Content-Length", "10"), ("content-length", "11")])).is_err()
        );
        assert!(request_framing(&hs(&[("Content-Length", "-1")])).is_err());
        assert_eq!(
            request_framing(&hs(&[("Transfer-Encoding", "gzip, chunked")])).unwrap(),
            Framing::Chunked
        );
        assert!(request_framing(&hs(&[("Transfer-Encoding", "gzip")])).is_err());
        assert_eq!(
            response_framing("HEAD", 200, &hs(&[("Content-Length", "5")])).unwrap(),
            Framing::None
        );
        assert_eq!(
            response_framing("GET", 304, &hs(&[])).unwrap(),
            Framing::None
        );
        assert_eq!(
            response_framing("GET", 200, &hs(&[])).unwrap(),
            Framing::UntilClose
        );
        assert_eq!(
            response_framing("GET", 200, &hs(&[("Transfer-Encoding", "gzip")])).unwrap(),
            Framing::UntilClose
        );
        assert_eq!(
            response_framing(
                "GET",
                200,
                &hs(&[("Transfer-Encoding", "chunked"), ("Content-Length", "3")])
            )
            .unwrap(),
            Framing::Chunked
        );
    }

    #[test]
    fn chunk_scanner_whole_and_bytewise() {
        let body = b"5;ext=1\r\nhello\r\n0a\r\n0123456789\r\n0\r\nX-T: 1\r\n\r\nNEXT";
        let end = body.len() - 4;
        let mut s = ChunkScanner::new();
        assert_eq!(s.feed(body).unwrap(), Some(end));
        let mut s = ChunkScanner::new();
        for (i, b) in body.iter().enumerate() {
            match s.feed(std::slice::from_ref(b)).unwrap() {
                Some(1) => {
                    assert_eq!(i + 1, end);
                    break;
                }
                Some(n) => panic!("unexpected {n}"),
                None => assert!(i + 1 < end),
            }
        }
        assert!(s.done());
        let mut s = ChunkScanner::new();
        assert_eq!(s.feed(b"0\r\n\r\n").unwrap(), Some(5));
    }

    #[test]
    fn chunk_scanner_rejects_garbage() {
        for bad in [&b"x\r\n"[..], b"\r\n", b"5\r\nhelloX", b"5\nhello"] {
            assert!(
                ChunkScanner::new().feed(bad).is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
        assert!(ChunkScanner::new().feed(b"fffffffffffffffff\r\n").is_err());
    }

    #[test]
    fn parses_heads() {
        let (r, n) = parse_request(b"CONNECT [::1]:443 HTTP/1.1\r\nHost: [::1]:443\r\n\r\nTLS")
            .unwrap()
            .unwrap();
        assert_eq!(
            (r.method.as_str(), r.target.as_str(), n),
            ("CONNECT", "[::1]:443", 47)
        );
        assert!(
            parse_request(b"GET http://a/ HTTP/1.1\r\nHost: a\r\n")
                .unwrap()
                .is_none()
        );
        let (r, _) = parse_response(b"HTTP/1.1 302 Found\r\nLocation: /x\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(
            (r.code, header(&r.headers, "location")),
            (302, Some(&b"/x"[..]))
        );
    }

    #[test]
    fn error_response_is_complete() {
        let r = error_response(502, "upstream refused");
        let (h, n) = parse_response(&r).unwrap().unwrap();
        assert_eq!(h.code, 502);
        assert_eq!(&r[n..], b"upstream refused\n");
        assert_eq!(header(&h.headers, "connection"), Some(&b"close"[..]));
    }
}
