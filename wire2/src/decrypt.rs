//! The decrypt decision (SPEC "Routing and decryption: Lazy decryption") and the TLS settings of both
//! sides of a decrypted tunnel.
//!
//! One question per CONNECT, answered by [`Policy::decide`]: does a connected http-layer filter claim
//! this host, and is the host not on `--no-decrypt`? Claims are registered with
//! [`Policy::add_http_claim`], which W2-F calls when an http filter completes its handshake; that call
//! is also what creates the CA (never earlier). Until W2-F nothing registers a claim, so every CONNECT
//! stays an opaque tunnel and no key material exists. The TEST-ONLY `--test-decrypt GLOB` flag
//! registers a stand-in claim at startup (see NOTES.md).

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use tokio_rustls::rustls;

use crate::ca::Ca;
use crate::host::glob_match;

/// An http-layer filter's claim: the hosts it wants to see in plaintext.
struct Claim {
    filter: String,
    hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Opaque tunnel (spliced), nothing decrypted.
    Tunnel,
    /// Terminate TLS; `by` names the first claiming filter (for the log).
    Decrypt { by: String },
}

pub struct Policy {
    no_decrypt: Vec<String>,
    claims: RwLock<Vec<Claim>>,
    pub ca: Ca,
    /// Hosts already logged as "decrypting for the first time" / "never decrypted".
    first_decrypt: Mutex<HashSet<String>>,
    first_skip: Mutex<HashSet<String>>,
}

impl Policy {
    pub fn new(no_decrypt: Vec<String>, ca: Ca) -> Policy {
        Policy {
            no_decrypt,
            claims: RwLock::new(Vec::new()),
            ca,
            first_decrypt: Mutex::new(HashSet::new()),
            first_skip: Mutex::new(HashSet::new()),
        }
    }

    /// An http-layer filter completed its handshake claiming `hosts` (normalised globs). Creates the CA
    /// first if this is the first claim (SPEC: "The CA is created the first time an http filter
    /// completes its handshake"). New tunnels only: open ones keep the decision they were made with.
    pub fn add_http_claim(&self, filter: &str, hosts: Vec<String>) -> Result<(), String> {
        self.ca.ensure()?;
        crate::info!(
            "http-layer claim by {filter}: decrypting {} (except --no-decrypt {})",
            hosts.join(","),
            if self.no_decrypt.is_empty() {
                "-".to_string()
            } else {
                self.no_decrypt.join(",")
            }
        );
        let mut claims = self.claims.write().unwrap_or_else(|e| e.into_inner());
        claims.retain(|c| c.filter != filter);
        claims.push(Claim {
            filter: filter.to_string(),
            hosts,
        });
        Ok(())
    }

    /// The one decrypt decision per CONNECT, on the normalised host (no port).
    pub fn decide(&self, host: &str) -> Decision {
        let by = {
            let claims = self.claims.read().unwrap_or_else(|e| e.into_inner());
            claims
                .iter()
                .find(|c| c.hosts.iter().any(|g| glob_match(g, host)))
                .map(|c| c.filter.clone())
        };
        let Some(by) = by else {
            return Decision::Tunnel;
        };
        if let Some(g) = self.no_decrypt.iter().find(|g| glob_match(g, host)) {
            if once(&self.first_skip, host) {
                crate::info!(
                    "never decrypting {host}: on --no-decrypt {g} (http filter {by} skipped for it)"
                );
            }
            return Decision::Tunnel;
        }
        if once(&self.first_decrypt, host) {
            crate::warn!("decrypting {host} for the first time (claimed by {by})");
        }
        Decision::Decrypt { by }
    }
}

/// True the first time `host` is seen in `set`.
fn once(set: &Mutex<HashSet<String>>, host: &str) -> bool {
    let mut s = set.lock().unwrap_or_else(|e| e.into_inner());
    if s.contains(host) {
        false
    } else {
        s.insert(host.to_string())
    }
}

/// The crypto provider for both TLS sides (ring: no C toolchain beyond what `ring` builds itself).
pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    static P: OnceLock<Arc<rustls::crypto::CryptoProvider>> = OnceLock::new();
    P.get_or_init(|| Arc::new(rustls::crypto::ring::default_provider()))
        .clone()
}

/// TLS client config for the upstream side: verified against the system roots as found by
/// `rustls-native-certs` (`SSL_CERT_FILE` / `SSL_CERT_DIR` when set, else the distribution's store,
/// e.g. /etc/pki/tls or /etc/ssl/certs), ALPN `http/1.1`. Built once, on the first decrypted request.
pub fn upstream_config() -> Result<Arc<rustls::ClientConfig>, String> {
    static C: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    C.get_or_init(|| {
        let found = rustls_native_certs::load_native_certs();
        for e in &found.errors {
            crate::warn!("system roots: {e}");
        }
        let mut roots = rustls::RootCertStore::empty();
        let (added, ignored) = roots.add_parsable_certificates(found.certs);
        if added == 0 {
            return Err("no usable system root certificates found".to_string());
        }
        crate::info!("upstream TLS: {added} system root(s) loaded ({ignored} unparsable skipped)");
        let mut cfg = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(cfg))
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_claims_and_no_decrypt() {
        let d = tempfile::tempdir().unwrap();
        let p = Policy::new(
            vec!["bank.example.test".into(), "*.pinned.example.test".into()],
            Ca::new(Some(d.path().join("ca"))),
        );
        // No claim: always a tunnel, and no CA on disk.
        assert_eq!(p.decide("www.example.test"), Decision::Tunnel);
        assert!(!d.path().join("ca").exists());
        p.add_http_claim("pass-decrypt", vec!["*.example.test".into()])
            .unwrap();
        assert!(d.path().join("ca").join(crate::ca::CERT_FILE).exists());
        let yes = Decision::Decrypt {
            by: "pass-decrypt".into(),
        };
        assert_eq!(p.decide("www.example.test"), yes);
        assert_eq!(p.decide("a.b.example.test"), yes);
        // `*.example.test` does not match example.test itself.
        assert_eq!(p.decide("example.test"), Decision::Tunnel);
        assert_eq!(p.decide("other.test"), Decision::Tunnel);
        // --no-decrypt wins over the claim.
        assert_eq!(p.decide("bank.example.test"), Decision::Tunnel);
        assert_eq!(p.decide("app.pinned.example.test"), Decision::Tunnel);
        // ... but `*.pinned.example.test` does not cover pinned.example.test.
        assert_eq!(p.decide("pinned.example.test"), yes);
    }
}
