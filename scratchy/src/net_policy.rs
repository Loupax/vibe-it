//! QF5: the request policy for page-JS networking (fetch/XHR, QF7/QF8).
//!
//! PURE functions only: no I/O, no JS exposure. Everything that the
//! fetch/XHR design memo left open (QF-D3, D5, D6, D8, D9) is a plain value
//! in `RequestPolicy`, so changing a decision later is a one-line change of
//! the default here, not a refactor.
//!
//! Three parts:
//! 1. `is_public_ip` + `RequestPolicy::check_request` (SSRF/mixed content/
//!    ports/methods). `is_public_ip` is mirrored by hand in `wire` (QF6):
//!    two crates, no shared code: keep the two lists identical.
//! 2. CORS decision helpers (browser rules, QF-D1).
//! 3. Forbidden header lists, kept identical to FORBIDDEN_REQ/forbiddenRes
//!    in src/fetch_prelude.js (a test compares them).
use super::*;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

/// A refused request. `Display` is the text for the QJ2-style log line and
/// the JS `TypeError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PolicyError {
    Scheme(String),
    FileNotAllowed,
    UrlTooLong(usize),
    MethodForbidden(String),
    BodyNotAllowed(String),
    MixedContent,
    PrivateHost(String),
    BadPort(u16),
    Cors(String),
    Redirects(u32),
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyError::Scheme(s) => write!(f, "scheme {s}: is not allowed from page JS (only http and https)"),
            PolicyError::FileNotAllowed => write!(f, "file: URLs are not allowed from page JS"),
            PolicyError::UrlTooLong(n) => write!(f, "URL is too long ({n} bytes)"),
            PolicyError::MethodForbidden(m) => write!(f, "method {m} is not allowed"),
            PolicyError::BodyNotAllowed(m) => write!(f, "a {m} request cannot have a body"),
            PolicyError::MixedContent => write!(f, "mixed content: an https page cannot request an http URL"),
            PolicyError::PrivateHost(h) => write!(f, "blocked: {h} is a private, loopback or link-local address"),
            PolicyError::BadPort(p) => write!(f, "blocked: port {p} is not allowed"),
            PolicyError::Cors(m) => write!(f, "CORS: {m}"),
            PolicyError::Redirects(n) => write!(f, "too many redirects (limit {n})"),
        }
    }
}

/// The browser's blocked ports (Fetch spec "bad ports").
pub(crate) const BAD_PORTS: &[u16] = &[
    1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101, 102, 103, 104, 109, 110, 111, 113, 115,
    117, 119, 123, 135, 137, 139, 143, 161, 179, 389, 427, 465, 512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587,
    601, 636, 989, 990, 993, 995, 1719, 1720, 1723, 2049, 3659, 4045, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668, 6669, 6697, 10080,
];

/// Request headers page JS may never set. MUST stay equal to FORBIDDEN_REQ
/// in src/fetch_prelude.js (plus the `proxy-`/`sec-` prefixes).
pub(crate) const FORBIDDEN_REQUEST_HEADERS: &[&str] = &[
    "accept-charset", "accept-encoding", "access-control-request-headers", "access-control-request-method", "connection",
    "content-length", "cookie", "cookie2", "date", "dnt", "expect", "host", "keep-alive", "origin", "referer", "set-cookie", "te",
    "trailer", "transfer-encoding", "upgrade", "via",
];

pub(crate) fn is_forbidden_request_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    FORBIDDEN_REQUEST_HEADERS.contains(&n.as_str()) || n.starts_with("proxy-") || n.starts_with("sec-")
}

/// Response headers hidden from page JS.
pub(crate) fn is_forbidden_response_header(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "set-cookie" | "set-cookie2")
}

/// Per-request caps scratchy asks wire to enforce (QF6 implements them; the
/// codec already carries them, `wire_cap_headers`). Wire ignores unknown
/// headers today, so sending them is harmless against an old wire.
pub(crate) const WIRE_HEADER_POLICY: &str = "X-Wire-Policy"; // value: public-only
pub(crate) const WIRE_HEADER_MAX_BYTES: &str = "X-Wire-Max-Bytes";
pub(crate) const WIRE_HEADER_TIMEOUT_MS: &str = "X-Wire-Timeout-Ms";
pub(crate) const WIRE_HEADER_MAX_REDIRECTS: &str = "X-Wire-Max-Redirects";

