//! web spike W1: one CEF window that loads one URL. No request interception yet.
//!
//! The same binary is the browser process and every CEF helper process
//! (renderer, GPU, network, ...): CEF re-executes it with `--type=<kind>` and
//! `execute_process` runs the helper and returns its exit code.
//!
//! Usage: web-spike --url=<url> (run it through run.sh; it needs WEB_SPIKE_PROFILE).

use cef::*;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

const DEFAULT_URL: &str = "https://example.com/";

/// Open browsers; the message loop quits when the last one closes.
static BROWSERS: AtomicUsize = AtomicUsize::new(0);
static START: OnceLock<Instant> = OnceLock::new();

fn ms() -> u128 {
    START.get().map(|s| s.elapsed().as_millis()).unwrap_or(0)
}

macro_rules! log {
    ($($a:tt)*) => { eprintln!("[web-spike +{}ms] {}", ms(), format!($($a)*)) };
}

/// Turns what the user typed into a URL CEF can load: no scheme means https.
fn normalize_url(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return DEFAULT_URL.to_string();
    }
    if s.contains("://") || s.starts_with("about:") || s.starts_with("data:") {
        s.to_string()
    } else {
        format!("https://{s}")
    }
}

fn switch(cmd: &CommandLine, name: &str) -> String {
    CefString::from(&cmd.switch_value(Some(&CefString::from(name)))).to_string()
}

wrap_window_delegate! {
    struct SpikeWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 1200, height: 800 }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let bv = self.browser_view.borrow();
            let (Some(window), Some(bv)) = (window, bv.as_ref()) else { return };
            window.add_child_view(Some(&mut View::from(bv)));
            window.show();
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            *self.browser_view.borrow_mut() = None;
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let bv = self.browser_view.borrow();
            match bv.as_ref().and_then(|bv| bv.browser()) {
                Some(b) => b.host().map(|h| h.try_close_browser()).unwrap_or(1),
                None => 1,
            }
        }
    }
}

wrap_browser_view_delegate! {
    struct SpikeBrowserViewDelegate {}

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn on_popup_browser_view_created(
            &self,
            _browser_view: Option<&mut BrowserView>,
            popup: Option<&mut BrowserView>,
            _is_devtools: i32,
        ) -> i32 {
            // Popups get their own top-level window.
            let mut d = SpikeWindowDelegate::new(RefCell::new(popup.cloned()));
            window_create_top_level(Some(&mut d));
            1
        }
    }
}

wrap_client! {
    struct SpikeClient {}

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(SpikeDisplayHandler::new())
        }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(SpikeLifeSpanHandler::new())
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(SpikeLoadHandler::new())
        }
    }
}

wrap_display_handler! {
    struct SpikeDisplayHandler {}

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            log!("title: {}", title.map(|t| t.to_string()).unwrap_or_default());
            let mut browser = browser.cloned();
            if let Some(w) = browser_view_get_for_browser(browser.as_mut()).and_then(|bv| bv.window()) {
                w.set_title(title);
            }
        }

        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            _level: LogSeverity,
            message: Option<&CefString>,
            source: Option<&CefString>,
            line: i32,
        ) -> i32 {
            log!(
                "console: {} ({}:{line})",
                message.map(|m| m.to_string()).unwrap_or_default(),
                source.map(|s| s.to_string()).unwrap_or_default()
            );
            0
        }
    }
}

wrap_life_span_handler! {
    struct SpikeLifeSpanHandler {}

    impl LifeSpanHandler {
        fn on_after_created(&self, _browser: Option<&mut Browser>) {
            BROWSERS.fetch_add(1, Ordering::SeqCst);
            log!("browser created");
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            if BROWSERS.fetch_sub(1, Ordering::SeqCst) == 1 {
                log!("last browser closed, quitting");
                quit_message_loop();
            }
        }
    }
}

wrap_load_handler! {
    struct SpikeLoadHandler {}

    impl LoadHandler {
        fn on_load_end(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, status: i32) {
            let Some(frame) = frame else { return };
            if frame.is_main() != 0 {
                log!("load end: {} (HTTP {status})", CefString::from(&frame.url()));
            }
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            if frame.map(|f| f.is_main() != 0).unwrap_or(false) {
                log!(
                    "load error: {} {:?} {}",
                    failed_url.map(|u| u.to_string()).unwrap_or_default(),
                    sys::cef_errorcode_t::from(error_code),
                    error_text.map(|t| t.to_string()).unwrap_or_default()
                );
            }
        }
    }
}

wrap_browser_process_handler! {
    struct SpikeBrowserProcessHandler {}

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            let cmd = command_line_get_global().expect("no global command line");
            let url = normalize_url(&switch(&cmd, "url"));
            log!("context initialized, opening {url}");
            let mut client = Some(SpikeClient::new());
            let browser_view = browser_view_create(
                client.as_mut(),
                Some(&CefString::from(url.as_str())),
                Some(&BrowserSettings::default()),
                None,
                None,
                Some(&mut SpikeBrowserViewDelegate::new()),
            );
            let mut d = SpikeWindowDelegate::new(RefCell::new(browser_view));
            window_create_top_level(Some(&mut d));
        }
    }
}

wrap_app! {
    struct SpikeApp {}

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(SpikeBrowserProcessHandler::new())
        }
    }
}

fn main() {
    START.get_or_init(Instant::now);
    // Declare the CEF API version this crate was built against.
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    let args = args::Args::new();
    let cmd = args.as_cmd_line().expect("cannot parse the command line");
    let is_browser = cmd.has_switch(Some(&CefString::from("type"))) != 1;

    // Helper processes (renderer, gpu, utility, zygote) run here and return >= 0.
    let code = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    if !is_browser {
        std::process::exit(code);
    }

    // Never fall back to CEF's default profile location (~/.config/...).
    let profile = std::env::var("WEB_SPIKE_PROFILE").unwrap_or_else(|_| {
        eprintln!("web-spike: set WEB_SPIKE_PROFILE to a throwaway profile directory (use run.sh)");
        std::process::exit(2)
    });
    log!("browser process pid {}, profile {profile}", std::process::id());

    let settings = Settings {
        // Chromium's own Linux sandbox (user namespaces); the `sandbox` cargo
        // feature only matters for the Windows/macOS bootstrap.
        no_sandbox: std::env::var("WEB_SPIKE_NO_SANDBOX").is_ok() as i32,
        root_cache_path: CefString::from(profile.as_str()),
        cache_path: CefString::from(format!("{profile}/default").as_str()),
        log_file: CefString::from(format!("{profile}/cef.log").as_str()),
        ..Default::default()
    };
    let mut app = SpikeApp::new();
    let ok = initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    assert_eq!(ok, 1, "cef initialize failed");
    run_message_loop();
    shutdown();
    log!("shutdown");
}

#[cfg(test)]
mod tests {
    use super::normalize_url;

    #[test]
    fn normalize() {
        assert_eq!(normalize_url(""), "https://example.com/");
        assert_eq!(normalize_url(" example.com "), "https://example.com");
        assert_eq!(normalize_url("http://a.test/x"), "http://a.test/x");
        assert_eq!(normalize_url("about:blank"), "about:blank");
    }
}
