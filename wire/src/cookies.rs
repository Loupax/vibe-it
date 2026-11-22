//! The in-memory cookie jar (QF14, wire half). RFC 6265bis parsing and
//! matching, partitioned by top-level site ("Total Cookie Protection", the
//! user's QF-D2a design): every cookie is keyed by
//! (partition, domain, path, name), where the partition is the top-level
//! site (`scheme://registrable-domain`) the request happened under.
//!
//! Nothing here touches the network or the clock: every method that cares
//! about time takes `now` (unix seconds), so tests are deterministic. Cookie
//! names and values are secrets: nothing in this file logs them (see
//! `log_line`, which only ever carries a host and counts).
//!
//! Limits (per cookie / per domain / total) live in ONE `Limits` struct.
//!
//! Known limitation: there is no real public suffix list. `is_public_suffix`
//! treats every single-label name as a suffix plus a small embedded list of
//! common multi-label suffixes (`MULTI_SUFFIXES`). A domain missing from that
//! list (a rare ccTLD second level) could be set as a "super cookie" by a
//! sibling site; adding the `psl` crate later is a drop-in replacement of
//! that one function.

use std::net::IpAddr;
use url::Url;

/// Every number in one place.
#[derive(Clone, Debug)]
pub struct Limits {
    /// name + value, bytes.
    pub max_cookie_bytes: usize,
    /// One attribute value (Domain, Path, ...): longer ones are ignored.
    pub max_attr_bytes: usize,
    /// Cookies per (partition, domain).
    pub per_domain: usize,
    pub total: usize,
    /// Lifetime cap for Max-Age / Expires (RFC 6265bis: 400 days).
    pub max_lifetime_secs: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_cookie_bytes: 4096, max_attr_bytes: 1024, per_domain: 180, total: 3000, max_lifetime_secs: 400 * 86400 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SameSite {
    Strict,
    Lax,
    /// Also what an unspecified attribute means (Firefox's current default;
    /// user decision).
    None,
}

#[derive(Clone, Debug)]
struct Cookie {
    partition: String,
    domain: String,
    host_only: bool,
    path: String,
    name: String,
    value: String,
    secure: bool,
    http_only: bool,
    same_site: SameSite,
    /// Unix seconds; `None` = session cookie (lives as long as wire).
    expiry: Option<i64>,
    created: u64,
    last_access: u64,
}

/// Everything the jar needs to know about one request.
#[derive(Clone, Debug)]
pub struct RequestCtx {
    pub host: String,
    pub path: String,
    pub https: bool,
    /// Top-level site the cookies are partitioned under.
    pub partition: String,
    /// Is the request same-site with the top-level site (or, for a
    /// navigation with an initiator, with the initiating site)?
    pub same_site: bool,
    /// Top-level navigation with a safe method: Lax cookies are sent.
    pub lax_ok: bool,
    /// `document.cookie` access: HttpOnly is hidden and cannot be created.
    pub script: bool,
}

impl RequestCtx {
    /// `top_site`: the `X-Wire-Top-Site` header (None = the request's own
    /// site). `nav`: `X-Wire-Nav: 1`. `initiator`: optional
    /// `X-Wire-Initiator-Site`, only meaningful with `nav`: for a navigation
    /// scratchy sends the destination as the top site (the partition), so
    /// without an initiator a navigation is always same-site; with one,
    /// Strict cookies are withheld on a cross-site navigation.
    pub fn new(url: &Url, method: &str, top_site: Option<&str>, nav: bool, initiator: Option<&str>, script: bool) -> RequestCtx {
        let own = site_of_url(url);
        let partition = top_site.and_then(normalize_site).unwrap_or_else(|| own.clone());
        let basis = if nav { initiator.and_then(normalize_site).unwrap_or_else(|| partition.clone()) } else { partition.clone() };
        let same_site = basis == own;
        let safe = matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD");
        let path = if url.path().is_empty() { "/".to_string() } else { url.path().to_string() };
        RequestCtx { host: host_of(url), path, https: url.scheme() == "https", partition, same_site, lax_ok: nav && safe, script }
    }
}

// ---------- hosts, sites, public suffixes ----------

/// Common multi-label public suffixes (ICANN second levels and the popular
/// private suffixes). NOT the full list: see the module docs.
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

pub fn is_public_suffix(domain: &str) -> bool {
    !domain.contains('.') || MULTI_SUFFIXES.contains(&domain)
}

/// The registrable domain (eTLD+1) of `host`; an IP, a single-label name or a
/// name that IS a suffix is returned unchanged.
pub fn registrable_domain(host: &str) -> String {
    if host.parse::<IpAddr>().is_ok() || !host.contains('.') {
        return host.to_string();
    }
    let labels: Vec<&str> = host.split('.').collect();
    for i in 0..labels.len() {
        if is_public_suffix(&labels[i..].join(".")) {
            return if i == 0 { host.to_string() } else { labels[i - 1..].join(".") };
        }
    }
    host.to_string()
}

pub fn host_of(url: &Url) -> String {
    url.host_str().unwrap_or("").trim_matches(['[', ']']).to_ascii_lowercase()
}

/// `scheme://registrable-domain` of a URL.
pub fn site_of_url(url: &Url) -> String {
    format!("{}://{}", url.scheme(), registrable_domain(&host_of(url)))
}

/// Accepts a full URL or a site and returns the normalized site.
pub fn normalize_site(s: &str) -> Option<String> {
    Url::parse(s.trim()).ok().filter(|u| u.host_str().is_some()).map(|u| site_of_url(&u))
}

fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// https, or loopback (user decision: loopback is exempt from Secure).
fn secure_context(host: &str, https: bool) -> bool {
    https || is_loopback_host(host)
}

fn domain_match(host: &str, domain: &str) -> bool {
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
            && host.parse::<IpAddr>().is_err())
}

