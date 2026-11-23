//! W2-2e: the pool of idle upstream connections of decrypted flows, shared by all of the browser's
//! connections to the same origin.
//!
//! A connection is parked after a response whose end was visible and which did not ask for a close
//! (W2-2a's rules, unchanged), and taken by the next request of ANY decrypted flow with the same key:
//! host, port and the tunnel-filter chain that carries it. The chain is part of the key because a tunnel
//! filter decided over one CONNECT for the whole connection; a connection that went through chain X is
//! never handed to a flow whose chain is Y. Each upstream connection still went through exactly one
//! CONNECT per tunnel filter, and a filter closing its pipe ends it (the next take sees the close and
//! dials again, through the filters again).
//!
//! Limits: an idle expiry of its own (`--pool-idle`, default 30 s), at most `--pool-per-key` idle
//! connections per key (default 6, what a browser opens per host) and `--pool-total` overall (default
//! 64); over a cap the oldest idle connection is closed. Taking is LIFO (the freshest connection is the
//! likeliest to be alive). The lock is held only to move entries, never across I/O.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio_util::sync::DropGuard;

use crate::upstream::Pooled;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub host: String,
    pub port: u16,
    /// Names of the tunnel filters on the way, in order.
    pub chain: Vec<String>,
}

/// An idle connection. Dropping it drops the connection; `guard` ends the hub's pipe behind it then.
pub struct Entry {
    pub conn: Pooled,
    pub guard: Option<DropGuard>,
    since: Instant,
}

impl Entry {
    pub fn new(conn: Pooled, guard: Option<DropGuard>) -> Entry {
        Entry {
            conn,
            guard,
            since: Instant::now(),
        }
    }
}

#[derive(Default)]
struct State {
    map: HashMap<Key, Vec<Entry>>,
    n: usize,
}

pub struct UpPool {
    st: Mutex<State>,
    idle: Duration,
    per_key: usize,
    total: usize,
}

impl UpPool {
    /// Must be called inside a tokio runtime (it starts the expiry task).
    pub fn new(idle: Duration, per_key: usize, total: usize) -> Arc<UpPool> {
        let p = Arc::new(UpPool {
            st: Mutex::new(State::default()),
            idle,
            per_key,
            total,
        });
        if !idle.is_zero() {
            let tick = (idle / 5).clamp(Duration::from_millis(10), Duration::from_secs(2));
            tokio::spawn(janitor(Arc::downgrade(&p), tick));
        }
        p
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Idle connections now (tests, debug).
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.lock().n
    }

    /// SIGUSR1: one summary line (totals and caps), then one line per key with its parked
    /// connections' idle ages, oldest first.
    pub fn dump(&self) {
        let mut keys: Vec<(Key, Vec<Duration>)> = {
            let st = self.lock();
            st.map
                .iter()
                .map(|(k, v)| (k.clone(), v.iter().map(|e| e.since.elapsed()).collect()))
                .collect()
        };
        keys.sort_by(|a, b| {
            (&a.0.host, a.0.port, &a.0.chain).cmp(&(&b.0.host, b.0.port, &b.0.chain))
        });
        let n: usize = keys.iter().map(|(_, v)| v.len()).sum();
        crate::info!(
            "pool: {n} idle upstream connection(s) in {} key(s), caps {} per key and {} total, idle expiry {}",
            keys.len(),
            self.per_key,
            self.total,
            crate::flow::fmt_secs(self.idle)
        );
        for (k, ages) in keys {
            let chain = if k.chain.is_empty() {
                "-".to_string()
            } else {
                k.chain.join(",")
            };
            let ages: Vec<String> = ages.into_iter().map(crate::flow::fmt_secs).collect();
            crate::info!(
                "pool host={} port={} chain={chain} idle={} ages={}",
                k.host,
                k.port,
                ages.len(),
                ages.join(",")
            );
        }
    }

    /// The freshest idle connection for `key`, if any (an expired one is closed instead). The caller
    /// checks that it is still alive.
    pub fn take(&self, key: &Key) -> Option<Entry> {
        let e = {
            let mut st = self.lock();
            let v = st.map.get_mut(key)?;
            let e = v.pop();
            if v.is_empty() {
                st.map.remove(key);
            }
            if e.is_some() {
                st.n -= 1;
            }
            e?
        };
        if e.since.elapsed() >= self.idle {
            close(e);
            return None;
        }
        Some(e)
    }

    /// Park a connection. Over the per-key or the overall cap, the oldest idle connection goes.
    pub fn put(&self, key: Key, e: Entry) {
        if self.idle.is_zero() || self.per_key == 0 || self.total == 0 {
            close(e);
            return;
        }
        let mut dead = Vec::new();
        {
            let mut st = self.lock();
            let v = st.map.entry(key).or_default();
            v.push(e);
            let mut added = 1usize;
            while v.len() > self.per_key {
                dead.push(v.remove(0));
                added -= 1;
            }
            st.n += added;
            while st.n > self.total {
                // The globally oldest: the first entry of some key's list.
                let oldest = st
                    .map
                    .iter()
                    .filter_map(|(k, v)| v.first().map(|e| (e.since, k.clone())))
                    .min_by_key(|(t, _)| *t)
                    .map(|(_, k)| k);
                let Some(k) = oldest else { break };
                let v = st.map.get_mut(&k).expect("key");
                dead.push(v.remove(0));
                if v.is_empty() {
                    st.map.remove(&k);
                }
                st.n -= 1;
            }
        }
        for e in dead {
            close(e);
        }
    }

    /// Close every idle connection that has waited `idle` or longer. Returns how many.
    pub fn sweep(&self) -> usize {
        let mut dead = Vec::new();
        {
            let mut st = self.lock();
            let idle = self.idle;
            st.map.retain(|_, v| {
                let (old, keep): (Vec<_>, Vec<_>) = std::mem::take(v)
                    .into_iter()
                    .partition(|e| e.since.elapsed() >= idle);
                dead.extend(old);
                *v = keep;
                !v.is_empty()
            });
            st.n -= dead.len();
        }
        let n = dead.len();
        for e in dead {
            close(e);
        }
        n
    }
}

async fn janitor(pool: Weak<UpPool>, tick: Duration) {
    loop {
        tokio::time::sleep(tick).await;
        let Some(p) = pool.upgrade() else { return };
        let n = p.sweep();
        if n > 0 {
            crate::debug!(
                "pool: closed {n} idle upstream connection(s) after {:?}",
                p.idle
            );
        }
    }
}

/// Close an idle connection cleanly (TLS close_notify, then the FIN) without making the caller wait;
/// dropping it afterwards also ends the hub's pipe behind a tunnel filter.
fn close(e: Entry) {
    let Entry { conn, guard, .. } = e;
    let (r, mut w) = conn;
    match tokio::runtime::Handle::try_current() {
        Ok(h) => {
            h.spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(2), w.shutdown()).await;
                drop((r, w, guard));
            });
        }
        Err(_) => drop((r, w, guard)),
    }
}
