//! Flow registry and access log (SPEC "Flow lifecycle").
//!
//! A flow is one tunnel or one plain-http browser connection. The registry lock is held only to insert,
//! remove or snapshot entries, never across I/O. Dropping a [`Flow`] removes its entry and writes the
//! access-log line, so every flow logs exactly once however it ends.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::filter::Route;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A browser `CONNECT`, spliced as opaque bytes.
    Tunnel,
    /// A plain `http://` request in absolute form.
    Http,
}

/// Status recorded for the access log: an HTTP status, or 0 for "none yet" (e.g. the browser left).
pub const STATUS_NONE: u16 = 0;

pub struct FlowInfo {
    pub id: u64,
    pub kind: Kind,
    pub method: String,
    pub host: String,
    pub port: u16,
    /// A `CONNECT` whose TLS the proxy terminates (W2-2); decided once when the flow opens. Its byte
    /// counters then count plaintext HTTP (heads and bodies), not TLS records.
    pub decrypted: bool,
    /// Filter names the flow passes through, computed once when it opens.
    pub chain: Vec<String>,
    /// The filters themselves and the hub's per-layer expectation (SPEC "Traffic": `Proxy-Flow`).
    pub route: Route,
    pub opened: Instant,
    /// Bytes browser -> upstream.
    pub up: AtomicU64,
    /// Bytes upstream -> browser.
    pub down: AtomicU64,
    pub status: AtomicU16,
    /// Cancelling it closes every connection of the flow.
    pub cancel: CancellationToken,
}

impl FlowInfo {
    pub fn set_status(&self, code: u16) {
        self.status.store(code, Ordering::Relaxed);
    }

    fn chain_str(&self) -> String {
        if self.chain.is_empty() {
            "-".into()
        } else {
            self.chain.join(",")
        }
    }
}

#[derive(Default)]
pub struct Registry {
    next: AtomicU64,
    flows: Mutex<HashMap<u64, Arc<FlowInfo>>>,
}

impl Registry {
    pub fn new() -> Arc<Registry> {
        Arc::new(Registry::default())
    }

    pub fn open(
        self: &Arc<Self>,
        kind: Kind,
        method: &str,
        host: &str,
        port: u16,
        decrypted: bool,
        route: Route,
    ) -> Flow {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let info = Arc::new(FlowInfo {
            id,
            kind,
            method: method.to_string(),
            host: host.to_string(),
            port,
            decrypted,
            chain: route.names(),
            route,
            opened: Instant::now(),
            up: AtomicU64::new(0),
            down: AtomicU64::new(0),
            status: AtomicU16::new(STATUS_NONE),
            cancel: CancellationToken::new(),
        });
        self.lock().insert(id, info.clone());
        crate::debug!("flow={id} open {method} host={host} port={port}");
        Flow {
            reg: self.clone(),
            info,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<FlowInfo>>> {
        self.flows.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// An open flow by id (the hub's `Proxy-Flow` check).
    pub fn get(&self, id: u64) -> Option<Arc<FlowInfo>> {
        self.lock().get(&id).cloned()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn snapshot(&self) -> Vec<Arc<FlowInfo>> {
        let mut v: Vec<_> = self.lock().values().cloned().collect();
        v.sort_by_key(|f| f.id);
        v
    }

    /// SIGUSR1: one summary line, then one line per open flow.
    pub fn dump(&self) {
        let flows = self.snapshot();
        crate::info!(
            "registry: {} open flow(s), {} pooled pipe(s)",
            flows.len(),
            crate::splice::pooled_pipes()
        );
        for f in flows {
            crate::info!(
                "registry flow={} {} host={} port={} decrypted={} chain={} age={} up={} down={}",
                f.id,
                f.method,
                f.host,
                f.port,
                if f.decrypted { "yes" } else { "no" },
                f.chain_str(),
                fmt_secs(f.opened.elapsed()),
                f.up.load(Ordering::Relaxed),
                f.down.load(Ordering::Relaxed),
            );
        }
    }

    /// Cancel every open flow (shutdown).
    pub fn cancel_all(&self) {
        for f in self.snapshot() {
            f.cancel.cancel();
        }
    }
}

pub struct Flow {
    reg: Arc<Registry>,
    pub info: Arc<FlowInfo>,
}

impl std::ops::Deref for Flow {
    type Target = FlowInfo;
    fn deref(&self) -> &FlowInfo {
        &self.info
    }
}

impl Drop for Flow {
    fn drop(&mut self) {
        self.reg.lock().remove(&self.info.id);
        let f = &self.info;
        let status = match (f.kind, f.status.load(Ordering::Relaxed)) {
            (_, STATUS_NONE) => "-".to_string(),
            (Kind::Tunnel, 200) => "tunnel".to_string(),
            (_, code) => code.to_string(),
        };
        let mut line = String::new();
        let _ = write!(
            line,
            "access flow={} {} host={} port={} decrypted={} chain={} status={} up={} down={} duration={}",
            f.id,
            f.method,
            f.host,
            f.port,
            if f.decrypted { "yes" } else { "no" },
            f.chain_str(),
            status,
            f.up.load(Ordering::Relaxed),
            f.down.load(Ordering::Relaxed),
            fmt_secs(f.opened.elapsed()),
        );
        crate::info!("{line}");
    }
}

pub fn fmt_secs(d: Duration) -> String {
    format!("{:.3}s", d.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_close_and_ids() {
        let r = Registry::new();
        let a = r.open(
            Kind::Tunnel,
            "CONNECT",
            "a.test",
            443,
            false,
            Route::default(),
        );
        let b = r.open(Kind::Http, "GET", "b.test", 80, false, Route::default());
        assert_eq!(r.get(2).map(|f| f.host.clone()), Some("b.test".into()));
        assert!(r.get(3).is_none());
        assert_eq!((a.id, b.id), (1, 2));
        assert_eq!(r.len(), 2);
        b.up.fetch_add(5, Ordering::Relaxed);
        assert_eq!(r.snapshot()[1].up.load(Ordering::Relaxed), 5);
        drop(a);
        assert_eq!(r.len(), 1);
        r.cancel_all();
        assert!(b.cancel.is_cancelled());
        drop(b);
        assert_eq!(r.len(), 0);
    }
}