fn path_match(request: &str, cookie: &str) -> bool {
    request == cookie
        || (request.starts_with(cookie) && (cookie.ends_with('/') || request.as_bytes().get(cookie.len()) == Some(&b'/')))
}

fn default_path(request_path: &str) -> String {
    match request_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => request_path[..i].to_string(),
    }
}

// ---------- Set-Cookie parsing ----------

#[derive(Debug, Default)]
struct Parsed {
    name: String,
    value: String,
    domain: Option<String>,
    path: Option<String>,
    secure: bool,
    http_only: bool,
    same_site: Option<SameSite>,
    max_age: Option<i64>,
    expires: Option<i64>,
}

fn parse_set_cookie(line: &str, limits: &Limits) -> Option<Parsed> {
    if line.bytes().any(|b| (b < 0x20 && b != b'\t') || b == 0x7f) {
        return None;
    }
    let mut parts = line.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let (name, value) = (name.trim(), value.trim());
    if name.is_empty() || name.len() + value.len() > limits.max_cookie_bytes {
        return None;
    }
    let mut p = Parsed { name: name.to_string(), value: value.to_string(), ..Parsed::default() };
    for attr in parts {
        let (an, av) = match attr.split_once('=') {
            Some((n, v)) => (n.trim().to_ascii_lowercase(), v.trim()),
            None => (attr.trim().to_ascii_lowercase(), ""),
        };
        if av.len() > limits.max_attr_bytes {
            continue;
        }
        match an.as_str() {
            "expires" => p.expires = parse_cookie_date(av).or(p.expires),
            "max-age" => {
                let digits = av.strip_prefix('-').unwrap_or(av);
                if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                    p.max_age = Some(av.parse::<i64>().unwrap_or(if av.starts_with('-') { i64::MIN } else { i64::MAX }));
                }
            }
            "domain" => {
                let d = av.trim_start_matches('.').to_ascii_lowercase();
                p.domain = if d.is_empty() { None } else { Some(d) };
            }
            "path" => p.path = if av.starts_with('/') { Some(av.to_string()) } else { None },
            "secure" => p.secure = true,
            "httponly" => p.http_only = true,
            "samesite" => {
                p.same_site = match av.to_ascii_lowercase().as_str() {
                    "strict" => Some(SameSite::Strict),
                    "lax" => Some(SameSite::Lax),
                    "none" => Some(SameSite::None),
                    _ => None,
                }
            }
            _ => {} // Partitioned and unknown attributes are ignored
        }
    }
    Some(p)
}

/// RFC 6265 section 5.1.1 lenient date parser; returns unix seconds.
pub fn parse_cookie_date(s: &str) -> Option<i64> {
    let is_delim = |c: char| matches!(c as u32, 0x09 | 0x20..=0x2F | 0x3B..=0x40 | 0x5B..=0x60 | 0x7B..=0x7E);
    let (mut time, mut day, mut month, mut year) = (None, None, None, None);
    let lead = |t: &str, max: usize| -> Option<i64> {
        let n = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if n == 0 || n > max { None } else { t[..n].parse().ok() }
    };
    for tok in s.split(is_delim).filter(|t| !t.is_empty()) {
        if time.is_none() {
            let f: Vec<&str> = tok.splitn(3, ':').collect();
            if f.len() == 3
                && let (Some(h), Some(m), Some(sec)) = (lead(f[0], 2), lead(f[1], 2), lead(f[2], 2))
            {
                time = Some((h, m, sec));
                continue;
            }
        }
        if day.is_none()
            && let Some(d) = lead(tok, 2)
        {
            day = Some(d);
            continue;
        }
        if month.is_none() && tok.len() >= 3 && tok.is_char_boundary(3) {
            const M: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
            if let Some(i) = M.iter().position(|m| tok[..3].eq_ignore_ascii_case(m)) {
                month = Some(i as i64 + 1);
                continue;
            }
        }
        if year.is_none()
            && let Some(y) = lead(tok, 4)
            && tok.chars().take_while(|c| c.is_ascii_digit()).count() >= 2
        {
            year = Some(y);
        }
    }
    let (h, m, sec) = time?;
    let (day, month, mut year) = (day?, month?, year?);
    if (70..=99).contains(&year) {
        year += 1900;
    } else if (0..=69).contains(&year) {
        year += 2000;
    }
    if !(1..=31).contains(&day) || year < 1601 || h > 23 || m > 59 || sec > 59 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86400 + h * 3600 + m * 60 + sec)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