impl RequestPolicy {
    /// The `X-Wire-*` request headers carrying this policy's caps. Append
    /// them to the header list passed to `build_wire_request`. `timeout_ms`
    /// overrides the default request timeout (sync XHR uses a shorter one).
    pub(crate) fn wire_cap_headers(&self, timeout_ms: Option<u64>) -> Vec<(String, String)> {
        vec![
            (WIRE_HEADER_POLICY.to_string(), "public-only".to_string()),
            (WIRE_HEADER_MAX_BYTES.to_string(), self.max_response_bytes.to_string()),
            (WIRE_HEADER_TIMEOUT_MS.to_string(), timeout_ms.unwrap_or(self.read_timeout.as_millis() as u64).to_string()),
            (WIRE_HEADER_MAX_REDIRECTS.to_string(), self.redirect_limit.to_string()),
        ]
    }
}

/// `X-Wire-*` is the scratchy<->wire control channel (QF6): page JS must
/// never be able to set one (e.g. drop `X-Wire-Policy: public-only`).
pub(crate) fn is_wire_reserved_header(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("x-wire-")
}

/// True when `ip` is a globally routable unicast address. Mirror of the
/// list QF6 puts in wire: keep both identical.
pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
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
        || o[0] == 0                                   // 0.0.0.0/8 "this network"
        || (o[0] == 100 && (o[1] & 0xc0) == 64)        // 100.64.0.0/10 CGNAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)     // 192.0.0.0/24 IETF protocol
        || (o[0] == 198 && (o[1] & 0xfe) == 18)        // 198.18.0.0/15 benchmarking
        || o[0] >= 240) // 240.0.0.0/4 reserved
}

fn is_public_v6(a: Ipv6Addr) -> bool {
    let s = a.segments();
    // ::ffff:a.b.c.d (IPv4-mapped) and ::a.b.c.d (IPv4-compatible): judge by the v4 part.
    if let Some(v4) = a.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    // ::/96 covers unspecified, loopback and the deprecated IPv4-compatible form.
    if s[..6].iter().all(|x| *x == 0) {
        return false;
    }
    // 64:ff9b::/96 NAT64: judge by the embedded v4.
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6].iter().all(|x| *x == 0) {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_public_v4(v4);
    }
    !(a.is_multicast()
        || (s[0] & 0xffc0) == 0xfe80                    // fe80::/10 link-local
        || (s[0] & 0xfe00) == 0xfc00                    // fc00::/7 ULA
        || (s[0] == 0x2001 && s[1] == 0x0db8))          // 2001:db8::/32 documentation
}

/// What to do with a request to a sensitive class of target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrivatePolicy {
    Block,
    Warn,
    Allow,
}

/// A request that passed the policy. `warning` is the target class
/// (`loopback`, `private`, `link-local`, `cgnat`, `ula`, `multicast`,
/// `unspecified`, `reserved`, `local-name`, `file`) when the policy is `Warn`
/// for it; the caller logs `format_net_warning(..)`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PolicyOutcome {
    pub warning: Option<&'static str>,
}

/// The one-line warning for an allowed sensitive request.
pub(crate) fn format_net_warning(page: &Url, target: &Url, class: &str) -> String {
    format!("scratchy: net warning: {page} -> {target} ({class}) allowed by policy")
}

/// Class of a non-public IP address.
fn ip_class(ip: IpAddr) -> Option<&'static str> {
    if is_public_ip(ip) {
        return None;
    }
    let v4 = match ip {
        IpAddr::V4(a) => Some(a),
        IpAddr::V6(a) => a.to_ipv4_mapped(),
    };
    if let Some(a) = v4 {
        let o = a.octets();
        return Some(if a.is_loopback() {
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
        });
    }
    let IpAddr::V6(a) = ip else { return None };
    let s = a.segments();
    Some(if a.is_loopback() {
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
    })
}

/// Class of a sensitive (non-public) http(s) target, or `None` for a public one.
pub(crate) fn private_target_class(url: &Url, name_suffixes: &[String]) -> Option<&'static str> {
    match url.host() {
        None => Some("reserved"),
        Some(url::Host::Domain(d)) => {
            let h = d.trim_end_matches('.').to_ascii_lowercase();
            if h == "localhost" || h.ends_with(".localhost") {
                Some("loopback")
            } else if private_name(&h, name_suffixes) {
                Some("local-name")
            } else {
                None
            }
        }
        Some(url::Host::Ipv4(a)) => ip_class(IpAddr::V4(a)),
        Some(url::Host::Ipv6(a)) => ip_class(IpAddr::V6(a)),
    }
}

fn is_loopback_class(url: &Url) -> bool {
    private_target_class(url, &[]) == Some("loopback")
}

