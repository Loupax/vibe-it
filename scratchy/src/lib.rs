use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use arboard::Clipboard;
use cosmic_text::{
    Align as CosmicAlign, Attrs, Buffer, Color as TextColor, Family as CosmicFamily, FontSystem,
    LegacyRenderer, Metrics, Renderer, Shaping, Style as FontStyle, SwashCache,
    TextDecoration, UnderlineStyle, Weight as FontWeight, Wrap,
};
use cssparser::{
    match_ignore_ascii_case, AtRuleParser, CowRcStr, DeclarationParser, ParseError,
    Parser as CssParser, ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
    StyleSheetParser, Token,
};
use cssparser_color::Color as CssColor;
use html5ever::driver::{parse_document, ParseOpts, Parser};
use html5ever::tendril::stream::Utf8LossyDecoder;
use html5ever::tendril::{ByteTendril, TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom, WeakHandle};
use precomputed_hash::PrecomputedHash;
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::{
    MatchingContext, MatchingForInvalidation, MatchingMode, NeedsSelectorFlags, QuirksMode,
    SelectorCaches,
};
use selectors::matching::{matches_selector, matches_selector_list, ElementSelectorFlags};
use selectors::parser::{
    Component, NonTSPseudoClass, ParseRelative, Parser as SelectorsParser, PseudoElement, SelectorImpl,
    SelectorList, SelectorParseErrorKind,
};
use selectors::{Element as SelectorsElement, OpaqueElement};
use signal_hook::consts::SIGUSR1;
use signal_hook::iterator::Signals;
use softbuffer::{Context, Surface};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

/// Diagnostic output (`scratchy: …` lines, usage text, timing). Use these
/// instead of `eprintln!`/`println!`: those panic on a closed pipe
/// (`scratchy … 2>&1 | head -1` exited 101; QB9). See `write_line_to`.
macro_rules! diag {
    ($($arg:tt)*) => { $crate::write_line_to(&mut io::stderr().lock(), format_args!($($arg)*)) };
}
macro_rules! out {
    ($($arg:tt)*) => { $crate::write_line_to(&mut io::stdout().lock(), format_args!($($arg)*)) };
}

mod wpt;
mod dom;
mod css;
mod style;
mod layout;
mod images;
mod paint;
mod net;
mod net_policy;
mod app;
mod js;
mod js_modules;
mod js_dom;
mod js_style;

pub use dom::*;
pub use css::*;
pub(crate) use style::*;
pub use layout::*;
pub use images::*;
pub(crate) use paint::*;
pub(crate) use net::*;
pub(crate) use net_policy::*;
pub(crate) use app::*;
pub(crate) use js::*;


