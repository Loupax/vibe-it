//! The proxy's startup and accept loop (the `wire2` binary's `main`).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, UnixListener};
use tokio::signal::unix::{SignalKind, signal};

use crate::config::{self, Config, Listen, Parsed};
use crate::conn::Conn;
use crate::filter::{self, Filters, State};
use crate::flow::Registry;
use crate::proxy::{self, Ctx};
use crate::{ca, decrypt, error, hub, info, log, upstream, warn};

pub fn main() -> ExitCode {
    let cfg = match config::parse_args(std::env::args().skip(1)) {
        Ok(Parsed::Run(c)) => *c,
        Ok(Parsed::Help) => {
            print!("{}", config::USAGE);
            return ExitCode::SUCCESS;
        }
        Ok(Parsed::Version) => {
            println!("wire2 {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("wire2: {e}\n\n{}", config::USAGE);
            return ExitCode::from(2);
        }
    };
    log::set_level(cfg.log_level);
    // SAFETY: plain getters.
    if unsafe { libc::geteuid() } == 0 || unsafe { libc::getuid() } == 0 {
        error!("refusing to run as root (uid 0); run wire2 as an unprivileged user");
        return ExitCode::from(1);
    }
    raise_fd_limit();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!("cannot start the runtime: {e}");
            return ExitCode::from(1);
        }
    };
    match rt.block_on(run(cfg)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e}");
            ExitCode::from(1)
        }
    }
}

/// Thousands of flows need two fds each: lift the soft RLIMIT_NOFILE to the hard limit.
fn raise_fd_limit() {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit/setrlimit with a valid struct.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) == 0 && lim.rlim_cur < lim.rlim_max {
            lim.rlim_cur = lim.rlim_max;
            libc::setrlimit(libc::RLIMIT_NOFILE, &lim);
        }
    }
}

