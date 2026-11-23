//! Host normalisation (SPEC "Routing and decryption"): lowercase, no trailing dot, punycode for
//! internationalised names, IPv6 literals without brackets. Logs use the normalised host today; routing
//! and `--no-decrypt` matching (W2-F, W2-2) will use the same function.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError(pub String);

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Normalise a host as it appears in an authority (`[::1]`, `Example.COM.`, `bücher.de`).
pub fn normalise_host(raw: &str) -> Result<String, HostError> {
    let bad = |why: &str| HostError(format!("bad host {raw:?}: {why}"));
    if raw.is_empty() {
        return Err(bad("empty"));
    }
    if let Some(inner) = raw.strip_prefix('[') {
        let inner = inner
            .strip_suffix(']')
            .ok_or_else(|| bad("unterminated IPv6 literal"))?;
        // A zone id (fe80::1%eth0) is meaningless across hosts; reject rather than guess.
        let ip: Ipv6Addr = inner.parse().map_err(|_| bad("not an IPv6 address"))?;
        return Ok(ip.to_string());
    }
    if raw.contains(':') {
        // Only reachable for a bare IPv6 literal (authorities are split before this).
        let ip: Ipv6Addr = raw
            .parse()
            .map_err(|_| bad("':' outside an IPv6 literal"))?;
        return Ok(ip.to_string());
    }
    let name = raw.strip_suffix('.').unwrap_or(raw);
    if name.is_empty() || name.ends_with('.') {
        return Err(bad("empty label"));
    }
    if let Ok(ip) = name.parse::<Ipv4Addr>() {
        return Ok(ip.to_string());
    }
    // UTS 46 processing: case folding, mapping, punycode.
    let ascii = idna::domain_to_ascii(name).map_err(|_| bad("not a valid domain name"))?;
    if ascii.is_empty()
        || ascii.split('.').any(str::is_empty)
        || !ascii.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.' || b == b'_'
        })
    {
        return Err(bad("not a valid domain name"));
    }
    Ok(ascii)
}

/// Split `host:port` / `[v6]:port` / `host` into the raw host and the optional port.
pub fn split_authority(auth: &str) -> Result<(&str, Option<u16>), HostError> {
    let bad = |why: &str| HostError(format!("bad authority {auth:?}: {why}"));
    let (host, port) = if auth.starts_with('[') {
        let end = auth
            .find(']')
            .ok_or_else(|| bad("unterminated IPv6 literal"))?;
        let (h, rest) = auth.split_at(end + 1);
        match rest {
            "" => (h, None),
            _ => (
                h,
                Some(
                    rest.strip_prefix(':')
                        .ok_or_else(|| bad("junk after IPv6 literal"))?,
                ),
            ),
        }
    } else {
        match auth.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h, Some(p)),
            Some(_) => return Err(bad("IPv6 address without brackets")),
            None => (auth, None),
        }
    };
    if host.is_empty() {
        return Err(bad("empty host"));
    }
    let port = match port {
        None => None,
        Some(p) => {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad("bad port"));
            }
            match p.parse::<u16>() {
                Ok(n) if n != 0 => Some(n),
                _ => return Err(bad("bad port")),
            }
        }
    };
    Ok((host, port))
}

/// Host as written back into an authority or a Host header: IPv6 literals get their brackets again.
pub fn authority_host(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_names() {
        for raw in [
            "www.example.test",
            "WWW.Example.Test",
            "www.example.test.",
            "WWW.EXAMPLE.TEST.",
        ] {
            assert_eq!(normalise_host(raw).unwrap(), "www.example.test", "{raw}");
        }
        assert_eq!(
            normalise_host("bücher.example").unwrap(),
            "xn--bcher-kva.example"
        );
        assert_eq!(
            normalise_host("BÜCHER.example.").unwrap(),
            "xn--bcher-kva.example"
        );
        assert_eq!(
            normalise_host("xn--bcher-kva.example").unwrap(),
            "xn--bcher-kva.example"
        );
        assert_eq!(
            normalise_host("例え.テスト").unwrap(),
            "xn--r8jz45g.xn--zckzah"
        );
        assert_eq!(
            normalise_host("_dmarc.example.test").unwrap(),
            "_dmarc.example.test"
        );
    }

    #[test]
    fn normalises_ips() {
        assert_eq!(normalise_host("[::1]").unwrap(), "::1");
        assert_eq!(normalise_host("[0:0:0:0:0:0:0:1]").unwrap(), "::1");
        assert_eq!(normalise_host("[2001:DB8::A]").unwrap(), "2001:db8::a");
        assert_eq!(normalise_host("127.0.0.1").unwrap(), "127.0.0.1");
        assert_eq!(normalise_host("127.0.0.1.").unwrap(), "127.0.0.1");
    }

    #[test]
    fn rejects_garbage() {
        for raw in [
            "", ".", "a..b", "[::1", "[nope]", "a b", "a/b", "a@b", "::1x",
        ] {
            assert!(normalise_host(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn splits_authorities() {
        assert_eq!(split_authority("[::1]:443").unwrap(), ("[::1]", Some(443)));
        assert_eq!(split_authority("[::1]").unwrap(), ("[::1]", None));
        assert_eq!(
            split_authority("Example.COM:8080").unwrap(),
            ("Example.COM", Some(8080))
        );
        assert_eq!(
            split_authority("example.com").unwrap(),
            ("example.com", None)
        );
        for bad in [
            "::1:443", "[::1]443", "a:", "a:0", "a:65536", "a:+1", ":80", "[::1]:x",
        ] {
            assert!(split_authority(bad).is_err(), "{bad}");
        }
        // CONNECT [::1]:443 parses as host ::1, port 443 (SPEC acceptance criterion).
        let (h, p) = split_authority("[::1]:443").unwrap();
        assert_eq!((normalise_host(h).unwrap().as_str(), p), ("::1", Some(443)));
    }

    #[test]
    fn authority_host_brackets_v6() {
        assert_eq!(authority_host("::1"), "[::1]");
        assert_eq!(authority_host("example.com"), "example.com");
    }
}
