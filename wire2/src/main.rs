//! wire2: local forward proxy (wire2/SPEC.md). This is step W2-1, the plain forwarding proxy ("zero
//! --filter flags"): CONNECT tunnels spliced as opaque bytes, plain http:// requests streamed through.
//! See NOTES.md for the design choices.

mod config;
mod conn;
mod flow;
mod host;
mod http;
mod log;
mod proxy;
mod splice;
mod upstream;

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, UnixListener};
use tokio::signal::unix::{SignalKind, signal};

use config::{Config, Listen, Parsed};
use conn::Conn;
use flow::Registry;
use proxy::Ctx;

fn main() -> ExitCode {
    let cfg = match config::parse_args(std::env::args().skip(1)) {
        Ok(Parsed::Run(c)) => c,
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

/// Random per-start instance id (SPEC: `Proxy-Id`; used by the filter handshake in W2-F).
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

fn bind_unix(path: &Path) -> Result<UnixListener, String> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() {
            return Err(format!(
                "--listen unix:{}: exists and is not a socket",
                path.display()
            ));
        }
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(format!(
                "--listen unix:{}: another process is listening there",
                path.display()
            ));
        }
        std::fs::remove_file(path)
            .map_err(|e| format!("removing stale socket {}: {e}", path.display()))?;
    }
    let l =
        UnixListener::bind(path).map_err(|e| format!("--listen unix:{}: {e}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("chmod 0600 {}: {e}", path.display()))?;
    Ok(l)
}

async fn run(cfg: Config) -> Result<(), String> {
    let listener = match &cfg.listen {
        Listen::Tcp(addr) => Listener::Tcp(
            TcpListener::bind(addr)
                .await
                .map_err(|e| format!("--listen {addr}: {e}"))?,
        ),
        Listen::Unix(path) => Listener::Unix(bind_unix(path)?),
    };
    let shown = match &listener {
        Listener::Tcp(l) => l.local_addr().map(|a| a.to_string()).unwrap_or_default(),
        Listener::Unix(_) => match &cfg.listen {
            Listen::Unix(p) => format!("unix:{}", p.display()),
            Listen::Tcp(_) => unreachable!(),
        },
    };
    let ctx = Arc::new(Ctx {
        cfg: cfg.clone(),
        reg: Registry::new(),
    });
    info!(
        "wire2 {} proxy-id={} listening on {shown} (no filters: CONNECT is an opaque tunnel, nothing is decrypted, no CA)",
        env!("CARGO_PKG_VERSION"),
        proxy_id()
    );
    if cfg.allow_remote {
        warn!("--allow-remote: anyone who can reach {shown} can use this proxy");
    }

    let mut usr1 =
        signal(SignalKind::user_defined1()).map_err(|e| format!("SIGUSR1 handler: {e}"))?;
    let mut term = signal(SignalKind::terminate()).map_err(|e| format!("SIGTERM handler: {e}"))?;
    let mut int = signal(SignalKind::interrupt()).map_err(|e| format!("SIGINT handler: {e}"))?;
    loop {
        let accepted = tokio::select! {
            a = accept(&listener) => a,
            _ = usr1.recv() => { ctx.reg.dump(); continue; }
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
    Ok(())
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