// ---------- the jar ----------

pub struct Jar {
    cookies: Vec<Cookie>,
    limits: Limits,
    seq: u64,
    /// A persistable change happened since the last `take_dirty` (W-PERSIST1).
    dirty: bool,
}

impl Default for Jar {
    fn default() -> Self {
        Jar::new(Limits::default())
    }
}

impl Jar {
    pub fn new(limits: Limits) -> Jar {
        Jar { cookies: Vec::new(), limits, seq: 0, dirty: false }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    /// Cookies stored under one (partition, domain): tests and logging.
    pub fn count_for(&self, partition: &str, domain: &str) -> usize {
        self.cookies.iter().filter(|c| c.partition == partition && c.domain == domain).count()
    }

    fn matching(&mut self, ctx: &RequestCtx, now: i64) -> Vec<usize> {
        let secure_ctx = secure_context(&ctx.host, ctx.https);
        let mut idx: Vec<usize> = (0..self.cookies.len())
            .filter(|&i| {
                let c = &self.cookies[i];
                c.partition == ctx.partition
                    && c.expiry.is_none_or(|e| e > now)
                    && if c.host_only { c.domain == ctx.host } else { domain_match(&ctx.host, &c.domain) }
                    && path_match(&ctx.path, &c.path)
                    && (!c.secure || secure_ctx)
                    && !(ctx.script && c.http_only)
                    && match c.same_site {
                        SameSite::None => true,
                        SameSite::Lax => ctx.same_site || ctx.lax_ok,
                        SameSite::Strict => ctx.same_site,
                    }
            })
            .collect();
        idx.sort_by(|&a, &b| {
            let (a, b) = (&self.cookies[a], &self.cookies[b]);
            b.path.len().cmp(&a.path.len()).then(a.created.cmp(&b.created))
        });
        idx
    }

    fn collect(&mut self, ctx: &RequestCtx, now: i64) -> Vec<(String, String)> {
        let idx = self.matching(ctx, now);
        let mut out = Vec::with_capacity(idx.len());
        for i in idx {
            self.seq += 1;
            self.cookies[i].last_access = self.seq;
            out.push((self.cookies[i].name.clone(), self.cookies[i].value.clone()));
        }
        out
    }

    /// The `Cookie:` header value for a request, `None` when nothing matches.
    pub fn cookie_header(&mut self, ctx: &RequestCtx, now: i64) -> Option<String> {
        let v = self.collect(ctx, now);
        if v.is_empty() { None } else { Some(v.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join("; ")) }
    }

    /// `document.cookie` getter: pass a `ctx` built with `script = true`.
    pub fn document_cookie(&mut self, ctx: &RequestCtx, now: i64) -> String {
        self.cookie_header(ctx, now).unwrap_or_default()
    }

    /// Removes every cookie in one partition (the top-level site `site`,
    /// already normalized): that site's own first-party cookies and every
    /// third-party cookie embedded under it. Returns how many were removed.
    pub fn clear_site(&mut self, site: &str) -> usize {
        let before = self.cookies.len();
        self.cookies.retain(|c| c.partition != site);
        let n = before - self.cookies.len();
        if n > 0 {
            self.dirty = true;
        }
        n
    }

    /// `Clear-Site-Data: "cookies"` (W-CACHE2): removes, in one partition, the
    /// cookies of the responding host: those it could have set (its own and
    /// its parent domains') and those of its subdomains.
    pub fn clear_host(&mut self, partition: &str, host: &str) -> usize {
        let before = self.cookies.len();
        self.cookies.retain(|c| !(c.partition == partition && (domain_match(host, &c.domain) || domain_match(&c.domain, host))));
        let n = before - self.cookies.len();
        if n > 0 {
            self.dirty = true;
        }
        n
    }

    /// Whether the jar changed since the last call (clears the flag).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Marks the jar changed again (a save failed and must be retried).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// The `--cookie-file` text: a header line, then one persistent cookie
    /// per line (session cookies and expired ones are left out), fields
    /// separated by tabs, text fields percent-escaped for `%` and control bytes:
    /// partition, domain, host_only, path, name, value, secure, http_only,
    /// same_site (0 none, 1 lax, 2 strict), expiry (unix s), created.
    pub fn to_file_text(&self, now: i64) -> String {
        let mut out = String::from(FILE_HEADER);
        out.push('\n');
        let mut keep: Vec<&Cookie> = self.cookies.iter().filter(|c| c.expiry.is_some_and(|e| e > now)).collect();
        keep.sort_by_key(|c| c.created);
        for c in keep {
            let ss = match c.same_site {
                SameSite::None => 0,
                SameSite::Lax => 1,
                SameSite::Strict => 2,
            };
            let fields = [
                esc(&c.partition),
                esc(&c.domain),
                u8::from(c.host_only).to_string(),
                esc(&c.path),
                esc(&c.name),
                esc(&c.value),
                u8::from(c.secure).to_string(),
                u8::from(c.http_only).to_string(),
                ss.to_string(),
                c.expiry.unwrap_or(0).to_string(),
                c.created.to_string(),
            ];
            out.push_str(&fields.join("\t"));
            out.push('\n');
        }
        out
    }

    /// Parses `to_file_text` output. Cookies already expired at `now` are
    /// dropped. `Err` (any bad line or header) means the file is corrupt and
    /// the caller starts with an empty jar.
    pub fn from_file_text(text: &str, now: i64) -> Result<Jar, String> {
        let mut lines = text.lines();
        if lines.next() != Some(FILE_HEADER) {
            return Err("missing or unknown header line".to_string());
        }
        let mut jar = Jar::default();
        let mut rows: Vec<Cookie> = Vec::new();
        for (n, line) in lines.enumerate() {
            let f: Vec<&str> = line.split('\t').collect();
            let bad = || format!("line {} is not a cookie", n + 2);
            if f.len() != 11 {
                return Err(bad());
            }
            let flag = |s: &str| match s {
                "0" => Ok(false),
                "1" => Ok(true),
                _ => Err(bad()),
            };
            let same_site = match f[8] {
                "0" => SameSite::None,
                "1" => SameSite::Lax,
                "2" => SameSite::Strict,
                _ => return Err(bad()),
            };
            let expiry: i64 = f[9].parse().map_err(|_| bad())?;
            let created: u64 = f[10].parse().map_err(|_| bad())?;
            let (partition, domain, path, name, value) =
                (unesc(f[0]).ok_or_else(bad)?, unesc(f[1]).ok_or_else(bad)?, unesc(f[3]).ok_or_else(bad)?, unesc(f[4]).ok_or_else(bad)?, unesc(f[5]).ok_or_else(bad)?);
            if partition.is_empty() || domain.is_empty() || name.is_empty() || !path.starts_with('/') {
                return Err(bad());
            }
            rows.push(Cookie {
                partition,
                domain,
                host_only: flag(f[2])?,
                path,
                name,
                value,
                secure: flag(f[6])?,
                http_only: flag(f[7])?,
                same_site,
                expiry: Some(expiry),
                created,
                last_access: 0,
            });
        }
        rows.retain(|c| c.expiry.is_some_and(|e| e > now));
        rows.sort_by_key(|c| c.created);
        for mut c in rows {
            jar.seq += 1;
            c.created = jar.seq;
            c.last_access = jar.seq;
            jar.cookies.push(c);
        }
        Ok(jar)
    }

    /// Stores one `Set-Cookie` line (or a `document.cookie = ...` string with
    /// `ctx.script`). Returns whether the jar accepted it.
    pub fn store(&mut self, ctx: &RequestCtx, line: &str, now: i64) -> bool {
        let Some(p) = parse_set_cookie(line, &self.limits) else { return false };
        let secure_ctx = secure_context(&ctx.host, ctx.https);
        if ctx.script && p.http_only {
            return false;
        }
        let (domain, host_only) = match &p.domain {
            None => (ctx.host.clone(), true),
            Some(d) if ctx.host.parse::<IpAddr>().is_ok() || is_public_suffix(d) => {
                if *d != ctx.host {
                    return false;
                }
                (d.clone(), true)
            }
            Some(d) => {
                if !domain_match(&ctx.host, d) {
                    return false;
                }
                (d.clone(), false)
            }
        };
        let path = p.path.clone().unwrap_or_else(|| default_path(&ctx.path));
        if p.secure && !secure_ctx {
            return false;
        }
        let lname = p.name.to_ascii_lowercase();
        if lname.starts_with("__secure-") && !(p.secure && secure_ctx) {
            return false;
        }
        if lname.starts_with("__host-") && !(p.secure && secure_ctx && p.domain.is_none() && p.path.as_deref() == Some("/")) {
            return false;
        }
        let cap = now + self.limits.max_lifetime_secs;
        let expiry = match (p.max_age, p.expires) {
            (Some(m), _) => Some(if m <= 0 { 0 } else { now.saturating_add(m).min(cap) }),
            (None, Some(e)) => Some(e.min(cap)),
            (None, None) => None,
        };
        let existing = self
            .cookies
            .iter()
            .position(|c| c.partition == ctx.partition && c.domain == domain && c.path == path && c.name == p.name);
        let mut created = None;
        if let Some(i) = existing {
            let old = &self.cookies[i];
            if (ctx.script && old.http_only) || (old.secure && !secure_ctx) {
                return false;
            }
            created = Some(old.created);
            self.dirty = true;
            self.cookies.remove(i);
        }
        if expiry.is_some_and(|e| e <= now) {
            return true; // a delete
        }
        self.seq += 1;
        self.dirty = true;
        self.cookies.push(Cookie {
            partition: ctx.partition.clone(),
            domain: domain.clone(),
            host_only,
            path,
            name: p.name,
            value: p.value,
            secure: p.secure,
            http_only: p.http_only,
            same_site: p.same_site.unwrap_or(SameSite::None),
            expiry,
            created: created.unwrap_or(self.seq),
            last_access: self.seq,
        });
        self.evict(&ctx.partition, &domain, now);
        true
    }

    /// Eviction: expired cookies first, then the least recently accessed in
    /// the over-full domain, then (total cap) the least recently accessed.
    fn evict(&mut self, partition: &str, domain: &str, now: i64) {
        if self.cookies.len() > self.limits.total || self.count_for(partition, domain) > self.limits.per_domain {
            self.cookies.retain(|c| c.expiry.is_none_or(|e| e > now));
        }
        while self.count_for(partition, domain) > self.limits.per_domain {
            let victim = (0..self.cookies.len())
                .filter(|&i| self.cookies[i].partition == partition && self.cookies[i].domain == domain)
                .min_by_key(|&i| self.cookies[i].last_access);
            match victim {
                Some(i) => {
                    self.cookies.remove(i);
                }
                None => break,
            }
        }
        while self.cookies.len() > self.limits.total {
            let i = (0..self.cookies.len()).min_by_key(|&i| self.cookies[i].last_access).unwrap_or(0);
            self.cookies.remove(i);
        }
    }
}

const FILE_HEADER: &str = "# wire cookies v1";

/// Percent-escapes `%`, tab, CR, LF and other control bytes so a field never
/// contains the file's separators.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '%' || ch.is_control() {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn unesc(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Writes `text` to `path` atomically: a temp file next to it (mode 0600),
/// fsync, rename over `path`, fsync of the directory.
pub fn write_file_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut tmp_name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    tmp_name.push(format!(".tmp.{}", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    let result = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return result;
    }
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() { std::path::Path::new(".") } else { dir };
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

/// The only thing wire may log about cookies: a host and counts.
pub fn log_line(host: &str, sent: usize, stored: usize) -> String {
    format!("wire: cookies host={host} sent={sent} stored={stored}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn ctx(url: &str) -> RequestCtx {
        RequestCtx::new(&Url::parse(url).unwrap(), "GET", None, false, None, false)
    }
    fn under(url: &str, top: &str) -> RequestCtx {
        RequestCtx::new(&Url::parse(url).unwrap(), "GET", Some(top), false, None, false)
    }
    fn script(url: &str) -> RequestCtx {
        RequestCtx::new(&Url::parse(url).unwrap(), "GET", None, false, None, true)
    }

    #[test]
    fn registrable_domains() {
        assert_eq!(registrable_domain("www.example.com"), "example.com");
        assert_eq!(registrable_domain("a.b.example.co.uk"), "example.co.uk");
        assert_eq!(registrable_domain("co.uk"), "co.uk");
        assert_eq!(registrable_domain("localhost"), "localhost");
        assert_eq!(registrable_domain("127.0.0.1"), "127.0.0.1");
        assert_eq!(registrable_domain("x.github.io"), "x.github.io");
        assert_eq!(site_of_url(&Url::parse("https://a.example.com/x").unwrap()), "https://example.com");
    }

    #[test]
    fn date_parsing() {
        assert_eq!(parse_cookie_date("Thu, 01 Jan 1970 00:00:01 GMT"), Some(1));
        assert_eq!(parse_cookie_date("Wed, 21 Oct 2015 07:28:00 GMT"), Some(1445412480));
        assert_eq!(parse_cookie_date("Wednesday, 21-Oct-15 07:28:00 GMT"), Some(1445412480));
        assert_eq!(parse_cookie_date("garbage"), None);
    }

    #[test]
    fn round_trip_and_replace() {
        let mut j = Jar::default();
        assert!(j.store(&ctx("http://example.com/a/b"), "k=v1", NOW));
        assert_eq!(j.cookie_header(&ctx("http://example.com/a/b"), NOW).as_deref(), Some("k=v1"));
        assert!(j.store(&ctx("http://example.com/a/b"), "k=v2", NOW));
        assert_eq!(j.len(), 1);
        assert_eq!(j.cookie_header(&ctx("http://example.com/a/c"), NOW).as_deref(), Some("k=v2"));
        // default path is /a: not sent to /other
        assert_eq!(j.cookie_header(&ctx("http://example.com/other"), NOW), None);
    }

    #[test]
    fn host_only_vs_domain_cookies() {
        let mut j = Jar::default();
        j.store(&ctx("http://www.example.com/"), "h=1", NOW);
        j.store(&ctx("http://www.example.com/"), "d=2; Domain=example.com", NOW);
        j.store(&ctx("http://www.example.com/"), "dot=3; Domain=.example.com", NOW);
        assert_eq!(j.cookie_header(&ctx("http://www.example.com/"), NOW).as_deref(), Some("h=1; d=2; dot=3"));
        // the same site (example.com) but another host: partition is shared, host-only is not sent
        let api = under("http://api.example.com/", "http://www.example.com/");
        assert_eq!(j.cookie_header(&api, NOW).as_deref(), Some("d=2; dot=3"));
        assert_eq!(j.cookie_header(&ctx("http://example.com/"), NOW).as_deref(), Some("d=2; dot=3"));
        assert_eq!(j.cookie_header(&ctx("http://notexample.com/"), NOW), None);
        // a sibling cannot set a Domain it does not match
        assert!(!j.store(&ctx("http://www.example.com/"), "x=1; Domain=other.com", NOW));
        assert!(!j.store(&ctx("http://example.com/"), "x=1; Domain=www.example.com", NOW));
    }

    #[test]
    fn public_suffix_domains_are_rejected() {
        let mut j = Jar::default();
        assert!(!j.store(&ctx("http://a.example.com/"), "x=1; Domain=com", NOW));
        assert!(!j.store(&ctx("http://a.example.co.uk/"), "x=1; Domain=co.uk", NOW));
        assert!(!j.store(&ctx("http://x.github.io/"), "x=1; Domain=github.io", NOW));
        // a host that IS a suffix may set a host-only cookie
        assert!(j.store(&ctx("http://localhost/"), "x=1; Domain=localhost", NOW));
        assert_eq!(j.cookie_header(&ctx("http://localhost/"), NOW).as_deref(), Some("x=1"));
    }

    #[test]
    fn ip_hosts_are_host_only() {
        let mut j = Jar::default();
        assert!(j.store(&ctx("http://127.0.0.1:99/"), "a=1; Domain=127.0.0.1", NOW));
        assert!(!j.store(&ctx("http://127.0.0.1/"), "b=1; Domain=0.0.1", NOW));
        assert_eq!(j.cookie_header(&ctx("http://127.0.0.1/"), NOW).as_deref(), Some("a=1"));
    }

    #[test]
    fn path_matching() {
        let mut j = Jar::default();
        j.store(&ctx("http://e.com/"), "a=1; Path=/foo", NOW);
        j.store(&ctx("http://e.com/"), "b=2; Path=/foo/bar", NOW);
        j.store(&ctx("http://e.com/"), "c=3; Path=/", NOW);
        assert_eq!(j.cookie_header(&ctx("http://e.com/foo/bar/x"), NOW).as_deref(), Some("b=2; a=1; c=3"));
        assert_eq!(j.cookie_header(&ctx("http://e.com/foobar"), NOW).as_deref(), Some("c=3"));
        assert_eq!(j.cookie_header(&ctx("http://e.com/foo"), NOW).as_deref(), Some("a=1; c=3"));
    }

    #[test]
    fn secure_only_over_https_except_loopback() {
        let mut j = Jar::default();
        assert!(!j.store(&ctx("http://e.com/"), "s=1; Secure", NOW), "a Secure cookie cannot be set from plain http");
        assert!(j.store(&ctx("https://e.com/"), "s=1; Secure", NOW));
        assert_eq!(j.cookie_header(&ctx("http://e.com/"), NOW), None);
        assert_eq!(j.cookie_header(&ctx("https://e.com/"), NOW).as_deref(), Some("s=1"));
        // loopback exempt
        assert!(j.store(&ctx("http://127.0.0.1/"), "s=1; Secure", NOW));
        assert_eq!(j.cookie_header(&ctx("http://127.0.0.1/"), NOW).as_deref(), Some("s=1"));
        // plain http cannot overwrite a Secure cookie
        assert!(j.store(&ctx("https://e.com/"), "t=1; Secure", NOW));
        assert!(!j.store(&under("http://e.com/", "https://e.com/"), "t=2", NOW));
    }

    #[test]
    fn expiry_max_age_wins_over_expires_and_deletes() {
        let mut j = Jar::default();
        let c = ctx("http://e.com/");
        j.store(&c, "a=1; Max-Age=100", NOW);
        assert!(j.cookie_header(&c, NOW + 99).is_some());
        assert!(j.cookie_header(&c, NOW + 100).is_none());
        // Max-Age beats a far-future Expires
        j.store(&c, "b=1; Max-Age=10; Expires=Fri, 01 Jan 2100 00:00:00 GMT", NOW);
        assert!(!j.cookie_header(&c, NOW + 11).unwrap().contains("b="));
        // Max-Age beats a past Expires
        j.store(&c, "c=1; Max-Age=1000; Expires=Thu, 01 Jan 1970 00:00:01 GMT", NOW);
        assert!(j.cookie_header(&c, NOW + 11).is_some());
        // Expires alone
        j.store(&c, "d=1; Expires=Fri, 01 Jan 2100 00:00:00 GMT", NOW);
        assert!(j.cookie_header(&c, NOW + 11).unwrap().contains("d=1"));
        // Max-Age=0 and a past Expires delete
        j.store(&c, "d=gone; Max-Age=0", NOW);
        assert!(!j.cookie_header(&c, NOW + 11).unwrap().contains("d="));
        j.store(&c, "c=2; Expires=Thu, 01 Jan 1970 00:00:01 GMT", NOW);
        assert_eq!(j.cookie_header(&c, NOW + 101), None);
        // lifetime is capped at 400 days
        j.store(&c, "e=1; Max-Age=99999999999", NOW);
        assert!(j.cookie_header(&c, NOW + 401 * 86400).is_none());
        assert!(j.cookie_header(&c, NOW + 399 * 86400).is_some());
    }

    #[test]
    fn httponly_is_hidden_from_scripts_but_sent_in_requests() {
        let mut j = Jar::default();
        j.store(&ctx("http://e.com/"), "h=secret; HttpOnly", NOW);
        j.store(&ctx("http://e.com/"), "v=open", NOW);
        assert_eq!(j.cookie_header(&ctx("http://e.com/"), NOW).as_deref(), Some("h=secret; v=open"));
        assert_eq!(j.document_cookie(&script("http://e.com/"), NOW), "v=open");
        // script cannot create HttpOnly nor overwrite one
        assert!(!j.store(&script("http://e.com/"), "x=1; HttpOnly", NOW));
        assert!(!j.store(&script("http://e.com/"), "h=pwn", NOW));
        assert_eq!(j.cookie_header(&ctx("http://e.com/"), NOW).as_deref(), Some("h=secret; v=open"));
    }

    #[test]
    fn samesite_rules_with_top_sites() {
        let me = "http://shop.test/";
        let all = "st=1; lx=1; nn=1; un=1";
        let mut j = Jar::default();
        for s in ["st=1; SameSite=Strict", "lx=1; SameSite=Lax", "nn=1; SameSite=None", "un=1"] {
            j.store(&ctx(me), s, NOW);
        }
        // same site: all
        assert_eq!(j.cookie_header(&ctx(me), NOW).as_deref(), Some(all));
        assert_eq!(j.cookie_header(&under(me, "http://www.shop.test/"), NOW).as_deref(), Some(all));
        // a cross-site request in the SAME partition (partition forced)
        let mut c = under(me, "http://other.test");
        c.partition = "http://shop.test".into();
        assert!(!c.same_site);
        assert_eq!(j.cookie_header(&c, NOW).as_deref(), Some("nn=1; un=1"));
        // top-level navigation from another site: Lax too, Strict not
        let nav = RequestCtx::new(&Url::parse(me).unwrap(), "GET", Some("http://shop.test"), true, Some("http://other.test"), false);
        assert_eq!(j.cookie_header(&nav, NOW).as_deref(), Some("lx=1; nn=1; un=1"));
        // ... but not for a cross-site POST navigation
        let post = RequestCtx::new(&Url::parse(me).unwrap(), "POST", Some("http://shop.test"), true, Some("http://other.test"), false);
        assert_eq!(j.cookie_header(&post, NOW).as_deref(), Some("nn=1; un=1"));
        // navigation without an initiator: same-site
        let nav2 = RequestCtx::new(&Url::parse(me).unwrap(), "GET", Some("http://shop.test"), true, None, false);
        assert_eq!(j.cookie_header(&nav2, NOW).as_deref(), Some(all));
    }

    #[test]
    fn partitioning_separates_a_third_party_under_two_top_sites() {
        let mut j = Jar::default();
        let tp = "http://tracker.test/pixel";
        j.store(&under(tp, "http://a.test"), "id=for-a", NOW);
        j.store(&under(tp, "http://b.test"), "id=for-b", NOW);
        assert_eq!(j.len(), 2);
        assert_eq!(j.cookie_header(&under(tp, "http://a.test"), NOW).as_deref(), Some("id=for-a"));
        assert_eq!(j.cookie_header(&under(tp, "http://b.test"), NOW).as_deref(), Some("id=for-b"));
        // first-party visit to the tracker itself sees neither
        assert_eq!(j.cookie_header(&ctx(tp), NOW), None);
        assert_eq!(j.cookie_header(&under(tp, "http://c.test"), NOW), None);
    }

    #[test]
    fn prefixes_are_enforced() {
        let mut j = Jar::default();
        let s = ctx("https://e.com/");
        assert!(!j.store(&s, "__Secure-a=1", NOW));
        assert!(j.store(&s, "__Secure-a=1; Secure", NOW));
        assert!(!j.store(&ctx("http://e.com/"), "__Secure-b=1; Secure", NOW));
        assert!(!j.store(&s, "__Host-a=1; Secure", NOW), "needs Path=/");
        assert!(!j.store(&s, "__Host-b=1; Secure; Path=/; Domain=e.com", NOW), "no Domain allowed");
        assert!(j.store(&s, "__Host-c=1; Secure; Path=/", NOW));
        assert!(!j.store(&s, "__HOST-d=1; Path=/", NOW), "case-insensitive prefix");
        assert_eq!(j.len(), 2);
    }

    #[test]
    fn malformed_set_cookie_is_ignored() {
        let mut j = Jar::default();
        let c = ctx("http://e.com/");
        for bad in ["", "novalue", "=v", "  =v", "a=b\r\nc=d", "a=\u{0}", ";;;"] {
            assert!(!j.store(&c, bad, NOW), "{bad:?}");
        }
        assert_eq!(j.len(), 0);
        // junk attributes are ignored, the cookie stays
        assert!(j.store(&c, "a=1; Max-Age=abc; Expires=nope; Path=relative; Frob; SameSite=weird", NOW));
        assert_eq!(j.cookie_header(&c, NOW).as_deref(), Some("a=1"));
        assert!(j.store(&c, "e=; Path=/", NOW), "empty value is fine");
    }

    #[test]
    fn size_cap_is_4096_bytes_per_cookie() {
        let mut j = Jar::default();
        let c = ctx("http://e.com/");
        assert!(j.store(&c, &format!("n={}", "v".repeat(4095)), NOW));
        assert!(!j.store(&c, &format!("m={}", "v".repeat(4096)), NOW));
        // an over-long attribute value is ignored, not fatal
        assert!(j.store(&c, &format!("p=1; Path=/{}", "x".repeat(2000)), NOW));
    }

    #[test]
    fn per_domain_cap_evicts_expired_first_then_least_recently_accessed() {
        let mut j = Jar::default();
        let c = ctx("http://e.com/");
        j.store(&c, "short=1; Max-Age=10", NOW);
        for i in 0..179 {
            j.store(&c, &format!("k{i}=1"), NOW);
        }
        assert_eq!(j.count_for("http://e.com", "e.com"), 180);
        // `short` expired: it is the one evicted, everything else stays
        j.store(&c, "new1=1", NOW + 11);
        assert_eq!(j.count_for("http://e.com", "e.com"), 180);
        let h = j.cookie_header(&c, NOW + 11).unwrap();
        assert!(!h.contains("short=") && h.contains("k0=1") && h.contains("new1=1"));
        // nothing expired: the least recently accessed goes
        j.store(&c, "new2=1", NOW + 12);
        assert_eq!(j.count_for("http://e.com", "e.com"), 180);
        // another domain is unaffected
        j.store(&ctx("http://f.com/"), "x=1", NOW);
        assert_eq!(j.count_for("http://f.com", "f.com"), 1);
    }

    #[test]
    fn least_recently_accessed_is_evicted_first() {
        let mut j = Jar::new(Limits { per_domain: 3, ..Limits::default() });
        let c = ctx("http://e.com/");
        for n in ["a", "b", "c"] {
            j.store(&c, &format!("{n}=1"), NOW);
        }
        // touch b and c only (a stays the least recently accessed)
        let mut only_bc = c.clone();
        only_bc.path = "/".into();
        j.cookies.iter_mut().filter(|k| k.name != "a").for_each(|k| k.last_access += 100);
        j.store(&c, "d=1", NOW);
        let h = j.cookie_header(&c, NOW).unwrap();
        assert!(!h.contains("a=1") && h.contains("b=1") && h.contains("d=1"), "{h}");
    }

    #[test]
    fn total_cap_is_enforced() {
        let mut j = Jar::new(Limits { total: 50, per_domain: 180, ..Limits::default() });
        for i in 0..80 {
            j.store(&ctx(&format!("http://h{i}.test/")), "k=1", NOW);
        }
        assert_eq!(j.len(), 50);
        assert_eq!(j.cookie_header(&ctx("http://h0.test/"), NOW), None);
        assert!(j.cookie_header(&ctx("http://h79.test/"), NOW).is_some());
        assert_eq!(Limits::default().total, 3000);
        assert_eq!(Limits::default().per_domain, 180);
    }

    #[test]
    fn log_line_carries_only_host_and_counts() {
        assert_eq!(log_line("e.com", 2, 1), "wire: cookies host=e.com sent=2 stored=1");
    }
}