/// All the knobs. `Default` is the memo's most restrictive working default;
/// the open decisions (D3 analytics, D5 targets, D6 file pages, D8 sync XHR,
/// D9 limits) are fields so the user's answers only change values.
#[derive(Debug, Clone)]
pub(crate) struct RequestPolicy {
    /// QF-D5 (user decision): loopback/localhost/private/link-local/CGNAT/ULA
    /// targets. Default `Warn`: allowed, one warning line each. A same-origin
    /// request is never warned about.
    pub private_targets: PrivatePolicy,
    /// QF-D5/D6 (user decision): `file:` targets (from any page, incl. file:
    /// pages). Default `Warn`.
    pub file_targets: PrivatePolicy,
    /// Host name suffixes that are private by name (`localhost` itself too).
    pub private_name_suffixes: Vec<String>,
    pub block_bad_ports: bool,
    pub block_mixed_content: bool,
    pub max_url_len: usize,
    /// Never allowed from page JS.
    pub forbidden_methods: Vec<String>,
    /// QF-D9 limits (consumed by the codec and QF7).
    pub max_response_bytes: usize,
    pub max_in_flight_bytes: usize,
    pub max_concurrent_requests: usize,
    pub max_requests_per_page: usize,
    pub connect_timeout: Duration,
    /// Total per-request timeout (send + read the whole response): 30 s.
    pub read_timeout: Duration,
    /// Idle read timeout for the legacy `fetch_raw_via_wire` callers: 60 s.
    pub legacy_idle_timeout: Duration,
    /// CORS preflight cache lifetime cap (Chrome: 2 h).
    pub preflight_max_age_cap_secs: u64,
    /// Redirects wire may follow (ureq default is 5). Sent as `X-Wire-Max-Redirects` (QF6).
    pub redirect_limit: u32,
    /// QF-D8: sync XHR blocks the main thread, but at most this long.
    pub sync_xhr_timeout_ms: u64,
    /// QF14b: `document.cookie` talks to wire's control channel SYNCHRONOUSLY
    /// (the engine thread blocks): at most this long per call, 2 s. A
    /// failure latches `document.cookie` to `''` for the rest of the page.
    pub cookie_control_timeout_ms: u64,
    /// QF-D3: when false (default) beacons / keepalive / no-cors analytics
    /// are LOGGED IN DETAIL (`format_beacon_log`) and NOT emitted.
    pub beacons_send: bool,
    /// QF10: `sendBeacon` returns false when the bytes queued (not yet
    /// finished) plus the new body exceed this (spec: 64 KiB).
    pub beacon_queue_bytes: usize,
}

impl Default for RequestPolicy {
    fn default() -> Self {
        RequestPolicy {
            private_targets: PrivatePolicy::Warn,
            file_targets: PrivatePolicy::Warn,
            private_name_suffixes: ["localhost", "local", "internal"].iter().map(|s| s.to_string()).collect(),
            block_bad_ports: true,
            block_mixed_content: true,
            max_url_len: 32 * 1024,
            forbidden_methods: ["CONNECT", "TRACE", "TRACK"].iter().map(|s| s.to_string()).collect(),
            max_response_bytes: 16 << 20,
            max_in_flight_bytes: 64 << 20,
            max_concurrent_requests: 6,
            max_requests_per_page: 1000,
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(30),
            legacy_idle_timeout: Duration::from_secs(60),
            preflight_max_age_cap_secs: 7200,
            redirect_limit: 5,
            sync_xhr_timeout_ms: 5000,
            cookie_control_timeout_ms: 2000,
            beacons_send: false,
            beacon_queue_bytes: 64 * 1024,
        }
    }
}

/// Methods the Fetch spec byte-case-normalises.
pub(crate) fn normalize_method(m: &str) -> String {
    let u = m.to_ascii_uppercase();
    if matches!(u.as_str(), "GET" | "HEAD" | "POST" | "PUT" | "DELETE" | "OPTIONS") { u } else { m.to_string() }
}

fn is_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// True when `host` names a private target by name alone.
fn private_name(host: &str, suffixes: &[String]) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    suffixes.iter().any(|s| h == *s || h.ends_with(&format!(".{s}")))
}