/// The actual entry point — `src/main.rs` is just `fn main() {
/// scratchy::run(); }`. Split into a lib so `benches/` (a separate
/// compilation unit, unable to reach into a binary crate's `main.rs` at
/// all) can link against the engine directly, for real `cargo bench`
/// targets instead of hand-editing this file with temporary
/// `eprintln!`/`Instant` timing for every performance question (see
/// NOTES.md/TODO.txt — long agreed worth doing, not done until now).
pub fn run() {
    // Headless WPT reftest runner: no window, no event loop.
    let raw_args: Vec<String> = env::args().skip(1).collect();
    if raw_args.first().map(String::as_str) == Some("--wpt") {
        std::process::exit(wpt::main(&raw_args[1..]));
    }
    if raw_args.first().map(String::as_str) == Some("--js-report") {
        std::process::exit(js::js_report_main(&raw_args[1..]));
    }
    let Args { input, socket, no_js } = parse_args();
    set_js_enabled(!no_js);

    let event_loop = EventLoop::<AppEvent>::with_user_event().build().unwrap();
    // Needed unconditionally now, not just for `Input::Stdin`'s own
    // background thread — `App::start_navigation`'s link-navigation
    // fetch (v65) needs a clone too, and `ActiveEventLoop` (unlike the
    // top-level `EventLoop` itself) has no `create_proxy` of its own
    // (confirmed by reading winit's vendored source), so this has to be
    // created here and threaded into `App` at construction time.
    let proxy = event_loop.create_proxy();

    // `SIGUSR1` bootstrap (TODO.txt's own vim-mode plan): the only way
    // to turn vim mode back *on* once `:set novim` has turned it off,
    // since nothing in-band could reach `:set vim` again at that point
    // — see `AppEvent::ToggleVimMode`'s own docs. Registered
    // unconditionally, same as the proxy above, not gated behind vim
    // mode ever having been used — a signal handler can't safely touch
    // `App` state directly (signal-safety: a raw handler interrupts the
    // main thread at an arbitrary point), so this follows the exact
    // `thread::spawn` + `proxy.send_event` shape `Input::Stdin`'s own
    // reader thread and `App::start_navigation`'s fetch thread already
    // use below — not new architecture, one more thread of the same
    // shape, blocking on `Signals::forever()` instead of a socket/stdin
    // read.
    let mut signals = Signals::new([SIGUSR1]).expect("failed to register SIGUSR1 handler");
    {
        let proxy = proxy.clone();
        thread::spawn(move || {
            for _ in signals.forever() {
                if proxy.send_event(AppEvent::ToggleVimMode).is_err() {
                    break;
                }
            }
        });
    }

    let mut app = match input {
        Input::File(path) => {
            let html = fs::read(&path).map(|b| decode_html_bytes(&b, None)).unwrap_or_else(|e| {
                diag!("scratchy: {e}");
                std::process::exit(1);
            });
            let dom = new_parser().read_from(&mut html.as_bytes()).unwrap();
            let file_resources = socket.as_deref().map(|s| load_file_resources(&proxy, s, &path, html.as_bytes()));
            let mut app = App::new_static(dom.document, proxy, socket);
            let scripts = match file_resources {
                Some((url, css, scripts)) => {
                    if app.base_url.is_none() {
                        app.base_url = url;
                    }
                    js_style::set_js_extra_css(&css);
                    app.extra_css = css;
                    scripts
                }
                None => inline_scripts(&app.document),
            };
            app.js = run_page_scripts(&app.document, &scripts, app.base_url.as_ref());
            app
        }
        Input::Stdin => {
            let stdin_proxy = proxy.clone();
            thread::spawn(move || {
                let proxy = stdin_proxy;
                let mut stdin = io::stdin();
                let mut buf = [0u8; 8192];
                let mut transcoder = StreamTranscoder::new();
                loop {
                    match stdin.read(&mut buf) {
                        Ok(0) => {
                            let tail = transcoder.decode(&[], true);
                            if !tail.is_empty() {
                                let _ = proxy.send_event(AppEvent::Chunk(tail));
                            }
                            let _ = proxy.send_event(AppEvent::Eof);
                            break;
                        }
                        Ok(n) => {
                            let out = transcoder.decode(&buf[..n], false);
                            if !out.is_empty() && proxy.send_event(AppEvent::Chunk(out)).is_err() {
                                break;
                            }
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => {
                            let _ = proxy.send_event(AppEvent::Eof);
                            break;
                        }
                    }
                }
            });
            App::new_streaming(proxy, socket)
        }
        Input::Url(url) => {
            // `fetch_url` already shows its own "Loading…" placeholder
            // and handles `socket` being `None` (prints the fallback
            // request, shows a clear inline error — see its own and
            // `start_navigation`'s docs) — the empty starting document
            // here is just a blank slate for that first `show_message`
            // call to immediately replace, never itself drawn.
            let mut app = App::new_static(parse_html(""), proxy, socket);
            app.fetch_url(url);
            app
        }
    };

    let result = event_loop.run_app(&mut app);
    if let Some(msg) = event_loop_exit_message(&result) {
        diag!("{msg}");
        std::process::exit(1);
    }
}

/// One clear line for a failed event loop, or `None` on a clean exit.
/// `ExitFailure` is what winit's Wayland backend returns when flushing
/// the connection fails (the compositor closed the socket; R8).
pub(crate) fn event_loop_exit_message(
    result: &Result<(), winit::error::EventLoopError>,
) -> Option<String> {
    use winit::error::EventLoopError;
    match result {
        Ok(()) => None,
        Err(EventLoopError::ExitFailure(_)) => Some(
            "scratchy: lost the connection to the Wayland compositor (it closed the socket; \
             usually because the main thread was busy while input queued up)"
                .to_string(),
        ),
        Err(e) => Some(format!("scratchy: event loop failed: {e}")),
    }
}

/// Writes one line and ignores errors (a closed pipe, a full disk). QB9:
/// `eprintln!`/`println!` panic on those, which turned `scratchy … | head -1`
/// into exit 101. Callers go through the `diag!`/`out!` macros.
pub(crate) fn write_line_to(w: &mut dyn Write, args: std::fmt::Arguments<'_>) {
    let _ = w.write_fmt(args);
    let _ = w.write_all(b"\n");
    let _ = w.flush();
}

/// Like `print!`: writes without a newline, ignoring errors (QB9).
pub(crate) fn write_raw_stdout(args: std::fmt::Arguments<'_>) {
    let mut out = io::stdout().lock();
    let _ = out.write_fmt(args);
    let _ = out.flush();
}

/// Logs a redraw failure once per process (a dead connection would
/// otherwise repeat it every frame).
pub(crate) fn log_redraw_error_once(what: &str, err: &dyn std::fmt::Display) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        diag!("scratchy: could not {what}: {err}");
    }
}


#[cfg(test)]
mod tests;