/// Random per-start instance id (SPEC: `Proxy-Id`, and the first half of every `Proxy-Flow`).
fn proxy_id() -> String {
    let mut b = [0u8; 6];
    // SAFETY: getrandom fills our buffer.
    let n = unsafe { libc::getrandom(b.as_mut_ptr().cast(), b.len(), 0) };
    if n != b.len() as isize {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        b.copy_from_slice(&(t.as_nanos() as u64 ^ std::process::id() as u64).to_le_bytes()[..6]);
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

enum Listener {
    Tcp(TcpListener),
    Unix(UnixListener),
}

/// `--hub`, or the spec default `$XDG_RUNTIME_DIR/proxy-hub.sock`.
fn hub_path(cfg: &Config) -> Result<PathBuf, String> {
    if let Some(h) = &cfg.hub {
        return Ok(h.clone());
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) if !d.is_empty() => Ok(PathBuf::from(d).join("proxy-hub.sock")),
        _ => Err(
            "filters need a hub socket: XDG_RUNTIME_DIR is not set, so pass --hub <path>".into(),
        ),
    }
}

async fn run(cfg: Config) -> Result<(), String> {
    // Socket trust (SPEC "Security requirements") is a startup error for a filter whose directory
    // exists and is not ours alone; a missing directory is only a filter that is down (for now).
    for f in &cfg.filters {
        if f.parent().is_some_and(|d| d.exists()) {
            filter::check_socket_dir(f)?;
        }
    }
    let listener = match &cfg.listen {
        Listen::Tcp(addr) => Listener::Tcp(
            TcpListener::bind(addr)
                .await
                .map_err(|e| format!("--listen {addr}: {e}"))?,
        ),
        Listen::Unix(path) => Listener::Unix(crate::conn::bind_unix("--listen unix:", path)?),
    };
    let shown = match &listener {
        Listener::Tcp(l) => l.local_addr().map(|a| a.to_string()).unwrap_or_default(),
        Listener::Unix(_) => match &cfg.listen {
            Listen::Unix(p) => format!("unix:{}", p.display()),
            Listen::Tcp(_) => unreachable!(),
        },
    };
    let ca_dir = cfg.ca_dir.clone().or_else(ca::default_dir);
    let policy = Arc::new(decrypt::Policy::new(
        cfg.no_decrypt.clone(),
        ca::Ca::new(ca_dir),
    ));
    if !cfg.test_resolve.is_empty() {
        let shown: Vec<String> = cfg
            .test_resolve
            .iter()
            .map(|(g, ip)| format!("{g}={ip}"))
            .collect();
        warn!(
            "TEST-ONLY --test-resolve: {} (DNS is not asked for these)",
            shown.join(",")
        );
        upstream::set_test_resolve(cfg.test_resolve.clone());
    }
    let id = proxy_id();
    let (hub_listener, hub) = if cfg.filters.is_empty() {
        (None, PathBuf::new())
    } else {
        let p = hub_path(&cfg)?;
        (Some(hub::bind(&p)?), p)
    };
    let filters = Arc::new(Filters::new(
        &cfg.filters,
        id.clone(),
        hub.clone(),
        cfg.connect_timeout,
        cfg.stage_timeout,
        cfg.filter_check,
    ));
    let ctx = Arc::new(Ctx {
        cfg: cfg.clone(),
        reg: Registry::new(),
        policy: policy.clone(),
        filters: filters.clone(),
        pool: crate::pool::UpPool::new(cfg.pool_idle, cfg.pool_per_key, cfg.pool_total),
    });
    let mode = if cfg.filters.is_empty() {
        "no filters: CONNECT is an opaque tunnel, nothing is decrypted, no CA".to_string()
    } else {
        let list: Vec<String> = cfg
            .filters
            .iter()
            .map(|p| format!("unix:{}", p.display()))
            .collect();
        format!(
            "{} filter(s) in chain order: {}; hub unix:{}",
            list.len(),
            list.join(" "),
            hub.display()
        )
    };
    info!(
        "wire2 {} proxy-id={id} listening on {shown} ({mode})",
        env!("CARGO_PKG_VERSION"),
    );
    if cfg.allow_remote {
        warn!("--allow-remote: anyone who can reach {shown} can use this proxy");
    }
    if let Some(l) = hub_listener {
        tokio::spawn(hub::serve(l, ctx.clone()));
        filters.start(policy);
    }

    let mut usr1 =
        signal(SignalKind::user_defined1()).map_err(|e| format!("SIGUSR1 handler: {e}"))?;
    let mut term = signal(SignalKind::terminate()).map_err(|e| format!("SIGTERM handler: {e}"))?;
    let mut int = signal(SignalKind::interrupt()).map_err(|e| format!("SIGINT handler: {e}"))?;
    loop {
        let accepted = tokio::select! {
            a = accept(&listener) => a,
            _ = usr1.recv() => { dump_filters(&filters); ctx.reg.dump(); continue; }
            _ = term.recv() => break,
            _ = int.recv() => break,
        };
        match accepted {
            Ok((conn, peer)) => {
                tokio::spawn(proxy::handle(conn, peer, ctx.clone()));
            }
            Err(e) => {
                // EMFILE and friends: back off instead of spinning.
                warn!("accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    info!("shutting down, closing {} open flow(s)", ctx.reg.len());
    ctx.reg.cancel_all();
    // Give flows a moment to write their access-log lines.
    for _ in 0..20 {
        if ctx.reg.len() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if let (Listen::Unix(p), Listener::Unix(_)) = (&cfg.listen, &listener) {
        let _ = std::fs::remove_file(p);
    }
    if !filters.is_empty() {
        let _ = std::fs::remove_file(&hub);
    }
    Ok(())
}

/// SIGUSR1: each filter's state, before the flow registry.
fn dump_filters(filters: &Filters) {
    for f in &filters.list {
        let st = match f.state() {
            State::Starting => "starting".to_string(),
            State::Up(c) => format!("up layer={:?} hosts={}", c.layer, c.hosts.join(",")),
            State::Down { last: Some(_) } => "down".to_string(),
            State::Down { last: None } => "down (no handshake: blocks all hosts)".to_string(),
        };
        info!("filter {} unix:{}: {st}", f.label(), f.path.display());
    }
}

async fn accept(l: &Listener) -> std::io::Result<(Conn, String)> {
    match l {
        Listener::Tcp(l) => {
            let (s, peer) = l.accept().await?;
            let _ = s.set_nodelay(true);
            Ok((Conn::Tcp(s), peer.to_string()))
        }
        Listener::Unix(l) => {
            let (s, _) = l.accept().await?;
            Ok((Conn::Unix(s), "unix".to_string()))
        }
    }
}