impl RequestPolicy {
    /// Decide whether page JS at `page_url` may send `method` to
    /// `request_url` (with or without a body).
    pub(crate) fn check_request(&self, page_url: &Url, request_url: &Url, method: &str, has_body: bool) -> Result<PolicyOutcome, PolicyError> {
        let method = normalize_method(method);
        if !is_token(&method) || self.forbidden_methods.iter().any(|m| m.eq_ignore_ascii_case(&method)) {
            return Err(PolicyError::MethodForbidden(method));
        }
        if has_body && matches!(method.as_str(), "GET" | "HEAD") {
            return Err(PolicyError::BodyNotAllowed(method));
        }
        if request_url.as_str().len() > self.max_url_len {
            return Err(PolicyError::UrlTooLong(request_url.as_str().len()));
        }
        match request_url.scheme() {
            "http" | "https" => {}
            "file" => {
                return match self.file_targets {
                    PrivatePolicy::Block => Err(PolicyError::FileNotAllowed),
                    PrivatePolicy::Warn => Ok(PolicyOutcome { warning: Some("file") }),
                    PrivatePolicy::Allow => Ok(PolicyOutcome::default()),
                };
            }
            other => return Err(PolicyError::Scheme(other.to_string())),
        }
        // https page -> http blocked, except loopback (potentially trustworthy).
        if self.block_mixed_content && page_url.scheme() == "https" && request_url.scheme() == "http" && !is_loopback_class(request_url) {
            return Err(PolicyError::MixedContent);
        }
        if self.block_bad_ports {
            if let Some(p) = request_url.port_or_known_default() {
                if BAD_PORTS.contains(&p) {
                    return Err(PolicyError::BadPort(p));
                }
            }
        }
        // A same-origin request never warns or blocks (a dev page may fetch
        // itself). The bad-port check above still applies.
        if !is_same_origin(page_url, request_url) {
            if let Some(class) = private_target_class(request_url, &self.private_name_suffixes) {
                return match self.private_targets {
                    PrivatePolicy::Block => Err(PolicyError::PrivateHost(request_url.host_str().unwrap_or("").to_string())),
                    PrivatePolicy::Warn => Ok(PolicyOutcome { warning: Some(class) }),
                    PrivatePolicy::Allow => Ok(PolicyOutcome::default()),
                };
            }
        }
        Ok(PolicyOutcome::default())
    }

    /// Re-run the policy for one redirect hop (wire follows redirects, so QF6
    /// enforces this server-side too; scratchy uses it for hops it can see).
    /// `hop` is 1 for the first redirect. Returns the method and body-ness
    /// the redirected request uses: 303, and 301/302 for POST, become GET
    /// without a body (Fetch spec).
    pub(crate) fn check_redirect_hop(
        &self,
        page_url: &Url,
        to: &Url,
        status: u16,
        method: &str,
        has_body: bool,
        hop: u32,
    ) -> Result<(String, bool, PolicyOutcome), PolicyError> {
        if hop > self.redirect_limit {
            return Err(PolicyError::Redirects(self.redirect_limit));
        }
        let m = normalize_method(method);
        let (m, has_body) = if status == 303 && m != "HEAD" || (matches!(status, 301 | 302) && m == "POST") { ("GET".to_string(), false) } else { (m, has_body) };
        let outcome = self.check_request(page_url, to, &m, has_body)?;
        Ok((m, has_body, outcome))
    }
}

/// Same scheme, host and port. Opaque origins (file:) are never same-origin
/// with anything, except that two identical file: URLs ... are NOT either
/// (browsers treat file: as opaque): so file pages are always cross-origin.
pub(crate) fn is_same_origin(a: &Url, b: &Url) -> bool {
    let (oa, ob) = (a.origin(), b.origin());
    oa.is_tuple() && oa == ob
}

// ------------------------------------------------------------------ beacons

/// Which page API issued a request (for beacon classification and logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NetApi {
    SendBeacon,
    Fetch,
    Xhr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BeaconKind {
    /// `navigator.sendBeacon`.
    SendBeacon,
    /// `fetch(..., {keepalive: true})`.
    Keepalive,
    /// Cross-origin `no-cors` fetch: its response is opaque, so the page
    /// cannot use the result (analytics/pixel style).
    NoCorsOpaque,
}

/// Pure classification: is this request a beacon (fire and forget, result
/// unreadable) that `beacons_send == false` suppresses? XHR and ordinary
/// reads are never beacons.
pub(crate) fn classify_beacon(api: NetApi, keepalive: bool, kind: Option<FetchKind>) -> Option<BeaconKind> {
    match api {
        NetApi::SendBeacon => Some(BeaconKind::SendBeacon),
        NetApi::Fetch if keepalive => Some(BeaconKind::Keepalive),
        NetApi::Fetch if kind == Some(FetchKind::Opaque) => Some(BeaconKind::NoCorsOpaque),
        _ => None,
    }
}

impl RequestPolicy {
    pub(crate) fn suppress_beacon(&self, beacon: Option<BeaconKind>) -> bool {
        beacon.is_some() && !self.beacons_send
    }
}

