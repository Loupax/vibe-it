//! web-host-cef: the CEF (Chromium) engine host. It shows one page in one window
//! (no browser UI) and is driven by the shell over a control socket in protocol v0
//! (web/PROTOCOL.md; encoding in protocol.rs).
//!
//! The same binary is the browser process and every CEF helper process
//! (renderer, GPU, network, ...): CEF re-executes it with `--type=<kind>` and
//! `execute_process` runs the helper and returns its exit code.
//!
//! Usage (run it through run.sh, which adds the memory-capped scope):
//!   web-host-cef --profile <dir> [--control <socket>] [--proxy <host:port>]
//!                [--url <url>] [--downloads <dir>] [Chromium switches...]
//! `--control`: a Unix socket the shell listens on; the host connects to it, sends
//! events and reads commands, and quits when the shell closes it. Without it the host
//! runs standalone (development). `--proxy`: an HTTP forward proxy (plain requests and
//! CONNECT) for every request, passed to Chromium as `--proxy-server`; without it
//! Chromium connects directly. Env: WEB_HOST_TRACE=1 logs the raw CEF callbacks.

use cef::*;
use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

fn ms() -> u128 {
    START.get().map(|s| s.elapsed().as_millis()).unwrap_or(0)
}

macro_rules! log {
    ($($a:tt)*) => { eprintln!("[web-host-cef +{}ms] {}", crate::ms(), format!($($a)*)) };
}

macro_rules! trace {
    ($($a:tt)*) => { if *crate::TRACE.get_or_init(|| std::env::var_os("WEB_HOST_TRACE").is_some()) { log!("trace: {}", format!($($a)*)) } };
}
static TRACE: OnceLock<bool> = OnceLock::new();

mod control;
mod download;
mod newtab;
mod forget;
mod page;
mod protocol;
mod renderer;

use protocol::{host_options, normalize_argv};

wrap_app! {
    struct HostApp {
        proxy: Option<String>,
        no_window: bool,
    }

    impl App {
        fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            // Only the browser process: Chromium hands the proxy setting to the network service.
            let is_browser = process_type.map(|t| t.to_string().is_empty()).unwrap_or(true);
            let (true, Some(cmd)) = (is_browser, command_line) else { return };
            if let Some(p) = &self.proxy {
                cmd.append_switch_with_value(Some(&CefString::from("proxy-server")), Some(&CefString::from(p.as_str())));
            }
            // --no-window needs no display: Chromium's headless platform (replaces
            // run.sh's --ozone-platform; the helper processes inherit it).
            if self.no_window {
                cmd.append_switch_with_value(Some(&CefString::from("ozone-platform")), Some(&CefString::from("headless")));
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(page::HostBrowserProcessHandler::new())
        }

        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(renderer::HostRenderProcessHandler::new())
        }
    }
}

/// argv as C strings for CEF, kept alive for the whole run.
struct Argv {
    _strings: Vec<std::ffi::CString>,
    ptrs: Vec<*const std::ffi::c_char>,
}

impl Argv {
    fn new(args: &[String]) -> Argv {
        let strings: Vec<_> = args
            .iter()
            .map(|a| std::ffi::CString::new(a.as_str()).expect("NUL in argv"))
            .collect();
        let ptrs = strings.iter().map(|s| s.as_ptr()).collect();
        Argv { _strings: strings, ptrs }
    }
    fn main_args(&self) -> MainArgs {
        MainArgs {
            argc: self.ptrs.len() as i32,
            argv: self.ptrs.as_ptr() as *mut *mut _,
        }
    }
}

fn main() {
    START.get_or_init(Instant::now);
    // Declare the CEF API version this crate was built against.
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    let raw: Vec<String> = std::env::args().collect();
    let is_browser = !raw.iter().any(|a| a.starts_with("--type="));
    // Helper processes get Chromium's own command line; only the browser process
    // sees ours, with `--opt value` that Chromium would read as a URL to open.
    let argv = if is_browser {
        normalize_argv(&raw).unwrap_or_else(|e| {
            eprintln!("web-host-cef: {e}");
            std::process::exit(2)
        })
    } else {
        raw
    };
    let opts = if is_browser {
        host_options(&argv).unwrap_or_else(|e| {
            eprintln!("web-host-cef: {e}");
            std::process::exit(2)
        })
    } else {
        Default::default()
    };
    let cargv = Argv::new(&argv);
    let main_args = cargv.main_args();
    let mut app = HostApp::new(opts.proxy.clone(), opts.no_window);

    // Helper processes (renderer, gpu, utility, zygote) run here and return >= 0.
    let code = execute_process(Some(&main_args), Some(&mut app), std::ptr::null_mut());
    if !is_browser {
        std::process::exit(code);
    }

    // Never fall back to CEF's default profile location (~/.config/...).
    let Some(profile) = opts.profile.clone() else {
        eprintln!("web-host-cef: --profile <dir> is required (a throwaway dir for development; use run.sh)");
        std::process::exit(2)
    };
    log!("browser process pid {}, profile {profile}", std::process::id());
    match &opts.proxy {
        Some(p) => log!("proxy: every request goes through the HTTP proxy {p}"),
        None => log!("no --proxy: direct connections"),
    }
    download::DIR.get_or_init(|| opts.downloads.clone().unwrap_or_else(|| format!("{profile}/downloads")));
    if let Some(path) = &opts.control {
        if let Err(e) = control::connect(path) {
            eprintln!("web-host-cef: cannot connect to the control socket {path}: {e}");
            std::process::exit(2)
        }
        log!("control socket {path} connected");
    }
    page::START_URL.get_or_init(|| protocol::normalize_url(opts.url.as_deref().unwrap_or("about:blank")));
    page::NO_WINDOW.get_or_init(|| opts.no_window);
    forget::ONE_SHOT.get_or_init(|| opts.no_window);
    forget::RECORD.get_or_init(|| format!("{profile}/web-host-origins"));
    if let Some(site) = &opts.forget_site {
        page::FORGET_AT_START.get_or_init(|| site.clone());
    }

    let settings = Settings {
        // forget-site's hidden browser is windowless (forget.rs).
        windowless_rendering_enabled: 1,
        // Chromium's own Linux sandbox (user namespaces) stays on.
        no_sandbox: std::env::var("WEB_HOST_NO_SANDBOX").is_ok() as i32,
        root_cache_path: CefString::from(profile.as_str()),
        cache_path: CefString::from(format!("{profile}/default").as_str()),
        log_file: CefString::from(format!("{profile}/cef.log").as_str()),
        ..Default::default()
    };
    let ok = initialize(Some(&main_args), Some(&settings), Some(&mut app), std::ptr::null_mut());
    assert_eq!(ok, 1, "cef initialize failed");
    control::start_reader();
    run_message_loop();
    shutdown();
    log!("shutdown");
    // --no-window --forget-site: 0 ok, 1 partial, 2 refused.
    let code = *forget::EXIT_CODE.lock().unwrap();
    if code != 0 {
        std::process::exit(code);
    }
}
