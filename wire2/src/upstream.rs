//! Upstream connector: resolves the host itself (browsers send names to a proxy) and dials it.
//!
//! In W2-F this becomes the place that first sends `CONNECT host:port` through the tunnel filters that
//! claim the host; in W2-2 it also opens verified TLS. Callers only see [`dial`] and [`DialError`].

use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::net::TcpStream;

#[derive(Debug)]
pub enum DialError {
    /// The name did not resolve.
    Dns(String),
    /// Every address refused (or was unreachable).
    Connect(String),
    /// No connection within `--dial-timeout`.
    Timeout(Duration),
}

impl DialError {
    /// SPEC: DNS failure or refused connection -> 502, dial timeout -> 504.
    pub fn status(&self) -> u16 {
        match self {
            DialError::Timeout(_) => 504,
            _ => 502,
        }
    }
}

impl fmt::Display for DialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialError::Dns(e) => write!(f, "dns: {e}"),
            DialError::Connect(e) => write!(f, "connect: {e}"),
            DialError::Timeout(d) => write!(f, "dial timeout after {}ms", d.as_millis()),
        }
    }
}

/// Resolve `host` (already normalised) and connect to the first address that answers, all within `timeout`.
pub async fn dial(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, DialError> {
    match tokio::time::timeout(timeout, dial_inner(host, port)).await {
        Ok(r) => r,
        Err(_) => Err(DialError::Timeout(timeout)),
    }
}

async fn dial_inner(host: &str, port: u16) -> Result<TcpStream, DialError> {
    let addrs: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| DialError::Dns(e.to_string()))?
            .collect(),
    };
    if addrs.is_empty() {
        return Err(DialError::Dns("no addresses".into()));
    }
    connect_any(host, port, interleave_families(addrs)).await
}

async fn connect_any(
    host: &str,
    port: u16,
    addrs: Vec<SocketAddr>,
) -> Result<TcpStream, DialError> {
    let n = addrs.len();
    // Happy Eyeballs (RFC 8305, simplified): start the next address when the current attempts fail or
    // after a short delay, first connection wins. A blackholed IPv6 route then costs 250 ms, not the
    // whole --dial-timeout.
    let mut attempts = tokio::task::JoinSet::new();
    let mut next = 0;
    let mut last: Option<io::Error> = None;
    loop {
        if next < n {
            let addr = addrs[next];
            attempts.spawn(async move { (addr, TcpStream::connect(addr).await) });
            next += 1;
        }
        let joined = if next < n {
            tokio::select! {
                j = attempts.join_next() => j,
                _ = tokio::time::sleep(ATTEMPT_DELAY) => continue,
            }
        } else {
            attempts.join_next().await
        };
        match joined {
            None => break,
            Some(Ok((addr, Ok(s)))) => {
                let _ = s.set_nodelay(true);
                crate::debug!("dialled {host}:{port} at {addr}");
                return Ok(s);
            }
            Some(Ok((addr, Err(e)))) => {
                crate::debug!("dial {host}:{port} at {addr}: {e}");
                last = Some(e);
            }
            Some(Err(e)) => last = Some(io::Error::other(e.to_string())),
        }
        if next >= n && attempts.is_empty() {
            break;
        }
    }
    let e = last.map(|e| e.to_string()).unwrap_or_default();
    Err(DialError::Connect(if n > 1 {
        format!("{e} (all {n} addresses)")
    } else {
        e
    }))
}

/// Delay before starting a connection attempt to the next address (RFC 8305 suggests 250 ms).
const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// Alternate address families, keeping the resolver's order within each family and its first choice first.
fn interleave_families(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let first_v6 = addrs[0].is_ipv6();
    let (mut a, mut b): (Vec<_>, Vec<_>) = addrs.into_iter().partition(|x| x.is_ipv6() == first_v6);
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut a, mut b) = (a.drain(..), b.drain(..));
    loop {
        match (a.next(), b.next()) {
            (None, None) => return out,
            (x, y) => out.extend(x.into_iter().chain(y)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaves() {
        let v6a: SocketAddr = "[2001:db8::1]:443".parse().unwrap();
        let v6b: SocketAddr = "[2001:db8::2]:443".parse().unwrap();
        let v4a: SocketAddr = "192.0.2.1:443".parse().unwrap();
        let v4b: SocketAddr = "192.0.2.2:443".parse().unwrap();
        assert_eq!(
            interleave_families(vec![v6a, v6b, v4a, v4b]),
            vec![v6a, v4a, v6b, v4b]
        );
        assert_eq!(
            interleave_families(vec![v4a, v6a, v6b]),
            vec![v4a, v6a, v6b]
        );
        assert_eq!(interleave_families(vec![v4a]), vec![v4a]);
    }

    /// A listener with a full accept queue: connects to it hang (the kernel drops the SYNs).
    fn hanging_addr() -> Option<(SocketAddr, std::net::TcpListener, Vec<std::net::TcpStream>)> {
        use std::os::fd::FromRawFd;
        // SAFETY: plain socket calls on a fresh fd that the returned TcpListener then owns.
        let l = unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
            let mut sa: libc::sockaddr_in = std::mem::zeroed();
            sa.sin_family = libc::AF_INET as libc::sa_family_t;
            sa.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
            let len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
            assert_eq!(
                libc::bind(fd, (&sa as *const libc::sockaddr_in).cast(), len),
                0
            );
            assert_eq!(libc::listen(fd, 0), 0);
            std::net::TcpListener::from_raw_fd(fd)
        };
        let addr = l.local_addr().unwrap();
        let mut held = Vec::new();
        for _ in 0..16 {
            match std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(300)) {
                Ok(s) => held.push(s),
                Err(_) => return Some((addr, l, held)),
            }
        }
        None
    }

    /// Happy Eyeballs: an address that never answers must not hold up a working one behind it, and a
    /// working first address wins without waiting.
    #[tokio::test]
    async fn hanging_first_address_does_not_block() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good = l.local_addr().unwrap();
        let Some((dead, _l, _held)) = hanging_addr() else {
            eprintln!("SKIP: could not fill an accept queue");
            return;
        };
        let t = std::time::Instant::now();
        let r = tokio::time::timeout(
            Duration::from_secs(3),
            connect_any("test", 0, vec![dead, good]),
        )
        .await
        .expect("dial hung");
        assert_eq!(r.unwrap().peer_addr().unwrap(), good);
        let took = t.elapsed();
        assert!(
            took >= ATTEMPT_DELAY && took < Duration::from_secs(1),
            "{took:?}"
        );
        let t = std::time::Instant::now();
        let r = connect_any("test", 0, vec![good, dead]).await;
        assert_eq!(r.unwrap().peer_addr().unwrap(), good);
        assert!(t.elapsed() < ATTEMPT_DELAY);
        let refused = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let e = connect_any("test", 0, vec![refused]).await.unwrap_err();
        assert!(matches!(e, DialError::Connect(_)), "{e}");
    }
}