/// The detailed log line for a suppressed beacon: API, method, absolute URL,
/// content type, body size, first 200 bytes of a text body, page URL.
pub(crate) fn format_beacon_log(
    beacon: BeaconKind,
    method: &str,
    url: &Url,
    content_type: Option<&str>,
    body: &[u8],
    page_url: &Url,
    sent: bool,
) -> String {
    let api = match beacon {
        BeaconKind::SendBeacon => "navigator.sendBeacon",
        BeaconKind::Keepalive => "fetch keepalive",
        BeaconKind::NoCorsOpaque => "fetch no-cors",
    };
    let ct = content_type.unwrap_or("(none)");
    let is_text = content_type.is_none_or(|c| {
        let c = c.to_ascii_lowercase();
        c.starts_with("text/") || c.contains("json") || c.contains("xml") || c.contains("x-www-form-urlencoded")
    });
    let preview = if body.is_empty() || !is_text {
        String::new()
    } else {
        let cut = &body[..body.len().min(200)];
        format!(" body[..200]={:?}", String::from_utf8_lossy(cut))
    };
    let status = if sent { "sent" } else { "suppressed (not sent)" };
    format!("beacon {api} {method} {url} content-type={ct} body={} bytes{preview} page={page_url}: {status}", body.len())
}

// ------------------------------------------------------------------ CORS

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestMode {
    SameOrigin,
    Cors,
    NoCors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialsMode {
    Omit,
    SameOrigin,
    Include,
}

/// What a fetch of `url` from `page` with `mode` turns into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FetchKind {
    /// Same origin: a `basic` response, always readable.
    Basic,
    /// Cross-origin `cors`: send, then check ACAO; `preflight` says whether
    /// an OPTIONS preflight is needed first.
    Cors { preflight: bool },
    /// Cross-origin `no-cors`: an opaque response, nothing readable.
    Opaque,
}

/// `headers` are the (non-forbidden) request headers the page set.
pub(crate) fn classify_fetch(
    mode: RequestMode,
    page: &Url,
    url: &Url,
    method: &str,
    headers: &[(String, String)],
) -> Result<FetchKind, PolicyError> {
    if is_same_origin(page, url) {
        return Ok(FetchKind::Basic);
    }
    match mode {
        RequestMode::SameOrigin => Err(PolicyError::Cors("a same-origin mode request targets another origin".into())),
        RequestMode::NoCors => {
            let m = normalize_method(method);
            if matches!(m.as_str(), "GET" | "HEAD" | "POST") {
                Ok(FetchKind::Opaque)
            } else {
                Err(PolicyError::Cors(format!("no-cors requests cannot use {m}")))
            }
        }
        RequestMode::Cors => Ok(FetchKind::Cors { preflight: needs_preflight(method, headers) }),
    }
}

pub(crate) fn is_cors_safelisted_method(m: &str) -> bool {
    matches!(normalize_method(m).as_str(), "GET" | "HEAD" | "POST")
}

/// Content-Type essences that do not force a preflight.
fn safelisted_content_type(v: &str) -> bool {
    let essence = v.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    matches!(essence.as_str(), "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain")
}

fn is_cors_safelisted_header(name: &str, value: &str) -> bool {
    if value.len() > 128 {
        return false;
    }
    match name.to_ascii_lowercase().as_str() {
        "accept" | "accept-language" | "content-language" => !value.bytes().any(|b| (b < 0x20 && b != b'\t') || b == 0x7f || b"\"():<>?@[\\]{}".contains(&b)),
        "content-type" => safelisted_content_type(value),
        _ => false,
    }
}

/// Lowercased, sorted, deduplicated names of request headers that are not
/// CORS-safelisted (the value of `Access-Control-Request-Headers`).
pub(crate) fn cors_unsafe_request_headers(headers: &[(String, String)]) -> Vec<String> {
    let mut v: Vec<String> =
        headers.iter().filter(|(n, val)| !is_cors_safelisted_header(n, val)).map(|(n, _)| n.to_ascii_lowercase()).collect();
    v.sort();
    v.dedup();
    v
}

pub(crate) fn needs_preflight(method: &str, headers: &[(String, String)]) -> bool {
    !is_cors_safelisted_method(method) || !cors_unsafe_request_headers(headers).is_empty()
}

fn header_values<'a>(headers: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).collect()
}

