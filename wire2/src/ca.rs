//! The local CA (SPEC "Security requirements: CA only on demand") and the per-host leaf certificates
//! the proxy shows the browser for decrypted hosts.
//!
//! Nothing here touches the disk until [`Ca::ensure`] is called, which happens only when an http-layer
//! claim is registered (`decrypt::Policy::add_http_claim`). The first call loads `ca.pem` +
//! `ca-key.pem` from `--ca-dir`, or creates them: the key with mode 0600 (directory 0700), and a loud log
//! line. Leaves are minted in memory per host, signed by the CA, and cached as ready rustls server
//! configs; they are never written anywhere.

use std::collections::HashMap;
use std::io::Write as _;
use std::net::IpAddr;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType, SerialNumber,
};
use tokio_rustls::rustls;

pub const CERT_FILE: &str = "ca.pem";
pub const KEY_FILE: &str = "ca-key.pem";

/// Leaf validity. Leaves live only in memory and are re-minted after [`LEAF_REFRESH`].
const LEAF_DAYS: i64 = 30;
const LEAF_REFRESH: Duration = Duration::from_secs(7 * 86_400);
/// Upper bound on cached leaves; the cache is simply emptied when it is reached.
const LEAF_CACHE_MAX: usize = 4096;
const CA_YEARS: i64 = 10;

/// rcgen's issuer: a certificate object with the CA's subject and key (re-signed from the PEM on load;
/// only its subject, key identifier and key matter for signing leaves).
struct Issuer {
    cert: Certificate,
    key: KeyPair,
}

pub struct Ca {
    dir: Option<PathBuf>,
    issuer: OnceLock<Issuer>,
    /// Serialises the first load/create; held only during that one-time file work.
    init: Mutex<()>,
    leaves: Mutex<HashMap<String, (Instant, Arc<rustls::ServerConfig>)>>,
}

/// `--ca-dir`, or the spec default `$XDG_DATA_HOME/proxy` (`~/.local/share/proxy`).
pub fn default_dir() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d).join("proxy")),
        _ => std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(|h| PathBuf::from(h).join(".local/share/proxy")),
    }
}

impl Ca {
    pub fn new(dir: Option<PathBuf>) -> Ca {
        Ca {
            dir,
            issuer: OnceLock::new(),
            init: Mutex::new(()),
            leaves: Mutex::new(HashMap::new()),
        }
    }

    pub fn cert_path(&self) -> String {
        match &self.dir {
            Some(d) => d.join(CERT_FILE).display().to_string(),
            None => format!("<--ca-dir>/{CERT_FILE}"),
        }
    }

    /// Load or create the CA. Idempotent; only the first successful call touches the disk.
    pub fn ensure(&self) -> Result<(), String> {
        if self.issuer.get().is_some() {
            return Ok(());
        }
        let _g = self.init.lock().unwrap_or_else(|e| e.into_inner());
        if self.issuer.get().is_some() {
            return Ok(());
        }
        let dir = self
            .dir
            .as_deref()
            .ok_or("no CA directory: pass --ca-dir, or set XDG_DATA_HOME or HOME".to_string())?;
        let issuer = if dir.join(KEY_FILE).exists() || dir.join(CERT_FILE).exists() {
            let i = load(dir)?;
            crate::info!(
                "using the CA in {} (browsers trust {})",
                dir.display(),
                self.cert_path()
            );
            i
        } else {
            let i = create(dir)?;
            crate::warn!(
                "CREATED A NEW CA in {}: import {} into the browser to trust decrypted hosts. Its key {} \
                 (mode 0600) can sign certificates for any site; it never leaves that directory",
                dir.display(),
                dir.join(CERT_FILE).display(),
                dir.join(KEY_FILE).display()
            );
            i
        };
        let _ = self.issuer.set(issuer);
        Ok(())
    }

    /// The TLS server config the browser sees for `host` (normalised): a leaf for that one name, signed
    /// by the CA, ALPN `http/1.1` only. Minted on first use, then cached.
    pub fn server_config(&self, host: &str) -> Result<Arc<rustls::ServerConfig>, String> {
        {
            let leaves = self.leaves.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((t, cfg)) = leaves.get(host)
                && t.elapsed() < LEAF_REFRESH
            {
                return Ok(cfg.clone());
            }
        }
        self.ensure()?;
        let issuer = self.issuer.get().ok_or("CA not loaded")?;
        // Minted outside the lock: two concurrent first CONNECTs may both mint, the later insert wins.
        let cfg = Arc::new(mint(issuer, host)?);
        let mut leaves = self.leaves.lock().unwrap_or_else(|e| e.into_inner());
        if leaves.len() >= LEAF_CACHE_MAX {
            leaves.clear();
        }
        leaves.insert(host.to_string(), (Instant::now(), cfg.clone()));
        Ok(cfg)
    }
}

fn now() -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc()
}

/// A random positive 128-bit serial (browsers reject two different certificates with the same issuer
/// and serial).
fn random_serial() -> SerialNumber {
    let mut b = [0u8; 16];
    // SAFETY: getrandom fills our buffer.
    let n = unsafe { libc::getrandom(b.as_mut_ptr().cast(), b.len(), 0) };
    if n != b.len() as isize {
        let t = now().unix_timestamp_nanos().to_le_bytes();
        b.copy_from_slice(&t);
    }
    b[0] &= 0x7f;
    b[0] |= 0x01;
    SerialNumber::from_slice(&b)
}

