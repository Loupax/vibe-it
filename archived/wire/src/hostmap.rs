//! `--map-host '*.SUFFIX=LOOPBACK-IP'` (QWPT8): resolve one reserved test domain to a loopback
//! address, so wire can reach a local web-platform-tests server (wptserve) under its real
//! hostnames (`web-platform.test`, `www1.web-platform.test`, ...) without /etc/hosts.
//!
//! What it allows: a request whose host is exactly SUFFIX or a subdomain of it connects to the
//! mapped loopback address on the URL's own port, and (the operator opted in) is exempt from
//! `X-Wire-Policy: public-only`. Nothing else changes: every other host resolves through DNS and is
//! checked exactly as before. What it refuses, at startup (wire exits 1): a pattern that is not
//! `*.NAME` (no bare `*`, no exact hosts, no IP literals, no empty labels), a suffix whose last
//! label is not a name RFC 2606/6761 reserves for testing (`test`, `localhost`, `invalid`,
//! `example`), a suffix of a single label (`*.test`), and any target that is not a loopback IP
//! literal (127.0.0.0/8 or ::1; not a hostname, not 0.0.0.0, not private or public addresses).
//! Real websites (`.com`, `.org`, ...) therefore cannot be mapped at all.

use std::net::IpAddr;
use std::sync::RwLock;

/// Top-level labels a mapping may use (reserved for tests and documentation, never real sites).
const RESERVED_TLDS: [&str; 4] = ["test", "localhost", "invalid", "example"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostMap {
    /// Lowercase, no leading `*.`, no trailing dot.
    pub suffix: String,
    pub ip: IpAddr,
}

impl HostMap {
    /// Parse `*.SUFFIX=IP`.
    pub fn parse(spec: &str) -> Result<HostMap, String> {
        let bad = |why: &str| Err(format!("--map-host {spec:?}: {why}"));
        let Some((pat, target)) = spec.split_once('=') else {
            return bad("expected '*.SUFFIX=IP'");
        };
        let Some(suffix) = pat.strip_prefix("*.") else {
            return bad("the pattern must be '*.SUFFIX' (no exact hosts, no bare '*')");
        };
        let suffix = suffix.to_ascii_lowercase();
        let labels: Vec<&str> = suffix.split('.').collect();
        let label_ok = |l: &&str| {
            !l.is_empty()
                && l.len() <= 63
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        };
        if !labels.iter().all(label_ok) {
            return bad("the suffix must be dot-separated hostname labels");
        }
        if labels.len() < 2 {
            return bad("the suffix needs at least two labels (e.g. web-platform.test)");
        }
        let tld = labels[labels.len() - 1];
        if !RESERVED_TLDS.contains(&tld) {
            return bad("the suffix must end in a reserved test TLD (.test, .localhost, .invalid, .example)");
        }
        let Ok(ip) = target.parse::<IpAddr>() else {
            return bad("the target must be an IP address literal");
        };
        let loopback = match ip {
            IpAddr::V4(v4) => v4.octets()[0] == 127,
            IpAddr::V6(v6) => v6.is_loopback(),
        };
        if !loopback {
            return bad("the target must be a loopback address (127.0.0.0/8 or ::1)");
        }
        Ok(HostMap { suffix, ip })
    }

    fn matches(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.');
        host.eq_ignore_ascii_case(&self.suffix)
            || (host.len() > self.suffix.len() + 1
                && host.as_bytes()[host.len() - self.suffix.len() - 1] == b'.'
                && host[host.len() - self.suffix.len()..].eq_ignore_ascii_case(&self.suffix))
    }
}

static MAPS: RwLock<Vec<HostMap>> = RwLock::new(Vec::new());

pub fn add(m: HostMap) {
    MAPS.write().unwrap_or_else(|e| e.into_inner()).push(m);
}

/// The mapped loopback address for a resolver `host:port` key, or None (resolve normally).
pub fn lookup(netloc: &str) -> Option<IpAddr> {
    let host = netloc.rsplit_once(':').map_or(netloc, |(h, _)| h);
    // An IP literal (v4, or bracketed v6) is never a mapped name.
    if host.starts_with('[') || host.parse::<IpAddr>().is_ok() {
        return None;
    }
    MAPS.read().unwrap_or_else(|e| e.into_inner()).iter().find(|m| m.matches(host)).map(|m| m.ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> HostMap {
        HostMap::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn accepts_the_wpt_mapping_and_other_reserved_loopback_ones() {
        assert_eq!(ok("*.web-platform.test=127.0.0.1").suffix, "web-platform.test");
        ok("*.a.localhost=127.1.2.3");
        ok("*.foo.example=::1");
        ok("*.Web-Platform.TEST=127.0.0.1");
    }

    #[test]
    fn refuses_non_loopback_targets() {
        for t in ["8.8.8.8", "10.0.0.1", "192.168.1.1", "0.0.0.0", "169.254.169.254", "example.com", "localhost", "::", "fe80::1", "2001:db8::1", "127.0.0.1:80", ""] {
            assert!(HostMap::parse(&format!("*.web-platform.test={t}")).is_err(), "target {t:?} must be refused");
        }
    }

    #[test]
    fn refuses_anything_but_a_star_dot_reserved_suffix() {
        for s in [
            "*=127.0.0.1",
            "*.=127.0.0.1",
            "*.com=127.0.0.1",
            "*.test=127.0.0.1",
            "*.example.com=127.0.0.1",
            "*.google.com=127.0.0.1",
            "*.org=127.0.0.1",
            "web-platform.test=127.0.0.1",
            "www.web-platform.test=127.0.0.1",
            "*.127.0.0.1=127.0.0.1",
            "*..web-platform.test=127.0.0.1",
            "*.-a.test=127.0.0.1",
            "*.a b.test=127.0.0.1",
            "**.web-platform.test=127.0.0.1",
            "*.web-platform.test",
            "",
        ] {
            let r = HostMap::parse(s);
            assert!(r.is_err(), "{s:?} must be refused, got {r:?}");
        }
    }

    #[test]
    fn matches_only_the_suffix_and_its_subdomains() {
        let m = ok("*.web-platform.test=127.0.0.1");
        for h in ["web-platform.test", "www.web-platform.test", "WWW1.Web-Platform.Test", "a.b.web-platform.test", "web-platform.test."] {
            assert!(m.matches(h), "{h}");
        }
        for h in ["evil-web-platform.test", "web-platform.test.evil.com", "xweb-platform.test", "platform.test", "test", "example.com", "web-platform.testx", ".web-platform.test", ""] {
            assert!(!m.matches(h), "{h}");
        }
    }

    #[test]
    fn lookup_ignores_ip_literals_and_unmapped_hosts() {
        add(ok("*.lookup-unit.test=127.0.0.2"));
        assert_eq!(lookup("a.lookup-unit.test:8000"), Some("127.0.0.2".parse().unwrap()));
        assert_eq!(lookup("lookup-unit.test:80"), Some("127.0.0.2".parse().unwrap()));
        assert_eq!(lookup("example.com:443"), None);
        assert_eq!(lookup("127.0.0.1:8000"), None);
        assert_eq!(lookup("[::1]:8000"), None);
    }
}