fn list_items(headers: &[(String, String)], name: &str) -> Vec<String> {
    header_values(headers, name)
        .iter()
        .flat_map(|v| v.split(','))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Origin string a browser sends in `Origin:` and compares with ACAO
/// (`null` for file:/opaque pages).
pub(crate) fn origin_header(page: &Url) -> String {
    page.origin().ascii_serialization()
}

/// The ACAO (+ ACAC) check for a cross-origin `cors` response (and for a
/// preflight response). `credentials` = the request includes credentials.
pub(crate) fn check_cors_response(origin: &str, credentials: bool, resp_headers: &[(String, String)]) -> Result<(), PolicyError> {
    let acao = header_values(resp_headers, "access-control-allow-origin");
    let v = match acao.as_slice() {
        [] => return Err(PolicyError::Cors("no Access-Control-Allow-Origin header".into())),
        [one] => one.trim(),
        _ => return Err(PolicyError::Cors("multiple Access-Control-Allow-Origin headers".into())),
    };
    if v == "*" {
        if credentials {
            return Err(PolicyError::Cors("Access-Control-Allow-Origin is * but the request includes credentials".into()));
        }
        return Ok(());
    }
    if v != origin {
        return Err(PolicyError::Cors(format!("Access-Control-Allow-Origin {v:?} does not match the page origin {origin:?}")));
    }
    if credentials {
        let acac = header_values(resp_headers, "access-control-allow-credentials");
        if acac.as_slice() != ["true"] {
            return Err(PolicyError::Cors("credentials were sent but Access-Control-Allow-Credentials is not true".into()));
        }
    }
    Ok(())
}

/// What a successful preflight tells the caller to cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreflightResult {
    pub max_age_secs: u64,
}

impl RequestPolicy {
    /// Check an OPTIONS preflight response for `method` and the non-safelisted
    /// request header names `unsafe_headers` (lowercase, from
    /// `cors_unsafe_request_headers`).
    pub(crate) fn check_preflight_response(
        &self,
        origin: &str,
        credentials: bool,
        method: &str,
        unsafe_headers: &[String],
        status: u16,
        resp_headers: &[(String, String)],
    ) -> Result<PreflightResult, PolicyError> {
        if !(200..300).contains(&status) {
            return Err(PolicyError::Cors(format!("preflight response has status {status}")));
        }
        check_cors_response(origin, credentials, resp_headers)?;
        let method = normalize_method(method);
        let methods = list_items(resp_headers, "access-control-allow-methods");
        let wildcard_ok = !credentials;
        let method_ok = methods.iter().any(|m| m == &method) || (wildcard_ok && methods.iter().any(|m| m == "*")) || is_cors_safelisted_method(&method);
        if !method_ok {
            return Err(PolicyError::Cors(format!("method {method} is not in Access-Control-Allow-Methods")));
        }
        let allowed: Vec<String> = list_items(resp_headers, "access-control-allow-headers").iter().map(|s| s.to_ascii_lowercase()).collect();
        let star = wildcard_ok && allowed.iter().any(|h| h == "*");
        for h in unsafe_headers {
            // `*` never covers Authorization.
            let ok = allowed.contains(h) || (star && h != "authorization");
            if !ok {
                return Err(PolicyError::Cors(format!("header {h} is not in Access-Control-Allow-Headers")));
            }
        }
        let max_age = header_values(resp_headers, "access-control-max-age").first().and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(5);
        Ok(PreflightResult { max_age_secs: max_age.min(self.preflight_max_age_cap_secs) })
    }
}

// ------------------------------------------------------------ QF14b cookies
//
// The jar lives in wire (QF14, protocol version 3); scratchy only says, per
// request, whether the jar is used and under which top-level site. ALL the
// `X-Wire-Jar`/`X-Wire-Top-Site`/`X-Wire-Nav`/`X-Wire-Initiator-Site`/
// `X-Wire-Cookies`/`X-Wire-Cookie-*` headers are built in this section
// (`wire_cookie_headers`, `cookie_control_headers`) and nowhere else: a test
// greps the sources for the literals.
//
// Old wire: wire 3's detection is `X-Wire-Version >= 3` and, for a request
// that asked for the jar, `X-Wire-Cookies: on`. `CookieSupport` learns that
// from the first jar response (the first request of a session is the top-level
// navigation, whose top site is its own destination, so a pre-QF6 wire that
// forwards unknown `X-Wire-*` headers upstream leaks nothing but the site
// being visited; wire v2 strips every `X-Wire-*` request header, see
// `is_request_header_to_skip`). Once learned "unavailable" no jar header is
// sent any more and a single line is logged.
//
// Not done (documented): `cookieStore` (async cookie API), the
// `crossorigin` attribute's credentials mode for images/scripts (they always
// use the jar), same-origin-mode credentials dropped on a cross-origin
// redirect hop (wire follows redirects itself), a `--profile` jar name.

