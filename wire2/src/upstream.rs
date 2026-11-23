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
    let mut last: Option<io::Error> = None;
    for addr in &addrs {
        match TcpStream::connect(addr).await {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                crate::debug!("dialled {host}:{port} at {addr}");
                return Ok(s);
            }
            Err(e) => {
                crate::debug!("dial {host}:{port} at {addr}: {e}");
                last = Some(e);
            }
        }
    }
    let e = last.map(|e| e.to_string()).unwrap_or_default();
    Err(DialError::Connect(if addrs.len() > 1 {
        format!("{e} (all {} addresses)", addrs.len())
    } else {
        e
    }))
}