fn ca_params() -> CertificateParams {
    let mut p = CertificateParams::default();
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    let mut dn = DistinguishedName::new();
    // A per-install suffix, so two installs never share a subject in a browser's trust store.
    let id: String = random_serial().as_ref()[..4]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    dn.push(DnType::CommonName, format!("wire2 local CA {id}"));
    dn.push(DnType::OrganizationName, "wire2 (local filtering proxy)");
    p.distinguished_name = dn;
    p.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    p.not_before = now() - time::Duration::days(1);
    p.not_after = now() + time::Duration::days(365 * CA_YEARS);
    p.serial_number = Some(random_serial());
    p
}

fn create(dir: &Path) -> Result<Issuer, String> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("creating CA directory {}: {e}", dir.display()))?;
    let key = KeyPair::generate().map_err(|e| format!("CA key: {e}"))?;
    let cert = ca_params()
        .self_signed(&key)
        .map_err(|e| format!("CA certificate: {e}"))?;
    // Key first (0600 from the moment it exists), then the certificate; each written to a temporary
    // name and renamed, so a crash never leaves a half-written file under the real name.
    write_file(&dir.join(KEY_FILE), key.serialize_pem().as_bytes(), 0o600)?;
    write_file(&dir.join(CERT_FILE), cert.pem().as_bytes(), 0o644)?;
    Ok(Issuer { cert, key })
}

fn write_file(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&tmp)
        .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    f.write_all(data)
        .and_then(|_| f.sync_all())
        .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("renaming to {}: {e}", path.display()))
}

fn load(dir: &Path) -> Result<Issuer, String> {
    let kp = dir.join(KEY_FILE);
    let cp = dir.join(CERT_FILE);
    let meta = std::fs::metadata(&kp).map_err(|e| {
        format!(
            "{}: {e} (remove {} to start over)",
            kp.display(),
            dir.display()
        )
    })?;
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "{} is readable by other users (mode {:o}); refusing to use it. chmod 600 it, or remove {} to \
             create a new CA",
            kp.display(),
            meta.permissions().mode() & 0o777,
            dir.display()
        ));
    }
    let key_pem = std::fs::read_to_string(&kp).map_err(|e| format!("{}: {e}", kp.display()))?;
    let cert_pem = std::fs::read_to_string(&cp).map_err(|e| {
        format!(
            "{}: {e} (remove {} to start over)",
            cp.display(),
            dir.display()
        )
    })?;
    let key = KeyPair::from_pem(&key_pem).map_err(|e| format!("{}: {e}", kp.display()))?;
    let params = CertificateParams::from_ca_cert_pem(&cert_pem)
        .map_err(|e| format!("{}: {e}", cp.display()))?;
    let cert = params
        .self_signed(&key)
        .map_err(|e| format!("{} does not match {}: {e}", cp.display(), kp.display()))?;
    Ok(Issuer { cert, key })
}

fn mint(issuer: &Issuer, host: &str) -> Result<rustls::ServerConfig, String> {
    let mut p = CertificateParams::default();
    p.subject_alt_names = vec![match host.parse::<IpAddr>() {
        Ok(ip) => SanType::IpAddress(ip),
        Err(_) => SanType::DnsName(
            host.to_string()
                .try_into()
                .map_err(|e| format!("leaf for {host}: {e}"))?,
        ),
    }];
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, host);
    p.distinguished_name = dn;
    p.is_ca = IsCa::ExplicitNoCa;
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    p.use_authority_key_identifier_extension = true;
    p.not_before = now() - time::Duration::days(1);
    p.not_after = now() + time::Duration::days(LEAF_DAYS);
    p.serial_number = Some(random_serial());
    let key = KeyPair::generate().map_err(|e| format!("leaf key: {e}"))?;
    let cert = p
        .signed_by(&key, &issuer.cert, &issuer.key)
        .map_err(|e| format!("leaf for {host}: {e}"))?;
    let der = rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into());
    let mut cfg = rustls::ServerConfig::builder_with_provider(crate::decrypt::provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], der)
        .map_err(|e| format!("leaf for {host}: {e}"))?;
    // SPEC: v1 speaks HTTP/1.1 to the browser for decrypted hosts.
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Creation, permissions and reload. The chain check (a browser-side client trusting only ca.pem
    // accepts the minted leaf, also after a reload) is end to end in tests/w2_2_decrypt.rs.
    #[test]
    fn creates_once_with_0600_key_and_reloads() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("nested/proxy");
        let ca = Ca::new(Some(dir.clone()));
        assert!(!dir.exists(), "nothing on disk before ensure");
        ca.ensure().unwrap();
        let key_mode = std::fs::metadata(dir.join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(key_mode & 0o777, 0o600);
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);
        let pem1 = std::fs::read_to_string(dir.join(CERT_FILE)).unwrap();
        ca.ensure().unwrap();
        ca.server_config("www.example.test").unwrap();
        ca.server_config("::1").unwrap();
        // A second proxy start reuses the same CA instead of creating a new one.
        let ca2 = Ca::new(Some(dir.clone()));
        ca2.ensure().unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(CERT_FILE)).unwrap(), pem1);
        ca2.server_config("www.example.test").unwrap();
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, vec![KEY_FILE, CERT_FILE]);
        // A world-readable key is refused.
        std::fs::set_permissions(dir.join(KEY_FILE), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let e = Ca::new(Some(dir)).ensure().unwrap_err();
        assert!(e.contains("readable by other users"), "{e}");
    }
}