/// The jar name (QF-D2a: one shared jar; a later `--profile` changes this).
pub(crate) const COOKIE_JAR_NAME: &str = "scratchy";
pub(crate) const WIRE_HEADER_JAR: &str = "X-Wire-Jar";
pub(crate) const WIRE_HEADER_TOP_SITE: &str = "X-Wire-Top-Site";
pub(crate) const WIRE_HEADER_NAV: &str = "X-Wire-Nav";
pub(crate) const WIRE_HEADER_INITIATOR: &str = "X-Wire-Initiator-Site";
pub(crate) const WIRE_HEADER_COOKIES: &str = "X-Wire-Cookies";
pub(crate) const WIRE_HEADER_COOKIE_OP: &str = "X-Wire-Cookie-Op";
pub(crate) const WIRE_HEADER_COOKIE_URL: &str = "X-Wire-Cookie-Url";

/// Common multi-label public suffixes. COPY of wire's `MULTI_SUFFIXES`
/// (wire/src/cookies.rs): keep the two identical (same as `is_public_ip`).
/// Not the full Public Suffix List: a missing ccTLD second level makes two
/// unrelated sibling sites share a partition. wire normalizes the header
/// again, so a mismatch can only make a partition coarser or finer, never
/// leak a cookie across two sites wire itself separates.
const MULTI_SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "ltd.uk", "plc.uk", "net.uk", "sch.uk", "com.au", "net.au", "org.au",
    "edu.au", "gov.au", "asn.au", "id.au", "co.nz", "org.nz", "net.nz", "govt.nz", "ac.nz", "co.jp", "ne.jp", "or.jp",
    "ac.jp", "go.jp", "com.br", "net.br", "org.br", "gov.br", "com.cn", "net.cn", "org.cn", "gov.cn", "edu.cn", "co.in",
    "net.in", "org.in", "ac.in", "gov.in", "co.za", "org.za", "com.mx", "com.ar", "com.tr", "com.tw", "com.hk", "com.sg",
    "co.kr", "or.kr", "co.il", "com.ua", "com.pl", "co.id", "com.my", "com.ph", "com.vn", "com.co", "com.pe", "com.ng",
    "co.th", "com.eg", "com.sa", "github.io", "gitlab.io", "herokuapp.com", "blogspot.com", "appspot.com", "netlify.app",
    "vercel.app", "pages.dev", "workers.dev", "cloudfront.net", "s3.amazonaws.com", "azurewebsites.net", "web.app",
    "firebaseapp.com", "myshopify.com", "wordpress.com", "fly.dev", "onrender.com",
];

fn registrable_domain(host: &str) -> String {
    if host.parse::<IpAddr>().is_ok() || !host.contains('.') {
        return host.to_string();
    }
    let labels: Vec<&str> = host.split('.').collect();
    for i in 0..labels.len() {
        let suffix = labels[i..].join(".");
        if !suffix.contains('.') || MULTI_SUFFIXES.contains(&suffix.as_str()) {
            return if i == 0 { host.to_string() } else { labels[i - 1..].join(".") };
        }
    }
    host.to_string()
}

/// `scheme://registrable-domain` of an http(s) URL; `None` for everything
/// else (`file:` has no cookie origin, `data:`/`about:` neither).
pub(crate) fn site_of(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_matches(['[', ']']).to_ascii_lowercase();
    Some(format!("{}://{}", url.scheme(), registrable_domain(&host)))
}

/// What wire told us about cookie support (see the section comment).
pub(crate) struct CookieSupport {
    /// 0 unknown (assume yes), 1 available, 2 unavailable.
    state: std::sync::atomic::AtomicU8,
    warned: std::sync::atomic::AtomicBool,
    /// False for the process-wide instance in test builds, so a fake wire
    /// without a version header cannot switch cookies off for other tests.
    learns: bool,
}

impl CookieSupport {
    pub(crate) fn new(learns: bool) -> Self {
        CookieSupport { state: std::sync::atomic::AtomicU8::new(0), warned: std::sync::atomic::AtomicBool::new(false), learns }
    }

    /// False once wire proved old or cookie-less.
    pub(crate) fn usable(&self) -> bool {
        self.state.load(std::sync::atomic::Ordering::Relaxed) != 2
    }

    /// Feeds one response to a request that asked for the jar (or used the
    /// control channel). Returns the line to log, once, the first time wire
    /// is found unable to do cookies.
    pub(crate) fn observe(&self, version: Option<&str>, cookies: Option<&str>) -> Option<String> {
        use std::sync::atomic::Ordering::Relaxed;
        if !self.learns {
            return None;
        }
        let v: u32 = version.and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let off = cookies.is_some_and(|c| c.trim().eq_ignore_ascii_case("off"));
        let on = cookies.is_some_and(|c| c.trim().eq_ignore_ascii_case("on"));
        let why = if v < 3 {
            "wire is too old (restart it)"
        } else if off {
            "wire has cookies disabled"
        } else {
            if on {
                self.state.store(1, Relaxed);
            }
            return None;
        };
        self.state.store(2, Relaxed);
        (!self.warned.swap(true, Relaxed)).then(|| format!("scratchy: cookies unavailable: {why}"))
    }
}

static COOKIE_SUPPORT: std::sync::OnceLock<CookieSupport> = std::sync::OnceLock::new();

pub(crate) fn cookie_support() -> &'static CookieSupport {
    COOKIE_SUPPORT.get_or_init(|| CookieSupport::new(!cfg!(test)))
}

/// Why a request is made: decides which cookie headers it gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CookieKind {
    /// A top-level navigation. `initiator` = the site of the page that
    /// started it (a link click or meta refresh); `None` for a browser-
    /// initiated one (`:go`, startup, `:refresh`, history), which counts as
    /// same-site because the user did it.
    Navigation { initiator: Option<String> },
    /// Image, stylesheet, script, module, iframe, favicon: always the jar.
    Subresource,
    /// Page `fetch`/XHR: the Fetch spec's credentials mode decides.
    Fetch { credentials: CredentialsMode, same_origin: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CookieContext {
    /// The top-level page's site (a navigation's own destination). `None`:
    /// no cookie origin (`file:`/`about:`/`data:` page): no jar at all.
    pub top_site: Option<String>,
    pub kind: CookieKind,
}

impl CookieContext {
    pub(crate) fn navigation(dest: &Url, initiator: Option<&Url>) -> Self {
        CookieContext { top_site: site_of(dest), kind: CookieKind::Navigation { initiator: initiator.and_then(site_of) } }
    }

    pub(crate) fn subresource(top: &Url) -> Self {
        CookieContext { top_site: site_of(top), kind: CookieKind::Subresource }
    }

    pub(crate) fn fetch(top: &Url, credentials: CredentialsMode, same_origin: bool) -> Self {
        CookieContext { top_site: site_of(top), kind: CookieKind::Fetch { credentials, same_origin } }
    }

    /// Whether the Fetch spec allows credentials for this request.
    pub(crate) fn sends_credentials(&self) -> bool {
        match &self.kind {
            CookieKind::Fetch { credentials, same_origin } => match credentials {
                CredentialsMode::Omit => false,
                CredentialsMode::SameOrigin => *same_origin,
                CredentialsMode::Include => true,
            },
            _ => true,
        }
    }
}

/// `omit` / `same-origin` / `include` (anything else is the default,
/// `same-origin`).
pub(crate) fn parse_credentials(s: &str) -> CredentialsMode {
    match s {
        "omit" => CredentialsMode::Omit,
        "include" => CredentialsMode::Include,
        _ => CredentialsMode::SameOrigin,
    }
}

/// THE one place the per-request cookie headers are built.
pub(crate) fn wire_cookie_headers_with(support: &CookieSupport, ctx: &CookieContext) -> Vec<(String, String)> {
    let Some(top) = ctx.top_site.as_ref().filter(|_| support.usable()) else { return Vec::new() };
    if !ctx.sends_credentials() {
        // Spec: no credentials. Nothing is sent and nothing is stored.
        return vec![(WIRE_HEADER_COOKIES.to_string(), "off".to_string())];
    }
    let mut h = vec![(WIRE_HEADER_JAR.to_string(), COOKIE_JAR_NAME.to_string()), (WIRE_HEADER_TOP_SITE.to_string(), top.clone())];
    if let CookieKind::Navigation { initiator } = &ctx.kind {
        h.push((WIRE_HEADER_NAV.to_string(), "1".to_string()));
        if let Some(i) = initiator {
            h.push((WIRE_HEADER_INITIATOR.to_string(), i.clone()));
        }
    }
    h
}

pub(crate) fn wire_cookie_headers(ctx: &CookieContext) -> Vec<(String, String)> {
    wire_cookie_headers_with(cookie_support(), ctx)
}

/// Headers of a `document.cookie` control request (`op` = `get` | `set`).
pub(crate) fn cookie_control_headers(op: &str, page: &Url) -> Option<Vec<(String, String)>> {
    let top = site_of(page)?;
    Some(vec![
        (WIRE_HEADER_JAR.to_string(), COOKIE_JAR_NAME.to_string()),
        (WIRE_HEADER_TOP_SITE.to_string(), top),
        (WIRE_HEADER_COOKIE_OP.to_string(), op.to_string()),
        (WIRE_HEADER_COOKIE_URL.to_string(), page.to_string()),
    ])
}
